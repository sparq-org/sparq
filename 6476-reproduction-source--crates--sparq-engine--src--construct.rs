//! CONSTRUCT / DESCRIBE evaluation (T16): queries whose result is an RDF *graph*
//! rather than a solution table.
//!
//! CONSTRUCT evaluates the WHERE pattern with the ordinary SELECT machinery, then
//! instantiates the template once per solution. Per the SPARQL 1.1 spec
//! (§16.2): a template triple with an unbound variable or an illegal RDF
//! construct (a literal in subject position, a non-IRI in predicate position) is
//! silently dropped, blank nodes in the template denote *fresh* blank nodes for
//! every solution, and the result is a graph — a **set** of triples (duplicates
//! are removed; first-production order is kept for determinism).
//!
//! DESCRIBE returns the **concise bounded description** (CBD) of each described
//! resource: every triple whose subject is the resource, recursing through
//! blank-node objects (so anonymous structure hanging off the resource is
//! included in full), with cycles cut by a visited set. The spec leaves the
//! DESCRIBE result form to the implementation; CBD is the de-facto standard
//! choice (it is what Virtuoso/Jena default to) and needs no schema knowledge.
//! Inbound triples (`?s ?p <resource>`) and reifications are NOT included.
//! `DESCRIBE <iri>` / `DESCRIBE ?var WHERE {…}` both work: spargebra rewrites
//! the explicit IRIs into projected constant bindings, so the described
//! resources are exactly the bound IRI/blank-node values of the pattern's
//! solutions (literals cannot be described and are skipped).

use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple, Variable};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};

use crate::{PreparedQuery, QueryBudget, QueryResult};

/// Executes a CONSTRUCT query, returning the constructed graph as a deduplicated
/// triple list (an RDF graph is a set; first-production order is preserved).
pub fn construct(graph: &Graph, sparql: &str) -> Result<Vec<Triple>, String> {
    construct_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`construct`] under a cooperative [`QueryBudget`] (deadline / max WHERE-solution rows).
pub fn construct_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<Triple>, String> {
    construct_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`construct`] over a [`PreparedQuery`] — no per-execution parse.
pub fn construct_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<Vec<Triple>, String> {
    construct_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`construct_prepared`] under a cooperative [`QueryBudget`].
pub fn construct_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<Vec<Triple>, String> {
    let q = prepared.query();
    let active = crate::active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    match q {
        Query::Construct { template, pattern, .. } => {
            let _guard = crate::exec::budget::install(budget);
            let solutions = crate::exec::eval_select(graph, pattern)?;
            Ok(instantiate(template, &solutions))
        }
        _ => Err("construct() requires a CONSTRUCT query".into()),
    }
}

/// Executes a DESCRIBE query, returning the union of the concise bounded
/// descriptions (see module docs) of every described resource.
pub fn describe(graph: &Graph, sparql: &str) -> Result<Vec<Triple>, String> {
    describe_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`describe`] under a cooperative [`QueryBudget`] (deadline / max rows).
pub fn describe_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<Triple>, String> {
    describe_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`describe`] over a [`PreparedQuery`] — no per-execution parse.
pub fn describe_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<Vec<Triple>, String> {
    describe_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`describe_prepared`] under a cooperative [`QueryBudget`].
pub fn describe_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<Vec<Triple>, String> {
    let q = prepared.query();
    let active = crate::active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    match q {
        Query::Describe { pattern, .. } => {
            let _guard = crate::exec::budget::install(budget);
            let solutions = crate::exec::eval_select(graph, pattern)?;
            cbd(graph, &solutions)
        }
        _ => Err("describe() requires a DESCRIBE query".into()),
    }
}

/// Executes a CONSTRUCT *or* DESCRIBE query and returns the resulting RDF graph as a
/// deduplicated triple list — the form-agnostic producer behind the graph-valued query
/// forms. CONSTRUCT instantiates its template per WHERE solution; DESCRIBE returns each
/// described resource's concise bounded description (see module docs). A SELECT/ASK query
/// is rejected. Callers wanting a serialised string use [`construct_ntriples`].
pub fn construct_or_describe(graph: &Graph, sparql: &str) -> Result<Vec<Triple>, String> {
    construct_or_describe_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`construct_or_describe`] under a cooperative [`QueryBudget`].
pub fn construct_or_describe_with_budget(
    graph: &Graph,
    sparql: &str,
    budget: &QueryBudget,
) -> Result<Vec<Triple>, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    let _guard = crate::exec::budget::install(budget);
    match q {
        Query::Construct { template, pattern, .. } => {
            let solutions = crate::exec::eval_select(graph, &pattern)?;
            Ok(instantiate(&template, &solutions))
        }
        Query::Describe { pattern, .. } => {
            let solutions = crate::exec::eval_select(graph, &pattern)?;
            cbd(graph, &solutions)
        }
        _ => Err("construct_or_describe() requires a CONSTRUCT or DESCRIBE query".to_string()),
    }
}

/// Executes a CONSTRUCT *or* DESCRIBE query and serialises the resulting graph as
/// N-Triples. N-Triples is a syntactic subset of Turtle, so the returned string is
/// also a valid `text/turtle` document (the server serves it under either type).
pub fn construct_ntriples(graph: &Graph, sparql: &str) -> Result<String, String> {
    construct_ntriples_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`construct_ntriples`] under a cooperative [`QueryBudget`].
pub fn construct_ntriples_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    let triples = construct_or_describe_with_budget(graph, sparql, budget)?;
    Ok(triples_to_ntriples(&triples))
}

/// Serialises triples as canonical N-Triples (one `s p o .` line per triple).
pub fn triples_to_ntriples(triples: &[Triple]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(triples.len() * 64);
    for t in triples {
        // oxrdf's Display for the term types is canonical N-Triples syntax.
        let _ = writeln!(out, "{} {} {} .", t.subject, t.predicate, t.object);
    }
    out
}

// ---------------------------------------------------------------------------
// CONSTRUCT template instantiation
// ---------------------------------------------------------------------------

/// Instantiates `template` once per solution row, skipping triples with unbound
/// or illegal slots, freshening template blank nodes per solution, and
/// deduplicating the output (set semantics, first-production order).
fn instantiate(template: &[TriplePattern], solutions: &QueryResult) -> Vec<Triple> {
    let cols: FxHashMap<&Variable, usize> =
        solutions.vars.iter().enumerate().map(|(i, v)| (v, i)).collect();
    let mut out: Vec<Triple> = Vec::new();
    let mut seen: FxHashSet<Triple> = FxHashSet::default();
    for row in &solutions.rows {
        let get = |v: &Variable| -> Option<Term> { cols.get(v).and_then(|&i| row[i].clone()) };
        // Blank nodes in the template are scoped to one solution: the same label
        // maps to one fresh node within a row, different nodes across rows.
        // `BlankNode::default()` is a random 128-bit id, so fresh nodes can never
        // collide with data blank nodes flowing in from the WHERE solutions.
        let mut row_bnodes: FxHashMap<&str, BlankNode> = FxHashMap::default();
        for tp in template {
            let Some(s) = subject_term(&tp.subject, &get, &mut row_bnodes) else { continue };
            let Some(p) = predicate_term(&tp.predicate, &get) else { continue };
            let Some(o) = object_term(&tp.object, &get, &mut row_bnodes) else { continue };
            let t = Triple { subject: s, predicate: p, object: o };
            if seen.insert(t.clone()) {
                out.push(t);
            }
        }
    }
    out
}

/// Template subject: an IRI or blank node. A variable bound to a literal (or an
/// RDF 1.2 triple term) is an illegal subject — the triple is skipped.
fn subject_term<'a>(
    tp: &'a TermPattern,
    get: &dyn Fn(&Variable) -> Option<Term>,
    row_bnodes: &mut FxHashMap<&'a str, BlankNode>,
) -> Option<NamedOrBlankNode> {
    match tp {
        TermPattern::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n.clone())),
        TermPattern::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(fresh(b.as_str(), row_bnodes))),
        TermPattern::Variable(v) => match get(v)? {
            Term::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n)),
            Term::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(b)),
            _ => None, // literal / triple term in subject position — illegal, skip
        },
        _ => None, // RDF 1.2 triple-term template — not supported, skip
    }
}

/// Template predicate: an IRI. A variable bound to anything else is illegal.
fn predicate_term(p: &NamedNodePattern, get: &dyn Fn(&Variable) -> Option<Term>) -> Option<oxrdf::NamedNode> {
    match p {
        NamedNodePattern::NamedNode(n) => Some(n.clone()),
        NamedNodePattern::Variable(v) => match get(v)? {
            Term::NamedNode(n) => Some(n),
            _ => None, // blank node / literal in predicate position — illegal, skip
        },
    }
}

/// Template object: any RDF term is legal.
fn object_term<'a>(
    tp: &'a TermPattern,
    get: &dyn Fn(&Variable) -> Option<Term>,
    row_bnodes: &mut FxHashMap<&'a str, BlankNode>,
) -> Option<Term> {
    match tp {
        TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        TermPattern::BlankNode(b) => Some(Term::BlankNode(fresh(b.as_str(), row_bnodes))),
        TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
        TermPattern::Variable(v) => get(v),
        // An RDF 1.2 triple-term template object: instantiate the components
        // recursively (an unbound / illegal component skips the whole triple).
        TermPattern::Triple(t) => {
            let subject = subject_term(&t.subject, get, row_bnodes)?;
            let predicate = predicate_term(&t.predicate, get)?;
            let object = object_term(&t.object, get, row_bnodes)?;
            Some(Term::Triple(Box::new(Triple { subject, predicate, object })))
        }
    }
}

fn fresh<'a>(label: &'a str, row_bnodes: &mut FxHashMap<&'a str, BlankNode>) -> BlankNode {
    row_bnodes.entry(label).or_default().clone()
}

// ---------------------------------------------------------------------------
// DESCRIBE — concise bounded description
// ---------------------------------------------------------------------------

/// The union of the CBDs of every IRI / blank node bound in `solutions`:
/// all triples with the resource as subject, recursing through blank-node
/// objects (cycle-safe), deduplicated.
fn cbd(graph: &Graph, solutions: &QueryResult) -> Result<Vec<Triple>, String> {
    // Described resources in first-seen order.
    let mut queue: Vec<Term> = Vec::new();
    let mut visited: FxHashSet<Term> = FxHashSet::default();
    for row in &solutions.rows {
        for cell in row.iter().flatten() {
            match cell {
                Term::NamedNode(_) | Term::BlankNode(_)
                    if visited.insert(cell.clone()) => {
                        queue.push(cell.clone());
                    }
                _ => {} // a literal has no description
            }
        }
    }

    let mut out: Vec<Triple> = Vec::new();
    let mut seen: FxHashSet<Triple> = FxHashSet::default();
    let mut i = 0usize;
    while i < queue.len() {
        let node = queue[i].clone();
        i += 1;
        crate::exec::budget::check(out.len())?;
        let Some(id) = graph.id_of(&node) else { continue }; // resource absent from the data
        let subject = match &node {
            Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
            Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
            _ => unreachable!("only IRIs/bnodes are queued"),
        };
        let scan = graph.store.scan(&[Some(id), None, None]);
        for r in scan.rows.iter() {
            let [_, p, o] = scan.to_spo(r);
            let predicate = match graph.dict.term(p) {
                Term::NamedNode(n) => n,
                other => return Err(format!("corrupt store: non-IRI predicate {other}")),
            };
            let object = graph.dict.term(o);
            // Recurse through anonymous structure (CBD).
            if matches!(object, Term::BlankNode(_)) && visited.insert(object.clone()) {
                queue.push(object.clone());
            }
            let t = Triple { subject: subject.clone(), predicate, object };
            if seen.insert(t.clone()) {
                out.push(t);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
        ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" .
        ex:carol ex:age 35 ; ex:name "Carol" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    fn nts(ts: &[Triple]) -> Vec<String> {
        ts.iter().map(|t| format!("{} {} {} .", t.subject, t.predicate, t.object)).collect()
    }

    #[test]
    fn construct_or_describe_handles_both_forms_and_rejects_select() {
        // CONSTRUCT path: same triples as `construct`.
        let c = construct_or_describe(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert_eq!(c.len(), 3);
        // DESCRIBE path: same triples as `describe` (CBD).
        let d = construct_or_describe(&g(), "DESCRIBE <http://ex/carol>").unwrap();
        assert_eq!(d, describe(&g(), "DESCRIBE <http://ex/carol>").unwrap());
        assert!(!d.is_empty());
        // SELECT / ASK are not graph-valued => rejected.
        assert!(construct_or_describe(&g(), "SELECT ?s WHERE { ?s ?p ?o }").is_err());
        assert!(construct_or_describe(&g(), "ASK { ?s ?p ?o }").is_err());
        // And the N-Triples serialiser stays consistent with the form-agnostic producer.
        assert_eq!(
            construct_ntriples(&g(), "DESCRIBE <http://ex/carol>").unwrap(),
            triples_to_ntriples(&d),
        );
    }

    #[test]
    fn construct_basic_rename() {
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert_eq!(ts.len(), 3);
        assert!(nts(&ts).contains(
            &"<http://ex/alice> <http://ex/years> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .".to_string()
        ));
    }

    #[test]
    fn construct_multi_triple_template_and_constants() {
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s a ex:Person . ?s ex:label ?n } WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert_eq!(ts.len(), 6); // 3 type triples + 3 labels
        assert!(nts(&ts).contains(
            &"<http://ex/carol> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .".to_string()
        ));
    }

    #[test]
    fn construct_dedups_set_semantics() {
        // Every solution emits the same constant triple — the graph keeps ONE.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ex:g ex:hasPeople true } WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert_eq!(ts.len(), 1);
    }

    #[test]
    fn construct_skips_unbound_slots() {
        // ?k is unbound for carol (no ex:knows) — her template triple is dropped,
        // not emitted with a hole.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:friend ?k } WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert_eq!(ts.len(), 2); // alice->bob, bob->carol
    }

    #[test]
    fn construct_skips_illegal_slots() {
        // ?n is a literal: illegal as subject — all triples skipped.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?n ex:of ?s } WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert!(ts.is_empty());
        // ?n as predicate (a literal): likewise illegal.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ?n ?s } WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert!(ts.is_empty());
        // …but a literal OBJECT is fine.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert_eq!(ts.len(), 3);
    }

    #[test]
    fn construct_template_bnodes_fresh_per_solution() {
        // One template bnode used twice in a row: SAME node within the row,
        // DIFFERENT nodes across the 3 solutions.
        let ts = construct(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:slot _:b . _:b ex:holds ?a } WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert_eq!(ts.len(), 6);
        let mut per_row_ok = 0;
        let mut bnodes = FxHashSet::default();
        for pair in ts.chunks(2) {
            let (Term::BlankNode(b1), NamedOrBlankNode::BlankNode(b2)) = (&pair[0].object, &pair[1].subject)
            else {
                panic!("expected bnode slot/holds pair")
            };
            assert_eq!(b1, b2); // same fresh node within one solution
            per_row_ok += 1;
            bnodes.insert(b1.clone());
        }
        assert_eq!(per_row_ok, 3);
        assert_eq!(bnodes.len(), 3); // distinct across solutions
    }

    #[test]
    fn construct_where_shorthand_and_empty_result() {
        // CONSTRUCT WHERE shorthand (template = pattern).
        let ts = construct(&g(), "PREFIX ex: <http://ex/> CONSTRUCT WHERE { ?s ex:age ?a }").unwrap();
        assert_eq!(ts.len(), 3);
        // Unsatisfiable WHERE — empty graph.
        let ts = construct(&g(), "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:x ?o } WHERE { ?s ex:nope ?o }").unwrap();
        assert!(ts.is_empty());
        // Non-CONSTRUCT input is a clear error.
        assert!(construct(&g(), "SELECT * WHERE { ?s ?p ?o }").is_err());
    }

    #[test]
    fn construct_ntriples_serialisation() {
        let s = construct_ntriples(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert_eq!(s.lines().count(), 3);
        assert!(s.contains("<http://ex/alice> <http://ex/label> \"Alice\" .\n"));
        // The output must load back as Turtle (N-Triples ⊂ Turtle).
        let round = Graph::load_str(&s, "turtle").unwrap();
        assert_eq!(round.len(), 3);
        // DESCRIBE is accepted by the same entry point.
        let s = construct_ntriples(&g(), "DESCRIBE <http://ex/carol>").unwrap();
        assert_eq!(s.lines().count(), 2); // age + name
    }

    #[test]
    fn construct_budget_applies_to_where() {
        let b = QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() };
        let e = construct_with_budget(
            &g(),
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            &b,
        )
        .unwrap_err();
        assert!(e.contains("query budget exceeded (max-rows)"), "got: {e}");
    }

    #[test]
    fn describe_iri_is_cbd() {
        let ts = describe(&g(), "DESCRIBE <http://ex/alice>").unwrap();
        // alice's 3 outbound triples; nothing about bob beyond the reference.
        assert_eq!(ts.len(), 3);
        assert!(nts(&ts).iter().all(|l| l.starts_with("<http://ex/alice>")));
    }

    #[test]
    fn describe_multiple_and_var_form() {
        let ts = describe(&g(), "DESCRIBE <http://ex/alice> <http://ex/carol>").unwrap();
        assert_eq!(ts.len(), 5); // 3 + 2
        // DESCRIBE ?x WHERE — describes every binding.
        let ts = describe(
            &g(),
            "PREFIX ex: <http://ex/> DESCRIBE ?x WHERE { ?x ex:age ?a FILTER(?a > 28) }",
        )
        .unwrap();
        assert_eq!(ts.len(), 5); // alice(3) + carol(2)
        // An IRI absent from the data describes to the empty graph.
        let ts = describe(&g(), "DESCRIBE <http://ex/nobody>").unwrap();
        assert!(ts.is_empty());
        // Non-DESCRIBE input is a clear error.
        assert!(describe(&g(), "ASK { ?s ?p ?o }").is_err());
    }

    #[test]
    fn describe_recurses_through_bnodes() {
        // alice -> _:addr (bnode) -> nested bnode -> literal; bob's description
        // must NOT be pulled in even though _:addr points at it (IRI object).
        let data = r#"
            @prefix ex: <http://ex/> .
            ex:alice ex:addr _:a .
            _:a ex:city "Springfield" ; ex:geo _:g ; ex:near ex:bob .
            _:g ex:lat "1.5" .
            ex:bob ex:name "Bob" .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let ts = describe(&g, "DESCRIBE <http://ex/alice>").unwrap();
        // alice->_:a, _:a's 3 triples, _:g's 1 triple = 5; ex:bob's own name excluded.
        assert_eq!(ts.len(), 5);
        assert!(!nts(&ts).iter().any(|l| l.contains("\"Bob\"")));
    }

    #[test]
    fn describe_bnode_cycle_terminates() {
        let data = r#"
            @prefix ex: <http://ex/> .
            ex:a ex:p _:x .
            _:x ex:q _:y .
            _:y ex:q _:x .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let ts = describe(&g, "DESCRIBE <http://ex/a>").unwrap();
        assert_eq!(ts.len(), 3);
    }
}

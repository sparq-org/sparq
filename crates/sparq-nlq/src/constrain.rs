//! N2 — dictionary-grounded constraint of generated SPARQL (`sq-9yjp`,
//! `research/genai-nl-to-sparql.md` §8.1, §6 step 2).
//!
//! Grammar-constrained *decoding* in the strict sense (mask the model's logits at
//! every step so only grammar-valid tokens are ever emitted) requires **logit
//! access** and is therefore only feasible on a *local* backend, or on an API that
//! accepts a supplied grammar (GBNF / structured-output). The crate's only live
//! backend — `crate::live::AnthropicLlm` — is the Anthropic Messages API, which
//! exposes **neither** logit bias **nor** a grammar parameter, so the strict variant
//! is not implementable against it (`research/genai-nl-to-sparql.md` §11: "needs
//! logit access — only works on local/open backends; API backends … fall back to
//! post-hoc validation"). Honest scope.
//!
//! What *is* both achievable and the part the bead literally names ("against the
//! **live dictionary**") is the design doc's prescribed fallback (§6 step 2): once a
//! candidate query parses, walk its algebra, collect every IRI used in **predicate**
//! or **class** position, and check each against the live dictionary
//! ([`sparq_core::Graph::id_of`]). An IRI absent from the dictionary cannot match any
//! triple, so the query is *valid-but-wrong* — it would execute to a silent empty (or
//! mis-grounded) result. We instead turn it into a **targeted repair signal**: the
//! unknown IRI plus the nearest known terms from the same namespace ("did you mean
//! `dbo:director`?"), candidates that are free from the dictionary. This is the
//! SPARQL-LLM repair pattern, sharpened by the store's own vocabulary.
//!
//! The syntactic half of constraint (does it parse at all?) is already covered by the
//! `spargebra` parse in [`crate::Nlq::ask`]; this module adds the **semantic**
//! (vocabulary) half.

use oxrdf::{NamedNode, Term};
use spargebra::algebra::{Expression, GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::Query;
use sparq_core::dict::TermParts;
use sparq_core::strdist::edit_distance;
use sparq_core::Graph;

/// `rdf:type` — the predicate whose *object* is a class IRI, the one object position
/// we treat as a vocabulary term (every other object is data, not schema).
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// How many "did you mean" candidates to surface per unknown term.
const MAX_SUGGESTIONS: usize = 3;

/// Where in the query an unknown IRI appeared — drives the wording of the repair hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermRole {
    /// Used as a triple/path predicate.
    Predicate,
    /// Used as the object of an `rdf:type` triple (a class).
    Class,
}

impl TermRole {
    fn label(self) -> &'static str {
        match self {
            TermRole::Predicate => "predicate",
            TermRole::Class => "class",
        }
    }
}

/// One IRI used in predicate/class position that is **absent from the live
/// dictionary** — so no triple can match it. Carries up to a few nearest known terms
/// from the same namespace as repair candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTerm {
    /// The offending IRI, exactly as written in the query.
    pub iri: String,
    /// Predicate vs class position.
    pub role: TermRole,
    /// Nearest known terms (same namespace, smallest edit distance first); possibly
    /// empty when the namespace itself is unknown to the store.
    pub suggestions: Vec<String>,
}

/// Walks the parsed `query` and returns every IRI used as a **predicate** or as an
/// `rdf:type` **object (class)** that is **not present in `graph`'s dictionary**, each
/// paired with nearest known terms (see [`UnknownTerm`]). De-duplicated; order is
/// stable (first appearance). An IRI that *is* in the dictionary is omitted — only the
/// genuinely-ungrounded vocabulary is reported.
///
/// Scope is deliberately predicate and class position: those are the vocabulary terms
/// a schema-grounded query must get right, and the two the design doc's repair step
/// targets. Subject/data-object IRIs (specific entities) are left to entity linking
/// (`sq-uw40`), not this vocabulary constraint.
pub fn unknown_terms(graph: &Graph, query: &Query) -> Vec<UnknownTerm> {
    let mut found: Vec<(String, TermRole)> = Vec::new();
    let mut seen: std::collections::HashSet<(String, TermRole)> = std::collections::HashSet::new();
    let mut push = |iri: &str, role: TermRole| {
        let key = (iri.to_string(), role);
        if seen.insert(key.clone()) {
            found.push(key);
        }
    };
    match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. } => walk_pattern(pattern, &mut push),
        Query::Construct {
            pattern, template, ..
        } => {
            walk_pattern(pattern, &mut push);
            for tp in template {
                walk_triple(tp, &mut push);
            }
        }
    }

    found
        .into_iter()
        .filter(|(iri, _)| !iri_in_dictionary(graph, iri))
        .map(|(iri, role)| {
            let suggestions = nearest_known(graph, &iri);
            UnknownTerm {
                iri,
                role,
                suggestions,
            }
        })
        .collect()
}

/// Renders the repair message handed back to the model for a non-empty set of unknown
/// terms — the dictionary-constraint feedback. Stable, deterministic wording (so a
/// repair prompt is reproducible / replayable).
pub fn dictionary_repair_message(unknowns: &[UnknownTerm]) -> String {
    let mut msg = String::from(
        "the query parses, but these terms are not in the dataset's dictionary, so they \
         match no triples:",
    );
    for u in unknowns {
        msg.push_str(&format!("\n- {} <{}>", u.role.label(), u.iri));
        if !u.suggestions.is_empty() {
            let hints: Vec<String> = u.suggestions.iter().map(|s| format!("<{s}>")).collect();
            msg.push_str(&format!(" — did you mean {}?", hints.join(", ")));
        }
    }
    msg.push_str("\nUse only predicates and classes that appear in the schema summary above.");
    msg
}

/// Is `iri` a term in the live dictionary? Uses the public [`Graph::id_of`] membership
/// test (absent IRI → no id → cannot match).
fn iri_in_dictionary(graph: &Graph, iri: &str) -> bool {
    match NamedNode::new(iri) {
        Ok(node) => graph.id_of(&Term::NamedNode(node)).is_some(),
        // A syntactically invalid IRI can never be in the dictionary; spargebra would
        // not have produced it as a NamedNode, but guard anyway.
        Err(_) => false,
    }
}

/// Nearest known IRIs to `iri` from the same namespace, smallest edit distance first,
/// capped at [`MAX_SUGGESTIONS`]. A single dictionary scan; only IRI terms sharing the
/// `iri`'s namespace are considered (suggestions across namespaces are noise) and only
/// those within edit distance ≤ the local-name length (a larger distance shares almost
/// nothing — noise too).
fn nearest_known(graph: &Graph, iri: &str) -> Vec<String> {
    let Some(split) = iri.rfind(['#', '/']) else {
        return Vec::new();
    };
    let prefix = &iri[..=split];
    let suffix = &iri[split + 1..];
    let cap = suffix.chars().count().max(1);

    let mut scored: Vec<(usize, String)> = Vec::new();
    for (_, parts) in graph.dict.iter() {
        let TermParts::Iri {
            prefix: p,
            suffix: s,
        } = parts
        else {
            continue;
        };
        if p != prefix || s == suffix {
            continue;
        }
        let d = edit_distance(suffix, s);
        if d <= cap {
            scored.push((d, format!("{p}{s}")));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(MAX_SUGGESTIONS);
    scored.into_iter().map(|(_, iri)| iri).collect()
}

// ---------------------------------------------------------------------------
// Algebra walk
// ---------------------------------------------------------------------------

fn walk_pattern<F: FnMut(&str, TermRole)>(p: &GraphPattern, f: &mut F) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                walk_triple(tp, f);
            }
        }
        GraphPattern::Path { path, .. } => walk_path(path, f),
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            walk_pattern(left, f);
            walk_pattern(right, f);
        }
        // `sep-0006` (lateral join) is always enabled in this workspace.
        GraphPattern::Lateral { left, right } => {
            walk_pattern(left, f);
            walk_pattern(right, f);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            walk_pattern(left, f);
            walk_pattern(right, f);
            if let Some(e) = expression {
                walk_expr(e, f);
            }
        }
        GraphPattern::Filter { expr, inner } => {
            walk_expr(expr, f);
            walk_pattern(inner, f);
        }
        GraphPattern::Graph { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Group { inner, .. } => walk_pattern(inner, f),
        GraphPattern::Extend {
            inner, expression, ..
        } => {
            walk_pattern(inner, f);
            walk_expr(expression, f);
        }
        // A SERVICE block targets a remote endpoint whose dictionary we do not hold;
        // its vocabulary is out of scope for the local-dictionary check.
        GraphPattern::Service { .. } => {}
        GraphPattern::Values { .. } => {}
    }
}

fn walk_triple<F: FnMut(&str, TermRole)>(tp: &TriplePattern, f: &mut F) {
    if let NamedNodePattern::NamedNode(p) = &tp.predicate {
        let p = p.as_str();
        f(p, TermRole::Predicate);
        // An `rdf:type` triple's object, when a fixed IRI, is a class.
        if p == RDF_TYPE {
            if let TermPattern::NamedNode(o) = &tp.object {
                f(o.as_str(), TermRole::Class);
            }
        }
    }
    // Triple terms (RDF 1.2, `sparql-12`, always enabled here) can nest
    // predicates/classes; descend.
    if let TermPattern::Triple(inner) = &tp.subject {
        walk_triple(inner, f);
    }
    if let TermPattern::Triple(inner) = &tp.object {
        walk_triple(inner, f);
    }
}

fn walk_path<F: FnMut(&str, TermRole)>(path: &PropertyPathExpression, f: &mut F) {
    match path {
        PropertyPathExpression::NamedNode(n) => f(n.as_str(), TermRole::Predicate),
        PropertyPathExpression::Reverse(p)
        | PropertyPathExpression::ZeroOrMore(p)
        | PropertyPathExpression::OneOrMore(p)
        | PropertyPathExpression::ZeroOrOne(p) => walk_path(p, f),
        PropertyPathExpression::Sequence(a, b) | PropertyPathExpression::Alternative(a, b) => {
            walk_path(a, f);
            walk_path(b, f);
        }
        PropertyPathExpression::NegatedPropertySet(ns) => {
            for n in ns {
                f(n.as_str(), TermRole::Predicate);
            }
        }
    }
}

fn walk_expr<F: FnMut(&str, TermRole)>(e: &Expression, f: &mut F) {
    match e {
        Expression::Exists(p) => walk_pattern(p, f),
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        Expression::In(a, list) => {
            walk_expr(a, f);
            for e in list {
                walk_expr(e, f);
            }
        }
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            walk_expr(a, f)
        }
        Expression::FunctionCall(_, args) => {
            for a in args {
                walk_expr(a, f);
            }
        }
        Expression::If(a, b, c) => {
            walk_expr(a, f);
            walk_expr(b, f);
            walk_expr(c, f);
        }
        Expression::Coalesce(list) => {
            for e in list {
                walk_expr(e, f);
            }
        }
        // A literal/variable/IRI/bound leaf carries no nested schema vocabulary we
        // check (a bare IRI in expression position is a value, not a predicate/class).
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:alice a ex:Person ; rdfs:label "Alice" ; ex:knows ex:bob .
            ex:bob a ex:Person ; rdfs:label "Bob" .
        "#;
        Graph::load_str(ttl, "turtle").expect("graph parses")
    }

    fn parse(q: &str) -> Query {
        spargebra::SparqlParser::new()
            .parse_query(q)
            .expect("query parses")
    }

    #[test]
    fn known_predicate_and_class_yield_no_unknowns() {
        let g = graph();
        let q = parse(
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s rdf:type ex:Person ; ex:knows ?o }",
        );
        assert!(unknown_terms(&g, &q).is_empty());
    }

    #[test]
    fn unknown_predicate_is_flagged_with_same_namespace_suggestion() {
        let g = graph();
        // `ex:know` (typo for `ex:knows`) is absent.
        let q = parse(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:know ?o }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns[0].iri, "http://example.org/know");
        assert_eq!(unknowns[0].role, TermRole::Predicate);
        assert!(
            unknowns[0]
                .suggestions
                .contains(&"http://example.org/knows".to_string()),
            "expected ex:knows suggested, got {:?}",
            unknowns[0].suggestions
        );
    }

    #[test]
    fn unknown_class_in_rdf_type_object_is_flagged_as_class() {
        let g = graph();
        let q = parse(
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s rdf:type ex:Persons }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns[0].iri, "http://example.org/Persons");
        assert_eq!(unknowns[0].role, TermRole::Class);
        assert!(unknowns[0]
            .suggestions
            .contains(&"http://example.org/Person".to_string()));
    }

    #[test]
    fn property_path_predicate_is_checked() {
        let g = graph();
        let q = parse(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:knows+ ?o . ?s ex:nope* ?z }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns[0].iri, "http://example.org/nope");
    }

    #[test]
    fn unknown_namespace_gives_no_suggestions_but_still_flags() {
        let g = graph();
        let q = parse(
            "PREFIX foo: <http://other.example/>\n\
             SELECT ?s WHERE { ?s foo:bar ?o }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        assert!(unknowns[0].suggestions.is_empty());
    }

    #[test]
    fn repair_message_lists_terms_and_hints() {
        let unknowns = vec![UnknownTerm {
            iri: "http://example.org/know".into(),
            role: TermRole::Predicate,
            suggestions: vec!["http://example.org/knows".into()],
        }];
        let msg = dictionary_repair_message(&unknowns);
        assert!(msg.contains("predicate <http://example.org/know>"));
        assert!(msg.contains("did you mean <http://example.org/knows>?"));
    }

    /// The set of unknown-predicate IRIs reported for `q`, sorted for order-insensitive
    /// comparison. Asserting on this proves the algebra walk actually *descended* into
    /// the construct under test (an undescended arm would report nothing).
    fn unknown_iris(q: &str) -> Vec<String> {
        let g = graph();
        let mut iris: Vec<String> = unknown_terms(&g, &parse(q))
            .into_iter()
            .map(|u| u.iri)
            .collect();
        iris.sort();
        iris
    }

    #[test]
    fn union_descends_into_both_branches() {
        // One unknown predicate in each UNION branch — both must be reported.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { { ?s ex:gone ?o } UNION { ?s ex:missing ?o } }",
        );
        assert_eq!(
            iris,
            vec![
                "http://example.org/gone".to_string(),
                "http://example.org/missing".to_string()
            ]
        );
    }

    #[test]
    fn minus_descends_into_both_branches() {
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:knows ?o MINUS { ?s ex:nope ?o } }",
        );
        assert_eq!(iris, vec!["http://example.org/nope".to_string()]);
    }

    #[test]
    fn optional_leftjoin_descends_into_right_and_its_filter() {
        // OPTIONAL lowers to a LeftJoin whose `expression` is the inline FILTER; the
        // unknown predicate lives in an EXISTS inside that join condition, so this also
        // exercises the LeftJoin.expression -> walk_expr -> Exists -> walk_pattern path.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE {\n\
               ?s ex:knows ?o\n\
               OPTIONAL { ?s ex:label ?l FILTER EXISTS { ?s ex:phantom ?x } }\n\
             }",
        );
        // `ex:label` is also absent from the fixture; both should surface.
        assert_eq!(
            iris,
            vec![
                "http://example.org/label".to_string(),
                "http://example.org/phantom".to_string()
            ]
        );
    }

    #[test]
    fn filter_expression_binary_and_unary_and_exists_are_walked() {
        // FILTER ( EXISTS{...} && ! EXISTS{...} ) — exercises the binary (And), unary
        // (Not) and Exists arms of walk_expr, each carrying a distinct unknown predicate
        // through a nested EXISTS pattern.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE {\n\
               ?s ex:knows ?o\n\
               FILTER ( EXISTS { ?s ex:a ?z } && ! EXISTS { ?s ex:b ?z } )\n\
             }",
        );
        assert_eq!(
            iris,
            vec![
                "http://example.org/a".to_string(),
                "http://example.org/b".to_string()
            ]
        );
    }

    #[test]
    fn bind_extend_expression_is_walked() {
        // BIND lowers to Extend{expression}; the EXISTS inside the bound expression must
        // be descended.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s ?b WHERE {\n\
               ?s ex:knows ?o\n\
               BIND( EXISTS { ?s ex:bound ?z } AS ?b )\n\
             }",
        );
        assert_eq!(iris, vec!["http://example.org/bound".to_string()]);
    }

    #[test]
    fn function_call_if_and_coalesce_expression_args_are_walked() {
        // COALESCE(IF(EXISTS{p1}, ..., ...), ...) inside a FILTER reaches the
        // FunctionCall/If/Coalesce arms of walk_expr.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE {\n\
               ?s ex:knows ?o\n\
               FILTER( COALESCE( IF( EXISTS { ?s ex:cond ?z }, true, false ), false ) )\n\
             }",
        );
        assert_eq!(iris, vec!["http://example.org/cond".to_string()]);
    }

    #[test]
    fn property_path_reverse_sequence_alternative_and_negated_set_are_walked() {
        // ^ex:r1 / (ex:r2 | ex:r3) and a negated property set !(ex:r4) — exercises
        // Reverse, Sequence, Alternative and NegatedPropertySet in walk_path. All four
        // predicates are absent from the fixture.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE {\n\
               ?s ^ex:r1 / (ex:r2 | ex:r3) ?o .\n\
               ?s !(ex:r4) ?z\n\
             }",
        );
        assert_eq!(
            iris,
            vec![
                "http://example.org/r1".to_string(),
                "http://example.org/r2".to_string(),
                "http://example.org/r3".to_string(),
                "http://example.org/r4".to_string(),
            ]
        );
    }

    #[test]
    fn property_path_zero_or_one_is_walked() {
        // ex:r? lowers to a ZeroOrOne path.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:opt? ?o }",
        );
        assert_eq!(iris, vec!["http://example.org/opt".to_string()]);
    }

    #[test]
    fn modifiers_distinct_orderby_slice_group_are_transparent() {
        // DISTINCT + GROUP BY + ORDER BY + LIMIT all wrap the inner pattern; the walk
        // must see through every modifier layer to the unknown predicate underneath.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT DISTINCT ?s WHERE { ?s ex:hidden ?o }\n\
             GROUP BY ?s ORDER BY ?s LIMIT 10",
        );
        assert_eq!(iris, vec!["http://example.org/hidden".to_string()]);
    }

    #[test]
    fn graph_block_inner_is_walked() {
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { GRAPH ?g { ?s ex:ingraph ?o } }",
        );
        assert_eq!(iris, vec!["http://example.org/ingraph".to_string()]);
    }

    #[test]
    fn values_block_carries_no_vocabulary() {
        // A VALUES-only body binds data, not schema; nothing is flagged (the Values arm
        // is a no-op by design).
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { VALUES ?s { ex:alice ex:bob } }",
        );
        assert!(iris.is_empty());
    }

    #[test]
    fn service_block_vocabulary_is_out_of_scope() {
        // A SERVICE block's predicates belong to a remote dictionary; they must NOT be
        // flagged against the local store.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { SERVICE <http://remote.example/sparql> { ?s ex:remote ?o } }",
        );
        assert!(iris.is_empty());
    }

    #[test]
    fn construct_walks_both_template_and_where() {
        // CONSTRUCT { ?s ex:outT ?o } WHERE { ?s ex:inW ?o } — the walk must cover both
        // the template (walk_triple) and the WHERE pattern.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             CONSTRUCT { ?s ex:outT ?o } WHERE { ?s ex:inW ?o }",
        );
        assert_eq!(
            iris,
            vec![
                "http://example.org/inW".to_string(),
                "http://example.org/outT".to_string()
            ]
        );
    }

    #[test]
    fn ask_query_pattern_is_walked() {
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             ASK { ?s ex:asked ?o }",
        );
        assert_eq!(iris, vec!["http://example.org/asked".to_string()]);
    }

    #[test]
    fn quoted_triple_subject_and_object_predicates_are_descended() {
        // RDF 1.2 triple terms nest predicates in subject and object position; the walk
        // recurses into both. spargebra lowers `<< ?a ex:inner ?b >>` to a reifier whose
        // predicate is `rdf:reifies`, so that vocabulary term (absent from our fixture)
        // also surfaces — what matters is that BOTH nested `ex:inner`/`ex:innerObj`
        // predicates are reached, proving the TermPattern::Triple descent fires.
        let iris = unknown_iris(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { << ?a ex:inner ?b >> ex:knows << ?c ex:innerObj ?d >> }",
        );
        assert!(
            iris.contains(&"http://example.org/inner".to_string()),
            "subject-position nested predicate must be reached, got {iris:?}"
        );
        assert!(
            iris.contains(&"http://example.org/innerObj".to_string()),
            "object-position nested predicate must be reached, got {iris:?}"
        );
    }

    #[test]
    fn duplicate_unknown_term_is_reported_once() {
        // Same unknown predicate twice (two BGP triples) — de-duplicated to one report.
        let g = graph();
        let q = parse(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:dup ?o . ?o ex:dup ?z }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns[0].iri, "http://example.org/dup");
    }

    #[test]
    fn same_iri_in_predicate_and_class_role_are_distinct_reports() {
        // `ex:Foo` used both as a predicate and as an rdf:type object: the (iri, role)
        // key keeps them separate, so two distinct UnknownTerms are produced.
        let g = graph();
        let q = parse(
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:Foo ?o . ?o rdf:type ex:Foo }",
        );
        let mut roles: Vec<TermRole> = unknown_terms(&g, &q)
            .into_iter()
            .filter(|u| u.iri == "http://example.org/Foo")
            .map(|u| u.role)
            .collect();
        roles.sort_by_key(|r| r.label());
        assert_eq!(roles, vec![TermRole::Class, TermRole::Predicate]);
    }

    #[test]
    fn suggestions_capped_and_ordered_by_edit_distance() {
        // A namespace with several near-neighbours: suggestions are capped at
        // MAX_SUGGESTIONS and the distance-1 terms beat the distance-2 one.
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            ex:a ex:val1 1 ; ex:val2 2 ; ex:val3 3 ; ex:val4 4 ; ex:valXY 9 .
        "#;
        let g = Graph::load_str(ttl, "turtle").expect("graph parses");
        let q = parse(
            "PREFIX ex: <http://example.org/>\n\
             SELECT ?s WHERE { ?s ex:val0 ?o }",
        );
        let unknowns = unknown_terms(&g, &q);
        assert_eq!(unknowns.len(), 1);
        let s = &unknowns[0].suggestions;
        assert!(s.len() <= MAX_SUGGESTIONS, "capped, got {s:?}");
        assert!(!s.is_empty(), "near neighbours exist, got {s:?}");
        // `ex:val1..val4` are edit-distance 1 from `ex:val0`; `ex:valXY` is distance 2,
        // so within the cap the distance-1 terms win and `valXY` does not lead.
        assert_ne!(s[0], "http://example.org/valXY");
    }

    #[test]
    fn repair_message_handles_unknown_with_no_suggestions() {
        // The no-hint branch of dictionary_repair_message (empty suggestions): the term
        // is listed but no "did you mean" clause is appended.
        let unknowns = vec![UnknownTerm {
            iri: "http://other.example/bar".into(),
            role: TermRole::Class,
            suggestions: vec![],
        }];
        let msg = dictionary_repair_message(&unknowns);
        assert!(msg.contains("class <http://other.example/bar>"));
        assert!(!msg.contains("did you mean"));
        assert!(msg.contains("appear in the schema summary"));
    }
}

//! [OPUS-4.8] (sq-7d3dj.30.1) End-to-end acceptance tests for the opt-in
//! `algebra-rewrite` pre-execution pass — the SP2Bench complex-shape fix
//! (design record `research/sp2bench-complex-shape-deficit.md` §2.1 / §2.5 / §4).
//!
//! 🤖 SPARQ agent. Whole file gated on the `algebra-rewrite` feature: in the
//! default (feature-OFF) build it compiles to nothing (0 tests run — green), and
//! `cargo test -p sparq-engine --features algebra-rewrite` runs it. The
//! load-bearing invariant is BAG result-equivalence on every rewritten query,
//! proved three ways per shape:
//!
//! - `rows_on` — evaluated through `query()` (string path → `PreparedQuery::parse`
//!   → REWRITTEN), i.e. exactly what production runs;
//! - `rows_off` — the SAME query fed as RAW `spargebra` algebra through
//!   `PreparedQuery::from` (which does NOT rewrite): the un-rewritten baseline
//!   (the literal "pass on vs off");
//! - `rows_oracle` — an INDEPENDENT hand-written equivalent query (constant
//!   inlined / explicit `MINUS`) that shares no code with the rewrite.
//!
//! Plus an ANTI-VACUITY assertion per shape: the un-rewritten algebra HAD the
//! operator the rewrite removes (a post-join `Filter` / a `LeftJoin`), the
//! rewrite changed the algebra (`rewrite_query(raw) != raw`), and `explain()`
//! (which reflects the executed plan) no longer shows it — so the plan ACTUALLY
//! changed, not merely the row count.
#![cfg(feature = "algebra-rewrite")]

use sparq_core::Graph;
use sparq_engine::rewrite::rewrite_query;
use sparq_engine::{explain, query, query_prepared, PreparedQuery};
use spargebra::algebra::GraphPattern;
use spargebra::{Query, SparqlParser};

const PFX: &str = concat!(
    "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ",
    "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ",
    "PREFIX bench: <http://ex/bench#> ",
    "PREFIX swrc: <http://swrc.ontoware.org/ontology#> ",
    "PREFIX ex: <http://ex/> ",
);

fn load(nt: &str) -> Graph {
    Graph::load_str(nt, "ntriples").unwrap()
}

/// A sorted, order-independent bag of the result rows as `(var, lexical)` cells.
/// Unbound cells get a sentinel no bound RDF term can serialise to.
fn result_bag(g: &Graph, q: &str) -> Vec<Vec<(String, String)>> {
    let r = query(g, q).unwrap();
    bag_of(&r)
}

fn bag_of(r: &sparq_engine::QueryResult) -> Vec<Vec<(String, String)>> {
    const UNBOUND: &str = "\0\u{1}unbound\u{1}\0";
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let mut bag: Vec<Vec<(String, String)>> = r
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<(String, String)> = vars
                .iter()
                .zip(row.iter())
                .map(|(v, cell)| {
                    (
                        v.clone(),
                        cell.as_ref().map(|t| t.to_string()).unwrap_or_else(|| UNBOUND.to_string()),
                    )
                })
                .collect();
            cells.sort();
            cells
        })
        .collect();
    bag.sort();
    bag
}

/// The un-rewritten baseline: parse to RAW algebra and evaluate it verbatim
/// through `PreparedQuery::from` (which does NOT rewrite).
fn result_bag_raw(g: &Graph, q: &str) -> Vec<Vec<(String, String)>> {
    let raw = SparqlParser::new().parse_query(q).unwrap();
    let prepared = PreparedQuery::from(raw);
    let r = query_prepared(g, &prepared).unwrap();
    bag_of(&r)
}

fn pattern_dbg(q: &Query) -> String {
    let p: &GraphPattern = match q {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        Query::Construct { pattern, .. } | Query::Describe { pattern, .. } => pattern,
    };
    format!("{:?}", p)
}

// ---------------------------------------------------------------------------
// (1) q03b-shaped: FILTER(?property = <iri>) constant substitution
// ---------------------------------------------------------------------------

const Q03B_DATA: &str = concat!(
    "<http://ex/a1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
    "<http://ex/a1> <http://swrc.ontoware.org/ontology#month> \"3\" .\n",
    "<http://ex/a1> <http://swrc.ontoware.org/ontology#title> \"A one\" .\n",
    "<http://ex/a2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
    "<http://ex/a2> <http://swrc.ontoware.org/ontology#month> \"7\" .\n",
    "<http://ex/a2> <http://swrc.ontoware.org/ontology#pages> \"12\" .\n",
    // a3 is an Article but has NO month → must NOT appear in the result.
    "<http://ex/a3> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
    "<http://ex/a3> <http://swrc.ontoware.org/ontology#title> \"A three\" .\n",
    // A non-Article subject carrying a month → must NOT appear (fails the type seed).
    "<http://ex/x1> <http://swrc.ontoware.org/ontology#month> \"9\" .\n",
);

const Q03B: &str = "SELECT ?article ?value WHERE { \
    ?article rdf:type bench:Article . \
    ?article ?property ?value . \
    FILTER(?property = swrc:month) }";

// Independent oracle: the constant written directly into the pattern (no FILTER).
const Q03B_INLINE: &str = "SELECT ?article ?value WHERE { \
    ?article rdf:type bench:Article . \
    ?article swrc:month ?value }";

#[test]
fn q03b_equality_substitution_result_equivalent() {
    let g = load(Q03B_DATA);
    let q = format!("{PFX}{Q03B}");
    let inline = format!("{PFX}{Q03B_INLINE}");

    let rows_on = result_bag(&g, &q);
    let rows_off = result_bag_raw(&g, &q);
    let rows_oracle = result_bag(&g, &inline);

    // {a1→"3", a2→"7"} — a3 (no month) and x1 (not an Article) excluded.
    assert_eq!(rows_on.len(), 2, "expected exactly the two months: {:?}", rows_on);
    assert_eq!(rows_on, rows_off, "rewritten (on) must equal un-rewritten (off)");
    assert_eq!(rows_on, rows_oracle, "rewritten must equal the inlined-constant oracle");
}

#[test]
fn q03b_plan_actually_changed() {
    let g = load(Q03B_DATA);
    let q = format!("{PFX}{Q03B}");

    // Anti-vacuity: the un-rewritten algebra HAD a Filter on ?property.
    let raw = SparqlParser::new().parse_query(&q).unwrap();
    let raw_dbg = pattern_dbg(&raw);
    assert!(raw_dbg.contains("Filter"), "baseline must have a post-join Filter: {}", raw_dbg);

    // The rewrite changed the algebra and consumed that Filter.
    let rw = rewrite_query(raw.clone());
    assert_ne!(rw, raw, "rewrite must change the algebra");
    assert!(!pattern_dbg(&rw).contains("Filter"), "the equality Filter must be gone");

    // explain() reflects the executed (rewritten) plan: no Filter line, and the
    // month IRI is now folded into a triple pattern (a constant-seeded scan).
    let ex = explain(&g, &q).unwrap();
    assert!(!ex.contains("Filter"), "EXPLAIN must no longer show the post-join Filter:\n{}", ex);
    assert!(
        ex.contains("http://swrc.ontoware.org/ontology#month"),
        "EXPLAIN must show the folded IRI in a pattern:\n{}",
        ex
    );
}

// ---------------------------------------------------------------------------
// (2) NEGATIVE: literal-equality FILTER is NEVER rewritten (sq-lr2ii contract)
// ---------------------------------------------------------------------------

#[test]
fn literal_equality_is_not_rewritten() {
    let g = load(concat!(
        "<http://ex/s1> <http://ex/num> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://ex/s2> <http://ex/num> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    ));

    for filter in ["?value = 1", "?value = \"1\"^^xsd:decimal", "?value = \"1\"^^xsd:integer"] {
        let q = format!("{PFX} SELECT ?s ?value WHERE {{ ?s ex:num ?value . FILTER({filter}) }}");
        let raw = SparqlParser::new().parse_query(&q).unwrap();
        // Structural: the pass leaves a literal-equality FILTER completely alone.
        assert_eq!(
            rewrite_query(raw.clone()),
            raw,
            "literal-equality FILTER must be byte-identical after the pass: {}",
            filter
        );
        // And the end-to-end result is unchanged (on == off).
        assert_eq!(
            result_bag(&g, &q),
            result_bag_raw(&g, &q),
            "on/off results must agree for the un-rewritten literal FILTER: {}",
            filter
        );
    }

    // Contrast: the SAME object position with an IRI constant IS rewritten,
    // confirming it is the IRI-ness (term identity) — not the position — that gates.
    let gi = load(concat!(
        "<http://ex/s1> <http://ex/ref> <http://ex/target> .\n",
        "<http://ex/s2> <http://ex/ref> <http://ex/other> .\n",
    ));
    let q = format!("{PFX} SELECT ?s ?value WHERE {{ ?s ex:ref ?value . FILTER(?value = ex:target) }}");
    let raw = SparqlParser::new().parse_query(&q).unwrap();
    assert_ne!(rewrite_query(raw.clone()), raw, "an IRI-constant FILTER on the same slot IS rewritten");
    assert_eq!(result_bag(&gi, &q), result_bag_raw(&gi, &q), "IRI rewrite stays result-equivalent");
    assert_eq!(result_bag(&gi, &q).len(), 1, "only s1 references ex:target");
}

// ---------------------------------------------------------------------------
// (3) q07-shaped: OPTIONAL + FILTER(!bound(?v)) → anti-join (Minus)
// ---------------------------------------------------------------------------

const Q07_DATA: &str = concat!(
    "<http://ex/d1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
    "<http://ex/d1> <http://ex/bench#ref> <http://ex/bag1> .\n",
    "<http://ex/d2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
    "<http://ex/d2> <http://ex/bench#ref> <http://ex/bag2> .\n",
    // d3 is an Article with NO ref → the only !bound(?bag) survivor.
    "<http://ex/d3> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/bench#Article> .\n",
);

const Q07_OPT: &str = "SELECT ?doc WHERE { \
    ?doc rdf:type bench:Article . \
    OPTIONAL { ?doc bench:ref ?bag } \
    FILTER(!bound(?bag)) }";

// Independent oracle: explicit MINUS.
const Q07_MINUS: &str = "SELECT ?doc WHERE { \
    ?doc rdf:type bench:Article . \
    MINUS { ?doc bench:ref ?bag } }";

#[test]
fn q07_antijoin_result_equivalent() {
    let g = load(Q07_DATA);
    let q = format!("{PFX}{Q07_OPT}");
    let minus = format!("{PFX}{Q07_MINUS}");

    let rows_on = result_bag(&g, &q);
    let rows_off = result_bag_raw(&g, &q);
    let rows_oracle = result_bag(&g, &minus);

    assert_eq!(rows_on.len(), 1, "only d3 has no ref: {:?}", rows_on);
    assert_eq!(rows_on, rows_off, "rewritten (on) must equal un-rewritten (off)");
    assert_eq!(rows_on, rows_oracle, "rewritten must equal the explicit-MINUS oracle");
}

#[test]
fn q07_plan_actually_changed() {
    let g = load(Q07_DATA);
    let q = format!("{PFX}{Q07_OPT}");

    let raw = SparqlParser::new().parse_query(&q).unwrap();
    let raw_dbg = pattern_dbg(&raw);
    assert!(raw_dbg.contains("LeftJoin"), "baseline must have a LeftJoin: {}", raw_dbg);

    let rw = rewrite_query(raw.clone());
    assert_ne!(rw, raw, "rewrite must change the algebra");
    let rw_dbg = pattern_dbg(&rw);
    assert!(rw_dbg.contains("Minus"), "OPTIONAL+!bound must become Minus: {}", rw_dbg);
    assert!(!rw_dbg.contains("LeftJoin"), "the LeftJoin must be gone: {}", rw_dbg);

    let ex = explain(&g, &q).unwrap();
    assert!(ex.contains("Minus"), "EXPLAIN must show the anti-join (Minus):\n{}", ex);
    assert!(!ex.contains("LeftJoin"), "EXPLAIN must no longer show the LeftJoin:\n{}", ex);
}

// ---------------------------------------------------------------------------
// (4) NEGATIVE anti-join: no shared variable between A and B → NOT rewritten
// ---------------------------------------------------------------------------

#[test]
fn antijoin_declines_without_shared_variable() {
    let g = load(Q07_DATA);
    // B (`ex:other bench:ref ?bag`) shares no variable with A → Minus is not
    // equivalent to the OPTIONAL+!bound, so the pass must decline (verbatim).
    let q = format!(
        "{PFX} SELECT ?doc WHERE {{ ?doc rdf:type bench:Article . \
         OPTIONAL {{ <http://ex/other> bench:ref ?bag }} FILTER(!bound(?bag)) }}"
    );
    let raw = SparqlParser::new().parse_query(&q).unwrap();
    assert_eq!(rewrite_query(raw.clone()), raw, "must decline: A and B share no variable");
    assert_eq!(result_bag(&g, &q), result_bag_raw(&g, &q), "on == off when the pass declines");
}

/// A zero-length-capable path endpoint is NOT substituted: `ex:missing ex:p* ?o` matches
/// `(ex:missing, ex:missing)` on any graph, while `?s ex:p* ?o FILTER(?s = ex:missing)` only
/// ranges `?s` over graph terms. Rewritten and un-rewritten results must agree.
#[test]
fn zero_length_path_endpoint_not_substituted() {
    let g = load("<http://ex/a> <http://ex/q> <http://ex/b> .\n");
    for path in ["ex:p*", "ex:p?", "^ex:p*", "ex:p*/ex:q?", "ex:p*|ex:q"] {
        for (s, o) in [("?s", "?o"), ("?o", "?s")] {
            let q = format!("{PFX} SELECT ?s ?o WHERE {{ {s} {path} {o} FILTER(?s = ex:missing) }}");
            assert_eq!(result_bag(&g, &q), result_bag_raw(&g, &q), "{q}");
            assert!(result_bag(&g, &q).is_empty(), "{q}");
            let raw = SparqlParser::new().parse_query(&q).unwrap();
            assert_eq!(rewrite_query(raw.clone()), raw, "must decline: {q}");
        }
    }
    let c = format!("{PFX} CONSTRUCT {{ ?s ex:r ?o }} WHERE {{ ?s ex:p* ?o FILTER(?s = ex:missing) }}");
    assert!(sparq_engine::construct_or_describe(&g, &c).unwrap().is_empty());
    // A path that needs at least one step is still substituted.
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p+ ?o FILTER(?s = ex:missing) }}");
    let raw = SparqlParser::new().parse_query(&q).unwrap();
    assert_ne!(rewrite_query(raw.clone()), raw);
}

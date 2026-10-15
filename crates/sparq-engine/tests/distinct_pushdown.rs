//! Acceptance tests for the DISTINCT-projection loose (skip) index scan
//! (bead sq-7d3dj.30.4).
//!
//! For `SELECT DISTINCT ?p WHERE { BGP / UNION-of-BGPs }` the engine enumerates the
//! DISTINCT `?p` values directly from an existing permutation sorted by `?p` (a loose
//! skip scan — the general form of qlever's pattern trick over the six permutations, NO
//! new index) instead of materialising every full-width join row and deduping post-hoc.
//! The load-bearing invariant is DISTINCT result-SET equivalence with the pushdown on vs
//! off; the anti-vacuity test asserts the produced/scanned rows COLLAPSE vs the full
//! join size (the SP2Bench q09 shape materialises tens of thousands of rows to answer a
//! handful of distinct predicates today).
//!
//! Coverage:
//!   * `q09_shape_equivalence`      — SP2Bench q09-shaped 2-branch UNION of person/
//!     predicate joins: pushdown-on == pushdown-off (set semantics), and the non-person
//!     "noise" predicates are correctly excluded.
//!   * `q09_shape_anti_vacuity`     — the pushdown FIRED and the permutation rows it
//!     touched COLLAPSED to a small fraction of the full (non-distinct) join size.
//!   * `single_pattern_distinct`    — the `DISTINCT ?p WHERE { ?s ?p ?o }` skip-scan.
//!   * `multi_var_projection_fallback` — `DISTINCT ?p ?person` (>1 projected var) falls
//!     back to the correct full path (pushdown does not fire).
//!   * `non_conjunctive_fallback`   — an OPTIONAL under the DISTINCT falls back.
//!   * `three_pattern_branch_fallback` — a 3-pattern branch falls back (still correct).
//!   * `shared_projected_var_fallback` — `?p` used as BOTH the join and projected var.
//!   * `quoted_triple_term_pattern_fallback` — a DISTINCT over an RDF 1.2 quoted-triple
//!     term pattern DECLINES the pushdown (never errors) and the fallback answers correctly
//!     (W3C sparql12 eval-triple-terms/pattern-10 regression).
//!   * `public_toggle_and_stats`    — the `distinct_pushdown_testing` public surface.

use sparq_core::Graph;
use sparq_engine::{distinct_pushdown_testing, query};

const PFX: &str = "PREFIX ex: <http://example.org/>\n\
                   PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                   PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

fn load(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PFX}{ttl}"), "turtle").expect("test graph")
}

/// A stringified result table: one row per solution, one `Option<String>` per column,
/// as a SORTED set (order-insensitive; DISTINCT is a set, so we also de-duplicate).
type Table = Vec<Vec<Option<String>>>;

fn table(g: &Graph, q: &str) -> Table {
    let r = query(g, &format!("{PFX}{q}")).expect("query failed");
    let mut rows: Table = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect();
    rows.sort();
    rows
}

/// Runs `q` with the pushdown forced OFF then forced ON, returning both result tables.
fn on_off(g: &Graph, q: &str) -> (Table, Table) {
    let prev = distinct_pushdown_testing::set_enabled(false);
    let off = table(g, q);
    distinct_pushdown_testing::set_enabled(true);
    let on = table(g, q);
    distinct_pushdown_testing::set_enabled(prev);
    (off, on)
}

// ── q09 dataset ──────────────────────────────────────────────────────────────
//
// An SP2Bench-q09-shaped corpus. `N` persons, then `docs` documents that each name a
// person as the OBJECT of foaf:maker / ex:cites (HIGH multiplicity — many documents per
// person, exactly the shape whose full join materialises tens of thousands of rows to
// answer a handful of distinct predicates). Persons are emitted FIRST (so their ids form
// a contiguous low range, as real generators such as SP2Bench emit entities of a type in
// runs), then the documents — which lets the loose scan's sound range-disjointness reject
// eliminate the document- and literal-valued predicate blocks in O(1). NOISE predicates
// (ex:Doc rdf:type, foaf:age) must NOT appear in the DISTINCT ?predicate answer.
fn q09_dataset(n: usize, docs: usize) -> Graph {
    let mut ttl = String::new();
    // Phase 1: person type triples → the lowest, contiguous entity ids.
    for p in 0..n {
        ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
    }
    // Phase 2: person-subject predicates (age = few distinct inline ints; knows = person).
    for p in 0..n {
        ttl.push_str(&format!("ex:p{p} foaf:age {} .\n", 20 + (p % 10)));
        ttl.push_str(&format!("ex:p{p} foaf:knows ex:p{} .\n", (p + 1) % n));
    }
    // Phase 3: documents (higher ids) with a person as the OBJECT, high multiplicity.
    for d in 0..docs {
        ttl.push_str(&format!("ex:d{d} rdf:type ex:Doc .\n")); // noise: subject a doc, object a class
        ttl.push_str(&format!("ex:d{d} foaf:maker ex:p{} .\n", d % n));
        ttl.push_str(&format!("ex:d{d} ex:cites ex:p{} .\n", (d * 7) % n));
    }
    load(&ttl)
}

const Q09_BODY: &str = "\
  {
    ?person rdf:type foaf:Person .
    ?subject ?predicate ?person
  } UNION {
    ?person rdf:type foaf:Person .
    ?person ?predicate ?object
  }
";

/// The five predicates that genuinely relate to a person subject/object (as `Term`
/// display strings — IRIs render inside angle brackets).
fn expected_q09_predicates() -> Vec<String> {
    let mut v = vec![
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>".to_string(), // person a Person (subject)
        "<http://xmlns.com/foaf/0.1/age>".to_string(),                   // person is subject
        "<http://xmlns.com/foaf/0.1/knows>".to_string(), // subject AND object of knows is a person
        "<http://xmlns.com/foaf/0.1/maker>".to_string(), // person is object
        "<http://example.org/cites>".to_string(),        // person is object
    ];
    v.sort();
    v
}

#[test]
fn q09_shape_equivalence() {
    let g = q09_dataset(30, 90);
    let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
    let (off, on) = on_off(&g, &q);
    assert_eq!(off, on, "q09 DISTINCT result differs with pushdown on vs off");

    // The exact expected set — and that the NOISE predicate (ex:Doc rdf:type has a class,
    // not a person, as object; it only qualifies via the person-subject rdf:type triple)
    // resolves to the same set on either path.
    let got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
    assert_eq!(got, expected_q09_predicates(), "unexpected DISTINCT predicate set");
}

#[test]
fn q09_shape_anti_vacuity() {
    // High document multiplicity: many docs per person, so the full join materialises
    // thousands of rows to answer a handful of distinct predicates.
    let g = q09_dataset(40, 2000);
    let distinct_q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
    let bag_q = format!("SELECT ?predicate WHERE {{\n{Q09_BODY}\n}}");

    // The full (non-distinct) join size — how many rows the blind path materialises.
    distinct_pushdown_testing::set_enabled(false);
    let full_bag = query(&g, &format!("{PFX}{bag_q}")).expect("bag query").rows.len();

    // The pushdown path: assert it FIRED and the permutation rows it TOUCHED collapsed.
    distinct_pushdown_testing::set_enabled(true);
    distinct_pushdown_testing::reset_stats();
    let res = query(&g, &format!("{PFX}{distinct_q}")).expect("distinct query");
    let (fired, emitted, scanned) = distinct_pushdown_testing::stats();

    // Correctness holds regardless of which permutations are built (pushdown or fallback).
    assert_eq!(res.rows.len(), 5, "q09 fixture should answer 5 distinct predicates");
    assert!(full_bag > 4000, "fixture too small for a meaningful collapse: {}", full_bag);

    // The pushdown fires on either index: the full native index has the `[P, J]` = PSO
    // permutation for the second UNION branch's loose scan, and the compact (wasm) index
    // {SPO, POS, OSP}, which lacks it, answers the shape with the pattern trick
    // (sq-7d3dj.30.10), which needs no PSO.
    assert!(fired, "the DISTINCT pushdown did not fire on the q09 shape");
    assert_eq!(emitted, 5, "pushdown should emit exactly the distinct predicates");
    // Anti-vacuity: the scan touched an order of magnitude fewer rows than the full join it
    // replaces (the q09 essence — a handful of predicates, not the join), on either index.
    assert!(
        scanned * 10 < full_bag,
        "loose scan did not collapse: scanned={} full_bag={}",
        scanned,
        full_bag
    );
}

#[test]
fn single_pattern_distinct() {
    // `DISTINCT ?p WHERE { ?s ?p ?o }` — the single-pattern skip scan enumerates the
    // distinct predicates of the whole graph.
    let g = load(
        "ex:a ex:p1 ex:b . ex:a ex:p1 ex:c . ex:a ex:p2 ex:b . ex:d ex:p3 ex:e .",
    );
    let (off, on) = on_off(&g, "SELECT DISTINCT ?p WHERE { ?s ?p ?o }");
    assert_eq!(off, on);
    assert_eq!(on.len(), 3, "three distinct predicates");

    distinct_pushdown_testing::set_enabled(true);
    distinct_pushdown_testing::reset_stats();
    let _ = query(&g, &format!("{PFX}SELECT DISTINCT ?p WHERE {{ ?s ?p ?o }}")).unwrap();
    let (fired, emitted, _) = distinct_pushdown_testing::stats();
    assert!(fired, "single-pattern pushdown should fire");
    assert_eq!(emitted, 3);
}

#[test]
fn multi_var_projection_fallback() {
    // More than one projected variable → the pushdown declines; the answer is still
    // correct via the full path and the pushdown does NOT fire.
    let g = q09_dataset(8, 20);
    let q = format!("SELECT DISTINCT ?predicate ?person WHERE {{\n{Q09_BODY}\n}}");

    distinct_pushdown_testing::reset_stats();
    distinct_pushdown_testing::set_enabled(true);
    let on = table(&g, &q);
    let (fired, _, _) = distinct_pushdown_testing::stats();
    assert!(!fired, "pushdown must not fire on a multi-variable projection");

    let prev = distinct_pushdown_testing::set_enabled(false);
    let off = table(&g, &q);
    distinct_pushdown_testing::set_enabled(prev);
    assert_eq!(off, on, "multi-var fallback answer differs");
    assert!(!on.is_empty());
}

#[test]
fn non_conjunctive_fallback() {
    // An OPTIONAL under the DISTINCT is not a plain BGP/UNION → fall back, correctly.
    let g = load(
        "ex:a ex:p ex:x . ex:a ex:q ex:y . ex:x ex:extra ex:z .",
    );
    let q = "SELECT DISTINCT ?p WHERE { ex:a ?p ?o OPTIONAL { ?o ex:extra ?e } }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "OPTIONAL fallback answer differs");

    distinct_pushdown_testing::reset_stats();
    distinct_pushdown_testing::set_enabled(true);
    let _ = query(&g, &format!("{PFX}{q}")).unwrap();
    let (fired, _, _) = distinct_pushdown_testing::stats();
    assert!(!fired, "pushdown must not fire through an OPTIONAL");
}

#[test]
fn three_pattern_branch_fallback() {
    // A 3-pattern branch is outside the enumerable class → fall back, still correct.
    let g = load(
        "ex:a rdf:type ex:T . ex:a ex:link ex:b . ex:b ex:rel ex:c .
         ex:d rdf:type ex:T . ex:d ex:link ex:e . ex:e ex:rel ex:f .",
    );
    let q = "SELECT DISTINCT ?p WHERE { ?a rdf:type ex:T . ?a ex:link ?b . ?b ?p ?c }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "three-pattern fallback answer differs");
    assert!(!on.is_empty());

    distinct_pushdown_testing::reset_stats();
    distinct_pushdown_testing::set_enabled(true);
    let _ = query(&g, &format!("{PFX}{q}")).unwrap();
    let (fired, _, _) = distinct_pushdown_testing::stats();
    assert!(!fired, "pushdown must not fire on a 3-pattern branch");
}

#[test]
fn shared_projected_var_fallback() {
    // `?p` is BOTH the join variable and the projected variable (appears in both
    // patterns) — the clean anchor/probe shape does not hold, so fall back.
    let g = load(
        "ex:a ex:knows ex:b . ex:b ex:knows ex:c . ex:c ex:knows ex:a .",
    );
    let q = "SELECT DISTINCT ?p WHERE { ?x ex:knows ?p . ?p ex:knows ?y }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "shared-var fallback answer differs");
    assert!(!on.is_empty());
}

#[test]
fn empty_result_equivalence() {
    // A branch that yields no solutions (absent predicate constant) — the DISTINCT of an
    // empty relation, on vs off, must agree (an empty table with the projected column).
    let g = load("ex:a ex:p ex:b .");
    let q = "SELECT DISTINCT ?o WHERE { ex:a ex:absent ?o }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on);
    assert!(on.is_empty(), "no solutions expected");
}

/// Regression: intra-triple repeated variable must NOT push down.
///
/// Pattern `?x ?p ?x` requires subject == object; the skip-scan helpers enumerate
/// predicates without checking that constraint, so they over-approximate. This test
/// checks that the pushdown DECLINES (does not fire), the fallback answer is correct
/// (only the self-loop predicate ex:knows qualifies), and on == off.
///
/// Counterexample data: `ex:a ex:knows ex:a` (self-loop) + `ex:b ex:likes ex:c`
/// (no self-loop). Correct DISTINCT ?p = {ex:knows}; an unguarded skip-scan would
/// return {ex:knows, ex:likes}.
#[test]
fn intra_triple_repeated_var_fallback() {
    let g = load("ex:a ex:knows ex:a . ex:b ex:likes ex:c .");
    let q = "SELECT DISTINCT ?p WHERE { ?x ?p ?x }";

    // pushdown OFF gives the oracle (enforces ?x==?x via build_row)
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "repeated-var query must give the same answer on vs off");

    // The correct answer contains only ex:knows (ex:b ex:likes ex:c has subject != object)
    let expected: Vec<Vec<Option<String>>> =
        vec![vec![Some("<http://example.org/knows>".to_string())]];
    assert_eq!(on, expected, "only the self-loop predicate qualifies");

    // Crucially: the pushdown must NOT fire (the guard declines it)
    distinct_pushdown_testing::reset_stats();
    distinct_pushdown_testing::set_enabled(true);
    let _ = query(&g, &format!("{PFX}{q}")).expect("query failed");
    let (fired, _, _) = distinct_pushdown_testing::stats();
    assert!(!fired, "pushdown must not fire on a pattern with a repeated variable (?x ?p ?x)");
}

/// Regression (W3C sparql12 eval-triple-terms/pattern-10): a DISTINCT projection over a
/// pattern that embeds an RDF 1.2 quoted-triple term must DECLINE the pushdown, never error.
///
/// The general BGP planner decomposes a variable-carrying quoted-triple term
/// (`extract_quoted_constraints` -> `quoted_relation`) BEFORE pattern preparation; the
/// DISTINCT skip-scan path prepares the raw pattern, so before the `has_quoted_triple_term`
/// guard `prepare_pattern` tried to resolve the whole quoted term to a single ground id and
/// returned the hard error "variable where a term was expected", failing the whole query
/// (W3C conformance regressed 1229 -> 1228). The guard makes the pushdown decline so the
/// fallback produces the equivalent answer. (sq-7d3dj.30.4)
#[test]
fn quoted_triple_term_pattern_fallback() {
    // Two documents each annotating a triple term, plus a plain (non-annotating) triple that
    // must NOT appear in the answer. Triple-term object syntax `<<( s p o )>>` (object-only).
    let g = load(
        "ex:doc1 ex:annotates <<( ex:alice foaf:age 30 )>> .\n\
         ex:doc2 ex:annotates <<( ex:bob foaf:age 40 )>> .\n\
         ex:plain ex:other ex:thing .",
    );

    // Single-pattern branch: the projected `?s` is a simple subject variable, the object is a
    // quoted-triple term carrying variables — exactly the shape that errored pre-fix.
    let single = "SELECT DISTINCT ?s WHERE { ?s ex:annotates <<( ?a foaf:age ?n )>> }";
    // UNION-of-branches variant, mirroring the W3C pattern-10 Distinct{Project} over UNION.
    let unioned = "SELECT DISTINCT ?s WHERE { \
        { ?s ex:annotates <<( ?a foaf:age 30 )>> } UNION \
        { ?s ex:annotates <<( ?a foaf:age 40 )>> } }";

    let expected: Vec<Vec<Option<String>>> = vec![
        vec![Some("<http://example.org/doc1>".to_string())],
        vec![Some("<http://example.org/doc2>".to_string())],
    ];

    for q in [single, unioned] {
        // Pushdown ON must not error (it did pre-fix); ON == OFF; and the answer is exact.
        let (off, on) = on_off(&g, q);
        assert_eq!(off, on, "quoted-triple DISTINCT result differs with pushdown on vs off: {}", q);
        assert_eq!(on, expected, "unexpected DISTINCT ?s over a quoted-triple pattern: {}", q);

        // The pushdown must DECLINE (never fire) on a quoted-triple-term branch — the fallback
        // answered above. This asserts the fix's plan decision, not just the result.
        distinct_pushdown_testing::reset_stats();
        distinct_pushdown_testing::set_enabled(true);
        let _ = query(&g, &format!("{PFX}{q}")).expect("quoted-triple query must not error");
        let (fired, _, _) = distinct_pushdown_testing::stats();
        assert!(!fired, "pushdown must decline a quoted-triple-term pattern: {}", q);
    }
}

#[test]
fn public_toggle_and_stats() {
    // Direct unit coverage of the public `distinct_pushdown_testing` surface.
    let prev = distinct_pushdown_testing::set_enabled(false);
    assert!(!distinct_pushdown_testing::set_enabled(true), "set_enabled returns prior value");
    assert!(distinct_pushdown_testing::set_enabled(prev));

    distinct_pushdown_testing::reset_stats();
    let (fired, emitted, scanned) = distinct_pushdown_testing::stats();
    assert!(!fired && emitted == 0 && scanned == 0, "stats not cleared by reset");
}

// ── (sq-7d3dj.30.10) permutation-metadata / pattern-trick strategy tests ──────
//
// bead sq-7d3dj.30.10 added TWO strategies under the existing DISTINCT pushdown, chosen by
// anchor cardinality, and a shared-anchor cache across UNION branches:
//   * a small anchor (≤ the internal threshold) uses the per-member "pattern trick"
//     (bind the join column to each anchor member, skip-enumerate its few `?p`);
//   * a large anchor uses the cost-aware, anchor-window-CLIPPED per-`?p`-block existence
//     scan (galloping intersection driven from the shorter side).
// The load-bearing invariant is unchanged: DISTINCT result-SET equivalence with the pushdown
// on vs off, on EITHER strategy. These tests straddle the threshold so both fire, and add a
// randomized differential + Slice(LIMIT/OFFSET)-over-DISTINCT preservation + a mutation guard.

/// A large-anchor q09 corpus: `n` persons (n far exceeds the internal small-anchor
/// threshold, so the BLOCK-scan strategy is exercised), `docs` documents naming a person as
/// object with high multiplicity, plus noise predicates on documents that must be excluded.
#[test]
fn large_anchor_q09_block_scan_equivalence_and_collapse() {
    // 1000 persons > the small-anchor threshold → forces the clipped block-scan path.
    let g = q09_dataset(1000, 4000);
    let distinct_q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
    let bag_q = format!("SELECT ?predicate WHERE {{\n{Q09_BODY}\n}}");

    // Result-set equivalence (the load-bearing invariant) on the large-anchor path.
    let (off, on) = on_off(&g, &distinct_q);
    assert_eq!(off, on, "large-anchor q09 DISTINCT differs pushdown on vs off");
    // Exact set (mutation guard: any wrong/extra/missing predicate fails).
    let got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
    assert_eq!(got, expected_q09_predicates(), "large-anchor DISTINCT predicate set wrong");

    // Anti-vacuity on the block-scan path: the loose scan touched far fewer rows than the
    // full (non-distinct) join it replaces (only when the required PSO perm is built).
    distinct_pushdown_testing::set_enabled(false);
    let full_bag = query(&g, &format!("{PFX}{bag_q}")).expect("bag").rows.len();
    distinct_pushdown_testing::set_enabled(true);
    distinct_pushdown_testing::reset_stats();
    let res = query(&g, &format!("{PFX}{distinct_q}")).expect("distinct");
    let (fired, emitted, scanned) = distinct_pushdown_testing::stats();
    assert_eq!(res.rows.len(), 5, "fixture answers 5 distinct predicates");
    if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
        assert!(fired, "large-anchor q09 pushdown must fire");
        assert_eq!(emitted, 5);
        assert!(scanned * 4 < full_bag, "block scan did not collapse: scanned={} full_bag={}", scanned, full_bag);
    }
}

/// A small-anchor branch: the anchor (`ex:Book` instances) is well under the threshold, so
/// the per-member pattern-trick path runs. Equivalence + exact set + the pushdown fires.
#[test]
fn small_anchor_pattern_trick_equivalence() {
    // 12 books (small anchor) each with a title + a person creator; a large mass of noise
    // triples with unrelated predicates whose subjects/objects are never books.
    let mut ttl = String::new();
    for b in 0..12 {
        ttl.push_str(&format!("ex:book{b} rdf:type ex:Book .\n"));
        ttl.push_str(&format!("ex:book{b} ex:title \"T{b}\" .\n"));
        ttl.push_str(&format!("ex:book{b} ex:creator ex:auth{b} .\n"));
    }
    // Noise: 3000 articles with their own predicates, none a Book subject/object.
    for a in 0..3000 {
        ttl.push_str(&format!("ex:art{a} ex:journal \"J{}\" .\n", a % 50));
        ttl.push_str(&format!("ex:art{a} ex:year {} .\n", 1990 + (a % 30)));
    }
    let g = load(&ttl);
    // Branch: person(book)-as-subject predicates.
    let q = "SELECT DISTINCT ?p WHERE { \
        { ?x rdf:type ex:Book . ?x ?p ?o } UNION { ?x rdf:type ex:Book . ?s ?p ?x } }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "small-anchor pattern-trick differs pushdown on vs off");
    let mut got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
    got.sort();
    // rdf:type + ex:title + ex:creator (book as subject); ex:creator's object is auth, not a
    // book, so the object-side branch contributes nothing new here.
    let mut want = vec![
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>".to_string(),
        "<http://example.org/title>".to_string(),
        "<http://example.org/creator>".to_string(),
    ];
    want.sort();
    assert_eq!(got, want, "small-anchor DISTINCT predicate set wrong");
    if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
        distinct_pushdown_testing::reset_stats();
        distinct_pushdown_testing::set_enabled(true);
        let _ = query(&g, &format!("{PFX}{q}")).unwrap();
        assert!(distinct_pushdown_testing::stats().0, "small-anchor pushdown must fire");
    }
}

/// Randomized differential: many small random graphs of the q09 join shape, over anchor
/// sizes that STRADDLE the internal small/large-anchor threshold, must give an identical
/// DISTINCT `?predicate` set with the pushdown on vs off — so BOTH strategies are exercised.
#[test]
fn randomized_differential_both_strategies() {
    // Deterministic xorshift PRNG (no dev-dep).
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for trial in 0..40 {
        // Anchor sizes chosen to straddle the 256 threshold.
        let n = 1 + (rng() % 400) as usize; // 1..=400 persons
        let docs = (rng() % 600) as usize;
        let mut ttl = String::new();
        for p in 0..n {
            ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
            if rng() % 2 == 0 {
                ttl.push_str(&format!("ex:p{p} foaf:age {} .\n", 20 + (rng() % 40)));
            }
            if rng() % 3 == 0 {
                ttl.push_str(&format!("ex:p{p} foaf:knows ex:p{} .\n", rng() as usize % n));
            }
        }
        for d in 0..docs {
            ttl.push_str(&format!("ex:d{d} rdf:type ex:Doc .\n"));
            ttl.push_str(&format!("ex:d{d} ex:title \"T{d}\" .\n"));
            if rng() % 2 == 0 {
                ttl.push_str(&format!("ex:d{d} foaf:maker ex:p{} .\n", rng() as usize % n));
            }
            if rng() % 4 == 0 {
                ttl.push_str(&format!("ex:d{d} ex:cites ex:p{} .\n", rng() as usize % n));
            }
        }
        let g = load(&ttl);
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
        let (off, on) = on_off(&g, &q);
        assert_eq!(off, on, "trial {trial} (n={n}, docs={docs}): pushdown on != off");
    }
}

/// A Slice (LIMIT/OFFSET) above the DISTINCT must be preserved: the pushdown produces the
/// same DISTINCT set, and the slice is applied above it identically on vs off. LIMIT without
/// ORDER BY is order-insensitive, so we compare the CARDINALITY (the retained-row count),
/// which the slice must not change relative to the naive path.
#[test]
fn slice_over_distinct_preserved() {
    let g = q09_dataset(20, 60); // small anchor → pattern-trick path
    for (off_n, lim) in [(0usize, 2usize), (1, 2), (2, 10), (0, 100)] {
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}} LIMIT {lim} OFFSET {off_n}");
        let prev = distinct_pushdown_testing::set_enabled(false);
        let off_rows = query(&g, &format!("{PFX}{q}")).expect("off").rows.len();
        distinct_pushdown_testing::set_enabled(true);
        let on_rows = query(&g, &format!("{PFX}{q}")).expect("on").rows.len();
        distinct_pushdown_testing::set_enabled(prev);
        // 5 distinct predicates total; the slice retains max(0, min(lim, 5-off)).
        let full = 5usize;
        let want = lim.min(full.saturating_sub(off_n));
        assert_eq!(off_rows, want, "naive slice count wrong for off={off_n} lim={lim}");
        assert_eq!(on_rows, want, "pushdown slice count differs from naive for off={off_n} lim={lim}");
    }
}

/// (sq-7d3dj.30.10) DIRECT coverage of the block-scan strategy's ANCHOR-DRIVEN
/// intersection side: a LARGE anchor (> the small-anchor threshold, so the block scan runs)
/// that is nonetheless much smaller than a candidate `?p`-block, so `block_intersects_anchor`
/// takes its "walk the shorter sorted anchor, binary-search into the block" branch. The
/// invariant is unchanged — on == off — and the exact predicate set is asserted.
#[test]
fn block_scan_anchor_driven_side_equivalence() {
    // 300 persons (> 256 threshold → block scan) but a huge `ex:bulk` predicate whose 6000
    // distinct subjects/objects are documents, not persons (a large no-hit block that the
    // anchor-driven intersection side sweeps by binary-searching the 300 persons into it).
    let mut ttl = String::new();
    for p in 0..300 {
        ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
        ttl.push_str(&format!("ex:p{p} foaf:name \"N{p}\" .\n"));
    }
    for d in 0..6000 {
        // Documents (higher ids) with a bulk predicate; none is a person.
        ttl.push_str(&format!("ex:doc{d} ex:bulk ex:val{d} .\n"));
        // A few documents cite a person as OBJECT (so ex:cites qualifies via the object join).
        if d % 500 == 0 {
            ttl.push_str(&format!("ex:doc{d} ex:cites ex:p{} .\n", d % 300));
        }
    }
    let g = load(&ttl);
    let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
    let (off, on) = on_off(&g, &q);
    assert_eq!(off, on, "block-scan anchor-driven side differs on vs off");
    let mut got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
    got.sort();
    // name + rdf:type (person subject); cites (person object). ex:bulk must be EXCLUDED.
    let mut want = vec![
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>".to_string(),
        "<http://xmlns.com/foaf/0.1/name>".to_string(),
        "<http://example.org/cites>".to_string(),
    ];
    want.sort();
    assert_eq!(got, want, "block-scan anchor-driven side predicate set wrong (ex:bulk leaked?)");
}

// ── characteristic-set anchor-incidence prune (sq-jnb1e, opt-in) ─────────────────
//
// The `cs-anchor-incidence` feature precomputes, per anchor join position, the SET of
// predicates that relate SOME anchor member, so a candidate predicate absent from that set is
// pruned by an O(1) membership test instead of the clip+gallop block scan. The load-bearing
// invariant is DISTINCT result-SET equivalence between the incidence-pruned path and the exact
// block scan; these tests differentially verify that within one feature-ON binary via the
// runtime toggle, plus the answer-safety corners (overlay invalidation, probe-bound decline).
#[cfg(feature = "cs-anchor-incidence")]
mod incidence {
    use super::*;
    use sparq_engine::anchor_incidence_testing;

    /// Runs `q` with the incidence prune forced OFF (exact block scan) then forced ON,
    /// returning both result tables. The DISTINCT pushdown itself stays ON for both.
    fn incidence_on_off(g: &Graph, q: &str) -> (Table, Table) {
        let prev = anchor_incidence_testing::set_enabled(false);
        let off = table(g, q);
        anchor_incidence_testing::set_enabled(true);
        let on = table(g, q);
        anchor_incidence_testing::set_enabled(prev);
        (off, on)
    }

    /// The load-bearing invariant: on the q09 shape the incidence-pruned path and the exact
    /// block scan return the IDENTICAL DISTINCT predicate set, and the prune actually FIRED
    /// (the set was built and eliminated the value-typed no-hit predicates). The anchor must
    /// exceed the small-anchor threshold (256) so the BLOCK-SCAN path runs — the incidence
    /// prune accelerates the block scan; a small anchor takes the exact pattern-trick path and
    /// never consults the set (which is why `n = 300` here, not the tens used elsewhere).
    #[test]
    fn q09_incidence_equivalence_and_fired() {
        let g = q09_dataset(300, 900);
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
        let (off, on) = incidence_on_off(&g, &q);
        assert_eq!(off, on, "incidence-pruned q09 differs from the exact block scan");
        let got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
        assert_eq!(got, expected_q09_predicates(), "incidence path predicate set wrong");

        // The prune FIRED: the incidence set was built and pruned at least one candidate
        // (the fixture's ex:Doc rdf:type block relates a class, not a person, at the object
        // position, so the object-join branch prunes rdf:type's doc block via incidence).
        anchor_incidence_testing::set_enabled(true);
        anchor_incidence_testing::reset_stats();
        let _ = table(&g, &q);
        let (built, pruned) = anchor_incidence_testing::stats();
        assert!(built, "the incidence set should have been built for the q09 shape");
        assert!(pruned > 0, "the incidence prune should have eliminated a candidate predicate");
    }

    /// A LARGE value-typed no-hit block (the exact q09 residual): documents carry a
    /// high-cardinality predicate whose objects/subjects are NEVER persons. The incidence set
    /// must prune it (so the block is never scanned) AND the answer must exclude it — identical
    /// to the exact path.
    #[test]
    fn large_no_hit_block_pruned_and_excluded() {
        let mut ttl = String::new();
        for p in 0..500 {
            ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
            ttl.push_str(&format!("ex:p{p} foaf:knows ex:p{} .\n", (p + 1) % 500));
        }
        // A large value-typed predicate whose 8000 distinct objects are literals/docs — never a
        // person. This is the block whose no-hit scan the incidence set eliminates.
        for d in 0..8000 {
            ttl.push_str(&format!("ex:doc{d} dc:title \"T{d}\" .\n"));
            ttl.push_str(&format!("ex:doc{d} rdfs:seeAlso ex:ext{d} .\n"));
        }
        let ttl = format!(
            "PREFIX dc: <http://purl.org/dc/elements/1.1/>\n\
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n{ttl}"
        );
        let g = load(&ttl);
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
        let (off, on) = incidence_on_off(&g, &q);
        assert_eq!(off, on, "incidence path differs on a large no-hit-block fixture");
        let got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
        // Only knows (person subject AND object) and rdf:type (person subject) qualify;
        // dc:title / rdfs:seeAlso relate documents/literals, never a person.
        let mut want = vec![
            "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>".to_string(),
            "<http://xmlns.com/foaf/0.1/knows>".to_string(),
        ];
        want.sort();
        let mut got_sorted = got.clone();
        got_sorted.sort();
        assert_eq!(got_sorted, want, "no-hit value-typed predicate leaked into the answer");

        // The prune fired and eliminated the two large no-hit predicates (dc:title,
        // rdfs:seeAlso) on BOTH branches → at least 2 predicates pruned.
        anchor_incidence_testing::set_enabled(true);
        anchor_incidence_testing::reset_stats();
        let _ = table(&g, &q);
        let (built, pruned) = anchor_incidence_testing::stats();
        assert!(built, "incidence set not built");
        assert!(pruned >= 2, "expected the two no-hit value-typed predicates pruned, got {}", pruned);
    }

    /// Randomised differential across many graphs straddling the small/large anchor threshold
    /// and mixing person-related / document-related predicates: the incidence-pruned path must
    /// match the exact block scan on every one.
    #[test]
    fn randomized_incidence_differential() {
        // A deterministic LCG so the corpus is reproducible per-commit (no rand dep here).
        let mut state: u64 = 0x9e3779b97f4a7c15;
        let mut next = |m: u64| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) % m
        };
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
        for _ in 0..40 {
            let n = 1 + next(400) as usize; // persons: straddles the 256 anchor threshold
            let docs = next(500) as usize;
            let mut ttl = String::new();
            for p in 0..n {
                ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
                // Sometimes a person-subject / person-object predicate.
                if next(2) == 0 {
                    ttl.push_str(&format!("ex:p{p} foaf:knows ex:p{} .\n", next(n as u64)));
                }
                if next(3) == 0 {
                    ttl.push_str(&format!("ex:p{p} foaf:age {} .\n", 20 + next(50)));
                }
            }
            for d in 0..docs {
                ttl.push_str(&format!("ex:d{d} ex:bulk ex:v{d} .\n")); // never a person
                if next(4) == 0 {
                    // Some docs name a person as OBJECT (person-object predicate).
                    ttl.push_str(&format!("ex:d{d} ex:maker ex:p{} .\n", next(n as u64)));
                }
            }
            let g = load(&ttl);
            let (off, on) = incidence_on_off(&g, &q);
            assert_eq!(off, on, "incidence differential mismatch at n={} docs={}", n, docs);
        }
    }

    /// Answer-safety under a PENDING UPDATE overlay: after an INSERT that adds a NEW
    /// predicate relating a person (one the base incidence set never saw), the incidence path
    /// must DECLINE (base-index set could be stale) and fall back to the exact scan, so the new
    /// predicate is correctly INCLUDED — identical to the exact path. The anchor is > 256 so the
    /// BLOCK-SCAN path (where incidence lives) runs; the NON-VACUITY of the decline is asserted
    /// by first confirming the SAME fixture builds a set on the base graph (no overlay).
    #[test]
    fn overlay_invalidates_incidence() {
        let q = format!("SELECT DISTINCT ?predicate WHERE {{\n{Q09_BODY}\n}}");
        // Base graph (no overlay), > 256 persons → block-scan path. Confirm the incidence set
        // IS built here, so the later decline is genuinely caused by the overlay, not the shape.
        let base = q09_dataset(300, 900);
        anchor_incidence_testing::set_enabled(true);
        anchor_incidence_testing::reset_stats();
        let _ = table(&base, &q);
        let (base_built, _) = anchor_incidence_testing::stats();
        assert!(base_built, "control: the incidence set must build on the base graph (no overlay)");

        // Apply a SPARQL INSERT: a NEW predicate (ex:befriends) relating one person to another —
        // a predicate the base-index incidence set never saw, so a stale base prune would
        // WRONGLY drop it.
        let mut g = q09_dataset(300, 900);
        sparq_engine::update_in_place(
            &mut g,
            "PREFIX ex: <http://example.org/>\n\
             INSERT DATA { ex:p0 ex:befriends ex:p1 . }",
        )
        .expect("insert");
        assert!(g.store.has_overlay(), "the INSERT must produce a delta overlay");

        let (off, on) = incidence_on_off(&g, &q);
        assert_eq!(off, on, "incidence path differs from exact under an update overlay");
        let got: Vec<String> = on.iter().map(|r| r[0].clone().unwrap()).collect();
        assert!(
            got.contains(&"<http://example.org/befriends>".to_string()),
            "the newly-inserted person-relating predicate must appear in the answer"
        );

        // The incidence prune must have DECLINED (no set built) because of the overlay.
        anchor_incidence_testing::set_enabled(true);
        anchor_incidence_testing::reset_stats();
        let _ = table(&g, &q);
        let (built, _) = anchor_incidence_testing::stats();
        assert!(!built, "incidence must decline (not build a set) on a graph with an overlay");
    }

    /// A probe with an EXTRA bound position (`?person ex:knows ?x` joined to the anchor via
    /// ?person, but here the projected predicate slot is bound to a constant in one branch)
    /// must DECLINE the incidence prune — the base incidence set is over the whole predicate,
    /// not conditioned on the extra constraint — and the exact scan answers correctly.
    #[test]
    fn probe_with_extra_bound_position_declines() {
        // Query whose probe pattern binds the OBJECT to a constant: the projected `?predicate`
        // relates a person only via SOME triple, but the pushdown's block scan is over the
        // constrained pattern. The incidence set (unconstrained) must NOT prune it. We assert
        // equivalence; the pushdown may decline the whole shape, which is also correct.
        let mut ttl = String::new();
        for p in 0..300 {
            ttl.push_str(&format!("ex:p{p} rdf:type foaf:Person .\n"));
            ttl.push_str(&format!("ex:p{p} foaf:knows ex:target .\n"));
            ttl.push_str(&format!("ex:p{p} foaf:mbox ex:other .\n"));
        }
        let g = load(&ttl);
        // Probe binds the object to ex:target: DISTINCT ?predicate where a person is subject of
        // a triple whose OBJECT is ex:target. (foaf:knows qualifies; foaf:mbox does not.)
        let q = "SELECT DISTINCT ?predicate WHERE {\n\
                   ?person rdf:type foaf:Person .\n\
                   ?person ?predicate ex:target\n\
                 }";
        let (off, on) = incidence_on_off(&g, q);
        assert_eq!(off, on, "extra-bound-position probe differs on vs off (unsound prune?)");
    }
}

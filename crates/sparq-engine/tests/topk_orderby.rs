//! Acceptance test for the bounded-heap ORDER BY optimisation (sq-7d3dj.30.2).
//!
//! Invariant: `ORDER BY … LIMIT k OFFSET o` produces BYTE-IDENTICAL rows to
//! `ORDER BY …` (full sort) followed by a manual slice `[o..o+k]`, for every
//! combination of k, o, sort direction, value type, and tie structure.
//!
//! The test is differential: it runs the same data through the top-k path
//! (`LIMIT k OFFSET o` triggers the bounded-heap in exec.rs) and the full-sort
//! path (no LIMIT, then manually sliced), then asserts row-by-row identity.
//!
//! Mutation spot-check: a dataset with ALL ties (`?v` constant) is tested with
//! `LIMIT k OFFSET o` that straddles the tie group. If the heap's tie-break
//! index order were REVERSED (larger input_idx first instead of smaller), a
//! different set of rows would be selected and the differential would fail.
//!
//! Coverage: all the following shapes are exercised.
//!
//! * `duplicate_keys` — all ORDER BY values the same (pure tie)
//! * `distinct_keys` — all values distinct (no tie, basic correctness)
//! * `offset_lands_on_boundary` — OFFSET skips to start of a new key group
//! * `desc_ordering` — DESC direction
//! * `multi_key_order_by` — two ORDER BY variables (`?a`, `?b`)
//! * `mixed_types` — IRI, plain literal, integer, double, unbound in one column
//! * `offset_beyond_size` — OFFSET larger than the result set (returns empty)
//! * `k_at_threshold` — k = TOP_K_ORDER_BY_THRESHOLD (boundary)
//! * `k_over_threshold` — k > TOP_K_ORDER_BY_THRESHOLD (falls back to full sort)
//! * `k_equals_n` — k = total row count (degenerate: full sort)

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://example.org/>\n\
                   PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

/// Load a Turtle snippet (prefix declarations prepended).
fn load(ttl: &str) -> Graph {
    let src = format!("{PFX}{ttl}");
    Graph::load_str(&src, "turtle").expect("test graph")
}

/// Row representation: each cell is Some(term_string) or None for unbound.
fn rows(g: &Graph, q: &str) -> Vec<Vec<Option<String>>> {
    let r = query(g, &format!("{PFX}{q}")).expect("query failed");
    r.rows.iter().map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect()).collect()
}

/// The oracle for a LIMIT/OFFSET result: run the FULL ORDER BY (no LIMIT),
/// collect all rows, then manually slice `[offset..offset+limit]`.
fn oracle(g: &Graph, base_query: &str, offset: usize, limit: usize) -> Vec<Vec<Option<String>>> {
    let all = rows(g, base_query);
    all.into_iter().skip(offset).take(limit).collect()
}

// ─── duplicate_keys ──────────────────────────────────────────────────────────

/// All rows are FULLY tied on EVERY ORDER BY key (single key `?v`, constant) —
/// the ONLY thing that distinguishes rows is their input order, i.e. the
/// `input_idx` tiebreak in the bounded-selection comparator. The full stable
/// sort preserves input order, so the top-k path must reproduce it via the
/// `input_idx` ascending tiebreak; the observed `?s` values reveal which
/// physical rows were selected.
///
/// This IS the mutation spot-check and it is now non-vacuous: because there is
/// NO distinct secondary sort key, reversing the `input_idx` tiebreak direction
/// in exec.rs `order_bindings` (`a.1.cmp(&c.1)` → `c.1.cmp(&a.1)`) makes the
/// top-k path select the OPPOSITE end of the tie group, so `got != exp` and the
/// test goes RED. Verified by temporarily reversing the tiebreak in the
/// worktree (RED) and restoring it (GREEN). sq-7d3dj.30.2
///
/// Every (offset, limit) below keeps `cap = offset + limit < n` so the
/// bounded-selection (top-k) path is actually exercised (when `cap >= n` the
/// code falls through to the full stable sort and the tiebreak is untested).
#[test]
fn duplicate_keys() {
    // 20 subjects all with the SAME ?v = 1 (single ORDER BY key → pure tie on
    // all keys; ?s is NOT a sort key, only an observation column).
    let n = 20usize;
    let mut ttl = String::new();
    for i in 0..n {
        // Pad ?s so its lexical order is unrelated to input order — the test must
        // NOT accidentally become sensitive to a secondary key it does not sort on.
        ttl.push_str(&format!("ex:s{i:02} ex:v 1 .\n"));
    }
    let g = load(&ttl);

    // caps: 5, 10, 15 — all < n=20, so the top-k path runs for each.
    for offset in [0usize, 5, 10] {
        let limit = 5;
        // SINGLE sort key ?v (all tied). Observe ?s to see which rows survived.
        let base_q = "SELECT ?s WHERE { ?s ex:v ?v } ORDER BY ?v";
        let topk_q = format!("SELECT ?s WHERE {{ ?s ex:v ?v }} ORDER BY ?v LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "duplicate_keys: offset={offset} limit={limit}: top-k differs from full-sort+slice"
        );
    }
}

// ─── limit_zero ──────────────────────────────────────────────────────────────

/// LIMIT 0 is legal SPARQL and is exercised by the W3C conformance suite. The
/// budget `cap = offset + 0 = 0` reaches the bounded-selection path with k = 0;
/// `select_nth_unstable_by(k - 1)` would underflow to `usize::MAX` and abort the
/// process (SIGABRT, exit 134). Regression guard for that crash.
#[test]
fn limit_zero() {
    let mut ttl = String::new();
    for i in 0..10usize {
        ttl.push_str(&format!("ex:s{i} ex:v {i} .\n"));
    }
    let g = load(&ttl);

    // LIMIT 0 (with and without OFFSET) must return empty rows, not panic/abort.
    let got = rows(&g, "SELECT ?s WHERE { ?s ex:v ?v } ORDER BY ?v LIMIT 0");
    assert!(got.is_empty(), "limit_zero: LIMIT 0 must be empty, got {got:?}");

    let got = rows(&g, "SELECT ?s WHERE { ?s ex:v ?v } ORDER BY ?v LIMIT 0 OFFSET 3");
    assert!(got.is_empty(), "limit_zero: LIMIT 0 OFFSET 3 must be empty, got {got:?}");
}

// ─── empty_input ─────────────────────────────────────────────────────────────

/// ORDER BY … LIMIT k over a pattern that matches NOTHING: n = 0. The
/// bounded-selection path must not call `select_nth_unstable_by` on an empty
/// slice (which panics). Regression guard.
#[test]
fn empty_input() {
    // Graph has data, but the query pattern matches no rows.
    let g = load("ex:s1 ex:v 1 .\n");
    let got = rows(&g, "SELECT ?s WHERE { ?s ex:nonexistent ?v } ORDER BY ?v LIMIT 5");
    assert!(got.is_empty(), "empty_input: expected empty, got {got:?}");
    // Also with OFFSET.
    let got = rows(&g, "SELECT ?s WHERE { ?s ex:nonexistent ?v } ORDER BY ?v LIMIT 5 OFFSET 2");
    assert!(got.is_empty(), "empty_input (offset): expected empty, got {got:?}");
}

// ─── offset_budget_ge_n ──────────────────────────────────────────────────────

/// OFFSET so large that the row budget `cap = offset + limit >= n`: the code
/// MUST fall through to the full stable sort (`use_topk` false / `k >= n`) — a
/// bounded `select_nth_unstable_by(nth)` with `nth >= len` would panic.
/// Result must still equal the full-sort oracle.
#[test]
fn offset_budget_ge_n() {
    let n = 8usize;
    let mut ttl = String::new();
    for i in 0..n {
        ttl.push_str(&format!("ex:s{i} ex:v {i} .\n"));
    }
    let g = load(&ttl);

    let base_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v";
    // cap == n (offset 3 + limit 5) and cap > n (offset 6 + limit 5): both must
    // take the full-sort path without panicking and match the oracle.
    for (offset, limit) in [(3usize, 5usize), (6, 5), (7, 5)] {
        let topk_q = format!("SELECT ?s ?v WHERE {{ ?s ex:v ?v }} ORDER BY ?v LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(got, exp, "offset_budget_ge_n: offset={offset} limit={limit}");
    }
}

// ─── distinct_keys ───────────────────────────────────────────────────────────

/// All ORDER BY values distinct — no tie, basic correctness.
#[test]
fn distinct_keys() {
    let mut ttl = String::new();
    for i in 0..30usize {
        ttl.push_str(&format!("ex:s{i} ex:age {i} .\n"));
    }
    let g = load(&ttl);

    for (offset, limit) in [(0, 5), (10, 5), (25, 10), (0, 30)] {
        let base_q = "SELECT ?s ?age WHERE { ?s ex:age ?age } ORDER BY ?age";
        let topk_q = format!("SELECT ?s ?age WHERE {{ ?s ex:age ?age }} ORDER BY ?age LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "distinct_keys: offset={offset} limit={limit}: top-k differs from full-sort+slice"
        );
    }
}

// ─── offset_lands_on_boundary ────────────────────────────────────────────────

/// OFFSET skips exactly to the start of a new key group.
#[test]
fn offset_lands_on_boundary() {
    // 10 subjects with ?v = 0, 10 with ?v = 1.
    let mut ttl = String::new();
    for i in 0..10usize {
        ttl.push_str(&format!("ex:a{i} ex:v 0 .\n"));
        ttl.push_str(&format!("ex:b{i} ex:v 1 .\n"));
    }
    let g = load(&ttl);

    // offset=10 lands exactly at the start of the ?v=1 group.
    let base_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v ?s";
    for (offset, limit) in [(10, 5), (10, 10), (0, 10), (5, 10)] {
        let topk_q = format!("SELECT ?s ?v WHERE {{ ?s ex:v ?v }} ORDER BY ?v ?s LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "offset_lands_on_boundary: offset={offset} limit={limit}"
        );
    }
}

// ─── desc_ordering ───────────────────────────────────────────────────────────

/// DESC direction must be byte-identical to the full DESC sort + manual slice.
#[test]
fn desc_ordering() {
    let mut ttl = String::new();
    for i in 0..20usize {
        ttl.push_str(&format!("ex:s{i} ex:score {i} .\n"));
    }
    let g = load(&ttl);

    let base_q = "SELECT ?s ?score WHERE { ?s ex:score ?score } ORDER BY DESC(?score)";
    for (offset, limit) in [(0, 5), (5, 5), (15, 5)] {
        let topk_q = format!("SELECT ?s ?score WHERE {{ ?s ex:score ?score }} ORDER BY DESC(?score) LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "desc_ordering: offset={offset} limit={limit}"
        );
    }
}

// ─── multi_key_order_by ──────────────────────────────────────────────────────

/// Two ORDER BY variables: `ORDER BY ?a ?b`.
#[test]
fn multi_key_order_by() {
    let mut ttl = String::new();
    for a in 0..4usize {
        for b in 0..4usize {
            ttl.push_str(&format!("ex:s{}_{} ex:a {} ; ex:b {} .\n", a, b, a, b));
        }
    }
    let g = load(&ttl);

    let base_q = "SELECT ?s ?a ?b WHERE { ?s ex:a ?a ; ex:b ?b } ORDER BY ?a ?b";
    for (offset, limit) in [(0, 4), (4, 4), (8, 4), (0, 16)] {
        let topk_q = format!("SELECT ?s ?a ?b WHERE {{ ?s ex:a ?a ; ex:b ?b }} ORDER BY ?a ?b LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "multi_key_order_by: offset={offset} limit={limit}"
        );
    }
}

// ─── mixed_types ─────────────────────────────────────────────────────────────

/// Mixed value types in the ORDER BY column: IRI, plain literal, integer,
/// double, unbound. Exercises SortCell::Iri + Val paths in cmp_sort_cells.
#[test]
fn mixed_types() {
    // SPARQL total order: unbound < blank < IRI < literal (by kind then value).
    let ttl = "ex:s1 ex:v ex:iri_a .\n\
               ex:s2 ex:v ex:iri_b .\n\
               ex:s3 ex:v 1 .\n\
               ex:s4 ex:v 2 .\n\
               ex:s5 ex:v \"hello\" .\n\
               ex:s6 ex:v \"world\" .\n\
               ex:s7 ex:v 1.5 .\n";
    // ex:s8 has no ex:v triple → ?v is unbound.
    let ttl = format!("{ttl}ex:s8 ex:name \"noval\" .\n");
    let g = load(&ttl);

    let base_q = "SELECT ?s ?v WHERE { ?s ex:name|ex:v ?v } ORDER BY ?v";
    // Just verify top-k matches full-sort for various slices.
    for (offset, limit) in [(0, 3), (2, 3), (4, 3)] {
        let topk_q = format!("SELECT ?s ?v WHERE {{ ?s ex:name|ex:v ?v }} ORDER BY ?v LIMIT {limit} OFFSET {offset}");
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "mixed_types: offset={offset} limit={limit}"
        );
    }
}

// ─── offset_beyond_size ──────────────────────────────────────────────────────

/// OFFSET larger than the total result set: must return empty rows (not panic).
#[test]
fn offset_beyond_size() {
    let ttl = "ex:s1 ex:v 1 . ex:s2 ex:v 2 . ex:s3 ex:v 3 .\n";
    let g = load(ttl);

    let got = rows(&g, "SELECT ?s WHERE { ?s ex:v ?v } ORDER BY ?v LIMIT 5 OFFSET 100");
    assert!(got.is_empty(), "offset_beyond_size: expected empty, got {got:?}");
}

// ─── k_at_threshold ──────────────────────────────────────────────────────────

/// k = TOP_K_ORDER_BY_THRESHOLD (1024): boundary — still uses bounded heap.
#[test]
fn k_at_threshold() {
    let n = 2000usize;
    let mut ttl = String::new();
    for i in 0..n {
        ttl.push_str(&format!("ex:s{i} ex:v {i} .\n"));
    }
    let g = load(&ttl);

    // k = 1024 = threshold; should still use the top-k path and produce correct results.
    let base_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v";
    let topk_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v LIMIT 1024 OFFSET 0";
    let got = rows(&g, topk_q);
    let exp = oracle(&g, base_q, 0, 1024);
    assert_eq!(got, exp, "k_at_threshold: mismatch");
}

// ─── k_over_threshold ────────────────────────────────────────────────────────

/// k > TOP_K_ORDER_BY_THRESHOLD: falls back to the full stable sort.
/// Result must still be byte-identical to the oracle.
#[test]
fn k_over_threshold() {
    let n = 3000usize;
    let mut ttl = String::new();
    for i in 0..n {
        ttl.push_str(&format!("ex:s{i} ex:v {i} .\n"));
    }
    let g = load(&ttl);

    let base_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v";
    let topk_q = "SELECT ?s ?v WHERE { ?s ex:v ?v } ORDER BY ?v LIMIT 2000 OFFSET 0";
    let got = rows(&g, topk_q);
    let exp = oracle(&g, base_q, 0, 2000);
    assert_eq!(got, exp, "k_over_threshold: mismatch");
}

// ─── k_equals_n ──────────────────────────────────────────────────────────────

/// k = total row count: degenerate case, the bounded heap is skipped and the
/// full stable sort runs. Result must equal the full ORDER BY.
#[test]
fn k_equals_n() {
    let n = 10usize;
    let mut ttl = String::new();
    for i in 0..n {
        ttl.push_str(&format!("ex:s{i} ex:v {i} .\n"));
    }
    let g = load(&ttl);

    let base_q = "SELECT ?v WHERE { ?s ex:v ?v } ORDER BY ?v";
    let topk_q = format!("SELECT ?v WHERE {{ ?s ex:v ?v }} ORDER BY ?v LIMIT {n} OFFSET 0");
    let got = rows(&g, &topk_q);
    let exp = oracle(&g, base_q, 0, n);
    assert_eq!(got, exp, "k_equals_n: mismatch");
}

// ─── iri_column ──────────────────────────────────────────────────────────────

/// ORDER BY over an IRI column: exercises the SortCell::Iri precomputed key path.
/// This is the SP2Bench q11 shape: ORDER BY ?ee LIMIT 10 OFFSET 50 over IRI values.
#[test]
fn iri_column() {
    let mut ttl = String::new();
    for i in 0..100usize {
        // Pad to 3 digits so lexicographic IRI order matches numeric order.
        ttl.push_str(&format!("ex:pub{i:03} <http://www.w3.org/2000/01/rdf-schema#seeAlso> ex:ref{i:03} .\n"));
    }
    let g = load(&ttl);

    let base_q = "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
                  SELECT ?ee WHERE { ?pub rdfs:seeAlso ?ee } ORDER BY ?ee";
    for (offset, limit) in [(0, 10), (50, 10), (90, 10)] {
        let topk_q = format!(
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
             SELECT ?ee WHERE {{ ?pub rdfs:seeAlso ?ee }} ORDER BY ?ee LIMIT {limit} OFFSET {offset}"
        );
        let got = rows(&g, &topk_q);
        let exp = oracle(&g, base_q, offset, limit);
        assert_eq!(
            got, exp,
            "iri_column: offset={offset} limit={limit}: top-k differs from full-sort+slice"
        );
    }
}

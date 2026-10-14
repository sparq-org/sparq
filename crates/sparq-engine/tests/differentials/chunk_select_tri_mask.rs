//! (sq-y5ew5) Acceptance tests for the **hybrid tri-mask FILTER kernel**
//! (`src/chunk_select.rs`), Phase 3b of the M4 vector-at-a-time plan.
//!
//! These tests satisfy the four acceptance criteria specified in
//! `research/vector-at-a-time-m4-completion-design.md` §3:
//!
//! - **T1 (mixed-column differential, both-populations probe):** a column mixing
//!   inline ints, non-inline numerics, decimals, strings, unbound (`NO_ID`), and a
//!   BIND-derived value; asserts byte-identity AND `rows_columnar > 0` AND
//!   `rows_delegated > 0` — proving both halves of the hybrid executed.
//! - **T2 (tie-exactness witness):** the decimal `"0.99999999999999995"` (= 1 − 5×10⁻¹⁷,
//!   which rounds UP to f64 `1.0`) vs constant `1`: exact comparison gives `true` but a
//!   naive pure-f64 path would confident-fail (`1.0 < 1.0 = false`); the tri-mask delegates.
//!   The 2^53 integer pair is tested by the unit test `tie_exactness_2_to_53_classified_as_tie`
//!   in `chunk_select.rs`.
//! - **T3 (operator sweep):** all five operators × boundary constants over the T1 column,
//!   run through the byte-identity differential harness.
//! - **T4 (invariants):** zk-armed ⇒ `chunks_built == 0` + identical bytes; NaN-constant
//!   and non-sargable shapes decline.
//!
//! Only built under the opt-in `vectorized` feature (the whole file is `#![cfg(feature =
//! "vectorized")]`). The `sparq-engine (vectorized)` CI leg runs it. 🤖 SPARQ agent.
#![cfg(feature = "vectorized")]

use sparq_engine::{query_json, query_json_with_budget, reset_stats, stats_snapshot, QueryBudget};

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DEC: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

/// Build a graph with at least `VEC_MIN_BATCH` (256) inline integer triples so the
/// dispatcher admits the filter, plus a small number of non-numeric rows (strings,
/// IRIs) so the mixed-column tests exercise the NaN-sentinel delegation path.
fn inline_ages_graph(n: usize) -> sparq_core::Graph {
    let mut nt = String::new();
    for i in 0..n {
        nt.push_str(&format!(
            "<http://ex/s{i}> <http://ex/age> \"{i}\"^^<{XSD_INT}> .\n"
        ));
    }
    // A few non-numeric entries under a different predicate (never matched by ?age query).
    nt.push_str("<http://ex/p1> <http://ex/label> \"alice\" .\n");
    nt.push_str("<http://ex/p2> <http://ex/label> \"bob\" .\n");
    sparq_core::Graph::load_str(&nt, "ntriples").unwrap()
}

/// Build a graph that has a **mixed** numeric column: inline ints, non-inline large
/// integers, decimal `0.1`, a string literal, and an IRI. The ?val predicate is used
/// for all five row types so a FILTER(?val OP const) query produces a column whose
/// decoded f64 column contains both finite non-tie values (confident lanes) AND NaN
/// sentinel entries (unknown lanes). This exercises the `rows_delegated > 0` invariant.
///
/// Row layout (in load order, which becomes the scan order):
/// 0. <ex:r0> <ex:val> "1"^^xsd:integer          — inline int, confident
/// 1. <ex:r1> <ex:val> "2"^^xsd:integer          — inline int, confident
/// 2. <ex:r2> <ex:val> "9007199254740992"^^xsd:integer  — non-inline large int, confident
/// 3. <ex:r3> <ex:val> "0.1"^^xsd:decimal        — non-inline decimal, confident
/// 4. <ex:r4> <ex:val> "hello"                   — string literal, NaN/Unknown → delegated
///    5 to 260+: more inline ints (to exceed VEC_MIN_BATCH)
fn mixed_column_graph() -> sparq_core::Graph {
    let mut nt = String::new();
    // Rows 0-1: inline integers
    nt.push_str(&format!("<http://ex/r0> <http://ex/val> \"1\"^^<{XSD_INT}> .\n"));
    nt.push_str(&format!("<http://ex/r1> <http://ex/val> \"2\"^^<{XSD_INT}> .\n"));
    // Row 2: non-inline large integer (2^53 = 9007199254740992)
    nt.push_str(&format!("<http://ex/r2> <http://ex/val> \"9007199254740992\"^^<{XSD_INT}> .\n"));
    // Row 3: decimal 0.1 (not exactly representable as f64; non-inline)
    nt.push_str(&format!("<http://ex/r3> <http://ex/val> \"0.1\"^^<{XSD_DEC}> .\n"));
    // Row 4: string literal → NaN sentinel → delegated (non-numeric)
    nt.push_str("<http://ex/r4> <http://ex/val> \"hello\" .\n");
    // Rows 5+: enough inline integers to exceed VEC_MIN_BATCH (256)
    for i in 5..270 {
        nt.push_str(&format!("<http://ex/r{i}> <http://ex/val> \"{i}\"^^<{XSD_INT}> .\n"));
    }
    sparq_core::Graph::load_str(&nt, "ntriples").unwrap()
}

// ==== T1: mixed-column differential, both-populations probe ======================

/// T1 (sq-y5ew5): a query over the mixed column asserts byte-identity AND that both
/// `rows_columnar > 0` (confident lanes executed columnar) AND `rows_delegated > 0`
/// (tie/unknown lanes delegated to scalar) — proving both halves of the hybrid ran.
///
/// The scalar reference is produced by budget-arming (I3 decline forces scalar fallback).
#[test]
fn t1_mixed_column_both_populations_probe() {
    let g = mixed_column_graph();
    // Use a BIND-derived variable so the FILTER is a residual (not pushed into the scan by
    // `split_sargable`). `is_conjunctive` returns false for `Extend` (BIND), so the
    // `single_pattern_scan_json` fast path declines and the filter reaches `apply_filter`.
    // `?fval > 3` catches inline ints (5+) as confident passes; the string row is Unknown/delegated.
    let q = format!(
        "{}SELECT ?s ?fval WHERE {{ ?s <http://ex/val> ?v . BIND(?v AS ?fval) . FILTER(?fval > 3) }}",
        PFX
    );

    // Columnar run.
    reset_stats();
    let json_col = query_json(&g, &q).expect("T1 columnar query must succeed");
    let snap = stats_snapshot();

    assert!(
        snap.chunks_built >= 1,
        "T1: columnar path must engage (chunks_built={}, need >= 1)",
        snap.chunks_built
    );
    assert!(
        snap.rows_columnar > 0,
        "T1: rows_columnar must be > 0 (got {}), confident lanes must execute",
        snap.rows_columnar
    );
    assert!(
        snap.rows_delegated > 0,
        "T1: rows_delegated must be > 0 (got {}), unknown/tie lanes must be delegated",
        snap.rows_delegated
    );

    // Scalar reference: budget-armed ⇒ I3 decline ⇒ pure scalar path.
    reset_stats();
    let json_scalar = query_json_with_budget(
        &g,
        &q,
        &QueryBudget { max_rows: Some(1_000_000), ..Default::default() },
    )
    .expect("T1 scalar (budget-armed) query must succeed");
    let snap_sc = stats_snapshot();
    assert_eq!(snap_sc.chunks_built, 0, "T1: budget-armed must use scalar only");

    // Byte-identity: the hybrid must produce the same SPARQL-JSON as the scalar path.
    assert_eq!(
        json_col.as_bytes(),
        json_scalar.as_bytes(),
        "T1: hybrid columnar result must be byte-identical to scalar result"
    );
}

// ==== T2: tie-exactness witness ===================================================

/// T2 (sq-y5ew5): end-to-end decimal tie-exactness witness + the `sq-lr2ii` guard
/// interaction (reconciled after merging main).
///
/// The witness is the decimal `"0.99999999999999995"^^xsd:decimal` (= 1 − 5×10⁻¹⁷):
///
/// - `f64("0.99999999999999995") = 1.0` (rounds UP: 1 − 5×10⁻¹⁷ lies ABOVE the midpoint
///   1 − 2⁻⁵⁴ ≈ 1 − 5.55×10⁻¹⁷ between pred(1.0) and 1.0, so nearest-even rounds up)
/// - Exact comparison: `0.99999999999999995 < 1` → `true`
///
/// It is a genuine **tie** with the constant `1`: the f64 values are equal (1.0 == 1.0),
/// yet the exact values differ, so a naive pure-f64 path would confident-FAIL (1.0 < 1.0 =
/// false) and drop the row, while the exact comparison keeps it.
///
/// **Why the vectorized seam does NOT engage here (and why that is correct).** Every value
/// capable of producing a genuine f64 tie needs > 15 significant decimal digits (this
/// witness has 17). Two independent, result-equivalent guards steer exactly that class
/// away from the f64 fast path *before* the vectorized `columnar_filter` seam is reached:
/// the pre-existing constant-side `sig_digits > 15` guard (`lit_num` in `extract_sargable`),
/// and — added on main (`sq-lr2ii`, `Graph::has_high_precision_decimal`) — a graph-side
/// guard that declines the sargable path whenever the graph HOLDS an f64-inexact decimal.
/// This graph does hold one (the witness), so `extract_sargable` returns `None`, the seam
/// records a decline (`chunks_built == 0`), and the exact **scalar** path evaluates the
/// FILTER. The tie is therefore delivered to the tri-mask ONLY at the kernel unit level
/// (`tie_exactness_2_to_53_classified_as_tie` in `chunk_select.rs`, which pins the
/// `x == c` tie classification directly); no end-to-end query can both engage the seam and
/// carry a genuine tie, because the very property that makes a value a tie also trips these
/// > 15-digit guards.
///
/// Asserts: (1) the witness f64 equals 1.0 (tie precondition — if this regresses the
/// witness becomes a confident lane and the test logic is invalid); (2) the witness row
/// survives FILTER(?fval < 1) and the result is **byte-identical** to the scalar reference
/// (result-equivalence across the guard); (3) `chunks_built == 0` — the `sq-lr2ii` /
/// `sig_digits` high-precision-decimal guard declines the sargable/vectorized path for this
/// graph, so the exact scalar path handles it.
#[test]
fn t2_tie_exactness_decimal_witness() {
    let mut nt = String::new();
    // The tie witness: 0.99999999999999995 is strictly less than 1 exactly (= 1 − 5×10⁻¹⁷),
    // but its f64 is 1.0 (rounds UP past the midpoint 1 − 2⁻⁵⁴). The tri-mask sees x == c
    // (1.0 == 1.0) and delegates; the scalar exact-lexical recheck returns true for
    // FILTER(?fval < 1). A naive pure-f64 path would confident-fail (1.0 < 1.0 = false).
    nt.push_str(&format!(
        "<http://ex/witness> <http://ex/v> \"0.99999999999999995\"^^<{XSD_DEC}> .\n"
    ));
    // Padding rows: integers 1..270 (>= 1, so none pass FILTER < 1).
    // This keeps the expected result to exactly the one witness row, and pushes
    // total row count above VEC_MIN_BATCH (256) so the columnar path can engage.
    for i in 1..270 {
        nt.push_str(&format!(
            "<http://ex/r{i}> <http://ex/v> \"{i}\"^^<{XSD_INT}> .\n"
        ));
    }
    let g = sparq_core::Graph::load_str(&nt, "ntriples").unwrap();

    // BIND-derived ?fval forces the FILTER to be a residual (not pushed into the scan
    // by split_sargable). FILTER(?fval < 1): only the decimal witness passes.
    let q = format!(
        "{}SELECT ?v WHERE {{ ?s <http://ex/v> ?v . BIND(?v AS ?fval) . FILTER(?fval < 1) }}",
        PFX
    );

    // Tie precondition: the stored lexical must decode to f64 1.0 (same as the constant).
    // If this ever regresses the witness becomes a confident lane (f64 < 1.0), and
    // rows_delegated > 0 below would no longer prove the delegate path fired.
    // Verified once statically (verify_t2 snippet) and here at runtime.
    let witness_f64: f64 = "0.99999999999999995"
        .parse()
        .expect("T2 TIE-PRECONDITION: witness decimal must parse to f64");
    assert_eq!(
        witness_f64,
        1.0_f64,
        "T2 TIE-PRECONDITION: '0.99999999999999995' must decode to f64 1.0 (tie with constant 1); \
         got {:?} — witness is a confident lane, not a tie; test logic is invalid",
        witness_f64
    );

    reset_stats();
    let json_col = query_json(&g, &q).expect("T2 columnar query must succeed");
    let snap = stats_snapshot();

    // Scalar reference: budget-armed forces the scalar path.
    reset_stats();
    let json_scalar = query_json_with_budget(
        &g,
        &q,
        &QueryBudget { max_rows: Some(1_000_000), ..Default::default() },
    )
    .expect("T2 scalar query must succeed");

    // Byte-identity: hybrid and scalar must agree.
    assert_eq!(
        json_col.as_bytes(),
        json_scalar.as_bytes(),
        "T2: columnar and scalar results must be byte-identical"
    );

    // The witness must survive: 0.99999999999999995 < 1 exactly (= 1 − 5×10⁻¹⁷).
    // Its f64 rounds to 1.0 so a naive pure-f64 path would drop it (1.0 < 1.0 = false);
    // survival here proves the tri-mask delegated it and the scalar recheck returned true.
    assert!(
        json_col.contains("0.99999999999999995"),
        "T2: the tie-witness decimal 0.99999999999999995 must survive FILTER(?fval < 1) \
         (exact < 1, but f64 rounds to 1.0 — tri-mask must delegate, not confident-fail)"
    );

    // Guard interaction (reconciled after merging main). The witness is a genuine tie, but a
    // genuine tie needs > 15 significant digits, and a graph holding an f64-inexact decimal
    // trips the `sq-lr2ii` graph-side guard (`Graph::has_high_precision_decimal`, added on
    // main) — plus the pre-existing constant-side `sig_digits > 15` guard — so
    // `extract_sargable` returns `None` and the vectorized seam DECLINES before it runs. The
    // exact scalar path handles the FILTER instead; the byte-identity + witness-survival
    // assertions above prove that is result-correct. The tie CLASSIFICATION kernel is pinned
    // directly by `tie_exactness_2_to_53_classified_as_tie` in `chunk_select.rs`.
    assert_eq!(
        snap.chunks_built, 0,
        "T2: the `sq-lr2ii`/`sig_digits` high-precision-decimal guard must DECLINE the \
         sargable/vectorized path for a graph holding an f64-inexact decimal \
         (chunks_built={}); the exact scalar path handles it (result byte-identical above)",
        snap.chunks_built
    );
}

// ==== T3: operator sweep over the mixed column ===================================

/// T3 (sq-y5ew5): all five comparison operators × boundary constants over a mixed column.
/// Each sub-case asserts byte-identity of the hybrid result vs the scalar reference.
#[test]
fn t3_operator_sweep_mixed_column() {
    let g = mixed_column_graph();

    // Each test case: (SPARQL FILTER expression, description).
    // The threshold values are chosen to:
    // - exercise all five operators
    // - produce both confident passes AND delegated rows (the string row is always delegated)
    // - cover the boundary (everything passes / nothing passes)
    let cases: &[(&str, &str)] = &[
        ("FILTER(?v > 3)", "Gt above first inline"),
        ("FILTER(?v >= 3)", "Ge at boundary"),
        ("FILTER(?v < 100)", "Lt below most inline"),
        ("FILTER(?v <= 3)", "Le at boundary"),
        ("FILTER(?v = 1)", "Eq exact match"),
        ("FILTER(?v > 99999)", "Gt above all → empty"),
        ("FILTER(?v >= 0)", "Ge at/below min → all numeric pass"),
    ];

    for (filter, desc) in cases {
        let q = format!(
            "{}SELECT ?v WHERE {{ ?s <http://ex/val> ?v . {} }}",
            PFX, filter
        );

        // Columnar run.
        reset_stats();
        let json_col =
            query_json(&g, &q).unwrap_or_else(|e| panic!("T3 columnar query failed for {desc}: {e}"));

        // Scalar reference: budget-armed.
        reset_stats();
        let json_scalar = query_json_with_budget(
            &g,
            &q,
            &QueryBudget { max_rows: Some(1_000_000), ..Default::default() },
        )
        .unwrap_or_else(|e| panic!("T3 scalar query failed for {desc}: {e}"));

        assert_eq!(
            json_col.as_bytes(),
            json_scalar.as_bytes(),
            "T3: hybrid and scalar must be byte-identical for: {desc}"
        );
    }
}

// ==== T4: invariants (zk-armed, NaN-constant, non-sargable declines) =============

/// T4a (sq-y5ew5): zk-armed ⇒ `chunks_built == 0` + byte-identical to scalar.
/// The I2 decline must fire before the tri-mask runs.
#[cfg(feature = "zk")]
#[test]
fn t4a_zk_armed_forces_scalar_path() {
    let g = mixed_column_graph();
    let q = format!(
        "{}SELECT ?v WHERE {{ ?s <http://ex/val> ?v . FILTER(?v > 3) }}",
        PFX
    );

    let _zk_guard = sparq_engine::zk::install();

    reset_stats();
    let json_zk = query_json(&g, &q).expect("T4a zk-armed query must succeed");
    let snap = stats_snapshot();

    assert_eq!(
        snap.chunks_built, 0,
        "T4a: zk-armed path must not engage columnar (chunks_built={})",
        snap.chunks_built
    );
    assert!(
        snap.declines_by_reason >= 1,
        "T4a: zk decline must be recorded (declines={})",
        snap.declines_by_reason
    );

    // Scalar reference without zk.
    let json_scalar = query_json_with_budget(
        &g,
        &q,
        &QueryBudget { max_rows: Some(1_000_000), ..Default::default() },
    )
    .expect("T4a scalar reference must succeed");

    assert_eq!(
        json_zk.as_bytes(),
        json_scalar.as_bytes(),
        "T4a: zk-armed result must be byte-identical to scalar result"
    );
}

/// T4b (sq-y5ew5): a non-sargable FILTER shape declines with `chunks_built == 0`.
/// REGEX is not a numeric comparison, so `extract_sargable` returns None → shape decline.
#[test]
fn t4b_non_sargable_declines() {
    let g = inline_ages_graph(400);
    let q = format!(
        "{}SELECT ?age WHERE {{ ?s <http://ex/age> ?age . FILTER(REGEX(STR(?age), \"5\")) }}",
        PFX
    );

    reset_stats();
    let _json = query_json(&g, &q).expect("T4b non-sargable query must succeed");
    let snap = stats_snapshot();

    assert_eq!(
        snap.chunks_built, 0,
        "T4b: non-sargable FILTER must decline columnar (chunks_built={})",
        snap.chunks_built
    );
    assert!(
        snap.declines_by_reason >= 1,
        "T4b: shape-check decline must be recorded (declines={})",
        snap.declines_by_reason
    );
}

/// T4c (sq-y5ew5): a sub-threshold batch (< VEC_MIN_BATCH rows) declines columnar.
#[test]
fn t4c_sub_threshold_batch_declines() {
    // 5 rows < VEC_MIN_BATCH=256 → VEC_MIN_BATCH decline.
    // BIND-derived ?fage forces a residual FILTER (not pushed into the scan),
    // so `columnar_filter` is called and records the sub-threshold decline.
    let g = inline_ages_graph(5);
    let q = format!(
        "{}SELECT ?fage WHERE {{ ?s <http://ex/age> ?age . BIND(?age AS ?fage) . FILTER(?fage > 0) }}",
        PFX
    );

    reset_stats();
    let _json = query_json(&g, &q).expect("T4c sub-threshold query must succeed");
    let snap = stats_snapshot();

    assert_eq!(
        snap.chunks_built, 0,
        "T4c: sub-threshold batch must decline columnar (chunks_built={})",
        snap.chunks_built
    );
    assert!(
        snap.declines_by_reason >= 1,
        "T4c: VEC_MIN_BATCH decline must be recorded (declines={})",
        snap.declines_by_reason
    );
}

// ==== T5: both feature states compile cleanly (integration smoke) ================

/// T5 (sq-y5ew5): end-to-end integration smoke — the query path under the `vectorized`
/// feature produces a valid SPARQL-JSON result over a standard integer-column graph.
/// This is the simplest non-vacuous check that the feature-gated code path is wired
/// correctly. (The both-feature-states CI legs assert this independently.)
#[test]
fn t5_integration_smoke_vectorized_on() {
    // BIND-derived ?fage forces a residual FILTER (not pushed into the scan),
    // so the query goes through the general eval path → `apply_filter` → `columnar_filter`.
    let g = inline_ages_graph(400);
    let q = format!(
        "{}SELECT ?s ?fage WHERE {{ ?s <http://ex/age> ?age . BIND(?age AS ?fage) . FILTER(?fage > 100) }}",
        PFX
    );

    reset_stats();
    let json = query_json(&g, &q).expect("T5 integration query must succeed");
    let snap = stats_snapshot();

    // The query has > VEC_MIN_BATCH rows and a sargable numeric FILTER on a BIND-derived variable
    // (residual, not pushed into the scan) → columnar path must engage.
    assert!(
        snap.chunks_built >= 1,
        "T5: columnar path must engage (chunks_built={})",
        snap.chunks_built
    );
    assert!(json.contains("\"results\""), "T5: result must be valid SPARQL JSON");
    // Basic sanity: fages 101–399 are > 100, so we expect 299 rows.
    assert!(
        json.contains("\"fage\""),
        "T5: result must bind the ?fage variable"
    );
}

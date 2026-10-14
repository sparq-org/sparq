//! Regression: `ORDER BY … LIMIT 0` must not panic. (sq-7d3dj.30.3)
//!
//! The bounded-select top-k ORDER BY path (bead sq-7d3dj.30.2) computes a row
//! budget `k = OFFSET + LIMIT` and, for `k` within the threshold, runs
//! `select_nth_unstable_by(k - 1)`, which requires `k >= 1`. A bare `LIMIT 0`
//! (`k = 0`) panicked (debug) / underflowed (release). Surfaced by the W3C
//! `sparql10/solution-seq` "Limit 3" (slice-03) conformance test; `try_topk_orderby`
//! now declines for `k = 0` so the full path slices to an empty result.
//!
//! (Kept in a dedicated file, disjoint from the sq-7d3dj.30.2 `topk_orderby.rs`.)

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX : <http://example.org/ns#>\n";

fn g() -> Graph {
    Graph::load_str(
        &format!("{PFX}:x :num 1 . :x :num 2 . :x :num 3 . :y :num 1 ."),
        "turtle",
    )
    .unwrap()
}

#[test]
fn order_by_limit_zero_is_empty_not_panic() {
    let r = query(&g(), &format!("{PFX}SELECT ?v WHERE {{ [] :num ?v }} ORDER BY ?v LIMIT 0"))
        .expect("LIMIT 0 must evaluate, not panic");
    assert!(r.rows.is_empty(), "LIMIT 0 must return zero rows");
}

#[test]
fn order_by_offset_limit_zero_is_empty() {
    // OFFSET 2 LIMIT 0 → k = 2 (> 0, no panic), then the [2..2] slice is empty.
    let r = query(&g(), &format!("{PFX}SELECT ?v WHERE {{ [] :num ?v }} ORDER BY ?v OFFSET 2 LIMIT 0"))
        .expect("OFFSET 2 LIMIT 0 must evaluate");
    assert!(r.rows.is_empty());
}

#[test]
fn order_by_limit_one_still_works() {
    // The smallest non-degenerate top-k budget (k = 1) still returns the min row.
    let r = query(&g(), &format!("{PFX}SELECT ?v WHERE {{ [] :num ?v }} ORDER BY ?v LIMIT 1"))
        .expect("LIMIT 1 must evaluate");
    assert_eq!(r.rows.len(), 1, "LIMIT 1 returns exactly one row");
}

//! (sq-pntvh.1) M4 **Phase 1** native micro-benchmark for the vectorized
//! (columnar / vector-at-a-time) FILTER + aggregate hot paths, over a synthetic numeric
//! column — the perf BASELINE every later M4 phase measures its speedup against
//! (`research/vector-at-a-time-m4.md` §5.1, §6 Phase 1).
//!
//! 🤖 SPARQ agent. Prints `metric_us` lines (min-of-`iters` microseconds) so a harness can
//! scrape them, matching the repo's established bench idiom (`sparq-algos`'
//! `examples/bench_algos.rs`, `sparq-vectors`' `examples/bench_vectors.rs`) — NOT the
//! criterion crate, which the root workspace and its production crates avoid (the only
//! criterion deps live in the isolated, standalone `bench/*` workspaces, e.g.
//! `bench/zk/Cargo.toml`), against the "lean, dependency-light core" constraint. Zero new
//! dependencies; the corpus is generated in-process (no external dataset).
//!
//! ## Honest Phase-1 semantics (no win is claimed here)
//!
//! The landed `select_numeric` kernel is a correctness SCAFFOLD, not yet a SIMD win: it
//! re-gathers `Graph::numeric_value(id)` per element — the SAME random-access pattern as
//! the row path (`research/vector-at-a-time-m4.md` §0 refinement 2). So the `*_row` and
//! `*_columnar_kernel` lines below are expected to be CLOSE at Phase 1; the point of this
//! bench is to fix the baseline so Phase 2's decode-to-contiguous-`Vec<f64>` primitive and
//! the Phase 3/4 columnar operators show up as a measurable delta against it. No number
//! from this bench is canonical (work-box timings are non-canonical) — it is a relative
//! micro-measurement, run on a quiet host per `bench/benchmarks.toml`.
//!
//! Run: `cargo run -p sparq-engine --release --example bench_vectorized --features
//! vectorized -- [N_ROWS] [ITERS]` — two optional POSITIONAL numeric args (not `key=value`);
//! defaults are `N_ROWS=200000` and `ITERS=7`.

use std::hint::black_box;
use std::time::Instant;

use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_engine::{query, DataChunk, VecCmp};

/// Min-of-`iters` wall time (microseconds) of `f`, printed as a `metric_us` line. The
/// closure returns an accumulator that is `black_box`ed so the loop is not optimised away.
fn best<T>(label: &str, iters: u32, f: impl Fn() -> T) {
    let mut lo = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = f();
        let dt = t.elapsed().as_micros();
        black_box(out);
        lo = lo.min(dt);
    }
    println!("metric_us {}={}", label, lo);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200_000);
    // Clamp to ≥1: `ITERS=0` would leave `best()`'s min-fold at `u128::MAX`, printing a
    // nonsense (huge) timing instead of running the closure at all.
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(7).max(1);

    // Synthetic integer-age graph: `(ex:s{i}, ex:age, i)` for i in 0..n.
    let mut nt = String::with_capacity(n * 48);
    for i in 0..n {
        nt.push_str(&format!(
            "<http://ex/s{}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            i, i
        ));
    }
    let t = Instant::now();
    let g = Graph::load_str(&nt, "ntriples").expect("parse");
    println!("metric_us load={}", t.elapsed().as_micros());

    // The object id column, exactly the batch a columnar FILTER/aggregate would receive.
    let col: Vec<Id> = query(&g, "SELECT ?o WHERE { ?s <http://ex/age> ?o }")
        .expect("query")
        .rows
        .iter()
        .map(|r| g.id_of(r[0].as_ref().unwrap()).unwrap())
        .collect();
    let chunk = DataChunk::from_columns(vec![col.clone()], col.len()).expect("uniform column");
    println!("rows={}", col.len());

    // Threshold ~half the range so the FILTER is genuinely selective (both branches hit).
    let t = (n as f64) / 2.0;

    // ---- FILTER: row per-element predicate vs the columnar select_numeric kernel ----
    best("filter_row", iters, || {
        let mut kept = 0usize;
        for &id in &col {
            if g.numeric_value(id).is_some_and(|x| x > t) {
                kept += 1;
            }
        }
        kept
    });
    best("filter_columnar_kernel", iters, || {
        chunk.select_numeric(&g, 0, VecCmp::Gt(t)).len()
    });
    // The full columnar FILTER: compare kernel + gather (what a residual-FILTER seam runs).
    best("filter_columnar_kernel_gather", iters, || {
        let sel = chunk.select_numeric(&g, 0, VecCmp::Gt(t));
        chunk.apply_selection(&sel).len()
    });

    // ---- AGGREGATE reducers over the same column: the row per-element reduction ----
    best("agg_sum_row", iters, || {
        let mut s = 0.0f64;
        for &id in &col {
            if let Some(x) = g.numeric_value(id) {
                s += x;
            }
        }
        s
    });
    best("agg_count_row", iters, || {
        col.iter().filter(|&&id| g.numeric_value(id).is_some()).count()
    });
    best("agg_min_row", iters, || {
        col.iter()
            .filter_map(|&id| g.numeric_value(id))
            .fold(f64::INFINITY, f64::min)
    });
    best("agg_max_row", iters, || {
        col.iter()
            .filter_map(|&id| g.numeric_value(id))
            .fold(f64::NEG_INFINITY, f64::max)
    });
}

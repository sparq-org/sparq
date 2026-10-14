//! sq-s2nob: EL transitive-reduction (Hasse) scale micro-benchmark — a self-asserting,
//! deterministic workload for Phase E3. Run (the `hasse` feature is required):
//!
//!     cargo run -p sparq-reason-el --features hasse --example hasse_bench --release [N]
//!
//! No dataset to fetch: the TBoxes are generated in-process (a pure function of N, byte-
//! deterministic, no RNG/network/toolchain), so this is a hermetic, regenerable benchmark.
//!
//! Two workloads, each over the SAME public `Classifier::classify` + `DirectHierarchy` API:
//!
//!   1. `chain` — a depth-N `rdfs:subClassOf` chain. The FULL closure has N·(N+1)/2 edges; the
//!      transitive reduction (Hasse diagram) has exactly N. This is the headline E3 win: a deep
//!      taxonomy's hierarchy is LINEAR in edges, not quadratic. The closed forms for both counts
//!      are asserted — a reduction regression (a redundant edge kept, or a real edge dropped)
//!      fails LOUDLY here.
//!
//!   2. `binary-tree` — a balanced binary subclass tree of N leaves (each node ⊑ its parent).
//!      The Hasse diagram is the tree itself (#nodes − 1 edges); the closure is much larger
//!      (each leaf gains log₂N supers). Asserts the tree's Hasse edge count = total nodes − 1.
//!
//! Wall-clock seconds are TREND-ONLY (quiet-box sensitive — a contended box inflates them). The
//! STRUCTURAL assertions (closure-size + Hasse-size closed forms) are the hard, deterministic,
//! load-robust CORRECTNESS gate — exactly the E3 acceptance discipline (subsumption COUNTS are
//! the canonical assertion; timings are advisory only, spike §5 E3).

use sparq_core::dict::{Dict, Id};
use sparq_reason_el::{Classifier, DirectHierarchy};
use std::time::Instant;

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const EX: &str = "http://sparq.dev/bench/el-hasse#";

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// Depth-N chain `C0 ⊑ C1 ⊑ … ⊑ CN`. Closure: N·(N+1)/2 edges. Hasse: N edges.
fn run_chain(n: usize) {
    let mut dict = Dict::new();
    let sc = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let classes: Vec<Id> = (0..=n)
        .map(|i| dict.intern_iri(&format!("{EX}C{i}")))
        .collect();
    let triples: Vec<[Id; 3]> = (0..n).map(|i| [classes[i], sc, classes[i + 1]]).collect();

    let t = Instant::now();
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    let dt = secs(t);

    let closure_edges: usize = (0..=n)
        .map(|i| closure.super_classes(classes[i]).len())
        .sum();
    let hasse_edges = direct.direct_edge_count();
    assert_eq!(
        closure_edges,
        n * (n + 1) / 2,
        "chain N {n}: closure edges {closure_edges} != N·(N+1)/2"
    );
    assert_eq!(
        hasse_edges, n,
        "chain N {n}: Hasse edges {hasse_edges} != N (reduction must be linear)"
    );
    println!(
        "chain        N {n:>7}: classify+reduce {dt:9.6}s   closure_edges {closure_edges}  hasse_edges {hasse_edges} (= N, asserted)"
    );
}

/// A balanced binary subclass tree of `levels` levels (2^levels − 1 nodes): node i ⊑ node (i-1)/2.
/// Hasse edges = nodes − 1 (every non-root node has its parent as sole direct super).
fn run_binary_tree(levels: u32) {
    let nodes = (1usize << levels) - 1;
    let mut dict = Dict::new();
    let sc = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let ids: Vec<Id> = (0..nodes)
        .map(|i| dict.intern_iri(&format!("{EX}T{i}")))
        .collect();
    // node i (i ≥ 1) ⊑ parent (i-1)/2.
    let triples: Vec<[Id; 3]> = (1..nodes).map(|i| [ids[i], sc, ids[(i - 1) / 2]]).collect();

    let t = Instant::now();
    let closure = Classifier::classify(&dict, &triples);
    let direct = DirectHierarchy::from_closure(&closure);
    let dt = secs(t);

    let hasse_edges = direct.direct_edge_count();
    assert_eq!(
        hasse_edges,
        nodes - 1,
        "binary-tree levels {levels}: Hasse edges {hasse_edges} != nodes-1 ({})",
        nodes - 1
    );
    // Spot-check: every non-root node's sole direct super is its parent.
    for i in 1..nodes {
        assert_eq!(
            direct.direct_super_classes(ids[i]),
            &[ids[(i - 1) / 2]][..],
            "T{i}'s sole direct super is its parent"
        );
    }
    println!(
        "binary-tree  nodes {nodes:>5}: classify+reduce {dt:9.6}s   hasse_edges {hasse_edges} (= nodes-1, asserted)"
    );
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    println!("sparq-reason-el hasse_bench — chain N {n} (deterministic, regenerable)");
    run_chain(n);
    // A 14-level tree is 16_383 nodes — enough to exercise the reduction without ballooning runtime.
    run_binary_tree(14);
    println!("OK: structural closure-size + Hasse-size assertions held (wall-clock above is trend-only).");
}

//! sq-q0o82 (E4 follow-up) — PARALLEL-SATURATION PHASE ATTRIBUTION bench.
//!
//! The E4 increment (sq-wy3i6) parallelises only the COMPUTE phase of the bulk-synchronous
//! saturation round — the membership-triggered rule derivation against the round-start
//! snapshot. The APPLY phase (`add` / `add_link`, and under `rbox` the CR10/CR11 closure
//! inside `add_link_rbox`) is still SEQUENTIAL. Whether that matters is an empirical
//! question, and parallelising apply would put the identical-closure invariant at risk, so
//! the refinement is gated on a measurement rather than a guess. This example is that
//! measurement: it is also the first in-repo CONSUMER of `classify_graph_par`, exercising
//! the thread-count knob end to end.
//!
//! Run:
//!
//!     cargo run -p sparq-reason-el --features par --release --example par_phase_bench [SCALE]
//!     cargo run -p sparq-reason-el --features par,rbox --release --example par_phase_bench -- <ontology.nt> ntriples
//!
//! TWO MODES
//! ---------
//!   * SYNTHETIC (no args, or a numeric SCALE): classify a generated wide-taxonomy +
//!     existential-traversal TBox — a shape whose saturation rounds carry a WIDE frontier, so
//!     the worker pool actually engages — at several thread counts, and ASSERT the two
//!     invariants that make the measurement trustworthy:
//!
//!       1. CLOSURE UNCHANGED — the emitted triples and `Report` at every thread count are
//!          bit-identical to the sequential `classify_graph`. (`tests/par_differential.rs`
//!          pins this over the fixture corpus; re-asserting it here means a phase number is
//!          never reported for a run that silently derived something else.)
//!       2. WORK COUNTS THREAD-INVARIANT — `rounds`, `frontier_items`, `derived_members` and
//!          `derived_links` are identical at every thread count. Chunking decides WHICH
//!          worker derives a conclusion, never WHICH conclusions are derived, so a drift here
//!          means a partitioning bug, not a scheduling artefact.
//!
//!     Exits 0 iff both hold, so it is a non-vacuous acceptance run, not just a printout.
//!
//!   * GATHER (`<path> [format]`, `format` defaults to `turtle`): classify an arbitrary
//!     ontology converted to RDF — a riot-converted GO / OpenGALEN / SNOMED-scale dump, as
//!     gathered by `scripts/bench/reason-el-same-box.sh` — and print the same phase row with
//!     NO assertion. This is the row the "does the apply phase dominate on a REAL ontology?"
//!     decision is made on; it needs an ontology this repo does not vendor.
//!
//! READING THE OUTPUT: `apply_frac` is the decision metric — the share of measured saturation
//! time spent in the sequential apply phase. Near 1 means compute-only parallelism is
//! Amdahl-bound and a sharded/work-stealing apply is worth its determinism cost; small means
//! it is not. Absolute seconds are TREND-ONLY: work-box wall-clock is non-canonical (a
//! contended runner inflates both phases), which is exactly why the ratio is reported and no
//! timing figure from this bench belongs in documentation.

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{classify_graph, classify_graph_par_stats, ParPhaseStats};
use std::num::NonZeroUsize;

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_ON_PROPERTY: &str = "http://www.w3.org/2002/07/owl#onProperty";
const OWL_SOME_VALUES_FROM: &str = "http://www.w3.org/2002/07/owl#someValuesFrom";
const EX: &str = "http://sparq.dev/bench/el-par#";

/// Thread counts the synthetic mode reports (and cross-checks for invariance): the
/// through-the-round-loop sequential case, the smallest genuinely concurrent pool, and an
/// oversubscribed pool so chunk boundaries land in different places.
const THREAD_COUNTS: [usize; 3] = [1, 2, 4];

/// Mid-level "branch" concepts the leaves hang off — fixed across scales so the taxonomy stays
/// wide and shallow, like a biomedical sub-hierarchy slice.
const BRANCHES: usize = 16;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("thread count is non-zero")
}

/// A wide taxonomy with existential traversal, sized by `leaves`:
///
///   * `L_k ⊑ M_{k mod BRANCHES}` and `M_b ⊑ Root` — a wide, shallow is-a forest, so every
///     saturation round drains a frontier of thousands of memberships (the regime where
///     partitioning across workers is meaningful at all).
///   * `L_k ⊑ ∃r.M_{k mod BRANCHES}` plus the CR4 back-propagation axiom `∃r.Root ⊑ Marked` —
///     every leaf then derives `L_k ⊑ Marked` THROUGH its r-successor, which is the
///     RL-unreachable consequence, and, for this bench's purpose, the shape that drives real
///     link-insertion work (`add_link`'s live-`S(f)` scan) into the sequential apply phase.
fn wide_tbox(dict: &mut Dict, leaves: usize) -> Vec<[Id; 3]> {
    let sc = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let on_prop = dict.intern_iri(OWL_ON_PROPERTY);
    let some_from = dict.intern_iri(OWL_SOME_VALUES_FROM);
    let r = dict.intern_iri(&format!("{EX}r"));
    let root = dict.intern_iri(&format!("{EX}Root"));
    let marked = dict.intern_iri(&format!("{EX}Marked"));
    let mids: Vec<Id> = (0..BRANCHES)
        .map(|b| dict.intern_iri(&format!("{EX}M{b}")))
        .collect();

    let mut t = Vec::with_capacity(leaves * 4 + BRANCHES + 3);
    for &m in &mids {
        t.push([m, sc, root]);
    }
    for k in 0..leaves {
        let l = dict.intern_iri(&format!("{EX}L{k}"));
        let m = mids[k % BRANCHES];
        t.push([l, sc, m]);
        // L_k ⊑ ∃r.M_{k mod BRANCHES} (one restriction node per leaf).
        let restr = dict.intern_iri(&format!("{EX}__restr_{k}"));
        t.push([l, sc, restr]);
        t.push([restr, on_prop, r]);
        t.push([restr, some_from, m]);
    }
    // ∃r.Root ⊑ Marked — the CR4 back-propagation axiom every leaf reaches through its link.
    let restr_root = dict.intern_iri(&format!("{EX}__restr_root"));
    t.push([restr_root, on_prop, r]);
    t.push([restr_root, some_from, root]);
    t.push([restr_root, sc, marked]);
    t
}

/// One phase row. `stats` counts are deterministic; the seconds are trend-only.
fn print_row(label: &str, threads: usize, stats: &ParPhaseStats, emitted: usize) {
    let compute_s = stats.compute_nanos as f64 / 1e9;
    let apply_s = stats.apply_nanos as f64 / 1e9;
    println!(
        "{label:<10} threads={threads} rounds={} frontier_items={} derived_members={} \
         derived_links={} emitted={emitted} compute_s={compute_s:.6} apply_s={apply_s:.6} \
         apply_frac={:.4}",
        stats.rounds,
        stats.frontier_items,
        stats.derived_members,
        stats.derived_links,
        stats.apply_fraction()
    );
}

/// SYNTHETIC mode: measure the phase split at each thread count and assert the closure and
/// the work counts are thread-count invariant.
fn run_synthetic(leaves: usize) {
    let mut dict = Dict::new();
    let triples = wide_tbox(&mut dict, leaves);
    println!(
        "sparq-reason-el par_phase_bench — synthetic wide taxonomy: leaves={leaves} \
         branches={BRANCHES} input_triples={}",
        triples.len()
    );

    // Sequential reference: the closure every parallel run must reproduce exactly.
    let (mut dict_seq, mut triples_seq) = (dict.clone(), triples.clone());
    let report_seq = classify_graph(&mut dict_seq, &mut triples_seq);
    assert!(
        report_seq.emitted_subsumptions > 0,
        "the workload must derive something — a vacuous bench measures nothing"
    );

    let mut reference: Option<ParPhaseStats> = None;
    for threads in THREAD_COUNTS {
        let (mut dict_par, mut triples_par) = (dict.clone(), triples.clone());
        let (report_par, stats) = classify_graph_par_stats(&mut dict_par, &mut triples_par, nz(threads));

        // (1) closure unchanged — never report a phase split for a run that derived something else.
        assert_eq!(
            report_par, report_seq,
            "threads={threads}: Report differs from the sequential classify_graph"
        );
        assert_eq!(
            triples_par, triples_seq,
            "threads={threads}: emitted triples differ from the sequential classify_graph"
        );

        // (2) work counts thread-invariant — chunking must not change WHAT is derived.
        match &reference {
            None => reference = Some(stats),
            Some(r) => {
                assert_eq!(
                    (r.rounds, r.frontier_items, r.derived_members, r.derived_links),
                    (
                        stats.rounds,
                        stats.frontier_items,
                        stats.derived_members,
                        stats.derived_links
                    ),
                    "threads={threads}: derivation work counts drifted from the threads={} \
                     run (a partitioning bug, not a scheduling artefact)",
                    THREAD_COUNTS[0]
                );
            }
        }

        print_row("synthetic", threads, &stats, report_par.emitted_subsumptions);
    }
    println!(
        "OK: closure identical to sequential and derivation work counts thread-invariant at \
         {THREAD_COUNTS:?}. Seconds above are trend-only (work-box wall-clock is non-canonical); \
         apply_frac is the decision metric for the parallel-apply refinement."
    );
}

/// GATHER mode: phase row for a real ontology file. No assertion — real-ontology counts are
/// recorded by the same-box harness, never baked in here.
fn run_gather(path: &str, format: &str) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read ontology {path}: {e}"));
    let (dict, triples) = Graph::parse_to_triples(&text, format)
        .unwrap_or_else(|e| panic!("cannot parse {path} as {format}: {e}"));
    println!(
        "sparq-reason-el par_phase_bench — gather: {path} ({format}) input_triples={}",
        triples.len()
    );
    for threads in THREAD_COUNTS {
        let (mut dict_par, mut triples_par) = (dict.clone(), triples.clone());
        let (report, stats) = classify_graph_par_stats(&mut dict_par, &mut triples_par, nz(threads));
        print_row("gather", threads, &stats, report.emitted_subsumptions);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_synthetic(20_000),
        Some(a) if a.parse::<usize>().is_ok() => {
            run_synthetic(a.parse().expect("checked parseable"))
        }
        Some(path) => {
            let format = args.get(1).map(String::as_str).unwrap_or("turtle");
            run_gather(path, format);
        }
    }
}

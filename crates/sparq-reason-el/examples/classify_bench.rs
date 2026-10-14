//! sq-evb1: EL classification micro-benchmark — a self-asserting, deterministic
//! workload for the OWL 2 EL consequence-based classifier. Run:
//!
//!     cargo run -p sparq-reason-el --example classify_bench --release [DEPTH]
//!
//! No dataset to fetch: the TBox is generated in-process (a pure function of DEPTH, byte-
//! deterministic, no RNG/network/toolchain), so this is a hermetic, regenerable benchmark.
//!
//! Two workloads, each over the SAME public `Classifier::classify` API:
//!
//!   1. `chain` — a DEPTH-deep `rdfs:subClassOf` chain (C0 ⊑ C1 ⊑ … ⊑ C_DEPTH). The complete
//!      transitive subsumption lattice has a CLOSED FORM: class C_i gains (DEPTH − i) named
//!      super-classes, so the total derived-subsumption count is DEPTH·(DEPTH+1)/2. This is the
//!      taxonomy-classification workload where a transitive-closure that materialised naively
//!      blows up O(N^2); the saturator's worklist keeps each membership processed once.
//!
//!   2. `cr4` — the existential-traversal workload OWL 2 RL CANNOT reach (spike §1.2). A
//!      DEPTH-long ∃-successor chain `A_i ⊑ ∃r.A_{i+1}`, `A_DEPTH ⊑ B`, and a back-propagation
//!      axiom `∃r.B ⊑ B` (CR4): every A_i then derives `A_i ⊑ B` THROUGH its r-successor. The
//!      closed form is exactly DEPTH+1 derived `A_i ⊑ B` subsumptions — none of which RL can
//!      produce, because no RL rule reasons through an existential successor.
//!
//! Wall-clock seconds are TREND-ONLY (quiet-box sensitive — print the engine-internal timer's
//! `in Xs` ratio, not an absolute number, on a contended box). The STRUCTURAL assertions
//! (closure-size closed forms) are the hard, deterministic, load-robust CORRECTNESS gate: a
//! classifier regression — silent under-derivation a single query would miss — fails LOUDLY
//! here, exactly like bench/deep-taxonomy and bench/owl-sameas do for the RL reasoner.

use sparq_core::dict::{Dict, Id};
use sparq_reason_el::Classifier;
use std::time::Instant;

const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const OWL_ON_PROPERTY: &str = "http://www.w3.org/2002/07/owl#onProperty";
const OWL_SOME_VALUES_FROM: &str = "http://www.w3.org/2002/07/owl#someValuesFrom";
const EX: &str = "http://sparq.dev/bench/el#";

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// Build a DEPTH-deep `rdfs:subClassOf` chain `C0 ⊑ C1 ⊑ … ⊑ C_DEPTH` as raw triples.
/// Complete transitive closure has DEPTH·(DEPTH+1)/2 derived subsumptions.
fn chain_tbox(dict: &mut Dict, depth: usize) -> Vec<[Id; 3]> {
    let sc = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let classes: Vec<Id> = (0..=depth)
        .map(|i| dict.intern_iri(&format!("{EX}C{i}")))
        .collect();
    (0..depth)
        .map(|i| [classes[i], sc, classes[i + 1]])
        .collect()
}

/// Build the CR4 existential-traversal workload: a DEPTH-long ∃-successor chain
/// `A_i ⊑ ∃r.A_{i+1}`, `A_DEPTH ⊑ B`, and `∃r.B ⊑ B`. Every A_i derives `A_i ⊑ B` through the
/// successor — DEPTH+1 derived subsumptions OWL 2 RL cannot reach.
fn cr4_tbox(dict: &mut Dict, depth: usize) -> (Vec<[Id; 3]>, Id, Vec<Id>) {
    let sc = dict.intern_iri(RDFS_SUB_CLASS_OF);
    let on_prop = dict.intern_iri(OWL_ON_PROPERTY);
    let some_from = dict.intern_iri(OWL_SOME_VALUES_FROM);
    let r = dict.intern_iri(&format!("{EX}r"));
    let b = dict.intern_iri(&format!("{EX}B"));
    let a: Vec<Id> = (0..=depth)
        .map(|i| dict.intern_iri(&format!("{EX}A{i}")))
        .collect();
    let mut t = Vec::new();
    // A_i ⊑ ∃r.A_{i+1}  (restriction bnode per edge).
    for i in 0..depth {
        let restr = dict.intern_iri(&format!("{EX}__restr_succ_{i}"));
        t.push([a[i], sc, restr]);
        t.push([restr, on_prop, r]);
        t.push([restr, some_from, a[i + 1]]);
    }
    // A_DEPTH ⊑ B.
    t.push([a[depth], sc, b]);
    // ∃r.B ⊑ B  (the CR4 back-propagation axiom).
    let restr_b = dict.intern_iri(&format!("{EX}__restr_b"));
    t.push([restr_b, on_prop, r]);
    t.push([restr_b, some_from, b]);
    t.push([restr_b, sc, b]);
    (t, b, a)
}

fn run_chain(depth: usize) {
    let mut dict = Dict::new();
    let triples = chain_tbox(&mut dict, depth);
    let t = Instant::now();
    let h = Classifier::classify(&dict, &triples);
    let dt = secs(t);

    // Closed form: sum_{i=0}^{depth-1} (depth - i) = depth·(depth+1)/2.
    let expected = depth * (depth + 1) / 2;
    let derived: usize = (0..=depth)
        .map(|i| {
            let c = dict.intern_iri(&format!("{EX}C{i}"));
            h.super_classes(c).len()
        })
        .sum();
    assert_eq!(
        derived, expected,
        "chain depth {depth}: derived subsumptions {derived} != closed form {expected}"
    );
    println!(
        "chain   depth {depth:>6}: classify {dt:9.6}s   derived_subsumptions {derived} (= d·(d+1)/2, asserted)"
    );
}

fn run_cr4(depth: usize) {
    let mut dict = Dict::new();
    let (triples, b, a) = cr4_tbox(&mut dict, depth);
    let t = Instant::now();
    let h = Classifier::classify(&dict, &triples);
    let dt = secs(t);

    // Every A_i (i = 0..=depth) must derive A_i ⊑ B through the r-successor chain: depth+1 of
    // them. This is the EL-only consequence OWL 2 RL silently misses.
    let derived_to_b = a.iter().filter(|&&ai| h.is_subclass_of(ai, b)).count();
    let expected = depth + 1;
    assert_eq!(
        derived_to_b, expected,
        "cr4 depth {depth}: A_i ⊑ B derivations {derived_to_b} != expected {expected} (RL-unreachable closure under-derived)"
    );
    println!(
        "cr4     depth {depth:>6}: classify {dt:9.6}s   A_i⊑B_derivations {derived_to_b} (= d+1, asserted; RL cannot derive any)"
    );
}

fn main() {
    let depth: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    println!("sparq-reason-el classify_bench — depth {depth} (deterministic, regenerable)");
    run_chain(depth);
    run_cr4(depth);
    println!("OK: structural closure-size assertions held (wall-clock above is trend-only).");
}

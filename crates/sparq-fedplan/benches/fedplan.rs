//! Criterion micro-benches for `sparq-fedplan`: `select_sources` + `plan_bgp`.
//!
//! Enabled by `--features fedplan`. The benches measure throughput shapes for the
//! planner's two public entry points over a small synthetic descriptor set; **no
//! hard-coded performance numbers are expected** — Criterion records measurements to
//! `target/criterion/` for local comparison with its own baseline mechanism. Do not
//! bake numbers into documentation or tests.
//!
//! # sq-my8wd.3 — epic sq-my8wd
//!
//! Surfaces covered:
//! - `select_sources` — HiBISCuS-style recall-safe source selection over a 3-source,
//!   3-pattern BGP with varying numbers of descriptors.
//! - `plan_bgp` — bind-vs-hash join planning over the selected sources.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
// criterion 0.8 deprecated its own black_box in favour of std::hint::black_box.
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId, Term,
    TriplePattern, Var,
};
use std::hint::black_box;

// sq-ioeih: distinct Criterion paths let the two feature-state runs
// retain separate baselines while exercising the identical fixed corpus.
#[cfg(feature = "foldhash-maps")]
const HASHER: &str = "foldhash";
#[cfg(not(feature = "foldhash-maps"))]
const HASHER: &str = "fxhash";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `SourceDescriptor` with `n_predicates` predicates, each with `triples`
/// triples. The source id is derived from the index so each is distinct.
fn make_source(idx: usize, n_predicates: usize, triples_per_pred: u64) -> SourceDescriptor {
    let mut b =
        SourceDescriptor::builder(SourceId::new(format!("https://s{}.example/sparql", idx)))
            .total_triples(n_predicates as u64 * triples_per_pred);
    for p in 0..n_predicates {
        b = b.predicate(PredPartition {
            predicate: format!("https://ex.example/p{}", p),
            triples: triples_per_pred,
            distinct_subjects: triples_per_pred / 2,
            distinct_objects: triples_per_pred / 3,
        });
    }
    b.build()
}

/// A 3-pattern BGP: each pattern uses a distinct predicate `p0`, `p1`, `p2`.
/// The variables chain: `?s p0 ?o0 . ?o0 p1 ?o1 . ?o1 p2 ?o2`.
fn chain_bgp() -> Bgp {
    Bgp::new(vec![
        TriplePattern::new(
            Term::Var(Var::new("s")),
            Term::Iri("https://ex.example/p0".to_string()),
            Term::Var(Var::new("o0")),
        ),
        TriplePattern::new(
            Term::Var(Var::new("o0")),
            Term::Iri("https://ex.example/p1".to_string()),
            Term::Var(Var::new("o1")),
        ),
        TriplePattern::new(
            Term::Var(Var::new("o1")),
            Term::Iri("https://ex.example/p2".to_string()),
            Term::Var(Var::new("o2")),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Bench: select_sources
// ---------------------------------------------------------------------------

fn bench_select_sources(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_sources");
    for n_sources in [1usize, 3, 10] {
        let sources: Vec<SourceDescriptor> =
            (0..n_sources).map(|i| make_source(i, 5, 10_000)).collect();
        let bgp = chain_bgp();
        group.bench_with_input(
            BenchmarkId::new(format!("{HASHER}/n_sources"), n_sources),
            &n_sources,
            |b, _| {
                b.iter(|| {
                    // black_box prevents LLVM from eliding the selection
                    // when the result would otherwise be dropped unobserved.
                    black_box(select_sources(black_box(&bgp), black_box(&sources)));
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: plan_bgp
// ---------------------------------------------------------------------------

fn bench_plan_bgp(c: &mut Criterion) {
    let mut group = c.benchmark_group("plan_bgp");
    for n_sources in [1usize, 3, 10] {
        let sources: Vec<SourceDescriptor> =
            (0..n_sources).map(|i| make_source(i, 5, 10_000)).collect();
        let bgp = chain_bgp();
        let selection = select_sources(&bgp, &sources);
        let opts = PlanOptions::default();
        group.bench_with_input(
            BenchmarkId::new(format!("{HASHER}/n_sources"), n_sources),
            &n_sources,
            |b, _| {
                b.iter(|| {
                    // plan_bgp returns Some(JoinTree) when the BGP is non-empty and sources
                    // exist; unwrap is infallible here — the bench setup guarantees it.
                    black_box(
                        plan_bgp(
                            black_box(&bgp),
                            black_box(&selection),
                            black_box(&sources),
                            black_box(&opts),
                        )
                        .unwrap(),
                    );
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

criterion_group!(planner_benches, bench_select_sources, bench_plan_bgp);
criterion_main!(planner_benches);

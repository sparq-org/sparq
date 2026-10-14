//! Self-relative microbenchmarks for the ALCH NNF/tableau check pipeline.
//!
//! The fixtures pin checker verdicts before Criterion times them. Those pins are a
//! regression/stability oracle only; this benchmark makes no new reasoner-soundness claim.
//! Criterion keeps measurements outside tracked files, under `target/criterion/`.

// sq-7ru8y: stability-oracle fixtures for self-relative depth scaling.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sparq_core::dict::Id;
use sparq_reason_dl::model::ClassExpression as CE;
use sparq_reason_dl::nnf;
use sparq_reason_dl::tableau::{class_satisfiability, Budget, Verdict};
use sparq_reason_dl::Ontology;
use std::hint::black_box;

const CLASS_A: Id = 1;
const CLASS_B: Id = 2;
const ROLE: Id = 10;
const DEPTHS: [usize; 4] = [1, 2, 4, 8];

#[derive(Clone, Copy, Debug)]
enum Check {
    Satisfiable,
    Unsatisfiable,
    Subsumed,
    NotSubsumed,
}

#[derive(Clone, Debug)]
struct Fixture {
    depth: usize,
    check: Check,
    expression: CE,
    expected: bool,
}

fn nested_some(depth: usize, filler: CE) -> CE {
    (0..depth).fold(filler, |inner, _| CE::some(ROLE, inner))
}

fn not(expression: CE) -> CE {
    CE::ObjectComplementOf(Box::new(expression))
}

fn intersection(left: CE, right: CE) -> CE {
    CE::ObjectIntersectionOf(vec![left, right])
}

fn fixtures() -> Vec<Fixture> {
    let mut fixtures = Vec::with_capacity(DEPTHS.len() * 4);
    for depth in DEPTHS {
        let nested_a = nested_some(depth, CE::Class(CLASS_A));
        fixtures.push(Fixture {
            depth,
            check: Check::Satisfiable,
            expression: nested_a.clone(),
            expected: true,
        });
        fixtures.push(Fixture {
            depth,
            check: Check::Unsatisfiable,
            expression: nested_some(
                depth,
                intersection(CE::Class(CLASS_A), not(CE::Class(CLASS_A))),
            ),
            expected: false,
        });

        // C is subsumed by D iff C intersected with not-D is unsatisfiable. Keeping the
        // reduction explicit makes both sat and subsumption fixtures use the same public
        // NNF/tableau seam.
        let nested_thing = nested_some(depth, CE::Thing);
        fixtures.push(Fixture {
            depth,
            check: Check::Subsumed,
            expression: intersection(nested_a.clone(), not(nested_thing)),
            expected: true,
        });
        fixtures.push(Fixture {
            depth,
            check: Check::NotSubsumed,
            expression: intersection(nested_a, not(nested_some(depth, CE::Class(CLASS_B)))),
            expected: false,
        });
    }
    fixtures
}

fn verdict(fixture: &Fixture) -> bool {
    // Exercise NNF as an explicit pipeline stage as well as the tableau's internal
    // normalization. For subsumption fixtures, an UNSAT reduction means "subsumed".
    let normalized = nnf::nnf(&fixture.expression);
    let result = class_satisfiability(&normalized, &Ontology::new(), Budget::default());
    match fixture.check {
        Check::Satisfiable | Check::Unsatisfiable => result == Verdict::Satisfiable,
        Check::Subsumed | Check::NotSubsumed => result == Verdict::Unsatisfiable,
    }
}

fn assert_verdicts(fixtures: &[Fixture]) {
    for fixture in fixtures {
        assert_eq!(
            verdict(fixture),
            fixture.expected,
            "stability verdict changed for {:?} at depth {}",
            fixture.check,
            fixture.depth
        );
    }
}

#[test]
fn verdict_matches_expected() {
    assert_verdicts(&fixtures());
}

fn bench_tableau_checks(c: &mut Criterion) {
    let fixtures = fixtures();
    // Validate every pinned answer once before any timed iteration begins.
    assert_verdicts(&fixtures);

    let mut group = c.benchmark_group("tableau_check_by_depth");
    for fixture in &fixtures {
        let kind = match fixture.check {
            Check::Satisfiable => "sat",
            Check::Unsatisfiable => "unsat",
            Check::Subsumed => "subsumed",
            Check::NotSubsumed => "not_subsumed",
        };
        group.bench_with_input(
            BenchmarkId::new(kind, fixture.depth),
            fixture,
            |b, fixture| b.iter(|| black_box(verdict(black_box(fixture)))),
        );
    }
    group.finish();
}

criterion_group!(tableau_benches, bench_tableau_checks);
criterion_main!(tableau_benches);

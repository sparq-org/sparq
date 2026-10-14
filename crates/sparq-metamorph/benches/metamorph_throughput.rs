//! Self-relative throughput bench for deterministic generation plus TLP + NoREC checks.
//!
//! sq-hgqza. There is no directly runnable RDF/SPARQL peer for this
//! SQLancer-family logic-bug harness, so Criterion's result is a regression signal
//! against this bench's own prior runs, not an external comparison. Measurements are
//! emitted only after every generated case has two exhaustively classified verdicts.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use sparq_metamorph::{check_norec, check_tlp, generate_case, InProcessSparq, Verdict};

const SEED_START: u64 = 0;
const SEED_COUNT: u64 = 16;

/// Classify one verdict into exactly one state. Keeping this match exhaustive makes a
/// newly added verdict state a compile failure until the accounting contract is updated.
fn count_verdict(verdict: &Verdict) -> u64 {
    match verdict {
        Verdict::Pass { .. } | Verdict::Violation(_) | Verdict::EngineFailure(_) => 1,
    }
}

fn generate_and_check_window(check_determinism: bool) -> u64 {
    let mut counted = 0;
    for seed in SEED_START..SEED_START + SEED_COUNT {
        let case = generate_case(seed);
        if check_determinism {
            assert_eq!(
                case,
                generate_case(seed),
                "seed {seed} must reproduce its case"
            );
        }
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|failure| panic!("seed {seed} did not load: {failure}"));

        let tlp = check_tlp(&engine, &case.pattern, &case.predicate);
        let norec = check_norec(&engine, &case.pattern, &case.predicate);
        counted += count_verdict(&tlp) + count_verdict(&norec);
    }

    assert_eq!(
        counted,
        SEED_COUNT * 2,
        "every case must produce one counted TLP and one counted NoREC verdict"
    );
    counted
}

fn metamorph_throughput(c: &mut Criterion) {
    // Fail before Criterion reports throughput if the deterministic corpus or verdict
    // accounting contract is broken.
    assert_eq!(generate_and_check_window(true), SEED_COUNT * 2);

    let mut group = c.benchmark_group("metamorph_generation_and_check");
    group.throughput(Throughput::Elements(SEED_COUNT));
    group.bench_function("fixed_seed_window", |b| {
        b.iter(|| black_box(generate_and_check_window(false)));
    });
    group.finish();
}

#[test]
fn all_cases_verdicted() {
    assert_eq!(generate_and_check_window(true), SEED_COUNT * 2);
}

criterion_group!(benches, metamorph_throughput);
criterion_main!(benches);

//! sq-ql2iy — the GeoSPARQL Compliance Benchmark (GSB) runner.
//!
//! Runs sparq through the **206-query GeoSPARQL Compliance Benchmark**
//! (Jovanovik, Homburg, Spasić — *A GeoSPARQL Compliance Benchmark*, ISPRS IJGI
//! 10(7):487, 2021; arXiv:2102.06139) and reports the benchmark's own two
//! metrics: **correct answers** (of 206) and the **requirements-weighted
//! compliance percentage**.
//!
//! Why: `research/gap-conformance-cross-engine-2026-07.md` §3.7 scored the
//! GeoSPARQL row **NOT-COMPARABLE** — sparq ratchets its own 197-assertion
//! DE-9IM battery, which is a different unit from the published percentages
//! (Fuseki-geosparql 82.75 %, GraphDB 69.75 %, …). This runner puts sparq on the
//! published axis.
//!
//! ```sh
//! GSB="$(bash crates/sparq-geo/fetch-geosparql-benchmark.sh)"
//! cargo run --release -p sparq-geo --features geosparql_rewrite,geof_accessors \
//!     --example gsb_compliance -- "$GSB/src/main/resources"
//! ```
//!
//! The benchmark artifact is GPL-2.0 and is NEVER vendored — `crates/sparq-geo/fetch-geosparql-benchmark.sh`
//! is the shared pinned fetch recipe (see its header).
//!
//! ## System under test
//!
//! The benchmark scores a *system*, so the runner drives one sparq GeoSPARQL
//! stack uniformly across all 206 queries — nothing is special-cased per query:
//!
//! * `sparq-core` RDF/XML load of the benchmark dataset;
//! * `sparq-reason` RDFS materialisation (the benchmark's RDFSE requirements
//!   R25–R27 test exactly "materialised **and** inferred" triples);
//! * `sparq-geo`'s `geof:` [`FunctionRegistry`](sparq_engine::FunctionRegistry)
//!   including the opt-in `geof_accessors` functions;
//! * the standard `sparq_engine` query entry point, or `sparq-geo`'s opt-in
//!   `geosparql_rewrite` one.
//!
//! Both of those last two axes are runtime toggles (`GSB_RDFS=0`,
//! `GSB_REWRITE=0`) because both change the score materially — a compliance
//! number is meaningless without naming the configuration that produced it. The
//! measured matrix is recorded in
//! `research/gap-conformance-cross-engine-2026-07.md` §3.7.
//!
//! ## Scoring — faithful to the benchmark's own harness
//!
//! * **Weights** (`plan`) reproduce the benchmark's scoring table: each of the
//!   30 requirements is worth `1/30`, split evenly over its query groups, and a
//!   4-query serialisation group splits `1/3, 1/3, 1/6, 1/6` (the WKT/WKT and
//!   GML/GML forms weigh double the two mixed forms). R17 has no query and is
//!   credited iff the system scores above zero — the upstream rule.
//! * **Answer comparison** reproduces the upstream evaluation module: a result
//!   matches iff its variable list and its *ordered* row list equal the expected
//!   answer's, after the upstream normalisations — `geo:wktLiteral` values have
//!   spaces/newlines removed, are trimmed and lower-cased, `geo:gmlLiteral` values go
//!   through XML C14N 1.0 without comments (character-data whitespace preserved). Any of a query's
//!   `-alternative-N.srx` answers counts.
//! * A query whose evaluation **errors** (unsupported function, parse failure)
//!   is simply an incorrect answer, exactly as an endpoint error would be.
//!
//! Output is one TSV row per query (`id`, status, weight) followed by the two
//! summary metrics; the exit status is 0 whenever the run completed, since the
//! score is a measurement, not a gate.

use std::fs;
#[path = "../tests/support/gsb.rs"]
mod gsb;
use gsb::{expected_answers, load_dataset, run_query};

/// One benchmark query: its file stem and its share of the total score.
struct Task {
    id: String,
    weight: f64,
}

/// The benchmark's scoring table, reproduced structurally.
///
/// Every requirement is worth `1/30`. `SERIALISATION_SPLIT` is the weighting the
/// benchmark gives a 4-query group that tests one operation over the WKT/WKT,
/// GML/GML, WKT/GML and GML/WKT serialisation pairs.
const SERIALISATION_SPLIT: [f64; 4] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0];

fn plan() -> Vec<Task> {
    let req = 1.0 / 30.0;
    let mut tasks: Vec<Task> = Vec::new();
    let mut push = |id: String, weight: f64| tasks.push(Task { id, weight });

    // Requirements answered by a single query.
    for n in [1u32, 2, 3, 7, 10, 11, 12, 14, 15, 18, 27] {
        push(format!("query-r{:02}", n), req);
    }
    // Requirements answered by a flat, evenly-weighted group.
    for (n, count) in [
        (4u32, 8u32),
        (5, 8),
        (6, 8),
        (8, 2),
        (9, 6),
        (13, 2),
        (16, 2),
        (20, 2),
        (25, 3),
        (26, 2),
        (28, 8),
        (29, 8),
        (30, 8),
    ] {
        for i in 1..=count {
            push(format!("query-r{:02}-{}", n, i), req / f64::from(count));
        }
    }
    // R21 (geof:relate) is a single serialisation group.
    for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
        push(format!("query-r21-{}", i + 1), req * share);
    }
    // R19 (the non-topological geof: functions): nine groups, four of which take
    // one argument (2 serialisations) and five two (4 serialisation pairs).
    for g in 1u32..=9 {
        let arity2 = matches!(g, 1 | 4 | 5 | 6 | 7);
        if arity2 {
            for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
                push(format!("query-r19-{}-{}", g, i + 1), req / 9.0 * share);
            }
        } else {
            for i in 1..=2 {
                push(format!("query-r19-{}-{}", g, i), req / 9.0 * 0.5);
            }
        }
    }
    // R22/R23/R24 (the geof: sf / eh / rcc8 topology functions): eight relations,
    // each a serialisation group.
    for n in [22u32, 23, 24] {
        for g in 1u32..=8 {
            for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
                push(format!("query-r{}-{}-{}", n, g, i + 1), req / 8.0 * share);
            }
        }
    }
    tasks
}

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            eprintln!("usage: gsb_compliance <gsb-resources-dir>   (see crates/sparq-geo/fetch-geosparql-benchmark.sh; append /src/main/resources to its output)");
            std::process::exit(2);
        }
    };

    let tasks = plan();
    assert_eq!(tasks.len(), 206, "the benchmark is 206 queries");
    let total_weight: f64 = tasks.iter().map(|t| t.weight).sum();
    // 29 of the 30 requirements carry queries; R17 is credited separately.
    assert!(
        (total_weight - 29.0 / 30.0).abs() < 1e-9,
        "scoring table must sum to 29/30, got {}",
        total_weight
    );

    let rdfs = std::env::var("GSB_RDFS").as_deref() != Ok("0");
    let rewrite = std::env::var("GSB_REWRITE").as_deref() != Ok("0");
    let graph = load_dataset(&dir, rdfs);
    eprintln!(
        "[gsb] dataset loaded: {} triples (rdfs={}, rewrite={})",
        graph.len(),
        rdfs,
        rewrite
    );

    // `GSB_DEBUG=<query-id-prefix>` dumps the actual-vs-expected result sets for
    // the matching failures — how a FAIL is triaged into "engine gap" vs
    // "serialisation difference" without re-deriving the harness.
    let debug = std::env::var("GSB_DEBUG").ok();
    let mut correct = 0usize;
    let mut score = 0.0f64;
    println!("query\tstatus\tweight");
    for task in &tasks {
        let qpath = dir.join(format!("gsb_queries/{}.rq", task.id));
        let Ok(sparql) = fs::read_to_string(&qpath) else {
            println!("{}\tMISSING\t{:.6}", task.id, task.weight);
            continue;
        };
        let expected = expected_answers(&dir, &task.id).unwrap_or_else(|e| panic!("{}", e));
        if expected.is_empty() {
            println!("{}\tNO-ANSWER\t{:.6}", task.id, task.weight);
            continue;
        }
        let status = match run_query(&graph, &sparql, rewrite) {
            Err(e) => {
                eprintln!("[gsb] {}: {}", task.id, e);
                "ERROR"
            }
            Ok(actual) => {
                if expected.contains(&actual) {
                    correct += 1;
                    score += task.weight;
                    "PASS"
                } else {
                    if debug.as_deref().is_some_and(|d| task.id.starts_with(d)) {
                        if let Ok(p) = sparq_geo::geosparql_rewrite(&sparql) {
                            eprintln!("[gsb] {} rewritten: {}", task.id, p.query());
                        }
                        eprintln!("[gsb] {} actual:   {:?}", task.id, actual);
                        for e in &expected {
                            eprintln!("[gsb] {} expected: {:?}", task.id, e);
                        }
                    }
                    "FAIL"
                }
            }
        };
        println!("{}\t{}\t{:.6}", task.id, status, task.weight);
    }

    // The upstream rule: R17 has no query, and is credited iff the system scored
    // above zero on the rest.
    if score > 0.0 {
        score += 1.0 / 30.0;
    }
    println!("gsb_correct_answers\t{}\t{}", correct, tasks.len());
    println!("gsb_compliance_pct\t{:.2}", score * 100.0);
}

/// The corpus-free half of the harness — the scoring table and the answer
/// comparator (shared with the fixture-gated integration test). These tests
/// run under `cargo test --all-features` without downloading the benchmark.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_table_reproduces_the_benchmark() {
        let tasks = plan();
        assert_eq!(tasks.len(), 206, "the benchmark is 206 queries");
        let total: f64 = tasks.iter().map(|t| t.weight).sum();
        // 29 requirements carry queries; R17 has none and is credited separately.
        assert!(
            (total - 29.0 / 30.0).abs() < 1e-12,
            "table sums to {}",
            total
        );

        let w = |id: &str| {
            tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("no task {}", id))
                .weight
        };
        // A whole requirement in one query.
        assert!((w("query-r01") - 1.0 / 30.0).abs() < 1e-12);
        // An evenly split eight-query requirement.
        assert!((w("query-r04-8") - 1.0 / 30.0 / 8.0).abs() < 1e-12);
        // A serialisation group: the two mixed WKT/GML forms weigh half.
        assert!((w("query-r22-1-1") - 1.0 / 30.0 / 8.0 / 3.0).abs() < 1e-12);
        assert!((w("query-r22-1-3") - 1.0 / 30.0 / 8.0 / 6.0).abs() < 1e-12);
        // R19's nine groups: a two-serialisation group splits evenly.
        assert!((w("query-r19-2-1") - 1.0 / 30.0 / 9.0 / 2.0).abs() < 1e-12);
        assert!((w("query-r19-1-3") - 1.0 / 30.0 / 9.0 / 6.0).abs() < 1e-12);
        // The ids must be the benchmark's own file stems.
        assert!(tasks.iter().any(|t| t.id == "query-r30-8"));
        assert!(!tasks.iter().any(|t| t.id.starts_with("query-r17")));
    }
}

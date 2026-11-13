//! Fixture-gated GeoSPARQL Compliance Benchmark; not an official OGC ETS.
//! Fetch with `crates/sparq-geo/fetch-geosparql-benchmark.sh`.
//! GEOSPARQL_BENCHMARK_DIR overrides the checkout (default tests/fixtures/...);
//! SPARQ_EARL_OUT receives one query-id TAB passed|failed line per query.
//! The configuration is fixed across all queries: RDFS materialization, accessors,
//! and the opt-in query-rewrite entry point over the normal engine executor.

mod support {
    pub mod gsb;
}

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

#[test]
fn geosparql_benchmark() {
    let mut raw =
        std::env::var_os("SPARQ_EARL_OUT").map(|p| File::create(p).expect("create outcomes"));

    let override_dir = std::env::var_os("GEOSPARQL_BENCHMARK_DIR");
    let root = override_dir.clone().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geosparql-benchmark")
    });
    if !root.exists() && override_dir.is_none() {
        assert!(
            std::env::var_os("SPARQ_EARL_OUT").is_none(),
            "cannot report an absent benchmark"
        );
        eprintln!(
            "SKIP: GeoSPARQL Benchmark absent; run crates/sparq-geo/fetch-geosparql-benchmark.sh"
        );
        return;
    }
    let resources = root.join("src/main/resources");
    let queries = resources.join("gsb_queries");
    let mut tasks = Vec::new();
    for entry in fs::read_dir(&queries).expect("read benchmark query directory") {
        let path = entry.unwrap().path();
        let id = path.file_stem().unwrap().to_str().unwrap();
        // query01..query05_detail are system/illustration queries, not scored
        // tasks (GSBConstants.GSB_QUERIES enumerates only query-r*).
        if path.extension().is_some_and(|e| e == "rq") && id.starts_with("query-r") {
            let requirement: usize = id[7..].split('-').next().unwrap().parse().unwrap();
            assert!((1..=30).contains(&requirement) && requirement != 17);
            tasks.push((id.to_string(), requirement, path));
        }
    }
    tasks.sort();
    assert_eq!(
        tasks.len(),
        206,
        "incomplete or changed benchmark query inventory"
    );
    // Parse every fixture before executing: a broken answer is a harness error,
    // never an apparent engine failure that a still-green floor could hide.
    let expected: Vec<_> = tasks
        .iter()
        .map(|(id, _, _)| {
            support::gsb::expected_answers(&resources, id).unwrap_or_else(|e| panic!("{}", e))
        })
        .collect();
    let graph = support::gsb::load_dataset(&resources, true);
    let mut requirements: BTreeMap<usize, (usize, usize)> = (1..=30).map(|n| (n, (0, 0))).collect();
    for ((id, requirement, path), expected) in tasks.iter().zip(&expected) {
        let query = fs::read_to_string(path).expect("read benchmark query");
        let passed = match support::gsb::run_query(&graph, &query, true) {
            Ok(actual) => {
                let passed = expected.contains(&actual);
                if !passed {
                    eprintln!("{}: answer mismatch", id);
                    if std::env::var("GSB_DEBUG")
                        .ok()
                        .is_some_and(|d| id.starts_with(&d))
                    {
                        eprintln!("actual: {:?}\nexpected: {:?}", actual, expected);
                    }
                }
                passed
            }
            Err(e) => {
                eprintln!("{}: {}", id, e);
                false
            }
        };
        let status = if passed { "passed" } else { "failed" };
        println!("{}\t{}", id, status);
        if let Some(raw) = &mut raw {
            writeln!(raw, "{}\t{}", id, status).unwrap();
        }
        let counts = requirements.get_mut(requirement).unwrap();
        if passed {
            counts.0 += 1;
        } else {
            counts.1 += 1;
        }
    }
    let (mut passed, mut failed) = (0, 0);
    println!("requirement\tpassed\tfailed\ttotal");
    for (r, (p, f)) in requirements {
        println!("R{:02}\t{}\t{}\t{}", r, p, f, p + f);
        passed += p;
        failed += f;
    }
    println!("TOTAL\t{}\t{}\t{}", passed, failed, passed + failed);
    println!("EARL complete geosparql-benchmark");
    assert!(
        passed >= sparq_conformance_floors::geo::GEOSPARQL_BENCHMARK_FLOOR,
        "benchmark pass count regressed: {} < {}",
        passed,
        sparq_conformance_floors::geo::GEOSPARQL_BENCHMARK_FLOOR
    );
}

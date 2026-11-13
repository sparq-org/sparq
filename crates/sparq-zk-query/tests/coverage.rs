#[path = "support/mod.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;
use fixture_support::{dataset, FixtureEncoder, Workspace};
use serde_json::Value;
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    interp::{evaluate, Cell, ResultValue, Trace},
    lower::lower,
    witness::prover_toml,
};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../")
}
fn corpus() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(
            root().join("zk/sparql-evaluator/fixtures/conformance/cases.json"),
        )
        .unwrap(),
    )
    .unwrap()
}
fn merged_manifest() -> Value {
    let mut m: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("zk/noir-query/coverage.json")).unwrap(),
    )
    .unwrap();
    let mut fragments: Vec<_> = std::fs::read_dir(root().join("zk/noir-query/coverage.d"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    fragments.sort_by_key(|e| e.file_name());
    let mut replaced = std::collections::BTreeSet::new();
    for f in fragments {
        let fragment: Value = serde_json::from_slice(&std::fs::read(f.path()).unwrap()).unwrap();
        for (group, key) in [("cases", "id"), ("features", "id"), ("functions", "name")] {
            for row in fragment[group].as_array().into_iter().flatten() {
                let name = row[key].as_str().unwrap();
                assert!(
                    replaced.insert((group.to_owned(), name.to_owned())),
                    "conflicting lane fragment: {group}/{name}"
                );
                let slot = m[group]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r[key] == name)
                    .expect("fragment refers to inventory entry");
                *slot = row.clone();
            }
        }
    }
    m
}
const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 16,
    rows_per_table: 16,
};

#[test]
fn manifest_covers_the_corpus_without_overstating_and_never_lowers_the_floor() {
    let m = merged_manifest();
    let c = corpus();
    let inventory: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("zk/sparql-evaluator/coverage.json")).unwrap(),
    )
    .unwrap();
    for (group, key) in [("cases", "id"), ("features", "id"), ("functions", "name")] {
        let source = if group == "cases" { &c } else { &inventory };
        let expected: std::collections::BTreeSet<_> = source[group]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[key].as_str().unwrap())
            .collect();
        let actual: std::collections::BTreeSet<_> = m[group]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[key].as_str().unwrap())
            .collect();
        assert_eq!(expected, actual, "{group} inventory");
        assert_eq!(
            actual.len(),
            m[group].as_array().unwrap().len(),
            "duplicate {group}"
        );
    }
    assert_eq!(m["evidence"]["negative_cases_are_feature_support"], false);
    let cases = m["cases"].as_array().unwrap();
    for entry in cases {
        let fixture = c["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == entry["id"])
            .unwrap();
        match entry["status"].as_str().unwrap() {
            "passing" => {
                assert_eq!(fixture["expected"]["kind"], "result");
                lower(fixture["query"].as_str().unwrap(), B).unwrap();
            }
            "refused" => assert_eq!(
                lower(fixture["query"].as_str().unwrap(), B)
                    .unwrap_err()
                    .to_string(),
                entry["refusal"].as_str().unwrap()
            ),
            "not_attempted" => (),
            other => panic!("unknown status {other}"),
        }
    }
    for group in ["features", "functions"] {
        for row in m[group].as_array().unwrap() {
            let positives = row["positive_cases"].as_array().unwrap();
            assert!(matches!(
                row["status"].as_str(),
                Some("not_attempted" | "refused" | "partial_fixture_coverage")
            ));
            if row["status"] == "partial_fixture_coverage" {
                assert!(!positives.is_empty());
            } else {
                assert!(positives.is_empty());
            }
            for id in positives {
                assert!(
                    cases
                        .iter()
                        .any(|c| c["id"] == *id && c["status"] == "passing"),
                    "feature claim lacks executed case: {id}"
                );
            }
        }
    }
    let floor: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("zk/noir-query/coverage-floor.json")).unwrap(),
    )
    .unwrap();
    for id in floor["passing_cases"].as_array().unwrap() {
        assert!(
            cases
                .iter()
                .any(|c| c["id"] == *id && c["status"] == "passing"),
            "coverage floor: {id}"
        );
    }
}

fn assert_expected(trace: &Trace, fixture: &Value) {
    let expected = &fixture["expected"]["result"];
    match &trace.result {
        ResultValue::Ask(value) => assert_eq!(Some(*value), expected["Ask"].as_bool()),
        ResultValue::Select { variables, rows } => {
            let expected = &expected["Select"];
            assert_eq!(
                *variables,
                expected["variables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            );
            let mut expected_rows = vec![];
            for row in expected["rows"].as_array().unwrap() {
                let mut cells = vec![];
                for term in row.as_array().unwrap() {
                    if term.is_null() {
                        cells.push(None);
                        continue;
                    }
                    let (_, terms) =
                        dataset(&format!("<urn:s> <urn:p> {} .", term.as_str().unwrap()));
                    use sparq_zk_query::interp::EncodeTerm;
                    cells.push(Some(FixtureEncoder.encode(&terms[2]).unwrap().enc));
                }
                expected_rows.push(cells);
            }
            let mut actual: Vec<Vec<_>> = rows
                .iter()
                .map(|r| r.iter().map(|c| c.bound.then_some(c.enc)).collect())
                .collect();
            if expected["order"] == "Bag" {
                actual.sort();
                expected_rows.sort();
            }
            assert_eq!(actual, expected_rows);
        }
    }
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn corpus_cases_execute_with_engine_expected_result_and_altered_witness_controls() {
    let Some(gate) = support::gate(
        "corpus_cases_execute_with_engine_expected_result_and_altered_witness_controls",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let corpus = corpus();
    let manifest = merged_manifest();
    for fixture in corpus["cases"].as_array().unwrap() {
        let entry = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == fixture["id"])
            .unwrap();
        let query = fixture["query"].as_str().unwrap();
        // In addition to the ratcheted claims, exercise every currently admitted
        // positive fixture so new lowering cannot bypass the differential runner.
        if fixture["expected"]["kind"] != "result" {
            continue;
        }
        let plan = match lower(query, B) {
            Ok(plan) => plan,
            Err(_) if entry["status"] != "passing" => continue,
            Err(e) => panic!("{}: {e}", fixture["id"]),
        };
        let rdf = fixture
            .get("dataset")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                std::fs::read_to_string(
                    root().join("zk/sparql-evaluator/fixtures/conformance/core.nt"),
                )
                .unwrap()
            });
        let (data, _) = dataset(&rdf);
        let trace = evaluate(&plan, &data, &FixtureEncoder)
            .unwrap_or_else(|e| panic!("{}: {e}", fixture["id"]));
        assert_expected(&trace, fixture);
        fixture_support::assert_engine(query, &rdf, &trace.result);
        let package = sparq_zk_query::codegen::generate(&plan).unwrap();
        let workspace = Workspace::new(&package);
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
        driver
            .execute(&input, "corpus")
            .unwrap_or_else(|e| panic!("{}: {e}", fixture["id"]));
        for (n, table) in trace.tables.iter().enumerate() {
            let mut changed = trace.clone();
            if table.rows.is_empty() {
                changed.tables[n].rows.push(vec![
                    Cell::default();
                    plan.nodes()[n].output.variables.len()
                ]);
            } else if table.rows[0].is_empty() {
                changed.tables[n].rows.pop();
            } else {
                changed.tables[n].rows[0][0].enc[31] ^= 1;
            }
            let input =
                prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&input, "altered_table").is_err(),
                "{}, node {n}",
                fixture["id"]
            );
        }
        let mut controls = 0;
        let result_line = input.lines().find(|l| l.starts_with("result = ")).unwrap();
        if matches!(trace.result, ResultValue::Ask(_)) {
            let changed = input.replace(
                result_line,
                if result_line == "result = true" {
                    "result = false"
                } else {
                    "result = true"
                },
            );
            assert!(driver.execute(&changed, "altered_result").is_err());
            controls += 1;
        } else {
            for (position, _) in result_line.match_indices("enc = \"0x") {
                let mut changed = result_line.as_bytes().to_vec();
                let p = position + "enc = \"0x".len() + 63;
                changed[p] = if changed[p] == b'1' { b'2' } else { b'1' };
                let changed = input.replace(result_line, &String::from_utf8(changed).unwrap());
                assert!(driver.execute(&changed, "altered_result").is_err());
                controls += 1;
            }
        }
        assert!(controls > 0);
        let changed = input.replace("request_digest = [7,", "request_digest = [8,");
        assert!(driver.execute(&changed, "altered_public").is_err());
    }
    support::complete(gate);
}

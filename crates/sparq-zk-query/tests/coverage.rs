#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/fixture.rs"]
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
    witness::prover_toml_authenticated,
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
                if let Some(evidence) = fragment.get("evidence") {
                    slot["evidence"] = evidence.clone();
                }
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
                if let Some(reference) = id.as_str().and_then(|s| s.strip_prefix("noir:")) {
                    assert_eq!(row["evidence"]["layer"], "isolated_gadget_execution");
                    assert_eq!(row["guest_proof_evidence"], "not_asserted");
                    assert_noir_reference(reference, false);
                    continue;
                }
                assert!(
                    cases
                        .iter()
                        .any(|c| c["id"] == *id && c["status"] == "passing"),
                    "feature claim lacks executed case: {id}"
                );
            }
            for id in row["rejection_cases"].as_array().into_iter().flatten() {
                if let Some(reference) = id.as_str().and_then(|s| s.strip_prefix("noir:")) {
                    assert_eq!(row["evidence"]["layer"], "isolated_gadget_execution");
                    assert_noir_reference(reference, true);
                }
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

// Isolated gadget references are tracked separately from generated-query cases.
// The workflow runs the committed package's tests; these references cannot raise
// the query corpus floor or claim guest-proof coverage.
fn assert_noir_reference(reference: &str, rejection: bool) {
    let (module, function) = reference.split_once("::").expect("Noir test reference");
    assert!(module
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    assert!(function
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    let source = std::fs::read_to_string(
        root()
            .join("zk/noir-query/core/src")
            .join(format!("{module}.nr")),
    )
    .expect("referenced Noir module");
    let declaration = format!("fn {function}(");
    let preceding = source
        .split_once(&declaration)
        .expect("referenced Noir test")
        .0;
    let attribute = preceding
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap()
        .trim();
    if rejection {
        assert!(
            attribute.starts_with("#[test(should_fail"),
            "rejection test: {reference}"
        );
    } else {
        assert_eq!(attribute, "#[test]", "positive test: {reference}");
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
        &["nargo", "bb", "node"],
    ) else {
        return;
    };
    let corpus = corpus();
    let manifest = merged_manifest();
    let mut workspaces = std::collections::BTreeMap::new();
    let mut compiler_agreement = std::collections::BTreeSet::new();
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
        let (data, trace) = host_fixture(fixture, &plan);
        let package = sparq_zk_query::codegen::generate(&plan).unwrap();
        let workspace = workspaces
            .entry(plan.hash())
            .or_insert_with(|| Workspace::new(&package));
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        if entry["status"] == "passing" && compiler_agreement.insert(plan.hash()) {
            let artifact = driver.compile().unwrap();
            let compile_only = support::live_tool(&gate, "node")
                .arg(root().join("zk/noir-query/compiler-agreement/check.mjs"))
                .arg(workspace.0.join(&package.name))
                .arg(&artifact)
                .arg("--compile-only")
                .arg(workspace.0.join("wasm-cache"))
                .output()
                .unwrap();
            assert!(
                compile_only.status.success(),
                "compiler bytecode agreement for {}: {}{}",
                fixture["id"],
                String::from_utf8_lossy(&compile_only.stdout),
                String::from_utf8_lossy(&compile_only.stderr)
            );
            let key = driver.canonical_vk(&workspace.0).unwrap();
            let key_file = workspace.0.join("agreement.vk");
            std::fs::write(&key_file, key).unwrap();
            let out = support::live_tool(&gate, "node")
                .arg(root().join("zk/noir-query/compiler-agreement/check.mjs"))
                .arg(workspace.0.join(&package.name))
                .arg(artifact)
                .arg(key_file)
                .arg(workspace.0.join("wasm-cache"))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "compiler agreement for {}: {}{}",
                fixture["id"],
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let input = prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap();
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
            let input = prover_toml_authenticated(
                &plan,
                &data,
                &changed,
                [7; 32],
                [3; 32],
                authenticated::issuer_root(),
            )
            .unwrap();
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
    // Every passing case was visited, and each distinct generated program
    // derived its bytecode and key with both pinned implementations.
    let passing_programs: std::collections::BTreeSet<_> = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| {
            manifest["cases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == f["id"] && c["status"] == "passing")
        })
        .map(|f| lower(f["query"].as_str().unwrap(), B).unwrap().hash())
        .collect();
    assert_eq!(compiler_agreement, passing_programs);
    support::complete(gate);
}

fn host_fixture(
    fixture: &Value,
    plan: &sparq_zk_query::plan::Plan,
) -> (Vec<sparq_zk_query::credential::Credential>, Trace) {
    let query = fixture["query"].as_str().unwrap();
    let rdf = fixture
        .get("dataset")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            std::fs::read_to_string(root().join("zk/sparql-evaluator/fixtures/conformance/core.nt"))
                .unwrap()
        });
    let data = vec![authenticated::sign(&rdf, 0)];
    let cells = vec![data[0].cells(0).unwrap()];
    let trace = evaluate(plan, &cells, &FixtureEncoder)
        .unwrap_or_else(|e| panic!("{}: {e}", fixture["id"]));
    assert_expected(&trace, fixture);
    fixture_support::assert_engine(query, &rdf, &trace.result);
    (data, trace)
}

#[test]
fn admitted_corpus_suite_inputs_match_both_host_oracles() {
    for fixture in corpus()["cases"].as_array().unwrap() {
        if fixture["expected"]["kind"] == "result" {
            if let Ok(plan) = lower(fixture["query"].as_str().unwrap(), B) {
                let _checked = host_fixture(fixture, &plan);
            }
        }
    }
}

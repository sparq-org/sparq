// [GPT-6] Actual V3 execution of unchanged shared goldens; these are not receipts.
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::embedded_artifact;
use sparq_proved_evaluator_model::{BudgetExceeded, DatasetAuthority, EvaluationCapacity, EvaluationError, ProofContract, Rejected, v3};
use std::path::PathBuf;

struct Suite {
    name: &'static str,
    json: &'static str,
    field: &'static str,
    capacity: bool,
    counts: (usize, usize, usize),
}

fn suites() -> Vec<Suite> {
    vec![
        Suite { name: "original", json: include_str!("../../fixtures/conformance/cases.json"), field: "cases", capacity: false, counts: (92, 24, 0) },
        Suite { name: "builtins", json: include_str!("../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json"), field: "cases", capacity: false, counts: (167, 0, 2) },
        Suite { name: "numeric-capacity", json: include_str!("../../fixtures/conformance/numeric-capacity.json"), field: "cases", capacity: true, counts: (0, 0, 21) },
        Suite { name: "temporal", json: include_str!("../../../../crates/sparq-engine/tests/fixtures/exact_temporal.json"), field: "cases", capacity: false, counts: (28, 0, 0) },
        Suite { name: "temporal-capacity", json: include_str!("../../fixtures/conformance/temporal-capacity.json"), field: "cases", capacity: true, counts: (0, 0, 13) },
        Suite { name: "temporal-lexical", json: include_str!("../../../../crates/sparq-engine/tests/fixtures/temporal_lexical.json"), field: "cases", capacity: false, counts: (37, 0, 0) },
        Suite { name: "temporal-lexical-capacity", json: include_str!("../../../../crates/sparq-engine/tests/fixtures/temporal_lexical.json"), field: "capacity_cases", capacity: true, counts: (0, 0, 3) },
        Suite { name: "numeric-ebv", json: include_str!("../../fixtures/conformance/numeric-ebv.json"), field: "cases", capacity: false, counts: (17, 0, 3) },
        Suite { name: "raw-lexical", json: include_str!("../../fixtures/conformance/raw-literal-whitespace.json"), field: "cases", capacity: false, counts: (32, 0, 0) },
        Suite { name: "aggregates", json: include_str!("../../fixtures/conformance/aggregate-boundaries.json"), field: "cases", capacity: false, counts: (12, 12, 0) },
        Suite { name: "exists-bound", json: include_str!("../../fixtures/conformance/exists-bound-boundaries.json"), field: "cases", capacity: false, counts: (8, 12, 0) },
        Suite { name: "exists-minus", json: include_str!("../../fixtures/conformance/exists-minus-domains.json"), field: "cases", capacity: false, counts: (8, 0, 0) },
        Suite { name: "nullable", json: include_str!("../../fixtures/conformance/nullable-alternatives.json"), field: "cases", capacity: false, counts: (15, 6, 1) },
        Suite { name: "dialect", json: include_str!("../../fixtures/conformance/ebv-dialect.json"), field: "cases", capacity: false, counts: (3, 5, 0) },
    ]
}

fn expected_capacity(suite: &Suite, case: &Value) -> EvaluationError {
    // Same reviewed expectations as the importer, bound to unchanged source bytes.
    let registry: Value = serde_json::from_str(include_str!(
        "../../../../bench/zk-bindings/capacity-expectations.json")).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(suite.json.as_bytes()));
    let source = registry["sources"].as_array().unwrap().iter()
        .find(|entry| entry["source_sha256"] == source_hash && entry["field"] == suite.field)
        .expect("unclassified capacity source");
    let expected = &source["cases"].as_array().unwrap().iter()
        .find(|entry| entry["id"] == case["id"]).expect("unclassified capacity case")["expected_rejection"];
    match expected["cause"].as_str() {
        Some("numeric_representation") => EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation),
        Some("temporal_year") => EvaluationError::Capacity(EvaluationCapacity::TemporalYear),
        Some("budget_rows") => EvaluationError::Budget(BudgetExceeded::Rows),
        None if expected["diagnostic"] == "temporal year capacity" => EvaluationError::Rejected(Rejected("temporal year capacity")),
        _ => panic!("unclassified original capacity expectation"),
    }
}

fn assert_capacity(suite: &Suite, case: &Value, actual: EvaluationError) {
    assert_eq!(actual, expected_capacity(suite, case), "{}", case["id"]);
}

#[test]
fn capacity_inventory_requires_each_original_cause() {
    let mut counts = [0; 4];
    for suite in suites() {
        let document: Value = serde_json::from_str(suite.json).unwrap();
        for case in document[suite.field].as_array().unwrap() {
            if !(suite.capacity || case["expected_capacity_error"] == true
                || case["expectation_kind"] == "implementation_capacity") { continue; }
            let expected = expected_capacity(&suite, case);
            let index = match expected {
                EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation) => 0,
                EvaluationError::Capacity(EvaluationCapacity::TemporalYear) => 1,
                EvaluationError::Budget(BudgetExceeded::Rows) => 2,
                EvaluationError::Rejected(Rejected("temporal year capacity")) => 3,
                _ => panic!("wrong original cause"),
            };
            counts[index] += 1;
            for actual in [
                EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation),
                EvaluationError::Capacity(EvaluationCapacity::TemporalYear),
                EvaluationError::Budget(BudgetExceeded::Rows),
                EvaluationError::Budget(BudgetExceeded::Bytes),
                EvaluationError::Budget(BudgetExceeded::Deadline),
                EvaluationError::Budget(BudgetExceeded::Cancelled),
                EvaluationError::Execution,
            ] {
                if actual != expected {
                    assert!(std::panic::catch_unwind(|| assert_capacity(&suite, case, actual)).is_err());
                }
            }
            assert_capacity(&suite, case, expected);
        }
    }
    assert_eq!(counts, [26, 9, 1, 7]);
}

#[test]
fn actual_v3_executes_shared_semantic_and_rejection_inventories() {
    let executor = ExternalProver::new("actual-v3-shared-regressions", PathBuf::from(
        std::env::var_os("RISC0_SERVER_PATH").expect("actual local r0vm required")));
    let versions: Value = serde_json::from_str(include_str!("../../../../bench/zk-bindings/version-expectations.json")).unwrap();
    let mut total = (0, 0, 0);
    for suite in suites() {
        let document: Value = serde_json::from_str(suite.json).unwrap();
        let cases = document[suite.field].as_array().expect("explicit original array");
        let mut counts = (0, 0, 0);
        // Successful original executions precede rejection evidence regardless
        // of fixture ordering. No query or expected result is evaluated to form a golden.
        for negative in [false, true] {
            for case in cases {
                let promoted = versions["entries"].as_array().unwrap().iter()
                    .find(|entry| entry["id"] == case["id"])
                    .and_then(|entry| entry["backend_expectations"].get("exact_v3"));
                let is_capacity = suite.capacity || case["expected_capacity_error"] == true
                    || case["expectation_kind"] == "implementation_capacity";
                let is_profile = promoted.is_none() && (case["admitted"] == false
                    || case["expected_admission_error"] == true || case["expected"]["kind"] == "rejection");
                if negative != (is_capacity || is_profile) { continue; }
                let source = case["dataset_ntriples"].as_str().or_else(|| case["dataset"].as_str())
                    .or_else(|| document["dataset_ntriples"].as_str())
                    .unwrap_or(if suite.name == "original" { include_str!("../../fixtures/conformance/core.nt") } else { "" });
                let mut policy = v3::Policy::default();
                if let Some(rows) = case["max_rows"].as_u64() {
                    policy.dataset.max_rows = rows.try_into().unwrap();
                }
                let dataset = v3::PrivateDataset { nquads: source.into(), named_graphs: vec![], salt: [89; 32] };
                let input = v3::Witness {
                    request: v3::Request {
                        version: v3::VERSION, contract: ProofContract::ExactDataset,
                        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
                        query: case["query"].as_str().unwrap().into(),
                        authority: DatasetAuthority::VerifierAgreed { commitment: v3::dataset_commitment(&dataset, &policy).unwrap() },
                        policy, nonce: [97; 32],
                    }, dataset,
                };
                if is_capacity {
                    assert_capacity(&suite, case, v3::evaluate_detailed(&input).expect_err("typed capacity preflight"));
                } else if is_profile {
                    assert!(v3::admit(&input.request).is_err(), "{}: original profile rejection", case["id"]);
                }
                eprintln!("actual V3 {} {}", suite.name, case["id"]);
                let env = ExecutorEnv::builder().session_limit(Some(1 << 25)).write(&input).unwrap().build().unwrap();
                let execution = executor.execute(env, embedded_artifact());
                if negative {
                    assert!(total.0 + counts.0 > 0, "positive actual execution must precede rejection evidence");
                    let message = format!("{:#}", execution.expect_err("relation rejection must not produce a journal"));
                    assert!(message.contains("Guest panicked:") && message.contains("bounded exact-dataset relation rejected"), "{}: {message}", case["id"]);
                    if is_capacity { counts.2 += 1; } else { counts.1 += 1; }
                } else {
                    let session = execution.unwrap_or_else(|error| panic!("{}: {error:#}", case["id"]));
                    let journal: v3::Journal = session.journal.decode().unwrap();
                    v3::bind_journal(&journal, &input.request).unwrap();
                    let expected = promoted.map(|entry| &entry["expected"])
                        .or_else(|| case.get("expected_result"))
                        .or_else(|| case["expected"].get("result"));
                    if let Some(expected) = expected {
                        assert_eq!(serde_json::json!(journal.result), *expected, "{}", case["id"]);
                    } else {
                        let v3::CanonicalResult::Select { rows, .. } = journal.result else { panic!("row-only original requires SELECT") };
                        assert!(case["expected_rows"].is_array(), "missing independent original golden");
                        assert_eq!(serde_json::json!(rows), case["expected_rows"], "{}", case["id"]);
                    }
                    counts.0 += 1;
                }
            }
        }
        assert_eq!(counts, suite.counts, "{} inventory", suite.name);
        total.0 += counts.0; total.1 += counts.1; total.2 += counts.2;
    }
    assert_eq!(total, (419, 59, 43));
}

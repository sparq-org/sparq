// Typed causes are native evidence, not guest receipt execution.
#![cfg(feature = "evaluate")]

use sparq_engine::{BudgetExceeded, EvaluationCapacity, QueryFailure};
use sparq_proved_evaluator_model::{
    DatasetAuthority, Dialect, EvaluationError, Policy, PrivateDataset, ProofContract, Rejected,
    Request, VERSION, Witness, dataset_commitment, evaluate, evaluate_detailed,
};

fn witness(query: &str, data: &str) -> Witness {
    let dataset = PrivateDataset {
        ntriples: data.into(),
        salt: [7; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: query.into(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [11; 32],
        },
        dataset,
    }
}

fn expect_capacity(case: &serde_json::Value, cause: EvaluationCapacity) {
    let input = witness(
        case["query"].as_str().unwrap(),
        case["dataset_ntriples"].as_str().unwrap_or_default(),
    );
    for authority in [
        input.request.authority.clone(),
        DatasetAuthority::HolderDeclared,
    ] {
        let mut input = input.clone();
        input.request.authority = authority;
        let error =
            evaluate_detailed(&input).expect_err(&format!("{}: expected capacity", case["id"]));
        assert_eq!(error, EvaluationError::Capacity(cause), "{}", case["id"]);
        assert_eq!(evaluate(&input).unwrap_err(), Rejected::from(error));
    }
}

#[test]
fn numeric_cases_keep_exact_results_and_actual_typed_causes() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/numeric-capacity.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 28);
    let mut counts = (0, 0);
    let mut promoted = std::collections::BTreeSet::new();
    let mut replaced = std::collections::BTreeSet::new();
    for case in cases {
        if case["expectation_kind"] == "implementation_capacity" {
            counts.1 += 1;
            expect_capacity(case, EvaluationCapacity::NumericRepresentation);
            if let Some(id) = case["capacity_replacement_for"].as_str() {
                assert!(replaced.insert(id), "duplicate capacity replacement: {id}");
            }
        } else {
            counts.0 += 1;
            assert!(promoted.insert(case["id"].as_str().unwrap()));
            let input = witness(
                case["query"].as_str().unwrap(),
                case["dataset_ntriples"].as_str().unwrap(),
            );
            for authority in [
                input.request.authority.clone(),
                DatasetAuthority::HolderDeclared,
            ] {
                let mut input = input.clone();
                input.request.authority = authority;
                let journal = evaluate_detailed(&input)
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["id"]));
                assert_eq!(
                    serde_json::json!(journal.result),
                    case["expected_result"],
                    "{}",
                    case["id"]
                );
                assert_eq!(
                    serde_json::to_vec(&journal).unwrap(),
                    serde_json::to_vec(&evaluate(&input).unwrap()).unwrap()
                );
            }
        }
    }
    assert_eq!(counts, (7, 21));
    assert_eq!(
        promoted, replaced,
        "each promotion retains a typed capacity control"
    );
}

#[test]
fn remaining_original_ambiguous_cases_keep_their_actual_domain_cause() {
    // These six original dynamic temporal cases reached evaluation, unlike
    // the corpus's static admission/data rejections. Do not reclassify the
    // latter merely because both groups deliberately exceed a profile bound.
    let temporal: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/temporal-capacity.json"
    ))
    .unwrap();
    let ids = [
        "dynamic-coalesce",
        "dynamic-bind-false-ask",
        "dynamic-filter-false-ask",
        "dynamic-cast",
        "zero-year-strdt",
        "zero-year-cast",
    ];
    for id in ids {
        let case = temporal["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == id)
            .expect("original temporal fixture must remain present");
        expect_capacity(case, EvaluationCapacity::TemporalYear);
    }
    let builtin: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    for id in ["integer-cast-outside-range", "integer-cast-double-overflow"] {
        let case = builtin["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == id)
            .expect("original constructor capacity fixture must remain present");
        assert_eq!(case["expectation_kind"], "implementation_capacity");
        expect_capacity(case, EvaluationCapacity::NumericRepresentation);
    }
}

#[test]
fn distinct_errors_and_success_preserve_legacy_outcomes_and_journal_bytes() {
    let query = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> ASK { BIND(STRDT(\"0000-01-01T00:00:00Z\", xsd:dateTime) AS ?v) }";
    assert_eq!(
        evaluate_detailed(&witness(query, "")).unwrap_err(),
        EvaluationError::Capacity(EvaluationCapacity::TemporalYear)
    );
    let invalid = witness("not SPARQL", "");
    assert_eq!(
        evaluate_detailed(&invalid).unwrap_err(),
        EvaluationError::Rejected(Rejected("SPARQL parse rejected"))
    );
    let success = witness("SELECT (1/0 AS ?v) {}", "");
    assert_eq!(
        serde_json::to_vec(&evaluate_detailed(&success).unwrap()).unwrap(),
        serde_json::to_vec(&evaluate(&success).unwrap()).unwrap()
    );
    let mut capped = witness("SELECT ?v { VALUES ?v {1 2} }", "");
    capped.request.policy.max_rows = 1;
    capped.request.authority = DatasetAuthority::VerifierAgreed {
        commitment: dataset_commitment(&capped.dataset, &capped.request.policy).unwrap(),
    };
    assert_eq!(
        evaluate_detailed(&capped).unwrap_err(),
        EvaluationError::Budget(BudgetExceeded::Rows)
    );
    // Strings cannot manufacture a typed cause, even if identical to an emitter's message.
    let forged = QueryFailure::Evaluation(
        "query evaluation capacity exceeded (numeric-representation)".into(),
    );
    assert_eq!(EvaluationError::from(forged), EvaluationError::Execution);
    assert_ne!(
        EvaluationError::from(QueryFailure::Budget(BudgetExceeded::Cancelled)),
        EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation)
    );
}

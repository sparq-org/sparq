// [GPT-6] Standards-derived host semantic cases, not guest proof evidence.
#![cfg(feature = "evaluate")]

use serde::Deserialize;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    RowOrder, VERSION, Witness, admit, dataset_commitment, evaluate,
};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    authored_by: String,
    fixture_origin: String,
    default_dataset: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    features: Vec<String>,
    spec: String,
    query: String,
    dataset: Option<String>,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Expected {
    Result { result: CanonicalResult },
    Rejection { stage: Stage, error: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Admission,
    Dataset,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("../../fixtures/conformance/cases.json"))
        .expect("valid committed conformance corpus")
}

fn witness(case: &Case) -> Witness {
    let dataset = PrivateDataset {
        ntriples: case
            .dataset
            .as_deref()
            .unwrap_or(include_str!("../../fixtures/conformance/core.nt"))
            .to_owned(),
        // Fixed synthetic fixture values; never use these as production entropy.
        salt: [17; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: case.query.clone(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).expect("bounded fixture"),
            },
            policy,
            nonce: [29; 32],
        },
        dataset,
    }
}

fn canonical_bag(mut result: CanonicalResult) -> CanonicalResult {
    if let CanonicalResult::Select {
        order: RowOrder::Bag,
        rows,
        ..
    } = &mut result
    {
        // An unordered oracle still checks exact multiplicities and unbound cells.
        rows.sort();
    }
    result
}

#[test]
fn deterministic_query_results_match_normative_expectations() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for case in corpus().cases {
        let Expected::Result { ref result } = case.expected else {
            continue;
        };
        executed += 1;
        let input = witness(&case);
        if let Err(error) = admit(&input.request) {
            failures.push(format!("{}: admission rejected: {error}", case.id));
            continue;
        }
        match evaluate(&input) {
            Ok(journal) => {
                let expected = canonical_bag(result.clone());
                if journal.result != expected {
                    failures.push(format!(
                        "{}: expected {expected:?}; actual {:?}",
                        case.id, journal.result
                    ));
                }
            }
            Err(error) => failures.push(format!("{}: evaluation rejected: {error}", case.id)),
        }
    }
    assert!(executed > 0, "positive corpus must not be empty");
    assert!(
        failures.is_empty(),
        "{} failures in {executed} host semantic cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn unsupported_features_fail_at_the_declared_boundary() {
    let mut failures = Vec::new();
    let mut executed = 0;
    for case in corpus().cases {
        let Expected::Rejection {
            ref stage,
            ref error,
        } = case.expected
        else {
            continue;
        };
        executed += 1;
        let input = witness(&case);
        let admitted = admit(&input.request);
        let admission_matches = match stage {
            Stage::Admission => admitted.as_ref().is_err_and(|e| e.0 == error),
            Stage::Dataset => admitted.is_ok(),
        };
        if !admission_matches {
            failures.push(format!("{}: unexpected admission {admitted:?}", case.id));
        }
        let actual = evaluate(&input);
        if !actual.as_ref().is_err_and(|e| e.0 == error) {
            failures.push(format!(
                "{}: expected rejection {error:?}; actual {actual:?}",
                case.id
            ));
        }
    }
    assert!(executed > 0, "negative corpus must not be empty");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn corpus_metadata_and_oracles_are_well_formed() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.authored_by, "GPT-6");
    assert!(
        corpus
            .fixture_origin
            .contains("not generated from evaluator output")
    );
    assert_eq!(corpus.default_dataset, "core.nt");
    let mut ids = BTreeSet::new();
    for case in corpus.cases {
        assert!(ids.insert(case.id.clone()), "duplicate case {}", case.id);
        assert!(
            !case.features.is_empty(),
            "{}: feature IDs required",
            case.id
        );
        assert!(
            case.spec.starts_with("https://www.w3.org/"),
            "{}: primary reference",
            case.id
        );
        assert!(!case.query.is_empty(), "{}: query required", case.id);
        if let Expected::Result {
            result: CanonicalResult::Select {
                variables, rows, ..
            },
        } = case.expected
        {
            assert!(
                rows.iter().all(|r| r.len() == variables.len()),
                "{}: row width",
                case.id
            );
        }
    }
}

// [GPT-6] Every coverage claim points to an executed case of the correct kind.
#[test]
fn coverage_manifest_cannot_count_rejections_as_support() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!("../../coverage.json"))
        .expect("valid committed coverage manifest");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["evidence"]["layer"], "host_semantics");
    assert_eq!(manifest["evidence"]["guest_proof_coverage"], "not_asserted");
    assert_eq!(
        manifest["evidence"]["negative_cases_are_feature_support"],
        false
    );
    let corpus = corpus();
    let features = manifest["features"].as_array().expect("feature inventory");
    let mut feature_ids = BTreeSet::new();
    for feature in features {
        let id = feature["id"].as_str().expect("feature ID");
        assert!(feature_ids.insert(id), "duplicate feature {id}");
        let positive: Vec<_> = corpus
            .cases
            .iter()
            .filter(|c| {
                c.features.iter().any(|f| f == id) && matches!(c.expected, Expected::Result { .. })
            })
            .map(|c| c.id.as_str())
            .collect();
        let negative: Vec<_> = corpus
            .cases
            .iter()
            .filter(|c| {
                c.features.iter().any(|f| f == id)
                    && matches!(c.expected, Expected::Rejection { .. })
            })
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            feature["positive_cases"],
            serde_json::json!(positive),
            "{id}: positive linkage"
        );
        assert_eq!(
            feature["rejection_cases"],
            serde_json::json!(negative),
            "{id}: rejection linkage"
        );
        assert_eq!(
            feature["guest_proof_evidence"], "not_asserted",
            "{id}: evidence layer"
        );
        match feature["status"].as_str().expect("feature status") {
            "partial_fixture_coverage" => {
                assert!(!positive.is_empty(), "{id}: needs positive fixture")
            }
            "known_gap" => {
                assert!(
                    positive.contains(&feature["failing_case"].as_str().expect("failing case"))
                );
                assert!(feature["implementation_requirement"].is_string());
            }
            "rejected" => assert!(
                positive.is_empty() && !negative.is_empty(),
                "{id}: rejection is not coverage"
            ),
            "unassessed" => assert!(
                positive.is_empty() && negative.is_empty(),
                "{id}: unassessed has no fixture"
            ),
            other => panic!("{id}: unknown coverage status {other}"),
        }
    }
    for case in &corpus.cases {
        for feature in &case.features {
            assert!(
                feature_ids.contains(feature.as_str()),
                "{}: missing feature {feature}",
                case.id
            );
        }
    }
    let mut function_names = BTreeSet::new();
    for function in manifest["functions"]
        .as_array()
        .expect("function inventory")
    {
        let name = function["name"].as_str().expect("function name");
        assert!(function_names.insert(name), "duplicate function {name}");
        assert!(feature_ids.contains(function["group"].as_str().expect("function group")));
        assert_eq!(function["guest_proof_evidence"], "not_asserted");
        for (key, positive) in [("positive_cases", true), ("rejection_cases", false)] {
            for id in function[key].as_array().expect("function cases") {
                let id = id.as_str().expect("function case ID");
                let case = corpus
                    .cases
                    .iter()
                    .find(|c| c.id == id)
                    .expect("function references executable case");
                assert_eq!(
                    matches!(case.expected, Expected::Result { .. }),
                    positive,
                    "{name}: evidence kind"
                );
            }
        }
        assert!(
            matches!(
                function["status"].as_str(),
                Some("rejected" | "known_gap" | "partial_fixture_coverage")
            ),
            "{name}: unknown function coverage status"
        );
        let key = if function["status"] == "rejected" {
            "rejection_cases"
        } else {
            "positive_cases"
        };
        assert!(
            !function[key]
                .as_array()
                .expect("function evidence")
                .is_empty(),
            "{name}: unsupported claim"
        );
    }
}

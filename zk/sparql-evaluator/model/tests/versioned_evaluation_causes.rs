// [GPT-6] Original queries and goldens remain immutable; these are native definitions.
#![cfg(feature = "graph-results")]

use sparq_engine::{BudgetExceeded, EvaluationCapacity};
use sparq_proved_evaluator_model::{
    DatasetAuthority, EvaluationError, ProofContract, Rejected, v2, v3,
};

fn inputs(query: &str, data: &str, agreed: bool) -> (v2::Witness, v3::Witness) {
    let dataset = v2::PrivateDataset { nquads: data.into(), named_graphs: vec![], salt: [7; 32] };
    let p2 = v2::Policy::default();
    let p3 = v3::Policy::default();
    let a2 = if agreed {
        DatasetAuthority::VerifierAgreed { commitment: v2::dataset_commitment(&dataset, &p2).unwrap() }
    } else { DatasetAuthority::HolderDeclared };
    let a3 = if agreed {
        DatasetAuthority::VerifierAgreed { commitment: v3::dataset_commitment(&dataset, &p3).unwrap() }
    } else { DatasetAuthority::HolderDeclared };
    (v2::Witness {
        request: v2::Request { version: v2::VERSION, contract: ProofContract::ExactDataset, dialect: v2::Dialect::SparqSparql11DatasetV2, query: query.into(), authority: a2, policy: p2, nonce: [11; 32] },
        dataset: dataset.clone(),
    }, v3::Witness {
        request: v3::Request { version: v3::VERSION, contract: ProofContract::ExactDataset, dialect: v3::Dialect::SparqSparql11GraphResultsV3, query: query.into(), authority: a3, policy: p3, nonce: [11; 32] },
        dataset,
    })
}

fn original_cases() -> Vec<(serde_json::Value, EvaluationCapacity)> {
    let numeric: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/conformance/numeric-capacity.json")).unwrap();
    let mut cases: Vec<_> = numeric["cases"].as_array().unwrap().iter().cloned().map(|case| (case, EvaluationCapacity::NumericRepresentation)).collect();
    assert_eq!(cases.len(), 21);
    let temporal: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/conformance/temporal-capacity.json")).unwrap();
    for id in ["dynamic-coalesce", "dynamic-bind-false-ask", "dynamic-filter-false-ask", "dynamic-cast", "zero-year-strdt", "zero-year-cast"] {
        let case = temporal["cases"].as_array().unwrap().iter().find(|case| case["id"] == id).expect("retained temporal fixture").clone();
        cases.push((case, EvaluationCapacity::TemporalYear));
    }
    let builtin: serde_json::Value = serde_json::from_str(include_str!("../../../../crates/sparq-engine/tests/fixtures/builtin_edges.json")).unwrap();
    for id in ["integer-cast-outside-range", "integer-cast-double-overflow"] {
        let case = builtin["cases"].as_array().unwrap().iter().find(|case| case["id"] == id).expect("retained capacity control").clone();
        assert_eq!(case["expectation_kind"], "implementation_capacity");
        cases.push((case, EvaluationCapacity::NumericRepresentation));
    }
    assert_eq!(cases.len(), 29);
    cases
}

#[test]
fn original_29_capacity_ids_keep_typed_causes_in_both_versions_and_authorities() {
    let mut contexts = [0; 2];
    for (case, cause) in original_cases() {
        for agreed in [false, true] {
            let (w2, w3) = inputs(case["query"].as_str().unwrap(), case["dataset_ntriples"].as_str().unwrap_or_default(), agreed);
            assert_eq!(v2::evaluate_detailed(&w2).unwrap_err(), EvaluationError::Capacity(cause), "V2 {}", case["id"]);
            assert_eq!(v3::evaluate_detailed(&w3).unwrap_err(), EvaluationError::Capacity(cause), "V3 {}", case["id"]);
            assert_eq!(v2::evaluate(&w2).unwrap_err(), Rejected("query evaluation or resource budget rejected"));
            assert_eq!(v3::evaluate(&w3).unwrap_err(), Rejected("V3 query evaluation or resource budget rejected"));
            contexts[0] += 1;
            contexts[1] += 1;
        }
    }
    assert_eq!(contexts, [58, 58]);
}

#[test]
fn graph_forms_capture_actual_capacity_and_keep_their_legacy_message() {
    for form in ["CONSTRUCT { ?s <http://ex/p> <http://ex/o> }", "DESCRIBE ?s"] {
        for (expression, cause) in [
            ("9223372036854775807 + 1", EvaluationCapacity::NumericRepresentation),
            ("STRDT(\"0000-01-01T00:00:00Z\", <http://www.w3.org/2001/XMLSchema#dateTime>)", EvaluationCapacity::TemporalYear),
        ] {
            let text = format!("{form} WHERE {{ BIND(<http://ex/a> AS ?s) BIND({expression} AS ?value) FILTER(BOUND(?value)) }}");
            for agreed in [false, true] {
                let (_, w3) = inputs(&text, "", agreed);
                assert_eq!(v3::evaluate_detailed(&w3).unwrap_err(), EvaluationError::Capacity(cause));
                assert_eq!(v3::evaluate(&w3).unwrap_err(), Rejected("V3 graph evaluation or resource budget rejected"));
            }
        }
    }
}

#[test]
fn success_journal_bytes_and_parse_resource_categories_remain_distinct() {
    for agreed in [false, true] {
        for text in ["ASK {}", "SELECT (1 / 0 AS ?v) {}", "SELECT ?s WHERE { ?s ?p ?o }"] {
            let (w2, w3) = inputs(text, "", agreed);
            assert_eq!(serde_json::to_vec(&v2::evaluate_detailed(&w2).unwrap()).unwrap(), serde_json::to_vec(&v2::evaluate(&w2).unwrap()).unwrap());
            assert_eq!(serde_json::to_vec(&v3::evaluate_detailed(&w3).unwrap()).unwrap(), serde_json::to_vec(&v3::evaluate(&w3).unwrap()).unwrap());
        }
        for text in ["CONSTRUCT { <http://ex/s> <http://ex/p> ?v } WHERE { BIND(1/0 AS ?v) }", "DESCRIBE <http://ex/s>"] {
            let (_, w3) = inputs(text, "<http://ex/s> <http://ex/p> <http://ex/o> .", agreed);
            assert_eq!(serde_json::to_vec(&v3::evaluate_detailed(&w3).unwrap()).unwrap(), serde_json::to_vec(&v3::evaluate(&w3).unwrap()).unwrap());
        }
        let (w2, w3) = inputs("not SPARQL", "", agreed);
        assert_eq!(v2::evaluate_detailed(&w2).unwrap_err(), EvaluationError::Rejected(Rejected("SPARQL parse rejected")));
        assert_eq!(v3::evaluate_detailed(&w3).unwrap_err(), EvaluationError::Rejected(Rejected("SPARQL parse rejected")));
        let (mut w2, mut w3) = inputs("SELECT ?v { VALUES ?v {1 2} }", "", false);
        w2.request.policy.max_rows = 1;
        w3.request.policy.dataset.max_rows = 1;
        if agreed {
            w2.request.authority = DatasetAuthority::VerifierAgreed { commitment: v2::dataset_commitment(&w2.dataset, &w2.request.policy).unwrap() };
            w3.request.authority = DatasetAuthority::VerifierAgreed { commitment: v3::dataset_commitment(&w3.dataset, &w3.request.policy).unwrap() };
        }
        assert_eq!(v2::evaluate_detailed(&w2).unwrap_err(), EvaluationError::Budget(BudgetExceeded::Rows));
        assert_eq!(v3::evaluate_detailed(&w3).unwrap_err(), EvaluationError::Budget(BudgetExceeded::Rows));
    }
}

#[test]
fn original_dialect_controls_apply_to_both_versioned_admission_and_execution() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/conformance/ebv-dialect.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 8);
    for case in corpus["cases"].as_array().unwrap() {
        for agreed in [false, true] {
            let (w2, w3) = inputs(case["query"].as_str().unwrap(), "", agreed);
            if case["admitted"] == true {
                v2::admit(&w2.request).unwrap();
                v3::admit(&w3.request).unwrap();
                assert_eq!(serde_json::to_value(v2::evaluate_detailed(&w2).unwrap().result).unwrap(), case["expected_result"]);
                assert_eq!(serde_json::to_value(v3::evaluate_detailed(&w3).unwrap().result).unwrap(), case["expected_result"]);
            } else {
                // All labels are syntactically valid strings in the pinned parser.
                // PreparedQuery additionally rejects unknown/incompatible labels
                // before the proof profile applies its explicit REC restriction.
                spargebra::SparqlParser::new().parse_query(&w2.request.query).unwrap();
                let preparation_error = match case["id"].as_str().unwrap() {
                    "unknown-version" => Some("unsupported SPARQL VERSION announcement"),
                    "contradictory-declarations" => Some("SPARQL VERSION announcements require incompatible EBV semantics"),
                    "conflicting-wd" | "conflicting-wd-basic"
                    | "compatible-draft-labels-conflict-with-proof-rec" => None,
                    id => panic!("unclassified original VERSION rejection: {id}"),
                };
                let (rejection, execution) = if let Some(expected) = preparation_error {
                    assert_eq!(sparq_engine::PreparedQuery::parse(&w2.request.query).unwrap_err(), expected);
                    let rejection = Rejected("SPARQL parse rejected");
                    // [OPUS-5.5] Rejected is Clone, not Copy: admission and execution each own one.
                    (rejection.clone(), EvaluationError::Rejected(rejection))
                } else {
                    sparq_engine::PreparedQuery::parse(&w2.request.query).unwrap();
                    (Rejected("query VERSION contradicts REC 2013 profile"), EvaluationError::Execution)
                };
                assert_eq!(v2::admit(&w2.request).unwrap_err(), rejection);
                assert_eq!(v3::admit(&w3.request).unwrap_err(), rejection);
                assert_eq!(v2::evaluate_detailed(&w2).unwrap_err(), execution);
                assert_eq!(v3::evaluate_detailed(&w3).unwrap_err(), execution);
            }
        }
    }
}

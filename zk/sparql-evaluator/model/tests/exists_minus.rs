// [GPT-6] Published-2013 domain substitution and its exact proof-profile boundary.
#![cfg(feature = "evaluate")]
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, Dialect, Policy, PrivateDataset, ProofContract, Request,
    VERSION, Witness, dataset_commitment,
};
use sparq_proved_evaluator_model::{admit, evaluate};

fn input(query: &str, ntriples: &str) -> Witness {
    let policy = Policy::default();
    let dataset = PrivateDataset {
        ntriples: ntriples.into(),
        salt: [61; 32],
    };
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
            nonce: [67; 32],
        },
        dataset,
    }
}

#[test]
fn minus_domains_match_published_goldens() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-minus-domains.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let witness = input(
            case["query"].as_str().unwrap(),
            corpus["dataset_ntriples"].as_str().unwrap(),
        );
        admit(&witness.request).unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
        let expected: CanonicalResult =
            serde_json::from_value(case["expected_result"].clone()).unwrap();
        assert_eq!(
            evaluate(&witness).unwrap().result,
            expected,
            "{}",
            case["id"]
        );
    }
}

#[test]
fn false_ask_fixture_discriminates_domain_repair() {
    let witness = input(
        include_str!("../../fixtures/false-absence.rq"),
        include_str!("../../fixtures/default.nt"),
    );
    assert_eq!(
        evaluate(&witness).unwrap().result,
        CanonicalResult::Ask(false)
    );
}

#[test]
fn minus_does_not_admit_captured_bound_or_nested_binders() {
    for body in [
        "?s <http://ex/p> ?m FILTER(BOUND(?s)) MINUS {?s <http://ex/p> ?z}",
        "?s <http://ex/p> ?m MINUS {VALUES ?z {1}}",
        "?s <http://ex/p> ?m MINUS {BIND(1 AS ?z)}",
        "?s <http://ex/p> ?m MINUS {{SELECT ?s {?s <http://ex/p> ?z}}}",
        "?s <http://ex/p> ?m MINUS {GRAPH ?g {?s <http://ex/p> ?z}}",
        "?s <http://ex/p> ?m MINUS {?s <http://ex/p>* ?z}",
        "?s <http://ex/p> ?m MINUS {?s <http://ex/p> ?z FILTER EXISTS {}}",
    ] {
        let witness = input(
            &format!("SELECT ?s {{VALUES ?s {{<http://ex/a>}} FILTER EXISTS {{{body}}}}}"),
            "",
        );
        assert!(admit(&witness.request).is_err(), "{body}");
        assert!(evaluate(&witness).is_err(), "{body}");
    }
}

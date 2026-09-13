// [GPT-6] Bound context semantics are distinct from wall-clock observations.
#![cfg(feature = "graph-results")]
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, Provenance, v3, v4};

const NOW: &str = "2026-09-13T01:02:03.000000001+01:00";
const DATATYPE: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

fn witness(query: &str) -> v4::Witness {
    v4::Witness {
        request: v4::Request {
            version: v4::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v4::Dialect::SparqSparql11NowContextV4,
            query: query.into(),
            context: v4::ExecutionContext {
                now: v4::NowContext {
                    datetime: NOW.into(),
                },
            },
            authority: DatasetAuthority::HolderDeclared,
            policy: v4::Policy::default(),
            nonce: [83; 32],
        },
        dataset: v4::PrivateDataset {
            nquads: String::new(),
            named_graphs: vec![],
            salt: [89; 32],
        },
    }
}

fn rows(input: &v4::Witness) -> Vec<Vec<Option<String>>> {
    v4::admit(&input.request).unwrap();
    let journal = v4::evaluate(input).unwrap();
    v4::bind_journal(&journal, &input.request).unwrap();
    assert_eq!(journal.context, input.request.context);
    let v4::CanonicalResult::Select { rows, .. } = journal.result else {
        panic!("SELECT expected")
    };
    rows
}

#[test]
fn shared_now_context_goldens_preserve_both_authority_modes() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/now-context.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let mut input = witness(case["query"].as_str().unwrap());
        input.dataset.nquads = case["nquads"].as_str().unwrap().into();
        for agreed in [false, true] {
            input.request.authority = if agreed {
                DatasetAuthority::VerifierAgreed {
                    commitment: v4::dataset_commitment(&input.dataset, &input.request.policy)
                        .unwrap(),
                }
            } else {
                DatasetAuthority::HolderDeclared
            };
            v4::admit(&input.request).unwrap();
            let journal = v4::evaluate(&input).unwrap();
            v4::bind_journal(&journal, &input.request).unwrap();
            let expected: v4::CanonicalResult =
                serde_json::from_value(case["expected_result"].clone()).unwrap();
            assert_eq!(journal.result, expected, "{} agreed={agreed}", case["id"]);
        }
    }
}

#[test]
fn same_now_literal_reaches_rows_nested_calls_subqueries_and_aggregates() {
    let literal = Some(format!("\"{NOW}\"^^<{DATATYPE}>"));
    let query = "SELECT (NOW() AS ?a) (IF(true, COALESCE(NOW(), NOW()), NOW()) AS ?b) { VALUES ?n {1 1} } ORDER BY NOW()";
    let input = witness(query);
    assert_eq!(
        rows(&input),
        vec![vec![literal.clone(), literal.clone()]; 2]
    );
    assert_eq!(
        input.request.query, query,
        "specialization must preserve original bytes"
    );
    let input = witness(
        "SELECT ?a (NOW() AS ?b) { { SELECT (NOW() AS ?a) {} } FILTER(sameTerm(?a, NOW())) }",
    );
    assert_eq!(rows(&input), vec![vec![literal.clone(), literal.clone()]]);
    let input = witness("SELECT (MIN(NOW()) AS ?a) (MAX(NOW()) AS ?b) {VALUES ?n {1 2}}");
    assert_eq!(rows(&input), vec![vec![literal.clone(), literal]]);
    let input = witness("SELECT (\"NOW()\" AS ?text) {} # NOW() is a comment");
    assert_eq!(rows(&input), vec![vec![Some("\"NOW()\"".into())]]);
}

#[test]
fn now_obeys_exact_timezone_and_graph_result_semantics() {
    let mut input = witness(&format!(
        "ASK {{ FILTER(NOW() = \"2026-09-13T00:02:03.000000001Z\"^^<{DATATYPE}>) }}"
    ));
    assert_eq!(
        v4::evaluate(&input).unwrap().result,
        v4::CanonicalResult::Ask(true)
    );
    input.request.query =
        "CONSTRUCT {<http://ex/s> <http://ex/at> ?at} WHERE {BIND(NOW() AS ?at)}".into();
    let v4::CanonicalResult::Graph { ntriples } = v4::evaluate(&input).unwrap().result else {
        panic!("graph")
    };
    assert_eq!(
        ntriples,
        format!("<http://ex/s> <http://ex/at> \"{NOW}\"^^<{DATATYPE}> .\n")
    );
    input.dataset.nquads = "<http://ex/s> <http://ex/p> <http://ex/o> .".into();
    input.request.query = "DESCRIBE <http://ex/s> WHERE {FILTER(NOW() = NOW())}".into();
    let v4::CanonicalResult::Graph { ntriples } = v4::evaluate(&input).unwrap().result else {
        panic!("graph")
    };
    assert_eq!(ntriples, input.dataset.nquads + "\n");
}

#[test]
fn original_query_context_and_authority_are_independently_bound() {
    let mut input = witness("SELECT (NOW() AS ?n) {}");
    let source_anchor = v4::dataset_commitment(&input.dataset, &input.request.policy).unwrap();
    assert_ne!(
        source_anchor,
        v3::dataset_commitment(&input.dataset, &input.request.policy).unwrap()
    );
    for authority in [
        DatasetAuthority::HolderDeclared,
        DatasetAuthority::VerifierAgreed {
            commitment: source_anchor,
        },
    ] {
        input.request.authority = authority;
        let journal = v4::evaluate(&input).unwrap();
        v4::bind_journal(&journal, &input.request).unwrap();
        let expected_provenance =
            if matches!(input.request.authority, DatasetAuthority::HolderDeclared) {
                Provenance::HolderDeclaredOnly
            } else {
                Provenance::VerifierAcceptedCommitment
            };
        assert_eq!(journal.provenance, expected_provenance);
        let mut expected = input.request.clone();
        expected.context.now.datetime = "2026-09-13T00:02:03.000000001Z".into();
        assert!(
            v4::bind_journal(&journal, &expected).is_err(),
            "equivalent dateTime values have distinct expected lexical contexts"
        );
        let mut tampered = journal.clone();
        tampered.context = expected.context;
        assert!(v4::bind_journal(&tampered, &input.request).is_err());
        expected = input.request.clone();
        expected.query.push_str(" # bytes differ");
        assert!(v4::bind_journal(&journal, &expected).is_err());
        expected = input.request.clone();
        expected.nonce[0] ^= 1;
        assert!(v4::bind_journal(&journal, &expected).is_err());
        expected = input.request.clone();
        expected.version = v3::VERSION;
        assert!(v4::bind_journal(&journal, &expected).is_err());
    }
    input.request.context.now.datetime = "2027-01-01T00:00:00Z".into();
    assert_eq!(
        source_anchor,
        v4::dataset_commitment(&input.dataset, &input.request.policy).unwrap()
    );
    input.request.authority = DatasetAuthority::VerifierAgreed {
        commitment: [0; 32],
    };
    assert!(v4::evaluate(&input).is_err());
}

#[test]
fn invalid_context_rejects_even_without_now_or_with_dead_expressions() {
    let invalid = [
        "",
        "2026-09-13T01:02:03",
        "2026-02-30T00:00:00Z",
        "2026-01-01T24:01:00Z",
        "2026-01-01T00:00:00+14:01",
        "0000-01-01T00:00:00Z",
        "-0001-01-01T00:00:00Z",
        "1000000001-01-01T00:00:00Z",
        "2026-01-01T00:00:60Z",
        " 2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z\t",
    ];
    for query in ["ASK {}", "SELECT (IF(false,NOW(),1) AS ?n) {}"] {
        for context in invalid {
            let mut input = witness(query);
            input.request.context.now.datetime = context.into();
            assert!(v4::admit(&input.request).is_err(), "{context}");
            assert!(v4::evaluate(&input).is_err(), "{context}");
        }
    }
    for context in [
        "0001-01-01T00:00:00Z",
        "1000000000-12-31T23:59:59.123456789012345678901Z",
    ] {
        let mut input = witness("SELECT (NOW() AS ?n) {}");
        input.request.context.now.datetime = context.into();
        assert_eq!(
            rows(&input),
            vec![vec![Some(format!(
                "\"{}\"^^<{DATATYPE}>",
                context.replace('\t', "\\t")
            ))]]
        );
    }
}

#[test]
fn replacement_capacity_and_inherited_exclusions_fail_closed() {
    let mut input = witness("ASK {}");
    input.request.context.now.datetime = "x".repeat(v4::MAX_NOW_BYTES + 1);
    assert_eq!(
        v4::admit(&input.request).unwrap_err().0,
        "V4 NOW context byte capacity"
    );
    input.request.context.now.datetime = format!(
        "2026-01-01T00:00:00.{}Z",
        "0".repeat(v4::MAX_NOW_BYTES - 21)
    );
    assert_eq!(input.request.context.now.datetime.len(), v4::MAX_NOW_BYTES);
    for count in [64, 65] {
        input.request.query = format!(
            "SELECT (COALESCE({}) AS ?n) {{}}",
            vec!["NOW()"; count].join(",")
        );
        if count == 64 {
            assert_eq!(rows(&input).len(), 1);
        } else {
            assert_eq!(
                v4::admit(&input.request).unwrap_err().0,
                "V4 context expansion capacity"
            );
        }
    }
    for expression in ["RAND()", "UUID()", "STRUUID()", "BNODE()"] {
        assert!(v4::admit(&witness(&format!("SELECT ({expression} AS ?n) {{}}")).request).is_err());
    }
    let mut input = witness("ASK {FILTER EXISTS {FILTER(NOW()=NOW())}}");
    assert_eq!(
        v4::evaluate(&input).unwrap().result,
        v4::CanonicalResult::Ask(true)
    );
    input.dataset.nquads = "_:source <http://ex/p> <http://ex/o> .".into();
    assert_eq!(
        v4::evaluate(&input).unwrap_err().0,
        "V3 EXISTS blank-node correlation is not admitted"
    );
    assert!(
        v4::admit(&witness("SELECT ?n {VALUES ?n {1} FILTER EXISTS {FILTER(BOUND(?n))}}").request)
            .is_err()
    );
    assert!(v4::admit(&witness("ASK {SERVICE <http://ex/remote> {}}").request).is_err());
}

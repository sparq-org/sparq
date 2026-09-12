// [GPT-6] Native V2 semantics and commitment tests; not receipt evidence.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::v2::*;
use sparq_proved_evaluator_model::{
    CanonicalResult, DatasetAuthority, ProofContract, Provenance, RowOrder,
};

fn witness(query: &str) -> Witness {
    let dataset = PrivateDataset {
        nquads: concat!(
            "<http://ex/default> <http://ex/p> <http://ex/d> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g2> .\n",
            "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .\n",
        )
        .into(),
        named_graphs: vec![
            "http://ex/g2".into(),
            "http://ex/empty".into(),
            "http://ex/g1".into(),
        ],
        salt: [17; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11DatasetV2,
            query: format!("PREFIX ex: <http://ex/> {query}"),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [29; 32],
        },
        dataset,
    }
}

fn rows(query: &str) -> Vec<Vec<Option<String>>> {
    let result = evaluate(&witness(query)).unwrap().result;
    let CanonicalResult::Select { order, rows, .. } = result else {
        panic!("SELECT")
    };
    assert_eq!(order, RowOrder::Bag);
    rows
}

fn iri(value: &str) -> Option<String> {
    Some(format!("<http://ex/{value}>"))
}

#[test]
fn default_and_named_graphs_remain_distinct_with_complete_empty_catalog() {
    assert_eq!(rows("SELECT ?s { ?s ex:p ?o }"), vec![vec![iri("default")]]);
    assert_eq!(
        rows("SELECT ?s { GRAPH ex:g2 { ?s ex:p ?o } }"),
        vec![vec![iri("a")], vec![iri("c")]]
    );
    assert_eq!(
        rows("SELECT ?g { GRAPH ?g {} }"),
        vec![vec![iri("empty")], vec![iri("g1")], vec![iri("g2")]]
    );
    assert_eq!(
        evaluate(&witness("ASK { GRAPH ex:empty {} }"))
            .unwrap()
            .result,
        CanonicalResult::Ask(true)
    );
    assert_eq!(
        evaluate(&witness("ASK { GRAPH ex:absent {} }"))
            .unwrap()
            .result,
        CanonicalResult::Ask(false)
    );
    assert_eq!(
        rows("SELECT ?s { GRAPH ?g { ?s ex:p ?o } }"),
        vec![vec![iri("a")], vec![iri("a")], vec![iri("c")]]
    );
}

#[test]
fn dataset_clauses_merge_only_selected_snapshot_graphs() {
    assert_eq!(
        rows("SELECT ?s FROM ex:g1 FROM ex:g2 { ?s ex:p ?o }"),
        vec![vec![iri("a")], vec![iri("c")]]
    );
    assert!(rows("SELECT ?g FROM ex:g1 { GRAPH ?g {} }").is_empty());
    assert!(rows("SELECT ?s FROM NAMED ex:g1 { ?s ex:p ?o }").is_empty());
    assert_eq!(
        rows("SELECT ?g FROM NAMED ex:g1 FROM NAMED ex:empty { GRAPH ?g {} }"),
        vec![vec![iri("empty")], vec![iri("g1")]]
    );
    assert_eq!(
        rows("SELECT ?g FROM NAMED ex:g1 FROM NAMED ex:g1 { GRAPH ?g {} }"),
        vec![vec![iri("g1")]]
    );
    // Local snapshot resolution: no network dereference. The engine constructs
    // an empty active named graph for an explicitly selected absent source IRI.
    assert_eq!(
        rows("SELECT ?g FROM NAMED ex:absent { GRAPH ?g {} }"),
        vec![vec![iri("absent")]]
    );
}

#[test]
fn nested_graph_uses_the_same_dataset_catalog_and_preserves_bindings() {
    assert_eq!(
        rows("SELECT ?s { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }"),
        vec![vec![iri("a")], vec![iri("c")]]
    );
    assert_eq!(
        rows("SELECT ?g { GRAPH ?g { GRAPH ?g {} } }"),
        vec![vec![iri("empty")], vec![iri("g1")], vec![iri("g2")]]
    );
    let cartesian = rows("SELECT ?g ?h { GRAPH ?g { GRAPH ?h {} } }");
    assert_eq!(cartesian.len(), 9);
    assert_eq!(
        cartesian.first().unwrap(),
        &vec![iri("empty"), iri("empty")]
    );
    assert_eq!(cartesian.last().unwrap(), &vec![iri("g2"), iri("g2")]);
    assert_eq!(
        rows("SELECT ?g ?h FROM NAMED ex:g1 FROM NAMED ex:empty { GRAPH ?g { GRAPH ?h {} } }"),
        vec![
            vec![iri("empty"), iri("empty")],
            vec![iri("empty"), iri("g1")],
            vec![iri("g1"), iri("empty")],
            vec![iri("g1"), iri("g1")],
        ]
    );
    assert!(
        rows("SELECT ?s FROM NAMED ex:g1 { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }")
            .is_empty()
    );
}

#[test]
fn both_authorities_enforce_catalog_integrity_and_never_strengthen_provenance() {
    let original = witness("SELECT ?g { GRAPH ?g {} }");
    let expected = evaluate(&original).unwrap();
    assert_eq!(expected.provenance, Provenance::VerifierAcceptedCommitment);
    bind_journal(&expected, &original.request).unwrap();
    let mut reordered = original.clone();
    reordered.dataset.named_graphs.reverse();
    assert_eq!(evaluate(&reordered).unwrap(), expected);
    let mut omitted_empty = original.clone();
    omitted_empty
        .dataset
        .named_graphs
        .retain(|name| name != "http://ex/empty");
    assert!(evaluate(&omitted_empty).is_err());
    omitted_empty.request.authority = DatasetAuthority::HolderDeclared;
    let declared = evaluate(&omitted_empty).unwrap();
    assert_eq!(declared.provenance, Provenance::HolderDeclaredOnly);
    assert!(bind_journal(&declared, &original.request).is_err());
    // Even holder-declared data must be a complete, well-formed declared catalog.
    omitted_empty
        .dataset
        .named_graphs
        .retain(|name| name != "http://ex/g2");
    assert!(evaluate(&omitted_empty).is_err());
    let mut source_tamper = original;
    source_tamper.dataset.nquads = source_tamper
        .dataset
        .nquads
        .replace("http://ex/g2", "http://ex/g1");
    assert!(evaluate(&source_tamper).is_err());
}

#[test]
fn source_terms_catalog_names_and_total_quad_capacity_are_checked() {
    for source in [
        "_:a <http://ex/p> <http://ex/b> .",
        "<http://ex/a> <http://ex/p> _:b .",
        "<http://ex/a> <http://ex/p> <http://ex/b> _:g .",
        "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/unknown> .",
        "not N-Quads",
    ] {
        let mut w = witness("ASK {}");
        w.request.authority = DatasetAuthority::HolderDeclared;
        w.dataset.nquads = source.into();
        assert!(evaluate(&w).is_err(), "{source}");
    }
    let mut w = witness("ASK {}");
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.dataset.named_graphs.push("relative-graph-name".into());
    assert!(evaluate(&w).is_err());
    w.dataset.named_graphs.pop();
    w.request.policy.max_triples = 3;
    assert!(
        evaluate(&w).is_err(),
        "source capacity counts all graphs before deduplication"
    );
}

#[test]
fn named_graph_proof_fixture_has_independent_count_expectations() {
    let input = witness(
        "SELECT ?g (COUNT(?s) AS ?n) FROM NAMED ex:g1 FROM NAMED ex:g2 FROM NAMED ex:empty WHERE { GRAPH ?g { OPTIONAL { ?s ex:p ?o } GRAPH ex:empty {} } } GROUP BY ?g ORDER BY ?g",
    );
    let CanonicalResult::Select { order, rows, .. } = evaluate(&input).unwrap().result else {
        panic!("SELECT")
    };
    assert_eq!(order, RowOrder::Sequence);
    let count = |n| {
        Some(format!(
            "\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        ))
    };
    assert_eq!(
        rows,
        vec![
            vec![iri("empty"), count(0)],
            vec![iri("g1"), count(1)],
            vec![iri("g2"), count(2)]
        ]
    );
    assert_eq!(
        evaluate(&witness(
            "ASK FROM NAMED ex:empty { { GRAPH ex:empty { GRAPH ex:g2 { ?s ex:p ?o } } } UNION { BIND((\"1200\"^^<http://www.w3.org/2001/XMLSchema#byte> + 0) > 5 AS ?invalid) FILTER(?invalid) } UNION { FILTER(\"2024-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> = \"2024-01-01T00:00:00.000000001Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>) } }"
        ))
        .unwrap()
        .result,
        CanonicalResult::Ask(false)
    );
}

#[test]
fn v2_admission_enforces_reference_capacity_and_remaining_query_exclusions() {
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/v2-admission-rejections.json")).unwrap();
    let cases = source["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5, "all V2 profile exclusions are required");
    for case in cases {
        let input = witness(case["query"].as_str().unwrap());
        assert!(admit(&input.request).is_err(), "{}: admission", case["id"]);
        assert!(evaluate(&input).is_err(), "{}: evaluation", case["id"]);
    }
    let query = format!(
        "SELECT ?g {} WHERE {{ GRAPH ?g {{}} }}",
        (0..64)
            .map(|i| format!("FROM NAMED <http://ex/selected-{i}>"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let input = witness(&query);
    admit(&input.request).unwrap();
    // The query-derived reference cap is separate from the committed input
    // catalog cap. These absent sources become empty graphs under this profile.
    let CanonicalResult::Select { rows, .. } = evaluate(&input).unwrap().result else {
        panic!("SELECT")
    };
    assert_eq!(rows.len(), 64);
}

// [GPT-6] Differential relation tests; these are not cryptographic proof tests.
#![cfg(feature = "evaluate")]

use sparq_proved_evaluator_model::*;

fn witness(query: &str) -> Witness {
    let dataset = PrivateDataset {
        ntriples: include_str!("../../fixtures/default.nt").into(),
        salt: [17; 32],
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
            nonce: [29; 32],
        },
        dataset,
    }
}

fn select(query: &str) -> (RowOrder, Vec<Vec<Option<String>>>) {
    let CanonicalResult::Select { order, rows, .. } = evaluate(&witness(query)).unwrap().result
    else {
        panic!("SELECT");
    };
    (order, rows)
}

fn iri(s: &str) -> Option<String> {
    Some(format!("<http://ex/{s}>"))
}
fn int(n: u32) -> Option<String> {
    Some(format!(
        "\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>"
    ))
}

#[test]
fn combined_optional_minus_count_order_limit_has_exact_bindings() {
    let (order, rows) = select(include_str!("../../fixtures/combined.rq"));
    assert_eq!(order, RowOrder::Sequence);
    assert_eq!(
        rows,
        vec![
            vec![iri("alice"), int(2), Some("\"Alice\"".into())],
            vec![iri("bob"), int(1), None]
        ]
    );
}

#[test]
fn canonical_bag_preserves_duplicate_derivations() {
    let (order, rows) = select("SELECT ?s WHERE { ?s <http://ex/value> ?v }");
    assert_eq!(order, RowOrder::Bag);
    assert_eq!(
        rows,
        vec![
            vec![iri("alice")],
            vec![iri("alice")],
            vec![iri("bob")],
            vec![iri("carol")]
        ]
    );
}

#[test]
fn distinct_removes_duplicates_only_when_requested() {
    let (_, rows) = select("SELECT DISTINCT ?s WHERE { ?s <http://ex/value> ?v }");
    assert_eq!(rows.len(), 3);
}

#[test]
fn optional_unbound_is_not_empty_literal() {
    let (_, rows) = select(
        "SELECT ?s ?name WHERE { ?s <http://ex/score> ?n OPTIONAL { ?s <http://ex/name> ?name } }",
    );
    assert_eq!(
        rows,
        vec![
            vec![iri("alice"), Some("\"Alice\"".into())],
            vec![iri("bob"), None],
            vec![iri("carol"), None]
        ]
    );
}

#[test]
fn negation_and_false_ask_evaluate_the_complete_graph() {
    let (_, rows) = select(
        "SELECT ?s WHERE { ?s <http://ex/score> ?n FILTER NOT EXISTS { ?s <http://ex/blocked> true } }",
    );
    assert_eq!(rows, vec![vec![iri("alice")], vec![iri("bob")]]);
    assert_eq!(
        evaluate(&witness("ASK { ?s <http://ex/score> ?n FILTER(?n > 100) }"))
            .unwrap()
            .result,
        CanonicalResult::Ask(false)
    );
    assert_eq!(
        evaluate(&witness("ASK { ?s <http://ex/score> ?n FILTER(?n > 50) }"))
            .unwrap()
            .result,
        CanonicalResult::Ask(true)
    );
}

#[test]
fn values_union_errors_and_numeric_expressions_remain_bound() {
    let (_, rows) = select("SELECT ?x ?y WHERE { VALUES ?x { 1 1 UNDEF } BIND((?x + 2) AS ?y) }");
    assert_eq!(
        rows,
        vec![vec![None, None], vec![int(1), int(3)], vec![int(1), int(3)]]
    );
    let (_, rows) = select("SELECT ?x WHERE { { VALUES ?x { 1 } } UNION { VALUES ?x { 2 } } }");
    assert_eq!(rows, vec![vec![int(1)], vec![int(2)]]);
}

#[test]
fn scope_is_explicit_and_selected_support_never_routes_here() {
    let mut w = witness("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }");
    w.request.contract = ProofContract::SelectedSupport;
    assert!(evaluate(&w).is_err());
    w.request.contract = ProofContract::ExactDataset;
    w.request.authority = DatasetAuthority::HolderDeclared;
    let j = evaluate(&w).unwrap();
    assert_eq!(j.provenance, Provenance::HolderDeclaredOnly);
    bind_journal(&j, &w.request).unwrap();
}

#[test]
fn omitted_or_added_triples_cannot_satisfy_an_accepted_anchor() {
    let mut w = witness("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }");
    w.dataset.ntriples = w
        .dataset
        .ntriples
        .lines()
        .take(1)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        evaluate(&w).unwrap_err().0,
        "complete dataset anchor mismatch"
    );
}

#[test]
fn every_request_field_and_policy_is_bound() {
    let w = witness("ASK { ?s ?p ?o }");
    let journal = evaluate(&w).unwrap();
    let mut altered = w.request.clone();
    altered.nonce[0] ^= 1;
    assert!(bind_journal(&journal, &altered).is_err());
    altered = w.request.clone();
    altered.query.push(' ');
    assert!(bind_journal(&journal, &altered).is_err());
    altered = w.request.clone();
    altered.policy.max_triples -= 1;
    assert!(bind_journal(&journal, &altered).is_err());
    altered = w.request.clone();
    altered.authority = DatasetAuthority::HolderDeclared;
    assert!(bind_journal(&journal, &altered).is_err());
    let mut forged = journal;
    forged.provenance = Provenance::HolderDeclaredOnly;
    assert!(bind_journal(&forged, &w.request).is_err());
}

#[test]
fn all_nested_nondeterminism_and_external_dependencies_reject() {
    for query in [
        "SELECT (NOW() AS ?x) WHERE {}",
        "SELECT (RAND() AS ?x) WHERE {}",
        "SELECT (UUID() AS ?x) WHERE {}",
        "SELECT (STRUUID() AS ?x) WHERE {}",
        "SELECT (BNODE(\"x\") AS ?x) WHERE {}",
        "SELECT ?s WHERE { ?s ?p ?o OPTIONAL { BIND(NOW() AS ?n) } }",
        "SELECT ?s WHERE { ?s ?p ?o FILTER EXISTS { FILTER(RAND() > 0) } }",
        "SELECT ?s WHERE { { SELECT (STRUUID() AS ?s) WHERE {} } }",
        "SELECT (COUNT(RAND()) AS ?n) WHERE { ?s ?p ?o }",
        "SELECT ?s WHERE { SERVICE <https://example.org> { ?s ?p ?o } }",
        "SELECT ?s WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }",
        "SELECT ?s FROM <http://ex/g> WHERE { ?s ?p ?o }",
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        "DESCRIBE <http://ex/alice>",
        "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
        "SELECT (<http://ex/custom>(1) AS ?x) WHERE {}",
    ] {
        assert!(admit(&witness(query).request).is_err(), "admitted {query}");
    }
}

#[test]
fn private_debug_and_source_validation_do_not_leak() {
    let mut w = witness("ASK { ?s ?p ?o }");
    assert!(!format!("{:?}", w.dataset).contains("alice"));
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.dataset.ntriples = "_:hidden <http://ex/p> \"secret\" .".into();
    let err = evaluate(&w).unwrap_err();
    assert!(!err.to_string().contains("secret"));
    w.dataset.salt = [0; 32];
    assert!(evaluate(&w).is_err());
}

#[test]
fn source_count_is_bounded_before_graph_deduplication() {
    let mut w = witness("ASK { ?s ?p ?o }");
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.request.policy.max_triples = 1;
    w.dataset.ntriples = "<http://ex/a> <http://ex/p> <http://ex/b> .\n".repeat(2);
    assert_eq!(evaluate(&w).unwrap_err().0, "source triple capacity");
}

#[test]
fn byte_nesting_and_materialization_limits_fail_without_partial_results() {
    let mut w = witness("SELECT ?s WHERE { ?s <http://ex/score> ?v }");
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.request.policy.max_rows = 1;
    assert!(
        evaluate(&w).is_err(),
        "oversized results must not be truncated"
    );
    w.request.policy.max_rows = MAX_ROWS;
    w.request.policy.max_dataset_bytes = 1;
    assert!(evaluate(&w).is_err());
    w.request.policy.max_dataset_bytes = MAX_DATASET_BYTES;
    w.request.query = " ".repeat(MAX_QUERY_BYTES + 1);
    assert!(evaluate(&w).is_err());
    w.request.query = format!(
        "ASK {{ FILTER({}true{}) }}",
        "(".repeat(140),
        ")".repeat(140)
    );
    assert!(evaluate(&w).is_err());
}

#[test]
fn aggregate_empty_input_and_ordered_offset_are_exact() {
    let (_, rows) = select("SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/missing> ?o }");
    assert_eq!(rows, vec![vec![int(0)]]);
    let (order, rows) =
        select("SELECT ?s WHERE { ?s <http://ex/score> ?n } ORDER BY DESC(?n) OFFSET 1 LIMIT 1");
    assert_eq!(order, RowOrder::Sequence);
    assert_eq!(rows, vec![vec![iri("alice")]]);
}

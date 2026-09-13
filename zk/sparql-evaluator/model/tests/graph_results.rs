// [GPT-6] Native V3 relation tests; actual guest/receipt coverage is separate.
#![cfg(feature = "graph-results")]
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, Provenance, RowOrder, v2, v3};

fn witness(query: &str, source: &str, named: &[&str]) -> v3::Witness {
    v3::Witness {
        request: v3::Request {
            version: 3,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: format!("PREFIX ex:<http://ex/> {query}"),
            authority: DatasetAuthority::HolderDeclared,
            policy: v3::Policy::default(),
            nonce: [7; 32],
        },
        dataset: v3::PrivateDataset {
            nquads: source.into(),
            named_graphs: named.iter().map(|s| (*s).into()).collect(),
            salt: [17; 32],
        },
    }
}

fn result(query: &str, source: &str) -> v3::CanonicalResult {
    v3::evaluate(&witness(query, source, &[])).unwrap().result
}

fn graph_text(result: v3::CanonicalResult) -> String {
    let v3::CanonicalResult::Graph { ntriples } = result else {
        panic!("graph result")
    };
    ntriples
}

#[test]
fn whole_table_identity_survives_relabeling_bag_duplicates_and_unbound_cells() {
    let source = "_:private <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n_:private <http://ex/p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    let query = "SELECT ?s ?n ?u { ?s ex:p ?n VALUES ?repeat { 1 1 } }";
    let expected = result(query, source);
    assert_eq!(
        expected,
        result(query, &source.replace("private", "renamed"))
    );
    assert_eq!(
        expected,
        result(query, &source.lines().rev().collect::<Vec<_>>().join("\n"))
    );
    let v3::CanonicalResult::Select {
        variables,
        order,
        rows,
    } = &expected
    else {
        panic!("table")
    };
    assert_eq!(variables, &vec!["s", "n", "u"]);
    assert_eq!(*order, RowOrder::Bag);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[2], rows[3]);
    assert!(
        rows.iter()
            .all(|row| row[0] == rows[0][0] && row[2].is_none())
    );
    assert!(!format!("{expected:?}").contains("private"));
    let separate = "_:a <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n_:b <http://ex/p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    let separate = result(query, separate);
    assert_ne!(expected, separate, "per-row relabeling would lose identity");
    let v3::CanonicalResult::Select { rows, .. } = separate else {
        panic!("table")
    };
    assert_ne!(
        rows[0][0], rows[2][0],
        "different source nodes must remain different across rows"
    );
}

#[test]
fn sequence_indices_and_outer_bag_boundaries_are_canonicalized_globally() {
    let source = "_:a <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n_:b <http://ex/p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    let asc = "SELECT ?s ?n { ?s ex:p ?n } ORDER BY ?n";
    let desc = "SELECT ?s ?n { ?s ex:p ?n } ORDER BY DESC(?n)";
    let value = result(asc, source);
    assert_eq!(
        value,
        result(asc, &source.replace("_:a", "_:x").replace("_:b", "_:y"))
    );
    assert_ne!(value, result(desc, source));
    let v3::CanonicalResult::Select { order, .. } = value else {
        panic!("table")
    };
    assert_eq!(order, RowOrder::Sequence);
    assert_eq!(
        result(
            "SELECT ?s ?n { { SELECT ?s ?n { ?s ex:p ?n } ORDER BY DESC(?n) } }",
            source
        ),
        result("SELECT ?s ?n { ?s ex:p ?n }", source)
    );
}

#[test]
fn construct_freshness_graph_set_semantics_and_illegal_template_omission() {
    let source = "_:tc0_0_0 <http://ex/p> <http://ex/a> .";
    let text = graph_text(result(
        "CONSTRUCT { _:new ex:p ?s; ex:q ?s . ?s ex:r ex:a . ?missing ex:p ex:a . ?illegal ex:p ex:a } WHERE { ?s ex:p ex:a VALUES ?repeat { 1 1 } BIND(1 AS ?illegal) }",
        source,
    ));
    assert_eq!(
        text.lines().count(),
        5,
        "four fresh-node triples and one deduplicated source triple"
    );
    assert!(!text.contains("tc0_0_0"));
    assert_eq!(
        text,
        graph_text(result(
            "CONSTRUCT { _:new ex:p ?s; ex:q ?s . ?s ex:r ex:a . ?missing ex:p ex:a . ?illegal ex:p ex:a } WHERE { ?s ex:p ex:a VALUES ?repeat { 1 1 } BIND(1 AS ?illegal) }",
            &source.replace("tc0_0_0", "renamed")
        ))
    );
    assert_eq!(
        graph_text(result(
            "CONSTRUCT { ex:s ex:p ex:a } WHERE { VALUES ?v { 1 1 } }",
            ""
        ))
        .lines()
        .count(),
        1
    );
}

#[test]
fn describe_uses_only_outgoing_blank_closure_of_active_default_graph() {
    let source = "<http://ex/a> <http://ex/p> _:x .\n_:x <http://ex/p> _:y .\n_:y <http://ex/p> _:x .\n_:y <http://ex/q> <http://ex/b> .\n<http://ex/inbound> <http://ex/p> <http://ex/a> .\n<http://ex/b> <http://ex/q> <http://ex/outside> .";
    let text = graph_text(result("DESCRIBE ex:a", source));
    assert_eq!(text.lines().count(), 4);
    assert!(!text.contains("inbound") && !text.contains("outside"));
    assert_eq!(
        text,
        graph_text(result(
            "DESCRIBE ?s WHERE { VALUES ?s { ex:a ex:a } }",
            source
        ))
    );
}

#[test]
fn complete_named_dataset_identity_and_from_separation_are_distinct() {
    let source = "_:same <http://ex/p> <http://ex/a> <http://ex/g1> .\n_:same <http://ex/q> <http://ex/b> <http://ex/g2> .";
    let named = ["http://ex/g1", "http://ex/g2", "http://ex/empty"];
    let w = witness(
        "ASK { GRAPH ex:g1 { ?s ex:p ex:a } GRAPH ex:g2 { ?s ex:q ex:b } }",
        source,
        &named,
    );
    assert_eq!(
        v3::evaluate(&w).unwrap().result,
        v3::CanonicalResult::Ask(true)
    );
    let w = witness(
        "ASK FROM ex:g1 FROM ex:g2 { ?s ex:p ex:a; ex:q ex:b }",
        source,
        &named,
    );
    assert_eq!(
        v3::evaluate(&w).unwrap().result,
        v3::CanonicalResult::Ask(false)
    );
    let w = witness(
        "CONSTRUCT { ?s ?p ?o } FROM ex:g1 FROM ex:g2 WHERE { ?s ?p ?o }",
        source,
        &named,
    );
    assert_eq!(
        graph_text(v3::evaluate(&w).unwrap().result).lines().count(),
        2
    );
}

#[test]
fn both_authorities_bind_source_catalog_query_policy_and_nonce() {
    let mut w = witness(
        "SELECT ?s { ?s ex:p ex:a }",
        "_:x <http://ex/p> <http://ex/a> .",
        &["http://ex/empty"],
    );
    let holder = v3::evaluate(&w).unwrap();
    assert_eq!(holder.provenance, Provenance::HolderDeclaredOnly);
    v3::bind_journal(&holder, &w.request).unwrap();
    let commitment = v3::dataset_commitment(&w.dataset, &w.request.policy).unwrap();
    assert_ne!(
        commitment,
        v2::dataset_commitment(&w.dataset, &w.request.policy.dataset).unwrap()
    );
    w.request.authority = DatasetAuthority::VerifierAgreed { commitment };
    let agreed = v3::evaluate(&w).unwrap();
    v3::bind_journal(&agreed, &w.request).unwrap();
    assert_eq!(agreed.provenance, Provenance::VerifierAcceptedCommitment);
    assert!(v3::bind_journal(&holder, &w.request).is_err());
    for field in 0..5 {
        let mut request = w.request.clone();
        match field {
            0 => request.version = 2,
            1 => request.nonce[0] ^= 1,
            2 => request.query.push(' '),
            3 => request.policy.canonicalization.max_quads -= 1,
            _ => request.policy.dataset.max_rows -= 1,
        }
        assert!(v3::bind_journal(&agreed, &request).is_err());
    }
    w.dataset.named_graphs.clear();
    assert!(
        v3::evaluate(&w).is_err(),
        "empty graph omission changes the accepted anchor"
    );
}

#[test]
fn every_canonicalization_and_graph_output_capacity_fails_without_partial_result() {
    let base = witness(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        "_:a <http://ex/p> _:b .\n_:b <http://ex/p> _:a .",
        &[],
    );
    v3::evaluate(&base).unwrap();
    for capacity in 0..6 {
        let mut w = base.clone();
        let policy = &mut w.request.policy;
        match capacity {
            0 => policy.canonicalization.max_quads = 1,
            1 => policy.canonicalization.max_input_bytes = 1,
            2 => policy.canonicalization.max_output_bytes = 1,
            3 => policy.canonicalization.max_hndq_calls = 1,
            4 => policy.canonicalization.max_permutation_steps = 1,
            _ => policy.dataset.max_rows = 1,
        }
        assert!(v3::evaluate(&w).is_err(), "capacity {capacity}");
    }
    let mut ask = witness("ASK {}", "", &[]);
    ask.request.policy.canonicalization.max_output_bytes = 1;
    assert!(v3::evaluate(&ask).is_err());
}

#[test]
fn unsupported_forms_and_terms_do_not_enter_the_v3_relation() {
    for query in [
        "SELECT (BNODE() AS ?x) {}",
        "SELECT (RAND() AS ?x) {}",
        "SELECT * { SERVICE <http://ex/service> {} }",
        "SELECT * { FILTER EXISTS { GRAPH ex:g {} } }",
    ] {
        assert!(
            v3::admit(&witness(query, "", &[]).request).is_err(),
            "{query}"
        );
    }
    for source in [
        "_:s <http://ex/p> <http://ex/o> _:g .",
        "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/p> <http://ex/b> )>> .",
    ] {
        assert!(v3::evaluate(&witness("ASK {}", source, &[])).is_err());
    }
}

#[test]
fn dynamic_domain_failures_abort_every_read_form_instead_of_empty_results() {
    for expression in [
        "(\"9223372036854775808\"^^<http://www.w3.org/2001/XMLSchema#integer> + 0)",
        "STRDT(CONCAT(\"0000\",\"-01-01T00:00:00Z\"), <http://www.w3.org/2001/XMLSchema#dateTime>)",
    ] {
        for form in ["SELECT ?v", "ASK", "CONSTRUCT {}", "DESCRIBE ex:a"] {
            let query = format!("{form} WHERE {{ BIND({expression} AS ?v) }}");
            assert!(v3::evaluate(&witness(&query, "", &[])).is_err(), "{query}");
        }
    }
}

#[test]
fn graph_production_budget_rejects_after_a_single_successful_solution() {
    let query = "CONSTRUCT { _:a ex:p ex:a; ex:q ex:b } WHERE {}";
    let mut w = witness(query, "", &[]);
    assert_eq!(
        graph_text(v3::evaluate(&w).unwrap().result).lines().count(),
        2
    );
    w.request.policy.dataset.max_rows = 1;
    assert!(
        v3::evaluate(&w).is_err(),
        "template expansion exceeds the graph budget"
    );
}

#[test]
fn v2_retains_blank_node_and_graph_form_rejections() {
    let w = witness(
        "SELECT * { ?s ?p ?o }",
        "_:x <http://ex/p> <http://ex/o> .",
        &[],
    );
    let mut request = v2::Request {
        version: 2,
        contract: ProofContract::ExactDataset,
        dialect: v2::Dialect::SparqSparql11DatasetV2,
        query: w.request.query,
        authority: DatasetAuthority::HolderDeclared,
        policy: w.request.policy.dataset,
        nonce: [7; 32],
    };
    assert!(
        v2::evaluate(&v2::Witness {
            request: request.clone(),
            dataset: w.dataset
        })
        .is_err()
    );
    request.query = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }".into();
    assert!(v2::admit(&request).is_err());
}

#[test]
fn exists_requires_all_source_graphs_blank_free_but_allows_later_template_nodes() {
    let query = "CONSTRUCT { _:fresh ex:p ?s } WHERE { ?s ex:p ?o FILTER EXISTS { ?s ex:p ?x } }";
    let source = "<http://ex/a> <http://ex/p> <http://ex/b> .";
    let input = witness(query, source, &[]);
    v3::admit(&input.request).unwrap();
    assert_eq!(
        graph_text(v3::evaluate(&input).unwrap().result)
            .lines()
            .count(),
        1
    );
    for (source, named) in [
        ("_:a <http://ex/p> <http://ex/b> .", vec![]),
        ("<http://ex/a> <http://ex/p> _:b .", vec![]),
        (
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n_:unselected <http://ex/p> <http://ex/b> <http://ex/g> .",
            vec!["http://ex/g"],
        ),
    ] {
        let input = witness(query, source, &named);
        v3::admit(&input.request).unwrap();
        assert_eq!(
            v3::evaluate(&input).unwrap_err().0,
            "V3 EXISTS blank-node correlation is not admitted"
        );
    }
    let input = witness(
        "ASK { [] ex:p ?o FILTER NOT EXISTS { ex:missing ex:p ?x } }",
        source,
        &[],
    );
    assert_eq!(
        v3::evaluate(&input).unwrap().result,
        v3::CanonicalResult::Ask(true)
    );
}

#[test]
fn captured_bound_profile_is_retained_in_v3_with_blank_free_input() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/conformance/exists-bound-boundaries.json"
    ))
    .unwrap();
    let mut counts = (0, 0);
    for case in corpus["cases"].as_array().unwrap() {
        let mut input = witness(
            "ASK {}",
            case["dataset_ntriples"].as_str().unwrap_or(""),
            &[],
        );
        input.request.query = case["query"].as_str().unwrap().into();
        if case["admitted"] == true {
            let v3::CanonicalResult::Select { rows, .. } = v3::evaluate(&input).unwrap().result
            else {
                panic!("SELECT")
            };
            assert_eq!(
                serde_json::json!(rows),
                case["expected_rows"],
                "{}",
                case["id"]
            );
            counts.0 += 1;
        } else {
            assert_eq!(
                v3::evaluate(&input).unwrap_err().0,
                "captured BOUND is outside the SPARQL 1.1 substitution profile",
                "{}",
                case["id"]
            );
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (8, 12));
}

#[test]
fn existing_positive_goldens_run_through_v3_without_changing_expectations() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/cases.json")).unwrap();
    let mut count = 0;
    for case in corpus["cases"].as_array().unwrap() {
        if case["expected"]["kind"] != "result" {
            continue;
        }
        let source = case["dataset"]
            .as_str()
            .unwrap_or(include_str!("../../fixtures/conformance/core.nt"));
        let mut w = witness("ASK {}", source, &[]);
        w.request.query = case["query"].as_str().unwrap().into();
        w.request.authority = DatasetAuthority::VerifierAgreed {
            commitment: v3::dataset_commitment(&w.dataset, &w.request.policy).unwrap(),
        };
        let actual = v3::evaluate(&w).unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
        let mut expected = case["expected"]["result"].clone();
        if expected["Select"]["order"] == "Bag" {
            expected["Select"]["rows"]
                .as_array_mut()
                .unwrap()
                .sort_by_key(|row| row.to_string());
        }
        let mut actual = serde_json::to_value(actual.result).unwrap();
        if actual["Select"]["order"] == "Bag" {
            actual["Select"]["rows"]
                .as_array_mut()
                .unwrap()
                .sort_by_key(|row| row.to_string());
        }
        assert_eq!(actual, expected, "{}", case["id"]);
        count += 1;
    }
    assert_eq!(count, 82);
}

#[test]
fn version_challenge_and_canonicalization_policy_bounds_fail_before_evaluation() {
    let original = witness("ASK {}", "", &[]);
    for field in 0..5 {
        for excessive in [false, true] {
            let mut w = original.clone();
            let policy = &mut w.request.policy.canonicalization;
            let value = match field {
                0 => &mut policy.max_quads,
                1 => &mut policy.max_input_bytes,
                2 => &mut policy.max_output_bytes,
                3 => &mut policy.max_hndq_calls,
                _ => &mut policy.max_permutation_steps,
            };
            *value = if excessive { *value + 1 } else { 0 };
            assert!(v3::validate_request(&w.request).is_err());
            assert!(v3::dataset_commitment(&w.dataset, &w.request.policy).is_err());
        }
    }
    for change in 0..4 {
        let mut request = original.request.clone();
        match change {
            0 => request.version = 2,
            1 => request.nonce = [0; 32],
            2 => request.contract = ProofContract::SelectedSupport,
            _ => request.query = " ".repeat(sparq_proved_evaluator_model::MAX_QUERY_BYTES + 1),
        }
        assert!(v3::validate_request(&request).is_err());
    }
}

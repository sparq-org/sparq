mod closed_native;
mod support;
#[path = "support/fixture.rs"]
mod fixtures;
use closed_native::Workspace;
use fixtures::FixtureEncoder;
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    codegen::generate_operator_fixture as generate,
    interp::{Cell, ResultValue},
    lower::lower,
    plan::Operator,
    plan_closed::ClosedOperator,
    witness::prover_toml,
};
fn evaluate(
    plan: &sparq_zk_query::plan::Plan,
    data: &[sparq_zk_query::interp::CredentialCells],
    encoder: &impl sparq_zk_query::interp::EncodeTerm,
) -> Result<sparq_zk_query::interp::Trace, sparq_zk_query::Error> {
    sparq_zk_query::credential_graphs::evaluate_cells_for_request(plan, data, encoder, [7; 32])
}
const B: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};
const QUERIES: &[&str] = &[
    "SELECT ?s ?y WHERE {?s <urn:p> ?x OPTIONAL {?s <urn:p> ?y FILTER(?y > ?x)}}",
    "SELECT ?s WHERE {?s <urn:p> ?x MINUS {VALUES ?x {1}}}",
    "SELECT ?s WHERE {?s <urn:p> ?x MINUS {VALUES ?z {1}}}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER EXISTS {FILTER(?x = 1)}}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER NOT EXISTS {?s <urn:p> ?y FILTER(?y > 1)}}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER EXISTS {?s <urn:p> ?m MINUS {?s <urn:p> ?z}}}",
    "SELECT ?s WHERE {{SELECT DISTINCT ?s WHERE {?s <urn:p> ?x}} ?s <urn:p> ?y}",
    "SELECT ?x ?y WHERE {{VALUES ?x {1 UNDEF}} UNION {VALUES ?x {2}} BIND(?x AS ?y)}",
    "SELECT ?x WHERE {VALUES ?x {1 2} FILTER(EXISTS {FILTER(?x = 1)} || ?x = 2)}",
    "SELECT ?s WHERE {?s <urn:p> ?x FILTER EXISTS {FILTER NOT EXISTS {?s <urn:q> ?y}}}",
];
#[test]
fn closed_shapes_are_canonical_and_inputs_topological() {
    for query in QUERIES {
        let p = lower(query, B).unwrap();
        for (i, n) in p.nodes().iter().enumerate() {
            for id in n.operator.inputs() {
                assert!((id as usize) < i);
            }
        }
        assert_eq!(generate(&p).unwrap(), generate(&p).unwrap());
        assert_eq!(p.hash(), lower(query, B).unwrap().hash());
    }
    let a = lower("SELECT ?x WHERE {{VALUES ?x {1}} UNION {VALUES ?x {2}}}", B).unwrap();
    let b = lower("SELECT ?y WHERE {{VALUES ?y {8}} UNION {VALUES ?y {9}}}", B).unwrap();
    let c = lower("SELECT ?x WHERE {VALUES ?x {1 2}}", B).unwrap();
    assert_eq!(a.hash(), b.hash());
    assert_ne!(a.hash(), c.hash());
}
#[test]
fn seeded_closed_interpreter_agrees_with_engine() {
    for seed in [0, 1, 7, 41, 893, 1201] {
        let (rdf, data, _) = fixtures::generated_dataset(seed);
        let bounds = Bounds {
            credentials: 1,
            quads_per_credential: 16,
            rows_per_table: 8,
        };
        for query in QUERIES {
            let plan = lower(query, bounds).unwrap();
            let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
            fixtures::assert_engine(query, &rdf, &trace.result);
        }
    }
}
#[test]
fn optional_errors_preserve_left_and_minus_observes_dynamic_domains() {
    let query =
        "SELECT ?x ?y WHERE {VALUES ?x {1 2} OPTIONAL {VALUES ?y {3} FILTER(?missing > ?x)}}";
    let p = lower(query, B).unwrap();
    let trace = evaluate(&p, &[], &FixtureEncoder).unwrap();
    if let ResultValue::Select { rows, .. } = trace.result {
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !r[1].bound));
    } else {
        panic!("SELECT");
    }
    let p = lower(
        "SELECT ?x WHERE {VALUES ?x {1 UNDEF} MINUS {VALUES ?x {UNDEF}}}",
        B,
    )
    .unwrap();
    let trace = evaluate(&p, &[], &FixtureEncoder).unwrap();
    if let ResultValue::Select { rows, .. } = trace.result {
        assert_eq!(rows.len(), 2);
    } else {
        panic!("SELECT");
    }
}
#[test]
fn subqueries_project_scope_distinct_and_canonical_slice() {
    let p=lower("SELECT ?x ?y WHERE {{SELECT DISTINCT ?x WHERE {VALUES (?x ?y) {(1 7) (1 8) (2 9)}} LIMIT 1} VALUES ?y {3}}",B).unwrap();
    let trace = evaluate(&p, &[], &FixtureEncoder).unwrap();
    if let ResultValue::Select { rows, .. } = trace.result {
        assert_eq!(rows.len(), 1);
        assert!(rows[0].iter().all(|c| c.bound));
    } else {
        panic!("SELECT");
    }
    assert!(lower(
        "SELECT * WHERE {{SELECT * WHERE {?s ?p ?o} ORDER BY ?s}}",
        B
    )
    .unwrap_err()
    .to_string()
    .contains("ORDER BY"));
    assert!(lower("SELECT ?x WHERE {BIND(ABS(1) AS ?x)}", B)
        .unwrap_err()
        .to_string()
        .contains("ABS"));
}
#[test]
fn closed_capacity_never_truncates_before_limit() {
    let p = lower(
        "SELECT ?x WHERE {{VALUES ?x {1 2 3}} UNION {VALUES ?x {4 5 6}}} LIMIT 1",
        B,
    )
    .unwrap();
    assert!(matches!(
        evaluate(&p, &[], &FixtureEncoder),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
}
#[test]
fn generated_closed_circuits_execute_and_reject_altered_tables_and_results() {
    let Some(gate) = support::gate(
        "generated_closed_circuits_execute_and_reject_altered_tables_and_results",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let data = fixtures::fixture();
    for query in QUERIES {
        let plan = lower(query, B).unwrap();
        let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
        let package = generate(&plan).unwrap();
        let workspace = Workspace::new(&package);
        let driver = ProgramProver::new(workspace.0.path(), &package.name).unwrap();
        let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
        driver
            .execute(&input, "closed_honest")
            .unwrap_or_else(|e| panic!("{query}: {e}"));
        for (i, n) in plan.nodes().iter().enumerate() {
            if !matches!(n.operator, Operator::Closed(_)) {
                continue;
            }
            let mut changed = trace.clone();
            if let Some(row) = changed.tables[i].rows.first_mut() {
                if let Some(c) = row.first_mut() {
                    c.enc[31] ^= 1;
                } else {
                    changed.tables[i].rows.clear();
                }
            } else {
                changed.tables[i]
                    .rows
                    .push(vec![Cell::default(); n.output.variables.len()]);
            }
            let changed =
                prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&changed, "closed_altered").is_err(),
                "{query}: node {i}"
            );
        }
        let mut changed = trace.clone();
        if let ResultValue::Select { rows, .. } = &mut changed.result {
            if !rows.is_empty() {
                rows.pop();
            } else {
                rows.push(vec![Cell::default(); plan.result_schema().variables.len()]);
            }
        }
        let changed =
            prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
        assert!(
            driver.execute(&changed, "closed_result").is_err(),
            "{query}"
        );
    }
    support::complete(gate);
}
#[test]
fn credential_graphs_are_separate_and_default_merge_deduplicates() {
    let Some(gate) = support::gate(
        "credential_graphs_are_separate_and_default_merge_deduplicates",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let mut data = fixtures::fixture();
    data.push(data[0].clone());
    let queries = [
        ("SELECT ?g ?s WHERE {GRAPH ?g {?s <urn:p> ?x}}", 4),
        (
            "SELECT ?s WHERE {GRAPH <urn:sparq:credential:0> {?s <urn:p> ?x}}",
            0,
        ),
        ("SELECT ?s WHERE {GRAPH <urn:absent> {?s <urn:p> ?x}}", 0),
        ("SELECT ?s WHERE {?s <urn:p> ?x}", 2),
    ];
    for (q, count) in queries {
        let plan = lower(q, B).unwrap();
        let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
        if let ResultValue::Select { rows, .. } = &trace.result {
            assert_eq!(rows.len(), count);
        } else {
            panic!("SELECT");
        }
        if plan.nodes().iter().any(|n| {
            matches!(
                n.operator,
                Operator::Closed(ClosedOperator::Graph { .. } | ClosedOperator::GraphScan { .. })
            )
        }) {
            let package = generate(&plan).unwrap();
            let workspace = Workspace::new(&package);
            let driver = ProgramProver::new(workspace.0.path(), &package.name).unwrap();
            let input =
                prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
            driver
                .execute(&input, "graph_honest")
                .unwrap_or_else(|e| panic!("{q}: {e}"));
        }
    }
    support::complete(gate);
}

#[test]
fn closed_corpus_admission_inventory() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("../../zk/sparql-evaluator/fixtures/conformance/cases.json")).unwrap(),
    )
    .unwrap();
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 16,
        rows_per_table: 16,
    };
    let entries = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let result = lower(case["query"].as_str().unwrap(), bounds);
            match result {
                Ok(_) => serde_json::json!({"id":case["id"],"status":"admitted"}),
                Err(e) => {
                    serde_json::json!({"id":case["id"],"status":"refused","refusal":e.to_string()})
                }
            }
        })
        .collect::<Vec<_>>();
    std::fs::write(
        "/tmp/lane-c1-admissions.json",
        serde_json::to_vec_pretty(&entries).unwrap(),
    )
    .unwrap();
}

#[test]
fn seeded_generated_closed_circuits_agree_and_reject_hidden_right_rows() {
    let Some(gate) = support::gate(
        "seeded_generated_closed_circuits_agree_and_reject_hidden_right_rows",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let queries=[
        "SELECT ?s ?y WHERE {?s <urn:p> ?x OPTIONAL {?s <urn:q> ?y} FILTER NOT EXISTS {?s <urn:p> ?z FILTER(?z < 0)}}",
        "SELECT ?s ?y WHERE {{SELECT DISTINCT ?s WHERE {?s <urn:p> ?x}} ?s <urn:q> ?v BIND(?v AS ?y) MINUS {?s <urn:p> ?z FILTER(?z < 0)}}",
    ];
    for seed in [7, 41] {
        let (rdf, data, _) = fixtures::generated_dataset(seed);
        let bounds = Bounds {
            credentials: 1,
            quads_per_credential: 16,
            rows_per_table: 8,
        };
        for query in queries {
            let plan = lower(query, bounds).unwrap();
            let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
            fixtures::assert_engine(query, &rdf, &trace.result);
            let package = generate(&plan).unwrap();
            let workspace = Workspace::new(&package);
            let driver = ProgramProver::new(workspace.0.path(), &package.name).unwrap();
            let input =
                prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
            driver
                .execute(&input, "seeded_closed")
                .unwrap_or_else(|e| panic!("seed={seed}, {query}: {e}"));
            let mut changed = trace.clone();
            let right = plan
                .nodes()
                .iter()
                .find_map(|node| match node.operator {
                    Operator::Closed(ClosedOperator::LeftJoin { right, .. })
                    | Operator::Closed(ClosedOperator::Minus { right, .. }) => Some(right as usize),
                    _ => None,
                })
                .expect("mixed query has a right input");
            assert!(!changed.tables[right].rows.is_empty());
            changed.tables[right].rows.pop();
            let changed =
                prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&changed, "hidden_input_row").is_err(),
                "seed={seed}, {query}"
            );
            let mut changed = trace.clone();
            if let ResultValue::Select { rows, .. } = &mut changed.result {
                if rows.is_empty() {
                    rows.push(vec![Cell::default(); plan.result_schema().variables.len()]);
                } else {
                    rows.pop();
                }
            }
            let changed =
                prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&changed, "seeded_altered_result").is_err(),
                "seed={seed}, {query}"
            );
        }
    }
    support::complete(gate);
}

#[test]
fn closed_positive_corpus_executes_with_expected_engine_result_and_controls() {
    let Some(gate) = support::gate(
        "closed_positive_corpus_executes_with_expected_engine_result_and_controls",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("../../zk/sparql-evaluator/fixtures/conformance/cases.json")).unwrap(),
    )
    .unwrap();
    let rdf =
        std::fs::read_to_string(root.join("../../zk/sparql-evaluator/fixtures/conformance/core.nt"))
            .unwrap();
    let admitted = [
        "union-keeps-duplicates",
        "optional-unbound-versus-empty",
        "minus-removes-compatible",
        "minus-disjoint-domains-keeps-left",
        "not-exists-correlated",
        "not-exists-fixed-pattern-removes-all",
        "exists-filter-only-outer-variable",
        "exists-minus-disjoint-domain-after-substitution",
        "subquery-project-scope",
    ];
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 16,
        rows_per_table: 4,
    };
    let mut executed = vec![];
    for id in admitted {
        let case = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap();
        let source = case["dataset"].as_str().unwrap_or(&rdf);
        let (data, _) = fixtures::dataset(source);
        let query = case["query"].as_str().unwrap();
        let plan = lower(query, bounds).unwrap();
        let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
        fixtures::assert_engine(query, source, &trace.result);
        if let ResultValue::Select { rows, variables } = &trace.result {
            let expected = &case["expected"]["result"]["Select"];
            assert_eq!(
                variables,
                &expected["variables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            );
            let mut expected_rows = vec![];
            for row in expected["rows"].as_array().unwrap() {
                let mut result = vec![];
                for term in row.as_array().unwrap() {
                    if let Some(term) = term.as_str() {
                        let (_, terms) = fixtures::dataset(&format!("<urn:s> <urn:p> {term} ."));
                        use sparq_zk_query::interp::EncodeTerm;
                        result.push(Some(FixtureEncoder.encode(&terms[2]).unwrap().enc));
                    } else {
                        result.push(None);
                    }
                }
                expected_rows.push(result);
            }
            let mut actual = rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.bound.then_some(c.enc))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            actual.sort();
            expected_rows.sort();
            assert_eq!(actual, expected_rows, "{id}");
        }
        let package = generate(&plan).unwrap();
        let workspace = Workspace::new(&package);
        let driver = ProgramProver::new(workspace.0.path(), &package.name).unwrap();
        let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
        driver
            .execute(&input, "corpus_closed")
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        for i in 0..trace.tables.len() {
            let mut changed = trace.clone();
            if changed.tables[i].rows.is_empty() {
                changed.tables[i].rows.push(vec![
                    Cell::default();
                    plan.nodes()[i].output.variables.len()
                ]);
            } else {
                changed.tables[i].rows.pop();
            }
            let changed =
                prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&changed, "corpus_closed_changed").is_err(),
                "{id}: {i}"
            );
        }
        let mut changed = trace.clone();
        if let ResultValue::Select { rows, .. } = &mut changed.result {
            if rows.is_empty() {
                rows.push(vec![Cell::default(); plan.result_schema().variables.len()]);
            } else {
                rows.pop();
            }
        }
        let changed =
            prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
        assert!(
            driver.execute(&changed, "corpus_closed_result").is_err(),
            "{id}"
        );
        assert!(
            driver
                .execute(
                    &input.replace("request_digest = [7,", "request_digest = [8,"),
                    "corpus_closed_public"
                )
                .is_err(),
            "{id}"
        );
        executed.push(id);
    }
    std::fs::write(
        "/tmp/lane-c1-corpus-executed.json",
        serde_json::to_vec_pretty(&executed).unwrap(),
    )
    .unwrap();
    support::complete(gate);
}

#[test]
fn nested_select_distinct_slice_executes_with_the_core_slice_relation() {
    let Some(gate) = support::gate(
        "nested_select_distinct_slice_executes_with_the_core_slice_relation",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let data = fixtures::fixture();
    let query = "SELECT ?s WHERE {{SELECT DISTINCT ?s WHERE {?s <urn:p> ?x} LIMIT 1 OFFSET 1}}";
    let plan = lower(query, B).unwrap();
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let flat = lower(
        "SELECT DISTINCT ?s WHERE {?s <urn:p> ?x} LIMIT 1 OFFSET 1",
        B,
    )
    .unwrap();
    assert_eq!(
        trace.result,
        evaluate(&flat, &data, &FixtureEncoder).unwrap().result
    );
    let package = generate(&plan).unwrap();
    let workspace = Workspace::new(&package);
    let driver = ProgramProver::new(workspace.0.path(), &package.name).unwrap();
    let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
    driver.execute(&input, "nested_slice").unwrap();
    let mut changed = trace.clone();
    changed.tables[plan.result_node() as usize].rows.clear();
    let changed = prover_toml(&plan, &data, &FixtureEncoder, &changed, [7; 32], [3; 32]).unwrap();
    assert!(driver.execute(&changed, "nested_slice_missing").is_err());
    support::complete(gate);
}

#[test]
fn named_graph_engine_interpreter_and_circuit_differential() {
    let Some(gate) = support::gate(
        "named_graph_engine_interpreter_and_circuit_differential",
        &["nargo", "bb"],
    ) else {
        return;
    };
    use sparq_zk_query::interp::EncodeTerm;
    let mut data = fixtures::fixture();
    data.push(data[0].clone());
    let mut rdf = String::new();
    for graph in [None, Some(0), Some(1)] {
        for (s, n) in [("urn:a", 1), ("urn:b", 2)] {
            let suffix = graph.map(|g| format!(" _:g{g}")).unwrap_or_default();
            rdf.push_str(&format!(
                "<{s}> <urn:p> \"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>{suffix} .\n"
            ));
        }
    }
    let engine = sparq_core::Graph::load_dataset(&rdf, "nquads").unwrap();
    for query in [
        "SELECT ?g ?s ?y WHERE {GRAPH ?g {?s <urn:p> ?x OPTIONAL {?s <urn:p> ?y FILTER(?y > 1)} FILTER EXISTS {?s <urn:p> ?z}}}",
        "SELECT ?s WHERE {GRAPH <urn:sparq:credential:0> {?s <urn:p> ?x} MINUS {GRAPH <urn:sparq:credential:1> {?s <urn:p> ?y FILTER(?y > 1)}}}",
        "SELECT ?g ?v WHERE {GRAPH ?g {VALUES ?x {1} BIND(?x AS ?v)}}",
        "SELECT ?g WHERE {GRAPH ?g {{SELECT DISTINCT ?s WHERE {?s <urn:p> ?x}}}}",
    ] {
        let plan=lower(query,B).unwrap();let trace=evaluate(&plan,&data,&FixtureEncoder).unwrap();
        let expected=sparq_engine::query(&engine,query).unwrap();
        if let ResultValue::Select {variables,rows}=&trace.result {
            assert_eq!(variables,&expected.vars.iter().map(|v|v.as_str().to_owned()).collect::<Vec<_>>());
            let mut actual=rows.iter().map(|r|r.iter().map(|c|c.bound.then_some(c.enc)).collect::<Vec<_>>()).collect::<Vec<_>>();
            let mut expected=expected.rows.iter().map(|r|r.iter().map(|t|t.as_ref().map(|t|if let oxrdf::Term::BlankNode(n)=t {{let mut word=[0;32];let label=n.as_str().trim_start_matches('g').parse::<u32>().unwrap()+1;word[28..].copy_from_slice(&label.to_be_bytes());word}} else {FixtureEncoder.encode(t).unwrap().enc})).collect::<Vec<_>>()).collect::<Vec<_>>();
            actual.sort();expected.sort();assert_eq!(actual,expected,"{query}");
        }else{panic!("SELECT");}
        let package=generate(&plan).unwrap();let workspace=Workspace::new(&package);
        let driver=ProgramProver::new(workspace.0.path(),&package.name).unwrap();
        let input=prover_toml(&plan,&data,&FixtureEncoder,&trace,[7;32],[3;32]).unwrap();
        driver.execute(&input,"graph_differential").unwrap_or_else(|e|panic!("{query}: {e}"));
        let right=plan.nodes().iter().enumerate().find_map(|(i,n)|matches!(n.operator,Operator::Closed(ClosedOperator::Graph {..} | ClosedOperator::GraphScan {..})).then_some(i)).unwrap();
        let mut changed=trace.clone();
        if changed.tables[right].rows.pop().is_none() {changed.tables[right].rows.push(vec![Cell::default();plan.nodes()[right].output.variables.len()]);}
        let changed=prover_toml(&plan,&data,&FixtureEncoder,&changed,[7;32],[3;32]).unwrap();
        assert!(driver.execute(&changed,"graph_hidden_row").is_err(),"{query}");
    }
    support::complete(gate);
}
#[test]
fn check_issuer_registry() {
    use sparq_zk_query::{credential::Credential, credential_graphs as graphs};
    let key = sparq_zk::sig::SecretKey::from_seed(7);
    let credential = sparq_merkle_suite::poseidon::issue(
        "<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n",
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n",
        &key,
        sparq_merkle_suite::Fr::from(19u64),
    )
    .unwrap();
    let c = Credential {
        credential,
        issuer: key.public_key(),
        issuer_index: 0,
        issuer_siblings: vec![sparq_merkle_suite::Fr::from(0u64); 4],
    };
    let p = graphs::bind_issuer_registry(
        graphs::lower("ASK {}", B).unwrap(),
        &[(
            oxrdf::NamedNode::new_unchecked("urn:issuer"),
            sparq_zk::sig::SecretKey::from_seed(8).public_key(),
        )],
    )
    .unwrap();
    assert!(graphs::evaluate(&p, &[c])
        .unwrap_err()
        .to_string()
        .contains("verified issuer registry"));
}

#[test]
fn check_literal_types() {
    use sparq_zk_query::{credential::Credential, credential_graphs as graphs};
    let key = sparq_zk::sig::SecretKey::from_seed(7);
    let credential = sparq_merkle_suite::poseidon::issue(
        "<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n<urn:s> <urn:p> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n",
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n",
        &key,
        sparq_merkle_suite::Fr::from(19u64),
    ).unwrap();
    let c = Credential {
        credential,
        issuer: key.public_key(),
        issuer_index: 0,
        issuer_siblings: vec![sparq_merkle_suite::Fr::from(0u64); 4],
    };
    let p = graphs::bind_issuer_registry(
        graphs::lower("ASK {}", B).unwrap(),
        &[(
            oxrdf::NamedNode::new_unchecked("urn:issuer"),
            key.public_key(),
        )],
    )
    .unwrap();
    assert_eq!(
        graphs::evaluate(&p, std::slice::from_ref(&c))
            .unwrap()
            .result,
        ResultValue::Ask(true)
    );
    let p = graphs::with_literal_types(p, 1).unwrap();
    assert!(graphs::evaluate(&p, &[c])
        .unwrap_err()
        .to_string()
        .contains("closed credential literal profile"));
}

#[test]
fn graph_evaluation_refuses_a_lost_credential_boundary() {
    let p = lower("SELECT ?g {?s <urn:p> ?x GRAPH ?g {?s <urn:p> ?x}}", B).unwrap();
    let error = sparq_zk_query::interp::evaluate(&p, &fixtures::fixture(), &FixtureEncoder)
        .unwrap_err();
    assert!(error.to_string().contains("complete credential graph view"));
}

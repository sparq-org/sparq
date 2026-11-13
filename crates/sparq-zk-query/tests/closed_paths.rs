#[allow(dead_code)]
#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/mod.rs"]
mod fixtures;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

use fixtures::{dataset, FixtureEncoder, Workspace};
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    codegen::{generate, generate_operator_fixture},
    interp::{evaluate, Cell, ResultValue, Trace},
    lower::lower,
    plan::{Operator, Parameter},
    plan_closed::ClosedOperator,
    statement::{Layout, Statement},
    witness::prover_toml,
    Error,
};
const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 4,
    rows_per_table: 8,
};
const RDF: &str =
    "<urn:a> <urn:p> <urn:b> .\n<urn:b> <urn:p> <urn:c> .\n<urn:a> <urn:q> <urn:b> .\n";
const QUERIES: &[&str] = &[
    "SELECT ?x ?y WHERE {?x <urn:p>/<urn:p> ?y}",
    "SELECT ?x ?y WHERE {?x ^(<urn:p>|<urn:q>) ?y}",
    "SELECT ?x ?y WHERE {?x (<urn:p>|<urn:q>) ?y}",
    "SELECT ?x ?y WHERE {?x !<urn:q> ?y}",
    "SELECT ?x ?y WHERE {?x !(<urn:q>|^<urn:p>) ?y}",
    "SELECT ?x ?y WHERE {?x <urn:p>? ?y}",
    "SELECT ?x ?y WHERE {?x <urn:p>* ?y}",
    "SELECT ?x ?y WHERE {?x <urn:p>+ ?y}",
    "SELECT ?y WHERE {<urn:missing> (<urn:p>*|<urn:p>*) ?y}",
    "SELECT ?y WHERE {<urn:missing> <urn:p>*/<urn:p>* ?y}",
    "SELECT ?y WHERE {<urn:missing> (<urn:p>?)+ ?y}",
    "SELECT ?x WHERE {?x <urn:p>* ?x}",
    "ASK {<urn:missing> <urn:p>* <urn:missing>}",
    "ASK {<urn:missing> <urn:p>+ <urn:missing>}",
];
// Freeze the library for every honest/altered pair, including parallel runs.
fn path_workspace(package: &sparq_zk_query::codegen::Package) -> Workspace {
    fn copy(source: &std::path::Path, target: &std::path::Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                copy(&path, &target.join(entry.file_name()));
            } else {
                std::fs::copy(path, target.join(entry.file_name())).unwrap();
            }
        }
    }
    let workspace = Workspace::new(package);
    let source = workspace.0.join("zk").canonicalize().unwrap();
    std::fs::remove_file(workspace.0.join("zk")).unwrap();
    let target = workspace.0.join("zk");
    std::fs::create_dir(&target).unwrap();
    for entry in std::fs::read_dir(&source).unwrap().map(Result::unwrap) {
        if entry.file_name() != "noir-query" {
            std::os::unix::fs::symlink(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
    let core = target.join("noir-query/core");
    std::fs::create_dir_all(&core).unwrap();
    std::fs::copy(
        source.join("noir-query/core/Nargo.toml"),
        core.join("Nargo.toml"),
    )
    .unwrap();
    copy(&source.join("noir-query/core/src"), &core.join("src"));
    workspace
}
fn path_node(plan: &sparq_zk_query::plan::Plan) -> usize {
    plan.nodes()
        .iter()
        .position(|n| matches!(n.operator, Operator::Closed(ClosedOperator::Path(_))))
        .expect("path node")
}
#[test]
fn depth_is_structural_and_public_and_generation_is_wiring_only() {
    let p = lower("SELECT ?x ?y WHERE {?x <urn:p>* ?y}", B).unwrap();
    let one = p.clone().with_path_depth(1).unwrap();
    let two = p.clone().with_path_depth(2).unwrap();
    assert_ne!(one.hash(), two.hash());
    assert_ne!(one.canonical_bytes(), two.canonical_bytes());
    assert!(matches!(one.parameters().last(), Some(Parameter::Index(1))));
    assert!(matches!(two.parameters().last(), Some(Parameter::Index(2))));
    assert!(p.clone().with_path_depth(0).is_err());
    assert!(p.with_path_depth(9).is_err());
    let (data, _) = dataset(RDF);
    let result = evaluate(&two, &data, &FixtureEncoder).unwrap().result;
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: [3; 32],
        signature_mode: 0,
        result,
    };
    assert_ne!(
        Layout::new(&one)
            .reconstruct(&one, &statement, &FixtureEncoder)
            .unwrap(),
        Layout::new(&two)
            .reconstruct(&two, &statement, &FixtureEncoder)
            .unwrap()
    );
    let package = generate_operator_fixture(&two).unwrap();
    assert_eq!(package, generate_operator_fixture(&two).unwrap());
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    for forbidden in ["assert(", "for ", "if ", "urn:p"] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
    assert!(source.contains("check_depth::<2>(indices[0])"));
    let other = lower("SELECT ?a ?b WHERE {?a <urn:q>* ?b}", B)
        .unwrap()
        .with_path_depth(2)
        .unwrap();
    assert_eq!(two.hash(), other.hash());
}
#[test]
fn seeded_paths_agree_with_engine_including_bags_cycles_and_nullable_endpoints() {
    for seed in [1u64, 7, 41, 893] {
        let mut state = seed;
        let mut rdf = String::new();
        for i in 0..3 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = (state >> 32) % 3;
            rdf.push_str(&format!("<urn:n{i}> <urn:p> <urn:n{j}> .\n"));
        }
        let (data, _) = dataset(&rdf);
        for q in QUERIES {
            let bounds = Bounds {
                rows_per_table: 16,
                ..B
            };
            let plan = lower(q, bounds).unwrap().with_path_depth(3).unwrap();
            let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
            fixtures::assert_engine(q, &rdf, &trace.result);
        }
    }
    for q in QUERIES {
        let (data, _) = dataset(RDF);
        let p = lower(q, B).unwrap().with_path_depth(2).unwrap();
        let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
        fixtures::assert_engine(q, RDF, &trace.result);
    }
}
#[test]
fn depth_and_capacity_refuse_before_slice_and_absence_never_truncates() {
    let (data, _) = dataset(RDF);
    let p = lower("SELECT ?y WHERE {<urn:a> <urn:p>+ ?y} LIMIT 1", B)
        .unwrap()
        .with_path_depth(1)
        .unwrap();
    assert!(
        matches!(evaluate(&p,&data,&FixtureEncoder),Err(Error::Capacity(s)) if s.contains("depth"))
    );
    let p = p.with_path_depth(2).unwrap();
    assert!(evaluate(&p, &data, &FixtureEncoder).is_ok());
    let small = lower(
        "SELECT ?x ?y WHERE {?x <urn:p>* ?y} LIMIT 1",
        Bounds {
            rows_per_table: 2,
            ..B
        },
    )
    .unwrap();
    assert!(matches!(
        evaluate(&small, &data, &FixtureEncoder),
        Err(Error::Capacity(_))
    ));
    let (empty, _) = dataset("");
    for (query, expected) in [
        ("ASK {<urn:missing> <urn:p>* <urn:missing>}", true),
        ("ASK {<urn:missing> (<urn:p>?)+ <urn:missing>}", true),
        ("ASK {?s <urn:p>* ?o}", false),
    ] {
        let p = lower(query, B).unwrap();
        assert_eq!(
            evaluate(&p, &empty, &FixtureEncoder).unwrap().result,
            ResultValue::Ask(expected)
        );
    }
}

fn alter_result(trace: &mut Trace, width: usize) {
    match &mut trace.result {
        ResultValue::Ask(v) => *v = !*v,
        ResultValue::Select { rows, .. } => {
            if rows.is_empty() {
                rows.push(vec![Cell::default(); width]);
            } else {
                rows.pop();
            }
        }
    }
}
#[test]
fn generated_paths_execute_and_reject_tables_results_depth_and_hidden_edges() {
    let Some(gate) = support::gate(
        "closed_paths::generated_paths_execute_and_reject_tables_results_depth_and_hidden_edges",
        &["nargo"],
    ) else {
        return;
    };
    let (data, _) = dataset(RDF);
    for query in QUERIES {
        let plan = lower(query, B).unwrap().with_path_depth(2).unwrap();
        let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
        let package = generate_operator_fixture(&plan).unwrap();
        let workspace = path_workspace(&package);
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        let render =
            |t: &Trace| prover_toml(&plan, &data, &FixtureEncoder, t, [7; 32], [3; 32]).unwrap();
        let input = render(&trace);
        driver
            .execute(&input, "path_honest")
            .unwrap_or_else(|e| panic!("{query}: {e}"));
        let i = path_node(&plan);
        for mutation in 0..4 {
            let mut changed = trace.clone();
            let rows = &mut changed.tables[i].rows;
            match mutation {
                0 => {
                    if rows.is_empty() {
                        rows.push(vec![
                            Cell::default();
                            plan.nodes()[i].output.variables.len()
                        ]);
                    } else {
                        rows.pop();
                    }
                }
                1 => {
                    if let Some(j) = (1..rows.len()).find(|&j| rows[j] != rows[0]) {
                        rows.swap(0, j);
                    } else if let Some(row) = rows.first_mut() {
                        if let Some(c) = row.first_mut() {
                            c.enc[31] ^= 1;
                        } else {
                            rows.clear();
                        }
                    } else {
                        rows.push(vec![
                            Cell::default();
                            plan.nodes()[i].output.variables.len()
                        ]);
                    }
                }
                2 => alter_result(&mut changed, plan.result_schema().variables.len()),
                _ => {
                    if let Some(r) = rows.first_mut() {
                        if let Some(c) = r.first_mut() {
                            c.enc[31] ^= 1;
                        } else {
                            rows.clear();
                        }
                    } else {
                        rows.push(vec![
                            Cell::default();
                            plan.nodes()[i].output.variables.len()
                        ]);
                    }
                }
            }
            assert!(
                driver.execute(&render(&changed), "path_changed").is_err(),
                "{query}: {mutation}"
            );
        }
        assert!(
            driver
                .execute(
                    &input.replace("indices = [2]", "indices = [1]"),
                    "path_changed_depth"
                )
                .is_err(),
            "{query}"
        );
        // Remove an edge but keep the original complete-dataset commitment.
        let mut hidden = data.clone();
        hidden[0].quads.remove(1);
        let mut changed =
            prover_toml(&plan, &hidden, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
        for name in ["input_commitment = ", "statement_binding = "] {
            let original = input.lines().find(|l| l.starts_with(name)).unwrap();
            let forged = changed
                .lines()
                .find(|l| l.starts_with(name))
                .unwrap()
                .to_owned();
            changed = changed.replace(&forged, original);
        }
        assert!(
            driver
                .execute(&changed, "path_hidden_extending_edge")
                .is_err(),
            "{query}"
        );
    }
    // A syntactically valid truncated trace is still impossible at shallow depth.
    let bounds = Bounds {
        rows_per_table: 4,
        ..B
    };
    let shallow = lower("SELECT ?x ?y WHERE {?x <urn:p>+ ?y}", bounds)
        .unwrap()
        .with_path_depth(1)
        .unwrap();
    let direct = lower("SELECT ?x ?y WHERE {?x <urn:p> ?y}", bounds).unwrap();
    let truncated = evaluate(&direct, &data, &FixtureEncoder).unwrap();
    let package = generate_operator_fixture(&shallow).unwrap();
    let workspace = path_workspace(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let input = prover_toml(
        &shallow,
        &data,
        &FixtureEncoder,
        &truncated,
        [7; 32],
        [3; 32],
    )
    .unwrap();
    assert!(driver.execute(&input, "path_shallow_truncated").is_err());
    support::complete(gate);
}

fn path_corpus_case(id: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("zk/sparql-evaluator/fixtures/conformance/cases.json")).unwrap(),
    )
    .unwrap();
    let rdf =
        std::fs::read_to_string(root.join("zk/sparql-evaluator/fixtures/conformance/core.nt"))
            .unwrap();
    let case = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .expect("path inventory case");
    let query = case["query"].as_str().unwrap();
    let source = case["dataset"].as_str().unwrap_or(&rdf);
    let (data, _) = dataset(source);
    let plan = lower(
        query,
        Bounds {
            quads_per_credential: 16,
            rows_per_table: 16,
            ..B
        },
    )
    .unwrap()
    .with_path_depth(3)
    .unwrap();
    let trace =
        evaluate(&plan, &data, &FixtureEncoder).unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
    fixtures::assert_engine(query, source, &trace.result);
    let ResultValue::Select { rows, variables } = &trace.result else {
        panic!("SELECT");
    };
    assert_eq!(
        serde_json::json!(variables),
        case["expected"]["result"]["Select"]["variables"]
    );
    use sparq_zk_query::interp::EncodeTerm;
    let mut expected = case["expected"]["result"]["Select"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    let (_, v) = dataset(&format!("<urn:s> <urn:p> {} .", t.as_str().unwrap()));
                    FixtureEncoder.encode(&v[2]).unwrap().enc
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut actual = rows
        .iter()
        .map(|r| r.iter().map(|c| c.enc).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected, "{}", case["id"]);
    let package = generate_operator_fixture(&plan).unwrap();
    let workspace = path_workspace(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let render =
        |t: &Trace| prover_toml(&plan, &data, &FixtureEncoder, t, [7; 32], [3; 32]).unwrap();
    let input = render(&trace);
    driver
        .execute(&input, "path_corpus")
        .unwrap_or_else(|e| panic!("{}: {e}", case["id"]));
    for i in 0..trace.tables.len() {
        let mut altered = trace.clone();
        if altered.tables[i].rows.is_empty() {
            altered.tables[i].rows.push(vec![
                Cell::default();
                plan.nodes()[i].output.variables.len()
            ]);
        } else {
            altered.tables[i].rows.pop();
        }
        assert!(
            driver
                .execute(&render(&altered), "path_corpus_table")
                .is_err(),
            "{}: node {i}",
            case["id"]
        );
    }
    let mut altered = trace.clone();
    alter_result(&mut altered, plan.result_schema().variables.len());
    assert!(
        driver
            .execute(&render(&altered), "path_corpus_result")
            .is_err(),
        "{}",
        case["id"]
    );
    assert!(
        driver
            .execute(
                &input.replace("request_digest = [7,", "request_digest = [8,"),
                "path_corpus_public"
            )
            .is_err(),
        "{}",
        case["id"]
    );
    std::fs::write(
        format!("/tmp/lane-c4-corpus-{id}.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"id":id,"status":"passing"})).unwrap(),
    )
    .unwrap();
}

#[test]
fn corpus_path_sequence() {
    let Some(gate) = support::gate("closed_paths::corpus_path_sequence", &["nargo"]) else {
        return;
    };
    path_corpus_case("path-sequence");
    support::complete(gate);
}

#[test]
fn corpus_path_inverse() {
    let Some(gate) = support::gate("closed_paths::corpus_path_inverse", &["nargo"]) else {
        return;
    };
    path_corpus_case("path-inverse");
    support::complete(gate);
}

#[test]
fn corpus_path_alternative_duplicates() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_path_alternative_duplicates",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("path-alternative-duplicates");
    support::complete(gate);
}

#[test]
fn corpus_path_transitive_deduplicates() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_path_transitive_deduplicates",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("path-transitive-deduplicates");
    support::complete(gate);
}

#[test]
fn corpus_path_zero_length() {
    let Some(gate) = support::gate("closed_paths::corpus_path_zero_length", &["nargo"]) else {
        return;
    };
    path_corpus_case("path-zero-length");
    support::complete(gate);
}

#[test]
fn corpus_path_zero_or_one() {
    let Some(gate) = support::gate("closed_paths::corpus_path_zero_or_one", &["nargo"]) else {
        return;
    };
    path_corpus_case("path-zero-or-one");
    support::complete(gate);
}

#[test]
fn corpus_path_negated_set() {
    let Some(gate) = support::gate("closed_paths::corpus_path_negated_set", &["nargo"]) else {
        return;
    };
    path_corpus_case("path-negated-set");
    support::complete(gate);
}

#[test]
fn corpus_nullable_path_alternative_absent_constant() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_nullable_path_alternative_absent_constant",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("nullable-path-alternative-absent-constant");
    support::complete(gate);
}

#[test]
fn corpus_path_negated_duplicate_forward() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_path_negated_duplicate_forward",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("path-negated-duplicate-forward");
    support::complete(gate);
}

#[test]
fn corpus_path_negated_duplicate_reverse() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_path_negated_duplicate_reverse",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("path-negated-duplicate-reverse");
    support::complete(gate);
}

#[test]
fn corpus_path_negated_both_orientations() {
    let Some(gate) = support::gate(
        "closed_paths::corpus_path_negated_both_orientations",
        &["nargo"],
    ) else {
        return;
    };
    path_corpus_case("path-negated-both-orientations");
    support::complete(gate);
}

#[test]
fn authenticated_generated_path_uses_complete_suite_dataset() {
    let Some(gate) = support::gate(
        "closed_paths::authenticated_generated_path_uses_complete_suite_dataset",
        &["nargo"],
    ) else {
        return;
    };
    let credential = authenticated::sign("<urn:a> <urn:p> <urn:b> .", 0);
    let cells = vec![credential.cells(0).unwrap()];
    let issuer_root = credential.issuer_root().unwrap();
    let plan = lower(
        "SELECT ?x ?y WHERE {?x <urn:p>+ ?y}",
        Bounds {
            rows_per_table: 2,
            ..B
        },
    )
    .unwrap()
    .with_path_depth(1)
    .unwrap();
    let trace = evaluate(&plan, &cells, &FixtureEncoder).unwrap();
    let package = generate(&plan).unwrap();
    let workspace = path_workspace(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let input = sparq_zk_query::witness::prover_toml_authenticated(
        &plan,
        &[credential],
        &trace,
        [7; 32],
        [3; 32],
        issuer_root,
    )
    .unwrap();
    driver.execute(&input, "path_authenticated").unwrap();
    support::complete(gate);
}

#[test]
fn paths_compose_with_closed_operators_and_named_graph_selection() {
    let Some(gate) = support::gate(
        "closed_paths::paths_compose_with_closed_operators_and_named_graph_selection",
        &["nargo"],
    ) else {
        return;
    };
    for query in [
        "SELECT ?s WHERE {?s <urn:p> ?o FILTER NOT EXISTS {?s <urn:q>+ ?v}}",
        "SELECT ?g ?o WHERE {GRAPH ?g {<urn:a> <urn:p>* ?o}}",
        "SELECT ?s ?o WHERE {?s <urn:p>+ ?o OPTIONAL {?o <urn:q>? ?x} MINUS {VALUES ?s {<urn:absent>}}}",
    ] {
        let (data,_)=dataset(RDF);
        let plan=lower(query,B).unwrap().with_path_depth(2).unwrap();
        let trace=evaluate(&plan,&data,&FixtureEncoder).unwrap();
        if !query.contains("GRAPH") {fixtures::assert_engine(query,RDF,&trace.result);}
        else {
            let graph=sparq_core::Graph::load_dataset(&RDF.lines().map(|l|l.trim_end_matches('.').to_owned()+" <urn:sparq:credential:0> .\n").collect::<String>(),"nquads").unwrap();
            let expected=sparq_engine::query(&graph,query).unwrap();
            let ResultValue::Select{rows,..}=&trace.result else {panic!("SELECT");};
            use sparq_zk_query::interp::EncodeTerm;
            let mut expected=expected.rows.iter().map(|r|r.iter().map(|t|t.as_ref().map(|t|FixtureEncoder.encode(t).unwrap().enc)).collect::<Vec<_>>()).collect::<Vec<_>>();
            let mut actual=rows.iter().map(|r|r.iter().map(|c|c.bound.then_some(c.enc)).collect::<Vec<_>>()).collect::<Vec<_>>();expected.sort();actual.sort();assert_eq!(expected,actual);
        }
        let package=generate_operator_fixture(&plan).unwrap();let workspace=path_workspace(&package);let driver=ProgramProver::new(&workspace.0,&package.name).unwrap();
        let input=prover_toml(&plan,&data,&FixtureEncoder,&trace,[7;32],[3;32]).unwrap();
        driver.execute(&input,"path_mixed_honest").unwrap_or_else(|e|panic!("{query}: {e}"));
        let mut changed=trace.clone();changed.tables[path_node(&plan)].rows.clear();
        let input=prover_toml(&plan,&data,&FixtureEncoder,&changed,[7;32],[3;32]).unwrap();
        assert!(driver.execute(&input,"path_mixed_hidden_rows").is_err(),"{query}");
    }
    support::complete(gate);
}

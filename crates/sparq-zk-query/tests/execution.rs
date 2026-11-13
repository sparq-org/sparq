#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/fixture.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

use fixture_support::{fixture, FixtureEncoder};
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    codegen::generate_operator_fixture as generate,
    interp::{evaluate, ResultValue},
    lower::lower,
    witness::prover_toml,
};

use fixture_support::Workspace;

#[test]
fn generated_driver_refuses_path_packages_before_io() {
    for name in [
        "",
        ".",
        "..",
        "../query",
        "a/b",
        "--query",
        "1query",
        "query.name",
        "query-name",
        "q%00",
    ] {
        match ProgramProver::new("/unused", name) {
            Err(sparq_zk_compose::driver::DriverError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput)
            }
            _ => panic!("package must be refused before tooling: {name:?}"),
        }
    }
    assert!(ProgramProver::new("/unused", "query_abc").is_ok());
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn generated_program_executes_trace_and_refuses_changed_rows_and_results() {
    let Some(gate) = support::gate(
        "generated_program_executes_trace_and_refuses_changed_rows_and_results",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 3)}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 4,
        },
    )
    .unwrap();
    let data = fixture();
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let package = generate(&plan).unwrap();
    let workspace = Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let artifact = driver.compile().unwrap();
    sparq_zk_query::statement::Layout::new(&plan)
        .audit_artifact(&serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap())
        .unwrap();
    let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
    driver.execute(&input, "honest").unwrap();
    for name in [
        "request_digest",
        "input_commitment",
        "signature_mode",
        "bounds",
        "parameters",
        "indices",
        "result",
        "statement_binding",
        "presentation_salt",
    ] {
        let altered = input
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{name} = ")) {
                    let position = line
                        .find("0x")
                        .map(|i| i + 65)
                        .unwrap_or_else(|| line.find(|c: char| c.is_ascii_digit()).unwrap());
                    let mut bytes = line.as_bytes().to_vec();
                    bytes[position] = if bytes[position] == b'1' { b'2' } else { b'1' };
                    String::from_utf8(bytes).unwrap()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(input.trim_end(), altered);
        assert!(
            driver.execute(&altered, "changed_public").is_err(),
            "{name}"
        );
    }
    for (i, t) in trace.tables.iter().enumerate() {
        for (r, row) in t.rows.iter().enumerate() {
            for v in 0..row.len() {
                let mut altered = trace.clone();
                altered.tables[i].rows[r][v].enc[31] ^= 1;
                let input =
                    prover_toml(&plan, &data, &FixtureEncoder, &altered, [7; 32], [3; 32]).unwrap();
                assert!(
                    driver.execute(&input, "changed_table").is_err(),
                    "table {i}, row {r}, column {v}"
                );
            }
        }
    }
    let mut altered = trace.clone();
    if let ResultValue::Select { rows, .. } = &mut altered.result {
        rows[0][0].enc[31] ^= 1;
    } else {
        panic!("SELECT")
    }
    let input = prover_toml(&plan, &data, &FixtureEncoder, &altered, [7; 32], [3; 32]).unwrap();
    assert!(driver.execute(&input, "changed_result").is_err());
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn generated_n0_shapes_execute_with_exact_table_controls() {
    use fixture_support::{iri, quad};
    use sparq_zk_query::interp::CredentialCells;
    let Some(gate) = support::gate(
        "generated_n0_shapes_execute_with_exact_table_controls",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let mut paired = fixture();
    paired.push(CredentialCells {
        quads: vec![
            quad(&iri("urn:a"), &iri("urn:q"), &iri("urn:g")),
            quad(&iri("urn:b"), &iri("urn:q"), &iri("urn:h")),
        ],
    });
    let decimal = |s: &str| {
        oxrdf::Literal::new_typed_literal(
            s,
            oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#decimal").unwrap(),
        )
        .into()
    };
    let decimals = vec![CredentialCells {
        quads: vec![
            quad(&iri("urn:a"), &iri("urn:p"), &decimal("1.25")),
            quad(&iri("urn:b"), &iri("urn:p"), &decimal("2.5")),
        ],
    }];
    let cases = vec![
        ("SELECT ?s WHERE {?s <urn:p> ?x}", fixture()),
        ("SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:p> ?y}", fixture()),
        ("SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:q> ?y}", paired),
        (
            "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 2)}",
            fixture(),
        ),
        (
            "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1.25 && ?x < 2.5)}",
            decimals,
        ),
        (
            "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x IN (1,?missing,2))}",
            fixture(),
        ),
        (
            "SELECT ?x WHERE {VALUES ?x {1 1 UNDEF} VALUES ?y {1}}",
            vec![],
        ),
        ("SELECT ?p WHERE {?s ?p ?o}", fixture()),
        ("SELECT DISTINCT ?p WHERE {?s ?p ?o}", fixture()),
        (
            "SELECT ?s WHERE {?s <urn:p> ?x} LIMIT 1 OFFSET 1",
            fixture(),
        ),
        ("SELECT ?s WHERE {?s <urn:p> ?x} LIMIT 0", fixture()),
        ("SELECT * WHERE {}", fixture()),
        ("ASK {}", vec![]),
        ("ASK {?s <urn:p> ?x}", fixture()),
        ("ASK {?s <urn:p> ?x FILTER(?x > 8)}", fixture()),
        ("ASK {FILTER(!( ?missing && false))}", vec![]),
        ("ASK {FILTER(2 NOT IN (1,3))}", vec![]),
        ("ASK {VALUES ?x {1} FILTER(?x IN ())}", vec![]),
    ];
    for (query, data) in cases {
        let plan = lower(
            query,
            Bounds {
                credentials: 2,
                quads_per_credential: 4,
                rows_per_table: 4,
            },
        )
        .unwrap();
        let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
        let package = generate(&plan).unwrap();
        let workspace = Workspace::new(&package);
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
        driver
            .execute(&input, "honest")
            .unwrap_or_else(|e| panic!("{query}: {e}"));
        for (i, table) in trace.tables.iter().enumerate() {
            let mut wrong = trace.clone();
            if let Some(row) = wrong.tables[i].rows.first_mut() {
                if let Some(cell) = row.first_mut() {
                    cell.enc[31] ^= 1;
                } else {
                    wrong.tables[i].rows.pop();
                }
            } else {
                wrong.tables[i].rows.push(vec![
                    sparq_zk_query::interp::Cell::default();
                    plan.nodes()[i].output.variables.len()
                ]);
            }
            let input =
                prover_toml(&plan, &data, &FixtureEncoder, &wrong, [7; 32], [3; 32]).unwrap();
            assert!(
                driver.execute(&input, "changed_table").is_err(),
                "{query}, table {i}, original rows {}",
                table.rows.len()
            );
        }
        let result_line = input.lines().find(|l| l.starts_with("result = ")).unwrap();
        let changed = match trace.result {
            ResultValue::Ask(value) => input.replace(
                result_line,
                if value {
                    "result = false"
                } else {
                    "result = true"
                },
            ),
            ResultValue::Select { .. } => {
                let mut line = result_line.as_bytes().to_vec();
                let position = result_line.find("enc = \"0x").unwrap() + "enc = \"0x".len() + 63;
                line[position] = if line[position] == b'1' { b'2' } else { b'1' };
                input.replace(result_line, &String::from_utf8(line).unwrap())
            }
        };
        assert!(
            driver.execute(&changed, "changed_result").is_err(),
            "{query}, result"
        );
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn seeded_engine_interpreter_and_circuit_differential() {
    let Some(gate) = support::gate(
        "seeded_engine_interpreter_and_circuit_differential",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let seeds = std::env::var("SPARQ_NOIR_QUERY_SEED")
        .map(|s| vec![s.parse().expect("integer seed")])
        .unwrap_or_else(|_| vec![0, 1, 7, 41, 893, 1201]);
    let mut workspaces = std::collections::BTreeMap::new();
    for seed in seeds {
        let (rdf, _, _) = fixture_support::generated_dataset(seed);
        let credentials = vec![authenticated::sign(&rdf, seed)];
        let data = vec![credentials[0].cells(0).unwrap()];
        for query in fixture_support::DIFFERENTIAL_QUERIES {
            let plan = lower(
                query,
                Bounds {
                    credentials: 1,
                    quads_per_credential: 16,
                    rows_per_table: 8,
                },
            )
            .unwrap();
            let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
            fixture_support::assert_engine(query, &rdf, &trace.result);
            let package = sparq_zk_query::codegen::generate(&plan).unwrap();
            let workspace = workspaces
                .entry(plan.hash())
                .or_insert_with(|| Workspace::new(&package));
            let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
            let input = sparq_zk_query::witness::prover_toml_authenticated(
                &plan,
                &credentials,
                &trace,
                [7; 32],
                [3; 32],
                authenticated::issuer_root(),
            )
            .unwrap();
            driver
                .execute(&input, "differential")
                .unwrap_or_else(|e| panic!("seed={seed}, {query}: {e}"));
            for (node, table) in trace.tables.iter().enumerate() {
                for row in 0..table.rows.len() {
                    let mut altered = trace.clone();
                    if let Some(cell) = altered.tables[node].rows[row].first_mut() {
                        cell.enc[31] ^= 1;
                    } else {
                        altered.tables[node].rows.remove(row);
                    }
                    let input = sparq_zk_query::witness::prover_toml_authenticated(
                        &plan,
                        &credentials,
                        &altered,
                        [7; 32],
                        [3; 32],
                        authenticated::issuer_root(),
                    )
                    .unwrap();
                    assert!(
                        driver.execute(&input, "altered_row").is_err(),
                        "seed={seed}, {query}, table={node}, row={row}"
                    );
                }
            }
            // A fixed binding also controls a changed request on every generated shape.
            let changed = input.replace("request_digest = [7,", "request_digest = [8,");
            assert_ne!(input, changed);
            assert!(
                driver.execute(&changed, "altered_request").is_err(),
                "seed={seed}, {query}"
            );
        }
    }
    support::complete(gate);
}

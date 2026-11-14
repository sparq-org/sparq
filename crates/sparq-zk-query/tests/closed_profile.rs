//! Credential profile and generated-runner contract controls.

mod closed_native;
mod support;
#[path = "support/fixture.rs"]
mod fixtures;
use sparq_zk_query::credential_graphs::evaluate_cells as evaluate;
use sparq_zk_query::{
    capacity::Bounds,
    codegen,
    credential::{Credential, SuiteEncoder},
    credential_graphs as graphs,
    interp::ResultValue,
    witness_closed,
};
use std::path::Path;
const EX: &str = "https://example.org/solid-suite#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
fn iri(s: &str) -> String {
    format!("<{s}>")
}
fn ex(s: &str) -> String {
    iri(&format!("{EX}{s}"))
}
fn triple(s: &str, p: &str, o: &str) -> String {
    let o = if o == "true" || o == "false" {
        format!("\"{o}\"^^<{XSD}boolean>")
    } else {
        o.into()
    };
    format!("{s} {p} {o} .\n")
}
fn int(n: i64) -> String {
    format!("\"{n}\"^^<{XSD}integer>")
}
fn signed(mut rdf: String, issuer: &str, salt: u64) -> Credential {
    rdf.push_str(&triple(
        "<urn:credential>",
        &iri(graphs::ISSUER),
        &iri(issuer),
    ));
    sign(&rdf, issuer, salt)
}
fn issuer_registry() -> Vec<(oxrdf::NamedNode, sparq_zk::sig::PublicKey)> {
    [
        "urn:fictional:insurance-quote:licensing",
        "urn:fictional:insurance-quote:claims-register",
        "urn:fictional:trial-eligibility:practice",
        "urn:fictional:recycled-content:owner",
        "urn:fictional:recycled-content:receiver",
        "urn:issuer",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        (
            oxrdf::NamedNode::new_unchecked(name),
            sparq_zk::sig::SecretKey::from_seed(19 + i as u64).public_key(),
        )
    })
    .collect()
}
fn issuer_root() -> [u8; 32] {
    let keys = issuer_registry()
        .into_iter()
        .map(|(_, k)| k)
        .collect::<Vec<_>>();
    sparq_zk::field::field_to_be_bytes_32(
        &sparq_zk_compose::issuer::key_set_root(&keys, 4).unwrap(),
    )
}
fn sign(source: &str, issuer: &str, salt: u64) -> Credential {
    let registry = issuer_registry();
    let index = registry
        .iter()
        .position(|(iri, _)| iri.as_str() == issuer)
        .unwrap();
    let key = sparq_zk::sig::SecretKey::from_seed(19 + index as u64);
    let config =
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
    let credential = sparq_merkle_suite::poseidon::issue(
        source,
        config,
        &key,
        sparq_merkle_suite::Fr::from(salt + 101),
    )
    .unwrap();
    let keys = registry.into_iter().map(|(_, key)| key).collect::<Vec<_>>();
    let issuer_siblings =
        sparq_zk_compose::issuer::key_membership_witness(&keys, 4, index as u64).unwrap();
    Credential {
        credential,
        issuer: key.public_key(),
        issuer_index: index as u64,
        issuer_siblings,
    }
}
fn bounds(_path: &str) -> Bounds {
    Bounds {
        credentials: 4,
        quads_per_credential: 8,
        rows_per_table: 4,
    }
}
#[test]
fn nested_exists_corpus_results_and_hidden_matches_are_bound() {
    let Some(gate) = support::gate(
        "nested_exists_corpus_results_and_hidden_matches_are_bound",
        &["nargo", "bb"],
    ) else {
        return;
    };
    use sparq_zk_query::{
        lower::lower, plan::Operator, plan_closed::ClosedOperator, witness::prover_toml,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("../../zk/sparql-evaluator/fixtures/conformance/cases.json"))
            .unwrap(),
    )
    .unwrap();
    let rdf = std::fs::read_to_string(
        root.join("../../zk/sparql-evaluator/fixtures/conformance/core.nt"),
    )
    .unwrap();
    let (data, _) = fixtures::dataset(&rdf);
    for (id, expected_rows) in [
        ("reject-exists-nested-exists", 3),
        ("reject-exists-nested-not-exists", 0),
    ] {
        let fixture = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == id)
            .unwrap();
        let query = fixture["query"].as_str().unwrap();
        let plan = lower(
            query,
            Bounds {
                credentials: 1,
                quads_per_credential: 16,
                rows_per_table: 4,
            },
        )
        .unwrap();
        let trace = evaluate(&plan, &data, &SuiteEncoder).unwrap();
        fixtures::assert_engine(query, &rdf, &trace.result);
        let ResultValue::Select { rows, .. } = &trace.result else {
            panic!("SELECT result")
        };
        assert_eq!(rows.len(), expected_rows, "{id}");
        let package = codegen::generate_operator_fixture(&plan).unwrap();
        let workspace = closed_native::Workspace::new(&package);
        let render = |trace: &sparq_zk_query::interp::Trace| {
            prover_toml(&plan, &data, &SuiteEncoder, trace, [7; 32], [3; 32]).unwrap()
        };
        execute_bounded(
            &gate,
            workspace.0.path(),
            &package.name,
            &render(&trace),
            "nested_honest",
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        // Hide a matching scan inside the nested EXISTS, including when
        // NOT EXISTS leaves the final result empty.
        let inner = plan
            .nodes()
            .iter()
            .enumerate()
            .find_map(|(i, node)| {
                matches!(node.operator, Operator::Closed(ClosedOperator::Scan { .. }))
                    .then_some(i)
                    .filter(|&i| !trace.tables[i].rows.is_empty())
            })
            .expect("matching nested scan");
        let mut hidden = trace.clone();
        hidden.tables[inner].rows.pop();
        let error = execute_bounded(
            &gate,
            workspace.0.path(),
            &package.name,
            &render(&hidden),
            "nested_hidden_match",
        )
        .unwrap_err();
        assert!(
            error.contains("operator length") || error.contains("operator rows"),
            "{id}: {error}"
        );
        let mut altered = trace.clone();
        let ResultValue::Select { rows, .. } = &mut altered.result else {
            unreachable!()
        };
        if rows.is_empty() {
            rows.push(trace.tables[0].rows[0][..1].to_vec());
        } else {
            rows.pop();
        }
        let error = execute_bounded(
            &gate,
            workspace.0.path(),
            &package.name,
            &render(&altered),
            "nested_wrong_result",
        )
        .unwrap_err();
        assert!(
            error.contains("operator length") || error.contains("operator rows"),
            "{id}: {error}"
        );
    }
    support::complete(gate);
}
#[test]
fn closed_string_reads_follow_the_shared_equality_contract() {
    for (body, value, identities, lexical_len) in [
        (
            "FILTER(?x = \"other\")",
            "\"a plain string longer than the scalar key bucket\"",
            true,
            0,
        ),
        (
            "FILTER(?x IN (\"other\", \"missing\"))",
            "\"a plain string longer than the scalar key bucket\"",
            true,
            0,
        ),
        ("BIND(?x = \"other\" AS ?ok)", "\"short\"", true, 0),
        (
            "OPTIONAL {?s <urn:q> ?y FILTER(?x = ?y)}",
            "\"short\"",
            true,
            0,
        ),
        ("FILTER(?x < \"z\")", "\"short\"", false, 5),
        (
            "FILTER(?x = \"other\" && ?x < \"z\")",
            "\"short\"",
            false,
            5,
        ),
        (
            "FILTER(?x = 2)",
            "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            true,
            1,
        ),
    ] {
        let plan = graphs::bind_issuer_registry(
            graphs::lower(
                &format!("SELECT * {{?s <urn:p> ?x {body}}}"),
                Bounds {
                    credentials: 1,
                    quads_per_credential: 4,
                    rows_per_table: 4,
                },
            )
            .unwrap(),
            &issuer_registry(),
        )
        .unwrap();
        let credentials = vec![signed(triple("<urn:s>", "<urn:p>", value), "urn:issuer", 3)];
        let trace = graphs::evaluate(&plan, &credentials).unwrap();
        let package = codegen::generate(&plan).unwrap();
        let source = &package.files[&format!("{}/src/main.nr", package.name)];
        let checker = if identities {
            "check_identity_equalities"
        } else {
            "check_with_parameters"
        };
        assert!(
            source.contains(&format!("core::ops_core_reads::{checker}(")),
            "{body}"
        );
        let witness = witness_closed::prover_toml(
            &plan,
            &credentials,
            &trace,
            [7; 32],
            [3; 32],
            issuer_root(),
        )
        .unwrap();
        let reads: Vec<_> = witness
            .lines()
            .filter(|line| line.starts_with("closed_reads_"))
            .collect();
        assert!(!reads.is_empty(), "{body}");
        let lengths: Vec<usize> = reads
            .iter()
            .flat_map(|line| line.split("lexical_len=").skip(1))
            .map(|value| value.split([',', '}']).next().unwrap().parse().unwrap())
            .collect();
        assert!(lengths.contains(&lexical_len), "{body}: {reads:?}");
        if lexical_len == 0 {
            assert!(
                reads.iter().all(|line| !line.contains("bound=true")),
                "{body}"
            );
        } else {
            assert!(
                reads.iter().any(|line| line.contains("bound=true")),
                "{body}"
            );
        }
    }
}
#[test]
fn duplicate_credentials_have_distinct_graphs_and_position_is_not_queryable() {
    let credential = signed(
        triple("<urn:s>", &ex("massGrams"), &int(1)),
        "urn:issuer",
        10,
    );
    let data = graphs::canonical_credentials(vec![credential.clone(), credential]).unwrap();
    let query = format!("SELECT ?g {{GRAPH ?g {{?s {} ?x}}}}", ex("massGrams"));
    let p = graphs::lower(
        &query,
        Bounds {
            credentials: 2,
            quads_per_credential: 8,
            rows_per_table: 4,
        },
    )
    .unwrap();
    let trace = evaluate(&p, &graphs::cells(&data).unwrap(), &SuiteEncoder).unwrap();
    let ResultValue::Select { rows, .. } = trace.result else {
        panic!("SELECT")
    };
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0][0].enc, rows[1][0].enc);
    for query in [
        format!("SELECT ?x {{GRAPH ?g {{?s {} ?x}}}}", ex("massGrams")),
        format!(
            "ASK {{GRAPH <urn:sparq:credential:0> {{?s {} ?x}}}}",
            ex("massGrams")
        ),
    ] {
        let p = graphs::lower(
            &query,
            Bounds {
                credentials: 2,
                quads_per_credential: 8,
                rows_per_table: 4,
            },
        )
        .unwrap();
        let cells = graphs::cells(&data).unwrap();
        let original = evaluate(&p, &cells, &SuiteEncoder).unwrap().result;
        let mut reversed = data.clone();
        reversed.reverse();
        let reversed = graphs::canonical_credentials(reversed).unwrap();
        assert_eq!(
            original,
            evaluate(&p, &graphs::cells(&reversed).unwrap(), &SuiteEncoder)
                .unwrap()
                .result
        );
        if query.starts_with("ASK") {
            assert_eq!(original, ResultValue::Ask(false));
        }
    }
}
#[test]
fn signed_blank_node_cannot_alias_the_credential_graph() {
    let credential = signed("_:vcqGraph <urn:p> <urn:o> .\n".into(), "urn:issuer", 1);
    let p = graphs::lower(
        "ASK {GRAPH ?g {?g <urn:p> <urn:o>}}",
        Bounds {
            credentials: 1,
            quads_per_credential: 8,
            rows_per_table: 4,
        },
    )
    .unwrap();
    let p = graphs::bind_issuer_registry(p, &issuer_registry()).unwrap();
    assert_eq!(
        graphs::evaluate(&p, &[credential]).unwrap().result,
        ResultValue::Ask(false)
    );
}
#[test]
fn compact_profile_credential_executes() {
    let Some(gate) = support::gate("compact_profile_credential_executes", &["nargo", "bb"]) else {
        return;
    };
    let p = graphs::bind_issuer_registry(
        graphs::lower(
            "ASK {}",
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 2,
            },
        )
        .unwrap(),
        &issuer_registry(),
    )
    .unwrap();
    let credentials = vec![signed(
        triple("<urn:s>", "<urn:p>", &int(1)),
        "urn:issuer",
        3,
    )];
    let trace = graphs::evaluate(&p, &credentials).unwrap();
    let pkg = codegen::generate(&p).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (name, source) in pkg.files {
        let target = temp.path().join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        temp.path().join("zk"),
    )
    .unwrap();
    let witness =
        witness_closed::prover_toml(&p, &credentials, &trace, [7; 32], [3; 32], issuer_root())
            .unwrap();
    execute_bounded(&gate, temp.path(), &pkg.name, &witness, "compact_authenticated").unwrap();
    let raw_line = witness
        .lines()
        .find(|line| line.starts_with("credential_0 = "))
        .unwrap();
    for (field, label) in [("subject", "nonliteral position"),("predicate", "predicate kind")] {
        let marker=format!("{field} = {{ kind = 1");
        let position=raw_line.find(&marker).expect("bounded resource opening")+marker.len()-8;
        let mut raw = raw_line.to_owned();
        raw.replace_range(position..position + 8, "kind = 3");
        let changed = witness.replace(raw_line, &raw);
        let error = execute_bounded(&gate, temp.path(), &pkg.name, &changed, "compact_wrong_position")
            .unwrap_err();
        assert!(error.contains(label), "{error}");
    }
    let raw_key = |n| {
        format!(
            "key = [{}]",
            sparq_merkle_suite::comparison_key(&oxrdf::Literal::from(n))
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    assert_eq!(witness.matches(&raw_key(1i64)).count(), 1);
    let changed = witness.replacen(&raw_key(1i64), &raw_key(2i64), 1);
    let error = execute_bounded(
        &gate,
        temp.path(),
        &pkg.name,
        &changed,
        "compact_wrong_integer_key",
    )
    .unwrap_err();
    // The suite's signed term encoding includes the key, so this alteration first fails its signature challenge.
    assert!(
        error.contains("challenge reduction not bound to its preimage"),
        "{error}"
    );
    let c = &credentials[0];
    let signature = sparq_zk::sig::in_circuit_witness(
        &c.issuer,
        &c.signed_message().unwrap(),
        &c.credential.signature,
    )
    .unwrap();
    let marker = format!("s = \"{}\"", sparq_zk::field::field_to_hex(&signature.s));
    assert_eq!(witness.matches(&marker).count(), 1);
    let changed = witness.replacen(&marker, "s = \"0\"", 1);
    let error = execute_bounded(
        &gate,
        temp.path(),
        &pkg.name,
        &changed,
        "compact_wrong_signature",
    )
    .unwrap_err();
    assert!(
        error.contains("schnorr verification equation failed"),
        "{error}"
    );
    support::complete(gate);
}

fn execute_bounded(
    gate: &support::LiveToolchain,
    root: &Path,
    package: &str,
    input: &str,
    tag: &str,
) -> Result<(), String> {
    for (tool, _) in pins::PINNED_TOOLS {
        let output = support::live_tool(gate, tool)
            .arg("--version")
            .output()
            .expect("pinned tool version probe starts");
        assert!(output.status.success(), "{tool} version probe failed");
        assert!(
            pins::pinned_version_output(tool, &output.stdout),
            "query method requires the pinned toolchain: {tool}"
        );
    }
    std::fs::write(root.join(package).join(format!("Prover_{tag}.toml")), input)
        .map_err(|e| e.to_string())?;
    let witness = root.join("target").join(format!("{tag}.gz"));
    if witness.exists() {
        std::fs::remove_file(&witness).map_err(|e| e.to_string())?;
    }
    let output = support::live_tool(gate, "nargo")
        .args([
            "execute",
            "--inliner-aggressiveness",
            "0",
            "--package",
            package,
            "--prover-name",
            &format!("Prover_{tag}"),
            tag,
        ])
        .current_dir(root)
        .output()
        .expect("nargo execute starts");
    if !output.status.success() {
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Assertion failed:"),
            "unexpected native failure: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if output.status.success() && witness.is_file() {
        Ok(())
    } else {
        Err(format!(
            "native circuit execution {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
#[path = "../../sparq-zk-compose/src/toolchain.rs"]
mod pins;

#[test]
fn bounded_runner_requires_a_fresh_witness_for_the_generated_relation() {
    let Some(gate) = support::gate(
        "bounded_runner_requires_a_fresh_witness_for_the_generated_relation",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let p = sparq_zk_query::lower::lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let trace = evaluate(&p, &[], &SuiteEncoder).unwrap();
    let package = codegen::generate_operator_fixture(&p).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (name, source) in &package.files {
        let path = temp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        temp.path().join("zk"),
    )
    .unwrap();
    let input =
        sparq_zk_query::witness::prover_toml(&p, &[], &SuiteEncoder, &trace, [7; 32], [3; 32])
            .unwrap();
    execute_bounded(&gate, temp.path(), &package.name, &input, "runner_guard").unwrap();
    assert!(temp.path().join("target/runner_guard.gz").exists());
    let mut false_result = trace;
    false_result.result = ResultValue::Ask(false);
    let input = sparq_zk_query::witness::prover_toml(
        &p,
        &[],
        &SuiteEncoder,
        &false_result,
        [7; 32],
        [3; 32],
    )
    .unwrap();
    assert!(execute_bounded(&gate, temp.path(), &package.name, &input, "runner_guard").is_err());
    assert!(!temp.path().join("target/runner_guard.gz").exists());
    support::complete(gate);
}

#[test]
fn literal_families_are_plan_bound_and_cover_unused_rows() {
    let Some(gate) = support::gate(
        "literal_families_are_plan_bound_and_cover_unused_rows",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let all = graphs::bind_issuer_registry(
        graphs::lower("ASK {}", bounds("recycled-content")).unwrap(),
        &issuer_registry(),
    )
    .unwrap();
    let integer = graphs::with_literal_types(all.clone(), 1).unwrap();
    assert_ne!(all.hash(), integer.hash());
    let dates = graphs::with_literal_types(all.clone(), 6).unwrap();
    assert_ne!(integer.hash(), dates.hash());
    let long = vec![signed(
        triple("<urn:s>", "<urn:p>", &int(10000000000000000)),
        "urn:issuer",
        5,
    )];
    assert_eq!(
        graphs::evaluate(&all, &long).unwrap().result,
        ResultValue::Ask(true)
    );
    assert!(matches!(
        graphs::evaluate(&integer, &long),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
    let credentials = vec![signed(
        triple("<urn:s>", "<urn:p>", "true"),
        "urn:issuer",
        5,
    )];
    assert_eq!(
        graphs::evaluate(&all, &credentials).unwrap().result,
        ResultValue::Ask(true)
    );
    assert!(graphs::evaluate(&integer, &credentials)
        .unwrap_err()
        .to_string()
        .contains("closed credential literal profile"));
    let cells = graphs::cells(&credentials).unwrap();
    assert!(evaluate(&integer, &cells, &SuiteEncoder)
        .unwrap_err()
        .to_string()
        .contains("closed credential literal profile"));
    let valid = vec![signed(
        triple("<urn:s>", "<urn:p>", &int(1)),
        "urn:issuer",
        5,
    )];
    let trace = graphs::evaluate(&integer, &valid).unwrap();
    let package = codegen::generate_operator_fixture(&integer).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (name, source) in &package.files {
        let target = temp.path().join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        temp.path().join("zk"),
    )
    .unwrap();
    let render = |credentials: &[Credential]| {
        sparq_zk_query::witness::prover_toml(
            &integer,
            &graphs::cells(credentials).unwrap(),
            &SuiteEncoder,
            &trace,
            [7; 32],
            [3; 32],
        )
        .unwrap()
    };
    execute_bounded(&gate, temp.path(), &package.name, &render(&valid), "types_honest").unwrap();
    let error = execute_bounded(
        &gate,
        temp.path(),
        &package.name,
        &render(&credentials),
        "types_unused_boolean",
    )
    .unwrap_err();
    assert!(
        error.contains("closed credential literal profile"),
        "{error}"
    );
    support::complete(gate);
}

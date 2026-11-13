//! Draft Solid acceptance paths. Envelope and issuer binding remain lane E seams.

#[path = "support/mod.rs"]
mod support;
use sparq_zk_query::{
    capacity::Bounds,
    codegen,
    credential::{Credential, SuiteEncoder},
    credential_graphs as graphs,
    interp::{evaluate, ResultValue},
    plan::{Operator, Plan},
    plan_closed::ClosedOperator,
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
fn date(s: &str) -> String {
    format!("\"{s}\"^^<{XSD}date>")
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
fn fixtures(path: &str, case: &str) -> Vec<Credential> {
    let owner = "<urn:owner>";
    let mut out = match path {
        "insurance-quote" => {
            let lic = triple(
                owner,
                &ex("dateOfBirth"),
                &date(if case == "young" {
                    "2003-05-01"
                } else {
                    "1990-01-01"
                }),
            ) + &triple(owner, &ex("method"), &ex("recorded-by-authority"));
            let mut reg = triple(owner, &ex("claimsPeriodStart"), &date("2021-11-13"))
                + &triple(owner, &ex("claimsPeriodEnd"), &date("2026-11-13"))
                + &triple(
                    owner,
                    &ex("claimsComplete"),
                    if case == "incomplete" {
                        "false"
                    } else {
                        "true"
                    },
                )
                + &triple(owner, &ex("method"), &ex("recorded-by-authority"));
            if case == "claim" {
                reg += &(triple("<urn:claim:1>", &ex("claimant"), owner)
                    + &triple("<urn:claim:1>", &ex("claimDate"), &date("2024-03-02")));
            }
            vec![
                signed(lic, "urn:fictional:insurance-quote:licensing", 0),
                signed(reg, "urn:fictional:insurance-quote:claims-register", 1),
            ]
        }
        "trial-eligibility" => {
            let prac = triple(
                owner,
                &ex("medicationPeriodStart"),
                &date(if case == "short-period" {
                    "2026-11-01"
                } else {
                    "2026-08-01"
                }),
            ) + &triple(owner, &ex("medicationPeriodEnd"), &date("2026-11-13"))
                + &triple(
                    owner,
                    &ex("medicationComplete"),
                    if case == "incomplete" {
                        "false"
                    } else {
                        "true"
                    },
                )
                + &triple(owner, &ex("method"), &ex("recorded-by-authority"))
                + &triple(
                    owner,
                    &ex("takes"),
                    if case == "excluded" {
                        "<urn:fictional:medicine-b>"
                    } else {
                        "<urn:fictional:medicine-a>"
                    },
                );
            vec![
                signed(prac, "urn:fictional:trial-eligibility:practice", 0),
                signed(
                    triple(owner, &ex("dateOfBirth"), &date("1990-01-01")),
                    "urn:fictional:trial-eligibility:practice",
                    1,
                ),
            ]
        }
        "recycled-content" => {
            let mut bom = String::new();
            if case != "empty" {
                bom += &(triple(&ex("product-1"), &ex("hasInput"), "<urn:lot:1>")
                    + &triple(&ex("product-1"), &ex("hasInput"), "<urn:lot:2>"));
            }
            if case == "missing-audit" {
                bom += &triple(&ex("product-1"), &ex("hasInput"), "<urn:lot:3>");
            }
            let audit = |lot: &str, mass, rec, method: &str| {
                triple(lot, &ex("massGrams"), &int(mass))
                    + &triple(lot, &ex("recycledGrams"), &int(rec))
                    + &triple(lot, &ex("method"), &ex(method))
            };
            let a1 = audit(
                "<urn:lot:1>",
                600000,
                if case == "below" { 290000 } else { 300000 },
                "audited-by-certifier",
            );
            let a2 = audit(
                "<urn:lot:2>",
                400000,
                if case == "below" || case == "boundary" {
                    0
                } else {
                    50000
                },
                if case == "self-reported" {
                    "self-reported"
                } else {
                    "audited-by-certifier"
                },
            );
            let mut v = vec![
                signed(bom, "urn:fictional:recycled-content:owner", 0),
                signed(a1.clone(), "urn:fictional:recycled-content:receiver", 1),
                signed(a2, "urn:fictional:recycled-content:receiver", 2),
            ];
            if case == "double-audit" {
                v.push(signed(a1, "urn:fictional:recycled-content:receiver", 3));
            }
            v
        }
        _ => panic!("path"),
    };
    out = graphs::canonical_credentials(out);
    out
}
fn query(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/solid_paths/{path}.rq")),
    )
    .unwrap()
}
fn bounds(_path: &str) -> Bounds {
    Bounds {
        credentials: 4,
        quads_per_credential: 8,
        rows_per_table: 4,
    }
}
fn prepare(path: &str) -> Plan {
    let p = graphs::bind_issuer_registry(
        graphs::lower(&query(path), bounds(path)).unwrap(),
        &issuer_registry(),
    )
    .unwrap();
    graphs::with_literal_types(p, if path == "recycled-content" { 1 } else { 6 })
        .unwrap()
        .with_credential_quad_bounds(&[8, 8, 8, 8])
        .unwrap()
}
fn engine(path: &str, credentials: &[Credential]) -> bool {
    let mut rdf = String::new();
    for (i, c) in credentials.iter().enumerate() {
        for q in oxttl::NQuadsParser::new().for_slice(c.credential.document.as_bytes()) {
            let q = q.unwrap();
            rdf += &format!("{} <{}> {} .\n", q.subject, q.predicate.as_str(), q.object);
            rdf += &format!(
                "{} <{}> {} _:g{i} .\n",
                q.subject,
                q.predicate.as_str(),
                q.object
            );
            if q.predicate.as_str() == graphs::ISSUER {
                rdf += &format!("_:g{i} <{}> {} .\n", graphs::VERIFIED_ISSUER, q.object);
            }
        }
    }
    let graph = sparq_core::Graph::load_dataset(&rdf, "nquads").unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&sparq_engine::query_json(&graph, &query(path)).unwrap()).unwrap();
    result["boolean"].as_bool().unwrap()
}
#[test]
fn solid_draft_queries_engine_interpreter_and_permutations() {
    for (path, cases) in [
        (
            "insurance-quote",
            vec![
                ("honest", true),
                ("claim", false),
                ("incomplete", false),
                ("young", false),
            ],
        ),
        (
            "trial-eligibility",
            vec![
                ("honest", true),
                ("excluded", false),
                ("incomplete", false),
                ("short-period", false),
            ],
        ),
        (
            "recycled-content",
            vec![
                ("honest", true),
                ("below", false),
                ("boundary", true),
                ("self-reported", false),
                ("missing-audit", false),
                ("double-audit", false),
                ("empty", false),
            ],
        ),
    ] {
        let p = prepare(path);
        println!("{path}: nodes={}", p.nodes().len());
        for (case, expected) in cases {
            let credentials = fixtures(path, case);
            let cells = graphs::cells(&credentials).unwrap();
            let trace = evaluate(&p, &cells, &SuiteEncoder)
                .unwrap_or_else(|e| panic!("{path}/{case}: {e}"));
            assert_eq!(trace.result, ResultValue::Ask(expected), "{path}/{case}");
            assert_eq!(engine(path, &credentials), expected, "engine {path}/{case}");
            let mut reversed = credentials.clone();
            reversed.reverse();
            let canonical = graphs::canonical_credentials(reversed);
            assert_eq!(graphs::cells(&canonical).unwrap(), cells);
        }
    }
}
fn export(path: &str, root: &Path) -> (Plan, Vec<Credential>, sparq_zk_query::interp::Trace) {
    let p = prepare(path);
    let credentials = fixtures(path, "honest");
    let cells = graphs::cells(&credentials).unwrap();
    let trace = evaluate(&p, &cells, &SuiteEncoder).unwrap();
    let package = codegen::generate(&p).unwrap();
    std::fs::create_dir_all(root).unwrap();
    for (name, body) in package.files {
        let target = root.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, body).unwrap();
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        root.join("zk"),
    )
    .unwrap();
    let witness =
        witness_closed::prover_toml(&p, &credentials, &trace, [7; 32], [3; 32], issuer_root())
            .unwrap();
    let name = std::fs::read_to_string(root.join("Nargo.toml"))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .to_owned();
    std::fs::write(root.join(&name).join("Prover.toml"), witness).unwrap();
    let mut wrong = trace.clone();
    wrong.result = ResultValue::Ask(false);
    std::fs::write(
        root.join(&name).join("Prover_wrong.toml"),
        witness_closed::prover_toml(&p, &credentials, &wrong, [7; 32], [3; 32], issuer_root())
            .unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("package-name"), name).unwrap();
    let op = codegen::generate_operator_fixture(&p).unwrap();
    let op_root = root.join("operator");
    for (name, body) in &op.files {
        let target = op_root.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, body).unwrap();
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        op_root.join("zk"),
    )
    .unwrap();
    std::fs::write(
        op_root.join(&op.name).join("Prover.toml"),
        sparq_zk_query::witness::prover_toml(&p, &cells, &SuiteEncoder, &trace, [7; 32], [3; 32])
            .unwrap(),
    )
    .unwrap();
    std::fs::write(op_root.join("package-name"), &op.name).unwrap();
    (p, credentials, trace)
}
#[test]
fn export_solid_path_programs_when_requested() {
    if let Ok(root) = std::env::var("SPARQ_CLOSED_EXPORT_DIR") {
        for path in ["insurance-quote", "trial-eligibility", "recycled-content"] {
            let _ = export(path, &Path::new(&root).join(path));
        }
    }
}
#[test]
fn solid_path_capacity_and_graph_identity_controls() {
    let p = prepare("insurance-quote");
    let mut c = fixtures("insurance-quote", "honest");
    c.extend(c.clone());
    c.push(c[0].clone());
    assert!(matches!(
        evaluate(&p, &graphs::cells(&c).unwrap(), &SuiteEncoder),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
    let mut oversized = String::new();
    for i in 0..9 {
        oversized.push_str(&triple(&format!("<urn:s:{i}>"), "<urn:p>", "<urn:o>"));
    }
    let oversized = signed(oversized, "urn:fictional:recycled-content:owner", 99);
    assert!(matches!(
        graphs::evaluate(&prepare("recycled-content"), &[oversized]),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
    let mut c = fixtures("recycled-content", "honest");
    c.push(c[1].clone());
    c = graphs::canonical_credentials(c);
    let p = graphs::lower(
        "SELECT ?g {GRAPH ?g {?s ?p ?o}}",
        Bounds {
            rows_per_table: 32,
            ..bounds("recycled-content")
        },
    )
    .unwrap();
    let trace = evaluate(&p, &graphs::cells(&c).unwrap(), &SuiteEncoder).unwrap();
    let ResultValue::Select { rows, .. } = trace.result else {
        panic!("SELECT")
    };
    assert!(rows.iter().all(|r| r[0].kind == 2));
    assert!(p.nodes().iter().any(|n| matches!(
        n.operator,
        Operator::Closed(ClosedOperator::GraphScan { .. })
    )));
}
fn exercise_authenticated_path(path: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("program");
    let (p, credentials, trace) = export(path, &root);
    let name = std::fs::read_to_string(root.join("package-name")).unwrap();
    let render = |credentials: &[Credential], trace: &sparq_zk_query::interp::Trace| {
        witness_closed::prover_toml(&p, credentials, trace, [7; 32], [3; 32], issuer_root())
            .unwrap()
    };
    execute_bounded(&root, &name, &render(&credentials, &trace), "solid_honest")
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    measure_gates(&root, &name, path);
    let cases: &[&str] = match path {
        "insurance-quote" => &["claim", "incomplete", "young"],
        "trial-eligibility" => &["excluded", "incomplete", "short-period"],
        _ => &[
            "below",
            "boundary",
            "self-reported",
            "missing-audit",
            "double-audit",
            "empty",
        ],
    };
    for case in cases {
        let data = fixtures(path, case);
        let cells = graphs::cells(&data).unwrap();
        let actual = evaluate(&p, &cells, &SuiteEncoder).unwrap();
        assert_eq!(actual.result, ResultValue::Ask(engine(path, &data)));
        execute_bounded(&root, &name, &render(&data, &actual), "solid_case")
            .unwrap_or_else(|e| panic!("{path}/{case}: {e}"));
    }
    let negative = fixtures(
        path,
        match path {
            "insurance-quote" => "claim",
            "trial-eligibility" => "excluded",
            _ => "double-audit",
        },
    );
    let negative_trace = graphs::evaluate(&p, &negative).unwrap();
    assert_eq!(negative_trace.result, ResultValue::Ask(false));
    let mut rights = vec![];
    for node in p.nodes() {
        if let Operator::Closed(ClosedOperator::Filter { predicate, .. }) = &node.operator {
            existence_inputs(predicate, &mut rights);
        }
    }
    let right = rights
        .into_iter()
        .find(|&id| !negative_trace.tables[id as usize].rows.is_empty())
        .expect("a complete matching right table");
    let mut hidden = negative_trace.clone();
    hidden.tables[right as usize].rows.pop();
    let failure = execute_bounded(
        &root,
        &name,
        &render(&negative, &hidden),
        "solid_hidden_matching_right",
    )
    .unwrap_err();
    assert!(
        failure.contains("operator length") || failure.contains("operator rows"),
        "{failure}"
    );
    let mut altered = trace.clone();
    altered.result = ResultValue::Ask(false);
    assert!(
        execute_bounded(
            &root,
            &name,
            &render(&credentials, &altered),
            "solid_wrong_result"
        )
        .is_err(),
        "{path}"
    );
    let scan = p
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(i, n)| {
            (matches!(
                n.operator,
                Operator::Closed(ClosedOperator::GraphScan { .. })
            ) && !trace.tables[i].rows.is_empty())
            .then_some(i)
        })
        .unwrap();
    let mut hidden = trace.clone();
    hidden.tables[scan].rows.pop();
    assert!(
        execute_bounded(
            &root,
            &name,
            &render(&credentials, &hidden),
            "solid_hidden_row"
        )
        .is_err(),
        "{path}"
    );
    let input = render(&credentials, &trace);
    let line = input.lines().find(|l| l.starts_with("counts = ")).unwrap();
    let mut counts = credentials
        .iter()
        .map(|c| c.credential.count)
        .collect::<Vec<_>>();
    counts.resize(4, 0);
    counts[0] -= 1;
    let changed = input.replace(line, &format!("counts = {counts:?}"));
    assert!(
        execute_bounded(&root, &name, &changed, "solid_hidden_credential_quad").is_err(),
        "{path}"
    );
}
#[test]
fn solid_recycled_content_authenticated() {
    exercise_authenticated_path("recycled-content");
}
#[test]
fn solid_insurance_quote_authenticated() {
    exercise_authenticated_path("insurance-quote");
}
#[test]
fn solid_trial_eligibility_authenticated() {
    exercise_authenticated_path("trial-eligibility");
}

fn existence_inputs(predicate: &sparq_zk_query::plan_closed::Predicate, ids: &mut Vec<u32>) {
    use sparq_zk_query::plan_closed::Predicate as P;
    match predicate {
        P::Exists(rights) => ids.extend(rights),
        P::Not(a) => existence_inputs(a, ids),
        P::And(a, b) | P::Or(a, b) => {
            existence_inputs(a, ids);
            existence_inputs(b, ids);
        }
        _ => (),
    }
}

#[test]
fn duplicate_credentials_have_distinct_graphs_and_position_is_not_queryable() {
    let credential = signed(
        triple("<urn:s>", &ex("massGrams"), &int(1)),
        "urn:issuer",
        10,
    );
    let data = graphs::canonical_credentials(vec![credential.clone(), credential]);
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
        let reversed = graphs::canonical_credentials(reversed);
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
fn issuer_identity_is_bound_to_the_verified_signing_key() {
    let p = prepare("insurance-quote");
    let mut credentials = fixtures("insurance-quote", "honest");
    let i = credentials
        .iter()
        .position(|c| {
            c.credential
                .document
                .contains("urn:fictional:insurance-quote:licensing")
        })
        .unwrap();
    let wrong = sign(
        &credentials[i].credential.document,
        "urn:fictional:trial-eligibility:practice",
        99,
    );
    credentials[i] = wrong;
    credentials = graphs::canonical_credentials(credentials);
    assert!(graphs::evaluate(&p, &credentials)
        .unwrap_err()
        .to_string()
        .contains("verified issuer registry"));
    let cells = graphs::cells(&credentials).unwrap();
    let untrusted = evaluate(&p, &cells, &SuiteEncoder).unwrap();
    assert!(witness_closed::prover_toml(
        &p,
        &credentials,
        &untrusted,
        [7; 32],
        [3; 32],
        issuer_root()
    )
    .unwrap_err()
    .to_string()
    .contains("verified issuer registry"));
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
    execute_bounded(temp.path(), &pkg.name, &witness, "compact_authenticated").unwrap();
    let raw_key = |n| {
        format!(
            "key=[{}]",
            sparq_merkle_suite::comparison_key(&oxrdf::Literal::from(n))
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert_eq!(witness.matches(&raw_key(1i64)).count(), 1);
    let changed = witness.replacen(&raw_key(1i64), &raw_key(2i64), 1);
    let error = execute_bounded(
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
    let signature =
        sparq_zk::sig::in_circuit_witness(&c.issuer, &c.signed_message(), &c.credential.signature)
            .unwrap();
    let marker = format!("s=\"{}\"", sparq_zk::field::field_to_hex(&signature.s));
    assert_eq!(witness.matches(&marker).count(), 1);
    let changed = witness.replacen(&marker, "s=\"0\"", 1);
    let error =
        execute_bounded(temp.path(), &pkg.name, &changed, "compact_wrong_signature").unwrap_err();
    assert!(
        error.contains("schnorr verification equation failed"),
        "{error}"
    );
}

fn execute_bounded(root: &Path, package: &str, input: &str, tag: &str) -> Result<(), String> {
    for (tool, _) in pins::PINNED_TOOLS {
        let output = std::process::Command::new(tool)
            .arg("--version")
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() || !pins::pinned_version_output(tool, &output.stdout) {
            return Err("query method requires the pinned toolchain".into());
        }
    }
    std::fs::write(root.join(package).join(format!("Prover_{tag}.toml")), input)
        .map_err(|e| e.to_string())?;
    let witness = root.join("target").join(format!("{tag}.gz"));
    match std::fs::remove_file(&witness) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    // Bound each native command by the permitted execution-step budget.
    let output = std::process::Command::new("timeout")
        .args([
            "600",
            "nargo",
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
        .map_err(|e| e.to_string())?;
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

fn measure_gates(root: &Path, package: &str, path: &str) {
    let output = std::process::Command::new("timeout")
        .args([
            "600",
            "bb",
            "gates",
            "-s",
            "ultra_honk",
            "-t",
            "noir-recursive",
            "-b",
        ])
        .arg(root.join("target").join(format!("{package}.json")))
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{path} gate measurement {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let destination = std::path::Path::new("/tmp/lane-c3-path-measurements");
    std::fs::create_dir_all(destination).unwrap();
    std::fs::write(destination.join(format!("{path}.json")), &output.stdout).unwrap();
    assert!(
        value["functions"][0]["circuit_size"].as_u64().unwrap() <= (1 << 23),
        "{path} exceeds the proving budget: {value}"
    );
}
fn exercise_operator_path(path: &str) {
    let p = prepare(path);
    let package = codegen::generate_operator_fixture(&p).unwrap();
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
    let render = |data: &[Credential], trace: &sparq_zk_query::interp::Trace| {
        sparq_zk_query::witness::prover_toml(
            &p,
            &graphs::cells(data).unwrap(),
            &SuiteEncoder,
            trace,
            [7; 32],
            [3; 32],
        )
        .unwrap()
    };
    let cases: &[&str] = match path {
        "insurance-quote" => &["honest", "claim", "incomplete", "young"],
        "trial-eligibility" => &["honest", "excluded", "incomplete", "short-period"],
        _ => &[
            "honest",
            "below",
            "boundary",
            "self-reported",
            "missing-audit",
            "double-audit",
            "empty",
        ],
    };
    for case in cases {
        let data = fixtures(path, case);
        let trace = graphs::evaluate(&p, &data).unwrap();
        assert_eq!(trace.result, ResultValue::Ask(engine(path, &data)));
        execute_bounded(
            temp.path(),
            &package.name,
            &render(&data, &trace),
            "operator_path",
        )
        .unwrap_or_else(|e| panic!("{path}/{case}: {e}"));
        if *case == "honest" {
            measure_gates(temp.path(), &package.name, &format!("{path}-operator"));
        }
    }
    let data = fixtures(
        path,
        match path {
            "insurance-quote" => "claim",
            "trial-eligibility" => "excluded",
            _ => "double-audit",
        },
    );
    let trace = graphs::evaluate(&p, &data).unwrap();
    assert_eq!(trace.result, ResultValue::Ask(false));
    let mut rights = vec![];
    for node in p.nodes() {
        if let Operator::Closed(ClosedOperator::Filter { predicate, .. }) = &node.operator {
            existence_inputs(predicate, &mut rights);
        }
    }
    let right = rights
        .into_iter()
        .find(|&id| !trace.tables[id as usize].rows.is_empty())
        .unwrap();
    let mut hidden = trace.clone();
    hidden.tables[right as usize].rows.pop();
    let failure = execute_bounded(
        temp.path(),
        &package.name,
        &render(&data, &hidden),
        "operator_hidden_right",
    )
    .unwrap_err();
    assert!(
        failure.contains("operator length") || failure.contains("operator rows"),
        "{failure}"
    );
    let mut wrong = trace.clone();
    wrong.result = ResultValue::Ask(true);
    assert!(execute_bounded(
        temp.path(),
        &package.name,
        &render(&data, &wrong),
        "operator_wrong_result"
    )
    .is_err());
}
#[test]
fn solid_recycled_content_operator_fixture() {
    exercise_operator_path("recycled-content");
}
#[test]
fn solid_insurance_quote_operator_fixture() {
    exercise_operator_path("insurance-quote");
}
#[test]
fn solid_trial_eligibility_operator_fixture() {
    exercise_operator_path("trial-eligibility");
}

#[test]
fn bounded_runner_requires_a_fresh_witness_for_the_generated_relation() {
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
    execute_bounded(temp.path(), &package.name, &input, "runner_guard").unwrap();
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
    assert!(execute_bounded(temp.path(), &package.name, &input, "runner_guard").is_err());
    assert!(!temp.path().join("target/runner_guard.gz").exists());
}

#[test]
fn literal_families_are_plan_bound_and_cover_unused_rows() {
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
    execute_bounded(temp.path(), &package.name, &render(&valid), "types_honest").unwrap();
    let error = execute_bounded(
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
}

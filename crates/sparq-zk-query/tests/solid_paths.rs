//! Consumer target queries over complete authenticated credential snapshots.
mod closed_native;
#[path = "support/mod.rs"]
mod support;
use sparq_zk_query::{
    answer::Answer,
    capacity::Bounds,
    codegen,
    credential::{Credential, SuiteEncoder},
    credential_graphs as graphs,
    interp::{ResultValue, Trace},
    plan::{Operator, Plan},
    plan_closed::{ClosedOperator, Predicate},
    witness_closed,
};
use std::path::Path;
const DIGEST: [u8; 32] = [7; 32];
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const ISSUERS: [&str; 6] = [
    "urn:fictional:insurance-quote:licensing",
    "urn:fictional:insurance-quote:claims-register",
    "urn:fictional:trial-eligibility:practice",
    "urn:fictional:recycled-content:tier",
    "urn:fictional:recycled-content:tier-2",
    "urn:fictional:recycled-content:tier-3",
];
const TARGETS: [&str; 6] = [
    "insurance-count",
    "insurance-closed",
    "trial-eligibility",
    "recycled-min",
    "recycled-sum",
    "recycled-having",
];
fn registry() -> Vec<(oxrdf::NamedNode, sparq_zk::sig::PublicKey)> {
    ISSUERS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                oxrdf::NamedNode::new_unchecked(*s),
                sparq_zk::sig::SecretKey::from_seed(19 + i as u64).public_key(),
            )
        })
        .collect()
}
fn issuer_root() -> [u8; 32] {
    sparq_zk::field::field_to_be_bytes_32(
        &sparq_zk_compose::issuer::key_set_root(
            &registry().into_iter().map(|(_, k)| k).collect::<Vec<_>>(),
            4,
        )
        .unwrap(),
    )
}
fn sign(mut source: String, issuer: usize, salt: u64) -> Credential {
    source += &format!(
        "<urn:credential> <{}> <{}> .\n",
        graphs::ISSUER,
        ISSUERS[issuer]
    );
    let key = sparq_zk::sig::SecretKey::from_seed(19 + issuer as u64);
    let credential = sparq_merkle_suite::poseidon::issue(
        &source,
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n",
        &key,
        sparq_merkle_suite::Fr::from(salt + 101),
    )
    .unwrap();
    Credential {
        credential,
        issuer: key.public_key(),
        issuer_index: issuer as u64,
        issuer_siblings: sparq_zk_compose::issuer::key_membership_witness(
            &registry().into_iter().map(|(_, k)| k).collect::<Vec<_>>(),
            4,
            issuer as u64,
        )
        .unwrap(),
    }
}
fn lit(v: impl std::fmt::Display, ty: &str) -> String {
    format!("\"{v}\"^^<{XSD}{ty}>")
}
fn fact(s: &str, p: &str, v: &str) -> String {
    format!("<{s}> <{p}> {v} .\n")
}
fn fixtures(target: &str, case: &str) -> Vec<Credential> {
    let owner = if target == "trial-eligibility" {
        "urn:fictional:trial-eligibility:owner"
    } else {
        "urn:fictional:insurance-quote:owner"
    };
    let period = |start: &str, end: &str| {
        fact(
            owner,
            "https://w3id.org/jeswr/awp#periodStart",
            &lit(start, "date"),
        ) + &fact(
            owner,
            "https://w3id.org/jeswr/awp#periodEnd",
            &lit(end, "date"),
        )
    };
    let data = if target.starts_with("insurance") {
        let years = fact(
            owner,
            "urn:claim:licence.years",
            &lit(if case == "years" { 4 } else { 12 }, "integer"),
        );
        let points = fact(
            owner,
            "urn:claim:licence.points",
            &lit(if case == "points" { 4 } else { 0 }, "integer"),
        );
        let mut lic = points;
        let mut reg = String::new();
        if case == "licence-other-issuer" {
            reg += &years;
        } else {
            lic += &years;
        }
        if target == "insurance-count" {
            let count = fact(
                owner,
                "urn:claim:claims.five-year-count",
                &lit(if case == "count" { 1 } else { 0 }, "integer"),
            );
            if case == "register-other-issuer" {
                lic += &count;
            } else {
                reg += &count;
            }
            reg += &fact(
                owner,
                "urn:claim:claims.five-year-complete",
                &lit(case != "complete", "boolean"),
            );
        } else {
            let dates = period(
                if case == "start" {
                    "2021-11-14"
                } else {
                    "2021-11-13"
                },
                if case == "end" {
                    "2026-11-12"
                } else {
                    "2026-11-13"
                },
            );
            if case == "register-other-issuer" {
                lic += &dates;
            } else {
                reg += &dates;
            }
            if case == "claim" || case == "old-claim" {
                reg += &fact(owner, "urn:claim:claims.claim", "<urn:claim:1>");
                reg += &fact(
                    "urn:claim:1",
                    "urn:claim:claims.date",
                    &lit(
                        if case == "old-claim" {
                            "2021-11-12"
                        } else {
                            "2024-03-02"
                        },
                        "date",
                    ),
                );
            }
        }
        vec![sign(lic, 0, 0), sign(reg, 1, 1)]
    } else if target == "trial-eligibility" {
        let mut data = fact(
            owner,
            "urn:claim:trial.age",
            &lit(
                if case == "young" {
                    17
                } else if case == "old" {
                    66
                } else {
                    45
                },
                "integer",
            ),
        );
        data += &fact(
            owner,
            "urn:claim:trial.diagnosis",
            if case == "diagnosis" {
                "\"condition-b\""
            } else {
                "\"condition-a\""
            },
        );
        data += &period(
            if case == "start" {
                "2026-11-02"
            } else {
                "2026-11-01"
            },
            if case == "end" {
                "2026-11-12"
            } else {
                "2026-11-13"
            },
        );
        data += &fact(
            owner,
            "urn:claim:trial.medications",
            if case == "excluded" {
                "\"excluded-x\""
            } else {
                "\"medicine-a\""
            },
        );
        if case == "medicines" {
            for i in 1..32 {
                data += &fact(
                    owner,
                    "urn:claim:trial.medications",
                    &format!("\"medicine-{i}\""),
                );
            }
        }
        vec![sign(data, 2, 0)]
    } else {
        let mut data = vec![];
        for i in 0..if case == "empty" {
            0
        } else if case == "two-suppliers" {
            2
        } else {
            3
        } {
            let mass = target == "recycled-sum" || target == "recycled-having";
            let mut rdf = fact(
                "urn:fictional:recycled-content:lot",
                if mass {
                    "urn:claim:material.recycled-mass"
                } else {
                    "urn:claim:material.recycled-fraction"
                },
                &lit(
                    if mass {
                        if case == "below" && i == 2 {
                            "110.0"
                        } else {
                            ["200.0", "180.0", "150.0"][i]
                        }
                    } else if case == "below" && i == 0 {
                        "0.29"
                    } else {
                        ["0.35", "0.40", "0.38"][i]
                    },
                    "decimal",
                ),
            );
            if !mass && case == "fourth-figure" && i == 0 {
                rdf += &fact(
                    "urn:fictional:recycled-content:lot",
                    "urn:claim:material.recycled-fraction",
                    &lit("0.36", "decimal"),
                );
            }
            data.push(sign(rdf, 3 + i, i as u64));
        }
        if case == "fourth-figure" && matches!(target, "recycled-sum" | "recycled-having") {
            data.push(sign(
                fact(
                    "urn:fictional:recycled-content:lot",
                    "urn:claim:material.recycled-mass",
                    &lit("50.0", "decimal"),
                ),
                3,
                13,
            ));
        }
        data
    };
    graphs::canonical_credentials(data).unwrap()
}
fn query(target: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/solid_paths/{target}.rq")),
    )
    .unwrap()
}
fn bounds() -> Bounds {
    Bounds {
        credentials: 4,
        quads_per_credential: 64,
        rows_per_table: 32,
    }
}
fn prepare(target: &str, full: bool) -> Plan {
    prepare_query(target, &query(target), full)
}
fn prepare_query(target: &str, text: &str, full: bool) -> Plan {
    let request = if full {
        bounds()
    } else {
        Bounds {
            credentials: if target.starts_with("insurance-") {
                2
            } else if target == "trial-eligibility" {
                1
            } else {
                4
            },
            quads_per_credential: 8,
            rows_per_table: 32,
        }
    };
    // The receiver chooses these table buckets from the public query/profile.
    // Lower at the outer-row bound so EXISTS emits exactly its capture slots.
    let small = if full || target.starts_with("recycled-") {
        4
    } else {
        2
    };
    let p = graphs::lower(
        text,
        Bounds {
            rows_per_table: small,
            ..request
        },
    )
    .unwrap();
    let rows =
        p.nodes()
            .iter()
            .map(|node| {
                if full
                    && node.output.variables.iter().any(|v| {
                        matches!(p.variable_names()[*v as usize].as_str(), "c" | "d" | "m")
                    })
                {
                    32
                } else {
                    small
                }
            })
            .collect::<Vec<_>>();
    let p = sparq_zk_query::plan_closed::with_row_bounds(p, request, &rows).unwrap();
    let p = graphs::bind_issuer_registry(p, &registry()).unwrap();
    if full {
        p
    } else {
        let quad_bounds = if target == "insurance-closed" {
            vec![8, 8]
        } else if target == "trial-eligibility" {
            vec![8]
        } else {
            vec![4; p.capacity().credentials as usize]
        };
        p.with_credential_quad_bounds(&quad_bounds).unwrap()
    }
}

fn cases(target: &str) -> Vec<(&str, Option<bool>)> {
    let mut out = vec![("honest", Some(true))];
    let names: &[&str] = match target {
        "insurance-count" => &["years", "points", "count", "complete"],
        "insurance-closed" => &["years", "points", "start", "end", "claim"],
        "trial-eligibility" => &["young", "old", "diagnosis", "start", "end", "excluded"],
        _ => &["below", "two-suppliers", "fourth-figure", "empty"],
    };
    out.extend(names.iter().map(|s| (*s, Some(false))));
    if target.starts_with("insurance") {
        out.extend([
            ("licence-other-issuer", None),
            ("register-other-issuer", None),
        ]);
    }
    if target == "insurance-closed" {
        out.push(("old-claim", Some(true)));
    }
    out
}
fn released_cell(term: oxrdf::Term) -> sparq_zk_query::interp::Cell {
    let ResultValue::Select { rows, .. } = (Answer::Select {
        variables: vec!["result".into()],
        rows: vec![vec![Some(term)]],
    })
    .encode(&SuiteEncoder)
    .unwrap() else {
        unreachable!("SELECT answer")
    };
    rows[0][0]
}
fn released(result: &ResultValue) -> Option<bool> {
    let ResultValue::Select { variables, rows } = result else {
        panic!("SELECT result")
    };
    assert_eq!(variables, &["result"]);
    if rows.is_empty() {
        return None;
    }
    assert_eq!(rows.len(), 1);
    for value in [false, true] {
        let expected = released_cell(oxrdf::Literal::from(value).into());
        if rows[0][0] == expected {
            return Some(value);
        }
    }
    panic!("noncanonical released boolean: {rows:?}")
}
fn released_for(target: &str, result: &ResultValue) -> Option<bool> {
    if target != "recycled-having" {
        return released(result);
    }
    let ResultValue::Select { variables, rows } = result else {
        panic!("SELECT result");
    };
    assert_eq!(variables, &["lot"]);
    if rows.is_empty() {
        return Some(false);
    }
    assert_eq!(
        rows,
        &vec![vec![released_cell(
            oxrdf::NamedNode::new_unchecked("urn:fictional:recycled-content:lot").into()
        )]]
    );
    Some(true)
}
fn alter_result(target: &str, trace: &mut Trace) {
    let ResultValue::Select { rows, .. } = &mut trace.result else {
        unreachable!()
    };
    if target == "recycled-having" {
        if rows.is_empty() {
            rows.push(vec![released_cell(
                oxrdf::NamedNode::new_unchecked("urn:fictional:recycled-content:lot").into(),
            )]);
        } else {
            rows.clear();
        }
    } else {
        let truth = released_cell(oxrdf::Literal::from(true).into());
        if rows.is_empty() {
            rows.push(vec![truth]);
        } else {
            let value = rows[0][0] == truth;
            rows[0][0] = released_cell(oxrdf::Literal::from(!value).into());
        }
    }
}
fn engine(target: &str, data: &[Credential]) -> Option<bool> {
    let mut rdf = String::new();
    for (i, c) in data.iter().enumerate() {
        for q in oxttl::NQuadsParser::new().for_slice(c.credential.document.as_bytes()) {
            let q = q.unwrap();
            rdf += &format!(
                "{} {} {} .\n{} {} {} _:g{i} .\n",
                q.subject, q.predicate, q.object, q.subject, q.predicate, q.object
            );
            if q.predicate.as_str() == graphs::ISSUER {
                rdf += &format!("_:g{i} <{}> {} .\n", graphs::VERIFIED_ISSUER, q.object);
            }
        }
    }
    let graph = sparq_core::Graph::load_dataset(&rdf, "nquads").unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&sparq_engine::query_json(&graph, &query(target)).unwrap()).unwrap();
    let rows = result["results"]["bindings"].as_array().unwrap();
    if target == "recycled-having" {
        if !rows.is_empty() {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["lot"]["type"], "uri");
            assert_eq!(
                rows[0]["lot"]["value"],
                "urn:fictional:recycled-content:lot"
            );
        }
        Some(!rows.is_empty())
    } else if rows.is_empty() {
        None
    } else {
        assert_eq!(rows.len(), 1);
        Some(rows[0]["result"]["value"] == "true")
    }
}
#[test]
fn target_queries_match_engine_and_complete_interpreter() {
    for target in TARGETS {
        let p = prepare(target, true);
        for (case, expected) in cases(target) {
            let data = fixtures(target, case);
            let trace = graphs::evaluate_for_request(&p, &data, DIGEST)
                .unwrap_or_else(|e| panic!("{target}/{case}: {e}"));
            assert_eq!(
                released_for(target, &trace.result),
                expected,
                "{target}/{case}"
            );
            assert_eq!(engine(target, &data), expected, "engine {target}/{case}");
            let mut reverse = data.clone();
            reverse.reverse();
            assert_eq!(
                graphs::cells(&graphs::canonical_credentials(reverse).unwrap()).unwrap(),
                graphs::cells(&data).unwrap()
            );
        }
    }
}
#[test]
fn public_constants_and_membership_length_do_not_change_shape_or_source() {
    for target in TARGETS {
        let text = query(target);
        let changed = text
            .replace("urn:claim:", "urn:public:")
            .replace("https://w3id.org/jeswr/awp#", "urn:period:")
            .replace(" = 3", " = 4")
            .replace("urn:fictional:", "urn:receiver:")
            .replace("2021-11-13", "2020-10-12")
            .replace("2026-11-13", "2025-10-12")
            .replace(">= 5", ">= 6")
            .replace("<= 3", "<= 2")
            .replace("0.3", "0.4")
            .replace("condition-a", "condition-b")
            .replace("excluded-x", "excluded-a")
            .replace(", \"excluded-y\"", "")
            .replace(
                ",\n                  <urn:receiver:recycled-content:tier-3>",
                "",
            );
        let a = graphs::lower(&text, bounds()).unwrap();
        let b = graphs::lower(&changed, bounds()).unwrap();
        assert_eq!(a.hash(), b.hash(), "{target}");
        let aa = graphs::bind_issuer_registry(a.clone(), &registry()).unwrap();
        let bb = graphs::bind_issuer_registry(b.clone(), &registry()).unwrap();
        assert_eq!(
            codegen::generate(&aa).unwrap(),
            codegen::generate(&bb).unwrap(),
            "authenticated {target}"
        );
        assert_eq!(
            codegen::generate_operator_fixture(&a).unwrap(),
            codegen::generate_operator_fixture(&b).unwrap(),
            "{target}"
        );
    }
}
fn write_package(package: &codegen::Package, root: &Path) {
    for (name, source) in &package.files {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    if root.join("zk").exists() {
        return;
    }
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk")
            .canonicalize()
            .unwrap(),
        root.join("zk"),
    )
    .unwrap();
}
fn execute(
    gate: &support::LiveToolchain,
    root: &Path,
    name: &str,
    input: &str,
    tag: &str,
) -> Result<(), String> {
    std::fs::write(root.join(name).join(format!("Prover_{tag}.toml")), input).unwrap();
    let witness = root.join("target").join(format!("{tag}.gz"));
    if witness.exists() {
        std::fs::remove_file(&witness).unwrap();
    }
    let output = support::live_tool(gate, "nargo")
        .args([
            "execute",
            "--inliner-aggressiveness",
            "0",
            "--package",
            name,
            "--prover-name",
            &format!("Prover_{tag}"),
            tag,
        ])
        .current_dir(root)
        .output()
        .unwrap();
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
            "{}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
fn render(p: &Plan, data: &[Credential], trace: &Trace, auth: bool) -> String {
    if auth {
        witness_closed::prover_toml(p, data, trace, DIGEST, [3; 32], issuer_root()).unwrap()
    } else {
        witness_closed::operator_prover_toml(
            p,
            &graphs::cells(data).unwrap(),
            &SuiteEncoder,
            trace,
            DIGEST,
            [3; 32],
        )
        .unwrap()
    }
}
fn rights(p: &Predicate, ids: &mut Vec<u32>) {
    match p {
        Predicate::Exists(x) => ids.extend(x),
        Predicate::Not(x) => rights(x, ids),
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            rights(a, ids);
            rights(b, ids);
        }
        _ => (),
    }
}
fn exercise(gate: &support::LiveToolchain, target: &str, auth: bool) {
    let p = prepare(target, false);
    let package = if auth {
        codegen::generate(&p)
    } else {
        codegen::generate_operator_fixture(&p)
    }
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let root = std::env::var_os("SPARQ_CLOSED_NATIVE_DIR")
        .map(|root| {
            Path::new(&root).join(format!(
                "{target}-{}",
                if auth { "authenticated" } else { "operators" }
            ))
        })
        .unwrap_or_else(|| scratch.path().to_path_buf());
    std::fs::create_dir_all(&root).unwrap();
    write_package(&package, &root);
    let compiled = support::live_tool(gate, "nargo")
        .args([
            "compile",
            "--inliner-aggressiveness",
            "0",
            "--package",
            &package.name,
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{target} compilation {}: {}{}",
        compiled.status,
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(root
        .join("target")
        .join(format!("{}.json", package.name))
        .is_file());
    for (case, expected) in cases(target) {
        let data = fixtures(target, case);
        let trace = graphs::evaluate_for_request(&p, &data, DIGEST).unwrap();
        assert_eq!(released_for(target, &trace.result), expected);
        assert_eq!(engine(target, &data), expected);
        eprintln!("executing {target}/{case} (authenticated={auth})");
        execute(
            gate,
            &root,
            &package.name,
            &render(&p, &data, &trace, auth),
            "honest",
        )
        .unwrap_or_else(|e| panic!("{target}/{case}: {e}"));
        {
            let mut altered = trace.clone();
            alter_result(target, &mut altered);
            assert_ne!(altered.result, trace.result, "{target}/{case}");
            assert!(
                execute(
                    gate,
                    &root,
                    &package.name,
                    &render(&p, &data, &altered, auth),
                    "wrong_result"
                )
                .is_err(),
                "{target}/{case}"
            );
            let source = p
                .nodes()
                .iter()
                .enumerate()
                .find(|(i, n)| {
                    matches!(
                        n.operator,
                        Operator::Closed(ClosedOperator::GraphScan { .. })
                    ) && !trace.tables[*i].rows.is_empty()
                })
                .map(|(i, _)| i)
                .or_else(|| trace.tables.iter().position(|t| !t.rows.is_empty()))
                .expect("an empty global group still has a checked producer");
            let mut hidden = trace.clone();
            hidden.tables[source].rows.pop();
            let error = execute(
                gate,
                &root,
                &package.name,
                &render(&p, &data, &hidden, auth),
                "hidden_row",
            )
            .unwrap_err();
            assert!(
                error.contains("operator length") || error.contains("operator rows"),
                "{error}"
            );
            let mut ids = vec![];
            for n in p.nodes() {
                if let Operator::Closed(
                    ClosedOperator::BindBoolean { predicate, .. }
                    | ClosedOperator::Filter { predicate, .. },
                ) = &n.operator
                {
                    rights(predicate, &mut ids);
                }
            }
            if let Some(id) = ids
                .into_iter()
                .find(|id| !trace.tables[*id as usize].rows.is_empty())
            {
                let mut hidden = trace.clone();
                hidden.tables[id as usize].rows.pop();
                let error = execute(
                    gate,
                    &root,
                    &package.name,
                    &render(&p, &data, &hidden, auth),
                    "hidden_matching_right",
                )
                .unwrap_err();
                assert!(
                    error.contains("operator length") || error.contains("operator rows"),
                    "{error}"
                );
            }
        }
    }
}
#[test]
fn insurance_count_operator() {
    let Some(gate) = support::gate("insurance_count_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "insurance-count", false);
    support::complete(gate);
}
#[test]
fn insurance_closed_operator() {
    let Some(gate) = support::gate("insurance_closed_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "insurance-closed", false);
    support::complete(gate);
}
#[test]
fn trial_eligibility_operator() {
    let Some(gate) = support::gate("trial_eligibility_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "trial-eligibility", false);
    support::complete(gate);
}
#[test]
fn recycled_min_operator() {
    let Some(gate) = support::gate("recycled_min_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-min", false);
    support::complete(gate);
}
#[test]
fn insurance_count_authenticated() {
    let Some(gate) = support::gate("insurance_count_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "insurance-count", true);
    support::complete(gate);
}
#[test]
fn insurance_closed_authenticated() {
    let Some(gate) = support::gate("insurance_closed_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "insurance-closed", true);
    support::complete(gate);
}
#[test]
fn trial_eligibility_authenticated() {
    let Some(gate) = support::gate("trial_eligibility_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "trial-eligibility", true);
    support::complete(gate);
}
#[test]
fn recycled_min_authenticated() {
    let Some(gate) = support::gate("recycled_min_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-min", true);
    support::complete(gate);
}
#[test]
fn recycled_sum_operator() {
    let Some(gate) = support::gate("recycled_sum_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-sum", false);
    support::complete(gate);
}
#[test]
fn recycled_having_operator() {
    let Some(gate) = support::gate("recycled_having_operator", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-having", false);
    support::complete(gate);
}
#[test]
fn recycled_sum_authenticated() {
    let Some(gate) = support::gate("recycled_sum_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-sum", true);
    support::complete(gate);
}
#[test]
fn recycled_having_authenticated() {
    let Some(gate) = support::gate("recycled_having_authenticated", &["nargo"]) else {
        return;
    };
    exercise(&gate, "recycled-having", true);
    support::complete(gate);
}
#[test]
fn export_solid_path_programs_when_requested() {
    if let Ok(root) = std::env::var("SPARQ_CLOSED_EXPORT_DIR") {
        for target in TARGETS {
            for auth in [false, true] {
                let p = prepare(
                    target,
                    std::env::var_os("SPARQ_CLOSED_EXPORT_SMALL").is_none(),
                );
                let data = fixtures(target, "honest");
                let trace = graphs::evaluate_for_request(&p, &data, DIGEST).unwrap();
                let pkg = if auth {
                    codegen::generate(&p)
                } else {
                    codegen::generate_operator_fixture(&p)
                }
                .unwrap();
                let root = Path::new(&root).join(format!(
                    "{target}-{}",
                    if auth { "authenticated" } else { "operators" }
                ));
                std::fs::create_dir_all(&root).unwrap();
                write_package(&pkg, &root);
                std::fs::write(root.join("package-name"), &pkg.name).unwrap();
                std::fs::write(root.join("row-buckets.json"),serde_json::to_vec_pretty(&p.nodes().iter()
                    .enumerate().map(|(i,n)| serde_json::json!({"node":i,"rows":n.output.capacity,
                        "variables":n.output.variables.iter().map(|v| &p.variable_names()[*v as usize]).collect::<Vec<_>>()})).collect::<Vec<_>>()).unwrap()).unwrap();
                std::fs::write(
                    root.join(&pkg.name).join("Prover.toml"),
                    render(&p, &data, &trace, auth),
                )
                .unwrap();
                let honest = render(&p, &data, &trace, auth);
                let line = honest
                    .lines()
                    .find(|line| line.starts_with("counts = "))
                    .unwrap();
                let mut counts = data.iter().map(|c| c.credential.count).collect::<Vec<_>>();
                counts.resize(4, 0);
                counts[0] = 65;
                let over = honest.replace(line, &format!("counts = {counts:?}"));
                std::fs::write(root.join(&pkg.name).join("Prover_over_capacity.toml"), over)
                    .unwrap();
                let mut wrong = trace.clone();
                alter_result(target, &mut wrong);
                std::fs::write(
                    root.join(&pkg.name).join("Prover_wrong.toml"),
                    render(&p, &data, &wrong, auth),
                )
                .unwrap();
            }
        }
    }
}
#[test]
fn target_capacity_and_reserved_issuer_controls() {
    for target in TARGETS {
        let p = prepare(target, true);
        assert_eq!(p.capacity().credentials, 4);
        assert_eq!(p.credential_quad_capacities(), vec![64; 4]);
        assert_eq!(p.capacity().rows_per_table, 4);
        assert_eq!(p.bounds().rows_per_table, 32);
        assert!(p.nodes().iter().all(|n| matches!(n.output.capacity,4|32)));
        assert!(p.nodes().iter().filter(|n| n.output.capacity==32).all(|n|
            n.output.variables.iter().any(|v| matches!(p.variable_names()[*v as usize].as_str(),"c"|"d"|"m"))));
    }
    let mut reversed = fixtures("recycled-sum", "honest");
    reversed.reverse();
    assert!(
        graphs::evaluate_for_request(&prepare("recycled-sum", true), &reversed, DIGEST)
            .unwrap_err()
            .to_string()
            .contains("canonical credential order")
    );
    let original = query("trial-eligibility");
    let members = (0..16)
        .map(|i| format!("\"excluded-{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let text = original.replace("\"excluded-x\", \"excluded-y\"", &members);
    let list_plan = prepare_query("trial-eligibility", &text, true);
    assert_eq!(list_plan.hash(), prepare("trial-eligibility", true).hash());
    let medicines = fixtures("trial-eligibility", "medicines");
    assert_eq!(
        released(
            &graphs::evaluate_for_request(&list_plan, &medicines, DIGEST)
                .unwrap()
                .result
        ),
        Some(true)
    );
    let p = prepare("trial-eligibility", true);
    let data = fixtures("trial-eligibility", "medicines");
    assert_eq!(
        released(
            &graphs::evaluate_for_request(&p, &data, DIGEST)
                .unwrap()
                .result
        ),
        Some(true)
    );
    let too_many = vec![medicines[0].clone(); 5];
    assert!(matches!(
        graphs::evaluate_for_request(&p, &too_many, DIGEST),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
    let mut source = String::new();
    for i in 0..64 {
        source += &fact("urn:subject", &format!("urn:extra:{i}"), &lit(i, "integer"));
    }
    let too_wide = vec![sign(source, 2, 12)];
    assert!(matches!(
        graphs::evaluate_for_request(&p, &too_wide, DIGEST),
        Err(sparq_zk_query::Error::Capacity(_))
    ));
    let injected = vec![sign(
        fact("urn:s", graphs::VERIFIED_ISSUER, "<urn:evil>"),
        2,
        10,
    )];
    assert!(graphs::cells(&injected)
        .unwrap_err()
        .to_string()
        .contains("verifiedIssuer"));
}

fn required_value_read(
    target: &str,
    p: &Plan,
    trace: &Trace,
) -> (usize, u32, sparq_zk_query::interp::Cell) {
    if target == "insurance-closed" {
        let d = p.variable_names().iter().position(|v| v == "d").unwrap() as u32;
        p.nodes()
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                let Operator::Closed(ClosedOperator::Filter { input, .. }) = &n.operator else {
                    return None;
                };
                let col = p.nodes()[*input as usize]
                    .output
                    .variables
                    .iter()
                    .position(|v| *v == d)?;
                let row = trace.tables[*input as usize].rows.first()?;
                // This real claim is rejected by the date FILTER in NOT EXISTS.
                assert!(trace.tables[i].rows.is_empty());
                Some((i, *input, row[col]))
            })
            .expect("a non-output claim date read")
    } else {
        p.nodes()
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                let Operator::Closed(ClosedOperator::Group {
                    input, aggregates, ..
                }) = &n.operator
                else {
                    return None;
                };
                let a = aggregates
                    .iter()
                    .find(|a| a.kind == sparq_zk_query::interp_closed_group::AggregateKind::Min)?;
                let col = p.nodes()[*input as usize]
                    .output
                    .variables
                    .iter()
                    .position(|v| Some(*v) == a.operand)?;
                let rows = &trace.tables[*input as usize].rows;
                let smallest = rows.iter().map(|r| r[col].key).min()?;
                let largest = rows.iter().map(|r| r[col]).max_by_key(|c| c.key)?;
                assert!(largest.key > smallest);
                Some((i, *input, largest))
            })
            .expect("a non-minimum aggregate operand read")
    }
}

#[test]
fn required_closed_value_reads_fail_at_actual_query_sites() {
    let Some(gate) = support::gate("closed target rejected-row value reads", &["nargo"]) else {
        return;
    };
    for (target, case) in [
        ("insurance-closed", "old-claim"),
        ("recycled-min", "honest"),
    ] {
        let p = prepare(target, false);
        let data = fixtures(target, case);
        let trace = graphs::evaluate_for_request(&p, &data, DIGEST).unwrap();
        let package = codegen::generate(&p).unwrap();
        let workspace = tempfile::tempdir().unwrap();
        write_package(&package, workspace.path());
        let honest = render(&p, &data, &trace, true);
        execute(&gate, workspace.path(), &package.name, &honest, "reads_honest")
            .unwrap_or_else(|e| panic!("{target}: {e}"));
        let (node, input, cell) = required_value_read(target, &p, &trace);
        let key = cell.key[1..].to_vec();
        let marker = format!(
            "key=[{}]",
            key.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
        );
        let mut forged = key;
        let payload = if target == "insurance-closed" { 8 } else { 30 };
        forged[payload] ^= 1;
        let changed_marker = format!(
            "key=[{}]",
            forged
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        let prefix = format!("closed_reads_{node}_{input} = ");
        let line = honest
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap();
        assert_eq!(line.matches(&marker).count(), 1);
        let changed_line = line.replacen(&marker, &changed_marker, 1);
        let changed = honest.replacen(line, &changed_line, 1);
        let error = execute(
            &gate,
            workspace.path(),
            &package.name,
            &changed,
            "forged_rejected_value_key",
        )
        .unwrap_err();
        assert!(
            error.contains("key disagrees with lexical form"),
            "{target}: {error}"
        );
    }
    support::complete(gate);
}

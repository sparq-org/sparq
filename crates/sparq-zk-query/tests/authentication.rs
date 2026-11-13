#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/mod.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

use serde_json::{json, Value};
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    credential::SuiteEncoder,
    interp::{evaluate, CredentialCells},
    lower::lower,
    statement::{Layout, Statement},
    witness::{prover_toml, prover_toml_authenticated},
};

// The renderer uses only inline objects, arrays and JSON-compatible scalars.
// Preserve strings while converting its bare field names into JSON object keys.
fn parse(input: &str) -> serde_json::Map<String, Value> {
    input
        .lines()
        .map(|line| {
            let (name, value) = line.split_once(" = ").unwrap();
            let mut out = String::new();
            let bytes = value.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    let start = i;
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' {
                            i += 2;
                        } else if bytes[i] == b'"' {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    out.push_str(&value[start..i]);
                } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                    let start = i;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    let word = &value[start..i];
                    if word == "true" || word == "false" {
                        out.push_str(word);
                    } else {
                        out.push('"');
                        out.push_str(word);
                        out.push('"');
                    }
                } else {
                    out.push(if bytes[i] == b'=' {
                        ':'
                    } else {
                        bytes[i] as char
                    });
                    i += 1;
                }
            }
            (name.to_owned(), serde_json::from_str(&out).unwrap())
        })
        .collect()
}
fn value_toml(v: &Value) -> String {
    match v {
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(value_toml).collect::<Vec<_>>().join(",")
        ),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{k}={}", value_toml(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => v.to_string(),
    }
}
fn render(input: &serde_json::Map<String, Value>) -> String {
    input
        .iter()
        .map(|(k, v)| format!("{k}={}\n", value_toml(v)))
        .collect()
}
fn change_field(value: &mut Value) {
    if let Some(text) = value.as_str() {
        let mut bytes = text.as_bytes().to_vec();
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'1' { b'2' } else { b'1' };
        *value = json!(String::from_utf8(bytes).unwrap());
    } else {
        *value = json!(value.as_u64().unwrap() + 1);
    }
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn suite_program_checks_authentication_membership_keys_and_credential_limits() {
    let Some(gate) = support::gate(
        "suite_program_checks_authentication_membership_keys_and_credential_limits",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let query = "ASK {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 2)}";
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 4,
        rows_per_table: 4,
    };
    let plan = lower(query, bounds).unwrap();
    let data = authenticated::fixture();
    let cells = vec![data[0].cells(0).unwrap()];
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    let input = prover_toml_authenticated(
        &plan,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        authenticated::issuer_root(),
    )
    .unwrap();
    let parsed = parse(&input);
    let package = sparq_zk_query::codegen::generate(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let artifact = driver.compile().unwrap();
    Layout::new(&plan)
        .audit_authenticated_artifact(
            &serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap(),
        )
        .unwrap();
    driver.execute(&render(&parsed), "suite_honest").unwrap();
    for field in ["s", "e", "e_k", "salt", "issuer_index"] {
        let mut changed = parsed.clone();
        change_field(&mut changed["credential_proof_0"][field]);
        assert!(
            driver
                .execute(&render(&changed), "altered_authentication")
                .is_err(),
            "{field}"
        );
    }
    for field in ["salts", "issuer_siblings", "config"] {
        let mut changed = parsed.clone();
        change_field(&mut changed["credential_proof_0"][field][0]);
        assert!(
            driver
                .execute(&render(&changed), "altered_membership")
                .is_err(),
            "{field}"
        );
    }
    for field in ["pk", "r"] {
        let mut changed = parsed.clone();
        change_field(&mut changed["credential_proof_0"][field]["x"]);
        assert!(
            driver
                .execute(&render(&changed), "altered_signature_point")
                .is_err(),
            "{field}"
        );
    }
    let mut changed = parsed.clone();
    changed["credential_0"][0][0]["lexical"][0] = json!(1);
    assert!(driver
        .execute(&render(&changed), "altered_lexical")
        .is_err());
    let mut changed = parsed.clone();
    let padding = data[0].credential.count as usize;
    changed["credential_0"][padding][0]["lexical"][0] = json!(1);
    assert!(driver
        .execute(&render(&changed), "altered_opening_padding")
        .is_err());
    let mut changed = parsed.clone();
    changed["counts"][0] = json!(1);
    assert!(driver
        .execute(&render(&changed), "omitted_signed_quad")
        .is_err());
    let narrow = plan.clone().with_credential_quad_bounds(&[1]).unwrap();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: sparq_zk_query::credential::dataset_commitment(&data, [3; 32]),
        signature_mode: 1,
        result: trace.result.clone(),
    };
    let words = Layout::new(&narrow)
        .reconstruct_authenticated(&narrow, &statement, authenticated::issuer_root())
        .unwrap();
    let mut changed = parsed.clone();
    changed["credential_quad_bounds"] = json!([1]);
    changed["statement_binding"] = json!(sparq_zk_query::statement::hex(*words.last().unwrap()));
    assert!(driver
        .execute(&render(&changed), "credential_limit_exceeded")
        .is_err());
    let forged = incorrect_key_signed_witness(&plan, &data[0], &parsed, &trace.result);
    assert!(driver
        .execute(&render(&forged), "issuer_key_disagrees_with_lexical_form")
        .is_err());
    support::complete(gate);
}

fn incorrect_key_signed_witness(
    plan: &sparq_zk_query::plan::Plan,
    credential: &sparq_zk_query::credential::Credential,
    honest: &serde_json::Map<String, Value>,
    original: &sparq_zk_query::interp::ResultValue,
) -> serde_json::Map<String, Value> {
    use sha2::{Digest, Sha256};
    use sparq_merkle_suite::poseidon as suite;
    use sparq_zk::{
        field::{field_from_hex_str_strict, field_to_hex},
        poseidon2, sig,
    };
    let mut rows = vec![];
    let mut changed = honest.clone();
    for (i, quad) in oxttl::NQuadsParser::new()
        .for_slice(credential.credential.document.as_bytes())
        .enumerate()
    {
        let quad = quad.unwrap();
        let mut cells = suite::quad_cells(&quad, 1).unwrap();
        let mut raw = changed["credential_0"][i].clone();
        if matches!(&quad.object,oxrdf::Term::Literal(l) if l.value()=="1") {
            let literal = oxrdf::Literal::new_typed_literal(
                "0",
                oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            );
            let key = sparq_merkle_suite::comparison_key(&literal);
            let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
            cells[2].key = field_from_hex_str_strict(&key_hex).unwrap();
            cells[2].enc = poseidon2::hash(&[
                sparq_merkle_suite::Fr::from(3u64),
                suite::hash_bytes(suite::TERM_DOMAIN, &cells[2].bytes),
                cells[2].key,
            ]);
            raw[2]["key"] = json!(key.as_slice());
        }
        let hash = suite::hash_quad(cells.clone().map(|c| c.enc));
        rows.push((suite::field_bytes(hash), hash, cells, raw));
    }
    rows.sort_by_key(|r| r.0);
    let hash_root = suite::root(
        &rows.iter().map(|r| r.1).collect::<Vec<_>>(),
        credential.credential.salt,
    )
    .unwrap();
    let config: [u8; 32] = Sha256::digest(credential.credential.proof_config.as_bytes()).into();
    let message = suite::signed_message(
        credential.credential.salt,
        credential.credential.count,
        hash_root,
        &config,
    );
    let key = sig::SecretKey::from_seed(19);
    let signature = sig::sign_deterministic(&key, &message);
    assert!(sig::verify(&key.public_key(), &message, &signature));
    let w = sig::in_circuit_witness(&key.public_key(), &message, &signature).unwrap();
    for (field, v) in [("s", w.s), ("e", w.e), ("e_k", w.e_k)] {
        changed["credential_proof_0"][field] = json!(field_to_hex(&v));
    }
    changed["credential_proof_0"]["r"] = json!({"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)});
    let mut operator_rows = vec![];
    for (i, (_, _, cells, raw)) in rows.into_iter().enumerate() {
        changed["credential_0"][i] = raw;
        let convert = |c: suite::TermCell| sparq_zk_query::interp::Cell {
            bound: c.bound,
            kind: c.kind,
            enc: suite::field_bytes(c.enc),
            key_class: c.key_class,
            key: suite::field_bytes(c.key),
            dt: suite::field_bytes(c.dt),
            lang: suite::field_bytes(c.lang),
            scope: c.credential,
        };
        operator_rows.push([
            convert(cells[0].clone()),
            convert(cells[1].clone()),
            convert(cells[2].clone()),
            Default::default(),
        ]);
    }
    let bad_cells = vec![CredentialCells {
        quads: operator_rows,
    }];
    let trace = evaluate(plan, &bad_cells, &SuiteEncoder).unwrap();
    assert_ne!(&trace.result, original);
    let raw =
        parse(&prover_toml(plan, &bad_cells, &SuiteEncoder, &trace, [7; 32], [3; 32]).unwrap());
    for (key, value) in raw {
        if key == "result"
            || key
                .strip_prefix('t')
                .is_some_and(|n| n.parse::<usize>().is_ok())
        {
            changed.insert(key, value);
        }
    }
    let mut commitment = b"NQ/SUITE/V1\0\0\0\0\0".to_vec();
    commitment.extend([3; 32]);
    commitment.extend(suite::field_bytes(message));
    let commitment: [u8; 32] = Sha256::digest(commitment).into();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: commitment,
        signature_mode: 1,
        result: trace.result,
    };
    let words = Layout::new(plan)
        .reconstruct_authenticated(plan, &statement, authenticated::issuer_root())
        .unwrap();
    changed["input_commitment"] = json!(commitment);
    changed["statement_binding"] = json!(sparq_zk_query::statement::hex(*words.last().unwrap()));
    changed
}

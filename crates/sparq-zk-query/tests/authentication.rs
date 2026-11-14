#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/fixture.rs"]
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
    witness::{prover_toml, prover_toml_suite_fixture as prover_toml_authenticated},
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
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
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
            "altered authentication field accepted: {field}"
        );
    }
    for field in ["salts", "issuer_siblings", "config"] {
        let mut changed = parsed.clone();
        change_field(&mut changed["credential_proof_0"][field][0]);
        assert!(
            driver
                .execute(&render(&changed), "altered_membership")
                .is_err(),
            "altered membership field accepted: {field}"
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
    changed["credential_0"][0]["subject"]["lexical"][0] = json!(1);
    assert!(driver
        .execute(&render(&changed), "altered_lexical")
        .is_err());
    let mut changed = parsed.clone();
    let padding = data[0].credential.count as usize;
    changed["credential_0"][padding]["subject"]["lexical"][0] = json!(1);
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
        input_commitment: sparq_zk_query::credential::dataset_commitment(&data, [3; 32]).unwrap(),
        signature_mode: 1,
        result: trace.result.clone(),
    };
    let words = Layout::new(&narrow)
        .reconstruct_authenticated(&narrow, &statement, authenticated::issuer_root())
        .unwrap();
    let mut changed = parsed.clone();
    changed["credential_quad_bounds"] = json!([1]);
    changed["statement_binding"] = json!(sparq_zk_query::statement::hex(*words.last().unwrap()));
    assert!(
        driver
            .execute(&render(&changed), "credential_limit_exceeded")
            .is_err(),
        "altered credential limit accepted"
    );
    let forged = incorrect_key_signed_witness(&plan, &data[0], &parsed, &trace.result);
    assert!(
        driver
            .execute(&render(&forged), "issuer_key_disagrees_with_lexical_form")
            .is_err(),
        "altered signed value key accepted"
    );
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn suite_constant_scan_preserves_literal_identity() {
    let Some(gate) = support::gate(
        "suite_constant_scan_preserves_literal_identity",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "ASK {?s <urn:p> 1}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let data = authenticated::fixture();
    let cells = vec![data[0].cells(0).unwrap()];
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    assert_eq!(trace.result, sparq_zk_query::interp::ResultValue::Ask(true));
    let honest = parse(
        &prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap(),
    );
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    driver.compile().unwrap();
    driver
        .execute(&render(&honest), "constant_scan_honest")
        .unwrap();
    let altered = incorrect_key_signed_witness(&plan, &data[0], &honest, &trace.result);
    fixture_support::assert_engine(
        "ASK {?s <urn:p> 1}",
        &data[0].credential.document,
        &sparq_zk_query::interp::ResultValue::Ask(true),
    );
    assert_eq!(altered["result"], json!(true));
    driver
        .execute(&render(&altered), "constant_scan_identity_statement")
        .unwrap();
    // A released literal is RDF identity, without an unused value-key claim.
    let plan = lower(
        "SELECT ?x WHERE {?s <urn:p> ?x}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    let honest = parse(
        &prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap(),
    );
    let altered = incorrect_key_signed_witness(&plan, &data[0], &honest, &trace.result);
    assert_eq!(altered["result"], honest["result"]);
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    driver.compile().unwrap();
    driver
        .execute(&render(&honest), "released_literal_honest")
        .unwrap();
    driver
        .execute(&render(&altered), "released_literal_identity")
        .unwrap();
    let plan = lower(
        "ASK {?s <urn:p> ?x FILTER(?x IN (\"alpha\",\"other\"))}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let data = vec![authenticated::sign("<urn:s> <urn:p> \"alpha\" .", 0)];
    let cells = vec![data[0].cells(0).unwrap()];
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    assert_eq!(trace.result, sparq_zk_query::interp::ResultValue::Ask(true));
    let honest = parse(
        &prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap(),
    );
    let altered = signed_key_witness(
        &plan,
        &data[0],
        &honest,
        "alpha",
        oxrdf::Literal::new_simple_literal("beta"),
    );
    assert_eq!(altered["result"], json!(true));
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    driver.compile().unwrap();
    driver
        .execute(&render(&honest), "plain_string_membership_honest")
        .unwrap();
    driver
        .execute(&render(&altered), "plain_string_membership_identity")
        .unwrap();
    long_object_profile_executes_a_suite_multikey_opening();
    support::complete(gate);
}

fn long_object_profile_executes_a_suite_multikey_opening() {
    use sparq_zk_query::capacity::OpeningProfile;
    let lexical = sparq_merkle_suite::metadata::multikey(
        &sparq_zk::sig::SecretKey::from_seed(23).public_key(),
    )
    .unwrap();
    let data = vec![authenticated::sign(
        &format!("<urn:s> <urn:key> \"{lexical}\" ."),
        0,
    )];
    let compact = lower(
        "ASK {?s <urn:key> ?key}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let plan = compact
        .clone()
        .with_opening_profile(OpeningProfile::LongObjects);
    let cells = vec![data[0]
        .cells_with_profile(0, OpeningProfile::LongObjects)
        .unwrap()];
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    assert!(prover_toml_authenticated(
        &compact,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        authenticated::issuer_root()
    )
    .is_err());
    let input = prover_toml_authenticated(
        &plan,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        authenticated::issuer_root(),
    )
    .unwrap();
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let artifact = driver.compile().unwrap();
    Layout::new(&plan)
        .audit_authenticated_artifact(
            &serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap(),
        )
        .unwrap();
    driver
        .execute(&input, "long_object_multikey_honest")
        .unwrap();
    let mut altered = parse(&input);
    altered["credential_0"][0]["object"]["lexical"][0] = json!(98);
    assert!(
        driver
            .execute(&render(&altered), "long_object_multikey_changed")
            .is_err(),
        "altered suite Multikey lexical accepted"
    );
}

fn incorrect_key_signed_witness(
    plan: &sparq_zk_query::plan::Plan,
    credential: &sparq_zk_query::credential::Credential,
    honest: &serde_json::Map<String, Value>,
    _original: &sparq_zk_query::interp::ResultValue,
) -> serde_json::Map<String, Value> {
    let replacement = oxrdf::Literal::new_typed_literal(
        "0",
        oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    );
    signed_key_witness(plan, credential, honest, "1", replacement)
}

fn signed_key_witness(
    plan: &sparq_zk_query::plan::Plan,
    credential: &sparq_zk_query::credential::Credential,
    honest: &serde_json::Map<String, Value>,
    target: &str,
    replacement: oxrdf::Literal,
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
        if matches!(&quad.object,oxrdf::Term::Literal(l) if l.value()==target) {
            let key = sparq_merkle_suite::comparison_key(&replacement);
            let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
            cells[2].key = field_from_hex_str_strict(&key_hex).unwrap();
            cells[2].enc = poseidon2::hash(&[
                sparq_merkle_suite::Fr::from(3u64),
                suite::hash_bytes(suite::TERM_DOMAIN, &cells[2].bytes),
                cells[2].key,
            ]);
            raw["object"]["key"] = json!(key.as_slice());
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
        credential.metadata().unwrap().digest,
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
    let mut read_sources = vec![];
    for (i, (_, _, cells, raw)) in rows.into_iter().enumerate() {
        changed["credential_0"][i] = raw.clone();
        let convert = |c: suite::TermCell| sparq_zk_query::interp::Cell {
            bound: c.bound,
            kind: c.kind,
            enc: suite::field_bytes(c.identity),
            key_class: c.key_class,
            key: suite::field_bytes(c.key),
            dt: suite::field_bytes(c.dt),
            lang: suite::field_bytes(c.lang),
            scope: c.credential,
        };
        let object = convert(cells[2].clone());
        let mut opening = raw["object"].clone();
        opening["bound"] = json!(true);
        read_sources.push((object, opening));
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
    for (i, node) in plan.nodes().iter().enumerate() {
        if let sparq_zk_query::plan::Operator::Filter(input, _) = node.operator {
            let reads = changed
                .get_mut(&format!("reads_{i}"))
                .unwrap()
                .as_array_mut()
                .unwrap();
            // These fixtures have one value variable; preserve padding beyond its complete input.
            assert!(reads.iter().all(|r| r.as_array().unwrap().len() == 1));
            for (r, row) in trace.tables[input as usize].rows.iter().enumerate() {
                let value = *row.iter().find(|c| c.bound && c.kind == 3).unwrap();
                use sparq_zk_query::interp::EncodeTerm;
                let public = (sparq_zk_query::codegen::generate_suite_fixture(plan)
                    .unwrap()
                    .files
                    .values()
                    .any(|source| source.contains("ops_core_reads::check_identity_equalities("))
                    && value.dt
                        == SuiteEncoder
                            .encode(&oxrdf::Literal::new_simple_literal("").into())
                            .unwrap()
                            .dt)
                    || plan.parameters().iter().any(|p| {
                        if let sparq_zk_query::plan::Parameter::Term(t) = p {
                            SuiteEncoder.encode(t).unwrap() == value
                        } else {
                            false
                        }
                    });
                let mut opening = read_sources
                    .iter()
                    .find(|(c, _)| *c == value)
                    .unwrap()
                    .1
                    .clone();
                if public {
                    opening["bound"] = json!(false);
                    for key in ["kind", "lexical_len", "datatype_len", "language_len"] {
                        opening[key] = json!(0);
                    }
                    for key in ["lexical", "datatype", "language", "key"] {
                        for byte in opening[key].as_array_mut().unwrap() {
                            *byte = json!(0);
                        }
                    }
                }
                reads[r][0] = opening;
            }
        }
    }
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

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn read_site_keys_bind_complete_consumer_inputs_and_lexical_openings() {
    let Some(gate) = support::gate(
        "read_site_keys_bind_complete_consumer_inputs_and_lexical_openings",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 4,
        rows_per_table: 4,
    };
    let cases = [
        // Closest N0 form for the claims subquery: the old claim is rejected by FILTER.
        (
            "date",
            "2010-01-01",
            "2024-01-01",
            "2000-01-01",
            "2020-01-01",
        ),
        // Closest N0 form for MIN inputs: the larger figure still enters the consumer.
        ("integer", "2", "1", "3", "0"),
    ];
    let mut compiled = None;
    for (datatype, target, other, replacement, threshold) in cases {
        let dt = format!("http://www.w3.org/2001/XMLSchema#{datatype}");
        let source = format!(
            "<urn:a> <urn:p> \"{target}\"^^<{dt}> .\n<urn:b> <urn:p> \"{other}\"^^<{dt}> ."
        );
        let query =
            format!("SELECT ?s WHERE {{?s <urn:p> ?x FILTER(?x >= \"{threshold}\"^^<{dt}>)}}");
        let plan = lower(&query, bounds).unwrap();
        let data = vec![authenticated::sign(&source, 0)];
        let cells = vec![data[0].cells(0).unwrap()];
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        let honest = parse(
            &prover_toml_authenticated(
                &plan,
                &data,
                &trace,
                [7; 32],
                [3; 32],
                authenticated::issuer_root(),
            )
            .unwrap(),
        );
        let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
        if let Some((expected, _)) = &compiled {
            assert_eq!(&package, expected);
        } else {
            let workspace = fixture_support::Workspace::new(&package);
            ProgramProver::new(&workspace.0, &package.name)
                .unwrap()
                .compile()
                .unwrap();
            compiled = Some((package.clone(), workspace));
        }
        let driver = ProgramProver::new(&compiled.as_ref().unwrap().1 .0, &package.name).unwrap();
        driver
            .execute(&render(&honest), "complete_read_input")
            .unwrap();
        let (filter, input) = plan
            .nodes()
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                if let sparq_zk_query::plan::Operator::Filter(input, _) = n.operator {
                    Some((i, input as usize))
                } else {
                    None
                }
            })
            .unwrap();
        let reads = format!("reads_{filter}");
        for field in ["lexical", "datatype"] {
            let mut altered = honest.clone();
            altered[&reads][0][0][field][0] = json!(88);
            assert!(
                driver
                    .execute(&render(&altered), "swapped_read_bytes")
                    .is_err(),
                "swapped read bytes accepted: {field}"
            );
        }
        let mut altered = honest.clone();
        altered[&reads][0][0] = honest[&reads][1][0].clone();
        assert!(driver
            .execute(&render(&altered), "different_quad_read")
            .is_err());
        let forged = signed_key_witness(
            &plan,
            &data[0],
            &honest,
            target,
            oxrdf::Literal::new_typed_literal(replacement, oxrdf::NamedNode::new_unchecked(&dt)),
        );
        // The hidden claim remains rejected; this is not a check on final output alone.
        if datatype == "date" {
            assert_eq!(forged["result"], honest["result"]);
        }
        let error = driver
            .execute(&render(&forged), "forged_key_in_complete_input")
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("key disagrees with lexical form"),
            "{error}"
        );
        let mut incomplete = cells.clone();
        incomplete[0].quads.remove(0);
        let incomplete_trace = evaluate(&plan, &incomplete, &SuiteEncoder).unwrap();
        assert!(incomplete_trace.tables[input].rows.len() < trace.tables[input].rows.len());
        let incomplete_input = prover_toml_authenticated(
            &plan,
            &data,
            &incomplete_trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap();
        assert!(driver
            .execute(&incomplete_input, "omitted_matching_scan_row")
            .is_err());
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn request_values_use_exact_verifier_encoded_keys_without_private_openings() {
    let Some(gate) = support::gate(
        "request_values_use_exact_verifier_encoded_keys_without_private_openings",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let data = vec![authenticated::sign("<urn:s> <urn:p> <urn:o> .", 0)];
    let cells = vec![data[0].cells(0).unwrap()];
    let mut compiled = None;
    for literal in [format!("\"{}\"", "a".repeat(100)), "\"hello\"@en".into()] {
        let query = format!("SELECT ?x WHERE {{VALUES ?x {{{literal}}} FILTER(?x = {literal})}}");
        let plan = lower(
            &query,
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 2,
            },
        )
        .unwrap();
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
        let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
        if let Some((expected, _)) = &compiled {
            assert_eq!(&package, expected);
        } else {
            let workspace = fixture_support::Workspace::new(&package);
            ProgramProver::new(&workspace.0, &package.name)
                .unwrap()
                .compile()
                .unwrap();
            compiled = Some((package.clone(), workspace));
        }
        let driver = ProgramProver::new(&compiled.as_ref().unwrap().1 .0, &package.name).unwrap();
        driver.execute(&input, "verifier_values_honest").unwrap();
        let mut changed = parse(&input);
        let reads = changed
            .keys()
            .find(|k| k.starts_with("reads_"))
            .unwrap()
            .clone();
        assert_eq!(changed[&reads][0][0]["bound"], json!(false));
        changed[&reads][0][0]["lexical"][0] = json!(1);
        assert!(driver
            .execute(&render(&changed), "verifier_values_unused_opening")
            .is_err());
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn authenticated_temporal_ranges_use_the_checked_scalar_payload() {
    let Some(gate) = support::gate(
        "authenticated_temporal_ranges_use_the_checked_scalar_payload",
        &["nargo", "bb"],
    ) else {
        return;
    };
    for (datatype, lo, hi) in [
        ("date", "2024-01-01", "2024-02-01"),
        ("date", "1969-12-31", "1970-01-01"),
        (
            "dateTime",
            "1969-12-31T23:59:59.999Z",
            "1970-01-01T00:00:00.000Z",
        ),
        (
            "dateTime",
            "1969-12-31T23:59:59.999",
            "1970-01-01T00:00:00.000",
        ),
    ] {
        let query=format!("PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT ?s WHERE {{?s <urn:p> ?d FILTER(?d >= \"{lo}\"^^xsd:{datatype} && ?d < \"{hi}\"^^xsd:{datatype})}}");
        let source=format!("<urn:a> <urn:p> \"{lo}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}> .\n<urn:b> <urn:p> \"{hi}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}> .\n");
        let plan = lower(
            &query,
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 2,
            },
        )
        .unwrap();
        let data = vec![authenticated::sign(&source, 0)];
        let cells = vec![data[0]
            .cells_with_profile(0, plan.opening_profile())
            .unwrap()];
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        fixture_support::assert_engine_with_encoder(&query, &source, &trace.result, &SuiteEncoder);
        let input = prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap();
        let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
        let workspace = fixture_support::Workspace::new(&package);
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        let artifact = driver.compile().unwrap();
        Layout::new(&plan)
            .audit_authenticated_artifact(
                &serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap(),
            )
            .unwrap();
        driver
            .execute(&input, "temporal_range_honest")
            .unwrap_or_else(|e| panic!("{datatype} {lo}: {e}"));
        let mut changed = trace.clone();
        changed.tables[plan.result_node() as usize].rows.clear();
        let input = prover_toml_authenticated(
            &plan,
            &data,
            &changed,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap();
        assert!(driver
            .execute(&input, "temporal_range_missing_row")
            .is_err());
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn revealed_messages_keys_and_count_are_public_and_bound_to_the_opening() {
    use sparq_zk::{field::field_to_be_bytes_32, sig};
    use sparq_zk_query::statement::{EnvelopeInputs, RevealedInputs, RevealedSignature};
    let Some(gate) = support::gate(
        "revealed_messages_keys_and_count_are_public_and_bound_to_the_opening",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "ASK {?s <urn:p> ?o}",
        Bounds {
            credentials: 2,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let data = vec![authenticated::sign("<urn:a> <urn:p> <urn:o> .", 0)];
    let cells = vec![data[0]
        .cells_with_profile(0, plan.opening_profile())
        .unwrap()];
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    let envelope = EnvelopeInputs::issuer_only(authenticated::issuer_root());
    let input = sparq_zk_query::witness::prover_toml_suite_fixture_revealed(
        &plan, &data, &trace, [7; 32], [3; 32], &envelope,
    )
    .unwrap();
    let package = sparq_zk_query::codegen::generate_suite_fixture(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let artifact = driver.compile().unwrap();
    let artifact: Value = serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap();
    let layout = Layout::new(&plan);
    layout.audit_authenticated_artifact(&artifact).unwrap();
    for name in ["revealed_messages", "revealed_keys", "revealed_count"] {
        let mut changed = artifact.clone();
        changed["abi"]["parameters"]
            .as_array_mut()
            .unwrap()
            .retain(|p| p["name"] != name);
        assert!(
            layout.audit_authenticated_artifact(&changed).is_err(),
            "missing {name}"
        );
    }
    let mut changed = artifact.clone();
    let messages = changed["abi"]["parameters"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["name"] == "revealed_messages")
        .unwrap();
    messages["type"]["length"] = json!(1);
    assert!(
        layout.audit_authenticated_artifact(&changed).is_err(),
        "missing one credential message"
    );
    driver.execute(&input, "revealed_honest").unwrap();
    let message = data[0].signed_message().unwrap();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: sparq_zk_query::credential::dataset_commitment(&data, [3; 32]).unwrap(),
        signature_mode: 0,
        result: trace.result,
    };
    for (label, key, message) in [
        (
            "message",
            sig::SecretKey::from_seed(19),
            message + sparq_merkle_suite::Fr::from(1u64),
        ),
        ("key", sig::SecretKey::from_seed(20), message),
    ] {
        let evidence = RevealedInputs::verify(&[RevealedSignature {
            key: key.public_key(),
            message: field_to_be_bytes_32(&message),
            signature: sig::sign_deterministic(&key, &message),
        }])
        .unwrap();
        let words = layout
            .reconstruct_revealed(&plan, &statement, &envelope, &evidence)
            .unwrap();
        let mut changed = parse(&input);
        if label == "message" {
            changed["revealed_messages"][0] = json!(sparq_zk::field::field_to_hex(&message));
        } else {
            changed["revealed_keys"][0] = json!(sparq_zk::field::field_to_hex(
                &sparq_merkle_suite::metadata::key_cell(&key.public_key())
                    .unwrap()
                    .enc
            ));
        }
        changed["statement_binding"] =
            json!(sparq_zk_query::statement::hex(*words.last().unwrap()));
        assert!(
            driver
                .execute(&render(&changed), "revealed_changed")
                .is_err(),
            "altered revealed {label} accepted"
        );
    }
    // Keep the valid public message/key while changing only the evidence count.
    let key = sig::SecretKey::from_seed(19);
    let admitted = RevealedInputs::verify(&[RevealedSignature {
        key: key.public_key(),
        message: field_to_be_bytes_32(&message),
        signature: sig::sign_deterministic(&key, &message),
    }])
    .unwrap();
    let mut words = layout
        .reconstruct_revealed(&plan, &statement, &envelope, &admitted)
        .unwrap();
    let count_word = words.len() - 6;
    words[count_word] = [0; 32];
    let rebound = sparq_zk_query::statement::binding(&words[..words.len() - 1]).unwrap();
    let mut changed = parse(&input);
    changed["revealed_count"] = json!(0);
    changed["statement_binding"] = json!(sparq_zk_query::statement::hex(rebound));
    assert!(
        driver
            .execute(&render(&changed), "revealed_changed_count")
            .is_err(),
        "altered revealed count accepted"
    );
    for field in ["r.x", "r.y", "s", "e", "e_k"] {
        let mut changed = parse(&input);
        let proof = &mut changed["credential_proof_0"];
        if let Some(coordinate) = field.strip_prefix("r.") {
            proof["r"][coordinate] = json!("1");
        } else {
            proof[field] = json!("1");
        }
        assert!(
            driver
                .execute(&render(&changed), "revealed_signature_padding")
                .is_err(),
            "altered revealed signature padding accepted: {field}"
        );
    }
    let two = vec![
        data[0].clone(),
        authenticated::sign("<urn:b> <urn:p> <urn:o> .", 1),
    ];
    let cells = two
        .iter()
        .enumerate()
        .map(|(i, c)| {
            c.cells_with_profile(i as u32, plan.opening_profile())
                .unwrap()
        })
        .collect::<Vec<_>>();
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    let input = sparq_zk_query::witness::prover_toml_suite_fixture_revealed(
        &plan, &two, &trace, [7; 32], [3; 32], &envelope,
    )
    .unwrap();
    driver.execute(&input, "revealed_two_honest").unwrap();
    let evidence = two
        .iter()
        .map(|c| RevealedSignature {
            key: c.issuer,
            message: field_to_be_bytes_32(&c.signed_message().unwrap()),
            signature: c.credential.signature,
        })
        .collect::<Vec<_>>();
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: sparq_zk_query::credential::dataset_commitment(&two, [3; 32]).unwrap(),
        signature_mode: 0,
        result: trace.result,
    };
    for label in ["message", "key", "order"] {
        let mut signatures = evidence.clone();
        match label {
            "message" => {
                let key = sig::SecretKey::from_seed(19);
                let message = two[1].signed_message().unwrap() + sparq_merkle_suite::Fr::from(1u64);
                signatures[1].message = field_to_be_bytes_32(&message);
                signatures[1].signature = sig::sign_deterministic(&key, &message);
            }
            "key" => {
                let key = sig::SecretKey::from_seed(20);
                signatures[1].key = key.public_key();
                signatures[1].signature =
                    sig::sign_deterministic(&key, &two[1].signed_message().unwrap());
            }
            _ => signatures.swap(0, 1),
        }
        let admitted = RevealedInputs::verify(&signatures).unwrap();
        let words = layout
            .reconstruct_revealed(&plan, &statement, &envelope, &admitted)
            .unwrap();
        let mut changed = parse(&input);
        changed["revealed_messages"] = json!(signatures
            .iter()
            .map(|s| sparq_zk_query::statement::hex(s.message))
            .collect::<Vec<_>>());
        changed["revealed_keys"] = json!(signatures
            .iter()
            .map(|s| sparq_zk::field::field_to_hex(
                &sparq_merkle_suite::metadata::key_cell(&s.key).unwrap().enc
            ))
            .collect::<Vec<_>>());
        changed["statement_binding"] =
            json!(sparq_zk_query::statement::hex(*words.last().unwrap()));
        assert!(
            driver
                .execute(&render(&changed), "revealed_later_slot")
                .is_err(),
            "altered later revealed {label} accepted"
        );
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn canonical_envelope_program_keeps_raw_evidence_outside_sealed_views() {
    let Some(gate) = support::gate(
        "canonical_envelope_program_keeps_raw_evidence_outside_sealed_views",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "ASK {?s <urn:p> ?x}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap().with_opening_profile(sparq_zk_query::capacity::OpeningProfile::LongObjects);
    let package = sparq_zk_query::codegen::generate_enveloped(&plan).unwrap();
    assert_eq!(
        package.files,
        sparq_zk_query::codegen::generate_enveloped(&plan)
            .unwrap()
            .files
    );
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let artifact = driver.compile().unwrap();
    let artifact: Value = serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap();
    Layout::new(&plan)
        .audit_authenticated_artifact(&artifact)
        .unwrap();
    let inputs = artifact["abi"]["parameters"].as_array().unwrap();
    assert!(inputs
        .iter()
        .any(|p| p["name"] == "holder_evidence" && p["visibility"] == "private"));
    assert!(inputs
        .iter()
        .any(|p| p["name"] == "envelope_0" && p["visibility"] == "private"));
    let mut missing = artifact.clone();
    missing["abi"]["parameters"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["name"] != "revealed_messages");
    assert!(Layout::new(&plan)
        .audit_authenticated_artifact(&missing)
        .is_err());
    let policy=authenticated::canonical_policy();
    for property in [
        "<urn:credential> <https://www.w3.org/2018/credentials#validFrom> <urn:not-a-time> .".to_string(),
        format!("<urn:credential> <https://www.w3.org/2018/credentials#validUntil> \"{}\" .", "x".repeat(80)),
    ] {
        let data=[authenticated::canonical_sign(&property,0)];
        let dataset=sparq_zk_query::credential::assemble_issuer_dataset(&data,&policy.trust_members,[7;32]).unwrap();
        let trace=sparq_zk_query::interp::evaluate_dataset(&plan,&dataset,&SuiteEncoder).unwrap();
        let input=sparq_zk_query::witness::prover_toml_enveloped(&plan,&data,&trace,[7;32],[3;32],
            sparq_zk_query::witness::EnvelopeInput{policy:&policy,evidence:&Default::default(),revealed:None}).unwrap();
        driver.execute(&input,"unrequested_validity").unwrap();
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn canonical_status_depth_binds_the_snapshot_and_padding() {
    let Some(gate) = support::gate(
        "canonical_status_depth_binds_the_snapshot_and_padding",
        &["nargo", "bb"],
    ) else {
        return;
    };
    use sparq_zk_query::{
        credential::assemble_issuer_dataset,
        interp::evaluate_dataset,
        witness::{prover_toml_enveloped, EnvelopeInput},
    };
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 16,
            rows_per_table: 2,
        },
    )
    .unwrap().with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures {status:true,..Default::default()});
    let package = sparq_zk_query::codegen::generate(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    for depth in [0, 1, 32] {
        let (data, policy, evidence) = authenticated::canonical_status_fixture(depth);
        let dataset = assemble_issuer_dataset(&data, &policy.trust_members, [7; 32]).unwrap();
        let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
        let input = prover_toml_enveloped(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            EnvelopeInput {
                policy: &policy,
                evidence: &evidence,
                revealed: None,
            },
        )
        .unwrap();
        let row = evidence.credentials[0].statuses[0].quad_index;
        let parsed = parse(&input);
        assert_eq!(parsed["envelope_0"]["status"][row]["depth"], json!(depth));
        driver
            .execute(&input, &format!("status_depth_{depth}"))
            .unwrap();
        for changed_depth in [if depth == 0 { 1 } else { depth - 1 }, 33] {
            let mut altered = parsed.clone();
            altered["envelope_0"]["status"][row]["depth"] = json!(changed_depth);
            assert!(driver
                .execute(
                    &render(&altered),
                    &format!("changed_depth_{depth}_{changed_depth}")
                )
                .is_err());
        }
        if depth == 1 {
            let mut altered = parsed.clone();
            altered["envelope_0"]["status"][row]["path"]["siblings"][1] = json!("1");
            assert!(driver.execute(&render(&altered), "status_padding").is_err());
        }
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn canonical_graph_views_keep_method_assertions_in_the_default_graph() {
    let Some(gate) = support::gate(
        "canonical_graph_views_keep_method_assertions_in_the_default_graph",
        &["nargo", "bb"],
    ) else {
        return;
    };
    use sparq_zk_query::{
        credential::{assemble_issuer_dataset, VERIFIED_ISSUER},
        interp::{evaluate_dataset, ResultValue},
        witness::{prover_toml_enveloped, EnvelopeInput, EnvelopeWitness},
    };
    let policy = authenticated::canonical_policy();
    let data = [authenticated::canonical_sign(
        "<urn:a> <urn:p> <urn:o> .",
        0,
    )];
    let digest = [7; 32];
    let dataset = assemble_issuer_dataset(&data, &policy.trust_members, digest).unwrap();
    for (query, expected) in [
        ("ASK {GRAPH ?g {?s <urn:p> ?o}}".to_owned(), true),
        (format!("ASK {{GRAPH ?g {{?g <{VERIFIED_ISSUER}> ?issuer}}}}"), false),
        (format!("ASK {{VALUES ?issuer {{<urn:issuer>}} FILTER EXISTS {{?g <{VERIFIED_ISSUER}> ?issuer}}}}"), true),
        (format!("ASK {{VALUES ?issuer {{<urn:other>}} FILTER EXISTS {{?g <{VERIFIED_ISSUER}> ?issuer}}}}"), false),
    ] {
        let plan = lower(&query, Bounds {credentials:1,quads_per_credential:4,rows_per_table:2}).unwrap();
        let trace = evaluate_dataset(&plan,&dataset,&SuiteEncoder).unwrap();
        assert_eq!(trace.result,ResultValue::Ask(expected),"{query}");
        let input = prover_toml_enveloped(&plan,&data,&trace,digest,[3;32],EnvelopeInput {
            policy:&policy,evidence:&EnvelopeWitness::default(),revealed:None,
        }).unwrap();
        let package = sparq_zk_query::codegen::generate(&plan).unwrap();
        let workspace = fixture_support::Workspace::new(&package);
        let driver = ProgramProver::new(&workspace.0,&package.name).unwrap();
        driver.execute(&input,"canonical_graph").unwrap();
        let mut altered = parse(&input);
        altered["result"] = json!(!expected);
        assert!(driver.execute(&render(&altered),"altered_graph_result").is_err());
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn canonical_envelope_executes_complete_private_evidence_and_policy_controls() {
    let Some(gate) = support::gate(
        "canonical_envelope_executes_complete_private_evidence_and_policy_controls",
        &["nargo", "bb"],
    ) else {
        return;
    };
    use oxrdf::{Literal, NamedNode, Term};
    use sparq_merkle_suite::poseidon as suite;
    use sparq_zk::{field::field_to_be_bytes_32, sig};
    use sparq_zk_query::{
        capacity::OpeningProfile,
        credential::{assemble_issuer_dataset, Credential},
        envelope_policy::{self, AcceptedSnapshots, HolderBinding, PolicyInput, TrustMember},
        interp::evaluate_dataset,
        witness::{
            prover_toml_enveloped, CredentialEvidence, EnvelopeInput, EnvelopeWitness,
            HolderWitness, StatusWitness,
        },
    };
    let key = sig::SecretKey::from_seed(19);
    let holder = sig::SecretKey::from_seed(23);
    let multikey = sparq_merkle_suite::metadata::multikey(&holder.public_key()).unwrap();
    let source=format!("<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n<urn:credential> <https://www.w3.org/2018/credentials#credentialSubject> <urn:s> .\n<urn:s> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n<urn:s> <https://www.w3.org/2018/credentials#confidenceMethod> <urn:key> .\n<urn:key> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#Multikey> .\n<urn:key> <https://w3id.org/security#publicKeyMultibase> \"{multikey}\" .\n<urn:credential> <https://www.w3.org/2018/credentials#validFrom> \"2020-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n<urn:credential> <https://www.w3.org/2018/credentials#validUntil> \"2030-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n<urn:credential> <https://www.w3.org/2018/credentials#credentialStatus> <urn:entry> .\n<urn:entry> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/status#BitstringStatusListEntry> .\n<urn:entry> <https://www.w3.org/ns/credentials/status#statusListCredential> <urn:list> .\n<urn:entry> <https://www.w3.org/ns/credentials/status#statusPurpose> \"revocation\" .\n<urn:entry> <https://www.w3.org/ns/credentials/status#statusListIndex> \"0\" .\n");
    // Equal-valued distractions still belong to different signed properties.
    let source=source+"<urn:unrelated> <urn:wrong-list> <urn:list> .\n<urn:unrelated> <urn:wrong-purpose> \"revocation\" .\n";
    let config="<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n<urn:proof> <https://w3id.org/security#verificationMethod> <urn:method> .\n<urn:proof> <http://purl.org/dc/terms/created> \"2026-11-13T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n";
    let credential = Credential {
        credential: suite::issue(&source, config, &key, sparq_merkle_suite::Fr::from(101u64))
            .unwrap(),
        issuer: key.public_key(),
        issuer_index: 0,
        issuer_siblings: vec![],
    };
    let mut status_root = envelope_policy::status_leaf(false);
    let mut empty = sparq_merkle_suite::Fr::from(0u64);
    let mut path = vec![];
    for _ in 0..32 {
        path.push(field_to_be_bytes_32(&empty));
        status_root = sparq_zk::poseidon2::hash(&[status_root, empty]);
        empty = sparq_zk::poseidon2::hash(&[empty, empty]);
    }
    let list = suite::term_cell(&Term::NamedNode(NamedNode::new_unchecked("urn:list")), 0).unwrap();
    let purpose = suite::term_cell(&Literal::new_simple_literal("revocation").into(), 0).unwrap();
    let snapshot = envelope_policy::snapshot_digest(
        field_to_be_bytes_32(&list.enc),
        field_to_be_bytes_32(&purpose.enc),
        1,
        32,
        field_to_be_bytes_32(&status_root),
    )
    .unwrap();
    let created = Literal::new_typed_literal(
        "2026-11-13T00:00:00Z",
        NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime"),
    );
    let key_word = sparq_merkle_suite::comparison_key(&created);
    let now = u64::from_be_bytes(key_word[1..9].try_into().unwrap()) ^ 0x8000000000000000;
    let policy = PolicyInput {
        trust_members: vec![TrustMember {
            issuer: NamedNode::new_unchecked("urn:issuer"),
            verification_method: NamedNode::new_unchecked("urn:method"),
            key: key.public_key(),
        }],
        holder_binding: Some(HolderBinding { issuers: None }),
        accepted_snapshots: Some(AcceptedSnapshots {
            digests: vec![snapshot],
            allow_missing: false,
        }),
        validity_time: Some(now),
        earliest_created: Some(now),
    };
    let quad_index = oxttl::NQuadsParser::new()
        .for_slice(credential.credential.document.as_bytes())
        .map(Result::unwrap)
        .position(|q| {
            q.predicate.as_str() == "https://www.w3.org/2018/credentials#credentialStatus"
        })
        .unwrap();
    let digest = [7; 32];
    let evidence = EnvelopeWitness {
        holders: vec![HolderWitness {
            key: holder.public_key(),
            signature: sig::sign_deterministic(&holder, &envelope_policy::holder_message(&digest)),
        }],
        credentials: vec![CredentialEvidence {
            holder_slot: 0,
            statuses: vec![StatusWitness {
                quad_index,
                bit_len: 1,
                depth: 32,
                root: field_to_be_bytes_32(&status_root),
                accepted_index: 0,
                accepted_siblings: vec![],
                bit: false,
                path_index: 0,
                path,
            }],
        }],
    };
    let plan = lower(
        "ASK {?s <urn:p> ?x}",
        Bounds {
            credentials: 1,
            quads_per_credential: 16,
            rows_per_table: 2,
        },
    )
    .unwrap()
    .with_opening_profile(OpeningProfile::LongObjects)
    .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures::ALL);
    let data = [credential];
    let dataset = assemble_issuer_dataset(&data, &policy.trust_members, digest).unwrap();
    let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
    let input = prover_toml_enveloped(
        &plan,
        &data,
        &trace,
        digest,
        [3; 32],
        EnvelopeInput {
            policy: &policy,
            evidence: &evidence,
            revealed: None,
        },
    )
    .unwrap();
    let package = sparq_zk_query::codegen::generate_enveloped(&plan).unwrap();
    let workspace = fixture_support::Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    driver.execute(&input, "canonical_honest").unwrap();
    let revealed = sparq_zk_query::statement::RevealedInputs::verify(&[
        sparq_zk_query::statement::RevealedSignature {
            key: data[0].issuer,
            message: field_to_be_bytes_32(&data[0].signed_message().unwrap()),
            signature: data[0].credential.signature,
        },
    ])
    .unwrap();
    let revealed_input = prover_toml_enveloped(
        &plan,
        &data,
        &trace,
        digest,
        [3; 32],
        EnvelopeInput {
            policy: &policy,
            evidence: &evidence,
            revealed: Some(&revealed),
        },
    )
    .unwrap();
    driver
        .execute(&revealed_input, "canonical_revealed")
        .unwrap();
    let quads=oxttl::NQuadsParser::new().for_slice(data[0].credential.document.as_bytes()).map(Result::unwrap).collect::<Vec<_>>();
    let unrelated_list=quads.iter().position(|q|q.predicate.as_str()=="urn:wrong-list").unwrap();
    let unrelated_purpose=quads.iter().position(|q|q.predicate.as_str()=="urn:wrong-purpose").unwrap();
    let parsed = parse(&input);
    let mut controls = vec![
        ("holder signature", {
            let mut p = parsed.clone();
            p["holder_evidence"][0]["sig"]["s"] = json!("0");
            p
        }),
        ("status bit", {
            let mut p = parsed.clone();
            p["envelope_0"]["status"][quad_index]["bit"] = json!(true);
            p
        }),
        ("status coverage", {
            let mut p = parsed.clone();
            p["envelope_0"]["status"][quad_index]["present"] = json!(false);
            p
        }),
        ("issuer path", {
            let mut p = parsed.clone();
            p["credential_proof_0"]["issuer_siblings"][0] = json!("1");
            p
        }),
        ("validity lexical", {
            let mut p = parsed.clone();
            p["envelope_0"]["validity"]["from"][0] = json!(49);
            p
        }),
        ("required holder absent", {
            let mut p = parsed.clone();
            p["holder_evidence"][0] = json!({"pk":{"x":"0","y":"0"},
                "sig":{"present":false,"r":{"x":"0","y":"0"},"s":"0","e":"0","e_k":"0"}});
            p
        }),
    ];
    for (name, field, row) in [
        ("status list row", "list_row", unrelated_list),
        ("status purpose row", "purpose_row", unrelated_purpose),
        ("status property capacity", "list_row", 16),
    ] {
        let mut changed=parsed.clone();
        changed["envelope_0"]["status"][quad_index][field]=json!(row);
        controls.push((name, changed));
    }
    let other_holder = sig::SecretKey::from_seed(29);
    let mut other_evidence = evidence.clone();
    other_evidence.holders[0] = HolderWitness {
        key: other_holder.public_key(),
        signature: sig::sign_deterministic(
            &other_holder,
            &envelope_policy::holder_message(&digest),
        ),
    };
    controls.push((
        "holder key",
        parse(
            &prover_toml_enveloped(
                &plan,
                &data,
                &trace,
                digest,
                [3; 32],
                EnvelopeInput {
                    policy: &policy,
                    evidence: &other_evidence,
                    revealed: None,
                },
            )
            .unwrap(),
        ),
    ));
    for (name, validity_time, earliest_created) in [
        ("freshness", now, now + 1),
        ("validity time", now + 10 * 366 * 24 * 60 * 60 * 1000, now),
    ] {
        let mut changed = policy.clone();
        changed.validity_time = Some(validity_time);
        changed.earliest_created = Some(earliest_created);
        controls.push((
            name,
            parse(
                &prover_toml_enveloped(
                    &plan,
                    &data,
                    &trace,
                    digest,
                    [3; 32],
                    EnvelopeInput {
                        policy: &changed,
                        evidence: &evidence,
                        revealed: None,
                    },
                )
                .unwrap(),
            ),
        ));
    }
    for (name, changed) in controls {
        assert!(
            driver
                .execute(&render(&changed), "canonical_altered")
                .is_err(),
            "{name}"
        );
    }
    support::complete(gate);
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn converted_credential_view_cannot_be_witnessed_or_constructed() {
    let Some(gate) = support::gate(
        "converted_credential_view_cannot_be_witnessed_or_constructed",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap();
    for (source, expected) in [
        ("fn main(view:core::ops_core_suite::OpenedCredential<1>)->pub Field{view.message()}",
            "Invalid type found in the entry point"),
        ("fn main()->pub Field{let view=core::ops_core_suite::OpenedCredential{constructed:[],rows:[[core::table::unbound();3]],active:[false],message:0,graph:core::table::unbound()};view.message()}",
            "constructed is private"),
    ] {
        let mut package = sparq_zk_query::codegen::generate(&plan).unwrap();
        package.files.insert(format!("{}/src/main.nr",package.name), source.into());
        let workspace = fixture_support::Workspace::new(&package);
        let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
        let diagnostic = format!("{}",driver.compile().unwrap_err());
        assert!(diagnostic.contains(expected),"{diagnostic}");
    }
    support::complete(gate);
}

// [GPT-6] Real corpus adapter deliberately reaches private witnesses after honest preparation.
use super::*;
use crate::verifier::InMemorySeenNonces;
use sha2::{Digest, Sha256};
use sparq_zk::{commit::commit_triples, field::Fr, sig::SecretKey};
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::PathBuf,
};

type Fallible<T> = Result<T, Box<dyn Error>>;

fn mapping(vars: &[Value], rows: &[Value]) -> Fallible<Vec<BTreeMap<String, Term>>> {
    rows.iter()
        .map(|row| {
            let cells = row.as_array().ok_or("mapping row")?;
            if cells.len() != vars.len() {
                return Err("mapping width".into());
            }
            vars.iter()
                .zip(cells)
                .map(|(v, t)| {
                    Ok((
                        v.as_str().ok_or("variable name")?.into(),
                        t.as_str().ok_or("selected binding is unbound")?.parse()?,
                    ))
                })
                .collect()
        })
        .collect()
}

fn fixture(job: &Value) -> Fallible<(Vec<ResultCredential>, ResultPolicy)> {
    let mut triples = Vec::new();
    for triple in job["triples"].as_array().ok_or("missing triples")? {
        let cells = triple.as_array().ok_or("triple array")?;
        if cells.len() != 3 {
            return Err("triple width".into());
        }
        let s: Term = cells[0].as_str().ok_or("subject")?.parse()?;
        let p: Term = cells[1].as_str().ok_or("predicate")?.parse()?;
        let o: Term = cells[2].as_str().ok_or("object")?.parse()?;
        let (Term::NamedNode(s), Term::NamedNode(p)) = (s, p) else {
            return Err("synthetic IRI subject/predicate required".into());
        };
        triples.push(Triple::new(s, p, o));
    }
    let sk = SecretKey::from_seed(123);
    let policy = ResultPolicy {
        trusted_issuers: vec![sk.public_key()],
        snapshots: vec![StatusListSnapshot {
            status_list: "urn:binding-status".into(),
            version: 7,
            bits: vec![0; 128],
        }],
        min_version: 7,
        max_version: 7,
    };
    if triples.is_empty() {
        return Ok((vec![], policy));
    }
    let graph = commit_triples(&triples, Fr::from(91u64))?;
    let reference =
        sig::status_ref_digest(&sig::status_list_id_to_field("urn:binding-status"), 0, 7);
    let message = sig::commitment_message_with_status(&graph.commitment, &graph.salt, &reference);
    Ok((
        vec![ResultCredential {
            graph,
            issuer: sk.public_key(),
            signature: sig::sign_deterministic(&sk, &message),
            status_list: "urn:binding-status".into(),
            status_version: 7,
            status_index: 0,
        }],
        policy,
    ))
}

fn replace(toml: &str, name: &str, replacement: &Value) -> Fallible<String> {
    let prefix = format!("{name} = ");
    let mut found = 0;
    let text = toml
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                found += 1;
                format!("{prefix}{}\n", toml_witness_value(replacement))
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    if found != 1 {
        return Err(format!("expected exactly one witness field {name}").into());
    }
    Ok(text)
}

fn unsatisfied(error: DriverError, output: &Path) -> Fallible<String> {
    match error {
        DriverError::Tool { tool, stderr }
            if tool.starts_with("nargo")
                && (stderr.contains("Failed to solve program")
                    || stderr.contains("Assertion failed")
                    || stderr.contains("assertion failed")
                    || stderr.contains("Cannot satisfy constraint")) =>
        {
            fs::write(output.join("constraint-rejection.txt"), &stderr)?;
            Ok("nargo_unsatisfied_relation".into())
        }
        other => Err(format!("not a demonstrated relation rejection: {other}").into()),
    }
}

fn witness_field(toml: &str, name: &str) -> Fallible<Value> {
    let prefix = format!("{name} = ");
    let value = toml
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .ok_or("missing witness field")?;
    Ok(serde_json::from_str(value)?)
}

// Every mutation starts from a solved honest witness. These inputs bypass the
// support planner, including slots it would never select or mark active.
fn attack_witness(toml: &str, kind: &str) -> Fallible<String> {
    let (name, mut value) = match kind {
        "selected_padding" | "leaf_out_of_range" | "duplicate_support_preimage" => {
            ("selected_leaves", witness_field(toml, "selected_leaves")?)
        }
        "credential_out_of_range" => ("selected_graphs", witness_field(toml, "selected_graphs")?),
        "empty_signed_graph" | "length_mismatch" => ("counts", witness_field(toml, "counts")?),
        "activate_padding_row" | "deactivate_real_row" => {
            ("result_count", witness_field(toml, "result_count")?)
        }
        "inactive_public_nonzero" => ("results", witness_field(toml, "results")?),
        "valid_looking_padding" => ("enc", witness_field(toml, "enc")?),
        _ => return Err("unimplemented malicious witness attack".into()),
    };
    match kind {
        "selected_padding" => value[0][0] = witness_field(toml, "counts")?[0].clone(),
        "leaf_out_of_range" => value[0][0] = json!(16),
        "credential_out_of_range" => {
            value[0][0] = json!(
                witness_field(toml, "counts")?
                    .as_array()
                    .ok_or("counts")?
                    .len()
            )
        }
        "empty_signed_graph" => value[0] = json!(0),
        "length_mismatch" => value[0] = json!(1),
        "activate_padding_row" => value = json!(2),
        "deactivate_real_row" => value = json!(0),
        "inactive_public_nonzero" => value[1][0] = value[0][0].clone(),
        "valid_looking_padding" => value[0][15] = value[0][0].clone(),
        "duplicate_support_preimage" => {
            if value[0][0] == value[0][1] {
                return Err(
                    "duplicate support attack requires two different honest preimages".into(),
                );
            }
            value[0][1] = value[0][0].clone();
        }
        _ => unreachable!(),
    }
    replace(toml, name, &value)
}

fn run(job: &Value, output: &Path) -> Fallible<Value> {
    if job["schema"] != "sparq.proof-binding-job.v1"
        || !matches!(job["operation"].as_str(), Some("binding" | "attack"))
    {
        return Err("Noir corpus schema/operation rejected".into());
    }
    let profile = match job["backend"].as_str() {
        Some("noir_unsigned") => NumericProfile::Unsigned,
        Some("noir_signed") => NumericProfile::Signed,
        _ => return Err("Noir backend unavailable".into()),
    };
    let tier = job["tier"].as_str().ok_or("tier")?;
    if !matches!(tier, "native" | "constraint" | "real") {
        return Err("unknown tier".into());
    }
    let query = job["query"].as_str().ok_or("query")?;
    let vars = job["variables"].as_array().ok_or("variables")?;
    let rows = mapping(vars, job["rows"].as_array().ok_or("rows")?)?;
    let (credentials, policy) = fixture(job)?;
    let mut bytes = b"sparq-proof-bindings-noir-nonce-v1\0".to_vec();
    bytes.extend_from_slice(job["id"].as_str().ok_or("job id")?.as_bytes());
    let nonce = VerifierNonce::from_field(field_from_hash_bytes(blake3::hash(&bytes).as_bytes()));
    let options = ResultOptions::default();
    let prepared = prepare_result_numeric(
        query,
        &credentials,
        &rows,
        &policy,
        &nonce,
        options,
        profile,
    );
    let mut outcome = json!({"schema":"sparq.proof-binding-outcome.v1","job_id":job["id"],
        "case_sha256":job["case_sha256"],"backend":job["backend"],"tier":tier,
        "observed":"rejected","stage":"support","proof_count":0,"verified_count":0,
        "error_class":null,"artifacts":[],"controls":[]});
    let driver = CircuitProver::from_crate_root();
    if tier != "native" {
        pinned_toolchain()?;
    }
    let prepared = match prepared {
        Ok(p) => p,
        Err(ResultError::Rejected(reason)) => {
            outcome["error_class"] = json!(reason);
            if tier == "native" {
                return Ok(outcome);
            }
            let anchor_rows = mapping(
                vars,
                job["attack"]["anchor_rows"]
                    .as_array()
                    .ok_or("anchor row inventory")?,
            )?;
            if anchor_rows.is_empty() {
                outcome["notes"] = json!(
                    "No valid anchor exists in this fixture. Honest support rejection only; mandatory malicious-witness suite is separate."
                );
                return Ok(outcome);
            }
            let anchor = prepare_result_numeric(
                query,
                &credentials,
                &anchor_rows,
                &policy,
                &nonce,
                options,
                profile,
            )?;
            // Establish that this exact package and driver can solve an honest
            // witness before any negative result is attributed to constraints.
            driver.private_package_witness(
                anchor.package,
                &anchor.toml,
                "binding_positive_control",
            )?;
            let mut forged = anchor.presentation.clone();
            forged.rows = rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|(v, t)| Ok((v.clone(), disclosed(t)?)))
                        .collect::<Result<_, ResultError>>()
                })
                .collect::<Result<_, _>>()?;
            let contract = match profile {
                NumericProfile::Unsigned => {
                    NumericContract::Unsigned(anchor.presentation.integer_capacity)
                }
                NumericProfile::Signed => NumericContract::Signed,
            };
            let statement = public_statement_for((&forged).into(), contract, &policy, &nonce)?;
            let mut malicious = anchor.toml.clone();
            for (name, value) in &statement.fields {
                malicious = replace(&malicious, name, value)?;
            }
            let values = malicious
                .lines()
                .find_map(|line| line.strip_prefix("values = "))
                .ok_or("private values")?;
            let mut values: Value = serde_json::from_str(values)?;
            for (i, name) in statement.vars.iter().enumerate() {
                values[0][i] = json!(field_to_hex(&field(
                    rows[0].get(name).ok_or("public binding incomplete")?
                )?));
            }
            malicious = replace(&malicious, "values", &values)?;
            // Bypass the planner: submit the contradictory public rows and
            // private values with the original signed graph and membership paths.
            let rejected =
                driver.private_package_witness(anchor.package, &malicious, "binding_false_witness");
            let class = unsatisfied(
                rejected
                    .err()
                    .ok_or("false binding satisfied the actual circuit")?,
                output,
            )?;
            fs::write(output.join("malicious-input.toml"), &malicious)?;
            outcome["stage"] = json!("constraint");
            outcome["error_class"] = json!(class);
            outcome["controls"] = json!([{"kind":"honest_constraint_positive","executed":true},
                {"kind":"planner_bypassed_false_binding","executed":true}]);
            return Ok(outcome);
        }
        Err(error) => return Err(error.into()),
    };
    if job["operation"] == "attack" {
        if tier == "native" || job["expected_accept"] != false {
            return Err(
                "malicious witness requires constraint/real tier and rejection expectation".into(),
            );
        }
        driver.private_package_witness(
            prepared.package,
            &prepared.toml,
            "attack_positive_control",
        )?;
        let malicious = attack_witness(
            &prepared.toml,
            job["attack"]["kind"].as_str().ok_or("attack kind")?,
        )?;
        let result =
            driver.private_package_witness(prepared.package, &malicious, "attack_negative_control");
        outcome["error_class"] = json!(unsatisfied(
            result
                .err()
                .ok_or("malicious witness satisfied actual circuit")?,
            output
        )?);
        fs::write(output.join("malicious-input.toml"), malicious)?;
        outcome["stage"] = json!("constraint");
        outcome["controls"] = json!([{"kind":"honest_constraint_positive","executed":true},
            {"kind":job["attack"]["kind"],"planner_bypassed":true,"executed":true}]);
        return Ok(outcome);
    }
    outcome["observed"] = json!("accepted");
    if tier == "native" {
        outcome["stage"] = json!("native");
        return Ok(outcome);
    }
    if tier == "constraint" {
        driver.private_package_witness(prepared.package, &prepared.toml, "binding_valid")?;
        outcome["stage"] = json!("constraint");
        return Ok(outcome);
    }
    // Use the real driver directly, then independently reconstruct the verifier
    // statement. This exercises both numeric versions without public test hooks.
    let artifact =
        driver.prove_private_package(prepared.package, &prepared.toml, output, "binding")?;
    if artifact.public_inputs != prepared.public_inputs {
        return Err("proved public ABI differs from expected statement".into());
    }
    let mut presentation = prepared.presentation;
    presentation.proof = artifact.proof;
    let verify = |q: &str,
                  p: &ResultPresentation,
                  policy: &ResultPolicy,
                  nonce: &VerifierNonce,
                  seen: &InMemorySeenNonces| match profile {
        NumericProfile::Unsigned => verify_result(q, p, policy, nonce, seen, &driver, output),
        NumericProfile::Signed => signed::verify_signed_result(
            q,
            &signed::SignedResultPresentation {
                version: p.version,
                query: p.query.clone(),
                rows: p.rows.clone(),
                challenge: p.challenge.clone(),
                issuer_slots: p.issuer_slots.clone(),
                proof: p.proof.clone(),
            },
            policy,
            nonce,
            seen,
            &driver,
            output,
        ),
    };
    let seen = InMemorySeenNonces::new();
    let verified = verify(query, &presentation, &policy, &nonce, &seen)?;
    if verified.rows != rows {
        return Err("actual verified bindings differ".into());
    }
    let replay = verify(query, &presentation, &policy, &nonce, &seen);
    if !matches!(replay,Err(ResultError::Rejected(ref s)) if s == "challenge already consumed") {
        return Err("replay error class differs".into());
    }
    let mut controls = vec![
        json!({"kind":"nonce_replay","error_class":"challenge already consumed","executed":true}),
    ];
    for kind in [
        "fabricated_row",
        "omitted_row",
        "duplicated_row",
        "wrong_query",
        "wrong_issuer",
        "revoked_status",
    ] {
        let mut altered = presentation.clone();
        let mut altered_policy = policy.clone();
        let mut altered_query = query.to_owned();
        match kind {
            "fabricated_row" => {
                *altered.rows[0]
                    .values_mut()
                    .next()
                    .ok_or("missing public binding")? = DisclosedTerm::Iri {
                    value: "urn:missing".into(),
                };
            }
            "omitted_row" => {
                altered.rows.pop();
            }
            "duplicated_row" => {
                altered.rows.push(altered.rows[0].clone());
            }
            "wrong_query" => {
                altered_query.push(' ');
            }
            "wrong_issuer" => {
                altered_policy.trusted_issuers = vec![SecretKey::from_seed(124).public_key()];
            }
            "revoked_status" => {
                altered_policy.snapshots[0].bits[0] = 1;
            }
            _ => unreachable!(),
        }
        let rejected = verify(
            &altered_query,
            &altered,
            &altered_policy,
            &nonce,
            &InMemorySeenNonces::new(),
        );
        let Err(ResultError::Rejected(reason)) = rejected else {
            return Err(format!(
                "genuine artifact control {kind} did not return a typed verifier rejection"
            )
            .into());
        };
        controls.push(json!({"kind":kind,"error_class":reason,"executed":true,
            "stage":if reason == "cryptographic proof rejected" {"cryptographic_verifier"} else {"verifier_admission"}}));
    }
    let proof = match profile {
        NumericProfile::Unsigned => serde_json::to_vec(&presentation)?,
        NumericProfile::Signed => serde_json::to_vec(&signed::SignedResultPresentation {
            version: presentation.version,
            query: presentation.query.clone(),
            rows: presentation.rows.clone(),
            challenge: presentation.challenge.clone(),
            issuer_slots: presentation.issuer_slots.clone(),
            proof: presentation.proof.clone(),
        })?,
    };
    fs::write(output.join("presentation.json"), &proof)?;
    outcome["stage"] = json!("proof_verifier");
    outcome["proof_count"] = json!(1);
    outcome["verified_count"] = json!(1);
    outcome["artifacts"] =
        json!([{"path":"presentation.json","sha256":format!("{:x}",Sha256::digest(&proof))}]);
    outcome["controls"] = json!(controls);
    Ok(outcome)
}

#[test]
#[ignore = "explicit corpus adapter; native tier is not a proof, real tier requires pinned tools"]
fn run_job() -> Fallible<()> {
    let input =
        PathBuf::from(std::env::var_os("SPARQ_PROOF_BINDING_JOB").ok_or("job path required")?);
    let output = PathBuf::from(
        std::env::var_os("SPARQ_PROOF_BINDING_OUTPUT").ok_or("output path required")?,
    );
    let mut data = Vec::new();
    fs::File::open(input)?
        .take(1_048_577)
        .read_to_end(&mut data)?;
    if data.len() > 1_048_576 {
        return Err("job byte capacity".into());
    }
    let job: Value = serde_json::from_slice(&data)?;
    fs::create_dir(&output)?;
    let mut result = run(&job, &output)?;
    for name in ["malicious-input.toml", "constraint-rejection.txt"] {
        let path = output.join(name);
        if path.is_file() {
            result["artifacts"]
                .as_array_mut()
                .ok_or("artifact inventory")?
                .push(json!({
                    "path":name,"sha256":format!("{:x}",Sha256::digest(fs::read(path)?))
                }));
        }
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("outcome.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

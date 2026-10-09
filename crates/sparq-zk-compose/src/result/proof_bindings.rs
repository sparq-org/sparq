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

// [OPUS-5.5] beadzkp-15.1.1: the only members the explicit version-four
// backend may count. A legacy or signed preparation is an error, never a pass.
const PUBLIC_PATTERN_PACKAGES: [&str; 2] = [
    "result_v4_k1_n16_p3_r4_f0_d10",
    "result_v4_k2_n16_p3_r4_f0_d10",
];

// Private fields that carry the original signed support. A false binding may
// change only public statement fields and the private `values` row.
const SIGNED_SUPPORT_FIELDS: [&str; 8] = [
    "counts",
    "salts",
    "enc",
    "signatures",
    "selected_graphs",
    "selected_leaves",
    "selected_types",
    "selected_hashes",
];

fn require_public_pattern(prepared: &PreparedResult) -> Fallible<Value> {
    if prepared.presentation.version != PUBLIC_PATTERN_VERSION
        || !PUBLIC_PATTERN_PACKAGES.contains(&prepared.package)
    {
        return Err("public-pattern backend fell back from its version-four contract".into());
    }
    Ok(json!({"version": PUBLIC_PATTERN_VERSION, "package": prepared.package}))
}

fn contract_for(
    public_pattern: bool,
    profile: NumericProfile,
    anchor: &PreparedResult,
) -> NumericContract {
    match (public_pattern, profile) {
        (true, _) => NumericContract::PublicPattern,
        (false, NumericProfile::Unsigned) => {
            NumericContract::Unsigned(anchor.presentation.integer_capacity)
        }
        (false, NumericProfile::Signed) => NumericContract::Signed,
    }
}

// Version four never reads pattern zero's typed openings. Changing them is a
// no-op for that relation, so it must not be counted as a rejected attack.
fn unused_opening_changed(honest: &str, candidate: &str) -> Fallible<bool> {
    for name in ["selected_types", "selected_hashes"] {
        let (a, b) = (
            witness_field(honest, name)?,
            witness_field(candidate, name)?,
        );
        let rows = a.as_array().ok_or("opening rows")?;
        if b.as_array().ok_or("opening rows")?.len() != rows.len() {
            return Ok(true);
        }
        if (0..rows.len()).any(|r| a[r][0] != b[r][0]) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Positive control: a type tag outside IRI/literal and a nonzero hash would
// fail any generic opening check that actually read pattern zero.
fn unused_opening_witness(toml: &str) -> Fallible<String> {
    let mut types = witness_field(toml, "selected_types")?;
    let mut hashes = witness_field(toml, "selected_hashes")?;
    types[0][0][0] = json!(3);
    hashes[0][0][0] = json!(field_to_hex(&Fr::from(1u64)));
    replace(
        &replace(toml, "selected_types", &types)?,
        "selected_hashes",
        &hashes,
    )
}

// Bypass the planner: submit contradictory public rows (and, for version four,
// the verifier-derived public triple table) plus matching private values with
// the anchor's original signed graph and membership paths.
fn false_binding_witness(
    anchor: &PreparedResult,
    rows: &[BTreeMap<String, Term>],
    contract: NumericContract,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Fallible<String> {
    let mut forged = anchor.presentation.clone();
    forged.rows = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|(v, t)| Ok((v.clone(), disclosed(t)?)))
                .collect::<Result<_, ResultError>>()
        })
        .collect::<Result<_, _>>()?;
    let statement = public_statement_for((&forged).into(), contract, policy, nonce)?;
    let mut malicious = anchor.toml.clone();
    for (name, value) in &statement.fields {
        malicious = replace(&malicious, name, value)?;
    }
    let mut values = witness_field(&malicious, "values")?;
    for (i, name) in statement.vars.iter().enumerate() {
        values[0][i] = json!(field_to_hex(&field(
            rows[0].get(name).ok_or("public binding incomplete")?
        )?));
    }
    malicious = replace(&malicious, "values", &values)?;
    for name in SIGNED_SUPPORT_FIELDS {
        if witness_field(&malicious, name)? != witness_field(&anchor.toml, name)? {
            return Err("false binding altered the original signed support".into());
        }
    }
    if matches!(contract, NumericContract::PublicPattern) {
        let (_, derived) = statement
            .fields
            .iter()
            .find(|(name, _)| *name == "public_triples")
            .ok_or("version-four statement omits public_triples")?;
        if witness_field(&malicious, "public_triples")? != *derived {
            return Err("false binding did not carry the reconstructed public triples".into());
        }
    }
    Ok(malicious)
}

fn run(job: &Value, output: &Path) -> Fallible<Value> {
    if job["schema"] != "sparq.proof-binding-job.v1"
        || !matches!(job["operation"].as_str(), Some("binding" | "attack"))
    {
        return Err("Noir corpus schema/operation rejected".into());
    }
    // [OPUS-5.5] beadzkp-15.1.1: version four is an explicit backend only.
    let (profile, public_pattern) = match job["backend"].as_str() {
        Some("noir_unsigned") => (NumericProfile::Unsigned, false),
        Some("noir_signed") => (NumericProfile::Signed, false),
        Some("noir_public_pattern") => (NumericProfile::Unsigned, true),
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
    let prepare = |rows: &[BTreeMap<String, Term>]| {
        if public_pattern {
            prepare_result_public_pattern(query, &credentials, rows, &policy, &nonce, options)
        } else {
            prepare_result_numeric(query, &credentials, rows, &policy, &nonce, options, profile)
        }
    };
    let prepared = prepare(rows.as_slice());
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
                // An empty signed graph is not the same evidence as refused support.
                if public_pattern {
                    outcome["rejection_scope"] = json!(if credentials.is_empty() {
                        "empty_graph"
                    } else {
                        "native_support"
                    });
                }
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
            let anchor = prepare(anchor_rows.as_slice())?;
            if public_pattern {
                outcome["contract"] = require_public_pattern(&anchor)?;
            }
            // Establish that this exact package and driver can solve an honest
            // witness before any negative result is attributed to constraints.
            driver.private_package_witness(
                anchor.package,
                &anchor.toml,
                "binding_positive_control",
            )?;
            let contract = contract_for(public_pattern, profile, &anchor);
            let malicious = false_binding_witness(&anchor, &rows, contract, &policy, &nonce)?;
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
            let mut controls = vec![
                json!({"kind":"honest_constraint_positive","executed":true}),
                json!({"kind":"planner_bypassed_false_binding","executed":true}),
            ];
            if public_pattern {
                controls.push(json!({"kind":"public_triples_reconstructed","executed":true}));
            }
            outcome["controls"] = json!(controls);
            return Ok(outcome);
        }
        Err(error) => return Err(error.into()),
    };
    if public_pattern {
        outcome["contract"] = require_public_pattern(&prepared)?;
    }
    if job["operation"] == "attack" {
        if tier == "native" || job["expected_accept"] != false {
            return Err(
                "malicious witness requires constraint/real tier and rejection expectation".into(),
            );
        }
        let malicious = attack_witness(
            &prepared.toml,
            job["attack"]["kind"].as_str().ok_or("attack kind")?,
        )?;
        if public_pattern && unused_opening_changed(&prepared.toml, &malicious)? {
            return Err(
                "attack mutates the unused version-four pattern-zero opening; \
                 classify it as an unused-opening positive control, not an attack"
                    .into(),
            );
        }
        driver.private_package_witness(
            prepared.package,
            &prepared.toml,
            "attack_positive_control",
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
        let mut controls = vec![
            json!({"kind":"honest_constraint_positive","executed":true}),
            json!({"kind":job["attack"]["kind"],"planner_bypassed":true,"executed":true}),
        ];
        if public_pattern {
            controls.push(json!({"kind":"pattern_zero_opening_untouched","executed":true}));
        }
        outcome["controls"] = json!(controls);
        return Ok(outcome);
    }
    outcome["observed"] = json!("accepted");
    if tier == "native" {
        outcome["stage"] = json!("native");
        return Ok(outcome);
    }
    // Legacy backends keep their previous (empty) prefix of controls.
    let mut controls = Vec::new();
    if public_pattern {
        let unused = unused_opening_witness(&prepared.toml)?;
        if !unused_opening_changed(&prepared.toml, &unused)? {
            return Err("unused-opening positive control is a no-op".into());
        }
        driver.private_package_witness(prepared.package, &unused, "unused_opening_positive")?;
        controls.push(json!({"kind":"unused_pattern_zero_opening_positive","executed":true}));
    }
    if tier == "constraint" {
        driver.private_package_witness(prepared.package, &prepared.toml, "binding_valid")?;
        outcome["stage"] = json!("constraint");
        outcome["controls"] = json!(controls);
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
    controls.push(
        json!({"kind":"nonce_replay","error_class":"challenge already consumed","executed":true}),
    );
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
    // A genuine version-four proof relabeled as another version must reject:
    // version one derives the legacy member and omits public_triples.
    let version_controls: &[(&str, u32)] = if public_pattern {
        &[("legacy_version_one", 1), ("version_three", 3)]
    } else {
        &[]
    };
    for &(kind, version) in version_controls {
        let mut altered = presentation.clone();
        altered.version = version;
        let rejected = verify(query, &altered, &policy, &nonce, &InMemorySeenNonces::new());
        let Err(ResultError::Rejected(reason)) = rejected else {
            return Err(format!("version control {kind} did not return a typed rejection").into());
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

// [OPUS-5.5] beadzkp-15.1.1: native adapter tests. They execute no Noir and
// establish no proof; constraint and real cells run only in the CI campaign.
const SCAN: &str = "SELECT DISTINCT ?s ?o WHERE { ?s <urn:p> ?o }";
const JOIN: &str = "SELECT DISTINCT ?s ?m ?o WHERE { ?s <urn:p> ?m . ?m <urn:p> ?o }";
// Mirrors corpus.py NOIR_ATTACKS; the Python plan pins the same ten kinds.
const NOIR_ATTACKS: [&str; 10] = [
    "selected_padding",
    "leaf_out_of_range",
    "credential_out_of_range",
    "empty_signed_graph",
    "length_mismatch",
    "activate_padding_row",
    "deactivate_real_row",
    "inactive_public_nonzero",
    "valid_looking_padding",
    "duplicate_support_preimage",
];

fn cycle() -> Value {
    json!([
        ["<urn:a>", "<urn:p>", "<urn:b>"],
        ["<urn:b>", "<urn:p>", "<urn:a>"]
    ])
}

fn native_job(backend: &str, query: &str, triples: Value, vars: Value, row: Value) -> Value {
    json!({"schema":"sparq.proof-binding-job.v1","operation":"binding","id":"native-unit",
        "case_sha256":"native-unit","backend":backend,"tier":"native","query":query,
        "triples":triples,"variables":vars,"rows":[row]})
}

fn native(job: &Value) -> Fallible<Value> {
    // The native tier never writes artifacts; a missing directory proves it.
    run(job, Path::new("/nonexistent-native-tier-output"))
}

fn prepared_for(
    job: &Value,
    public_pattern: bool,
    profile: NumericProfile,
) -> Fallible<PreparedResult> {
    let (credentials, policy) = fixture(job)?;
    let vars = job["variables"].as_array().ok_or("variables")?;
    let rows = mapping(vars, job["rows"].as_array().ok_or("rows")?)?;
    let nonce = VerifierNonce::from_field(Fr::from(7u64));
    let query = job["query"].as_str().ok_or("query")?;
    let options = ResultOptions::default();
    Ok(if public_pattern {
        prepare_result_public_pattern(query, &credentials, &rows, &policy, &nonce, options)?
    } else {
        prepare_result_numeric(
            query,
            &credentials,
            &rows,
            &policy,
            &nonce,
            options,
            profile,
        )?
    })
}

#[test]
fn public_pattern_native_valid_bindings_dispatch_v4_only() -> Fallible<()> {
    for (query, vars, row) in [
        (SCAN, json!(["s", "o"]), json!(["<urn:a>", "<urn:b>"])),
        (
            JOIN,
            json!(["s", "m", "o"]),
            json!(["<urn:a>", "<urn:b>", "<urn:a>"]),
        ),
    ] {
        let job = native_job(
            "noir_public_pattern",
            query,
            cycle(),
            vars.clone(),
            row.clone(),
        );
        let outcome = native(&job)?;
        assert_eq!(outcome["observed"], "accepted");
        assert_eq!(outcome["stage"], "native");
        assert_eq!(
            outcome["contract"],
            json!({"version": 4, "package": "result_v4_k1_n16_p3_r4_f0_d10"})
        );
        // Existing backends keep their outcome shape and legacy dispatch.
        let legacy = native(&native_job("noir_unsigned", query, cycle(), vars, row))?;
        assert_eq!(legacy["observed"], "accepted");
        assert!(legacy.get("contract").is_none());
    }
    Ok(())
}

#[test]
fn public_pattern_native_rejections_separate_empty_graph_from_support() -> Fallible<()> {
    let absent = native_job(
        "noir_public_pattern",
        SCAN,
        cycle(),
        json!(["s", "o"]),
        json!(["<urn:a>", "<urn:missing>"]),
    );
    let outcome = native(&absent)?;
    assert_eq!(outcome["observed"], "rejected");
    assert_eq!(outcome["stage"], "support");
    assert_eq!(outcome["rejection_scope"], "native_support");
    assert!(outcome["error_class"].is_string());
    assert!(outcome.get("contract").is_none());
    let empty = native_job(
        "noir_public_pattern",
        SCAN,
        json!([]),
        json!(["s", "o"]),
        json!(["<urn:a>", "<urn:b>"]),
    );
    let outcome = native(&empty)?;
    assert_eq!(outcome["observed"], "rejected");
    assert_eq!(outcome["rejection_scope"], "empty_graph");
    let mut unknown = empty.clone();
    unknown["backend"] = json!("noir_public_pattern_v1");
    assert!(native(&unknown).is_err());
    Ok(())
}

#[test]
fn public_pattern_guard_refuses_legacy_and_signed_preparations() -> Fallible<()> {
    let job = native_job(
        "noir_public_pattern",
        SCAN,
        cycle(),
        json!(["s", "o"]),
        json!(["<urn:a>", "<urn:b>"]),
    );
    let v4 = prepared_for(&job, true, NumericProfile::Unsigned)?;
    assert_eq!(
        require_public_pattern(&v4)?["version"],
        PUBLIC_PATTERN_VERSION
    );
    for profile in [NumericProfile::Unsigned, NumericProfile::Signed] {
        let legacy = prepared_for(&job, false, profile)?;
        assert!(require_public_pattern(&legacy).is_err());
    }
    Ok(())
}

#[test]
fn public_pattern_attack_inventory_leaves_unused_opening_untouched() -> Fallible<()> {
    let job = native_job(
        "noir_public_pattern",
        JOIN,
        cycle(),
        json!(["s", "m", "o"]),
        json!(["<urn:a>", "<urn:b>", "<urn:a>"]),
    );
    let prepared = prepared_for(&job, true, NumericProfile::Unsigned)?;
    let zero = json!(field_to_hex(&Fr::from(0u64)));
    assert_eq!(
        witness_field(&prepared.toml, "selected_types")?[0][0],
        json!([0, 0, 0])
    );
    assert_eq!(
        witness_field(&prepared.toml, "selected_hashes")?[0][0],
        json!([zero.clone(), zero.clone(), zero])
    );
    for kind in NOIR_ATTACKS {
        let malicious = attack_witness(&prepared.toml, kind)?;
        assert_ne!(malicious, prepared.toml, "{kind} is a no-op");
        assert!(
            !unused_opening_changed(&prepared.toml, &malicious)?,
            "{kind}"
        );
    }
    let unused = unused_opening_witness(&prepared.toml)?;
    assert!(unused_opening_changed(&prepared.toml, &unused)?);
    Ok(())
}

#[test]
fn public_pattern_false_binding_reconstructs_v4_statement() -> Fallible<()> {
    let job = native_job(
        "noir_public_pattern",
        SCAN,
        cycle(),
        json!(["s", "o"]),
        json!(["<urn:a>", "<urn:b>"]),
    );
    let anchor = prepared_for(&job, true, NumericProfile::Unsigned)?;
    let (_, policy) = fixture(&job)?;
    let nonce = VerifierNonce::from_field(Fr::from(7u64));
    let forged = mapping(
        job["variables"].as_array().ok_or("variables")?,
        &[json!(["<urn:a>", "<urn:missing>"])],
    )?;
    let contract = contract_for(true, NumericProfile::Unsigned, &anchor);
    let malicious = false_binding_witness(&anchor, &forged, contract, &policy, &nonce)?;
    let table = witness_field(&malicious, "public_triples")?;
    assert_ne!(table, witness_field(&anchor.toml, "public_triples")?);
    assert_eq!(table[0][2], json!(field_to_hex(&field(&forged[0]["o"])?)));
    assert_eq!(
        witness_field(&malicious, "version")?,
        json!(PUBLIC_PATTERN_VERSION)
    );
    // Wrong-version controls: a legacy contract cannot adopt the V4 statement,
    // and relabeling it as version one derives the legacy member without a table.
    let legacy = contract_for(false, NumericProfile::Unsigned, &anchor);
    assert!(false_binding_witness(&anchor, &forged, legacy, &policy, &nonce).is_err());
    let mut relabeled = anchor.presentation.clone();
    relabeled.version = 1;
    let statement = public_statement(&relabeled, &policy, &nonce)?;
    assert_eq!(statement.package, "result_v1_k1_n16_p3_r4_f0");
    assert!(statement.fields.iter().all(|(n, _)| *n != "public_triples"));
    Ok(())
}

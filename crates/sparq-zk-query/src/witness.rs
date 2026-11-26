//! Render the interpreter's complete table trace for generated-package execution.

use crate::{
    interp::{row_key, Cell, CredentialCells, EncodeTerm, ResultValue, Trace},
    plan::{Operator, Parameter, Plan, ResultForm, Schema, Slot},
    Error,
};
use std::{collections::HashMap, fmt::Write};

/// A research execution input. The method layer supplies its stored request digest.
pub fn prover_toml(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
) -> Result<String, Error> {
    prover_toml_with_scan_dataset(
        plan,
        credentials,
        encoder,
        trace,
        request_digest,
        presentation_salt,
        || {
            let cap = plan.capacity();
            Ok(padded_scan_dataset(
                credentials,
                std::iter::repeat_n(cap.quads_per_credential, cap.credentials as usize),
            ))
        },
    )
}

fn prover_toml_with_scan_dataset(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    scan_dataset: impl FnOnce() -> Result<Vec<Option<[Cell; 3]>>, Error>,
) -> Result<String, Error> {
    if trace.tables.len() != plan.nodes().len() {
        return Err(Error::Invalid("trace node count".into()));
    }
    let input_commitment = crate::statement::dataset_commitment(credentials, presentation_salt)?;
    let statement = crate::statement::Statement {
        request_digest,
        input_commitment,
        signature_mode: 0,
        result: trace.result.clone(),
    };
    let words = crate::statement::Layout::new(plan).reconstruct(plan, &statement, encoder)?;
    let statement_binding = words.last().expect("statement binding");
    let mut out = String::new();
    writeln!(
        out,
        "input_commitment = {}",
        array(input_commitment.iter().map(u8::to_string))
    )
    .unwrap();
    writeln!(out, "signature_mode = 0").unwrap();
    writeln!(out, "statement_binding = {}", field(*statement_binding)).unwrap();
    writeln!(
        out,
        "presentation_salt = {}",
        array(presentation_salt.iter().map(u8::to_string))
    )
    .unwrap();
    writeln!(
        out,
        "request_digest = {}",
        array(request_digest.iter().map(u8::to_string))
    )
    .unwrap();
    let bounds = plan.bounds();
    writeln!(
        out,
        "bounds = [{}, {}, {}]",
        bounds.credentials, bounds.quads_per_credential, bounds.rows_per_table
    )
    .unwrap();
    let mut parameters = plan
        .parameters()
        .iter()
        .filter_map(|p| {
            if let Parameter::Term(t) = p {
                Some(encoder.encode(t).map(cell))
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parameters.is_empty() {
        parameters.push(cell(Cell::default()));
    }
    writeln!(out, "parameters = {}", array(parameters)).unwrap();
    let mut indices: Vec<_> = plan
        .parameters()
        .iter()
        .filter_map(|p| {
            if let Parameter::Index(n) = p {
                Some(n.to_string())
            } else {
                None
            }
        })
        .collect();
    if indices.is_empty() {
        indices.push("0".into());
    }
    writeln!(out, "indices = {}", array(indices)).unwrap();
    let result = match (plan.form(), &trace.result) {
        (ResultForm::Ask, ResultValue::Ask(v)) => v.to_string(),
        (ResultForm::Select, ResultValue::Select { rows, variables }) => {
            let expected: Vec<_> = plan
                .result_schema()
                .variables
                .iter()
                .map(|&v| plan.variable_names()[v as usize].clone())
                .collect();
            if *variables != expected {
                return Err(Error::Invalid("result variables".into()));
            }
            table(rows, plan.result_schema())?
        }
        _ => return Err(Error::Invalid("result form".into())),
    };
    writeln!(out, "result = {result}").unwrap();
    let cap = plan.capacity();
    if credentials.len() > cap.credentials as usize
        || credentials
            .iter()
            .any(|c| c.quads.len() > cap.quads_per_credential as usize)
    {
        return Err(Error::Capacity("witness credentials".into()));
    }
    let mut padded = vec![];
    let mut counts = vec![];
    for c in 0..cap.credentials as usize {
        let quads = credentials.get(c).map_or(&[][..], |c| c.quads.as_slice());
        counts.push(quads.len().to_string());
        let mut encoded = vec![];
        for q in 0..cap.quads_per_credential as usize {
            encoded.push(array(
                quads
                    .get(q)
                    .copied()
                    .unwrap_or([Cell::default(); 4])
                    .into_iter()
                    .map(cell),
            ));
        }
        padded.push(array(encoded));
    }
    writeln!(
        out,
        "credentials = {}\ncounts = {}\ncredential_count = {}",
        array(padded),
        array(counts),
        credentials.len()
    )
    .unwrap();
    for (i, (t, node)) in trace.tables.iter().zip(plan.nodes()).enumerate() {
        writeln!(out, "t{i} = {}", table(&t.rows, &node.output)?).unwrap();
    }
    if plan
        .nodes()
        .iter()
        .any(|node| matches!(node.operator, Operator::Scan(_)))
    {
        append_scan_hints(plan, encoder, trace, &scan_dataset()?, &mut out)?;
    }
    Ok(out)
}

fn padded_scan_dataset(
    credentials: &[CredentialCells],
    capacities: impl IntoIterator<Item = u32>,
) -> Vec<Option<[Cell; 3]>> {
    #[cfg(test)]
    SCAN_DATASET_BUILDS.set(SCAN_DATASET_BUILDS.get() + 1);
    capacities
        .into_iter()
        .enumerate()
        .flat_map(|(c, capacity)| {
            (0..capacity as usize).map(move |q| {
                credentials
                    .get(c)
                    .and_then(|credential| credential.quads.get(q))
                    .map(|quad| [quad[0], quad[1], quad[2]])
            })
        })
        .collect()
}

#[cfg(test)]
thread_local! {
    static SCAN_DATASET_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn append_scan_hints(
    plan: &Plan,
    encoder: &impl EncodeTerm,
    trace: &Trace,
    dataset: &[Option<[Cell; 3]>],
    out: &mut String,
) -> Result<(), Error> {
    for (node_id, node) in plan.nodes().iter().enumerate() {
        let Operator::Scan(pattern) = &node.operator else {
            continue;
        };
        let mut constants = [Cell::default(); 3];
        for (p, slot) in pattern.iter().enumerate() {
            if let Slot::Parameter(id) = slot {
                let Parameter::Term(term) = &plan.parameters()[*id as usize] else {
                    return Err(Error::Invalid("scan term parameter".into()));
                };
                constants[p] = encoder.encode(term)?;
            }
        }
        let mut target = vec![0; dataset.len()];
        let mut source = vec![0; node.output.capacity as usize];
        let mut seen = vec![false; source.len()];
        let rows = &trace.tables[node_id].rows;
        let mut positions = HashMap::with_capacity(rows.len());
        for (r, row) in rows.iter().enumerate() {
            positions.entry(row_key(row)).or_insert(r);
        }
        for (i, triple) in dataset.iter().enumerate() {
            let Some(triple) = triple else { continue };
            let mut row = vec![Cell::default(); node.output.variables.len()];
            let mut matches = true;
            for (p, slot) in pattern.iter().enumerate() {
                match slot {
                    Slot::Parameter(_) => matches &= triple[p].same_term(constants[p]),
                    Slot::Variable(v) => {
                        let col = node
                            .output
                            .variables
                            .iter()
                            .position(|a| a == v)
                            .expect("scan column");
                        matches &= !row[col].bound || row[col].same_term(triple[p]);
                        row[col] = triple[p];
                    }
                }
            }
            if !matches {
                continue;
            }
            if let Some(&r) = positions.get(&row_key(&row)) {
                target[i] = r as u32;
                if !seen[r] {
                    source[r] = i as u32;
                    seen[r] = true;
                }
            }
        }
        writeln!(
            out,
            "target_{node_id} = {}",
            array(target.iter().map(u32::to_string))
        )
        .unwrap();
        writeln!(
            out,
            "source_{node_id} = {}",
            array(source.iter().map(u32::to_string))
        )
        .unwrap();
    }
    Ok(())
}

fn array(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}
fn field(word: [u8; 32]) -> String {
    format!(
        "\"0x{}\"",
        word.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
fn cell(c: Cell) -> String {
    format!("{{ bound = {}, kind = {}, enc = {}, key_class = {}, key = {}, dt = {}, lang = {}, scope = {} }}",c.bound,c.kind,field(c.enc),c.key_class,field(c.key),field(c.dt),field(c.lang),c.scope)
}
fn table(rows: &[Vec<Cell>], schema: &Schema) -> Result<String, Error> {
    if rows.len() > schema.capacity as usize {
        return Err(Error::Capacity("witness table".into()));
    }
    if rows.iter().any(|r| r.len() != schema.variables.len()) {
        return Err(Error::Invalid("witness row width".into()));
    }
    let mut padded: Vec<_> = rows
        .iter()
        .map(|r| {
            if r.is_empty() {
                array([cell(Cell::default())])
            } else {
                array(r.iter().copied().map(cell))
            }
        })
        .collect();
    padded.resize(
        schema.capacity as usize,
        array(std::iter::repeat_n(
            cell(Cell::default()),
            schema.abi_width(),
        )),
    );
    Ok(format!(
        "{{ len = {}, rows = {} }}",
        rows.len(),
        array(padded)
    ))
}

/// Render a canonical issuer-only witness with the compiled trust-set depth.
/// Credentials carry canonical trust-member paths supplied independently of the proof.
pub fn prover_toml_authenticated(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    issuer_root: [u8; 32],
) -> Result<String, Error> {
    prover_toml_authenticated_with_envelope(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        &crate::statement::EnvelopeInputs::issuer_only(issuer_root),
    )
}

/// Render canonical issuer and envelope inputs with no private holder/status evidence.
/// Use `prover_toml_enveloped` when the stored policy requires private evidence.
pub fn prover_toml_authenticated_with_envelope(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
) -> Result<String, Error> {
    envelope::render_canonical(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        envelope::CanonicalInput {
            public: envelope,
            evidence: &EnvelopeWitness::default(),
            revealed: None,
        },
    )
}

/// Render the canonical envelope in revealed mode after native signature checks.
pub fn prover_toml_revealed(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
) -> Result<String, Error> {
    let revealed = revealed_inputs(credentials)?;
    envelope::render_canonical(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        envelope::CanonicalInput {
            public: envelope,
            evidence: &EnvelopeWitness::default(),
            revealed: Some(&revealed),
        },
    )
}

/// Render the suite-opening fixture, without canonical envelope claims.
pub fn prover_toml_suite_fixture(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    issuer_root: [u8; 32],
) -> Result<String, Error> {
    prover_toml_suite_fixture_with_envelope(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        &crate::statement::EnvelopeInputs::issuer_only(issuer_root),
    )
}

/// Encode public words for the suite-opening fixture.
pub fn prover_toml_suite_fixture_with_envelope(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
) -> Result<String, Error> {
    prover_toml_authenticated_mode(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        envelope,
        Authentication {
            revealed: None,
            canonical_issuers: None,
        },
    )
}

/// Render the suite-opening fixture in revealed-signature mode.
pub fn prover_toml_suite_fixture_revealed(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
) -> Result<String, Error> {
    let revealed = revealed_inputs(credentials)?;
    prover_toml_authenticated_mode(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        envelope,
        Authentication {
            revealed: Some(&revealed),
            canonical_issuers: None,
        },
    )
}

fn revealed_inputs(
    credentials: &[crate::credential::Credential],
) -> Result<crate::statement::RevealedInputs, Error> {
    let signatures = credentials
        .iter()
        .map(|c| {
            Ok(crate::statement::RevealedSignature {
                key: c.issuer,
                message: sparq_zk::field::field_to_be_bytes_32(&c.signed_message()?),
                signature: c.credential.signature,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    crate::statement::RevealedInputs::verify(&signatures)
}

struct Authentication<'a> {
    revealed: Option<&'a crate::statement::RevealedInputs>,
    canonical_issuers: Option<&'a [Cell]>,
}
fn prover_toml_authenticated_mode(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
    authentication: Authentication<'_>,
) -> Result<String, Error> {
    let Authentication {
        revealed,
        canonical_issuers,
    } = authentication;
    let issuer_root = envelope.issuer_root;
    use crate::credential::SuiteEncoder;
    use sha2::{Digest, Sha256};
    use sparq_merkle_suite::poseidon as suite;
    use sparq_zk::{field::field_to_hex, sig};
    let cells = credentials
        .iter()
        .enumerate()
        .map(|(i, c)| c.cells_with_profile(i as u32, plan.opening_profile()))
        .collect::<Result<Vec<_>, _>>()?;
    plan.bounds()
        .check_credentials(&cells.iter().map(|c| c.quads.len()).collect::<Vec<_>>())?;
    let input_commitment = crate::credential::dataset_commitment(credentials, presentation_salt)?;
    let statement = crate::statement::Statement {
        request_digest,
        input_commitment,
        signature_mode: u8::from(revealed.is_none()),
        result: trace.result.clone(),
    };
    let layout = crate::statement::Layout::new(plan);
    let words = if let Some(revealed) = revealed {
        layout.reconstruct_revealed(plan, &statement, envelope, revealed)?
    } else {
        layout.reconstruct_authenticated_with_envelope(plan, &statement, envelope)?
    };
    let cap = plan.capacity();
    let capacities = plan.credential_quad_capacities();
    let base = prover_toml_with_scan_dataset(
        plan,
        &cells,
        &SuiteEncoder,
        trace,
        request_digest,
        presentation_salt,
        || {
            let mut scan_dataset = padded_scan_dataset(&cells, capacities.iter().copied());
            if let Some(issuers) = canonical_issuers {
                let predicate = SuiteEncoder.encode(
                    &oxrdf::NamedNode::new_unchecked(crate::credential::VERIFIED_ISSUER).into(),
                )?;
                for c in 0..cap.credentials as usize {
                    scan_dataset.push(if let Some(&issuer) = issuers.get(c) {
                        Some([
                            crate::credential::credential_graph(request_digest, c as u32)?,
                            predicate,
                            issuer,
                        ])
                    } else {
                        None
                    });
                }
            }
            Ok(scan_dataset)
        },
    )?;
    let opening = plan.opening_profile().limits();
    let mut raw = vec![];
    let mut proofs = vec![];
    for (c, &capacity) in capacities.iter().enumerate() {
        let mut quads = vec![];
        if let Some(credential) = credentials.get(c) {
            if canonical_issuers.is_none() && credential.issuer_root()? != issuer_root {
                return Err(Error::Invalid("issuer membership root".into()));
            }
            for q in credential.quads()? {
                let mut object =
                    term_opening_with_capacity(Some(&q.object), opening.object as usize)?;
                object.as_object_mut().unwrap().remove("bound");
                quads.push(serde_json::json!({
                    "subject":resource_digest(Some(&oxrdf::Term::from(q.subject)))?,
                    "predicate":resource_digest(Some(&oxrdf::Term::NamedNode(q.predicate)))?,
                    "object":object,
                }));
            }
            let config: [u8; 32] =
                Sha256::digest(credential.credential.proof_config.as_bytes()).into();
            let w = sig::in_circuit_witness(
                &credential.issuer,
                &credential.signed_message()?,
                &credential.credential.signature,
            )
            .ok_or_else(|| Error::Invalid("suite signature witness".into()))?;
            let salts = (0..capacity)
                .map(|i| {
                    field_to_hex(&if i < credential.credential.count {
                        suite::leaf_salt(credential.credential.salt, i)
                    } else {
                        sparq_merkle_suite::Fr::from(0u64)
                    })
                })
                .collect::<Vec<_>>();
            let metadata = credential.metadata()?;
            let created = metadata_opening(
                metadata.created.as_ref(),
                crate::credential::CREATION_CAPACITY,
            )?;
            let method = resource_opening(
                metadata.verification_method.as_ref(),
                crate::credential::METHOD_CAPACITY,
            )?;
            let mut proof = serde_json::json!({"created":created,"method":method,"salt":field_to_hex(&credential.credential.salt),"salts":salts,
            "config":[u128::from_be_bytes(config[..16].try_into().unwrap()).to_string(),u128::from_be_bytes(config[16..].try_into().unwrap()).to_string()],
            "pk":{"x":field_to_hex(&w.pk_x),"y":field_to_hex(&w.pk_y)},"r":{"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)},
            "s":field_to_hex(&w.s),"e":field_to_hex(&w.e),"e_k":field_to_hex(&w.e_k),"issuer_index":credential.issuer_index,
            "issuer_siblings":({
                let mut path=credential.issuer_siblings.iter().map(field_to_hex).collect::<Vec<_>>();path.resize(4,"0".into());path
            })});
            if revealed.is_some() {
                proof["r"] = serde_json::json!({"x":"0","y":"0"});
                for name in ["s", "e", "e_k"] {
                    proof[name] = serde_json::json!("0");
                }
            }
            proofs.push(proof);
        } else {
            proofs.push(serde_json::json!({"created":metadata_opening(None, crate::credential::CREATION_CAPACITY)?,
                "method":resource_opening(None, crate::credential::METHOD_CAPACITY)?,"salt":"0","salts":vec!["0";capacity as usize],"config":["0","0"],
                "pk":{"x":"0","y":"0"},"r":{"x":"0","y":"0"},"s":"0","e":"0","e_k":"0","issuer_index":0,"issuer_siblings":vec!["0";4]}));
        }
        let mut object = term_opening_with_capacity(None, opening.object as usize)?;
        object.as_object_mut().unwrap().remove("bound");
        let resource = resource_digest(None)?;
        quads.resize(
            capacity as usize,
            serde_json::json!({"subject":resource,"predicate":resource,"object":object}),
        );
        raw.push(serde_json::Value::Array(quads));
    }
    let mut out = String::new();
    for line in base.lines() {
        if line.starts_with("credentials = ") {
            for (c, credential) in raw.iter().enumerate() {
                writeln!(out, "credential_{c} = {}", json_toml(credential)).unwrap();
            }
        } else if line.starts_with("input_commitment = ") {
            writeln!(
                out,
                "input_commitment = {}",
                array(input_commitment.iter().map(u8::to_string))
            )
            .unwrap();
        } else if line.starts_with("signature_mode = ") {
            writeln!(out, "signature_mode = {}", statement.signature_mode).unwrap();
        } else if line.starts_with("statement_binding = ") {
            writeln!(out, "statement_binding = {}", field(*words.last().unwrap())).unwrap();
        } else {
            writeln!(out, "{line}").unwrap();
        }
    }
    writeln!(out, "issuer_root = {}", field(issuer_root)).unwrap();
    writeln!(out, "issuer_depth = {}", envelope.issuer_depth).unwrap();
    writeln!(out, "holder_required = {}", envelope.holder_required).unwrap();
    writeln!(
        out,
        "holder_issuers_present = {}",
        envelope.holder_issuers_present
    )
    .unwrap();
    writeln!(
        out,
        "holder_issuer_count = {}",
        envelope.holder_issuer_count
    )
    .unwrap();
    writeln!(
        out,
        "holder_issuers = {}",
        array(envelope.holder_issuers.iter().copied().map(field))
    )
    .unwrap();
    writeln!(out, "status_required = {}", envelope.status_required).unwrap();
    writeln!(
        out,
        "allow_missing_status = {}",
        envelope.allow_missing_status
    )
    .unwrap();
    writeln!(out, "snapshot_root = {}", field(envelope.snapshot_root)).unwrap();
    writeln!(out, "snapshot_depth = {}", envelope.snapshot_depth).unwrap();
    writeln!(out, "validity_required = {}", envelope.validity_required).unwrap();
    writeln!(out, "now = {}", envelope.now).unwrap();
    writeln!(out, "freshness_required = {}", envelope.freshness_required).unwrap();
    writeln!(out, "earliest_created = {}", envelope.earliest_created).unwrap();
    let (count, messages, keys) = if let Some(revealed) = revealed {
        revealed.words(plan)?
    } else {
        (
            0,
            vec![[0; 32]; cap.credentials as usize],
            vec![[0; 32]; cap.credentials as usize],
        )
    };
    writeln!(out, "revealed_count = {count}").unwrap();
    writeln!(
        out,
        "revealed_messages = {}",
        array(messages.into_iter().map(field))
    )
    .unwrap();
    writeln!(
        out,
        "revealed_keys = {}",
        array(keys.into_iter().map(field))
    )
    .unwrap();
    for (c, proof) in proofs.iter().enumerate() {
        writeln!(out, "credential_proof_{c} = {}", json_toml(proof)).unwrap();
    }
    let mut quad_bounds = plan.credential_quad_bounds();
    quad_bounds.resize(cap.credentials as usize, 0);
    writeln!(
        out,
        "credential_quad_bounds = {}",
        array(quad_bounds.iter().map(u32::to_string))
    )
    .unwrap();
    // The table encoding is the binding, so no private location selects the source.
    // Include request literals as VALUES can also feed a value-consuming operator.
    let mut literals = Vec::new();
    let mut public_literals = Vec::new();
    for credential in credentials {
        for quad in credential.quads()? {
            if let oxrdf::Term::Literal(_) = &quad.object {
                literals.push((SuiteEncoder.encode(&quad.object)?, quad.object));
            }
        }
    }
    for parameter in plan.parameters() {
        if let Parameter::Term(term @ oxrdf::Term::Literal(_)) = parameter {
            public_literals.push(SuiteEncoder.encode(term)?);
        }
    }
    let string_datatype = SuiteEncoder
        .encode(&oxrdf::Literal::new_simple_literal("").into())?
        .dt;
    for (i, node) in plan.nodes().iter().enumerate() {
        if let Operator::Filter(input, expr) = &node.operator {
            let input = *input as usize;
            let columns = crate::codegen::read_columns(expr, &plan.nodes()[input].output.variables);
            if columns.is_empty() {
                continue;
            }
            let mut reads = Vec::new();
            for r in 0..plan.nodes()[input].output.capacity as usize {
                let mut row = Vec::new();
                for &column in &columns {
                    let cell = trace.tables[input].rows.get(r).map(|row| row[column]);
                    let term = if let Some(cell) = cell.filter(|c| {
                        c.bound
                            && c.kind == 3
                            && !public_literals.contains(c)
                            && !(crate::codegen::identity_string_reads(expr)
                                && c.dt == string_datatype)
                    }) {
                        Some(match literals.iter().find(|(c, _)| *c == cell) {
                            Some((_, term)) => term.clone(),
                            None => crate::witness_closed::computed(cell).map_err(|e| match e {
                                Error::Capacity(_) => e,
                                _ => Error::Invalid("expression read opening".into()),
                            })?,
                        })
                    } else {
                        None
                    };
                    row.push(value_read_opening(term.as_ref(), opening.object as usize)?);
                }
                reads.push(serde_json::Value::Array(row));
            }
            writeln!(
                out,
                "reads_{i} = {}",
                json_toml(&serde_json::Value::Array(reads))
            )
            .unwrap();
        }
    }
    Ok(out)
}

// Identity-only source terms do not need the narrower scalar parser profile.
// Admit those bounds only when a private lexical opening is consumed as a value.
fn value_read_opening(
    term: Option<&oxrdf::Term>,
    capacity: usize,
) -> Result<serde_json::Value, Error> {
    if let Some(oxrdf::Term::Literal(literal)) = term {
        match literal.datatype().as_str() {
            "http://www.w3.org/2001/XMLSchema#string" => {}
            "http://www.w3.org/2001/XMLSchema#integer"
            | "http://www.w3.org/2001/XMLSchema#decimal"
            | "http://www.w3.org/2001/XMLSchema#date"
            | "http://www.w3.org/2001/XMLSchema#dateTime"
            | "http://www.w3.org/2001/XMLSchema#boolean" => {
                if literal.value().len() > crate::credential::SCALAR_CAPACITY {
                    return Err(Error::Capacity("read scalar lexical bytes".into()));
                }
            }
            _ => return Err(Error::Unsupported("N0 credential value datatype".into())),
        }
    }
    term_opening_with_capacity(term, capacity)
}

/// A credential subject or predicate as `{kind, framed}`: the suite's framed term
/// digest, which the signed root binds; padding is all zero.
fn resource_digest(term: Option<&oxrdf::Term>) -> Result<serde_json::Value, Error> {
    let Some(term) = term else {
        return Ok(serde_json::json!({"kind":0,"framed":"0"}));
    };
    let kind = match term {
        oxrdf::Term::NamedNode(_) => 1,
        oxrdf::Term::BlankNode(_) => 2,
        _ => return Err(Error::Invalid("credential resource term".into())),
    };
    let framed = sparq_merkle_suite::poseidon::resource_digest(term)
        .map_err(|_| Error::Invalid("credential resource digest".into()))?;
    Ok(serde_json::json!({"kind":kind,"framed":sparq_zk::field::field_to_hex(&framed)}))
}
fn resource_opening(
    term: Option<&oxrdf::Term>,
    capacity: usize,
) -> Result<serde_json::Value, Error> {
    let mut raw = term_opening_with_capacity(term, capacity)?;
    let fields = raw.as_object_mut().unwrap();
    for key in [
        "bound",
        "datatype",
        "datatype_len",
        "language",
        "language_len",
        "key",
    ] {
        fields.remove(key);
    }
    Ok(raw)
}
fn metadata_opening(
    term: Option<&oxrdf::Term>,
    capacity: usize,
) -> Result<serde_json::Value, Error> {
    let mut opening = term_opening_with_capacity(term, capacity)?;
    if term.is_none() {
        opening["bound"] = serde_json::json!(false);
    }
    Ok(opening)
}
fn term_opening_with_capacity(
    term: Option<&oxrdf::Term>,
    capacity: usize,
) -> Result<serde_json::Value, Error> {
    let (kind, text, dt, lang, key) = match term {
        None => (0, "", "", "", [0; 31]),
        Some(oxrdf::Term::NamedNode(n)) => (1, n.as_str(), "", "", [0; 31]),
        Some(oxrdf::Term::BlankNode(n)) => (2, n.as_str(), "", "", [0; 31]),
        Some(oxrdf::Term::Literal(l)) => (
            3,
            l.value(),
            l.datatype().as_str(),
            l.language().unwrap_or(""),
            sparq_merkle_suite::comparison_key(l),
        ),
        Some(oxrdf::Term::Triple(_)) => return Err(Error::Excluded("triple term".into())),
    };
    let pad = |text: &str, capacity: usize| -> Result<Vec<u8>, Error> {
        if text.len() > capacity {
            return Err(Error::Capacity("term opening bytes".into()));
        }
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(capacity, 0);
        Ok(bytes)
    };
    Ok(
        serde_json::json!({"bound":term.is_some(),"kind":kind,"lexical":pad(text,capacity)?,"lexical_len":text.len(),
        "datatype":pad(dt,crate::credential::DATATYPE_CAPACITY)?,"datatype_len":dt.len(),"language":pad(lang,crate::credential::LANGUAGE_CAPACITY)?,"language_len":lang.len(),"key":key.as_slice()}),
    )
}
fn json_toml(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(values) => array(values.iter().map(json_toml)),
        serde_json::Value::Object(values) => format!(
            "{{ {} }}",
            values
                .iter()
                .map(|(k, v)| format!("{k} = {}", json_toml(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => value.to_string(),
    }
}

#[path = "witness_envelope.rs"]
mod envelope;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use envelope::{authenticated_issuers, authorize, render_authorized, CanonicalInput};
pub use envelope::{
    prover_toml_enveloped, CredentialEvidence, EnvelopeInput, EnvelopeWitness, HolderWitness,
    StatusWitness,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capacity::Bounds,
        credential::SuiteEncoder,
        interp::{evaluate, SAME_TERM_CHECKS},
        lower::lower,
    };
    use oxrdf::NamedNode;

    #[test]
    fn operator_witness_without_a_scan_skips_dataset_build() {
        let plan = lower(
            "ASK {}",
            Bounds {
                credentials: 1,
                quads_per_credential: 16,
                rows_per_table: 2,
            },
        )
        .unwrap();
        let term = SuiteEncoder
            .encode(&NamedNode::new_unchecked("urn:term").into())
            .unwrap();
        let credentials = [CredentialCells {
            quads: vec![[term, term, term, Cell::default()]],
        }];
        let trace = evaluate(&plan, &credentials, &SuiteEncoder).unwrap();
        let before = SCAN_DATASET_BUILDS.get();
        let input =
            prover_toml(&plan, &credentials, &SuiteEncoder, &trace, [7; 32], [3; 32]).unwrap();
        assert_eq!(SCAN_DATASET_BUILDS.get(), before);
        assert!(input.lines().any(|line| line == "result = true"));
        assert!(!input
            .lines()
            .any(|line| line.starts_with("target_") || line.starts_with("source_")));
    }

    #[test]
    fn distinct_scan_bindings_only_compare_the_pattern_for_each_triple() {
        let plan = lower(
            "SELECT ?s ?o WHERE {?s <urn:p> ?o}",
            Bounds {
                credentials: 1,
                quads_per_credential: 64,
                rows_per_table: 32,
            },
        )
        .unwrap();
        let encode = |iri| {
            SuiteEncoder
                .encode(&NamedNode::new_unchecked(iri).into())
                .unwrap()
        };
        let predicate = encode("urn:p");
        let object = encode("urn:o");
        let mut quads = Vec::new();
        for value in (1..=32).rev() {
            let mut subject = encode("urn:s");
            subject.enc[31] = value;
            quads.push([subject, predicate, object, Cell::default()]);
        }
        quads.push(quads[0]);
        let credentials = [CredentialCells { quads }];
        let dataset = padded_scan_dataset(&credentials, [64]);
        let trace = evaluate(&plan, &credentials, &SuiteEncoder).unwrap();
        let mut hints = String::new();
        let before = SAME_TERM_CHECKS.get();
        append_scan_hints(&plan, &SuiteEncoder, &trace, &dataset, &mut hints).unwrap();
        assert_eq!(SAME_TERM_CHECKS.get() - before, 33);
        let source = array((0..32).rev().map(|i| i.to_string()));
        let source_line = format!("source_0 = {source}");
        assert!(hints.lines().any(|line| line == source_line));
    }
}

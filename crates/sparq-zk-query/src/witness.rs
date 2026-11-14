//! Render the interpreter's complete table trace for generated-package execution.

use crate::{
    interp::{Cell, CredentialCells, EncodeTerm, ResultValue, Trace},
    plan::{Operator, Parameter, Plan, ResultForm, Schema},
    Error,
};
use std::fmt::Write;

/// A research execution input. The method layer supplies its stored request digest.
pub fn prover_toml(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
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
    Ok(out)
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

/// Render complete lexical openings, suite signatures and issuer membership.
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

/// Encode the stored envelope inputs alongside the authenticated witness.
pub fn prover_toml_authenticated_with_envelope(
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
        None,
    )
}

/// Render the revealed-signature circuit mode after native signature verification.
pub fn prover_toml_revealed(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
) -> Result<String, Error> {
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
    let revealed = crate::statement::RevealedInputs::verify(&signatures)?;
    prover_toml_authenticated_mode(
        plan,
        credentials,
        trace,
        request_digest,
        presentation_salt,
        envelope,
        Some(&revealed),
    )
}

fn prover_toml_authenticated_mode(
    plan: &Plan,
    credentials: &[crate::credential::Credential],
    trace: &Trace,
    request_digest: [u8; 32],
    presentation_salt: [u8; 32],
    envelope: &crate::statement::EnvelopeInputs,
    revealed: Option<&crate::statement::RevealedInputs>,
) -> Result<String, Error> {
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
    let base = prover_toml(
        plan,
        &cells,
        &SuiteEncoder,
        trace,
        request_digest,
        presentation_salt,
    )?;
    let cap = plan.capacity();
    let opening = plan.opening_profile().limits();
    let mut raw = vec![];
    let mut proofs = vec![];
    let capacities = plan.credential_quad_capacities();
    for (c, &capacity) in capacities.iter().enumerate() {
        let mut quads = vec![];
        if let Some(credential) = credentials.get(c) {
            if credential.issuer_root()? != issuer_root {
                return Err(Error::Invalid("issuer membership root".into()));
            }
            for q in credential.quads()? {
                let mut object =
                    term_opening_with_capacity(Some(&q.object), opening.object as usize)?;
                object.as_object_mut().unwrap().remove("bound");
                quads.push(serde_json::json!({
                    "subject":resource_opening(Some(&oxrdf::Term::from(q.subject)), opening.resource as usize)?,
                    "predicate":resource_opening(Some(&oxrdf::Term::NamedNode(q.predicate)), opening.resource as usize)?,
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
                "issuer_siblings":credential.issuer_siblings.iter().map(field_to_hex).collect::<Vec<_>>()});
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
        let resource = resource_opening(None, opening.resource as usize)?;
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
                    let term = if let Some(cell) =
                        cell.filter(|c| c.bound && c.kind == 3 && !public_literals.contains(c))
                    {
                        Some(
                            &literals
                                .iter()
                                .find(|(c, _)| *c == cell)
                                .ok_or_else(|| Error::Invalid("expression read opening".into()))?
                                .1,
                        )
                    } else {
                        None
                    };
                    row.push(term_opening_with_capacity(term, opening.object as usize)?);
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

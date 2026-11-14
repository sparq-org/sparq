//! Graph-profile lexical-opening adapter; lane E replaces the envelope stand-in.
//! The private lexical capacity is part of the closed profile's plan shape.
use crate::{
    credential::{Credential, SuiteEncoder},
    interp::Trace,
    plan::Plan,
    Error,
};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::poseidon as suite;
use sparq_zk::{field::field_to_hex, sig};
use std::fmt::Write;
fn array(v: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", v.into_iter().collect::<Vec<_>>().join(","))
}
fn word(w: [u8; 32]) -> String {
    format!(
        "\"0x{}\"",
        w.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
fn toml(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Array(v) => array(v.iter().map(toml)),
        serde_json::Value::Object(v) => format!(
            "{{{}}}",
            v.iter()
                .map(|(k, v)| format!("{k}={}", toml(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => v.to_string(),
    }
}
fn opening(term: Option<&oxrdf::Term>) -> Result<serde_json::Value, Error> {
    use oxrdf::Term;
    let (kind, text, dt, lang, key) = match term {
        None => (0, "", "", "", [0; 31]),
        Some(Term::NamedNode(n)) => (1, n.as_str(), "", "", [0; 31]),
        Some(Term::BlankNode(n)) => (2, n.as_str(), "", "", [0; 31]),
        Some(Term::Literal(l)) => (
            3,
            l.value(),
            l.datatype().as_str(),
            l.language().unwrap_or(""),
            sparq_merkle_suite::comparison_key(l),
        ),
        _ => return Err(Error::Excluded("triple term".into())),
    };
    let pad = |s: &str, n: usize| -> Result<Vec<u8>, Error> {
        if s.len() > n {
            return Err(Error::Capacity("graph-profile lexical opening".into()));
        }
        let mut v = s.as_bytes().to_vec();
        v.resize(n, 0);
        Ok(v)
    };
    let datatype_tag = match dt {
        "" => 0,
        "http://www.w3.org/2001/XMLSchema#integer" => 1,
        "http://www.w3.org/2001/XMLSchema#boolean" => 2,
        "http://www.w3.org/2001/XMLSchema#date" => 3,
        "http://www.w3.org/2001/XMLSchema#decimal" => 4,
        "http://www.w3.org/2001/XMLSchema#string" => 5,
        _ => {
            return Err(Error::Unsupported(
                "closed credential literal datatype".into(),
            ))
        }
    };
    if !lang.is_empty() {
        return Err(Error::Unsupported("closed credential language tag".into()));
    }
    Ok(
        serde_json::json!({"bound":true,"kind":kind,"lexical":pad(text,64)?,"lexical_len":text.len(),"datatype_tag":datatype_tag,"key":key.as_slice()}),
    )
}
fn metadata_opening(
    term: Option<&oxrdf::Term>,
    capacity: usize,
) -> Result<serde_json::Value, Error> {
    let (kind, key) = match term {
        Some(oxrdf::Term::NamedNode(_)) => (1, [0; 31]),
        Some(oxrdf::Term::Literal(l)) => (3, sparq_merkle_suite::comparison_key(l)),
        None => (0, [0; 31]),
        _ => return Err(Error::Invalid("suite metadata term".into())),
    };
    let mut v = serde_json::json!({"kind":kind,"key":key.as_slice()});
    let text = match term {
        Some(oxrdf::Term::NamedNode(n)) => n.as_str(),
        Some(oxrdf::Term::Literal(l)) => l.value(),
        None => "",
        _ => return Err(Error::Invalid("suite metadata term".into())),
    };
    if text.len() > capacity {
        return Err(Error::Capacity("suite metadata opening".into()));
    }
    let mut lexical = text.as_bytes().to_vec();
    lexical.resize(capacity, 0);
    let dt = match term {
        Some(oxrdf::Term::Literal(l)) => l.datatype().as_str(),
        _ => "",
    };
    let mut datatype = dt.as_bytes().to_vec();
    datatype.resize(128, 0);
    v.as_object_mut().unwrap().remove("datatype_tag");
    v["lexical_len"] = serde_json::json!(text.len());
    v["lexical"] = serde_json::json!(lexical);
    v["datatype"] = serde_json::json!(datatype);
    v["datatype_len"] = serde_json::json!(dt.len());
    v["language"] = serde_json::json!(vec![0; 64]);
    v["language_len"] = serde_json::json!(0);
    v["bound"] = serde_json::json!(term.is_some());
    Ok(v)
}
/// Produce the ordinary authenticated public layout and complete private openings.
pub fn prover_toml(
    plan: &Plan,
    credentials: &[Credential],
    trace: &Trace,
    digest: [u8; 32],
    salt: [u8; 32],
    issuer_root: [u8; 32],
) -> Result<String, Error> {
    crate::credential_graphs::check_issuer_registry(plan, credentials)?;
    let cells = crate::credential_graphs::cells(credentials)?;
    let input_commitment = crate::credential::dataset_commitment(credentials, salt)?;
    let statement = crate::statement::Statement {
        request_digest: digest,
        input_commitment,
        signature_mode: 1,
        result: trace.result.clone(),
    };
    let words = crate::statement::Layout::new(plan).reconstruct_authenticated(
        plan,
        &statement,
        issuer_root,
    )?;
    let base = crate::witness::prover_toml(plan, &cells, &SuiteEncoder, trace, digest, salt)?;
    let mut raw = vec![];
    let mut proofs = vec![];
    for (c, n) in plan.credential_quad_capacities().into_iter().enumerate() {
        let mut rows = vec![];
        if let Some(credential) = credentials.get(c) {
            if credential.issuer_root()? != issuer_root {
                return Err(Error::Invalid("issuer membership root".into()));
            }
            for q in credential.quads()? {
                rows.push(serde_json::json!([
                    opening(Some(&oxrdf::Term::from(q.subject)))?,
                    opening(Some(&oxrdf::Term::from(q.predicate)))?,
                    opening(Some(&q.object))?,
                    opening(None)?
                ]));
            }
            if rows.len() > n as usize {
                return Err(Error::Capacity("per-credential graph profile".into()));
            }
            let config: [u8; 32] =
                Sha256::digest(credential.credential.proof_config.as_bytes()).into();
            let metadata = credential.metadata()?;
            let created = metadata_opening(metadata.created.as_ref(), 32)?;
            let method = metadata_opening(metadata.verification_method.as_ref(), 128)?;
            let w = sig::in_circuit_witness(
                &credential.issuer,
                &credential.signed_message()?,
                &credential.credential.signature,
            )
            .ok_or_else(|| Error::Invalid("suite signature witness".into()))?;
            let salts = (0..n)
                .map(|i| {
                    field_to_hex(&if i < credential.credential.count {
                        suite::leaf_salt(credential.credential.salt, i)
                    } else {
                        sparq_merkle_suite::Fr::from(0u64)
                    })
                })
                .collect::<Vec<_>>();
            proofs.push(serde_json::json!({"created":created,"method":method,"salt":field_to_hex(&credential.credential.salt),"salts":salts,"config":[u128::from_be_bytes(config[..16].try_into().unwrap()).to_string(),u128::from_be_bytes(config[16..].try_into().unwrap()).to_string()],"pk":{"x":field_to_hex(&w.pk_x),"y":field_to_hex(&w.pk_y)},"r":{"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)},"s":field_to_hex(&w.s),"e":field_to_hex(&w.e),"e_k":field_to_hex(&w.e_k),"issuer_index":credential.issuer_index,"issuer_siblings":credential.issuer_siblings.iter().map(field_to_hex).collect::<Vec<_>>()}));
        } else {
            proofs.push(serde_json::json!({"created":metadata_opening(None,32)?,"method":metadata_opening(None,128)?,"salt":"0","salts":vec!["0";n as usize],"config":["0","0"],"pk":{"x":"0","y":"0"},"r":{"x":"0","y":"0"},"s":"0","e":"0","e_k":"0","issuer_index":0,"issuer_siblings":vec!["0";4]}));
        }
        let mut padding = opening(None)?;
        padding["bound"] = serde_json::Value::Bool(false);
        rows.resize(
            n as usize,
            serde_json::json!([padding, padding, padding, padding]),
        );
        raw.push(serde_json::Value::Array(rows));
    }
    let mut out = String::new();
    for line in base.lines() {
        if line.starts_with("credentials = ") {
            for (c, r) in raw.iter().enumerate() {
                writeln!(out, "credential_{c} = {}", toml(r)).unwrap();
            }
        } else if line.starts_with("input_commitment = ") {
            writeln!(
                out,
                "input_commitment = {}",
                array(input_commitment.iter().map(u8::to_string))
            )
            .unwrap();
        } else if line.starts_with("signature_mode = ") {
            writeln!(out, "signature_mode = 1").unwrap();
        } else if line.starts_with("statement_binding = ") {
            writeln!(out, "statement_binding = {}", word(*words.last().unwrap())).unwrap();
        } else {
            writeln!(out, "{line}").unwrap();
        }
    }
    writeln!(out, "issuer_root = {}", word(issuer_root)).unwrap();
    out.push_str("issuer_depth = 4\nholder_required = false\nholder_issuers_present = false\nholder_issuer_count = 0\nholder_issuers = [\"0\",\"0\",\"0\",\"0\"]\nstatus_required = false\nallow_missing_status = false\nsnapshot_root = \"0\"\nsnapshot_depth = 0\nvalidity_required = false\nnow = 0\nfreshness_required = false\nearliest_created = 0\n");
    for (c, p) in proofs.iter().enumerate() {
        writeln!(out, "credential_proof_{c} = {}", toml(p)).unwrap();
    }
    let mut bounds = plan.credential_quad_bounds();
    bounds.resize(plan.capacity().credentials as usize, 0);
    writeln!(
        out,
        "credential_quad_bounds = {}",
        array(bounds.iter().map(u32::to_string))
    )
    .unwrap();
    Ok(out)
}

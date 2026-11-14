//! Closed graph-profile witness adapter over the shared authenticated ABI.
use crate::{
    credential::{Credential, SuiteEncoder},
    interp::{Cell, EncodeTerm, Trace},
    plan::{Parameter, Plan},
    Error,
};
use oxrdf::{Literal, NamedNode, Term};
use std::fmt::Write;
fn toml(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Array(v) => {
            format!("[{}]", v.iter().map(toml).collect::<Vec<_>>().join(","))
        }
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
fn raw(term: Option<&Term>, len: usize) -> Result<serde_json::Value, Error> {
    let literal = match term {
        Some(Term::Literal(l)) => Some(l),
        _ => None,
    };
    let lexical = literal.map_or("", Literal::value);
    let datatype = literal.map_or("", |l| l.datatype().as_str());
    let language = literal.and_then(Literal::language).unwrap_or("");
    fn pad(text: &str, n: usize) -> Result<Vec<u8>, Error> {
        if text.len() > n {
            return Err(Error::Capacity("closed read lexical opening".into()));
        }
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(n, 0);
        Ok(bytes)
    }
    let key = literal.map_or([0; 31], sparq_merkle_suite::comparison_key);
    Ok(
        serde_json::json!({"bound":literal.is_some(),"kind":if literal.is_some(){3}else{0},
        "lexical":pad(lexical,len)?,"lexical_len":lexical.len(),"datatype":pad(datatype,41)?,
        "datatype_len":datatype.len(),"language":pad(language,1)?,"language_len":language.len(),"key":key.as_slice()}),
    )
}
fn computed(cell: Cell) -> Result<Term, Error> {
    for value in [false, true] {
        let term = Term::Literal(Literal::from(value));
        if SuiteEncoder.encode(&term)? == cell {
            return Ok(term);
        }
    }
    for decimal in [false, true] {
        let dt = NamedNode::new_unchecked(format!(
            "http://www.w3.org/2001/XMLSchema#{}",
            if decimal { "decimal" } else { "integer" }
        ));
        if cell.dt
            == SuiteEncoder
                .encode(&Literal::new_typed_literal("0", dt.clone()).into())?
                .dt
        {
            let mut key = cell.key;
            key[1] = 0;
            let text = crate::interp_closed_group::key_lexical(key, decimal)?;
            let term = Term::Literal(Literal::new_typed_literal(text, dt));
            if SuiteEncoder.encode(&term)? == cell {
                return Ok(term);
            }
        }
    }
    Err(Error::Invalid("closed value-read opening".into()))
}
/// Produce complete suite openings and lexical reads for every closed value consumer.
/// This delegates the shared ABI, opening profiles and public statement to lane B.
pub fn prover_toml(
    plan: &Plan,
    credentials: &[Credential],
    trace: &Trace,
    digest: [u8; 32],
    salt: [u8; 32],
    issuer_root: [u8; 32],
) -> Result<String, Error> {
    crate::credential_graphs::check_issuer_registry(plan, credentials)?;
    let mut out = crate::witness::prover_toml_authenticated(
        plan,
        credentials,
        trace,
        digest,
        salt,
        issuer_root,
    )?;
    let mut literals = vec![];
    for c in credentials {
        for q in c.quads()? {
            if matches!(q.object, Term::Literal(_)) {
                literals.push((SuiteEncoder.encode(&q.object)?, q.object));
            }
        }
    }
    let parameters = plan
        .parameters()
        .iter()
        .filter_map(|p| match p {
            Parameter::Term(t) => Some(SuiteEncoder.encode(t)),
            _ => None,
        })
        .collect::<Result<Vec<_>, _>>()?;
    for i in 0..plan.nodes().len() {
        for site in crate::reads_closed::sites(plan, i) {
            let mut rows = vec![];
            for r in 0..plan.nodes()[site.input as usize].output.capacity as usize {
                let mut row = vec![];
                for &column in &site.columns {
                    let cell = trace.tables[site.input as usize]
                        .rows
                        .get(r)
                        .map(|row| row[column]);
                    let term = if let Some(cell) =
                        cell.filter(|c| c.bound && c.kind == 3 && !parameters.contains(c))
                    {
                        Some(match literals.iter().find(|(c, _)| *c == cell) {
                            Some((_, t)) => t.clone(),
                            None => computed(cell)?,
                        })
                    } else {
                        None
                    };
                    row.push(raw(
                        term.as_ref(),
                        plan.opening_profile().limits().object as usize,
                    )?);
                }
                rows.push(serde_json::Value::Array(row));
            }
            writeln!(
                out,
                "{} = {}",
                site.name(i),
                toml(&serde_json::Value::Array(rows))
            )
            .unwrap();
        }
    }
    append_join_pairs(plan, trace, &mut out)?;
    Ok(out)
}

fn append_join_pairs(plan: &Plan, trace: &Trace, out: &mut String) -> Result<(), Error> {
    for (i, node) in plan.nodes().iter().enumerate() {
        if let crate::plan::Operator::Closed(crate::plan_closed::ClosedOperator::Join {
            left,
            right,
        }) = node.operator
        {
            let pairs = crate::interp_closed_join::evaluate(
                &trace.tables[left as usize],
                &plan.nodes()[left as usize].output.variables,
                &trace.tables[right as usize],
                &plan.nodes()[right as usize].output.variables,
                &node.output.variables,
                node.output.capacity as usize,
            )?;
            let mut pairs = pairs.into_iter().map(|(_, p)| p).collect::<Vec<_>>();
            pairs.resize(node.output.capacity as usize, [0, 0]);
            writeln!(
                out,
                "closed_pairs_{i} = {}",
                toml(&serde_json::json!(pairs))
            )
            .unwrap();
        }
    }
    Ok(())
}
/// Supplied-cell operator fixture, including private join inverse maps.
/// This entry point establishes no credential authentication.
pub fn operator_prover_toml(
    plan: &Plan,
    credentials: &[crate::interp::CredentialCells],
    encoder: &impl EncodeTerm,
    trace: &Trace,
    digest: [u8; 32],
    salt: [u8; 32],
) -> Result<String, Error> {
    let mut out = crate::witness::prover_toml(plan, credentials, encoder, trace, digest, salt)?;
    append_join_pairs(plan, trace, &mut out)?;
    Ok(out)
}

//! Render the interpreter's complete table trace for generated-package execution.

use crate::{
    interp::{Cell, CredentialCells, EncodeTerm, ResultValue, Trace},
    plan::{Parameter, Plan, ResultForm, Schema},
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
) -> Result<String, Error> {
    if trace.tables.len() != plan.nodes().len() {
        return Err(Error::Invalid("trace node count".into()));
    }
    let mut out = String::new();
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
    let parameters = plan
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
    writeln!(out, "parameters = {}", array(parameters)).unwrap();
    writeln!(
        out,
        "indices = {}",
        array(
            plan.parameters()
                .iter()
                .filter_map(|p| if let Parameter::Index(n) = p {
                    Some(n.to_string())
                } else {
                    None
                })
        )
    )
    .unwrap();
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
        .map(|r| array(r.iter().copied().map(cell)))
        .collect();
    padded.resize(
        schema.capacity as usize,
        array(std::iter::repeat_n(
            cell(Cell::default()),
            schema.variables.len(),
        )),
    );
    Ok(format!(
        "{{ len = {}, rows = {} }}",
        rows.len(),
        array(padded)
    ))
}

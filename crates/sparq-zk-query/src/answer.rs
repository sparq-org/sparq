//! RDF answers reconstructed into the stored plan's public statement.

use crate::{
    interp::{Cell, EncodeTerm, ResultValue},
    plan::Plan,
    statement::number,
    Error,
};
use oxrdf::{BlankNode, Term};

/// Released RDF terms, preserving row order, duplicates and unbound columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Ask(bool),
    Select {
        variables: Vec<String>,
        rows: Vec<Vec<Option<Term>>>,
    },
}
impl Answer {
    pub fn encode(&self, encoder: &impl EncodeTerm) -> Result<ResultValue, Error> {
        Ok(match self {
            Self::Ask(value) => ResultValue::Ask(*value),
            Self::Select { variables, rows } => ResultValue::Select {
                variables: variables.clone(),
                rows: rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|term| match term {
                                None => Ok(Cell::default()),
                                Some(Term::BlankNode(b)) => {
                                    let label = b
                                        .as_str()
                                        .strip_prefix('b')
                                        .and_then(|s| s.parse::<u32>().ok())
                                        .filter(|n| *n > 0)
                                        .ok_or_else(|| {
                                            Error::Invalid("result blank-node label".into())
                                        })?;
                                    if b.as_str() != format!("b{label}") {
                                        return Err(Error::Invalid(
                                            "result blank-node label".into(),
                                        ));
                                    }
                                    Ok(Cell {
                                        bound: true,
                                        kind: 2,
                                        enc: number(u64::from(label)),
                                        ..Cell::default()
                                    })
                                }
                                Some(term) => {
                                    encoder.encode(term).map(crate::interp::released_cell)
                                }
                            })
                            .collect()
                    })
                    .collect::<Result<_, Error>>()?,
            },
        })
    }
    /// Release a canonical RDF answer using the trusted plan and vocabulary.
    pub fn release(
        value: &ResultValue,
        plan: &Plan,
        vocabulary: &[Term],
        encoder: &impl EncodeTerm,
    ) -> Result<Self, Error> {
        let mut catalog = vocabulary.to_vec();
        catalog.extend([true, false].map(|value| Term::Literal(oxrdf::Literal::from(value))));
        catalog.extend(plan.parameters().iter().filter_map(|p| {
            if let crate::plan::Parameter::Term(t) = p {
                Some(t.clone())
            } else {
                None
            }
        }));
        let catalog: Vec<_> = catalog
            .into_iter()
            .map(|term| {
                encoder
                    .encode(&term)
                    .map(|cell| (crate::interp::released_cell(cell), term))
            })
            .collect::<Result<_, _>>()?;
        Ok(match value {
            ResultValue::Ask(value) => Self::Ask(*value),
            ResultValue::Select { variables, rows } => Self::Select {
                variables: variables.clone(),
                rows: rows
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|cell| {
                                if !cell.bound {
                                    return Ok(None);
                                }
                                if cell.kind == 2 {
                                    let label =
                                        u32::from_be_bytes(cell.enc[28..].try_into().unwrap());
                                    if cell.enc[..28] != [0; 28] || label == 0 {
                                        return Err(Error::Invalid(
                                            "result blank-node label".into(),
                                        ));
                                    }
                                    return Ok(Some(Term::BlankNode(
                                        BlankNode::new(format!("b{label}"))
                                            .map_err(|e| Error::Invalid(e.to_string()))?,
                                    )));
                                }
                                catalog
                                    .iter()
                                    .find(|(c, _)| c == cell)
                                    .map(|(_, t)| Some(t.clone()))
                                    .ok_or_else(|| Error::Invalid("missing RDF result term".into()))
                            })
                            .collect()
                    })
                    .collect::<Result<_, Error>>()?,
            },
        })
    }
}

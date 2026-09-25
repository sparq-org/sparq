// [GPT-6] Whole-result RDFC encoding preserves cross-row blank-node identity.
use super::{CanonicalResult, CanonicalizationPolicy};
use crate::{Rejected, RowOrder};
use oxrdf::{BlankNode, GraphName, Literal, NamedNode, Quad, Term, Triple};
use sha2::Sha256;
use sparq_canon::{
    CanonicalizationLimits, canonicalize_quads_bounded_with, issue_quads_bounded_with,
};
use sparq_engine::QueryResult;
use std::{collections::BTreeMap, fmt::Write};

fn limits(policy: &CanonicalizationPolicy) -> CanonicalizationLimits {
    CanonicalizationLimits {
        max_quads: policy.max_quads as usize,
        max_input_bytes: policy.max_input_bytes as usize,
        max_output_bytes: policy.max_output_bytes as usize,
        max_hndq_calls: policy.max_hndq_calls as usize,
        max_permutation_steps: policy.max_permutation_steps as usize,
    }
}

struct Encoding {
    quads: Vec<Quad>,
    bytes: usize,
    limits: CanonicalizationLimits,
}

impl Encoding {
    fn push(&mut self, quad: Quad) -> Result<(), Rejected> {
        if self.quads.len() >= self.limits.max_quads {
            return Err(Rejected("V3 encoded quad capacity"));
        }
        let mut count = Counter {
            bytes: self.bytes,
            limit: self.limits.max_input_bytes,
        };
        writeln!(count, "{quad} .").map_err(|_| Rejected("V3 encoded input byte capacity"))?;
        self.bytes = count.bytes;
        self.quads.push(quad);
        Ok(())
    }
}

struct Counter {
    bytes: usize,
    limit: usize,
}

impl std::fmt::Write for Counter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if text.len() > self.limit.saturating_sub(self.bytes) {
            return Err(std::fmt::Error);
        }
        self.bytes += text.len();
        Ok(())
    }
}

impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            return Err(std::io::Error::other("V3 result byte capacity"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn iri(suffix: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("urn:sparq:result-table:v3:{suffix}"))
}

fn quad(subject: &BlankNode, predicate: &str, object: impl Into<Term>) -> Quad {
    Quad::new(
        subject.clone(),
        iri(predicate),
        object,
        GraphName::DefaultGraph,
    )
}

fn validate_term(term: &Term) -> Result<(), Rejected> {
    match term {
        Term::NamedNode(_) | Term::BlankNode(_) => Ok(()),
        Term::Literal(literal) => crate::evaluate::literal(literal),
        Term::Triple(_) => Err(Rejected("V3 triple terms are not admitted")),
    }
}

pub(super) fn select(
    result: QueryResult,
    order: RowOrder,
    policy: &CanonicalizationPolicy,
) -> Result<CanonicalResult, Rejected> {
    let mut values = BTreeMap::new();
    for row in &result.rows {
        if row.len() != result.vars.len() {
            return Err(Rejected("V3 result row width"));
        }
        for term in row.iter().flatten() {
            validate_term(term)?;
            if let Term::BlankNode(node) = term {
                let index = values.len();
                values
                    .entry(node.as_str().to_owned())
                    .or_insert_with(|| BlankNode::new_unchecked(format!("value{index}")));
            }
        }
    }
    // [GPT-6] Blank-free results require no isomorphism work. Artificial row nodes
    // are introduced only when their incidence is needed to preserve value identity.
    let labels = if values.is_empty() {
        BTreeMap::new()
    } else {
        let mut encoding = Encoding {
            quads: Vec::new(),
            bytes: 0,
            limits: limits(policy),
        };
        for node in values.values() {
            encoding.push(quad(node, "type", iri("Value")))?;
        }
        for (index, row) in result.rows.iter().enumerate() {
            let row_node = BlankNode::new_unchecked(format!("row{index}"));
            encoding.push(quad(&row_node, "type", iri("Row")))?;
            if order == RowOrder::Sequence {
                encoding.push(quad(&row_node, "index", Literal::from(index as u64)))?;
            }
            for (column, term) in row.iter().enumerate() {
                let Some(term) = term else { continue };
                let value = match term {
                    Term::BlankNode(node) => Term::BlankNode(values[node.as_str()].clone()),
                    term => term.clone(),
                };
                encoding.push(quad(&row_node, &format!("column{column}"), value))?;
            }
        }
        let issued = issue_quads_bounded_with::<Sha256>(&encoding.quads, &encoding.limits)
            .map_err(|_| Rejected("V3 table canonicalization rejected"))?;
        values
            .into_iter()
            .map(|(source, encoded)| {
                let label = issued
                    .get(encoded.as_str())
                    .ok_or(Rejected("V3 canonical label missing"))?;
                Ok((source, format!("_:{label}")))
            })
            .collect::<Result<BTreeMap<_, _>, Rejected>>()?
    };
    let mut remaining = policy.max_output_bytes as usize;
    let mut charge = |size: usize| -> Result<(), Rejected> {
        remaining = remaining
            .checked_sub(size)
            .ok_or(Rejected("V3 result byte capacity"))?;
        Ok(())
    };
    let variables = result
        .vars
        .iter()
        .map(|v| {
            charge(v.as_str().len())?;
            Ok(v.as_str().to_owned())
        })
        .collect::<Result<Vec<_>, Rejected>>()?;
    let mut rows = Vec::with_capacity(result.rows.len());
    for row in result.rows {
        let mut encoded = Vec::with_capacity(row.len());
        charge(1)?;
        for term in row {
            let text = match term {
                Some(Term::BlankNode(node)) => Some(labels[&node.as_str().to_owned()].clone()),
                Some(term) => Some(term.to_string()),
                None => None,
            };
            charge(text.as_ref().map_or(1, String::len))?;
            encoded.push(text);
        }
        rows.push(encoded);
    }
    if order == RowOrder::Bag {
        rows.sort();
    }
    let result = CanonicalResult::Select {
        variables,
        order,
        rows,
    };
    check_output(&result, policy)?;
    Ok(result)
}

pub(super) fn graph(
    triples: Vec<Triple>,
    policy: &CanonicalizationPolicy,
) -> Result<CanonicalResult, Rejected> {
    let mut encoding = Encoding {
        quads: Vec::new(),
        bytes: 0,
        limits: limits(policy),
    };
    for triple in triples {
        validate_term(&triple.object)?;
        encoding.push(Quad::new(
            triple.subject,
            triple.predicate,
            triple.object,
            GraphName::DefaultGraph,
        ))?;
    }
    let ntriples = canonicalize_quads_bounded_with::<Sha256>(&encoding.quads, &encoding.limits)
        .map_err(|_| Rejected("V3 graph canonicalization rejected"))?;
    let result = CanonicalResult::Graph { ntriples };
    check_output(&result, policy)?;
    Ok(result)
}

pub(super) fn check_output(
    result: &CanonicalResult,
    policy: &CanonicalizationPolicy,
) -> Result<(), Rejected> {
    serde_json::to_writer(
        Counter {
            bytes: 0,
            limit: policy.max_output_bytes as usize,
        },
        result,
    )
    .map_err(|_| Rejected("V3 serialized result byte capacity"))
}

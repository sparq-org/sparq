// Bound input/output before RDFC and meter its HNDQ calls and permutations.
use crate::{CanonError, Digest};
use oxrdf::{GraphName, NamedOrBlankNode, Quad, Term};
use std::{collections::HashMap, fmt::Write};

/// Explicit resource limits for the standard RDFC-1.0 entry points.
///
/// The permutation limit counts candidates across all HNDQ calls.
/// These limits bound input, output and algorithmic work, not elapsed time or RSS.
#[derive(Clone, Copy, Debug)]
pub struct CanonicalizationLimits {
    pub max_quads: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Passed explicitly to the vendored RDFC total HNDQ-call counter.
    pub max_hndq_calls: usize,
    /// Maximum total permutation-loop iterations across all HNDQ calls.
    pub max_permutation_steps: usize,
}

/// Canonicalizes an RDF 1.1 dataset with preflight and library work limits.
///
/// # Errors
/// Rejects RDF 1.2 terms, excessive input/output/work and library failures.
pub fn canonicalize_quads_bounded_with<D: Digest>(
    dataset: &[Quad],
    limits: &CanonicalizationLimits,
) -> Result<String, CanonError> {
    let quads = prepare(dataset, limits)?;
    let result = crate::rdfc::canonicalize_quads_with::<D>(
        &quads,
        &crate::rdfc::CanonicalizationOptions {
            hndq_call_limit: Some(limits.max_hndq_calls),
            permutation_limit: Some(limits.max_permutation_steps),
        },
    )
    .map_err(|error| CanonError::Canonicalization(error.to_string()))?;
    if result.len() > limits.max_output_bytes {
        return Err(capacity("canonical output capacity"));
    }
    Ok(result)
}

/// Issues RDFC-1.0 blank-node labels under the same limits as canonical output.
///
/// # Errors
/// Rejects RDF 1.2 terms, excessive input/output/work and library failures.
pub fn issue_quads_bounded_with<D: Digest>(
    dataset: &[Quad],
    limits: &CanonicalizationLimits,
) -> Result<HashMap<String, String>, CanonError> {
    let quads = prepare(dataset, limits)?;
    crate::rdfc::issue_quads_with::<D>(
        &quads,
        &crate::rdfc::CanonicalizationOptions {
            hndq_call_limit: Some(limits.max_hndq_calls),
            permutation_limit: Some(limits.max_permutation_steps),
        },
    )
    .map_err(|error| CanonError::Canonicalization(error.to_string()))
}

fn capacity(message: &str) -> CanonError {
    CanonError::Canonicalization(message.into())
}

struct Document {
    text: String,
    limit: usize,
}

impl std::fmt::Write for Document {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if text.len() > self.limit.saturating_sub(self.text.len()) {
            return Err(std::fmt::Error);
        }
        self.text.push_str(text);
        Ok(())
    }
}

fn prepare(
    dataset: &[Quad],
    limits: &CanonicalizationLimits,
) -> Result<Vec<oxrdf02::Quad>, CanonError> {
    if dataset.len() > limits.max_quads {
        return Err(capacity("canonical input quad capacity"));
    }
    let mut document = Document {
        text: String::new(),
        limit: limits.max_input_bytes,
    };
    let mut related: HashMap<&str, ()> = HashMap::new();
    let mut occurrences = 0_usize;
    for quad in dataset {
        if matches!(quad.object, Term::Triple(_)) {
            return Err(CanonError::TripleTerm);
        }
        let nodes = [
            match &quad.subject {
                NamedOrBlankNode::BlankNode(node) => Some(node.as_str()),
                _ => None,
            },
            match &quad.object {
                Term::BlankNode(node) => Some(node.as_str()),
                _ => None,
            },
            match &quad.graph_name {
                GraphName::BlankNode(node) => Some(node.as_str()),
                _ => None,
            },
        ];
        // Keep the occurrence count for the canonical-label output allowance.
        for node in nodes.iter().flatten() {
            occurrences = occurrences
                .checked_add(1)
                .ok_or_else(|| capacity("canonical input size overflow"))?;
            related.entry(node).or_default();
        }
        let written = match &quad.graph_name {
            GraphName::DefaultGraph => {
                writeln!(
                    document,
                    "{} {} {} .",
                    quad.subject, quad.predicate, quad.object
                )
            }
            graph => writeln!(
                document,
                "{} {} {} {} .",
                quad.subject, quad.predicate, quad.object, graph
            ),
        };
        written.map_err(|_| capacity("canonical input byte capacity"))?;
    }
    // Every canonical identifier is c14n followed by an index less than the
    // number of input blank nodes. Allow its entire length as extra bytes on
    // every occurrence (source identifier bytes are already in document).
    let label_allowance = related.len().max(1).ilog10() as usize + 5;
    let output_bound = occurrences
        .checked_mul(label_allowance)
        .and_then(|extra| document.text.len().checked_add(extra))
        .ok_or_else(|| capacity("canonical output size overflow"))?;
    if output_bound > limits.max_output_bytes {
        return Err(capacity("canonical output capacity"));
    }
    crate::parse_02(&document.text)
}

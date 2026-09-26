// [GPT-6] Bound the library's uncounted permutation loops before invoking RDFC.
use crate::{CanonError, Digest};
use oxrdf::{GraphName, NamedOrBlankNode, Quad, Term};
use std::{collections::HashMap, fmt::Write};

/// Explicit resource limits for the standard RDFC-1.0 entry points.
///
/// The permutation limit is a conservative upper bound, not a measured cost.
/// It can reject an easy graph whose first-degree hashes would resolve quickly.
/// These limits bound input, output and algorithmic work, not elapsed time or RSS.
#[derive(Clone, Copy, Debug)]
pub struct CanonicalizationLimits {
    pub max_quads: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Passed explicitly to rdf-canon's total HNDQ-call counter.
    pub max_hndq_calls: usize,
    /// Maximum precomputed upper bound on all permutation-loop iterations.
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
    let result = rdf_canon::canonicalize_quads_with::<D>(
        &quads,
        &rdf_canon::CanonicalizationOptions {
            hndq_call_limit: Some(limits.max_hndq_calls),
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
    rdf_canon::issue_quads_with::<D>(
        &quads,
        &rdf_canon::CanonicalizationOptions {
            hndq_call_limit: Some(limits.max_hndq_calls),
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
    let mut related: HashMap<&str, usize> = HashMap::new();
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
        // Count occurrences, not distinct neighbours. rdf-canon's Hn lists can
        // repeat a related node across quads or graph/subject/object positions.
        // Counting a quad again for repeated owner positions only overestimates.
        for node in nodes.iter().flatten() {
            occurrences = occurrences
                .checked_add(1)
                .ok_or_else(|| capacity("canonical input size overflow"))?;
            let degree = related.entry(node).or_default();
            *degree = degree
                .checked_add(
                    nodes
                        .iter()
                        .flatten()
                        .filter(|other| *other != node)
                        .count(),
                )
                .ok_or_else(|| capacity("canonical related-node overflow"))?;
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
    let degree = related.values().copied().max().unwrap_or(0);
    let mut factorial = 1_usize;
    for value in 2..=degree {
        factorial = factorial
            .checked_mul(value)
            .filter(|bound| *bound <= limits.max_permutation_steps)
            .ok_or_else(|| capacity("canonical permutation capacity"))?;
    }
    // In rdf-canon 0.15.3, canon.rs hash_n_degree_quads checks the global
    // call counter before its body. Hn has at most degree groups, each with
    // <= degree occurrences. Its sole permutation loop has <= degree! entries
    // per group, including entries skipped before recursive calls are counted.
    // Thus all calls together have <= calls * degree * degree! entries.
    let permutations = limits
        .max_hndq_calls
        .checked_mul(degree)
        .and_then(|bound| bound.checked_mul(factorial))
        .ok_or_else(|| capacity("canonical permutation capacity"))?;
    if permutations > limits.max_permutation_steps {
        return Err(capacity("canonical permutation capacity"));
    }
    // Encoding, first-degree hashing and sorting are bounded by input quads
    // and bytes. Within a permutation, issuer/path sizes are bounded by the
    // input blank-node count and degree. No host timeout is the work guard.
    crate::parse_02(&document.text)
}

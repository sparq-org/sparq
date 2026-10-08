//! The entry points sparq-canon calls. Trimmed from upstream `rdf-canon` 0.15.3 `api.rs`
//! to the functions this crate uses; bodies are unchanged (see `SPARQ-PATCHES.md`).

use super::{
    canon::{canonicalize_core, serialize},
    counter::{HndqCallCounter, SimpleHndqCallCounter},
    CanonicalizationError,
};
use digest::Digest;
use oxrdf02::{
    BlankNode, BlankNodeRef, Dataset, GraphName, GraphNameRef, Quad, QuadRef, Subject, SubjectRef,
    Term, TermRef,
};
use sha2::Sha256;
use std::collections::HashMap;

/// Returns the serialized canonical form of the canonicalized dataset,
/// where any blank nodes in the input quads are assigned deterministic identifiers.
pub fn canonicalize_quads(input_quads: &[Quad]) -> Result<String, CanonicalizationError> {
    let options = CanonicalizationOptions::default();
    canonicalize_quads_with::<Sha256>(input_quads, &options)
}

#[derive(Default)]
pub struct CanonicalizationOptions {
    pub hndq_call_limit: Option<usize>,
}

/// Given some options (e.g., call limit), returns the serialized canonical form of the
/// canonicalized dataset, where any blank nodes in the input quads are assigned
/// deterministic identifiers.
pub fn canonicalize_quads_with<D: Digest>(
    input_quads: &[Quad],
    options: &CanonicalizationOptions,
) -> Result<String, CanonicalizationError> {
    let input_dataset = Dataset::from_iter(input_quads);
    let issued_identifiers_map = issue_with::<D>(&input_dataset, options)?;
    let relabeled_dataset = relabel(&input_dataset, &issued_identifiers_map)?;
    Ok(serialize(&relabeled_dataset))
}

/// Given some options (e.g., call limit), assigns deterministic identifiers to any blank
/// nodes in the input dataset and returns the assignment result as a map.
pub fn issue_with<D: Digest>(
    input_dataset: &Dataset,
    options: &CanonicalizationOptions,
) -> Result<HashMap<String, String>, CanonicalizationError> {
    let hndq_call_counter = SimpleHndqCallCounter::new(options.hndq_call_limit);
    canonicalize_core::<D>(input_dataset, hndq_call_counter)
}

/// Assigns deterministic identifiers to any blank nodes in the input quads
/// and returns the assignment result as a map.
pub fn issue_quads(input_quads: &[Quad]) -> Result<HashMap<String, String>, CanonicalizationError> {
    let options = CanonicalizationOptions::default();
    issue_quads_with::<Sha256>(input_quads, &options)
}

/// Given some options (e.g., call limit), assigns deterministic identifiers to any blank
/// nodes in the input quads and returns the assignment result as a map.
pub fn issue_quads_with<D: Digest>(
    input_quads: &[Quad],
    options: &CanonicalizationOptions,
) -> Result<HashMap<String, String>, CanonicalizationError> {
    let input_dataset = Dataset::from_iter(input_quads);
    let hndq_call_counter = SimpleHndqCallCounter::new(options.hndq_call_limit);
    canonicalize_core::<D>(&input_dataset, hndq_call_counter)
}

/// Re-label blank node identifiers in the input dataset according to the issued identifiers map.
fn relabel(
    input_dataset: &Dataset,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<Dataset, CanonicalizationError> {
    input_dataset
        .iter()
        .map(|q| relabel_quad(q, issued_identifiers_map))
        .collect()
}

fn relabel_quad(
    q: QuadRef,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<Quad, CanonicalizationError> {
    Ok(Quad::new(
        relabel_subject(q.subject, issued_identifiers_map)?,
        q.predicate,
        relabel_term(q.object, issued_identifiers_map)?,
        relabel_graph_name(q.graph_name, issued_identifiers_map)?,
    ))
}

fn relabel_subject(
    s: SubjectRef,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<Subject, CanonicalizationError> {
    match s {
        SubjectRef::BlankNode(b) => Ok(Subject::BlankNode(relabel_blank_node(
            b,
            issued_identifiers_map,
        )?)),
        _ => Ok(s.into()),
    }
}

fn relabel_term(
    o: TermRef,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<Term, CanonicalizationError> {
    match o {
        TermRef::BlankNode(b) => Ok(Term::BlankNode(relabel_blank_node(
            b,
            issued_identifiers_map,
        )?)),
        _ => Ok(o.into()),
    }
}

fn relabel_graph_name(
    g: GraphNameRef,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<GraphName, CanonicalizationError> {
    match g {
        GraphNameRef::BlankNode(b) => Ok(GraphName::BlankNode(relabel_blank_node(
            b,
            issued_identifiers_map,
        )?)),
        _ => Ok(g.into()),
    }
}

fn relabel_blank_node(
    b: BlankNodeRef,
    issued_identifiers_map: &HashMap<String, String>,
) -> Result<BlankNode, CanonicalizationError> {
    match issued_identifiers_map.get(b.as_str()) {
        Some(id) => Ok(BlankNode::new(id)?),
        None => Err(CanonicalizationError::CanonicalIdentifierNotExist),
    }
}

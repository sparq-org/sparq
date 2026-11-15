//! The entry points sparq-canon calls. Trimmed from upstream `rdf-canon` 0.15.3 `api.rs`
//! to the functions this crate uses; bodies are unchanged (see `SPARQ-PATCHES.md`).

use super::{
    canon::{canonicalize_core, serialize},
    counter::{HndqCallCounter, SimpleHndqCallCounter},
    CanonicalizationError,
};
use digest::Digest;
use super::dataset::Dataset;
use oxrdf02::{
    BlankNode, BlankNodeRef, GraphName, GraphNameRef, Quad, QuadRef, Subject, SubjectRef,
    Term, TermRef,
};
use sha2::Sha256;
#[cfg(target_os = "zkvm")]
use std::collections::BTreeMap as HashMap;
#[cfg(not(target_os = "zkvm"))]
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
    pub permutation_limit: Option<usize>,
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
    let hndq_call_counter = SimpleHndqCallCounter::new(options.hndq_call_limit)
        .with_permutation_limit(options.permutation_limit);
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
    let hndq_call_counter = SimpleHndqCallCounter::new(options.hndq_call_limit)
        .with_permutation_limit(options.permutation_limit);
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

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn clique() -> Vec<Quad> {
        let mut quads = Vec::new();
        for a in 0..5 {
            for b in 0..5 {
                if a != b {
                    quads.push(Quad::new(
                        BlankNode::new(format!("n{a}")).unwrap(),
                        oxrdf02::NamedNode::new("urn:p").unwrap(),
                        BlankNode::new(format!("n{b}")).unwrap(),
                        GraphName::DefaultGraph,
                    ));
                }
            }
        }
        quads
    }

    #[test]
    fn pruned_candidates_consume_the_exact_budget() {
        let quads = crate::parse_02(include_str!(
            "../../tests/rdf-canon-testdata/rdfc10/test044-in.nq"
        ))
        .unwrap();
        let options = CanonicalizationOptions {
            hndq_call_limit: Some(4_000),
            permutation_limit: Some(2_800),
        };
        for result in [
            canonicalize_quads_with::<Sha256>(&quads, &options).map(|_| ()),
            issue_quads_with::<Sha256>(&quads, &options).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(CanonicalizationError::PermutationLimitExceeded)
            ));
        }
        let options = CanonicalizationOptions {
            hndq_call_limit: Some(4_000),
            permutation_limit: Some(3_000),
        };
        assert_eq!(
            canonicalize_quads_with::<Sha256>(&quads, &options).unwrap(),
            include_str!("../../tests/rdf-canon-testdata/rdfc10/test044-rdfc10.nq")
        );
    }

    #[test]
    fn exact_budget_boundary_includes_recursive_calls() {
        let quads = vec![
            Quad::new(
                BlankNode::new("a").unwrap(),
                oxrdf02::NamedNode::new("urn:p").unwrap(),
                BlankNode::new("b").unwrap(),
                GraphName::DefaultGraph,
            ),
            Quad::new(
                BlankNode::new("b").unwrap(),
                oxrdf02::NamedNode::new("urn:p").unwrap(),
                BlankNode::new("a").unwrap(),
                GraphName::DefaultGraph,
            ),
        ];
        // Two roots each visit two nodes, with two singleton groups per node.
        for budget in [7, 8] {
            let options = CanonicalizationOptions {
                hndq_call_limit: Some(usize::MAX),
                permutation_limit: Some(budget),
            };
            for result in [
                canonicalize_quads_with::<Sha256>(&quads, &options).map(|_| ()),
                issue_quads_with::<Sha256>(&quads, &options).map(|_| ()),
            ] {
                if budget == 7 {
                    assert!(matches!(
                        result,
                        Err(CanonicalizationError::PermutationLimitExceeded)
                    ));
                } else {
                    assert!(result.is_ok());
                }
            }
        }
    }

    #[test]
    fn exact_budget_rejects_exhaustion_and_preserves_output() {
        let quads = clique();
        let options = CanonicalizationOptions {
            hndq_call_limit: Some(usize::MAX),
            permutation_limit: Some(8),
        };
        for result in [
            canonicalize_quads_with::<Sha256>(&quads, &options).map(|_| ()),
            issue_quads_with::<Sha256>(&quads, &options).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(CanonicalizationError::PermutationLimitExceeded)
            ));
        }
        let options = CanonicalizationOptions {
            hndq_call_limit: Some(usize::MAX),
            permutation_limit: Some(20_000),
        };
        assert_eq!(
            canonicalize_quads_with::<Sha256>(&quads, &options).unwrap(),
            canonicalize_quads(&quads).unwrap()
        );
        assert_eq!(
            issue_quads_with::<Sha256>(&quads, &options).unwrap().len(),
            5
        );
    }
}

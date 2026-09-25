//! [GPT-6] Deterministic blank-node separation for read-query RDF merges.

use oxrdf::{BlankNode, NamedOrBlankNode, Term};
use rustc_hash::FxHashSet;

use crate::dataset::TripleSet;

// [GPT-6] Each preserved node can occupy at most one numbered namespace, so a
// free namespace exists among 0..=occupied.len(). No entropy or guessed seed is used.
pub(crate) fn namespace<'a>(preserved: impl Iterator<Item = &'a TripleSet>) -> usize {
    let mut occupied = FxHashSet::default();
    for triples in preserved {
        for triple in triples {
            for term in triple {
                collect_namespaces(term, &mut occupied);
            }
        }
    }
    (0..=occupied.len())
        .find(|n| !occupied.contains(n))
        .unwrap_or(occupied.len())
}

fn collect_namespaces(term: &Term, occupied: &mut FxHashSet<usize>) {
    match term {
        Term::BlankNode(node) => {
            if let Some((number, _)) = node
                .as_str()
                .strip_prefix("fm")
                .and_then(|s| s.split_once('_'))
            {
                if let Ok(number) = number.parse() {
                    occupied.insert(number);
                }
            }
        }
        Term::Triple(triple) => {
            if let NamedOrBlankNode::BlankNode(node) = &triple.subject {
                collect_namespaces(&Term::BlankNode(node.clone()), occupied);
            }
            collect_namespaces(&triple.object, occupied);
        }
        _ => {}
    }
}

// [GPT-6] A graph-index prefix and the original valid label form an injective
// mapping. Repeated occurrences, including in triple terms, retain their identity.
pub(crate) fn rename(mut term: Term, namespace: usize, graph: usize) -> Term {
    fn node(node: &BlankNode, namespace: usize, graph: usize) -> BlankNode {
        BlankNode::new_unchecked(format!("fm{namespace}_{graph}_{}", node.as_str()))
    }
    match &mut term {
        Term::BlankNode(value) => *value = node(value, namespace, graph),
        Term::Triple(triple) => {
            if let NamedOrBlankNode::BlankNode(value) = &mut triple.subject {
                *value = node(value, namespace, graph);
            }
            triple.object = rename(triple.object.clone(), namespace, graph);
        }
        _ => {}
    }
    term
}

// sq-3f1u6: property witnesses for the structural-similarity metric axioms.

use std::collections::HashSet;

use oxrdf::{NamedNode, Term};
use proptest::prelude::*;
use sparq_core::Graph;
use sparq_sim::{weighted_jaccard, SignatureMode, Sim, SimConfig};

const FEATURES: usize = 8;

fn entity(name: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://example.com/{name}")).unwrap())
}

fn graph_for_masks(masks: &[u8]) -> Graph {
    let mut triples = String::new();
    for (entity_index, mask) in masks.iter().enumerate() {
        for feature in 0..FEATURES {
            if mask & (1 << feature) != 0 {
                triples.push_str(&format!(
                    "<http://example.com/e{entity_index}> <http://example.com/p{feature}> \"value\" .\n"
                ));
            }
        }
    }
    Graph::load_str(&triples, "ntriples").unwrap()
}

fn unweighted(graph: &Graph, mode: SignatureMode) -> Sim<'_> {
    let cfg = SimConfig {
        mode,
        idf: false,
        profile_fallback: false,
        max_pair_frequency: usize::MAX,
        ..SimConfig::default()
    };
    // Like `profile_fallback` above: fallback tiers append blocks scored on their own
    // scale, so they are disabled to pin the exactly-scored ranking properties.
    #[cfg(feature = "lexical")]
    let cfg = SimConfig { lexical_fallback: false, ..cfg };
    Sim::with_config(graph, cfg)
}

proptest! {
    #[test]
    fn term_similarity_is_reflexive_symmetric_and_bounded(a in 1_u8..=u8::MAX, b in any::<u8>()) {
        let graph = graph_for_masks(&[a, b]);
        let sim = unweighted(&graph, SignatureMode::PredicateNeighbor);
        let ea = entity("e0");
        let eb = entity("e1");
        let ab = sim.similarity(&ea, &eb);

        prop_assert_eq!(sim.similarity(&ea, &ea), 1.0);
        prop_assert_eq!(ab, sim.similarity(&eb, &ea));
        prop_assert!((0.0..=1.0).contains(&ab), "similarity out of range: {ab}");
    }

    #[test]
    fn disjoint_term_signatures_have_zero_similarity(
        a in 1_u8..=u8::MAX,
        b in 1_u8..=u8::MAX,
    ) {
        let left = a & !b;
        let right = b & !a;
        prop_assume!(left != 0 && right != 0);
        let graph = graph_for_masks(&[left, right]);
        let sim = unweighted(&graph, SignatureMode::PredicateNeighbor);

        prop_assert_eq!(sim.similarity(&entity("e0"), &entity("e1")), 0.0);
    }

    #[test]
    fn weighted_jaccard_obeys_metric_axioms(a in 1_u8..=u8::MAX, b in 1_u8..=u8::MAX) {
        let graph = graph_for_masks(&[a, b]);
        let sim = Sim::new(&graph);
        let sa = sim.signature(&entity("e0")).unwrap();
        let sb = sim.signature(&entity("e1")).unwrap();
        let ab = weighted_jaccard(&sa, &sb);

        prop_assert_eq!(weighted_jaccard(&sa, &sa), 1.0);
        prop_assert_eq!(ab, weighted_jaccard(&sb, &sa));
        prop_assert!((0.0..=1.0).contains(&ab), "weighted Jaccard out of range: {ab}");
        if a & b == 0 {
            prop_assert_eq!(ab, 0.0);
        }
    }

    #[cfg(feature = "multi-hop")]
    #[test]
    fn depth_two_weighted_jaccard_is_symmetric_and_bounded(
        extra_a in any::<u8>(),
        extra_b in any::<u8>(),
    ) {
        let mut triples = String::from(
            "<http://example.com/a> <http://example.com/p> <http://example.com/shared> .\n\
             <http://example.com/b> <http://example.com/route> <http://example.com/bridge> .\n\
             <http://example.com/bridge> <http://example.com/p> <http://example.com/shared> .\n",
        );
        for feature in 0..FEATURES {
            if extra_a & (1 << feature) != 0 {
                triples.push_str(&format!(
                    "<http://example.com/a> <http://example.com/ap{feature}> <http://example.com/an{feature}> .\n"
                ));
            }
            if extra_b & (1 << feature) != 0 {
                triples.push_str(&format!(
                    "<http://example.com/b> <http://example.com/bp{feature}> <http://example.com/bn{feature}> .\n"
                ));
            }
        }
        let graph = Graph::load_str(&triples, "ntriples").unwrap();
        let sim = Sim::with_config(
            &graph,
            SimConfig {
                idf: false,
                depth: 2,
                profile_fallback: false,
                ..SimConfig::default()
            },
        );
        let a = sim.signature(&entity("a")).unwrap();
        let b = sim.signature(&entity("b")).unwrap();
        let ab = weighted_jaccard(&a, &b);
        let reverse = weighted_jaccard(&b, &a);

        prop_assert_eq!(ab, reverse);
        prop_assert!((0.0..=1.0).contains(&ab), "depth-2 similarity out of range: {ab}");
    }

    #[test]
    fn most_similar_is_sorted_bounded_and_deduplicated(
        query in 1_u8..=u8::MAX,
        candidates in prop::collection::vec(any::<u8>(), 1..16),
        k in 0_usize..20,
    ) {
        let mut masks = Vec::with_capacity(candidates.len() + 1);
        masks.push(query);
        masks.extend(candidates);
        let graph = graph_for_masks(&masks);
        let sim = unweighted(&graph, SignatureMode::Predicates);
        let query_term = entity("e0");
        let results = sim.most_similar(&query_term, k);
        let mut seen = HashSet::new();

        prop_assert!(results.len() <= k);
        for pair in results.windows(2) {
            prop_assert!(pair[0].1 >= pair[1].1, "ranking is not non-increasing: {results:?}");
        }
        for (term, score) in results {
            prop_assert_ne!(&term, &query_term);
            prop_assert!((0.0..=1.0).contains(&score), "ranked score out of range: {score}");
            prop_assert!(seen.insert(term), "duplicate ranked entity");
        }
    }
}

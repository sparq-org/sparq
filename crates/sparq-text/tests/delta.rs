//! Differential test: after every random `apply_delta` batch the
//! incrementally maintained index must equal a from-scratch rebuild
//! (`TextIndex::build` over the post-delta graph) — the same pattern the
//! GeoIndex uses. Deletes are part of the batches: the dictionary retains
//! deleted terms, so rebuild and incremental agree EXACTLY (full-state
//! `PartialEq`, not just query equivalence).

use oxrdf::{Literal, NamedNode, Term};
use rand::rngs::StdRng;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_text::TextIndex;

const WORDS: &[&str] = &[
    "quick", "brown", "fox", "lazy", "dog", "jumps", "over", "the", "café", "straße",
    "σοφία", "москва", "data", "graph", "query", "index", "text", "search",
];

fn random_sentence(rng: &mut StdRng) -> String {
    let n = rng.random_range(1..=8);
    (0..n).map(|_| WORDS[rng.random_range(0..WORDS.len())]).collect::<Vec<_>>().join(" ")
}

fn random_triple(rng: &mut StdRng) -> [Term; 3] {
    let s = Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/s{}", rng.random_range(0..40))));
    let p = Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/p{}", rng.random_range(0..4))));
    let o = match rng.random_range(0..10) {
        // Mostly string literals; some language-tagged, some typed/IRI noise.
        0..=5 => Term::Literal(Literal::new_simple_literal(random_sentence(rng))),
        6 => Term::Literal(Literal::new_language_tagged_literal_unchecked(
            random_sentence(rng),
            "en",
        )),
        7 => Term::Literal(Literal::new_typed_literal(
            format!("{}", rng.random_range(0..1000)),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )),
        _ => Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/o{}", rng.random_range(0..20)))),
    };
    [s, p, o]
}

#[test]
fn incremental_equals_rebuilt_after_random_deltas() {
    let mut rng = StdRng::seed_from_u64(0x5fa9_7e57);

    // Seed graph.
    let seed: Vec<[Term; 3]> = (0..120).map(|_| random_triple(&mut rng)).collect();
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    graph.apply_delta(&seed, &[]).unwrap();
    let mut live: Vec<[Term; 3]> = seed;

    let mut index = TextIndex::build(&graph);
    assert!(!index.is_empty());

    for round in 0..30 {
        // A random batch: fresh inserts (some duplicating live triples or
        // already-interned literals) + deletes of live triples.
        let inserts: Vec<[Term; 3]> = (0..rng.random_range(1..=15)).map(|_| random_triple(&mut rng)).collect();
        let mut deletes: Vec<[Term; 3]> = Vec::new();
        for _ in 0..rng.random_range(0..=10) {
            if live.is_empty() {
                break;
            }
            deletes.push(live.swap_remove(rng.random_range(0..live.len())));
        }

        graph.apply_delta(&inserts, &deletes).unwrap();
        index.apply_delta(&graph, &inserts, &deletes);
        live.extend(inserts.iter().cloned());

        let rebuilt = TextIndex::build(&graph);
        assert_eq!(index, rebuilt, "incremental != rebuilt after round {round}");

        // And the observable behaviour agrees too (belt and braces).
        for q in ["quick fox", "café", "straße data", "mosk*", "σοφία"] {
            assert_eq!(index.search(q), rebuilt.search(q), "search({q:?}) diverged in round {round}");
            assert_eq!(index.search_any(q), rebuilt.search_any(q));
        }
    }
}

#[test]
fn delta_ignores_non_text_inserts() {
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut index = TextIndex::build(&graph);
    let batch = [[
        Term::NamedNode(NamedNode::new_unchecked("http://ex/s")),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/p")),
        Term::Literal(Literal::new_typed_literal(
            "7",
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    ]];
    graph.apply_delta(&batch, &[]).unwrap();
    index.apply_delta(&graph, &batch, &[]);
    assert!(index.is_empty());
    assert_eq!(index, TextIndex::build(&graph));
}

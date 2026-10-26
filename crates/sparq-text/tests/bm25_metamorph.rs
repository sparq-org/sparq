//! sq-8ncxj — seeded metamorphic laws for the pure BM25 index.

use oxrdf::Term;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_text::{Hit, TextIndex};

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "graph", "query", "text", "index",
];

fn graph_of(docs: &[String]) -> Graph {
    let nt = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| format!("<http://ex/s{i}> <http://ex/text> \"{doc}\" .\n"))
        .collect::<String>();
    Graph::load_str(&nt, "ntriples").unwrap()
}

fn random_corpus(rng: &mut StdRng, len: usize) -> Vec<String> {
    (0..len)
        .map(|i| {
            let words = (0..rng.random_range(1..=10))
                .map(|_| WORDS[rng.random_range(0..WORDS.len())])
                .collect::<Vec<_>>()
                .join(" ");
            // The marker makes every literal a distinct document without
            // changing the query-term frequencies under test.
            format!("{words} marker{i}")
        })
        .collect()
}

fn literal(graph: &Graph, hit: &Hit) -> String {
    match graph.dict().term(hit.id) {
        Term::Literal(value) => value.value().to_owned(),
        term => panic!("text hit referred to non-literal {term:?}"),
    }
}

/// Preserve score-group order while canonicalising only equal-score ties.
/// IDs are graph-local and legitimately change when the same RDF terms are
/// interned in another order, so identity is compared by literal value.
fn ranked(graph: &Graph, hits: &[Hit]) -> Vec<(u32, Vec<String>)> {
    assert!(
        hits.windows(2).all(|pair| pair[0].score >= pair[1].score),
        "BM25 hits were not ranked best-first"
    );
    let mut groups: Vec<(u32, Vec<String>)> = Vec::new();
    for hit in hits {
        let bits = hit.score.to_bits();
        if groups.last().is_none_or(|(score, _)| *score != bits) {
            groups.push((bits, Vec::new()));
        }
        groups.last_mut().unwrap().1.push(literal(graph, hit));
    }
    for (_, docs) in &mut groups {
        docs.sort_unstable();
    }
    groups
}

#[test]
fn build_order_invariant_ranking() {
    let mut rng = StdRng::seed_from_u64(0x8ac5_0001);
    for round in 0..24 {
        let docs = random_corpus(&mut rng, 24);
        let mut permuted = docs.clone();
        for i in (1..permuted.len()).rev() {
            let j = rng.random_range(0..=i);
            permuted.swap(i, j);
        }
        let first = graph_of(&docs);
        let second = graph_of(&permuted);
        let first_index = TextIndex::build(&first);
        let second_index = TextIndex::build(&second);

        for query in WORDS {
            assert_eq!(
                ranked(&first, &first_index.search(query)),
                ranked(&second, &second_index.search(query)),
                "ranking changed under insertion permutation in round {round} for {query:?}"
            );
        }
    }
}

#[test]
fn tf_monotone_scoring() {
    let mut rng = StdRng::seed_from_u64(0x8ac5_0002);
    for round in 0..64 {
        let tf = rng.random_range(1..=6);
        let filler = rng.random_range(0..=8);
        let target = std::iter::repeat_n("needle", tf)
            .chain(std::iter::repeat_n("filler", filler))
            .collect::<Vec<_>>()
            .join(" ");
        let stronger = format!("{target} needle");
        let background = random_corpus(&mut rng, 18);

        let mut before_docs = background.clone();
        before_docs.push(target.clone());
        let before_graph = graph_of(&before_docs);
        let before = TextIndex::build(&before_graph)
            .search("needle")
            .into_iter()
            .find(|hit| literal(&before_graph, hit) == target)
            .unwrap()
            .score;

        let mut after_docs = background;
        after_docs.push(stronger.clone());
        let after_graph = graph_of(&after_docs);
        let after = TextIndex::build(&after_graph)
            .search("needle")
            .into_iter()
            .find(|hit| literal(&after_graph, hit) == stronger)
            .unwrap()
            .score;

        assert!(
            after >= before,
            "score decreased when tf increased in round {round}: {before} -> {after}"
        );
    }
}

#[test]
fn absent_term_zero_hits() {
    let mut rng = StdRng::seed_from_u64(0x8ac5_0003);
    for round in 0..64 {
        let len = rng.random_range(1..=30);
        let graph = graph_of(&random_corpus(&mut rng, len));
        let index = TextIndex::build(&graph);
        assert!(
            index.search("term-not-in-vocabulary").is_empty(),
            "phantom AND hit in round {round}"
        );
        assert!(
            index.search_any("term-not-in-vocabulary").is_empty(),
            "phantom OR hit in round {round}"
        );
    }
}

//! [OPUS-5] sq-lsp7k.9.4: the dictionary-id-keyed score map, and the cross-crate
//! composition it exists for — feeding PageRank into `sparq_text::CompletionIndex`.
//!
//! `sparq-text` is a TEST-ONLY dependency here (see `Cargo.toml`). The point of the
//! composition tests is that the map produced by this crate type-checks as
//! `CompletionIndex::complete`'s `scores` argument and actually reorders its output —
//! a claim that a test inside `sparq-algos` alone could only assert, not demonstrate.

#![cfg(feature = "ranking")]

use oxrdf::{NamedNode, Term};
use sparq_algos::{
    pagerank, pagerank_scores_by_dict_id, scores_by_dict_id, NodeGraph, PageRankConfig, ScoreBudget,
};
use sparq_core::{dict::Id, Graph};
use sparq_text::CompletionIndex;

fn graph(nt: &str) -> Graph {
    Graph::load_str(nt, "nt").expect("parse N-Triples")
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).expect("valid IRI"))
}

fn id_of(g: &Graph, s: &str) -> Id {
    g.id_of(&iri(s)).expect("term is in the dictionary")
}

/// `a1`, `a2`, `a3` all point at the hub `zhub`; `zhub` points at the sink `zsink`.
/// PageRank order is `zsink` > `zhub` > the three (tied) leaves: rank flows downstream, so
/// the hub concentrates the leaves' mass and then passes all of it to its only
/// out-neighbour, which — being dangling — additionally takes a share of the uniformly
/// redistributed dangling mass. All five entities share the `http://e/` prefix, so all
/// five are completion candidates for it; the predicate `http://p/k` deliberately is not.
///
/// The local names are chosen so the PageRank order is disjoint from alphabetical order
/// (`a1` < `a2` < `a3` < `zhub` < `zsink`). `CompletionIndex` falls back to ascending key
/// order when no scores are supplied, so a fixture whose top-ranked entity also sorted
/// first alphabetically could not distinguish "ranking worked" from "ranking did nothing".
const HUB: &str = r#"
<http://e/a1> <http://p/k> <http://e/zhub> .
<http://e/a2> <http://p/k> <http://e/zhub> .
<http://e/a3> <http://p/k> <http://e/zhub> .
<http://e/zhub> <http://p/k> <http://e/zsink> .
"#;

#[test]
fn scores_are_rekeyed_from_node_index_to_dictionary_id() {
    let g = graph(HUB);
    let ng = NodeGraph::build(&g);
    let ranks = pagerank(&ng, PageRankConfig::default());
    let scores = scores_by_dict_id(&ng, &ranks, ScoreBudget::All);

    assert_eq!(scores.len(), ng.len(), "every node must appear exactly once");
    // The value stored under a node's dictionary id is that node's rank — not a
    // neighbour's, and not the rank at the same *dictionary* index.
    for i in 0..ng.len() {
        assert_eq!(
            scores.get(&ng.dict_id(i)).copied(),
            Some(ranks[i]),
            "node {i} is mis-keyed"
        );
    }
    // And the keys really are dictionary ids resolving back to the source terms.
    assert!(scores.contains_key(&id_of(&g, "http://e/zhub")));
}

#[test]
fn convenience_matches_the_explicit_two_step() {
    let g = graph(HUB);
    let ng = NodeGraph::build(&g);
    let cfg = PageRankConfig::default();
    let explicit = scores_by_dict_id(&ng, &pagerank(&ng, cfg), ScoreBudget::All);
    let convenience = pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::All);
    assert_eq!(explicit, convenience);
}

#[test]
fn top_k_retains_exactly_the_highest_ranked_entities() {
    let g = graph(HUB);
    let scores = pagerank_scores_by_dict_id(&g, PageRankConfig::default(), ScoreBudget::TopK(2));

    assert_eq!(scores.len(), 2, "budget must bound the resident map");
    let (sink, hub) = (id_of(&g, "http://e/zsink"), id_of(&g, "http://e/zhub"));
    assert!(scores.contains_key(&sink), "the sink is top-ranked");
    assert!(scores.contains_key(&hub), "the hub is second");
    for leaf in ["http://e/a1", "http://e/a2", "http://e/a3"] {
        assert!(
            !scores.contains_key(&id_of(&g, leaf)),
            "{leaf} outranked the hub"
        );
    }
    // Truncation must not rescale: a retained score is the true global rank, so it
    // matches the value the unbudgeted map holds for the same entity.
    let all = pagerank_scores_by_dict_id(&g, PageRankConfig::default(), ScoreBudget::All);
    assert_eq!(scores[&sink], all[&sink]);
    assert_eq!(scores[&hub], all[&hub]);
    assert!(all[&sink] > all[&hub], "rank flows downstream to the sink");
}

#[test]
fn top_k_of_zero_is_empty_and_a_large_k_is_the_whole_graph() {
    let g = graph(HUB);
    let cfg = PageRankConfig::default();
    assert!(pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::TopK(0)).is_empty());
    assert_eq!(
        pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::TopK(999)),
        pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::All),
        "a budget above the node count must retain everything"
    );
}

#[test]
fn empty_graph_yields_an_empty_score_map() {
    let g = graph("");
    let cfg = PageRankConfig::default();
    assert!(pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::All).is_empty());
    assert!(pagerank_scores_by_dict_id(&g, cfg, ScoreBudget::TopK(5)).is_empty());
}

#[test]
fn non_finite_scores_are_dropped_rather_than_poisoning_the_ordering() {
    let g = graph(HUB);
    let ng = NodeGraph::build(&g);
    let mut ranks = pagerank(&ng, PageRankConfig::default());
    ranks[0] = f64::NAN;
    ranks[1] = f64::INFINITY;
    let scores = scores_by_dict_id(&ng, &ranks, ScoreBudget::All);
    assert_eq!(scores.len(), ng.len() - 2);
    assert!(!scores.contains_key(&ng.dict_id(0)));
    assert!(!scores.contains_key(&ng.dict_id(1)));
    assert!(scores.values().all(|s| s.is_finite()));
}

/// `total_cmp` sorts `NaN` above `+inf` above every finite score, so a `TopK` budget that
/// filtered non-finite values only *after* truncating would hand the retained slots to the
/// poison values and return fewer — possibly zero — finite scores than the budget allows.
#[test]
fn top_k_drops_non_finite_scores_before_truncating_rather_than_after() {
    let g = graph(HUB);
    let ng = NodeGraph::build(&g);
    let clean = pagerank(&ng, PageRankConfig::default());

    // Poison the two TOP-ranked nodes, so a post-truncation filter would empty the map.
    let (sink, hub) = (id_of(&g, "http://e/zsink"), id_of(&g, "http://e/zhub"));
    let index_of = |id: Id| (0..ng.len()).find(|&i| ng.dict_id(i) == id).expect("node");
    let mut ranks = clean.clone();
    ranks[index_of(sink)] = f64::NAN;
    ranks[index_of(hub)] = f64::INFINITY;

    // `min(k, finite_count)` entries, all finite, and they are the highest finite scores:
    // with the sink and hub poisoned, the three tied leaves are what remains.
    let finite_count = ng.len() - 2;
    for k in [1, 2, finite_count, finite_count + 1, 999] {
        let scores = scores_by_dict_id(&ng, &ranks, ScoreBudget::TopK(k));
        assert_eq!(
            scores.len(),
            k.min(finite_count),
            "TopK({k}) must retain min(k, finite_count) entries"
        );
        assert!(scores.values().all(|s| s.is_finite()), "TopK({k}) kept a non-finite score");
        assert!(!scores.contains_key(&sink), "TopK({k}) kept the NaN-scored node");
        assert!(!scores.contains_key(&hub), "TopK({k}) kept the +inf-scored node");
        // Retained scores are the true global ranks, untouched by the poisoning.
        for (id, score) in &scores {
            assert_eq!(*score, clean[index_of(*id)], "TopK({k}) mis-keyed a score");
        }
    }

    // -inf sorts BELOW every finite score, so it is excluded by ordering alone — but it
    // must not consume a slot either when the budget exceeds the finite count.
    let mut with_neg_inf = clean.clone();
    with_neg_inf[0] = f64::NEG_INFINITY;
    let scores = scores_by_dict_id(&ng, &with_neg_inf, ScoreBudget::TopK(999));
    assert_eq!(scores.len(), ng.len() - 1);
    assert!(!scores.contains_key(&ng.dict_id(0)));
}

#[test]
#[should_panic(expected = "does not match the node graph")]
fn a_mismatched_score_vector_panics_rather_than_mis_attributing() {
    let g = graph(HUB);
    let ng = NodeGraph::build(&g);
    let _ = scores_by_dict_id(&ng, &[0.5, 0.5], ScoreBudget::All);
}

/// The composition this bead exists for: PageRank scores injected into
/// `CompletionIndex::complete` must actually change the returned order.
#[test]
fn pagerank_scores_rank_completion_candidates() {
    let g = graph(HUB);
    let index = CompletionIndex::build(&g);
    let scores = pagerank_scores_by_dict_id(&g, PageRankConfig::default(), ScoreBudget::All);

    // Unranked: the index falls back to ascending key order, so the two important entities
    // sort LAST. This is the baseline the injected scores have to overturn.
    let unranked = index.complete("http://e/", 5, None);
    assert_eq!(unranked.len(), 5);
    assert_eq!(
        unranked[0].id,
        id_of(&g, "http://e/a1"),
        "unscored completion must be plain ascending key order"
    );

    // Ranked: the sink is the top PageRank entity, then the hub.
    let ranked = index.complete("http://e/", 5, Some(&scores));
    assert_eq!(
        ranked[0].id,
        id_of(&g, "http://e/zsink"),
        "PageRank scores did not reorder the completions"
    );
    assert_eq!(
        ranked[1].id,
        id_of(&g, "http://e/zhub"),
        "the hub is the second-ranked entity"
    );
    // The scores really did travel: the candidate carries its injected rank.
    assert_eq!(ranked[0].score, scores[&id_of(&g, "http://e/zsink")]);
    // Same candidate set, only reordered.
    let mut before: Vec<Id> = unranked.iter().map(|c| c.id).collect();
    let mut after: Vec<Id> = ranked.iter().map(|c| c.id).collect();
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after, "ranking must not add or drop candidates");
}

/// A `TopK` budget is a valid ranking source: the entities it retains still sort to the
/// front, and the unscored tail is served at score `0.0` rather than being hidden.
#[test]
fn a_top_k_budget_still_ranks_and_never_hides_candidates() {
    let g = graph(HUB);
    let index = CompletionIndex::build(&g);
    let budgeted = pagerank_scores_by_dict_id(&g, PageRankConfig::default(), ScoreBudget::TopK(1));

    let ranked = index.complete("http://e/", 5, Some(&budgeted));
    assert_eq!(ranked.len(), 5, "the unscored tail must still be returned");
    assert_eq!(ranked[0].id, id_of(&g, "http://e/zsink"));
    assert!(ranked[0].score > 0.0);
    assert!(
        ranked[1..].iter().all(|c| c.score == 0.0),
        "entities outside the budget score zero"
    );
}

/// A term that appears only as a PREDICATE is not a `NodeGraph` node, so it is unscored —
/// but `CompletionIndex` still completes it. It must rank last, not disappear.
#[test]
fn a_predicate_only_term_is_unscored_but_still_completable() {
    let nt = r#"
<http://e/aaa> <http://e/zzz> <http://e/bbb> .
<http://e/bbb> <http://e/zzz> <http://e/aaa> .
"#;
    let g = graph(nt);
    let scores = pagerank_scores_by_dict_id(&g, PageRankConfig::default(), ScoreBudget::All);
    let predicate = id_of(&g, "http://e/zzz");
    assert!(
        !scores.contains_key(&predicate),
        "predicates are erased by the NodeGraph projection"
    );

    let index = CompletionIndex::build(&g);
    let ranked = index.complete("http://e/", 3, Some(&scores));
    assert_eq!(ranked.len(), 3);
    assert_eq!(
        ranked[2].id, predicate,
        "the unscored predicate must sort last, not vanish"
    );
    assert_eq!(ranked[2].score, 0.0);
}

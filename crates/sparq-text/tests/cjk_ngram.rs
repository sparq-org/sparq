//! The opt-in CJK character-bigram analyzer ([`Analyzer::CjkNgram`]). sq-m3ln
//!
//! The default UAX #29 analyzer splits an unspaced Han run per ideograph, so a
//! multi-char CJK term searches as the AND of its (very common) characters —
//! low precision. The n-gram analyzer indexes overlapping character bigrams,
//! so a query for `東京` matches only documents where that *pair* co-occurs.
//! These tests pin (a) the precision win over the default analyzer, (b) that
//! the index records its analyzer so build and query always agree, and (c) that
//! non-CJK behaviour and the incremental/rebuild contract are unchanged.

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_text::{Analyzer, TextIndex};

/// A graph whose `ex:comment` objects are the given literals (one subject each).
fn graph_of(literals: &[&str]) -> Graph {
    let nt: String = literals
        .iter()
        .enumerate()
        .map(|(i, l)| format!("<http://ex/s{i}> <http://ex/comment> {l} .\n"))
        .collect();
    Graph::load_str(&nt, "ntriples").unwrap()
}

fn values(graph: &Graph, hits: &[sparq_text::Hit]) -> Vec<String> {
    let mut v: Vec<String> = hits
        .iter()
        .map(|h| match graph.dict.term(h.id) {
            Term::Literal(l) => l.value().to_string(),
            other => panic!("hit {} not a literal: {other}", h.id),
        })
        .collect();
    v.sort();
    v
}

#[test]
fn analyzer_is_recorded_and_defaults_to_unicode() {
    assert_eq!(TextIndex::default().analyzer(), Analyzer::Unicode);
    let g = graph_of(&[r#""hi""#]);
    assert_eq!(TextIndex::build(&g).analyzer(), Analyzer::Unicode);
    assert_eq!(
        TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram).analyzer(),
        Analyzer::CjkNgram
    );
    assert_eq!(TextIndex::with_analyzer(Analyzer::CjkNgram).analyzer(), Analyzer::CjkNgram);
    assert_eq!(
        TextIndex::with_positions_analyzer(Analyzer::CjkNgram).analyzer(),
        Analyzer::CjkNgram
    );
    assert!(TextIndex::with_positions_analyzer(Analyzer::CjkNgram).has_positions());
}

#[test]
fn ngram_improves_multichar_cjk_precision() {
    // doc0 has 東 and 京 ADJACENT (東京); doc1 has them only as separate chars
    // (北京 = 京 present, 東 absent ... actually keep it as a spurious co-occurrence):
    // doc1 below has BOTH 東 and 京 but NOT adjacent — the default analyzer wrongly
    // matches it for the term 東京; the n-gram analyzer does not.
    let g = graph_of(&[
        r#""東京都に住む""#, // 東京 adjacent -> should match "東京"
        r#""京都の東""#,     // contains 京 and 東 but NOT the bigram 東京 (の splits them)
        r#""大阪市""#,       // unrelated
    ]);

    // Default analyzer: "東京" = AND(東, 京) -> matches BOTH docs that contain
    // both characters anywhere (a false positive on doc1, where 東 and 京 are
    // separated by の and never adjacent).
    let uni = TextIndex::build(&g);
    assert_eq!(
        values(&g, &uni.search("東京")),
        ["京都の東", "東京都に住む"],
        "default analyzer matches any doc containing both ideographs (low precision)"
    );

    // N-gram analyzer: "東京" = the single bigram 東京 -> matches ONLY the doc
    // where the pair actually occurs adjacently.
    let ng = TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram);
    assert_eq!(
        values(&g, &ng.search("東京")),
        ["東京都に住む"],
        "n-gram analyzer matches only the doc with the co-occurring bigram (high precision)"
    );

    // Single-ideograph queries under the n-gram analyzer match on the bigram
    // vocabulary, NOT on individual characters embedded in a run: a lone-char
    // query `東` only matches a doc where 東 appeared as an ISOLATED run (here,
    // doc1 "京都の東" — の splits 東 off from the 京都 run and nothing follows, so
    // 東 is a one-char run → indexed as the unigram 東). In doc0 "東京都に住む"
    // the 東 is fused into the 東京 bigram, so it is NOT reachable by a bare 東.
    // This is the inherent precision/recall trade of n-gram indexing and is the
    // exact reason it stays OPT-IN. sq-m3ln
    assert_eq!(values(&g, &ng.search("東")), ["京都の東"]);
}

#[test]
fn ngram_lone_ideograph_indexed_as_unigram() {
    // A document that is a single ideograph (a one-char run) is indexed as that
    // unigram, so a single-char query finds it under the n-gram analyzer.
    let g = graph_of(&[r#""東""#, r#""東京""#]);
    let ng = TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram);
    // The lone-char doc is reachable by the single-char query; the fused 東京 is
    // reachable by the bigram query.
    assert_eq!(values(&g, &ng.search("東")), ["東"]);
    assert_eq!(values(&g, &ng.search("東京")), ["東京"]);
}

#[test]
fn ngram_three_char_term_is_and_of_its_bigrams() {
    // 東京都 -> bigrams 東京, 京都. doc0 has the full run; doc1 has 東京 but a
    // different third char; doc2 has 京都 but a different first char. Only doc0
    // contains BOTH bigrams.
    let g = graph_of(&[r#""東京都""#, r#""東京府""#, r#""西京都""#]);
    let ng = TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram);
    assert_eq!(values(&g, &ng.search("東京都")), ["東京都"]);
    // matchesAny over the bigrams hits every doc sharing a bigram.
    assert_eq!(
        values(&g, &ng.search_any("東京都")),
        ["東京府", "東京都", "西京都"]
    );
}

#[test]
fn ngram_phrase_over_bigrams_keeps_adjacency() {
    // With positions, a phrase over the bigram stream still distinguishes order
    // and adjacency. doc0 "東京都" -> bigrams 東京,京都 at positions 0,1, so
    // phrase("東京都") (bigrams 東京 then 京都) matches; the reversed run does not.
    let g = graph_of(&[r#""東京都""#, r#""都京東""#]);
    let ng = TextIndex::build_with_positions_analyzer(&g, Analyzer::CjkNgram);
    assert!(ng.has_positions());
    let hits: Vec<String> = ng
        .phrase("東京都")
        .iter()
        .map(|&id| match g.dict.term(id) {
            Term::Literal(l) => l.value().to_string(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(hits, ["東京都"]);
    // The reversed run shares no bigram with the forward query, so no match.
    assert!(ng.phrase("都京東").iter().all(|&id| match g.dict.term(id) {
        Term::Literal(l) => l.value() == "都京東",
        _ => true,
    }));
}

#[test]
fn ngram_agrees_with_default_on_latin_corpus() {
    // A Latin corpus indexes and searches identically under both analyzers
    // (the opt-in must not perturb non-CJK text).
    let g = graph_of(&[r#""The quick brown fox""#, r#""Fox hunting""#, r#""quick start""#]);
    let uni = TextIndex::build(&g);
    let ng = TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram);
    for q in ["quick", "fox", "quick fox", "qui*"] {
        assert_eq!(
            values(&g, &uni.search(q)),
            values(&g, &ng.search(q)),
            "analyzers must agree on Latin query {q:?}"
        );
    }
    // Prefix search still works under the n-gram analyzer for Latin tokens.
    assert_eq!(values(&g, &ng.search("qui*")), ["The quick brown fox", "quick start"]);
}

#[test]
fn ngram_incremental_matches_rebuild() {
    // The incremental path (apply_delta) over an n-gram index must stay equal to
    // a from-scratch n-gram rebuild — the same differential invariant the
    // default analyzer holds (and it must NOT equal a default-analyzer rebuild).
    let p = Term::NamedNode(NamedNode::new_unchecked("http://ex/p"));
    let lit = |v: &str| Term::Literal(Literal::new_simple_literal(v));
    let terms = ["東京都", "京都府", "北京市"];

    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut idx = TextIndex::with_analyzer(Analyzer::CjkNgram);

    let inserts: Vec<[Term; 3]> = terms
        .iter()
        .enumerate()
        .map(|(i, w)| {
            [
                Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/s{i}"))),
                p.clone(),
                lit(w),
            ]
        })
        .collect();
    graph.apply_delta(&inserts, &[]).unwrap();
    idx.apply_delta(&graph, &inserts, &[]);

    assert_eq!(
        idx,
        TextIndex::build_with_analyzer(&graph, Analyzer::CjkNgram),
        "incremental n-gram index == n-gram rebuild"
    );
    // The analyzer survives apply_delta.
    assert_eq!(idx.analyzer(), Analyzer::CjkNgram);
    // And it is NOT equal to a default-analyzer rebuild (different vocabulary).
    assert_ne!(idx, TextIndex::build(&graph));
    // 東京 (the bigram) matches only the adjacent-pair doc.
    assert_eq!(values(&graph, &idx.search("東京")), ["東京都"]);
}

#[cfg(feature = "engine")]
#[test]
fn ngram_index_drives_text_matches_rewrite() {
    // End-to-end: the `text:matches` rewrite reads hits from whatever analyzer
    // the index was built with, so an n-gram index gives the higher-precision
    // CJK result through plain SPARQL.
    let g = graph_of(&[r#""東京都に住む""#, r#""京都の東側""#]);
    let idx = TextIndex::build_with_analyzer(&g, Analyzer::CjkNgram);
    let r = sparq_text::query_text(
        &g,
        r#"PREFIX text: <http://sparq.dev/text#>
           SELECT ?lit WHERE { ?lit text:matches "東京" }"#,
        &idx,
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "only the co-occurring-bigram doc matches");
}

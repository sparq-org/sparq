//! Index correctness: build -> query -> expected literal ids (case folding,
//! unicode, prefix, AND/OR, what is and is not indexed, BM25 ordering).

use oxrdf::Term;
use sparq_core::Graph;
use sparq_text::{Hit, TextIndex};

/// A graph whose `ex:comment` objects are the given literals (one subject each).
fn graph_of(literals: &[&str]) -> Graph {
    let nt: String = literals
        .iter()
        .enumerate()
        .map(|(i, l)| format!("<http://ex/s{i}> <http://ex/comment> {l} .\n"))
        .collect();
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// The lexical values of the hits, best-first.
fn values(graph: &Graph, hits: &[Hit]) -> Vec<String> {
    hits.iter()
        .map(|h| match graph.dict().term(h.id) {
            Term::Literal(l) => l.value().to_string(),
            other => panic!("hit {} is not a literal: {other}", h.id),
        })
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn and_semantics() {
    let g = graph_of(&[
        r#""The quick brown fox""#,
        r#""the lazy dog""#,
        r#""quick dogs and lazy foxes""#, // no stemming: "fox" != "foxes"
    ]);
    let idx = TextIndex::build(&g);
    assert_eq!(idx.len(), 3);
    assert_eq!(values(&g, &idx.search("quick fox")), ["The quick brown fox"]);
    // Every token must be present.
    assert!(idx.search("quick cat").is_empty());
    // Token order and duplication don't matter.
    assert_eq!(idx.search("fox quick"), idx.search("quick quick fox"));
}

#[test]
fn or_semantics() {
    let g = graph_of(&[r#""alpha beta""#, r#""beta gamma""#, r#""delta""#]);
    let idx = TextIndex::build(&g);
    assert_eq!(
        sorted(values(&g, &idx.search_any("alpha gamma"))),
        ["alpha beta", "beta gamma"]
    );
    // Unknown tokens are ignored under OR (but kill AND).
    assert_eq!(values(&g, &idx.search_any("delta zzz")), ["delta"]);
    assert!(idx.search("delta zzz").is_empty());
}

#[test]
fn case_folding_and_unicode() {
    let g = graph_of(&[
        r#""ΣΟΦΊΑ City""#,
        r#""Большой Театр""#,
        r#""Straße 42""#,
        r#""café"@fr"#,
    ]);
    let idx = TextIndex::build(&g);
    assert_eq!(values(&g, &idx.search("σοφία")), ["ΣΟΦΊΑ City"]);
    assert_eq!(values(&g, &idx.search("большой театр")), ["Большой Театр"]);
    assert_eq!(values(&g, &idx.search("straße")), ["Straße 42"]);
    // Language-tagged literals are indexed; no diacritic folding (café != cafe).
    assert_eq!(values(&g, &idx.search("café")), ["café"]);
    assert!(idx.search("cafe").is_empty());
}

#[test]
fn prefix_tokens() {
    let g = graph_of(&[r#""automatic transmission""#, r#""autonomous driving""#, r#""matic""#]);
    let idx = TextIndex::build(&g);
    assert_eq!(
        sorted(values(&g, &idx.search("auto*"))),
        ["automatic transmission", "autonomous driving"]
    );
    // Prefix + AND.
    assert_eq!(values(&g, &idx.search("auto* driving")), ["autonomous driving"]);
    // The empty prefix never panics (matches everything).
    assert_eq!(idx.search_any("a*").len(), 2);
}

#[test]
fn only_string_literals_are_indexed() {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/p> "plain text" .
        <http://ex/a> <http://ex/p> "tagged text"@en .
        <http://ex/a> <http://ex/p> "42"^^<http://www.w3.org/2001/XMLSchema#integer> .
        <http://ex/a> <http://ex/p> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> .
        <http://ex/a> <http://ex/text> <http://ex/not-text> .
        "#,
        "ntriples",
    )
    .unwrap();
    let idx = TextIndex::build(&g);
    // Exactly the two string literals; IRIs, typed literals never match.
    assert_eq!(idx.len(), 2);
    assert!(idx.search("42").is_empty());
    assert!(idx.search("true").is_empty());
    assert_eq!(sorted(values(&g, &idx.search("text"))), ["plain text", "tagged text"]);
}

#[test]
fn empty_inputs() {
    let empty = Graph::load_str("", "ntriples").unwrap();
    let idx = TextIndex::build(&empty);
    assert!(idx.is_empty());
    assert!(idx.search("anything").is_empty());

    let g = graph_of(&[r#""""#, r#""words here""#]);
    let idx = TextIndex::build(&g);
    // The empty literal is a (zero-length) document; it can never match.
    assert_eq!(idx.len(), 2);
    assert!(idx.search("").is_empty());
    assert!(idx.search("?!,.").is_empty());
    assert_eq!(values(&g, &idx.search("words")), ["words here"]);
}

#[test]
fn bm25_ordering() {
    let g = graph_of(&[
        r#""fox""#,                                        // short doc, tf 1
        r#""fox fox fox""#,                                // higher tf
        r#""fox and a very long sentence about many other animals entirely""#, // long doc
        r#""unrelated""#,
    ]);
    let idx = TextIndex::build(&g);
    let hits = values(&g, &idx.search("fox"));
    assert_eq!(hits.len(), 3);
    // Higher term frequency outranks tf=1; the long document ranks last
    // (length normalisation).
    assert_eq!(hits[0], "fox fox fox");
    assert_eq!(hits[2], "fox and a very long sentence about many other animals entirely");
    // Scores are positive and descending.
    let scores: Vec<f32> = idx.search("fox").iter().map(|h| h.score).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]));
    assert!(scores.iter().all(|s| *s > 0.0));

    // A rare term contributes more than a ubiquitous one: the doc matching
    // the rare term outranks docs matching only the common term under OR.
    let g2 = graph_of(&[r#""common rare""#, r#""common""#, r#""common""#, r#""common""#]);
    let idx2 = TextIndex::build(&g2);
    let hits2 = values(&g2, &idx2.search_any("common rare"));
    assert_eq!(hits2[0], "common rare");
}

#[test]
fn term_stats_match_bm25_corpus_statistics() {
    let g = graph_of(&[r#""fox""#, r#""jumps""#, r#""quick fox""#]);
    let idx = TextIndex::build(&g);

    let fox = idx.term_stats("FOX").expect("fox has postings");
    assert_eq!(fox.doc_count, 2);
    assert_eq!(fox.total_tf, 2);
    let expected_fox_idf = ((3.0_f32 - 2.0 + 0.5) / (2.0 + 0.5) + 1.0).ln();
    assert_eq!(fox.idf, expected_fox_idf);

    let jumps = idx.term_stats("jumps").expect("jumps has postings");
    assert_eq!(jumps.doc_count, 1);
    assert_eq!(jumps.total_tf, 1);
    assert_eq!(idx.term_stats("nonexistent"), None);

    // Exact single-token lookup: query-prefix syntax is not expanded.
    assert_eq!(idx.term_stats("fo*"), None);
    assert_eq!(idx.term_stats("fox jumps"), None);

    // Total term frequency is occurrence-counted, not an alias for document
    // frequency.
    let repeated = TextIndex::build(&graph_of(&[r#""echo echo""#, r#""echo""#]));
    let echo = repeated.term_stats("echo").expect("echo has postings");
    assert_eq!(echo.doc_count, 2);
    assert_eq!(echo.total_tf, 3);
}

#[test]
fn index_counts_posting_pairs() {
    let index = TextIndex::build(&graph_of(&[
        r#""hello world""#,
        r#""hello there""#,
        r#""goodbye""#,
    ]));
    assert_eq!(index.len(), 3);
    assert_eq!(index.token_count(), 4);
    assert_eq!(index.total_postings(), 5);

    let empty = TextIndex::build(&Graph::load_str("", "ntriples").unwrap());
    assert_eq!(empty.total_postings(), 0);

    // Distinct RDF literals with the same lexical form are three documents
    // in the single `cat` posting list.
    let cats = TextIndex::build(&graph_of(&[r#""cat""#, r#""cat"@en"#, r#""cat"@fr"#]));
    assert_eq!(cats.len(), 3);
    assert_eq!(cats.token_count(), 1);
    assert_eq!(cats.total_postings(), 3);
}

#[test]
fn duplicate_literals_are_one_document() {
    // The same literal in many triples is ONE dictionary term = one document.
    let nt = r#"
        <http://ex/a> <http://ex/p> "shared label" .
        <http://ex/b> <http://ex/p> "shared label" .
        <http://ex/c> <http://ex/q> "shared label" .
    "#;
    let g = Graph::load_str(nt, "ntriples").unwrap();
    let idx = TextIndex::build(&g);
    assert_eq!(idx.len(), 1);
    assert_eq!(idx.search("shared").len(), 1);
}

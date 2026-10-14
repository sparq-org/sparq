//! Tokenizer / analyzer EDGE CASES + the delete-then-readd (delta) no-orphan
//! invariant.
//!
//! These pin behaviours the audit flagged as thin: Unicode normalization (NFC vs
//! NFD are DISTINCT — the analyzer casefolds but does not normalize), CJK /
//! no-whitespace text (UAX #29 splits Han ideographs per character — there is no
//! dictionary segmentation), casefolding corners (German ß, Greek final sigma,
//! Turkish dotted-I, fullwidth Latin), punctuation at phrase boundaries, and an
//! incremental delete-then-readd that must leave NO orphan postings (querying a
//! deleted-then-readded document returns the correct hits, and the index stays
//! equal to a rebuild).

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_text::tokenize::tokenize;
use sparq_text::TextIndex;

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

// ---- Unicode normalization -------------------------------------------------------------

#[test]
fn nfc_and_nfd_are_distinct_tokens() {
    // The analyzer casefolds (lowercases) but does NOT Unicode-normalize, so a
    // precomposed "é" (U+00E9) and a decomposed "e"+combining-acute (U+0301)
    // tokenize to DIFFERENT strings — the documented "no diacritic folding"
    // stance taken to its logical end. Pinned so a future normalization change
    // is a conscious, tested decision.
    let nfc = "café"; // precomposed
    let nfd = "cafe\u{0301}"; // decomposed
    assert_ne!(tokenize(nfc), tokenize(nfd), "NFC and NFD must not collapse");

    let g = graph_of(&[&format!(r#""{nfc}""#), &format!(r#""{nfd}""#)]);
    let idx = TextIndex::build(&g);
    // Two distinct documents; a precomposed query hits only the precomposed doc.
    assert_eq!(idx.len(), 2);
    assert_eq!(idx.search(nfc).len(), 1);
    assert_eq!(idx.search(nfd).len(), 1);
    // …and the two queries never return the same document.
    assert_ne!(idx.search(nfc)[0].id, idx.search(nfd)[0].id);
}

// ---- CJK / no-whitespace ---------------------------------------------------------------

#[test]
fn cjk_han_splits_per_ideograph() {
    // No dictionary segmentation: UAX #29 emits each Han ideograph as its own
    // word, so "東京都" is three tokens. (Pins the corrected tokenize.rs doc.)
    assert_eq!(tokenize("東京都"), ["東", "京", "都"]);
    // Mixed CJK + Latin: the Latin run stays one token, each ideograph its own.
    assert_eq!(tokenize("東京Tokyo"), ["東", "京", "tokyo"]);

    let g = graph_of(&[r#""東京都に住む""#, r#""北京市""#, r#""京都府""#]);
    let idx = TextIndex::build(&g);
    // A single-ideograph query "京" hits every document containing that char.
    // (`values` sorts by Rust string order = codepoint: 京 < 北 < 東.)
    assert_eq!(values(&g, &idx.search("京")), ["京都府", "北京市", "東京都に住む"]);
    // A multi-char CJK term is the AND of its ideographs (co-occurrence, any order).
    // "東" and "都" both occur only in the first doc.
    assert_eq!(values(&g, &idx.search("東都")), ["東京都に住む"]);
    // "京" + "都" co-occur in two docs (東京都に住む has both; 京都府 has both).
    assert_eq!(values(&g, &idx.search("京都")), ["京都府", "東京都に住む"]);
}

#[test]
fn cjk_phrase_requires_adjacency() {
    // For an ORDERED, adjacent CJK term, use phrase: it requires the ideographs
    // consecutive and in order, so it distinguishes 東京 from 京東 and from a
    // doc where the two chars merely co-occur.
    let g = graph_of(&[r#""東京都""#, r#""京都の東""#, r#""北京市""#]);
    let idx = TextIndex::build_with_positions(&g);
    // Only the doc with adjacent 東→京 matches the phrase "東京".
    let mut hits = idx.phrase("東京");
    hits.sort_unstable();
    let vals: Vec<String> = hits
        .iter()
        .map(|&id| match g.dict.term(id) {
            Term::Literal(l) => l.value().to_string(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(vals, ["東京都"]);
    // The reversed phrase 京東 matches nothing (order is significant).
    assert!(idx.phrase("京東").is_empty());
}

// ---- Casefolding corners ---------------------------------------------------------------

#[test]
fn casefolding_corners() {
    // German ß: STRASSE lowercases to "strasse" (NOT "straße"), so they are
    // distinct tokens — documented, deliberate (no special-casing).
    assert_eq!(tokenize("STRASSE"), ["strasse"]);
    assert_eq!(tokenize("Straße"), ["straße"]);
    assert_ne!(tokenize("STRASSE"), tokenize("Straße"));

    // Greek final sigma: `str::to_lowercase` applies the Unicode final-sigma
    // SpecialCasing rule, so a word-final Σ folds to ς (not medial σ): "ΟΔΟΣ" ->
    // "οδος". A medial Σ stays σ ("ΣΟΦΊΑ" -> "σοφία", pinned in index.rs tests).
    assert_eq!(tokenize("ΟΔΟΣ"), ["οδος"]);

    // Turkish dotted capital İ lowercases to ASCII i + combining dot (U+0307),
    // locale-INDEPENDENTLY (no Turkish tailoring) — so "İ" != plain "i".
    assert_eq!(tokenize("İ"), ["i\u{0307}"]);
    assert_ne!(tokenize("İstanbul"), tokenize("istanbul"));

    // Fullwidth Latin lowercases within the fullwidth block — it does NOT fold to
    // ASCII, so fullwidth ＦＯＸ never matches ASCII "fox".
    assert_eq!(tokenize("ＦＯＸ"), ["ｆｏｘ"]);
    assert_ne!(tokenize("ＦＯＸ"), tokenize("FOX"));

    // Search reflects all of the above: an ASCII "fox" query never hits the
    // fullwidth document.
    let g = graph_of(&[r#""ＦＯＸ""#, r#""fox""#]);
    let idx = TextIndex::build(&g);
    assert_eq!(values(&g, &idx.search("fox")), ["fox"]);
    assert_eq!(values(&g, &idx.search("ｆｏｘ")), ["ＦＯＸ"]);
}

// ---- Punctuation at phrase boundaries --------------------------------------------------

#[test]
fn punctuation_at_phrase_boundaries() {
    // Punctuation is segmentation, never a token, so it cannot break an otherwise
    // adjacent phrase NOR create spurious adjacency. "quick, brown" tokenizes to
    // ["quick","brown"] and a doc "quick brown" matches it.
    let g = graph_of(&[
        r#""quick brown fox""#,        // clean
        r#""quick, brown! fox.""#,     // heavy punctuation between every word
        r#""quick — brown — dog""#,    // em-dash separators
        r#""quickbrown fox""#,         // joined: ONE token "quickbrown", no "quick"/"brown"
    ]);
    let idx = TextIndex::build_with_positions(&g);

    // The punctuation-heavy doc still matches the adjacency, just like the clean one.
    // (sq-qmth) Compare by VALUE, not by dict id: `Graph::load_str` takes the
    // SHARDED parallel-merge path on a multi-core host (>=2 rayon threads), which assigns
    // dictionary ids by hash bucket, NOT document order. Sorting `hits` (a `Vec<Id>`) and
    // mapping to values therefore yields a host-dependent order — green locally, RED in CI.
    // Sort the resulting values lexically so the assertion is id-order-independent.
    let hits = idx.phrase("quick brown");
    let mut vals: Vec<String> = hits
        .iter()
        .map(|&id| match g.dict.term(id) {
            Term::Literal(l) => l.value().to_string(),
            _ => unreachable!(),
        })
        .collect();
    vals.sort();
    // Expected list is in byte-lexicographic order (how `Vec<String>::sort` orders them):
    // index 5 is space(0x20) < comma(0x2C), so the comma doc sorts LAST; between the two
    // space docs, 'b'(0x62) < the em-dash's leading UTF-8 byte(0xE2).
    assert_eq!(vals, ["quick brown fox", "quick — brown — dog", "quick, brown! fox."]);

    // A phrase query with leading/trailing/embedded punctuation tokenizes the
    // same way and matches identically. Sort BOTH id lists before comparing so the
    // equality is independent of the (hash-sharded) dict-id assignment order.
    let mut hits2 = idx.phrase("  quick,  brown  ");
    hits2.sort_unstable();
    let mut hits_sorted = hits;
    hits_sorted.sort_unstable();
    assert_eq!(hits2, hits_sorted);

    // The joined token is NOT split — "quick" alone never matches "quickbrown".
    assert!(idx.search("quick").iter().all(|h| match g.dict.term(h.id) {
        Term::Literal(l) => l.value() != "quickbrown fox",
        _ => true,
    }));
}

// ---- Incremental delete-then-readd: NO orphan postings ---------------------------------

/// Insert a string-literal triple, delete it, then re-insert the SAME literal —
/// the index must still answer the query correctly (no orphaned/missing
/// postings), and equal a from-scratch rebuild of the final graph.
#[test]
fn delete_then_readd_leaves_no_orphan_postings() {
    let p = Term::NamedNode(NamedNode::new_unchecked("http://ex/p"));
    let s = Term::NamedNode(NamedNode::new_unchecked("http://ex/doc"));
    let lit = |v: &str| Term::Literal(Literal::new_simple_literal(v));
    let triple = |o: Term| [s.clone(), p.clone(), o];

    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut idx = TextIndex::build(&graph);

    // 1. Insert "the quick brown fox".
    let ins = [triple(lit("the quick brown fox"))];
    graph.apply_delta(&ins, &[]).unwrap();
    idx.apply_delta(&graph, &ins, &[]);
    assert_eq!(idx.search("quick fox").len(), 1, "indexed after insert");

    // 2. Delete it. apply_delta's deletes are a documented no-op (the dictionary
    //    retains the term, so the index equals a rebuild — which still contains
    //    the document, since the dictionary still holds the literal id).
    graph.apply_delta(&[], &ins).unwrap();
    idx.apply_delta(&graph, &[], &ins);
    assert_eq!(idx, TextIndex::build(&graph), "incremental == rebuilt after delete");

    // 3. Re-add the SAME literal (a new triple with the same object). The literal
    //    id is unchanged (dict retains it), so this must NOT create a duplicate
    //    posting or a second document — and the query still returns exactly one
    //    correct hit.
    let readd = [[
        Term::NamedNode(NamedNode::new_unchecked("http://ex/doc2")),
        p.clone(),
        lit("the quick brown fox"),
    ]];
    graph.apply_delta(&readd, &[]).unwrap();
    idx.apply_delta(&graph, &readd, &[]);

    let hits = idx.search("quick fox");
    assert_eq!(hits.len(), 1, "exactly one document after delete-then-readd");
    assert_eq!(
        values(&graph, &hits),
        ["the quick brown fox"],
        "the deleted+readded document returns the correct hit"
    );
    // No orphan/duplicate posting: token frequency is what a fresh rebuild has.
    assert_eq!(idx, TextIndex::build(&graph), "incremental == rebuilt after readd");

    // The literal occupies a single dictionary document throughout (no orphan id
    // proliferation): the only indexed doc is the one literal.
    assert_eq!(idx.len(), 1);
}

/// A harder churn: many distinct literals inserted, a subset deleted, then the
/// EXACT deleted literals re-added — every original document must still be
/// queryable and the index byte-equal to a rebuild (full-state `PartialEq`).
#[test]
fn churned_delete_readd_matches_rebuild() {
    let p = Term::NamedNode(NamedNode::new_unchecked("http://ex/p"));
    let lit = |v: &str| Term::Literal(Literal::new_simple_literal(v));
    let words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"];

    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut idx = TextIndex::build(&graph);

    // Insert eight distinct one-word documents.
    let inserts: Vec<[Term; 3]> = words
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
    assert_eq!(idx.len(), words.len());

    // Delete the even-indexed ones, then re-add them (same literals).
    let deleted: Vec<[Term; 3]> = inserts.iter().step_by(2).cloned().collect();
    graph.apply_delta(&[], &deleted).unwrap();
    idx.apply_delta(&graph, &[], &deleted);
    graph.apply_delta(&deleted, &[]).unwrap();
    idx.apply_delta(&graph, &deleted, &[]);

    // Every original word is still found exactly once — no orphans, no dupes.
    for w in words {
        let hits = idx.search(w);
        assert_eq!(hits.len(), 1, "word {w:?} should resolve to one document");
        assert_eq!(values(&graph, &hits), [w.to_string()]);
    }
    assert_eq!(idx, TextIndex::build(&graph), "incremental == rebuilt after churn");
    assert_eq!(idx.len() as Id, words.len() as Id);
}

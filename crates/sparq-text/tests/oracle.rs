//! Brute-force inverted-index ORACLE for [`TextIndex`].
//!
//! Over many random documents we assert the index's query answers agree with an
//! INDEPENDENT brute-force scan of the raw documents — the oracle-vs-index shape
//! sparq-vectors uses in `tests/recall.rs`. The brute force re-tokenizes each
//! document with the SAME public tokenizer and answers each query directly:
//!
//!  * `search` (AND) — every query token present (prefix `*` = some token has it
//!    as a prefix);
//!  * `search_any` (OR) — at least one query token present;
//!  * `phrase` — the query's tokens occur at consecutive positions, in order.
//!  * `phrase_near` — the query's tokens occur IN ORDER within a bounded total
//!    gap (slop), relevance-scored `1/(1+gap)`; the oracle recomputes the
//!    smallest in-order gap per document independently.
//!
//! For ranked queries we compare the hit-ID SET (not BM25 scores — the prompt's
//! "SET, not exact scores" rule); for `phrase` we compare the exact ascending ID
//! list the index returns. For `phrase_near` the proximity score is a closed
//! form of the oracle's min-gap, so there we DO pin the exact per-doc score (and
//! the best-first ordering). Because the index keys documents by dictionary term
//! id, the oracle scans the dictionary's string literals (the documents), so
//! duplicate literals collapse to one document identically on both sides.

use oxrdf::Term;
use rand::rngs::StdRng;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{RngExt, SeedableRng};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_text::TextIndex;

/// A small mixed vocabulary: ASCII words (with shared prefixes so `*` queries
/// have something to expand), repeats, plus a few Unicode/CJK tokens so the
/// oracle exercises the same casefolding/segmentation the index uses.
const VOCAB: &[&str] = &[
    "auto", "automatic", "autonomous", "autobahn", // shared "auto" prefix
    "brown", "fox", "quick", "lazy", "dog", "jumps", "over", "the", "data", "graph",
    "café", "straße", "ΣΟΦΊΑ", "Большой", "東京", "北京市", // unicode + CJK
];

/// A random document: 0..=10 vocab tokens joined by spaces/punctuation (so the
/// segmenter — not raw `split`—decides token boundaries), occasionally empty.
fn random_doc(rng: &mut StdRng) -> String {
    let n = rng.random_range(0..=10);
    let seps = [" ", ", ", " — ", "! ", ". ", "\t"];
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push_str(seps[rng.random_range(0..seps.len())]);
        }
        s.push_str(VOCAB[rng.random_range(0..VOCAB.len())]);
    }
    s
}

/// Build a graph whose `ex:comment` objects are the given literals (one subject
/// each). DISTINCT documents only (the caller dedups) so doc count == lit count.
fn graph_of(literals: &[String]) -> Graph {
    let nt: String = literals
        .iter()
        .enumerate()
        // Escape the few literal chars NT cares about; our vocab has none, but
        // the separators are all safe and quotes/backslashes never appear.
        .map(|(i, l)| format!("<http://ex/s{i}> <http://ex/comment> \"{l}\" .\n"))
        .collect();
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// The set of string-literal documents the index sees, as `(doc id, tokens)` —
/// re-tokenized INDEPENDENTLY with the public tokenizer. This is the oracle's
/// view of the corpus; it must enumerate exactly what `TextIndex::build` indexes
/// (plain `xsd:string` + language-tagged literals).
fn corpus(graph: &Graph) -> Vec<(Id, Vec<String>)> {
    let mut out = Vec::new();
    for id in 1..=graph.dict().len() as Id {
        if let Term::Literal(l) = graph.dict().term(id) {
            // Mirror TextIndex's "string literal" test: plain/xsd:string or lang-tagged.
            let is_string = l.language().is_some()
                || l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string";
            if is_string {
                out.push((id, sparq_text::tokenize::tokenize(l.value())));
            }
        }
    }
    out
}

/// One parsed query token for the oracle: lowercased text + prefix flag, exactly
/// as the index's `tokenize_query` would parse it (we reuse that very function).
fn query_tokens(query: &str) -> Vec<(String, bool)> {
    sparq_text::tokenize::tokenize_query(query)
        .into_iter()
        .map(|t| (t.token, t.prefix))
        .collect()
}

/// Does `tokens` (a document) contain the query token (prefix-aware)?
fn doc_has(tokens: &[String], (qt, prefix): &(String, bool)) -> bool {
    if *prefix {
        tokens.iter().any(|t| t.starts_with(qt.as_str()))
    } else {
        tokens.iter().any(|t| t == qt)
    }
}

/// Brute-force AND: doc ids whose tokens contain EVERY query token. Empty query
/// (no word tokens) matches nothing — the index's documented behaviour.
fn brute_and(corpus: &[(Id, Vec<String>)], query: &str) -> Vec<Id> {
    let qs = query_tokens(query);
    if qs.is_empty() {
        return Vec::new();
    }
    let mut ids: Vec<Id> = corpus
        .iter()
        .filter(|(_, toks)| qs.iter().all(|q| doc_has(toks, q)))
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Brute-force OR: doc ids whose tokens contain AT LEAST ONE query token.
fn brute_any(corpus: &[(Id, Vec<String>)], query: &str) -> Vec<Id> {
    let qs = query_tokens(query);
    if qs.is_empty() {
        return Vec::new();
    }
    let mut ids: Vec<Id> = corpus
        .iter()
        .filter(|(_, toks)| qs.iter().any(|q| doc_has(toks, q)))
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Brute-force phrase: doc ids where the query's (non-prefix) tokens occur at
/// consecutive positions, in order. A trailing `*` is plain text here (phrase
/// does NOT treat `*` as a prefix marker), so we tokenize the phrase as a
/// document. Empty phrase matches nothing.
fn brute_phrase(corpus: &[(Id, Vec<String>)], query: &str) -> Vec<Id> {
    let needle = sparq_text::tokenize::tokenize(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let mut ids: Vec<Id> = corpus
        .iter()
        .filter(|(_, toks)| {
            toks.windows(needle.len()).any(|w| w == needle.as_slice())
                || (needle.len() == 1 && toks.contains(&needle[0]))
        })
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

/// The smallest in-order phrase gap of `needle` within a document's `tokens`,
/// computed by an INDEPENDENT exhaustive scan (the proximity oracle): for every
/// occurrence of token 0, greedily take the earliest later occurrence of each
/// subsequent token (positions are in document order), and keep the smallest
/// `(last − first) − (n − 1)`. `None` if the tokens never occur in order.
fn brute_min_gap(tokens: &[String], needle: &[String]) -> Option<u32> {
    if needle.len() == 1 {
        return tokens.contains(&needle[0]).then_some(0);
    }
    // Positions of each needle token within the document, in document order.
    let pos: Vec<Vec<usize>> = needle
        .iter()
        .map(|t| tokens.iter().enumerate().filter(|(_, w)| *w == t).map(|(i, _)| i).collect())
        .collect();
    let mut best: Option<u32> = None;
    for &start in &pos[0] {
        let mut prev = start;
        let mut ok = true;
        for ps in &pos[1..] {
            match ps.iter().copied().find(|&p| p > prev) {
                Some(next) => prev = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let gap = (prev - start - (needle.len() - 1)) as u32;
            best = Some(best.map_or(gap, |b| b.min(gap)));
        }
    }
    best
}

/// Brute-force proximity: `(doc id, min gap)` for docs where the query's tokens
/// occur in order within `slop` total gap. A trailing `*` is plain text (no
/// prefix marker), as for phrase; empty phrase matches nothing.
fn brute_phrase_near(corpus: &[(Id, Vec<String>)], query: &str, slop: u32) -> Vec<(Id, u32)> {
    let needle = sparq_text::tokenize::tokenize(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(Id, u32)> = corpus
        .iter()
        .filter_map(|(id, toks)| brute_min_gap(toks, &needle).filter(|&g| g <= slop).map(|g| (*id, g)))
        .collect();
    out.sort_unstable();
    out
}

/// The hit ids of a ranked search, sorted (we compare the SET, not the order or
/// BM25 scores — see the module docs).
fn hit_ids(hits: &[sparq_text::Hit]) -> Vec<Id> {
    let mut v: Vec<Id> = hits.iter().map(|h| h.id).collect();
    v.sort_unstable();
    v
}

/// A spread of queries: single/multi-token, prefix, casefolded, Unicode, CJK,
/// absent tokens, empty/punctuation-only. Reused for AND/OR; the phrase set is
/// separate (no `*` semantics).
const RANKED_QUERIES: &[&str] = &[
    "fox",
    "quick fox",
    "the quick brown fox",
    "auto*",
    "auto* dog",
    "AUTO*",       // casefold
    "café",
    "straße",
    "σοφία",       // matches ΣΟΦΊΑ after casefold
    "большой",
    "東京",
    "北京市",
    "missing",     // absent token
    "fox missing", // AND with absent -> empty; OR -> fox's docs
    "",            // empty
    " ,.;! ",      // punctuation only
    "fox fox",     // duplicate token must not change membership
];

const PHRASE_QUERIES: &[&str] = &[
    "quick brown",
    "quick brown fox",
    "the quick",
    "fox quick", // reversed order
    "auto",      // single token = presence
    "café straße",
    "東京",
    "missing phrase",
    "",
    "  ,.; ",
];

#[test]
fn brute_force_oracle_over_random_documents() {
    let mut rng = StdRng::seed_from_u64(0x000B_F548_u64 ^ 0x00C0_FFEE);
    // Several independent corpora of growing size, each freshly seeded.
    for &n in &[5usize, 25, 120, 600] {
        // Random documents plus one deliberate DUPLICATE literal — the dictionary
        // collapses repeats into a single document, and the oracle (which scans
        // the dictionary, not the raw triples) must agree. Random repeats among
        // the rest are fine for the same reason: both sides see the dictionary.
        let mut docs: Vec<String> = (0..n).map(|_| random_doc(&mut rng)).collect();
        if let Some(first) = docs.first().cloned() {
            docs.push(first); // a duplicate literal — one dictionary term
        }
        let graph = graph_of(&docs);

        let plain = TextIndex::build(&graph);
        let positional = TextIndex::build_with_positions(&graph);
        let corp = corpus(&graph);

        // The oracle's document count must match the index's (dictionary dedup).
        assert_eq!(plain.len(), corp.len(), "doc-count mismatch at n={n}");

        for &q in RANKED_QUERIES {
            assert_eq!(
                hit_ids(&plain.search(q)),
                brute_and(&corp, q),
                "AND search({q:?}) != brute force at n={n}"
            );
            assert_eq!(
                hit_ids(&plain.search_any(q)),
                brute_any(&corp, q),
                "OR search_any({q:?}) != brute force at n={n}"
            );
            // Positions are a pure add-on: the ranked paths must be identical.
            assert_eq!(hit_ids(&positional.search(q)), hit_ids(&plain.search(q)));
            assert_eq!(hit_ids(&positional.search_any(q)), hit_ids(&plain.search_any(q)));
        }

        for &q in PHRASE_QUERIES {
            let mut got = positional.phrase(q);
            got.sort_unstable();
            assert_eq!(got, brute_phrase(&corp, q), "phrase({q:?}) != brute force at n={n}");

            // Proximity/slop: at every slop level the index must agree with the
            // brute-force min-gap oracle on BOTH the id set AND the per-doc gap
            // (encoded in the score 1/(1+gap)), and rank tightest-first.
            for slop in [0u32, 1, 2, 4, 8] {
                let hits = positional.phrase_near(q, slop);
                let oracle = brute_phrase_near(&corp, q, slop);

                // Same id set.
                assert_eq!(
                    hit_ids(&hits),
                    oracle.iter().map(|&(id, _)| id).collect::<Vec<_>>(),
                    "phrase_near({q:?}, {slop}) id set != brute force at n={n}"
                );
                // Each score is exactly 1/(1+oracle_gap) for that doc.
                let want: std::collections::HashMap<Id, u32> = oracle.iter().copied().collect();
                for h in &hits {
                    let g = want[&h.id];
                    assert_eq!(
                        h.score,
                        1.0 / (1.0 + g as f32),
                        "phrase_near({q:?}, {slop}) score for {} != 1/(1+{g}) at n={n}",
                        h.id
                    );
                }
                // Ranked tightest-first: non-increasing score (== non-decreasing gap).
                assert!(
                    hits.windows(2).all(|w| w[0].score >= w[1].score),
                    "phrase_near({q:?}, {slop}) not ranked best-first at n={n}"
                );
            }
            // slop 0 reproduces exact-adjacency `phrase` (gap 0 = adjacency).
            assert_eq!(
                hit_ids(&positional.phrase_near(q, 0)),
                got,
                "phrase_near({q:?}, 0) != phrase({q:?}) at n={n}"
            );
        }
    }
}

/// BM25 ranking is deterministic given the index, so when two documents have
/// genuinely different match strength the oracle can assert the ORDER too (not
/// just the set) — a stronger check the membership oracle above intentionally
/// avoids. Higher tf and rarer terms must rank ahead; equal-strength docs tie by
/// ascending id (the index's documented tiebreak).
#[test]
fn bm25_ordering_is_deterministic_and_tie_breaks_by_id() {
    // doc0: "fox" x3 (high tf); doc1: "fox" once; doc2: "fox" in a long doc.
    let docs = vec![
        "fox fox fox".to_string(),
        "fox".to_string(),
        "fox among many other unrelated trailing filler words here now".to_string(),
        "nothing".to_string(),
    ];
    let g = graph_of(&docs);
    let idx = TextIndex::build(&g);
    let hits = idx.search("fox");
    let ranked: Vec<String> = hits
        .iter()
        .map(|h| match g.dict().term(h.id) {
            Term::Literal(l) => l.value().to_string(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ranked[0], "fox fox fox", "highest tf ranks first");
    assert_eq!(
        ranked.last().unwrap(),
        "fox among many other unrelated trailing filler words here now",
        "length normalisation pushes the long doc last"
    );
    // Scores are non-increasing.
    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));

    // Tie-break by id: two DIFFERENT two-token docs that each contain "alpha"
    // once have identical length and tf → equal BM25 → must tie by ascending id.
    let same = vec!["alpha one".to_string(), "alpha two".to_string()];
    let g2 = graph_of(&same);
    let idx2 = TextIndex::build(&g2);
    let hits2 = idx2.search("alpha");
    assert_eq!(hits2.len(), 2);
    assert_eq!(hits2[0].score, hits2[1].score, "equal-strength docs tie on score");
    assert!(hits2[0].id < hits2[1].id, "ties break by ascending id");
}

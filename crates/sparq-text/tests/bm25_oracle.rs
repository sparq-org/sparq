//! sq-ripcg — DIFFERENTIAL BM25 ORACLE for [`TextIndex`]'s ranked
//! search, wired into the central conformance scoreboard as a sparq-EXTENSION
//! ratchet (NOT a W3C/standards conformance claim — there is no normative
//! full-text-over-RDF or BM25 conformance suite to point at; this asserts
//! sparq's OWN scoring against an independent reference).
//!
//! ## What this pins (and how it differs from `oracle.rs`)
//!
//! The sibling `tests/oracle.rs` is a MEMBERSHIP oracle: it brute-forces the
//! AND/OR/phrase HIT SETS and (for `phrase_near`) the closed-form proximity
//! score, but it deliberately does NOT check the BM25 *scores* of `search` /
//! `search_any` — its module docs spell out "we compare the SET, not the order
//! or BM25 scores". So the actual relevance arithmetic — the idf, the
//! tf-saturation, the document-length normalisation, the score SUM across query
//! terms, and the best-first ordering — is unchecked there.
//!
//! This test closes that gap with a DIFFERENTIAL oracle: a from-scratch,
//! independent BM25 scorer (`RefScorer` below) recomputes the exact expected
//! score of every hit directly from the tokenized corpus, and we assert the
//! index reproduces — to the bit, via `f32::to_bits` — the same per-hit score
//! AND the same best-first ranking. The reference shares NOTHING with the
//! index's posting-list machinery: it builds its own `doc -> {token -> tf}` and
//! `doc -> length` tables by re-tokenizing the documents with the public
//! tokenizer, then applies the Robertson/Spärck-Jones BM25 closed form. If the
//! index's scoring path drifts (a parameter, the idf floor, the length-norm,
//! the prefix-union tf aggregation, the multi-term sum), the two diverge and
//! the ratchet fails.
//!
//! ## Why an EXTENSION ratchet, not a conformance one
//!
//! BM25 is an information-retrieval scoring function (Robertson & Spärck Jones),
//! NOT a W3C/OGC/IETF standard, and there is no normative "full-text search over
//! RDF" recommendation or its accompanying test suite. So the floor here is
//! HONESTLY a sparq-extension self-consistency ratchet — "sparq's BM25 still
//! matches sparq's own documented BM25 closed form on a fixed corpus" — and the
//! central scoreboard labels it `family = sparq extension`, never a conformance
//! claim. The floor is the ACTUAL measured count of score-exact assertions the
//! fixed query battery makes; it may only RISE (more queries / corpora), and a
//! drop means a genuine scoring regression.
//!
//! sq-z1xv8 — the floor is no longer MIRRORED anywhere. It is the one
//! `sparq_conformance_floors::text::BM25_ORACLE_FLOOR` const, which the central
//! `sparq_conformance::scoreboard::SUITES` row imports too, so the enforced and the
//! reported floor cannot drift by construction (previously the conformance crate's
//! `tests/scoreboard_floors.rs` guard re-read this file's source). The dev-only
//! conformance crate still takes NO dependency edge on sparq-text: the shared floors
//! crate is a zero-dependency leaf both sides depend on.

use oxrdf::Term;
use rand::rngs::StdRng;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{RngExt, SeedableRng};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_text::TextIndex;
use std::collections::BTreeMap;

/// The MINIMUM number of score-exact differential assertions the fixed query
/// battery below makes against the reference BM25 scorer. It is the actual
/// measured count — every `(corpus × query × mode)` triple that produces hits
/// contributes one assertion per hit. May only RISE (the standard ratchet
/// rule); the value lives in `sparq-conformance-floors` and is SHARED with the
/// central conformance scoreboard (`sparq-conformance` `scoreboard::SUITES`, the
/// `text-search differential oracle` row), so the two read one const and cannot
/// drift. The pinned value is the ACTUAL
/// measured count over the fixed-seed corpora + query battery below (printed as
/// `text-search differential oracle assertions N (floor F)`), not a round
/// number. sq-ripcg
/// sq-z1xv8 — the VALUE now lives once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floor and the reported floor are
/// ONE `const` and cannot drift (replacing the old textual re-read of this file).
/// Raise it THERE; the measurement narrative stays here.
const TEXT_ORACLE_FLOOR: usize = sparq_conformance_floors::text::BM25_ORACLE_FLOOR;

/// BM25 parameters — MUST match `TextIndex`'s (`crate::index`): k1 = 1.2,
/// b = 0.75, Robertson/Spärck-Jones idf with the `+1` floor. Re-declared here
/// (not imported) on purpose: this is an INDEPENDENT reference, so a silent
/// edit to the index's constants must surface as a divergence, not be inherited.
const K1: f32 = 1.2;
const B: f32 = 0.75;

/// A small mixed vocabulary with shared prefixes (so `*` queries expand), repeats
/// (so tf varies), and Unicode/CJK (so casefolding/segmentation is exercised).
const VOCAB: &[&str] = &[
    "auto", "automatic", "autonomous", "autobahn", // shared "auto" prefix
    "brown", "fox", "quick", "lazy", "dog", "jumps", "over", "the", "data", "graph",
    "café", "straße", "ΣΟΦΊΑ", "Большой", "東京", "北京市", // unicode + CJK
];

/// A random document: 0..=12 vocab tokens joined by varied separators (so the
/// UAX #29 segmenter, not a naive split, decides boundaries), occasionally empty.
fn random_doc(rng: &mut StdRng) -> String {
    let n = rng.random_range(0..=12);
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
/// each). The dictionary collapses duplicate literals into one document, which
/// both the index and the reference see identically.
fn graph_of(literals: &[String]) -> Graph {
    let nt: String = literals
        .iter()
        .enumerate()
        .map(|(i, l)| format!("<http://ex/s{i}> <http://ex/comment> \"{l}\" .\n"))
        .collect();
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// The reference corpus: `(doc id, tokens)` for every string literal in the
/// graph, re-tokenized INDEPENDENTLY with the public tokenizer. This mirrors
/// exactly what `TextIndex::build` indexes (plain `xsd:string` + lang-tagged).
fn corpus(graph: &Graph) -> Vec<(Id, Vec<String>)> {
    let mut out = Vec::new();
    for id in 1..=graph.dict().len() as Id {
        if let Term::Literal(l) = graph.dict().term(id) {
            let is_string = l.language().is_some()
                || l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string";
            if is_string {
                out.push((id, sparq_text::tokenize::tokenize(l.value())));
            }
        }
    }
    out
}

/// One parsed query token for the reference: lowercased text + prefix flag,
/// parsed by the SAME public `tokenize_query` the index uses (the query SYNTAX
/// — `*` prefix, casefolding — is part of the contract, not the thing under
/// test; the SCORING is).
fn query_tokens(query: &str) -> Vec<(String, bool)> {
    sparq_text::tokenize::tokenize_query(query)
        .into_iter()
        .map(|t| (t.token, t.prefix))
        .collect()
}

/// An independent inverted view of the corpus the reference scorer needs:
/// document lengths (token counts), the document count, the average document
/// length, and per-document term frequencies. Built fresh from the tokenized
/// documents, so it shares no code with `TextIndex`'s posting lists.
struct RefScorer {
    /// doc id -> document length (token count).
    len: BTreeMap<Id, u32>,
    /// number of documents.
    n: usize,
    /// average document length.
    avgdl: f32,
    /// doc id -> token -> tf (term frequency) — the raw aggregation.
    tf: BTreeMap<Id, BTreeMap<String, u32>>,
}

impl RefScorer {
    fn new(corpus: &[(Id, Vec<String>)]) -> RefScorer {
        let mut len = BTreeMap::new();
        let mut tf: BTreeMap<Id, BTreeMap<String, u32>> = BTreeMap::new();
        let mut total: u64 = 0;
        for (id, toks) in corpus {
            len.insert(*id, toks.len() as u32);
            total += toks.len() as u64;
            let m = tf.entry(*id).or_default();
            for t in toks {
                *m.entry(t.clone()).or_insert(0) += 1;
            }
        }
        let n = corpus.len();
        // Mirror the index's avgdl floor (avoid a divide-by-zero on an all-empty
        // corpus); for n == 0 the scorer is never consulted.
        let avgdl = if n == 0 {
            f32::MIN_POSITIVE
        } else {
            (total as f32 / n as f32).max(f32::MIN_POSITIVE)
        };
        RefScorer { len, n, avgdl, tf }
    }

    /// Resolve a query token to `(per-doc tf)` exactly as the index does: a plain
    /// token reads its own postings; a `*`-prefix token UNIONS every expansion
    /// (tf summed per doc, df = the union's document count). Computed here
    /// directly from the per-doc tf aggregation, not from the index.
    fn resolve(&self, (qt, prefix): &(String, bool)) -> BTreeMap<Id, u32> {
        let mut out: BTreeMap<Id, u32> = BTreeMap::new();
        for (id, m) in &self.tf {
            let mut summed = 0u32;
            for (tok, &f) in m {
                let hit = if *prefix { tok.starts_with(qt.as_str()) } else { tok == qt };
                if hit {
                    summed += f;
                }
            }
            if summed > 0 {
                out.insert(*id, summed);
            }
        }
        out
    }

    /// The BM25 score contribution of one resolved query term (with document
    /// frequency `df`) to document `id` carrying term frequency `tf` — the
    /// Robertson/Spärck-Jones closed form the index documents.
    fn term_score(&self, df: usize, id: Id, tf: u32) -> f32 {
        let n = self.n as f32;
        let df = df as f32;
        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        let dl = self.len[&id] as f32;
        let tf = tf as f32;
        idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / self.avgdl))
    }

    /// The reference ranked answer for `query`: `all` = AND (every query term
    /// present), else OR (score sums over present terms). Returns
    /// `(doc id -> expected score)` for the hit set — the index must reproduce
    /// each score to the bit and rank by it (ties by ascending id).
    fn answer(&self, query: &str, all: bool) -> BTreeMap<Id, f32> {
        // Dedup query tokens — a repeated token must not double-score
        // ("fox fox" == "fox"), exactly the index's `dedup` contract.
        let mut qs = query_tokens(query);
        qs.dedup();
        let mut seen: Vec<(String, bool)> = Vec::new();
        for q in qs {
            if !seen.contains(&q) {
                seen.push(q);
            }
        }
        if seen.is_empty() || self.n == 0 {
            return BTreeMap::new();
        }
        // Resolve each (present) term; AND with an absent term matches nothing.
        let mut resolved: Vec<BTreeMap<Id, u32>> = Vec::new();
        for q in &seen {
            let rows = self.resolve(q);
            if rows.is_empty() {
                if all {
                    return BTreeMap::new();
                }
                continue;
            }
            resolved.push(rows);
        }
        if resolved.is_empty() {
            return BTreeMap::new();
        }
        // doc id -> (accumulated score, number of query terms present).
        let mut acc: BTreeMap<Id, (f32, u32)> = BTreeMap::new();
        for rows in &resolved {
            let df = rows.len();
            for (&id, &tf) in rows {
                let s = self.term_score(df, id, tf);
                let e = acc.entry(id).or_insert((0.0, 0));
                e.0 += s;
                e.1 += 1;
            }
        }
        let need = resolved.len() as u32;
        acc.into_iter()
            .filter(|(_, (_, cnt))| !all || *cnt == need)
            .map(|(id, (score, _))| (id, score))
            .collect()
    }
}

/// A spread of queries: single/multi-token, prefix, casefolded, Unicode, CJK,
/// absent tokens, mixed-strength, empty/punctuation-only. Reused for AND + OR.
const QUERIES: &[&str] = &[
    "fox",
    "quick fox",
    "the quick brown fox",
    "auto*",
    "auto* dog",
    "AUTO*",            // casefold
    "café",
    "straße",
    "σοφία",            // matches ΣΟΦΊΑ after casefold
    "большой",
    "東京",
    "北京市",
    "fox dog",
    "quick lazy dog",
    "data graph",
    "auto* fox quick",
    "the over jumps",
    "missing",          // absent token
    "fox missing",      // AND with absent -> empty; OR -> fox's docs
    "auto* missing",
    "",                 // empty
    " ,.;! ",           // punctuation only
    "fox fox",          // duplicate token must not change the score
];

/// The index hits as `(id -> score)`, plus the ordered id list (for the ranking
/// check). Scores are kept as `f32` for the bit-exact comparison.
fn index_answer(hits: &[sparq_text::Hit]) -> (BTreeMap<Id, f32>, Vec<Id>) {
    let map = hits.iter().map(|h| (h.id, h.score)).collect();
    let order = hits.iter().map(|h| h.id).collect();
    (map, order)
}

/// Assert the index's ranked answer matches the reference: same hit set, the
/// SAME score per hit (bit-exact via `to_bits`), and best-first ordering (ties
/// by ascending id). Returns the number of per-hit score assertions made (each
/// hit is one), so the caller can tally the ratchet count.
fn assert_matches_reference(
    label: &str,
    n: usize,
    query: &str,
    index_hits: &[sparq_text::Hit],
    reference: &BTreeMap<Id, f32>,
) -> usize {
    let (got, order) = index_answer(index_hits);

    // Same hit SET.
    let got_ids: Vec<Id> = {
        let mut v: Vec<Id> = got.keys().copied().collect();
        v.sort_unstable();
        v
    };
    let ref_ids: Vec<Id> = reference.keys().copied().collect(); // BTreeMap is sorted
    assert_eq!(
        got_ids, ref_ids,
        "{label}({query:?}) hit SET != reference at n={n}"
    );

    // Same SCORE per hit, bit-exact.
    for (&id, &want) in reference {
        let have = got[&id];
        assert_eq!(
            have.to_bits(),
            want.to_bits(),
            "{label}({query:?}) BM25 score for doc {id} = {have} != reference {want} at n={n}"
        );
    }

    // Best-first ranking: non-increasing score, ties by ascending id — exactly
    // the index's documented ordering. This re-derives the expected order from
    // the reference and asserts the index produced it.
    let mut expected_order: Vec<(Id, f32)> = reference.iter().map(|(&id, &s)| (id, s)).collect();
    expected_order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let expected_ids: Vec<Id> = expected_order.into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        order, expected_ids,
        "{label}({query:?}) ranking != reference best-first order at n={n}"
    );

    reference.len()
}

/// The differential BM25 ratchet: over several fixed-seed corpora of growing
/// size, assert `search` (AND) and `search_any` (OR) reproduce the independent
/// reference scorer's exact scores and ranking, and tally the assertion count
/// against `TEXT_ORACLE_FLOOR`. Prints a parseable scoreboard line so CI can
/// re-check the floor (`text-search differential oracle assertions N (floor F)`).
#[test]
fn bm25_differential_oracle() {
    let mut rng = StdRng::seed_from_u64(0x00B2_5BAD_u64 ^ 0x00C0_FFEE);
    let mut total_assertions = 0usize;

    for &n in &[5usize, 25, 120, 400, 900] {
        // Random documents plus one deliberate DUPLICATE literal — the dictionary
        // collapses repeats into a single document; both the index and the
        // reference scan the dictionary, so they agree on the doc count.
        let mut docs: Vec<String> = (0..n).map(|_| random_doc(&mut rng)).collect();
        if let Some(first) = docs.first().cloned() {
            docs.push(first);
        }
        let graph = graph_of(&docs);
        let idx = TextIndex::build(&graph);
        let corp = corpus(&graph);
        let scorer = RefScorer::new(&corp);

        assert_eq!(idx.len(), scorer.n, "doc-count mismatch at n={n}");

        for &q in QUERIES {
            let and_ref = scorer.answer(q, true);
            total_assertions += assert_matches_reference("AND", n, q, &idx.search(q), &and_ref);

            let or_ref = scorer.answer(q, false);
            total_assertions += assert_matches_reference("OR", n, q, &idx.search_any(q), &or_ref);
        }
    }

    // The parseable ratchet line (CI greps this; mirrors the ODRL/Solid shape).
    println!(
        "text-search differential oracle assertions {total_assertions} (floor {TEXT_ORACLE_FLOOR})"
    );
    assert!(
        total_assertions >= TEXT_ORACLE_FLOOR,
        "BM25 differential oracle made {total_assertions} score-exact assertions, \
         below the pinned floor {TEXT_ORACLE_FLOOR} — the fixed query battery shrank \
         (a coverage regression). Raise the corpora/queries, or investigate why fewer \
         hits scored. The floor may only RISE."
    );
}

/// A small HAND-PINNED corpus where the BM25 numbers are computed by hand below,
/// as a fully independent cross-check on `RefScorer` itself (so a bug shared
/// between the index and the reference scorer cannot hide a regression). A
/// shorter doc outranks a longer one at equal tf (length normalisation), and a
/// higher-tf doc outranks both. sq-ripcg
#[test]
fn bm25_hand_pinned_cross_check() {
    // Distinct literals so the dictionary keeps them separate.
    let docs = vec![
        "fox".to_string(),      // len 1, tf 1
        "fox tail".to_string(), // len 2, tf 1 (longer -> lower)
        "fox fox".to_string(),  // len 2, tf 2 (higher tf -> highest)
        "cat".to_string(),      // no "fox"
    ];
    let g = graph_of(&docs);
    let idx = TextIndex::build(&g);
    let corp = corpus(&g);
    let scorer = RefScorer::new(&corp);

    // Cross-check: index scores == reference scores, bit-exact, on this corpus.
    let hits = idx.search("fox");
    let reference = scorer.answer("fox", true);
    assert_eq!(hits.len(), reference.len(), "hand corpus hit count");
    for h in &hits {
        assert_eq!(
            h.score.to_bits(),
            reference[&h.id].to_bits(),
            "hand corpus: index score for {} diverges from the reference",
            h.id
        );
    }

    // Independent hand computation of the three "fox"-bearing docs' scores.
    // n = 4 docs, df("fox") = 3, avgdl = (1+2+2+1)/4 = 1.5.
    let n = 4.0f32;
    let df = 3.0f32;
    let avgdl = 1.5f32;
    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
    let bm25 = |tf: f32, dl: f32| idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avgdl));
    let s_a = bm25(1.0, 1.0); // "fox"      tf 1, len 1
    let s_b = bm25(1.0, 2.0); // "fox tail" tf 1, len 2
    let s_c = bm25(2.0, 2.0); // "fox fox"  tf 2, len 2

    // Map hand scores onto the actual ids by literal value, bit-exact.
    for h in &hits {
        let want = match g.dict().term(h.id) {
            Term::Literal(l) => match l.value() {
                "fox" => s_a,
                "fox tail" => s_b,
                "fox fox" => s_c,
                other => unreachable!("unexpected hit literal {}", other),
            },
            _ => unreachable!(),
        };
        assert_eq!(
            h.score.to_bits(),
            want.to_bits(),
            "hand-pinned BM25 for {:?} mismatched",
            g.dict().term(h.id)
        );
    }

    // Ranking sanity from the hand numbers: highest tf ("fox fox") first, the
    // longer single-tf doc ("fox tail") last among the three.
    assert!(s_c > s_a, "higher tf must outrank");
    assert!(s_a > s_b, "shorter doc must outrank a longer one at equal tf");
    let ranked: Vec<String> = hits
        .iter()
        .map(|h| match g.dict().term(h.id) {
            Term::Literal(l) => l.value().to_string(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ranked, vec!["fox fox", "fox", "fox tail"], "hand-pinned ranking");
}

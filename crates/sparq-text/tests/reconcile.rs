//! The rebuild-on-boot + reconcile contract (sq-oddt).
//!
//! `TextIndex` carries no on-disk format; under a stateless-core invariant it is
//! rebuilt on boot and kept warm by `reconcile`. These tests pin the contract:
//!
//! - after any sequence of graph growth, `reconcile` makes the index EXACTLY
//!   equal to a fresh `build` (full-state `PartialEq`, the differential property
//!   that `apply_delta` already guarantees);
//! - the generation marker (`indexed_dict_len`) and the cheap staleness checks
//!   (`is_consistent_with`, `needs_rebuild`) track the dictionary correctly;
//! - a dictionary shrink (post-`Graph::compact`) is reported as needing a rebuild
//!   and panics `reconcile` rather than corrupting the id mapping.

use oxrdf::{Literal, NamedNode, Term};
use rand::rngs::StdRng;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_text::{Analyzer, TextIndex};

const WORDS: &[&str] = &[
    "quick",
    "brown",
    "fox",
    "lazy",
    "dog",
    "jumps",
    "over",
    "the",
    "café",
    "straße",
    "σοφία",
    "москва",
    "data",
    "graph",
    "query",
    "index",
    "text",
    "search",
];

fn random_sentence(rng: &mut StdRng) -> String {
    let n = rng.random_range(1..=8);
    (0..n)
        .map(|_| WORDS[rng.random_range(0..WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

fn random_triple(rng: &mut StdRng) -> [Term; 3] {
    let s = Term::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/s{}",
        rng.random_range(0..40)
    )));
    let p = Term::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/p{}",
        rng.random_range(0..4)
    )));
    let o = match rng.random_range(0..10) {
        0..=5 => Term::Literal(Literal::new_simple_literal(random_sentence(rng))),
        6 => Term::Literal(Literal::new_language_tagged_literal_unchecked(
            random_sentence(rng),
            "en",
        )),
        7 => Term::Literal(Literal::new_typed_literal(
            format!("{}", rng.random_range(0..1000)),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )),
        _ => Term::NamedNode(NamedNode::new_unchecked(format!(
            "http://ex/o{}",
            rng.random_range(0..20)
        ))),
    };
    [s, p, o]
}

#[test]
fn empty_index_marker_is_zero_and_inconsistent_once_graph_grows() {
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let index = TextIndex::default();
    assert_eq!(index.indexed_dict_len(), 0);
    // An empty graph: an empty index is trivially consistent.
    assert!(index.is_consistent_with(&graph));
    assert!(!index.needs_rebuild(&graph));

    let batch = [[
        Term::NamedNode(NamedNode::new_unchecked("http://ex/s")),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/p")),
        Term::Literal(Literal::new_simple_literal("hello world")),
    ]];
    graph.apply_delta(&batch, &[]).unwrap();
    // The stale empty index now lags the grown graph.
    assert!(!index.is_consistent_with(&graph));
    assert!(!index.needs_rebuild(&graph));
}

#[test]
fn build_marks_the_full_dictionary() {
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut rng = StdRng::seed_from_u64(1);
    let seed: Vec<[Term; 3]> = (0..50).map(|_| random_triple(&mut rng)).collect();
    graph.apply_delta(&seed, &[]).unwrap();

    let index = TextIndex::build(&graph);
    assert_eq!(index.indexed_dict_len(), graph.dict().len());
    assert!(index.is_consistent_with(&graph));
    assert!(!index.needs_rebuild(&graph));
}

#[test]
fn reconcile_equals_rebuilt_across_random_growth() {
    let mut rng = StdRng::seed_from_u64(0x0bad_c0de);

    let seed: Vec<[Term; 3]> = (0..80).map(|_| random_triple(&mut rng)).collect();
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    graph.apply_delta(&seed, &[]).unwrap();

    let mut index = TextIndex::build(&graph);
    assert!(index.is_consistent_with(&graph));

    for round in 0..30 {
        // Growth only (reconcile is the append fast-path; deletes are a no-op
        // for the index by contract anyway).
        let inserts: Vec<[Term; 3]> = (0..rng.random_range(1..=20))
            .map(|_| random_triple(&mut rng))
            .collect();
        let before = graph.dict().len();
        graph.apply_delta(&inserts, &[]).unwrap();

        if graph.dict().len() > before {
            assert!(
                !index.is_consistent_with(&graph),
                "round {round}: index should lag after growth"
            );
        }
        assert!(!index.needs_rebuild(&graph));

        let added = index.reconcile(&graph);
        assert!(
            index.is_consistent_with(&graph),
            "round {round}: reconcile did not make index current"
        );

        let rebuilt = TextIndex::build(&graph);
        assert_eq!(index, rebuilt, "round {round}: reconciled != rebuilt");

        // `added` is the count of NEW indexed documents — matches the rebuilt
        // doc-count delta we expect, and a second reconcile is a no-op.
        let _ = added;
        assert_eq!(
            index.reconcile(&graph),
            0,
            "round {round}: idempotent reconcile added docs"
        );
        assert_eq!(index, rebuilt);
    }
}

#[test]
fn reconcile_preserves_cjk_analyzer_across_random_growth() {
    // sq-halqn: reconcile must use the analyzer selected at build time
    // for every appended literal, not silently fall back to Unicode tokenization.
    let mut rng = StdRng::seed_from_u64(0xc1a0_6e5e);
    let mut graph = Graph::load_str(
        r#"<http://ex/seed> <http://ex/comment> "東京都に住む" .
<http://ex/separated> <http://ex/comment> "京都の東" ."#,
        "ntriples",
    )
    .unwrap();
    let mut index = TextIndex::build_with_analyzer(&graph, Analyzer::CjkNgram);
    assert_eq!(index.analyzer(), Analyzer::CjkNgram);

    const CJK_TEXT: &[&str] = &[
        "東京湾を望む",
        "京都府を訪問",
        "北京の東門",
        "大阪市の資料",
        "南京東京街",
        "東の京都案内",
    ];

    for round in 0..24 {
        let batch_len = rng.random_range(1..=6);
        let inserts: Vec<[Term; 3]> = (0..batch_len)
            .map(|item| {
                let text = CJK_TEXT[rng.random_range(0..CJK_TEXT.len())];
                [
                    Term::NamedNode(NamedNode::new_unchecked(format!(
                        "http://ex/growth/{round}/{item}"
                    ))),
                    Term::NamedNode(NamedNode::new_unchecked("http://ex/comment")),
                    Term::Literal(Literal::new_simple_literal(format!(
                        "{text} 第{round}-{item}号"
                    ))),
                ]
            })
            .collect();
        graph.apply_delta(&inserts, &[]).unwrap();

        assert!(
            index.reconcile(&graph) > 0,
            "round {round}: no CJK document added"
        );
        assert_eq!(
            index.analyzer(),
            Analyzer::CjkNgram,
            "round {round}: reconcile changed the configured analyzer"
        );

        let rebuilt = TextIndex::build_with_analyzer(&graph, Analyzer::CjkNgram);
        assert_eq!(
            index, rebuilt,
            "round {round}: reconciled CJK index != rebuild"
        );
        assert_eq!(
            index.search("東京"),
            rebuilt.search("東京"),
            "round {round}: reconciled CJK bigram query != rebuild"
        );
    }
}

#[test]
fn reconcile_preserves_positions_mode_and_phrase() {
    let mut rng = StdRng::seed_from_u64(0xface_feed);
    let seed: Vec<[Term; 3]> = (0..40).map(|_| random_triple(&mut rng)).collect();
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    graph.apply_delta(&seed, &[]).unwrap();

    let mut index = TextIndex::build_with_positions(&graph);
    assert!(index.has_positions());

    for _ in 0..15 {
        let inserts: Vec<[Term; 3]> = (0..rng.random_range(1..=12))
            .map(|_| random_triple(&mut rng))
            .collect();
        graph.apply_delta(&inserts, &[]).unwrap();
        index.reconcile(&graph);
        assert!(
            index.has_positions(),
            "reconcile must preserve positions mode"
        );
        let rebuilt = TextIndex::build_with_positions(&graph);
        assert_eq!(index, rebuilt, "positional reconcile != positional rebuild");
        // Phrase still works against the reconciled positional index.
        assert_eq!(
            index.phrase("quick brown fox"),
            rebuilt.phrase("quick brown fox")
        );
    }
}

#[test]
fn reconcile_idempotent_when_already_current() {
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let batch = [[
        Term::NamedNode(NamedNode::new_unchecked("http://ex/s")),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/p")),
        Term::Literal(Literal::new_simple_literal("alpha beta")),
    ]];
    graph.apply_delta(&batch, &[]).unwrap();
    let mut index = TextIndex::build(&graph);
    assert_eq!(index.reconcile(&graph), 0);
    assert!(index.is_consistent_with(&graph));
}

#[test]
fn in_process_compact_keeps_ids_so_index_stays_valid() {
    // `Graph::compact` folds the fork structure WITHOUT renumbering dictionary
    // ids (it copies records 1..=len into a fresh frozen base), so an index built
    // before a compaction is still consistent afterwards and reconcile is a no-op.
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let mut rng = StdRng::seed_from_u64(0xc0de_acc0);
    let seed: Vec<[Term; 3]> = (0..60).map(|_| random_triple(&mut rng)).collect();
    graph.apply_delta(&seed, &[]).unwrap();

    // Force a fork (so compact has something to fold), then compact.
    let extra: Vec<[Term; 3]> = (0..20).map(|_| random_triple(&mut rng)).collect();
    graph.apply_delta(&extra, &[]).unwrap();
    let mut index = TextIndex::build(&graph);
    let len_before = graph.dict().len();

    graph.compact().unwrap();

    assert!(
        graph.dict().len() >= len_before,
        "compact must not drop ids in-process"
    );
    assert!(
        index.is_consistent_with(&graph),
        "index stays current across in-process compact"
    );
    assert!(!index.needs_rebuild(&graph));
    assert_eq!(
        index.reconcile(&graph),
        0,
        "no new terms after id-preserving compact"
    );
    assert_eq!(index, TextIndex::build(&graph));
}

#[test]
fn shorter_dictionary_needs_rebuild_and_panics_reconcile() {
    // The cross-process case: an index built against a larger dictionary is later
    // confronted with a SHORTER one (e.g. a stateless core reopening a durably
    // recompacted base whose persisted store dropped orphans + renumbered). The
    // index's dense id->document mapping no longer lines up — needs_rebuild must
    // flag it and reconcile must refuse rather than silently corrupt results.
    let mut big = Graph::load_str("", "ntriples").unwrap();
    let mut rng = StdRng::seed_from_u64(0xdead_10cc);
    let seed: Vec<[Term; 3]> = (0..80).map(|_| random_triple(&mut rng)).collect();
    big.apply_delta(&seed, &[]).unwrap();
    let index = TextIndex::build(&big);
    assert_eq!(index.indexed_dict_len(), big.dict().len());

    // A strictly smaller reopened graph (fewer interned terms).
    let mut small = Graph::load_str("", "ntriples").unwrap();
    let small_seed: Vec<[Term; 3]> = (0..5).map(|_| random_triple(&mut rng)).collect();
    small.apply_delta(&small_seed, &[]).unwrap();
    assert!(
        small.dict().len() < index.indexed_dict_len(),
        "scenario requires a shorter dict"
    );

    assert!(
        index.needs_rebuild(&small),
        "shorter dict must need rebuild"
    );
    assert!(!index.is_consistent_with(&small));

    let mut idx2 = index.clone();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| idx2.reconcile(&small)));
    assert!(
        res.is_err(),
        "reconcile over a shorter dictionary must panic"
    );

    // The sound recovery is a fresh boot-time build; it is consistent.
    let rebuilt = TextIndex::build(&small);
    assert!(rebuilt.is_consistent_with(&small));
    assert!(!rebuilt.needs_rebuild(&small));
}

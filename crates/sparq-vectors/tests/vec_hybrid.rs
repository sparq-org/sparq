//! [SONNET-4.6] sq-lhcot.4 — the `vec:hybrid` magic predicate end to end: hybrid retrieval
//! (dense + caller-supplied arms, fused by deterministic weighted RRF) and out-of-process
//! reranking, expressed INSIDE plain SPARQL and evaluated through the ordinary engine.
//!
//! What these tests pin, in order:
//!
//! 1. the rewrite binds `?node` / `?score` / `?rank` / `?prov`, and `?prov` reports which arm
//!    ranked each row and where;
//! 2. an arm genuinely changes the answer — a row NO dense neighbour search returns still
//!    surfaces when a sparse arm ranks it, and consensus reorders the result;
//! 3. the second stage reorders under `?rank`, and its fail-open / fail-closed policy behaves as
//!    documented (fail-open never marks a row `rerank=…`);
//! 4. the guard rails: `vec:hybrid` through the plain `query_vec` is a hard error, a text query
//!    without a query embedder is a hard error, and a `vec:` query with no hybrid pattern is
//!    unaffected by the config.
//!
//! No test here asserts that fusion or reranking IMPROVES retrieval — that is an empirical
//! question about a corpus, answered by `hybrid::ablate`, not by this crate.

#![cfg(feature = "vec-predicate")]

use oxrdf::{NamedNode, Term};
#[cfg(feature = "filtered-ann")]
use sparq_core::dict::INLINE_BASE;
use sparq_core::dict::Id;
use sparq_core::Graph;
#[cfg(feature = "filtered-ann")]
use sparq_vectors::hybrid::MAX_ARM_PAGE;
use sparq_vectors::hybrid::{
    ArmQuery, FusedHit, HybridConfig, RerankPolicy, Reranker, Rescored,
};
use sparq_vectors::{
    parse_provenance, query_vec, query_vec_hybrid, query_vec_hybrid_with_budget,
    rewrite_query_hybrid, QueryResult, VectorStore,
};

/// Four entities on the unit square. `a` is the x-axis; `c` is nearly x; `b` is the y-axis; `d`
/// is embedded but far from x. A dense query of `1,0` therefore ranks `a`, `c`, then `d`/`b`.
const TTL: &str = r#"
<http://ex/a> <http://ex/p> "a" .
<http://ex/b> <http://ex/p> "b" .
<http://ex/c> <http://ex/p> "c" .
<http://ex/d> <http://ex/p> "d" .
"#;

/// [OPUS-4.8] (review #4519) The same four entities, but only `c` carries the constraining
/// `<http://ex/ok>` triple — so a surrounding BGP pattern on that predicate genuinely NARROWS the
/// candidate set, unlike [`TTL`] where every entity satisfies `<http://ex/p>`. Only the
/// `filtered-ann` tests below use it (without that feature there is no BGP-derived mask at all).
#[cfg(feature = "filtered-ann")]
const TTL_FILTERED: &str = r#"
<http://ex/a> <http://ex/p> "a" .
<http://ex/b> <http://ex/p> "b" .
<http://ex/c> <http://ex/p> "c" .
<http://ex/d> <http://ex/p> "d" .
<http://ex/c> <http://ex/ok> "yes" .
"#;

fn fixture(tag: &str) -> (Graph, VectorStore, std::path::PathBuf) {
    fixture_from(tag, TTL)
}

fn fixture_from(tag: &str, ttl: &str) -> (Graph, VectorStore, std::path::PathBuf) {
    let g = Graph::load_str(ttl, "ntriples").unwrap();
    let path = std::env::temp_dir().join(format!(
        "sparq-vec-hybrid-{}-{}.spqv",
        tag,
        std::process::id()
    ));
    let mut store = VectorStore::create(&path, 2).unwrap();
    store.put(id(&g, "a"), &[1.0, 0.0]).unwrap();
    store.put(id(&g, "b"), &[0.0, 1.0]).unwrap();
    store.put(id(&g, "c"), &[0.95, 0.05]).unwrap();
    store.put(id(&g, "d"), &[0.6, 0.8]).unwrap();
    store.finalize().unwrap();
    (g, store, path)
}

fn iri(local: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://ex/{local}")).unwrap())
}

fn id(g: &Graph, local: &str) -> Id {
    g.id_of(&iri(local)).unwrap()
}

/// `(node local name, score, rank, provenance)` per row, ordered by the bound `?rank` so the
/// assertions read in result order (VALUES rows carry no order through joins).
fn rows(r: &QueryResult) -> Vec<(String, f64, i64, String)> {
    let mut out: Vec<(String, f64, i64, String)> = r
        .rows
        .iter()
        .map(|row| {
            let node = match row[0].as_ref().unwrap() {
                Term::NamedNode(n) => n.as_str().rsplit('/').next().unwrap().to_string(),
                other => panic!("expected an IRI, got {}", other),
            };
            let lit = |i: usize| match row[i].as_ref().unwrap() {
                Term::Literal(l) => l.value().to_string(),
                other => panic!("expected a literal, got {}", other),
            };
            (
                node,
                lit(1).parse().unwrap(),
                lit(2).parse().unwrap(),
                lit(3),
            )
        })
        .collect();
    out.sort_by_key(|r| r.2);
    out
}

const HYBRID_Q: &str = r#"
PREFIX vec: <http://sparq.dev/vec#>
SELECT ?node ?score ?rank ?prov WHERE {
  ( ?node ?score ?rank ?prov ) vec:hybrid ( "1,0" 3 )
}"#;

/// An arm that returns a fixed ranking, ignoring the query — a stand-in for a lexical/BM25 or
/// structural retriever, whose only job here is to be a SECOND opinion the fusion must honour.
fn fixed_arm(ranked: Vec<(Id, f64)>) -> sparq_vectors::ArmFn<'static> {
    Box::new(move |_q: &ArmQuery<'_>, n: usize| {
        Ok(ranked.iter().copied().take(n).collect::<Vec<_>>())
    })
}

#[test]
fn hybrid_binds_node_score_rank_and_rank_provenance() {
    let (g, store, _p) = fixture("bind");
    // A sparse arm that ranks `b` first — `b` is the WORST dense match for the query "1,0".
    let cfg = HybridConfig::new().arm("text", 1.0, fixed_arm(vec![(id(&g, "b"), 9.0)]));

    let r = query_vec_hybrid(&g, HYBRID_Q, &store, &cfg).unwrap();
    let rows = rows(&r);

    // Ranks are a gap-free 1..n in fused order.
    assert_eq!(
        rows.iter().map(|r| r.2).collect::<Vec<_>>(),
        (1..=rows.len() as i64).collect::<Vec<_>>()
    );
    // `a` is the dense rank-1 hit and is unranked by the text arm; `b` is text rank-1 only.
    let a = rows.iter().find(|r| r.0 == "a").unwrap();
    assert_eq!(a.3, "vector=1", "the dense arm alone ranked a, at rank 1");
    let b = rows.iter().find(|r| r.0 == "b").unwrap();
    assert!(
        b.3.contains("text=1"),
        "b must carry its text-arm provenance, got {}",
        b.3
    );
    // The provenance parses back into (arm, rank) pairs.
    assert_eq!(parse_provenance(&a.3).unwrap(), vec![("vector".into(), 1)]);
    // Scores are the fused RRF scores: finite, positive, and non-increasing down the ranking.
    assert!(rows.iter().all(|r| r.1 > 0.0 && r.1.is_finite()));
    for w in rows.windows(2) {
        assert!(w[0].1 >= w[1].1, "fused scores must not increase with rank");
    }
}

#[test]
fn an_arm_changes_the_answer_consensus_outranks_a_single_arm() {
    let (g, store, _p) = fixture("consensus");

    // Dense-only (an arm with no ranking) — the baseline the fusion must move away from.
    let dense_only = HybridConfig::new();
    let baseline = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &dense_only).unwrap());
    assert_eq!(
        baseline.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["a", "c", "d"],
        "the dense arm alone ranks by cosine to (1,0)"
    );
    assert!(baseline.iter().all(|r| r.3 == format!("vector={}", r.2)));

    // Now a text arm that ranks `c` first and `b` second. `c` is dense rank 2 AND text rank 1 —
    // consensus — so it must overtake `a`, which only the dense arm ranks.
    let cfg = HybridConfig::new().arm(
        "text",
        1.0,
        fixed_arm(vec![(id(&g, "c"), 9.0), (id(&g, "b"), 8.0)]),
    );
    let fused = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &cfg).unwrap());
    assert_eq!(fused[0].0, "c", "consensus beats a single arm's top hit");
    assert_eq!(fused[0].3, "vector=2;text=1");
    assert_ne!(
        fused.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        baseline.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        "the arm must actually change the answer — otherwise this test proves nothing"
    );

    // A muted dense arm (weight 0) is a pure sparse ranking: ONLY the text arm's items appear.
    let sparse_only = HybridConfig::new()
        .vector_weight(0.0)
        .arm("text", 1.0, fixed_arm(vec![(id(&g, "b"), 8.0)]));
    let only = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &sparse_only).unwrap());
    assert_eq!(only.len(), 1);
    assert_eq!((only[0].0.as_str(), only[0].3.as_str()), ("b", "text=1"));
}

/// A reranker driven by a fixed script, so both policy paths are exercised deterministically.
struct Scripted(Result<Vec<Rescored>, String>);

impl Reranker for Scripted {
    fn rerank(
        &self,
        _query: &ArmQuery<'_>,
        _candidates: &[FusedHit],
    ) -> Result<Vec<Rescored>, String> {
        self.0.clone()
    }
}

#[test]
fn the_second_stage_reorders_and_marks_its_provenance() {
    let (g, store, _p) = fixture("rerank");
    // Promote the fused runner-up to first place and drop everything else.
    let reranker = Scripted(Ok(vec![
        Rescored {
            index: 1,
            score: 42.0,
        },
        Rescored {
            index: 0,
            score: 1.0,
        },
    ]));
    let cfg = HybridConfig::new().reranker(&reranker, RerankPolicy::FailClosed);
    let out = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &cfg).unwrap());

    assert_eq!(out.len(), 2, "the reranker dropped the remaining candidates");
    assert_eq!(out[0].0, "c", "the fused runner-up was promoted to rank 1");
    assert_eq!(out[0].1, 42.0, "?score is the SECOND-STAGE score once reranked");
    assert_eq!(out[0].3, "vector=2;rerank=1");
    assert_eq!(out[1].3, "vector=1;rerank=2");
    // The rank provenance survives the round trip through SPARQL.
    assert_eq!(
        parse_provenance(&out[0].3).unwrap(),
        vec![("vector".to_string(), 2), ("rerank".to_string(), 1)]
    );
}

#[test]
fn a_failing_second_stage_is_fail_open_or_fail_closed() {
    let (g, store, _p) = fixture("policy");
    let broken = Scripted(Err("reranker service timed out".to_string()));

    // Fail open: the query still answers, with the first-stage order, and NO row claims a
    // second stage that did not run.
    let open = HybridConfig::new().reranker(&broken, RerankPolicy::FailOpen);
    let out = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &open).unwrap());
    assert_eq!(
        out.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["a", "c", "d"]
    );
    assert!(
        out.iter().all(|r| !r.3.contains("rerank")),
        "fail-open must not mark rows as reranked: {:?}",
        out
    );

    // Fail closed: a hard query error naming the underlying failure.
    let closed = HybridConfig::new().reranker(&broken, RerankPolicy::FailClosed);
    let err = query_vec_hybrid(&g, HYBRID_Q, &store, &closed).unwrap_err();
    assert!(err.contains("fail-closed"), "got: {}", err);
    assert!(err.contains("timed out"), "got: {}", err);

    // A malformed response — an index no candidate has — is treated the same way: an
    // out-of-process stage must never be able to inject a result no arm retrieved.
    let liar = Scripted(Ok(vec![Rescored {
        index: 99,
        score: 1.0,
    }]));
    let closed = HybridConfig::new().reranker(&liar, RerankPolicy::FailClosed);
    assert!(query_vec_hybrid(&g, HYBRID_Q, &store, &closed)
        .unwrap_err()
        .contains("out of range"));
}

#[test]
fn a_natural_language_query_needs_a_query_embedder() {
    let (g, store, _p) = fixture("text");
    const TEXT_Q: &str = r#"
PREFIX vec: <http://sparq.dev/vec#>
SELECT ?node ?score ?rank ?prov WHERE {
  ( ?node ?score ?rank ?prov ) vec:hybrid ( "the x axis"@en 2 )
}"#;

    // Without an embedder the dense arm has no query: a hard error, never a silent degradation.
    let err = query_vec_hybrid(&g, TEXT_Q, &store, &HybridConfig::new()).unwrap_err();
    assert!(err.contains("needs a query embedder"), "got: {}", err);

    // With one, the text form searches exactly like the equivalent vector literal, and the arms
    // see the text (here: an arm that only fires for an "@en" query).
    let cfg = HybridConfig::new()
        .query_embedder(Box::new(|text: &str, _lang: &str| {
            assert_eq!(text, "the x axis");
            Ok(vec![1.0, 0.0])
        }))
        .arm(
            "text",
            1.0,
            Box::new(move |q: &ArmQuery<'_>, _n| {
                assert_eq!(q.text.unwrap().1, "en");
                assert_eq!(q.vector, Some(&[1.0f32, 0.0][..]));
                Ok(Vec::new())
            }),
        );
    let out = rows(&query_vec_hybrid(&g, TEXT_Q, &store, &cfg).unwrap());
    assert_eq!(out.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);

    // An embedder whose dimension disagrees with the store is rejected, not silently scored.
    let wrong = HybridConfig::new()
        .query_embedder(Box::new(|_t: &str, _l: &str| Ok(vec![1.0, 0.0, 0.0])));
    let err = query_vec_hybrid(&g, TEXT_Q, &store, &wrong).unwrap_err();
    assert!(err.contains("3 dims but the store has 2"), "got: {}", err);
}

#[test]
fn hybrid_without_a_config_is_a_hard_error_and_other_predicates_are_untouched() {
    let (g, store, _p) = fixture("guard");

    // Plain `query_vec` carries no arms: answering with the dense arm alone would be a
    // well-formed ranking that is NOT the hybrid ranking the query asked for.
    let err = query_vec(&g, HYBRID_Q, &store).unwrap_err();
    assert!(err.contains("HybridConfig"), "got: {}", err);

    // `vec:nearest` / `vec:search` are unaffected by a hybrid config, and an unknown `vec:` term
    // still fails loudly.
    let cfg = HybridConfig::new().arm("text", 1.0, fixed_arm(vec![(id(&g, "b"), 1.0)]));
    let plain = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
        &store,
        &cfg,
    )
    .unwrap();
    assert_eq!(plain.rows.len(), 2, "vec:nearest keeps its own semantics");

    let err = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:teleport ( \"1,0\" 2 ) }",
        &store,
        &cfg,
    )
    .unwrap_err();
    assert!(err.contains("unknown magic predicate"), "got: {}", err);
}

#[test]
fn the_subject_list_is_prefix_optional_and_validated() {
    let (g, store, _p) = fixture("shape");
    let cfg = HybridConfig::new();

    // A bare `?node` subject is the minimal form.
    let bare = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:hybrid ( \"1,0\" 2 ) }",
        &store,
        &cfg,
    )
    .unwrap();
    assert_eq!(bare.rows.len(), 2);

    // `( ?node ?score )` binds two columns.
    let two = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score WHERE { ( ?node ?score ) vec:hybrid ( \"1,0\" 2 ) }",
        &store,
        &cfg,
    )
    .unwrap();
    assert_eq!(two.rows.len(), 2);
    assert!(two.rows.iter().all(|r| r[1].is_some()));

    // A non-variable position, a too-long list, and a bad query argument are hard errors.
    for (q, want) in [
        (
            "SELECT ?n WHERE { ( ?n <http://ex/a> ) vec:hybrid ( \"1,0\" 2 ) }",
            "must be a variable",
        ),
        (
            "SELECT ?n WHERE { ( ?n ?s ?r ?p ?x ) vec:hybrid ( \"1,0\" 2 ) }",
            "1- to 4-element list",
        ),
        (
            "SELECT ?n WHERE { ( ?n ?s ) vec:hybrid ( \"1,0\" ) }",
            "exactly two elements",
        ),
        (
            "SELECT ?n WHERE { ( ?n ?s ) vec:hybrid ( _:b 2 ) }",
            "must be a node IRI",
        ),
    ] {
        let sparql = format!("PREFIX vec: <http://sparq.dev/vec#>\n{}", q);
        let err = query_vec_hybrid(&g, &sparql, &store, &cfg).unwrap_err();
        assert!(err.contains(want), "expected {:?} in: {}", want, err);
    }
}

#[test]
fn the_budget_and_algebra_entry_points_agree_with_the_query_one() {
    let (g, store, _p) = fixture("entrypoints");
    let cfg = HybridConfig::new().arm("text", 1.0, fixed_arm(vec![(id(&g, "b"), 1.0)]));
    let want = rows(&query_vec_hybrid(&g, HYBRID_Q, &store, &cfg).unwrap());

    // Under an unlimited budget the answer is identical.
    let budgeted = query_vec_hybrid_with_budget(
        &g,
        HYBRID_Q,
        &store,
        &cfg,
        &sparq_vectors::QueryBudget::unlimited(),
    )
    .unwrap();
    assert_eq!(rows(&budgeted), want);

    // And so is the algebra-level rewrite evaluated through the engine directly: the rewrite is
    // the whole mechanism — no engine change is involved.
    let parsed = spargebra::SparqlParser::new().parse_query(HYBRID_Q).unwrap();
    let rewritten = rewrite_query_hybrid(parsed, &g, &store, &cfg).unwrap();
    let out = sparq_vectors::query_prepared(&g, &rewritten.into()).unwrap();
    assert_eq!(rows(&out), want);
}

/// The hit table JOINS — nothing more. [OPUS-4.8] (review #4519) Every entity in [`TTL`] carries
/// `<http://ex/p>`, so this constraint admits the whole store by construction and this test says
/// nothing about FILTERED hybrid retrieval; that is what the two candidate-mask tests below pin,
/// on [`TTL_FILTERED`].
#[test]
fn a_hybrid_pattern_joins_the_surrounding_bgp() {
    let (g, store, _p) = fixture("join");
    let cfg = HybridConfig::new();
    // The hit table joins to ordinary triples through the store's permutation indexes, exactly
    // as the vec:nearest/vec:search tables do.
    let r = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?o WHERE {
           ( ?node ?score ?rank ) vec:hybrid ( \"1,0\" 4 ) .
           ?node <http://ex/p> ?o .
         }",
        &store,
        &cfg,
    )
    .unwrap();
    let mut nodes: Vec<String> = r
        .rows
        .iter()
        .map(|row| match row[0].as_ref().unwrap() {
            Term::NamedNode(n) => n.as_str().rsplit('/').next().unwrap().to_string(),
            other => panic!("expected an IRI, got {}", other),
        })
        .collect();
    nodes.sort();
    assert_eq!(nodes, vec!["a", "b", "c", "d"]);
    assert!(r.rows.iter().all(|row| row[1].is_some()));
}

/// [OPUS-4.8] (review #4519) An auxiliary arm's ranking must be restricted to the SAME
/// BGP-derived candidate mask the built-in dense arm searches under.
///
/// The witness is the shape that loses an answer outright: the arm ranks all three
/// NON-matching entities above the single matching one, and `k` is 1. Fused unrestricted, the
/// top-1 is the non-matching `a` — the truncation happens before the join, so `c` is not merely
/// reordered, it is gone, and the query returns nothing. Restricted, the arm's ranking over the
/// admissible candidates is `[c]` and `c` comes back at rank 1.
///
/// The dense arm is muted (`vector_weight(0.0)`, the documented pure sparse/structural fusion) so
/// the assertion is about the auxiliary arm alone — the dense arm already honours the mask.
#[cfg(feature = "filtered-ann")]
#[test]
fn an_auxiliary_arm_honours_the_bgp_derived_candidate_mask() {
    let (g, store, _p) = fixture_from("mask-aux", TTL_FILTERED);
    let cfg = HybridConfig::new().vector_weight(0.0).arm(
        "text",
        1.0,
        fixed_arm(vec![
            (id(&g, "a"), 9.0),
            (id(&g, "b"), 8.0),
            (id(&g, "d"), 7.0),
            (id(&g, "c"), 1.0),
        ]),
    );
    let r = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score ?rank ?prov WHERE {
           ( ?node ?score ?rank ?prov ) vec:hybrid ( \"1,0\" 1 ) .
           ?node <http://ex/ok> ?o .
         }",
        &store,
        &cfg,
    )
    .unwrap();
    let rows = rows(&r);
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["c"],
        "the only BGP-admissible entity must hold the constrained top-1 rather than be evicted \
         by higher-ranked non-matching arm hits"
    );
    // Rank 1 of the SURVIVING table, and rank 1 within the arm's admissible ranking (not 4).
    assert_eq!(rows[0].2, 1);
    assert_eq!(rows[0].3, "text=1");
}

/// [OPUS-4.8] (review #4519) The same mask also fixes the RRF ranks of the hits that DO qualify:
/// unrestricted, the three non-matching entities push `c` down to the arm's rank 4, which changes
/// its fused score and its reported `?prov`. Restricted, `c` is the arm's rank 1.
#[cfg(feature = "filtered-ann")]
#[test]
fn the_candidate_mask_corrects_the_arm_ranks_of_qualifying_hits() {
    let (g, store, _p) = fixture_from("mask-rank", TTL_FILTERED);
    let cfg = HybridConfig::new().arm(
        "text",
        1.0,
        fixed_arm(vec![
            (id(&g, "a"), 9.0),
            (id(&g, "b"), 8.0),
            (id(&g, "d"), 7.0),
            (id(&g, "c"), 1.0),
        ]),
    );
    let r = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score ?rank ?prov WHERE {
           ( ?node ?score ?rank ?prov ) vec:hybrid ( \"1,0\" 2 ) .
           ?node <http://ex/ok> ?o .
         }",
        &store,
        &cfg,
    )
    .unwrap();
    let rows = rows(&r);
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["c"]);
    assert_eq!(
        rows[0].3, "vector=1;text=1",
        "the text arm's rank for c is its position among the ADMISSIBLE candidates"
    );
}

/// [OPUS-4.8] (review #4519, round 2) A graph deep enough to push the one admissible entity BELOW
/// the first page an arm is asked for: eight non-matching `n0…n7`, then `m` — the only entity
/// carrying the constraining `<http://ex/ok>` triple.
#[cfg(feature = "filtered-ann")]
fn deep_fixture(tag: &str) -> (Graph, VectorStore, std::path::PathBuf) {
    let mut ttl = String::new();
    for i in 0..8 {
        ttl.push_str(&format!("<http://ex/n{}> <http://ex/p> \"n{}\" .\n", i, i));
    }
    ttl.push_str("<http://ex/m> <http://ex/p> \"m\" .\n<http://ex/m> <http://ex/ok> \"yes\" .\n");
    let g = Graph::load_str(&ttl, "ntriples").unwrap();
    let path = std::env::temp_dir().join(format!(
        "sparq-vec-hybrid-{}-{}.spqv",
        tag,
        std::process::id()
    ));
    let mut store = VectorStore::create(&path, 2).unwrap();
    // Terms are interned in parse order, so writing the vectors in that order keeps the store's
    // ids monotonic. The dense arm is muted in the test below; these exist only so the store is
    // well formed.
    for i in 0..8 {
        store.put(id(&g, &format!("n{}", i)), &[1.0, 0.0]).unwrap();
    }
    store.put(id(&g, "m"), &[0.0, 1.0]).unwrap();
    store.finalize().unwrap();
    (g, store, path)
}

/// [OPUS-4.8] (review #4519, round 2) An arm's restricted ranking must be its ranking over the
/// ADMISSIBLE domain — not over whichever page it happened to return.
///
/// This is the boundary the two tests above do not reach: they fit every arm hit inside the
/// requested `cfg.candidates(k)` prefix, so masking only had to compact it. Here the arm ranks
/// EIGHT non-matching entities above the sole matching `m` while `k = 1` asks for just
/// `DEFAULT_OVER_FETCH` (4) candidates. Masking that one page alone leaves the arm empty and the
/// query answers nothing — the very loss the mask exists to prevent, moved one step later. Paging
/// the arm deeper reaches `m`, and it comes back at the arm's compacted rank 1.
#[cfg(feature = "filtered-ann")]
#[test]
fn an_admissible_arm_hit_below_the_first_page_is_still_found() {
    let (g, store, _p) = deep_fixture("mask-page");
    let mut ranked: Vec<(Id, f64)> = (0..8)
        .map(|i| (id(&g, &format!("n{}", i)), 9.0 - f64::from(i)))
        .collect();
    ranked.push((id(&g, "m"), 0.5));
    let cfg = HybridConfig::new()
        .vector_weight(0.0)
        .arm("text", 1.0, fixed_arm(ranked));
    let r = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score ?rank ?prov WHERE {
           ( ?node ?score ?rank ?prov ) vec:hybrid ( \"1,0\" 1 ) .
           ?node <http://ex/ok> ?o .
         }",
        &store,
        &cfg,
    )
    .unwrap();
    let rows = rows(&r);
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["m"],
        "the admissible hit sits below the first page the arm was asked for, so it is found only \
         if the arm is paged deeper rather than masked-then-dropped"
    );
    assert_eq!(rows[0].2, 1);
    assert_eq!(rows[0].3, "text=1");
}

/// [OPUS-4.8] (review #4519, round 5) An admissible id is not always a STORED-TERM id: a `?node`
/// constrained in an OBJECT position to a small canonical `xsd:integer` is bound to an INLINE
/// literal id, far outside `1..=dict.len()` — and `check_arm_ids` admits exactly that. So the
/// paging backstop may not be `dict.len()`: an arm is free to rank `dict.len()` inadmissible ids
/// above the admissible inline one, and stopping at the dictionary length loses it.
///
/// The witness: `?node` is pinned by `<s> <val> ?node` to `"7"^^xsd:integer`, and the arm ranks
/// every stored-term id (all inadmissible) before that inline id — putting it one past the OLD
/// `candidates.max(dict.len())` request ceiling. Under that ceiling the arm's masked ranking comes
/// back EMPTY and the query answers nothing; bounded by the real id domain the arm is paged one
/// round deeper and the sole admissible answer is found.
#[cfg(feature = "filtered-ann")]
#[test]
fn an_admissible_inline_integer_id_beyond_the_dictionary_length_is_still_found() {
    // Enough stored terms that the dictionary length exceeds the first page the arm is asked for
    // (`cfg.candidates(1)` = DEFAULT_OVER_FETCH = 4), so the old ceiling was `dict.len()`.
    let mut ttl = String::new();
    for i in 0..6 {
        ttl.push_str(&format!("<http://ex/e{}> <http://ex/p> \"e{}\" .\n", i, i));
    }
    ttl.push_str(
        "<http://ex/s> <http://ex/val> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    );
    let g = Graph::load_str(&ttl, "ntriples").unwrap();
    let path = std::env::temp_dir().join(format!(
        "sparq-vec-hybrid-inline-{}.spqv",
        std::process::id()
    ));
    let mut store = VectorStore::create(&path, 2).unwrap();
    store.finalize().unwrap();

    let inline_id = g
        .id_of(&Term::Literal(oxrdf::Literal::new_typed_literal(
            "7",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )))
        .unwrap();
    let len = g.dict.len() as Id;
    assert!(
        inline_id > len,
        "the constraining literal must inline (id {} outside 1..={})",
        inline_id,
        len
    );
    // Every stored-term id first — all inadmissible under the mask `{inline_id}` — then the one
    // admissible id, at position `dict.len() + 1`.
    let mut ranked: Vec<(Id, f64)> = (1..=len).map(|i| (i, f64::from(len - i) + 1.0)).collect();
    ranked.push((inline_id, 0.5));

    let cfg = HybridConfig::new()
        .vector_weight(0.0)
        .arm("text", 1.0, fixed_arm(ranked));
    let r = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score ?rank ?prov WHERE {
           ( ?node ?score ?rank ?prov ) vec:hybrid ( \"1,0\" 1 ) .
           <http://ex/s> <http://ex/val> ?node .
         }",
        &store,
        &cfg,
    )
    .unwrap();
    let nodes: Vec<String> = r
        .rows
        .iter()
        .map(|row| match row[0].as_ref().unwrap() {
            Term::Literal(l) => l.value().to_string(),
            other => panic!("expected the constrained integer literal, got {}", other),
        })
        .collect();
    assert_eq!(
        nodes,
        vec!["7".to_string()],
        "the admissible id is an INLINE literal id past dict.len(), so it is found only if the \
         paging backstop bounds the whole valid id domain rather than the dictionary alone"
    );
}

/// [SONNET-4.6] (review #4519, round 6) The paging loop's escalation is bounded by a documented
/// finite cap, and hitting it is a hard arm-named error rather than a deeper request.
///
/// The correctness bound on a prefix-consistent arm's ranking is the whole valid id domain —
/// `dict.len()` plus the ~1.07e9 inline-integer ids — but that is not a safe paging protocol: every
/// request makes the arm MATERIALIZE a `Vec` of that size, so doubling towards the domain would let
/// this tiny query ask for over a billion results.
///
/// The witness: an arm that PADS (always returns exactly the `n` asked for, here as distinct inline
/// ids) and whose results are all inadmissible under the mask `{m}` — so no round ever satisfies
/// `candidates`, `mask.len()` or exhaustion, and only the cap can stop it. Every requested page
/// must stay at or under [`MAX_ARM_PAGE`], and the query must fail closed naming the arm rather
/// than answer from a ranking whose admissible hits were never reached.
#[cfg(feature = "filtered-ann")]
#[test]
fn a_non_exhausting_arm_is_paged_no_deeper_than_the_documented_cap() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (g, store, _p) = deep_fixture("page-cap");
    let deepest = AtomicUsize::new(0);
    let cfg = HybridConfig::new().vector_weight(0.0).arm(
        "text",
        1.0,
        Box::new(|_q: &ArmQuery<'_>, n: usize| {
            deepest.fetch_max(n, Ordering::Relaxed);
            // Exactly `n` distinct ids `check_arm_ids` admits, none of them `m`: an arm that never
            // signals exhaustion and never offers an admissible hit.
            Ok((0..n as Id).map(|i| (INLINE_BASE + i, 1.0)).collect())
        }),
    );
    let err = query_vec_hybrid(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score ?rank ?prov WHERE {
           ( ?node ?score ?rank ?prov ) vec:hybrid ( \"1,0\" 1 ) .
           ?node <http://ex/ok> ?o .
         }",
        &store,
        &cfg,
    )
    .unwrap_err();
    assert!(
        err.contains("\"text\"") && err.contains("cap"),
        "the exhausted paging budget must be an arm-named hard error, got {}",
        err
    );
    let deepest = deepest.load(Ordering::Relaxed);
    assert!(
        deepest <= MAX_ARM_PAGE,
        "no request may exceed the documented {}-result cap, but one asked for {}",
        MAX_ARM_PAGE,
        deepest
    );
    assert_eq!(
        deepest, MAX_ARM_PAGE,
        "the loop must page up to the cap before giving up — a smaller ceiling would abandon \
         admissible hits sooner than the budget requires"
    );
}

/// [OPUS-4.8] (review #4519) An arm is a caller closure, so its ids are untrusted input. An id
/// outside the graph dictionary's domain — `0` (never a valid 1-based id) or one past the end —
/// must be a HARD query error naming the arm, not a hit that resolves to the dictionary's
/// out-of-range placeholder term and is then silently dropped from the inlined `VALUES` table.
#[test]
fn an_arm_id_outside_the_graph_dictionary_is_a_hard_query_error() {
    let (g, store, _p) = fixture("bad-id");
    for bad in [0u32, 1_000_000] {
        let cfg = HybridConfig::new().arm("text", 1.0, fixed_arm(vec![(bad, 9.0)]));
        let err = query_vec_hybrid(&g, HYBRID_Q, &store, &cfg).unwrap_err();
        assert!(
            err.contains("\"text\"") && err.contains("dictionary"),
            "expected an arm-named dictionary-domain error for id {}, got {}",
            bad,
            err
        );
    }
}

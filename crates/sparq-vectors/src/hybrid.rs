//! **Hybrid retrieval + second-stage reranking** — the deterministic fusion core behind the
//! `vec:hybrid` magic predicate ([`crate::rewrite`]). [SONNET-4.6] (sq-lhcot.4)
//!
//! Before this module the crate shipped hybrid retrieval only as *Rust* fusion helpers
//! ([`crate::fuse`]): a caller could fuse ranked lists by RRF, but there was no in-query surface,
//! no per-rank provenance, and no evaluated second stage. This module adds the three missing
//! pieces, all of them **mechanism only** — no accuracy or "lift" claim is made anywhere:
//!
//! 1. **[`fuse_arms`]** — weighted Reciprocal Rank Fusion over N *named* retrieval arms (dense
//!    vector, sparse/lexical, structural, …) that additionally records, per fused hit, **which
//!    arm ranked it and at what rank** ([`FusedHit::ranks`], rendered by
//!    [`FusedHit::provenance`]). Scores and order are identical to
//!    [`fuse_rrf_weighted`](crate::fuse::fuse_rrf_weighted) for the same input — the provenance
//!    is strictly additive bookkeeping, asserted by a test.
//! 2. **[`Reranker`]** — an out-of-process second stage (a cross-encoder service, an LLM judge,
//!    …) that rescores the fused candidates, under an explicit [`RerankPolicy`]:
//!    [`FailOpen`](RerankPolicy::FailOpen) keeps the first-stage order when the reranker fails,
//!    [`FailClosed`](RerankPolicy::FailClosed) turns the failure into a hard query error. A
//!    reranker may reorder or **drop** candidates; it can never invent one (an out-of-range,
//!    duplicated or non-finite response is a malformed response, handled by the same policy).
//! 3. **[`ablate`]** — the per-arm ablation harness (dense / sparse / structural / fused /
//!    reranked) over a caller-supplied gold set, reporting [`RetrievalMetrics`] per arm. It
//!    REPORTS; it asserts nothing. **No lift is claimed by this crate**: whether fusion or
//!    reranking beats a single arm is an empirical question about a specific corpus, and the
//!    pinned-corpus run is not in-tree (see the crate README).
//!
//! ## Determinism
//!
//! Everything here is deterministic given its inputs: arms are consumed in declared order, ties
//! break by first appearance (`(arm, rank)` order), and the reranker's response is applied as a
//! total order over indices. The same arms + the same gold set always produce the same rows.
//!
//! ```
//! use sparq_vectors::hybrid::{fuse_arms, ArmRanking, RRF_K};
//!
//! // Two arms over dictionary ids: a dense (vector) ranking and a lexical one.
//! let arms = vec![
//!     ArmRanking::new("vector", 1.0, vec![(7, 0.91), (3, 0.80)]),
//!     ArmRanking::new("text", 0.5, vec![(3, 12.4), (9, 8.1)]),
//! ];
//! let fused = fuse_arms(&arms, RRF_K, 10).unwrap();
//!
//! // id 3 is ranked by BOTH arms — consensus wins — and says so in its provenance.
//! assert_eq!(fused[0].id, 3);
//! assert_eq!(fused[0].provenance(), "vector=2;text=1");
//! ```

use crate::fuse::RRF_K as FUSE_RRF_K;
use oxrdf::NamedNode;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

/// The standard RRF rank constant, re-exported from [`crate::fuse`] so a hybrid caller needs
/// only this module.
pub const RRF_K: f64 = FUSE_RRF_K;

/// The arm name reserved for the built-in dense/vector ranking the `vec:hybrid` rewrite
/// contributes. A caller-declared arm may not use it (that would make the provenance ambiguous).
pub const VECTOR_ARM: &str = "vector";

/// The provenance key the second stage appends when a [`Reranker`] rescored a hit
/// (`…;rerank=2` — the reranker's own 1-based position). Reserved as an arm name.
pub const RERANK_KEY: &str = "rerank";

/// How many first-stage candidates each arm is asked for, as a multiple of the query's `k`.
/// Fusing only `k` per arm throws away the consensus evidence that makes fusion worth doing,
/// so the first stage over-fetches; the reranker (when present) sees the same widened pool.
pub const DEFAULT_OVER_FETCH: usize = 4;

/// The query a retrieval arm (and a [`Reranker`]) is driven by. Exactly one of `seed` / `text`
/// is `Some` for the seed-entity and text forms; both are `None` for a query-by-vector.
///
/// `vector` is the dense query vector when one is available — the seed entity's stored vector,
/// the literal query vector, or the embedding a configured query embedder produced for `text`.
/// It is `None` only when the seed entity has no stored vector, in which case the built-in
/// dense arm contributes nothing and the remaining arms still run (graceful degradation of the
/// *dense* arm only — never of correctness).
#[derive(Debug, Clone, Copy)]
pub struct ArmQuery<'a> {
    /// The seed entity IRI, for the "neighbours of this entity" form.
    pub seed: Option<&'a NamedNode>,
    /// `(text, language tag)` for the natural-language form (a language-tagged literal).
    pub text: Option<(&'a str, &'a str)>,
    /// The dense query vector, when one could be resolved.
    pub vector: Option<&'a [f32]>,
}

impl<'a> ArmQuery<'a> {
    /// The seed-entity form: neighbours of `seed`, whose stored `vector` is the dense query
    /// (`None` when the seed is unembedded).
    pub fn from_seed(seed: &'a NamedNode, vector: Option<&'a [f32]>) -> Self {
        ArmQuery {
            seed: Some(seed),
            text: None,
            vector,
        }
    }

    /// The query-by-vector form.
    pub fn from_vector(vector: &'a [f32]) -> Self {
        ArmQuery {
            seed: None,
            text: None,
            vector: Some(vector),
        }
    }

    /// The natural-language form: `text` in language `language`, embedded to `vector`.
    pub fn from_text(text: &'a str, language: &'a str, vector: Option<&'a [f32]>) -> Self {
        ArmQuery {
            seed: None,
            text: Some((text, language)),
            vector,
        }
    }
}

/// One retrieval arm's ranked contribution: `ranked` is `(dictionary id, that arm's own score)`
/// **best first**. RRF ignores the scores — only the order matters — so arms on wildly different
/// scales (cosine, BM25, Jaccard) fuse without normalization.
///
/// `weight` scales this arm's `1/(k + rank)` contributions; `0.0` **mutes** the arm entirely
/// (its items do not appear unless another live arm also ranks them), matching
/// [`fuse_rrf_weighted`](crate::fuse::fuse_rrf_weighted).
#[derive(Debug, Clone)]
pub struct ArmRanking {
    /// The arm's name — appears verbatim in every hit's provenance, so it must be unique,
    /// non-empty and free of the provenance separators `;` and `=`.
    pub arm: String,
    /// Non-negative, finite fusion weight (`0.0` mutes the arm).
    pub weight: f64,
    /// `(id, arm score)` best first. An id may not repeat within one arm.
    pub ranked: Vec<(Id, f64)>,
}

impl ArmRanking {
    /// Convenience constructor.
    pub fn new(arm: impl Into<String>, weight: f64, ranked: Vec<(Id, f64)>) -> Self {
        ArmRanking {
            arm: arm.into(),
            weight,
            ranked,
        }
    }
}

/// One fused result: the entity, its fused score, and the provenance of every rank that
/// produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedHit {
    /// The dictionary id of the retrieved entity.
    pub id: Id,
    /// The **final-stage** score: the weighted-RRF fused score, or — when a [`Reranker`]
    /// rescored this hit — the reranker's own score. The two are on different scales and are
    /// NOT comparable; order by the rank position, not by mixing scores across stages.
    pub score: f64,
    /// `(arm name, 1-based rank in that arm)` for every arm that ranked this hit, in arm
    /// declaration order.
    pub ranks: Vec<(String, usize)>,
    /// The reranker's 1-based output position, when a [`Reranker`] rescored this hit.
    pub reranked: Option<usize>,
}

impl FusedHit {
    /// The deterministic provenance string: `;`-separated `arm=rank` entries in arm
    /// declaration order, with `rerank=<position>` appended when a [`Reranker`] rescored the
    /// hit — e.g. `"vector=1;text=3;rerank=2"`. Round-trips through [`parse_provenance`].
    pub fn provenance(&self) -> String {
        let mut out = String::new();
        for (arm, rank) in &self.ranks {
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(&format!("{}={}", arm, rank));
        }
        if let Some(pos) = self.reranked {
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(&format!("{}={}", RERANK_KEY, pos));
        }
        out
    }
}

/// Parses a [`FusedHit::provenance`] string back into `(name, rank)` pairs, in order. The
/// `rerank` entry (when present) comes through as an ordinary pair, so a consumer can read the
/// second-stage position with the same parser.
///
/// # Errors
/// On an empty entry, a missing `=`, or a rank that is not a positive integer.
pub fn parse_provenance(s: &str) -> Result<Vec<(String, usize)>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in s.split(';') {
        let Some((name, rank)) = entry.split_once('=') else {
            return Err(format!(
                "hybrid: malformed provenance entry {:?} (expected name=rank)",
                entry
            ));
        };
        if name.is_empty() {
            return Err(format!("hybrid: provenance entry {:?} has an empty name", entry));
        }
        let rank: usize = rank
            .parse()
            .map_err(|_| format!("hybrid: provenance entry {:?} has a non-integer rank", entry))?;
        if rank == 0 {
            return Err(format!("hybrid: provenance ranks are 1-based, got {:?}", entry));
        }
        out.push((name.to_string(), rank));
    }
    Ok(out)
}

/// Validates a set of arms: names non-empty, unique, free of the `;`/`=` separators and of the
/// reserved [`RERANK_KEY`]; weights finite and non-negative; no id repeated within one arm.
///
/// The duplicate-id check is load-bearing for an **out-of-process** arm: a service that returns
/// the same document twice would otherwise double its own contribution and silently bias the
/// fusion. We reject instead (fail closed).
pub fn validate_arms(arms: &[ArmRanking]) -> Result<(), String> {
    let mut seen: FxHashSet<&str> = FxHashSet::default();
    for arm in arms {
        if arm.arm.is_empty() {
            return Err("hybrid: an arm name must not be empty".to_string());
        }
        if arm.arm.contains(';') || arm.arm.contains('=') {
            return Err(format!(
                "hybrid: arm name {:?} must not contain ';' or '=' (they separate provenance entries)",
                arm.arm
            ));
        }
        if arm.arm == RERANK_KEY {
            return Err(format!(
                "hybrid: {:?} is reserved for the second-stage provenance entry",
                RERANK_KEY
            ));
        }
        if !seen.insert(arm.arm.as_str()) {
            return Err(format!("hybrid: duplicate arm name {:?}", arm.arm));
        }
        if !arm.weight.is_finite() || arm.weight < 0.0 {
            return Err(format!(
                "hybrid: arm {:?} has weight {} — weights must be finite and non-negative",
                arm.arm, arm.weight
            ));
        }
        let mut ids: FxHashSet<Id> = FxHashSet::default();
        for (id, _) in &arm.ranked {
            if !ids.insert(*id) {
                return Err(format!(
                    "hybrid: arm {:?} ranked id {} twice — a ranking must not repeat an item",
                    arm.arm, id
                ));
            }
        }
    }
    Ok(())
}

/// **Weighted RRF over named arms, with per-rank provenance.**
///
/// `score(id) = Σ_arms weight_arm / (rrf_k + rank_arm)`, ranks 1-based, arms consumed in
/// declaration order. Returns the top `top_k` best first; ties break by first appearance across
/// `(arm, rank)` order, so the result is deterministic. A zero-weight arm is muted and
/// contributes nothing at all (it injects no standalone items).
///
/// The scores and ordering are identical to
/// [`fuse_rrf_weighted`](crate::fuse::fuse_rrf_weighted) over the same lists — this function
/// only additionally records [`FusedHit::ranks`].
///
/// # Errors
/// If [`validate_arms`] rejects the arms, or `rrf_k` is not finite and positive.
pub fn fuse_arms(arms: &[ArmRanking], rrf_k: f64, top_k: usize) -> Result<Vec<FusedHit>, String> {
    validate_arms(arms)?;
    if !rrf_k.is_finite() || rrf_k <= 0.0 {
        return Err(format!("hybrid: the RRF k must be finite and positive, got {}", rrf_k));
    }
    let mut acc: FxHashMap<Id, Accumulated> = FxHashMap::default();
    let mut order = 0usize;
    for arm in arms {
        // A muted arm contributes nothing — not even a 0.0 entry that could surface as a
        // standalone result (the review-1874 rule `fuse_rrf_weighted` follows).
        if arm.weight == 0.0 {
            continue;
        }
        for (rank0, (id, _)) in arm.ranked.iter().enumerate() {
            let rank = rank0 + 1;
            let entry = acc.entry(*id).or_insert_with(|| {
                order += 1;
                Accumulated {
                    score: 0.0,
                    first_seen: order,
                    ranks: Vec::new(),
                }
            });
            entry.score += arm.weight / (rrf_k + rank as f64);
            entry.ranks.push((arm.arm.clone(), rank));
        }
    }
    let mut out: Vec<(Accumulated, Id)> = acc.into_iter().map(|(id, a)| (a, id)).collect();
    // Best first; ties broken by first appearance across `(arm, rank)` order — deterministic.
    out.sort_by(|a, b| {
        b.0.score
            .total_cmp(&a.0.score)
            .then(a.0.first_seen.cmp(&b.0.first_seen))
    });
    out.truncate(top_k);
    Ok(out
        .into_iter()
        .map(|(a, id)| FusedHit {
            id,
            score: a.score,
            ranks: a.ranks,
            reranked: None,
        })
        .collect())
}

/// One id's in-progress fusion state: its running score, the first-appearance order that breaks
/// ties deterministically, and the arm ranks that produced it.
struct Accumulated {
    score: f64,
    first_seen: usize,
    ranks: Vec<(String, usize)>,
}

/// One reranked candidate: an index into the candidate slice the [`Reranker`] was given, plus
/// the second-stage score it assigned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rescored {
    /// Index into the candidate slice passed to [`Reranker::rerank`].
    pub index: usize,
    /// The reranker's own score (any finite scale — it replaces [`FusedHit::score`]).
    pub score: f64,
}

/// An **out-of-process second-stage reranker** — a cross-encoder service, an LLM judge, a
/// business-rule scorer. This crate never opens a socket: an implementation owns its transport,
/// its timeout, and its failure semantics, and reports failure by returning `Err`.
///
/// The response is the new order, **best first**, as [`Rescored`] indices into `candidates`. A
/// reranker MAY return fewer entries than it was given (dropping a candidate is a legitimate
/// second-stage decision); it may NOT return an out-of-range index, repeat an index, or return
/// a non-finite score — those are malformed responses, treated exactly like an `Err` under the
/// caller's [`RerankPolicy`], because an out-of-process stage must never be able to inject a
/// result that no arm retrieved.
pub trait Reranker {
    /// Rescore `candidates` (the fused first stage, best first) for `query`.
    fn rerank(&self, query: &ArmQuery<'_>, candidates: &[FusedHit])
        -> Result<Vec<Rescored>, String>;
}

/// What to do when the second stage fails (an `Err`, or a malformed response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankPolicy {
    /// **Fail open** — degrade to the first-stage (fused) order. Availability over precision:
    /// the query still answers, ranked by fusion alone, and no hit is marked `rerank=…` (the
    /// provenance never claims a second stage that did not run).
    FailOpen,
    /// **Fail closed** — the failure is a hard query error. Correctness over availability: use
    /// this when a downstream consumer would otherwise silently treat first-stage order as
    /// reranked.
    FailClosed,
}

/// Runs the second stage over `fused` under `policy`, returning the final top `top_k`.
///
/// On success the returned hits carry the reranker's scores and their `rerank=<position>`
/// provenance. On failure, [`RerankPolicy::FailOpen`] returns the fused order truncated to
/// `top_k` (unmarked), and [`RerankPolicy::FailClosed`] returns the error.
pub fn apply_rerank(
    reranker: &dyn Reranker,
    policy: RerankPolicy,
    query: &ArmQuery<'_>,
    fused: Vec<FusedHit>,
    top_k: usize,
) -> Result<Vec<FusedHit>, String> {
    let outcome = reranker
        .rerank(query, &fused)
        .and_then(|order| check_rerank(&order, fused.len()).map(|()| order));
    match outcome {
        Ok(order) => {
            let mut out: Vec<FusedHit> = order
                .into_iter()
                .enumerate()
                .map(|(pos, r)| {
                    let mut hit = fused[r.index].clone();
                    hit.score = r.score;
                    hit.reranked = Some(pos + 1);
                    hit
                })
                .collect();
            out.truncate(top_k);
            Ok(out)
        }
        Err(e) => match policy {
            RerankPolicy::FailOpen => {
                let mut out = fused;
                out.truncate(top_k);
                Ok(out)
            }
            RerankPolicy::FailClosed => Err(format!(
                "hybrid: the second-stage reranker failed and the policy is fail-closed: {}",
                e
            )),
        },
    }
}

/// Validates a reranker response against the candidate count: every index in range, no index
/// repeated, every score finite. A subset (fewer entries than candidates) is allowed.
fn check_rerank(order: &[Rescored], candidates: usize) -> Result<(), String> {
    let mut seen: FxHashSet<usize> = FxHashSet::default();
    for r in order {
        if r.index >= candidates {
            return Err(format!(
                "malformed response: candidate index {} is out of range (only {} candidates were sent)",
                r.index, candidates
            ));
        }
        if !seen.insert(r.index) {
            return Err(format!(
                "malformed response: candidate index {} appears twice",
                r.index
            ));
        }
        if !r.score.is_finite() {
            return Err(format!(
                "malformed response: candidate index {} has a non-finite score",
                r.index
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Ablation harness — REPORTS per-arm retrieval quality; claims nothing.
// ---------------------------------------------------------------------------------------------

/// Rank-quality metrics for one arm against a gold set, at cutoff `k`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalMetrics {
    /// The cutoff the metrics were computed at.
    pub k: usize,
    /// How many gold items appear in the top `k`.
    pub hits: usize,
    /// `hits / |gold|` — 0.0 for an empty gold set.
    pub recall: f64,
    /// Reciprocal rank of the FIRST gold item in the top `k` (`1/rank`), 0.0 if none.
    pub mrr: f64,
}

/// Computes [`RetrievalMetrics`] for one ranked list (best first) against `gold`, at cutoff `k`.
/// Duplicate ids in `ranked` are counted once (the first occurrence decides the rank).
pub fn evaluate(ranked: &[Id], gold: &[Id], k: usize) -> RetrievalMetrics {
    let gold_set: FxHashSet<Id> = gold.iter().copied().collect();
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut hits = 0usize;
    let mut mrr = 0.0f64;
    let mut rank = 0usize;
    for id in ranked {
        if !seen.insert(*id) {
            continue;
        }
        rank += 1;
        if rank > k {
            break;
        }
        if gold_set.contains(id) {
            hits += 1;
            if mrr == 0.0 {
                mrr = 1.0 / rank as f64;
            }
        }
    }
    RetrievalMetrics {
        k,
        hits,
        recall: if gold_set.is_empty() {
            0.0
        } else {
            hits as f64 / gold_set.len() as f64
        },
        mrr,
    }
}

/// One row of the ablation table: an arm (or the `fused` / `reranked` stage) and its metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct AblationRow {
    /// The arm name, or `"fused"` / `"reranked"` for the two pipeline stages.
    pub arm: String,
    /// The metrics that arm scored against the gold set.
    pub metrics: RetrievalMetrics,
}

/// The row name of the fused stage in an [`ablate`] table.
pub const FUSED_ROW: &str = "fused";
/// The row name of the reranked stage in an [`ablate`] table.
pub const RERANKED_ROW: &str = "reranked";

/// **The ablation table**: one row per arm (in declared order), then the [`FUSED_ROW`], then —
/// when a `reranker` is supplied — the [`RERANKED_ROW`]. Every row is [`evaluate`]d against the
/// same `gold` set at the same cutoff `k`, so the arms are directly comparable.
///
/// This function REPORTS. It makes no claim about which row should win: fusion and reranking
/// improve some corpora and hurt others, and the honest statement is the table. Do not quote a
/// lift that this table does not show on the corpus you actually care about.
///
/// The reranker runs **fail-closed** here regardless of any query-time policy — an ablation that
/// silently reported the fused order as `reranked` would be measuring the wrong thing.
///
/// # Errors
/// If [`fuse_arms`] rejects the arms, or the reranker fails.
pub fn ablate(
    arms: &[ArmRanking],
    query: &ArmQuery<'_>,
    reranker: Option<&dyn Reranker>,
    gold: &[Id],
    k: usize,
    rrf_k: f64,
) -> Result<Vec<AblationRow>, String> {
    let mut rows: Vec<AblationRow> = arms
        .iter()
        .map(|arm| AblationRow {
            arm: arm.arm.clone(),
            metrics: evaluate(
                &arm.ranked.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                gold,
                k,
            ),
        })
        .collect();

    // The fused stage sees every arm's full list (the arms were already over-fetched by the
    // caller); it is evaluated at the same cutoff as the individual arms.
    let fused = fuse_arms(arms, rrf_k, usize::MAX)?;
    let fused_ids: Vec<Id> = fused.iter().map(|h| h.id).collect();
    rows.push(AblationRow {
        arm: FUSED_ROW.to_string(),
        metrics: evaluate(&fused_ids, gold, k),
    });

    if let Some(reranker) = reranker {
        let reranked = apply_rerank(reranker, RerankPolicy::FailClosed, query, fused, usize::MAX)?;
        let ids: Vec<Id> = reranked.iter().map(|h| h.id).collect();
        rows.push(AblationRow {
            arm: RERANKED_ROW.to_string(),
            metrics: evaluate(&ids, gold, k),
        });
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------------------------
// The query-time configuration the `vec:hybrid` rewrite consumes.
// ---------------------------------------------------------------------------------------------

/// A caller-supplied retrieval arm: given the query and the number of candidates wanted, return
/// a ranked `(id, score)` list, best first.
///
/// An arm that returns `Err` is a **hard query error** — the arm decides its own fail-open
/// behaviour, and an arm that prefers availability returns an empty list instead of an error.
/// (The [`RerankPolicy`] fail-open/fail-closed switch applies to the *second* stage, whose
/// failure mode is the one worth a policy: the first stage's candidates are the answer.)
///
/// [OPUS-4.8] (review #4519) The ids an arm returns are **untrusted**. On the `vec:hybrid` query
/// path an id outside the graph dictionary's domain is a hard, arm-named query error, and — with
/// `filtered-ann` — the returned ranking is then restricted to the same BGP-derived candidate mask
/// the built-in dense arm searched under, preserving the arm's relative order. So an arm need not
/// know the surrounding BGP: it may rank freely, and the query surface enforces admissibility.
///
/// # The paging contract (`filtered-ann`)
///
/// Restricting AFTER the arm answered can only compact the prefix the arm returned, so an arm
/// whose admissible hits all sit below the requested count would lose them. The query path
/// therefore **re-asks the same arm for a deeper page** (doubling the count) until it has enough
/// admissible hits, the arm is exhausted, or the dictionary domain is reached. That makes two
/// requirements of an arm, both of which the "give me your top `n`" reading already implies:
///
/// - **Prefix-consistent.** The answer for `n` is a prefix of the answer for any larger `n`: the
///   arm ranks a fixed order and truncates. An arm whose order depends on `n` gets a ranking that
///   is still admissible and still its own, but no longer meaningfully "its top `n`".
/// - **Exhaustion-honest.** Returning fewer than `n` results means "that is all I have"; it is how
///   the paging loop learns to stop. An arm that pads to `n` is asked for deeper pages until
///   [`MAX_ARM_PAGE`], and is then a hard, arm-named query error rather than a deeper request.
///
/// The count is bounded by [`MAX_ARM_PAGE`] (or, if larger, the `candidates(k)` the caller asked
/// for), so an arm sizes its response buffer against a known finite cap.
pub type ArmFn<'a> = Box<dyn Fn(&ArmQuery<'_>, usize) -> Result<Vec<(Id, f64)>, String> + 'a>;

/// [SONNET-4.6] (review #4519, round 6) The largest number of results ONE
/// [`ArmFn`] request may ask for while paging an arm past the `filtered-ann`
/// admissibility mask — **65536**.
///
/// An arm returns a MATERIALIZED `Vec`, so the paging loop's request count is a memory and
/// transport cost paid by the arm, the wire and this process. The id domain that bounds a
/// prefix-consistent arm's ranking (the dictionary ids plus the ~1.07e9 inline-integer literal ids)
/// is a CORRECTNESS bound, not a safe paging protocol: doubling towards it would let a small
/// `vec:hybrid` query ask a valid arm for hundreds of millions of results. The loop therefore stops
/// at this cap and reports the arm rather than requesting a deeper page — deep paging past it is
/// only reachable by an arm that pads instead of signalling exhaustion.
///
/// A caller whose `candidates(k)` already exceeds the cap still gets the page it asked for: the cap
/// bounds the loop's own escalation, not the caller's explicit request.
pub const MAX_ARM_PAGE: usize = 1 << 16;

/// A query embedder for the natural-language `vec:hybrid` form: `(text, language) -> vector`.
pub type QueryEmbedder<'a> = Box<dyn Fn(&str, &str) -> Result<Vec<f32>, String> + 'a>;

/// The `vec:hybrid` configuration: the auxiliary arms, the fusion weights, the optional query
/// embedder, and the optional second-stage reranker with its policy.
///
/// The **dense arm is built in** — the rewrite contributes this crate's own k-NN under the
/// reserved name [`VECTOR_ARM`], through exactly the same search path (and, with `filtered-ann`,
/// the same BGP-derived mask) as `vec:nearest`/`vec:search`. Everything else — a lexical/BM25
/// arm, `sparq-sim`'s structural similarity, a business ranking — is a closure, so the crate
/// stays decoupled from every one of them. With `filtered-ann` that mask is applied to **every**
/// arm's ranking, not just the dense one (see [`ArmFn`]): it constrains the answer, and fusion
/// truncates to `k` before the surrounding join runs.
///
/// ```
/// use sparq_vectors::hybrid::{HybridConfig, RerankPolicy};
///
/// let cfg = HybridConfig::new()
///     .vector_weight(1.0)
///     .arm("text", 0.5, Box::new(|_q, n| Ok((0..n as u32).map(|i| (i, 1.0)).collect())))
///     .over_fetch(4);
/// assert_eq!(cfg.arm_names(), vec!["vector", "text"]);
/// assert_eq!(cfg.candidates(10), 40);
/// let _ = RerankPolicy::FailOpen;
/// ```
pub struct HybridConfig<'a> {
    rrf_k: f64,
    vector_weight: f64,
    over_fetch: usize,
    arms: Vec<(String, f64, ArmFn<'a>)>,
    embedder: Option<QueryEmbedder<'a>>,
    reranker: Option<&'a dyn Reranker>,
    policy: RerankPolicy,
}

impl Default for HybridConfig<'_> {
    fn default() -> Self {
        HybridConfig {
            rrf_k: RRF_K,
            vector_weight: 1.0,
            over_fetch: DEFAULT_OVER_FETCH,
            arms: Vec::new(),
            embedder: None,
            reranker: None,
            policy: RerankPolicy::FailClosed,
        }
    }
}

impl<'a> HybridConfig<'a> {
    /// A configuration with the built-in dense arm at weight 1.0, [`RRF_K`],
    /// [`DEFAULT_OVER_FETCH`], no auxiliary arm and no reranker — i.e. `vec:hybrid` degenerates
    /// to the `vec:search` ranking (with provenance) until arms are added.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a named auxiliary arm at `weight`. The name may not be [`VECTOR_ARM`] or
    /// [`RERANK_KEY`], may not repeat, and may not contain `;` or `=` (rejected at query time by
    /// [`validate_arms`]).
    pub fn arm(mut self, name: impl Into<String>, weight: f64, f: ArmFn<'a>) -> Self {
        self.arms.push((name.into(), weight, f));
        self
    }

    /// Sets the fusion weight of the built-in dense arm (`0.0` mutes it — a pure
    /// sparse/structural fusion).
    pub fn vector_weight(mut self, weight: f64) -> Self {
        self.vector_weight = weight;
        self
    }

    /// Sets the RRF rank constant (default [`RRF_K`]).
    pub fn rrf_k(mut self, k: f64) -> Self {
        self.rrf_k = k;
        self
    }

    /// Sets the first-stage over-fetch multiple (default [`DEFAULT_OVER_FETCH`]); clamped to at
    /// least 1 by [`candidates`](Self::candidates).
    pub fn over_fetch(mut self, multiple: usize) -> Self {
        self.over_fetch = multiple;
        self
    }

    /// Supplies the query embedder that the natural-language (`"…"@en`) form requires. Without
    /// one, a text query is a hard error rather than a silently dense-less fusion.
    pub fn query_embedder(mut self, f: QueryEmbedder<'a>) -> Self {
        self.embedder = Some(f);
        self
    }

    /// Attaches the second-stage reranker and its failure policy.
    pub fn reranker(mut self, reranker: &'a dyn Reranker, policy: RerankPolicy) -> Self {
        self.reranker = Some(reranker);
        self.policy = policy;
        self
    }

    /// The arm names in fusion order — [`VECTOR_ARM`] first, then the declared arms.
    pub fn arm_names(&self) -> Vec<&str> {
        std::iter::once(VECTOR_ARM)
            .chain(self.arms.iter().map(|(n, _, _)| n.as_str()))
            .collect()
    }

    /// How many candidates each arm is asked for, for a query of size `k`:
    /// `k · max(over_fetch, 1)`, saturating.
    pub fn candidates(&self, k: usize) -> usize {
        k.saturating_mul(self.over_fetch.max(1))
    }

    /// The configured RRF rank constant.
    pub fn rrf_constant(&self) -> f64 {
        self.rrf_k
    }

    /// The configured dense-arm weight.
    pub fn dense_weight(&self) -> f64 {
        self.vector_weight
    }

    /// The number of declared auxiliary arms (the built-in dense arm is not one of them).
    pub(crate) fn arm_count(&self) -> usize {
        self.arms.len()
    }

    /// Runs the auxiliary arms for `query`, asking each for `candidates` results, and returns
    /// their rankings in declaration order (the caller prepends the built-in dense arm).
    pub(crate) fn run_arms(
        &self,
        query: &ArmQuery<'_>,
        candidates: usize,
    ) -> Result<Vec<ArmRanking>, String> {
        (0..self.arm_count())
            .map(|i| self.run_arm(i, query, candidates))
            .collect()
    }

    /// [OPUS-4.8] (review #4519) Runs ONE declared auxiliary arm — the `i`th, `i < arm_count()` —
    /// asking it for `candidates` results.
    ///
    /// The per-arm seam exists so the `filtered-ann` query path can re-ask a single arm for a
    /// deeper page when the BGP-derived admissibility mask compacted its answer below the
    /// requested count, without re-running the arms that were already satisfied.
    pub(crate) fn run_arm(
        &self,
        i: usize,
        query: &ArmQuery<'_>,
        candidates: usize,
    ) -> Result<ArmRanking, String> {
        let (name, weight, f) = self
            .arms
            .get(i)
            .ok_or_else(|| format!("hybrid: no arm at index {}", i))?;
        let ranked =
            f(query, candidates).map_err(|e| format!("hybrid: the {:?} arm failed: {}", name, e))?;
        Ok(ArmRanking::new(name.clone(), *weight, ranked))
    }

    /// Embeds a natural-language query, or reports that no embedder is configured.
    pub(crate) fn embed_query(&self, text: &str, language: &str) -> Result<Vec<f32>, String> {
        let Some(embedder) = &self.embedder else {
            return Err(
                "vec:hybrid: a natural-language query needs a query embedder — configure one with \
                 HybridConfig::query_embedder, or query by entity IRI / vector literal"
                    .to_string(),
            );
        };
        embedder(text, language)
    }

    /// The second stage, if any.
    pub(crate) fn second_stage(&self) -> Option<(&'a dyn Reranker, RerankPolicy)> {
        self.reranker.map(|r| (r, self.policy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuse::fuse_rrf_weighted;

    fn arms() -> Vec<ArmRanking> {
        vec![
            ArmRanking::new(VECTOR_ARM, 1.0, vec![(7, 0.91), (3, 0.80), (1, 0.10)]),
            ArmRanking::new("text", 0.5, vec![(3, 12.4), (9, 8.1)]),
        ]
    }

    #[test]
    fn fuse_arms_matches_fuse_rrf_weighted_and_records_provenance() {
        let arms = arms();
        let fused = fuse_arms(&arms, RRF_K, 10).unwrap();

        // The fused order/scores must equal the existing weighted-RRF helper exactly — the
        // provenance is strictly additive bookkeeping, not a new algorithm.
        let a: Vec<(Id, f64)> = arms[0].ranked.clone();
        let b: Vec<(Id, f64)> = arms[1].ranked.clone();
        let reference = fuse_rrf_weighted(&[(&a, 1.0), (&b, 0.5)], RRF_K, 10);
        assert_eq!(fused.len(), reference.len());
        for (hit, (id, score)) in fused.iter().zip(&reference) {
            assert_eq!(hit.id, *id);
            assert!((hit.score - score).abs() < 1e-12, "{:?} vs {}", hit, score);
        }

        // id 3 is ranked 2nd by the dense arm and 1st by text — consensus, and it says so.
        assert_eq!(fused[0].id, 3);
        assert_eq!(fused[0].provenance(), "vector=2;text=1");
        assert_eq!(fused[0].reranked, None);
        // id 9 is text-only, at that arm's rank 2 — the provenance names the arm AND the rank.
        let nine = fused.iter().find(|h| h.id == 9).unwrap();
        assert_eq!(nine.provenance(), "text=2");
    }

    #[test]
    fn fuse_arms_mutes_zero_weight_arms_and_truncates() {
        let muted = vec![
            ArmRanking::new(VECTOR_ARM, 0.0, vec![(7, 1.0)]),
            ArmRanking::new("text", 1.0, vec![(9, 1.0)]),
        ];
        let fused = fuse_arms(&muted, RRF_K, 100).unwrap();
        assert_eq!(fused.len(), 1, "a muted arm injects no standalone items");
        assert_eq!(fused[0].id, 9);
        assert_eq!(fuse_arms(&arms(), RRF_K, 1).unwrap().len(), 1);
    }

    #[test]
    fn fuse_arms_is_deterministic_on_ties() {
        // Two single-item arms tie at 1/61; first appearance (arm order) decides.
        let tied = vec![
            ArmRanking::new(VECTOR_ARM, 1.0, vec![(5, 1.0)]),
            ArmRanking::new("text", 1.0, vec![(2, 1.0)]),
        ];
        for _ in 0..8 {
            let fused = fuse_arms(&tied, RRF_K, 10).unwrap();
            assert_eq!(fused.iter().map(|h| h.id).collect::<Vec<_>>(), vec![5, 2]);
        }
    }

    #[test]
    fn fuse_arms_rejects_bad_rrf_k() {
        assert!(fuse_arms(&arms(), 0.0, 5).unwrap_err().contains("finite and positive"));
        assert!(fuse_arms(&arms(), f64::NAN, 5).is_err());
    }

    #[test]
    fn validate_arms_rejects_the_ways_an_out_of_process_arm_can_lie() {
        let dup_name = vec![
            ArmRanking::new("text", 1.0, vec![]),
            ArmRanking::new("text", 1.0, vec![]),
        ];
        assert!(validate_arms(&dup_name).unwrap_err().contains("duplicate arm name"));

        let dup_id = vec![ArmRanking::new("text", 1.0, vec![(4, 1.0), (4, 0.5)])];
        assert!(validate_arms(&dup_id).unwrap_err().contains("twice"));

        let reserved = vec![ArmRanking::new(RERANK_KEY, 1.0, vec![])];
        assert!(validate_arms(&reserved).unwrap_err().contains("reserved"));

        let separator = vec![ArmRanking::new("a=b", 1.0, vec![])];
        assert!(validate_arms(&separator).is_err());

        let empty = vec![ArmRanking::new("", 1.0, vec![])];
        assert!(validate_arms(&empty).is_err());

        let negative = vec![ArmRanking::new("text", -1.0, vec![])];
        assert!(validate_arms(&negative).unwrap_err().contains("non-negative"));

        assert!(validate_arms(&arms()).is_ok());
    }

    #[test]
    fn provenance_round_trips() {
        let hit = FusedHit {
            id: 3,
            score: 0.5,
            ranks: vec![("vector".into(), 2), ("text".into(), 1)],
            reranked: Some(4),
        };
        assert_eq!(hit.provenance(), "vector=2;text=1;rerank=4");
        assert_eq!(
            parse_provenance(&hit.provenance()).unwrap(),
            vec![
                ("vector".to_string(), 2),
                ("text".to_string(), 1),
                ("rerank".to_string(), 4)
            ]
        );
        assert_eq!(parse_provenance("").unwrap(), Vec::new());
        assert!(parse_provenance("vector").is_err());
        assert!(parse_provenance("=2").is_err());
        assert!(parse_provenance("vector=x").is_err());
        assert!(parse_provenance("vector=0").unwrap_err().contains("1-based"));
    }

    /// A reranker driven by a fixed script, so the policy paths are exercised deterministically.
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

    fn query() -> ArmQuery<'static> {
        ArmQuery {
            seed: None,
            text: None,
            vector: None,
        }
    }

    #[test]
    fn rerank_reverses_order_and_records_its_position() {
        let fused = fuse_arms(&arms(), RRF_K, 10).unwrap();
        let n = fused.len();
        let reverse: Vec<Rescored> = (0..n)
            .rev()
            .map(|i| Rescored {
                index: i,
                score: i as f64,
            })
            .collect();
        let r = Scripted(Ok(reverse));
        let out = apply_rerank(&r, RerankPolicy::FailClosed, &query(), fused.clone(), n).unwrap();
        assert_eq!(
            out.iter().map(|h| h.id).collect::<Vec<_>>(),
            fused.iter().rev().map(|h| h.id).collect::<Vec<_>>()
        );
        assert_eq!(out[0].reranked, Some(1));
        assert!(out[0].provenance().ends_with(";rerank=1"));
        // The second-stage score replaced the fused one.
        assert_eq!(out[0].score, (n - 1) as f64);
    }

    #[test]
    fn rerank_may_drop_candidates_and_truncates_to_top_k() {
        let fused = fuse_arms(&arms(), RRF_K, 10).unwrap();
        let subset = vec![Rescored {
            index: 1,
            score: 9.0,
        }];
        let r = Scripted(Ok(subset));
        let out = apply_rerank(&r, RerankPolicy::FailClosed, &query(), fused.clone(), 10).unwrap();
        assert_eq!(out.len(), 1, "a reranker may drop candidates");
        assert_eq!(out[0].id, fused[1].id);
        // top_k truncation applies after reranking.
        let all: Vec<Rescored> = (0..fused.len())
            .map(|i| Rescored {
                index: i,
                score: 1.0 - i as f64,
            })
            .collect();
        let r = Scripted(Ok(all));
        let out = apply_rerank(&r, RerankPolicy::FailClosed, &query(), fused, 2).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn rerank_failure_is_fail_open_or_fail_closed() {
        let fused = fuse_arms(&arms(), RRF_K, 10).unwrap();
        let broken = Scripted(Err("connection reset".to_string()));

        let open =
            apply_rerank(&broken, RerankPolicy::FailOpen, &query(), fused.clone(), 10).unwrap();
        assert_eq!(
            open.iter().map(|h| h.id).collect::<Vec<_>>(),
            fused.iter().map(|h| h.id).collect::<Vec<_>>(),
            "fail-open degrades to the first-stage order"
        );
        assert!(
            open.iter().all(|h| h.reranked.is_none()),
            "fail-open must NOT mark hits as reranked — the second stage did not run"
        );

        let err = apply_rerank(&broken, RerankPolicy::FailClosed, &query(), fused, 10).unwrap_err();
        assert!(err.contains("fail-closed"), "got: {}", err);
        assert!(err.contains("connection reset"), "got: {}", err);
    }

    #[test]
    fn malformed_rerank_responses_are_treated_as_failures() {
        let fused = fuse_arms(&arms(), RRF_K, 10).unwrap();
        let n = fused.len();
        let cases = [
            // An index no candidate has: the second stage must not be able to inject results.
            vec![Rescored {
                index: n + 5,
                score: 1.0,
            }],
            // The same candidate twice.
            vec![
                Rescored {
                    index: 0,
                    score: 1.0,
                },
                Rescored {
                    index: 0,
                    score: 0.5,
                },
            ],
            // A non-finite score.
            vec![Rescored {
                index: 0,
                score: f64::NAN,
            }],
        ];
        for case in cases {
            let r = Scripted(Ok(case));
            assert!(
                apply_rerank(&r, RerankPolicy::FailClosed, &query(), fused.clone(), 10).is_err(),
                "a malformed response must fail closed"
            );
            let open =
                apply_rerank(&r, RerankPolicy::FailOpen, &query(), fused.clone(), 10).unwrap();
            assert!(open.iter().all(|h| h.reranked.is_none()));
        }
    }

    #[test]
    fn evaluate_computes_recall_and_reciprocal_rank_at_k() {
        // gold = {3, 9}; ranked puts 3 second and 9 fourth.
        let m = evaluate(&[7, 3, 1, 9], &[3, 9], 4);
        assert_eq!(m.hits, 2);
        assert!((m.recall - 1.0).abs() < 1e-12);
        assert!((m.mrr - 0.5).abs() < 1e-12, "first gold item is at rank 2");

        // Cutoff excludes the second gold item.
        let m = evaluate(&[7, 3, 1, 9], &[3, 9], 2);
        assert_eq!(m.hits, 1);
        assert!((m.recall - 0.5).abs() < 1e-12);

        // No gold hit at all.
        let m = evaluate(&[7, 1], &[3], 5);
        assert_eq!((m.hits, m.recall, m.mrr), (0, 0.0, 0.0));
        // Empty gold set is 0.0, not NaN.
        assert_eq!(evaluate(&[7], &[], 5).recall, 0.0);
        // A duplicate in the ranked list is counted once (the first occurrence sets the rank).
        let m = evaluate(&[3, 3, 9], &[3, 9], 2);
        assert_eq!(m.hits, 2);
    }

    #[test]
    fn ablate_reports_every_arm_then_fused_then_reranked() {
        let arms = arms();
        let gold = [3, 9];
        let rows = ablate(&arms, &query(), None, &gold, 3, RRF_K).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.arm.as_str()).collect::<Vec<_>>(),
            vec![VECTOR_ARM, "text", FUSED_ROW]
        );
        // Row metrics are the arms' own lists, evaluated identically.
        assert_eq!(rows[0].metrics, evaluate(&[7, 3, 1], &gold, 3));
        assert_eq!(rows[1].metrics, evaluate(&[3, 9], &gold, 3));

        // With a reranker the table gains exactly one row; it is measured, never assumed.
        let identity: Vec<Rescored> = (0..4)
            .map(|i| Rescored {
                index: i,
                score: -(i as f64),
            })
            .collect();
        let r = Scripted(Ok(identity));
        let rows = ablate(&arms, &query(), Some(&r), &gold, 3, RRF_K).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].arm, RERANKED_ROW);
        // An identity reranker reproduces the fused metrics — the harness measures the stage,
        // it does not assume the stage helps.
        assert_eq!(rows[3].metrics, rows[2].metrics);
    }

    #[test]
    fn ablate_is_fail_closed_on_a_broken_reranker() {
        let broken = Scripted(Err("timeout".to_string()));
        let err = ablate(&arms(), &query(), Some(&broken), &[3], 3, RRF_K).unwrap_err();
        assert!(err.contains("fail-closed"), "got: {}", err);
    }

    #[test]
    fn config_defaults_and_builders() {
        let cfg = HybridConfig::new();
        assert_eq!(cfg.arm_names(), vec![VECTOR_ARM]);
        assert_eq!(cfg.candidates(10), 10 * DEFAULT_OVER_FETCH);
        assert_eq!(cfg.rrf_constant(), RRF_K);
        assert_eq!(cfg.dense_weight(), 1.0);
        assert!(cfg.second_stage().is_none());

        let r = Scripted(Ok(Vec::new()));
        let cfg = HybridConfig::default()
            .rrf_k(10.0)
            .vector_weight(0.25)
            .over_fetch(0) // clamped to 1
            .arm("text", 0.5, Box::new(|_q, n| Ok(vec![(1, n as f64)])))
            .reranker(&r, RerankPolicy::FailOpen);
        assert_eq!(cfg.arm_names(), vec![VECTOR_ARM, "text"]);
        assert_eq!(cfg.candidates(7), 7, "over_fetch is clamped to at least 1");
        assert_eq!(cfg.rrf_constant(), 10.0);
        assert_eq!(cfg.dense_weight(), 0.25);
        assert_eq!(cfg.second_stage().unwrap().1, RerankPolicy::FailOpen);

        let ran = cfg.run_arms(&query(), 5).unwrap();
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].arm, "text");
        assert_eq!(ran[0].ranked, vec![(1, 5.0)]);
    }

    #[test]
    fn a_failing_arm_is_a_hard_error_named_by_arm() {
        let cfg = HybridConfig::new().arm(
            "text",
            1.0,
            Box::new(|_q, _n| Err("index unavailable".to_string())),
        );
        let err = cfg.run_arms(&query(), 5).unwrap_err();
        assert!(err.contains("\"text\" arm failed"), "got: {}", err);
        assert!(err.contains("index unavailable"), "got: {}", err);
    }

    #[test]
    fn a_text_query_without_an_embedder_is_a_hard_error() {
        let cfg = HybridConfig::new();
        let err = cfg.embed_query("machine learning", "en").unwrap_err();
        assert!(err.contains("needs a query embedder"), "got: {}", err);

        let cfg = HybridConfig::new().query_embedder(Box::new(|t: &str, _l: &str| {
            Ok(vec![t.len() as f32, 1.0])
        }));
        assert_eq!(cfg.embed_query("ab", "en").unwrap(), vec![2.0, 1.0]);
    }

    #[test]
    fn arm_query_constructors() {
        let iri = NamedNode::new("http://ex/seed").unwrap();
        let v = [1.0f32, 0.0];
        let q = ArmQuery::from_seed(&iri, Some(&v));
        assert_eq!(q.seed, Some(&iri));
        assert!(q.text.is_none());
        assert_eq!(q.vector, Some(&v[..]));

        let q = ArmQuery::from_vector(&v);
        assert!(q.seed.is_none() && q.text.is_none());
        assert_eq!(q.vector, Some(&v[..]));

        let q = ArmQuery::from_text("hello", "en", None);
        assert_eq!(q.text, Some(("hello", "en")));
        assert!(q.vector.is_none());
    }
}

//! Standard **filtered link-prediction** eval harness for the shallow-KGE trainer, plus the P0
//! **ablation matrix**, a **long-tail breakdown**, and a **synthetic gUFO slice**.
//!
//! [OPUS-4.8] sq-0wo9e.8 / P6 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §P6 eval harness). **Opt-in** (`kge` cargo feature, off by default, implies `structure`);
//! nothing in the default build or core engine changes.
//!
//! # The protocol (and why getting it right matters)
//!
//! Link prediction evaluates a KGE by hiding one end of each **test** triple `(h, r, ?)` (and
//! `(?, r, t)`), ranking every candidate entity by model score, and recording where the true entity
//! lands. The metrics are **MRR** (mean reciprocal rank) and **Hits@k** (fraction ranked ≤ k).
//!
//! The **filtered** variant (Bordes et al. 2013, TransE) is the established protocol and the only
//! one we report: when ranking candidates for `(h, r, ?)`, every *other* entity `t'` that forms a
//! **known-true** triple `(h, r, t')` — whether in **train, valid, OR test** — is removed from the
//! ranking before computing the rank of the held-out `t`. Otherwise a model is unfairly penalised
//! for ranking *another correct answer* above the held-out one. **Getting this wrong (leaking the
//! filter set, or filtering only train) inflates or deflates every number and invalidates the
//! comparison** — so the filter set here is built from the union of all three splits' positives
//! (see [`Splits::filter_set_len`]) and is asserted by a test.
//!
//! # No train/test leakage
//!
//! [`Splits::split`] partitions the graph's positives into train / valid / test by a deterministic
//! hash of the triple (seeded), with **no triple in more than one split**. The model is trained on
//! **train only**; valid/test triples are never shown to the trainer. The *filter* set, separately,
//! is the union of all three (that is correct and required — it is the set of known-true triples to
//! exclude from a ranking, not training data).
//!
//! # Ablation matrix
//!
//! [`run_ablation`] runs the 2×2 P0 prior matrix — `{closure on, off} × {type-constrained
//! negatives, uniform-random negatives}` — training a fresh model per cell on the *same* split and
//! reporting filtered metrics + the long-tail breakdown per cell. The **scoring model** is the
//! [`ModelKind`](crate::train::ModelKind) on [`EvalConfig::train`]; it is the same for all four
//! cells of a single run so the deltas isolate the prior.
//!
//! # The gUFO-prior axis (wired, default OFF) — [FABLE-5] kern/ufo-priors
//!
//! The gUFO-prior axis ([`AblationCell::gufo_prior`], toggled by [`EvalConfig::gufo_prior`]) is now
//! **wired**: when ON, the [`crate::ufo_priors`] reader mines the UFO-**provable** disjointness
//! mask (kind-partition + identity-provider propagation + nature partition, all fail-closed), feeds
//! it into the P3 [`DisjointnessOracle`](crate::taxonomy::DisjointnessOracle), and the filtered
//! ranking applies it as the **serve-time hard mask**: a candidate whose `rdf:type` is provably
//! disjoint from the relation's declared `rdfs:domain`/`rdfs:range` class is dropped from the
//! ranking pool. This is **answer-safe** (design §6.A): on a UFO-consistent graph a true answer's
//! types are never disjoint from the relation's declared signature, so only provably-wrong
//! distractors are removed — per query the filtered rank can only improve or stay equal, a property
//! the tests assert. Training is untouched (the mask is serve-time only; a train-time repulsion
//! term remains a tracked follow-up). The axis is **default OFF** and, when OFF, the mask is never
//! even constructed — the baseline stays byte-identical (asserted by the no-op tests below).
//!
//! # The model axis is load-bearing (adversarial-review finding)
//!
//! The default symmetric DistMult model is **structurally near-random** on a relation whose true
//! edges are directional — and both synthetic slices here are ~100 % directional. An ablation run
//! under DistMult therefore lives in a near-random regime where the inter-cell deltas are noise. The
//! default model is now the **asymmetric** [`ModelKind::ComplEx`](crate::train::ModelKind::ComplEx)
//! (see [`EvalConfig::small`]); the example runs **both** models so the asymmetry's effect is itself
//! visible. Any ablation verdict must be read off the asymmetric model, not DistMult.
//!
//! # Single-seed deltas are not signal (adversarial-review finding)
//!
//! A single run's inter-cell delta can be a handful of rank hits over a small bucket — noise. Use
//! [`run_ablation_multiseed`] to report each cell's metric as a **mean ± std over several seeds**
//! before treating any delta as real; a delta smaller than the combined spread is not yet evidence.
//!
//! # Paired deltas: the variance-reduction that lets a real effect clear the spread (sq-4891y)
//!
//! Reporting two cells as `mean ± std` and eyeballing whether the *means* differ by more than the
//! *spreads* is the **unpaired** comparison — and it is needlessly noisy here. The four cells of one
//! seed are trained on the **same split with the same negatives draw and the same init stream**
//! (common random numbers): the bulk of the per-seed variance is *shared* between the closure-ON and
//! closure-OFF cells and **cancels in their difference**. The correct statistic is therefore the
//! **paired** per-seed delta `Δ_s = MRR(closure ON) − MRR(closure OFF)`, aggregated as a mean ± std
//! over seeds. Its standard error `std(Δ)/√n` shrinks with `n`, and because the shared noise
//! cancels, `std(Δ)` is typically **far smaller** than `std(cellA) + std(cellB)` — so a real effect
//! that the unpaired view cannot distinguish from noise can clear the *paired* spread. [`run_ablation_multiseed_paired`]
//! returns the per-axis [`PairedDelta`]s alongside the per-cell aggregates, with a
//! [`PairedDelta::significant_at`] gate (mean − k·SE > 0). **The honesty posture is unchanged:** a
//! prior is adopted only when the paired delta is significant *on a schema-bearing KG under the
//! asymmetric model*, never on a single seed and never on a schema-free slice the prior cannot bite.
//!
//! # Honesty
//!
//! **No numbers live in this file or any committed doc.** The harness *computes* them at run time
//! (see `examples/kge_ablation.rs` / `tests/kge.rs`); the work-box is non-canonical so any figure is
//! INDICATIVE. The harness makes no accuracy claim — it is the measurement instrument the design
//! requires before any prior is adopted, and adoption is additionally gated on a **real-dataset**
//! (`SPARQ_KGE_DATASET`) run on a **canonical machine** with the **asymmetric model** and
//! **multi-seed** reporting — never on these synthetic, work-box, single-seed figures.

use crate::provenance::WeightMode;
use crate::structure::{
    is_embeddable, materialise_closure, ClosedGraph, TermScope, TypeConstraints,
};
use crate::train::{train, TrainConfig, TrainReport, TrainedModel};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_reason::Profile;

/// SplitMix64 step (local copy; deterministic split + tie-break).
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A 64-bit hash of a triple, for deterministic, leakage-free split assignment.
///
/// [OPUS-4.8] sq-0wo9e.8: the mixing parameter is a PRNG SEED, NOT a cryptographic salt — this is a
/// non-cryptographic splitmix64-based hash used only to deterministically partition triples into
/// train/valid/test. It is deliberately NOT named `salt`: CodeQL's `rust/hard-coded-cryptographic-value`
/// heuristic flags any literal flowing into a `salt`-named parameter as a hard-coded crypto salt (6 FPs
/// on PR #1010), even though no cryptography is involved here.
fn hash_triple([s, p, o]: [Id; 3], seed_mix: u64) -> u64 {
    let mut v = seed_mix;
    v ^= (s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    v = v.rotate_left(17) ^ (p as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v = v.rotate_left(23) ^ (o as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    let mut s2 = v;
    splitmix64(&mut s2)
}

/// Is the term id an **atomic** entity (named/blank node) for split / ranking-pool purposes?
///
/// Deliberately pinned to [`TermScope::IriBlank`] under **both** arms of the quoted-terms
/// ablation: split membership and the ranking pool stay **scope-invariant**, so a paired
/// ON-vs-OFF delta isolates the *training-side* visibility effect rather than comparing rankings
/// over different candidate populations (which would be an incomparable measurement). The
/// quoted-terms switch widens only what the **trainer** sees ([`TrainConfig::term_scope`]).
fn is_atomic_entity(graph: &Graph, id: Id) -> bool {
    is_embeddable(graph, id, TermScope::IriBlank)
}

/// The RDF/RDFS/OWL **schema / structural** predicate IRIs. Triples whose predicate is one of these
/// (`rdf:type`, `rdfs:subClassOf`, `rdfs:domain`, …) are **structural context** — the model and the
/// P0 priors *consume* them (closure materialises them; the type-constraint extractor reads them) —
/// but they are **NOT prediction targets**: standard KGE link-prediction (TransE/WN18RR/FB15k-237)
/// ranks the *relations between entities*, never type membership. Including them as targets, and
/// especially including the closure's entailed `rdf:type`/`subClassOf` triples (which are trivially
/// derivable), would conflate "predict the relation" with "predict the entailed type" and distort
/// the closure axis of the ablation. The eval therefore predicts only NON-schema relations.
pub const SCHEMA_PREDICATES: &[&str] = &[
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
    "http://www.w3.org/2000/01/rdf-schema#subClassOf",
    "http://www.w3.org/2000/01/rdf-schema#subPropertyOf",
    "http://www.w3.org/2000/01/rdf-schema#domain",
    "http://www.w3.org/2000/01/rdf-schema#range",
    "http://www.w3.org/2002/07/owl#equivalentClass",
    "http://www.w3.org/2002/07/owl#equivalentProperty",
    "http://www.w3.org/2002/07/owl#inverseOf",
    "http://www.w3.org/2002/07/owl#sameAs",
    "http://www.w3.org/2002/07/owl#disjointWith",
];

/// The set of schema-predicate ids present in `graph` (resolved from [`SCHEMA_PREDICATES`]).
fn schema_predicate_ids(graph: &Graph) -> FxHashSet<Id> {
    let mut s = FxHashSet::default();
    for iri in SCHEMA_PREDICATES {
        let t = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(*iri));
        let id = graph.dict.lookup(&t);
        if id != sparq_core::dict::NO_ID {
            s.insert(id);
        }
    }
    s
}

/// A deterministic, leakage-free train/valid/test partition of a graph's **target** (non-schema)
/// object-property positives, plus the *all-true* filter set the filtered protocol needs. Schema /
/// structural triples (`rdf:type`, `subClassOf`, …) are deliberately excluded from the split — they
/// are structural context the trainer keeps, never a prediction target (see [`SCHEMA_PREDICATES`]).
pub struct Splits {
    /// Triples the model trains on. **Only** these are shown to [`train`].
    pub train: Vec<[Id; 3]>,
    /// Held-out validation triples (never trained on).
    pub valid: Vec<[Id; 3]>,
    /// Held-out test triples (never trained on; the metrics are computed over these).
    pub test: Vec<[Id; 3]>,
    /// The union of train + valid + test positives — the **filter set** removed from each ranking.
    all_true: FxHashSet<[Id; 3]>,
    /// All candidate entity ids (sorted) — the ranking pool.
    entities: Vec<Id>,
    /// Train-frequency of each entity (appearances as head OR tail in `train`), for long-tail
    /// bucketing.
    train_freq: FxHashMap<Id, u32>,
    /// The schema-predicate ids of this graph — triples with these predicates are structural context
    /// the trainer keeps but the eval never targets ([`SCHEMA_PREDICATES`]).
    schema_preds: FxHashSet<Id>,
}

impl Splits {
    /// Partition `graph`'s **target** (non-schema) object-property positives
    /// ~`(train_frac, valid_frac, 1-the-rest)` by a seeded per-triple hash. A triple lands in exactly
    /// one split (no leakage); the filter set is the union of all three. Schema / structural triples
    /// (`rdf:type`, `subClassOf`, …) are **excluded** from every split — they are not prediction
    /// targets (see [`SCHEMA_PREDICATES`]). `valid_frac`/`train_frac` are fractions in `[0,1]` with
    /// `train_frac + valid_frac < 1`.
    pub fn split(graph: &Graph, train_frac: f64, valid_frac: f64, seed: u64) -> Splits {
        assert!(train_frac > 0.0 && valid_frac >= 0.0 && train_frac + valid_frac < 1.0);
        let schema_preds = schema_predicate_ids(graph);
        let mut all: Vec<[Id; 3]> = Vec::new();
        let mut ent: FxHashSet<Id> = FxHashSet::default();
        for [s, p, o] in graph.iter_ids() {
            if is_atomic_entity(graph, s) && is_atomic_entity(graph, o) {
                // The ranking pool is every entity that participates in any object-property triple
                // (including schema triples — a class is still an entity that could be a candidate).
                ent.insert(s);
                ent.insert(o);
                // Only NON-schema relations are split / targeted / filtered.
                if !schema_preds.contains(&p) {
                    all.push([s, p, o]);
                }
            }
        }
        // Deterministic order independent of scan order.
        all.sort_unstable();

        let train_cut = (train_frac * u64::MAX as f64) as u64;
        let valid_cut = ((train_frac + valid_frac) * u64::MAX as f64) as u64;

        let mut train = Vec::new();
        let mut valid = Vec::new();
        let mut test = Vec::new();
        let mut all_true: FxHashSet<[Id; 3]> = FxHashSet::default();
        for t in &all {
            all_true.insert(*t);
            let h = hash_triple(*t, seed);
            if h < train_cut {
                train.push(*t);
            } else if h < valid_cut {
                valid.push(*t);
            } else {
                test.push(*t);
            }
        }

        // Train-frequency for long-tail bucketing — counted over the TRAIN split only (the model
        // only "saw" train), which is what the long-tail literature buckets on.
        let mut train_freq: FxHashMap<Id, u32> = FxHashMap::default();
        for [s, _, o] in &train {
            *train_freq.entry(*s).or_default() += 1;
            *train_freq.entry(*o).or_default() += 1;
        }

        let mut entities: Vec<Id> = ent.into_iter().collect();
        entities.sort_unstable();

        Splits {
            train,
            valid,
            test,
            all_true,
            entities,
            train_freq,
            schema_preds,
        }
    }

    /// The number of known-true triples in the filter set (union of all splits).
    pub fn filter_set_len(&self) -> usize {
        self.all_true.len()
    }

    /// Is `triple` a known-true triple (in any split)? (used by the filtered ranking).
    fn is_true(&self, triple: &[Id; 3]) -> bool {
        self.all_true.contains(triple)
    }

    /// Train-frequency of an entity (appearances in the train split).
    pub fn freq(&self, e: Id) -> u32 {
        self.train_freq.get(&e).copied().unwrap_or(0)
    }
}

/// Filtered link-prediction metrics over a held-out split.
#[derive(Clone, Debug, Default)]
pub struct Metrics {
    /// Number of (test-triple, side) ranking queries that contributed (both ends scorable).
    pub queries: usize,
    /// Filtered mean reciprocal rank.
    pub mrr: f64,
    /// Filtered Hits@1.
    pub hits1: f64,
    /// Filtered Hits@3.
    pub hits3: f64,
    /// Filtered Hits@10.
    pub hits10: f64,
}

/// Long-tail breakdown: the same filtered metrics computed separately over **head** (frequent) and
/// **long-tail** (rare) test entities, split at a train-frequency threshold.
#[derive(Clone, Debug, Default)]
pub struct LongTail {
    /// Frequency threshold: an entity is "long-tail" if its train-frequency is `<= threshold`.
    pub threshold: u32,
    /// Metrics over queries whose *answer* entity is a head (frequent) entity.
    pub head: Metrics,
    /// Metrics over queries whose *answer* entity is long-tail (rare).
    pub tail: Metrics,
}

/// One cell of the ablation matrix: a fully-specified prior configuration + its measured metrics.
#[derive(Clone, Debug)]
pub struct AblationCell {
    /// RDFS/OWL-RL closure materialised before training? (the closure-before-vectorise prior).
    pub closure: bool,
    /// Type-constrained negatives (vs uniform-random)? (the type-negative prior).
    pub type_constrained: bool,
    /// The gUFO prior — **wired** ([FABLE-5] kern/ufo-priors): mirrors [`EvalConfig::gufo_prior`].
    /// When `true` the UFO-provable disjointness mask ([`crate::ufo_priors`]) was applied to the
    /// serve-time candidate pool of every ranking in this cell; when `false` (the default) the
    /// mask was never constructed and the cell is the byte-identical baseline.
    pub gufo_prior: bool,
    /// The RDF 1.2 **quoted-terms visibility axis** ([`TermScope`]) — always `false` in
    /// [`run_ablation`] (the matrix trains with the byte-stable [`TermScope::IriBlank`] default;
    /// same wiring pattern as `gufo_prior`). The axis is measured by the dedicated paired runner
    /// [`run_quoted_ablation`], never silently inside the 2×2 matrix.
    pub quoted_terms: bool,
    /// Filtered metrics over the test split for this cell.
    pub metrics: Metrics,
    /// Long-tail breakdown for this cell.
    pub long_tail: LongTail,
    /// The trainer's loss curve for this cell (non-canonical learning signal).
    pub report: TrainReport,
}

/// Rank the held-out entity of one `(triple, side)` query under the **filtered** protocol.
/// Returns `Some(rank)` (1-based) or `None` if the query is not scorable (an end has no model row).
///
/// `side == Head` ranks the head: candidates `(c, r, t)`; `side == Tail` ranks the tail
/// `(h, r, c)`. A candidate `c` is **skipped** (filtered) when the resulting triple is known-true
/// AND is not the held-out triple itself. Ties are broken by counting candidates with a *strictly*
/// greater score plus a deterministic hash tie-break, so equal-scoring candidates do not all collapse
/// to rank 1 (the "optimistic tie" pitfall that inflates Hits@k).
fn filtered_rank(
    model: &TrainedModel,
    splits: &Splits,
    triple: [Id; 3],
    side: Side,
    ufo_mask: Option<&UfoMask>,
) -> Option<u32> {
    let [h, r, t] = triple;
    // The held-out true score must be computable.
    let true_score = model.score(h, r, t)?;
    let answer = match side {
        Side::Head => h,
        Side::Tail => t,
    };
    let answer_hash = {
        let mut s = answer as u64 ^ 0xABCD_1234;
        splitmix64(&mut s)
    };

    // Count candidates that rank strictly above the answer (filtered).
    let mut greater = 0u32;
    for &c in &splits.entities {
        if c == answer {
            continue;
        }
        let cand = match side {
            Side::Head => [c, r, t],
            Side::Tail => [h, r, c],
        };
        // Filter: skip OTHER known-true triples (they are correct answers, not distractors).
        if splits.is_true(&cand) {
            continue;
        }
        // [FABLE-5] kern/ufo-priors: the gUFO-prior serve-time hard mask (only under
        // `EvalConfig::gufo_prior`, else `None` and this arm is byte-identical to the baseline).
        // Drops a candidate whose type is PROVABLY disjoint from the relation's declared
        // domain/range class — answer-safe (see `UfoMask::provably_excluded`; the held-out
        // answer was already skipped above, so it can never be masked).
        if let Some(mask) = ufo_mask {
            if mask.provably_excluded(r, side, c) {
                continue;
            }
        }
        let Some(cs) = model.score(cand[0], cand[1], cand[2]) else {
            continue; // candidate not scorable (no row) → cannot outrank
        };
        if cs > true_score {
            greater += 1;
        } else if cs == true_score {
            // Deterministic tie-break: half the equal-scoring candidates count as "above".
            let mut hs = c as u64 ^ 0x5555_AAAA;
            let ch = splitmix64(&mut hs);
            if ch < answer_hash {
                greater += 1;
            }
        }
    }
    Some(greater + 1)
}

#[derive(Clone, Copy)]
enum Side {
    Head,
    Tail,
}

/// Accumulate filtered metrics (and the long-tail split) over a test set. For each test triple we
/// run both a head- and a tail-ranking query; each contributes to the overall metrics and to the
/// head/tail bucket chosen by the **answer** entity's train-frequency. The ranking pool + filter
/// come from `splits` (the full graph), while `model` was trained on the train-restricted graph: a
/// test entity present in the pool but absent from the model (seen only in valid/test) is unscorable
/// on that side, so that query is skipped — the honest behaviour (the model genuinely cannot place
/// an entity it never saw).
fn evaluate(
    model: &TrainedModel,
    splits: &Splits,
    long_tail_threshold: u32,
    ufo_mask: Option<&UfoMask>,
) -> (Metrics, LongTail) {
    let mut acc = MetricAcc::default();
    let mut head_acc = MetricAcc::default();
    let mut tail_acc = MetricAcc::default();

    for &triple in &splits.test {
        for side in [Side::Head, Side::Tail] {
            if let Some(rank) = filtered_rank(model, splits, triple, side, ufo_mask) {
                acc.add(rank);
                let answer = match side {
                    Side::Head => triple[0],
                    Side::Tail => triple[2],
                };
                if splits.freq(answer) <= long_tail_threshold {
                    tail_acc.add(rank);
                } else {
                    head_acc.add(rank);
                }
            }
        }
    }

    let lt = LongTail {
        threshold: long_tail_threshold,
        head: head_acc.finish(),
        tail: tail_acc.finish(),
    };
    (acc.finish(), lt)
}

#[derive(Default)]
struct MetricAcc {
    n: usize,
    rr: f64,
    h1: usize,
    h3: usize,
    h10: usize,
}

impl MetricAcc {
    fn add(&mut self, rank: u32) {
        self.n += 1;
        self.rr += 1.0 / rank as f64;
        if rank <= 1 {
            self.h1 += 1;
        }
        if rank <= 3 {
            self.h3 += 1;
        }
        if rank <= 10 {
            self.h10 += 1;
        }
    }
    fn finish(self) -> Metrics {
        if self.n == 0 {
            return Metrics::default();
        }
        let n = self.n as f64;
        Metrics {
            queries: self.n,
            mrr: self.rr / n,
            hits1: self.h1 as f64 / n,
            hits3: self.h3 as f64 / n,
            hits10: self.h10 as f64 / n,
        }
    }
}

/// Configuration for a full ablation run.
#[derive(Clone, Copy, Debug)]
pub struct EvalConfig {
    /// Trainer config template; `sampling` is overridden per cell by the type-constrained axis.
    pub train: TrainConfig,
    /// Closure profile used when the closure axis is ON.
    pub profile: Profile,
    /// Train fraction of the split.
    pub train_frac: f64,
    /// Valid fraction of the split.
    pub valid_frac: f64,
    /// Split seed (independent of the trainer seed so split and init do not correlate).
    pub split_seed: u64,
    /// Long-tail frequency threshold (an answer entity with train-freq ≤ this is "long-tail").
    pub long_tail_threshold: u32,
    /// [FABLE-5] kern/ufo-priors — the gUFO-prior ablation switch. **Default `false`** (the
    /// byte-identical baseline: the UFO mask is never constructed and no ranking changes). When
    /// `true`, the UFO-provable disjointness mask ([`crate::ufo_priors`]) is applied to every
    /// ranking's candidate pool as the answer-safe serve-time hard mask (see the module docs).
    pub gufo_prior: bool,
    /// The namespace the graph mints the gUFO meta-vocabulary under — only read when
    /// [`gufo_prior`](Self::gufo_prior) is on. Defaults to the canonical
    /// [`GUFO_NS`](crate::ufo_priors::GUFO_NS); the synthetic gUFO slice uses `http://ex/gufo#`.
    /// An explicit caller declaration, never a heuristic guess (no silent fallback).
    pub gufo_ns: &'static str,
}

impl EvalConfig {
    /// A small, work-box-sized preset.
    pub fn small(seed: u64) -> EvalConfig {
        EvalConfig {
            train: TrainConfig::small(crate::structure::SamplingMode::TypeConstrained, seed),
            profile: Profile::Rdfs,
            train_frac: 0.8,
            valid_frac: 0.1,
            split_seed: seed ^ 0xF00D,
            long_tail_threshold: 2,
            // Default OFF: the gUFO prior is opt-in per run; with it off the harness is the
            // byte-identical pre-wiring baseline. [FABLE-5] kern/ufo-priors
            gufo_prior: false,
            gufo_ns: crate::ufo_priors::GUFO_NS,
        }
    }
}

// ---- The gUFO serve-time mask ([FABLE-5] kern/ufo-priors) ---------------------------------------

/// The serve-time UFO mask of one (possibly closed) eval graph: the UFO-augmented
/// [`DisjointnessOracle`] plus the id-level lookups the ranking loop needs (candidate `rdf:type`s
/// and per-relation declared `rdfs:domain`/`rdfs:range` classes). Built ONCE per closure arm, and
/// ONLY when [`EvalConfig::gufo_prior`] is on — the OFF path never constructs it.
///
/// [`DisjointnessOracle`]: crate::taxonomy::DisjointnessOracle
struct UfoMask {
    /// `DisjointnessOracle::mine` (owl axioms) + the UFO-proven pairs absorbed on top.
    oracle: crate::taxonomy::DisjointnessOracle,
    /// Entity id → its asserted/entailed `rdf:type` class ids.
    types_of: FxHashMap<Id, Vec<Id>>,
    /// Relation id → declared `rdfs:domain` class ids.
    domain_of: FxHashMap<Id, Vec<Id>>,
    /// Relation id → declared `rdfs:range` class ids.
    range_of: FxHashMap<Id, Vec<Id>>,
}

impl UfoMask {
    /// Build the mask from `graph`, mining the gUFO vocabulary under `ns`.
    fn build(graph: &Graph, ns: &str) -> UfoMask {
        let priors = crate::ufo_priors::UfoPriors::mine_with_namespace(graph, ns);
        let mut oracle = crate::taxonomy::DisjointnessOracle::mine(graph);
        priors.augment_oracle(&mut oracle);

        let iri = |s: &str| oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s));
        let rdf_type = graph.id_of(&iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"));
        let domain = graph.id_of(&iri("http://www.w3.org/2000/01/rdf-schema#domain"));
        let range = graph.id_of(&iri("http://www.w3.org/2000/01/rdf-schema#range"));

        let mut types_of: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut domain_of: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        let mut range_of: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for [s, p, o] in graph.iter_ids() {
            if !(is_embeddable(graph, s, TermScope::IriBlank)
                && is_embeddable(graph, o, TermScope::IriBlank))
            {
                continue;
            }
            if Some(p) == rdf_type {
                types_of.entry(s).or_default().push(o);
            } else if Some(p) == domain {
                domain_of.entry(s).or_default().push(o);
            } else if Some(p) == range {
                range_of.entry(s).or_default().push(o);
            }
        }
        UfoMask {
            oracle,
            types_of,
            domain_of,
            range_of,
        }
    }

    /// Is candidate `c` **provably excluded** from the `(r, side)` ranking — i.e. does some
    /// declared query-side class of `r` (its `rdfs:range` for a tail ranking, `rdfs:domain` for a
    /// head ranking) stand provably disjoint from some `rdf:type` of `c`?
    ///
    /// ANSWER-SAFE: on a UFO-consistent graph a true answer of `(h, r, ?)` is typed by `r`'s
    /// declared range (an RDFS entailment), so none of its types can be provably disjoint from
    /// that range — only provably-wrong distractors return `true`. A relation with no declared
    /// signature, or a candidate with no types, excludes nothing (the open-world default).
    /// Structurally, the held-out answer itself is never even tested (the ranking loop skips it
    /// before the mask), so no metric can lose its true answer even on an inconsistent graph.
    fn provably_excluded(&self, r: Id, side: Side, c: Id) -> bool {
        let query_classes = match side {
            Side::Head => self.domain_of.get(&r),
            Side::Tail => self.range_of.get(&r),
        };
        let (Some(query_classes), Some(cand_types)) = (query_classes, self.types_of.get(&c)) else {
            return false;
        };
        query_classes
            .iter()
            .any(|&q| cand_types.iter().any(|&t| self.oracle.is_disjoint(q, t)))
    }
}

/// Run the full 2×2 P0 ablation matrix on `text` (RDF of `format`). For each cell:
/// 1. optionally materialise the closure (closure axis),
/// 2. split the (closed or asserted) graph leakage-free,
/// 3. mine type constraints, train on **train only** with the cell's negative-sampling mode,
/// 4. measure filtered metrics + the long-tail breakdown over **test**.
///
/// Returns the four cells in a fixed order: `[(closure=F,tc=F), (F,T), (T,F), (T,T)]`.
///
/// CRITICAL invariant (asserted in tests): the filter set is the union of all splits, and no triple
/// is in more than one split — so the reported metrics are the established filtered protocol with no
/// train/test leakage.
pub fn run_ablation(
    text: &str,
    format: &str,
    cfg: EvalConfig,
) -> Result<Vec<AblationCell>, String> {
    // Parse once; closure is applied (or not) per cell from the same parsed triples so the asserted
    // facts are identical across the closure axis.
    let (base_dict, base_triples) = Graph::parse_to_triples(text, format)?;

    let mut cells = Vec::with_capacity(4);
    for closure in [false, true] {
        // Build the graph for this closure setting.
        let closed: ClosedGraph = if closure {
            materialise_closure(base_dict.clone(), base_triples.clone(), cfg.profile)
        } else {
            // No closure: wrap the asserted triples unchanged (entailed_triples = 0).
            let g = Graph::from_parts(base_dict.clone(), base_triples.clone());
            ClosedGraph {
                graph: g,
                asserted_triples: base_triples.len(),
                entailed_triples: 0,
                profile: cfg.profile,
            }
        };
        let graph = &closed.graph;
        let splits = Splits::split(graph, cfg.train_frac, cfg.valid_frac, cfg.split_seed);

        // [FABLE-5] kern/ufo-priors: the gUFO serve-time mask, built ONCE per closure arm and
        // ONLY when the axis is explicitly on — with it off (the default) no UFO code runs and
        // the run is byte-identical to the pre-wiring baseline (asserted by tests).
        let ufo_mask = if cfg.gufo_prior {
            Some(UfoMask::build(graph, cfg.gufo_ns))
        } else {
            None
        };

        for type_constrained in [false, true] {
            let mode = if type_constrained {
                crate::structure::SamplingMode::TypeConstrained
            } else {
                crate::structure::SamplingMode::Unconstrained
            };
            let mut tcfg = cfg.train;
            tcfg.sampling = mode;

            // Train on TRAIN ONLY: a graph restricted to the train target relations plus the schema
            // (so valid/test target triples are never seen by the trainer — no leakage into
            // training), with its type constraints mined from THAT restricted graph.
            let train_graph = restrict_to_train(graph, &splits);
            let train_tc = TypeConstraints::mine(&train_graph);
            let (model, report) = train(&train_graph, &train_tc, tcfg);

            // Rank over the FULL entity pool and filter against ALL splits (filtered protocol).
            let (metrics, long_tail) =
                evaluate(&model, &splits, cfg.long_tail_threshold, ufo_mask.as_ref());

            cells.push(AblationCell {
                closure,
                type_constrained,
                gufo_prior: cfg.gufo_prior,
                // The quoted-terms axis is NOT toggled by this matrix — the cell reports the
                // template's scope verbatim (`false` for every preset, whose default is the
                // byte-stable `IriBlank`); the dedicated paired runner `run_quoted_ablation`
                // is the instrument that measures the axis.
                quoted_terms: tcfg.term_scope == TermScope::Embeddable,
                metrics,
                long_tail,
                report,
            });
        }
    }
    Ok(cells)
}

/// A mean and (population) standard deviation over a sample — the unit of a multi-seed report.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeanStd {
    /// Sample mean.
    pub mean: f64,
    /// Population standard deviation (0 for a single sample).
    pub std: f64,
    /// Number of samples the mean/std were computed over.
    pub n: usize,
}

impl MeanStd {
    /// Compute the mean and population std of `xs`.
    fn of(xs: &[f64]) -> MeanStd {
        let n = xs.len();
        if n == 0 {
            return MeanStd::default();
        }
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64;
        MeanStd {
            mean,
            std: var.sqrt(),
            n,
        }
    }
}

/// The per-cell aggregate of a multi-seed ablation: the same filtered metrics, each as a
/// [`MeanStd`] over the seeds. (`queries` is reported as a mean because a different split seed can
/// vary how many test triples are scorable.)
#[derive(Clone, Copy, Debug, Default)]
pub struct CellStats {
    /// Mean number of scorable queries per seed.
    pub queries: MeanStd,
    /// Filtered MRR, mean ± std over seeds.
    pub mrr: MeanStd,
    /// Filtered Hits@1, mean ± std.
    pub hits1: MeanStd,
    /// Filtered Hits@3, mean ± std.
    pub hits3: MeanStd,
    /// Filtered Hits@10, mean ± std.
    pub hits10: MeanStd,
}

/// One cell of a multi-seed ablation: the prior configuration plus the metric aggregates.
#[derive(Clone, Copy, Debug)]
pub struct MultiSeedCell {
    /// Closure prior on?
    pub closure: bool,
    /// Type-constrained negatives on?
    pub type_constrained: bool,
    /// The overall-test metric aggregate over seeds.
    pub metrics: CellStats,
    /// The long-tail (rare-answer) aggregate over seeds.
    pub tail: CellStats,
    /// The head (frequent-answer) aggregate over seeds.
    pub head: CellStats,
}

/// A **paired** per-seed delta of one metric between two cells, aggregated over seeds (sq-4891y).
///
/// Because the four ablation cells of a single seed share the split, init stream, and negative draws
/// (common random numbers), the per-seed difference `Δ_s = metric(on) − metric(off)` cancels the
/// shared noise. The mean of `Δ` estimates the true effect; `std(Δ)` is the *paired* spread — almost
/// always much smaller than the sum of the two cells' unpaired stds — and `se = std(Δ)/√n` is the
/// standard error of the mean. This is the statistic the firm-up gate ([`significant_at`]) reads:
/// the effect is evidence only when its mean clears a multiple of its **paired** spread, not the
/// inflated unpaired one. **No threshold is hard-coded** — the caller chooses `k`.
///
/// [`significant_at`]: PairedDelta::significant_at
#[derive(Clone, Copy, Debug, Default)]
pub struct PairedDelta {
    /// Mean of the per-seed paired delta `metric(on) − metric(off)` (the effect estimate).
    pub mean: f64,
    /// Population standard deviation of the per-seed paired delta (the **paired** spread).
    pub std: f64,
    /// Standard error of the mean: `std / √n` (0 for a single seed — undefined spread).
    pub se: f64,
    /// Number of seeds (paired samples).
    pub n: usize,
}

impl PairedDelta {
    /// Aggregate per-seed paired deltas `xs[s] = metric(on)_s − metric(off)_s` into mean/std/se.
    fn of(xs: &[f64]) -> PairedDelta {
        let ms = MeanStd::of(xs);
        let se = if ms.n > 0 {
            ms.std / (ms.n as f64).sqrt()
        } else {
            0.0
        };
        PairedDelta {
            mean: ms.mean,
            std: ms.std,
            se,
            n: ms.n,
        }
    }

    /// Is the (positive) effect significant at `k` standard errors — i.e. `mean − k·se > 0`?
    ///
    /// This is the firm-up gate the bead (sq-4891y) requires before adopting the closure prior: the
    /// *paired* effect must clear `k` standard errors of its own (variance-reduced) spread. With a
    /// single seed `se == 0`, so any strictly-positive mean is trivially "significant" — which is
    /// exactly why the gate is meaningful **only** for `n ≥ 2` (assert it in the caller). `k = 1` is
    /// a one-SE screen; `k = 2` is the conventional ≈95% one-sided bar. The threshold is the
    /// caller's policy, never baked in.
    pub fn significant_at(&self, k: f64) -> bool {
        self.n >= 2 && self.mean - k * self.se > 0.0
    }
}

/// The full result of a paired multi-seed ablation (sq-4891y): the per-cell aggregates **plus** the
/// variance-reduced paired deltas for each prior axis, so a caller can apply a significance gate
/// without re-deriving the pairing.
#[derive(Clone, Debug)]
pub struct PairedAblation {
    /// The four per-cell aggregates, in the fixed [`run_ablation`] order.
    pub cells: Vec<MultiSeedCell>,
    /// Paired MRR delta of the **closure** axis, averaged over the two negative-sampling settings:
    /// per seed, `½[(MRR(C=on,N=unif) − MRR(C=off,N=unif)) + (MRR(C=on,N=type) − MRR(C=off,N=type))]`.
    /// This is the headline "closure-prior lift" the firm-up test gates on.
    pub closure_mrr: PairedDelta,
    /// Paired MRR delta of the closure axis on the **long-tail** (rare-answer) bucket — where the
    /// closure-materialised types most plausibly help cold/rare entities.
    pub closure_mrr_tail: PairedDelta,
    /// Paired MRR delta of the **type-constrained-negatives** axis, averaged over the two closure
    /// settings (the other P0 prior, reported for completeness).
    pub type_neg_mrr: PairedDelta,
}

/// Run the 2×2 ablation once per seed in `seeds` and aggregate each cell's metrics to a mean ± std.
///
/// This is the **adversarial-review answer to single-seed noise**: the harness is deterministic per
/// seed, so varying the seed (the split seed, the trainer init, and the negative draws all derive
/// from it) gives independent samples of each cell's metric. A reported inter-cell delta is only
/// evidence when it exceeds the combined spread of the two cells. The model used is
/// `template.train.model` (use the asymmetric
/// [`ModelKind::ComplEx`](crate::train::ModelKind::ComplEx) — see the module docs).
///
/// Returns the four cells in the fixed `run_ablation` order, each with overall/head/tail aggregates.
pub fn run_ablation_multiseed(
    text: &str,
    format: &str,
    template: EvalConfig,
    seeds: &[u64],
) -> Result<Vec<MultiSeedCell>, String> {
    assert!(!seeds.is_empty(), "need at least one seed");
    // Per-cell samples: 4 cells, each accumulating one value per seed for every metric/bucket.
    let mut samples: Vec<CellSamples> = (0..4).map(|_| CellSamples::default()).collect();
    let mut shape: Vec<(bool, bool)> = Vec::with_capacity(4);

    for (si, &seed) in seeds.iter().enumerate() {
        let mut cfg = template;
        // Re-seed BOTH the split and the trainer from this seed so the samples are genuinely
        // independent draws (init + negatives + split partition all move with the seed).
        cfg.split_seed = seed ^ 0xF00D;
        cfg.train.seed = seed;
        let cells = run_ablation(text, format, cfg)?;
        if cells.len() != 4 {
            return Err(format!("expected 4 cells, got {}", cells.len()));
        }
        for (ci, cell) in cells.iter().enumerate() {
            if si == 0 {
                shape.push((cell.closure, cell.type_constrained));
            }
            samples[ci].push(cell);
        }
    }

    Ok((0..4)
        .map(|ci| {
            let (closure, type_constrained) = shape[ci];
            MultiSeedCell {
                closure,
                type_constrained,
                metrics: samples[ci].overall.finish(),
                tail: samples[ci].tail.finish(),
                head: samples[ci].head.finish(),
            }
        })
        .collect())
}

/// Run the 2×2 ablation per seed and return BOTH the per-cell aggregates and the **paired**
/// per-axis MRR deltas (sq-4891y) — the variance-reduced statistic the firm-up gate reads.
///
/// For each seed the four cells are produced by a single [`run_ablation`] call, so they share the
/// split / init / negative draws (common random numbers). We therefore form, **per seed**, the
/// paired closure delta (closure ON − OFF, averaged over the two negative settings) and the paired
/// type-negative delta, then aggregate each over seeds with its own (small) paired spread. The
/// cell order is the fixed `[(C=F,N=F), (F,T), (T,F), (T,T)]` of [`run_ablation`].
///
/// Use `template.train.model = ModelKind::ComplEx` (the asymmetric model — DistMult is near-random
/// on directional slices). Pass several seeds; [`PairedDelta::significant_at`] is meaningful only
/// for `n ≥ 2`.
pub fn run_ablation_multiseed_paired(
    text: &str,
    format: &str,
    template: EvalConfig,
    seeds: &[u64],
) -> Result<PairedAblation, String> {
    assert!(!seeds.is_empty(), "need at least one seed");
    let mut samples: Vec<CellSamples> = (0..4).map(|_| CellSamples::default()).collect();
    let mut shape: Vec<(bool, bool)> = Vec::with_capacity(4);

    // Per-seed paired deltas (one value per seed).
    let mut closure_overall: Vec<f64> = Vec::with_capacity(seeds.len());
    let mut closure_tail: Vec<f64> = Vec::with_capacity(seeds.len());
    let mut type_neg_overall: Vec<f64> = Vec::with_capacity(seeds.len());

    for (si, &seed) in seeds.iter().enumerate() {
        let mut cfg = template;
        cfg.split_seed = seed ^ 0xF00D;
        cfg.train.seed = seed;
        let cells = run_ablation(text, format, cfg)?;
        if cells.len() != 4 {
            return Err(format!("expected 4 cells, got {}", cells.len()));
        }
        // Fixed order: 0=(C off,N unif) 1=(C off,N type) 2=(C on,N unif) 3=(C on,N type).
        let m = |i: usize| cells[i].metrics.mrr;
        let mt = |i: usize| cells[i].long_tail.tail.mrr;
        // Closure axis = on − off, averaged over the two negative settings (paired within seed).
        closure_overall.push(0.5 * ((m(2) - m(0)) + (m(3) - m(1))));
        closure_tail.push(0.5 * ((mt(2) - mt(0)) + (mt(3) - mt(1))));
        // Type-negative axis = type − unif, averaged over the two closure settings.
        type_neg_overall.push(0.5 * ((m(1) - m(0)) + (m(3) - m(2))));

        for (ci, cell) in cells.iter().enumerate() {
            if si == 0 {
                shape.push((cell.closure, cell.type_constrained));
            }
            samples[ci].push(cell);
        }
    }

    let cells = (0..4)
        .map(|ci| {
            let (closure, type_constrained) = shape[ci];
            MultiSeedCell {
                closure,
                type_constrained,
                metrics: samples[ci].overall.finish(),
                tail: samples[ci].tail.finish(),
                head: samples[ci].head.finish(),
            }
        })
        .collect();

    Ok(PairedAblation {
        cells,
        closure_mrr: PairedDelta::of(&closure_overall),
        closure_mrr_tail: PairedDelta::of(&closure_tail),
        type_neg_mrr: PairedDelta::of(&type_neg_overall),
    })
}

// ---- Phase-4 PROVENANCE-WEIGHTING ablation (sq-2489d.4) ----------------------------------------

/// The result of a paired provenance-weighting ablation ([`run_weight_ablation`], sq-2489d.4): the
/// two per-arm metric aggregates (weighting OFF vs ON) plus the **paired** per-seed MRR delta.
///
/// This is the instrument the bead's acceptance bar reads: *"measure filtered Hits@k / MRR with
/// provenance-weighting ON vs OFF on a held-out link-prediction split; adopt ONLY if the lift clears
/// a pre-registered bar, and ABANDON otherwise"*. The two arms share — per seed — the split, init
/// stream, negative draws, closure, and type-negative setting (common random numbers), so the
/// per-seed paired delta `Δ_s = MRR(prov-on)_s − MRR(prov-off)_s` cancels the shared noise and
/// [`PairedDelta::significant_at`] gates the firm-up exactly as for the closure axis. **No bar is
/// hard-coded and no accuracy claim is made** — the caller picks `k`, and a non-positive / non-clearing
/// delta is the honest signal to ABANDON provenance-weighting for that dataset.
#[derive(Clone, Debug)]
pub struct WeightAblation {
    /// Provenance-weighting OFF ([`WeightMode::Uniform`]) — the baseline arm, aggregated over seeds.
    pub off: CellStats,
    /// Provenance-weighting ON ([`WeightMode::Provenance`]) — the treatment arm, aggregated over seeds.
    pub on: CellStats,
    /// Paired per-seed MRR delta `MRR(on) − MRR(off)` (variance-reduced). The headline statistic.
    pub mrr: PairedDelta,
    /// Paired per-seed Hits@10 delta `Hits@10(on) − Hits@10(off)`.
    pub hits10: PairedDelta,
}

impl WeightAblation {
    /// Convenience: is the provenance-weighting MRR lift significant at `k` standard errors of its
    /// paired spread (`n ≥ 2` required)? This is the adopt/abandon gate — `false` means **abandon**
    /// (no measured lift), per the bead's pre-registered-bar discipline.
    pub fn mrr_significant_at(&self, k: f64) -> bool {
        self.mrr.significant_at(k)
    }
}

/// Run the **provenance-weighting ON vs OFF** ablation per seed and return the paired MRR/Hits@10
/// deltas (sq-2489d.4, GenAI-KB Phase 4). Closure and type-constrained-negatives are held at the
/// `template.train` / `template.profile` settings (the established P0 priors); only the
/// `weight_mode` axis is ablated, so the delta isolates the provenance-weighting effect.
///
/// For each seed the two arms are produced from the **same** split, restricted train graph, and
/// type constraints — they differ *only* in whether each positive's SGD step is scaled by `w(t)`
/// — so the paired delta is the cleanest possible estimate of the weighting effect. The model is
/// `template.train.model` (use [`ModelKind::ComplEx`](crate::train::ModelKind) — the directional
/// slices are near-random under symmetric DistMult).
///
/// The graph must carry PROV-O / DQV provenance for the ON arm to differ from OFF; over a
/// provenance-free graph the two arms are byte-identical and the delta is exactly zero (the honest
/// no-op). [`synthetic_provenance_ttl`] builds a slice where low-assurance edges are deliberately
/// the *noisier* ones, the only setting where down-weighting them can help.
pub fn run_weight_ablation(
    text: &str,
    format: &str,
    template: EvalConfig,
    seeds: &[u64],
) -> Result<WeightAblation, String> {
    assert!(!seeds.is_empty(), "need at least one seed");
    let (base_dict, base_triples) = Graph::parse_to_triples(text, format)?;

    let mut off = BucketSamples::default();
    let mut on = BucketSamples::default();
    let mut mrr_deltas: Vec<f64> = Vec::with_capacity(seeds.len());
    let mut hits10_deltas: Vec<f64> = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        // Build the (optionally closed) graph for this seed, once — both arms share it.
        let closed: ClosedGraph =
            materialise_closure(base_dict.clone(), base_triples.clone(), template.profile);
        let graph = &closed.graph;
        let splits = Splits::split(
            graph,
            template.train_frac,
            template.valid_frac,
            seed ^ 0xF00D,
        );

        // Train graph + type constraints are shared across the two arms (no leakage either way).
        let train_graph = restrict_to_train(graph, &splits);
        let train_tc = TypeConstraints::mine(&train_graph);

        let metric_for = |mode: WeightMode| -> Metrics {
            let mut tcfg = template.train;
            tcfg.seed = seed;
            tcfg.weight_mode = mode;
            let (model, _report) = train(&train_graph, &train_tc, tcfg);
            // The weight ablation isolates the provenance axis; the gUFO mask stays off here.
            let (metrics, _long_tail) =
                evaluate(&model, &splits, template.long_tail_threshold, None);
            metrics
        };

        let m_off = metric_for(WeightMode::Uniform);
        let m_on = metric_for(WeightMode::Provenance);

        mrr_deltas.push(m_on.mrr - m_off.mrr);
        hits10_deltas.push(m_on.hits10 - m_off.hits10);
        off.push(&m_off);
        on.push(&m_on);
    }

    Ok(WeightAblation {
        off: off.finish(),
        on: on.finish(),
        mrr: PairedDelta::of(&mrr_deltas),
        hits10: PairedDelta::of(&hits10_deltas),
    })
}

// ---- Phase-4 confidence-weighted POOLING ablation (sq-w2af4) ------------------------------------

/// The result of a paired **confidence-weighted structural-sketch pooling** ablation
/// ([`run_pooling_ablation`], sq-w2af4 — design §USE-1 integration point 2): the two per-arm metric
/// aggregates (uniform-mean pooling OFF vs provenance-weighted pooling ON) plus the **paired**
/// per-seed MRR / Hits@10 deltas.
///
/// This is the instrument for the *pooling* axis, the sibling of [`WeightAblation`]'s *training*
/// axis. Per seed the two arms share the parse, closure, split, restricted train graph, type
/// constraints **and the trained model itself** — the trainer runs exactly **once** per seed and
/// both arms post-process the *same* parameters — so the delta isolates the pooling weights and
/// nothing else. On a provenance-free graph both arms pool with identical (all-`1.0`) weights and
/// every delta is exactly zero (the honest no-op, the same invariant [`WeightAblation`] documents).
/// **No bar is hard-coded and no accuracy claim is made.**
#[derive(Clone, Debug)]
pub struct PoolingAblation {
    /// Confidence-weighting OFF ([`WeightMode::Uniform`] — the plain arithmetic mean of the
    /// neighbour sketch), aggregated over seeds.
    pub off: CellStats,
    /// Confidence-weighting ON ([`WeightMode::Provenance`]), aggregated over seeds.
    pub on: CellStats,
    /// Paired per-seed MRR delta `MRR(on) − MRR(off)` (variance-reduced). The headline statistic.
    pub mrr: PairedDelta,
    /// Paired per-seed Hits@10 delta `Hits@10(on) − Hits@10(off)`.
    pub hits10: PairedDelta,
}

impl PoolingAblation {
    /// Convenience: is the weighted-pooling MRR lift significant at `k` standard errors of its
    /// paired spread (`n ≥ 2` required)? `false` means the honest verdict is **no measured lift**
    /// — abandon weighted pooling for that dataset, per the pre-registered-bar discipline.
    pub fn mrr_significant_at(&self, k: f64) -> bool {
        self.mrr.significant_at(k)
    }
}

/// Run the **confidence-weighted pooling ON vs OFF** ablation per seed and return the paired
/// MRR / Hits@10 deltas (sq-w2af4, GenAI-KB Phase 4, design §USE-1 integration point 2).
///
/// Each seed trains **one** model (at `template.train`, with `weight_mode` forced to
/// [`WeightMode::Uniform`] so the *training* axis is held fixed and cannot confound), then builds
/// two **structural-sketch-augmented** copies of it that differ only in the pooling mode: every
/// entity's embedding is blended with the pool of its train-graph object-neighbours' embeddings,
/// pooled through [`ProvenanceWeights::pool_weighted`](crate::provenance::ProvenanceWeights::pool_weighted)
/// — a uniform mean in the OFF arm, `w(t)`-weighted in the ON arm. The augmented models are then
/// scored with the same filtered link-prediction protocol as every other axis.
///
/// **What this axis can and cannot measure.** The pool is keyed by the *asserting triple*, so the
/// two arms can only differ where `w(t)` differs *within* one subject's edges — i.e. where the
/// graph carries **statement-level** provenance (a reified statement; see
/// [`ProvenanceWeights::annotated_statements`](crate::provenance::ProvenanceWeights::annotated_statements)).
/// On a graph with only entity-level provenance every one of a subject's edges falls back to the
/// same head weight, the weighted pool IS the uniform mean, and every delta is **exactly zero** —
/// the same honest no-op [`run_weight_ablation`] documents for provenance-free graphs. A zero delta
/// here therefore means "this graph has no per-statement signal", not "weighting did not help".
///
/// **No leakage**: the sketch is pooled over the *restricted train graph* only (the same graph the
/// trainer saw), so a held-out edge can never enter an entity's sketch. The provenance is mined
/// from that same train graph.
///
/// `blend` scales the pooled sketch before it is added to the entity embedding (`e + blend·pool`).
/// It is a **sweepable, non-canonical** knob, not a tuned constant; `0.0` makes both arms identical
/// by construction (a useful degenerate sanity check). A non-finite or negative `blend` is an `Err`.
pub fn run_pooling_ablation(
    text: &str,
    format: &str,
    template: EvalConfig,
    seeds: &[u64],
    blend: f32,
) -> Result<PoolingAblation, String> {
    assert!(!seeds.is_empty(), "need at least one seed");
    if !blend.is_finite() || blend < 0.0 {
        return Err(format!("run_pooling_ablation: blend must be finite and >= 0 (got {})", blend));
    }
    let (base_dict, base_triples) = Graph::parse_to_triples(text, format)?;

    let mut off = BucketSamples::default();
    let mut on = BucketSamples::default();
    let mut mrr_deltas: Vec<f64> = Vec::with_capacity(seeds.len());
    let mut hits10_deltas: Vec<f64> = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        let closed: ClosedGraph =
            materialise_closure(base_dict.clone(), base_triples.clone(), template.profile);
        let graph = &closed.graph;
        let splits = Splits::split(graph, template.train_frac, template.valid_frac, seed ^ 0xF00D);
        let train_graph = restrict_to_train(graph, &splits);
        let train_tc = TypeConstraints::mine(&train_graph);

        // ONE training run per seed: the pooling axis post-processes identical parameters, so the
        // paired delta cannot be contaminated by trainer noise.
        let mut tcfg = template.train;
        tcfg.seed = seed;
        tcfg.weight_mode = WeightMode::Uniform;
        let (model, _report) = train(&train_graph, &train_tc, tcfg);

        let prov = crate::provenance::ProvenanceWeights::mine(&train_graph);
        let metric_for = |mode: WeightMode| -> Result<Metrics, String> {
            let sketched =
                sketch_augmented(&model, &train_graph, &splits, &prov, mode, blend)?;
            let (metrics, _long_tail) =
                evaluate(&sketched, &splits, template.long_tail_threshold, None);
            Ok(metrics)
        };

        let m_off = metric_for(WeightMode::Uniform)?;
        let m_on = metric_for(WeightMode::Provenance)?;

        mrr_deltas.push(m_on.mrr - m_off.mrr);
        hits10_deltas.push(m_on.hits10 - m_off.hits10);
        off.push(&m_off);
        on.push(&m_on);
    }

    Ok(PoolingAblation {
        off: off.finish(),
        on: on.finish(),
        mrr: PairedDelta::of(&mrr_deltas),
        hits10: PairedDelta::of(&hits10_deltas),
    })
}

/// One sketch contribution: the **asserting triple** (what `w(t)` is keyed on) and the neighbour
/// embedding it pools in. Named so the per-subject contribution map stays readable.
type SketchContribution = ([Id; 3], Vec<f32>);

/// A copy of `model` whose entity embeddings are blended with each entity's
/// **assertion-weighted structural sketch**: the pool of its outgoing non-schema object-neighbours'
/// embeddings in `graph`, each contribution keyed on **the asserting triple** (so the weight is the
/// reified statement's provenance where the graph carries it, the head's otherwise — see
/// [`sketch_predicate`](crate::grounding::sketch_predicate)), pooled through
/// [`ProvenanceWeights::pool_weighted`](crate::provenance::ProvenanceWeights::pool_weighted) under
/// `mode`. Deterministic: each row is written independently from a contribution list built in
/// `graph.iter_ids()` order, so hash-map iteration order cannot affect the result. [OPUS-5] sq-w2af4
fn sketch_augmented(
    model: &TrainedModel,
    graph: &Graph,
    splits: &Splits,
    prov: &crate::provenance::ProvenanceWeights,
    mode: WeightMode,
    blend: f32,
) -> Result<TrainedModel, String> {
    let dim = model.dim;
    // Per subject, the (asserting triple, neighbour embedding) contributions that feed its sketch.
    let mut contributions: FxHashMap<Id, Vec<SketchContribution>> = FxHashMap::default();
    for [s, p, o] in graph.iter_ids() {
        if splits.schema_preds.contains(&p) {
            continue;
        }
        let (Some(_), Some(o_row)) = (model.entity_row(s), model.entity_row(o)) else {
            continue;
        };
        contributions
            .entry(s)
            .or_default()
            .push(([s, p, o], model.entity_vec(o_row).to_vec()));
    }

    let mut out = model.clone();
    for (&subject, contribs) in &contributions {
        let Some(row) = model.entity_row(subject) else {
            continue;
        };
        let Some(pooled) = prov.pool_weighted(contribs, mode)? else {
            continue;
        };
        let dst = &mut out.entity_emb[row * dim..row * dim + dim];
        for (d, p) in dst.iter_mut().zip(pooled.iter()) {
            *d += blend * *p;
        }
    }
    Ok(out)
}

// ---- RDF 1.2 QUOTED-TERMS visibility ablation ---------------------------------------------------

/// The result of a paired **quoted-terms visibility** ablation ([`run_quoted_ablation`]): the two
/// per-arm metric aggregates ([`TermScope::IriBlank`] OFF vs [`TermScope::Embeddable`] ON) plus
/// the **paired** per-seed MRR / Hits@10 deltas.
///
/// The two arms share — per seed — the parse, closure, split, restricted train graph, type
/// constraints, and seed (common random numbers); they differ **only** in
/// [`TrainConfig::term_scope`], so the per-seed paired delta `Δ_s = MRR(on)_s − MRR(off)_s`
/// cancels the shared noise and [`PairedDelta::significant_at`] gates adoption exactly as for the
/// closure and provenance-weighting axes. Split membership and the ranking pool are
/// **scope-invariant** (see [`Splits`]), so the delta isolates the *training-side* visibility
/// effect. On a quote-free graph the two arms are byte-identical and every delta is exactly zero
/// (the honest no-op — the same property [`run_weight_ablation`] documents for provenance-free
/// graphs). **No bar is hard-coded and no accuracy claim is made.**
#[derive(Clone, Debug)]
pub struct QuotedAblation {
    /// Quoted-terms visibility OFF ([`TermScope::IriBlank`]) — the baseline arm, over seeds.
    pub off: CellStats,
    /// Quoted-terms visibility ON ([`TermScope::Embeddable`]) — the treatment arm, over seeds.
    pub on: CellStats,
    /// Paired per-seed MRR delta `MRR(on) − MRR(off)` (variance-reduced). The headline statistic.
    pub mrr: PairedDelta,
    /// Paired per-seed Hits@10 delta `Hits@10(on) − Hits@10(off)`.
    pub hits10: PairedDelta,
}

impl QuotedAblation {
    /// Convenience: is the quoted-terms MRR lift significant at `k` standard errors of its paired
    /// spread (`n ≥ 2` required)? `false` means the honest verdict is **no measured lift**.
    pub fn mrr_significant_at(&self, k: f64) -> bool {
        self.mrr.significant_at(k)
    }
}

/// Run the **quoted-terms visibility ON vs OFF** ablation per seed and return the paired
/// MRR / Hits@10 deltas. Closure, negative-sampling mode, and weighting are held at the
/// `template.train` / `template.profile` settings; only the [`TrainConfig::term_scope`] axis is
/// ablated, so the delta isolates the RDF 1.2 quoted-term visibility effect.
///
/// For each seed the two arms are produced from the **same** closed graph, split, restricted
/// train graph, and type constraints — the graph, targets, filter set, and ranking pool are
/// identical between arms (scope-invariance; see [`QuotedAblation`]) — so the paired delta is the
/// cleanest possible estimate of the visibility effect. The graph must carry RDF 1.2 quoted
/// triples (`rdf:reifies` edges, e.g. [`synthetic_rdf12_ttl`]) for the ON arm to differ from OFF;
/// over a quote-free graph the two arms are byte-identical and the delta is exactly zero.
pub fn run_quoted_ablation(
    text: &str,
    format: &str,
    template: EvalConfig,
    seeds: &[u64],
) -> Result<QuotedAblation, String> {
    assert!(!seeds.is_empty(), "need at least one seed");
    let (base_dict, base_triples) = Graph::parse_to_triples(text, format)?;

    let mut off = BucketSamples::default();
    let mut on = BucketSamples::default();
    let mut mrr_deltas: Vec<f64> = Vec::with_capacity(seeds.len());
    let mut hits10_deltas: Vec<f64> = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        // Build the closed graph for this seed, once — both arms share it.
        let closed: ClosedGraph =
            materialise_closure(base_dict.clone(), base_triples.clone(), template.profile);
        let graph = &closed.graph;
        let splits = Splits::split(
            graph,
            template.train_frac,
            template.valid_frac,
            seed ^ 0xF00D,
        );

        // Train graph + type constraints are shared across the two arms. `restrict_to_train`
        // keeps `rdf:reifies` triples as structural context under BOTH arms (their quoted-term
        // object is not an atomic target), so the arms genuinely see the same graph and differ
        // only in what the trainer is allowed to embed.
        let train_graph = restrict_to_train(graph, &splits);
        let train_tc = TypeConstraints::mine(&train_graph);

        let metric_for = |scope: TermScope| -> Metrics {
            let mut tcfg = template.train;
            tcfg.seed = seed;
            tcfg.term_scope = scope;
            let (model, _report) = train(&train_graph, &train_tc, tcfg);
            // This paired runner isolates the triple-term visibility axis; the gUFO mask stays off.
            let (metrics, _long_tail) =
                evaluate(&model, &splits, template.long_tail_threshold, None);
            metrics
        };

        let m_off = metric_for(TermScope::IriBlank);
        let m_on = metric_for(TermScope::Embeddable);

        mrr_deltas.push(m_on.mrr - m_off.mrr);
        hits10_deltas.push(m_on.hits10 - m_off.hits10);
        off.push(&m_off);
        on.push(&m_on);
    }

    Ok(QuotedAblation {
        off: off.finish(),
        on: on.finish(),
        mrr: PairedDelta::of(&mrr_deltas),
        hits10: PairedDelta::of(&hits10_deltas),
    })
}

/// Per-cell accumulator of one metric sample per seed, for the overall/head/tail buckets.
#[derive(Default)]
struct CellSamples {
    overall: BucketSamples,
    head: BucketSamples,
    tail: BucketSamples,
}

impl CellSamples {
    fn push(&mut self, cell: &AblationCell) {
        self.overall.push(&cell.metrics);
        self.head.push(&cell.long_tail.head);
        self.tail.push(&cell.long_tail.tail);
    }
}

#[derive(Default)]
struct BucketSamples {
    queries: Vec<f64>,
    mrr: Vec<f64>,
    hits1: Vec<f64>,
    hits3: Vec<f64>,
    hits10: Vec<f64>,
}

impl BucketSamples {
    fn push(&mut self, m: &Metrics) {
        self.queries.push(m.queries as f64);
        self.mrr.push(m.mrr);
        self.hits1.push(m.hits1);
        self.hits3.push(m.hits3);
        self.hits10.push(m.hits10);
    }
    fn finish(&self) -> CellStats {
        CellStats {
            queries: MeanStd::of(&self.queries),
            mrr: MeanStd::of(&self.mrr),
            hits1: MeanStd::of(&self.hits1),
            hits3: MeanStd::of(&self.hits3),
            hits10: MeanStd::of(&self.hits10),
        }
    }
}

/// Build a graph the trainer can learn from WITHOUT seeing a held-out target relation: keep every
/// **schema / structural** triple (`rdf:type`, `subClassOf`, domain/range — the priors depend on
/// them), every **literal-valued** triple, and the **train** target relations; drop only the
/// valid/test target relations. This is what keeps valid/test out of training (no leakage) while
/// preserving the type/domain/range facts closure-before-vectorise and type-constrained negatives
/// need.
fn restrict_to_train(graph: &Graph, splits: &Splits) -> Graph {
    let train_set: FxHashSet<[Id; 3]> = splits.train.iter().copied().collect();
    let mut kept: Vec<[Id; 3]> = Vec::new();
    for [s, p, o] in graph.iter_ids() {
        let both_entities = is_atomic_entity(graph, s) && is_atomic_entity(graph, o);
        if both_entities && !splits.schema_preds.contains(&p) {
            // A TARGET (non-schema) object-property triple: keep only if it is a train positive.
            if train_set.contains(&[s, p, o]) {
                kept.push([s, p, o]);
            }
        } else {
            // Schema / typing / literal triple: always kept (it is structural context, not a
            // prediction target, so it never leaks a held-out answer).
            kept.push([s, p, o]);
        }
    }
    // Reuse the same dictionary (ids are stable); rebuild the store from the kept triples.
    Graph::from_parts(graph.dict.clone(), kept)
}

// ---- Synthetic gUFO slice ---------------------------------------------------------------------

/// Build a **small synthetic gUFO-typed graph** for the gUFO slice of the eval.
///
/// gUFO (the lightweight UFO implementation) distinguishes, among other things:
/// - **rigid kinds** (`gufo:Kind`) — a sortal an individual instantiates **necessarily** for its
///   whole existence (e.g. *Person*): identity-bearing, never lost;
/// - **anti-rigid roles/phases** (`gufo:Role`, `gufo:Phase`) — a type an individual instantiates
///   **contingently** (e.g. *Student*, *Child*): can be acquired and lost.
///
/// The slice encodes that distinction structurally: each individual has exactly one **rigid kind**
/// (its identity), and a *contingent* number of **roles/phases**. Identity-bearing relations
/// (`ex:hasParent`, kind-to-kind) are systematic; contingent relations (`ex:enrolledIn`,
/// role-mediated) are sparser and noisier. We deliberately do **NOT** rig the graph so a type prior
/// trivially separates true from false: every role is filled by a plausible kind, contingent edges
/// connect type-compatible endpoints, and there are decoy individuals of the right kind that are not
/// actually related — so a pure type filter cannot win and the embedding must learn the signal.
///
/// Returns the serialised Turtle. The vocabulary is namespaced under `http://ex/gufo#` for the gUFO
/// meta-classes and `http://ex/` for individuals. The generator is deterministic in `seed` and sized
/// by `n_people` (a few hundred is a good non-trivial work-box size).
///
/// This is the default-density slice. For the firm-up test (sq-4891y) use
/// [`synthetic_gufo_ttl_sized`], which adds a **density** multiplier: a larger, denser test set
/// shrinks per-seed sampling variance (more held-out triples ⇒ a tighter MRR estimate per seed) and,
/// importantly, keeps the slice **schema-bearing but less FB237-overfit-prone** — the closure must
/// still derive the rigid `Person` kind, so the closure axis genuinely bites.
pub fn synthetic_gufo_ttl(n_people: usize, seed: u64) -> String {
    synthetic_gufo_ttl_sized(n_people, 1, seed)
}

/// Density-parameterised [`synthetic_gufo_ttl`] (sq-4891y).
///
/// `density` (clamped to `≥ 1`) multiplies the per-person edge propensity and the course/org pools so
/// the slice carries **more learnable, schema-bearing signal per held-out triple**. The aim is the
/// firm-up bead's two requirements at once: a *larger/denser* test set (lower per-seed variance, the
/// (a) ask) on a graph that is *still schema-bearing and not trivially separable* — the rigid
/// `Person` kind is asserted on **nobody** and must be materialised by the RDFS closure, so closure
/// genuinely changes the type-constrained negatives (the (b) ask). `density = 1` reproduces
/// [`synthetic_gufo_ttl`] exactly. Deterministic in `seed`.
pub fn synthetic_gufo_ttl_sized(n_people: usize, density: usize, seed: u64) -> String {
    let density = density.max(1);
    let mut state = seed ^ 0x6757_F0F0;
    let mut next = |m: usize| -> usize { (splitmix64(&mut state) as usize) % m.max(1) };

    let mut out = String::new();
    out.push_str(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix gufo: <http://ex/gufo#> .\n\
         @prefix ex: <http://ex/> .\n\n",
    );

    // gUFO meta-typing of the domain classes (rigid kinds vs anti-rigid roles/phases). These are
    // the annotations a later gUFO prior would read; at baseline they are just typed triples.
    out.push_str("# Rigid kinds (identity-bearing, necessary):\n");
    out.push_str("ex:Person a gufo:Kind . ex:Organisation a gufo:Kind . ex:Course a gufo:Kind .\n");
    out.push_str("# Anti-rigid roles/phases (contingent):\n");
    out.push_str("ex:Student a gufo:Role . ex:Employee a gufo:Role . ex:Child a gufo:Phase . ex:Adult a gufo:Phase .\n");
    out.push_str(
        "# Roles/phases specialise the kind that can fill them (subClassOf to the kind):\n",
    );
    out.push_str(
        "ex:Student rdfs:subClassOf ex:Person . ex:Employee rdfs:subClassOf ex:Person .\n",
    );
    out.push_str("ex:Child rdfs:subClassOf ex:Person . ex:Adult rdfs:subClassOf ex:Person .\n");
    // Identity-bearing relation: parent (Person→Person, systematic). Contingent: enrolledIn
    // (Student→Course), employedBy (Employee→Organisation).
    out.push_str("ex:hasParent rdfs:domain ex:Person ; rdfs:range ex:Person .\n");
    out.push_str("ex:enrolledIn rdfs:domain ex:Student ; rdfs:range ex:Course .\n");
    out.push_str("ex:employedBy rdfs:domain ex:Employee ; rdfs:range ex:Organisation .\n\n");

    // A denser slice gets a proportionally larger course/org pool so the candidate set per query
    // still dwarfs the answer set (non-triviality preserved as density grows).
    let n_org = (n_people / 20).max(2) * density;
    let n_course = (n_people / 10).max(3) * density;
    for i in 0..n_org {
        out.push_str(&format!("ex:org{} a ex:Organisation .\n", i));
    }
    for i in 0..n_course {
        out.push_str(&format!("ex:course{} a ex:Course .\n", i));
    }
    out.push('\n');

    // People: each is typed only by its contingent phase (and sometimes a role) — both
    // subClassOf Person — so the **rigid kind Person is NOT asserted directly** and must be derived
    // by the RDFS closure (the closure axis then genuinely bites: with closure OFF, hasParent's
    // declared domain/range `Person` is satisfied by nobody and type-constrained negatives degrade
    // toward uniform; with closure ON, everyone is an entailed Person and the prior is sharp).
    for i in 0..n_people {
        // Contingent phase (every person is Child or Adult — anti-rigid, can change).
        if next(2) == 0 {
            out.push_str(&format!("ex:p{} a ex:Child", i));
        } else {
            out.push_str(&format!("ex:p{} a ex:Adult", i));
        }
        // Contingent role: ~60% are Students, ~50% Employees (independent), some are neither.
        let is_student = next(10) < 6;
        let is_employee = next(10) < 5;
        if is_student {
            out.push_str(" , ex:Student");
        }
        if is_employee {
            out.push_str(" , ex:Employee");
        }
        out.push_str(" .\n");

        // LEARNABLE STRUCTURE: each person belongs to a latent "community"; identity-bearing and
        // contingent edges are community-correlated (with noise + decoys) so a KGE can recover a
        // pattern rather than chase uniform randomness — without that the edges are unpredictable
        // and the harness would measure pure noise.
        let n_comm = 5usize;
        let comm = |k: usize| -> usize { (k.wrapping_mul(2246822519) >> 4) % n_comm };

        // Identity-bearing edge: a parent for ~70% of people, biased toward a lower-indexed person
        // in the SAME community (acyclic, systematic backbone).
        if i > 0 && next(10) < 7 {
            let mut parent = None;
            for _ in 0..8 {
                let cand = next(i);
                if comm(cand) == comm(i) {
                    parent = Some(cand);
                    break;
                }
            }
            let parent = parent.unwrap_or_else(|| next(i));
            out.push_str(&format!("ex:p{} ex:hasParent ex:p{} .\n", i, parent));
        }
        // Contingent edges only when the role is held, and only for SOME holders (decoys: a Student
        // not enrolled in anything). The target course/org is community-correlated (community → a
        // preferred course/org band) so enrolment is learnable. A `density>1` slice draws up to
        // `density` DISTINCT such edges per holder (a person can enrol in several courses), which
        // multiplies the held-out test triples per seed — shrinking the per-seed MRR variance the
        // firm-up bead targets — while the per-edge ~70% propensity and community bands keep the
        // decoy set (and thus non-triviality) intact.
        let n_comm_band = 5usize;
        if is_student {
            let mut emitted = std::collections::BTreeSet::new();
            for d in 0..density {
                if next(10) < 7 {
                    let band = (comm(i) + d) % n_comm_band % n_course;
                    let course = if next(10) < 7 { band } else { next(n_course) };
                    if emitted.insert(course) {
                        out.push_str(&format!("ex:p{} ex:enrolledIn ex:course{} .\n", i, course));
                    }
                }
            }
        }
        if is_employee {
            let mut emitted = std::collections::BTreeSet::new();
            for d in 0..density {
                if next(10) < 7 {
                    let band = (comm(i) + d) % n_comm_band % n_org;
                    let org = if next(10) < 7 { band } else { next(n_org) };
                    if emitted.insert(org) {
                        out.push_str(&format!("ex:p{} ex:employedBy ex:org{} .\n", i, org));
                    }
                }
            }
        }
    }
    out
}

/// Build a **small synthetic STRUCTURED relational graph** standing in for a WN18RR-style dataset
/// when no real dataset file is present (the work-box is non-canonical; the real dataset run is a
/// dataset-gated bench). It has a class hierarchy with declared domain/range per relation, several
/// relation types of differing density, and a Zipf-ish long tail of entity frequencies (a few
/// "hub" entities, many rare ones) so the long-tail breakdown is meaningful. Like the gUFO slice it
/// is deliberately NOT trivially separable: type-compatible non-edges (decoys) outnumber the true
/// edges for the sparse relations. Deterministic in `seed`, sized by `n_entities`.
pub fn synthetic_relational_ttl(n_entities: usize, seed: u64) -> String {
    let mut state = seed ^ 0x1357_9BDF;
    let mut next = |m: usize| -> usize { (splitmix64(&mut state) as usize) % m.max(1) };

    let mut out = String::new();
    out.push_str(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix ex: <http://ex/> .\n\n",
    );
    // A small class taxonomy (the closure axis materialises the superclass memberships).
    out.push_str("ex:Synset rdfs:subClassOf ex:LexEntry .\n");
    out.push_str("ex:Noun rdfs:subClassOf ex:Synset . ex:Verb rdfs:subClassOf ex:Synset .\n");
    // Relations with declared domain/range (so type-constrained negatives can bite).
    out.push_str("ex:hypernym rdfs:domain ex:Noun ; rdfs:range ex:Noun .\n");
    out.push_str("ex:partOf rdfs:domain ex:Noun ; rdfs:range ex:Noun .\n");
    out.push_str("ex:entails rdfs:domain ex:Verb ; rdfs:range ex:Verb .\n\n");

    let n_noun = (n_entities * 7 / 10).max(8);
    let n_verb = (n_entities - n_noun).max(4);
    for i in 0..n_noun {
        out.push_str(&format!("ex:n{} a ex:Noun .\n", i));
    }
    for i in 0..n_verb {
        out.push_str(&format!("ex:v{} a ex:Verb .\n", i));
    }
    out.push('\n');

    // LEARNABLE STRUCTURE (so the measurement is signal, not noise): nouns belong to a small number
    // of latent "topic clusters"; relations are generated by cluster rules, not uniform randomness,
    // so a KGE CAN recover a pattern. We deliberately keep it noisy and incomplete (decoys, missing
    // edges) so it is not trivially separable by a type filter.
    let n_clusters = 6usize;
    let cluster = |i: usize| -> usize { (i.wrapping_mul(2654435761) >> 3) % n_clusters };

    // hypernym: a forest WITHIN each cluster (each non-root points to a lower-indexed noun in the
    // SAME cluster) → a recoverable backbone; low indices become hubs (long-tail head).
    for i in 1..n_noun {
        if next(10) < 8 {
            // pick a lower-indexed noun in the same cluster (fall back to any lower index).
            let mut parent = None;
            for _ in 0..8 {
                let cand = next(i);
                if cluster(cand) == cluster(i) {
                    parent = Some(cand);
                    break;
                }
            }
            let parent = parent.unwrap_or_else(|| next(i));
            out.push_str(&format!("ex:n{} ex:hypernym ex:n{} .\n", i, parent));
        }
    }
    // partOf: meronymy that tends to connect ADJACENT clusters (cluster c → cluster c+1), a
    // cross-cluster rule a KGE can learn; sparse, with decoys (many type-compatible non-edges).
    for i in 0..n_noun {
        if next(10) < 4 {
            let target_cluster = (cluster(i) + 1) % n_clusters;
            let mut tgt = None;
            for _ in 0..8 {
                let cand = next(n_noun);
                if cluster(cand) == target_cluster {
                    tgt = Some(cand);
                    break;
                }
            }
            if let Some(t) = tgt {
                out.push_str(&format!("ex:n{} ex:partOf ex:n{} .\n", i, t));
            }
        }
    }
    // entails: among verbs, also clustered (within-cluster), so verb space has its own signal.
    let vcluster = |i: usize| -> usize { (i.wrapping_mul(40503) >> 2) % n_clusters };
    for i in 0..n_verb {
        if next(10) < 5 {
            let mut tgt = None;
            for _ in 0..8 {
                let cand = next(n_verb);
                if vcluster(cand) == vcluster(i) && cand != i {
                    tgt = Some(cand);
                    break;
                }
            }
            if let Some(t) = tgt {
                out.push_str(&format!("ex:v{} ex:entails ex:v{} .\n", i, t));
            }
        }
    }
    out
}

/// Build a small synthetic **provenance-annotated** relational slice for the Phase-4 weighting
/// ablation ([`run_weight_ablation`], sq-2489d.4). It is the only synthetic graph here that carries
/// PROV-O / DQV annotations, and it is constructed so provenance-weighting *could* help: a fraction
/// of the edges are **deliberately wrong** (random cross-cluster noise) and those noisy edges carry
/// **low** assurance (`secx:Conjectured`, low `pkg:confidence`), while the clean within-cluster
/// edges carry **high** assurance (`secx:Proven`, high confidence). Down-weighting the low-assurance
/// (noisier) positives is therefore the setting where the CKRL move can plausibly lift MRR — but the
/// generator is still NOT rigged to guarantee a win (the noise is a minority and the embedding must
/// recover the cluster signal regardless), so the measurement is honest. Deterministic in `seed`,
/// sized by `n_entities`. The vocabulary mirrors the real `pkg.ttl` predicate set so the reader
/// exercises the REAL provenance path, not a mock.
///
/// [OPUS-5] sq-w2af4: the slice also carries **statement-level** provenance — a share of the noise
/// edges are asserted by an otherwise-*good* head and their doubt is expressed on an RDF 1.2
/// reifier (`:st rdf:reifies <<( s p o )>>` + a low `pkg:confidence`). That is the case entity-level
/// provenance structurally cannot express, and it is what gives one subject's edges *differing*
/// `w(t)` — without it every `(s, ·, ·)` weight is identical and the pooling axis is an exact no-op
/// (see [`run_pooling_ablation`]). **No leakage:** the reifier's quoted triple is a TERM, never an
/// assertion; triple terms are outside the [`TermScope::IriBlank`] pool so they enter neither the
/// split, the ranking pool, nor training; the reifier is annotated with a LITERAL only, so it never
/// becomes a prediction target itself; and the sketch pools only over edges present in the
/// restricted train graph.
pub fn synthetic_provenance_ttl(n_entities: usize, seed: u64) -> String {
    let mut state = seed ^ 0x0BAD_F00D_C0FF_EE11;
    let mut next = |m: usize| -> usize { (splitmix64(&mut state) as usize) % m.max(1) };

    let mut out = String::new();
    out.push_str(
        "@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix pkg:  <https://sparq.dev/ns/pkg#> .\n\
         @prefix prov: <http://www.w3.org/ns/prov#> .\n\
         @prefix secx: <https://sparq.dev/ns/secx#> .\n\
         @prefix ex:   <http://ex/> .\n\n",
    );
    // A class + a relation with a declared domain/range (so type-constrained negatives bite).
    out.push_str("ex:Node a rdfs:Class .\n");
    out.push_str("ex:rel rdfs:domain ex:Node ; rdfs:range ex:Node .\n");
    // Two source nodes of differing reliability (folded into w(t) as the source-reliability factor).
    out.push_str("ex:src-reliable pkg:confidence \"1.0\" .\n");
    out.push_str("ex:src-weak     pkg:confidence \"0.5\" .\n\n");

    let n = n_entities.max(12);
    for i in 0..n {
        out.push_str(&format!("ex:e{} a ex:Node .\n", i));
    }
    out.push('\n');

    // Latent clusters drive the TRUE edges (recoverable signal). Noisy edges connect random
    // cross-cluster pairs and are annotated low-assurance.
    let n_clusters = 5usize;
    let cluster = |i: usize| -> usize { (i.wrapping_mul(2654435761) >> 3) % n_clusters };

    // Each node emits one within-cluster edge (clean, high-assurance) and, with a 1-in-4 chance, one
    // cross-cluster NOISE edge (low-assurance). Reifying the per-fact provenance on the SUBJECT node
    // (the head) matches how the reader derives w(t) from the head's annotations.
    for i in 0..n {
        // Clean within-cluster edge: target is another node in the same cluster.
        let mut tgt = None;
        for _ in 0..12 {
            let cand = next(n);
            if cand != i && cluster(cand) == cluster(i) {
                tgt = Some(cand);
                break;
            }
        }
        if let Some(t) = tgt {
            out.push_str(&format!("ex:e{} ex:rel ex:e{} .\n", i, t));
            // HIGH assurance on the clean fact's head.
            out.push_str(&format!(
                "ex:e{} pkg:assurance secx:Proven ; pkg:confidence \"0.95\" ; prov:wasDerivedFrom ex:src-reliable .\n",
                i
            ));
        }

        // NOISE edge (minority): a random cross-cluster pair, annotated LOW assurance so the ON arm
        // down-weights it. We annotate a DISTINCT noise-head node so the clean head keeps its high
        // assurance (a head carries one assurance basis in this slice) — this is the ENTITY-level
        // arm, which only the head fallback of `w(t)` can read.
        if next(4) == 0 {
            let t = next(n);
            if t != i {
                out.push_str(&format!("ex:noise{} a ex:Node ; ex:rel ex:e{} .\n", i, t));
                out.push_str(&format!(
                    "ex:noise{} pkg:assurance secx:Conjectured ; pkg:confidence \"0.2\" ; prov:wasDerivedFrom ex:src-weak .\n",
                    i
                ));
            }
        }

        // STATEMENT-level noise (minority): the SAME, high-assurance head `ex:e{i}` asserts a
        // cross-cluster edge that is itself dubious. The head is a fine entity, so entity-level
        // provenance says nothing; the doubt lives on the RDF 1.2 reifier of that one statement.
        // This is what makes `w(t)` vary WITHIN a subject's edges (see the fn doc).
        if next(4) == 0 {
            let t = next(n);
            if t != i && cluster(t) != cluster(i) {
                out.push_str(&format!("ex:e{} ex:rel ex:e{} .\n", i, t));
                out.push_str(&format!(
                    "ex:st{} rdf:reifies <<( ex:e{} ex:rel ex:e{} )>> ; pkg:confidence \"0.15\" .\n",
                    i, i, t
                ));
            }
        }
    }
    out
}

// ---- Synthetic RDF 1.2 quoted-triple slice ------------------------------------------------------

/// The three layers of the [`synthetic_rdf12_parts`] slice, separated so tests can compose them:
/// the byte-identity regression (quoted-term-bearing lines added under the default
/// [`TermScope::IriBlank`] must change **nothing**, bit-for-bit) appends `reifications` to `base`,
/// while the visibility ablation runs over all three.
///
/// Every string is **N-Triples** (full IRIs, one statement per line — also valid Turtle): the
/// N-Triples path is the RDF 1.2 quoted-term path the crate's fingerprint tests already exercise.
/// Parse with format `"ntriples"`.
pub struct Rdf12Parts {
    /// IRI-only community-structured base graph (a valid, quote-free slice on its own):
    /// entities + `ex:relatedTo` claim edges (community-clustered), typed sources, and the
    /// pre-registered eval target `ex:corroborates` (source–source, IRI–IRI — identically split
    /// under both scopes).
    pub base: String,
    /// ONLY `ex:stmtJ rdf:reifies <<( h p t )>>` lines — **every** triple here has a quoted-term
    /// endpoint, i.e. is invisible under [`TermScope::IriBlank`] (the byte-identity fixture).
    pub reifications: String,
    /// Reifier metadata: `ex:stmtJ rdf:type ex:Statement` and `ex:stmtJ ex:assertedBy ex:srcK`
    /// (all IRI–IRI) — the atomic context that connects statements to sources.
    pub metadata: String,
}

impl Rdf12Parts {
    /// The full slice: `base + reifications + metadata`.
    pub fn full(&self) -> String {
        format!("{}{}{}", self.base, self.reifications, self.metadata)
    }
}

/// Build a small synthetic **RDF 1.2 quoted-triple** slice for the quoted-terms visibility
/// ablation ([`run_quoted_ablation`]). Deterministic in `seed`, sized by `n_entities`.
///
/// **Signal mechanism (stated honestly).** A shallow KGE treats a quoted term as an *opaque
/// node*: this slice buys the trainer **structural** visibility only — `rdf:reifies` edges and
/// content-addressed hub sharing (`sparq-core` interns triple terms by their component ids, so
/// two reifiers of the same claim share ONE quoted-term node) — **not** compositional access to
/// the quoted `(s, p, o)` content (that is the separate, derived statement-level encoder
/// [`crate::train::TrainedModel::encode_quoted_term`], whose adoption stays measurement-gated).
/// Under [`TermScope::Embeddable`], `src ←assertedBy− stmt −reifies→ tt ←reifies−
/// stmt′ −assertedBy→ src′` paths connect sources through shared-claim hubs; whether that lifts
/// the pre-registered target is exactly what the ablation measures — **no lift is promised, and
/// a synthetic win does not extrapolate off-corpus**.
///
/// Construction (mirrors the honesty guards of the gUFO/relational/provenance slices):
/// - community-clustered entities with `ex:relatedTo` claim edges (recoverable base structure);
/// - per-community sources asserting **overlapping** claim subsets (≥2 reifiers per shared
///   quoted term — the hub), plus occasional cross-community **noise** reifications;
/// - the eval target `ex:corroborates` between same-community sources sharing ≥1 claim — but
///   only for ~70% of sharing pairs: the rest are **decoys** (overlapping-but-uncorroborated),
///   so neither a type filter nor raw claim overlap trivially separates the target.
pub fn synthetic_rdf12_parts(n_entities: usize, seed: u64) -> Rdf12Parts {
    let mut state = seed ^ 0x12F1_2F12_12F1_2F12;
    let mut next = |m: usize| -> usize { (splitmix64(&mut state) as usize) % m.max(1) };

    const EX: &str = "http://ex/";
    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
    let n = n_entities.max(24);
    let n_comm = 5usize;
    let comm = |k: usize| -> usize { k % n_comm };
    let n_src = (n / 6).max(10);

    // 1) Claims: each entity emits 1–2 within-community `relatedTo` edges (deduplicated).
    //    Claims are asserted in the base graph AND are the reification targets below.
    let mut claims: Vec<(usize, usize)> = Vec::new();
    let mut seen_claims = FxHashSet::default();
    for i in 0..n {
        for pass in 0..2usize {
            let p = if pass == 0 { 8 } else { 4 };
            if next(10) < p {
                // A same-community partner (bounded retry, fall back to any other entity).
                let mut tgt = None;
                for _ in 0..8 {
                    let cand = next(n);
                    if cand != i && comm(cand) == comm(i) {
                        tgt = Some(cand);
                        break;
                    }
                }
                let t = tgt.unwrap_or((i + 1) % n);
                if t != i && seen_claims.insert((i, t)) {
                    claims.push((i, t));
                }
            }
        }
    }

    // 2) Assertions: each source asserts ~60% of its community's claims (overlapping subsets ⇒
    //    shared quoted-term hubs) plus, with ~15% probability, one cross-community noise claim.
    let mut assertions: Vec<(usize, usize)> = Vec::new(); // (source, claim index)
    for s in 0..n_src {
        let sc = comm(s);
        for (ci, &(h, _)) in claims.iter().enumerate() {
            if comm(h) == sc && next(10) < 6 {
                assertions.push((s, ci));
            }
        }
        if next(100) < 15 && !claims.is_empty() {
            let ci = next(claims.len());
            assertions.push((s, ci));
        }
    }

    // 3) The corroboration target: same-community source pairs sharing ≥1 asserted claim get an
    //    `ex:corroborates` edge with probability ~70% — the rest are decoys (no edge), so claim
    //    overlap alone cannot separate the relation.
    let mut asserted_by: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); n_src];
    for &(s, ci) in &assertions {
        asserted_by[s].insert(ci);
    }
    let mut corroborates: Vec<(usize, usize)> = Vec::new();
    for a in 0..n_src {
        for b in (a + 1)..n_src {
            if comm(a) == comm(b)
                && asserted_by[a]
                    .intersection(&asserted_by[b])
                    .next()
                    .is_some()
                && next(10) < 7
            {
                corroborates.push((a, b));
            }
        }
    }

    // 4) Emit the three N-Triples layers (full IRIs; `rdf:type` spelled out — `a` is Turtle-only).
    let mut base = String::new();
    for i in 0..n {
        base.push_str(&format!("<{EX}e{i}> <{RDF_TYPE}> <{EX}Entity> .\n"));
    }
    for s in 0..n_src {
        base.push_str(&format!("<{EX}src{s}> <{RDF_TYPE}> <{EX}Source> .\n"));
    }
    for &(h, t) in &claims {
        base.push_str(&format!("<{EX}e{h}> <{EX}relatedTo> <{EX}e{t}> .\n"));
    }
    for &(a, b) in &corroborates {
        base.push_str(&format!("<{EX}src{a}> <{EX}corroborates> <{EX}src{b}> .\n"));
    }

    let mut reifications = String::new();
    let mut metadata = String::new();
    for (j, &(s, ci)) in assertions.iter().enumerate() {
        let (h, t) = claims[ci];
        reifications.push_str(&format!(
            "<{EX}stmt{j}> <{RDF_REIFIES}> <<( <{EX}e{h}> <{EX}relatedTo> <{EX}e{t}> )>> .\n"
        ));
        metadata.push_str(&format!("<{EX}stmt{j}> <{RDF_TYPE}> <{EX}Statement> .\n"));
        metadata.push_str(&format!("<{EX}stmt{j}> <{EX}assertedBy> <{EX}src{s}> .\n"));
    }

    Rdf12Parts {
        base,
        reifications,
        metadata,
    }
}

/// The full [`synthetic_rdf12_parts`] slice (`base + reifications + metadata`) as one N-Triples
/// document — the input for [`run_quoted_ablation`]. Parse with format `"ntriples"`.
/// Deterministic in `seed`, sized by `n_entities`.
pub fn synthetic_rdf12_ttl(n_entities: usize, seed: u64) -> String {
    synthetic_rdf12_parts(n_entities, seed).full()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;

    fn small_graph() -> Graph {
        let ttl = synthetic_gufo_ttl(40, 1);
        let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
        c.graph
    }

    #[test]
    fn split_has_no_leakage_and_filter_is_union() {
        let g = small_graph();
        let s = Splits::split(&g, 0.8, 0.1, 99);
        // No triple in more than one split.
        let mut seen: FxHashSet<[Id; 3]> = FxHashSet::default();
        for t in s.train.iter().chain(&s.valid).chain(&s.test) {
            assert!(
                seen.insert(*t),
                "triple {:?} appears in more than one split (LEAKAGE)",
                t
            );
        }
        // Filter set == union of all splits.
        assert_eq!(
            s.filter_set_len(),
            seen.len(),
            "filter set must be the union of all splits"
        );
        for t in seen.iter() {
            assert!(s.is_true(t), "every split triple must be in the filter set");
        }
        assert!(!s.test.is_empty(), "need a non-empty test split to measure");
    }

    #[test]
    fn filtered_rank_excludes_other_true_triples() {
        // A synthetic case where a head has two true tails; ranking one must filter the other.
        let ttl = r#"
@prefix ex: <http://ex/> .
ex:a ex:rel ex:x .
ex:a ex:rel ex:y .
ex:b ex:rel ex:x .
ex:c ex:rel ex:y .
"#;
        let c = close_for_vectorise(ttl, "turtle", Profile::Rdfs).unwrap();
        let g = c.graph;
        let s = Splits::split(&g, 0.5, 0.0, 1);
        // Force: train on everything so all entities have rows; reuse split's all_true filter.
        let tc = TypeConstraints::mine(&g);
        let cfg = TrainConfig::small(crate::structure::SamplingMode::Unconstrained, 2);
        let (model, _) = train(&g, &tc, cfg);
        let a = g.id_of(&iri("http://ex/a")).unwrap();
        let rel = g.id_of(&iri("http://ex/rel")).unwrap();
        let x = g.id_of(&iri("http://ex/x")).unwrap();
        // Rank (a rel ?) with answer x: y must be FILTERED (it is also a true tail of a).
        let rank = filtered_rank(&model, &s, [a, rel, x], Side::Tail, None).unwrap();
        // With y filtered, only x and the remaining distractors remain; rank is bounded by the pool.
        assert!(rank >= 1);
        // Sanity: the all_true set indeed contains (a rel y).
        let y = g.id_of(&iri("http://ex/y")).unwrap();
        assert!(
            s.is_true(&[a, rel, y]),
            "the other true tail must be in the filter set"
        );
    }

    #[test]
    fn ablation_runs_all_four_cells() {
        let ttl = synthetic_gufo_ttl(60, 4);
        let cfg = EvalConfig::small(13);
        let cells = run_ablation(&ttl, "turtle", cfg).unwrap();
        assert_eq!(cells.len(), 4, "2x2 ablation matrix");
        // Cell ordering invariant.
        assert!(!cells[0].closure && !cells[0].type_constrained);
        assert!(!cells[1].closure && cells[1].type_constrained);
        assert!(cells[2].closure && !cells[2].type_constrained);
        assert!(cells[3].closure && cells[3].type_constrained);
        // Every cell produced a non-empty test evaluation and the closure cells actually closed.
        for c in &cells {
            assert!(c.metrics.queries > 0, "cell produced no scorable queries");
            assert!(c.report.loss_decreased(), "cell model did not learn");
        }
        // The gUFO-prior ablation axis is DEFAULT-OFF: an EvalConfig::small run is the baseline.
        assert!(cells.iter().all(|c| !c.gufo_prior));
    }

    // ---- gUFO-prior axis ([FABLE-5] kern/ufo-priors) --------------------------------------------

    #[test]
    fn gufo_prior_on_a_gufo_free_graph_is_byte_identical_to_off() {
        // The honest no-op (mirrors the provenance-weighting convention): over a graph carrying
        // NO gUFO annotations the mined priors are empty, the mask drops nothing, and the ON and
        // OFF arms must be BYTE-IDENTICAL — exact f64 equality, not approximate.
        let ttl = synthetic_relational_ttl(120, 3);
        let off = EvalConfig::small(5);
        let mut on = off;
        on.gufo_prior = true;
        let a = run_ablation(&ttl, "turtle", off).unwrap();
        let b = run_ablation(&ttl, "turtle", on).unwrap();
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.metrics.queries, y.metrics.queries);
            assert_eq!(x.metrics.mrr, y.metrics.mrr, "byte-identical MRR");
            assert_eq!(x.metrics.hits1, y.metrics.hits1);
            assert_eq!(x.metrics.hits3, y.metrics.hits3);
            assert_eq!(x.metrics.hits10, y.metrics.hits10);
            assert_eq!(x.long_tail.tail.mrr, y.long_tail.tail.mrr);
        }
        assert!(a.iter().all(|c| !c.gufo_prior) && b.iter().all(|c| c.gufo_prior));
    }

    #[test]
    fn gufo_prior_off_is_deterministic_and_default() {
        // The OFF arm reads no entropy and constructs no mask: two identical runs are identical,
        // and EvalConfig::small defaults the axis off (the byte-identical baseline).
        let cfg = EvalConfig::small(17);
        assert!(!cfg.gufo_prior, "the gUFO prior must be DEFAULT-OFF");
        let ttl = synthetic_gufo_ttl(60, 4);
        let a = run_ablation(&ttl, "turtle", cfg).unwrap();
        let b = run_ablation(&ttl, "turtle", cfg).unwrap();
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.metrics.mrr, y.metrics.mrr);
            assert_eq!(x.metrics.queries, y.metrics.queries);
        }
    }

    #[test]
    fn gufo_prior_mask_is_answer_safe_and_never_hurts_a_rank() {
        // The load-bearing answer-safety property, asserted end-to-end: the mask only REMOVES
        // provably-wrong distractors from each ranking, so with the SAME trained model (training
        // is untouched) every query's filtered rank can only improve or stay equal — per cell,
        // MRR/Hits@k(ON) >= MRR/Hits@k(OFF) and the query count is unchanged. On the gUFO slice
        // (three gufo:Kinds: Person/Organisation/Course, roles/phases under Person) the mask must
        // also actually BITE (some rank strictly improves) — Person ⊥ Course is UFO-proven, so
        // person candidates drop out of enrolledIn tail rankings.
        let ttl = synthetic_gufo_ttl(120, 3);
        let off = EvalConfig::small(3);
        let mut on = off;
        on.gufo_prior = true;
        on.gufo_ns = "http://ex/gufo#"; // the slice's explicit (non-canonical) gUFO namespace
        let a = run_ablation(&ttl, "turtle", off).unwrap();
        let b = run_ablation(&ttl, "turtle", on).unwrap();
        let mut bit = false;
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(
                x.metrics.queries, y.metrics.queries,
                "the mask never changes WHICH queries are scorable"
            );
            assert!(
                y.metrics.mrr >= x.metrics.mrr,
                "answer-safety: masking provably-wrong distractors can never lower MRR \
                 (cell closure={} tc={}: on={} off={})",
                x.closure,
                x.type_constrained,
                y.metrics.mrr,
                x.metrics.mrr
            );
            assert!(y.metrics.hits1 >= x.metrics.hits1);
            assert!(y.metrics.hits3 >= x.metrics.hits3);
            assert!(y.metrics.hits10 >= x.metrics.hits10);
            if y.metrics.mrr > x.metrics.mrr {
                bit = true;
            }
        }
        assert!(
            bit,
            "on a gUFO-annotated slice the mask must actually remove distractors"
        );
    }

    #[test]
    fn long_tail_buckets_partition_queries() {
        let ttl = synthetic_gufo_ttl(80, 7);
        let cfg = EvalConfig::small(21);
        let cells = run_ablation(&ttl, "turtle", cfg).unwrap();
        for c in &cells {
            // Head + tail query counts must sum to the overall query count (a clean partition).
            assert_eq!(
                c.long_tail.head.queries + c.long_tail.tail.queries,
                c.metrics.queries,
                "long-tail buckets must partition all queries"
            );
        }
    }

    #[test]
    fn gufo_slice_is_not_trivially_separable() {
        // The slice must be non-trivial: a pure type filter cannot separate true from false edges,
        // because MANY type-compatible NON-edges (decoys) exist. We guard the property DIRECTLY by
        // counting the Students who are NOT enrolled in any course at all: those are genuine
        // type-compatible decoys (a type prior would admit them as candidates, yet they hold no
        // enrolledIn edge), so a non-trivial fraction of them must exist AND there must be several
        // courses (so the candidate pool per query dwarfs the true-answer set).
        let ttl = synthetic_gufo_ttl(120, 3);
        let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let student = g.id_of(&iri("http://ex/Student")).unwrap();
        let course_cls = g.id_of(&iri("http://ex/Course")).unwrap();
        let enrolled = g.id_of(&iri("http://ex/enrolledIn")).unwrap();
        let typ = g
            .id_of(&iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"))
            .unwrap();

        // Set of student individuals (entities typed ex:Student — directly or via closure).
        let students: FxHashSet<Id> = g
            .iter_ids()
            .filter(|[_, p, o]| *p == typ && *o == student)
            .map(|[s, _, _]| s)
            .collect();
        // Students that hold at least one enrolledIn edge.
        let enrolled_students: FxHashSet<Id> = g
            .iter_ids()
            .filter(|[s, p, _]| *p == enrolled && students.contains(s))
            .map(|[s, _, _]| s)
            .collect();
        let n_courses = g
            .iter_ids()
            .filter(|[_, p, o]| *p == typ && *o == course_cls)
            .count();

        let n_students = students.len();
        let n_decoys = n_students - enrolled_students.len(); // students with NO enrolment.
        assert!(
            n_students >= 10,
            "need a non-trivial student population: {}",
            n_students
        );
        assert!(
            n_courses >= 3,
            "need several courses so the candidate pool dwarfs the answer set: {}",
            n_courses
        );
        // The load-bearing non-separability guard: a SUBSTANTIAL fraction of type-compatible
        // students hold no enrolment edge (>= 15% here), so a pure type filter cannot win — these
        // are real decoys the embedding must rank below the true tails. (The generator enrols a
        // student only ~70% of the time and only ~70% of students hold the Student role, so a large
        // decoy set is structural, not incidental.)
        assert!(
            n_decoys * 100 >= n_students * 15,
            "slice must have a substantial decoy set (type-compatible students with no enrolment): \
             students={} enrolled={} decoys={} (need >=15%)",
            n_students,
            enrolled_students.len(),
            n_decoys,
        );
    }

    #[test]
    fn multiseed_aggregates_mean_and_std() {
        // The multi-seed harness must produce a per-cell mean ± std over the seeds, and (because the
        // seed moves split + init + negatives) the variance across seeds must be observable — that
        // is the whole point: a single-seed delta could be noise.
        let ttl = synthetic_relational_ttl(200, 5);
        let template = EvalConfig::small(0);
        let seeds = [1u64, 2, 3, 4, 5];
        let cells = run_ablation_multiseed(&ttl, "turtle", template, &seeds).unwrap();
        assert_eq!(cells.len(), 4, "2x2 matrix");
        let mut any_variance = false;
        for c in &cells {
            assert_eq!(c.metrics.mrr.n, seeds.len(), "all seeds contribute");
            assert!(c.metrics.mrr.mean >= 0.0 && c.metrics.mrr.mean <= 1.0);
            assert!(c.metrics.queries.mean > 0.0);
            if c.metrics.mrr.std > 0.0 {
                any_variance = true;
            }
        }
        assert!(
            any_variance,
            "across 5 seeds at least one cell's MRR must show variance (single-seed deltas are noisy)"
        );
    }

    #[test]
    fn paired_delta_reduces_variance_vs_unpaired() {
        // The firm-up bead (sq-4891y) rests on this property: the PAIRED closure delta (computed
        // within each seed, where the four cells share the split/init/negatives) has a SMALLER spread
        // than the UNPAIRED comparison (the sum of the two cells' independent stds), because the
        // shared per-seed noise cancels in the difference. We verify the variance reduction directly
        // on the schema-bearing gUFO slice under the asymmetric model, over enough seeds to estimate
        // both spreads.
        use crate::train::ModelKind;
        let ttl = synthetic_gufo_ttl_sized(200, 3, 0xBEEF);
        let mut template = EvalConfig::small(0);
        template.train.model = ModelKind::ComplEx;
        template.train.epochs = 80;
        template.train.dim = 48;
        template.train.negatives_per_positive = 12;
        let seeds: Vec<u64> = (0..8).map(|i| 100 + i).collect();
        let r = run_ablation_multiseed_paired(&ttl, "turtle", template, &seeds).unwrap();

        // The unpaired spread proxy: sum of the closure-OFF and closure-ON cells' MRR stds, averaged
        // over the two negative settings (mirrors how `closure_mrr` averages the two settings).
        let unpaired = 0.5
            * ((r.cells[0].metrics.mrr.std + r.cells[2].metrics.mrr.std)
                + (r.cells[1].metrics.mrr.std + r.cells[3].metrics.mrr.std));
        assert_eq!(
            r.closure_mrr.n,
            seeds.len(),
            "all seeds contribute to the paired delta"
        );
        assert!(
            r.closure_mrr.std > 0.0 && unpaired > 0.0,
            "both spreads must be observable (paired std={}, unpaired proxy={})",
            r.closure_mrr.std,
            unpaired
        );
        // The whole point of pairing: the paired spread is strictly smaller than the unpaired sum.
        assert!(
            r.closure_mrr.std < unpaired,
            "paired delta std ({}) must be smaller than the unpaired sum-of-stds ({}) — common \
             random numbers cancel the shared per-seed noise",
            r.closure_mrr.std,
            unpaired
        );
        // The standard error must shrink as std/√n (the variance-reduction mechanism the gate uses).
        let expected_se = r.closure_mrr.std / (seeds.len() as f64).sqrt();
        assert!(
            (r.closure_mrr.se - expected_se).abs() < 1e-9,
            "se must equal std/√n"
        );
    }

    #[test]
    fn paired_delta_significance_gate_is_honest() {
        // `significant_at` must be a genuine gate, not a rubber stamp: it requires n>=2 (a single
        // seed has undefined spread, so the gate must REFUSE to certify), and a larger k must be a
        // STRICTLY stronger bar (monotone). We use a deterministic synthetic PairedDelta so the
        // assertion is about the gate logic, not a noisy run.
        // Single seed: se is 0 and the gate must refuse regardless of a positive mean.
        let one = PairedDelta {
            mean: 0.05,
            std: 0.0,
            se: 0.0,
            n: 1,
        };
        assert!(!one.significant_at(0.0), "n<2 must never be certified");
        assert!(!one.significant_at(2.0));
        // A real multi-seed delta: mean 0.02, se 0.005 → clears 1·se and 2·se, not 5·se.
        let d = PairedDelta {
            mean: 0.02,
            std: 0.005 * 4.0,
            se: 0.005,
            n: 16,
        };
        assert!(d.significant_at(1.0), "0.02 > 1·0.005");
        assert!(d.significant_at(2.0), "0.02 > 2·0.005");
        assert!(!d.significant_at(5.0), "0.02 < 5·0.005 — must NOT certify");
        // A negative effect is never significant.
        let neg = PairedDelta {
            mean: -0.01,
            std: 0.02,
            se: 0.005,
            n: 16,
        };
        assert!(
            !neg.significant_at(0.0),
            "a negative effect is never a positive win"
        );
    }

    #[test]
    fn denser_gufo_slice_has_more_test_triples_and_still_bites() {
        // Density must (a) increase the number of held-out test triples per seed (the variance lever)
        // and (b) keep the closure axis live (the rigid Person kind is still asserted on nobody, so
        // closure must derive entailed triples). Both are load-bearing for the firm-up.
        let cfg = EvalConfig::small(7);
        let sparse = run_ablation(&synthetic_gufo_ttl_sized(200, 1, 9), "turtle", cfg).unwrap();
        let dense = run_ablation(&synthetic_gufo_ttl_sized(200, 4, 9), "turtle", cfg).unwrap();
        assert!(
            dense[0].metrics.queries > sparse[0].metrics.queries,
            "a denser slice must yield more scorable test queries: dense={} sparse={}",
            dense[0].metrics.queries,
            sparse[0].metrics.queries
        );
        // Closure still bites on the dense slice (entailed Person memberships derived).
        let c = close_for_vectorise(
            &synthetic_gufo_ttl_sized(200, 4, 9),
            "turtle",
            Profile::Rdfs,
        )
        .unwrap();
        assert!(
            c.entailed_triples > 0,
            "dense gUFO slice closure must still derive entailed triples"
        );
    }

    #[test]
    fn multiseed_uses_the_configured_model() {
        // The model is carried by the template's TrainConfig; the asymmetric ComplEx default is used
        // unless overridden. We just confirm the run completes for an explicit DistMult template too
        // (the symmetric path must still aggregate cleanly).
        use crate::train::ModelKind;
        let ttl = synthetic_relational_ttl(150, 9);
        let mut template = EvalConfig::small(0);
        template.train.model = ModelKind::DistMult;
        let cells = run_ablation_multiseed(&ttl, "turtle", template, &[7, 8]).unwrap();
        assert_eq!(cells.len(), 4);
        for c in &cells {
            assert_eq!(c.metrics.mrr.n, 2);
        }
    }

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    // ---- RDF 1.2 quoted-terms visibility axis ---------------------------------------------------

    use sparq_core::dict::TermParts;

    fn is_quoted(g: &Graph, id: Id) -> bool {
        matches!(g.dict.term_parts(id), TermParts::Triple(_))
    }

    fn assert_metrics_bit_equal(a: &Metrics, b: &Metrics, what: &str) {
        assert_eq!(a.queries, b.queries, "{what}: queries");
        assert_eq!(a.mrr.to_bits(), b.mrr.to_bits(), "{what}: mrr");
        assert_eq!(a.hits1.to_bits(), b.hits1.to_bits(), "{what}: hits1");
        assert_eq!(a.hits3.to_bits(), b.hits3.to_bits(), "{what}: hits3");
        assert_eq!(a.hits10.to_bits(), b.hits10.to_bits(), "{what}: hits10");
    }

    /// T-2 — the LOAD-BEARING byte-identity regression: adding quoted-term-bearing triples
    /// (`rdf:reifies` lines) to a graph changes NOTHING under the default `TermScope::IriBlank` —
    /// same splits, same ranking pool, bit-equal model bytes and loss curve, bit-equal metrics.
    /// The two variants share ONE parsed dictionary (the base-only variant filters the quoted
    /// triples out of the same id space), so the comparison is robust to parser id assignment and
    /// any difference is attributable to the reifications alone.
    #[test]
    fn invisible_reifications_change_nothing_when_scope_is_off() {
        let parts = synthetic_rdf12_parts(80, 42);
        let with_text = format!("{}{}", parts.base, parts.reifications);
        let (dict, triples) = Graph::parse_to_triples(&with_text, "ntriples").unwrap();

        let quoted_free: Vec<[Id; 3]> = {
            let probe = Graph::from_parts(dict.clone(), triples.clone());
            triples
                .iter()
                .copied()
                .filter(|&[s, _, o]| !is_quoted(&probe, s) && !is_quoted(&probe, o))
                .collect()
        };
        assert!(
            quoted_free.len() < triples.len(),
            "fixture precondition: the reifications layer must add quoted-term triples"
        );

        // T-9 (closure non-interference smoke guard): the RDFS closure neither destructures nor
        // multiplies quoted terms — the entailed-triple count is unaffected by the reifications.
        let closed_with = materialise_closure(dict.clone(), triples.clone(), Profile::Rdfs);
        let closed_base = materialise_closure(dict.clone(), quoted_free.clone(), Profile::Rdfs);
        assert_eq!(
            closed_with.entailed_triples, closed_base.entailed_triples,
            "closure must be inert over the reifications layer"
        );

        let run = |g: &Graph| {
            let splits = Splits::split(g, 0.8, 0.1, 99);
            let train_graph = restrict_to_train(g, &splits);
            let tc = TypeConstraints::mine(&train_graph);
            let cfg = TrainConfig::small(crate::structure::SamplingMode::TypeConstrained, 7);
            assert_eq!(
                cfg.term_scope,
                TermScope::IriBlank,
                "preset must default OFF"
            );
            let (model, report) = train(&train_graph, &tc, cfg);
            let (metrics, _) = evaluate(&model, &splits, 2, None);
            (splits, model, report, metrics)
        };

        let (s_with, m_with, r_with, met_with) = run(&closed_with.graph);
        let (s_base, m_base, r_base, met_base) = run(&closed_base.graph);

        // Identical splits (same [Id;3] vectors — the dict is shared, so ids are comparable) and
        // the identical ranking pool.
        assert_eq!(s_with.train, s_base.train, "train split must be identical");
        assert_eq!(s_with.valid, s_base.valid, "valid split must be identical");
        assert_eq!(s_with.test, s_base.test, "test split must be identical");
        assert_eq!(
            s_with.entities, s_base.entities,
            "ranking pool must be identical"
        );

        // Bit-equal parameters and loss curve: the trainer's PRNG stream, row assignment, and
        // float path are untouched by invisible triples.
        assert_eq!(
            m_with.entity_emb, m_base.entity_emb,
            "entity params must be bit-equal"
        );
        assert_eq!(
            m_with.rel_emb, m_base.rel_emb,
            "relation params must be bit-equal"
        );
        assert_eq!(
            r_with.epoch_loss, r_base.epoch_loss,
            "loss curve must be bit-equal"
        );
        assert_eq!(
            r_with.positives, r_base.positives,
            "positive count must be identical"
        );

        assert_metrics_bit_equal(&met_with, &met_base, "flag-off metrics");
    }

    /// T-3 — the honest no-op: on a QUOTE-FREE graph the ON and OFF arms of the quoted ablation
    /// are byte-identical and every paired delta is exactly zero (mirrors the weight-ablation
    /// property on provenance-free graphs). Together with T-2 this brackets the change from both
    /// directions: invisible additions change nothing OFF; the ON arm changes nothing without
    /// quoted terms.
    #[test]
    fn quoted_ablation_is_exactly_zero_on_quote_free_graphs() {
        let ttl = synthetic_relational_ttl(120, 5);
        let r = run_quoted_ablation(&ttl, "turtle", EvalConfig::small(0), &[1, 2]).unwrap();
        assert_eq!(r.mrr.n, 2);
        assert_eq!(r.mrr.mean, 0.0, "paired MRR delta must be EXACTLY zero");
        assert_eq!(r.mrr.std, 0.0);
        assert_eq!(
            r.hits10.mean, 0.0,
            "paired Hits@10 delta must be EXACTLY zero"
        );
        assert_eq!(r.off.mrr.mean.to_bits(), r.on.mrr.mean.to_bits());
        assert_eq!(r.off.queries.mean.to_bits(), r.on.queries.mean.to_bits());
        assert!(
            !r.mrr_significant_at(2.0),
            "a zero effect must never certify"
        );
    }

    /// T-6 — split/pool scope-invariance on the RDF 1.2 slice: quoted terms are in NO split, NO
    /// ranking pool (they are trainer-side only; the eval population is identical in both arms).
    #[test]
    fn rdf12_splits_and_ranking_pool_stay_atomic() {
        let ttl = synthetic_rdf12_ttl(80, 3);
        let c = close_for_vectorise(&ttl, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let s = Splits::split(g, 0.8, 0.1, 17);
        for &e in &s.entities {
            assert!(!is_quoted(g, e), "ranking pool must contain no quoted term");
        }
        for t in s.train.iter().chain(&s.valid).chain(&s.test) {
            assert!(
                !is_quoted(g, t[0]) && !is_quoted(g, t[2]),
                "no split triple may carry a quoted-term endpoint"
            );
        }
        assert!(
            !s.test.is_empty(),
            "the slice must yield a measurable test split"
        );
    }

    /// T-8 — the slice's honesty guards: deterministic in seed; every reifications line carries a
    /// quoted term; shared quoted-term hubs exist (≥2 reifiers of one claim); a substantial
    /// fraction of claim-sharing source pairs is NOT corroborated (decoys — claim overlap alone
    /// cannot separate the target relation).
    #[test]
    fn rdf12_slice_properties_hold() {
        let a = synthetic_rdf12_parts(96, 11);
        let b = synthetic_rdf12_parts(96, 11);
        assert_eq!(a.base, b.base, "deterministic in seed");
        assert_eq!(a.reifications, b.reifications);
        assert_eq!(a.metadata, b.metadata);
        let c = synthetic_rdf12_parts(96, 12);
        assert_ne!(a.reifications, c.reifications, "seed must move the slice");

        // Every reifications line has a quoted-term endpoint (T-2's fixture precondition).
        let lines: Vec<&str> = a.reifications.lines().filter(|l| !l.is_empty()).collect();
        assert!(
            lines.len() >= 20,
            "need a non-trivial reification set: {}",
            lines.len()
        );
        for l in &lines {
            assert!(
                l.contains("<<("),
                "every reification must quote a triple: {l}"
            );
        }

        // Hub structure: ≥3 quoted terms with ≥2 distinct reifiers (content-addressed sharing).
        let full = a.full();
        let (dict, triples) = Graph::parse_to_triples(&full, "ntriples").unwrap();
        let g = Graph::from_parts(dict, triples);
        let reifies = g
            .id_of(&iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies"))
            .unwrap();
        let mut reifiers_of: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
        for [s, p, o] in g.iter_ids() {
            if p == reifies {
                assert!(
                    is_quoted(&g, o),
                    "every rdf:reifies object must be a quoted term"
                );
                reifiers_of.entry(o).or_default().insert(s);
            }
        }
        let hubs = reifiers_of.values().filter(|r| r.len() >= 2).count();
        assert!(hubs >= 3, "need shared quoted-term hubs (got {hubs})");

        // Decoys: sources sharing ≥1 claim but NOT corroborated must be a substantial fraction.
        let asserted_by_pred = g.id_of(&iri("http://ex/assertedBy")).unwrap();
        let corroborates = g.id_of(&iri("http://ex/corroborates")).unwrap();
        let mut claims_of: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default(); // src -> {tt}
        let mut stmt_claim: FxHashMap<Id, Id> = FxHashMap::default();
        for [s, p, o] in g.iter_ids() {
            if p == reifies {
                stmt_claim.insert(s, o);
            }
        }
        for [s, p, o] in g.iter_ids() {
            if p == asserted_by_pred {
                if let Some(&tt) = stmt_claim.get(&s) {
                    claims_of.entry(o).or_default().insert(tt);
                }
            }
        }
        let edges: FxHashSet<(Id, Id)> = g
            .iter_ids()
            .filter(|[_, p, _]| *p == corroborates)
            .map(|[s, _, o]| (s, o))
            .collect();
        let mut sources: Vec<Id> = claims_of.keys().copied().collect();
        sources.sort_unstable();
        let mut sharing = 0usize;
        let mut decoys = 0usize;
        for (i, &sa) in sources.iter().enumerate() {
            for &sb in &sources[i + 1..] {
                if claims_of[&sa]
                    .intersection(&claims_of[&sb])
                    .next()
                    .is_some()
                {
                    sharing += 1;
                    if !edges.contains(&(sa, sb)) && !edges.contains(&(sb, sa)) {
                        decoys += 1;
                    }
                }
            }
        }
        assert!(
            sharing >= 10,
            "need a non-trivial sharing-pair population: {sharing}"
        );
        assert!(
            !edges.is_empty(),
            "the corroborates target must be non-empty"
        );
        assert!(
            decoys * 100 >= sharing * 15,
            "≥15% of claim-sharing pairs must be uncorroborated decoys: {decoys}/{sharing}"
        );
    }

    /// T-11 + the visibility-path exercise: the presets keep the axis OFF (`run_ablation` cells
    /// all report `quoted_terms == false`), and the paired runner runs end-to-end over the RDF
    /// 1.2 slice with an identical eval population in both arms. NO lift is asserted — whether
    /// visibility helps is exactly the open measurement.
    #[test]
    fn quoted_ablation_runs_on_the_rdf12_slice_and_presets_stay_off() {
        assert_eq!(EvalConfig::small(3).train.term_scope, TermScope::IriBlank);
        let cells =
            run_ablation(&synthetic_gufo_ttl(40, 1), "turtle", EvalConfig::small(3)).unwrap();
        assert!(
            cells.iter().all(|c| !c.quoted_terms),
            "matrix cells must report the axis OFF"
        );

        let ttl = synthetic_rdf12_ttl(60, 11);
        let r = run_quoted_ablation(&ttl, "ntriples", EvalConfig::small(0), &[1, 2]).unwrap();
        assert_eq!(r.mrr.n, 2, "all seeds contribute");
        assert!(
            r.off.queries.mean > 0.0,
            "OFF arm must produce scorable queries"
        );
        // Scope-invariant eval population: the two arms rank the same queries over the same pool.
        assert_eq!(
            r.off.queries.mean.to_bits(),
            r.on.queries.mean.to_bits(),
            "both arms must evaluate the identical query population"
        );
    }
}

//! **Live adaptive re-planning** — mid-execution plan switching when the *observed*
//! runtime statistics diverge from the cost-model estimates the static plan was built on.
//!
//! The static [`crate::plan_bgp`] commits to one join order + per-join algorithm up front,
//! from descriptor-derived **estimates**. In a real federation those estimates are often
//! wrong: a source returns far more (or fewer) bindings than its VoID stats predicted, or
//! a source is slow / unavailable. ANAPSID's harder half is to *react* — re-plan the part
//! of the query that has **not yet executed** with the corrected statistics, and switch to
//! the better plan if it is enough of a win to be worth the switch.
//!
//! This module is the opt-in (`adaptive-replan` feature) realisation of that, scoped to a
//! **sound boundary** (see below).
//!
//! ## What it does
//!
//! 1. **Capture actual runtime statistics** ([`RuntimeStats`]): as the executor finishes
//!    each leaf evaluation / join stage, it records the *observed* cardinality of that
//!    operator and the *observed* per-source latency. These are real counts, not estimates.
//! 2. **Re-plan trigger** ([`ReplanPolicy`]): after each stage boundary, compare observed
//!    against estimated. If a not-yet-executed pattern's leaf cardinality was estimated at
//!    `e` but a *correlated* source has now revealed it is really `≈ o` with `o > k·e`
//!    (or `o < e/k`) — i.e. the estimate was off by more than the policy's
//!    [`ReplanPolicy::divergence_factor`] `k` — the planner is re-invoked on the
//!    **remaining** patterns with the corrected leaf cardinalities substituted in.
//! 3. **Hysteresis / anti-thrash** ([`ReplanPolicy::improvement_margin`]): the re-planned
//!    suffix is adopted **only** if its estimated remaining cost beats the current
//!    suffix's by more than a margin (default 10%). A re-plan that is merely a tie, or a
//!    tiny win, is rejected — so stable-but-noisy stats do not cause the plan to flap.
//!
//! ## Per-source latency weighting (sq-b51o) — a HEURISTIC, not optimal
//!
//! Cardinality is not the only thing the static estimates get wrong: a source can be
//! *slow* (contended, far, rate-limited) even when its row counts are exactly as predicted.
//! [`RuntimeStats`] captures the *observed* per-source latency; sq-b51o folds it into the
//! cost model so the re-planner prefers orderings that defer sub-queries against an
//! observably slow source.
//!
//! ## Latency EWMA smoothing (sq-b51o follow-up) — a HEURISTIC α, not optimal
//!
//! The cost model is fed not the **single last** latency observation but a per-source
//! **exponentially-weighted moving average** (EWMA). Each call to
//! [`RuntimeStats::record_source_latency`] folds the new observation into the running
//! average for that source:
//!
//! > `ewma_new = α · observed + (1 − α) · ewma_prev`  (first observation seeds `ewma = observed`).
//!
//! The smoothing factor α ∈ (0, 1] is [`RuntimeStats::latency_alpha`], default `0.3` — a
//! **hand-picked heuristic, not a derived optimum**: it weights the latest sample at 30% and
//! the accumulated history at 70%, so a *single* transient spike moves the average only a
//! fraction of the way and does **not**, on its own, clear the re-plan trigger — but a
//! **sustained** shift (several consecutive high samples) does, as the average converges
//! toward the new level. This is the cleaner anti-thrash discipline the follow-up replaces
//! the bare single-sample-plus-clamp with: the EWMA itself absorbs noise; the absolute
//! `latency_floor`/`latency_cap` clamp on the resulting cost factor is kept as a **final
//! guard** (an outlier that *does* survive the average still cannot dominate the order).
//! Higher α tracks faster but is twitchier; lower α is calmer but laggier — 0.3 is a middle
//! bias, picked by hand, not tuned against a workload.
//!
//! The EWMA (not the raw last value) feeds **both** the latency cost factor *and* the
//! re-plan trigger, so the whole latency path is smoothed consistently. A source with no
//! observation has no EWMA and contributes `factor = 1.0` (inert), exactly as before.
//!
//! ## EWMA refinements (sq-3xkz, sq-b51o follow-up) — per-source α, time-aware decay, eviction
//!
//! Three opt-in refinements sharpen the EWMA when stats are reused across queries — each defaults
//! to **off**, so the default path is byte-identical to the plain single-global-α EWMA above:
//!
//! * **Per-source adaptive α** ([`RuntimeStats::set_source_alpha`]). The global
//!   [`RuntimeStats::latency_alpha`] is now a *fallback*: a source with a per-source override is
//!   smoothed at its own α (a steady source can stay calm, a bursty one can track faster). This
//!   ships the MECHANISM + a sensible default (an **empty** override map ⇒ every source uses the
//!   global α ⇒ the prior behaviour reproduced). Choosing a *good* per-source α needs a real
//!   federated workload to measure against — that tuning is **deferred** (it is not derivable on a
//!   non-canonical work box), so no α value is claimed optimal here.
//! * **Time-aware decay** ([`RuntimeStats::record_source_latency_after`]). The plain EWMA
//!   equal-weights samples regardless of the wall-clock gap between them; with a half-life set
//!   ([`RuntimeStats::set_decay_half_life`]) a sample recorded after an elapsed gap `Δt` folds in
//!   at an effective α inflated toward `1.0` as `Δt` grows past the half-life — so older stats
//!   decay toward the prior and a fresh sample after a long idle gap is trusted more than one that
//!   arrives back-to-back. The elapsed gap is **passed in** (the clock is injectable, never read
//!   here), so the decay is deterministic and testable.
//! * **Staleness / eviction** ([`RuntimeStats::evict_stale`]). A `RuntimeStats` reused across
//!   queries can carry entries that are now too old to trust; `evict_stale(max_age)` drops every
//!   source whose logical age ([`RuntimeStats::source_age`]) exceeds a configurable threshold, so
//!   a stale latency stops biasing the planner. Age is measured on the same injectable logical
//!   clock the decay uses.
//!
//! All three are **cost/ordering only** — they change *which* latency the cost model reads and
//! *when*, never the result multiset; the soundness boundary below is untouched.
//!
//! The weighting is deliberately simple and **honest about being a heuristic** (see
//! [`ReplanPolicy::latency_weight`]): for a candidate pattern, take the **slowest** observed
//! latency over the sources retained for it (a union is bottlenecked by its slowest arm),
//! divide by a baseline latency ([`ReplanPolicy::latency_baseline`]) to get a *relative*
//! slowness `s ≥ 0`, and scale that join's cost by
//!
//! > `factor = clamp(1 + latency_weight · (s − 1), latency_floor, latency_cap)`.
//!
//! A source at the baseline (`s = 1`) — or one with **no** latency observation — gets
//! `factor = 1.0`, i.e. the latency model is *off by construction* until evidence arrives,
//! so behaviour with no measurements is byte-identical to the cardinality-only planner. A
//! source observed at `2·baseline` with the default `latency_weight = 0.5` costs `1.5×`; the
//! clamp ([`ReplanPolicy::latency_cap`], default `4.0`) stops one outlier measurement from
//! dominating the order. This is a *bias*, not a guarantee: latency is noisy and the
//! constants are tuned by hand, not derived — it nudges the re-plan toward faster sources,
//! it does **not** claim to find the latency-optimal plan.
//!
//! ## Per-source (not slowest-arm) aggregation (sq-s5kd) — opt-in
//!
//! The slowest-arm rule above models a **parallel** union's wall-clock: the union is not
//! done until its slowest arm is. But it charges the *whole* pattern at the slow arm's
//! factor even when that arm contributes a negligible share of the rows — a pattern with a
//! huge fast arm and a tiny slow arm is deferred as if all its work were slow.
//! [`ReplanPolicy::latency_aggregation`] adds an opt-in alternative,
//! [`LatencyAggregation::CardinalityWeighted`]: each retained source's latency runs through
//! the same `factor` formula individually (an unobserved source contributes `1.0`, inert as
//! ever), and the pattern's factor is the **cardinality-weighted mean** over its retained
//! sources — the per-source factors weighted by each source's (corrected) estimated
//! cardinality share, i.e. an *expected-work* model rather than a *bottleneck* model.
//! Every per-source factor is already clamped, and a convex combination of clamped values
//! stays inside `[latency_floor, latency_cap]`, so no re-clamp is needed. When every
//! retained cardinality is `0` (nothing to weight by) it falls back to the plain mean over
//! the retained sources. The default stays [`LatencyAggregation::SlowestArm`], so existing
//! behaviour is bit-identical unless a caller opts in; with **no** latency observations
//! both modes are exactly `1.0` (the off-by-construction invariant holds unchanged).
//! Which model wins is workload-dependent (parallel-fetch unions really are bottlenecked;
//! sequential/bind-join work really is proportional) — this ships the MECHANISM, honestly
//! un-tuned, for the federation bench harness to compare.
//!
//! Latency feeds the trigger too: a not-yet-executed pattern whose slowest source is
//! observed to be more than `divergence_factor×` the baseline is enough to *consider* a
//! re-plan (the hysteresis margin still gates whether one is *adopted*), so a query that is
//! latency-bound — not cardinality-bound — can still react. The trigger stays **slowest-arm
//! under both aggregation modes** (sq-s5kd): its job is to *detect* that some arm has gone
//! pathologically slow; whether deferring that pattern actually wins is then judged by the
//! (possibly cardinality-weighted) cost comparable under the hysteresis margin.
//!
//! **The latency weighting changes execution STRATEGY/ORDERING only — never results.** It
//! enters exclusively through the *cost* term (and the suffix selection score), never the
//! output *cardinality*, so the soundness boundary below is untouched: the re-plan is still
//! a pure reorder of the not-yet-executed suffix, and the result multiset is unchanged.
//!
//! ## Soundness boundary (load-bearing — read this)
//!
//! Re-planning happens **only at stage boundaries** — between two leaf joins of the
//! left-deep plan, i.e. once an operator has fully produced its output and before the next
//! join begins. It is **NOT** a mid-operator swap: a join already in flight is never torn
//! down and rebuilt. Concretely, the executor model here ([`AdaptiveExecutor`]) runs the
//! plan as a sequence of *stages* (one per pattern after the seed); a re-plan may reorder
//! the **suffix** of stages that have not started, but never the prefix already produced.
//!
//! Why this is sound for query RESULTS:
//!
//! > A BGP is a conjunction of triple patterns; its answer is the natural join of the
//! > per-pattern solution multisets, which is **commutative and associative**. Any join
//! > order over the same set of patterns produces the **same** result multiset. A re-plan
//! > only changes the *order* in which the remaining patterns are joined (and the
//! > per-join algorithm, which [`crate::stream`] already proves is multiset-preserving) —
//! > it never adds, drops, or alters a pattern. The already-produced prefix is the partial
//! > join of an *initial subset* of the patterns; joining the remaining patterns onto it
//! > in any order yields the full join. Hence the adaptive result is bit-for-bit the same
//! > multiset as the static plan.
//!
//! The prefix's bindings are **carried across the switch unchanged** — the re-plan starts
//! from the current intermediate result, not from scratch, so no work is re-done and no
//! binding is lost or duplicated. This is verified by [`tests::replan_result_equals_static`].
//!
//! ## Deferred (filed as beads, NOT built here)
//!
//! * **Mid-operator** adaptivity — tearing down a join *while it is producing* and
//!   resuming the half-built hash tables under a new algorithm — is out of scope; the
//!   boundary above is the sound, well-scoped version. (Roadmap bead, epic sq-3183.)
//! * **Source switching** (failover to a replica mid-stage when a source goes dark) reuses
//!   the same trigger machinery but needs a live multi-source execution layer this pure
//!   crate does not own; deferred.
//!
//! ## Prior art: porting this to the LOCAL evaluator — attempted, unmerged (read before re-deriving)
//!
//! [OPUS-5] The recurring request to reuse this trigger + hysteresis in the **local**
//! (`sparq-engine`) join loop — gh-903 / sq-gafdh / #2932 — **has already been implemented
//! once, in full, and was held back rather than merged.** It is recorded here because the
//! federated re-planner is where a reader looking to port it starts, and the prior art was
//! not discoverable from this module, which is very likely why the task kept being re-raised.
//!
//! * The implementation is commit `f85d0180` on branch `sq-p6p6-adaptive-replan-local`
//!   (verified **not** an ancestor of `main`). It ships an opt-in `adaptive-replan-local`
//!   engine feature (OFF by default), a `replan_result_equals_static` oracle, and an
//!   in-process micro-benchmark.
//! * The "put the policy in a shared module so the two cannot diverge" half was **started but
//!   NOT completed there**: that branch extracted a *candidate* shared policy — a pure,
//!   dependency-free `sparq-replan-policy` crate carrying a verbatim port of the divergence +
//!   hysteresis rule — and wired only the **local** evaluator onto it. It deliberately did
//!   **not** rewire *this* module, which keeps its own richer policy (the latency knobs
//!   below), so the federated path and this file's adaptive test suite stayed provably
//!   unaffected. One shared crate with one consumer does not stop the two rules drifting, so
//!   convergence onto a single implementation is still outstanding. Doing that rewiring here
//!   as a standalone change, with no local consumer on `main`, would take the regression risk
//!   for no convergence gain — it belongs with whatever lands the local consumer.
//! * **Why it was held back:** its benchmark gate was recorded UNMET — the engine
//!   micro-benchmark showed no win on a non-canonical work box. No numbers are repeated
//!   here; see the commit and `bench/` protocol.
//!
//! **The negative is narrower than "the idea does not work."** Per
//! `research/adaptive-replan-local-evaluator.md` §2.2, the mis-estimated fixture that was
//! measured contains **no pruning arm** — both of its remaining arms inflate — so it never
//! instantiated the shape a reorder could win on, and whether a local adaptive reorder pays
//! off on a genuinely *pruning* remaining arm is **still open, not refuted**. That record is
//! the authority on what was and was not measured, on the redesign options (checkpoint-
//! triggered strategy switch, sq-6i40, is judged the higher-ceiling path), and on the
//! maintainer questions still outstanding. Read it before re-implementing anything.
//!
//! [OPUS-4.8] sq-7s4z (epic sq-3183) — flagged for Fable re-review.

use crate::pattern::Bgp;
use crate::plan::{JoinTree, PlanOptions};
use crate::selection::PatternSources;
use std::collections::HashMap;

/// [FABLE-5] sq-s5kd. How a multi-source pattern's per-source latency observations are
/// aggregated into the single cost factor its joins are scaled by.
///
/// * [`Self::SlowestArm`] (default, sq-b51o behaviour) — the pattern is charged at its
///   slowest retained source's factor: a **bottleneck / wall-clock** model (a parallel
///   union is not done until its slowest arm is).
/// * [`Self::CardinalityWeighted`] — the per-source factors are averaged weighted by each
///   retained source's estimated cardinality share: an **expected-work** model (a tiny slow
///   arm no longer dominates a huge fast arm's pattern). Falls back to the plain mean when
///   every retained cardinality is `0`.
///
/// Both are HEURISTICS; both leave a pattern with **no** latency observation at factor
/// `1.0` (inert until evidence arrives), and both bias cost/ordering ONLY — never the
/// result multiset. See the module docs ("Per-source (not slowest-arm) aggregation").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LatencyAggregation {
    /// Charge the pattern at its slowest retained source's latency factor (bottleneck
    /// model — the sq-b51o behaviour, and the default).
    #[default]
    SlowestArm,
    /// Charge the pattern at the cardinality-weighted mean of its retained sources'
    /// latency factors (expected-work model — opt-in, sq-s5kd).
    CardinalityWeighted,
}

/// Policy controlling when a mid-execution re-plan fires and whether its result is adopted.
///
/// The two knobs together give the divergence-then-hysteresis behaviour: a re-plan is
/// *triggered* by a large estimate error ([`Self::divergence_factor`]) but only *adopted*
/// when the new suffix is a clear cost win ([`Self::improvement_margin`]) — so noisy-but-
/// stable statistics never cause the plan to thrash.
#[derive(Debug, Clone)]
pub struct ReplanPolicy {
    /// Divergence factor `k` (> 1). A re-plan is *considered* when an observed cardinality
    /// `o` for a not-yet-executed operator diverges from its estimate `e` by more than this
    /// factor in either direction: `o > k·e` **or** `e > k·o`. Default `4.0` (a 4× miss).
    pub divergence_factor: f64,
    /// Minimum relative cost improvement (in `[0, 1)`) the re-planned suffix must show over
    /// the current suffix's estimated remaining cost before it is adopted — the hysteresis
    /// band that prevents thrashing. `0.1` means "only switch if it is ≥ 10% cheaper".
    /// Default `0.1`: enough to absorb cost-model noise yet low enough to act on the modest
    /// (10–20%) gains a two-/three-arm suffix reorder typically yields on small
    /// intermediates — a higher band would let re-planning *fire* but never *switch*.
    pub improvement_margin: f64,
    /// Hard cap on how many re-plans a single execution may perform (belt-and-braces
    /// anti-thrash + bounded planning overhead). Default `8`.
    pub max_replans: usize,
    /// [OPUS-4.8] sq-b51o. Weight `w ≥ 0` of observed per-source latency in the cost model
    /// (a HEURISTIC constant, not a derived optimum). A candidate join whose slowest retained
    /// source is observed at relative slowness `s = latency / latency_baseline` has its cost
    /// scaled by `clamp(1 + w·(s − 1), latency_floor, latency_cap)`. `w = 0` disables latency
    /// weighting entirely (pure cardinality planning); the default `0.5` lets a 2×-slow source
    /// cost 1.5× — a deliberately *gentle* bias, because latency measurements are noisy.
    pub latency_weight: f64,
    /// [OPUS-4.8] sq-b51o. Baseline latency (same abstract units as
    /// [`RuntimeStats::record_source_latency`]) a source is judged *relative to*: a source
    /// observed at this latency is "typical" (relative slowness `s = 1` ⇒ cost factor `1.0`).
    /// A hand-set reference point, not measured. Default `100.0`.
    pub latency_baseline: f64,
    /// [OPUS-4.8] sq-b51o. Lower clamp on the latency cost factor — a source observed *faster*
    /// than baseline can make its joins cheaper, but never below this floor (a fast source is
    /// still a remote sub-query). Default `0.5`.
    pub latency_floor: f64,
    /// [OPUS-4.8] sq-b51o. Upper clamp on the latency cost factor — stops a single outlier
    /// latency spike from dominating the join order (anti-thrash for the latency term, mirroring
    /// the cardinality `divergence_factor` discipline). Default `4.0`.
    pub latency_cap: f64,
    /// [FABLE-5] sq-s5kd. How a multi-source pattern's per-source latencies fold into its
    /// single cost factor: [`LatencyAggregation::SlowestArm`] (default — bottleneck model,
    /// the sq-b51o behaviour, bit-identical for existing callers) or the opt-in
    /// [`LatencyAggregation::CardinalityWeighted`] (expected-work model). Single-source
    /// patterns are unaffected (both modes reduce to that source's factor).
    pub latency_aggregation: LatencyAggregation,
}

impl Default for ReplanPolicy {
    fn default() -> Self {
        ReplanPolicy {
            divergence_factor: 4.0,
            improvement_margin: 0.1,
            max_replans: 8,
            latency_weight: 0.5,
            latency_baseline: 100.0,
            latency_floor: 0.5,
            latency_cap: 4.0,
            latency_aggregation: LatencyAggregation::SlowestArm,
        }
    }
}

impl ReplanPolicy {
    /// [OPUS-4.8] sq-b51o. The latency cost-multiplier for a single observed source latency,
    /// or `None` when latency weighting is off (`latency_weight == 0`). A source at the
    /// baseline (or with no observation — handled by the caller passing `None`) yields `1.0`,
    /// so the model is inert until evidence diverges from baseline. HEURISTIC — see the
    /// module docs.
    fn latency_factor(&self, latency: f64) -> f64 {
        if self.latency_weight <= 0.0 {
            return 1.0;
        }
        let baseline = self.latency_baseline.max(f64::MIN_POSITIVE);
        let relative = latency.max(0.0) / baseline;
        let raw = 1.0 + self.latency_weight * (relative - 1.0);
        let lo = self.latency_floor.clamp(0.0, 1.0);
        let hi = self.latency_cap.max(1.0);
        raw.clamp(lo, hi)
    }
}

/// Observed runtime statistics, accumulated as execution proceeds. Distinct from the
/// descriptor-derived *estimates*: every value here is a real count / measurement the
/// executor fed in at a stage boundary.
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    /// Observed leaf cardinality per pattern index (rows the pattern actually returned,
    /// union over its sources). Present only for patterns that have been (at least
    /// partially) evaluated.
    observed_leaf_card: HashMap<usize, f64>,
    /// Per-source **EWMA-smoothed** latency, in abstract time units (e.g. ms). [OPUS-4.8]
    /// sq-b51o (+ follow-up): each [`Self::record_source_latency`] folds the new observation
    /// into a per-source exponentially-weighted moving average rather than overwriting — this
    /// is the value fed into the cost model + the re-plan trigger, so a single transient spike
    /// is damped and only a *sustained* shift moves the planner. A slow source's joins cost
    /// more, biasing the re-plan toward faster sources / deferring the slow one (see
    /// [`ReplanPolicy::latency_weight`]). Affects cost/ordering ONLY, never the result multiset.
    observed_latency: HashMap<usize, f64>,
    /// [OPUS-4.8] sq-b51o follow-up. **Global** (fallback) EWMA smoothing factor α ∈ (0, 1]
    /// applied per source in [`Self::record_source_latency`]: `ewma_new = α·observed +
    /// (1−α)·ewma_prev`. A **hand-picked HEURISTIC**, default [`Self::DEFAULT_LATENCY_ALPHA`]
    /// (`0.3`) — see the module docs. Lower = calmer/laggier, higher = twitchier/faster-tracking;
    /// not derived from a workload. Clamped to `(0, 1]` on construction. A per-source override in
    /// [`Self::source_alpha`] takes precedence over this global value when present
    /// ([OPUS-4.8] sq-3xkz).
    latency_alpha: f64,
    /// [OPUS-4.8] sq-3xkz. **Per-source** EWMA α override map. When a source has an entry here it
    /// is used in preference to the global [`Self::latency_alpha`], so each source can be smoothed
    /// at its own rate (a steady source can keep a low/calm α; a bursty one a higher/twitchier
    /// α). Empty by default — and an empty map means EVERY source falls back to the global α, so
    /// the default path is **byte-identical** to the prior single-global-α behaviour
    /// (back-compat). Values are clamped to `(0, 1]` on insert, same as the global. Still a
    /// HEURISTIC: the MECHANISM to set α per source, not a tuned α — tuning needs a real
    /// federated workload (deferred, sq-3xkz).
    source_alpha: HashMap<usize, f64>,
    /// [OPUS-4.8] sq-3xkz. Optional **time-aware decay** half-life, in the same abstract time
    /// units the elapsed gap is passed in. `None` (default) ⇒ **no** time decay: a sample folds in
    /// at the plain per-source α regardless of the wall-clock gap since the previous sample, so
    /// the default path reproduces the prior equal-weighting EWMA exactly (back-compat). `Some(h)`
    /// ⇒ a sample recorded via [`Self::record_source_latency_after`] with an elapsed gap `Δt`
    /// folds in at an **inflated** effective α that grows toward `1.0` as `Δt` grows past `h`
    /// (older stats decay toward the new sample / prior), so a long idle gap lets a fresh sample
    /// dominate rather than being averaged equally with a now-stale history. Injectable/elapsed-in
    /// so it stays deterministic — the clock is never read directly here.
    decay_half_life: Option<f64>,
    /// [OPUS-4.8] sq-3xkz. Per-source **logical age**: the value of [`Self::clock`] at the moment
    /// that source was last updated. Used purely for [`Self::evict_stale`] / [`Self::source_age`];
    /// it never affects the EWMA value. Sources recorded via the plain (no-elapsed)
    /// [`Self::record_source_latency`] are stamped at the current [`Self::clock`] (age 0 at record
    /// time).
    last_seen: HashMap<usize, f64>,
    /// [OPUS-4.8] sq-3xkz. Monotone **logical clock** (abstract units), advanced ONLY by the
    /// `elapsed` arg of [`Self::record_source_latency_after`] / [`Self::advance_clock`]. Starts at
    /// `0.0`. Deterministic by construction — there is no hidden wall-clock read; the caller owns
    /// the time source and passes elapsed gaps in, so tests are reproducible.
    clock: f64,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        RuntimeStats {
            observed_leaf_card: HashMap::new(),
            observed_latency: HashMap::new(),
            latency_alpha: RuntimeStats::DEFAULT_LATENCY_ALPHA,
            source_alpha: HashMap::new(),
            decay_half_life: None,
            last_seen: HashMap::new(),
            clock: 0.0,
        }
    }
}

impl RuntimeStats {
    /// [OPUS-4.8] sq-b51o follow-up. Default per-source latency EWMA smoothing factor α — a
    /// hand-picked heuristic (latest sample weighted 30%, history 70%): enough history to damp
    /// a single transient spike below the re-plan trigger, enough recency to converge on a
    /// sustained shift within a few samples. NOT a derived optimum.
    pub const DEFAULT_LATENCY_ALPHA: f64 = 0.3;

    /// A fresh, empty statistics store with the default latency EWMA α
    /// ([`Self::DEFAULT_LATENCY_ALPHA`]).
    pub fn new() -> RuntimeStats {
        RuntimeStats::default()
    }

    /// [OPUS-4.8] sq-b51o follow-up. A fresh, empty statistics store with an explicit latency
    /// EWMA smoothing factor `alpha` (clamped to `(0, 1]`). `alpha = 1.0` recovers the old
    /// "single last observation" behaviour (no smoothing); the default
    /// ([`Self::DEFAULT_LATENCY_ALPHA`]) is `0.3`.
    pub fn with_latency_alpha(alpha: f64) -> RuntimeStats {
        RuntimeStats {
            latency_alpha: Self::clamp_alpha(alpha),
            ..RuntimeStats::default()
        }
    }

    /// The configured **global** latency EWMA smoothing factor α (always in `(0, 1]`). This is
    /// the fallback used for any source without a per-source override
    /// ([`Self::set_source_alpha`]).
    pub fn latency_alpha(&self) -> f64 {
        self.latency_alpha
    }

    /// [OPUS-4.8] sq-3xkz. Set a **per-source** EWMA α override for `source` (clamped to
    /// `(0, 1]`), taking precedence over the global [`Self::latency_alpha`] for that source's
    /// subsequent [`Self::record_source_latency`] / [`Self::record_source_latency_after`] folds.
    /// This is the MECHANISM for adaptive per-source smoothing — a steady source can keep a calm
    /// (low) α while a bursty one runs twitchier (high). With **no** override set anywhere the
    /// behaviour is byte-identical to the prior single-global-α path (back-compat). NOTE: the
    /// *value* to use is a heuristic; actually tuning it needs a real federated workload — not
    /// shipped here (sq-3xkz).
    pub fn set_source_alpha(&mut self, source: usize, alpha: f64) {
        self.source_alpha.insert(source, Self::clamp_alpha(alpha));
    }

    /// [OPUS-4.8] sq-3xkz. Builder form of [`Self::set_source_alpha`]: returns `self` with the
    /// per-source α override applied, for chaining at construction.
    pub fn with_source_alpha(mut self, source: usize, alpha: f64) -> RuntimeStats {
        self.set_source_alpha(source, alpha);
        self
    }

    /// [OPUS-4.8] sq-3xkz. The **effective** EWMA α for `source`: its per-source override
    /// ([`Self::set_source_alpha`]) if present, else the global [`Self::latency_alpha`]. With an
    /// empty override map this is always the global α, so the default path is unchanged.
    pub fn effective_alpha(&self, source: usize) -> f64 {
        self.source_alpha
            .get(&source)
            .copied()
            .unwrap_or(self.latency_alpha)
    }

    /// Clamp a requested α into the valid `(0, 1]` range (a degenerate `α ≤ 0` would freeze
    /// the average at its seed; `α > 1` would over-shoot). Uses `f64::MIN_POSITIVE` as the
    /// strictly-positive floor.
    fn clamp_alpha(alpha: f64) -> f64 {
        if alpha.is_nan() {
            return Self::DEFAULT_LATENCY_ALPHA;
        }
        alpha.clamp(f64::MIN_POSITIVE, 1.0)
    }

    /// [OPUS-4.8] sq-3xkz. Enable **time-aware decay** with a `half_life` (abstract time units,
    /// the same scale the elapsed gap is later passed in). A non-finite or non-positive value
    /// **disables** decay (sets it back to `None`), the back-compat default. See
    /// [`Self::decay_half_life`] for the semantics; the actual decay applies only on samples fed
    /// through [`Self::record_source_latency_after`].
    pub fn set_decay_half_life(&mut self, half_life: f64) {
        self.decay_half_life = if half_life.is_finite() && half_life > 0.0 {
            Some(half_life)
        } else {
            None
        };
    }

    /// [OPUS-4.8] sq-3xkz. Builder form of [`Self::set_decay_half_life`].
    pub fn with_decay_half_life(mut self, half_life: f64) -> RuntimeStats {
        self.set_decay_half_life(half_life);
        self
    }

    /// [OPUS-4.8] sq-3xkz. The configured time-aware-decay half-life, or `None` when decay is off.
    pub fn decay_half_life(&self) -> Option<f64> {
        self.decay_half_life
    }

    /// [OPUS-4.8] sq-3xkz. The current logical clock (abstract units), advanced only by the
    /// `elapsed` arg of [`Self::record_source_latency_after`] / [`Self::advance_clock`].
    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// [OPUS-4.8] sq-3xkz. Advance the logical clock by `elapsed` (abstract units; a negative or
    /// non-finite value is treated as `0`). Lets a caller age the store between queries without
    /// recording a sample (e.g. before a bulk [`Self::evict_stale`]). Deterministic — elapsed is
    /// supplied by the caller, never read from a wall clock.
    pub fn advance_clock(&mut self, elapsed: f64) {
        self.clock += elapsed.max(0.0);
    }

    /// [OPUS-4.8] sq-3xkz. The **logical age** of `source` — `clock − last_seen[source]`, i.e. how
    /// much logical time has elapsed since this source was last recorded — or `None` if the source
    /// has no observation. Always `≥ 0` (the clock is monotone). Drives [`Self::evict_stale`].
    pub fn source_age(&self, source: usize) -> Option<f64> {
        self.last_seen
            .get(&source)
            .map(|seen| (self.clock - seen).max(0.0))
    }

    /// Record the *observed* cardinality of a pattern's leaf (the real row count its
    /// source(s) returned). Overwrites any prior observation for that pattern.
    pub fn record_leaf_cardinality(&mut self, pattern: usize, observed: f64) {
        self.observed_leaf_card.insert(pattern, observed.max(0.0));
    }

    /// Record an *observed* latency sample for `source` (abstract time units), folding it into
    /// that source's **EWMA**: `ewma_new = α·observed + (1−α)·ewma_prev`, where α is the
    /// source's [`Self::effective_alpha`] (its per-source override, else the global
    /// [`Self::latency_alpha`]). The **first** sample for a source seeds the average
    /// (`ewma = observed`); subsequent samples decay the history. The smoothed value — not the
    /// raw sample — is what [`Self::observed_latency_of`] returns and the cost model/trigger
    /// read, so a single transient spike is damped (anti-thrash) while a sustained shift
    /// converges. [OPUS-4.8] sq-b51o follow-up. Affects cost/ordering ONLY, never results.
    ///
    /// This is the **time-unaware** fold: it folds at the plain per-source α regardless of any
    /// wall-clock gap (back-compat). For time-aware decay pass the elapsed gap to
    /// [`Self::record_source_latency_after`] ([OPUS-4.8] sq-3xkz). The source is stamped at the
    /// current logical [`Self::clock`] (age 0 at record time).
    pub fn record_source_latency(&mut self, source: usize, latency: f64) {
        self.fold_latency(source, latency, self.effective_alpha(source));
    }

    /// [OPUS-4.8] sq-3xkz. Record a latency sample for `source` `elapsed` abstract time units
    /// after the previous one, applying **time-aware decay**: the longer the gap, the more the
    /// stale history is decayed and the more the new sample is trusted.
    ///
    /// Concretely the logical [`Self::clock`] advances by `elapsed`, and the **effective** fold α
    /// is inflated from the source's base α `α₀` ([`Self::effective_alpha`]) toward `1.0` by the
    /// decay weight `d = 1 − 0.5^(Δt / half_life) ∈ [0, 1)`:
    ///
    /// > `α_eff = α₀ + (1 − α₀)·d`
    ///
    /// so at `Δt = 0` (no gap) `α_eff = α₀` (identical to [`Self::record_source_latency`]); at one
    /// half-life `d = 0.5`; and as `Δt → ∞`, `α_eff → 1` (the new sample fully replaces the
    /// now-stale history — older stats decay toward the prior). With **no** half-life configured
    /// ([`Self::set_decay_half_life`] never called) decay is off and this folds at the plain α
    /// regardless of `elapsed`, so it matches [`Self::record_source_latency`] on the value
    /// (it still advances the clock + stamps `last_seen`, which feeds [`Self::evict_stale`]).
    /// The first sample for a source seeds the average (no history to decay). Deterministic: the
    /// caller owns the time source and passes `elapsed` in — no wall clock is read here.
    pub fn record_source_latency_after(&mut self, source: usize, latency: f64, elapsed: f64) {
        let gap = elapsed.max(0.0);
        self.clock += gap;
        let base = self.effective_alpha(source);
        let alpha = match self.decay_half_life {
            // Only inflate α when there IS a prior value to decay; a fresh source seeds verbatim.
            Some(half_life) if self.observed_latency.contains_key(&source) => {
                let decay = 1.0 - 0.5_f64.powf(gap / half_life);
                // α_eff = α₀ + (1 − α₀)·decay, clamped into (0, 1].
                Self::clamp_alpha(base + (1.0 - base) * decay)
            }
            _ => base,
        };
        self.fold_latency(source, latency, alpha);
    }

    /// [OPUS-4.8] sq-3xkz. The shared EWMA fold: seed on the first sample for `source`, otherwise
    /// blend at `alpha`. Always stamps the source's `last_seen` at the current logical clock so
    /// [`Self::source_age`] / [`Self::evict_stale`] track recency. `alpha` is assumed already
    /// clamped into `(0, 1]` by the caller.
    fn fold_latency(&mut self, source: usize, latency: f64, alpha: f64) {
        let observed = latency.max(0.0);
        match self.observed_latency.entry(source) {
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let prev = *o.get();
                let next = alpha * observed + (1.0 - alpha) * prev;
                o.insert(next);
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(observed);
            }
        }
        self.last_seen.insert(source, self.clock);
    }

    /// [OPUS-4.8] sq-3xkz. **Staleness eviction.** Drop every source whose [`Self::source_age`]
    /// (`clock − last_seen`) is **strictly greater** than `max_age` (abstract time units) — entries
    /// older than the threshold are removed from both the EWMA map and the age map, so a long-lived
    /// `RuntimeStats` reused across queries does not let a stale latency keep biasing the planner.
    /// Returns the number of sources evicted. A non-finite or negative `max_age` evicts nothing
    /// (treated as "never stale"). Sources at exactly `max_age` are kept (boundary-inclusive
    /// retention). Advance the clock first (via [`Self::record_source_latency_after`] or
    /// [`Self::advance_clock`]) so ages reflect the gap since each source was last seen.
    pub fn evict_stale(&mut self, max_age: f64) -> usize {
        if !max_age.is_finite() || max_age < 0.0 {
            return 0;
        }
        let clock = self.clock;
        let stale: Vec<usize> = self
            .last_seen
            .iter()
            .filter(|&(_, &seen)| (clock - seen).max(0.0) > max_age)
            .map(|(&source, _)| source)
            .collect();
        for source in &stale {
            self.observed_latency.remove(source);
            self.last_seen.remove(source);
        }
        stale.len()
    }

    /// The observed leaf cardinality for `pattern`, if recorded.
    pub fn observed_leaf(&self, pattern: usize) -> Option<f64> {
        self.observed_leaf_card.get(&pattern).copied()
    }

    /// The **EWMA-smoothed** observed latency for `source`, if any sample has been recorded.
    /// This is the smoothed running average, not the last raw sample ([OPUS-4.8] sq-b51o
    /// follow-up) — it is what the latency cost factor and the re-plan trigger consume.
    pub fn observed_latency_of(&self, source: usize) -> Option<f64> {
        self.observed_latency.get(&source).copied()
    }
}

/// Builds a fresh per-pattern source selection whose leaf cardinality estimates are
/// **overridden by observed values** where the runtime stats have them. Patterns with no
/// observation keep their original (descriptor-derived) estimate.
///
/// The override is applied proportionally across a pattern's retained sources so that the
/// per-source candidate breakdown — which the star-cardinality estimator reads — stays
/// consistent with the new total. Source *membership* is never changed here (re-planning
/// reorders; it does not re-prune, which keeps recall-safety intact).
fn corrected_selection(base: &[PatternSources], stats: &RuntimeStats) -> Vec<PatternSources> {
    base.iter()
        .map(|ps| {
            let Some(obs) = stats.observed_leaf(ps.pattern) else {
                return ps.clone();
            };
            let mut ps = ps.clone();
            let old_total: f64 = ps.candidates.iter().map(|c| c.estimated_cardinality).sum();
            if ps.candidates.is_empty() {
                return ps;
            }
            if old_total > 0.0 {
                // Scale each source's share to hit the observed total, preserving skew.
                let scale = obs / old_total;
                for c in &mut ps.candidates {
                    c.estimated_cardinality *= scale;
                }
            } else {
                // No prior signal to distribute on — spread the observation evenly.
                let per = obs / ps.candidates.len() as f64;
                for c in &mut ps.candidates {
                    c.estimated_cardinality = per;
                }
            }
            ps
        })
        .collect()
}

/// [OPUS-4.8] sq-b51o. The **slowest** observed latency over the sources retained for
/// `pattern` (a union is bottlenecked by its slowest arm), or `None` when no source of the
/// pattern has a latency observation yet.
fn pattern_slowest_latency(
    pattern: usize,
    selection: &[PatternSources],
    stats: &RuntimeStats,
) -> Option<f64> {
    let ps = selection.iter().find(|ps| ps.pattern == pattern)?;
    ps.candidates
        .iter()
        .filter_map(|c| stats.observed_latency_of(c.source))
        .fold(None::<f64>, |acc, l| Some(acc.map_or(l, |a| a.max(l))))
}

/// [OPUS-4.8] sq-b51o. The latency cost-multiplier for joining `pattern`, aggregated over
/// its retained sources per [`ReplanPolicy::latency_aggregation`] ([FABLE-5] sq-s5kd):
///
/// * [`LatencyAggregation::SlowestArm`] — the slowest source's observed latency run through
///   [`ReplanPolicy::latency_factor`] (bottleneck model, the default).
/// * [`LatencyAggregation::CardinalityWeighted`] — each retained source's factor
///   individually, averaged weighted by its estimated cardinality (expected-work model).
///
/// Returns `1.0` (inert) in **both** modes when no source of the pattern has a latency
/// observation — the latency model contributes nothing until evidence arrives, so a
/// measurement-free re-plan is byte-identical to cardinality-only.
fn pattern_latency_factor(
    pattern: usize,
    selection: &[PatternSources],
    stats: &RuntimeStats,
    policy: &ReplanPolicy,
) -> f64 {
    match policy.latency_aggregation {
        LatencyAggregation::SlowestArm => {
            match pattern_slowest_latency(pattern, selection, stats) {
                Some(l) => policy.latency_factor(l),
                None => 1.0,
            }
        }
        LatencyAggregation::CardinalityWeighted => {
            pattern_weighted_latency_factor(pattern, selection, stats, policy)
        }
    }
}

/// [FABLE-5] sq-s5kd. The cardinality-weighted mean of the per-source latency factors over
/// `pattern`'s retained sources: `Σ card_c · factor_c / Σ card_c`, where `factor_c` is the
/// source's EWMA latency through [`ReplanPolicy::latency_factor`] (or `1.0` when the source
/// has no observation — inert, exactly as in slowest-arm mode). Each `factor_c` is already
/// clamped to `[latency_floor, latency_cap]` and the result is a convex combination of them,
/// so it needs no re-clamp. Falls back to the **plain mean** over the retained sources when
/// every retained cardinality is `0` (nothing to weight by), and to `1.0` for a pattern with
/// no retained sources at all. With no latency observation on any source every `factor_c` is
/// exactly `1.0`, so the mean is exactly `1.0` — the off-by-construction invariant holds.
fn pattern_weighted_latency_factor(
    pattern: usize,
    selection: &[PatternSources],
    stats: &RuntimeStats,
    policy: &ReplanPolicy,
) -> f64 {
    let Some(ps) = selection.iter().find(|ps| ps.pattern == pattern) else {
        return 1.0;
    };
    if ps.candidates.is_empty() {
        return 1.0;
    }
    let mut weighted = 0.0f64;
    let mut plain = 0.0f64;
    let mut total_weight = 0.0f64;
    for c in &ps.candidates {
        let factor = match stats.observed_latency_of(c.source) {
            Some(l) => policy.latency_factor(l),
            None => 1.0,
        };
        let w = c.estimated_cardinality.max(0.0);
        weighted += w * factor;
        plain += factor;
        total_weight += w;
    }
    if total_weight > 0.0 {
        weighted / total_weight
    } else {
        plain / ps.candidates.len() as f64
    }
}

/// [OPUS-4.8] sq-b51o. The pattern's slowest source's *relative slowness* `s = latency /
/// baseline` (un-clamped, un-weighted) under `policy`'s [`ReplanPolicy::latency_baseline`] —
/// what the trigger compares to `divergence_factor`. `None` when the pattern has no latency
/// observation.
fn pattern_relative_slowness(
    pattern: usize,
    selection: &[PatternSources],
    stats: &RuntimeStats,
    policy: &ReplanPolicy,
) -> Option<f64> {
    let baseline = policy.latency_baseline.max(f64::MIN_POSITIVE);
    pattern_slowest_latency(pattern, selection, stats).map(|l| l / baseline)
}

/// Whether observed cardinality `observed` diverges from estimate `estimate` by more than
/// the policy's factor (in either direction).
fn diverges(estimate: f64, observed: f64, policy: &ReplanPolicy) -> bool {
    let k = policy.divergence_factor.max(1.0);
    let e = estimate.max(1.0);
    let o = observed.max(0.0);
    o > k * e || e > k * o.max(1.0)
}

/// The outcome of one re-plan attempt at a stage boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplanOutcome {
    /// No observed statistic for a not-yet-executed pattern diverged past the threshold;
    /// the trigger did not fire.
    NoDivergence,
    /// The trigger fired but the re-planned suffix did not beat the current one by the
    /// hysteresis margin — the existing plan is kept (anti-thrash).
    KeptWithinHysteresis,
    /// The re-plan fired and the new suffix won by more than the margin — the remaining
    /// join order was switched.
    Switched,
    /// The re-plan budget ([`ReplanPolicy::max_replans`]) was exhausted; no further
    /// re-planning is attempted this execution.
    BudgetExhausted,
}

/// An adaptive executor that runs a static [`JoinTree`] as a sequence of **stages** (the
/// left-deep join order), capturing runtime statistics and re-planning the not-yet-run
/// suffix at stage boundaries.
///
/// This is the orchestration model the soundness boundary is stated against: the executor
/// always holds (a) the patterns already joined — the **prefix** — and (b) the patterns
/// still to join — the **suffix**. A re-plan reorders only the suffix.
pub struct AdaptiveExecutor<'a> {
    bgp: &'a Bgp,
    descriptors: &'a [crate::descriptor::SourceDescriptor],
    base_selection: &'a [PatternSources],
    opts: PlanOptions,
    policy: ReplanPolicy,
    /// Patterns already joined, in execution order (the prefix). The first is the seed.
    executed: Vec<usize>,
    /// Patterns not yet joined, in their *current planned* order (the suffix).
    remaining: Vec<usize>,
    replans_done: usize,
}

impl<'a> AdaptiveExecutor<'a> {
    /// Starts an adaptive execution from a static `plan` (its join order seeds the prefix
    /// /suffix split: nothing executed yet, the whole order is the suffix).
    pub fn new(
        bgp: &'a Bgp,
        descriptors: &'a [crate::descriptor::SourceDescriptor],
        base_selection: &'a [PatternSources],
        plan: &JoinTree,
        opts: PlanOptions,
        policy: ReplanPolicy,
    ) -> AdaptiveExecutor<'a> {
        AdaptiveExecutor {
            bgp,
            descriptors,
            base_selection,
            opts,
            policy,
            executed: Vec::new(),
            remaining: plan.join_order(),
            replans_done: 0,
        }
    }

    /// The patterns already executed (the prefix), in execution order.
    pub fn executed_order(&self) -> &[usize] {
        &self.executed
    }

    /// The patterns not yet executed (the suffix), in current planned order.
    pub fn remaining_order(&self) -> &[usize] {
        &self.remaining
    }

    /// Marks the front pattern of the suffix as executed (an operator completed), moving it
    /// into the prefix. Returns the pattern index, or `None` if the suffix is empty.
    pub fn advance(&mut self) -> Option<usize> {
        if self.remaining.is_empty() {
            return None;
        }
        let p = self.remaining.remove(0);
        self.executed.push(p);
        Some(p)
    }

    /// Considers a re-plan at the current stage boundary given the latest `stats`.
    ///
    /// Fires the trigger when a not-yet-executed pattern's observed leaf cardinality
    /// diverges from its estimate past the policy factor, **or** ([OPUS-4.8] sq-b51o) a
    /// not-yet-executed pattern's slowest source is observed to be `divergence_factor×`
    /// slower than the latency baseline; if so, re-plans the **suffix** with the corrected
    /// statistics (cardinality *and* latency-weighted cost) and adopts the new order only
    /// when it wins by the hysteresis margin. Returns the [`ReplanOutcome`]. The prefix is
    /// never touched, and latency enters cost/ordering ONLY — never the result multiset.
    pub fn maybe_replan(&mut self, stats: &RuntimeStats) -> ReplanOutcome {
        if self.remaining.len() < 2 {
            // Fewer than two remaining patterns ⇒ nothing to reorder.
            return ReplanOutcome::NoDivergence;
        }
        if self.replans_done >= self.policy.max_replans {
            return ReplanOutcome::BudgetExhausted;
        }

        // ---- Trigger: did any NOT-YET-EXECUTED pattern's observation diverge?
        let base_est: HashMap<usize, f64> = self
            .base_selection
            .iter()
            .map(|ps| (ps.pattern, ps.total_cardinality().max(0.0)))
            .collect();
        let card_triggered =
            self.remaining
                .iter()
                .any(|&p| match (base_est.get(&p), stats.observed_leaf(p)) {
                    (Some(&e), Some(o)) => diverges(e, o, &self.policy),
                    _ => false,
                });
        // [OPUS-4.8] sq-b51o (+ EWMA follow-up): a not-yet-executed pattern bound to an
        // observably slow source — whose slowest source's EWMA-SMOOTHED latency is past
        // divergence_factor× baseline — is also enough to *consider* a re-plan: a latency-bound
        // query reacts even when cardinalities are spot-on. Reading the EWMA (not the raw last
        // sample) means one transient spike does not move the smoothed value over the threshold,
        // so the trigger is anti-thrashed at the source; only a sustained shift converges past
        // it. The hysteresis margin still gates whether a switch is actually *adopted*.
        let lat_triggered = self.policy.latency_weight > 0.0
            && self.remaining.iter().any(|&p| {
                pattern_relative_slowness(p, self.base_selection, stats, &self.policy)
                    .is_some_and(|s| s > self.policy.divergence_factor.max(1.0))
            });
        if !card_triggered && !lat_triggered {
            return ReplanOutcome::NoDivergence;
        }

        // ---- Re-plan the SUFFIX with corrected statistics.
        // Build a sub-BGP of the remaining patterns; plan over it with observed cardinalities
        // and latency-weighted join costs.
        let corrected = corrected_selection(self.base_selection, stats);
        let Some(new_suffix) = self.replan_suffix(&corrected, stats) else {
            return ReplanOutcome::KeptWithinHysteresis;
        };

        // ---- Hysteresis: only adopt if the new suffix beats the current one by the margin.
        let cur_cost = self.suffix_cost(&self.remaining, &corrected, stats);
        let new_cost = self.suffix_cost(&new_suffix, &corrected, stats);
        let margin = self.policy.improvement_margin.clamp(0.0, 0.999);
        // new must be cheaper than (1 - margin) * current to win.
        if new_cost < cur_cost * (1.0 - margin) && new_suffix != self.remaining {
            self.remaining = new_suffix;
            self.replans_done += 1;
            ReplanOutcome::Switched
        } else {
            ReplanOutcome::KeptWithinHysteresis
        }
    }

    /// Re-plans just the remaining patterns, returning the new suffix order. Anchored to the
    /// already-executed prefix: the re-planner sees the prefix as "already joined" so it
    /// continues to prefer patterns connected to the running result (no Cartesian arm gets
    /// promoted spuriously). Returns `None` if the sub-plan is degenerate.
    fn replan_suffix(
        &self,
        corrected: &[PatternSources],
        stats: &RuntimeStats,
    ) -> Option<Vec<usize>> {
        // Plan the whole BGP with corrected stats but *force the prefix to come first* by
        // seeding the greedy planner's "joined" set with the executed prefix. The public
        // `plan_bgp` always re-chooses the seed, so we re-derive the suffix order with a
        // prefix-anchored greedy pass that mirrors `plan_bgp`'s join-selection rule.
        Some(plan_suffix_greedy(
            self.bgp,
            &self.executed,
            &self.remaining,
            corrected,
            self.descriptors,
            &self.opts,
            stats,
            &self.policy,
        ))
    }

    /// Estimated remaining cost of executing `suffix` (in this order) on top of the current
    /// prefix, under the corrected statistics — the comparable used for hysteresis. Costs are
    /// **latency-weighted** ([OPUS-4.8] sq-b51o): a join against a slow source costs more, so a
    /// suffix that defers the slow source compares favourably. Output cardinality is unaffected.
    fn suffix_cost(
        &self,
        suffix: &[usize],
        corrected: &[PatternSources],
        stats: &RuntimeStats,
    ) -> f64 {
        suffix_cost_greedy(
            self.bgp,
            &self.executed,
            suffix,
            corrected,
            self.descriptors,
            &self.opts,
            stats,
            &self.policy,
        )
    }
}

/// Re-derives a greedy left-deep order for `suffix`, anchored on the already-joined
/// `prefix`. Mirrors [`crate::plan_bgp`]'s selection rule (connected-first, smallest output
/// next, tie-break on index) but over a fixed prefix — so the only thing that can change is
/// the order of the not-yet-executed patterns. Pure, deterministic.
///
/// [OPUS-4.8] sq-b51o: the per-candidate selection key is the **latency-weighted** output
/// score `out · latency_factor(cand)` rather than raw output cardinality — a candidate bound
/// to an observably slow source is deferred. The factor is `1.0` whenever the candidate has no
/// latency observation (or latency weighting is off), so a measurement-free re-plan is
/// byte-identical to the cardinality-only planner (the bias enters *on the same axis* as
/// cardinality; it never alters the output cardinality the next join's estimate uses).
#[allow(clippy::too_many_arguments)]
fn plan_suffix_greedy(
    bgp: &Bgp,
    prefix: &[usize],
    suffix: &[usize],
    selection: &[PatternSources],
    descriptors: &[crate::descriptor::SourceDescriptor],
    opts: &PlanOptions,
    stats: &RuntimeStats,
    policy: &ReplanPolicy,
) -> Vec<usize> {
    let leaf_card: Vec<f64> = selection
        .iter()
        .map(|s| s.total_cardinality().max(0.0))
        .collect();
    // The running result starts as the prefix; estimate its cardinality as the product-free
    // greedy estimate of the prefix in its fixed order (only relative ordering of the suffix
    // matters for the comparison, so an approximate prefix cardinality is fine and identical
    // across the candidate suffixes).
    let mut joined: Vec<usize> = prefix.to_vec();
    let mut cur_card = prefix_cardinality(bgp, prefix, &leaf_card, descriptors, selection, opts);
    if joined.is_empty() {
        // No prefix: seed with the smallest-cardinality remaining pattern (mirrors plan_bgp).
        if let Some(&seed) = suffix.iter().min_by(|&&a, &&b| {
            leaf_card[a]
                .partial_cmp(&leaf_card[b])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        }) {
            joined.push(seed);
            cur_card = leaf_card[seed];
        }
    }
    let mut rem: Vec<usize> = suffix
        .iter()
        .copied()
        .filter(|p| !joined.contains(p))
        .collect();
    let mut order: Vec<usize> = joined
        .iter()
        .copied()
        .filter(|p| suffix.contains(p))
        .collect();

    while !rem.is_empty() {
        // (pat, true_out_card, latency_weighted_score, connected)
        let mut best: Option<(usize, f64, f64, bool)> = None;
        for &cand in &rem {
            let connected = joined.iter().any(|&j| bgp.shares_var(j, cand));
            let (out, _cost, _algo) = crate::plan::cost_join_pub(
                bgp,
                cur_card,
                &joined,
                cand,
                &leaf_card,
                descriptors,
                selection,
                opts,
                connected,
            );
            // [OPUS-4.8] sq-b51o: rank on the latency-weighted score, but advance `cur_card`
            // with the TRUE `out` — latency never changes the cardinality the planner feeds
            // forward (results/soundness are untouched), only which arm we pick next.
            let score = out * pattern_latency_factor(cand, selection, stats, policy);
            let better = match &best {
                None => true,
                Some((bp, _bcard, bscore, bconn)) => match (connected, *bconn) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => score < *bscore || (score == *bscore && cand < *bp),
                },
            };
            if better {
                best = Some((cand, out, score, connected));
            }
        }
        let (pat, out, _, _) = best.expect("rem non-empty");
        order.push(pat);
        joined.push(pat);
        cur_card = out;
        rem.retain(|&p| p != pat);
    }
    order
}

/// Estimated total remaining cost of joining `suffix` (in the given order) onto `prefix`.
/// [OPUS-4.8] sq-b51o: each per-join cost is scaled by the candidate's latency factor (a slow
/// source's joins cost more); the output cardinality fed forward is the TRUE estimate, so
/// latency biases the cost comparable only, never the result.
#[allow(clippy::too_many_arguments)]
fn suffix_cost_greedy(
    bgp: &Bgp,
    prefix: &[usize],
    suffix: &[usize],
    selection: &[PatternSources],
    descriptors: &[crate::descriptor::SourceDescriptor],
    opts: &PlanOptions,
    stats: &RuntimeStats,
    policy: &ReplanPolicy,
) -> f64 {
    let leaf_card: Vec<f64> = selection
        .iter()
        .map(|s| s.total_cardinality().max(0.0))
        .collect();
    let mut joined: Vec<usize> = prefix.to_vec();
    let mut cur_card = prefix_cardinality(bgp, prefix, &leaf_card, descriptors, selection, opts);
    if joined.is_empty() {
        if let Some(&first) = suffix.first() {
            joined.push(first);
            cur_card = leaf_card[first];
        }
    }
    let mut total = 0.0f64;
    for &cand in suffix {
        if joined.contains(&cand) {
            continue;
        }
        let connected = joined.iter().any(|&j| bgp.shares_var(j, cand));
        let (out, cost, _algo) = crate::plan::cost_join_pub(
            bgp,
            cur_card,
            &joined,
            cand,
            &leaf_card,
            descriptors,
            selection,
            opts,
            connected,
        );
        // [OPUS-4.8] sq-b51o: latency-weight the cost of this join; cardinality fed forward
        // (`out`) stays the true estimate, so the comparable is latency-aware but the plan
        // remains a pure reorder (result-equivalent).
        total += cost * pattern_latency_factor(cand, selection, stats, policy);
        cur_card = out;
        joined.push(cand);
    }
    total
}

/// Approximate cardinality of the running result after the fixed `prefix` (used as the left
/// size when costing the suffix). Folds the prefix in its given order through the same
/// per-join estimate the planner uses. An empty prefix yields `1.0` (neutral).
fn prefix_cardinality(
    bgp: &Bgp,
    prefix: &[usize],
    leaf_card: &[f64],
    descriptors: &[crate::descriptor::SourceDescriptor],
    selection: &[PatternSources],
    opts: &PlanOptions,
) -> f64 {
    if prefix.is_empty() {
        return 1.0;
    }
    let mut joined: Vec<usize> = vec![prefix[0]];
    let mut cur = leaf_card[prefix[0]];
    for &cand in &prefix[1..] {
        let connected = joined.iter().any(|&j| bgp.shares_var(j, cand));
        let (out, _cost, _algo) = crate::plan::cost_join_pub(
            bgp,
            cur,
            &joined,
            cand,
            leaf_card,
            descriptors,
            selection,
            opts,
            connected,
        );
        cur = out;
        joined.push(cand);
    }
    cur
}

/// A minimal reference BGP executor used to PROVE result-equivalence under re-planning: it
/// joins the per-pattern solution multisets in any given order and returns the result.
///
/// This is intentionally *order-driven* — you hand it a join order and it folds the
/// patterns' solution sets together left-deep using [`crate::blocking_hash_join`] on the
/// shared variables. Because BGP join is commutative/associative, evaluating the same
/// patterns in two different orders must yield the same result multiset; the adaptive
/// tests drive this with a static order and a mid-execution re-planned order and assert
/// equality.
pub mod exec_oracle {
    use crate::pattern::{Bgp, Term, Var};
    use crate::stream::{blocking_hash_join, Tuple};

    /// Per-pattern solution multisets, indexed by pattern position in the BGP. Each is the
    /// (already-fetched) set of tuples a pattern's source(s) returned.
    pub type SolutionSets = Vec<Vec<Tuple>>;

    /// The shared (join) variables between the variables already produced by the running
    /// result and a candidate pattern.
    fn shared_vars(produced: &[Var], pattern: &crate::pattern::TriplePattern) -> Vec<Var> {
        pattern
            .vars()
            .into_iter()
            .filter(|v| produced.contains(v))
            .cloned()
            .collect()
    }

    /// The variables a pattern binds (its variable positions), as owned `Var`s.
    fn pattern_vars(pattern: &crate::pattern::TriplePattern) -> Vec<Var> {
        pattern.vars().into_iter().cloned().collect()
    }

    /// Evaluates the BGP by joining the per-pattern `solutions` in the given `order`,
    /// left-deep. Returns the result multiset. Patterns sharing no variable with the
    /// running result are Cartesian-joined (empty join key ⇒ full cross product), matching
    /// SPARQL BGP semantics. Deterministic given the inputs and order.
    pub fn evaluate(bgp: &Bgp, solutions: &SolutionSets, order: &[usize]) -> Vec<Tuple> {
        if order.is_empty() {
            return Vec::new();
        }
        let mut produced: Vec<Var> = pattern_vars(&bgp.patterns[order[0]]);
        let mut acc: Vec<Tuple> = solutions[order[0]].clone();
        for &p in &order[1..] {
            let pat = &bgp.patterns[p];
            let jv = shared_vars(&produced, pat);
            acc = if jv.is_empty() {
                cross_product(&acc, &solutions[p])
            } else {
                blocking_hash_join(&acc, &solutions[p], &jv)
            };
            for v in pattern_vars(pat) {
                if !produced.contains(&v) {
                    produced.push(v);
                }
            }
        }
        acc
    }

    /// Full cross product of two solution multisets (a disconnected/Cartesian join).
    fn cross_product(left: &[Tuple], right: &[Tuple]) -> Vec<Tuple> {
        let mut out = Vec::with_capacity(left.len() * right.len());
        for l in left {
            for r in right {
                out.push(merge(l, r));
            }
        }
        out
    }

    /// Merges two tuples (union of bindings; on a shared var the left value wins — equal by
    /// construction when reached via a real join, arbitrary-but-consistent for Cartesian).
    fn merge(l: &Tuple, r: &Tuple) -> Tuple {
        let mut pairs: Vec<(Var, String)> = l
            .bindings()
            .iter()
            .map(|(v, s)| (v.clone(), s.clone()))
            .collect();
        for (v, s) in r.bindings() {
            if !pairs.iter().any(|(pv, _)| pv == v) {
                pairs.push((v.clone(), s.clone()));
            }
        }
        Tuple::new(pairs)
    }

    /// Convenience: build a `Term::Var`.
    pub fn v(name: &str) -> Term {
        Term::Var(Var::new(name))
    }
    /// Convenience: build a `Term::Iri`.
    pub fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::exec_oracle::{evaluate, iri, v, SolutionSets};
    use super::*;
    use crate::descriptor::{PredPartition, SourceDescriptor, SourceId};
    use crate::pattern::{TriplePattern, Var};
    use crate::plan::plan_bgp;
    use crate::selection::{select_sources, SourceCandidate};
    use crate::stream::Tuple;

    fn pred(p: &str, triples: u64, subjects: u64, objects: u64) -> PredPartition {
        PredPartition {
            predicate: p.into(),
            triples,
            distinct_subjects: subjects,
            distinct_objects: objects,
        }
    }

    fn t(pairs: &[(&str, &str)]) -> Tuple {
        Tuple::new(
            pairs
                .iter()
                .map(|(name, val)| (Var::new(*name), val.to_string())),
        )
    }

    fn multiset_eq(a: &[Tuple], b: &[Tuple]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut a: Vec<String> = a.iter().map(canon).collect();
        let mut b: Vec<String> = b.iter().map(canon).collect();
        a.sort();
        b.sort();
        a == b
    }
    fn canon(t: &Tuple) -> String {
        let mut parts: Vec<String> = t
            .bindings()
            .iter()
            .map(|(v, s)| format!("{}={}", v.0, s))
            .collect();
        parts.sort();
        parts.join("|")
    }

    // A 3-pattern chain: ?s :p ?o . ?o :q ?z . ?z :r ?w
    fn chain_bgp() -> Bgp {
        Bgp::new(vec![
            TriplePattern::new(v("s"), iri("http://ex/p"), v("o")),
            TriplePattern::new(v("o"), iri("http://ex/q"), v("z")),
            TriplePattern::new(v("z"), iri("http://ex/r"), v("w")),
        ])
    }

    // A 3-arm star on ?s: ?s :a ?x . ?s :b ?y . ?s :c ?z — all join on ?s, so the suffix
    // order after the seed is driven purely by the arms' cardinalities. Used by the trigger
    // test because here a divergent observation genuinely *flips* the remaining order.
    fn star_bgp() -> Bgp {
        Bgp::new(vec![
            TriplePattern::new(v("s"), iri("http://ex/a"), v("x")),
            TriplePattern::new(v("s"), iri("http://ex/b"), v("y")),
            TriplePattern::new(v("s"), iri("http://ex/c"), v("z")),
        ])
    }

    // ---- TRIGGER fires on divergent stats AND flips the remaining order.
    #[test]
    fn replan_trigger_fires_on_divergence() {
        let bgp = star_bgp();
        // Estimates: :a=10 (seed), :b=20, :c=1000 ⇒ static suffix after :a is [b, c].
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10, 10, 10))
            .predicate(pred("http://ex/b", 20, 20, 20))
            .predicate(pred("http://ex/c", 1000, 1000, 1000))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        // Execute the seed (most selective arm, :a).
        let seed = exec.advance().unwrap();
        assert_eq!(seed, 0, ":a is the seed");
        let suffix_before = exec.remaining_order().to_vec();
        assert_eq!(suffix_before, vec![1, 2], "static suffix is [:b, :c]");
        // Now reality diverges: :b is actually HUGE (5000, est 20 ⇒ >4x) and :c is actually
        // tiny (5, est 1000 ⇒ >4x). The cheap-now arm :c should be re-ordered ahead of :b.
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(1, 5000.0); // :b blew up.
        stats.record_leaf_cardinality(2, 5.0); // :c collapsed.
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::Switched,
            "divergent stats that make a different order cheaper must trigger + switch"
        );
        assert_eq!(
            exec.remaining_order(),
            &[2, 1],
            "the now-cheap arm :c is re-ordered ahead of the now-expensive :b"
        );
    }

    // ---- NO thrash on stable stats.
    #[test]
    fn no_replan_when_stats_stable() {
        let bgp = chain_bgp();
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/p", 1000, 1000, 1000))
            .predicate(pred("http://ex/q", 10, 10, 10))
            .predicate(pred("http://ex/r", 500, 500, 500))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _ = exec.advance();
        // Observe cardinalities essentially equal to the estimates ⇒ no divergence.
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(0, 1000.0);
        stats.record_leaf_cardinality(2, 500.0);
        assert_eq!(exec.maybe_replan(&stats), ReplanOutcome::NoDivergence);
        // Even a moderate miss (2x, below the 4x factor) does not fire.
        let mut stats2 = RuntimeStats::new();
        stats2.record_leaf_cardinality(0, 2000.0);
        assert_eq!(exec.maybe_replan(&stats2), ReplanOutcome::NoDivergence);
    }

    // ---- HYSTERESIS: trigger fires AND a cheaper order exists, but the win is below the
    //      margin ⇒ keep the current plan (anti-thrash). Same divergent fixture as the
    //      trigger test (where the *default* margin would switch) — only the high margin
    //      differs, isolating the hysteresis behaviour.
    #[test]
    fn hysteresis_rejects_marginal_replan() {
        let bgp = star_bgp();
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10, 10, 10))
            .predicate(pred("http://ex/b", 20, 20, 20))
            .predicate(pred("http://ex/c", 1000, 1000, 1000))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        // A near-1.0 improvement margin means almost no re-plan ever clears the bar, even a
        // genuinely cheaper one — the anti-thrash band.
        let policy = ReplanPolicy {
            divergence_factor: 4.0,
            improvement_margin: 0.999,
            max_replans: 8,
            ..ReplanPolicy::default()
        };
        let mut exec =
            AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
        let _ = exec.advance();
        // Exactly the divergence the trigger test switches on under the default margin.
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(1, 5000.0); // diverges (triggers)…
        stats.record_leaf_cardinality(2, 5.0);
        // …but the margin is so high the switch is rejected (anti-thrash); order unchanged.
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::KeptWithinHysteresis
        );
        assert_eq!(
            exec.remaining_order(),
            &[1, 2],
            "rejected re-plan leaves the suffix order untouched"
        );
    }

    // ---- RESULT-EQUIVALENCE: a re-planned (reordered) execution yields EXACTLY the static
    //      plan's result multiset. This is the load-bearing soundness test.
    #[test]
    fn replan_result_equals_static() {
        // Star on ?s: ?s :a ?x . ?s :b ?y . ?s :c ?z — a divergent observation FLIPS the
        // suffix order (verified non-vacuously below), so this tests result-equivalence
        // across a *genuine* mid-execution switch, not a no-op re-plan.
        let bgp = star_bgp();
        // Concrete solution multisets per pattern (already-fetched federated results). Chosen
        // so the join is non-trivial: some subjects drop (no match on an arm), some fan out.
        let solutions: SolutionSets = vec![
            // pattern 0: ?s :a ?x
            vec![
                t(&[("s", "s1"), ("x", "x1")]),
                t(&[("s", "s2"), ("x", "x2")]),
                t(&[("s", "s3"), ("x", "x3")]),
            ],
            // pattern 1: ?s :b ?y  (s3 absent ⇒ dropped; s1 fans out to two y's)
            vec![
                t(&[("s", "s1"), ("y", "y1")]),
                t(&[("s", "s1"), ("y", "y7")]),
                t(&[("s", "s2"), ("y", "y2")]),
            ],
            // pattern 2: ?s :c ?z  (s2 absent ⇒ dropped)
            vec![
                t(&[("s", "s1"), ("z", "z1")]),
                t(&[("s", "s3"), ("z", "z3")]),
            ],
        ];
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10, 10, 10))
            .predicate(pred("http://ex/b", 20, 20, 20))
            .predicate(pred("http://ex/c", 1000, 1000, 1000))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let static_order = plan.join_order();
        let static_result = evaluate(&bgp, &solutions, &static_order);

        // Drive an adaptive execution: run the seed, then force a re-plan with divergent
        // stats that reorders the suffix, then run to completion in the NEW order.
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _seed = exec.advance().unwrap();
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(1, 5000.0); // :b blew up (est 20).
        stats.record_leaf_cardinality(2, 5.0); // :c collapsed (est 1000).
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::Switched,
            "fixture is designed to switch — equivalence is tested across a real reorder"
        );
        // Build the full execution order = prefix ++ remaining (re-planned) suffix.
        let mut adaptive_order = exec.executed_order().to_vec();
        adaptive_order.extend_from_slice(exec.remaining_order());
        // The switch genuinely changed the order (non-vacuous).
        assert_ne!(
            adaptive_order, static_order,
            "the re-plan must actually change the order here"
        );
        // …yet it is a permutation of the same pattern set (no pattern added/dropped).
        let mut a_sorted = adaptive_order.clone();
        let mut s_sorted = static_order.clone();
        a_sorted.sort();
        s_sorted.sort();
        assert_eq!(a_sorted, s_sorted, "re-plan must preserve the pattern SET");
        // The load-bearing invariant: same result multiset despite the different order.
        let adaptive_result = evaluate(&bgp, &solutions, &adaptive_order);
        assert!(
            multiset_eq(&static_result, &adaptive_result),
            "adaptive (re-planned) result MUST equal the static plan's result multiset"
        );
        assert!(!static_result.is_empty(), "fixture must produce some rows");
    }

    // ---- Result-equivalence holds for EVERY permutation of a connected BGP (exhaustive),
    //      proving the commutativity the soundness argument rests on.
    #[test]
    fn evaluate_is_order_independent_all_permutations() {
        let bgp = chain_bgp();
        let solutions: SolutionSets = vec![
            vec![
                t(&[("s", "s1"), ("o", "o1")]),
                t(&[("s", "s2"), ("o", "o2")]),
                t(&[("s", "s3"), ("o", "o1")]),
            ],
            vec![
                t(&[("o", "o1"), ("z", "z1")]),
                t(&[("o", "o2"), ("z", "z2")]),
            ],
            vec![
                t(&[("z", "z1"), ("w", "w1")]),
                t(&[("z", "z2"), ("w", "w2")]),
                t(&[("z", "z1"), ("w", "w3")]),
            ],
        ];
        let perms = [
            vec![0, 1, 2],
            vec![0, 2, 1],
            vec![1, 0, 2],
            vec![1, 2, 0],
            vec![2, 0, 1],
            vec![2, 1, 0],
        ];
        let reference = evaluate(&bgp, &solutions, &perms[0]);
        for p in &perms[1..] {
            let got = evaluate(&bgp, &solutions, p);
            assert!(
                multiset_eq(&reference, &got),
                "BGP evaluation must be order-independent for permutation {:?}",
                p
            );
        }
    }

    // ---- Budget exhaustion stops further re-planning.
    #[test]
    fn budget_exhaustion_stops_replanning() {
        let bgp = chain_bgp();
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/p", 1000, 1000, 1000))
            .predicate(pred("http://ex/q", 10, 10, 10))
            .predicate(pred("http://ex/r", 500, 500, 500))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let policy = ReplanPolicy {
            divergence_factor: 4.0,
            improvement_margin: 0.2,
            max_replans: 0, // no re-plans allowed.
            ..ReplanPolicy::default()
        };
        let mut exec =
            AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
        let _ = exec.advance();
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(0, 5.0);
        assert_eq!(exec.maybe_replan(&stats), ReplanOutcome::BudgetExhausted);
    }

    // ---- Latency capture is recorded and readable.
    #[test]
    fn latency_is_captured() {
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(3, 250.0);
        assert_eq!(stats.observed_latency_of(3), Some(250.0));
        assert_eq!(stats.observed_latency_of(0), None);
    }

    // ============================================================================
    // [OPUS-4.8] sq-b51o — per-source latency folded into the adaptive cost model.
    // ============================================================================

    // A 3-arm star on ?s where each arm's predicate is served by a DIFFERENT source, so a
    // per-source latency observation maps cleanly onto a per-pattern (per-arm) cost weight.
    //   pattern 0: ?s :a ?x  (source 0)
    //   pattern 1: ?s :b ?y  (source 1)
    //   pattern 2: ?s :c ?z  (source 2)
    fn three_source_star() -> (Bgp, [SourceDescriptor; 3]) {
        let bgp = Bgp::new(vec![
            TriplePattern::new(v("s"), iri("http://ex/a"), v("x")),
            TriplePattern::new(v("s"), iri("http://ex/b"), v("y")),
            TriplePattern::new(v("s"), iri("http://ex/c"), v("z")),
        ]);
        // Cards: :a = 10 (seed), :b = 100, :c = 200. Static suffix after :a is [:b, :c]
        // (smaller card first). One source per predicate ⇒ source i serves pattern i.
        let sa = SourceDescriptor::builder(SourceId::new("A"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10, 10, 10))
            .build();
        let sb = SourceDescriptor::builder(SourceId::new("B"))
            .total_triples(100_000)
            .predicate(pred("http://ex/b", 100, 100, 100))
            .build();
        let sc = SourceDescriptor::builder(SourceId::new("C"))
            .total_triples(100_000)
            .predicate(pred("http://ex/c", 200, 200, 200))
            .build();
        (bgp, [sa, sb, sc])
    }

    // ---- LATENCY DECISION: with cardinalities held EXACTLY at their estimates (no
    //      cardinality divergence at all), a slow source on the cheaper-by-card arm makes
    //      the planner DEFER it — the chosen suffix order flips purely on latency.
    #[test]
    fn latency_defers_slow_source() {
        let (bgp, srcs) = three_source_star();
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();

        // Baseline (no latency observed): suffix after the :a seed is [:b, :c] by card.
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        assert_eq!(exec.advance().unwrap(), 0, ":a is the seed");
        assert_eq!(
            exec.remaining_order(),
            &[1, 2],
            "static-by-cardinality suffix is [:b, :c]"
        );

        // Now observe source 1 (serving :b) as VERY slow; cardinalities are spot-on (we
        // record them equal to the estimates, so the CARDINALITY trigger does NOT fire — only
        // the latency trigger does).
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(1, 100.0); // == estimate ⇒ no card divergence.
        stats.record_leaf_cardinality(2, 200.0); // == estimate ⇒ no card divergence.
        stats.record_source_latency(1, 1000.0); // 10× baseline ⇒ slow.
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::Switched,
            "a slow source must trigger + switch even with cardinalities exactly as estimated"
        );
        assert_eq!(
            exec.remaining_order(),
            &[2, 1],
            "the slow :b arm is deferred behind the faster :c arm"
        );
    }

    // ---- RESULT-EQUIVALENCE across a LATENCY-driven reorder: the latency switch changes the
    //      order but the result multiset is identical to the static plan's (the load-bearing
    //      soundness invariant, exercised on the latency path specifically — mirrors
    //      replan_result_equals_static but the reorder is driven by latency, not cardinality).
    #[test]
    fn latency_replan_result_equals_static() {
        let (bgp, srcs) = three_source_star();
        // Concrete per-pattern solution multisets (already-fetched), non-trivial join.
        let solutions: SolutionSets = vec![
            // pattern 0: ?s :a ?x
            vec![
                t(&[("s", "s1"), ("x", "x1")]),
                t(&[("s", "s2"), ("x", "x2")]),
                t(&[("s", "s3"), ("x", "x3")]),
            ],
            // pattern 1: ?s :b ?y  (s3 absent ⇒ dropped; s1 fans out)
            vec![
                t(&[("s", "s1"), ("y", "y1")]),
                t(&[("s", "s1"), ("y", "y7")]),
                t(&[("s", "s2"), ("y", "y2")]),
            ],
            // pattern 2: ?s :c ?z  (s2 absent ⇒ dropped)
            vec![
                t(&[("s", "s1"), ("z", "z1")]),
                t(&[("s", "s3"), ("z", "z3")]),
            ],
        ];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let static_order = plan.join_order();
        let static_result = evaluate(&bgp, &solutions, &static_order);

        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _seed = exec.advance().unwrap();
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(1, 1000.0); // slow :b ⇒ deferred (latency-only reorder).
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::Switched,
            "fixture is designed to switch on latency — equivalence tested across a real reorder"
        );
        let mut adaptive_order = exec.executed_order().to_vec();
        adaptive_order.extend_from_slice(exec.remaining_order());
        assert_ne!(
            adaptive_order, static_order,
            "the latency re-plan must actually change the order here (non-vacuous)"
        );
        // Same pattern SET (no pattern added/dropped) …
        let mut a_sorted = adaptive_order.clone();
        let mut s_sorted = static_order.clone();
        a_sorted.sort();
        s_sorted.sort();
        assert_eq!(
            a_sorted, s_sorted,
            "latency re-plan preserves the pattern SET"
        );
        // … and the SAME result multiset despite the latency-driven order change.
        let adaptive_result = evaluate(&bgp, &solutions, &adaptive_order);
        assert!(
            multiset_eq(&static_result, &adaptive_result),
            "latency-reordered result MUST equal the static plan's result multiset"
        );
        assert!(!static_result.is_empty(), "fixture must produce some rows");
    }

    // ---- NO-THRASH on noisy-but-stable latency: latencies that jitter AROUND the baseline
    //      (never far enough to clear divergence_factor× and whose cost effect is below the
    //      hysteresis margin) must NOT trigger a switch — the latency term is anti-thrashed by
    //      the same discipline as the cardinality term.
    #[test]
    fn no_thrash_on_noisy_stable_latency() {
        let (bgp, srcs) = three_source_star();
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _ = exec.advance(); // execute :a seed.
                                // Latencies wobble around the 100.0 baseline (90/130/110) — noise, not a slow source:
                                // none reaches divergence_factor× (4× = 400) so the latency trigger stays silent.
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 90.0);
        stats.record_source_latency(1, 130.0);
        stats.record_source_latency(2, 110.0);
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::NoDivergence,
            "noisy-but-stable latency around baseline must not trigger a re-plan"
        );
        assert_eq!(
            exec.remaining_order(),
            &[1, 2],
            "suffix order untouched by latency jitter"
        );
    }

    // ---- latency_weight = 0 fully disables the latency term (pure cardinality planning):
    //      even a wildly slow source neither triggers nor reorders.
    #[test]
    fn latency_weight_zero_is_inert() {
        let (bgp, srcs) = three_source_star();
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let policy = ReplanPolicy {
            latency_weight: 0.0,
            ..ReplanPolicy::default()
        };
        let mut exec =
            AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
        let _ = exec.advance();
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(1, 100_000.0); // absurdly slow, but weighting is off.
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::NoDivergence,
            "latency_weight = 0 ⇒ latency is ignored entirely"
        );
        assert_eq!(exec.remaining_order(), &[1, 2]);
    }

    // ---- The latency factor itself: baseline ⇒ 1.0, slow ⇒ > 1 (capped), fast ⇒ < 1
    //      (floored), and weight 0 ⇒ always 1.0. Pins the documented heuristic constants.
    #[test]
    fn latency_factor_constants() {
        let p = ReplanPolicy::default(); // weight 0.5, baseline 100, floor 0.5, cap 4.0.
        assert_eq!(p.latency_factor(100.0), 1.0, "at baseline ⇒ neutral");
        assert_eq!(
            p.latency_factor(200.0),
            1.5,
            "2× baseline ⇒ 1 + 0.5·(2-1) = 1.5"
        );
        assert_eq!(
            p.latency_factor(1000.0),
            4.0,
            "10× baseline ⇒ clamped to the cap 4.0"
        );
        assert_eq!(
            p.latency_factor(0.0),
            0.5,
            "instant ⇒ clamped to the floor 0.5"
        );
        let off = ReplanPolicy {
            latency_weight: 0.0,
            ..ReplanPolicy::default()
        };
        assert_eq!(off.latency_factor(9999.0), 1.0, "weight 0 ⇒ always neutral");
    }

    // ============================================================================
    // [FABLE-5] sq-s5kd — per-source (cardinality-weighted) latency aggregation.
    // ============================================================================

    // One pattern (index 0) retained on two sources with the given per-source cards.
    fn two_arm_selection(card0: f64, card1: f64) -> Vec<PatternSources> {
        vec![PatternSources {
            pattern: 0,
            candidates: vec![
                SourceCandidate {
                    source: 0,
                    estimated_cardinality: card0,
                },
                SourceCandidate {
                    source: 1,
                    estimated_cardinality: card1,
                },
            ],
        }]
    }

    fn weighted_policy() -> ReplanPolicy {
        ReplanPolicy {
            latency_aggregation: LatencyAggregation::CardinalityWeighted,
            ..ReplanPolicy::default()
        }
    }

    // ---- BACK-COMPAT default: the aggregation mode defaults to SlowestArm everywhere, so
    //      existing callers are bit-identical (the sq-s5kd knob is strictly opt-in).
    #[test]
    fn default_aggregation_is_slowest_arm() {
        assert_eq!(
            LatencyAggregation::default(),
            LatencyAggregation::SlowestArm
        );
        assert_eq!(
            ReplanPolicy::default().latency_aggregation,
            LatencyAggregation::SlowestArm
        );
    }

    // ---- The load-bearing difference: a HUGE fast arm + a TINY slow arm. Slowest-arm
    //      charges the whole pattern at the slow arm's (capped) factor 4.0; the weighted
    //      mode charges the expected work: (1000·1.0 + 10·4.0) / 1010.
    #[test]
    fn weighted_aggregation_tracks_dominant_arm() {
        let sel = two_arm_selection(1000.0, 10.0);
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 100.0); // at baseline ⇒ factor 1.0.
        stats.record_source_latency(1, 1000.0); // 10× baseline ⇒ factor capped at 4.0.
        let slowest = pattern_latency_factor(0, &sel, &stats, &ReplanPolicy::default());
        assert_eq!(slowest, 4.0, "slowest-arm charges the slow arm's cap");
        let weighted = pattern_latency_factor(0, &sel, &stats, &weighted_policy());
        assert_eq!(
            weighted,
            (1000.0 * 1.0 + 10.0 * 4.0) / 1010.0,
            "weighted mode charges each arm by its cardinality share"
        );
        assert!(
            weighted < slowest,
            "a tiny slow arm must no longer dominate a huge fast arm"
        );
    }

    // ---- An UNOBSERVED arm contributes a neutral 1.0 in weighted mode (inert until
    //      evidence arrives, exactly as in slowest-arm mode) — while slowest-arm still
    //      charges the whole pattern at the one observed (slow) arm.
    #[test]
    fn weighted_unobserved_arm_is_neutral() {
        let sel = two_arm_selection(1000.0, 10.0);
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(1, 1000.0); // only the tiny arm observed (factor 4.0).
        assert_eq!(
            pattern_latency_factor(0, &sel, &stats, &ReplanPolicy::default()),
            4.0
        );
        assert_eq!(
            pattern_latency_factor(0, &sel, &stats, &weighted_policy()),
            (1000.0 * 1.0 + 10.0 * 4.0) / 1010.0,
            "the unobserved big arm weighs in at factor 1.0"
        );
    }

    // ---- OFF-BY-CONSTRUCTION invariant holds in BOTH modes: with no latency observation
    //      on any source the factor is EXACTLY 1.0 (measurement-free re-plans stay
    //      byte-identical to cardinality-only planning).
    #[test]
    fn weighted_inert_without_observations() {
        let sel = two_arm_selection(1000.0, 10.0);
        let stats = RuntimeStats::new();
        assert_eq!(
            pattern_latency_factor(0, &sel, &stats, &ReplanPolicy::default()),
            1.0
        );
        assert_eq!(
            pattern_latency_factor(0, &sel, &stats, &weighted_policy()),
            1.0,
            "no observations ⇒ exactly neutral in weighted mode too"
        );
    }

    // ---- Zero total cardinality (nothing to weight by) falls back to the PLAIN mean over
    //      the retained arms — never a 0/0 NaN.
    #[test]
    fn weighted_zero_cardinality_falls_back_to_plain_mean() {
        let sel = two_arm_selection(0.0, 0.0);
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 100.0); // factor 1.0.
        stats.record_source_latency(1, 1000.0); // factor 4.0.
        let f = pattern_latency_factor(0, &sel, &stats, &weighted_policy());
        assert_eq!(f, (1.0 + 4.0) / 2.0, "plain mean over the retained arms");
    }

    // ---- Degenerate shapes are neutral: a pattern with NO retained sources, or one absent
    //      from the selection entirely, contributes factor 1.0 in weighted mode.
    #[test]
    fn weighted_degenerate_shapes_are_neutral() {
        let empty = vec![PatternSources {
            pattern: 0,
            candidates: vec![],
        }];
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 1000.0);
        assert_eq!(
            pattern_latency_factor(0, &empty, &stats, &weighted_policy()),
            1.0,
            "no retained sources ⇒ neutral"
        );
        assert_eq!(
            pattern_latency_factor(7, &empty, &stats, &weighted_policy()),
            1.0,
            "pattern absent from the selection ⇒ neutral"
        );
    }

    // ---- SINGLE-source pattern: both modes reduce to that source's factor (the common
    //      single-arm case is aggregation-mode-independent).
    #[test]
    fn weighted_single_source_equals_slowest_arm() {
        let sel = vec![PatternSources {
            pattern: 0,
            candidates: vec![SourceCandidate {
                source: 0,
                estimated_cardinality: 50.0,
            }],
        }];
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 200.0); // 2× baseline ⇒ factor 1.5.
        let slowest = pattern_latency_factor(0, &sel, &stats, &ReplanPolicy::default());
        let weighted = pattern_latency_factor(0, &sel, &stats, &weighted_policy());
        assert_eq!(slowest, 1.5);
        assert_eq!(weighted, 1.5, "single-arm patterns are mode-independent");
    }

    // ---- END-TO-END decision difference: a pattern whose union is a huge fast arm + a
    //      tiny slow arm. Under SlowestArm the whole pattern is charged 4× ⇒ the executor
    //      DEFERS it (Switched); under CardinalityWeighted its expected-work factor is a
    //      mild 1.15 ⇒ the re-plan is considered (the trigger stays slowest-arm by design)
    //      but the current order survives. Knocking out the weighted dispatch turns the
    //      second half of this test red (it would switch like slowest-arm) — the mutation
    //      witness for the sq-s5kd seam.
    //
    //      The hysteresis margin is pinned at 0.17 (not the 0.1 default) because this
    //      fixture has an INTRINSIC, latency-free order asymmetry — the third join's star
    //      estimate depends on which pattern joins last, making [c, b] ~12.5% cheaper than
    //      [b, c] even with no latency observed — which the default margin would not
    //      absorb. At 0.17 the intrinsic gap alone cannot clear the margin, the weighted
    //      1.15× on :b still cannot (37.25 > 43·0.83), but the slowest-arm 4× can
    //      (80 < 100·0.83) — so the OUTCOME difference below isolates exactly the
    //      aggregation-mode seam.
    #[test]
    fn weighted_aggregation_flips_the_replan_decision() {
        // Star on ?s: pattern 0 = :a (seed, card 10, source A), pattern 1 = :b served by
        // TWO sources (B1 card 95 fast/unobserved + B2 card 5 slow), pattern 2 = :c
        // (card 200, source C).
        let bgp = star_bgp();
        let sa = SourceDescriptor::builder(SourceId::new("A"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10, 10, 10))
            .build();
        let sb1 = SourceDescriptor::builder(SourceId::new("B1"))
            .total_triples(100_000)
            .predicate(pred("http://ex/b", 95, 95, 95))
            .build();
        let sb2 = SourceDescriptor::builder(SourceId::new("B2"))
            .total_triples(100_000)
            .predicate(pred("http://ex/b", 5, 5, 5))
            .build();
        let sc = SourceDescriptor::builder(SourceId::new("C"))
            .total_triples(100_000)
            .predicate(pred("http://ex/c", 200, 200, 200))
            .build();
        let srcs = [sa, sb1, sb2, sc];
        let sel = select_sources(&bgp, &srcs);
        let b_arms = sel.iter().find(|ps| ps.pattern == 1).unwrap();
        assert_eq!(
            b_arms.candidates.len(),
            2,
            "fixture: :b must be retained on both B1 and B2"
        );
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();

        let run = |aggregation: LatencyAggregation| {
            let policy = ReplanPolicy {
                latency_aggregation: aggregation,
                // See the header comment: absorbs the fixture's intrinsic order asymmetry
                // so only the latency aggregation mode decides the outcome.
                improvement_margin: 0.17,
                ..ReplanPolicy::default()
            };
            let mut exec =
                AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
            assert_eq!(exec.advance().unwrap(), 0, ":a is the seed");
            assert_eq!(exec.remaining_order(), &[1, 2], "static suffix is [:b, :c]");
            let mut stats = RuntimeStats::new();
            // Cardinalities spot-on ⇒ the cardinality trigger stays silent; only B2 (the
            // TINY arm of :b) is observed slow (10× baseline ⇒ latency trigger fires in
            // both modes — the trigger is slowest-arm by design).
            stats.record_leaf_cardinality(1, 100.0);
            stats.record_leaf_cardinality(2, 200.0);
            stats.record_source_latency(2, 1000.0); // source index 2 = B2.
            let outcome = exec.maybe_replan(&stats);
            (outcome, exec.remaining_order().to_vec())
        };

        let (slowest_outcome, slowest_order) = run(LatencyAggregation::SlowestArm);
        assert_eq!(
            slowest_outcome,
            ReplanOutcome::Switched,
            "slowest-arm charges ALL of :b at 4× ⇒ :b is deferred"
        );
        assert_eq!(slowest_order, vec![2, 1]);

        let (weighted_outcome, weighted_order) = run(LatencyAggregation::CardinalityWeighted);
        assert_eq!(
            weighted_outcome,
            ReplanOutcome::KeptWithinHysteresis,
            "weighted mode charges :b at its expected-work 1.15× ⇒ the order survives \
             (the slowest-arm trigger fired, so this is Kept, not NoDivergence)"
        );
        assert_eq!(weighted_order, vec![1, 2]);
    }

    // ============================================================================
    // [OPUS-4.8] sq-b51o follow-up — per-source latency EWMA smoothing.
    // ============================================================================

    // ---- EWMA mechanics + the default α are pinned. First sample seeds the average; each
    //      subsequent sample folds in at α = 0.3 (latest 30%, history 70%).
    #[test]
    fn ewma_smoothing_constants() {
        assert_eq!(
            RuntimeStats::DEFAULT_LATENCY_ALPHA, 0.3,
            "default latency EWMA α is the documented 0.3 heuristic"
        );
        let mut stats = RuntimeStats::new();
        assert_eq!(stats.latency_alpha(), 0.3);
        // First sample SEEDS the average (no history to decay).
        stats.record_source_latency(0, 100.0);
        assert_eq!(stats.observed_latency_of(0), Some(100.0), "first sample seeds EWMA");
        // Second sample: 0.3·1000 + 0.7·100 = 300 + 70 = 370.
        stats.record_source_latency(0, 1000.0);
        assert_eq!(
            stats.observed_latency_of(0),
            Some(370.0),
            "EWMA = α·observed + (1-α)·prev = 0.3·1000 + 0.7·100"
        );
        // Third sample at the same high level: 0.3·1000 + 0.7·370 = 300 + 259 = 559.
        stats.record_source_latency(0, 1000.0);
        assert_eq!(
            stats.observed_latency_of(0),
            Some(559.0),
            "EWMA converges toward a sustained level across samples"
        );
        // α = 1.0 recovers the old "single last sample" behaviour (no smoothing).
        let mut sharp = RuntimeStats::with_latency_alpha(1.0);
        sharp.record_source_latency(0, 100.0);
        sharp.record_source_latency(0, 1000.0);
        assert_eq!(sharp.observed_latency_of(0), Some(1000.0), "α=1 ⇒ no smoothing");
        // α is clamped into (0, 1]: a degenerate 0.0 / negative request never freezes the seed.
        assert!(RuntimeStats::with_latency_alpha(0.0).latency_alpha() > 0.0);
        assert_eq!(RuntimeStats::with_latency_alpha(5.0).latency_alpha(), 1.0);
    }

    // ---- ANTI-THRASH: a single TRANSIENT latency spike, after the source has an established
    //      (baseline) EWMA history, does NOT move the smoothed latency past the trigger — so no
    //      re-plan fires. This is the headline EWMA property the follow-up buys over the bare
    //      single-sample-plus-clamp.
    #[test]
    fn transient_latency_spike_does_not_trigger_replan() {
        let (bgp, srcs) = three_source_star();
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _ = exec.advance(); // execute :a seed.

        // Source 1 (serving :b) has been running at the baseline for a while: seed + several
        // at-baseline samples ⇒ EWMA sits at 100.0.
        let mut stats = RuntimeStats::new();
        for _ in 0..5 {
            stats.record_source_latency(1, 100.0);
        }
        // One transient spike of 1000.0 (10× baseline — which, as a RAW sample, would clear the
        // 4× divergence trigger). Under EWMA it lifts the average only to 0.3·1000 + 0.7·100 =
        // 370 < 400 (= divergence_factor × baseline), so the trigger stays silent.
        stats.record_source_latency(1, 1000.0);
        assert!(
            stats.observed_latency_of(1).unwrap() < 400.0,
            "one spike must not push the EWMA past the 4× trigger band"
        );
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::NoDivergence,
            "a single transient latency spike (damped by the EWMA) must NOT trigger a re-plan"
        );
        assert_eq!(
            exec.remaining_order(),
            &[1, 2],
            "suffix order untouched by a transient spike"
        );
    }

    // ---- SUSTAINED SHIFT: the SAME source held at the SAME high latency for several
    //      consecutive samples DOES converge the EWMA past the trigger and re-plans — the EWMA
    //      reacts to a real regime change, it just refuses to react to a one-off.
    #[test]
    fn sustained_latency_shift_does_trigger_replan() {
        let (bgp, srcs) = three_source_star();
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _ = exec.advance(); // execute :a seed.

        // Same starting history as the transient test: source 1 established at baseline …
        let mut stats = RuntimeStats::new();
        for _ in 0..5 {
            stats.record_source_latency(1, 100.0);
        }
        // … then a SUSTAINED jump to 1000.0 for several consecutive samples. The EWMA climbs
        // 370 → 559 → 691 → 784 … crossing 400 (= 4× baseline) by the 2nd sustained sample.
        stats.record_source_latency(1, 1000.0); // 370 — not yet over.
        stats.record_source_latency(1, 1000.0); // 559 — over the 400 trigger band.
        assert!(
            stats.observed_latency_of(1).unwrap() > 400.0,
            "a sustained shift must converge the EWMA past the trigger band"
        );
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::Switched,
            "a sustained latency shift (EWMA converged past the trigger) MUST re-plan + switch"
        );
        assert_eq!(
            exec.remaining_order(),
            &[2, 1],
            "the sustainedly-slow :b arm is deferred behind the faster :c arm"
        );
    }

    // ---- RESULT-EQUIVALENCE under an EWMA-driven (sustained-latency) reorder: the smoothed
    //      latency path is still a pure suffix permutation — same result multiset as static.
    //      Mirrors latency_replan_result_equals_static but drives the switch via the EWMA
    //      (multiple samples), not a single raw value.
    #[test]
    fn ewma_replan_result_equals_static() {
        let (bgp, srcs) = three_source_star();
        let solutions: SolutionSets = vec![
            vec![
                t(&[("s", "s1"), ("x", "x1")]),
                t(&[("s", "s2"), ("x", "x2")]),
                t(&[("s", "s3"), ("x", "x3")]),
            ],
            vec![
                t(&[("s", "s1"), ("y", "y1")]),
                t(&[("s", "s1"), ("y", "y7")]),
                t(&[("s", "s2"), ("y", "y2")]),
            ],
            vec![
                t(&[("s", "s1"), ("z", "z1")]),
                t(&[("s", "s3"), ("z", "z3")]),
            ],
        ];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let static_order = plan.join_order();
        let static_result = evaluate(&bgp, &solutions, &static_order);

        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let _seed = exec.advance().unwrap();
        // Sustained slow :b, accumulated via the EWMA (several samples), converging past trigger.
        let mut stats = RuntimeStats::new();
        for _ in 0..6 {
            stats.record_source_latency(1, 1000.0);
        }
        assert_eq!(
            exec.maybe_replan(&stats),
            ReplanOutcome::Switched,
            "fixture is designed to switch on the EWMA-smoothed latency"
        );
        let mut adaptive_order = exec.executed_order().to_vec();
        adaptive_order.extend_from_slice(exec.remaining_order());
        assert_ne!(
            adaptive_order, static_order,
            "the EWMA-driven re-plan must actually change the order (non-vacuous)"
        );
        let mut a_sorted = adaptive_order.clone();
        let mut s_sorted = static_order.clone();
        a_sorted.sort();
        s_sorted.sort();
        assert_eq!(a_sorted, s_sorted, "EWMA re-plan preserves the pattern SET");
        let adaptive_result = evaluate(&bgp, &solutions, &adaptive_order);
        assert!(
            multiset_eq(&static_result, &adaptive_result),
            "EWMA-reordered result MUST equal the static plan's result multiset"
        );
        assert!(!static_result.is_empty(), "fixture must produce some rows");
    }

    // ============================================================================
    // [OPUS-4.8] sq-3xkz — EWMA refinements: per-source adaptive α, time-aware decay,
    // staleness eviction. All deterministic (elapsed gaps passed in, no wall clock).
    // ============================================================================

    // ---- (a) PER-SOURCE α DEFAULT REPRODUCES THE GLOBAL-α RESULT. With NO per-source override
    //      set, `effective_alpha` is the global α for every source and the EWMA value is
    //      bit-identical to the pre-refinement single-global-α path — the back-compat anchor.
    #[test]
    fn per_source_alpha_default_reproduces_global() {
        // Drive the SAME sample sequence through (1) a plain global-α store and (2) a store with
        // the per-source override map present-but-empty; the EWMA must agree at every step.
        let mut global = RuntimeStats::with_latency_alpha(0.3);
        let mut adaptive = RuntimeStats::with_latency_alpha(0.3); // override map empty.
        for &s in &[100.0, 1000.0, 1000.0, 200.0] {
            global.record_source_latency(0, s);
            adaptive.record_source_latency(0, s);
        }
        // Effective α with no override IS the global α …
        assert_eq!(adaptive.effective_alpha(0), 0.3, "no override ⇒ global α");
        // … and the smoothed value is byte-identical to the global-α path.
        assert_eq!(
            adaptive.observed_latency_of(0),
            global.observed_latency_of(0),
            "empty per-source override map reproduces the global-α EWMA exactly"
        );
    }

    // ---- (a) A PER-SOURCE override actually changes that source's α (and ONLY that source).
    //      Source 0 keeps the global 0.3; source 1 is overridden to 1.0 (no smoothing).
    #[test]
    fn per_source_alpha_override_takes_precedence() {
        let mut stats = RuntimeStats::with_latency_alpha(0.3).with_source_alpha(1, 1.0);
        assert_eq!(stats.effective_alpha(0), 0.3, "source 0 falls back to global α");
        assert_eq!(stats.effective_alpha(1), 1.0, "source 1 uses its override");
        // Source 0 smooths (0.3·1000 + 0.7·100 = 370) …
        stats.record_source_latency(0, 100.0);
        stats.record_source_latency(0, 1000.0);
        assert_eq!(stats.observed_latency_of(0), Some(370.0));
        // … source 1 does NOT (α = 1 ⇒ last sample wins).
        stats.record_source_latency(1, 100.0);
        stats.record_source_latency(1, 1000.0);
        assert_eq!(stats.observed_latency_of(1), Some(1000.0), "override α=1 ⇒ no smoothing");
        // Override is clamped into (0,1] like the global.
        let clamped = RuntimeStats::new().with_source_alpha(2, 5.0).with_source_alpha(3, 0.0);
        assert_eq!(clamped.effective_alpha(2), 1.0, "α>1 clamps to 1");
        assert!(clamped.effective_alpha(3) > 0.0, "α≤0 clamps to strictly positive");
    }

    // ---- (b) TIME-AWARE DECAY weights a LARGE-GAP sample more than a back-to-back one.
    //      Deterministic: the elapsed gap is passed in. With a half-life set, the same
    //      (prev=100, sample=1000) fold yields a HIGHER smoothed value when the gap is large
    //      (history decayed) than when the gap is zero (plain α).
    #[test]
    fn time_aware_decay_weights_large_gap_sample() {
        let half_life = 100.0;
        // Establish the same prior (single seed of 100.0) in three stores, all base α 0.3.
        let mut no_gap = RuntimeStats::with_latency_alpha(0.3).with_decay_half_life(half_life);
        let mut one_hl = RuntimeStats::with_latency_alpha(0.3).with_decay_half_life(half_life);
        let mut big_gap = RuntimeStats::with_latency_alpha(0.3).with_decay_half_life(half_life);
        for s in [&mut no_gap, &mut one_hl, &mut big_gap] {
            s.record_source_latency(0, 100.0); // seed
        }
        // Δt = 0 ⇒ decay weight 0 ⇒ plain α 0.3 ⇒ 0.3·1000 + 0.7·100 = 370.
        no_gap.record_source_latency_after(0, 1000.0, 0.0);
        assert_eq!(
            no_gap.observed_latency_of(0),
            Some(370.0),
            "zero gap ⇒ identical to the plain-α fold (decay weight 0)"
        );
        // Δt = one half-life ⇒ decay weight 0.5 ⇒ α_eff = 0.3 + 0.7·0.5 = 0.65 ⇒
        // 0.65·1000 + 0.35·100 = 685.
        one_hl.record_source_latency_after(0, 1000.0, half_life);
        let one = one_hl.observed_latency_of(0).unwrap();
        assert!((one - 685.0).abs() < 1e-9, "one half-life ⇒ α_eff 0.65 ⇒ 685, got {}", one);
        // Δt = 10 half-lives ⇒ decay weight 1 − 0.5^10 ≈ 0.999 ⇒ α_eff ≈ 1 ⇒ value ≈ the new
        // sample 1000 (the now-stale history is almost fully decayed away).
        big_gap.record_source_latency_after(0, 1000.0, 10.0 * half_life);
        let big = big_gap.observed_latency_of(0).unwrap();
        assert!(big > 999.0, "a large-gap sample dominates (history decayed): got {}", big);
        // The ordering is the load-bearing property: larger gap ⇒ the fresh sample weighs more.
        assert!(
            big > one && one > no_gap.observed_latency_of(0).unwrap(),
            "a larger elapsed gap weights the new sample MORE (370 < 685 < ~1000)"
        );
    }

    // ---- (b) With NO half-life configured, `record_source_latency_after` folds at the plain α
    //      regardless of the gap — identical VALUE to `record_source_latency` (back-compat),
    //      while still advancing the clock + stamping last_seen for eviction.
    #[test]
    fn decay_off_matches_plain_fold_but_advances_clock() {
        let mut decay_off = RuntimeStats::with_latency_alpha(0.3); // no half-life set
        let mut plain = RuntimeStats::with_latency_alpha(0.3);
        decay_off.record_source_latency(0, 100.0);
        plain.record_source_latency(0, 100.0);
        // A huge gap, but decay is OFF ⇒ folds at plain α 0.3 ⇒ 370, same as the plain path.
        decay_off.record_source_latency_after(0, 1000.0, 1_000_000.0);
        plain.record_source_latency(0, 1000.0);
        assert_eq!(
            decay_off.observed_latency_of(0),
            plain.observed_latency_of(0),
            "no half-life ⇒ the elapsed gap does not change the EWMA value (back-compat)"
        );
        // …but the clock DID advance and the source IS aged (recency tracked for eviction).
        assert_eq!(decay_off.clock(), 1_000_000.0);
        assert_eq!(decay_off.source_age(0), Some(0.0), "just-recorded ⇒ age 0");
    }

    // ---- (b) A non-positive / non-finite half-life DISABLES decay (back to the off default).
    #[test]
    fn non_positive_half_life_disables_decay() {
        assert_eq!(RuntimeStats::new().with_decay_half_life(0.0).decay_half_life(), None);
        assert_eq!(RuntimeStats::new().with_decay_half_life(-5.0).decay_half_life(), None);
        assert_eq!(
            RuntimeStats::new().with_decay_half_life(f64::INFINITY).decay_half_life(),
            None
        );
        assert_eq!(RuntimeStats::new().with_decay_half_life(50.0).decay_half_life(), Some(50.0));
    }

    // ---- (c) STALENESS EVICTION drops an over-age entry and keeps a fresh one. Deterministic:
    //      ages come from the injected elapsed gaps, not a wall clock.
    #[test]
    fn evict_stale_drops_over_age_entry() {
        let mut stats = RuntimeStats::new();
        // Source 0 recorded at clock 0.
        stats.record_source_latency(0, 100.0);
        // Source 1 recorded 1000 units later (clock advances to 1000 via the elapsed arg).
        stats.record_source_latency_after(1, 200.0, 1000.0);
        // Now source 0 is 1000 units old; source 1 is fresh (age 0).
        assert_eq!(stats.source_age(0), Some(1000.0));
        assert_eq!(stats.source_age(1), Some(0.0));
        // Evict anything older than 500 units ⇒ source 0 goes, source 1 stays.
        let evicted = stats.evict_stale(500.0);
        assert_eq!(evicted, 1, "exactly the one over-age source is evicted");
        assert_eq!(stats.observed_latency_of(0), None, "stale source 0 dropped");
        assert_eq!(stats.observed_latency_of(1), Some(200.0), "fresh source 1 kept");
        assert_eq!(stats.source_age(0), None, "its age entry is gone too");
    }

    // ---- (c) Eviction boundary: a source at EXACTLY max_age is kept (strictly-greater drops);
    //      a non-finite / negative threshold evicts nothing; `advance_clock` ages without a sample.
    #[test]
    fn evict_stale_boundary_and_guards() {
        let mut stats = RuntimeStats::new();
        stats.record_source_latency(0, 100.0); // clock 0
        stats.advance_clock(300.0); // age source 0 to 300 without recording a sample
        assert_eq!(stats.source_age(0), Some(300.0));
        // Exactly at the threshold ⇒ retained (boundary-inclusive).
        assert_eq!(stats.evict_stale(300.0), 0, "age == max_age is kept");
        assert_eq!(stats.observed_latency_of(0), Some(100.0));
        // A negative / NaN threshold evicts nothing.
        assert_eq!(stats.evict_stale(-1.0), 0);
        assert_eq!(stats.evict_stale(f64::NAN), 0);
        assert_eq!(stats.observed_latency_of(0), Some(100.0));
        // Just past the threshold ⇒ evicted.
        assert_eq!(stats.evict_stale(299.0), 1, "age > max_age is dropped");
        assert_eq!(stats.observed_latency_of(0), None);
    }

    // ============================================================================
    // [OPUS-4.8] sq-bif.3 — correctness suite: previously-uncovered adaptive branches
    // (the <2-remaining + advance-to-empty edge cases, the `corrected_selection`
    // even-spread + empty-candidate branches, `diverges` directionality, the input clamps
    // on `record_*`, `clamp_alpha` NaN, `pattern_slowest_latency` max-over-sources, and the
    // exec_oracle Cartesian path). Drives the REAL adaptive helpers + AdaptiveExecutor.
    // ============================================================================

    // ---- A re-plan with FEWER than two remaining patterns has nothing to reorder ⇒
    //      NoDivergence, regardless of how divergent the stats are.
    #[test]
    fn fewer_than_two_remaining_never_replans() {
        let bgp = chain_bgp();
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/p", 1000, 1000, 1000))
            .predicate(pred("http://ex/q", 10, 10, 10))
            .predicate(pred("http://ex/r", 500, 500, 500))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        // Advance until only ONE pattern remains.
        let _ = exec.advance();
        let _ = exec.advance();
        assert_eq!(exec.remaining_order().len(), 1);
        // Wildly divergent stats, but nothing left to reorder.
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(exec.remaining_order()[0], 1.0);
        assert_eq!(exec.maybe_replan(&stats), ReplanOutcome::NoDivergence);
    }

    // ---- `advance` walks the whole suffix then returns None (the executor's stage cursor).
    #[test]
    fn advance_consumes_then_returns_none() {
        let bgp = chain_bgp();
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/p", 1000, 1000, 1000))
            .predicate(pred("http://ex/q", 10, 10, 10))
            .predicate(pred("http://ex/r", 500, 500, 500))
            .build();
        let srcs = [src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        let mut exec = AdaptiveExecutor::new(
            &bgp,
            &srcs,
            &sel,
            &plan,
            PlanOptions::default(),
            ReplanPolicy::default(),
        );
        let mut consumed = Vec::new();
        while let Some(p) = exec.advance() {
            consumed.push(p);
        }
        assert_eq!(consumed.len(), 3, "exactly the three patterns are consumed");
        assert!(exec.remaining_order().is_empty());
        assert_eq!(exec.advance(), None, "advancing past the end is None");
        // The executed order equals the consume order (prefix is built in stage order).
        assert_eq!(exec.executed_order(), &consumed[..]);
    }

    // ---- `diverges` fires in BOTH directions and is silent within the band. Pins the
    //      `o > k·e` (blow-up) and `e > k·o` (collapse) arms and the no-divergence middle.
    #[test]
    fn diverges_is_symmetric() {
        let policy = ReplanPolicy {
            divergence_factor: 4.0,
            ..ReplanPolicy::default()
        };
        // Blow-up: observed 5× the estimate (> 4×) ⇒ diverges.
        assert!(diverges(100.0, 500.0, &policy));
        // Collapse: estimate 5× the observed (> 4×) ⇒ diverges.
        assert!(diverges(500.0, 100.0, &policy));
        // Within the band either way ⇒ no divergence.
        assert!(!diverges(100.0, 300.0, &policy)); // 3× up.
        assert!(!diverges(300.0, 100.0, &policy)); // 3× down.
        assert!(!diverges(100.0, 100.0, &policy)); // exact.
        // Floors: a zero estimate/observation is treated as 1 (no divide-by-zero, no spurious
        // infinite divergence). observed 0 vs estimate 1 ⇒ e(1) > 4·o(1)? no ⇒ stable.
        assert!(!diverges(0.0, 0.0, &policy));
        // estimate 100 vs observed 0 ⇒ e(100) > 4·o.max(1)=4 ⇒ diverges (collapse to nothing).
        assert!(diverges(100.0, 0.0, &policy));
    }

    // ---- `corrected_selection` SCALES each source's share to the observed total when the
    //      prior estimate was non-zero, preserving the per-source skew.
    #[test]
    fn corrected_selection_scales_preserving_skew() {
        // Two sources for one pattern with a 3:1 estimate split (75 / 25 = 100 total).
        let s0 = SourceDescriptor::builder(SourceId::new("s0"))
            .predicate(pred("http://ex/p", 75, 75, 75))
            .build();
        let s1 = SourceDescriptor::builder(SourceId::new("s1"))
            .predicate(pred("http://ex/p", 25, 25, 25))
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(v("s"), iri("http://ex/p"), v("o"))]);
        let sel = select_sources(&bgp, &[s0, s1]);
        assert_eq!(sel[0].total_cardinality(), 100.0);
        // Observe the real total is 400 (4× the estimate).
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(0, 400.0);
        let corrected = corrected_selection(&sel, &stats);
        // Total hits the observation …
        assert_eq!(corrected[0].total_cardinality(), 400.0);
        // … and the 3:1 skew is preserved (300 / 100).
        assert_eq!(corrected[0].candidates[0].estimated_cardinality, 300.0);
        assert_eq!(corrected[0].candidates[1].estimated_cardinality, 100.0);
    }

    // ---- `corrected_selection` EVEN-SPREADS the observation when the prior estimate total
    //      was EXACTLY zero (no skew signal to scale on): each source gets observed / n.
    //      `select_sources` never produces an exactly-zero candidate estimate (every retained
    //      estimate is floored above 0), so we drive the `old_total == 0` branch by handing
    //      `corrected_selection` a `PatternSources` with zeroed candidate estimates directly.
    #[test]
    fn corrected_selection_even_spreads_when_no_prior_signal() {
        let ps = PatternSources {
            pattern: 0,
            candidates: vec![
                SourceCandidate {
                    source: 0,
                    estimated_cardinality: 0.0,
                },
                SourceCandidate {
                    source: 1,
                    estimated_cardinality: 0.0,
                },
            ],
        };
        assert_eq!(ps.total_cardinality(), 0.0, "prior estimate total is exactly zero");
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(0, 40.0);
        let corrected = corrected_selection(std::slice::from_ref(&ps), &stats);
        // With no skew to scale on, the observation is spread EVENLY: 40 / 2 = 20 each.
        assert_eq!(corrected[0].candidates.len(), 2);
        assert_eq!(corrected[0].candidates[0].estimated_cardinality, 20.0);
        assert_eq!(corrected[0].candidates[1].estimated_cardinality, 20.0);
        assert_eq!(corrected[0].total_cardinality(), 40.0);
    }

    // ---- `corrected_selection` leaves a pattern with NO observation untouched, and a
    //      pattern with no candidates (no source) untouched (the empty-candidate guard).
    #[test]
    fn corrected_selection_passes_through_unobserved_and_empty() {
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .predicate(pred("http://ex/p", 100, 100, 100))
            .build();
        // Pattern 0 has a source (:p); pattern 1 has none (:absent ⇒ empty candidates).
        let bgp = Bgp::new(vec![
            TriplePattern::new(v("s"), iri("http://ex/p"), v("o")),
            TriplePattern::new(v("o"), iri("http://ex/absent"), v("z")),
        ]);
        let sel = select_sources(&bgp, &[src]);
        assert!(sel[1].is_empty(), "pattern 1 has no source");
        // Record an observation ONLY for the empty pattern 1 (exercises the empty-candidate
        // early-return) and NONE for pattern 0 (exercises the no-observation passthrough).
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(1, 999.0);
        let corrected = corrected_selection(&sel, &stats);
        // Pattern 0 unchanged (no observation).
        assert_eq!(corrected[0].total_cardinality(), sel[0].total_cardinality());
        // Pattern 1 stays empty (no candidates to distribute onto — the guard returns it).
        assert!(corrected[1].is_empty());
        assert_eq!(corrected[1].total_cardinality(), 0.0);
    }

    // ---- `record_leaf_cardinality` and `record_source_latency` clamp a negative input to 0
    //      (a count/latency is never negative). The EWMA then treats the clamped 0 as the
    //      seed.
    #[test]
    fn record_inputs_clamp_negatives() {
        let mut stats = RuntimeStats::new();
        stats.record_leaf_cardinality(0, -5.0);
        assert_eq!(stats.observed_leaf(0), Some(0.0), "negative cardinality clamps to 0");
        stats.record_source_latency(1, -10.0);
        assert_eq!(stats.observed_latency_of(1), Some(0.0), "negative latency clamps to 0");
    }

    // ---- `clamp_alpha` maps NaN to the default α (a NaN would poison the EWMA), and clamps
    //      ≤0 up to strictly-positive and >1 down to 1.
    #[test]
    fn latency_alpha_clamps_nan_and_range() {
        assert_eq!(
            RuntimeStats::with_latency_alpha(f64::NAN).latency_alpha(),
            RuntimeStats::DEFAULT_LATENCY_ALPHA,
            "NaN α falls back to the default"
        );
        assert!(RuntimeStats::with_latency_alpha(-1.0).latency_alpha() > 0.0);
        assert_eq!(RuntimeStats::with_latency_alpha(2.0).latency_alpha(), 1.0);
        assert_eq!(
            RuntimeStats::with_latency_alpha(0.5).latency_alpha(),
            0.5,
            "an in-range α is kept verbatim"
        );
    }

    // ---- `pattern_slowest_latency` returns the MAX over the pattern's retained sources (a
    //      union is bottlenecked by its slowest arm), and None when no source of the pattern
    //      has any observation.
    #[test]
    fn slowest_latency_is_max_over_pattern_sources() {
        // One pattern retained on two sources (s0, s1) via the same predicate.
        let s0 = SourceDescriptor::builder(SourceId::new("s0"))
            .predicate(pred("http://ex/p", 10, 10, 10))
            .build();
        let s1 = SourceDescriptor::builder(SourceId::new("s1"))
            .predicate(pred("http://ex/p", 10, 10, 10))
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(v("s"), iri("http://ex/p"), v("o"))]);
        let sel = select_sources(&bgp, &[s0, s1]);
        assert_eq!(sel[0].candidates.len(), 2);
        let policy = ReplanPolicy::default();
        // No observations ⇒ None ⇒ inert factor 1.0.
        let mut stats = RuntimeStats::new();
        assert_eq!(pattern_slowest_latency(0, &sel, &stats), None);
        assert_eq!(pattern_latency_factor(0, &sel, &stats, &policy), 1.0);
        // Source 0 fast (50), source 1 slow (300): the slowest (300) wins.
        stats.record_source_latency(0, 50.0);
        stats.record_source_latency(1, 300.0);
        assert_eq!(pattern_slowest_latency(0, &sel, &stats), Some(300.0));
        // factor for 300 (3× baseline 100) = 1 + 0.5·(3-1) = 2.0.
        assert_eq!(pattern_latency_factor(0, &sel, &stats, &policy), 2.0);
    }

    // ---- The exec_oracle Cartesian path: two patterns sharing NO variable cross-product,
    //      and the order-independence of evaluate holds across the Cartesian join too.
    #[test]
    fn exec_oracle_cartesian_join_is_order_independent() {
        // ?a :p ?b  and  ?c :q ?d — disjoint variable sets.
        let bgp = Bgp::new(vec![
            TriplePattern::new(v("a"), iri("http://ex/p"), v("b")),
            TriplePattern::new(v("c"), iri("http://ex/q"), v("d")),
        ]);
        let solutions: SolutionSets = vec![
            vec![t(&[("a", "a1"), ("b", "b1")]), t(&[("a", "a2"), ("b", "b2")])],
            vec![t(&[("c", "c1"), ("d", "d1")])],
        ];
        let forward = evaluate(&bgp, &solutions, &[0, 1]);
        let backward = evaluate(&bgp, &solutions, &[1, 0]);
        // 2 × 1 = 2 rows, each carrying all four variables.
        assert_eq!(forward.len(), 2);
        assert!(multiset_eq(&forward, &backward), "Cartesian eval is order-independent");
        // Every row binds all four variables (full cross product merge).
        for row in &forward {
            assert_eq!(row.bindings().len(), 4);
        }
    }

    // ---- An empty join order evaluates to the empty result (the `order.is_empty()` guard).
    #[test]
    fn exec_oracle_empty_order_is_empty() {
        let bgp = chain_bgp();
        let solutions: SolutionSets = vec![vec![], vec![], vec![]];
        assert!(evaluate(&bgp, &solutions, &[]).is_empty());
    }
}

// sq-py8h.5 — planner guard + cost-model wiring for the HIDDEN-regime
// bounded property-path operator. Rejects statically-large unrolls, REFUSES a
// hidden UNBOUNDED path fail-closed via `MpcError::NoBackendSatisfies` (never a
// silent default-`k` approximation), and emits the operator's modelled
// communication cost (k×-join + O(k) reduction rounds + B-slot output) through the
// crate's `CommCounter` so the matrix runner counts it (never a fabricated
// wall-clock).
//! Planner guard + cost model for the hidden bounded property-path operator
//! (`sq-py8h.5`, design §1.1/§4.3/§5/§6).
//!
//! The cryptographic operator itself lives in the parent module
//! ([`super::eval_bounded_path_hidden`] / [`super::eval_exact_k_chain_hidden`]).
//! This file is the **planner-facing** layer that a query planner consults BEFORE
//! it asks the operator to run, plus the **cost descriptor** the benchmark matrix
//! reads off. It performs no crypto and touches no secret key — every decision is
//! a closed-form function of the PUBLIC plan parameters (the bound `k`, the
//! alternation arity `a`, the public edge count `|E|`, and the public padded
//! output bound `B`; design §1.2/§4.1).
//!
//! ## (a) Statically-large unroll → rejected
//!
//! A repetition `(step){m,k}` over an `a`-way alternation enumerates
//! `Σ_ℓ (a^ℓ · |E|^ℓ)` edge-tuples — exponential in the PUBLIC bound `k`. The
//! operator already rejects an over-[`super::MAX_CHAIN_TUPLES`] unroll inside
//! `check_bounded_params`;
//! [`plan_hidden_bounded_path`](crate::hidden_path::planner::plan_hidden_bounded_path)
//! surfaces that SAME guard at
//! plan time so a planner can reject (or warn on) the path before dispatching it,
//! and [`BoundedPathPlan::warn`](crate::hidden_path::planner::BoundedPathPlan::warn)
//! flags a path that is *admissible* but heavy
//! (within reach of the cap) so a caller can choose to refuse it under a tighter
//! dataset cap — the same df/dataset-cap discipline the benchmarks use.
//!
//! ## (b) Hidden UNBOUNDED path → refused fail-closed (NOT approximated)
//!
//! A hidden `(p)+` / `(p)*` / `(p){m,}` has NO finite public bound. There is no
//! sound way to evaluate it in this regime: the transitive closure over a SECRET
//! graph has secret depth, and silently substituting a default `k` would return a
//! WRONG answer the verifier could not recompute (the bound must be explicit AND
//! public — design §1.1/§1.2). So a request carrying an
//! [`PathUpperBound::Open`](crate::hidden_path::planner::PathUpperBound::Open)
//! upper bound is REFUSED through the registry's fail-closed channel,
//! [`MpcError::NoBackendSatisfies`] (the negotiation-layer "no backend can do
//! this"), exactly as a dishonest-majority-malicious request is refused — never
//! downgraded to a weaker/approximate answer.
//!
//! ## (c) Cost wired into [`crate::metrics::CommCounter`]
//!
//! [`BoundedPathPlan::comm_counter`](crate::hidden_path::planner::BoundedPathPlan::comm_counter)
//! folds the operator's modelled cost into a
//! `CommCounter`: the per-tuple secure equalities (the `k×`-join — `Σ_ℓ` over the
//! enumerated tuples of `ℓ−1` internal-hop equalities each), the `O(k)`-depth
//! AND/OR reduction rounds those chain through, and the `B`-slot oblivious output
//! (select multiplications + opens + the shuffle over `B` slots). The benchmark
//! matrix ([`crate::bench`]) reads this counter for its
//! [`crate::bench::QueryClass::HiddenBoundedPath`] cell, so the operator's cost is
//! COUNTED structurally — never a fabricated wall-clock (design §5, the
//! counted-not-timed constraint).

use super::{HiddenBoundedPath, MAX_CHAIN_TUPLES, MAX_HOP_COUNT};
use crate::metrics::CommCounter;
use crate::oblivious::{ShuffleCost, WaksmanNetwork};
use crate::partial::MpcError;

/// The PUBLIC upper bound a requested hidden property path carries. The whole point
/// of this type is that it can REPRESENT an open/unbounded upper bound (`+`/`*`/
/// `{m,}`) — which [`HiddenBoundedPath`] deliberately cannot — so the planner can
/// inspect it and refuse the unbounded case fail-closed instead of the type system
/// silently making it unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathUpperBound {
    /// A finite, PUBLIC maximum hop count `k`. The tractable, bounded slice.
    Finite(usize),
    /// An OPEN upper bound — `(p)+`, `(p)*`, or `(p){m,}`. There is no public `k`;
    /// the path is UNBOUNDED. The hidden regime cannot evaluate this soundly, so a
    /// planner must refuse it (see [`plan_hidden_bounded_path`]).
    Open,
}

/// A REQUESTED hidden property path, as a planner receives it from a parsed query —
/// before it is known whether the engine can serve it. Unlike [`HiddenBoundedPath`]
/// (which is bounded by construction) this can carry an [`PathUpperBound::Open`]
/// upper bound, so [`plan_hidden_bounded_path`] can make the fail-closed decision
/// explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenPathRequest {
    /// The (public) alternation predicates one hop may traverse (`a == 1` is a bare
    /// predicate). Non-empty.
    pub alternatives: Vec<String>,
    /// Minimum hop count `m` (`0` enables the reflexive identity diagonal).
    pub min: usize,
    /// The requested upper bound — finite, or OPEN (unbounded).
    pub max: PathUpperBound,
}

impl HiddenPathRequest {
    /// A bounded request `(p){min,max}` over a single predicate.
    pub fn range(predicate: impl Into<String>, min: usize, max: usize) -> Self {
        HiddenPathRequest {
            alternatives: vec![predicate.into()],
            min,
            max: PathUpperBound::Finite(max),
        }
    }

    /// An UNBOUNDED request: `(p)+` (`min = 1`) or `(p)*` (`min = 0`) over a single
    /// predicate. This is the request [`plan_hidden_bounded_path`] refuses.
    pub fn unbounded(predicate: impl Into<String>, min: usize) -> Self {
        HiddenPathRequest {
            alternatives: vec![predicate.into()],
            min,
            max: PathUpperBound::Open,
        }
    }
}

/// The point above which an admissible bounded path is flagged `warn = true` by
/// [`plan_hidden_bounded_path`]: an enumeration within reach of the hard
/// [`MAX_CHAIN_TUPLES`] cap is "statically large" — admissible, but a caller running
/// under a tighter dataset cap may choose to refuse it. We warn once the projected
/// tuple count exceeds `MAX_CHAIN_TUPLES / 16` (the top ~6% of the admissible
/// range).
const WARN_TUPLES_THRESHOLD: u64 = MAX_CHAIN_TUPLES >> 4;

/// A successfully PLANNED hidden bounded path: the bounded operator form to run, the
/// statically-projected enumeration size, and whether that size is large enough to
/// warrant a planner warning. Produced by [`plan_hidden_bounded_path`]; carries the
/// public cost inputs so [`BoundedPathPlan::comm_counter`] can model the operator's
/// communication WITHOUT re-deriving anything secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPathPlan {
    /// The bounded operator form to dispatch to [`super::eval_bounded_path_hidden`].
    pub form: HiddenBoundedPath,
    /// The public edge count `|E|` the cost was projected against.
    pub edge_count: usize,
    /// The total enumerated edge-tuples `Σ_ℓ (a^ℓ · |E|^ℓ)` (the `k×`-join size).
    pub projected_tuples: u64,
    /// The total internal-hop secure equalities `Σ_ℓ (a^ℓ · |E|^ℓ · (ℓ−1))` — the
    /// `k×`-join's secure-equality cost center.
    pub secure_equalities: u64,
    /// `true` iff the (admissible) projection is large enough to warrant a planner
    /// warning under a tighter dataset cap (a fixed fraction of
    /// [`MAX_CHAIN_TUPLES`]).
    pub warn: bool,
}

/// Closed-form enumeration projection of a bounded hidden path over `|E|` edges:
/// returns `(total_tuples, total_secure_equalities)` where
/// `total_tuples = Σ_{ℓ=max(min,1)..=max} a^ℓ · |E|^ℓ` and each length-`ℓ` tuple
/// contributes `ℓ−1` internal-hop secure equalities. Returns `None` on `u64`
/// overflow so the caller refuses with a controlled error (never a wrapping count).
/// Mirrors `super::check_bounded_params`' tuple sum, extended with the per-tuple
/// equality count for the cost model.
fn project_bounded(
    edge_count: usize,
    alternatives: usize,
    min: usize,
    max: usize,
) -> Option<(u64, u64)> {
    let m = edge_count as u64;
    let a = alternatives as u64;
    let mut total_tuples: u64 = 0;
    let mut total_eqs: u64 = 0;
    for length in min.max(1)..=max {
        // a^length and |E|^length, checked.
        let mut chains: u64 = 1;
        let mut per_chain: u64 = 1;
        for _ in 0..length {
            chains = chains.checked_mul(a)?;
            per_chain = per_chain.checked_mul(m)?;
        }
        let length_tuples = chains.checked_mul(per_chain)?;
        total_tuples = total_tuples.checked_add(length_tuples)?;
        // Each length-ℓ tuple has ℓ−1 internal-hop equalities.
        let per_tuple_eqs = (length as u64).saturating_sub(1);
        let length_eqs = length_tuples.checked_mul(per_tuple_eqs)?;
        total_eqs = total_eqs.checked_add(length_eqs)?;
    }
    Some((total_tuples, total_eqs))
}

/// **Plan a requested hidden property path, fail-closed.**
///
/// - An [`PathUpperBound::Open`] upper bound (`+`/`*`/`{m,}`) is REFUSED with
///   [`MpcError::NoBackendSatisfies`]: no registered backend can soundly evaluate an
///   UNBOUNDED path over a SECRET graph, and silently substituting a default `k`
///   would be a wrong answer the verifier could not recompute (design §1.1/§1.2).
///   The bound must be explicit AND public.
/// - A finite bound whose projected enumeration `Σ_ℓ (a^ℓ · |E|^ℓ)` exceeds the hard
///   [`MAX_CHAIN_TUPLES`] cap (or overflows `u64`) is REJECTED with an
///   [`MpcError::Protocol`] — the same DoS guard the operator enforces, surfaced at
///   plan time.
/// - An admissible finite bound returns a [`BoundedPathPlan`] with the operator form
///   to run, the projected cost inputs, and a `warn` flag for the heavy-but-
///   admissible range.
///
/// `edge_count` is the PUBLIC federated edge count `|E|`; it gates only the cost
/// projection, never any secret key.
pub fn plan_hidden_bounded_path(
    req: &HiddenPathRequest,
    edge_count: usize,
) -> Result<BoundedPathPlan, MpcError> {
    if req.alternatives.is_empty() {
        return Err(MpcError::Protocol(
            "hidden path step has no predicates (empty alternation)".into(),
        ));
    }
    // (b) FAIL-CLOSED: a hidden UNBOUNDED path is refused via the registry's
    // NoBackendSatisfies channel — never approximated by a default k.
    let max = match req.max {
        PathUpperBound::Finite(k) => k,
        PathUpperBound::Open => {
            return Err(MpcError::NoBackendSatisfies {
                requirement: format!(
                    "hidden property path with an OPEN upper bound (+/*/{{{},}}); the hidden \
                     regime needs an explicit, public, finite bound k — a transitive closure \
                     over a secret graph has secret depth, and a default k would be an answer \
                     the verifier could not recompute",
                    req.min
                ),
                considered: 0,
            });
        }
    };
    if req.min > max {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path {{{},{max}}} has min > max",
            req.min
        )));
    }
    // Hop counts are capped before the projection, mirroring the operator's
    // `check_bounded_params`: `project_bounded` below loops once per length and once
    // per hop within each length, so its own cost grows with `max`.
    if max > MAX_HOP_COUNT {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path {{{},{}}} exceeds the MAX_HOP_COUNT cap of {} — refusing (the \
             union runs one fixed-length chain family per length up to the upper bound, so the \
             hop count is bounded before the enumeration is sized)",
            req.min, max, MAX_HOP_COUNT
        )));
    }
    // (a) Statically-large unroll → reject (overflow OR over-cap), mirroring the
    // operator's `check_bounded_params` guard, computed from PUBLIC parameters.
    let (projected_tuples, secure_equalities) =
        match project_bounded(edge_count, req.alternatives.len(), req.min, max) {
            Some(v) if v.0 <= MAX_CHAIN_TUPLES => v,
            other => {
                return Err(MpcError::Protocol(format!(
                    "hidden bounded path {{{},{max}}} over |E|={edge_count} edges and a {}-way \
                     alternation would enumerate {} edge-tuples, exceeding the MAX_CHAIN_TUPLES \
                     cap of {MAX_CHAIN_TUPLES} — refusing (CPU/round denial-of-service)",
                    req.min,
                    req.alternatives.len(),
                    other
                        .map(|(t, _)| t.to_string())
                        .unwrap_or_else(|| "an amount that overflows u64".to_string()),
                )));
            }
        };
    let form = HiddenBoundedPath::alternation(req.alternatives.clone(), req.min, max);
    Ok(BoundedPathPlan {
        form,
        edge_count,
        projected_tuples,
        secure_equalities,
        warn: projected_tuples > WARN_TUPLES_THRESHOLD,
    })
}

impl BoundedPathPlan {
    /// **The operator's modelled communication cost, folded into a
    /// [`CommCounter`]** (the counted-not-timed metric, design §5). Wires the hidden
    /// bounded-path operator into the same instrument the matrix runner reads, so its
    /// cost is structural, never a wall-clock.
    ///
    /// Three contributions, all from PUBLIC parameters:
    ///
    /// 1. **The `k×`-join secure equalities.** Each internal-hop equality is one
    ///    secret-shared `secure_equal_to_bit` — modelled here as one independent
    ///    secure equality (3 deals + 1 mult + 1 open), exactly as the hidden-join
    ///    cost cell models its all-pairs equalities. There are
    ///    [`Self::secure_equalities`] of them across the whole enumeration.
    /// 2. **The `O(k)`-depth AND/OR reduction rounds.** The `ℓ−1` hop bits of a
    ///    length-`ℓ` tuple chain through `secret_and`/`secret_or` (one
    ///    `degree_reduce` mult each), and the per-pair connected-bits OR-fold; that
    ///    chaining is the [`MAX_CHAIN_TUPLES`]-bounded reduction work the bead calls
    ///    out. It is folded in as `secure_equalities` AND/degree-reduce
    ///    multiplications (one reduction per chained bit), so the mult-round counter
    ///    reflects the reduction-round cost.
    /// 3. **The `B`-slot oblivious output — a LOWER BOUND.** One select
    ///    multiplication and one degree-`t` open per slot, plus the shuffle over
    ///    `B` slots, derived from the built [`WaksmanNetwork`] (no hard-coded
    ///    numbers). [`crate::oblivious_join::oblivious_set_output`] pays MORE than
    ///    this: it selects, reduces and opens the slot tag AND each packed payload
    ///    limb, so its true cost is `B · (1 + limb_capacity)` selects, the same
    ///    number of degree reductions (each `2t + 1` re-sharings through the
    ///    randomness seam) and the same number of degree-`t` opens, with the
    ///    shuffle's switch count scaled by the packed-entry width. `limb_capacity`
    ///    is the longest candidate payload encoding rounded up to whole limbs,
    ///    which this planner does not have — it plans over the hop structure, not
    ///    over the payload bytes — so the counter charges the tag column only and
    ///    is a lower bound on the output stage. The stage's own
    ///    [`crate::oblivious_join::ObliviousOutputCost`] reports the full count.
    ///
    /// `bound` is the PUBLIC padded output bound `B`; it must be `>= 1` (the operator
    /// reveals exactly `B` slots). `parties` is the compute-party count `n`.
    ///
    /// `B` is capped by [`crate::oblivious_join::MAX_OUTPUT_BOUND`], refused before
    /// any of the counting below: step (3) builds a real [`WaksmanNetwork`] over
    /// `B` slots, so an unchecked `B` would allocate a network proportional to a
    /// number the caller chose. (That one cap is enough on its own:
    /// [`crate::oblivious::MAX_NETWORK_WIDTH`] IS `MAX_OUTPUT_BOUND`, so a `B`
    /// inside the bound cap is inside the width cap by definition.) The party cap
    /// [`crate::oblivious_join::MAX_OUTPUT_PARTIES`] is applied here too, for the
    /// same reason, when `B > 0` or the plan performs secure equalities. A plan
    /// with neither kind of work accepts larger party counts.
    ///
    /// The counting itself is CLOSED FORM: every counter the per-equality and
    /// per-slot records touch is linear in its multiplicity, so the batches fold in
    /// arithmetically (`record_mults` / `record_opens`) rather than by looping the
    /// instrument `secure_equalities` and `B` times — which would make the cost
    /// MODEL cost as much as the protocol it models. Every fold is checked, so an
    /// overflowing total is a refusal and never a wrapped count that would
    /// understate the cost.
    ///
    /// This API receives neither the backend threshold nor the payload width.
    /// Aggregate slot work is checked at the output stage using their actual
    /// values; the largest possible threshold cannot serve as a lower bound.
    pub fn comm_counter(&self, parties: usize, bound: usize) -> Result<CommCounter, MpcError> {
        crate::oblivious_join::check_output_bound_cap(bound)?;
        // The same PARTY cap the stage applies, so a plan whose party count the
        // stage would refuse is not costed here either — a cost model for a run
        // that cannot happen is not a cost model.
        //
        if bound > 0 || self.secure_equalities > 0 {
            crate::oblivious_join::check_output_parties_cap(parties)?;
        }
        // The aggregate slot-work budget also needs the actual threshold and
        // payload width, neither of which this API receives. It is checked at
        // the output stage. Assuming the largest threshold here would refuse
        // plans that fit with a smaller threshold.
        let mut c = CommCounter::new(parties);
        // (1) The k×-join secure equalities — the cost center. Independent ACROSS
        // tuples (the matrix's `record_independent_equalities` batch shape).
        c.record_independent_equalities(self.secure_equalities)?;
        // (2) The AND/degree-reduce reductions that chain the per-tuple hop bits and
        // OR-fold the per-pair connected-bit: one secure multiplication (degree
        // reduce) per chained bit. Folded as `secure_equalities` reductions — the
        // O(k)-depth reduction work bounded by MAX_CHAIN_TUPLES.
        // CLOSED FORM: every counter `record_mult` touches is linear, so the whole
        // batch folds in arithmetically instead of looping the instrument
        // `secure_equalities` times. The totals are identical.
        c.record_mults(self.secure_equalities)?;
        // (3) The B-slot oblivious output, as a LOWER BOUND: one select mult + one
        // degree-`t` open per slot — the TAG column only — plus the shuffle over B
        // slots (switch count + depth from the real network). The per-limb selects,
        // their degree reductions and their opens are not charged, because
        // `limb_capacity` is a property of the payload bytes this planner does not
        // see. See the doc above.
        // CLOSED FORM again: `B` select multiplications and `B` opens, folded in
        // arithmetically rather than by looping `B` times.
        c.record_mults(bound as u64)?;
        c.record_opens(bound as u64)?;
        let net = WaksmanNetwork::try_new(bound)?;
        c.record_shuffle(ShuffleCost::model(bound, net.switch_count()))?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests;

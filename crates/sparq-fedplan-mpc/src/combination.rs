//! **Phase 6** — FedUP-style result-aware source-**combination** pruning (design record
//! §3 / §8 Phase 6; bead sq-pwr.3, epics sq-pwr / sq-0jsc).
//!
//! Phase 2 ([`crate::selection`]) keeps each `(pattern, source)` candidate **in isolation**:
//! it answers "could this source contribute to *this pattern*?". A federated plan, however,
//! executes a **conjunction** of patterns, and the seam's secure-join cost grows with the
//! number of source-**combinations** (one source per pattern, the cross-product over the
//! per-pattern candidate sets) it must consider. FedUP (WWW'24) is the prior art for
//! *result-aware* plans: prune source-combinations that **provably contribute no answer**
//! before any execution. This pass is the seam's pre-MPC analogue — it refines the Phase-2
//! selection by surfacing which combinations are dead for a full-BGP answer, so the seam
//! feeds fewer combinations toward the (expensive) MPC path.
//!
//! # The recall-safe, summary-expressible combination prunes (and the honest non-rule)
//!
//! The load-bearing question is *what* a result-aware combination prune can prove from the
//! served descriptor **summaries** (no live data) that the per-pattern selector cannot already
//! prove. The honest answer is two rules plus a deliberately-declined non-rule:
//!
//! * **Rule C1 — unsatisfiable-conjunct collapse.** A full-BGP answer is one variable
//!   assignment satisfying **every** conjunct simultaneously. If **any** pattern's Phase-2
//!   candidate list is empty, then — by the upstream HiBISCuS recall-safety invariant — *no*
//!   retained source can contribute any binding for that conjunct, so the conjunct matches the
//!   **empty relation**. An empty conjunct makes the whole conjunction empty (`∅ ⋈ R = ∅`),
//!   so **zero** full-BGP answers exist and **every** source-combination is dead. This is
//!   genuinely combination-level: per-pattern selection records the one pattern as empty but
//!   still hands the join planner the *other* patterns' full candidate lists, which would be
//!   multiplied into source-combinations the seam would route toward MPC. This pass is the
//!   first place that can prove those candidates are dead **for a full-BGP answer** and
//!   collapse the combination space to zero.
//!
//! * **Rule C2 — dead same-source star pairing (the FedUP quotient-summary rule).** This is the
//!   rule that fires on the **live** path, i.e. the one that actually cuts the combination
//!   blow-up of a *satisfiable* query. A source's served **characteristic sets**
//!   ([`CharSet`]) are exactly a *quotient summary* of its subjects: each set is a distinct
//!   **exact** per-subject predicate set with the number of subjects carrying it, so the sets
//!   **partition** the source's subjects. Now take two patterns `i`, `j` that share a variable
//!   in **subject** position with bound predicates `p_i`, `p_j`, and a source-combination that
//!   assigns **both** to the *same* source `S`. Such a combination can only yield an answer if
//!   some subject *in `S`* carries **both** `p_i` and `p_j` — i.e. if some characteristic set of
//!   `S` contains both. If none does, that pairing is **provably dead**, and every
//!   source-combination containing it (there are `Π` of them over the other patterns) is dead
//!   with it. This is the *provenance* half of FedUP: the combination, not the per-pattern
//!   candidate, is what is pruned — `S` stays a perfectly good candidate for `i` *and* for `j`
//!   individually, because a combination that takes `i` from `S` and `j` from a **different**
//!   source is untouched.
//!
//!   **The completeness guard (why this is recall-safe).** Served characteristic sets are
//!   routinely **truncated** (the long tail is elided — `sparq-introspect`'s `max_char_sets`),
//!   and a missing set would make "no set contains both" a lie. The rule therefore fires only
//!   when the descriptor's **own accounting proves the sets are complete for each predicate
//!   involved**: because the sets partition subjects, `Σ_{C ∋ p} C.subjects` must equal
//!   `void:distinctSubjects` of `p`'s predicate partition. Truncation strictly *lowers* that
//!   sum (an elided set has `subjects > 0` by construction), so the equality fails and the rule
//!   declines. A predicate whose `distinct_subjects` was not served (`0` = unknown) also
//!   declines. The guard is **per predicate**, so eliding sets that do not mention `p` leaves
//!   `p`'s accounting exact — the rule stays available exactly where it is provable.
//!
//! * **The declined non-rule — value-overlap / bound-IRI propagation.** A tempting idea is to
//!   propagate a bound IRI constant across a shared join variable and prune a source whose
//!   value space cannot reach it. This is **not** recall-safely expressible from the public
//!   [`SourceDescriptor`] API, and declining it is a *correctness* decision, not an omission:
//!   (a) a constant in an object/subject *position* is already tested per-source by the
//!   upstream authority prune ([`SourceDescriptor::may_hold_authority`], `select_sources`
//!   Rule 2) — there is nothing left for a combination pass to add for a bound position; and
//!   (b) a join *variable* is bound to data-dependent resource values unknown at plan time, so
//!   treating an object-position constant as a binding on the join variable would be unsound,
//!   and intersecting two sources' authority *sets* would need a set enumerator the public API
//!   deliberately does not expose ([`SourceDescriptor::may_hold_authority`] is a single-IRI
//!   test). With only a single-IRI test and no value enumeration there is **no** recall-safe
//!   value-overlap prune at the combination level. Recorded here so a future contributor does
//!   not reach into the private authority set to build an unsound prune.
//!
//! # Recall-safety (the load-bearing property, mirroring the upstream invariant)
//!
//! > **A source-combination is marked dead only when no full-BGP answer can use it; on any
//! > uncertainty it is kept.**
//!
//! Rule C1 marks candidates dead **only** when some pattern is empty, i.e. when the conjunction
//! is *proved* unsatisfiable (an empty conjunct ⇒ no answers exist ⇒ marking every candidate
//! dead loses nothing) — it merely **propagates** an already-recall-safe emptiness verdict
//! across the conjunction. Rule C2 marks a *pairing* dead only behind the completeness guard
//! above, i.e. only when the descriptor's own subject accounting **proves** no subject of that
//! source carries both predicates; every uncertain case (truncated sets, unknown
//! `distinct_subjects`, an unbound predicate, a non-subject-position join) declines and keeps
//! the pairing. Rule C2 removes a *(pattern, source) candidate* only in the one case where the
//! dead pairing leaves it with nowhere to go: when the peer pattern's **only** surviving
//! candidate is that same source, so every combination through this candidate contains the dead
//! pairing.
//!
//! # Honesty / threat model (no claim beyond selection plumbing)
//!
//! This is **plumbing — result-aware source-combination routing, not a cryptographic
//! guarantee.** It performs **NO** MPC, runs **NO** secret-sharing, opens nothing, verifies
//! nothing, and reads only the (public) Phase-2 selection it is handed. It makes **NO**
//! soundness/privacy/security claim. The MPC estate (`sparq-mpc`) is research-grade,
//! **honest-majority semi-honest only**, and is **NOT** externally audited — the
//! accredited-cryptographer sign-off (sq-qhy4) and the coZK re-audit (sq-9hrn) are pending.
//! This pass does not change that posture by one inch. The only thing it "reveals" is which
//! combinations are infeasible, derived entirely from the caller's own descriptors (Rule C2
//! reads only the *public* served VoID/`scs:` summary of each source — never live data). See
//! the crate `README.md` and `research/mpc-untrusted-planner-routing-design.md`.
//!
//! [OPUS-4.8] sq-pwr.3 / sq-xkrt.

use sparq_fedplan::{Bgp, CharSet, SourceDescriptor, SourceId, Term};

use crate::seam::{SeamError, SeamPhase};
use crate::selection::{PrivateCandidate, PrivatePatternSources, SelectedPrivateSources};

/// Why a (pattern, source) candidate is **combination-dead** — it survived the Phase-2 prune
/// but cannot contribute to any full-BGP answer at the combination level. The enum is
/// `#[non_exhaustive]` so a later result-aware rule can add one without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CombinationPruneReason {
    /// The full BGP is unsatisfiable because pattern `witness` (a conjunct) has **zero**
    /// live sources — either it was empty after Phase 2 (Rule C1) or every one of its
    /// candidates was proved dead by Rule C2. An empty conjunct makes the whole conjunction
    /// empty, so this candidate cannot participate in any answer.
    UnsatisfiableBgp {
        /// The empty-pattern index that witnesses the unsatisfiability (the smallest such index,
        /// for determinism).
        witness: usize,
    },
    /// **Rule C2.** This candidate's source is dead-paired with pattern `peer` (they share a
    /// subject-position join variable and no characteristic set of the source carries both
    /// predicates), *and* `peer`'s **only** surviving candidate is that same source — so every
    /// source-combination through this candidate contains the dead pairing.
    DeadStarPairing {
        /// The peer pattern of the dead same-source star pairing (the smallest such peer, for
        /// determinism).
        peer: usize,
    },
}

impl CombinationPruneReason {
    /// A short human label for the reason (for diagnostics / the audit trail).
    pub fn label(self) -> String {
        match self {
            CombinationPruneReason::UnsatisfiableBgp { witness } => format!(
                "full BGP unsatisfiable: conjunct (pattern {}) has no contributing source",
                witness
            ),
            CombinationPruneReason::DeadStarPairing { peer } => format!(
                "dead same-source star pairing with pattern {}: no characteristic set of the \
                 source carries both predicates, and pattern {} has no other candidate source",
                peer, peer
            ),
        }
    }
}

/// One **source-combination pairing** proved dead by **Rule C2**: assigning *both* patterns of
/// `patterns` to source `source` can never yield a full-BGP answer, because the two patterns
/// share a subject-position join variable and **no** characteristic set of that source carries
/// both predicates (so no subject *of that source* satisfies both conjuncts).
///
/// This is the **combination**-level unit, not a candidate-level one: the source remains a valid
/// candidate for each pattern *individually* — only the combinations that take **both** from it
/// are dead. Every full source-combination containing this pairing can be skipped before MPC;
/// [`PrunedCombinations::combination_is_dead`] applies exactly that test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadPairing {
    /// The two pattern indices of the dead pairing, **ascending** (`patterns.0 < patterns.1`).
    pub patterns: (usize, usize),
    /// The source index both patterns would be assigned to.
    pub source: usize,
    /// The resolved source id.
    pub source_id: SourceId,
    /// The name of the shared subject-position join variable (without the leading `?`).
    pub join_var: String,
    /// The two bound predicate IRIs, aligned with `patterns`, that no single characteristic set
    /// of the source carries together.
    pub predicates: (String, String),
}

/// One (pattern, source) candidate that survived Phase 2 but is dead for a full-BGP answer at
/// the combination level, with the [`CombinationPruneReason`]. Surfaced so the combination prune
/// is **auditable** — a relying party sees exactly which surviving candidates the result-aware
/// pass found infeasible and why, rather than a silent collapse (the "declared, not implicit"
/// discipline, applied to combination pruning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedCombination {
    /// The pattern the dead candidate belongs to.
    pub pattern: usize,
    /// The source index that is dead for a full-BGP answer.
    pub source: usize,
    /// The resolved source id.
    pub source_id: SourceId,
    /// Why it is dead.
    pub reason: CombinationPruneReason,
}

/// One connected component of the BGP join graph (pattern indices that are transitively linked
/// by a shared variable), surfaced so an auditor can see the Cartesian structure the
/// satisfiability verdict was computed over. Two patterns are in the same component iff they are
/// connected by a chain of [`Bgp::shares_var`] edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgpComponent {
    /// The pattern indices in this component, ascending (deterministic).
    pub patterns: Vec<usize>,
    /// Whether every member pattern is non-empty after Phase 2. `false` iff some member pattern
    /// has no surviving source (this component's sub-answer is empty). NOTE: a single empty
    /// component still makes the **whole** BGP unsatisfiable — see [`PrunedCombinations`].
    pub satisfiable: bool,
}

/// The output of the **Phase-6 result-aware source-combination prune**: the Phase-2 selection
/// carried through, plus the satisfiability verdict, the empty-pattern witnesses, the BGP join
/// structure, and the combination-dead audit trail.
///
/// This carries **no** secret material and runs **no** privacy-bearing logic — it is the
/// (public) Phase-2 candidate set plus a result-aware feasibility annotation derived entirely
/// from the caller's own descriptors. The per-pattern lists are carried through **unchanged**
/// (never mutated): the verdict is **advisory and auditable**, so the caller (a future
/// join-planner feeder) chooses whether to act on `bgp_satisfiable` / `pruned` rather than
/// having candidates silently removed.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct PrunedCombinations {
    /// The Phase-2 per-pattern candidate sets, carried through **verbatim** (the prune is
    /// advisory — the dead candidates are enumerated in `pruned`, not removed here).
    pub per_pattern: Vec<PrivatePatternSources>,
    /// Whether **any** full-BGP source-combination can produce an answer. `false` iff some
    /// conjunct has no live source — either it was empty after Phase 2 (Rule C1) or Rule C2
    /// killed its last candidate — regardless of the BGP's connectivity.
    pub bgp_satisfiable: bool,
    /// The Phase-2 empty-pattern witness indices (ascending) that prove unsatisfiability under
    /// Rule C1; empty when no conjunct was emptied by Phase 2 (so always empty when
    /// `bgp_satisfiable`).
    pub empty_patterns: Vec<usize>,
    /// Pattern indices (ascending) that were **non**-empty after Phase 2 but whose every
    /// candidate Rule C2 proved dead; empty when `bgp_satisfiable`. Disjoint from
    /// `empty_patterns` — Rule C2 is only evaluated for candidate removal when Phase 2 left no
    /// empty conjunct.
    pub dead_patterns: Vec<usize>,
    /// The **Rule C2** dead same-source star pairings, ascending by `(patterns, source)`.
    /// Advisory: these are combinations to skip, not candidates to drop (the source stays valid
    /// for each pattern individually). Computed on both the live and the dead path.
    pub dead_pairings: Vec<DeadPairing>,
    /// The connected components of the BGP join graph (sorted by minimum pattern index;
    /// deterministic). Structural metadata for the auditor — the Rule-C1 verdict is the simple
    /// "any empty conjunct" theorem, not a per-component decision.
    pub components: Vec<BgpComponent>,
    /// The combination-dead candidates with reasons, ascending by `(pattern, source)`. Empty
    /// when `bgp_satisfiable` (nothing is dropped on the live path).
    pub pruned: Vec<PrunedCombination>,
}

impl PrunedCombinations {
    /// Whether the full BGP is dead (no source-combination can produce an answer).
    pub fn is_bgp_dead(&self) -> bool {
        !self.bgp_satisfiable
    }

    /// The pattern indices that still have at least one *live* candidate for a full-BGP answer,
    /// ascending. When the BGP is dead this is empty (every combination collapsed); otherwise it
    /// is every pattern that retains a candidate not listed in `pruned`.
    pub fn surviving_combination_patterns(&self) -> Vec<usize> {
        if self.is_bgp_dead() {
            return Vec::new();
        }
        self.per_pattern
            .iter()
            .filter(|p| p.candidates.iter().any(|c| !self.is_pruned(p.pattern, c.source)))
            .map(|p| p.pattern)
            .collect()
    }

    /// Whether the (pattern, source) candidate is in the combination-dead `pruned` audit trail.
    fn is_pruned(&self, pattern: usize, source: usize) -> bool {
        self.pruned
            .iter()
            .any(|p| p.pattern == pattern && p.source == source)
    }

    /// Whether the source-combination `assignment` (one source **index** per pattern, in pattern
    /// order) is **proved** to contribute no full-BGP answer — the test a combination enumerator
    /// runs to skip a combination before routing it toward MPC.
    ///
    /// Returns `true` when the whole BGP is dead (Rule C1 / C2 collapse) or when the assignment
    /// contains a Rule-C2 [`DeadPairing`]. **Recall-safe default:** anything it cannot prove —
    /// including an `assignment` whose length does not match the BGP — returns `false` ("keep
    /// it"), never a speculative `true`.
    pub fn combination_is_dead(&self, assignment: &[usize]) -> bool {
        if self.is_bgp_dead() {
            return true;
        }
        if assignment.len() != self.per_pattern.len() {
            return false; // cannot align ⇒ cannot prove dead ⇒ keep.
        }
        self.dead_pairings.iter().any(|dp| {
            let (i, j) = dp.patterns;
            assignment.get(i) == Some(&dp.source) && assignment.get(j) == Some(&dp.source)
        })
    }
}

/// **Phase 6** — the result-aware source-combination prune over the Phase-2 selection (design
/// record §3 / §8 Phase 6).
///
/// Given the BGP, the source descriptors, and the Phase-2 [`SelectedPrivateSources`], compute
/// whether any full-BGP source-combination can produce an answer (Rule C1: an empty conjunct
/// makes the whole conjunction unsatisfiable; Rule C2: a same-source star pairing no
/// characteristic set supports is dead), and surface the dead pairings + the combination-dead
/// candidates + the BGP join structure. The Phase-2 per-pattern lists are carried through
/// **unchanged**; the verdict is advisory and auditable.
///
/// `sources` supplies the served **quotient summary** (the characteristic sets + predicate
/// partitions) Rule C2 reads; its indices must be the same slice Phase 2 selected over — an
/// invariant this pass **enforces** by matching each candidate's `source_id` against the
/// descriptor at its index, rather than trusting the caller (see *Errors*). Rule C1
/// re-reads no descriptor capability at all — it operates over the already-recall-safe Phase-2
/// emptiness — and Rule C2 reads only the characteristic-set accounting behind the completeness
/// guard, never the authority set (the declined value-overlap non-rule; see the module docs).
///
/// **Recall-safe by construction:** a combination is marked dead **only** when it is *proved* to
/// contribute nothing — the conjunction is unsatisfiable (Rule C1), or no subject of the shared
/// source carries both star predicates and the descriptor's own subject accounting proves its
/// characteristic sets complete for both (Rule C2). Every uncertain case declines. Deterministic
/// throughout (index-ascending iteration, components sorted by minimum index, the audit trails
/// explicitly sorted).
///
/// # Errors
///
/// Returns [`SeamError::DescriptorMismatch`] (phase [`SeamPhase::SourceCombination`]) if the
/// Phase-2 selection's pattern count does not match the BGP's, if its entries' `pattern` ids are
/// not a permutation of `0..bgp.patterns.len()` (out of range, or repeated — and so leaving a BGP
/// pattern undescribed), if a retained candidate's source
/// index is out of range for `sources`, or if a candidate's `source_id` does not match the id of
/// the descriptor at that index (i.e. the selection was made over a different or reordered slice)
/// — the pass refuses to guess the alignment rather than silently mis-attribute a pattern or a
/// descriptor (fail-closed, the same posture as Phase 2). It never panics and performs **no** MPC.
///
/// This function makes **no** soundness/privacy claim — see the module docs and the crate
/// `README.md`. [OPUS-4.8] sq-pwr.3 / sq-xkrt.
pub fn prune_source_combinations(
    bgp: &Bgp,
    sources: &[SourceDescriptor],
    selected: &SelectedPrivateSources,
) -> Result<PrunedCombinations, SeamError> {
    // Fail-closed on a selection/BGP shape mismatch — never guess which pattern is which.
    if selected.per_pattern.len() != bgp.patterns.len() {
        return Err(SeamError::DescriptorMismatch {
            phase: SeamPhase::SourceCombination,
            source_id: String::new(),
            detail:
                "Phase-2 selection pattern count does not match the BGP (cannot align combinations)",
        });
    }
    // The count alone does not pin WHICH pattern each entry describes. `per_pattern` is a public
    // field of a public struct, and both rules key off the `pattern` FIELD rather than the position
    // (see `candidates_of`), so a hand-built selection could repeat one pattern id and omit
    // another while still matching the BGP's length — e.g. an EMPTY entry for pattern 0 plus a
    // non-empty duplicate entry for pattern 0, which makes Rule C1 declare the whole BGP dead from
    // a "witness" that also has live candidates, suppressing live work. Require the ids to be a
    // permutation of `0..len` instead of trusting them: in range, no repeats — with the count
    // already checked, that leaves none missing (pigeonhole). Order-free, so a reordered but
    // complete selection is still accepted.
    let mut seen = vec![false; bgp.patterns.len()];
    for ps in &selected.per_pattern {
        let Some(slot) = seen.get_mut(ps.pattern) else {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                source_id: String::new(),
                detail: "Phase-2 selection carries a pattern index out of range for the BGP \
                         (cannot align combinations)",
            });
        };
        if *slot {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                source_id: String::new(),
                detail: "Phase-2 selection repeats a pattern index, so some BGP pattern has no \
                         entry (cannot align combinations)",
            });
        }
        *slot = true;
    }
    // Rule C2 resolves candidates against `sources` by index — fail closed rather than silently
    // skipping a candidate whose descriptor cannot be resolved. The index must ALSO agree with the
    // candidate's own `source_id`: Rule C2 reads `sources[cand.source]`'s characteristic sets but
    // reports `cand.source_id`, so a selection built over a different (or reordered) descriptor
    // slice would otherwise prove a pairing dead from the WRONG source's summary and prune a
    // possibly-live combination. Enforce the "same slice" invariant instead of trusting it.
    for ps in &selected.per_pattern {
        for cand in &ps.candidates {
            let Some(desc) = sources.get(cand.source) else {
                return Err(SeamError::DescriptorMismatch {
                    phase: SeamPhase::SourceCombination,
                    source_id: cand.source_id.0.clone(),
                    detail:
                        "Phase-2 candidate source index is out of range for the descriptor slice",
                });
            };
            if desc.id() != &cand.source_id {
                return Err(SeamError::DescriptorMismatch {
                    phase: SeamPhase::SourceCombination,
                    source_id: cand.source_id.0.clone(),
                    detail: "Phase-2 candidate source id does not match the descriptor at that \
                             index (the selection was made over a different descriptor slice)",
                });
            }
        }
    }

    // 1. The empty conjuncts (ascending). An empty pattern is a PROVED-empty relation (upstream
    //    recall-safety + the recall-safe Phase-2 participation prune), so it is a sound witness.
    let empty_patterns: Vec<usize> = selected
        .per_pattern
        .iter()
        .filter(|p| p.is_empty())
        .map(|p| p.pattern)
        .collect();

    // 2. Rule C2 — the dead same-source star pairings over the served quotient summary. Reported
    //    on both paths (they are a fact about the descriptors, not about satisfiability).
    let dead_pairings = dead_star_pairings(bgp, sources, selected);

    // 3. The combination-dead audit trail.
    let mut pruned: Vec<PrunedCombination> = Vec::new();
    let mut dead_patterns: Vec<usize> = Vec::new();
    if !empty_patterns.is_empty() {
        // Rule C1: the conjunction is already proved unsatisfiable — every surviving candidate is
        // dead for a full-BGP answer. Record it against the smallest empty witness (determinism).
        let witness = empty_patterns[0];
        for ps in &selected.per_pattern {
            for cand in &ps.candidates {
                pruned.push(PrunedCombination {
                    pattern: ps.pattern,
                    source: cand.source,
                    source_id: cand.source_id.clone(),
                    reason: CombinationPruneReason::UnsatisfiableBgp { witness },
                });
            }
        }
    } else {
        // Rule C2: a dead pairing kills a *candidate* only when the peer pattern's ONLY surviving
        // candidate is that same source — then every combination through this candidate contains
        // the dead pairing. Keyed on (pattern, source), smallest peer wins for determinism.
        for dp in &dead_pairings {
            let (i, j) = dp.patterns;
            for (victim, peer) in [(i, j), (j, i)] {
                if sole_candidate(selected, peer) != Some(dp.source) {
                    continue;
                }
                if pruned
                    .iter()
                    .any(|p| p.pattern == victim && p.source == dp.source)
                {
                    continue; // already recorded against a smaller peer.
                }
                pruned.push(PrunedCombination {
                    pattern: victim,
                    source: dp.source,
                    source_id: dp.source_id.clone(),
                    reason: CombinationPruneReason::DeadStarPairing { peer },
                });
            }
        }

        // A pattern whose every candidate is now dead has no live source at all, so — exactly as
        // in Rule C1 — the whole conjunction is unsatisfiable and the remaining candidates die
        // with it, against the smallest such witness.
        dead_patterns = selected
            .per_pattern
            .iter()
            .filter(|ps| {
                !ps.candidates.is_empty()
                    && ps.candidates.iter().all(|c| {
                        pruned
                            .iter()
                            .any(|p| p.pattern == ps.pattern && p.source == c.source)
                    })
            })
            .map(|ps| ps.pattern)
            .collect();
        if let Some(&witness) = dead_patterns.first() {
            for ps in &selected.per_pattern {
                for cand in &ps.candidates {
                    if pruned
                        .iter()
                        .any(|p| p.pattern == ps.pattern && p.source == cand.source)
                    {
                        continue;
                    }
                    pruned.push(PrunedCombination {
                        pattern: ps.pattern,
                        source: cand.source,
                        source_id: cand.source_id.clone(),
                        reason: CombinationPruneReason::UnsatisfiableBgp { witness },
                    });
                }
            }
        }
    }
    // Make the (pattern, source) ordering of the audit trail an explicit guarantee, mirroring
    // `selection.rs` (the two-rule construction above does not build it in order).
    pruned.sort_by_key(|p| (p.pattern, p.source));

    let bgp_satisfiable = empty_patterns.is_empty() && dead_patterns.is_empty();

    // 4. The connected components of the join graph (structural metadata for the auditor). A
    //    component is unsatisfiable if it holds an empty OR a Rule-C2-killed pattern.
    let mut no_live: Vec<usize> = empty_patterns.clone();
    no_live.extend(dead_patterns.iter().copied());
    let components = connected_components(bgp, &no_live);

    Ok(PrunedCombinations {
        per_pattern: selected.per_pattern.clone(),
        bgp_satisfiable,
        empty_patterns,
        dead_patterns,
        dead_pairings,
        components,
        pruned,
    })
}

/// The Phase-2 candidates retained for `pattern` (empty when the selection has no such pattern).
/// Looked up by the `pattern` *field* rather than by position, so nothing here depends on the
/// Phase-2 output happening to be in pattern order.
fn candidates_of(selected: &SelectedPrivateSources, pattern: usize) -> &[PrivateCandidate] {
    selected
        .per_pattern
        .iter()
        .find(|p| p.pattern == pattern)
        .map(|p| p.candidates.as_slice())
        .unwrap_or_default()
}

/// The source index of `pattern`'s **only** candidate, or `None` when it has zero or more than
/// one. Used by Rule C2: a dead pairing removes a candidate only when the peer has nowhere else
/// to go.
fn sole_candidate(selected: &SelectedPrivateSources, pattern: usize) -> Option<usize> {
    match candidates_of(selected, pattern) {
        [only] => Some(only.source),
        _ => None,
    }
}

/// Whether `cs` carries `predicate`. A linear scan: the sets are small, and (unlike a binary
/// search) this stays correct even for a hand-built [`CharSet`] whose predicates are unsorted.
fn char_set_has(cs: &CharSet, predicate: &str) -> bool {
    cs.predicates.iter().any(|p| p == predicate)
}

/// **The Rule-C2 completeness guard.** Whether `desc`'s served characteristic sets are *provably
/// complete for `predicate`* — i.e. whether "no set carries `predicate` together with the other
/// star predicate" is a proof rather than an artefact of a truncated summary.
///
/// Characteristic sets partition a source's subjects by their **exact** predicate set, so the
/// subjects carrying `predicate` are exactly `Σ_{C ∋ predicate} C.subjects`; that must equal the
/// predicate partition's `void:distinctSubjects`. Any elided set carrying `predicate` strictly
/// lowers the sum (a served set has `subjects > 0` by construction), and an unknown
/// `distinct_subjects` is served as `0` — both decline, which is the recall-safe direction.
///
/// The sum is **checked**, not saturating: a descriptor is public and hand-buildable, and a
/// saturated `u64::MAX` sum would spuriously *equal* a `u64::MAX` `distinct_subjects` and so
/// "prove" completeness from an accounting that never balanced. An overflowing sum is not a valid
/// subject partition, so it declines like any other unprovable case (fail-closed).
///
/// The sum equality is only *evidence* of completeness if the served sets really are keyed by the
/// **exact** predicate set — the builder normalises predicate order but does not reject a repeated
/// key, and two entries under the same key are not a partition. Their counts can be chosen to make
/// the singleton cohorts balance for each predicate separately while the real co-occurrence set is
/// omitted, "proving" both predicates complete and emitting a dead pairing that suppresses a live
/// combination. So a repeated key declines. This does **not** establish that the served sets are a
/// genuine subject partition — that is the source's own declaration — it only rejects the
/// violations detectable from the descriptor alone (fail-closed, the recall-safe direction).
fn char_sets_complete_for(desc: &SourceDescriptor, predicate: &str) -> bool {
    let Some(part) = desc.predicate(predicate) else {
        return false;
    };
    if part.distinct_subjects == 0 {
        return false; // unknown ⇒ cannot prove completeness ⇒ decline.
    }
    // Keys normalised to sorted order, so (like `char_set_has`) this stays correct for a
    // hand-built `CharSet` whose predicates are unsorted.
    let mut keys: Vec<Vec<&str>> = desc
        .char_sets()
        .iter()
        .map(|cs| {
            let mut key: Vec<&str> = cs.predicates.iter().map(String::as_str).collect();
            key.sort_unstable();
            key
        })
        .collect();
    keys.sort_unstable();
    if keys.windows(2).any(|w| w[0] == w[1]) {
        return false; // a repeated exact predicate set is not a partition ⇒ decline.
    }
    let mut accounted: u64 = 0;
    for cs in desc.char_sets().iter().filter(|cs| char_set_has(cs, predicate)) {
        let Some(next) = accounted.checked_add(cs.subjects) else {
            return false; // overflow ⇒ not a valid partition ⇒ cannot prove ⇒ decline.
        };
        accounted = next;
    }
    accounted == part.distinct_subjects
}

/// **Rule C2** — the dead same-source star pairings, ascending by `(patterns, source)`.
///
/// For every pattern pair `(i, j)`, `i < j`, that shares a variable in **subject** position with
/// bound predicates, and every source that is a Phase-2 candidate for **both**: the pairing is
/// dead iff the source's characteristic sets are provably complete for both predicates
/// ([`char_sets_complete_for`]) and **no** set carries them together — i.e. no subject of that
/// source satisfies both conjuncts. Every other case declines and keeps the pairing.
fn dead_star_pairings(
    bgp: &Bgp,
    sources: &[SourceDescriptor],
    selected: &SelectedPrivateSources,
) -> Vec<DeadPairing> {
    let mut out: Vec<DeadPairing> = Vec::new();
    let n = bgp.patterns.len();
    for i in 0..n {
        for j in (i + 1)..n {
            // The join must be on a shared SUBJECT-position variable: characteristic sets are
            // subject-keyed, so they say nothing about an object-position join.
            let (Term::Var(vi), Term::Var(vj)) =
                (&bgp.patterns[i].subject, &bgp.patterns[j].subject)
            else {
                continue;
            };
            if vi != vj {
                continue;
            }
            // Both predicates must be bound IRIs to be looked up in the summary.
            let (Some(pi), Some(pj)) = (
                bgp.patterns[i].predicate_iri(),
                bgp.patterns[j].predicate_iri(),
            ) else {
                continue;
            };

            for cand in candidates_of(selected, i) {
                // Only a SAME-source pairing is constrained by one source's characteristic sets;
                // a combination taking `j` from a different source is untouched.
                if !candidates_of(selected, j)
                    .iter()
                    .any(|c| c.source == cand.source)
                {
                    continue;
                }
                let Some(desc) = sources.get(cand.source) else {
                    continue; // unreachable: the caller bounds-checked. Recall-safe anyway.
                };
                if !char_sets_complete_for(desc, pi) || !char_sets_complete_for(desc, pj) {
                    continue; // truncated / unknown summary ⇒ cannot prove ⇒ keep.
                }
                if desc
                    .char_sets()
                    .iter()
                    .any(|cs| char_set_has(cs, pi) && char_set_has(cs, pj))
                {
                    continue; // some subject carries both ⇒ the pairing may contribute.
                }
                out.push(DeadPairing {
                    patterns: (i, j),
                    source: cand.source,
                    source_id: cand.source_id.clone(),
                    join_var: vi.0.clone(),
                    predicates: (pi.to_string(), pj.to_string()),
                });
            }
        }
    }
    // Already built in (patterns, source) order; make it an explicit guarantee.
    out.sort_by_key(|d| (d.patterns, d.source));
    out
}

/// The connected components of `bgp`'s join graph (two patterns linked iff they share a
/// variable, [`Bgp::shares_var`]), as a deterministic list. Each component's pattern list is
/// ascending; the component vector is sorted by minimum pattern index. A component is
/// `satisfiable` iff none of its patterns is in `no_live` (the patterns left with no live source
/// — Phase-2-empty or Rule-C2-killed).
fn connected_components(bgp: &Bgp, no_live: &[usize]) -> Vec<BgpComponent> {
    let n = bgp.patterns.len();
    // Union-find over pattern indices.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path halving.
            x = parent[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if bgp.shares_var(i, j) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    // Union toward the smaller root for a deterministic representative.
                    if ri < rj {
                        parent[rj] = ri;
                    } else {
                        parent[ri] = rj;
                    }
                }
            }
        }
    }

    // Group indices by root, in ascending member order.
    use std::collections::BTreeMap;
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        by_root.entry(r).or_default().push(i);
    }

    let mut components: Vec<BgpComponent> = by_root
        .into_values()
        .map(|patterns| {
            let satisfiable = patterns.iter().all(|p| !no_live.contains(p));
            BgpComponent {
                patterns,
                satisfiable,
            }
        })
        .collect();
    // Sort by minimum pattern index (each `patterns` is already ascending, so [0] is the min).
    components.sort_by_key(|c| c.patterns.first().copied().unwrap_or(usize::MAX));
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use sparq_fedplan::{PredPartition, TriplePattern, Var};

    use crate::selection::{select_private_sources, PrivateCandidate};
    use crate::SourcePrivacyDescriptor;

    // ── Fixtures ────────────────────────────────────────────────────────────────────────

    const SHARES_HOUSE: &str = "http://ex/sharesHouse";
    const NAME: &str = "http://ex/name";
    const OWES: &str = "http://ex/owes";
    const MEMBER_OF: &str = "http://ex/memberOf";
    const ABSENT: &str = "http://ex/predicateNoSourceHolds";

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(Var::new(s))
    }

    // A source holding a set of predicates, identified by `id`, with a COMPLETE authority set
    // (so the upstream authority prune is active — used where we want a foreign-authority bound
    // position to legitimately empty a pattern).
    fn source(id: &str, preds: &[&str]) -> SourceDescriptor {
        source_with_authorities(id, preds, true)
    }

    // A source whose authority set is INCOMPLETE (the realistic VoID-parsed posture) — a bound
    // foreign-authority position never prunes it (recall-safe).
    fn source_incomplete(id: &str, preds: &[&str]) -> SourceDescriptor {
        source_with_authorities(id, preds, false)
    }

    fn source_with_authorities(id: &str, preds: &[&str], complete: bool) -> SourceDescriptor {
        let mut b = SourceDescriptor::builder(SourceId::new(id)).total_triples(100);
        for p in preds {
            b = b.predicate(PredPartition {
                predicate: (*p).to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 10,
            });
        }
        if complete {
            b = b.authorities_complete();
        }
        b.build()
    }

    // ── Rule-C2 (characteristic-set / quotient-summary) fixtures ────────────────────────

    /// How the fixture's predicate partitions report `void:distinctSubjects` relative to the
    /// served characteristic sets — the input to the Rule-C2 completeness guard.
    enum Accounting {
        /// The sets account for every subject exactly (a COMPLETE served summary).
        Exact,
        /// `n` extra subjects per predicate are unaccounted for — the realistic TRUNCATED
        /// posture (`sparq-introspect` elides the char-set long tail).
        Elided(u64),
        /// `void:distinctSubjects` was not served (`0` = unknown).
        Unknown,
    }

    /// A source serving characteristic sets: each `(predicates, subjects)` entry is one exact
    /// per-subject predicate set. The predicate partitions are derived by summing the sets that
    /// carry each predicate, then adjusted per `accounting`.
    fn cs_source(id: &str, sets: &[(&[&str], u64)], accounting: Accounting) -> SourceDescriptor {
        let mut subjects_per_pred: BTreeMap<&str, u64> = BTreeMap::new();
        for (preds, subjects) in sets {
            for p in *preds {
                *subjects_per_pred.entry(*p).or_default() += *subjects;
            }
        }
        let mut b = SourceDescriptor::builder(SourceId::new(id)).total_triples(1000);
        for (p, subjects) in &subjects_per_pred {
            let distinct_subjects = match accounting {
                Accounting::Exact => *subjects,
                Accounting::Elided(n) => *subjects + n,
                Accounting::Unknown => 0,
            };
            b = b.predicate(PredPartition {
                predicate: (*p).to_string(),
                triples: *subjects,
                distinct_subjects,
                distinct_objects: *subjects,
            });
        }
        for (preds, subjects) in sets {
            b = b.char_set(CharSet {
                predicates: preds.iter().map(|p| (*p).to_string()).collect(),
                subjects: *subjects,
                avg_multiplicity: vec![1.0; preds.len()],
            });
        }
        b.build()
    }

    /// `?p sharesHouse ?h . ?p name ?n` — the canonical two-pattern star on the subject `?p`.
    fn star_bgp() -> Bgp {
        Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(NAME), var("n")),
        ])
    }

    /// A source whose subjects are PARTITIONED by the two star predicates — no subject carries
    /// both, so a same-source pairing over `star_bgp()` is provably dead.
    fn split_source(id: &str, accounting: Accounting) -> SourceDescriptor {
        cs_source(id, &[(&[SHARES_HOUSE], 5), (&[NAME], 7)], accounting)
    }

    /// A source with a characteristic set carrying BOTH star predicates — the pairing lives.
    fn covering_source(id: &str) -> SourceDescriptor {
        cs_source(
            id,
            &[(&[NAME, SHARES_HOUSE], 3), (&[NAME], 4)],
            Accounting::Exact,
        )
    }

    // ── Tests ───────────────────────────────────────────────────────────────────────────

    /// A satisfiable connected 2-pattern BGP is a pure pass-through: nothing pruned, the
    /// selection carried through verbatim, the BGP one live component.
    #[test]
    fn satisfiable_bgp_carries_through_unchanged() {
        // ?p sharesHouse ?h . ?p name ?n   — joins on ?p.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(NAME), var("n")),
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.bgp_satisfiable);
        assert!(out.empty_patterns.is_empty());
        assert!(out.pruned.is_empty(), "nothing pruned on the live path");
        // The Phase-2 selection is carried through verbatim.
        assert_eq!(out.per_pattern, selected.per_pattern);
        // One connected component covering both patterns, satisfiable.
        assert_eq!(out.components.len(), 1);
        assert_eq!(out.components[0].patterns, vec![0, 1]);
        assert!(out.components[0].satisfiable);
        assert_eq!(out.surviving_combination_patterns(), vec![0, 1]);
        assert!(!out.is_bgp_dead());
    }

    /// Positive control (the required four-flatmates shape): a four-pattern star/chain where
    /// every pattern is held by some source must NOT fire the prune — a healthy federated query
    /// keeps every combination.
    #[test]
    fn four_flatmates_positive_control() {
        // ?p sharesHouse :H . ?p name ?n . ?p owes ?amt . ?p memberOf :Flat — a star on ?p.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), iri("http://ex/H")),
            TriplePattern::new(var("p"), iri(NAME), var("n")),
            TriplePattern::new(var("p"), iri(OWES), var("amt")),
            TriplePattern::new(var("p"), iri(MEMBER_OF), iri("http://ex/Flat")),
        ]);
        // Four flatmate sources, each holding all four predicates (so every pattern is non-empty).
        let sources = vec![
            source("http://a/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF]),
            source("http://b/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF]),
            source("http://c/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF]),
            source("http://d/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF]),
        ];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();

        assert!(
            out.bgp_satisfiable,
            "a healthy four-flatmates query is satisfiable"
        );
        assert!(out.pruned.is_empty(), "no combination pruned");
        // One connected star component over all four patterns.
        assert_eq!(out.components.len(), 1);
        assert_eq!(out.components[0].patterns, vec![0, 1, 2, 3]);
        assert!(out.components[0].satisfiable);
    }

    /// Rule C1: an empty conjunct in a CONNECTED BGP makes the whole BGP dead — every surviving
    /// candidate of the non-empty patterns is recorded combination-dead with the empty pattern
    /// as the witness.
    #[test]
    fn empty_pattern_makes_whole_connected_bgp_dead() {
        // Pattern 1's predicate is held by no source ⇒ Phase 2 empties it.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(ABSENT), var("z")), // no source holds ABSENT.
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Confirm the upstream really emptied pattern 1.
        assert!(selected.per_pattern[1].is_empty());

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(!out.bgp_satisfiable);
        assert!(out.is_bgp_dead());
        assert_eq!(out.empty_patterns, vec![1]);
        // The one surviving candidate (pattern 0, source 0) is recorded dead with witness 1.
        assert_eq!(out.pruned.len(), 1);
        assert_eq!(out.pruned[0].pattern, 0);
        assert_eq!(out.pruned[0].source, 0);
        assert_eq!(
            out.pruned[0].reason,
            CombinationPruneReason::UnsatisfiableBgp { witness: 1 }
        );
        assert!(out.pruned[0].reason.label().contains("pattern 1"));
        // No pattern is live for a full answer.
        assert!(out.surviving_combination_patterns().is_empty());
    }

    /// The whole-BGP verdict holds regardless of connectivity: an empty pattern in one
    /// DISCONNECTED component still makes the full BGP unsatisfiable, while the components are
    /// reported with their individual satisfiability.
    #[test]
    fn empty_pattern_in_disconnected_bgp_still_kills_full_answer() {
        // Component A: ?p sharesHouse ?h (non-empty). Component B: ?x owes ?y joined with
        // ?x absent ?z — B's second pattern is empty.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")), // comp A (alone)
            TriplePattern::new(var("x"), iri(OWES), var("y")),         // comp B
            TriplePattern::new(var("x"), iri(ABSENT), var("z")),       // comp B (empty)
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, OWES])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(
            !out.bgp_satisfiable,
            "any empty conjunct kills the whole BGP"
        );
        assert_eq!(out.empty_patterns, vec![2]);
        // Two components: A = {0}, B = {1, 2}. A satisfiable, B not.
        assert_eq!(out.components.len(), 2);
        assert_eq!(out.components[0].patterns, vec![0]);
        assert!(out.components[0].satisfiable);
        assert_eq!(out.components[1].patterns, vec![1, 2]);
        assert!(!out.components[1].satisfiable);
    }

    /// Multiple empty patterns: the witness is the SMALLEST empty index (deterministic), and the
    /// audit trail is sorted by (pattern, source).
    #[test]
    fn multiple_empty_patterns_pick_smallest_witness_deterministically() {
        // Patterns 1 and 3 are empty (ABSENT); 0 and 2 are non-empty.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")), // 0 non-empty
            TriplePattern::new(var("p"), iri(ABSENT), var("a")),       // 1 empty
            TriplePattern::new(var("p"), iri(NAME), var("n")),         // 2 non-empty
            TriplePattern::new(var("p"), iri(ABSENT), var("b")),       // 3 empty
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(!out.bgp_satisfiable);
        assert_eq!(out.empty_patterns, vec![1, 3]);
        // Both surviving candidates (patterns 0 and 2) name witness 1.
        assert_eq!(out.pruned.len(), 2);
        for pc in &out.pruned {
            assert_eq!(
                pc.reason,
                CombinationPruneReason::UnsatisfiableBgp { witness: 1 }
            );
        }
        // Sorted by (pattern, source).
        assert_eq!(out.pruned[0].pattern, 0);
        assert_eq!(out.pruned[1].pattern, 2);
    }

    /// Recall-safety negative control for the DECLINED value-overlap non-rule: a join `?v p c`
    /// (object bound IRI) ⋈ `?v q ?z` where the source holding `q` has an authority set disjoint
    /// from `c` must NOT prune the source — the module does not invent an unsound bound-IRI
    /// propagation prune across a join variable.
    #[test]
    fn recall_safe_no_value_overlap_prune() {
        // Pattern 0: ?v sharesHouse <http://h-authority.example/H> (object on a foreign
        // authority). Pattern 1: ?v name ?n. They join on ?v.
        let bgp = Bgp::new(vec![
            TriplePattern::new(
                var("v"),
                iri(SHARES_HOUSE),
                iri("http://h-authority.example/H"),
            ),
            TriplePattern::new(var("v"), iri(NAME), var("n")),
        ]);
        // Source A holds sharesHouse (its authority is http://ex, NOT h-authority.example) and
        // name, with an INCOMPLETE authority set (the realistic VoID-parsed posture), so the
        // foreign-authority bound OBJECT does NOT prune A per-pattern — and crucially the join
        // variable ?v is NOT forced to the constant. Both patterns therefore retain A: the
        // scenario the declined value-overlap non-rule would (wrongly) try to prune.
        let sources = vec![source_incomplete("http://ex/a", &[SHARES_HOUSE, NAME])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Positive precondition: BOTH patterns kept A (so we are genuinely on the live path).
        assert!(
            !selected.per_pattern[0].is_empty(),
            "pattern 0 keeps the source"
        );
        assert!(
            !selected.per_pattern[1].is_empty(),
            "pattern 1 keeps the source"
        );

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        // The BGP is satisfiable and NOTHING is pruned — no unsound value-overlap inference
        // across the join variable, no bound-IRI propagation.
        assert!(out.bgp_satisfiable);
        assert!(
            out.pruned.is_empty(),
            "no candidate is pruned while the BGP is satisfiable (value-overlap prune declined)"
        );
        assert_eq!(out.surviving_combination_patterns(), vec![0, 1]);
    }

    /// The pass trusts the upstream recall-safe emptiness: when `select_sources`' complete-
    /// authority prune legitimately empties a pattern, that pattern is the witness — the
    /// combination pass does not re-derive or widen the capability prune.
    #[test]
    fn complete_authority_emptiness_is_respected_not_reproduced() {
        // A complete-authority source minting only `http://ex` terms; a pattern with a foreign-
        // authority bound subject is emptied upstream.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")), // 0 non-empty
            TriplePattern::new(
                iri("http://foreign.example/Subj"), // foreign-authority bound subject
                iri(NAME),
                var("n"),
            ), // 1 emptied by the complete-authority prune
        ]);
        let sources = vec![source("http://ex/a", &[SHARES_HOUSE, NAME])]; // authorities_complete()
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        assert!(
            selected.per_pattern[1].is_empty(),
            "upstream complete-authority prune empties pattern 1"
        );

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(!out.bgp_satisfiable);
        assert_eq!(out.empty_patterns, vec![1]);
        // The witness is the upstream-emptied pattern; the live candidate (pattern 0) is dead.
        assert_eq!(out.pruned.len(), 1);
        assert_eq!(
            out.pruned[0].reason,
            CombinationPruneReason::UnsatisfiableBgp { witness: 1 }
        );
    }

    /// A pattern emptied by the Phase-2 PARTICIPATION prune (not by `select_sources`) is an
    /// equally valid witness — the rule keys on Phase-2 emptiness whatever its cause, and the
    /// participation prune is itself recall-safe.
    #[test]
    fn participation_emptied_pattern_triggers_collapse() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")), // 0
            TriplePattern::new(var("p"), iri(NAME), var("n")),         // 1
        ]);
        // The only source holding `name` declares it will not participate ⇒ Phase 2 empties
        // pattern 1 (the source holds sharesHouse too, so pattern 0 survives only if another
        // source holds it; give pattern 0 its own participating source).
        let sources = vec![
            source("http://keep/", &[SHARES_HOUSE]),
            source("http://refuse/", &[NAME]),
        ];
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://refuse/"))
                .participates(false)
                .build(),
        ];
        let selected = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(
            selected.per_pattern[1].is_empty(),
            "the participation prune empties pattern 1"
        );

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(!out.bgp_satisfiable);
        assert_eq!(out.empty_patterns, vec![1]);
        // Pattern 0's surviving candidate is recorded dead.
        assert_eq!(out.pruned.len(), 1);
        assert_eq!(out.pruned[0].pattern, 0);
    }

    /// A single-pattern BGP: satisfiable with a source (no prune), unsatisfiable without one
    /// (empty witness, nothing to mark, but the BGP is dead).
    #[test]
    fn single_pattern_bgp_satisfiable_and_empty_cases() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("p"),
            iri(SHARES_HOUSE),
            var("h"),
        )]);
        // Satisfiable.
        let sources = vec![source("http://a/", &[SHARES_HOUSE])];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &sel).unwrap();
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
        assert_eq!(out.components.len(), 1);

        // Unsatisfiable: no source holds the predicate.
        let none = vec![source("http://a/", &[NAME])]; // holds name, not sharesHouse.
        let sel2 = select_private_sources(&bgp, &none, &[]).unwrap();
        let out2 = prune_source_combinations(&bgp, &none, &sel2).unwrap();
        assert!(!out2.bgp_satisfiable);
        assert_eq!(out2.empty_patterns, vec![0]);
        assert!(
            out2.pruned.is_empty(),
            "the empty pattern has no candidate to mark"
        );
        assert!(out2.is_bgp_dead());
    }

    /// Components are deterministic and sorted: a BGP with two components and a shared-variable
    /// chain reports them sorted by minimum index, each with ascending members.
    #[test]
    fn components_are_deterministic_and_sorted() {
        // Comp X: 0-1-2 chained on ?p then ?q. Comp Y: 3 alone.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("q")), // 0 (p,q)
            TriplePattern::new(var("q"), iri(NAME), var("r")),         // 1 joins 0 on q
            TriplePattern::new(var("r"), iri(OWES), var("s")),         // 2 joins 1 on r
            TriplePattern::new(var("z"), iri(MEMBER_OF), var("w")),    // 3 disjoint
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF])];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &sel).unwrap();
        assert_eq!(out.components.len(), 2);
        assert_eq!(out.components[0].patterns, vec![0, 1, 2]);
        assert_eq!(out.components[1].patterns, vec![3]);
        // Sorted by min index: component {0,1,2} first.
        assert!(out.components[0].patterns[0] < out.components[1].patterns[0]);
    }

    /// Determinism: identical inputs ⇒ identical output, twice (including the unsatisfiable
    /// path).
    #[test]
    fn determinism_same_inputs_same_output() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(ABSENT), var("z")),
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE])];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        let a = prune_source_combinations(&bgp, &sources, &sel).unwrap();
        let b = prune_source_combinations(&bgp, &sources, &sel).unwrap();
        assert_eq!(a, b);
    }

    /// A longer shared-variable chain (0-1-2-3-4) must still collapse to ONE component with all
    /// five patterns — the union-find path-compression over a transitive chain produces a single
    /// connected component, exercising the multi-level `find` walk. A regression in the component
    /// merge (e.g. losing transitivity) would split the chain. [OPUS-4.8] sq-bif.
    #[test]
    fn long_shared_variable_chain_is_one_component() {
        // ?a p ?b . ?b p ?c . ?c p ?d . ?d p ?e . ?e p ?f  — a five-pattern chain, each joining
        // the next on the shared variable.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("a"), iri(SHARES_HOUSE), var("b")),
            TriplePattern::new(var("b"), iri(NAME), var("c")),
            TriplePattern::new(var("c"), iri(OWES), var("d")),
            TriplePattern::new(var("d"), iri(MEMBER_OF), var("e")),
            TriplePattern::new(var("e"), iri(SHARES_HOUSE), var("f")),
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME, OWES, MEMBER_OF])];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &sel).unwrap();
        assert!(out.bgp_satisfiable);
        // Exactly one component covering the whole transitive chain, ascending.
        assert_eq!(out.components.len(), 1);
        assert_eq!(out.components[0].patterns, vec![0, 1, 2, 3, 4]);
        assert!(out.components[0].satisfiable);
    }

    /// `BgpComponent::satisfiable` is per-component: in a disconnected BGP with one empty and one
    /// non-empty component, the non-empty component reports `satisfiable == true` while the empty
    /// one reports `false` — even though the WHOLE-BGP verdict is unsatisfiable. This pins the
    /// distinction between the per-component flag and the global `bgp_satisfiable`. [OPUS-4.8]
    /// sq-bif.
    #[test]
    fn per_component_satisfiability_is_independent_of_the_global_verdict() {
        // Comp A: 0-1 chained on ?p, both non-empty (satisfiable). Comp B: 2 alone, empty.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")), // 0 comp A
            TriplePattern::new(var("p"), iri(NAME), var("n")),         // 1 comp A
            TriplePattern::new(var("z"), iri(ABSENT), var("w")),       // 2 comp B (empty)
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &sel).unwrap();

        // Global verdict: unsatisfiable (an empty conjunct kills the whole BGP).
        assert!(!out.bgp_satisfiable);
        assert_eq!(out.empty_patterns, vec![2]);

        // But the per-component flag distinguishes the live component from the dead one.
        assert_eq!(out.components.len(), 2);
        let comp_a = out
            .components
            .iter()
            .find(|c| c.patterns == vec![0, 1])
            .expect("the {0,1} component exists");
        assert!(
            comp_a.satisfiable,
            "the non-empty component is itself satisfiable"
        );
        let comp_b = out
            .components
            .iter()
            .find(|c| c.patterns == vec![2])
            .expect("the {2} component exists");
        assert!(!comp_b.satisfiable, "the empty component is unsatisfiable");
    }

    /// `CombinationPruneReason::label` names the witnessing pattern explicitly (a regression that
    /// blanked it or dropped the witness index would lose the audit-trail diagnostic). [OPUS-4.8]
    /// sq-bif.
    #[test]
    fn combination_prune_reason_label_names_the_witness_pattern() {
        let label = CombinationPruneReason::UnsatisfiableBgp { witness: 7 }.label();
        assert!(
            label.contains("unsatisfiable"),
            "label names the cause: {label}"
        );
        assert!(
            label.contains("pattern 7"),
            "label names the witness pattern: {label}"
        );
    }

    // ── Rule C2 — the dead same-source star pairing (the FedUP quotient-summary rule) ───

    /// **The headline Rule-C2 win.** Source A's subjects are partitioned by the two star
    /// predicates (no subject carries both), source B has a set carrying both. The A-A pairing is
    /// proved dead — so the seam skips that source-combination before MPC — while every
    /// cross-source combination and the B-B combination survive, and NO per-pattern candidate is
    /// dropped (A remains valid for each pattern individually).
    #[test]
    fn dead_star_pairing_kills_only_the_same_source_combination() {
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact), // index 0
            covering_source("http://b/"),                 // index 1
        ];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Precondition: both patterns retain BOTH sources, so we are genuinely on the live path
        // and the prune is combination-level, not per-pattern.
        assert_eq!(selected.per_pattern[0].candidates.len(), 2);
        assert_eq!(selected.per_pattern[1].candidates.len(), 2);

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();

        // Exactly one dead pairing: (pattern 0, pattern 1) both on source A.
        assert_eq!(out.dead_pairings.len(), 1, "{:?}", out.dead_pairings);
        let dp = &out.dead_pairings[0];
        assert_eq!(dp.patterns, (0, 1));
        assert_eq!(dp.source, 0);
        assert_eq!(dp.source_id, SourceId::new("http://a/"));
        assert_eq!(dp.join_var, "p");
        assert_eq!(
            dp.predicates,
            (SHARES_HOUSE.to_string(), NAME.to_string()),
            "predicates are aligned with `patterns`"
        );

        // The BGP is still satisfiable and NO candidate is dropped — a combination taking
        // pattern 0 from A and pattern 1 from B is untouched.
        assert!(out.bgp_satisfiable);
        assert!(out.dead_patterns.is_empty());
        assert!(out.pruned.is_empty(), "{:?}", out.pruned);
        assert_eq!(out.per_pattern, selected.per_pattern);
        assert_eq!(out.surviving_combination_patterns(), vec![0, 1]);

        // The combination test: only A-A is dead; the other three combinations survive.
        assert!(out.combination_is_dead(&[0, 0]), "A-A is proved dead");
        assert!(!out.combination_is_dead(&[0, 1]), "A-B survives");
        assert!(!out.combination_is_dead(&[1, 0]), "B-A survives");
        assert!(!out.combination_is_dead(&[1, 1]), "B-B survives");
    }

    /// **Recall-safety, the load-bearing negative control.** The same partitioned source, but its
    /// served characteristic sets are TRUNCATED (the elided long tail `sparq-introspect` produces)
    /// — `Σ_{C ∋ p} subjects` no longer equals `void:distinctSubjects`, so the completeness guard
    /// declines and NOTHING is pruned. Deleting the guard would make this test fail with a
    /// wrong-answer prune.
    #[test]
    fn truncated_char_sets_decline_the_prune() {
        let bgp = star_bgp();
        let sources = vec![split_source("http://a/", Accounting::Elided(1))];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        assert!(!selected.per_pattern[0].is_empty() && !selected.per_pattern[1].is_empty());

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(
            out.dead_pairings.is_empty(),
            "an incomplete (truncated) summary cannot prove a pairing dead"
        );
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
        assert!(!out.combination_is_dead(&[0, 0]));
    }

    /// Recall-safety: a predicate partition that did not serve `void:distinctSubjects` (`0` =
    /// unknown) cannot prove completeness either, so the rule declines.
    #[test]
    fn unknown_distinct_subjects_declines_the_prune() {
        let bgp = star_bgp();
        let sources = vec![split_source("http://a/", Accounting::Unknown)];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
    }

    /// A source with NO served characteristic sets (the plain fixture the rest of this module
    /// uses) never fires Rule C2 — the accounting sums to 0, which cannot equal a positive
    /// `distinct_subjects`.
    #[test]
    fn no_char_sets_declines_the_prune() {
        let bgp = star_bgp();
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
    }

    /// Positive control for the rule's *precondition*: when some characteristic set carries BOTH
    /// star predicates, a subject satisfying both conjuncts may exist — the pairing is kept.
    #[test]
    fn covering_char_set_keeps_the_pairing() {
        let bgp = star_bgp();
        let sources = vec![covering_source("http://b/")];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
        assert!(!out.combination_is_dead(&[0, 0]));
    }

    /// Recall-safety: characteristic sets are SUBJECT-keyed, so a join on a shared **object**
    /// variable says nothing about co-occurrence on one subject — the rule must decline even
    /// though the source's sets separate the two predicates.
    #[test]
    fn object_position_join_is_not_a_star_prune() {
        // ?s sharesHouse ?v . ?w name ?v — joined on ?v in OBJECT position of both; the subjects
        // are distinct variables, so two different subjects can satisfy the two conjuncts.
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri(SHARES_HOUSE), var("v")),
            TriplePattern::new(var("w"), iri(NAME), var("v")),
        ]);
        let sources = vec![split_source("http://a/", Accounting::Exact)];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(
            out.dead_pairings.is_empty(),
            "an object-position join is not constrained by subject characteristic sets"
        );
        assert!(out.bgp_satisfiable);
        assert!(!out.combination_is_dead(&[0, 0]));
    }

    /// Recall-safety: two patterns whose subject variables merely *differ* (no shared subject
    /// join at all) are never paired, even on one source with separated sets.
    #[test]
    fn unrelated_subject_variables_are_not_paired() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("q"), iri(NAME), var("n")),
        ]);
        let sources = vec![split_source("http://a/", Accounting::Exact)];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
    }

    /// Recall-safety: an UNBOUND predicate cannot be looked up in the summary, so the pairing is
    /// kept even though the other predicate is bound and the sets are complete.
    #[test]
    fn unbound_predicate_declines_the_prune() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), var("anyPred"), var("h")),
            TriplePattern::new(var("p"), iri(NAME), var("n")),
        ]);
        let sources = vec![split_source("http://a/", Accounting::Exact)];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        assert!(!selected.per_pattern[0].is_empty(), "unbound predicate keeps the source");

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
    }

    /// Rule C2 removes a *candidate* only when the dead pairing leaves it nowhere to go: with a
    /// single partitioned source, each pattern's ONLY candidate is dead-paired with the other, so
    /// both candidates die, both patterns lose their last source, and the whole BGP collapses.
    #[test]
    fn sole_peer_candidate_collapses_the_bgp() {
        let bgp = star_bgp();
        let sources = vec![split_source("http://a/", Accounting::Exact)];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Phase 2 leaves NO empty pattern — the collapse is entirely Rule C2's doing.
        assert!(!selected.per_pattern[0].is_empty() && !selected.per_pattern[1].is_empty());

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert_eq!(out.dead_pairings.len(), 1);
        assert!(!out.bgp_satisfiable);
        assert!(out.is_bgp_dead());
        assert!(
            out.empty_patterns.is_empty(),
            "no conjunct was empty after Phase 2 — this is a Rule-C2 collapse"
        );
        assert_eq!(out.dead_patterns, vec![0, 1]);
        // Both candidates recorded, each naming the other pattern as the dead-pairing peer.
        assert_eq!(out.pruned.len(), 2);
        assert_eq!(
            out.pruned[0].reason,
            CombinationPruneReason::DeadStarPairing { peer: 1 }
        );
        assert_eq!(
            out.pruned[1].reason,
            CombinationPruneReason::DeadStarPairing { peer: 0 }
        );
        assert!(out.pruned[0].reason.label().contains("pattern 1"));
        assert!(out.surviving_combination_patterns().is_empty());
        // The one component is reported unsatisfiable.
        assert_eq!(out.components.len(), 1);
        assert!(!out.components[0].satisfiable);
        // Every combination is dead.
        assert!(out.combination_is_dead(&[0, 0]));
    }

    /// A dead pairing whose peer has ANOTHER candidate never removes a candidate — the peer can
    /// simply be taken from the other source. (The complement of the collapse above: same
    /// partitioned source A, but pattern 1 also has source B.)
    #[test]
    fn dead_pairing_with_an_alternative_peer_source_removes_no_candidate() {
        // Pattern 0 is held only by A (B does not hold sharesHouse); pattern 1 by A and B.
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact),               // holds both
            cs_source("http://b/", &[(&[NAME], 9)], Accounting::Exact), // holds `name` only
        ];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        assert_eq!(selected.per_pattern[0].candidates.len(), 1, "only A holds sharesHouse");
        assert_eq!(selected.per_pattern[1].candidates.len(), 2);

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert_eq!(out.dead_pairings.len(), 1, "the A-A pairing is dead");
        // Pattern 1 has an alternative (B), so candidate (0, A) survives; and pattern 0's sole
        // candidate IS A, so candidate (1, A) is dead.
        assert_eq!(out.pruned.len(), 1);
        assert_eq!(out.pruned[0].pattern, 1);
        assert_eq!(out.pruned[0].source, 0);
        assert_eq!(
            out.pruned[0].reason,
            CombinationPruneReason::DeadStarPairing { peer: 0 }
        );
        // Pattern 1 still has B, so the BGP lives.
        assert!(out.bgp_satisfiable);
        assert!(out.dead_patterns.is_empty());
        assert_eq!(out.surviving_combination_patterns(), vec![0, 1]);
        assert!(out.combination_is_dead(&[0, 0]));
        assert!(!out.combination_is_dead(&[0, 1]), "pattern 1 from B survives");
    }

    /// `combination_is_dead` is recall-safe on anything it cannot align: a wrong-length
    /// assignment returns `false` ("keep it"), never a speculative `true`.
    #[test]
    fn combination_is_dead_declines_a_misaligned_assignment() {
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact),
            covering_source("http://b/"),
        ];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.bgp_satisfiable, "precondition: the live path");
        assert!(!out.combination_is_dead(&[0]), "too short ⇒ cannot prove ⇒ keep");
        assert!(!out.combination_is_dead(&[0, 0, 0]), "too long ⇒ keep");
        assert!(!out.combination_is_dead(&[]), "empty ⇒ keep");
    }

    /// On a dead BGP every combination is dead, including a misaligned one — the collapse is
    /// unconditional.
    #[test]
    fn combination_is_dead_is_unconditional_on_a_dead_bgp() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(ABSENT), var("z")),
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE])];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(out.is_bgp_dead());
        assert!(out.combination_is_dead(&[0, 0]));
        assert!(out.combination_is_dead(&[]));
    }

    /// Determinism with Rule C2 active: identical inputs ⇒ identical output, twice.
    #[test]
    fn determinism_with_dead_pairings() {
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact),
            covering_source("http://b/"),
        ];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        let a = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        let b = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert_eq!(a, b);
        assert!(!a.dead_pairings.is_empty(), "the run is not vacuous");
    }

    /// A candidate whose source index is out of range for the descriptor slice is fail-closed
    /// (Rule C2 resolves descriptors by index — never silently skip an unresolvable one).
    #[test]
    fn out_of_range_candidate_index_is_descriptor_mismatch() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("p"),
            iri(SHARES_HOUSE),
            var("h"),
        )]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE])];
        let bogus = SelectedPrivateSources {
            per_pattern: vec![PrivatePatternSources {
                pattern: 0,
                candidates: vec![PrivateCandidate {
                    source: 7, // out of range for a one-source slice.
                    source_id: SourceId::new("http://a/"),
                    estimated_cardinality: 1.0,
                }],
            }],
            pruned: Vec::new(),
        };
        let err = prune_source_combinations(&bgp, &sources, &bogus).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                ..
            }
        ));
        assert!(format!("{}", err).contains("out of range"));
    }

    /// Recall-safety against an OVERFLOWING subject accounting: characteristic-set subject counts
    /// that overflow `u64` are not a valid subject partition, so the completeness guard must
    /// decline. A *saturating* sum would land on `u64::MAX` and spuriously equal a `u64::MAX`
    /// `void:distinctSubjects`, "proving" completeness on a descriptor whose accounting never
    /// balanced and emitting a dead pairing that suppresses a combination.
    #[test]
    fn overflowing_char_set_accounting_declines_the_prune() {
        let bgp = star_bgp();
        // Both star predicates report `distinct_subjects == u64::MAX`, and each is carried by two
        // characteristic sets whose subject counts sum PAST `u64::MAX`. No set carries both, so
        // with a saturating sum the pairing would be "proved" dead.
        let mut b =
            SourceDescriptor::builder(SourceId::new("http://overflow/")).total_triples(1000);
        for p in [SHARES_HOUSE, NAME] {
            b = b.predicate(PredPartition {
                predicate: p.to_string(),
                triples: 1000,
                distinct_subjects: u64::MAX,
                distinct_objects: 1000,
            });
        }
        for (preds, subjects) in [
            (vec![SHARES_HOUSE], u64::MAX),
            (vec![SHARES_HOUSE, OWES], 1),
            (vec![NAME], u64::MAX),
            (vec![NAME, MEMBER_OF], 1),
        ] {
            let arity = preds.len();
            b = b.char_set(CharSet {
                predicates: preds.iter().map(|p| (*p).to_string()).collect(),
                subjects,
                avg_multiplicity: vec![1.0; arity],
            });
        }
        let sources = vec![b.build()];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Precondition: both patterns retain the source, so the rule is genuinely evaluated.
        assert!(!selected.per_pattern[0].is_empty() && !selected.per_pattern[1].is_empty());

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(
            out.dead_pairings.is_empty(),
            "an overflowing subject accounting cannot prove the sets complete: {:?}",
            out.dead_pairings
        );
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
        assert!(!out.combination_is_dead(&[0, 0]));
    }

    /// An in-range candidate index whose `source_id` disagrees with the descriptor at that index —
    /// a selection made over a different or reordered descriptor slice — is fail-closed. Without
    /// the identity check Rule C2 would read the INDEXED source's characteristic sets (here the
    /// partitioned A) while attributing the dead pairing to the named source (B, whose own sets
    /// cover both predicates), pruning a live combination.
    #[test]
    fn candidate_id_not_matching_indexed_descriptor_is_descriptor_mismatch() {
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact), // index 0 — partitioned (would prune)
            covering_source("http://b/"),                 // index 1 — covers both predicates
        ];
        // A candidate naming B but indexing A: the disagreement Phase 2 could never produce.
        let mismatched = SelectedPrivateSources {
            per_pattern: (0..2)
                .map(|pattern| PrivatePatternSources {
                    pattern,
                    candidates: vec![PrivateCandidate {
                        source: 0,
                        source_id: SourceId::new("http://b/"),
                        estimated_cardinality: 1.0,
                    }],
                })
                .collect(),
            pruned: Vec::new(),
        };

        let err = prune_source_combinations(&bgp, &sources, &mismatched).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                ..
            }
        ));
        let msg = format!("{}", err);
        assert!(
            msg.contains("does not match the descriptor"),
            "the error names the identity disagreement: {}",
            msg
        );
        assert!(msg.contains("http://b/"), "the error names the offending id: {}", msg);

        // Control: the SAME selection against the slice it actually describes is accepted, and
        // (B covering both predicates) prunes nothing — so the rejection above is the identity
        // check firing, not the shape.
        let honest = vec![covering_source("http://b/")];
        let out = prune_source_combinations(&bgp, &honest, &mismatched).unwrap();
        assert!(out.dead_pairings.is_empty());
        assert!(out.bgp_satisfiable);
    }

    /// A selection whose pattern count does not match the BGP is a fail-closed
    /// DescriptorMismatch (phase SourceCombination) — never guess the alignment.
    #[test]
    fn selection_bgp_length_mismatch_is_descriptor_mismatch() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("p"), iri(SHARES_HOUSE), var("h")),
            TriplePattern::new(var("p"), iri(NAME), var("n")),
        ]);
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        // Hand-craft a one-pattern selection against a two-pattern BGP.
        let mismatched = SelectedPrivateSources {
            per_pattern: vec![PrivatePatternSources {
                pattern: 0,
                candidates: vec![PrivateCandidate {
                    source: 0,
                    source_id: SourceId::new("http://a/"),
                    estimated_cardinality: 1.0,
                }],
            }],
            pruned: Vec::new(),
        };
        let err = prune_source_combinations(&bgp, &sources, &mismatched).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                ..
            }
        ));
        assert!(format!("{}", err).contains("cannot align combinations"));
    }

    /// A selection with the right entry COUNT whose `pattern` ids do not cover the BGP — one id
    /// repeated, another therefore missing — is fail-closed. Both rules look patterns up by the
    /// `pattern` field, so without the permutation check an EMPTY duplicate entry for pattern 0
    /// would give Rule C1 a witness while pattern 0 also carries live candidates: the whole BGP
    /// would be declared dead and `combination_is_dead` would return true for every assignment,
    /// suppressing live work.
    #[test]
    fn repeated_pattern_index_in_selection_is_descriptor_mismatch() {
        let bgp = star_bgp();
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let cand = || {
            vec![PrivateCandidate {
                source: 0,
                source_id: SourceId::new("http://a/"),
                estimated_cardinality: 1.0,
            }]
        };
        let bogus = SelectedPrivateSources {
            per_pattern: vec![
                // An "empty" pattern 0 (the Rule-C1 witness) …
                PrivatePatternSources {
                    pattern: 0,
                    candidates: Vec::new(),
                },
                // … plus a non-empty DUPLICATE of pattern 0, leaving pattern 1 undescribed.
                PrivatePatternSources {
                    pattern: 0,
                    candidates: cand(),
                },
            ],
            pruned: Vec::new(),
        };
        let err = prune_source_combinations(&bgp, &sources, &bogus).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                ..
            }
        ));
        let msg = format!("{}", err);
        assert!(
            msg.contains("repeats a pattern index"),
            "the error names the repeated id: {}",
            msg
        );

        // Control: the honestly-aligned selection over the same BGP and sources is accepted and
        // stays live — so the rejection above is the alignment check firing, not the shape.
        let honest = select_private_sources(&bgp, &sources, &[]).unwrap();
        let out = prune_source_combinations(&bgp, &sources, &honest).unwrap();
        assert!(out.bgp_satisfiable);
        assert!(!out.combination_is_dead(&[0, 0]));
    }

    /// A `pattern` id past the end of the BGP is fail-closed for the same reason: the entry cannot
    /// be attributed to a pattern, so some real pattern is undescribed.
    #[test]
    fn out_of_range_pattern_index_in_selection_is_descriptor_mismatch() {
        let bgp = star_bgp();
        let sources = vec![source("http://a/", &[SHARES_HOUSE, NAME])];
        let bogus = SelectedPrivateSources {
            per_pattern: [0, 7]
                .into_iter()
                .map(|pattern| PrivatePatternSources {
                    pattern,
                    candidates: vec![PrivateCandidate {
                        source: 0,
                        source_id: SourceId::new("http://a/"),
                        estimated_cardinality: 1.0,
                    }],
                })
                .collect(),
            pruned: Vec::new(),
        };
        let err = prune_source_combinations(&bgp, &sources, &bogus).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceCombination,
                ..
            }
        ));
        assert!(format!("{}", err).contains("out of range for the BGP"));
    }

    /// The alignment check is order-FREE (patterns are looked up by field, not position): a
    /// COMPLETE selection whose entries are permuted is accepted and reaches the same verdict.
    #[test]
    fn reordered_but_complete_selection_is_accepted() {
        let bgp = star_bgp();
        let sources = vec![
            split_source("http://a/", Accounting::Exact),
            covering_source("http://b/"),
        ];
        let in_order = select_private_sources(&bgp, &sources, &[]).unwrap();
        let mut reordered = in_order.clone();
        reordered.per_pattern.reverse();

        let a = prune_source_combinations(&bgp, &sources, &in_order).unwrap();
        let b = prune_source_combinations(&bgp, &sources, &reordered).unwrap();
        assert!(!a.dead_pairings.is_empty(), "the run is not vacuous");
        assert_eq!(a.dead_pairings, b.dead_pairings, "verdict is order-free");
        assert_eq!(a.empty_patterns, b.empty_patterns);
        assert_eq!(a.dead_patterns, b.dead_patterns);
        assert_eq!(a.pruned, b.pruned);
        assert_eq!(a.bgp_satisfiable, b.bgp_satisfiable);
        assert_eq!(a.components, b.components);
    }

    /// **Recall-safety against a DUPLICATED characteristic set.** The builder normalises predicate
    /// order but keeps a repeated key, and the completeness guard sums every served set carrying the
    /// predicate — so serving the `{sharesHouse}` cohort twice (and `{name}` twice) can make both
    /// sums equal `void:distinctSubjects` while the real `{sharesHouse, name}` co-occurrence set is
    /// omitted. Two entries under one exact-predicate-set key are not a partition, so nothing is
    /// proved: the guard must decline rather than emit a dead pairing that prunes a live
    /// combination.
    #[test]
    fn duplicate_char_sets_decline_the_prune() {
        let bgp = star_bgp();
        // `distinct_subjects` is 10 for sharesHouse (5+5) and 14 for name (7+7) — each balanced by
        // DUPLICATE singleton cohorts, and no served set carries both predicates.
        let sources = vec![cs_source(
            "http://dup/",
            &[
                (&[SHARES_HOUSE], 5),
                (&[SHARES_HOUSE], 5),
                (&[NAME], 7),
                (&[NAME], 7),
            ],
            Accounting::Exact,
        )];
        let selected = select_private_sources(&bgp, &sources, &[]).unwrap();
        // Precondition: both patterns retain the source, so the rule is genuinely evaluated.
        assert!(!selected.per_pattern[0].is_empty() && !selected.per_pattern[1].is_empty());

        let out = prune_source_combinations(&bgp, &sources, &selected).unwrap();
        assert!(
            out.dead_pairings.is_empty(),
            "a repeated characteristic-set key is not a partition and proves nothing: {:?}",
            out.dead_pairings
        );
        assert!(out.bgp_satisfiable);
        assert!(out.pruned.is_empty());
        assert!(!out.combination_is_dead(&[0, 0]));

        // Control (non-vacuity): the same shape with DISTINCT keys — one cohort per predicate — is
        // a valid partition and does prove the pairing dead.
        let honest = vec![split_source("http://a/", Accounting::Exact)];
        let sel = select_private_sources(&bgp, &honest, &[]).unwrap();
        let dead = prune_source_combinations(&bgp, &honest, &sel).unwrap();
        assert!(
            !dead.dead_pairings.is_empty(),
            "distinct cohorts still prove the pairing dead"
        );
    }
}

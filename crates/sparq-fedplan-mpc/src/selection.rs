//! **Phase 2** — the privacy/authorisation-aware source-selection adapter (design record
//! §4.3 pass 1 / §8 Phase 2; bead sq-fix4, epics sq-pwr / sq-0jsc).
//!
//! This module turns the [`select_private_sources`] stub from a
//! `SeamError::Deferred` into a real adapter: it runs [`sparq_fedplan::select_sources`] over the
//! BGP + source descriptors, then **prunes** any source that has declared it will not participate
//! for this query (the per-source [`SourcePrivacyDescriptor`]'s reserved authorisation field),
//! and surfaces the retained candidate set plus an audit trail of what was pruned and why.
//!
//! # What this phase DOES and does NOT prune (the load-bearing scope)
//!
//! This is **plumbing — source-selection routing, not a cryptographic guarantee.** It performs
//! **NO** MPC, runs **NO** secret-sharing, and reveals nothing it was not already given in the
//! clear (the descriptors are the caller's own inputs). The one and only pruning rule here is
//! **participation/authorisation**:
//!
//! * A source whose [`SourcePrivacyDescriptor::participates`] is `Some(false)` is dropped from
//!   every pattern's candidate list — it has declared it will not answer for this query.
//! * A source with `participates == Some(true)`, with `participates == None` (no authorisation
//!   policy declared at this tier — the reserved B7 hook), or with **no privacy descriptor at
//!   all**, is **kept**. Dropping a source we have no *non*-participation evidence for would
//!   silently lose answers — exactly the recall-unsafe behaviour the upstream
//!   [`select_sources`] is careful to avoid.
//!
//! **A private predicate does NOT cause a source to be pruned here.** Whether a source's
//! *content* must be disclosed in the clear or routed through MPC is the **disclosed/hidden
//! routing decision (Phase 3)**, not source selection. A source that holds a private predicate
//! the query needs is still a *candidate* — it just gets routed `Hidden` later. Pruning it at
//! selection time would drop a source that genuinely contributes to the answer, i.e. produce a
//! **wrong** result. So Phase 2 reads [`SourcePrivacyDescriptor::may_disclose`] for **no** prune
//! decision; that read is Phase 3's job.
//!
//! # Honesty / threat model (no claim beyond what is established)
//!
//! The MPC estate (`sparq-mpc`) is research-grade, **honest-majority semi-honest only**, and is
//! **not** externally audited — the accredited-cryptographer sign-off (sq-qhy4) and the coZK
//! re-audit (sq-9hrn) are pending. This adapter does **not** change that posture by one inch and
//! makes **no** soundness/privacy/security claim. Its documented assumption is the design
//! record's constraint C-B (§2.2): the privacy descriptor is the **source's own declaration**;
//! this adapter merely *reads* it to route, and a later phase has each source *re-enforce* it
//! fail-closed against the declared envelope. The adapter itself enforces nothing
//! cryptographic — see the crate `README.md` and the design record.
//!
//! [OPUS-4.8] sq-fix4.

use std::collections::BTreeMap;

use sparq_fedplan::{select_sources, Bgp, SourceDescriptor, SourceId};

use crate::seam::{SeamError, SeamPhase};
use crate::SourcePrivacyDescriptor;

/// One source retained for a pattern after the privacy/authorisation prune (Phase 2).
///
/// Mirrors [`sparq_fedplan::SourceCandidate`] but additionally carries the resolved
/// [`SourceId`] so a downstream phase can join it back to a [`SourcePrivacyDescriptor`] without
/// re-deriving the index→id map.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateCandidate {
    /// Index into the `sources` slice passed to [`select_private_sources`] (stable; the same
    /// index [`sparq_fedplan::SourceCandidate::source`] uses).
    pub source: usize,
    /// The resolved source id (the descriptor's [`SourceDescriptor::id`]). Carried so a later
    /// phase keys directly on it.
    pub source_id: SourceId,
    /// CostFed-style estimated triples this source contributes to the pattern, as computed by
    /// the upstream planner. Carried through unchanged — Phase 2 only prunes, never re-estimates.
    pub estimated_cardinality: f64,
}

/// The sources retained for one triple pattern after the Phase-2 prune.
///
/// The retained-candidate analogue of [`sparq_fedplan::PatternSources`], in the same ascending
/// source-index order (determinism preserved through the prune).
#[derive(Debug, Clone, PartialEq)]
pub struct PrivatePatternSources {
    /// Index of the pattern within the BGP (matches the upstream `PatternSources::pattern`).
    pub pattern: usize,
    /// Retained sources for this pattern, ascending by source index (deterministic).
    pub candidates: Vec<PrivateCandidate>,
}

impl PrivatePatternSources {
    /// Whether any source survived the prune for this pattern.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Total estimated cardinality across the retained sources (the per-pattern input size a
    /// later join planner would start from). A pass-through sum of the surviving candidates'
    /// upstream estimates — Phase 2 does not re-estimate.
    pub fn total_cardinality(&self) -> f64 {
        self.candidates
            .iter()
            .map(|c| c.estimated_cardinality)
            .sum()
    }
}

/// Why a (pattern, source) candidate was pruned by Phase 2. There is exactly one reason at this
/// tier — participation/authorisation — but the enum is `#[non_exhaustive]` so later phases (or
/// the deferred B7 access-control hook) can add reasons without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PruneReason {
    /// The source's [`SourcePrivacyDescriptor::participates`] is `Some(false)` — it has declared
    /// it will not answer for this query, so it is dropped from every pattern's candidate list.
    NonParticipating,
}

impl PruneReason {
    /// A short human label for the reason (for diagnostics / the audit trail).
    pub fn label(self) -> &'static str {
        match self {
            PruneReason::NonParticipating => {
                "source declared non-participating (participates=false)"
            }
        }
    }
}

/// One audit-trail entry: a (pattern, source) candidate the upstream planner retained but Phase 2
/// dropped, with the [`PruneReason`]. Surfaced so the prune is **auditable** — a relying party can
/// see exactly which sources were skipped and why, rather than the prune being a silent side
/// effect (the design record's "declared, not implicit" discipline, applied to source selection).
#[derive(Debug, Clone, PartialEq)]
pub struct PrunedCandidate {
    /// The pattern index the candidate was pruned from.
    pub pattern: usize,
    /// The source index that was pruned.
    pub source: usize,
    /// The resolved source id.
    pub source_id: SourceId,
    /// Why it was pruned.
    pub reason: PruneReason,
}

/// The output of the Phase-2 privacy/authorisation-aware source-selection adapter: the retained
/// per-pattern candidate sets, plus an audit trail of what was pruned.
///
/// This carries **no** secret material and runs **no** privacy-bearing logic — it is the (public)
/// candidate set the caller's own descriptors produced, with non-participating sources removed.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SelectedPrivateSources {
    /// Per-pattern retained sources, in BGP pattern order. Same length and order as the upstream
    /// [`select_sources`] output — a pattern with every source
    /// pruned keeps its (now-empty) entry, so pattern indices stay stable.
    pub per_pattern: Vec<PrivatePatternSources>,
    /// The candidates the upstream planner retained but Phase 2 pruned, with reasons. Empty when
    /// nothing was pruned. Ascending by `(pattern, source)` (deterministic).
    pub pruned: Vec<PrunedCandidate>,
}

impl SelectedPrivateSources {
    /// Whether any source survived the prune for **any** pattern.
    pub fn is_empty(&self) -> bool {
        self.per_pattern.iter().all(PrivatePatternSources::is_empty)
    }

    /// The set of distinct source ids that contribute to at least one pattern after the prune,
    /// in deterministic ascending order. Useful for a later phase that needs the participating
    /// holder set (e.g. to collect attestation-key ids).
    pub fn participating_sources(&self) -> Vec<&SourceId> {
        let mut ids: Vec<&SourceId> = self
            .per_pattern
            .iter()
            .flat_map(|p| p.candidates.iter().map(|c| &c.source_id))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

/// **Phase 2** — run [`sparq_fedplan::select_sources`] and prune by the per-source privacy /
/// authorisation descriptor (design record §4.3 pass 1 / §8 Phase 2).
///
/// Pipeline:
/// 1. Call the upstream cost-based [`select_sources`] to get the
///    recall-safe per-pattern candidate sets — Phase 2 reuses it verbatim, adding **no** new
///    selection logic.
/// 2. For each candidate, resolve its [`SourceId`] (via `sources[i].id()`) and look up the
///    matching [`SourcePrivacyDescriptor`] (matched by id). If that descriptor declares
///    [`participates`](SourcePrivacyDescriptor::participates) `== Some(false)`, the candidate is
///    **pruned** (and recorded in [`SelectedPrivateSources::pruned`]); otherwise it is **kept**.
///
/// **Recall-safe by construction:** a source is dropped *only* on the positive evidence
/// `participates == false`. A source with no privacy descriptor, or one with
/// `participates == None` / `Some(true)`, is always kept — pruning on the absence of a policy
/// would lose answers. **Disclosability is not consulted here:** routing a source's private
/// content through MPC vs in the clear is the Phase-3 decision, so a source holding a private
/// predicate stays a candidate. See the module docs for the full scope.
///
/// # Errors
///
/// Returns [`SeamError::DescriptorMismatch`] if the same [`SourceId`] appears more than once in
/// `privacy` (an ambiguous authorisation policy — the adapter refuses to guess which one binds).
/// Extra `privacy` entries for sources not in `sources` are ignored (harmless); a source in
/// `sources` with no `privacy` entry is treated as "no authorisation policy declared" and kept.
///
/// This function performs **no** MPC and makes **no** soundness/privacy claim — see the module
/// docs and the crate `README.md`. [OPUS-4.8] sq-fix4.
pub fn select_private_sources(
    bgp: &Bgp,
    sources: &[SourceDescriptor],
    privacy: &[SourcePrivacyDescriptor],
) -> Result<SelectedPrivateSources, SeamError> {
    // Build the SourceId -> privacy-descriptor index. A duplicate id is an ambiguous policy:
    // refuse rather than silently let the last one win (fail-closed on an ambiguous declaration).
    let mut by_id: BTreeMap<&SourceId, &SourcePrivacyDescriptor> = BTreeMap::new();
    for p in privacy {
        if by_id.insert(p.id(), p).is_some() {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceSelection,
                source_id: p.id().0.clone(),
                detail: "duplicate SourcePrivacyDescriptor for this source id (ambiguous authorisation policy)",
            });
        }
    }

    // 1. Upstream recall-safe source selection (reused verbatim).
    let upstream = select_sources(bgp, sources);

    // 2. Prune by participation/authorisation, preserving pattern + source order.
    let mut per_pattern: Vec<PrivatePatternSources> = Vec::with_capacity(upstream.len());
    let mut pruned: Vec<PrunedCandidate> = Vec::new();

    for ps in &upstream {
        let mut kept: Vec<PrivateCandidate> = Vec::with_capacity(ps.candidates.len());
        for cand in &ps.candidates {
            // `cand.source` is an index into `sources` (the upstream invariant); resolve its id.
            // Defensive: if the index is somehow out of range we keep the candidate rather than
            // panic — a missing id cannot be evidence of non-participation.
            let Some(src) = sources.get(cand.source) else {
                kept.push(PrivateCandidate {
                    source: cand.source,
                    source_id: SourceId::new(String::new()),
                    estimated_cardinality: cand.estimated_cardinality,
                });
                continue;
            };
            let id = src.id();
            let non_participating = by_id
                .get(id)
                .and_then(|p| p.participates())
                .map(|participates| !participates)
                .unwrap_or(false);

            if non_participating {
                pruned.push(PrunedCandidate {
                    pattern: ps.pattern,
                    source: cand.source,
                    source_id: id.clone(),
                    reason: PruneReason::NonParticipating,
                });
            } else {
                kept.push(PrivateCandidate {
                    source: cand.source,
                    source_id: id.clone(),
                    estimated_cardinality: cand.estimated_cardinality,
                });
            }
        }
        per_pattern.push(PrivatePatternSources {
            pattern: ps.pattern,
            candidates: kept,
        });
    }

    // `pruned` is already in (pattern, source) ascending order because the upstream output is
    // pattern-ordered and each pattern's candidates are source-index ascending — but make it
    // explicit so the audit trail's order is a guarantee, not an accident.
    pruned.sort_by_key(|p| (p.pattern, p.source));

    Ok(SelectedPrivateSources {
        per_pattern,
        pruned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_fedplan::{PredPartition, Term, TriplePattern};

    // ── Fixtures ────────────────────────────────────────────────────────────────────────
    // A source that holds predicate `p` (so the upstream selector keeps it for a `?s p ?o`
    // pattern), identified by `id`.
    fn source_with_pred(id: &str, pred: &str) -> SourceDescriptor {
        SourceDescriptor::builder(SourceId::new(id))
            .total_triples(100)
            .predicate(PredPartition {
                predicate: pred.to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .authorities_complete()
            .build()
    }

    // A single-pattern BGP `?s <pred> ?o`.
    fn bgp_pred(pred: &str) -> Bgp {
        Bgp::new(vec![TriplePattern::new(
            Term::Var(sparq_fedplan::Var::new("s")),
            Term::Iri(pred.to_string()),
            Term::Var(sparq_fedplan::Var::new("o")),
        )])
    }

    const PRED: &str = "http://ex/memberOf";

    // ── Tests ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn no_privacy_descriptors_keeps_the_upstream_selection_verbatim() {
        // With NO privacy descriptors at all, the adapter must be a pure pass-through of the
        // upstream selection: nothing pruned, every candidate retained, cardinalities preserved.
        let bgp = bgp_pred(PRED);
        let sources = vec![
            source_with_pred("http://a/", PRED),
            source_with_pred("http://b/", PRED),
        ];
        let upstream = select_sources(&bgp, &sources);

        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        assert!(
            sel.pruned.is_empty(),
            "nothing should be pruned without a policy"
        );
        assert_eq!(sel.per_pattern.len(), upstream.len());
        // Both sources retained for the one pattern, cardinalities carried through unchanged.
        assert_eq!(
            sel.per_pattern[0].candidates.len(),
            upstream[0].candidates.len()
        );
        for (got, want) in sel.per_pattern[0]
            .candidates
            .iter()
            .zip(&upstream[0].candidates)
        {
            assert_eq!(got.source, want.source);
            assert_eq!(got.estimated_cardinality, want.estimated_cardinality);
        }
        // Two distinct participating sources surfaced.
        assert_eq!(sel.participating_sources().len(), 2);
    }

    #[test]
    fn non_participating_source_is_pruned_and_recorded() {
        // Source B declares participates=false → it is dropped from the candidate set AND the
        // drop is recorded in the audit trail with the NonParticipating reason. Source A (no
        // policy) is kept. This is the load-bearing Phase-2 behaviour.
        let bgp = bgp_pred(PRED);
        let sources = vec![
            source_with_pred("http://a/", PRED),
            source_with_pred("http://b/", PRED),
        ];
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://b/"))
            .participates(false)
            .build()];

        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();

        // Exactly source A (index 0) survives.
        let surviving: Vec<usize> = sel.per_pattern[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(surviving, vec![0]);
        assert_eq!(
            sel.participating_sources(),
            vec![&SourceId::new("http://a/")]
        );

        // The prune is audited: B (index 1), pattern 0, NonParticipating.
        assert_eq!(sel.pruned.len(), 1);
        assert_eq!(sel.pruned[0].source, 1);
        assert_eq!(sel.pruned[0].pattern, 0);
        assert_eq!(sel.pruned[0].source_id, SourceId::new("http://b/"));
        assert_eq!(sel.pruned[0].reason, PruneReason::NonParticipating);
    }

    #[test]
    fn participates_true_and_none_are_both_kept() {
        // participates=true (explicit yes) and participates=None (no policy declared) must BOTH
        // keep the source — pruning is on positive non-participation evidence only (recall-safe).
        let bgp = bgp_pred(PRED);
        let sources = vec![
            source_with_pred("http://a/", PRED),
            source_with_pred("http://b/", PRED),
        ];
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .participates(true)
                .build(),
            // B: a descriptor with NO participation flag (the reserved-field default).
            SourcePrivacyDescriptor::deny_all(SourceId::new("http://b/")),
        ];
        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(sel.pruned.is_empty());
        assert_eq!(sel.per_pattern[0].candidates.len(), 2);
    }

    #[test]
    fn private_predicate_does_not_prune_the_source() {
        // A source whose predicate is marked Private (default-deny) but that PARTICIPATES must
        // STILL be a candidate — the disclosed/hidden routing is Phase 3, not source selection.
        // Pruning it here would lose answers; that would be a correctness bug, not a privacy win.
        let bgp = bgp_pred(PRED);
        let sources = vec![source_with_pred("http://a/", PRED)];
        // A's descriptor marks the predicate Private (i.e. NOT disclosable) but does not refuse
        // participation. It must remain a candidate.
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .private_predicate(PRED)
            .participates(true)
            .build()];
        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(
            sel.pruned.is_empty(),
            "a private (but participating) source must not be pruned"
        );
        assert_eq!(sel.per_pattern[0].candidates.len(), 1);
        // And nothing here discloses anything: may_disclose is NOT consulted for a prune.
        assert!(!privacy[0].may_disclose(PRED));
    }

    #[test]
    fn duplicate_descriptor_for_one_source_is_a_descriptor_mismatch() {
        // Two privacy descriptors for the same source id is an ambiguous authorisation policy:
        // the adapter must refuse (fail-closed), not silently pick one.
        let bgp = bgp_pred(PRED);
        let sources = vec![source_with_pred("http://a/", PRED)];
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .participates(true)
                .build(),
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .participates(false)
                .build(),
        ];
        let err = select_private_sources(&bgp, &sources, &privacy).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::SourceSelection,
                ..
            }
        ));
        assert!(format!("{}", err).contains("http://a/"));
    }

    #[test]
    fn extra_privacy_descriptor_for_an_absent_source_is_ignored() {
        // A privacy descriptor for a source NOT in `sources` is harmless — it simply never
        // matches. The selection is unaffected.
        let bgp = bgp_pred(PRED);
        let sources = vec![source_with_pred("http://a/", PRED)];
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://ghost/"))
                .participates(false)
                .build(),
        ];
        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(sel.pruned.is_empty());
        assert_eq!(sel.per_pattern[0].candidates.len(), 1);
    }

    #[test]
    fn all_sources_non_participating_yields_empty_but_well_formed_selection() {
        // If every source refuses, every pattern's candidate list is empty but the per-pattern
        // entries (and their indices) are preserved, and every drop is audited.
        let bgp = bgp_pred(PRED);
        let sources = vec![
            source_with_pred("http://a/", PRED),
            source_with_pred("http://b/", PRED),
        ];
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .participates(false)
                .build(),
            SourcePrivacyDescriptor::builder(SourceId::new("http://b/"))
                .participates(false)
                .build(),
        ];
        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(sel.is_empty());
        assert_eq!(sel.per_pattern.len(), 1);
        assert!(sel.per_pattern[0].is_empty());
        assert_eq!(sel.pruned.len(), 2);
        // Audit trail is ordered by (pattern, source).
        assert_eq!(sel.pruned[0].source, 0);
        assert_eq!(sel.pruned[1].source, 1);
        assert!(sel.participating_sources().is_empty());
    }

    #[test]
    fn determinism_same_inputs_same_output() {
        // The seam is deterministic throughout: same inputs ⇒ identical output, twice.
        let bgp = bgp_pred(PRED);
        let sources = vec![
            source_with_pred("http://a/", PRED),
            source_with_pred("http://b/", PRED),
        ];
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .participates(false)
            .build()];
        let a = select_private_sources(&bgp, &sources, &privacy).unwrap();
        let b = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert_eq!(a, b);
    }

    // A two-pattern BGP `?s p1 ?o . ?s p2 ?o2` (joins on ?s) so a source can appear in BOTH
    // patterns and a per-pattern cardinality sum is non-trivial. [OPUS-4.8] sq-bif.
    fn bgp_two_preds(p1: &str, p2: &str) -> Bgp {
        Bgp::new(vec![
            TriplePattern::new(
                Term::Var(sparq_fedplan::Var::new("s")),
                Term::Iri(p1.to_string()),
                Term::Var(sparq_fedplan::Var::new("o")),
            ),
            TriplePattern::new(
                Term::Var(sparq_fedplan::Var::new("s")),
                Term::Iri(p2.to_string()),
                Term::Var(sparq_fedplan::Var::new("o2")),
            ),
        ])
    }

    /// `total_cardinality` sums ONLY the surviving candidates, and it excludes a pruned one — a
    /// regression that summed pruned candidates (or the wrong field) would change the number. The
    /// surviving sum must equal the upstream per-pattern total minus the pruned source's estimate.
    /// [OPUS-4.8] sq-bif.
    #[test]
    fn total_cardinality_sums_only_surviving_candidates() {
        let pred = "http://ex/p";
        let bgp = bgp_pred(pred);
        let sources = vec![
            source_with_pred("http://a/", pred),
            source_with_pred("http://b/", pred),
        ];
        // Source B is non-participating ⇒ pruned; only A's estimate should remain in the total.
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://b/"))
            .participates(false)
            .build()];

        let upstream = select_sources(&bgp, &sources);
        let upstream_total = upstream[0].total_cardinality();
        // The pruned source's upstream estimate (B is index 1).
        let pruned_estimate = upstream[0]
            .candidates
            .iter()
            .find(|c| c.source == 1)
            .map(|c| c.estimated_cardinality)
            .expect("upstream retained B before the prune");

        let sel = select_private_sources(&bgp, &sources, &privacy).unwrap();
        let surviving_total = sel.per_pattern[0].total_cardinality();

        // The surviving total is the kept candidate's estimate — A only.
        assert_eq!(sel.per_pattern[0].candidates.len(), 1);
        assert_eq!(
            surviving_total,
            sel.per_pattern[0].candidates[0].estimated_cardinality
        );
        // …and it is strictly the upstream total minus the pruned source's contribution (so a
        // regression that left B in the sum would be caught).
        assert_eq!(surviving_total, upstream_total - pruned_estimate);
        assert!(
            surviving_total < upstream_total,
            "pruning a contributing source must lower the per-pattern total"
        );
    }

    /// `participating_sources` deduplicates a source that contributes to MULTIPLE patterns — it
    /// returns each distinct id once, ascending. A regression that concatenated per-pattern ids
    /// without the dedup would report the source twice.  [OPUS-4.8] sq-bif.
    #[test]
    fn participating_sources_dedups_a_source_across_patterns() {
        let p1 = "http://ex/p1";
        let p2 = "http://ex/p2";
        let bgp = bgp_two_preds(p1, p2);
        // One source holding BOTH predicates ⇒ it is a candidate for pattern 0 AND pattern 1.
        let sources = vec![SourceDescriptor::builder(SourceId::new("http://a/"))
            .total_triples(100)
            .predicate(PredPartition {
                predicate: p1.to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .predicate(PredPartition {
                predicate: p2.to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .authorities_complete()
            .build()];
        let sel = select_private_sources(&bgp, &sources, &[]).unwrap();
        // The source survives in both patterns…
        assert_eq!(sel.per_pattern.len(), 2);
        assert_eq!(sel.per_pattern[0].candidates.len(), 1);
        assert_eq!(sel.per_pattern[1].candidates.len(), 1);
        // …but participating_sources reports it exactly ONCE (deduped), not twice.
        assert_eq!(
            sel.participating_sources(),
            vec![&SourceId::new("http://a/")]
        );
    }

    /// The audit-trail `PruneReason::label` is the human-readable diagnostic surfaced to a relying
    /// party. Pin its content (a regression that blanked or reworded it past the load-bearing
    /// `participates=false` token would be caught). [OPUS-4.8] sq-bif.
    #[test]
    fn prune_reason_label_names_the_non_participation_cause() {
        let label = PruneReason::NonParticipating.label();
        assert!(
            label.contains("non-participating"),
            "label must name the cause: {label}"
        );
        assert!(
            label.contains("participates=false"),
            "label must name the load-bearing flag: {label}"
        );
    }
}

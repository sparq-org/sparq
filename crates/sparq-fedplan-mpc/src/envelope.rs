//! **Phase 4** — the leakage-envelope assembly + dual ratification (design record §2.1(3) /
//! §4.3 pass 3 / §8 Phase 4; bead sq-pwr.2, epics sq-pwr / sq-0jsc).
//!
//! This module turns the [`assemble_leakage_envelope`] and
//! [`ratify_envelope`] surface from a `SeamError::Deferred` stub into a real, honest
//! leakage-accounting + ratification pass over the Phase-3 routing. Given the Phase-3
//! [`PrivateRouting`], the [`QueryOperator`]s it was derived from, the participating sources
//! ([`SelectedPrivateSources`]), and the per-source [`SourcePrivacyDescriptor`]s, it:
//!
//! 1. **Assembles a declared [`LeakageEnvelope`]** — the explicit, enumerated statement of what
//!    the chosen routing+disclosure plan reveals (the design record's "declared bounded leak", the
//!    Conclave lesson: a leak must be *declared*, never implicit). It enumerates the operator
//!    structure, the disclose/hide partition, the disclosed operands, and which sources
//!    participate — **honestly over-counting rather than under-counting** what is revealed.
//! 2. **Runs the dual ratification** ([`ratify_envelope`]): each **holder** fail-closed-rejects a
//!    envelope that *declares* a disclosure of one of *its own* private predicates (constraint
//!    C-B; it checks the declared operand labels only, not the executed operators), AND the
//!    **verifier** ratifies the envelope against an acceptance policy (an over-leaking envelope —
//!    one whose disclosed-operand count exceeds the verifier's declared budget — is rejected).
//!    Only a plan that survives **both** ratifications is cleared for execution.
//!
//! # The honest leakage enumeration (what the plan reveals — over-count, never under-count)
//!
//! This is the load-bearing honesty obligation: the envelope must **enumerate what is revealed,
//! and must not under-claim.** The plan's [`LeakageEnvelope`] declares:
//!
//! * **Operator structure** — the *count* of operators, and the ordered `(label, class)` of each.
//!   This always leaks: a `Vec<OperatorRouting>` *is* metadata about the query shape (how many
//!   operators of which class, in which order), regardless of the disclose/hide split.
//! * **The disclose/hide partition** — for each operator, whether it was routed
//!   [`Routing::Disclosed`] (recomputed in the clear) or [`Routing::Hidden`] (under MPC). The
//!   partition itself is revealed — an observer learns *which* parts of the query ran in the clear.
//! * **The disclosed operands** — for each `Disclosed` operator, the operands whose bindings are
//!   revealed in the clear (the global IRIs and the explicitly-public predicates). These are the
//!   actual facts the disclose route exposes.
//! * **Source participation** — the distinct set of sources that contribute (after the Phase-2
//!   prune). *That a source participates at all* is itself disclosed by the plan.
//!
//! # What the envelope does NOT claim (honest boundary)
//!
//! This is **leakage-accounting + ratification plumbing — NOT a cryptographic guarantee.** It
//! performs **NO** MPC, runs **NO** secret-sharing, opens nothing, and proves nothing. The
//! envelope is a *declaration* of what the plan reveals, and the ratification is a *policy check*
//! over that declaration — neither is a cryptographic enforcement. The fail-closed holder check is
//! the design record's constraint C-B (§2.2): the disclosure decision belongs to the data owners
//! and the verifier, not the planner, so an envelope that *declares* a disclosure of a holder's
//! private predicate is **rejected here, not honoured**. The check attests only the declared
//! operand list: the caller must build the envelope from the actual operators with
//! [`assemble_leakage_envelope`], keep it in step with what executes, and rebuild and re-ratify
//! after any change. A planner that also writes the envelope can mislabel a disclosure, and
//! nothing here detects that. This is a *plan-time policy gate*, not a runtime guarantee
//! that a malicious holder/verifier honours it; it makes **NO** soundness/privacy/security claim.
//! The MPC estate (`sparq-mpc`) is research-grade, **honest-majority semi-honest only**, and is
//! **NOT** externally audited — the accredited-cryptographer sign-off (sq-qhy4) and the coZK
//! re-audit (sq-9hrn) are pending. This pass does not change that posture by one inch. See the
//! crate `README.md` and `research/mpc-untrusted-planner-routing-design.md` §2.1 / §2.2 / §4.4 / §6.
//!
//! sq-pwr.2.

use std::collections::{BTreeMap, BTreeSet};

use sparq_fedplan::SourceId;
use sparq_mpc::pipeline::Routing;

use crate::routing::{validate_operands, Operand, QueryOperator, ANONYMOUS_GLOBAL_IRI};
use crate::seam::{
    LeakageEnvelope, OperatorDisclosure, PrivateRouting, RatificationOutcome, SeamError, SeamPhase,
};
use crate::selection::SelectedPrivateSources;
use crate::SourcePrivacyDescriptor;

/// **Phase 4** — assemble the declared [`LeakageEnvelope`] of what the Phase-3 routing reveals
/// (design record §2.1(3) / §4.3 pass 3).
///
/// Derives, from the Phase-3 `routing` and the `operators` it was computed over, the explicit
/// enumeration of what the chosen disclose/hide plan reveals: the operator structure, the
/// disclose/hide partition, the operands each `Disclosed` operator exposes, and (from `selected`)
/// which sources participate. The envelope **over-counts rather than under-counts** — it is the
/// honest upper statement of the leak, never a minimised one.
///
/// `routing` and `operators` must describe the **same** operator sequence in the **same** order
/// (the routing was computed from the operators by [`route_operators`](crate::route_operators)).
///
/// # Errors
///
/// Returns [`SeamError::DescriptorMismatch`] (phase [`SeamPhase::LeakageEnvelope`]) if an
/// operand's predicate id is empty or the reserved [`ANONYMOUS_GLOBAL_IRI`] label, or if `routing`
/// and `operators` disagree in length or in an operator's label — an envelope cannot honestly
/// enumerate what is disclosed if the routing it accounts for does not match the operators it
/// claims to describe (fail-closed: refuse rather than emit a misleading envelope).
///
/// This function performs **no** MPC and makes **no** soundness/privacy claim — it produces a
/// *declaration*. See the module docs and the crate `README.md`. sq-pwr.2.
pub fn assemble_leakage_envelope(
    routing: &PrivateRouting,
    operators: &[QueryOperator],
    selected: &SelectedPrivateSources,
) -> Result<LeakageEnvelope, SeamError> {
    validate_operands(operators, SeamPhase::LeakageEnvelope)?;

    // The envelope can only honestly account for the routing if it was computed over THESE
    // operators: same count, and the labels line up (the routing carries the operator label
    // verbatim). A mismatch is fail-closed — refuse rather than emit a leakage statement that
    // does not correspond to the plan it claims to describe.
    if routing.routing.len() != operators.len() {
        return Err(SeamError::DescriptorMismatch {
            phase: SeamPhase::LeakageEnvelope,
            source_id: String::new(),
            detail: "routing/operator count mismatch (envelope cannot account for a plan it does not match)",
        });
    }

    let mut disclosures: Vec<OperatorDisclosure> = Vec::with_capacity(operators.len());
    for (route, op) in routing.routing.iter().zip(operators) {
        if route.operator != op.operator {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::LeakageEnvelope,
                source_id: String::new(),
                detail: "routing/operator label mismatch (envelope cannot account for a plan it does not match)",
            });
        }
        let disclosed = matches!(route.routing, Routing::Disclosed);
        // When disclosed, the operator's operands are revealed in the clear — enumerate the
        // predicate id behind each, deterministically (BTreeSet) and honestly (every operand). A
        // hidden operator discloses no operand (it runs under MPC), so the set is empty.
        let disclosed_operands: Vec<String> = if disclosed {
            op.operands
                .iter()
                .map(operand_disclosure_label)
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        disclosures.push(OperatorDisclosure {
            operator: op.operator.clone(),
            class: op.class,
            disclosed,
            disclosed_operands,
        });
    }

    // Source participation is itself disclosed by the plan: enumerate the distinct participating
    // source ids (deterministic ascending order, from the Phase-2 selection).
    let participating_sources: Vec<String> = selected
        .participating_sources()
        .into_iter()
        .map(|id| id.0.clone())
        .collect();

    Ok(LeakageEnvelope {
        operators: disclosures,
        participating_sources,
    })
}

/// The honest label for what a disclosed `operand` reveals: a global IRI reveals the bound IRI's
/// predicate (the membership key); a predicate operand reveals that predicate's bindings. Either
/// way, the *predicate id* is the unit of disclosure recorded in the envelope.
fn operand_disclosure_label(operand: &Operand) -> String {
    match operand {
        // A global-IRI operand reveals which predicate it binds (e.g. the membership key on the
        // flat IRI). The empty-predicate case (an IRI binding no specific predicate) records the
        // honest fact that *a* global IRI is disclosed, labelled as such.
        Operand::GlobalIri { predicate } if predicate.is_empty() => {
            ANONYMOUS_GLOBAL_IRI.to_string()
        }
        Operand::GlobalIri { predicate } => predicate.clone(),
        Operand::Predicate(p) => p.clone(),
    }
}

/// **Phase 4** — the dual-ratification gate over a declared [`LeakageEnvelope`] (design record
/// §4.3 pass 3 / §4.4 constraint C-B).
///
/// A plan is cleared for execution **only** if it survives *both* ratifications:
///
/// 0. **Policy coverage.** `holders` must hold exactly one descriptor for each source in
///    `envelope.participating_sources` and none for any other source. A missing participant
///    policy (including an empty `holders` for a nonempty participation set) and a descriptor
///    for a non-participating source are both rejected as
///    [`RatificationOutcome::ParticipationMismatch`], so a holder can neither be skipped nor be
///    substituted by an unrelated one. An envelope with no participating source that discloses
///    any operand is rejected the same way: with no holder, nobody consented to the disclosure.
/// 1. **Holder fail-closed re-check (constraint C-B).** Each `holder` descriptor independently
///    re-derives whether the envelope discloses one of *its own* private predicates. If the plan
///    routes an operator `Disclosed` whose operands include a predicate the holder has **not**
///    marked [`Disclosability::Public`](crate::Disclosability::Public), the holder **rejects**
///    (fail-closed) — the over-disclosing plan is *not honoured*. This checks the declaration
///    only: it does not by itself enforce C-B against a planner that mislabels the envelope.
/// 2. **Verifier acceptance policy.** The verifier rejects an **over-leaking** envelope — one
///    whose total count of distinct disclosed operands exceeds `verifier_budget`. `None` means
///    "no budget cap" (accept any envelope the holders cleared). This is the verifier-side
///    "the leak is bounded and declared" check.
///
/// **No exemptions.** Every disclosed operand goes through the holder re-check, including the
/// [`ANONYMOUS_GLOBAL_IRI`] label recorded for a disclosed global IRI that binds no predicate: a
/// holder consents to that disclosure only by marking the label public.
///
/// **Trust boundary.** Ratification attests only the declared operand list; it does not
/// re-derive labels from the typed operators. Build `envelope` with
/// [`assemble_leakage_envelope`] from the operators that will run, keep that correspondence
/// through execution, and rebuild and re-ratify after any change.
///
/// Returns a [`RatificationOutcome`] enumerating, honestly, *which* ratification(s) failed and
/// *why* — a declared over-disclosure is reported with the offending source + predicate (holder
/// rejection) or the budget overrun (verifier rejection), never silently dropped.
///
/// This is a **plan-time policy gate, not a cryptographic enforcement.** It runs **no** MPC and
/// makes **no** soundness/privacy claim — see the module docs. sq-pwr.2.
pub fn ratify_envelope(
    envelope: &LeakageEnvelope,
    holders: &[SourcePrivacyDescriptor],
    verifier_budget: Option<usize>,
) -> RatificationOutcome {
    // Index the holders by id, fail-closed on a duplicate (an ambiguous policy — the same posture
    // as Phase 2 / Phase 3; a single holder must not present two conflicting disclosure policies).
    let mut by_id: BTreeMap<&SourceId, &SourcePrivacyDescriptor> = BTreeMap::new();
    let mut duplicate: Option<String> = None;
    for h in holders {
        if by_id.insert(h.id(), h).is_some() {
            duplicate = Some(h.id().0.clone());
        }
    }
    if let Some(dup) = duplicate {
        return RatificationOutcome::HolderRejected {
            source_id: dup,
            predicate: String::new(),
            detail:
                "duplicate SourcePrivacyDescriptor for this holder (ambiguous disclosure policy)",
        };
    }

    // 0. Policy coverage: the holder set must be exactly the participation set. A participant
    // without a policy is never skipped, and a policy for a non-participant is refused rather than
    // silently ignored or allowed to stand in for a missing one.
    let participants: BTreeSet<&str> = envelope
        .participating_sources
        .iter()
        .map(String::as_str)
        .collect();
    if let Some(missing) = participants
        .iter()
        .find(|id| !by_id.keys().any(|h| h.0 == **id))
    {
        return RatificationOutcome::ParticipationMismatch {
            source_id: (*missing).to_string(),
            detail: "no SourcePrivacyDescriptor for this participating holder (a missing policy fails closed)",
        };
    }
    if let Some(extra) = by_id.keys().find(|h| !participants.contains(h.0.as_str())) {
        return RatificationOutcome::ParticipationMismatch {
            source_id: extra.0.clone(),
            detail: "SourcePrivacyDescriptor for a source outside the envelope's participation set",
        };
    }
    if participants.is_empty() && !envelope.distinct_disclosed_operands().is_empty() {
        return RatificationOutcome::ParticipationMismatch {
            source_id: String::new(),
            detail: "plan discloses an operand but declares no participating holder to consent",
        };
    }

    // 1. Holder fail-closed re-check (constraint C-B). The most-private holder wins: if ANY holder
    // would have one of its private predicates disclosed by the plan, the whole plan is rejected.
    // Deterministic order: scan the disclosed operands per operator, then the holders by id.
    for op in &envelope.operators {
        if !op.disclosed {
            continue;
        }
        for predicate in &op.disclosed_operands {
            for (id, holder) in &by_id {
                if !holder.may_disclose(predicate) {
                    return RatificationOutcome::HolderRejected {
                        source_id: id.0.clone(),
                        predicate: predicate.clone(),
                        detail: "plan discloses a predicate this holder marked private (fail-closed C-B re-check)",
                    };
                }
            }
        }
    }

    // 2. Verifier acceptance policy: reject an over-leaking envelope. The leak metric is the total
    // count of DISTINCT disclosed operands across the plan — the honest "how much is revealed in
    // the clear" number the verifier bounds. (Operator structure always leaks and is not counted
    // against the budget; the budget caps the *operand* disclosure the plan chose.)
    if let Some(budget) = verifier_budget {
        let disclosed_total = envelope.distinct_disclosed_operands().len();
        if disclosed_total > budget {
            return RatificationOutcome::VerifierRejected {
                disclosed: disclosed_total,
                budget,
                detail: "envelope discloses more operands than the verifier's declared budget",
            };
        }
    }

    RatificationOutcome::Ratified
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_fedplan::{Bgp, PredPartition, SourceDescriptor, Term, TriplePattern, Var};
    use sparq_mpc::backend::OperatorClass;

    use crate::routing::{route_operators, RoutingPolicy};
    use crate::selection::select_private_sources;

    const MEMBER_OF: &str = "http://ex/memberOf";
    const SALARY: &str = "http://ex/salary";
    const FLAT_IRI: &str = "http://ex/flat";

    // ── Fixtures ────────────────────────────────────────────────────────────────────────

    fn source(id: &str) -> SourceDescriptor {
        SourceDescriptor::builder(SourceId::new(id))
            .total_triples(200)
            .predicate(PredPartition {
                predicate: MEMBER_OF.to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 1,
            })
            .predicate(PredPartition {
                predicate: SALARY.to_string(),
                triples: 100,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .authorities_complete()
            .build()
    }

    fn flatmate_privacy(id: &str) -> SourcePrivacyDescriptor {
        SourcePrivacyDescriptor::builder(SourceId::new(id))
            .public_predicate(MEMBER_OF)
            .private_predicate(SALARY)
            .participates(true)
            .build()
    }

    // The three operators of the four-flatmates query, in pipeline order.
    fn four_flatmates_operators() -> Vec<QueryOperator> {
        vec![
            QueryOperator::new(
                "membership-join (global-IRI key-on-key)",
                OperatorClass::EqualityJoin,
                vec![Operand::GlobalIri {
                    predicate: MEMBER_OF.to_string(),
                }],
            ),
            QueryOperator::new(
                "salary-cumulative-sum",
                OperatorClass::LinearAggregate,
                vec![Operand::Predicate(SALARY.to_string())],
            ),
            QueryOperator::new(
                "salary-threshold (> 100000)",
                OperatorClass::Comparison,
                vec![Operand::Predicate(SALARY.to_string())],
            ),
        ]
    }

    // The full Phase-2 → Phase-3 → envelope pipeline over the four-flatmates query, returning
    // the (selected, routing, operators, privacy) so a test can assemble + ratify.
    fn four_flatmates_plan() -> (
        SelectedPrivateSources,
        PrivateRouting,
        Vec<QueryOperator>,
        Vec<SourcePrivacyDescriptor>,
    ) {
        let sources = vec![source("http://a/"), source("http://b/")];
        let privacy = vec![flatmate_privacy("http://a/"), flatmate_privacy("http://b/")];
        let bgp = Bgp::new(vec![TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )]);
        let selected = select_private_sources(&bgp, &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        (selected, routing, ops, privacy)
    }

    // ── Assembly tests ────────────────────────────────────────────────────────────────────

    /// The load-bearing leakage enumeration over the four-flatmates plan: the envelope declares
    /// the operator structure, the disclose/hide partition, the disclosed operands, and the
    /// participating sources — honestly, over the REAL Phase-3 routing.
    #[test]
    fn assembles_the_four_flatmates_envelope_honestly() {
        let (selected, routing, ops, _privacy) = four_flatmates_plan();
        let env = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();

        // Operator structure: all three operators enumerated, in order, with their class.
        assert_eq!(env.operators.len(), 3);
        assert_eq!(
            env.operators[0].operator,
            "membership-join (global-IRI key-on-key)"
        );
        assert_eq!(env.operators[0].class, OperatorClass::EqualityJoin);
        assert_eq!(env.operators[1].class, OperatorClass::LinearAggregate);
        assert_eq!(env.operators[2].class, OperatorClass::Comparison);

        // The disclose/hide partition: membership disclosed, both salary ops hidden.
        assert!(env.operators[0].disclosed);
        assert!(!env.operators[1].disclosed);
        assert!(!env.operators[2].disclosed);

        // The disclosed operand of the membership join is the memberOf key (in the clear); the
        // hidden salary ops disclose NO operand.
        assert_eq!(env.operators[0].disclosed_operands, vec![MEMBER_OF]);
        assert!(env.operators[1].disclosed_operands.is_empty());
        assert!(env.operators[2].disclosed_operands.is_empty());

        // Exactly the memberOf predicate is disclosed across the whole plan — salary never is.
        assert_eq!(
            env.distinct_disclosed_operands(),
            vec![MEMBER_OF.to_string()]
        );
        assert!(!env
            .distinct_disclosed_operands()
            .iter()
            .any(|p| p == SALARY));

        // Source participation is declared: both holders contribute.
        assert_eq!(env.participating_sources, vec!["http://a/", "http://b/"]);
    }

    /// A routing/operator length mismatch is fail-closed (the envelope refuses to account for a
    /// plan it does not match), not a silently-wrong envelope.
    #[test]
    fn length_mismatch_is_fail_closed() {
        let (selected, routing, ops, _) = four_flatmates_plan();
        // Drop the last operator so routing (3) and operators (2) disagree.
        let short_ops = ops[..2].to_vec();
        let err = assemble_leakage_envelope(&routing, &short_ops, &selected).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::LeakageEnvelope,
                ..
            }
        ));
        assert!(format!("{}", err).contains("count mismatch"));
    }

    /// A label mismatch (same count, different operator) is fail-closed.
    #[test]
    fn label_mismatch_is_fail_closed() {
        let (selected, routing, mut ops, _) = four_flatmates_plan();
        ops[0].operator = "a-different-operator".to_string();
        let err = assemble_leakage_envelope(&routing, &ops, &selected).unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::LeakageEnvelope,
                ..
            }
        ));
        assert!(format!("{}", err).contains("label mismatch"));
    }

    // ── Ratification tests ──────────────────────────────────────────────────────────────

    /// The happy path: the four-flatmates envelope discloses only the (public) memberOf key, so
    /// both holders accept and the verifier (generous budget) ratifies.
    #[test]
    fn four_flatmates_envelope_is_dual_ratified() {
        let (selected, routing, ops, privacy) = four_flatmates_plan();
        let env = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        let outcome = ratify_envelope(&env, &privacy, Some(8));
        assert_eq!(outcome, RatificationOutcome::Ratified);
        assert!(outcome.is_ratified());
    }

    /// NEGATIVE (holder): an envelope that declares a disclosure of a predicate a holder marked
    /// PRIVATE is rejected by that holder, fail-closed (constraint C-B, over the declaration).
    #[test]
    fn holder_rejects_a_plan_disclosing_a_private_term() {
        // Build an envelope that explicitly declares a disclosure of the SALARY predicate. The
        // holder marks salary PRIVATE, so its fail-closed re-check must reject it.
        let leaking_env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "salary-threshold-DISCLOSED (over-disclosing)".to_string(),
                class: OperatorClass::Comparison,
                disclosed: true,
                disclosed_operands: vec![SALARY.to_string()],
            }],
            participating_sources: vec!["http://a/".to_string()],
        };
        let holders = vec![flatmate_privacy("http://a/")];
        let outcome = ratify_envelope(&leaking_env, &holders, None);
        assert!(!outcome.is_ratified());
        match outcome {
            RatificationOutcome::HolderRejected {
                source_id,
                predicate,
                ..
            } => {
                assert_eq!(source_id, "http://a/");
                assert_eq!(predicate, SALARY);
            }
            other => panic!("expected a holder rejection, got {:?}", other),
        }
    }

    /// The most-private holder wins: even if only ONE of several holders marks the disclosed
    /// predicate private, the whole plan is rejected.
    #[test]
    fn one_private_holder_rejects_the_whole_plan() {
        let env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "join-on-p".to_string(),
                class: OperatorClass::EqualityJoin,
                disclosed: true,
                disclosed_operands: vec![MEMBER_OF.to_string()],
            }],
            participating_sources: vec!["http://a/".to_string(), "http://b/".to_string()],
        };
        let holders = vec![
            // A marks memberOf public…
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .public_predicate(MEMBER_OF)
                .build(),
            // …but B leaves it private (default-deny) — B must reject.
            SourcePrivacyDescriptor::deny_all(SourceId::new("http://b/")),
        ];
        let outcome = ratify_envelope(&env, &holders, None);
        assert!(matches!(
            outcome,
            RatificationOutcome::HolderRejected { .. }
        ));
    }

    /// NEGATIVE (verifier): an over-leaking envelope — more disclosed operands than the verifier's
    /// declared budget — is rejected by the verifier even when every holder accepts.
    #[test]
    fn verifier_rejects_an_over_leaking_envelope() {
        // Three distinct disclosed (public) operands; the verifier budget is 2.
        let env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "wide-disclose".to_string(),
                class: OperatorClass::EqualityJoin,
                disclosed: true,
                disclosed_operands: vec![
                    "http://ex/p1".to_string(),
                    "http://ex/p2".to_string(),
                    "http://ex/p3".to_string(),
                ],
            }],
            participating_sources: vec!["http://a/".to_string()],
        };
        // The holder marks all three public, so the HOLDER check passes — the rejection is the
        // verifier's budget, not a holder's policy.
        let holder = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate("http://ex/p1")
            .public_predicate("http://ex/p2")
            .public_predicate("http://ex/p3")
            .build();
        let outcome = ratify_envelope(&env, &[holder], Some(2));
        match outcome {
            RatificationOutcome::VerifierRejected {
                disclosed, budget, ..
            } => {
                assert_eq!(disclosed, 3);
                assert_eq!(budget, 2);
            }
            other => panic!("expected a verifier rejection, got {:?}", other),
        }
    }

    /// The holder check runs BEFORE the verifier budget: a plan that both over-discloses a private
    /// term AND blows the budget is reported as a HOLDER rejection (the data owner's veto is
    /// primary — fail-closed on the most-specific privacy violation first).
    #[test]
    fn holder_rejection_precedes_verifier_budget() {
        let env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "leak".to_string(),
                class: OperatorClass::Comparison,
                disclosed: true,
                disclosed_operands: vec![SALARY.to_string(), "http://ex/other".to_string()],
            }],
            participating_sources: vec!["http://a/".to_string()],
        };
        // Holder marks salary private; budget is 0 (so the verifier would also reject). The holder
        // veto must win.
        let holder = flatmate_privacy("http://a/");
        let outcome = ratify_envelope(&env, &[holder], Some(0));
        assert!(matches!(
            outcome,
            RatificationOutcome::HolderRejected { .. }
        ));
    }

    /// A budget of `None` accepts any envelope the holders cleared (no verifier cap).
    #[test]
    fn no_budget_accepts_any_holder_cleared_envelope() {
        let env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "wide-but-public".to_string(),
                class: OperatorClass::EqualityJoin,
                disclosed: true,
                disclosed_operands: vec![
                    "http://ex/p1".to_string(),
                    "http://ex/p2".to_string(),
                    "http://ex/p3".to_string(),
                ],
            }],
            participating_sources: vec!["http://a/".to_string()],
        };
        let holder = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate("http://ex/p1")
            .public_predicate("http://ex/p2")
            .public_predicate("http://ex/p3")
            .build();
        assert_eq!(
            ratify_envelope(&env, &[holder], None),
            RatificationOutcome::Ratified
        );
    }

    /// A duplicate holder descriptor (ambiguous policy) is fail-closed-rejected, like Phase 2/3.
    #[test]
    fn duplicate_holder_descriptor_is_rejected() {
        let env = LeakageEnvelope {
            operators: vec![],
            participating_sources: vec![],
        };
        let holders = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .public_predicate(MEMBER_OF)
                .build(),
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .private_predicate(MEMBER_OF)
                .build(),
        ];
        let outcome = ratify_envelope(&env, &holders, None);
        match outcome {
            RatificationOutcome::HolderRejected { source_id, .. } => {
                assert_eq!(source_id, "http://a/")
            }
            other => panic!(
                "expected a holder rejection for the duplicate, got {:?}",
                other
            ),
        }
    }

    /// An all-hidden plan discloses NO operand, so the holders accept and the verifier ratifies
    /// even a budget of 0 (nothing is disclosed to count against it).
    #[test]
    fn all_hidden_plan_discloses_nothing_and_is_ratified() {
        let env = LeakageEnvelope {
            operators: vec![
                OperatorDisclosure {
                    operator: "sum".to_string(),
                    class: OperatorClass::LinearAggregate,
                    disclosed: false,
                    disclosed_operands: vec![],
                },
                OperatorDisclosure {
                    operator: "cmp".to_string(),
                    class: OperatorClass::Comparison,
                    disclosed: false,
                    disclosed_operands: vec![],
                },
            ],
            participating_sources: vec!["http://a/".to_string()],
        };
        let holder = flatmate_privacy("http://a/");
        assert_eq!(env.distinct_disclosed_operands().len(), 0);
        assert_eq!(
            ratify_envelope(&env, &[holder], Some(0)),
            RatificationOutcome::Ratified
        );
    }

    /// The end-to-end Phase-2 → 3 → 4 happy path: select, route, assemble, dual-ratify — all green
    /// over the real four-flatmates query, exercising the REAL path (no mock).
    #[test]
    fn end_to_end_four_flatmates_is_ratified() {
        let (selected, routing, ops, privacy) = four_flatmates_plan();
        let env = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        // The plan discloses only the public memberOf key → both holders accept, verifier ratifies.
        let outcome = ratify_envelope(&env, &privacy, Some(4));
        assert!(outcome.is_ratified(), "four-flatmates plan must ratify");
    }

    // ── Empty global-IRI operand: the disclosed-by-convention path (envelope label + C-B skip) ──

    // A single-operator plan that discloses one global IRI binding NO specific predicate, plus the
    // routing that matches it (so `assemble_leakage_envelope` accepts the pair). Returns
    // (routing, ops). sq-bif.
    fn disclosed_empty_global_iri_plan() -> (PrivateRouting, Vec<QueryOperator>) {
        let ops = vec![QueryOperator::new(
            "membership-join (anonymous global IRI)",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                // Empty predicate ⇒ a global IRI binding no specific predicate.
                predicate: String::new(),
            }],
        )];
        // No contributing privacy descriptor at all ⇒ the default policy discloses the global IRI.
        let routing = route_operators(
            &SelectedPrivateSources::default(),
            &[],
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(routing.routing[0].routing, Routing::Disclosed);
        (routing, ops)
    }

    /// An empty-predicate global-IRI operand, when disclosed, is recorded HONESTLY in the envelope
    /// as the `<global-iri>` placeholder (not silently dropped, not a panic) — the disclosure of
    /// *a* global IRI is enumerated even though it binds no named predicate. sq-bif.
    #[test]
    fn empty_global_iri_operand_is_recorded_as_the_placeholder() {
        let (routing, ops) = disclosed_empty_global_iri_plan();
        let env =
            assemble_leakage_envelope(&routing, &ops, &SelectedPrivateSources::default()).unwrap();
        assert_eq!(env.operators.len(), 1);
        assert!(env.operators[0].disclosed);
        assert_eq!(env.operators[0].disclosed_operands, vec!["<global-iri>"]);
        // It counts as one distinct disclosed operand across the plan (a real, declared leak).
        assert_eq!(
            env.distinct_disclosed_operands(),
            vec!["<global-iri>".to_string()]
        );
    }

    /// The placeholder gets no holder-check exemption: a deny-all holder rejects a disclosed
    /// anonymous global IRI, and a holder that explicitly marks the label public consents. A
    /// relabelled envelope therefore cannot use the label to skip a holder's veto.
    #[test]
    fn placeholder_global_iri_needs_explicit_holder_consent() {
        let (routing, ops) = disclosed_empty_global_iri_plan();
        let mut env =
            assemble_leakage_envelope(&routing, &ops, &SelectedPrivateSources::default()).unwrap();
        env.participating_sources = vec!["http://a/".to_string()];
        let deny_all = SourcePrivacyDescriptor::deny_all(SourceId::new("http://a/"));
        match ratify_envelope(&env, &[deny_all], None) {
            RatificationOutcome::HolderRejected { predicate, .. } => {
                assert_eq!(predicate, ANONYMOUS_GLOBAL_IRI)
            }
            other => panic!("expected a holder rejection, got {other:?}"),
        }
        let consenting = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(ANONYMOUS_GLOBAL_IRI)
            .build();
        assert_eq!(
            ratify_envelope(&env, &[consenting], None),
            RatificationOutcome::Ratified
        );
    }

    /// A private predicate relabelled as the placeholder in a hand-edited envelope is still vetoed
    /// by a holder that has not consented to the label.
    #[test]
    fn relabelled_private_predicate_is_still_vetoed() {
        let env = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "salary-threshold".to_string(),
                class: OperatorClass::Comparison,
                disclosed: true,
                disclosed_operands: vec![ANONYMOUS_GLOBAL_IRI.to_string()],
            }],
            participating_sources: vec!["http://a/".to_string()],
        };
        assert!(matches!(
            ratify_envelope(&env, &[flatmate_privacy("http://a/")], Some(1)),
            RatificationOutcome::HolderRejected { .. }
        ));
    }

    /// A disclosed placeholder with no participant is a disclosure nobody consented to: rejected.
    #[test]
    fn placeholder_disclosure_without_any_participant_is_rejected() {
        let (routing, ops) = disclosed_empty_global_iri_plan();
        let env =
            assemble_leakage_envelope(&routing, &ops, &SelectedPrivateSources::default()).unwrap();
        assert!(env.participating_sources.is_empty());
        assert!(matches!(
            ratify_envelope(&env, &[], Some(1)),
            RatificationOutcome::ParticipationMismatch { .. }
        ));
    }

    /// The placeholder counts against the VERIFIER budget like any disclosed operand: a budget of
    /// 0 rejects it even though the (consenting) holder accepted. sq-bif.
    #[test]
    fn placeholder_global_iri_counts_against_the_verifier_budget() {
        let (routing, ops) = disclosed_empty_global_iri_plan();
        let mut env =
            assemble_leakage_envelope(&routing, &ops, &SelectedPrivateSources::default()).unwrap();
        env.participating_sources = vec!["http://a/".to_string()];
        let holders = [SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(ANONYMOUS_GLOBAL_IRI)
            .build()];
        // Budget 0: one distinct disclosed operand (`<global-iri>`) exceeds it ⇒ verifier rejects.
        match ratify_envelope(&env, &holders, Some(0)) {
            RatificationOutcome::VerifierRejected {
                disclosed, budget, ..
            } => {
                assert_eq!(disclosed, 1);
                assert_eq!(budget, 0);
            }
            other => panic!(
                "expected a verifier rejection on a 0 budget, got {:?}",
                other
            ),
        }
        // Budget 1: it fits ⇒ ratified.
        assert_eq!(
            ratify_envelope(&env, &holders, Some(1)),
            RatificationOutcome::Ratified
        );
    }

    /// Phase-2 prune narrows the Phase-3 disclosure gating: a private holder that is PRUNED (it
    /// declared non-participation) no longer gates a predicate's disclosability, so a predicate the
    /// *remaining* sources all mark public becomes Disclosed — and the assembled envelope honestly
    /// records that disclosure. This is the cross-phase interaction (selection → routing →
    /// envelope) over the REAL path. sq-bif.
    #[test]
    fn pruned_private_holder_does_not_gate_disclosure_downstream() {
        // Source A marks memberOf public and participates; source B marks it PRIVATE but declares
        // non-participation, so Phase 2 prunes B. With only A contributing, memberOf is disclosable.
        let a_pub = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(MEMBER_OF)
            .participates(true)
            .build();
        let b_private_refuses = SourcePrivacyDescriptor::builder(SourceId::new("http://b/"))
            .private_predicate(MEMBER_OF)
            .participates(false)
            .build();
        let sources = vec![source("http://a/"), source("http://b/")];
        let privacy = vec![a_pub.clone(), b_private_refuses];
        let bgp = Bgp::new(vec![TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )]);
        let selected = select_private_sources(&bgp, &sources, &privacy).unwrap();
        // B was pruned (non-participating) ⇒ only A participates.
        assert_eq!(
            selected.participating_sources(),
            vec![&SourceId::new("http://a/")]
        );

        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(MEMBER_OF.to_string())],
        )];
        // Routed over the REAL selection: only A gates memberOf, and A marks it public ⇒ Disclosed.
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        assert_eq!(routing.routing[0].routing, Routing::Disclosed);

        let env = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        assert_eq!(env.operators[0].disclosed_operands, vec![MEMBER_OF]);
        // Only A is declared participating, and only A ratifies (B is no longer a holder of record).
        assert_eq!(
            ratify_envelope(&env, &[a_pub], Some(4)),
            RatificationOutcome::Ratified
        );
    }

    // ── Policy coverage: missing / extra holder policies fail closed ──────────────────────

    fn membership_disclosed_envelope(participants: &[&str]) -> LeakageEnvelope {
        LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "membership-join".to_string(),
                class: OperatorClass::EqualityJoin,
                disclosed: true,
                disclosed_operands: vec![MEMBER_OF.to_string()],
            }],
            participating_sources: participants.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn public_member_of(id: &str) -> SourcePrivacyDescriptor {
        SourcePrivacyDescriptor::builder(SourceId::new(id))
            .public_predicate(MEMBER_OF)
            .build()
    }

    /// An empty holder list for a nonempty participation set is rejected, naming the participant
    /// with no policy. Before the coverage check the holder loop had no iterations and ratified.
    #[test]
    fn empty_holders_for_a_nonempty_participation_set_are_rejected() {
        let env = membership_disclosed_envelope(&["http://a/"]);
        for budget in [None, Some(8)] {
            assert_eq!(
                ratify_envelope(&env, &[], budget),
                RatificationOutcome::ParticipationMismatch {
                    source_id: "http://a/".to_string(),
                    detail: "no SourcePrivacyDescriptor for this participating holder (a missing policy fails closed)",
                }
            );
        }
    }

    /// Partial omission: the supplied holder permits the disclosure, but a second participant has
    /// no policy. The plan is rejected and the unlisted participant named.
    #[test]
    fn partially_missing_holder_policies_are_rejected() {
        let env = membership_disclosed_envelope(&["http://a/", "http://b/"]);
        match ratify_envelope(&env, &[public_member_of("http://a/")], None) {
            RatificationOutcome::ParticipationMismatch { source_id, .. } => {
                assert_eq!(source_id, "http://b/")
            }
            other => panic!("expected a participation mismatch, got {other:?}"),
        }
    }

    /// A policy for a source outside the participation set is rejected rather than ignored, so it
    /// cannot stand in for a missing participant's policy.
    #[test]
    fn holder_policy_for_a_non_participant_is_rejected() {
        let env = membership_disclosed_envelope(&["http://a/"]);
        let holders = [public_member_of("http://a/"), public_member_of("http://c/")];
        match ratify_envelope(&env, &holders, None) {
            RatificationOutcome::ParticipationMismatch { source_id, .. } => {
                assert_eq!(source_id, "http://c/")
            }
            other => panic!("expected a participation mismatch, got {other:?}"),
        }
        // Substitution: the non-participant's policy in place of the participant's.
        match ratify_envelope(&env, &[public_member_of("http://c/")], None) {
            RatificationOutcome::ParticipationMismatch { source_id, .. } => {
                assert_eq!(source_id, "http://a/")
            }
            other => panic!("expected a participation mismatch, got {other:?}"),
        }
    }

    /// An envelope that discloses a predicate while declaring no participant has no holder to
    /// consent, so it is rejected; an all-hidden envelope with no participant still ratifies.
    #[test]
    fn disclosure_without_any_participant_is_rejected() {
        let env = membership_disclosed_envelope(&[]);
        assert!(matches!(
            ratify_envelope(&env, &[], None),
            RatificationOutcome::ParticipationMismatch { .. }
        ));
        let hidden = LeakageEnvelope {
            operators: vec![OperatorDisclosure {
                operator: "sum".to_string(),
                class: OperatorClass::LinearAggregate,
                disclosed: false,
                disclosed_operands: vec![],
            }],
            participating_sources: vec![],
        };
        assert_eq!(
            ratify_envelope(&hidden, &[], Some(0)),
            RatificationOutcome::Ratified
        );
    }

    /// Every participant's policy present and each marks the disclosed predicate public: the
    /// explicitly permitted case ratifies.
    #[test]
    fn complete_explicitly_permitting_policies_ratify() {
        let env = membership_disclosed_envelope(&["http://a/", "http://b/"]);
        let holders = [public_member_of("http://b/"), public_member_of("http://a/")];
        assert_eq!(
            ratify_envelope(&env, &holders, Some(1)),
            RatificationOutcome::Ratified
        );
    }

    /// The end-to-end omission path: a valid selection names source A, but routing and
    /// ratification are handed no policy for it. Routing refuses, and ratifying the envelope the
    /// selection would produce with no holder policy is rejected too.
    #[test]
    fn omitted_policy_is_refused_end_to_end() {
        let privacy = vec![flatmate_privacy("http://a/")];
        let bgp = Bgp::new(vec![TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )]);
        let selected = select_private_sources(&bgp, &[source("http://a/")], &privacy).unwrap();
        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                predicate: MEMBER_OF.to_string(),
            }],
        )];
        assert!(route_operators(&selected, &[], &ops, RoutingPolicy::Strict).is_err());

        // With the policy supplied, routing proceeds; the envelope still names A as participant.
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Strict).unwrap();
        let env = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        assert_eq!(env.participating_sources, vec!["http://a/"]);
        assert!(matches!(
            ratify_envelope(&env, &[], None),
            RatificationOutcome::ParticipationMismatch { .. }
        ));
        assert_eq!(
            ratify_envelope(&env, &privacy, None),
            RatificationOutcome::Ratified
        );
    }

    /// Assembly refuses an operand that would be recorded under the reserved placeholder label,
    /// so a predicate disclosure cannot be recorded as an anonymous global IRI.
    #[test]
    fn assembly_refuses_the_reserved_placeholder_as_a_predicate() {
        let ops = vec![QueryOperator::new(
            "disguised",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(ANONYMOUS_GLOBAL_IRI.to_string())],
        )];
        let routing = PrivateRouting {
            routing: vec![sparq_mpc::pipeline::OperatorRouting {
                operator: "disguised".to_string(),
                routing: Routing::Disclosed,
            }],
        };
        let err = assemble_leakage_envelope(&routing, &ops, &SelectedPrivateSources::default())
            .unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::LeakageEnvelope,
                ..
            }
        ));
    }

    /// Determinism: same plan ⇒ identical envelope + identical outcome, twice.
    #[test]
    fn determinism_same_plan_same_envelope_and_outcome() {
        let (selected, routing, ops, privacy) = four_flatmates_plan();
        let a = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        let b = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            ratify_envelope(&a, &privacy, Some(4)),
            ratify_envelope(&b, &privacy, Some(4))
        );
    }
}

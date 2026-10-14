//! **Phase 3** — the disclosed/hidden routing pass (design record §4.3 pass 2 / §8 Phase 3;
//! bead sq-i1wh2, epics sq-pwr / sq-0jsc).
//!
//! This module turns the [`route_operators`] stub from a `SeamError::Deferred` into a real,
//! policy-parameterised partition: given the Phase-2 selected sources, the per-source privacy
//! descriptors, and the query's operators, it classifies each operator as **disclosed** (runs
//! in the clear, outside the cryptographic core) or **hidden** (must route through the MPC
//! path) and emits the [`sparq_mpc::pipeline::OperatorRouting`] vector the pipeline already
//! consumes — the structure `sparq-mpc::pipeline` today receives hand-written.
//!
//! # The decision rule (design record §4.3 pass 2 — convention #4, applied greedily)
//!
//! Per operator, route **`Disclosed`** when *every* operand is disclosable in the clear, and
//! **`Hidden(class)`** otherwise — default-deny: an operator whose disclosability cannot be
//! affirmatively established is hidden. An operand is disclosable when:
//!
//! * it is a **global IRI** ([`Operand::GlobalIri`]) — public by convention #6 — **and** the
//!   policy is [`RoutingPolicy::Default`] (the strict policy drops this shortcut and treats a
//!   global IRI like a predicate: disclosed only when every contributing source marks the
//!   predicate it binds public), or
//! * it is a **predicate** ([`Operand::Predicate`]) that **every** contributing source has
//!   *explicitly* marked [`crate::Disclosability::Public`] via
//!   [`SourcePrivacyDescriptor::may_disclose`]. A predicate that *any* contributing source
//!   leaves private (the default) is **not** disclosable — the most-private source wins, so a
//!   single private holder forces the whole operator into the hidden route.
//!
//! [`RoutingPolicy::Strict`] honours the design record's §5 "hide even a public term" knob: a
//! global-IRI operand needs the same affirmative per-source public mark as a predicate, so a
//! source that leaves the bound predicate private (the default) routes the operator hidden even
//! though the default policy would disclose it. An empty contributing set never counts as
//! consent.
//!
//! **Policy coverage (fail-closed).** When a Phase-2 selection is supplied, every participating
//! source must have exactly one [`SourcePrivacyDescriptor`]; a missing or duplicated descriptor
//! is refused before any operator is classified. This is a plan-time policy check over the
//! supplied descriptors, not a cryptographic enforcement. This is the documented cost-vs-privacy knob (§5); the default policy is the cheap,
//! demo-matching route.
//!
//! When an operator is routed `Hidden`, the [`sparq_mpc::backend::OperatorClass`] it carries is
//! the caller-supplied class of the operator ([`QueryOperator::class`]) — a linear aggregate, an
//! equality/hidden-value join, or a comparison — so the pipeline reads off the exact per-operator
//! security tier via [`sparq_mpc::backend::MpcBackend::operator_security`]. This pass does **not**
//! choose the class; it chooses the *route*, and propagates the class the operator already has.
//!
//! # Honesty / threat model — what this pass IS and what it LEAKS (documented limitation)
//!
//! This is **routing plumbing — a partition over typed operators, not a cryptographic
//! guarantee.** It performs **NO** MPC, runs **NO** secret-sharing, opens nothing, and verifies
//! nothing. It makes **NO** soundness/privacy/security claim. The MPC estate (`sparq-mpc`) is
//! research-grade, **honest-majority semi-honest only**, and is **NOT** externally audited — the
//! accredited-cryptographer sign-off (sq-qhy4) and the coZK re-audit (sq-9hrn) are pending. This
//! pass does not change that posture by one inch.
//!
//! **What the routing decision itself reveals (a documented leakage assumption, not a bound).**
//! The output `Vec<OperatorRouting>` *is* metadata about the query: it discloses the **operator
//! structure** (how many operators, of which class, in which order) and **which operators were
//! chosen disclosed vs hidden** — i.e. the shape of the query and the disclose/hide partition
//! itself. It does **not** reveal operand *values* or result *cardinalities* (those live in the
//! later evaluation, not in this typed plan). The disclose decision is the design record's
//! constraint C-B (§2.2): the planner here only *proposes* the partition; a later phase (Phase 4)
//! has each source **re-enforce** its own policy fail-closed and the verifier ratify the declared
//! leakage envelope assembled from these operators, so a declared over-disclosure is rejected,
//! not honoured (a check of the declaration only; see [`crate::ratify_envelope`]). This pass
//! does **not** perform that ratification — it computes the proposed partition only. See the
//! crate `README.md` and `research/mpc-untrusted-planner-routing-design.md` §2.2 / §4.4 / §6.
//!
//! sq-i1wh2.

use std::collections::BTreeMap;

use sparq_fedplan::SourceId;
use sparq_mpc::backend::OperatorClass;
use sparq_mpc::pipeline::{OperatorRouting, Routing};

use crate::seam::{PrivateRouting, SeamError, SeamPhase};
use crate::selection::SelectedPrivateSources;
use crate::SourcePrivacyDescriptor;

/// How aggressively the pass discloses operands (design record §5 cost-vs-privacy knob / §8
/// Phase 3 "default mode vs strict mode"). The default policy is the cheap, demo-matching route;
/// the strict policy is the most-private one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutingPolicy {
    /// **Default** — disclose global-IRI operands (public by convention #6) and any predicate
    /// every contributing source has explicitly marked [`crate::Disclosability::Public`]. This
    /// is the cheapest route and matches the hand-written four-flatmates routing.
    #[default]
    Default,
    /// **Strict** — honour each source's "hide even a public term" mark: a global-IRI operand is
    /// disclosed only when there is at least one contributing source and every contributing
    /// source marks the predicate it binds public — the same rule as for a predicate operand. An
    /// anonymous global IRI (empty predicate) is therefore always hidden under this policy.
    /// The most-private route — the §5 knob that hides a public-by-convention term on request.
    Strict,
}

/// One operand an operator reads — the unit the disclose/hide decision is taken over. An
/// operator is disclosable iff **all** of its operands are disclosable (default-deny otherwise).
///
/// This is the *typed* operand model, deliberately small: the pass decides on the operand's
/// **kind** (a global IRI vs a predicate) and its **predicate id** (to look up the per-source
/// disclosability), not on any operand *value*. No operand value is read here — the pass never
/// touches secret data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// A **global IRI** — a value that is public by convention #6 of the design record (e.g. the
    /// `ex:flat` membership key). `predicate` is the predicate whose object/subject this IRI
    /// binds, so the strict policy can still consult a source's "hide even this public term"
    /// mark for it. Disclosable in the [`RoutingPolicy::Default`] policy unconditionally; in
    /// [`RoutingPolicy::Strict`] only when every contributing source marks `predicate` public.
    GlobalIri {
        /// The predicate this global-IRI operand binds (for the strict-policy private-mark check
        /// and for diagnostics). Empty string when the operand binds no specific predicate.
        predicate: String,
    },
    /// A **predicate** operand whose bindings are private unless every contributing source has
    /// explicitly marked it [`crate::Disclosability::Public`] (default-deny). The salary
    /// predicate is the canonical private operand → forces the operator `Hidden`.
    Predicate(String),
}

impl Operand {
    /// The predicate id this operand is keyed on (for the per-source disclosability lookup).
    fn predicate(&self) -> &str {
        match self {
            Operand::GlobalIri { predicate } => predicate,
            Operand::Predicate(p) => p,
        }
    }
}

/// One operator of the federated query, as the routing pass sees it: a human label (carried
/// verbatim into the emitted [`OperatorRouting::operator`] so the produced routing is
/// byte-comparable to the hand-written one), the [`OperatorClass`] it runs as **when hidden**,
/// and the operands the disclose/hide decision is taken over.
///
/// The class is the operator's *own* property (a SUM is a [`OperatorClass::LinearAggregate`], a
/// hidden-value join is an [`OperatorClass::EqualityJoin`], a `>` is an
/// [`OperatorClass::Comparison`]) — this pass does not choose it, it only chooses the *route* and
/// propagates the class into the `Hidden(class)` arm. A disclosed operator drops the class (it
/// runs in the clear, so no MPC operator-class applies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOperator {
    /// Human-readable operator label (e.g. `"membership-join (global-IRI key-on-key)"`). Carried
    /// verbatim into [`OperatorRouting::operator`].
    pub operator: String,
    /// The MPC operator class this operator runs as **if routed hidden**. Ignored for a disclosed
    /// operator (it runs in the clear).
    pub class: OperatorClass,
    /// The operands the disclose/hide decision is taken over. An operator with **no** operands is
    /// vacuously disclosable (it reads nothing private) — but in practice every operator that
    /// touches data names at least one operand.
    pub operands: Vec<Operand>,
}

impl QueryOperator {
    /// A disclosed (in-the-clear) operator over `operands`, labelled `operator`. The `class` is
    /// recorded for completeness but is unused while the operator stays disclosed.
    pub fn new(operator: impl Into<String>, class: OperatorClass, operands: Vec<Operand>) -> Self {
        QueryOperator {
            operator: operator.into(),
            class,
            operands,
        }
    }
}

/// **Phase 3** — the policy-parameterised disclosed/hidden routing pass (design record §4.3
/// pass 2).
///
/// Classify each operator in `operators` as [`Routing::Disclosed`] (every operand disclosable in
/// the clear under `policy`) or [`Routing::Hidden`]`(class)` (default-deny otherwise), and emit
/// the [`OperatorRouting`] vector the `sparq-mpc` pipeline consumes — preserving operator order
/// (the seam is deterministic throughout).
///
/// The `selected` Phase-2 candidate set determines **which** sources contribute (and therefore
/// whose [`SourcePrivacyDescriptor`] gates a predicate's disclosability). When `selected` is
/// empty (no Phase-2 selection threaded in), the decision falls back to consulting **all** the
/// supplied `privacy` descriptors — so a caller that only wants the operator-level partition can
/// pass `SelectedPrivateSources::default()`.
///
/// # The disclosability rule (default-deny)
///
/// An operand is disclosable when (a) it is a [`Operand::GlobalIri`] under
/// [`RoutingPolicy::Default`] (public by convention #6), or (b) **every** contributing source
/// marks its predicate [`crate::Disclosability::Public`]. Under [`RoutingPolicy::Strict`] the
/// global-IRI shortcut is off: a global IRI needs that same affirmative mark on the predicate it
/// binds (the §5 "hide even a public term" knob), and no contributing source means no consent. An operator is disclosed iff **all** its
/// operands are — the most-private operand wins.
///
/// # Errors
///
/// Returns [`SeamError::DescriptorMismatch`] (phase [`SeamPhase::Routing`]), before any operator
/// is classified, if:
///
/// * the same [`SourceId`] appears more than once in `privacy` (an ambiguous policy — the pass
///   refuses to guess which one binds, the same fail-closed posture as Phase 2);
/// * `selected` is non-empty and one of its participating sources has no descriptor in `privacy`
///   (a missing policy is refused, never skipped); descriptors for sources outside the
///   participation set are ignored; or
/// * an operand's predicate id is empty (for [`Operand::Predicate`]) or the reserved
///   [`ANONYMOUS_GLOBAL_IRI`] label.
///
/// It never panics and performs **no** MPC.
///
/// This function makes **no** soundness/privacy claim and computes only the *proposed* partition;
/// the per-source fail-closed re-enforcement + verifier ratification of the leakage envelope is
/// the deferred, audit-gated Phase 4. See the module docs and the crate `README.md`.
/// sq-i1wh2.
pub fn route_operators(
    selected: &SelectedPrivateSources,
    privacy: &[SourcePrivacyDescriptor],
    operators: &[QueryOperator],
    policy: RoutingPolicy,
) -> Result<PrivateRouting, SeamError> {
    // Index the privacy descriptors by source id, refusing a duplicate (ambiguous policy) —
    // fail-closed, mirroring Phase 2 exactly.
    let mut by_id: BTreeMap<&SourceId, &SourcePrivacyDescriptor> = BTreeMap::new();
    for p in privacy {
        if by_id.insert(p.id(), p).is_some() {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::Routing,
                source_id: p.id().0.clone(),
                detail: "duplicate SourcePrivacyDescriptor for this source id (ambiguous disclosure policy)",
            });
        }
    }

    validate_operands(operators, SeamPhase::Routing)?;

    // The set of sources whose descriptor gates a predicate's disclosability. When the Phase-2
    // selection is non-empty, restrict to the participating sources it retained (only they
    // contribute operands); otherwise fall back to every supplied descriptor. Every participating
    // source must have exactly one descriptor: a missing one is refused before any operator is
    // classified, because dropping it would let the remaining sources' public marks disclose
    // data the unlisted source holds. Descriptors for non-participating sources are ignored.
    let contributing: Vec<&SourcePrivacyDescriptor> = if selected.is_empty() {
        privacy.iter().collect()
    } else {
        selected
            .participating_sources()
            .into_iter()
            .map(|id| {
                by_id
                    .get(id)
                    .copied()
                    .ok_or_else(|| SeamError::DescriptorMismatch {
                        phase: SeamPhase::Routing,
                        source_id: id.0.clone(),
                        detail: "no SourcePrivacyDescriptor for this participating source (a missing policy fails closed)",
                    })
            })
            .collect::<Result<_, _>>()?
    };

    let routing = operators
        .iter()
        .map(|op| {
            let route = if op
                .operands
                .iter()
                .all(|operand| operand_disclosable(operand, &contributing, policy))
            {
                Routing::Disclosed
            } else {
                Routing::Hidden(op.class)
            };
            OperatorRouting {
                operator: op.operator.clone(),
                routing: route,
            }
        })
        .collect();

    Ok(PrivateRouting { routing })
}

/// Whether one `operand` is disclosable in the clear under `policy`, given the `contributing`
/// sources' privacy descriptors. Default-deny: `false` unless an affirmative rule fires.
fn operand_disclosable(
    operand: &Operand,
    contributing: &[&SourcePrivacyDescriptor],
    policy: RoutingPolicy,
) -> bool {
    // A predicate is disclosable only when EVERY contributing source explicitly marks it public
    // (the most-private source wins). With no contributing source at all there is no holder that
    // has opted the predicate in, so default-deny keeps it hidden.
    let predicate = operand.predicate();
    let all_sources_disclose =
        !contributing.is_empty() && contributing.iter().all(|d| d.may_disclose(predicate));

    match operand {
        Operand::GlobalIri { .. } => match policy {
            // Default: a global IRI is public by convention #6, disclosed unconditionally.
            RoutingPolicy::Default => true,
            // Strict: the convention shortcut is off, so a global IRI needs the same affirmative
            // per-source mark as a predicate. An empty contributing set is not consent, and an
            // anonymous global IRI (empty predicate) has no predicate a source can mark public,
            // so both stay hidden.
            RoutingPolicy::Strict => !predicate.is_empty() && all_sources_disclose,
        },
        // A predicate operand is disclosable only by the affirmative per-source public mark,
        // identically under both policies (default-deny — no convention shortcut for a predicate).
        Operand::Predicate(_) => all_sources_disclose,
    }
}

/// The envelope label recorded for a disclosed [`Operand::GlobalIri`] that binds no predicate
/// (an anonymous global IRI). [`ratify_envelope`](crate::ratify_envelope) treats it like any
/// other disclosed operand: each participating holder must mark this label public to consent.
/// No real operand may carry it as a predicate id: [`route_operators`] and
/// [`assemble_leakage_envelope`](crate::assemble_leakage_envelope) refuse one that does.
pub const ANONYMOUS_GLOBAL_IRI: &str = "<global-iri>";

/// Refuse operands whose predicate id would be ambiguous in the leakage envelope: a
/// [`Operand::Predicate`] with an empty id, or any operand whose predicate id is the
/// [`ANONYMOUS_GLOBAL_IRI`] label. Either would let a predicate disclosure be recorded as, or
/// confused with, an anonymous global IRI.
pub(crate) fn validate_operands(
    operators: &[QueryOperator],
    phase: SeamPhase,
) -> Result<(), SeamError> {
    for op in operators {
        for operand in &op.operands {
            let reserved = operand.predicate() == ANONYMOUS_GLOBAL_IRI;
            let empty_predicate = matches!(operand, Operand::Predicate(p) if p.is_empty());
            if reserved || empty_predicate {
                return Err(SeamError::DescriptorMismatch {
                    phase,
                    source_id: String::new(),
                    detail:
                        "operand predicate id is empty or the reserved anonymous-global-IRI label",
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_fedplan::{Bgp, PredPartition, SourceDescriptor, Term, TriplePattern, Var};

    use crate::selection::select_private_sources;

    const MEMBER_OF: &str = "http://ex/memberOf";
    const SALARY: &str = "http://ex/salary";
    const FLAT_IRI: &str = "http://ex/flat";

    // ── Fixtures ────────────────────────────────────────────────────────────────────────

    // A source holding both predicates, identified by `id`.
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

    // The flatmate privacy descriptor: memberOf is public (global-IRI membership), salary private.
    fn flatmate_privacy(id: &str) -> SourcePrivacyDescriptor {
        SourcePrivacyDescriptor::builder(SourceId::new(id))
            .public_predicate(MEMBER_OF)
            .private_predicate(SALARY)
            .participates(true)
            .build()
    }

    // The three operators of the four-flatmates query, in pipeline order. Mirrors the hand-written
    // routing in sparq-mpc::pipeline (the differential target).
    fn four_flatmates_operators(threshold: u64) -> Vec<QueryOperator> {
        vec![
            QueryOperator::new(
                "membership-join (global-IRI key-on-key)",
                // class is unused while disclosed, but a join is an EqualityJoin if it were hidden.
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
                format!("salary-threshold (> {})", threshold),
                OperatorClass::Comparison,
                vec![Operand::Predicate(SALARY.to_string())],
            ),
        ]
    }

    // The membership join keyed on a global IRI bound by memberOf.
    fn membership_global_iri_join() -> QueryOperator {
        QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                predicate: MEMBER_OF.to_string(),
            }],
        )
    }

    // A real Phase-2 selection over a memberOf pattern in which every source in `ids` participates.
    fn membership_selection(
        ids: &[&str],
        privacy: &[SourcePrivacyDescriptor],
    ) -> SelectedPrivateSources {
        let sources: Vec<SourceDescriptor> = ids.iter().map(|id| source(id)).collect();
        let bgp = Bgp::new(vec![TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )]);
        let selected = select_private_sources(&bgp, &sources, privacy).unwrap();
        let got: Vec<&str> = selected
            .participating_sources()
            .into_iter()
            .map(|id| id.0.as_str())
            .collect();
        assert_eq!(got, ids);
        selected
    }

    // ── Tests ───────────────────────────────────────────────────────────────────────────

    /// The load-bearing differential: the produced routing reproduces the hand-written
    /// four-flatmates routing EXACTLY (operator labels + Disclosed/Hidden + OperatorClass).
    #[test]
    fn reproduces_the_handwritten_four_flatmates_routing() {
        let privacy = vec![
            flatmate_privacy("http://a/"),
            flatmate_privacy("http://b/"),
            flatmate_privacy("http://c/"),
            flatmate_privacy("http://d/"),
        ];
        let ops = four_flatmates_operators(100_000);
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();

        assert_eq!(out.routing.len(), 3);
        // 1. membership join → Disclosed, label verbatim.
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
        assert_eq!(
            out.routing[0].operator,
            "membership-join (global-IRI key-on-key)"
        );
        // 2. salary cumulative sum → Hidden(LinearAggregate).
        assert_eq!(
            out.routing[1].routing,
            Routing::Hidden(OperatorClass::LinearAggregate)
        );
        assert_eq!(out.routing[1].operator, "salary-cumulative-sum");
        // 3. salary threshold → Hidden(Comparison), threshold in the label.
        assert_eq!(
            out.routing[2].routing,
            Routing::Hidden(OperatorClass::Comparison)
        );
        assert_eq!(out.routing[2].operator, "salary-threshold (> 100000)");
    }

    /// Threaded through the REAL Phase-2 selection (not the empty default): the same routing is
    /// produced over the participating sources the adapter retained.
    #[test]
    fn routes_over_the_real_phase2_selection() {
        let sources = vec![source("http://a/"), source("http://b/")];
        let privacy = vec![flatmate_privacy("http://a/"), flatmate_privacy("http://b/")];
        let bgp = Bgp::new(vec![TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )]);
        let selected = select_private_sources(&bgp, &sources, &privacy).unwrap();
        assert!(!selected.is_empty());

        let ops = four_flatmates_operators(100_000);
        let out = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
        assert_eq!(
            out.routing[1].routing,
            Routing::Hidden(OperatorClass::LinearAggregate)
        );
    }

    /// Default-deny: a predicate NO source has opted in is hidden even though it is named in an
    /// operator's operands — there is no implicit disclosure.
    #[test]
    fn unmarked_predicate_is_hidden_default_deny() {
        let privacy = vec![SourcePrivacyDescriptor::deny_all(SourceId::new(
            "http://a/",
        ))];
        let ops = vec![QueryOperator::new(
            "join-on-unmarked",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(MEMBER_OF.to_string())],
        )];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(
            out.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// The most-private source wins: if even ONE contributing source leaves a predicate private,
    /// the operator is hidden, even when the others marked it public.
    #[test]
    fn one_private_source_forces_the_operator_hidden() {
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .public_predicate(MEMBER_OF)
                .build(),
            // B leaves memberOf private (default-deny).
            SourcePrivacyDescriptor::deny_all(SourceId::new("http://b/")),
        ];
        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(MEMBER_OF.to_string())],
        )];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(
            out.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// A predicate every contributing source marks public IS disclosed (the affirmative rule).
    #[test]
    fn all_public_predicate_is_disclosed() {
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .public_predicate(MEMBER_OF)
                .build(),
            SourcePrivacyDescriptor::builder(SourceId::new("http://b/"))
                .public_predicate(MEMBER_OF)
                .build(),
        ];
        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(MEMBER_OF.to_string())],
        )];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
    }

    /// The §5 cost-vs-privacy knob: under the STRICT policy a global IRI is HIDDEN when a
    /// contributing source has marked its predicate private — even though the default policy
    /// would disclose it. This is the "hide even a public term" route.
    #[test]
    fn strict_policy_hides_a_global_iri_a_source_marked_private() {
        // The source marks the membership predicate PRIVATE (it does not want to reveal which
        // flat), so under strict the global-IRI membership join is hidden.
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .private_predicate(MEMBER_OF)
            .build()];
        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                predicate: MEMBER_OF.to_string(),
            }],
        )];

        // Default policy: the global IRI is disclosed (cheap route).
        let def = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(def.routing[0].routing, Routing::Disclosed);

        // Strict policy: the same global IRI is hidden (honours the private mark).
        let strict = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Strict,
        )
        .unwrap();
        assert_eq!(
            strict.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// Strict policy with NO contributing source hides a global IRI: an empty contributing set
    /// is not consent, so the convention shortcut the default policy uses does not apply.
    #[test]
    fn strict_policy_hides_a_global_iri_when_no_source_contributes() {
        let ops = vec![membership_global_iri_join()];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &[],
            &ops,
            RoutingPolicy::Strict,
        )
        .unwrap();
        assert_eq!(
            out.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// Strict policy discloses a global IRI once every contributing source explicitly marks the
    /// bound predicate public — the valid, explicitly permitted case.
    #[test]
    fn strict_policy_discloses_a_global_iri_every_source_marks_public() {
        let privacy = vec![flatmate_privacy("http://a/"), flatmate_privacy("http://b/")];
        let selected = membership_selection(&["http://a/", "http://b/"], &privacy);
        let out = route_operators(
            &selected,
            &privacy,
            &[membership_global_iri_join()],
            RoutingPolicy::Strict,
        )
        .unwrap();
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
    }

    /// An anonymous global IRI (no bound predicate) has nothing a source can mark public, so the
    /// strict policy hides it even when every source has a descriptor.
    #[test]
    fn strict_policy_hides_an_anonymous_global_iri() {
        let privacy = vec![flatmate_privacy("http://a/")];
        let selected = membership_selection(&["http://a/"], &privacy);
        let ops = vec![QueryOperator::new(
            "anonymous-iri-join",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                predicate: String::new(),
            }],
        )];
        let strict = route_operators(&selected, &privacy, &ops, RoutingPolicy::Strict).unwrap();
        assert_eq!(
            strict.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// A descriptor that marks the empty predicate public does not make an anonymous global IRI
    /// disclosable under the strict policy: the empty predicate is never consent.
    #[test]
    fn strict_policy_ignores_a_public_mark_on_the_empty_predicate() {
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(MEMBER_OF)
            .public_predicate("")
            .build()];
        let selected = membership_selection(&["http://a/"], &privacy);
        let ops = vec![QueryOperator::new(
            "anonymous-iri-join",
            OperatorClass::EqualityJoin,
            vec![Operand::GlobalIri {
                predicate: String::new(),
            }],
        )];
        let out = route_operators(&selected, &privacy, &ops, RoutingPolicy::Strict).unwrap();
        assert_eq!(
            out.routing[0].routing,
            Routing::Hidden(OperatorClass::EqualityJoin)
        );
    }

    /// A selected participant with NO privacy descriptor is refused before any operator is
    /// classified, under both policies and for both operand kinds. Without this check the empty
    /// contributing set made a strict global IRI disclosable.
    #[test]
    fn missing_descriptor_for_a_participant_is_refused() {
        let privacy = vec![flatmate_privacy("http://a/")];
        let selected = membership_selection(&["http://a/"], &privacy);
        let operand_cases = [
            vec![membership_global_iri_join()],
            vec![QueryOperator::new(
                "membership-join",
                OperatorClass::EqualityJoin,
                vec![Operand::Predicate(MEMBER_OF.to_string())],
            )],
        ];
        for ops in &operand_cases {
            for policy in [RoutingPolicy::Default, RoutingPolicy::Strict] {
                let err = route_operators(&selected, &[], ops, policy).unwrap_err();
                assert_eq!(
                    err,
                    SeamError::DescriptorMismatch {
                        phase: SeamPhase::Routing,
                        source_id: "http://a/".to_string(),
                        detail: "no SourcePrivacyDescriptor for this participating source (a missing policy fails closed)",
                    }
                );
            }
        }
    }

    /// Partial omission: one participant's descriptor is missing while the remaining one marks the
    /// predicate public. The missing source must not be dropped from the gating set — the pass
    /// refuses and names it, for both operand kinds and both policies.
    #[test]
    fn partially_missing_descriptors_are_refused() {
        let a_public = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(MEMBER_OF)
            .build();
        let b_private = SourcePrivacyDescriptor::deny_all(SourceId::new("http://b/"));
        let selected =
            membership_selection(&["http://a/", "http://b/"], &[a_public.clone(), b_private]);
        let operand_cases = [
            vec![membership_global_iri_join()],
            vec![QueryOperator::new(
                "membership-join",
                OperatorClass::EqualityJoin,
                vec![Operand::Predicate(MEMBER_OF.to_string())],
            )],
        ];
        for ops in &operand_cases {
            for policy in [RoutingPolicy::Default, RoutingPolicy::Strict] {
                let err = route_operators(&selected, std::slice::from_ref(&a_public), ops, policy)
                    .unwrap_err();
                assert!(
                    matches!(&err, SeamError::DescriptorMismatch { phase: SeamPhase::Routing, source_id, .. } if source_id == "http://b/"),
                    "expected a refusal naming http://b/, got {err:?}"
                );
            }
        }
    }

    /// A descriptor for a source outside the participation set is ignored: it neither gates nor
    /// authorises anything. A non-participating deny-all source does not hide a predicate every
    /// participant marks public.
    #[test]
    fn descriptors_for_non_participants_are_ignored() {
        let a_public = SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(MEMBER_OF)
            .build();
        let selected = membership_selection(&["http://a/"], std::slice::from_ref(&a_public));
        let privacy = vec![
            a_public,
            SourcePrivacyDescriptor::deny_all(SourceId::new("http://c/")),
        ];
        let ops = vec![QueryOperator::new(
            "membership-join",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(MEMBER_OF.to_string())],
        )];
        let out = route_operators(&selected, &privacy, &ops, RoutingPolicy::Strict).unwrap();
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
    }

    /// `validate_operands` accepts ordinary and anonymous global-IRI operands and refuses the
    /// reserved label or an empty predicate, naming the phase it was called for.
    #[test]
    fn validate_operands_refuses_only_ambiguous_predicate_ids() {
        let ok = vec![QueryOperator::new(
            "ok",
            OperatorClass::EqualityJoin,
            vec![
                Operand::Predicate(MEMBER_OF.to_string()),
                Operand::GlobalIri {
                    predicate: String::new(),
                },
            ],
        )];
        assert_eq!(validate_operands(&ok, SeamPhase::Routing), Ok(()));
        let bad = vec![QueryOperator::new(
            "bad",
            OperatorClass::EqualityJoin,
            vec![Operand::Predicate(String::new())],
        )];
        assert!(matches!(
            validate_operands(&bad, SeamPhase::LeakageEnvelope),
            Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::LeakageEnvelope,
                ..
            })
        ));
    }

    /// An operand whose predicate id is empty or the reserved anonymous-global-IRI label is
    /// refused, so a predicate disclosure cannot be recorded as the anonymous-global-IRI label.
    #[test]
    fn reserved_or_empty_operand_predicate_is_refused() {
        let privacy = vec![SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
            .public_predicate(ANONYMOUS_GLOBAL_IRI)
            .public_predicate("")
            .build()];
        for operand in [
            Operand::Predicate(ANONYMOUS_GLOBAL_IRI.to_string()),
            Operand::Predicate(String::new()),
            Operand::GlobalIri {
                predicate: ANONYMOUS_GLOBAL_IRI.to_string(),
            },
        ] {
            let ops = vec![QueryOperator::new(
                "reserved",
                OperatorClass::EqualityJoin,
                vec![operand],
            )];
            let err = route_operators(
                &SelectedPrivateSources::default(),
                &privacy,
                &ops,
                RoutingPolicy::Default,
            )
            .unwrap_err();
            assert!(matches!(
                err,
                SeamError::DescriptorMismatch {
                    phase: SeamPhase::Routing,
                    ..
                }
            ));
        }
    }

    /// An operator with a MIX of operands is disclosed only when ALL are disclosable: a disclosed
    /// global IRI + a private salary predicate ⇒ hidden (the private operand wins).
    #[test]
    fn mixed_operands_hide_if_any_operand_is_private() {
        let privacy = vec![flatmate_privacy("http://a/")];
        let ops = vec![QueryOperator::new(
            "filter-membership-and-salary",
            OperatorClass::Comparison,
            vec![
                Operand::GlobalIri {
                    predicate: MEMBER_OF.to_string(),
                },
                Operand::Predicate(SALARY.to_string()),
            ],
        )];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(
            out.routing[0].routing,
            Routing::Hidden(OperatorClass::Comparison)
        );
    }

    /// An operator with NO operands is vacuously disclosed (it reads nothing private).
    #[test]
    fn operator_with_no_operands_is_disclosed() {
        let ops = vec![QueryOperator::new(
            "constant",
            OperatorClass::LinearAggregate,
            vec![],
        )];
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &[],
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        assert_eq!(out.routing[0].routing, Routing::Disclosed);
    }

    /// A duplicate privacy descriptor for one source id is refused (fail-closed), mirroring
    /// Phase 2.
    #[test]
    fn duplicate_descriptor_is_a_descriptor_mismatch() {
        let privacy = vec![
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .public_predicate(MEMBER_OF)
                .build(),
            SourcePrivacyDescriptor::builder(SourceId::new("http://a/"))
                .private_predicate(MEMBER_OF)
                .build(),
        ];
        let ops = four_flatmates_operators(100_000);
        let err = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::Routing,
                ..
            }
        ));
        assert!(format!("{}", err).contains("http://a/"));
    }

    /// Determinism: same inputs ⇒ identical routing, twice (the seam is deterministic).
    #[test]
    fn determinism_same_inputs_same_routing() {
        let privacy = vec![flatmate_privacy("http://a/"), flatmate_privacy("http://b/")];
        let ops = four_flatmates_operators(100_000);
        let a = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        let b = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        // OperatorRouting is not PartialEq; compare the observable fields.
        assert_eq!(a.routing.len(), b.routing.len());
        for (x, y) in a.routing.iter().zip(&b.routing) {
            assert_eq!(x.operator, y.operator);
            assert_eq!(x.routing, y.routing);
        }
    }

    /// Order preservation: the emitted routing is in the SAME order as the input operators.
    #[test]
    fn preserves_operator_order() {
        let privacy = vec![flatmate_privacy("http://a/")];
        let ops = four_flatmates_operators(50_000);
        let out = route_operators(
            &SelectedPrivateSources::default(),
            &privacy,
            &ops,
            RoutingPolicy::Default,
        )
        .unwrap();
        let got: Vec<&str> = out.routing.iter().map(|r| r.operator.as_str()).collect();
        assert_eq!(
            got,
            vec![
                "membership-join (global-IRI key-on-key)",
                "salary-cumulative-sum",
                "salary-threshold (> 50000)",
            ]
        );
    }
}

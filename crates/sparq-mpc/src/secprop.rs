//! # `secprop` — the per-protocol security-properties annotation graph (Phase 6)
//!
//! The **machine-readable form** of this crate's per-operator security posture. A
//! **static annotation graph** (`ontologies/secprop-methods.ttl`) keyed on `mpc:`
//! protocol IRIs, expressed with the `secx:` extension vocabulary (design record
//! `research/security-properties-ontology-design.md` §5c). Each protocol's
//! `secx:hasProperty` is a reified [`PropertyAssertion`]: a `(property, level,
//! assurance, audit-status, assumptions)` claim.
//!
//! ## Why §5c wants this
//!
//! "sparq-mpc is semi-honest only" is stated in prose in the README, `PLAN.md` and a
//! dozen module headers. Prose does not mechanically exclude anything. Recording the
//! **assumptions** a protocol's guarantee rests on — `secx:HonestMajority` and, where
//! it applies, `secx:SemiHonest` — lets a caller *compute* the exclusion instead of
//! reading it: see [`excluded_by_requiring_malicious_security`] and
//! [`excluded_by_requiring_dishonest_majority`].
//!
//! ## What this records — and what it is NOT
//!
//! It **records** a (protocol, property) → (level, assurance, audit-status,
//! assumptions) claim and its **epistemic basis**; it is **NOT** a proof of any
//! property. sparq-mpc is research-grade and **externally UNAUDITED** (`sq-qhy4`, P0),
//! and the collaborative ZK proof layer ([`crate::proof`]) is an honest
//! `NotYetImplemented` stub.
//!
//! ## Not a parallel source of truth
//!
//! [`crate::backend::SecurityDescriptor`] already models the posture in the type
//! system (adversary × output-guarantee × corruption-threshold). This module does not
//! restate it — it **pins** to it: [`descriptor_drift_violations`] checks the three
//! [`OperatorClass`]-keyed subjects against what
//! [`ShamirBackend::operator_descriptor`](crate::shamir::ShamirBackend::operator_descriptor)
//! reports for the same operator. Harden an operator in code without updating the
//! Turtle and the guard goes red.
//!
//! ## The four guards
//!
//! 1. **No over-claim on assurance** ([`assurance_overclaim_violations`]): no
//!    **positive** property may carry [`Assurance::Proven`] while `sq-qhy4` is open.
//!    Only settled NEGATIVE facts (here: `secx:NotZK`) may be `Proven`.
//! 2. **Every claim names its assumptions** ([`assumption_completeness_violations`]):
//!    every assertion declares at least one `secx:assumption`, and every protocol
//!    rests on `secx:HonestMajority` — nothing here is dishonest-majority secure.
//! 3. **Completeness** ([`completeness_violations`]): every protocol in
//!    [`ANNOTATED_METHODS`] has an annotation block.
//! 4. **Descriptor drift** ([`descriptor_drift_violations`]): the Turtle's
//!    semi-honest / honest-majority assumptions agree with the code's own descriptor.
//!
//! All four are exposed as public functions (so a caller can re-run them) and asserted
//! by this crate's tests.
//!
//! ## Opt-in by construction
//!
//! Behind the **default-OFF `secprop-annotations`** cargo feature, so it adds nothing
//! to the lean default build. The `secx:` IRIs come from the ZERO-dependency
//! [`sparq_secprop_vocab`] leaf crate, so the opt-in build grows by a `const &str`
//! table and the `oxttl` Turtle parser, and nothing else.
//!
//! [OPUS-5] sq-dz10l (Phase 6; epic sq-0dksu; design record
//! `research/security-properties-ontology-design.md` §5c). 🤖 SPARQ agent —
//! security-properties ontology.

use crate::backend::{AdversaryModel, OperatorClass};
use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use std::collections::{BTreeMap, BTreeSet};

/// The canonical machine-readable per-protocol annotation graph (the source of truth
/// this module's accessors and guards parse).
const METHODS_TTL: &str = include_str!("../ontologies/secprop-methods.ttl");

// ── the `secx:` IRIs this module references ──────────────────────────────────
// Imported from the ZERO-dependency `sparq-secprop-vocab` leaf, which owns the
// constants, the canonical `secprop-ext.ttl` and the single drift test that pins the
// two together (sq-3705). There is no local copy to drift.
use sparq_secprop_vocab::{
    SECX_ASSUMPTION, SECX_ASSURANCE, SECX_AUDIT_STATUS, SECX_CLAIMED, SECX_CONJECTURED,
    SECX_HAS_PROPERTY, SECX_HONEST_MAJORITY, SECX_LEVEL, SECX_NOT_ZK, SECX_PROPERTY, SECX_PROVEN,
    SECX_SEMI_HONEST,
};

/// The `sec-prop:` namespace base the `secx:` IRIs share. Re-exported from
/// [`sparq_secprop_vocab`] — this crate declares no copy of its own.
pub use sparq_secprop_vocab::SEC_PROP_NS;

/// `secx:ExternalSignOffPending` — the live audit status of every positive sparq
/// security property while `sq-qhy4` is open (re-exported so a caller can recognise
/// the unaudited basis on which a `Claimed` property is admitted).
pub use sparq_secprop_vocab::SECX_EXTERNAL_SIGN_OFF_PENDING;

/// The `mpc:` protocol namespace the annotation subjects are minted under.
pub const MPC_NS: &str = "https://sparq.dev/ns/mpc#";

/// `mpc:linear-aggregate-v1` — the degree-`t` linear aggregate
/// ([`OperatorClass::LinearAggregate`]).
pub const MPC_LINEAR_AGGREGATE_V1: &str = "https://sparq.dev/ns/mpc#linear-aggregate-v1";
/// `mpc:equality-join-v1` — the hidden-value join's degree-`2t` equality open
/// ([`OperatorClass::EqualityJoin`]).
pub const MPC_EQUALITY_JOIN_V1: &str = "https://sparq.dev/ns/mpc#equality-join-v1";
/// `mpc:comparison-v1` — the semi-honest secure comparison ([`OperatorClass::Comparison`],
/// [`crate::compare`]).
pub const MPC_COMPARISON_V1: &str = "https://sparq.dev/ns/mpc#comparison-v1";
/// `mpc:auth-comparison-v1` — the IT-MAC authenticated comparison
/// ([`crate::auth_compare`]). The crate's strongest construction, and still annotated
/// semi-honest: [`crate::shamir::MacSession::auth_mul`] ADOPTS a value tamper on its
/// second operand, and every gate of the chain is an `auth_mul`, so an actively
/// deviating party has a known route to a wrong verdict without an abort.
pub const MPC_AUTH_COMPARISON_V1: &str = "https://sparq.dev/ns/mpc#auth-comparison-v1";
/// `mpc:tamper-evident-disclose-v1` — the EXPERIMENTAL tamper-evident threshold
/// disclosure ([`crate::auth_disclose`]). MAC layer notwithstanding, it is annotated
/// semi-honest: it does not deliver malicious security.
pub const MPC_TAMPER_EVIDENT_DISCLOSE_V1: &str =
    "https://sparq.dev/ns/mpc#tamper-evident-disclose-v1";

/// Every protocol that MUST carry an annotation block (guard 3,
/// [`completeness_violations`]).
///
/// The crypto-free paths ([`crate::join::DisclosedKeyJoin`], [`crate::bounded_path`])
/// are deliberately absent: they run no MPC and open every operand, so they have no
/// adversary model to annotate and recording one would invent a guarantee they do not
/// make.
pub const ANNOTATED_METHODS: &[&str] = &[
    MPC_LINEAR_AGGREGATE_V1,
    MPC_EQUALITY_JOIN_V1,
    MPC_COMPARISON_V1,
    MPC_AUTH_COMPARISON_V1,
    MPC_TAMPER_EVIDENT_DISCLOSE_V1,
];

/// The protocol IRI each [`OperatorClass`] is annotated under — the mapping
/// [`descriptor_drift_violations`] walks to compare the Turtle against the code.
pub const OPERATOR_CLASS_METHODS: &[(OperatorClass, &str)] = &[
    (OperatorClass::LinearAggregate, MPC_LINEAR_AGGREGATE_V1),
    (OperatorClass::EqualityJoin, MPC_EQUALITY_JOIN_V1),
    (OperatorClass::Comparison, MPC_COMPARISON_V1),
];

// ── typed model ──────────────────────────────────────────────────────────────

/// The epistemic-basis axis of a [`PropertyAssertion`] — `Proven ⊐ Claimed ⊐
/// Conjectured`. The honesty mechanism: a **positive** sparq-mpc property may only ever
/// be [`Self::Claimed`] or weaker while `sq-qhy4` is open; [`Self::Proven`] is reserved
/// for settled NEGATIVE facts about the construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Assurance {
    /// Backed by a formal proof or an external audit (including a provable NEGATIVE
    /// result). Not applicable to a positive sparq-mpc property while `sq-qhy4` is open.
    Proven,
    /// Asserted, not independently verified. The default for a sparq-mpc property.
    Claimed,
    /// Believed plausible but not established to the `Claimed` bar.
    Conjectured,
}

impl Assurance {
    fn from_iri(iri: &str) -> Option<Self> {
        match iri {
            SECX_PROVEN => Some(Assurance::Proven),
            SECX_CLAIMED => Some(Assurance::Claimed),
            SECX_CONJECTURED => Some(Assurance::Conjectured),
            _ => None,
        }
    }
}

/// A reified `(property, level, assurance, audit-status, assumptions)` claim about one
/// protocol IRI — a parsed `secx:PropertyAssertion`.
///
/// Unlike the ZK estate's analogue, `assumptions` is **multi-valued**: an MPC guarantee
/// rests on a *conjunction* (`secx:HonestMajority` AND `secx:SemiHonest`), and
/// collapsing that to one IRI would silently drop half the precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyAssertion {
    /// The dimension the assertion is about (a `secx:` property IRI, e.g.
    /// `secx:Soundness`).
    pub property: String,
    /// The level/class held within the dimension (e.g. `secx:Sound`). `None` if the
    /// assertion only fixes a parameter.
    pub level: Option<String>,
    /// The epistemic basis of the claim.
    pub assurance: Assurance,
    /// The independent-review state IRI (e.g. `secx:ExternalSignOffPending`), if set.
    pub audit_status: Option<String>,
    /// The assumptions the claim rests on, as `secx:` assumption IRIs, in a stable
    /// order. Never empty in the shipped graph (guard 2).
    pub assumptions: BTreeSet<String>,
}

impl PropertyAssertion {
    /// Whether this is a **positive** assertion subject to the no-`Proven` guard — i.e.
    /// NOT one of the settled-negative levels. A negative level may legitimately be
    /// `Proven`: `secx:NotZK` says the protocol provides NO zero-knowledge property,
    /// which is a conservative statement that cannot over-claim.
    fn is_positive(&self) -> bool {
        !matches!(self.level.as_deref(), Some(SECX_NOT_ZK))
    }

    /// Whether this claim rests on `assumption` (a `secx:` assumption IRI).
    pub fn rests_on(&self, assumption: &str) -> bool {
        self.assumptions.contains(assumption)
    }
}

/// All [`PropertyAssertion`]s attached to one protocol IRI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodAnnotations {
    /// The annotated protocol IRI (an `mpc:` IRI).
    pub method: String,
    /// Its property assertions, in a stable (property, level) order.
    pub assertions: Vec<PropertyAssertion>,
}

impl MethodAnnotations {
    /// Whether ANY of this protocol's claims rests on `assumption`.
    ///
    /// This is the §5c exclusion primitive: a protocol whose guarantees rest on
    /// `secx:SemiHonest` cannot satisfy a preference that requires security against an
    /// actively-deviating party.
    pub fn rests_on(&self, assumption: &str) -> bool {
        self.assertions.iter().any(|a| a.rests_on(assumption))
    }

    /// The union of every assumption this protocol's claims rest on.
    pub fn assumptions(&self) -> BTreeSet<&str> {
        self.assertions
            .iter()
            .flat_map(|a| a.assumptions.iter().map(String::as_str))
            .collect()
    }

    /// Whether this protocol **admits** a constraint requiring `property` (optionally
    /// at `level`). Fail-closed: an unknown property / no matching assertion =>
    /// `false`.
    pub fn admits_property(&self, property: &str, level: Option<&str>) -> bool {
        self.assertions.iter().any(|a| {
            a.property == property
                && match level {
                    None => true,
                    Some(l) => a.level.as_deref() == Some(l),
                }
        })
    }
}

// ── the §5c exclusion queries ────────────────────────────────────────────────

/// The protocols a preference **requiring malicious security** excludes — those whose
/// guarantees rest on `secx:SemiHonest`, i.e. hold only against a PASSIVE adversary.
///
/// This is §5c's headline: the honest encoding of "semi-honest only" in the property
/// data. Today it returns **every** annotated protocol — including
/// [`MPC_AUTH_COMPARISON_V1`], whose IT-MAC chain is the crate's strongest construction
/// but still inherits `auth_mul`'s adopted second-operand tamper. The annotation fails
/// closed: a protocol leaves this set only once the active-deviation hole is closed and
/// an end-to-end adversarial witness shows the real pipeline aborting.
pub fn excluded_by_requiring_malicious_security(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<String> {
    annotations
        .values()
        .filter(|m| m.rests_on(SECX_SEMI_HONEST))
        .map(|m| m.method.clone())
        .collect()
}

/// The protocols a preference **requiring dishonest-majority security** excludes —
/// those whose guarantees rest on `secx:HonestMajority`.
///
/// Today this is ALL of them: nothing in this crate is dishonest-majority secure (the
/// SPDZ/MASCOT regime is a different construction and is not built). Guard 2
/// ([`assumption_completeness_violations`]) is what keeps that true by construction.
pub fn excluded_by_requiring_dishonest_majority(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<String> {
    annotations
        .values()
        .filter(|m| m.rests_on(SECX_HONEST_MAJORITY))
        .map(|m| m.method.clone())
        .collect()
}

// ── guard reporting ──────────────────────────────────────────────────────────

/// An over-claim guard violation — what the guards report (a parse-stable,
/// human-readable record, never a panic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The protocol IRI the violation is on.
    pub method: String,
    /// The machine-readable reason kind.
    pub kind: ViolationKind,
    /// A one-line human-readable explanation.
    pub detail: String,
}

/// The kind of an over-claim [`Violation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// A positive property carries `secx:Proven` while `sq-qhy4` is open (guard 1).
    ProvenPositiveWhileAuditOpen,
    /// An assertion declares no assumption, or a protocol does not rest on
    /// `secx:HonestMajority` (guard 2).
    UnstatedAssumption,
    /// A protocol in [`ANNOTATED_METHODS`] has no annotation block (guard 3).
    MissingAnnotation,
    /// The Turtle's assumptions disagree with the code's own
    /// [`crate::backend::SecurityDescriptor`] for the same operator (guard 4).
    DescriptorDrift,
}

// ── parsing ──────────────────────────────────────────────────────────────────

/// Parse the static annotation graph into one [`MethodAnnotations`] per protocol IRI,
/// keyed by IRI in a stable order. Pure over the bundled Turtle — no I/O, never panics
/// on a well-formed bundled file (the file is a compiled-in constant and a test asserts
/// it parses).
///
/// Implementation note: the Turtle uses `secx:hasProperty [ … ]` blank-node objects, so
/// we first index every triple by blank-node subject, then resolve each
/// `(method, hasProperty, _:b)` edge to the pairs on `_:b`.
pub fn parse_annotations() -> BTreeMap<String, MethodAnnotations> {
    // blank-node id -> its (predicate IRI, object) pairs.
    let mut bnode_pairs: BTreeMap<String, Vec<(String, Term)>> = BTreeMap::new();
    // protocol IRI -> the blank-node ids of its assertion nodes.
    let mut method_assertion_bnodes: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for result in TurtleParser::new().for_reader(METHODS_TTL.as_bytes()) {
        let t = result.expect("bundled secprop-methods.ttl must be valid Turtle");
        let pred = t.predicate.into_string();

        match &t.subject {
            NamedOrBlankNode::NamedNode(s) => {
                if pred == SECX_HAS_PROPERTY {
                    if let Term::BlankNode(b) = &t.object {
                        method_assertion_bnodes
                            .entry(s.as_str().to_owned())
                            .or_default()
                            .push(b.as_str().to_owned());
                    }
                }
            }
            NamedOrBlankNode::BlankNode(b) => {
                bnode_pairs
                    .entry(b.as_str().to_owned())
                    .or_default()
                    .push((pred, t.object.clone()));
            }
        }
    }

    let mut out: BTreeMap<String, MethodAnnotations> = BTreeMap::new();
    for (method, bnodes) in method_assertion_bnodes {
        let mut assertions = Vec::new();
        for b in bnodes {
            if let Some(pairs) = bnode_pairs.get(&b) {
                if let Some(a) = assertion_from_pairs(pairs) {
                    assertions.push(a);
                }
            }
        }
        // Stable order: by (property, level) so output is deterministic.
        assertions.sort_by(|x, y| {
            (x.property.as_str(), x.level.as_deref())
                .cmp(&(y.property.as_str(), y.level.as_deref()))
        });
        out.insert(method.clone(), MethodAnnotations { method, assertions });
    }
    out
}

/// Build a [`PropertyAssertion`] from the predicate-object pairs of one blank node.
/// Returns `None` if there is no `secx:property` (not an assertion node) or the
/// assurance is missing/unknown (fail-closed: an annotation must state its basis).
fn assertion_from_pairs(pairs: &[(String, Term)]) -> Option<PropertyAssertion> {
    let iri_obj = |p: &str| -> Option<String> {
        pairs.iter().find_map(|(pred, obj)| {
            if pred == p {
                if let Term::NamedNode(n) = obj {
                    return Some(n.as_str().to_owned());
                }
            }
            None
        })
    };

    let property = iri_obj(SECX_PROPERTY)?;
    let assurance = Assurance::from_iri(&iri_obj(SECX_ASSURANCE)?)?;

    // `secx:assumption` is multi-valued (a conjunction of preconditions), so collect
    // EVERY object rather than the first — dropping one would silently weaken the
    // recorded precondition.
    let assumptions: BTreeSet<String> = pairs
        .iter()
        .filter(|(pred, _)| pred == SECX_ASSUMPTION)
        .filter_map(|(_, obj)| match obj {
            Term::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .collect();

    Some(PropertyAssertion {
        property,
        level: iri_obj(SECX_LEVEL),
        assurance,
        audit_status: iri_obj(SECX_AUDIT_STATUS),
        assumptions,
    })
}

// ── the four guards ──────────────────────────────────────────────────────────

/// **Guard 1 — no over-claim on assurance.** No **positive** property may carry
/// [`Assurance::Proven`] while the external accredited-cryptographer sign-off
/// (`sq-qhy4`) is open. Only settled NEGATIVE levels (`secx:NotZK`) may be `Proven`.
pub fn assurance_overclaim_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in annotations.values() {
        for a in &m.assertions {
            if a.assurance == Assurance::Proven && a.is_positive() {
                out.push(Violation {
                    method: m.method.clone(),
                    kind: ViolationKind::ProvenPositiveWhileAuditOpen,
                    detail: format!(
                        "positive property {} (level {:?}) is secx:Proven while sq-qhy4 is open; \
                         positive sparq-mpc properties must be secx:Claimed or weaker",
                        a.property, a.level
                    ),
                });
            }
        }
    }
    out
}

/// **Guard 2 — every claim names its assumptions.** Every assertion must declare at
/// least one `secx:assumption`, and every protocol must rest on
/// `secx:HonestMajority`: nothing in this crate is dishonest-majority secure, and an
/// unstated assumption is exactly the over-claim §5c exists to prevent.
pub fn assumption_completeness_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    for m in annotations.values() {
        for a in &m.assertions {
            if a.assumptions.is_empty() {
                out.push(Violation {
                    method: m.method.clone(),
                    kind: ViolationKind::UnstatedAssumption,
                    detail: format!(
                        "assertion on {} states no secx:assumption; every MPC claim must name \
                         the adversary model / corruption threshold it rests on",
                        a.property
                    ),
                });
            }
        }
        if !m.rests_on(SECX_HONEST_MAJORITY) {
            out.push(Violation {
                method: m.method.clone(),
                kind: ViolationKind::UnstatedAssumption,
                detail: "protocol does not rest on secx:HonestMajority; no sparq-mpc protocol is \
                         dishonest-majority secure, so omitting it would over-claim"
                    .to_owned(),
            });
        }
    }
    out
}

/// **Guard 3 — completeness.** Every protocol in [`ANNOTATED_METHODS`] has an
/// annotation block.
pub fn completeness_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<Violation> {
    ANNOTATED_METHODS
        .iter()
        .filter(|m| !annotations.contains_key(**m))
        .map(|m| Violation {
            method: (*m).to_owned(),
            kind: ViolationKind::MissingAnnotation,
            detail: "protocol has no secx:hasProperty block in ontologies/secprop-methods.ttl"
                .to_owned(),
        })
        .collect()
}

/// **Guard 4 — descriptor drift.** The Turtle's `secx:SemiHonest` /
/// `secx:HonestMajority` assumptions must agree, for each [`OperatorClass`]-keyed
/// subject, with what the CODE reports via
/// [`ShamirBackend::operator_descriptor`](crate::shamir::ShamirBackend::operator_descriptor)
/// at `(n, t)`.
///
/// This is what stops the annotation graph from becoming a stale parallel truth: if an
/// operator is hardened to [`AdversaryModel::Malicious`] in code, the `secx:SemiHonest`
/// row here becomes a lie and this guard reports it.
pub fn descriptor_drift_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
    backend: &crate::shamir::ShamirBackend,
) -> Vec<Violation> {
    let mut out = Vec::new();
    for (operator, iri) in OPERATOR_CLASS_METHODS {
        let Some(m) = annotations.get(*iri) else {
            // Absence is guard 3's report, not this one's.
            continue;
        };
        let descriptor = backend.operator_descriptor(*operator);

        let code_semi_honest = descriptor.adversary == AdversaryModel::SemiHonest;
        let ttl_semi_honest = m.rests_on(SECX_SEMI_HONEST);
        if code_semi_honest != ttl_semi_honest {
            out.push(Violation {
                method: m.method.clone(),
                kind: ViolationKind::DescriptorDrift,
                detail: format!(
                    "code reports adversary {:?} for {:?} (semi-honest = {code_semi_honest}) but \
                     the Turtle {} secx:SemiHonest",
                    descriptor.adversary,
                    operator,
                    if ttl_semi_honest { "asserts" } else { "omits" }
                ),
            });
        }

        let code_honest_majority = descriptor.threshold.is_honest_majority();
        let ttl_honest_majority = m.rests_on(SECX_HONEST_MAJORITY);
        if code_honest_majority != ttl_honest_majority {
            out.push(Violation {
                method: m.method.clone(),
                kind: ViolationKind::DescriptorDrift,
                detail: format!(
                    "code reports threshold {:?} for {:?} (honest-majority = \
                     {code_honest_majority}) but the Turtle {} secx:HonestMajority",
                    descriptor.threshold,
                    operator,
                    if ttl_honest_majority {
                        "asserts"
                    } else {
                        "omits"
                    }
                ),
            });
        }
    }
    out
}

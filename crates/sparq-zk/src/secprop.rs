//! # `secprop` — the per-method security-properties annotation graph (Phase 3)
//!
//! The **machine-readable form** of the per-method security-posture table in
//! `research/zk-configurable-commitment-security.md` §4. A **static annotation
//! graph** (`ontologies/secprop-methods.ttl`) keyed on the `zk:scheme` (commitment
//! method) and `zk:cryptosuite` (signature suite) IRIs that [`crate::registry`]
//! records per credential, expressed with the `secx:` extension vocabulary
//! (`sec-prop:` extension, design record
//! `research/security-properties-ontology-design.md` §5a). Each method's
//! `secx:hasProperty` is a reified [`PropertyAssertion`]: a `(property, level,
//! assurance, audit-status, assumption, scope)` claim.
//!
//! ## What this records — and what it is NOT
//!
//! It **records** a (method, property) → (level, assurance, audit-status, assumption,
//! scope) claim and its **epistemic basis**; it is **NOT** a proof of any property.
//! sparq's whole ZK estate is research-grade and **externally UNAUDITED** (`sq-qhy4`,
//! P0; CR-G1 of `compliance/cryptoreview/gap-register.md`).
//!
//! ## The three over-claim guards (design §5a)
//!
//! 1. **No over-claim on assurance** ([`audit_overclaim_violations`]): no method may
//!    carry [`Assurance::Proven`] on a **positive** privacy/soundness property while
//!    `sq-qhy4` is open. Only **negative** facts about the construction
//!    (`PQForgeable`, `Replayable`) may be `Proven` — the privacy-claims gate encoded
//!    in the data.
//! 2. **Source-layer non-transfer** ([`source_layer_transfer_violations`] +
//!    [`MethodAnnotations::admits_query_proof_property`]): a property scoped
//!    [`Scope::SourceLayerOnly`] must **never** satisfy a *query-proof* constraint —
//!    `zk:sourceCryptosuite` is provenance, not an in-proof property (design §5.3).
//! 3. **Completeness** ([`completeness_violations`]): every **production-selectable**
//!    `zk:scheme` ([`crate::commit::CommitmentMethod::is_production_selectable`]) has
//!    an annotation block — the analogue of the "every circuit has a gate-count
//!    baseline" gate.
//!
//! All three are asserted by the module's tests and exposed as public functions so a
//! caller (e.g. the trust-graph admission gate, Phase 5) can re-run them.
//!
//! ## Opt-in by construction
//!
//! Behind the **default-OFF `secprop-annotations`** cargo feature, so it adds nothing
//! to the lean default build. The `secx:` IRIs it references come from the
//! ZERO-dependency [`sparq_secprop_vocab`] leaf crate, so the opt-in build grows by a
//! `const &str` table and nothing else.
//!
//! ## The cycle this used to work around (issue #3705)
//!
//! `sparq-trust` is the declared owner of the `secx:` vocabulary but it **depends on**
//! `sparq-zk`, so this crate could not import from it — a cycle. The `secx:` IRIs were
//! therefore redeclared here and pinned to `sparq-trust`'s canonical
//! `secprop-ext.ttl` by a drift test that `include_str!`-ed `../../sparq-trust/…`: not a
//! crate edge, but a compile-time read of another package's files, which also broke
//! `cargo package` file inclusion for this crate. Both the copies and that read are
//! gone: the constants, the Turtle and the single drift test now live once in
//! `sparq-secprop-vocab`, a leaf below `sparq-trust`, `sparq-policy` and `sparq-zk`
//! alike, so importing them creates no cycle.
//!
//! [OPUS-4.8] sq-bevd3 (Phase 3; epic sq-0dksu; design record
//! `research/security-properties-ontology-design.md` §5a); de-duplicated onto
//! `sparq-secprop-vocab` by [OPUS-5] sq-3705. 🤖 SPARQ agent —
//! security-properties ontology. Flag for re-review when Fable returns.

use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use std::collections::BTreeMap;

/// The canonical machine-readable per-method annotation graph (the source of truth
/// this module's accessors and guards parse).
const METHODS_TTL: &str = include_str!("../ontologies/secprop-methods.ttl");

// ── the `secx:` IRIs this module references ──────────────────────────────────
// Imported from the ZERO-dependency `sparq-secprop-vocab` leaf, which owns the
// constants, the canonical `secprop-ext.ttl` and the single drift test that pins the
// two together (sq-3705). There is no local copy left to drift.
use sparq_secprop_vocab::{
    SECX_ASSUMPTION, SECX_ASSURANCE, SECX_AUDIT_STATUS, SECX_CLAIMED, SECX_CONJECTURED,
    SECX_HAS_PROPERTY, SECX_LEVEL, SECX_PQ_FORGEABLE, SECX_PROPERTY, SECX_PROVEN,
    SECX_QUERY_PROOF_LAYER, SECX_REPLAYABLE, SECX_SCHEME_REVEALED, SECX_SCOPE,
    SECX_SOURCE_LAYER_ONLY,
};

/// The `sec-prop:` namespace base the `secx:` IRIs share (the vendored ZKP-SPARQL
/// permanent identifier this estate's annotations key on). Re-exported from
/// [`sparq_secprop_vocab`] — this crate no longer declares its own copy (sq-3705).
pub use sparq_secprop_vocab::SEC_PROP_NS;

/// `secx:ExternalSignOffPending` — the live audit status of every positive sparq ZK
/// property while `sq-qhy4` is open (re-exported so a caller can recognise the
/// unaudited basis on which a `Claimed` property is admitted).
pub use sparq_secprop_vocab::SECX_EXTERNAL_SIGN_OFF_PENDING;

// ── typed model ──────────────────────────────────────────────────────────────

/// The epistemic-basis axis of a [`PropertyAssertion`] — `Proven ⊐ Claimed ⊐
/// Conjectured`. The honesty mechanism: a positive sparq ZK property may only ever be
/// [`Self::Claimed`] (the default, #1001 Option A) or weaker while `sq-qhy4` is open;
/// [`Self::Proven`] is reserved for settled NEGATIVE facts about the construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Assurance {
    /// Backed by a formal proof or an external audit (including a provable NEGATIVE
    /// result). Not applicable to a sparq positive property while `sq-qhy4` is open.
    Proven,
    /// Asserted, not independently verified. The default for a sparq ZK property.
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

/// The layer an assertion is realised at (design §5a rule 2). Omitting `secx:scope`
/// in the Turtle means [`Self::QueryProofLayer`] — the conservative default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Re-verifiable in the sparq query proof (the default).
    QueryProofLayer,
    /// A property of the SOURCE credential's cryptosuite that does NOT transfer to
    /// the query proof. Must never satisfy a query-proof constraint.
    SourceLayerOnly,
}

impl Scope {
    fn from_iri(iri: &str) -> Option<Self> {
        match iri {
            SECX_QUERY_PROOF_LAYER => Some(Scope::QueryProofLayer),
            SECX_SOURCE_LAYER_ONLY => Some(Scope::SourceLayerOnly),
            _ => None,
        }
    }
}

/// A reified `(property, level, assurance, audit-status, assumption, scope)` claim
/// about one method IRI — a parsed `secx:PropertyAssertion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyAssertion {
    /// The dimension the assertion is about (a `secx:` property IRI, e.g.
    /// `secx:Soundness`).
    pub property: String,
    /// The level/class held within the dimension (a `secx:` level IRI, e.g.
    /// `secx:KnowledgeSound`). `None` if the assertion only fixes a parameter.
    pub level: Option<String>,
    /// The epistemic basis of the claim.
    pub assurance: Assurance,
    /// The independent-review state IRI (e.g. `secx:ExternalSignOffPending`), if set.
    pub audit_status: Option<String>,
    /// The assumption the claim rests on (a `secx:` assumption IRI), if any.
    pub assumption: Option<String>,
    /// The layer the property is realised at (defaults to [`Scope::QueryProofLayer`]).
    pub scope: Scope,
}

impl PropertyAssertion {
    /// Whether this is a **positive** assertion subject to the no-`Proven` guard — i.e.
    /// NOT one of the settled-negative levels (`PQForgeable`, `Replayable`,
    /// `SchemeRevealed`). A negative level may legitimately be `Proven`.
    fn is_positive(&self) -> bool {
        !matches!(
            self.level.as_deref(),
            Some(SECX_PQ_FORGEABLE) | Some(SECX_REPLAYABLE) | Some(SECX_SCHEME_REVEALED)
        )
    }
}

/// All [`PropertyAssertion`]s attached to one method IRI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodAnnotations {
    /// The annotated method IRI (a `zk:scheme` / `zk:cryptosuite` IRI).
    pub method: String,
    /// Its property assertions, in a stable (property, level) order.
    pub assertions: Vec<PropertyAssertion>,
}

impl MethodAnnotations {
    /// Whether this method **admits** a query-proof constraint requiring `property`
    /// (optionally at `level`). Source-layer-only assertions are **excluded** — the
    /// non-transfer rule (§5a rule 2): a property of a source credential's cryptosuite
    /// cannot satisfy a property the sparq query proof must itself establish.
    /// Fail-closed: an unknown property / no matching query-proof-layer assertion =>
    /// `false`.
    pub fn admits_query_proof_property(&self, property: &str, level: Option<&str>) -> bool {
        self.assertions.iter().any(|a| {
            a.scope == Scope::QueryProofLayer
                && a.property == property
                && match level {
                    None => true,
                    Some(l) => a.level.as_deref() == Some(l),
                }
        })
    }
}

/// An over-claim guard violation — what the guards report (a parse-stable,
/// human-readable record, never a panic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The method IRI the violation is on.
    pub method: String,
    /// The machine-readable reason kind.
    pub kind: ViolationKind,
    /// A one-line human-readable explanation.
    pub detail: String,
}

/// The kind of an over-claim [`Violation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// A positive property carries `secx:Proven` while `sq-qhy4` is open (§5a rule 1).
    ProvenPositiveWhileAuditOpen,
    /// A production-selectable `zk:scheme` has no annotation block (§5a rule 3).
    MissingAnnotation,
    /// A source-layer-only property was used to satisfy a query-proof constraint
    /// (§5a rule 2). Reported by [`source_layer_transfer_violations`].
    SourceLayerTransfer,
}

// ── parsing ──────────────────────────────────────────────────────────────────

/// Parse the static annotation graph into one [`MethodAnnotations`] per method IRI,
/// keyed by IRI in a stable order. Pure over the bundled Turtle — no I/O, never
/// panics on a well-formed bundled file (the file is a compiled-in constant and a
/// test asserts it parses).
///
/// Implementation note: the Turtle uses `secx:hasProperty [ … ]` blank-node objects,
/// so we first index every triple by blank-node subject, then resolve each
/// `(method, hasProperty, _:b)` edge to the pairs on `_:b`.
pub fn parse_annotations() -> BTreeMap<String, MethodAnnotations> {
    // blank-node id -> its (predicate IRI, object) pairs.
    let mut bnode_pairs: BTreeMap<String, Vec<(String, Term)>> = BTreeMap::new();
    // method IRI -> the blank-node ids of its assertion nodes.
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
        out.insert(
            method.clone(),
            MethodAnnotations { method, assertions },
        );
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
    let scope = iri_obj(SECX_SCOPE)
        .and_then(|s| Scope::from_iri(&s))
        .unwrap_or(Scope::QueryProofLayer);

    Some(PropertyAssertion {
        property,
        level: iri_obj(SECX_LEVEL),
        assurance,
        audit_status: iri_obj(SECX_AUDIT_STATUS),
        assumption: iri_obj(SECX_ASSUMPTION),
        scope,
    })
}

// ── over-claim guards (design §5a) ───────────────────────────────────────────

/// Whether the external audit `sq-qhy4` is still open. The annotation graph and the
/// guards are written for the open state; this is the single switch the Phase 7
/// assurance-promotion bead (`sq-8ac8l`) flips. It is **hard-coded `true`** here —
/// only an external accredited-cryptographer sign-off (out of agent scope) may flip
/// it, and the README/SKILL document that. (Kept as a function, not a `const`, so a
/// future promotion has one obvious edit site.)
pub const fn audit_qhy4_open() -> bool {
    true
}

/// **Guard 1 (§5a rule 1).** Every method annotation where a **positive** property
/// carries [`Assurance::Proven`] while `sq-qhy4` is open. An empty result = no
/// over-claim. Only settled-negative levels (`PQForgeable`, `Replayable`,
/// `SchemeRevealed`) may be `Proven`. Operates over the supplied annotation set so a
/// caller can run it over a re-loaded or extended graph.
pub fn audit_overclaim_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
) -> Vec<Violation> {
    if !audit_qhy4_open() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (method, ann) in annotations {
        for a in &ann.assertions {
            if a.assurance == Assurance::Proven && a.is_positive() {
                out.push(Violation {
                    method: method.clone(),
                    kind: ViolationKind::ProvenPositiveWhileAuditOpen,
                    // Positional format args (CodeQL rust/unused-variable).
                    detail: format!(
                        "positive property {} (level {}) is Proven while sq-qhy4 is open",
                        a.property,
                        a.level.as_deref().unwrap_or("<none>"),
                    ),
                });
            }
        }
    }
    out
}

/// **Guard 3 (§5a rule 3).** Every production-selectable `zk:scheme` IRI in
/// `required_method_iris` that has no annotation block in `annotations`. An empty
/// result = complete coverage. The caller passes the production set (typically
/// [`production_method_iris`]).
pub fn completeness_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
    required_method_iris: &[&str],
) -> Vec<Violation> {
    required_method_iris
        .iter()
        .filter(|iri| !annotations.contains_key(**iri))
        .map(|iri| Violation {
            method: (*iri).to_owned(),
            kind: ViolationKind::MissingAnnotation,
            detail: format!("production-selectable scheme {} has no annotation block", iri),
        })
        .collect()
}

/// **Guard 2 (§5a rule 2).** Given a set of `(method, property, level)` query-proof
/// constraints a caller wants satisfied, report any that are satisfiable ONLY by a
/// source-layer-only assertion — i.e. a non-transfer violation. An empty result = no
/// improper transfer. (The positive form is
/// [`MethodAnnotations::admits_query_proof_property`].)
pub fn source_layer_transfer_violations(
    annotations: &BTreeMap<String, MethodAnnotations>,
    constraints: &[(&str, &str, Option<&str>)],
) -> Vec<Violation> {
    let mut out = Vec::new();
    for (method, property, level) in constraints {
        let Some(ann) = annotations.get(*method) else {
            continue;
        };
        // Is the constraint matched by ANY assertion, but NOT by a query-proof one?
        let matched_any = ann.assertions.iter().any(|a| {
            a.property == *property
                && match level {
                    None => true,
                    Some(l) => a.level.as_deref() == Some(*l),
                }
        });
        let matched_query_proof = ann.admits_query_proof_property(property, *level);
        if matched_any && !matched_query_proof {
            out.push(Violation {
                method: (*method).to_owned(),
                kind: ViolationKind::SourceLayerTransfer,
                detail: format!(
                    "{} satisfies query-proof constraint on {} only via a source-layer-only assertion",
                    method, property,
                ),
            });
        }
    }
    out
}

/// The `zk:scheme` IRIs that MUST carry an annotation block — the
/// production-selectable commitment methods
/// ([`crate::commit::CommitmentMethod::is_production_selectable`] == `true`). Built
/// from [`crate::commit::CommitmentMethod`] so it cannot drift from the registry's
/// notion of "production-selectable".
pub fn production_method_iris() -> Vec<&'static str> {
    use crate::commit::CommitmentMethod;
    // The string-canonical + dual-leaf methods are always present; value-only is the
    // research dial (NOT production-selectable) and is intentionally not required.
    [
        CommitmentMethod::StringCanonicalV1,
        CommitmentMethod::DualLeafV1,
    ]
    .into_iter()
    .filter(|m| m.is_production_selectable())
    .map(|m| m.scheme_iri())
    .collect()
}

/// The least-safe research dial `value-only`'s posture, returned in prose (it is NOT
/// a graph subject — it is OFF-by-default and not production-selectable, design §4.3).
/// Honest one-line summary for a caller that surfaces the full cost/safety frontier.
pub const fn value_only_posture() -> &'static str {
    "value-only (zk:poseidon2-valuehook-v1): research/benchmark dial only, NEVER \
     production-selectable; term identity is UNAVAILABLE (the lexical component is \
     dropped) and identity operators must be rejected at plan time; the cheapest and \
     least-safe point on the frontier (security write-up §4.3)"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled annotation graph is valid Turtle and non-empty.
    #[test]
    fn methods_ttl_is_valid_turtle() {
        let mut triples = 0usize;
        for result in TurtleParser::new().for_reader(METHODS_TTL.as_bytes()) {
            result.expect("secprop-methods.ttl must be valid Turtle");
            triples += 1;
        }
        assert!(triples > 0, "secprop-methods.ttl parsed to zero triples");
    }

    /// **Re-divergence tripwire (sq-3705).** This module must own NO `sec-prop:` IRI
    /// string of its own: every `secx:` term it keys on comes from the
    /// `sparq-secprop-vocab` leaf, which is where the one drift test against the
    /// canonical `secprop-ext.ttl` lives. Re-introducing a literal here would rebuild
    /// the copy #3705 deleted — and, because `sparq-trust` depends on `sparq-zk`, it
    /// could only be pinned again by a cross-package `include_str!`, the very thing
    /// that broke `cargo package` for this crate.
    ///
    /// Reads THIS file at compile time (an in-crate `include_str!` of `src/`, not a
    /// cross-package read), so it fails the moment such a literal reappears. The
    /// namespace it looks for is assembled at runtime so this assertion's own source
    /// is not a match.
    #[test]
    fn module_declares_no_local_sec_prop_iri_literal() {
        const THIS_FILE: &str = include_str!("secprop.rs");
        let ns = format!("{}{}", "https://w3id.org/zkp-sparql/", "sec-prop#");

        assert!(
            !THIS_FILE.contains(&ns),
            "crates/sparq-zk/src/secprop.rs contains a literal `{}` IRI — import the \
             constant from `sparq_secprop_vocab` instead (sq-3705): the vocabulary, \
             its canonical Turtle and the single drift test live there so this crate \
             needs neither a copy nor a cross-package `include_str!`",
            ns,
        );
    }

    /// The `secx:` terms this module keys on really are the leaf crate's — the
    /// positive half of the tripwire above. `ALL_SECPROP_IRIS` is pinned to
    /// `secprop-ext.ttl` by `sparq-secprop-vocab`'s own drift tests, so this
    /// transitively re-establishes the guarantee the deleted cross-package test gave.
    #[test]
    fn keyed_terms_come_from_the_canonical_vocabulary() {
        for iri in [
            SECX_HAS_PROPERTY,
            SECX_PROPERTY,
            SECX_LEVEL,
            SECX_ASSURANCE,
            SECX_AUDIT_STATUS,
            SECX_ASSUMPTION,
            SECX_SCOPE,
            SECX_PROVEN,
            SECX_CLAIMED,
            SECX_CONJECTURED,
            SECX_EXTERNAL_SIGN_OFF_PENDING,
            SECX_SOURCE_LAYER_ONLY,
            SECX_QUERY_PROOF_LAYER,
            SECX_PQ_FORGEABLE,
            SECX_REPLAYABLE,
            SECX_SCHEME_REVEALED,
        ] {
            assert!(
                iri.starts_with(SEC_PROP_NS),
                "`{}` is not in the sec-prop: namespace",
                iri,
            );
            assert!(
                sparq_secprop_vocab::ALL_SECPROP_IRIS.contains(&iri),
                "`{}` is not in the canonical `sparq_secprop_vocab::ALL_SECPROP_IRIS` \
                 registry — the annotation graph drifted from the vocabulary (sq-3705)",
                iri,
            );
        }
    }

    /// The graph parses to the production methods + the cryptosuite, each with at
    /// least one assertion.
    #[test]
    fn parses_each_production_method_with_assertions() {
        let ann = parse_annotations();
        for iri in production_method_iris() {
            let m = ann
                .get(iri)
                .unwrap_or_else(|| panic!("no annotation block for {}", iri));
            assert!(
                !m.assertions.is_empty(),
                "{} has an empty assertion set",
                iri,
            );
        }
        // The query-proof signature suite is annotated too.
        assert!(
            ann.contains_key(crate::sig::SignatureScheme::POSEIDON2_SCHNORR_V1_IRI),
            "the cryptosuite must be annotated",
        );
    }

    /// **Guard 3 holds**: every production-selectable scheme is annotated.
    #[test]
    fn completeness_guard_is_satisfied() {
        let ann = parse_annotations();
        let prod = production_method_iris();
        let v = completeness_violations(&ann, &prod);
        assert!(v.is_empty(), "completeness violations: {:?}", v);
    }

    /// **Guard 1 holds**: no positive property is `Proven` while sq-qhy4 is open.
    #[test]
    fn no_proven_positive_property_while_audit_open() {
        assert!(audit_qhy4_open(), "the guard is written for the open state");
        let ann = parse_annotations();
        let v = audit_overclaim_violations(&ann);
        assert!(v.is_empty(), "over-claim violations: {:?}", v);
    }

    /// And the settled NEGATIVE facts ARE `Proven` (the rule is two-sided: negatives
    /// are stated with full assurance, positives are not over-claimed).
    #[test]
    fn settled_negatives_are_proven() {
        let ann = parse_annotations();
        let rdfc10 = ann
            .get(crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1)
            .expect("string-canonical annotated");
        let pq = rdfc10
            .assertions
            .iter()
            .find(|a| a.level.as_deref() == Some(SECX_PQ_FORGEABLE))
            .expect("PQForgeable assertion present");
        assert_eq!(pq.assurance, Assurance::Proven, "PQ-forgeable is settled");
        let replay = rdfc10
            .assertions
            .iter()
            .find(|a| a.level.as_deref() == Some(SECX_REPLAYABLE))
            .expect("Replayable assertion present");
        assert_eq!(replay.assurance, Assurance::Proven, "Replayable is settled");
    }

    /// Every positive assertion that is `Claimed` carries the live
    /// `ExternalSignOffPending` audit status — the honesty pairing (a Claimed positive
    /// property must say it is unaudited).
    #[test]
    fn claimed_positives_are_external_sign_off_pending() {
        let ann = parse_annotations();
        for m in ann.values() {
            for a in &m.assertions {
                if a.assurance == Assurance::Claimed && a.is_positive() {
                    assert_eq!(
                        a.audit_status.as_deref(),
                        Some(SECX_EXTERNAL_SIGN_OFF_PENDING),
                        "{} property {} is Claimed but not ExternalSignOffPending",
                        m.method,
                        a.property,
                    );
                }
            }
        }
    }

    /// **Guard 2 holds (non-transfer)**: the illustrative source-layer-only
    /// `CrossPresentation` unlinkability does NOT satisfy a query-proof unlinkability
    /// constraint, even though the assertion exists on the method.
    #[test]
    fn source_layer_only_does_not_transfer_to_query_proof() {
        const ILLUSTRATIVE_SOURCE: &str =
            "https://sparq.dev/ns/zk#illustrative-source-bbs-2023";
        use sparq_secprop_vocab::{
            SECX_CROSS_PRESENTATION as CROSS_PRESENTATION,
            SECX_UNLINKABILITY_SCOPE as UNLINKABILITY_SCOPE,
        };

        let ann = parse_annotations();
        let src = ann
            .get(ILLUSTRATIVE_SOURCE)
            .expect("the illustrative source cryptosuite is annotated");
        // The assertion exists...
        assert!(
            src.assertions
                .iter()
                .any(|a| a.scope == Scope::SourceLayerOnly),
            "the illustrative block must carry a SourceLayerOnly assertion",
        );
        // ...but it does NOT admit a query-proof constraint (non-transfer).
        assert!(
            !src.admits_query_proof_property(UNLINKABILITY_SCOPE, Some(CROSS_PRESENTATION)),
            "a source-layer-only property must NOT satisfy a query-proof constraint",
        );
        // And the guard reports it as a transfer violation if a caller tries.
        let v = source_layer_transfer_violations(
            &ann,
            &[(ILLUSTRATIVE_SOURCE, UNLINKABILITY_SCOPE, Some(CROSS_PRESENTATION))],
        );
        assert_eq!(v.len(), 1, "expected exactly one transfer violation: {:?}", v);
        assert_eq!(v[0].kind, ViolationKind::SourceLayerTransfer);
    }

    /// A query-proof-layer property on a real method DOES transfer (the positive
    /// control for guard 2).
    #[test]
    fn query_proof_layer_property_transfers() {
        use sparq_secprop_vocab::{
            SECX_KNOWLEDGE_SOUND as KNOWLEDGE_SOUND, SECX_SOUNDNESS as SOUNDNESS,
        };
        let ann = parse_annotations();
        let m = ann
            .get(crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1)
            .expect("string-canonical annotated");
        assert!(
            m.admits_query_proof_property(SOUNDNESS, Some(KNOWLEDGE_SOUND)),
            "a query-proof-layer soundness claim must transfer",
        );
        // No transfer violation for a legitimately query-proof-layer property.
        let v = source_layer_transfer_violations(
            &ann,
            &[(
                crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1,
                SOUNDNESS,
                Some(KNOWLEDGE_SOUND),
            )],
        );
        assert!(v.is_empty(), "unexpected transfer violation: {:?}", v);
    }

    /// The dual-leaf method records the INV-VL downgrade as an explicit
    /// `secx:IssuerHonesty` assumption on its soundness claim (honest encoding of the
    /// #769-accepted downgrade), and it stays `Claimed` (never `Proven`).
    #[test]
    fn dual_leaf_records_issuer_honesty_assumption() {
        use sparq_secprop_vocab::{
            SECX_ISSUER_HONESTY as ISSUER_HONESTY, SECX_SOUNDNESS as SOUNDNESS,
        };
        let ann = parse_annotations();
        let dl = ann
            .get(crate::registry::ZK_SCHEME_POSEIDON2_DUALLEAF_V1)
            .expect("dual-leaf annotated");
        let soundness = dl
            .assertions
            .iter()
            .find(|a| a.property == SOUNDNESS)
            .expect("dual-leaf has a soundness assertion");
        assert_eq!(
            soundness.assumption.as_deref(),
            Some(ISSUER_HONESTY),
            "the dual-leaf INV-VL downgrade must be recorded as an IssuerHonesty assumption",
        );
        assert_eq!(
            soundness.assurance,
            Assurance::Claimed,
            "the dual-leaf soundness claim must stay Claimed (never Proven)",
        );
    }

    /// `value-only` is NOT a graph subject (it is the OFF-by-default research dial),
    /// and its posture is surfaced in prose instead.
    #[test]
    fn value_only_is_not_a_graph_subject_but_has_a_prose_posture() {
        let ann = parse_annotations();
        assert!(
            !ann.contains_key(crate::registry::ZK_SCHEME_POSEIDON2_VALUEHOOK_V1),
            "value-only must NOT be a production annotation subject",
        );
        assert!(
            value_only_posture().contains("NEVER"),
            "the value-only prose posture must flag it as never production-selectable",
        );
    }

    /// Minimal Turtle-aware tokenizer for the drift guard below: drops `#`
    /// comments (outside `<…>` IRIs and `"…"` literals), splits on whitespace,
    /// and emits `[ ] ( ) ; ,` as their own tokens — so a predicate and its
    /// object are ADJACENT tokens regardless of line breaks, indentation, or
    /// interleaved comments. NOT a Turtle parser (no long strings, no escapes
    /// beyond `\"`); just enough to make the scan formatting-independent.
    fn turtle_tokens(src: &str) -> Vec<&str> {
        let bytes = src.as_bytes();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                c if c.is_ascii_whitespace() => i += 1,
                b'#' => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'<' => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'>' {
                        i += 1;
                    }
                    i = (i + 1).min(bytes.len());
                    tokens.push(&src[start..i]);
                }
                b'"' => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                    i = (i + 1).min(bytes.len());
                    tokens.push(&src[start..i]);
                }
                b'[' | b']' | b'(' | b')' | b';' | b',' => {
                    tokens.push(&src[i..i + 1]);
                    i += 1;
                }
                _ => {
                    let start = i;
                    while i < bytes.len()
                        && !bytes[i].is_ascii_whitespace()
                        && !matches!(bytes[i], b'#' | b'[' | b']' | b'(' | b')' | b';' | b',')
                    {
                        i += 1;
                    }
                    tokens.push(&src[start..i]);
                }
            }
        }
        tokens
    }

    /// The object token of every `secx:property` assertion in `src`, in order.
    /// Token-based (see [`turtle_tokens`]), so `secx:property\n  # note\n  secx:X`
    /// is found exactly like the single-line form, and a commented-out
    /// annotation is NOT.
    fn secx_property_objects(src: &str) -> Vec<&str> {
        let tokens = turtle_tokens(src);
        tokens
            .windows(2)
            .filter(|pair| pair[0] == "secx:property")
            .map(|pair| pair[1])
            .collect()
    }

    /// **Dimension-IRI drift guard (sq-mgxz8, re-homed here by sq-3705).** Every
    /// `secx:property secx:*` dimension IRI in the bundled `secprop-methods.ttl`
    /// must be a term of the canonical `secx:` vocabulary — the registry
    /// `sparq-secprop-vocab` pins to `secprop-ext.ttl`. A typo or rename in the
    /// annotation graph's dimension IRI fails here.
    ///
    /// This guard used to live in `sparq_policy::secprop` and reach the graph with
    /// an `include_str!` of `../../sparq-zk/ontologies/secprop-methods.ttl`, because
    /// there was no crate that could see both the vocabulary and this graph. Now
    /// that the vocabulary is a dependency-free leaf, the guard sits in the crate
    /// that OWNS the graph and reads it over a real dependency edge — no
    /// cross-package file read, in either direction.
    #[test]
    fn methods_ttl_dims_are_canonical_vocabulary() {
        let canonical: std::collections::HashSet<&str> = sparq_secprop_vocab::ALL_SECPROP_IRIS
            .iter()
            .filter_map(|iri| iri.strip_prefix(SEC_PROP_NS))
            .collect();

        let mut found = 0usize;
        for object in secx_property_objects(METHODS_TTL) {
            // A non-`secx:` object (e.g. the full-IRI form) must fail LOUDLY,
            // not fall out of the scan: silent skips are exactly the drift this
            // guard exists to catch.
            let local = object.strip_prefix("secx:").unwrap_or_else(|| {
                panic!(
                    "secprop-methods.ttl has a `secx:property` object `{}` that is \
                     not in `secx:LocalName` prefix form — this drift guard only \
                     understands prefixed dimension IRIs (sq-mgxz8); either keep \
                     the annotation graph in prefix form or extend the guard",
                    object,
                )
            });
            found += 1;

            assert!(
                canonical.contains(local),
                "secprop-methods.ttl uses `secx:property secx:{}` but that IRI is not \
                 a term of the canonical secx: vocabulary \
                 (`sparq_secprop_vocab::ALL_SECPROP_IRIS`, pinned to secprop-ext.ttl) \
                 — possible IRI drift or an undeclared term (sq-mgxz8); if the term is \
                 intentional, declare it in the vocabulary crate",
                local,
            );
        }

        assert!(
            found > 0,
            "no `secx:property secx:*` dimension values found in secprop-methods.ttl — \
             the methods TTL format may have changed, breaking this drift guard (sq-mgxz8)",
        );
    }

    /// Regression (PR #3440 review): the drift-guard scan must be Turtle-
    /// formatting-independent. A reformatted annotation with the object on its
    /// own line behind a comment — plus a typo'd dimension name — MUST still be
    /// surfaced to the validation loop; the old exact-text `secx:property secx:`
    /// scan silently skipped it.
    #[test]
    fn secx_property_scan_survives_multiline_and_comment_formatting() {
        let ttl = "[ secx:property # the dimension\n      secx:TypoDim ;\n\
                   secx:level secx:Sound ] .\n\
                   [ secx:property secx:Soundness ; secx:level secx:Sound ] .";
        assert_eq!(
            secx_property_objects(ttl),
            vec!["secx:TypoDim", "secx:Soundness"],
            "a multiline/commented `secx:property` annotation must be found by \
             the scan exactly like the single-line form (sq-mgxz8)",
        );
    }

    /// Regression (PR #3440 review): a commented-out annotation is NOT a
    /// dimension assertion and must not be counted by the scan.
    #[test]
    fn secx_property_scan_ignores_comments_and_iris() {
        let ttl = "# [ secx:property secx:CommentedOut ] .\n\
                   <http://example.org/x#secx:property> a secx:Thing .\n\
                   [ secx:property secx:Real ] .";
        assert_eq!(
            secx_property_objects(ttl),
            vec!["secx:Real"],
            "commented-out annotations and IRI-internal text must not count as \
             `secx:property` assertions (sq-mgxz8)",
        );
    }
}

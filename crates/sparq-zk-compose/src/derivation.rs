// [OPUS-4.8] sq-314: derivation_step module + entailmentRegime end-to-end.
//! Derivation steps: the recorded inference that justifies a DERIVED triple under
//! a non-`Simple` entailment regime (plan §S2.5 with-inference).
//!
//! # Why this module exists (the sq-314 soundness gap it closes)
//! `ProofManifest::entailment_regime` used to be FREE METADATA: the verifier never
//! checked it, so a manifest could claim `Rdfs`/`Owl` while the proof attested only
//! the asserted triples (`Simple` semantics). A relying party reading the regime
//! field would believe inference was applied when nothing established it. This
//! module + the verifier's `crate::verifier::bind_entailment` make the regime
//! ENFORCED:
//! - the relying party declares which regimes it accepts (`EntailmentPolicy`);
//! - a regime the policy does not accept REJECTS (fail-closed);
//! - a non-`Simple` regime REQUIRES `derivation_steps` that STRUCTURALLY justify
//!   every derived triple against the regime's rule set, re-checked verifier-side.
//!
//! # Scope (HONEST — what is and is NOT proved)
//! A [`DerivationStep`] records a single rule application: the rule, its antecedent
//! triples, and the derived triple. The verifier STRUCTURALLY re-checks that each
//! step is a valid instance of its rule and that the derived triple chains from
//! antecedents that are either themselves derived by an earlier step or are
//! ASSERTED (disclosed by a scan sub-proof). This makes the regime claim
//! non-vacuous and auditable.
//!
//! What THIS host re-check DEFERS (documented, not silently assumed): a
//! ZERO-KNOWLEDGE proof that each antecedent triple is in the committed graph's
//! closure. This module ties antecedents to DISCLOSED scan rows (the asserted
//! base) by encoding-equality, so a derivation over disclosed triples is soundly
//! re-checkable; antecedents that are themselves only claimed (not disclosed and
//! not chained to disclosed triples) are NOT accepted. So the HOST capability is
//! sound for the disclosed-base fragment and fail-closed otherwise.
//!
//! The IN-CIRCUIT single-step upgrade now EXISTS as a research-grade Noir
//! relation — `zk/compose/compose_core::entail::entail_step_check` (sq-g91d): it
//! proves a derived triple follows by ONE RDFS rule application from antecedents
//! that are members of the COMMITTED graph, WITHOUT disclosing the antecedents,
//! bound to the SAME per-graph commitments the scan sub-proofs carry — the
//! privacy upgrade this disclosed-base re-check cannot give. It is single-step /
//! disclosed-base-membership scope (multi-hop fixpoint SATURATION is a separate,
//! UNBUILT deliverable) and NOT-yet-sound (sq-qhy4, not externally audited). The
//! compiled `entail_rdfs_k{K}_n{N}` bin member (with its gate-count baseline) and
//! the verifier-side manifest/dispatch/public-input wiring that would let this
//! host path CONSUME an in-circuit derivation are follow-up beads; until they
//! land, this host re-check remains disclosed-base only.
//!
//! # The TWO obligations — never conflate them (`sq-rsd3v.7`)
//! `research/zk-inference-and-credentials.md` §3.7 keeps two DISTINCT properties
//! apart, and this module supplies exactly ONE of them:
//!
//! 1. **SOUNDNESS of derivation** — "every disclosed derived triple IS entailed".
//!    That is what [`DerivationStep`] + `verifier::bind_entailment` re-check
//!    host-side over the disclosed base, and what the in-circuit relation
//!    (`zk/compose/compose_core/src/derivation.nr`, `sq-rsd3v.2`) states over
//!    HIDDEN antecedents. Supplied, as scoped (and NOT externally audited —
//!    `sq-qhy4`).
//! 2. **COMPLETENESS under entailment** — "no entailed answer is MISSING": a
//!    NEGATIVE over a fixpoint. **UNBUILT and NOT claimed.** It needs BOTH halves
//!    listed in [`COMPLETENESS_UNDER_ENTAILMENT_UNBUILT`], and the saturation half
//!    exists nowhere in sparq.
//!
//! A relying party that needs (2) must NOT read an accepted `Rdfs`/`Owl` manifest
//! as supplying it. That conflation is MACHINE-CHECKED rather than left to prose:
//! `EntailmentPolicy::require_completeness_under_entailment` makes the verifier
//! REFUSE every non-`Simple` manifest fail-closed
//! (`CheckError::CompletenessUnderEntailmentUnavailable`) instead of returning an
//! accept the relying party could misread as completeness.
//!
//! Folding (Nova/HyperNova/Protostar) / zkVM re-execution is the only shape that
//! natively gives (2), and it was measured OUT at credential scale (design §3.6(c);
//! `research/zkp-performance-landscape.md` §5 trigger 4). The documented RE-ENTRY
//! TRIGGER is a huge closure PLUS a verifier that demands full completeness. Until
//! that fires, soundness-first (`sq-rsd3v.2`/`.3`) is the path and (2) stays an
//! OPEN, honestly-deferred problem.

use serde::{Deserialize, Serialize};

use crate::manifest::{EntailmentRegime, FieldHex};

/// The TWO obligations completeness-under-entailment requires (design §3.7),
/// **NEITHER of which is built** in sparq. This is the single source of truth the
/// verifier's refusal message and its regression test share, so the honest scope
/// cannot decay into prose only one of them remembers.
///
/// Half (a) is the entailment analogue of `scan.nr`'s asserted-base sweep: the
/// closure — not just the base — would have to be swept in-circuit so no matching
/// entailed triple can be withheld. Half (b) is the part with no precedent
/// anywhere in the estate: proving that the fixpoint was REACHED, i.e. that no
/// rule fires producing a new triple.
// [OPUS-5] sq-rsd3v.7: completeness-under-entailment is UNBUILT and NOT claimed.
pub const COMPLETENESS_UNDER_ENTAILMENT_UNBUILT: [&str; 2] = [
    "in-circuit closure-sweep over the flat full graph",
    "fixpoint-saturation proof (no rule fires producing a new triple)",
];

/// The RDFS/OWL-RL rule a [`DerivationStep`] instantiates. v1 ships the RDFS
/// rules whose antecedents are fixed-shape Datalog over term encodings (the
/// subset expressible over disclosed/committed triple encodings — the phased
/// RULE SCOPE of `research/zk-inference-and-credentials.md §3.5`); the enum is
/// the extension point for the full rule set. `owl:sameAs` is gated SEPARATELY
/// (`sq-rsd3v.6`, the [`crate::sameas`] canonicalisation path — NOT `.7`, which
/// is the distinct completeness obligation above) — encoding-equality re-checks
/// are UNSOUND under equality reasoning — and must never ride this fixed-shape
/// path; [`DerivationStep::mentions_equality_predicate`] enforces that
/// fail-closed.
///
/// Each rule's antecedent/consequent SHAPE is fixed and re-checked by
/// [`DerivationStep::is_well_formed`]; the rule is identified in the manifest by
/// its kebab-case tag so the schema is stable as rules are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntailmentRule {
    /// RDFS rule rdfs9 (subClassOf transitivity into type):
    /// `(?x rdf:type ?c1)` + `(?c1 rdfs:subClassOf ?c2)` ⊢ `(?x rdf:type ?c2)`.
    Rdfs9SubClassType,
    /// RDFS rule rdfs7 (subPropertyOf):
    /// `(?p1 rdfs:subPropertyOf ?p2)` + `(?x ?p1 ?y)` ⊢ `(?x ?p2 ?y)`.
    Rdfs7SubProperty,
    /// RDFS rule rdfs2 (property domain into type):
    /// `(?p rdfs:domain ?c)` + `(?x ?p ?y)` ⊢ `(?x rdf:type ?c)`.
    Rdfs2Domain,
    /// RDFS rule rdfs3 (property range into type):
    /// `(?p rdfs:range ?c)` + `(?x ?p ?y)` ⊢ `(?y rdf:type ?c)`.
    Rdfs3Range,
    /// RDFS rule rdfs5 (subPropertyOf transitivity):
    /// `(?p1 rdfs:subPropertyOf ?p2)` + `(?p2 rdfs:subPropertyOf ?p3)`
    /// ⊢ `(?p1 rdfs:subPropertyOf ?p3)`.
    Rdfs5SubPropertyTrans,
    /// RDFS rule rdfs11 (subClassOf transitivity):
    /// `(?c1 rdfs:subClassOf ?c2)` + `(?c2 rdfs:subClassOf ?c3)`
    /// ⊢ `(?c1 rdfs:subClassOf ?c3)`.
    Rdfs11SubClassTrans,
}

impl EntailmentRule {
    /// The minimum entailment regime under which this rule is admissible. All v1
    /// rules are RDFS rules, so they require at least `Rdfs` (and are also valid
    /// under `Owl`, which subsumes RDFS). A `Simple` manifest may carry NO
    /// derivation steps (no inference).
    pub fn min_regime(self) -> EntailmentRegime {
        match self {
            EntailmentRule::Rdfs9SubClassType
            | EntailmentRule::Rdfs7SubProperty
            | EntailmentRule::Rdfs2Domain
            | EntailmentRule::Rdfs3Range
            | EntailmentRule::Rdfs5SubPropertyTrans
            | EntailmentRule::Rdfs11SubClassTrans => EntailmentRegime::Rdfs,
        }
    }
}

/// Whether `regime` permits `rule` (the regime is at least the rule's minimum).
/// `Owl` subsumes `Rdfs`; `Simple` permits NO rule.
pub fn regime_admits(regime: EntailmentRegime, rule: EntailmentRule) -> bool {
    match (regime, rule.min_regime()) {
        (EntailmentRegime::Simple, _) => false,
        (EntailmentRegime::Rdfs, EntailmentRegime::Rdfs) => true,
        (EntailmentRegime::Owl, EntailmentRegime::Rdfs | EntailmentRegime::Owl) => true,
        // Owl rule under Rdfs regime, etc.
        _ => false,
    }
}

/// A single recorded inference step: a rule applied to `antecedents` yielding the
/// `derived` triple. Triples are carried as their per-slot TERM ENCODINGS (the
/// same `FieldHex` the scan sub-proofs disclose), so the verifier can tie an
/// antecedent to a disclosed scan row by encoding-equality.
///
/// The verifier re-checks (see `crate::verifier::bind_entailment`):
/// 1. the step is well-formed for its rule (arity + the rule's variable-sharing
///    shape — [`Self::is_well_formed`]);
/// 2. its regime admits the rule ([`regime_admits`]);
/// 3. every antecedent is GROUNDED: it equals either an earlier step's `derived`
///    triple or a triple disclosed by a scan sub-proof (the asserted base).
///
/// A step that fails any of these REJECTS the manifest (fail-closed) — a derived
/// triple cannot rest on an ungrounded antecedent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationStep {
    /// The rule this step instantiates.
    pub rule: EntailmentRule,
    /// The antecedent triples (term encodings), in the rule's declared order.
    pub antecedents: Vec<[FieldHex; 3]>,
    /// The derived triple (term encodings).
    pub derived: [FieldHex; 3],
}

impl DerivationStep {
    /// Whether this step is a WELL-FORMED instance of its rule: the antecedent
    /// arity and the rule's variable-sharing/consequent shape hold over the term
    /// encodings (an encoding equality is a term equality, so the shape check is
    /// sound over disclosed encodings). Returns `false` on any mismatch.
    ///
    /// Rule shapes (positions are [s, p, o]):
    /// - `Rdfs9SubClassType`: antecedents `[(x, type, c1), (c1, subClassOf, c2)]`,
    ///   derived `(x, type, c2)`. Checks: 2 antecedents; a0.p == rdf:type (the
    ///   `rdf_type` arg); a1.p == rdfs:subClassOf (`rdfs_subclassof`); a0.o == a1.s
    ///   (the shared class c1); derived == (a0.s, rdf:type, a1.o).
    /// - `Rdfs7SubProperty`: antecedents `[(p1, subPropertyOf, p2), (x, p1, y)]`,
    ///   derived `(x, p2, y)`. Checks: 2 antecedents; a0.p == rdfs:subPropertyOf;
    ///   a0.s == a1.p (the shared property p1); derived == (a1.s, a0.o, a1.o).
    /// - `Rdfs2Domain`: antecedents `[(p, domain, c), (x, p, y)]`, derived
    ///   `(x, type, c)`. Checks: 2 antecedents; a0.p == rdfs:domain; a0.s == a1.p
    ///   (the shared property p); derived == (a1.s, rdf:type, a0.o).
    /// - `Rdfs3Range`: antecedents `[(p, range, c), (x, p, y)]`, derived
    ///   `(y, type, c)`. Checks: 2 antecedents; a0.p == rdfs:range; a0.s == a1.p
    ///   (the shared property p); derived == (a1.o, rdf:type, a0.o).
    /// - `Rdfs5SubPropertyTrans`: antecedents
    ///   `[(p1, subPropertyOf, p2), (p2, subPropertyOf, p3)]`, derived
    ///   `(p1, subPropertyOf, p3)`. Checks: 2 antecedents; a0.p == a1.p ==
    ///   rdfs:subPropertyOf; a0.o == a1.s (the shared property p2); derived ==
    ///   (a0.s, subPropertyOf, a1.o).
    /// - `Rdfs11SubClassTrans`: antecedents
    ///   `[(c1, subClassOf, c2), (c2, subClassOf, c3)]`, derived
    ///   `(c1, subClassOf, c3)`. Checks: 2 antecedents; a0.p == a1.p ==
    ///   rdfs:subClassOf; a0.o == a1.s (the shared class c2); derived ==
    ///   (a0.s, subClassOf, a1.o).
    ///
    /// The schema-term encodings (`rdf:type`, `rdfs:subClassOf`,
    /// `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`) are supplied by the
    /// caller (the verifier computes them once via `encode_term`) so this module
    /// stays free of the encoding layer.
    pub fn is_well_formed(
        &self,
        rdf_type: &FieldHex,
        rdfs_subclassof: &FieldHex,
        rdfs_subpropertyof: &FieldHex,
        rdfs_domain: &FieldHex,
        rdfs_range: &FieldHex,
    ) -> bool {
        match self.rule {
            EntailmentRule::Rdfs9SubClassType => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (x, type, c1)
                let a1 = &self.antecedents[1]; // (c1, subClassOf, c2)
                a0[1] == *rdf_type
                    && a1[1] == *rdfs_subclassof
                    && a0[2] == a1[0] // shared c1
                    && self.derived[0] == a0[0] // x
                    && self.derived[1] == *rdf_type
                    && self.derived[2] == a1[2] // c2
            }
            EntailmentRule::Rdfs7SubProperty => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (p1, subPropertyOf, p2)
                let a1 = &self.antecedents[1]; // (x, p1, y)
                a0[1] == *rdfs_subpropertyof
                    && a0[0] == a1[1] // shared p1
                    && self.derived[0] == a1[0] // x
                    && self.derived[1] == a0[2] // p2
                    && self.derived[2] == a1[2] // y
            }
            EntailmentRule::Rdfs2Domain => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (p, domain, c)
                let a1 = &self.antecedents[1]; // (x, p, y)
                a0[1] == *rdfs_domain
                    && a0[0] == a1[1] // shared p
                    && self.derived[0] == a1[0] // x
                    && self.derived[1] == *rdf_type
                    && self.derived[2] == a0[2] // c
            }
            EntailmentRule::Rdfs3Range => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (p, range, c)
                let a1 = &self.antecedents[1]; // (x, p, y)
                a0[1] == *rdfs_range
                    && a0[0] == a1[1] // shared p
                    && self.derived[0] == a1[2] // y
                    && self.derived[1] == *rdf_type
                    && self.derived[2] == a0[2] // c
            }
            EntailmentRule::Rdfs5SubPropertyTrans => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (p1, subPropertyOf, p2)
                let a1 = &self.antecedents[1]; // (p2, subPropertyOf, p3)
                a0[1] == *rdfs_subpropertyof
                    && a1[1] == *rdfs_subpropertyof
                    && a0[2] == a1[0] // shared p2
                    && self.derived[0] == a0[0] // p1
                    && self.derived[1] == *rdfs_subpropertyof
                    && self.derived[2] == a1[2] // p3
            }
            EntailmentRule::Rdfs11SubClassTrans => {
                if self.antecedents.len() != 2 {
                    return false;
                }
                let a0 = &self.antecedents[0]; // (c1, subClassOf, c2)
                let a1 = &self.antecedents[1]; // (c2, subClassOf, c3)
                a0[1] == *rdfs_subclassof
                    && a1[1] == *rdfs_subclassof
                    && a0[2] == a1[0] // shared c2
                    && self.derived[0] == a0[0] // c1
                    && self.derived[1] == *rdfs_subclassof
                    && self.derived[2] == a1[2] // c3
            }
        }
    }

    /// Whether this step INTRODUCES or CONSUMES an `owl:sameAs` fact — i.e.
    /// whether the `owl:sameAs` encoding stands in the PREDICATE slot of any
    /// antecedent or of the derived triple.
    ///
    /// # Why the fixed-shape path must refuse these (sq-rsd3v.6)
    /// The rules above are re-checked by term-encoding equality, and an
    /// encoding equality is a term IDENTITY. Under equality reasoning that
    /// proxy is wrong: `owl:sameAs` quotients the term universe, so term
    /// identity is strictly FINER than entailment, and an equality-flavoured
    /// step re-checked this way would either fire vacuously or — if the
    /// equality test were relaxed to admit claimed `sameAs` links — TRUST
    /// equalities instead of proving them. Equality reasoning needs the
    /// in-circuit union-find canonicalisation of [`crate::sameas`], whose
    /// soundness argument rests on canonical representatives, not raw
    /// encodings.
    ///
    /// The shapes that would otherwise slip through are real, not theoretical:
    /// `rdfs7` with `p1 = owl:sameAs` CONSUMES an equality
    /// (`(sameAs subPropertyOf q), (x sameAs y) ⊢ (x q y)`), and `rdfs7` with
    /// `p2 = owl:sameAs` INTRODUCES one (`(q subPropertyOf sameAs), (x q y)
    /// ⊢ (x sameAs y)`). Both are legitimate *RDFS* entailments, so refusing
    /// them costs only provability — the conservative, fail-closed direction —
    /// while making it impossible for `owl:sameAs` to ride this path silently.
    ///
    /// Only the PREDICATE slot is tested: a step may still mention the
    /// `owl:sameAs` IRI as a subject or object (schema talk *about* the
    /// property, e.g. `(owl:sameAs rdfs:subPropertyOf ?q)`), because that alone
    /// neither asserts nor uses an equality — the equality fact can only enter
    /// or be consumed through a predicate slot, which is exactly what this
    /// covers.
    ///
    /// Encodings are compared NORMALIZED (the `crate::verifier` hex
    /// normalization the grounding check uses). `is_well_formed` can compare
    /// raw, because it only ever tests a prover encoding against a
    /// verifier-computed canonical one and a spelling mismatch there REJECTS;
    /// here a spelling mismatch would ACCEPT, so the safe direction is the
    /// opposite one and normalization is load-bearing.
    // [OPUS-5] sq-rsd3v.6 (#3265): keep owl:sameAs off the fixed-shape path.
    pub fn mentions_equality_predicate(&self, owl_sameas: &FieldHex) -> bool {
        let same = crate::verifier::normalize_hex(&owl_sameas.0);
        self.antecedents
            .iter()
            .chain(std::iter::once(&self.derived))
            .any(|t| crate::verifier::normalize_hex(&t[1].0) == same)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    // Schema-vocabulary encodings the verifier supplies (arbitrary distinct
    // constants; the shape check only cares about equality structure).
    const T: &str = "0x01"; // rdf:type
    const SC: &str = "0x02"; // rdfs:subClassOf
    const SP: &str = "0x03"; // rdfs:subPropertyOf
    const DOM: &str = "0x04"; // rdfs:domain
    const RNG: &str = "0x05"; // rdfs:range

    /// Every schema encoding, in the `is_well_formed` argument order.
    fn schema() -> (FieldHex, FieldHex, FieldHex, FieldHex, FieldHex) {
        (fh(T), fh(SC), fh(SP), fh(DOM), fh(RNG))
    }

    /// `step.is_well_formed(..)` with the fixed schema constants spread in.
    fn wf(step: &DerivationStep) -> bool {
        let (t, sc, sp, dom, rng) = schema();
        step.is_well_formed(&t, &sc, &sp, &dom, &rng)
    }

    #[test]
    fn rdfs9_well_formed_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs9SubClassType,
            // (x type c1), (c1 subClassOf c2) ⊢ (x type c2)
            antecedents: vec![
                [fh("0xaa"), fh(T), fh("0xc1")],
                [fh("0xc1"), fh(SC), fh("0xc2")],
            ],
            derived: [fh("0xaa"), fh(T), fh("0xc2")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs9_rejects_broken_class_chain() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs9SubClassType,
            antecedents: vec![
                [fh("0xaa"), fh(T), fh("0xc1")],
                // c1 != cX (broken shared class)
                [fh("0xcX"), fh(SC), fh("0xc2")],
            ],
            derived: [fh("0xaa"), fh(T), fh("0xc2")],
        };
        assert!(!wf(&step));
    }

    #[test]
    fn rdfs7_well_formed_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs7SubProperty,
            // (p1 subPropertyOf p2), (x p1 y) ⊢ (x p2 y)
            antecedents: vec![
                [fh("0xp1"), fh(SP), fh("0xp2")],
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            derived: [fh("0xaa"), fh("0xp2"), fh("0xbb")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs2_domain_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs2Domain,
            // (p domain c), (x p y) ⊢ (x type c)
            antecedents: vec![
                [fh("0xp1"), fh(DOM), fh("0xcc")],
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            derived: [fh("0xaa"), fh(T), fh("0xcc")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs2_domain_rejects_wrong_derived_subject() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs2Domain,
            antecedents: vec![
                [fh("0xp1"), fh(DOM), fh("0xcc")],
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            // domain types the SUBJECT (x); using the object (y) is unsound.
            derived: [fh("0xbb"), fh(T), fh("0xcc")],
        };
        assert!(!wf(&step));
    }

    #[test]
    fn rdfs3_range_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs3Range,
            // (p range c), (x p y) ⊢ (y type c)
            antecedents: vec![
                [fh("0xp1"), fh(RNG), fh("0xcc")],
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            derived: [fh("0xbb"), fh(T), fh("0xcc")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs3_range_rejects_wrong_derived_subject() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs3Range,
            antecedents: vec![
                [fh("0xp1"), fh(RNG), fh("0xcc")],
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            // range types the OBJECT (y); using the subject (x) is unsound.
            derived: [fh("0xaa"), fh(T), fh("0xcc")],
        };
        assert!(!wf(&step));
    }

    #[test]
    fn rdfs2_and_rdfs3_do_not_cross_accept() {
        // A rdfs2 (domain) antecedent must NOT satisfy the rdfs3 (range) shape:
        // the predicate slot differs (domain vs range), so the tag is load-bearing.
        let domain_step = DerivationStep {
            rule: EntailmentRule::Rdfs3Range, // claim range...
            antecedents: vec![
                [fh("0xp1"), fh(DOM), fh("0xcc")], // ...over a domain edge
                [fh("0xaa"), fh("0xp1"), fh("0xbb")],
            ],
            derived: [fh("0xbb"), fh(T), fh("0xcc")],
        };
        assert!(!wf(&domain_step));
    }

    #[test]
    fn rdfs5_subproperty_transitivity_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs5SubPropertyTrans,
            // (p1 subPropertyOf p2), (p2 subPropertyOf p3) ⊢ (p1 subPropertyOf p3)
            antecedents: vec![
                [fh("0xp1"), fh(SP), fh("0xp2")],
                [fh("0xp2"), fh(SP), fh("0xp3")],
            ],
            derived: [fh("0xp1"), fh(SP), fh("0xp3")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs5_rejects_broken_property_chain() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs5SubPropertyTrans,
            antecedents: vec![
                [fh("0xp1"), fh(SP), fh("0xp2")],
                // p2 != pX (broken shared middle property)
                [fh("0xpX"), fh(SP), fh("0xp3")],
            ],
            derived: [fh("0xp1"), fh(SP), fh("0xp3")],
        };
        assert!(!wf(&step));
    }

    #[test]
    fn rdfs11_subclass_transitivity_accepts_valid_shape() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs11SubClassTrans,
            // (c1 subClassOf c2), (c2 subClassOf c3) ⊢ (c1 subClassOf c3)
            antecedents: vec![
                [fh("0xc1"), fh(SC), fh("0xc2")],
                [fh("0xc2"), fh(SC), fh("0xc3")],
            ],
            derived: [fh("0xc1"), fh(SC), fh("0xc3")],
        };
        assert!(wf(&step));
    }

    #[test]
    fn rdfs11_rejects_broken_class_chain() {
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs11SubClassTrans,
            antecedents: vec![
                [fh("0xc1"), fh(SC), fh("0xc2")],
                // c2 != cX (broken shared middle class)
                [fh("0xcX"), fh(SC), fh("0xc3")],
            ],
            derived: [fh("0xc1"), fh(SC), fh("0xc3")],
        };
        assert!(!wf(&step));
    }

    // The `owl:sameAs` encoding the verifier recomputes (an arbitrary distinct
    // constant here — the guard only cares about equality structure).
    const SAME: &str = "0x06";

    #[test]
    fn equality_predicate_guard_catches_a_consumed_sameas() {
        // rdfs7 with p1 = owl:sameAs CONSUMES an equality:
        //   (sameAs subPropertyOf q), (x sameAs y) ⊢ (x q y)
        // The shape is a legitimate RDFS instance, so `is_well_formed` accepts
        // it — only the equality guard refuses it.
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs7SubProperty,
            antecedents: vec![
                [fh(SAME), fh(SP), fh("0xq")],
                [fh("0xaa"), fh(SAME), fh("0xbb")],
            ],
            derived: [fh("0xaa"), fh("0xq"), fh("0xbb")],
        };
        assert!(wf(&step), "the forge is shape-valid — the guard must be what stops it");
        assert!(step.mentions_equality_predicate(&fh(SAME)));
    }

    #[test]
    fn equality_predicate_guard_catches_an_introduced_sameas() {
        // rdfs7 with p2 = owl:sameAs INTRODUCES one:
        //   (q subPropertyOf sameAs), (x q y) ⊢ (x sameAs y)
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs7SubProperty,
            antecedents: vec![
                [fh("0xq"), fh(SP), fh(SAME)],
                [fh("0xaa"), fh("0xq"), fh("0xbb")],
            ],
            derived: [fh("0xaa"), fh(SAME), fh("0xbb")],
        };
        assert!(wf(&step));
        assert!(step.mentions_equality_predicate(&fh(SAME)));
    }

    #[test]
    fn equality_predicate_guard_normalizes_the_encoding() {
        // A prover-chosen spelling must not slip the guard: here the encoding
        // is unprefixed and upper-case, but it is the SAME term.
        let step = DerivationStep {
            rule: EntailmentRule::Rdfs7SubProperty,
            antecedents: vec![
                [fh("0xq"), fh(SP), fh("06")],
                [fh("0xaa"), fh("0xq"), fh("0xbb")],
            ],
            derived: [fh("0xaa"), fh("06"), fh("0xbb")],
        };
        assert!(step.mentions_equality_predicate(&fh(SAME)));
    }

    #[test]
    fn equality_predicate_guard_leaves_ordinary_rdfs_steps_alone() {
        // The guard is precise: an ordinary rdfs9 step, and a step that merely
        // mentions owl:sameAs as a TERM (schema talk about the property, not an
        // equality assertion), both stay admissible.
        let ordinary = DerivationStep {
            rule: EntailmentRule::Rdfs9SubClassType,
            antecedents: vec![
                [fh("0xaa"), fh(T), fh("0xc1")],
                [fh("0xc1"), fh(SC), fh("0xc2")],
            ],
            derived: [fh("0xaa"), fh(T), fh("0xc2")],
        };
        assert!(!ordinary.mentions_equality_predicate(&fh(SAME)));

        let mentions_as_term = DerivationStep {
            rule: EntailmentRule::Rdfs11SubClassTrans,
            antecedents: vec![
                [fh(SAME), fh(SC), fh("0xc2")],
                [fh("0xc2"), fh(SC), fh("0xc3")],
            ],
            derived: [fh(SAME), fh(SC), fh("0xc3")],
        };
        assert!(!mentions_as_term.mentions_equality_predicate(&fh(SAME)));
    }

    #[test]
    fn regime_admits_only_matching_rules() {
        assert!(!regime_admits(EntailmentRegime::Simple, EntailmentRule::Rdfs9SubClassType));
        assert!(regime_admits(EntailmentRegime::Rdfs, EntailmentRule::Rdfs7SubProperty));
        assert!(regime_admits(EntailmentRegime::Owl, EntailmentRule::Rdfs9SubClassType));
        // The four new rules are RDFS rules and admitted under Rdfs and Owl.
        for rule in [
            EntailmentRule::Rdfs2Domain,
            EntailmentRule::Rdfs3Range,
            EntailmentRule::Rdfs5SubPropertyTrans,
            EntailmentRule::Rdfs11SubClassTrans,
        ] {
            assert!(!regime_admits(EntailmentRegime::Simple, rule));
            assert!(regime_admits(EntailmentRegime::Rdfs, rule));
            assert!(regime_admits(EntailmentRegime::Owl, rule));
        }
    }
}

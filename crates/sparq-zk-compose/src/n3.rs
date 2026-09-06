// [OPUS-5] sq-rsd3v.3 (#2946): witnessed-rule-shape N3 derivation -- the host
// mirror of the in-circuit relation `zk/compose/compose_core/src/n3.nr`, plus
// the PROVABLE-SUBSET admission gate and the rule-graph commitment builder.
// Epic sq-rsd3v; parent #2616. Blocked-by sq-rsd3v.2 (#2614).
//! N3 `{premise} => {conclusion}` rules as a **witnessed rule shape**: the
//! declared provable subset, its fail-closed admission gate, and the rule-graph
//! commitment that makes a proof say **whose** rules it used.
//!
//! # Why this is the harder sibling of [`crate::derivation`]
//! For RDFS/OWL-RL the rule set is a FIXED PUBLIC TABLE, so
//! [`crate::derivation::EntailmentRule`] can be a closed enum of six shapes and
//! the verifier re-checks each by name. For N3 the rule set is
//! **dataset-supplied**, so there is no table to name. Two moves follow
//! (`research/zk-inference-and-credentials.md` §4.1):
//!
//! 1. **The rule author becomes an ISSUER.** The rule graph is committed into
//!    the signed-input set exactly like a TBox — [`N3RuleSet::commit`] folds it
//!    to a root the author signs (with the existing `sq-z9l` issuer machinery;
//!    this module produces the ROOT, not the signature). A derivation is only
//!    as sound as its rules, so the proof must say WHOSE rules, and a prover
//!    cannot substitute a convenient shape without changing the root.
//! 2. **The rule shape is itself a WITNESS.** Each pattern slot is a
//!    [`N3Slot::Const`] term encoding or a [`N3Slot::Var`] index; the circuit
//!    checks the conclusion IS the premise pattern under a CONSISTENT variable
//!    substitution, which is structural because the substitution is an array
//!    indexed by variable id.
//!
//! # SAFETY is a soundness condition, not a style rule
//! The substitution is a PRIVATE witness. If a conclusion variable were not
//! bound by the premise, the prover would be free to CHOOSE its value and
//! "derive" an arbitrary triple. So the subset's range-restriction condition —
//! every conclusion variable bound in the premise — is load-bearing, and
//! [`N3Rule::admit`] enforces it with the same BINDING SCHEDULE the circuit
//! runs (`n3.nr`'s `rule_subset_check`): premise atoms in order, a join atom
//! binds every variable slot it carries, an arithmetic builtin requires its
//! input variables already bound and binds its output, a comparison binds
//! nothing and requires both operands bound. That order mirrors how
//! `sparq-reason` already reorders premises so a builtin runs only after the
//! atoms producing its inputs (cwm's "when ready").
//!
//! # The PROVABLE SUBSET v1 (declared; fail-closed outside it — §4.3)
//! **INCLUDED:** safe, ground-deriving, FORWARD rules `{p1 . p2 …} => {c1 …}`
//! with universals, plus the whitelisted `math:` builtins as fixed gadgets —
//! comparisons ([`N3Builtin::GreaterThan`], [`N3Builtin::LessThan`],
//! [`N3Builtin::NotGreaterThan`], [`N3Builtin::NotLessThan`],
//! [`N3Builtin::EqualTo`]), which reuse the existing `filter_signed` comparison
//! circuit verbatim, and arithmetic ([`N3Builtin::Sum`],
//! [`N3Builtin::Difference`], [`N3Builtin::Product`]) as field ops with range
//! checks. Path syntax (`!` / `^`) is fine: `sparq-reason` desugars it into
//! fresh-variable join triples BEFORE the proof step, so it never reaches here.
//!
//! **EXCLUDED and fail-closed:** existentials-in-conclusion (no in-circuit
//! fresh blank-node minting); scoped negation-as-failure (`log:notIncludes` /
//! `log:collectAllIn` — proving a NEGATIVE needs the saturation machinery,
//! deferred with completeness, `sq-rsd3v.7`); `math:quotient` /
//! `math:exponentiation` / floats; `string:` builtins (no in-circuit UTF-8
//! reasoning); `list:` generators; `time:` decomposition; `log:semantics` /
//! `log:includes`; backward rules that do not reduce to a forward closure; any
//! rule whose closure is unbounded.
//!
//! A rule outside the subset is **REJECTED, never approximated**, and the
//! refusal is structural rather than advisory: [`N3RuleSet::commit`] runs
//! [`N3RuleSet::admit`] FIRST, so an out-of-subset rule graph has no root — it
//! cannot be committed, so it can never reach the circuit's `rules_root` in the
//! first place. The circuit independently re-checks the same conditions over
//! the witnessed shape, so a hand-rolled witness cannot bypass this gate
//! either.
//!
//! # `owl:sameAs`
//! Variable sharing is re-checked by term-encoding equality, which is term
//! IDENTITY — the wrong proxy once `owl:sameAs` quotients the term universe.
//! Equality reasoning must ride [`crate::sameas`]'s canonicalisation instead,
//! exactly as for [`crate::derivation`]; this module has no equality semantics.
//!
//! # What is NOT claimed
//! - **No completeness, in either direction.** This is SOUNDNESS of derivation
//!   ("every disclosed derived row IS entailed by the committed rules").
//!   COMPLETENESS under entailment ("no entailed answer is MISSING") is the
//!   distinct `sq-rsd3v.7` obligation and is UNBUILT — see
//!   [`crate::derivation::COMPLETENESS_UNDER_ENTAILMENT_UNBUILT`]. A rule this
//!   subset refuses is not thereby shown unsound; it is shown unprovable HERE.
//! - **No cost claim.** The in-circuit relation's cost has NOT been measured
//!   (`bb gates` has not been run over a compiled monomorphisation) and repo
//!   policy forbids an unmeasured figure.
//! - **Status.** Research-grade, NOT externally audited (`sq-qhy4`); it
//!   inherits every soundness caveat of `sq-rsd3v.2`. This module is the host
//!   MIRROR + commitment builder: no `ProofManifest` schema, [`crate::CircuitId`]
//!   or `verify_manifest` dispatch binds an N3 proof yet, no compiled
//!   `n3_k{K}_n{N}_r{R}_m{M}` bin package exists, and the off-circuit witness
//!   generator that maps `sparq-reason`'s `reason_n3_proof` / `n3_proof_tree`
//!   `ProofStep`s onto these slots is a follow-up.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sparq_zk::field::Fr;
use sparq_zk::poseidon2;

use crate::manifest::FieldHex;

/// The SWAP `math:` namespace — the only builtin namespace with whitelisted
/// members in v1.
pub const MATH_NS: &str = "http://www.w3.org/2000/10/swap/math#";
/// The SWAP `log:` namespace. Wholly EXCLUDED: `log:notIncludes` /
/// `log:collectAllIn` are negation-as-failure (a NEGATIVE over the closure,
/// which needs the unbuilt saturation machinery, `sq-rsd3v.7`), and
/// `log:semantics` / `log:includes` reach outside the committed graph.
pub const LOG_NS: &str = "http://www.w3.org/2000/10/swap/log#";
/// The SWAP `string:` namespace. Wholly EXCLUDED: no in-circuit UTF-8
/// reasoning.
pub const STRING_NS: &str = "http://www.w3.org/2000/10/swap/string#";
/// The SWAP `list:` namespace. Wholly EXCLUDED: list generators have no
/// bounded in-circuit shape.
pub const LIST_NS: &str = "http://www.w3.org/2000/10/swap/list#";
/// The SWAP `time:` namespace. Wholly EXCLUDED: calendar decomposition is not
/// an in-circuit gadget.
pub const TIME_NS: &str = "http://www.w3.org/2000/10/swap/time#";

/// One slot of a rule's triple pattern. Mirrors the circuit's
/// `(kind, konst, var)` witness triple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum N3Slot {
    /// A ground term, carried as its salt-independent term encoding.
    Const(FieldHex),
    /// A universally-quantified variable, by index into the firing's
    /// substitution. Two slots carrying the same index necessarily resolve to
    /// the same term — that is where "consistent substitution" comes from.
    Var(u32),
    /// A fresh blank node the rule would MINT in its conclusion (an
    /// existential). **EXCLUDED** from the provable subset: there is no
    /// in-circuit fresh-bnode minting, so a rule carrying one is rejected
    /// rather than approximated. It has no term encoding and therefore cannot
    /// be committed.
    Existential,
}

impl N3Slot {
    /// The circuit's `SLOT_*` kind code (`0` const, `1` var). `None` for an
    /// [`N3Slot::Existential`], which has no in-circuit representation at all.
    fn kind(&self) -> Option<u32> {
        match self {
            N3Slot::Const(_) => Some(0),
            N3Slot::Var(_) => Some(1),
            N3Slot::Existential => None,
        }
    }

    /// The variable index this slot binds/reads, if it is a variable.
    fn var(&self) -> Option<u32> {
        match self {
            N3Slot::Var(v) => Some(*v),
            _ => None,
        }
    }
}

/// A premise atom's builtin tag. [`N3Builtin::None`] marks an ordinary JOIN
/// atom (matched against the graph); every other variant is a whitelisted
/// `math:` gadget. Values mirror the circuit's `BUILTIN_*` globals exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum N3Builtin {
    /// An ordinary join atom; its predicate is the atom's slot 1.
    None,
    /// `?a math:greaterThan ?b`.
    GreaterThan,
    /// `?a math:lessThan ?b`.
    LessThan,
    /// `?a math:notGreaterThan ?b` (`<=`).
    NotGreaterThan,
    /// `?a math:notLessThan ?b` (`>=`).
    NotLessThan,
    /// `?a math:equalTo ?b`.
    EqualTo,
    /// `(?a ?b) math:sum ?c`.
    Sum,
    /// `(?a ?b) math:difference ?c`.
    Difference,
    /// `(?a ?b) math:product ?c`.
    Product,
}

impl N3Builtin {
    /// The circuit's `BUILTIN_*` tag value.
    pub fn tag(self) -> u32 {
        match self {
            N3Builtin::None => 0,
            N3Builtin::GreaterThan => 1,
            N3Builtin::LessThan => 2,
            N3Builtin::NotGreaterThan => 3,
            N3Builtin::NotLessThan => 4,
            N3Builtin::EqualTo => 5,
            N3Builtin::Sum => 6,
            N3Builtin::Difference => 7,
            N3Builtin::Product => 8,
        }
    }

    /// Whether this is a binary arithmetic gadget, whose slot 2 is an OUTPUT
    /// the gadget determines (and therefore BINDS) rather than an input.
    pub fn is_arithmetic(self) -> bool {
        matches!(self, N3Builtin::Sum | N3Builtin::Difference | N3Builtin::Product)
    }

    /// Whether this is a comparison gadget: both operands are inputs and
    /// nothing is bound.
    pub fn is_comparison(self) -> bool {
        !matches!(self, N3Builtin::None) && !self.is_arithmetic()
    }

    /// The slot indices this builtin READS. A comparison reads slots 0 and 2;
    /// arithmetic reads slots 0 and 1 (the list elements) and writes slot 2.
    ///
    /// Slot 1 of a COMPARISON carries nothing: a builtin's predicate is the tag,
    /// so slot 1 is free, and arithmetic reuses it for the second list element.
    fn input_slots(self) -> &'static [usize] {
        if self.is_arithmetic() {
            &[0, 1]
        } else if self.is_comparison() {
            &[0, 2]
        } else {
            &[]
        }
    }

    /// **The fail-closed builtin gate.** Classify a premise atom's predicate
    /// IRI: a whitelisted `math:` member yields its gadget, an ordinary
    /// (non-builtin-namespace) predicate yields [`N3Builtin::None`], and
    /// anything in a builtin namespace that is NOT whitelisted is REJECTED.
    ///
    /// Rejecting rather than treating an unknown `math:`/`log:`/`string:` term
    /// as an ordinary join predicate is the whole point: an unrecognised
    /// builtin would otherwise be silently matched against the graph as data,
    /// which is not what the rule means — the design's "must be rejected, not
    /// silently approximated".
    pub fn classify(predicate_iri: &str) -> Result<N3Builtin, N3SubsetError> {
        if let Some(local) = predicate_iri.strip_prefix(MATH_NS) {
            return match local {
                "greaterThan" => Ok(N3Builtin::GreaterThan),
                "lessThan" => Ok(N3Builtin::LessThan),
                "notGreaterThan" => Ok(N3Builtin::NotGreaterThan),
                "notLessThan" => Ok(N3Builtin::NotLessThan),
                "equalTo" => Ok(N3Builtin::EqualTo),
                "sum" => Ok(N3Builtin::Sum),
                "difference" => Ok(N3Builtin::Difference),
                "product" => Ok(N3Builtin::Product),
                // math:quotient / exponentiation / the float-valued members are
                // deliberately absent.
                _ => Err(N3SubsetError::BuiltinNotWhitelisted {
                    predicate: predicate_iri.to_string(),
                }),
            };
        }
        for ns in [LOG_NS, STRING_NS, LIST_NS, TIME_NS] {
            if predicate_iri.starts_with(ns) {
                return Err(N3SubsetError::BuiltinNotWhitelisted {
                    predicate: predicate_iri.to_string(),
                });
            }
        }
        Ok(N3Builtin::None)
    }
}

/// One premise atom: a builtin tag plus three pattern slots.
///
/// For a JOIN atom (`builtin == N3Builtin::None`) the slots are the ordinary
/// `(subject, predicate, object)`. For a BUILTIN atom the predicate is carried
/// by the tag, so slot 1 holds no predicate: a comparison leaves it unused, and
/// binary arithmetic REUSES it as the second list element of `(?a ?b) math:op
/// ?c`. It is still committed like every other slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct N3Premise {
    /// The whitelisted gadget, or [`N3Builtin::None`] for a join atom.
    pub builtin: N3Builtin,
    /// The three pattern slots.
    pub slots: [N3Slot; 3],
}

impl N3Premise {
    /// An ordinary join atom, matched against the derivation DAG (and so,
    /// transitively, against the committed graphs).
    pub fn join(slots: [N3Slot; 3]) -> Self {
        N3Premise { builtin: N3Builtin::None, slots }
    }

    /// A builtin atom, classified through the fail-closed
    /// [`N3Builtin::classify`] gate. Prefer this over constructing the struct
    /// literally: it is what refuses an unwhitelisted builtin at the boundary.
    pub fn builtin(predicate_iri: &str, slots: [N3Slot; 3]) -> Result<Self, N3SubsetError> {
        Ok(N3Premise { builtin: N3Builtin::classify(predicate_iri)?, slots })
    }
}

/// One dataset-supplied N3 rule: `{premises} => {conclusions}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct N3Rule {
    /// The premise atoms, in the order `sparq-reason` evaluates them (a builtin
    /// after the atoms producing its inputs). The binding schedule depends on
    /// this order.
    pub premises: Vec<N3Premise>,
    /// The conclusion atoms. A derivation step derives ONE of them, selected
    /// per firing.
    pub conclusions: Vec<[N3Slot; 3]>,
}

/// The dataset's rule graph — what the rule author signs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct N3RuleSet {
    /// The rules, in the order they are committed (the leaf order of the root).
    pub rules: Vec<N3Rule>,
}

/// Why a rule (or rule graph) is outside the declared provable subset. Each
/// variant mirrors one in-circuit assert of `n3.nr`'s `rule_subset_check` /
/// `n3_derivation_check`, so the host gate and the relation cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum N3SubsetError {
    /// The rule graph is empty, so it entails nothing and proves nothing.
    EmptyRuleSet,
    /// A rule with no premise would fire unconditionally.
    EmptyPremise { rule: usize },
    /// A rule with no conclusion derives nothing.
    EmptyConclusion { rule: usize },
    /// A predicate in a builtin namespace that v1 does not implement as a
    /// gadget (`math:quotient`, `log:notIncludes`, `string:concat`, …). Treating
    /// it as an ordinary join predicate would silently change the rule's
    /// meaning, so it is refused.
    BuiltinNotWhitelisted { predicate: String },
    /// A rule mints a fresh blank node (an existential). There is no in-circuit
    /// fresh-bnode minting, so the rule is refused rather than approximated.
    ExistentialInConclusion { rule: usize, atom: usize },
    /// A builtin reads a variable no earlier premise atom bound — the rule is
    /// not evaluable "when ready" and the gadget would read an unconstrained
    /// witness.
    BuiltinInputNotBound { rule: usize, atom: usize, variable: u32 },
    /// **The safety / range-restriction violation.** A conclusion variable is
    /// not bound by the premise, so the prover could CHOOSE its value and
    /// derive an arbitrary triple. This is the condition that makes the subset
    /// sound, not merely tidy.
    UnboundConclusionVariable { rule: usize, atom: usize, variable: u32 },
    /// The rule has more premise atoms than the circuit member's `P` bucket.
    PremiseBucketOverflow { rule: usize, atoms: usize, bucket: usize },
    /// The rule has more conclusion atoms than the circuit member's `C` bucket.
    ConclusionBucketOverflow { rule: usize, atoms: usize, bucket: usize },
    /// A constant slot's term encoding is not a well-formed field element.
    MalformedTermEncoding { rule: usize, atom: usize, slot: usize },
}

impl std::fmt::Display for N3SubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            N3SubsetError::EmptyRuleSet => {
                write!(f, "the N3 rule graph is empty, so it entails nothing")
            }
            N3SubsetError::EmptyPremise { rule } => {
                write!(f, "N3 rule {rule} has an empty premise (it would fire unconditionally)")
            }
            N3SubsetError::EmptyConclusion { rule } => {
                write!(f, "N3 rule {rule} has an empty conclusion (it derives nothing)")
            }
            N3SubsetError::BuiltinNotWhitelisted { predicate } => write!(
                f,
                "builtin <{predicate}> is outside the provable subset (sq-rsd3v.3: an unimplemented builtin is rejected, never treated as an ordinary predicate)"
            ),
            N3SubsetError::ExistentialInConclusion { rule, atom } => write!(
                f,
                "N3 rule {rule} conclusion atom {atom} mints a fresh blank node; in-circuit existential introduction is not supported (sq-rsd3v.3)"
            ),
            N3SubsetError::BuiltinInputNotBound { rule, atom, variable } => write!(
                f,
                "N3 rule {rule} premise atom {atom} reads variable ?{variable} before any earlier atom binds it"
            ),
            N3SubsetError::UnboundConclusionVariable { rule, atom, variable } => write!(
                f,
                "N3 rule {rule} is unsafe: conclusion atom {atom} uses variable ?{variable}, which the premise never binds (the prover could then choose the derived triple)"
            ),
            N3SubsetError::PremiseBucketOverflow { rule, atoms, bucket } => write!(
                f,
                "N3 rule {rule} has {atoms} premise atoms, exceeding the circuit member's bucket of {bucket}"
            ),
            N3SubsetError::ConclusionBucketOverflow { rule, atoms, bucket } => write!(
                f,
                "N3 rule {rule} has {atoms} conclusion atoms, exceeding the circuit member's bucket of {bucket}"
            ),
            N3SubsetError::MalformedTermEncoding { rule, atom, slot } => write!(
                f,
                "N3 rule {rule} atom {atom} slot {slot} carries a malformed term encoding"
            ),
        }
    }
}

impl std::error::Error for N3SubsetError {}

impl N3Rule {
    /// Admit this rule into the PROVABLE SUBSET, or say precisely why not.
    ///
    /// Runs the same BINDING SCHEDULE the circuit runs (`n3.nr`'s
    /// `rule_subset_check`): premise atoms in order; a join atom binds every
    /// variable slot it carries; an arithmetic builtin requires its input
    /// variables already bound and then binds its output; a comparison binds
    /// nothing and requires both operands bound. Finally every conclusion
    /// variable must be bound — the SAFETY condition, without which the private
    /// substitution lets a prover derive anything.
    ///
    /// `rule` is only the index used in error messages.
    pub fn admit(&self, rule: usize) -> Result<(), N3SubsetError> {
        if self.premises.is_empty() {
            return Err(N3SubsetError::EmptyPremise { rule });
        }
        if self.conclusions.is_empty() {
            return Err(N3SubsetError::EmptyConclusion { rule });
        }

        let mut bound: BTreeSet<u32> = BTreeSet::new();
        for (atom, premise) in self.premises.iter().enumerate() {
            for slot in &premise.slots {
                if matches!(slot, N3Slot::Existential) {
                    return Err(N3SubsetError::ExistentialInConclusion { rule, atom });
                }
            }

            // A builtin's inputs must already be bound ("when ready").
            for &s in premise.builtin.input_slots() {
                if let Some(v) = premise.slots[s].var() {
                    if !bound.contains(&v) {
                        return Err(N3SubsetError::BuiltinInputNotBound {
                            rule,
                            atom,
                            variable: v,
                        });
                    }
                }
            }

            match premise.builtin {
                // A join atom binds every variable it carries.
                N3Builtin::None => {
                    for slot in &premise.slots {
                        if let Some(v) = slot.var() {
                            bound.insert(v);
                        }
                    }
                }
                // Arithmetic determines — and therefore binds — its output.
                b if b.is_arithmetic() => {
                    if let Some(v) = premise.slots[2].var() {
                        bound.insert(v);
                    }
                }
                // A comparison binds nothing.
                _ => {}
            }
        }

        for (atom, conclusion) in self.conclusions.iter().enumerate() {
            for slot in conclusion {
                match slot {
                    N3Slot::Existential => {
                        return Err(N3SubsetError::ExistentialInConclusion { rule, atom })
                    }
                    N3Slot::Var(v) if !bound.contains(v) => {
                        return Err(N3SubsetError::UnboundConclusionVariable {
                            rule,
                            atom,
                            variable: *v,
                        })
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The canonical commitment leaf of this rule shape, folded EXACTLY as the
    /// circuit's `rule_leaf` folds it: `h2(prem_len, concl_len)`, then per
    /// premise atom `h2(acc, builtin_tag)` followed by `h4(acc, kind, konst,
    /// var)` per slot, then the same per conclusion slot.
    ///
    /// Padding up to the member's `P` / `C` buckets is folded as
    /// `(tag 0, kind 0, konst 0, var 0)` so a rule's leaf is canonical for a
    /// given bucket pair. Because EVERY slot enters the fold, a prover who edits
    /// any slot, retags a builtin or shortens a premise produces a different
    /// leaf and fails the circuit's rule-set recompute.
    ///
    /// Requires an ADMITTED rule: an [`N3Slot::Existential`] has no encoding, so
    /// this returns [`N3SubsetError::ExistentialInConclusion`] rather than
    /// inventing one.
    pub fn leaf(
        &self,
        rule: usize,
        premise_bucket: usize,
        conclusion_bucket: usize,
    ) -> Result<Fr, N3SubsetError> {
        if self.premises.len() > premise_bucket {
            return Err(N3SubsetError::PremiseBucketOverflow {
                rule,
                atoms: self.premises.len(),
                bucket: premise_bucket,
            });
        }
        if self.conclusions.len() > conclusion_bucket {
            return Err(N3SubsetError::ConclusionBucketOverflow {
                rule,
                atoms: self.conclusions.len(),
                bucket: conclusion_bucket,
            });
        }

        let mut acc = h2(
            Fr::from(self.premises.len() as u64),
            Fr::from(self.conclusions.len() as u64),
        );
        for atom in 0..premise_bucket {
            let premise = self.premises.get(atom);
            let tag = premise.map_or(0, |p| p.builtin.tag());
            acc = h2(acc, Fr::from(tag as u64));
            for slot in 0..3 {
                let (kind, konst, var) = match premise.map(|p| &p.slots[slot]) {
                    None => (0, Fr::from(0u64), 0),
                    Some(s) => slot_fields(s, rule, atom, slot)?,
                };
                acc = h4(acc, Fr::from(kind as u64), konst, Fr::from(var as u64));
            }
        }
        for atom in 0..conclusion_bucket {
            let conclusion = self.conclusions.get(atom);
            for slot in 0..3 {
                let (kind, konst, var) = match conclusion.map(|c| &c[slot]) {
                    None => (0, Fr::from(0u64), 0),
                    Some(s) => slot_fields(s, rule, atom, slot)?,
                };
                acc = h4(acc, Fr::from(kind as u64), konst, Fr::from(var as u64));
            }
        }
        Ok(acc)
    }
}

/// The circuit's `(kind, konst, var)` witness triple for one slot.
fn slot_fields(
    slot: &N3Slot,
    rule: usize,
    atom: usize,
    index: usize,
) -> Result<(u32, Fr, u32), N3SubsetError> {
    let kind = slot
        .kind()
        .ok_or(N3SubsetError::ExistentialInConclusion { rule, atom })?;
    let konst = match slot {
        N3Slot::Const(hex) => hex
            .to_field()
            .ok_or(N3SubsetError::MalformedTermEncoding { rule, atom, slot: index })?,
        _ => Fr::from(0u64),
    };
    Ok((kind, konst, slot.var().unwrap_or(0)))
}

impl N3RuleSet {
    /// Admit the WHOLE rule graph into the provable subset. Fail-closed: the
    /// first out-of-subset rule wins, and nothing is approximated.
    pub fn admit(&self) -> Result<(), N3SubsetError> {
        if self.rules.is_empty() {
            return Err(N3SubsetError::EmptyRuleSet);
        }
        for (i, rule) in self.rules.iter().enumerate() {
            rule.admit(i)?;
        }
        Ok(())
    }

    /// **The rule author as issuer.** Commit the rule graph to the `rules_root`
    /// the circuit takes as a PUBLIC input (and which the author then signs
    /// with the existing `sq-z9l` issuer machinery — this produces the root,
    /// not the signature).
    ///
    /// [`N3RuleSet::admit`] runs FIRST, so **an out-of-subset rule graph has no
    /// root**: it cannot be committed, and therefore cannot reach the circuit
    /// at all. That is the design's "a rule outside the subset MUST be
    /// rejected, not silently approximated", enforced structurally rather than
    /// by convention.
    ///
    /// `premise_bucket` / `conclusion_bucket` are the target circuit member's
    /// `P` / `C` parameters; the root is only meaningful for that member,
    /// exactly as a graph commitment is only meaningful for its `(k, n)`
    /// bucket.
    pub fn commit(
        &self,
        premise_bucket: usize,
        conclusion_bucket: usize,
    ) -> Result<Fr, N3SubsetError> {
        self.admit()?;
        let leaves = self
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| r.leaf(i, premise_bucket, conclusion_bucket))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(poseidon2::hash(&leaves))
    }

    /// The number of distinct variable slots the rules use, i.e. the smallest
    /// `V` bucket a circuit member could carry. `0` for a rule set with no
    /// variables.
    pub fn var_count(&self) -> u32 {
        let mut max: Option<u32> = None;
        let mut note = |slot: &N3Slot| {
            if let Some(v) = slot.var() {
                max = Some(max.map_or(v, |m: u32| m.max(v)));
            }
        };
        for rule in &self.rules {
            for premise in &rule.premises {
                for slot in &premise.slots {
                    note(slot);
                }
            }
            for conclusion in &rule.conclusions {
                for slot in conclusion {
                    note(slot);
                }
            }
        }
        max.map_or(0, |m| m + 1)
    }
}

/// Two-input Poseidon2 — bit-identical to the circuit's `h2`.
fn h2(a: Fr, b: Fr) -> Fr {
    poseidon2::hash(&[a, b])
}

/// Four-input Poseidon2 — bit-identical to the circuit's `h4`.
fn h4(a: Fr, b: Fr, c: Fr, d: Fr) -> Fr {
    poseidon2::hash(&[a, b, c, d])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    fn konst(s: &str) -> N3Slot {
        N3Slot::Const(fh(s))
    }

    const WORKS_AT: &str = "0x71";
    const LOCATED_IN: &str = "0x72";
    const BASED_IN: &str = "0x73";
    const SALARY: &str = "0x74";
    const HIGH_EARNER: &str = "0x75";
    const YES: &str = "0x76";
    const TWENTY: &str = "0x20";

    /// `{ ?x worksAt ?c . ?c locatedIn ?r } => { ?x basedIn ?r }` — the safe,
    /// builtin-free base case. Variables: ?x = 0, ?c = 1, ?r = 2.
    fn join_rule() -> N3Rule {
        N3Rule {
            premises: vec![
                N3Premise::join([N3Slot::Var(0), konst(WORKS_AT), N3Slot::Var(1)]),
                N3Premise::join([N3Slot::Var(1), konst(LOCATED_IN), N3Slot::Var(2)]),
            ],
            conclusions: vec![[N3Slot::Var(0), konst(BASED_IN), N3Slot::Var(2)]],
        }
    }

    /// `{ ?x salary ?s . ?s math:greaterThan 20 } => { ?x highEarner yes }`.
    fn comparison_rule() -> N3Rule {
        N3Rule {
            premises: vec![
                N3Premise::join([N3Slot::Var(0), konst(SALARY), N3Slot::Var(1)]),
                N3Premise::builtin(
                    &format!("{MATH_NS}greaterThan"),
                    [N3Slot::Var(1), konst("0x0"), konst(TWENTY)],
                )
                .expect("math:greaterThan is whitelisted"),
            ],
            conclusions: vec![[N3Slot::Var(0), konst(HIGH_EARNER), konst(YES)]],
        }
    }

    #[test]
    fn classify_admits_every_whitelisted_math_builtin() {
        for (local, want) in [
            ("greaterThan", N3Builtin::GreaterThan),
            ("lessThan", N3Builtin::LessThan),
            ("notGreaterThan", N3Builtin::NotGreaterThan),
            ("notLessThan", N3Builtin::NotLessThan),
            ("equalTo", N3Builtin::EqualTo),
            ("sum", N3Builtin::Sum),
            ("difference", N3Builtin::Difference),
            ("product", N3Builtin::Product),
        ] {
            let iri = format!("{MATH_NS}{local}");
            assert_eq!(N3Builtin::classify(&iri), Ok(want), "math:{local}");
        }
        // An ordinary dataset predicate is a JOIN atom, not a builtin.
        assert_eq!(
            N3Builtin::classify("http://example.org/worksAt"),
            Ok(N3Builtin::None)
        );
    }

    // The fail-closed edge of the declared subset: every EXCLUDED construct the
    // design record names must be REJECTED, never silently matched as data.
    #[test]
    fn classify_rejects_every_excluded_builtin() {
        for iri in [
            // math: members with no in-circuit gadget.
            "http://www.w3.org/2000/10/swap/math#quotient",
            "http://www.w3.org/2000/10/swap/math#exponentiation",
            // Negation-as-failure: a NEGATIVE over the closure (sq-rsd3v.7).
            "http://www.w3.org/2000/10/swap/log#notIncludes",
            "http://www.w3.org/2000/10/swap/log#collectAllIn",
            // Reaching outside the committed graph.
            "http://www.w3.org/2000/10/swap/log#semantics",
            "http://www.w3.org/2000/10/swap/log#includes",
            // No in-circuit UTF-8 reasoning / list generators / calendar maths.
            "http://www.w3.org/2000/10/swap/string#concatenation",
            "http://www.w3.org/2000/10/swap/list#member",
            "http://www.w3.org/2000/10/swap/time#day",
        ] {
            assert_eq!(
                N3Builtin::classify(iri),
                Err(N3SubsetError::BuiltinNotWhitelisted { predicate: iri.to_string() }),
                "{iri} must be refused, not approximated"
            );
        }
    }

    #[test]
    fn admit_accepts_the_safe_rules() {
        join_rule().admit(0).expect("a safe join rule is in the subset");
        comparison_rule().admit(0).expect("a whitelisted comparison is in the subset");
    }

    // An arithmetic builtin BINDS its output, so a conclusion may legitimately
    // use it — the binding schedule has to model that, not just "bound by a
    // join atom". `{ ?x salary ?s . (?s 20) math:sum ?t } => { ?x raised ?t }`.
    #[test]
    fn admit_accepts_a_conclusion_variable_bound_by_arithmetic() {
        let rule = N3Rule {
            premises: vec![
                N3Premise::join([N3Slot::Var(0), konst(SALARY), N3Slot::Var(1)]),
                N3Premise::builtin(
                    &format!("{MATH_NS}sum"),
                    [N3Slot::Var(1), konst(TWENTY), N3Slot::Var(2)],
                )
                .expect("math:sum is whitelisted"),
            ],
            conclusions: vec![[N3Slot::Var(0), konst(BASED_IN), N3Slot::Var(2)]],
        };
        rule.admit(0).expect("math:sum binds its output variable");
    }

    // THE safety regression: an unbound conclusion variable lets the prover
    // choose the derived triple, because the substitution is a private witness.
    #[test]
    fn admit_rejects_an_unbound_conclusion_variable() {
        let mut rule = join_rule();
        rule.premises.truncate(1); // drops the atom that bound ?r
        assert_eq!(
            rule.admit(3),
            Err(N3SubsetError::UnboundConclusionVariable { rule: 3, atom: 0, variable: 2 })
        );
    }

    #[test]
    fn admit_rejects_existential_introduction() {
        let mut rule = join_rule();
        rule.conclusions = vec![[N3Slot::Var(0), konst(BASED_IN), N3Slot::Existential]];
        assert_eq!(
            rule.admit(1),
            Err(N3SubsetError::ExistentialInConclusion { rule: 1, atom: 0 })
        );
    }

    // A builtin whose input no earlier atom bound would read an unconstrained
    // witness; the "when ready" ordering is what rules that out.
    #[test]
    fn admit_rejects_a_builtin_reading_an_unbound_input() {
        let mut rule = comparison_rule();
        rule.premises.swap(0, 1); // the comparison now runs BEFORE ?s is bound
        assert_eq!(
            rule.admit(0),
            Err(N3SubsetError::BuiltinInputNotBound { rule: 0, atom: 0, variable: 1 })
        );
    }

    #[test]
    fn admit_rejects_degenerate_rules() {
        let empty_premise = N3Rule { premises: vec![], conclusions: vec![] };
        assert_eq!(empty_premise.admit(0), Err(N3SubsetError::EmptyPremise { rule: 0 }));
        let empty_conclusion =
            N3Rule { premises: join_rule().premises, conclusions: vec![] };
        assert_eq!(empty_conclusion.admit(2), Err(N3SubsetError::EmptyConclusion { rule: 2 }));
        assert_eq!(N3RuleSet::default().admit(), Err(N3SubsetError::EmptyRuleSet));
    }

    // The structural fail-closed property this bead is about: an out-of-subset
    // rule graph has NO root, so it can never reach the circuit's `rules_root`.
    #[test]
    fn commit_refuses_a_rule_set_outside_the_subset() {
        let mut unsafe_rule = join_rule();
        unsafe_rule.premises.truncate(1);
        let set = N3RuleSet { rules: vec![join_rule(), unsafe_rule] };
        assert_eq!(
            set.commit(2, 1),
            Err(N3SubsetError::UnboundConclusionVariable { rule: 1, atom: 0, variable: 2 })
        );
    }

    #[test]
    fn commit_is_deterministic_and_bucket_dependent() {
        let set = N3RuleSet { rules: vec![join_rule(), comparison_rule()] };
        let root = set.commit(2, 1).expect("the rules are in the subset");
        assert_eq!(root, set.commit(2, 1).unwrap(), "the fold must be deterministic");
        // The root is only meaningful for ONE member's buckets, exactly like a
        // graph commitment and its (k, n) bucket.
        assert_ne!(root, set.commit(3, 1).unwrap(), "padding enters the fold");
    }

    // The anti-forgery property the circuit's rule-set recompute relies on:
    // editing ANY committed slot must move the root.
    #[test]
    fn commit_changes_when_any_slot_is_edited() {
        let honest = N3RuleSet { rules: vec![join_rule(), comparison_rule()] };
        let root = honest.commit(2, 1).unwrap();

        // (a) a different conclusion predicate
        let mut tampered = honest.clone();
        tampered.rules[0].conclusions[0][1] = konst(WORKS_AT);
        assert_ne!(root, tampered.commit(2, 1).unwrap());

        // (b) a different variable index (same shape, different sharing)
        let mut tampered = honest.clone();
        tampered.rules[0].premises[1].slots[0] = N3Slot::Var(0);
        assert_ne!(root, tampered.commit(2, 1).unwrap());

        // (c) a retagged builtin — greaterThan becomes lessThan
        let mut tampered = honest.clone();
        tampered.rules[1].premises[1].builtin = N3Builtin::LessThan;
        assert_ne!(root, tampered.commit(2, 1).unwrap());

        // (d) rule ORDER (the leaf order is part of the commitment)
        let swapped = N3RuleSet { rules: vec![comparison_rule(), join_rule()] };
        assert_ne!(root, swapped.commit(2, 1).unwrap());
    }

    #[test]
    fn builtin_slot_roles_match_the_circuit() {
        // A comparison reads slots 0 and 2; slot 1 carries nothing (the
        // predicate is the tag).
        assert_eq!(N3Builtin::GreaterThan.input_slots(), &[0, 2]);
        // Arithmetic reads the two list elements and WRITES slot 2.
        assert_eq!(N3Builtin::Sum.input_slots(), &[0, 1]);
        assert!(N3Builtin::Sum.is_arithmetic());
        assert!(N3Builtin::EqualTo.is_comparison());
        assert!(!N3Builtin::None.is_comparison());
        // Tags mirror the circuit's BUILTIN_* globals.
        assert_eq!(N3Builtin::None.tag(), 0);
        assert_eq!(N3Builtin::Product.tag(), 8);
    }

    #[test]
    fn var_count_reports_the_bucket_the_rules_need() {
        assert_eq!(N3RuleSet { rules: vec![join_rule()] }.var_count(), 3);
        assert_eq!(N3RuleSet { rules: vec![comparison_rule()] }.var_count(), 2);
        assert_eq!(N3RuleSet::default().var_count(), 0);
    }

    #[test]
    fn leaf_refuses_a_rule_that_overflows_the_member_bucket() {
        let rule = join_rule();
        assert_eq!(
            rule.leaf(0, 1, 1),
            Err(N3SubsetError::PremiseBucketOverflow { rule: 0, atoms: 2, bucket: 1 })
        );
        assert_eq!(
            rule.leaf(0, 2, 0),
            Err(N3SubsetError::ConclusionBucketOverflow { rule: 0, atoms: 1, bucket: 0 })
        );
    }
}

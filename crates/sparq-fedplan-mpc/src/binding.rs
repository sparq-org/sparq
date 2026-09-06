//! **Phase 5 (sq-1fo4) — the untrusted-plan binding + soundness re-validation seam.**
//!
//! Design record `research/mpc-untrusted-planner-routing-design.md` §8 Phase 5, discharging
//! constraint **C-A** (§2.2): *the plan is UNTRUSTED for soundness* — a planner that drops a
//! source, mis-orders a join or mis-routes an operator must at worst yield a
//! **wrong-but-rejected** answer, never a **forged-accepted** one.
//!
//! # What this module delivers, and what it deliberately does NOT
//!
//! C-A is discharged, per §4.4, by binding the produced plan to the **eventual collaborative
//! proof**, so that a dropped or mis-routed source makes the proof *fail to verify*. That proof
//! does not exist: [`sparq_mpc::proof::CollaborativeProof`] is an honest
//! `MpcError::NotYetImplemented` stub, and the external accredited-cryptographer audit
//! (`sq-qhy4`) plus the collaborative-coZK re-audit (`sq-9hrn`) are both pending. **So the
//! soundness half of Phase 5 is NOT delivered here and this module makes NO soundness claim.**
//!
//! What *is* deliverable ahead of those gates — and is all this module does — is the
//! **canonicalisation** half, exactly mirroring the precedent `sparq-mpc`'s
//! [`federated_binding`](sparq_mpc::federated_binding) set for the M4-v1 freshness binding
//! (`sq-34ml`: *scoping + layout only, still hard-gated on the ZK foundation*):
//!
//! 1. [`commit_plan`] reduces the produced plan — the Phase-2 per-pattern source assignment, the
//!    Phase-3 per-operator disclosed/hidden route, and the Phase-4 declared participation set —
//!    to a deterministic, domain-separated [`PlanCommitment`] whose [`PlanCommitment::plan_digest`]
//!    is a single [`FieldWord`], i.e. a value shaped to be carried verbatim as a **public input**
//!    once a collaborative proof exists to carry it.
//! 2. [`revalidate_plan`] compares a *claimed* commitment against an *independently recomputed*
//!    one and **fails closed**, reporting a structured [`PlanDivergence`] list that names the
//!    dropped source / re-routed operator rather than a bare digest mismatch.
//! 3. [`bind_plan_to_proof`] — the step that would actually discharge C-A — carries its real
//!    signature and returns [`SeamError::Deferred`], naming every gate it waits on. It fabricates
//!    nothing and never panics.
//!
//! # The honest limit of the out-of-circuit check (read this before relying on it)
//!
//! [`revalidate_plan`] is a **transcript-integrity and canonicality** check, **not** a soundness
//! mechanism, and on its own it defends against **nothing a malicious planner does**. The planner
//! computes the plan *and* its commitment, so a planner that drops a source can simply commit to
//! the plan it actually ran; the digests then agree and the check passes. It catches *accidental*
//! divergence — a plan mutated between ratification and execution, a verifier and planner that
//! disagree about what was selected, a transport that reordered or truncated the plan — which is
//! genuinely useful, but it is the *weak* half of C-A.
//!
//! The check becomes load-bearing **only** when [`PlanCommitment::plan_digest`] is a public input
//! the collaborative proof is computed over, so that the verifier recomputes the commitment from
//! *independently attested* sources and a divergence is a **proof-verification failure** rather
//! than a self-consistent story. That is [`bind_plan_to_proof`], and it is deferred. Until it
//! lands **and** the audits clear, do NOT present anything here as providing soundness.
//!
//! Where the binding ultimately attaches is design-record **§9 Q3, still OPEN**. The
//! [`bind_plan_to_proof`] signature names [`sparq_mpc::FederatedStatement`] as the *proposed*
//! attachment point (its public-input layout already carries the analogous `query_digest` and
//! `key_set_digest` words) — a proposal for the maintainer, not a decision, and it may change
//! when the proof's shape is known.
//!
//! # Leakage
//!
//! A [`PlanCommitment`] carries **plan metadata only** — source ids, operator labels, operator
//! classes and disclosed predicate ids — every one of which the Phase-4
//! [`LeakageEnvelope`] already declares as disclosed by the plan. It
//! carries **no** operand values, **no** result rows and **no** cardinalities, and it runs no MPC.
//! It therefore widens the declared leakage envelope by nothing.
//!
//! `[OPUS-5]` sq-1fo4 — Phase 5, canonicalisation half only; the soundness half stays audit-gated.

use std::collections::BTreeSet;

use sparq_mpc::backend::OperatorClass;
use sparq_mpc::pipeline::Routing;
use sparq_mpc::{FederatedStatement, FieldWord};

use crate::seam::{LeakageEnvelope, PrivateRouting, SeamError, SeamPhase};
use crate::selection::SelectedPrivateSources;

/// Domain tag for the overall plan digest ([`PlanCommitment::plan_digest`]).
///
/// Follows the estate's `sparq-*/purpose/version\0` domain-separation convention (compare
/// [`sparq_mpc::FRESHNESS_DOMAIN_TAG`]), so a plan digest can never collide with a digest drawn
/// for another purpose under the same hash.
pub const PLAN_DOMAIN_TAG: &[u8] = b"sparq-fedplan-mpc/plan-binding/v1\0";
/// Domain tag for the Phase-2 per-pattern source-assignment digest
/// ([`PlanCommitment::selection_digest`]).
pub const SELECTION_DOMAIN_TAG: &[u8] = b"sparq-fedplan-mpc/plan-binding/selection/v1\0";
/// Domain tag for the Phase-3 per-operator routing digest ([`PlanCommitment::routing_digest`]).
pub const ROUTING_DOMAIN_TAG: &[u8] = b"sparq-fedplan-mpc/plan-binding/routing/v1\0";
/// Domain tag for the Phase-4 participation-set digest ([`PlanCommitment::participation_digest`]).
pub const PARTICIPATION_DOMAIN_TAG: &[u8] = b"sparq-fedplan-mpc/plan-binding/participation/v1\0";

/// The gates Phase 5's soundness half waits on. Carried verbatim in the
/// [`SeamError::Deferred`] returned by [`bind_plan_to_proof`] so the refusal is auditable at the
/// call site rather than only in a doc-comment.
const PLAN_BINDING_GATED_ON: &str = "sparq-mpc::proof::CollaborativeProof (still \
     MpcError::NotYetImplemented) + external accredited-cryptographer audit sq-qhy4 + \
     collaborative-coZK re-audit sq-9hrn; the attachment point is design record §9 Q3, still open";

/// One pattern's committed source assignment: *which sources the plan says answer this pattern*.
///
/// This is the field a **dropped or mis-routed source** perturbs, and the reason the commitment
/// binds the per-pattern assignment rather than only the flat participation set: moving a source
/// from pattern 0 to pattern 1 leaves [`PlanCommitment::participating_sources`] identical while
/// changing the plan's meaning entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommittedPattern {
    /// The BGP pattern index.
    pub pattern: usize,
    /// The source ids assigned to this pattern, ascending and duplicate-free (canonical order —
    /// the commitment must not depend on the planner's enumeration order).
    pub sources: Vec<String>,
}

/// One operator's committed route: the Phase-3 disclosed/hidden decision plus what a disclosed
/// route reveals. This is the field a **mis-routed operator** perturbs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommittedOperator {
    /// The operator's label, verbatim from the routing.
    pub operator: String,
    /// The operator's [`OperatorClass`] — part of the always-leaking operator structure.
    pub class: OperatorClass,
    /// `true` if routed [`Routing::Disclosed`] (recomputed in the clear), `false` if
    /// [`Routing::Hidden`] (run under MPC).
    pub disclosed: bool,
    /// For a disclosed operator, the predicate ids it reveals, ascending and duplicate-free.
    /// Empty for a hidden operator.
    pub disclosed_operands: Vec<String>,
}

/// A deterministic, domain-separated commitment to the plan an **untrusted** planner produced.
///
/// Built only by [`commit_plan`]. The digests are shaped as [`FieldWord`]s — the estate's
/// public-input word — so [`plan_digest`](Self::plan_digest) can be carried verbatim into a proof
/// statement once one exists. **The commitment by itself provides no soundness**; see the module
/// docs for the honest limit.
///
/// # What is deliberately NOT committed
///
/// The Phase-2 `estimated_cardinality` is excluded. It is a *cost estimate*, not plan semantics:
/// it changes whenever the cost model is retuned (which would spuriously invalidate an otherwise
/// identical plan), and `f64` has no canonical byte encoding that is stable across `NaN` payloads
/// and signed zero. Binding it would make the digest brittle without binding anything a planner
/// could cheat with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanCommitment {
    /// The committed per-pattern source assignment, ascending by pattern index.
    pub patterns: Vec<CommittedPattern>,
    /// The committed per-operator route, in plan (input-operator) order.
    pub operators: Vec<CommittedOperator>,
    /// The distinct participating source ids the Phase-4 envelope declared, ascending.
    pub participating_sources: Vec<String>,
    selection_digest: FieldWord,
    routing_digest: FieldWord,
    participation_digest: FieldWord,
    plan_digest: FieldWord,
}

impl PlanCommitment {
    /// The digest over the Phase-2 per-pattern source assignment.
    pub fn selection_digest(&self) -> FieldWord {
        self.selection_digest
    }

    /// The digest over the Phase-3 per-operator disclosed/hidden route.
    pub fn routing_digest(&self) -> FieldWord {
        self.routing_digest
    }

    /// The digest over the Phase-4 declared participation set.
    pub fn participation_digest(&self) -> FieldWord {
        self.participation_digest
    }

    /// The overall plan digest — the domain-separated fold of the three component digests, and
    /// the single word that would become a public input under [`bind_plan_to_proof`].
    pub fn plan_digest(&self) -> FieldWord {
        self.plan_digest
    }
}

/// One way a re-validated plan diverged from the claimed commitment.
///
/// Reported instead of a bare digest mismatch so the failure is *auditable*: an operator can see
/// **which** source was dropped or **which** operator was re-routed. `#[non_exhaustive]` so a
/// later phase can name a new divergence without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanDivergence {
    /// The two plans commit to a different number of patterns.
    PatternArity {
        /// Pattern count in the claimed commitment.
        claimed: usize,
        /// Pattern count in the recomputed commitment.
        observed: usize,
    },
    /// A source the claimed plan assigned to a pattern is absent from the recomputed plan — the
    /// **dropped-source** case constraint C-A is about.
    SourceDropped {
        /// The pattern index it was dropped from.
        pattern: usize,
        /// The absent source id.
        source_id: String,
    },
    /// A source the recomputed plan assigns to a pattern was absent from the claimed plan.
    SourceAdded {
        /// The pattern index it appears at.
        pattern: usize,
        /// The extra source id.
        source_id: String,
    },
    /// The two plans commit to a different number of operators.
    OperatorArity {
        /// Operator count in the claimed commitment.
        claimed: usize,
        /// Operator count in the recomputed commitment.
        observed: usize,
    },
    /// The operator at this index carries a different label — the plan was re-ordered or the
    /// operator substituted.
    OperatorRelabelled {
        /// The operator index.
        index: usize,
        /// The claimed label.
        claimed: String,
        /// The recomputed label.
        observed: String,
    },
    /// The operator at this index runs as a different [`OperatorClass`].
    OperatorClassChanged {
        /// The operator index.
        index: usize,
        /// The operator label (from the claimed commitment).
        operator: String,
    },
    /// The operator at this index changed side of the disclosed/hidden partition — the
    /// **mis-routed-operator** case.
    OperatorRerouted {
        /// The operator index.
        index: usize,
        /// The operator label (from the claimed commitment).
        operator: String,
        /// Whether the claimed plan routed it disclosed.
        claimed_disclosed: bool,
        /// Whether the recomputed plan routed it disclosed.
        observed_disclosed: bool,
    },
    /// The operator at this index reveals a different operand set in the clear.
    DisclosedOperandsChanged {
        /// The operator index.
        index: usize,
        /// The operator label (from the claimed commitment).
        operator: String,
        /// The claimed disclosed operands.
        claimed: Vec<String>,
        /// The recomputed disclosed operands.
        observed: Vec<String>,
    },
    /// The declared participation set differs.
    ParticipationChanged {
        /// Source ids claimed as participating that the recomputed plan omits.
        dropped: Vec<String>,
        /// Source ids the recomputed plan adds.
        added: Vec<String>,
    },
    /// The plan digests differ but no structured divergence explained it. Emitted so
    /// [`revalidate_plan`] can NEVER report [`PlanRevalidation::Bound`] on a digest mismatch —
    /// the fail-closed catch-all.
    DigestMismatch,
}

/// The outcome of [`revalidate_plan`].
///
/// [`PlanRevalidation::Bound`] means the recomputed plan matched the claimed commitment
/// **byte-for-byte and digest-for-digest** — it does *not* mean the plan is sound (see the module
/// docs).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanRevalidation {
    /// The recomputed plan agrees with the claimed commitment in every committed field and in the
    /// overall digest.
    Bound,
    /// The plans diverge. Every divergence found is listed; the list is never empty.
    Diverged(Vec<PlanDivergence>),
}

impl PlanRevalidation {
    /// `true` only for [`PlanRevalidation::Bound`] — the guard a caller should branch on, so a
    /// divergent plan is never treated as bound by accident.
    pub fn is_bound(&self) -> bool {
        matches!(self, PlanRevalidation::Bound)
    }
}

/// Length-prefix an arity as a big-endian `u64` part.
fn arity(n: usize) -> Vec<u8> {
    (n as u64).to_be_bytes().to_vec()
}

/// A stable byte code per [`OperatorClass`]. Explicit (not `as u8` over the declaration order) so
/// re-ordering the upstream enum cannot silently change every committed plan digest.
fn class_code(class: OperatorClass) -> u8 {
    match class {
        OperatorClass::LinearAggregate => 1,
        OperatorClass::EqualityJoin => 2,
        OperatorClass::Comparison => 3,
    }
}

/// Digest `parts` under `tag`. [`FieldWord::digest`] length-prefixes each part, and every
/// variable-arity run below is preceded by its explicit count, so no two distinct plans share a
/// pre-image by concatenation ambiguity.
fn digest_parts(tag: &[u8], parts: &[Vec<u8>]) -> FieldWord {
    let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
    FieldWord::digest(tag, &refs)
}

/// Commit to the plan an untrusted planner produced.
///
/// Takes the Phase-2 selection, the Phase-3 routing and the Phase-4 leakage envelope — the three
/// artefacts that *are* the plan — and reduces them to a canonical [`PlanCommitment`]. The result
/// is deterministic: it depends on the plan's **content**, never on the planner's enumeration
/// order (per-pattern source lists and disclosed-operand lists are sorted; patterns are ordered by
/// index).
///
/// This performs **no** MPC, opens nothing, verifies nothing cryptographic, and makes **no**
/// soundness or privacy claim — it is a canonical encoding. See the module docs.
///
/// # Errors
///
/// Fails closed with [`SeamError::DescriptorMismatch`] (phase [`SeamPhase::PlanBinding`]) rather
/// than committing to an ambiguous plan, when:
///
/// * the selection's pattern indices are not exactly `0..per_pattern.len()` (missing, repeated or
///   out-of-range — the commitment would otherwise silently reorder or drop a pattern);
/// * a pattern lists the same source id twice (it is ambiguous whether the plan intends one
///   contribution or two);
/// * `routing` and `envelope` disagree — different operator counts, a label mismatch at some
///   index, a disclose/hide disagreement, or a hidden operator whose routed class differs from the
///   class the envelope declares. Committing to either side of a disagreement would bind a plan
///   that no phase actually produced;
/// * the envelope declares a **hidden** operator with a non-empty `disclosed_operands` list. A
///   hidden operator runs under MPC and discloses nothing, so such an envelope contradicts its own
///   leakage statement;
/// * the envelope's declared participation set is not **exactly** the distinct set of source ids
///   the selection assigns across its patterns (either an omitted or an extra participant). The
///   participation set is the plan's declared leakage/accounting artefact; committing to a
///   selection and a participation claim that contradict each other would bind a plan that no
///   phase produced.
pub fn commit_plan(
    selected: &SelectedPrivateSources,
    routing: &PrivateRouting,
    envelope: &LeakageEnvelope,
) -> Result<PlanCommitment, SeamError> {
    let patterns = commit_patterns(selected)?;
    let operators = commit_operators(routing, envelope)?;

    // Ascending + duplicate-free via the BTreeSet: the commitment binds the participation SET,
    // never the order the envelope happened to list it in.
    let declared_participants: BTreeSet<&String> = envelope.participating_sources.iter().collect();

    // The selection and the envelope are committed as independent components, so they must first
    // be cross-checked against each other — exactly as `commit_operators` cross-checks routing
    // against the envelope. The distinct sources the selection ASSIGNS are precisely the sources
    // that participate (this is how `assemble_leakage_envelope` derives the set), so any omission
    // or addition means the commitment would bind a selection and a participation claim that
    // contradict one another. Fail closed rather than commit to that.
    let assigned_participants: BTreeSet<&String> =
        patterns.iter().flat_map(|p| p.sources.iter()).collect();
    if assigned_participants != declared_participants {
        return Err(SeamError::DescriptorMismatch {
            phase: SeamPhase::PlanBinding,
            source_id: assigned_participants
                .symmetric_difference(&declared_participants)
                .next()
                .copied()
                .cloned()
                .unwrap_or_default(),
            detail: "the envelope's participation set is not the distinct source set the selection assigns",
        });
    }

    let participating_sources: Vec<String> = declared_participants.into_iter().cloned().collect();

    // ── Component digests ──────────────────────────────────────────────────────────────
    let mut selection_parts = vec![arity(patterns.len())];
    for pattern in &patterns {
        selection_parts.push(arity(pattern.pattern));
        selection_parts.push(arity(pattern.sources.len()));
        for source in &pattern.sources {
            selection_parts.push(source.as_bytes().to_vec());
        }
    }
    let selection_digest = digest_parts(SELECTION_DOMAIN_TAG, &selection_parts);

    let mut routing_parts = vec![arity(operators.len())];
    for op in &operators {
        routing_parts.push(op.operator.as_bytes().to_vec());
        routing_parts.push(vec![class_code(op.class)]);
        routing_parts.push(vec![u8::from(op.disclosed)]);
        routing_parts.push(arity(op.disclosed_operands.len()));
        for operand in &op.disclosed_operands {
            routing_parts.push(operand.as_bytes().to_vec());
        }
    }
    let routing_digest = digest_parts(ROUTING_DOMAIN_TAG, &routing_parts);

    let mut participation_parts = vec![arity(participating_sources.len())];
    for source in &participating_sources {
        participation_parts.push(source.as_bytes().to_vec());
    }
    let participation_digest = digest_parts(PARTICIPATION_DOMAIN_TAG, &participation_parts);

    let plan_digest = digest_parts(
        PLAN_DOMAIN_TAG,
        &[
            selection_digest.as_be_bytes().to_vec(),
            routing_digest.as_be_bytes().to_vec(),
            participation_digest.as_be_bytes().to_vec(),
        ],
    );

    Ok(PlanCommitment {
        patterns,
        operators,
        participating_sources,
        selection_digest,
        routing_digest,
        participation_digest,
        plan_digest,
    })
}

/// Canonicalise the Phase-2 selection into ascending-by-index [`CommittedPattern`]s, failing
/// closed on a non-permutation index set or a duplicated source id.
fn commit_patterns(selected: &SelectedPrivateSources) -> Result<Vec<CommittedPattern>, SeamError> {
    let len = selected.per_pattern.len();
    let mut seen_indices = BTreeSet::new();
    let mut patterns = Vec::with_capacity(len);

    for entry in &selected.per_pattern {
        if entry.pattern >= len || !seen_indices.insert(entry.pattern) {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                source_id: String::new(),
                detail: "selection pattern indices are not a permutation of 0..per_pattern.len()",
            });
        }
        let mut sources = BTreeSet::new();
        for candidate in &entry.candidates {
            if !sources.insert(candidate.source_id.0.clone()) {
                return Err(SeamError::DescriptorMismatch {
                    phase: SeamPhase::PlanBinding,
                    source_id: candidate.source_id.0.clone(),
                    detail: "a pattern lists the same source id twice — ambiguous plan",
                });
            }
        }
        patterns.push(CommittedPattern {
            pattern: entry.pattern,
            sources: sources.into_iter().collect(),
        });
    }

    patterns.sort_by_key(|p| p.pattern);
    Ok(patterns)
}

/// Cross-check the Phase-3 routing against the Phase-4 envelope and fold them into
/// [`CommittedOperator`]s, failing closed on any disagreement.
fn commit_operators(
    routing: &PrivateRouting,
    envelope: &LeakageEnvelope,
) -> Result<Vec<CommittedOperator>, SeamError> {
    if routing.routing.len() != envelope.operators.len() {
        return Err(SeamError::DescriptorMismatch {
            phase: SeamPhase::PlanBinding,
            source_id: String::new(),
            detail: "routing and leakage envelope disagree on the operator count",
        });
    }

    let mut operators = Vec::with_capacity(routing.routing.len());
    for (routed, declared) in routing.routing.iter().zip(&envelope.operators) {
        if routed.operator != declared.operator {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                source_id: String::new(),
                detail: "routing and leakage envelope disagree on an operator label",
            });
        }
        let routed_disclosed = matches!(routed.routing, Routing::Disclosed);
        if routed_disclosed != declared.disclosed {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                source_id: String::new(),
                detail: "routing and leakage envelope disagree on an operator's disclose/hide route",
            });
        }
        if let Routing::Hidden(class) = routed.routing {
            if class != declared.class {
                return Err(SeamError::DescriptorMismatch {
                    phase: SeamPhase::PlanBinding,
                    source_id: String::new(),
                    detail: "routing and leakage envelope disagree on a hidden operator's class",
                });
            }
        }
        // A hidden operator runs under MPC and discloses NO operand — that is what the envelope's
        // own leakage statement says, and what `CommittedOperator::disclosed_operands` documents.
        // An envelope that hides an operator while listing operands for it contradicts itself, so
        // refuse rather than commit to a non-canonical `disclosed = false` + non-empty pair whose
        // digest would assert operands are revealed on a route that reveals nothing.
        if !declared.disclosed && !declared.disclosed_operands.is_empty() {
            return Err(SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                source_id: String::new(),
                detail: "the leakage envelope lists disclosed operands for a HIDDEN operator",
            });
        }
        let disclosed_operands: Vec<String> = declared
            .disclosed_operands
            .iter()
            .cloned()
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        operators.push(CommittedOperator {
            operator: routed.operator.clone(),
            class: declared.class,
            disclosed: declared.disclosed,
            disclosed_operands,
        });
    }
    Ok(operators)
}

/// Re-validate an **independently recomputed** plan against the planner's **claimed** commitment,
/// failing closed.
///
/// `claimed` is the commitment the (untrusted) planner published; `observed` is the commitment a
/// verifier recomputed with [`commit_plan`] from what it independently determined the plan to be.
/// Every divergence is reported, so the caller sees *which* source was dropped and *which*
/// operator was re-routed rather than a bare mismatch.
///
/// # Fail-closed invariant
///
/// [`PlanRevalidation::Bound`] is returned **only** when every committed field matches **and**
/// [`PlanCommitment::plan_digest`] matches. If the digests differ but no field-level difference is
/// found, the result is [`PlanRevalidation::Diverged`] carrying
/// [`PlanDivergence::DigestMismatch`] — never `Bound`.
///
/// # This is not a soundness check
///
/// A malicious planner computes both the plan and its commitment, so it can always publish a
/// commitment to the plan it actually ran and pass this check. Only the deferred
/// [`bind_plan_to_proof`] makes divergence a *proof-verification* failure. See the module docs.
pub fn revalidate_plan(claimed: &PlanCommitment, observed: &PlanCommitment) -> PlanRevalidation {
    let mut divergences = Vec::new();

    if claimed.patterns.len() != observed.patterns.len() {
        divergences.push(PlanDivergence::PatternArity {
            claimed: claimed.patterns.len(),
            observed: observed.patterns.len(),
        });
    }
    // Walk the UNION of the two pattern lists, not a `zip` of them. A `zip` truncates to the
    // shorter side, which would leave a dropped *trailing* pattern's sources unnamed — reported
    // only as a bare arity mismatch. Since naming the dropped source is this function's whole
    // contract, the tail is walked explicitly. (Both lists are sorted by, and are a permutation
    // of, `0..len`, so position == pattern index and the pairing below is the right one.)
    for index in 0..claimed.patterns.len().max(observed.patterns.len()) {
        let claimed_set: BTreeSet<&String> = claimed
            .patterns
            .get(index)
            .map(|p| p.sources.iter().collect())
            .unwrap_or_default();
        let observed_set: BTreeSet<&String> = observed
            .patterns
            .get(index)
            .map(|p| p.sources.iter().collect())
            .unwrap_or_default();
        for dropped in claimed_set.difference(&observed_set) {
            divergences.push(PlanDivergence::SourceDropped {
                pattern: index,
                source_id: (*dropped).clone(),
            });
        }
        for added in observed_set.difference(&claimed_set) {
            divergences.push(PlanDivergence::SourceAdded {
                pattern: index,
                source_id: (*added).clone(),
            });
        }
    }

    if claimed.operators.len() != observed.operators.len() {
        divergences.push(PlanDivergence::OperatorArity {
            claimed: claimed.operators.len(),
            observed: observed.operators.len(),
        });
    }
    // Operators are positional and carry no index of their own, so — unlike the patterns above —
    // a count mismatch IS the full description of the tail ("operators n..m are absent"), and
    // `OperatorArity` above names both counts. The pairwise walk below therefore covers only the
    // common prefix by design; the truncation is not a silent one.
    for (index, (claimed_op, observed_op)) in
        claimed.operators.iter().zip(&observed.operators).enumerate()
    {
        if claimed_op.operator != observed_op.operator {
            divergences.push(PlanDivergence::OperatorRelabelled {
                index,
                claimed: claimed_op.operator.clone(),
                observed: observed_op.operator.clone(),
            });
        }
        if claimed_op.class != observed_op.class {
            divergences.push(PlanDivergence::OperatorClassChanged {
                index,
                operator: claimed_op.operator.clone(),
            });
        }
        if claimed_op.disclosed != observed_op.disclosed {
            divergences.push(PlanDivergence::OperatorRerouted {
                index,
                operator: claimed_op.operator.clone(),
                claimed_disclosed: claimed_op.disclosed,
                observed_disclosed: observed_op.disclosed,
            });
        }
        if claimed_op.disclosed_operands != observed_op.disclosed_operands {
            divergences.push(PlanDivergence::DisclosedOperandsChanged {
                index,
                operator: claimed_op.operator.clone(),
                claimed: claimed_op.disclosed_operands.clone(),
                observed: observed_op.disclosed_operands.clone(),
            });
        }
    }

    let claimed_participants: BTreeSet<&String> = claimed.participating_sources.iter().collect();
    let observed_participants: BTreeSet<&String> = observed.participating_sources.iter().collect();
    if claimed_participants != observed_participants {
        divergences.push(PlanDivergence::ParticipationChanged {
            dropped: claimed_participants
                .difference(&observed_participants)
                .map(|s| s.to_string())
                .collect(),
            added: observed_participants
                .difference(&claimed_participants)
                .map(|s| s.to_string())
                .collect(),
        });
    }

    // Fail closed: a digest mismatch must never be reported as bound, even if every structured
    // comparison above happened to agree.
    if claimed.plan_digest != observed.plan_digest && divergences.is_empty() {
        divergences.push(PlanDivergence::DigestMismatch);
    }

    if divergences.is_empty() {
        PlanRevalidation::Bound
    } else {
        PlanRevalidation::Diverged(divergences)
    }
}

/// **Deferred (Phase 5's soundness half).** Bind a [`PlanCommitment`] into the collaborative
/// proof's public inputs, so that a dropped or mis-routed source yields a **verification
/// failure** rather than a forged accept (constraint C-A).
///
/// This is the step that would actually discharge C-A, and it cannot be built yet: there is no
/// collaborative proof to bind to ([`sparq_mpc::proof::CollaborativeProof`] is an honest
/// `MpcError::NotYetImplemented` stub), and the audits that would have to clear any such claim —
/// the external accredited-cryptographer sign-off (`sq-qhy4`) and the collaborative-coZK
/// re-audit (`sq-9hrn`) — are pending.
///
/// It carries its real signature and is callable, so the seam's shape is explicit and testable:
/// it returns [`SeamError::Deferred`] naming every gate. It fabricates no binding, performs no
/// MPC, and never panics.
///
/// `statement` names [`FederatedStatement`] as the **proposed** attachment point — its M4-v1
/// public-input layout already carries the analogous `query_digest` / `key_set_digest` words, so a
/// plan word would sit naturally beside them. Design record **§9 Q3 is still OPEN**: this is a
/// proposal for the maintainer, not a settled decision, and the signature may change once the
/// proof's shape is known. Both parameters are therefore unused today.
///
/// # Errors
///
/// Always returns [`SeamError::Deferred`] with phase [`SeamPhase::PlanBinding`].
pub fn bind_plan_to_proof(
    _commitment: &PlanCommitment,
    _statement: &FederatedStatement,
) -> Result<FieldWord, SeamError> {
    Err(SeamError::Deferred {
        phase: SeamPhase::PlanBinding,
        gated_on: PLAN_BINDING_GATED_ON,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_fedplan::{Bgp, PredPartition, SourceDescriptor, SourceId, Term, TriplePattern, Var};

    use crate::envelope::assemble_leakage_envelope;
    use crate::privacy::SourcePrivacyDescriptor;
    use crate::routing::{route_operators, Operand, QueryOperator, RoutingPolicy};
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
        ]
    }

    fn membership_pattern() -> TriplePattern {
        TriplePattern::new(
            Term::Var(Var::new("h")),
            Term::Iri(MEMBER_OF.to_string()),
            Term::Iri(FLAT_IRI.to_string()),
        )
    }

    fn bgp() -> Bgp {
        Bgp::new(vec![membership_pattern()])
    }

    /// A two-pattern BGP — used to exercise the pattern TAIL of `revalidate_plan`.
    fn bgp_two_patterns() -> Bgp {
        Bgp::new(vec![
            membership_pattern(),
            TriplePattern::new(
                Term::Var(Var::new("h")),
                Term::Iri(SALARY.to_string()),
                Term::Var(Var::new("s")),
            ),
        ])
    }

    /// The REAL Phase-2 → Phase-3 → Phase-4 pipeline over the given BGP + source ids, committed.
    fn plan_over_bgp(bgp: &Bgp, ids: &[&str]) -> PlanCommitment {
        let sources: Vec<SourceDescriptor> = ids.iter().map(|id| source(id)).collect();
        let privacy: Vec<SourcePrivacyDescriptor> =
            ids.iter().map(|id| flatmate_privacy(id)).collect();
        let selected = select_private_sources(bgp, &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        let envelope = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        commit_plan(&selected, &routing, &envelope).unwrap()
    }

    /// The REAL Phase-2 → Phase-3 → Phase-4 pipeline over the given source ids, committed.
    fn plan_over(ids: &[&str]) -> PlanCommitment {
        let sources: Vec<SourceDescriptor> = ids.iter().map(|id| source(id)).collect();
        let privacy: Vec<SourcePrivacyDescriptor> =
            ids.iter().map(|id| flatmate_privacy(id)).collect();
        let selected = select_private_sources(&bgp(), &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        let envelope = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        commit_plan(&selected, &routing, &envelope).unwrap()
    }

    // ── Canonicalisation ────────────────────────────────────────────────────────────────

    /// The commitment is deterministic over the REAL pipeline: the same plan commits to the same
    /// digest every time. Without this, re-validation could never conclude anything.
    #[test]
    fn commitment_is_deterministic() {
        let first = plan_over(&["http://a/", "http://b/"]);
        let second = plan_over(&["http://a/", "http://b/"]);
        assert_eq!(first.plan_digest(), second.plan_digest());
        assert_eq!(first, second);
        assert!(revalidate_plan(&first, &second).is_bound());
    }

    /// The digest binds plan CONTENT, not the planner's enumeration order: presenting the same two
    /// sources in the opposite order must commit identically, or a benign reordering would read as
    /// a dropped source.
    #[test]
    fn commitment_is_independent_of_source_enumeration_order() {
        let forward = plan_over(&["http://a/", "http://b/"]);
        let reversed = plan_over(&["http://b/", "http://a/"]);
        assert_eq!(forward.plan_digest(), reversed.plan_digest());
        assert!(revalidate_plan(&forward, &reversed).is_bound());
    }

    /// The three component digests are domain-separated, so no component's digest can be replayed
    /// as another's.
    #[test]
    fn component_digests_are_domain_separated() {
        let plan = plan_over(&["http://a/", "http://b/"]);
        let digests = [
            plan.selection_digest(),
            plan.routing_digest(),
            plan.participation_digest(),
            plan.plan_digest(),
        ];
        for (i, a) in digests.iter().enumerate() {
            for b in digests.iter().skip(i + 1) {
                assert_ne!(a, b, "component digests must not collide");
            }
        }
    }

    // ── Re-validation: the divergences C-A is about ─────────────────────────────────────

    /// **The load-bearing case.** A plan that DROPS a source must not re-validate as bound, and
    /// the divergence must name the source that went missing.
    #[test]
    fn a_dropped_source_diverges_and_is_named() {
        let claimed = plan_over(&["http://a/", "http://b/"]);
        let observed = plan_over(&["http://a/"]);

        assert_ne!(claimed.plan_digest(), observed.plan_digest());
        let outcome = revalidate_plan(&claimed, &observed);
        assert!(!outcome.is_bound(), "a dropped source must NOT read as bound");

        let PlanRevalidation::Diverged(divergences) = outcome else {
            panic!("expected divergence");
        };
        assert!(
            divergences.contains(&PlanDivergence::SourceDropped {
                pattern: 0,
                source_id: "http://b/".to_string(),
            }),
            "the dropped source must be named: {divergences:?}"
        );
        assert!(divergences
            .iter()
            .any(|d| matches!(d, PlanDivergence::ParticipationChanged { .. })));
    }

    /// A source dropped from a **trailing** pattern must still be NAMED, not merely counted. A
    /// `zip` of the two pattern lists truncates to the shorter side and would report only a bare
    /// `PatternArity`, hiding *which* source went missing — the divergence this function exists to
    /// surface.
    #[test]
    fn a_source_dropped_from_a_trailing_pattern_is_still_named() {
        let claimed = plan_over_bgp(&bgp_two_patterns(), &["http://a/"]);
        let observed = plan_over_bgp(&bgp(), &["http://a/"]);

        assert!(
            claimed.patterns.len() > observed.patterns.len(),
            "fixture must exercise the tail: {} vs {}",
            claimed.patterns.len(),
            observed.patterns.len()
        );
        let PlanRevalidation::Diverged(divergences) = revalidate_plan(&claimed, &observed) else {
            panic!("expected divergence");
        };
        assert!(divergences
            .iter()
            .any(|d| matches!(d, PlanDivergence::PatternArity { .. })));
        assert!(
            divergences.contains(&PlanDivergence::SourceDropped {
                pattern: 1,
                source_id: "http://a/".to_string(),
            }),
            "the trailing pattern's dropped source must be named: {divergences:?}"
        );
    }

    /// A MIS-ROUTED operator — one moved from the hidden route to the disclosed route — must
    /// diverge and be named. This is the second half of constraint C-A's threat.
    #[test]
    fn a_rerouted_operator_diverges_and_is_named() {
        let claimed = plan_over(&["http://a/", "http://b/"]);

        // Tamper with the recomputed plan exactly as an over-disclosing planner would: flip the
        // hidden salary aggregate onto the disclosed route.
        let mut observed = claimed.clone();
        observed.operators[1].disclosed = true;
        observed.operators[1].disclosed_operands = vec![SALARY.to_string()];

        let outcome = revalidate_plan(&claimed, &observed);
        assert!(!outcome.is_bound());
        let PlanRevalidation::Diverged(divergences) = outcome else {
            panic!("expected divergence");
        };
        assert!(divergences.contains(&PlanDivergence::OperatorRerouted {
            index: 1,
            operator: "salary-cumulative-sum".to_string(),
            claimed_disclosed: false,
            observed_disclosed: true,
        }));
        assert!(divergences
            .iter()
            .any(|d| matches!(d, PlanDivergence::DisclosedOperandsChanged { .. })));
    }

    /// Fail-closed: a plan whose structured fields agree but whose digest does not must STILL
    /// report divergence. `Bound` may never be returned on a digest mismatch.
    #[test]
    fn digest_mismatch_alone_is_never_bound() {
        let claimed = plan_over(&["http://a/", "http://b/"]);
        let mut observed = claimed.clone();
        observed.plan_digest = FieldWord::ZERO;

        let outcome = revalidate_plan(&claimed, &observed);
        assert!(!outcome.is_bound());
        assert_eq!(
            outcome,
            PlanRevalidation::Diverged(vec![PlanDivergence::DigestMismatch])
        );
    }

    // ── Fail-closed construction ────────────────────────────────────────────────────────

    /// `commit_plan` refuses a routing/envelope pair that disagrees rather than committing to a
    /// plan neither phase produced.
    #[test]
    fn routing_envelope_disagreement_is_refused() {
        let sources = vec![source("http://a/")];
        let privacy = vec![flatmate_privacy("http://a/")];
        let selected = select_private_sources(&bgp(), &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        let mut envelope = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();

        // The envelope now claims a disclose route the Phase-3 routing did not take.
        envelope.operators[1].disclosed = true;

        let err = commit_plan(&selected, &routing, &envelope)
            .expect_err("a routing/envelope disagreement must be refused");
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                ..
            }
        ));
    }

    /// A truncated envelope (fewer operators than the routing) is refused, not silently zipped.
    #[test]
    fn operator_count_disagreement_is_refused() {
        let sources = vec![source("http://a/")];
        let privacy = vec![flatmate_privacy("http://a/")];
        let selected = select_private_sources(&bgp(), &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        let mut envelope = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        envelope.operators.truncate(1);

        assert!(commit_plan(&selected, &routing, &envelope).is_err());
    }

    /// The REAL pipeline over two flatmates, returned unassembled so a test can MUTATE the
    /// envelope the way a lying planner would before committing.
    fn two_flatmate_plan_parts() -> (SelectedPrivateSources, PrivateRouting, LeakageEnvelope) {
        let ids = ["http://a/", "http://b/"];
        let sources: Vec<SourceDescriptor> = ids.iter().map(|id| source(id)).collect();
        let privacy: Vec<SourcePrivacyDescriptor> =
            ids.iter().map(|id| flatmate_privacy(id)).collect();
        let selected = select_private_sources(&bgp(), &sources, &privacy).unwrap();
        let ops = four_flatmates_operators();
        let routing = route_operators(&selected, &privacy, &ops, RoutingPolicy::Default).unwrap();
        let envelope = assemble_leakage_envelope(&routing, &ops, &selected).unwrap();
        (selected, routing, envelope)
    }

    /// The participation set is the plan's declared leakage/accounting artefact, so DROPPING a
    /// participant the selection still assigns must be refused — not committed as a
    /// selection/participation pair that contradicts itself.
    #[test]
    fn a_participant_dropped_from_the_envelope_is_refused() {
        let (selected, routing, mut envelope) = two_flatmate_plan_parts();
        assert_eq!(envelope.participating_sources.len(), 2, "fixture");

        // The envelope now under-declares: source B still answers the pattern in the selection.
        envelope
            .participating_sources
            .retain(|id| id != "http://b/");

        let err = commit_plan(&selected, &routing, &envelope)
            .expect_err("an omitted participant must be refused");
        match err {
            SeamError::DescriptorMismatch {
                phase, source_id, ..
            } => {
                assert_eq!(phase, SeamPhase::PlanBinding);
                assert_eq!(source_id, "http://b/", "the divergent source must be named");
            }
            other => panic!("expected DescriptorMismatch, got {:?}", other),
        }
    }

    /// …and ADDING a participant the selection assigns to no pattern is refused too, so the
    /// cross-check is an equality, not a containment that an over-declaring envelope could pass.
    #[test]
    fn an_extra_envelope_participant_is_refused() {
        let (selected, routing, mut envelope) = two_flatmate_plan_parts();

        envelope
            .participating_sources
            .push("http://unrelated/".to_string());

        let err = commit_plan(&selected, &routing, &envelope)
            .expect_err("an extra participant must be refused");
        match err {
            SeamError::DescriptorMismatch {
                phase, source_id, ..
            } => {
                assert_eq!(phase, SeamPhase::PlanBinding);
                assert_eq!(source_id, "http://unrelated/");
            }
            other => panic!("expected DescriptorMismatch, got {:?}", other),
        }
    }

    /// A HIDDEN operator discloses nothing, so an envelope that lists an operand for one
    /// contradicts its own leakage statement and must be refused — otherwise the commitment would
    /// carry `disclosed = false` alongside a non-empty operand list, exactly the non-canonical
    /// pair `CommittedOperator::disclosed_operands` documents as impossible.
    #[test]
    fn a_hidden_operator_with_disclosed_operands_is_refused() {
        let (selected, routing, mut envelope) = two_flatmate_plan_parts();
        assert!(
            !envelope.operators[1].disclosed && envelope.operators[1].disclosed_operands.is_empty(),
            "fixture: the salary aggregate must be hidden and disclose nothing"
        );

        // The envelope now claims the hidden salary aggregate reveals the salary predicate.
        envelope.operators[1].disclosed_operands = vec![SALARY.to_string()];

        let err = commit_plan(&selected, &routing, &envelope)
            .expect_err("a hidden operator carrying disclosed operands must be refused");
        assert!(matches!(
            err,
            SeamError::DescriptorMismatch {
                phase: SeamPhase::PlanBinding,
                ..
            }
        ));
        assert!(format!("{}", err).contains("HIDDEN"));
    }

    // ── The gated half stays gated ──────────────────────────────────────────────────────

    /// The soundness half of Phase 5 must stay an honest, gate-naming `Deferred` — never a
    /// fabricated binding, never a panic. This is the no-fake-crypto discipline
    /// `sparq-mpc`'s `proof.rs` holds, enforced here as a test so it cannot drift.
    #[test]
    fn binding_to_the_proof_stays_deferred_and_names_its_gates() {
        let plan = plan_over(&["http://a/", "http://b/"]);
        let statement = statement();

        let err = bind_plan_to_proof(&plan, &statement)
            .expect_err("the plan→proof binding must NOT silently succeed");
        match err {
            SeamError::Deferred { phase, gated_on } => {
                assert_eq!(phase, SeamPhase::PlanBinding);
                assert!(gated_on.contains("sq-qhy4"), "must name the external audit");
                assert!(gated_on.contains("sq-9hrn"), "must name the coZK re-audit");
                assert!(
                    gated_on.contains("NotYetImplemented"),
                    "must name the missing collaborative proof"
                );
            }
            other => panic!("expected Deferred, got {other:?}"),
        }
    }

    /// A minimal, well-formed `FederatedStatement` — only needed to call the deferred binding at
    /// its real signature.
    fn statement() -> FederatedStatement {
        use sparq_mpc::partial::HolderId;
        use sparq_mpc::{HolderSegment, ValidityWindow, VerifierChallenge};

        let segment = HolderSegment::new(
            HolderId::new("http://a/"),
            vec![FieldWord::ZERO],
            vec![[FieldWord::ZERO, FieldWord::ZERO, FieldWord::ZERO]],
            1,
            vec![true],
        )
        .unwrap();
        FederatedStatement::new(
            VerifierChallenge::from_word(FieldWord::ZERO),
            FieldWord::ZERO,
            FieldWord::ZERO,
            ValidityWindow::new(0, 1).unwrap(),
            vec![segment],
        )
        .unwrap()
    }
}

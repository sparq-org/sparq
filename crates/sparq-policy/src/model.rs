//! The typed ODRL Information Model (the subset sparq-policy evaluates).
//!
//! This is a faithful-but-scoped Rust rendering of the [ODRL 2.2 Information
//! Model](https://www.w3.org/TR/odrl-model/): a [`Policy`] is a container of
//! deontic [`Rule`]s (Permission / Prohibition), each carrying an [`Action`], a
//! `target` asset, optional `assignee`/`assigner` parties, zero or more atomic
//! [`Constraint`]s and compound [`LogicalConstraint`]s (`odrl:and`/`odrl:or`/
//! `odrl:xone`), and (for permissions) zero or more [`Duty`] obligations.
//!
//! Scope (the single-node base case — see the crate README): the evaluator
//! supports `Permission` and `Prohibition` rules with the common
//! `leftOperand`/`operator`/`rightOperand` constraints; `Offer`/`Agreement`
//! policy subtypes are parsed identically to `Set` (the subtype affects
//! contracting, not single-node evaluation).
//! [OPUS-4.8]

use std::collections::BTreeSet;
use std::fmt;

/// The ODRL namespace IRI prefix (`odrl:`).
pub const ODRL_NS: &str = "http://www.w3.org/ns/odrl/2/";

/// An ODRL policy: a container of deontic rules.
///
/// A policy is evaluated as a whole against a single request: a `Permission`
/// whose action/target/constraints all match the request grants access, and any
/// matching `Prohibition` overrides it (fail-closed — see
/// [`crate::evaluate`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Policy {
    /// The policy IRI (the `odrl:Policy`/`Set`/`Offer`/`Agreement` subject), if any.
    pub iri: Option<String>,
    /// `odrl:permission` rules — each, if matched and unprohibited, grants access.
    pub permissions: Vec<Rule>,
    /// `odrl:prohibition` rules — each, if matched, forbids access (overrides).
    pub prohibitions: Vec<Rule>,
    /// The declared `odrl:conflict` conflict-resolution [`ConflictStrategy`], if the
    /// policy names one. `None` means the policy left it unset. [OPUS-4.8] sq-ihqbl.
    ///
    /// **The bridge implements exactly one strategy — `odrl:prohibit` (deny-overrides).**
    /// A strategy this engine cannot faithfully honour (`odrl:perm`, an `odrl:invalid`
    /// policy that has a detected conflict, or an unknown value) is *refused* rather than
    /// silently mis-applied — see [`crate::conflict_admissibility`].
    pub conflict: Option<ConflictStrategy>,
    /// The IRIs this policy document identifies as an **`odrl:PartyCollection`** —
    /// collection IDENTITY, carried independently of any member list. [SONNET-4.6]
    /// sq-rf9uv.
    ///
    /// Populated by [`crate::parse_policy`] from the policy graph itself: every subject
    /// asserted `a odrl:PartyCollection`, plus every object of an `odrl:partOf` edge the
    /// document states (writing `<alice> odrl:partOf <lab>` identifies `<lab>` as a
    /// collection whether or not the type triple is also present). Empty for a
    /// hand-built [`Policy`].
    ///
    /// **Why it is separate from membership.** Membership evidence lives on the
    /// [`Request`](crate::Request), is per-request and possibly partial, so "this head
    /// has at least one known member" is a lower bound on collection-ness, never a test
    /// for it. A consumer that must fail CLOSED on collection-valued input — the
    /// sparq-solid ODRL→ACP bridge, which freezes a rule into an identity-matched ACP
    /// head that cannot re-check membership — needs the identity regardless of whether
    /// this particular request happened to supply an edge. Reading collection-ness off
    /// the member list instead lets a zero-edge collection be frozen as a plain party
    /// IRI, and every member then escapes the frozen deny (fail-OPEN).
    pub party_collections: BTreeSet<String>,
}

/// The ODRL `odrl:conflict` conflict-resolution strategy (an `odrl:ConflictTerm`)
/// declared on a [`Policy`] to resolve a permission/prohibition conflict.
/// [OPUS-4.8] sq-ihqbl.
///
/// Per the [ODRL 2.2 Information Model](https://www.w3.org/TR/odrl-model/#conflict) the
/// property takes one of three terms; sparq's bridge implements only `odrl:prohibit`
/// (deny-overrides), so the others are recorded here and *refused* at materialisation
/// time (see [`crate::conflict_admissibility`]) rather than silently coerced — silently
/// mishandling a conflict strategy is an authorization-correctness hazard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// `odrl:prohibit` — the Prohibitions override the Permissions (deny-overrides).
    /// The one strategy the bridge implements: enforcement subtracts `∪ deny` from
    /// `∪ allow`, so a matching prohibition already beats any permission.
    Prohibit,
    /// `odrl:perm` — the Permissions override the Prohibitions. **Unrepresentable** by
    /// the bridge's allow-minus-deny subtraction (a deny always wins), so a policy
    /// declaring it is refused rather than silently enforced as deny-overrides.
    Perm,
    /// `odrl:invalid` — a conflicting policy is void *as a whole* (the ODRL default when
    /// unset). The bridge cannot represent "void the whole policy", so an `Invalid`
    /// policy with a detected permission/prohibition conflict is refused; one with no
    /// detected conflict is admissible (nothing to void).
    Invalid,
    /// An `odrl:conflict` value that is not one of the three ODRL `ConflictTerm`s — an
    /// unknown/unsupported strategy IRI. Always refused (fail-closed): the engine has no
    /// semantics for it, so it must not be silently ignored.
    Unknown(String),
}

impl ConflictStrategy {
    /// Parse an `odrl:conflict` value IRI into a [`ConflictStrategy`]. The three ODRL
    /// `ConflictTerm`s (`odrl:prohibit`/`odrl:perm`/`odrl:invalid`) map to their variant;
    /// anything else becomes [`ConflictStrategy::Unknown`] (carrying the raw IRI) so an
    /// unrecognised strategy is refused, never silently dropped. [OPUS-4.8] sq-ihqbl.
    pub fn from_iri(iri: &str) -> ConflictStrategy {
        match iri.strip_prefix(ODRL_NS) {
            Some("prohibit") => ConflictStrategy::Prohibit,
            Some("perm") => ConflictStrategy::Perm,
            Some("invalid") => ConflictStrategy::Invalid,
            _ => ConflictStrategy::Unknown(iri.to_owned()),
        }
    }
}

/// A deontic rule (a Permission or a Prohibition). They share the same shape;
/// the [`Policy`] field they live under fixes their deontic force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule node IRI/blank-id (for justification in the decision).
    pub id: String,
    /// The regulated [`Action`] (`odrl:action`).
    pub action: Action,
    /// The target asset IRI (`odrl:target`) — the data/graph the rule is about.
    /// `None` means the rule applies to any target (an unrefined rule).
    pub target: Option<String>,
    /// The party the rule is assigned to (`odrl:assignee`) — the requesting
    /// party it grants/forbids for. `None` = applies to any party.
    pub assignee: Option<String>,
    /// The party issuing the rule (`odrl:assigner`). Informational for single-node
    /// evaluation; carried for justification + audit.
    pub assigner: Option<String>,
    /// Atomic constraints (`odrl:constraint` that names a direct
    /// `leftOperand`/`operator`/`rightOperand` block) that must all hold for the
    /// rule to be *satisfied* (logical AND, per the ODRL default `odrl:and`).
    pub constraints: Vec<Constraint>,
    /// Compound `odrl:LogicalConstraint` refinements (`odrl:and` / `odrl:or` /
    /// `odrl:xone` over a *set* of sub-constraints — [OPUS-4.8] sq-a0zef). A rule's
    /// atomic [`constraints`](Rule::constraints) **and** its `logical_constraints`
    /// are ALL ANDed together: the rule is satisfied only when every atomic
    /// constraint holds AND every logical constraint holds. Empty for a rule whose
    /// `odrl:constraint` objects are all direct (non-`LogicalConstraint`) blocks —
    /// the common single-node base case.
    pub logical_constraints: Vec<LogicalConstraint>,
    /// Duties (`odrl:duty`) — obligations that must be discharged for a
    /// *permission* to be exercisable. Ignored on prohibitions (a prohibition
    /// has no pre-condition duty in this base case).
    pub duties: Vec<Duty>,
}

/// An ODRL action: the regulated operation (`odrl:use`, `odrl:read`,
/// `odrl:distribute`, `odrl:aggregate`, `odrl:anonymize`, …).
///
/// Stored as the full IRI. The well-known `odrl:use` action is the ODRL
/// "umbrella" action: a permission for `odrl:use` matches a request for any action
/// **that the ODRL 2.2 action hierarchy places under `use`** (`odrl:includedIn`).
/// All other actions match by exact IRI equality in this base case.
///
/// **The `use` umbrella reconciliation ([OPUS-4.8] sq-euhr3).** sparq-policy
/// previously treated `odrl:use` as subsuming *every* action; the SolidLab ODRL Test
/// Suite (and the ODRL 2.2 vocabulary) is more precise — `use` includes its proper
/// sub-actions (`read`, `display`, `print`, `modify`, `delete`, … — 39 actions are
/// `includedIn use` transitively in [ODRL 2.2](https://www.w3.org/TR/odrl-vocab/)),
/// but **NOT** the ownership-`transfer` subtree: `odrl:sell` / `odrl:give` are
/// `includedIn odrl:transfer`, not `use`, so a `use` permission does **not** authorise
/// selling. The canonical reading is therefore: `use` permits a requested action that
/// is `use` itself or any action *not provably in the `transfer` subtree* (the one
/// disjoint top-level branch the vocabulary defines) — which makes a `use` permission
/// Active for `read`/`write` and Inactive for `sell`/`give`, matching the suite's
/// cases 007/008/009 (Active) vs 010/017 (Inactive). See [`permits`](Action::permits).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action(pub String);

impl Action {
    /// The `odrl:use` umbrella action IRI.
    pub fn use_() -> Action {
        Action(format!("{ODRL_NS}use"))
    }

    /// True iff this action permits a request for `requested`: exact-IRI match, or
    /// this action is the `odrl:use` umbrella and `requested` is **included in `use`**
    /// under the ODRL 2.2 action hierarchy (i.e. `requested` is not in the disjoint
    /// `odrl:transfer` ownership subtree). [OPUS-4.8] sq-euhr3.
    ///
    /// The transfer subtree (`odrl:sell`, `odrl:give`, `odrl:transfer`) is the one
    /// well-defined top-level branch the ODRL vocabulary places *outside* `use`, so it
    /// is excluded explicitly; every other action — including non-vocabulary actions
    /// such as `odrl:write` — is treated as a use-subtree action (the broad "use this
    /// asset" reading), matching the SolidLab reference evaluator.
    pub fn permits(&self, requested: &Action) -> bool {
        if self == requested {
            return true;
        }
        self.0 == format!("{ODRL_NS}use") && !is_transfer_subtree(&requested.0)
    }
}

/// Whether `action_iri` is in the ODRL 2.2 ownership-`transfer` subtree — the one
/// top-level action branch the vocabulary places *outside* `odrl:use`. These actions
/// are `odrl:includedIn odrl:transfer` (or are `odrl:transfer` itself), so a `use`
/// permission does **not** subsume them. [OPUS-4.8] sq-euhr3.
///
/// The transfer subtree is finite and closed in ODRL 2.2: `transfer` plus its
/// included actions `sell` and `give`. Encoded as an explicit set so the `use`
/// umbrella excludes exactly this branch (and nothing else); any action not here is
/// treated as a use-subtree action.
fn is_transfer_subtree(action_iri: &str) -> bool {
    let Some(local) = action_iri.strip_prefix(ODRL_NS) else {
        return false;
    };
    matches!(local, "transfer" | "sell" | "give")
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An ODRL constraint: a `(leftOperand, operator, rightOperand)` triple
/// (`odrl:Constraint`) evaluated against the request + state-of-the-world.
///
/// Example: `(odrl:dateTime, odrl:lteq, "2026-12-31T00:00:00Z")` — the request
/// must occur at or before that instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// The dimension being constrained (`odrl:leftOperand`), e.g. `odrl:dateTime`,
    /// `odrl:purpose`, `odrl:recipient`, `odrl:spatial`, `odrl:count`. Stored as
    /// the full IRI.
    pub left: String,
    /// The comparison [`Operator`] (`odrl:operator`).
    pub operator: Operator,
    /// The value to compare against (`odrl:rightOperand`), as a typed
    /// [`Value`].
    pub right: Value,
}

/// A compound `odrl:LogicalConstraint` — a logical combinator (`odrl:and` /
/// `odrl:or` / `odrl:xone`) over a *set* of sub-[`Constraint`]s. [OPUS-4.8] sq-a0zef.
///
/// ODRL expresses a compound constraint as an `odrl:LogicalConstraint` node whose
/// `odrl:and` / `odrl:or` / `odrl:xone` property points at a *refining set* of
/// operand constraints (each itself an `odrl:Constraint` **or** a nested
/// `odrl:LogicalConstraint`). In the suite (and the common compact serialisation) the
/// operands are several objects of that one property (`odrl:and <c1>, <c2>`),
/// evaluated as:
///
/// * `odrl:and`  — satisfied iff **every** operand is satisfied (conjunction).
/// * `odrl:or`   — satisfied iff **at least one** operand is satisfied (disjunction).
/// * `odrl:xone` — satisfied iff **exactly one** operand is satisfied (exclusive-or).
///
/// Operands may themselves be compound ([`ConstraintNode`] is atomic-or-compound), so a
/// nested shape such as an `or` of `and`s (a disjunction of time windows) is supported.
///
/// **Fail-closed:** an operand whose dimension the request supplies no evidence for
/// is *unprovable* (not silently satisfied); see [`crate::evaluate`] for how the
/// three-valued operand verdict folds into each combinator (an `and`/`xone` is never
/// claimed on an unprovable operand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalConstraint {
    /// The compound node IRI/blank-id (for justification in the decision).
    pub id: String,
    /// The logical combinator over the operands.
    pub operator: LogicalOperator,
    /// The operand (sub-)constraints the combinator ranges over — each atomic or itself
    /// compound ([`ConstraintNode`]). An **empty** operand set is treated fail-closed (an
    /// `or`/`xone` with no operand can never be satisfied; an `and` with no operand is
    /// vacuously *not asserted* — see eval).
    pub operands: Vec<ConstraintNode>,
}

/// One operand of a [`LogicalConstraint`] — either an atomic [`Constraint`] or a nested
/// compound [`LogicalConstraint`]. [OPUS-4.8] sq-a0zef.
///
/// This makes compound constraints arbitrarily nestable (an `odrl:or` of `odrl:and`s,
/// etc.) while keeping the *atomic* [`Constraint`] type — used pervasively by the
/// evaluator, the static analyser, and count enforcement — unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintNode {
    /// A leaf atomic constraint (`leftOperand`/`operator`/`rightOperand`).
    Atomic(Constraint),
    /// A nested compound `odrl:LogicalConstraint`.
    Compound(LogicalConstraint),
}

/// The logical combinator of an [`odrl:LogicalConstraint`](LogicalConstraint).
/// [OPUS-4.8] sq-a0zef.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicalOperator {
    /// `odrl:and` — conjunction: every operand must hold.
    And,
    /// `odrl:or` — disjunction: at least one operand must hold.
    Or,
    /// `odrl:xone` — exclusive-or: exactly one operand must hold.
    Xone,
}

impl LogicalOperator {
    /// Parse an `odrl:` logical-combinator IRI into a [`LogicalOperator`]. Unknown
    /// combinators (e.g. the deprecated `odrl:andSequence`) return `None`, and the
    /// enclosing compound constraint then fails closed.
    pub fn from_iri(iri: &str) -> Option<LogicalOperator> {
        let local = iri.strip_prefix(ODRL_NS)?;
        Some(match local {
            "and" => LogicalOperator::And,
            "or" => LogicalOperator::Or,
            "xone" => LogicalOperator::Xone,
            _ => return None,
        })
    }
}

/// The constraint comparison operators sparq-policy supports (the common ODRL
/// operator set — [ODRL operators](https://www.w3.org/TR/odrl-vocab/#constraintLeftOperandCommon)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// `odrl:eq` — equality.
    Eq,
    /// `odrl:neq` — inequality.
    Neq,
    /// `odrl:lt` — strictly less than (numeric or dateTime).
    Lt,
    /// `odrl:lteq` — less than or equal.
    Lteq,
    /// `odrl:gt` — strictly greater than.
    Gt,
    /// `odrl:gteq` — greater than or equal.
    Gteq,
    /// `odrl:isPartOf` — the request value is a member of / part of the
    /// right-operand set, OR — for the taxonomic dimensions `odrl:purpose` and
    /// `odrl:spatial` — **transitively part-of** a named value under the subsumption
    /// closure the request supplies (the DPV-purpose taxonomy / spatial region *tree*
    /// case — see `Request::with_purpose_subsumption`). [OPUS-4.8] sq-z3ve / sq-wukl.
    IsPartOf,
    /// `odrl:isA` — the request value IS the right operand (a type/identity
    /// membership check, treated as equality here).
    IsA,
    /// `odrl:isAnyOf` — the request value equals **at least one** member of the
    /// right-operand set (the [ODRL 2.2 set-relation
    /// operator](https://www.w3.org/TR/odrl-vocab/#term-isAnyOf): the left operand
    /// is *any of* the right-operand values). The right operand uses the same
    /// `|`/space/comma-separated set encoding as [`Operator::IsPartOf`].
    /// [FABLE-5] sq-uaz85.
    IsAnyOf,
    /// `odrl:isNoneOf` — the request value equals **none** of the members of the
    /// right-operand set (the [ODRL 2.2
    /// operator](https://www.w3.org/TR/odrl-vocab/#term-isNoneOf) — the negative
    /// dual of [`Operator::IsAnyOf`]). Fail-closed like every negative operator: a
    /// request with **no** evidence for the dimension is *unprovable*, never
    /// silently satisfied, and a numeric/dateTime operand (no faithful lexical set
    /// form to negate) is not satisfied either — see [`crate::evaluate`].
    /// [FABLE-5] sq-uaz85.
    IsNoneOf,
}

impl Operator {
    /// Parse an `odrl:` operator IRI into an [`Operator`]. Unknown operators
    /// return `None` (and an unparseable constraint fails closed).
    pub fn from_iri(iri: &str) -> Option<Operator> {
        let local = iri.strip_prefix(ODRL_NS)?;
        Some(match local {
            "eq" => Operator::Eq,
            "neq" => Operator::Neq,
            "lt" => Operator::Lt,
            "lteq" => Operator::Lteq,
            "gt" => Operator::Gt,
            "gteq" => Operator::Gteq,
            "isPartOf" => Operator::IsPartOf,
            "isA" => Operator::IsA,
            "isAnyOf" => Operator::IsAnyOf,
            "isNoneOf" => Operator::IsNoneOf,
            _ => return None,
        })
    }
}

/// A constraint right-operand or request value, kept as a typed scalar so the
/// evaluator can compare numerically / temporally where appropriate and fall
/// back to string equality otherwise.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// An IRI (an `odrl:rightOperandReference` or a named resource/party/purpose).
    Iri(String),
    /// A plain or typed string literal (purpose codes, recipient ids, regions).
    Str(String),
    /// An `xsd:decimal`/`integer`/`double` numeric literal (for `odrl:count`, …).
    Num(f64),
    /// An `xsd:dateTime`/`xsd:date` literal, normalized to a comparable key.
    DateTime(String),
}

impl Value {
    /// The value as a string for equality / membership comparison.
    pub fn as_str(&self) -> &str {
        match self {
            Value::Iri(s) | Value::Str(s) | Value::DateTime(s) => s,
            Value::Num(_) => "",
        }
    }
}

impl Eq for Value {}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Iri(s) => write!(f, "<{s}>"),
            Value::Str(s) | Value::DateTime(s) => write!(f, "{s:?}"),
            Value::Num(n) => write!(f, "{n}"),
        }
    }
}

/// An ODRL duty (`odrl:duty`): an obligation that must be discharged for a
/// permission to be exercisable (the "usage-control kernel" pure access control
/// lacks). In the single-node base case a duty is modelled by its [`Action`] and
/// (optionally) its own constraints; the evaluator checks whether the request
/// context reports the duty as discharged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duty {
    /// The duty node IRI/blank-id (for justification).
    pub id: String,
    /// The action that must be performed (`odrl:action`), e.g.
    /// `odrl:anonymize`, `odrl:compensate`, a custom `proveAttestation`.
    pub action: Action,
    /// Constraints refining the duty (e.g. compensate `count = 5`). Carried for
    /// justification; the base case checks discharge by action.
    pub constraints: Vec<Constraint>,
}

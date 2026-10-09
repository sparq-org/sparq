//! Static (policy-vs-policy) ODRL analysis: **conflict** and **containment**
//! detection. [OPUS-4.8] sq-zabv.
//!
//! Where [`crate::evaluate`] answers *"may THIS request go through?"* against one
//! policy, this module answers two **request-free** questions about the policies
//! themselves — candidate #7 of `research/feature-research-odrl-policy.md`, on the
//! query-containment comparison semantics ([arXiv 2509.05139
//! §comparison](https://arxiv.org/html/2509.05139v1)):
//!
//! 1. **Conflict** — [`detect_conflicts`]: which permission/prohibition pairs
//!    *overlap* (a request could satisfy both)? Because a matching prohibition
//!    carves out a permission (deny-overrides — see [`crate::evaluate`]), an
//!    overlapping pair is exactly a request set the permission *appears* to grant
//!    but the prohibition forbids. This is the ODRL author's "did I prohibit
//!    something I also permitted?" lint.
//! 2. **Containment** — [`contains`]: does policy `outer` permit *everything*
//!    policy `inner` permits (query containment / refinement)? The
//!    requester-vs-provider check candidate #7 names: a requester's ask is
//!    acceptable iff the provider's offer **contains** it.
//!
//! ## Soundness contract (the honesty gate)
//!
//! Both verdicts are **sound, never over-claimed**. Constraint satisfiability and
//! query containment are undecidable in the general ODRL constraint language, so
//! this module reasons only about what it can *prove* from the rule structure and a
//! conservative, per-dimension constraint comparison:
//!
//! - A conflict is [`Overlap::Certain`] **only** when the structural attributes
//!   (action / target / assignee) prove an overlap AND the prohibition adds **no**
//!   constraint the permission lacks (so for every request the permission grants,
//!   the prohibition also fires). Otherwise it degrades to [`Overlap::Possible`]
//!   (the rules *might* overlap for some request, but we cannot prove they always
//!   do). A pair that provably never overlaps yields **no** conflict at all.
//! - Containment is [`Containment::Contains`] **only** when every `inner`
//!   permission is provably subsumed by some `outer` permission AND no `outer`
//!   prohibition could carve into that subsumption. A provable witness to
//!   non-containment yields [`Containment::NotContained`]; anything in between is
//!   [`Containment::Unknown`] — we **never** report `Contains` we cannot prove
//!   (that would be the fail-OPEN failure mode: claiming an ask is covered when it
//!   is not).

use crate::model::{Action, ConflictStrategy, Constraint, Operator, Policy, Rule, Value};
use crate::eval::{ODRL_PURPOSE, ODRL_RECIPIENT, ODRL_SPATIAL};

/// How strongly two rules overlap — the three-valued result of the conflict test.
/// [OPUS-4.8] sq-zabv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// The rules provably overlap for **every** request the permission grants: the
    /// structural attributes agree (or the prohibition is broader) AND the
    /// prohibition adds no constraint the permission lacks. The prohibition carves
    /// out the **whole** permission — a definite conflict the author should resolve.
    Certain,
    /// The rules *may* overlap for *some* request, but we cannot prove they always
    /// do (the prohibition carries a constraint whose joint satisfiability with the
    /// permission we do not decide). Reported so the conflict is not silently
    /// dropped — but honestly flagged as not-proven-total.
    Possible,
}

/// A detected permission/prohibition conflict: a permission whose granted requests
/// a prohibition (wholly or partly) carves out. [OPUS-4.8] sq-zabv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The conflicting permission's [`Rule::id`].
    pub permission_id: String,
    /// The conflicting prohibition's [`Rule::id`].
    pub prohibition_id: String,
    /// How strongly they overlap (see [`Overlap`]).
    pub overlap: Overlap,
    /// The action IRI the two rules overlap on — the prohibition's action when it
    /// subsumes (e.g. the `odrl:use` umbrella), else the shared action. `None` only
    /// if neither rule names an action (degenerate).
    pub action: Option<String>,
    /// The concrete target the conflict is about, when both rules pin (or the
    /// prohibition leaves open and the permission pins) one; `None` for an
    /// all-targets overlap.
    pub target: Option<String>,
}

/// Whether `outer` permits everything `inner` permits — the three-valued
/// containment / refinement verdict. [OPUS-4.8] sq-zabv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Containment {
    /// Proven: every request `inner` permits, `outer` also permits (and no `outer`
    /// prohibition could carve into that). `inner` is a refinement of `outer`.
    Contains,
    /// Proven NOT contained: `inner` has a permission that grants a request `outer`
    /// provably does not — a definite witness (different concrete target/action, or
    /// a strictly looser inner constraint dimension).
    NotContained,
    /// Neither could be proven — undecidable under the conservative comparison
    /// (e.g. an `outer` prohibition that *might* carve in, or constraints whose
    /// implication we do not decide). **Never** silently read as `Contains`.
    Unknown,
}

/// Detect every permission/prohibition pair in `policy` that overlaps — the ODRL
/// conflict lint. [OPUS-4.8] sq-zabv. See the module-level docs for the soundness
/// contract.
///
/// A pair is reported when the permission and prohibition *could* both apply to
/// some request (their actions are compatible AND their targets/assignees are
/// compatible). The [`Conflict::overlap`] is [`Overlap::Certain`] when the
/// prohibition carves out the **whole** permission (it adds no constraint the
/// permission lacks), else [`Overlap::Possible`]. Pairs that provably never overlap
/// are omitted. Conflict is strictly across the permission/prohibition divide — two
/// permissions (or two prohibitions) never conflict.
///
/// # Examples
///
/// ```
/// use sparq_policy::{detect_conflicts, parse_policy_str, Overlap};
/// let p = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ;
///   odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
///   odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
/// "#, "turtle").unwrap();
/// let conflicts = detect_conflicts(&p);
/// assert_eq!(conflicts.len(), 1);
/// assert_eq!(conflicts[0].overlap, Overlap::Certain);
/// ```
pub fn detect_conflicts(policy: &Policy) -> Vec<Conflict> {
    let mut out = Vec::new();
    for perm in &policy.permissions {
        for proh in &policy.prohibitions {
            if let Some(overlap) = rule_overlap(perm, proh) {
                out.push(Conflict {
                    permission_id: perm.id.clone(),
                    prohibition_id: proh.id.clone(),
                    overlap,
                    action: overlap_action(perm, proh),
                    target: perm.target.clone().or_else(|| proh.target.clone()),
                });
            }
        }
    }
    out
}

/// Decide whether sparq can faithfully honour `policy`'s declared `odrl:conflict`
/// conflict-resolution strategy — the fail-closed authorization guard the bridge
/// consults before it materialises **any** grant or deny. [OPUS-4.8] sq-ihqbl.
///
/// The bridge implements exactly one ODRL conflict strategy — `odrl:prohibit`
/// (deny-overrides): the session layer subtracts `∪ deny` from `∪ allow`, so a matching
/// prohibition already beats any permission. Any *other* declared strategy is one the
/// bridge cannot represent, so this returns `Err(reason)` — a **loud refusal** the
/// caller surfaces and fails closed on, instead of silently coercing the policy into
/// deny-overrides (which would mis-apply the author's intent — an authorization-
/// correctness hazard):
///
/// * [`ConflictStrategy::Perm`] (`odrl:perm`, permissions override prohibitions) —
///   unrepresentable by allow-minus-deny subtraction (a deny always wins). **Always
///   refused.**
/// * [`ConflictStrategy::Invalid`] (`odrl:invalid`, the ODRL default) — a conflicting
///   policy is void as a whole, which the bridge cannot represent. Refused **iff**
///   [`detect_conflicts`] finds a permission/prohibition conflict; an `Invalid` policy
///   with no detected conflict has nothing to void and is admissible.
/// * [`ConflictStrategy::Unknown`] — an `odrl:conflict` value that is not a recognised
///   ODRL `ConflictTerm`. **Always refused** (the engine has no semantics for it).
///
/// Admissible (`Ok`): a policy that declares [`ConflictStrategy::Prohibit`], or declares
/// no `odrl:conflict` at all (the bridge's operative default is deny-overrides — the one
/// strategy it implements). An unset default is treated as `prohibit`, **not** the ODRL
/// spec default of `invalid`: fully honouring `invalid` for every conflicting-yet-
/// undeclared policy would refuse the bridge's core deny-overrides use case. This
/// divergence is deliberate and documented (issue #1375, decided: keep deny-overrides —
/// fail-closed, never authorises what a prohibition forbids). See the crate README's
/// ODRL conformance note for the honest boundary.
///
/// # Examples
///
/// ```
/// use sparq_policy::{conflict_admissibility, parse_policy_str};
/// // `odrl:conflict odrl:perm` cannot be honoured → a loud refusal.
/// let p = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/p> a odrl:Set ; odrl:conflict odrl:perm ;
///   odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
///   odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
/// "#, "turtle").unwrap();
/// assert!(conflict_admissibility(&p).is_err());
/// ```
pub fn conflict_admissibility(policy: &Policy) -> Result<(), String> {
    match &policy.conflict {
        // Deny-overrides is exactly what the bridge implements; unset defaults to it.
        None | Some(ConflictStrategy::Prohibit) => Ok(()),
        Some(ConflictStrategy::Perm) => Err(
            "policy declares `odrl:conflict odrl:perm` (permissions override prohibitions), \
             which the bridge cannot represent (its `∪ allow ∖ ∪ deny` enforcement always \
             lets a deny win); refusing to materialise rather than silently enforce \
             deny-overrides"
                .to_owned(),
        ),
        Some(ConflictStrategy::Invalid) => {
            let conflicts = detect_conflicts(policy);
            if conflicts.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "policy declares `odrl:conflict odrl:invalid` (a conflicting policy is void \
                     as a whole) and has {} detected permission/prohibition conflict(s); the \
                     bridge cannot void a whole policy, so refusing to materialise any rule \
                     rather than honour its uncontested rules",
                    conflicts.len()
                ))
            }
        }
        Some(ConflictStrategy::Unknown(iri)) => Err(format!(
            "policy declares an unsupported `odrl:conflict` strategy <{}>; the engine has no \
             semantics for it, so refusing to materialise rather than silently ignore it",
            iri
        )),
    }
}

/// Does `outer` permit everything `inner` permits? See the module-level docs for
/// the soundness contract. [OPUS-4.8] sq-zabv.
///
/// Returns [`Containment::Contains`] only when **every** `inner` permission is
/// provably subsumed by some `outer` permission AND no `outer` prohibition could
/// carve into the subsuming permission; [`Containment::NotContained`] when an
/// `inner` permission provably grants a request `outer` does not; and
/// [`Containment::Unknown`] when neither is provable (e.g. an `outer` prohibition
/// might carve in). An `inner` with no permissions permits nothing and is contained
/// vacuously.
///
/// # Examples
///
/// ```
/// use sparq_policy::{contains, parse_policy_str, Containment};
/// let broad = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/b> a odrl:Set ; odrl:permission [ odrl:action odrl:use ] .
/// "#, "turtle").unwrap();
/// let narrow = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/n> a odrl:Set ;
///   odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
/// "#, "turtle").unwrap();
/// assert_eq!(contains(&broad, &narrow), Containment::Contains);
/// assert_eq!(contains(&narrow, &broad), Containment::NotContained);
/// ```
pub fn contains(outer: &Policy, inner: &Policy) -> Containment {
    let mut any_unknown = false;
    for inner_perm in &inner.permissions {
        match best_subsumption(outer, inner_perm) {
            Subsumption::Subsumed => {}
            Subsumption::NotSubsumed => return Containment::NotContained,
            Subsumption::Unknown => any_unknown = true,
        }
    }
    if any_unknown {
        Containment::Unknown
    } else {
        Containment::Contains
    }
}

/// Per-inner-permission subsumption verdict against the whole `outer` policy.
enum Subsumption {
    /// Some `outer` permission provably subsumes the inner one AND no `outer`
    /// prohibition could carve into it.
    Subsumed,
    /// No `outer` permission can subsume the inner one — a definite witness.
    NotSubsumed,
    /// An `outer` permission subsumes, but an `outer` prohibition might carve in
    /// (cannot prove it does not) — undecidable.
    Unknown,
}

/// The three-valued verdict for whether ONE outer permission subsumes ONE inner
/// permission. The distinction between `No` (a *proven* structural witness of
/// non-subsumption) and `Indeterminate` (compatible footprint, undecided constraint
/// refinement) is the load-bearing soundness boundary — see [`best_subsumption`].
enum SubsumeOne {
    /// Proven: every request `ip` grants, `op` grants.
    Yes,
    /// Proven NOT: a structural witness shows `op` denies a request `ip` grants
    /// (disjoint concrete action/target/assignee, or `op` pins an attribute `ip`
    /// leaves open).
    No,
    /// Compatible footprint but an undecided constraint — cannot prove either way.
    Indeterminate,
}

/// Find `outer`'s strongest verdict for one inner permission. Sound aggregation:
/// * if ANY outer permission **proves** subsumption ([`SubsumeOne::Yes`]) and no
///   outer prohibition could carve in → [`Subsumption::Subsumed`];
/// * else, if EVERY outer permission **proves** non-subsumption ([`SubsumeOne::No`])
///   → [`Subsumption::NotSubsumed`] (a real witness — the inner ask is reachable by
///   none of outer's permissions);
/// * else → [`Subsumption::Unknown`] (at least one outer permission is
///   `Indeterminate`, or a prohibition might carve in). We **never** report
///   `NotSubsumed` on a merely-undecided permission — that would over-claim
///   non-containment.
fn best_subsumption(outer: &Policy, inner_perm: &Rule) -> Subsumption {
    let mut any_indeterminate = false;
    let mut any_yes = false;
    for op in &outer.permissions {
        match permission_subsumes(op, inner_perm) {
            SubsumeOne::Yes => {
                any_yes = true;
                break;
            }
            SubsumeOne::Indeterminate => any_indeterminate = true,
            SubsumeOne::No => {}
        }
    }
    if any_yes {
        // A subsuming permission exists. If any outer prohibition could fire on a
        // request the inner permission grants, we cannot prove containment (the deny
        // would carve out part of what we just claimed to contain) → Unknown.
        if outer
            .prohibitions
            .iter()
            .any(|proh| prohibition_can_carve(proh, inner_perm))
        {
            return Subsumption::Unknown;
        }
        return Subsumption::Subsumed;
    }
    // No outer permission proved subsumption. NotSubsumed is only sound when EVERY
    // outer permission *proved* non-subsumption (no `Indeterminate` left over) — an
    // empty `outer.permissions` vacuously qualifies (nothing can subsume).
    if any_indeterminate {
        Subsumption::Unknown
    } else {
        Subsumption::NotSubsumed
    }
}

/// Does outer permission `op` subsume inner permission `ip` — i.e. is every request
/// `ip` grants also granted by `op`? Three-valued and **sound**: `Yes`/`No` are only
/// returned when *proven*; everything undecided is `Indeterminate`.
///
/// `op` ⊇ `ip` requires, on each axis, that `op` is **no narrower** than `ip`:
/// * action: `op` permits every action `ip` permits (equal IRIs, or `op` is the
///   `use` umbrella) — else a **proven** `No` (disjoint concrete actions, or `op`
///   non-`use` against a `use` `ip`);
/// * target / assignee: `op` is unrefined (`None`, = any) OR pins the same value
///   `ip` pins — else a **proven** `No` (`op` pins one and `ip` pins another, or `op`
///   pins one and `ip` leaves it open so `ip` reaches values `op` denies);
/// * constraints: every constraint `op` carries must be **implied** by some
///   constraint `ip` carries (so `ip` is at-least-as-restricted on that dimension).
///   An `op` constraint with no implying `ip` constraint means `op` *may* be
///   narrower, but we cannot prove `ip` actually reaches outside it →
///   `Indeterminate` (not `No`).
fn permission_subsumes(op: &Rule, ip: &Rule) -> SubsumeOne {
    // Structural axes give *proven* witnesses of non-subsumption.
    if !action_at_least_as_broad(&op.action, &ip.action) {
        return SubsumeOne::No;
    }
    if !attr_at_least_as_broad(op.target.as_deref(), ip.target.as_deref()) {
        return SubsumeOne::No;
    }
    if !attr_at_least_as_broad(op.assignee.as_deref(), ip.assignee.as_deref()) {
        return SubsumeOne::No;
    }
    // Structurally `op ⊇ ip`. Now the constraints, per `op` constraint `oc`:
    //  * some `ip` constraint **implies** `oc` (ip no looser on that dimension) → OK;
    //  * `ip` carries **no** constraint on `oc`'s dimension at all → `ip` is
    //    *unconstrained* there, so it provably reaches values outside `oc`'s bound
    //    (every ODRL constraint dimension admits >1 value) → a proven `No`;
    //  * `ip` constrains the dimension but we cannot prove implication → undecided
    //    (`ip` might still be a subset under a relation we do not model) →
    //    `Indeterminate`.
    let mut any_indeterminate = false;
    for oc in &op.constraints {
        if ip.constraints.iter().any(|ic| constraint_implies(ic, oc)) {
            continue; // ip refines this dimension at least as tightly.
        }
        if ip.constraints.iter().any(|ic| ic.left == oc.left) {
            // ip constrains the dimension but not provably within oc's bound.
            any_indeterminate = true;
        } else {
            // ip leaves this dimension wide open while op restricts it → ip reaches
            // outside op. Proven non-subsumption.
            return SubsumeOne::No;
        }
    }
    // [OPUS-4.8] sq-a0zef — the static analyser does not model compound
    // `LogicalConstraint`s. A compound constraint on `op` may make `op` strictly
    // narrower than its atomic+structural footprint suggests, so we cannot PROVE
    // `op ⊇ ip`: degrade a would-be `Yes` to `Indeterminate` (never over-claim
    // subsumption — fail-OPEN would be claiming `op` covers an `ip` request it might
    // actually carve out). A compound on `ip` only makes `ip` narrower, which can only
    // help subsumption, so it needs no guard here.
    if !op.logical_constraints.is_empty() {
        any_indeterminate = true;
    }
    if any_indeterminate {
        SubsumeOne::Indeterminate
    } else {
        SubsumeOne::Yes
    }
}

/// Is action `a` at least as broad as `b` — does `a` permit every action `b` does?
/// `use` permits everything, so `use ⊇ anything`; a specific action subsumes only
/// itself, and crucially does NOT subsume the `use` umbrella.
fn action_at_least_as_broad(a: &Action, b: &Action) -> bool {
    if is_use(a) {
        return true; // use permits everything b could.
    }
    if is_use(b) {
        return false; // b is the umbrella; a (non-use) cannot cover all of it.
    }
    a == b
}

fn is_use(a: &Action) -> bool {
    a == &Action::use_()
}

/// Is the structural attribute `outer` (target or assignee) at least as broad as
/// `inner`? `None` (= any) is broadest; otherwise both must pin the same value.
fn attr_at_least_as_broad(outer: Option<&str>, inner: Option<&str>) -> bool {
    match (outer, inner) {
        (None, _) => true,            // outer = any ⊇ anything inner pins (or any)
        (Some(_), None) => false,     // outer pins one, inner = any → outer narrower
        (Some(o), Some(i)) => o == i, // both pin → must be identical
    }
}

/// Does constraint `ic` (from the inner/narrower rule) **imply** constraint `oc`
/// (from the outer/broader rule)? Sound (conservative): true only when proven.
///
/// We decide this only when the two constrain the **same** `leftOperand`. The cases
/// we can prove:
/// * identical constraint → implies itself;
/// * `ic`'s bound is a *subset* of `oc`'s under compatible order operators (e.g.
///   inner `lt 2026-01-01` implies outer `lt 2027-01-01`; inner `eq v` implies
///   outer `lteq`/`gteq`/`isPartOf` that admit `v`).
///
/// Anything we do not decide returns `false` — so [`permission_subsumes`] treats an
/// un-implied outer constraint as "outer is (possibly) narrower" and does not claim
/// subsumption. Fail-safe.
fn constraint_implies(ic: &Constraint, oc: &Constraint) -> bool {
    if ic.left != oc.left {
        return false; // different dimensions — cannot relate.
    }
    // Identical (same op + same right value) always implies.
    if ic.operator == oc.operator && ic.right == oc.right {
        return true;
    }
    // A few sound, common refinements. `eq v` (inner) implies an outer bound that
    // admits v.
    if ic.operator == Operator::Eq || ic.operator == Operator::IsA {
        return outer_admits_value(&ic.right, oc);
    }
    // Same order operator, tighter inner bound implies looser outer bound.
    match (ic.operator, oc.operator) {
        // inner lt/lteq B1 implies outer lt/lteq B2 when B1 <= B2, except that an
        // inclusive inner bound under a strict outer one needs B1 < B2 (`lteq 5` admits 5,
        // which `lt 5` does not).
        (Operator::Lt | Operator::Lteq, Operator::Lt | Operator::Lteq) => bound_le(
            &ic.right,
            &oc.right,
            ic.operator == Operator::Lteq && oc.operator == Operator::Lt,
        ),
        // inner gt/gteq B1 implies outer gt/gteq B2 when B1 >= B2 (strictly, for an
        // inclusive inner bound under a strict outer one).
        (Operator::Gt | Operator::Gteq, Operator::Gt | Operator::Gteq) => bound_le(
            &oc.right,
            &ic.right,
            ic.operator == Operator::Gteq && oc.operator == Operator::Gt,
        ),
        _ => false,
    }
}

/// Does the outer constraint `oc` admit the single value `v` (an inner `eq v`)? Only
/// when the evaluator's own comparison is definitely True for `v` ([`atomic_status`]),
/// so an incomparable pair, a mixed-offset dateTime spelling, or any value the
/// evaluator would read as Unknown is never claimed. A negative operator (`neq`,
/// `isNoneOf`) on a dimension the evaluator widens by membership or subsumption
/// (recipient, purpose, spatial) is not claimed either: the request's evidence could
/// place `v` inside the excluded value.
///
/// [`atomic_status`]: crate::eval::atomic_status
fn outer_admits_value(v: &Value, oc: &Constraint) -> bool {
    let negative = matches!(oc.operator, Operator::Neq | Operator::IsNoneOf);
    let widened = [ODRL_RECIPIENT, ODRL_PURPOSE, ODRL_SPATIAL].contains(&oc.left.as_str());
    if negative && widened {
        return false;
    }
    crate::eval::atomic_status(v, oc.operator, &oc.right) == Some(true)
}

/// `a <= b` (`strict`: `a < b`) for two bound values by the evaluator's order; `false`
/// if incomparable.
fn bound_le(a: &Value, b: &Value, strict: bool) -> bool {
    match crate::eval::order(a, b) {
        Some(std::cmp::Ordering::Less) => true,
        Some(std::cmp::Ordering::Equal) => !strict,
        _ => false,
    }
}

/// Can prohibition `proh` carve out any request inner permission `ip` grants?
/// Sound *over*-approximation (we say "can" unless we can prove it cannot), because
/// a missed carve-out would be the fail-OPEN error: claiming containment a deny
/// breaks. We can prove it CANNOT carve only when the structural attributes are
/// disjoint (different concrete action, target, or assignee).
fn prohibition_can_carve(proh: &Rule, ip: &Rule) -> bool {
    // Disjoint action (both concrete and different, neither the umbrella) → cannot.
    if actions_disjoint(&proh.action, &ip.action) {
        return false;
    }
    // Disjoint concrete target → cannot.
    if attrs_disjoint(proh.target.as_deref(), ip.target.as_deref()) {
        return false;
    }
    // Disjoint concrete assignee → cannot.
    if attrs_disjoint(proh.assignee.as_deref(), ip.assignee.as_deref()) {
        return false;
    }
    // Otherwise the structural footprints intersect — conservatively, it can carve
    // (we do not try to prove a constraint makes the prohibition vacuous).
    true
}

/// Two actions are provably disjoint iff both are concrete (non-`use`) and unequal.
fn actions_disjoint(a: &Action, b: &Action) -> bool {
    !is_use(a) && !is_use(b) && a != b
}

/// Two structural attributes are provably disjoint iff both pin a value and differ.
/// A `None` (= any) overlaps with anything.
fn attrs_disjoint(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x != y)
}

/// Compute the action IRI a conflict overlaps on (the prohibition's when it is the
/// broader/umbrella action, else the shared action).
fn overlap_action(perm: &Rule, proh: &Rule) -> Option<String> {
    if is_use(&proh.action) {
        Some(proh.action.0.clone())
    } else {
        Some(perm.action.0.clone())
    }
}

/// The conflict test for one permission/prohibition pair. Returns `None` if they
/// provably never overlap; else the [`Overlap`] strength. [OPUS-4.8] sq-zabv.
fn rule_overlap(perm: &Rule, proh: &Rule) -> Option<Overlap> {
    // Provably-disjoint structural footprints → no conflict at all.
    if actions_disjoint(&perm.action, &proh.action)
        || attrs_disjoint(perm.target.as_deref(), proh.target.as_deref())
        || attrs_disjoint(perm.assignee.as_deref(), proh.assignee.as_deref())
    {
        return None;
    }
    // The footprints intersect. The carve-out is CERTAIN (covers the whole
    // permission) iff the prohibition adds no constraint the permission lacks — then
    // for every request the permission grants, the prohibition also fires. (Each
    // prohibition constraint must be implied by some permission constraint, OR the
    // permission is unconstrained on a dimension the prohibition restricts, in which
    // case we cannot prove the prohibition always fires → Possible.)
    //
    // [OPUS-4.8] sq-a0zef — a compound `LogicalConstraint` on the PROHIBITION is not
    // modelled here, and it may make the prohibition fire only conditionally; so a
    // prohibition carrying any compound constraint can never be proven to carve out the
    // *whole* permission → degrade to `Possible` (never over-claim `Certain`).
    let certain = proh.logical_constraints.is_empty()
        && proh
            .constraints
            .iter()
            .all(|pc| perm.constraints.iter().any(|qc| constraint_implies(qc, pc)));
    Some(if certain {
        Overlap::Certain
    } else {
        Overlap::Possible
    })
}

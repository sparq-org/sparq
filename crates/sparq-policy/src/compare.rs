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
//! this module claims only what it can *prove*, and anything else is undecided:
//!
//! - A conflict is [`Overlap::Certain`] **only** when the prohibition's action covers
//!   the permission's, it names every target and assignee the permission does, and
//!   each of its constraints appears identically on the permission (so for every
//!   request the permission grants, the prohibition also fires). Otherwise it is
//!   [`Overlap::Possible`]. Only a pair with disjoint concrete actions yields **no**
//!   conflict: unequal targets or assignees can meet through membership evidence.
//! - [`Containment::NotContained`] is backed by a concrete request the comparison
//!   builds from an unconstrained, duty-free `inner` permission, which [`decide`]
//!   grants under `inner` and not under `outer`.
//! - [`Containment::Contains`] is claimed **only** when neither side declares a party
//!   collection or carries a duty or compound constraint, and every `inner`
//!   permission has an `outer` one whose action permits its action, whose target and
//!   assignee are open or the same IRIs, and whose every constraint appears
//!   identically on it, with no `outer` prohibition on an overlapping action.
//! - Everything else is [`Containment::Unknown`] — we **never** report `Contains` we
//!   cannot prove (that would be the fail-OPEN failure mode: claiming an ask is
//!   covered when it is not).

use crate::eval::{decide, Request};
use crate::model::{Action, ConflictStrategy, Policy, Rule};
use crate::validate::ValidatedPolicy;

/// How strongly two rules overlap — the three-valued result of the conflict test.
/// [OPUS-4.8] sq-zabv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// The rules provably overlap for **every** request the permission grants: the
    /// prohibition's action, target and assignee cover the permission's AND each of
    /// its constraints appears identically on the permission. The prohibition carves
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
    /// Proven NOT contained: a concrete request built from an unconstrained,
    /// duty-free `inner` permission that [`decide`] grants under `inner` and not
    /// under `outer`.
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
/// Returns [`Containment::NotContained`] when a request built from an unconstrained,
/// duty-free `inner` permission is granted by [`decide`] under `inner` and not under
/// `outer`; [`Containment::Contains`] only for the narrow shape the module docs name;
/// and [`Containment::Unknown`] otherwise. An `inner` with no permissions permits
/// nothing and is contained vacuously.
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
pub fn contains(outer: &ValidatedPolicy, inner: &ValidatedPolicy) -> Containment {
    // The comparison assumes deny-overrides; a policy whose conflict strategy `decide`
    // refuses grants nothing, which the rule comparison does not model.
    if conflict_admissibility(outer).is_err() || conflict_admissibility(inner).is_err() {
        return Containment::Unknown;
    }
    // A non-containment verdict is backed by a concrete request: `decide` grants it
    // under `inner` and not under `outer`.
    if inner
        .permissions
        .iter()
        .filter_map(witness_request)
        .any(|r| decide(inner, &r).allow && !decide(outer, &r).allow)
    {
        return Containment::NotContained;
    }
    // Containment is proven only for the narrow shape below; everything else is
    // undecided. A declared party collection, a duty or a compound constraint on
    // either side depends on evidence or semantics the comparison does not model.
    let modelled = |p: &Policy| {
        p.party_collections.is_empty()
            && p.permissions
                .iter()
                .chain(&p.prohibitions)
                .all(|r| r.duties.is_empty() && r.logical_constraints.is_empty())
    };
    if !modelled(outer) || !modelled(inner) {
        return Containment::Unknown;
    }
    let covered = |ip: &Rule| {
        outer.permissions.iter().any(|op| permission_covers(op, ip))
            && !outer.prohibitions.iter().any(|proh| prohibition_can_carve(proh, ip))
    };
    if inner.permissions.iter().all(covered) {
        Containment::Contains
    } else {
        Containment::Unknown
    }
}

/// Reserved IRIs a witness request uses for an attribute the inner rule leaves open,
/// so no outer rule can pin it.
const WITNESS_PARTY: &str = "urn:sparq:compare:any-party";
const WITNESS_ASSET: &str = "urn:sparq:compare:any-asset";

/// The request `rule` most plainly grants: its own action, target and assignee (a
/// reserved IRI where it leaves one open), with no evidence and no membership. Only
/// built for a rule with no constraints or duties; whether it is granted is left to
/// `decide`.
fn witness_request(rule: &Rule) -> Option<Request> {
    let plain =
        rule.constraints.is_empty() && rule.logical_constraints.is_empty() && rule.duties.is_empty();
    plain.then(|| {
        Request::new(rule.action.0.clone())
            .on(rule.target.as_deref().unwrap_or(WITNESS_ASSET))
            .by(rule.assignee.as_deref().unwrap_or(WITNESS_PARTY))
    })
}

/// Does outer permission `op` grant every request inner permission `ip` grants? Proven
/// only when `op`'s action permits `ip`'s, `op` leaves the target and assignee open or
/// pins the same IRIs, and every constraint `op` carries also appears identically on
/// `ip` (so `decide` reads it the same way for both). Anything else is not claimed.
fn permission_covers(op: &Rule, ip: &Rule) -> bool {
    op.action.permits(&ip.action)
        && attr_at_least_as_broad(op.target.as_deref(), ip.target.as_deref())
        && attr_at_least_as_broad(op.assignee.as_deref(), ip.assignee.as_deref())
        && op.constraints.iter().all(|oc| ip.constraints.contains(oc))
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

fn is_use(a: &Action) -> bool {
    a == &Action::use_()
}

/// Can prohibition `proh` carve out any request inner permission `ip` grants?
/// Sound *over*-approximation (we say "can" unless we can prove it cannot), because
/// a missed carve-out would be the fail-OPEN error: claiming containment a deny
/// breaks. We can prove it CANNOT carve only when the actions are disjoint. Unequal
/// targets or assignees prove nothing: a request's membership evidence can make any
/// party or asset IRI a collection the other is part of.
fn prohibition_can_carve(proh: &Rule, ip: &Rule) -> bool {
    !actions_disjoint(&proh.action, &ip.action)
}

/// Two actions are provably disjoint iff both are concrete (non-`use`) and unequal.
fn actions_disjoint(a: &Action, b: &Action) -> bool {
    !is_use(a) && !is_use(b) && a != b
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
    // Only disjoint actions prove the rules never meet. Unequal targets or assignees
    // do not: a request's membership evidence can place one inside the other.
    if actions_disjoint(&perm.action, &proh.action) {
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
    // The prohibition must also cover the permission's whole action and name every
    // party and asset the permission does (unpinned, or the same IRI).
    let certain = proh.logical_constraints.is_empty()
        && proh.action.permits(&perm.action)
        && attr_at_least_as_broad(proh.target.as_deref(), perm.target.as_deref())
        && attr_at_least_as_broad(proh.assignee.as_deref(), perm.assignee.as_deref())
        && proh
            .constraints
            .iter()
            .all(|pc| perm.constraints.contains(pc));
    Some(if certain {
        Overlap::Certain
    } else {
        Overlap::Possible
    })
}

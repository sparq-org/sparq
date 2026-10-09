//! [OPUS-4.8] sq-h3uk — bridge the `sparq-policy` ODRL evaluator into the
//! `<urn:sparq:auth>` AUTH_GRAPH so the existing graph-level WAC/ACP enforcement
//! applies a matched ODRL [`Permission`](sparq_policy::Rule).
//!
//! # What this is (and is NOT)
//!
//! This is the **single-node** bridge of epic sq-3183 — a research-track surface,
//! NOT a production cutover and NOT the federated/ZK-disclosure path (that one is
//! gated on ZK soundness remediation). It composes two opt-in crates:
//!
//! - `sparq-policy` answers a usage-control question — *may this party USE this
//!   asset, for purpose P, until time T, with obligation O discharged?* — and
//!   returns a fail-closed [`Decision`](sparq_policy::Decision).
//! - `sparq-solid` enforces graph-level access: a principal × [`Mode`] → graph set
//!   is read out of the `<urn:sparq:auth>` view (see [`crate::AuthIndex`]).
//!
//! The bridge **materializes** a definite ODRL Permit as the very same
//! `principal auth:<mode> graphName` triples the existing enforcement already
//! understands, appending them into the AUTH_GRAPH. Net effect: an ODRL
//! `Permission` (action + constraints satisfied + duties discharged) becomes a
//! concrete WAC/ACP grant honoured by the current enforcement — **no new
//! enforcement engine**, the existing one is reused unchanged.
//!
//! # Fail-closed
//!
//! A grant is materialized **only** when [`sparq_policy::evaluate`] returns a
//! definite Permit (`decision.allow == true`) AND the requested ODRL action maps to
//! a concrete WAC/ACP [`Mode`] AND the request names a concrete WebID party + target
//! graph IRI. A Deny, an ambiguous evaluation, an unmapped action, or a missing
//! party/target materializes **nothing** — access is never widened on ambiguity.
//!
//! # Action → Mode mapping
//!
//! The ODRL request's `action` IRI is mapped to a WAC/ACP [`Mode`] by
//! [`action_to_mode`]. The mapping is deliberately conservative (a permit only ever
//! grants the *narrowest* mode the action denotes):
//!
//! | ODRL action (`odrl:`)                                   | WAC/ACP [`Mode`] |
//! |---------------------------------------------------------|------------------|
//! | `read`, `display`, `present`, `print`, `play`           | [`Mode::Read`]   |
//! | `append`                                                | [`Mode::Append`] |
//! | `modify`, `delete`, `write`                             | [`Mode::Write`]  |
//! | anything else (incl. the `odrl:use` umbrella)           | **unmapped → no grant** |
//!
//! The `odrl:use` umbrella is intentionally **not** mapped to a write/control mode:
//! `use` subsumes its sub-actions in the ODRL hierarchy (`read`/`write`/`modify`/… —
//! everything except the disjoint `transfer` ownership subtree, [OPUS-4.8] sq-euhr3),
//! so materializing it as a single WAC mode would have to pick the widest, violating
//! fail-closed. A caller that wants `use → Read` should request `odrl:read` explicitly
//! (a `use` permission in the policy still *grants* a `read` request — `odrl:use`
//! permits its sub-actions in the evaluator — so the bridge maps the **request**
//! action, which is concrete).
//!
//! ## SPARQL-query profile decision — [SONNET-4.6] sq-lrtc3.2
//!
//! A SPARQL query is represented by the standard `odrl:read` action; sparq does
//! **not** mint a profile-specific query action IRI. Query execution observes graph
//! content and therefore fits the existing `read` action and [`Mode::Read`] exactly,
//! while a new action would require every policy producer and ODRL processor to learn
//! sparq-specific vocabulary without providing a narrower enforcement mode.
//!
//! This contract does not map `odrl:use` transitively. The ODRL evaluator may use the
//! action hierarchy to decide that a `use` permission covers a concrete `read`
//! request, but materialization maps the **request action**, never the permission's
//! ancestor action. A request presented merely as `odrl:use` remains unmapped because
//! that umbrella also covers mutation actions; treating it as query/read would silently
//! narrow an ambiguous request and could grant the wrong capability. Callers must
//! present SPARQL query requests as `odrl:read`.
//!
//! # Prohibitions → `auth:deny<Mode>` (deny-overrides) — [OPUS-4.8] sq-w693
//!
//! A matched ODRL **Prohibition** is the dual of a Permit: it carves an action out
//! of access. [`materialize_prohibition`] materializes it as the explicit
//! `principal auth:deny<Mode> target` triple the enforcement already understands —
//! `auth:denyRead`/`denyWrite`/`denyAppend`/`denyControl`, via the SAME
//! [`action_to_mode`] mapping (the request's concrete action → the narrowest denied
//! mode). The existing session layer ([`crate::AuthIndex::accessible`]) computes
//! `∪ allow ∖ ∪ deny`, so a materialized deny **takes precedence over any allow
//! grant** for the same principal+target+mode — *deny-overrides*, the same conflict
//! resolution the ODRL evaluator applies (a matching prohibition overrides any
//! permission). No new enforcement engine: the deny path already existed
//! ([`Mode::from_pred`](crate::Mode) parses `deny*`); the bridge only emits the triples.
//!
//! A prohibition materializes a deny **only** when [`sparq_policy::matched_prohibition`]
//! reports it carves THIS request out (action permits + target/assignee agree +
//! constraints satisfied) AND the request action [`action_to_mode`]-maps AND the
//! request names a concrete party (WebID) + target graph IRI. An un-matched / unmapped
//! / partyless / targetless prohibition materializes **nothing** — fail-closed; a deny
//! is never widened on ambiguity (and certainly never silently dropped to widen access).
//!
//! [`materialize_policy`] composes the two: it materializes every applicable Permit
//! grant AND every matched-Prohibition deny for the request, so a policy carrying both
//! a permission and a prohibition on the same principal+target+mode emits both triples,
//! and the deny **wins** at enforcement time.
//!
//! # Deny RETRACTION on prohibition withdrawal — [OPUS-4.8] sq-2pcf
//!
//! The sq-dpk4 [`BridgeLedger`] replays each tracked materialization on
//! [`refresh`](BridgeLedger::refresh), dropping the ones that no longer hold. For an
//! *allow* grant the rule is simple: re-evaluate, and a non-Permit (withdrawn / lapsed
//! / now-Denied / **ambiguous**) emits nothing → the grant is dropped → access is GONE.
//! Dropping on ambiguity is the right (fail-closed) call for an allow.
//!
//! A materialized **deny** is the mirror image, and so its retraction rule must be the
//! mirror image too. A deny should be RETRACTED — restoring access — only when the
//! underlying ODRL Prohibition is genuinely **withdrawn or lapses**. If we reused the
//! allow rule (drop the deny whenever the prohibition no longer matches), an *ambiguous*
//! re-evaluation — a prohibition that still structurally names the request but carries a
//! constraint the refresh request supplies no evidence for — would silently retract the
//! deny and **restore access on missing evidence: fail-OPEN**. That is exactly the trap
//! this bead closes.
//!
//! So deny retraction consults [`sparq_policy::prohibition_status`], a three-valued
//! refinement of [`matched_prohibition`]:
//!
//! | [`ProhibitionStatus`]                       | meaning                                                     | deny action        |
//! |---------------------------------------------|-------------------------------------------------------------|--------------------|
//! | [`Applies`](ProhibitionStatus::Applies)     | a prohibition still carves the request out                  | **keep** (re-emit) |
//! | [`Ambiguous`](ProhibitionStatus::Ambiguous) | still structurally names it, but a constraint is unprovable | **keep** (re-emit) |
//! | [`Withdrawn`](ProhibitionStatus::Withdrawn) | no prohibition names it, or every one is *definitely* false | **retract** (drop) |
//!
//! "Definitely false" means the refresh request DID supply evidence for the dimension
//! and the comparison failed (e.g. a `dateTime < 2026-01-01` window with an actual time
//! of `2026-06-01` — the window has provably lapsed). Only then is the carve-out known
//! to be gone. A retracted deny composes with deny-overrides as expected: it may
//! re-expose an allow grant for the same principal+target+mode — correct, *because the
//! prohibition is genuinely gone*. Static (non-bridged) `auth:deny*` rules are never in
//! the ledger and so are never re-evaluated or retracted by this path.

use crate::authindex::{Mode, AUTHENTICATED, PUBLIC};
use crate::{AUTH_BRIDGED_GRAPH, AUTH_GRAPH, AUTH_NS, SOLIDX_NS};
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_policy::{
    conflict_admissibility, evaluate, matched_prohibition, parse_policy_str, prohibition_status,
    Operator, Policy, ProhibitionStatus, Request, Rule, Value,
};
use sparq_reason::n3::compiled::{compile, eval, intern_facts, CompiledRuleSet};
use std::fmt::Write as _;
use std::sync::OnceLock;

/// [OPUS-5] Stratum A0 — the operand/combinator-arity guard whose `odrlx:shape
/// odrlx:Ambiguous` marks stratum A's and C's satisfaction rules negate. See
/// `rules/odrl-a0.n3` for why the satisfaction rules cannot be sound without it.
const ODRL_A0: &str = include_str!("../rules/odrl-a0.n3");
const ODRL_A: &str = include_str!("../rules/odrl-a.n3");
const ODRL_B: &str = include_str!("../rules/odrl-b.n3");
const ODRL_C: &str = include_str!("../rules/odrl-c.n3");
const ODRL_D: &str = include_str!("../rules/odrl-d.n3");

/// The five ODRL strata (`rules/odrl-{a0,a,b,c,d}.n3`, in evaluation order) lowered to
/// the id-level compiled IR, once per process — the same `OnceLock` shape
/// `crate::materialize`'s `wac_rules`/`acp_rules` use after sq-zgbso.4.
///
/// [SONNET-4.6] sq-zgbso.5: compilation is deterministic over `const` rule text, so the
/// result is cached verbatim — including a failure, which is returned as an ordinary
/// `Err` (a rule leaving the compiled subset must surface as a materialize error and a
/// refusal, never a panic and never a silently empty auth view).
fn odrl_rules() -> Result<&'static [CompiledRuleSet; 5], String> {
    static RULES: OnceLock<Result<[CompiledRuleSet; 5], String>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            Ok([
                compile(ODRL_A0)?,
                compile(ODRL_A)?,
                compile(ODRL_B)?,
                compile(ODRL_C)?,
                compile(ODRL_D)?,
            ])
        })
        .as_ref()
        .map_err(|e| format!("ODRL rules: {}", e))
}

/// The ODRL namespace prefix (`odrl:`), re-exported for caller convenience.
pub const ODRL_NS: &str = sparq_policy::ODRL_NS;

/// Map an ODRL **request action** IRI to the WAC/ACP [`Mode`] it materializes as,
/// or `None` if the action has no faithful single-mode mapping (fail-closed: an
/// unmapped action never materializes a grant).
///
/// See the [module docs](self#action--mode-mapping) for the full table and the
/// rationale for leaving the `odrl:use` umbrella unmapped.
///
/// # Examples
///
/// ```
/// # use sparq_solid::odrl_bridge::action_to_mode;
/// # use sparq_solid::Mode;
/// assert_eq!(action_to_mode("http://www.w3.org/ns/odrl/2/read"), Some(Mode::Read));
/// assert_eq!(action_to_mode("http://www.w3.org/ns/odrl/2/modify"), Some(Mode::Write));
/// assert_eq!(action_to_mode("http://www.w3.org/ns/odrl/2/append"), Some(Mode::Append));
/// // the umbrella action and unknown actions are unmapped (no grant)
/// assert_eq!(action_to_mode("http://www.w3.org/ns/odrl/2/use"), None);
/// ```
pub fn action_to_mode(action_iri: &str) -> Option<Mode> {
    let local = action_iri.strip_prefix(ODRL_NS)?;
    Some(match local {
        // Read-family: observe/render the resource's content.
        "read" | "display" | "present" | "print" | "play" => Mode::Read,
        // Append: add without removing (the strictly weaker write).
        "append" => Mode::Append,
        // Write-family: mutate/replace/remove the resource's content.
        "modify" | "delete" | "write" => Mode::Write,
        // `use` (umbrella) and everything else: no faithful single-mode mapping.
        _ => return None,
    })
}

/// The `auth:` view predicate a [`Mode`] grant is materialized under — the SAME
/// predicate the WAC/ACP rules emit and [`crate::AuthIndex`] reads
/// (`auth:read|write|append|control`).
fn mode_predicate(mode: Mode) -> &'static str {
    match mode {
        Mode::Read => "read",
        Mode::Write => "write",
        Mode::Append => "append",
        Mode::Control => "control",
    }
}

/// The `auth:deny<Mode>` view predicate a [`Mode`] DENY is materialized under — the
/// SAME predicate [`crate::AuthIndex`] parses into its deny map
/// (`auth:denyRead|denyWrite|denyAppend|denyControl`), so a materialized deny is
/// subtracted from the allow set by the existing `∪ allow ∖ ∪ deny` enforcement.
/// [OPUS-4.8] sq-w693.
fn deny_predicate(mode: Mode) -> &'static str {
    match mode {
        Mode::Read => "denyRead",
        Mode::Write => "denyWrite",
        Mode::Append => "denyAppend",
        Mode::Control => "denyControl",
    }
}

/// What the bridge decided and (on a Permit / matched Prohibition) materialized.
///
/// [`materialize_permission`] sets the `granted` / `grant_triple` fields;
/// [`materialize_prohibition`] sets the `prohibited` / `deny_triple` fields
/// ([OPUS-4.8] sq-w693); [`materialize_policy`] may set both at once (a policy with a
/// permission AND a prohibition for the request — the deny wins at enforcement time).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeOutcome {
    /// Whether a definite Permit produced an allow grant.
    /// `false` ⇒ no allow triple was written to the AUTH_GRAPH (fail-closed).
    pub granted: bool,
    /// Whether a matched Prohibition produced a DENY. `false` ⇒ no `auth:deny*`
    /// triple was written (fail-closed). [OPUS-4.8] sq-w693.
    pub prohibited: bool,
    /// On a grant: the WAC/ACP mode the matched ODRL action mapped to. On a deny:
    /// the mode the prohibited action mapped to (`deny_triple`'s mode).
    pub mode: Option<Mode>,
    /// On a grant: the materialized `principal auth:<mode> graph` triple, as
    /// `(principal_iri, mode_predicate_iri, graph_iri)`, for audit/justification.
    pub grant_triple: Option<(String, String, String)>,
    /// On a deny: the materialized `principal auth:deny<Mode> graph` triple, as
    /// `(principal_iri, deny_predicate_iri, graph_iri)`. [OPUS-4.8] sq-w693.
    pub deny_triple: Option<(String, String, String)>,
    /// Whether the whole policy was **REFUSED** as unimplementable — the bridge cannot
    /// faithfully honour its declared `odrl:conflict` strategy, so it materialised
    /// NOTHING (no grant, no deny). Distinct from a plain deny (`granted == false`): a
    /// refusal is a loud, fail-closed rejection of the policy itself, not a rule that
    /// simply did not match. The reason is in [`reasons`](BridgeOutcome::reasons). See
    /// [`sparq_policy::conflict_admissibility`]. [OPUS-4.8] sq-ihqbl.
    pub refused: bool,
    /// Human-readable reason a grant/deny was NOT materialized (the ODRL decision's
    /// caveats, an unmapped action, a missing party/target, or a policy-level refusal).
    /// Empty on success.
    pub reasons: Vec<String>,
    /// On a STATEFUL `odrl:count` grant ([OPUS-4.8] sq-58mh, via
    /// [`crate::PodStore::materialize_odrl_permission_counted`]): the new consumed count
    /// (`1..=limit`) after this exercise atomically consumed one unit of budget. `None`
    /// for an uncounted permission, any non-counted bridge path, or a deny.
    #[cfg_attr(not(feature = "count-enforcement"), allow(rustdoc::broken_intra_doc_links))]
    pub consumed: Option<u64>,
    /// The exact auth-view triples this call materialized (allow grant, deny, and/or
    /// the conditional-grant head triples), for the bridge ledger to track so a later
    /// refresh can retract precisely these. Empty when nothing was materialized.
    /// [OPUS-4.8] sq-dpk4.
    pub(crate) emitted: Vec<[Term; 3]>,
}

impl BridgeOutcome {
    fn denied(reasons: Vec<String>) -> BridgeOutcome {
        BridgeOutcome { reasons, ..BridgeOutcome::default() }
    }

    /// A loud, fail-closed **refusal**: the policy's `odrl:conflict` strategy is one the
    /// bridge cannot faithfully honour, so nothing is materialised. [OPUS-4.8] sq-ihqbl.
    fn refused(reason: String) -> BridgeOutcome {
        BridgeOutcome { refused: true, reasons: vec![reason], ..BridgeOutcome::default() }
    }
}

/// Refuse (fail-closed) to materialise `policy` when its declared `odrl:conflict`
/// conflict-resolution strategy is one the bridge cannot faithfully honour — the guard
/// every materialise entry point runs FIRST. Returns `Some(refusal outcome)` to
/// short-circuit (materialising nothing), or `None` when the strategy is admissible
/// (deny-overrides — the one strategy the bridge implements). [OPUS-4.8] sq-ihqbl.
///
/// Silently coercing an unimplementable strategy (e.g. `odrl:perm`) into the bridge's
/// deny-overrides would mis-apply the policy author's intent — an authorization-
/// correctness hazard — so the bridge refuses loudly instead. See
/// [`sparq_policy::conflict_admissibility`] for the exact admissibility rules.
fn refuse_unimplementable_conflict(policy: &Policy) -> Option<BridgeOutcome> {
    conflict_admissibility(policy)
        .err()
        .map(|reason| BridgeOutcome::refused(format!("REFUSED (odrl:conflict): {}", reason)))
}

/// Evaluate `policy` against `request` and, **iff** the result is a definite Permit
/// for a mappable action with a concrete party + target, materialize the equivalent
/// `principal auth:<mode> graph` grant into the dataset's `<urn:sparq:auth>` view —
/// the triple the existing WAC/ACP enforcement ([`crate::AuthIndex`],
/// [`crate::PodStore::accessible`]) already honours.
///
/// The grant is **appended** to the current auth view: any WAC/ACP grants already
/// materialized there are preserved (this widens nothing they denied — it only adds
/// an allow triple, which the `∪ allow ∖ ∪ deny` semantics still subtract any
/// matching deny from). If no `<urn:sparq:auth>` graph exists yet, one is created
/// holding just the bridged grant.
///
/// # Fail-closed
///
/// Returns a [`BridgeOutcome`] with `granted == false` and writes **nothing** when:
/// the ODRL [`Decision`](sparq_policy::Decision) is a Deny (or any caveat blocked the
/// permission); the requested action does not [`action_to_mode`]-map; or the request
/// omits the party (assignee/WebID) or the target graph IRI. The grant principal is
/// the request's `party` (a concrete WebID) — an anonymous/partyless request never
/// materializes a grant, since a partyless grant would widen access to everyone.
///
/// # Note (PodStore observation)
///
/// Like [`crate::materialize_wac`], a direct call on a [`crate::PodStore`]'s `graph`
/// field does NOT rebuild its session index/cache. Use
/// [`crate::PodStore::materialize_odrl_permission`], which reindexes afterward.
///
/// # Examples
///
/// ```
/// use oxrdf::Term;
/// use sparq_solid::odrl_bridge::materialize_permission;
/// use sparq_policy::{parse_policy_str, Request};
///
/// // alice MAY read the target graph (a bare matching permission).
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/1> a odrl:Set ; odrl:permission [
///     odrl:action odrl:read ;
///     odrl:target <https://pod.ex/notes/n1> ;
///     odrl:assignee <https://alice.ex/card#me> ] .
/// "#, "turtle")?;
///
/// let mut graph = sparq_core::Graph::load_dataset(
///     "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#t> \"x\" <https://pod.ex/notes/n1> .",
///     "nquads")?;
/// let req = Request::new("http://www.w3.org/ns/odrl/2/read")
///     .on("https://pod.ex/notes/n1")
///     .by("https://alice.ex/card#me");
///
/// let out = materialize_permission(&mut graph, &pol, &req);
/// assert!(out.granted);
/// // a concrete WAC/ACP grant now lives in the auth view
/// assert!(graph.named.iter().any(|(name, _)| matches!(
///     name, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH)));
/// # Ok::<(), String>(())
/// ```
pub fn materialize_permission(
    graph: &mut Graph,
    policy: &Policy,
    request: &Request,
) -> BridgeOutcome {
    // 0. Refuse (fail-closed) an unimplementable odrl:conflict strategy BEFORE evaluating —
    //    a policy the bridge cannot faithfully honour materialises nothing. [OPUS-4.8] sq-ihqbl.
    if let Some(refusal) = refuse_unimplementable_conflict(policy) {
        return refusal;
    }
    // 1. ODRL evaluation — the single source of the allow/deny decision.
    let decision = evaluate(policy, request);
    if !decision.allow {
        // Deny / ambiguous → materialize NOTHING (fail-closed).
        return BridgeOutcome::denied(decision.unmet_constraints);
    }

    // 2. Action → Mode. An unmapped action (incl. the `use` umbrella) → no grant.
    let Some(mode) = action_to_mode(&request.action) else {
        return BridgeOutcome::denied(vec![format!(
            "ODRL action <{}> has no WAC/ACP mode mapping; no grant materialized",
            request.action
        )]);
    };

    // 3. Concrete party (WebID principal) + target graph required — a partyless or
    //    targetless grant would widen access (fail-closed).
    let Some(party) = request.party.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL Permit has no concrete party (assignee/WebID); no grant materialized".to_owned(),
        ]);
    };
    let Some(target) = request.target.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL Permit has no concrete target graph IRI; no grant materialized".to_owned(),
        ]);
    };

    // 4. Materialize `party auth:<mode> target` into the auth view.
    let pred = format!("{AUTH_NS}{}", mode_predicate(mode));
    let triple = append_grant(graph, party, &pred, target);

    BridgeOutcome {
        granted: true,
        mode: Some(mode),
        grant_triple: Some((party.to_owned(), pred, target.to_owned())),
        emitted: vec![triple],
        ..BridgeOutcome::default()
    }
}

/// Evaluate `policy`'s **prohibitions** against `request` and, **iff** a prohibition
/// matches (carves the request out — action permits + target/assignee agree +
/// constraints satisfied) for a mappable action with a concrete party + target,
/// materialize the equivalent `principal auth:deny<Mode> graph` DENY triple into the
/// dataset's `<urn:sparq:auth>` view. [OPUS-4.8] sq-w693.
///
/// The deny is **appended** to the current auth view (any grants/denies already there
/// are preserved). Because the session layer computes `∪ allow ∖ ∪ deny`
/// ([`crate::AuthIndex::accessible`]), this materialized deny **takes precedence over
/// any allow grant** for the same principal+target+mode — *deny-overrides*. The deny
/// path is honoured by the EXISTING enforcement unchanged ([`crate::Mode`]'s
/// `from_pred` already parses `auth:deny*`); this function only emits the triple.
///
/// Whether a prohibition carves THIS request out is decided by
/// [`sparq_policy::matched_prohibition`] — the same match test the ODRL evaluator
/// applies in its conflict step — NOT by `evaluate(...).allow == false`, which would
/// conflate a carve-out with a plain no-matching-permission deny.
///
/// # Fail-closed
///
/// Returns a [`BridgeOutcome`] with `prohibited == false` and writes **nothing** when:
/// no prohibition matches the request; the requested action does not [`action_to_mode`]
/// -map; or the request omits the party (assignee/WebID) or the target graph IRI.
/// A deny is never widened on ambiguity, and an unmappable carve-out is reported (not
/// silently dropped — dropping a deny would widen access).
///
/// # Examples
///
/// ```
/// use oxrdf::Term;
/// use sparq_solid::odrl_bridge::materialize_prohibition;
/// use sparq_solid::Mode;
/// use sparq_policy::{parse_policy_str, Request};
///
/// // alice is PROHIBITED from writing the target graph.
/// let pol = parse_policy_str(r#"
/// @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
/// <urn:pol/1> a odrl:Set ; odrl:prohibition [
///     odrl:action odrl:modify ;
///     odrl:target <https://pod.ex/notes/n1> ;
///     odrl:assignee <https://alice.ex/card#me> ] .
/// "#, "turtle")?;
///
/// let mut graph = sparq_core::Graph::load_dataset(
///     "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#t> \"x\" <https://pod.ex/notes/n1> .",
///     "nquads")?;
/// let req = Request::new("http://www.w3.org/ns/odrl/2/modify")
///     .on("https://pod.ex/notes/n1")
///     .by("https://alice.ex/card#me");
///
/// let out = materialize_prohibition(&mut graph, &pol, &req);
/// assert!(out.prohibited);
/// assert_eq!(out.mode, Some(Mode::Write));
/// // an explicit auth:denyWrite triple now lives in the auth view
/// assert!(graph.named.iter().any(|(name, _)| matches!(
///     name, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH)));
/// # Ok::<(), String>(())
/// ```
pub fn materialize_prohibition(
    graph: &mut Graph,
    policy: &Policy,
    request: &Request,
) -> BridgeOutcome {
    // 0. Refuse (fail-closed) an unimplementable odrl:conflict strategy first — under an
    //    unimplementable strategy we trust NONE of the policy's rules, not even a deny
    //    (e.g. `odrl:perm` means the PERMISSION should win). [OPUS-4.8] sq-ihqbl.
    if let Some(refusal) = refuse_unimplementable_conflict(policy) {
        return refusal;
    }
    // 1. Does a prohibition CARVE THIS REQUEST OUT? (Same match test the evaluator's
    //    conflict step uses — not `!decision.allow`, which over-fires on a plain
    //    no-permission deny.)
    if matched_prohibition(policy, request).is_none() {
        return BridgeOutcome::denied(vec![
            "no ODRL prohibition matches the request; no deny materialized".to_owned(),
        ]);
    }

    // 2. Action → Mode. An unmapped action (incl. the `use` umbrella) → no deny.
    //    Fail-closed: we do NOT widen the deny to a default mode, but we also must
    //    not silently drop it — the carve-out is reported in `reasons`.
    let Some(mode) = action_to_mode(&request.action) else {
        return BridgeOutcome::denied(vec![format!(
            "ODRL prohibition matched but action <{}> has no WAC/ACP mode mapping; \
             no deny materialized",
            request.action
        )]);
    };

    // 3. Concrete party (WebID) + target graph required (fail-closed): a partyless
    //    deny would be meaningless / a targetless deny ambiguous.
    let Some(party) = request.party.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL prohibition has no concrete party (assignee/WebID); no deny materialized"
                .to_owned(),
        ]);
    };
    let Some(target) = request.target.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL prohibition has no concrete target graph IRI; no deny materialized".to_owned(),
        ]);
    };

    // 4. Materialize `party auth:deny<Mode> target` into the auth view.
    let pred = format!("{AUTH_NS}{}", deny_predicate(mode));
    let triple = append_grant(graph, party, &pred, target);

    BridgeOutcome {
        prohibited: true,
        mode: Some(mode),
        deny_triple: Some((party.to_owned(), pred, target.to_owned())),
        emitted: vec![triple],
        ..BridgeOutcome::default()
    }
}

/// Materialize **both** sides of `policy` for `request`: the Permit allow grant (via
/// [`materialize_permission`]) AND the matched-Prohibition deny (via
/// [`materialize_prohibition`]), composing them into one [`BridgeOutcome`].
/// [OPUS-4.8] sq-w693.
///
/// A policy carrying both a permission and a prohibition for the same
/// principal+target+mode materializes **both** triples. The deny **wins** at
/// enforcement time — the session layer subtracts `∪ deny` from `∪ allow`
/// ([`crate::AuthIndex::accessible`]), so *deny-overrides* falls out of the existing
/// enforcement without any conflict logic here. Each side is independently
/// fail-closed: a side that does not produce a definite, mappable, concrete result
/// materializes nothing for that side.
///
/// The returned outcome carries `granted`/`grant_triple` from the Permit side and
/// `prohibited`/`deny_triple` from the Prohibition side; `mode` reflects the deny
/// mode when a deny was materialized (the operative decision under deny-overrides),
/// else the grant mode; `reasons` aggregates the caveats of whichever side(s) did not
/// materialize.
pub fn materialize_policy(graph: &mut Graph, policy: &Policy, request: &Request) -> BridgeOutcome {
    // Fail-closed FIRST on an unimplementable odrl:conflict strategy — materialise
    // nothing (neither side), rather than silently apply deny-overrides. [OPUS-4.8] sq-ihqbl.
    if let Some(refusal) = refuse_unimplementable_conflict(policy) {
        return refusal;
    }
    let allow = materialize_permission(graph, policy, request);
    let deny = materialize_prohibition(graph, policy, request);

    let mut reasons = allow.reasons;
    reasons.extend(deny.reasons);
    let mut emitted = allow.emitted;
    emitted.extend(deny.emitted);
    BridgeOutcome {
        granted: allow.granted,
        prohibited: deny.prohibited,
        // A refusal on either side (defence-in-depth; the top gate already short-circuits).
        refused: allow.refused || deny.refused,
        // Under deny-overrides the deny is the operative decision when present.
        mode: deny.mode.or(allow.mode),
        grant_triple: allow.grant_triple,
        deny_triple: deny.deny_triple,
        reasons,
        // The policy path is not the stateful-count path; no unit is consumed here.
        consumed: None,
        emitted,
    }
}

/// Append a single `subject predicate object` triple to the `<urn:sparq:auth>`
/// named graph, preserving the triples already there (the WAC/ACP grants), and
/// **mirror it into the bridged-provenance graph** ([`AUTH_BRIDGED_GRAPH`]) so the
/// triple is structurally marked as bridged (vs static) — see [`mirror_bridged`].
/// Returns the emitted triple so the caller can record it in its bridge ledger
/// ([OPUS-4.8] sq-dpk4). Idempotent: an identical grant is not duplicated.
fn append_grant(graph: &mut Graph, subject: &str, predicate: &str, object: &str) -> [Term; 3] {
    let s = Term::NamedNode(NamedNode::new_unchecked(subject));
    let p = Term::NamedNode(NamedNode::new_unchecked(predicate));
    let o = Term::NamedNode(NamedNode::new_unchecked(object));
    let triple = [s, p, o];
    append_bridged_triples(graph, std::slice::from_ref(&triple));
    triple
}

/// Append `new_triples` to BOTH the `<urn:sparq:auth>` enforcement view and the
/// `<urn:sparq:auth-bridged>` provenance graph, preserving existing triples in each
/// and skipping duplicates (idempotent). Mirroring into the provenance graph is what
/// lets a later refresh/static-re-materialization tell bridged triples apart from
/// static WAC/ACP grants without ever inspecting predicate shape. [OPUS-4.8] sq-dpk4.
fn append_bridged_triples(graph: &mut Graph, new_triples: &[[Term; 3]]) {
    extend_named_graph(graph, AUTH_GRAPH, new_triples);
    extend_named_graph(graph, AUTH_BRIDGED_GRAPH, new_triples);
}

/// Re-intern `name`'s existing triples plus `additions` (deduplicated) into a fresh
/// sub-graph dictionary and swap it in (matches `install_auth_view`'s rebuild shape).
fn extend_named_graph(graph: &mut Graph, name: &str, additions: &[[Term; 3]]) {
    let g_name = Term::NamedNode(NamedNode::new_unchecked(name));
    let mut terms: Vec<[Term; 3]> = match graph.named.iter().find(|(n, _)| *n == g_name) {
        Some((_, sub)) => crate::loader::graph_triples(sub),
        None => Vec::new(),
    };
    for t in additions {
        if !terms.contains(t) {
            terms.push(t.clone());
        }
    }
    install_triples(graph, name, terms);
}

/// Reset the `<urn:sparq:auth>` enforcement view to exactly `terms`, PRESERVING the
/// named graph's presence when `terms` is empty, the view already exists, AND
/// `preserve_empty_marker` says a static materialization actually produced it. The
/// view's PRESENCE is the "materialized" marker: `AclIndex::build` reports a retryable
/// [`crate::AclStatus::Unloaded`] (a 503 at the server) when it is absent, and the
/// static materializer deliberately installs an EMPTY view for a closure that grants
/// nothing — a definitive "materialized, no grants" deny (403). Routing this reset
/// through the plain [`install_triples`] (which drops an empty graph) silently turned
/// that definitive deny into a retryable `Unloaded` on every post-materialize ledger
/// reconcile (sq-37f1a). The converse must not happen either — the reset must not
/// INVENT the marker: a bridged grant creates the view without any static
/// materialization, so the graph's current presence alone cannot distinguish
/// "statically materialized, no grants" from "bridged-only, all grants retracted".
/// `preserve_empty_marker` (whether a static baseline was ever captured —
/// `BridgeLedger::static_baseline.is_some()`) carries that distinction: when `false`,
/// an empty reset REMOVES the view and the store returns to `Unloaded`.
fn reset_auth_view(graph: &mut Graph, terms: Vec<[Term; 3]>, preserve_empty_marker: bool) {
    if terms.is_empty() && preserve_empty_marker {
        let g_name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
        if let Some(slot) = graph.named.iter_mut().find(|(n, _)| *n == g_name) {
            slot.1 = Graph::from_parts(Dict::new(), Vec::new());
        }
        return;
    }
    install_triples(graph, AUTH_GRAPH, terms);
}

/// Replace `name`'s sub-graph with exactly `terms` (re-interned into a fresh dict).
/// When `terms` is empty the named graph is removed entirely (fail-closed: no empty
/// shell left behind that a reader could otherwise treat as an existing-but-empty view).
/// NOT for the `<urn:sparq:auth>` view's baseline reset — that must keep an existing
/// empty view present (see [`reset_auth_view`]).
fn install_triples(graph: &mut Graph, name: &str, terms: Vec<[Term; 3]>) {
    let g_name = Term::NamedNode(NamedNode::new_unchecked(name));
    if terms.is_empty() {
        graph.named.retain(|(n, _)| *n != g_name);
        return;
    }
    let mut dict = Dict::new();
    let ids: Vec<[sparq_core::dict::Id; 3]> = terms
        .iter()
        .map(|t| [dict.intern(&t[0]), dict.intern(&t[1]), dict.intern(&t[2])])
        .collect();
    let sub = Graph::from_parts(dict, ids);
    if let Some(slot) = graph.named.iter_mut().find(|(n, _)| *n == g_name) {
        slot.1 = sub;
    } else {
        graph.named.push((g_name, sub));
    }
}

/// The triples currently in a named graph (empty if absent).
fn named_graph_triples(graph: &Graph, name: &str) -> Vec<[Term; 3]> {
    let g_name = Term::NamedNode(NamedNode::new_unchecked(name));
    match graph.named.iter().find(|(n, _)| *n == g_name) {
        Some((_, sub)) => crate::loader::graph_triples(sub),
        None => Vec::new(),
    }
}

// ============================================================================
// [OPUS-4.8] sq-hiz4 — persist a FAITHFULLY-mappable ODRL constraint as an ACP
// re-checked condition (`auth:ConditionalGrant`) instead of freezing it into a
// one-shot materialization-time allow.
// ============================================================================

/// The `acl:` mode IRI a [`Mode`] is written as on a conditional grant's
/// `auth:mode` (the form [`crate::AuthIndex`] reads via `from_mode_iri`).
fn mode_iri(mode: Mode) -> &'static str {
    match mode {
        Mode::Read => "http://www.w3.org/ns/auth/acl#Read",
        Mode::Write => "http://www.w3.org/ns/auth/acl#Write",
        Mode::Append => "http://www.w3.org/ns/auth/acl#Append",
        Mode::Control => "http://www.w3.org/ns/auth/acl#Control",
    }
}

/// The ODRL `recipient` constraint left-operand IRI.
const ODRL_RECIPIENT: &str = "http://www.w3.org/ns/odrl/2/recipient";
/// The ODRL `assignee` constraint left-operand IRI (the party as a *constraint*,
/// distinct from the `odrl:assignee` rule attribute the evaluator already matches).
const ODRL_ASSIGNEE: &str = "http://www.w3.org/ns/odrl/2/assignee";
/// The ODRL `dateTime` constraint left-operand IRI — the request-time dimension.
/// [OPUS-4.8] sq-0q7n.
const ODRL_DATETIME: &str = "http://www.w3.org/ns/odrl/2/dateTime";

/// A faithfully-mappable `odrl:dateTime` validity window persisted onto an
/// `auth:ConditionalGrant` as live-clock bounds. [OPUS-4.8] sq-0q7n.
///
/// Both bounds are `xsd:dateTime` lexical strings (the constraint's right-operand,
/// verbatim) and **inclusive**, matching [`crate::AuthIndex`]'s `window_admits`:
/// `not_before` = the grant is inactive until this instant; `not_after` = inactive
/// after it. The session layer re-checks `Session::now` against the window per request,
/// so a lapsed window denies *this request* without waiting for a ledger refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TimeWindow {
    /// Lower bound (`odrl:dateTime gteq T` → "from T", inclusive). `None` = unbounded.
    not_before: Option<String>,
    /// Upper bound (`odrl:dateTime lteq T` → "until T", inclusive). `None` = unbounded.
    not_after: Option<String>,
}

impl TimeWindow {
    /// Whether this window constrains anything (else it is the always-open window).
    fn is_some(&self) -> bool {
        self.not_before.is_some() || self.not_after.is_some()
    }
}

/// What [`map_constraints_to_agents`] concluded about a permission's constraints.
enum AgentMapping {
    /// Every constraint maps faithfully to an agent-dimension matcher; persist a
    /// `ConditionalGrant` whose `auth:agent` re-checks each of these principals,
    /// with the `except` principals carved out via an ACP `noneOf` exception matcher
    /// (the "everyone-except-X" / `recipient neq X` shape). [OPUS-4.8] sq-5037.
    Faithful {
        /// Positive recipient principals (`eq`/`isA`/`isPartOf`/`isAnyOf`). Empty ⇒ no
        /// positive restriction → grant to `auth:Public` (everyone), narrowed only by
        /// `except`.
        agents: Vec<String>,
        /// Principals carved OUT (`recipient neq X` / `recipient isNoneOf <set>`) —
        /// each becomes an ACP `noneOf` exception matcher on the grant: a session
        /// matching the grant head is denied the grant if it is one of these. Empty ⇒
        /// no exception.
        except: Vec<String>,
        /// The live-clock validity window from a faithfully-mappable `odrl:dateTime`
        /// constraint (`gteq` → `not_before`, `lteq` → `not_after`). [OPUS-4.8] sq-0q7n.
        /// The always-open window (no `odrl:dateTime` constraint) is the default.
        window: TimeWindow,
    },
    /// At least one constraint has NO faithful ACP-condition analogue (purpose /
    /// `odrl:count` / a strict `odrl:dateTime` bound / an unrecognised operand): the rule
    /// MUST stay one-shot (fail-closed — never approximate an unmappable constraint with
    /// a looser persisted condition).
    Unmappable,
}

/// Inspect a matched permission's constraints and decide whether the WHOLE rule can
/// be persisted as re-checked ACP agent conditions.
///
/// **Faithful (→ agent matcher):** an `odrl:recipient`/`odrl:assignee` constraint
/// under `eq`/`isA` (the recipient IS this principal) or `isPartOf`/`isAnyOf` (the
/// recipient is a member of a static principal set — the evaluator matches both
/// operators as the same flat lexical set, sq-uaz85). The recipient-of-data is exactly
/// the session agent the ACP `auth:agent` head re-checks, so the persisted condition
/// has the SAME semantics — it just re-evaluates per session instead of being frozen.
///
/// **Faithful (→ noneOf exception):** an `odrl:recipient`/`odrl:assignee` constraint
/// under `neq` (the recipient is everyone EXCEPT the named party — the
/// "everyone-except-X" shape) or `isNoneOf` (everyone except the members of a static
/// set — the list-valued dual, one exception matcher per member; [FABLE-5] sq-5fkpp).
/// This maps to an ACP `noneOf`: the grant head is the positive recipient set (or
/// `auth:Public` if there is no positive constraint) with an `auth:exceptMatcher`
/// carving out each named party, re-checked per session by the same machinery WAC/ACP
/// `noneOf` already uses. [OPUS-4.8] sq-5037.
///
/// **Faithful (→ live-clock window):** an `odrl:dateTime` constraint under `lteq`
/// ("until T", inclusive → `auth:notAfter`) or `gteq` ("from T", inclusive →
/// `auth:notBefore`) with an `xsd:dateTime` right-operand. [OPUS-4.8] sq-0q7n — the
/// window is persisted onto the grant and re-checked against the session clock
/// (`Session::now`) per request, so a lapsed window denies *immediately* on the next
/// request instead of only on the next ledger refresh. Strict bounds (`lt`/`gt`) are
/// deliberately left **Unmappable** to avoid an inclusive/exclusive off-by-one against
/// the inclusive `auth:notBefore`/`auth:notAfter` semantics (the one-shot path still
/// enforces them, frozen).
///
/// **Unmappable (→ stay one-shot):** `odrl:purpose` (ACP sessions carry no purpose —
/// a client app is not a purpose-of-use, so mapping it to a client matcher would
/// over-grant), a STRICT `odrl:dateTime` bound (`lt`/`gt` — see above), `odrl:count`
/// (ACP is stateless — no usage counter), any unrecognised left-operand, and a
/// malformed set right-operand — an EMPTY member set under `isPartOf`/`isAnyOf`/
/// `isNoneOf`, or a numeric/dateTime operand under `isNoneOf` (the evaluator never
/// satisfies those — `set_negation_representable` — so persisting an exception for
/// them would widen access). Any one such constraint forces the whole rule one-shot.
///
/// **Unmappable (→ stay one-shot): a COMPOUND `odrl:LogicalConstraint`** ([OPUS-4.8]
/// sq-izzak — WIDENING FIX). A rule that carries ANY `odrl:and`/`odrl:or`/`odrl:xone`
/// (`rule.logical_constraints`) has no faithful single-head ACP analogue and MUST stay
/// one-shot, even when it has ZERO atomic constraints. A grant head is a UNION of
/// `auth:agent` allows, so: an `odrl:and` of recipient constraints is an INTERSECTION
/// (folding it as a union widens — wrong direction); an `odrl:or` may mix a recipient
/// operand with a non-recipient dimension (time/purpose) that has no head analogue; and
/// `odrl:xone` (exactly-one) has no ACP analogue at all. Over-approximating any compound
/// agent-restriction into the head would grant unlisted/anonymous agents — the very
/// widening this classification prevents. Before this fix the loop below examined only
/// `rule.constraints`, so a rule whose ONLY restriction was a compound constraint mapped
/// `Faithful` with an EMPTY recipient set → an `auth:Public` head (the compound
/// restriction silently DROPPED). Fail-closed: any `logical_constraints` forces the whole
/// rule to the one-shot path ([`materialize_permission`]/[`materialize_prohibition`]),
/// whose evaluator DOES enforce the compound constraint (frozen) — never dropping it.
fn map_constraints_to_agents(rule: &Rule) -> AgentMapping {
    // [OPUS-4.8] sq-izzak: a rule carrying any compound `odrl:LogicalConstraint` has no
    // faithful ACP-condition head — stay one-shot so the compound restriction is enforced
    // (frozen) rather than silently dropped by folding an empty recipient set to public.
    if !rule.logical_constraints.is_empty() {
        return AgentMapping::Unmappable;
    }
    let mut agents: Vec<String> = Vec::new();
    let mut except: Vec<String> = Vec::new();
    let mut window = TimeWindow::default();
    for c in &rule.constraints {
        // [OPUS-4.8] sq-0q7n: a faithfully-mappable dateTime window → live-clock bounds.
        if c.left == ODRL_DATETIME {
            let Value::DateTime(t) = &c.right else {
                // a non-dateTime right-operand on a dateTime constraint is malformed.
                return AgentMapping::Unmappable;
            };
            match c.operator {
                // request time ≤ T → "until T" (inclusive upper bound).
                Operator::Lteq => {
                    // Two upper bounds → keep the EARLIER (tightest) — fail-closed.
                    set_tighter(&mut window.not_after, t, /*keep_earlier=*/ true);
                }
                // request time ≥ T → "from T" (inclusive lower bound).
                Operator::Gteq => {
                    // Two lower bounds → keep the LATER (tightest) — fail-closed.
                    set_tighter(&mut window.not_before, t, /*keep_earlier=*/ false);
                }
                // strict bounds have no inclusive window analogue → one-shot.
                _ => return AgentMapping::Unmappable,
            }
            continue;
        }
        if c.left != ODRL_RECIPIENT && c.left != ODRL_ASSIGNEE {
            // purpose / count / anything else → no faithful condition.
            return AgentMapping::Unmappable;
        }
        match c.operator {
            // recipient IS this principal (identity) → one agent matcher.
            Operator::Eq | Operator::IsA => match &c.right {
                Value::Iri(s) | Value::Str(s) => agents.push(s.clone()),
                // a numeric/dateTime recipient is malformed → fail-closed.
                _ => return AgentMapping::Unmappable,
            },
            // recipient ∈ {a|b|c} (static set) → one agent matcher per member.
            // `odrl:isAnyOf` is evaluated by sparq-policy exactly as the flat
            // `isPartOf` lexical set (sq-uaz85), so it maps identically. An EMPTY
            // set (incl. a numeric operand, whose lexical form here is empty) is
            // unsatisfiable under both operators → fail-closed. [FABLE-5] sq-5fkpp.
            Operator::IsPartOf | Operator::IsAnyOf => {
                let members = set_members(c.right.as_str());
                if members.is_empty() {
                    return AgentMapping::Unmappable;
                }
                agents.extend(members);
            }
            // recipient ≠ X (everyone EXCEPT X) → an ACP noneOf exception matcher
            // carving out X from the grant. A numeric/dateTime right-operand is
            // malformed → fail-closed. [OPUS-4.8] sq-5037.
            Operator::Neq => match &c.right {
                Value::Iri(s) | Value::Str(s) => except.push(s.clone()),
                _ => return AgentMapping::Unmappable,
            },
            // recipient ∉ {a|b|c} (`odrl:isNoneOf` — the list-valued dual of `neq`)
            // → one ACP noneOf exception matcher per member, carving each out of the
            // grant. The evaluator only ever satisfies `isNoneOf` for IRI/string
            // operands (`set_negation_representable`), so a numeric/dateTime operand
            // is malformed here → fail-closed (persisting an exception for a value
            // the evaluator flat-denies would WIDEN access). The degenerate EMPTY
            // set also stays one-shot: it excludes nothing, and promoting a (likely
            // malformed) empty operand to a bare re-checked grant is not worth the
            // widened persistence — the one-shot path still enforces it, frozen.
            // [FABLE-5] sq-5fkpp.
            Operator::IsNoneOf => match &c.right {
                Value::Iri(s) | Value::Str(s) => {
                    let members = set_members(s);
                    if members.is_empty() {
                        return AgentMapping::Unmappable;
                    }
                    except.extend(members);
                }
                _ => return AgentMapping::Unmappable,
            },
            // order operators (lt/gt/…) on a recipient are not meaningful → one-shot.
            _ => return AgentMapping::Unmappable,
        }
    }
    AgentMapping::Faithful { agents, except, window }
}

/// Split the compact `|`/space/comma right-operand set encoding into its members —
/// the SAME lexical set `sparq_policy::evaluate` matches for the flat
/// `isPartOf`/`isAnyOf`/`isNoneOf` base case (its `is_part_of`), so a bridged
/// matcher-per-member grant re-checks exactly the set the evaluator would.
/// Empty/whitespace-only members are dropped. [FABLE-5] sq-5fkpp.
fn set_members(right: &str) -> Vec<String> {
    right
        .split(['|', ' ', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Tighten an inclusive window bound with a new `xsd:dateTime` candidate `t`, compared
/// by the **real UTC instant** each denotes (`sparq_policy::cmp_datetime`, the SAME
/// offset-aware normalizer the evaluator uses — never raw lexical `str::cmp`, which
/// would pick the wrong bound for mixed-offset times). When two constraints set the
/// same side, keep the TIGHTER bound (the earlier upper bound / the later lower bound)
/// so the persisted window is the intersection — fail-closed, never wider than any one
/// constraint. An **unparseable** incoming `t` is treated as not-tighter (the existing
/// bound is kept) so a malformed candidate can never *widen* the window. [OPUS-4.8]
/// sq-0q7n.
fn set_tighter(slot: &mut Option<String>, t: &str, keep_earlier: bool) {
    use std::cmp::Ordering;
    match slot {
        None => *slot = Some(t.to_owned()),
        Some(cur) => {
            // Replace only when `t` is strictly tighter than `cur` by instant order;
            // an incomparable (unparseable) pair never replaces (fail-closed).
            let replace = match sparq_policy::cmp_datetime(t, cur.as_str()) {
                Some(Ordering::Less) => keep_earlier,
                Some(Ordering::Greater) => !keep_earlier,
                _ => false,
            };
            if replace {
                *cur = t.to_owned();
            }
        }
    }
}

/// Whether a recipient principal IRI is safe to write as an `auth:agent` head: it
/// must NOT smuggle the reserved pair encoding (`urn:sparq:` / `&client=`), or a
/// crafted recipient could impersonate a minted pair principal (defense in depth,
/// mirrors [`crate::loader::session_value_allowed`]). The session-layer check
/// already rejects such SESSIONS; this also keeps them out of the GRANT head.
fn recipient_principal_allowed(p: &str) -> bool {
    crate::loader::session_value_allowed(p)
}

/// Evaluate `policy` against `request` and, for the matched permission, EITHER
/// persist its recipient/assignee constraint as a re-checked ACP
/// `auth:ConditionalGrant` (so the granted agent is verified per session through the
/// real enforcement path) OR — when the permission carries a constraint with no
/// faithful ACP-condition analogue — fall back to the one-shot
/// [`materialize_permission`] (the constraint is checked once, at materialization).
///
/// # When a conditional grant is emitted (faithful)
///
/// All of the matched permission's constraints are `odrl:recipient`/`odrl:assignee`
/// constraints under `eq`/`isA`/`isPartOf`/`isAnyOf` (positive heads) or
/// `neq`/`isNoneOf` (noneOf exceptions), plus an inclusive `odrl:dateTime` window
/// (see the crate-internal `map_constraints_to_agents`). The emitted grant is:
///
/// ```text
/// <grant> a auth:ConditionalGrant ; auth:effect auth:Allow ;
///         auth:agent <recipient> ; auth:client auth:AnyClient ; auth:issuer auth:AnyIssuer ;
///         auth:mode acl:<Mode> ; auth:graph <target> .
/// ```
///
/// re-checked by [`crate::AuthIndex::accessible`]: a session whose agent is the
/// recipient is granted; any other agent (or anonymous) is denied — **without**
/// re-running the ODRL evaluator. A recipient *set* emits one grant per member (the
/// auth view unions the allows). When the rule has NO recipient constraint but carries
/// an `odrl:assignee` PROPERTY, the grant head is scoped to that one assignee ([OPUS-4.8]
/// sq-9n1q4 — a bare-assignee rule grants ONLY the assignee, never `auth:Public`). Only
/// when the rule has NO recipient constraint AND NO assignee is the grant head
/// `auth:Public` (any session) — legitimately public because the action/target/duties
/// were already satisfied at materialization.
///
/// A head naming an `odrl:PartyCollection` for which the request supplied `odrl:partOf`
/// evidence ([`sparq_policy::Request::with_party_membership`]) is expanded to one grant
/// per KNOWN member, keeping the collection IRI itself as a head ([SONNET-4.6] sq-rf9uv).
/// An ACP `auth:agent` head matches by identity and a session carries no membership
/// evidence, so the unexpanded collection head matched no member and the grant was dead
/// — an over-restriction. The expanded head is exactly the party set the evaluator would
/// admit under the supplied evidence, never wider. A head the policy declares a collection
/// ([`sparq_policy::Policy::party_collections`]) but for which this request supplied no
/// member falls back to one-shot instead: there is nothing to expand to, and freezing the
/// bare collection IRI would persist a grant that binds nobody.
///
/// # Fail-closed
///
/// - A Deny (prohibition override, unmet *unmappable* constraint, undischarged duty)
///   materializes **nothing** — exactly as the one-shot path.
/// - An unmapped action, or a missing target, materializes nothing.
/// - **Mixed constraints fail safe:** if ANY constraint is unmappable (`purpose`,
///   a strict `dateTime` bound, `count`, an order/malformed-operand recipient), the
///   WHOLE rule falls back to the one-shot path so the unmappable bound is still
///   enforced (frozen) — a persisted condition is emitted ONLY when every constraint
///   maps faithfully.
/// - A recipient IRI inside the reserved pair encoding is dropped from the grant head
///   (it could otherwise impersonate a minted pair principal).
/// - A `neq`/`isNoneOf` carve-out naming a party collection falls back to one-shot
///   ([SONNET-4.6] sq-rf9uv): a frozen `noneOf` matcher is matched by identity and cannot
///   re-check membership, so every member of the excluded collection would escape the
///   exception and keep access (fail-open). This does **not** depend on the request
///   supplying membership evidence — collection identity is read from the policy document
///   too ([`sparq_policy::Policy::party_collections`]), so a collection with zero supplied
///   member edges takes the same one-shot route.
///
/// Returns a [`BridgeOutcome`]; on a conditional grant `grant_triple` reports the
/// `(agent, auth:effect, graph)` of the FIRST emitted grant (audit anchor) and
/// `mode` the mapped mode. The free-function form does not reindex a [`crate::PodStore`]
/// — use [`crate::PodStore::materialize_odrl_permission_conditional`].
pub fn materialize_permission_conditional(
    graph: &mut Graph,
    policy: &Policy,
    request: &Request,
) -> BridgeOutcome {
    // 0. Refuse (fail-closed) an unimplementable odrl:conflict strategy first. [OPUS-4.8] sq-ihqbl.
    if let Some(refusal) = refuse_unimplementable_conflict(policy) {
        return refusal;
    }
    // 1. Action → Mode (shared with the one-shot path). Unmapped → no grant.
    let Some(mode) = action_to_mode(&request.action) else {
        return BridgeOutcome::denied(vec![format!(
            "ODRL action <{}> has no WAC/ACP mode mapping; no grant materialized",
            request.action
        )]);
    };

    // 2. Find the permission whose action/target match AND whose duties are
    //    discharged AND whose constraints map faithfully to agent conditions. The
    //    recipient constraint is NOT required to hold against the request party here
    //    — the persisted condition re-checks it per session. Prohibitions still
    //    override (deny-overrides), so consult the evaluator's prohibition verdict.
    if let Some(p) = matched_prohibition(policy, request) {
        return BridgeOutcome::denied(vec![format!(
            "prohibition {} matches the request (deny-overrides); no grant materialized",
            p.id
        )]);
    }
    let Some(target) = request.target.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL Permit has no concrete target graph IRI; no grant materialized".to_owned(),
        ]);
    };

    let mut fallback_reasons: Vec<String> = Vec::new();
    for rule in &policy.permissions {
        // Action + target must agree (assignee/recipient are handled as conditions).
        if !rule_action_target_match(rule, request, mode, target) {
            continue;
        }
        // Duties must be discharged at materialization (no ACP analogue → one-shot
        // semantics; an undischarged duty blocks this rule).
        if rule.duties.iter().any(|d| !request.discharged_duties.contains(&d.action.0)) {
            fallback_reasons.push(format!("permission {} has an undischarged duty", rule.id));
            continue;
        }
        match map_constraints_to_agents(rule) {
            AgentMapping::Faithful { agents: recipients, except, window } => {
                let agents = condition_agents(rule, &recipients);
                if agents.is_empty() {
                    // every recipient was reserved-encoded → fail-closed, nothing.
                    fallback_reasons.push(format!(
                        "permission {} recipients are all reserved-encoded; no grant",
                        rule.id
                    ));
                    continue;
                }
                // `recipient neq X` → an ACP noneOf exception carving out X. A reserved-
                // encoded exclusion would be silently un-enforceable as a matcher, which
                // would WIDEN the grant (X regains access) — fail-closed: drop the whole
                // rule to one-shot rather than emit an exception that cannot bite.
                let excepts = condition_excepts(&except);
                if excepts.len() != except.len() {
                    fallback_reasons.push(format!(
                        "permission {} has a reserved-encoded neq recipient; one-shot path",
                        rule.id
                    ));
                    continue;
                }
                // [SONNET-4.6] sq-rf9uv: a carve-out naming a party COLLECTION cannot be
                // frozen into `noneOf` heads — the matcher is matched by identity, so every
                // member escapes the exception and keeps access (fail-OPEN). One-shot
                // instead, where the evaluator does the real identity-or-membership check.
                // Collection-ness comes from the POLICY's own declaration as well as the
                // request's evidence, so a collection with zero supplied member edges is
                // caught here too.
                if heads_name_party_collection(policy, request, &excepts) {
                    fallback_reasons.push(format!(
                        "permission {} carves out a party collection (its members would \
                         escape a frozen noneOf); one-shot path",
                        rule.id
                    ));
                    continue;
                }
                // [SONNET-4.6] sq-rf9uv: a collection-valued assignee/recipient head is
                // matched by the evaluator via membership, but by ACP via identity — expand
                // it to the members the request evidenced, else the grant is dead. A head
                // the POLICY declares a collection but that this request supplied no member
                // for has nothing to expand to, so it goes one-shot rather than freeze a
                // grant that binds nobody.
                if agents.iter().any(|a| {
                    policy.party_collections.contains(a)
                        && request.party_collection_members(a).is_empty()
                }) {
                    fallback_reasons.push(format!(
                        "permission {} grants a party collection with no supplied membership \
                         evidence (a frozen head would bind no member); one-shot path",
                        rule.id
                    ));
                    continue;
                }
                let agents = expand_party_collection_heads(request, &agents);
                let (first, emitted) = append_conditional_grants(
                    graph, &agents, &excepts, &window, mode, target, GrantEffect::Allow,
                );
                return BridgeOutcome {
                    granted: true,
                    mode: Some(mode),
                    grant_triple: Some(first),
                    emitted,
                    ..BridgeOutcome::default()
                };
            }
            AgentMapping::Unmappable => {
                // This permission carries a constraint with no faithful condition
                // analogue → the one-shot path must check it (frozen) instead.
                fallback_reasons.push(format!(
                    "permission {} has a constraint with no faithful ACP condition; one-shot path",
                    rule.id
                ));
            }
        }
    }

    // 3. No faithfully-conditional permission applied → fall back to the EXISTING
    //    one-shot behaviour (which evaluates the unmappable constraints against the
    //    supplied request context and emits a frozen allow iff they hold).
    let out = materialize_permission(graph, policy, request);
    if !out.granted && out.reasons.is_empty() {
        return BridgeOutcome::denied(fallback_reasons);
    }
    out
}

/// Evaluate `policy`'s **prohibitions** against `request` and, for a prohibition whose
/// recipient/assignee constraints map faithfully to agent conditions, persist a
/// re-checked ACP **conditional deny** (`auth:ConditionalGrant` with `auth:effect
/// auth:Deny`) — the dual of [`materialize_permission_conditional`]. [OPUS-4.8] sq-4r70.
///
/// This is the constraint-CONDITIONAL deny: instead of freezing a deny at
/// materialization time (one-shot [`materialize_prohibition`]), the carve-out is
/// persisted as a condition the session layer re-checks per session. A prohibition
/// `recipient eq bob` materializes a deny that applies **only to bob's sessions**; a
/// `recipient neq bob` materializes a deny that applies to **everyone except bob** (an
/// ACP `noneOf` exception carving bob back IN to access). The deny composes with
/// deny-overrides via the SAME `∪ allow ∖ ∪ deny` enforcement
/// ([`crate::AuthIndex::accessible`]) — a conditional deny that applies to a session
/// removes the target from that session's accessible set, beating any allow.
///
/// # When a conditional deny is emitted (faithful)
///
/// All of the matched prohibition's constraints are `odrl:recipient`/`odrl:assignee`
/// constraints under `eq`/`isA`/`isPartOf`/`isAnyOf`/`neq`/`isNoneOf` (see the
/// crate-internal `map_constraints_to_agents`), the action [`action_to_mode`]-maps,
/// and the request names a target. The recipient
/// constraint is NOT required to hold against the request party — the persisted
/// condition re-checks it per session, exactly as the allow path.
///
/// # Fail-closed
///
/// - **Mixed / unmappable constraints fall back to one-shot:** if the prohibition
///   carries a constraint with no faithful ACP-condition analogue (`purpose`,
///   `dateTime`, `count`), the WHOLE rule falls back to [`materialize_prohibition`], so
///   the unmappable bound is still enforced (the one-shot deny is materialized iff the
///   prohibition currently matches — frozen). A persisted deny condition is emitted ONLY
///   when every constraint maps faithfully.
/// - An unmapped action or a missing target materializes nothing.
/// - A reserved-encoded recipient/exclusion cannot become an enforceable matcher; the
///   rule falls back to one-shot rather than emit a deny condition that cannot bite
///   (which would FAIL OPEN — a deny silently dropped widens access).
/// - A deny head naming a party collection falls back to one-shot ([SONNET-4.6] sq-rf9uv)
///   — the DENY dual of the allow path's member expansion. A frozen deny head is matched
///   against the session agent by IDENTITY, so a bare collection IRI binds no member at
///   all and a head frozen to the members this request happened to evidence lets every
///   other member escape the deny; the collection-member expansion is sound in the ALLOW
///   direction only.
///
/// ## Where collection identity comes from
///
/// The check is on collection IDENTITY, carried independently of any member list, so it
/// fires for a collection with **zero** supplied membership edges — the case that would
/// otherwise persist a bare collection IRI no member session can match. Two sources are
/// unioned (see the crate-internal `heads_name_party_collection`):
///
/// - [`Policy::party_collections`] — retained by [`sparq_policy::parse_policy`] from the
///   policy document: a subject typed `a odrl:PartyCollection`, or the object of an
///   `odrl:partOf` edge the document states.
/// - A non-empty `Request::party_collection_members` — this request's own `odrl:partOf`
///   evidence, for a caller that reads membership out of the state-of-the-world graph
///   rather than the policy.
///
/// A head neither source identifies is treated as a plain party IRI and frozen as one. In
/// ODRL 2.2 an IRI is a party collection because it is *declared* one, so a policy
/// document asserting neither the type nor a membership edge, materialized against a
/// request supplying no `odrl:partOf` evidence either, has given nothing in reach of the
/// bridge a reason to read that head as a collection. Hand-building a [`Policy`] instead
/// of parsing one leaves `party_collections` empty; populate it (or supply the request's
/// membership evidence) if the rule heads are collections.
///
/// Returns a [`BridgeOutcome`]; on a conditional deny `prohibited == true`,
/// `deny_triple` reports the `(agent, auth:effect, graph)` anchor of the first emitted
/// deny head and `mode` the mapped mode. The free-function form does not reindex a
/// [`crate::PodStore`] — go through a `materialize_*` method for that.
pub fn materialize_prohibition_conditional(
    graph: &mut Graph,
    policy: &Policy,
    request: &Request,
) -> BridgeOutcome {
    // 0. Refuse (fail-closed) an unimplementable odrl:conflict strategy first. [OPUS-4.8] sq-ihqbl.
    if let Some(refusal) = refuse_unimplementable_conflict(policy) {
        return refusal;
    }
    // 1. Action → Mode (shared with the one-shot path). Unmapped → no deny.
    let Some(mode) = action_to_mode(&request.action) else {
        return BridgeOutcome::denied(vec![format!(
            "ODRL action <{}> has no WAC/ACP mode mapping; no deny materialized",
            request.action
        )]);
    };
    let Some(target) = request.target.as_deref() else {
        return BridgeOutcome::denied(vec![
            "ODRL prohibition has no concrete target graph IRI; no deny materialized".to_owned(),
        ]);
    };

    // 2. Find a prohibition whose action/target match AND whose constraints map
    //    faithfully to agent conditions. The recipient/assignee constraint is NOT
    //    required to hold against the request party — the persisted condition re-checks
    //    it per session (the dual of the conditional allow path).
    let mut fallback_reasons: Vec<String> = Vec::new();
    for rule in &policy.prohibitions {
        if !rule_action_target_match(rule, request, mode, target) {
            continue;
        }
        match map_constraints_to_agents(rule) {
            AgentMapping::Faithful { agents: recipients, except, window } => {
                // [OPUS-4.8] sq-0q7n: a time-windowed DENY is fail-OPEN — outside the
                // window the deny would lapse and the carved-out party would regain
                // access. A live-clock window is safe only on an ALLOW (a lapsed allow
                // removes access — fail-closed). So a dateTime-windowed prohibition must
                // stay one-shot (the frozen deny is materialized iff it matches now).
                if window.is_some() {
                    fallback_reasons.push(format!(
                        "prohibition {} carries a dateTime window (fail-open as a live deny); one-shot path",
                        rule.id
                    ));
                    continue;
                }
                let agents = condition_agents(rule, &recipients);
                if agents.is_empty() {
                    fallback_reasons.push(format!(
                        "prohibition {} recipients are all reserved-encoded; no deny",
                        rule.id
                    ));
                    continue;
                }
                // [SONNET-4.6] sq-rf9uv: the DENY dual of the collection-head expansion is
                // fail-OPEN and must stay one-shot. A concrete deny head is matched by
                // identity and cannot re-check membership, so a bare collection IRI binds no
                // member session at all, and freezing it to the members this request
                // happened to evidence lets every other member escape the deny. The one-shot
                // path's evaluator does the real identity-or-membership check. Collection-
                // ness comes from the POLICY's own declaration as well as the request's
                // evidence, so a collection with zero supplied member edges is caught too.
                if heads_name_party_collection(policy, request, &agents) {
                    fallback_reasons.push(format!(
                        "prohibition {} denies a party collection (its members would escape a \
                         frozen deny head); one-shot path",
                        rule.id
                    ));
                    continue;
                }
                let excepts = condition_excepts(&except);
                if excepts.len() != except.len() {
                    // A reserved-encoded exclusion would silently re-admit the carved-out
                    // party to the DENY (i.e. they'd escape it) — fail-closed to one-shot.
                    fallback_reasons.push(format!(
                        "prohibition {} has a reserved-encoded neq recipient; one-shot path",
                        rule.id
                    ));
                    continue;
                }
                let (first, emitted) = append_conditional_grants(
                    graph, &agents, &excepts, &TimeWindow::default(), mode, target, GrantEffect::Deny,
                );
                return BridgeOutcome {
                    prohibited: true,
                    mode: Some(mode),
                    deny_triple: Some(first),
                    emitted,
                    ..BridgeOutcome::default()
                };
            }
            AgentMapping::Unmappable => {
                // A constraint with no faithful condition analogue (purpose / dateTime /
                // count) → the one-shot deny path must check it (frozen) instead.
                fallback_reasons.push(format!(
                    "prohibition {} has a constraint with no faithful ACP condition; one-shot path",
                    rule.id
                ));
            }
        }
    }

    // 3. No faithfully-conditional prohibition applied → fall back to the EXISTING
    //    one-shot deny (which checks the unmappable constraints against the supplied
    //    request context and emits a frozen `auth:deny*` iff the prohibition matches).
    let out = materialize_prohibition(graph, policy, request);
    if !out.prohibited && out.reasons.is_empty() {
        return BridgeOutcome::denied(fallback_reasons);
    }
    out
}

/// The principal-space `auth:agent` heads for a faithful recipient set, dropping any
/// reserved-encoded recipient.
///
/// When the recipient CONSTRAINT set is empty, the rule's `odrl:assignee` PROPERTY (a
/// distinct field on [`Rule`], not an `odrl:recipient`/`odrl:assignee` *constraint*
/// block) still scopes the rule to a single party: fold it in as the sole head so a
/// bare-assignee rule grants/denies ONLY that assignee. [OPUS-4.8] sq-9n1q4 — WIDENING
/// FIX: this slot previously ignored `rule.assignee` and defaulted an empty set to
/// `auth:Public`, so a permission scoped to one assignee granted EVERYONE (incl.
/// anonymous) and the prohibition dual over-denied everyone. The assignee is normalised
/// and reserved-encoding-filtered exactly like a recipient (so an all-reserved assignee
/// yields an empty set, which the callers already treat as fail-closed).
///
/// Only when there is NO recipient constraint AND NO assignee does the head fall back to
/// a single `auth:Public` head (any session matches) — a legitimately public rule whose
/// action/target/duties were already satisfied at materialization.
///
/// The heads returned here are identity-space and request-independent, and nothing here
/// (or in the parsed [`Policy`]) knows whether a head is an `odrl:PartyCollection` — that
/// is resolved afterwards, and ONLY against the request's own membership evidence, by
/// [`expand_party_collection_heads`] (ALLOW only). See [SONNET-4.6] sq-rf9uv for why the
/// DENY dual must not do the same, and *The limit of the collection check* on
/// [`materialize_prohibition_conditional`] for the un-evidenced case neither can detect.
fn condition_agents(rule: &Rule, recipients: &[String]) -> Vec<String> {
    if recipients.is_empty() {
        // [OPUS-4.8] sq-9n1q4: a bare `odrl:assignee` PROPERTY scopes the head to that
        // one party — it is NOT an unrestricted (auth:Public) rule.
        match &rule.assignee {
            Some(assignee) if recipient_principal_allowed(assignee) => {
                return vec![normalise_recipient_principal(assignee)];
            }
            // An all-reserved-encoded assignee → empty head → the caller fails closed
            // (an empty grant/deny head is never widened to auth:Public here).
            Some(_) => return Vec::new(),
            // No recipient AND no assignee → legitimately public.
            None => return vec![PUBLIC.to_owned()],
        }
    }
    recipients
        .iter()
        .filter(|r| recipient_principal_allowed(r))
        .map(|r| normalise_recipient_principal(r))
        .collect()
}

/// The principal-space carve-out heads for a `recipient neq X` exception set, dropping
/// any reserved-encoded principal (the caller treats a shortfall as fail-closed — a
/// dropped exclusion would re-admit the carved-out party). [OPUS-4.8] sq-5037.
fn condition_excepts(except: &[String]) -> Vec<String> {
    except
        .iter()
        .filter(|r| recipient_principal_allowed(r))
        .map(|r| normalise_recipient_principal(r))
        .collect()
}

/// Expand every head that names an `odrl:PartyCollection` into one head per KNOWN
/// member, keeping the collection IRI itself as a head. [SONNET-4.6] sq-rf9uv.
///
/// The evaluator matches a collection-valued `odrl:assignee`/`odrl:recipient` by
/// identity-OR-membership (`Request::party_matches` / its recipient twin), but an ACP
/// `auth:agent` head is matched against the session agent by identity ALONE — a session
/// carries no membership evidence, so a lone collection-IRI head matches NO member and
/// the persisted grant is dead (the over-restriction sq-rf9uv reports). Emitting
/// `{collection} ∪ members(collection)` makes the head the EXACT set of parties the
/// evaluator would admit under the membership evidence this request supplied.
///
/// **Soundness.** The expansion draws only on that caller-supplied evidence, so it can
/// never grant a party the evaluator would not; a member the request did not evidence is
/// simply absent (fail-closed under-grant, never a widening). With no evidence the result
/// is byte-for-byte the input heads. A member inside the reserved pair encoding is
/// dropped, exactly as [`condition_agents`] drops a reserved-encoded recipient.
///
/// This is sound for a positive ALLOW head only. The DENY dual and an ALLOW's `noneOf`
/// carve-out must NOT be expanded this way — see [`heads_name_party_collection`]. A
/// positive head that IS a known collection but has no evidenced member is likewise not
/// expandable here (the frozen head would bind nobody); the caller routes that case to
/// the one-shot path before calling this.
fn expand_party_collection_heads(request: &Request, heads: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for head in heads {
        if !out.contains(head) {
            out.push(head.clone());
        }
        for member in request.party_collection_members(head) {
            if !recipient_principal_allowed(member) {
                continue;
            }
            let m = normalise_recipient_principal(member);
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    out
}

/// Does any of `heads` name an `odrl:PartyCollection`? The fail-closed trigger for every
/// direction a frozen identity-matched head cannot faithfully carry a collection.
/// [SONNET-4.6] sq-rf9uv.
///
/// Collection IDENTITY is read from two independent sources, and **membership evidence is
/// not required for either**:
///
/// 1. [`Policy::party_collections`] — what the policy DOCUMENT says: a subject typed
///    `a odrl:PartyCollection`, or the object of an `odrl:partOf` edge the document
///    states. This is the source that closes the zero-member case: a real collection the
///    request supplied no edges for is still recognised, so its DENY head / carve-out
///    still routes to the sound path.
/// 2. A non-empty [`Request::party_collection_members`] — this request's own `odrl:partOf`
///    evidence. Strictly a lower bound (empty proves nothing on its own), kept as a second
///    source so a caller reading membership out of the state-of-the-world graph is covered
///    even when the policy document is silent.
///
/// **Why any collection head must leave the frozen path.** The head is fixed at
/// materialization and the ACP session re-check matches it against the session agent by
/// IDENTITY alone. For a **DENY** head, or an ALLOW's `noneOf` **carve-out**, that means
/// every member of the collection — evidenced or not — walks past the restriction and
/// keeps access, the exact widening the bridge exists to prevent. For a **positive ALLOW**
/// head with no evidenced member there is nothing to expand to, so the frozen grant binds
/// nobody and the permission is silently lost. In all three cases the rule falls back to
/// the one-shot path, whose evaluator performs the real identity-or-membership check
/// against each request (frozen, but sound). The one case that stays on the frozen path is
/// a positive ALLOW head WITH evidenced members, where
/// [`expand_party_collection_heads`] emits exactly the party set the evaluator would admit.
fn heads_name_party_collection(policy: &Policy, request: &Request, heads: &[String]) -> bool {
    heads.iter().any(|h| {
        policy.party_collections.contains(h) || !request.party_collection_members(h).is_empty()
    })
}

/// Map an ODRL recipient value to a principal-space `auth:agent` IRI. The two ODRL
/// "any recipient" sentinels are folded onto the auth principals the session layer
/// already understands; a concrete WebID passes through unchanged.
fn normalise_recipient_principal(r: &str) -> String {
    match r {
        "http://www.w3.org/ns/odrl/2/All" | "http://www.w3.org/ns/odrl/2/Group" => PUBLIC.to_owned(),
        "http://www.w3.org/ns/odrl/2/AllConnections" => AUTHENTICATED.to_owned(),
        _ => r.to_owned(),
    }
}

/// Does the rule's action permit `mode`'s action and its target agree with the
/// request? (Assignee/recipient are deferred to the persisted condition.)
fn rule_action_target_match(rule: &Rule, request: &Request, _mode: Mode, target: &str) -> bool {
    let req_action = sparq_policy::Action(request.action.clone());
    if !rule.action.permits(&req_action) {
        return false;
    }
    match &rule.target {
        Some(t) => t == target,
        None => true,
    }
}


/// The deontic force of a materialized `auth:ConditionalGrant` — selects the
/// `auth:effect` object emitted by [`append_conditional_grants`]. [OPUS-4.8] sq-4r70.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrantEffect {
    /// A conditional **allow** (`auth:effect auth:Allow`) — the recipient is granted.
    Allow,
    /// A conditional **deny** (`auth:effect auth:Deny`) — the dual: the matched
    /// session is denied, and the deny overrides any allow for the same
    /// principal+target+mode (the session layer subtracts `∪ deny` from `∪ allow`).
    Deny,
}

impl GrantEffect {
    /// The `auth:`-local effect object (`Allow` / `Deny`) and the grant-IRI key.
    fn iri_local(self) -> &'static str {
        match self {
            GrantEffect::Allow => "Allow",
            GrantEffect::Deny => "Deny",
        }
    }
}

/// Append `auth:ConditionalGrant` triples (allow OR deny — see `effect`) for each agent
/// head onto BOTH the `<urn:sparq:auth>` view and the bridged-provenance graph,
/// preserving existing triples. Returns the `(agent, auth:effect, graph)` audit anchor
/// of the first grant AND the full set of emitted head triples (for the bridge ledger).
/// [OPUS-4.8] sq-dpk4 / sq-4r70.
///
/// `excepts` are the principals carved OUT (the `recipient neq X` / "everyone-except"
/// shape — [OPUS-4.8] sq-5037). Each becomes an ACP `noneOf` exception: the grant gets
/// an `auth:exceptMatcher <m>` and the matcher `<m>` is materialized with the accept-set
/// facts the session layer reads (`solidx:acceptsAgentP <X>` + `solidx:acceptsClientP
/// auth:AnyClient`). [`crate::AuthIndex::cond_applies`] then suppresses the grant for any
/// session the matcher accepts — i.e. for `X` under any client — so `X` is denied while
/// every other session keeps the grant. This is EXACTLY the shape the WAC/ACP `noneOf`
/// rules (`rules/acp-c.n3`) emit, re-checked by the same code path.
///
/// `effect` selects the deontic force ([OPUS-4.8] sq-4r70): [`GrantEffect::Allow`]
/// emits `auth:effect auth:Allow` (the conditional grant); [`GrantEffect::Deny`] emits
/// `auth:effect auth:Deny` (the conditional deny — the dual). A conditional deny is
/// honoured by the SAME [`crate::AuthIndex::accessible`] path: a matching deny condition
/// adds the graph to the `denied` set, which is subtracted from `allowed`
/// (deny-overrides). The deny's grant IRI carries an `&effect=deny` key so it never
/// collides with an allow grant for the same `(agent, mode, graph, excepts)`.
fn append_conditional_grants(
    graph: &mut Graph,
    agents: &[String],
    excepts: &[String],
    window: &TimeWindow,
    mode: Mode,
    target: &str,
    effect: GrantEffect,
) -> ((String, String, String), Vec<[Term; 3]>) {
    let mut emitted: Vec<[Term; 3]> = Vec::new();

    let type_p = NamedNode::new_unchecked(RDF_TYPE);
    let cond_class = NamedNode::new_unchecked(format!("{AUTH_NS}ConditionalGrant"));
    let effect_p = NamedNode::new_unchecked(format!("{AUTH_NS}effect"));
    let effect_o = NamedNode::new_unchecked(format!("{AUTH_NS}{}", effect.iri_local()));
    let agent_p = NamedNode::new_unchecked(format!("{AUTH_NS}agent"));
    let client_p = NamedNode::new_unchecked(format!("{AUTH_NS}client"));
    let any_client = NamedNode::new_unchecked(crate::authindex::ANY_CLIENT);
    // [OPUS-4.8] sq-3jtd.6: a bridged grant carries no issuer constraint, so it spans the
    // issuer-dimension top — exactly as it spans the client top. The head MUST state this
    // explicitly: the session layer treats a conditional grant whose `auth:issuer` head is
    // absent as fail-closed (it never applies), matching the `auth:client` convention.
    let issuer_p = NamedNode::new_unchecked(format!("{AUTH_NS}issuer"));
    let any_issuer = NamedNode::new_unchecked(crate::authindex::ANY_ISSUER);
    let mode_p = NamedNode::new_unchecked(format!("{AUTH_NS}mode"));
    let mode_o = NamedNode::new_unchecked(mode_iri(mode));
    let graph_p = NamedNode::new_unchecked(format!("{AUTH_NS}graph"));
    let graph_o = NamedNode::new_unchecked(target);
    let except_p = NamedNode::new_unchecked(format!("{AUTH_NS}exceptMatcher"));
    let accepts_agent_p = NamedNode::new_unchecked(format!("{SOLIDX_NS}acceptsAgentP"));
    let accepts_client_p = NamedNode::new_unchecked(format!("{SOLIDX_NS}acceptsClientP"));
    let accepts_issuer_p = NamedNode::new_unchecked(format!("{SOLIDX_NS}acceptsIssuerP"));
    // [OPUS-4.8] sq-0q7n: the live-clock window predicates + the xsd:dateTime datatype the
    // bounds are typed with (so they round-trip as the same literal the evaluator parses).
    let not_before_p = NamedNode::new_unchecked(format!("{AUTH_NS}notBefore"));
    let not_after_p = NamedNode::new_unchecked(format!("{AUTH_NS}notAfter"));
    let xsd_datetime = NamedNode::new_unchecked(XSD_DATETIME);

    // One deterministic exception matcher per carved-out principal, shared across the
    // grant heads (its accept-set is the same regardless of which positive head it
    // attaches to). Each accepts (agent = X, client = any, issuer = any) → suppresses the
    // grant for X. [OPUS-4.8] sq-3jtd.6: the issuer accept (auth:AnyIssuer) makes the
    // carve-out span the issuer dimension too — without it the three-dimensional
    // matcher_accepts would never fire (it requires all three dimensions to accept).
    let except_matchers: Vec<(String, [Term; 3])> = excepts
        .iter()
        .map(|x| {
            let m = format!("urn:sparq:odrl-except?agent={}", sparq_reason::n3::encode_for_uri(x));
            (
                m,
                [
                    Term::NamedNode(NamedNode::new_unchecked(x)),
                    Term::NamedNode(any_client.clone()),
                    Term::NamedNode(any_issuer.clone()),
                ],
            )
        })
        .collect();

    let mut first: Option<(String, String, String)> = None;
    for agent in agents {
        // A deterministic grant IRI keyed on (agent, mode, graph, excepts) so re-
        // materializing the same condition is idempotent (no duplicate heads) and an
        // exception-bearing grant never collides with an unconditional one.
        let except_key = except_matchers
            .iter()
            .map(|(m, _)| sparq_reason::n3::encode_for_uri(m))
            .collect::<Vec<_>>()
            .join(",");
        // [OPUS-4.8] sq-0q7n: the window is part of the grant identity — two grants for
        // the same (agent, mode, graph, excepts, effect) but DIFFERENT windows must not
        // collide on the same IRI (one would silently overwrite the other's bounds).
        let grant_iri = format!(
            "urn:sparq:odrl-cond?agent={}&mode={}&graph={}&except={}&effect={}&notBefore={}&notAfter={}",
            sparq_reason::n3::encode_for_uri(agent),
            sparq_reason::n3::encode_for_uri(mode_iri(mode)),
            sparq_reason::n3::encode_for_uri(target),
            except_key,
            effect.iri_local(),
            sparq_reason::n3::encode_for_uri(window.not_before.as_deref().unwrap_or("")),
            sparq_reason::n3::encode_for_uri(window.not_after.as_deref().unwrap_or("")),
        );
        let g = Term::NamedNode(NamedNode::new_unchecked(&grant_iri));
        let agent_o = Term::NamedNode(NamedNode::new_unchecked(agent));
        let mut head = vec![
            [g.clone(), Term::NamedNode(type_p.clone()), Term::NamedNode(cond_class.clone())],
            [g.clone(), Term::NamedNode(effect_p.clone()), Term::NamedNode(effect_o.clone())],
            [g.clone(), Term::NamedNode(agent_p.clone()), agent_o],
            [g.clone(), Term::NamedNode(client_p.clone()), Term::NamedNode(any_client.clone())],
            [g.clone(), Term::NamedNode(issuer_p.clone()), Term::NamedNode(any_issuer.clone())],
            [g.clone(), Term::NamedNode(mode_p.clone()), Term::NamedNode(mode_o.clone())],
            [g.clone(), Term::NamedNode(graph_p.clone()), Term::NamedNode(graph_o.clone())],
        ];
        // [OPUS-4.8] sq-0q7n: the live-clock window bounds (xsd:dateTime literals),
        // re-checked against `Session::now` by `crate::AuthIndex::cond_applies`.
        if let Some(nb) = &window.not_before {
            head.push([
                g.clone(),
                Term::NamedNode(not_before_p.clone()),
                Term::Literal(Literal::new_typed_literal(nb.as_str(), xsd_datetime.clone())),
            ]);
        }
        if let Some(na) = &window.not_after {
            head.push([
                g.clone(),
                Term::NamedNode(not_after_p.clone()),
                Term::Literal(Literal::new_typed_literal(na.as_str(), xsd_datetime.clone())),
            ]);
        }
        // Wire each exception matcher onto the grant + materialize its accept-set facts.
        for (m, [agent_accept, client_accept, issuer_accept]) in &except_matchers {
            let m_node = Term::NamedNode(NamedNode::new_unchecked(m));
            head.push([g.clone(), Term::NamedNode(except_p.clone()), m_node.clone()]);
            head.push([m_node.clone(), Term::NamedNode(accepts_agent_p.clone()), agent_accept.clone()]);
            head.push([m_node.clone(), Term::NamedNode(accepts_client_p.clone()), client_accept.clone()]);
            head.push([m_node, Term::NamedNode(accepts_issuer_p.clone()), issuer_accept.clone()]);
        }
        for t in head {
            if !emitted.contains(&t) {
                emitted.push(t);
            }
        }
        if first.is_none() {
            first = Some((
                agent.clone(),
                format!("{AUTH_NS}effect"),
                target.to_owned(),
            ));
        }
    }

    append_bridged_triples(graph, &emitted);
    (first.expect("at least one agent head emitted"), emitted)
}

/// `rdf:type` IRI, for the conditional-grant class triple.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// `xsd:dateTime` datatype IRI, for the live-clock window bound literals. [OPUS-4.8]
/// sq-0q7n.
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

// ============================================================================
// [OPUS-4.8] sq-dpk4 — refresh / revocation of bridged ODRL grants.
//
// THE GAP this closes: the materialize_* functions above only ever APPEND. When the
// underlying ODRL policy changes — a permission withdrawn, a time window lapsed, a
// re-evaluation that now Denies — the previously-materialized grant stays in the auth
// view, so access that should be GONE persists. And a wholesale static WAC/ACP
// re-materialization (`install_auth_view`) rebuilds `<urn:sparq:auth>` and would drop
// every bridged grant. Both are reconciled by tracking each bridged materialization in
// a ledger and REPLAYING the ledger over a captured static baseline on demand.
//
// FAIL-CLOSED: refresh rebuilds the auth view as `static_baseline ∪ replay(ledger)`.
// An entry whose ODRL re-evaluation no longer yields a grant emits NOTHING on replay,
// so it is dropped — a withdrawn / lapsed / now-Denied grant LOSES access. The static
// baseline is captured independently (the exact `install_auth_view` output), so a
// static grant is never inspected, re-evaluated, or dropped by this path.
// ============================================================================

/// Which bridge entry point produced a tracked grant — replayed verbatim on refresh so
/// the SAME fail-closed evaluation re-runs (a withdrawn/lapsed/now-Denied policy emits
/// nothing → the entry is retracted). [OPUS-4.8] sq-dpk4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeKind {
    /// [`materialize_permission`] — a definite-Permit allow grant.
    Permission,
    /// [`materialize_prohibition`] — a matched-Prohibition deny.
    Prohibition,
    /// [`materialize_policy`] — both sides of a policy at once.
    Policy,
    /// [`materialize_permission_conditional`] — a re-checked conditional grant (or its
    /// one-shot fallback).
    PermissionConditional,
    /// [`materialize_prohibition_conditional`] — a re-checked conditional deny (or its
    /// one-shot fallback). [OPUS-4.8] sq-4r70.
    ProhibitionConditional,
    /// A STATEFUL `odrl:count` allow grant (via
    /// [`crate::PodStore::materialize_odrl_permission_counted`]). The count was consumed
    /// atomically at exercise; on refresh the grant is re-checked (read-only — never
    /// consumes again) against the usage state and RETRACTED once the budget is
    /// exhausted, so the bridged grant self-retracts on exhaustion. Only ever recorded
    /// under the `count-enforcement` feature. [OPUS-4.8] sq-58mh.
    #[cfg(feature = "count-enforcement")]
    PermissionCounted,
}

/// One tracked bridged materialization: the originating ODRL `(policy, request, kind)`
/// plus the auth triples it last emitted. The provenance that distinguishes a bridged
/// grant from a static WAC/ACP one (those are never in the ledger), and the unit of
/// retraction on refresh. [OPUS-4.8] sq-dpk4.
#[derive(Debug, Clone)]
pub struct BridgeEntry {
    /// The ODRL policy this grant was bridged from.
    pub policy: Policy,
    /// The request `(action, target, party, context, duties)` it was evaluated against.
    pub request: Request,
    /// Which bridge entry point produced it (replayed verbatim).
    pub kind: BridgeKind,
}

/// The ordered set of bridged materializations a [`crate::PodStore`] is tracking, plus
/// the static-baseline auth view captured at the last static (WAC/ACP) materialization.
///
/// # The model (sq-dpk4)
///
/// - Each successful bridge call records a [`BridgeEntry`] (the `(policy, request, kind)`).
/// - [`BridgeLedger::capture_static_baseline`] snapshots the EXACT `<urn:sparq:auth>`
///   triples produced by a static WAC/ACP materialization — the grants the refresh must
///   never touch. Capturing the install output directly (not by subtracting provenance)
///   means a static grant byte-identical to a bridged one still survives a refresh.
/// - [`BridgeLedger::refresh`] rebuilds the auth view as `static_baseline ∪
///   replay(valid entries)`: it resets `<urn:sparq:auth>` to the baseline, clears the
///   provenance graph, replays every entry, and DROPS the entries that re-evaluate to
///   nothing (withdrawn permission / lapsed window / now-Deny / now-matching prohibition).
///
/// Fail-closed throughout: a retracted entry loses access immediately, and on any
/// ambiguity the re-evaluation denies (the underlying evaluator is fail-closed), so the
/// entry is dropped rather than left stale.
#[derive(Debug, Clone, Default)]
pub struct BridgeLedger {
    entries: Vec<BridgeEntry>,
    /// The static (WAC/ACP) `<urn:sparq:auth>` triples to rebuild from on refresh.
    /// `None` until the first static materialization is captured (a store that only
    /// ever bridged grants has an empty static baseline — refresh starts from nothing).
    static_baseline: Option<Vec<[Term; 3]>>,
    /// The injected usage-counter store backing every [`BridgeKind::PermissionCounted`]
    /// entry's stateful `odrl:count` budget. Set on the first counted bridge call and
    /// re-used by [`refresh`](BridgeLedger::refresh) to re-check (read-only) whether a
    /// counted grant's budget is still available — so an exhausted grant is retracted.
    /// `None` ⇒ no counted grant was ever bridged. [OPUS-4.8] sq-58mh.
    #[cfg(feature = "count-enforcement")]
    count_store: Option<count::CounterHandle>,
}

impl BridgeLedger {
    /// A fresh, empty ledger.
    pub fn new() -> BridgeLedger {
        BridgeLedger::default()
    }

    /// The tracked bridged entries (for inspection/audit).
    pub fn entries(&self) -> &[BridgeEntry] {
        &self.entries
    }

    /// Record a successful bridge of `kind` for `(policy, request)`. Call this only when
    /// the corresponding `materialize_*` returned a materialized outcome (`granted` /
    /// `prohibited`). A no-op materialization is NOT tracked (nothing to retract).
    ///
    /// Idempotent on `(kind, target, party)`: re-recording the same grant slot REPLACES
    /// the tracked `(policy, request)` rather than appending a duplicate, so a caller can
    /// re-bridge with an updated policy/request and the ledger tracks exactly one entry
    /// per logical grant.
    pub fn record(&mut self, policy: &Policy, request: &Request, kind: BridgeKind) {
        let slot = (kind, request.target.clone(), request.party.clone());
        if let Some(e) = self.entries.iter_mut().find(|e| {
            (e.kind, e.request.target.clone(), e.request.party.clone()) == slot
        }) {
            e.policy = policy.clone();
            e.request = request.clone();
            return;
        }
        self.entries.push(BridgeEntry { policy: policy.clone(), request: request.clone(), kind });
    }

    /// Replace the tracked `(policy, request)` for the grant slot matching
    /// `(kind, request.target, request.party)` with the supplied (updated) ones, so the
    /// next [`BridgeLedger::refresh`] re-evaluates against the NEW policy / request
    /// context (a withdrawn permission, a lapsed window, a now-Deny). Returns `true` if a
    /// tracked entry matched. A no-match returns `false` and changes nothing — there is
    /// no bridged grant to refresh for that slot. [OPUS-4.8] sq-dpk4.
    pub fn update(&mut self, policy: &Policy, request: &Request, kind: BridgeKind) -> bool {
        let slot = (kind, request.target.clone(), request.party.clone());
        match self.entries.iter_mut().find(|e| {
            (e.kind, e.request.target.clone(), e.request.party.clone()) == slot
        }) {
            Some(e) => {
                e.policy = policy.clone();
                e.request = request.clone();
                true
            }
            None => false,
        }
    }

    /// Snapshot the current `<urn:sparq:auth>` triples as the static baseline — the
    /// grants a refresh rebuilds from and never re-evaluates. Call this right after a
    /// static WAC/ACP materialization (which produced the view) and BEFORE replaying any
    /// bridged grant on top.
    pub fn capture_static_baseline(&mut self, graph: &Graph) {
        self.static_baseline = Some(named_graph_triples(graph, AUTH_GRAPH));
    }

    /// Re-evaluate every tracked bridged grant against its (possibly changed) ODRL
    /// policy and rebuild the auth view as `static_baseline ∪ replay(still-valid
    /// entries)`, retracting the grants that no longer hold. [OPUS-4.8] sq-dpk4.
    ///
    /// Returns the number of entries RETRACTED (re-evaluated to nothing). The caller
    /// ([`crate::PodStore::refresh_odrl_grants`]) reindexes afterward so the change
    /// takes effect on the next `accessible`/`query_as`.
    ///
    /// # Fail-closed
    ///
    /// - The view is first reset to the static baseline (or REMOVED if none was ever
    ///   captured — a bridged-only store returns to the retryable, view-absent
    ///   [`crate::AclStatus::Unloaded`] state when every entry retracts) and the
    ///   provenance graph cleared, so NO stale bridged triple can survive unless an
    ///   entry re-emits it.
    /// - Each entry is replayed through its original `materialize_*` function, which
    ///   re-runs the fail-closed ODRL evaluation: a withdrawn permission, a lapsed time
    ///   window, a now-matching prohibition, or any ambiguous re-eval emits nothing, and
    ///   the entry is dropped.
    /// - A static grant is never in `entries`, never re-evaluated, and always present in
    ///   the baseline — refresh cannot widen or drop it.
    pub fn refresh(&mut self, graph: &mut Graph) -> usize {
        // 1. Reset the enforcement view to the captured static baseline, and clear ALL
        //    bridged provenance — nothing bridged survives unless an entry re-emits it.
        //    `None` vs `Some(empty)` matters: only a CAPTURED (static) baseline may keep
        //    an empty view present as the "materialized" marker — a bridged-only store
        //    (baseline never captured) whose entries all retract must return to the
        //    view-absent `Unloaded` state, not fake a definitive empty-view deny.
        let preserve_empty_marker = self.static_baseline.is_some();
        let baseline = self.static_baseline.clone().unwrap_or_default();
        reset_auth_view(graph, baseline, preserve_empty_marker);
        install_triples(graph, AUTH_BRIDGED_GRAPH, Vec::new());

        // 2. Replay each entry; keep only those that still materialize something. A
        //    counted entry ([OPUS-4.8] sq-58mh) is re-checked against the usage store
        //    (read-only) — exhausted ⇒ emits nothing ⇒ retracted (self-retract on
        //    exhaustion).
        #[cfg(feature = "count-enforcement")]
        let count_store = self.count_store.clone();
        let entries = std::mem::take(&mut self.entries);
        let before = entries.len();
        for entry in entries {
            #[cfg(feature = "count-enforcement")]
            let out = replay(graph, &entry, count_store.as_ref());
            #[cfg(not(feature = "count-enforcement"))]
            let out = replay(graph, &entry);
            if !out.emitted.is_empty() {
                self.entries.push(entry);
            }
        }
        before - self.entries.len()
    }

    /// Remember the usage-counter `store` backing this ledger's counted grants, so a
    /// later [`refresh`](BridgeLedger::refresh) can re-check (read-only) whether each
    /// [`BridgeKind::PermissionCounted`] entry's budget is still available. Idempotent;
    /// the latest store wins (a caller injecting a different store re-points refresh at
    /// it). [OPUS-4.8] sq-58mh.
    #[cfg(feature = "count-enforcement")]
    pub(crate) fn set_count_store(&mut self, store: &count::CounterHandle) {
        self.count_store = Some(store.clone());
    }
}

/// Re-run a tracked entry's original bridge function against the current graph,
/// re-evaluating its ODRL policy and re-emitting iff it still holds. [OPUS-4.8] sq-dpk4.
///
/// # Deny retraction is asymmetric to grant retraction — [OPUS-4.8] sq-2pcf
///
/// A grant replays straight through its `materialize_*`: any non-Permit (withdrawn /
/// lapsed / now-Denied / ambiguous) emits nothing → the grant is dropped → access is
/// GONE. That is fail-closed for an *allow*.
///
/// A **deny** must NOT use the same rule. [`materialize_prohibition`] re-emits only
/// when [`matched_prohibition`] currently matches; on *any* non-match it emits nothing,
/// so the deny would be retracted — restoring access. But `matched_prohibition` returns
/// no-match both when the prohibition is genuinely withdrawn AND when it is merely
/// *unprovable* (a constraint with no evidence in the refresh request). Retracting in
/// the unprovable case is fail-OPEN. So for the deny side we consult
/// [`prohibition_status`] and re-emit the deny on [`ProhibitionStatus::Ambiguous`] just
/// as on [`ProhibitionStatus::Applies`]; the deny is retracted ONLY on a definite
/// [`ProhibitionStatus::Withdrawn`].
#[cfg(not(feature = "count-enforcement"))]
fn replay(graph: &mut Graph, entry: &BridgeEntry) -> BridgeOutcome {
    match entry.kind {
        BridgeKind::Permission => materialize_permission(graph, &entry.policy, &entry.request),
        BridgeKind::Prohibition => refresh_prohibition(graph, &entry.policy, &entry.request),
        BridgeKind::Policy => refresh_policy(graph, &entry.policy, &entry.request),
        BridgeKind::PermissionConditional => {
            materialize_permission_conditional(graph, &entry.policy, &entry.request)
        }
        BridgeKind::ProhibitionConditional => {
            refresh_prohibition_conditional(graph, &entry.policy, &entry.request)
        }
    }
}

/// Re-run a tracked entry's original bridge function against the current graph. The
/// `count_store` variant ([OPUS-4.8] sq-58mh): a [`BridgeKind::PermissionCounted`] entry
/// is re-checked (read-only) against the usage store via
/// [`count::refresh_permission_counted`] — exhausted ⇒ emits nothing ⇒ retracted; every
/// other kind ignores the store.
#[cfg(feature = "count-enforcement")]
fn replay(
    graph: &mut Graph,
    entry: &BridgeEntry,
    count_store: Option<&count::CounterHandle>,
) -> BridgeOutcome {
    match entry.kind {
        BridgeKind::Permission => materialize_permission(graph, &entry.policy, &entry.request),
        BridgeKind::Prohibition => refresh_prohibition(graph, &entry.policy, &entry.request),
        BridgeKind::Policy => refresh_policy(graph, &entry.policy, &entry.request),
        BridgeKind::PermissionConditional => {
            materialize_permission_conditional(graph, &entry.policy, &entry.request)
        }
        BridgeKind::ProhibitionConditional => {
            refresh_prohibition_conditional(graph, &entry.policy, &entry.request)
        }
        BridgeKind::PermissionCounted => {
            count::refresh_permission_counted(graph, &entry.policy, &entry.request, count_store)
        }
    }
}

/// Re-evaluate a tracked **conditional deny** ([`materialize_prohibition_conditional`])
/// on refresh with the fail-closed deny-retraction rule (sq-2pcf). [OPUS-4.8] sq-4r70.
///
/// A faithfully-conditional deny re-checks its recipient/assignee carve-out per session
/// at enforcement time, so on refresh the question is only whether the prohibition still
/// *structurally* names the request (action/target) — which is exactly what
/// [`materialize_prohibition_conditional`] re-checks before re-emitting. A prohibition
/// withdrawn entirely (or whose action/target no longer match) re-emits nothing → the
/// deny condition is retracted → access restored (correct: the prohibition is gone).
///
/// When the tracked deny FELL BACK to one-shot (an unmappable `dateTime`/`purpose`/
/// `count` constraint), [`materialize_prohibition_conditional`]'s fallback runs the
/// one-shot [`materialize_prohibition`] — which would retract on an *unprovable*
/// constraint, FAIL-OPEN. So for the one-shot fallback we route through the deny-
/// retraction-aware [`refresh_prohibition`] (re-emit on Ambiguous, retract only on a
/// definite Withdrawn), exactly as the plain [`BridgeKind::Prohibition`] refresh does.
/// We detect the fallback case by whether ANY prohibition maps faithfully for the
/// request's action/target.
fn refresh_prohibition_conditional(
    graph: &mut Graph,
    policy: &Policy,
    request: &Request,
) -> BridgeOutcome {
    if prohibition_maps_faithfully(policy, request) {
        // Faithful conditional deny: the recipient carve-out is re-checked per session,
        // so re-emitting whenever the prohibition structurally names the request is
        // correct (and fail-closed: a withdrawn prohibition emits nothing → retracted).
        materialize_prohibition_conditional(graph, policy, request)
    } else {
        // One-shot fallback (an unmappable constraint): apply the deny-retraction rule
        // so an unprovable bound KEEPS the deny rather than restoring access (fail-open).
        refresh_prohibition(graph, policy, request)
    }
}

/// Whether SOME prohibition in `policy` whose action/target structurally name `request`
/// maps faithfully to agent conditions (so the conditional-deny path would emit a
/// re-checked condition rather than fall back to one-shot). [OPUS-4.8] sq-4r70.
fn prohibition_maps_faithfully(policy: &Policy, request: &Request) -> bool {
    let Some(mode) = action_to_mode(&request.action) else { return false };
    let Some(target) = request.target.as_deref() else { return false };
    policy.prohibitions.iter().any(|rule| {
        rule_action_target_match(rule, request, mode, target)
            && matches!(map_constraints_to_agents(rule), AgentMapping::Faithful { .. })
    })
}

/// Re-evaluate a tracked **Prohibition** deny on refresh with the fail-closed
/// deny-retraction rule (sq-2pcf): re-emit the `auth:deny*` triple unless the
/// prohibition is *definitely* withdrawn. [OPUS-4.8] sq-2pcf.
///
/// - [`ProhibitionStatus::Applies`] — a prohibition still carves the request out:
///   re-emit (identical to [`materialize_prohibition`]).
/// - [`ProhibitionStatus::Ambiguous`] — a prohibition still structurally names the
///   request but a constraint is unprovable for lack of evidence: **re-emit anyway**
///   (KEEP the deny — never restore access on an unprovable carve-out).
/// - [`ProhibitionStatus::Withdrawn`] — no prohibition names the request, or every one
///   that does is *definitely* false given the evidence: emit nothing → the deny is
///   retracted and access is restored (subject to deny-overrides composition).
fn refresh_prohibition(graph: &mut Graph, policy: &Policy, request: &Request) -> BridgeOutcome {
    match prohibition_status(policy, request) {
        // Genuinely gone → behave exactly like the materialize path (which now also
        // finds no match), emitting nothing so the deny is dropped.
        ProhibitionStatus::Withdrawn => materialize_prohibition(graph, policy, request),
        // Still applies → re-emit through the normal match path.
        ProhibitionStatus::Applies => materialize_prohibition(graph, policy, request),
        // Unprovable → KEEP the deny by re-emitting it directly (fail-closed).
        ProhibitionStatus::Ambiguous => reemit_deny(graph, request),
    }
}

/// Re-evaluate a tracked **Policy** (both sides) on refresh: the allow side keeps the
/// fail-closed grant semantics ([`materialize_permission`]); the deny side uses the
/// fail-closed deny-retraction rule ([`refresh_prohibition`]). Composed exactly as
/// [`materialize_policy`] so deny-overrides still holds. [OPUS-4.8] sq-2pcf.
fn refresh_policy(graph: &mut Graph, policy: &Policy, request: &Request) -> BridgeOutcome {
    let allow = materialize_permission(graph, policy, request);
    let deny = refresh_prohibition(graph, policy, request);

    let mut reasons = allow.reasons;
    reasons.extend(deny.reasons);
    let mut emitted = allow.emitted;
    emitted.extend(deny.emitted);
    BridgeOutcome {
        granted: allow.granted,
        prohibited: deny.prohibited,
        refused: allow.refused || deny.refused,
        mode: deny.mode.or(allow.mode),
        grant_triple: allow.grant_triple,
        deny_triple: deny.deny_triple,
        reasons,
        // The policy-refresh path is not the stateful-count path; no unit consumed.
        consumed: None,
        emitted,
    }
}

/// Re-emit the `principal auth:deny<Mode> target` deny for `request` WITHOUT consulting
/// the prohibition match — used by [`refresh_prohibition`] on an *ambiguous* re-eval to
/// KEEP a deny we cannot prove is gone (fail-closed). The deny triple is fully
/// determined by the request (party + action→mode + target), the same one
/// [`materialize_prohibition`] would emit when the prohibition matched. [OPUS-4.8] sq-2pcf.
///
/// If the request lacks a mappable action or a concrete party/target the deny cannot be
/// reconstructed; that emits nothing (and so retracts) — but such an entry could never
/// have been materialized in the first place (those are the exact fail-closed gates in
/// [`materialize_prohibition`]), so this branch is unreachable for a tracked deny.
fn reemit_deny(graph: &mut Graph, request: &Request) -> BridgeOutcome {
    let Some(mode) = action_to_mode(&request.action) else {
        return BridgeOutcome::denied(vec![
            "ambiguous prohibition re-eval but action no longer maps; deny not re-emitted"
                .to_owned(),
        ]);
    };
    let (Some(party), Some(target)) = (request.party.as_deref(), request.target.as_deref()) else {
        return BridgeOutcome::denied(vec![
            "ambiguous prohibition re-eval but request lost its party/target; deny not re-emitted"
                .to_owned(),
        ]);
    };
    let pred = format!("{AUTH_NS}{}", deny_predicate(mode));
    let triple = append_grant(graph, party, &pred, target);
    BridgeOutcome {
        prohibited: true,
        mode: Some(mode),
        deny_triple: Some((party.to_owned(), pred, target.to_owned())),
        emitted: vec![triple],
        ..BridgeOutcome::default()
    }
}

// ============================================================================
// [OPUS-4.8] sq-58mh — STATEFUL `odrl:count` enforcement wired THROUGH the bridge so a
// bridged grant SELF-RETRACTS on exhaustion (the sq-zi5w follow-up).
//
// ACP is stateless: there is no per-session usage counter to decrement, so the count
// limit cannot be a re-checked ACP *condition* the way `recipient`/`dateTime` are (a
// `ConditionalGrant` re-checks a STATELESS dimension of the session — agent / clock).
// A usage count lives OUTSIDE any one session. So count is wired through the EXISTING
// refresh/retraction ledger (sq-dpk4) instead:
//
//   - At EXERCISE, `materialize_permission_counted` calls sparq-policy's atomic
//     `evaluate_and_exercise`, which runs the real base decision AND consumes exactly
//     one unit of `odrl:count` budget on a grant. On a grant the equivalent one-shot
//     `principal auth:<mode> graph` allow is materialized (the same triple the existing
//     enforcement honours). On a base deny / exhausted / store-unavailable: NOTHING
//     (fail-closed — a denied request never burns budget).
//   - On REFRESH the tracked counted entry is re-checked READ-ONLY against the usage
//     store (`refresh_permission_counted` → `count::count_status`, which never consumes):
//     while budget remains the grant is re-emitted; once the budget is exhausted (or the
//     store is unavailable, or the base permission was withdrawn / now-prohibited) it
//     emits nothing → the grant is RETRACTED → access is GONE through the real
//     enforcement path. That is the "self-retracts on exhaustion" the bead asks for.
//
// FAIL-CLOSED throughout: an exhausted / unprovable / store-missing re-check retracts
// (never re-grants beyond a budget we cannot account for); refresh NEVER consumes a unit
// (so re-checking is idempotent and a refresh can never itself exhaust a budget).
// ============================================================================
#[cfg(feature = "count-enforcement")]
pub(crate) mod count {
    use super::{action_to_mode, append_grant, refuse_unimplementable_conflict, BridgeOutcome, AUTH_NS};
    use sparq_core::Graph;
    use sparq_policy::{
        count_status, evaluate, evaluate_and_exercise, CountStatus, Policy, Request,
        UsageCounterStore, ODRL_COUNT,
    };
    use std::sync::Arc;

    /// A cloneable, `Debug`-able handle to the injected usage-counter store, so the
    /// [`super::BridgeLedger`] (which derives `Debug`/`Clone`/`Default`) can hold one.
    /// [OPUS-4.8] sq-58mh.
    #[derive(Clone)]
    pub(crate) struct CounterHandle(pub(crate) Arc<dyn UsageCounterStore + Send + Sync>);

    impl std::fmt::Debug for CounterHandle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // The store interior is opaque (a trait object); name the handle only.
            f.write_str("CounterHandle(<UsageCounterStore>)")
        }
    }

    /// The `auth:` allow-view predicate a mode grant is materialized under — the SAME
    /// predicate [`super::materialize_permission`] uses and [`crate::AuthIndex`] reads.
    fn mode_predicate(mode: crate::Mode) -> &'static str {
        match mode {
            crate::Mode::Read => "read",
            crate::Mode::Write => "write",
            crate::Mode::Append => "append",
            crate::Mode::Control => "control",
        }
    }

    /// Evaluate `policy` against `request` and, on a grant, **atomically consume** one
    /// unit of any applicable `odrl:count` budget from `store`, materializing the
    /// equivalent `principal auth:<mode> graph` allow into the `<urn:sparq:auth>` view.
    /// [OPUS-4.8] sq-58mh.
    ///
    /// This is the stateful-count entry point of the bridge: it routes the allow/deny
    /// decision through sparq-policy's [`evaluate_and_exercise`] (which runs the REAL
    /// base [`evaluate`] decision and consumes exactly one count unit on a grant), so the
    /// first *N* exercises of an "at most *N*" permission materialize a grant and the
    /// *(N+1)*th denies — and a denied / exhausted / store-unavailable exercise burns no
    /// budget and materializes NOTHING (fail-closed).
    ///
    /// The grant is materialized **only** when the exercise was granted AND the request
    /// action [`action_to_mode`]-maps AND the request names a concrete party + target —
    /// the SAME fail-closed gates as [`super::materialize_permission`]. The returned
    /// [`BridgeOutcome`] carries `consumed = Some(n)` on a count-constrained grant
    /// (`None` for an uncounted permission or a deny).
    pub(crate) fn materialize_permission_counted(
        graph: &mut Graph,
        policy: &Policy,
        request: &Request,
        store: &dyn UsageCounterStore,
    ) -> BridgeOutcome {
        // 0. Refuse (fail-closed) an unimplementable odrl:conflict strategy BEFORE the
        //    atomic exercise — a refused policy must consume no budget. [OPUS-4.8] sq-ihqbl.
        if let Some(refusal) = refuse_unimplementable_conflict(policy) {
            return refusal;
        }
        // 1. The atomic, count-aware decision — the single source of allow/deny AND the
        //    one place a unit is consumed. A base deny / exhausted / store-unavailable
        //    returns allow == false and consumes nothing.
        let exercise = evaluate_and_exercise(policy, request, store);
        if !exercise.allow {
            return BridgeOutcome::denied(exercise.reasons);
        }

        // 2. Same fail-closed mapping gates as the one-shot allow path.
        let Some(mode) = action_to_mode(&request.action) else {
            return BridgeOutcome::denied(vec![format!(
                "ODRL action <{}> has no WAC/ACP mode mapping; no grant materialized",
                request.action
            )]);
        };
        let Some(party) = request.party.as_deref() else {
            return BridgeOutcome::denied(vec![
                "ODRL Permit has no concrete party (assignee/WebID); no grant materialized"
                    .to_owned(),
            ]);
        };
        let Some(target) = request.target.as_deref() else {
            return BridgeOutcome::denied(vec![
                "ODRL Permit has no concrete target graph IRI; no grant materialized".to_owned(),
            ]);
        };

        // 3. Materialize `party auth:<mode> target` (one-shot allow shape; the count was
        //    consumed in step 1, and is re-checked read-only on refresh).
        let pred = format!("{AUTH_NS}{}", mode_predicate(mode));
        let triple = append_grant(graph, party, &pred, target);
        BridgeOutcome {
            granted: true,
            mode: Some(mode),
            grant_triple: Some((party.to_owned(), pred, target.to_owned())),
            consumed: exercise.consumed,
            emitted: vec![triple],
            ..BridgeOutcome::default()
        }
    }

    /// Re-check a tracked counted grant on refresh and re-emit it **iff** it still holds —
    /// WITHOUT consuming a unit (refresh is read-only; a refresh must never itself burn
    /// budget). [OPUS-4.8] sq-58mh.
    ///
    /// Fail-closed: the grant is re-emitted only when (a) a counter store is available,
    /// (b) the base permission STILL grants (re-evaluated with the `odrl:count`
    /// constraints stripped — the same shape [`evaluate_and_exercise`] uses for its base
    /// decision — so a withdrawn permission / now-matching prohibition denies and the
    /// grant is retracted), and (c) the granting rule's count budget is **not** exhausted
    /// ([`CountStatus::Satisfied`]/[`NotConstrained`](CountStatus::NotConstrained)).
    /// An exhausted ([`CountStatus::DefinitelyUnsatisfied`]), unprovable
    /// ([`CountStatus::Unprovable`] — store outage / malformed limit), or store-missing
    /// re-check emits NOTHING → the grant is retracted → access is GONE. This is what
    /// makes a bridged grant self-retract on exhaustion.
    pub(crate) fn refresh_permission_counted(
        graph: &mut Graph,
        policy: &Policy,
        request: &Request,
        store: Option<&CounterHandle>,
    ) -> BridgeOutcome {
        // No store ⇒ a counted entry cannot be accounted for → fail-closed retract.
        let Some(handle) = store else {
            return BridgeOutcome::denied(vec![
                "counted grant has no usage-counter store on refresh; retracted (fail-closed)"
                    .to_owned(),
            ]);
        };
        let store: &dyn UsageCounterStore = handle.0.as_ref();

        // (b) Does the BASE permission still grant? Re-evaluate with `odrl:count`
        //     stripped from the permissions (the stateless evaluator would otherwise deny
        //     for a missing count value) — exactly the base shape `evaluate_and_exercise`
        //     uses. A withdrawn permission / now-matching prohibition denies here →
        //     retract. NOTE: this never consumes; consumption only happens at exercise.
        let stripped = strip_count_constraints(policy);
        let decision = evaluate(&stripped, request);
        if !decision.allow {
            return BridgeOutcome::denied(decision.unmet_constraints);
        }

        // Locate the granting rule in the ORIGINAL policy (which still carries the count
        // constraint) so `count_status` can read its effective limit.
        let Some(rule) = decision
            .matched_rules
            .first()
            .and_then(|id| policy.permissions.iter().find(|r| &r.id == id))
        else {
            // A base grant with no identifiable rule (should not happen for a permission)
            // → nothing to count; re-emit the plain allow.
            return super::materialize_permission(graph, &stripped, request);
        };

        // (c) Read-only count check — NEVER consumes a unit on refresh.
        match count_status(rule, request, store) {
            // Budget remains, or the rule has no count limit → re-emit the allow grant.
            CountStatus::Satisfied { .. } | CountStatus::NotConstrained => {
                reemit_grant(graph, request)
            }
            // Exhausted, or unprovable (store outage / malformed) → retract (fail-closed).
            CountStatus::DefinitelyUnsatisfied { consumed, limit } => BridgeOutcome::denied(vec![
                format!(
                    "permission {} count budget exhausted ({consumed} >= {limit}); grant retracted",
                    rule.id
                ),
            ]),
            CountStatus::Unprovable => BridgeOutcome::denied(vec![format!(
                "permission {} count state unprovable on refresh; grant retracted (fail-closed)",
                rule.id
            )]),
        }
    }

    /// Re-emit the `principal auth:<mode> target` allow for `request` (the same triple
    /// [`materialize_permission_counted`] emitted), used on a still-valid count refresh.
    /// The triple is fully determined by the request (party + action→mode + target).
    fn reemit_grant(graph: &mut Graph, request: &Request) -> BridgeOutcome {
        let Some(mode) = action_to_mode(&request.action) else {
            return BridgeOutcome::denied(vec![
                "counted grant action no longer maps on refresh; not re-emitted".to_owned(),
            ]);
        };
        let (Some(party), Some(target)) = (request.party.as_deref(), request.target.as_deref())
        else {
            return BridgeOutcome::denied(vec![
                "counted grant lost its party/target on refresh; not re-emitted".to_owned(),
            ]);
        };
        let pred = format!("{AUTH_NS}{}", mode_predicate(mode));
        let triple = append_grant(graph, party, &pred, target);
        BridgeOutcome {
            granted: true,
            mode: Some(mode),
            grant_triple: Some((party.to_owned(), pred, target.to_owned())),
            emitted: vec![triple],
            ..BridgeOutcome::default()
        }
    }

    /// A copy of `policy` with every `odrl:count` constraint removed from its PERMISSION
    /// rules — the base shape the stateless [`evaluate`] sees (count is enforced against
    /// the store, not as a stateless numeric comparison). Mirrors sparq-policy's internal
    /// `strip_count_constraints` (kept here because that helper is crate-private).
    /// Prohibitions are untouched (a count on a prohibition keeps its stateless meaning).
    fn strip_count_constraints(policy: &Policy) -> Policy {
        let mut out = policy.clone();
        for rule in &mut out.permissions {
            rule.constraints.retain(|c| c.left != ODRL_COUNT);
        }
        out
    }
}

// ============================================================================
// [SONNET-4.6] sq-zgbso.2 — N3-stratum ODRL bridge.
// Runs the stateless ODRL core as five stratified rule evaluations
// (rules/odrl-{a0,a,b,c,d}.n3), mirroring the WAC/ACP stratification pattern —
// on the id-level COMPILED evaluator since sq-zgbso.5, as WAC/ACP are since
// sq-zgbso.4.
// Decision-equivalent to the one-shot Rust bridge for the supported constraint
// types (dateTime lteq/gteq over the canonical UTC lexical subset, recipient
// eq/neq, odrl:or, odrl:and) — on permissions AND prohibitions.
// ============================================================================

/// Materialize an ODRL policy's auth-view triples via five stratified N3 rule
/// strata (`rules/odrl-{a0,a,b,c,d}.n3`), installing the result into the dataset's
/// `<urn:sparq:auth>` and `<urn:sparq:auth-bridged>` views exactly as the
/// one-shot Rust bridge does.
///
/// # Relationship to the Rust reference path (the honest claim)
///
/// The N3 path is **never more permissive** than [`materialize_policy`]: for every
/// policy and request, either this call returns `Err` (refusing the policy as a whole,
/// materializing nothing — the caller must fail closed), or every allow triple it emits
/// is also emitted by the Rust path AND every deny triple the Rust path emits is also
/// emitted here. `tests/odrl_n3_differential.rs` checks that property over a generated
/// policy corpus, not a hand-picked sample.
///
/// Within the **supported stateless scope** the two paths are decision-EQUIVALENT
/// (same auth-triple set, checked case-by-case against an independently-stated
/// expectation): permissions AND prohibitions over `odrl:dateTime` lteq/gteq windows
/// (canonical UTC lexical forms only, see below) and `odrl:recipient` eq/neq, combined
/// by at most one `odrl:or`/`odrl:and` logical constraint per rule, each constraint node
/// carrying exactly ONE operand tuple, under an admissible `odrl:conflict` strategy.
/// Everything outside that scope is refused or denied, never granted:
///
/// - an unsupported construct on a *permission* produces NO grant (fail-closed);
/// - an unsupported construct on a *prohibition* — which the Rust evaluator might
///   satisfy, so silently ignoring it would drop a deny and WIDEN access — makes the
///   whole call refuse with `Err`;
/// - an `odrl:conflict` strategy the bridge cannot honour (`odrl:perm`, an unknown
///   strategy IRI, or `odrl:invalid` with a detected conflict) makes the whole call
///   refuse with `Err`, via the SAME [`sparq_policy::conflict_admissibility`] verdict
///   the Rust path's refusal uses. [OPUS-5]
/// - a constraint node carrying more than one operand triple per position, or more than
///   one logical combinator, makes the whole call refuse with `Err`. Satisfaction is
///   keyed on the constraint NODE, so such a node would otherwise be decided from a
///   single cross-product combination — "some operand satisfied" where "every operand
///   satisfied" is required. [OPUS-5]
///
/// # Injection safety
///
/// `policy_ttl` is parsed **strictly as Turtle** first (N3 implication rules
/// `=>`, quoted graphs, and every other Turtle-extension construct are syntax
/// errors → `Err`), and only the parsed ground terms are re-serialized — with
/// IRI validation and literal escaping — into the reasoner input. The raw text
/// never reaches the reasoner — the strata are compiled separately, from `const`
/// rule text, and the policy/request facts enter through a fact-ONLY interner
/// (`sparq_reason::n3::compiled::intern_facts`) that rejects outright any document
/// carrying rules — so a crafted policy cannot smuggle rules or
/// out-of-band triples that derive `auth:*` grants directly. The request is
/// serialized the same way, as validated RDF terms (`<urn:odrl-req>` subject,
/// `odrl:dateTime` for the temporal evidence): a request field that is not a
/// valid IRI (embedded `>`, quote, whitespace, …) is an `Err`, never an
/// interpolated triple.
///
/// Parsing alone does NOT make caller facts trusted: strict Turtle can still
/// *assert* engine-owned ground facts — an `auth:*` grant triple, an internal
/// `odrlx:` derivation (`atomicSat`/`prohibMatches`/`mode`/…), a `string:`/
/// `log:` builtin fact, facts about the reserved `<urn:odrl-req>` request
/// subject, or a fresh `a odrl:Request`-typed subject the grant rules would
/// bind `?req` to — which the strata would forward into the closure as if the
/// engine had derived them. So the parsed policy is additionally
/// vocabulary-checked: any triple whose subject/predicate/object is a
/// reserved/engine-owned IRI term is an `Err` and nothing is materialized
/// (see the crate-internal `validate_policy_for_n3`).
///
/// # Canonical dateTime subset
///
/// The strata compare `xsd:dateTime` values lexically (`string:notGreaterThan`),
/// which equals XSD dateTime VALUE order only on the canonical UTC form
/// `YYYY-MM-DDTHH:MM:SSZ` (fixed length, `Z` only, no fractional seconds,
/// hour ≤ 23). Every `xsd:dateTime` literal in the policy and the request time
/// is validated against that subset (and against the Rust evaluator's own
/// parser, so both paths denote the same instants); offsets, fractional
/// seconds, and other equivalent-but-different lexical forms are rejected with
/// `Err` before reasoning rather than mis-ordered.
///
/// # Errors
///
/// Returns `Err` — materializing NOTHING — when `policy_ttl` is not strict
/// Turtle, does not parse as an ODRL policy for the Rust reference evaluator, declares
/// an `odrl:conflict` strategy the bridge cannot honour, a policy triple mentions a
/// reserved/engine-owned IRI term (the
/// `https://sparq.dev/ns/` vocabulary, the reasoner's `swap` builtin
/// namespaces, `<urn:odrl-req>`, or the `odrl:Request` class), an
/// `xsd:dateTime` lexical form is outside the canonical UTC subset,
/// a constraint node is ambiguous (several operand triples in one position, or several
/// logical combinators), a prohibition is outside the supported stateless scope, a rule
/// carries more than one logical constraint, a request field is not a valid IRI, the
/// reasoner rejects the assembled fact source, or a rule file falls outside the compiled
/// evaluator's N3 subset (a build-time property of the `const` `rules/odrl-*.n3`, so in
/// practice that last one cannot fire at runtime — but it is reported as an error rather
/// than a panic, and nothing is materialized).
pub fn materialize_odrl_n3(
    graph: &mut Graph,
    policy_ttl: &str,
    request: &Request,
) -> Result<BridgeOutcome, String> {
    // 0. STRICT Turtle parse + ground-term re-serialization (injection guard), then
    //    the canonical-dateTime / prohibition-scope validation (fail-closed).
    let policy_graph = Graph::load_dataset(policy_ttl, "turtle").map_err(|e| {
        format!("policy is not strict Turtle (N3/Turtle-extension constructs are rejected): {}", e)
    })?;
    let policy_triples = crate::loader::graph_triples(&policy_graph);

    // 0a. [OPUS-5] Run the SAME `odrl:conflict` admissibility guard every Rust
    //     materialise entry point runs, FIRST. This call previously skipped it entirely,
    //     so a policy declaring `odrl:conflict odrl:perm` (permissions override
    //     prohibitions — unrepresentable by the bridge's `∪ allow ∖ ∪ deny`) or an
    //     unrecognised strategy IRI yielded Rust `refused=true` but an N3-materialised
    //     grant: a fail-OPEN. Sharing `sparq_policy::conflict_admissibility` (rather than
    //     re-deriving a coarser rule here) is what makes the two paths refuse exactly the
    //     same policies — including the CONDITIONAL `odrl:invalid` case, which is
    //     admissible only when no permission/prohibition conflict is detected.
    //
    //     The policy is re-parsed into the `sparq_policy` model for this: a policy the
    //     Rust evaluator cannot even parse is refused here too (strictly more restrictive
    //     than materialising from a shape only one path understands).
    let parsed_policy = parse_policy_str(policy_ttl, "turtle").map_err(|e| {
        format!(
            "policy does not parse as an ODRL policy for the Rust reference evaluator \
             ({}); the N3 path refuses what the reference path cannot read (fail-closed)",
            e
        )
    })?;
    if let Some(refusal) = refuse_unimplementable_conflict(&parsed_policy) {
        return Err(refusal.reasons.join("; "));
    }

    validate_policy_for_n3(&policy_triples)?;
    let policy_facts = triples_to_n3(&policy_triples);
    let req_n3 = serialize_request_n3(request)?;

    // [SONNET-4.6] sq-zgbso.5 — the five strata, in evaluation order:
    //   A0: the operand/combinator-arity guard (odrlx:shape odrlx:Ambiguous) the
    //       stratum A/C satisfaction rules negate [OPUS-5]
    //   A:  action facts + atomicSat + lcSat(or)
    //   B:  andSubUnsat + anyRuleConstrUnsat + unconstrained prohibMatches/denies
    //   C:  lcSat(and) + CONSTRAINED prohibMatches/denies
    //   D:  permission grants (NAF over the now-complete prohibMatches)
    //
    // They now chain entirely at the id level, in ONE dictionary, over the compiled IR
    // (`odrl_rules()`) — each stratum's closure IS the next one's fact set, so a negated
    // predicate is still complete before the stratum that negates it runs (design doc
    // §3.5) and the four intermediate closure → N3 text → re-parse round trips are gone.
    // The stratification, the rule text, and the fact set entering stratum A0 are all
    // unchanged: this is a behaviour-identical switch of the evaluation seam, and
    // `tests/odrl_n3_differential.rs` is the oracle that says so.
    let rules = odrl_rules()?;
    let mut dict = Dict::new();
    let mut closure = intern_facts(&mut dict, &format!("{}\n{}", policy_facts, req_n3))?;
    for stratum in rules {
        closure = eval(&mut dict, &closure, stratum);
    }

    // Extract auth:* triples from the final closure.
    let mut new_triples: Vec<[Term; 3]> = Vec::new();
    let mut grant_triple: Option<(String, String, String)> = None;
    let mut deny_triple: Option<(String, String, String)> = None;

    for t in &closure {
        let Term::NamedNode(p) = dict.term(t[1]) else { continue };
        let p_str = p.as_str();
        if !p_str.starts_with(AUTH_NS) {
            continue;
        }
        let Term::NamedNode(s) = dict.term(t[0]) else { continue };
        let Term::NamedNode(o) = dict.term(t[2]) else { continue };
        let triple = [
            Term::NamedNode(NamedNode::new_unchecked(s.as_str())),
            Term::NamedNode(NamedNode::new_unchecked(p_str)),
            Term::NamedNode(NamedNode::new_unchecked(o.as_str())),
        ];
        let local = p_str.strip_prefix(AUTH_NS).unwrap_or("");
        if local.starts_with("deny") {
            deny_triple = Some((s.as_str().to_owned(), p_str.to_owned(), o.as_str().to_owned()));
        } else {
            grant_triple = Some((s.as_str().to_owned(), p_str.to_owned(), o.as_str().to_owned()));
        }
        new_triples.push(triple);
    }

    if !new_triples.is_empty() {
        append_bridged_triples(graph, &new_triples);
    }

    let emitted = new_triples;
    Ok(BridgeOutcome {
        granted: grant_triple.is_some(),
        prohibited: deny_triple.is_some(),
        grant_triple,
        deny_triple,
        emitted,
        ..BridgeOutcome::default()
    })
}

/// The `xsd:dateTime` datatype IRI (canonical-subset validation on the N3 path).
const XSD_DATETIME_IRI: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// Whether `s` is in the canonical UTC `xsd:dateTime` lexical subset
/// `YYYY-MM-DDTHH:MM:SSZ` on which LEXICAL order equals XSD dateTime VALUE order:
/// fixed length, `Z` offset only, no fractional seconds, hour ≤ 23 (XSD's
/// `24:00:00` denotes the next day's midnight and would break lexicographic
/// monotonicity). Additionally the Rust evaluator's own parser must accept it
/// (`cmp_datetime` self-compare), so a calendar-invalid form the two paths could
/// disagree on is rejected too.
fn canonical_utc_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return false;
    }
    const DIGITS: [usize; 14] = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if DIGITS.iter().any(|&i| !b[i].is_ascii_digit()) {
        return false;
    }
    let field = |i: usize, j: usize| -> u32 { s[i..j].parse().unwrap_or(u32::MAX) };
    (1..=12).contains(&field(5, 7))
        && (1..=31).contains(&field(8, 10))
        && field(11, 13) <= 23
        && field(14, 16) <= 59
        && field(17, 19) <= 59
        && sparq_policy::cmp_datetime(s, s) == Some(std::cmp::Ordering::Equal)
}

/// Whether `t` is the named node `iri`.
fn is_named(t: &Term, iri: &str) -> bool {
    matches!(t, Term::NamedNode(n) if n.as_str() == iri)
}

/// The objects of every `subject predicate ?o` triple in `triples`.
fn objects_of<'a>(
    triples: &'a [[Term; 3]],
    subject: &'a Term,
    predicate: &'a str,
) -> impl Iterator<Item = &'a Term> + 'a {
    triples.iter().filter(move |t| &t[0] == subject && is_named(&t[1], predicate)).map(|t| &t[2])
}

/// Engine-owned IRI prefixes/terms a caller-supplied policy may never mention as
/// an actual RDF term (any position). The policy is UNTRUSTED data, but the N3
/// strata forward every input fact into the closure, and closure extraction /
/// rule bodies trust these vocabularies as engine-derived — so a caller
/// asserting them would cross the trust boundary (inject an `auth:*` grant,
/// fake an `odrlx:` derivation like `atomicSat`/`prohibMatches`/`mode`, assert
/// a `string:`/`log:` builtin result, or steer the rules with facts about the
/// reserved request subject). `https://sparq.dev/ns/` covers ALL sparq-internal
/// vocabularies at once (`auth#`, `odrlx#`, `solidx#`, `odrl-spike#`, and any
/// future one) rather than enumerating them.
const RESERVED_IRI_PREFIXES: [&str; 2] =
    ["https://sparq.dev/ns/", "http://www.w3.org/2000/10/swap/"];

/// The reserved request subject IRI `serialize_request_n3` emits — the ONLY
/// legitimate source of request facts on the N3 path.
const REQUEST_IRI: &str = "urn:odrl-req";

/// Fail-closed input validation for the N3 path, over the PARSED policy terms:
///
/// 0. no triple may mention a reserved/engine-owned IRI term in ANY position —
///    a prefix of [`RESERVED_IRI_PREFIXES`], the reserved [`REQUEST_IRI`]
///    request subject, or the `odrl:Request` class (the grant/deny rules bind
///    `?req` to ANY `a odrl:Request`-typed node, so a caller-minted request
///    would be matched exactly like the real one). Parsing/escaping stops
///    source-syntax injection but does not make untrusted RDF facts trusted;
///    this rule is what keeps caller assertions out of the engine-owned fact
///    space (see the docs on [`RESERVED_IRI_PREFIXES`]);
/// 1. every `xsd:dateTime` literal must be in the canonical UTC lexical subset
///    ([`canonical_utc_datetime`]) — the strata compare dateTimes lexically,
///    which is value-correct only there;
/// 2. no rule (permission or prohibition) may carry more than one logical
///    constraint — the strata satisfy a rule on ANY satisfied LC while the Rust
///    evaluator requires ALL, so a second LC could widen; likewise no
///    `odrl:duty`/`odrl:obligation` anywhere — the strata do not check duty
///    discharge, so granting a duty-carrying permission would widen;
/// 3. no constraint node (a rule's own, or a member of its logical constraint) may
///    be AMBIGUOUS — several distinct `odrl:leftOperand`/`odrl:operator`/right-operand
///    objects, or several distinct logical combinators, on one node
///    (`unambiguous_operands`). This is the driver half of the stratum-A0 fail-OPEN
///    fix and applies to BOTH rule kinds. [OPUS-5];
/// 4. no rule (permission or prohibition) may carry MORE THAN ONE distinct
///    `odrl:action`/`odrl:target`/`odrl:assignee` object — the strata bind each
///    attribute as a triple pattern and so match a request that agrees with ANY
///    asserted value, whereas the Rust reference evaluator selects exactly ONE
///    (the first) per attribute (`first_str`). A second action/target/assignee
///    could therefore make an N3 grant fire (or a prohibition match) for a value
///    the Rust path never selected — widening. Refuse rather than decide the
///    rule from a subset of its attribute values (the rule-attribute analogue of
///    the constraint-node `unambiguous_operands` guard). [OPUS-4.8];
/// 5. every PROHIBITION must name `odrl:action`/`odrl:target`/`odrl:assignee`
///    (the strata match prohibitions structurally on all three; the Rust
///    evaluator treats a missing attribute as matching ANY request) and every
///    prohibition constraint — atomic, or each `odrl:or`/`odrl:and` member —
///    must be inside the supported stateless scope; `odrl:xone` on a
///    prohibition is refused, as is a prohibition on the `odrl:use` umbrella
///    action (the strata match actions by exact IRI while the Rust evaluator's
///    use-umbrella denies any non-transfer action). The Rust evaluator can
///    satisfy out-of-scope
///    prohibition constructs (purpose, count, isPartOf, …), so an N3 stratum
///    that cannot see them satisfied would silently drop the deny and WIDEN
///    access. (On a PERMISSION an out-of-scope construct merely never grants —
///    already fail-closed — so permissions are not scope-guarded.)
fn validate_policy_for_n3(triples: &[[Term; 3]]) -> Result<(), String> {
    let request_class = format!("{}Request", ODRL_NS);
    for t in triples {
        for term in t {
            let Term::NamedNode(n) = term else { continue };
            let iri = n.as_str();
            if RESERVED_IRI_PREFIXES.iter().any(|p| iri.starts_with(p))
                || iri == REQUEST_IRI
                || iri == request_class
            {
                return Err(format!(
                    "policy mentions the reserved/engine-owned term <{}>: a \
                     caller-supplied policy is untrusted data and may not assert \
                     auth-view grants, internal odrlx: derivations, reasoner \
                     builtin facts, facts about the reserved request subject, or \
                     request-typed subjects — refusing (fail-closed)",
                    iri
                ));
            }
        }
    }
    for t in triples {
        if let Term::Literal(l) = &t[2] {
            if l.datatype().as_str() == XSD_DATETIME_IRI && !canonical_utc_datetime(l.value()) {
                return Err(format!(
                    "xsd:dateTime literal {:?} is outside the canonical UTC subset \
                     YYYY-MM-DDTHH:MM:SSZ the N3 strata compare correctly; refusing \
                     (fail-closed) rather than mis-order it lexically",
                    l.value()
                ));
            }
        }
    }
    for duty_local in ["duty", "obligation"] {
        let duty_p = format!("{}{}", ODRL_NS, duty_local);
        if triples.iter().any(|t| is_named(&t[1], &duty_p)) {
            return Err(format!(
                "odrl:{} is outside the N3 strata's scope (they do not check duty \
                 discharge; the Rust evaluator requires it for a Permit) — granting \
                 regardless would widen access; refusing (fail-closed)",
                duty_local
            ));
        }
    }
    let permission = format!("{}permission", ODRL_NS);
    let prohibition = format!("{}prohibition", ODRL_NS);
    let constraint = format!("{}constraint", ODRL_NS);
    let or_p = format!("{}or", ODRL_NS);
    let and_p = format!("{}and", ODRL_NS);
    let xone_p = format!("{}xone", ODRL_NS);
    for t in triples {
        let is_prohib = is_named(&t[1], &prohibition);
        if !is_prohib && !is_named(&t[1], &permission) {
            continue;
        }
        let rule = &t[2];
        // [OPUS-4.8] Reject a MULTI-VALUED rule attribute on EITHER rule kind. The N3
        // strata bind odrl:action/target/assignee as triple patterns, so a rule asserting
        // several distinct values matches a request agreeing with ANY of them, while the
        // Rust reference evaluator selects exactly ONE (`first_str`). A second value could
        // fire an N3 grant (or a prohibition match) the Rust path never selects — widening.
        // Refuse the whole call (fail-closed) rather than decide the rule from a subset of
        // its attribute values — the rule-attribute analogue of `unambiguous_operands`.
        for attr in ["action", "target", "assignee"] {
            let pred = format!("{}{}", ODRL_NS, attr);
            if distinct_objects(triples, rule, std::slice::from_ref(&pred)).len() > 1 {
                return Err(format!(
                    "rule with more than one distinct odrl:{} object is outside the N3 \
                     strata's scope (they bind the attribute as a triple pattern and match \
                     ANY asserted value, while the Rust evaluator selects exactly one) — a \
                     second value could widen an N3 grant or prohibition match; refusing \
                     (fail-closed)",
                    attr
                ));
            }
        }
        if is_prohib {
            for attr in ["action", "target", "assignee"] {
                let pred = format!("{}{}", ODRL_NS, attr);
                if objects_of(triples, rule, &pred).next().is_none() {
                    return Err(format!(
                        "prohibition without odrl:{} is outside the N3 strata's scope \
                         (they match prohibitions structurally on action/target/assignee, \
                         while the Rust evaluator treats the missing attribute as matching \
                         ANY request) — silently ignoring it would widen access; refusing \
                         (fail-closed)",
                        attr
                    ));
                }
            }
            let action_p = format!("{}action", ODRL_NS);
            let use_iri = format!("{}use", ODRL_NS);
            if objects_of(triples, rule, &action_p).any(|a| is_named(a, &use_iri)) {
                return Err(
                    "prohibition with the odrl:use umbrella action is outside the N3 \
                     strata's scope (they match actions by exact IRI; the Rust \
                     evaluator's use-umbrella denies any non-transfer action) — \
                     silently ignoring it would widen access; refusing (fail-closed)"
                        .to_owned(),
                );
            }
        }
        let mut lc_count = 0usize;
        for c in objects_of(triples, rule, &constraint) {
            let xone_subs: Vec<&Term> = objects_of(triples, c, &xone_p).collect();
            let has_xone = !xone_subs.is_empty();
            let subs: Vec<&Term> =
                objects_of(triples, c, &or_p).chain(objects_of(triples, c, &and_p)).collect();
            if has_xone || !subs.is_empty() {
                lc_count += 1;
                if lc_count > 1 {
                    return Err(
                        "rule with more than one logical constraint is outside the N3 \
                         strata's scope (they satisfy a rule on ANY satisfied logical \
                         constraint; the Rust evaluator requires ALL) — refusing \
                         (fail-closed)"
                            .to_owned(),
                    );
                }
            }
            // [OPUS-5] Operand-ARITY guard on BOTH rule kinds — the driver half of the
            // stratum-A0 fix. The strata key satisfaction on the constraint NODE, so a
            // node carrying several operand triples (or several combinators) is decided
            // from ONE cross-product combination. Refuse the whole call rather than
            // evaluate a subset of the operands.
            unambiguous_operands(triples, c)?;
            for sub in subs.iter().chain(xone_subs.iter()) {
                unambiguous_operands(triples, sub)?;
            }
            if !is_prohib {
                continue;
            }
            if has_xone {
                return Err(
                    "odrl:xone on a prohibition is outside the N3 strata's scope; the Rust \
                     evaluator may satisfy it, so silently ignoring it would drop the deny \
                     and widen access — refusing (fail-closed)"
                        .to_owned(),
                );
            }
            if subs.is_empty() {
                prohibition_atomic_supported(triples, c)?;
            } else {
                for sub in subs {
                    prohibition_atomic_supported(triples, sub)?;
                }
            }
        }
    }
    Ok(())
}

/// The DISTINCT objects `c` carries across `predicates`, in `triples` order.
fn distinct_objects<'a>(
    triples: &'a [[Term; 3]],
    c: &'a Term,
    predicates: &'a [String],
) -> Vec<&'a Term> {
    let mut seen: Vec<&'a Term> = Vec::new();
    for p in predicates {
        for o in objects_of(triples, c, p) {
            if !seen.contains(&o) {
                seen.push(o);
            }
        }
    }
    seen
}

/// Refuse (fail-closed) a constraint node whose operand or combinator positions are
/// AMBIGUOUS: more than one distinct `odrl:leftOperand`, `odrl:operator`, or
/// right-operand object (`odrl:rightOperand`/`odrl:rightOperandReference`), or more
/// than one distinct logical combinator (`odrl:and`/`odrl:or`/`odrl:xone`) on the same
/// node. [OPUS-5] — the driver half of the stratum-A0 fail-OPEN fix.
///
/// RDF puts no cardinality bound on a node's properties, and the strata key
/// satisfaction on the constraint NODE rather than on an individual operand triple. A
/// node with several operand triples is therefore matched as the CROSS PRODUCT of those
/// objects, and ANY ONE satisfiable combination marks the whole node satisfied — "some
/// operand is satisfied" where "every operand is satisfied" is required. Measured
/// consequence before this guard: a permission constrained by `recipient eq alice` AND
/// `dateTime lteq 2000-01-01` on one node GRANTED long after expiry, while the Rust
/// reference evaluator folded the ambiguous node into an unsatisfiable guard and denied.
///
/// `rules/odrl-a0.n3` suppresses satisfaction for these nodes inside the rules, which
/// closes the grant direction. It cannot close the DENY direction: suppressing an
/// ambiguous prohibition's satisfaction would drop its deny and widen access. So the
/// driver refuses the whole call — the only disposition that is fail-closed on both
/// sides. Both layers are load-bearing; neither subsumes the other.
fn unambiguous_operands(triples: &[[Term; 3]], c: &Term) -> Result<(), String> {
    let positions: [(&str, Vec<String>); 4] = [
        ("odrl:leftOperand", vec![format!("{}leftOperand", ODRL_NS)]),
        ("odrl:operator", vec![format!("{}operator", ODRL_NS)]),
        (
            "right-operand",
            vec![
                format!("{}rightOperand", ODRL_NS),
                format!("{}rightOperandReference", ODRL_NS),
            ],
        ),
        (
            "logical combinator",
            vec![
                format!("{}and", ODRL_NS),
                format!("{}or", ODRL_NS),
                format!("{}xone", ODRL_NS),
            ],
        ),
    ];
    for (label, predicates) in &positions {
        // A combinator legitimately takes SEVERAL operand objects (`odrl:and c1, c2`),
        // so what is ambiguous there is several distinct combinator PREDICATES, not
        // several objects of one.
        let ambiguous = if *label == "logical combinator" {
            predicates.iter().filter(|p| objects_of(triples, c, p).next().is_some()).count() > 1
        } else {
            distinct_objects(triples, c, predicates).len() > 1
        };
        if ambiguous {
            return Err(format!(
                "constraint node {} is AMBIGUOUS in its {} position (several distinct \
                 values on one node). The N3 strata key satisfaction on the constraint \
                 NODE, so such a node is matched as the cross product of its operands and \
                 is marked satisfied as soon as ANY ONE combination holds — while the Rust \
                 reference evaluator folds it into an unsatisfiable guard. Refusing \
                 (fail-closed) rather than deciding the node from a subset of its operands",
                c, label
            ));
        }
    }
    Ok(())
}

/// Whether one PROHIBITION atomic constraint is inside the stateless scope the N3
/// strata decide: `odrl:dateTime` under `lteq`/`gteq`, or `odrl:recipient` under
/// `eq`/`neq`. Anything else is refused — the Rust evaluator may satisfy it, and
/// a satisfied prohibition constraint the N3 side cannot see would silently drop
/// its deny (widening access).
///
/// [OPUS-5] This used to read only `.next()` of `odrl:leftOperand`/`odrl:operator` and
/// was therefore structurally BLIND to any further operand on the node: a constraint
/// carrying `recipient eq …` (in scope) *and* `purpose eq …` (out of scope) was waved
/// through on its first pair alone. It now inspects EVERY operand object and declines
/// unless the node carries exactly ONE complete, in-scope operand tuple — a constraint
/// whose operands it cannot fully evaluate is refused, never evaluated in part. The
/// exactly-one requirement is deliberately re-checked here rather than delegated to
/// [`unambiguous_operands`], so this predicate is sound on its own terms regardless of
/// the order the validator's guards run in.
fn prohibition_atomic_supported(triples: &[[Term; 3]], c: &Term) -> Result<(), String> {
    let left_preds = [format!("{}leftOperand", ODRL_NS)];
    let op_preds = [format!("{}operator", ODRL_NS)];
    let right_preds =
        [format!("{}rightOperand", ODRL_NS), format!("{}rightOperandReference", ODRL_NS)];
    let lefts = distinct_objects(triples, c, &left_preds);
    let ops = distinct_objects(triples, c, &op_preds);
    let rights = distinct_objects(triples, c, &right_preds);
    let single_tuple = lefts.len() == 1 && ops.len() == 1 && rights.len() == 1;
    let supported = single_tuple
        && ((is_named(lefts[0], &format!("{}dateTime", ODRL_NS))
            && (is_named(ops[0], &format!("{}lteq", ODRL_NS))
                || is_named(ops[0], &format!("{}gteq", ODRL_NS))))
            || (is_named(lefts[0], &format!("{}recipient", ODRL_NS))
                && (is_named(ops[0], &format!("{}eq", ODRL_NS))
                    || is_named(ops[0], &format!("{}neq", ODRL_NS)))));
    if supported {
        Ok(())
    } else {
        Err(format!(
            "prohibition constraint {} is outside the supported stateless scope \
             (exactly one operand tuple: dateTime lteq/gteq, or recipient eq/neq; \
             found {} leftOperand / {} operator / {} rightOperand objects): the Rust \
             evaluator may satisfy it, so silently ignoring it — or deciding it from a \
             subset of its operands — would drop the deny and widen access; refusing \
             (fail-closed)",
            c,
            lefts.len(),
            ops.len(),
            rights.len()
        ))
    }
}

/// Serialize parsed ground triples as N-Triples-style N3 facts for the reasoner
/// input — `oxrdf`'s `Display` does the IRI/literal escaping, so no policy-
/// controlled string reaches the source unescaped.
fn triples_to_n3(triples: &[[Term; 3]]) -> String {
    let mut out = String::with_capacity(triples.len() * 64);
    for t in triples {
        let _ = writeln!(out, "{} {} {} .", t[0], t[1], t[2]);
    }
    out
}

/// Serialize a `Request` as N3 ground facts using `<urn:odrl-req>` as the IRI.
///
/// Every field is serialized as a VALIDATED RDF term — IRIs through
/// [`NamedNode::new`] (an embedded `>`, quote, or whitespace is an `Err`, never
/// an injected triple) and the time as an escaped `xsd:dateTime` literal
/// restricted to the canonical UTC subset the strata compare correctly.
fn serialize_request_n3(req: &Request) -> Result<String, String> {
    fn iri(value: &str, what: &str) -> Result<NamedNode, String> {
        NamedNode::new(value).map_err(|e| {
            format!(
                "request {} {:?} is not a valid IRI ({}); refusing to serialize it \
                 into N3 (fail-closed)",
                what, value, e
            )
        })
    }
    let mut out = String::new();
    let _ = writeln!(out, "<urn:odrl-req> a <{}Request> ;", ODRL_NS);
    let _ = writeln!(out, "    <{}action> {} ;", ODRL_NS, iri(&req.action, "action")?);
    if let Some(target) = &req.target {
        let _ = writeln!(out, "    <{}target> {} ;", ODRL_NS, iri(target, "target")?);
    }
    if let Some(party) = &req.party {
        let _ = writeln!(out, "    <{}assignee> {} ;", ODRL_NS, iri(party, "party")?);
    }
    if let Some(v) = req.request_time() {
        let s = v.as_str();
        if !canonical_utc_datetime(s) {
            return Err(format!(
                "request time {:?} is outside the canonical UTC subset \
                 YYYY-MM-DDTHH:MM:SSZ the N3 strata compare correctly; refusing \
                 (fail-closed)",
                s
            ));
        }
        let lit = Literal::new_typed_literal(s, NamedNode::new_unchecked(XSD_DATETIME_IRI));
        let _ = writeln!(out, "    <{}dateTime> {} ;", ODRL_NS, lit);
    }
    let _ = writeln!(out, "    .");
    Ok(out)
}

#[cfg(test)]
mod set_tighter_tests {
    //! [OPUS-4.8] sq-0q7n — `set_tighter` keeps the instant-tightest window bound when
    //! two same-side `odrl:dateTime` constraints intersect. The pre-fix lexical `<`/`>`
    //! picked the wrong bound for mixed timezone offsets (a fail-open: a wider persisted
    //! window than the constraints allow). These pin the offset-aware behavior.
    use super::set_tighter;

    #[test]
    fn upper_bound_keeps_earlier_instant_across_offsets() {
        // Two notAfter bounds: 12:00Z (= 12:00Z) and 13:00+02:00 (= 11:00Z). The EARLIER
        // instant is the offset form (11:00Z); lexically "12…Z" < "13…+02:00" so the OLD
        // code wrongly kept 12:00Z (later instant → wider window).
        let mut slot = Some("2026-06-16T12:00:00Z".to_owned());
        set_tighter(&mut slot, "2026-06-16T13:00:00+02:00", /*keep_earlier=*/ true);
        assert_eq!(slot.as_deref(), Some("2026-06-16T13:00:00+02:00"), "kept earlier instant");
    }

    #[test]
    fn lower_bound_keeps_later_instant_across_offsets() {
        // Two notBefore bounds: 12:00Z and 09:00-02:00 (= 11:00Z). The LATER instant is
        // 12:00Z; lexically "09…-02:00" < "12…Z" — verify the tighter (later) is kept.
        let mut slot = Some("2026-06-16T12:00:00Z".to_owned());
        set_tighter(&mut slot, "2026-06-16T09:00:00-02:00", /*keep_earlier=*/ false);
        assert_eq!(slot.as_deref(), Some("2026-06-16T12:00:00Z"), "kept later instant");
        // And a genuinely-later offset bound DOES replace.
        set_tighter(&mut slot, "2026-06-16T16:00:00+02:00", /*keep_earlier=*/ false); // = 14:00Z
        assert_eq!(slot.as_deref(), Some("2026-06-16T16:00:00+02:00"), "later instant wins");
    }

    #[test]
    fn unparseable_candidate_never_widens() {
        let mut slot = Some("2026-06-16T12:00:00Z".to_owned());
        set_tighter(&mut slot, "not-a-date", true);
        assert_eq!(slot.as_deref(), Some("2026-06-16T12:00:00Z"), "malformed never replaces");
    }
}

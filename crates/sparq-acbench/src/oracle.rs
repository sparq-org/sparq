//! By-construction procedural oracle (§2.3 of `research/ac-query-benchmark.md`).
//!
//! This module is the **ground truth** the system under test is judged against.
//! It is intentionally simple and must NEVER call any sparq crate — its value comes
//! from being structurally independent of the implementation under test.
//!
//! # Fail-closed invariant
//! Every public function returns [`Decision::Deny`] by default. A request is allowed
//! only when there exists a matching `Allow` intent that is not overridden by a `Deny`.
//!
//! # Determinism (clock-free)
//! All functions are pure (no mutation, no randomness, **no wall-clock read**). Temporal
//! ODRL conditions are decided against a *pinned* evaluation instant
//! (`BENCH_EVAL_INSTANT`) threaded through `evaluate_odrl_at`, never
//! `SystemTime::now()`, so an expired-window intent stays `Deny` no matter what day the
//! benchmark is re-run. Given the same inputs the oracle always returns the same output.
//!
//! # Completeness
//! Per-row evaluation (WAC / ACP / ODRL semantics, including the clock-free temporal +
//! purpose condition evaluation) lives here; the workload-engine integration (W1–W4
//! decision batches, result-set oracles, churn delta computation) lives in `workload.rs`.
//! Both are bead `sq-i6du2.6`'s scope. The three public wrappers in `lib.rs`
//! (`oracle_wac` / `oracle_acp` / `oracle_odrl`) call the evaluators here.

use crate::{Audience, Condition, Decision, Effect, IntentRow, Request};

// ── WAC oracle ──────────────────────────────────────────────────────────────────────

/// Evaluate a request against an intent table using WAC semantics —
/// **nearest-ACL-document-wins** (bead `sq-o4orz` / `sq-kvvcl.2`).
///
/// # Why this is not a `starts_with` union
/// WAC does **not** accumulate ancestor grants the way ACP does (design record
/// §2.1/§4: "WAC nearest-ancestor vs ACP cumulative inheritance"). A resource `R` is
/// governed by exactly **one** ACL document — `R.acl` if it exists, otherwise the ACL of
/// the *nearest* ancestor container that has one — and a closer ACL **fully shadows**
/// every farther one, grants included. Unioning all matching ancestors (the previous
/// behaviour) over-approximates `Allow`, which showed up in the live driver
/// (`bench/ac/live`) as an oracle=Allow / engine=Deny **under-share** divergence.
///
/// # The ACL-document model over the intent table
/// The intent table is model-agnostic, so the ACL-document layout is re-derived here from
/// exactly what [`crate::compile_wac`] emits — never by calling it:
/// - a WAC-**expressible** row (`Effect::Allow`, `Condition::None`, and not an unbounded
///   `Audience::AllExcept(vec![])`; each of those three compiles to an empty policy)
///   materializes an authorization in the ACL document of its own `resource_uri`. An
///   inexpressible row therefore does **not** bring an ACL document into existence and so
///   cannot shadow an ancestor either;
/// - `Scope::Resource` → `acl:accessTo <resource_uri>`: applies to that resource only,
///   and only when that resource's *own* ACL document is the effective one;
/// - `Scope::Subtree` → `acl:default <resource_uri>` **plus** `acl:accessTo <resource_uri>`:
///   `default` reaches the container's **members** (and only when that container's ACL
///   document is the effective one — a container is not a member of itself, which is why
///   `default` alone would leave the container itself unreachable), while `accessTo`
///   reaches the container resource. Together they make `Scope::Subtree` mean "this
///   container and everything under it".
///
/// # Other WAC rules (unchanged)
/// - `Condition ≠ None` rows are UNSUPPORTED in WAC and are skipped.
/// - `Effect::Deny` rows are UNSUPPORTED in WAC and are skipped.
/// - `Audience::AllExcept(vec![])` (the unbounded-exclusion placeholder) is UNSUPPORTED in
///   WAC and is skipped; a *bounded* `AllExcept` enumerates its allows and is expressible.
/// - Fail-closed: no governing ACL document, or no matching Allow inside it →
///   [`Decision::Deny`].
pub(crate) fn evaluate_wac(request: &Request, intents: &[IntentRow]) -> Decision {
    use crate::Scope;

    // Nearest-ACL resolution: the single ACL document that governs this resource.
    let Some((governed, is_own)) = wac_effective_acl(&request.resource, intents) else {
        return Decision::Deny; // no ACL document governs the resource — fail-closed
    };

    for intent in intents {
        if !wac_expressible(intent) {
            continue;
        }

        // A row living in any OTHER ACL document is shadowed by the effective one.
        if intent.resource_uri != governed {
            continue;
        }

        // Within the effective document: in the resource's OWN ACL every row targets the
        // resource, because BOTH scopes emit `acl:accessTo <own IRI>` (see
        // [`crate::compile_wac`]). Only an INHERITED document needs the `acl:default`
        // membership test, and only `Scope::Subtree` emits that `default`.
        let applies = is_own
            || (intent.scope == Scope::Subtree && wac_is_member(governed, &request.resource));
        if !applies {
            continue;
        }

        // Mode check.
        if !mode_matches(&request.mode, &intent.mode) {
            continue;
        }

        // Audience check.
        if audience_matches_wac(request, &intent.audience) {
            return Decision::Allow;
        }
    }

    Decision::Deny // fail-closed
}

/// Is this intent row expressible in WAC — i.e. does it materialize an authorization
/// (and therefore an ACL document) at all?
///
/// Mirrors **every** empty-policy return in [`crate::compile_wac`] — the two early returns
/// (`Condition ≠ None`, `Effect::Deny`) *and* the unbounded `Audience::AllExcept(vec![])`
/// return inside the audience match. All three compile to an EMPTY policy, so none of them
/// creates an ACL document. This matters for nearest-ACL resolution: an inexpressible row
/// must not shadow the ancestor ACL that really governs the resource.
///
/// Keep this list in lockstep with `compile_wac`; a missed case makes the oracle invent a
/// shadowing ACL document the compiler never emitted.
fn wac_expressible(intent: &IntentRow) -> bool {
    if intent.effect != Effect::Allow || intent.condition != Condition::None {
        return false;
    }
    // An empty exclusion set is the "unbounded" placeholder — inexpressible in WAC.
    !matches!(intent.audience, Audience::AllExcept(ref excl) if excl.is_empty())
}

/// The IRI of the ACL document sited at `iri`, if any WAC-expressible row puts one there.
/// Borrowed from the intent table so the caller's result outlives the ancestry cursor.
fn wac_acl_doc_at<'a>(iri: &str, intents: &'a [IntentRow]) -> Option<&'a str> {
    intents
        .iter()
        .find(|i| wac_expressible(i) && i.resource_uri == iri)
        .map(|i| i.resource_uri.as_str())
}

/// Resolve the **effective** (nearest-wins) ACL document governing `resource`.
///
/// Returns `(governed_iri, is_own)`: the IRI whose ACL document governs the resource, and
/// whether that document is the resource's *own* ACL (`true`) or one inherited from an
/// ancestor container (`false`). `None` — fail-closed — when neither the resource nor any
/// ancestor has an ACL document.
fn wac_effective_acl<'a>(resource: &str, intents: &'a [IntentRow]) -> Option<(&'a str, bool)> {
    if let Some(own) = wac_acl_doc_at(resource, intents) {
        return Some((own, true));
    }
    let mut cursor = resource.to_string();
    while let Some(parent) = wac_parent_container(&cursor).map(str::to_owned) {
        if let Some(hit) = wac_acl_doc_at(&parent, intents) {
            return Some((hit, false));
        }
        cursor = parent;
    }
    None
}

/// The Solid slash-semantics parent container of an IRI, derived here independently of any
/// sparq crate (the oracle must never link the system under test).
///
/// Strip exactly one trailing `/` (so `…/c/` parents to `…/`, not to itself), then cut at
/// the last `/` that is still inside the path. `None` at or above the authority root.
fn wac_parent_container(iri: &str) -> Option<&str> {
    let scheme_end = iri.find("://")? + 3;
    let host_end = scheme_end + iri.get(scheme_end..)?.find('/')?;
    let trimmed = iri.strip_suffix('/').unwrap_or(iri);
    if trimmed.len() <= host_end {
        return None; // already the authority root container
    }
    let cut = trimmed.rfind('/')?;
    if cut < host_end {
        return None;
    }
    Some(&iri[..=cut])
}

/// Is `resource` a member (structural descendant) of the container `container`?
///
/// A container's own IRI is NOT a member of itself — `acl:default` reaches members, not
/// the container resource. Containers are named with a trailing `/` in Solid slash
/// semantics, so a strict prefix relation over that boundary is the membership test; an
/// intent whose `resource_uri` is not slash-terminated names a plain resource, which the
/// parent chain can never reach, and so governs nothing by inheritance.
fn wac_is_member(container: &str, resource: &str) -> bool {
    container.ends_with('/') && resource != container && resource.starts_with(container)
}

/// Check whether the request's agent/client matches a WAC audience.
fn audience_matches_wac(request: &Request, audience: &Audience) -> bool {
    match audience {
        Audience::Public => true,
        Audience::Authenticated => !request.agent.is_empty(),
        Audience::Owner => request.agent == format!("{}#owner", request.resource),
        Audience::Agent(a) => &request.agent == a,
        Audience::Group(g) => {
            // Group membership is evaluated by the generator; at oracle level,
            // the group IRI is opaque. The generator-level oracle resolves group
            // closure before calling this function (via `evaluate_wac_with_closure`).
            // For the scaffold smoke tests, treat a group match as "agent IRI contains
            // the group IRI as a prefix" — full resolution is B2–B5's responsibility.
            request.agent.starts_with(g.as_str())
        }
        Audience::ClientRestricted { agent, client } => {
            &request.agent == agent
                && request.client.as_deref() == Some(client.as_str())
        }
        Audience::AllExcept(excl) => {
            // WAC has no deny: AllExcept is approximated as "not in exclusion list".
            !excl.iter().any(|e| e == &request.agent)
        }
    }
}

// ── ACP oracle ──────────────────────────────────────────────────────────────────────

/// Evaluate a request against an intent table using ACP semantics.
///
/// ACP rules:
/// - `Effect::Deny` overrides any `Effect::Allow` (deny-wins within matching intents).
/// - `Condition ≠ None` intents are UNSUPPORTED in ACP and are skipped.
/// - `Scope::Subtree` applies to the resource if the resource URI starts with
///   the intent's `resource_uri` (cumulative-ancestor inheritance, all matching ancestors).
/// - Fail-closed: no matching Allow (or any matching Deny) → [`Decision::Deny`].
pub(crate) fn evaluate_acp(request: &Request, intents: &[IntentRow]) -> Decision {
    use crate::Scope;

    let mut any_allow = false;
    let mut any_deny = false;

    for intent in intents {
        // ACP has no usage conditions in the base spec.
        if intent.condition != Condition::None {
            continue;
        }

        let resource_matches = match intent.scope {
            Scope::Resource => request.resource == intent.resource_uri,
            Scope::Subtree => request.resource.starts_with(&intent.resource_uri),
        };
        if !resource_matches {
            continue;
        }

        if !mode_matches(&request.mode, &intent.mode) {
            continue;
        }

        if audience_matches_acp(request, &intent.audience) {
            match intent.effect {
                Effect::Allow => any_allow = true,
                Effect::Deny => any_deny = true,
            }
        }
    }

    // ACP: deny-wins. Any explicit deny overrides any allow.
    if any_deny || !any_allow {
        Decision::Deny
    } else {
        Decision::Allow
    }
}

/// Check whether the request's agent/client matches an ACP audience.
fn audience_matches_acp(request: &Request, audience: &Audience) -> bool {
    match audience {
        Audience::Public => true,
        Audience::Authenticated => !request.agent.is_empty(),
        Audience::Owner => request.agent == format!("{}#owner", request.resource),
        Audience::Agent(a) => &request.agent == a,
        Audience::Group(g) => {
            // Same placeholder as WAC oracle; full group closure resolved by generators.
            request.agent.starts_with(g.as_str())
        }
        Audience::ClientRestricted { agent, client } => {
            &request.agent == agent
                && request.client.as_deref() == Some(client.as_str())
        }
        Audience::AllExcept(excl) => {
            !excl.iter().any(|e| e == &request.agent)
        }
    }
}

// ── ODRL oracle ─────────────────────────────────────────────────────────────────────

/// The **pinned benchmark evaluation instant** ("now") that all temporal conditions are
/// evaluated against.
///
/// The oracle must be **clock-free** (no `SystemTime::now()`) so the ground truth is
/// byte-identical on every run, platform, and date — a wall-clock read would make an
/// expired-window intent flip from `Deny` to `Allow` merely by re-running the benchmark
/// on a later day. Instead we pin a single fixed instant, in the same fixed-width UTC
/// ISO-8601 form the generators emit, and every `Condition::Temporal { start, end }` is
/// decided by the half-open membership test `start ≤ NOW < end`.
///
/// Value: `2026-07-06T00:00:00Z` — the benchmark's authoring date (design record §0). It
/// sits inside the U3/U4 "current" windows and after the expired ones, so a corpus
/// deterministically exercises BOTH the in-window (`Allow`) and expired-window (`Deny`)
/// paths. This is a design decision recorded here and in the PR body; a future EC2 tier
/// that wants to sweep the clock threads its own instant through
/// [`evaluate_odrl_at`] instead of relying on this default.
pub(crate) const BENCH_EVAL_INSTANT: &str = "2026-07-06T00:00:00Z";

/// Evaluate a request against an intent table using ODRL semantics, at the pinned
/// benchmark evaluation instant ([`BENCH_EVAL_INSTANT`]).
///
/// ODRL rules:
/// - `Effect::Allow` (Permission): a matching permission grants access if all conditions
///   are satisfied.
/// - `Effect::Deny` (Prohibition): a matching prohibition denies access.
/// - If both a Permission and a Prohibition match: Permission wins (ODRL precedence).
/// - Conditions are evaluated procedurally against `now` (see [`evaluate_condition`]).
/// - Fail-closed: no matching Permission → [`Decision::Deny`].
pub(crate) fn evaluate_odrl(request: &Request, intents: &[IntentRow]) -> Decision {
    evaluate_odrl_at(request, intents, BENCH_EVAL_INSTANT)
}

/// Evaluate a request against an intent table using ODRL semantics, at an explicit,
/// caller-supplied evaluation instant `now` (fixed-width UTC ISO-8601, e.g.
/// `"2026-07-06T00:00:00Z"`).
///
/// This is the clock-free core: `now` is threaded in, never read from the system clock,
/// so the result is a pure function of `(request, intents, now)`. [`evaluate_odrl`] is the
/// thin wrapper that pins `now` to [`BENCH_EVAL_INSTANT`]; an alternate-clock lane (e.g. a
/// U4 embargo sweep) calls this directly.
pub(crate) fn evaluate_odrl_at(request: &Request, intents: &[IntentRow], now: &str) -> Decision {
    use crate::Scope;

    let mut any_permission = false;

    for intent in intents {
        let resource_matches = match intent.scope {
            Scope::Resource => request.resource == intent.resource_uri,
            Scope::Subtree => request.resource.starts_with(&intent.resource_uri),
        };
        if !resource_matches {
            continue;
        }

        if !mode_matches(&request.mode, &intent.mode) {
            continue;
        }

        if !audience_matches_odrl(request, &intent.audience) {
            continue;
        }

        if !evaluate_condition(&intent.condition, now) {
            continue;
        }

        if intent.effect == Effect::Allow {
            any_permission = true;
        }
    }

    // ODRL: Permission overrides Prohibition when both match.
    // Fail-closed: no Permission → Deny regardless of whether Prohibition matched.
    if any_permission {
        Decision::Allow
    } else {
        Decision::Deny
    }
}

/// Check whether the request's agent/client matches an ODRL assignee.
fn audience_matches_odrl(request: &Request, audience: &Audience) -> bool {
    match audience {
        Audience::Public => true,
        Audience::Authenticated => !request.agent.is_empty(),
        Audience::Owner => request.agent == format!("{}#owner", request.resource),
        Audience::Agent(a) => &request.agent == a,
        Audience::Group(g) => {
            // ODRL PartyCollection: group membership resolved by generators.
            request.agent.starts_with(g.as_str())
        }
        Audience::ClientRestricted { agent, client } => {
            &request.agent == agent
                && request.client.as_deref() == Some(client.as_str())
        }
        Audience::AllExcept(excl) => {
            !excl.iter().any(|e| e == &request.agent)
        }
    }
}

/// The **purpose declared by a request** for the benchmark's purpose-of-use evaluation.
///
/// A request carries no purpose field in the current [`Request`] IR (the generators do not
/// yet declare one — bead `sq-i6du2.4`/`.5`), so a `Condition::Purpose` constraint is
/// evaluated against this single pinned "authorized purpose" that the corpus grants for.
/// A purpose-constrained permission is therefore satisfied iff its purpose equals this
/// pinned value — modelling "this agent's session is running under the granted purpose".
/// Any other purpose is a deny (fail-closed), so a wrong-purpose ground truth is caught.
///
/// This is a design decision (recorded in the PR body): it keeps purpose evaluation
/// non-vacuous — a mismatched purpose denies — without inventing a `Request.purpose` field
/// that would break the sibling generator branches. When those branches add a declared
/// request purpose, this constant becomes the fixture default and the real field threads
/// through the same way `now` does for temporal windows.
pub(crate) const BENCH_GRANTED_PURPOSE: &str = "https://sparq.dev/vocab/purpose#reporting";

/// Evaluate an ODRL usage condition against a pinned evaluation instant `now`.
///
/// Clock-free by construction: `now` is a fixed-width UTC ISO-8601 string passed in by the
/// caller (see [`BENCH_EVAL_INSTANT`]); nothing here reads the system clock, so the result
/// is a pure function of `(condition, now)`.
///
/// - `Condition::None` — always satisfied.
/// - `Condition::Temporal { start, end }` — satisfied iff `start ≤ now < end` (half-open
///   window). Comparison is lexicographic, which is exact for the fixed-width `Z`-suffixed
///   UTC ISO-8601 form the generators emit (same width, same zone ⇒ lexical order equals
///   chronological order). An **expired** window (`now ≥ end`) or a **not-yet-open** one
///   (`now < start`) is therefore a deny — the correct ground truth for a retention /
///   embargo constraint.
/// - `Condition::Purpose(p)` — satisfied iff `p` equals the pinned
///   [`BENCH_GRANTED_PURPOSE`]; any other purpose denies (fail-closed).
/// - `Condition::Count(n)` — satisfied iff `n > 0` (a zero-count grant admits nothing).
/// - `Condition::And(a, b)` — conjunction; both sub-conditions must hold.
///
/// # Fail-closed
/// A malformed temporal window (a bound that is not parseable as the expected fixed-width
/// form, detected by a length check) returns `false` (deny) rather than silently admitting.
pub(crate) fn evaluate_condition(condition: &Condition, now: &str) -> bool {
    match condition {
        Condition::None => true,
        Condition::Temporal { start, end } => temporal_in_window(start, end, now),
        Condition::Purpose(p) => p == BENCH_GRANTED_PURPOSE,
        Condition::Count(n) => *n > 0,
        Condition::And(a, b) => {
            evaluate_condition(a, now) && evaluate_condition(b, now)
        }
    }
}

/// Half-open temporal-window membership test: `start ≤ now < end`.
///
/// The three arguments are all fixed-width `Z`-suffixed UTC ISO-8601 instants
/// (`YYYY-MM-DDTHH:MM:SSZ`). For that canonical form, lexicographic `str` ordering is
/// identical to chronological ordering, so no date parsing (and no timezone / DST logic,
/// and no external crate) is needed — this is what keeps the oracle simple and trustworthy.
///
/// # Fail-closed
/// If any of the three instants is not the expected 20-character fixed-width form, the
/// window is treated as unsatisfiable (`false`) — a malformed bound must never silently
/// admit access.
fn temporal_in_window(start: &str, end: &str, now: &str) -> bool {
    // Canonical form is exactly `YYYY-MM-DDTHH:MM:SSZ` = 20 chars. A differing width would
    // break the lexicographic ⇔ chronological equivalence, so reject it fail-closed.
    const CANONICAL_LEN: usize = 20;
    if start.len() != CANONICAL_LEN || end.len() != CANONICAL_LEN || now.len() != CANONICAL_LEN {
        return false;
    }
    start <= now && now < end
}

// ── Shared helpers ───────────────────────────────────────────────────────────────────

/// Check that the requested modes are all covered by the intent's modes.
/// The intent must grant AT LEAST the requested modes.
fn mode_matches(requested: &crate::AccessMode, granted: &crate::AccessMode) -> bool {
    (!requested.read || granted.read)
        && (!requested.write || granted.write)
        && (!requested.control || granted.control)
}

// ── Workload-engine model dispatch (bead `sq-i6du2.6`) ────────────────────────────────

/// Dispatch a request to the by-construction oracle for a given [`AcModel`].
///
/// This is the single entry point the workload engine (`workload.rs`, W1/W3/W4) uses to
/// obtain the ground-truth decision for a `(request, model)` pair. It is a thin router
/// over [`evaluate_wac`], [`evaluate_acp`], and [`evaluate_odrl`] so the workload engine
/// never re-implements per-model semantics (single source of truth for the oracle).
///
/// **Structural independence**: like the three underlying evaluators, this function reads
/// only the intent table — it never links or calls any sparq crate. That independence is
/// the whole point of the benchmark: an unsound system-under-test must *disagree* with
/// this oracle and thereby FAIL the harness.
///
/// **Determinism**: pure function of `(model, request, intents)`.
#[must_use]
pub(crate) fn evaluate(
    model: &crate::AcModel,
    request: &Request,
    intents: &[IntentRow],
) -> Decision {
    match model {
        crate::AcModel::Wac => evaluate_wac(request, intents),
        crate::AcModel::Acp => evaluate_acp(request, intents),
        crate::AcModel::Odrl => evaluate_odrl(request, intents),
    }
}

// ── Direct unit tests for the clock-free condition evaluator (bead `sq-i6du2.6`) ──────
//
// These pin the condition-eval semantics at the unit level (per-branch, exact value) so a
// comparison/boundary mutant in the temporal window test or a stubbed-out condition arm
// goes red WITHOUT needing a full generator corpus. The integration lane in
// `tests/workloads.rs` covers the model-differentiating dispatch; this covers the leaf
// evaluator's arms directly.
#[cfg(test)]
mod tests {
    use super::{
        BENCH_EVAL_INSTANT, BENCH_GRANTED_PURPOSE, evaluate_condition, evaluate_wac,
        temporal_in_window, wac_effective_acl, wac_is_member, wac_parent_container,
    };
    use crate::{
        AccessMode, Audience, Condition, Decision, Effect, IntentRow, Request, Scope,
    };

    // ── WAC nearest-ACL-document-wins helpers (bead `sq-o4orz` / `sq-kvvcl.2`) ────────
    //
    // These pin the ACL-document resolution directly, including the fail-closed edges the
    // four generators never produce (authority root, scheme-less IRI, non-slash container).

    const POD: &str = "https://pod.ex/u/";
    const ALICE: &str = "https://alice.ex/card#me";
    const BOB: &str = "https://bob.ex/card#me";

    /// A WAC-expressible `Allow` row granting `audience` read at `resource_uri`.
    fn row(audience: Audience, scope: Scope, resource_uri: &str) -> IntentRow {
        IntentRow {
            audience,
            scope,
            mode: AccessMode::read_only(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: resource_uri.to_string(),
        }
    }

    fn read_req(agent: &str, resource: &str) -> Request {
        Request {
            agent: agent.to_string(),
            client: None,
            resource: resource.to_string(),
            mode: AccessMode::read_only(),
        }
    }

    #[test]
    fn parent_container_walks_to_the_authority_root_and_stops() {
        assert_eq!(wac_parent_container("https://pod.ex/a/b/doc"), Some("https://pod.ex/a/b/"));
        // A trailing slash is stripped exactly once, so a container parents ABOVE itself.
        assert_eq!(wac_parent_container("https://pod.ex/a/b/"), Some("https://pod.ex/a/"));
        assert_eq!(wac_parent_container("https://pod.ex/a/"), Some("https://pod.ex/"));
        // At/above the authority root there is no parent — the walk terminates.
        assert_eq!(wac_parent_container("https://pod.ex/"), None);
        // Fail-closed on shapes the walk cannot reason about.
        assert_eq!(wac_parent_container("urn:sparq:auth"), None);
        assert_eq!(wac_parent_container("not-an-iri"), None);
    }

    #[test]
    fn membership_excludes_the_container_itself_and_non_slash_containers() {
        assert!(wac_is_member("https://pod.ex/c/", "https://pod.ex/c/doc"));
        assert!(wac_is_member("https://pod.ex/c/", "https://pod.ex/c/sub/doc"));
        // A container is NOT a member of itself (`acl:default` reaches members only).
        assert!(!wac_is_member("https://pod.ex/c/", "https://pod.ex/c/"));
        // A non-slash `resource_uri` names a plain resource; the parent chain, which only
        // ever yields slash-terminated IRIs, can never reach it, so it inherits to nothing.
        assert!(!wac_is_member("https://pod.ex/c", "https://pod.ex/c/doc"));
        // Prefix-but-not-descendant must not match.
        assert!(!wac_is_member("https://pod.ex/c/", "https://pod.ex/other"));
    }

    #[test]
    fn effective_acl_prefers_the_resource_own_document_then_the_nearest_ancestor() {
        let doc = "https://pod.ex/u/a/doc";
        let intents = vec![
            row(Audience::Agent(ALICE.to_string()), Scope::Subtree, POD),
            row(Audience::Agent(BOB.to_string()), Scope::Resource, doc),
        ];
        // Own ACL wins and is flagged as own.
        assert_eq!(wac_effective_acl(doc, &intents), Some((doc, true)));
        // A sibling with no own ACL falls back to the nearest ancestor that has one.
        assert_eq!(
            wac_effective_acl("https://pod.ex/u/a/other", &intents),
            Some((POD, false))
        );
        // Nothing anywhere up the chain → fail-closed.
        assert_eq!(wac_effective_acl("https://elsewhere.ex/x/y", &intents), None);
    }

    #[test]
    fn nearest_acl_shadows_the_ancestor_grant() {
        let shadowed = "https://pod.ex/u/a/bobs-doc";
        let inherited = "https://pod.ex/u/a/plain-doc";
        let intents = vec![
            // Pod-root subtree grant to alice.
            row(Audience::Agent(ALICE.to_string()), Scope::Subtree, POD),
            // `bobs-doc` carries its OWN ACL, naming only bob.
            row(Audience::Agent(BOB.to_string()), Scope::Resource, shadowed),
        ];

        // The closer ACL fully shadows the pod-root grant — alice is DENIED even though the
        // ancestor grant's URI is a prefix of the resource. This is the exact case a
        // `starts_with` union got wrong (oracle=Allow vs engine=Deny under-share).
        assert_eq!(evaluate_wac(&read_req(ALICE, shadowed), &intents), Decision::Deny);
        assert_eq!(evaluate_wac(&read_req(BOB, shadowed), &intents), Decision::Allow);

        // A resource with NO own ACL still inherits the pod-root grant.
        assert_eq!(evaluate_wac(&read_req(ALICE, inherited), &intents), Decision::Allow);
        assert_eq!(evaluate_wac(&read_req(BOB, inherited), &intents), Decision::Deny);
    }

    #[test]
    fn an_inexpressible_row_creates_no_acl_document_and_cannot_shadow() {
        let doc = "https://pod.ex/u/a/doc";
        for inexpressible in [
            // WAC has no deny — `compile_wac` emits an EMPTY policy, so no ACL document.
            IntentRow { effect: Effect::Deny, ..row(Audience::Public, Scope::Resource, doc) },
            // WAC has no usage conditions — likewise empty.
            IntentRow {
                condition: Condition::Purpose("https://ex.dev/p".to_string()),
                ..row(Audience::Public, Scope::Resource, doc)
            },
            // An UNBOUNDED `AllExcept` (empty exclusion set) is the third empty-policy
            // return in `compile_wac` — inside the audience match, not an early return.
            row(Audience::AllExcept(vec![]), Scope::Resource, doc),
        ] {
            let intents = vec![
                row(Audience::Agent(ALICE.to_string()), Scope::Subtree, POD),
                inexpressible,
            ];
            // The row materializes nothing, so the pod-root ACL still governs `doc`.
            assert_eq!(wac_effective_acl(doc, &intents), Some((POD, false)));
            assert_eq!(evaluate_wac(&read_req(ALICE, doc), &intents), Decision::Allow);
        }
    }

    #[test]
    fn a_subtree_row_grants_the_container_resource_itself() {
        // `compile_wac` emits `acl:accessTo` alongside `acl:default` for a subtree row, so
        // the container itself is reachable — otherwise a container-targeted request would
        // be denied for everyone and the U2 project-container lane would go vacuously Deny.
        let intents = vec![row(Audience::Agent(ALICE.to_string()), Scope::Subtree, POD)];
        assert_eq!(evaluate_wac(&read_req(ALICE, POD), &intents), Decision::Allow);
        assert_eq!(evaluate_wac(&read_req(BOB, POD), &intents), Decision::Deny);
    }

    #[test]
    fn wac_is_fail_closed_when_no_acl_document_governs_the_resource() {
        let intents = vec![row(Audience::Public, Scope::Resource, "https://pod.ex/u/a/doc")];
        // Public grant exists, but it governs a DIFFERENT resource with no ancestry link.
        assert_eq!(
            evaluate_wac(&read_req(ALICE, "https://other.ex/z"), &intents),
            Decision::Deny
        );
    }

    #[test]
    fn none_condition_is_always_satisfied() {
        assert!(evaluate_condition(&Condition::None, BENCH_EVAL_INSTANT));
    }

    #[test]
    fn temporal_window_half_open_boundaries() {
        // start ≤ now < end. Same-width canonical instants ⇒ lexical == chronological.
        assert!(temporal_in_window(
            "2026-01-01T00:00:00Z",
            "2027-01-01T00:00:00Z",
            "2026-07-06T00:00:00Z"
        ));
        // now == start is IN (inclusive lower bound).
        assert!(temporal_in_window(
            "2026-07-06T00:00:00Z",
            "2027-01-01T00:00:00Z",
            "2026-07-06T00:00:00Z"
        ));
        // now == end is OUT (exclusive upper bound).
        assert!(!temporal_in_window(
            "2026-01-01T00:00:00Z",
            "2026-07-06T00:00:00Z",
            "2026-07-06T00:00:00Z"
        ));
        // expired (now ≥ end).
        assert!(!temporal_in_window(
            "2020-01-01T00:00:00Z",
            "2021-01-01T00:00:00Z",
            "2026-07-06T00:00:00Z"
        ));
        // not yet open (now < start).
        assert!(!temporal_in_window(
            "2099-01-01T00:00:00Z",
            "2100-01-01T00:00:00Z",
            "2026-07-06T00:00:00Z"
        ));
    }

    #[test]
    fn temporal_malformed_bound_fails_closed() {
        // A non-canonical width breaks the lexical⇔chronological equivalence ⇒ deny.
        assert!(!temporal_in_window("2026", "2027-01-01T00:00:00Z", BENCH_EVAL_INSTANT));
        assert!(!temporal_in_window(
            "2026-01-01T00:00:00Z",
            "not-a-date",
            BENCH_EVAL_INSTANT
        ));
        assert!(!temporal_in_window(
            "2026-01-01T00:00:00Z",
            "2027-01-01T00:00:00Z",
            "now"
        ));
    }

    #[test]
    fn temporal_condition_uses_pinned_instant() {
        let valid = Condition::Temporal {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2027-01-01T00:00:00Z".to_string(),
        };
        let expired = Condition::Temporal {
            start: "2020-01-01T00:00:00Z".to_string(),
            end: "2021-01-01T00:00:00Z".to_string(),
        };
        assert!(evaluate_condition(&valid, BENCH_EVAL_INSTANT));
        assert!(!evaluate_condition(&expired, BENCH_EVAL_INSTANT));
    }

    #[test]
    fn purpose_condition_matches_only_granted_purpose() {
        assert!(evaluate_condition(
            &Condition::Purpose(BENCH_GRANTED_PURPOSE.to_string()),
            BENCH_EVAL_INSTANT
        ));
        assert!(!evaluate_condition(
            &Condition::Purpose("https://sparq.dev/vocab/purpose#marketing".to_string()),
            BENCH_EVAL_INSTANT
        ));
    }

    #[test]
    fn count_condition_zero_denies_positive_admits() {
        assert!(!evaluate_condition(&Condition::Count(0), BENCH_EVAL_INSTANT));
        assert!(evaluate_condition(&Condition::Count(1), BENCH_EVAL_INSTANT));
    }

    #[test]
    fn and_condition_is_conjunction() {
        let both_true = Condition::And(
            Box::new(Condition::Purpose(BENCH_GRANTED_PURPOSE.to_string())),
            Box::new(Condition::Temporal {
                start: "2026-01-01T00:00:00Z".to_string(),
                end: "2027-01-01T00:00:00Z".to_string(),
            }),
        );
        assert!(evaluate_condition(&both_true, BENCH_EVAL_INSTANT));

        // One expired arm makes the conjunction false.
        let one_expired = Condition::And(
            Box::new(Condition::Purpose(BENCH_GRANTED_PURPOSE.to_string())),
            Box::new(Condition::Temporal {
                start: "2020-01-01T00:00:00Z".to_string(),
                end: "2021-01-01T00:00:00Z".to_string(),
            }),
        );
        assert!(!evaluate_condition(&one_expired, BENCH_EVAL_INSTANT));

        // Wrong purpose also makes it false.
        let wrong_purpose = Condition::And(
            Box::new(Condition::Purpose("https://sparq.dev/vocab/purpose#other".to_string())),
            Box::new(Condition::Temporal {
                start: "2026-01-01T00:00:00Z".to_string(),
                end: "2027-01-01T00:00:00Z".to_string(),
            }),
        );
        assert!(!evaluate_condition(&wrong_purpose, BENCH_EVAL_INSTANT));
    }
}

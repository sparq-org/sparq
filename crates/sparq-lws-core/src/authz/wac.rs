// AUTHORED-BY Claude Opus 4.8
//! The Web Access Control authorizer (Solid WAC).
//!
//! Resolves the effective `.acl` for a target by walking the container hierarchy (`acl:default`
//! inheritance), reads each candidate ACL **through the [`Store`]** (ACLs are RDF resources), parses
//! it with `oxttl`/`oxjsonld`, and computes the modes granted to the requester. Denies with `401`
//! when the requester is anonymous (so the client authenticates), `403` when authenticated but
//! unauthorized — exactly the prod-solid-server `src/authz/wac.ts` semantics.
//!
//! ## ACL resolution (WAC)
//!  1. The target's OWN ACL (`<target>.acl` for a document, `<container>/.acl` for a container) — if
//!     present, only its `acl:accessTo <target>` rules apply.
//!  2. Otherwise the NEAREST ancestor container that HAS an ACL — its `acl:default` rules apply
//!     (inheritance). The search proceeds child→root and STOPS at the first ACL found (a closer ACL
//!     fully overrides a more distant one — WAC does not union across levels).
//!  3. If no ACL exists anywhere up to and including the storage root, access is DENIED (fail-closed).
//!
//! Reading/writing an `.acl` resource itself requires `acl:Control`; [`super::mode::mode_for_operation`]
//! encodes that, and the protected resource the ACL belongs to is what this resolver gates.
//!
//! ## Fail-closed on store error
//! A `NotFound` reading an ACL is the COMMON case (most resources inherit) → "no own ACL, keep
//! walking". Any OTHER store error (a transient backend failure) PROPAGATES — it must never be
//! silently treated as "no ACL" (that would fail OPEN by skipping a real ACL).
//!
//! ## `acl:agentGroup` membership
//! A rule may name a group instead of an agent. Because the pure matcher in [`super::acl`] does no
//! I/O, this authorizer resolves membership first and hands the VERIFIED groups to the matcher:
//! after the effective ACL is resolved, [`super::acl::agent_group_iris`] names the groups that could
//! still change the outcome; each group's document (the group IRI minus its fragment) is read
//! THROUGH the same [`Store`] seam and tested for `<group> vcard:hasMember <webid>`. Everything
//! about the path is fail-closed and it can only ever WIDEN a grant on a positive, resolved
//! membership:
//!  - an ACL naming no applicable group does ZERO extra I/O — the decision is bit-for-bit the
//!    pre-group one, so ordinary requests are untouched;
//!  - an anonymous requester is never resolved (a `vcard:hasMember` can only name a WebID);
//!  - a MISSING group document, a malformed one (it parses to an empty triple set, exactly as a
//!    malformed ACL does), or one that simply does not list the requester ⇒ no membership;
//!  - a group document on ANOTHER origin is left UNRESOLVED rather than fetched — this engine reads
//!    only through the `Store` (the pod's own resources) and performs no request-driven outbound
//!    HTTP, so a remote group grants nothing;
//!  - at most [`MAX_GROUP_DOCUMENTS`] distinct documents are read per decision;
//!  - the group document is read with SERVER authority (no nested WAC decision), exactly as the
//!    `.acl` itself is. That is not a read primitive handed to the requester: the ACL author — who
//!    holds `acl:Control` — chooses both the document and the group IRI, no document content is
//!    returned, and the only fact that can reach the decision is whether that document says
//!    `<group> vcard:hasMember <the requester's own WebID>`;
//!  - any OTHER store error PROPAGATES, exactly as for an ACL read — a transient backend failure
//!    must not be silently read as "not a member" (it is not evidence either way), so the request
//!    fails rather than quietly deciding on an incomplete membership picture.

use std::collections::BTreeSet;

use crate::acl_cache::AclCache;
use crate::error::ServerError;
use crate::ldp::content::{classify, parse_to_triples, RdfFormat};
use crate::store::Store;

use super::acl::{
    agent_group_iris, group_document_iri, group_has_member, modes_for, satisfies, AclScope,
    Requester,
};
use super::mode::{is_acl_resource, AccessMode, ACL_SUFFIX};
use super::wac_allow::EffectivePermissions;

/// The outcome of an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Permitted: the FULL set of modes the requester holds over the target (threaded into the
    /// `WAC-Allow` advertisement so a permitted read need not re-walk the hierarchy).
    Allow(BTreeSet<AccessMode>),
    /// Denied because the requester is ANONYMOUS → 401 + `WWW-Authenticate` (the client should obtain
    /// a token).
    Unauthenticated,
    /// Denied because the requester IS authenticated but lacks the required mode → 403.
    Forbidden,
}

/// The outcome of a single-pass READ authorization ([`WacAuthorizer::authorize_read`]). Mirrors
/// [`Decision`], but the `Allow` variant carries the full [`EffectivePermissions`] (the requester's
/// AND the public's modes) resolved in the SAME pass — so the read path builds `WAC-Allow` with no
/// further ACL work. The denial variants map to the SAME 401 (+ challenge) / 403 as [`Decision`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadDecision {
    /// Permitted: the effective `user` + `public` modes for the target, from one resolution.
    Allow(EffectivePermissions),
    /// Denied, requester anonymous → 401 + `WWW-Authenticate`.
    Unauthenticated,
    /// Denied, requester authenticated but unauthorized → 403.
    Forbidden,
}

/// One entry of the read path's ACL-candidate chain (nearest-first): the ACL document IRI to
/// probe, plus the resource it would GOVERN if present (the base the rules match against — the
/// protected resource for the own-ACL candidate, the ancestor container for an inherited one).
/// Produced by [`WacAuthorizer::read_plan_candidates`]; consumed, zipped with the combined
/// read-plan query's presence/etag rows, by [`WacAuthorizer::authorize_read_planned`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclCandidate {
    /// The candidate ACL document IRI (`<governed>.acl`).
    pub acl: String,
    /// The resource this ACL governs when present (the rule-matching base).
    pub governed: String,
}

/// The EFFECTIVE ACL governing a resource, resolved ONCE (the walk + read + parse) so it can be
/// evaluated against MULTIPLE requesters (e.g. a read's `user` + `public` audiences) without
/// re-resolving. `None` (no governing ACL anywhere) is the fail-closed case: no grants for anyone.
struct ResolvedAcl {
    /// The parsed ACL triples + the base resource the rules match against + the matching scope —
    /// `None` when no ACL governs the resource (fail-closed → empty modes for every requester).
    parsed: Option<(Vec<oxrdf::Triple>, String, AclScope)>,
}

impl ResolvedAcl {
    fn found(triples: Vec<oxrdf::Triple>, base: String, scope: AclScope) -> Self {
        Self {
            parsed: Some((triples, base, scope)),
        }
    }

    fn none() -> Self {
        Self { parsed: None }
    }

    /// The modes the resolved ACL grants `requester` — pure rule-matching over the already-parsed
    /// triples (no I/O). An unresolved ACL (`None`) grants nothing (fail-closed). Identical in result
    /// to the prior inline `modes_for(&triples, base, requester, scope)` call.
    fn modes_for(&self, requester: &Requester<'_>) -> BTreeSet<AccessMode> {
        match &self.parsed {
            Some((triples, base, scope)) => modes_for(triples, base, requester, *scope),
            None => BTreeSet::new(),
        }
    }

    /// The `acl:agentGroup` IRIs this resolution's applicable rules name for `requester` — the groups
    /// whose documents must be read before matching (see [`agent_group_iris`]). Empty for an
    /// unresolved ACL, an anonymous requester, or — the common case — an ACL naming no group, in
    /// which case the caller does no group I/O at all.
    fn agent_group_iris(&self, requester: &Requester<'_>) -> Vec<String> {
        match &self.parsed {
            Some((triples, base, scope)) => agent_group_iris(triples, base, requester, *scope)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            None => Vec::new(),
        }
    }
}

/// The maximum number of DISTINCT group documents ONE authorization decision will read while
/// resolving `acl:agentGroup` membership.
///
/// An `.acl` is written by the resource owner, so without a cap an ACL naming hundreds of groups
/// would turn a single request into that many store reads. Groups beyond the cap are left
/// UNRESOLVED, so they never match — the cap can only ever withhold a grant, never create one.
pub const MAX_GROUP_DOCUMENTS: usize = 16;

/// The Web Access Control authorizer over a [`Store`] and the server base URL.
///
/// Optionally fronted by a per-instance [`AclCache`] (read-path optimisation #3): when present, the
/// effective-ACL resolution reuses the PARSED triples of an UNCHANGED ACL across requests (keyed by
/// `(acl-iri, etag)`), skipping the byte-fetch + `oxttl` re-parse — without ever changing the decision
/// (the cache is never authoritative; see [`crate::acl_cache`]). When absent (`None`) the resolver
/// reads + parses every ACL every time — the pre-cache behaviour, also exactly what the `=0` disabled
/// cache yields.
pub struct WacAuthorizer<'a, S: Store> {
    store: &'a S,
    base_url: String,
    /// The shared, per-instance parsed-ACL cache (`None` ⇒ no caching, every ACL read+parsed afresh).
    acl_cache: Option<&'a AclCache>,
}

impl<'a, S: Store> WacAuthorizer<'a, S> {
    /// Build an authorizer with NO ACL cache — every effective-ACL resolution reads + parses each
    /// candidate ACL afresh (the pre-cache path; used by unit tests and any caller without a cache).
    pub fn new(store: &'a S, base_url: impl Into<String>) -> Self {
        Self {
            store,
            base_url: base_url.into(),
            acl_cache: None,
        }
    }

    /// Build an authorizer fronted by the per-instance [`AclCache`]. The cache reuses the PARSED
    /// triples of an unchanged ACL across requests (keyed by `(acl-iri, etag)`), never changing a
    /// decision (see [`crate::acl_cache`]).
    pub fn with_cache(store: &'a S, base_url: impl Into<String>, acl_cache: &'a AclCache) -> Self {
        Self {
            store,
            base_url: base_url.into(),
            acl_cache: Some(acl_cache),
        }
    }

    /// Authorize an operation: the `target` IRI, the `required` mode (from
    /// [`super::mode::mode_for_operation`]), the requester's `web_id` (`None` ⇒ anonymous), and the
    /// request's `origin` (the HTTP `Origin` header; `None` when the request carried none).
    ///
    /// The `origin` is threaded into rule-matching so an `acl:origin`-restricted authorization grants
    /// ONLY when the request's Origin matches one of the rule's `acl:origin` values (an app-scoping
    /// restriction); a rule with no `acl:origin` is unaffected by it.
    ///
    /// Resolves the effective ACL of the PROTECTED resource (for an `.acl` target that is the resource
    /// the ACL governs — Control of THAT resource gates reading/writing its `.acl`), computes the
    /// requester's modes, and returns a [`Decision`].
    pub async fn authorize(
        &self,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Decision, ServerError> {
        let protected = self.protected_resource(target);
        let granted = self.effective_modes(&protected, web_id, origin).await?;

        if satisfies(&granted, required) {
            return Ok(Decision::Allow(granted));
        }
        Ok(if web_id.is_none() {
            Decision::Unauthenticated
        } else {
            Decision::Forbidden
        })
    }

    /// Mode-generic authorization over an ALREADY-FETCHED read plan (write-2 — the write-path
    /// sibling of [`authorize_read_planned`](Self::authorize_read_planned)): identical
    /// [`Decision`] to [`authorize`](Self::authorize), but the O(depth) sequential ACL walk's
    /// presence/etag probes come from the caller's ONE combined [`crate::store::ReadPlan`]
    /// round-trip instead of k+1 per-candidate queries.
    ///
    /// `candidates` MUST be this authorizer's own
    /// [`read_plan_candidates`](Self::read_plan_candidates) for the same target, and `plan_acls`
    /// the [`crate::store::ReadPlan::acls`](crate::store::ReadPlan) produced FROM those
    /// candidates — the pairing is verified entry-by-entry by the shared planned resolver and any
    /// mismatch is FATAL (fail-closed), never a partial evaluation.
    ///
    /// SECURITY (the equivalence argument): the effective-ACL resolution is the SAME
    /// `resolve_effective_acl_planned` the READ path uses — nearest-first, absent-per-plan skipped,
    /// the ONE found ACL re-confirmed with a LIVE probe (`read_acl_confirmed`, fail-closed on
    /// delete-after-plan), no-candidate ⇒ `ResolvedAcl::none` (no grants). The decision is then
    /// computed by the SAME `modes_for` / `satisfies` helpers over the same parsed triples as
    /// [`authorize`](Self::authorize) — including the 401-vs-403 split on `web_id` — so the
    /// planned decision is bit-for-bit the sequential one. The differential tests in this module
    /// run BOTH paths over the full WAC case matrix for EVERY [`AccessMode`] (not just Read) and
    /// assert identical [`Decision`]s.
    pub async fn authorize_planned(
        &self,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
        candidates: &[AclCandidate],
        plan_acls: &[(String, Option<String>)],
    ) -> Result<Decision, ServerError> {
        let resolved = self
            .resolve_effective_acl_planned(candidates, plan_acls)
            .await?;
        // From here the logic is byte-identical to `authorize` (the same modes_with_groups +
        // satisfies + 401/403 split over the same `ResolvedAcl` shape).
        let granted = self.modes_with_groups(&resolved, web_id, origin).await?;
        if satisfies(&granted, required) {
            return Ok(Decision::Allow(granted));
        }
        Ok(if web_id.is_none() {
            Decision::Unauthenticated
        } else {
            Decision::Forbidden
        })
    }

    /// Single-pass READ authorization (Optimization #2): resolve the target's effective ACL ONCE and
    /// derive BOTH the access decision AND the `WAC-Allow` audiences from that single resolution.
    ///
    /// The GET/HEAD read path previously called [`authorize`](Self::authorize) (resolve the protected
    /// resource → walk + read + parse the `.acl` → compute the requester's modes) and THEN
    /// [`effective_permissions`](Self::effective_permissions) (a SECOND `WacAuthorizer`, a second
    /// `protected_resource`, and — for an authenticated requester — a SECOND full ACL walk/read/parse
    /// to compute the public set). This resolves the effective ACL EXACTLY ONCE
    /// (`resolve_effective_acl` — the only walk/read/parse) and derives
    /// BOTH audiences from that shared, already-parsed resolution via pure rule-matching:
    ///
    /// - the requester's modes (`user`) are the gate input AND the `WAC-Allow` `user` audience;
    /// - the access decision is `satisfies(user, required)` → `Allow` / `Unauthenticated` (anonymous) /
    ///   `Forbidden` (authenticated) — byte-identical to [`authorize`](Self::authorize);
    /// - the `public` audience is `user.clone()` for an anonymous requester (it IS the public — no
    ///   extra work), else a second `modes_for` over the SAME parsed triples against
    ///   `Requester { web_id: None, origin }` (the origin-scoped public set — identical RESULT to
    ///   [`effective_permissions`](Self::effective_permissions), preserving `acl:origin` semantics,
    ///   but WITHOUT a second ACL walk/read/parse). So an AUTHENTICATED read reads + parses the `.acl`
    ///   once, not twice.
    ///
    /// The returned [`ReadDecision::Allow`] carries the full [`EffectivePermissions`] (user + public),
    /// so the read handler needs no further ACL work to build `WAC-Allow`. The denial variants are the
    /// same as [`Decision`] so the handler maps them to the SAME 401 (+ challenge) / 403 as before.
    ///
    /// SECURITY: the decision (`satisfies(user, required)`, the 401-vs-403 split on `web_id`) and the
    /// `public`/`user` sets are computed by the SAME helpers (`modes_for`, `satisfies`) the split path
    /// used, against the SAME `protected_resource` and the SAME parsed ACL triples, so the gate and the
    /// advertisement are unchanged — including fail-closed on a missing/broken ACL
    /// (`ResolvedAcl::none` / an empty-triples `Some` both yield empty modes, which `satisfies`
    /// rejects for any required mode) and the origin-scoped public set.
    pub async fn authorize_read(
        &self,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) -> Result<ReadDecision, ServerError> {
        let protected = self.protected_resource(target);
        // Resolve the effective ACL ONCE (the only walk/read/parse — the expensive part). Both the
        // `user` and `public` audiences are then evaluated against this SHARED, already-parsed
        // resolution (pure rule-matching, no further I/O) — so an AUTHENTICATED read no longer
        // re-walks/re-reads/re-parses the same `.acl` for its public set (the roborev finding).
        let resolved = self.resolve_effective_acl(&protected).await?;

        // 1) The requester's modes — the gate input AND the `WAC-Allow` `user` audience. Any
        //    `acl:agentGroup` membership the resolved rules need is resolved here (and ONLY when
        //    such a rule exists — see `modes_with_groups`).
        let user = self.modes_with_groups(&resolved, web_id, origin).await?;

        // 2) The access decision — identical to `authorize`: a permitted read requires the resolved
        //    set to `satisfy` the required mode; a denial is 401 (anonymous) / 403 (authenticated).
        if !satisfies(&user, required) {
            return Ok(if web_id.is_none() {
                ReadDecision::Unauthenticated
            } else {
                ReadDecision::Forbidden
            });
        }

        // 3) The `public` audience, from the SAME resolution: `user.clone()` for an anonymous
        //    requester (it IS the public — no extra evaluation); else a second rule-match (NOT a
        //    second walk/read/parse) against the origin-scoped public requester — identical RESULT to
        //    `effective_permissions`, preserving `acl:origin` semantics, at the cost of only the pure
        //    `modes_for` over the already-parsed triples. Still no group I/O: the public has no
        //    WebID, so no `acl:agentGroup` rule can name it (`groups` stays empty — fail-closed).
        let public = if web_id.is_none() {
            user.clone()
        } else {
            resolved.modes_for(&Requester {
                web_id: None,
                origin,
                groups: &[],
            })
        };
        Ok(ReadDecision::Allow(EffectivePermissions { user, public }))
    }

    /// The full ACL-candidate chain for `target`, NEAREST-FIRST — the up-front, pure-string
    /// derivation the combined read-plan query (read-2, `research/lws-design-records.md` §7)
    /// needs: element 0 is the PROTECTED resource's OWN ACL (scope [`AclScope::AccessTo`]), the
    /// rest are its ancestors' ACLs child→root (scope [`AclScope::Default`]).
    ///
    /// Derived from the PROTECTED resource, not the raw target (the design's "two IRI roles"): a
    /// GET of `foo.acl` is governed by Control on `foo`, so its chain starts at `foo.acl` itself —
    /// deriving from the raw target would probe a non-existent `foo.acl.acl` and change
    /// ACL-resource authorization. Exactly the candidates the (private, sequential)
    /// `resolve_effective_acl` walk visits, in the same order.
    pub fn read_plan_candidates(&self, target: &str) -> Vec<AclCandidate> {
        let protected = self.protected_resource(target);
        let mut candidates = Vec::with_capacity(4);
        candidates.push(AclCandidate {
            acl: self.acl_for(&protected),
            governed: protected.clone(),
        });
        for ancestor in self.ancestors_nearest_first(&protected) {
            candidates.push(AclCandidate {
                acl: self.acl_for(&ancestor),
                governed: ancestor,
            });
        }
        candidates
    }

    /// Single-pass READ authorization over an ALREADY-FETCHED read plan (read-2): identical
    /// decision + `WAC-Allow` audiences to [`authorize_read`](Self::authorize_read), but the ACL
    /// walk's presence/etag probes come from the caller's ONE combined [`crate::store::ReadPlan`]
    /// round-trip instead of k+1 sequential per-candidate queries.
    ///
    /// `candidates` MUST be this authorizer's own
    /// [`read_plan_candidates`](Self::read_plan_candidates) for the same target, and `plan_acls` the
    /// [`crate::store::ReadPlan::acls`] produced FROM those candidates — the pairing is verified
    /// entry-by-entry and any mismatch is a FATAL error (fail-closed), never a partial evaluation.
    ///
    /// SECURITY (the equivalence argument): the walk semantics are exactly the sequential
    /// `resolve_effective_acl`'s —
    /// - candidates are visited in the SAME nearest-first order; the FIRST present one governs
    ///   (a closer ACL fully overrides a more distant one — no union across levels);
    /// - a candidate the PLAN reports ABSENT is skipped WITHOUT a probe (this is the walk-collapse
    ///   win: the plan already told us, authoritatively as of plan time, that no `.acl` sits there);
    /// - a candidate the plan reports PRESENT is the found ACL — but its parse is NOT served from a
    ///   stale plan-time etag. The private `read_acl_confirmed` does a LIVE index probe of that one
    ///   ACL and gates the cache on its CURRENT etag — exactly `read_acl`'s live-`meta` semantics —
    ///   so an ACL DELETED between [`Store::read_plan`](crate::store::Store) and here is
    ///   treated as vanished (keep walking / fail-closed), never granted from a stale cache entry.
    ///   The plan-time etag alone is NOT a safe cache gate: a delete-after-plan leaves the plan
    ///   reporting present with a now-stale etag, and a cached parse under that etag would authorize
    ///   from a deleted ACL (the fail-open the live re-confirm closes);
    /// - no candidate present anywhere ⇒ `ResolvedAcl::none` (fail-closed, no grants);
    /// - the decision + both `WAC-Allow` audiences are then computed by the SAME `modes_for` /
    ///   `satisfies` helpers over the same parsed triples as
    ///   [`authorize_read`](Self::authorize_read).
    ///
    /// The differential tests in this module run BOTH paths over the full WAC case matrix — AND the
    /// delete-after-plan / rotate-after-plan CACHED windows — and assert identical [`ReadDecision`]s
    /// (the security oracle: bit-for-bit with the sequential walk, incl. the cached-delete case).
    pub async fn authorize_read_planned(
        &self,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
        candidates: &[AclCandidate],
        plan_acls: &[(String, Option<String>)],
    ) -> Result<ReadDecision, ServerError> {
        let resolved = self
            .resolve_effective_acl_planned(candidates, plan_acls)
            .await?;

        // From here the logic is byte-identical to `authorize_read` (the same steps 1–3 over the
        // same `ResolvedAcl` shape), including the `acl:agentGroup` resolution.
        let user = self.modes_with_groups(&resolved, web_id, origin).await?;
        if !satisfies(&user, required) {
            return Ok(if web_id.is_none() {
                ReadDecision::Unauthenticated
            } else {
                ReadDecision::Forbidden
            });
        }
        let public = if web_id.is_none() {
            user.clone()
        } else {
            resolved.modes_for(&Requester {
                web_id: None,
                origin,
                groups: &[],
            })
        };
        Ok(ReadDecision::Allow(EffectivePermissions { user, public }))
    }

    /// Resolve the effective ACL from the plan's presence/etag rows — the in-memory walk over the
    /// combined query's results (§3.1: "the resolver then walks the candidate list in memory,
    /// nearest-first: first present row wins").
    async fn resolve_effective_acl_planned(
        &self,
        candidates: &[AclCandidate],
        plan_acls: &[(String, Option<String>)],
    ) -> Result<ResolvedAcl, ServerError> {
        // The plan rows MUST pair 1:1 with the candidates they were derived from. A mismatch is a
        // programming error — FAIL CLOSED (refuse to authorize), never evaluate a partial/shifted
        // chain.
        if candidates.len() != plan_acls.len() {
            return Err(ServerError::Storage(
                "read-plan/candidate length mismatch".into(),
            ));
        }
        for (idx, (candidate, (plan_iri, etag))) in
            candidates.iter().zip(plan_acls.iter()).enumerate()
        {
            if candidate.acl != *plan_iri {
                return Err(ServerError::Storage(
                    "read-plan/candidate IRI mismatch".into(),
                ));
            }
            // Plan reports ABSENT ⇒ skip WITHOUT a probe. The plan consulted SPARQ (not a cache), so
            // this is authoritatively "no `.acl` here as of plan time" — the walk-collapse win: the
            // O(depth) absent-candidate probes the sequential walk pays are folded into the one
            // combined read-plan query. (A create-after-plan of a nearer ACL is snapshot-deferred to
            // the next request, symmetric with any snapshot; the SECURITY-critical direction — a
            // DELETE not granting — is enforced below by a LIVE re-confirm, never snapshot-trusted.)
            if etag.is_none() {
                continue;
            }
            // Plan reports PRESENT ⇒ this is the found ACL. Do NOT trust the plan-time etag as the
            // cache gate: re-confirm the ACL's CURRENT existence with a live probe (`read_acl_confirmed`).
            // `None` ⇒ the ACL was DELETED between the plan and here (or vanished mid-read) — fail-closed,
            // keep walking, exactly as the sequential walk's live `store.meta` → `NotFound` does.
            if let Some(triples) = self.read_acl_confirmed(&candidate.acl).await? {
                let scope = if idx == 0 {
                    AclScope::AccessTo
                } else {
                    AclScope::Default
                };
                return Ok(ResolvedAcl::found(
                    triples,
                    candidate.governed.clone(),
                    scope,
                ));
            }
        }
        // No candidate present anywhere ⇒ no grants (fail-closed) — identical to the walk.
        Ok(ResolvedAcl::none())
    }

    /// Read + parse ONE ACL the read plan reported PRESENT, re-confirming its CURRENT existence with
    /// a LIVE index probe — the fail-closed twin of [`read_acl`](Self::read_acl) for the planned
    /// path. Semantics are bit-for-bit [`read_acl`](Self::read_acl)'s, so the planned decision equals
    /// the sequential one:
    ///  - LIVE `store.meta(acl)` first: `None` ⇒ the ACL is GONE NOW (deleted since the plan) ⇒
    ///    `Ok(None)`, keep walking (fail-closed). This is the security core — the plan's plan-time
    ///    etag is NEVER used as the cache gate, so a delete-after-plan can never serve a stale grant;
    ///  - present ⇒ gate the cache on the ACL's CURRENT etag: a HIT reuses the cached parse (bytes
    ///    provably unchanged — a rotation changes the etag and misses), a MISS fetches the bytes
    ///    through the JUST-PROBED metadata (`read_at` — no duplicate `get_meta`, the §3.3 F2 fix) and
    ///    parses, refreshing the cache under that same current etag;
    ///  - the ACL vanished between the probe and the byte fetch (a concurrent DELETE) ⇒ `read_at`
    ///    surfaces `NotFound` ⇒ `Ok(None)` (keep walking), matching `read_acl`'s post-probe race;
    ///  - no cache attached ⇒ the plain live read+parse ([`read_and_parse_acl`](Self::read_and_parse_acl),
    ///    which itself re-reads the store live) — already fail-closed on a delete;
    ///  - a malformed body parses to an EMPTY triple set (PRESENT-but-granting-nothing, fail-closed)
    ///    via the shared [`parse_acl_body`](Self::parse_acl_body);
    ///  - any non-`NotFound` store error PROPAGATES (never treated as "no ACL" — fail-closed).
    ///
    /// The plan's role for this candidate was only to IDENTIFY it as the nearest present one (folding
    /// away the O(depth) absent-candidate probes); its existence is then re-confirmed here, so the
    /// combined-query win is the WALK collapse, not a skipped existence check on the governing ACL.
    async fn read_acl_confirmed(
        &self,
        acl: &str,
    ) -> Result<Option<Vec<oxrdf::Triple>>, ServerError> {
        // No cache: the plain path already re-reads the store LIVE (a deleted ACL ⇒ NotFound ⇒
        // Ok(None)), so it is fail-closed on a delete without any extra work.
        let Some(cache) = self.acl_cache else {
            return self.read_and_parse_acl(acl).await;
        };
        // LIVE existence re-confirm (the fix): the ACL must still exist NOW. An absent ACL ⇒ vanished
        // (deleted since the plan) ⇒ keep walking — the cache is never consulted for a gone ACL.
        let meta = match self.store.meta(acl).await? {
            Some(m) => m,
            None => return Ok(None),
        };
        let now = Self::now_secs();
        // Gate the cache on the ACL's CURRENT etag (not the plan's): a rotation misses + re-parses.
        if let Some(triples) = cache.get(acl, &meta.etag, now) {
            return Ok(Some(triples));
        }
        // Miss: fetch the bytes through the just-probed metadata (read_at — no duplicate get_meta).
        // A concurrent DELETE between the probe and the fetch surfaces as NotFound ⇒ vanished
        // (keep walking); any other store error propagates (fail-closed) — matching `read_acl`.
        let body = match self.store.read_at(acl, &meta).await {
            Ok(b) => b,
            Err(ServerError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let resource = crate::store::Resource { body, meta };
        let triples = Self::parse_acl_body(&resource, acl);
        cache.insert(acl, &resource.meta.etag, triples.clone(), now);
        Ok(Some(triples))
    }

    /// The effective access modes a `WAC-Allow` header should advertise on a permitted read of
    /// `target`: `user` (what the requester may do) and `public` (what an unauthenticated agent may
    /// do), both from the SAME effective ACL.
    ///
    /// `user_modes`, when supplied, is the requester's already-resolved mode set (e.g. the value a
    /// prior [`authorize`](Self::authorize) returned for the SAME target+web_id) — passing it skips
    /// recomputing `user`. For an anonymous requester `public == user` and no extra work is done.
    pub async fn effective_permissions(
        &self,
        target: &str,
        web_id: Option<&str>,
        origin: Option<&str>,
        user_modes: Option<BTreeSet<AccessMode>>,
    ) -> Result<EffectivePermissions, ServerError> {
        let protected = self.protected_resource(target);

        let user = match user_modes {
            Some(m) => m,
            None => self.effective_modes(&protected, web_id, origin).await?,
        };
        // The public set: for an anonymous requester it EQUALS user (an anonymous requester IS the
        // public); for an authenticated requester it is resolved independently against the public —
        // an ANONYMOUS IDENTITY (no WebID) but carrying THIS request's `origin`. Threading the Origin
        // matters: an ORIGIN-SCOPED public grant (`acl:agentClass foaf:Agent` + `acl:origin <o>`)
        // grants the public ONLY from a matching Origin. Resolving the public set with
        // `Requester::anonymous()` (origin `None`) would always FAIL such an `acl:origin`-restricted
        // public rule and so UNDER-REPORT `public=` even when the current request's Origin satisfies
        // it. Using `Requester { web_id: None, origin }` reports exactly the public modes available
        // from the request's own Origin (and still omits them when the Origin does not match / is
        // absent — fail-closed in `matches_origin`).
        let public = if web_id.is_none() {
            user.clone()
        } else {
            self.effective_modes(&protected, None, origin).await?
        };
        Ok(EffectivePermissions { user, public })
    }

    /// The modes granted to `requester` over `resource` by the effective ACL (its OWN ACL via
    /// `acl:accessTo`, else the nearest ancestor's `acl:default`). Empty set when no ACL governs it
    /// (fail-closed).
    ///
    /// Implemented as resolve-once ([`resolve_effective_acl`](Self::resolve_effective_acl)) + evaluate
    /// the requester ([`modes_with_groups`](Self::modes_with_groups)) — so a caller that needs SEVERAL
    /// audiences over the same resource (e.g. the read path's `user` + `public`) can resolve once and
    /// evaluate many.
    async fn effective_modes(
        &self,
        resource: &str,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) -> Result<BTreeSet<AccessMode>, ServerError> {
        let resolved = self.resolve_effective_acl(resource).await?;
        self.modes_with_groups(&resolved, web_id, origin).await
    }

    /// The modes an ALREADY-RESOLVED ACL grants the requester, first resolving any `acl:agentGroup`
    /// membership those rules need ([`resolve_group_memberships`](Self::resolve_group_memberships)).
    ///
    /// This is the ONLY place a group grant can enter a decision, and it is a strict extension of the
    /// pure [`ResolvedAcl::modes_for`]: when no applicable rule names a group — the overwhelmingly
    /// common case — the resolved membership set is empty, no I/O happens, and the result is
    /// bit-for-bit what the pre-group matcher returned.
    async fn modes_with_groups(
        &self,
        resolved: &ResolvedAcl,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) -> Result<BTreeSet<AccessMode>, ServerError> {
        let groups = self
            .resolve_group_memberships(resolved, web_id, origin)
            .await?;
        Ok(resolved.modes_for(&Requester {
            web_id,
            origin,
            groups: &groups,
        }))
    }

    /// Resolve which of the `acl:agentGroup`s named by the resolved ACL actually list this requester
    /// as a member — the I/O half of group matching (the pure half is [`group_has_member`]).
    ///
    /// Each referenced group's DOCUMENT (the group IRI minus its fragment) is read ONCE through the
    /// [`Store`] and every group in it tested, so several groups sharing a document cost one read.
    /// Returns the group IRIs whose membership was POSITIVELY established; everything else is
    /// fail-closed:
    ///  - anonymous requester, or no applicable group ⇒ empty, with NO store access at all;
    ///  - a group document on another origin ⇒ skipped unresolved (no outbound fetch — the `Store`
    ///    holds this pod's resources, and authorization performs no request-driven egress);
    ///  - beyond [`MAX_GROUP_DOCUMENTS`] distinct documents ⇒ skipped unresolved;
    ///  - an ABSENT document ⇒ no membership (it cannot vouch for anyone);
    ///  - a MALFORMED document ⇒ an empty triple set via [`parse_acl_body`](Self::parse_acl_body) ⇒
    ///    no membership, the same present-but-granting-nothing posture a malformed ACL gets;
    ///  - any OTHER store error PROPAGATES — a transient backend failure is not evidence of
    ///    non-membership, so the request fails rather than deciding on an incomplete picture.
    async fn resolve_group_memberships(
        &self,
        resolved: &ResolvedAcl,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Vec<String>, ServerError> {
        // Only a WebID can be a `vcard:hasMember`, so the public/anonymous audience never resolves.
        let Some(web_id) = web_id else {
            return Ok(Vec::new());
        };
        let referenced = resolved.agent_group_iris(&Requester {
            web_id: Some(web_id),
            origin,
            groups: &[],
        });
        if referenced.is_empty() {
            // The hot path: no applicable rule names a group ⇒ zero extra I/O, decision unchanged.
            return Ok(Vec::new());
        }

        // De-duplicate to DOCUMENTS (several groups may live in one) and apply the read cap.
        let mut documents: Vec<&str> = Vec::new();
        for group in &referenced {
            let doc = group_document_iri(group);
            if !documents.contains(&doc) && documents.len() < MAX_GROUP_DOCUMENTS {
                documents.push(doc);
            }
        }

        let mut members: Vec<String> = Vec::new();
        for doc in documents {
            if !self.is_local(doc) {
                continue;
            }
            let Some(triples) = self.read_and_parse_acl(doc).await? else {
                continue;
            };
            for group in &referenced {
                if group_document_iri(group) == doc
                    && !members.contains(group)
                    && group_has_member(&triples, group, web_id)
                {
                    members.push(group.clone());
                }
            }
        }
        Ok(members)
    }

    /// Whether an IRI names a resource inside THIS server's storage — the only thing the [`Store`]
    /// can read. Group resolution uses it to leave a remote group document unresolved instead of
    /// fetching it, so an authorization decision never triggers outbound HTTP.
    fn is_local(&self, iri: &str) -> bool {
        let root = format!("{}/", self.base_url.trim_end_matches('/'));
        iri.starts_with(&root)
    }

    /// Resolve the EFFECTIVE ACL governing `resource` ONCE — the expensive part (the child→root walk,
    /// the per-`.acl` `store.read`, and the `oxttl`/`oxjsonld` parse). Returns the parsed triples + the
    /// base resource the rules match against + the [`AclScope`] (`AccessTo` for the resource's own ACL,
    /// `Default` for an inherited ancestor ACL), or [`ResolvedAcl::none`] when no ACL governs it
    /// anywhere (fail-closed — no grants for ANY requester).
    ///
    /// The returned [`ResolvedAcl`] is then evaluated PER requester via [`ResolvedAcl::modes_for`]
    /// (pure rule-matching over the already-parsed triples — no further I/O), so resolving once and
    /// evaluating both the authenticated requester AND the public requester costs ONE walk/read/parse,
    /// not two. This is the substance of Optimization #2 for the authenticated read path.
    async fn resolve_effective_acl(&self, resource: &str) -> Result<ResolvedAcl, ServerError> {
        // 1. The resource's OWN ACL (accessTo scope).
        if let Some(triples) = self.read_acl(&self.acl_for(resource)).await? {
            return Ok(ResolvedAcl::found(
                triples,
                resource.to_string(),
                AclScope::AccessTo,
            ));
        }

        // 2. Walk ancestors child→root; the first one with an ACL governs via `acl:default`.
        for ancestor in self.ancestors_nearest_first(resource) {
            if let Some(triples) = self.read_acl(&self.acl_for(&ancestor)).await? {
                return Ok(ResolvedAcl::found(triples, ancestor, AclScope::Default));
            }
        }

        // 3. No ACL anywhere → no grants (fail-closed) for any requester.
        Ok(ResolvedAcl::none())
    }

    /// Read and parse an ACL resource through the [`Store`] into triples. `Ok(None)` if the ACL does
    /// NOT exist (the common case). Any other store error propagates (a transient failure must not be
    /// silently treated as "no ACL" → fail-open). A malformed ACL body yields an empty triple set via
    /// the parser error being mapped to "no usable rules" — but here we propagate a parse error as a
    /// storage error is avoided: an unparseable ACL is treated as PRESENT-but-granting-nothing
    /// (fail-closed), NOT as absent (which would wrongly inherit the parent's grants).
    ///
    /// ## ETag-keyed parsed-ACL cache (read-path optimisation #3)
    /// When an [`AclCache`] is attached, this:
    ///  1. cheaply probes the ACL's CURRENT etag via [`Store::meta`] (an index lookup — NO blob
    ///     byte-fetch, NO parse). An ABSENT ACL (`None`) is the `Ok(None)` "no own ACL, keep walking"
    ///     case (unchanged), and the cache holds nothing for it (it can never fabricate a removed ACL);
    ///  2. on a cache HIT for `(acl, etag)` returns the cached parse — the byte-fetch + `oxttl` parse
    ///     are SKIPPED (the win);
    ///  3. on a MISS reads the bytes + parses, then REFRESHES the entry under the etag of the bytes it
    ///     ACTUALLY read (so the cached parse always corresponds to the cached etag — no TOCTOU stale).
    ///
    /// SECURITY: the cache only avoids the re-PARSE of an UNCHANGED ACL — the etag-equality gate
    /// guarantees a cached parse is reused ONLY when the bytes are unchanged, so a rotated/removed ACL
    /// can never be served stale and the resulting triples (hence the decision + `WAC-Allow`) are
    /// byte-identical to the cold path. When NO cache is attached, the original single `store.read` +
    /// parse path runs (no extra `meta` round-trip) — byte-identical to the pre-cache code AND to the
    /// `=0` disabled-cache configuration.
    async fn read_acl(&self, acl: &str) -> Result<Option<Vec<oxrdf::Triple>>, ServerError> {
        // No cache attached: the original path — ONE `store.read` (get_meta + blob.get) + parse. No
        // extra `meta` probe, so cost is identical to the pre-cache code.
        let Some(cache) = self.acl_cache else {
            return self.read_and_parse_acl(acl).await;
        };

        // Cache attached: probe the ACL's current etag CHEAPLY (index get_meta — no bytes, no parse).
        let meta = match self.store.meta(acl).await? {
            Some(m) => m,
            // Absent ACL → `Ok(None)` (the common "no own ACL here, keep walking" case). The cache
            // cannot resurrect a removed ACL: there is no `get` for an absent IRI.
            None => return Ok(None),
        };
        let now = Self::now_secs();
        // HIT on `(acl, current-etag)`: reuse the cached parse — skip the byte-fetch + `oxttl` parse.
        if let Some(triples) = cache.get(acl, &meta.etag, now) {
            return Ok(Some(triples));
        }
        // MISS (no entry / rotated etag / TTL-stale): read the bytes + parse, then refresh the cache
        // under the etag of the BYTES ACTUALLY READ (the authoritative etag for this parse). A
        // concurrent rotation between the `meta` probe and this `read` just means we parse + cache the
        // newer bytes — never a stale parse.
        let resource = match self.store.read(acl).await {
            Ok(r) => r,
            // The ACL vanished between the `meta` probe and the read (a concurrent DELETE) → treat as
            // absent: `Ok(None)`, exactly as a cold walk that found it gone would.
            Err(ServerError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let triples = Self::parse_acl_body(&resource, acl);
        cache.insert(acl, &resource.meta.etag, triples.clone(), now);
        Ok(Some(triples))
    }

    /// The uncached read+parse of an RDF control document: ONE `store.read` (get_meta + blob.get) +
    /// the `oxttl` parse — the exact pre-cache path, used when no cache is attached, and also the
    /// read for an `acl:agentGroup` group document (which wants exactly this contract: absent ⇒
    /// `Ok(None)`, malformed ⇒ empty triples, any other store error propagates; the parsed-ACL cache
    /// is deliberately not involved, so a group read can never evict or serve an ACL entry).
    async fn read_and_parse_acl(
        &self,
        acl: &str,
    ) -> Result<Option<Vec<oxrdf::Triple>>, ServerError> {
        let resource = match self.store.read(acl).await {
            Ok(r) => r,
            Err(ServerError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(Self::parse_acl_body(&resource, acl)))
    }

    /// Parse an ACL resource's bytes into triples, mapping a PARSE error to an EMPTY triple set
    /// (PRESENT-but-granting-nothing, fail-closed) — NOT to absent. A broken own-ACL must DENY, never
    /// fall through to a parent's `acl:default`. The single home for the ACL parse + its fail-closed
    /// error mapping, shared by the cached and uncached paths so they are byte-identical.
    fn parse_acl_body(resource: &crate::store::Resource, acl: &str) -> Vec<oxrdf::Triple> {
        let format = classify(Some(&resource.meta.content_type)).unwrap_or(RdfFormat::Turtle);
        // A PRESENT but malformed ACL grants nothing (fail-closed) — it is NOT absent. A parse error
        // maps to an EMPTY triple set (NOT propagated, NOT treated as absent): the caller returns it as
        // `Some(Vec::new())`, which stops the inheritance walk so a broken own-ACL DENIES rather than
        // falling through to a parent's `acl:default`.
        parse_to_triples(format, &resource.body, acl).unwrap_or_default()
    }

    /// Current epoch seconds for the ACL-cache freshness gate (the validation TTL). A clock error
    /// (pre-1970, impossible in practice) yields 0 — the cache then treats every entry as old (a miss),
    /// which is the SAFE direction (re-read + re-parse, never a stale hit).
    fn now_secs() -> i64 {
        crate::clock::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// The protected resource an `.acl` target governs: for an `.acl` IRI, strip the trailing `.acl`
    /// (Control of the governed resource gates its ACL); otherwise the target itself.
    fn protected_resource(&self, target: &str) -> String {
        if is_acl_resource(target) {
            target[..target.len() - ACL_SUFFIX.len()].to_string()
        } else {
            target.to_string()
        }
    }

    /// The ACL identifier for a resource: `<document>.acl` and `<container>/.acl`. For a container
    /// `https://pod/c/` the ACL is `https://pod/c/.acl`.
    fn acl_for(&self, resource: &str) -> String {
        format!("{resource}{ACL_SUFFIX}")
    }

    /// The ancestor containers of `resource`, NEAREST first, up to and including the storage root.
    /// For a document `https://pod/a/b/doc`: `[https://pod/a/b/, https://pod/a/, https://pod/]`. For a
    /// container `https://pod/a/b/`: `[https://pod/a/, https://pod/]` (its own ACL is checked
    /// separately, so its PARENT is the first ancestor). The root has no ancestors.
    fn ancestors_nearest_first(&self, resource: &str) -> Vec<String> {
        let root = format!("{}/", self.base_url.trim_end_matches('/'));
        let mut ancestors: Vec<String> = Vec::new();
        if resource == root {
            return ancestors;
        }
        // The immediate parent of `resource`. For a container, drop its own trailing slash first.
        let mut current = resource.to_string();
        if current.ends_with('/') {
            current.pop();
        }
        while current.len() > root.len() {
            let Some(slash) = current.rfind('/') else {
                break;
            };
            let parent = current[..=slash].to_string();
            ancestors.push(parent.clone());
            // Step to the parent without its trailing slash for the next iteration.
            current = parent[..parent.len() - 1].to_string();
        }
        // Ensure the root is included (the loop stops once `current` reaches the root length).
        if ancestors.last().map(String::as_str) != Some(root.as_str()) {
            ancestors.push(root);
        }
        ancestors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
    use axum::body::Bytes;

    const BASE: &str = "https://pod.example";
    const ALICE: &str = "https://pod.example/alice/profile/card#me";
    const BOB: &str = "https://pod.example/bob/profile/card#me";

    type TestStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

    fn store() -> TestStore {
        CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
    }

    async fn put_acl(store: &TestStore, acl_iri: &str, body: &str) {
        store
            .write(acl_iri, Bytes::from(body.to_string()), "text/turtle")
            .await
            .expect("write acl");
    }

    fn owner_default_acl(target: &str, owner: &str) -> String {
        format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#owner> a acl:Authorization;
                     acl:agent <{owner}>;
                     acl:accessTo <{target}>;
                     acl:default <{target}>;
                     acl:mode acl:Read, acl:Write, acl:Control."#
        )
    }

    // --- own-vs-inherited resolution ----------------------------------------------------------

    #[tokio::test]
    async fn own_acl_wins_over_inherited() {
        let s = store();
        let container = "https://pod.example/alice/";
        let resource = "https://pod.example/alice/data";
        // The container grants Bob default Read (inheritable).
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#d> a acl:Authorization; acl:agent <{BOB}>; acl:default <{container}>; acl:mode acl:Read."#
            ),
        )
        .await;
        // The resource has its OWN acl granting Bob nothing (only Alice).
        put_acl(
            &s,
            "https://pod.example/alice/data.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Bob is DENIED on the resource (own acl wins; the inherited default does NOT apply).
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        // Alice IS allowed by the own acl.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn inherits_default_from_nearest_ancestor() {
        let s = store();
        let resource = "https://pod.example/alice/test/data";
        // The pod root grants Alice default control; /alice/test/ has NO own acl.
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Alice inherits read/write/control via the pod-root default.
        let d = wac
            .authorize(resource, AccessMode::Write, Some(ALICE), None)
            .await
            .unwrap();
        assert!(matches!(d, Decision::Allow(_)));
        // Bob inherits nothing → 403.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }

    #[tokio::test]
    async fn nearest_acl_fully_overrides_more_distant() {
        let s = store();
        let resource = "https://pod.example/alice/test/data";
        // Root grants Bob default read.
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#d> a acl:Authorization; acl:agent <{BOB}>; acl:default <https://pod.example/alice/>; acl:mode acl:Read."#
            ),
        )
        .await;
        // The nearer container /alice/test/ has its OWN acl granting only Alice (default). This fully
        // overrides root — Bob gets nothing.
        put_acl(
            &s,
            "https://pod.example/alice/test/.acl",
            &owner_default_acl("https://pod.example/alice/test/", ALICE),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    // --- public vs authenticated vs specific-agent + 401-vs-403 -------------------------------

    #[tokio::test]
    async fn public_read_allows_anonymous() {
        let s = store();
        let resource = "https://pod.example/alice/test/pub";
        put_acl(
            &s,
            "https://pod.example/alice/test/pub.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn anonymous_denied_is_401_authenticated_denied_is_403() {
        let s = store();
        let resource = "https://pod.example/alice/test/secret";
        // Only Alice may read.
        put_acl(
            &s,
            "https://pod.example/alice/test/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Anonymous → 401 (Unauthenticated).
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        // Bob (authenticated, not granted) → 403 (Forbidden).
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }

    // --- Control governs .acl -----------------------------------------------------------------

    #[tokio::test]
    async fn control_governs_reading_the_acl_document() {
        let s = store();
        let resource = "https://pod.example/alice/test/data";
        let acl = "https://pod.example/alice/test/data.acl";
        // Bob has Read but NOT Control on the resource; Alice has Control.
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read.
                <#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Reading the .acl requires Control: Bob (Read only) is FORBIDDEN; Alice (Control) is ALLOWED.
        assert_eq!(
            wac.authorize(acl, AccessMode::Control, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        assert!(matches!(
            wac.authorize(acl, AccessMode::Control, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn write_holder_is_denied_control() {
        // The foundation of the container-DELETE rule (DELETE of a container needs Control, not mere
        // Write): a requester granted Read+Write but NOT Control must be DENIED a Control decision —
        // Control is never implied by Write.
        let s = store();
        let resource = "https://pod.example/alice/test/c/";
        put_acl(
            &s,
            "https://pod.example/alice/test/c/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write.
                <#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Bob has Write but not Control → a Control decision is FORBIDDEN.
        assert_eq!(
            wac.authorize(resource, AccessMode::Control, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        // Bob's Write decision is still allowed (Write does not imply, but is granted).
        assert!(matches!(
            wac.authorize(resource, AccessMode::Write, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // Alice (Control) is allowed Control.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Control, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    // --- fail-closed on missing / malformed ----------------------------------------------------

    #[tokio::test]
    async fn no_acl_anywhere_denies_fail_closed() {
        let s = store();
        let resource = "https://pod.example/alice/test/data";
        let wac = WacAuthorizer::new(&s, BASE);
        // No ACL exists at all → denied. Anonymous → 401, authenticated → 403.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }

    #[tokio::test]
    async fn malformed_own_acl_denies_does_not_inherit() {
        let s = store();
        let resource = "https://pod.example/alice/test/data";
        // The pod root would grant Alice control by inheritance...
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        // ...but the resource has a MALFORMED own acl. A present-but-broken own acl must DENY, NOT fall
        // through to the parent's default (fail-closed).
        put_acl(
            &s,
            "https://pod.example/alice/test/data.acl",
            "this is not valid turtle @@@ <<< broken",
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }

    // --- the no-bypass test -------------------------------------------------------------------

    #[tokio::test]
    async fn no_bypass_wrong_webid_or_anonymous_cannot_read_or_write() {
        let s = store();
        let resource = "https://pod.example/alice/test/private";
        // Only Alice may read+write.
        put_acl(
            &s,
            "https://pod.example/alice/test/private.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Wrong WebID (Bob) cannot read or write.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        assert_eq!(
            wac.authorize(resource, AccessMode::Write, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        // Anonymous cannot read or write.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        assert_eq!(
            wac.authorize(resource, AccessMode::Write, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        // A near-miss WebID (same prefix, different agent) is NOT Alice — no string-prefix bypass.
        let near = "https://pod.example/alice/profile/card#evil";
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(near), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }

    // --- WAC-Allow accuracy --------------------------------------------------------------------

    #[tokio::test]
    async fn wac_allow_reflects_owner_full_and_public_subset() {
        let s = store();
        let resource = "https://pod.example/alice/test/doc";
        // Alice (owner) full control; public read only — the public-access-direct shape.
        put_acl(
            &s,
            "https://pod.example/alice/test/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        // Alice's WAC-Allow: user = read/write/control, public = read.
        let perms = wac
            .effective_permissions(resource, Some(ALICE), None, None)
            .await
            .unwrap();
        assert_eq!(
            perms.user,
            [AccessMode::Read, AccessMode::Write, AccessMode::Control]
                .into_iter()
                .collect()
        );
        assert_eq!(perms.public, [AccessMode::Read].into_iter().collect());

        // An anonymous reader's WAC-Allow: user == public == read.
        let pub_perms = wac
            .effective_permissions(resource, None, None, None)
            .await
            .unwrap();
        assert_eq!(pub_perms.user, [AccessMode::Read].into_iter().collect());
        assert_eq!(pub_perms.public, [AccessMode::Read].into_iter().collect());
    }

    #[tokio::test]
    async fn wac_allow_user_only_no_public() {
        let s = store();
        let container = "https://pod.example/alice/test/c/";
        let resource = "https://pod.example/alice/test/c/doc";
        // Bob granted inheritable read via the container default; no public access.
        put_acl(
            &s,
            "https://pod.example/alice/test/c/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#d> a acl:Authorization; acl:agent <{BOB}>; acl:default <{container}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        let perms = wac
            .effective_permissions(resource, Some(BOB), None, None)
            .await
            .unwrap();
        assert_eq!(perms.user, [AccessMode::Read].into_iter().collect());
        assert!(
            perms.public.is_empty(),
            "public must be empty: {:?}",
            perms.public
        );
    }

    #[tokio::test]
    async fn wac_allow_public_reflects_origin_scoped_grant_for_authenticated_request() {
        // Finding 3: WAC-Allow `public=` for an AUTHENTICATED request must carry the CURRENT request's
        // Origin when resolving the public set, so an ORIGIN-SCOPED public grant
        // (`acl:agentClass foaf:Agent` + `acl:origin <o>`) is reported when the request Origin matches
        // — and omitted when it does not / when no Origin is sent (fail-closed). Resolving the public
        // set with `Requester::anonymous()` (origin None) would always omit it (the under-report bug).
        const APP: &str = "https://app.example";
        const OTHER: &str = "https://evil.example";
        let s = store();
        let resource = "https://pod.example/alice/test/scoped";
        // Alice (owner) full control; the PUBLIC gets Read but ONLY from https://app.example.
        put_acl(
            &s,
            "https://pod.example/alice/test/scoped.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <{APP}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);

        // Authenticated (Alice) request FROM the trusted origin: public= must report the origin-scoped
        // public Read.
        let matched = wac
            .effective_permissions(resource, Some(ALICE), Some(APP), None)
            .await
            .unwrap();
        assert_eq!(
            matched.public,
            [AccessMode::Read].into_iter().collect(),
            "an origin-scoped public grant must be reported in public= when the request Origin matches"
        );
        // Alice herself still has her full set regardless of origin (her grant has no acl:origin).
        assert_eq!(
            matched.user,
            [AccessMode::Read, AccessMode::Write, AccessMode::Control]
                .into_iter()
                .collect()
        );

        // Authenticated request from a DIFFERENT origin: the origin-scoped public grant must be OMITTED.
        let other_origin = wac
            .effective_permissions(resource, Some(ALICE), Some(OTHER), None)
            .await
            .unwrap();
        assert!(
            other_origin.public.is_empty(),
            "an origin-scoped public grant must be omitted from public= for a non-matching Origin: {:?}",
            other_origin.public
        );

        // Authenticated request with NO Origin: an origin-restricted public rule never matches
        // (fail-closed) ⇒ public= empty.
        let no_origin = wac
            .effective_permissions(resource, Some(ALICE), None, None)
            .await
            .unwrap();
        assert!(
            no_origin.public.is_empty(),
            "an origin-scoped public grant must be omitted from public= when no Origin is sent: {:?}",
            no_origin.public
        );
    }

    // --- ancestor walk shape -------------------------------------------------------------------

    #[test]
    fn ancestors_for_a_document() {
        let s = store();
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.ancestors_nearest_first("https://pod.example/a/b/doc"),
            vec![
                "https://pod.example/a/b/".to_string(),
                "https://pod.example/a/".to_string(),
                "https://pod.example/".to_string(),
            ]
        );
    }

    #[test]
    fn ancestors_for_a_container_start_at_parent() {
        let s = store();
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.ancestors_nearest_first("https://pod.example/a/b/"),
            vec![
                "https://pod.example/a/".to_string(),
                "https://pod.example/".to_string(),
            ]
        );
    }

    #[test]
    fn root_has_no_ancestors() {
        let s = store();
        let wac = WacAuthorizer::new(&s, BASE);
        assert!(wac
            .ancestors_nearest_first("https://pod.example/")
            .is_empty());
    }

    #[test]
    fn protected_resource_strips_dot_acl() {
        let s = store();
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.protected_resource("https://pod.example/a/b.acl"),
            "https://pod.example/a/b"
        );
        assert_eq!(
            wac.protected_resource("https://pod.example/a/.acl"),
            "https://pod.example/a/"
        );
        assert_eq!(
            wac.protected_resource("https://pod.example/a/b"),
            "https://pod.example/a/b"
        );
    }

    // --- Opt #2: authorize_read is byte-equivalent to authorize + effective_permissions -----------

    /// Cross-check `authorize_read` against the OLD two-call path (`authorize` then
    /// `effective_permissions(Some(granted))`) for the SAME (target, required, web_id, origin): the
    /// access decision AND the resulting `EffectivePermissions` must be IDENTICAL. This is the
    /// security-critical invariant of the single-pass refactor — the gate and the `WAC-Allow`
    /// advertisement do not change.
    async fn assert_read_matches_old_path(
        wac: &WacAuthorizer<'_, TestStore>,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) {
        // OLD path: authorize → (on Allow) effective_permissions reusing the granted user set.
        let old_decision = wac
            .authorize(target, required, web_id, origin)
            .await
            .unwrap();
        let old = match old_decision {
            Decision::Allow(granted) => {
                let perms = wac
                    .effective_permissions(target, web_id, origin, Some(granted))
                    .await
                    .unwrap();
                ReadDecision::Allow(perms)
            }
            Decision::Unauthenticated => ReadDecision::Unauthenticated,
            Decision::Forbidden => ReadDecision::Forbidden,
        };
        // NEW single-pass path.
        let new = wac
            .authorize_read(target, required, web_id, origin)
            .await
            .unwrap();
        assert_eq!(
            new, old,
            "authorize_read must match the old authorize+effective_permissions path \
             for target={target} web_id={web_id:?} origin={origin:?}"
        );
    }

    #[tokio::test]
    async fn authorize_read_equivalence_across_cases() {
        const APP: &str = "https://app.example";
        const OTHER: &str = "https://evil.example";
        let s = store();
        // Owner full control; public Read; AND an origin-scoped public Append from APP only.
        let resource = "https://pod.example/alice/test/doc";
        put_acl(
            &s,
            "https://pod.example/alice/test/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{resource}>; acl:mode acl:Read.
                <#s> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <{APP}>; acl:accessTo <{resource}>; acl:mode acl:Append."#
            ),
        )
        .await;
        // A fully-private sibling (only Alice; no public) for the 401/403 paths.
        let secret = "https://pod.example/alice/test/secret";
        put_acl(
            &s,
            "https://pod.example/alice/test/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{secret}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        // A resource with NO ACL anywhere (fail-closed).
        let orphan = "https://pod.example/bob/orphan";

        let wac = WacAuthorizer::new(&s, BASE);
        // Public-read resource: anonymous allow (public==user); authed allow with public resolved
        // separately; authed allow from a matching/ non-matching/ no origin (origin-scoped public).
        assert_read_matches_old_path(&wac, resource, AccessMode::Read, None, None).await;
        assert_read_matches_old_path(&wac, resource, AccessMode::Read, Some(ALICE), None).await;
        assert_read_matches_old_path(&wac, resource, AccessMode::Read, Some(ALICE), Some(APP))
            .await;
        assert_read_matches_old_path(&wac, resource, AccessMode::Read, Some(ALICE), Some(OTHER))
            .await;
        assert_read_matches_old_path(&wac, resource, AccessMode::Read, Some(BOB), Some(APP)).await;
        // Private secret: anonymous → Unauthenticated; wrong authed agent → Forbidden.
        assert_read_matches_old_path(&wac, secret, AccessMode::Read, None, None).await;
        assert_read_matches_old_path(&wac, secret, AccessMode::Read, Some(BOB), None).await;
        // No-ACL orphan: fail-closed (anon 401, authed 403).
        assert_read_matches_old_path(&wac, orphan, AccessMode::Read, None, None).await;
        assert_read_matches_old_path(&wac, orphan, AccessMode::Read, Some(BOB), None).await;
        // Reading the `.acl` itself requires Control (the read path passes Control for an `.acl`):
        // Alice (Control) allowed, Bob (none) forbidden, anon unauthenticated — all must match.
        let acl = "https://pod.example/alice/test/doc.acl";
        assert_read_matches_old_path(&wac, acl, AccessMode::Control, Some(ALICE), None).await;
        assert_read_matches_old_path(&wac, acl, AccessMode::Control, Some(BOB), None).await;
        assert_read_matches_old_path(&wac, acl, AccessMode::Control, None, None).await;
    }

    // --- acl:agentGroup: membership resolved through the Store -------------------------------

    const GROUP_DOC: &str = "https://pod.example/groups/team";
    const TEAM: &str = "https://pod.example/groups/team#g";
    const GROUP_RES: &str = "https://pod.example/alice/team/data";
    const GROUP_ACL: &str = "https://pod.example/alice/team/data.acl";

    /// An `.acl` granting Read on [`GROUP_RES`] to the members of `group` (and to nobody else).
    fn group_grant_acl(group: &str) -> String {
        format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#g> a acl:Authorization;
                 acl:agentGroup <{group}>;
                 acl:accessTo <{GROUP_RES}>;
                 acl:mode acl:Read."#
        )
    }

    /// A store whose `.acl` grants Read to [`TEAM`]; the group document is written by the caller so
    /// each test controls whether — and how — membership resolves.
    async fn group_store() -> TestStore {
        let s = store();
        put_acl(&s, GROUP_ACL, &group_grant_acl(TEAM)).await;
        s
    }

    async fn put_group_doc(s: &TestStore, iri: &str, body: &str) {
        s.write(iri, Bytes::from(body.to_string()), "text/turtle")
            .await
            .expect("write group document");
    }

    /// The headline guard: a WebID named ONLY through `acl:agentGroup` is granted once the group
    /// document — read through the `Store` — lists it, and a non-member still is not.
    #[tokio::test]
    async fn agent_group_grants_a_member_named_in_the_group_document() {
        let s = group_store().await;
        put_group_doc(
            &s,
            GROUP_DOC,
            &format!(
                r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                <{TEAM}> vcard:hasMember <{BOB}>."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);

        assert!(
            matches!(
                wac.authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                    .await
                    .unwrap(),
                Decision::Allow(_)
            ),
            "Bob is a vcard:hasMember of the granted group, so the agentGroup rule must grant him"
        );
        // A WebID the group document does not list gets nothing.
        assert_eq!(
            wac.authorize(GROUP_RES, AccessMode::Read, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        // The rule grants Read only — Write is still denied to the member.
        assert_eq!(
            wac.authorize(GROUP_RES, AccessMode::Write, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        // And an anonymous requester can never be a member.
        assert_eq!(
            wac.authorize(GROUP_RES, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
    }

    /// Every way the group document can fail to vouch for the requester denies (fail-closed): it is
    /// ABSENT, it is MALFORMED, or it names the member under a DIFFERENT group in the same document.
    #[tokio::test]
    async fn agent_group_unresolvable_document_fails_closed() {
        // 1. No group document at all.
        let s = group_store().await;
        let wac = WacAuthorizer::new(&s, BASE);
        assert_eq!(
            wac.authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden,
            "a missing group document cannot vouch for anyone"
        );

        // 2. A malformed group document parses to an empty triple set — present, granting nothing.
        let s = group_store().await;
        put_group_doc(&s, GROUP_DOC, "this is not turtle <<<").await;
        assert_eq!(
            WacAuthorizer::new(&s, BASE)
                .authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden,
            "a malformed group document must not confer membership"
        );

        // 3. Bob is a member of a DIFFERENT group that happens to share the document — matching is on
        //    the full group IRI, so the fragment must keep the memberships apart.
        let s = group_store().await;
        put_group_doc(
            &s,
            GROUP_DOC,
            &format!(
                r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                <{GROUP_DOC}#other> vcard:hasMember <{BOB}>."#
            ),
        )
        .await;
        assert_eq!(
            WacAuthorizer::new(&s, BASE)
                .authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden,
            "membership of a sibling group in the same document must not grant"
        );
    }

    /// A group document on ANOTHER origin is left unresolved rather than fetched: authorization reads
    /// only through the `Store`, so it performs no request-driven outbound HTTP and a remote group
    /// grants nothing. The identically-named LOCAL group does grant — proving the denial is the
    /// remoteness check, not a broken group path.
    #[tokio::test]
    async fn agent_group_remote_document_is_not_resolved_fail_closed() {
        const REMOTE_TEAM: &str = "https://other.example/groups/team#g";
        let s = store();
        put_acl(&s, GROUP_ACL, &group_grant_acl(REMOTE_TEAM)).await;
        // Even if a document at that IRI somehow existed in this store, it is outside the server's
        // base URL, so group resolution must skip it.
        put_group_doc(
            &s,
            "https://other.example/groups/team",
            &format!(
                r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                <{REMOTE_TEAM}> vcard:hasMember <{BOB}>."#
            ),
        )
        .await;
        assert_eq!(
            WacAuthorizer::new(&s, BASE)
                .authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Forbidden,
            "a group document outside this server's storage must be left unresolved"
        );

        // Control: the same shape with a LOCAL group IRI does grant.
        let s = group_store().await;
        put_group_doc(
            &s,
            GROUP_DOC,
            &format!(
                r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                <{TEAM}> vcard:hasMember <{BOB}>."#
            ),
        )
        .await;
        assert!(matches!(
            WacAuthorizer::new(&s, BASE)
                .authorize(GROUP_RES, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    /// A group grant belongs to the `user` audience ONLY: the public has no WebID, so it can never be
    /// a `vcard:hasMember`, and `WAC-Allow`'s `public=` must not advertise a group-derived mode.
    #[tokio::test]
    async fn agent_group_grant_never_reaches_the_public_audience() {
        let s = group_store().await;
        put_group_doc(
            &s,
            GROUP_DOC,
            &format!(
                r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                <{TEAM}> vcard:hasMember <{BOB}>."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        let ReadDecision::Allow(perms) = wac
            .authorize_read(GROUP_RES, AccessMode::Read, Some(BOB), None)
            .await
            .unwrap()
        else {
            panic!("the member must be allowed to read");
        };
        assert!(perms.user.contains(&AccessMode::Read));
        assert!(
            perms.public.is_empty(),
            "a group grant must never be advertised as public"
        );
    }

    /// Group resolution must not diverge between the sequential walk and the read-plan path — the
    /// same equivalence the rest of the WAC matrix asserts, over the member / non-member / no-document
    /// cases and both read paths.
    #[tokio::test]
    async fn agent_group_planned_paths_match_the_sequential_walk() {
        for with_doc in [false, true] {
            let s = group_store().await;
            if with_doc {
                put_group_doc(
                    &s,
                    GROUP_DOC,
                    &format!(
                        r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
                        <{TEAM}> vcard:hasMember <{BOB}>."#
                    ),
                )
                .await;
            }
            let wac = WacAuthorizer::new(&s, BASE);
            for who in [Some(BOB), Some(ALICE), None] {
                assert_planned_matches_sequential(&wac, &s, GROUP_RES, AccessMode::Read, who, None)
                    .await;
                assert_planned_authorize_matches_sequential(
                    &wac,
                    &s,
                    GROUP_RES,
                    AccessMode::Read,
                    who,
                    None,
                )
                .await;
            }
        }
    }

    // --- Opt #3: the ETag-keyed parsed-ACL cache is decision-equivalent to the cold resolve ---------

    use crate::acl_cache::AclCache;

    /// A cached `authorize` must return the IDENTICAL [`Decision`] a NON-cached `authorize` does — on
    /// the COLD pass (cache miss → populates) AND on the WARM pass (cache hit → reuses the parse). This
    /// is the security-critical invariant: the cache only avoids the re-parse; it never changes the
    /// decision. Asserts the warm pass equals the cold/uncached decision for the SAME inputs.
    async fn assert_cached_authorize_matches_uncached(
        store: &TestStore,
        cache: &AclCache,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) {
        let uncached = WacAuthorizer::new(store, BASE)
            .authorize(target, required, web_id, origin)
            .await
            .unwrap();
        let cached = WacAuthorizer::with_cache(store, BASE, cache);
        // COLD pass (cache miss → populate) must match the uncached decision.
        let cold = cached
            .authorize(target, required, web_id, origin)
            .await
            .unwrap();
        assert_eq!(
            cold, uncached,
            "cold cached authorize must equal uncached for target={target} web_id={web_id:?} origin={origin:?}"
        );
        // WARM pass (cache hit → reuse the parse) must ALSO match — a hit cannot change the decision.
        let warm = cached
            .authorize(target, required, web_id, origin)
            .await
            .unwrap();
        assert_eq!(
            warm, uncached,
            "warm (cache-hit) authorize must equal uncached for target={target} web_id={web_id:?} origin={origin:?}"
        );
    }

    /// Same equivalence for `authorize_read` (the GET/HEAD WAC-Allow path): the cached COLD + WARM
    /// [`ReadDecision`] (incl. the full `EffectivePermissions` on Allow) must equal the uncached one.
    async fn assert_cached_read_matches_uncached(
        store: &TestStore,
        cache: &AclCache,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) {
        let uncached = WacAuthorizer::new(store, BASE)
            .authorize_read(target, required, web_id, origin)
            .await
            .unwrap();
        let cached = WacAuthorizer::with_cache(store, BASE, cache);
        let cold = cached
            .authorize_read(target, required, web_id, origin)
            .await
            .unwrap();
        assert_eq!(
            cold, uncached,
            "cold cached authorize_read must equal uncached for {target}"
        );
        let warm = cached
            .authorize_read(target, required, web_id, origin)
            .await
            .unwrap();
        assert_eq!(warm, uncached, "warm cached authorize_read must equal uncached (hit cannot change WAC-Allow) for {target}");
    }

    /// The cache is decision-equivalent across EVERY ACL shape — public-read / private / no-ACL /
    /// `.acl`-Control / origin match/non-match/absent / broken-ACL-fail-closed / inherited-default —
    /// on BOTH a cold (populating) and a warm (hit) pass. Proves a hit returns the identical decision +
    /// WAC-Allow as a cold resolve.
    #[tokio::test]
    async fn cached_resolve_is_decision_equivalent_across_shapes() {
        const APP: &str = "https://app.example";
        const OTHER: &str = "https://evil.example";
        let s = store();
        // public-read + owner-control + an origin-scoped public Append.
        let public_doc = "https://pod.example/alice/test/doc";
        put_acl(
            &s,
            "https://pod.example/alice/test/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{public_doc}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{public_doc}>; acl:mode acl:Read.
                <#s> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <{APP}>; acl:accessTo <{public_doc}>; acl:mode acl:Append."#
            ),
        )
        .await;
        // private (only Alice).
        let secret = "https://pod.example/alice/test/secret";
        put_acl(
            &s,
            "https://pod.example/alice/test/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{secret}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        // inherited-default: /alice/.acl grants Alice control; /alice/inh/data has no own ACL.
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let inherited = "https://pod.example/alice/inh/data";
        // broken own-ACL (fail-closed): present-but-malformed.
        let broken = "https://pod.example/alice/broken";
        put_acl(
            &s,
            "https://pod.example/alice/broken.acl",
            "@@@ not valid turtle <<< broken",
        )
        .await;
        // no-ACL orphan anywhere.
        let orphan = "https://pod.example/zzz/orphan";

        let cache = AclCache::new(64);
        // Run each (target, mode, web_id, origin) tuple through BOTH authorize + authorize_read,
        // cold-then-warm, and assert decision-equivalence with the uncached resolve.
        let cases: &[(&str, AccessMode, Option<&str>, Option<&str>)] = &[
            (public_doc, AccessMode::Read, None, None),
            (public_doc, AccessMode::Read, Some(ALICE), None),
            (public_doc, AccessMode::Read, Some(ALICE), Some(APP)),
            (public_doc, AccessMode::Read, Some(ALICE), Some(OTHER)),
            (public_doc, AccessMode::Read, Some(BOB), Some(APP)),
            (secret, AccessMode::Read, None, None),
            (secret, AccessMode::Read, Some(BOB), None),
            (secret, AccessMode::Read, Some(ALICE), None),
            (inherited, AccessMode::Write, Some(ALICE), None),
            (inherited, AccessMode::Read, Some(BOB), None),
            (broken, AccessMode::Read, Some(ALICE), None),
            (broken, AccessMode::Read, None, None),
            (orphan, AccessMode::Read, None, None),
            (orphan, AccessMode::Read, Some(BOB), None),
            // The `.acl` document itself (Control-gated).
            (
                "https://pod.example/alice/test/doc.acl",
                AccessMode::Control,
                Some(ALICE),
                None,
            ),
            (
                "https://pod.example/alice/test/doc.acl",
                AccessMode::Control,
                Some(BOB),
                None,
            ),
            (
                "https://pod.example/alice/test/doc.acl",
                AccessMode::Control,
                None,
                None,
            ),
        ];
        for (target, mode, web_id, origin) in cases {
            assert_cached_authorize_matches_uncached(&s, &cache, target, *mode, *web_id, *origin)
                .await;
            assert_cached_read_matches_uncached(&s, &cache, target, *mode, *web_id, *origin).await;
        }
    }

    /// A WRITE to the ACL that CHANGES its rules must be seen by the NEXT cached read — no stale grant.
    /// Two mechanisms guarantee this: (1) the rewritten ACL has DIFFERENT bytes ⇒ a DIFFERENT etag ⇒
    /// the `(acl, etag)` gate misses and re-parses; (2) the handler also explicitly invalidates on an
    /// `.acl` write. This test exercises (1) directly at the resolver: it populates the cache with a
    /// permissive ACL, then rewrites the SAME `.acl` to a restrictive one and asserts the cached
    /// resolve now DENIES (the new rules), proving the cache cannot serve a stale ALLOW after a change.
    #[tokio::test]
    async fn acl_write_is_seen_by_next_cached_read_no_stale_grant() {
        let s = store();
        let resource = "https://pod.example/alice/rot/data";
        let acl = "https://pod.example/alice/rot/data.acl";
        // Initially: BOB may read.
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#b> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let cache = AclCache::new(64);
        let wac = WacAuthorizer::with_cache(&s, BASE, &cache);
        // Populate the cache: Bob is allowed (cold), and a second read confirms the warm hit allows.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert!(
            matches!(
                wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                    .await
                    .unwrap(),
                Decision::Allow(_)
            ),
            "second read must still allow (this is the cache hit being populated/served)"
        );
        // ROTATE the ACL: now ONLY Alice may read — Bob is removed. Different bytes ⇒ different etag.
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#a> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        // The NEXT cached read MUST see the new rules: Bob is now FORBIDDEN (no stale Allow), Alice now
        // allowed. The etag changed, so the cache misses + re-parses the new ACL.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None).await.unwrap(),
            Decision::Forbidden,
            "a rotated ACL must DENY the now-removed agent — the cache must not serve a stale grant"
        );
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    /// The `=0` disabled cache yields BYTE-IDENTICAL decisions to no cache at all (the off-switch). A
    /// disabled cache never stores, so every read re-resolves — its decisions must equal the
    /// uncached path exactly, across the same shapes.
    #[tokio::test]
    async fn disabled_cache_is_byte_identical_to_no_cache() {
        let s = store();
        let resource = "https://pod.example/alice/d/doc";
        put_acl(
            &s,
            "https://pod.example/alice/d/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let disabled = AclCache::disabled();
        for (web_id, origin) in [(None, None), (Some(ALICE), None), (Some(BOB), None)] {
            let uncached = WacAuthorizer::new(&s, BASE)
                .authorize_read(resource, AccessMode::Read, web_id, origin)
                .await
                .unwrap();
            let off = WacAuthorizer::with_cache(&s, BASE, &disabled)
                .authorize_read(resource, AccessMode::Read, web_id, origin)
                .await
                .unwrap();
            assert_eq!(
                off, uncached,
                "disabled cache must equal no-cache for web_id={web_id:?}"
            );
        }
        // A disabled cache never stored anything.
        assert_eq!(disabled.len(), 0);
    }

    // --- read-2: the PLANNED resolve is decision-equivalent to the sequential walk ----------------

    /// Run BOTH read paths — the sequential [`WacAuthorizer::authorize_read`] and the planned
    /// [`WacAuthorizer::authorize_read_planned`] over a REAL [`Store::read_plan`] round — for the
    /// same `(target, required, web_id, origin)` and assert IDENTICAL [`ReadDecision`]s (incl. the
    /// full `EffectivePermissions` on Allow). This is the security-critical equivalence of read-2:
    /// the combined-query walk must be byte-for-byte the same decision as the sequential walk.
    async fn assert_planned_matches_sequential(
        wac: &WacAuthorizer<'_, TestStore>,
        store: &TestStore,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) {
        let sequential = wac
            .authorize_read(target, required, web_id, origin)
            .await
            .unwrap();
        let candidates = wac.read_plan_candidates(target);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = store.read_plan(target, &acl_iris).await.unwrap();
        let planned = wac
            .authorize_read_planned(required, web_id, origin, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(
            planned, sequential,
            "planned read must equal the sequential walk for target={target} web_id={web_id:?} \
             origin={origin:?}"
        );
    }

    /// The full-matrix differential: every ACL shape the suite exercises — public-read /
    /// origin-scoped-public / private / inherited-default / nearest-overrides / broken-fail-closed
    /// / no-ACL-orphan / `.acl`-Control (the two-IRI-roles case) — through BOTH paths, uncached AND
    /// cached (cold + warm), asserting identical decisions throughout.
    #[tokio::test]
    async fn planned_read_is_decision_equivalent_across_the_wac_matrix() {
        const APP: &str = "https://app.example";
        const OTHER: &str = "https://evil.example";
        let s = store();
        // public-read + owner-control + an origin-scoped public Append (own ACL, k=0).
        let public_doc = "https://pod.example/alice/test/doc";
        put_acl(
            &s,
            "https://pod.example/alice/test/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{public_doc}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{public_doc}>; acl:mode acl:Read.
                <#s> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <{APP}>; acl:accessTo <{public_doc}>; acl:mode acl:Append."#
            ),
        )
        .await;
        // private (only Alice; own ACL).
        let secret = "https://pod.example/alice/test/secret";
        put_acl(
            &s,
            "https://pod.example/alice/test/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{secret}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        // inherited-default at depth (k>0): /alice/.acl governs /alice/inh/deeper/data.
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let inherited = "https://pod.example/alice/inh/deeper/data";
        // nearest-overrides: /alice/test/.acl (Alice only) overrides /alice/.acl for its subtree.
        put_acl(
            &s,
            "https://pod.example/alice/test/.acl",
            &owner_default_acl("https://pod.example/alice/test/", ALICE),
        )
        .await;
        let overridden = "https://pod.example/alice/test/inh";
        // broken own-ACL (fail-closed, must NOT fall through to /alice/.acl).
        let broken = "https://pod.example/alice/broken";
        put_acl(
            &s,
            "https://pod.example/alice/broken.acl",
            "@@@ not valid turtle <<< broken",
        )
        .await;
        // no-ACL orphan anywhere.
        let orphan = "https://pod.example/zzz/orphan";
        // the `.acl` documents themselves (Control-gated; candidates derive from the PROTECTED
        // resource — the two-IRI-roles case).
        let doc_acl = "https://pod.example/alice/test/doc.acl";
        let container_acl = "https://pod.example/alice/test/.acl";

        let cases: &[(&str, AccessMode, Option<&str>, Option<&str>)] = &[
            (public_doc, AccessMode::Read, None, None),
            (public_doc, AccessMode::Read, Some(ALICE), None),
            (public_doc, AccessMode::Read, Some(ALICE), Some(APP)),
            (public_doc, AccessMode::Read, Some(ALICE), Some(OTHER)),
            (public_doc, AccessMode::Read, Some(BOB), Some(APP)),
            (secret, AccessMode::Read, None, None),
            (secret, AccessMode::Read, Some(BOB), None),
            (secret, AccessMode::Read, Some(ALICE), None),
            (inherited, AccessMode::Read, Some(ALICE), None),
            (inherited, AccessMode::Read, Some(BOB), None),
            (inherited, AccessMode::Read, None, None),
            (overridden, AccessMode::Read, Some(ALICE), None),
            (overridden, AccessMode::Read, Some(BOB), None),
            (broken, AccessMode::Read, Some(ALICE), None),
            (broken, AccessMode::Read, None, None),
            (orphan, AccessMode::Read, None, None),
            (orphan, AccessMode::Read, Some(BOB), None),
            (doc_acl, AccessMode::Control, Some(ALICE), None),
            (doc_acl, AccessMode::Control, Some(BOB), None),
            (doc_acl, AccessMode::Control, None, None),
            (container_acl, AccessMode::Control, Some(ALICE), None),
            (container_acl, AccessMode::Control, Some(BOB), None),
        ];

        // UNCACHED: planned == sequential for every case.
        let uncached = WacAuthorizer::new(&s, BASE);
        for (target, mode, web_id, origin) in cases {
            assert_planned_matches_sequential(&uncached, &s, target, *mode, *web_id, *origin).await;
        }
        // CACHED, cold then warm: the FIRST pass populates via the planned path (cold), the SECOND
        // reuses the etag-gated parse (warm) — both must still equal the sequential walk.
        let cache = AclCache::new(64);
        let cached = WacAuthorizer::with_cache(&s, BASE, &cache);
        for (target, mode, web_id, origin) in cases {
            assert_planned_matches_sequential(&cached, &s, target, *mode, *web_id, *origin).await;
            assert_planned_matches_sequential(&cached, &s, target, *mode, *web_id, *origin).await;
        }
    }

    // --- write-2: the mode-generic PLANNED authorize is decision-equivalent to `authorize` -------

    /// Run BOTH mode-generic paths — the sequential [`WacAuthorizer::authorize`] and the planned
    /// [`WacAuthorizer::authorize_planned`] over a REAL [`Store::read_plan`] round — for the same
    /// `(target, required, web_id, origin)` and assert IDENTICAL [`Decision`]s (incl. the granted
    /// mode set on Allow). This is the security-critical equivalence of write-2: the write verbs'
    /// planned walk must decide byte-for-byte like the sequential walk, for EVERY mode.
    async fn assert_planned_authorize_matches_sequential(
        wac: &WacAuthorizer<'_, TestStore>,
        store: &TestStore,
        target: &str,
        required: AccessMode,
        web_id: Option<&str>,
        origin: Option<&str>,
    ) {
        let sequential = wac
            .authorize(target, required, web_id, origin)
            .await
            .unwrap();
        let candidates = wac.read_plan_candidates(target);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = store.read_plan(target, &acl_iris).await.unwrap();
        let planned = wac
            .authorize_planned(required, web_id, origin, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(
            planned, sequential,
            "planned authorize must equal the sequential walk for target={target} \
             required={required:?} web_id={web_id:?} origin={origin:?}"
        );
    }

    /// The full-matrix mode-generic differential: the SAME ACL shapes as the read matrix —
    /// public-read / origin-scoped-public-Append / private / inherited-default / nearest-overrides
    /// / broken-fail-closed / no-ACL-orphan / `.acl`-Control — crossed with EVERY [`AccessMode`]
    /// (Read, Write, Append, Control), every principal (anonymous, owner, other), and every origin
    /// (none, matching, foreign), through BOTH paths, uncached AND cached (cold + warm). This is
    /// the write-verb (PUT/POST/DELETE/PATCH) decision surface, exhaustively equal.
    #[tokio::test]
    async fn planned_authorize_is_decision_equivalent_across_modes_and_the_wac_matrix() {
        const APP: &str = "https://app.example";
        const OTHER: &str = "https://evil.example";
        let s = store();
        let public_doc = "https://pod.example/alice/test/doc";
        put_acl(
            &s,
            "https://pod.example/alice/test/doc.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                @prefix foaf: <http://xmlns.com/foaf/0.1/>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{public_doc}>; acl:mode acl:Read, acl:Write, acl:Control.
                <#p> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <{public_doc}>; acl:mode acl:Read.
                <#s> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <{APP}>; acl:accessTo <{public_doc}>; acl:mode acl:Append."#
            ),
        )
        .await;
        let secret = "https://pod.example/alice/test/secret";
        put_acl(
            &s,
            "https://pod.example/alice/test/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{secret}>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let inherited = "https://pod.example/alice/inh/deeper/data";
        put_acl(
            &s,
            "https://pod.example/alice/test/.acl",
            &owner_default_acl("https://pod.example/alice/test/", ALICE),
        )
        .await;
        let overridden = "https://pod.example/alice/test/inh";
        let broken = "https://pod.example/alice/broken";
        put_acl(
            &s,
            "https://pod.example/alice/broken.acl",
            "@@@ not valid turtle <<< broken",
        )
        .await;
        let orphan = "https://pod.example/zzz/orphan";
        let doc_acl = "https://pod.example/alice/test/doc.acl";
        let container_acl = "https://pod.example/alice/test/.acl";

        let targets = [
            public_doc,
            secret,
            inherited,
            overridden,
            broken,
            orphan,
            doc_acl,
            container_acl,
        ];
        let modes = [
            AccessMode::Read,
            AccessMode::Write,
            AccessMode::Append,
            AccessMode::Control,
        ];
        let principals = [None, Some(ALICE), Some(BOB)];
        let origins = [None, Some(APP), Some(OTHER)];

        // UNCACHED: planned == sequential for the full cross-product.
        let uncached = WacAuthorizer::new(&s, BASE);
        for target in targets {
            for mode in modes {
                for web_id in principals {
                    for origin in origins {
                        assert_planned_authorize_matches_sequential(
                            &uncached, &s, target, mode, web_id, origin,
                        )
                        .await;
                    }
                }
            }
        }
        // CACHED, cold then warm — both must still equal the sequential walk.
        let cache = AclCache::new(64);
        let cached = WacAuthorizer::with_cache(&s, BASE, &cache);
        for target in targets {
            for mode in modes {
                for web_id in principals {
                    assert_planned_authorize_matches_sequential(
                        &cached, &s, target, mode, web_id, None,
                    )
                    .await;
                    assert_planned_authorize_matches_sequential(
                        &cached, &s, target, mode, web_id, None,
                    )
                    .await;
                }
            }
        }
    }

    /// `authorize_planned` REFUSES a plan that does not pair 1:1 with the candidates — fail-closed
    /// (an error, never a partial/shifted evaluation) — the write-verb twin of the read guard.
    #[tokio::test]
    async fn planned_authorize_refuses_a_mismatched_plan_fail_closed() {
        let s = store();
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let target = "https://pod.example/alice/deep/doc";
        let wac = WacAuthorizer::new(&s, BASE);
        let candidates = wac.read_plan_candidates(target);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = s.read_plan(target, &acl_iris).await.unwrap();

        // Too short.
        let short = plan.acls[..plan.acls.len() - 1].to_vec();
        assert!(wac
            .authorize_planned(AccessMode::Write, Some(ALICE), None, &candidates, &short)
            .await
            .is_err());
        // Shifted IRIs.
        let mut shifted = plan.acls.clone();
        shifted.swap(0, 1);
        assert!(wac
            .authorize_planned(AccessMode::Write, Some(ALICE), None, &candidates, &shifted)
            .await
            .is_err());
    }

    /// A governing ACL that the plan saw PRESENT (and whose parse is CACHED) but is DELETED before
    /// `authorize_planned` evaluates must NOT grant a WRITE from the stale cache — the live
    /// re-confirm fails closed, bit-for-bit with the sequential walk. This is the
    /// delete-after-plan window for the WRITE verbs (the fail-open a stale plan-time etag would
    /// reintroduce).
    #[tokio::test]
    async fn planned_authorize_cached_acl_deleted_after_plan_fails_closed() {
        let s = store();
        let acl_iri = "https://pod.example/alice/.acl";
        put_acl(
            &s,
            acl_iri,
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let target = "https://pod.example/alice/doc";

        let cache = AclCache::new(64);
        let wac = WacAuthorizer::with_cache(&s, BASE, &cache);
        // Warm the parse cache + take the plan while the ACL is PRESENT.
        assert!(matches!(
            wac.authorize(target, AccessMode::Write, Some(ALICE), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        let candidates = wac.read_plan_candidates(target);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = s.read_plan(target, &acl_iris).await.unwrap();

        // DELETE the governing ACL after the plan (cache still holds the parse).
        s.delete(acl_iri, None).await.unwrap();

        let planned = wac
            .authorize_planned(
                AccessMode::Write,
                Some(ALICE),
                None,
                &candidates,
                &plan.acls,
            )
            .await
            .unwrap();
        assert_eq!(
            planned,
            Decision::Forbidden,
            "a write must NEVER be granted from a cached parse of a deleted ACL"
        );
        let sequential = wac
            .authorize(target, AccessMode::Write, Some(ALICE), None)
            .await
            .unwrap();
        assert_eq!(planned, sequential, "fail-closed on both paths");
    }

    /// The candidate chain derives from the PROTECTED resource (the two-IRI-roles rule): for an
    /// `.acl` target the chain starts at that `.acl` ITSELF (governing the stripped resource),
    /// never at a non-existent `foo.acl.acl`; for a container its own `/.acl` comes first, then the
    /// parents'.
    #[test]
    fn read_plan_candidates_derive_from_the_protected_resource() {
        let s = store();
        let wac = WacAuthorizer::new(&s, BASE);
        // A document: own acl first, then ancestors nearest-first up to the root.
        let doc = wac.read_plan_candidates("https://pod.example/a/b/doc");
        assert_eq!(
            doc.iter().map(|c| c.acl.as_str()).collect::<Vec<_>>(),
            vec![
                "https://pod.example/a/b/doc.acl",
                "https://pod.example/a/b/.acl",
                "https://pod.example/a/.acl",
                "https://pod.example/.acl",
            ]
        );
        assert_eq!(doc[0].governed, "https://pod.example/a/b/doc");
        assert_eq!(doc[1].governed, "https://pod.example/a/b/");
        // An `.acl` target: the chain is the PROTECTED resource's — element 0 is the target itself.
        let acl = wac.read_plan_candidates("https://pod.example/a/b/doc.acl");
        assert_eq!(
            acl.iter().map(|c| c.acl.as_str()).collect::<Vec<_>>(),
            vec![
                "https://pod.example/a/b/doc.acl",
                "https://pod.example/a/b/.acl",
                "https://pod.example/a/.acl",
                "https://pod.example/.acl",
            ],
            "an .acl target's chain starts at itself (never foo.acl.acl)"
        );
        assert_eq!(acl[0].governed, "https://pod.example/a/b/doc");
        // A container: its own /.acl first, then the PARENT's.
        let c = wac.read_plan_candidates("https://pod.example/a/b/");
        assert_eq!(
            c.iter().map(|c| c.acl.as_str()).collect::<Vec<_>>(),
            vec![
                "https://pod.example/a/b/.acl",
                "https://pod.example/a/.acl",
                "https://pod.example/.acl",
            ]
        );
        // The root itself: only its own ACL.
        let root = wac.read_plan_candidates("https://pod.example/");
        assert_eq!(
            root.iter().map(|c| c.acl.as_str()).collect::<Vec<_>>(),
            vec!["https://pod.example/.acl"]
        );
    }

    /// A mismatched plan (wrong length or wrong IRI pairing) is REFUSED fail-closed — never a
    /// partial/shifted evaluation.
    #[tokio::test]
    async fn planned_read_refuses_a_mismatched_plan_fail_closed() {
        let s = store();
        let resource = "https://pod.example/alice/x";
        put_acl(
            &s,
            "https://pod.example/alice/x.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#o> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        let candidates = wac.read_plan_candidates(resource);
        // Wrong LENGTH (a truncated plan).
        let short: Vec<(String, Option<String>)> = vec![(candidates[0].acl.clone(), None)];
        let err = wac
            .authorize_read_planned(AccessMode::Read, Some(ALICE), None, &candidates, &short)
            .await
            .expect_err("a truncated plan must be refused");
        assert!(matches!(err, ServerError::Storage(_)));
        // Wrong IRI pairing (a shifted plan).
        let mut shifted: Vec<(String, Option<String>)> =
            candidates.iter().map(|c| (c.acl.clone(), None)).collect();
        shifted.swap(0, 1);
        let err = wac
            .authorize_read_planned(AccessMode::Read, Some(ALICE), None, &candidates, &shifted)
            .await
            .expect_err("a shifted plan must be refused");
        assert!(matches!(err, ServerError::Storage(_)));
    }

    /// An ACL that VANISHES between the plan and the triple read (a concurrent DELETE) is treated
    /// as absent-keep-walking — exactly the sequential walk's post-probe `NotFound` semantics: the
    /// resolution falls through to the next present candidate.
    #[tokio::test]
    async fn planned_read_vanished_acl_falls_through_to_the_next_candidate() {
        let s = store();
        let resource = "https://pod.example/alice/v/data";
        let own_acl = "https://pod.example/alice/v/data.acl";
        // The own ACL would grant BOB; the ancestor grants ALICE (default).
        put_acl(
            &s,
            own_acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#b> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        put_acl(
            &s,
            "https://pod.example/alice/.acl",
            &owner_default_acl("https://pod.example/alice/", ALICE),
        )
        .await;
        let wac = WacAuthorizer::new(&s, BASE);
        let candidates = wac.read_plan_candidates(resource);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        // Take the plan WHILE the own ACL exists…
        let plan = s.read_plan(resource, &acl_iris).await.unwrap();
        assert!(plan.acls[0].1.is_some(), "own acl present in the plan");
        // …then DELETE it before evaluation (the concurrent-DELETE window).
        s.delete(own_acl, None).await.unwrap();
        // Bob's own-ACL grant is GONE (the vanished ACL is not resurrected); the walk falls
        // through to the ancestor default, which grants only Alice.
        let bob = wac
            .authorize_read_planned(AccessMode::Read, Some(BOB), None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(bob, ReadDecision::Forbidden);
        let alice = wac
            .authorize_read_planned(AccessMode::Read, Some(ALICE), None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert!(matches!(alice, ReadDecision::Allow(_)));
    }

    /// SECURITY REGRESSION (the roborev Medium): the CACHED delete-after-plan window. An ACL that is
    /// PRESENT and its parse is CACHED at plan time, then DELETED before `authorize_read_planned`,
    /// must NOT authorize from the stale cache — the planned eval must fail-closed, bit-for-bit with
    /// the sequential walk (which re-probes existence live). Before the fix, `read_acl_pinned` served
    /// the cached grant keyed on the stale plan-time etag → authorized from a DELETED ACL.
    #[tokio::test]
    async fn planned_read_cached_acl_deleted_after_plan_fails_closed() {
        let s = store();
        let resource = "https://pod.example/alice/cd/data";
        let acl = "https://pod.example/alice/cd/data.acl";
        // The own ACL grants BOB read.
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#b> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let cache = AclCache::new(64);
        let wac = WacAuthorizer::with_cache(&s, BASE, &cache);
        // Populate the cache: a warm read caches the ACL's parse under its (present) etag.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // Take the plan WHILE the ACL is present + cached (present at plan time).
        let candidates = wac.read_plan_candidates(resource);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = s.read_plan(resource, &acl_iris).await.unwrap();
        assert!(
            plan.acls[0].1.is_some(),
            "the own ACL is present (with an etag) in the plan"
        );
        // DELETE the ACL AFTER the plan (the delete-after-plan window the cache-hit path missed).
        s.delete(acl, None).await.unwrap();
        // Planned eval with the STALE plan must fail-closed — NOT grant from the stale cache.
        let planned = wac
            .authorize_read_planned(AccessMode::Read, Some(BOB), None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(
            planned,
            ReadDecision::Forbidden,
            "must NOT authorize BOB from a deleted-since-plan ACL's stale cache"
        );
        // Anonymous likewise → 401 (fail-closed).
        let planned_anon = wac
            .authorize_read_planned(AccessMode::Read, None, None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(planned_anon, ReadDecision::Unauthenticated);
        // The security oracle: bit-for-bit with the sequential walk (which re-probes live).
        let sequential = wac
            .authorize_read(resource, AccessMode::Read, Some(BOB), None)
            .await
            .unwrap();
        assert_eq!(
            planned, sequential,
            "planned must equal the sequential decision on the cached-delete case"
        );
    }

    /// SECURITY REGRESSION: the CACHED rotate-after-plan window. An ACL present + cached at plan
    /// time, then REWRITTEN to different rules (new etag) before eval, must reflect the CURRENT
    /// rules — never the stale cache. The live re-confirm re-probes the ACL's CURRENT etag, so the
    /// cache misses on the rotated etag and re-parses; bit-for-bit with the sequential walk.
    #[tokio::test]
    async fn planned_read_cached_acl_rotated_after_plan_uses_current_rules() {
        let s = store();
        let resource = "https://pod.example/alice/rot2/data";
        let acl = "https://pod.example/alice/rot2/data.acl";
        // Initially: BOB may read.
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#b> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let cache = AclCache::new(64);
        let wac = WacAuthorizer::with_cache(&s, BASE, &cache);
        // Populate the cache (BOB allowed) + take the plan while present.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        let candidates = wac.read_plan_candidates(resource);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = s.read_plan(resource, &acl_iris).await.unwrap();
        // ROTATE: now only ALICE may read (different bytes ⇒ different etag).
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#a> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        // Planned eval with the STALE plan must see the NEW rules: BOB now forbidden, ALICE allowed.
        let bob = wac
            .authorize_read_planned(AccessMode::Read, Some(BOB), None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert_eq!(
            bob,
            ReadDecision::Forbidden,
            "the rotated (now-removed) BOB grant must not be served from the stale cache"
        );
        let alice = wac
            .authorize_read_planned(AccessMode::Read, Some(ALICE), None, &candidates, &plan.acls)
            .await
            .unwrap();
        assert!(matches!(alice, ReadDecision::Allow(_)));
        // Bit-for-bit with the sequential walk.
        assert_eq!(
            bob,
            wac.authorize_read(resource, AccessMode::Read, Some(BOB), None)
                .await
                .unwrap()
        );
    }

    /// A removed ACL is NEVER resurrected by the cache: populate the cache with an ALLOW via an own
    /// ACL, then DELETE that `.acl` so the resource has no governing ACL anywhere → the cached resolve
    /// must now DENY (fail-closed), proving the cache cannot fabricate a deleted grant. The `meta`
    /// probe returns `None` for the deleted ACL, so the resolver never even consults the cache for it.
    #[tokio::test]
    async fn deleted_acl_is_not_resurrected_by_cache() {
        let s = store();
        let resource = "https://pod.example/alice/del/data";
        let acl = "https://pod.example/alice/del/data.acl";
        put_acl(
            &s,
            acl,
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
                <#p> a acl:Authorization; acl:agentClass <http://xmlns.com/foaf/0.1/Agent>; acl:accessTo <{resource}>; acl:mode acl:Read."#
            ),
        )
        .await;
        let cache = AclCache::new(64);
        let wac = WacAuthorizer::with_cache(&s, BASE, &cache);
        // Cold + warm: anonymous read is ALLOWED (public) and the cache is populated.
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert!(matches!(
            wac.authorize(resource, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // DELETE the own ACL — no ACL governs the resource anywhere now (no ancestor ACL either).
        s.delete(acl, None).await.expect("delete acl");
        // The cached resolve must now DENY (fail-closed: no ACL → 401 for anonymous). The deleted ACL
        // is gone from the index, so the `meta` probe reports it absent and the walk inherits nothing.
        assert_eq!(
            wac.authorize(resource, AccessMode::Read, None, None).await.unwrap(),
            Decision::Unauthenticated,
            "a deleted ACL must NOT be served from cache — removing it must fail-close the resource"
        );
    }
}

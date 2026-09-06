// AUTHORED-BY Claude Opus 4.8
//! Web Access Control rule-matching: turn a parsed `.acl` graph into the set of [`AccessMode`]s
//! granted to a requester for a given resource + inheritance scope.
//!
//! The ACL document is parsed via `oxttl`/`oxjsonld` into `oxrdf::Triple`s (the house rule — NEVER
//! hand-parse/concat ACL by string) and matched here against the `acl:` vocabulary. This is the
//! semantic port of prod-solid-server `src/authz/acl.ts`.
//!
//! A rule (an `acl:Authorization`) grants a requester a mode when:
//!  - the rule's scope predicate (`acl:accessTo` for an own ACL, `acl:default`/`acl:defaultForNew`
//!    for an inherited ancestor ACL) references the target resource, AND
//!  - the rule matches the requester — by `acl:agent <webid>`, by `acl:agentClass foaf:Agent`
//!    (public — everyone, incl. anonymous), by `acl:agentClass acl:AuthenticatedAgent` (any
//!    authenticated WebID), or by `acl:agentGroup <group>` when the requester has been RESOLVED into
//!    that group's membership (see below), AND
//!  - the rule lists the mode via `acl:mode`.
//!
//! ## `acl:agentGroup`
//! Deciding group membership needs I/O — the group document has to be read — and this module is
//! PURE rule-matching, so it never fetches anything. The caller resolves memberships FIRST
//! ([`agent_group_iris`] names exactly the group IRIs worth resolving, [`group_document_iri`] +
//! [`group_has_member`] do the vocabulary work) and passes the verified set in [`Requester::groups`].
//! A group that is not in that set NEVER matches, so an anonymous requester, an unresolvable or
//! unreachable group document, and a caller that resolves nothing at all all stay fail-closed.
//! [`super::wac::WacAuthorizer`] performs the resolution through the `Store`.

use std::collections::BTreeSet;

use oxrdf::{NamedOrBlankNode, Term, Triple};

use super::mode::AccessMode;

const ACL: &str = "http://www.w3.org/ns/auth/acl#";

fn acl_iri(local: &str) -> String {
    format!("{ACL}{local}")
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";
/// The WAC group-membership predicate: `<group> vcard:hasMember <webid>` in a group document.
const VCARD_HAS_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";

/// Which scope of a rule applies: a rule for the resource itself (`acl:accessTo`), or one inherited
/// from an ancestor container (`acl:default`, plus the legacy `acl:defaultForNew`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclScope {
    /// The ACL is the resource's OWN ACL: only `acl:accessTo <resource>` rules apply.
    AccessTo,
    /// The ACL belongs to an ancestor container: only `acl:default <container>` (or the legacy
    /// `acl:defaultForNew`) rules apply (WAC inheritance).
    Default,
}

/// The verified requester identity as the matcher needs it.
#[derive(Debug, Clone, Default)]
pub struct Requester<'a> {
    /// The requester's WebID, or `None` for an anonymous/public request.
    pub web_id: Option<&'a str>,
    /// The request's HTTP `Origin` header (the requesting web app's origin), or `None` when the
    /// request carried no `Origin` (e.g. a non-browser / same-origin / server-to-server request).
    ///
    /// WAC's `acl:origin` restriction is matched against THIS value: a rule that lists one or more
    /// `acl:origin` values grants ONLY when this origin matches one of them. A request with NO
    /// `Origin` can never satisfy an `acl:origin`-restricted rule (fail-closed) — but it is fully
    /// unaffected by a rule that carries no `acl:origin` (the common case).
    pub origin: Option<&'a str>,
    /// The `acl:agentGroup` IRIs this requester has been VERIFIED to be a member of, resolved by the
    /// caller BEFORE matching (membership resolution is I/O — see the module docs). A rule's
    /// `acl:agentGroup <g>` matches this requester iff `g` appears here.
    ///
    /// Empty — the [`Default`] — means "no group membership was established", which is exactly the
    /// fail-closed posture for an anonymous requester, for a group document that is missing,
    /// unreachable or malformed, and for a caller that does not resolve groups at all: such a rule
    /// simply never matches. Widening a grant therefore always requires a positive, resolved
    /// membership; nothing about group handling can grant by omission.
    pub groups: &'a [String],
}

impl<'a> Requester<'a> {
    pub fn anonymous() -> Self {
        Self {
            web_id: None,
            origin: None,
            groups: &[],
        }
    }
    pub fn authenticated(web_id: &'a str) -> Self {
        Self {
            web_id: Some(web_id),
            origin: None,
            groups: &[],
        }
    }
    fn is_authenticated(&self) -> bool {
        self.web_id.is_some()
    }
}

/// Compute the set of access modes the `.acl` graph (`triples`) grants to `requester` for `resource`
/// under the inheritance `scope`.
///
/// Returns an empty set when no rule matches (fail-closed). A malformed `acl:mode` object (e.g. a
/// literal where a NamedNode is expected) is IGNORED, never fatal — ACL documents are user-controlled,
/// so a single bad triple must not deny a whole, otherwise-valid rule or crash authorization.
pub fn modes_for(
    triples: &[Triple],
    resource: &str,
    requester: &Requester<'_>,
    scope: AclScope,
) -> BTreeSet<AccessMode> {
    let mut granted = BTreeSet::new();

    for rule in authorization_subjects(triples) {
        if !applies_to_resource(triples, &rule, resource, scope) {
            continue;
        }
        if !matches_agent(triples, &rule, requester) {
            continue;
        }
        // `acl:origin` restriction (app-scoping): a rule with one or more `acl:origin` values grants
        // ONLY when the request's Origin matches one of them. A rule with NO `acl:origin` applies
        // regardless of origin (the common case). Checked AFTER the agent match — both must hold.
        if !matches_origin(triples, &rule, requester) {
            continue;
        }
        for mode in granted_modes(triples, &rule) {
            granted.insert(mode);
        }
    }
    granted
}

/// Whether `granted` satisfies the `required` mode. WAC's `acl:Write` subsumes `acl:Append` (a writer
/// may also append), so an `Append` requirement is met by either an explicit `Append` or a `Write`
/// grant. No other implications hold — `Control` does NOT imply Read/Write of the resource body.
pub fn satisfies(granted: &BTreeSet<AccessMode>, required: AccessMode) -> bool {
    if granted.contains(&required) {
        return true;
    }
    required == AccessMode::Append && granted.contains(&AccessMode::Write)
}

/// The subjects of every `?s a acl:Authorization` triple in the graph (the authorization rules).
fn authorization_subjects(triples: &[Triple]) -> Vec<NamedOrBlankNode> {
    let authorization = acl_iri("Authorization");
    let mut subjects: Vec<NamedOrBlankNode> = Vec::new();
    for t in triples {
        if t.predicate.as_str() == RDF_TYPE {
            if let Term::NamedNode(obj) = &t.object {
                if obj.as_str() == authorization && !subjects.contains(&t.subject) {
                    subjects.push(t.subject.clone());
                }
            }
        }
    }
    subjects
}

/// All NamedNode objects of `(subject, predicate)` in the graph.
fn named_objects<'a>(
    triples: &'a [Triple],
    subject: &NamedOrBlankNode,
    predicate: &str,
) -> Vec<&'a str> {
    let mut out = Vec::new();
    for t in triples {
        if &t.subject == subject && t.predicate.as_str() == predicate {
            if let Term::NamedNode(obj) = &t.object {
                out.push(obj.as_str());
            }
        }
    }
    out
}

/// Whether a rule's scope predicate references `resource`. WAC permits an authorization to list
/// MULTIPLE `acl:accessTo`/`acl:default` targets, so every object of the scope predicate is checked.
fn applies_to_resource(
    triples: &[Triple],
    rule: &NamedOrBlankNode,
    resource: &str,
    scope: AclScope,
) -> bool {
    let predicates: &[String] = &match scope {
        AclScope::AccessTo => vec![acl_iri("accessTo")],
        AclScope::Default => vec![acl_iri("default"), acl_iri("defaultForNew")],
    };
    for predicate in predicates {
        for obj in named_objects(triples, rule, predicate) {
            if obj == resource {
                return true;
            }
        }
    }
    false
}

/// Whether the rule grants access to the requester (by exact WebID, the public class, the
/// authenticated class, or a RESOLVED `acl:agentGroup` membership — see [`Requester::groups`]).
fn matches_agent(triples: &[Triple], rule: &NamedOrBlankNode, requester: &Requester<'_>) -> bool {
    let agent_class = acl_iri("agentClass");
    let authenticated_agent = acl_iri("AuthenticatedAgent");

    // `acl:agentClass foaf:Agent` — public, matches every requester (authenticated or not).
    // `acl:agentClass acl:AuthenticatedAgent` — matches any authenticated WebID.
    for class in named_objects(triples, rule, &agent_class) {
        if class == FOAF_AGENT {
            return true;
        }
        if class == authenticated_agent && requester.is_authenticated() {
            return true;
        }
    }

    // `acl:agent <webid>` — matches the requester's exact WebID.
    if let Some(web_id) = requester.web_id {
        let agent = acl_iri("agent");
        for a in named_objects(triples, rule, &agent) {
            if a == web_id {
                return true;
            }
        }
    }

    // `acl:agentGroup <group>` — matches when the caller has already RESOLVED this requester into
    // that group's membership. `requester.groups` is empty unless a caller did that (I/O-bearing)
    // resolution, so an unresolved group never matches (fail-closed). Also guarded on being
    // authenticated: only a WebID can be a `vcard:hasMember`, so an anonymous requester can never
    // reach a group grant even if a caller mistakenly hands one over.
    if requester.is_authenticated() && !requester.groups.is_empty() {
        let agent_group = acl_iri("agentGroup");
        for group in named_objects(triples, rule, &agent_group) {
            if requester.groups.iter().any(|resolved| resolved == group) {
                return true;
            }
        }
    }

    false
}

/// The DISTINCT `acl:agentGroup` IRIs whose membership could still change what this graph grants
/// `requester` on `resource` under `scope` — exactly the groups a caller must resolve before calling
/// [`modes_for`] with a populated [`Requester::groups`].
///
/// Rules are pre-filtered so the caller only pays I/O where it can matter: a rule that does not
/// apply to `resource` under this `scope`, whose `acl:origin` restriction the request fails, or that
/// ALREADY matches the requester (its modes are granted regardless) contributes no groups. The
/// common ACL that names no group at all therefore yields an EMPTY list, so group support costs the
/// hot path nothing. An anonymous requester likewise yields an empty list: `vcard:hasMember` can
/// only name a WebID, so no resolution could help it.
///
/// Returned in the graph's own rule order, de-duplicated.
pub fn agent_group_iris<'t>(
    triples: &'t [Triple],
    resource: &str,
    requester: &Requester<'_>,
    scope: AclScope,
) -> Vec<&'t str> {
    let mut referenced: Vec<&str> = Vec::new();
    if !requester.is_authenticated() {
        return referenced;
    }
    let agent_group = acl_iri("agentGroup");
    for rule in authorization_subjects(triples) {
        if !applies_to_resource(triples, &rule, resource, scope) {
            continue;
        }
        if !matches_origin(triples, &rule, requester) {
            continue;
        }
        if matches_agent(triples, &rule, requester) {
            continue;
        }
        for group in named_objects(triples, &rule, &agent_group) {
            if !referenced.contains(&group) {
                referenced.push(group);
            }
        }
    }
    referenced
}

/// The document IRI a group IRI lives in: the group IRI with its fragment removed
/// (`https://pod.example/groups#team` → `https://pod.example/groups`). A group IRI carrying no
/// fragment IS its own document.
pub fn group_document_iri(group: &str) -> &str {
    match group.find('#') {
        Some(hash) => &group[..hash],
        None => group,
    }
}

/// Whether the parsed group document `triples` lists `web_id` as a member of `group` — the WAC
/// `acl:agentGroup` membership test, `<group> vcard:hasMember <web_id>`.
///
/// The subject is matched on the FULL group IRI (fragment included), so several groups sharing one
/// document keep separate memberships. A blank-node subject or a non-`NamedNode` object is ignored:
/// group documents are user-controlled exactly like ACLs, so a single bad triple must never deny a
/// valid membership or crash authorization.
pub fn group_has_member(triples: &[Triple], group: &str, web_id: &str) -> bool {
    triples.iter().any(|t| {
        t.predicate.as_str() == VCARD_HAS_MEMBER
            && matches!(&t.subject, NamedOrBlankNode::NamedNode(s) if s.as_str() == group)
            && matches!(&t.object, Term::NamedNode(o) if o.as_str() == web_id)
    })
}

/// Whether the rule's `acl:origin` restriction (if any) is satisfied by the request's Origin.
///
/// WAC `acl:origin` (the trusted-app restriction): a rule that lists one or more `acl:origin` values
/// applies ONLY to a request whose `Origin` header EXACTLY matches one of those values; a rule with
/// NO `acl:origin` value applies to ANY origin (the common, unrestricted case). A request that
/// carried no `Origin` (anonymous-of-origin: a non-browser, same-origin, or server-to-server request)
/// can NEVER satisfy an origin-restricted rule (fail-closed) — otherwise an app-scoped authorization
/// would be valid from any caller that simply omits the header.
fn matches_origin(triples: &[Triple], rule: &NamedOrBlankNode, requester: &Requester<'_>) -> bool {
    let origin_pred = acl_iri("origin");
    let allowed = named_objects(triples, rule, &origin_pred);
    if allowed.is_empty() {
        // Unrestricted rule — applies regardless of the request's Origin.
        return true;
    }
    // Origin-restricted: the request MUST carry an Origin that exactly matches a listed value.
    match requester.origin {
        Some(origin) => allowed.contains(&origin),
        None => false,
    }
}

/// The modes a rule lists via `acl:mode`. A non-NamedNode object is ignored (defensive — ACLs are
/// user-controlled), and an unrecognised mode IRI contributes nothing.
fn granted_modes(triples: &[Triple], rule: &NamedOrBlankNode) -> Vec<AccessMode> {
    let mode = acl_iri("mode");
    let mut modes = Vec::new();
    for iri in named_objects(triples, rule, &mode) {
        if let Some(m) = AccessMode::from_acl_iri(iri) {
            modes.push(m);
        }
    }
    modes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ldp::content::{parse_to_triples, RdfFormat};

    const RES: &str = "https://pod.example/alice/test/data";
    const CONTAINER: &str = "https://pod.example/alice/test/";
    const ALICE: &str = "https://pod.example/alice/profile/card#me";
    const BOB: &str = "https://pod.example/bob/profile/card#me";

    fn parse(ttl: &str) -> Vec<Triple> {
        parse_to_triples(
            RdfFormat::Turtle,
            ttl.as_bytes(),
            "https://pod.example/alice/test/.acl",
        )
        .expect("valid acl turtle")
    }

    fn modes(
        t: &[Triple],
        resource: &str,
        web_id: Option<&str>,
        scope: AclScope,
    ) -> BTreeSet<AccessMode> {
        let r = Requester {
            web_id,
            origin: None,
            groups: &[],
        };
        modes_for(t, resource, &r, scope)
    }

    const TEAM: &str = "https://pod.example/groups#team";
    const OTHER_TEAM: &str = "https://pod.example/groups#other";

    /// An ACL granting Read on [`RES`] to the members of [`TEAM`] and nobody else.
    fn group_acl() -> String {
        format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#grp> a acl:Authorization;
                   acl:agentGroup <{TEAM}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        )
    }

    /// Like [`modes`] but with a RESOLVED `acl:agentGroup` membership set — what the WAC authorizer
    /// hands the matcher after reading the group documents.
    fn modes_in_groups(
        t: &[Triple],
        resource: &str,
        web_id: Option<&str>,
        groups: &[String],
        scope: AclScope,
    ) -> BTreeSet<AccessMode> {
        let r = Requester {
            web_id,
            origin: None,
            groups,
        };
        modes_for(t, resource, &r, scope)
    }

    /// Like [`modes`] but with an explicit request `Origin` — for the `acl:origin` tests.
    fn modes_with_origin(
        t: &[Triple],
        resource: &str,
        web_id: Option<&str>,
        origin: Option<&str>,
        scope: AclScope,
    ) -> BTreeSet<AccessMode> {
        let r = Requester {
            web_id,
            origin,
            groups: &[],
        };
        modes_for(t, resource, &r, scope)
    }

    #[test]
    fn agent_access_to_grants_only_that_agent() {
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#bob> a acl:Authorization;
                   acl:agent <{BOB}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        // Bob gets read on the resource via its OWN acl.
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Read));
        // Alice (a different agent) gets nothing.
        assert!(modes(&t, RES, Some(ALICE), AclScope::AccessTo).is_empty());
        // Anonymous gets nothing.
        assert!(modes(&t, RES, None, AclScope::AccessTo).is_empty());
        // Under the DEFAULT scope this accessTo rule does NOT apply.
        assert!(modes(&t, RES, Some(BOB), AclScope::Default).is_empty());
    }

    #[test]
    fn public_foaf_agent_grants_everyone_including_anonymous() {
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            @prefix foaf: <http://xmlns.com/foaf/0.1/>.
            <#pub> a acl:Authorization;
                   acl:agentClass foaf:Agent;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        assert!(modes(&t, RES, None, AclScope::AccessTo).contains(&AccessMode::Read));
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Read));
    }

    #[test]
    fn authenticated_agent_grants_any_authenticated_but_not_anonymous() {
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#auth> a acl:Authorization;
                    acl:agentClass acl:AuthenticatedAgent;
                    acl:accessTo <{RES}>;
                    acl:mode acl:Write."#
        );
        let t = parse(&ttl);
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Write));
        assert!(modes(&t, RES, None, AclScope::AccessTo).is_empty());
    }

    #[test]
    fn default_scope_only_matches_acl_default() {
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#bobdef> a acl:Authorization;
                      acl:agent <{BOB}>;
                      acl:default <{CONTAINER}>;
                      acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        // The default rule grants Bob read under DEFAULT scope on the container.
        assert!(modes(&t, CONTAINER, Some(BOB), AclScope::Default).contains(&AccessMode::Read));
        // Under accessTo scope (the container's OWN acl), a default-only rule does NOT apply.
        assert!(modes(&t, CONTAINER, Some(BOB), AclScope::AccessTo).is_empty());
    }

    #[test]
    fn all_four_modes_are_recognised() {
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#full> a acl:Authorization;
                    acl:agent <{ALICE}>;
                    acl:accessTo <{RES}>;
                    acl:mode acl:Read, acl:Write, acl:Append, acl:Control."#
        );
        let t = parse(&ttl);
        let m = modes(&t, RES, Some(ALICE), AclScope::AccessTo);
        assert!(m.contains(&AccessMode::Read));
        assert!(m.contains(&AccessMode::Write));
        assert!(m.contains(&AccessMode::Append));
        assert!(m.contains(&AccessMode::Control));
    }

    #[test]
    fn agent_group_unresolved_never_matches_fail_closed() {
        let t = parse(&group_acl());
        // An UNRESOLVED group grants nothing: `modes` passes an empty `Requester::groups`, which is
        // what an anonymous requester, a missing/unreachable group document, and a caller that does
        // no group resolution all produce.
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).is_empty());
        assert!(modes(&t, RES, None, AclScope::AccessTo).is_empty());
    }

    #[test]
    fn rule_for_a_different_resource_does_not_apply() {
        let other = "https://pod.example/alice/test/other";
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#x> a acl:Authorization;
                 acl:agent <{BOB}>;
                 acl:accessTo <{other}>;
                 acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).is_empty());
        assert!(modes(&t, other, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Read));
    }

    #[test]
    fn satisfies_write_subsumes_append() {
        let mut g = BTreeSet::new();
        g.insert(AccessMode::Write);
        assert!(satisfies(&g, AccessMode::Append));
        assert!(satisfies(&g, AccessMode::Write));
        assert!(!satisfies(&g, AccessMode::Read));
        // Control does NOT imply read/write.
        let mut c = BTreeSet::new();
        c.insert(AccessMode::Control);
        assert!(!satisfies(&c, AccessMode::Read));
        assert!(!satisfies(&c, AccessMode::Write));
    }

    #[test]
    fn malformed_mode_is_ignored_not_fatal() {
        // A literal `acl:mode` object (malformed) is skipped; the valid mode still grants.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#x> a acl:Authorization;
                 acl:agent <{BOB}>;
                 acl:accessTo <{RES}>;
                 acl:mode "not-a-mode";
                 acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        let m = modes(&t, RES, Some(BOB), AclScope::AccessTo);
        assert_eq!(m.len(), 1);
        assert!(m.contains(&AccessMode::Read));
    }

    #[test]
    fn multiple_access_to_targets_on_one_rule() {
        let other = "https://pod.example/alice/test/other";
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#x> a acl:Authorization;
                 acl:agent <{BOB}>;
                 acl:accessTo <{RES}>, <{other}>;
                 acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Read));
        assert!(modes(&t, other, Some(BOB), AclScope::AccessTo).contains(&AccessMode::Read));
    }

    // --- acl:agentGroup (resolved membership) --------------------------------------------------

    #[test]
    fn agent_group_grants_a_resolved_member() {
        let t = parse(&group_acl());
        let member_of_team = vec![TEAM.to_string()];
        assert!(
            modes_in_groups(&t, RES, Some(BOB), &member_of_team, AclScope::AccessTo)
                .contains(&AccessMode::Read),
            "a requester resolved into the named group must be granted the rule's modes"
        );
    }

    #[test]
    fn agent_group_does_not_grant_membership_of_a_different_group() {
        let t = parse(&group_acl());
        let member_of_other = vec![OTHER_TEAM.to_string()];
        // A DIFFERENT group — including one sharing the same document, differing only by fragment —
        // must not satisfy the rule.
        assert!(
            modes_in_groups(&t, RES, Some(BOB), &member_of_other, AclScope::AccessTo).is_empty()
        );
    }

    #[test]
    fn agent_group_never_grants_an_anonymous_requester() {
        let t = parse(&group_acl());
        // Even if a caller wrongly hands over a membership, no WebID ⇒ no group grant (fail-closed):
        // `vcard:hasMember` can only name a WebID, so an anonymous requester is never a member.
        let bogus = vec![TEAM.to_string()];
        assert!(modes_in_groups(&t, RES, None, &bogus, AclScope::AccessTo).is_empty());
    }

    #[test]
    fn agent_group_rule_still_obeys_scope_and_origin() {
        let member_of_team = vec![TEAM.to_string()];
        // Scope: an accessTo group rule does not apply under the DEFAULT scope.
        let t = parse(&group_acl());
        assert!(modes_in_groups(&t, RES, Some(BOB), &member_of_team, AclScope::Default).is_empty());

        // Origin: a group rule carrying `acl:origin` is app-scoped exactly like an agent rule.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#grp> a acl:Authorization;
                   acl:agentGroup <{TEAM}>;
                   acl:origin <{APP}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        let matching = Requester {
            web_id: Some(BOB),
            origin: Some(APP),
            groups: &member_of_team,
        };
        assert!(modes_for(&t, RES, &matching, AclScope::AccessTo).contains(&AccessMode::Read));
        let wrong_origin = Requester {
            web_id: Some(BOB),
            origin: Some(OTHER_APP),
            groups: &member_of_team,
        };
        assert!(modes_for(&t, RES, &wrong_origin, AclScope::AccessTo).is_empty());
    }

    #[test]
    fn agent_group_iris_lists_the_groups_worth_resolving() {
        let t = parse(&group_acl());
        let bob = Requester::authenticated(BOB);
        assert_eq!(agent_group_iris(&t, RES, &bob, AclScope::AccessTo), vec![TEAM]);

        // A rule that does not apply to the resource contributes nothing to resolve.
        let other = "https://pod.example/alice/test/other";
        assert!(agent_group_iris(&t, other, &bob, AclScope::AccessTo).is_empty());
        // Nor does one out of scope.
        assert!(agent_group_iris(&t, RES, &bob, AclScope::Default).is_empty());
        // Nor does any rule for an anonymous requester (no WebID can be a `vcard:hasMember`).
        assert!(agent_group_iris(&t, RES, &Requester::anonymous(), AclScope::AccessTo).is_empty());
    }

    #[test]
    fn agent_group_iris_skips_rules_that_already_match_and_dedupes() {
        // Rule 1 already matches Bob by WebID, so resolving ITS group could not change the outcome.
        // Rule 2 does not match him, so its group must be resolved — and the shared group is listed
        // exactly once.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#direct> a acl:Authorization;
                      acl:agent <{BOB}>;
                      acl:agentGroup <{OTHER_TEAM}>;
                      acl:accessTo <{RES}>;
                      acl:mode acl:Read.
            <#viagroup> a acl:Authorization;
                        acl:agentGroup <{TEAM}>, <{OTHER_TEAM}>;
                        acl:accessTo <{RES}>;
                        acl:mode acl:Write."#
        );
        let t = parse(&ttl);
        let bob = Requester::authenticated(BOB);
        let referenced = agent_group_iris(&t, RES, &bob, AclScope::AccessTo);
        assert_eq!(referenced.len(), 2, "de-duplicated across rules");
        assert!(referenced.contains(&TEAM));
        assert!(referenced.contains(&OTHER_TEAM));

        // Alice matches neither rule directly, so the first rule's group is worth resolving too.
        let alice = Requester::authenticated(ALICE);
        assert_eq!(agent_group_iris(&t, RES, &alice, AclScope::AccessTo).len(), 2);

        // And the group grant does widen: resolved into TEAM, Bob picks up Write from rule 2 on top
        // of the Read his direct rule already gave him.
        let in_team = vec![TEAM.to_string()];
        let m = modes_in_groups(&t, RES, Some(BOB), &in_team, AclScope::AccessTo);
        assert!(m.contains(&AccessMode::Read));
        assert!(m.contains(&AccessMode::Write));
    }

    #[test]
    fn group_document_iri_strips_the_fragment() {
        assert_eq!(group_document_iri(TEAM), "https://pod.example/groups");
        assert_eq!(group_document_iri(OTHER_TEAM), "https://pod.example/groups");
        // No fragment ⇒ the group IRI is its own document.
        assert_eq!(
            group_document_iri("https://pod.example/groups"),
            "https://pod.example/groups"
        );
    }

    #[test]
    fn group_has_member_matches_the_full_group_iri() {
        // ONE document holding TWO groups — memberships must not bleed across the fragment.
        let doc = parse(&format!(
            r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
            <{TEAM}> vcard:hasMember <{BOB}>.
            <{OTHER_TEAM}> vcard:hasMember <{ALICE}>."#
        ));
        assert!(group_has_member(&doc, TEAM, BOB));
        assert!(group_has_member(&doc, OTHER_TEAM, ALICE));
        assert!(!group_has_member(&doc, TEAM, ALICE));
        assert!(!group_has_member(&doc, OTHER_TEAM, BOB));
    }

    #[test]
    fn group_has_member_ignores_malformed_membership_triples() {
        // A literal member and a foreign predicate are both ignored; the valid triple still matches.
        let doc = parse(&format!(
            r#"@prefix vcard: <http://www.w3.org/2006/vcard/ns#>.
            @prefix foaf: <http://xmlns.com/foaf/0.1/>.
            <{TEAM}> vcard:hasMember "{BOB}".
            <{TEAM}> foaf:member <{ALICE}>.
            <{TEAM}> vcard:hasMember <{BOB}>."#
        ));
        assert!(group_has_member(&doc, TEAM, BOB));
        assert!(
            !group_has_member(&doc, TEAM, ALICE),
            "a non-vcard:hasMember predicate must not confer membership"
        );
    }

    // --- acl:origin (app-scoping) -------------------------------------------------------------

    const APP: &str = "https://app.example";
    const OTHER_APP: &str = "https://evil.example";

    fn origin_restricted_acl() -> Vec<Triple> {
        // Bob is granted Read on RES, but ONLY from the app at https://app.example.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#bob> a acl:Authorization;
                   acl:agent <{BOB}>;
                   acl:origin <{APP}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        parse(&ttl)
    }

    #[test]
    fn origin_restricted_rule_grants_only_from_matching_origin() {
        let t = origin_restricted_acl();
        // Matching Origin → granted.
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(APP), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
    }

    #[test]
    fn origin_restricted_rule_denies_from_other_origin() {
        let t = origin_restricted_acl();
        // A DIFFERENT Origin → not granted (the over-grant the HIGH finding is about).
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(OTHER_APP), AclScope::AccessTo).is_empty()
        );
    }

    #[test]
    fn origin_restricted_rule_denies_when_origin_absent() {
        let t = origin_restricted_acl();
        // No Origin header at all → an origin-restricted rule must NOT match (fail-closed): a
        // non-browser/server-to-server caller cannot bypass an app restriction by omitting Origin.
        assert!(modes_with_origin(&t, RES, Some(BOB), None, AclScope::AccessTo).is_empty());
        // The legacy `modes` helper (no origin) must likewise deny an origin-restricted rule.
        assert!(modes(&t, RES, Some(BOB), AclScope::AccessTo).is_empty());
    }

    #[test]
    fn rule_without_acl_origin_grants_from_any_origin() {
        // The common case: a rule with NO acl:origin applies regardless of the request Origin.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#bob> a acl:Authorization;
                   acl:agent <{BOB}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        // From a specific Origin.
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(APP), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
        // From a different Origin.
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(OTHER_APP), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
        // With no Origin at all.
        assert!(
            modes_with_origin(&t, RES, Some(BOB), None, AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
    }

    #[test]
    fn multiple_acl_origin_values_any_one_matches() {
        // A rule listing several acl:origin values grants from ANY of them, denies from outside the set.
        let app2 = "https://app2.example";
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            <#bob> a acl:Authorization;
                   acl:agent <{BOB}>;
                   acl:origin <{APP}>, <{app2}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(APP), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(app2), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
        assert!(
            modes_with_origin(&t, RES, Some(BOB), Some(OTHER_APP), AclScope::AccessTo).is_empty()
        );
    }

    #[test]
    fn origin_restriction_also_applies_to_public_class_rules() {
        // An `acl:agentClass foaf:Agent` (public) rule that ALSO carries acl:origin is app-scoped: it
        // grants the public ONLY from the trusted origin, never from another origin or no Origin.
        let ttl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
            @prefix foaf: <http://xmlns.com/foaf/0.1/>.
            <#pub> a acl:Authorization;
                   acl:agentClass foaf:Agent;
                   acl:origin <{APP}>;
                   acl:accessTo <{RES}>;
                   acl:mode acl:Read."#
        );
        let t = parse(&ttl);
        assert!(
            modes_with_origin(&t, RES, None, Some(APP), AclScope::AccessTo)
                .contains(&AccessMode::Read)
        );
        assert!(modes_with_origin(&t, RES, None, Some(OTHER_APP), AclScope::AccessTo).is_empty());
        assert!(modes_with_origin(&t, RES, None, None, AclScope::AccessTo).is_empty());
    }
}

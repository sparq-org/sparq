//! sq-gwayp — pins the container-`default` walk contract of
//! [`PodStore::resolve_acl`] against the depth-4 `wac_fixture()` pod, through the PUBLIC
//! API only (read-only w.r.t. `src/`):
//!
//! 1. the target's OWN `.acl` (`acl:accessTo`) fully overrides any ancestor;
//! 2. with no own ACL, the NEAREST ancestor container with an ACL governs via
//!    [`AclScope::Default`] and the walk STOPS there — no union across levels
//!    (child → root);
//! 3. no ACL anywhere up to the storage root ⇒ deny (fail-closed).
//!
//! Anti-vacuity witness: [`inheritance_depth_witness_at_least_two_levels`] asserts the
//! fixture actually EXERCISES ≥2-level inheritance — a resource whose governing ACL is
//! the ROOT `.acl` with an intermediate container that has NO ACL of its own — so this
//! suite cannot pass on a degenerate one-level (own-parent-only) fixture.

use std::sync::OnceLock;

use sparq_core::Graph;
use sparq_solid::fixture::{ALICE, BOB, CAROL, DAVE, POD, TEAM_GROUP_DOC};
use sparq_solid::{wac_fixture, AclScope, AclStatus, Mode, PodStore, Session};

/// One materialized store shared by every test in this binary (materializing the
/// ~1.1k-graph fixture is the expensive step; `decide`/`resolve_acl` are `&self`).
fn store() -> &'static PodStore {
    static STORE: OnceLock<PodStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
        let mut s = PodStore::new(g);
        let stats = s.materialize_wac().expect("wac materializes");
        assert!(stats.auth_triples > 0, "auth view non-empty");
        s
    })
}

fn session(agent: Option<&str>) -> Session<'_> {
    Session {
        agent,
        client: None,
        issuer: None,
        now: None,
    }
}

/// The governing `.acl` IRI for `resource`, per the public walk.
fn governing(resource: &str) -> (String, AclScope) {
    let eff = store()
        .resolve_acl(resource)
        .unwrap_or_else(|| panic!("{resource}: no governing ACL"));
    (eff.acl.as_str().to_owned(), eff.scope)
}

/// Solid slash-semantics parent (mirrors the loader's walk; re-derived here from the
/// PUBLIC contract so the test stays independent of `src/` internals).
fn parent(iri: &str) -> Option<&str> {
    let host_end = iri
        .find("://")
        .map(|i| i + 3)
        .and_then(|s| iri[s..].find('/').map(|j| s + j))?;
    let trimmed = iri.strip_suffix('/').unwrap_or(iri);
    if trimmed.len() <= host_end {
        return None;
    }
    let cut = trimmed.rfind('/')?;
    (cut >= host_end).then(|| &iri[..cut + 1])
}

// ─── (1) the target's OWN .acl fully overrides any ancestor ───────────────────────────

#[test]
fn own_acl_wins_with_access_to_scope_and_fully_overrides_ancestors() {
    // mixed4/c1/g0/d0.ttl is the fixture's ONE resource-specific ACL (bob-only, no
    // acl:default). BOTH mixed4/ and the root carry owner (alice) ACLs above it.
    let doc = format!("{POD}mixed4/c1/g0/d0.ttl");
    let (acl, scope) = governing(&doc);
    assert_eq!(
        acl,
        format!("{doc}.acl"),
        "own .acl governs, not any ancestor"
    );
    assert_eq!(scope, AclScope::AccessTo, "own .acl ⇒ accessTo scope");

    // FULL override — no union with ancestors: alice holds Read/Write/Control via the
    // root AND mixed4/ defaults, yet the own ACL (which grants her nothing) is the
    // ONLY document consulted. Bob (its sole agent) reads; alice is denied.
    let s = store();
    let bob = s.decide(&session(Some(BOB)), &doc, Mode::Read);
    assert!(bob.allow && bob.status == AclStatus::Resolved);
    assert_eq!(bob.scope, Some(AclScope::AccessTo));
    let alice = s.decide(&session(Some(ALICE)), &doc, Mode::Read);
    assert!(
        !alice.allow,
        "ancestor owner grant must NOT union into the own-ACL verdict"
    );
    assert_eq!(
        alice.status,
        AclStatus::Resolved,
        "authoritative deny, not transient"
    );

    // Contrast: the UN-overridden sibling inherits mixed4/'s default — alice reads it.
    let sibling = format!("{POD}mixed4/c1/g0/d1.ttl");
    let (sib_acl, sib_scope) = governing(&sibling);
    assert_eq!(sib_acl, format!("{POD}mixed4/.acl"));
    assert_eq!(sib_scope, AclScope::Default);
    assert!(s.decide(&session(Some(ALICE)), &sibling, Mode::Read).allow);
}

// ─── (2) nearest ancestor governs via Default; the walk STOPS there (no union) ────────

#[test]
fn nearest_ancestor_governs_by_default_and_walk_stops_there() {
    // team2/c3/ has no restated ACL, so team2/c3/g0/d0.ttl is governed by team2/.acl —
    // the NEAREST ancestor ACL — not by the root's, which also matches the prefix.
    let doc = format!("{POD}team2/c3/g0/d0.ttl");
    let (acl, scope) = governing(&doc);
    assert_eq!(
        acl,
        format!("{POD}team2/.acl"),
        "nearest ancestor wins over the root"
    );
    assert_eq!(scope, AclScope::Default, "inherited ⇒ acl:default scope");

    // Every ACL-less container on the way up resolves to the SAME nearest ancestor.
    for c in [format!("{POD}team2/c3/g0/"), format!("{POD}team2/c3/")] {
        let (acl, scope) = governing(&c);
        assert_eq!(
            acl,
            format!("{POD}team2/.acl"),
            "{c}: same nearest ancestor"
        );
        assert_eq!(scope, AclScope::Default);
    }

    // NO UNION ACROSS LEVELS: the root .acl grants alice Read/Write/Control with
    // acl:default over the whole pod, but team2/.acl (group-only, no alice) shadows it —
    // if the walk unioned root+team2 grants, alice would be allowed here.
    let s = store();
    let alice = s.decide(&session(Some(ALICE)), &doc, Mode::Read);
    assert!(
        !alice.allow,
        "root owner grant must NOT leak past the nearer team2/.acl"
    );
    assert_eq!(alice.status, AclStatus::Resolved);
    // …while the governing group grant works exactly as written (Read+Write, not Control).
    for member in [BOB, CAROL] {
        assert!(s.decide(&session(Some(member)), &doc, Mode::Read).allow);
        assert!(s.decide(&session(Some(member)), &doc, Mode::Write).allow);
        assert!(!s.decide(&session(Some(member)), &doc, Mode::Control).allow);
    }
    assert!(
        !s.decide(&session(Some(DAVE)), &doc, Mode::Read).allow,
        "non-member denied"
    );
}

// ─── (3) no ACL anywhere up to the storage root ⇒ deny (fail-closed) ──────────────────

#[test]
fn no_acl_anywhere_up_the_chain_is_a_fail_closed_deny() {
    // A resource on an authority with NO control documents at all: the walk reaches the
    // storage root without finding one ⇒ resolve_acl is None and decide denies (NoAcl),
    // even for the pod owner, even with the auth view fully materialized.
    let s = store();
    for resource in ["https://other.ex/notes/deep/n1", "https://other.ex/x"] {
        assert!(
            s.resolve_acl(resource).is_none(),
            "{resource}: no governing ACL exists"
        );
        let d = s.decide(&session(Some(ALICE)), resource, Mode::Read);
        assert!(
            !d.allow,
            "{resource}: absence of any ACL must deny, never allow"
        );
        assert_eq!(d.status, AclStatus::NoAcl);
        assert_eq!(d.governing_acl, None);
        assert!(!s.decide(&session(None), resource, Mode::Read).allow);
    }
}

// ─── anti-vacuity witness: the fixture exercises ≥2-level inheritance ─────────────────

/// Walk `resource`'s parent chain (public slash semantics) and return
/// `(steps_to_governing_container, intermediate_containers)` for its governing ACL.
fn walk_depth(resource: &str) -> (usize, Vec<String>) {
    let (acl, _) = governing(resource);
    let container = acl
        .strip_suffix(".acl")
        .expect("WAC fixture governs via .acl documents");
    assert_ne!(
        container, resource,
        "witness requires an INHERITED (non-own) ACL"
    );
    let mut steps = 0;
    let mut intermediates = Vec::new();
    let mut cur = resource;
    loop {
        let p = parent(cur).unwrap_or_else(|| {
            panic!("{resource}: walked past the root without meeting {container}")
        });
        steps += 1;
        if p == container {
            return (steps, intermediates);
        }
        intermediates.push(p.to_owned());
        cur = p;
    }
}

#[test]
fn inheritance_depth_witness_at_least_two_levels() {
    // The group document lives under groups/ — a container with NO ACL of its own — so
    // its governing ACL is the ROOT's, TWO levels up. This is the bead's anti-vacuity
    // witness: it FAILS if the fixture is flattened so every resource is governed by
    // its immediate parent (or itself).
    let (acl, scope) = governing(TEAM_GROUP_DOC);
    assert_eq!(
        acl,
        format!("{POD}.acl"),
        "governing ACL is the storage ROOT's"
    );
    assert_eq!(scope, AclScope::Default);

    let (depth, intermediates) = walk_depth(TEAM_GROUP_DOC);
    assert!(
        depth >= 2,
        "inheritance depth {depth} < 2 — fixture degenerated to one level"
    );
    assert!(
        !intermediates.is_empty(),
        "no intermediate container between resource and root"
    );

    // Each intermediate genuinely has NO own ACL: if one did, IT would govern itself
    // with accessTo scope; instead each resolves to the same root ACL by default.
    for c in &intermediates {
        let (c_acl, c_scope) = governing(c);
        assert_eq!(
            c_acl,
            format!("{POD}.acl"),
            "{c}: must not have its own ACL"
        );
        assert_eq!(
            c_scope,
            AclScope::Default,
            "{c}: an own ACL would resolve as accessTo"
        );
    }

    // And the root ACL genuinely GOVERNS across that gap: the owner reads the group doc
    // via the root acl:default; an agent without a root grant is denied.
    let s = store();
    assert!(
        s.decide(&session(Some(ALICE)), TEAM_GROUP_DOC, Mode::Read)
            .allow
    );
    assert!(
        !s.decide(&session(Some(DAVE)), TEAM_GROUP_DOC, Mode::Read)
            .allow
    );

    // A second, DEEPER witness: priv0/ itself has NO own ACL (its owner-only semantics
    // ARE the root default), and priv0/c4/ has no restated ACL either — so
    // priv0/c4/g0/d0.ttl walks FOUR levels (g0/ → c4/ → priv0/ → root) to its governing
    // ACL, crossing three ACL-less intermediates.
    let deep = format!("{POD}priv0/c4/g0/d0.ttl");
    let (deep_acl, deep_scope) = governing(&deep);
    assert_eq!(deep_acl, format!("{POD}.acl"));
    assert_eq!(deep_scope, AclScope::Default);
    let (deep_depth, deep_intermediates) = walk_depth(&deep);
    assert!(
        deep_depth >= 3,
        "expected a ≥3-level walk, got {deep_depth}"
    );
    for c in &deep_intermediates {
        let (c_acl, c_scope) = governing(c);
        assert_eq!(
            c_acl,
            format!("{POD}.acl"),
            "{c}: intermediate must be ACL-less"
        );
        assert_eq!(c_scope, AclScope::Default);
    }
    // …and the root default genuinely reaches across the gap (fixture case 1: owner
    // access inherited at depth 4; a restated deeper ACL like priv0/c0/ would instead
    // govern its own subtree — cross-checked by the nearest-ancestor test above).
    assert!(s.decide(&session(Some(ALICE)), &deep, Mode::Read).allow);
    assert!(!s.decide(&session(Some(BOB)), &deep, Mode::Read).allow);
}

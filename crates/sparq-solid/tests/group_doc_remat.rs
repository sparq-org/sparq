//! issue #55 — a permitted write to a GROUP DOCUMENT auto-re-materializes.
//!
//! `PodStore::update_as` already re-materializes the auth view after a permitted write to
//! an `.acl`/`.acr` document, and after any `CLEAR`/`DROP ALL` update. A `GRAPH ?g` update
//! re-materializes only when a resolved target is an auth-view input (issue #3384).
//! The remaining hole was the *statically targeted* write to a WAC **group document** — a
//! graph named by some authorization's `acl:agentGroup`. Such a document has no naming
//! convention (`https://pod.ex/groups` looks like any other resource), so the write path
//! could not recognize it, and a `vcard:hasMember` triple removed through `update_as` left
//! the materialized view granting access to an agent who was no longer a member until
//! something else forced a re-materialization.
//!
//! These tests drive the REAL path end to end — the fixture's own `.acl`, the real
//! `update_as` enforcement, and the real read oracle — with NO explicit `materialize_wac`
//! call after the group write. Both directions are covered, because a trigger that only
//! ever grants (or only ever revokes) would be half a fix:
//!
//! - **revoke** (the fail-open-critical direction): removing a membership must drop the
//!   grant on the very next read;
//! - **grant**: adding a membership must be visible on the very next read;
//! - **grants unchanged**: a write to an ordinary content resource changes nobody's
//!   grants (this one pins the observable contract only — it would also hold under a
//!   "re-materialize on every write" implementation, so it is not evidence of scoping);
//! - **still enforced**: the new trigger is not an authorization bypass — a group
//!   document remains an ordinary resource governed by its own ACL, so an agent with
//!   Read only still cannot write it.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// The group document — deliberately NOT `.acl`/`.acr`-suffixed, so it is recognizable
/// only by being referenced from the ACL via `acl:agentGroup`.
const GROUP_DOC: &str = "https://pod.ex/groups";
const GROUP: &str = "https://pod.ex/groups#team";
const NOTE: &str = "https://pod.ex/notes/n1";

const HAS_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";

fn sess(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

/// A pod whose root `.acl` gives ADMIN Read+Write+Control over everything (the root and its
/// own `.acl` included) and gives the members of `<https://pod.ex/groups#team>` Read — with
/// ALICE the sole initial member.
fn store() -> PodStore {
    let nq = concat!(
        // Content.
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n",
        "<https://pod.ex/other#it> <https://ex.dev/ns#title> \"other\" <https://pod.ex/other> .\n",
        // The group document (an ORDINARY resource IRI — no .acl/.acr suffix).
        "<https://pod.ex/groups#team> <http://www.w3.org/2006/vcard/ns#hasMember> \
            <https://alice.ex/card#me> <https://pod.ex/groups> .\n",
        // Root ACL: admin owns the pod.
        "<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> \
            <https://admin.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .\n",
        // Root ACL: the team group reads the pod.
        "<https://pod.ex/.acl#team> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> \
            <https://pod.ex/groups#team> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
    );
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    s.materialize_wac().expect("wac materializes");
    s
}

/// Whether `agent` may read `NOTE`, asked through BOTH the session oracle and the real
/// query path (a stale grant that survives in either one is a fail-open).
fn can_read_note(store: &PodStore, agent: &str) -> bool {
    let s = sess(agent);
    let via_oracle = store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|n| n.as_str() == NOTE);
    let via_query = store
        .ask_as(&s, Mode::Read, &format!("ASK {{ GRAPH <{}> {{ ?s ?p ?o }} }}", NOTE))
        .expect("ask evaluates");
    assert_eq!(via_oracle, via_query, "oracle and query path must agree for <{}>", agent);
    via_oracle
}

fn member_triple(agent: &str) -> String {
    format!(
        "GRAPH <{}> {{ <{}> <{}> <{}> }}",
        GROUP_DOC, GROUP, HAS_MEMBER, agent
    )
}

#[test]
fn group_doc_membership_revoke_rematerializes() {
    let mut s = store();
    assert!(can_read_note(&s, ALICE), "alice starts with a group-derived read grant");

    // ADMIN removes alice's membership through the enforced write path. NOTE: no
    // `materialize_wac()` call follows — the write path must trigger it.
    s.update_as(&sess(ADMIN), &format!("DELETE DATA {{ {} }}", member_triple(ALICE)))
        .expect("admin may write the group document");

    assert!(
        !can_read_note(&s, ALICE),
        "alice's grant must be gone on the very next read: the group-document write did \
         not re-materialize the auth view (stale grant = fail-open)"
    );
}

#[test]
fn group_doc_membership_grant_rematerializes() {
    let mut s = store();
    assert!(!can_read_note(&s, BOB), "bob starts with no grant");

    s.update_as(&sess(ADMIN), &format!("INSERT DATA {{ {} }}", member_triple(BOB)))
        .expect("admin may write the group document");

    assert!(
        can_read_note(&s, BOB),
        "bob's new membership must be visible on the very next read"
    );
}

#[test]
fn ordinary_resource_write_leaves_grants_unchanged() {
    // Negative control: the trigger is scoped to auth-view INPUTS. Writing a resource that
    // no authorization references as a group document changes nobody's grants — so this
    // test would still pass under a "re-materialize on every write" implementation, and it
    // is here to pin the observable contract (grants unchanged), not the trigger count.
    let mut s = store();
    let before: Vec<String> =
        s.accessible(&sess(ALICE), Mode::Read).iter().map(|n| n.as_str().to_owned()).collect();

    s.update_as(
        &sess(ADMIN),
        "INSERT DATA { GRAPH <https://pod.ex/other> { \
            <https://pod.ex/other#it> <https://ex.dev/ns#tag> \"x\" } }",
    )
    .expect("admin may write an ordinary resource");

    let after: Vec<String> =
        s.accessible(&sess(ALICE), Mode::Read).iter().map(|n| n.as_str().to_owned()).collect();
    assert_eq!(before, after, "an ordinary content write must not change any grant");
    assert!(can_read_note(&s, ALICE), "alice keeps her group-derived grant");
}

#[test]
fn group_doc_write_still_requires_write_permission() {
    // The re-materialization trigger must not become an authorization bypass: a group
    // document is an ordinary resource governed by its own ACL, and ALICE holds Read only.
    let mut s = store();
    let err = s
        .update_as(&sess(ALICE), &format!("DELETE DATA {{ {} }}", member_triple(ALICE)))
        .expect_err("alice has Read only on the group document");
    assert!(err.contains("update denied"), "unexpected error: {}", err);
    assert!(
        can_read_note(&s, ALICE),
        "the denied write left the store — and so alice's grant — untouched"
    );
}

#[test]
fn ordinary_resource_write_does_not_rebuild_the_auth_view() {
    // The trigger-count half of the negative control above: a static write to a graph that
    // is neither an `.acl`/`.acr` nor a referenced group document leaves the view alone.
    let mut s = store();
    let gen = s.auth_generation();
    s.update_as(
        &sess(ADMIN),
        "INSERT DATA { GRAPH <https://pod.ex/other> { \
            <https://pod.ex/other#it> <https://ex.dev/ns#tag> \"x\" } }",
    )
    .expect("admin may write an ordinary resource");
    assert_eq!(
        s.auth_generation(),
        gen,
        "a content-only write must not rebuild the view"
    );
}

#[test]
fn variable_graph_content_write_does_not_rebuild_the_auth_view() {
    // issue #3384: a `GRAPH ?g` update whose resolved targets are all plain content graphs
    // is authorized per resolved graph and does not re-materialize.
    let mut s = store();
    let gen = s.auth_generation();
    s.update_as(
        &sess(ADMIN),
        "DELETE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } } \
         INSERT { GRAPH ?g { ?s <https://ex.dev/ns#title> \"renamed\" } } \
         WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }",
    )
    .expect("admin may rewrite content graphs");
    let renamed = s
        .ask_as(
            &sess(ADMIN),
            Mode::Read,
            "ASK { GRAPH ?g { ?s <https://ex.dev/ns#title> \"renamed\" } }",
        )
        .expect("ask evaluates");
    assert!(renamed, "the update must have rewritten the content graphs");
    assert_eq!(
        s.auth_generation(),
        gen,
        "content-only GRAPH ?g targets must not rebuild the view"
    );
    assert!(
        can_read_note(&s, ALICE),
        "alice keeps her group-derived grant"
    );
}

#[test]
fn variable_graph_group_doc_write_rebuilds_the_auth_view() {
    // The same `GRAPH ?g` shape resolving to the referenced group document must still
    // re-materialize: the revoke is visible on the very next read.
    let mut s = store();
    let gen = s.auth_generation();
    s.update_as(
        &sess(ADMIN),
        &format!(
            "DELETE {{ GRAPH ?g {{ ?grp <{HAS_MEMBER}> <{ALICE}> }} }} \
             WHERE {{ GRAPH ?g {{ ?grp <{HAS_MEMBER}> <{ALICE}> }} }}"
        ),
    )
    .expect("admin may write the group document");
    assert!(
        s.auth_generation() > gen,
        "a GRAPH ?g write to a group document must rebuild the view"
    );
    assert!(
        !can_read_note(&s, ALICE),
        "alice's grant must be gone on the very next read"
    );
}

#[test]
fn variable_graph_acl_write_rebuilds_the_auth_view() {
    // And resolving to an `.acl` document: dropping the team's Read mode revokes alice.
    // `?g` is bound by `VALUES`, so the WHERE reads no graph.
    let mut s = store();
    let gen = s.auth_generation();
    s.update_as(
        &sess(ADMIN),
        "DELETE { GRAPH ?g { <https://pod.ex/.acl#team> \
            <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> } } \
         WHERE { VALUES ?g { <https://pod.ex/.acl> } }",
    )
    .expect("admin (Control) may write the .acl");
    assert!(
        s.auth_generation() > gen,
        "a GRAPH ?g write to an .acl must rebuild the view"
    );
    assert!(
        !can_read_note(&s, ALICE),
        "alice's grant must be gone on the very next read"
    );
}

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";

/// An ACP pod: the root `.acr` gives ADMIN Read, Write and Control on the root's members,
/// `<https://pod.ex/x>` has its own `.acr`, and `extra` adds more `(graph, s, p, o)` ACP
/// quads.
fn acp_store(extra: &[(&str, &str, &str, String)]) -> PodStore {
    let root = "https://pod.ex/.acr";
    let mut nq = String::from(
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n\
         <https://pod.ex/x#it> <https://ex.dev/ns#title> \"x\" <https://pod.ex/x> .\n",
    );
    let mut quads: Vec<(&str, &str, &str, String)> = vec![
        (root, root, "resource", "https://pod.ex/".to_owned()),
        (root, root, "accessControl", "https://pod.ex/.acr#c".to_owned()),
        (root, root, "memberAccessControl", "https://pod.ex/.acr#c".to_owned()),
        (root, "https://pod.ex/.acr#c", "apply", "https://pod.ex/.acr#p".to_owned()),
        (root, "https://pod.ex/.acr#p", "allow", format!("{ACL}Read")),
        (root, "https://pod.ex/.acr#p", "allow", format!("{ACL}Write")),
        (root, "https://pod.ex/.acr#p", "allow", format!("{ACL}Control")),
        (root, "https://pod.ex/.acr#p", "anyOf", "https://pod.ex/.acr#m".to_owned()),
        (root, "https://pod.ex/.acr#m", "agent", ADMIN.to_owned()),
        ("https://pod.ex/x.acr", "https://pod.ex/x.acr", "resource", "https://pod.ex/x".to_owned()),
    ];
    quads.extend(extra.iter().cloned());
    for (g, s_, p, o) in quads {
        nq += &format!("<{s_}> <{ACP}{p}> <{o}> <{g}> .\n");
    }
    let mut s = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("fixture loads"));
    s.materialize_acp().expect("acp materializes");
    s
}

/// A `GRAPH ?g` update that grants BOB Read on `<https://pod.ex/x>` through its `.acr`.
const GRANT_BOB_X: &str = "INSERT { GRAPH ?g { \
    <https://pod.ex/x.acr> <http://www.w3.org/ns/solid/acp#accessControl> <https://pod.ex/x.acr#b> . \
    <https://pod.ex/x.acr#b> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/x.acr#bp> . \
    <https://pod.ex/x.acr#bp> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> . \
    <https://pod.ex/x.acr#bp> <http://www.w3.org/ns/solid/acp#anyOf> <https://pod.ex/x.acr#bm> . \
    <https://pod.ex/x.acr#bm> <http://www.w3.org/ns/solid/acp#agent> <https://bob.ex/card#me> } } \
    WHERE { VALUES ?g { <https://pod.ex/x.acr> } }";

fn bob_reads_x(s: &PodStore) -> bool {
    s.accessible(&sess(BOB), Mode::Read).iter().any(|g| g.as_str() == "https://pod.ex/x")
}

#[test]
fn variable_graph_acr_write_without_a_write_grant_is_refused() {
    // An `.acr` is an auth-view input by its suffix, like an `.acl`. Here no policy grants
    // Write on `<https://pod.ex/x.acr>` itself (ACP has no WAC-style Control ⇒ write on
    // the control document), so even ADMIN is refused and nothing is rebuilt.
    let mut s = acp_store(&[]);
    let gen = s.auth_generation();
    assert!(s.update_as_acp(&sess(ADMIN), GRANT_BOB_X).is_err(), "no Write grant on the .acr");
    assert_eq!(s.auth_generation(), gen, "a refused update rebuilds nothing");
    assert!(!bob_reads_x(&s));
}

#[test]
fn variable_graph_acr_write_rebuilds_the_auth_view() {
    // The `.acr` of `<https://pod.ex/x.acr>` (`x.acr.acr`) grants ADMIN Write on it, so a
    // `GRAPH ?g` write there succeeds, rebuilds the view, and its grant is in force on the
    // very next read.
    let meta = "https://pod.ex/x.acr.acr";
    let mut s = acp_store(&[
        (meta, meta, "resource", "https://pod.ex/x.acr".to_owned()),
        (meta, meta, "accessControl", format!("{meta}#c")),
        (meta, &format!("{meta}#c"), "apply", format!("{meta}#p")),
        (meta, &format!("{meta}#p"), "allow", format!("{ACL}Write")),
        (meta, &format!("{meta}#p"), "anyOf", format!("{meta}#m")),
        (meta, &format!("{meta}#m"), "agent", ADMIN.to_owned()),
    ]);
    assert!(!bob_reads_x(&s), "bob starts with no grant on x");
    let gen = s.auth_generation();
    s.update_as_acp(&sess(ADMIN), GRANT_BOB_X).expect("admin holds Write on the .acr");
    assert!(s.auth_generation() > gen, "a GRAPH ?g write to an .acr must rebuild the view");
    assert!(bob_reads_x(&s), "the new policy is in force on the very next read");
}

#[test]
fn mixed_content_and_acl_update_rebuilds_the_auth_view() {
    // One successful update that changes a content graph AND the `.acl`: the content
    // change lands and the rule change is in force on the very next read.
    let mut s = store();
    let gen = s.auth_generation();
    s.update_as(
        &sess(ADMIN),
        "DELETE { GRAPH ?acl { <https://pod.ex/.acl#team> \
            <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> } } \
         INSERT { GRAPH ?doc { <https://pod.ex/other#it> <https://ex.dev/ns#tag> \"y\" } } \
         WHERE { VALUES (?acl ?doc) { (<https://pod.ex/.acl> <https://pod.ex/other>) } }",
    )
    .expect("admin may write both graphs");
    assert!(s.auth_generation() > gen, "a mixed update touching an .acl must rebuild the view");
    let tagged = s
        .ask_as(
            &sess(ADMIN),
            Mode::Read,
            "ASK { GRAPH <https://pod.ex/other> { ?s <https://ex.dev/ns#tag> \"y\" } }",
        )
        .expect("ask evaluates");
    assert!(tagged, "the content half of the update landed");
    assert!(!can_read_note(&s, ALICE), "alice's grant must be gone on the very next read");
}

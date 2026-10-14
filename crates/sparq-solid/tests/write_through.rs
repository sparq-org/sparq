//! issue #992 FR-3 (sq-snopa.5): the AUTHORITATIVE ACL write-through.
//!
//! Exercises the REAL path — `PodStore::put_acl`/`delete_acl` (WAC) and
//! `put_acl_acp`/`delete_acl_acp` (ACP) over the live materializer + session oracle — for
//! the load-bearing invariants:
//!
//! - **write-through takes effect immediately**: a `put_acl` granting access makes the
//!   grant visible through the SAME enforcement path (`accessible`) with no separate
//!   `materialize_*` call — the auth view is rebuilt atomically by the call;
//! - **delete narrows**: a `delete_acl` removing the governing ACL revokes the grant;
//! - **source-of-truth / pure-function**: after a put-then-delete the auth view equals what
//!   a fresh load + materialize of the SAME final `.acl` graphs produces (no stale grant
//!   survives a rule change);
//! - **atomic rollback / fail-closed**: a `put_acl` whose new ACL is rejected at
//!   materialization (a reserved-encoding principal) leaves the store EXACTLY as it was —
//!   the prior content and the prior auth view both intact, and `Err` returned;
//! - **input validation**: a non-`.acl`/`.acr` target IRI and malformed content are both
//!   rejected before any mutation.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

fn sess(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// A root `.acl` (N-Triples body, no graph column — `put_acl` parses the document body)
/// granting `agent` `Read` on the whole pod by `acl:default`.
fn root_acl_read(agent: &str) -> String {
    format!(
        "<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <{agent}> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .\n"
    )
}

/// One content document under /notes/, no ACL — every session fails closed until one is PUT.
fn empty_pod() -> PodStore {
    let nq = "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hi\" <https://pod.ex/notes/n1> .\n";
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    store
}

/// How many triples the named graph `name` holds in the store (0 if absent).
fn graph_len(store: &PodStore, name: &str) -> usize {
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(name));
    store
        .graph
        .named
        .iter()
        .find(|(n, _)| *n == term)
        .map(|(_, g)| {
            let pat: sparq_core::store::Pattern = [None, None, None];
            g.store.scan(&pat).rows.len()
        })
        .unwrap_or(0)
}

// --- put_acl: create + immediate effect ------------------------------------------------

#[test]
fn put_acl_creates_and_takes_effect_immediately() {
    let mut store = empty_pod();
    let alice = sess(ALICE);
    assert_eq!(store.accessible(&alice, Mode::Read).len(), 0, "no grant before PUT");

    let out = store
        .put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples")
        .expect("put_acl succeeds");
    assert!(!out.existed, "a fresh .acl is CREATED, not replaced");
    assert_eq!(out.acl.as_str(), "https://pod.ex/.acl");
    assert!(out.triples >= 4, "the four authorization triples are stored");

    // No separate materialize_wac() — the write-through rebuilt the view atomically.
    assert_eq!(
        store.accessible(&alice, Mode::Read).len(),
        2,
        "alice now reads n1 + the notes/ container"
    );
}

// --- put_acl: replace swaps the grant wholesale ----------------------------------------

#[test]
fn put_acl_replaces_rules_wholesale() {
    let mut store = empty_pod();
    store
        .put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples")
        .expect("first put");
    assert_eq!(store.accessible(&sess(ALICE), Mode::Read).len(), 2);
    assert_eq!(store.accessible(&sess(BOB), Mode::Read).len(), 0);

    // PUT a NEW body that grants BOB instead of alice — the old rules are gone.
    let out = store
        .put_acl("https://pod.ex/.acl", &root_acl_read(BOB), "ntriples")
        .expect("replace");
    assert!(out.existed, "the .acl already existed → replaced");
    assert_eq!(store.accessible(&sess(BOB), Mode::Read).len(), 2, "bob now reads");
    assert_eq!(
        store.accessible(&sess(ALICE), Mode::Read).len(),
        0,
        "alice's grant was REPLACED, not merged"
    );
}

// --- delete_acl: narrows access --------------------------------------------------------

#[test]
fn delete_acl_revokes_and_rebuilds_atomically() {
    let mut store = empty_pod();
    store
        .put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples")
        .expect("put");
    assert_eq!(store.accessible(&sess(ALICE), Mode::Read).len(), 2);

    let out = store.delete_acl("https://pod.ex/.acl").expect("delete");
    assert!(out.existed);
    assert_eq!(out.triples, 0);
    assert_eq!(graph_len(&store, "https://pod.ex/.acl"), 0, "the .acl graph is gone");
    assert_eq!(
        store.accessible(&sess(ALICE), Mode::Read).len(),
        0,
        "un-protected ⇒ every session fails closed"
    );
}

#[test]
fn delete_absent_acl_is_noop_success() {
    let mut store = empty_pod();
    let out = store
        .delete_acl("https://pod.ex/missing.acl")
        .expect("deleting an absent ACL is a no-op success");
    assert!(!out.existed, "nothing was there");
}

// --- source-of-truth: the auth view is a PURE FUNCTION of the present .acl graphs -------

#[test]
fn auth_view_is_pure_function_of_final_acl_state() {
    // Path A: empty pod → PUT alice's ACL → PUT bob's ACL → DELETE → PUT alice again.
    let mut a = empty_pod();
    a.put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples").unwrap();
    a.put_acl("https://pod.ex/.acl", &root_acl_read(BOB), "ntriples").unwrap();
    a.delete_acl("https://pod.ex/.acl").unwrap();
    a.put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples").unwrap();

    // Path B: load the SAME final dataset (n1 content + alice's .acl) from scratch.
    let final_nq = format!(
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hi\" <https://pod.ex/notes/n1> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <{ALICE}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n"
    );
    let mut b = PodStore::new(Graph::load_dataset(&final_nq, "nquads").unwrap());
    b.materialize_wac().unwrap();

    // The two stores authorize IDENTICALLY — no stale grant survived the put/delete churn.
    for s in [sess(ALICE), sess(BOB), Session::default()] {
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            assert_eq!(
                a.accessible(&s, mode).len(),
                b.accessible(&s, mode).len(),
                "write-through churn must converge to the same auth view as a fresh load ({:?}, {:?})",
                s.agent,
                mode
            );
        }
    }
}

// --- atomic rollback: a rejected new ACL leaves the store EXACTLY as it was -------------

#[test]
fn put_acl_rolls_back_on_reserved_principal() {
    let mut store = empty_pod();
    // Establish a known-good grant first.
    store
        .put_acl("https://pod.ex/.acl", &root_acl_read(ALICE), "ntriples")
        .expect("good put");
    assert_eq!(store.accessible(&sess(ALICE), Mode::Read).len(), 2);
    let prior_acl_len = graph_len(&store, "https://pod.ex/.acl");

    // Now PUT a body naming a RESERVED-encoding agent — materialization REJECTS it.
    let bad = "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:evil> .\n";
    let err = store
        .put_acl("https://pod.ex/.acl", bad, "ntriples")
        .expect_err("reserved-encoding principal must be rejected");
    assert!(err.contains("urn:sparq:") || err.to_lowercase().contains("reserved"), "{err}");

    // Atomic: the PRIOR content and the PRIOR auth view are both intact.
    assert_eq!(
        graph_len(&store, "https://pod.ex/.acl"),
        prior_acl_len,
        "the rejected content was rolled back — prior .acl restored"
    );
    assert_eq!(
        store.accessible(&sess(ALICE), Mode::Read).len(),
        2,
        "alice's prior grant still stands after the failed put (no partial write)"
    );
}

// --- input validation ------------------------------------------------------------------

#[test]
fn put_acl_rejects_non_control_iri() {
    let mut store = empty_pod();
    let err = store
        .put_acl("https://pod.ex/notes/n1", "", "ntriples")
        .expect_err("a content graph IRI is not an ACL target");
    assert!(err.contains("access-control document"), "{err}");
    // The content graph was NOT touched.
    assert_eq!(graph_len(&store, "https://pod.ex/notes/n1"), 1);
}

#[test]
fn delete_acl_rejects_non_control_iri() {
    let mut store = empty_pod();
    let err = store
        .delete_acl("https://pod.ex/notes/n1")
        .expect_err("a content graph IRI is not an ACL target");
    assert!(err.contains("access-control document"), "{err}");
    assert_eq!(graph_len(&store, "https://pod.ex/notes/n1"), 1, "content graph untouched");
}

#[test]
fn put_acl_rejects_malformed_content() {
    let mut store = empty_pod();
    let err = store
        .put_acl("https://pod.ex/.acl", "this is not RDF <<<", "ntriples")
        .expect_err("malformed content must be rejected");
    assert!(err.contains("did not parse"), "{err}");
    // Nothing was created.
    assert_eq!(graph_len(&store, "https://pod.ex/.acl"), 0);
}

// --- ACP variant -----------------------------------------------------------------------

#[test]
fn put_acl_acp_creates_and_takes_effect() {
    // An empty ACP pod: one content doc under /docs/, no .acr yet.
    let nq = "<https://pod.ex/docs/d0#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/docs/d0> .\n";
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
    store.materialize_acp().unwrap();
    let alice = sess(ALICE);
    assert_eq!(store.accessible(&alice, Mode::Read).len(), 0, "no grant yet");

    // A root .acr granting alice Read on all member resources (memberAccessControl).
    let acp = "http://www.w3.org/ns/solid/acp#";
    let acr = format!(
        "<https://pod.ex/.acr> <{acp}memberAccessControl> <https://pod.ex/.acr#c> .\n\
         <https://pod.ex/.acr#c> <{acp}apply> <https://pod.ex/.acr#pol> .\n\
         <https://pod.ex/.acr#pol> <{acp}allow> <http://www.w3.org/ns/auth/acl#Read> .\n\
         <https://pod.ex/.acr#pol> <{acp}allOf> <https://pod.ex/.acr#m> .\n\
         <https://pod.ex/.acr#m> <{acp}agent> <{ALICE}> .\n"
    );
    let out = store
        .put_acl_acp("https://pod.ex/.acr", &acr, "ntriples")
        .expect("put_acl_acp succeeds");
    assert!(!out.existed);
    assert!(
        !store.accessible(&alice, Mode::Read).is_empty(),
        "the ACP grant is materialized and visible immediately"
    );

    // DELETE the .acr → grant gone, atomically.
    let out = store.delete_acl_acp("https://pod.ex/.acr").expect("delete_acl_acp");
    assert!(out.existed);
    assert_eq!(
        store.accessible(&alice, Mode::Read).len(),
        0,
        "removing the .acr revokes the grant"
    );
}

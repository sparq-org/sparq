//! A write entry point names the access-control model it rebuilds the authorization view
//! under. An update that changes the access-control documents rebuilds; asked to rebuild under
//! the model the view in force was NOT built under, the store refuses instead of replacing
//! that view — a WAC rebuild of an ACP pod would drop every grant and deny its `.acr` rules
//! decide (and the trusted creator/owner facts they rest on), and an ACP rebuild of a WAC pod
//! would drop everything its `.acl` rules decide. An update that leaves those documents alone
//! rebuilds nothing and is unaffected, whichever entry point it came through.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";
const DOC: &str = "https://pod.ex/a/d";

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

/// `/a/d` readable and writable by alice under ACP member rules, and under WAC rules that say
/// nothing: whichever model is in force decides, and they do not agree.
fn pod() -> String {
    let acr = "https://pod.ex/a/.acr";
    format!(
        "<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .
<{acr}> <{ACP}memberAccessControl> <{acr}#c> <{acr}> .
<{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .
<{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .
<{acr}#p> <{ACP}allow> <{ACL}Write> <{acr}> .
<{acr}#p> <{ACP}allow> <{ACL}Control> <{acr}> .
<{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .
<{acr}#m> <{ACP}agent> <{ALICE}> <{acr}> .
<{acr}.acr> <{ACP}accessControl> <{acr}.acr#c> <{acr}.acr> .
<{acr}.acr#c> <{ACP}apply> <{acr}.acr#p> <{acr}.acr> .
<{acr}.acr#p> <{ACP}allow> <{ACL}Write> <{acr}.acr> .
<{acr}.acr#p> <{ACP}anyOf> <{acr}.acr#m> <{acr}.acr> .
<{acr}.acr#m> <{ACP}agent> <{ALICE}> <{acr}.acr> .
"
    )
}

/// An update that leaves the access-control documents alone: it rebuilds nothing.
fn insert() -> String {
    format!("INSERT DATA {{ GRAPH <{DOC}> {{ <{DOC}#it> <https://ex.dev/ns#t> \"t\" }} }}")
}

/// An update that writes a rule document, so the view is rebuilt under the model the entry
/// point names.
fn insert_rule(acr: &str) -> String {
    format!(
        "INSERT DATA {{ GRAPH <{acr}> {{ <{acr}#p> <{ACP}allow> <{ACL}Append> }} }}"
    )
}

#[test]
fn the_wac_update_entry_point_is_refused_on_a_store_whose_view_is_acp() {
    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("parses"));
    store.materialize_acp().expect("acp materializes");
    assert!(store.decide(&alice(), DOC, Mode::Write).allow);

    // A data update rebuilds nothing, so either entry point writes it.
    store
        .update_as(&alice(), &insert())
        .expect("a data update rebuilds nothing");

    let acr = "https://pod.ex/a/.acr";
    let err = store
        .update_as(&alice(), &insert_rule(acr))
        .expect_err("a WAC rebuild is refused on an ACP view");
    assert!(err.contains("ACP"), "{err}");
    assert!(
        store.decide(&alice(), DOC, Mode::Write).allow,
        "the ACP view is still in force"
    );
    // The ACP entry point goes through and keeps it.
    store
        .update_as_acp(&alice(), &insert_rule(acr))
        .expect("the matching entry point writes the rule");
    assert!(store.decide(&alice(), DOC, Mode::Write).allow);
}

#[test]
fn the_acp_update_entry_point_is_refused_on_a_store_whose_view_is_wac() {
    let nq = format!(
        "<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .
<https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACL}Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}agent> <{ALICE}> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}mode> <{ACL}Read> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}mode> <{ACL}Write> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}accessTo> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#a> <{ACL}mode> <{ACL}Control> <https://pod.ex/.acl> .
"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_wac().expect("wac materializes");
    assert!(store.decide(&alice(), DOC, Mode::Write).allow);

    store
        .update_as_acp(&alice(), &insert())
        .expect("a data update rebuilds nothing");

    let acl = "https://pod.ex/.acl";
    let rule = format!(
        "INSERT DATA {{ GRAPH <{acl}> {{ <{acl}#a> <{ACL}mode> <{ACL}Append> }} }}"
    );
    let err = store
        .update_as_acp(&alice(), &rule)
        .expect_err("an ACP rebuild is refused on a WAC view");
    assert!(err.contains("WAC"), "{err}");
    assert!(
        store.decide(&alice(), DOC, Mode::Write).allow,
        "the WAC view is still in force"
    );
    store
        .update_as(&alice(), &rule)
        .expect("the matching entry point writes the rule");
    assert!(store.decide(&alice(), DOC, Mode::Write).allow);
}

/// A store with no view in force is left to its caller: the first materialization decides the
/// model, as it does today.
#[test]
fn a_store_with_no_view_in_force_refuses_neither_entry_point_for_its_model() {
    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("parses"));
    // No materialization yet: the update is denied for want of a grant, not for the model.
    let err = store.update_as(&alice(), &insert()).expect_err("no grants");
    assert!(!err.contains("authorization view was built under"), "{err}");
    let err = store
        .update_as(&alice(), &insert_rule("https://pod.ex/a/.acr"))
        .expect_err("no grants");
    assert!(!err.contains("authorization view was built under"), "{err}");
}

//! A write entry point names the access-control model it rebuilds the authorization view
//! under. An update that changes the access-control documents rebuilds; asked to rebuild under
//! the model the view in force was NOT built under, the store refuses instead of replacing
//! that view — a WAC rebuild of an ACP pod would drop every grant and deny its `.acr` rules
//! decide (and the trusted creator/owner facts they rest on), and an ACP rebuild of a WAC pod
//! would drop everything its `.acl` rules decide. An update that leaves those documents alone
//! rebuilds nothing and is unaffected, whichever entry point it came through.

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";
const DOC: &str = "https://pod.ex/a/d";
const ACR: &str = "https://pod.ex/a/.acr";

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
    let mut store = PodStore::new(Graph::load_dataset(&wac_pod(), "nquads").expect("parses"));
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

/// What a refusal must leave alone: every stored graph with its size, the stored creator/owner
/// facts, and the decisions the view in force gives.
fn snapshot(store: &PodStore) -> (Vec<(String, usize)>, AccessProvenance, Vec<bool>) {
    let mut graphs: Vec<(String, usize)> = store
        .graph()
        .named
        .iter()
        .map(|(n, g)| (format!("{}", n), g.len()))
        .collect();
    graphs.sort();
    let mut decisions = Vec::new();
    for iri in [DOC, "https://pod.ex/a/", "https://pod.ex/", "https://pod.ex/a/.acr"] {
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            decisions.push(store.decide(&alice(), iri, mode).allow);
        }
    }
    (graphs, store.acp_provenance().clone(), decisions)
}

/// One call per entry point that rebuilds the view: `acp` is the model the call names, `doc`
/// the access-control document it writes (the one the view in force reads).
type Call = (&'static str, fn(&mut PodStore, bool, &str) -> Result<(), String>);

fn doc_content(doc: &str) -> String {
    format!("<{doc}#p> <{ACP}allow> <{ACL}Append> .\n")
}

fn rebuilding_entry_points() -> Vec<Call> {
    vec![
        ("update_as*", |store, acp, doc| {
            let rule = insert_rule(doc);
            if acp {
                store.update_as_acp(&alice(), &rule)
            } else {
                store.update_as(&alice(), &rule)
            }
        }),
        ("update_as*_with_budget", |store, acp, doc| {
            let rule = insert_rule(doc);
            let budget = QueryBudget::unlimited();
            if acp {
                store.update_as_acp_with_budget(&alice(), &rule, &budget)
            } else {
                store.update_as_with_budget(&alice(), &rule, &budget)
            }
        }),
        ("put_acl*", |store, acp, doc| {
            let content = doc_content(doc);
            if acp {
                store.put_acl_acp(doc, &content, "ntriples").map(|_| ())
            } else {
                store.put_acl(doc, &content, "ntriples").map(|_| ())
            }
        }),
        ("delete_acl*", |store, acp, doc| {
            if acp {
                store.delete_acl_acp(doc).map(|_| ())
            } else {
                store.delete_acl(doc).map(|_| ())
            }
        }),
        ("rematerialize_after_edit", |store, acp, _doc| {
            let before = store.stored_graphs();
            store
                .rematerialize_after_edit(&before, None, &[], acp)
                .map(|_| ())
        }),
    ]
}

/// The WAC pod of `the_acp_update_entry_point_is_refused_on_a_store_whose_view_is_wac`.
fn wac_pod() -> String {
    let acl = "https://pod.ex/.acl";
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    format!(
        "<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .
<{acl}#a> <{rdf_type}> <{ACL}Authorization> <{acl}> .
<{acl}#a> <{ACL}default> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}accessTo> <https://pod.ex/> <{acl}> .
<{acl}#a> <{ACL}agent> <{ALICE}> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Read> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Write> <{acl}> .
<{acl}#a> <{ACL}mode> <{ACL}Control> <{acl}> .
"
    )
}

/// Every entry point that rebuilds the authorization view refuses to rebuild under the model
/// the view in force was not built under — and the refusal changes nothing: the stored graphs,
/// the stored creator/owner facts and the decisions the view gives are all as they were.
/// Both directions: each WAC call on an ACP view, and each ACP call on a WAC view.
#[test]
fn every_rebuilding_entry_point_refuses_a_mismatched_model_and_changes_nothing() {
    for installed_acp in [true, false] {
        for (name, call) in rebuilding_entry_points() {
            let (nquads, doc) = if installed_acp {
                (pod(), ACR)
            } else {
                (wac_pod(), "https://pod.ex/.acl")
            };
            let mut store = PodStore::new(Graph::load_dataset(&nquads, "nquads").expect("parses"));
            if installed_acp {
                store.materialize_acp().expect("acp materializes");
            } else {
                store.materialize_wac().expect("wac materializes");
            }
            assert!(store.decide(&alice(), DOC, Mode::Write).allow, "{}", name);
            let before = snapshot(&store);

            let err = call(&mut store, !installed_acp, doc)
                .expect_err("the entry point is refused for the other model");
            assert!(
                err.contains(if installed_acp { "ACP" } else { "WAC" })
                    && err.contains("authorization view was built under"),
                "{}: {}",
                name,
                err
            );
            assert_eq!(snapshot(&store), before, "{} changed something", name);
        }
    }
}

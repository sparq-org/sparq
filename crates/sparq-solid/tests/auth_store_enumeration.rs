//! The STORE-SIDE resource enumeration is metered on the same caps as a materialization's
//! own, and refuses the authorization view rather than walking an unbounded name.
//!
//! A materialization charges the resources it assembles its input from
//! (`AuthBudget::max_resource_iri_bytes` / `max_resource_path_depth`, and every string it
//! owns against the aggregate `max_input_bytes`). Three enumerations of the SAME names run
//! outside that input assembly and used to be unmetered: the store's own provenance pruning
//! (`loader::resources`), the whole-set walk behind `ResourceGraphs::clear`, and the
//! lifecycle REPLAY that `PodStore::rematerialize_after_edit` runs — which walks the
//! container chain of every name the edit touches and OWNS each container it finds, for a
//! name the edit is ADDING rather than one the store already holds.
//!
//! Each of them is now charged on the store's `AuthBudget`, before anything is installed,
//! so a name past the caps makes the authorization view REFUSE (fail-closed, the view in
//! force untouched) instead of the walk running to completion. Normal names are unaffected:
//! the defaults admit a 2 KiB IRI under 64 containers, which is far past what a pod's own
//! names are.

use sparq_core::Graph;
use sparq_solid::{
    auth_allowance_refusal, default_auth_budget, AuthAllowance, Mode, PodStore, Session,
};

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ALICE: &str = "https://alice.ex/card#me";

fn alice() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

/// A pod whose root `.acl` gives alice Read by default, plus one document under it.
fn pod() -> PodStore {
    let acl = "https://pod.ex/.acl";
    let nq = format!(
        "<{acl}#r> <{RDF_TYPE}> <{ACL}Authorization> <{acl}> .\n\
         <{acl}#r> <{ACL}agent> <{ALICE}> <{acl}> .\n\
         <{acl}#r> <{ACL}default> <https://pod.ex/> <{acl}> .\n\
         <{acl}#r> <{ACL}mode> <{ACL}Read> <{acl}> .\n\
         <https://pod.ex/n1#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/n1> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_wac().expect("the fixture materializes");
    assert!(
        store
            .decide(&alice(), "https://pod.ex/n1", Mode::Read)
            .allow,
        "the fixture grants alice Read"
    );
    store
}

/// Store an empty named graph called `name` directly, the way an embedder's write path does.
fn store_graph(store: &mut PodStore, name: &str) {
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new(name).expect("an IRI"));
    store
        .graph_unchecked_mut()
        .named
        .push((term, Graph::default()));
}

/// A resource IRI under `depth` containers, well inside `max_resource_iri_bytes` so the
/// DEPTH cap is the one that refuses it.
fn deep_name(depth: usize) -> String {
    format!("https://pod.ex/{}doc", "a/".repeat(depth))
}

/// The replay a direct edit's rebuild runs is metered: a newly added name under more
/// containers than `max_resource_path_depth` refuses the rebuild, with the view in force
/// untouched and nothing installed.
#[test]
fn an_added_name_past_the_depth_cap_refuses_the_rebuild() {
    let mut store = pod();
    let before = store.stored_graphs();
    let name = deep_name(default_auth_budget().max_resource_path_depth + 8);
    assert!(
        name.len() < default_auth_budget().max_resource_iri_bytes,
        "the name is inside the LENGTH cap, so the depth cap is what refuses it"
    );
    store_graph(&mut store, &name);

    let err = store
        .rematerialize_after_edit(&before, Some(ALICE), &[name.as_str()], false)
        .expect_err("a name past the depth cap must refuse the rebuild");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::ResourceIri),
        "the refusal is the typed resource-enumeration one: {}",
        err
    );
    // Fail-closed: the view in force is untouched (it still describes the pod it was built
    // for), and the refused name was granted nothing.
    assert!(
        store
            .decide(&alice(), "https://pod.ex/n1", Mode::Read)
            .allow,
        "the view in force is left in place"
    );
    assert!(
        !store.decide(&alice(), &name, Mode::Read).allow,
        "the refused resource is granted nothing"
    );
}

/// The same guard on the LENGTH dimension: a single very long name refuses the rebuild.
#[test]
fn an_added_name_past_the_length_cap_refuses_the_rebuild() {
    let mut store = pod();
    let before = store.stored_graphs();
    let name = format!(
        "https://pod.ex/{}",
        "x".repeat(default_auth_budget().max_resource_iri_bytes + 1)
    );
    store_graph(&mut store, &name);

    let err = store
        .rematerialize_after_edit(&before, Some(ALICE), &[name.as_str()], false)
        .expect_err("a name past the length cap must refuse the rebuild");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::ResourceIri),
        "the refusal is the typed resource-enumeration one: {}",
        err
    );
    assert!(
        store
            .decide(&alice(), "https://pod.ex/n1", Mode::Read)
            .allow,
        "the view in force is left in place"
    );
}

/// The replay's AGGREGATE charge: many names, each inside both per-name caps, together past
/// `max_input_bytes` refuse the rebuild — so no shape of names can make the replay cost more
/// than the input budget.
#[test]
fn an_edit_whose_names_exceed_the_aggregate_refuses_the_rebuild() {
    let mut store = pod();
    let mut budget = default_auth_budget();
    budget.max_input_bytes = 2_000;
    store.set_auth_budget(budget);
    let before = store.stored_graphs();
    let names: Vec<String> = (0..200)
        .map(|i| format!("https://pod.ex/bulk/{}/doc-{}", i, i))
        .collect();
    for name in &names {
        store_graph(&mut store, name);
    }
    let created: Vec<&str> = names.iter().map(String::as_str).collect();

    let err = store
        .rematerialize_after_edit(&before, Some(ALICE), &created, false)
        .expect_err("names past the aggregate must refuse the rebuild");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::InputBytes),
        "the refusal is the aggregate input-bytes one: {}",
        err
    );
    assert!(
        store
            .decide(&alice(), "https://pod.ex/n1", Mode::Read)
            .allow,
        "the view in force is left in place"
    );
}

/// A name ALREADY stored past the caps refuses the authorization view rather than making the
/// store unusable: the store opens, holds its graphs, and answers decisions from the view in
/// force; it is the REBUILD that refuses.
#[test]
fn an_already_stored_name_past_the_caps_refuses_the_view_not_the_store() {
    let name = deep_name(default_auth_budget().max_resource_path_depth + 8);
    let nq = format!("<{name}#it> <https://ex.dev/ns#k> \"v\" <{name}> .\n");
    // The store opens over the dataset and is readable as plain RDF.
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    assert_eq!(store.graph().named.len(), 1, "the store holds its graph");

    let err = store
        .materialize_wac()
        .expect_err("the view over a name past the caps is refused");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::ResourceIri),
        "the refusal is the typed resource-enumeration one: {}",
        err
    );
    // Fail-closed: no view, so nothing is granted.
    assert!(!store.decide(&alice(), &name, Mode::Read).allow);
}

/// Normal behaviour is unchanged: an ordinary edit still rebuilds, records its creator, and
/// keeps the grants the pod's documents state. The metering is a ceiling, not a filter.
#[test]
fn an_ordinary_edit_still_rebuilds_and_records_its_creator() {
    let mut store = pod();
    let before = store.stored_graphs();
    store_graph(&mut store, "https://pod.ex/deep/er/n2");

    store
        .rematerialize_after_edit(&before, Some(ALICE), &["https://pod.ex/deep/er/n2"], false)
        .expect("an ordinary edit rebuilds");
    assert_eq!(
        store.acp_provenance().creator("https://pod.ex/deep/er/n2"),
        Some(ALICE),
        "the creator of the resource the edit named is recorded"
    );
    assert_eq!(
        store.acp_provenance().creator("https://pod.ex/deep/"),
        None,
        "a container the edit only brings into being keeps no creator"
    );
    assert!(
        store
            .decide(&alice(), "https://pod.ex/deep/er/n2", Mode::Read)
            .allow,
        "the rebuilt view still grants what the root .acl states"
    );
}

//! Replay-specific guards: the final dataset fits, but an intermediate added name does
//! not. Materialization cannot mask a missing replay guard in these tests.

use sparq_core::Graph;
use sparq_solid::{
    auth_allowance_refusal, default_auth_budget, AuthAllowance, Mode, PodStore, Session,
};

const ALICE: &str = "https://alice.ex/card#me";

fn owner() -> Session<'static> {
    Session {
        agent: Some(ALICE),
        ..Session::default()
    }
}

fn pod() -> PodStore {
    let mut nq = "<https://pod.ex/n1> <https://ex.dev/p> \"v\" <https://pod.ex/n1> .\n".to_owned();
    let acl = "https://pod.ex/.acl";
    let ns = "http://www.w3.org/ns/auth/acl#";
    nq.push_str(&format!("\n<{acl}#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ns}Authorization> <{acl}> .\n"));
    for (predicate, object) in [
        ("agent", ALICE),
        ("default", "https://pod.ex/"),
        ("accessTo", "https://pod.ex/"),
        ("mode", "http://www.w3.org/ns/auth/acl#Read"),
        ("mode", "http://www.w3.org/ns/auth/acl#Write"),
        ("mode", "http://www.w3.org/ns/auth/acl#Control"),
    ] {
        nq.push_str(&format!(
            "<{acl}#r> <{ns}{predicate}> <{object}> <{acl}> .\n"
        ));
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
    store.materialize_wac().unwrap();
    store
}

fn add_graph(store: &mut PodStore, name: &str) {
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new(name).unwrap());
    store.graph_unchecked_mut().ensure_named(&term).unwrap();
}

/// Install a view and creator fact under a generous budget. Then remove only the data
/// graph, retaining the authorization view as an embedder can between edit and rebuild.
/// The final store fits the lowered budget; only replay sees the transient added name.
fn refuses_transient_name(name: &str, operations: &str) {
    let mut store = pod();
    let before = store.stored_graphs();
    add_graph(&mut store, name);
    let mut generous = default_auth_budget();
    generous.max_resource_iri_bytes = name.len().max(generous.max_resource_iri_bytes);
    generous.max_resource_path_depth = 1000;
    store.set_auth_budget(generous);
    store
        .rematerialize_after_edit(&before, Some(ALICE), &[name], false)
        .unwrap();
    assert_eq!(store.acp_provenance().creator(name), Some(ALICE));
    assert!(store.decide(&owner(), name, Mode::Write).allow);
    store
        .graph_unchecked_mut()
        .named
        .retain(|(n, _)| !matches!(n, oxrdf::Term::NamedNode(n) if n.as_str() == name));
    store.set_auth_budget(default_auth_budget());
    let mut fresh = PodStore::new(store.graph().fork());
    fresh
        .materialize_wac()
        .expect("the final dataset fits from scratch");

    let err = store
        .update_as(&owner(), operations)
        .expect_err("replay refuses the transient added name");
    assert_eq!(
        auth_allowance_refusal(&err),
        Some(AuthAllowance::ResourceIri),
        "{err}"
    );
    assert!(
        store
            .decide(&owner(), "https://pod.ex/n1", Mode::Read)
            .allow
    );
    assert!(store
        .graph()
        .named_graph(&oxrdf::Term::NamedNode(
            oxrdf::NamedNode::new(name).unwrap()
        ))
        .is_none());
}

#[test]
fn create_then_drop_checks_the_added_names_depth() {
    let name = format!("https://pod.ex/{}doc", "a/".repeat(72));
    refuses_transient_name(
        &name,
        &format!("CREATE GRAPH <{name}>; DROP GRAPH <{name}>"),
    );
}

#[test]
fn insert_then_drop_checks_the_added_names_length() {
    let name = format!("https://pod.ex/{}", "x".repeat(2100));
    refuses_transient_name(
        &name,
        &format!(
            "INSERT DATA {{ GRAPH <{name}> {{ <urn:s> <urn:p> <urn:o> }} }}; DROP GRAPH <{name}>"
        ),
    );
}

#[test]
fn removal_before_create_does_not_bypass_the_added_name_cap() {
    let name = format!("https://pod.ex/{}doc", "a/".repeat(72));
    refuses_transient_name(
        &name,
        &format!("DROP SILENT GRAPH <{name}>; CREATE GRAPH <{name}>; DROP GRAPH <{name}>"),
    );
}

#[test]
fn clear_all_before_create_does_not_bypass_the_added_name_cap() {
    let name = format!("https://pod.ex/{}doc", "a/".repeat(72));
    refuses_transient_name(
        &name,
        &format!("DROP NAMED; CREATE GRAPH <{name}>; DROP GRAPH <{name}>"),
    );
}

#[test]
fn clear_named_after_create_does_not_mask_the_added_name_cap() {
    let name = format!("https://pod.ex/{}", "x".repeat(2100));
    refuses_transient_name(&name, &format!("CREATE GRAPH <{name}>; DROP NAMED"));
}

#[test]
fn lowering_the_depth_budget_never_prevents_dropping_a_stored_graph() {
    let mut store = pod();
    let name = format!("https://pod.ex/{}doc", "a/".repeat(100));
    let before = store.stored_graphs();
    add_graph(&mut store, &name);
    let mut budget = default_auth_budget();
    budget.max_resource_path_depth = 1000;
    store.set_auth_budget(budget);
    store
        .rematerialize_after_edit(&before, Some(ALICE), &[&name], false)
        .unwrap();
    store.set_auth_budget(default_auth_budget());
    store
        .update_as(&owner(), &format!("DROP GRAPH <{name}>"))
        .expect("removals never refuse");
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new(&name).unwrap());
    assert!(store.graph().named_graph(&term).is_none());
    assert_eq!(store.acp_provenance().creator(&name), None);
}

#[test]
fn moving_resources_fits_the_smallest_materialization_budget_of_either_store() {
    fn graph(folder: &str) -> Graph {
        let mut graph = Graph::default();
        for i in 0..100 {
            let name = oxrdf::Term::NamedNode(
                oxrdf::NamedNode::new(format!("https://pod.ex/{folder}/folder/{i}")).unwrap(),
            );
            graph.ensure_named(&name).unwrap();
        }
        graph
    }
    fn fits(graph: &Graph, bytes: usize) -> bool {
        let mut store = PodStore::new(graph.fork());
        let mut budget = default_auth_budget();
        budget.max_input_bytes = bytes;
        store.set_auth_budget(budget);
        store.materialize_wac().is_ok()
    }
    let before_graph = graph("old");
    let after_graph = graph("new");
    let mut low = 0;
    let mut high = default_auth_budget().max_input_bytes;
    assert!(fits(&before_graph, high) && fits(&after_graph, high));
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if fits(&before_graph, mid) && fits(&after_graph, mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    assert!(!fits(&before_graph, high - 1) || !fits(&after_graph, high - 1));
    let mut store = PodStore::new(before_graph);
    let mut budget = default_auth_budget();
    budget.max_input_bytes = high;
    store.set_auth_budget(budget);
    store.materialize_wac().unwrap();
    let before = store.stored_graphs();
    *store.graph_unchecked_mut() = after_graph;
    store
        .rematerialize_after_edit(&before, None, &[], false)
        .expect("the move charges no aggregate replay bytes");
    assert!(store
        .graph()
        .named_graph(&oxrdf::Term::NamedNode(
            oxrdf::NamedNode::new("https://pod.ex/new/folder/0").unwrap()
        ))
        .is_some());
}

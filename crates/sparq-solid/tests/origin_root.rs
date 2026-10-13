//! A graph named by a bare `scheme://` (no authority) sits above every authority root, so
//! it is never the container of a pod root: neither it nor its control document takes
//! part in the auth view, under WAC or ACP.

use sparq_core::Graph;
use sparq_solid::{acp_fixture, wac_fixture, Mode, PodStore, Session};

const BOB: &str = "https://bob.ex/card#me";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// A fixture subtree Bob has no access to.
const PRIV: &str = "https://pod.ex/priv0/";

fn bob() -> Session<'static> {
    Session {
        agent: Some(BOB),
        client: None,
        issuer: None,
        now: None,
    }
}

/// The `scheme://` resource graph itself (it must exist to be a parent candidate).
fn bare_scheme_graph() -> String {
    "<https://> <https://ex.dev/ns#x> \"y\" <https://> .\n".to_owned()
}

fn wac_member_read_for_bob() -> String {
    let g = "https://.acl";
    format!(
        "{}<{g}#r> <{RDF_TYPE}> <{ACL}Authorization> <{g}> .\n\
         <{g}#r> <{ACL}default> <https://> <{g}> .\n\
         <{g}#r> <{ACL}agent> <{BOB}> <{g}> .\n\
         <{g}#r> <{ACL}mode> <{ACL}Read> <{g}> .\n",
        bare_scheme_graph()
    )
}

fn acp_member_read_for_bob() -> String {
    let g = "https://.acr";
    format!(
        "{}<{g}> <{ACP}resource> <https://> <{g}> .\n\
         <{g}> <{ACP}memberAccessControl> <{g}#c> <{g}> .\n\
         <{g}#c> <{ACP}apply> <{g}#p> <{g}> .\n\
         <{g}#p> <{ACP}allow> <{ACL}Read> <{g}> .\n\
         <{g}#p> <{ACP}anyOf> <{g}#m> <{g}> .\n\
         <{g}#m> <{ACP}agent> <{BOB}> <{g}> .\n",
        bare_scheme_graph()
    )
}

fn bob_reads_in_priv(store: &PodStore) -> usize {
    store
        .accessible(&bob(), Mode::Read)
        .iter()
        .filter(|g| g.as_str().starts_with(PRIV))
        .count()
}

#[test]
fn bare_scheme_acl_does_not_reach_pod_roots() {
    // WAC inherits from the NEAREST ancestor ACL, so use a pod whose root has none.
    let mut nq = "<https://pod.ex/priv0/n1#it> <https://ex.dev/ns#title> \"hi\" <https://pod.ex/priv0/n1> .\n"
        .to_owned();
    nq.push_str(&wac_member_read_for_bob());
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    assert_eq!(bob_reads_in_priv(&store), 0);
}

#[test]
fn pod_root_acls_still_govern_with_a_bare_scheme_graph_present() {
    let mut nq = wac_fixture();
    nq.push_str(&wac_member_read_for_bob());
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    let mut plain = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").expect("loads"));
    plain.materialize_wac().expect("materializes");
    assert_eq!(
        *store.accessible(&bob(), Mode::Read),
        *plain.accessible(&bob(), Mode::Read)
    );
}

#[test]
fn bare_scheme_acr_does_not_reach_pod_roots() {
    let mut nq = acp_fixture();
    nq.push_str(&acp_member_read_for_bob());
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    store.materialize_acp().expect("materializes");
    assert_eq!(bob_reads_in_priv(&store), 0);
}

#[test]
fn sessions_cannot_write_bare_scheme_graphs() {
    let mut store = PodStore::new(Graph::load_dataset(&acp_fixture(), "nquads").expect("loads"));
    store.materialize_acp().expect("materializes");
    let alice = Session {
        agent: Some("https://alice.ex/card#me"),
        client: None,
        issuer: None,
        now: None,
    };
    for g in ["https://", "https://.acr"] {
        let q = format!("INSERT DATA {{ GRAPH <{g}> {{ <https://a> <https://b> <https://c> }} }}");
        assert!(
            store.update_as_acp(&alice, &q).is_err(),
            "write to <{g}> must be refused"
        );
    }
}

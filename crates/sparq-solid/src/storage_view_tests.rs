//! Durable authorization view writes, their deltas, and refused writes.
use super::*;
use std::path::{Path, PathBuf};

const DOC: &str = "https://pod.ex/note";
const ALICE: &str = "https://alice.ex/card#me";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";

fn named(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}

fn pod(tag: &str) -> (PathBuf, PodStore) {
    let dir = std::env::temp_dir().join(format!("sparq-view-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let nq = format!(
        "<https://pod.ex/note.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACL}Authorization> <https://pod.ex/note.acl> .\n\
         <{DOC}#it> <https://ex.dev/p> \"value\" <{DOC}> .\n\
         <https://pod.ex/note.acl#r> <{ACL}accessTo> <{DOC}> <https://pod.ex/note.acl> .\n\
         <https://pod.ex/note.acl#r> <{ACL}agent> <{ALICE}> <https://pod.ex/note.acl> .\n\
         <https://pod.ex/note.acl#r> <{ACL}mode> <{ACL}Read> <https://pod.ex/note.acl> .\n"
    );
    Graph::load_dataset(&nq, "nquads")
        .unwrap()
        .save(&dir)
        .unwrap();
    let mut store = PodStore::new(Graph::open(&dir).unwrap());
    store.materialize_wac().unwrap();
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    assert!(
        store.decide(&session, DOC, Mode::Read).allow,
        "fixture must grant Read"
    );
    (dir, store)
}

fn wal(dir: &Path, store: &PodStore) -> PathBuf {
    let slot = store
        .graph
        .named()
        .iter()
        .position(|(n, _)| *n == named(AUTH_GRAPH))
        .unwrap();
    dir.join("named").join(slot.to_string()).join("wal.log")
}

/// Count committed WAL records, checking the frame boundaries rather than searching text.
fn records(bytes: &[u8]) -> usize {
    let mut pos = 0;
    let mut records = 0;
    while pos < bytes.len() {
        assert!(bytes.len() - pos >= 24, "incomplete frame");
        records += u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap()) as usize;
        pos += 12 + body + 12;
    }
    assert_eq!(pos, bytes.len());
    records
}

#[test]
fn unchanged_materialization_writes_nothing_and_one_rule_logs_only_its_difference() {
    let (dir, mut store) = pod("delta");
    let log = wal(&dir, &store);
    let before = std::fs::read(&log).unwrap();
    for _ in 0..2 {
        store.materialize_wac().unwrap();
        assert_eq!(
            std::fs::read(&log).unwrap(),
            before,
            "unchanged view appended or rewrote its WAL"
        );
    }
    let old: FxHashSet<_> =
        loader::graph_triples(store.graph.named_graph(&named(AUTH_GRAPH)).unwrap())
            .into_iter()
            .collect();
    let slot = store
        .graph
        .named()
        .iter()
        .position(|(n, _)| *n == named("https://pod.ex/note.acl"))
        .unwrap();
    store
        .graph
        .apply_delta_to_named(
            slot,
            &[[
                named("https://pod.ex/note.acl#r"),
                named(&format!("{ACL}mode")),
                named(&format!("{ACL}Write")),
            ]],
            &[],
        )
        .unwrap();
    store.materialize_wac().unwrap();
    let new: FxHashSet<_> =
        loader::graph_triples(store.graph.named_graph(&named(AUTH_GRAPH)).unwrap())
            .into_iter()
            .collect();
    let after = std::fs::read(&log).unwrap();
    assert!(
        after.starts_with(&before),
        "materialization rewrote the existing WAL"
    );
    let difference = new.symmetric_difference(&old).count();
    assert!(difference > 0 && difference < new.len() + old.len());
    assert_eq!(
        records(&after[before.len()..]),
        difference,
        "only changed grants belong in the WAL"
    );
    drop(store);
    let reopened = Graph::open(&dir).unwrap();
    let recovered: FxHashSet<_> =
        loader::graph_triples(reopened.named_graph(&named(AUTH_GRAPH)).unwrap())
            .into_iter()
            .collect();
    assert_eq!(recovered, new);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn absent_view_drop_does_not_attempt_a_write() {
    let (dir, mut store) = pod("absent");
    loader::FAIL_VIEW_WRITE.set(true);
    loader::drop_named_view(&mut store.graph, &named("urn:sparq:absent")).unwrap();
    assert!(
        loader::FAIL_VIEW_WRITE.replace(false),
        "absent view attempted a storage write"
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "odrl-bridge")]
#[test]
fn bridge_write_and_drop_failures_return_errors_and_disable_the_view() {
    let policy = sparq_policy::parse_policy_str(&format!(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:policy> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; odrl:target <{DOC}> ; odrl:assignee <https://bob.ex/#me> ] ."), "turtle").unwrap();
    let request = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
        .on(DOC)
        .by("https://bob.ex/#me");
    for operation in ["install", "reset", "drop"] {
        let (dir, mut store) = pod(operation);
        if operation != "install" {
            store
                .materialize_odrl_permission(&policy, &request)
                .unwrap();
        }
        if operation == "drop" {
            // Keep the enforcement view equal to the baseline so refresh's first write
            // is removal of the bridged-provenance slot.
            store.bridge_ledger.capture_static_baseline(&store.graph);
        }
        loader::FAIL_VIEW_WRITE.set(true);
        let error = if operation == "install" {
            store
                .materialize_odrl_permission(&policy, &request)
                .unwrap_err()
        } else {
            store.refresh_odrl_grants().unwrap_err()
        };
        assert!(error.contains("injected authorization view write failure"));
        assert!(!loader::FAIL_VIEW_WRITE.get(), "fault hook was not reached");
        assert!(store.view_dropped && !store.may_install_grants());
        for agent in [ALICE, "https://bob.ex/#me"] {
            let session = Session {
                agent: Some(agent),
                ..Session::default()
            };
            assert!(store.accessible(&session, Mode::Read).is_empty());
        }
        store.materialize_wac().unwrap();
        assert!(!store.view_dropped);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(feature = "odrl-bridge")]
#[test]
fn persistent_view_write_failure_disables_grants_even_when_cleanup_is_refused() {
    let (dir, mut store) = pod("persistent-failure");
    let session = Session {
        agent: Some(ALICE),
        ..Session::default()
    };
    assert!(store.decide(&session, DOC, Mode::Read).allow);
    assert!(!store.accessible(&session, Mode::Read).is_empty());
    let policy = sparq_policy::parse_policy_str(&format!(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> . <urn:policy> a odrl:Set ; odrl:permission [ odrl:action odrl:read ; odrl:target <{DOC}> ; odrl:assignee <https://bob.ex/#me> ] ."), "turtle").unwrap();
    let request = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
        .on(DOC)
        .by("https://bob.ex/#me");
    loader::FAIL_ALL_VIEW_WRITES.set(true);
    let result = store.materialize_odrl_permission(&policy, &request);
    loader::FAIL_ALL_VIEW_WRITES.set(false);
    assert!(
        result
            .unwrap_err()
            .contains("injected authorization view write failure")
    );
    assert!(
        store.graph.named_graph(&named(AUTH_GRAPH)).is_some(),
        "cleanup must also have been refused"
    );
    assert!(store.view_dropped);
    assert!(store.accessible(&session, Mode::Read).is_empty());
    assert!(store.accessible_set(&session, Mode::Read).is_empty());
    let decision = store.decide(&session, DOC, Mode::Read);
    assert!(!decision.allow && decision.status == AclStatus::Unloaded);
    assert!(
        store
            .query_as(&session, Mode::Read, "SELECT * { GRAPH ?g { ?s ?p ?o } }")
            .unwrap()
            .rows
            .is_empty()
    );
    store.materialize_wac().unwrap();
    assert!(store.decide(&session, DOC, Mode::Read).allow);
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

//! Every public update entry point takes the one commit path: it authorizes what the update
//! actually wrote (not only what a static reading of the request predicted), commits it to
//! the store itself (a directory-backed store keeps it across a reopen), and keeps the auth
//! view consistent with the rules it leaves.

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const NOTE: &str = "https://pod.ex/notes/n1";

type Entry = fn(&mut PodStore, &Session, &str) -> Result<(), String>;

/// Every public update entry point, by name.
const ENTRY_POINTS: &[(&str, Entry)] = &[
    ("update_as", |p, s, q| p.update_as(s, q)),
    ("update_as_acp", |p, s, q| p.update_as_acp(s, q)),
    ("update_as_with_budget", |p, s, q| {
        p.update_as_with_budget(s, q, &QueryBudget::unlimited())
    }),
    ("update_as_acp_with_budget", |p, s, q| {
        p.update_as_acp_with_budget(s, q, &QueryBudget::unlimited())
    }),
];

fn sess(agent: &str) -> Session<'_> {
    Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    }
}

fn reads(store: &PodStore, agent: &str, graph: &str) -> bool {
    store
        .accessible(&sess(agent), Mode::Read)
        .iter()
        .any(|g| g == graph)
}

const ACL: &str = "http://www.w3.org/ns/auth/acl#";

fn fixture() -> String {
    let rule =
        |doc: &str, id: &str, p: &str, o: &str| format!("<{doc}#{id}> <{p}> <{o}> <{doc}> .\n");
    let mut nq = String::new();
    nq += &format!("<{NOTE}#it> <https://ex.dev/ns#title> \"hello\" <{NOTE}> .\n");
    nq += "<https://pod.ex/bob/b1#it> <https://ex.dev/ns#title> \"bob's\" <https://pod.ex/bob/b1> .\n";
    let root = "https://pod.ex/.acl";
    for (id, agent, modes) in [
        ("owner", ADMIN, &["Read", "Write", "Control"][..]),
        ("alice", ALICE, &["Read"][..]),
    ] {
        nq += &rule(
            root,
            id,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            &format!("{ACL}Authorization"),
        );
        nq += &rule(root, id, &format!("{ACL}accessTo"), "https://pod.ex/");
        nq += &rule(root, id, &format!("{ACL}default"), "https://pod.ex/");
        nq += &rule(root, id, &format!("{ACL}agent"), agent);
        for m in modes {
            nq += &rule(root, id, &format!("{ACL}mode"), &format!("{ACL}{m}"));
        }
    }
    let bob = "https://pod.ex/bob/.acl";
    nq += &rule(
        bob,
        "own",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        &format!("{ACL}Authorization"),
    );
    nq += &rule(bob, "own", &format!("{ACL}accessTo"), "https://pod.ex/bob/");
    nq += &rule(bob, "own", &format!("{ACL}default"), "https://pod.ex/bob/");
    nq += &rule(bob, "own", &format!("{ACL}agent"), BOB);
    for m in ["Read", "Write"] {
        nq += &rule(bob, "own", &format!("{ACL}mode"), &format!("{ACL}{m}"));
    }
    nq
}

fn store() -> PodStore {
    let mut s = PodStore::new(Graph::load_dataset(&fixture(), "nquads").expect("fixture loads"));
    s.materialize_wac().expect("wac materializes");
    s
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_solid_entry_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn graph_len(store: &PodStore, name: &str) -> usize {
    store
        .graph
        .named
        .iter()
        .find(|(n, _)| matches!(n, oxrdf::Term::NamedNode(n) if n.as_str() == name))
        .map_or(0, |(_, g)| g.len())
}

/// Review finding: a successful update replaced a directory-backed graph with an in-memory
/// fork, so what it wrote was lost at the next open. Every entry point commits to the store.
#[test]
fn every_entry_point_persists_what_it_writes() {
    for (i, (name, entry)) in ENTRY_POINTS.iter().enumerate() {
        let dir = tmp(&format!("persist{i}"));
        Graph::load_dataset(&fixture(), "nquads")
            .unwrap()
            .save(&dir)
            .unwrap();
        let triple = format!("<{NOTE}#it> <https://ex.dev/ns#via> \"{name}\"");
        {
            let mut store = PodStore::new(Graph::open(&dir).unwrap());
            store.materialize_wac().unwrap();
            let before = graph_len(&store, NOTE);
            entry(
                &mut store,
                &sess(ADMIN),
                &format!("INSERT DATA {{ GRAPH <{NOTE}> {{ {triple} }} }}"),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(graph_len(&store, NOTE), before + 1, "{name}");
        }
        let reopened = PodStore::new(Graph::open(&dir).unwrap());
        assert_eq!(
            graph_len(&reopened, NOTE),
            2,
            "{name}: the write did not survive a reopen"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Review finding: the static check resolved a `GRAPH ?g` target against the store before the
/// request, so a later operation could reach a graph an earlier one named: Bob, who may write
/// only his own container, removed a rule from the root ACL. Every entry point authorizes what
/// the update actually wrote.
#[test]
fn every_entry_point_authorizes_what_the_update_wrote() {
    let attack = "INSERT DATA { GRAPH <https://pod.ex/bob/b1> { \
        <https://pod.ex/bob/b1#it> <https://ex.dev/ns#ptr> <https://pod.ex/.acl> } } ; \
        DELETE { GRAPH ?g { <https://pod.ex/.acl#alice> ?p ?o } } \
        WHERE { GRAPH <https://pod.ex/bob/b1> { <https://pod.ex/bob/b1#it> <https://ex.dev/ns#ptr> ?g } \
                GRAPH ?g { <https://pod.ex/.acl#alice> ?p ?o } }";
    for (name, entry) in ENTRY_POINTS {
        let mut store = store();
        let rules = graph_len(&store, "https://pod.ex/.acl");
        let bobs = graph_len(&store, "https://pod.ex/bob/b1");
        let err = entry(&mut store, &sess(BOB), attack).expect_err(name);
        assert!(err.contains("denied"), "{name}: {err}");
        assert_eq!(graph_len(&store, "https://pod.ex/.acl"), rules, "{name}");
        // Nothing of the request was applied, its permitted first operation included.
        assert_eq!(graph_len(&store, "https://pod.ex/bob/b1"), bobs, "{name}");
        assert!(reads(&store, ALICE, NOTE), "{name}");
    }
}

/// Every WAC entry point that changes the rules leaves the view describing them: a
/// revocation takes effect at once.
#[test]
fn every_wac_entry_point_rebuilds_the_view_it_changes() {
    let revoke = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
        <https://pod.ex/.acl#alice> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> } }";
    for (name, entry) in ENTRY_POINTS.iter().filter(|(n, _)| !n.contains("acp")) {
        let mut store = store();
        assert!(reads(&store, ALICE, NOTE), "{name}");
        entry(&mut store, &sess(ADMIN), revoke).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            !reads(&store, ALICE, NOTE),
            "{name}: the revocation did not take effect"
        );
        assert!(reads(&store, ADMIN, NOTE), "{name}");
    }
}

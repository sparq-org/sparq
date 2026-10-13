//! Which path [`PodStore::update_as`] takes (issue #6796): a single `DELETE`/`INSERT … WHERE`
//! whose targets leave the rules alone commits its effects without forking the store, and a
//! refusal on that path leaves the store exactly as it was; an update that touches the rules
//! still runs on a fork first.

use super::*;
use std::cell::Cell;

thread_local! {
    /// How many updates on this thread ran on a fork of the store graph.
    pub(crate) static FORKS: Cell<usize> = const { Cell::new(0) };
}

fn forks() -> usize {
    FORKS.with(Cell::get)
}

const N1: &str = "https://pod.ex/notes/n1";
const ACL: &str = "https://pod.ex/.acl";
const TITLE: &str = "https://ex.dev/ns#title";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// One document; Alice holds Read/Write/Control on the pod, Bob Read/Append.
fn pod() -> PodStore {
    let a = "http://www.w3.org/ns/auth/acl#";
    let mut nq = format!("<{N1}#it> <{TITLE}> \"old\" <{N1}> .\n");
    for (rule, agent, modes) in [
        ("alice", ALICE, &["Read", "Write", "Control"][..]),
        ("bob", BOB, &["Read", "Append"]),
    ] {
        let r = format!("{ACL}#{rule}");
        nq += &format!("<{r}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{a}Authorization> <{ACL}> .\n");
        nq += &format!("<{r}> <{a}accessTo> <https://pod.ex/> <{ACL}> .\n");
        nq += &format!("<{r}> <{a}default> <https://pod.ex/> <{ACL}> .\n");
        nq += &format!("<{r}> <{a}agent> <{agent}> <{ACL}> .\n");
        for m in modes {
            nq += &format!("<{r}> <{a}mode> <{a}{m}> <{ACL}> .\n");
        }
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    store
}

fn session(agent: &str) -> Session<'_> {
    Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    }
}

/// Every quad of the store (the reserved auth graph included) plus the pending delta size: a
/// refused update must leave both exactly as they were.
fn snapshot(graph: &Graph) -> (Vec<String>, usize) {
    fn dump(g: &Graph, name: &str, out: &mut Vec<String>) {
        let scan = g.store.scan(&[None, None, None]);
        for r in scan.rows.iter() {
            let [s, p, o] = scan.to_spo(r);
            out.push(format!(
                "{name} {} {} {}",
                g.dict.term(s),
                g.dict.term(p),
                g.dict.term(o)
            ));
        }
    }
    let mut out = Vec::new();
    dump(graph, "", &mut out);
    for (name, g) in &graph.named {
        out.push(format!("graph {name}"));
        dump(g, &name.to_string(), &mut out);
    }
    out.sort();
    (out, graph.pending_delta_len())
}

/// The LDP PATCH shape: replace N1's title.
fn patch(to: &str) -> String {
    format!(
        "DELETE {{ GRAPH <{N1}> {{ <{N1}#it> <{TITLE}> ?t }} }} \
         INSERT {{ GRAPH <{N1}> {{ <{N1}#it> <{TITLE}> \"{to}\" }} }} \
         WHERE {{ GRAPH <{N1}> {{ <{N1}#it> <{TITLE}> ?t }} }}"
    )
}

fn titles(store: &PodStore) -> Vec<String> {
    let q = format!("SELECT ?t WHERE {{ GRAPH <{N1}> {{ <{N1}#it> <{TITLE}> ?t }} }}");
    let r = sparq_engine::query(&store.graph, &q).expect("query");
    r.rows
        .iter()
        .map(|row| row[0].as_ref().expect("bound").to_string())
        .collect()
}

#[test]
fn a_permitted_where_update_commits_without_a_fork() {
    let mut store = pod();
    // The engine's own in-place apply of the same update is the reference result.
    let mut reference = store.graph.fork();
    sparq_engine::update_in_place(&mut reference, &patch("new")).expect("reference apply");
    let before = forks();
    store
        .update_as(&session(ALICE), &patch("new"))
        .expect("alice may write");
    assert_eq!(
        forks(),
        before,
        "a PATCH that leaves the rules alone took the fork"
    );
    assert_eq!(titles(&store), ["\"new\""]);
    assert_eq!(
        snapshot(&store.graph).0,
        snapshot(&reference).0,
        "fast path diverges from the in-place apply"
    );
}

#[test]
fn a_where_update_matching_nothing_changes_nothing() {
    let mut store = pod();
    let before = (snapshot(&store.graph), forks());
    let upd = format!(
        "DELETE {{ GRAPH <{N1}> {{ ?s ?p ?o }} }} WHERE {{ GRAPH <{N1}> {{ ?s ?p ?o FILTER(false) }} }}"
    );
    store
        .update_as(&session(ALICE), &upd)
        .expect("alice may write");
    assert_eq!((snapshot(&store.graph), forks()), before);
}

#[test]
fn a_refused_where_update_on_the_fast_path_changes_nothing() {
    let mut store = pod();
    let before = (snapshot(&store.graph), forks());

    // Over budget: the WHERE is evaluated (on the live store, read-only) and aborts.
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let cancelled = QueryBudget::cancelled_by(flag);
    let e = store
        .update_as_with_budget(&session(ALICE), &patch("new"), &cancelled)
        .expect_err("a cancelled budget aborts the update");
    assert!(e.contains("query budget exceeded"), "{e}");
    assert_eq!(
        (snapshot(&store.graph), forks()),
        before,
        "an aborted PATCH changed the store"
    );

    // Denied: Bob may only append, and a PATCH deletes.
    let e = store
        .update_as(&session(BOB), &patch("bob"))
        .expect_err("bob may not delete");
    assert!(e.contains("update denied"), "{e}");
    assert_eq!(
        (snapshot(&store.graph), forks()),
        before,
        "a denied PATCH changed the store"
    );

    // Denied: the WHERE reads a graph nobody may read.
    let upd = format!(
        "INSERT {{ GRAPH <{N1}> {{ <{N1}#it> <{TITLE}> ?o }} }} \
         WHERE {{ GRAPH <https://elsewhere.ex/secret> {{ ?s ?p ?o }} }}"
    );
    let e = store
        .update_as(&session(ALICE), &upd)
        .expect_err("an unreadable WHERE is denied");
    assert!(e.contains("update denied"), "{e}");
    assert_eq!(
        (snapshot(&store.graph), forks()),
        before,
        "a denied PATCH changed the store"
    );

    // The store still takes a permitted PATCH afterwards.
    store
        .update_as(&session(ALICE), &patch("new"))
        .expect("alice may write");
    assert_eq!(titles(&store), ["\"new\""]);
}

#[test]
fn a_where_update_touching_the_rules_takes_the_fork() {
    let mut store = pod();
    let a = "http://www.w3.org/ns/auth/acl#";
    // Promote Bob from Append to Write: the PATCH shape, on the pod's `.acl`.
    let upd = format!(
        "DELETE {{ GRAPH <{ACL}> {{ ?r <{a}mode> <{a}Append> }} }} \
         INSERT {{ GRAPH <{ACL}> {{ ?r <{a}mode> <{a}Write> }} }} \
         WHERE {{ GRAPH <{ACL}> {{ ?r <{a}agent> <{BOB}> ; <{a}mode> <{a}Append> }} }}"
    );
    let before = forks();
    store
        .update_as(&session(ALICE), &upd)
        .expect("alice controls the pod");
    assert_eq!(
        forks(),
        before + 1,
        "a rule-changing PATCH skipped the fork"
    );
    // The rebuilt view grants what the new rules say.
    store
        .update_as(&session(BOB), &patch("bob"))
        .expect("bob now holds Write");
    assert_eq!(titles(&store), ["\"bob\""]);
}

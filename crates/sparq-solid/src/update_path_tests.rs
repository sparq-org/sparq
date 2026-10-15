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

thread_local! {
    /// Whether the next commit on this thread fails after applying its effects.
    static FAIL_COMMIT: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    /// Whether the next commit on this thread is refused at its commit point, before it
    /// applies anything.
    pub(crate) static FAIL_BEFORE_COMMIT: Cell<bool> = const { Cell::new(false) };
}

/// The injected refusal at the commit point, consumed by the commit it refuses.
pub(crate) fn injected_failure_before_commit() -> Result<(), String> {
    if FAIL_BEFORE_COMMIT.with(|f| f.replace(false)) {
        return Err("injected refusal at the commit point".to_owned());
    }
    Ok(())
}

/// The injected commit failure, consumed by the commit it fails.
pub(crate) fn injected_commit_failure() -> Result<(), String> {
    if FAIL_COMMIT.with(|f| f.replace(false)) {
        return Err("injected commit failure".to_owned());
    }
    Ok(())
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

/// An ACP pod whose `/docs/` members grant their creator Read and Write, and ADMIN Write;
/// alice created d0.
fn creator_pod() -> PodStore {
    let (acp, acl) = ("http://www.w3.org/ns/solid/acp#", "http://www.w3.org/ns/auth/acl#");
    let acr = "https://pod.ex/docs/.acr";
    let nq = format!(
        "<{D0}#it> <{TITLE}> \"v\" <{D0}> .\n\
         <{acr}> <{acp}memberAccessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#creator> <{acr}> .\n\
         <{acr}#creator> <{acp}allow> <{acl}Read> <{acr}> .\n\
         <{acr}#creator> <{acp}allow> <{acl}Write> <{acr}> .\n\
         <{acr}#creator> <{acp}anyOf> <{acr}#mc> <{acr}> .\n\
         <{acr}#mc> <{acp}agent> <{acp}CreatorAgent> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#admin> <{acr}> .\n\
         <{acr}#admin> <{acp}allow> <{acl}Write> <{acr}> .\n\
         <{acr}#admin> <{acp}anyOf> <{acr}#ma> <{acr}> .\n\
         <{acr}#ma> <{acp}agent> <{ADMIN}> <{acr}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("pod loads"));
    let mut prov = AccessProvenance::new();
    prov.set_creator(D0, ALICE);
    store.materialize_acp_with(&prov).expect("acp");
    store
}

const D0: &str = "https://pod.ex/docs/d0";
const ADMIN: &str = "https://admin.ex/card#me";

fn reads_d0(store: &PodStore, agent: &str) -> bool {
    store.decide(&session(agent), D0, Mode::Read).allow
}

/// A commit that fails after applying part of an update fails closed even when the update
/// would leave every resource in place (here d0 dropped and created again): an
/// intermediate step may be what was applied.
#[test]
fn a_partly_applied_commit_fails_closed_whatever_its_net_effect() {
    for update in [
        format!("INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"),
        format!(
            "DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"
        ),
    ] {
        let mut store = creator_pod();
        let before = store.acp_provenance().clone();
        FAIL_COMMIT.with(|f| f.set(true));
        store
            .update_as_acp(&session(ADMIN), &update)
            .expect_err("the commit fails");
        assert_eq!(
            *store.acp_provenance(),
            before,
            "{update}: nothing installed, and d0, there before and after, keeps its facts"
        );
        assert!(!reads_d0(&store, ALICE), "{update}: the view is gone");
        assert!(
            store.materialize_acp().is_err(),
            "{update}: the stored context is not used"
        );
        store
            .materialize_acp_with(&before)
            .expect("the caller supplies the context");
        assert!(reads_d0(&store, ALICE), "{update}: and decisions resume");
    }
}

/// After a commit that fails past its commit point, a resource the update touched that
/// exists before it and after the failure keeps its creator and owner, so a deny keyed on
/// them still applies once the provenance is passed back.
#[test]
fn a_failed_update_keeps_the_facts_of_a_resource_there_throughout() {
    const ERIN: &str = "https://erin.ex/card#me";
    let (acp, acl) = ("http://www.w3.org/ns/solid/acp#", "http://www.w3.org/ns/auth/acl#");
    let acr = "https://pod.ex/docs/.acr";
    let nq = format!(
        "<{D0}#it> <{TITLE}> \"v\" <{D0}> .\n\
         <{acr}> <{acp}memberAccessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#all> <{acr}> .\n\
         <{acr}#all> <{acp}allow> <{acl}Read> <{acr}> .\n\
         <{acr}#all> <{acp}anyOf> <{acr}#mp> <{acr}> .\n\
         <{acr}#mp> <{acp}agent> <{acp}PublicAgent> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#deny> <{acr}> .\n\
         <{acr}#deny> <{acp}deny> <{acl}Read> <{acr}> .\n\
         <{acr}#deny> <{acp}anyOf> <{acr}#md> <{acr}> .\n\
         <{acr}#md> <{acp}agent> <{acp}CreatorAgent> <{acr}> .\n\
         <{acr}#md> <{acp}agent> <{acp}OwnerAgent> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#admin> <{acr}> .\n\
         <{acr}#admin> <{acp}allow> <{acl}Write> <{acr}> .\n\
         <{acr}#admin> <{acp}anyOf> <{acr}#ma> <{acr}> .\n\
         <{acr}#ma> <{acp}agent> <{ADMIN}> <{acr}> .\n"
    );
    let mk = || {
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("pod loads"));
        let mut prov = AccessProvenance::new();
        prov.set_creator(D0, ALICE);
        prov.set_owner(D0, ERIN);
        store.materialize_acp_with(&prov).expect("acp");
        assert!(!reads_d0(&store, ALICE) && !reads_d0(&store, ERIN), "the deny applies");
        store
    };
    for update in [
        format!("INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"),
        format!(
            "DELETE DATA {{ GRAPH <{D0}> {{ <{D0}#it> <{TITLE}> \"v\" }} }} ; \
             INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"
        ),
        format!("CLEAR ALL ; INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"),
    ] {
        let mut store = mk();
        let before = store.acp_provenance().clone();
        FAIL_COMMIT.with(|f| f.set(true));
        store.update_as_acp(&session(ADMIN), &update).expect_err("the commit fails");
        let back = store.acp_provenance().clone();
        assert_eq!(back, before, "{update}: d0 keeps its facts");
        store.materialize_acp_with(&back).expect("passed back");
        assert!(!reads_d0(&store, ALICE), "{update}: the creator deny still applies");
        assert!(!reads_d0(&store, ERIN), "{update}: the owner deny still applies");
    }
}

/// A failed commit of an update that would have moved the provenance (here d0 dropped, so
/// alice is no longer its creator) leaves the store holding an unknown part of it: the view
/// goes, and no rebuild uses the stored provenance until the caller supplies the context
/// again.
#[test]
fn a_failed_commit_that_moves_provenance_fails_closed_until_the_context_is_supplied() {
    let mut store = creator_pod();
    FAIL_COMMIT.with(|f| f.set(true));
    let update = format!("DROP GRAPH <{D0}>");
    store.update_as_acp(&session(ADMIN), &update).expect_err("the commit fails");
    assert_eq!(
        *store.acp_provenance(),
        AccessProvenance::new(),
        "nothing staged installed, and the resource it touched keeps nothing"
    );
    assert!(!reads_d0(&store, ALICE), "the view is gone");
    assert!(store.materialize_acp().is_err(), "the stored context is not used");
    assert!(
        store.update_as_acp(&session(ADMIN), &update).is_err(),
        "nor is any update authorized"
    );
    store
        .materialize_acp_with(&AccessProvenance::new())
        .expect("the caller supplies the context");
    store.materialize_acp().expect("the stored context is usable again");
}

/// A commit refused at its commit point applies nothing, so the view and the stored context
/// stay as they were even when the update would have moved the provenance.
#[test]
fn a_commit_refused_before_it_applies_anything_keeps_the_context_and_the_view() {
    let mut store = creator_pod();
    let before = store.acp_provenance().clone();
    FAIL_BEFORE_COMMIT.with(|f| f.set(true));
    store
        .update_as_acp(&session(ADMIN), &format!("DROP GRAPH <{D0}>"))
        .expect_err("the commit is refused");
    assert_eq!(*store.acp_provenance(), before);
    assert!(reads_d0(&store, ALICE), "the view stays, d0 with it");
    store
        .materialize_acp()
        .expect("the stored context is still usable");
    assert!(reads_d0(&store, ALICE));
}

/// An update that drops a resource and creates it again keeps its creator: an update never
/// records its agent as one.
#[test]
fn an_update_never_takes_over_a_resource() {
    let mut store = creator_pod();
    let update = format!(
        "DROP GRAPH <{D0}> ; INSERT DATA {{ GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}"
    );
    store
        .update_as_acp(&session(ADMIN), &update)
        .expect("ADMIN may replace d0");
    assert_eq!(store.acp_provenance().creator(D0), Some(ALICE));
    assert!(!reads_d0(&store, ADMIN), "ADMIN gains nothing");
    assert!(reads_d0(&store, ALICE));
}

/// Under WAC, a partly applied commit leaves the store's own WAC rebuild usable: it uses no
/// provenance.
#[test]
fn a_failed_commit_under_wac_leaves_its_rebuild_usable() {
    let mut store = pod();
    let n2 = "https://pod.ex/notes/n2";
    assert!(
        store.decide(&session(BOB), N1, Mode::Read).allow,
        "control: bob reads n1"
    );
    FAIL_COMMIT.with(|f| f.set(true));
    store
        .update_as(
            &session(ALICE),
            &format!("INSERT DATA {{ GRAPH <{n2}> {{ <{n2}#x> <{TITLE}> \"y\" }} }}"),
        )
        .expect_err("the commit fails");
    let names = store.stored_graphs();
    store
        .rematerialize_after_edit(&names, Some(ALICE), false)
        .expect("a WAC rebuild does not use the provenance");
    assert!(
        store.decide(&session(BOB), N1, Mode::Read).allow,
        "bob reads n1"
    );
}

/// A failed commit of an update that drops and stores again d0.acr while d0 does not exist,
/// then the stored provenance passed back: d0 made again has no earlier creator.
#[test]
fn a_failed_update_leaves_no_acr_kept_for_provenance_passed_back() {
    let acr = format!("{D0}.acr");
    let mk = || {
        let mut store = creator_pod();
        store
            .put_acl_acp(&acr, &format!("<{acr}> <http://www.w3.org/ns/solid/acp#accessControl> <{acr}#c> ."), "ntriples")
            .expect("d0.acr");
        let aa = format!("{acr}.acr");
        let a = "http://www.w3.org/ns/solid/acp#";
        let l = "http://www.w3.org/ns/auth/acl#";
        store
            .put_acl_acp(&aa, &format!("<{aa}> <{a}accessControl> <{aa}#c> .\n<{aa}#c> <{a}apply> <{aa}#p> .\n<{aa}#p> <{a}allow> <{l}Read> .\n<{aa}#p> <{a}allow> <{l}Write> .\n<{aa}#p> <{a}allow> <{l}Control> .\n<{aa}#p> <{a}anyOf> <{aa}#m> .\n<{aa}#m> <{a}agent> <{ADMIN}> .\n"), "ntriples")
            .expect("d0.acr.acr");
        store.update_as_acp(&session(ADMIN), &format!("DROP GRAPH <{D0}>")).expect("drop d0");
        assert_eq!(store.acp_provenance().creator(D0), Some(ALICE), "kept with d0.acr");
        store
    };
    let update = format!(
        "DROP GRAPH <{acr}> ; INSERT DATA {{ GRAPH <{acr}> {{ <{acr}> <http://www.w3.org/ns/solid/acp#accessControl> <{acr}#c> }} }}"
    );
    let mut ok = mk();
    ok.update_as_acp(&session(ADMIN), &update).expect("the update commits");
    assert_eq!(ok.acp_provenance().creator(D0), None, "the drop ended it");
    let mut store = mk();
    FAIL_COMMIT.with(|f| f.set(true));
    store.update_as_acp(&session(ADMIN), &update).expect_err("the commit fails");
    let back = store.acp_provenance().clone();
    store.materialize_acp_with(&back).expect("passed back");
    let names = store.stored_graphs();
    store.graph.named.extend(Graph::load_dataset(&format!("<{D0}#x> <{TITLE}> \"y\" <{D0}> .\n"), "nquads").expect("nq").named);
    store.rematerialize_after_edit(&names, None, true).expect("d0 again");
    assert_eq!(store.acp_provenance().creator(D0), None);
    assert!(!reads_d0(&store, ALICE), "no creator grant");
}

/// A commit that fails after its commit point leaves no fact of a resource the update touched
/// for the provenance passed back, so a resource the update brought back keeps nothing.
#[test]
fn a_failed_update_leaves_no_fact_of_a_resource_it_touched_for_provenance_passed_back() {
    let acr = format!("{D0}.acr");
    let a = "http://www.w3.org/ns/solid/acp#";
    let l = "http://www.w3.org/ns/auth/acl#";
    let policy = format!("<{acr}> <{a}accessControl> <{acr}#c> . <{acr}#c> <{a}apply> <{acr}#p> . <{acr}#p> <{a}allow> <{l}Write> . <{acr}#p> <{a}anyOf> <{acr}#m> . <{acr}#m> <{a}agent> <{ADMIN}> .");
    let mk = || {
        let mut store = creator_pod();
        store.put_acl_acp(&acr, &format!("{}\n<{acr}#p> <{a}allow> <{l}Read> .\n", policy.replace(" . ", " .\n")), "ntriples").expect("d0.acr");
        let aa = format!("{acr}.acr");
        store
            .put_acl_acp(&aa, &format!("<{aa}> <{a}accessControl> <{aa}#c> .\n<{aa}#c> <{a}apply> <{aa}#p> .\n<{aa}#p> <{a}allow> <{l}Read> .\n<{aa}#p> <{a}allow> <{l}Write> .\n<{aa}#p> <{a}allow> <{l}Control> .\n<{aa}#p> <{a}anyOf> <{aa}#m> .\n<{aa}#m> <{a}agent> <{ADMIN}> .\n"), "ntriples")
            .expect("d0.acr.acr");
        store.update_as_acp(&session(ADMIN), &format!("DROP GRAPH <{D0}>")).expect("drop d0");
        assert_eq!(store.acp_provenance().creator(D0), Some(ALICE), "kept with d0.acr");
        store
    };
    let update = format!("DROP GRAPH <{acr}> ; INSERT DATA {{ GRAPH <{acr}> {{ {policy} }} GRAPH <{D0}> {{ <{D0}#x> <{TITLE}> \"y\" }} }}");
    let mut ok = mk();
    ok.update_as_acp(&session(ADMIN), &update).expect("the update commits");
    assert_eq!(ok.acp_provenance().creator(D0), None, "the drop ended it");
    let mut store = mk();
    FAIL_COMMIT.with(|f| f.set(true));
    store.update_as_acp(&session(ADMIN), &update).expect_err("the commit fails");
    let back = store.acp_provenance().clone();
    store.materialize_acp_with(&back).expect("passed back");
    assert_eq!(store.acp_provenance().creator(D0), None);
    assert!(!reads_d0(&store, ALICE), "no creator grant");
}

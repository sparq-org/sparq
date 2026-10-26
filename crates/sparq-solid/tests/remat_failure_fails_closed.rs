//! A permitted write that changes the access-control rules re-materializes the auth view.
//! The view is rebuilt from the rules the write would leave before the write is committed:
//! rules that cannot be materialized refuse the write, which then changes nothing, so the
//! current view still describes the current rules. An ACL naming a reserved `urn:sparq:`
//! agent is rejected whole by the loader instead, so it fails nothing. A write that fails
//! part way changes nothing either, and a controller of one access-control document cannot
//! take the store's whole view down.

use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

const ADMIN: &str = "https://admin.ex/card#me";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const NOTE: &str = "https://pod.ex/notes/n1";

fn sess(agent: &str) -> Session<'_> {
    Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    }
}

fn reads(store: &PodStore, agent: &str) -> bool {
    store
        .accessible(&sess(agent), Mode::Read)
        .iter()
        .any(|g| g == NOTE)
}

fn store() -> PodStore {
    let nq = concat!(
        // Content.
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n",
        "<https://pod.ex/other#it> <https://ex.dev/ns#title> \"other\" <https://pod.ex/other> .\n",
        // The group document (an ORDINARY resource IRI — no .acl/.acr suffix).
        "<https://pod.ex/groups#team> <http://www.w3.org/2006/vcard/ns#hasMember> \
            <https://alice.ex/card#me> <https://pod.ex/groups> .\n",
        // Root ACL: admin owns the pod.
        "<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> \
            <https://admin.ex/card#me> <https://pod.ex/.acl> .\n",
        // ...and controls the root itself, so it may write the root ACL.
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .\n",
        // Root ACL: the team group reads the pod.
        "<https://pod.ex/.acl#team> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> \
            <https://pod.ex/groups#team> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        // Bob controls his own container, and only it.
        "<https://pod.ex/bob/b1#it> <https://ex.dev/ns#title> \"bob's\" <https://pod.ex/bob/b1> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
            <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#accessTo> \
            <https://pod.ex/bob/> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#default> \
            <https://pod.ex/bob/> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#agent> \
            <https://bob.ex/card#me> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/bob/.acl> .\n",
        "<https://pod.ex/bob/.acl#own> <http://www.w3.org/ns/auth/acl#mode> \
            <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/bob/.acl> .\n",
    );
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    s.materialize_wac().expect("wac materializes");
    s
}

/// Revoke the team's Read and, in the same write, name a reserved agent: rules the view
/// cannot be rebuilt from.
const REVOKE_AND_BREAK: &str = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> } } ; \
    INSERT DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";

const REVOKE: &str = "DELETE DATA { GRAPH <https://pod.ex/.acl> { \
    <https://pod.ex/.acl#team> <http://www.w3.org/ns/auth/acl#agentGroup> <https://pod.ex/groups#team> } }";

/// An ACL write naming a reserved agent is refused before anything changes, so the writer
/// keeps the rules (and the Control) it had; the same revocation without it goes through.
#[test]
fn rules_naming_a_reserved_agent_are_refused() {
    let mut store = store();
    assert!(reads(&store, ALICE), "control: the team reads");
    let err = store
        .update_as(&sess(ADMIN), REVOKE_AND_BREAK)
        .expect_err("the write is refused");
    assert!(err.contains("may not state"), "{err}");
    assert!(reads(&store, ALICE), "nothing changed");
    assert!(reads(&store, ADMIN));
    store.update_as(&sess(ADMIN), REVOKE).expect("sound rules");
    assert!(!reads(&store, ALICE), "the revocation took effect");
    assert!(reads(&store, ADMIN));
}

/// Loaded data naming a reserved agent in an ACL is rejected whole by the loader instead: the
/// document governs with no authorizations (fail-closed), and nothing else is affected.
#[test]
fn a_loaded_acl_naming_a_reserved_agent_governs_with_no_grants() {
    let mut store = store();
    let iri = |s: &str| oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s));
    let slot = store
        .graph_unchecked_mut()
        .named()
        .iter()
        .position(|(n, _)| *n == iri("https://pod.ex/bob/.acl"))
        .expect("bob's acl");
    let _ = store.graph_unchecked_mut().apply_delta_to_named(
        slot,
        &[[
            iri("https://pod.ex/bob/.acl#bad"),
            iri("http://www.w3.org/ns/auth/acl#agent"),
            iri("urn:sparq:forged"),
        ]],
        &[],
    );
    store.materialize_wac().expect("rejected, not fatal");
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert!(store.accessible(&sess("urn:sparq:forged"), Mode::Read).is_empty());
    assert!(
        !store
            .accessible(&sess(BOB), Mode::Read)
            .iter()
            .any(|g| g == "https://pod.ex/bob/b1"),
        "bob's rejected ACL governs his container with no grants"
    );
}

/// Review finding: a rule change that failed part way (here a later operation over its
/// budget) left its earlier operations applied without the view rebuilt from them. A rule
/// change is applied whole or not at all.
#[test]
fn a_rule_change_that_fails_part_way_changes_nothing() {
    let mut store = store();
    let mut budget = sparq_engine::QueryBudget::unlimited();
    budget.max_rows = Some(0);
    let revoke_then_overrun = format!(
        "{REVOKE} ; INSERT {{ GRAPH <{NOTE}> {{ ?s <https://ex.dev/ns#copy> ?o }} }} \
         WHERE {{ GRAPH <{NOTE}> {{ ?s ?p ?o }} }}"
    );
    store
        .update_as_with_budget(&sess(ADMIN), &revoke_then_overrun, &budget)
        .expect_err("the second operation is over budget");
    // The revocation was not applied, and the view still grants what the rules grant.
    assert!(reads(&store, ALICE));
    store.update_as(&sess(ADMIN), REVOKE).expect("sound rules");
    assert!(!reads(&store, ALICE));
}

/// Review finding: a controller of one access-control document could write rules that fail
/// to materialize, and the store dropped its whole view, denying every pod. Such a write is
/// refused; everyone else's access, and bob's own, is untouched.
#[test]
fn one_controller_cannot_deny_the_whole_store() {
    let mut store = store();
    let break_own = "INSERT DATA { GRAPH <https://pod.ex/bob/.acl> { \
        <https://pod.ex/bob/.acl#bad> <http://www.w3.org/ns/auth/acl#agent> <urn:sparq:forged> } }";
    store
        .update_as(&sess(BOB), break_own)
        .expect_err("the write is refused");
    assert!(reads(&store, ALICE));
    assert!(reads(&store, ADMIN));
    assert!(store
        .accessible(&sess(BOB), Mode::Read)
        .iter()
        .any(|g| g == "https://pod.ex/bob/b1"));
}

/// Under ACP, a controller cannot make its own access-control resource use a policy of one
/// the store already rejects (which would reject it too, and with it the controller's
/// Control): the write is refused, by update and by replace, and nothing changes. The refusal
/// names the statement the writer sent and nothing about the state of the document it reaches
/// into, which is a fact about another resource.
#[test]
fn an_acr_cannot_take_up_a_rejected_documents_policy() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let r = "https://pod.ex/r";
    let acr = format!("{r}.acr");
    let other = "https://pod.ex/other.acr";
    let nq = format!(
        "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Control> <{acr}> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> <{acr}> .\n\
         <{acr}.acr> <{ACP}accessControl> <{acr}.acr#c> <{acr}.acr> .\n\
         <{acr}.acr#c> <{ACP}apply> <{acr}.acr#p> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}allow> <{ACL}Write> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}anyOf> <{acr}.acr#m> <{acr}.acr> .\n\
         <{acr}.acr#m> <{ACP}agent> <{ADMIN}> <{acr}.acr> .\n\
         <{other}#p> <{ACP}allow> <{ACL}Read> <{other}> .\n\
         <https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{other}> .\n"
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    let decisions = |store: &PodStore| {
        [r, acr.as_str(), other]
            .iter()
            .flat_map(|target| {
                [Mode::Read, Mode::Write, Mode::Append, Mode::Control]
                    .map(|mode| store.decide(&sess(ADMIN), target, mode).allow)
            })
            .collect::<Vec<bool>>()
    };
    let (before, stored) = (decisions(&store), snapshot(store.graph()));
    assert!(store.decide(&sess(ADMIN), r, Mode::Control).allow, "control: ADMIN controls r");
    let link = format!("<{acr}#c> <{ACP}apply> <{other}#p>");
    let err = store
        .update_as_acp(
            &sess(ADMIN),
            &format!("INSERT DATA {{ GRAPH <{acr}> {{ {link} }} }}"),
        )
        .expect_err("the update is refused");
    assert_refusal_says_nothing_of_the_other_document(&err, &acr, other);
    assert_eq!(decisions(&store), before, "no decision changed");
    assert_eq!(snapshot(store.graph()), stored, "nothing was stored");
    // Added and removed again in the same update, it leaves nothing to refuse.
    store
        .update_as_acp(
            &sess(ADMIN),
            &format!(
                "INSERT DATA {{ GRAPH <{acr}> {{ {link} }} }} ; \
                 DELETE DATA {{ GRAPH <{acr}> {{ {link} }} }}"
            ),
        )
        .expect("nothing it leaves uses the rejected document");
    assert_eq!(decisions(&store), before);
    assert_eq!(snapshot(store.graph()), stored);
    let doc = format!(
        "<{acr}> <{ACP}accessControl> <{acr}#c> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> .\n\
         {link} .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Write> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Control> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> .\n"
    );
    let err = store
        .put_acl_acp(&acr, &doc, "ntriples")
        .expect_err("the replace is refused");
    assert_refusal_says_nothing_of_the_other_document(&err, &acr, other);
    assert_eq!(decisions(&store), before, "no decision changed");
    assert_eq!(snapshot(store.graph()), stored, "nothing was stored");
}

/// A refusal for a write reaching into another access-control document is ONE uniform body:
/// it names the document written and nothing else — not the statement, not the reference, and
/// nothing about the state of the document reached into, which is a fact about another
/// resource.
fn assert_refusal_says_nothing_of_the_other_document(err: &str, doc: &str, other: &str) {
    assert_eq!(
        err.split_once(": ").map(|(_, body)| body),
        Some(uniform_dependency_refusal(doc).as_str()),
        "{}",
        err
    );
    for leak in ["not admitted", "rejected", "reject", other] {
        assert!(!err.contains(leak), "the refusal states `{leak}`: {err}");
    }
}

/// The one refusal body for a write into `doc` that uses a node of another access-control
/// document, whichever statement and whichever document that is.
fn uniform_dependency_refusal(doc: &str) -> String {
    format!(
        "the access-control document <{}> may not use a node of another access-control \
         document; nothing was changed",
        doc
    )
}

/// Every stored quad outside the reserved space, sorted.
fn snapshot(g: &Graph) -> Vec<String> {
    let mut quads = Vec::new();
    for (name, sub) in g.named() {
        if name.to_string().starts_with("<urn:sparq:") {
            continue;
        }
        let scan = sub.store().scan(&[None, None, None]);
        for row in scan.rows.iter() {
            let t = scan.to_spo(row);
            quads.push(format!(
                "{} {} {} {name}",
                sub.dict().term(t[0]),
                sub.dict().term(t[1]),
                sub.dict().term(t[2])
            ));
        }
    }
    quads.sort();
    quads
}

/// The same check refuses nothing it need not: a reference to a sound access-control
/// resource, the repair of a rejected one that references one rejected only through it,
/// and, in a WAC store, any `.acr` (which the WAC view does not read).
#[test]
fn an_acr_may_take_up_a_sound_policy_and_be_repaired() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let (r, e) = ("https://pod.ex/r", "https://pod.ex/e");
    let (racr, eacr) = (format!("{r}.acr"), format!("{e}.acr"));
    let body = |extra: &str| {
        format!(
            "<{racr}> <{ACP}accessControl> <{racr}#c> .\n\
             <{racr}#c> <{ACP}apply> <{racr}#p> .\n\
             <{racr}#p> <{ACP}allow> <{ACL}Read> .\n\
             <{racr}#p> <{ACP}anyOf> <{racr}#m> .\n\
             <{racr}#m> <{ACP}agent> <{ADMIN}> .\n{extra}"
        )
    };
    let quads = |doc: &str, nt: &str| -> String {
        nt.lines()
            .map(|l| format!("{} <{doc}> .\n", l.trim_end_matches(" .")))
            .collect()
    };
    let e_doc = format!(
        "<{eacr}> <{ACP}accessControl> <{eacr}#c> .\n\
         <{eacr}#c> <{ACP}apply> <{eacr}#p> .\n\
         <{eacr}#p> <{ACP}allow> <{ACL}Read> .\n\
         <{eacr}#p> <{ACP}anyOf> <{racr}#m> .\n"
    );
    let data = format!(
        "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n<{e}#it> <https://ex.dev/ns#k> \"v\" <{e}> .\n"
    );
    let link = format!("<{racr}#c> <{ACP}apply> <{eacr}#p> .\n");
    // A reference to a sound access-control resource is accepted.
    let nq = format!("{data}{}{}", quads(&racr, &body("")), quads(&eacr, &e_doc));
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    store
        .put_acl_acp(&racr, &body(&link), "ntriples")
        .expect("a sound reference is accepted");
    // A rejected r.acr (and e.acr, rejected only through it) is repaired in one write.
    let stray = format!("<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{racr}> .\n");
    let nq = format!(
        "{data}{}{stray}{}",
        quads(&racr, &body("")),
        quads(&eacr, &e_doc)
    );
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    store
        .put_acl_acp(&racr, &body(&link), "ntriples")
        .expect("the repair is accepted");
    assert!(
        store.decide(&sess(ADMIN), r, Mode::Read).allow,
        "r is governed again"
    );
    // A WAC store does not read .acr documents.
    let bad = "https://pod.ex/bad.acr";
    let nq = format!("{data}<{bad}#p> <{ACP}allow> <{ACL}Read> <{bad}> .\n<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{bad}> .\n");
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_wac().expect("materializes");
    store
        .put_acl(
            &racr,
            &body(&format!("<{racr}#c> <{ACP}apply> <{bad}#p> .\n")),
            "ntriples",
        )
        .expect("no ACP check in a WAC store");
    // Nor a statement about another document's node in a .acr, which the WAC loader filters
    // per triple; and an ACP store likewise leaves a .acl to its loader.
    store
        .put_acl(
            &racr,
            &body("<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" .\n"),
            "ntriples",
        )
        .expect("a .acr is not a WAC control document");
    let nq = format!("{data}{}", quads(&racr, &body("")));
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");
    store
        .put_acl_acp(
            &format!("{r}.acl"),
            "<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" .\n",
            "ntriples",
        )
        .expect("a .acl is not an ACP control document");
}

/// A rejected access control resource can be replaced, whether it is rejected for what it
/// states itself or only through what it uses, by content that uses nothing rejected; content
/// still using a rejected document is refused.
#[test]
fn an_acr_rejected_by_itself_or_through_another_can_be_replaced() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let r = "https://pod.ex/r";
    let acr = format!("{r}.acr");
    let bad = "https://pod.ex/bad.acr";
    let sound = format!(
        "<{acr}> <{ACP}accessControl> <{acr}#c> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> .\n"
    );
    let quads = |nt: &str| -> String {
        nt.lines()
            .map(|l| format!("{} <{acr}> .\n", l.trim_end_matches(" .")))
            .collect()
    };
    let base = format!(
        "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n\
         <{bad}#p> <{ACP}allow> <{ACL}Read> <{bad}> .\n\
         <https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{bad}> .\n"
    );
    let uses_bad = format!("<{acr}#c> <{ACP}apply> <{bad}#p> .\n");
    let seeds = [
        ("by itself", format!("<https://elsewhere.ex/y#n> <https://ex.dev/ns#t> \"t\" <{acr}> .\n")),
        ("through bad.acr", quads(&uses_bad)),
    ];
    for (why, seed) in seeds {
        let nq = format!("{base}{}{seed}", quads(&sound));
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
        store.materialize_acp().expect("materializes");
        assert!(!store.decide(&sess(ADMIN), r, Mode::Read).allow, "rejected {why}");
        let err = store
            .put_acl_acp(&acr, &format!("{sound}{uses_bad}"), "ntriples")
            .expect_err("still using bad.acr");
        assert!(err.contains("may not use a node of another"), "{why}: {err}");
        store
            .put_acl_acp(&acr, &sound, "ntriples")
            .unwrap_or_else(|e| panic!("{why}: the replacement is accepted: {e}"));
        assert!(store.decide(&sess(ADMIN), r, Mode::Read).allow, "{why}: governed again");
    }
}

/// Documents the loader never reads as access control (reserved or skolem names, or one
/// governing an authority-less IRI) are not refused for what they state.
#[test]
fn documents_the_loader_does_not_read_are_not_refused() {
    let stray = "<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" .\n";
    for doc in ["urn:sparq:x.acr", "urn:skolem:x.acr", "https://.acr"] {
        let mut store = PodStore::new(Graph::load_dataset(
            "<https://pod.ex/r#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/r> .\n",
            "nquads",
        )
        .expect("parses"));
        store.materialize_acp().expect("materializes");
        if let Err(e) = store.put_acl_acp(doc, stray, "ntriples") {
            assert!(!e.contains("may not use a node of another"), "{doc}: {e}");
        }
    }
}

/// An update is judged in the state it leaves: one that takes up a sound policy while breaking
/// its document is refused, and nothing is stored.
#[test]
fn an_update_is_judged_in_the_state_it_leaves() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let r = "https://pod.ex/r";
    let acr = format!("{r}.acr");
    let other = "https://pod.ex/other.acr";
    let stray = "<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\"";
    let nq = || {
        let nq = format!(
            "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n\
             <https://pod.ex/other#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/other> .\n\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Control> <{acr}> .\n\
             <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{ADMIN}> <{acr}> .\n\
             <{acr}.acr> <{ACP}accessControl> <{acr}.acr#c> <{acr}.acr> .\n\
             <{acr}.acr#c> <{ACP}apply> <{acr}.acr#p> <{acr}.acr> .\n\
             <{acr}.acr#p> <{ACP}allow> <{ACL}Write> <{acr}.acr> .\n\
             <{acr}.acr#p> <{ACP}anyOf> <{acr}.acr#m> <{acr}.acr> .\n\
             <{acr}.acr#m> <{ACP}agent> <{ADMIN}> <{acr}.acr> .\n\
             <{other}#p> <{ACP}allow> <{ACL}Read> <{other}> .\n\
             <{other}.acr> <{ACP}accessControl> <{other}.acr#c> <{other}.acr> .\n\
             <{other}.acr#c> <{ACP}apply> <{other}.acr#p> <{other}.acr> .\n\
             <{other}.acr#p> <{ACP}allow> <{ACL}Write> <{other}.acr> .\n\
             <{other}.acr#p> <{ACP}anyOf> <{other}.acr#m> <{other}.acr> .\n\
             <{other}.acr#m> <{ACP}agent> <{ADMIN}> <{other}.acr> .\n"
        );
        PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"))
    };
    let link = format!("<{acr}#c> <{ACP}apply> <{other}#p>");
    // A sound other.acr broken in the same update: the statement other.acr may not make is
    // refused, whichever order the update makes them in.
    let mut store = nq();
    store.materialize_acp().expect("materializes");
    let stored = snapshot(store.graph());
    let err = store
        .update_as_acp(
            &sess(ADMIN),
            &format!(
                "INSERT DATA {{ GRAPH <{acr}> {{ {link} }} }} ; \
                 INSERT DATA {{ GRAPH <{other}> {{ {stray} }} }}"
            ),
        )
        .expect_err("the update is refused");
    assert!(err.contains("may not use a node of another"), "{err}");
    assert_eq!(snapshot(store.graph()), stored, "nothing was stored");
}

/// The refusal body is the same whichever reference fails: two writes into one
/// access-control document that differ only in which other document they reach into are
/// refused with identical text, so the text carries nothing about either of them.
#[test]
fn the_dependency_refusal_reads_the_same_whichever_reference_fails() {
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    let r = "https://pod.ex/r";
    let acr = format!("{r}.acr");
    let (first, second) = ("https://pod.ex/one.acr", "https://pod.ex/two.acr");
    let mut nq = format!(
        "<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Write> <{acr}> .\n\
         <{acr}#p> <{ACP}allow> <{ACL}Control> <{acr}> .\n\
         <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{ACP}agent> <{ADMIN}> <{acr}> .\n\
         <{acr}.acr> <{ACP}accessControl> <{acr}.acr#c> <{acr}.acr> .\n\
         <{acr}.acr#c> <{ACP}apply> <{acr}.acr#p> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}allow> <{ACL}Write> <{acr}.acr> .\n\
         <{acr}.acr#p> <{ACP}anyOf> <{acr}.acr#m> <{acr}.acr> .\n\
         <{acr}.acr#m> <{ACP}agent> <{ADMIN}> <{acr}.acr> .\n"
    );
    // Two documents the view does not admit, alike in every way but their names.
    for other in [first, second] {
        nq.push_str(&format!(
            "<{other}#p> <{ACP}allow> <{ACL}Read> <{other}> .\n\
             <https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{other}> .\n"
        ));
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("parses"));
    store.materialize_acp().expect("materializes");

    let refusal = |store: &mut PodStore, other: &str| {
        store
            .update_as_acp(
                &sess(ADMIN),
                &format!(
                    "INSERT DATA {{ GRAPH <{acr}> {{ <{acr}#c> <{ACP}apply> <{other}#p> }} }}"
                ),
            )
            .expect_err("the update is refused")
    };
    let (a, b) = (refusal(&mut store, first), refusal(&mut store, second));
    assert_eq!(a, b, "the two refusals differ");
    assert_refusal_says_nothing_of_the_other_document(&a, &acr, first);
    assert_refusal_says_nothing_of_the_other_document(&b, &acr, second);
}

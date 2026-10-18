//! A provenance matcher (`acp:CreatorAgent`, `acp:OwnerAgent`) is decided per resource in
//! every role: allOf, anyOf and noneOf. Being the creator or owner of one resource never
//! satisfies, or excepts from, a policy on another.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const DOCS: &str = "https://pod.ex/docs/.acr";
const R1: &str = "https://pod.ex/docs/r1";
const R2: &str = "https://pod.ex/docs/r2";
const R3: &str = "https://pod.ex/docs/r3";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const DAVE: &str = "https://dave.ex/card#me";

/// `/docs/` members get the policies `pols` (each `(name, triples about <DOCS#name>)`),
/// under `prov`.
fn store(policies: &[(&str, &str)], prov: &AccessProvenance) -> PodStore {
    let mut nq = String::new();
    for r in [R1, R2, R3] {
        nq += &format!("<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n");
    }
    nq += &format!("<{DOCS}> <{ACP}memberAccessControl> <{DOCS}#c> <{DOCS}> .\n");
    for (name, body) in policies {
        nq += &format!("<{DOCS}#c> <{ACP}apply> <{DOCS}#{name}> <{DOCS}> .\n");
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            nq += &format!("{} <{DOCS}> .\n", line.trim().trim_end_matches('.').trim());
        }
    }
    let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("fixture loads"));
    store.materialize_acp_with(prov).expect("acp materializes");
    store
}

/// The same fixture from raw n-quads, for the shapes `store` cannot express (a second
/// `.acr`, resources outside `/docs/`).
fn store_nq(nq: &str, prov: &AccessProvenance) -> PodStore {
    let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture loads"));
    store.materialize_acp_with(prov).expect("acp materializes");
    store
}

fn allows(
    store: &PodStore,
    agent: &str,
    client: Option<&str>,
    issuer: Option<&str>,
    r: &str,
    mode: Mode,
) -> bool {
    let s = Session {
        agent: Some(agent),
        client,
        issuer,
        now: None,
    };
    store.decide(&s, r, mode).allow
}

fn reads(store: &PodStore, agent: &str, r: &str) -> bool {
    allows(store, agent, None, None, r, Mode::Read)
}

fn matcher(name: &str, agent: &str) -> String {
    format!("<{DOCS}#{name}> <{ACP}agent> <{agent}>\n")
}

fn client_matcher(name: &str, client: &str) -> String {
    format!("<{DOCS}#{name}> <{ACP}client> <{client}>\n")
}

fn issuer_matcher(name: &str, issuer: &str) -> String {
    format!("<{DOCS}#{name}> <{ACP}issuer> <{issuer}>\n")
}

/// `acp:allow Read` to the public, so a deny under test has something to override.
fn public_read(name: &str) -> String {
    format!(
        "<{DOCS}#{name}> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#{name}> <{ACP}anyOf> <{DOCS}#anyone>\n{}",
        matcher("anyone", &format!("{ACP}PublicAgent")),
    )
}

#[test]
fn all_of_creator_and_owner_needs_both_on_the_same_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    prov.set_owner(R2, ALICE);
    prov.set_creator(R3, BOB);
    prov.set_owner(R3, BOB);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mc>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mo>\n{}{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(reads(&s, BOB, R3), "creator and owner of r3");
    assert!(!reads(&s, ALICE, R1), "creator of r1, owner of r2 only");
    assert!(!reads(&s, ALICE, R2), "owner of r2, creator of r1 only");
}

#[test]
fn any_of_creator_counts_only_on_the_created_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R2, ALICE);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#ma>\n\
         <{DOCS}#p> <{ACP}anyOf> <{DOCS}#mc>\n{}{}",
        matcher("ma", ALICE),
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(reads(&s, ALICE, R2), "alice created r2");
    assert!(!reads(&s, ALICE, R1), "alice did not create r1");
}

#[test]
fn none_of_creator_excepts_the_creator_of_that_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let public = format!("{ACP}PublicAgent");
    // Everyone reads, except each resource's creator.
    let allow = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}anyOf> <{DOCS}#pub>\n\
         <{DOCS}#p> <{ACP}noneOf> <{DOCS}#mc>\n{}{}",
        matcher("pub", &public),
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("p", &allow)], &prov);
    assert!(!reads(&s, ALICE, R1), "the creator of r1 is excepted on r1");
    // r2 has no recorded creator, so the exception is undecided there and the allow it
    // guards does not apply — to alice or to anyone else.
    assert!(
        !reads(&s, ALICE, R2),
        "on r2 the exception is undecided, so the allow is withheld"
    );
    assert!(!reads(&s, DAVE, R2), "for every agent");
    assert!(
        reads(&s, DAVE, R1),
        "everyone else reads r1, whose creator is recorded"
    );

    // Everyone reads; everyone but each resource's owner is denied.
    let mut prov = AccessProvenance::new();
    prov.set_owner(R1, ALICE);
    let read = format!(
        "<{DOCS}#all> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#all> <{ACP}anyOf> <{DOCS}#pub>\n{}",
        matcher("pub", &public),
    );
    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}anyOf> <{DOCS}#pub>\n\
         <{DOCS}#d> <{ACP}noneOf> <{DOCS}#mo>\n{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("all", &read), ("d", &deny)], &prov);
    assert!(reads(&s, ALICE, R1), "the owner of r1 is excepted from the deny on r1");
    assert!(!reads(&s, ALICE, R2), "but not on r2");
    assert!(!reads(&s, DAVE, R1), "everyone else is denied r1");
}

// ---------------------------------------------------------------------------
// A resource with no recorded creator / owner leaves a provenance matcher UNDECIDED:
// an allow that needs it does not apply, a deny that needs it does.
// ---------------------------------------------------------------------------

#[test]
fn an_allow_needing_an_unrecorded_creator_does_not_apply() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mc>\n{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(reads(&s, ALICE, R1), "r1's recorded creator reads r1");
    // No creator was supplied for r2/r3, so the matcher is undecided there and the
    // allow it gates does not apply — to anyone.
    assert!(
        !reads(&s, ALICE, R2),
        "undecided creator on r2 withholds the allow"
    );
    assert!(!reads(&s, DAVE, R3), "and on r3, for every agent");
}

#[test]
fn a_deny_needing_an_unrecorded_creator_still_applies() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}allOf> <{DOCS}#mc>\n{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let s = store(&[("all", &public_read("all")), ("d", &deny)], &prov);
    assert!(!reads(&s, ALICE, R1), "r1's recorded creator is denied r1");
    assert!(
        reads(&s, BOB, R1),
        "a non-creator keeps the public read on r1"
    );
    // r2 has no recorded creator: the deny's condition is undecided, so it applies.
    assert!(
        !reads(&s, ALICE, R2),
        "undecided creator on r2 keeps the deny"
    );
    assert!(!reads(&s, BOB, R2), "for every agent");
}

#[test]
fn an_undecided_owner_withholds_an_any_of_allow_and_keeps_an_any_of_deny() {
    let mut prov = AccessProvenance::new();
    prov.set_owner(R1, ALICE);
    let allow = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Write>\n\
         <{DOCS}#p> <{ACP}anyOf> <{DOCS}#mo>\n{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("p", &allow)], &prov);
    assert!(
        allows(&s, ALICE, None, None, R1, Mode::Write),
        "r1's owner writes r1"
    );
    assert!(
        !allows(&s, ALICE, None, None, R2, Mode::Write),
        "undecided owner withholds it"
    );

    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}anyOf> <{DOCS}#mo>\n{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
    );
    let s = store(&[("all", &public_read("all")), ("d", &deny)], &prov);
    assert!(!reads(&s, ALICE, R1), "r1's owner is denied r1");
    assert!(
        reads(&s, BOB, R1),
        "a non-owner keeps the public read on r1"
    );
    assert!(!reads(&s, BOB, R2), "undecided owner on r2 keeps the deny");
}

// ---------------------------------------------------------------------------
// A sibling client / issuer matcher under the same policy combines with the
// provenance one: the candidates are the resource's creator/owner crossed with every
// client and issuer value the policy's matchers name.
// ---------------------------------------------------------------------------

const APP: &str = "https://app.ex/id";
const OTHER_APP: &str = "https://other-app.ex/id";
const IDP: &str = "https://idp.ex";
const OTHER_IDP: &str = "https://other-idp.ex";

#[test]
fn a_sibling_client_matcher_combines_with_creator_agent_in_an_allow() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mc>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mcl>\n{}{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
        client_matcher("mcl", APP),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(
        allows(&s, ALICE, Some(APP), None, R1, Mode::Read),
        "r1's creator through the named client"
    );
    assert!(
        !allows(&s, ALICE, None, None, R1, Mode::Read),
        "the client matcher still binds"
    );
    assert!(
        !allows(&s, ALICE, Some(OTHER_APP), None, R1, Mode::Read),
        "another client does not"
    );
    assert!(
        !allows(&s, BOB, Some(APP), None, R1, Mode::Read),
        "a non-creator gets nothing"
    );
    assert!(
        !allows(&s, ALICE, Some(APP), None, R2, Mode::Read),
        "and nothing on a resource she did not create"
    );
}

#[test]
fn a_sibling_client_matcher_combines_with_creator_agent_in_a_deny() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}allOf> <{DOCS}#mc>\n\
         <{DOCS}#d> <{ACP}allOf> <{DOCS}#mcl>\n{}{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
        client_matcher("mcl", APP),
    );
    let s = store(&[("all", &public_read("all")), ("d", &deny)], &prov);
    assert!(
        !allows(&s, ALICE, Some(APP), None, R1, Mode::Read),
        "r1's creator is denied through that client"
    );
    assert!(
        allows(&s, ALICE, None, None, R1, Mode::Read),
        "the public read stands with no client"
    );
    assert!(
        allows(&s, ALICE, Some(OTHER_APP), None, R1, Mode::Read),
        "and through another client"
    );
    assert!(
        allows(&s, BOB, Some(APP), None, R1, Mode::Read),
        "a non-creator is not denied"
    );
}

#[test]
fn a_sibling_issuer_matcher_combines_with_owner_agent_in_an_allow() {
    let mut prov = AccessProvenance::new();
    prov.set_owner(R1, ALICE);
    let pol = format!(
        "<{DOCS}#p> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mo>\n\
         <{DOCS}#p> <{ACP}allOf> <{DOCS}#mis>\n{}{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
        issuer_matcher("mis", IDP),
    );
    let s = store(&[("p", &pol)], &prov);
    assert!(
        allows(&s, ALICE, None, Some(IDP), R1, Mode::Read),
        "r1's owner through the named issuer"
    );
    assert!(
        allows(&s, ALICE, Some(APP), Some(IDP), R1, Mode::Read),
        "any client, that issuer"
    );
    assert!(
        !allows(&s, ALICE, None, None, R1, Mode::Read),
        "the issuer matcher still binds"
    );
    assert!(
        !allows(&s, ALICE, None, Some(OTHER_IDP), R1, Mode::Read),
        "another issuer does not"
    );
    assert!(
        !allows(&s, BOB, None, Some(IDP), R1, Mode::Read),
        "a non-owner gets nothing"
    );
    assert!(
        !allows(&s, ALICE, None, Some(IDP), R2, Mode::Read),
        "and nothing on a resource she does not own"
    );
}

#[test]
fn a_sibling_issuer_matcher_combines_with_owner_agent_in_a_deny() {
    let mut prov = AccessProvenance::new();
    prov.set_owner(R1, ALICE);
    prov.set_owner(R2, ALICE);
    prov.set_owner(R3, BOB);
    let deny = format!(
        "<{DOCS}#d> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#d> <{ACP}allOf> <{DOCS}#mo>\n\
         <{DOCS}#d> <{ACP}allOf> <{DOCS}#mis>\n{}{}",
        matcher("mo", &format!("{ACP}OwnerAgent")),
        issuer_matcher("mis", IDP),
    );
    let s = store(&[("all", &public_read("all")), ("d", &deny)], &prov);
    assert!(
        !allows(&s, ALICE, None, Some(IDP), R1, Mode::Read),
        "r1's owner is denied through that issuer"
    );
    assert!(
        allows(&s, ALICE, None, Some(OTHER_IDP), R1, Mode::Read),
        "another issuer is not denied"
    );
    assert!(
        allows(&s, ALICE, None, None, R1, Mode::Read),
        "nor an issuerless session"
    );
    assert!(
        allows(&s, BOB, None, Some(IDP), R1, Mode::Read),
        "nor a non-owner"
    );
    assert!(
        !allows(&s, BOB, None, Some(IDP), R3, Mode::Read),
        "r3's own owner is denied r3"
    );
    assert!(
        allows(&s, BOB, None, Some(IDP), R2, Mode::Read),
        "but bob does not own r2"
    );
}

// ---------------------------------------------------------------------------
// Sharing: one policy reached through two `.acr`s, and one matcher reused by two
// policies, stay decided per resource.
// ---------------------------------------------------------------------------

#[test]
fn one_policy_reached_through_two_acrs_is_decided_per_resource() {
    const OTHER_ACR: &str = "https://pod.ex/other/.acr";
    const O1: &str = "https://pod.ex/other/o1";
    let mut nq = String::new();
    for r in [R1, O1] {
        nq += &format!("<{r}#it> <https://ex.dev/ns#k> \"v\" <{r}> .\n");
    }
    // Both containers' access-control documents apply THE SAME policy node.
    nq += &format!("<{DOCS}> <{ACP}memberAccessControl> <{DOCS}#c> <{DOCS}> .\n");
    nq += &format!("<{DOCS}#c> <{ACP}apply> <{DOCS}#p> <{DOCS}> .\n");
    nq += &format!("<{OTHER_ACR}> <{ACP}memberAccessControl> <{OTHER_ACR}#c> <{OTHER_ACR}> .\n");
    nq += &format!("<{OTHER_ACR}#c> <{ACP}apply> <{DOCS}#p> <{OTHER_ACR}> .\n");
    nq += &format!("<{DOCS}#p> <{ACP}allow> <{ACL}Read> <{DOCS}> .\n");
    nq += &format!("<{DOCS}#p> <{ACP}allOf> <{DOCS}#mc> <{DOCS}> .\n");
    nq += &format!("<{DOCS}#mc> <{ACP}agent> <{ACP}CreatorAgent> <{DOCS}> .\n");

    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    prov.set_creator(O1, BOB);
    let s = store_nq(&nq, &prov);
    assert!(reads(&s, ALICE, R1), "alice created r1");
    assert!(
        !reads(&s, ALICE, O1),
        "but not o1, under the same shared policy"
    );
    assert!(reads(&s, BOB, O1), "bob created o1");
    assert!(!reads(&s, BOB, R1), "but not r1");
}

#[test]
fn one_matcher_reused_by_two_policies_stays_decided_per_resource() {
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    prov.set_creator(R2, BOB);
    // `#mc` is an allOf matcher of p1 and a noneOf exception of p2 at once.
    let p1 = format!(
        "<{DOCS}#p1> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#p1> <{ACP}allOf> <{DOCS}#mc>\n{}",
        matcher("mc", &format!("{ACP}CreatorAgent")),
    );
    let p2 = format!(
        "<{DOCS}#p2> <{ACP}allow> <{ACL}Write>\n\
         <{DOCS}#p2> <{ACP}anyOf> <{DOCS}#anyone>\n\
         <{DOCS}#p2> <{ACP}noneOf> <{DOCS}#mc>\n{}",
        matcher("anyone", &format!("{ACP}PublicAgent")),
    );
    let s = store(&[("p1", &p1), ("p2", &p2)], &prov);
    // p1: read for the creator of that resource only.
    assert!(reads(&s, ALICE, R1), "alice created r1");
    assert!(!reads(&s, ALICE, R2), "bob created r2");
    assert!(reads(&s, BOB, R2), "bob reads r2");
    // p2: write for everyone BUT that resource's creator.
    assert!(
        !allows(&s, ALICE, None, None, R1, Mode::Write),
        "r1's creator is excepted on r1"
    );
    assert!(
        allows(&s, ALICE, None, None, R2, Mode::Write),
        "but writes r2"
    );
    assert!(
        allows(&s, DAVE, None, None, R1, Mode::Write),
        "everyone else writes r1"
    );
    // r3 has no recorded creator: the shared matcher is undecided there, so BOTH
    // allows are withheld.
    assert!(!reads(&s, ALICE, R3), "no read on r3");
    assert!(
        !allows(&s, DAVE, None, None, R3, Mode::Write),
        "and no write on r3"
    );
}

// ---------------------------------------------------------------------------
// Equivalence: a policy set that names no provenance matcher decides exactly as it did
// before provenance became three-valued, and materializes the same auth view.
// ---------------------------------------------------------------------------

#[test]
fn policies_without_a_provenance_matcher_decide_unchanged() {
    let public = format!("{ACP}PublicAgent");
    let authed = format!("{ACP}AuthenticatedAgent");
    let pub_read = format!(
        "<{DOCS}#pr> <{ACP}allow> <{ACL}Read>\n\
         <{DOCS}#pr> <{ACP}anyOf> <{DOCS}#mpub>\n{}",
        matcher("mpub", &public),
    );
    let alice_write = format!(
        "<{DOCS}#aw> <{ACP}allow> <{ACL}Write>\n\
         <{DOCS}#aw> <{ACP}allOf> <{DOCS}#ma>\n\
         <{DOCS}#aw> <{ACP}allOf> <{DOCS}#mcl>\n{}{}",
        matcher("ma", ALICE),
        client_matcher("mcl", APP),
    );
    let authed_append = format!(
        "<{DOCS}#aa> <{ACP}allow> <{ACL}Append>\n\
         <{DOCS}#aa> <{ACP}anyOf> <{DOCS}#mauth>\n\
         <{DOCS}#aa> <{ACP}noneOf> <{DOCS}#mb>\n{}{}",
        matcher("mauth", &authed),
        matcher("mb", BOB),
    );
    let dave_deny = format!(
        "<{DOCS}#dd> <{ACP}deny> <{ACL}Read>\n\
         <{DOCS}#dd> <{ACP}allOf> <{DOCS}#md>\n\
         <{DOCS}#dd> <{ACP}allOf> <{DOCS}#mis>\n{}{}",
        matcher("md", DAVE),
        issuer_matcher("mis", IDP),
    );
    // Provenance IS supplied, and must make no difference: no matcher names it.
    let mut prov = AccessProvenance::new();
    prov.set_creator(R1, ALICE);
    prov.set_owner(R2, BOB);
    let s = store(
        &[
            ("pr", &pub_read),
            ("aw", &alice_write),
            ("aa", &authed_append),
            ("dd", &dave_deny),
        ],
        &prov,
    );

    // The full decision matrix over (agent, client, issuer) × resource × mode.
    let sessions: [(&str, Option<&str>, Option<&str>); 6] = [
        (ALICE, None, None),
        (ALICE, Some(APP), None),
        (ALICE, Some(OTHER_APP), None),
        (BOB, None, None),
        (DAVE, None, None),
        (DAVE, None, Some(IDP)),
    ];
    let mut matrix = String::new();
    for (agent, client, issuer) in sessions {
        for r in [R1, R2, R3] {
            for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
                matrix.push(if allows(&s, agent, client, issuer, r, mode) {
                    '1'
                } else {
                    '0'
                });
            }
            matrix.push(' ');
        }
        matrix.push('|');
    }
    assert_eq!(
        matrix,
        // alice: public read everywhere; write only through APP; append (authenticated,
        // not bob). bob: read + no append (excepted). dave: read, append; read denied
        // only when the session carries IDP.
        "1010 1010 1010 |1110 1110 1110 |1010 1010 1010 |1000 1000 1000 |1010 1010 1010 |0010 0010 0010 |",
        "non-provenance policies decide exactly as before",
    );

    // …and the auth view itself is unchanged: same triple count, no provenance marker.
    let all = "SELECT ?s ?p ?o WHERE { GRAPH <urn:sparq:auth> { ?s ?p ?o } }";
    assert_eq!(
        sparq_engine::query(s.graph(), all)
            .expect("query")
            .rows
            .len(),
        AUTH_TRIPLES_WITHOUT_PROVENANCE,
        "the auth view of a provenance-free policy set is unchanged",
    );
    let marker = "SELECT ?s WHERE { GRAPH <urn:sparq:auth> { \
        ?s <https://sparq.dev/ns/solidx#provUndecided> ?k } }";
    assert_eq!(
        sparq_engine::query(s.graph(), marker)
            .expect("query")
            .rows
            .len(),
        0,
        "no provenance marker reaches the view",
    );
}

/// Pinned so a change to the auth view of a provenance-free policy set has to be
/// deliberate. Not a performance number: it is the shape of the materialized view.
const AUTH_TRIPLES_WITHOUT_PROVENANCE: usize = 38;

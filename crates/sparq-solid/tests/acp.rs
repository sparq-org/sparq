//! ACP correctness: hand-computed expected access for the ACP fixture variant —
//! cumulative inheritance, allOf/anyOf user-app pairs, deny-overrides, noneOf.

use sparq_core::Graph;
use sparq_solid::fixture::{acp_fixture, ALICE, APP, BOB, CAROL, DAVE};
use sparq_solid::{AccessProvenance, Mode, PodStore, Session};

fn store() -> PodStore {
    let g = Graph::load_dataset(&acp_fixture(), "nquads").expect("fixture loads");
    let mut s = PodStore::new(g);
    let stats = s.materialize_acp().expect("acp materializes");
    assert_eq!(stats.strata_facts.len(), 3, "three strata");
    assert!(stats.auth_triples > 0);
    s
}

fn can(s: &mut PodStore, agent: Option<&str>, client: Option<&str>, mode: Mode, graph: &str) -> bool {
    s.accessible(&Session { agent, client, issuer: None, now: None }, mode).iter().any(|g| g.as_str() == graph)
}

/// [OPUS-4.8] sq-3jtd.6: as [`can`] but with the issuer dimension supplied.
fn can_iss(
    s: &mut PodStore,
    agent: Option<&str>,
    client: Option<&str>,
    issuer: Option<&str>,
    mode: Mode,
    graph: &str,
) -> bool {
    s.accessible(&Session { agent, client, issuer, now: None }, mode).iter().any(|g| g.as_str() == graph)
}

#[test]
fn acp_expected_access_matrix() {
    let mut s = store();
    let r = Mode::Read;

    // 1. owner: root memberAccessControl reaches depth 4 (and the root itself via
    //    accessControl)
    assert!(can(&mut s, Some(ALICE), None, r, "https://pod.ex/priv0/c4/g0/d0.ttl"));
    assert!(can(&mut s, Some(ALICE), None, Mode::Write, "https://pod.ex/priv0/c4/g0/d0.ttl"));
    assert!(can(&mut s, Some(ALICE), None, r, "https://pod.ex/"));
    assert!(!can(&mut s, Some(BOB), None, r, "https://pod.ex/priv0/c4/g0/d0.ttl"));
    // 2. CUMULATIVE inheritance (the key WAC difference): pub1's own ACR does NOT
    //    shadow the root owner policy — alice keeps access; and the public matcher
    //    grants everyone incl. anonymous
    assert!(can(&mut s, Some(ALICE), None, r, "https://pod.ex/pub1/c2/g3/d1.ttl"));
    assert!(can(&mut s, None, None, r, "https://pod.ex/pub1/c2/g3/d1.ttl"));
    assert!(can(&mut s, Some(DAVE), None, r, "https://pod.ex/pub1/c2/g3/d1.ttl"));
    // 3. anyOf over enumerated agents
    assert!(can(&mut s, Some(BOB), None, Mode::Write, "https://pod.ex/team2/c3/g0/d0.ttl"));
    assert!(can(&mut s, Some(CAROL), None, r, "https://pod.ex/team2/c3/g0/d0.ttl"));
    assert!(!can(&mut s, Some(DAVE), None, r, "https://pod.ex/team2/c3/g0/d0.ttl"));
    assert!(can(&mut s, Some(ALICE), None, r, "https://pod.ex/team2/c3/g0/d0.ttl")); // cumulative root
    // 4. the NATIVE user/app pair: allOf { agent bob } { client app } — exactly the
    //    (user, application) pair, nothing else
    assert!(can(&mut s, Some(BOB), Some(APP), r, "https://pod.ex/friends3/c2/g0/d0.ttl"));
    assert!(!can(&mut s, Some(BOB), Some("https://evil.ex"), r, "https://pod.ex/friends3/c2/g0/d0.ttl"));
    assert!(!can(&mut s, Some(BOB), None, r, "https://pod.ex/friends3/c2/g0/d0.ttl"));
    assert!(!can(&mut s, Some(CAROL), Some(APP), r, "https://pod.ex/friends3/c2/g0/d0.ttl"));
    // 5. DENY-OVERRIDES: dave is denied even though the public policy allows
    assert!(!can(&mut s, Some(DAVE), None, r, "https://pod.ex/mixed4/c3/g0/d0.ttl"));
    assert!(can(&mut s, Some(BOB), None, r, "https://pod.ex/mixed4/c3/g0/d0.ttl"));
    assert!(can(&mut s, None, None, r, "https://pod.ex/mixed4/c3/g0/d0.ttl"));
    // 6. noneOf: public-except-carol (a conditional grant evaluated per session)
    assert!(!can(&mut s, Some(CAROL), None, r, "https://pod.ex/origin5/c2/g0/d0.ttl"));
    assert!(can(&mut s, Some(BOB), None, r, "https://pod.ex/origin5/c2/g0/d0.ttl"));
    assert!(can(&mut s, None, None, r, "https://pod.ex/origin5/c2/g0/d0.ttl"));
    assert!(can(&mut s, Some(DAVE), None, r, "https://pod.ex/origin5/c2/g0/d0.ttl"));
}

#[test]
fn acp_rematerialization_picks_up_policy_change() {
    let mut s = store();
    assert!(!can(&mut s, Some(DAVE), None, Mode::Read, "https://pod.ex/team2/c3/g0/d0.ttl"));
    // extend team2's anyOf with dave: swap the .acr graph, re-materialize
    let name = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("https://pod.ex/team2/.acr"));
    let pos = s.graph.named.iter().position(|(n, _)| *n == name).expect("acr graph");
    let acr = "https://pod.ex/team2/.acr";
    let team = format!(
        "<{acr}> <http://www.w3.org/ns/solid/acp#memberAccessControl> <{acr}#ctl-team> .\n\
         <{acr}#ctl-team> <http://www.w3.org/ns/solid/acp#apply> <{acr}#pol-team> .\n\
         <{acr}#pol-team> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> .\n\
         <{acr}#pol-team> <http://www.w3.org/ns/solid/acp#anyOf> <{acr}#m-dave> .\n\
         <{acr}#m-dave> <http://www.w3.org/ns/solid/acp#agent> <https://dave.ex/card#me> .\n"
    );
    s.graph.named[pos].1 = Graph::load_str(&team, "ntriples").unwrap();
    s.materialize_acp().expect("re-materialize");
    assert!(can(&mut s, Some(DAVE), None, Mode::Read, "https://pod.ex/team2/c3/g0/d0.ttl"));
    // …and bob lost the write grant the old policy carried
    assert!(!can(&mut s, Some(BOB), None, Mode::Write, "https://pod.ex/team2/c3/g0/d0.ttl"));
}

/// [OPUS-4.8] sq-xor3: ACP write-path enforcement. `update_as_acp` gates a SPARQL Update
/// against `acl:Write` (cumulative ACP allow ∖ deny) before mutating. Expected from the
/// ACP fixture: ALICE has Read/Write/Control everywhere (cumulative root owner policy);
/// team2's `acp:allow Read,Write` anyOf {bob,carol} gives BOB+CAROL write; DAVE has none.
#[test]
fn acp_write_enforcement_matches_grants() {
    let mut s = store();
    let ins = |g: &str| format!("INSERT DATA {{ GRAPH <{g}> {{ <{g}#it> <https://ex.dev/ns#k> \"v\" }} }}");

    let priv0 = "https://pod.ex/priv0/c4/g0/d0.ttl";
    assert!(can(&mut s, Some(ALICE), None, Mode::Write, priv0));
    s.update_as_acp(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, &ins(priv0))
        .expect("alice (cumulative owner) writes priv0");
    assert!(!can(&mut s, Some(BOB), None, Mode::Write, priv0));
    assert!(
        s.update_as_acp(&Session { agent: Some(BOB), client: None, issuer: None, now: None }, &ins(priv0)).is_err(),
        "bob has no write on priv0"
    );

    let team2 = "https://pod.ex/team2/c3/g0/d0.ttl";
    assert!(can(&mut s, Some(BOB), None, Mode::Write, team2));
    s.update_as_acp(&Session { agent: Some(BOB), client: None, issuer: None, now: None }, &ins(team2))
        .expect("bob (anyOf) writes team2");
    assert!(!can(&mut s, Some(DAVE), None, Mode::Write, team2));
    assert!(
        s.update_as_acp(&Session { agent: Some(DAVE), client: None, issuer: None, now: None }, &ins(team2)).is_err(),
        "dave denied write on team2"
    );
}

/// How many triples the named graph `name` currently holds (0 if absent).
///
/// NON-wasm32 with the deadline test below, its only caller (see that test's note).
#[cfg(not(target_arch = "wasm32"))]
fn graph_len(store: &PodStore, name: &str) -> usize {
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(name));
    store
        .graph
        .named
        .iter()
        .find(|(n, _)| *n == term)
        .map(|(_, g)| {
            let pat: sparq_core::store::Pattern = [None, None, None];
            g.store.scan(&pat).rows.len()
        })
        .unwrap_or(0)
}

/// [FABLE-5] sq-yhlf0: the ACP twin of the budgeted write path. `update_as_acp_with_budget`
/// must enforce the SAME ACP grants as `update_as_acp` and, on top of that, abort when the
/// caller's budget is exhausted — leaving the store untouched. (The WAC side's fuller matrix
/// lives in tests/update.rs; this pins that the ACP entry point is wired to the same path
/// rather than quietly dropping the budget.)
///
/// NON-wasm32: `QueryBudget::deadline` does not exist on `wasm32-unknown-unknown` (the
/// field is `#[cfg(not(target_arch = "wasm32"))]` in sparq-engine, because
/// `std::time::Instant` panics there), and this crate's test targets are COMPILED for
/// wasm32 by the CI wasm lane. Nothing is lost: a plain `#[test]` is never RUN by
/// `wasm-pack test`, which executes only `#[wasm_bindgen_test]`s (tests/wasm_materialize.rs).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn acp_budgeted_write_enforces_grants_and_the_budget() {
    use sparq_engine::QueryBudget;

    let mut s = store();
    let priv0 = "https://pod.ex/priv0/c4/g0/d0.ttl";
    // A WHERE-bearing update: the engine's bulk-data operations do not consult the budget.
    let sparql = format!(
        "INSERT {{ GRAPH <{priv0}> {{ <{priv0}#it> <https://ex.dev/ns#k> \"v\" }} }} \
         WHERE {{ GRAPH <{priv0}> {{ ?s ?p ?o }} }}"
    );
    let alice = Session { agent: Some(ALICE), client: None, issuer: None, now: None };
    let before = graph_len(&s, priv0);

    // Exhausted deadline -> abort, nothing written (alice DOES hold the grant, so this can
    // only be the budget).
    let mut expired = QueryBudget::unlimited();
    expired.deadline = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    let e = s
        .update_as_acp_with_budget(&alice, &sparql, &expired)
        .expect_err("an exhausted deadline aborts the ACP write path too");
    assert!(e.contains("query budget exceeded"), "{e}");
    assert_eq!(graph_len(&s, priv0), before, "an aborted ACP update mutates nothing");

    // Unlimited budget -> the same update applies, and BOB is still denied.
    s.update_as_acp_with_budget(&alice, &sparql, &QueryBudget::unlimited())
        .expect("alice (cumulative owner) writes priv0 under an unlimited budget");
    assert_eq!(graph_len(&s, priv0), before + 1, "the unbudgeted-equivalent write applied");
    let bob = Session { agent: Some(BOB), client: None, issuer: None, now: None };
    assert!(
        s.update_as_acp_with_budget(&bob, &sparql, &QueryBudget::unlimited()).is_err(),
        "the budget never widens authorization: bob still has no write on priv0"
    );
}

// ── [OPUS-4.8] sq-3jtd.6: acp:issuer support — the (agent, client, issuer) principal ──
//
// The issuer dimension is the exact twin of the client dimension (design doc §3.6): an
// ACP Matcher can constrain on the OIDC issuer that vouched for the requester's WebID.
// These tests build small inline ACP pods (one .acr graph) so the issuer matrix is
// hand-verifiable, and exercise BOTH outcomes — allow when the issuer matches, deny when
// it does not (or is absent), through the unchanged `materialize_acp` + `accessible` path.

const GOOD_IDP: &str = "https://good-idp.ex";
const EVIL_IDP: &str = "https://evil-idp.ex";
const ISS_DOC: &str = "https://pod.ex/iss/d0.ttl";
const ISS_ACR: &str = "https://pod.ex/iss/.acr";

/// One inline ACP pod: a single document `iss/d0.ttl` whose containing `iss/` ACR
/// (`iss/.acr`) carries `policy_body` (raw N-Triple body lines, sharing the `#pol` policy
/// node) under `acp:memberAccessControl`, so the policy applies cumulatively to the
/// member document — the same inheritance shape as the shared fixture.
fn iss_store(policy_body: &str) -> PodStore {
    let acr = format!(
        "<{ISS_DOC}#it> <https://ex.dev/ns#title> \"iss\" <{ISS_DOC}> .\n\
         <{ISS_ACR}> <http://www.w3.org/ns/solid/acp#memberAccessControl> <{ISS_ACR}#ctl> <{ISS_ACR}> .\n\
         <{ISS_ACR}#ctl> <http://www.w3.org/ns/solid/acp#apply> <{ISS_ACR}#pol> <{ISS_ACR}> .\n\
         {policy_body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("inline acp pod loads");
    let mut s = PodStore::new(g);
    s.materialize_acp().expect("acp materializes");
    s
}

/// allOf { acp:agent BOB ; acp:issuer GOOD_IDP }: BOB is granted Read ONLY when the
/// session's issuer is GOOD_IDP. Allow AND deny are both decided by the issuer.
#[test]
fn acp_issuer_allof_allow_and_deny() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{ISS_ACR}#pol> <{acp}allow> <{acl}Read> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}allOf> <{ISS_ACR}#m> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}agent> <{BOB}> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}issuer> <{GOOD_IDP}> <{ISS_ACR}> .\n"
    );
    let mut s = iss_store(&body);
    let r = Mode::Read;

    // ALLOW: bob through the trusted issuer.
    assert!(can_iss(&mut s, Some(BOB), None, Some(GOOD_IDP), r, ISS_DOC), "bob @ good-idp allowed");
    // DENY decided by issuer: same agent, wrong issuer.
    assert!(!can_iss(&mut s, Some(BOB), None, Some(EVIL_IDP), r, ISS_DOC), "bob @ evil-idp denied");
    // DENY fail-closed: issuer-constrained grant never matches a session with no issuer.
    assert!(!can_iss(&mut s, Some(BOB), None, None, r, ISS_DOC), "bob w/o issuer denied");
    // DENY: right issuer, wrong agent (allOf needs both).
    assert!(!can_iss(&mut s, Some(CAROL), None, Some(GOOD_IDP), r, ISS_DOC), "carol @ good-idp denied");
}

/// anyOf { acp:issuer GOOD_IDP } (issuer-only, no agent attr): ANY agent vouched for by
/// GOOD_IDP is granted, regardless of WebID; nobody from another issuer is.
#[test]
fn acp_issuer_only_matcher_any_agent() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{ISS_ACR}#pol> <{acp}allow> <{acl}Read> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}anyOf> <{ISS_ACR}#m> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}issuer> <{GOOD_IDP}> <{ISS_ACR}> .\n"
    );
    let mut s = iss_store(&body);
    let r = Mode::Read;

    // ALLOW: any agent from the trusted issuer (the agent dimension is unconstrained).
    assert!(can_iss(&mut s, Some(BOB), None, Some(GOOD_IDP), r, ISS_DOC), "bob @ good-idp allowed");
    assert!(can_iss(&mut s, Some(CAROL), None, Some(GOOD_IDP), r, ISS_DOC), "carol @ good-idp allowed");
    // DENY decided by issuer: wrong issuer, or no issuer asserted at all.
    assert!(!can_iss(&mut s, Some(BOB), None, Some(EVIL_IDP), r, ISS_DOC), "bob @ evil-idp denied");
    assert!(!can_iss(&mut s, Some(CAROL), None, None, r, ISS_DOC), "carol w/o issuer denied");
    assert!(!can_iss(&mut s, None, None, None, r, ISS_DOC), "anonymous w/o issuer denied");
    // The matcher is agent-unconstrained, so it grants the agent-dimension top
    // (auth:Public): a session presenting only the trusted issuer is accepted even without
    // a WebID — faithful, since the matcher asserts nothing about the agent.
    assert!(can_iss(&mut s, None, None, Some(GOOD_IDP), r, ISS_DOC), "any context @ good-idp allowed");
}

/// allOf { acp:agent BOB ; acp:client APP ; acp:issuer GOOD_IDP }: the full three-
/// dimension principal — exactly the (user, app, issuer) triple, nothing wider.
#[test]
fn acp_issuer_client_triple() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{ISS_ACR}#pol> <{acp}allow> <{acl}Read> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}allOf> <{ISS_ACR}#m> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}agent> <{BOB}> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}client> <{APP}> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}issuer> <{GOOD_IDP}> <{ISS_ACR}> .\n"
    );
    let mut s = iss_store(&body);
    let r = Mode::Read;

    // ALLOW: the exact triple.
    assert!(can_iss(&mut s, Some(BOB), Some(APP), Some(GOOD_IDP), r, ISS_DOC), "bob+app@good-idp allowed");
    // DENY decided by issuer alone (agent + client correct).
    assert!(!can_iss(&mut s, Some(BOB), Some(APP), Some(EVIL_IDP), r, ISS_DOC), "wrong issuer denied");
    // DENY: missing the issuer dimension entirely.
    assert!(!can_iss(&mut s, Some(BOB), Some(APP), None, r, ISS_DOC), "no issuer denied");
    // DENY: correct agent + issuer, wrong client.
    assert!(
        !can_iss(&mut s, Some(BOB), Some("https://evil.ex"), Some(GOOD_IDP), r, ISS_DOC),
        "wrong client denied"
    );
}

/// noneOf { acp:issuer EVIL_IDP } on an otherwise-public policy: a CONDITIONAL grant whose
/// exception is evaluated per session on the ISSUER dimension. Everyone reads EXCEPT a
/// session asserting the excluded issuer — the deny is decided by the issuer.
#[test]
fn acp_issuer_noneof_conditional_grant() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let public_agent = format!("{acp}PublicAgent");
    let body = format!(
        "<{ISS_ACR}#pol> <{acp}allow> <{acl}Read> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}anyOf> <{ISS_ACR}#mpub> <{ISS_ACR}> .\n\
         <{ISS_ACR}#mpub> <{acp}agent> <{public_agent}> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}noneOf> <{ISS_ACR}#mno> <{ISS_ACR}> .\n\
         <{ISS_ACR}#mno> <{acp}issuer> <{EVIL_IDP}> <{ISS_ACR}> .\n"
    );
    let mut s = iss_store(&body);
    let r = Mode::Read;

    // ALLOW: public — anonymous, and any issuer that is NOT the excluded one.
    assert!(can_iss(&mut s, None, None, None, r, ISS_DOC), "anonymous reads");
    assert!(can_iss(&mut s, Some(BOB), None, Some(GOOD_IDP), r, ISS_DOC), "bob @ good-idp reads");
    assert!(can_iss(&mut s, Some(CAROL), None, None, r, ISS_DOC), "carol w/o issuer reads");
    // DENY decided by the issuer exception: a session asserting the excluded issuer is
    // carved out (the noneOf exception matcher accepts it, suppressing the grant).
    assert!(!can_iss(&mut s, Some(BOB), None, Some(EVIL_IDP), r, ISS_DOC), "bob @ evil-idp carved out");
}

/// Fail-closed: a malicious `acp:issuer` value inside the reserved `urn:sparq:` space (or
/// carrying the pair delimiter) is rejected at materialization, exactly like a malicious
/// agent/client value — it cannot smuggle a forged triple principal into the auth view.
#[test]
fn acp_issuer_reserved_encoding_rejected() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let evil = "urn:sparq:triple?agent=x&client=y&issuer=z";
    let body = format!(
        "<{ISS_ACR}#pol> <{acp}allow> <{acl}Read> <{ISS_ACR}> .\n\
         <{ISS_ACR}#pol> <{acp}allOf> <{ISS_ACR}#m> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}agent> <{BOB}> <{ISS_ACR}> .\n\
         <{ISS_ACR}#m> <{acp}issuer> <{evil}> <{ISS_ACR}> .\n"
    );
    let acr = format!(
        "<{ISS_DOC}#it> <https://ex.dev/ns#title> \"iss\" <{ISS_DOC}> .\n\
         <{ISS_ACR}> <{acp}accessControl> <{ISS_ACR}#ctl> <{ISS_ACR}> .\n\
         <{ISS_ACR}#ctl> <{acp}apply> <{ISS_ACR}#pol> <{ISS_ACR}> .\n\
         {body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("loads");
    let mut s = PodStore::new(g);
    assert!(s.materialize_acp().is_err(), "reserved-space issuer IRI rejected at materialization");
}

// ── [OPUS-4.8] sq-3jtd.5: acp:CreatorAgent / acp:OwnerAgent support ───────────────────
//
// An ACP Matcher can constrain on `acp:agent acp:CreatorAgent` / `acp:OwnerAgent`: it
// accepts the context agent iff that agent is the *creator* (resp. *owner*) of the
// resource being accessed. That per-resource fact ("who created <r>") is NOT in the pod
// or the ACR — it is structural storage metadata the TRUSTED caller (PSS, which created
// the resource) supplies through [`AccessProvenance`]. Crucially it is NEVER read from
// the resource graph: a writer who could embed `<r> solidx:creator <self>` in their own
// document must not thereby grant themselves access (design doc §2.4). These tests build
// small inline ACP pods so the matrix is hand-verifiable, and exercise BOTH outcomes —
// allow when the session agent is the trusted creator/owner, deny otherwise — plus the
// resource-scoping and trust-boundary invariants.

const CRE_DOC: &str = "https://pod.ex/cre/d0.ttl";
const CRE_ACR: &str = "https://pod.ex/cre/.acr";
const CRE_DOC2: &str = "https://pod.ex/cre/d1.ttl";

/// One inline ACP pod: documents under `cre/` whose containing `cre/` ACR carries
/// `policy_body` under `acp:memberAccessControl`, materialized with the trusted
/// `provenance` (per-resource creator/owner WebIDs).
fn cre_store(policy_body: &str, provenance: &AccessProvenance) -> PodStore {
    let acr = format!(
        "<{CRE_DOC}#it> <https://ex.dev/ns#title> \"d0\" <{CRE_DOC}> .\n\
         <{CRE_DOC2}#it> <https://ex.dev/ns#title> \"d1\" <{CRE_DOC2}> .\n\
         <{CRE_ACR}> <http://www.w3.org/ns/solid/acp#memberAccessControl> <{CRE_ACR}#ctl> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <http://www.w3.org/ns/solid/acp#apply> <{CRE_ACR}#pol> <{CRE_ACR}> .\n\
         {policy_body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("inline acp pod loads");
    let mut s = PodStore::new(g);
    s.materialize_acp_with(provenance).expect("acp materializes with provenance");
    s
}

/// allOf { acp:agent acp:CreatorAgent }: the resource's trusted creator is granted Read;
/// everyone else (including another WebID, or the SAME WebID when no creator fact is
/// supplied for that resource) is denied. Allow AND deny are decided by the creator fact.
#[test]
fn acp_creator_agent_allow_and_deny() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB); // PSS: bob created d0
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    // ALLOW: bob is the trusted creator of d0.
    assert!(can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator) reads d0");
    // DENY decided by the creator fact: carol is not the creator of d0.
    assert!(!can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol (non-creator) denied d0");
    // DENY: anonymous has no WebID, so cannot be a creator.
    assert!(!can(&mut s, None, None, r, CRE_DOC), "anonymous denied d0");
    // DENY fail-closed: NO creator fact for d1, so even bob is denied there.
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC2), "no creator fact for d1 -> bob denied d1");
}

/// Resource-scoping: a CreatorAgent grant is per-resource. bob is the creator of d0 and
/// carol of d1; the SINGLE shared CreatorAgent matcher grants each ONLY their own
/// resource, never the other's.
#[test]
fn acp_creator_agent_is_resource_scoped() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB);
    prov.set_creator(CRE_DOC2, CAROL);
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    assert!(can(&mut s, Some(BOB), None, r, CRE_DOC), "bob reads d0 (his)");
    assert!(can(&mut s, Some(CAROL), None, r, CRE_DOC2), "carol reads d1 (hers)");
    // the load-bearing soundness check: creator of d0 is NOT granted d1, and vice-versa.
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC2), "bob NOT granted d1 (carol's)");
    assert!(!can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol NOT granted d0 (bob's)");
}

/// acp:OwnerAgent is the twin of acp:CreatorAgent over the trusted owner fact.
#[test]
fn acp_owner_agent_allow_and_deny() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Write> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}OwnerAgent> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_owner(CRE_DOC, ALICE); // PSS: alice owns d0
    let mut s = cre_store(&body, &prov);
    let w = Mode::Write;

    assert!(can(&mut s, Some(ALICE), None, w, CRE_DOC), "alice (owner) writes d0");
    assert!(!can(&mut s, Some(BOB), None, w, CRE_DOC), "bob (non-owner) denied write on d0");
    // a creator fact does NOT satisfy an OwnerAgent matcher (distinct dimensions).
    let mut prov2 = AccessProvenance::default();
    prov2.set_creator(CRE_DOC, BOB);
    let mut s2 = cre_store(&body, &prov2);
    assert!(!can(&mut s2, Some(BOB), None, w, CRE_DOC), "creator is not owner -> denied");
}

/// Deny-overrides with a CreatorAgent matcher: a public-Read policy plus a
/// `acp:deny Read` policy whose matcher is `acp:agent acp:CreatorAgent` carves the creator
/// out — everyone reads EXCEPT the resource's creator, and the deny is resource-scoped
/// (it only carves the creator out of THEIR resource).
#[test]
fn acp_creator_agent_deny_overrides() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    // pol-pub: public Read.  pol-deny: deny Read to the resource's creator.
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}anyOf> <{CRE_ACR}#mpub> <{CRE_ACR}> .\n\
         <{CRE_ACR}#mpub> <{acp}agent> <{acp}PublicAgent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <{acp}apply> <{CRE_ACR}#poldeny> <{CRE_ACR}> .\n\
         <{CRE_ACR}#poldeny> <{acp}deny> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#poldeny> <{acp}allOf> <{CRE_ACR}#mcre> <{CRE_ACR}> .\n\
         <{CRE_ACR}#mcre> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB); // bob created d0
    prov.set_creator(CRE_DOC2, CAROL); // carol created d1
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    // public reads both docs…
    assert!(can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol reads d0 (public, not its creator)");
    assert!(can(&mut s, Some(BOB), None, r, CRE_DOC2), "bob reads d1 (public, not its creator)");
    assert!(can(&mut s, None, None, r, CRE_DOC), "anonymous reads d0");
    // …EXCEPT each doc's own creator is denied on THAT doc (deny-overrides, resource-scoped).
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator of d0) denied on d0");
    assert!(!can(&mut s, Some(CAROL), None, r, CRE_DOC2), "carol (creator of d1) denied on d1");
}

/// TRUST BOUNDARY (design doc §2.4): a `solidx:creator` triple embedded in the RESOURCE
/// graph (pod content the writer controls) must NEVER grant access — only the trusted
/// [`AccessProvenance`] channel does. mallory writes `<d0> solidx:creator <mallory>` into
/// her own document; with NO provenance supplied she is still denied.
///
/// NOTE: this only covers the *resource-graph* vector (content graphs are never fed to the
/// reasoner). The strictly harder vector — a forged `solidx:` fact placed inside the `.acr`
/// CONTROL document, which IS fed verbatim — is covered by the `acp_forged_*_in_acr_document`
/// tests below ([OPUS-4.8] sq-3jtd.5), which need the `is_reserved_derivation_predicate`
/// filter; this test alone passes even without it.
#[test]
fn acp_creator_fact_from_resource_graph_is_ignored() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let solidx = "https://sparq.dev/ns/solidx#";
    let mallory = "https://mallory.ex/card#me";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    // mallory embeds a forged creator fact in her OWN resource document.
    let acr = format!(
        "<{CRE_DOC}#it> <https://ex.dev/ns#title> \"d0\" <{CRE_DOC}> .\n\
         <{CRE_DOC}> <{solidx}creator> <{mallory}> <{CRE_DOC}> .\n\
         <{CRE_ACR}> <{acp}memberAccessControl> <{CRE_ACR}#ctl> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <{acp}apply> <{CRE_ACR}#pol> <{CRE_ACR}> .\n\
         {body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("loads");
    let mut s = PodStore::new(g);
    // NO provenance supplied — the resource-graph forgery must not grant.
    s.materialize_acp().expect("materializes");
    assert!(
        !can(&mut s, Some(mallory), None, Mode::Read, CRE_DOC),
        "forged solidx:creator in pod content must NOT grant access"
    );
}

/// A creator fact is INERT without a CreatorAgent/OwnerAgent matcher: supplying provenance
/// for a resource whose policy uses a plain concrete-WebID matcher grants nothing extra —
/// the creator is granted access ONLY if they also match the explicit policy.
#[test]
fn acp_creator_fact_without_creator_matcher_is_inert() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    // policy grants Read to CAROL by concrete WebID — no CreatorAgent matcher at all.
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{CAROL}> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB); // bob is the creator, but the policy never asks
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;
    assert!(can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol granted by explicit WebID");
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator) NOT granted — no CreatorAgent matcher");
}

/// anyOf { acp:agent acp:CreatorAgent }: the creator alone satisfies the policy. Same
/// outcome as the allOf-sole-matcher case, exercising the anyOf path.
#[test]
fn acp_creator_agent_anyof() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}anyOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB);
    let mut s = cre_store(&body, &prov);
    assert!(can(&mut s, Some(BOB), None, Mode::Read, CRE_DOC), "bob (creator) reads d0 via anyOf");
    assert!(!can(&mut s, Some(CAROL), None, Mode::Read, CRE_DOC), "carol denied");
}

/// allOf { acp:agent acp:CreatorAgent ; acp:client APP }: the creator-AND-client pair on
/// ONE matcher — the resource's creator is granted ONLY through the named app, composing
/// the provenance agent dimension with the client dimension.
#[test]
fn acp_creator_agent_with_client_constraint() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}client> <{APP}> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB);
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    // ALLOW: bob (creator) through the named app.
    assert!(
        s.accessible(&Session { agent: Some(BOB), client: Some(APP), issuer: None, now: None }, r)
            .iter()
            .any(|g| g.as_str() == CRE_DOC),
        "bob (creator) + app reads d0"
    );
    // DENY: creator without the named client (the client dimension is constrained).
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator) w/o app denied");
    // DENY: named client but not the creator.
    assert!(
        !s.accessible(&Session { agent: Some(CAROL), client: Some(APP), issuer: None, now: None }, r)
            .iter()
            .any(|g| g.as_str() == CRE_DOC),
        "carol (non-creator) + app denied"
    );
}

/// [OPUS-4.8] sq-az1b — allOf { CreatorAgent } { concrete WebID } where the WebID == the
/// resource's creator. The bead asked whether this "degenerate" composition is supported;
/// it IS, with no special-casing: the provenance candidate's agent dimension is the creator
/// WebID, and the sibling concrete-agent matcher's accept-set is exactly that one WebID, so
/// the allOf rejection check (acp-b.n3) finds the agent accepted by BOTH matchers and the
/// resource-scoped grant fires. Crucially the grant stays RESOURCE-SCOPED: the creator of
/// d0 (who is ALSO the literal WebID the second matcher names) is granted ONLY d0, never d1
/// where they are not the creator — the second concrete matcher does not widen the
/// provenance candidate's resource binding.
#[test]
fn acp_creator_agent_allof_with_matching_concrete_agent_is_supported() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    // allOf { CreatorAgent } { concrete agent BOB } — BOB is the creator of d0.
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#mcre> <{CRE_ACR}> .\n\
         <{CRE_ACR}#mcre> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#magent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#magent> <{acp}agent> <{BOB}> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB); // bob created d0 (and is the concrete WebID named)
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    // ALLOW: bob is BOTH the creator of d0 AND the WebID the second matcher names.
    assert!(can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator == named WebID) reads d0");
    // DENY: carol is neither the creator nor the named WebID.
    assert!(!can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol denied d0");
    // SOUNDNESS — still resource-scoped: bob is NOT the creator of d1, so even though the
    // second matcher names bob, the contradictory CreatorAgent half blocks d1. The concrete
    // matcher must not widen the provenance candidate's resource binding to every resource.
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC2), "bob (named, but not creator of d1) denied d1");
}

/// [OPUS-4.8] sq-az1b — the genuinely contradictory shape: allOf { CreatorAgent } { concrete
/// WebID } where the WebID is a FIXED agent that is NOT the creator. The two matchers demand
/// the agent be BOTH the creator AND a different fixed WebID — unsatisfiable for any agent,
/// so NOBODY is granted. This is correct-by-soundness (an empty intersection), not a missing
/// feature: the creator is rejected by the concrete matcher, and the fixed WebID is rejected
/// by the CreatorAgent matcher (it is not the creator). Documented bound, design doc §3.6.
#[test]
fn acp_creator_agent_allof_with_contradictory_concrete_agent_grants_nobody() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    // allOf { CreatorAgent } { concrete agent CAROL } — CAROL is NOT the creator of d0.
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#mcre> <{CRE_ACR}> .\n\
         <{CRE_ACR}#mcre> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#magent> <{CRE_ACR}> .\n\
         <{CRE_ACR}#magent> <{acp}agent> <{CAROL}> <{CRE_ACR}> .\n"
    );
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, BOB); // bob created d0; CAROL is a distinct fixed WebID
    let mut s = cre_store(&body, &prov);
    let r = Mode::Read;

    // The creator (bob) is rejected by the concrete CAROL matcher…
    assert!(!can(&mut s, Some(BOB), None, r, CRE_DOC), "bob (creator, not CAROL) denied d0");
    // …and the fixed WebID (carol) is rejected by the CreatorAgent matcher (she is not the creator).
    assert!(!can(&mut s, Some(CAROL), None, r, CRE_DOC), "carol (named, not creator) denied d0");
}

/// Fail-closed: a malicious creator/owner WebID inside the reserved `urn:sparq:` space is
/// rejected at materialization, exactly like a malicious agent/client/issuer value — it
/// cannot smuggle a forged principal into the auth view.
#[test]
fn acp_creator_reserved_encoding_rejected() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let body = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    let acr = format!(
        "<{CRE_DOC}#it> <https://ex.dev/ns#title> \"d0\" <{CRE_DOC}> .\n\
         <{CRE_ACR}> <{acp}accessControl> <{CRE_ACR}#ctl> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <{acp}apply> <{CRE_ACR}#pol> <{CRE_ACR}> .\n\
         {body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("loads");
    let mut s = PodStore::new(g);
    let mut prov = AccessProvenance::default();
    prov.set_creator(CRE_DOC, "urn:sparq:pair?agent=x&client=y");
    assert!(
        s.materialize_acp_with(&prov).is_err(),
        "reserved-space creator WebID rejected at materialization"
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-3jtd.5 — ADVERSARIAL: forged derivation-internal `solidx:` facts
// smuggled INSIDE the `.acr` access-control document (not the resource graph) must
// NEVER reach the reasoner. These exploit the real attack surface that
// `acp_creator_fact_from_resource_graph_is_ignored` does NOT cover: that test placed
// the forged triple in a *resource* graph (`<CRE_DOC>`), whose triples the loader never
// feeds to the reasoner anyway. The `.acr` graph's triples ARE emitted verbatim — so
// without the `is_reserved_derivation_predicate` filter each of these GRANTS access
// (cross-resource privilege escalation). Each test FAILS pre-filter, PASSES post-filter.
// ---------------------------------------------------------------------------

/// Builds an inline ACP pod where the `.acr` for `cre/` carries (a) a CreatorAgent (resp.
/// OwnerAgent) matcher under `prov_pol` granting `prov_mode`, and (b) a SEPARATE matcher
/// listing `mallory` as a concrete `acp:agent` on a DIFFERENT mode (`alt_mode`) — that
/// concrete reference is what legitimately makes `mallory solidx:isWebId true`, exactly
/// as it would for any agent named anywhere in the ACR. Then `forged_fact` smuggles
/// `<d0> solidx:creator|owner <mallory>` directly into the `.acr`. NO trusted provenance.
/// Without the `solidx:`-predicate filter, the forged fact + the legitimate `isWebId`
/// together mint a provenance candidate and escalate mallory from `alt_mode` to
/// `prov_mode` on d0. With the filter, the forged fact never reaches the reasoner.
fn forged_prov_store(prov_pol_body: &str, alt_pol_body: &str, forged_fact: &str) -> PodStore {
    let acp = "http://www.w3.org/ns/solid/acp#";
    let acr = format!(
        "<{CRE_DOC}#it> <https://ex.dev/ns#title> \"d0\" <{CRE_DOC}> .\n\
         <{CRE_ACR}> <{acp}memberAccessControl> <{CRE_ACR}#ctl> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <{acp}apply> <{CRE_ACR}#pol> <{CRE_ACR}> .\n\
         <{CRE_ACR}#ctl> <{acp}apply> <{CRE_ACR}#altpol> <{CRE_ACR}> .\n\
         {forged_fact}\
         {prov_pol_body}\
         {alt_pol_body}"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("forged inline acp pod loads");
    let mut s = PodStore::new(g);
    // NO provenance — the ONLY creator/owner facts are the forged ones inside the .acr.
    s.materialize_acp().expect("acp materializes (forged facts must be filtered, not error)");
    s
}

/// EXPLOIT 1: forged `<R> solidx:creator <mallory>` inside the `.acr`, against a Read
/// `CreatorAgent` matcher. Mallory is legitimately known (`isWebId`) because a separate
/// matcher grants her Write by concrete WebID — but she has NO legitimate Read. The
/// forged creator fact, if it reached the reasoner, would escalate her to Read on d0.
#[test]
fn acp_forged_creator_in_acr_document_does_not_grant() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let solidx = "https://sparq.dev/ns/solidx#";
    let mallory = "https://mallory.ex/card#me";
    // #pol: Read to the (forged) CreatorAgent.
    let prov_pol = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}CreatorAgent> <{CRE_ACR}> .\n"
    );
    // #altpol: Write to mallory by concrete WebID — gives her isWebId (and a Write grant),
    // but NOT Read.
    let alt_pol = format!(
        "<{CRE_ACR}#altpol> <{acp}allow> <{acl}Write> <{CRE_ACR}> .\n\
         <{CRE_ACR}#altpol> <{acp}allOf> <{CRE_ACR}#altm> <{CRE_ACR}> .\n\
         <{CRE_ACR}#altm> <{acp}agent> <{mallory}> <{CRE_ACR}> .\n"
    );
    let forged = format!("<{CRE_DOC}> <{solidx}creator> <{mallory}> <{CRE_ACR}> .\n");
    let mut s = forged_prov_store(&prov_pol, &alt_pol, &forged);
    // Baseline: her LEGITIMATE Write grant is intact (the filter does not over-block).
    assert!(can(&mut s, Some(mallory), None, Mode::Write, CRE_DOC), "mallory keeps her legit Write");
    // The exploit: the forged creator fact must NOT escalate her to Read.
    assert!(
        !can(&mut s, Some(mallory), None, Mode::Read, CRE_DOC),
        "forged `solidx:creator` inside the .acr must NOT self-grant Read (privilege escalation)"
    );
}

/// EXPLOIT 2: forged `<R> solidx:owner <mallory>` inside the `.acr`, against a Write
/// `OwnerAgent` matcher — the OwnerAgent twin of EXPLOIT 1. Mallory legitimately has
/// only Read (concrete WebID on `#altpol`); the forged owner fact must not escalate her
/// to Write on d0.
#[test]
fn acp_forged_owner_in_acr_document_does_not_grant() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let solidx = "https://sparq.dev/ns/solidx#";
    let mallory = "https://mallory.ex/card#me";
    let prov_pol = format!(
        "<{CRE_ACR}#pol> <{acp}allow> <{acl}Write> <{CRE_ACR}> .\n\
         <{CRE_ACR}#pol> <{acp}allOf> <{CRE_ACR}#m> <{CRE_ACR}> .\n\
         <{CRE_ACR}#m> <{acp}agent> <{acp}OwnerAgent> <{CRE_ACR}> .\n"
    );
    let alt_pol = format!(
        "<{CRE_ACR}#altpol> <{acp}allow> <{acl}Read> <{CRE_ACR}> .\n\
         <{CRE_ACR}#altpol> <{acp}allOf> <{CRE_ACR}#altm> <{CRE_ACR}> .\n\
         <{CRE_ACR}#altm> <{acp}agent> <{mallory}> <{CRE_ACR}> .\n"
    );
    let forged = format!("<{CRE_DOC}> <{solidx}owner> <{mallory}> <{CRE_ACR}> .\n");
    let mut s = forged_prov_store(&prov_pol, &alt_pol, &forged);
    assert!(can(&mut s, Some(mallory), None, Mode::Read, CRE_DOC), "mallory keeps her legit Read");
    assert!(
        !can(&mut s, Some(mallory), None, Mode::Write, CRE_DOC),
        "forged `solidx:owner` inside the .acr must NOT self-grant Write (privilege escalation)"
    );
}

/// EXPLOIT 3 (the broader pre-existing class — also affects origin/main): forged
/// `<pol> solidx:appliesToResource <secret>` inside the `.acr`. `appliesToResource` is
/// derived in stratum A from the `acp:accessControl`/`memberAccessControl` chain; if a
/// writer can smuggle it directly, a policy that legitimately grants them on THEIR OWN
/// container is redirected to apply to a resource in a DIFFERENT container that they do
/// not control. Two isolated containers prove this cleanly. `forge/.acr`
/// `memberAccessControl` → `#pol` grants mallory Read, so she legitimately reads
/// `forge/d0.ttl` (a member of `forge/`). `secret/s.ttl` lives in a SEPARATE container
/// with no policy of its own (private by default) and is NOT a member of `forge/`, so
/// `#pol` does not legitimately reach it. The forged
/// `<#pol> solidx:appliesToResource <secret/s.ttl>` inside `forge/.acr` is the ONLY path
/// that could extend the grant onto the secret resource — it must be filtered.
#[test]
fn acp_forged_applies_to_resource_in_acr_document_does_not_grant() {
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acp = "http://www.w3.org/ns/solid/acp#";
    let solidx = "https://sparq.dev/ns/solidx#";
    let mallory = "https://mallory.ex/card#me";
    let forge_acr = "https://pod.ex/forge/.acr";
    let forge_doc = "https://pod.ex/forge/d0.ttl";
    let secret_doc = "https://pod.ex/secret/s.ttl";

    // forge/.acr: memberAccessControl → #pol allow Read to mallory (concrete agent).
    // Plus the forged appliesToResource redirecting #pol onto the unrelated secret_doc.
    let acr = format!(
        "<{forge_doc}#it> <https://ex.dev/ns#title> \"d0\" <{forge_doc}> .\n\
         <{secret_doc}#it> <https://ex.dev/ns#secret> \"top\" <{secret_doc}> .\n\
         <{forge_acr}> <{acp}memberAccessControl> <{forge_acr}#ctl> <{forge_acr}> .\n\
         <{forge_acr}#ctl> <{acp}apply> <{forge_acr}#pol> <{forge_acr}> .\n\
         <{forge_acr}#pol> <{acp}allow> <{acl}Read> <{forge_acr}> .\n\
         <{forge_acr}#pol> <{acp}allOf> <{forge_acr}#m> <{forge_acr}> .\n\
         <{forge_acr}#m> <{acp}agent> <{mallory}> <{forge_acr}> .\n\
         <{forge_acr}#pol> <{solidx}appliesToResource> <{secret_doc}> <{forge_acr}> .\n"
    );
    let g = Graph::load_dataset(&acr, "nquads").expect("loads");
    let mut s = PodStore::new(g);
    s.materialize_acp().expect("materializes (forged appliesToResource filtered, not error)");

    // Sanity: the policy is live — mallory legitimately reads forge/d0.ttl.
    assert!(can(&mut s, Some(mallory), None, Mode::Read, forge_doc), "mallory reads forge/d0.ttl (legit)");
    // The exploit: the forged appliesToResource must NOT leak the grant onto secret/s.ttl.
    assert!(
        !can(&mut s, Some(mallory), None, Mode::Read, secret_doc),
        "forged `solidx:appliesToResource` inside the .acr must NOT redirect a policy onto a secret resource in another container"
    );
}

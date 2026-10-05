//! [SONNET-4.6] sq-ysv3u (issue #2935) — ACP `acp:vc`: credential-gated authorization.
//!
//! The headline guard is [`vc_matcher_denies_when_no_credential_is_presented`]: before this
//! bead `acp:vc` was an attribute the rules did not recognize, so a matcher carrying it looked
//! agent-UNCONSTRAINED, accepted `auth:Public`, and a credential-gated policy granted
//! EVERYONE — anonymous included. That is an over-grant, the wrong-direction error for a
//! security oracle. Deleting the two `acp:vc` accept-set rules from `rules/acp-a.n3` (or the
//! `log:notIncludes { ?m acp:vc ?v }` guard on the pass-through rule) turns that test red.

use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session, VerifiedCredentials};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const DOC: &str = "https://pod.ex/clinic/d0.ttl";
const OVER_18: &str = "https://issuer.ex/req#OverEighteen";
const MEMBER: &str = "https://issuer.ex/req#ClinicMember";

/// A pod whose `/clinic/` policy applies `combinator` over ONE matcher carrying the
/// `acp:vc` requirements `vc` and the `acp:agent` values `agents`.
fn pod(combinator: &str, agents: &[&str], vc: &[&str]) -> String {
    let g = "https://pod.ex/clinic/.acr";
    let mut s = format!("<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .\n");
    s.push_str(&format!("<{g}> <{ACP}memberAccessControl> <{g}#c> <{g}> .\n"));
    s.push_str(&format!("<{g}#c> <{ACP}apply> <{g}#pol> <{g}> .\n"));
    s.push_str(&format!("<{g}#pol> <{ACP}allow> <{ACL}Read> <{g}> .\n"));
    s.push_str(&format!("<{g}#pol> <{ACP}{combinator}> <{g}#m> <{g}> .\n"));
    for a in agents {
        s.push_str(&format!("<{g}#m> <{ACP}agent> <{a}> <{g}> .\n"));
    }
    for c in vc {
        s.push_str(&format!("<{g}#m> <{ACP}vc> <{c}> <{g}> .\n"));
    }
    s
}

fn store(nquads: &str, creds: &VerifiedCredentials) -> PodStore {
    let mut s = PodStore::new(Graph::load_dataset(nquads, "nquads").expect("dataset loads"));
    s.materialize_acp_with_credentials(&AccessProvenance::new(), creds).expect("materializes");
    s
}

fn can_read(s: &PodStore, agent: Option<&str>) -> bool {
    let session = Session { agent, client: None, issuer: None, now: None };
    s.accessible(&session, Mode::Read).iter().any(|g| g.as_str() == DOC)
}

/// THE HEADLINE GUARD. A credential-gated policy with no credential presented grants
/// NOBODY — not the anonymous session, not an authenticated stranger.
#[test]
fn vc_matcher_denies_when_no_credential_is_presented() {
    let s = store(&pod("allOf", &[], &[OVER_18]), &VerifiedCredentials::new());
    assert!(!can_read(&s, None), "anonymous must not read a credential-gated resource");
    assert!(!can_read(&s, Some(ALICE)), "an authenticated non-holder must not read either");
    // …and the policy is not simply inert: the SAME policy grants once a holding exists.
    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, OVER_18);
    let granted = store(&pod("allOf", &[], &[OVER_18]), &creds);
    assert!(can_read(&granted, Some(ALICE)), "the fixture must be able to grant — else vacuous");
}

/// The grant is bound to the agent the credential was verified FOR: a second agent with no
/// holding gains nothing from alice's credential, and anonymous never holds anything.
#[test]
fn vc_grant_is_scoped_to_the_holder() {
    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, OVER_18);
    let s = store(&pod("allOf", &[], &[OVER_18]), &creds);
    assert!(can_read(&s, Some(ALICE)));
    assert!(!can_read(&s, Some(BOB)), "a non-holder must not ride alice's credential");
    assert!(!can_read(&s, None), "anonymous holds no credential");
}

/// A holding for a DIFFERENT requirement does not satisfy the matcher — requirement IRIs are
/// matched exactly, not merely "the agent presented something".
#[test]
fn a_different_requirement_does_not_satisfy_the_matcher() {
    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, MEMBER);
    let s = store(&pod("allOf", &[], &[OVER_18]), &creds);
    assert!(!can_read(&s, Some(ALICE)));
}

/// ACP attributes are conjunctive ACROSS attributes: `acp:agent bob` + `acp:vc <r>` means
/// "bob AND holding <r>", never "bob OR any holder".
#[test]
fn agent_and_vc_on_one_matcher_are_conjunctive() {
    let nq = pod("allOf", &[BOB], &[OVER_18]);

    // bob without the credential: rejected by the vc dimension.
    let s = store(&nq, &VerifiedCredentials::new());
    assert!(!can_read(&s, Some(BOB)), "the acp:agent value alone must not satisfy acp:vc");

    // alice holds the credential but is not the named agent: rejected by the agent dimension.
    let mut alice_only = VerifiedCredentials::new();
    alice_only.hold(ALICE, OVER_18);
    let s = store(&nq, &alice_only);
    assert!(!can_read(&s, Some(ALICE)), "holding the credential must not bypass acp:agent");
    assert!(!can_read(&s, Some(BOB)));

    // both together: granted.
    let mut bob_holds = VerifiedCredentials::new();
    bob_holds.hold(BOB, OVER_18);
    let s = store(&nq, &bob_holds);
    assert!(can_read(&s, Some(BOB)));
    assert!(!can_read(&s, Some(ALICE)));
}

/// `acp:vc` composes conjunctively with the OTHER principal dimensions too: a holder still
/// has to arrive through the matcher's `acp:client` / `acp:issuer` constraints.
#[test]
fn vc_composes_conjunctively_with_client_and_issuer() {
    let g = "https://pod.ex/clinic/.acr";
    let mut nq = pod("allOf", &[], &[OVER_18]);
    nq.push_str(&format!("<{g}#m> <{ACP}client> <https://app.ex> <{g}> .\n"));
    nq.push_str(&format!("<{g}#m> <{ACP}issuer> <https://idp.ex> <{g}> .\n"));

    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, OVER_18);
    let s = store(&nq, &creds);
    let read = |client, issuer| {
        let session = Session { agent: Some(ALICE), client, issuer, now: None };
        s.accessible(&session, Mode::Read).iter().any(|g| g.as_str() == DOC)
    };
    assert!(read(Some("https://app.ex"), Some("https://idp.ex")), "all three dimensions met");
    assert!(!read(Some("https://evil.ex"), Some("https://idp.ex")), "wrong client");
    assert!(!read(Some("https://app.ex"), Some("https://evil.ex")), "wrong issuer");
    assert!(!read(None, None), "the credential alone must not satisfy client/issuer");
}

/// Several `acp:vc` values on ONE matcher are disjunctive (ACP: within an attribute, any
/// value matches) — holding either requirement satisfies it.
#[test]
fn multiple_vc_values_on_one_matcher_are_disjunctive() {
    let nq = pod("allOf", &[], &[OVER_18, MEMBER]);
    for held in [OVER_18, MEMBER] {
        let mut creds = VerifiedCredentials::new();
        creds.hold(ALICE, held);
        assert!(can_read(&store(&nq, &creds), Some(ALICE)), "holding {} should grant", held);
    }
    assert!(!can_read(&store(&nq, &VerifiedCredentials::new()), Some(ALICE)));
}

/// An `acp:vc` matcher used as a `noneOf` EXCEPTION suppresses the grant for holders only —
/// a non-holder keeps the access the policy otherwise gives.
#[test]
fn vc_none_of_excepts_only_the_holder() {
    let g = "https://pod.ex/clinic/.acr";
    let mut nq = format!("<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .\n");
    nq.push_str(&format!("<{g}> <{ACP}memberAccessControl> <{g}#c> <{g}> .\n"));
    nq.push_str(&format!("<{g}#c> <{ACP}apply> <{g}#pol> <{g}> .\n"));
    nq.push_str(&format!("<{g}#pol> <{ACP}allow> <{ACL}Read> <{g}> .\n"));
    // public read …
    nq.push_str(&format!("<{g}#pol> <{ACP}anyOf> <{g}#pub> <{g}> .\n"));
    nq.push_str(&format!("<{g}#pub> <{ACP}agent> <{ACP}PublicAgent> <{g}> .\n"));
    // … EXCEPT holders of the requirement.
    nq.push_str(&format!("<{g}#pol> <{ACP}noneOf> <{g}#no> <{g}> .\n"));
    nq.push_str(&format!("<{g}#no> <{ACP}vc> <{OVER_18}> <{g}> .\n"));

    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, OVER_18);
    let s = store(&nq, &creds);
    assert!(!can_read(&s, Some(ALICE)), "the holder is excepted");
    assert!(can_read(&s, Some(BOB)), "a non-holder keeps the public grant");
}

/// §2.4 trust boundary: a `solidx:holdsVc` triple FORGED inside the `.acr` (a document its
/// author controls) must not establish a holding. Only the `VerifiedCredentials` channel may.
#[test]
fn forged_holds_vc_in_the_acr_grants_nothing() {
    let g = "https://pod.ex/clinic/.acr";
    let mut nq = pod("allOf", &[], &[OVER_18]);
    nq.push_str(&format!(
        "<{ALICE}> <https://sparq.dev/ns/solidx#holdsVc> <{OVER_18}> <{g}> .\n"
    ));
    let s = store(&nq, &VerifiedCredentials::new());
    assert!(!can_read(&s, Some(ALICE)), "a forged solidx:holdsVc must be dropped by the loader");
}

/// A credential holder's WebID is a candidate agent principal, so it goes through the same
/// reserved-encoding validation as every other principal — a holder inside `urn:sparq:` space
/// (which could impersonate a minted pair/triple principal) fails materialization.
#[test]
fn reserved_holder_webid_is_rejected() {
    let mut creds = VerifiedCredentials::new();
    creds.hold("urn:sparq:pair?agent=x&client=y", OVER_18);
    let mut s = PodStore::new(
        Graph::load_dataset(&pod("allOf", &[], &[OVER_18]), "nquads").expect("dataset loads"),
    );
    let err = s
        .materialize_acp_with_credentials(&AccessProvenance::new(), &creds)
        .expect_err("a reserved-space holder must be refused");
    assert!(err.contains("reserved"), "got: {}", err);
}

/// Strict additivity: supplying credentials must not change the auth view of a pod whose
/// policies use no `acp:vc` at all.
#[test]
fn credentials_do_not_disturb_a_pod_without_acp_vc() {
    let nq = sparq_solid::acp_fixture();
    let plain = store(&nq, &VerifiedCredentials::new());
    let mut creds = VerifiedCredentials::new();
    creds.hold(ALICE, OVER_18);
    creds.hold(BOB, MEMBER);
    let with_creds = store(&nq, &creds);

    let target = "https://pod.ex/pub1/c2/g3/d1.ttl";
    for agent in [None, Some(ALICE), Some(BOB)] {
        let session = Session { agent, client: None, issuer: None, now: None };
        assert_eq!(
            plain.accessible(&session, Mode::Read).iter().any(|g| g.as_str() == target),
            with_creds.accessible(&session, Mode::Read).iter().any(|g| g.as_str() == target),
            "credentials changed a non-acp:vc decision for {:?}",
            agent
        );
    }
}

//! sq-t58w.6 — the ACP parity corpus, extracted verbatim from
//! `conformance_acp.rs` so a second test target can consume the IDENTICAL scenarios.
//! No scenario, emitted `.acr` shape, or expected decision is changed by the move.
//!
//! The conformance surface is the Solid ACP spec at the library level — the binary
//! `(agent, client, mode, resource) → allow | deny` decision, NOT HTTP wire conformance.

use super::{ALICE, APP, BOB, CAROL, EVIL};
use sparq_solid::conformance::{
    AcpScenario, AcrBuilder, Decision, Expect, AUTHENTICATED_AGENT, PUBLIC_AGENT,
};
use sparq_solid::Mode;

/// ACP §"Matchers" + §"Policies": an `acp:anyOf` agent matcher grants the enumerated
/// agent and nobody else; the anonymous requestor is refused.
fn agent_anyof() -> AcpScenario {
    let doc = "https://pod.example/agent/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| p.allow(Mode::Read).any_of_agent(ALICE));
    acr.document(doc);
    AcpScenario::new("agent-anyOf-allow")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// ACP §"Matchers": `acp:PublicAgent` accepts EVERY requestor, including the anonymous
/// one and any client.
fn public_agent() -> AcpScenario {
    let doc = "https://pod.example/public/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| p.allow(Mode::Read).any_of_agent(PUBLIC_AGENT));
    acr.document(doc);
    AcpScenario::new("public-agent")
        .acr(acr)
        .expect(Expect::anonymous().read(doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::pair(BOB, APP).read(doc).is(Decision::Allow))
}

/// ACP §"Matchers": `acp:AuthenticatedAgent` accepts any authenticated requestor but
/// NOT the anonymous one.
fn authenticated_agent() -> AcpScenario {
    let doc = "https://pod.example/authn/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| {
        p.allow(Mode::Read).any_of_agent(AUTHENTICATED_AGENT)
    });
    acr.document(doc);
    AcpScenario::new("authenticated-agent")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// ACP §"Policies" `acp:allOf`: the native (user, application) PAIR — agent BOB AND
/// client APP must BOTH hold. A right agent with the wrong client (or no client) fails;
/// a wrong agent with the right client fails.
fn allof_pair() -> AcpScenario {
    let doc = "https://pod.example/pair/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| {
        p.allow(Mode::Read).all_of_agent(BOB).all_of_client(APP)
    });
    acr.document(doc);
    AcpScenario::new("allOf-user-app-pair")
        .acr(acr)
        .expect(Expect::pair(BOB, APP).read(doc).is(Decision::Allow))
        .expect(Expect::pair(BOB, EVIL).read(doc).is(Decision::Deny))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny))
        .expect(Expect::pair(CAROL, APP).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// ACP §"Policies" `acp:allOf` on a single matcher carrying both `acp:agent` and
/// `acp:client`: same (user, app) pair semantics, one matcher.
fn allof_single_matcher_pair() -> AcpScenario {
    let doc = "https://pod.example/pair2/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| p.allow(Mode::Read).all_of_pair(BOB, APP));
    acr.document(doc);
    AcpScenario::new("allOf-single-matcher-pair")
        .acr(acr)
        .expect(Expect::pair(BOB, APP).read(doc).is(Decision::Allow))
        .expect(Expect::pair(BOB, EVIL).read(doc).is(Decision::Deny))
        .expect(Expect::pair(CAROL, APP).read(doc).is(Decision::Deny))
}

/// ACP §"Access Modes" deny-overrides: a public allow plus a deny for one agent — the
/// denied agent loses access, everyone else keeps it. (Allow and deny are both
/// materialized as triples; the session layer computes allow ∖ deny.)
fn deny_overrides() -> AcpScenario {
    let doc = "https://pod.example/deny/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| p.allow(Mode::Read).any_of_agent(PUBLIC_AGENT));
    acr.access_control(doc, |p| p.deny(Mode::Read).any_of_agent(BOB));
    acr.document(doc);
    AcpScenario::new("deny-overrides")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::anonymous().read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny))
}

/// ACP §"Matchers" `acp:noneOf` (the "except" carve-out): public read EXCEPT carol.
/// Evaluated per session as a conditional grant.
fn none_of() -> AcpScenario {
    let doc = "https://pod.example/except/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| {
        p.allow(Mode::Read)
            .any_of_agent(PUBLIC_AGENT)
            .none_of_agent(CAROL)
    });
    acr.document(doc);
    AcpScenario::new("noneOf-public-except-carol")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::anonymous().read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::agent(CAROL).read(doc).is(Decision::Deny))
}

/// ACP §"Effective Policies": `acp:memberAccessControl` on an ancestor applies to its
/// members (IRI-structural descendants) but NOT to the ancestor resource itself, while
/// `acp:accessControl` applies to the resource itself only. Here the container grants
/// alice on members; a deep document inherits it, the container's own resource does not.
fn member_access_control_inheritance() -> AcpScenario {
    let container = "https://pod.example/team/";
    let deep = "https://pod.example/team/sub/d1";
    let mut acr = AcrBuilder::new();
    // memberAccessControl on the container: applies to descendants.
    acr.member_access_control(container, |p| p.allow(Mode::Read).any_of_agent(ALICE));
    acr.document(deep); // the protected descendant
    acr.document(container); // make the container a concrete resource too
    AcpScenario::new("memberAccessControl-inheritance")
        .acr(acr)
        // alice reads the descendant (inherited member policy)
        .expect(Expect::agent(ALICE).read(deep).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(deep).is(Decision::Deny))
        // the container resource itself is NOT a member of itself: memberAccessControl
        // does not grant it (no accessControl present), so alice cannot read it.
        .expect(Expect::agent(ALICE).read(container).is(Decision::Deny))
}

/// ACP §"Effective Policies", CUMULATIVE inheritance (the key WAC difference): a deeper
/// resource accrues EVERY ancestor's memberAccessControl. The root grants alice; a
/// nested container ALSO grants bob; a deep document under it is readable by BOTH —
/// the nested ACR does not shadow the root (cumulative, not nearest-wins).
fn cumulative_inheritance() -> AcpScenario {
    let root = "https://pod.example/";
    let mid = "https://pod.example/proj/";
    let deep = "https://pod.example/proj/c/d1";
    let mut acr = AcrBuilder::new();
    acr.member_access_control(root, |p| p.allow(Mode::Read).any_of_agent(ALICE));
    acr.member_access_control(mid, |p| p.allow(Mode::Read).any_of_agent(BOB));
    acr.document(deep);
    AcpScenario::new("cumulative-inheritance")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(deep).is(Decision::Allow)) // root, cumulative
        .expect(Expect::agent(BOB).read(deep).is(Decision::Allow)) // mid, cumulative
        .expect(Expect::agent(CAROL).read(deep).is(Decision::Deny))
}

/// ACP §"Access Modes": the four modes are independent. A policy that allows Read+Write
/// grants both; a Read-only policy denies Write; Control is a separate mode that (per
/// the rules) governs the ACR document, NOT the resource — so a Control grant does not
/// by itself make the resource readable.
fn mode_independence() -> AcpScenario {
    let rw = "https://pod.example/modes/rw";
    let ro = "https://pod.example/modes/ro";
    let mut acr = AcrBuilder::new();
    acr.access_control(rw, |p| {
        p.allow(Mode::Read).allow(Mode::Write).any_of_agent(ALICE)
    });
    acr.access_control(ro, |p| p.allow(Mode::Read).any_of_agent(ALICE));
    acr.document(rw);
    acr.document(ro);
    AcpScenario::new("mode-independence")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(rw).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(rw).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(ro).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(ro).is(Decision::Deny))
}

/// sq-61uvs: the ACP analogue exercises the ACR as a resource. Unlike WAC,
/// ACP does not translate Control on the governed resource into Read/Write on its ACR;
/// nor may a member Read policy disclose the control document.
fn acr_document_control_gate() -> AcpScenario {
    let container = "https://pod.example/control/";
    let acr_doc = "https://pod.example/control/.acr";
    let mut acr = AcrBuilder::new();
    acr.access_control(container, |p| p.allow(Mode::Control).any_of_agent(ALICE));
    acr.member_access_control(container, |p| p.allow(Mode::Read).any_of_agent(BOB));
    acr.document(container);
    AcpScenario::new("acr-document-control-gate")
        .acr(acr)
        .expect(Expect::agent(ALICE).control(container).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(acr_doc).is(Decision::Deny))
        .expect(Expect::agent(ALICE).write(acr_doc).is(Decision::Deny))
        .expect(Expect::agent(BOB).read(acr_doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(acr_doc).is(Decision::Deny))
}

/// ACP fail-closed: a resource with NO access control grants nobody (not even the
/// anonymous requestor, not any authenticated agent). An unprotected resource is
/// invisible.
fn fail_closed_no_policy() -> AcpScenario {
    let doc = "https://pod.example/orphan/d1";
    let mut acr = AcrBuilder::new();
    acr.document(doc); // present, but no ACR controls it
    AcpScenario::new("fail-closed-no-policy")
        .acr(acr)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// ACP `acp:anyOf` is a disjunction: enumerating two agents grants EITHER, not BOTH-
/// required. (ACP has no groups, so a group is modelled by enumerating members.)
fn anyof_disjunction() -> AcpScenario {
    let doc = "https://pod.example/anyof/d1";
    let mut acr = AcrBuilder::new();
    acr.access_control(doc, |p| {
        p.allow(Mode::Read)
            .allow(Mode::Write)
            .any_of_agent(BOB)
            .any_of_agent(CAROL)
    });
    acr.document(doc);
    AcpScenario::new("anyOf-disjunction")
        .acr(acr)
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::agent(CAROL).write(doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Deny))
}

/// The full ACP corpus — the single reusable source consumed by `conformance_acp.rs` and
/// the differential oracle.
pub fn acp_corpus() -> Vec<AcpScenario> {
    vec![
        agent_anyof(),
        public_agent(),
        authenticated_agent(),
        allof_pair(),
        allof_single_matcher_pair(),
        deny_overrides(),
        none_of(),
        member_access_control_inheritance(),
        cumulative_inheritance(),
        mode_independence(),
        acr_document_control_gate(),
        fail_closed_no_policy(),
        anyof_disjunction(),
    ]
}

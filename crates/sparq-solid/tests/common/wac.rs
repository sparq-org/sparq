//! sq-t58w.6 — the WAC parity corpus, extracted verbatim from
//! `conformance_wac.rs` so a second test target can consume the IDENTICAL scenarios.
//! No scenario, emitted `.acl` shape, or expected decision is changed by the move.
//!
//! The scenarios are derived from the WAC spec's normative semantics, NOT vendored over
//! HTTP from the live `solid/specification-tests` / CTH corpus — that corpus asserts on
//! HTTP-shaped outputs (status codes, the `WAC-Allow` header) which have no library entry
//! point here. The decision table below is the authorization *content* of those tests (the
//! `.acl` shapes and their expected allow/deny per principal), which is exactly what an
//! authorization oracle can reproduce.

use super::{ALICE, APP, BOB, CAROL, DAVE, EVIL};
use sparq_solid::conformance::{Decision, Expect};
use sparq_solid::wac_conformance::{AclBuilder, WacScenario, AUTHENTICATED_AGENT, PUBLIC_AGENT};
use sparq_solid::Mode;

/// WAC §"Access subjects" `acl:agent` + §"Access modes" `acl:accessTo`: an authorization
/// on the resource's own ACL grants the named agent and nobody else; the anonymous
/// requestor is refused (fail-closed).
fn agent_access_to() -> WacScenario {
    let doc = "https://pod.example/agent/d1";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.agent(ALICE).mode(Mode::Read));
    acl.document(doc);
    WacScenario::new("agent-accessTo")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// WAC §"Access subjects" `acl:agentClass foaf:Agent` (the public class): grants EVERY
/// requestor including the anonymous one and any client.
fn public_agent_class() -> WacScenario {
    let doc = "https://pod.example/public/d1";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.public().mode(Mode::Read));
    acl.document(doc);
    WacScenario::new("agentClass-foaf-Agent-public")
        .acl(acl)
        .expect(Expect::anonymous().read(doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::pair(BOB, APP).read(doc).is(Decision::Allow))
        // sanity: the `PUBLIC_AGENT` constant is `foaf:Agent`.
        .expect(Expect::agent(CAROL).read(doc).is(
            if PUBLIC_AGENT == "http://xmlns.com/foaf/0.1/Agent" {
                Decision::Allow
            } else {
                Decision::Deny
            },
        ))
}

/// WAC §"Access subjects" `acl:agentClass acl:AuthenticatedAgent`: grants any *authenticated*
/// requestor but NOT the anonymous one.
fn authenticated_agent_class() -> WacScenario {
    let doc = "https://pod.example/authn/d1";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.agent_class(AUTHENTICATED_AGENT).mode(Mode::Read));
    acl.document(doc);
    WacScenario::new("agentClass-AuthenticatedAgent")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// WAC §"Access subjects" `acl:agentGroup` + `vcard:hasMember`: every group member is
/// granted; a non-member is not. (The group document is the group IRI with the fragment
/// stripped — the loader's resolution convention.)
fn agent_group() -> WacScenario {
    let doc = "https://pod.example/team/d1";
    let group = "https://pod.example/groups/team#g";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| {
        a.agent_group(group, &[BOB, CAROL])
            .mode(Mode::Read)
            .mode(Mode::Write)
    });
    acl.document(doc);
    WacScenario::new("agentGroup-vcard-members")
        .acl(acl)
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::agent(CAROL).read(doc).is(Decision::Allow))
        // group grant carries Write too (independent mode).
        .expect(Expect::agent(BOB).write(doc).is(Decision::Allow))
        .expect(Expect::agent(DAVE).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// WAC §"ACL resolution": `acl:default` on a container inherits to its members (its
/// IRI-structural descendants) but NOT to the container resource itself, while
/// `acl:accessTo` applies to the resource named by the ACL only. Here a container's ACL
/// grants alice on members via `acl:default`; a deep document inherits it, the container's
/// own resource does not.
fn default_inheritance_vs_access_to() -> WacScenario {
    let container = "https://pod.example/c/";
    let deep = "https://pod.example/c/sub/d1";
    let mut acl = AclBuilder::new();
    // acl:default on the container: inherited by descendants, not the container itself.
    acl.default_for(container, |a| a.agent(ALICE).mode(Mode::Read));
    acl.document(deep); // the protected descendant
    acl.document(container); // make the container a concrete resource + inheritance anchor
    WacScenario::new("default-inheritance-vs-accessTo")
        .acl(acl)
        // alice reads the descendant (inherited acl:default)
        .expect(Expect::agent(ALICE).read(deep).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(deep).is(Decision::Deny))
        // the container resource itself is not a member of itself: acl:default does not
        // grant it (no acl:accessTo present), so alice cannot read the container.
        .expect(Expect::agent(ALICE).read(container).is(Decision::Deny))
}

/// WAC §"ACL resolution" NEAREST-ACL (the key WAC difference from ACP's cumulative
/// inheritance): a resource is governed by exactly the **nearest** ancestor that has an
/// ACL — a closer ACL fully SHADOWS the ancestors above it. Root grants alice on all
/// members; a nested container restates an ACL granting only bob; a deep document under
/// the nested container is readable by BOB ONLY — alice's root grant is shadowed (NOT
/// cumulative).
fn nearest_acl_shadows() -> WacScenario {
    let root = "https://pod.example/";
    let mid = "https://pod.example/proj/";
    let deep = "https://pod.example/proj/c/d1";
    let mut acl = AclBuilder::new();
    acl.default_for(root, |a| a.agent(ALICE).mode(Mode::Read));
    acl.default_for(mid, |a| a.agent(BOB).mode(Mode::Read));
    acl.document(deep);
    WacScenario::new("nearest-acl-shadows-ancestor")
        .acl(acl)
        // bob via the NEAREST acl (mid)
        .expect(Expect::agent(BOB).read(deep).is(Decision::Allow))
        // alice's root grant is SHADOWED by the closer mid ACL — denied.
        .expect(Expect::agent(ALICE).read(deep).is(Decision::Deny))
        .expect(Expect::agent(CAROL).read(deep).is(Decision::Deny))
}

/// WAC §"ACL resolution": a resource-specific own ACL (`acl:accessTo` on the document
/// itself) overrides the container's `acl:default` for THAT document only — a sibling
/// without its own ACL still inherits the container default.
fn resource_specific_override() -> WacScenario {
    let container = "https://pod.example/docs/";
    let overridden = "https://pod.example/docs/d0";
    let sibling = "https://pod.example/docs/d1";
    let mut acl = AclBuilder::new();
    // container default: alice on members
    acl.default_for(container, |a| a.agent(ALICE).mode(Mode::Read));
    acl.document(container);
    // overridden document has its OWN acl granting bob (shadows the container default)
    acl.access_to(overridden, |a| a.agent(BOB).mode(Mode::Read));
    acl.document(overridden);
    acl.document(sibling); // no own acl -> inherits the container default
    WacScenario::new("resource-specific-override")
        .acl(acl)
        // the overridden doc: bob only (its own ACL shadows the container default)
        .expect(Expect::agent(BOB).read(overridden).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(overridden).is(Decision::Deny))
        // the sibling still inherits the container default: alice only
        .expect(Expect::agent(ALICE).read(sibling).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(sibling).is(Decision::Deny))
}

/// WAC §"Access subjects" `acl:origin`: the grant is a (user, application) PAIR — the agent
/// is granted only when the session presents that origin/client. The right agent with the
/// wrong client (or no client) is refused; a wrong agent with the right client is refused.
fn origin_user_app_pair() -> WacScenario {
    let doc = "https://pod.example/origin/d1";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.agent(BOB).origin(APP).mode(Mode::Read));
    acl.document(doc);
    WacScenario::new("origin-user-app-pair")
        .acl(acl)
        .expect(Expect::pair(BOB, APP).read(doc).is(Decision::Allow))
        .expect(Expect::pair(BOB, EVIL).read(doc).is(Decision::Deny))
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny)) // no client
        .expect(Expect::pair(CAROL, APP).read(doc).is(Decision::Deny)) // wrong agent
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// WAC §"Access modes": the four modes are independent. An authorization that names
/// Read+Write grants both; a Read-only authorization denies Write/Append.
fn mode_independence() -> WacScenario {
    let rw = "https://pod.example/modes/rw";
    let ro = "https://pod.example/modes/ro";
    let mut acl = AclBuilder::new();
    acl.access_to(rw, |a| a.agent(ALICE).mode(Mode::Read).mode(Mode::Write));
    acl.access_to(ro, |a| a.agent(ALICE).mode(Mode::Read));
    acl.document(rw);
    acl.document(ro);
    WacScenario::new("mode-independence")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(rw).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(rw).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(ro).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(ro).is(Decision::Deny))
        .expect(Expect::agent(ALICE).append(ro).is(Decision::Deny))
}

/// WAC §"Access modes" `acl:Control`: Control governs the resource's **ACL document**, not
/// the resource. A Control grant on the resource makes the holder Read+Write the
/// `<resource>.acl` graph (and Control on the resource itself), but does NOT by itself make
/// the resource readable. Here alice has Read+Control on the doc; she can Read the doc (via
/// Read) and Write its `.acl` (via Control), while bob — with neither — can do neither.
fn control_governs_acl_document() -> WacScenario {
    let doc = "https://pod.example/ctl/d1";
    let acl_doc = "https://pod.example/ctl/d1.acl";
    let mut acl = AclBuilder::new();
    acl.access_to(doc, |a| a.agent(ALICE).mode(Mode::Read).mode(Mode::Control));
    acl.document(doc);
    WacScenario::new("control-governs-acl-document")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).control(doc).is(Decision::Allow))
        // Control -> Read+Write of the .acl graph
        .expect(Expect::agent(ALICE).read(acl_doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(acl_doc).is(Decision::Allow))
        // bob holds nothing
        .expect(Expect::agent(BOB).read(doc).is(Decision::Deny))
        .expect(Expect::agent(BOB).read(acl_doc).is(Decision::Deny))
}

/// sq-61uvs: an ACL document is never an ordinary descendant for
/// `acl:default` inheritance. Only Control on the governed resource exposes it.
fn acl_document_control_gate_blocks_default_and_anonymous() -> WacScenario {
    let container = "https://pod.example/control/";
    let acl_doc = "https://pod.example/control/.acl";
    let mut acl = AclBuilder::new();
    acl.access_to(container, |a| a.agent(ALICE).mode(Mode::Control));
    acl.default_for(container, |a| a.agent(BOB).mode(Mode::Read));
    acl.document(container);
    WacScenario::new("acl-document-control-gate")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(acl_doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).write(acl_doc).is(Decision::Allow))
        .expect(Expect::agent(BOB).read(acl_doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(acl_doc).is(Decision::Deny))
}

/// WAC fail-closed: a resource with NO applicable ACL (no own ACL, no ancestor ACL) grants
/// nobody — not the anonymous requestor, not any authenticated agent. An unprotected
/// resource is invisible.
fn fail_closed_no_acl() -> WacScenario {
    let doc = "https://pod.example/orphan/d1";
    let mut acl = AclBuilder::new();
    acl.document(doc); // present, but no ACL controls it (and no ancestor ACL)
    WacScenario::new("fail-closed-no-acl")
        .acl(acl)
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Deny))
        .expect(Expect::anonymous().read(doc).is(Decision::Deny))
}

/// WAC: multiple `acl:agent`s in one authorization is a disjunction — EITHER named agent is
/// granted (the grant is the union of its access subjects). Two authorizations in one ACL
/// likewise union.
fn multi_agent_union() -> WacScenario {
    let doc = "https://pod.example/multi/d1";
    let mut acl = AclBuilder::new();
    // one authorization, two agents
    acl.access_to(doc, |a| {
        a.agent(BOB).agent(CAROL).mode(Mode::Read).mode(Mode::Write)
    });
    acl.document(doc);
    WacScenario::new("multi-agent-union")
        .acl(acl)
        .expect(Expect::agent(BOB).read(doc).is(Decision::Allow))
        .expect(Expect::agent(CAROL).write(doc).is(Decision::Allow))
        .expect(Expect::agent(ALICE).read(doc).is(Decision::Deny))
}

/// The full WAC corpus — the single reusable source consumed by `conformance_wac.rs` and
/// the differential oracle.
pub fn wac_corpus() -> Vec<WacScenario> {
    vec![
        agent_access_to(),
        public_agent_class(),
        authenticated_agent_class(),
        agent_group(),
        default_inheritance_vs_access_to(),
        nearest_acl_shadows(),
        resource_specific_override(),
        origin_user_app_pair(),
        mode_independence(),
        control_governs_acl_document(),
        acl_document_control_gate_blocks_default_and_anonymous(),
        fail_closed_no_acl(),
        multi_agent_union(),
    ]
}

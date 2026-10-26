//! Synthetic Solid pod fixtures (N-Quads): ~1000 documents in a depth-4 containment
//! tree under `https://pod.ex/`, with a WAC variant (`.acl` graphs at ~10% of
//! containers + one resource-specific ACL) and an ACP variant (`.acr` graphs).
//!
//! Tree: 6 depth-1 containers × 6 × 6, each leaf container holding 4 documents
//! → 864 documents + 259 containers. The depth-1 subtrees exercise distinct policy
//! shapes (see tests/): priv0 (owner-only), pub1 (public), team2 (group), friends3
//! (agent grant / user-app pair), mixed4 (authenticated + deep overrides + ACP deny),
//! origin5 (origin pair / ACP noneOf).

use std::fmt::Write;

/// The pod root container IRI.
pub const POD: &str = "https://pod.ex/";
/// The pod owner's WebID (Read/Write/Control almost everywhere — but deliberately
/// shadowed out of `team2/` in the WAC fixture by its nearest-ACL).
pub const ALICE: &str = "https://alice.ex/card#me";
/// WebID of a team-group member who also holds the `friends3/` agent grant and the
/// `origin5/` client-restricted grant (WAC) / `friends3/` allOf pair (ACP).
pub const BOB: &str = "https://bob.ex/card#me";
/// WebID of a team-group member; holds the `mixed4/c0/` deep override (WAC) and is
/// the `origin5/` noneOf exception (ACP).
pub const CAROL: &str = "https://carol.ex/card#me";
/// WebID with no grants of its own; target of the `mixed4/` ACP deny (deny-overrides).
pub const DAVE: &str = "https://dave.ex/card#me";
/// The client identifier (WAC `acl:origin` / ACP `acp:client`) used by the
/// pair-principal grants.
pub const APP: &str = "https://app.ex";
/// The vcard group document (a pod resource/named graph itself); the group IRI is
/// `<{TEAM_GROUP_DOC}#g>` with members [`BOB`] and [`CAROL`].
pub const TEAM_GROUP_DOC: &str = "https://pod.ex/groups/team";

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";
const VCARD: &str = "http://www.w3.org/2006/vcard/ns#";
const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";
const EX: &str = "https://ex.dev/ns#";

const SUBTREES: [&str; 6] = ["priv0", "pub1", "team2", "friends3", "mixed4", "origin5"];

fn quad(out: &mut String, s: &str, p: &str, o: &str, g: &str) {
    let _ = writeln!(out, "<{s}> <{p}> <{o}> <{g}> .");
}

fn quad_lit(out: &mut String, s: &str, p: &str, o: &str, g: &str) {
    let _ = writeln!(out, "<{s}> <{p}> \"{o}\" <{g}> .");
}

/// The shared containment tree + document contents (no access control).
/// `extra` = additional filler triples per document (scaling the per-query copy cost
/// measurement; access-control semantics are unaffected).
fn tree_sized(out: &mut String, extra: usize) {
    tree(out);
    if extra == 0 {
        return;
    }
    for top in SUBTREES {
        for j in 0..6 {
            for k in 0..6 {
                for d in 0..4 {
                    let doc = format!("{POD}{top}/c{j}/g{k}/d{d}.ttl");
                    for n in 0..extra {
                        quad_lit(out, &format!("{doc}#it"), &format!("{EX}note{n}"), &format!("filler {n}"), &doc);
                    }
                }
            }
        }
    }
}

fn tree(out: &mut String) {
    for (i, top) in SUBTREES.iter().enumerate() {
        let c1 = format!("{POD}{top}/");
        quad(out, POD, LDP_CONTAINS, &c1, POD);
        for j in 0..6 {
            let c2 = format!("{c1}c{j}/");
            quad(out, &c1, LDP_CONTAINS, &c2, &c1);
            for k in 0..6 {
                let c3 = format!("{c2}g{k}/");
                quad(out, &c2, LDP_CONTAINS, &c3, &c2);
                for d in 0..4 {
                    let doc = format!("{c3}d{d}.ttl");
                    quad(out, &c3, LDP_CONTAINS, &doc, &c3);
                    let it = format!("{doc}#it");
                    quad_lit(out, &it, &format!("{EX}title"), &format!("doc {i}-{j}-{k}-{d}"), &doc);
                    quad(out, &it, &format!("{EX}inSubtree"), &format!("{POD}{top}/"), &doc);
                }
            }
        }
    }
    // the group document is a pod resource too
    let g = format!("{TEAM_GROUP_DOC}#g");
    quad(out, &g, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", &format!("{VCARD}Group"), TEAM_GROUP_DOC);
    quad(out, &g, &format!("{VCARD}hasMember"), BOB, TEAM_GROUP_DOC);
    quad(out, &g, &format!("{VCARD}hasMember"), CAROL, TEAM_GROUP_DOC);
}

/// One WAC Authorization. `target`: (accessTo?, default?) on `container`.
struct Auth<'a> {
    frag: &'a str,
    access_to: bool,
    default: bool,
    agents: &'a [&'a str],
    agent_classes: &'a [&'a str],
    agent_groups: &'a [&'a str],
    origin: Option<&'a str>,
    modes: &'a [&'a str],
}

impl Default for Auth<'_> {
    fn default() -> Self {
        Auth {
            frag: "a",
            access_to: true,
            default: true,
            agents: &[],
            agent_classes: &[],
            agent_groups: &[],
            origin: None,
            modes: &["Read"],
        }
    }
}

fn acl_doc(out: &mut String, resource: &str, auths: &[Auth]) {
    let g = format!("{resource}.acl");
    for a in auths {
        let s = format!("{g}#{}", a.frag);
        quad(out, &s, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", &format!("{ACL}Authorization"), &g);
        if a.access_to {
            quad(out, &s, &format!("{ACL}accessTo"), resource, &g);
        }
        if a.default {
            quad(out, &s, &format!("{ACL}default"), resource, &g);
        }
        for ag in a.agents {
            quad(out, &s, &format!("{ACL}agent"), ag, &g);
        }
        for c in a.agent_classes {
            quad(out, &s, &format!("{ACL}agentClass"), c, &g);
        }
        for grp in a.agent_groups {
            quad(out, &s, &format!("{ACL}agentGroup"), grp, &g);
        }
        if let Some(o) = a.origin {
            quad(out, &s, &format!("{ACL}origin"), o, &g);
        }
        for m in a.modes {
            quad(out, &s, &format!("{ACL}mode"), &format!("{ACL}{m}"), &g);
        }
    }
}

/// The WAC fixture (N-Quads). ACLs at root + every depth-1 container (semantics below)
/// + restating ACLs at ~10% of containers + one resource-specific ACL.
///
/// # Examples
///
/// ```
/// let graph = sparq_core::Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads")?;
/// assert_eq!(graph.named().len(), 1148); // 864 docs + 259 containers + ACLs + group doc
/// # Ok::<(), String>(())
/// ```
///
/// Materializing it takes ~1 s in release mode (see `examples/bench.rs`), so prefer
/// the tiny inline pods from the crate-level docs for fast tests.
pub fn wac_fixture() -> String {
    wac_fixture_sized(0)
}

/// [`wac_fixture`] with `extra` filler triples per document (copy-cost scaling).
///
/// # Examples
///
/// ```no_run
/// // ~46k quads: scales the v1 per-query copy cost without changing access semantics
/// let fat = sparq_solid::fixture::wac_fixture_sized(50);
/// # let _ = fat;
/// ```
pub fn wac_fixture_sized(extra: usize) -> String {
    let mut out = String::with_capacity(1 << 20);
    tree_sized(&mut out, extra);
    let rwc: &[&str] = &["Read", "Write", "Control"];
    let owner = |frag| Auth { frag, agents: &[ALICE], modes: rwc, ..Auth::default() };

    // root: alice owns everything (until a deeper ACL shadows it)
    acl_doc(&mut out, POD, &[owner("owner")]);
    // pub1: public read + owner re-grant
    acl_doc(&mut out, &format!("{POD}pub1/"), &[
        Auth { frag: "pub", agent_classes: &[FOAF_AGENT], ..Auth::default() },
        owner("owner"),
    ]);
    // team2: GROUP read+write, NO alice re-grant (nearest-ACL shadows the root owner!)
    acl_doc(&mut out, &format!("{POD}team2/"), &[Auth {
        frag: "team",
        agent_groups: &[&format!("{TEAM_GROUP_DOC}#g")],
        modes: &["Read", "Write"],
        ..Auth::default()
    }]);
    // friends3: bob may read MEMBERS (default only); alice owns the container (accessTo only)
    acl_doc(&mut out, &format!("{POD}friends3/"), &[
        Auth { frag: "bob", access_to: false, agents: &[BOB], ..Auth::default() },
        Auth { frag: "owner", default: false, agents: &[ALICE], modes: rwc, ..Auth::default() },
    ]);
    // mixed4: any AUTHENTICATED agent reads members; alice owns
    acl_doc(&mut out, &format!("{POD}mixed4/"), &[
        Auth { frag: "authd", agent_classes: &[&format!("{ACL}AuthenticatedAgent")], ..Auth::default() },
        owner("owner"),
    ]);
    // origin5: bob ONLY through client/origin https://app.ex; alice owns
    acl_doc(&mut out, &format!("{POD}origin5/"), &[
        Auth { frag: "app", access_to: false, agents: &[BOB], origin: Some(APP), ..Auth::default() },
        owner("owner"),
    ]);
    // deep override: mixed4/c0 — carol only (alice + authenticated lose this subtree)
    acl_doc(&mut out, &format!("{POD}mixed4/c0/"), &[Auth {
        frag: "deep",
        agents: &[CAROL],
        ..Auth::default()
    }]);
    // resource-specific override: ONE document readable by bob only
    acl_doc(&mut out, &format!("{POD}mixed4/c1/g0/d0.ttl"), &[Auth {
        frag: "doc",
        default: false,
        agents: &[BOB],
        ..Auth::default()
    }]);
    // deeper container ACLs (~10% of the 259 containers have an own ACL). Most RESTATE
    // the parent's effective semantics; some deliberately diverge (nearest-ACL coverage):
    //   • friends3/c0,c1 WIDEN — alice gains acl:default rights she lacks at friends3/
    //   • mixed4/c2 + origin5/c0,c1 NARROW to owner-only — authenticated agents
    //     (resp. the bob+app pair) lose those subtrees
    for c in ["priv0/c0/", "priv0/c1/", "priv0/c2/", "priv0/c3/"] {
        acl_doc(&mut out, &format!("{POD}{c}"), &[owner("owner")]);
    }
    for c in ["pub1/c0/", "pub1/c1/", "pub1/c3/", "pub1/c4/"] {
        acl_doc(&mut out, &format!("{POD}{c}"), &[
            Auth { frag: "pub", agent_classes: &[FOAF_AGENT], ..Auth::default() },
            owner("owner"),
        ]);
    }
    for c in ["team2/c0/", "team2/c1/", "team2/c2/"] {
        acl_doc(&mut out, &format!("{POD}{c}"), &[Auth {
            frag: "team",
            agent_groups: &[&format!("{TEAM_GROUP_DOC}#g")],
            modes: &["Read", "Write"],
            ..Auth::default()
        }]);
    }
    for c in ["friends3/c0/", "friends3/c1/"] {
        acl_doc(&mut out, &format!("{POD}{c}"), &[
            Auth { frag: "bob", agents: &[BOB], ..Auth::default() },
            owner("owner"),
        ]);
    }
    for c in ["mixed4/c2/", "origin5/c0/", "origin5/c1/"] {
        acl_doc(&mut out, &format!("{POD}{c}"), &[owner("owner")]);
    }
    out
}

/// One ACP policy: matchers are written inline into the ACR graph.
fn acr_doc(
    out: &mut String,
    resource: &str,
    // (frag, member?, allow-modes, deny-modes, allOf agent/client pairs, anyOf, noneOf)
    policies: &[AcpPolicy],
) {
    let g = format!("{resource}.acr");
    quad(out, &g, &format!("{ACP}resource"), resource, &g);
    for p in policies {
        let control = format!("{g}#ctl-{}", p.frag);
        let pred = if p.member { "memberAccessControl" } else { "accessControl" };
        quad(out, &g, &format!("{ACP}{pred}"), &control, &g);
        let pol = format!("{g}#pol-{}", p.frag);
        quad(out, &control, &format!("{ACP}apply"), &pol, &g);
        for m in p.allow {
            quad(out, &pol, &format!("{ACP}allow"), &format!("{ACL}{m}"), &g);
        }
        for m in p.deny {
            quad(out, &pol, &format!("{ACP}deny"), &format!("{ACL}{m}"), &g);
        }
        for (combinator, matchers) in
            [("allOf", p.all_of), ("anyOf", p.any_of), ("noneOf", p.none_of)]
        {
            for (i, (agent, client)) in matchers.iter().enumerate() {
                let m = format!("{g}#m-{}-{combinator}{i}", p.frag);
                quad(out, &pol, &format!("{ACP}{combinator}"), &m, &g);
                if let Some(a) = agent {
                    quad(out, &m, &format!("{ACP}agent"), a, &g);
                }
                if let Some(c) = client {
                    quad(out, &m, &format!("{ACP}client"), c, &g);
                }
            }
        }
    }
}

/// Matcher spec: (agent attr, client attr) — `None` = attribute absent.
type M<'a> = (Option<&'a str>, Option<&'a str>);

struct AcpPolicy<'a> {
    frag: &'a str,
    member: bool,
    allow: &'a [&'a str],
    deny: &'a [&'a str],
    all_of: &'a [M<'a>],
    any_of: &'a [M<'a>],
    none_of: &'a [M<'a>],
}

impl Default for AcpPolicy<'_> {
    fn default() -> Self {
        AcpPolicy { frag: "p", member: true, allow: &["Read"], deny: &[], all_of: &[], any_of: &[], none_of: &[] }
    }
}

/// The ACP fixture: same tree, `.acr` graphs. ACP inheritance is CUMULATIVE (every
/// ancestor's memberAccessControl applies), so the root owner policy reaches everywhere.
///
/// # Examples
///
/// ```
/// let nq = sparq_solid::acp_fixture();
/// assert!(nq.contains(".acr") && !nq.contains(".acl")); // ACP variant
/// ```
pub fn acp_fixture() -> String {
    let mut out = String::with_capacity(1 << 20);
    tree(&mut out);
    let public_agent = format!("{ACP}PublicAgent");

    // root: alice everything, on the root itself AND all members
    let alice_all = |member| AcpPolicy {
        frag: "owner",
        member,
        allow: &["Read", "Write", "Control"],
        all_of: &[(Some(ALICE), None)],
        ..AcpPolicy::default()
    };
    acr_doc(&mut out, POD, &[alice_all(false), alice_all(true)]);
    // pub1: public read for members (alice keeps access via the root — cumulative)
    acr_doc(&mut out, &format!("{POD}pub1/"), &[AcpPolicy {
        frag: "pub",
        any_of: &[(Some(&public_agent), None)],
        ..AcpPolicy::default()
    }]);
    // team2: bob + carol read/write (ACP has no groups; enumerate)
    acr_doc(&mut out, &format!("{POD}team2/"), &[AcpPolicy {
        frag: "team",
        allow: &["Read", "Write"],
        any_of: &[(Some(BOB), None), (Some(CAROL), None)],
        ..AcpPolicy::default()
    }]);
    // friends3: the native user/app PAIR — agent bob AND client app.ex (allOf)
    acr_doc(&mut out, &format!("{POD}friends3/"), &[AcpPolicy {
        frag: "pair",
        all_of: &[(Some(BOB), None), (None, Some(APP))],
        ..AcpPolicy::default()
    }]);
    // mixed4: public read, but DENY dave (deny-overrides)
    acr_doc(&mut out, &format!("{POD}mixed4/"), &[
        AcpPolicy { frag: "pub", any_of: &[(Some(&public_agent), None)], ..AcpPolicy::default() },
        AcpPolicy {
            frag: "nodave",
            allow: &[],
            deny: &["Read"],
            any_of: &[(Some(DAVE), None)],
            ..AcpPolicy::default()
        },
    ]);
    // origin5: public read EXCEPT carol (noneOf -> conditional grant)
    acr_doc(&mut out, &format!("{POD}origin5/"), &[AcpPolicy {
        frag: "nocarol",
        any_of: &[(Some(&public_agent), None)],
        none_of: &[(Some(CAROL), None)],
        ..AcpPolicy::default()
    }]);
    // a few extra ACRs (cumulative ⇒ harmless re-grants), keeping density comparable
    for c in ["priv0/c0/", "priv0/c1/", "pub1/c0/", "team2/c0/", "friends3/c0/", "origin5/c0/"] {
        acr_doc(&mut out, &format!("{POD}{c}"), &[AcpPolicy {
            frag: "regrant",
            all_of: &[(Some(ALICE), None)],
            ..AcpPolicy::default()
        }]);
    }
    out
}

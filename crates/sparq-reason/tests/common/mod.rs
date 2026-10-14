//! sq-zgbso.3 — loader-shaped WAC/ACP fixture FACTS for the compiled-rules
//! equivalence suite and the `compiled_rules_bench` measurement example.
//!
//! Emits the same fact vocabulary `sparq-solid`'s `loader::assemble_input` synthesizes
//! from its pod fixtures (`solidx:isResource` / `ownAcl` / `ownAcr` / `inDoc` /
//! `isWebId` / `creator` + the raw `.acl`/`.acr` document triples), over a containment
//! tree shaped like `sparq_solid::fixture` — WITHOUT depending on sparq-solid (that
//! would be a dev-dependency cycle; the rules files themselves are read READ-ONLY from
//! `../sparq-solid/rules/` at runtime). Fact-shape fidelity here only affects how
//! REPRESENTATIVE the measurements are; the equivalence oracle is relative (both
//! engines see the same facts).

use sparq_core::dict::{Dict, Id};
use std::fmt::Write as _;

pub const POD: &str = "https://pod.ex/";
pub const ALICE: &str = "https://alice.ex/card#me";
pub const BOB: &str = "https://bob.ex/card#me";
pub const CAROL: &str = "https://carol.ex/card#me";
pub const DAVE: &str = "https://dave.ex/card#me";
pub const APP: &str = "https://app.ex";
pub const IDP: &str = "https://idp.ex";
pub const TEAM_GROUP_DOC: &str = "https://pod.ex/groups/team";

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const SOLIDX: &str = "https://sparq.dev/ns/solidx#";
const VCARD: &str = "http://www.w3.org/2006/vcard/ns#";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";

/// Containment-tree scale: `tops` depth-1 subtrees × `mids` × `leaves` containers, each
/// leaf holding `docs` documents. `(6, 6, 6, 4)` mirrors the sparq-solid fixture tree
/// (259 containers + 864 documents); the equivalence tests use a small scale.
pub struct Scale {
    pub tops: usize,
    pub mids: usize,
    pub leaves: usize,
    pub docs: usize,
}

struct Tree {
    /// Every resource IRI (documents + all containers + the pod root).
    resources: Vec<String>,
    /// Leaf documents only.
    docs: Vec<String>,
}

fn tree(sc: &Scale) -> Tree {
    let mut resources = vec![POD.to_string()];
    let mut docs = Vec::new();
    for i in 0..sc.tops {
        let c1 = format!("{POD}s{i}/");
        resources.push(c1.clone());
        for j in 0..sc.mids {
            let c2 = format!("{c1}c{j}/");
            resources.push(c2.clone());
            for k in 0..sc.leaves {
                let c3 = format!("{c2}g{k}/");
                resources.push(c3.clone());
                for d in 0..sc.docs {
                    let doc = format!("{c3}d{d}.ttl");
                    resources.push(doc.clone());
                    docs.push(doc);
                }
            }
        }
    }
    Tree { resources, docs }
}

fn iri3(out: &mut String, s: &str, p: &str, o: &str) {
    let _ = writeln!(out, "<{s}> <{p}> <{o}> .");
}

fn tru(out: &mut String, s: &str, p: &str) {
    let _ = writeln!(out, "<{s}> <{p}> true .");
}

/// One WAC authorization inside `<{resource}.acl>`, with `solidx:inDoc` provenance.
#[allow(clippy::too_many_arguments)]
fn wac_auth(
    out: &mut String,
    resource: &str,
    frag: &str,
    access_to: bool,
    default: bool,
    agents: &[&str],
    agent_classes: &[&str],
    agent_groups: &[&str],
    origin: Option<&str>,
    modes: &[&str],
) {
    let g = format!("{resource}.acl");
    let a = format!("{g}#{frag}");
    iri3(out, &a, RDF_TYPE, &format!("{ACL}Authorization"));
    if access_to {
        iri3(out, &a, &format!("{ACL}accessTo"), resource);
    }
    if default {
        iri3(out, &a, &format!("{ACL}default"), resource);
    }
    for ag in agents {
        iri3(out, &a, &format!("{ACL}agent"), ag);
    }
    for c in agent_classes {
        iri3(out, &a, &format!("{ACL}agentClass"), c);
    }
    for grp in agent_groups {
        iri3(out, &a, &format!("{ACL}agentGroup"), grp);
    }
    if let Some(o) = origin {
        iri3(out, &a, &format!("{ACL}origin"), o);
    }
    for m in modes {
        iri3(out, &a, &format!("{ACL}mode"), &format!("{ACL}{m}"));
    }
    iri3(out, &a, &format!("{SOLIDX}inDoc"), &g);
    // The ACL'd resource declares its own ACL (loader naming convention).
    iri3(out, resource, &format!("{SOLIDX}ownAcl"), &g);
}

/// Loader-shaped WAC reasoning input over a `Scale`-sized pod: the sparq-solid fixture's
/// policy shapes (root owner; per-subtree public / group / agent / authenticated /
/// origin-pair variants; a deep override; a resource-specific ACL) as assemble_input-
/// style facts.
pub fn wac_facts(sc: &Scale) -> String {
    let t = tree(sc);
    let mut out = String::with_capacity(1 << 18);
    for r in &t.resources {
        tru(&mut out, r, &format!("{SOLIDX}isResource"));
    }
    // The group document is a pod resource too (with its structural container).
    tru(
        &mut out,
        "https://pod.ex/groups/",
        &format!("{SOLIDX}isResource"),
    );
    tru(&mut out, TEAM_GROUP_DOC, &format!("{SOLIDX}isResource"));
    let group = format!("{TEAM_GROUP_DOC}#g");
    iri3(&mut out, &group, RDF_TYPE, &format!("{VCARD}Group"));
    iri3(&mut out, &group, &format!("{VCARD}hasMember"), BOB);
    iri3(&mut out, &group, &format!("{VCARD}hasMember"), CAROL);

    let rwc: &[&str] = &["Read", "Write", "Control"];
    // Root: alice owns everything until shadowed.
    wac_auth(
        &mut out,
        POD,
        "owner",
        true,
        true,
        &[ALICE],
        &[],
        &[],
        None,
        rwc,
    );
    for i in 0..sc.tops {
        let c1 = format!("{POD}s{i}/");
        match i % 5 {
            0 => {
                // public read + owner re-grant
                wac_auth(
                    &mut out,
                    &c1,
                    "pub",
                    true,
                    true,
                    &[],
                    &[FOAF_AGENT],
                    &[],
                    None,
                    &["Read"],
                );
                wac_auth(
                    &mut out,
                    &c1,
                    "owner",
                    true,
                    true,
                    &[ALICE],
                    &[],
                    &[],
                    None,
                    rwc,
                );
            }
            1 => {
                // group read+write, NO owner re-grant (nearest-ACL shadows the root)
                wac_auth(
                    &mut out,
                    &c1,
                    "team",
                    true,
                    true,
                    &[],
                    &[],
                    &[&group],
                    None,
                    &["Read", "Write"],
                );
            }
            2 => {
                // bob reads members (default only); alice owns the container (accessTo only)
                wac_auth(
                    &mut out,
                    &c1,
                    "bob",
                    false,
                    true,
                    &[BOB],
                    &[],
                    &[],
                    None,
                    &["Read"],
                );
                wac_auth(
                    &mut out,
                    &c1,
                    "owner",
                    true,
                    false,
                    &[ALICE],
                    &[],
                    &[],
                    None,
                    rwc,
                );
            }
            3 => {
                // any authenticated agent reads; alice owns
                wac_auth(
                    &mut out,
                    &c1,
                    "authd",
                    true,
                    true,
                    &[],
                    &[&format!("{ACL}AuthenticatedAgent")],
                    &[],
                    None,
                    &["Read"],
                );
                wac_auth(
                    &mut out,
                    &c1,
                    "owner",
                    true,
                    true,
                    &[ALICE],
                    &[],
                    &[],
                    None,
                    rwc,
                );
            }
            _ => {
                // bob ONLY through client/origin APP (pair-principal mint); alice owns
                wac_auth(
                    &mut out,
                    &c1,
                    "app",
                    true,
                    true,
                    &[BOB],
                    &[],
                    &[],
                    Some(APP),
                    &["Read"],
                );
                wac_auth(
                    &mut out,
                    &c1,
                    "owner",
                    true,
                    true,
                    &[ALICE],
                    &[],
                    &[],
                    None,
                    rwc,
                );
            }
        }
        // Deep override: subtree 3's first mid-container narrows to carol only.
        if i % 5 == 3 && sc.mids > 0 {
            wac_auth(
                &mut out,
                &format!("{c1}c0/"),
                "deep",
                true,
                true,
                &[CAROL],
                &[],
                &[],
                None,
                &["Read"],
            );
        }
        // ~restating container ACLs (nearest-ACL coverage at depth 2).
        if sc.mids > 1 {
            wac_auth(
                &mut out,
                &format!("{c1}c1/"),
                "owner",
                true,
                true,
                &[ALICE],
                &[],
                &[],
                None,
                rwc,
            );
        }
    }
    // One resource-specific ACL: the first document, bob read (accessTo only).
    if let Some(doc) = t.docs.first() {
        wac_auth(
            &mut out,
            doc,
            "doc",
            true,
            false,
            &[BOB],
            &[],
            &[],
            None,
            &["Read"],
        );
    }
    for w in [ALICE, BOB, CAROL] {
        tru(&mut out, w, &format!("{SOLIDX}isWebId"));
    }
    out
}

/// One ACP policy inside `<{resource}.acr>`: matcher specs are `(agent?, client?,
/// issuer?)` triples (`None` = attribute absent).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn acp_policy(
    out: &mut String,
    resource: &str,
    frag: &str,
    member: bool,
    allow: &[&str],
    deny: &[&str],
    all_of: &[(Option<&str>, Option<&str>, Option<&str>)],
    any_of: &[(Option<&str>, Option<&str>, Option<&str>)],
    none_of: &[(Option<&str>, Option<&str>, Option<&str>)],
) {
    let g = format!("{resource}.acr");
    iri3(out, resource, &format!("{SOLIDX}ownAcr"), &g);
    let control = format!("{g}#ctl-{frag}");
    let pred = if member {
        "memberAccessControl"
    } else {
        "accessControl"
    };
    iri3(out, &g, &format!("{ACP}{pred}"), &control);
    iri3(out, &g, &format!("{SOLIDX}inDoc"), &g);
    iri3(out, &control, &format!("{SOLIDX}inDoc"), &g);
    let pol = format!("{g}#pol-{frag}");
    iri3(out, &control, &format!("{ACP}apply"), &pol);
    iri3(out, &pol, &format!("{SOLIDX}inDoc"), &g);
    for m in allow {
        iri3(out, &pol, &format!("{ACP}allow"), &format!("{ACL}{m}"));
    }
    for m in deny {
        iri3(out, &pol, &format!("{ACP}deny"), &format!("{ACL}{m}"));
    }
    for (comb, matchers) in [("allOf", all_of), ("anyOf", any_of), ("noneOf", none_of)] {
        for (i, (agent, client, issuer)) in matchers.iter().enumerate() {
            let m = format!("{g}#m-{frag}-{comb}{i}");
            iri3(out, &pol, &format!("{ACP}{comb}"), &m);
            if let Some(a) = agent {
                iri3(out, &m, &format!("{ACP}agent"), a);
            }
            if let Some(c) = client {
                iri3(out, &m, &format!("{ACP}client"), c);
            }
            if let Some(ii) = issuer {
                iri3(out, &m, &format!("{ACP}issuer"), ii);
            }
            iri3(out, &m, &format!("{SOLIDX}inDoc"), &g);
        }
    }
}

/// Loader-shaped ACP reasoning input over a `Scale`-sized pod: cumulative-inheritance
/// member policies, allOf/anyOf/noneOf matchers, a deny policy, an issuer-constrained
/// pair, and a CreatorAgent provenance policy with trusted `solidx:creator` facts.
pub fn acp_facts(sc: &Scale) -> String {
    let t = tree(sc);
    let mut out = String::with_capacity(1 << 18);
    for r in &t.resources {
        tru(&mut out, r, &format!("{SOLIDX}isResource"));
    }
    let public = format!("{ACP}PublicAgent");
    // Root: alice everything, on the root itself AND all members.
    acp_policy(
        &mut out,
        POD,
        "owner",
        false,
        &["Read", "Write", "Control"],
        &[],
        &[(Some(ALICE), None, None)],
        &[],
        &[],
    );
    acp_policy(
        &mut out,
        POD,
        "owner-m",
        true,
        &["Read", "Write", "Control"],
        &[],
        &[(Some(ALICE), None, None)],
        &[],
        &[],
    );
    for i in 0..sc.tops {
        let c1 = format!("{POD}s{i}/");
        match i % 5 {
            0 => {
                // public read for members
                acp_policy(
                    &mut out,
                    &c1,
                    "pub",
                    true,
                    &["Read"],
                    &[],
                    &[],
                    &[(Some(&public), None, None)],
                    &[],
                );
            }
            1 => {
                // bob + carol read/write; PLUS an issuer-constrained pair on c0
                acp_policy(
                    &mut out,
                    &c1,
                    "team",
                    true,
                    &["Read", "Write"],
                    &[],
                    &[],
                    &[(Some(BOB), None, None), (Some(CAROL), None, None)],
                    &[],
                );
                if sc.mids > 0 {
                    acp_policy(
                        &mut out,
                        &format!("{c1}c0/"),
                        "idp",
                        true,
                        &["Read"],
                        &[],
                        &[(Some(BOB), None, Some(IDP))],
                        &[],
                        &[],
                    );
                }
            }
            2 => {
                // the user/app PAIR (agent bob AND client app); PLUS CreatorAgent on c0
                acp_policy(
                    &mut out,
                    &c1,
                    "pair",
                    true,
                    &["Read"],
                    &[],
                    &[(Some(BOB), None, None), (None, Some(APP), None)],
                    &[],
                    &[],
                );
                if sc.mids > 0 {
                    let c0 = format!("{c1}c0/");
                    acp_policy(
                        &mut out,
                        &c0,
                        "creator",
                        true,
                        &["Write"],
                        &[],
                        &[(Some(&format!("{ACP}CreatorAgent")), None, None)],
                        &[],
                        &[],
                    );
                    // Trusted provenance facts for the documents under c0/g0/.
                    if sc.leaves > 0 {
                        for d in 0..sc.docs {
                            iri3(
                                &mut out,
                                &format!("{c0}g0/d{d}.ttl"),
                                &format!("{SOLIDX}creator"),
                                CAROL,
                            );
                        }
                    }
                }
            }
            3 => {
                // public read but DENY dave (deny-overrides at the session layer)
                acp_policy(
                    &mut out,
                    &c1,
                    "pub",
                    true,
                    &["Read"],
                    &[],
                    &[],
                    &[(Some(&public), None, None)],
                    &[],
                );
                acp_policy(
                    &mut out,
                    &c1,
                    "nodave",
                    true,
                    &[],
                    &["Read"],
                    &[],
                    &[(Some(DAVE), None, None)],
                    &[],
                );
            }
            _ => {
                // public read EXCEPT carol (noneOf -> conditional grant)
                acp_policy(
                    &mut out,
                    &c1,
                    "nocarol",
                    true,
                    &["Read"],
                    &[],
                    &[],
                    &[(Some(&public), None, None)],
                    &[(Some(CAROL), None, None)],
                );
            }
        }
    }
    for w in [ALICE, BOB, CAROL, DAVE] {
        tru(&mut out, w, &format!("{SOLIDX}isWebId"));
    }
    out
}

/// Read one sparq-solid rules file (READ-ONLY — the bead's disjointness contract).
pub fn solid_rules(name: &str) -> String {
    let path = format!("{}/../sparq-solid/rules/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Re-serialize an id-level closure as N3 fact text — the exact shape sparq-solid's
/// `materialize.rs::closure_to_n3` feeds the next ACP stratum (and, transitively, the
/// shape `assemble_input` produces: one N-Triples-form statement per fact).
pub fn closure_to_n3(dict: &Dict, closure: &[[Id; 3]]) -> String {
    let mut out = String::with_capacity(closure.len() * 64);
    for t in closure {
        let _ = writeln!(
            out,
            "{} {} {} .",
            dict.term(t[0]),
            dict.term(t[1]),
            dict.term(t[2])
        );
    }
    out
}

/// A closure as dictionary-independent N-Triples-formatted strings, for set comparison
/// across engines with different dictionaries.
pub fn triples_as_strings(dict: &Dict, ids: &[[Id; 3]]) -> std::collections::BTreeSet<String> {
    ids.iter()
        .map(|t| {
            format!(
                "{} {} {}",
                dict.term(t[0]),
                dict.term(t[1]),
                dict.term(t[2])
            )
        })
        .collect()
}

/// Assert two closures are set-equal, failing LOUDLY with the full symmetric difference.
pub fn assert_set_equal(
    text: &std::collections::BTreeSet<String>,
    compiled: &std::collections::BTreeSet<String>,
    what: &str,
) {
    let missing: Vec<_> = text.difference(compiled).collect();
    let extra: Vec<_> = compiled.difference(text).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: compiled closure diverges from reason_n3\n  text-only ({} of {}): {:#?}\n  compiled-only ({} of {}): {:#?}",
        missing.len(),
        text.len(),
        &missing[..missing.len().min(25)],
        extra.len(),
        compiled.len(),
        &extra[..extra.len().min(25)]
    );
}

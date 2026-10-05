//! Reasoning-input assembly: reflect the named-graph structure into facts and hand the
//! ACCESS-CONTROL graphs (only!) to the reasoner.
//!
//! [SONNET-4.6] sq-zgbso.4: [`assemble_facts`] is the SINGLE traversal both fact-entry
//! paths share. It yields ground `[Term; 3]` triples (blank nodes already skolemized);
//! [`assemble_input_ids`] interns those straight into the caller's [`Dict`] for the
//! id-level compiled evaluator — no N3 text is produced or re-parsed — and the
//! `#[cfg(test)]`-only `assemble_input` serializes the SAME triples to N3 source for
//! the differential reference path. One traversal means the two entries cannot diverge.
//!
//! Security boundary (design doc §2.4): pod *content* graphs are never fed to the
//! reasoner — otherwise any agent who can write a document could embed `acl:`/`acp:`
//! triples granting themselves access. The inputs are: `.acl`/`.acr` graphs, group
//! documents referenced via `acl:agentGroup` (fragment stripped), and the synthesized
//! structural facts below.
//!
//! [OPUS-4.8] sq-3jtd.5: the `.acl`/`.acr`/group graphs ARE emitted verbatim, so they are a
//! second smuggling surface. The reasoner's derivation-internal vocabulary — the `solidx:`
//! namespace (`solidx:creator|owner|appliesToResource|isResource|isWebId|…`) — must only be
//! produced by THIS loader (from trusted structural metadata + the caller-supplied
//! `AccessProvenance`) or derived by the rules; a forged `solidx:` fact inside a control
//! document would otherwise grant access (cross-resource privilege escalation / policy
//! redirection). So [`is_reserved_derivation_predicate`] hard-rejects any control-graph or
//! group-document triple whose predicate is in `solidx:` space — the analogue of the
//! `urn:sparq:` reserved-principal guard ([`validate_principal_iri`]).

use crate::{AccessProvenance, VerifiedCredentials, SOLIDX_NS};
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;

pub(crate) const ACL_SUFFIX: &str = ".acl";
pub(crate) const ACR_SUFFIX: &str = ".acr";

const ACL_AGENT: &str = "http://www.w3.org/ns/auth/acl#agent";
const ACL_AGENT_GROUP: &str = "http://www.w3.org/ns/auth/acl#agentGroup";
const ACL_ORIGIN: &str = "http://www.w3.org/ns/auth/acl#origin";
const ACP_AGENT: &str = "http://www.w3.org/ns/solid/acp#agent";
const ACP_CLIENT: &str = "http://www.w3.org/ns/solid/acp#client";
// [OPUS-4.8] sq-3jtd.6: acp:issuer is a pair/triple-principal ingredient too, so its
// values go through the same reserved-encoding validation as agents/clients/origins.
const ACP_ISSUER: &str = "http://www.w3.org/ns/solid/acp#issuer";
const VCARD_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";

/// Reserved IRI space: the auth view, the rewrite sentinel, minted pair/candidate/grant
/// principals. Graphs named under it are stripped at PodStore/materializer boundaries,
/// and agent/client/origin values inside it (or containing the pair-IRI delimiter) are
/// REJECTED (roborev 1723). Pair/candidate minting now percent-encodes its components
/// (`string:encodeForUri` / [`sparq_reason::n3::encode_for_uri`]), so a crafted WebID
/// like `…&client=…` can no longer collide with a minted pair at the encoding level —
/// this validation stays as defense in depth, and it remains LOAD-BEARING for raw
/// reserved-space values: a session or ACL agent equal to a full minted IRI
/// (`urn:sparq:pair?…`) would otherwise match that principal's grants directly.
pub(crate) const RESERVED_PREFIX: &str = "urn:sparq:";
const PAIR_DELIMITER: &str = "&client=";

fn validate_principal_iri(iri: &str) -> Result<(), String> {
    if iri.starts_with(RESERVED_PREFIX) || iri.contains(PAIR_DELIMITER) {
        return Err(format!(
            "agent/client/origin IRI <{iri}> is not allowed: \
             `{RESERVED_PREFIX}` and the literal `{PAIR_DELIMITER}` are reserved by the \
             pair-principal encoding"
        ));
    }
    Ok(())
}

/// [OPUS-4.8] sq-3jtd.5: Predicates in the `solidx:` namespace are the reasoner's
/// DERIVATION-INTERNAL vocabulary (`solidx:creator`, `solidx:owner`,
/// `solidx:appliesToResource`, `solidx:isResource`, `solidx:inDoc`, `solidx:isWebId`,
/// `solidx:provForResource`, …). Those facts are synthesized by THIS loader from
/// trusted inputs (structural metadata + the caller-supplied [`AccessProvenance`]) or
/// derived by the N3 rules — they must NEVER originate from access-control-document
/// content. A writer who can place a triple inside an `.acr`/`.acl` they control could
/// otherwise smuggle a forged `<r> solidx:creator <self>` (cross-resource privilege
/// escalation) or `<pol> solidx:appliesToResource <secret>` (policy redirection) that
/// the rules cannot distinguish from a loader-synthesized trusted fact.
///
/// So any control-graph (or group-document) triple whose PREDICATE is in `solidx:`
/// space is DROPPED before it reaches the reasoner — the direct analogue of the
/// `urn:sparq:` reserved-principal guard in [`validate_principal_iri`]. The trusted
/// channel for creator/owner facts is [`AccessProvenance`] and nothing else.
fn is_reserved_derivation_predicate(t: &[Term; 3]) -> bool {
    matches!(&t[1], Term::NamedNode(n) if n.as_str().starts_with(SOLIDX_NS))
}

/// Drop ALL named graphs in the reserved IRI space — including a pre-existing
/// `<urn:sparq:auth>`: a loaded dataset must not be able to smuggle in the rewrite
/// sentinel (`urn:sparq:nothing`) or a FORGED auth view; only `install_auth_view`
/// creates `<urn:sparq:auth>`, after a successful materialization (roborev 1727).
pub(crate) fn strip_reserved_graphs(graph: &mut Graph) {
    graph.named.retain(|(name, _)| match name {
        Term::NamedNode(n) => !n.as_str().starts_with(RESERVED_PREFIX),
        _ => true,
    });
}

/// Whether a session-supplied agent/client value may participate in principal
/// expansion: anything in the reserved space or containing the pair delimiter could
/// IMPERSONATE a minted pair principal — fail closed.
pub(crate) fn session_value_allowed(v: &str) -> bool {
    !v.starts_with(RESERVED_PREFIX) && !v.contains(PAIR_DELIMITER)
}
/// `acp:agent` objects that are NOT concrete WebIDs.
const SPECIAL_AGENTS: [&str; 5] = [
    "http://www.w3.org/ns/solid/acp#PublicAgent",
    "http://www.w3.org/ns/solid/acp#AuthenticatedAgent",
    "http://www.w3.org/ns/solid/acp#CreatorAgent",
    "http://www.w3.org/ns/solid/acp#OwnerAgent",
    "http://www.w3.org/ns/auth/acl#AuthenticatedAgent",
];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum System {
    Wac,
    Acp,
}

/// The triples of one named graph as ground term triples.
pub(crate) fn graph_triples(g: &Graph) -> Vec<[Term; 3]> {
    let pat: sparq_core::store::Pattern = [None, None, None];
    let scan = g.store.scan(&pat);
    scan.rows
        .iter()
        .map(|r| {
            let t = scan.to_spo(r);
            [g.dict.term(t[0]), g.dict.term(t[1]), g.dict.term(t[2])]
        })
        .collect()
}

fn graph_iri(name: &Term) -> Option<&str> {
    match name {
        Term::NamedNode(n) => Some(n.as_str()),
        _ => None,
    }
}

/// A term with blank nodes skolemized per graph (`gix`) so the single-graph merge keeps
/// per-document scoping and `solidx:inDoc` stays sound. Named nodes and literals pass
/// through unchanged — the reasoner sees exactly the control document's own terms.
fn skolemize(t: &Term, gix: usize) -> Term {
    match t {
        Term::BlankNode(b) => named(&format!("urn:skolem:g{}:{}", gix, b.as_str())),
        other => other.clone(),
    }
}

/// A term for the IRI `iri` (the loader only ever mints IRIs it built itself).
fn named(iri: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(iri))
}

/// The `xsd:boolean` `true` object of the structural marker facts (`solidx:isResource`,
/// `solidx:isWebId`) — the term the N3 keyword `true` parses to, so the id-level and the
/// text entry paths agree on it.
fn xsd_true() -> Term {
    Term::Literal(Literal::new_typed_literal(
        "true",
        NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#boolean"),
    ))
}

/// Assemble the full fact set: structural facts + the access-control graphs +
/// (ACP only) the TRUSTED per-resource creator/owner facts from `provenance` and the
/// TRUSTED verified-credential holdings from `credentials`.
/// Errors if any agent/client/origin/creator/owner/credential-holder value collides with
/// the reserved principal encoding (see [`validate_principal_iri`]).
///
/// [OPUS-4.8] sq-3jtd.5: `provenance` is the trusted channel for `acp:CreatorAgent` /
/// `acp:OwnerAgent`. Its `<r> solidx:creator|owner <webid>` facts are synthesized HERE,
/// from the caller-supplied map ONLY — never read from the resource graphs (design doc
/// §2.4). For WAC (no creator/owner vocabulary) `provenance` is ignored.
///
/// [SONNET-4.6] sq-ysv3u: `credentials` is the exactly analogous trusted channel for ACP
/// `acp:vc`. Its `<webid> solidx:holdsVc <requirement>` facts are synthesized HERE and
/// nowhere else. WAC has no credential vocabulary, so `credentials` is ignored for it.
pub(crate) fn assemble_facts(
    graph: &Graph,
    system: System,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
) -> Result<Vec<[Term; 3]>, String> {
    let mut out: Vec<[Term; 3]> = Vec::new();
    let suffix = if system == System::Wac { ACL_SUFFIX } else { ACR_SUFFIX };
    let own_pred = if system == System::Wac { "ownAcl" } else { "ownAcr" };
    let is_resource = named(&format!("{SOLIDX_NS}isResource"));
    let owns = named(&format!("{SOLIDX_NS}{own_pred}"));
    let in_doc_p = named(&format!("{SOLIDX_NS}inDoc"));
    let is_webid = named(&format!("{SOLIDX_NS}isWebId"));

    // 1) resources: every non-control, non-auth graph + every structural container
    //    prefix (containers exist as inheritance anchors even without their own graph).
    let mut resources: FxHashSet<String> = FxHashSet::default();
    let mut control_graphs: Vec<(usize, &str, &Graph)> = Vec::new(); // (idx, name, sub)
    for (gix, (name, sub)) in graph.named.iter().enumerate() {
        let Some(iri) = graph_iri(name) else { continue };
        // Skip the whole reserved space (`<urn:sparq:auth>` included): a graph there is
        // never reasoning input. [`strip_reserved_graphs`] also drops these from the
        // dataset, but assembly must not DEPEND on that having run first — the
        // materializer defers every mutation until after this fallible pass so an error
        // leaves the previous auth view in place (roborev 5005).
        if iri.starts_with(RESERVED_PREFIX) {
            continue;
        }
        if iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX) {
            if iri.ends_with(suffix) {
                control_graphs.push((gix, iri, sub));
            }
            continue;
        }
        resources.insert(iri.to_owned());
        // structural container chain: https://host/a/b/doc -> /a/b/ -> /a/ -> /
        let mut cur = iri;
        while let Some(parent) = parent_iri(cur) {
            if !resources.insert(parent.to_owned()) {
                break;
            }
            cur = parent;
        }
    }
    for r in &resources {
        out.push([named(r), is_resource.clone(), xsd_true()]);
    }

    // 2) control-document linkage by naming convention: <R + ".acl"> controls <R>.
    //    The .acl/.acr graphs are themselves resources too (Control gates them).
    for (_, iri, _) in &control_graphs {
        let r = &iri[..iri.len() - suffix.len()];
        out.push([named(r), owns.clone(), named(iri)]);
    }

    // 3) the access-control graphs' triples (skolemized) + inDoc provenance + WebIDs,
    //    and (WAC) group documents referenced via acl:agentGroup.
    let mut group_docs: FxHashSet<String> = FxHashSet::default();
    let mut webids: FxHashSet<String> = FxHashSet::default();
    let mut principal_iris: FxHashSet<String> = FxHashSet::default();
    for (gix, iri, sub) in &control_graphs {
        let mut in_doc: FxHashSet<Term> = FxHashSet::default();
        for t in graph_triples(sub) {
            // [OPUS-4.8] sq-3jtd.5: hard-reject any forged derivation-internal fact
            // (`solidx:creator|owner|appliesToResource|…`) smuggled into the control
            // document — only the loader/rules may produce `solidx:` facts.
            if is_reserved_derivation_predicate(&t) {
                continue;
            }
            let s = skolemize(&t[0], *gix);
            in_doc.insert(s.clone());
            out.push([s, skolemize(&t[1], *gix), skolemize(&t[2], *gix)]);
            collect_agents(&t, &mut webids, &mut group_docs, &mut principal_iris);
        }
        for s in in_doc {
            out.push([s, in_doc_p.clone(), named(iri)]);
        }
    }
    if system == System::Wac {
        for (gix, (name, sub)) in graph.named.iter().enumerate() {
            let Some(iri) = graph_iri(name) else { continue };
            // Same reserved-space skip as the resource pass above: a forged
            // `<urn:sparq:…>` graph named by an `acl:agentGroup` must never be read.
            if iri.starts_with(RESERVED_PREFIX) || !group_docs.contains(iri) {
                continue;
            }
            for t in graph_triples(sub) {
                // [OPUS-4.8] sq-3jtd.5: same derivation-internal guard for group
                // documents — a forged `solidx:` fact here would feed the reasoner too.
                if is_reserved_derivation_predicate(&t) {
                    continue;
                }
                out.push([
                    skolemize(&t[0], gix),
                    skolemize(&t[1], gix),
                    skolemize(&t[2], gix),
                ]);
                if let (Term::NamedNode(p), Term::NamedNode(o)) = (&t[1], &t[2]) {
                    if p.as_str() == VCARD_MEMBER {
                        webids.insert(o.as_str().to_owned());
                        principal_iris.insert(o.as_str().to_owned());
                    }
                }
            }
        }
    }
    for iri in &principal_iris {
        validate_principal_iri(iri)?;
    }
    // [OPUS-4.8] sq-3jtd.5: TRUSTED creator/owner facts (ACP only). Emitted ONLY from the
    // caller-supplied provenance map — never from pod content (design doc §2.4). The
    // WebIDs go through the SAME reserved-encoding validation as agents/clients/issuers
    // (they become candidate agents in the rules) and are marked isWebId so the
    // candidate-generation lattice picks them up.
    if system == System::Acp {
        for (resource, creator, owner) in provenance.iter() {
            if let Some(c) = creator {
                validate_principal_iri(c)?;
                webids.insert(c.to_owned());
                out.push([
                    named(resource),
                    named(&format!("{SOLIDX_NS}creator")),
                    named(c),
                ]);
            }
            if let Some(o) = owner {
                validate_principal_iri(o)?;
                webids.insert(o.to_owned());
                out.push([named(resource), named(&format!("{SOLIDX_NS}owner")), named(o)]);
            }
        }
        // [SONNET-4.6] sq-ysv3u: TRUSTED `acp:vc` holdings (ACP only), on exactly the same
        // footing as the creator/owner facts above — emitted ONLY from the caller-supplied
        // map, never from pod or `.acr` content (design doc §2.4; the solidx: guard at the
        // top of this pass drops any forged `holdsVc` smuggled into a control document).
        // A holder becomes a candidate agent, so its WebID goes through the SAME
        // reserved-encoding validation and is marked isWebId.
        for (agent, requirement) in credentials.iter() {
            validate_principal_iri(agent)?;
            webids.insert(agent.to_owned());
            out.push([named(agent), named(&format!("{SOLIDX_NS}holdsVc")), named(requirement)]);
        }
    }
    for a in &webids {
        out.push([named(a), is_webid.clone(), xsd_true()]);
    }
    Ok(out)
}

/// The reasoning input as `[Id; 3]` facts interned DIRECTLY into `dict` — the id-level
/// fact entry of the compiled evaluator ([SONNET-4.6] sq-zgbso.4).
///
/// Same fact set as the `assemble_input` text entry by construction (both are thin
/// adapters over [`assemble_facts`]), with the serialize → re-parse round trip gone: terms
/// go from the source [`Graph`] to the caller's dictionary without ever becoming text.
pub(crate) fn assemble_input_ids(
    dict: &mut Dict,
    graph: &Graph,
    system: System,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
) -> Result<Vec<[Id; 3]>, String> {
    let facts = assemble_facts(graph, system, provenance, credentials)?;
    Ok(facts
        .iter()
        .map(|t| [dict.intern(&t[0]), dict.intern(&t[1]), dict.intern(&t[2])])
        .collect())
}

/// The reasoning input as N3 source — the DEV-ONLY differential reference path
/// ([SONNET-4.6] sq-zgbso.4). The production materializer feeds
/// [`assemble_input_ids`] to the compiled evaluator; this serialization exists so the
/// in-crate equivalence tests can run the same facts through the text engine
/// (`sparq_reason::reason_n3`) and compare the resulting auth views.
///
/// Every term is written in its N-Triples shape (`<iri>` / `"…"`/`"…"^^<dt>`/`"…"@lang`),
/// all of which the N3 parser accepts.
#[cfg(test)]
pub(crate) fn assemble_input(
    graph: &Graph,
    system: System,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
) -> Result<String, String> {
    use std::fmt::Write;
    let facts = assemble_facts(graph, system, provenance, credentials)?;
    let mut out = String::new();
    for t in &facts {
        let _ = writeln!(out, "{} {} {} .", t[0], t[1], t[2]);
    }
    Ok(out)
}

/// The group DOCUMENT an `acl:agentGroup` value names: the group IRI without its
/// fragment (`https://pod.ex/groups#team` → `https://pod.ex/groups`). Single source of
/// truth for the mapping — [`collect_agents`] uses it to decide which named graphs the
/// materializer merges in, and [`referenced_group_docs`] uses it to tell the write path
/// which graphs are auth-view inputs.
fn group_doc_of(group_iri: &str) -> &str {
    group_iri.split('#').next().unwrap_or(group_iri)
}

/// Every graph IRI referenced as an `acl:agentGroup` group document by an access-control
/// document currently in `graph` — the WAC group-membership INPUTS of the auth view.
///
/// The write path ([`crate::update`]) needs this because a group document, unlike an
/// `.acl`/`.acr`, has no naming convention: nothing about the IRI `https://pod.ex/groups`
/// says a write to it changes who may read what. Without the referenced set, a write that
/// adds or removes a `vcard:hasMember` triple leaves the materialized auth view stale
/// (design record research/solid-pod-scoped-materializer-design.md §2.2 case A).
///
/// Deliberately an OVER-approximation, never an under-one:
/// - it is the set of REFERENCED documents, not of present graphs, so a write that
///   CREATES a not-yet-existing group document still triggers re-materialization;
/// - it scans `.acl` **and** `.acr` control documents. `acl:agentGroup` is WAC-only
///   vocabulary (the ACP materializer ignores it), so an occurrence inside an `.acr` can
///   at worst cost one redundant re-materialization — it can never miss one.
///
/// Reserved-space (`urn:sparq:`) values are excluded on both sides, mirroring the
/// assembly pass: the materializer never reads a reserved-space graph as a group
/// document, so a write to one cannot change the auth view either.
pub(crate) fn referenced_group_docs(graph: &Graph) -> FxHashSet<String> {
    let mut docs: FxHashSet<String> = FxHashSet::default();
    for (name, sub) in graph.named.iter() {
        let Some(iri) = graph_iri(name) else { continue };
        if iri.starts_with(RESERVED_PREFIX)
            || !(iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX))
        {
            continue;
        }
        for t in graph_triples(sub) {
            let (Term::NamedNode(p), Term::NamedNode(o)) = (&t[1], &t[2]) else { continue };
            if p.as_str() != ACL_AGENT_GROUP {
                continue;
            }
            let doc = group_doc_of(o.as_str());
            if !doc.starts_with(RESERVED_PREFIX) {
                docs.insert(doc.to_owned());
            }
        }
    }
    docs
}

/// Concrete agents + group documents mentioned by an access-control triple; every
/// pair/triple-principal ingredient (agents, group members, origins, clients, issuers)
/// is recorded for reserved-encoding validation.
fn collect_agents(
    t: &[Term; 3],
    webids: &mut FxHashSet<String>,
    groups: &mut FxHashSet<String>,
    principal_iris: &mut FxHashSet<String>,
) {
    let (Term::NamedNode(p), Term::NamedNode(o)) = (&t[1], &t[2]) else { return };
    match p.as_str() {
        ACL_AGENT | ACP_AGENT | VCARD_MEMBER => {
            if !SPECIAL_AGENTS.contains(&o.as_str()) {
                webids.insert(o.as_str().to_owned());
            }
            principal_iris.insert(o.as_str().to_owned());
        }
        ACL_ORIGIN | ACP_CLIENT | ACP_ISSUER => {
            principal_iris.insert(o.as_str().to_owned());
        }
        ACL_AGENT_GROUP => {
            groups.insert(group_doc_of(o.as_str()).to_owned());
        }
        _ => {}
    }
}

/// The origin (`scheme://authority`) of an IRI — the coarse pod boundary used to bucket
/// the auth index + session cache for an ACL-write invalidation ([OPUS-4.8] sq-b7k7u).
/// `https://pod.ex/a/b` → `https://pod.ex`; an authority with no path (`https://pod.ex`)
/// is its own origin. An IRI with no `://` (e.g. `urn:…`) returns the whole IRI — a
/// self-contained fallback bucket only ever invalidated by a FULL re-materialization.
///
/// Soundness of scoping on this key: `reindex_with`'s **diff-based** invalidation
/// ([SONNET-4.6] sq-b7k7u fix) diffs old vs new `AuthIndex` per-origin and invalidates
/// exactly the origins whose buckets changed — so a cross-origin dependency (WAC
/// agentGroup membership, foreign-subject grant, ACP cross-document indirection) is
/// caught automatically, without relying on any confinement argument about where grants
/// can originate.
pub(crate) fn iri_origin(iri: &str) -> &str {
    match iri.find("://") {
        Some(i) => {
            let after = i + 3;
            match iri[after..].find('/') {
                Some(j) => &iri[..after + j],
                None => iri,
            }
        }
        None => iri,
    }
}

/// Solid slash-semantics parent of an IRI (None at/above the authority root).
pub(crate) fn parent_iri(iri: &str) -> Option<&str> {
    let scheme_end = iri.find("://").map(|i| i + 3)?;
    let path = &iri[scheme_end..];
    let host_end = scheme_end + path.find('/')?;
    let trimmed = iri.strip_suffix('/').unwrap_or(iri);
    if trimmed.len() <= host_end {
        return None; // already the root container
    }
    let cut = trimmed.rfind('/')?;
    if cut < host_end {
        return None;
    }
    Some(&iri[..cut + 1])
}

#[cfg(test)]
mod tests {
    use super::{iri_origin, parent_iri};

    #[test]
    fn parent_walks_to_root_and_stops() {
        assert_eq!(parent_iri("https://pod.ex/a/b/doc.ttl"), Some("https://pod.ex/a/b/"));
        assert_eq!(parent_iri("https://pod.ex/a/b/"), Some("https://pod.ex/a/"));
        assert_eq!(parent_iri("https://pod.ex/a/"), Some("https://pod.ex/"));
        assert_eq!(parent_iri("https://pod.ex/"), None);
    }

    #[test]
    fn iri_origin_is_scheme_authority() {
        // [OPUS-4.8] sq-b7k7u: origin = scheme://authority, stable across the whole subtree.
        assert_eq!(iri_origin("https://pod.ex/a/b/doc.ttl"), "https://pod.ex");
        assert_eq!(iri_origin("https://pod.ex/.acl"), "https://pod.ex");
        assert_eq!(iri_origin("https://pod.ex/"), "https://pod.ex");
        assert_eq!(iri_origin("https://pod.ex"), "https://pod.ex"); // authority, no path
        assert_eq!(iri_origin("http://host:8080/x"), "http://host:8080"); // port kept
        // Every graph a `.acl` at origin O governs shares O — the scoping soundness key.
        let acl = "https://pod.ex/notes/.acl";
        assert_eq!(iri_origin(acl), iri_origin("https://pod.ex/notes/n1"));
        assert_ne!(iri_origin(acl), iri_origin("https://other.ex/notes/n1"));
        // No scheme → the whole IRI is its own fallback bucket (no slash `.acl` governs it).
        assert_eq!(iri_origin("urn:sparq:auth"), "urn:sparq:auth");
    }
}

//! Reasoning-input assembly: reflect the named-graph structure into facts and hand the
//! ACCESS-CONTROL graphs (only!) to the reasoner.
//!
//! sq-zgbso.4: [`assemble_facts`] is the SINGLE traversal both fact-entry
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
//! sq-3jtd.5: the `.acl`/`.acr`/group graphs ARE emitted verbatim, so they are a
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
use sparq_reason::n3::encode_for_uri;

pub(crate) const ACL_SUFFIX: &str = ".acl";
pub(crate) const ACR_SUFFIX: &str = ".acr";

const ACL_AGENT: &str = "http://www.w3.org/ns/auth/acl#agent";
const ACL_AGENT_GROUP: &str = "http://www.w3.org/ns/auth/acl#agentGroup";
const ACL_ORIGIN: &str = "http://www.w3.org/ns/auth/acl#origin";
const ACP_AGENT: &str = "http://www.w3.org/ns/solid/acp#agent";
const ACP_CLIENT: &str = "http://www.w3.org/ns/solid/acp#client";
// sq-3jtd.6: acp:issuer is a pair/triple-principal ingredient too, so its
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

pub(crate) fn validate_principal_iri(iri: &str) -> Result<(), String> {
    if iri.starts_with(RESERVED_PREFIX) || iri.contains(PAIR_DELIMITER) {
        return Err(format!(
            "agent/client/origin IRI <{iri}> is not allowed: \
             `{RESERVED_PREFIX}` and the literal `{PAIR_DELIMITER}` are reserved by the \
             pair-principal encoding"
        ));
    }
    Ok(())
}

/// sq-3jtd.5: Predicates in the `solidx:` namespace are the reasoner's
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

/// Skolemize blank nodes using the graph IRI, independent of named-graph ordering.
/// Both components are percent-encoded so the separator cannot occur inside either
/// component: distinct (graph IRI, label) pairs produce distinct, valid IRIs.
/// Named nodes and literals pass through unchanged.
fn skolemize(t: &Term, graph_iri: &str) -> Term {
    match t {
        Term::BlankNode(b) => named(&format!(
            "urn:skolem:graph:{}:{}",
            encode_for_uri(graph_iri),
            encode_for_uri(b.as_str())
        )),
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
/// sq-3jtd.5: `provenance` is the trusted channel for `acp:CreatorAgent` /
/// `acp:OwnerAgent`. Its `<r> solidx:creator|owner <webid>` facts are synthesized HERE,
/// from the caller-supplied map ONLY — never read from the resource graphs (design doc
/// §2.4). For WAC (no creator/owner vocabulary) `provenance` is ignored.
///
/// sq-ysv3u: `credentials` is the exactly analogous trusted channel for ACP
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

    // 1) resources (see [`enumerate`]).
    let (resources, control_graphs) = enumerate(graph, suffix);
    for r in &resources {
        out.push([named(r), is_resource.clone(), xsd_true()]);
    }

    // 2) control-document linkage by naming convention: <R + ".acl"> controls <R>.
    //    The .acl/.acr graphs are themselves resources too (Control gates them).
    for (iri, _) in &control_graphs {
        let r = &iri[..iri.len() - suffix.len()];
        out.push([named(r), owns.clone(), named(iri)]);
    }

    // 3) the access-control graphs' triples (skolemized) + inDoc provenance + WebIDs,
    //    and (WAC) group documents referenced via acl:agentGroup.
    let mut group_docs: FxHashSet<String> = FxHashSet::default();
    let mut webids: FxHashSet<String> = FxHashSet::default();
    let mut principal_iris: FxHashSet<String> = FxHashSet::default();
    for (iri, sub) in &control_graphs {
        let mut in_doc: FxHashSet<Term> = FxHashSet::default();
        for t in graph_triples(sub) {
            // sq-3jtd.5: hard-reject any forged derivation-internal fact
            // (`solidx:creator|owner|appliesToResource|…`) smuggled into the control
            // document — only the loader/rules may produce `solidx:` facts.
            if is_reserved_derivation_predicate(&t) {
                continue;
            }
            let s = skolemize(&t[0], iri);
            in_doc.insert(s.clone());
            out.push([s, skolemize(&t[1], iri), skolemize(&t[2], iri)]);
            collect_agents(&t, &mut webids, &mut group_docs, &mut principal_iris);
        }
        for s in in_doc {
            out.push([s, in_doc_p.clone(), named(iri)]);
        }
    }
    if system == System::Wac {
        for (name, sub) in &graph.named {
            let Some(iri) = graph_iri(name) else { continue };
            // Same reserved-space skip as the resource pass above: a forged
            // `<urn:sparq:…>` graph named by an `acl:agentGroup` must never be read.
            if iri.starts_with(RESERVED_PREFIX) || !group_docs.contains(iri) {
                continue;
            }
            for t in graph_triples(sub) {
                // sq-3jtd.5: same derivation-internal guard for group
                // documents — a forged `solidx:` fact here would feed the reasoner too.
                if is_reserved_derivation_predicate(&t) {
                    continue;
                }
                out.push([
                    skolemize(&t[0], iri),
                    skolemize(&t[1], iri),
                    skolemize(&t[2], iri),
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
    // sq-3jtd.5: TRUSTED creator/owner facts (ACP only). Emitted ONLY from the
    // caller-supplied provenance map — never from pod content (design doc §2.4). The
    // WebIDs go through the SAME reserved-encoding validation as agents/clients/issuers
    // (they become candidate agents in the rules) and are marked isWebId so the
    // candidate-generation lattice picks them up.
    if system == System::Acp {
        for (resource, creator, owner) in provenance.iter() {
            // Only for a resource the view holds: the store keeps no facts for any other
            // (see [`resources`]), and one created again later has none.
            if !resources.contains(resource) {
                continue;
            }
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
        // sq-ysv3u: TRUSTED `acp:vc` holdings (ACP only), on exactly the same
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
/// fact entry of the compiled evaluator (sq-zgbso.4).
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
/// (sq-zgbso.4). The production materializer feeds
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
/// the auth index + session cache for an ACL-write invalidation (sq-b7k7u).
/// `https://pod.ex/a/b` → `https://pod.ex`; an authority with no path (`https://pod.ex`)
/// is its own origin. An IRI with no `://` (e.g. `urn:…`) returns the whole IRI — a
/// self-contained fallback bucket only ever invalidated by a FULL re-materialization.
///
/// Soundness of scoping on this key: `reindex_with`'s **diff-based** invalidation
/// (sq-b7k7u fix) diffs old vs new `AuthIndex` per-origin and invalidates
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

/// The auth view's resources in `graph`, and its control documents ending in `suffix`.
/// The resources are every graph that is not reserved, not an `.acl`/`.acr` and has an
/// authority, plus every structural container above one (a container is an inheritance
/// anchor even without a graph of its own, but not when only control documents sit below
/// it). The materializer and the store's provenance pruning both use this, so trusted
/// creator/owner facts are kept for exactly the resources the view holds.
fn enumerate<'g>(graph: &'g Graph, suffix: &str) -> (FxHashSet<String>, Vec<(&'g str, &'g Graph)>) {
    let mut resources: FxHashSet<String> = FxHashSet::default();
    let mut control_graphs: Vec<(&str, &Graph)> = Vec::new(); // (name, sub)
    for (name, sub) in &graph.named {
        let Some(iri) = graph_iri(name) else { continue };
        // Skip the whole reserved space (`<urn:sparq:auth>` included): a graph there is
        // never reasoning input. [`strip_reserved_graphs`] also drops these from the
        // dataset, but assembly must not DEPEND on that having run first — the
        // materializer defers every mutation until after this fallible pass so an error
        // leaves the previous auth view in place (roborev 5005).
        if iri.starts_with(RESERVED_PREFIX) {
            continue;
        }
        if is_control_graph(iri) {
            if iri.ends_with(suffix) && !is_authority_less(control_target(iri)) {
                control_graphs.push((iri, sub));
            }
            continue;
        }
        if is_resource_graph(iri) {
            insert_with_containers(&mut resources, iri);
        }
    }
    (resources, control_graphs)
}

/// Whether the stored graph `iri` is an access-control document.
fn is_control_graph(iri: &str) -> bool {
    iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX)
}

/// The resource an access-control document `iri` governs.
/// The resources whose own access-control document (`.acl` or `.acr`) is stored in `graph`.
pub(crate) fn controlled_resources(graph: &Graph) -> FxHashSet<&str> {
    graph
        .named
        .iter()
        .filter_map(|(n, _)| graph_iri(n))
        .filter(|iri| iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX))
        .map(control_target)
        .collect()
}

fn control_target(iri: &str) -> &str {
    iri.strip_suffix(ACL_SUFFIX)
        .or_else(|| iri.strip_suffix(ACR_SUFFIX))
        .unwrap_or(iri)
}

/// Whether the stored graph `iri` is a resource of the auth view by itself: outside the
/// reserved space, not an access-control document, and below an authority. A bare
/// `scheme://` names no authority, so it is not a container of any pod root, and neither it
/// nor its control document takes part in the auth view. (The parent rule's textual scrape
/// of `https://pod.ex/` yields `https://`; [`parent_iri`] already stops at the authority
/// root.)
fn is_resource_graph(iri: &str) -> bool {
    !iri.starts_with(RESERVED_PREFIX) && !is_control_graph(iri) && !is_authority_less(iri)
}

/// Whether `iri` can be a structural container: a `/`-terminated IRI at or below an
/// authority root (`scheme://host/…`), the only names [`parent_iri`] yields.
fn is_container_name(iri: &str) -> bool {
    iri.find("://")
        .is_some_and(|i| iri[i + 3..].contains('/') && !is_authority_less(iri))
}

/// Add `iri` and its structural container chain (https://host/a/b/doc -> /a/b/ -> /a/ -> /).
fn insert_with_containers(resources: &mut FxHashSet<String>, iri: &str) {
    resources.insert(iri.to_owned());
    let mut cur = iri;
    while let Some(parent) = parent_iri(cur) {
        if !resources.insert(parent.to_owned()) {
            break;
        }
        cur = parent;
    }
}

/// The stored graphs that are resources by themselves (see [`is_resource_graph`]), kept in
/// order so the resources they make can be tested one at a time: a graph is a resource, and
/// so is every container on its chain (see [`enumerate`]). Every lifecycle decision about
/// the stored provenance is made against this one rule.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceGraphs(std::collections::BTreeSet<String>);

impl ResourceGraphs {
    /// The resource graphs among the names of `graph`'s named graphs.
    pub(crate) fn of(graph: &Graph) -> Self {
        Self::from_names(graph.named.iter().filter_map(|(n, _)| graph_iri(n)))
    }

    /// The resource graphs among `names`.
    pub(crate) fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        ResourceGraphs(
            names
                .into_iter()
                .filter(|n| is_resource_graph(n))
                .map(str::to_owned)
                .collect(),
        )
    }

    /// Whether `iri` is a resource: a stored resource graph, or a container on the chain of
    /// one. Every `/`-terminated prefix of a graph name (below its authority) is on its chain,
    /// so a container is held exactly when some graph name starts with it.
    pub(crate) fn holds(&self, iri: &str) -> bool {
        self.0.contains(iri)
            || (iri.ends_with('/')
                && is_container_name(iri)
                && self
                    .0
                    .range::<str, _>((std::ops::Bound::Included(iri), std::ops::Bound::Unbounded))
                    .next()
                    .is_some_and(|n| n.starts_with(iri)))
    }

    /// Every resource, with its containers.
    pub(crate) fn all(&self) -> FxHashSet<String> {
        let mut out = FxHashSet::default();
        for name in &self.0 {
            insert_with_containers(&mut out, name);
        }
        out
    }

    /// Store a graph named `name`, calling `appears` for each resource it makes that was
    /// not one just before (the graph itself and the containers it brings).
    pub(crate) fn add(&mut self, name: &str, mut appears: impl FnMut(&str)) {
        if !is_resource_graph(name) || self.0.contains(name) {
            return;
        }
        let mut chain = vec![name];
        let mut cur = name;
        while let Some(parent) = parent_iri(cur) {
            chain.push(parent);
            cur = parent;
        }
        let fresh: Vec<&str> = chain.into_iter().filter(|r| !self.holds(r)).collect();
        self.0.insert(name.to_owned());
        fresh.into_iter().for_each(&mut appears);
    }

    /// Remove the graph named `name`, calling `gone` for each resource that is not one any
    /// more (the graph itself, and each container left with no graph below it).
    pub(crate) fn remove(&mut self, name: &str, mut gone: impl FnMut(&str)) {
        if !self.0.remove(name) {
            return;
        }
        let mut cur = name;
        if !self.holds(cur) {
            gone(cur);
        }
        while let Some(parent) = parent_iri(cur) {
            if self.holds(parent) {
                break;
            }
            gone(parent);
            cur = parent;
        }
    }

    /// Remove every graph, calling `gone` for every resource.
    pub(crate) fn clear(&mut self, mut gone: impl FnMut(&str)) {
        self.all().iter().for_each(|r| gone(r));
        self.0.clear();
    }
}

/// Whether an access-control resource document in `graph` names `acp:CreatorAgent` or
/// `acp:OwnerAgent`: whether an ACP view built from it depends on the trusted provenance.
pub(crate) fn control_documents_name_provenance(graph: &Graph) -> bool {
    graph.named.iter().any(|(name, g)| {
        graph_iri(name).is_some_and(|iri| iri.ends_with(ACR_SUFFIX))
            && graph_triples(g).iter().any(|t| {
                matches!(&t[2], Term::NamedNode(o) if SPECIAL_AGENTS[2..4].contains(&o.as_str()))
            })
    })
}

/// The auth view's resources in `graph` (see [`enumerate`]).
pub(crate) fn resources(graph: &Graph) -> FxHashSet<String> {
    enumerate(graph, ACR_SUFFIX).0
}

/// Whether `iri` is exactly a bare `scheme://` with no authority — above every authority
/// root. The `://` must follow the scheme itself (an opaque IRI such as
/// `urn:example:x://` does not match).
fn is_authority_less(iri: &str) -> bool {
    let Some(scheme) = iri.strip_suffix("://") else { return false };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Solid slash-semantics parent of an IRI (None at/above the authority root): the one
/// container traversal the auth view, the store's provenance and the MCP server's
/// containment all use.
pub fn parent_iri(iri: &str) -> Option<&str> {
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
    use super::{assemble_facts, graph_triples, named, skolemize, System};
    use super::{iri_origin, parent_iri};
    use crate::{
        AccessProvenance, Mode, PodStore, Session, VerifiedCredentials, AUTH_GRAPH, AUTH_NS,
    };
    use oxrdf::{BlankNode, Literal, NamedNode, Term};
    use rustc_hash::FxHashSet;
    use sparq_core::Graph;

    const ACR: &str = "https://pod.ex/doc.acr";
    const OTHER_ACR: &str = "https://other.ex/doc.acr";

    #[test]
    fn skolem_components_are_valid_and_unambiguous() {
        let graphs = [
            "https://pod.ex/doc.acr",
            "https://pod.ex/doc:part.acr",
            "https://pod.ex/doc%3Apart.acr",
            "https://例.example/文?x=1&y=2#片",
            "urn:doc:a",
            "urn:doc:a:b",
        ];
        let labels = ["b", "b:c", "é", "c"];
        let mut seen = FxHashSet::default();
        for graph in graphs {
            NamedNode::new(graph).unwrap();
            for label in labels {
                let blank = Term::BlankNode(BlankNode::new(label).unwrap());
                let skolem = skolemize(&blank, graph);
                let Term::NamedNode(node) = &skolem else {
                    panic!("expected skolem IRI")
                };
                NamedNode::new(node.as_str()).unwrap();
                assert_eq!(skolem, skolemize(&blank, graph));
                assert!(
                    seen.insert(skolem),
                    "distinct graph/label pairs must not collide"
                );
            }
        }
        assert_eq!(
            skolemize(
                &Term::BlankNode(BlankNode::new("b:c").unwrap()),
                "urn:doc:a"
            ),
            named("urn:skolem:graph:urn%3Adoc%3Aa:b%3Ac")
        );
        for term in [
            named("https://pod.ex/node"),
            Term::Literal(Literal::new_simple_literal("value")),
        ] {
            assert_eq!(skolemize(&term, ACR), term);
        }
    }

    #[test]
    fn wac_group_and_control_blank_nodes_keep_distinct_stable_scopes() {
        let mut graph = Graph::new();
        // Construct the same label directly: separate parser calls may rename it.
        for (iri, predicate, object) in [
            ("https://pod.ex/doc.acl", super::ACL_AGENT_GROUP, "https://pod.ex/group#team"),
            ("https://pod.ex/group", super::VCARD_MEMBER, "https://alice.ex/card#me"),
        ] {
            let mut sub = Graph::new();
            sub.insert_triple(BlankNode::new("shared").unwrap(), named(predicate), named(object))
                .unwrap();
            graph.named.push((named(iri), sub));
        }
        let facts = |g: &Graph| -> FxHashSet<_> {
            assemble_facts(
                g,
                System::Wac,
                &AccessProvenance::new(),
                &VerifiedCredentials::default(),
            )
            .unwrap()
            .into_iter()
            .collect()
        };
        let before = facts(&graph);
        let subjects: FxHashSet<_> = before
            .iter()
            .filter(|t| t[1] == named(super::ACL_AGENT_GROUP) || t[1] == named(super::VCARD_MEMBER))
            .map(|t| t[0].clone())
            .collect();
        assert_eq!(
            subjects.len(),
            2,
            "the same label belongs to separate documents"
        );
        graph.named.reverse();
        assert_eq!(facts(&graph), before);
    }

    fn blank_matcher_store() -> PodStore {
        let mut graph = Graph::new();
        graph.named.push((named(OTHER_ACR), Graph::new()));
        graph
            .named
            .push((named("https://pod.ex/doc"), Graph::new()));
        graph.named.push((
            named(ACR),
            Graph::load_str(
                include_str!("../tests/fixtures/blank_node_matcher.ttl"),
                "turtle",
            )
            .unwrap(),
        ));
        let mut store = PodStore::new(graph);
        store.materialize_acp().unwrap();
        store
    }

    fn assert_blank_matcher_decisions(store: &PodStore) {
        for (agent, allowed) in [
            (None, true),
            (Some("https://alice.ex/card#me"), true),
            (Some("https://bob.ex/card#me"), false),
        ] {
            let session = Session {
                agent,
                ..Session::default()
            };
            for mode in [Mode::Read, Mode::Write] {
                let expected = if mode == Mode::Read && allowed {
                    vec!["https://pod.ex/doc"]
                } else {
                    vec![]
                };
                let actual = store.accessible(&session, mode);
                assert_eq!(
                    actual.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }

    #[test]
    fn blank_node_matcher_authorization_decisions() {
        assert_blank_matcher_decisions(&blank_matcher_store());
    }

    fn skolem_iris(store: &PodStore) -> FxHashSet<Term> {
        assemble_facts(
            &store.graph,
            System::Acp,
            &AccessProvenance::new(),
            &VerifiedCredentials::default(),
        )
        .unwrap()
        .into_iter()
        .flatten()
        .filter(|t| matches!(t, Term::NamedNode(n) if n.as_str().starts_with("urn:skolem:")))
        .collect()
    }

    #[test]
    fn blank_node_matchers_survive_unrelated_put_and_delete() {
        for delete in [false, true] {
            let mut store = blank_matcher_store();
            assert_blank_matcher_decisions(&store);
            let before = skolem_iris(&store);
            assert_eq!(before.len(), 2, "both anonymous matchers are present");
            let old_auth = store.auth.clone();
            let auth_graph = &store
                .graph
                .named
                .iter()
                .find(|(n, _)| *n == named(AUTH_GRAPH))
                .unwrap()
                .1;
            assert!(
                graph_triples(auth_graph)
                    .iter()
                    .any(|t| t[1] == named(&format!("{AUTH_NS}exceptMatcher"))
                        && before.contains(&t[2])),
                "the blank noneOf matcher reaches the auth view"
            );
            let position = |s: &PodStore| {
                s.graph
                    .named
                    .iter()
                    .position(|(n, _)| *n == named(ACR))
                    .unwrap()
            };
            let old_position = position(&store);
            if delete {
                assert!(store.delete_acl_acp(OTHER_ACR).unwrap().existed);
            } else {
                assert!(store.put_acl_acp(OTHER_ACR, "", "turtle").unwrap().existed);
            }
            assert_ne!(position(&store), old_position, "exercise graph renumbering");
            assert_eq!(
                skolem_iris(&store),
                before,
                "unrelated writes preserve skolem IRIs"
            );
            assert!(
                old_auth.matchers_eq(&store.auth),
                "matcher diff must stay unchanged"
            );
            store
                .cache
                .with_entry(&(None, None, None, None, Mode::Read), |entry| {
                    let entry = entry.expect("the warmed anonymous session survives the write");
                    assert!(
                        entry.memo.is_some() && entry.dirty.is_empty(),
                        "an unrelated write leaves the warmed session's memo intact"
                    );
                });
            assert_blank_matcher_decisions(&store);
        }
    }

    #[test]
    fn parent_walks_to_root_and_stops() {
        assert_eq!(parent_iri("https://pod.ex/a/b/doc.ttl"), Some("https://pod.ex/a/b/"));
        assert_eq!(parent_iri("https://pod.ex/a/b/"), Some("https://pod.ex/a/"));
        assert_eq!(parent_iri("https://pod.ex/a/"), Some("https://pod.ex/"));
        assert_eq!(parent_iri("https://pod.ex/"), None);
        assert!(super::is_authority_less("https://"));
        assert!(!super::is_authority_less("https://pod.ex/"));
        assert!(!super::is_authority_less("file:///pod/"));
        assert!(!super::is_authority_less("urn:x:y"));
        assert!(!super::is_authority_less("urn:example:resource://"));
        assert!(super::is_authority_less("HTTPS://"));
    }

    #[test]
    fn iri_origin_is_scheme_authority() {
        // sq-b7k7u: origin = scheme://authority, stable across the whole subtree.
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

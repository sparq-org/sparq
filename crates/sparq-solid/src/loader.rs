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
//! The `.acl`/`.acr`/group graphs are emitted as they are, so each must speak only for
//! itself. [`admits`] keeps a document's triple only when it describes one of the
//! document's own nodes, names no minted `urn:skolem:` IRI, and uses none of the view's
//! `auth:`/`solidx:` vocabulary (which only this loader and the rules produce). A document
//! with any other triple contributes nothing, and the resources it governs fail closed
//! (see [`assemble_facts`]). Principal values in the view's reserved spaces are refused
//! ([`validate_principal_iri`]).

use crate::budget::AuthLedger;
#[cfg(test)]
use crate::budget::AuthBudget;
use crate::{AccessProvenance, VerifiedCredentials, AUTH_NS, SOLIDX_NS};
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_core::meter::Dim;

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

/// Whether `iri` is outside every principal the view mints itself: the reserved pair
/// encoding, the minted skolem names, and the `auth:`/`solidx:` names (`auth:Public`,
/// `auth:AnyClient`, …) the rules and the index use for wildcard principals.
fn external_principal(iri: &str) -> bool {
    !iri.starts_with(RESERVED_PREFIX)
        && !iri.starts_with(SKOLEM_PREFIX)
        && !iri.starts_with(AUTH_NS)
        && !iri.starts_with(SOLIDX_NS)
        && !iri.contains(PAIR_DELIMITER)
}

pub(crate) fn validate_principal_iri(iri: &str) -> Result<(), String> {
    if !external_principal(iri) {
        return Err(format!(
            "agent/client/origin IRI <{iri}> is not allowed: \
             `{RESERVED_PREFIX}`, `{SKOLEM_PREFIX}`, `{AUTH_NS}`, `{SOLIDX_NS}` and the literal \
             `{PAIR_DELIMITER}` are reserved for the principals the view mints"
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
///
/// The same holds for the `auth:` view vocabulary: only the rules conclude `auth:`
/// facts and `rdf:type auth:…` classes, and those are what the auth view installs. A
/// document triple in that vocabulary is dropped too, as is an `rdf:type` naming a
/// `solidx:` or `auth:` class.
fn is_reserved_derivation_predicate(t: &[Term; 3]) -> bool {
    let reserved = |iri: &str| iri.starts_with(SOLIDX_NS) || iri.starts_with(AUTH_NS);
    match (&t[1], &t[2]) {
        (Term::NamedNode(p), _) if reserved(p.as_str()) => true,
        (Term::NamedNode(p), Term::NamedNode(o)) => {
            p.as_str() == RDF_TYPE && reserved(o.as_str())
        }
        _ => false,
    }
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The IRI prefix [`skolemize`] mints for document blank nodes. Loaded data may not
/// supply it, as a term or as a graph name: either would denote another document's blank
/// node.
const SKOLEM_PREFIX: &str = "urn:skolem:";

/// Whether `t` names an IRI the view mints itself: a skolem name ([`SKOLEM_PREFIX`]) or a
/// name in the reserved space ([`RESERVED_PREFIX`]: principals, candidates, matcher
/// instances, grants), in any position, as a datatype or inside a triple term. No document
/// can then address a node the rules create.
fn names_minted_iri(t: &[Term; 3]) -> bool {
    t.iter().any(term_names_minted_iri)
}

/// Whether `x` names a minted IRI anywhere: as itself, as a literal's datatype, or inside a
/// triple term.
fn term_names_minted_iri(x: &Term) -> bool {
    let minted = |n: &str| n.starts_with(SKOLEM_PREFIX) || n.starts_with(RESERVED_PREFIX);
    match x {
        Term::NamedNode(n) => minted(n.as_str()),
        Term::BlankNode(_) => false,
        Term::Literal(l) => minted(l.datatype().as_str()),
        Term::Triple(tt) => {
            term_names_minted_iri(&tt.subject.clone().into())
                || minted(tt.predicate.as_str())
                || term_names_minted_iri(&tt.object)
        }
    }
}

/// Whether document `graph_iri` may state triple `t`: a document describes only its own
/// nodes, i.e. a blank node or an IRI whose fragment-stripped document is `graph_iri`
/// itself, names no minted skolem IRI, and uses none of the view's vocabulary. It can
/// still reference any IRI as an object, which is how an ACR applies a policy defined in
/// another document or an ACL names a group, but statements about a node defined
/// elsewhere come only from that node's own document. A document with any triple it may
/// not state contributes nothing (see [`assemble_facts`]).
///
/// A reference is still a dependency: an ACR applying a policy from another document, or
/// an ACL naming a group document, trusts whoever may edit that document, as in Solid.
/// Pointing at a document is the owner's choice; what this check removes is a document
/// describing nodes it does not define.
fn admits(t: &[Term; 3], graph_iri: &str) -> bool {
    !is_reserved_derivation_predicate(t)
        && !names_reserved_principal(t)
        && !names_minted_iri(t)
        && speaks_for(t, graph_iri)
}

/// Refuse a write that would make the access-control document `doc` state a triple it may not
/// (see [`admits`]): the loader would reject the whole document, and its resource would be
/// left with no grants, Control included, so the writer could not repair it. Only `added`
/// triples are checked, so a write removing such a triple is never refused.
fn refuse_inadmissible(doc: &str, added: &[[Term; 3]]) -> Result<(), String> {
    match added.iter().find(|t| !admits(t, doc)) {
        None => Ok(()),
        Some(t) => Err(format!(
            "the access-control document <{doc}> may not state `{} {} {}`: it may describe only \
             its own nodes, in the access-control vocabularies, naming no reserved name or \
             principal; nothing was changed",
            t[0], t[1], t[2]
        )),
    }
}

/// The check of what one write adds to the access-control documents the active view reads
/// (`.acr` under ACP, `.acl` under WAC, outside the reserved and skolem spaces and below an
/// authority; the loader filters any other per triple): [`refuse_inadmissible`], and, in an
/// ACP store, a statement that uses (as an access control, a policy or a matcher) a node of
/// another access-control resource the loader would reject in the state the write leaves, or
/// of one it would not read at all, either of which would withhold the written one with it.
/// The view of the control documents is computed at most once per
/// write. The refusal names the statement the write sent and says nothing about the state of
/// the document it reaches into.
pub(crate) struct ControlWrites<'g> {
    graph: &'g Graph,
    acp: bool,
    /// A document whose content the write replaces, with its new content, when `graph` does
    /// not hold it yet.
    replaced: Option<(&'g str, Vec<[Term; 3]>)>,
    documents: std::cell::OnceCell<Documents<'g>>,
}

impl<'g> ControlWrites<'g> {
    /// The check for a write that leaves the store as `graph`, whose view is ACP when `acp`.
    pub(crate) fn new(graph: &'g Graph, acp: bool) -> Self {
        ControlWrites {
            graph,
            acp,
            replaced: None,
            documents: std::cell::OnceCell::new(),
        }
    }

    /// The check for a write that replaces document `doc` of `graph` with `content`.
    pub(crate) fn replacing(graph: &'g Graph, acp: bool, doc: &'g str, content: Vec<[Term; 3]>) -> Self {
        ControlWrites {
            replaced: Some((doc, content)),
            ..Self::new(graph, acp)
        }
    }

    /// The control graphs the ACP loader reads from the state the write leaves, but `except`.
    fn control_graphs(&self, except: Option<&str>) -> Vec<(&'g str, Vec<[Term; 3]>)> {
        let replaced = self.replaced.as_ref().map(|(doc, _)| *doc);
        let mut graphs: Vec<(&'g str, Vec<[Term; 3]>)> = self
            .graph
            .named()
            .iter()
            .filter_map(|(n, g)| {
                graph_iri(n)
                    .filter(|iri| {
                        Some(*iri) != except
                            && Some(*iri) != replaced
                            && reads_as_control(iri, ACR_SUFFIX)
                    })
                    .map(|iri| (iri, graph_triples(g)))
            })
            .collect();
        if let Some((doc, content)) = &self.replaced {
            if Some(*doc) != except && reads_as_control(doc, ACR_SUFFIX) {
                graphs.push((doc, content.clone()));
            }
        }
        graphs
    }

    /// Refuse `added` to the access-control document `doc`, naming the first statement it
    /// may not make.
    pub(crate) fn check(&self, doc: &str, added: &[[Term; 3]]) -> Result<(), String> {
        let suffix = if self.acp { ACR_SUFFIX } else { ACL_SUFFIX };
        if !reads_as_control(doc, suffix) {
            return Ok(());
        }
        refuse_inadmissible(doc, added)?;
        let depends = |t: &[Term; 3]| {
            matches!((&t[1], &t[2]), (Term::NamedNode(p), Term::NamedNode(o))
                if ACP_DEPENDENCY_PREDICATES.contains(&p.as_str())
                    && group_doc_of(o.as_str()) != doc)
        };
        if !self.acp || !added.iter().any(&depends) {
            return Ok(());
        }
        let documents = self
            .documents
            .get_or_init(|| document_view(&self.control_graphs(None), System::Acp));
        // A document the store rejects now may be under repair: what it would depend on is
        // judged without it.
        let without_doc;
        let documents = if documents.rejected.contains(doc) {
            without_doc = document_view(&self.control_graphs(Some(doc)), System::Acp);
            &without_doc
        } else {
            documents
        };
        // A reference the view does not resolve — to a document it rejects, or to one it does
        // not read at all — would withhold `doc` too (see [`document_view`]), Control
        // included, so the writer could not repair it. `depends` has already excluded a
        // reference to `doc`'s own nodes, which the state the write leaves always resolves.
        let unresolved = |t: &&[Term; 3]| {
            depends(t)
                && matches!(&t[2], Term::NamedNode(o)
                    if !documents.loaded.contains(group_doc_of(o.as_str()))
                        || documents.rejected.contains(group_doc_of(o.as_str())))
        };
        if !added.iter().any(|t| unresolved(&t)) {
            return Ok(());
        }
        // One body for every such refusal. It names neither the statement nor the reference
        // that failed: WHICH of the writer's references the view does not resolve is a fact
        // about ANOTHER resource, and a writer holding Control of `doc` need hold nothing over
        // that one. One bit remains — this write is refused where an otherwise identical one
        // is accepted — and is not closed here; a caller that must not expose even that
        // orders its own writes so that a document is stored before one that uses its nodes.
        Err(format!(
            "the access-control document <{}> may not use a node of another \
             access-control document; nothing was changed",
            doc
        ))
    }
}

/// Whether the loader reads the stored graph `iri` as a control document with `suffix`:
/// outside the reserved and skolem spaces, and governing a resource below an authority.
fn reads_as_control(iri: &str, suffix: &str) -> bool {
    !iri.starts_with(RESERVED_PREFIX)
        && !iri.starts_with(SKOLEM_PREFIX)
        && iri
            .strip_suffix(suffix)
            .is_some_and(|r| !is_authority_less(r))
}

/// Whether `t` names, as an agent, group member, origin, client or issuer, a principal the
/// view keeps for itself: one of its own `auth:`/`solidx:` principals (`auth:Public`,
/// `auth:AnyClient`, …), a name in the reserved or skolem space, or one containing the pair
/// delimiter (see [`external_principal`]). Such a document is rejected like any other, so
/// no document can make the whole materialization fail.
fn names_reserved_principal(t: &[Term; 3]) -> bool {
    let (Term::NamedNode(p), Term::NamedNode(o)) = (&t[1], &t[2]) else { return false };
    PRINCIPAL_PREDICATES.contains(&p.as_str()) && !external_principal(o.as_str())
}

const PRINCIPAL_PREDICATES: [&str; 6] =
    [ACL_AGENT, ACP_AGENT, VCARD_MEMBER, ACL_ORIGIN, ACP_CLIENT, ACP_ISSUER];

fn speaks_for(t: &[Term; 3], graph_iri: &str) -> bool {
    match &t[0] {
        Term::BlankNode(_) => true,
        Term::NamedNode(s) => group_doc_of(s.as_str()) == graph_iri,
        _ => false,
    }
}

/// The ACP edges along which a control document depends on the document defining the
/// object: the access controls, policies and matchers ACP evaluation reads.
const ACP_DEPENDENCY_PREDICATES: [&str; 6] = [
    "http://www.w3.org/ns/solid/acp#accessControl",
    "http://www.w3.org/ns/solid/acp#memberAccessControl",
    "http://www.w3.org/ns/solid/acp#apply",
    "http://www.w3.org/ns/solid/acp#allOf",
    "http://www.w3.org/ns/solid/acp#anyOf",
    "http://www.w3.org/ns/solid/acp#noneOf",
];

/// What the loader makes of the control documents of one store state.
struct Documents<'a> {
    /// The control documents the loader reads, by graph IRI. A node whose
    /// fragment-stripped document is not in here is one no loaded document defines: its
    /// statements are not available, so neither is what they would have decided.
    loaded: FxHashSet<&'a str>,
    /// The control documents that contribute nothing (see [`document_view`]).
    rejected: FxHashSet<&'a str>,
}

/// The control documents the loader reads, and those among them that contribute nothing:
/// each with a triple it may not state ([`admits`]) and, for ACP, each that names, as an
/// access control, a policy or a matcher, a node of such a document OR a node of a
/// document that is not loaded at all, to a fixpoint.
///
/// The second case is the one a reference out of the loaded set falls into — another pod's
/// document, or a document a partial load left out. What such a node says is unknown, and
/// an unknown statement may be an `acp:deny`, an `acp:noneOf` exception or a condition on
/// an allow; a loaded document, by contrast, is known in full, so a node it simply does
/// not describe is a known-empty access control, policy or matcher and decides as ACP
/// §6.5 says it does. So a document that names a node no loaded document defines is
/// withheld like a rejected one, through every referrer, and the resources it governs get
/// no grant from any source. A reference whose target IS loaded, in this pod or another,
/// keeps applying: naming it delegates to whoever may edit that document, which is the
/// referring document's own choice (see [`admits`]).
fn document_view<'a>(
    control_graphs: &[(&'a str, Vec<[Term; 3]>)],
    system: System,
) -> Documents<'a> {
    document_view_metered(control_graphs, system, None).expect("unmetered document validation")
}

fn document_view_metered<'a>(
    control_graphs: &[(&'a str, Vec<[Term; 3]>)],
    system: System,
    mut meter: Option<&mut AuthLedger>,
) -> Result<Documents<'a>, String> {
    let loaded: FxHashSet<&'a str> = control_graphs.iter().map(|(iri, _)| *iri).collect();
    let mut rejected: FxHashSet<&str> = control_graphs
        .iter()
        .filter(|(iri, triples)| !triples.iter().all(|t| admits(t, iri)))
        .map(|(iri, _)| *iri)
        .collect();
    if system == System::Acp {
        // Referenced document -> the control documents that use a node of it as an access
        // control, a policy or a matcher. A principal or any other value named as an object
        // is not a dependency. A reference to a document outside the loaded set resolves to
        // nothing, so it withholds its referrer straight away.
        let mut referrers: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
        for (iri, triples) in control_graphs {
            for t in triples {
                let Term::NamedNode(p) = &t[1] else { continue };
                if !ACP_DEPENDENCY_PREDICATES.contains(&p.as_str()) {
                    continue;
                }
                if let Term::NamedNode(o) = &t[2] {
                    let target = group_doc_of(o.as_str());
                    if loaded.contains(target) {
                        let refs = referrers.entry(target).or_default();
                        if let Some(meter) = meter.as_deref_mut() {
                            meter.reserve(refs, 1, Dim::Nodes)?;
                        }
                        refs.push(iri);
                    } else {
                        rejected.insert(iri);
                    }
                }
            }
        }
        let mut work = Vec::new();
        if let Some(meter) = meter.as_deref_mut() {
            meter.reserve(&mut work, rejected.len(), Dim::Nodes)?;
        }
        work.extend(rejected.iter().copied());
        while let Some(doc) = work.pop() {
            for &referrer in referrers.get(doc).into_iter().flatten() {
                if rejected.insert(referrer) {
                    if let Some(meter) = meter.as_deref_mut() {
                        meter.reserve(&mut work, 1, Dim::Nodes)?;
                    }
                    work.push(referrer);
                }
            }
        }
    }
    Ok(Documents { loaded, rejected })
}

/// Drop ALL named graphs in the reserved IRI space — including a pre-existing
/// `<urn:sparq:auth>`: a loaded dataset must not be able to smuggle in the rewrite
/// sentinel (`urn:sparq:nothing`) or a FORGED auth view; only `install_auth_view`
/// creates `<urn:sparq:auth>`, after a successful materialization (roborev 1727).
///
/// Directory-backed roots remove reserved slots through the durable drop methods.
/// Returns an error when a removal cannot complete.
pub(crate) fn strip_reserved_graphs(graph: &mut Graph) -> Result<(), String> {
    if graph.is_directory_backed() {
        let reserved: Vec<_> = graph
            .named()
            .iter()
            .filter_map(|(name, _)| match name {
                Term::NamedNode(n) if n.as_str().starts_with(RESERVED_PREFIX) => Some(name.clone()),
                _ => None,
            })
            .collect();
        for name in reserved {
            drop_named_view(graph, &name)?;
        }
        return Ok(());
    }
    graph.retain_named(|name, _| match name {
        Term::NamedNode(n) => !n.as_str().starts_with(RESERVED_PREFIX),
        _ => true,
    })
}

/// Remove a view only when its slot exists; a durable drop rewrites surviving slots.
pub(crate) fn drop_named_view(graph: &mut Graph, name: &Term) -> Result<(), String> {
    if graph.named_graph(name).is_none() {
        return Ok(());
    }
    view_write_fault()?;
    if graph.is_directory_backed() {
        while graph.named_graph(name).is_some() {
            graph.drop_named_durable(name)?;
        }
        Ok(())
    } else {
        graph.retain_named(|n, _| n != name)
    }
}

#[cfg(test)]
std::thread_local! {
    pub(crate) static FAIL_VIEW_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static FAIL_ALL_VIEW_WRITES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
fn view_write_fault() -> Result<(), String> {
    #[cfg(test)]
    if FAIL_VIEW_WRITE.replace(false) || FAIL_ALL_VIEW_WRITES.get() {
        return Err("injected authorization view write failure".into());
    }
    Ok(())
}

/// Install a named view through logged writes when the dataset has a durable root.
pub(crate) fn install_named_view(graph: &mut Graph, name: Term, view: Graph) -> Result<(), String> {
    if graph.is_directory_backed() {
        let slot = graph.ensure_named(&name)?;
        let old: FxHashSet<_> = graph_triples(&graph.named()[slot].1).into_iter().collect();
        let new: FxHashSet<_> = graph_triples(&view).into_iter().collect();
        let inserts: Vec<_> = new.difference(&old).cloned().collect();
        let deletes: Vec<_> = old.difference(&new).cloned().collect();
        if inserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        view_write_fault()?;
        graph.apply_delta_to_named(slot, &inserts, &deletes)
    } else {
        graph.set_named(name, view).map(|_| ())
    }
}

/// Empties writable in-memory reserved slots when removal did not complete, preserving their
/// positions. Durable slots stay in place; the caller disables the authorization view until
/// materialization succeeds.
pub(crate) fn empty_reserved_graphs(graph: &mut Graph) {
    let reserved: Vec<Term> = graph
        .named()
        .iter()
        .filter(|(name, g)| {
            !g.is_directory_backed()
                && matches!(name, Term::NamedNode(n) if n.as_str().starts_with(RESERVED_PREFIX))
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in reserved {
        let _ = graph.set_named(name, Graph::new());
    }
}

/// `Err` unless the auth view can be installed in `graph`: it accepts writes (checked before
/// anything changes, so a refusal leaves the previous view in place).
pub(crate) fn require_writable(graph: &Graph) -> Result<(), String> {
    graph.check_writable()
}

/// Whether a session-supplied agent/client value may participate in principal
/// expansion: anything in the reserved space, in the view's own vocabulary, or containing
/// the pair delimiter could IMPERSONATE a principal the view mints — fail closed.
pub(crate) fn session_value_allowed(v: &str) -> bool {
    external_principal(v)
}
/// `acp:agent` objects that are NOT concrete WebIDs.
const SPECIAL_AGENTS: [&str; 5] = [
    "http://www.w3.org/ns/solid/acp#PublicAgent",
    "http://www.w3.org/ns/solid/acp#AuthenticatedAgent",
    "http://www.w3.org/ns/solid/acp#CreatorAgent",
    "http://www.w3.org/ns/solid/acp#OwnerAgent",
    "http://www.w3.org/ns/auth/acl#AuthenticatedAgent",
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum System {
    Wac,
    Acp,
}

/// [`graph_triples`] under the input budget `meter`, charging each term's bytes as it is
/// materialized.
///
/// This is the seam the per-document and aggregate byte budgets act at: the terms of a
/// control or group document are charged WHILE the owned triples are built, so a document
/// set past the budget stops being read at the limit instead of being materialized whole
/// and checked afterwards. `iri` names the document in a refusal.
pub(crate) fn graph_triples_metered(
    g: &Graph,
    iri: &str,
    meter: &mut AuthLedger,
) -> Result<Vec<[Term; 3]>, String> {
    meter.open()?;
    let pat: sparq_core::store::Pattern = [None, None, None];
    let scan = g.store().scan(&pat);
    let mut out = Vec::new();
    // The existing byte caps also bound the row vector before allocation.
    meter.document_rows(&mut out, scan.rows.len(), iri)?;
    for r in scan.rows.iter() {
        let t = scan.to_spo(r);
        for id in t {
            meter.stored_term(iri, g.dict(), id)?;
        }
        out.push([
            g.dict().term(t[0]),
            g.dict().term(t[1]),
            g.dict().term(t[2]),
        ]);
    }
    Ok(out)
}

/// The triples of one named graph as ground term triples.
pub(crate) fn graph_triples(g: &Graph) -> Vec<[Term; 3]> {
    let pat: sparq_core::store::Pattern = [None, None, None];
    let scan = g.store().scan(&pat);
    scan.rows
        .iter()
        .map(|r| {
            let t = scan.to_spo(r);
            [
                g.dict().term(t[0]),
                g.dict().term(t[1]),
                g.dict().term(t[2]),
            ]
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
#[cfg(test)]
fn skolemize(t: &Term, graph: &str) -> Term {
    skolemize_metered(t, graph, &mut AuthLedger::new(crate::default_auth_budget())).unwrap()
}

fn skolemize_metered(t: &Term, graph: &str, ledger: &mut AuthLedger) -> Result<Term, String> {
    match t {
        Term::BlankNode(b) => ledger
            .skolem(SKOLEM_PREFIX, graph, b.as_str())
            .map(|iri| NamedNode::new_unchecked(iri).into()),
        other => Ok(other.clone()),
    }
}

fn minted(ledger: &mut AuthLedger, iri: &str) -> Result<Term, String> {
    ledger
        .mint(&[iri])
        .map(|s| NamedNode::new_unchecked(s).into())
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
    meter: &mut AuthLedger,
) -> Result<Vec<[Term; 3]>, String> {
    meter.checkpoint()?;
    let mut out: Vec<[Term; 3]> = Vec::new();
    let suffix = if system == System::Wac {
        ACL_SUFFIX
    } else {
        ACR_SUFFIX
    };
    let own_pred = if system == System::Wac {
        "ownAcl"
    } else {
        "ownAcr"
    };
    let is_resource = named(&format!("{SOLIDX_NS}isResource"));
    let owns = named(&format!("{SOLIDX_NS}{own_pred}"));
    let in_doc_p = named(&format!("{SOLIDX_NS}inDoc"));
    let is_webid = named(&format!("{SOLIDX_NS}isWebId"));
    let true_term = xsd_true();

    // 1) resources (see [`enumerate`]).
    // The resources are enumerated under the SAME input meter as the documents, and
    // charged before their container chains are built: the enumeration's cost grows with
    // the pod's resource IRIs rather than with its documents' bytes or its fact count, so
    // neither of the budgets checked after this one bounds it.
    let (resources, control_graphs) = enumerate(graph, suffix, Some(meter))?;
    // Under the input budget: each document's terms are charged as they are read, so the
    // whole set is never materialized past it (sq-7159 follow-up, `crate::AuthBudget`).
    let mut documents = Vec::new();
    meter.reserve(&mut documents, control_graphs.len(), Dim::Nodes)?;
    for (iri, sub) in control_graphs {
        documents.push((iri, graph_triples_metered(sub, iri, meter)?));
    }
    let control_graphs = documents;
    // A control document with a triple it may not state contributes nothing. For ACP,
    // whose policies also deny, so does every ACR that references a node of a rejected
    // document or a node no loaded document defines (transitively — see [`document_view`]),
    // and the resources a rejected ACR governs, with everything
    // below them, get no grants at all: a deny or exception it held can never be lost
    // while an allow elsewhere still applies. A rejected WAC ACL keeps governing its
    // resource with no authorizations, so no ancestor's default takes over.
    meter.checkpoint()?;
    let rejected = document_view_metered(&control_graphs, system, Some(meter))?.rejected;
    let blocked: FxHashSet<&str> = if system == System::Acp {
        rejected
            .iter()
            .map(|iri| &iri[..iri.len() - suffix.len()])
            .collect()
    } else {
        FxHashSet::default()
    };
    // A blocked resource, or one below a blocked container.
    let is_blocked = |r: &str| {
        !blocked.is_empty()
            && (blocked.contains(r)
                || r.match_indices('/')
                    .any(|(i, _)| blocked.contains(&r[..=i])))
    };
    macro_rules! generated {
        ($triple:expr) => {{
            meter.reserve(&mut out, 1, Dim::Facts)?;
            out.push($triple);
        }};
    }
    macro_rules! copied {
        ($triple:expr) => {{
            meter.reserve(&mut out, 1, Dim::Facts)?;
            out.push($triple);
        }};
    }
    // Each resource's Solid slash-semantics parent, when that is a resource too: the one
    // container traversal ([`parent_iri`]) the store and the MCP server use, so the rules
    // walk exactly the containers the resource enumeration made.
    let parent_p = named(&format!("{SOLIDX_NS}parent"));
    for (batch, r) in resources.iter().enumerate() {
        if batch % 64 == 0 {
            meter.checkpoint()?;
        }
        if !is_blocked(r) {
            generated!([
                minted(meter, r)?,
                meter.copied_term(&is_resource)?,
                meter.copied_term(&true_term)?
            ]);
            if let Some(p) = parent_iri(r).filter(|p| resources.contains(*p) && !is_blocked(p)) {
                generated!([
                    minted(meter, r)?,
                    meter.copied_term(&parent_p)?,
                    minted(meter, p)?
                ]);
            }
        }
    }
    // The auth view carries the blocked resources too, so no other grant source (an
    // attached or bridged ODRL permission, a trust grant) applies to them either.
    for b in &blocked {
        generated!([
            minted(meter, b)?,
            minted(meter, crate::authindex::WITHHELD)?,
            meter.copied_term(&true_term)?
        ]);
    }
    // WAC: a resource whose own ACL was rejected is marked, and so (when any was) is each
    // resource whose own ACL was accepted, so the index can tell which ACL governs any
    // resource, stored or not, and withhold every grant source from those a rejected ACL
    // governs.
    if system == System::Wac && !rejected.is_empty() {
        for (iri, _) in &control_graphs {
            let r = &iri[..iri.len() - suffix.len()];
            let mark = if rejected.contains(iri) {
                crate::authindex::REJECTED_ACL
            } else {
                crate::authindex::ACCEPTED_ACL
            };
            generated!([
                minted(meter, r)?,
                minted(meter, mark)?,
                meter.copied_term(&true_term)?
            ]);
        }
    }

    meter.checkpoint()?;
    // 2) control-document linkage by naming convention: <R + ".acl"> controls <R>.
    //    The .acl/.acr graphs are themselves resources too (Control gates them).
    for (iri, _) in &control_graphs {
        meter.checkpoint()?;
        let r = &iri[..iri.len() - suffix.len()];
        if !is_blocked(r) {
            generated!([
                minted(meter, r)?,
                meter.copied_term(&owns)?,
                minted(meter, iri)?
            ]);
        }
    }

    // 3) the access-control graphs' triples (skolemized) + inDoc provenance + WebIDs,
    //    and (WAC) group documents referenced via acl:agentGroup.
    let mut group_docs: FxHashSet<String> = FxHashSet::default();
    let mut webids: FxHashSet<String> = FxHashSet::default();
    let mut principal_iris: FxHashSet<String> = FxHashSet::default();
    for (iri, triples) in &control_graphs {
        meter.checkpoint()?;
        if rejected.contains(iri) {
            continue;
        }
        let mut in_doc: FxHashSet<Term> = FxHashSet::default();
        for t in triples {
            let s = skolemize_metered(&t[0], iri, meter)?;
            if !in_doc.contains(&s) {
                in_doc.insert(meter.copied_term(&s)?);
            }
            copied!([
                s,
                skolemize_metered(&t[1], iri, meter)?,
                skolemize_metered(&t[2], iri, meter)?
            ]);
            collect_agents(t, &mut webids, &mut group_docs, &mut principal_iris);
        }
        for s in in_doc {
            generated!([
                s,
                meter.copied_term(&in_doc_p)?,
                minted(meter, iri)?
            ]);
        }
    }
    if system == System::Wac {
        for (batch, (name, sub)) in graph.named().iter().enumerate() {
            if batch % 64 == 0 {
                meter.checkpoint()?;
            }
            let Some(iri) = graph_iri(name) else { continue };
            // Same reserved-space skip as the resource pass above: a forged
            // `<urn:sparq:…>` graph named by an `acl:agentGroup` must never be read.
            if iri.starts_with(RESERVED_PREFIX)
                || iri.starts_with(SKOLEM_PREFIX)
                || !group_docs.contains(iri)
                || rejected.contains(iri)
            {
                continue;
            }
            // A group document only grants (it lists members), so a triple it may not
            // state is dropped on its own; the rest of the document still counts.
            for t in graph_triples_metered(sub, iri, meter)?
                .into_iter()
                .filter(|t| admits(t, iri))
            {
                copied!([
                    skolemize_metered(&t[0], iri, meter)?,
                    skolemize_metered(&t[1], iri, meter)?,
                    skolemize_metered(&t[2], iri, meter)?,
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
        for (batch, (resource, creator, owner)) in provenance.iter().enumerate() {
            if batch % 64 == 0 {
                meter.checkpoint()?;
            }
            // Every fact is validated, emitted or not: the store keeps the facts of a
            // resource that does not exist while its own `.acr` is stored, so a later rebuild
            // must not be the first to refuse one.
            for p in creator.into_iter().chain(owner) {
                validate_principal_iri(p)?;
            }
            // Emitted only for a resource the view holds.
            if !resources.contains(resource) {
                continue;
            }
            if let Some(c) = creator {
                if !webids.contains(c) {
                    webids.insert(meter.mint(&[c])?);
                }
                generated!([
                    minted(meter, resource)?,
                    minted(meter, &format!("{SOLIDX_NS}creator"))?,
                    minted(meter, c)?,
                ]);
            }
            if let Some(o) = owner {
                if !webids.contains(o) {
                    webids.insert(meter.mint(&[o])?);
                }
                generated!([
                    minted(meter, resource)?,
                    minted(meter, &format!("{SOLIDX_NS}owner"))?,
                    minted(meter, o)?
                ]);
            }
        }
        // sq-ysv3u: TRUSTED `acp:vc` holdings (ACP only), on exactly the same
        // footing as the creator/owner facts above — emitted ONLY from the caller-supplied
        // map, never from pod or `.acr` content (design doc §2.4; the solidx: guard at the
        // top of this pass drops any forged `holdsVc` smuggled into a control document).
        // A holder becomes a candidate agent, so its WebID goes through the SAME
        // reserved-encoding validation and is marked isWebId.
        for (batch, (agent, requirement)) in credentials.iter().enumerate() {
            if batch % 64 == 0 {
                meter.checkpoint()?;
            }
            validate_principal_iri(agent)?;
            if !webids.contains(agent) {
                webids.insert(meter.mint(&[agent])?);
            }
            generated!([
                minted(meter, agent)?,
                minted(meter, &format!("{SOLIDX_NS}holdsVc"))?,
                minted(meter, requirement)?
            ]);
        }
    }
    for a in &webids {
        generated!([
            minted(meter, a)?,
            meter.copied_term(&is_webid)?,
            meter.copied_term(&true_term)?
        ]);
    }
    // The assembled total — document content plus the structural, provenance and
    // credential facts — before any caller interns it.
    meter.checkpoint()?;
    Ok(out)
}

/// [`assemble_facts`] under [`crate::default_auth_budget`] — the shorthand the in-crate
/// tests that are not about the budget itself use.
#[cfg(test)]
pub(crate) fn assemble_facts_default(
    graph: &Graph,
    system: System,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
) -> Result<Vec<[Term; 3]>, String> {
    assemble_facts(
        graph,
        system,
        provenance,
        credentials,
        &mut AuthLedger::new(crate::default_auth_budget()),
    )
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
    meter: &mut AuthLedger,
) -> Result<Vec<[Id; 3]>, String> {
    let facts = assemble_facts(graph, system, provenance, credentials, meter)?;
    let mut ids = Vec::new();
    meter.reserve(&mut ids, facts.len(), Dim::Nodes)?;
    for batch in facts.chunks(256) {
        meter.checkpoint()?;
        ids.extend(
            batch
                .iter()
                .map(|t| [dict.intern(&t[0]), dict.intern(&t[1]), dict.intern(&t[2])]),
        );
    }
    meter.checkpoint()?;
    Ok(ids)
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
    budget: AuthBudget,
) -> Result<String, String> {
    use std::fmt::Write;
    let facts = assemble_facts(
        graph,
        system,
        provenance,
        credentials,
        &mut AuthLedger::new(budget),
    )?;
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
    for (name, sub) in graph.named().iter() {
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

/// What [`enumerate`] yields: the view's resources, and its control documents with the
/// stored graph of each.
type Enumerated<'g> = (FxHashSet<String>, Vec<(&'g str, &'g Graph)>);

/// The auth view's resources in `graph`, and its control documents ending in `suffix`.
/// The resources are every graph outside the reserved and skolem spaces that is not an
/// `.acl`/`.acr` and has an authority, plus every structural container above one (a container is an inheritance
/// anchor even without a graph of its own, but not when only control documents sit below
/// it). The materializer and the store's provenance pruning both use this, so trusted
/// creator/owner facts are stated for exactly the resources the view holds.
///
/// `meter`, when given, is the input budget of the materialization this enumeration feeds:
/// the resources it walks are charged against it ([`crate::AuthBudget`]) BEFORE their
/// container chains are built, so a pod cannot be made to spend unbounded memory here on
/// the way to a budget that only bounds what comes after. A caller with no budget to charge
/// (the store's own provenance pruning, over names it already holds) passes `None`.
fn enumerate<'g>(
    graph: &'g Graph,
    suffix: &str,
    mut meter: Option<&mut AuthLedger>,
) -> Result<Enumerated<'g>, String> {
    let mut resources: FxHashSet<String> = FxHashSet::default();
    let mut control_graphs: Vec<(&str, &Graph)> = Vec::new(); // (name, sub)
    for (batch, (name, sub)) in graph.named().iter().enumerate() {
        if batch % 64 == 0 {
            if let Some(meter) = meter.as_deref() {
                meter.checkpoint()?;
            }
        }
        let Some(iri) = graph_iri(name) else { continue };
        // Skip the whole reserved space (`<urn:sparq:auth>` included): a graph there is
        // never reasoning input. [`strip_reserved_graphs`] also drops these from the
        // dataset, but assembly must not DEPEND on that having run first — the
        // materializer defers every mutation until after this fallible pass so an error
        // leaves the previous auth view in place (roborev 5005). A graph named in the
        // skolem space is skipped too, so it is never a resource: no one is granted it.
        if iri.starts_with(RESERVED_PREFIX) || iri.starts_with(SKOLEM_PREFIX) {
            continue;
        }
        if is_control_graph(iri) {
            if iri.ends_with(suffix) && !is_authority_less(control_target(iri)) {
                if let Some(meter) = meter.as_deref_mut() {
                    meter.reserve(&mut control_graphs, 1, Dim::Nodes)?;
                }
                control_graphs.push((iri, sub));
            }
            continue;
        }
        if is_resource_graph(iri) {
            insert_with_containers(&mut resources, iri, meter.as_deref_mut())?;
        }
    }
    Ok((resources, control_graphs))
}

/// Whether the stored graph `iri` is an access-control document.
fn is_control_graph(iri: &str) -> bool {
    iri.ends_with(ACL_SUFFIX) || iri.ends_with(ACR_SUFFIX)
}

/// The resources whose own access control resource (`.acr`) is stored in `graph`: the only
/// document the ACP view, the one reader of creators and owners, reads.
pub(crate) fn controlled_resources(graph: &Graph) -> FxHashSet<&str> {
    graph
        .named()
        .iter()
        .filter_map(|(n, _)| graph_iri(n))
        .filter_map(|iri| iri.strip_suffix(ACR_SUFFIX))
        .collect()
}

/// The resource an access-control document `iri` governs.
fn control_target(iri: &str) -> &str {
    iri.strip_suffix(ACL_SUFFIX)
        .or_else(|| iri.strip_suffix(ACR_SUFFIX))
        .unwrap_or(iri)
}

/// Whether the stored graph `iri` is a resource of the auth view by itself: outside the
/// reserved and skolem spaces, not an access-control document, and below an authority. A bare
/// `scheme://` names no authority, so it is not a container of any pod root, and neither it
/// nor its control document takes part in the auth view. (The parent rule's textual scrape
/// of `https://pod.ex/` yields `https://`; [`parent_iri`] already stops at the authority
/// root.)
fn is_resource_graph(iri: &str) -> bool {
    !iri.starts_with(RESERVED_PREFIX)
        && !iri.starts_with(SKOLEM_PREFIX)
        && !is_control_graph(iri)
        && !is_authority_less(iri)
}

/// Whether `iri` can be a structural container: a `/`-terminated IRI at or below an
/// authority root (`scheme://host/…`), the only names [`parent_iri`] yields.
fn is_container_name(iri: &str) -> bool {
    iri.find("://")
        .is_some_and(|i| iri[i + 3..].contains('/') && !is_authority_less(iri))
}

/// Add `iri` and its structural container chain (https://host/a/b/doc -> /a/b/ -> /a/ -> /).
///
/// The chain is the whole cost of this: one owned prefix string per container, so an IRI of
/// length L under D containers owns up to L x D bytes here. `meter`, when given, is charged
/// for each of those strings BEFORE it is made, and refuses the IRI's length and depth
/// before the walk starts at all ([`AuthLedger::resource`]) — the one point where a single
/// name can cost more than its own length, and the reason the length and depth are bounded
/// rather than only the facts the names become.
///
/// A container already in `resources` ends the walk, as its own chain is there already;
/// nothing is allocated for it (nor charged), so the enumeration of a whole pod costs the
/// bytes of its DISTINCT resources, not of every chain.
fn insert_with_containers(
    resources: &mut FxHashSet<String>,
    iri: &str,
    mut meter: Option<&mut AuthLedger>,
) -> Result<(), String> {
    if let Some(m) = meter.as_deref_mut() {
        m.resource_name(iri)?;
    }
    if resources.contains(iri) {
        return Ok(());
    }
    if let Some(m) = meter.as_deref_mut() {
        m.resource(iri)?;
    }
    resources.insert(iri.to_owned());
    let mut cur = iri;
    while let Some(parent) = parent_iri(cur) {
        if resources.contains(parent) {
            break;
        }
        if let Some(m) = meter.as_deref_mut() {
            m.resource_bytes(parent.len())?;
        }
        resources.insert(parent.to_owned());
        cur = parent;
    }
    Ok(())
}

/// The stored graphs that are resources by themselves (see [`is_resource_graph`]), kept in
/// order so the resources they make can be tested one at a time: a graph is a resource, and
/// so is every container on its chain (see [`enumerate`]). Every lifecycle decision about
/// the stored provenance is made against this one rule.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceGraphs<'a>(std::collections::BTreeSet<&'a str>);

/// Whether each of `resources` is a resource of `graph` (as [`ResourceGraphs::holds`]), in one
/// pass over its graph names: for judging a few resources without enumerating them all.
pub(crate) fn held(graph: &Graph, resources: &[String]) -> Vec<bool> {
    let mut held = vec![false; resources.len()];
    if resources.is_empty() {
        return held;
    }
    for name in graph.named().iter().filter_map(|(n, _)| graph_iri(n)) {
        if !is_resource_graph(name) {
            continue;
        }
        for (r, h) in resources.iter().zip(held.iter_mut()) {
            if !*h
                && (name == r
                    || (r.ends_with('/') && is_container_name(r) && name.starts_with(r.as_str())))
            {
                *h = true;
            }
        }
    }
    held
}

impl<'a> ResourceGraphs<'a> {
    /// The resource graphs among the names of `graph`'s named graphs.
    pub(crate) fn of(graph: &'a Graph) -> Self {
        Self::from_names(graph.named().iter().filter_map(|(n, _)| graph_iri(n)))
    }

    /// The resource graphs among `names`.
    pub(crate) fn from_names(names: impl IntoIterator<Item = &'a str>) -> Self {
        ResourceGraphs(names.into_iter().filter(|n| is_resource_graph(n)).collect())
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

    /// Every resource, with its containers, charged against `meter`.
    ///
    /// The names are the store's own, but their container chains are built HERE, so this
    /// walk costs what the names say and not what the store already holds — the same
    /// product [`insert_with_containers`] describes. It is therefore metered on the same
    /// terms as a materialization's own enumeration: a name past
    /// [`crate::AuthBudget::max_resource_iri_bytes`] /
    /// [`crate::AuthBudget::max_resource_path_depth`], or an enumeration past the aggregate
    /// [`crate::AuthBudget::max_input_bytes`], refuses rather than being walked.
    #[cfg(test)]
    pub(crate) fn all(&self, meter: &mut AuthLedger) -> Result<FxHashSet<String>, String> {
        let mut out = FxHashSet::default();
        for name in &self.0 {
            insert_with_containers(&mut out, name, Some(meter))?;
        }
        Ok(out)
    }

    /// Add a graph, announcing borrowed names for resources newly brought into being.
    ///
    /// Only added names face the per-name length/depth caps, before their chain is walked.
    /// The replay owns no strings, so it spends no aggregate input-byte allowance.
    /// A refusal leaves the set unchanged and nothing announced.
    pub(crate) fn add(
        &mut self,
        name: &'a str,
        meter: &mut AuthLedger,
        mut appears: impl FnMut(&'a str),
    ) -> Result<(), String> {
        if !is_resource_graph(name) || self.0.contains(name) {
            return Ok(());
        }
        meter.resource_name(name)?;
        let mut cur = Some(name);
        while let Some(r) = cur {
            if self.holds(r) {
                break;
            }
            appears(r);
            cur = parent_iri(r);
        }
        self.0.insert(name);
        Ok(())
    }

    /// Remove a graph, announcing borrowed names for resources that cease to exist.
    ///
    /// Removals never refuse, including names beyond the current per-name caps. Each
    /// parent is a subslice of the removed name; no container strings are allocated.
    pub(crate) fn remove(&mut self, name: &'a str, mut gone: impl FnMut(&'a str)) {
        if !self.0.remove(name) {
            return;
        }
        let mut cur = Some(name);
        while let Some(r) = cur {
            if self.holds(r) {
                break;
            }
            gone(r);
            cur = parent_iri(r);
        }
    }

    /// Remove all graphs, announcing each resource once through borrowed names.
    ///
    /// Like individual removals, clears never refuse or charge an input allowance.
    pub(crate) fn clear(&mut self, mut gone: impl FnMut(&'a str)) {
        let mut announced = FxHashSet::default();
        for &name in &self.0 {
            let mut cur = Some(name);
            while let Some(r) = cur {
                if !announced.insert(r) {
                    break;
                }
                gone(r);
                cur = parent_iri(r);
            }
        }
        self.0.clear();
    }
}

/// Whether an access-control resource document in `graph` names `acp:CreatorAgent` or
/// `acp:OwnerAgent`: whether an ACP view built from it depends on the trusted provenance.
pub(crate) fn control_documents_name_provenance(graph: &Graph) -> bool {
    graph.named().iter().any(|(name, g)| {
        graph_iri(name).is_some_and(|iri| iri.ends_with(ACR_SUFFIX))
            && graph_triples(g).iter().any(|t| {
                matches!(&t[2], Term::NamedNode(o) if SPECIAL_AGENTS[2..4].contains(&o.as_str()))
            })
    })
}

/// The auth view's resources in `graph` (see [`enumerate`]), charged against `meter`.
///
/// This is the store's own provenance pruning, over names it already holds — but it BUILDS
/// their container chains, so it costs what those names say rather than what is already
/// held, and it is metered on exactly the terms a materialization's own enumeration is. A
/// name past the per-name caps, or an enumeration past the aggregate, refuses: the
/// authorization view is not rebuilt (the caller propagates the refusal before anything is
/// installed), rather than the walk being done unbounded.
pub(crate) fn resources(
    graph: &Graph,
    meter: &mut AuthLedger,
) -> Result<FxHashSet<String>, String> {
    Ok(enumerate(graph, ACR_SUFFIX, Some(meter))?.0)
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
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn require_writable_refuses_a_frozen_graph() {
        let mut graph = sparq_core::Graph::new();
        assert!(super::require_writable(&graph).is_ok());
        graph.freeze_for_test("storage unavailable");
        assert!(super::require_writable(&graph).is_err());
    }

    use super::{assemble_facts_default, graph_triples, named, skolemize, System};
    use super::{iri_origin, parent_iri};
    use crate::{
        AccessProvenance, Mode, PodStore, Session, VerifiedCredentials, AUTH_GRAPH, AUTH_NS,
    };
    use oxrdf::{BlankNode, Literal, NamedNode, Term};
    use rustc_hash::FxHashSet;
    use sparq_core::Graph;

    const ACR: &str = "https://pod.ex/doc.acr";
    const OTHER_ACR: &str = "https://other.ex/doc.acr";

    /// A fresh input meter over the default budget, for the enumeration helpers.
    fn meter() -> crate::budget::AuthLedger {
        crate::budget::AuthLedger::new(crate::default_auth_budget())
    }

    /// A resource IRI under `depth` containers, inside the LENGTH cap so the depth cap is
    /// the one that refuses it.
    fn deep(depth: usize) -> String {
        format!("https://pod.ex/{}doc", "a/".repeat(depth))
    }

    /// [`ResourceGraphs::add`] charges the name it is given: past the depth cap it refuses
    /// before the chain is walked, and announces nothing — the caller's `appears` is what
    /// would own each container.
    #[test]
    fn resource_graphs_add_refuses_a_name_past_the_caps_announcing_nothing() {
        let mut graphs = super::ResourceGraphs::default();
        let mut announced: Vec<String> = Vec::new();
        let name = deep(crate::default_auth_budget().max_resource_path_depth + 4);
        let err = graphs
            .add(&name, &mut meter(), |r| announced.push(r.to_owned()))
            .expect_err("a name past the depth cap is refused");
        assert_eq!(
            crate::auth_allowance_refusal(&err),
            Some(crate::AuthAllowance::ResourceIri),
            "the typed enumeration refusal: {}",
            err
        );
        assert!(
            announced.is_empty(),
            "nothing is announced, so nothing owns the chain: {:?}",
            announced
        );
        // A name inside the caps still goes in, with its containers announced.
        let ok = "https://pod.ex/a/b/doc";
        graphs
            .add(ok, &mut meter(), |r| announced.push(r.to_owned()))
            .expect("an ordinary name is admitted");
        assert!(graphs.holds(ok));
        assert!(announced.iter().any(|r| r == "https://pod.ex/a/"));
    }

    /// [`ResourceGraphs::all`] and [`resources`] walk the SAME chains and are charged the
    /// same way: a held name past the caps refuses the enumeration rather than being walked.
    #[test]
    fn the_store_side_enumerations_refuse_a_held_name_past_the_caps() {
        let name = deep(crate::default_auth_budget().max_resource_path_depth + 4);
        let graphs = super::ResourceGraphs::from_names([name.as_str()]);
        let err = graphs
            .all(&mut meter())
            .expect_err("a held name past the caps is refused");
        assert_eq!(
            crate::auth_allowance_refusal(&err),
            Some(crate::AuthAllowance::ResourceIri),
            "the typed enumeration refusal: {}",
            err
        );

        let nq = format!("<{name}#it> <https://ex.dev/ns#k> \"v\" <{name}> .\n");
        let graph = Graph::load_dataset(&nq, "nquads").expect("parses");
        let err = super::resources(&graph, &mut meter())
            .expect_err("the provenance pruning is refused on the same terms");
        assert_eq!(
            crate::auth_allowance_refusal(&err),
            Some(crate::AuthAllowance::ResourceIri),
            "the typed enumeration refusal: {}",
            err
        );
    }

    /// Removal still works after the caller lowers its per-name caps.
    #[test]
    fn resource_graphs_remove_borrows_a_name_past_the_caps() {
        let name = deep(crate::default_auth_budget().max_resource_path_depth + 4);
        let mut graphs = super::ResourceGraphs::from_names([name.as_str()]);
        let mut ended = Vec::new();
        graphs.remove(&name, |r| ended.push(r));
        assert!(!graphs.holds(&name));
        assert_eq!(ended.first(), Some(&name.as_str()));
        assert_eq!(ended.last(), Some(&"https://pod.ex/"));
        assert!(ended.iter().all(|r| name.as_ptr() == r.as_ptr()));
    }

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
            graph.add_named(named(iri), sub).unwrap();
        }
        let facts = |g: &Graph| -> FxHashSet<_> {
            assemble_facts_default(
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
        let mut reversed = graph.take_named().unwrap();
        reversed.reverse();
        graph.extend_named(reversed).unwrap();
        assert_eq!(facts(&graph), before);
    }

    fn blank_matcher_store() -> PodStore {
        let mut graph = Graph::new();
        graph.add_named(named(OTHER_ACR), Graph::new()).unwrap();
        graph.add_named(named("https://pod.ex/doc"), Graph::new()).unwrap();
        graph
            .add_named(
                named(ACR),
                Graph::load_str(
                    include_str!("../tests/fixtures/blank_node_matcher.ttl"),
                    "turtle",
                )
                .unwrap(),
            )
            .unwrap();
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
        assemble_facts_default(
            store.graph(),
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
                .graph()
                .named()
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
                    .named()
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

    /// Document triples in the view vocabulary (an `auth:` predicate, or an `rdf:type`
    /// naming an `auth:`/`solidx:` class) never reach the reasoner, for either system.
    /// Each statement is about the document's own node, so only the vocabulary check
    /// rejects it, and each is tried on its own.
    #[test]
    fn view_vocabulary_in_documents_is_dropped() {
        let target = "https://pod.ex/secret";
        let rdf_type = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
        let docs = [
            (System::Wac, "https://other.ex/.acl"),
            (System::Acp, "https://other.ex/.acr"),
        ];
        for (system, doc) in docs {
            let me = format!("{doc}#me");
            let statements = [
                format!("<{me}> <{AUTH_NS}read> <{target}>"),
                format!("<{me}> <{AUTH_NS}write> <{target}>"),
                format!("<{doc}#g> {rdf_type} <{AUTH_NS}ConditionalGrant>"),
                format!("<{doc}#g> {rdf_type} <{solidx}Thing>", solidx = crate::SOLIDX_NS),
            ];
            for statement in &statements {
                let nq = format!(
                    "<https://pod.ex/secret#it> <https://ex.dev/ns#title> \"t\" <{target}> .\n\
                     <https://other.ex/doc#it> <https://ex.dev/ns#title> \"t\" \
                     <https://other.ex/doc> .\n\
                     {statement} <{doc}> .\n"
                );
                let graph = Graph::load_dataset(&nq, "nquads").unwrap();
                let (prov, creds) = (AccessProvenance::new(), VerifiedCredentials::new());
                let facts = assemble_facts_default(&graph, system, &prov, &creds).unwrap();
                let own = |x: &Term| matches!(x, Term::NamedNode(n) if n.as_str().starts_with(doc));
                let leaked: Vec<_> = facts.iter().filter(|t| own(&t[0])).collect();
                assert!(leaked.is_empty(), "{statement}: {leaked:?}");

                let mut store = PodStore::new(graph);
                match system {
                    System::Wac => store.materialize_wac().unwrap(),
                    System::Acp => store.materialize_acp().unwrap(),
                };
                let session = Session {
                    agent: Some(&me),
                    client: None,
                    issuer: None,
                    now: None,
                };
                for mode in [Mode::Read, Mode::Write] {
                    assert!(store.accessible(&session, mode).is_empty(), "{statement}: {mode:?}");
                }
            }
        }
    }

    /// A document's statements about a node defined in another document are dropped;
    /// references to nodes elsewhere still work. Each case adds one statement to a
    /// document (`doc`), through the initial load, an authoritative control-document
    /// replace and a session update the session is authorized to make. The agent behind
    /// the statement never gains access to `pod.ex/secret`, while the legitimate
    /// cross-document reference keeps granting carol.
    #[test]
    fn documents_describe_only_their_own_nodes() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        const ACP: &str = "http://www.w3.org/ns/solid/acp#";
        const VCARD: &str = "http://www.w3.org/2006/vcard/ns#";
        use super::RDF_TYPE;
        let (alice, carol) = ("https://alice.ex/me", "https://carol.ex/me");
        let secret = "https://pod.ex/secret";
        let (oacl, oacr, grp, ogrp) = (
            "https://other.ex/doc.acl",
            "https://other.ex/doc.acr",
            "https://pod.ex/groups",
            "https://other.ex/grp",
        );
        // The other agent's WebID lives in the document it edits, so a statement with
        // that subject is about one of the document's own nodes.
        let (wother, aother) = (format!("{oacl}#me"), format!("{oacr}#me"));
        let data = format!(
            "<{secret}#it> <https://ex.dev/ns#t> \"t\" <{secret}> .\n\
             <https://other.ex/doc#it> <https://ex.dev/ns#t> \"t\" <https://other.ex/doc> .\n"
        );
        let wac = format!(
            "{data}\
             <{secret}.acl#owner> <{RDF_TYPE}> <{ACL}Authorization> <{secret}.acl> .\n\
             <{secret}.acl#owner> <{ACL}accessTo> <{secret}> <{secret}.acl> .\n\
             <{secret}.acl#owner> <{ACL}agent> <{alice}> <{secret}.acl> .\n\
             <{secret}.acl#owner> <{ACL}mode> <{ACL}Read> <{secret}.acl> .\n\
             <{secret}.acl#team> <{RDF_TYPE}> <{ACL}Authorization> <{secret}.acl> .\n\
             <{secret}.acl#team> <{ACL}accessTo> <{secret}> <{secret}.acl> .\n\
             <{secret}.acl#team> <{ACL}agentGroup> <{grp}#team> <{secret}.acl> .\n\
             <{secret}.acl#team> <{ACL}mode> <{ACL}Read> <{secret}.acl> .\n\
             <{grp}#team> <{VCARD}hasMember> <{carol}> <{grp}> .\n\
             <{oacl}#a> <{RDF_TYPE}> <{ACL}Authorization> <{oacl}> .\n\
             <{oacl}#a> <{ACL}accessTo> <https://other.ex/doc> <{oacl}> .\n\
             <{oacl}#a> <{ACL}agent> <{wother}> <{oacl}> .\n\
             <{oacl}#a> <{ACL}mode> <{ACL}Control> <{oacl}> .\n\
             <{oacl}#a> <{ACL}agentGroup> <{ogrp}#g> <{oacl}> .\n\
             <{ogrp}#g> <{VCARD}hasMember> <{wother}> <{ogrp}> .\n\
             <https://other.ex/.acl#o> <{RDF_TYPE}> <{ACL}Authorization> <https://other.ex/.acl> .\n\
             <https://other.ex/.acl#o> <{ACL}accessTo> <https://other.ex/> <https://other.ex/.acl> .\n\
             <https://other.ex/.acl#o> <{ACL}default> <https://other.ex/> <https://other.ex/.acl> .\n\
             <https://other.ex/.acl#o> <{ACL}agent> <{wother}> <https://other.ex/.acl> .\n\
             <https://other.ex/.acl#o> <{ACL}mode> <{ACL}Write> <https://other.ex/.acl> .\n"
        );
        let shared = "https://shared.ex/p.acr";
        let meta = format!("{oacr}.acr");
        let acp = format!(
            "{data}\
             <{secret}.acr> <{ACP}accessControl> <{secret}.acr#c> <{secret}.acr> .\n\
             <{secret}.acr#c> <{ACP}apply> <{shared}#pol> <{secret}.acr> .\n\
             <{shared}#pol> <{ACP}allow> <{ACL}Read> <{shared}> .\n\
             <{shared}#pol> <{ACP}anyOf> <{shared}#m> <{shared}> .\n\
             <{shared}#m> <{ACP}agent> <{carol}> <{shared}> .\n\
             <{oacr}#c> <{ACP}apply> <{oacr}#p> <{oacr}> .\n\
             <{oacr}#p> <{ACP}allow> <{ACL}Read> <{oacr}> .\n\
             <{oacr}#p> <{ACP}anyOf> <{oacr}#m> <{oacr}> .\n\
             <{oacr}#m> <{ACP}agent> <{aother}> <{oacr}> .\n\
             <{meta}> <{ACP}accessControl> <{meta}#c> <{meta}> .\n\
             <{meta}#c> <{ACP}apply> <{meta}#p> <{meta}> .\n\
             <{meta}#p> <{ACP}allow> <{ACL}Write> <{meta}> .\n\
             <{meta}#p> <{ACP}anyOf> <{meta}#m> <{meta}> .\n\
             <{meta}#m> <{ACP}agent> <{aother}> <{meta}> .\n"
        );
        let auth_read = format!("{AUTH_NS}read");
        // (system, base, other agent, document, added triple)
        let cases = [
            (
                System::Wac,
                &wac,
                &wother,
                oacl,
                format!("<{wother}> <{auth_read}> <{secret}>"),
            ),
            (
                System::Acp,
                &acp,
                &aother,
                oacr,
                format!("<{aother}> <{auth_read}> <{secret}>"),
            ),
            (
                System::Wac,
                &wac,
                &wother,
                oacl,
                format!("<{secret}.acl#owner> <{ACL}agent> <{wother}>"),
            ),
            (
                System::Wac,
                &wac,
                &wother,
                ogrp,
                format!("<{grp}#team> <{VCARD}hasMember> <{wother}>"),
            ),
            (
                System::Acp,
                &acp,
                &aother,
                oacr,
                format!("<{secret}.acr> <{ACP}accessControl> <{oacr}#c>"),
            ),
            (
                System::Acp,
                &acp,
                &aother,
                oacr,
                format!("<{shared}#m> <{ACP}agent> <{aother}>"),
            ),
        ];
        let check = |store: &PodStore, system: System, other: &str, how: &str| {
            let can_read = |agent: &str| {
                let s = Session {
                    agent: Some(agent),
                    client: None,
                    issuer: None,
                    now: None,
                };
                store
                    .accessible(&s, Mode::Read)
                    .iter()
                    .any(|r| r.as_str() == secret)
            };
            assert!(
                !can_read(other),
                "{system:?} {how}: the added statement granted access"
            );
            assert!(
                can_read(carol),
                "{system:?} {how}: a cross-document reference stopped working"
            );
        };
        for (system, base, other, doc, triple) in cases {
            let acp = system == System::Acp;
            let fresh = || {
                let mut store = PodStore::new(Graph::load_dataset(base, "nquads").unwrap());
                if acp {
                    store.materialize_acp()
                } else {
                    store.materialize_wac()
                }
                .unwrap();
                store
            };

            let nq = format!("{base}{triple} <{doc}> .\n");
            let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
            if acp {
                store.materialize_acp()
            } else {
                store.materialize_wac()
            }
            .unwrap();
            check(&store, system, other, &format!("load {triple}"));

            if doc.ends_with(".acl") || doc.ends_with(".acr") {
                let suffix = format!(" <{doc}> .");
                let mut body: String = base
                    .lines()
                    .filter_map(|l| l.strip_suffix(suffix.as_str()))
                    .map(|l| format!("{l} .\n"))
                    .collect();
                body.push_str(&format!("{triple} .\n"));
                let mut store = fresh();
                let put = if acp {
                    store.put_acl_acp(doc, &body, "ntriples")
                } else {
                    store.put_acl(doc, &body, "ntriples")
                };
                assert!(
                    put.as_ref().is_err_and(|e| e.contains("may not state")),
                    "{system:?}: the write is refused: {put:?}"
                );
                check(&store, system, other, &format!("put {triple}"));
            }

            let mut store = fresh();
            let session = Session {
                agent: Some(other),
                client: None,
                issuer: None,
                now: None,
            };
            let insert = format!("INSERT DATA {{ GRAPH <{doc}> {{ {triple} }} }}");
            let updated = if acp {
                store.update_as_acp(&session, &insert)
            } else {
                store.update_as(&session, &insert)
            };
            if doc.ends_with(".acl") || doc.ends_with(".acr") {
                let e =
                    updated.expect_err("a control-document statement it may not make is refused");
                assert!(e.contains("may not state"), "{system:?}: {e}");
            } else {
                updated.unwrap_or_else(|e| panic!("{system:?}: the update is authorized: {e}"));
            }
            check(&store, system, other, &format!("update {triple}"));
        }
    }

    /// A rejected control document lists no group members either, so it grants nothing
    /// through an accepted ACL that names a group in it.
    #[test]
    fn a_rejected_control_document_is_no_group_document() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        const VCARD: &str = "http://www.w3.org/2006/vcard/ns#";
        use super::RDF_TYPE;
        let (root, x, bob) = (
            "https://pod.ex/.acl",
            "https://pod.ex/x.acl",
            "https://bob.ex/me",
        );
        let base = format!(
            "<https://pod.ex/n#it> <https://ex.dev/ns#t> \"t\" <https://pod.ex/n> .\n\
             <{root}#r> <{RDF_TYPE}> <{ACL}Authorization> <{root}> .\n\
             <{root}#r> <{ACL}default> <https://pod.ex/> <{root}> .\n\
             <{root}#r> <{ACL}accessTo> <https://pod.ex/> <{root}> .\n\
             <{root}#r> <{ACL}mode> <{ACL}Read> <{root}> .\n\
             <{root}#r> <{ACL}agentGroup> <{x}#g> <{root}> .\n\
             <{x}#g> <{VCARD}hasMember> <{bob}> <{x}> .\n"
        );
        let reads = |nq: &str| {
            let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
            s.materialize_wac().unwrap();
            let bob = Session {
                agent: Some(bob),
                client: None,
                issuer: None,
                now: None,
            };
            s.accessible(&bob, Mode::Read)
                .iter()
                .any(|g| g.as_str() == "https://pod.ex/n")
        };
        assert!(reads(&base), "control: the group grants bob Read");
        let stray = format!("<https://other.ex/n> <https://ex.dev/ns#p> \"o\" <{x}> .\n");
        assert!(
            !reads(&format!("{base}{stray}")),
            "the rejected document lists no members"
        );
    }

    /// Loaded data cannot name the IRI a document's blank node is skolemized to: a group
    /// document stored under that IRI adds no agent to the blank-node authorization.
    #[test]
    fn documents_cannot_name_skolemized_nodes() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        use super::RDF_TYPE;
        let (alice, other) = ("https://alice.ex/me", "https://other.ex/card#me");
        let (secret, acl) = ("https://pod.ex/secret", "https://pod.ex/secret.acl");
        let base = format!(
            "<{secret}#it> <https://ex.dev/ns#t> \"t\" <{secret}> .\n\
             _:b <{RDF_TYPE}> <{ACL}Authorization> <{acl}> .\n\
             _:b <{ACL}accessTo> <{secret}> <{acl}> .\n\
             _:b <{ACL}agent> <{alice}> <{acl}> .\n\
             _:b <{ACL}mode> <{ACL}Read> <{acl}> .\n"
        );
        let graph = Graph::load_dataset(&base, "nquads").unwrap();
        let blank = graph
            .named()
            .iter()
            .filter(|(name, _)| matches!(name, Term::NamedNode(n) if n.as_str() == acl))
            .flat_map(|(_, sub)| graph_triples(sub))
            .find_map(|t| matches!(t[0], Term::BlankNode(_)).then(|| t[0].clone()))
            .expect("the authorization is a blank node");
        let Term::NamedNode(minted) = skolemize(&blank, acl) else {
            unreachable!()
        };
        let minted = minted.as_str();
        let nq = format!(
            "{base}\
             <https://other.ex/doc.acl#a> <{ACL}agentGroup> <{minted}> <https://other.ex/doc.acl> .\n\
             <{minted}> <{ACL}agent> <{other}> <{minted}> .\n"
        );
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
        store.materialize_wac().unwrap();
        let can_read = |agent: &str| {
            let s = Session {
                agent: Some(agent),
                client: None,
                issuer: None,
                now: None,
            };
            store
                .accessible(&s, Mode::Read)
                .iter()
                .any(|r| r.as_str() == secret)
        };
        assert!(
            !can_read(other),
            "a stored skolem IRI extended the blank-node authorization"
        );
        assert!(
            can_read(alice),
            "the blank-node authorization itself still applies"
        );
    }

    /// A minted IRI is recognised wherever a triple names it: in any position, as a datatype
    /// or inside a triple term, not only as a top-level node.
    #[test]
    fn minted_names_are_found_in_every_position() {
        use super::names_minted_iri;
        use oxrdf::Triple;
        let minted = NamedNode::new_unchecked("urn:skolem:graph:x:b");
        let plain = NamedNode::new_unchecked("https://pod.ex/doc#a");
        let typed = Term::Literal(Literal::new_typed_literal("v", minted.clone()));
        let nested = Term::Triple(Box::new(Triple::new(
            minted.clone(),
            plain.clone(),
            Term::from(plain.clone()),
        )));
        let deep = Term::Triple(Box::new(Triple::new(plain.clone(), plain.clone(), typed.clone())));
        for (object, names) in [(typed, true), (nested, true), (deep, true), (Term::from(plain.clone()), false)] {
            let t = [Term::from(plain.clone()), Term::from(plain.clone()), object];
            assert_eq!(names_minted_iri(&t), names, "{t:?}");
        }
        let reserved = Term::from(NamedNode::new_unchecked("urn:sparq:matcher?m=a&res=b"));
        let p = |iri: &str| Term::from(NamedNode::new_unchecked(iri));
        let any_of = p("http://www.w3.org/ns/solid/acp#anyOf");
        let agent = p(super::ACP_AGENT);
        let doc = Term::from(plain.clone());
        assert!(names_minted_iri(&[reserved.clone(), agent.clone(), doc.clone()]), "subject");
        assert!(names_minted_iri(&[doc.clone(), reserved.clone(), doc.clone()]), "predicate");
        assert!(names_minted_iri(&[doc.clone(), any_of, reserved.clone()]), "object");
        assert!(names_minted_iri(&[doc.clone(), agent.clone(), reserved]), "principal");
        let minted_type = NamedNode::new_unchecked("urn:sparq:x");
        let typed_principal = Term::Literal(Literal::new_typed_literal("v", minted_type));
        assert!(names_minted_iri(&[doc, agent, typed_principal]), "datatype of a principal");
    }

    /// A rejected document never leaves an allow standing without the restrictions it
    /// carried. ACP: a resource whose ACR is rejected, or whose ACR applies a policy from
    /// a rejected document, gets no grants, and no ancestor's member policy takes over.
    /// WAC: a rejected ACL still governs its resource, so no ancestor default applies.
    #[test]
    fn rejected_documents_fail_closed() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        const ACP: &str = "http://www.w3.org/ns/solid/acp#";
        use super::RDF_TYPE;
        let (alice, mallory) = ("https://alice.ex/me", "https://mallory.ex/me");
        let secret = "https://pod.ex/c/secret";
        let data = format!("<{secret}#it> <https://ex.dev/ns#t> \"t\" <{secret}> .\n");
        let reads = |nq: &str, system: System, agent: &str| {
            let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
            match system {
                System::Wac => store.materialize_wac().unwrap(),
                System::Acp => store.materialize_acp().unwrap(),
            };
            let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
            store.accessible(&s, Mode::Read).iter().any(|r| r.as_str() == secret)
        };
        let stray = "<https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\"";

        // ACP: the root grants everyone Read on members; secret's ACR denies mallory.
        let root = "https://pod.ex/.acr";
        let acr = format!("{secret}.acr");
        let shared = "https://shared.ex/p.acr";
        let acp_root = format!(
            "{data}\
             <{root}> <{ACP}memberAccessControl> <{root}#c> <{root}> .\n\
             <{root}#c> <{ACP}apply> <{root}#all> <{root}> .\n\
             <{root}#all> <{ACP}allow> <{ACL}Read> <{root}> .\n\
             <{root}#all> <{ACP}anyOf> <{root}#pub> <{root}> .\n\
             <{root}#pub> <{ACP}agent> <{ACP}PublicAgent> <{root}> .\n"
        );
        let own_deny = format!(
            "<{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{acr}#deny> <{acr}> .\n\
             <{acr}#deny> <{ACP}deny> <{ACL}Read> <{acr}> .\n\
             <{acr}#deny> <{ACP}allOf> <{acr}#m> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{mallory}> <{acr}> .\n"
        );
        let base = format!("{acp_root}{own_deny}");
        assert!(reads(&base, System::Acp, alice), "control: alice reads");
        assert!(!reads(&base, System::Acp, mallory), "control: the deny applies");
        let nq = format!("{base}{stray} <{acr}> .\n");
        assert!(!reads(&nq, System::Acp, mallory), "ACP: a rejected ACR's deny is not lost");
        assert!(!reads(&nq, System::Acp, alice), "ACP: a rejected ACR grants nothing");
        // The ACR defines a deny under another document's name: the definition cannot
        // count, and neither can the rest of the ACR.
        let inline = format!(
            "{acp_root}\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{shared}#deny> <{acr}> .\n\
             <{shared}#deny> <{ACP}deny> <{ACL}Read> <{acr}> .\n\
             <{shared}#deny> <{ACP}allOf> <{acr}#m> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{mallory}> <{acr}> .\n"
        );
        assert!(!reads(&inline, System::Acp, mallory), "ACP: an inline foreign deny is not lost");
        let via_shared = format!(
            "{acp_root}\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{shared}#deny> <{acr}> .\n\
             <{shared}#deny> <{ACP}deny> <{ACL}Read> <{shared}> .\n\
             <{shared}#deny> <{ACP}allOf> <{shared}#m> <{shared}> .\n\
             <{shared}#m> <{ACP}agent> <{mallory}> <{shared}> .\n"
        );
        assert!(!reads(&via_shared, System::Acp, mallory), "control: the shared deny applies");
        let nq = format!("{via_shared}{stray} <{shared}> .\n");
        assert!(!reads(&nq, System::Acp, mallory), "ACP: a rejected shared deny is not lost");
        assert!(!reads(&nq, System::Acp, alice), "ACP: its referrer grants nothing");

        // WAC: the root ACL's default grants everyone Read; secret's own ACL grants alice.
        let wac = format!(
            "{data}\
             <https://pod.ex/.acl#all> <{RDF_TYPE}> <{ACL}Authorization> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#all> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#all> <{ACL}agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#all> <{ACL}mode> <{ACL}Read> <https://pod.ex/.acl> .\n\
             <{secret}.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}accessTo> <{secret}> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}agent> <{alice}> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}mode> <{ACL}Read> <{secret}.acl> .\n"
        );
        assert!(reads(&wac, System::Wac, alice), "control: alice reads");
        assert!(!reads(&wac, System::Wac, mallory), "control: the own ACL governs");
        let nq = format!("{wac}{stray} <{secret}.acl> .\n");
        assert!(!reads(&nq, System::Wac, mallory), "WAC: no ancestor default takes over");
        assert!(!reads(&nq, System::Wac, alice), "WAC: a rejected ACL grants nothing");

        // ACP: a rejected container ACR held a member deny; a member's own ACR allows.
        let c_acr = "https://pod.ex/c/.acr";
        let member = format!(
            "{data}\
             <{c_acr}> <{ACP}memberAccessControl> <{c_acr}#c> <{c_acr}> .\n\
             <{c_acr}#c> <{ACP}apply> <{c_acr}#deny> <{c_acr}> .\n\
             <{c_acr}#deny> <{ACP}deny> <{ACL}Read> <{c_acr}> .\n\
             <{c_acr}#deny> <{ACP}allOf> <{c_acr}#m> <{c_acr}> .\n\
             <{c_acr}#m> <{ACP}agent> <{mallory}> <{c_acr}> .\n\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{acr}#all> <{acr}> .\n\
             <{acr}#all> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
             <{acr}#all> <{ACP}anyOf> <{acr}#pub> <{acr}> .\n\
             <{acr}#pub> <{ACP}agent> <{ACP}PublicAgent> <{acr}> .\n"
        );
        assert!(reads(&member, System::Acp, alice), "control: the member's ACR allows");
        assert!(!reads(&member, System::Acp, mallory), "control: the container's deny applies");
        let nq = format!("{member}{stray} <{c_acr}> .\n");
        assert!(!reads(&nq, System::Acp, mallory), "ACP: a member of a rejected container gets nothing");

        // ACP: the rejection cascades along a chain of references.
        let (b, c) = ("https://shared.ex/b.acr", "https://shared.ex/c.acr");
        let chain = format!(
            "{acp_root}\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{b}#deny> <{acr}> .\n\
             <{b}#deny> <{ACP}deny> <{ACL}Read> <{b}> .\n\
             <{b}#deny> <{ACP}allOf> <{c}#m> <{b}> .\n\
             <{c}#m> <{ACP}agent> <{mallory}> <{c}> .\n"
        );
        assert!(!reads(&chain, System::Acp, mallory), "control: the chained deny applies");
        let nq = format!("{chain}{stray} <{c}> .\n");
        assert!(!reads(&nq, System::Acp, mallory), "ACP: a deny two references away is not lost");
        assert!(!reads(&nq, System::Acp, alice), "ACP: the chain's head grants nothing");

        // ACP: a cycle of references through a rejected document rejects the whole cycle.
        let cycle = format!(
            "{acp_root}\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{b}#deny> <{acr}> .\n\
             <{b}#deny> <{ACP}deny> <{ACL}Read> <{b}> .\n\
             <{b}#deny> <{ACP}allOf> <{c}#m> <{b}> .\n\
             <{c}#m> <{ACP}agent> <{mallory}> <{c}> .\n\
             <{c}#c> <{ACP}apply> <{b}#deny> <{c}> .\n"
        );
        assert!(!reads(&cycle, System::Acp, mallory), "control: the deny in a cycle applies");
        let nq = format!("{cycle}{stray} <{b}> .\n");
        assert!(!reads(&nq, System::Acp, mallory), "ACP: a deny in a rejected cycle is not lost");

        // ACP: naming an agent whose IRI lives in a rejected document is not a dependency.
        let named_agent = "https://pod.ex/mal.acr#me";
        let names = format!(
            "{data}\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
             <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{alice}> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{named_agent}> <{acr}> .\n\
             {stray} <https://pod.ex/mal.acr> .\n"
        );
        assert!(reads(&names, System::Acp, alice), "ACP: a named principal's document is not a dependency");
    }

    /// A group document's triple about a node it does not define is dropped on its own:
    /// the group's members still count, and the triple adds no member to another group.
    #[test]
    fn group_documents_drop_only_foreign_statements() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        use super::{RDF_TYPE, VCARD_MEMBER};
        let (carol, mallory) = ("https://carol.ex/me", "https://mallory.ex/me");
        let (secret, team, other) = (
            "https://pod.ex/secret",
            "https://pod.ex/team",
            "https://pod.ex/other-team",
        );
        let nq = format!(
            "<{secret}#it> <https://ex.dev/ns#t> \"t\" <{secret}> .\n\
             <{secret}.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}accessTo> <{secret}> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}agentGroup> <{team}#g> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}agentGroup> <{other}#g> <{secret}.acl> .\n\
             <{secret}.acl#a> <{ACL}mode> <{ACL}Read> <{secret}.acl> .\n\
             <{team}#g> <{VCARD_MEMBER}> <{carol}> <{team}> .\n\
             <{carol}> <http://www.w3.org/2006/vcard/ns#fn> \"Carol\" <{team}> .\n\
             <{other}#g> <{VCARD_MEMBER}> <{mallory}> <{team}> .\n"
        );
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
        store.materialize_wac().unwrap();
        let reads = |agent: &str| {
            let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
            store.accessible(&s, Mode::Read).iter().any(|r| r.as_str() == secret)
        };
        assert!(reads(carol), "a member described in the group document still reads");
        assert!(!reads(mallory), "a membership stated for another group adds no member");
    }

    /// An access-control document naming an identity the rules mint (a matcher instance, a
    /// candidate) is rejected, so no input can attach facts to one or point a policy at it.
    #[test]
    fn documents_naming_minted_identities_are_rejected() {
        const ACP: &str = "http://www.w3.org/ns/solid/acp#";
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        let (alice, doc) = ("https://alice.ex/me", "https://pod.ex/n1");
        let acr = format!("{doc}.acr");
        let base = format!(
            "<{doc}#it> <https://ex.dev/ns#t> \"t\" <{doc}> .\n\
             <{acr}> <{ACP}accessControl> <{acr}#c> <{acr}> .\n\
             <{acr}#c> <{ACP}apply> <{acr}#p> <{acr}> .\n\
             <{acr}#p> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
             <{acr}#p> <{ACP}anyOf> <{acr}#m> <{acr}> .\n\
             <{acr}#m> <{ACP}agent> <{alice}> <{acr}> .\n"
        );
        let reads = |nq: &str| {
            let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
            store.materialize_acp().unwrap();
            let s = Session { agent: Some(alice), client: None, issuer: None, now: None };
            !store.accessible(&s, Mode::Read).is_empty()
        };
        assert!(reads(&base), "control: alice reads");
        let instance = "urn:sparq:matcher?m=x&res=y";
        for extra in [
            format!("<{acr}#p> <{ACP}anyOf> <{instance}> <{acr}> .\n"),
            format!("<{acr}#m> <{ACP}vc> <{instance}> <{acr}> .\n"),
            format!("<{acr}#m> <https://ex.dev/ns#n> \"v\"^^<{instance}> <{acr}> .\n"),
        ] {
            assert!(!reads(&format!("{base}{extra}")), "rejected: {extra}");
        }
    }

    /// A control document naming a skolem IRI only as a literal's datatype is rejected
    /// like any other, so the grants it holds do not apply.
    #[test]
    fn a_skolem_datatype_rejects_the_document() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        use super::RDF_TYPE;
        let (alice, doc) = ("https://alice.ex/me", "https://pod.ex/n1");
        let base = format!(
            "<{doc}#it> <https://ex.dev/ns#t> \"t\" <{doc}> .\n\
             <{doc}.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <{doc}.acl> .\n\
             <{doc}.acl#a> <{ACL}accessTo> <{doc}> <{doc}.acl> .\n\
             <{doc}.acl#a> <{ACL}agent> <{alice}> <{doc}.acl> .\n\
             <{doc}.acl#a> <{ACL}mode> <{ACL}Read> <{doc}.acl> .\n"
        );
        let reads = |nq: &str| {
            let mut store = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
            store.materialize_wac().unwrap();
            let s = Session { agent: Some(alice), client: None, issuer: None, now: None };
            !store.accessible(&s, Mode::Read).is_empty()
        };
        assert!(reads(&base), "control: alice reads");
        let typed = format!(
            "{base}<{doc}.acl#a> <https://ex.dev/ns#note> \"v\"^^<urn:skolem:graph:x:b> <{doc}.acl> .\n"
        );
        assert!(!reads(&typed), "a skolem datatype rejects the document");
        let nested = format!(
            "{base}<{doc}.acl#a> <https://ex.dev/ns#said>              <<( <{doc}.acl#a> <https://ex.dev/ns#p> <urn:sparq:x> )>> <{doc}.acl> .\n"
        );
        assert!(!reads(&nested), "a reserved name inside a triple term rejects the document");
    }

    /// The view's own principal names are not principals a document or a session can
    /// supply: a document naming one is rejected, and a session claiming one gets nothing.
    #[test]
    fn internal_names_are_not_principals() {
        const ACL: &str = "http://www.w3.org/ns/auth/acl#";
        use super::RDF_TYPE;
        let doc = "https://pod.ex/n1";
        let acl = |agent: &str| {
            format!(
                "<{doc}#it> <https://ex.dev/ns#t> \"t\" <{doc}> .\n\
                 <{doc}.acl#a> <{RDF_TYPE}> <{ACL}Authorization> <{doc}.acl> .\n\
                 <{doc}.acl#a> <{ACL}accessTo> <{doc}> <{doc}.acl> .\n\
                 <{doc}.acl#a> <{ACL}agent> <{agent}> <{doc}.acl> .\n\
                 <{doc}.acl#a> <{ACL}mode> <{ACL}Read> <{doc}.acl> .\n"
            )
        };
        let bob = Session { agent: Some("https://bob.ex/me"), client: None, issuer: None, now: None };
        let pair = "https://bob.ex/me&client=https://app.ex";
        for internal in [
            crate::authindex::AUTHENTICATED,
            crate::authindex::ANY_CLIENT,
            "urn:sparq:pair?agent=https://bob.ex/me&client=https://app.ex",
            "urn:skolem:graph:x:b",
            pair,
        ] {
            let mut store = PodStore::new(Graph::load_dataset(&acl(internal), "nquads").unwrap());
            store.materialize_wac().expect("the rest of the pod still materializes");
            assert!(store.accessible(&bob, Mode::Read).is_empty(), "{internal} named as an agent");
            let app = Session { agent: Some("https://bob.ex/me"), client: Some("https://app.ex"), issuer: None, now: None };
            assert!(store.accessible(&app, Mode::Read).is_empty(), "{internal}: nor through a client");
        }
        let alice = "https://alice.ex/me";
        let mut store = PodStore::new(Graph::load_dataset(&acl(alice), "nquads").unwrap());
        store.materialize_wac().unwrap();
        let reads = |agent: &str, client: Option<&str>| {
            let s = Session { agent: Some(agent), client, issuer: None, now: None };
            !store.accessible(&s, Mode::Read).is_empty()
        };
        assert!(reads(alice, None), "control: alice reads");
        assert!(!reads(crate::authindex::AUTHENTICATED, None), "an internal agent gets nothing");
        assert!(!reads(alice, Some(crate::authindex::ANY_CLIENT)), "an internal client gets nothing");
    }

    /// An authoritative ACL replace resolves relative IRIs against the document itself.
    #[test]
    fn put_acl_resolves_relative_iris_against_the_document() {
        let doc = "https://pod.ex/n1";
        let nq = format!("<{doc}#it> <https://ex.dev/ns#t> \"t\" <{doc}> .\n");
        let mut store = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
        store.materialize_wac().unwrap();
        let body = "@prefix acl: <http://www.w3.org/ns/auth/acl#> .\n\
                    <#a> a acl:Authorization ; acl:accessTo <n1> ;\n\
                    acl:agent <https://alice.ex/me> ; acl:mode acl:Read .\n";
        store.put_acl("https://pod.ex/n1.acl", body, "turtle").unwrap();
        let s = Session { agent: Some("https://alice.ex/me"), client: None, issuer: None, now: None };
        assert!(store.accessible(&s, Mode::Read).iter().any(|r| r.as_str() == doc));
    }
}

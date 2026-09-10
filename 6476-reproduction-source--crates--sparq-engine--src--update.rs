//! SPARQL 1.1 Update (roadmap T10) over the FULL DATASET — default graph + named graphs.
//!
//! v1 applied data operations on the default graph only and silently DROPPED named graphs on
//! rebuild (conformance finding F19, a data-loss class). v2 models the dataset as a set of
//! term-triples per graph and implements every `GraphUpdateOperation`:
//! INSERT DATA / DELETE DATA with `GRAPH` blocks, `DELETE/INSERT … WHERE` with graph templates
//! (including variable graph names) and `USING (NAMED)` dataset re-scoping, CLEAR / DROP /
//! CREATE (ADD / COPY / MOVE arrive from the parser desugared into DROP + DELETE/INSERT),
//! and LOAD for `file://` sources. The rebuild path ([`update`]) is correct-and-simple O(n);
//! the delta-overlay path ([`update_in_place`]) keeps data operations O(batch) per graph.
//!
//! Failure policy: operations whose failure the spec leaves optional (CLEAR/DROP of an absent
//! graph, CREATE of an existing one) are no-ops here — a graph store that auto-creates graphs
//! is explicitly allowed to succeed on them — so `SILENT` only matters for LOAD.

// [OPUS-4.8] (sq-glw2) `empty_graph` is now only needed by the tests — the named-graph
// CLEAR/DROP arms route through the durable `Graph` methods instead of swapping in a fresh
// in-memory graph — so it is imported locally in the test module rather than crate-wide.
use crate::dataset::{build, decode_triples, TripleSet, TripleTerms};
use oxrdf::{BlankNode, NamedOrBlankNode, Term, Variable};
use rustc_hash::FxHashMap;
use sparq_core::Graph;
use spargebra::algebra::{GraphTarget, QueryDataset};
use spargebra::term::{
    GraphName, GraphNamePattern, GroundQuad, GroundQuadPattern, GroundTerm, GroundTermPattern,
    NamedNodePattern, Quad, QuadPattern, TermPattern,
};
use spargebra::GraphUpdateOperation;
use spargebra::SparqlParser;
use spargebra::Update;

/// A graph slot: `None` = the default graph, `Some(term)` = that named graph.
type GraphSlot = Option<Term>;

/// An instantiated update triple plus the graph slot it targets.
type SlotTriple = (GraphSlot, TripleTerms);

fn nob_to_term(s: &NamedOrBlankNode) -> Term {
    match s {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

fn ground_to_term(t: &GroundTerm) -> Term {
    match t {
        GroundTerm::NamedNode(n) => Term::NamedNode(n.clone()),
        GroundTerm::Literal(l) => Term::Literal(l.clone()),
        // A ground RDF 1.2 triple term in DELETE DATA: fully concrete.
        GroundTerm::Triple(t) => Term::Triple(Box::new(oxrdf::Triple::new(
            t.subject.clone(),
            t.predicate.clone(),
            ground_to_term(&t.object),
        ))),
    }
}

fn graph_name_slot(g: &GraphName) -> GraphSlot {
    match g {
        GraphName::DefaultGraph => None,
        GraphName::NamedNode(n) => Some(Term::NamedNode(n.clone())),
    }
}

/// [OPUS-4.8] roborev 1646 (Med): per SPARQL 1.1 Update §3.1.1, blank nodes in INSERT DATA are
/// "assumed to be disjoint from the blank nodes in the Graph Store" — they MUST be inserted as
/// FRESH nodes. The old `quad_to_triple` preserved the parsed label, so `INSERT DATA { _:b … }`
/// would conflate with an existing `_:b` in the store. We rewrite every blank-node label
/// (including inside RDF-1.2 triple terms) through `fresh`, so the same label within one INSERT
/// DATA operation stays one node while colliding with no existing graph blank node.
fn freshen_term(t: &Term, fresh: &mut FreshBnodes) -> Term {
    match t {
        Term::BlankNode(b) => fresh.get(b.as_str()),
        Term::Triple(tr) => {
            let subject = match freshen_term(&Term::from(tr.subject.clone()), fresh) {
                Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
                Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
                // a triple-term subject is always a named/blank node, so this is unreachable;
                // fall back to the original to stay total.
                _ => tr.subject.clone(),
            };
            let object = freshen_term(&tr.object, fresh);
            Term::Triple(Box::new(oxrdf::Triple::new(subject, tr.predicate.clone(), object)))
        }
        other => other.clone(),
    }
}

fn quad_to_triple_fresh(q: &Quad, fresh: &mut FreshBnodes) -> SlotTriple {
    let subject = freshen_term(&nob_to_term(&q.subject), fresh);
    (
        graph_name_slot(&q.graph_name),
        [subject, Term::NamedNode(q.predicate.clone()), freshen_term(&q.object, fresh)],
    )
}

fn ground_quad_to_triple(q: &GroundQuad) -> SlotTriple {
    (
        graph_name_slot(&q.graph_name),
        [Term::NamedNode(q.subject.clone()), Term::NamedNode(q.predicate.clone()), ground_to_term(&q.object)],
    )
}

// --- the mutable dataset model (rebuild path) ---------------------------------------------------
// (Decode/build primitives live in `crate::dataset`, shared with the query side's
// FROM / FROM NAMED active-dataset construction.)

/// The whole dataset as decoded term-triple sets: the working representation of the rebuild
/// path. Named-graph order is preserved; a named graph may be present and EMPTY (CREATE).
struct Dataset {
    default: TripleSet,
    named: Vec<(Term, TripleSet)>,
}

impl Dataset {
    fn decode(graph: &Graph) -> Dataset {
        Dataset {
            default: decode_triples(graph),
            named: graph.named.iter().map(|(name, g)| (name.clone(), decode_triples(g))).collect(),
        }
    }

    fn graph(&self, name: &Term) -> Option<&TripleSet> {
        self.named.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    /// The named graph's set, CREATING an empty one if absent (auto-create store semantics).
    fn graph_mut(&mut self, name: &Term) -> &mut TripleSet {
        if let Some(i) = self.named.iter().position(|(n, _)| n == name) {
            return &mut self.named[i].1;
        }
        self.named.push((name.clone(), TripleSet::default()));
        &mut self.named.last_mut().unwrap().1
    }

    fn slot_mut(&mut self, slot: &GraphSlot) -> &mut TripleSet {
        match slot {
            None => &mut self.default,
            Some(name) => self.graph_mut(name),
        }
    }

    fn insert(&mut self, slot: &GraphSlot, t: TripleTerms) {
        self.slot_mut(slot).insert(t);
    }

    fn remove(&mut self, slot: &GraphSlot, t: &TripleTerms) {
        match slot {
            None => {
                self.default.remove(t);
            }
            // Deleting from an absent graph is a no-op (it has nothing to delete).
            Some(name) => {
                if let Some(i) = self.named.iter().position(|(n, _)| n == name) {
                    self.named[i].1.remove(t);
                }
            }
        }
    }

    /// Materialise the dataset as a queryable [`Graph`] (named graphs included, even empty
    /// ones — `GRAPH <g> {}` over a CREATEd-but-empty graph must yield the unit row).
    fn build(&self) -> Graph {
        let mut g = build(&self.default);
        g.named = self.named.iter().map(|(name, set)| (name.clone(), build(set))).collect();
        g
    }

    /// The active dataset for a `USING (NAMED)` / `WITH` re-scoped WHERE: the default
    /// graph is the UNION of the `USING` graphs (absent store graphs contribute nothing,
    /// like a FROM of an empty document). `named: Some(list)` — explicit `USING NAMED` —
    /// makes exactly those the named graphs; `named: None` (how the parser encodes `WITH`,
    /// which only re-scopes the DEFAULT graph) keeps the store's named graphs untouched.
    fn build_using(&self, u: &QueryDataset) -> Graph {
        let mut default = TripleSet::default();
        for n in &u.default {
            if let Some(s) = self.graph(&Term::NamedNode(n.clone())) {
                default.extend(s.iter().cloned());
            }
        }
        let mut g = build(&default);
        match &u.named {
            Some(named) => {
                for n in named {
                    let name = Term::NamedNode(n.clone());
                    if let Some(s) = self.graph(&name) {
                        g.named.push((name, build(s)));
                    }
                }
            }
            None => g.named = self.named.iter().map(|(name, set)| (name.clone(), build(set))).collect(),
        }
        g
    }
}

// --- template instantiation for DELETE/INSERT … WHERE ------------------------------------------
// Substitute a template term from a WHERE solution. `None` => an unbound variable (or an
// ill-formed substitution), so the whole quad is skipped (per SPARQL, a template quad with an
// unbound/invalid slot is not produced).
type Subst<'a> = dyn Fn(&Variable) -> Option<Term> + 'a;

/// Fresh-blank-node state for ONE solution row: per SPARQL, every blank node in an INSERT
/// template is instantiated FRESH per solution (same label, same row → same fresh node;
/// different rows — and different operations in one request — get DIFFERENT nodes, hence
/// the process-wide counter).
struct FreshBnodes {
    map: FxHashMap<String, Term>,
}

static FRESH_BNODE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl FreshBnodes {
    fn get(&mut self, label: &str) -> Term {
        if let Some(t) = self.map.get(label) {
            return t.clone();
        }
        let n = FRESH_BNODE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = Term::BlankNode(BlankNode::new_unchecked(format!("fb{n}")));
        self.map.insert(label.to_string(), t.clone());
        t
    }
}

fn tp_subst(tp: &TermPattern, get: &Subst, fresh: &mut FreshBnodes) -> Option<Term> {
    match tp {
        TermPattern::Variable(v) => get(v),
        TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        TermPattern::BlankNode(b) => Some(fresh.get(b.as_str())),
        TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
        // RDF 1.2 triple term in an INSERT template: substitute the components recursively.
        TermPattern::Triple(t) => {
            let subject = match tp_subst(&t.subject, get, fresh)? {
                Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
                Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
                _ => return None,
            };
            let predicate = match nnp_subst(&t.predicate, get)? {
                Term::NamedNode(n) => n,
                _ => return None,
            };
            let object = tp_subst(&t.object, get, fresh)?;
            Some(Term::Triple(Box::new(oxrdf::Triple::new(subject, predicate, object))))
        }
    }
}

fn gtp_subst(tp: &GroundTermPattern, get: &Subst) -> Option<Term> {
    match tp {
        GroundTermPattern::Variable(v) => get(v),
        GroundTermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        GroundTermPattern::Literal(l) => Some(Term::Literal(l.clone())),
        GroundTermPattern::Triple(t) => {
            let subject = match gtp_subst(&t.subject, get)? {
                Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
                Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
                _ => return None,
            };
            let predicate = match nnp_subst(&t.predicate, get)? {
                Term::NamedNode(n) => n,
                _ => return None,
            };
            let object = gtp_subst(&t.object, get)?;
            Some(Term::Triple(Box::new(oxrdf::Triple::new(subject, predicate, object))))
        }
    }
}

fn nnp_subst(p: &NamedNodePattern, get: &Subst) -> Option<Term> {
    match p {
        NamedNodePattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        NamedNodePattern::Variable(v) => get(v),
    }
}

/// Resolve a template's graph slot. Outer `None` = skip the quad (unbound variable or a
/// literal where a graph name is required); `Some(None)` = the default graph.
fn gnp_subst(g: &GraphNamePattern, get: &Subst) -> Option<GraphSlot> {
    match g {
        GraphNamePattern::DefaultGraph => Some(None),
        GraphNamePattern::NamedNode(n) => Some(Some(Term::NamedNode(n.clone()))),
        GraphNamePattern::Variable(v) => match get(v)? {
            t @ (Term::NamedNode(_) | Term::BlankNode(_)) => Some(Some(t)),
            _ => None, // a literal / triple term cannot name a graph
        },
    }
}

/// Evaluates a DELETE/INSERT … WHERE pattern against `active` and instantiates the delete /
/// insert templates per solution as (graph-slot, triple) pairs. Shared by the rebuild
/// (`update`) and delta-overlay (`update_in_place`) paths.
fn instantiate_templates(
    active: &Graph,
    delete: &[GroundQuadPattern],
    insert: &[QuadPattern],
    pattern: &spargebra::algebra::GraphPattern,
) -> Result<(Vec<SlotTriple>, Vec<SlotTriple>), String> {
    let result = crate::exec::eval_select(active, pattern)?;
    let cols: FxHashMap<Variable, usize> =
        result.vars.iter().enumerate().map(|(i, v)| (v.clone(), i)).collect();
    let mut dels: Vec<SlotTriple> = Vec::new();
    let mut inss: Vec<SlotTriple> = Vec::new();
    for row in &result.rows {
        let get = |v: &Variable| -> Option<Term> { cols.get(v).and_then(|&i| row[i].clone()) };
        for dp in delete {
            let (Some(slot), Some(s), Some(p), Some(o)) = (
                gnp_subst(&dp.graph_name, &get),
                gtp_subst(&dp.subject, &get),
                nnp_subst(&dp.predicate, &get),
                gtp_subst(&dp.object, &get),
            ) else {
                continue;
            };
            dels.push((slot, [s, p, o]));
        }
        let mut fresh = FreshBnodes { map: FxHashMap::default() };
        for ip in insert {
            let (Some(slot), Some(s), Some(p), Some(o)) = (
                gnp_subst(&ip.graph_name, &get),
                tp_subst(&ip.subject, &get, &mut fresh),
                nnp_subst(&ip.predicate, &get),
                tp_subst(&ip.object, &get, &mut fresh),
            ) else {
                continue;
            };
            inss.push((slot, [s, p, o]));
        }
    }
    Ok((dels, inss))
}

// --- LOAD ----------------------------------------------------------------------------------------

/// LOAD `file://` policy (roborev 1646, High). [OPUS-4.8] `LOAD <file://…>` dereferences a
/// server-LOCAL file, so on the public update path (HTTP `apply_update`) an untrusted client
/// could otherwise import any RDF-readable file the process can read and query it back. The
/// DEFAULT policy is therefore REJECT: `file://` LOAD only works when a caller has explicitly
/// installed an allowlisted base directory via [`with_load_base`] (the conformance runner does
/// this, pointing at the local test-data tree). The requested path is canonicalised and must
/// resolve UNDER that base, so `../` traversal and symlinks cannot escape it.
pub(crate) mod load_policy {
    use std::cell::RefCell;
    use std::path::PathBuf;

    thread_local! {
        static BASE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Uninstalls the allowlisted base when the installing scope returns (also on
    /// error/unwind, so a poisoned thread never leaks a trusted base).
    pub(crate) struct Guard(Option<PathBuf>);
    impl Drop for Guard {
        fn drop(&mut self) {
            BASE.with(|b| *b.borrow_mut() = self.0.take());
        }
    }

    /// Installs `base` (canonicalised) as the allowlisted LOAD directory for the duration of
    /// the returned guard. Restores the previous base on drop (supports nesting).
    pub(crate) fn install(base: PathBuf) -> Guard {
        let canon = std::fs::canonicalize(&base).unwrap_or(base);
        BASE.with(|b| {
            let prev = b.borrow_mut().replace(canon);
            Guard(prev)
        })
    }

    pub(crate) fn base() -> Option<PathBuf> {
        BASE.with(|b| b.borrow().clone())
    }
}

/// Runs `f` with `base` allowlisted as the root for `LOAD <file://…>` (roborev 1646). Without
/// this, every `file://` LOAD is REJECTED — the secure default for public update paths. Trusted
/// local callers (the SPARQL conformance runner) wrap their `update` call in this to permit
/// loading the test-data files that live under `base`.
pub fn with_load_base<R>(base: impl Into<std::path::PathBuf>, f: impl FnOnce() -> R) -> R {
    let _guard = load_policy::install(base.into());
    f()
}

/// Reads and parses a `file://` document for LOAD. `file://` is REJECTED unless an allowlisted
/// base directory is installed (see [`with_load_base`]); when one is, the resolved path is
/// canonicalised and must lie UNDER it. Non-file schemes are an error — `LOAD SILENT` turns any
/// of these into a no-op at the call sites.
fn load_document(source: &str) -> Result<TripleSet, String> {
    let raw = source
        .strip_prefix("file://")
        .ok_or_else(|| format!("LOAD source not supported (only file:// URIs): {source}"))?;
    // `file:///abs` -> "/abs"; keep the literal-strip behaviour for the relative-path form the
    // conformance suites use, but require an installed allowlist either way.
    let Some(base) = load_policy::base() else {
        return Err(format!(
            "LOAD of {source} refused: file:// access is disabled (no allowlisted base directory configured)"
        ));
    };
    let requested = std::path::Path::new(raw);
    // Resolve relative paths against the allowlisted base, then canonicalise to collapse `..`
    // and follow symlinks so the containment check cannot be bypassed.
    let joined = if requested.is_absolute() { requested.to_path_buf() } else { base.join(requested) };
    let path = std::fs::canonicalize(&joined).map_err(|e| format!("LOAD {source}: {e}"))?;
    if !path.starts_with(&base) {
        return Err(format!("LOAD of {source} refused: resolves outside the allowlisted base directory"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("LOAD {source}: {e}"))?;
    let format = match path.extension().and_then(|e| e.to_str()) {
        Some("nt") => "ntriples",
        Some("nq") => "nquads",
        Some("trig") => "trig",
        _ => "turtle",
    };
    let g = Graph::load_str(&text, format).map_err(|e| format!("LOAD {source}: {e}"))?;
    Ok(decode_triples(&g))
}

// --- the rebuild path ----------------------------------------------------------------------------

/// Applies one parsed operation to the dataset model.
fn apply_op(ds: &mut Dataset, op: &GraphUpdateOperation) -> Result<(), String> {
    match op {
        GraphUpdateOperation::InsertData { data } => {
            // Fresh blank nodes for the whole operation (roborev 1646): same label in this
            // INSERT DATA -> same node; never the existing store's `_:b`.
            let mut fresh = FreshBnodes { map: FxHashMap::default() };
            for q in data {
                let (slot, t) = quad_to_triple_fresh(q, &mut fresh);
                ds.insert(&slot, t);
            }
        }
        GraphUpdateOperation::DeleteData { data } => {
            for q in data {
                let (slot, t) = ground_quad_to_triple(q);
                ds.remove(&slot, &t);
            }
        }
        // CLEAR empties the target graph(s) but keeps named-graph entries; clearing an
        // absent named graph is a no-op (auto-create stores may succeed — we do).
        GraphUpdateOperation::Clear { graph: target, .. } => match target {
            GraphTarget::DefaultGraph => ds.default.clear(),
            GraphTarget::NamedNode(n) => {
                let name = Term::NamedNode(n.clone());
                if let Some(i) = ds.named.iter().position(|(g, _)| *g == name) {
                    ds.named[i].1.clear();
                }
            }
            GraphTarget::NamedGraphs => ds.named.iter_mut().for_each(|(_, s)| s.clear()),
            GraphTarget::AllGraphs => {
                ds.default.clear();
                ds.named.iter_mut().for_each(|(_, s)| s.clear());
            }
        },
        // DROP removes named-graph entries; the default graph always exists, so DROP
        // DEFAULT only empties it.
        GraphUpdateOperation::Drop { graph: target, .. } => match target {
            GraphTarget::DefaultGraph => ds.default.clear(),
            GraphTarget::NamedNode(n) => {
                let name = Term::NamedNode(n.clone());
                ds.named.retain(|(g, _)| *g != name);
            }
            GraphTarget::NamedGraphs => ds.named.clear(),
            GraphTarget::AllGraphs => {
                ds.default.clear();
                ds.named.clear();
            }
        },
        GraphUpdateOperation::Create { graph, .. } => {
            ds.graph_mut(&Term::NamedNode(graph.clone()));
        }
        GraphUpdateOperation::Load { silent, source, destination } => {
            match load_document(source.as_str()) {
                Ok(triples) => ds.slot_mut(&graph_name_slot(destination)).extend(triples),
                Err(e) => {
                    if !silent {
                        return Err(e);
                    }
                }
            }
        }
        // DELETE { d } INSERT { i } WHERE { p } — evaluate the WHERE against the dataset
        // state so far (re-scoped by USING when present), instantiate the templates per
        // solution, then apply all deletes and then all inserts (SPARQL semantics).
        GraphUpdateOperation::DeleteInsert { delete, insert, using, pattern } => {
            let active = match using {
                Some(u) => ds.build_using(u),
                None => ds.build(),
            };
            let (dels, inss) = instantiate_templates(&active, delete, insert, pattern)?;
            for (slot, t) in &dels {
                ds.remove(slot, t);
            }
            for (slot, t) in inss {
                ds.insert(&slot, t);
            }
        }
    }
    Ok(())
}

/// Apply a SPARQL Update string to `graph`, returning the updated graph (named graphs
/// preserved). Errors (leaving the input untouched — it is borrowed) on a parse error or a
/// non-SILENT failing LOAD.
pub fn update(graph: &Graph, sparql: &str) -> Result<Graph, String> {
    let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
    apply_update_rebuild(graph, &upd)
}

/// The shared rebuild loop over an ALREADY-PARSED `Update` (decode → apply ops →
/// rebuild). Shared by [`update`] and, under the `params` feature, the parameterized
/// prepared-update path so the bound algebra is applied DIRECTLY (no re-serialise /
/// re-parse — a hostile bound value can never re-enter the parser). [OPUS-4.8] (sq-rp3um)
fn apply_update_rebuild(graph: &Graph, upd: &Update) -> Result<Graph, String> {
    let mut ds = Dataset::decode(graph);
    for op in &upd.operations {
        apply_op(&mut ds, op)?;
    }
    Ok(ds.build())
}

/// [OPUS-4.8] (sq-rp3um) [`update`] over an ALREADY-PARSED bound `Update` — the rebuild
/// path for a parameterized [`crate::PreparedUpdate`] (`params` feature).
#[cfg(feature = "params")]
pub(crate) fn update_prepared_impl(graph: &Graph, upd: &Update) -> Result<Graph, String> {
    apply_update_rebuild(graph, upd)
}

/// [OPUS-4.8] (sq-rp3um) [`update_in_place_with_budget`] over an ALREADY-PARSED bound
/// `Update` (parameterized [`crate::PreparedUpdate`], `params` feature). Applies the
/// bound algebra in place through the delta overlay without re-serialising it.
#[cfg(feature = "params")]
pub(crate) fn update_in_place_prepared_with_budget(
    graph: &mut Graph,
    upd: &Update,
    budget: &crate::QueryBudget,
) -> Result<(), String> {
    let _budget = crate::exec::budget::install(budget);
    apply_update_in_place(graph, upd, None)
}

// --- the delta-overlay path ------------------------------------------------------------------

/// Groups (graph-slot, triple) pairs per graph slot, preserving first-seen slot order.
fn group_by_slot(items: Vec<SlotTriple>) -> Vec<(GraphSlot, Vec<TripleTerms>)> {
    let mut out: Vec<(GraphSlot, Vec<TripleTerms>)> = Vec::new();
    for (slot, t) in items {
        match out.iter_mut().find(|(s, _)| *s == slot) {
            Some((_, v)) => v.push(t),
            None => out.push((slot, vec![t])),
        }
    }
    out
}

/// Applies one insert/delete batch to the graph slot through the delta overlay,
/// auto-creating an empty named graph for an insert into an absent one.
fn apply_slot_delta(
    graph: &mut Graph,
    slot: &GraphSlot,
    inserts: &[TripleTerms],
    deletes: &[TripleTerms],
) -> Result<(), String> {
    match slot {
        None => graph.apply_delta(inserts, deletes),
        Some(name) => {
            if let Some(i) = graph.named.iter().position(|(n, _)| n == name) {
                return graph.named[i].1.apply_delta(inserts, deletes);
            }
            if inserts.is_empty() {
                return Ok(()); // deleting from an absent graph is a no-op
            }
            // [OPUS-4.8] (sq-7cxr, gh-44) Route NEW named-graph creation through
            // `Graph::ensure_named` so that on a DIRECTORY-BACKED parent the sub-graph is
            // born durable (its own WAL + manifest entry) — its first triples then persist
            // across a restart like any other write. For an in-memory parent this is the
            // unchanged `empty_graph()` path.
            let i = graph.ensure_named(name)?;
            graph.named[i].1.apply_delta(inserts, &[])
        }
    }
}

// --- durable-mirror effect capture (sq-7cxr, gh-44; Copilot PR#80 fix) -------------------------

/// [OPUS-4.8] (sq-7cxr, Copilot PR#80) A **resolved**, deterministic record of what one
/// `update_in_place` actually did, captured during the single in-memory application so a
/// durable mirror can reproduce EXACTLY that state without re-executing the update string.
///
/// Re-running the update *text* against a second (durable) graph is unsound for any update
/// whose effect is not a pure function of the text: `NOW()`/`RAND()`/`UUID()`/`STRUUID()` and
/// fresh `BNODE()`s re-roll to different values, and `LOAD <remote>` can fetch different
/// content — so the durable graph would diverge from the already-acked in-memory state,
/// breaking the "204 ⇒ durable" guarantee after a restart. Capturing the resolved per-slot
/// triple delta (and replaying THAT) makes the second phase deterministic by construction:
/// the durable graph receives the identical resolved triples the in-memory side committed.
///
/// Structural operations (CLEAR/DROP/CREATE) ARE pure functions of the text, so they are
/// recorded as the operation itself and re-applied through the same in-place machinery — there
/// is no value to re-roll. Only the data-bearing operations (INSERT/DELETE DATA, LOAD, and
/// DELETE/INSERT … WHERE) carry resolved triples.
#[derive(Debug, Clone)]
pub enum UpdateEffect {
    /// A resolved per-slot insert/delete batch — the deterministic delta the in-memory
    /// application produced for one graph slot (`None` = default graph, `Some` = named graph).
    /// Deletes are applied before inserts, mirroring the in-memory order.
    Delta { slot: GraphSlot, inserts: Vec<TripleTerms>, deletes: Vec<TripleTerms> },
    /// `CLEAR <target>` — deterministic; replayed verbatim.
    Clear(GraphTarget),
    /// `DROP <target>` — deterministic; replayed verbatim.
    Drop(GraphTarget),
    /// `CREATE GRAPH <name>` — deterministic; replayed verbatim (a durable empty named graph).
    Create(Term),
}

/// [OPUS-4.8] (sq-7cxr, Copilot PR#80) Optional sink for [`UpdateEffect`]s, threaded through
/// the in-place apply so capture is zero-cost (`None`) when no durable mirror is configured.
type EffectSink<'a> = Option<&'a mut Vec<UpdateEffect>>;

/// Records a resolved delta effect (only when a sink is present and the batch is non-empty).
fn record_delta(sink: &mut EffectSink, slot: &GraphSlot, inserts: &[TripleTerms], deletes: &[TripleTerms]) {
    if let Some(s) = sink {
        if !inserts.is_empty() || !deletes.is_empty() {
            s.push(UpdateEffect::Delta {
                slot: slot.clone(),
                inserts: inserts.to_vec(),
                deletes: deletes.to_vec(),
            });
        }
    }
}

/// Applies a SPARQL Update IN PLACE through the store's DELTA-OVERLAY (roadmap T17): the data
/// operations route to [`Graph::apply_delta`] per target graph — O(batch) per operation instead
/// of the O(n) decode-everything-and-rebuild that [`update`] performs — and, for a
/// directory-backed graph, each batch is WAL-logged (fsync'd) before it is applied, so updates
/// are durable across a crash. [OPUS-4.8] (sq-glw2) `CLEAR`/`DROP` are durable BEFORE the ack on
/// a directory-backed graph too: default-graph CLEAR/DROP retract via [`Graph::clear_default_durable`];
/// named-graph CLEAR empties the slot via [`Graph::clear_named_durable`] (WAL-logged retraction,
/// slot preserved) and named-graph DROP removes the sub-dir + manifest entry via
/// [`Graph::drop_named_durable`]. Fold the accumulated overlay back into the base periodically
/// with [`Graph::compact`].
pub fn update_in_place(graph: &mut Graph, sparql: &str) -> Result<(), String> {
    update_in_place_with_budget(graph, sparql, &crate::QueryBudget::unlimited())
}

/// [OPUS-4.8] (sq-ebii) [`update_in_place`] under a cooperative [`crate::QueryBudget`].
///
/// The budget is installed thread-locally for the whole update, so a `DELETE/INSERT … WHERE`
/// whose WHERE pattern blows up is bounded EXACTLY as a budgeted query's `SELECT` is: the
/// cross-product allocation is capped up-front (`budget::cap_alloc`) and the evaluation
/// aborts at the deadline / row cap (`"query budget exceeded (timeout|max-rows)"`), instead
/// of running the writer thread to an OOM. The non-WHERE operations (INSERT/DELETE DATA,
/// CLEAR/DROP/CREATE/LOAD) do not consult the budget — they are bounded by their operand
/// size, which the request body-size limit already caps — so an unlimited budget (the
/// default, via [`update_in_place`]) is byte-for-byte the previous behaviour.
pub fn update_in_place_with_budget(
    graph: &mut Graph,
    sparql: &str,
    budget: &crate::QueryBudget,
) -> Result<(), String> {
    // No effect sink: capture is fully elided, so this is byte-for-byte the previous behaviour.
    update_in_place_core(graph, sparql, budget, None)
}

/// [OPUS-4.8] (sq-7cxr, Copilot PR#80) [`update_in_place_with_budget`] that ALSO returns the
/// ordered, *resolved* [`UpdateEffect`] log of what it applied — for a durable mirror that must
/// reproduce the committed state WITHOUT re-executing the (possibly non-deterministic /
/// side-effecting) update text. See [`UpdateEffect`] and [`apply_effects`].
///
/// On error the in-memory `graph` is left in whatever partially-applied state the update
/// reached (identical to [`update_in_place_with_budget`]); the returned effect log is only
/// produced on success, so a caller never mirrors a half-applied update.
pub fn update_in_place_capturing(
    graph: &mut Graph,
    sparql: &str,
    budget: &crate::QueryBudget,
) -> Result<Vec<UpdateEffect>, String> {
    let mut effects = Vec::new();
    update_in_place_core(graph, sparql, budget, Some(&mut effects))?;
    Ok(effects)
}

/// [OPUS-4.8] (sq-o1wp) Request-ATOMIC variant of [`update_in_place`]: applies the whole
/// `;`-separated update request to a private [`fork`](Graph::fork) and only commits it back into
/// `graph` (by replacing `*graph`) when EVERY operation succeeds. On any error — a parse failure,
/// a non-SILENT failing `LOAD`, or a budget/WHERE blow-up on operation *K* — the partial fork is
/// discarded and `graph` is left EXACTLY at its pre-request state (full rollback).
///
/// This packages, as a safe public default, the request-level-atomicity pattern the production
/// serve writer already uses (fork → apply → seal-only-on-`Ok`; see
/// `fork_and_seal_recovers_request_atomicity`). The bare [`update_in_place`] primitive is, by its
/// documented contract, NON-atomic on error: a failing later operation leaves the earlier ops'
/// partial prefix applied (see `in_place_request_is_partial_on_error_by_contract`). That is the
/// fast path the serve writer wraps; a *direct library consumer* that does not implement its own
/// fork/seal recovery should reach for THIS function to get SPARQL 1.1's all-or-nothing request
/// semantics without the footgun.
///
/// Cost: one [`fork`](Graph::fork) (O(pending delta), not O(triples)) plus the in-place apply. The
/// fork carries no WAL/redo-journal, so the committed state inherits `graph`'s durability story
/// only after the swap — for a directory-backed graph prefer the serve writer path, which seals
/// through the durable base. Runs under an unlimited [`crate::QueryBudget`]; use
/// [`update_in_place_atomic_with_budget`] to bound a `DELETE/INSERT … WHERE`.
pub fn update_in_place_atomic(graph: &mut Graph, sparql: &str) -> Result<(), String> {
    update_in_place_atomic_with_budget(graph, sparql, &crate::QueryBudget::unlimited())
}

/// [OPUS-4.8] (sq-o1wp) [`update_in_place_atomic`] under a cooperative [`crate::QueryBudget`] (the
/// budget bounds a `DELETE/INSERT … WHERE` exactly as in [`update_in_place_with_budget`]). The
/// fork is applied under the budget; a budget-exceeded error rolls the request back whole.
pub fn update_in_place_atomic_with_budget(
    graph: &mut Graph,
    sparql: &str,
    budget: &crate::QueryBudget,
) -> Result<(), String> {
    let mut working = graph.fork();
    update_in_place_with_budget(&mut working, sparql, budget)?;
    *graph = working;
    Ok(())
}

/// The shared in-place apply core. When `sink` is `Some`, every operation's RESOLVED effect is
/// recorded into it so a durable mirror can replay the exact committed delta (see
/// [`update_in_place_capturing`]); when `None`, capture is fully elided.
fn update_in_place_core(
    graph: &mut Graph,
    sparql: &str,
    budget: &crate::QueryBudget,
    sink: EffectSink,
) -> Result<(), String> {
    let _budget = crate::exec::budget::install(budget);
    let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
    apply_update_in_place(graph, &upd, sink)
}

/// The shared per-operation in-place apply loop over an ALREADY-PARSED `Update`.
/// Split out of [`update_in_place_core`] so the parameterized prepared-update path
/// ([`crate::PreparedUpdate`], `params` feature) can apply a bound algebra without
/// re-serialising it to a string. [OPUS-4.8] (sq-rp3um)
fn apply_update_in_place(
    graph: &mut Graph,
    upd: &Update,
    mut sink: EffectSink,
) -> Result<(), String> {
    for op in &upd.operations {
        match op {
            GraphUpdateOperation::InsertData { data } => {
                // Fresh blank nodes for the whole operation (roborev 1646) — see apply_op.
                let mut fresh = FreshBnodes { map: FxHashMap::default() };
                let triples: Vec<_> = data.iter().map(|q| quad_to_triple_fresh(q, &mut fresh)).collect();
                for (slot, ins) in group_by_slot(triples) {
                    apply_slot_delta(graph, &slot, &ins, &[])?;
                    record_delta(&mut sink, &slot, &ins, &[]);
                }
            }
            GraphUpdateOperation::DeleteData { data } => {
                for (slot, del) in group_by_slot(data.iter().map(ground_quad_to_triple).collect()) {
                    apply_slot_delta(graph, &slot, &[], &del)?;
                    record_delta(&mut sink, &slot, &[], &del);
                }
            }
            // [OPUS-4.8] (sq-glw2) CLEAR keeps the named-graph SLOT but empties it. On a
            // directory-backed parent, retract the existing graph's contents through its own WAL
            // (`Graph::clear_named_durable`) so the emptied state is fsync'd BEFORE the ack — the
            // old `entry.1 = empty_graph()` dropped the sub-graph's WAL/dir association, so the
            // clear was durable only at the next compaction. Absent graph: a no-op; in-memory
            // parent: a plain store clear (unchanged).
            GraphUpdateOperation::Clear { graph: target, .. } => {
                match target {
                    GraphTarget::DefaultGraph => replace_default(graph)?,
                    GraphTarget::NamedNode(n) => {
                        graph.clear_named_durable(&Term::NamedNode(n.clone()))?;
                    }
                    GraphTarget::NamedGraphs => clear_all_named_durable(graph)?,
                    GraphTarget::AllGraphs => {
                        replace_default(graph)?;
                        clear_all_named_durable(graph)?;
                    }
                }
                if let Some(s) = &mut sink {
                    s.push(UpdateEffect::Clear(target.clone()));
                }
            }
            // [OPUS-4.8] (sq-glw2) DROP makes the named graph cease to exist. On a directory-
            // backed parent, `Graph::drop_named_durable` removes the sub-dir + manifest entry
            // durably (fsync'd) so the removal survives a reopen immediately — the old
            // `graph.named.retain(...)` dropped only the in-memory entry, leaving the on-disk
            // sub-dir + manifest entry so a reopen RESTORED the dropped graph. Absent graph: a
            // no-op; in-memory parent: a plain entry removal (unchanged).
            GraphUpdateOperation::Drop { graph: target, .. } => {
                match target {
                    GraphTarget::DefaultGraph => replace_default(graph)?,
                    GraphTarget::NamedNode(n) => {
                        graph.drop_named_durable(&Term::NamedNode(n.clone()))?;
                    }
                    GraphTarget::NamedGraphs => drop_all_named_durable(graph)?,
                    GraphTarget::AllGraphs => {
                        replace_default(graph)?;
                        drop_all_named_durable(graph)?;
                    }
                }
                if let Some(s) = &mut sink {
                    s.push(UpdateEffect::Drop(target.clone()));
                }
            }
            GraphUpdateOperation::Create { graph: name, .. } => {
                // [OPUS-4.8] (sq-7cxr, gh-44) `ensure_named` makes the new graph DURABLE on a
                // directory-backed parent (empty graphs are persisted via the manifest, so an
                // empty CREATE survives a restart) and is a no-op when it already exists.
                let name = Term::NamedNode(name.clone());
                graph.ensure_named(&name)?;
                if let Some(s) = &mut sink {
                    s.push(UpdateEffect::Create(name));
                }
            }
            GraphUpdateOperation::Load { silent, source, destination } => {
                match load_document(source.as_str()) {
                    Ok(triples) => {
                        let slot = graph_name_slot(destination);
                        let ins: Vec<TripleTerms> = triples.into_iter().collect();
                        apply_slot_delta(graph, &slot, &ins, &[])?;
                        // Capture the RESOLVED loaded triples: a re-LOAD on the durable graph
                        // could fetch different remote content, so we mirror what was actually
                        // committed in memory, not a second fetch.
                        record_delta(&mut sink, &slot, &ins, &[]);
                    }
                    Err(e) => {
                        if !silent {
                            return Err(e);
                        }
                    }
                }
            }
            GraphUpdateOperation::DeleteInsert { delete, insert, using, pattern } => {
                // Without USING the WHERE pattern is evaluated against the graph as updated
                // so far in this request (scans merge the overlay; GRAPH blocks see the live
                // named graphs), matching the rebuild path's semantics. With USING, the
                // re-scoped active dataset is materialised from the current state.
                let (dels, inss) = match using {
                    Some(u) => {
                        let active = Dataset::decode(graph).build_using(u);
                        instantiate_templates(&active, delete, insert, pattern)?
                    }
                    None => instantiate_templates(graph, delete, insert, pattern)?,
                };
                // All deletes first, then all inserts (SPARQL semantics), per graph slot.
                for (slot, del) in group_by_slot(dels) {
                    apply_slot_delta(graph, &slot, &[], &del)?;
                    record_delta(&mut sink, &slot, &[], &del);
                }
                for (slot, ins) in group_by_slot(inss) {
                    apply_slot_delta(graph, &slot, &ins, &[])?;
                    record_delta(&mut sink, &slot, &ins, &[]);
                }
            }
        }
    }
    Ok(())
}

/// [OPUS-4.8] (sq-7cxr, Copilot PR#80) Replays a captured [`UpdateEffect`] log onto `graph`,
/// reproducing the exact committed state a [`update_in_place_capturing`] call applied to its
/// (identically-seeded) in-memory working copy — WITHOUT re-parsing or re-evaluating the
/// original update text. This is the durable mirror's apply path: each [`UpdateEffect::Delta`]
/// goes through [`Graph::apply_delta`] (WAL-appended + fsync'd BEFORE it is applied on a
/// directory-backed graph), and the structural ops go through the same deterministic in-place
/// machinery the data path uses, so the durable graph is byte-equivalent to the in-memory one.
///
/// Effects are applied in capture order, which is the order the in-memory side committed them
/// (deletes before inserts within a DELETE/INSERT … WHERE), so order-sensitive sequences
/// reproduce faithfully.
///
/// [OPUS-4.8] (sq-ycle) ATOMICITY: a multi-op body materialises through MANY independent
/// `apply_delta` fsyncs across DIFFERENT files (the per-slot `wal.log`s + the `named.bin`
/// manifest), so a crash between them used to leave a PARTIAL durable write (e.g. a `<parent>
/// ldp:contains <r>` containment present without the `<r>` graph it points at). To make the WHOLE
/// body ONE all-or-nothing durable commit we are now JOURNAL-FIRST, in four steps.
///
/// 1. RESOLVE every effect into a flat ordered quad-delta against the CURRENT graph state, so
///    CLEAR/DROP expand to the concrete retractions of the triples present NOW.
/// 2. [`Graph::commit_txn`] writes that delta as ONE fsync'd `txn.log` frame — THE commit point
///    (a no-op for an in-memory graph and for an empty delta).
/// 3. MATERIALISE via the existing per-effect loop; the per-graph WAL fsyncs here are now
///    redundant-on-crash, not the commit point — correct, since `open` redoes the journal frame.
/// 4. [`Graph::clear_txn`] empties the journal so the next body starts fresh.
///
/// For an IN-MEMORY graph (no journal/WAL) steps 2 & 4 are no-ops, so its behaviour is byte-for-byte
/// unchanged. `DROP`'s manifest/sub-dir removal stays a materialisation concern (already crash-safe
/// via `recover_named_drop`); the journal only carries the DATA retraction needed for atomicity.
pub fn apply_effects(graph: &mut Graph, effects: &[UpdateEffect]) -> Result<(), String> {
    // [OPUS-4.8] (sq-ycle) (1) RESOLVE + (2) COMMIT: the single durable commit point for the body.
    let records = resolve_effect_records(graph, effects);
    graph.commit_txn(&records)?;
    // (3) MATERIALISE: the per-effect loop (unchanged from the pre-journal `apply_effects`).
    for effect in effects {
        match effect {
            UpdateEffect::Delta { slot, inserts, deletes } => {
                // Deletes first, then inserts — the same per-slot order the in-memory side used.
                apply_slot_delta(graph, slot, &[], deletes)?;
                apply_slot_delta(graph, slot, inserts, &[])?;
            }
            // [OPUS-4.8] (sq-glw2) The durable mirror replays CLEAR/DROP through the SAME durable
            // helpers the live path uses (`clear_named_durable` / `drop_named_durable`), so the
            // mirror's named-graph clear/drop is WAL/manifest-durable before its ack too — not
            // just at the next compaction (the old `empty_graph()`/`retain` swap dropped the
            // sub-graph WAL / left the on-disk sub-dir, so the mirror would resurrect the data).
            UpdateEffect::Clear(target) => match target {
                GraphTarget::DefaultGraph => replace_default(graph)?,
                GraphTarget::NamedNode(n) => {
                    graph.clear_named_durable(&Term::NamedNode(n.clone()))?;
                }
                GraphTarget::NamedGraphs => clear_all_named_durable(graph)?,
                GraphTarget::AllGraphs => {
                    replace_default(graph)?;
                    clear_all_named_durable(graph)?;
                }
            },
            UpdateEffect::Drop(target) => match target {
                GraphTarget::DefaultGraph => replace_default(graph)?,
                GraphTarget::NamedNode(n) => {
                    graph.drop_named_durable(&Term::NamedNode(n.clone()))?;
                }
                GraphTarget::NamedGraphs => drop_all_named_durable(graph)?,
                GraphTarget::AllGraphs => {
                    replace_default(graph)?;
                    drop_all_named_durable(graph)?;
                }
            },
            UpdateEffect::Create(name) => {
                graph.ensure_named(name)?;
            }
        }
    }
    // [OPUS-4.8] (sq-ycle) (4) The body is fully materialised into the per-graph state — clear the
    // redo journal so the next body starts fresh and a clean reopen is a no-op (no-op in-memory).
    graph.clear_txn()?;
    Ok(())
}

/// [OPUS-4.8] (sq-ycle) RESOLVE a captured `&[UpdateEffect]` into the flat, ORDERED quad-delta
/// that the atomic redo journal commits in ONE fsync — computed against the CURRENT `graph` state
/// so structural ops are concrete:
///   * `Delta { slot, inserts, deletes }` -> the deletes (false) then the inserts (true), the same
///     per-slot order [`apply_effects`] materialises them in.
///   * `Clear`/`Drop` -> scan the affected slot(s) NOW and emit a delete record (false) for every
///     triple PRESENT (default graph: scan `graph`; a NamedNode: scan that named sub-graph if it
///     exists; NamedGraphs: every named slot; AllGraphs: every named slot PLUS the default). The
///     data retraction is all the journal needs for atomicity — `Drop`'s manifest/sub-dir removal
///     stays a materialisation concern (already crash-safe via `recover_named_drop`).
///   * `Create(_)` -> nothing (an empty graph carries no triples).
///
/// The result is the per-slot quad-delta whose single-frame commit makes the WHOLE body atomic.
fn resolve_effect_records(graph: &Graph, effects: &[UpdateEffect]) -> Vec<(bool, GraphSlot, TripleTerms)> {
    let mut records: Vec<(bool, GraphSlot, TripleTerms)> = Vec::new();

    // [OPUS-4.8] (sq-aalh) A CLEAR/DROP retracts the triples PRESENT WHEN IT RUNS — i.e. the state
    // of its target slot AFTER every prior op in THIS body, not the pre-body state. Resolving it
    // against the pre-body `graph` (as the old code did) missed the retraction of triples inserted
    // EARLIER in the same body (e.g. `INSERT DATA { GRAPH X { … } } ; CLEAR GRAPH X`): the journal
    // recorded the insert with no matching delete, so a crash-recovery redo of the frame would
    // resurrect the inserted-then-cleared quads — the durable journal diverged from the (correct)
    // final in-memory state. PSS happens to order DROP before INSERT so it never hit this, but it
    // is a latent durability-correctness bug in general. Fix: walk the effects maintaining a RUNNING
    // per-slot view (seeded from the current `graph`), and resolve each CLEAR/DROP against THAT view.
    //
    // Set semantics: the redo journal replays records in append order via set-semantic `apply_delta`
    // (re-inserting a present triple / deleting an absent one is a no-op), so the running view need
    // only track membership. We model each slot as a `TripleSet` and emit retractions for exactly
    // the members present at the point the CLEAR/DROP runs. Slot insertion order is preserved so a
    // CLEAR NAMED / DROP ALL visits slots in a stable order.
    //
    // [OPUS-4.8] (sq-aalh, review #189) PERF: the running view is decoded LAZILY, per slot, ONLY when
    // an op actually touches that slot — `decode_triples` is the expensive part (a full scan of the
    // slot's store), and the common path is delta-only updates that never CLEAR/DROP. The previous
    // version eagerly decoded the default slot AND every named slot up-front, forcing a full-dataset
    // scan on every multi-op body even when no slot's pre-body contents are ever read. With lazy
    // seeding a delta-only body decodes nothing (a Delta seeds its slot then never reads pre-body
    // members back out for journalling); only CLEAR/DROP/Delta of a slot pays for that slot's decode.
    // CLEAR/DROP NAMED ALL still visits EVERY existing named graph (via `graph.named`) plus any slot
    // created intra-body — the same retraction set as before — but each is decoded on demand.
    let mut running: Vec<(GraphSlot, TripleSet)> = Vec::new();

    // Find the running view of a slot, SEEDING it on first access from the slot's CURRENT (pre-body)
    // decoded contents so a later CLEAR/DROP retracts pre-body triples too — not just intra-body
    // inserts. Decoding happens at most once per slot, the first time it is touched.
    fn slot_mut<'a>(
        graph: &Graph,
        running: &'a mut Vec<(GraphSlot, TripleSet)>,
        slot: &GraphSlot,
    ) -> &'a mut TripleSet {
        match running.iter().position(|(s, _)| s == slot) {
            Some(i) => &mut running[i].1,
            None => {
                let seed = match slot {
                    None => decode_triples(graph),
                    Some(name) => graph
                        .named
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, sub)| decode_triples(sub))
                        .unwrap_or_default(),
                };
                running.push((slot.clone(), seed));
                let i = running.len() - 1;
                &mut running[i].1
            }
        }
    }

    // Retract every triple currently in the running view of `slot`, then empty it — so a later op
    // in the same body sees the emptied slot.
    fn clear_slot(
        graph: &Graph,
        records: &mut Vec<(bool, GraphSlot, TripleTerms)>,
        running: &mut Vec<(GraphSlot, TripleSet)>,
        slot: &GraphSlot,
    ) {
        let view = slot_mut(graph, running, slot);
        for t in view.drain() {
            records.push((false, slot.clone(), t));
        }
    }

    // Every NAMED slot a `NamedGraphs`/`AllGraphs` CLEAR/DROP must retract: every named graph that
    // exists pre-body, UNION every named slot already created/touched intra-body, in a stable order
    // (pre-body graphs first, then any new intra-body slots in touch order). Decoding stays lazy —
    // this only collects the slot NAMES; `clear_slot` decodes each as it visits it.
    fn named_slots(graph: &Graph, running: &[(GraphSlot, TripleSet)]) -> Vec<GraphSlot> {
        let mut names: Vec<GraphSlot> =
            graph.named.iter().map(|(n, _)| Some(n.clone())).collect();
        for (slot, _) in running {
            if slot.is_some() && !names.contains(slot) {
                names.push(slot.clone());
            }
        }
        names
    }

    for effect in effects {
        match effect {
            UpdateEffect::Delta { slot, inserts, deletes } => {
                let view = slot_mut(graph, &mut running, slot);
                for t in deletes {
                    records.push((false, slot.clone(), t.clone()));
                    view.remove(t);
                }
                for t in inserts {
                    records.push((true, slot.clone(), t.clone()));
                    view.insert(t.clone());
                }
            }
            // CLEAR and DROP retract the SAME data set (every triple present in the target WHEN the
            // op runs); they differ only in DROP also removing the slot, which is a manifest/sub-dir
            // concern, not a journalled DATA change. So both resolve to the present-triple retractions
            // — computed against the RUNNING view so intra-body inserts are correctly retracted.
            UpdateEffect::Clear(target) | UpdateEffect::Drop(target) => match target {
                GraphTarget::DefaultGraph => clear_slot(graph, &mut records, &mut running, &None),
                GraphTarget::NamedNode(n) => {
                    clear_slot(graph, &mut records, &mut running, &Some(Term::NamedNode(n.clone())))
                }
                GraphTarget::NamedGraphs => {
                    for name in named_slots(graph, &running) {
                        clear_slot(graph, &mut records, &mut running, &name);
                    }
                }
                GraphTarget::AllGraphs => {
                    clear_slot(graph, &mut records, &mut running, &None);
                    for name in named_slots(graph, &running) {
                        clear_slot(graph, &mut records, &mut running, &name);
                    }
                }
            },
            // CREATE adds an empty graph — no triples to journal — but it makes the slot exist so a
            // later CLEAR NAMED / DROP ALL in the same body sees it (as an empty, no-op target).
            UpdateEffect::Create(name) => {
                slot_mut(graph, &mut running, &Some(name.clone()));
            }
        }
    }
    records
}

/// [OPUS-4.8] (review 1593) DURABLY empties the default graph, preserving the named graphs
/// AND (crucially) a directory-backed graph's WAL/directory association — so a CLEAR/DROP of
/// the default graph is persistent across a reopen, instead of being silently lost (the old
/// `*graph = empty_graph()` dropped the WAL, and the on-disk base was untouched, so a reopen
/// restored the cleared data). Falls back to an in-place store clear for in-memory graphs.
fn replace_default(graph: &mut Graph) -> Result<(), String> {
    graph.clear_default_durable()
}

/// [OPUS-4.8] (sq-glw2) DURABLY empties EVERY named graph (CLEAR NAMED / the named part of
/// CLEAR ALL), preserving each slot — mirrors a per-graph CLEAR GRAPH across the whole set. On a
/// directory-backed parent each sub-graph's contents are WAL-logged + fsync'd before the ack.
fn clear_all_named_durable(graph: &mut Graph) -> Result<(), String> {
    let names: Vec<Term> = graph.named.iter().map(|(n, _)| n.clone()).collect();
    for name in &names {
        graph.clear_named_durable(name)?;
    }
    Ok(())
}

/// [OPUS-4.8] (sq-glw2, Copilot #123) DURABLY removes EVERY named graph (DROP NAMED / the named
/// part of DROP ALL) in ONE batch. The old implementation looped, calling
/// [`Graph::drop_named_durable`] once per graph — but EACH such call rebuilds the entire surviving
/// named set (renumber + manifest rewrite + per-survivor save/re-open), making DROP ALL O(n²) in
/// the number of named graphs. [`Graph::drop_all_named_durable`] tears the whole sub-tree down in
/// a single manifest removal + single sub-tree removal (no survivors to renumber), which is O(n)
/// — one pass to release every sub-graph's handles, then two unlinks.
fn drop_all_named_durable(graph: &mut Graph) -> Result<(), String> {
    graph.drop_all_named_durable()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::empty_graph; // [OPUS-4.8] (sq-glw2) test-only now (see top-of-file note)
    use sparq_core::store::Pattern as IdPattern;

    fn count(g: &Graph) -> usize {
        let pat: IdPattern = [None, None, None];
        g.store.scan(&pat).rows.len()
    }

    // --- [OPUS-4.8] (sq-aalh) intra-body CLEAR/DROP durable-journal resolution ------------------
    //
    // The DURABLE multi-op path (`apply_effects`) resolves the redo-journal frame ONCE up-front via
    // `resolve_effect_records`. The bug: a CLEAR/DROP in the body resolved its retraction set
    // against the PRE-BODY graph, so a triple INSERTed earlier in the SAME body (e.g.
    // `INSERT DATA { GRAPH X { … } } ; CLEAR GRAPH X`) was journaled as an insert with NO matching
    // delete. The materialised per-graph state was correct (the per-effect loop runs in sequence),
    // but the JOURNAL FRAME diverged: a crash-recovery redo (open replays a committed-but-not-yet-
    // materialised frame) would resurrect the inserted-then-cleared quads. PSS orders DROP before
    // INSERT so it never hit this; the tests below prove it for the general case.

    /// Engine-internal helper: capture the resolved effect log of `sparql` applied to a SEPARATE
    /// in-memory working copy loaded from `dataset` (the EXACT effects the server batches into
    /// `apply_effects`, captured on the in-memory fork). The journal frame is then
    /// `resolve_effect_records(durable, &effects)` where `durable` starts from the same `dataset`.
    fn captured(dataset: &str, sparql: &str) -> Vec<UpdateEffect> {
        let mut working = Graph::load_dataset(dataset, "nquads").unwrap();
        update_in_place_capturing(&mut working, sparql, &crate::QueryBudget::unlimited()).unwrap()
    }

    /// The net per-slot membership a journal frame yields after a set-semantic redo (insert adds,
    /// delete removes, in append order) — i.e. exactly what `open` reconstructs from the frame.
    fn journal_net(records: &[(bool, GraphSlot, TripleTerms)]) -> Vec<(GraphSlot, TripleTerms)> {
        let mut set: Vec<(GraphSlot, TripleTerms)> = Vec::new();
        for (insert, slot, t) in records {
            let key = (slot.clone(), t.clone());
            if *insert {
                if !set.contains(&key) {
                    set.push(key);
                }
            } else {
                set.retain(|k| k != &key);
            }
        }
        set
    }

    /// REPRODUCE-then-FIX: `INSERT DATA { GRAPH X { … } } ; CLEAR GRAPH X` must journal to a NET-
    /// EMPTY frame (the inserted quads ARE retracted). Before the fix the CLEAR resolved against the
    /// pre-body (empty) X, so the frame's net was the inserted quad — a crash-redo would resurrect
    /// it. This asserts the journal, not just the materialised state.
    #[test]
    fn journal_intra_body_insert_then_clear_named_is_net_empty() {
        let base = Graph::load_dataset("", "nquads").unwrap();
        let sparql = "PREFIX : <http://ex/> INSERT DATA { GRAPH :x { :a :b :c } } ; CLEAR GRAPH :x";
        let effects = captured("", sparql);
        let frame = resolve_effect_records(&base, &effects);
        assert!(
            journal_net(&frame).is_empty(),
            "the durable journal frame must NET to empty (insert retracted by the intra-body CLEAR), \
             got {:?}",
            journal_net(&frame)
        );
    }

    /// Same for DROP: `INSERT DATA { GRAPH X { … } } ; DROP GRAPH X` → net-empty journal frame.
    #[test]
    fn journal_intra_body_insert_then_drop_named_is_net_empty() {
        let base = Graph::load_dataset("", "nquads").unwrap();
        let effects = captured("", "PREFIX : <http://ex/> INSERT DATA { GRAPH :x { :a :b :c } } ; DROP GRAPH :x");
        let frame = resolve_effect_records(&base, &effects);
        assert!(
            journal_net(&frame).is_empty(),
            "DROP after an intra-body INSERT must net-retract the inserted quad in the journal, got {:?}",
            journal_net(&frame)
        );
    }

    /// CONTROL (PSS-ordered): `DROP GRAPH X ; INSERT DATA { GRAPH X { … } }` — the DROP precedes the
    /// INSERT, so the journal frame's net is exactly the inserted quad (NOT broken by the fix).
    #[test]
    fn journal_drop_then_insert_keeps_inserted_quad() {
        let ds = "<http://ex/old> <http://ex/p> <http://ex/o> <http://ex/x> .";
        let base = Graph::load_dataset(ds, "nquads").unwrap();
        let effects = captured(ds, "PREFIX : <http://ex/> DROP GRAPH :x ; INSERT DATA { GRAPH :x { :a :b :c } }");
        let net = journal_net(&resolve_effect_records(&base, &effects));
        let x = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/x"));
        let want = [
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/a")),
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/b")),
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/c")),
        ];
        assert_eq!(net, vec![(Some(x), want)], "DROP-then-INSERT journals exactly the new quad");
    }

    /// MULTI-GRAPH: insert into X and Y, CLEAR only X → journal nets to Y's quad alone (X cleared,
    /// Y intact), proving the running-state resolution is per-slot.
    #[test]
    fn journal_clear_one_of_two_named_graphs() {
        let base = Graph::load_dataset("", "nquads").unwrap();
        let sparql = "PREFIX : <http://ex/> INSERT DATA { GRAPH :x { :a :b :c } GRAPH :y { :d :e :f } } ; CLEAR GRAPH :x";
        let effects = captured("", sparql);
        let net = journal_net(&resolve_effect_records(&base, &effects));
        let y = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/y"));
        let yf = [
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/d")),
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/e")),
            Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/f")),
        ];
        assert_eq!(net, vec![(Some(y), yf)], "only Y survives the X-only CLEAR in the journal");
    }

    /// END-TO-END crash-recovery: build the resolved frame, COMMIT it to a durable store's redo
    /// journal with NO materialisation, drop the handle (= crash right after the commit fsync), then
    /// REOPEN — the journal redo must reconstruct the FINAL state (X empty), not resurrect the
    /// inserted-then-cleared quad. This is the failure the bug describes; it FAILS pre-fix.
    #[cfg(feature = "parallel")] // mmap comes from the sparq-core dev-dep
    #[test]
    fn crash_recovery_intra_body_insert_then_clear_leaves_x_empty() {
        let dir = std::env::temp_dir().join(format!("sparq_aalh_crash_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // Pre-existing state: X already holds one quad (so we also prove the PRE-body quad IS
        // correctly retracted while the intra-body insert is too).
        let ds = "<http://ex/old> <http://ex/p> <http://ex/o> <http://ex/x> .";
        Graph::load_dataset(ds, "nquads").unwrap().save(&dir).unwrap();

        let sparql = "PREFIX : <http://ex/> INSERT DATA { GRAPH :x { :a :b :c } } ; CLEAR GRAPH :x";
        {
            let mut durable = Graph::open(&dir).unwrap();
            // Server flow: capture effects on an in-memory working copy (same start state), resolve
            // the frame against the durable graph's CURRENT state, and COMMIT it — the commit point.
            let effects = captured(ds, sparql);
            let frame = resolve_effect_records(&durable, &effects);
            durable.commit_txn(&frame).unwrap();
            // Drop WITHOUT materialising / clearing the journal: simulates a crash right after the
            // single commit fsync, before the per-graph WAL materialisation `apply_effects` does.
        }
        // Reopen: the journal redo reconstructs the committed frame. X must be EMPTY.
        let g = Graph::open(&dir).unwrap();
        // Match the EXACT graph IRI (not a brittle `contains("/x")`, which would also match `/xyz`
        // and depends on `Term` string formatting).
        let x_name = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/x"));
        let x = g.named.iter().find(|(n, _)| *n == x_name);
        let x_rows = x.map(|(_, sub)| sub.store.scan(&[None, None, None]).rows.len()).unwrap_or(0);
        assert_eq!(
            x_rows, 0,
            "after crash-recovery redo, X must be empty (pre-body AND intra-body quads retracted)"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// FULL-PATH parity: the realistic `apply_effects` call on a durable store, then RELOAD, leaves
    /// X empty — AND matches the pure in-memory `update` result. (apply_effects materialises +
    /// truncates the journal, so this passes both before and after the fix — it locks in the
    /// final-state contract that the journal fix must not regress.)
    #[cfg(feature = "parallel")]
    #[test]
    fn apply_effects_reload_parity_insert_then_clear() {
        let dir = std::env::temp_dir().join(format!("sparq_aalh_apply_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Graph::load_str("", "ntriples").unwrap().save(&dir).unwrap();
        let sparql =
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :x { :a :b :c } GRAPH :y { :d :e :f } } ; CLEAR GRAPH :x";

        {
            let mut durable = Graph::open(&dir).unwrap();
            let effects = captured("", sparql);
            apply_effects(&mut durable, &effects).unwrap();
        }
        let g = Graph::open(&dir).unwrap();
        // Match the EXACT graph IRIs (not a brittle `contains("/x")`/`contains("/y")`, which can
        // match other IRIs like `/xyz` and depends on `Term` string formatting).
        let x_name = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/x"));
        let y_name = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/y"));
        let x_rows = g
            .named
            .iter()
            .find(|(n, _)| *n == x_name)
            .map(|(_, s)| s.store.scan(&[None, None, None]).rows.len())
            .unwrap_or(0);
        let y_rows = g
            .named
            .iter()
            .find(|(n, _)| *n == y_name)
            .map(|(_, s)| s.store.scan(&[None, None, None]).rows.len())
            .unwrap_or(0);
        assert_eq!((x_rows, y_rows), (0, 1), "after reload: X empty, Y intact");

        // In-memory parity: `update` (rebuild path) must reach the identical final state.
        let mem = update(&Graph::load_str("", "ntriples").unwrap(), sparql).unwrap();
        let mx = mem.named.iter().find(|(n, _)| *n == x_name).map(|(_, s)| count(s)).unwrap_or(0);
        let my = mem.named.iter().find(|(n, _)| *n == y_name).map(|(_, s)| count(s)).unwrap_or(0);
        assert_eq!((mx, my), (0, 1), "in-memory final state matches the reloaded durable state");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn insert_delete_clear() {
        let g = Graph::load_str("@prefix : <http://ex/> . :a :p :b . :b :p :c .", "turtle").unwrap();
        assert_eq!(count(&g), 2);
        // INSERT DATA adds; set semantics (a re-insert is a no-op).
        let g = update(&g, "PREFIX : <http://ex/> INSERT DATA { :c :p :d . :a :q :x }").unwrap();
        assert_eq!(count(&g), 4);
        let g = update(&g, "PREFIX : <http://ex/> INSERT DATA { :a :p :b }").unwrap();
        assert_eq!(count(&g), 4);
        // DELETE DATA removes a present triple; deleting an absent one is a no-op.
        let g = update(&g, "PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();
        assert_eq!(count(&g), 3);
        let g = update(&g, "PREFIX : <http://ex/> DELETE DATA { :z :z :z }").unwrap();
        assert_eq!(count(&g), 3);
        // The graph is still queryable after a rebuild.
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { :c :p ?o }").unwrap(), 1);
        // CLEAR empties the default graph.
        let g = update(&g, "CLEAR ALL").unwrap();
        assert_eq!(count(&g), 0);
    }

    #[test]
    fn delete_insert_where() {
        let g = Graph::load_str("@prefix : <http://ex/> . :a :age 30 . :b :age 25 .", "turtle").unwrap();
        // Rename predicate :age -> :years for every match.
        let g = update(
            &g,
            "PREFIX : <http://ex/> DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a }",
        )
        .unwrap();
        assert_eq!(count(&g), 2);
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { ?s :age ?a }").unwrap(), 0);
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { ?s :years ?a }").unwrap(), 2);
        // DELETE WHERE shorthand removes all matches.
        let g = update(&g, "PREFIX : <http://ex/> DELETE WHERE { ?s :years ?a }").unwrap();
        assert_eq!(count(&g), 0);
    }

    /// F19 regression: updates must PRESERVE named graphs, route GRAPH-scoped data ops,
    /// and implement the graph-management operations.
    #[test]
    fn named_graph_updates() {
        let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .";
        let g = Graph::load_dataset(src, "nquads").unwrap();
        assert_eq!((count(&g), g.named.len()), (1, 1));

        // A default-graph INSERT DATA must keep :g1 intact.
        let g = update(&g, "PREFIX : <http://ex/> INSERT DATA { :a :p :c }").unwrap();
        assert_eq!((count(&g), g.named.len()), (2, 1));
        assert_eq!(count(&g.named[0].1), 1);

        // GRAPH-scoped INSERT DATA goes to the right graph (auto-creating :g2).
        let g = update(&g, "PREFIX : <http://ex/> INSERT DATA { GRAPH :g1 { :x :q :z } GRAPH :g2 { :n :m :o } }").unwrap();
        assert_eq!(g.named.len(), 2);
        assert_eq!(count(&g.named[0].1), 2);

        // GRAPH-scoped DELETE DATA.
        let g = update(&g, "PREFIX : <http://ex/> DELETE DATA { GRAPH :g1 { :x :q :y } }").unwrap();
        assert_eq!(count(&g.named[0].1), 1);

        // DELETE/INSERT WHERE with GRAPH templates + GRAPH WHERE (the ADD desugaring shape).
        let g = update(
            &g,
            "PREFIX : <http://ex/> INSERT { GRAPH :g3 { ?s ?p ?o } } WHERE { GRAPH :g2 { ?s ?p ?o } }",
        )
        .unwrap();
        let g3 = g.named.iter().find(|(n, _)| n.to_string().contains("g3")).unwrap();
        assert_eq!(count(&g3.1), 1);

        // CLEAR GRAPH empties only that graph; DROP removes it.
        let g = update(&g, "PREFIX : <http://ex/> CLEAR GRAPH :g2").unwrap();
        assert_eq!(g.named.len(), 3); // entry kept, empty
        let g2 = g.named.iter().find(|(n, _)| n.to_string().contains("g2")).unwrap();
        assert_eq!(count(&g2.1), 0);
        let g = update(&g, "PREFIX : <http://ex/> DROP GRAPH :g3").unwrap();
        assert_eq!(g.named.len(), 2);

        // CLEAR DEFAULT keeps named graphs.
        let g = update(&g, "CLEAR DEFAULT").unwrap();
        assert_eq!(count(&g), 0);
        assert_eq!(g.named.len(), 2);
        assert_eq!(count(&g.named[0].1), 1);

        // CREATE makes an (empty) graph that GRAPH <g> {} can see.
        let g = update(&g, "PREFIX : <http://ex/> CREATE GRAPH :fresh").unwrap();
        assert!(g.named.iter().any(|(n, _)| n.to_string().contains("fresh")));

        // DROP ALL empties everything.
        let g = update(&g, "DROP ALL").unwrap();
        assert_eq!((count(&g), g.named.len()), (0, 0));
    }

    /// ADD / COPY / MOVE arrive desugared from the parser; end-to-end they must move the
    /// triples and preserve everything else.
    #[test]
    fn add_copy_move() {
        let src = "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
                   <http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .";
        let g = Graph::load_dataset(src, "nquads").unwrap();
        // ADD the default graph into :g1 (union).
        let g = update(&g, "ADD DEFAULT TO GRAPH <http://ex/g1>").unwrap();
        assert_eq!(count(&g), 1); // source kept
        assert_eq!(count(&g.named[0].1), 2);
        // COPY replaces the destination.
        let g = update(&g, "COPY DEFAULT TO GRAPH <http://ex/g1>").unwrap();
        assert_eq!(count(&g.named.iter().find(|(n, _)| n.to_string().contains("g1")).unwrap().1), 1);
        // MOVE drops the source.
        let g = update(&g, "MOVE GRAPH <http://ex/g1> TO GRAPH <http://ex/g2>").unwrap();
        assert!(!g.named.iter().any(|(n, _)| n.to_string().contains("g1")));
        assert_eq!(count(&g.named.iter().find(|(n, _)| n.to_string().contains("g2")).unwrap().1), 1);
    }

    /// USING re-scopes the WHERE dataset: the named graph becomes the active default graph.
    #[test]
    fn using_rescopes_where() {
        let src = "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
                   <http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .";
        let g = Graph::load_dataset(src, "nquads").unwrap();
        let g = update(
            &g,
            "PREFIX : <http://ex/> INSERT { ?s :copied ?o } USING :g1 WHERE { ?s :p ?o }",
        )
        .unwrap();
        // Only :g1's triple matched (the real default graph was re-scoped away).
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { ?s :copied ?o }").unwrap(), 1);
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { :a :copied :b }").unwrap(), 1);
    }

    /// Blank nodes in an INSERT template are FRESH per solution.
    #[test]
    fn insert_template_bnodes_fresh_per_solution() {
        let g = Graph::load_str("@prefix : <http://ex/> . :a :age 30 . :b :age 25 .", "turtle").unwrap();
        let g = update(
            &g,
            "PREFIX : <http://ex/> INSERT { ?s :note _:n . _:n :label \"x\" } WHERE { ?s :age ?a }",
        )
        .unwrap();
        // 2 solutions × 2 template triples = 4 inserted (distinct bnodes per solution).
        assert_eq!(count(&g), 2 + 4);
        // Two DISTINCT bnodes -> two :label triples.
        assert_eq!(crate::count(&g, "PREFIX : <http://ex/> SELECT * WHERE { ?n :label ?l }").unwrap(), 2);
    }

    /// The delta-overlay path (`update_in_place`) must be observationally identical to the
    /// rebuild path (`update`) across a sequence of every supported operation — and the
    /// overlay-carrying graph must answer queries identically without compaction.
    #[test]
    fn update_in_place_matches_rebuild() {
        let src = "@prefix : <http://ex/> . :a :p :b . :b :p :c . :a :age 30 . :b :age 25 .";
        let mut inplace = Graph::load_str(src, "turtle").unwrap();
        let mut rebuilt = Graph::load_str(src, "turtle").unwrap();
        let ops = [
            "PREFIX : <http://ex/> INSERT DATA { :c :p :d . :a :q 42 . :a :q 9.5 }",
            "PREFIX : <http://ex/> DELETE DATA { :a :p :b . :z :z :z }",
            "PREFIX : <http://ex/> INSERT DATA { :a :p :b }", // re-insert of a delete
            "PREFIX : <http://ex/> DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a }",
            "PREFIX : <http://ex/> DELETE WHERE { ?s :q ?v }",
            // Named-graph operations (F19): both paths must route + preserve identically.
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :g1 { :n :m :o . :n :m :p } }",
            "PREFIX : <http://ex/> DELETE DATA { GRAPH :g1 { :n :m :p } }",
            "PREFIX : <http://ex/> INSERT { GRAPH :g2 { ?s ?p ?o } } WHERE { GRAPH :g1 { ?s ?p ?o } }",
            "PREFIX : <http://ex/> CLEAR GRAPH :g1",
            "PREFIX : <http://ex/> DROP GRAPH :g2",
        ];
        let dump = |g: &Graph| -> Vec<(String, String, String, String)> {
            let mut v: Vec<_> = Vec::new();
            let mut one = |g: &Graph, name: &str| {
                let scan = g.store.scan(&[None, None, None]);
                for r in scan.rows.iter() {
                    let t = scan.to_spo(r);
                    v.push((
                        g.dict.term(t[0]).to_string(),
                        g.dict.term(t[1]).to_string(),
                        g.dict.term(t[2]).to_string(),
                        name.to_string(),
                    ));
                }
            };
            one(g, "");
            for (name, sub) in &g.named {
                one(sub, &name.to_string());
            }
            v.sort();
            v
        };
        for (i, op) in ops.iter().enumerate() {
            update_in_place(&mut inplace, op).unwrap();
            rebuilt = update(&rebuilt, op).unwrap();
            assert_eq!(dump(&inplace), dump(&rebuilt), "states diverged after op {i}");
            // The overlay graph must also answer pattern queries identically (un-compacted).
            for q in [
                "PREFIX : <http://ex/> SELECT * WHERE { ?s :p ?o }",
                "PREFIX : <http://ex/> SELECT * WHERE { ?s ?p ?o . FILTER(?o > 20) }",
                "PREFIX : <http://ex/> SELECT * WHERE { ?s :p ?m . ?m :p ?o }",
                "PREFIX : <http://ex/> SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }",
            ] {
                assert_eq!(crate::count(&inplace, q).unwrap(), crate::count(&rebuilt, q).unwrap(), "query {q} diverged after op {i}");
            }
        }
        assert!(inplace.store.has_overlay(), "the in-place path must have gone through the overlay");
        // CLEAR still empties via replacement.
        update_in_place(&mut inplace, "CLEAR ALL").unwrap();
        assert_eq!(dump(&inplace).len(), 0);
        // And compaction folds without changing anything.
        let mut g = Graph::load_str(src, "turtle").unwrap();
        update_in_place(&mut g, ops[0]).unwrap();
        let before = dump(&g);
        g.compact().unwrap();
        assert!(!g.store.has_overlay());
        assert_eq!(dump(&g), before);
    }

    /// HEADLINE BENCHMARK (T17): per-update latency of a 10-triple INSERT DATA into a
    /// 10M-triple graph — the old decode-everything-and-rebuild path vs the new delta-
    /// overlay path. Run with:
    ///   cargo test -p sparq-engine --release -- --ignored bench_update_latency --nocapture
    /// Override the size with SPARQ_BENCH_TRIPLES.
    #[test]
    #[ignore = "benchmark — run explicitly in --release"]
    fn bench_update_latency() {
        use sparq_core::dict::{Dict as D, Id};
        use std::time::Instant;
        let n: usize =
            std::env::var("SPARQ_BENCH_TRIPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000_000);
        let nsubj = (n / 10).max(1);
        let t0 = Instant::now();
        let mut dict = D::new();
        let preds: Vec<Id> = (0..10).map(|p| dict.intern_iri(&format!("http://ex/p{p}"))).collect();
        let subjs: Vec<Id> = (0..nsubj).map(|s| dict.intern_iri(&format!("http://ex/s{s}"))).collect();
        // NON-LINEAR mix (murmur3 finalizer) for the object index, so the triple set has
        // ~n DISTINCT triples — any linear pattern is periodic mod nsubj and collapses
        // under dedup.
        let mix = |i: usize| -> usize {
            let mut h = i as u64;
            h ^= h >> 33;
            h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
            h ^= h >> 33;
            (h % nsubj as u64) as usize
        };
        let triples: Vec<[Id; 3]> =
            (0..n).map(|i| [subjs[i % nsubj], preds[i % 10], subjs[mix(i)]]).collect();
        let g = Graph::from_parts(dict, triples);
        eprintln!("graph: {} triples, built in {:.2?}", g.len(), t0.elapsed());

        let ins = "PREFIX : <http://ex/> INSERT DATA { :n0 :q :m0 . :n1 :q :m1 . :n2 :q :m2 . \
                   :n3 :q :m3 . :n4 :q :m4 . :n5 :q :m5 . :n6 :q :m6 . :n7 :q :m7 . :n8 :q :m8 . :n9 :q :m9 }";

        // OLD: full rebuild.
        let t = Instant::now();
        let g_old = update(&g, ins).unwrap();
        let old = t.elapsed();
        eprintln!("old (rebuild) 10-triple INSERT DATA: {:.2?}  -> {} triples", old, g_old.len());
        drop(g_old);

        // NEW: delta-overlay, measured over several batches for a stable per-update figure.
        let mut g = g;
        let t = Instant::now();
        update_in_place(&mut g, ins).unwrap();
        let new_first = t.elapsed();
        let t = Instant::now();
        let reps = 100;
        for i in 0..reps {
            let upd = format!("PREFIX : <http://ex/> INSERT DATA {{ {} }}",
                (0..10).map(|j| format!(":x{i}_{j} :q :y{i}_{j} .")).collect::<Vec<_>>().join(" "));
            update_in_place(&mut g, &upd).unwrap();
        }
        let new_avg = t.elapsed() / reps;
        eprintln!(
            "new (overlay) 10-triple INSERT DATA: first {:.2?}, avg over {reps} batches {:.2?}  -> {} triples",
            new_first, new_avg, g.len()
        );
        eprintln!("speedup: {:.0}x (rebuild {:.3}s vs overlay {:.6}s)",
            old.as_secs_f64() / new_avg.as_secs_f64(), old.as_secs_f64(), new_avg.as_secs_f64());
    }

    /// [OPUS-4.8] roborev 1646 (High): `LOAD <file://…>` must be REFUSED by default — an
    /// untrusted update on the public path cannot import server-local files. It only works
    /// inside an allowlisted base, and even then cannot escape it via `..`.
    #[test]
    fn load_file_uri_refused_by_default_and_sandboxed() {
        let dir = std::env::temp_dir().join(format!("sparq_load_1646_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let inside = dir.join("data.ttl");
        std::fs::write(&inside, "@prefix : <http://ex/> . :a :p :b .").unwrap();
        // A secret file OUTSIDE the allowlisted base (one level up).
        let secret = dir.parent().unwrap().join(format!("sparq_secret_1646_{}.ttl", std::process::id()));
        std::fs::write(&secret, "@prefix : <http://ex/> . :secret :leaked :value .").unwrap();

        let g = empty_graph();
        let load_inside = format!("LOAD <file://{}>", inside.display());
        // `Result<Graph, String>::unwrap_err` would need Graph: Debug — extract the error string.
        let err = |r: Result<Graph, String>| -> String { r.err().expect("expected a LOAD error") };

        // (1) DEFAULT: no allowlist installed -> refused (the file is never read).
        let e = err(update(&g, &load_inside));
        assert!(e.contains("file:// access is disabled"), "default LOAD must be refused, got: {e}");
        // SILENT swallows the refusal into a no-op (still loads nothing).
        let g_silent = update(&g, &format!("LOAD SILENT <file://{}>", inside.display())).unwrap();
        assert_eq!(count(&g_silent), 0);

        // (2) Allowlisted base: a file UNDER the base loads.
        let loaded = with_load_base(dir.clone(), || update(&g, &load_inside)).unwrap();
        assert_eq!(count(&loaded), 1, "LOAD under the allowlisted base must succeed");

        // (3) A path that escapes the base (absolute, outside) is refused even with a base set.
        let load_secret = format!("LOAD <file://{}>", secret.display());
        let e = err(with_load_base(dir.clone(), || update(&g, &load_secret)));
        assert!(e.contains("outside the allowlisted base"), "escape must be refused, got: {e}");
        // …and a `..` traversal to the same secret is likewise refused.
        let rel = format!("LOAD <file://../{}>", secret.file_name().unwrap().to_str().unwrap());
        let e = err(with_load_base(dir.clone(), || update(&g, &rel)));
        assert!(e.contains("outside the allowlisted base") || e.contains("LOAD"), "traversal must be refused, got: {e}");

        // (4) The base does not leak past the scope: a subsequent LOAD is refused again.
        let e = err(update(&g, &load_inside));
        assert!(e.contains("file:// access is disabled"), "base must not leak, got: {e}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&secret);
    }

    /// [OPUS-4.8] roborev 1646 (Med): blank nodes in INSERT DATA are FRESH per SPARQL — they
    /// must not conflate with an existing store blank node of the same label. The rebuild and
    /// delta-overlay paths must both freshen.
    #[test]
    fn insert_data_blank_nodes_are_fresh() {
        // Graph already holds a blank node labelled `b` (subject of one triple).
        let src = "@prefix : <http://ex/> . _:b :p :existing .";
        let ins = "PREFIX : <http://ex/> INSERT DATA { _:b :p :inserted }";

        // Rebuild path: the inserted `_:b` must be a DIFFERENT node, so :p has two distinct
        // blank-node subjects and the two objects stay on separate subjects.
        let g = Graph::load_str(src, "turtle").unwrap();
        let g = update(&g, ins).unwrap();
        assert_eq!(count(&g), 2, "both triples present");
        let subjects = crate::count(&g, "SELECT DISTINCT ?s WHERE { ?s <http://ex/p> ?o }").unwrap();
        assert_eq!(subjects, 2, "INSERT DATA bnode must be fresh (distinct from existing _:b)");

        // Delta-overlay path: same freshness guarantee.
        let mut g2 = Graph::load_str(src, "turtle").unwrap();
        update_in_place(&mut g2, ins).unwrap();
        assert_eq!(count(&g2), 2);
        let subjects2 = crate::count(&g2, "SELECT DISTINCT ?s WHERE { ?s <http://ex/p> ?o }").unwrap();
        assert_eq!(subjects2, 2, "in-place INSERT DATA bnode must be fresh too");

        // Within ONE INSERT DATA op, the same label is the SAME fresh node.
        let g3 = Graph::load_str("@prefix : <http://ex/> . :x :p :y .", "turtle").unwrap();
        let g3 = update(&g3, "PREFIX : <http://ex/> INSERT DATA { _:s :a :o . _:s :b :o2 }").unwrap();
        let one_subj = crate::count(&g3, "SELECT DISTINCT ?s WHERE { ?s ?p ?o . FILTER(isBlank(?s)) }").unwrap();
        assert_eq!(one_subj, 1, "same label in one INSERT DATA op is one node");
    }
}

/// [OPUS-4.8] (sq-qu8o) Pins the SPARQL 1.1 Update SILENT / non-SILENT error contract per
/// operation AND request-level atomicity, against the W3C SPARQL 1.1 Update Recommendation.
///
/// IMPORTANT — read before changing an assertion here. Two findings this module DOCUMENTS
/// (neither is a bug; both are spec-conformant choices that diverge from a naive "must error"
/// reading, so they are written down rather than left implicit):
///
///  1. **CLEAR / DROP of an absent graph, and CREATE of an existing one, SUCCEED here whether or
///     not SILENT is present.** The spec's failure clauses for these are normative **SHOULD**,
///     not MUST, and are explicitly conditioned on whether the store *records the existence of
///     empty graphs* (§3.1.3 CLEAR, §3.1.4 DROP, §3.1.5 CREATE: "If the store records the
///     existence of empty graphs … SHOULD return failure"). sparq is an auto-creating graph
///     store — `Dataset::graph_mut` materialises an absent named graph on demand and CLEAR keeps
///     empty entries — so the SHOULD's precondition does not hold and success is conformant. The
///     consequence is that `SILENT` is a **no-op distinction** for CLEAR/DROP/CREATE (the flag is
///     accepted by the parser but the outcome is success either way). The ONE operation where
///     SILENT genuinely changes the outcome is **LOAD** (§3.1.2): a failing dereference/parse is
///     an error without SILENT and a success no-op with it. The table below asserts that exact
///     split so a future change that makes CLEAR/DROP/CREATE start erroring (or makes LOAD stop
///     honouring SILENT) trips a test instead of silently changing the public contract.
///
///  2. **Request-level atomicity differs by entry point — by design, and the layering is the
///     point.** [`update`] (the rebuild path) is atomic *by construction*: it decodes a private
///     working `Dataset`, applies every op to it, and only returns the rebuilt graph if ALL ops
///     succeed — a mid-request failure drops the working copy and the borrowed input is untouched
///     (full rollback). [`update_in_place`] (the delta-overlay path) is, by its documented
///     contract (`sparq_serve::ApplyUpdates::apply`: "On `Err` the working copy may hold a
///     partially applied prefix"), NOT atomic on its own — request-level atomicity for the
///     in-place path is the caller's responsibility and is provided in production by the serve
///     writer's fork → apply → seal-only-on-`Ok` recovery. The atomicity tests below pin all
///     three: rebuild rolls back fully; in-place leaves a partial prefix on failure (the hazard,
///     captured so a regression that "fixes" it in place is a deliberate decision); and the
///     fork-and-discard pattern recovers full atomicity. The shape mirrors sparq-core's
///     `wal_torn_multi_record_batch_is_all_or_nothing` — failed multi-record unit applies nothing.
#[cfg(test)]
mod update_contract {
    use super::*;
    use sparq_core::store::Pattern as IdPattern;

    /// Total triple count across the default graph and every named graph — the dataset-wide
    /// "did anything change" probe used by the atomicity assertions.
    fn dataset_count(g: &Graph) -> usize {
        let pat: IdPattern = [None, None, None];
        g.store.scan(&pat).rows.len() + g.named.iter().map(|(_, sub)| sub.store.scan(&pat).rows.len()).sum::<usize>()
    }

    /// A canonical dataset dump (sorted (s,p,o,graph) strings) for exact PRE/POST-request
    /// equality — the "rolled back to the pre-request snapshot" assertion. Mirrors the dump
    /// helper in `update_in_place_matches_rebuild`.
    fn dump(g: &Graph) -> Vec<(String, String, String, String)> {
        let mut v: Vec<(String, String, String, String)> = Vec::new();
        let mut one = |g: &Graph, name: &str| {
            let scan = g.store.scan(&[None, None, None]);
            for r in scan.rows.iter() {
                let t = scan.to_spo(r);
                v.push((
                    g.dict.term(t[0]).to_string(),
                    g.dict.term(t[1]).to_string(),
                    g.dict.term(t[2]).to_string(),
                    name.to_string(),
                ));
            }
        };
        one(g, "");
        for (name, sub) in &g.named {
            one(sub, &name.to_string());
        }
        v.sort();
        v
    }

    /// A store with one named graph `:g1` (and an empty default graph) — the fixture for the
    /// graph-management contract rows. Returned fresh per row so rows never interfere.
    fn store() -> Graph {
        Graph::load_dataset("<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .", "nquads").unwrap()
    }

    // ------------------------------------------------------------------------------------------
    // 1. SILENT / non-SILENT outcome table — one row per applicable operation, each anchored by
    //    the OPPOSITE (valid) case, with the governing spec clause cited inline.
    // ------------------------------------------------------------------------------------------

    /// CLEAR (§3.1.3). Non-existent target: spec SHOULD-error is *conditioned* on a store that
    /// records empty graphs — sparq auto-creates, so CLEAR of an absent graph is a no-op SUCCESS
    /// with AND without SILENT. Valid anchor: CLEARing an existing graph empties it (success).
    #[test]
    fn contract_clear() {
        // Absent target — success either way (auto-create store; SHOULD precondition unmet).
        assert!(update(&store(), "CLEAR GRAPH <http://ex/absent>").is_ok(), "CLEAR absent (non-SILENT) is a no-op success here");
        assert!(update(&store(), "CLEAR SILENT GRAPH <http://ex/absent>").is_ok(), "CLEAR SILENT absent is a no-op success");
        // Anchor (valid): CLEAR of the existing :g1 succeeds and empties it.
        let g = update(&store(), "CLEAR GRAPH <http://ex/g1>").unwrap();
        let g1 = g.named.iter().find(|(n, _)| n.to_string().contains("g1")).expect("entry kept");
        assert_eq!(g1.1.store.scan(&[None, None, None]).rows.len(), 0, "CLEAR empties the existing graph");
    }

    /// DROP (§3.1.4). Same SHOULD-conditioned-on-empty-graph-recording story as CLEAR: DROP of an
    /// absent graph is a no-op SUCCESS with/without SILENT. Valid anchor: DROP of :g1 removes it.
    #[test]
    fn contract_drop() {
        assert!(update(&store(), "DROP GRAPH <http://ex/absent>").is_ok(), "DROP absent (non-SILENT) is a no-op success here");
        assert!(update(&store(), "DROP SILENT GRAPH <http://ex/absent>").is_ok(), "DROP SILENT absent is a no-op success");
        // Anchor (valid): DROP of the existing :g1 removes its entry.
        let g = update(&store(), "DROP GRAPH <http://ex/g1>").unwrap();
        assert!(!g.named.iter().any(|(n, _)| n.to_string().contains("g1")), "DROP removes the existing graph entry");
    }

    /// CREATE (§3.1.5). Existing target: spec SHOULD-error, again conditioned on recording empty
    /// graphs — sparq's CREATE is idempotent, so CREATE of an existing graph is a no-op SUCCESS
    /// with/without SILENT. Valid anchor: CREATE of a fresh graph adds a queryable empty entry.
    #[test]
    fn contract_create() {
        assert!(update(&store(), "CREATE GRAPH <http://ex/g1>").is_ok(), "CREATE existing (non-SILENT) is a no-op success here");
        assert!(update(&store(), "CREATE SILENT GRAPH <http://ex/g1>").is_ok(), "CREATE SILENT existing is a no-op success");
        // Anchor (valid): CREATE of a fresh graph materialises an (empty) entry.
        let g = update(&store(), "CREATE GRAPH <http://ex/fresh>").unwrap();
        assert!(g.named.iter().any(|(n, _)| n.to_string().contains("fresh")), "CREATE adds the new graph");
    }

    /// LOAD (§3.1.2) — the ONE operation where SILENT actually flips the outcome. A failing LOAD
    /// (here: a non-`file://` scheme, which `load_document` rejects; and a `file://` with no
    /// allowlisted base, also rejected — see `load_file_uri_refused_by_default_and_sandboxed`)
    /// is an ERROR without SILENT and a SUCCESS no-op (loads nothing) with SILENT.
    #[test]
    fn contract_load_silent_is_the_real_lever() {
        // Unsupported scheme: hard error without SILENT.
        let e = update(&store(), "LOAD <http://ex/data.ttl>");
        assert!(e.is_err(), "non-SILENT LOAD of an unfetchable source must ERROR (§3.1.2)");
        // SILENT swallows the failure into a no-op success that changes nothing.
        let before = dump(&store());
        let g = update(&store(), "LOAD SILENT <http://ex/data.ttl>").expect("LOAD SILENT failure is success");
        assert_eq!(dump(&g), before, "LOAD SILENT failure loads nothing (no-op success)");
        // file:// with no allowlisted base is likewise refused without SILENT, no-op with it.
        assert!(update(&store(), "LOAD <file:///etc/hostname>").is_err(), "file:// LOAD refused by default (non-SILENT errors)");
        assert!(update(&store(), "LOAD SILENT <file:///etc/hostname>").is_ok(), "file:// LOAD refusal is swallowed by SILENT");
    }

    /// ADD / MOVE / COPY with source == destination (§3.2). The parser desugars the identity case
    /// to an EMPTY operation list, so it is a no-op SUCCESS that leaves the source graph intact —
    /// regardless of SILENT. Valid anchor: a real MOVE between distinct graphs relocates triples.
    #[test]
    fn contract_add_move_copy_self_is_noop() {
        for verb in ["ADD", "MOVE", "COPY"] {
            let req = format!("{verb} GRAPH <http://ex/g1> TO GRAPH <http://ex/g1>");
            let g = update(&store(), &req).unwrap_or_else(|e| panic!("{verb} self must be a no-op success, got: {e}"));
            // The source/destination graph is untouched (its one triple survives).
            let g1 = g.named.iter().find(|(n, _)| n.to_string().contains("g1")).expect("g1 preserved");
            assert_eq!(g1.1.store.scan(&[None, None, None]).rows.len(), 1, "{verb} self leaves the graph unchanged");
            // SILENT identity is also a success no-op.
            let req_s = format!("{verb} SILENT GRAPH <http://ex/g1> TO GRAPH <http://ex/g1>");
            assert!(update(&store(), &req_s).is_ok(), "{verb} SILENT self is a no-op success");
        }
        // Anchor (valid): MOVE between DISTINCT graphs relocates the triple and drops the source.
        let g = update(&store(), "MOVE GRAPH <http://ex/g1> TO GRAPH <http://ex/g2>").unwrap();
        assert!(!g.named.iter().any(|(n, _)| n.to_string().contains("g1")), "MOVE drops the source");
        let g2 = g.named.iter().find(|(n, _)| n.to_string().contains("g2")).expect("destination created");
        assert_eq!(g2.1.store.scan(&[None, None, None]).rows.len(), 1, "MOVE delivered the triple");
    }

    // ------------------------------------------------------------------------------------------
    // 2. Request-level atomicity. A SPARQL Update *request* is a sequence of operations separated
    //    by `;`; if operation K fails, the WHOLE request must roll back. Mirrors the all-or-nothing
    //    shape of sparq-core's `wal_torn_multi_record_batch_is_all_or_nothing`.
    // ------------------------------------------------------------------------------------------

    /// REBUILD path ([`update`]): a multi-op request whose later op fails must leave the dataset
    /// EXACTLY at its pre-request snapshot (full rollback); an all-succeed request commits fully.
    #[test]
    fn rebuild_request_is_atomic() {
        let base = Graph::load_str("@prefix : <http://ex/> . :seed :p :o .", "turtle").unwrap();
        let snapshot = dump(&base);
        assert_eq!(dataset_count(&base), 1);

        // A request: a VALID INSERT DATA, then a non-SILENT LOAD that fails (op K = 2).
        let failing = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; LOAD <http://ex/nope.ttl>";
        let r = update(&base, failing);
        assert!(r.is_err(), "the request must fail (the LOAD op errors)");
        // `update` borrows `base` and rebuilds a private copy, so the input is provably untouched;
        // the *returned* (would-be-new) graph is never produced — full rollback.
        assert_eq!(dump(&base), snapshot, "a failed multi-op request rolls the dataset back to its pre-request snapshot");
        assert_eq!(dataset_count(&base), 1, "the valid INSERT DATA prefix must NOT have committed");

        // The all-succeed counterpart commits every operation.
        let ok = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; INSERT DATA { :c :p :d }";
        let g = update(&base, ok).expect("an all-succeed request commits");
        assert_eq!(dataset_count(&g), 3, "an all-succeed request commits every operation");
    }

    /// IN-PLACE path ([`update_in_place`]): by its DOCUMENTED contract it is NOT atomic on its own
    /// — a failing later op leaves the earlier ops' partial prefix applied. This is the hazard the
    /// serve writer recovers from; pinning it means any change that makes the raw call atomic (or
    /// makes it leak a different partial state) is a deliberate, reviewed decision.
    #[test]
    fn in_place_request_is_partial_on_error_by_contract() {
        let mut g = Graph::load_str("@prefix : <http://ex/> . :seed :p :o .", "turtle").unwrap();
        let before = dataset_count(&g);
        let failing = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; LOAD <http://ex/nope.ttl>";
        let r = update_in_place(&mut g, failing);
        assert!(r.is_err(), "the request still reports the failure");
        // The INSERT DATA prefix committed before the LOAD failed — documented non-atomicity.
        assert_eq!(dataset_count(&g), before + 1, "in-place leaves the pre-failure prefix applied (documented contract)");
    }

    /// The PRODUCTION request-level-atomicity pattern for the in-place path: fork a private
    /// working copy, apply the request to it, and publish (here: keep) the working copy ONLY on
    /// `Ok` — on `Err` discard it and keep the original. This is exactly what the serve writer
    /// does (fork → `update_in_place` → seal-only-on-success), and it recovers full atomicity.
    #[test]
    fn fork_and_seal_recovers_request_atomicity() {
        let base = Graph::load_str("@prefix : <http://ex/> . :seed :p :o .", "turtle").unwrap();
        let snapshot = dump(&base);
        let failing = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; LOAD <http://ex/nope.ttl>";

        // Apply to a fork; the failing request leaves the fork partial — but we never seal it.
        let mut working = base.fork();
        let published = match update_in_place(&mut working, failing) {
            Ok(()) => working,        // would seal the new state
            Err(_) => base.fork(),    // discard the partial fork, re-fork from the untouched base
        };
        assert_eq!(dump(&published), snapshot, "fork-and-discard on error publishes the pre-request snapshot (atomic)");

        // The all-succeed counterpart seals the new state.
        let ok = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; INSERT DATA { :c :p :d }";
        let mut working = base.fork();
        let published = match update_in_place(&mut working, ok) {
            Ok(()) => working,
            Err(_) => base.fork(),
        };
        assert_eq!(dataset_count(&published), 3, "an all-succeed request seals the fully-applied working copy");
        // The base the fork came from is, throughout, untouched by either request.
        assert_eq!(dump(&base), snapshot, "the published base is never mutated by a working-copy apply");
    }

    /// [OPUS-4.8] (sq-o1wp) [`update_in_place_atomic`] packages the fork → apply → seal-only-on-`Ok`
    /// pattern as a safe public default, so a DIRECT library consumer gets SPARQL-1.1 all-or-nothing
    /// request semantics WITHOUT writing its own recovery. Pins both arms: a failing multi-op request
    /// leaves `graph` exactly at its pre-request snapshot (full rollback — the hazard the bare
    /// `update_in_place` leaks per `in_place_request_is_partial_on_error_by_contract`), and an
    /// all-succeed request commits every operation in place.
    #[test]
    fn in_place_atomic_rolls_back_whole_request_on_error() {
        let mut g = Graph::load_str("@prefix : <http://ex/> . :seed :p :o .", "turtle").unwrap();
        let snapshot = dump(&g);
        assert_eq!(dataset_count(&g), 1);

        // A request: a VALID INSERT DATA, then a non-SILENT LOAD that fails (op K = 2).
        let failing = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; LOAD <http://ex/nope.ttl>";
        let r = update_in_place_atomic(&mut g, failing);
        assert!(r.is_err(), "the request reports the failure");
        // Unlike the bare in-place path, the valid INSERT DATA prefix is NOT committed.
        assert_eq!(dump(&g), snapshot, "atomic in-place rolls the dataset back to its pre-request snapshot");
        assert_eq!(dataset_count(&g), 1, "the pre-failure prefix must NOT have committed in place");

        // The all-succeed counterpart commits every operation in place.
        let ok = "PREFIX : <http://ex/> INSERT DATA { :a :p :b } ; INSERT DATA { :c :p :d }";
        update_in_place_atomic(&mut g, ok).expect("an all-succeed request commits in place");
        assert_eq!(dataset_count(&g), 3, "an all-succeed request commits every operation in place");
    }

    /// [OPUS-4.8] (sq-o1wp) A parse error never mutates `graph` under the atomic wrapper (the fork is
    /// built but `parse_update` fails before any op applies), and the budgeted entry point shares the
    /// same all-or-nothing contract.
    #[test]
    fn in_place_atomic_parse_error_and_budget_entry_point() {
        let mut g = Graph::load_str("@prefix : <http://ex/> . :seed :p :o .", "turtle").unwrap();
        let snapshot = dump(&g);
        assert!(update_in_place_atomic(&mut g, "NOT A VALID UPDATE").is_err(), "parse error is reported");
        assert_eq!(dump(&g), snapshot, "a parse error leaves the graph untouched");

        // Budgeted entry point: an all-succeed request under an unlimited budget commits.
        update_in_place_atomic_with_budget(
            &mut g,
            "PREFIX : <http://ex/> INSERT DATA { :a :p :b }",
            &crate::QueryBudget::unlimited(),
        )
        .expect("budgeted atomic apply commits an all-succeed request");
        assert_eq!(dataset_count(&g), 2, "the budgeted atomic apply committed the insert");
    }
}

/// [OPUS-4.8] (sq-7cxr, gh-44, Copilot PR#80) Coverage for the RESOLVED-DELTA CAPTURE/REPLAY
/// machinery — [`update_in_place_capturing`], [`apply_effects`], the [`UpdateEffect`] enum and
/// its four variants, [`record_delta`]'s empty-batch elision, and the `ensure_named` durable
/// routing reached through `apply_slot_delta`. PR#80 added this whole code path with no unit
/// tests, dropping sparq-engine's coverage below the 83% floor; these tests exercise it.
///
/// The LOAD-bearing invariant these tests pin: capturing the resolved delta during the ONE
/// in-memory application and replaying THAT (rather than re-executing the update text) makes the
/// durable mirror byte-equivalent to the in-memory state even when the update contains
/// non-deterministic functions (`NOW()`/`RAND()`/`UUID()`/`STRUUID()`, fresh `BNODE()`s). The
/// test method is therefore: capture on graph A, `apply_effects` onto an INDEPENDENTLY-built
/// graph B, and assert A and B are triple-for-triple identical across the whole dataset.
#[cfg(test)]
mod capture_replay {
    use super::*;

    /// Triple count of a single graph (default-graph store only) — the per-graph size probe.
    fn count(g: &Graph) -> usize {
        g.store.scan(&[None, None, None]).rows.len()
    }

    /// A canonical, sorted (s, p, o, graph) dump of the WHOLE dataset — the exact-equality probe
    /// used to assert that an `apply_effects` replay reproduces the captured in-memory state.
    /// (Same shape as the dump helpers in the other two test modules.)
    fn dump(g: &Graph) -> Vec<(String, String, String, String)> {
        let mut v: Vec<(String, String, String, String)> = Vec::new();
        let mut one = |g: &Graph, name: &str| {
            let scan = g.store.scan(&[None, None, None]);
            for r in scan.rows.iter() {
                let t = scan.to_spo(r);
                v.push((
                    g.dict.term(t[0]).to_string(),
                    g.dict.term(t[1]).to_string(),
                    g.dict.term(t[2]).to_string(),
                    name.to_string(),
                ));
            }
        };
        one(g, "");
        for (name, sub) in &g.named {
            one(sub, &name.to_string());
        }
        v.sort();
        v
    }

    fn budget() -> crate::QueryBudget {
        crate::QueryBudget::unlimited()
    }

    /// Capture the resolved effects of running `sparql` against a fresh graph built from `src`,
    /// then replay those effects onto a SECOND, independently-built graph from the same `src`.
    /// Returns `(captured_graph, replayed_graph, effects)`. The two graphs MUST then be equal —
    /// that is the durable-mirror guarantee under test.
    fn capture_and_replay(src: &str, fmt: &str, sparql: &str) -> (Graph, Graph, Vec<UpdateEffect>) {
        let mut a = load(src, fmt);
        let mut b = load(src, fmt);
        let effects = update_in_place_capturing(&mut a, sparql, &budget()).unwrap();
        apply_effects(&mut b, &effects).unwrap();
        (a, b, effects)
    }

    fn load(src: &str, fmt: &str) -> Graph {
        match fmt {
            "nquads" => Graph::load_dataset(src, "nquads").unwrap(),
            _ => Graph::load_str(src, fmt).unwrap(),
        }
    }

    fn assert_equiv(a: &Graph, b: &Graph, ctx: &str) {
        assert_eq!(dump(a), dump(b), "captured and replayed datasets diverged: {ctx}");
        assert_eq!(a.named.len(), b.named.len(), "named-graph slot count diverged: {ctx}");
    }

    /// THE CORE GUARANTEE. An UPDATE containing `NOW()`, `RAND()`, `UUID()`, `STRUUID()` and a
    /// fresh `BNODE()` resolves those to concrete values ONCE during the in-memory application;
    /// replaying the captured delta onto a second graph reproduces those EXACT values. Re-running
    /// the *text* would re-roll every one of them and diverge. We prove non-divergence by exact
    /// dataset equality, and we prove the values were actually non-trivial by counting them.
    #[test]
    fn nondeterministic_functions_replay_identically() {
        let src = "@prefix : <http://ex/> . :a :p :x . :b :p :y . :c :p :z .";
        // One INSERT … WHERE per match binds NOW/RAND/UUID/STRUUID/BNODE — five fresh
        // values per solution, all of which would re-roll on a text re-execution.
        let sparql = "PREFIX : <http://ex/> \
            INSERT { \
                ?s :ts ?t . ?s :r ?r . ?s :id ?u . ?s :sid ?su . ?s :note _:n . _:n :for ?s \
            } WHERE { \
                ?s :p ?o . \
                BIND(NOW() AS ?t) BIND(RAND() AS ?r) BIND(UUID() AS ?u) BIND(STRUUID() AS ?su) \
            }";
        let (a, b, effects) = capture_and_replay(src, "turtle", sparql);
        assert_equiv(&a, &b, "NOW/RAND/UUID/STRUUID/BNODE");
        // 3 solutions, original 3 + (6 template triples × 3) = 21 triples.
        assert_eq!(count(&a), 3 + 18, "all template triples inserted");
        // Exactly one resolved INSERT Delta was captured (no DELETE side, default graph).
        let deltas: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, UpdateEffect::Delta { .. }))
            .collect();
        assert_eq!(deltas.len(), 1, "one resolved insert delta");
        // The captured timestamps/uuids are concrete: three DISTINCT uuid objects exist
        // (would also be three after a re-roll, but the point is they are PINNED in the log).
        let uuids = crate::count(&a, "PREFIX : <http://ex/> SELECT DISTINCT ?u WHERE { ?s :id ?u }").unwrap();
        assert_eq!(uuids, 3, "three distinct captured UUIDs replayed verbatim");
        // The fresh BNODE() / template bnode resolved once and replayed as the SAME node shape.
        let notes = crate::count(&a, "PREFIX : <http://ex/> SELECT * WHERE { ?n :for ?s }").unwrap();
        assert_eq!(notes, 3, "fresh blank nodes replayed");
    }

    /// `UpdateEffect::Delta` for INSERT DATA — default graph AND a named-graph slot — captured
    /// once each and replayed exactly.
    #[test]
    fn delta_insert_data_default_and_named() {
        let sparql = "PREFIX : <http://ex/> \
            INSERT DATA { :a :p :b . :b :p :c . GRAPH :g1 { :x :q :y . :x :q :z } }";
        let (a, b, effects) = capture_and_replay("", "turtle", sparql);
        assert_equiv(&a, &b, "insert-data default+named");
        // Two slots → two Delta effects (default + :g1), each insert-only.
        let mut slots: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                UpdateEffect::Delta { slot, inserts, deletes } => {
                    assert!(deletes.is_empty(), "INSERT DATA produces no deletes");
                    Some((slot.is_none(), inserts.len()))
                }
                _ => None,
            })
            .collect();
        slots.sort();
        assert_eq!(slots, vec![(false, 2), (true, 2)], "default(2) + named(2) insert deltas");
        assert_eq!(a.named.len(), 1, ":g1 created once");
    }

    /// `UpdateEffect::Delta` for DELETE DATA (default graph and a named graph).
    #[test]
    fn delta_delete_data() {
        let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .";
        let sparql = "PREFIX : <http://ex/> \
            DELETE DATA { :a :p :b . GRAPH :g1 { :x :q :y } }";
        let (a, b, effects) = capture_and_replay(src, "nquads", sparql);
        assert_equiv(&a, &b, "delete-data default+named");
        assert_eq!(count(&a), 0, "default triple deleted");
        assert_eq!(count(&a.named[0].1), 0, "named triple deleted");
        // Two delete-only Delta effects.
        let deletes_only = effects.iter().all(|e| matches!(e, UpdateEffect::Delta { inserts, .. } if inserts.is_empty()));
        assert!(deletes_only, "DELETE DATA produces delete-only deltas");
        assert_eq!(effects.len(), 2, "default + named delete deltas");
    }

    /// `UpdateEffect::Delta` for DELETE/INSERT … WHERE in the default graph AND a named-graph
    /// slot — and the deletes-before-inserts capture order that `apply_effects` must preserve.
    #[test]
    fn delta_delete_insert_where_default_and_named() {
        // Default-graph rename; capture order must put the delete delta before the insert delta.
        let src = "@prefix : <http://ex/> . :a :age 30 . :b :age 25 .";
        let sparql =
            "PREFIX : <http://ex/> DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a }";
        let (a, b, effects) = capture_and_replay(src, "turtle", sparql);
        assert_equiv(&a, &b, "delete/insert-where default");
        assert_eq!(crate::count(&a, "PREFIX : <http://ex/> SELECT * WHERE { ?s :years ?a }").unwrap(), 2);
        assert_eq!(crate::count(&a, "PREFIX : <http://ex/> SELECT * WHERE { ?s :age ?a }").unwrap(), 0);
        // The first Delta captured is the deletes (inserts empty), then the inserts.
        let kinds: Vec<(bool, bool)> = effects
            .iter()
            .filter_map(|e| match e {
                UpdateEffect::Delta { inserts, deletes, .. } => Some((!inserts.is_empty(), !deletes.is_empty())),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![(false, true), (true, false)], "deletes captured before inserts");

        // A named-graph slot via a GRAPH template (the ADD-desugar shape).
        let src2 = "<http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .";
        let sparql2 = "PREFIX : <http://ex/> INSERT { GRAPH :g2 { ?s ?p ?o } } WHERE { GRAPH :g1 { ?s ?p ?o } }";
        let (a2, b2, _) = capture_and_replay(src2, "nquads", sparql2);
        assert_equiv(&a2, &b2, "delete/insert-where named");
        let g2 = a2.named.iter().find(|(n, _)| n.to_string().contains("g2")).expect(":g2 created");
        assert_eq!(count(&g2.1), 1, "named-graph insert delta replayed");
    }

    /// `UpdateEffect::Clear` — every `GraphTarget` variant — captured and replayed.
    #[test]
    fn clear_effect_all_targets() {
        let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .\n\
                   <http://ex/m> <http://ex/n> <http://ex/o> <http://ex/g2> .";
        for (clause, expect_clear) in [
            ("CLEAR DEFAULT", "default"),
            ("CLEAR GRAPH <http://ex/g1>", "g1"),
            ("CLEAR NAMED", "named"),
            ("CLEAR ALL", "all"),
        ] {
            let (a, b, effects) = capture_and_replay(src, "nquads", clause);
            assert_equiv(&a, &b, clause);
            assert!(
                matches!(effects.as_slice(), [UpdateEffect::Clear(_)]),
                "{clause} captured one Clear effect"
            );
            match expect_clear {
                "default" => assert_eq!(count(&a), 0),
                "all" => assert_eq!(count(&a), 0),
                _ => {}
            }
        }
    }

    /// `UpdateEffect::Drop` — every `GraphTarget` variant — captured and replayed.
    #[test]
    fn drop_effect_all_targets() {
        let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .\n\
                   <http://ex/m> <http://ex/n> <http://ex/o> <http://ex/g2> .";
        for clause in ["DROP DEFAULT", "DROP GRAPH <http://ex/g1>", "DROP NAMED", "DROP ALL"] {
            let (a, b, effects) = capture_and_replay(src, "nquads", clause);
            assert_equiv(&a, &b, clause);
            assert!(
                matches!(effects.as_slice(), [UpdateEffect::Drop(_)]),
                "{clause} captured one Drop effect"
            );
        }
        // DROP GRAPH removes the entry; replay must remove it on B too.
        let (a, b, _) = capture_and_replay(src, "nquads", "DROP GRAPH <http://ex/g1>");
        assert!(!a.named.iter().any(|(n, _)| n.to_string().contains("g1")));
        assert_eq!(a.named.len(), b.named.len());
    }

    /// `UpdateEffect::Create` — CREATE GRAPH captured and replayed; the empty named slot must
    /// exist on the replayed graph too.
    #[test]
    fn create_effect_replays_empty_named_graph() {
        let (a, b, effects) = capture_and_replay("", "turtle", "CREATE GRAPH <http://ex/fresh>");
        assert!(matches!(effects.as_slice(), [UpdateEffect::Create(_)]), "one Create effect");
        assert!(a.named.iter().any(|(n, _)| n.to_string().contains("fresh")), "slot created on A");
        assert_equiv(&a, &b, "create graph");
        // Idempotent: CREATE of an existing graph is still a captured Create that replays cleanly.
        let mut a2 = a;
        let mut b2 = b;
        let e2 = update_in_place_capturing(&mut a2, "CREATE GRAPH <http://ex/fresh>", &budget()).unwrap();
        apply_effects(&mut b2, &e2).unwrap();
        assert_eq!(a2.named.len(), 1, "no duplicate slot from a re-CREATE");
        assert_equiv(&a2, &b2, "idempotent create");
    }

    /// `record_delta` EMPTY-BATCH ELISION: a DELETE/INSERT … WHERE whose WHERE matches nothing
    /// produces ZERO `Delta` effects (empty inserts+deletes are never recorded), so the replay
    /// is a clean no-op.
    #[test]
    fn empty_where_records_no_delta() {
        let src = "@prefix : <http://ex/> . :a :p :b .";
        // WHERE matches nothing → no per-solution templates → no recorded delta.
        let sparql = "PREFIX : <http://ex/> DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a }";
        let (a, b, effects) = capture_and_replay(src, "turtle", sparql);
        assert!(effects.is_empty(), "empty WHERE records no Delta effect, got {effects:?}");
        assert_equiv(&a, &b, "empty where");
        assert_eq!(count(&a), 1, "the unmatched update changed nothing");

        // A DELETE DATA of a triple that is present still records (the batch is non-empty even
        // though the delete may be a no-op against the store) — this anchors the elision: it is
        // the empty BATCH, not the empty EFFECT, that is elided. DELETE DATA of nothing parses
        // to an empty data list, so it records nothing either.
        let empty_insert = "PREFIX : <http://ex/> INSERT { ?s :q ?o } WHERE { ?s :nomatch ?o }";
        let e2 = update_in_place_capturing(&mut load(src, "turtle"), empty_insert, &budget()).unwrap();
        assert!(e2.is_empty(), "an INSERT-WHERE with no matches records nothing");
    }

    /// `ensure_named` ROUTING through `apply_slot_delta`: an INSERT DATA into a BRAND-NEW named
    /// graph creates the slot ONCE; a subsequent INSERT into the same name targets the SAME slot
    /// (no duplicate). Both the capturing run and the replay must agree.
    #[test]
    fn ensure_named_creates_slot_once() {
        // First INSERT creates :ng; capture it.
        let mut a = load("", "turtle");
        let mut b = load("", "turtle");
        let e1 = update_in_place_capturing(
            &mut a,
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :ng { :x :p :y } }",
            &budget(),
        )
        .unwrap();
        apply_effects(&mut b, &e1).unwrap();
        assert_eq!(a.named.len(), 1, "one named slot after first insert");
        assert_eq!(b.named.len(), 1, "replay created the same single slot");

        // Second INSERT into the SAME named graph must reuse the slot (no duplicate).
        let e2 = update_in_place_capturing(
            &mut a,
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :ng { :x :p :z } }",
            &budget(),
        )
        .unwrap();
        apply_effects(&mut b, &e2).unwrap();
        assert_eq!(a.named.len(), 1, "no duplicate slot on a second insert into the same graph");
        assert_eq!(count(&a.named[0].1), 2, "both triples landed in the one slot");
        assert_equiv(&a, &b, "ensure_named single slot");

        // A DELETE into an ABSENT named graph is a no-op (apply_slot_delta's inserts-empty
        // early return) AND records nothing — replay stays consistent.
        let e3 = update_in_place_capturing(
            &mut a,
            "PREFIX : <http://ex/> DELETE DATA { GRAPH :absent { :a :b :c } }",
            &budget(),
        )
        .unwrap();
        // The delete batch is non-empty so it IS recorded, but applies to an absent graph as a
        // no-op; replaying it on B must likewise be a no-op (no slot created).
        apply_effects(&mut b, &e3).unwrap();
        assert_eq!(a.named.len(), 1, "delete into an absent graph creates no slot");
        assert_equiv(&a, &b, "delete absent named graph");
    }

    /// SINK-ABSENT path is unchanged: `update_in_place_with_budget` (None sink) applies the same
    /// state a capturing run does, and capture is fully elided (it returns nothing to inspect —
    /// the contract is that the in-memory mutation is byte-identical). We assert the two paths
    /// produce the same dataset.
    #[test]
    fn sink_absent_path_matches_capturing() {
        let src = "@prefix : <http://ex/> . :a :p :b . :b :p :c . :a :age 30 .";
        let sparql = "PREFIX : <http://ex/> \
            INSERT DATA { :c :p :d } ; \
            DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a } ; \
            INSERT DATA { GRAPH :g1 { :n :m :o } }";
        // No-sink path.
        let mut none = load(src, "turtle");
        update_in_place_with_budget(&mut none, sparql, &budget()).unwrap();
        // Capturing path on an independent graph + replay onto a third.
        let (cap, replay, _) = capture_and_replay(src, "turtle", sparql);
        assert_eq!(dump(&none), dump(&cap), "no-sink and capturing apply the same state");
        assert_eq!(dump(&none), dump(&replay), "replay reproduces the no-sink state");
        // And the plain `update_in_place` wrapper (unlimited budget) is the same again.
        let mut plain = load(src, "turtle");
        update_in_place(&mut plain, sparql).unwrap();
        assert_eq!(dump(&plain), dump(&none), "update_in_place wrapper == budgeted no-sink");
    }

    /// `apply_effects` on a FRESH (empty) graph reproduces a multi-operation request's full
    /// committed state — the end-to-end durable-mirror replay: capture against a seeded graph,
    /// replay the SAME effect log onto a graph that started empty but receives the resolved
    /// deltas (this is the durable-after-restart scenario where the mirror starts from base).
    #[test]
    fn apply_effects_on_fresh_graph_full_request() {
        let src = "@prefix : <http://ex/> . :a :p :b . :b :p :c . :a :age 30 . :b :age 25 .";
        // A request touching every effect-producing shape in one go.
        let sparql = "PREFIX : <http://ex/> \
            INSERT DATA { :c :p :d } ; \
            DELETE { ?s :age ?a } INSERT { ?s :years ?a } WHERE { ?s :age ?a } ; \
            INSERT DATA { GRAPH :g1 { :n :m :o } } ; \
            CREATE GRAPH :g2 ; \
            CLEAR GRAPH :g1 ; \
            DROP GRAPH :g2";
        let mut captured = load(src, "turtle");
        let effects = update_in_place_capturing(&mut captured, sparql, &budget()).unwrap();
        // Replay onto an independently-built graph (the durable mirror, identically seeded).
        let mut mirror = load(src, "turtle");
        apply_effects(&mut mirror, &effects).unwrap();
        assert_eq!(dump(&captured), dump(&mirror), "full multi-op request replayed identically");
        // CLEARed g1 is empty-but-present; dropped g2 is gone.
        assert!(captured.named.iter().any(|(n, _)| n.to_string().contains("g1")));
        assert!(!captured.named.iter().any(|(n, _)| n.to_string().contains("g2")));
        // Replaying the SAME log a SECOND time onto the same mirror is idempotent for the
        // structural ops and set-semantic for the data ops (no divergence).
        apply_effects(&mut mirror, &effects).unwrap();
        assert_eq!(dump(&captured), dump(&mirror), "re-applying the effect log does not diverge");
    }
}


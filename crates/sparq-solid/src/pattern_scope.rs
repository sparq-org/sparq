//! Pattern-scoped (sub-named-graph) result masking — the `pattern-scope` feature.
//!
//! Spike `sq-lrtc3.3` (epic `sq-lrtc3`): an ODRL target can denote a
//! **derived sub-graph asset** — the subset of a named graph visible under a set of
//! allow/deny triple patterns — instead of a whole graph. Design record:
//! `research/odrl-pattern-scoped-targets-2026-07.md`.
//!
//! # Architecture (masked-subgraph materialization)
//!
//! Masking happens by **materialization, not filtering**: a scoped graph is decoded,
//! filtered per its [`GraphScope`], and rebuilt as a physical [`Graph`] replica; the
//! engine then evaluates over a dataset in which masked triples are **physically
//! absent**. Soundness under `OPTIONAL` / `EXISTS` / `MINUS` / aggregates / `COUNT` /
//! `ASK` is therefore an identity with the oracle dataset (the source with the masked
//! triples deleted), not a per-construct audit obligation — no engine scan path, fast
//! path, or `estimate()` shortcut can observe a triple that is not there. The
//! graph-granular enforcement machinery (`DatasetView` allowlist, `wrap_read`
//! default-graph semantics) is reused unchanged.
//!
//! # Fail-closed semantics
//!
//! * A triple is visible iff it matches ≥ 1 `allow` pattern AND 0 `deny` patterns
//!   (**deny overrides allow**; an empty `allow` list grants nothing).
//! * [`masked_dataset`] is **explicit-decision**: a graph absent from the decision map
//!   is absent from the assembled dataset (absence = deny).
//! * [`PodStore::scoped_dataset`] is **refinement-only**: a scope entry can only
//!   shrink the session's graph-level accessible set, never widen it.
//! * A scoped graph whose mask evaluates empty is **omitted entirely** — a fully
//!   masked graph is indistinguishable from an absent one (`GRAPH <g> {}` unit-row
//!   semantics and `GRAPH ?g` enumeration reveal nothing).
//!
//! Matching is **term-identity** (the same syntactic identity the RDF dataset model
//! uses), not value equality: `"01"^^xsd:integer` does not match a pattern naming
//! `"1"^^xsd:integer`. Policies must name terms as they appear in the data.
//!
//! The read path only: `UPDATE` enforcement keeps its existing graph granularity
//! (design record §2.4). A [`ScopedDataset`] is a read-only replica of a moment in
//! time — re-materialize it after writing to the underlying store.

use crate::authindex::{Mode, Session};
use crate::PodStore;
use oxrdf::Term;
use rustc_hash::FxHashMap;
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_engine::{DatasetView, DefaultGraphMode, FxHashSet, QueryResult};
use std::sync::Arc;

/// One triple pattern of a scope: each component is a concrete [`Term`] or a wildcard
/// (`None`). No join variables — a `ScopePattern` is a per-triple predicate, so a
/// masked sub-graph is well-defined without any evaluation-order dependence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopePattern {
    s: Option<Term>,
    p: Option<Term>,
    o: Option<Term>,
}

impl ScopePattern {
    /// A pattern matching triples whose components equal the given terms
    /// (`None` = wildcard, matches anything in that position).
    pub fn new(s: Option<Term>, p: Option<Term>, o: Option<Term>) -> Self {
        ScopePattern { s, p, o }
    }

    /// The all-wildcard pattern (matches every triple).
    pub fn any() -> Self {
        ScopePattern { s: None, p: None, o: None }
    }

    /// `true` iff every concrete component equals the triple's component
    /// (term identity — see the module docs).
    pub fn matches(&self, triple: &[Term; 3]) -> bool {
        fn hit(want: &Option<Term>, got: &Term) -> bool {
            want.as_ref().is_none_or(|t| t == got)
        }
        hit(&self.s, &triple[0]) && hit(&self.p, &triple[1]) && hit(&self.o, &triple[2])
    }
}

/// The visibility scope applied to one source graph: a triple is visible iff it
/// matches at least one `allow` pattern and no `deny` pattern (deny overrides allow;
/// empty `allow` grants nothing — fail-closed).
#[derive(Clone, Debug, Default)]
pub struct GraphScope {
    allow: Vec<ScopePattern>,
    deny: Vec<ScopePattern>,
}

impl GraphScope {
    /// A scope granting ONLY the given patterns (an ODRL *permission* targeting a
    /// pattern asset). An empty list grants nothing.
    pub fn allow_only(allow: Vec<ScopePattern>) -> Self {
        GraphScope { allow, deny: Vec::new() }
    }

    /// A scope granting the whole graph EXCEPT the given patterns (an ODRL
    /// *prohibition* carving a hole out of an otherwise-granted graph).
    pub fn deny_within(deny: Vec<ScopePattern>) -> Self {
        GraphScope { allow: vec![ScopePattern::any()], deny }
    }

    /// The visibility predicate: matches ≥ 1 allow AND 0 deny.
    pub fn visible(&self, triple: &[Term; 3]) -> bool {
        self.allow.iter().any(|a| a.matches(triple)) && !self.deny.iter().any(|d| d.matches(triple))
    }
}

/// Decode → filter → rebuild: the sub-graph of `base` visible under `scope`, as a
/// fresh [`Graph`] (own dictionary + indexes). Triples failing [`GraphScope::visible`]
/// are **physically absent** from the result.
pub fn masked_graph(base: &Graph, scope: &GraphScope) -> Graph {
    let scan = base.store().scan(&[None, None, None]);
    let mut dict = Dict::new();
    let mut ids = Vec::new();
    for row in scan.rows.iter() {
        let spo = scan.to_spo(row);
        let terms = [base.dict().term(spo[0]), base.dict().term(spo[1]), base.dict().term(spo[2])];
        if scope.visible(&terms) {
            ids.push([dict.intern(&terms[0]), dict.intern(&terms[1]), dict.intern(&terms[2])]);
        }
    }
    Graph::from_parts(dict, ids)
}

/// Assemble a dataset from **explicit, complete visibility decisions**: the result
/// holds, for each `(name, scope)` entry whose masked sub-graph is non-empty, that
/// masked replica under `name` — and nothing else. The default graph is empty (pod
/// data never lives in the default graph) and a graph absent from `decisions` is
/// absent from the result (absence = deny). Intended caller: a policy layer (the
/// future ODRL bridge extension, design record §5) that has already run
/// deny-overrides across all rules and owns the final per-graph outcome.
pub fn masked_dataset(base: &Graph, decisions: &FxHashMap<Term, GraphScope>) -> Graph {
    let mut out = Graph::from_parts(Dict::new(), Vec::new());
    for (name, sub) in &base.named {
        if let Some(scope) = decisions.get(name) {
            let masked = masked_graph(sub, scope);
            if !masked.is_empty() {
                out.named.push((name.clone(), masked));
            }
        }
    }
    out
}

/// A session's pattern-scoped, materialized dataset replica: build once per
/// (session × scope map), query many times. Produced by [`PodStore::scoped_dataset`];
/// see the module docs for the semantics and the design record for the cost model.
pub struct ScopedDataset {
    graph: Graph,
    named: Arc<FxHashSet<Term>>,
}

impl ScopedDataset {
    /// The engine view over this replica: every contained named graph is visible,
    /// the default graph is empty — the same fail-closed shape as
    /// [`PodStore::view_for`]. Take it to drive any other engine entry point via
    /// `sparq_engine::with_view`.
    pub fn view(&self) -> DatasetView<'_> {
        DatasetView { base: &self.graph, named: Arc::clone(&self.named), default: DefaultGraphMode::Empty }
    }

    /// Evaluate a read query over the replica — the same read-path rewrite
    /// (`solid-sparql-query` empty default + explicit union opt-in) and view path as
    /// [`PodStore::query_as`], so results differ from it **only** by the masked
    /// triples' absence.
    ///
    /// # Errors
    /// `Err` iff the query does not parse or the engine fails on the wrapped query.
    pub fn query(&self, sparql: &str) -> Result<QueryResult, String> {
        let wrapped = crate::wrap_read(sparql)?;
        sparq_engine::query_view(&self.view(), &wrapped)
    }

    /// [`ScopedDataset::query`], returning the SPARQL 1.1 JSON results serialization.
    ///
    /// # Errors
    /// `Err` iff the query does not parse or the engine fails on the wrapped query.
    pub fn query_json(&self, sparql: &str) -> Result<String, String> {
        let wrapped = crate::wrap_read(sparql)?;
        sparq_engine::query_json_view(&self.view(), &wrapped)
    }

    /// ASK over the replica (same rewrite + view path as [`PodStore::ask_as`]).
    ///
    /// # Errors
    /// `Err` iff the query does not parse or the engine fails on the wrapped query.
    pub fn ask(&self, sparql: &str) -> Result<bool, String> {
        let wrapped = crate::wrap_read(sparql)?;
        sparq_engine::ask_view(&self.view(), &wrapped)
    }
}

impl PodStore {
    /// The session's pattern-scoped dataset: the graph-level accessible set (same
    /// mode-checked, fail-closed walk as [`PodStore::query_as`]) **refined** by
    /// `scopes` — an accessible graph with a scope entry contributes its masked
    /// sub-graph (omitted entirely when fully masked), an accessible graph without
    /// one contributes whole, and a non-accessible graph contributes nothing no
    /// matter what `scopes` says (restriction composes, never widens).
    ///
    /// The replica materializes every contributing graph (O(accessible dataset) —
    /// measured envelope in `bench/pattern-scope/`); reuse it across queries and
    /// rebuild it after any store mutation or re-materialization.
    pub fn scoped_dataset(&self, s: &Session, mode: Mode, scopes: &FxHashMap<Term, GraphScope>) -> ScopedDataset {
        let accessible = self.accessible_set(s, mode);
        let full = GraphScope::deny_within(Vec::new());
        let mut decisions: FxHashMap<Term, GraphScope> = FxHashMap::default();
        for (name, _) in &self.graph.named {
            if !accessible.contains(name) {
                continue; // graph-level denial/absence wins — a scope never widens
            }
            decisions.insert(name.clone(), scopes.get(name).unwrap_or(&full).clone());
        }
        let graph = masked_dataset(&self.graph, &decisions);
        let named: FxHashSet<Term> = graph.named.iter().map(|(n, _)| n.clone()).collect();
        ScopedDataset { graph, named: Arc::new(named) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    fn triple(s: &str, p: &str, o: &str) -> [Term; 3] {
        [iri(s), iri(p), iri(o)]
    }

    const T: (&str, &str, &str) = ("http://ex/s", "http://ex/p", "http://ex/o");

    #[test]
    fn scope_pattern_new_matches_each_component_and_wildcards() {
        let t = triple(T.0, T.1, T.2);
        assert!(ScopePattern::new(Some(iri(T.0)), Some(iri(T.1)), Some(iri(T.2))).matches(&t));
        assert!(ScopePattern::new(None, Some(iri(T.1)), None).matches(&t));
        assert!(!ScopePattern::new(None, Some(iri("http://ex/other")), None).matches(&t));
        assert!(!ScopePattern::new(Some(iri("http://ex/other")), None, None).matches(&t));
        assert!(!ScopePattern::new(None, None, Some(iri("http://ex/other"))).matches(&t));
    }

    #[test]
    fn scope_pattern_any_matches_everything() {
        assert!(ScopePattern::any().matches(&triple(T.0, T.1, T.2)));
        assert!(ScopePattern::any().matches(&triple("http://a/x", "http://a/y", "http://a/z")));
    }

    #[test]
    fn graph_scope_allow_only_is_fail_closed_and_deny_overrides_allow() {
        let t = triple(T.0, T.1, T.2);
        // Empty allow grants nothing (fail-closed).
        assert!(!GraphScope::allow_only(Vec::new()).visible(&t));
        // A matching allow grants…
        let p = ScopePattern::new(None, Some(iri(T.1)), None);
        assert!(GraphScope::allow_only(vec![p.clone()]).visible(&t));
        // …but an equally-matching deny overrides it.
        let both = GraphScope { allow: vec![p.clone()], deny: vec![p] };
        assert!(!both.visible(&t));
    }

    #[test]
    fn graph_scope_deny_within_carves_a_hole_out_of_allow_all() {
        let scope = GraphScope::deny_within(vec![ScopePattern::new(None, Some(iri(T.1)), None)]);
        assert!(!scope.visible(&triple(T.0, T.1, T.2)));
        assert!(scope.visible(&triple(T.0, "http://ex/q", T.2)));
    }

    #[test]
    fn masked_graph_drops_exactly_the_invisible_triples() {
        let g = Graph::load_dataset(
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n\
             <http://ex/s> <http://ex/q> <http://ex/o> .",
            "ntriples",
        )
        .unwrap();
        let masked =
            masked_graph(&g, &GraphScope::deny_within(vec![ScopePattern::new(None, Some(iri("http://ex/p")), None)]));
        assert_eq!(masked.len(), 1);
        let none = masked_graph(&g, &GraphScope::allow_only(Vec::new()));
        assert_eq!(none.len(), 0);
        let all = masked_graph(&g, &GraphScope::deny_within(Vec::new()));
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn masked_dataset_is_explicit_decision_and_omits_fully_masked_graphs() {
        let base = Graph::load_dataset(
            "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
             <http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g2> .",
            "nquads",
        )
        .unwrap();
        let mut decisions = FxHashMap::default();
        decisions.insert(iri("http://ex/g1"), GraphScope::deny_within(Vec::new()));
        // g2 has NO decision entry -> absent (absence = deny). Default graph empty.
        let out = masked_dataset(&base, &decisions);
        assert_eq!(out.len(), 0, "default graph must be empty");
        assert_eq!(out.named.len(), 1);
        assert_eq!(out.named[0].0, iri("http://ex/g1"));
        // A decision masking EVERYTHING omits the graph entirely.
        decisions.insert(iri("http://ex/g1"), GraphScope::allow_only(Vec::new()));
        assert!(masked_dataset(&base, &decisions).named.is_empty());
    }

    /// Tiny WAC pod: alice may Read the subtree; one data graph with two triples.
    fn pod() -> PodStore {
        let nq = "<https://pod.ex/d#s> <http://ex/p> \"keep\" <https://pod.ex/d> .\n\
                  <https://pod.ex/d#s> <http://ex/hide> \"mask\" <https://pod.ex/d> .\n\
                  <https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
                  <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
                  <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n\
                  <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .";
        let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").unwrap());
        s.materialize_wac().unwrap();
        s
    }

    fn alice() -> Session<'static> {
        Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None }
    }

    fn hide_scope() -> FxHashMap<Term, GraphScope> {
        let mut scopes = FxHashMap::default();
        scopes.insert(
            iri("https://pod.ex/d"),
            GraphScope::deny_within(vec![ScopePattern::new(None, Some(iri("http://ex/hide")), None)]),
        );
        scopes
    }

    #[test]
    fn scoped_dataset_refines_the_accessible_set() {
        let store = pod();
        let scoped = store.scoped_dataset(&alice(), Mode::Read, &hide_scope());
        assert_eq!(scoped.graph.named.len(), 1, "only the accessible data graph");
        assert_eq!(scoped.graph.named[0].1.len(), 1, "hide-triple masked out");
        // Grant-less session: nothing, no matter the scopes.
        let none = store.scoped_dataset(&Session::default(), Mode::Read, &hide_scope());
        assert!(none.graph.named.is_empty());
    }

    #[test]
    fn scoped_dataset_view_exposes_exactly_the_contained_names() {
        let scoped = pod().scoped_dataset(&alice(), Mode::Read, &hide_scope());
        let v = scoped.view();
        assert!(matches!(v.default, DefaultGraphMode::Empty));
        assert!(v.named.contains(&iri("https://pod.ex/d")));
        assert_eq!(v.named.len(), 1);
    }

    #[test]
    fn scoped_dataset_query_masks_and_errors_on_bad_sparql() {
        let scoped = pod().scoped_dataset(&alice(), Mode::Read, &hide_scope());
        let q = "SELECT ?o WHERE { GRAPH ?g { ?s <http://ex/hide> ?o } }";
        assert_eq!(scoped.query(q).unwrap().rows.len(), 0);
        let kept = "SELECT ?o WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }";
        assert_eq!(scoped.query(kept).unwrap().rows.len(), 1);
        assert!(scoped.query("SELECT nonsense").is_err());
    }

    #[test]
    fn scoped_dataset_query_json_masks() {
        let scoped = pod().scoped_dataset(&alice(), Mode::Read, &hide_scope());
        let json = scoped
            .query_json("SELECT ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?o")
            .unwrap();
        assert!(json.contains("keep") && !json.contains("mask"));
    }

    #[test]
    fn scoped_dataset_ask_masks() {
        let scoped = pod().scoped_dataset(&alice(), Mode::Read, &hide_scope());
        assert!(scoped.ask("ASK { GRAPH ?g { ?s <http://ex/p> ?o } }").unwrap());
        assert!(!scoped.ask("ASK { GRAPH ?g { ?s <http://ex/hide> ?o } }").unwrap());
    }
}

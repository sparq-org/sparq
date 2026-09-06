//! The `vec:` magic predicates: vector k-NN search inside plain SPARQL, with
//! ZERO engine changes. [OPUS-4.8] (sq-k6ex, epic sq-3183)
//!
//! ```
//! use sparq_core::Graph;
//! use sparq_vectors::VectorStore;
//! use oxrdf::NamedNode;
//!
//! # fn doit() -> Result<(), String> {
//! let g = Graph::load_str(r#"
//!     <http://ex/a> <http://ex/p> "a" .
//!     <http://ex/b> <http://ex/p> "b" .
//!     <http://ex/c> <http://ex/p> "c" .
//! "#, "ntriples").unwrap();
//! let id = |s: &str| g.id_of(&NamedNode::new(s).unwrap().into()).unwrap();
//! let path = std::env::temp_dir().join("sparq_vec_doctest.spqv");
//! let mut store = VectorStore::create(&path, 2).unwrap();
//! store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap();
//! store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap();
//! store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap();
//! // The two entities most aligned with the x-axis query vector "1,0".
//! let r = sparq_vectors::query_vec(&g,
//!     "PREFIX vec: <http://sparq.dev/vec#>
//!      SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
//!     &store)?;
//! assert_eq!(r.len(), 2); // <http://ex/a> and <http://ex/c>
//! # Ok(()) }
//! # doit().unwrap();
//! ```
//!
//! ## The rewrite
//!
//! [`rewrite_query`] walks the parsed spargebra algebra; in every basic graph
//! pattern, each magic triple pattern
//!
//! - `?node vec:nearest ( <query> <k> )` — bind `?node` to the `<k>` nearest
//!   neighbours (best first) of the query;
//! - `( ?node ?score ) vec:search ( <query> <k> )` — the same, additionally
//!   binding `?score` to each neighbour's cosine similarity (`xsd:double`);
//! - `( ?node ?score ?rank ?prov ) vec:hybrid ( <query> <k> )` — **hybrid retrieval**:
//!   the dense ranking fused with the caller's other retrieval arms, optionally
//!   reranked (see the `vec:hybrid` section below).
//!
//! is REMOVED and replaced by an inline [`Values`](GraphPattern::Values) table
//! of the search hits — the neighbour graph nodes (and, for `vec:search`, their
//! scores), resolved through the graph's dictionary. The surrounding query then
//! joins those nodes to triples through the store's ordinary permutation
//! indexes; the rewritten algebra runs through sparq-engine's prepared-query
//! seam ([`PreparedQuery`]`: From<spargebra::Query>`), so the engine — planner,
//! executor, wasm bundle — is completely unaware of vector search.
//!
//! The argument lists `( … )` are ordinary SPARQL RDF collections — spargebra
//! lowers them to `rdf:first`/`rdf:rest` blank-node chains in the same BGP, and
//! the rewrite walks those chains back into the argument tuple, so the surface
//! is plain SPARQL with no custom grammar.
//!
//! ## The query argument
//!
//! `<query>` is a **constant**, either of:
//!
//! - a **node IRI** (`<http://ex/seed>`) whose stored vector is the query — the
//!   "neighbours of this entity" form (the seed is itself excluded from its
//!   neighbours); empty if the IRI is absent from the graph or unembedded;
//! - a **vector literal** — a comma-separated list of `dim` floats
//!   (`"0.1,0.9,..."`) — a query-by-vector. The dimension must match the store.
//!
//! `<k>` is a **constant** non-negative integer literal.
//!
//! ## Constraints (each a hard query error, not a silent mismatch)
//!
//! the neighbour position(s) must be variables; the query/`k` arguments must be
//! constants (bind-time rewriting has no per-row values); the object argument
//! list must be exactly `( query k )`, and the `vec:search` subject list exactly
//! `( ?node ?score )` with both positions variables. Any other IRI in the `vec:`
//! namespace is unknown.
//!
//! Result ordering: `VALUES` rows carry no order through joins — sort with
//! `ORDER BY DESC(?score)` over a `vec:search` score variable to recover the
//! best-first order in the output.
//!
//! The store is per-graph (dictionary-local ids), so hits come from the store
//! you pass — typically the one built against the default graph.
//!
//! ## `vec:hybrid` — hybrid retrieval + reranking
//!
//! [SONNET-4.6] (sq-lhcot.4) `vec:hybrid` is the in-query surface for **hybrid retrieval**: this
//! crate's dense k-NN fused, by deterministic weighted RRF, with the caller's other retrieval
//! arms (a lexical/BM25 index, `sparq-sim`'s structural similarity, a business ranking), then
//! optionally passed through an out-of-process second-stage [`Reranker`](crate::hybrid::Reranker).
//! The arms and the reranker are supplied in a [`HybridConfig`], so the crate depends on none of
//! them; the predicate is reachable only through the hybrid entry points ([`query_vec_hybrid`],
//! [`prepare_vec_hybrid`], [`rewrite_query_hybrid`]) — a `vec:hybrid` pattern in a plain
//! [`query_vec`] is a hard error rather than a silently dense-only answer.
//!
//! The subject is a **prefix-optional** list — a bare `?node`, or `( ?node ?score )`,
//! `( ?node ?score ?rank )`, `( ?node ?score ?rank ?prov )` — binding, in order:
//!
//! - `?node` — the retrieved entity;
//! - `?score` — the **final-stage** score (`xsd:double`): the fused RRF score, or the reranker's
//!   own score when a reranker rescored the row. The two are different scales;
//! - `?rank` — the 1-based final rank (`xsd:integer`). `VALUES` rows carry no order through
//!   joins, so this is the join-safe ordering key: `ORDER BY ?rank`;
//! - `?prov` — the **rank provenance** (`xsd:string`): which arm ranked the row and where, e.g.
//!   `"vector=1;text=3;rerank=2"` ([`FusedHit::provenance`](crate::hybrid::FusedHit::provenance),
//!   parsed back by [`parse_provenance`](crate::hybrid::parse_provenance)).
//!
//! The `( <query> <k> )` argument list takes the two `vec:nearest` forms plus one more, so a
//! text arm can be driven by an actual text query:
//!
//! - a node IRI — neighbours of that entity (seed excluded), as `vec:nearest`;
//! - a **plain** literal — a comma-separated query vector, as `vec:nearest`;
//! - a **language-tagged** literal (`"machine learning"@en`) — a natural-language query. The
//!   language tag is what makes the form unambiguous against the vector literal. It requires a
//!   [`HybridConfig::query_embedder`](crate::hybrid::HybridConfig::query_embedder); without one
//!   it is a hard error, never a silently dense-less fusion.
//!
//! `<k>` is the number of FINAL results; each arm is over-fetched
//! ([`HybridConfig::candidates`](crate::hybrid::HybridConfig::candidates)) so fusion has
//! consensus evidence to work with and the reranker sees a wider pool.
//!
//! **Provisional surface.** `vec:hybrid` is implemented here ahead of the corresponding
//! amendment to the SPARQL Vector+GenAI extension draft, so [`vocab::VOCAB_REVISION`] is
//! deliberately NOT bumped — the term is listed in [`vocab::PROVISIONAL`] instead, and its
//! shape may change when the spec revision lands.
//!
//! **No lift is claimed.** Fusion and reranking are mechanisms; whether either improves
//! retrieval on a given corpus is an empirical question, answered by
//! [`hybrid::ablate`](crate::hybrid::ablate) on that corpus.

use crate::ann::{nearest_exact, nearest_exact_tiebreak};
// [SONNET-4.6] (sq-lhcot.4) The `vec:hybrid` fusion core: named weighted-RRF arms with per-rank
// provenance + the out-of-process second-stage reranker and its fail-open/fail-closed policy. Same
// `vec-predicate` feature — the module ships with the query surface it exists to serve and adds no
// dependency.
#[cfg(feature = "filtered-ann")]
use crate::hybrid::MAX_ARM_PAGE;
use crate::hybrid::{
    apply_rerank, fuse_arms, ArmQuery, ArmRanking, FusedHit, HybridConfig, VECTOR_ARM,
};
use crate::spqv_provenance::Metric;
use crate::store::VectorStore;
use crate::vocab;
// [OPUS-4.8] (sq-z589, epic sq-3183) The approximate backend the `*_approx` entry points search
// the UNFILTERED `vec:` path through — the on-disk Vamana graph. `approx-ann` only (the only
// feature that compiles an approximate index); with it off the crate carries only the exact path.
#[cfg(feature = "approx-ann")]
use crate::diskann::DiskAnnIndex;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use spargebra::algebra::GraphPattern;
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;
use sparq_core::dict::{is_inline, Id, NO_ID};
use sparq_core::Graph;
use sparq_engine::{PreparedQuery, QueryBudget, QueryResult};

// [OPUS-4.8] (sq-bvmd, epic sq-3183) Filtered-ANN BGP→IdMask wiring: when a `vec:`
// request shares its neighbour variable with ordinary patterns in the SAME BGP, those
// patterns carve out the eligible graph nodes (the candidate id-set), and the k-NN
// search is restricted to that set — correct (and, for a selective constraint, faster)
// than post-filtering the unfiltered neighbours. This needs the `filtered-ann` API
// (`IdMask` + the cost-model `nearest_filtered_costed_tiebreak`, sq-7hx6, which picks
// pre-filter vs post-filter by selectivity — identical answer either way), so the whole
// derive-and-filter path is additionally gated on `filtered-ann`; with that feature off
// the `vec:` predicate behaves EXACTLY as before (plain unfiltered `nearest_exact`).
#[cfg(feature = "filtered-ann")]
use crate::cost::{nearest_filtered_costed_tiebreak, CostModel};
#[cfg(feature = "filtered-ann")]
use crate::filter::IdMask;
// [OPUS-4.8] (sq-36ol, epic sq-3183) The derived-mask cache: a `vec:` predicate that shares its
// neighbour variable with constraining patterns re-evaluates that sub-BGP (a full SELECT through
// the engine) on EVERY prepare to build the `IdMask`. Against an unchanged graph the answer is
// identical every time, so we cache it keyed by (constraining sub-BGP, graph fingerprint) — see
// `mask_cache` below. Gated on `filtered-ann` (the only feature with a mask to cache).
#[cfg(feature = "filtered-ann")]
use crate::fingerprint::Fingerprint;

/// `rdf:first`, `rdf:rest`, `rdf:nil` — the collection vocabulary spargebra
/// lowers `( … )` argument lists into.
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

/// [OPUS-4.8] (sq-z589, epic sq-3183) The backend the UNFILTERED `vec:` k-NN runs through. The
/// rewrite picks neighbours by this seam, so swapping the exact full scan for an approximate index
/// is a one-line backend swap with NO other change to the rewrite (parse, list-walk, error checks,
/// seed-self-exclusion, VALUES inlining and the join all stay identical). Two impls:
///
/// - [`ExactSearch`] (default; the original `vec:` behaviour) — a full [`nearest_exact`] scan;
///   answer-exact, the deterministic ground truth, a fine default below ~10⁵ vectors.
/// - `ApproxSearch` (`approx-ann` only) — the on-disk [`DiskAnnIndex`] Vamana greedy beam search;
///   APPROXIMATE (recall < 1.0 — see [`crate::ann`]/[`crate::diskann`]), for large `.spqv` stores
///   where the full scan is the bottleneck. NEVER claimed as exact: the index can miss a true
///   neighbour. The FILTERED `vec:` path (when `filtered-ann` composes in) is unchanged — it still
///   goes through the cost-model'd, VG-TIE-1 tie-broken `nearest_filtered_costed_tiebreak`; this
///   seam is only the unfiltered scan.
trait UnfilteredSearch {
    /// Top-`k` `(id, cosine)` by similarity to `query`, best first (same contract as
    /// [`nearest_exact`]: all-zero query → no results).
    fn search(&self, query: &[f32], k: usize) -> Vec<(Id, f32)>;

    /// [SONNET-4.6] (sq-tb9p0) Answer-exact top-`k` with the VG-TIE-1 deterministic boundary
    /// tie-break ([`nearest_exact_tiebreak`]) and the seed already excluded, or `Ok(None)` when
    /// this backend cannot honour the rule — an APPROXIMATE backend has recall < 1 and no true
    /// boundary to break ties at, so it keeps the plain [`search`](Self::search) path. `Err` is
    /// the fail-closed embeddable-domain rejection: a blank-node candidate whose term (not its
    /// score alone) would decide membership is a hard query error, never ranked.
    fn search_tied(
        &self,
        _query: &[f32],
        _k: usize,
        _graph: &Graph,
        _exclude: Option<Id>,
    ) -> Result<Option<Vec<(Id, f32)>>, String> {
        Ok(None)
    }
}

/// The exact full-scan backend — preserves the pre-sq-z589 `vec:` behaviour exactly.
struct ExactSearch<'a> {
    store: &'a VectorStore,
}

impl UnfilteredSearch for ExactSearch<'_> {
    fn search(&self, query: &[f32], k: usize) -> Vec<(Id, f32)> {
        nearest_exact(self.store, query, k)
    }

    fn search_tied(
        &self,
        query: &[f32],
        k: usize,
        graph: &Graph,
        exclude: Option<Id>,
    ) -> Result<Option<Vec<(Id, f32)>>, String> {
        nearest_exact_tiebreak(self.store, graph, query, k, exclude).map(Some)
    }
}

/// [OPUS-4.8] (sq-z589) The approximate on-disk Vamana backend — `approx-ann` only.
#[cfg(feature = "approx-ann")]
struct ApproxSearch<'a> {
    index: &'a DiskAnnIndex,
}

#[cfg(feature = "approx-ann")]
impl UnfilteredSearch for ApproxSearch<'_> {
    fn search(&self, query: &[f32], k: usize) -> Vec<(Id, f32)> {
        self.index.nearest(query, k)
    }
}

/// Executes a SPARQL query that may use the `vec:` magic predicates: parse,
/// [`rewrite_query`], then evaluate with the standard engine. The unfiltered k-NN is the
/// answer-exact full scan — for the **approximate** index path see [`query_vec_approx`].
pub fn query_vec(graph: &Graph, sparql: &str, store: &VectorStore) -> Result<QueryResult, String> {
    sparq_engine::query_prepared(graph, &prepare_vec(graph, sparql, store)?)
}

/// [SONNET-4.6] (sq-lhcot.4) [`query_vec`] with the **hybrid** retrieval arms + optional
/// second-stage reranker `cfg` supplies, so the query may use `vec:hybrid` (see the module docs).
/// `vec:nearest`/`vec:search` behave exactly as through [`query_vec`]; a query with no
/// `vec:hybrid` pattern is unaffected by `cfg`.
pub fn query_vec_hybrid(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    cfg: &HybridConfig<'_>,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared(graph, &prepare_vec_hybrid(graph, sparql, store, cfg)?)
}

/// [SONNET-4.6] (sq-lhcot.4) [`query_vec_hybrid`] under a cooperative [`QueryBudget`].
pub fn query_vec_hybrid_with_budget(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    cfg: &HybridConfig<'_>,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared_with_budget(
        graph,
        &prepare_vec_hybrid(graph, sparql, store, cfg)?,
        budget,
    )
}

/// [SONNET-4.6] (sq-lhcot.4) The prepare-time twin of [`query_vec_hybrid`]: parse + rewrite
/// (running the arms and the second stage now, exactly as [`prepare_vec`] runs the k-NN now) into
/// a [`PreparedQuery`]. Re-prepare after the graph, the store, or any arm's index changes.
pub fn prepare_vec_hybrid(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    cfg: &HybridConfig<'_>,
) -> Result<PreparedQuery, String> {
    prepare_with(graph, sparql, store, &ExactSearch { store }, Some(cfg))
}

/// [SONNET-4.6] (sq-lhcot.4) [`rewrite_query`] with the hybrid arms available, so `vec:hybrid`
/// patterns rewrite too.
pub fn rewrite_query_hybrid(
    query: Query,
    graph: &Graph,
    store: &VectorStore,
    cfg: &HybridConfig<'_>,
) -> Result<Query, String> {
    rewrite_query_with(query, graph, store, &ExactSearch { store }, Some(cfg))
}

/// [`query_vec`] under a cooperative [`QueryBudget`].
pub fn query_vec_with_budget(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared_with_budget(graph, &prepare_vec(graph, sparql, store)?, budget)
}

/// Parses and rewrites into a [`PreparedQuery`] — compose with any of the
/// engine's `*_prepared` entry points (`ask_prepared`, `construct_prepared`,
/// …). Note the hits are frozen at rewrite time: re-prepare after the graph
/// (and store) change.
pub fn prepare_vec(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
) -> Result<PreparedQuery, String> {
    prepare_with(graph, sparql, store, &ExactSearch { store }, None)
}

/// [OPUS-4.8] (sq-z589, epic sq-3183) [`query_vec`] but with the **unfiltered** k-NN run through an
/// approximate on-disk [`DiskAnnIndex`] (Vamana) instead of the exact full scan — for large `.spqv`
/// stores where the brute-force scan is the bottleneck. Everything else is identical to
/// [`query_vec`]: parse, rewrite, VALUES inlining, joins, error checks, the seed-self-exclusion, and
/// (when `filtered-ann` composes in) the BGP→mask filtered path.
///
/// **Approximate ⇒ recall < 1.0.** The index can miss a true neighbour the greedy beam never
/// visits; this is the *approximate* ranking, NOT the exact top-`k`. Use [`query_vec`] (the exact
/// scan) when answer-exactness matters. `OFF by default` — gated on `approx-ann` (the only feature
/// that pulls an approximate index into the build) on top of `vec-predicate`.
///
/// The `index` must be built against the SAME graph generation as `graph`/`store` (its dictionary
/// ids must match): pass an index built with [`DiskAnnIndex::build_for`] so the fingerprint guard
/// catches a stale index, exactly as the store's own fingerprint guard does.
///
/// Note: the **filtered** `vec:` path (when `filtered-ann` is also on and the neighbour variable is
/// constrained) is unaffected — it still goes through the cost-model'd filtered search. This entry
/// point only swaps the *unfiltered* scan for the approximate index.
#[cfg(feature = "approx-ann")]
pub fn query_vec_approx(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    index: &DiskAnnIndex,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared(graph, &prepare_vec_approx(graph, sparql, store, index)?)
}

/// [OPUS-4.8] (sq-z589) [`query_vec_approx`] under a cooperative [`QueryBudget`].
#[cfg(feature = "approx-ann")]
pub fn query_vec_approx_with_budget(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    index: &DiskAnnIndex,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared_with_budget(
        graph,
        &prepare_vec_approx(graph, sparql, store, index)?,
        budget,
    )
}

/// [OPUS-4.8] (sq-z589) [`prepare_vec`] but resolving the **unfiltered** `vec:` k-NN through the
/// approximate `index` — the prepare-time twin of [`query_vec_approx`]. See that function for the
/// recall / staleness caveats.
#[cfg(feature = "approx-ann")]
pub fn prepare_vec_approx(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    index: &DiskAnnIndex,
) -> Result<PreparedQuery, String> {
    // The index is keyed by `graph`'s dictionary ids too, so verify it alongside the store when it
    // carries a fingerprint (built with `build_for`); a legacy/unbound index is left to work as
    // before, like the store guard below.
    if index.fingerprint().is_some() {
        index.check_graph(store, graph)?;
    }
    prepare_with(graph, sparql, store, &ApproxSearch { index }, None)
}

/// Shared prepare body: staleness-guard the store, parse, then rewrite every `vec:` pattern with
/// `searcher` as the unfiltered backend.
fn prepare_with(
    graph: &Graph,
    sparql: &str,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    hybrid: Option<&HybridConfig<'_>>,
) -> Result<PreparedQuery, String> {
    // [OPUS-4.8] Opportunistic staleness guard: the store is keyed by `graph`'s
    // dictionary ids, so a store built against a DIFFERENT graph would silently
    // mis-resolve neighbours. If the store carries a graph fingerprint, verify
    // it now (hard error on mismatch). Unbound / legacy (version-1) stores carry
    // no fingerprint and are left to work as before — we only check when we can.
    if store.fingerprint().is_some() {
        store.check_graph(graph)?;
    }
    // [SONNET-4.6] (sq-tb9p0) VG-MET-1/VG-MET-4 (mainline): the `vec:` query surface evaluates
    // exactly ONE metric — cosine similarity. A store whose persisted v3 provenance declares a
    // DIFFERENT metric must be rejected, not scored: evaluating a cosine ranking over vectors
    // embedded for another metric returns well-formed but meaningless numbers. A legacy v1/v2
    // store (or one built without binding a provenance) declares nothing and is left to work as
    // before — implicit cosine, exactly the pre-provenance behaviour; the strict fail-closed
    // check for those is `VectorStore::check_provenance`. The declared NORMALIZATION regime is
    // deliberately NOT a rejection axis on this path: the exact scan computes cosine with the
    // norms taken at comparison time (scale-invariant), so both declared regimes (`none` / `l2`)
    // are evaluated faithfully under the declared cosine metric and no cross-normalisation
    // mismatch can arise here.
    if let Some(prov) = store.provenance() {
        if prov.metric != Metric::Cosine {
            return Err(format!(
                "vec: this store declares the '{}' distance metric, but the vec: query surface \
                 evaluates cosine similarity only; refusing the cross-metric evaluation (the \
                 scores would be well-formed but meaningless). Rebuild the store under cosine, \
                 or rank it through a '{}'-aware code path",
                prov.metric, prov.metric
            ));
        }
    }
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| e.to_string())?;
    Ok(PreparedQuery::from(rewrite_query_with(
        query, graph, store, searcher, hybrid,
    )?))
}

/// Rewrites every `vec:` magic pattern in the query into inline `VALUES` over
/// the search hits (see the module docs). A query without `vec:` patterns
/// passes through unchanged. The unfiltered k-NN uses the exact full scan — for the approximate
/// index path use [`query_vec_approx`] / [`prepare_vec_approx`].
pub fn rewrite_query(query: Query, graph: &Graph, store: &VectorStore) -> Result<Query, String> {
    rewrite_query_with(query, graph, store, &ExactSearch { store }, None)
}

/// Threaded form of [`rewrite_query`]: `searcher` is the unfiltered k-NN backend (exact scan or an
/// approximate index). [OPUS-4.8] (sq-z589)
fn rewrite_query_with(
    mut query: Query,
    graph: &Graph,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    hybrid: Option<&HybridConfig<'_>>,
) -> Result<Query, String> {
    let pattern = match &mut query {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    };
    rewrite_pattern(pattern, graph, store, searcher, hybrid)?;
    Ok(query)
}

/// Recursively rewrites the magic patterns inside `p`.
fn rewrite_pattern(
    p: &mut GraphPattern,
    graph: &Graph,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    hybrid: Option<&HybridConfig<'_>>,
) -> Result<(), String> {
    match p {
        GraphPattern::Bgp { patterns } => {
            let patterns = std::mem::take(patterns);
            *p = rewrite_bgp(patterns, graph, store, searcher, hybrid)?;
        }
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::Lateral { left, right } => {
            rewrite_pattern(left, graph, store, searcher, hybrid)?;
            rewrite_pattern(right, graph, store, searcher, hybrid)?;
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => {
            rewrite_pattern(inner, graph, store, searcher, hybrid)?
        }
        // No BGPs inside: property paths and inline VALUES pass through.
        GraphPattern::Path { .. } | GraphPattern::Values { .. } => {}
    }
    Ok(())
}

/// The query a `vec:` request searches by.
enum QueryArg {
    /// A node IRI whose stored vector is the query (and which is excluded from
    /// its own neighbours).
    Node(NamedNode),
    /// An explicit query vector parsed from a comma-separated literal.
    Vector(Vec<f32>),
}

/// One `vec:nearest`/`vec:search` request found in a BGP. The `score` variable
/// is `Some` only for the `vec:search` form. [OPUS-4.8]
struct KnnReq {
    node: Variable,
    score: Option<Variable>,
    query: QueryArg,
    k: usize,
}

/// [SONNET-4.6] (sq-lhcot.4) The query a `vec:hybrid` request searches by — the two [`QueryArg`]
/// forms plus the natural-language one a lexical arm can actually use.
enum HybridQueryArg {
    /// A node IRI whose stored vector is the dense query (and which is excluded from its own
    /// neighbours).
    Node(NamedNode),
    /// An explicit dense query vector, from a plain comma-separated literal.
    Vector(Vec<f32>),
    /// `(text, language tag)` from a language-tagged literal. The dense arm needs a configured
    /// query embedder for this form.
    Text(String, String),
}

/// [SONNET-4.6] (sq-lhcot.4) One `vec:hybrid` request found in a BGP. `score`/`rank`/`prov` are
/// the optional trailing subject-list positions, in that order (a position can only be present if
/// every earlier one is).
struct HybridReq {
    node: Variable,
    score: Option<Variable>,
    rank: Option<Variable>,
    prov: Option<Variable>,
    query: HybridQueryArg,
    k: usize,
}

/// Splits a BGP's `vec:` magic patterns out, runs the k-NN search, and joins
/// the `VALUES` hit tables onto the remaining ordinary patterns. The collection
/// (`rdf:first`/`rdf:rest`) triples that spargebra emitted for the argument
/// lists are consumed by the rewrite and removed from the surviving BGP.
fn rewrite_bgp(
    patterns: Vec<TriplePattern>,
    graph: &Graph,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    hybrid: Option<&HybridConfig<'_>>,
) -> Result<GraphPattern, String> {
    // Index the rdf:first/rdf:rest triples by their (blank-node) list-cell
    // subject so the magic-predicate handlers can walk each `( … )` chain.
    let lists = ListCells::collect(&patterns);

    let mut rest: Vec<TriplePattern> = Vec::with_capacity(patterns.len());
    let mut reqs: Vec<KnnReq> = Vec::new();
    // [SONNET-4.6] (sq-lhcot.4) `vec:hybrid` requests are collected separately: they bind a
    // different (wider) row shape and are resolved through the fusion pipeline, not the plain k-NN.
    let mut hybrid_reqs: Vec<HybridReq> = Vec::new();
    // [OPUS-4.8] Blank-node-subject rdf:first/rdf:rest cells are the lowered
    // form of `( … )` argument lists. They are only an implementation detail of
    // a `vec:` predicate, so we *defer* the decision to drop them: they are
    // consumed (removed) ONLY if this BGP actually contains a `vec:` request.
    // If no `vec:` predicate is present the BGP passes through unchanged —
    // including any legitimate RDF-collection query — and these deferred cells
    // are restored into `rest` below. User-authored `rdf:first`/`rest` patterns
    // with variable/IRI subjects are never deferred (they fall through to
    // `rest` like any other ordinary pattern).
    let mut deferred_lists: Vec<&TriplePattern> = Vec::new();

    for tp in &patterns {
        if is_list_cell(tp) {
            deferred_lists.push(tp);
            continue;
        }
        let NamedNodePattern::NamedNode(pred) = &tp.predicate else {
            rest.push(tp.clone());
            continue;
        };
        let iri = pred.as_str();
        if !iri.starts_with(vocab::VEC_NS) {
            rest.push(tp.clone());
            continue;
        }
        match iri {
            vocab::NEAREST => reqs.push(parse_nearest(tp, &lists)?),
            vocab::SEARCH => reqs.push(parse_search(tp, &lists)?),
            // [SONNET-4.6] (sq-lhcot.4) The hybrid arms + reranker live in the caller's
            // `HybridConfig`, which only the `*_hybrid` entry points carry. Reaching `vec:hybrid`
            // without one must be a HARD error: silently answering with the dense arm alone would
            // return a well-formed ranking that is not the hybrid ranking the query asked for.
            vocab::HYBRID => {
                if hybrid.is_none() {
                    return Err(
                        "vec: vec:hybrid needs the retrieval arms in a HybridConfig — use \
                         query_vec_hybrid / prepare_vec_hybrid / rewrite_query_hybrid (plain \
                         query_vec carries no arms, and answering with the dense arm alone would \
                         not be the hybrid ranking)"
                            .to_string(),
                    );
                }
                hybrid_reqs.push(parse_hybrid(tp, &lists)?);
            }
            _ => {
                // [SONNET-4.6] (sq-tb9p0) VG-GOV-3: the VG-VOC-1 hard error states the highest
                // vocabulary revision this build implements, so a query written against a NEWER
                // spec revision's term fails with the version gap named, not just "unknown".
                return Err(format!(
                    "vec: unknown magic predicate <{}> (supported: vec:nearest, vec:search, and \
                     the PROVISIONAL vec:hybrid; this build implements vec: vocabulary revision \
                     {})",
                    iri,
                    vocab::VOCAB_REVISION
                ))
            }
        }
    }

    // No `vec:` request in this BGP: it (and any RDF-collection cells in it)
    // must pass through completely unchanged.
    if reqs.is_empty() && hybrid_reqs.is_empty() {
        rest.extend(deferred_lists.into_iter().cloned());
        return Ok(GraphPattern::Bgp { patterns: rest });
    }

    // [OPUS-4.8] (sq-bvmd) The ordinary patterns that survive this BGP are also the
    // patterns that *constrain* each `vec:` neighbour variable: the subjects they admit
    // are the candidate id-set for a filtered ANN search. Derive that set from `rest`
    // BEFORE it is moved into the output (the borrow ends with `mask_constraint`).
    #[cfg(feature = "filtered-ann")]
    let mask_constraint = rest.clone();

    // Join the hit tables onto the remaining ordinary patterns.
    let mut out = GraphPattern::Bgp { patterns: rest };
    for req in reqs {
        #[cfg(feature = "filtered-ann")]
        let hits = run_knn(&req, graph, store, searcher, &mask_constraint)?;
        #[cfg(not(feature = "filtered-ann"))]
        let hits = run_knn(&req, graph, store, searcher)?;
        let mut variables = vec![req.node];
        if let Some(s) = &req.score {
            variables.push(s.clone());
        }
        let score_wanted = req.score.is_some();
        let bindings = hits
            .into_iter()
            .filter_map(|(id, score)| {
                // Neighbour ids resolve to graph nodes (IRIs/literals); a blank
                // node cannot appear in a VALUES row, so skip it (entities embed
                // as IRIs in practice).
                let node = term_to_ground(graph.dict.term(id))?;
                let mut row = vec![Some(node)];
                if score_wanted {
                    row.push(Some(GroundTerm::Literal(Literal::from(f64::from(score)))));
                }
                Some(row)
            })
            .collect();
        let values = GraphPattern::Values {
            variables,
            bindings,
        };
        out = join_values(out, values);
    }

    // [SONNET-4.6] (sq-lhcot.4) The hybrid requests: fuse the dense arm with the caller's arms,
    // optionally rerank, and inline the resulting rows the same way.
    // `hybrid_reqs` is non-empty only when a config was supplied (the match above rejects a
    // `vec:hybrid` pattern without one), so this `if let` can never skip a pending request.
    if let Some(cfg) = hybrid {
        for req in hybrid_reqs {
            #[cfg(feature = "filtered-ann")]
            let hits = run_hybrid(&req, graph, store, searcher, cfg, &mask_constraint)?;
            #[cfg(not(feature = "filtered-ann"))]
            let hits = run_hybrid(&req, graph, store, searcher, cfg)?;

            let mut variables = vec![req.node];
            for v in [&req.score, &req.rank, &req.prov].into_iter().flatten() {
                variables.push(v.clone());
            }
            let mut bindings: Vec<Vec<Option<GroundTerm>>> = Vec::with_capacity(hits.len());
            for hit in hits {
                // Same rule as the k-NN path: a neighbour that resolves to a blank node cannot appear
                // in a VALUES row, so it is skipped — and `?rank` is assigned over the SURVIVING rows
                // so the bound ranks stay a gap-free 1..n.
                let Some(node) = term_to_ground(graph.dict.term(hit.id)) else {
                    continue;
                };
                let mut row = vec![Some(node)];
                if req.score.is_some() {
                    row.push(Some(GroundTerm::Literal(Literal::from(hit.score))));
                }
                if req.rank.is_some() {
                    row.push(Some(GroundTerm::Literal(Literal::from(
                        bindings.len() as i64 + 1,
                    ))));
                }
                if req.prov.is_some() {
                    row.push(Some(GroundTerm::Literal(Literal::new_simple_literal(
                        hit.provenance(),
                    ))));
                }
                bindings.push(row);
            }
            out = join_values(
                out,
                GraphPattern::Values {
                    variables,
                    bindings,
                },
            );
        }
    }

    Ok(out)
}

/// Joins an inlined hit table onto the BGP's surviving patterns. An all-magic BGP leaves an empty
/// `Bgp` behind, whose unit table is dropped rather than joined.
fn join_values(out: GraphPattern, values: GraphPattern) -> GraphPattern {
    match out {
        GraphPattern::Bgp { ref patterns } if patterns.is_empty() => values,
        other => GraphPattern::Join {
            left: Box::new(values),
            right: Box::new(other),
        },
    }
}

/// True for a *synthetic* list-cell triple — a `rdf:first`/`rdf:rest` triple
/// with a **blank-node subject**, the exact shape spargebra emits when it lowers
/// a `( … )` collection. [OPUS-4.8] The blank-node-subject restriction is
/// load-bearing: it means user-authored collection patterns with variable or
/// IRI subjects (e.g. `?x rdf:first ?y`) are NOT treated as argument-list cells
/// and survive into the engine. (Combined with the caller only dropping these
/// when a `vec:` predicate is present, a query with no `vec:` pattern is left
/// entirely unchanged.)
fn is_list_cell(tp: &TriplePattern) -> bool {
    matches!(&tp.subject, TermPattern::BlankNode(_))
        && matches!(&tp.predicate, NamedNodePattern::NamedNode(p) if p.as_str() == RDF_FIRST || p.as_str() == RDF_REST)
}

/// The `rdf:first`/`rdf:rest` cells of every collection in a BGP, keyed by the
/// list-cell blank node, so an argument list can be walked from its head.
struct ListCells<'a> {
    /// blank-node id → (first element, rest cell)
    cells: FxHashMap<&'a str, (&'a TermPattern, &'a TermPattern)>,
}

impl<'a> ListCells<'a> {
    fn collect(patterns: &'a [TriplePattern]) -> ListCells<'a> {
        let mut firsts: FxHashMap<&str, &TermPattern> = FxHashMap::default();
        let mut rests: FxHashMap<&str, &TermPattern> = FxHashMap::default();
        for tp in patterns {
            let TermPattern::BlankNode(b) = &tp.subject else {
                continue;
            };
            let NamedNodePattern::NamedNode(p) = &tp.predicate else {
                continue;
            };
            match p.as_str() {
                RDF_FIRST => {
                    firsts.insert(b.as_str(), &tp.object);
                }
                RDF_REST => {
                    rests.insert(b.as_str(), &tp.object);
                }
                _ => {}
            }
        }
        let cells = firsts
            .iter()
            .filter_map(|(&b, &first)| rests.get(b).map(|&rest| (b, (first, rest))))
            .collect();
        ListCells { cells }
    }

    /// Walks the collection whose head is `head` into its element [`TermPattern`]s.
    /// `head` is the object/subject of a `vec:` predicate — a blank-node list
    /// head, or `rdf:nil` for the empty list. Errors if the chain is malformed
    /// (dangling cell, or a non-collection term where a list was required).
    fn elements(&self, head: &'a TermPattern) -> Result<Vec<&'a TermPattern>, String> {
        let mut out = Vec::new();
        let mut cur = head;
        let mut guard = 0usize;
        loop {
            match cur {
                TermPattern::NamedNode(n) if n.as_str() == RDF_NIL => return Ok(out),
                TermPattern::BlankNode(b) => {
                    let Some(&(first, rest)) = self.cells.get(b.as_str()) else {
                        return Err(
                            "vec: malformed argument list (dangling rdf:rest cell)".to_string()
                        );
                    };
                    out.push(first);
                    cur = rest;
                }
                other => {
                    return Err(format!(
                        "vec: a vec: predicate requires a `( … )` argument list, got {other}"
                    ))
                }
            }
            guard += 1;
            if guard > 1 << 20 {
                return Err("vec: argument list is cyclic".to_string());
            }
        }
    }
}

/// Parses `?node vec:nearest ( <query> <k> )`.
fn parse_nearest(tp: &TriplePattern, lists: &ListCells) -> Result<KnnReq, String> {
    let node = require_var(&tp.subject, "the subject of vec:nearest")?;
    let (query, k) = parse_obj_args(&tp.object, lists)?;
    Ok(KnnReq {
        node,
        score: None,
        query,
        k,
    })
}

/// Parses `( ?node ?score ) vec:search ( <query> <k> )`.
fn parse_search(tp: &TriplePattern, lists: &ListCells) -> Result<KnnReq, String> {
    let subj = lists.elements(&tp.subject)?;
    let [node, score] = subj.as_slice() else {
        return Err(format!(
            "vec: the subject of vec:search must be a 2-element list ( ?node ?score ), got {} \
             element(s)",
            subj.len()
        ));
    };
    let node = require_var(node, "the first vec:search subject element")?;
    let score = require_var(score, "the second vec:search subject element")?;
    let (query, k) = parse_obj_args(&tp.object, lists)?;
    Ok(KnnReq {
        node,
        score: Some(score),
        query,
        k,
    })
}

/// [SONNET-4.6] (sq-lhcot.4) Parses `( ?node ?score ?rank ?prov ) vec:hybrid ( <query> <k> )`.
/// The subject is prefix-optional: a bare `?node`, or a 1- to 4-element list binding
/// `?node`, `?score`, `?rank`, `?prov` in that order. Every position must be a variable.
fn parse_hybrid(tp: &TriplePattern, lists: &ListCells) -> Result<HybridReq, String> {
    // A bare variable subject is the minimal `?node vec:hybrid ( … )` form; anything else must be
    // a `( … )` list (a non-list, non-variable subject is reported by `ListCells::elements`).
    let subj: Vec<&TermPattern> = match &tp.subject {
        TermPattern::Variable(_) => vec![&tp.subject],
        other => lists.elements(other)?,
    };
    if subj.is_empty() || subj.len() > 4 {
        return Err(format!(
            "vec: the subject of vec:hybrid must be ?node or a 1- to 4-element list \
             ( ?node ?score ?rank ?prov ), got {} element(s)",
            subj.len()
        ));
    }
    let names = [
        "the first vec:hybrid subject element (?node)",
        "the second vec:hybrid subject element (?score)",
        "the third vec:hybrid subject element (?rank)",
        "the fourth vec:hybrid subject element (?prov)",
    ];
    let mut vars = Vec::with_capacity(subj.len());
    for (t, what) in subj.iter().zip(names) {
        vars.push(require_var(t, what)?);
    }
    let mut vars = vars.into_iter();
    let node = vars.next().expect("the list is non-empty");
    let (query, k) = parse_hybrid_obj_args(&tp.object, lists)?;
    Ok(HybridReq {
        node,
        score: vars.next(),
        rank: vars.next(),
        prov: vars.next(),
        query,
        k,
    })
}

/// [SONNET-4.6] (sq-lhcot.4) Decodes `vec:hybrid`'s `( <query> <k> )` object argument list.
fn parse_hybrid_obj_args(
    o: &TermPattern,
    lists: &ListCells,
) -> Result<(HybridQueryArg, usize), String> {
    let args = lists.elements(o)?;
    let [query, k] = args.as_slice() else {
        return Err(format!(
            "vec: the argument list must be ( <query> <k> ) — exactly two elements, got {}",
            args.len()
        ));
    };
    Ok((parse_hybrid_query_arg(query)?, parse_k(k)?))
}

/// [SONNET-4.6] (sq-lhcot.4) `<query>` → an entity IRI, a dense query vector (a PLAIN literal of
/// comma-separated floats, as `vec:nearest`), or a natural-language query (a LANGUAGE-TAGGED
/// literal). The language tag is the disambiguator: it is what distinguishes `"1,2"@en` (the text
/// "1,2") from `"1,2"` (a 2-dimensional query vector), so neither form can be mistaken for the
/// other.
fn parse_hybrid_query_arg(t: &TermPattern) -> Result<HybridQueryArg, String> {
    match t {
        TermPattern::NamedNode(n) => Ok(HybridQueryArg::Node(n.clone())),
        TermPattern::Literal(l) => match l.language() {
            Some(lang) => Ok(HybridQueryArg::Text(
                l.value().to_string(),
                lang.to_string(),
            )),
            None => Ok(HybridQueryArg::Vector(parse_vector_literal(l)?)),
        },
        other => Err(format!(
            "vec: the vec:hybrid query argument must be a node IRI, a vector literal, or a \
             language-tagged text literal, got {other}"
        )),
    }
}

/// Decodes the `( <query> <k> )` object argument list shared by both predicates.
fn parse_obj_args(o: &TermPattern, lists: &ListCells) -> Result<(QueryArg, usize), String> {
    let args = lists.elements(o)?;
    let [query, k] = args.as_slice() else {
        return Err(format!(
            "vec: the argument list must be ( <query> <k> ) — exactly two elements, got {}",
            args.len()
        ));
    };
    Ok((parse_query_arg(query)?, parse_k(k)?))
}

/// `<query>` → an entity IRI or a parsed comma-separated query vector.
fn parse_query_arg(t: &TermPattern) -> Result<QueryArg, String> {
    match t {
        TermPattern::NamedNode(n) => Ok(QueryArg::Node(n.clone())),
        TermPattern::Literal(l) => Ok(QueryArg::Vector(parse_vector_literal(l)?)),
        other => Err(format!(
            "vec: the query argument must be a node IRI or a vector literal, got {other}"
        )),
    }
}

/// A query-vector literal → its floats. [SONNET-4.6] (sq-lhcot.4) Split out of
/// [`parse_query_arg`] so `vec:hybrid` reuses the identical parse (and identical errors) for its
/// plain-literal form.
fn parse_vector_literal(l: &Literal) -> Result<Vec<f32>, String> {
    let v: Result<Vec<f32>, _> = l
        .value()
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect();
    let v = v.map_err(|_| {
        format!(
            "vec: the query literal must be a comma-separated list of floats, got \"{}\"",
            l.value()
        )
    })?;
    if v.is_empty() {
        return Err("vec: the query vector literal is empty".to_string());
    }
    Ok(v)
}

/// `<k>` → a non-negative integer.
fn parse_k(t: &TermPattern) -> Result<usize, String> {
    let TermPattern::Literal(l) = t else {
        return Err(format!(
            "vec: k must be a non-negative integer literal, got {t}"
        ));
    };
    l.value().parse::<usize>().map_err(|_| {
        format!(
            "vec: k must be a non-negative integer, got \"{}\"",
            l.value()
        )
    })
}

/// Requires `t` to be a bare variable; `what` names the position for errors.
fn require_var(t: &TermPattern, what: &str) -> Result<Variable, String> {
    match t {
        TermPattern::Variable(v) => Ok(v.clone()),
        other => Err(format!("vec: {what} must be a variable, got {other}")),
    }
}

/// Maps a dictionary [`Term`] to a [`GroundTerm`] for a VALUES row, or `None`
/// for a blank node (including one nested inside a triple term).
fn term_to_ground(t: Term) -> Option<GroundTerm> {
    match t {
        Term::NamedNode(n) => Some(GroundTerm::NamedNode(n)),
        Term::Literal(l) => Some(GroundTerm::Literal(l)),
        // [GPT-5.6] sq-1ijoc: SPARQL 1.2 VALUES admits ground triple terms;
        // spargebra's conversion recursively rejects any nested blank node.
        Term::Triple(t) => spargebra::term::GroundTriple::try_from(*t)
            .ok()
            .map(|t| GroundTerm::Triple(Box::new(t))),
        Term::BlankNode(_) => None,
    }
}

/// Runs the k-NN search for `req` and returns `(neighbour id, cosine score)`
/// pairs, best first.
///
/// [OPUS-4.8] (sq-z589) The UNFILTERED search runs through `searcher` — the exact full scan
/// ([`ExactSearch`], the default `vec:` behaviour) or an approximate index (`ApproxSearch`, the
/// `*_approx` entry points). The store is still used to look up the seed vector and (for the
/// filtered path) to scan the masked candidates.
///
/// [OPUS-4.8] (sq-bvmd) When the `filtered-ann` feature is on, `constraint` is the
/// BGP's surviving ordinary patterns; if any of them constrain `req.node`, the search
/// is restricted to the candidate id-set those patterns admit (filtered ANN) instead
/// of scanning the whole store and joining afterwards — and that filtered path goes through the
/// cost-model'd `nearest_filtered_costed_tiebreak`, INDEPENDENT of `searcher` (the approximate
/// seam is only the unfiltered scan; the filtered path is always answer-exact and applies the
/// same VG-TIE-1 boundary rule as the unfiltered exact path, with the seed excluded before the
/// boundary is determined — sq-bvmd/sq-7hx6 + sq-tb9p0). When no pattern constrains the
/// neighbour variable — or with `filtered-ann` off — `searcher` runs.
fn run_knn(
    req: &KnnReq,
    graph: &Graph,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    #[cfg(feature = "filtered-ann")] constraint: &[TriplePattern],
) -> Result<Vec<(Id, f32)>, String> {
    // Derive the candidate id-set (IdMask) from the patterns that constrain `req.node`.
    // `None` => no constraining pattern => fall through to the unfiltered path.
    #[cfg(feature = "filtered-ann")]
    let mask = derive_mask(&req.node, constraint, graph)?;

    match &req.query {
        QueryArg::Node(iri) => {
            let term = Term::NamedNode(iri.clone());
            let Some(id) = graph.id_of(&term) else {
                return Ok(Vec::new());
            };
            let Some(query) = store.get(id) else {
                return Ok(Vec::new());
            };
            #[cfg(feature = "filtered-ann")]
            if let Some(mask) = &mask {
                // Filtered: search only BGP-admitted candidates, with the seed excluded
                // from the pool BEFORE the k boundary is determined and boundary-tie
                // membership decided by the VG-TIE-1 N-Triples rule — so the filtered
                // answer equals post-filtering the unfiltered tie-broken retrieval
                // (VG-FILT-2 in answer-exact mode). [OPUS-4.8] (sq-7hx6) The cost model
                // picks pre-filter (scan the mask) vs post-filter (scan the whole store +
                // drop non-members) by the mask's selectivity; BOTH branches return the
                // identical answer either way.
                let (hits, _est) = nearest_filtered_costed_tiebreak(
                    store,
                    graph,
                    query,
                    mask,
                    req.k,
                    Some(id),
                    &CostModel::default(),
                )?;
                return Ok(hits);
            }
            // [SONNET-4.6] (sq-tb9p0) Answer-exact path: rank with the VG-TIE-1 boundary
            // tie-break (seed excluded before ranking, so the boundary is computed over the
            // true candidate pool). A blank-node candidate whose term would decide
            // membership is a fail-closed hard error (`?`), per the embeddable domain.
            if let Some(hits) = searcher.search_tied(query, req.k, graph, Some(id))? {
                return Ok(hits);
            }
            // Approximate backend: over-fetch by one and drop the seed itself (recall < 1 —
            // no true boundary exists to tie-break at).
            Ok(searcher
                .search(query, req.k + 1)
                .into_iter()
                .filter(|&(n, _)| n != id)
                .take(req.k)
                .collect())
        }
        QueryArg::Vector(v) => {
            if v.len() != store.dim() {
                return Err(format!(
                    "vec: query vector has {} dims but the store has {}",
                    v.len(),
                    store.dim()
                ));
            }
            #[cfg(feature = "filtered-ann")]
            if let Some(mask) = &mask {
                // [OPUS-4.8] (sq-7hx6) Cost-model choice of pre-filter vs post-filter —
                // identical answer either way — with boundary-tie membership decided by
                // the VG-TIE-1 N-Triples rule over the admitted pool (no seed to exclude
                // in the query-by-vector form).
                let (hits, _est) = nearest_filtered_costed_tiebreak(
                    store,
                    graph,
                    v,
                    mask,
                    req.k,
                    None,
                    &CostModel::default(),
                )?;
                return Ok(hits);
            }
            // [SONNET-4.6] (sq-tb9p0) Answer-exact path with the VG-TIE-1 boundary tie-break;
            // an approximate backend falls through to its plain search. A blank-node
            // candidate whose term would decide membership is a fail-closed hard error (`?`).
            if let Some(hits) = searcher.search_tied(v, req.k, graph, None)? {
                return Ok(hits);
            }
            Ok(searcher.search(v, req.k))
        }
    }
}

/// [SONNET-4.6] (sq-lhcot.4) Resolves one `vec:hybrid` request into its final ranked hits.
///
/// The pipeline is exactly three deterministic stages:
///
/// 1. **Arms.** The built-in dense arm runs through the SAME path `vec:search` takes — including,
///    with `filtered-ann`, the BGP-derived candidate mask and the VG-TIE-1 boundary tie-break — so
///    hybrid retrieval inherits the k-NN semantics rather than reimplementing them. Each of the
///    caller's arms is then asked for the same number of candidates. Every arm is over-fetched
///    (`cfg.candidates(k)`) so fusion has consensus evidence beyond the final `k`. Every arm's
///    ids are then checked against the graph dictionary (`check_arm_ids`) and, with
///    `filtered-ann`, restricted to the SAME BGP-derived mask the dense arm searched under — the
///    mask is an admissibility constraint on the answer, not a dense-arm optimisation. An arm the
///    mask leaves short is re-asked for a deeper page rather than truncated to whatever survived
///    (see [`run_aux_arms`]), so its ranking really is its top candidates over the admissible
///    domain, not over the first page it happened to return.
/// 2. **Fusion.** Weighted RRF over the named arms, recording which arm ranked each hit and where
///    ([`fuse_arms`]).
/// 3. **Second stage.** The optional out-of-process [`Reranker`](crate::hybrid::Reranker), under
///    its fail-open / fail-closed policy; without one the fused list is simply truncated to `k`.
///
/// A muted dense arm (`vector_weight == 0.0`) is not searched at all — a pure sparse/structural
/// fusion pays no vector cost.
fn run_hybrid(
    req: &HybridReq,
    graph: &Graph,
    store: &VectorStore,
    searcher: &dyn UnfilteredSearch,
    cfg: &HybridConfig<'_>,
    #[cfg(feature = "filtered-ann")] constraint: &[TriplePattern],
) -> Result<Vec<FusedHit>, String> {
    let candidates = cfg.candidates(req.k);

    // Resolve the query into the dense k-NN request plus the view the arms and the reranker see.
    // `dense` is None only when a seed entity has no stored vector: the dense arm then contributes
    // nothing and the remaining arms still answer (the dense arm degrades, never correctness).
    let (knn_query, seed, text, dense) = match &req.query {
        HybridQueryArg::Node(iri) => {
            let dense = graph
                .id_of(&Term::NamedNode(iri.clone()))
                .and_then(|id| store.get(id))
                .map(<[f32]>::to_vec);
            (QueryArg::Node(iri.clone()), Some(iri.clone()), None, dense)
        }
        HybridQueryArg::Vector(v) => {
            check_dim(v.len(), store.dim(), "the query vector literal")?;
            (QueryArg::Vector(v.clone()), None, None, Some(v.clone()))
        }
        HybridQueryArg::Text(t, lang) => {
            let v = cfg.embed_query(t, lang)?;
            check_dim(v.len(), store.dim(), "the configured query embedder")?;
            (
                QueryArg::Vector(v.clone()),
                None,
                Some((t.clone(), lang.clone())),
                Some(v),
            )
        }
    };

    let arm_query = ArmQuery {
        seed: seed.as_ref(),
        text: text.as_ref().map(|(t, l)| (t.as_str(), l.as_str())),
        vector: dense.as_deref(),
    };

    let dense_ranked: Vec<(Id, f64)> = if cfg.dense_weight() == 0.0 {
        Vec::new()
    } else {
        let knn = KnnReq {
            node: req.node.clone(),
            score: None,
            query: knn_query,
            k: candidates,
        };
        #[cfg(feature = "filtered-ann")]
        let hits = run_knn(&knn, graph, store, searcher, constraint)?;
        #[cfg(not(feature = "filtered-ann"))]
        let hits = run_knn(&knn, graph, store, searcher)?;
        hits.into_iter().map(|(id, s)| (id, f64::from(s))).collect()
    };

    let mut arms = vec![ArmRanking::new(
        VECTOR_ARM,
        cfg.dense_weight(),
        dense_ranked,
    )];

    #[cfg(feature = "filtered-ann")]
    let aux = run_aux_arms(cfg, graph, &arm_query, candidates, &req.node, constraint)?;
    #[cfg(not(feature = "filtered-ann"))]
    let aux = run_aux_arms(cfg, graph, &arm_query, candidates)?;
    arms.extend(aux);

    let fused = fuse_arms(&arms, cfg.rrf_constant(), candidates)?;
    match cfg.second_stage() {
        Some((reranker, policy)) => {
            apply_rerank(reranker, policy, &arm_query, fused, req.k)
        }
        None => {
            let mut fused = fused;
            fused.truncate(req.k);
            Ok(fused)
        }
    }
}

/// [OPUS-4.8] (review #4519) Runs the auxiliary arms, validating every id each returns and — with
/// `filtered-ann` — returning each arm's ranking OVER THE ADMISSIBLE CANDIDATES.
///
/// Without `filtered-ann` there is no mask, so this is one pass: ask each arm for `candidates`
/// results and check the ids it returned.
#[cfg(not(feature = "filtered-ann"))]
fn run_aux_arms(
    cfg: &HybridConfig<'_>,
    graph: &Graph,
    query: &ArmQuery<'_>,
    candidates: usize,
) -> Result<Vec<ArmRanking>, String> {
    let aux = cfg.run_arms(query, candidates)?;
    // An auxiliary arm is a CALLER closure — an out-of-process service, a separate index — so the
    // ids it returns are untrusted input to this query. Check them against the dictionary domain
    // before anything downstream consumes them.
    for arm in &aux {
        check_arm_ids(graph, arm)?;
    }
    Ok(aux)
}

/// [OPUS-4.8] (review #4519, round 5) The number of inline-integer literal ids — the sub-domain
/// `[INLINE_BASE, INLINE_BASE + 2^30)` that [`check_arm_ids`] admits alongside the `1..=dict.len()`
/// stored-term ids. `run_aux_arms`'s paging backstop has to bound that whole domain, so it needs
/// the size of the inline half. `sparq-core` does not export it, so it is restated here and pinned
/// against `is_inline` by `the_inline_domain_matches_the_dictionary`.
#[cfg(feature = "filtered-ann")]
const INLINE_DOMAIN: usize = 1 << 30;

/// [OPUS-4.8] (review #4519, round 2) Runs the auxiliary arms and restricts each to the
/// BGP-derived candidate mask, **paging an arm deeper when the restriction leaves it short**.
///
/// The mask is an ADMISSIBILITY constraint on the answer, not a dense-arm optimisation: an id the
/// surrounding BGP cannot bind contributes no solution. Fusing auxiliary arms UNRESTRICTED would
/// let such ids occupy the fused top-k — and the truncation to `k` happens before the join, so a
/// qualifying lower-ranked candidate they evict is lost for good rather than merely reordered — as
/// well as inflating the RRF ranks (and therefore the scores and `?prov` entries) of the hits that
/// do qualify.
///
/// # Why one masked pass is not enough
///
/// Masking a single `candidates`-long response can only COMPACT the prefix the arm chose to
/// return; it cannot make the arm rank over the admissible domain. With `k = 1` and the default
/// over-fetch, an arm that ranks five inadmissible ids above the sole admissible one returns just
/// the first four, masking empties it, and the one real answer is lost — the exact failure the
/// mask exists to prevent, moved one step later. So when the mask leaves an arm short we re-ask
/// THAT arm for a deeper page (doubling), which is the [`ArmFn`](crate::hybrid::ArmFn) paging
/// contract: a prefix-consistent, exhaustion-honest arm converges on its true top-`candidates`
/// over the admissible domain — what the dense arm's filtered search already returns.
///
/// The alternative — handing the mask to the arm so it can filter at its own source — would put a
/// graph-internal id-set into the public arm signature and require every out-of-process arm to
/// honour it; the paging protocol keeps admissibility enforced HERE, where it is not optional.
///
/// # Termination
///
/// Each round doubles the request, and stops at the first of: enough admissible hits
/// (`>= candidates`); every admissible id already collected (`>= mask.len()`, so a deeper page
/// cannot add one); the arm returned fewer results than asked for (exhausted); or the request
/// reached the per-request CEILING — which is a hard, arm-named error, not a quiet short answer.
///
/// The ceiling is the smaller of two bounds, and never below the `candidates` page the caller
/// explicitly asked every arm for:
///
/// - **Correctness** — an arm's ids are distinct ([`validate_arms`](crate::hybrid::validate_arms))
///   and drawn from the domain [`check_arm_ids`] admits (the dictionary ids PLUS the
///   inline-integer literal ids), so no arm can return more than `dict.len() + INLINE_DOMAIN`
///   results and a deeper page than that cannot exist. The inline half is not slack:
///   [`derive_mask`] admits an inline id whenever `?node` is bound in an object position to an
///   integer literal, so an ADMISSIBLE id genuinely can sit beyond `dict.len()` in an arm's
///   ranking — bounding at `dict.len()` would stop while such an id was still unreached, losing it
///   (review #4519, round 5).
/// - **Safety** — [`MAX_ARM_PAGE`], because each request makes the arm MATERIALIZE a `Vec` of that
///   size and ship it here. The correctness bound is ~1.07e9 wide, so doubling towards it would let
///   a small filtered `vec:hybrid` query ask a valid arm for hundreds of millions of results and
///   exhaust the arm, the transport or this process (review #4519, round 6).
///
/// Reaching the safety cap means the arm kept padding full pages while the mask admitted almost
/// none of them, so its top candidates over the admissible domain cannot be established within a
/// bounded budget. Answering anyway would silently drop admissible hits — exactly the loss this
/// loop exists to prevent — so it fails closed and names the arm instead. An exhaustion-honest arm
/// never sees the cap: it stops at its first short page.
#[cfg(feature = "filtered-ann")]
fn run_aux_arms(
    cfg: &HybridConfig<'_>,
    graph: &Graph,
    query: &ArmQuery<'_>,
    candidates: usize,
    node: &Variable,
    constraint: &[TriplePattern],
) -> Result<Vec<ArmRanking>, String> {
    if cfg.arm_count() == 0 {
        // Nothing to restrict, so do not pay the constraining sub-BGP evaluation for it.
        return Ok(Vec::new());
    }
    let Some(mask) = derive_mask(node, constraint, graph)? else {
        // `?node` is unconstrained: every id is admissible and one pass is exact.
        let aux = cfg.run_arms(query, candidates)?;
        for arm in &aux {
            check_arm_ids(graph, arm)?;
        }
        return Ok(aux);
    };

    // The correctness bound is the WHOLE domain an arm may legitimately rank — the dictionary ids
    // and the inline-integer literal ids `check_arm_ids` also admits — because an admissible id can
    // be an inline one; `dict.len()` alone is NOT that domain. But that domain is ~1.07e9 wide and
    // every request costs a materialized `Vec`, so the SAFETY cap wins: escalation stops at
    // `MAX_ARM_PAGE`, and only the caller's own `candidates` page may exceed it.
    let domain = graph.dict.len().saturating_add(INLINE_DOMAIN);
    let ceiling = candidates.max(MAX_ARM_PAGE.min(domain));
    (0..cfg.arm_count())
        .map(|i| {
            let mut want = candidates;
            loop {
                let arm = cfg.run_arm(i, query, want)?;
                check_arm_ids(graph, &arm)?;
                let ArmRanking { arm, weight, ranked } = arm;
                let returned = ranked.len();
                let ranked: Vec<(Id, f64)> = ranked
                    .into_iter()
                    .filter(|(id, _)| mask.contains(*id))
                    .collect();
                if ranked.len() >= candidates || ranked.len() >= mask.len() || returned < want {
                    return Ok(ArmRanking { arm, weight, ranked });
                }
                if want >= ceiling {
                    // Fail closed: the arm padded a full page of `want` results of which only
                    // `ranked.len()` are admissible, so its true top candidates over the admissible
                    // domain are still unknown — and a deeper page is not affordable.
                    return Err(format!(
                        "vec:hybrid: the {:?} arm returned all {} results asked for without \
                         signalling exhaustion, and only {} of them are admissible under the \
                         surrounding BGP; paging it deeper would exceed the {}-result per-request \
                         cap, so its top candidates over the admissible domain cannot be \
                         established. Make the arm exhaustion-honest (return fewer results than \
                         asked once it has no more) or constrain the query further",
                        arm,
                        want,
                        ranked.len(),
                        ceiling
                    ));
                }
                want = want.saturating_mul(2).min(ceiling);
            }
        })
        .collect()
}

/// [OPUS-4.8] (review #4519) Rejects an auxiliary arm's result whose id is not a term of `graph`'s
/// dictionary — a valid 1-based dictionary id (`1..=dict.len()`) or an inline-integer literal id.
///
/// An arm is a caller closure, so its ids are untrusted: an out-of-process service answering from
/// a stale or foreign index can return anything. Left unchecked such an id flows all the way to
/// `graph.dict.term(id)`, which resolves an out-of-domain id to the dictionary's wrong-but-safe
/// OUT-OF-RANGE PLACEHOLDER (a blank node) — and a blank node cannot appear in a `VALUES` row, so
/// the hit is silently dropped from the inlined table. That is exactly the failure mode
/// [`validate_arms`](crate::hybrid::validate_arms) already refuses for a duplicated id: a
/// malformed arm response quietly changing the answer. Fail closed instead, naming the arm so the
/// operator knows which service to fix.
fn check_arm_ids(graph: &Graph, arm: &ArmRanking) -> Result<(), String> {
    let len = graph.dict.len();
    for &(id, _) in &arm.ranked {
        // An inline-integer id encodes its literal value directly and is resolvable against any
        // dictionary; every other id must index a stored term.
        if id == NO_ID || (!is_inline(id) && id as usize > len) {
            return Err(format!(
                "vec:hybrid: the {:?} arm returned id {}, which is not a term in this graph's \
                 dictionary (valid ids are 1..={}, or an inline-integer literal id)",
                arm.arm, id, len
            ));
        }
    }
    Ok(())
}

/// Rejects a query vector whose dimension does not match the store's; `what` names the source of
/// the vector so the error points at the right thing to fix.
fn check_dim(got: usize, want: usize, what: &str) -> Result<(), String> {
    if got != want {
        return Err(format!(
            "vec:hybrid: {} produced {} dims but the store has {}",
            what, got, want
        ));
    }
    Ok(())
}

/// [OPUS-4.8] (sq-3tjd, epic sq-3183) Builds the candidate [`IdMask`] for `node` from the
/// surrounding BGP `constraint` patterns — the dictionary ids of every binding `node` takes
/// when the **join-connected** constraining sub-BGP is evaluated against `graph`, then
/// projected back onto `node`.
///
/// The constraining sub-BGP is the **connected component of the BGP join-graph** that
/// contains `node`: starting from the patterns that mention `node`, follow shared variables
/// transitively into the patterns those reach, and so on. This generalises the original
/// single-variable (#281/sq-bvmd) rule — a *direct-mention-only* component reduces to exactly
/// the old behaviour — to **transitive / multi-variable** constraints: `?node :owns ?x .
/// ?x a :Vehicle` restricts `?node` to subjects that own a `:Vehicle` even though the second
/// pattern never names `?node`.
///
/// Patterns *not* reachable from `node` through shared variables are deliberately excluded:
/// a disconnected pattern joins as a cartesian product, which never *removes* a `node`
/// binding, so pulling it in could only widen (a cross-product can drop a binding only if it
/// is empty — but an empty disconnected BGP would zero the whole join in the outer query too,
/// which is a different concern handled by the engine, not a mask we may narrow with). We
/// only ever **narrow**, never widen: the projected `node` set of the connected component is
/// a superset of (or equal to) the set the full outer BGP would bind, so the filtered top-k
/// stays a subset of the unfiltered top-k and equals post-filtering by the (now transitive)
/// constraint. If `node` is unconstrained (no pattern mentions it) the function returns
/// `Ok(None)` and the caller runs the plain unfiltered search, preserving the existing
/// `vec:` semantics exactly.
///
/// The connected sub-BGP is evaluated through the standard engine (the same seam the outer
/// query uses), so shared join variables are honoured correctly: the mask is exactly the set
/// the engine binds to `node` for that component.
#[cfg(feature = "filtered-ann")]
fn derive_mask(
    node: &Variable,
    constraint: &[TriplePattern],
    graph: &Graph,
) -> Result<Option<IdMask>, String> {
    // The connected component of the BGP join-graph that contains `node`: every pattern
    // join-reachable from `node` through shared variables. A direct-mention-only BGP yields
    // exactly the same patterns the old single-variable rule did (strict no-regression).
    let sub = connected_component(node, constraint);
    if sub.is_empty() {
        return Ok(None);
    }

    // [OPUS-4.8] (sq-36ol) The mask the connected sub-BGP derives is a pure function of (the
    // sub-BGP, the graph) — re-deriving it for the SAME sub-BGP against the SAME graph always
    // yields the same ids. So cache it keyed by (sub-BGP terms, graph fingerprint). Crucially the
    // sub-BGP here is the **connected component** (sq-3tjd) `connected_component` computed above,
    // so a cached entry corresponds to the full TRANSITIVE constraint — caching wraps the
    // transitive derivation, not the old single-variable one. The fingerprint folds dict_len +
    // triple_count + an id-ordered content hash (see `crate::fingerprint`), so ANY mutation that
    // could change which ids the sub-BGP binds — an added/removed triple, a shifted dict id, a
    // changed term — changes the fingerprint and therefore MISSES the cache (recompute). The query
    // variable `?node` is normalised to a fixed name so two queries that differ only in the
    // neighbour variable's name share the entry (the derived id-set is independent of that name).
    // A cache HIT returns an `IdMask` byte-identical to a fresh derivation; a MISS recomputes
    // through the engine as before.
    let fp = Fingerprint::of(graph);
    let key = mask_cache::key(&sub, node);
    if let Some(mask) = mask_cache::get(&key, fp) {
        return Ok(Some(mask));
    }

    // SELECT ?node WHERE { <connected sub-BGP> } — evaluate the constraint through the engine
    // and collect the ids it binds to `node`. Projecting `node` discards the intermediate
    // join variables (e.g. `?x` above), so the mask is precisely the eligible neighbour set.
    let select = Query::Select {
        dataset: None,
        pattern: GraphPattern::Project {
            inner: Box::new(GraphPattern::Bgp { patterns: sub }),
            variables: vec![node.clone()],
        },
        base_iri: None,
    };
    let result = sparq_engine::query_prepared(graph, &PreparedQuery::from(select))?;

    // Map each bound `node` term back to its dictionary id. A row whose `node` is unbound
    // (impossible for a BGP-only projection, but cheap to guard) or whose term is not in
    // the dictionary contributes nothing — it cannot match a stored vector anyway.
    let mask: IdMask = result
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(|c| c.as_ref()))
        .filter_map(|term| graph.id_of(term))
        .collect();
    mask_cache::put(key, fp, mask.clone());
    Ok(Some(mask))
}

/// [OPUS-4.8] (sq-36ol, epic sq-3183) Cross-prepare cache for the filtered-ANN `IdMask` that
/// [`derive_mask`] computes from a constraining sub-BGP. Keyed by **(canonical sub-BGP, graph
/// [`Fingerprint`])**; reused on a hit, invalidated — by construction — on any graph change.
///
/// # Why the invalidation is SOUND (the load-bearing property)
///
/// The cached value is the set of dictionary ids the sub-BGP binds to the neighbour variable
/// against a specific graph. That set is a pure function of (sub-BGP, graph content). The
/// [`Fingerprint`] folds the graph's `dict_len`, `triple_count`, and an **id-ordered content
/// hash** over every dictionary term (see [`crate::fingerprint`]). Any mutation that could
/// change which ids the sub-BGP binds necessarily changes one of those: adding/removing a triple
/// bumps `triple_count`; adding/removing/changing a term changes `dict_len` and/or the content
/// hash; even a pure dict-id *shift* (same terms, different interning order) changes the content
/// hash because each term's id is folded in. A changed fingerprint is a DIFFERENT key, so a
/// post-mutation lookup MISSES and recomputes — a stale mask can never be served. When in doubt
/// we miss: the fingerprint is conservative (it can differ for two graphs that happen to bind the
/// same ids), which only ever costs a recompute, never correctness.
///
/// The cache lives in a `thread_local!` so it needs no change to the `&Graph`/`&VectorStore`
/// shared-reference API and stays internal to the `vec:` rewrite — `sparq-core`/`sparq-engine`
/// are untouched. It is bounded (LRU-ish capacity cap) so a long-lived process running many
/// distinct queries cannot grow it without bound.
#[cfg(feature = "filtered-ann")]
mod mask_cache {
    use super::{Fingerprint, IdMask, TriplePattern, Variable};
    use spargebra::term::TermPattern;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Cache key: the canonicalised constraining sub-BGP. The neighbour variable is renamed to a
    /// fixed placeholder so two otherwise-identical constraints that differ only in the neighbour
    /// variable's *name* share an entry — the derived id-set does not depend on that name.
    #[derive(Clone, PartialEq, Eq, Hash)]
    pub(super) struct Key {
        patterns: Vec<TriplePattern>,
    }

    /// Builds a [`Key`] from the constraining `sub` patterns, normalising occurrences of `node`
    /// to a fixed variable name so the entry is shared across neighbour-variable renamings.
    pub(super) fn key(sub: &[TriplePattern], node: &Variable) -> Key {
        let placeholder = Variable::new_unchecked("\u{0}vec_node");
        let rename = |t: &TermPattern| -> TermPattern {
            match t {
                TermPattern::Variable(v) if v == node => TermPattern::Variable(placeholder.clone()),
                other => other.clone(),
            }
        };
        let patterns = sub
            .iter()
            .map(|tp| TriplePattern {
                subject: rename(&tp.subject),
                predicate: match &tp.predicate {
                    spargebra::term::NamedNodePattern::Variable(v) if v == node => {
                        spargebra::term::NamedNodePattern::Variable(placeholder.clone())
                    }
                    other => other.clone(),
                },
                object: rename(&tp.object),
            })
            .collect();
        Key { patterns }
    }

    /// Soft cap on distinct cached entries. A long-lived process issuing many distinct
    /// constraint/graph combinations clears the cache wholesale on overflow rather than growing
    /// unbounded — correctness is unaffected (a cleared entry just re-derives).
    const CAPACITY: usize = 256;

    thread_local! {
        // (Key, Fingerprint) → mask. The fingerprint is PART of the key so an entry for a stale
        // graph can never be read — a changed graph yields a different fingerprint, hence a miss.
        static CACHE: RefCell<HashMap<(Key, Fingerprint), IdMask>> = RefCell::new(HashMap::new());
    }

    /// Returns the cached mask for `(key, fp)`, or `None` on a miss (different sub-BGP OR a
    /// different graph fingerprint — i.e. any graph mutation).
    pub(super) fn get(key: &Key, fp: Fingerprint) -> Option<IdMask> {
        CACHE.with(|c| c.borrow().get(&(key.clone(), fp)).cloned())
    }

    /// Inserts the freshly-derived mask under `(key, fp)`.
    pub(super) fn put(key: Key, fp: Fingerprint, mask: IdMask) {
        CACHE.with(|c| {
            let mut m = c.borrow_mut();
            if m.len() >= CAPACITY {
                m.clear();
            }
            m.insert((key, fp), mask);
        });
    }

    /// Test-only: drop all entries so a test starts from a known-cold cache.
    #[cfg(test)]
    pub(super) fn clear() {
        CACHE.with(|c| c.borrow_mut().clear());
    }

    /// Test-only: number of live entries (to assert a hit reused rather than re-inserted).
    #[cfg(test)]
    pub(super) fn len() -> usize {
        CACHE.with(|c| c.borrow().len())
    }
}

/// [OPUS-4.8] (sq-3tjd) The connected component of the BGP join-graph containing `node`.
///
/// Models the BGP as a graph whose nodes are *variables* and whose edges link the variables
/// that co-occur in a triple pattern; the component is every pattern reachable from `node`
/// through shared variables. Implemented as a worklist over a frontier of "live" variables:
/// seed with `node`, and repeatedly admit any not-yet-taken pattern that mentions a live
/// variable, adding that pattern's own variables to the frontier — a transitive closure that
/// terminates because each pass either consumes a pattern or grows no further.
///
/// Returns the patterns in the component (empty iff no pattern mentions `node`, i.e. `node`
/// is unconstrained). The order mirrors the input BGP for determinism.
#[cfg(feature = "filtered-ann")]
fn connected_component(node: &Variable, patterns: &[TriplePattern]) -> Vec<TriplePattern> {
    // Live variables whose patterns are (transitively) part of the component.
    let mut live: Vec<Variable> = vec![node.clone()];
    // `taken[i]` once pattern `i` has been pulled into the component.
    let mut taken = vec![false; patterns.len()];

    loop {
        let mut grew = false;
        for (i, tp) in patterns.iter().enumerate() {
            if taken[i] {
                continue;
            }
            let vars = pattern_vars(tp);
            if vars.iter().any(|v| live.contains(v)) {
                taken[i] = true;
                grew = true;
                // Promote this pattern's other variables into the live frontier so the
                // closure follows the join transitively.
                for v in vars {
                    if !live.contains(&v) {
                        live.push(v);
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }

    patterns
        .iter()
        .zip(&taken)
        .filter(|(_, &t)| t)
        .map(|(tp, _)| tp.clone())
        .collect()
}

/// [OPUS-4.8] (sq-3tjd) The variables `tp` mentions, in any term position (subject,
/// predicate, object).
#[cfg(feature = "filtered-ann")]
fn pattern_vars(tp: &TriplePattern) -> Vec<Variable> {
    let mut out = Vec::with_capacity(3);
    if let TermPattern::Variable(v) = &tp.subject {
        out.push(v.clone());
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        out.push(v.clone());
    }
    if let TermPattern::Variable(v) = &tp.object {
        out.push(v.clone());
    }
    out
}

// [OPUS-4.8] (sq-36ol, epic sq-3183) Unit tests for the derived-mask cache. These reach the
// PRIVATE `mask_cache` (entry count, clear) so they can assert a HIT *reused* rather than
// re-derived, and that a graph mutation *misses*. The end-to-end public-API behaviour is also
// covered in tests/filtered_bgp.rs. Gated on both features (the cache only exists with them).
#[cfg(all(test, feature = "filtered-ann"))]
mod mask_cache_tests {
    use super::*;
    use sparq_core::dict::{Id, INLINE_BASE};

    /// [OPUS-4.8] (review #4519, round 5) `INLINE_DOMAIN` restates a `sparq-core` invariant that
    /// crate does not export, and `run_aux_arms`'s paging backstop is only sound if it is exact:
    /// too small and an admissible inline id past the bound is never paged to. Pin it against the
    /// dictionary's own `is_inline` — the last inline id must be inline, the next one must not be.
    #[test]
    fn the_inline_domain_matches_the_dictionary() {
        let last = INLINE_BASE + (INLINE_DOMAIN as Id - 1);
        assert!(is_inline(last), "id {} must be the last inline id", last);
        assert!(
            !is_inline(INLINE_BASE + INLINE_DOMAIN as Id),
            "the inline sub-domain must hold exactly {} ids",
            INLINE_DOMAIN
        );
    }

    /// A constraining sub-BGP `?node <kind> :Car` over the given variable name.
    fn car_constraint(node: &str) -> (Variable, Vec<TriplePattern>) {
        let node = Variable::new_unchecked(node);
        let tp = TriplePattern {
            subject: TermPattern::Variable(node.clone()),
            predicate: NamedNodePattern::NamedNode(NamedNode::new("http://ex/kind").unwrap()),
            object: TermPattern::NamedNode(NamedNode::new("http://ex/Car").unwrap()),
        };
        (node, vec![tp])
    }

    /// Sorted ids of a mask, for an order-independent identity comparison (IdMask is a set).
    fn sorted(mask: &IdMask) -> Vec<Id> {
        let mut v: Vec<Id> = mask.iter().collect();
        v.sort_unstable();
        v
    }

    fn cars_graph() -> Graph {
        Graph::load_str(
            "<http://ex/a> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/b> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/d> <http://ex/kind> <http://ex/Boat> .\n",
            "ntriples",
        )
        .unwrap()
    }

    /// A HIT returns a mask byte-identical (same id set) to a fresh derivation, and reuses the
    /// SAME entry (no second insertion) — proving the second prepare didn't re-run the engine.
    #[test]
    fn hit_returns_identical_mask_without_reinserting() {
        mask_cache::clear();
        let g = cars_graph();
        let (node, sub) = car_constraint("node");

        let first = derive_mask(&node, &sub, &g).unwrap().unwrap();
        let after_first = mask_cache::len();
        assert_eq!(after_first, 1, "first derivation must populate one entry");

        let second = derive_mask(&node, &sub, &g).unwrap().unwrap();
        assert_eq!(
            mask_cache::len(),
            after_first,
            "a HIT must reuse the entry, not insert another"
        );
        assert_eq!(
            sorted(&first),
            sorted(&second),
            "the cached mask must be identical to the freshly-derived one"
        );
        // {a, b} are the Cars.
        assert_eq!(sorted(&first).len(), 2, "two Cars in the fixture");
    }

    /// ADVERSARIAL invalidation: the same constraining sub-BGP against a MUTATED graph must NOT
    /// serve the stale mask. We construct a second graph that adds a Car (`c`) — the mask must
    /// now include it. Because the fingerprint folds triple_count + an id-ordered content hash,
    /// the post-mutation lookup is a different key → a miss → a recompute → the CHANGED mask.
    ///
    /// The mutation is deliberately the hard case: the new graph keeps every original triple AND
    /// every original (a, b, d) dict id binding, adding only `c`. A naive cache keyed on the
    /// sub-BGP alone (or on triple_count alone, had the change been a pure dict-id permutation)
    /// would wrongly hit. The full fingerprint catches it.
    #[test]
    fn mutation_invalidates_and_recomputes_changed_mask() {
        mask_cache::clear();
        let (node, sub) = car_constraint("node");

        let g1 = cars_graph();
        let mask1 = derive_mask(&node, &sub, &g1).unwrap().unwrap();
        assert_eq!(sorted(&mask1).len(), 2, "g1 has two Cars (a, b)");

        // Mutation: add a third Car `c` (a new triple AND a new term). Anything that could change
        // the bound id-set changes the fingerprint.
        let g2 = Graph::load_str(
            "<http://ex/a> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/b> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/c> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/d> <http://ex/kind> <http://ex/Boat> .\n",
            "ntriples",
        )
        .unwrap();

        let mask2 = derive_mask(&node, &sub, &g2).unwrap().unwrap();
        assert_eq!(
            sorted(&mask2).len(),
            3,
            "after adding Car c the mask must include it — NOT the stale 2-Car set"
        );
        // The stale mask must never be served: the new mask differs from the old one.
        assert_ne!(
            sorted(&mask1),
            sorted(&mask2),
            "the post-mutation mask must differ from the pre-mutation (stale) one"
        );
        // And it resolves to the live graph's Car ids (soundness against g2's dictionary).
        for iri in ["http://ex/a", "http://ex/b", "http://ex/c"] {
            let id = g2
                .id_of(&Term::NamedNode(NamedNode::new(iri).unwrap()))
                .unwrap();
            assert!(mask2.contains(id), "{iri} must be admitted by the g2 mask");
        }
    }

    /// ADVERSARIAL: two graphs that bind the SAME Car IRIs but have DIFFERENT fingerprints must
    /// occupy distinct cache entries — the fingerprint is PART of the key, so the cache can never
    /// serve g1's mask for a query against g2 even when the constraining sub-BGP is byte-identical.
    /// Here g2 adds an unrelated triple (`x p y`): the Car set {a,b} is unchanged but the
    /// fingerprint (triple_count + content hash) differs, which is the exact soundness property —
    /// the cache keys on the WHOLE-graph fingerprint, conservatively missing whenever it changes.
    #[test]
    fn different_fingerprint_same_constraint_misses() {
        mask_cache::clear();
        let (node, sub) = car_constraint("node");

        let g1 = Graph::load_str(
            "<http://ex/a> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/b> <http://ex/kind> <http://ex/Car> .\n",
            "ntriples",
        )
        .unwrap();
        // g2: same two Cars PLUS an unrelated triple → same Car id-set, DIFFERENT fingerprint.
        let g2 = Graph::load_str(
            "<http://ex/a> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/b> <http://ex/kind> <http://ex/Car> .\n\
             <http://ex/x> <http://ex/p> <http://ex/y> .\n",
            "ntriples",
        )
        .unwrap();
        // Precondition: the two graphs genuinely differ in fingerprint (the cache key's graph half).
        assert_ne!(
            Fingerprint::of(&g1),
            Fingerprint::of(&g2),
            "the two graphs must have distinct fingerprints for this test to be meaningful"
        );

        let _m1 = derive_mask(&node, &sub, &g1).unwrap().unwrap();
        let entries_after_g1 = mask_cache::len();
        let m2 = derive_mask(&node, &sub, &g2).unwrap().unwrap();
        // A different fingerprint → a second, distinct entry (the g1 mask was not served).
        assert_eq!(
            mask_cache::len(),
            entries_after_g1 + 1,
            "a different graph fingerprint must MISS and insert a fresh entry (never reuse a stale one)"
        );
        // The g2 mask must resolve to g2's own ids for a and b.
        for iri in ["http://ex/a", "http://ex/b"] {
            let id = g2
                .id_of(&Term::NamedNode(NamedNode::new(iri).unwrap()))
                .unwrap();
            assert!(m2.contains(id), "{iri} must be admitted against g2's dict");
        }
    }

    /// The key distinguishes DIFFERENT sub-BGPs against the same graph: a `:Car` constraint and a
    /// `:Boat` constraint must not share an entry, and must yield different masks.
    #[test]
    fn distinct_sub_bgps_do_not_collide() {
        mask_cache::clear();
        let g = cars_graph();

        let (node, car_sub) = car_constraint("node");
        let boat_sub = vec![TriplePattern {
            subject: TermPattern::Variable(node.clone()),
            predicate: NamedNodePattern::NamedNode(NamedNode::new("http://ex/kind").unwrap()),
            object: TermPattern::NamedNode(NamedNode::new("http://ex/Boat").unwrap()),
        }];

        let car_mask = derive_mask(&node, &car_sub, &g).unwrap().unwrap();
        let boat_mask = derive_mask(&node, &boat_sub, &g).unwrap().unwrap();
        assert_eq!(
            mask_cache::len(),
            2,
            "two distinct sub-BGPs must occupy two distinct entries"
        );
        assert_ne!(
            sorted(&car_mask),
            sorted(&boat_mask),
            "the Car and Boat constraints must derive different masks"
        );
        assert_eq!(sorted(&car_mask).len(), 2); // a, b
        assert_eq!(sorted(&boat_mask).len(), 1); // d
    }

    /// Two queries that differ ONLY in the neighbour variable's name share an entry (the derived
    /// id-set is independent of that name) — the key normalises the neighbour variable.
    #[test]
    fn neighbour_variable_rename_shares_entry() {
        mask_cache::clear();
        let g = cars_graph();

        let (node_a, sub_a) = car_constraint("node");
        let (node_b, sub_b) = car_constraint("entity"); // same shape, different var name

        let ma = derive_mask(&node_a, &sub_a, &g).unwrap().unwrap();
        let mb = derive_mask(&node_b, &sub_b, &g).unwrap().unwrap();
        assert_eq!(
            mask_cache::len(),
            1,
            "a neighbour-variable rename must share the cache entry"
        );
        assert_eq!(sorted(&ma), sorted(&mb));
    }
}

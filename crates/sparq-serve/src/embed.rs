//! [OPUS-4.8] sq-xa15c (#1248 items 1+2) — the **in-process embedding seam**.
//!
//! A small, documented facade that lets an external host (e.g. `solid-server-rs`,
//! the PSS agent's experimental Rust Solid server) call the sparq engine
//! **in-process** instead of over the SPARQL-1.1-over-HTTP transport — dropping the
//! HTTP hop, the results (de)serialise, the connection pool, and the serial
//! round-trips of a multi-resource read.
//!
//! Everything here is a **thin re-export / one-line wrapper** over already-public
//! entry points — nothing is reimplemented, and binding to this module instead of
//! reaching into `sparq-core` / `sparq-engine` directly buys the embedder ONE place
//! that the maintainer intends to keep stable.
//!
//! # The two halves
//!
//! 1. **The data-path facade** (`query_json` / `query`, `ask`, `update_in_place`,
//!    `apply_delta_nquads`, `exists` / `metadata`) — the read/write/probe calls a
//!    per-resource Solid backend makes against one `sparq_core::Graph`.
//! 2. **The concurrency wrapper** — the runtime-agnostic [`GenerationRing`] (+
//!    [`GraphApplier`], [`Writer`]) re-exported from [`crate`] so an embedder gets
//!    sparq's *tested* fork → update → publish + generation-pinning read/write
//!    concurrency model instead of re-implementing (and diverging from) it. The
//!    ring already lives in this crate (the runtime-agnostic core was factored out
//!    of `sparq-server`'s axum/tokio glue); the embedding seam is the documented
//!    front door to it.
//!
//! # Stability — **API tier-1 (proposed-stable)**, semver freeze PENDING ratification
//!
//! This whole module is the proposed **tier-1** (semver-stable) embedding surface in the
//! [API stability & deprecation policy]. It is the surface the maintainer **intends** to
//! commit to as the stable embedding API (the request in [issue #1248]). It is **NOT yet a
//! frozen semver-tier-1 surface**: the formal commitment is the maintainer's to ratify on
//! #1248, and until that ratification a minor pre-`1.0` release MAY still change it — the
//! marker asserts a *proposal*, not an active guarantee. It is documented as the embedding
//! seam so an embedder pins ONE module rather than a scatter of functions the underlying
//! crates mark "unstable", and so the freeze, when ratified, has a single well-defined
//! shape. No behaviour here is new: every call is the same engine path `sparq-server`
//! already exercises in production.
//!
//! [API stability & deprecation policy]: https://github.com/sparq-org/sparq/blob/main/docs/api-stability.md
//! [issue #1248]: https://github.com/sparq-org/sparq/issues/1248
//!
//! # Example — embed one resource graph behind the generation ring
//!
//! ```
//! use sparq_serve::embed::{self, GenerationRing, GraphApplier};
//! use sparq_core::Graph;
//!
//! // A per-resource (named-graph-per-document) store, seeded once.
//! let mut g = Graph::load_str(
//!     "<http://ex/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .",
//!     "ntriples",
//! ).unwrap();
//!
//! // Read path: ASK / SELECT-as-JSON, no HTTP, no serialise round-trip.
//! assert!(embed::ask(&g, "ASK { ?s ?p ?o }").unwrap());
//! assert_eq!(embed::exists(&g), true);
//!
//! // Write path: apply a SPARQL Update in place …
//! embed::update_in_place(
//!     &mut g,
//!     "INSERT DATA { <http://ex/bob> <http://xmlns.com/foaf/0.1/name> \"Bob\" }",
//! ).unwrap();
//! assert_eq!(embed::metadata(&g).triples, 2);
//!
//! // … or publish it through the ring for snapshot-isolated concurrent reads.
//! let ring = GenerationRing::new(g);
//! let pinned = ring.current(); // a reader pins this snapshot
//! assert_eq!(pinned.snapshot().len(), 2);
//! ```

use sparq_core::Graph;
use sparq_engine::{QueryBudget, QueryResult};

// ── The concurrency wrapper (item 2): the runtime-agnostic core, re-exported ──
//
// These are the SAME types `sparq-server` wraps behind its axum endpoint; an
// embedder gets the tested fork → update → publish + generation-pinning model
// without an HTTP/async dependency. Re-exported here so the embedding seam is one
// `use sparq_serve::embed::*;` rather than a scatter across the crate root.
pub use crate::{
    ApplyUpdates, Generation, GenerationRing, GraphApplier, PodId, RingConfig, TimeTravelConfig,
    WriteError, Writer, WriterConfig,
};

// ── The data-path facade (item 1): thin wrappers over the engine entry points ──

/// Evaluate a SELECT/ASK query against `graph`, serialised directly to a **SPARQL
/// 1.1 Query Results JSON** string (the `query(&Graph, sparql) -> Json` shape).
///
/// This is the fast path the server itself returns to clients — it skips the
/// intermediate [`QueryResult`] and its per-cell term allocation. For an embedder
/// that wants the structured rows instead of JSON bytes, use [`query`].
///
/// Wraps [`sparq_engine::query_json`]; identical behaviour.
#[inline]
pub fn query_json(graph: &Graph, sparql: &str) -> Result<String, String> {
    sparq_engine::query_json(graph, sparql)
}

/// [`query_json`] under a cooperative [`QueryBudget`] (deadline / max result rows /
/// max bytes) — the embedder-facing knob for bounding an untrusted query.
#[inline]
pub fn query_json_with_budget(
    graph: &Graph,
    sparql: &str,
    budget: &QueryBudget,
) -> Result<String, String> {
    sparq_engine::query_json_with_budget(graph, sparql, budget)
}

/// Evaluate a SELECT/ASK query against `graph`, returning the structured
/// [`QueryResult`] (variables + rows of [`oxrdf::Term`]) rather than JSON bytes.
///
/// Wraps [`sparq_engine::query`]; identical behaviour. Use [`query_json`] when the
/// caller just needs the JSON body to hand back over a wire.
#[inline]
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    sparq_engine::query(graph, sparql)
}

/// Evaluate an ASK query: `true` iff the pattern has at least one solution. The
/// engine early-exits (effectively `LIMIT 1`) where it has a streaming path.
///
/// Wraps [`sparq_engine::ask`]; identical behaviour.
#[inline]
pub fn ask(graph: &Graph, sparql: &str) -> Result<bool, String> {
    sparq_engine::ask(graph, sparql)
}

/// Apply a SPARQL 1.1 Update to `graph` **in place** (the
/// `update_in_place(&mut Graph, sparql)` shape) — the O(batch) delta-overlay write
/// path. On `Err` the graph may hold a partially applied prefix of the update's
/// operations (the same hazard `sparq-server`'s sequenced writer handles by
/// re-fork-and-replay); an embedder that needs all-or-nothing on one graph should
/// use [`update_in_place_atomic`].
///
/// Wraps [`sparq_engine::update_in_place`]; identical behaviour.
#[inline]
pub fn update_in_place(graph: &mut Graph, sparql: &str) -> Result<(), String> {
    sparq_engine::update_in_place(graph, sparql)
}

/// [`update_in_place`] with all-or-nothing semantics on a single graph: a failing
/// operation leaves `graph` exactly as it was (no partial prefix applied).
///
/// Wraps [`sparq_engine::update_in_place_atomic`]; identical behaviour.
#[inline]
pub fn update_in_place_atomic(graph: &mut Graph, sparql: &str) -> Result<(), String> {
    sparq_engine::update_in_place_atomic(graph, sparql)
}

/// Apply a quad-level delta to the whole dataset from two **N-Quads** documents
/// (`deletes` applied first, then `inserts`), grouped and applied per graph —
/// default-graph statements to `graph`, named-graph statements to the matching
/// named sub-graph (auto-created on first insert; deleting from an absent graph is a
/// no-op). O(batch) through each graph's delta overlay, no index rebuild.
///
/// Unlike SPARQL `DELETE DATA`, blank nodes here denote concrete nodes **by label**,
/// so quad-level retraction of blank-node triples is possible — the property a
/// Solid `PATCH` / per-resource-replace backend relies on.
///
/// Wraps [`Graph::apply_delta_nquads`]; identical behaviour.
#[inline]
pub fn apply_delta_nquads(graph: &mut Graph, inserts: &str, deletes: &str) -> Result<(), String> {
    graph.apply_delta_nquads(inserts, deletes)
}

/// A cheap existence/metadata probe over `graph` — the "does this resource exist
/// and how big is it" check a per-resource backend makes before/without a full
/// query (e.g. to answer a Solid `HEAD` or to decide create-vs-update).
///
/// `triples` is the default-graph triple count; `named_graphs` is the number of
/// distinct named graphs (named-graph-per-document deployments map one resource to
/// one named graph). Both are O(1)/O(graphs) reads, not a query evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// Triples in the default graph ([`Graph::len`]).
    pub triples: usize,
    /// Distinct named graphs in the dataset.
    pub named_graphs: usize,
}

/// Existence probe: `true` iff `graph`'s default graph holds at least one triple.
///
/// Equivalent to `!graph.is_empty()`; named here as the embedding seam's
/// resource-existence check so the intent (and the stable name) is explicit.
#[inline]
pub fn exists(graph: &Graph) -> bool {
    !graph.is_empty()
}

/// `true` iff `graph` contains the named graph `name` — the per-resource existence
/// check in a named-graph-per-document deployment, where one Solid resource maps to
/// one named graph addressed by its IRI.
///
/// Wraps [`Graph::named_graph`]`(name).is_some()`.
#[inline]
pub fn named_graph_exists(graph: &Graph, name: &oxrdf::Term) -> bool {
    graph.named_graph(name).is_some()
}

/// The cheap [`Metadata`] probe over `graph` (default-graph triple count + named
/// graph count). No query evaluation; see [`Metadata`] for the field meanings.
#[inline]
pub fn metadata(graph: &Graph) -> Metadata {
    Metadata {
        triples: graph.len(),
        named_graphs: graph.named.len(),
    }
}

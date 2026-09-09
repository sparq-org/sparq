#![doc = include_str!("../README.md")]
#![warn(clippy::undocumented_unsafe_blocks)]

// [OPUS-4.8] (sq-a9cn) Opt-in materialised-view / query-result cache. NON-DEFAULT
// `result-cache` feature — when off, zero cache code compiles and the default native +
// wasm builds are byte-identical (no new deps).
#[cfg(feature = "result-cache")]
pub mod cache;
// [FABLE-5] (sq-7d3dj.30.14) Membership-cluster pre-materialisation for the greedy BGP
// planner (SP2Bench q07). NON-DEFAULT `cluster-materialize` feature — when off, zero of
// this code compiles and the default native + wasm builds are byte-identical. Pure
// join-order choice; results are identical either way (differentially tested).
#[cfg(feature = "cluster-materialize")]
pub(crate) mod cluster;
// [FABLE-5] (sq-7d3dj.30.14) Test-only hook so an integration differential can force the
// membership-cluster planner path on a small graph. Not part of the stable query API.
#[cfg(feature = "cluster-materialize")]
#[doc(hidden)]
pub use cluster::with_test_thresholds;
mod construct;
#[cfg(feature = "cs-planner")]
pub mod cs;
#[cfg(all(test, feature = "cs-planner"))]
mod cs_gate;
mod dataset;
mod exec;
mod explain;
#[cfg(feature = "persistent-stats")]
pub mod stats;
// [OPUS-4.8] (sq-u4lgr, #902) Structured EXPLAIN: typed `PlanNode` plan tree + JSON +
// per-operator q-error + bounded slow-query ring. NON-DEFAULT `explain-json` feature —
// when off, zero of this code compiles and the default native + wasm builds are
// byte-identical (no new deps; the human-readable text EXPLAIN stays the default).
#[cfg(feature = "explain-json")]
pub mod explain_json;
pub mod json;
// [OPUS-4.8] (sq-rp3um) Parameterized prepared queries — safe value binding
// (`PreparedQuery::bind` / `PreparedUpdate`) to prevent SPARQL injection (#901).
// NON-DEFAULT `params` feature; when off, zero of this code compiles. Pulls in no
// new deps (oxrdf + spargebra are already direct deps).
#[cfg(feature = "params")]
pub mod params;
// [GPT-5.6] (sq-lsp7k.3.1) Opt-in programmatic full-path enumeration. NON-DEFAULT
// and dependency-free, so feature-off engine and wasm builds remain unchanged.
#[cfg(feature = "paths")]
pub mod paths;
#[cfg(feature = "paths")]
pub use paths::{enumerate_paths, Endpoint, PathMode, PathSolution, PathSpec, Via};
// [GPT-5.6] (sq-lsp7k.3.2) Dedicated non-standard PATHS syntax. The ordinary
// SPARQL parser remains untouched, so this surface is available only by explicit opt-in.
#[cfg(feature = "paths")]
mod paths_syntax;
#[cfg(feature = "paths")]
pub use paths_syntax::{explain_paths, query_paths};
// [OPUS-4.8] (sq-7d3dj.30.1) Pre-execution SPARQL algebra rewrite pass (result-equivalent
// FILTER `?v = <iri>` constant-substitution + `OPTIONAL … !bound` → anti-join). NON-DEFAULT
// `algebra-rewrite` feature; when off, zero of this code compiles, `PreparedQuery::parse`
// takes the algebra verbatim, and the default native + wasm builds are byte-identical. Pulls
// in no new deps (oxrdf + spargebra are already direct deps).
#[cfg(feature = "algebra-rewrite")]
pub mod rewrite;
// SPARQL 1.1 federated query (SERVICE). NON-DEFAULT `service` feature; pulls a
// blocking HTTP client (ureq) + serde_json, both gated off wasm. When off, zero
// federation code compiles. [OPUS-4.8]
// [OPUS-4.8] (sq-6vshe.4) Seam A2 of the facade split (RFC research/engine-split-rfc.md §4
// Option A / §7 Phase A2): the `service` module moved to the internal `sparq-engine-service`
// sub-crate. The executor references it as `sparq_engine_service::service::*`; the facade
// re-exports its public fns verbatim below, so every public path (`sparq_engine::
// with_service_egress_allow`, …) + the `service` feature NAME are preserved for external users.
// SSRF default-deny egress filter + opt-in allowlist for SERVICE federation
// (threat-model B4 / sq-2v6f). [OPUS-4.8]
#[cfg(feature = "service")]
pub use sparq_engine_service::service::with_service_egress_allow;
// [OPUS-4.8] (sq-iu0c) Stable marker substring in every SERVICE egress-refusal engine
// error, so a network-exposed host (sparq-server) can classify a blocked SERVICE as a
// policy refusal (403-style) rather than a server fault (500), mirroring the existing
// `"query budget exceeded (timeout)"` → 503 pattern.
#[cfg(feature = "service")]
pub use sparq_engine_service::service::SERVICE_EGRESS_REFUSED_MARKER;
// Strict allowlist-only egress policy: only listed hosts reachable (even public ones
// off the list are refused). The network-exposed server wires this to --service-allow
// so federation is restricted to operator-configured endpoints. [OPUS-4.8] (sq-4w18)
#[cfg(feature = "service")]
pub use sparq_engine_service::service::with_service_egress_policy;
// [OPUS-4.8] (sq-vbnyc) The SERVICE-egress per-entry host:port matching rule, exposed as a
// pure function so `sparq-fedclient`'s independent egress guard adopts the SAME port-scoping
// semantics (port-0/overflow/IPv6-bracket/trailing-colon all handled identically) instead of
// keeping a second, divergent copy of the allowlist-matching logic — one source of truth.
#[cfg(feature = "service")]
pub use sparq_engine_service::service::{allowlist_entry_host_matches, allowlist_entry_permits};
// Bind-join (VALUES pushdown) block-size knob — the only OPT-IN tunable for the
// SERVICE bound-join pushdown (on-by-default, correctness-preserving). [OPUS-4.8] (sq-sjkj)
#[cfg(feature = "service")]
pub use sparq_engine_service::service::with_service_bound_join_block_size;
// Per-query remote-request cap for a high-cardinality `SERVICE ?ep` (endpoint var bound
// to many distinct IRIs): an OPT-IN ceiling on the distinct endpoints one SERVICE ?ep
// evaluation may dial, enforced PRE-HTTP (a typed refusal, not post-hoc cancellation).
// DEFAULT is uncapped, so normal SERVICE queries are unchanged. [OPUS-4.8] (sq-b93pv)
#[cfg(feature = "service")]
pub use sparq_engine_service::service::{with_service_remote_request_cap, SERVICE_REMOTE_CAP_MARKER};
// [OPUS-4.8] (sq-678h, sq-6vshe.4) RDF serializer matrix (Turtle / TriG / N-Quads / JSON-LD
// writers). NON-DEFAULT `serialize-rdf` feature — when off, zero serializer code compiles and
// the default build's dependency graph is unchanged. Seam 1 of the facade split (RFC
// research/engine-split-rfc.md §4 Option A / §7 Phase A1): the module moved to the internal
// `sparq-engine-serialize` sub-crate and is re-exported VERBATIM here, so every existing public
// path (`sparq_engine::serialize::write_turtle`, `…::graph_to_jsonld`, `…::JsonLdForm`, …) is
// preserved. The always-on N-Triples writer stays in `construct::triples_to_ntriples`.
#[cfg(feature = "serialize-rdf")]
pub use sparq_engine_serialize::serialize;
mod update;
// zk-trace seam (NON-DEFAULT `zk` feature; consumed only by `sparq-zk`).
// When off, zero zk code is compiled — default builds and wasm are untouched.
#[cfg(feature = "zk")]
pub mod zk;
pub use construct::{
    construct, construct_ntriples, construct_ntriples_with_budget, construct_or_describe,
    construct_or_describe_with_budget, construct_prepared, construct_prepared_with_budget,
    construct_with_budget, describe, describe_prepared, describe_prepared_with_budget,
    describe_with_budget, triples_to_ntriples,
};
// [OPUS-4.8] (sq-it1x) Opt-in MVCC / ACID transaction isolation. NON-DEFAULT `txn`
// feature — when off, zero transaction code compiles and the default native + wasm builds
// are byte-identical (no new deps). Built on the COW delta-overlay substrate
// (`Graph::fork`/`snapshot`/`apply_delta`).
#[cfg(feature = "txn")]
pub mod txn;
#[cfg(feature = "cs-planner")]
pub use cs::{with_cs_table, CsSet, CsTable};
// [OPUS-4.8] (sq-iywur) Opt-in DPccp dynamic-programming join-order enumerator — gated
// on the non-default `dp-planner` feature. `with_dp_planner`(`_budget`) installs it per
// thread (like `with_cs_table`); the default planner stays greedy GOO. Order-only: it
// changes join order, never the query answer. When off, zero DP code compiles and the
// default native + wasm builds are byte-identical.
#[cfg(feature = "dp-planner")]
pub mod dp;
#[cfg(feature = "dp-planner")]
pub use dp::{with_dp_planner, with_dp_planner_budget, without_dp_planner};
pub use explain::{explain, explain_analyze, explain_analyze_with_budget};
// [OPUS-4.8] (sq-u4lgr, #902) Structured EXPLAIN re-exports — gated on `explain-json`.
// `set_trace_clock` (sq-vx7ez, #2428) lets a host without a monotonic `Instant`
// (wasm32) supply the ANALYZE trace clock, e.g. `performance.now()`.
#[cfg(feature = "explain-json")]
pub use explain_json::{
    explain_plan, explain_plan_analyze, explain_plan_analyze_with_budget, set_trace_clock,
    PlanNode, SlowQuery, SlowQueryRing,
};
pub use update::{
    apply_effects, update, update_in_place, update_in_place_atomic,
    update_in_place_atomic_with_budget, update_in_place_capturing, update_in_place_with_budget,
    with_load_base, UpdateEffect,
};

/// Test/measurement hooks for sideways information passing (SIP) — the correlated
/// graph-pattern join optimisation (bead sq-7d3dj.30.3). SIP is a semantics-preserving
/// perf optimisation that is always on in normal evaluation; these hooks exist so the
/// differential acceptance test can force it OFF and read whether it fired. Not part of
/// the stable query API. [OPUS-4.8]
#[doc(hidden)]
pub mod sip_testing {
    /// Enables/disables SIP on the current thread, returning the previous value.
    pub fn set_enabled(v: bool) -> bool {
        crate::exec::sip::set_enabled(v)
    }

    /// Clears the per-query SIP firing statistics.
    pub fn reset_stats() {
        crate::exec::sip::reset_stats()
    }

    /// `(fired, correlated_child_rows, distinct_bindings_evaluated)` since the last
    /// [`reset_stats`].
    pub fn stats() -> (bool, usize, usize) {
        crate::exec::sip::stats()
    }
}

/// Test-only surface for the DISTINCT-projection loose skip-scan (bead sq-7d3dj.30.4):
/// toggle the pushdown and read its per-query statistics. NOT part of the stable query
/// API. [OPUS-4.8]
#[doc(hidden)]
pub mod distinct_pushdown_testing {
    /// Enables/disables the DISTINCT-projection pushdown on the current thread, returning
    /// the previous value.
    pub fn set_enabled(v: bool) -> bool {
        crate::exec::distinct_pushdown::set_enabled(v)
    }

    /// Clears the per-query pushdown statistics.
    pub fn reset_stats() {
        crate::exec::distinct_pushdown::reset_stats()
    }

    /// `(fired, distinct_values_emitted, permutation_rows_scanned)` since the last
    /// [`reset_stats`].
    pub fn stats() -> (bool, usize, usize) {
        crate::exec::distinct_pushdown::stats()
    }
}

/// Test-only surface for the OPT-IN characteristic-set anchor-incidence prune (bead
/// `sq-jnb1e`, the `cs-anchor-incidence` feature): toggle the prune and read whether it fired
/// plus how many candidate predicates it eliminated, so the differential acceptance test can
/// compare the incidence-pruned block scan against the exact scan within one binary. NOT part
/// of the stable query API. [FABLE-5]
#[cfg(feature = "cs-anchor-incidence")]
#[doc(hidden)]
pub mod anchor_incidence_testing {
    /// Enables/disables the incidence prune on the current thread, returning the previous
    /// value.
    pub fn set_enabled(v: bool) -> bool {
        crate::exec::anchor_incidence::set_enabled(v)
    }

    /// Clears the per-query incidence statistics.
    pub fn reset_stats() {
        crate::exec::anchor_incidence::reset_stats()
    }

    /// `(incidence_built, predicates_pruned)` since the last [`reset_stats`].
    pub fn stats() -> (bool, usize) {
        crate::exec::anchor_incidence::stats()
    }
}

/// Test/measurement hooks for the correlated (theta) anti-join — the
/// `OPTIONAL … FILTER(!bound(?nb))` negation idiom with a correlated inner FILTER
/// (bead sq-7d3dj.30.9, SP2Bench q06). A semantics-preserving perf optimisation that
/// is always on in normal evaluation; these hooks exist so the differential
/// acceptance test can force it OFF and read whether it fired. NOT part of the stable
/// query API. [FABLE-5]
#[doc(hidden)]
pub mod theta_antijoin_testing {
    /// Enables/disables the correlated theta anti-join on the current thread,
    /// returning the previous value.
    pub fn set_enabled(v: bool) -> bool {
        crate::exec::theta_antijoin::set_enabled(v)
    }

    /// Clears the per-query firing statistics.
    pub fn reset_stats() {
        crate::exec::theta_antijoin::reset_stats()
    }

    /// `(fired, correlated_right_rows, distinct_correlations_evaluated)` since the
    /// last [`reset_stats`].
    pub fn stats() -> (bool, usize, usize) {
        crate::exec::theta_antijoin::stats()
    }

    /// [OPUS-4.8] (sq-7d3dj.30.20) Number of anti-join shapes the STATIC early-decline gate
    /// (opt-in `antijoin-static-decline`) rejected BEFORE evaluating the mandatory left side
    /// since the last [`reset_stats`] — i.e. the redundant left evaluations the feature saved.
    /// Compiled only with the feature on; the differential test asserts it is `> 0` on the
    /// SP2Bench-q07 shape (the gate fired) and `0` on a correlated (q06) shape (it did not).
    #[cfg(feature = "antijoin-static-decline")]
    pub fn early_declined() -> usize {
        crate::exec::theta_antijoin::early_declined()
    }
}

use oxrdf::{Term, Variable};
use sparq_core::Graph;
use spargebra::{Query, SparqlParser};

/// A cooperative resource budget for one query evaluation (T15 server hardening).
///
/// The executor checks it at coarse sites only (operator entry, once per outer
/// iteration of the big scan/join loops), so enforcement is approximate but cheap:
/// an unlimited budget (the default) costs nothing on the hot paths. When a limit
/// trips, evaluation stops and the query fails with
/// `"query budget exceeded (timeout)"` / `"query budget exceeded (max-rows)"` /
/// `"query budget exceeded (max-bytes)"` / `"query budget exceeded (cancelled)"`.
#[derive(Debug, Clone, Default)]
pub struct QueryBudget {
    /// Wall-clock deadline. Native only: `std::time::Instant` is unusable on
    /// `wasm32-unknown-unknown` (it panics), so the field does not exist there —
    /// the row budget below stays fully portable.
    #[cfg(not(target_arch = "wasm32"))]
    pub deadline: Option<std::time::Instant>,
    /// Upper bound on the rows of any materialised (intermediate or final) result.
    /// This is a *working-set* bound: a query whose intermediate result exceeds it
    /// is refused even if a later operator (e.g. LIMIT) would have shrunk it.
    pub max_rows: Option<usize>,
    /// [OPUS-4.8] (sq-s5is) Upper bound, in BYTES, on the estimated working-set size of
    /// any materialised (intermediate or final) result — the byte-accounted twin of
    /// `max_rows`. Where `max_rows` counts ROWS and so misses a query with FEW but very
    /// WIDE rows (many projected variables, or huge computed string literals), this bounds
    /// the estimated heap footprint of the id-level working set: `rows × width ×
    /// size_of::<Id>()` for each materialised intermediate, PLUS the bytes of any
    /// query-computed terms (BIND / aggregate / CONSTRUCT scratch) interned into the
    /// per-query local vocabulary. Checked cooperatively at the same coarse sites as
    /// `max_rows` (operator entry / per outer-loop iteration); a query whose estimate
    /// crosses it aborts with `"query budget exceeded (max-bytes)"`. The estimate is a
    /// portable LOWER bound on real heap (it ignores allocator overhead and `SmallVec`
    /// inline storage), so it is conservative in the SAME direction `max_rows` is — a blunt
    /// anti-OOM ceiling, not an exact RSS quota. `None` (the default) disables it; it
    /// composes with `max_rows` (whichever trips first aborts).
    pub max_bytes: Option<usize>,
    /// Cross-thread cooperative cancellation flag. The executor observes `true`
    /// at the same coarse polling sites as the other limits and aborts with
    /// `"query budget exceeded (cancelled)"`; cancellation is therefore prompt at
    /// the next poll, not immediate. The flag controls evaluation only and does
    /// not publish shared query data.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl QueryBudget {
    /// The do-nothing budget every non-budgeted entry point uses.
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Creates an otherwise unlimited budget cancelled by `flag`.
    pub fn cancelled_by(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::unlimited().with_cancel(flag)
    }

    /// Adds a cross-thread cooperative cancellation flag to this budget.
    #[must_use]
    pub fn with_cancel(mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }
}

/// An extension function: concrete RDF terms in, one concrete RDF term out.
///
/// Arguments arrive fully materialised (computed numerics/booleans as their typed
/// literals). Returning `Err` is a SPARQL *expression* error — the row is filtered
/// by a `FILTER`, left unbound by a `BIND` — never a hard query error, matching how
/// the builtin functions report bad arguments (wrong arity, unparsable lexicals, …).
/// The message itself is discarded, so it only needs to be useful to a human
/// debugging the extension.
pub type ExtFn = std::sync::Arc<dyn Fn(&[Term]) -> Result<Term, String> + Send + Sync>;

/// A map from function IRIs to [`ExtFn`]s, consulted by the evaluator for
/// `Function::Custom` IRIs that are not XSD constructor casts (SPARQL 17.6,
/// extensible value testing). Installed per query by [`query_with_functions`] /
/// [`with_functions`]; the registry-free entry points never consult it, so they
/// keep their exact pre-registry behaviour (an unknown custom IRI is a hard
/// "unsupported SPARQL function" error) and hot-path cost.
///
/// Cloning is cheap (the functions are `Arc`-shared), so a long-lived registry can
/// be built once and reused across queries and threads.
#[derive(Clone, Default)]
pub struct FunctionRegistry {
    map: std::collections::HashMap<String, ExtFn>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `f` under the function IRI (replacing any previous registration).
    pub fn register(
        &mut self,
        iri: impl Into<String>,
        f: impl Fn(&[Term]) -> Result<Term, String> + Send + Sync + 'static,
    ) {
        self.map.insert(iri.into(), std::sync::Arc::new(f));
    }

    /// The function registered under `iri`, if any.
    pub fn get(&self, iri: &str) -> Option<&ExtFn> {
        self.map.get(iri)
    }

    /// [OPUS-4.8] sq-qfcb: iterates the IRIs of every registered extension function
    /// (unspecified order). Lets a caller advertise EXACTLY the functions actually
    /// installed (e.g. the SPARQL Service Description's `sd:extensionFunction`) without
    /// hand-maintaining a parallel list that could drift from the registry.
    pub fn iris(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Runs `f` with `fns` installed as the active extension-function registry — the
/// scoped form behind [`query_with_functions`], for callers that need one of the
/// OTHER entry points (`ask`, `query_json_chunks_with_budget`, `construct`, …) to
/// see the registry:
///
/// ```ignore
/// let chunks = with_functions(&reg, || query_json_chunks_with_budget(&g, q, &budget))?;
/// ```
///
/// The registry is visible to every query the closure runs on this thread (it is
/// installed thread-locally and propagated into the engine's rayon workers), and
/// uninstalled when the closure returns.
pub fn with_functions<T>(fns: &FunctionRegistry, f: impl FnOnce() -> T) -> T {
    let _guard = exec::functions::install(fns);
    f()
}

// ---- Local rows-returning SERVICE handlers (sq-lsp7k.2.2) --------------------
//
// The ROWS-returning twin of `FunctionRegistry` (which is scalar: terms in, ONE term
// out). A registered IRI is intercepted at `SERVICE <iri> { … }` BEFORE the HTTP
// transport, so a handled SERVICE performs no network I/O and the egress allowlist has
// nothing to police; an unregistered IRI falls through to the unchanged `service` path
// where the default-deny SSRF filter still applies in full. NON-DEFAULT
// `service-local` feature: when off, none of this compiles, the executor's SERVICE
// dispatch is byte-identical to before, and no new dependency enters the graph
// (oxrdf + spargebra are already direct deps). [OPUS-5]
#[cfg(feature = "service-local")]
pub mod service_local;
#[cfg(feature = "service-local")]
pub use service_local::{
    LocalServiceFn, LocalServicePattern, LocalServiceRegistry, LocalServiceRequest,
    LocalServiceRows, LocalServiceSlot,
};

/// Runs `f` with `reg` installed as the active local SERVICE-handler registry: every
/// `SERVICE <iri> { … }` whose IRI is registered is answered IN PROCESS by the handler
/// instead of being forwarded over HTTP. See the [`service_local`] module docs for the
/// egress/SSRF argument and the v1 scope-outs.
///
/// The registry is visible to every query the closure runs on this thread (installed
/// thread-locally and propagated into the engine's rayon workers, so a `SERVICE` nested
/// under a `FILTER EXISTS` still sees it) and uninstalled when the closure returns.
/// Composes with [`with_functions`] / [`with_spatial_index`] in either nesting order.
///
/// Nests with ITSELF: an inner install SHADOWS the outer registry only for the inner
/// closure, and the outer one is restored when that closure returns — including on
/// unwind, so a panicking inner scope never leaves the outer scope's handlers
/// unregistered.
///
/// ```ignore
/// let mut reg = LocalServiceRegistry::new();
/// reg.register("urn:sparq:local:evens", |_req| {
///     Ok(LocalServiceRows::new(vec![Variable::new("n")?], rows))
/// });
/// with_local_services(&reg, || query(&graph, "SELECT * WHERE { SERVICE <urn:sparq:local:evens> { ?n } }"))
/// ```
#[cfg(feature = "service-local")]
pub fn with_local_services<T>(reg: &LocalServiceRegistry, f: impl FnOnce() -> T) -> T {
    let _guard = exec::local_services::install(reg);
    f()
}

// ---- Custom aggregate registry + window functions (sq-5qz9) -----------------
//
// NON-DEFAULT `window-functions` feature. Two distinct, OPT-IN extension surfaces;
// when the feature is off, NOTHING below compiles and the default build is
// byte-identical (these add no dependencies). [OPUS-4.8]
#[cfg(feature = "window-functions")]
mod aggregate;
#[cfg(feature = "window-functions")]
pub mod window;
// Inline `OVER(…)` window-clause SYNTAX (sq-h564) — a source-rewrite front end over
// the programmatic `window` pass, on the dedicated `query_over` entry point only. It
// does NOT touch the (conformance-tracking) vendored spargebra parser. [OPUS-4.8]
#[cfg(feature = "window-functions")]
mod window_syntax;
#[cfg(feature = "window-functions")]
pub use aggregate::{
    query_with_aggregates, query_with_aggregates_and_budget, with_aggregates, AggFn,
    CustomAggregateRegistry,
};
#[cfg(feature = "window-functions")]
pub use window_syntax::{query_over, query_over_with_budget};
// [OPUS-4.8] (sq-a9cn) Materialised-view / query-result cache re-exports (NON-DEFAULT
// `result-cache` feature).
#[cfg(feature = "result-cache")]
pub use cache::{is_cacheable, CacheStats, ResultCache};

// [OPUS-4.8] (sq-hvfe) Vectorized (columnar / vector-at-a-time) execution primitives —
// the first building block of the M4 plan (research/optimization-techniques.md). NON-DEFAULT
// `vectorized` feature: when off, zero columnar code compiles and the default native + wasm
// builds are byte-identical (no new dependencies — sparq-core is already a direct dep).
#[cfg(feature = "vectorized")]
pub mod chunk;
#[cfg(feature = "vectorized")]
pub use chunk::{DataChunk, SelVec, VecCmp};
// [SONNET-4.6] (sq-pntvh.4) M4 Phase 4 columnar reducer kernels (SUM/COUNT/MIN/MAX/AVG
// over inline-integer id slices). NON-DEFAULT `vectorized` feature; zero code compiles
// when off. No new dependencies.
#[cfg(feature = "vectorized")]
pub(crate) mod reduce;
// [SONNET-4.6] (sq-pntvh.5) M4 Phase 5: columnar_eligible dispatcher + morsel constants
// (VEC_MIN_BATCH/VEC_MORSEL) + I5 probe counters {chunks_built, rows_columnar,
// rows_delegated, declines_by_reason}. NON-DEFAULT `vectorized` feature; compiled out
// entirely when off (relaxed atomics + dispatch helpers compile away). The probe counters
// and VecStats snapshot are public but UNSTABLE/test-facing (not semver-stable). No new
// dependencies.
#[cfg(feature = "vectorized")]
pub mod vec_dispatch;
#[cfg(feature = "vectorized")]
pub use vec_dispatch::{reset_stats, stats_snapshot, VecStats};
// [SONNET-4.6] (sq-y5ew5) M4 hybrid tri-mask FILTER kernel for general decoded columns:
// classifies each lane into Confident / Tie / Unknown and delegates Tie+Unknown to the
// scalar predicate. NON-DEFAULT `vectorized` feature; zero code compiles when off. No new
// dependencies. `pub(crate)` only — purely an internal accelerator, no public surface.
#[cfg(feature = "vectorized")]
pub(crate) mod chunk_select;

// [OPUS-4.8] (sq-gr8mb, survey §A3) Exact-bitmap semi-join reducer on dense u32 ids
// (CIDR'26 "Not Yannakakis"; research/optimization-techniques.md §1.1/§2(a)). The binary
// BGP executor builds a membership filter over a join's connecting-variable ids and
// passes it to the next pattern's scan, dropping rows whose join key cannot match before
// they enter the join. INTERNAL (`pub(crate)`) — the executor's only consumer is
// `exec.rs`, so no public surface is added. NON-DEFAULT `semijoin-bitmap` feature: when
// off, zero of this code compiles and the default native + wasm builds are byte-identical
// (no new dependencies — sparq-core + rustc-hash are already direct deps; no `unsafe`).
//
// [OPUS-4.8] (sq-5zf8i, survey §A4) The same module also hosts the PURE join-tree topology
// (`build_join_tree`) of the opt-in `yannakakis` Yannakakis bottom-up full-semijoin prepass
// for acyclic BGPs, which reuses `KeyFilter` for the exact membership test; `yannakakis`
// implies `semijoin-bitmap`, so the module is available whenever the prepass is.
#[cfg(feature = "semijoin-bitmap")]
pub(crate) mod semijoin;

// [OPUS-4.8] (sq-xj29q) Oxigraph-shaped per-solution view over `QueryResult` —
// `QueryResult::solutions()` yields borrowed, zero-copy `QuerySolution` views (per-row
// map from `Variable` to bound `Term`: `get`, `iter`, `Index`) matching Oxigraph's
// `QuerySolution`, WITHOUT abandoning the columnar `{vars, rows}` layout the engine
// uses for perf. NON-DEFAULT `query-solution` feature: when off, zero of this code
// compiles and the default native + wasm builds are byte-identical (no new
// dependencies — oxrdf is already a direct dep; no `unsafe`).
#[cfg(feature = "query-solution")]
pub mod solution;
#[cfg(feature = "query-solution")]
pub use solution::{QuerySolution, QuerySolutionBindings, QuerySolutionIter, VarIndex};

// ---- Spatial pushdown seam (sq-mg9) -----------------------------------------
//
// The engine is GEOMETRY-FREE: the dependency direction is sparq-geo ->
// sparq-engine, so the engine cannot parse WKT or query an R-tree. The seam that
// lets a `geof:` spatial FILTER be pushed into a spatial index is therefore a
// thread-local PROVIDER trait — installed exactly like [`FunctionRegistry`] —
// that sparq-geo implements over its `GeoIndex`. The engine recognises a pushable
// spatial FILTER, asks the provider for the CANDIDATE SUPERSET of geometry-variable
// bindings the index says could match, and pre-restricts rows to those candidates
// BEFORE the exact `geof:` FILTER refinement still runs. [OPUS-4.8]

/// A request the engine's planner makes of an installed [`SpatialProvider`]: a
/// recognised pushable `geof:` spatial FILTER, in fully-resolved terms.
///
/// The engine does no geometry: it forwards the function IRI and the constant
/// operand(s) it lifted out of the FILTER, and the provider decides whether it
/// can serve a candidate set. The geometry *variable*'s bindings are NOT sent —
/// only the query CONSTANT (the `$point` / `$box` literal and, for distance, the
/// threshold + unit) — so the provider answers purely from its index.
#[derive(Debug, Clone)]
pub enum SpatialQuery<'a> {
    /// `geof:distance(?g, point) OP radius` with `OP` ∈ `{<, <=}` — a
    /// distance-WITHIN window. `point_wkt` is the constant geometry's lexical
    /// form (a `geo:wktLiteral` value), `radius`/`unit_iri` the bound.
    /// `inclusive` distinguishes `<=` (true) from `<` (false); the provider may
    /// ignore it and return a (still-correct) superset.
    DistanceWithin { point_wkt: &'a str, radius: f64, unit_iri: &'a str, inclusive: bool },
    /// `geof:sfWithin(?g, box)` / `geof:sfIntersects(?g, box)` — a window/range
    /// scan. `arg_wkt` is the constant geometry's lexical form.
    BboxIntersects { arg_wkt: &'a str },
}

/// A request for an EXACT-certified candidate set — see
/// [`SpatialProvider::candidates_exact`]. Separate from [`SpatialQuery`] on
/// purpose: a `SpatialQuery` names a WINDOW the provider may over-approximate,
/// while a `SpatialExactQuery` names the precise PREDICATE the certification is
/// about, so "exact" is never ambiguous about which relation was certified.
/// [FABLE-5] (sq-lk3aw.4)
#[cfg(feature = "spatial-exact-pushdown")]
#[cfg_attr(docsrs, doc(cfg(feature = "spatial-exact-pushdown")))]
#[derive(Debug, Clone)]
pub enum SpatialExactQuery<'a> {
    /// `geof:sfWithin(?g, region)` — equivalently `geof:sfContains(region, ?g)`
    /// (Simple Features defines `contains(a, b) ⇔ within(b, a)`): the geometry
    /// variable's binding lies within the constant `region`. `region_wkt` is the
    /// constant geometry's lexical form (a `geo:wktLiteral` value).
    WithinRegion { region_wkt: &'a str },
}

/// A spatial index the engine can push a recognised `geof:` FILTER into.
///
/// Implemented by sparq-geo over its `GeoIndex`; installed per-query via
/// [`with_spatial_index`].
///
/// CORRECTNESS CONTRACT (a one-way FILTER that must never drop a true match):
///
/// * [`candidates`](Self::candidates) returns EVERY *indexed* geometry-variable
///   binding that could satisfy the predicate — a SUPERSET of the matches AMONG
///   the geometries the index actually holds (false positives allowed; false
///   negatives among indexed geometries are NOT). The exact `geof:` check still
///   runs afterwards and removes the false positives.
/// * [`is_indexed`](Self::is_indexed) reports whether a `Term` is one the index
///   has an opinion on. The engine keeps a row when its binding is a candidate
///   OR is NOT indexed — so a geometry the index never saw (bound via a
///   non-`geo:asWKT` predicate, a non-geographic CRS, a different graph, …) is
///   left for the exact `geof:` FILTER to judge, NEVER silently dropped.
///
/// Together these make the pushed-down plan return IDENTICALLY to the post-hoc
/// path regardless of which subset of the queried bindings the index covers.
/// Returning `None` from `candidates` declines the pushdown entirely (e.g.
/// unparsable constant, a metric the index cannot bound) and the engine falls
/// back to the full per-row scan.
pub trait SpatialProvider: Send + Sync {
    /// The candidate superset (among indexed geometries) of geometry-variable
    /// bindings — the WKT-literal `Term`s — that could satisfy `query`, or
    /// `None` to decline the pushdown.
    fn candidates(&self, query: &SpatialQuery) -> Option<Vec<Term>>;

    /// Whether `term` is a geometry this index holds an opinion on (i.e. it was
    /// indexed). A binding for which this is `false` is NEVER dropped by the
    /// pushdown — the index cannot rule it out, so the exact `geof:` FILTER
    /// decides. (For a `GeoIndex`: the geographic-CRS `geo:asWKT` literals it
    /// extracted.)
    fn is_indexed(&self, term: &Term) -> bool;

    /// The ID-LEVEL indexed universe: the dictionary ids of the geometries this
    /// index holds an opinion on — the exact id-set of the terms for which
    /// [`is_indexed`](Self::is_indexed) returns `true` — returned ONLY when that
    /// set was resolved against the SAME dict identified by `dict_ptr`
    /// (`std::ptr::from_ref(&graph.dict) as usize`), else `None`.
    ///
    /// This lets the pushdown replace a per-row `Term` materialisation +
    /// `is_indexed` hash with a pure `FxHashSet<Id>` lookup on the scanned
    /// column. It is a PURE OPTIMISATION of the `is_indexed` decision: for a row
    /// whose binding id is `i`, `set.contains(&i)` MUST equal
    /// `is_indexed(&graph.dict.term(i))` — same keep/drop verdict, so the result
    /// is byte-identical to the per-row path. The FRESHNESS contract is
    /// load-bearing: an id-set only maps to the right terms against the dict it
    /// came from, so the provider returns `Some` ONLY when certain the dict
    /// matches and `None` on ANY doubt — whereupon the engine uses the per-row
    /// fallback (always correct). Returned as an `Arc` so the engine holds it
    /// cheaply. The default returns `None` (a provider with no id-level universe
    /// is served entirely by the per-row path). [OPUS-4.8]
    fn indexed_ids(
        &self,
        dict_ptr: usize,
    ) -> Option<std::sync::Arc<rustc_hash::FxHashSet<sparq_core::dict::Id>>> {
        let _ = dict_ptr;
        None
    }

    /// An EXACT-certified candidate set, or `None` for "no exact certification"
    /// (the default — the engine then uses the superset [`candidates`](Self::candidates)
    /// + residual-FILTER path, always correct). [FABLE-5] (sq-lk3aw.4)
    ///
    /// EXACTNESS CONTRACT (STRICTLY stronger than `candidates`' superset contract —
    /// a wrong `Some` here silently returns WRONG query answers):
    ///
    /// * `Some(v)` CERTIFIES that `v` is EXACTLY the set of INDEXED
    ///   geometry-variable bindings (the WKT-literal `Term`s for which
    ///   [`is_indexed`](Self::is_indexed) is `true`) that satisfy `query` — no
    ///   false positives AND no false negatives among the indexed universe. On
    ///   the strength of that certificate the engine MAY skip the residual
    ///   `geof:` FILTER for rows whose binding is in `v`; a false positive would
    ///   ADD a wrong row to the query answer (the superset path's false
    ///   positives are harmless only because the residual FILTER removes them —
    ///   here it will not run).
    /// * "Satisfy" means: the registered `geof:` extension function for the
    ///   certified predicate, applied to that binding and the query constant,
    ///   evaluates to `true`. The provider asserts its exact refinement agrees
    ///   with the function-registry semantics it is deployed with (sparq-geo
    ///   pins that equivalence in its `topology_index` tests).
    /// * The certificate says NOTHING about a binding that is NOT indexed: the
    ///   engine still routes every not-indexed binding through the residual
    ///   `geof:` FILTER (never drops it, never keeps it unjudged), exactly as
    ///   the superset path does.
    /// * Return `None` on ANY doubt (unparsable constant, non-geographic CRS, a
    ///   predicate the index cannot decide exactly, …) — declining is always
    ///   sound; the engine falls back to `candidates`.
    #[cfg(feature = "spatial-exact-pushdown")]
    #[cfg_attr(docsrs, doc(cfg(feature = "spatial-exact-pushdown")))]
    fn candidates_exact(&self, query: &SpatialExactQuery) -> Option<Vec<Term>> {
        let _ = query;
        None
    }
}

/// Runs `f` with `idx` installed as the active spatial index — the planner
/// consults it to push a recognised `geof:` spatial FILTER into a candidate
/// window/range scan. Visible to every query the closure runs on this thread
/// (installed thread-locally, propagated into the engine's rayon workers) and
/// uninstalled when the closure returns. Composes with [`with_functions`] (you
/// want BOTH: the registry does the exact refinement, the index the pushdown) in
/// either nesting order. [OPUS-4.8]
pub fn with_spatial_index<T>(idx: std::sync::Arc<dyn SpatialProvider>, f: impl FnOnce() -> T) -> T {
    let _guard = exec::spatial::install(idx);
    f()
}

/// [`query`] with an extension-function registry: `Function::Custom` IRIs that are
/// not XSD constructor casts are dispatched to `fns` (SPARQL 17.6). An IRI absent
/// from the registry remains the same hard error the registry-free entry points
/// raise.
pub fn query_with_functions(graph: &Graph, sparql: &str, fns: &FunctionRegistry) -> Result<QueryResult, String> {
    query_with_functions_and_budget(graph, sparql, fns, &QueryBudget::unlimited())
}

/// [`query_with_functions`] under a cooperative [`QueryBudget`].
pub fn query_with_functions_and_budget(
    graph: &Graph,
    sparql: &str,
    fns: &FunctionRegistry,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    with_functions(fns, || query_with_budget(graph, sparql, budget))
}

/// A zero-copy restriction of a [`Graph`]'s dataset for one query execution
/// (the L1 "dataset view"): only the named graphs in `named` are visible, and
/// the default graph is the store's own or empty per [`DefaultGraphMode`].
///
/// The security property is INDISTINGUISHABILITY: a non-visible named graph
/// behaves exactly like an absent one — `GRAPH <g> { … }` yields zero
/// solutions, `GRAPH ?g` never enumerates it, and a dataset clause
/// (`FROM` / `FROM NAMED`) first intersects with the view, so a query can only
/// ever restrict further, never widen. Evaluation runs in place on the
/// existing sub-`Graph`s — zero decode, zero rebuild, zero copy (contrast the
/// `FROM NAMED` path, which rebuilds an active dataset per query).
///
/// The engine holds no session state: `named` is shared from the caller's
/// cache (an `Arc` clone per call, never a set copy) and visibility is one
/// O(1) hash lookup per graph name.
pub struct DatasetView<'g> {
    /// The full store the view restricts.
    pub base: &'g Graph,
    /// The visible named-graph names.
    pub named: std::sync::Arc<FxHashSet<Term>>,
    /// What the view exposes as the default graph.
    pub default: DefaultGraphMode,
}

/// The default graph a [`DatasetView`] exposes. Phase 1 ships these two modes;
/// a union-of-visible-graphs default is phase 2 (it cannot be zero-copy while
/// each named sub-`Graph` owns a private dictionary) and the variant will be
/// added when it is implemented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DefaultGraphMode {
    /// The store's own default graph (today's behaviour).
    #[default]
    StoreDefault,
    /// An empty default graph (e.g. data lives only in named graphs).
    Empty,
}

/// The hash set type [`DatasetView::named`] uses, re-exported so callers build
/// the exact type (`rustc-hash`'s `FxHashSet`).
pub use rustc_hash::FxHashSet;

/// Runs `f` with `v` installed as the active dataset view — the scoped form
/// behind [`query_view`], for callers that need one of the OTHER entry points
/// (`construct`, `query_json_chunks_with_budget`, …) to see the view:
///
/// ```ignore
/// let chunks = with_view(&v, || query_json_chunks_with_budget(v.base, q, &budget))?;
/// ```
///
/// The view is visible to every query the closure runs on this thread (it is
/// installed thread-locally and propagated into the engine's rayon workers),
/// and uninstalled when the closure returns. Composes with [`with_functions`]
/// in either nesting order.
pub fn with_view<T>(v: &DatasetView, f: impl FnOnce() -> T) -> T {
    let _guard = exec::view::install(v);
    f()
}

/// [`query`] restricted to a [`DatasetView`]: `GRAPH` only sees the view's
/// named graphs and the default graph follows the view's mode.
pub fn query_view(v: &DatasetView, sparql: &str) -> Result<QueryResult, String> {
    query_view_with_budget(v, sparql, &QueryBudget::unlimited())
}

/// [`query_view`] under a cooperative [`QueryBudget`].
pub fn query_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    with_view(v, || query_with_budget(v.base, sparql, budget))
}

/// [`query_json`] restricted to a [`DatasetView`].
pub fn query_json_view(v: &DatasetView, sparql: &str) -> Result<String, String> {
    query_json_view_with_budget(v, sparql, &QueryBudget::unlimited())
}

/// [`query_json_view`] under a cooperative [`QueryBudget`].
pub fn query_json_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    with_view(v, || query_json_with_budget(v.base, sparql, budget))
}

/// [`count`] restricted to a [`DatasetView`].
pub fn count_view(v: &DatasetView, sparql: &str) -> Result<usize, String> {
    count_view_with_budget(v, sparql, &QueryBudget::unlimited())
}

/// [`count_view`] under a cooperative [`QueryBudget`].
pub fn count_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<usize, String> {
    with_view(v, || count_with_budget(v.base, sparql, budget))
}

/// [`ask`] restricted to a [`DatasetView`].
pub fn ask_view(v: &DatasetView, sparql: &str) -> Result<bool, String> {
    ask_view_with_budget(v, sparql, &QueryBudget::unlimited())
}

/// [`ask_view`] under a cooperative [`QueryBudget`].
pub fn ask_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<bool, String> {
    with_view(v, || ask_with_budget(v.base, sparql, budget))
}

/// When the query carries a dataset clause (FROM / FROM NAMED), the ACTIVE
/// dataset it describes — built from the store's named graphs; see
/// [`dataset::build_active`]. `None` (the common case) means: evaluate against
/// the store itself. Every query entry point calls this once after parsing, so
/// the no-clause path costs exactly one `Option` check.
pub(crate) fn active_dataset(graph: &Graph, q: &Query) -> Option<Graph> {
    q.dataset().map(|ds| dataset::build_active(graph, ds))
}

/// Suspends an installed [`DatasetView`] while a dataset-clause query evaluates:
/// [`dataset::build_active`] has already INTERSECTED the clause with the view
/// (non-visible ≡ absent), so re-filtering during evaluation would make a
/// non-visible `FROM NAMED` graph distinguishable from an absent one (both must
/// be the empty active graph, with its unit-row `GRAPH <g> {}` semantics). A
/// no-op when no view is installed or the query has no dataset clause.
pub(crate) fn view_scope(active: &Option<Graph>) -> Option<exec::view::Guard> {
    active.is_some().then(exec::view::suspend_all)
}

/// A SPARQL query parsed ONCE for repeated execution — the parse/plan-once seam
/// used by sparq-rsp (continuous queries execute the same query per window)
/// and any caller that runs one query against many graphs. Wraps the `spargebra`
/// algebra; build with [`PreparedQuery::parse`] (or `From<spargebra::Query>` for
/// algebra constructed/rewritten programmatically), execute with
/// [`query_prepared`] / [`ask_prepared`] / [`count_prepared`] /
/// [`construct_prepared`] / [`describe_prepared`] (and their `_with_budget`
/// forms). The string entry points ([`query`], [`ask`], …) are thin wrappers:
/// parse + execute-prepared, so both paths evaluate identically.
#[derive(Debug, Clone)]
pub struct PreparedQuery {
    query: Query,
}

impl PreparedQuery {
    /// Parses a SPARQL query string into its reusable algebra form.
    ///
    /// With the opt-in `algebra-rewrite` feature ON, the parsed algebra is run
    /// through the result-equivalent pre-execution rewrite pass (`rewrite`
    /// module) here — the single seam every string query entry point
    /// ([`query`], [`ask`], [`count`], the JSON paths) funnels through — so
    /// production benefits without touching the executor. The `From<Query>`
    /// conversion deliberately does NOT rewrite: it takes an already-built
    /// algebra verbatim (the opt-out / test-baseline path). When the feature is
    /// OFF the algebra is stored verbatim and the build is byte-identical.
    pub fn parse(sparql: &str) -> Result<PreparedQuery, String> {
        // Feature-OFF arm is the VERBATIM pre-`algebra-rewrite` expression so the
        // default build's codegen is byte-identical (the `feature_off_exact` wasm
        // gate). Only the feature-ON arm introduces the rewrite call. [OPUS-4.8]
        #[cfg(not(feature = "algebra-rewrite"))]
        {
            Ok(PreparedQuery { query: SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())? })
        }
        #[cfg(feature = "algebra-rewrite")]
        {
            let query = rewrite::rewrite_query(SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?);
            Ok(PreparedQuery { query })
        }
    }

    /// The wrapped `spargebra` algebra (e.g. to inspect the query form or dataset
    /// clause before execution).
    pub fn query(&self) -> &Query {
        &self.query
    }

    /// True for the graph-valued query forms (CONSTRUCT / DESCRIBE), which produce a set
    /// of triples rather than a solution sequence. SELECT/ASK go through [`query`]/[`count`];
    /// the graph forms go through [`construct_or_describe`]. Lets callers (e.g. the CLI bench
    /// suite) route a query to the right executor without re-parsing or string-matching.
    // [OPUS-4.8]
    pub fn is_graph_form(&self) -> bool {
        matches!(self.query, Query::Construct { .. } | Query::Describe { .. })
    }

    /// Unwraps into the `spargebra` algebra.
    pub fn into_query(self) -> Query {
        self.query
    }

    /// Safely binds a typed RDF value to a free placeholder variable, returning a
    /// NEW prepared query — the canonical mitigation for SPARQL injection (#901).
    ///
    /// `name` is the placeholder variable (with or without a leading `?`/`$`).
    /// Every free occurrence is replaced *in the parsed algebra* by `value`
    /// ([`oxrdf::Term`]) — a pure structural rewrite, **never** string
    /// concatenation — so a hostile value (an IRI/literal containing
    /// `> } INSERT … {`, a `"` break-out, etc.) is carried as opaque DATA and can
    /// NEVER alter the query structure. The template is parsed once and reusable:
    /// bind different values for different requests, plan-cache stays effective.
    ///
    /// **Fail-closed.** Returns [`params::BindError`] (as a `String`) if the
    /// placeholder does not occur free (typo'd name), if it is a projected /
    /// aggregate / `BIND` / `VALUES` *result* variable (binding it would change the
    /// result shape — rename the placeholder), or if a blank node is bound into a
    /// predicate / graph-name slot.
    ///
    /// Only available with the opt-in `params` feature. See the crate
    /// [`params`] module for the safety argument and convenience value
    /// constructors ([`params::value`]).
    #[cfg(feature = "params")]
    pub fn bind(&self, name: &str, value: oxrdf::Term) -> Result<PreparedQuery, String> {
        params::bind_query(&self.query, name, value).map_err(|e| e.to_string())
    }
}

impl From<Query> for PreparedQuery {
    /// Wraps already-parsed (or programmatically built / rewritten) algebra.
    fn from(query: Query) -> PreparedQuery {
        PreparedQuery { query }
    }
}

impl std::str::FromStr for PreparedQuery {
    type Err = String;

    fn from_str(s: &str) -> Result<PreparedQuery, String> {
        PreparedQuery::parse(s)
    }
}

/// A SPARQL 1.1 UPDATE parsed ONCE, with safe value binding ([`PreparedUpdate::bind`])
/// to prevent SPARQL injection on the write path (#901, bead `sq-rp3um`). The
/// UPDATE counterpart of [`PreparedQuery`]: a `DELETE`/`INSERT … WHERE` template is
/// parsed into algebra, a free placeholder variable is `bind`-substituted by a typed
/// [`oxrdf::Term`] via a pure algebra rewrite (NEVER string concatenation), and the
/// bound algebra is applied DIRECTLY by [`update_prepared`] / [`update_in_place_prepared`]
/// — so a hostile bound value never re-enters the parser. See the [`params`] module.
///
/// Only available with the opt-in `params` feature.
#[cfg(feature = "params")]
#[derive(Debug, Clone)]
pub struct PreparedUpdate {
    update: spargebra::Update,
}

#[cfg(feature = "params")]
impl PreparedUpdate {
    /// Parses a SPARQL UPDATE string into its reusable algebra form.
    pub fn parse(sparql: &str) -> Result<PreparedUpdate, String> {
        Ok(PreparedUpdate {
            update: SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?,
        })
    }

    /// Wraps already-parsed (or programmatically rewritten) UPDATE algebra.
    pub(crate) fn from_algebra(update: spargebra::Update) -> PreparedUpdate {
        PreparedUpdate { update }
    }

    /// The wrapped `spargebra` UPDATE algebra.
    pub fn update(&self) -> &spargebra::Update {
        &self.update
    }

    /// Safely binds a typed RDF value to a free placeholder variable in the UPDATE
    /// template, returning a NEW prepared update. Same fail-closed semantics and
    /// injection-safety guarantee as [`PreparedQuery::bind`]: the placeholder is
    /// substituted in the parsed algebra (`DELETE`/`INSERT` templates + the WHERE
    /// pattern), never by string concatenation, so a hostile value such as an IRI
    /// containing `> } ; DROP ALL ; INSERT { … }` is carried as opaque DATA.
    ///
    /// Returns an `Err` if the placeholder is not free, is a `BIND`/`GROUP`/aggregate
    /// output of the WHERE clause, or a blank node is bound into a predicate /
    /// graph-name / ground-`DELETE` slot.
    pub fn bind(&self, name: &str, value: oxrdf::Term) -> Result<PreparedUpdate, String> {
        params::bind_update(&self.update, name, value).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "params")]
impl std::str::FromStr for PreparedUpdate {
    type Err = String;

    fn from_str(s: &str) -> Result<PreparedUpdate, String> {
        PreparedUpdate::parse(s)
    }
}

/// [OPUS-4.8] (sq-rp3um) Apply a parameterized [`PreparedUpdate`] to `graph`, returning
/// the updated graph (the [`update`] rebuild path over the bound algebra). Only with the
/// opt-in `params` feature.
#[cfg(feature = "params")]
pub fn update_prepared(graph: &Graph, prepared: &PreparedUpdate) -> Result<Graph, String> {
    update::update_prepared_impl(graph, &prepared.update)
}

/// [OPUS-4.8] (sq-rp3um) Apply a parameterized [`PreparedUpdate`] IN PLACE through the
/// delta overlay (the [`update_in_place`] path over the bound algebra). Only with the
/// opt-in `params` feature.
#[cfg(feature = "params")]
pub fn update_in_place_prepared(graph: &mut Graph, prepared: &PreparedUpdate) -> Result<(), String> {
    update_in_place_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [OPUS-4.8] (sq-rp3um) [`update_in_place_prepared`] under a cooperative [`QueryBudget`].
#[cfg(feature = "params")]
pub fn update_in_place_prepared_with_budget(
    graph: &mut Graph,
    prepared: &PreparedUpdate,
    budget: &QueryBudget,
) -> Result<(), String> {
    update::update_in_place_prepared_with_budget(graph, &prepared.update, budget)
}

/// Executes a SPARQL query string against a graph, materialising the solutions.
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    query_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`query`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`query`] over a [`PreparedQuery`] — no per-execution parse.
pub fn query_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`query_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
        // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
        // is satisfiable — the standard "unit row" encoding of a boolean result.
        Query::Ask { pattern, .. } => Ok(QueryResult {
            vars: Vec::new(),
            rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
        }),
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}

/// Executes an ASK query: `true` iff the pattern has at least one solution.
/// Evaluation early-exits where the engine has a streaming path (the pattern is
/// evaluated under a `LIMIT 1`).
pub fn ask(graph: &Graph, sparql: &str) -> Result<bool, String> {
    ask_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`ask`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn ask_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<bool, String> {
    ask_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`ask`] over a [`PreparedQuery`] — no per-execution parse.
pub fn ask_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<bool, String> {
    ask_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`ask_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn ask_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<bool, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Ask { pattern, .. } => exec::eval_ask(graph, pattern),
        _ => Err("ask() requires an ASK query".into()),
    }
}

/// Executes a SELECT and serialises it directly to a SPARQL 1.1 JSON results string,
/// skipping the intermediate `QueryResult` and its per-cell `oxrdf::Term` allocation
/// (the dictionary case is formatted straight from the stored prefix/suffix). This is
/// the fast path for the actual end-use — returning results to the CLI / browser.
pub fn query_json(graph: &Graph, sparql: &str) -> Result<String, String> {
    query_json_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`query_json`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_json_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    query_json_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`query_json`] over a [`PreparedQuery`] — no per-execution parse.
pub fn query_json_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<String, String> {
    query_json_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`query_json_prepared`] under a cooperative [`QueryBudget`].
pub fn query_json_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<String, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => exec::eval_select_json(graph, pattern),
        // The SPARQL 1.1 JSON results boolean form.
        Query::Ask { pattern, .. } => Ok(format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)),
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}

/// Flush threshold for [`query_json_chunks_with_budget`]: large enough that the
/// per-chunk overhead (stream item, HTTP write) is negligible, small enough that a
/// streamed body never holds a second whole-result copy in memory.
const JSON_CHUNK_BYTES: usize = 64 * 1024;

/// [`query_json_with_budget`] as an ordered sequence of chunks whose concatenation is
/// **byte-identical** to the single-string result — the server streams these as one
/// HTTP body instead of concatenating a giant `String` (T16), which removes the
/// second whole-result copy from peak memory on large SELECTs.
pub fn query_json_chunks_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<String>, String> {
    let prepared = PreparedQuery::parse(sparql)?;
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => exec::eval_select_json_chunks(graph, pattern, Some(JSON_CHUNK_BYTES)),
        Query::Ask { pattern, .. } => {
            Ok(vec![format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)])
        }
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}

/// Streams the SPARQL-JSON serialisation of a SELECT (or ASK) result, invoking `sink` for
/// each serialised chunk **as it is produced** rather than materialising the whole
/// `Vec<String>` first ([`query_json_chunks_with_budget`]).
///
/// [OPUS-4.8] (sq-7d3dj.34.2) This is the TTFB-streaming entry point: the server sinks each
/// chunk straight onto the HTTP socket, so the results header + early solutions are written
/// before the whole result is serialised — and, for the single-pattern scan fast path,
/// before the scan even finishes. The concatenation of the chunks handed to `sink` is
/// **byte-identical** to [`query_json_with_budget`] for the same query and budget.
///
/// `sink` returns [`std::ops::ControlFlow::Break`] to stop early (the consumer went away —
/// e.g. the HTTP client disconnected); the engine then abandons the remaining work. A
/// cooperative budget (row / byte cap or deadline) that trips is returned as `Err` exactly
/// as on the buffered path, but note that on this streaming path some chunks may already
/// have been handed to `sink` (and flushed to the socket) when the trip is detected — a
/// post-first-byte trip cannot change the already-sent HTTP status, so the caller truncates
/// the body (see the server's `stream_select_json`).
pub fn query_json_stream_with_budget(
    graph: &Graph,
    sparql: &str,
    budget: &QueryBudget,
    sink: impl FnMut(String) -> std::ops::ControlFlow<()>,
) -> Result<(), String> {
    query_json_stream_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget, sink)
}

/// [`query_json_stream_with_budget`] over a [`PreparedQuery`] — no per-execution parse.
///
/// [OPUS-4.8] (sq-7d3dj.34.1) The HTTP floor path: `sparq-server` parses the request query
/// ONCE (to classify its form + apply any protocol dataset override) and hands the resulting
/// algebra straight here, so the streamed SELECT-JSON body is produced without the engine
/// re-parsing the query string — the per-request parse is paid exactly once, not twice. The
/// concatenation of the chunks handed to `sink` is byte-identical to
/// [`query_json_stream_with_budget`] for the same query and budget.
pub fn query_json_stream_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
    mut sink: impl FnMut(String) -> std::ops::ControlFlow<()>,
) -> Result<(), String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => {
            exec::eval_select_json_emit(graph, pattern, Some(JSON_CHUNK_BYTES), &mut sink)
        }
        Query::Ask { pattern, .. } => {
            let doc = format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?);
            let _ = sink(doc);
            Ok(())
        }
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}

/// Counts the solutions of a SELECT query *without* materialising the result
/// terms (the id-level row count equals the solution count). Used to measure
/// engine compute in isolation from result serialisation.
pub fn count(graph: &Graph, sparql: &str) -> Result<usize, String> {
    count_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`count`] under a cooperative [`QueryBudget`] (the server's budgeted ASK path).
pub fn count_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<usize, String> {
    count_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`count`] over a [`PreparedQuery`] — no per-execution parse.
pub fn count_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<usize, String> {
    count_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`count_prepared`] under a cooperative [`QueryBudget`].
pub fn count_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<usize, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => exec::count_select(graph, pattern),
        // An ASK counts its unit row: 1 when satisfiable, 0 otherwise.
        Query::Ask { pattern, .. } => Ok(usize::from(exec::eval_ask(graph, pattern)?)),
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}

#[derive(Debug)]
pub struct QueryResult {
    pub vars: Vec<Variable>,
    /// Each row has one entry per `vars` position; `None` is unbound.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl QueryResult {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod temporal_cache_tests {
    //! Semantics of the temporal (dateTime/date) fast paths — the load-time
    //! `temporals` cache feeding FILTER pushdown, the general comparison
    //! operators, ORDER BY sort keys and the MIN/MAX id-level fold. Each test
    //! pins behaviour the cache must NOT change relative to the per-row
    //! dict-parse path: XPath timezone semantics (equal instants across
    //! offsets; the ±14h mixed-presence indeterminate window — a type error for
    //! the RELATIONAL operators, positioned by the sq-2k5py total-order
    //! extension for the SORT), sub-second precision, dateTime/date family
    //! disjointness, and ORDER BY / MIN/MAX tie handling (first/last-of-equals).
    //! The MIN/MAX results are LOCKED against the pre-cache (per-row dict-parse)
    //! engine, verified by running both; the mixed-tz ORDER BY permutation was
    //! too, until sq-2k5py deliberately replaced its lexical fallback.
    use super::*;

    const TDATA: &str = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:utc      ex:at "2024-03-15T13:00:00Z"^^xsd:dateTime .
        ex:plus1    ex:at "2024-03-15T14:00:00+01:00"^^xsd:dateTime .
        ex:minus5   ex:at "2024-03-15T08:00:00-05:00"^^xsd:dateTime .
        ex:floating ex:at "2024-03-15T13:00:00"^^xsd:dateTime .
        ex:later    ex:at "2024-03-16T09:00:00Z"^^xsd:dateTime .
        ex:subsec   ex:at "2024-03-15T13:00:00.250Z"^^xsd:dateTime .
        ex:farpast  ex:at "1999-01-01T00:00:00"^^xsd:dateTime .
        ex:aday     ex:on "2024-03-15"^^xsd:date .
    "#;

    fn tg() -> Graph {
        Graph::load_str(TDATA, "turtle").unwrap()
    }

    fn names(g: &Graph, q: &str) -> Vec<String> {
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        query(g, &format!("{pfx}{q}"))
            .unwrap()
            .rows
            .iter()
            .map(|r| match r[0].as_ref().unwrap() {
                Term::NamedNode(n) => n.as_str().strip_prefix("http://ex/").unwrap().to_string(),
                other => other.to_string(),
            })
            .collect()
    }

    /// FILTER `=` across timezones: equal instants are equal regardless of offset,
    /// through both the pushed-down scan predicate and the general path.
    #[test]
    fn equality_across_timezones() {
        let g = tg();
        // Pushed-down (single pattern + sargable temporal filter).
        let mut r = names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d = "2024-03-15T08:00:00-05:00"^^xsd:dateTime) }"#);
        r.sort();
        assert_eq!(r, ["minus5", "plus1", "utc"], "equal instants across offsets must all match");
        // General path (the residual-filter shape: arithmetic-free but two patterns).
        let mut r2 = names(
            &g,
            r#"SELECT ?s WHERE { ?s ex:at ?d . ?s ex:at ?e . FILTER(?d = "2024-03-15T13:00:00Z"^^xsd:dateTime && ?e = ?d) }"#,
        );
        r2.sort();
        assert_eq!(r2, ["minus5", "plus1", "utc"]);
    }

    /// Mixed timezone presence: inside the ±14h window the comparison is
    /// INDETERMINATE — a type error, so FILTER excludes the row; outside the
    /// window it decides.
    #[test]
    fn mixed_presence_window() {
        let g = tg();
        // ?d > 13:00Z: plus1/minus5 equal -> excluded; later/subsec greater -> included;
        // floating is INSIDE the window -> indeterminate -> excluded; farpast outside
        // the window (decidably less) -> excluded.
        let mut r = names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d > "2024-03-15T13:00:00Z"^^xsd:dateTime) }"#);
        r.sort();
        assert_eq!(r, ["later", "subsec"]);
        // ?d < 13:00Z: only farpast is DECIDABLY less (floating is indeterminate).
        let r = names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d < "2024-03-15T13:00:00Z"^^xsd:dateTime) }"#);
        assert_eq!(r, ["farpast"]);
        // A floating constant: zoned values inside the window are indeterminate; only
        // the exact floating term itself passes `=`.
        let r = names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d = "2024-03-15T13:00:00"^^xsd:dateTime) }"#);
        assert_eq!(r, ["floating"]);
    }

    /// Sub-second precision must survive the cache (the f64 instant is bit-identical
    /// to the parse path).
    #[test]
    fn subsecond_precision() {
        let g = tg();
        let r = names(
            &g,
            r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d > "2024-03-15T13:00:00Z"^^xsd:dateTime && ?d < "2024-03-15T13:00:01Z"^^xsd:dateTime) }"#,
        );
        assert_eq!(r, ["subsec"]);
    }

    /// dateTime and date are DISJOINT families: `=` is known-false (not an error),
    /// ordering is a type error — both exclude in FILTER, including the pushdown.
    #[test]
    fn date_datetime_disjoint() {
        let g = tg();
        assert!(names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d = "2024-03-15"^^xsd:date) }"#).is_empty());
        assert!(names(&g, r#"SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d >= "2024-03-15"^^xsd:date) }"#).is_empty());
        assert_eq!(names(&g, r#"SELECT ?s WHERE { ?s ex:on ?d . FILTER(?d = "2024-03-15"^^xsd:date) }"#), ["aday"]);
        // BIND makes the known-false vs error distinction observable: != of disjoint
        // families is TRUE (not unbound/error).
        let r = query(
            &tg(),
            r#"PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
               SELECT ?x WHERE { ex:utc ex:at ?d . BIND((?d != "2024-03-15"^^xsd:date) AS ?x) }"#,
        )
        .unwrap();
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>");
    }

    /// ORDER BY over temporals: every pair orders by INSTANT (across offsets), with
    /// timezone PRESENCE breaking an equal-instant tie — `Temporal::cmp_t_total`, the
    /// total-order extension over XPath's indeterminate mixed-presence window
    /// (sq-2k5py). `floating` is the same instant as utc/plus1/minus5 and tz-less, so it
    /// sorts immediately BEFORE that equal-instant class; the three zoned members are
    /// mutually Equal and keep scan (dictionary) order under the stable sort. Before
    /// sq-2k5py the indeterminate pairs fell back to LEXICAL order, which put `floating`
    /// lexically between them and made the comparator intransitive.
    #[test]
    fn order_by_mixed_timezones_matches_lenient_order() {
        let g = tg();
        let got = names(&g, "SELECT ?s WHERE { ?s ex:at ?d } ORDER BY ?d");
        let want = ["farpast", "floating", "utc", "plus1", "minus5", "subsec", "later"];
        assert_eq!(got, want, "ORDER BY dateTime must match the total temporal order");
    }

    /// The GENERAL comparison path (`compare_values` → `CompareTerm::strict_cmp` →
    /// `temporal_total_cmp`) must order the indeterminate window exactly as the
    /// temporals-cache sort-cell path (`Temporal::cmp_t_total`) does — the two comparators
    /// sq-2k5py had to fix in LOCK-STEP. These `VALUES` lexicals are absent from the graph,
    /// so they are local-vocab terms with no cache cell and take the general path.
    ///
    /// The three are the sq-2k5py WITNESS: `12:00:00-01:00` and `14:00:00+01:00` are the
    /// SAME instant (timeline-Equal) while the tz-less `13:00:00` is XPath-indeterminate
    /// against both and sits lexically BETWEEN them. Under the old lexical fallback the
    /// comparator was inconsistent and produced the lexical permutation; the total order
    /// puts the tz-less value first and keeps the equal-instant pair in input order.
    #[test]
    fn order_by_general_path_orders_the_indeterminate_window_totally() {
        let g = tg();
        let got = names(
            &g,
            r#"SELECT ?v WHERE { VALUES ?v {
                 "2024-03-15T12:00:00-01:00"^^xsd:dateTime
                 "2024-03-15T13:00:00.000"^^xsd:dateTime
                 "2024-03-15T14:00:00+01:00"^^xsd:dateTime } } ORDER BY ?v"#,
        );
        let dt = |lex: &str| format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>", lex);
        // tz-less FIRST (same instant, floating < zoned), then the equal-instant zoned
        // pair in input order. The lexical fallback would give 12:00 / 13:00 / 14:00.
        let want = [dt("2024-03-15T13:00:00.000"), dt("2024-03-15T12:00:00-01:00"), dt("2024-03-15T14:00:00+01:00")];
        assert_eq!(got, want, "the general path must order the indeterminate window by instant, then presence");
    }

    /// MIN/MAX over temporals through the id-level fold: value semantics across
    /// timezones must match the general path; mixed groups fall back.
    #[test]
    fn minmax_temporal_semantics() {
        let g = tg();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        let one = |q: &str| -> String {
            let r = query(&g, &format!("{pfx}{q}")).unwrap();
            r.rows[0][0].as_ref().unwrap().to_string()
        };
        assert!(one("SELECT (MIN(?d) AS ?m) WHERE { ?s ex:at ?d }").contains("1999-01-01T00:00:00"));
        assert!(one("SELECT (MAX(?d) AS ?m) WHERE { ?s ex:at ?d }").contains("2024-03-16T09:00:00Z"));
        assert!(one("SELECT (MIN(DISTINCT ?d) AS ?m) WHERE { ?s ex:at ?d }").contains("1999-01-01T00:00:00"));
        // Mixed temporal/non-temporal group falls back to the general path (no wrong
        // fast-path answer).
        let r = query(
            &g,
            &format!("{pfx}SELECT (MAX(?v) AS ?m) WHERE {{ {{ ?s ex:at ?v }} UNION {{ BIND(5 AS ?v) }} }}"),
        )
        .unwrap();
        assert!(r.rows[0][0].is_some(), "mixed group MAX must still produce a value");
    }

    /// MIN/MAX over the INDETERMINATE mixed-timezone window: the id-level fold
    /// (`minmax_temporal`) and the general (`minmax_values` → `compare_values`) path must
    /// agree, and both must use the total order rather than a lexical fallback — the third
    /// lock-step site of sq-2k5py.
    ///
    /// All three values are the SAME instant, with the tz-less one lexically BETWEEN the two
    /// zoned ones. The total order puts the tz-less value strictly first, so MIN is `ex:b`;
    /// a lexical fallback would compare `"12:00:00-01:00" < "13:00:00"` and answer `ex:a`.
    #[test]
    fn minmax_over_the_indeterminate_window_is_total_on_both_paths() {
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:at "2024-03-15T12:00:00-01:00"^^xsd:dateTime .
            ex:b ex:at "2024-03-15T13:00:00"^^xsd:dateTime .
            ex:c ex:at "2024-03-15T14:00:00+01:00"^^xsd:dateTime .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        let one = |q: &str| -> String {
            let r = query(&g, &format!("{pfx}{q}")).unwrap();
            r.rows[0][0].as_ref().unwrap().to_string()
        };
        // The id-level fold (every member is a cached graph temporal).
        let fast = one("SELECT (MIN(?d) AS ?m) WHERE { ?s ex:at ?d }");
        assert!(fast.contains("\"2024-03-15T13:00:00\""), "MIN must be the tz-less value, got {fast}");
        // The GENERAL path: the local-vocab BIND member aborts the fast fold, and its own
        // value is later, so the answer must be identical.
        let general = one(
            r#"SELECT (MIN(?d) AS ?m) WHERE
               { { ?s ex:at ?d } UNION { BIND("2024-03-15T20:00:00Z"^^xsd:dateTime AS ?d) } }"#,
        );
        assert_eq!(general, fast, "the general MIN path must agree with the id-level fold");
    }

    /// MAX tie semantics: equal instants in different offsets are equal-comparing
    /// DISTINCT terms — `Iterator::max_by` keeps the LAST of equals, `min_by` the
    /// FIRST, and the id-level fold must reproduce that.
    #[test]
    fn max_keeps_last_of_equals_min_keeps_first() {
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:at "2024-03-15T13:00:00Z"^^xsd:dateTime .
            ex:b ex:at "2024-03-15T14:00:00+01:00"^^xsd:dateTime .
            ex:c ex:at "2024-03-15T08:00:00-05:00"^^xsd:dateTime .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> ";
        let one = |q: &str| -> String {
            let r = query(&g, &format!("{pfx}{q}")).unwrap();
            r.rows[0][0].as_ref().unwrap().to_string()
        };
        // Scan order of `?s ex:at ?d` is subject order a, b, c (SPO index).
        let min = one("SELECT (MIN(?d) AS ?m) WHERE { ?s ex:at ?d }");
        let max = one("SELECT (MAX(?d) AS ?m) WHERE { ?s ex:at ?d }");
        assert!(min.contains("13:00:00Z"), "MIN must keep the first of equal members, got {min}");
        assert!(max.contains("08:00:00-05:00"), "MAX must keep the last of equal members, got {max}");
    }

    /// xsd:time must stay on the lexical (OtherXsd) comparison path — the cache must
    /// not pull it onto the timeline.
    #[test]
    fn time_stays_lexical() {
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:t "09:00:00"^^xsd:time . ex:b ex:t "13:00:00"^^xsd:time .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let r = query(
            &g,
            r#"PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
               SELECT ?s WHERE { ?s ex:t ?v . FILTER(?v < "12:00:00"^^xsd:time) }"#,
        )
        .unwrap();
        assert_eq!(r.rows.len(), 1); // lexical "09:..." < "12:..." — same answer as before
    }

    /// Computed (BIND) integers in the inline range resolve to inline ids without a
    /// Term — they must still JOIN against stored data and serialise canonically.
    #[test]
    fn bind_inline_int_fast_path_joins() {
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:v "41"^^xsd:integer . ex:b ex:v "42"^^xsd:integer .
        "#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        // BIND(?v + 1) = 42 joins back to ex:b's stored value via the inline id.
        let r = query(&g, &format!("{pfx}SELECT ?s ?o WHERE {{ ex:a ex:v ?v . BIND(?v + 1 AS ?w) . ?s ex:v ?w . BIND(STR(?w) AS ?o) }}")).unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "<http://ex/b>");
        assert_eq!(r.rows[0][1].as_ref().unwrap().to_string(), "\"42\"");
        // Negative / out-of-inline-range integers and decimals still resolve correctly
        // (dictionary probe or local vocab) — DISTINCT dedupes equal computed values.
        let r = query(&g, &format!("{pfx}SELECT DISTINCT ?w WHERE {{ ?s ex:v ?v . BIND(?v - 100 AS ?w) }}")).unwrap();
        assert_eq!(r.rows.len(), 2);
        let r = query(&g, &format!("{pfx}SELECT DISTINCT ?w WHERE {{ ?s ex:v ?v . BIND(?v + 0.5 AS ?w) }}")).unwrap();
        assert_eq!(r.rows.len(), 2);
    }

    /// dateTime terms appended through the delta overlay must enter the temporal
    /// cache (extend_for), so filters see them without a rebuild.
    #[test]
    fn delta_appended_datetimes_are_cached() {
        let mut g = tg();
        let dt = |s: &str| Term::Literal(oxrdf::Literal::new_typed_literal(s, oxrdf::vocab::xsd::DATE_TIME));
        let nn = |s: &str| Term::NamedNode(oxrdf::NamedNode::new(s).unwrap());
        g.apply_delta(&[[nn("http://ex/newer"), nn("http://ex/at"), dt("2030-01-01T00:00:00Z")]], &[])
            .unwrap();
        let r = query(
            &g,
            r#"PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
               SELECT ?s WHERE { ?s ex:at ?d . FILTER(?d > "2025-01-01T00:00:00Z"^^xsd:dateTime) }"#,
        )
        .unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "<http://ex/newer>");
    }

    /// The compressed (sparse-cache) storage mode must answer temporal filters
    /// identically to the raw build.
    #[test]
    fn compressed_mode_temporal_filter() {
        let g = Graph::load_str(TDATA, "turtle").unwrap().into_compressed();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        let r = query(&g, &format!(r#"{pfx}SELECT ?s WHERE {{ ?s ex:at ?d . FILTER(?d > "2024-03-15T13:00:00Z"^^xsd:dateTime) }}"#)).unwrap();
        assert_eq!(r.rows.len(), 2); // later + subsec, as in the raw build
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
        ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" .
        ex:carol ex:age 35 ; ex:name "Carol" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    fn count(q: &str) -> usize {
        query(&g(), q).unwrap().len()
    }

    #[test]
    fn single_pattern() {
        assert_eq!(count("SELECT ?s WHERE { ?s <http://ex/age> ?a }"), 3);
    }

    // [OPUS-4.8] is_graph_form classifies CONSTRUCT/DESCRIBE (graph-valued) vs SELECT/ASK.
    #[test]
    fn graph_form_classification() {
        let gf = |q: &str| PreparedQuery::parse(q).unwrap().is_graph_form();
        assert!(!gf("SELECT ?s WHERE { ?s <http://ex/age> ?a }"));
        assert!(!gf("ASK { ?s <http://ex/age> ?a }"));
        assert!(gf("CONSTRUCT { ?s <http://ex/a> ?a } WHERE { ?s <http://ex/age> ?a }"));
        assert!(gf("DESCRIBE <http://ex/alice>"));
    }

    #[test]
    fn two_pattern_join() {
        // who does someone know, and what is that person's age
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        // alice->bob(25), bob->carol(35)
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn exact_decimal_arithmetic_filter() {
        // Arithmetic on integer/decimal operands must be EXACT (not f64): `0.1 + 0.2`
        // is 0.3 exactly, `0.3 - 0.1` is 0.2, integers past 2^53 stay distinct. The f64
        // arithmetic path gets all of these wrong.
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:v "0.1"^^xsd:decimal .
            ex:b ex:v "0.2"^^xsd:decimal .
            ex:c ex:v "0.3"^^xsd:decimal .
            ex:d ex:v "9007199254740993"^^xsd:integer ."#;
        let gg = Graph::load_str(data, "turtle").unwrap();
        let n = |q: &str| query(&gg, q).unwrap().len();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        // 0.2 + 0.1 = 0.3 exactly -> the 0.2 row passes.
        assert_eq!(n(&format!("{pfx}SELECT ?s WHERE {{ ?s ex:v ?v FILTER((?v + \"0.1\"^^xsd:decimal) = \"0.3\"^^xsd:decimal) }}")), 1);
        // 0.3 - 0.1 = 0.2 exactly.
        assert_eq!(n(&format!("{pfx}SELECT ?s WHERE {{ ?s ex:v ?v FILTER((?v - \"0.1\"^^xsd:decimal) = \"0.2\"^^xsd:decimal) }}")), 1);
        // 0.1 * 0.1 = 0.01 (none equal, but ordering exact): values < 0.25 are a,b (0.1,0.2).
        assert_eq!(n(&format!("{pfx}SELECT ?s WHERE {{ ?s ex:v ?v FILTER((?v + ?v) <= \"0.4\"^^xsd:decimal) }}")), 2);
        // Integer arithmetic beyond 2^53 stays exact: (n - 1) = 2^53+2 is false for n=2^53+3.
        assert_eq!(n(&format!("{pfx}SELECT ?s WHERE {{ ?s ex:v ?v FILTER((?v - 1) = \"9007199254740992\"^^xsd:integer) }}")), 1);
    }

    #[test]
    fn filter_numeric() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a > 28) }",
        )
        .unwrap();
        assert_eq!(r.len(), 2); // alice(30), carol(35)
    }

    #[test]
    fn distinct_and_limit() {
        assert_eq!(count("SELECT DISTINCT ?p WHERE { ?s ?p ?o }"), 3); // knows, age, name
        assert_eq!(count("SELECT ?s WHERE { ?s ?p ?o } LIMIT 2"), 2);
    }

    #[test]
    fn unsatisfiable_absent_term() {
        assert_eq!(count("SELECT ?s WHERE { ?s <http://ex/nope> ?o }"), 0);
    }

    #[test]
    fn blank_node_is_existential_variable() {
        // _:x acts as a variable: matches any subject with ex:age. SELECT *
        // must not expose the blank-node variable.
        let r = query(&g(), "SELECT * WHERE { _:x <http://ex/age> ?a }").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r.vars.len(), 1); // only ?a, not _:x
        // repeated blank label behaves like a repeated variable (no self-knows here)
        assert_eq!(
            count("PREFIX ex: <http://ex/> SELECT * WHERE { _:x ex:knows _:x }"),
            0
        );
    }

    #[test]
    fn chain_join_three_patterns() {
        // ?a knows ?b, ?b knows ?c : alice->bob->carol
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:knows ?c }",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn star_join_merge() {
        // star on ?s: all three patterns share ?s -> merge joins on ?s
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . ?s ex:name ?n . ?s ex:knows ?k }",
        )
        .unwrap();
        // only alice and bob have knows+age+name
        assert_eq!(r.len(), 2);
    }

    // Differential check: merge-join path must agree with a forced hash-only path
    // across many random small graphs would be ideal; here we cross-check that
    // join result counts are order-independent on a few shapes.
    #[test]
    fn join_two_shared_vars() {
        // ?x ex:knows ?y . ?y ex:knows ?x  (symmetric knows? none here) -> 0
        assert_eq!(
            count("PREFIX ex: <http://ex/> SELECT * WHERE { ?x ex:knows ?y . ?y ex:knows ?x }"),
            0
        );
    }

    #[test]
    fn filter_ebv_boolean_and_string() {
        // FILTER(true) keeps all; numeric/string EBV
        assert_eq!(count("SELECT ?s WHERE { ?s <http://ex/age> ?a . FILTER(true) }"), 3);
        // string EBV: ?n is a non-empty string for all
        assert_eq!(
            count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:name ?n . FILTER(?n) }"),
            3
        );
    }

    #[test]
    fn optional() {
        // carol has no ex:knows -> OPTIONAL leaves ?k unbound for carol
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?k WHERE { ?s ex:age ?a . OPTIONAL { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        let unbound = r.rows.iter().filter(|row| row[1].is_none()).count();
        assert_eq!(unbound, 1); // carol
    }

    #[test]
    fn union() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?x WHERE { { ?x ex:age 30 } UNION { ?x ex:age 25 } }",
        )
        .unwrap();
        assert_eq!(r.len(), 2); // alice, bob
    }

    #[test]
    fn bind_and_arithmetic() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?next WHERE { ?s ex:age ?a . BIND(?a + 1 AS ?next) FILTER(?next > 31) }",
        )
        .unwrap();
        // ages 30,25,35 -> next 31,26,36 -> >31 keeps carol(36)
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn minus() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . MINUS { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert_eq!(r.len(), 1); // only carol has no knows
    }

    #[test]
    fn aggregate_count_and_group() {
        // total count
        let r = query(&g(), "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>");

        // group by predicate, count
        let r = query(
            &g(),
            "SELECT ?p (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?p",
        )
        .unwrap();
        assert_eq!(r.len(), 3); // knows, age, name
    }

    #[test]
    fn aggregate_sum_avg_min_max() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT (SUM(?a) AS ?s)(AVG(?a) AS ?av)(MIN(?a) AS ?mn)(MAX(?a) AS ?mx) WHERE { ?x ex:age ?a }",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        // sum 90, min 25, max 35
        assert!(r.rows[0][0].as_ref().unwrap().to_string().contains("90"));
        assert!(r.rows[0][2].as_ref().unwrap().to_string().contains("25"));
        assert!(r.rows[0][3].as_ref().unwrap().to_string().contains("35"));
    }

    /// [OPUS-4.8] roborev 1610 (High): SUM/AVG over a column that is UNBOUND for some
    /// rows (here via OPTIONAL) must still aggregate the rows that DO have a value — an
    /// unbound member is skipped, not fatal. Previously any unbound member collapsed the
    /// whole aggregate to unbound.
    #[test]
    fn sum_avg_skip_unbound_members() {
        // ex:a..ex:d all exist; only a (10) and c (30) carry ex:score.
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:a ex:t 1 . ex:b ex:t 1 . ex:c ex:t 1 . ex:d ex:t 1 .
            ex:a ex:score 10 . ex:c ex:score 30 ."#;
        let gg = Graph::load_str(data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> ";
        let one = |q: &str| -> String {
            let r = query(&gg, &format!("{pfx}{q}")).unwrap();
            r.rows[0][0].as_ref().unwrap().to_string()
        };
        // SUM/AVG over the OPTIONAL ?score: two bound rows (10, 30) out of four solutions.
        let s = one("SELECT (SUM(?score) AS ?v) WHERE { ?s ex:t ?t OPTIONAL { ?s ex:score ?score } }");
        assert!(s.contains("40"), "SUM over OPTIONAL must sum bound rows only, got {s}");
        let a = one("SELECT (AVG(?score) AS ?v) WHERE { ?s ex:t ?t OPTIONAL { ?s ex:score ?score } }");
        assert!(a.contains("20"), "AVG over OPTIONAL must average bound rows only, got {a}");
    }

    /// [OPUS-4.8] roborev 1610 (High/Med): CEIL/FLOOR/ROUND on a decimal with a very
    /// large fractional scale must not overflow `10^scale` (debug panic / release wrap).
    /// The value here is +/-1e-40, i.e. magnitude < 0.5, so the rounded results follow
    /// purely from the sign.
    #[test]
    fn round_large_scale_decimal_no_overflow() {
        let tiny = format!("0.{}1", "0".repeat(39)); // scale 40, value 1e-40
        let data = format!(
            r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:p ex:v "{tiny}"^^xsd:decimal .
            ex:n ex:v "-{tiny}"^^xsd:decimal ."#
        );
        let gg = Graph::load_str(&data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        let n = |q: &str| query(&gg, &format!("{pfx}{q}")).unwrap().len();
        // CEIL(+1e-40)=1, FLOOR(+1e-40)=0, ROUND(+1e-40)=0  (decimal-typed).
        assert_eq!(n("SELECT ?s WHERE { ex:p ex:v ?v FILTER(CEIL(?v) = \"1\"^^xsd:decimal) }"), 1);
        assert_eq!(n("SELECT ?s WHERE { ex:p ex:v ?v FILTER(FLOOR(?v) = \"0\"^^xsd:decimal) }"), 1);
        assert_eq!(n("SELECT ?s WHERE { ex:p ex:v ?v FILTER(ROUND(?v) = \"0\"^^xsd:decimal) }"), 1);
        // CEIL(-1e-40)=0, FLOOR(-1e-40)=-1, ROUND(-1e-40)=0.
        assert_eq!(n("SELECT ?s WHERE { ex:n ex:v ?v FILTER(CEIL(?v) = \"0\"^^xsd:decimal) }"), 1);
        assert_eq!(n("SELECT ?s WHERE { ex:n ex:v ?v FILTER(FLOOR(?v) = \"-1\"^^xsd:decimal) }"), 1);
        assert_eq!(n("SELECT ?s WHERE { ex:n ex:v ?v FILTER(ROUND(?v) = \"0\"^^xsd:decimal) }"), 1);
    }

    /// [OPUS-4.8] sq-l11x2: fn:round at the FLOAT tiers must not double-round. The xsd:double
    /// value just below one-half (0.49999999999999994, the f64 predecessor of 0.5) must ROUND
    /// to 0, NOT 1 — the naive `(x + 0.5).floor()` returns 1 because `x + 0.5` rounds up to
    /// exactly 1.0 before the floor. Same defect at the xsd:float tier.
    #[test]
    fn round_float_tier_no_double_rounding_via_sparql() {
        let data = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:p ex:v "0.49999999999999994"^^xsd:double .
            ex:f ex:v "0.49999997"^^xsd:float ."#;
        let gg = Graph::load_str(data, "turtle").unwrap();
        let pfx = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
        let n = |q: &str| query(&gg, &format!("{pfx}{q}")).unwrap().len();
        // xsd:double just below 1/2 rounds DOWN to 0, and NOT up to 1.
        assert_eq!(n("SELECT ?s WHERE { ex:p ex:v ?v FILTER(ROUND(?v) = 0.0) }"), 1);
        assert_eq!(n("SELECT ?s WHERE { ex:p ex:v ?v FILTER(ROUND(?v) = 1.0) }"), 0);
        // xsd:float predecessor of 1/2 likewise rounds to 0.
        assert_eq!(n("SELECT ?s WHERE { ex:f ex:v ?v FILTER(ROUND(?v) = 0.0) }"), 1);
    }

    /// [OPUS-4.8] roborev 1429 (Med): GROUP BY aggregate over a >PAR_THRESHOLD (50k) input
    /// must produce complete, correct per-group results. This crosses the chunked parallel
    /// eval+intern boundary (default `parallel` feature) and the streaming sequential path
    /// (no-default), which must agree group-for-group. 1,000 groups x 60 rows = 60,000 rows;
    /// group `k` has values k..k+60 so SUM = 60*k + (0+..+59) = 60*k + 1770.
    #[test]
    fn group_aggregate_above_par_threshold_complete() {
        let mut data = String::from("@prefix ex: <http://ex/> .\n");
        for k in 0..1000u32 {
            for i in 0..60u32 {
                data.push_str(&format!("ex:s{k} ex:v {} .\n", k + i));
            }
        }
        let gg = Graph::load_str(&data, "turtle").unwrap();
        let r = query(
            &gg,
            "PREFIX ex: <http://ex/> SELECT ?s (SUM(?v) AS ?sum) WHERE { ?s ex:v ?v } GROUP BY ?s",
        )
        .unwrap();
        assert_eq!(r.len(), 1000, "every group must be emitted exactly once");
        // Index group -> sum string and spot-check a few groups across the chunk boundary.
        use std::collections::HashMap;
        let sums: HashMap<String, String> = r
            .rows
            .iter()
            .map(|row| (row[0].as_ref().unwrap().to_string(), row[1].as_ref().unwrap().to_string()))
            .collect();
        for k in [0u32, 1, 499, 500, 833, 999] {
            let want = 60 * k + 1770;
            let got = sums.get(&format!("<http://ex/s{k}>")).expect("group present");
            assert!(got.contains(&want.to_string()), "group {k}: SUM want {want}, got {got}");
        }
    }

    #[test]
    fn order_by() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY DESC(?a)",
        )
        .unwrap();
        let ages: Vec<String> = r.rows.iter().map(|row| row[1].as_ref().unwrap().to_string()).collect();
        // 35, 30, 25 descending
        assert!(ages[0].contains("35") && ages[2].contains("25"));
    }

    #[test]
    fn sameterm_numeric_identity() {
        // sameTerm on a numeric variable with itself is true — the numeric fast
        // path must not discard term identity (regression for roborev 1271).
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(sameTerm(?a, ?a)) }",
        )
        .unwrap();
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn bind_preserves_numeric_lexical_form() {
        // BIND(?x AS ?y) must re-emit the ORIGINAL term, not a canonicalised number
        // (regression for roborev 1271): "1.0"^^decimal must stay "1.0"^^decimal.
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:score \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal> .",
            "turtle",
        )
        .unwrap();
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?d WHERE { ?s ex:score ?sc . BIND(?sc AS ?d) }",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            r.rows[0][0].as_ref().unwrap().to_string(),
            "\"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>"
        );
    }

    #[test]
    fn filter_numeric_still_fast_path() {
        // The numeric comparison fast path still gives correct results.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a + 1 > 31) }",
        )
        .unwrap();
        // ages 30,25,35 -> +1 = 31,26,36 -> >31 keeps only carol(36)
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn filter_pushdown_with_join() {
        // A numeric FILTER on a variable that is also a join variable: the filter
        // is pushed into that pattern's scan, then joined. Must stay correct.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?k WHERE { ?s ex:age ?a . ?s ex:knows ?k . FILTER(?a > 28) }",
        )
        .unwrap();
        // ages: alice 30, bob 25, carol 35. >28 keeps alice, carol. knows: alice->bob,
        // bob->carol. After filter+join on ?s: only alice(30)->bob (carol has no knows).
        assert_eq!(r.len(), 1);
        assert!(r.rows[0][0].as_ref().unwrap().to_string().contains("alice"));
    }

    #[test]
    fn filter_pushdown_boundaries() {
        // >=, <=, = pushed-down comparisons.
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a >= 30) }"), 2);
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a <= 30) }"), 2);
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a = 25) }"), 1);
    }

    #[test]
    fn planner_four_pattern_multi_predicate() {
        // 4 patterns over predicates of different selectivity (knows/age/name) —
        // exercises the cost-based GOO planner's candidate scoring. Result must be
        // correct regardless of the order it picks.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?k ?ka WHERE { \
               ?s ex:knows ?k . ?s ex:age ?a . ?s ex:name ?n . ?k ex:age ?ka }",
        )
        .unwrap();
        // alice->bob(25), bob->carol(35); carol has no knows
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn planner_path_vs_naive() {
        // 3-hop path ?a e ?b . ?b e ?c . ?c e ?d over a random graph, checked
        // against a brute-force count — the planner orders 3 candidates and the
        // result must match a naive evaluator.
        let mut seed = 0x00C0FFEEu64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let n = 22u32;
        let mut edges: Vec<(u32, u32)> = Vec::new();
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for _ in 0..130 {
            let (a, b) = (next() % n, next() % n);
            edges.push((a, b));
            ttl.push_str(&format!("ex:n{a} ex:e ex:n{b} .\n"));
        }
        edges.sort_unstable();
        edges.dedup();
        let graph = Graph::load_str(&ttl, "turtle").unwrap();

        // naive 3-hop path count: a->b, b->c, c->d
        let mut naive = 0usize;
        for &(_, b) in &edges {
            for &(b2, c) in &edges {
                if b2 != b {
                    continue;
                }
                for &(c2, _) in &edges {
                    if c2 == c {
                        naive += 1;
                    }
                }
            }
        }
        let q = query(
            &graph,
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?d }",
        )
        .unwrap();
        assert_eq!(q.len(), naive);
    }

    #[test]
    fn lazy_count_matches_materialized() {
        // The lazy count() (single-pattern range size, two-pattern group-count
        // join) must equal the materialised result length.
        let cases = [
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a }", // single pattern
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:age ?age }", // chain join
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a . ?s ex:name ?n }", // subject-shared
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?x ex:knows ?x }",               // repeated var
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:knows ?c }", // chain
            // 3-pattern STAR on ?s — the N-star count path (Σ_s Πc_i(s)).
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a . ?s ex:name ?n . ?s ex:knows ?k }",
            // star with a constant-object pattern mixed in.
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a . ?s ex:name ?n . ?s ex:knows ex:bob }",
            // 3-pattern CHAIN — NOT a star; must fall back to materialised count (still correct).
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:knows ?c . ?c ex:age ?x }",
            // OPTIONAL — lazy left-join count Σ_s c_left(s)·max(1, c_right(s)).
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:age ?a } }",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:knows ?k } }",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }",
        ];
        for q in cases {
            assert_eq!(super::count(&g(), q).unwrap(), query(&g(), q).unwrap().len(), "count mismatch: {q}");
        }
    }

    #[test]
    fn lazy_count_join_vs_naive_random() {
        // Two-pattern group-count join vs the materialised count over a random graph.
        let mut seed = 0xBEEF_1234u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for _ in 0..140 {
            ttl.push_str(&format!("ex:n{} ex:e ex:n{} .\n", next() % 18, next() % 18));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let q = "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c }";
        assert_eq!(super::count(&g, q).unwrap(), query(&g, q).unwrap().len());
    }

    #[test]
    fn limit_early_termination() {
        // LIMIT over a single-pattern scan (early-terminating path).
        assert_eq!(count("SELECT * WHERE { ?s ?p ?o } LIMIT 5"), 5);
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } LIMIT 2"), 2);
        // OFFSET + LIMIT.
        assert_eq!(count("SELECT * WHERE { ?s ?p ?o } LIMIT 3 OFFSET 2"), 3);
        // LIMIT larger than the result is fine.
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } LIMIT 100"), 3);
        // LIMIT with a pushed-down sargable filter.
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a > 20) } LIMIT 1"), 1);
    }

    #[test]
    fn limit_with_order_by_is_correct() {
        // ORDER BY before LIMIT must NOT early-terminate — it returns the globally
        // smallest, not just the first scanned.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a LIMIT 1",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert!(r.rows[0][1].as_ref().unwrap().to_string().contains("25")); // bob, youngest
    }

    #[test]
    fn having() {
        // group ?s, count its triples, keep groups with >= 3 triples
        let r = query(
            &g(),
            "SELECT ?s (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?s HAVING (COUNT(*) >= 3)",
        )
        .unwrap();
        // alice & bob have 3 triples each (knows,age,name); carol has 2
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn values_clause() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a . VALUES ?s { ex:alice ex:carol } }",
        )
        .unwrap();
        assert_eq!(r.len(), 2); // alice(30), carol(35)
    }

    #[test]
    fn sub_select() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { { SELECT ?s WHERE { ?s ex:age ?a } ORDER BY DESC(?a) LIMIT 1 } }",
        )
        .unwrap();
        assert_eq!(r.len(), 1); // carol (oldest)
    }

    #[test]
    fn values_undef_is_wildcard() {
        // VALUES with UNDEF for ?s must join with every binding, not just an
        // (impossible) "unbound" id. Expect all three people.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a . VALUES ?s { UNDEF } }",
        )
        .unwrap();
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn optional_then_join_unbound_compatible() {
        // "Non-well-designed" nested OPTIONAL (Pérez–Arenas–Gutierrez): ?k is
        // introduced in the first OPTIONAL and reused in the second. Under the
        // SPARQL bottom-up algebra, carol's UNBOUND ?k is compatible with EVERY
        // `?k ex:name ?kn` row, so the second LeftJoin pairs carol with all three
        // names. This (surprising) count of 5 is the spec-correct answer and
        // exercises the unbound-as-wildcard path in the compatibility join.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?kn WHERE { \
               ?s ex:age ?a . \
               OPTIONAL { ?s ex:knows ?k } \
               OPTIONAL { ?k ex:name ?kn } }",
        )
        .unwrap();
        // alice->\"Bob\", bob->\"Carol\", carol-><\"Alice\",\"Bob\",\"Carol\"> = 5
        assert_eq!(r.len(), 5);
        assert!(r.rows.iter().all(|row| row[1].is_some())); // every ?kn ends up bound
    }

    #[test]
    fn computed_values_dedup() {
        // Two distinct ages (30,35,25) all map via floor()-like BIND to a constant
        // computed literal; DISTINCT must collapse them to one row, proving equal
        // computed terms share a local id.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT DISTINCT ?c WHERE { ?s ex:age ?a . BIND(1 AS ?c) }",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn minus_undef_domain_overlap() {
        // Left has ?k possibly UNBOUND (via OPTIONAL); right binds ?k. MINUS must
        // exercise the general compatibility+domain-overlap path:
        //   alice(k=bob)   -> right has k=bob   -> compatible & overlap -> REMOVED
        //   bob(k=carol)   -> right has k=carol -> compatible & overlap -> REMOVED
        //   carol(k=UNDEF) -> compatible with all, but NO bound overlap -> KEPT
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { \
               ?s ex:age ?a . \
               OPTIONAL { ?s ex:knows ?k } \
               MINUS { ?x ex:knows ?k } }",
        )
        .unwrap();
        assert_eq!(r.len(), 1); // only carol survives
        assert!(r.rows[0][0].as_ref().unwrap().to_string().contains("carol"));
    }

    #[test]
    fn range_pruning_boundaries_and_mixed_columns() {
        // Inline-integer range-pruning: an all-canonical-integer column binary-searches
        // to the passing value range. Check every operator at exact boundaries, plus
        // out-of-range and empty cases. Ages 10,20,30,40,50.
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:age 10 . ex:b ex:age 20 . ex:c ex:age 30 . ex:d ex:age 40 . ex:e ex:age 50 .",
            "turtle",
        )
        .unwrap();
        let c = |q: &str| query(&g, &format!("PREFIX ex: <http://ex/> SELECT ?s WHERE {{ ?s ex:age ?a . FILTER({q}) }}")).unwrap().len();
        assert_eq!(c("?a > 30"), 2); // 40,50
        assert_eq!(c("?a >= 30"), 3); // 30,40,50
        assert_eq!(c("?a < 30"), 2); // 10,20
        assert_eq!(c("?a <= 30"), 3);
        assert_eq!(c("?a = 30"), 1);
        assert_eq!(c("?a != 30"), 4);
        assert_eq!(c("?a > 50"), 0); // above max -> empty
        assert_eq!(c("?a < 10"), 0); // below min -> empty
        assert_eq!(c("?a >= 0"), 5); // all
        assert_eq!(c("?a > -100"), 5); // negative threshold (non-sargable) -> all

        // MIXED column: the range-pruning guard must fall back to a full scan so a
        // non-inline numeric (xsd:int) that passes the filter is NOT dropped.
        let gm = Graph::load_str(
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> . \
             ex:a ex:v 50 . ex:b ex:v \"200\"^^xsd:int . ex:c ex:v \"95\"^^xsd:integer . ex:d ex:v \"-10\"^^xsd:integer .",
            "turtle",
        )
        .unwrap();
        let cm = |q: &str| query(&gm, &format!("PREFIX ex: <http://ex/> SELECT ?s WHERE {{ ?s ex:v ?v . FILTER({q}) }}")).unwrap().len();
        assert_eq!(cm("?v > 90"), 2); // 95 (inline) AND 200 (xsd:int, non-inline) — both kept
        assert_eq!(cm("?v < 60"), 2); // 50 and -10
    }

    #[test]
    fn query_json_matches_materialized_json() {
        // The direct id->JSON path must produce byte-identical output to building the
        // QueryResult (Terms) then serialising — across IRIs (prefix-factored), inline
        // integers, language tags, OPTIONAL-unbound cells, and computed aggregates.
        let queries = [
            "SELECT * WHERE { ?s ?p ?o }",
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a }",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:knows ?k } }",
            "PREFIX ex: <http://ex/> SELECT (AVG(?a) AS ?avg) WHERE { ?s ex:age ?a }",
            "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n } ORDER BY ?n",
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(?a > 28) }",
        ];
        for q in queries {
            let direct = query_json(&g(), q).unwrap();
            let via_result = json::to_sparql_json(&query(&g(), q).unwrap());
            assert_eq!(direct, via_result, "json mismatch for: {q}");
        }
    }

    #[test]
    fn relational_type_error_semantics() {
        // A numeric ordering comparison against a NON-numeric term is a SPARQL type
        // error -> the row is excluded (NOT a string comparison). Regression for the
        // adversarial-audit finding: `?v > -1` with ?v a string wrongly passed via a
        // lexical-byte comparison. A negative threshold is non-sargable, so this runs
        // the residual compare path. Cross-checks several non-numeric term kinds.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s1 ex:v 50 .
               ex:s2 ex:v "hi" .
               ex:s3 ex:v ex:thing .
               ex:s4 ex:v "true"^^xsd:boolean .
               ex:s5 ex:v "bonjour"@fr ."#,
            "turtle",
        )
        .unwrap();
        let cnt = |q: &str| query(&g, q).unwrap().len();
        // Only the number 50 satisfies an ordering comparison; every non-numeric is a
        // type error -> excluded.
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v > -1) }"), 1);
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v < 100) }"), 1);
        // OPEN-WORLD `=` / `!=` (W3C open-world suite): an IRI and a language-tagged
        // literal are KNOWN different from a number (`!=` true), but a plain string or
        // boolean against a number is a cross-family TYPE ERROR -> excluded. So 50,
        // ex:thing and "bonjour"@fr pass; "hi" and true error out.
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v != 5) }"), 3);
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v = 5) }"), 0);
        // Value equality across integer datatypes still holds.
        let g2 = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:v \"05\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "turtle",
        )
        .unwrap();
        assert_eq!(query(&g2, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v = 5) }").unwrap().len(), 1);
        // 3-valued OR: `(?v > -1) || (?v = "hi")` — s1 true via >, s2 ("hi") true via =,
        // the rest error||false = false. So 2 rows.
        assert_eq!(
            cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER((?v > -1) || (?v = \"hi\")) }"),
            2
        );
    }

    #[test]
    fn logical_error_propagation_and_short_circuit() {
        // Roborev follow-ups to the type-error fix: IF must propagate a condition
        // error (not select the else branch); && / || must short-circuit on the
        // dominating value; IN must reuse `=` semantics. Two rows: 50 and "hi".
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:s1 ex:v 50 . ex:s2 ex:v \"hi\" .",
            "turtle",
        )
        .unwrap();
        let cnt = |q: &str| query(&g, q).unwrap().len();
        // IF(error, _, _) is an error -> the "hi" row is excluded (NOT the else branch).
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(IF(?v > -1, true, true)) }"), 1);
        // IN reuses `=`: only the numeric matches the numeric list entry.
        assert_eq!(cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER(?v IN (50)) }"), 1);

        // Short-circuit, proven rigorously: STRLEN is UNSUPPORTED, so evaluating the
        // right operand returns Err and `query` would fail. The query SUCCEEDING is
        // proof the dominating left operand skipped the right (not just that the
        // truth table tolerates an error).
        let g1 = Graph::load_str("@prefix ex: <http://ex/> . ex:s1 ex:v 50 .", "turtle").unwrap();
        let r1 = query(&g1, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER((?v = 99) && (STRLEN(?v) > 0)) }");
        assert_eq!(r1.expect("false && _ must short-circuit, not error").len(), 0);
        let r2 = query(&g1, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v FILTER((?v = 50) || (STRLEN(?v) > 0)) }");
        assert_eq!(r2.expect("true || _ must short-circuit, not error").len(), 1);

        // IN preserves a type error (unbound operand): `!(?k IN (ex:x))` with ?k
        // unbound is error -> false -> excluded, NOT `!(false)` = true -> included.
        assert_eq!(
            cnt("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:v ?v OPTIONAL { ?s ex:nomatch ?k } FILTER(!(?k IN (<http://ex/x>))) }"),
            0
        );
    }

    #[test]
    fn from_and_from_named_define_the_active_dataset() {
        // Store: default graph (1 triple), :g1 (2), :g2 (1). A dataset clause must
        // REPLACE the dataset: FROM merges into the active default graph, FROM NAMED
        // selects the active named graphs, and the store's own graphs never leak in.
        let src = "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
                   <http://ex/x> <http://ex/p> <http://ex/y> <http://ex/g1> .\n\
                   <http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
                   <http://ex/x> <http://ex/q> <http://ex/z> <http://ex/g2> .";
        let g = Graph::load_dataset(src, "nquads").unwrap();
        let n = |q: &str| query(&g, q).unwrap().len();
        // FROM g1: the store default graph is re-scoped away.
        assert_eq!(n("SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }"), 2);
        // FROM g1 + g2 merge; no FROM NAMED -> GRAPH matches nothing.
        assert_eq!(n("SELECT * FROM <http://ex/g1> FROM <http://ex/g2> WHERE { ?s ?p ?o }"), 3);
        assert_eq!(n("SELECT * FROM <http://ex/g1> WHERE { GRAPH ?g { ?s ?p ?o } }"), 0);
        // FROM NAMED only: empty active default graph, exactly that named graph.
        assert_eq!(n("SELECT * FROM NAMED <http://ex/g2> WHERE { ?s ?p ?o }"), 0);
        assert_eq!(n("SELECT * FROM NAMED <http://ex/g2> WHERE { GRAPH ?g { ?s ?p ?o } }"), 1);
        // An absent graph name denotes the empty graph (no error, no rows).
        assert_eq!(n("SELECT * FROM <http://ex/nope> WHERE { ?s ?p ?o }"), 0);
        // ASK / count / JSON run against the same active dataset.
        assert!(!ask(&g, "ASK FROM NAMED <http://ex/g1> { ?s ?p ?o }").unwrap());
        assert!(ask(&g, "ASK FROM <http://ex/g1> { ?s ?p ?o }").unwrap());
        assert_eq!(super::count(&g, "SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }").unwrap(), 2);
        assert_eq!(
            query_json(&g, "SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }").unwrap(),
            json::to_sparql_json(&query(&g, "SELECT * FROM <http://ex/g1> WHERE { ?s ?p ?o }").unwrap())
        );
        // CONSTRUCT honours the clause too.
        let ts = construct(&g, "CONSTRUCT { ?s ?p ?o } FROM <http://ex/g2> WHERE { ?s ?p ?o }").unwrap();
        assert_eq!(ts.len(), 1);
        // No dataset clause: the store itself, unchanged.
        assert_eq!(n("SELECT * WHERE { ?s ?p ?o }"), 1);
    }

    #[test]
    fn named_graph_query_over_default_only_is_empty() {
        // A graph loaded without named graphs: GRAPH ?g matches nothing (no error). Named-graph
        // querying over an actual dataset is covered in exec::path_tests::named_graphs.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { GRAPH ?g { ?s ex:age ?a } }",
        )
        .unwrap();
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn bnode_prefix_does_not_collide() {
        // A user variable that looks like the old synthetic prefix must be a real,
        // projected SELECT * variable now that synthetic vars use an illegal char.
        let r = query(&g(), "SELECT * WHERE { ?__bn_x <http://ex/age> ?a }").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r.vars.len(), 2); // both ?__bn_x and ?a are visible
    }

    #[test]
    fn ask_true_false_and_result_forms() {
        // Satisfiable and unsatisfiable patterns.
        assert!(ask(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }").unwrap());
        assert!(!ask(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }").unwrap());
        // ASK with FILTER / join.
        assert!(ask(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 34) }").unwrap());
        assert!(!ask(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 100) }").unwrap());
        // query(): unit-row encoding (zero vars, 0/1 rows).
        let r = query(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age 30 }").unwrap();
        assert_eq!((r.vars.len(), r.rows.len()), (0, 1));
        let r = query(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age 31 }").unwrap();
        assert_eq!((r.vars.len(), r.rows.len()), (0, 0));
        // count(): 1 / 0.
        assert_eq!(super::count(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o }").unwrap(), 1);
        assert_eq!(super::count(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }").unwrap(), 0);
        // query_json(): the SPARQL 1.1 JSON boolean form.
        assert_eq!(
            query_json(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o }").unwrap(),
            "{\"head\":{},\"boolean\":true}"
        );
        assert_eq!(
            query_json(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }").unwrap(),
            "{\"head\":{},\"boolean\":false}"
        );
        // ask() on a non-ASK query is a clear error.
        assert!(ask(&g(), "SELECT * WHERE { ?s ?p ?o }").is_err());
    }

    #[test]
    fn exists_and_not_exists() {
        // EXISTS correlated on the outer row: people who know someone.
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a FILTER EXISTS { ?s ex:knows ?o } }"), 2);
        // NOT EXISTS: people who know no-one (carol).
        let r = query(&g(), "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a FILTER NOT EXISTS { ?s ex:knows ?o } }").unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "<http://ex/carol>");
        // Uncorrelated EXISTS: a satisfiable / unsatisfiable constant pattern keeps / drops all rows.
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a FILTER EXISTS { ex:alice ex:knows ex:bob } }"), 3);
        assert_eq!(count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a FILTER EXISTS { ex:alice ex:knows ex:carol } }"), 0);
        // Nested EXISTS (exists04 shape): knows someone who is known by someone.
        assert_eq!(
            count(
                "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a \
                 FILTER EXISTS { ?s ex:knows ?o FILTER EXISTS { ?o ex:knows ?p } } }"
            ),
            1 // alice: knows bob, and bob knows carol
        );
        // NOT EXISTS in ASK.
        assert!(ask(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age 35 FILTER NOT EXISTS { ?s ex:knows ?o } }").unwrap());
    }

    #[test]
    #[cfg(feature = "digest")]
    fn hash_builtins() {
        let one = |q: &str| {
            let r = query(&g(), q).unwrap();
            r.rows[0][0].as_ref().unwrap().to_string()
        };
        // RFC / spec vectors for "abc".
        assert_eq!(one("SELECT (MD5(\"abc\") AS ?h) {}"), "\"900150983cd24fb0d6963f7d28e17f72\"");
        assert_eq!(one("SELECT (SHA1(\"abc\") AS ?h) {}"), "\"a9993e364706816aba3e25717850c26c9cd0d89d\"");
        assert_eq!(
            one("SELECT (SHA256(\"abc\") AS ?h) {}"),
            "\"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\""
        );
        // A language-tagged operand is a type error -> unbound.
        let r = query(&g(), "SELECT (MD5(\"abc\"@en) AS ?h) {}").unwrap();
        assert!(r.rows[0][0].is_none());
    }

    #[test]
    fn timezone_tz_bnode_uuid_builtins() {
        let one = |q: &str| {
            let r = query(&g(), q).unwrap();
            r.rows[0][0].as_ref().map(|t| t.to_string())
        };
        let dt = "\"2010-12-21T15:38:02-08:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime>";
        assert_eq!(one(&format!("SELECT (TZ({dt}) AS ?x) {{}}")).unwrap(), "\"-08:00\"");
        assert_eq!(
            one(&format!("SELECT (TIMEZONE({dt}) AS ?x) {{}}")).unwrap(),
            "\"-PT8H\"^^<http://www.w3.org/2001/XMLSchema#dayTimeDuration>"
        );
        let dtz = "\"2010-12-21T15:38:02Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>";
        assert_eq!(one(&format!("SELECT (TZ({dtz}) AS ?x) {{}}")).unwrap(), "\"Z\"");
        assert_eq!(
            one(&format!("SELECT (TIMEZONE({dtz}) AS ?x) {{}}")).unwrap(),
            "\"PT0S\"^^<http://www.w3.org/2001/XMLSchema#dayTimeDuration>"
        );
        // No timezone: TZ -> "", TIMEZONE -> type error (unbound).
        let dn = "\"2011-02-01T01:02:03\"^^<http://www.w3.org/2001/XMLSchema#dateTime>";
        assert_eq!(one(&format!("SELECT (TZ({dn}) AS ?x) {{}}")).unwrap(), "\"\"");
        assert_eq!(one(&format!("SELECT (TIMEZONE({dn}) AS ?x) {{}}")), None);

        // BNODE(): two calls in one row give two distinct fresh blank nodes.
        let r = query(&g(), "SELECT (BNODE() AS ?a) (BNODE() AS ?b) {}").unwrap();
        let (a, b) = (r.rows[0][0].as_ref().unwrap(), r.rows[0][1].as_ref().unwrap());
        assert_ne!(a, b);
        // BNODE(str): same argument in the same solution -> same bnode; different
        // rows -> different bnodes.
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT (BNODE(\"x\") AS ?a) (BNODE(\"x\") AS ?b) WHERE { ?s ex:age ?n }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 3);
        for row in &r.rows {
            assert_eq!(row[0], row[1]);
        }
        assert_ne!(r.rows[0][0], r.rows[1][0]);

        // UUID()/STRUUID() (native targets).
        let u = one("SELECT (UUID() AS ?u) {}").unwrap();
        assert!(u.starts_with("<urn:uuid:") && u.len() == 47, "got: {u}");
        let s = one("SELECT (STRUUID() AS ?s) {}").unwrap();
        assert_eq!(s.len(), 38); // quoted 36-char UUID
        assert_ne!(one("SELECT (STRUUID() AS ?s) {}").unwrap(), s);
    }

    #[test]
    fn string_functions_preserve_language_tags() {
        let one = |q: &str| {
            let r = query(&g(), q).unwrap();
            r.rows[0][0].as_ref().map(|t| t.to_string())
        };
        assert_eq!(one("SELECT (UCASE(\"bar\"@en) AS ?x) {}").unwrap(), "\"BAR\"@en");
        assert_eq!(one("SELECT (LCASE(\"BAR\"@en) AS ?x) {}").unwrap(), "\"bar\"@en");
        assert_eq!(one("SELECT (SUBSTR(\"bar\"@en, 2) AS ?x) {}").unwrap(), "\"ar\"@en");
        assert_eq!(one("SELECT (SUBSTR(\"bar\"@en, 1, 1) AS ?x) {}").unwrap(), "\"b\"@en");
        // CONCAT: same tag everywhere -> tagged; mixed -> simple; non-string -> error.
        assert_eq!(one("SELECT (CONCAT(\"a\"@en, \"b\"@en) AS ?x) {}").unwrap(), "\"ab\"@en");
        assert_eq!(one("SELECT (CONCAT(\"a\"@en, \"b\") AS ?x) {}").unwrap(), "\"ab\"");
        assert_eq!(one("SELECT (CONCAT(\"a\", 1) AS ?x) {}"), None);
        // STRBEFORE/STRAFTER: result carries arg1's tag on a match, plain "" on no
        // match, and incompatible language tags are a type error.
        assert_eq!(one("SELECT (STRBEFORE(\"abc\"@en, \"b\") AS ?x) {}").unwrap(), "\"a\"@en");
        assert_eq!(one("SELECT (STRAFTER(\"abc\"@en, \"b\"@en) AS ?x) {}").unwrap(), "\"c\"@en");
        assert_eq!(one("SELECT (STRBEFORE(\"abc\"@en, \"z\") AS ?x) {}").unwrap(), "\"\"");
        assert_eq!(one("SELECT (STRBEFORE(\"abc\"@en, \"b\"@cy) AS ?x) {}"), None);
        assert_eq!(one("SELECT (STRAFTER(\"abc\", \"b\"@en) AS ?x) {}"), None);
        // REPLACE keeps the text's tag.
        #[cfg(feature = "regex")]
        assert_eq!(one("SELECT (REPLACE(\"abc\"@en, \"b\", \"-\") AS ?x) {}").unwrap(), "\"a-c\"@en");
        // STRDT/STRLANG require a simple-literal first argument.
        assert_eq!(
            one("PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT (STRDT(\"1\", xsd:integer) AS ?x) {}").unwrap(),
            "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        );
        assert_eq!(one("PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT (STRDT(\"1\"@en, xsd:integer) AS ?x) {}"), None);
        assert_eq!(one("SELECT (STRLANG(\"a\", \"en\") AS ?x) {}").unwrap(), "\"a\"@en");
        assert_eq!(one("SELECT (STRLANG(\"a\"@en, \"en\") AS ?x) {}"), None);
    }

    #[test]
    fn query_json_chunks_concat_is_byte_identical() {
        // The streamed chunk sequence must concatenate to EXACTLY the single-string
        // JSON — across the single-pattern fast path, the general path, OPTIONAL
        // unbounds, aggregates and ASK.
        let queries = [
            "SELECT * WHERE { ?s ?p ?o }",                                                  // fast path
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a }",                  // fast path, projected
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:knows ?k } }", // general
            "PREFIX ex: <http://ex/> SELECT (AVG(?a) AS ?avg) WHERE { ?s ex:age ?a }",      // aggregate
            "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }",                                 // boolean form
        ];
        let b = QueryBudget::unlimited();
        for q in queries {
            let single = query_json(&g(), q).unwrap();
            let chunks = query_json_chunks_with_budget(&g(), q, &b).unwrap();
            assert_eq!(chunks.concat(), single, "chunk concat mismatch for: {q}");
        }

        // A result big enough to actually split (>64 KiB of JSON): every chunk
        // boundary must fall so that the concatenation is still byte-identical.
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..3000 {
            ttl.push_str(&format!("ex:subject{i} ex:somePredicate \"value-{i}-padding-padding\" .\n"));
        }
        let big = Graph::load_str(&ttl, "turtle").unwrap();
        for q in ["SELECT * WHERE { ?s ?p ?o }", "SELECT ?s ?o WHERE { ?s ?p ?o . ?s ?p2 ?o }"] {
            let single = query_json(&big, q).unwrap();
            let chunks = query_json_chunks_with_budget(&big, q, &b).unwrap();
            assert!(chunks.len() > 1, "expected a multi-chunk stream for: {q}");
            assert_eq!(chunks.concat(), single, "chunk concat mismatch for: {q}");
        }
    }

    #[test]
    fn query_json_chunks_respects_budget() {
        let b = QueryBudget { max_rows: Some(3), ..QueryBudget::unlimited() };
        let e = query_json_chunks_with_budget(&g(), "SELECT * WHERE { ?s ?p ?o }", &b).unwrap_err();
        assert!(e.contains("query budget exceeded (max-rows)"), "got: {e}");
    }

    /// [SONNET-4.6] (sq-yfcu2) The general (multi-pattern) SELECT-JSON path bounds the
    /// SERIALIZE step by the budget, not just evaluation. The pre-serialize gate prices the
    /// row/byte caps exactly (the rows are materialised, so the count is known), but
    /// serialising a large materialised set is itself unbounded work — a deadline can fall
    /// due *during* it, and before this the full body was serialised and returned as if it
    /// were in time. The post-serialization gate now reports the trip instead.
    ///
    /// Pinned deterministically with the CANCEL limit rather than a wall clock: the sink
    /// raises the flag when it receives the FIRST flush chunk, i.e. strictly after the
    /// pre-serialize gate has passed and while the body is still being produced — exactly
    /// the mid-serialize window a deadline falls into, and `budget::exhausted` treats the
    /// two identically. Covers BOTH branches: a below-threshold result (the cooperative
    /// serial loop) and a >`PAR_THRESHOLD` one (the rayon fan-out, where the whole set is
    /// already serialised when the trip is seen, so only the post-serialization gate can
    /// catch it).
    #[test]
    fn json_serialize_is_budget_bounded_mid_stream() {
        use std::ops::ControlFlow;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        // A two-pattern (general-path) query over enough rows that the body exceeds the
        // 64 KiB flush size several times over, so the sink is reached mid-serialize.
        // `parallel` is default-on, so the 60k case takes the fan-out branch.
        let q = "SELECT ?s ?o WHERE { ?s ?p ?o . ?s ?p2 ?o }";
        for rows in [3_000u32, 60_000] {
            let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
            for i in 0..rows {
                ttl.push_str(&format!("ex:s{} ex:p \"value-{}-padding-padding-padding\" .\n", i, i));
            }
            let graph = Graph::load_str(&ttl, "turtle").unwrap();

            // Reference: the untripped body, complete and unaffected by the new gate.
            let full = query_json(&graph, q).unwrap();
            assert_eq!(full.matches("\"s\":").count(), rows as usize, "unbudgeted body must be complete");

            let flag = Arc::new(AtomicBool::new(false));
            let budget = QueryBudget::cancelled_by(Arc::clone(&flag));
            let mut chunks = 0usize;
            let err = query_json_stream_with_budget(&graph, q, &budget, |_c| {
                chunks += 1;
                flag.store(true, Ordering::Relaxed); // budget falls due mid-serialize
                ControlFlow::Continue(())
            })
            .unwrap_err();
            // The flag is raised ONLY by the sink, so reaching the error at all proves the
            // trip happened after the pre-serialize gate. The serial branch stops within
            // ~1024 further rows, so it legitimately emits just the one chunk.
            assert!(chunks >= 1, "sink never reached at {} rows — trip is not mid-serialize", rows);
            assert_eq!(err, "query budget exceeded (cancelled)", "at {} rows", rows);

            // A budget that never trips still yields the complete body byte-for-byte —
            // the gate costs nothing on the success path.
            let ok = Arc::new(AtomicBool::new(false));
            let budget = QueryBudget::cancelled_by(ok);
            let mut streamed = String::new();
            query_json_stream_with_budget(&graph, q, &budget, |c| {
                streamed.push_str(&c);
                ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(streamed, full, "untripped budgeted body must equal the unbudgeted one at {} rows", rows);
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.34.2) `query_json_stream_with_budget` hands each chunk to the
    /// sink AS IT IS PRODUCED and its concatenation is byte-identical to the buffered JSON.
    #[test]
    fn query_json_stream_concat_is_byte_identical() {
        let b = QueryBudget::unlimited();
        for q in [
            "SELECT * WHERE { ?s ?p ?o }",
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a }",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:knows ?k } }",
            "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }",
        ] {
            let single = query_json(&g(), q).unwrap();
            let mut streamed = String::new();
            query_json_stream_with_budget(&g(), q, &b, |c| {
                streamed.push_str(&c);
                std::ops::ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(streamed, single, "stream concat mismatch for: {q}");
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.34.1) The prepared streaming entry (no per-execution re-parse —
    /// the HTTP floor path) produces a body byte-identical to BOTH the string-streaming entry
    /// and the buffered `query_json`, over SELECT and ASK, so reusing the algebra parsed by the
    /// server does not change any response byte.
    #[test]
    fn query_json_stream_prepared_matches_string_and_buffered() {
        let b = QueryBudget::unlimited();
        for q in [
            "SELECT * WHERE { ?s ?p ?o }",
            "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:name ?n OPTIONAL { ?s ex:knows ?k } }",
            "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }",
            // 0-row (floor-shaped) result.
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:nope ?o }",
        ] {
            let buffered = query_json(&g(), q).unwrap();
            let prepared = PreparedQuery::parse(q).unwrap();
            let mut streamed = String::new();
            query_json_stream_prepared_with_budget(&g(), &prepared, &b, |c| {
                streamed.push_str(&c);
                std::ops::ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(streamed, buffered, "prepared-stream concat mismatch for: {q}");
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.34.2) ANTI-VACUITY: on a large (>64 KiB) result the sink is
    /// invoked MORE THAN ONCE and the FIRST chunk (carrying the results header) is delivered
    /// BEFORE the last chunk — i.e. bytes are emitted before the result is exhausted. This is
    /// the property the server relies on to flush a first byte before full serialisation.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn query_json_stream_flushes_first_chunk_before_exhaustion() {
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..3000 {
            ttl.push_str(&format!("ex:subject{i} ex:somePredicate \"value-{i}-padding-padding\" .\n"));
        }
        let big = Graph::load_str(&ttl, "turtle").unwrap();
        let q = "SELECT * WHERE { ?s ?p ?o }";
        let single = query_json(&big, q).unwrap();
        assert!(single.len() > 64 * 1024, "test corpus must exceed one chunk");

        let mut chunks: Vec<String> = Vec::new();
        query_json_stream_with_budget(&big, q, &QueryBudget::unlimited(), |c| {
            chunks.push(c);
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
        // Multiple flushes: the first arrived before the last was produced.
        assert!(chunks.len() > 1, "expected an incremental multi-chunk stream, got {}", chunks.len());
        // The first chunk opens the SPARQL-results document (header emitted before all rows).
        assert!(chunks[0].starts_with("{\"head\""), "first chunk must carry the header: {}", &chunks[0][..chunks[0].len().min(40)]);
        // Only the final chunk closes the document.
        assert!(chunks.last().unwrap().ends_with("]}}"), "last chunk must close the document");
        assert!(!chunks[0].ends_with("]}}"), "the header chunk must NOT be the whole document");
        assert_eq!(chunks.concat(), single, "stream concat mismatch");
    }

    /// [OPUS-4.8] (sq-7d3dj.34.2) A sink that returns `Break` (the HTTP client disconnected)
    /// stops the engine early instead of serialising the rest of a large result.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn query_json_stream_break_stops_early() {
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..3000 {
            ttl.push_str(&format!("ex:subject{i} ex:somePredicate \"value-{i}-padding-padding\" .\n"));
        }
        let big = Graph::load_str(&ttl, "turtle").unwrap();
        let mut calls = 0usize;
        query_json_stream_with_budget(&big, "SELECT * WHERE { ?s ?p ?o }", &QueryBudget::unlimited(), |_c| {
            calls += 1;
            std::ops::ControlFlow::Break(()) // bail after the very first chunk
        })
        .unwrap();
        assert_eq!(calls, 1, "engine must stop after the sink breaks on the first chunk");
    }

    /// [OPUS-4.8] roborev 1538 (High) / sq-7d3dj.10: a budget must bound CPU/memory on a
    /// LARGE single-pattern SELECT, not just the final response. A ROW/BYTE cap cannot be
    /// enforced mid-fan-out (fragments are built before any count is known), so it takes
    /// the cooperative serial loop that stops within ~1024 scanned rows. A DEADLINE-only
    /// budget IS allowed to fan out (audit item 6), but the coarse per-par-chunk deadline
    /// re-check bounds the overrun: an already-expired deadline makes every chunk produce
    /// nothing, so the call still returns near-instantly. We build >50k matching rows and
    /// assert (a) the row-cap budget error fires, and (b) an already-expired deadline
    /// returns the timeout error near-instantly (a full 60k-row materialisation would be
    /// far slower) — guarding against an UNBOUNDED fan-out under a deadline.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn budget_bounds_large_single_pattern_json_scan() {
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..60_000u32 {
            ttl.push_str(&format!("ex:s{i} ex:p \"value-{i}-some-padding-text\" .\n"));
        }
        let big = Graph::load_str(&ttl, "turtle").unwrap();
        let q = "SELECT * WHERE { ?s ?p ?o }";
        // Tiny row cap on a 60k scan: must refuse with the budget error.
        let b = QueryBudget { max_rows: Some(5), ..QueryBudget::unlimited() };
        let e = query_json_with_budget(&big, q, &b).unwrap_err();
        assert!(e.contains("query budget exceeded (max-rows)"), "got: {e}");
        // Already-expired deadline-only budget: the multi-core fan-out is now ADMITTED
        // (no row/byte cap to enforce mid-serialize), but the coarse per-par-chunk
        // deadline re-check sees the passed deadline so every chunk produces nothing —
        // the call returns near-instantly without serialising the 60k rows, and the
        // installing thread's post-fan-out gate reports the timeout.
        let b = QueryBudget {
            deadline: Some(std::time::Instant::now() - std::time::Duration::from_millis(1)),
            ..QueryBudget::unlimited()
        };
        let start = std::time::Instant::now();
        let e = query_json_with_budget(&big, q, &b).unwrap_err();
        let elapsed = start.elapsed();
        assert!(e.contains("query budget exceeded (timeout)"), "got: {e}");
        assert!(elapsed < std::time::Duration::from_secs(2), "expired-deadline scan took {elapsed:?} — budget not bounding work");
        // Without a budget the same large scan still works (parallel path) and is complete:
        // one `"s":` binding key per result row.
        let full = query_json(&big, q).unwrap();
        assert_eq!(full.matches("\"s\":").count(), 60_000, "unbudgeted scan must return all rows");
    }

    /// [OPUS-4.8] (sq-7d3dj.10) The multi-core SELECT-JSON serializer runs under a
    /// DEADLINE-ONLY budget (the default HTTP server's shape) but MUST produce bytes
    /// identical to the single-core serial path — same row order, same formatting — and
    /// MUST bound its overrun on an expired deadline. The #1 risk is a nondeterministic
    /// row order or formatting drift, which breaks SPARQL-results-JSON conformance; the
    /// `par_chunks` split + ordered `collect` keep the order deterministic.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parallel_select_json_deadline_only_is_byte_identical_and_bounded() {
        use std::time::{Duration, Instant};

        // A >PAR_THRESHOLD (50k) result so the fan-out actually engages, plus small and
        // empty result sets for the required size coverage.
        let mut big_ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..60_000u32 {
            big_ttl.push_str(&format!("ex:s{} ex:p \"value-{}-padding-padding-padding\" .\n", i, i));
        }
        let big = Graph::load_str(&big_ttl, "turtle").unwrap();
        let small = g();
        let cases: Vec<(&Graph, &str)> = vec![
            (&big, "SELECT * WHERE { ?s ?p ?o }"),                   // large — fan-out engaged
            (&big, "SELECT ?o WHERE { ?s ?p ?o }"),                 // large, projected
            (&big, "SELECT * WHERE { ?s <http://ex/absent> ?o }"),  // empty (unsatisfiable predicate)
            (&small, "SELECT * WHERE { ?s ?p ?o }"),                // small — below the fan-out threshold
        ];

        for (graph, q) in cases {
            // Single-core reference: a generous ROW cap forces the cooperative SERIAL
            // loop (parallel_json_fanout → None) yet never trips (cap ≫ result size).
            let serial_b = QueryBudget { max_rows: Some(10_000_000), ..QueryBudget::unlimited() };
            let serial = query_json_with_budget(graph, q, &serial_b).unwrap();

            // Multi-core: a DEADLINE-only budget far in the future admits the fan-out.
            let par_b = QueryBudget {
                deadline: Some(Instant::now() + Duration::from_secs(3600)),
                ..QueryBudget::unlimited()
            };
            let parallel = query_json_with_budget(graph, q, &par_b).unwrap();
            assert_eq!(parallel, serial, "multi-core deadline-only body must be byte-identical to single-core for: {}", q);

            // The chunked (streamed) form the HTTP server uses must concatenate to the
            // same bytes — Content-Length is derived from these bytes, so this pins it.
            let chunks = query_json_chunks_with_budget(graph, q, &par_b).unwrap();
            assert_eq!(chunks.concat(), serial, "chunked deadline-only concat must equal single-core for: {}", q);

            // And the fully-unbudgeted parallel path agrees too.
            assert_eq!(query_json(graph, q).unwrap(), serial, "unbudgeted body must equal single-core for: {}", q);
        }

        // Bounded overrun: a huge result under an ALREADY-EXPIRED deadline-only budget
        // must NOT serialise the whole thing — every par-chunk re-check trips, so the
        // call returns the timeout error near-instantly rather than 60k serialised rows.
        let expired = QueryBudget {
            deadline: Some(Instant::now() - Duration::from_millis(1)),
            ..QueryBudget::unlimited()
        };
        let start = Instant::now();
        let e = query_json_with_budget(&big, "SELECT * WHERE { ?s ?p ?o }", &expired).unwrap_err();
        let elapsed = start.elapsed();
        assert!(e.contains("query budget exceeded (timeout)"), "got: {}", e);
        assert!(elapsed < Duration::from_secs(2), "expired-deadline fan-out took {:?} — overrun not bounded", elapsed);
    }

    #[test]
    fn budget_unlimited_matches_query() {
        let q = "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:age ?age }";
        let plain = query(&g(), q).unwrap();
        let budgeted = query_with_budget(&g(), q, &QueryBudget::unlimited()).unwrap();
        assert_eq!(plain.len(), budgeted.len());
        assert_eq!(
            query_json(&g(), q).unwrap(),
            query_json_with_budget(&g(), q, &QueryBudget::unlimited()).unwrap()
        );
    }

    #[test]
    fn budget_max_rows_refuses_not_truncates() {
        // 8 triples; max_rows 3 must REFUSE (error), never return a truncated result.
        let b = QueryBudget { max_rows: Some(3), ..QueryBudget::unlimited() };
        let e = query_with_budget(&g(), "SELECT * WHERE { ?s ?p ?o }", &b).map(|r| r.len()).unwrap_err();
        assert!(e.contains("query budget exceeded (max-rows)"), "got: {e}");
        let e = query_json_with_budget(&g(), "SELECT * WHERE { ?s ?p ?o }", &b).unwrap_err();
        assert!(e.contains("query budget exceeded (max-rows)"), "got: {e}");
        // A generous row budget changes nothing.
        let b = QueryBudget { max_rows: Some(1000), ..QueryBudget::unlimited() };
        assert_eq!(query_with_budget(&g(), "SELECT * WHERE { ?s ?p ?o }", &b).unwrap().len(), 8);
    }

    /// [OPUS-4.8] (sq-s5is) The byte-accounted cap prices ROW WIDTH — the dimension the
    /// row cap misses. Two queries over the same graph have the SAME row count but
    /// different widths; a byte cap set between their two working-set sizes admits the
    /// narrow one and refuses the wide one, while a pure row cap (same row count) could
    /// not tell them apart.
    #[test]
    fn budget_max_bytes_prices_row_width() {
        // 9 ex:p triples → a 1-pattern BGP yields the same 9 rows whether read as
        // `?o` (≈1 binding-id column) or `?s ?p ?o` (3 columns). With ~4 bytes/id the
        // wide working set is ~3× the bytes of the narrow one at identical row count.
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..9 {
            ttl.push_str(&format!("ex:s{i} ex:p ex:o{i} .\n"));
        }
        let wide = Graph::load_str(&ttl, "turtle").unwrap();
        // A byte budget that comfortably fits the 3-wide BGP must NOT refuse it…
        let generous = QueryBudget { max_bytes: Some(1 << 20), ..QueryBudget::unlimited() };
        assert_eq!(query_with_budget(&wide, "SELECT * WHERE { ?s ?p ?o }", &generous).unwrap().len(), 9);
        // …and a byte budget far below any 9-row working set MUST refuse with max-bytes
        // (not max-rows: the row cap is unset here, so width is the only thing tripping).
        let tight = QueryBudget { max_bytes: Some(4), ..QueryBudget::unlimited() };
        let e = query_with_budget(&wide, "SELECT * WHERE { ?s ?p ?o }", &tight).unwrap_err();
        assert!(e.contains("query budget exceeded (max-bytes)"), "got: {e}");
        let e = query_json_with_budget(&wide, "SELECT * WHERE { ?s ?p ?o }", &tight).unwrap_err();
        assert!(e.contains("query budget exceeded (max-bytes)"), "got: {e}");
    }

    /// [OPUS-4.8] (sq-s5is) The byte cap also prices query-COMPUTED literals (BIND /
    /// CONCAT scratch interned into the local vocab) — the NON-row dimension the row
    /// cap misses. A single-row result whose one computed value is a huge string trips
    /// the byte cap even though the row count is 1.
    #[test]
    fn budget_max_bytes_prices_computed_literals() {
        // One row, one BIND that builds a large string; the row cap (=10) is generous,
        // but the computed literal's bytes blow a tight byte cap. CONCAT of two long string
        // literals needs no feature-gated builtin, so this holds in BOTH feature states.
        let q = "SELECT ?big WHERE { BIND(CONCAT('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
                 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb') AS ?big) }";
        let b = QueryBudget { max_rows: Some(10), max_bytes: Some(16), ..QueryBudget::unlimited() };
        let e = query_with_budget(&g(), q, &b).unwrap_err();
        assert!(e.contains("query budget exceeded (max-bytes)"), "got: {e}");
        // A generous byte cap returns the single row unmodified.
        let ok = QueryBudget { max_rows: Some(10), max_bytes: Some(1 << 20), ..QueryBudget::unlimited() };
        assert_eq!(query_with_budget(&g(), q, &ok).unwrap().len(), 1);
    }

    /// [OPUS-4.8] (sq-s5is) An unset byte cap (the default) is a true no-op: results are
    /// byte-identical to the unbudgeted query, and a query that EXCEEDS a generous row
    /// cap but would fit a non-existent byte cap still errors on the row cap alone.
    #[test]
    fn budget_max_bytes_unset_is_noop() {
        let q = "SELECT * WHERE { ?s ?p ?o }";
        let only_rows = QueryBudget { max_rows: Some(100), ..QueryBudget::unlimited() };
        assert_eq!(query_with_budget(&g(), q, &only_rows).unwrap().len(), 8);
        assert_eq!(query_with_budget(&g(), q, &QueryBudget::unlimited()).unwrap().len(), 8);
        assert_eq!(
            query_json(&g(), q).unwrap(),
            query_json_with_budget(&g(), q, &QueryBudget::unlimited()).unwrap()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn budget_deadline_times_out() {
        // A deadline already in the past trips the first cooperative check.
        let b = QueryBudget {
            deadline: Some(std::time::Instant::now() - std::time::Duration::from_millis(1)),
            ..QueryBudget::unlimited()
        };
        let q = "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:age ?age }";
        let e = query_with_budget(&g(), q, &b).map(|r| r.len()).unwrap_err();
        assert!(e.contains("query budget exceeded (timeout)"), "got: {e}");
        // …and the budget never leaks into the next (unbudgeted) query on this thread.
        assert_eq!(query(&g(), q).unwrap().len(), 2);
    }

    // Differential: the engine's join machinery (merge/hash/greedy ordering)
    // must agree with a brute-force nested-loop evaluator over a random graph,
    // for a chain query and a triangle (cyclic) query.
    #[test]
    fn differential_vs_naive() {
        // Deterministic pseudo-random graph (seeded LCG, no external randomness).
        let mut seed: u64 = 0x1234_5678;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let n_nodes = 40u32;
        let mut edges: Vec<(u32, u32)> = Vec::new();
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for _ in 0..200 {
            let a = next() % n_nodes;
            let b = next() % n_nodes;
            edges.push((a, b));
            ttl.push_str(&format!("ex:n{a} ex:e ex:n{b} .\n"));
        }
        edges.sort_unstable();
        edges.dedup();

        let graph = Graph::load_str(&ttl, "turtle").unwrap();

        // Chain: ?a e ?b . ?b e ?c
        let chain = naive_count_chain(&edges);
        let q = query(&graph, "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c }").unwrap();
        assert_eq!(q.len(), chain, "chain join count mismatch");

        // Triangle: ?a e ?b . ?b e ?c . ?c e ?a  (cyclic)
        let tri = naive_count_triangle(&edges);
        let q = query(
            &graph,
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?a }",
        )
        .unwrap();
        assert_eq!(q.len(), tri, "triangle join count mismatch");
    }

    fn naive_count_chain(e: &[(u32, u32)]) -> usize {
        let mut c = 0;
        for &(_, b) in e {
            for &(b2, _) in e {
                if b == b2 {
                    c += 1;
                }
            }
        }
        c
    }

    fn naive_count_triangle(e: &[(u32, u32)]) -> usize {
        use std::collections::HashSet;
        let set: HashSet<(u32, u32)> = e.iter().copied().collect();
        let mut c = 0;
        for &(a, b) in e {
            for &(b2, cc) in e {
                if b == b2 && set.contains(&(cc, a)) {
                    c += 1;
                }
            }
        }
        c
    }

    // ---- extension-function registry (SPARQL 17.6) ----------------------------

    /// `ex:double(?n)` — a 1-arg numeric extension used by the registry tests.
    fn doubling_registry() -> FunctionRegistry {
        let mut reg = FunctionRegistry::new();
        reg.register("http://ex/fn#double", |args: &[Term]| {
            let [Term::Literal(l)] = args else {
                return Err(format!("double() expects 1 literal argument, got {}", args.len()));
            };
            let n: i64 = l.value().parse().map_err(|e| format!("double(): {e}"))?;
            Ok(Term::Literal(oxrdf::Literal::from(n * 2)))
        });
        reg
    }

    #[test]
    fn function_registry_dispatch_bind_and_filter() {
        let reg = doubling_registry();
        // BIND: the extension result is a first-class computed value.
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?d WHERE { ?s ex:age ?a . BIND(fn:double(?a) AS ?d) } ORDER BY ?d",
            &reg,
        )
        .unwrap();
        let ds: Vec<String> = r.rows.iter().map(|row| row[0].as_ref().unwrap().to_string()).collect();
        assert!(ds[0].contains("\"50\"") && ds[1].contains("\"60\"") && ds[2].contains("\"70\""));
        // FILTER: the result participates in ordinary value comparison.
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s WHERE { ?s ex:age ?a . FILTER(fn:double(?a) > 65) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 1); // carol (35 -> 70)
        // The budget variant threads the registry too.
        let r = query_with_functions_and_budget(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s WHERE { ?s ex:age ?a . FILTER(fn:double(?a) > 65) }",
            &reg,
            &QueryBudget::unlimited(),
        )
        .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn function_registry_unknown_iri_is_hard_error() {
        let reg = doubling_registry();
        let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(<http://ex/fn#nope>(?a) > 0) }";
        // Registry installed but the IRI is not in it: the same hard error …
        let err = query_with_functions(&g(), q, &reg).map(|r| r.len()).unwrap_err();
        assert!(err.contains("unsupported SPARQL function"), "got: {err}");
        // … as the registry-free entry point raises (pre-registry behaviour intact).
        let err = query(&g(), q).map(|r| r.len()).unwrap_err();
        assert!(err.contains("unsupported SPARQL function"), "got: {err}");
        // And a registered IRI without a registry stays a hard error too.
        let err = query(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(fn:double(?a) > 0) }",
        )
        .map(|r| r.len())
        .unwrap_err();
        assert!(err.contains("unsupported SPARQL function"), "got: {err}");
    }

    #[test]
    fn function_registry_errors_are_expression_errors() {
        let reg = doubling_registry();
        // Arity error (2 args -> the extension returns Err): a per-row expression
        // error, so the BIND leaves ?d unbound — the query itself succeeds.
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s ?d WHERE { ?s ex:age ?a . BIND(fn:double(?a, ?a) AS ?d) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        assert!(r.rows.iter().all(|row| row[1].is_none()), "errored BIND must be unbound");
        // In a FILTER the same error excludes every row (error -> EBV false).
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s WHERE { ?s ex:age ?a . FILTER(fn:double(?a, ?a) > 0) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 0);
        // A value error inside the extension (non-numeric lexical) behaves the same:
        // names error out per-row, the query still succeeds with unbound ?d.
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s ?d WHERE { ?s ex:name ?n . BIND(fn:double(?n) AS ?d) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        assert!(r.rows.iter().all(|row| row[1].is_none()));
        // An UNBOUND argument is an expression error before the extension is called.
        let r = query_with_functions(
            &g(),
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?s ?d WHERE { ?s ex:age ?a . OPTIONAL { ?s ex:nope ?k } BIND(fn:double(?k) AS ?d) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        assert!(r.rows.iter().all(|row| row[1].is_none()));
    }

    #[test]
    fn function_registry_xsd_cast_precedence_and_uninstall() {
        // A (pathological) registration under an XSD cast IRI must NOT shadow the
        // builtin constructor cast — the cast check runs first.
        let mut reg = doubling_registry();
        reg.register("http://www.w3.org/2001/XMLSchema#integer", |_args: &[Term]| {
            Ok(Term::Literal(oxrdf::Literal::from(999)))
        });
        let r = query_with_functions(
            &g(),
            "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT (xsd:integer(\"7\") AS ?i) WHERE {}",
            &reg,
        )
        .unwrap();
        assert_eq!(r.rows[0][0].as_ref().unwrap().to_string(), "\"7\"^^<http://www.w3.org/2001/XMLSchema#integer>");
        // The registry is uninstalled when the entry point returns: the same custom
        // IRI is a hard error again on the plain entry point afterwards.
        let q = "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . FILTER(fn:double(?a) > 0) }";
        assert!(query_with_functions(&g(), q, &reg).is_ok());
        assert!(query(&g(), q).is_err());
    }

    /// The registry must reach expressions evaluated INSIDE rayon workers: the
    /// parallel FILTER / BIND branches engage at `PAR_THRESHOLD` (50k) rows, where
    /// the installing thread's thread-local is invisible without re-installation.
    #[test]
    #[cfg(feature = "parallel")]
    fn function_registry_reaches_parallel_workers() {
        let n: i64 = 50_001;
        let mut ttl = String::with_capacity(n as usize * 32);
        ttl.push_str("@prefix ex: <http://ex/> .\n");
        for i in 0..n {
            ttl.push_str(&format!("ex:n{i} ex:v {i} .\n"));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let reg = doubling_registry();
        // Parallel FILTER: double(?v) > 2*(n-1) - 1 keeps exactly the last row.
        let r = query_with_functions(
            &graph,
            &format!(
                "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
                 SELECT ?s WHERE {{ ?s ex:v ?v . FILTER(fn:double(?v) > {}) }}",
                2 * (n - 1) - 1
            ),
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        // Parallel BIND: every row gets a bound ?d.
        let r = query_with_functions(
            &graph,
            "PREFIX fn: <http://ex/fn#> PREFIX ex: <http://ex/> \
             SELECT ?d WHERE { ?s ex:v ?v . BIND(fn:double(?v) AS ?d) }",
            &reg,
        )
        .unwrap();
        assert_eq!(r.len(), n as usize);
        assert!(r.rows.iter().all(|row| row[0].is_some()));
    }

    // [OPUS-4.8] (sq-bif) The public `count` / `count_prepared` entry points — the
    // id-level solution counter that the server's budgeted ASK path leans on. The
    // local `count` helper above routes through `query().len()`; these exercise the
    // REAL `crate::count` (no result materialisation) and pin its agreement with the
    // materialised count plus its form-rejection error path.

    /// `count` must agree with `query(..).len()` across the modifier shapes that
    /// change the cardinality (DISTINCT collapses duplicates, LIMIT caps, the
    /// projection of a constant). A divergence here means the count fast path and
    /// the materialising path disagree — a real correctness bug.
    #[test]
    fn count_agrees_with_materialized_len() {
        let g = g();
        let cases = [
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a }",
            "PREFIX ex: <http://ex/> SELECT DISTINCT ?a WHERE { ?s ex:knows ?b ; ex:age ?a }",
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } LIMIT 2",
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } ORDER BY ?a LIMIT 1",
            // Empty result: an unsatisfiable pattern.
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:nope ?a }",
            // A cross/star join.
            "PREFIX ex: <http://ex/> SELECT ?a ?b WHERE { ?a ex:knows ?b . ?b ex:age ?x }",
        ];
        for q in cases {
            assert_eq!(
                crate::count(&g, q).unwrap(),
                query(&g, q).unwrap().len(),
                "count disagrees with materialised len for {q:?}"
            );
        }
    }

    /// An ASK counts its single unit row: 1 when satisfiable, 0 otherwise — the
    /// `usize::from(bool)` arm of `count_prepared_with_budget`.
    #[test]
    fn count_of_ask_is_zero_or_one() {
        let g = g();
        let cnt = |q: &str| crate::count(&g, q).unwrap();
        assert_eq!(cnt("PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }"), 1);
        assert_eq!(cnt("PREFIX ex: <http://ex/> ASK { ?s ex:nope ?a }"), 0);
    }

    /// `count` (and `query_json`) only support SELECT / ASK; the graph-valued
    /// forms (CONSTRUCT / DESCRIBE) are an explicit error, not a panic or a wrong
    /// number. This guards the `_ => Err(...)` arms that have no other coverage.
    #[test]
    fn count_and_query_json_reject_graph_forms() {
        let g = g();
        let construct = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:a ?a } WHERE { ?s ex:age ?a }";
        let describe = "DESCRIBE <http://ex/alice>";
        let rejects = |e: String, q: &str| assert!(e.contains("only SELECT and ASK"), "{q:?}: {e}");
        for q in [construct, describe] {
            rejects(crate::count(&g, q).unwrap_err(), q);
            rejects(query_json(&g, q).unwrap_err(), q);
        }
        // count_prepared takes the same path over a pre-parsed query.
        let prepared = PreparedQuery::parse(construct).unwrap();
        assert!(count_prepared(&g, &prepared).is_err());
    }

    /// `query_json` of an ASK emits the SPARQL-results-JSON boolean document
    /// (`{"head":{},"boolean":…}`), distinct from the SELECT `bindings` document.
    #[test]
    fn query_json_ask_emits_boolean_document() {
        let g = g();
        assert_eq!(
            query_json(&g, "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }").unwrap(),
            r#"{"head":{},"boolean":true}"#
        );
        assert_eq!(
            query_json(&g, "PREFIX ex: <http://ex/> ASK { ?s ex:nope ?a }").unwrap(),
            r#"{"head":{},"boolean":false}"#
        );
    }

    /// The `PreparedQuery` round-trips: parse / `From<Query>` / `FromStr` /
    /// `into_query` all produce the SAME execution. A query built programmatically
    /// from algebra (the sparq-rsp / rewrite seam) must evaluate identically to the
    /// parsed string form.
    #[test]
    fn prepared_query_construction_round_trips() {
        let g = g();
        let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } ORDER BY ?s";

        let from_parse = PreparedQuery::parse(q).unwrap();
        // FromStr is the same as parse.
        let from_str: PreparedQuery = q.parse().unwrap();
        // From<Query> wraps already-parsed algebra; into_query unwraps it back.
        let algebra = from_parse.clone().into_query();
        let from_algebra = PreparedQuery::from(algebra);

        let want = query_prepared(&g, &from_parse).unwrap();
        for p in [&from_str, &from_algebra] {
            let got = query_prepared(&g, p).unwrap();
            assert_eq!(got.vars, want.vars);
            assert_eq!(got.rows, want.rows);
        }
        // `query()` exposes the wrapped algebra without consuming it.
        assert!(matches!(from_parse.query(), Query::Select { .. }));
        // A malformed query is a parse error, not a panic.
        assert!(PreparedQuery::parse("SELECT ?s WHERE { ?s ?p").is_err());
        assert!("not a query".parse::<PreparedQuery>().is_err());
    }

    /// `QueryResult::len` / `is_empty` track the row count exactly — including the
    /// degenerate one-unbound-row case a no-match aggregate produces (NOT empty),
    /// which is a classic off-by-one trap.
    #[test]
    fn query_result_len_and_is_empty() {
        let g = g();
        let run = |q: &str| query(&g, q).unwrap();

        let some = run("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a }");
        assert_eq!(some.len(), 3);
        assert!(!some.is_empty());

        let none = run("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:nope ?a }");
        assert_eq!(none.len(), 0);
        assert!(none.is_empty());

        // A whole-dataset aggregate over an empty match is ONE row (the unbound
        // group), so the result is NOT empty even though the pattern matched nothing.
        let agg = run("PREFIX ex: <http://ex/> SELECT (COUNT(*) AS ?c) WHERE { ?s ex:nope ?a }");
        assert_eq!(agg.len(), 1);
        assert!(!agg.is_empty());
    }

    /// [OPUS-4.8] (sq-bif) The `FunctionRegistry` introspection accessors
    /// (`new`/`is_empty`/`len`/`get`/`iris`) that the Service-Description path
    /// advertises from, plus the documented "register REPLACES a duplicate IRI"
    /// behaviour — none of which had a direct assertion.
    #[test]
    fn function_registry_accessors_and_replace() {
        let mut reg = FunctionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("http://ex/fn#id").is_none());
        assert_eq!(reg.iris().count(), 0);

        reg.register("http://ex/fn#a", |args: &[Term]| Ok(args[0].clone()));
        reg.register("http://ex/fn#b", |args: &[Term]| Ok(args[0].clone()));
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 2);
        assert!(reg.get("http://ex/fn#a").is_some());
        let mut iris: Vec<&str> = reg.iris().collect();
        iris.sort_unstable();
        assert_eq!(iris, ["http://ex/fn#a", "http://ex/fn#b"]);

        // Re-registering the SAME IRI replaces the closure (and never grows len).
        let replaced = Term::NamedNode(oxrdf::NamedNode::new("http://ex/replaced").unwrap());
        let r2 = replaced.clone();
        reg.register("http://ex/fn#a", move |_: &[Term]| Ok(r2.clone()));
        assert_eq!(reg.len(), 2, "duplicate IRI must replace, not add");
        let q = "SELECT ?r WHERE { BIND(<http://ex/fn#a>(<http://ex/x>) AS ?r) }";
        let r = query_with_functions(&g(), q, &reg).unwrap();
        assert_eq!(
            r.rows[0][0].as_ref().unwrap(),
            &replaced,
            "the replacement closure, not the original, must run"
        );
    }
}

// [FABLE-5] (sq-lsp7k.10) Named parameterized SPARQL templates — the shared definition +
// typed-JSON-binding layer under the server's REST template store and the MCP
// `template_invoke` tool. NON-DEFAULT `templates` feature (builds on `params`); when off,
// zero of this code compiles. Declared at the END of this file deliberately: inserting a
// compiled-out declaration mid-file would shift `line!()`/`Location` values of the
// always-compiled code below it and move feature-OFF wasm bytes for no reason (the
// vectorized-feature-off gate's drift class).
#[cfg(feature = "templates")]
#[cfg_attr(docsrs, doc(cfg(feature = "templates")))]
pub mod templates;

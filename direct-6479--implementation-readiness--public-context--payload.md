# Public source context supplement for issue6479

Source-only context at f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464. No execution evidence is included. Inspect the exact per-function nesting: ordinary prepared/cache paths put BASE inside the budget; ANALYZE paths install BASE before the budget and trace. A replacement must preserve each existing order.

## Extension closure and registry contract

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L329-L387

```rust
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
```

## Registry synchronous closure

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L401-L404

```rust
pub fn with_functions<T>(fns: &FunctionRegistry, f: impl FnOnce() -> T) -> T {
    let _guard = exec::functions::install(fns);
    f()
}
```

## Complete function registry TLS module

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L1923-L1981

```rust
pub(crate) mod functions {
    use crate::FunctionRegistry;
    use std::cell::RefCell;
    use std::sync::Arc;

    thread_local! {
        static ACTIVE: RefCell<Option<Arc<FunctionRegistry>>> = const { RefCell::new(None) };
    }

    /// Uninstalls the registry when the installing entry point returns (also on
    /// error/unwind, so a poisoned thread never leaks a stale registry).
    pub(crate) struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.borrow_mut().take());
        }
    }

    pub(crate) fn install(fns: &FunctionRegistry) -> Guard {
        ACTIVE.with(|a| *a.borrow_mut() = Some(Arc::new(fns.clone())));
        Guard
    }

    /// Snapshot of the installed registry for the rayon-parallel branches
    /// (`None` — the overwhelmingly common case — makes [`worker_install`] free).
    // [OPUS-4.8] Only the `parallel`-gated worker branches snapshot the registry (see
    // `worker_install`, gated the same way); match the sibling `limits::snapshot` so the
    // `-D warnings` clippy gate stays clean in no-parallel/wasm builds.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn snapshot() -> Option<Arc<FunctionRegistry>> {
        ACTIVE.with(|a| a.borrow().clone())
    }

    /// Scoped re-install of a snapshot inside a rayon worker item. Restores the
    /// PREVIOUS thread-local value on drop: rayon runs some items on the
    /// installing thread itself, whose registry must survive the item.
    pub(crate) struct WorkerGuard(Option<Option<Arc<FunctionRegistry>>>);
    impl Drop for WorkerGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                ACTIVE.with(|a| *a.borrow_mut() = prev);
            }
        }
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn worker_install(snap: &Option<Arc<FunctionRegistry>>) -> WorkerGuard {
        match snap {
            None => WorkerGuard(None),
            Some(fns) => WorkerGuard(Some(ACTIVE.with(|a| a.borrow_mut().replace(fns.clone())))),
        }
    }

    /// The extension function registered for `iri`, if a registry is installed
    /// and contains it.
    pub(crate) fn lookup(iri: &str) -> Option<crate::ExtFn> {
        ACTIVE.with(|a| a.borrow().as_ref().and_then(|fns| fns.get(iri).cloned()))
    }
}
```

## Synchronous custom-function invocation after lookup returns

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L14515-L14538

```rust
            if vals.len() == 1 {
                if let Some(out) = eval_cast(nn.as_str(), &vals[0]) {
                    return Ok(out);
                }
            }
            if let Some(f) = functions::lookup(nn.as_str()) {
                // Arguments are materialised as concrete RDF terms; an unbound or
                // errored argument is an expression ERROR (row filtered / BIND
                // unbound), exactly like the builtins. The extension returning
                // `Err` (wrong arity, bad lexical, …) is the same expression
                // error — per-row, never a hard query error.
                let mut terms = Vec::with_capacity(vals.len());
                for v in &vals {
                    match value_as_term(v) {
                        Some(t) => terms.push(t),
                        None => return Ok(Value::Error),
                    }
                }
                return Ok(match f(&terms) {
                    Ok(t) => Value::Term(t),
                    Err(_) => Value::Error,
                });
            }
            return Err(format!("unsupported SPARQL function: Custom({})", nn.as_str()));
```

## PreparedQuery owned query and parse feature branches

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L836-L869

```rust
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
```

## Owned query algebra and dataset/base accessors

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/vendor/spargebra/src/query.rs#L24-L110

```rust
pub enum Query {
    /// [SELECT](https://www.w3.org/TR/sparql11-query/#select).
    Select {
        /// The [query dataset specification](https://www.w3.org/TR/sparql11-query/#specifyingDataset).
        dataset: Option<QueryDataset>,
        /// The query selection graph pattern.
        pattern: GraphPattern,
        /// The query base IRI.
        base_iri: Option<Iri<String>>,
    },
    /// [CONSTRUCT](https://www.w3.org/TR/sparql11-query/#construct).
    Construct {
        /// The query construction template.
        template: Vec<TriplePattern>,
        /// The [query dataset specification](https://www.w3.org/TR/sparql11-query/#specifyingDataset).
        dataset: Option<QueryDataset>,
        /// The query selection graph pattern.
        pattern: GraphPattern,
        /// The query base IRI.
        base_iri: Option<Iri<String>>,
    },
    /// [DESCRIBE](https://www.w3.org/TR/sparql11-query/#describe).
    Describe {
        /// The [query dataset specification](https://www.w3.org/TR/sparql11-query/#specifyingDataset).
        dataset: Option<QueryDataset>,
        /// The query selection graph pattern.
        pattern: GraphPattern,
        /// The query base IRI.
        base_iri: Option<Iri<String>>,
    },
    /// [ASK](https://www.w3.org/TR/sparql11-query/#ask).
    Ask {
        /// The [query dataset specification](https://www.w3.org/TR/sparql11-query/#specifyingDataset).
        dataset: Option<QueryDataset>,
        /// The query selection graph pattern.
        pattern: GraphPattern,
        /// The query base IRI.
        base_iri: Option<Iri<String>>,
    },
}

impl Query {
    /// Parses a SPARQL query with an optional base IRI to resolve relative IRIs in the query.
    #[deprecated(
        note = "Use `SparqlParser::new().parse_query` instead",
        since = "0.4.0"
    )]
    pub fn parse(query: &str, base_iri: Option<&str>) -> Result<Self, SparqlSyntaxError> {
        let mut parser = SparqlParser::new();
        if let Some(base_iri) = base_iri {
            parser = parser
                .with_base_iri(base_iri)
                .map_err(SparqlSyntaxError::from_bad_base_iri)?;
        }
        parser.parse_query(query)
    }

    #[inline]
    pub fn dataset(&self) -> Option<&QueryDataset> {
        match self {
            Query::Select { dataset, .. }
            | Query::Construct { dataset, .. }
            | Query::Describe { dataset, .. }
            | Query::Ask { dataset, .. } => dataset.as_ref(),
        }
    }

    #[inline]
    pub fn dataset_mut(&mut self) -> Option<&mut QueryDataset> {
        match self {
            Query::Select { dataset, .. }
            | Query::Construct { dataset, .. }
            | Query::Describe { dataset, .. }
            | Query::Ask { dataset, .. } => dataset.as_mut(),
        }
    }

    #[inline]
    pub fn base_iri(&self) -> Option<&Iri<String>> {
        match self {
            Query::Select { base_iri, .. }
            | Query::Construct { base_iri, .. }
            | Query::Describe { base_iri, .. }
            | Query::Ask { base_iri, .. } => base_iri.as_ref(),
        }
    }

```

## Active dataset and view scoping

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L811-L829

```rust

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
```

## Complete production dataset reconstruction helpers

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/dataset.rs#L13-L96

```rust
use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::Dict;
use sparq_core::store::Pattern as IdPattern;
use sparq_core::Graph;
use spargebra::algebra::QueryDataset;

pub(crate) type TripleTerms = [Term; 3];
pub(crate) type TripleSet = FxHashSet<TripleTerms>;

/// The triples of one graph as ground term-triples (decoded from its dictionary).
pub(crate) fn decode_triples(g: &Graph) -> TripleSet {
    let pat: IdPattern = [None, None, None];
    let scan = g.store.scan(&pat);
    scan.rows
        .iter()
        .map(|r| {
            let t = scan.to_spo(r);
            [g.dict.term(t[0]), g.dict.term(t[1]), g.dict.term(t[2])]
        })
        .collect()
}

/// Rebuild an immutable graph from a term-triple set (fresh dictionary + permutation indexes).
pub(crate) fn build(triples: &TripleSet) -> Graph {
    let mut dict = Dict::new();
    let mut ids = Vec::with_capacity(triples.len());
    for [s, p, o] in triples {
        ids.push([dict.intern(s), dict.intern(p), dict.intern(o)]);
    }
    Graph::from_parts(dict, ids)
}

pub(crate) fn empty_graph() -> Graph {
    Graph::from_parts(Dict::new(), Vec::new())
}

/// The store's named graph called `name`, if it holds one.
fn find_named<'a>(graph: &'a Graph, name: &NamedNode) -> Option<&'a Graph> {
    graph.named.iter().find_map(|(g, sub)| match g {
        Term::NamedNode(n) if n == name => Some(sub),
        _ => None,
    })
}

/// The ACTIVE dataset of a query that carries a dataset clause: the default
/// graph is the merge of the `FROM` graphs and the named graphs are exactly the
/// `FROM NAMED` ones — the store's own default/named graphs do NOT leak in. A
/// graph name the store does not hold denotes the EMPTY graph (it still exists
/// in the active dataset, so `GRAPH <absent> {}` keeps its unit-row semantics).
///
/// Under an installed dataset view (L1) the clause first INTERSECTS with the
/// view: a non-visible graph is treated exactly like an absent one (it
/// contributes nothing to `FROM` and denotes the empty graph for `FROM NAMED`),
/// so the restriction composes and never widens — and a non-visible graph stays
/// indistinguishable from an absent one.
///
/// Only called when the query has a dataset clause; the common no-clause path
/// never pays for this.
pub(crate) fn build_active(graph: &Graph, ds: &QueryDataset) -> Graph {
    let visible = |n: &NamedNode| crate::exec::view::allows(&Term::NamedNode(n.clone()));
    let mut default = TripleSet::default();
    for n in &ds.default {
        if !visible(n) {
            continue; // view: non-visible ≡ absent
        }
        if let Some(g) = find_named(graph, n) {
            default.extend(decode_triples(g));
        }
    }
    let mut out = build(&default);
    for n in ds.named.as_deref().unwrap_or_default() {
        let name = Term::NamedNode(n.clone());
        if out.named.iter().any(|(g, _)| *g == name) {
            continue; // a repeated FROM NAMED still names ONE graph
        }
        let g = match find_named(graph, n).filter(|_| visible(n)) {
            Some(g) => build(&decode_triples(g)),
            None => empty_graph(),
        };
        out.named.push((name, g));
    }
    out
}
```

## Complete dataset-view TLS module

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L2332-L2440

```rust
pub(crate) mod view {
    use crate::{DatasetView, DefaultGraphMode};
    use oxrdf::Term;
    use rustc_hash::FxHashSet;
    use std::cell::RefCell;
    use std::sync::Arc;

    /// The installed view, plus the "inside GRAPH" suspend flag:
    /// `eval_graph_named` swaps evaluation to the named sub-`Graph`, whose inner
    /// patterns must NOT be empty-defaulted (only the TOP-LEVEL graph scope is).
    #[derive(Clone, Default)]
    pub(crate) struct State {
        named: Option<Arc<FxHashSet<Term>>>,
        default_empty: bool,
        suspended: bool,
    }

    thread_local! {
        static ACTIVE: RefCell<State> = RefCell::new(State::default());
    }

    /// Restores the pre-install state when the installing entry point returns
    /// (also on error/unwind, so a poisoned thread never leaks a stale view).
    pub(crate) struct Guard(State);
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|a| *a.borrow_mut() = std::mem::take(&mut self.0));
        }
    }

    pub(crate) fn install(v: &DatasetView) -> Guard {
        let new = State {
            named: Some(Arc::clone(&v.named)),
            default_empty: matches!(v.default, DefaultGraphMode::Empty),
            suspended: false,
        };
        Guard(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), new)))
    }

    /// Fully suspends the view (named filter AND empty default) for a scope —
    /// used by the entry points once `dataset::build_active` has folded the view
    /// into a dataset-clause ACTIVE graph: the restriction is already applied,
    /// and re-filtering would make a non-visible FROM NAMED graph behave
    /// differently from an absent one (both must be the EMPTY active graph).
    pub(crate) fn suspend_all() -> Guard {
        Guard(ACTIVE.with(|a| std::mem::take(&mut *a.borrow_mut())))
    }

    /// RAII suspension of the empty-default short-circuit only, for GRAPH scope
    /// (the named-graph visibility filter stays active). Restores the previous
    /// flag on drop, so nested scopes compose.
    pub(crate) struct GraphScope(bool);
    impl Drop for GraphScope {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.borrow_mut().suspended = self.0);
        }
    }

    pub(crate) fn enter_graph() -> GraphScope {
        GraphScope(ACTIVE.with(|a| std::mem::replace(&mut a.borrow_mut().suspended, true)))
    }

    /// `true` when `name` is a visible named graph under the installed view
    /// (always true with no view installed).
    #[inline]
    pub(crate) fn allows(name: &Term) -> bool {
        ACTIVE.with(|a| a.borrow().named.as_ref().is_none_or(|s| s.contains(name)))
    }

    /// `true` when the view's default graph is EMPTY at the current scope —
    /// false with no view, under `StoreDefault`, or inside a GRAPH pattern.
    #[inline]
    pub(crate) fn default_is_empty() -> bool {
        ACTIVE.with(|a| {
            let s = a.borrow();
            s.default_empty && !s.suspended
        })
    }

    /// Snapshot of the installed view for the rayon-parallel expression branches
    /// (`None` — no view, the common case — makes [`worker_install`] free).
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn snapshot() -> Option<State> {
        ACTIVE.with(|a| {
            let s = a.borrow();
            (s.named.is_some() || s.default_empty).then(|| s.clone())
        })
    }

    /// Scoped re-install of a snapshot inside a rayon worker item. Restores the
    /// PREVIOUS thread-local value on drop: rayon runs some items on the
    /// installing thread itself, whose view must survive the item.
    pub(crate) struct WorkerGuard(Option<State>);
    impl Drop for WorkerGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                ACTIVE.with(|a| *a.borrow_mut() = prev);
            }
        }
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn worker_install(snap: &Option<State>) -> WorkerGuard {
        match snap {
            None => WorkerGuard(None),
            Some(s) => WorkerGuard(Some(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), s.clone())))),
        }
    }
}
```

## Complete trace TLS module

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L962-L1058

```rust
pub(crate) mod trace {
    use std::cell::{Cell, RefCell};

    /// One traced operator (pre-order position + depth reconstruct the tree).
    pub(crate) struct Node {
        pub(crate) label: String,
        pub(crate) depth: usize,
        pub(crate) rows: usize,
        pub(crate) nanos: u64,
        /// The planner's estimated output cardinality for this operator, when the
        /// operator has a cardinality model (BGP nodes); `None` for operators whose
        /// output size the planner does not estimate. Used by the structured EXPLAIN
        /// (`explain-json`) to compute the per-operator q-error; the text trace
        /// ignores it. Present only under `explain-json` so the default trace node is
        /// byte-identical. [OPUS-4.8] sq-u4lgr
        #[cfg(feature = "explain-json")]
        pub(crate) est: Option<f64>,
    }

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static DEPTH: Cell<usize> = const { Cell::new(0) };
        static NODES: RefCell<Vec<Node>> = const { RefCell::new(Vec::new()) };
    }

    /// Disables tracing (and clears any partial trace) when the installing entry
    /// point returns — also on error/unwind, so a failed query never leaks a trace.
    pub(crate) struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ENABLED.with(|e| e.set(false));
            DEPTH.with(|d| d.set(0));
            NODES.with(|n| n.borrow_mut().clear());
        }
    }

    pub(crate) fn install() -> Guard {
        ENABLED.with(|e| e.set(true));
        DEPTH.with(|d| d.set(0));
        NODES.with(|n| n.borrow_mut().clear());
        Guard
    }

    #[inline]
    pub(crate) fn enabled() -> bool {
        ENABLED.with(|e| e.get())
    }

    /// Opens a node, returning its index for [`exit`] to fill in.
    pub(crate) fn enter(label: String) -> usize {
        let depth = DEPTH.with(|d| {
            let v = d.get();
            d.set(v + 1);
            v
        });
        NODES.with(|n| {
            let mut n = n.borrow_mut();
            n.push(Node {
                label,
                depth,
                rows: 0,
                nanos: 0,
                #[cfg(feature = "explain-json")]
                est: None,
            });
            n.len() - 1
        })
    }

    pub(crate) fn exit(idx: usize, rows: usize, nanos: u64) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        NODES.with(|n| {
            if let Some(node) = n.borrow_mut().get_mut(idx) {
                node.rows = rows;
                node.nanos = nanos;
            }
        });
    }

    /// Records the planner's estimated output cardinality for the node opened at
    /// `idx` (called by `eval_graph_pattern_traced` only when the BGP estimate is
    /// computed). Separate from [`exit`] so the estimate is captured at ENTER time,
    /// before the (recursive) child evaluation runs. [OPUS-4.8] sq-u4lgr
    #[cfg(feature = "explain-json")]
    pub(crate) fn set_est(idx: usize, est: f64) {
        NODES.with(|n| {
            if let Some(node) = n.borrow_mut().get_mut(idx) {
                node.est = Some(est);
            }
        });
    }

    /// Drains the recorded nodes (pre-order). Call before the guard drops.
    pub(crate) fn take() -> Vec<Node> {
        NODES.with(|n| std::mem::take(&mut *n.borrow_mut()))
    }
}
```

## Complete cache eligibility grammar

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/cache.rs#L58-L197

```rust
///
/// Returns `false` (conservatively) when the algebra contains any value-producing
/// construct that can differ between two evaluations of the same query against the
/// same data: `NOW`, `RAND`, `UUID`, `STRUUID`, `BNODE`, a remote `SERVICE`, or any
/// custom function / custom aggregate (whose Rust closure the engine cannot prove
/// deterministic). Everything else — BGPs, joins, OPTIONAL, UNION, MINUS, FILTER over
/// the built-in deterministic functions, GROUP BY with the standard aggregates,
/// ORDER BY, projection, DISTINCT, LIMIT/OFFSET, VALUES — is cacheable.
///
/// Only the SELECT / ASK forms are considered; CONSTRUCT / DESCRIBE return `false`
/// because the cache stores [`QueryResult`], not a triple set.
pub fn is_cacheable(query: &Query) -> bool {
    match query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => {
            pattern_is_deterministic(pattern)
        }
        // Graph-valued forms are not stored by this cache.
        Query::Construct { .. } | Query::Describe { .. } => false,
    }
}

fn pattern_is_deterministic(p: &GraphPattern) -> bool {
    match p {
        GraphPattern::Bgp { .. } | GraphPattern::Path { .. } | GraphPattern::Values { .. } => true,
        // A remote SERVICE is time-varying and outside our data — never cache.
        GraphPattern::Service { .. } => false,
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            pattern_is_deterministic(left) && pattern_is_deterministic(right)
        }
        // `Lateral` (SEP-0006) is unconditionally present — the workspace enables
        // spargebra's `sep-0006`, which is a spargebra feature, not one of ours.
        GraphPattern::Lateral { left, right } => {
            pattern_is_deterministic(left) && pattern_is_deterministic(right)
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            pattern_is_deterministic(left)
                && pattern_is_deterministic(right)
                && expression
                    .as_ref()
                    .map(expr_is_deterministic)
                    .unwrap_or(true)
        }
        GraphPattern::Filter { expr, inner } => {
            expr_is_deterministic(expr) && pattern_is_deterministic(inner)
        }
        GraphPattern::Graph { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Slice { inner, .. } => pattern_is_deterministic(inner),
        GraphPattern::Extend {
            inner, expression, ..
        } => pattern_is_deterministic(inner) && expr_is_deterministic(expression),
        GraphPattern::OrderBy { inner, expression } => {
            pattern_is_deterministic(inner)
                && expression.iter().all(|o| {
                    let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = o;
                    expr_is_deterministic(e)
                })
        }
        GraphPattern::Group {
            inner, aggregates, ..
        } => {
            pattern_is_deterministic(inner)
                && aggregates
                    .iter()
                    .all(|(_, a)| aggregate_is_deterministic(a))
        }
    }
}

fn aggregate_is_deterministic(a: &AggregateExpression) -> bool {
    match a {
        AggregateExpression::CountSolutions { .. } => true,
        AggregateExpression::FunctionCall { name, expr, .. } => {
            // A custom aggregate's closure is opaque — refuse it. The standard
            // aggregates (incl. SAMPLE, which is implementation-defined but stable
            // for a fixed input on a fixed engine) are fine.
            !matches!(name, AggregateFunction::Custom(_)) && expr_is_deterministic(expr)
        }
    }
}

fn expr_is_deterministic(e: &Expression) -> bool {
    match e {
        Expression::NamedNode(_)
        | Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::Bound(_) => true,
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => expr_is_deterministic(a) && expr_is_deterministic(b),
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            expr_is_deterministic(a)
        }
        Expression::In(a, list) => {
            expr_is_deterministic(a) && list.iter().all(expr_is_deterministic)
        }
        Expression::If(a, b, c) => {
            expr_is_deterministic(a) && expr_is_deterministic(b) && expr_is_deterministic(c)
        }
        Expression::Coalesce(list) => list.iter().all(expr_is_deterministic),
        // EXISTS evaluates an inner pattern — a SERVICE or non-deterministic call in
        // there would taint the result, so recurse.
        Expression::Exists(inner) => pattern_is_deterministic(inner),
        Expression::FunctionCall(func, args) => {
            function_is_deterministic(func) && args.iter().all(expr_is_deterministic)
        }
    }
}

fn function_is_deterministic(f: &Function) -> bool {
    !matches!(
        f,
        // Value-producing, evaluation-varying builtins.
        Function::Now
            | Function::Rand
            | Function::Uuid
            | Function::StrUuid
            // BNODE() mints a fresh blank node; not safe to replay verbatim.
            | Function::BNode
            // A custom (`<iri>`) extension function's closure is opaque to us.
            | Function::Custom(_)
    )
}
```

## Cache key includes full owned Query

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/cache.rs#L204-L208

```rust
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    version: u64,
    query: Query,
}
```

## Cache get/hit/miss/insert scope

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/cache.rs#L271-L332

```rust
    pub fn get_or_eval(
        &self,
        graph: &sparq_core::Graph,
        query: &Query,
        version: u64,
        budget: &QueryBudget,
    ) -> Result<Arc<QueryResult>, String> {
        if !is_cacheable(query) || self.capacity == 0 {
            // Evaluate without touching the store; count it as a miss.
            let r = eval(graph, query, budget)?;
            let mut inner = self.inner.lock().expect("result-cache mutex poisoned");
            inner.misses += 1;
            return Ok(Arc::new(r));
        }

        let key = Key {
            version,
            query: query.clone(),
        };

        // Fast path: a hit at the current version.
        {
            let mut inner = self.inner.lock().expect("result-cache mutex poisoned");
            inner.clock += 1;
            let now = inner.clock;
            if let Some(e) = inner.map.get_mut(&key) {
                e.last_used = now;
                let r = Arc::clone(&e.result);
                inner.hits += 1;
                return Ok(r);
            }
        }

        // Miss: evaluate OUTSIDE the lock so concurrent queries are not serialised.
        let result = Arc::new(eval(graph, query, budget)?);

        let mut inner = self.inner.lock().expect("result-cache mutex poisoned");
        inner.misses += 1;
        // Drop any entry from a superseded version (the writer advanced the epoch).
        inner.map.retain(|k, _| k.version == version);
        inner.clock += 1;
        let now = inner.clock;
        // Evict LRU if at capacity (and this is a genuinely new key).
        if inner.map.len() >= self.capacity && !inner.map.contains_key(&key) {
            if let Some(lru) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                inner.map.remove(&lru);
            }
        }
        inner.map.insert(
            key,
            Entry {
                result: Arc::clone(&result),
                last_used: now,
            },
        );
        Ok(result)
    }
```

## Cache hit/miss observability

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/cache.rs#L342-L349

```rust
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().expect("result-cache mutex poisoned");
        CacheStats {
            hits: inner.hits,
            misses: inner.misses,
            entries: inner.map.len(),
        }
    }
```

## Feature-gated cache and explain-json modules

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1-L35

```rust
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
```

## Engine defaults and parallel forwarding

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/Cargo.toml#L30-L35

```rust
# for native, off for wasm (the wasm crate disables defaults) so the hash cores never
# enter the browser bundle.
default = ["parallel", "regex", "digest"]
parallel = ["dep:rayon", "sparq-core/parallel"]
regex = ["dep:regex"]
digest = ["dep:md-5", "dep:sha1", "dep:sha2"]
```

## No-dependency opt-in feature 231

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/Cargo.toml#L231-L231

```rust
result-cache = []
```

## No-dependency opt-in feature 293

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/Cargo.toml#L293-L293

```rust
explain-json = []
```

## No-dependency opt-in feature 348

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/Cargo.toml#L348-L348

```rust
algebra-rewrite = []
```

## Production panic profiles

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/Cargo.toml#L97-L139

```rust

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"

# [SONNET-4.6] sq-6vshe.10 — optimized correctness/smoke lanes do not need the
# shipping profile's serial codegen and fat-LTO link tail. This profile is never
# valid for benchmarks, perf ratchets, published measurements, or shipped artifacts.
[profile.release-fast]
inherits = "release"
lto = "thin"
codegen-units = 16

# Python extension wheels (T21): release optimisation but UNWINDING panics, so a Rust
# panic inside the `sparq` Python module surfaces as `pyo3_runtime.PanicException`
# instead of `panic = "abort"` killing the whole interpreter. Used by
# `maturin build --profile python-release`; the default release profile is unchanged.
[profile.python-release]
inherits = "release"
panic = "unwind"

# [OPUS-4.8] sq-7d3dj.1 — WASM release profile that BUILDS THE SHIPPED BROWSER BUNDLE.
# Inherits the tuned `release` profile (opt-level 3, fat LTO, codegen-units 1, panic=abort) so
# the HOT engine/core query-eval crates keep their opt-level-3 + simd128 / core::simd FILTER
# investment untouched, then makes only the COLD, run-once RDF/SPARQL PARSE crates build for
# SIZE (opt-level "z") and strips local symbols. Pure parser code sits off every hot query
# path, so this shrinks the wasm bundle with no runtime-perf change. Wired into BOTH the gated
# measurement (scripts/ci-bench.sh -> the wasm_bundle_bytes ratchet) AND the shipped bundle
# (js `build:wasm{,:lean}` -> `wasm-pack --profile release-wasm`), so the gate builds under the
# same cargo profile as the shipped bundle (it ratchets the raw `cargo build` `.wasm`, not the
# post-wasm-bindgen/wasm-opt npm artifact). Blanket opt-level "z" was REJECTED — it would deopt
# the eval kernels.
# NATIVE / non-wasm builds NEVER select this profile, so the default `release` profile — and
# every engine opt-level — is byte-for-byte unchanged.
[profile.release-wasm]
inherits = "release"
strip = "symbols"

# Cold, run-once RDF/SPARQL parse crates only (spargebra = SPARQL query parser, oxiri = IRI
# parser, oxttl = Turtle/N-Triples parser). They are reached once at load/parse time, never on
# the hot query-eval loop, so size-optimising them costs no measurable runtime perf.
```

## Parallel threshold

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L2451-L2454

```rust
#[cfg(feature = "parallel")]
const PAR_THRESHOLD: usize = 50_000;

/// Maximum `offset + limit` (row budget) for the bounded-heap ORDER BY path.
```

## BIND parallel/serial expression branches

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L10660-L10707

```rust
    // the serial intern below, applied in row order → ids byte-identical to the serial path.
    // (Safe to parallelise: rows reference only ids created by EARLIER operators, never ids
    // created within this BIND loop.)
    // Row identity for BNODE(str)'s per-solution scoping (see ROW_SCOPE).
    let scope = b.rows.as_ptr() as usize;
    #[cfg(feature = "parallel")]
    let resolved: Vec<Result<Id, Term>> = if b.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let lv: &LocalVocab = local;
        let bref = &b;
        // Thread-local extension-function registry and dataset view: snapshot +
        // per-item re-install (free when neither is installed) — see the FILTER
        // branch (the view matters here via EXISTS in the expression).
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .enumerate()
            .map(|(i, row)| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                ROW_SCOPE.set((scope, i));
                let v = eval_compiled(graph, lv, bref, row, &compiled)?;
                Ok(value_to_id_readonly(graph, lv, &v))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        let mut out = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            let v = eval_compiled(graph, local, &b, row, &compiled)?;
            out.push(value_to_id_readonly(graph, local, &v));
        }
        out
    };
    #[cfg(not(feature = "parallel"))]
```

## FILTER parallel/serial expression branches

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L12786-L12836

```rust
    }
    // Per-row FILTER evaluation is independent and read-only over the graph/bindings, so a
    // large residual (non-pushed-down) filter is evaluated in parallel on native.
    // Row identity for BNODE(str)'s per-solution scoping (see ROW_SCOPE).
    let scope = b.rows.as_ptr() as usize;
    #[cfg(feature = "parallel")]
    let keep: Vec<bool> = if b.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // The extension-function registry and the dataset view are thread-local:
        // snapshot them here and re-install per worker item (free when neither is
        // installed). The view matters because a FILTER can re-enter pattern
        // evaluation via EXISTS — without the re-install it would silently
        // evaluate UNRESTRICTED on a rayon worker.
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .enumerate()
            .map(|(i, row)| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                ROW_SCOPE.set((scope, i));
                Ok(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?))
            })
            .collect::<Result<Vec<bool>, String>>()?
    } else {
        let mut keep = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            keep.push(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?));
        }
        keep
    };
    #[cfg(not(feature = "parallel"))]
    let keep: Vec<bool> = {
        let mut keep = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            keep.push(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?));
```

## Left-join worker context

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L10090-L10124

```rust
        // path: today the build truncates to nothing and the caller's operator-exit
        // `budget::check` raises this same message, just after the wasted work.
        #[cfg(feature = "parallel")]
        if left_b.rows.len() >= PAR_THRESHOLD {
            use rayon::prelude::*;
            let limits = budget::snapshot();
            let fns = functions::snapshot();
            let vw = view::snapshot();
            let spx = spatial::snapshot();
            // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
            // a worker that missed the registry would dial the IRI instead of answering it.
            #[cfg(feature = "service-local")]
            let lsv = local_services::snapshot();
            #[cfg(not(target_arch = "wasm32"))]
            let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
            let verdicts: Vec<bool> = left_b
                .rows
                .par_iter()
                .map(|lrow| {
                    // One non-tripping `Instant` read per row under a deadline budget, and
                    // a single `on` test when no budget is installed.
                    if let Some(why) = limits.why(0) {
                        return Err(format!("query budget exceeded ({})", why));
                    }
                    let _fns = functions::worker_install(&fns);
                    let _vw = view::worker_install(&vw);
                    let _spx = spatial::worker_install(&spx);
                    #[cfg(feature = "service-local")]
                    let _lsv = local_services::worker_install(&lsv);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _qn = query_now::worker_install(qn);
                    eliminated(lrow)
                })
                .collect::<Result<Vec<bool>, String>>()?;

```

## Grouped expression worker context

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L10861-L10900

```rust
    if parallel_eval {
        use rayon::prelude::*;
        // Thread-local extension-function registry and dataset view: snapshot +
        // per-item re-install (free when neither is installed) — see the FILTER branch.
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        // [OPUS-4.8] (sq-5qz9) keep a custom-aggregate registry visible off-thread, like `fns`.
        // (`self::` because the `aggregates` *parameter* below shadows the module name.)
        #[cfg(feature = "window-functions")]
        let aggs = self::aggregates::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        // Process `members`/`order` in PAR_THRESHOLD-sized batches: evaluate + read-only
        // resolve each batch in parallel, then serially intern only that batch's genuinely
        // new terms and emit its rows. Peak `Value` footprint is one batch, not all groups.
        for (key_chunk, member_chunk) in order.chunks(PAR_THRESHOLD).zip(members.chunks(PAR_THRESHOLD)) {
            let lv: &LocalVocab = local; // immutable reborrow for the read-only parallel phase
            let bref = &b;
            let resolved: Vec<Vec<Result<Id, Term>>> = member_chunk
                .par_iter()
                .map(|members| {
                    let _fns = functions::worker_install(&fns);
                    let _vw = view::worker_install(&vw);
                    let _spx = spatial::worker_install(&spx);
                    #[cfg(feature = "service-local")]
                    let _lsv = local_services::worker_install(&lsv);
                    #[cfg(feature = "window-functions")]
                    let _aggs = self::aggregates::worker_install(&aggs);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _qn = query_now::worker_install(qn);
                    aggregates
                        .iter()
                        .map(|(_, agg)| eval_aggregate(graph, lv, bref, members, agg).map(|v| value_to_id_readonly(graph, lv, &v)))
                        .collect::<Result<Vec<_>, String>>()
                })
```

## ORDER-key worker context

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L11801-L11831

```rust
            let mut keyed: Vec<_> = if n >= PAR_THRESHOLD {
                use rayon::prelude::*;
                // sq-6aefu: mirror FILTER/BIND worker_install pattern — key_of -> eval_compiled
                // can re-enter EXISTS / custom extension functions / spatial expressions on rayon
                // workers; without the snapshot+reinstall the workers see NO view (named-graph
                // leak) and NO function registry (spurious error). [SONNET-4.6]
                let fns = functions::snapshot();
                let vw = view::snapshot();
                let spx = spatial::snapshot();
                // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
                // a worker that missed the registry would dial the IRI instead of answering it.
                #[cfg(feature = "service-local")]
                let lsv = local_services::snapshot();
                #[cfg(not(target_arch = "wasm32"))]
                let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
                b.rows
                    .par_iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let _fns = functions::worker_install(&fns);
                        let _vw = view::worker_install(&vw);
                        let _spx = spatial::worker_install(&spx);
                        #[cfg(feature = "service-local")]
                        let _lsv = local_services::worker_install(&lsv);
                        #[cfg(not(target_arch = "wasm32"))]
                        let _qn = query_now::worker_install(qn);
                        Ok((key_of(row)?, i))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                b.rows
```

## General ORDER-key worker context

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L11883-L11909

```rust
        use rayon::prelude::*;
        // sq-6aefu: mirror FILTER/BIND worker_install pattern — same rationale as the
        // top-k path above: key_of -> eval_compiled can re-enter EXISTS / custom
        // extension functions / spatial on rayon workers. [SONNET-4.6]
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot();
        // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .map(|row| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                Ok((key_of(row)?, row.clone()))
            })
            .collect::<Result<_, String>>()?
    } else {
```

## Text ANALYZE actual BASE/budget/trace order

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain.rs#L73-L115

```rust
pub fn explain_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    // [OPUS-4.8] (sq-7d3dj.30.1) ANALYZE the ACTUAL executed plan (feature-gated rewrite).
    #[cfg(feature = "algebra-rewrite")]
    let q = crate::rewrite::rewrite_query(q);
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let (form, pattern) = query_form_pattern(&q);
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use EXPLAIN for CONSTRUCT/DESCRIBE)".into());
    }

    let mut out = String::new();
    let _ = writeln!(out, "EXPLAIN ANALYZE ({form}) — plan below, then the per-operator execution trace.");
    let _ = writeln!(out, "Plan:");
    render_pattern(graph, pattern, &mut out, 1)?;

    // Execute under the budget with the operator trace installed.
    exec::budget::with_budget(budget, || {
        let _tguard = exec::trace::install();
        #[cfg(not(target_arch = "wasm32"))]
        let start = std::time::Instant::now();
        let total_rows = match &q {
            Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
            Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
            _ => unreachable!(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let total_nanos = start.elapsed().as_nanos() as u64;
        #[cfg(target_arch = "wasm32")]
        let total_nanos = 0u64;
        let nodes = exec::trace::take();

        let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
        for n in &nodes {
            let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
        }
        let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
        Ok(out)
    })
}
```

## Structured ANALYZE actual BASE/budget/trace order

https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain_json.rs#L210-L236

```rust
pub fn explain_plan_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<PlanNode, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use explain_plan for CONSTRUCT/DESCRIBE)".into());
    }

    // Execute under the budget with the operator trace installed (exactly as the
    // text `explain_analyze` does), then reconstruct the typed tree from the trace.
    exec::budget::with_budget(budget, || {
        let _tguard = exec::trace::install();
        match &q {
            Query::Select { pattern, .. } => {
                exec::eval_select(graph, pattern)?;
            }
            Query::Ask { pattern, .. } => {
                exec::eval_ask(graph, pattern)?;
            }
            _ => unreachable!(),
        }
        let nodes = exec::trace::take();
        tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
    })
}
```

//! [OPUS-4.8] (sq-a9cn) Opt-in materialised-view / query-result cache.
//!
//! A SPARQL endpoint that serves the *same* read query repeatedly against a graph
//! that changes only occasionally (the common operational pattern — dashboards,
//! popular saved queries, federation sub-queries) can skip re-execution by keeping
//! the materialised [`QueryResult`] and replaying it. This is the vendor-parity
//! "query result cache" / "materialised view" feature; it is **OFF by default**
//! (the `result-cache` Cargo feature) and pulls **zero new dependencies** — when the
//! feature is off, none of this module compiles and the default build is unchanged.
//!
//! # Soundness — the two correctness obligations
//!
//! A result cache is only correct if a *hit* is exactly the result a fresh
//! evaluation would have produced. Two things can break that, and this module
//! guards both:
//!
//! 1. **The graph changed.** The engine evaluates against a borrowed `&Graph` and
//!    cannot observe mutations through that borrow, so the cache CANNOT detect
//!    staleness on its own. The caller therefore supplies a monotonically-increasing
//!    **version token** ([`u64`]) that it bumps on every mutation
//!    (`apply_delta` / `update` / reload). A cached entry is keyed by
//!    `(query, version)`; once the version advances, every prior entry is
//!    unreachable (and reclaimed lazily). This is the standard "the writer owns the
//!    epoch" contract — the same discipline the snapshot-generation machinery in
//!    `sparq-core` already uses. A caller that holds an immutable graph snapshot can
//!    simply pass a constant version for that snapshot's lifetime.
//!
//! 2. **The query is non-deterministic.** `NOW()`, `RAND()`, `UUID()`, `STRUUID()`,
//!    `BNODE()`, a remote `SERVICE`, or any custom (`<iri>`) function / aggregate can
//!    return a different value on each evaluation *even for an unchanged graph*, so
//!    caching their result would be observably wrong. The cache refuses to store such
//!    queries: [`is_cacheable`] walks the algebra and returns `false`, and
//!    [`ResultCache::get_or_eval`] then always evaluates fresh and never inserts.
//!    Determinism is conservative — anything the walker is unsure about is treated as
//!    non-cacheable.
//!
//! Only SELECT and ASK forms are cached (the [`QueryResult`]-valued surface);
//! CONSTRUCT / DESCRIBE go through a different return type and are out of scope here.
//!
//! # Eviction
//!
//! The cache is a bounded LRU keyed on the parsed [`Query`] algebra (whitespace- and
//! comment-independent — two textually-different spellings of the same query share an
//! entry). When it is full, the least-recently-used entry is dropped. Entries from a
//! superseded version are removed opportunistically on insert, so a long-lived cache
//! never accumulates stale generations.

use crate::{exec, QueryBudget, QueryResult};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
};
use spargebra::Query;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Whether a query's result may be cached without violating the soundness contract
/// in the [module docs](self) — i.e. it is **deterministic** for a fixed graph.
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

/// The cache key: the parsed query algebra paired with the caller's graph version.
///
/// Keying on [`Query`] (which is `Eq + Hash`) rather than the raw SPARQL string makes
/// the cache insensitive to whitespace, comments and prefix spelling — two textual
/// forms that parse to the same algebra share one entry.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    version: u64,
    query: Query,
}

struct Entry {
    result: Arc<QueryResult>,
    /// Monotonic access stamp for LRU eviction.
    last_used: u64,
}

/// A bounded, version-aware SPARQL result cache (a materialised-view store).
///
/// Construct with [`ResultCache::new`], then serve queries through
/// [`get_or_eval`](ResultCache::get_or_eval), passing the current graph **version**
/// (see the [soundness contract](self)). The cache is `Send + Sync` (it locks an
/// internal map) so it can be shared across request threads behind an
/// [`Arc`].
pub struct ResultCache {
    inner: Mutex<Inner>,
    capacity: usize,
}

struct Inner {
    map: HashMap<Key, Entry>,
    /// Monotonic clock for LRU stamps.
    clock: u64,
    hits: u64,
    misses: u64,
}

/// A point-in-time snapshot of cache counters, returned by [`ResultCache::stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of [`get_or_eval`](ResultCache::get_or_eval) calls served from a stored entry.
    pub hits: u64,
    /// Number of calls that had to evaluate (a miss or a non-cacheable query).
    pub misses: u64,
    /// Number of entries currently resident.
    pub entries: usize,
}

impl ResultCache {
    /// A cache holding at most `capacity` distinct cacheable results (LRU eviction).
    /// A `capacity` of 0 disables storage: every call evaluates fresh (useful to
    /// keep the call-site uniform while turning the cache off at runtime).
    pub fn new(capacity: usize) -> Self {
        ResultCache {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                clock: 0,
                hits: 0,
                misses: 0,
            }),
            capacity,
        }
    }

    /// Returns the cached result for `query` at `version`, evaluating and storing it
    /// on a miss. A non-cacheable query (see [`is_cacheable`]) is always evaluated
    /// fresh and never stored — the result is still returned, so the call-site can
    /// route every read query through the cache unconditionally.
    ///
    /// `version` is the caller's graph epoch (bumped on every mutation); a hit is
    /// returned only for the *current* version, so a stale generation can never be
    /// served. The returned [`Arc`] shares the stored result — clone it freely.
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

    /// Drops every stored entry (keeps the configured capacity). A coarse alternative
    /// to bumping the version when the caller cannot thread an epoch through.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("result-cache mutex poisoned");
        inner.map.clear();
    }

    /// Hit / miss counters and resident-entry count.
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().expect("result-cache mutex poisoned");
        CacheStats {
            hits: inner.hits,
            misses: inner.misses,
            entries: inner.map.len(),
        }
    }

    /// The configured maximum number of resident entries.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Evaluate one SELECT / ASK query to a [`QueryResult`], mirroring
/// [`crate::query_prepared_with_budget`] (the dataset / base / budget wiring) so a
/// cached result is identical to the uncached path.
fn eval(
    graph: &sparq_core::Graph,
    query: &Query,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let active = crate::active_dataset(graph, query);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(query.base_iri().map(|b| b.as_str()));
    match query {
        Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
        Query::Ask { pattern, .. } => Ok(QueryResult {
            vars: Vec::new(),
            rows: if exec::eval_ask(graph, pattern)? {
                vec![Vec::new()]
            } else {
                Vec::new()
            },
        }),
        _ => Err("result cache only stores SELECT and ASK queries".into()),
    }
}

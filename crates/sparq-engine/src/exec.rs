//! Physical execution: BGP via greedy-ordered merge/hash joins over the
//! permutation indexes, plus OPTIONAL / UNION / MINUS / BIND / VALUES,
//! aggregation (GROUP BY / HAVING via Filter), ORDER BY, sub-SELECT and the
//! solution modifiers. All intermediate results are id-level (`Bindings`);
//! values computed at query time (BIND, aggregates) get ids in a per-query
//! `LocalVocab`, mirroring QLever's local vocabulary.

use crate::QueryResult;
use std::ops::ControlFlow;
use oxrdf::vocab::xsd;
use oxrdf::{BlankNode, Literal, NamedOrBlankNode, Term, Variable};
use rustc_hash::FxHashMap;
use sparq_core::dict::{self, Id, NO_ID};
use sparq_core::store::Pattern as IdPattern;
use sparq_core::temporal::{ExactTemporal, ExactTimeline, Temporal};
use sparq_core::Graph;
// sq-ev41x (epic sq-qonbz): the id-level numeric value tower (`Num` / `Dec` /
// `ArithOp` / `RoundMode` and the XSD lexical helpers) now lives in the `sparq-substrate`
// leaf crate so the engine AND the reasoners share ONE definition with no `Box<dyn>` on the
// hot path (research/shared-eval-substrate.md, Option C, Phase 2). This is a behaviour-neutral
// code-move: the engine re-imports the same items here under the same names, so every call
// site below is unchanged and the W3C conformance / ORDER BY / numeric tests are bit-identical.
use sparq_substrate::numeric::{f64_exact_decimal, parse_xsd_f32, parse_xsd_f64, split_decimal, ArithOp, Dec, Num};
// sq-hknqs (epic sq-qonbz, Phase 3): the four id-tuple join kernels live in the
// shared substrate now (merge / hash / bind / leapfrog-trie). The engine's planner, `Bindings`,
// `LocalVocab` interning and `ScanCmp` pushdown stay private here; the thin adapters below build
// the `JoinKeys` column layout from a `Bindings` variable layout and supply the engine's
// thread-local query budget as the generic `Budget` hook — no `Box<dyn>` on the probe loop.
use sparq_substrate::join as sjoin;
// sq-vezew (epic sq-qonbz, Phase 4): the SPARQL term TOTAL ORDER —
// `compare_values` (`ORDER BY` / `<` ordering: error < blank < IRI < literal < triple,
// numeric-aware + strict typed/temporal + string fallback + recursive triple-term order) — now
// lives in the shared substrate as the generic `compare::compare_terms`. The engine implements
// the substrate's `CompareTerm` trait for its `Value` (zero-cost wrappers over `value_str` /
// `as_num` / `value_compare_strict`) and `compare_values` is now a thin call into it, so the
// engine AND the reasoners share ONE total-order body with no `Box<dyn>` on the compare path.
// `Value` / `LitKind` / `value_compare_strict` stay engine-private (they also drive the
// relational operators); only the algorithm moved. Behaviour-neutral: the W3C SPARQL / ORDER BY
// / FILTER conformance is bit-identical.
use sparq_substrate::compare::{compare_terms, CompareTerm, LiteralKind, TermClass};
use rustc_hash::FxHashSet;
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
};
use smallvec::SmallVec;
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern};
use std::cmp::Ordering;

// (sq-7d3dj.30.7) Equality-FILTER → value-join unification, behind the opt-in
// `value-join` feature. Declared as a CHILD module of `exec` (via `#[path]`, the file
// lives at `src/eqjoin.rs`) so it reuses the private evaluation machinery — `Bindings`,
// `LocalVocab`, `equal_expr`, `eval_flat_conjunctive`, the query budget — without
// widening any item's visibility. When the feature is off, zero of this code compiles
// and the conjunctive path below is byte-identical to before.
#[cfg(feature = "value-join")]
#[path = "eqjoin.rs"]
mod eqjoin;

#[path = "numeric_capacity.rs"]
mod numeric_capacity;

// ---- Cooperative query budget (T15 server hardening) -------------------------
//
// A thread-local, cooperatively-checked budget installed by the
// `*_with_budget` entry points in lib.rs. Checked at COARSE sites only: operator
// entry (`eval_graph_pattern`) and once per outer iteration / key group of the
// big row-producing loops — never in inner loops. Evaluation is synchronous on
// the installing thread (rayon offloads block the caller), so a thread-local
// suffices; the rayon-parallel branches use a captured `Limits` snapshot to cap
// their own work and the next on-thread check converts that into the error.
pub(crate) mod budget;

// ---- EXPLAIN ANALYZE operator trace (T22) -------------------------------------
//
// A thread-local trace installed only by `explain_analyze*`: every
// `eval_graph_pattern` operator entry records a node (label, depth, output rows,
// wall time). When no trace is installed the entire mechanism is one thread-local
// `Cell<bool>` read per operator entry — the same cost class as the budget check
// that already sits there, and nothing on any per-row path.
pub(crate) mod trace;

// ---- Sideways information passing (SIP): correlated graph-pattern join (sq-7d3dj.30.3) ----
//
// When `Join(A, B)` is evaluated and the ALREADY-EVALUATED side `A` is
// SMALL, evaluate the big child `B` CORRELATED on A's bindings instead of cold: for
// each distinct binding of A's variables that are CERTAINLY bound in B (and bound to
// an IRI in A), substitute that IRI as a constant into B's patterns — pushing into
// UNION branches, BGP scans, property paths and filters — so the scan seeds from the
// constant, then recombine preserving MULTIPLICITY. Conservative: any scope/threshold
// condition failure returns to the existing cold path, bit-for-bit.
//
// Soundness (bag semantics): `A ⋈ B = ⊎_C (A_C ⋈ B)` where `A_C` partitions `A` by the
// pushed-variable values `C`, and `A_C ⋈ B = A_C ⋈ σ_{P=C}(B)`. Because every pushed
// variable is a CERTAIN variable of `B` (bound in every `B` solution), and the
// admitted shape is positive and scope-preserving, substituting the constant is
// `σ_{P=C}(B)` with the pushed columns projected out (re-supplied by
// `A_C`). Pushed values are restricted to IRIs (term identity — no literal value-space /
// numeric-precision hazard), matching the design record (research/
// sp2bench-complex-shape-deficit.md §2.2). SP2Bench q08/q12b: the 1-row `?erdoes` side
// seeds the Union's `?document dc:creator ?erdoes` scan instead of a blind whole-corpus
// creator self-join.
pub(crate) mod sip;

// ---- Correlated (theta) anti-join for OPTIONAL+FILTER(!bound) (sq-7d3dj.30.9) ----
//
// The SPARQL negation idiom `Filter(!bound(?nb), LeftJoin(A, B, Some(F)))`
// with `?nb` CERTAIN in `B` and ABSENT from `A` is exactly an ANTI-JOIN with a theta
// condition: a left row `a` survives iff there is NO `b ∈ B` with `a` and `b`
// compatible on their shared variables AND `F(merge(a, b))` effectively TRUE.
//
// The #1735 rewrite turns the `expression: None` case into `Minus`, but declines here
// because `F` references OUTER variables (SP2Bench q06:
// `?author = ?author2 && ?yr2 < ?yr`), so the cold `left_outer_join` evaluates the
// full right side against the whole document set. This path instead:
//   (a) partitions `F` into var-to-var CORRELATION conjuncts `?outer θ ?inner` whose
//       `?inner` is certain-in-`B` and `?outer` is a left variable — where θ is value
//       `=`, `sameTerm`, or a single-variable `?a IN (?b)` membership (sq-3cmr4) — and a
//       RESIDUAL theta remainder (everything else, e.g. `?yr2 < ?yr`);
//   (b) for each distinct left correlation tuple, SEEDS the `B` scan sideways with the
//       outer IRI bound into `?inner`'s position (extending the #1741 SIP machinery to
//       the anti-join), evaluating a tiny correlated `B'` instead of the cold corpus;
//   (c) for each surviving `b ∈ B'` re-checks the FULL residual theta with verbatim
//       `eval_expr` (3-valued: an error / false does NOT match, so the left row
//       SURVIVES); the left row is dropped iff some `b` makes the whole condition true.
//
// For a very large anchor side the hash-path per-left-row probe is fanned out over
// cores under the `parallel` feature (sq-f1emb): only the boolean elimination VERDICTS
// are parallelised — the ordered output build + budget truncation stay serial — so the
// result is byte-identical to the serial probe.
//
// Soundness / semantic traps (each has a witness test):
//   * The correlation relation θ is load-bearing: value `=`, `sameTerm` and id-equality
//     are DIFFERENT relations on value-equal id-distinct literals (`"1"` vs `"01"`). A
//     value-`=` correlation (including single-var `IN`) seeds / id-probes EXACTLY only
//     when the outer value is an IRI or blank node (where `=` IS term identity); a
//     LITERAL routes through the value-correct COLD anti-join with `=` re-checked (the
//     sq-lr2ii class — a term-identity probe would miss a value-equal-but-not-identical
//     literal and SPURIOUSLY KEEP a row that must drop). A `sameTerm` correlation is
//     TERM IDENTITY for EVERY kind, so a literal key is itself id-probeable and any
//     value-correct scan re-checks with `sameTerm`, never `=`.
//   * Multiplicity: each surviving left row is emitted ONCE (with its original
//     multiplicity), never multiplied by the number of `B` matches it lacks.
//   * Output layout is the LeftJoin's: surviving left rows padded with `NO_ID` for every
//     right-only variable (they never matched), so it is drop-in for the cold plan.
//   * DECLINE is total: any shape/scope/threshold miss returns `Ok(None)` and the caller
//     runs the identical prior `Filter{LeftJoin}` plan, bit-for-bit.
pub(crate) mod theta_antijoin;

/// (sq-7d3dj.30.4) DISTINCT-projection loose (skip) index scan.
///
/// For `Distinct{Project{[?p]}{ BGP / Union-of-BGPs }}` the engine can enumerate the
/// DISTINCT projected values directly from an existing permutation sorted by the
/// projected column (a loose/skip scan — the general form of qlever's "pattern trick"
/// over the six permutations, NO new index) instead of materialising every full-width
/// join row and deduping post-hoc. The load-bearing invariant is DISTINCT result-SET
/// equivalence with the pushdown on vs off; the stats below back the anti-vacuity
/// acceptance test (the produced/scanned rows must COLLAPSE vs the full join size).
pub(crate) mod distinct_pushdown;

/// (sq-jnb1e) Runtime toggle + telemetry for the OPT-IN characteristic-set
/// anchor-incidence prune (the `cs-anchor-incidence` feature). Lets the differential
/// acceptance test compare the incidence-pruned block scan against the exact scan WITHIN one
/// feature-ON binary, and read whether the prune fired plus how many candidate predicates it
/// eliminated. Enabled by default when the feature is compiled in.
#[cfg(feature = "cs-anchor-incidence")]
pub(crate) mod anchor_incidence;

mod sip_rewrite;
use self::sip_rewrite::*;

// ---- Extension-function registry (SPARQL 17.6) --------------------------------
//
// A thread-local registry installed by the `*_with_functions` entry points /
// `with_functions` in lib.rs, consulted ONLY in `eval_function`'s `F::Custom` arm
// after the XSD constructor-cast check misses. The registry-free entry points
// never install one, so their behaviour and hot-path cost are exactly the
// pre-registry ones (the `F::Custom` arm gains one thread-local `Option` check,
// on a path that previously always errored). Like the budget, evaluation is
// synchronous on the installing thread; the three rayon-parallel branches that
// evaluate expressions off-thread (FILTER, BIND, aggregates) snapshot the
// registry and re-install it around each worker item via [`worker_install`].
pub(crate) mod functions;

// ---- Local rows-returning SERVICE handlers (sq-lsp7k.2.2) ----------------------
//
// Mirrors `functions` (install guard + rayon-worker snapshot/re-install) so a
// `SERVICE <iri> { … }` nested under a `FILTER EXISTS` — which re-enters pattern
// evaluation on a rayon worker — still sees the registry. Propagation is load-bearing,
// not a nicety: a worker that MISSED the registry would fall through to the HTTP path
// and try to dial the IRI instead of answering it locally. The overwhelmingly common
// case is NO registry installed, which makes the snapshot path free. NON-DEFAULT
// `service-local` feature: when off, this module does not compile and the default build
// is byte-identical.
#[cfg(feature = "service-local")]
pub(crate) mod local_services;

// ---- Custom aggregate registry (sq-5qz9) ----------------------------------------
//
// Mirrors `functions` exactly (install guard + rayon-worker snapshot/re-install)
// so a custom `AggregateFunction::Custom` IRI in a `GROUP BY` is visible to the
// off-thread group-evaluation path. The overwhelmingly common case is NO registry
// installed, which makes the snapshot path free. NON-DEFAULT `window-functions`
// feature: when off, this module does not compile and the default build is
// byte-identical.
#[cfg(feature = "window-functions")]
pub(crate) mod aggregates;

// ---- MULTIPLICITY() context (sq-v411r, survey §B2) -----------------------------
//
// The SPARQL 1.2 algebra's `multiplicity` device, exposed as a
// zero-arg extension builtin. When an aggregate folds a group's member multiset,
// the evaluator sets this thread-local to the bag cardinality of the member
// solution currently being evaluated (how many byte-identical solution rows the
// group collapses into that member), then clears it. `MULTIPLICITY()` reads it;
// OUTSIDE an aggregate argument the thread-local is `None`, so the builtin is an
// expression error (it is only meaningful within a Set Function over a multiset).
//
// Each rayon group-eval worker has its OWN thread-local and sets it on-thread
// immediately before `eval_expr`, so — unlike the registries above — no
// cross-thread snapshot/re-install is needed: the value never has to outlive the
// single `eval_expr` call on the thread that set it. A `Cell<Option<u64>>` read
// is the same cost class as the budget check; the common (no-aggregate) path
// never writes it. The RAII `Guard` restores the previous value so a nested
// aggregate or an `EXISTS`/sub-query re-entry inside an aggregate argument cannot
// observe a stale outer multiplicity.
pub(crate) mod multiplicity;

// ---- Query-execution NOW() instant (sq-98w7z.1) ---------------------------------
//
// SPARQL 1.1 §17.4.5.1: every `NOW()` within ONE query execution must
// return the SAME `xsd:dateTime`. The instant is pinned by `scope()` at the top of
// `eval_modified` — the single choke point all SELECT / ASK / JSON / count /
// UPDATE-WHERE evaluation funnels through — with OUTERMOST-WINS semantics: the
// guard samples the clock only when no instant is already active, so the recursive
// `eval_modified` calls (solution modifiers, sub-SELECTs) and the `eval_exists`
// re-entry all observe the OUTER execution's instant, and only the outermost
// guard's drop clears the slot. The clear-on-drop is what makes the thread-local
// safe: the NEXT execution on this thread re-samples instead of inheriting a stale
// instant (a plain set-once thread-local would return the first query's time
// forever). The rayon-parallel FILTER / BIND / aggregate branches evaluate
// expressions on worker threads whose slot is empty; they snapshot the instant (a
// `Copy`) and re-install it around each item exactly like the `functions` / `view`
// / `spatial` registries above, restoring the previous value on drop (rayon runs
// some items on the installing thread itself).
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
pub(crate) mod query_now;

// ---- Spatial index (sq-mg9) ----------------------------------------------------
//
// The thread-local [`SpatialProvider`] installed by `with_spatial_index`. The
// planner consults it (`spatial::active`) to push a recognised `geof:` spatial
// FILTER into a candidate window/range scan. Mirrors `functions` exactly: install
// guard, plus a rayon-worker snapshot/re-install so the off-thread FILTER/BIND
// branches see the same index. The overwhelmingly common case is NO index
// installed, which makes the snapshot path free.
pub(crate) mod spatial;

// ---- Dataset view (L1) ---------------------------------------------------------
//
// A thread-local named-graph-subset view installed by the `*_view` entry points /
// `with_view` in lib.rs (see research/solid-access-control-design.md §5 in the
// sparq-solid worktree). Consulted at the only places a dataset is enumerated:
// `eval_graph_named` (named-graph visibility), the BGP/path entries
// (`DefaultGraphMode::Empty`) and `dataset::build_active` (FROM / FROM NAMED
// intersect with the view — restriction composes, never widens). A non-visible
// graph must be INDISTINGUISHABLE from an absent one; that is the security
// property. Like the budget/functions guards, evaluation is synchronous on the
// installing thread; the rayon-parallel expression branches (FILTER / BIND /
// aggregates — the only off-thread paths that can re-enter pattern evaluation,
// via EXISTS) snapshot the view and re-install it around each worker item.
pub(crate) mod view;

/// One solution row: the ids bound to each of a [`Bindings`]' variables. Inlined
/// up to 4 columns (the common case) so a join produces no heap allocation per
/// row — the dominant cost on large join results.
type Row = SmallVec<[Id; 4]>;

/// Result sizes at/above which the embarrassingly-parallel materialisation steps
/// (row building, term reconstruction) are worth handing to rayon; below it the
/// thread hand-off costs more than it saves. Native only (the wasm build has no
/// `parallel` feature, so these paths compile to the sequential loops).
#[cfg(feature = "parallel")]
const PAR_THRESHOLD: usize = 50_000;

/// Maximum `offset + limit` (row budget) for the bounded-heap ORDER BY path.
/// When the requested budget k is at most this threshold AND k < n (total rows),
/// the `Slice{...OrderBy{...}}` pattern uses bounded selection O(n + k log k)
/// instead of a full stable sort O(n log n). The threshold avoids heap overhead
/// when k is large relative to n. sq-7d3dj.30.2
const TOP_K_ORDER_BY_THRESHOLD: usize = 1_024;

/// A join / group key (the ids of the shared or grouping columns). Inlined up to
/// 2 columns — most joins are on one or two variables — so building a hash table
/// or probing it allocates nothing per key.
type Key = SmallVec<[Id; 2]>;

/// A hash-table posting list (row indices sharing a key). Inlined up to 2 — many
/// join keys (and almost all OPTIONAL keys) match only one or two rows — so the
/// build allocates nothing per bucket in the common case.
type Posting = SmallVec<[usize; 2]>;

/// sq-hknqs (epic sq-qonbz, Phase 3): the engine's cooperative-cancel hook for the
/// shared substrate join kernels. A zero-sized type whose [`exhausted`](sjoin::Budget::exhausted)
/// reads the engine's thread-local [`budget`] (the per-WHERE-solution `QueryBudget`), so a
/// capped/timed-out query truncates a join cleanly at a key-group / probe-row boundary — exactly
/// the engine's pre-move per-group `budget::exhausted(...)` check. Generic, not a trait object,
/// so the substrate kernel monomorphises the call into the same direct check it was before the
/// move (no vtable on the probe loop).
struct EngineBudget;

impl sjoin::Budget for EngineBudget {
    #[inline]
    fn exhausted(&self, rows: usize) -> bool {
        budget::exhausted(rows)
    }
}

/// sq-hknqs: the parallel hash-join's per-worker exhaustion snapshot. Wraps the
/// flattened [`budget::Limits`] (the installing thread's sticky flag is invisible to rayon
/// workers) so a worker that hits the limits stops adding to its accumulator; the caller's next
/// on-thread check raises the actual error. Generic ([`sjoin::BudgetSnapshot`]), so the rayon
/// fold carries no vtable.
#[cfg(feature = "parallel")]
struct EngineSnapshot(budget::Limits);

#[cfg(feature = "parallel")]
impl sjoin::BudgetSnapshot for EngineSnapshot {
    #[inline]
    fn hit(&self, rows: usize) -> bool {
        self.0.hit(rows)
    }
}

/// Ids at or above this base index into the per-query [`LocalVocab`] instead of the graph
/// dictionary. It sits ABOVE the dictionary range `[1, INLINE_BASE)` and the inline-integer
/// range `[INLINE_BASE, INLINE_BASE + 2^30)`, i.e. at `INLINE_BASE + 2^30 = 3·2^30`, leaving
/// the local vocab `[3·2^30, 2^32)` (≈1.07B query-computed terms — far more than any query).
const LOCAL_BASE: Id = dict::INLINE_BASE + (1 << 30);

#[inline]
fn is_local(id: Id) -> bool {
    id >= LOCAL_BASE
}

/// (sq-s5is) Estimated heap bytes of a query-computed term, for the
/// byte-accounted budget's local-vocab accounting. The struct itself plus the lexical
/// bytes of the value/IRI (+ datatype IRI / language tag for a literal). A LOWER bound
/// (ignores allocator overhead), conservative in the same direction the rest of the cap
/// is. Only ever called while a budget is installed (from `LocalVocab::intern`).
fn local_term_bytes(t: &Term) -> usize {
    let base = std::mem::size_of::<Term>();
    match t {
        Term::NamedNode(n) => base + n.as_str().len(),
        Term::BlankNode(b) => base + b.as_str().len(),
        Term::Literal(l) => {
            base + l.value().len()
                + l.datatype().as_str().len()
                + l.language().map_or(0, str::len)
        }
        // RDF 1.2 triple terms: charge the struct + each component recursively.
        other => base + format!("{other}").len(),
    }
}

/// Per-query vocabulary for terms produced during evaluation (BIND, aggregates)
/// that are not in the graph dictionary.
#[derive(Default)]
pub struct LocalVocab<'dataset> {
    ebv_semantics: crate::EbvSemantics,
    terms: Vec<Term>,
    ids: FxHashMap<Term, Id>,
    /// Parallel to `terms`: the f64 value of each numeric local literal (NaN
    /// otherwise) — the local-vocab twin of the graph's `numerics` cache, so a
    /// FILTER/comparison over a BIND-computed numeric does not clone + re-parse
    /// the term per row.
    nums: Vec<f64>,
    /// Bound outer terms for EXISTS expression substitution. Kept with
    /// this evaluation's vocabulary, so nested queries and Rayon workers cannot
    /// inherit another evaluation's bindings through thread-local state.
    correlation: FxHashMap<Variable, Term>,
    /// The active dataset catalog is independent of the active graph.
    /// Nested GRAPH switches graph dictionaries without losing this borrowed
    /// catalog; it is scoped to this evaluation, with no cloning or global state.
    dataset: Option<&'dataset Graph>,
    /// Remove substituted variables before domain-sensitive operators.
    substitute_exists_domains: bool,
}

impl<'dataset> LocalVocab<'dataset> {
    fn for_query() -> Self {
        Self { ebv_semantics: budget::ebv_semantics(), ..Self::default() }
    }

    fn for_dataset(dataset: &'dataset Graph) -> Self {
        Self { dataset: Some(dataset), ..Self::for_query() }
    }
    /// Interns a term, returning a stable id: equal terms get the same id so
    /// DISTINCT, GROUP BY, joins and equality work on computed values.
    fn intern(&mut self, t: Term) -> Id {
        if let Term::Literal(literal) = &t {
            // Sticky: interning cannot turn an evaluation-capacity failure into unbound.
            let _ = budget::check_temporal(literal.value(), literal.datatype().as_str());
        }
        if let Some(&id) = self.ids.get(&t) {
            return id;
        }
        let id = LOCAL_BASE + self.terms.len() as Id;
        self.nums.push(match &t {
            // sq-74oy4 / sq-6b1lj: cache the DATATYPE-AWARE f64 (`numeric_cache_f64`)
            // — the SAME acceptance the graph `numeric_value` cache and the lenient `as_num`
            // seam use — so a computed (BIND/aggregate) numeric term joins/compares identically
            // to a graph term. It validates raw RDF lexical bytes verbatim and rejects a per-datatype-
            // ill-formed lexical (`"1.5"^^xsd:integer`); either folds to the NaN cache-miss
            // sentinel, deferring `=`/`<`/`>` to the exact evaluator (which type-errors it).
            Term::Literal(l) => numeric_cache_f64(l).unwrap_or(f64::NAN),
            _ => f64::NAN,
        });
        // (sq-s5is) Charge the byte cap for this newly-computed term (BIND /
        // aggregate / CONSTRUCT scratch) — the NON-row dimension the row cap misses. A
        // query that materialises few rows but huge string literals is now bounded. The
        // term is stored TWICE (the `terms` vec + the `ids` key), so count both.
        budget::add_bytes(2usize.saturating_mul(local_term_bytes(&t)));
        self.terms.push(t.clone());
        self.ids.insert(t, id);
        id
    }
    /// The append cursor into the local vocab, paired with [`LocalVocab::rollback_to`]
    /// to discard a speculative interning burst. (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    fn savepoint(&self) -> usize {
        self.terms.len()
    }
    /// Drop every term interned since `mark`, so the rows a SILENT SERVICE error
    /// discards leave the local vocab EXACTLY as if they were never interned — no
    /// retained memory. Pairs with a `budget::ByteSavepoint` (which refunds the bytes
    /// those terms charged); together they are the resource twin of dropping the
    /// discarded id rows, keeping SILENT SERVICE behaviour-neutral with the pre-streaming
    /// path. Interns BEFORE `mark` — and re-interns that returned an existing id, which
    /// pushed nothing — are untouched. (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    fn rollback_to(&mut self, mark: usize) {
        for t in self.terms.drain(mark..) {
            self.ids.remove(&t);
        }
        self.nums.truncate(mark);
    }
    fn term(&self, id: Id) -> &Term {
        &self.terms[(id - LOCAL_BASE) as usize]
    }
    /// The cached numeric value of a local id (`None` for a non-numeric term).
    /// For every non-NaN numeric term this is exactly what `as_num` of the
    /// materialised term returns — both parse through `parse_xsd_f64`. A genuine
    /// `xsd:double`/`float` `NaN` is the one case they differ: the cache uses NaN as
    /// its not-a-number sentinel, so it reports `None` where `as_num` returns
    /// `Some(NaN)` (a pre-existing sentinel limitation, not a parse-set disagreement).
    #[inline]
    fn numeric(&self, id: Id) -> Option<f64> {
        let v = self.nums[(id - LOCAL_BASE) as usize];
        if v.is_nan() {
            None
        } else {
            Some(v)
        }
    }
}

/// Resolves an id to its term (graph dictionary or local vocab); `NO_ID` -> None.
fn term_of(graph: &Graph, local: &LocalVocab, id: Id) -> Option<Term> {
    if id == NO_ID {
        None
    } else if is_local(id) {
        Some(local.term(id).clone())
    } else {
        Some(graph.dict.term(id))
    }
}

/// Intermediate result: rows of one id per `vars[i]` (`NO_ID` = unbound).
/// `sorted_by` records the variable the rows are sorted on (if any), enabling a
/// merge join instead of a hash table.
struct Bindings {
    vars: Vec<Variable>,
    rows: Vec<Row>,
    sorted_by: Option<Variable>,
}

impl Bindings {
    fn col(&self, v: &Variable) -> Option<usize> {
        self.vars.iter().position(|x| x == v)
    }
    fn unsorted(vars: Vec<Variable>, rows: Vec<Row>) -> Self {
        Bindings { vars, rows, sorted_by: None }
    }
}

mod entry;
pub use self::entry::*;

mod dispatch;
pub(crate) use self::dispatch::*;
// Test-only observations pin retained preparation work, not timing.
#[cfg(test)]
mod indexed_topk_preparation_tests;
#[path = "exists_domain.rs"]
mod exists_domain;
/// Test/embedder seam for the SERVICE HTTP transport. By default `with` runs the
/// closure against the production `sparq_engine_service::service::HttpTransport`; tests install a
/// fake (loopback / canned) transport for the duration of a scope so SERVICE can be
/// exercised without a public-network dependency.
#[cfg(feature = "service")]
pub(crate) mod service_transport;
/// Reader-seam transport dispatcher: analogous to `service_transport` but uses the
/// `ReaderTransport` seam so the HTTP body is NEVER buffered into a full `String`.
/// (bead sq-my8wd.5)
///
/// When a test `Transport` is installed via `service_transport::install`, it is
/// wrapped in `TransportAsReader` so it satisfies `ReaderTransport` — tests continue
/// to work unchanged. The production path (`None` installed) uses `HttpTransport`
/// directly as `ReaderTransport`, which streams the body via ureq's `into_reader()`.
#[cfg(feature = "service")]
pub(crate) mod service_reader_transport;

mod sargable;
pub(crate) use self::sargable::*;

mod spatial_pushdown;
use self::spatial_pushdown::*;

mod bgp;
use self::bgp::*;

mod triple_terms;
pub(crate) use self::triple_terms::*;

#[cfg(feature = "yannakakis")]
mod yannakakis;
#[cfg(feature = "yannakakis")]
use self::yannakakis::*;

mod planning;
pub(crate) use self::planning::*;

mod wcoj;
pub(crate) use self::wcoj::*;
// sq-hknqs (epic sq-qonbz, Phase 3): the id-tuple combine/compatibility helpers
// (`compatible` / `merge_rows` / `any_unbound`) now live in the shared substrate
// (`sparq_substrate::join`). The engine's OPTIONAL / UNION / MINUS / VALUES-UNDEF nested-loop
// fallbacks and the hash-join build/probe call them under these private aliases, so every
// existing `Bindings`-side call site is unchanged.
use sjoin::{any_unbound, compatible, merge_rows};
// The join hash (`key_hash` / `JOIN_PARTS`) is only reached by the radix-partitioned PARALLEL
// hash-join build (native only); the wasm / serial build computes the table without them.
#[cfg(feature = "parallel")]
use sjoin::{key_hash, JOIN_PARTS};
/// Bag equivalence, bounded per-key scans, and real-query reachability for bind-join
/// grouping. Test-only counters observe the chosen path without depending on hash-map
/// iteration order; the randomized oracle is independent of either grouping strategy.
///
#[cfg(test)]
mod bind_join_run_grouping;

mod compose;
use self::compose::*;

mod antijoin;
use self::antijoin::*;

mod aggregation;
pub(crate) use self::aggregation::*;

mod modifiers;
use self::modifiers::*;
/// sq-7d3dj.30.12 — DIFFERENTIAL property test for the precomputed sort-collation
/// keys. The `SortCell::Val` arm of `cmp_sort_cells` now hoists the SPARQL total-order class
/// / literal-kind dispatch to cell-CONSTRUCTION time (`val_ranks`) and only falls back to the
/// full `compare_values` within a tied class+kind. This suite pins the load-bearing
/// invariant: for EVERY pair of key values, the precomputed-key ordering equals the current
/// comparator's ordering EXACTLY — including the sq-lr2ii f64-collision trap (high-precision
/// decimals / integers past 2^53 that share one f64 image), NaN/INF spellings, and mixed
/// kinds. A wrong rank, or a fall-through that reordered a within-kind tie, goes red here.
#[cfg(test)]
mod sort_collation_key_differential;
/// sq-7d3dj.30.21 — DIFFERENTIAL test for the LAZY plain-string-literal sort cell
/// (`topk-lazy-strkey`). It pins the load-bearing invariant of the feature: a `SortCell::StrId`
/// (an interned plain `xsd:string` literal carried as a dict id, compared via the ZERO-COPY
/// `plain_string_value` bytes) orders BYTE-IDENTICALLY to the eager `SortCell::Val` cell the
/// feature-OFF path builds for the SAME literal — for EVERY pair, both string-vs-string (the
/// hot q11 case, incl. the `"10" < "2"` / empty-string ties) and string-vs-every-other-kind
/// (the mixed-column cross-type arms). A wrong class/kind rank, a mis-read value slice, or a
/// mis-ordered fall-through goes red here. If it agreed with itself but not with the eager
/// path, the top-k output under the feature would silently differ.
#[cfg(all(test, feature = "topk-lazy-strkey"))]
mod lazy_strkey_differential;

mod expr;
use self::expr::*;

mod compiled;
pub(crate) use self::compiled::*;
/// Thread-local PRNG behind `RAND()` (sq-98w7z.1).
///
/// The old arm drew `uuid::Uuid::new_v4()` — an OS-RNG syscall — PER ROW. SPARQL
/// `RAND()` (§17.4.5.2) has no cryptographic requirement, only "a fresh value per
/// call", so each thread seeds ONCE from the OS RNG (uuid v4, the crate's existing
/// entropy source — no new dependency) and then advances a splitmix64 state: every
/// call still yields a fresh, non-deterministic value (the seed is fresh entropy
/// per thread per process), rayon workers get independent streams, and `EXISTS`
/// re-entry merely draws the next value — there is no layout- or query-dependent
/// state to leak. Deliberately NOT deterministic or query-constant.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "zkvm")))]
mod rand_unit;
/// Per-thread `REGEX()`/`REPLACE()` compile memo (sq-98w7z.1).
///
/// `build_regex` used to run INSIDE the per-row scalar eval, so a constant-pattern
/// FILTER recompiled the regex on every row (compile is µs-scale vs ns-scale match).
/// The memo compiles each distinct `(pattern, flags)` ONCE per thread and hands out
/// the same instance via `Rc` — sharing the INSTANCE (not clones of it) is what
/// preserves the warmed lazy-DFA cache, the same bug class the SHACL fix (PR #1891)
/// found. A compile FAILURE is memoised as `None`, so an invalid pattern or flag
/// stays a per-row type error (`Value::Error`) exactly as before — the cache stores
/// the error-or-regex OUTCOME, never swallows it.
///
/// Re-entry safety: the key `(pattern, flags)` fully determines the compiled regex
/// independent of any `Bindings` layout or query context, so an `EXISTS`-nested
/// FILTER re-entering this thread can only HIT the memo correctly — unlike a
/// column-index thread-local, there is nothing execution-scoped to drain or reset,
/// and for the same reason entries carrying over across queries cannot change any
/// result. The `RefCell` borrow never spans user code (`build_regex` is pure), so
/// re-entry cannot observe a live borrow.
#[cfg(feature = "regex")]
mod regex_cache;

mod terms;
use self::terms::*;
#[cfg(test)]
mod exact_decimal_tests;
#[cfg(test)]
mod wcoj_tests;
#[cfg(test)]
mod function_tests;
/// sq-98w7z.1 — per-row builtin memoisation: (A) the REGEX/REPLACE
/// compile memo, (B) the RAND() thread-local PRNG, (C) NOW() query-constancy
/// (SPARQL 1.1 §17.4.5.1). Every test here exercises the REAL evaluation path
/// (`crate::query*` or the actual thread-local modules), never a mock.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod builtin_memo_tests;
/// sq-6qkq — per-builtin ERROR-PATH table test.
///
/// The happy-path suite above (`function_tests`) only proves builtins compute the
/// right value on WELL-TYPED input. This module is the systematic complement: one
/// table ROW per SPARQL builtin, each carrying up to three probes — `valid` (a
/// well-typed call → the EXACT expected term, a sanity anchor), `type_err` (a
/// wrong-typed argument → the engine's expression-error outcome), and `boundary` (a
/// meaningful edge input → the EXACT expected term, where one exists).
///
/// HOW AN EXPRESSION ERROR IS OBSERVED. SPARQL has no first-class "error value" a
/// SELECT can surface; an expression error manifests structurally. We BIND the probe
/// in `SELECT ?out WHERE { ?s :p ?o BIND(<EXPR> AS ?out) }` over a single-row graph:
/// `extend_bindings` evaluates the expression and, on `Value::Error`/`Value::Unbound`,
/// resolves it to `NO_ID` → the row is KEPT but `?out` projects as `None` (unbound).
/// So the contract is precise and machine-checkable: a VALID/BOUNDARY probe yields
/// exactly one row with `?out = Some(expected term)`, and a TYPE-ERROR probe yields
/// exactly one row with `?out = None` (unbound). This is the SAME `Value::Error`
/// convention asserted lexically by `lib.rs::relational_type_error_semantics` and
/// `logical_error_propagation_and_short_circuit` via FILTER row-drops; here we read the
/// BIND column back so a row can assert BOTH a concrete value AND the unbound-on-error
/// outcome in one shape.
///
/// SPEC, NOT BUG-FOR-BUG. The probes encode the SPARQL 1.1 / 1.2 SPEC-correct
/// outcome. Where the engine diverged (STRLEN / ENCODE_FOR_URI / LANGMATCHES wrongly
/// `STR()`-coerced an IRI/number instead of erroring), the divergence was FIXED in the
/// dispatch above (see the `` arms) so these rows pass on the corrected
/// behaviour; a row that the engine cannot yet satisfy correctly would be `#[ignore]`d
/// with a comment + a filed bead rather than asserting the wrong value as "expected".
///
/// The table shape (a variant-indexed list of probes, driven once) mirrors
/// `sparq-zk-compose/tests/forge_gates.rs`'s gate-indexed map.
#[cfg(test)]
mod builtin_error_paths;
#[cfg(test)]
mod path_tests;
#[cfg(test)]
mod ask_exists_tests;
#[cfg(test)]
mod path_pushdown_tests;
/// SERVICE algebra-integration tests that drive `eval_service` directly through the
/// `service_transport` injection seam — no HTTP, no network. They cover what the
/// out-of-crate `tests/service_federation.rs` cannot reach: the term-interning and
/// the SILENT/variable-endpoint branches with a mocked transport.
#[cfg(all(test, feature = "service"))]
mod service_exec_tests;
/// sq-53ti (gh-49) — SPARQL 1.1 aggregate-over-EMPTY-multiset semantics.
///
/// Pins the §18.5.1 "Set Functions over an empty group" contract that downstream
/// consumers (notably PSS `usage()`, which sums resource sizes over a possibly-empty
/// graph match and reads a single all-but-COUNT-bound row) depend on. The rule, split
/// by the one axis that matters — whether a `GROUP BY` is present:
///
/// * **No `GROUP BY`, the WHERE clause matches nothing** → the whole solution set is a
///   SINGLE implicit group, so the result is exactly ONE row (NOT zero). In that row:
///   - `COUNT(*)` and `COUNT(?x)` → `"0"^^xsd:integer` (§18.5.1.1: Count({}) = 0)
///   - `SUM(?x)` → `"0"^^xsd:integer` (§18.5.1.4: Sum({}) = 0)
///   - `AVG(?x)` → `"0"^^xsd:integer` (§18.5.1.5: Avg({}) = 0)
///   - `MIN(?x)` / `MAX(?x)` → UNBOUND (§18.5.1.6/7: an error over {} ⇒ the variable is left unbound for that row)
///   - `SAMPLE(?x)` → UNBOUND (§18.5.1.8: no element to sample)
///   - `GROUP_CONCAT(?x)` → `""` (empty string) (§18.5.1.9: concat of zero strings)
///
///   The SUM/AVG-vs-MIN/MAX split is the subtle part: SUM and AVG over the empty
///   multiset are DEFINED to be the integer `0` (so `COALESCE(SUM(?x), 0)` and a bare
///   `SUM(?x)` both yield `0`, and a single bound `0` cell — not an unbound one), whereas
///   MIN/MAX/SAMPLE raise an error that surfaces as an UNBOUND cell in the one result row.
///
/// * **With `GROUP BY`, the input is empty** → there are ZERO groups, hence ZERO result
///   rows (the implicit-single-group rule applies ONLY when no GROUP BY is written).
///
/// Ground truth: this matches the engine's behaviour today AND the W3C SPARQL 1.1
/// `aggregates` conformance suite — `agg-empty-group-count-2` (COUNT, no GROUP BY → one
/// `0` row), `agg-empty-group-max-2` (MAX, no GROUP BY → one all-unbound row),
/// `agg-empty-group-count-1` / `agg-empty-group-max-1` (with GROUP BY → zero rows). These
/// tests are the GUARANTEE that pins the contract against regressions.
#[cfg(test)]
mod aggregate_empty_semantics;
/// `GROUP BY ?v` where `?v` is **never bound** anywhere in the WHERE clause must NOT panic.
/// Per SPARQL 1.1 §11.1 the key for an unbound grouping variable is unbound for every solution,
/// so all matching rows collapse into ONE group with `?v` unbound in the output. Regression for
/// bead sq-vymy4 — the engine previously panicked with `group var present` at the `key_cols`
/// build.
#[cfg(test)]
mod group_by_unbound_var;
/// (sq-v411r, survey §B2) The SPARQL 1.2 algebra's `multiplicity` device exposed
/// as a zero-argument extension builtin `MULTIPLICITY()`. These tests drive the REAL query
/// path (`crate::query`) end-to-end — parse (vendored spargebra) → plan → aggregate eval →
/// the `F::Multiplicity` dispatch arm reading the per-member thread-local set by
/// `group_multiplicities` — so the new lines are covered by the coverage ratchet.
///
/// The multiset is built with a sub-SELECT that projects away the per-row subject, so several
/// subjects sharing a value collapse to byte-identical solutions (bag semantics, no implicit
/// DISTINCT). That gives a group whose distinct members carry a multiplicity > 1, which is the
/// only configuration in which `MULTIPLICITY()` is observable.
#[cfg(test)]
mod multiplicity_builtin;
/// sq-rikm7 — ORDER BY / MIN / MAX agree with relational `=`/`<` on
/// f64-collapsed DISTINCT numeric values (integers beyond 2^53, high-precision decimals).
///
/// The lenient numeric arm coerces operands to f64, whose rounding COLLAPSES two distinct
/// exact values that share one f64. The relational operators (`cmp_expr`/`equal_expr`) and
/// MIN/MAX (`num_compare`) already recheck exactly; before this fix ORDER BY did NOT (the
/// substrate `compare_terms` numeric arm and the engine's numeric SortCell fast path both
/// collapsed), so ORDER BY disagreed with them. These tests pin the agreement and FAIL if
/// either recheck (the `compare_terms` `exact_cmp` hook or the `cmp_sort_num` tie recheck)
/// is reverted.
#[cfg(test)]
mod f64_collapse_order_agreement;
// (sq-pntvh.3, M4 Phase 3) Seam-level differential for the columnar residual-FILTER
// path. The chunk-primitive kernels are unit-tested in `chunk.rs`; this asserts the load-bearing
// wiring invariant: `apply_filter`'s columnar seam produces `b.rows` BYTE-IDENTICAL (same rows,
// same order) to the scalar row path over a REAL graph, and DECLINES (falls back to the scalar
// path) on every column shape not provably identical. Only built under the opt-in `vectorized`
// feature.
#[cfg(all(test, feature = "vectorized"))]
mod columnar_filter_seam;
// sq-qcnn.13 — Direct unit tests for internal exec functions (private API).
// These supplement the integration tests in tests/eval_semantics.rs with white-box
// coverage of paths reachable only from inside the crate: `minus_bindings` (disjoint
// fast path + fully-bound fast path + the unbound-shared-variable general compatibility
// scan), `effective_boolean` / `ebv` error arms, aggregate-error propagation via
// `sum_values(_, errored=true)`, and the `minmax_values` numeric-selection + mixed-type
// fallback paths.

/// Direct unit tests for `minus_bindings` — exercises the fast-path (disjoint
/// domains → no-op, fully-bound compatible rows) and the general path (unbound
/// shared variable triggering the compatibility scan with bound-domain-overlap check).
#[cfg(test)]
mod minus_bindings_unit;
/// Direct unit tests for `effective_boolean` / `ebv`: the three-valued logic gate
/// used by every FILTER application. Pins the SPARQL 3VL contract at the function
/// level so a mutation that collapses error→false or error→true is caught here.
#[cfg(test)]
mod effective_boolean_unit;
/// Direct unit tests for `sum_values` — the typed SUM with XSD promotion. Pins the
/// `errored=true` → `None` branch and the integer-only and decimal-promotion paths.
#[cfg(test)]
mod sum_values_unit;
/// Direct unit tests for `minmax_values` — numeric selection path and empty-set path.
#[cfg(test)]
mod minmax_values_unit;
// (sq-pntvh.4, M4 Phase 4) Seam-level differential for the columnar
// GROUP-BY aggregate path. Verifies that `columnar_aggregate` produces output
// BYTE-IDENTICAL to the scalar group fold (`group_aggregate`) over a REAL graph, and
// DECLINES on every column shape not provably identical. Only built under `vectorized`.
#[cfg(all(test, feature = "vectorized"))]
mod columnar_aggregate_seam;

// ── Expression-compilation invariant tests (sq-7d3dj.4) ─────────────────────────────────
//
// These tests pin the LOAD-BEARING INVARIANT: eval_compiled produces bit-identical results
// to eval_expr for all expression types and binding shapes, including the FILTER, BIND, and
// ORDER BY operators. A divergence here would be a semantic change.  sq-7d3dj.4

#[cfg(test)]
mod compiled_expr_tests;

#[cfg(test)]
mod sip_unit;

#[cfg(test)]
mod theta_antijoin_unit;

// ── id-level term-identity FILTER fast path (sq-7d3dj.30.11) ──────────────────────────────
#[cfg(all(test, feature = "id-filter-fastpath"))]
mod idfast_unit;

// ── order_bindings parallel worker reinstall regression (sq-6aefu) ─────────────────────────────
//
// Both PAR_THRESHOLD key-build sites (top-k index-carry AND full stable sort) must snapshot +
// re-install functions / view / spatial on rayon workers, mirroring the FILTER/BIND/aggregates
// pattern that sq-98w7z.1 established for query_now.  Without the reinstall:
//   (1) EXISTS in ORDER BY re-enters pattern evaluation on workers with NO DatasetView →
//       named-graph visibility leaks into sort keys (security-adjacent, silent wrong order).
//   (2) A custom extension function in ORDER BY loses the FunctionRegistry on workers →
//       `eval_function` returns Err("unsupported") → the whole ORDER BY call fails.
//
// The tests below are gated on `parallel` (the rayon path is feature-gated) and exercise
// BOTH sites via the engine's normal SPARQL query entry points:
//   - `query_view_with_budget` for the view/EXISTS scenario
//   - `query_with_functions_and_budget` for the FunctionRegistry scenario
//
// NON-VACUITY: each test was confirmed to FAIL (red) on the un-patched exec.rs (fix stashed)
// and PASS (green) after the patch, satisfying the acceptance criterion.  The failure mode
// without the fix is documented in each test's doc-comment.
#[cfg(all(test, feature = "parallel"))]
mod order_bindings_worker_reinstall;

// Actual public-query path and physical RHS-work witness for #3105.
#[cfg(test)]
mod capped_rhs_tests;

// Nullable path semantics have an independent bottom-up test oracle.
#[cfg(test)]
#[path = "nullable_path_tests.rs"]
mod nullable_path_tests;

#[cfg(test)]
mod exact_temporal_sort_cache_tests;

/// #4467 — the scoped registry guards restore the registry the install replaced
/// (rather than clearing it), so a nested install hands the outer scope its own
/// registry back, on normal return and on unwind alike.
#[cfg(test)]
mod scoped_registry_tests;

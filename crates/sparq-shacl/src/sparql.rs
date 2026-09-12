//! SHACL-SPARQL (`sh:sparql`, W3C SHACL §5.2): SPARQL-based constraints.
//!
//! A `sh:sparql` constraint carries an `sh:select` query that is run against the
//! data graph once per focus node; **each returned solution is a violation**. The
//! focus node is supplied to the query by *pre-binding* the variable `$this`
//! (SHACL §5.2.1) — done here at the algebra level by injecting a `VALUES (?this)
//! { (<focus>) }` table into the parsed query's WHERE clause, which is robust to
//! however the author laid the query text out (a textual prepend is not).
//!
//! `sh:prefixes` (via `sh:declare` / `sh:prefix` / `sh:namespace`) supply the
//! query's prefix declarations; they are pre-pended to the `sh:select` text
//! before parsing. Per the spec a `sh:sparql` constraint MUST carry a `sh:select`
//! whose result variables include neither a reserved nor an undeclared-prefix
//! token — anything that fails to parse is treated as an ill-formed constraint
//! and skipped (consistent with this crate's lenient handling of ill-formed
//! shapes; see this crate's open beads — `bd list -l area:sparq-shacl`).
//!
//! Solution → validation-result mapping (SHACL §5.2.2):
//!   * `sh:focusNode`   ← the focus node ($this);
//!   * `sh:value`       ← the `?value` binding, if the solution binds it;
//!   * `sh:resultPath`  ← the `?path` binding when it is an IRI (a predicate path);
//!   * `sh:resultMessage` ← the constraint's `sh:message` (with `{?var}`
//!     substitution from the solution), else a `?message` binding, else a default.
//!
//! `$shapesGraph` / `$currentShape` pre-binding: scoped honestly. `$currentShape`
//! is bound to the source shape's node (available and cheap). `$shapesGraph` is
//! NOT bound — this validator evaluates the `sh:select` against the DATA graph
//! only (it has no named-graph handle to the shapes graph at query time), so a
//! query that dereferences `$shapesGraph` will simply find no solutions for it.
//! See the crate TODO for the deferred full SPARQL-based-constraint-component
//! machinery.

use crate::model::SparqlConstraint;
use crate::report::ValidationResult;
use oxrdf::{Term, Variable};
use rustc_hash::FxHashMap;
use spargebra::algebra::GraphPattern;
use spargebra::term::GroundTerm;
use spargebra::{Query, SparqlParser};
use std::cell::Cell;

thread_local! {
    /// [OPUS-4.8] (sq-7d3dj.33.1) Per-thread monotonic count of `sh:sparql`
    /// constraint query executions (one per `sparq_engine::query_prepared` call the
    /// SHACL-SPARQL path makes). The per-focus-node path runs one query PER focus
    /// node — O(N_focus); the batched path ([`PreparedSparql::evaluate_batch`]) runs
    /// one query per ~10 000-focus chunk — O(N_focus / chunk). A perf-regression
    /// guard or an anti-vacuity test can assert the batched delta is ~O(#shapes),
    /// not O(#focus_nodes). Thread-local (not a global atomic) so a `validate` on one
    /// thread is not perturbed by a concurrent `validate` on another — the SHACL-SPARQL
    /// evaluation runs the query on the calling thread, so the count is exact there.
    /// Read via [`exec_count`].
    static SPARQL_EXEC_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Bumps the calling thread's `sh:sparql`
/// query-execution counter by one (called at each engine query the path runs).
fn bump_exec_count() {
    SPARQL_EXEC_COUNT.with(|c| c.set(c.get() + 1));
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Read the calling thread's `sh:sparql`
/// query-execution counter (see [`SPARQL_EXEC_COUNT`]). Monotonic; snapshot before
/// and after a [`crate::validate`] call on the same thread and take the delta.
pub(crate) fn exec_count() -> u64 {
    SPARQL_EXEC_COUNT.with(Cell::get)
}

/// [OPUS-4.8] (sq-7d3dj.33.1) The maximum number of focus nodes injected into a
/// single batched `VALUES ?this { … }` execution. Very large focus sets are
/// chunked so query plans stay sane; chunking never changes the result set (the
/// VALUES table partitions cleanly by `?this`, and results are grouped by `?this`
/// afterwards regardless of chunk boundaries).
const FOCUS_BATCH_CHUNK: usize = 10_000;

/// One `($name, value)` pre-binding for a validator/constraint query: the value
/// is injected as a single-row `VALUES (?name) { (value) }` table. SHACL §6.3
/// pre-binds `$this`, `$value`, each shape parameter (`$paramName`) and (on a
/// property shape) `$PATH` this way.
pub(crate) type Binding<'a> = (&'a str, &'a Term);

/// [OPUS-4.8] (sq-0mjfd) The variable a `sh:sparql` constraint query always has
/// pre-bound: `$this` (the focus node). A SPARQL-based constraint COMPONENT
/// validator additionally pre-binds `$value` and each parameter — see
/// [`check_pre_binding`], which takes the pre-bound name set.
const PRE_BOUND_THIS: &[&str] = &["this"];

/// [OPUS-4.8] (sq-0mjfd) A SHACL-SPARQL **pre-binding violation**: a `sh:sparql`
/// constraint query uses a construct for which SHACL pre-binding of `$this` is not
/// defined (W3C SHACL §5.2.1 NOTE "pre-binding"), so a conformant processor MUST
/// signal a *failure* rather than validate. Surfaced through
/// [`PreparedSparql::build`] as an `Err` and threaded to a `validate_strict`
/// failure outcome (the `mf:result sht:Failure` shacl12 entries
/// `sparql/pre-binding/{unsupported-sparql-001..006, pre-binding-006}`).
///
/// The variants are exactly the disallowed constructs the suite enumerates — no
/// more (over-rejecting would break the 17 passing `sh:sparql` entries):
///   * [`Self::Minus`]   — a `MINUS` clause (`unsupported-sparql-001`);
///   * [`Self::Values`]  — an author-written `VALUES` table (`unsupported-sparql-002`);
///   * [`Self::Service`] — a `SERVICE` clause (`unsupported-sparql-003`);
///   * [`Self::SubSelectDropsPreBound`] — a sub-`SELECT` whose projection does not
///     keep the pre-bound variable in scope, i.e. a `SELECT *` or a projection
///     list omitting it (`unsupported-sparql-004`, `pre-binding-006`);
///   * [`Self::RebindsPreBound`] — a `BIND (… AS $this)` that re-binds the
///     pre-bound variable (`unsupported-sparql-005`; the ASK form
///     `unsupported-sparql-006` re-binds `?value` and is caught the same way).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreBindingViolation {
    /// A `MINUS` clause — pre-binding is not defined across `MINUS`.
    Minus,
    /// An author-written `VALUES` table — disallowed in a pre-bound query.
    Values,
    /// A `SERVICE` (federated) clause — not evaluable under pre-binding here.
    Service,
    /// A sub-`SELECT` that re-scopes the pre-bound variable (drops it from / does
    /// not explicitly project it in the inner projection). Carries the variable
    /// name re-scoped, for the diagnostic.
    SubSelectDropsPreBound(String),
    /// A `BIND (expr AS ?v)` that re-binds an already-pre-bound variable `?v`.
    RebindsPreBound(String),
}

impl PreBindingViolation {
    /// A human-readable explanation (for a `ShapeDiagnostic` / failure message).
    pub(crate) fn message(&self) -> String {
        match self {
            PreBindingViolation::Minus => {
                "SHACL-SPARQL pre-binding is not defined across a MINUS clause".to_string()
            }
            PreBindingViolation::Values => {
                "an author-written VALUES table is not allowed in a pre-bound sh:sparql query"
                    .to_string()
            }
            PreBindingViolation::Service => {
                "a SERVICE clause cannot be evaluated under SHACL pre-binding".to_string()
            }
            // [OPUS-4.8] positional format args (rust/unused-variable CodeQL FP).
            PreBindingViolation::SubSelectDropsPreBound(v) => format!(
                "a sub-SELECT re-scopes the pre-bound variable ?{} (it must be explicitly projected)",
                v
            ),
            PreBindingViolation::RebindsPreBound(v) => {
                format!("a BIND re-binds the pre-bound variable ?{}", v)
            }
        }
    }
}

/// A parsed-and-validated `sh:sparql` constraint, ready to run per focus node.
/// Built once when the shapes model is parsed (so a malformed query is rejected
/// up front, not per focus node).
#[derive(Debug, Clone)]
pub(crate) struct PreparedSparql {
    /// The parsed `sh:select` algebra (prefixes already resolved at parse time).
    query: Query,
    /// [OPUS-4.8] (sq-7d3dj.33.1) Whether all focus nodes may be validated in ONE
    /// batched `VALUES ?this { … }` execution (see [`Self::evaluate_batch`]). Batching
    /// is result-equivalent to the per-focus path for the pre-binding-legal subset the
    /// crate already enforces (no `MINUS` / author `VALUES` / `SERVICE`; a sub-`SELECT`
    /// must project `$this`, so a NESTED aggregate necessarily groups by `$this`),
    /// EXCEPT a TOP-level solution modifier whose effect is global rather than
    /// per-focus: `LIMIT`/`OFFSET` (a global row cap), a `GROUP BY`/aggregate not keyed
    /// on `$this` (mixes foci), or `REDUCED` (implementation-defined multiplicity).
    /// Those keep the per-focus path (conservative fallback; see [`query_batchable`]).
    batchable: bool,
}

impl PreparedSparql {
    /// Parses `constraint.select` (with its prefixes prepended) into algebra.
    ///
    /// Outcome (a three-state result, distinguishing *skip* from *reject*):
    ///   * `Ok(Some(_))` — a well-formed, pre-bindable SELECT query;
    ///   * `Ok(None)` — a non-SELECT or unparsable query (ill-formed → skipped
    ///     leniently, matching the crate's ill-formed-shape policy);
    ///   * `Err(_)` — a SELECT query that VIOLATES the SHACL-SPARQL pre-binding
    ///     rules ([`PreBindingViolation`]). A conformant processor MUST signal a
    ///     *failure* for such a constraint (the `mf:result sht:Failure` shacl12
    ///     entries), so this is propagated to a strict-validation failure outcome
    ///     rather than silently skipped. [OPUS-4.8] (sq-0mjfd)
    ///
    /// `SELECT *` is accepted at the TOP level (every in-scope variable, including
    /// `$this`, is a result variable); a `SELECT *` *sub-select*, by contrast,
    /// re-scopes `$this` and is a pre-binding violation (see [`check_pre_binding`]).
    pub(crate) fn build(
        constraint: &SparqlConstraint,
    ) -> Result<Option<PreparedSparql>, PreBindingViolation> {
        let text = format!("{}\n{}", constraint.prefixes, constraint.select);
        let Ok(query) = SparqlParser::new().parse_query(&text) else {
            return Ok(None);
        };
        // Only SELECT is meaningful for sh:sparql; the solutions drive the result count.
        let Query::Select { pattern, .. } = &query else {
            return Ok(None);
        };
        // [OPUS-4.8] (sq-0mjfd) Reject a query whose pre-binding of `$this` is
        // unsound (MINUS / VALUES / SERVICE / a sub-SELECT that drops `$this` /
        // a BIND re-binding it). The check runs on the RAW author query, BEFORE
        // any pre-binding VALUES injection, so an author-written VALUES is caught.
        // The TOP-level projection is the result projection (a `SELECT *` is fine
        // here — every in-scope variable, incl. `$this`, is a result), NOT a
        // re-scoping sub-SELECT, so descend THROUGH it; the Project arm of
        // `check_pre_binding` only fires for an INNER (author) sub-SELECT.
        let body = match pattern {
            GraphPattern::Project { inner, .. } => inner.as_ref(),
            other => other,
        };
        check_pre_binding(body, PRE_BOUND_THIS)?;
        let batchable = query_batchable(pattern);
        Ok(Some(PreparedSparql { query, batchable }))
    }

    /// [OPUS-4.8] (sq-7d3dj.33.1) Whether this constraint may be evaluated for all
    /// focus nodes in ONE batched execution (see the `batchable` field). A `false`
    /// keeps the per-focus [`Self::evaluate`] path (still correct, just not sped up).
    pub(crate) fn is_batchable(&self) -> bool {
        self.batchable
    }

    /// Runs the constraint for one `focus` node against `data`, pushing one
    /// [`ValidationResult`] per solution. `make_result` lets the caller stamp the
    /// shape-owned fields (source shape/component, severity, shape messages).
    ///
    /// This is the per-focus path. [`Self::evaluate_batch`] runs the identical
    /// mapping for ALL focus nodes in ONE execution when [`Self::is_batchable`];
    /// both share [`row_to_fields`] so their reports are byte-for-byte identical.
    pub(crate) fn evaluate(
        &self,
        data: &sparq_core::Graph,
        focus: &Term,
        constraint: &SparqlConstraint,
        mut make_result: impl FnMut(ResultFields) -> ValidationResult,
        out: &mut Vec<ValidationResult>,
    ) {
        let Some(bound) = pre_bind_select(&self.query, &[("this", focus)]) else {
            return; // focus node not expressible as a VALUES ground term (unreachable today)
        };
        bump_exec_count();
        let prepared = sparq_engine::PreparedQuery::from(bound);
        let Ok(result) = sparq_engine::query_prepared(data, &prepared) else {
            return; // a runtime query error → no solutions (lenient; never panics validate())
        };
        // Map QueryResult var positions to our cached projection indices: the
        // injected query keeps the same projected variables (we add ?this to the
        // projection if it was absent), so look variables up by name to be safe.
        let pos = |name: &str| result.vars.iter().position(|v| v.as_str() == name);
        let (vpos, ppos, mpos) = (pos("value"), pos("path"), pos("message"));
        // Message {?var} substitution indexes the EXECUTED result's variables (the
        // injected query appends ?this to the projection, so positions can differ
        // from the source query's projected `self.vars`).
        let result_vars: Vec<String> = result.vars.iter().map(|v| v.as_str().to_string()).collect();

        for row in &result.rows {
            let fields = row_to_fields(focus, row, vpos, ppos, mpos, &result_vars, constraint);
            out.push(make_result(fields));
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.33.1) Runs the constraint for ALL `foci` in ONE batched
    /// execution: injects a single `VALUES ?this { <f1> <f2> … }` (chunked at
    /// [`FOCUS_BATCH_CHUNK`]), executes once per chunk, then GROUPS the solution rows
    /// by their `?this` binding to build the per-focus results. This is the O(1)-query
    /// replacement for the O(N_focus)-query per-focus loop (measured ~580× on the
    /// LUBM `sparql_constraint` workload). MUST only be called when [`Self::is_batchable`].
    ///
    /// Result-equivalence: the injected multi-row `VALUES` uses the SAME algebra-level
    /// push-down as the single-row path, and `?this` is projected, so a solution for
    /// focus `f` is exactly the per-focus solution for `f` (the natural join on the
    /// distinct-`?this` VALUES is idempotent per branch). Each result row is mapped by
    /// the SAME [`row_to_fields`] the per-focus path uses. `make_result` is invoked with
    /// the row's `?this` so the caller stamps the correct focus node.
    ///
    /// Returns a map with an entry for EVERY focus in `foci` (empty `Vec` when a focus
    /// has no violations), so the caller can distinguish "covered, no violations" from
    /// "not batched" — a focus not in the returned map (e.g. a blank node, which cannot
    /// be a `VALUES` ground term, matching [`Self::evaluate`]'s early return) is simply
    /// absent and yields no results, identical to the per-focus path.
    pub(crate) fn evaluate_batch(
        &self,
        data: &sparq_core::Graph,
        foci: &[Term],
        constraint: &SparqlConstraint,
        mut make_result: impl FnMut(&Term, ResultFields) -> ValidationResult,
    ) -> FxHashMap<Term, Vec<ValidationResult>> {
        // Every focus expressible as a VALUES ground term gets a (possibly empty)
        // entry so the caller treats it as covered; a blank-node focus is excluded
        // (no VALUES syntax) and left absent — exactly the per-focus early return.
        let mut grouped: FxHashMap<Term, Vec<ValidationResult>> = FxHashMap::default();
        let ground: Vec<&Term> = foci
            .iter()
            .filter(|f| term_to_ground(f).is_some())
            .collect();
        for f in &ground {
            grouped.entry((*f).clone()).or_default();
        }
        for chunk in ground.chunks(FOCUS_BATCH_CHUNK) {
            let Some(bound) = pre_bind_select_multi(&self.query, "this", chunk) else {
                continue; // inexpressible term in the chunk (unreachable — all ground)
            };
            bump_exec_count();
            let prepared = sparq_engine::PreparedQuery::from(bound);
            let Ok(result) = sparq_engine::query_prepared(data, &prepared) else {
                continue; // runtime query error → no solutions for this chunk (lenient)
            };
            let pos = |name: &str| result.vars.iter().position(|v| v.as_str() == name);
            let (tpos, vpos, ppos, mpos) =
                (pos("this"), pos("value"), pos("path"), pos("message"));
            let result_vars: Vec<String> =
                result.vars.iter().map(|v| v.as_str().to_string()).collect();
            for row in &result.rows {
                // The row's ?this routes it to its focus group. `?this` is always
                // projected + bound (the injected VALUES binds it), so this is Some
                // in practice; a row without it is skipped rather than mis-grouped.
                let Some(this) = tpos.and_then(|i| row.get(i)).and_then(|c| c.clone()) else {
                    continue;
                };
                let fields =
                    row_to_fields(&this, row, vpos, ppos, mpos, &result_vars, constraint);
                let vr = make_result(&this, fields);
                grouped.entry(this).or_default().push(vr);
            }
        }
        grouped
    }
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Maps one executed solution `row` to the per-solution
/// [`ResultFields`] (SHACL §5.2.2). Shared by [`PreparedSparql::evaluate`] (single
/// focus) and [`PreparedSparql::evaluate_batch`] (grouped by `?this`) so both
/// produce byte-for-byte identical reports; `focus` is the loop focus / the row's
/// `?this` respectively.
fn row_to_fields(
    focus: &Term,
    row: &[Option<Term>],
    vpos: Option<usize>,
    ppos: Option<usize>,
    mpos: Option<usize>,
    result_vars: &[String],
    constraint: &SparqlConstraint,
) -> ResultFields {
    // sh:value (SHACL §5.2.2): the solution's ?value binding when the query
    // projects ?value; otherwise the focus node itself. The focus-node default
    // (when ?value is not projected at all) is what the W3C suite expects — e.g.
    // sparql/node/sparql-003 projects only ?path and its expected sh:value is the
    // focus node. A query that DOES project ?value but leaves it unbound on a row
    // reports no sh:value (None).
    let value = match vpos {
        Some(i) => row.get(i).and_then(|c| c.clone()),
        None => Some(focus.clone()),
    };
    let path = ppos
        .and_then(|i| row.get(i))
        .and_then(|c| c.clone())
        .and_then(|t| match t {
            Term::NamedNode(n) => Some(crate::path::Path::Predicate(n.as_str().to_string())),
            _ => None,
        });
    // Message precedence: the constraint's sh:message (with {?var} substitution),
    // else a bound ?message, else a generated default.
    let row_message = mpos
        .and_then(|i| row.get(i))
        .and_then(|c| c.clone())
        .map(term_lexical);
    let default_message = match (&constraint.message, &row_message) {
        (Some(tpl), _) => substitute(tpl, result_vars, row),
        (None, Some(m)) => m.clone(),
        (None, None) => "Violates SPARQL-based constraint".to_string(),
    };
    ResultFields {
        value,
        path,
        default_message,
    }
}

/// [OPUS-4.8] (sq-7d3dj.33.1) `true` iff the query is safe to evaluate for all focus
/// nodes in ONE batched `VALUES ?this` execution (the multi-row VALUES joined into
/// the WHERE, results grouped by `?this` afterwards). Unsafe when a TOP-level
/// solution modifier applies GLOBALLY across foci instead of per-focus:
///   * `LIMIT`/`OFFSET` (`Slice`) — a global row cap;
///   * `REDUCED` — implementation-defined duplicate multiplicity (batched vs
///     per-focus could keep a different number of duplicates → different count);
///   * a TOP-level aggregate (`Group`) NOT grouped by `$this` — an implicit single
///     group (empty group vars) or a `GROUP BY ?other` mixes rows from all foci into
///     one aggregate. A `GROUP BY $this` IS per-focus-equivalent (each focus is its
///     own group), so it stays batchable. A NESTED aggregate sub-select is always
///     safe: the pre-binding check ([`check_pre_binding`]) forces every sub-`SELECT`
///     to project — hence `GROUP BY` — `$this`, so the multi-row VALUES join
///     partitions it per focus; only the TOP-level query modifiers are inspected.
///
/// `Distinct`/`OrderBy`/`Project`/`Extend`/`Filter` are transparent: order is
/// irrelevant to the violation bag; `Distinct` includes the distinct `?this` (so it
/// collapses per-focus exactly as the per-focus path); `Extend`/`Filter` are the
/// aggregate-projection / `HAVING` wrappers we descend through to reach a top-level
/// `Group`, or ordinary per-row `BIND`/`FILTER` that are per-focus-equivalent. The
/// walk stops (batchable) at the first branching/leaf node (`Join`/`Union`/`Bgp`/…),
/// because a top-level aggregate's `Group` sits directly above the WHERE with only
/// those transparent wrappers between it and the projection.
fn query_batchable(pattern: &GraphPattern) -> bool {
    match pattern {
        GraphPattern::Slice { .. } | GraphPattern::Reduced { .. } => false,
        GraphPattern::Group { variables, .. } => variables.iter().any(|v| v.as_str() == "this"),
        GraphPattern::Distinct { inner }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::Filter { inner, .. }
        | GraphPattern::Project { inner, .. } => query_batchable(inner),
        _ => true,
    }
}

/// The per-solution fields the SPARQL evaluation contributes; the caller fills in
/// the shape-owned fields (focus node, source shape/component, severity, messages).
pub(crate) struct ResultFields {
    pub value: Option<Term>,
    pub path: Option<crate::path::Path>,
    pub default_message: String,
}

/// [OPUS-4.8] (sq-rnkdh) A parsed SHACL 1.2 **SPARQL-based node expression**
/// (`sh:select` / `sh:sparqlExpr`, SHACL 1.2 §"SPARQL-based Node Expressions"):
/// a SELECT query that, run with `$this` pre-bound to a focus node, yields the
/// output nodes as the bindings of its FIRST result variable. Used by the
/// SHACL-1.2 SPARQL-valued **targets** (`sh:targetNode [ sh:select … ]`) and the
/// SPARQL-valued **value nodes** of a property shape (`sh:values [ sh:select … ]`
/// / `[ sh:sparqlExpr … ]`). Built once at shapes-model parse so a malformed
/// query is rejected up front (`None`), not per focus node.
///
/// This deliberately reuses the always-on `sh:sparql` pre-binding machinery
/// ([`pre_bind_select`] / `sparq_engine::query_prepared`), so it works in BOTH
/// the default and `shacl-af` feature states (it is SHACL-1.2 core/SPARQL, not a
/// SHACL-AF rule).
#[derive(Debug, Clone)]
pub(crate) struct PreparedSelectExpr {
    /// The parsed SELECT algebra (prefixes already resolved at build time).
    query: Query,
}

impl PreparedSelectExpr {
    /// Builds from a `sh:select` SELECT query (with its `sh:prefixes` prepended).
    /// `None` for a non-SELECT / unparsable query (ill-formed → the expression is
    /// dropped leniently, so its target/value set is empty rather than misfiring).
    pub(crate) fn build_select(prefixes: &str, select: &str) -> Option<PreparedSelectExpr> {
        let text = format!("{}\n{}", prefixes, select);
        let query = SparqlParser::new().parse_query(&text).ok()?;
        if !matches!(query, Query::Select { .. }) {
            return None;
        }
        Some(PreparedSelectExpr { query })
    }

    /// Builds from a `sh:sparqlExpr` SPARQL EXPRESSION string. The SHACL-1.2
    /// derivation wraps the expression in a single-solution projection
    /// `SELECT ((EXPR) AS ?result) WHERE {}`; running it with `$this` pre-bound
    /// evaluates the expression for the focus node and yields its value as the
    /// (single) output node. `None` if the derived query does not parse.
    pub(crate) fn build_expr(prefixes: &str, expr: &str) -> Option<PreparedSelectExpr> {
        let select = format!("SELECT (({}) AS ?result) WHERE {{ }}", expr);
        Self::build_select(prefixes, &select)
    }

    /// Evaluates the expression for `focus` against `data`, returning the output
    /// nodes (the bindings of the FIRST result variable, in solution order,
    /// duplicates preserved — the caller dedups/uses them as value nodes). An
    /// unbound first-variable solution contributes no node. A focus node not
    /// expressible as a VALUES ground term (a blank node) or a runtime query error
    /// yields an empty set (lenient; never panics `validate`).
    pub(crate) fn eval(&self, data: &sparq_core::Graph, focus: &Term) -> Vec<Term> {
        let Some(bound) = pre_bind_select(&self.query, &[("this", focus)]) else {
            return Vec::new();
        };
        Self::run(data, bound)
    }

    /// [OPUS-4.8] (sq-rnkdh) Evaluates the expression as a **target** query — with
    /// no `$this` pre-binding (a target SELECT computes the focus nodes itself) —
    /// returning the bindings of the FIRST result variable as the focus-node set.
    pub(crate) fn eval_target(&self, data: &sparq_core::Graph) -> Vec<Term> {
        Self::run(data, self.query.clone())
    }

    /// [SONNET-4.6] (sq-7d3dj.33.5) Batch-evaluates the node expression for ALL
    /// `foci` in ONE batched execution: injects a single `VALUES ?this { <f1> … }`
    /// (chunked at [`FOCUS_BATCH_CHUNK`]), executes once per chunk, then GROUPS the
    /// first-result-variable bindings by `?this` to build the per-focus value-node
    /// sets. This is the O(1)-query replacement for calling [`Self::eval`] per focus
    /// node (the O(N_focus) root cause for `sh:values` property shapes).
    ///
    /// Returns a map with an entry for EVERY expressible focus in `foci` (empty
    /// `Vec` when a focus yields no output nodes). A blank-node focus (not a VALUES
    /// ground term) is absent — identical to the empty-return of [`Self::eval`].
    pub(crate) fn eval_batch(
        &self,
        data: &sparq_core::Graph,
        foci: &[Term],
    ) -> FxHashMap<Term, Vec<Term>> {
        let mut grouped: FxHashMap<Term, Vec<Term>> = FxHashMap::default();
        let ground: Vec<&Term> = foci
            .iter()
            .filter(|f| term_to_ground(f).is_some())
            .collect();
        for f in &ground {
            grouped.entry((*f).clone()).or_default();
        }
        for chunk in ground.chunks(FOCUS_BATCH_CHUNK) {
            let Some(bound) = pre_bind_select_multi(&self.query, "this", chunk) else {
                continue; // inexpressible term in chunk (unreachable — all ground)
            };
            bump_exec_count();
            let prepared = sparq_engine::PreparedQuery::from(bound);
            let Ok(result) = sparq_engine::query_prepared(data, &prepared) else {
                continue; // runtime query error → no nodes for this chunk (lenient)
            };
            // The output nodes are the bindings of the FIRST result variable (position 0).
            // pre_bind_select_multi appends ?this to the projection when absent, so the
            // author's first projected variable stays at position 0.
            let tpos = result.vars.iter().position(|v| v.as_str() == "this");
            for row in &result.rows {
                let Some(this) = tpos.and_then(|i| row.get(i)).and_then(|c| c.clone()) else {
                    continue;
                };
                if let Some(Some(node)) = row.first() {
                    grouped.entry(this).or_default().push(node.clone());
                }
            }
        }
        grouped
    }

    /// Runs a (possibly pre-bound) SELECT and collects the FIRST-result-variable
    /// bindings, skipping unbound rows. A runtime query error yields no nodes
    /// (lenient; never panics `validate`).
    fn run(data: &sparq_core::Graph, query: Query) -> Vec<Term> {
        let prepared = sparq_engine::PreparedQuery::from(query);
        let Ok(result) = sparq_engine::query_prepared(data, &prepared) else {
            return Vec::new();
        };
        // The output nodes are the bindings of the FIRST result variable. For an
        // `eval` (pre-bound) query the injection appends `?this` to the projection
        // only if it was absent, so the author's first projected variable stays at
        // position 0.
        let mut out = Vec::new();
        for row in &result.rows {
            if let Some(Some(t)) = row.first() {
                out.push(t.clone());
            }
        }
        out
    }
}

/// [OPUS-4.8] (sq-0mjfd) Walks a SHACL-SPARQL constraint/validator WHERE algebra
/// and returns `Err` for the first construct that makes pre-binding of the
/// `pre_bound` variables unsound (W3C SHACL §5.2.1 NOTE "pre-binding"). `in_scope`
/// holds the pre-bound names still in scope along this branch; a sub-`SELECT` that
/// does not re-project a name drops it, so a re-bind *inside* a re-scoped
/// sub-select no longer concerns the OUTER pre-binding.
///
/// For a `sh:sparql` constraint `pre_bound = ["this"]`; for a SPARQL-based
/// constraint COMPONENT validator it is `["this", "value", <each parameter var>]`
/// (§6.3) — e.g. the ASK `unsupported-sparql-006` re-binds `?value`.
///
/// Detected violations (and ONLY these — over-rejecting would break the 17
/// passing `sh:sparql` entries / the component-validator suite):
///   * `Minus` — pre-binding undefined across MINUS (`unsupported-sparql-001`);
///   * `Values` — an author-written VALUES table (the pre-binding injection itself
///     runs LATER, on a separately-built copy, so any VALUES seen here is the
///     author's) (`unsupported-sparql-002`);
///   * `Service` — a federated clause (`unsupported-sparql-003`);
///   * a sub-`SELECT` `Project` that does not keep an in-scope pre-bound name in
///     scope (a `SELECT *` — empty projection list in this algebra — or a list
///     omitting it) (`unsupported-sparql-004`, `pre-binding-006`); the descent
///     then continues with that name out of scope;
///   * a `BIND (… AS ?v)` (`Extend`) re-binding an in-scope pre-bound name `?v`
///     (`unsupported-sparql-005`, ASK `unsupported-sparql-006`). A
///     `BIND ($this AS ?other)` binding a NEW name is fine (`pre-binding-004`).
fn check_pre_binding(
    pattern: &GraphPattern,
    in_scope: &[&str],
) -> Result<(), PreBindingViolation> {
    match pattern {
        GraphPattern::Minus { .. } => Err(PreBindingViolation::Minus),
        GraphPattern::Values { .. } => Err(PreBindingViolation::Values),
        GraphPattern::Service { .. } => Err(PreBindingViolation::Service),
        GraphPattern::Extend {
            inner,
            variable,
            expression: _,
        } => {
            if in_scope.contains(&variable.as_str()) {
                return Err(PreBindingViolation::RebindsPreBound(
                    variable.as_str().to_string(),
                ));
            }
            check_pre_binding(inner, in_scope)
        }
        // A sub-SELECT (`Project`) re-scopes its inner: a pre-bound name survives
        // into it ONLY if the projection explicitly lists it. A `SELECT *` (empty
        // list in this algebra) or a list omitting it re-scopes it — a violation
        // while it is still in scope at this point.
        GraphPattern::Project { inner, variables } => {
            let kept: Vec<&str> = in_scope
                .iter()
                .copied()
                .filter(|n| variables.iter().any(|v| v.as_str() == *n))
                .collect();
            if let Some(dropped) = in_scope.iter().find(|n| !kept.contains(n)) {
                return Err(PreBindingViolation::SubSelectDropsPreBound(
                    (*dropped).to_string(),
                ));
            }
            check_pre_binding(inner, &kept)
        }
        // Single-child wrappers keep the outer scope; descend.
        GraphPattern::Filter { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Graph { inner, .. } => check_pre_binding(inner, in_scope),
        // Both children share the outer scope (a UNION branch / JOIN sibling / the
        // optional side of an OPTIONAL each keeps the pre-bound names in scope).
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::LeftJoin { left, right, .. } => {
            check_pre_binding(left, in_scope)?;
            check_pre_binding(right, in_scope)
        }
        // Leaves carry no pre-binding-affecting construct.
        GraphPattern::Bgp { .. } | GraphPattern::Path { .. } => Ok(()),
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

/// Builds the single-row `VALUES (?n1 ?n2 …) { (v1 v2 …) }` table that pre-binds
/// each `(name, term)` in `bindings`. Returns `None` if any value is not
/// expressible as a SPARQL ground term (e.g. a blank node — see `term_to_ground`).
fn pre_bind_values(bindings: &[Binding]) -> Option<GraphPattern> {
    let mut variables = Vec::with_capacity(bindings.len());
    let mut row = Vec::with_capacity(bindings.len());
    for (name, term) in bindings {
        variables.push(Variable::new_unchecked(*name));
        row.push(Some(term_to_ground(term)?));
    }
    Some(GraphPattern::Values {
        variables,
        bindings: vec![row],
    })
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Builds the MULTI-row `VALUES (?name) { (v1) (v2) … }`
/// table that pre-binds `name` to EACH term in `foci` (one row per focus), for the
/// batched `sh:sparql` path. Single column, N rows — as opposed to
/// [`pre_bind_values`]'s single-row/N-column shape. Returns `None` if any focus is
/// not a SPARQL ground term (all callers pre-filter to ground terms).
fn pre_bind_values_multi(name: &str, foci: &[&Term]) -> Option<GraphPattern> {
    let variables = vec![Variable::new_unchecked(name)];
    let mut bindings = Vec::with_capacity(foci.len());
    for f in foci {
        bindings.push(vec![Some(term_to_ground(f)?)]);
    }
    Some(GraphPattern::Values {
        variables,
        bindings,
    })
}

/// [OPUS-4.8] (sq-7d3dj.33.1) Pre-binds `name` to EVERY term in `foci` at once by
/// injecting the multi-row `VALUES` from [`pre_bind_values_multi`] through the SAME
/// [`inject_values`] push-down the single-focus [`pre_bind_select`] uses, so the
/// batched query is the per-focus query unioned over foci. `name` is added to the
/// projection so the executor returns it (used to group solutions by focus). The
/// batched result set is exactly the per-focus results' union because the natural
/// join on the distinct-`?this` VALUES is idempotent per branch.
fn pre_bind_select_multi(query: &Query, name: &str, foci: &[&Term]) -> Option<Query> {
    let Query::Select {
        dataset,
        pattern,
        base_iri,
    } = query
    else {
        return None;
    };
    let values = pre_bind_values_multi(name, foci)?;
    Some(Query::Select {
        dataset: dataset.clone(),
        pattern: inject_values(pattern, values, &[name]),
        base_iri: base_iri.clone(),
    })
}

/// Pre-binds each `(name, value)` in `bindings` into a SELECT query by injecting a
/// single-row `VALUES` table joined under the projection, and ensures every bound
/// name is itself projected (so a consumer reading `?this` / `?value` back gets
/// the pre-bound term). Returns `None` for a non-SELECT query or an inexpressible
/// value. SHACL §5.2 (`$this`) and §6.3 (`$this`/`$value`/`$paramName`).
fn pre_bind_select(query: &Query, bindings: &[Binding]) -> Option<Query> {
    let Query::Select {
        dataset,
        pattern,
        base_iri,
    } = query
    else {
        return None;
    };
    let values = pre_bind_values(bindings)?;
    let names: Vec<&str> = bindings.iter().map(|(n, _)| *n).collect();
    Some(Query::Select {
        dataset: dataset.clone(),
        pattern: inject_values(pattern, values, &names),
        base_iri: base_iri.clone(),
    })
}

/// Pre-binds `bindings` into an ASK query's WHERE pattern (no projection to
/// extend) and returns the boolean. A SELECT-validator never reaches here.
/// `None` for a non-ASK query or an inexpressible value.
fn pre_bind_ask(query: &Query, bindings: &[Binding]) -> Option<Query> {
    let Query::Ask {
        dataset,
        pattern,
        base_iri,
    } = query
    else {
        return None;
    };
    let values = pre_bind_values(bindings)?;
    // An ASK's WHERE is wrapped in a (zero-variable) `Project`; descend through it
    // so the VALUES lands inside, not above (a `Project` is otherwise a sub-SELECT
    // boundary `push_values_down` must NOT cross).
    let bound_pattern = match pattern {
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: Box::new(push_values_down(inner, &values)),
            variables: variables.clone(),
        },
        other => push_values_down(other, &values),
    };
    Some(Query::Ask {
        dataset: dataset.clone(),
        pattern: bound_pattern,
        base_iri: base_iri.clone(),
    })
}

/// The variable names a pre-binding `values` table binds (each pushed-down
/// VALUES table is the single-row one built by [`pre_bind_values`]). Used to
/// decide whether a sub-SELECT keeps a pre-bound variable in scope (it must be
/// projected) — Union/Join siblings are descended unconditionally because the
/// single-row VALUES join is idempotent for a branch that does not mention it.
fn values_var_names(values: &GraphPattern) -> Vec<&str> {
    match values {
        GraphPattern::Values { variables, .. } => variables.iter().map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    }
}

/// Pushes the pre-binding `values` table *down* through algebra wrappers, joining
/// it at every leaf where the pre-bound variables are in scope. This is
/// load-bearing for FILTER-only patterns like `ASK { FILTER(?value = $param) }`: a
/// VALUES table joined ABOVE the `Filter` leaves `?value`/`$param` UNBOUND inside
/// the filter (the filter scopes only its own inner), so the filter evaluates over
/// empty bindings and the constraint never holds. Joining the VALUES table BELOW
/// the filter puts the pre-bound terms in scope. Robust to however the validator
/// query is laid out (no textual prepend). SHACL §6.3 pre-binding semantics
/// (substitute the variable's value everywhere it occurs).
///
/// [OPUS-4.8] (sq-mue75) The descent now follows EVERY scope a pre-bound variable
/// can reach, not just one chain, so the binding constrains the whole pattern
/// (the shacl12 `sparql/pre-binding` 002/005/007 entries):
///   * single-child wrappers (`Filter` / `Extend` / `OrderBy` / `Group` /
///     `Distinct` / `Reduced` / `Slice` / `Graph`) — descend into the inner;
///   * `Minus`-left / `LeftJoin`-left — bind the REQUIRED left only (the
///     excluded / optional right keeps SHACL substitution semantics);
///   * `Union` — push into BOTH branches independently, so a pre-bound variable
///     used only inside one branch's FILTER is in scope there (pre-binding-002);
///   * `Join` — push into BOTH siblings, so a sibling sub-block that wraps a
///     FILTER over an empty inner (`{ FILTER(bound($this)) }`) sees the binding
///     (pre-binding-005); the single-row VALUES join is idempotent for a sibling
///     that does not mention the variable;
///   * sub-`SELECT` `Project` — descend through the projection ONLY when every
///     pre-bound variable is explicitly projected (so it stays in scope inside the
///     sub-select, pre-binding-007); a `SELECT *` (an empty projection list in this
///     algebra) or a projection that drops a pre-bound variable re-scopes it, so
///     the table is joined ABOVE the sub-select instead (the variable is then
///     out of scope inside — consistent with the spec rejecting that re-binding,
///     e.g. the `sht:Failure` pre-binding-006).
fn push_values_down(pattern: &GraphPattern, values: &GraphPattern) -> GraphPattern {
    match pattern {
        GraphPattern::Filter { expr, inner } => GraphPattern::Filter {
            expr: expr.clone(),
            inner: Box::new(push_values_down(inner, values)),
        },
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => GraphPattern::Extend {
            inner: Box::new(push_values_down(inner, values)),
            variable: variable.clone(),
            expression: expression.clone(),
        },
        GraphPattern::OrderBy { inner, expression } => GraphPattern::OrderBy {
            inner: Box::new(push_values_down(inner, values)),
            expression: expression.clone(),
        },
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: Box::new(push_values_down(inner, values)),
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: Box::new(push_values_down(inner, values)),
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: Box::new(push_values_down(inner, values)),
            start: *start,
            length: *length,
        },
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => GraphPattern::Group {
            inner: Box::new(push_values_down(inner, values)),
            variables: variables.clone(),
            aggregates: aggregates.clone(),
        },
        // A GRAPH block keeps the outer pre-bound variables in scope; descend.
        GraphPattern::Graph { name, inner } => GraphPattern::Graph {
            name: name.clone(),
            inner: Box::new(push_values_down(inner, values)),
        },
        // For a left-join / minus, bind on the (required) left side only.
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => GraphPattern::LeftJoin {
            left: Box::new(push_values_down(left, values)),
            right: right.clone(),
            expression: expression.clone(),
        },
        GraphPattern::Minus { left, right } => GraphPattern::Minus {
            left: Box::new(push_values_down(left, values)),
            right: right.clone(),
        },
        // [OPUS-4.8] (sq-mue75) UNION: each branch is an independent scope; push
        // the pre-binding into BOTH so a variable used only inside one branch's
        // FILTER is bound there (shacl12 sparql/pre-binding-002).
        GraphPattern::Union { left, right } => GraphPattern::Union {
            left: Box::new(push_values_down(left, values)),
            right: Box::new(push_values_down(right, values)),
        },
        // [OPUS-4.8] (sq-mue75) JOIN: push into BOTH siblings so a sibling block
        // that wraps a FILTER over an empty inner still sees the binding (shacl12
        // sparql/pre-binding-005). Joining the single-row VALUES into a sibling
        // that does not mention the variable is idempotent (a cross product with
        // one deterministic row).
        GraphPattern::Join { left, right } => GraphPattern::Join {
            left: Box::new(push_values_down(left, values)),
            right: Box::new(push_values_down(right, values)),
        },
        // [OPUS-4.8] (sq-mue75) sub-SELECT projection: a pre-bound variable stays
        // in scope inside the sub-select ONLY if it is explicitly projected, so
        // descend through the Project (and keep it projected) in that case; else
        // join the VALUES ABOVE the sub-select (fall through to the leaf arm).
        GraphPattern::Project { inner, variables }
            if projects_all(variables, &values_var_names(values)) =>
        {
            let mut projected = variables.clone();
            for name in values_var_names(values) {
                if !projected.iter().any(|v| v.as_str() == name) {
                    projected.push(Variable::new_unchecked(name));
                }
            }
            GraphPattern::Project {
                inner: Box::new(push_values_down(inner, values)),
                variables: projected,
            }
        }
        // A leaf (BGP / Path / Values / a sub-SELECT that re-scopes a pre-bound
        // variable / …): join the VALUES table here so its bindings flow up into
        // every parent.
        other => GraphPattern::Join {
            left: Box::new(values.clone()),
            right: Box::new(other.clone()),
        },
    }
}

/// `true` iff `projected` explicitly lists EVERY name in `names` (so a sub-SELECT
/// keeps those pre-bound variables in scope inside it). An empty `projected` is a
/// `SELECT *` in this algebra — treated as NOT explicitly projecting (so a
/// pre-bound variable is re-scoped, the spec-rejection case, e.g. the
/// `sht:Failure` pre-binding-006); an empty `names` (nothing pre-bound) trivially
/// holds but never reaches a Project leaf in practice.
fn projects_all(projected: &[Variable], names: &[&str]) -> bool {
    if names.is_empty() {
        return true;
    }
    if projected.is_empty() {
        return false; // SELECT * — not an explicit projection of the pre-bound var
    }
    names
        .iter()
        .all(|n| projected.iter().any(|v| v.as_str() == *n))
}

/// Joins `values` into a SELECT pattern at the right depth: it descends through
/// the solution-modifier wrappers (`Distinct`/`Reduced`/`Slice`/`OrderBy`) so the
/// VALUES table lands *below* them (a `LIMIT`/`DISTINCT` must apply to the
/// pre-bound results, not the other way around), then joins it under the
/// `Project`'s inner and adds every pre-bound `names` variable to the projection
/// if absent. If there is no `Project` (`SELECT *`), it joins at that point — the
/// pre-bound variables surface regardless.
fn inject_values(pattern: &GraphPattern, values: GraphPattern, names: &[&str]) -> GraphPattern {
    match pattern {
        GraphPattern::Project { inner, variables } => {
            let mut variables = variables.clone();
            for name in names {
                if !variables.iter().any(|v| v.as_str() == *name) {
                    variables.push(Variable::new_unchecked(*name));
                }
            }
            // Push the VALUES down through the WHERE pattern so a pre-bound
            // variable used only inside a FILTER/BIND is still in scope there.
            GraphPattern::Project {
                inner: Box::new(push_values_down(inner, &values)),
                variables,
            }
        }
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: Box::new(inject_values(inner, values, names)),
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: Box::new(inject_values(inner, values, names)),
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: Box::new(inject_values(inner, values, names)),
            start: *start,
            length: *length,
        },
        GraphPattern::OrderBy { inner, expression } => GraphPattern::OrderBy {
            inner: Box::new(inject_values(inner, values, names)),
            expression: expression.clone(),
        },
        // No projection wrapper (SELECT *): push down; the bound vars surface regardless.
        other => push_values_down(other, &values),
    }
}

/// An RDF term as a SPARQL `GroundTerm` for a VALUES row. Variables/triple-terms
/// that cannot appear in VALUES yield `None` (focus nodes are IRIs/blank nodes in
/// practice, both of which are expressible).
fn term_to_ground(t: &Term) -> Option<GroundTerm> {
    match t {
        Term::NamedNode(n) => Some(GroundTerm::NamedNode(n.clone())),
        Term::Literal(l) => Some(GroundTerm::Literal(l.clone())),
        // A blank node cannot be written in a VALUES table (no syntax); SHACL focus
        // nodes are rarely blank, and when they are this constraint is skipped
        // rather than mis-evaluated. (oxrdf BlankNode has no GroundTerm form.)
        Term::BlankNode(_) => None,
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// The lexical string a bound term contributes to a `?message` / `{?var}` slot.
fn term_lexical(t: Term) -> String {
    match t {
        Term::NamedNode(n) => n.into_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        #[allow(unreachable_patterns)]
        other => other.to_string(),
    }
}

/// Substitutes `{?var}` / `{$var}` placeholders in a SHACL message template with
/// the solution's bindings (SHACL §5.2.2 message templates). Unbound or unknown
/// variables are left as the literal placeholder text.
fn substitute(template: &str, vars: &[String], row: &[Option<Term>]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let token = after[..close].trim();
            let name = token.strip_prefix(['?', '$']);
            let replaced = name.and_then(|n| {
                vars.iter()
                    .position(|v| v == n)
                    .and_then(|i| row.get(i))
                    .and_then(|c| c.clone())
                    .map(term_lexical)
            });
            match replaced {
                Some(s) => out.push_str(&s),
                None => {
                    // Not a known {?var}: keep it verbatim.
                    out.push('{');
                    out.push_str(&after[..close]);
                    out.push('}');
                }
            }
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

// =============================================================================
// [OPUS-4.8] SHACL §6 — SPARQL-based constraint COMPONENTS (sq-sm2).
//
// A `sh:ConstraintComponent` declares parameters (`sh:parameter` → `sh:path`)
// and a validator (`sh:validator` / `sh:nodeValidator` / `sh:propertyValidator`)
// carrying a `sh:ask` or `sh:select` query. When a shape uses the component's
// parameter predicates, the component activates: the parameter VALUES, `$this`,
// `$value` and (on a property shape) `$PATH` are pre-bound, and the validator
// runs — reusing the same VALUES-injection pre-binding + solution→result mapping
// as the §5.2 `sh:sparql` path above.
// =============================================================================

/// A compiled component validator: either an ASK query (run per value node;
/// ASK=false → one violation) or a SELECT query (run per focus node; each
/// solution → one violation, §5.2 mapping). Built once when the shapes model is
/// parsed (so a malformed query is rejected up front, not per focus node).
#[derive(Debug, Clone)]
pub(crate) enum PreparedValidator {
    Ask(Query),
    Select(Query),
}

impl PreparedValidator {
    /// Parses a validator's query text (prefixes already prepended): an ASK form
    /// is an [`Ask`](Self::Ask), a SELECT form a [`Select`](Self::Select), any
    /// other form (or an unparsable query) is `None` (ill-formed → skipped).
    pub(crate) fn build(text: &str, is_ask: bool) -> Option<PreparedValidator> {
        let query = SparqlParser::new().parse_query(text).ok()?;
        match (&query, is_ask) {
            (Query::Ask { .. }, true) => Some(PreparedValidator::Ask(query)),
            (Query::Select { .. }, false) => Some(PreparedValidator::Select(query)),
            _ => None,
        }
    }
}

/// [OPUS-4.8] (sq-0mjfd) Checks a SPARQL-based constraint-COMPONENT validator's
/// query text for a pre-binding violation (§6.3 pre-binds `$this`, `$value` and
/// each parameter `pre_bound`). Returns `Ok(())` when sound or when the text does
/// not parse / is not an ASK/SELECT (those are handled leniently elsewhere), and
/// `Err` only for a genuine pre-binding violation — e.g. the ASK
/// `unsupported-sparql-006` re-binds `?value`.
pub(crate) fn check_validator_pre_binding(
    text: &str,
    pre_bound: &[&str],
) -> Result<(), PreBindingViolation> {
    let Ok(query) = SparqlParser::new().parse_query(text) else {
        return Ok(());
    };
    match &query {
        // An ASK's WHERE is wrapped in a (zero-variable) top-level `Project` — that
        // is the ASK form itself, NOT an author sub-SELECT, so descend THROUGH it
        // (else the empty projection would be misread as dropping every pre-bound
        // variable). The pre-bound names stay in scope inside the ASK body.
        Query::Ask { pattern, .. } => match pattern {
            GraphPattern::Project { inner, .. } => check_pre_binding(inner, pre_bound),
            other => check_pre_binding(other, pre_bound),
        },
        Query::Select { pattern, .. } => check_pre_binding(pattern, pre_bound),
        _ => Ok(()),
    }
}

/// Substitutes `{?name}` / `{$name}` placeholders in a validator `sh:message`
/// with the given `bindings` (the pre-bound `$this` / `$value` / `$paramName`
/// terms). Used by the ASK-validator path, which has no solution row to draw
/// `{?var}` values from (SHACL §6.3 message templates over pre-bound params).
pub(crate) fn substitute_bindings(template: &str, bindings: &[Binding]) -> String {
    let names: Vec<String> = bindings.iter().map(|(n, _)| (*n).to_string()).collect();
    let row: Vec<Option<Term>> = bindings.iter().map(|(_, t)| Some((*t).clone())).collect();
    substitute(template, &names, &row)
}

/// The shape-side fields a custom-component result inherits (filled by `eval.rs`).
pub(crate) struct ComponentResultFields {
    pub focus: Term,
    pub value: Option<Term>,
    pub path: Option<crate::path::Path>,
    pub default_message: String,
}

/// Runs an ASK validator for one `value` node (SHACL §6.3): pre-binds `$this`,
/// `$value`, the shape's parameter VALUES and (on a property shape) `$PATH`, and
/// returns `true` when the value VIOLATES the constraint (ASK = false). A query
/// that fails to pre-bind (inexpressible value) or errors at runtime is treated
/// as conforming (lenient — never panics validate()).
pub(crate) fn ask_violates(
    query: &Query,
    data: &sparq_core::Graph,
    bindings: &[Binding],
) -> bool {
    let Some(bound) = pre_bind_ask(query, bindings) else {
        return false;
    };
    let prepared = sparq_engine::PreparedQuery::from(bound);
    match sparq_engine::ask_prepared(data, &prepared) {
        Ok(holds) => !holds, // ASK true = conforms; ASK false = violation
        Err(_) => false,
    }
}

/// Runs a SELECT validator for one `focus` node (SHACL §6.3): each returned
/// solution is one violation, mapped exactly as the §5.2 `sh:sparql` path
/// (`?value`/`?path`/`?message` → result fields, with `{?var}` message
/// substitution). `bindings` carries `$this`, the parameter VALUES and (on a
/// property shape) `$PATH`. `message` is the component's `sh:message`, if any.
/// [SONNET-4.6] (sq-ou3) `label_template` is the component's `sh:labelTemplate`
/// with its PARAMETER placeholders already rendered by the caller — used as the
/// message only when neither `message` nor the solution's `?message` supplies one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_validate(
    query: &Query,
    data: &sparq_core::Graph,
    focus: &Term,
    bindings: &[Binding],
    message: Option<&str>,
    label_template: Option<&str>,
    mut make_result: impl FnMut(ComponentResultFields) -> ValidationResult,
    out: &mut Vec<ValidationResult>,
) {
    let Some(bound) = pre_bind_select(query, bindings) else {
        return;
    };
    let prepared = sparq_engine::PreparedQuery::from(bound);
    let Ok(result) = sparq_engine::query_prepared(data, &prepared) else {
        return;
    };
    let pos = |name: &str| result.vars.iter().position(|v| v.as_str() == name);
    let (vpos, ppos, mpos) = (pos("value"), pos("path"), pos("message"));
    let result_vars: Vec<String> = result.vars.iter().map(|v| v.as_str().to_string()).collect();
    for row in &result.rows {
        let value = match vpos {
            Some(i) => row.get(i).and_then(|c| c.clone()),
            None => Some(focus.clone()),
        };
        let path = ppos
            .and_then(|i| row.get(i))
            .and_then(|c| c.clone())
            .and_then(|t| match t {
                Term::NamedNode(n) => Some(crate::path::Path::Predicate(n.as_str().to_string())),
                _ => None,
            });
        let row_message = mpos
            .and_then(|i| row.get(i))
            .and_then(|c| c.clone())
            .map(term_lexical);
        let default_message = match (message, &row_message, label_template) {
            (Some(tpl), _, _) => substitute(tpl, &result_vars, row),
            (None, Some(m), _) => m.clone(),
            // [SONNET-4.6] (sq-ou3) `sh:labelTemplate` is the LAST resort — below
            // the validator's `sh:message` and the solution's own `?message`. Its
            // parameter placeholders are already rendered by the caller; this pass
            // resolves any `{?var}` naming a projected solution variable.
            (None, None, Some(label)) => substitute(label, &result_vars, row),
            (None, None, None) => "Violates SPARQL-based constraint".to_string(),
        };
        out.push(make_result(ComponentResultFields {
            focus: focus.clone(),
            value,
            path,
            default_message,
        }));
    }
}

// =============================================================================
// [OPUS-4.8] 🤖 SPARQ agent — sq-qcnn.1 (epic sq-qcnn test-quality program).
//
// Correctness coverage for the DARK branches of the SHACL-SPARQL pre-binding
// (`sh:sparql` §5.2 and the §6 component validators): the deep-algebra arms of
// `push_values_down` (Group / Slice / Distinct / Reduced / OrderBy / Minus) and
// the fail-closed error paths of `ask_violates` / `select_validate` /
// `PreparedSparql::evaluate` / `pre_bind_ask` / `PreparedValidator::build`.
//
// Two complementary layers:
//   * STRUCTURAL — drive `push_values_down` directly over a hand-shaped
//     `Modifier { Filter { Bgp } }` algebra and assert the load-bearing
//     invariant: the pre-binding VALUES table is JOINED at the deepest leaf
//     (BELOW the modifier and the FILTER), with the modifier wrapper preserved
//     unchanged. This is the real production recursion (not a mock).
//   * SEMANTIC — run a real `sh:select` / `sh:ask` validator whose WHERE wraps
//     the FILTER in each solution-modifier shape against real data, and assert
//     the conforms/violation count derived BY HAND from SHACL §5.2 / §6.3 (the
//     pre-bound `$this` / `$value` / `$param` stays in scope through the
//     modifier, so the constraint evaluates correctly).
//
// Test-only: no behaviour change. (The provably-equivalent `func.rs:318` mutant
// noted on sq-qcnn.1 is in a different module and is deliberately not chased.)
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode};
    use spargebra::algebra::GraphPattern;
    use sparq_core::Graph;

    /// Parse a SELECT and return its top-level WHERE pattern.
    fn select_pattern(q: &str) -> GraphPattern {
        match SparqlParser::new().parse_query(q).unwrap() {
            Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {:?}", other),
        }
    }

    /// A `Filter { Bgp }` leaf to nest under a solution modifier — exactly the
    /// shape an authored `{ ?this :p ?v . FILTER(?v > 0) }` produces.
    fn filter_bgp() -> GraphPattern {
        // `SELECT * { ?this :p ?v . FILTER(?v > 0) }` => Project { Filter { Bgp } };
        // unwrap the Project to get the inner Filter { Bgp }.
        match select_pattern("SELECT * WHERE { ?this <http://x/p> ?v . FILTER(?v > 0) }") {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        }
    }

    /// A single-row VALUES table binding `?this` to an IRI (the pre-binding the
    /// production code injects).
    fn values_this() -> GraphPattern {
        let term = Term::NamedNode(NamedNode::new("http://example.org/n").unwrap());
        pre_bind_values(&[("this", &term)]).unwrap()
    }

    /// Assert that `pushed` is `Wrapper -> … -> Filter { Join { Values, Bgp } }`:
    /// the modifier wrapper(s) are preserved above, and the VALUES table is joined
    /// at the deepest leaf (BELOW the FILTER). Walks past the modifier shells the
    /// caller named (`wrappers`), then a Filter, then the Join.
    fn assert_values_at_leaf(pushed: &GraphPattern, descend_filter: bool) {
        // Descend any chain down to the Join that wraps Values + the BGP leaf.
        fn find_values_join(p: &GraphPattern) -> bool {
            match p {
                GraphPattern::Join { left, right } => {
                    matches!(**left, GraphPattern::Values { .. })
                        && matches!(**right, GraphPattern::Bgp { .. })
                        || find_values_join(left)
                        || find_values_join(right)
                }
                GraphPattern::Filter { inner, .. }
                | GraphPattern::Extend { inner, .. }
                | GraphPattern::OrderBy { inner, .. }
                | GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. }
                | GraphPattern::Group { inner, .. } => find_values_join(inner),
                GraphPattern::Minus { left, .. } | GraphPattern::LeftJoin { left, .. } => {
                    find_values_join(left)
                }
                _ => false,
            }
        }
        assert!(
            find_values_join(pushed),
            "VALUES table not joined at the deepest leaf: {:?}",
            pushed
        );
        if descend_filter {
            // The FILTER must STILL wrap the join (the pre-bound vars are in scope
            // inside it) — i.e. there is a Filter somewhere above the Values join.
            fn has_filter_above_join(p: &GraphPattern) -> bool {
                match p {
                    GraphPattern::Filter { inner, .. } => {
                        find_values_join_inner(inner) || has_filter_above_join(inner)
                    }
                    GraphPattern::Extend { inner, .. }
                    | GraphPattern::OrderBy { inner, .. }
                    | GraphPattern::Distinct { inner }
                    | GraphPattern::Reduced { inner }
                    | GraphPattern::Slice { inner, .. }
                    | GraphPattern::Group { inner, .. } => has_filter_above_join(inner),
                    GraphPattern::Minus { left, .. } | GraphPattern::LeftJoin { left, .. } => {
                        has_filter_above_join(left)
                    }
                    _ => false,
                }
            }
            fn find_values_join_inner(p: &GraphPattern) -> bool {
                match p {
                    GraphPattern::Join { left, .. } => {
                        matches!(**left, GraphPattern::Values { .. })
                    }
                    GraphPattern::Filter { inner, .. } => find_values_join_inner(inner),
                    _ => false,
                }
            }
            assert!(
                has_filter_above_join(pushed),
                "FILTER no longer wraps the pre-binding join: {:?}",
                pushed
            );
        }
    }

    // --- push_values_down: deep-algebra arms (structural invariant) ----------

    #[test]
    fn push_values_down_through_group() {
        let inner = GraphPattern::Group {
            inner: Box::new(filter_bgp()),
            variables: vec![Variable::new_unchecked("this")],
            aggregates: vec![],
        };
        let out = push_values_down(&inner, &values_this());
        // Group preserved on top; VALUES joined below the FILTER at the BGP leaf.
        assert!(matches!(out, GraphPattern::Group { .. }), "{:?}", out);
        assert_values_at_leaf(&out, true);
    }

    #[test]
    fn push_values_down_through_slice() {
        let inner = GraphPattern::Slice {
            inner: Box::new(filter_bgp()),
            start: 1,
            length: Some(5),
        };
        let out = push_values_down(&inner, &values_this());
        // The Slice (LIMIT/OFFSET) MUST stay above the pre-binding join.
        match &out {
            GraphPattern::Slice { start, length, .. } => {
                assert_eq!(*start, 1);
                assert_eq!(*length, Some(5));
            }
            other => panic!("expected Slice, got {:?}", other),
        }
        assert_values_at_leaf(&out, true);
    }

    #[test]
    fn push_values_down_through_distinct() {
        let inner = GraphPattern::Distinct {
            inner: Box::new(filter_bgp()),
        };
        let out = push_values_down(&inner, &values_this());
        assert!(matches!(out, GraphPattern::Distinct { .. }), "{:?}", out);
        assert_values_at_leaf(&out, true);
    }

    #[test]
    fn push_values_down_through_reduced() {
        let inner = GraphPattern::Reduced {
            inner: Box::new(filter_bgp()),
        };
        let out = push_values_down(&inner, &values_this());
        assert!(matches!(out, GraphPattern::Reduced { .. }), "{:?}", out);
        assert_values_at_leaf(&out, true);
    }

    #[test]
    fn push_values_down_through_order_by() {
        let inner = match select_pattern(
            "SELECT * WHERE { ?this <http://x/p> ?v . FILTER(?v > 0) } ORDER BY ?v",
        ) {
            // `Project { OrderBy { Filter { Bgp } } }` => take the OrderBy inner.
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::OrderBy { .. }));
        let out = push_values_down(&inner, &values_this());
        assert!(matches!(out, GraphPattern::OrderBy { .. }), "{:?}", out);
        assert_values_at_leaf(&out, true);
    }

    #[test]
    fn push_values_down_through_minus_binds_left_only() {
        // `Minus { left, right }` => VALUES joins into the (required) LEFT only;
        // the RIGHT (the excluded set) is left untouched (SHACL §6.3: pre-bind the
        // value everywhere it occurs in the REQUIRED pattern).
        let inner = match select_pattern(
            "SELECT * WHERE { ?this <http://x/p> ?v MINUS { ?this <http://x/q> ?w } }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::Minus { .. }));
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::Minus { left, right } => {
                // Left now joins the VALUES table; right is unchanged (still a Bgp).
                assert!(
                    matches!(
                        &**left,
                        GraphPattern::Join { left: jl, .. } if matches!(**jl, GraphPattern::Values { .. })
                    ),
                    "MINUS left should join VALUES, got {:?}",
                    left
                );
                assert!(
                    matches!(&**right, GraphPattern::Bgp { .. }),
                    "MINUS right must be untouched, got {:?}",
                    right
                );
            }
            other => panic!("expected Minus, got {:?}", other),
        }
    }

    #[test]
    fn push_values_down_left_join_binds_left_only() {
        // OPTIONAL => LeftJoin; the pre-binding joins the required LEFT only.
        let inner = match select_pattern(
            "SELECT * WHERE { ?this <http://x/p> ?v OPTIONAL { ?this <http://x/q> ?w } }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::LeftJoin { .. }));
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::LeftJoin { left, .. } => assert!(
                matches!(
                    &**left,
                    GraphPattern::Join { left: jl, .. } if matches!(**jl, GraphPattern::Values { .. })
                ),
                "LeftJoin left should join VALUES, got {:?}",
                left
            ),
            other => panic!("expected LeftJoin, got {:?}", other),
        }
    }

    // --- [OPUS-4.8] (sq-mue75) push_values_down: the new multi-scope arms --------
    //
    // UNION / sibling-JOIN / sub-SELECT projection. Each pre-bound variable must
    // reach EVERY scope it can appear in (shacl12 sparql/pre-binding-002/005/007),
    // not just one descent chain. Structural assertions over the real recursion.

    /// Is there a `Join { Values, … }` directly under `p` (the pre-binding join)?
    fn values_join_here(p: &GraphPattern) -> bool {
        matches!(p, GraphPattern::Join { left, .. } if matches!(**left, GraphPattern::Values { .. }))
    }

    /// Does some node in `p`'s subtree wrap the FILTER over a `Join { Values, … }`
    /// (i.e. the pre-binding is in scope INSIDE the filter)?
    fn filter_over_values_join(p: &GraphPattern) -> bool {
        match p {
            GraphPattern::Filter { inner, .. } => {
                values_join_here(inner) || filter_over_values_join(inner)
            }
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right }
            | GraphPattern::LeftJoin { left, right, .. } => {
                filter_over_values_join(left) || filter_over_values_join(right)
            }
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Group { inner, .. }
            | GraphPattern::Extend { inner, .. } => filter_over_values_join(inner),
            _ => false,
        }
    }

    #[test]
    fn push_values_down_into_both_union_branches() {
        // pre-binding-002: `{ FILTER(false) } UNION { FILTER($this = <x>) }`.
        // The VALUES must be joined under BOTH branches' FILTER so $this is in
        // scope inside each (joining ABOVE the Union leaves it unbound there).
        let inner = match select_pattern(
            "SELECT $this WHERE { { FILTER(false) } UNION { FILTER(?this = <http://x/n>) } }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::Union { .. }), "{:?}", inner);
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::Union { left, right } => {
                // Each branch is `Filter { Join { Values, Bgp } }`.
                assert!(
                    matches!(&**left, GraphPattern::Filter { inner, .. } if values_join_here(inner)),
                    "UNION left branch missing pre-binding: {:?}",
                    left
                );
                assert!(
                    matches!(&**right, GraphPattern::Filter { inner, .. } if values_join_here(inner)),
                    "UNION right branch missing pre-binding: {:?}",
                    right
                );
            }
            other => panic!("expected Union, got {:?}", other),
        }
    }

    #[test]
    fn push_values_down_into_join_siblings() {
        // pre-binding-005: `{ FILTER(bound($this)) } ?this :p "L" . FILTER(bound($this))`.
        // The inner `{ FILTER(bound($this)) }` sibling-block wraps a FILTER over an
        // EMPTY inner; the pre-binding must reach inside it (not just the first
        // descent), so $this is bound in that filter too.
        let inner = match select_pattern(
            "SELECT $this WHERE { { FILTER(bound(?this)) } ?this <http://x/p> \"L\" }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::Join { .. }), "{:?}", inner);
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::Join { left, right } => {
                // The FILTER sibling now wraps a Values-join (its empty inner is
                // bound); the BGP sibling is joined with Values directly.
                assert!(
                    filter_over_values_join(left),
                    "JOIN sibling FILTER block not pre-bound: {:?}",
                    left
                );
                assert!(
                    values_join_here(right) || filter_over_values_join(right),
                    "JOIN BGP sibling not pre-bound: {:?}",
                    right
                );
            }
            other => panic!("expected Join, got {:?}", other),
        }
    }

    #[test]
    fn push_values_down_through_projecting_subselect() {
        // pre-binding-007: `{ SELECT $this WHERE { FILTER($this = <x>) } }`. The
        // inner sub-SELECT explicitly projects ?this, so it stays in scope inside;
        // the pre-binding must descend THROUGH the Project to the FILTER.
        let inner = match select_pattern(
            "SELECT $this WHERE { { SELECT $this WHERE { FILTER(?this = <http://x/n>) } } }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        assert!(matches!(inner, GraphPattern::Project { .. }), "{:?}", inner);
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::Project { inner, variables } => {
                assert!(
                    variables.iter().any(|v| v.as_str() == "this"),
                    "sub-SELECT must keep ?this projected: {:?}",
                    variables
                );
                assert!(
                    filter_over_values_join(inner),
                    "sub-SELECT FILTER not pre-bound: {:?}",
                    inner
                );
            }
            other => panic!("expected Project, got {:?}", other),
        }
    }

    #[test]
    fn push_values_down_above_select_star_subselect() {
        // A `SELECT *` sub-select (an EMPTY projection list in this algebra) does
        // NOT explicitly project the pre-bound variable, so the spec re-scopes it
        // (the `sht:Failure` pre-binding-006). The pre-binding is joined ABOVE the
        // sub-select (the leaf arm), not descended into it.
        let inner = match select_pattern(
            "SELECT $this WHERE { { SELECT * WHERE { FILTER(?this = <http://x/n>) } } }",
        ) {
            GraphPattern::Project { inner, .. } => *inner,
            other => other,
        };
        // The inner sub-select is `Project { …, variables: [] }` (SELECT *).
        assert!(
            matches!(&inner, GraphPattern::Project { variables, .. } if variables.is_empty()),
            "{:?}",
            inner
        );
        let out = push_values_down(&inner, &values_this());
        // Joined ABOVE: `Join { Values, Project{…} }`, NOT descended into the inner.
        assert!(
            matches!(&out, GraphPattern::Join { left, right }
                if matches!(**left, GraphPattern::Values { .. })
                    && matches!(**right, GraphPattern::Project { .. })),
            "SELECT * sub-select must be bound ABOVE, got {:?}",
            out
        );
    }

    #[test]
    fn push_values_down_through_graph_block() {
        // [OPUS-4.8] (sq-mue75) A GRAPH block keeps the outer pre-bound variable in
        // scope; the pre-binding descends into its inner.
        let inner = GraphPattern::Graph {
            name: spargebra::term::NamedNodePattern::NamedNode(
                NamedNode::new("http://g/named").unwrap(),
            ),
            inner: Box::new(filter_bgp()),
        };
        let out = push_values_down(&inner, &values_this());
        match &out {
            GraphPattern::Graph { inner, .. } => assert!(
                filter_over_values_join(inner),
                "GRAPH inner not pre-bound: {:?}",
                inner
            ),
            other => panic!("expected Graph, got {:?}", other),
        }
    }

    // --- inject_values: solution-modifier descent (Reduced/Slice/OrderBy) -----

    #[test]
    fn inject_values_descends_distinct_reduced_slice_orderby() {
        let this = Term::NamedNode(NamedNode::new("http://example.org/n").unwrap());
        // Each top-level modifier above the Project is descended by inject_values
        // (the LIMIT/DISTINCT/ORDER BY apply to the PRE-BOUND results), then the
        // bound var is added to the projection.
        for q in [
            "SELECT DISTINCT ?this WHERE { ?this <http://x/p> ?v . FILTER(?v > 0) }",
            "SELECT REDUCED ?this WHERE { ?this <http://x/p> ?v . FILTER(?v > 0) }",
            "SELECT ?this WHERE { ?this <http://x/p> ?v . FILTER(?v > 0) } ORDER BY ?v LIMIT 3",
        ] {
            let query = SparqlParser::new().parse_query(q).unwrap();
            let bound = pre_bind_select(&query, &[("this", &this)]).unwrap();
            // The pre-bound `?this` must surface in the projection (so a consumer
            // reading it back gets the focus node).
            let Query::Select { pattern, .. } = &bound else {
                unreachable!()
            };
            assert!(
                projection_contains(pattern, "this"),
                "?this not projected for `{}`: {:?}",
                q,
                pattern
            );
        }
    }

    /// Does some Project under `p` list a variable named `name`?
    fn projection_contains(p: &GraphPattern, name: &str) -> bool {
        match p {
            GraphPattern::Project { inner, variables } => {
                variables.iter().any(|v| v.as_str() == name) || projection_contains(inner, name)
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Filter { inner, .. }
            | GraphPattern::Group { inner, .. }
            | GraphPattern::Extend { inner, .. } => projection_contains(inner, name),
            GraphPattern::Join { left, right } | GraphPattern::Minus { left, right } => {
                projection_contains(left, name) || projection_contains(right, name)
            }
            _ => false,
        }
    }

    // --- pre_bind_ask: descend the ASK Project, bind through a modifier --------

    #[test]
    fn pre_bind_ask_descends_into_project_and_modifier() {
        let value = Term::Literal(Literal::new_simple_literal("abcdef"));
        // `ASK { FILTER(STRLEN(STR($value)) <= 3) }` => Project { Filter { Bgp } }.
        let q = SparqlParser::new()
            .parse_query("ASK { FILTER (STRLEN(STR(?value)) <= 3) }")
            .unwrap();
        let bound = pre_bind_ask(&q, &[("value", &value)]).unwrap();
        let Query::Ask { pattern, .. } = &bound else {
            panic!("expected ASK")
        };
        // The outer Project shell is preserved; VALUES joined below the FILTER.
        match pattern {
            GraphPattern::Project { inner, .. } => assert_values_at_leaf(inner, true),
            other => panic!("expected Project shell, got {:?}", other),
        }
    }

    // --- semantic: a real validator whose WHERE uses each modifier ------------

    fn graph(ttl: &str) -> Graph {
        Graph::load_str(ttl, "turtle").unwrap()
    }

    const DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:age 30 ; ex:tag "a", "b" .
        ex:bob   ex:age -1 ; ex:tag "c" .
    "#;

    /// SELECT-component validator: run per focus, each solution is a violation.
    /// The WHERE shape (Group/OrderBy/Slice/Minus over the FILTER) must NOT change
    /// the answer — the pre-bound `$this` stays in scope through the modifier.
    fn select_violations(select: &str, focus_iri: &str) -> usize {
        let data = graph(DATA);
        let query = SparqlParser::new().parse_query(select).unwrap();
        let validator = match query {
            Query::Select { .. } => query,
            _ => panic!("expected SELECT"),
        };
        let focus = Term::NamedNode(NamedNode::new(focus_iri).unwrap());
        let bindings: &[Binding] = &[("this", &focus)];
        let mut out = Vec::new();
        select_validate(
            &validator,
            &data,
            &focus,
            bindings,
            None,
            None,
            |f: ComponentResultFields| ValidationResult {
                focus_node: f.focus,
                path: f.path,
                value: f.value,
                source_shape: Term::NamedNode(NamedNode::new("http://example.org/S").unwrap()),
                source_constraint: None,
                source_component: "http://www.w3.org/ns/shacl#SPARQLConstraintComponent"
                    .to_string(),
                severity: "http://www.w3.org/ns/shacl#Violation".to_string(),
                messages: vec![],
                default_message: f.default_message,
                details: vec![],
            },
            &mut out,
        );
        out.len()
    }

    #[test]
    fn select_validate_group_pre_binds_this() {
        // Per focus, GROUP BY $this with HAVING(COUNT > 1): bob has 1 tag, alice 2.
        // The validator flags a focus with MORE than one ex:tag.
        let q = "SELECT ?this WHERE { ?this <http://example.org/tag> ?t } \
                 GROUP BY ?this HAVING (COUNT(?t) > 1)";
        // alice (2 tags) -> 1 solution -> 1 violation.
        assert_eq!(select_violations(q, "http://example.org/alice"), 1);
        // bob (1 tag) -> 0 solutions -> conforms.
        assert_eq!(select_violations(q, "http://example.org/bob"), 0);
    }

    #[test]
    fn select_validate_order_by_slice_pre_binds_this() {
        // ORDER BY + LIMIT over the pre-binding: alice has age 30 (>0 -> conforms,
        // FILTER keeps only negatives), bob has age -1 -> 1 violation.
        let q = "SELECT ?this ?value WHERE { ?this <http://example.org/age> ?value . \
                 FILTER(?value < 0) } ORDER BY ?value LIMIT 10";
        assert_eq!(select_violations(q, "http://example.org/bob"), 1);
        assert_eq!(select_violations(q, "http://example.org/alice"), 0);
    }

    #[test]
    fn select_validate_minus_pre_binds_this() {
        // MINUS over the pre-binding: flag a focus that has an ex:age but is NOT
        // excluded by the MINUS (here MINUS removes nothing matching). bob age -1
        // present -> the pattern { ?this :age ?value } MINUS {} yields a solution.
        let q = "SELECT ?this ?value WHERE { ?this <http://example.org/age> ?value . \
                 FILTER(?value < 0) MINUS { ?this <http://example.org/never> ?z } }";
        assert_eq!(select_violations(q, "http://example.org/bob"), 1);
        assert_eq!(select_violations(q, "http://example.org/alice"), 0);
    }

    /// ASK-component validator: run per value node; ASK=false (constraint not
    /// satisfied) is a violation.
    fn ask_is_violation(ask: &str, value: &Term) -> bool {
        let data = graph(DATA);
        let query = SparqlParser::new().parse_query(ask).unwrap();
        let this = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        ask_violates(&query, &data, &[("this", &this), ("value", value)])
    }

    #[test]
    fn ask_violates_filter_pre_binds_value() {
        // ASK { FILTER(STRLEN(STR($value)) <= 3) }: a 6-char value VIOLATES
        // (ASK=false); a 2-char value conforms (ASK=true).
        let q = "ASK { FILTER (STRLEN(STR($value)) <= 3) }";
        let long = Term::Literal(Literal::new_simple_literal("abcdef"));
        let short = Term::Literal(Literal::new_simple_literal("ab"));
        assert!(ask_is_violation(q, &long), "6-char value should violate");
        assert!(!ask_is_violation(q, &short), "2-char value should conform");
    }

    // --- ERROR / fail-closed paths -------------------------------------------

    #[test]
    fn prepared_validator_build_rejects_form_mismatch() {
        // is_ask=true but the query is a SELECT -> None (ill-formed -> skipped).
        assert!(PreparedValidator::build("SELECT * WHERE { ?s ?p ?o }", true).is_none());
        // is_ask=false but the query is an ASK -> None.
        assert!(PreparedValidator::build("ASK { ?s ?p ?o }", false).is_none());
        // A CONSTRUCT (neither ASK nor SELECT) -> None for both flags.
        let construct = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
        assert!(PreparedValidator::build(construct, true).is_none());
        assert!(PreparedValidator::build(construct, false).is_none());
        // Unparsable -> None.
        assert!(PreparedValidator::build("NOT A QUERY", true).is_none());
        // Well-formed matches build OK.
        assert!(matches!(
            PreparedValidator::build("ASK { ?s ?p ?o }", true),
            Some(PreparedValidator::Ask(_))
        ));
        assert!(matches!(
            PreparedValidator::build("SELECT * WHERE { ?s ?p ?o }", false),
            Some(PreparedValidator::Select(_))
        ));
    }

    #[test]
    fn prepared_sparql_build_rejects_non_select() {
        // A non-SELECT sh:select is ill-formed -> None.
        let ask = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "ASK { ?s ?p ?o }".to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        // [OPUS-4.8] (sq-0mjfd) build now returns Result<Option<_>, _>: a non-SELECT
        // / unparsable query is `Ok(None)` (skip), not `Err` (a pre-binding failure).
        assert!(matches!(PreparedSparql::build(&ask), Ok(None)));
        // Unparsable -> Ok(None).
        let bad = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ??? garbage".to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        assert!(matches!(PreparedSparql::build(&bad), Ok(None)));
        // Valid SELECT -> Ok(Some).
        let ok = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ?this WHERE { ?this ?p ?o }".to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        assert!(matches!(PreparedSparql::build(&ok), Ok(Some(_))));
    }

    #[test]
    fn pre_bind_select_rejects_non_select_query() {
        let this = Term::NamedNode(NamedNode::new("http://example.org/n").unwrap());
        let ask = SparqlParser::new().parse_query("ASK { ?s ?p ?o }").unwrap();
        assert!(pre_bind_select(&ask, &[("this", &this)]).is_none());
    }

    #[test]
    fn pre_bind_ask_rejects_non_ask_query() {
        let this = Term::NamedNode(NamedNode::new("http://example.org/n").unwrap());
        let sel = SparqlParser::new()
            .parse_query("SELECT * WHERE { ?s ?p ?o }")
            .unwrap();
        assert!(pre_bind_ask(&sel, &[("this", &this)]).is_none());
    }

    #[test]
    fn pre_bind_rejects_inexpressible_blank_node() {
        // A blank-node focus cannot be written as a VALUES ground term -> None
        // (the constraint is skipped rather than mis-evaluated, SHACL-leniently).
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("b0").unwrap());
        let sel = SparqlParser::new()
            .parse_query("SELECT ?this WHERE { ?this ?p ?o }")
            .unwrap();
        assert!(pre_bind_select(&sel, &[("this", &bnode)]).is_none());
        let ask = SparqlParser::new().parse_query("ASK { ?s ?p ?o }").unwrap();
        assert!(pre_bind_ask(&ask, &[("this", &bnode)]).is_none());
        // term_to_ground itself returns None for a blank node.
        assert!(term_to_ground(&bnode).is_none());
    }

    #[test]
    fn ask_violates_fail_closed_on_inexpressible_value() {
        // ask_violates pre-binds a blank-node value -> pre_bind_ask None ->
        // returns false (conforms, fail-closed; never panics validate()).
        let data = graph(DATA);
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("b1").unwrap());
        let q = SparqlParser::new()
            .parse_query("ASK { FILTER (STRLEN(STR($value)) <= 3) }")
            .unwrap();
        assert!(!ask_violates(&q, &data, &[("value", &bnode)]));
    }

    #[test]
    fn select_validate_fail_closed_on_inexpressible_focus() {
        // A blank-node focus -> pre_bind_select None -> no results pushed.
        let data = graph(DATA);
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("b2").unwrap());
        let q = SparqlParser::new()
            .parse_query("SELECT ?this WHERE { ?this ?p ?o }")
            .unwrap();
        let mut out = Vec::new();
        select_validate(
            &q,
            &data,
            &bnode,
            &[("this", &bnode)],
            None,
            None,
            |_f: ComponentResultFields| unreachable!("no solutions for an inexpressible focus"),
            &mut out,
        );
        assert!(out.is_empty());
    }

    // --- term_lexical / substitute helper edges -------------------------------

    #[test]
    fn term_lexical_covers_each_term_kind() {
        assert_eq!(
            term_lexical(Term::NamedNode(NamedNode::new("http://x/n").unwrap())),
            "http://x/n"
        );
        assert_eq!(
            term_lexical(Term::Literal(Literal::new_simple_literal("hi"))),
            "hi"
        );
        assert_eq!(
            term_lexical(Term::BlankNode(oxrdf::BlankNode::new("b").unwrap())),
            "_:b"
        );
    }

    #[test]
    fn substitute_keeps_unknown_and_unclosed_braces_verbatim() {
        let vars = vec!["this".to_string()];
        let row = vec![Some(Term::NamedNode(NamedNode::new("http://x/n").unwrap()))];
        // Known {?this} substitutes; unknown {?missing} and a non-var {literal}
        // and an unclosed `{` are kept verbatim.
        assert_eq!(
            substitute("a {?this} b {?missing} c {plain} d {", &vars, &row),
            "a http://x/n b {?missing} c {plain} d {"
        );
        // No placeholders at all -> identity.
        assert_eq!(substitute("no braces here", &vars, &row), "no braces here");
    }

    #[test]
    fn substitute_bindings_over_pre_bound_terms() {
        let value = Term::Literal(Literal::new_simple_literal("xyz"));
        let out = substitute_bindings("len of {$value} too big", &[("value", &value)]);
        assert_eq!(out, "len of xyz too big");
    }

    // --- runtime-query-error fail-closed paths --------------------------------
    //
    // A `SERVICE` clause parses fine but the local-only executor refuses it at
    // RUNTIME (`Err("unsupported graph pattern: Service …")`). The three
    // SHACL-SPARQL entry points must treat that as conforming / no-solutions
    // (lenient — a query error NEVER panics validate(); SHACL §5.2 / §6.3).

    #[test]
    fn ask_violates_fail_closed_on_runtime_query_error() {
        let data = graph(DATA);
        let this = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        let q = SparqlParser::new()
            .parse_query("ASK { SERVICE <http://example.org/remote> { ?s ?p ?o } }")
            .unwrap();
        // ask_prepared returns Err -> ask_violates returns false (conforms).
        assert!(!ask_violates(&q, &data, &[("this", &this)]));
    }

    #[test]
    fn select_validate_fail_closed_on_runtime_query_error() {
        let data = graph(DATA);
        let focus = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        let q = SparqlParser::new()
            .parse_query(
                "SELECT ?this WHERE { SERVICE <http://example.org/remote> { ?this ?p ?o } }",
            )
            .unwrap();
        let mut out = Vec::new();
        select_validate(
            &q,
            &data,
            &focus,
            &[("this", &focus)],
            None,
            None,
            |_f: ComponentResultFields| unreachable!("a runtime query error yields no solutions"),
            &mut out,
        );
        // query_prepared returns Err -> no results pushed.
        assert!(out.is_empty());
    }

    #[test]
    fn prepared_sparql_build_rejects_service_at_build_time() {
        // [OPUS-4.8] (sq-0mjfd) The §5.2 `sh:sparql` path: a SERVICE clause is a
        // SHACL-SPARQL pre-binding violation (pre-binding into a federated block is
        // undefined), so `build` now REJECTS it up front (an `Err` failure outcome,
        // shacl12 unsupported-sparql-003) rather than deferring to a runtime error.
        let constraint = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ?this WHERE { SERVICE <http://example.org/remote> { ?this ?p ?o } }"
                .to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        assert!(matches!(
            PreparedSparql::build(&constraint),
            Err(PreBindingViolation::Service)
        ));
    }

    // --- PreparedSparql::evaluate happy path: ?value / ?path / {?var} mapping --
    //
    // Drives the §5.2 solution -> ValidationResult field mapping (the ?value /
    // ?path / {?var}-message branches) directly through the public evaluate().

    #[test]
    fn prepared_sparql_evaluate_maps_value_path_and_message() {
        // Data: alice ex:knows bob (an IRI value on a predicate path).
        let data =
            graph("@prefix ex: <http://example.org/> . ex:alice ex:knows ex:bob ; ex:age 30 .");
        let focus = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        // Project ?value and ?path; the constraint message references {?value}.
        let constraint = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ?this ?value ?path WHERE { \
                       BIND(<http://example.org/bob> AS ?value) \
                       BIND(<http://example.org/knows> AS ?path) \
                       ?this <http://example.org/knows> ?value }"
                .to_string(),
            prefixes: String::new(),
            message: Some("offender {?value} via {?path}".to_string()),
            severity: None,
            deactivated: false,
            prepared: None,
        };
        let prepared = PreparedSparql::build(&constraint).unwrap().unwrap();
        let mut out = Vec::new();
        prepared.evaluate(
            &data,
            &focus,
            &constraint,
            |f: ResultFields| ValidationResult {
                focus_node: focus.clone(),
                path: f.path,
                value: f.value,
                source_shape: focus.clone(),
                source_constraint: None,
                source_component: "x".to_string(),
                severity: "x".to_string(),
                messages: vec![],
                default_message: f.default_message,
                details: vec![],
            },
            &mut out,
        );
        assert_eq!(out.len(), 1, "alice ex:knows ex:bob -> one solution");
        let r = &out[0];
        assert_eq!(
            r.value,
            Some(Term::NamedNode(
                NamedNode::new("http://example.org/bob").unwrap()
            )),
            "?value maps to the bound bob"
        );
        assert!(
            matches!(&r.path, Some(crate::path::Path::Predicate(p)) if p == "http://example.org/knows"),
            "?path (an IRI) maps to a predicate path, got {:?}",
            r.path
        );
        assert_eq!(
            r.default_message, "offender http://example.org/bob via http://example.org/knows",
            "{{?value}}/{{?path}} substituted from the solution row"
        );
    }

    #[test]
    fn prepared_sparql_evaluate_uses_row_message_when_no_constraint_message() {
        // No constraint sh:message + a projected ?message binding -> the result's
        // default_message is taken from the ?message row value (§5.2.2 precedence:
        // constraint message > ?message binding > generated default).
        let data = graph("@prefix ex: <http://example.org/> . ex:alice ex:age 30 .");
        let focus = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        let constraint = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ?this ?message WHERE { ?this <http://example.org/age> ?v . \
                       BIND(\"row-level reason\" AS ?message) }"
                .to_string(),
            prefixes: String::new(),
            message: None, // no constraint message -> fall through to ?message
            severity: None,
            deactivated: false,
            prepared: None,
        };
        let prepared = PreparedSparql::build(&constraint).unwrap().unwrap();
        let mut out = Vec::new();
        prepared.evaluate(
            &data,
            &focus,
            &constraint,
            |f: ResultFields| ValidationResult {
                focus_node: focus.clone(),
                path: f.path,
                value: f.value,
                source_shape: focus.clone(),
                source_constraint: None,
                source_component: "x".to_string(),
                severity: "x".to_string(),
                messages: vec![],
                default_message: f.default_message,
                details: vec![],
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].default_message, "row-level reason");
    }

    #[test]
    fn prepared_sparql_evaluate_fail_closed_on_blank_node_focus() {
        // A blank-node focus is inexpressible as VALUES -> evaluate() returns
        // early (the §5.2 unreachable-today guard): no results pushed.
        let data = graph("@prefix ex: <http://example.org/> . ex:alice ex:age 30 .");
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("focus0").unwrap());
        let constraint = SparqlConstraint {
            node: Term::NamedNode(NamedNode::new("http://example.org/c").unwrap()),
            select: "SELECT ?this WHERE { ?this ?p ?o }".to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        let prepared = PreparedSparql::build(&constraint).unwrap().unwrap();
        let mut out = Vec::new();
        prepared.evaluate(
            &data,
            &bnode,
            &constraint,
            |_f: ResultFields| unreachable!("an inexpressible focus yields no solutions"),
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn select_validate_maps_path_when_projected() {
        // A §6 SELECT validator that projects ?path (an IRI) -> the result's
        // sh:resultPath is that predicate path (§5.2.2 mapping in select_validate).
        let data = graph("@prefix ex: <http://example.org/> . ex:alice ex:knows ex:bob .");
        let focus = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        let query = SparqlParser::new()
            .parse_query(
                "SELECT ?this ?path WHERE { BIND(<http://example.org/knows> AS ?path) \
                 ?this <http://example.org/knows> ?o }",
            )
            .unwrap();
        let mut out = Vec::new();
        select_validate(
            &query,
            &data,
            &focus,
            &[("this", &focus)],
            None,
            None,
            |f: ComponentResultFields| ValidationResult {
                focus_node: f.focus,
                path: f.path,
                value: f.value,
                source_shape: focus.clone(),
                source_constraint: None,
                source_component: "x".to_string(),
                severity: "x".to_string(),
                messages: vec![],
                default_message: f.default_message,
                details: vec![],
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0].path, Some(crate::path::Path::Predicate(p)) if p == "http://example.org/knows"),
            "?path (an IRI) -> a predicate path, got {:?}",
            out[0].path
        );
    }

    #[test]
    fn select_validate_non_iri_path_maps_to_none() {
        // A ?path bound to a NON-IRI (a literal) is not a predicate path -> the
        // result carries no sh:resultPath (the `_ => None` arm of the §5.2.2 path
        // mapping). Spec-correct: only an IRI ?path is a predicate path.
        let data = graph("@prefix ex: <http://example.org/> . ex:alice ex:age 30 .");
        let focus = Term::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
        let query = SparqlParser::new()
            .parse_query(
                "SELECT ?this ?path WHERE { BIND(\"not-an-iri\" AS ?path) \
                 ?this <http://example.org/age> ?o }",
            )
            .unwrap();
        let mut out = Vec::new();
        select_validate(
            &query,
            &data,
            &focus,
            &[("this", &focus)],
            None,
            None,
            |f: ComponentResultFields| ValidationResult {
                focus_node: f.focus,
                path: f.path,
                value: f.value,
                source_shape: focus.clone(),
                source_constraint: None,
                source_component: "x".to_string(),
                severity: "x".to_string(),
                messages: vec![],
                default_message: f.default_message,
                details: vec![],
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert!(
            out[0].path.is_none(),
            "a literal ?path is not a predicate path"
        );
    }

    // --- [OPUS-4.8] (sq-rnkdh) SHACL 1.2 SPARQL-based node expressions ---------

    const EXPR_DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:age 30 ; ex:firstName "Al" ; ex:lastName "Ice" .
        ex:bob   ex:age 15 .
    "#;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    #[test]
    fn select_expr_build_rejects_non_select() {
        // A non-SELECT (or unparsable) sh:select is ill-formed -> None.
        assert!(PreparedSelectExpr::build_select("", "ASK { ?s ?p ?o }").is_none());
        assert!(PreparedSelectExpr::build_select("", "SELECT ??? nope").is_none());
        assert!(PreparedSelectExpr::build_select("", "SELECT ?x WHERE { ?x ?p ?o }").is_some());
    }

    #[test]
    fn select_expr_eval_pre_binds_this_and_takes_first_var() {
        // sh:values-style: per focus, project the value via $this.
        let data = graph(EXPR_DATA);
        let expr = PreparedSelectExpr::build_select(
            "PREFIX ex: <http://example.org/>",
            "SELECT ?n WHERE { $this ex:firstName ?n }",
        )
        .unwrap();
        let nodes = expr.eval(&data, &iri("http://example.org/alice"));
        assert_eq!(
            nodes,
            vec![Term::Literal(oxrdf::Literal::new_simple_literal("Al"))]
        );
        // A focus with no firstName yields no value nodes.
        assert!(expr.eval(&data, &iri("http://example.org/bob")).is_empty());
    }

    #[test]
    fn select_expr_build_expr_evaluates_expression() {
        // sh:sparqlExpr: SELECT ((EXPR) AS ?result) WHERE {} with $this bound.
        let data = graph(EXPR_DATA);
        let expr = PreparedSelectExpr::build_expr("", "STRLEN(STR($this))").unwrap();
        let nodes = expr.eval(&data, &iri("http://example.org/alice"));
        // STRLEN("http://example.org/alice") = 24.
        assert_eq!(
            nodes,
            vec![Term::Literal(oxrdf::Literal::new_typed_literal(
                "24",
                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            ))]
        );
    }

    #[test]
    fn select_expr_eval_target_runs_unbound_and_collects_first_var() {
        // sh:targetNode [ sh:select ]: no $this, the query SELECTs the focus nodes.
        let data = graph(EXPR_DATA);
        let expr = PreparedSelectExpr::build_select(
            "PREFIX ex: <http://example.org/>",
            "SELECT ?p WHERE { ?p ex:age ?a . FILTER (?a < 18) }",
        )
        .unwrap();
        let nodes = expr.eval_target(&data);
        assert_eq!(nodes, vec![iri("http://example.org/bob")]);
    }

    // --- [SONNET-4.6] (sq-7d3dj.33.5) PreparedSelectExpr::eval_batch ---------------
    //
    // Three tests covering the batched node-expression path:
    //   * anti-vacuity: 4 foci → exactly 1 execution, correct per-focus grouping.
    //   * equivalence: eval_batch and per-focus eval produce the same value-node sets.
    //   * blank-node filtering: blank-node foci are absent from the result map.

    const BATCH_DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:a ex:color "red"    .
        ex:b ex:color "blue"   .
        ex:c ex:color "green"  .
        ex:d ex:color "yellow" .
    "#;

    #[test]
    fn eval_batch_groups_output_nodes_by_focus_and_fires_once() {
        // [SONNET-4.6] (sq-7d3dj.33.5) 4 foci → exactly ONE query execution (not 4);
        // each focus's output-node set is correct.
        let data = graph(BATCH_DATA);
        let expr = PreparedSelectExpr::build_select(
            "PREFIX ex: <http://example.org/>",
            "SELECT ?c WHERE { $this ex:color ?c }",
        )
        .unwrap();
        let foci = [
            iri("http://example.org/a"),
            iri("http://example.org/b"),
            iri("http://example.org/c"),
            iri("http://example.org/d"),
        ];
        let before = exec_count();
        let grouped = expr.eval_batch(&data, &foci);
        // Anti-vacuity: 4 foci → 1 execution, not 4.
        assert_eq!(exec_count() - before, 1, "eval_batch must fire exactly once");
        // Every expressible focus gets an entry; each has its color literal.
        assert_eq!(grouped.len(), 4);
        assert_eq!(
            grouped[&foci[0]],
            vec![Term::Literal(oxrdf::Literal::new_simple_literal("red"))]
        );
        assert_eq!(
            grouped[&foci[1]],
            vec![Term::Literal(oxrdf::Literal::new_simple_literal("blue"))]
        );
        assert_eq!(
            grouped[&foci[2]],
            vec![Term::Literal(oxrdf::Literal::new_simple_literal("green"))]
        );
        assert_eq!(
            grouped[&foci[3]],
            vec![Term::Literal(oxrdf::Literal::new_simple_literal("yellow"))]
        );
    }

    #[test]
    fn eval_batch_result_equals_per_focus_eval() {
        // [SONNET-4.6] (sq-7d3dj.33.5) eval_batch must produce the same value-node
        // sets as calling eval() once per focus node.
        let data = graph(BATCH_DATA);
        let expr = PreparedSelectExpr::build_select(
            "PREFIX ex: <http://example.org/>",
            "SELECT ?c WHERE { $this ex:color ?c }",
        )
        .unwrap();
        let foci = [
            iri("http://example.org/a"),
            iri("http://example.org/b"),
            iri("http://example.org/c"),
            iri("http://example.org/d"),
        ];
        let grouped = expr.eval_batch(&data, &foci);
        for f in &foci {
            let batched = grouped.get(f).cloned().unwrap_or_default();
            let per_focus = expr.eval(&data, f);
            assert_eq!(
                batched, per_focus,
                "eval_batch disagrees with eval() for focus {}",
                f
            );
        }
    }

    #[test]
    fn eval_batch_skips_blank_node_foci() {
        // [SONNET-4.6] (sq-7d3dj.33.5) A blank-node focus cannot appear in VALUES
        // syntax; eval_batch must omit it from the result map (not panic or error).
        let data = graph(BATCH_DATA);
        let expr = PreparedSelectExpr::build_select(
            "PREFIX ex: <http://example.org/>",
            "SELECT ?c WHERE { $this ex:color ?c }",
        )
        .unwrap();
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("bn0").unwrap());
        let iri_focus = iri("http://example.org/a");
        let foci = [bnode.clone(), iri_focus.clone()];
        let grouped = expr.eval_batch(&data, &foci);
        // Blank node is absent; the IRI focus has its entry.
        assert!(
            !grouped.contains_key(&bnode),
            "blank-node focus must be absent from eval_batch result"
        );
        assert!(grouped.contains_key(&iri_focus));
    }

    // --- [OPUS-4.8] (sq-0mjfd) SHACL-SPARQL pre-binding rejection (sht:Failure) --

    /// Parse a top-level SELECT and run the `$this` pre-binding check on its WHERE,
    /// descending through the top-level result `Project` exactly as `build` does.
    fn check_select(select: &str) -> Result<(), PreBindingViolation> {
        match SparqlParser::new().parse_query(select).unwrap() {
            Query::Select { pattern, .. } => {
                let body = match &pattern {
                    GraphPattern::Project { inner, .. } => inner.as_ref(),
                    other => other,
                };
                check_pre_binding(body, PRE_BOUND_THIS)
            }
            other => panic!("expected SELECT, got {:?}", other),
        }
    }

    #[test]
    fn pre_binding_rejects_minus() {
        // unsupported-sparql-001: MINUS is undefined under pre-binding.
        assert_eq!(
            check_select("SELECT $this WHERE { $this ?x ?any . MINUS { $this ?x \"V\" } }"),
            Err(PreBindingViolation::Minus)
        );
    }

    #[test]
    fn pre_binding_rejects_values_and_service() {
        // unsupported-sparql-002 / 003: an author VALUES table / a SERVICE clause.
        assert_eq!(
            check_select("SELECT $this WHERE { VALUES ?any { true } }"),
            Err(PreBindingViolation::Values)
        );
        assert_eq!(
            check_select(
                "SELECT $this WHERE { $this ?x ?any . SERVICE <http://e/> { ?a ?b ?c } }"
            ),
            Err(PreBindingViolation::Service)
        );
    }

    #[test]
    fn pre_binding_rejects_subselect_dropping_this() {
        // unsupported-sparql-004: a sub-SELECT that projects ?other ?b (drops $this).
        let q = "SELECT $this WHERE { $this ?x ?any . { SELECT ?other ?b WHERE { ?other ?b ?c } } }";
        assert_eq!(
            check_select(q),
            Err(PreBindingViolation::SubSelectDropsPreBound("this".into()))
        );
        // pre-binding-006: a `SELECT *` sub-select also re-scopes $this.
        let star = "SELECT $this WHERE { { SELECT * WHERE { FILTER ($this = <http://e/x>) } } }";
        assert_eq!(
            check_select(star),
            Err(PreBindingViolation::SubSelectDropsPreBound("this".into()))
        );
    }

    #[test]
    fn pre_binding_rejects_rebind_of_this() {
        // unsupported-sparql-005: BIND (true AS $this) re-binds the pre-bound var.
        assert_eq!(
            check_select("SELECT $this WHERE { BIND (true AS $this) }"),
            Err(PreBindingViolation::RebindsPreBound("this".into()))
        );
    }

    #[test]
    fn pre_binding_accepts_sound_queries() {
        // The forms the 17 passing entries use must NOT be rejected: a sub-SELECT
        // that re-projects $this (pre-binding-007), a BIND to a NEW var
        // (pre-binding-004), a UNION (pre-binding-002), and a plain BGP+FILTER.
        assert_eq!(
            check_select(
                "SELECT $this WHERE { { SELECT $this WHERE { FILTER ($this = <http://e/x>) } } }"
            ),
            Ok(())
        );
        assert_eq!(
            check_select("SELECT $this WHERE { BIND ($this AS ?that) FILTER (?that = <http://e/x>) }"),
            Ok(())
        );
        assert_eq!(
            check_select(
                "SELECT $this WHERE { { FILTER (false) } UNION { FILTER ($this = <http://e/x>) } }"
            ),
            Ok(())
        );
        assert_eq!(
            check_select("SELECT $this WHERE { $this <http://e/p> \"L\" }"),
            Ok(())
        );
    }

    #[test]
    fn validator_pre_binding_rejects_value_rebind() {
        // unsupported-sparql-006: an ASK validator that re-binds the pre-bound
        // `?value` is a pre-binding violation in the component-validator path.
        assert_eq!(
            check_validator_pre_binding(
                "ASK { BIND (true AS ?value) FILTER (isLiteral(?value)) }",
                &["this", "value", "lang"],
            ),
            Err(PreBindingViolation::RebindsPreBound("value".into()))
        );
        // A validator that merely READS ?value is fine.
        assert_eq!(
            check_validator_pre_binding(
                "ASK { FILTER (isLiteral(?value)) }",
                &["this", "value", "lang"],
            ),
            Ok(())
        );
    }

    // --- [OPUS-4.8] (sq-7d3dj.33.1) focus-node batching ----------------------

    fn top_pattern(q: &str) -> GraphPattern {
        match SparqlParser::new().parse_query(q).unwrap() {
            Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {:?}", other),
        }
    }

    #[test]
    fn query_batchable_plain_select_is_batchable() {
        // A plain BGP+FILTER SELECT (the common sh:sparql shape) batches.
        assert!(query_batchable(&top_pattern(
            "SELECT ?this ?value WHERE { ?this <http://x/p> ?value . FILTER(?value < 0) }"
        )));
    }

    #[test]
    fn query_batchable_distinct_and_orderby_are_batchable() {
        // DISTINCT includes the distinct ?this (collapses per-focus); ORDER BY is
        // irrelevant to the violation bag — both stay batchable.
        assert!(query_batchable(&top_pattern(
            "SELECT DISTINCT ?this ?value WHERE { ?this <http://x/p> ?value } ORDER BY ?value"
        )));
    }

    #[test]
    fn query_batchable_rejects_top_level_limit() {
        // A top-level LIMIT is a GLOBAL row cap — not equivalent per focus.
        assert!(!query_batchable(&top_pattern(
            "SELECT ?this ?value WHERE { ?this <http://x/p> ?value } LIMIT 5"
        )));
    }

    #[test]
    fn query_batchable_allows_group_by_this() {
        // GROUP BY $this = each focus is its own group → per-focus-equivalent.
        assert!(query_batchable(&top_pattern(
            "SELECT ?this (COUNT(?value) AS ?c) WHERE { ?this <http://x/p> ?value } GROUP BY ?this"
        )));
    }

    #[test]
    fn query_batchable_rejects_implicit_aggregate() {
        // An implicit single group (no GROUP BY) aggregates across ALL foci.
        assert!(!query_batchable(&top_pattern(
            "SELECT (COUNT(?value) AS ?c) WHERE { ?this <http://x/p> ?value }"
        )));
    }

    #[test]
    fn query_batchable_rejects_group_by_other() {
        // GROUP BY a non-$this variable mixes rows from different foci into one group.
        assert!(!query_batchable(&top_pattern(
            "SELECT ?p (COUNT(?value) AS ?c) WHERE { ?this ?p ?value } GROUP BY ?p"
        )));
    }

    #[test]
    fn query_batchable_allows_nested_aggregate_subselect() {
        // A NESTED aggregate sub-select is fine: the pre-binding check forces it to
        // project (hence GROUP BY) $this, so the multi-row VALUES join partitions it
        // per focus. Only the TOP-level chain is inspected.
        assert!(query_batchable(&top_pattern(
            "SELECT ?this WHERE { ?this <http://x/p> ?v . \
             { SELECT ?this (COUNT(*) AS ?c) WHERE { ?this <http://x/q> ?w } GROUP BY ?this } \
             FILTER(?c > 1) }"
        )));
    }

    #[test]
    fn pre_bind_values_multi_builds_one_column_n_rows() {
        let a = iri("http://example.org/a");
        let b = iri("http://example.org/b");
        let c = iri("http://example.org/c");
        let vals = pre_bind_values_multi("this", &[&a, &b, &c]).unwrap();
        match vals {
            GraphPattern::Values {
                variables,
                bindings,
            } => {
                assert_eq!(variables.len(), 1);
                assert_eq!(variables[0].as_str(), "this");
                assert_eq!(bindings.len(), 3, "one row per focus");
                assert!(bindings.iter().all(|r| r.len() == 1), "single column");
            }
            other => panic!("expected VALUES, got {:?}", other),
        }
    }

    #[test]
    fn pre_bind_values_multi_none_for_blank_node() {
        // A blank node has no VALUES ground-term form (matches the per-focus skip).
        let bnode = Term::BlankNode(oxrdf::BlankNode::new("b0").unwrap());
        assert!(pre_bind_values_multi("this", &[&bnode]).is_none());
    }

    #[test]
    fn pre_bind_select_multi_projects_this_and_injects_multi_row() {
        // The injected query must project ?this (so results can be grouped by focus)
        // and carry the N-row VALUES table.
        let q = SparqlParser::new()
            .parse_query("SELECT ?value WHERE { ?this <http://x/p> ?value }")
            .unwrap();
        let a = iri("http://example.org/a");
        let b = iri("http://example.org/b");
        let bound = pre_bind_select_multi(&q, "this", &[&a, &b]).unwrap();
        let Query::Select { pattern, .. } = bound else {
            panic!("expected SELECT");
        };
        // ?this added to the projection.
        let GraphPattern::Project { variables, .. } = &pattern else {
            panic!("expected Project at top, got {:?}", pattern);
        };
        assert!(
            variables.iter().any(|v| v.as_str() == "this"),
            "?this must be projected for grouping: {:?}",
            variables
        );
        // A 2-row VALUES exists somewhere in the injected pattern.
        fn max_values_rows(p: &GraphPattern) -> usize {
            match p {
                GraphPattern::Values { bindings, .. } => bindings.len(),
                GraphPattern::Project { inner, .. }
                | GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. }
                | GraphPattern::OrderBy { inner, .. }
                | GraphPattern::Filter { inner, .. }
                | GraphPattern::Extend { inner, .. }
                | GraphPattern::Group { inner, .. }
                | GraphPattern::Graph { inner, .. } => max_values_rows(inner),
                GraphPattern::Join { left, right }
                | GraphPattern::Union { left, right }
                | GraphPattern::LeftJoin { left, right, .. }
                | GraphPattern::Minus { left, right } => {
                    max_values_rows(left).max(max_values_rows(right))
                }
                _ => 0,
            }
        }
        assert_eq!(max_values_rows(&pattern), 2, "multi-row VALUES injected");
    }

    /// End-to-end batched execution: one `evaluate_batch` groups solutions by focus
    /// and stamps them, with the executed-query count amortised to ONE for many foci.
    #[test]
    fn evaluate_batch_groups_by_focus_and_fires_once() {
        use crate::model::SparqlConstraint;
        let data = Graph::load_str(
            "@prefix ex: <http://example.org/> .\n\
             ex:a ex:age -1 . ex:b ex:age 5 . ex:c ex:age -2 . ex:d ex:age 9 .",
            "turtle",
        )
        .unwrap();
        let constraint = SparqlConstraint {
            node: iri("http://example.org/constraint"),
            select: "SELECT $this ?value WHERE { $this <http://example.org/age> ?value . \
                     FILTER(?value < 0) }"
                .to_string(),
            prefixes: String::new(),
            message: None,
            severity: None,
            deactivated: false,
            prepared: None,
        };
        let prepared = PreparedSparql::build(&constraint).unwrap().unwrap();
        assert!(prepared.is_batchable());
        let foci = [
            iri("http://example.org/a"),
            iri("http://example.org/b"),
            iri("http://example.org/c"),
            iri("http://example.org/d"),
        ];
        let before = exec_count();
        let grouped = prepared.evaluate_batch(&data, &foci, &constraint, |focus, fields| {
            crate::report::ValidationResult {
                focus_node: focus.clone(),
                path: fields.path,
                value: fields.value,
                source_shape: Term::NamedNode(NamedNode::new("http://example.org/S").unwrap()),
                source_constraint: None,
                source_component: crate::model::sh("SPARQLConstraintComponent"),
                severity: crate::model::sh("Violation"),
                messages: vec![],
                default_message: fields.default_message,
                details: vec![],
            }
        });
        // Anti-vacuity: ONE execution for four focus nodes (not four).
        assert_eq!(exec_count() - before, 1, "batched path must fire exactly once");
        // Every focus has an entry; only the negative-age foci have a violation.
        assert_eq!(grouped.len(), 4);
        assert_eq!(grouped[&foci[0]].len(), 1); // ex:a age -1
        assert_eq!(grouped[&foci[1]].len(), 0); // ex:b age 5
        assert_eq!(grouped[&foci[2]].len(), 1); // ex:c age -2
        assert_eq!(grouped[&foci[3]].len(), 0); // ex:d age 9
    }
}

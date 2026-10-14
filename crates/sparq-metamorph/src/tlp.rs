//! **Ternary Logic Partitioning (TLP), re-derived for SPARQL's three-valued
//! effective-boolean-value semantics.** sq-gum8.6 — the research core of the
//! planned SPARQL logic-bug paper (sq-gum8.7).
//!
//! TLP for SQL (Rigger & Su, OOPSLA 2020) partitions a query on a predicate `p` into
//! `WHERE p`, `WHERE NOT p`, `WHERE p IS NULL` and checks that the three partitions
//! recompose the unpartitioned result. That construction does **not** transfer to SPARQL
//! verbatim, because SPARQL's third truth state is different in kind: SQL has a third
//! *value* (`NULL`, testable in-language with `IS NULL`), while SPARQL has a third
//! *evaluation outcome* — a **type error** — which is not a value, cannot be bound, and
//! has no `IS ERROR` test. The re-derivation below reifies the error outcome using the
//! only two error-absorbing forms the language offers (`COALESCE` and `IF`).
//!
//! # The trichotomy
//!
//! Let `P` be a group graph pattern and `c` a filter expression. For each solution
//! mapping `μ ∈ eval(P)`, the effective boolean value of `c` under `μ` — write
//! `ebv(c, μ)` — is exactly one of **true**, **false**, or **error** (SPARQL 1.1 §17.2.2
//! "Effective Boolean Value": rules for `xsd:boolean`, strings, and numerics, and "all
//! other arguments, including unbound arguments … produce a type error"). The
//! trichotomy is total and exclusive *provided `c` is deterministic and a function of
//! `μ` alone* — see *Scope preconditions* below.
//!
//! `FILTER` keeps exactly the **true** outcome: solutions for which the expression
//! "either result\[s\] in an effective boolean value of false or produce\[s\] an error …
//! are eliminated" (SPARQL 1.1 §17.2 "Filter Evaluation"; algebra: §18.5 `Filter`).
//! So the false and error outcomes are *both* dropped, indistinguishably — the third
//! branch must be reconstructed, not merely negated.
//!
//! # The three partition queries
//!
//! [`tlp_queries`] builds, for base query `SELECT * WHERE { P }`:
//!
//! | branch | filter appended to `P` | keeps `μ` iff `ebv(c, μ)` is |
//! |--------|------------------------|------------------------------|
//! | true   | `FILTER( c )`          | **true** |
//! | false  | `FILTER( !( c ) )`     | **false** |
//! | error  | `FILTER( COALESCE( IF( c , false, false), true) )` | **error** |
//!
//! *Case analysis, per branch:*
//!
//! * **true** — `FILTER(c)` keeps exactly `ebv(c, μ) = true` by the filter-evaluation
//!   rule quoted above. No rewrite machinery is involved.
//! * **false** — `!` is `fn:not` applied to the operand's EBV (SPARQL 1.1 §17.3
//!   operator mapping; XPath `fn:not`): `!true = false`, `!false = true`, and an
//!   erroring operand **propagates the error** (`fn:not` never converts an error into a
//!   boolean). Hence `FILTER(!(c))` keeps exactly `ebv(c, μ) = false` — the error rows
//!   stay eliminated, they are *not* folded into this branch.
//! * **error** — `IF(c, false, false)` evaluates `c`, takes its EBV, and returns
//!   `false` on both `true` and `false`; "if evaluating the first argument raises an
//!   error, then an error is raised for the evaluation of the IF expression"
//!   (§17.4.1.2 `IF`). `COALESCE(x, true)` "returns the value of the first expression
//!   that evaluates without error" (§17.4.1.3 `COALESCE`), i.e. `false` when `c`
//!   evaluated (either way), `true` when it errored. The composite is therefore a
//!   **total** boolean expression that is `true` exactly on the error rows — the error
//!   outcome reified as a value.
//!
//! Since the three kept-sets partition `eval(P)`, and `Filter` neither alters a
//! solution mapping nor changes the in-scope variables (a `FILTER` contributes no
//! bindings, §18.2.1, so `SELECT *` projects identically in all four queries), the
//! metamorphic relation is **multiset equality**:
//!
//! ```text
//! eval(SELECT * { P })  ≡  eval(true-branch) ⊎ eval(false-branch) ⊎ eval(error-branch)
//! ```
//!
//! [`check_tlp`] evaluates all four queries on one engine and compares with
//! sparq-difftest's multiset comparator. Note TLP requires **no cross-engine value
//! agreement and no external oracle**: only the engine's *internal consistency*.
//! Implementation-defined extensions (e.g. extra comparable datatypes) do not break
//! the relation as long as `ebv(c, μ)` is deterministic within the engine.
//!
//! # Where the error branch comes from (the SPARQL-specific cases)
//!
//! * **Type errors from bound values** — e.g. `"abc" < 5` (no operator mapping),
//!   arithmetic on non-numerics (`?v + 1` with `?v` a string), `LANG(?v)` /
//!   `DATATYPE(?v)` on an IRI, `=` between literals of incomparable types (RDFterm-equal
//!   "produces a type error" there, §17.4.1.7).
//! * **Unbound variables** — evaluating a variable unbound in `μ` (typically introduced
//!   by `OPTIONAL`) is a type error in every operator and function **except** the
//!   unbound-tolerant forms: `BOUND` (returns `false`, §17.4.1.1), `COALESCE` (skips
//!   erroring arguments, §17.4.1.3), and `IF`/`||`/`&&` insofar as they absorb rather
//!   than evaluate. So `?w >= 2` with `?w` unbound lands in the **error** branch, while
//!   `BOUND(?w)` lands in the **false** branch and `COALESCE(?w, 0) >= 2` lands in
//!   true/false — the partition is on the evaluation *outcome*, not on the syntactic
//!   cause, and stays exhaustive and exclusive in every such case.
//! * **Error absorption by the connectives** — SPARQL's `&&`/`||` are non-strict in
//!   errors (§17.2 "Truth Table for && and ||"): `false && error = false`,
//!   `true || error = true`, but `true && error = error`, `false || error = error`.
//!   An error in a *sub*-expression can therefore be absorbed into a top-level true or
//!   false; only the top-level outcome decides the branch. This is exactly the surface
//!   where engines historically diverge (short-circuit implementations that are
//!   accidentally strict), and the partition tests it for free.
//!
//! # Scope preconditions (violating these voids the relation — generator-enforced)
//!
//! 1. **Deterministic `c`, function of `μ` alone**: no `RAND()`, `NOW()`, `UUID()`,
//!    `STRUUID()`, `BNODE(…)`. A nondeterministic `c` can land the same `μ` in
//!    different branches across the four evaluations.
//! 2. **No `EXISTS` / `NOT EXISTS` in `c`**: their substitution semantics is a known
//!    SPARQL 1.1 spec defect (the Errata / SPARQL 1.2 `EXISTS` revision); a violation
//!    involving `EXISTS` would measure spec ambiguity, not an engine logic bug.
//! 3. **Plain `SELECT *` at the partition level**: no `DISTINCT`/`REDUCED` (breaks
//!    multiset additivity), no `ORDER BY`/`LIMIT`/`OFFSET` (partitioning does not
//!    commute with slicing), no aggregates. The *pattern* `P` may still contain
//!    `OPTIONAL`, `UNION`, subqueries, etc. `DISTINCT` and aggregates are not
//!    *untestable* — they need a **different** recombination law, each derived in its
//!    own module: [`crate::distinct`] (set union instead of multiset union) and
//!    [`crate::aggregate`] (numeric recombination plus an error-status law). `LIMIT`
//!    /`OFFSET` remain out of scope for every variant. sq-gum8.12
//! 4. **Top-level filter placement**: the branch filter is appended at the top level of
//!    the group, where a constraint applies to the whole group regardless of position
//!    (§17.2). The construction never pushes the filter into `OPTIONAL`/subquery scope
//!    (where filter scoping rules differ).
//!
//! # Attribution caveat (honest reporting)
//!
//! A TLP violation is evidence of a wrong result **somewhere in** the engine's
//! evaluation of {`P`, `FILTER`, EBV, `!`, `IF`, `COALESCE`, the operators of `c`} —
//! the rewrite machinery (`!`/`IF`/`COALESCE`) is part of the tested surface. It is a
//! genuine logic bug (some query among the four returned a wrong result set), but the
//! violation alone does not localise *which* construct is wrong; campaign triage
//! reduces the failing case before it enters the found-bug ledger.

use sparq_difftest::{multiset_equal, QueryResults, Solution};

use crate::engine::SparqlEngine;
use crate::verdict::{EngineFailure, FailureKind, OracleKind, Verdict, Violation};

/// The four queries of one TLP instance: the unpartitioned base plus the three
/// branch queries (see the module docs for the exact semantics of each).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlpQueries {
    /// `SELECT * WHERE { P }` — the unpartitioned base.
    pub base: String,
    /// Keeps `ebv(c, μ) = true`.
    pub branch_true: String,
    /// Keeps `ebv(c, μ) = false`.
    pub branch_false: String,
    /// Keeps `ebv(c, μ) = error` (the reified third branch).
    pub branch_error: String,
}

/// The three branch `FILTER` clauses — true, false, reified-error — in that order.
///
/// Single source of the partitioning construction: every TLP variant
/// (`tlp_queries`, `crate::distinct`, `crate::aggregate`) appends exactly these
/// clauses, so the case analysis in the module docs above covers all of them and the
/// variants cannot drift apart. sq-gum8.12
pub(crate) fn branch_filters(predicate: &str) -> [String; 3] {
    [
        format!("FILTER( {predicate} )"),
        format!("FILTER( !( {predicate} ) )"),
        format!("FILTER( COALESCE( IF( {predicate} , false, false), true) )"),
    ]
}

/// Build the four TLP queries for group graph pattern body `pattern` and filter
/// expression `predicate` (both SPARQL text; IRIs must be absolute — no prologue is
/// emitted). See the module docs for the derivation and the scope preconditions the
/// caller must respect.
pub fn tlp_queries(pattern: &str, predicate: &str) -> TlpQueries {
    let [keep_true, keep_false, keep_error] = branch_filters(predicate);
    TlpQueries {
        base: format!("SELECT * WHERE {{ {pattern} }}"),
        branch_true: format!("SELECT * WHERE {{ {pattern} {keep_true} }}"),
        branch_false: format!("SELECT * WHERE {{ {pattern} {keep_false} }}"),
        branch_error: format!("SELECT * WHERE {{ {pattern} {keep_error} }}"),
    }
}

/// Run one query and require a `SELECT` (solutions) result shape.
pub(crate) fn run_select(
    engine: &dyn SparqlEngine,
    query: &str,
) -> Result<Vec<Solution>, EngineFailure> {
    match engine.select(query)? {
        QueryResults::Solutions { solutions, .. } => Ok(solutions),
        QueryResults::Boolean(_) => Err(EngineFailure {
            engine: engine.name().to_string(),
            query: query.to_string(),
            kind: FailureKind::InvalidResults,
            message: "boolean result for a SELECT query".to_string(),
        }),
    }
}

/// Check the TLP metamorphic relation on one engine: the base result must equal the
/// multiset union of the three branch results.
///
/// Fail-closed: if any of the four queries fails to evaluate, the verdict is
/// [`Verdict::EngineFailure`] (never a pass, never a wrong-result claim).
pub fn check_tlp(engine: &dyn SparqlEngine, pattern: &str, predicate: &str) -> Verdict {
    let queries = tlp_queries(pattern, predicate);
    let base = match run_select(engine, &queries.base) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };
    let kept_true = match run_select(engine, &queries.branch_true) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };
    let kept_false = match run_select(engine, &queries.branch_false) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };
    let kept_error = match run_select(engine, &queries.branch_error) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };

    let detail = format!(
        "base={} true={} false={} error={}",
        base.len(),
        kept_true.len(),
        kept_false.len(),
        kept_error.len()
    );
    let mut union = kept_true;
    union.extend(kept_false);
    union.extend(kept_error);
    if multiset_equal(&base, &union) {
        Verdict::Pass { detail }
    } else {
        Verdict::Violation(Violation {
            oracle: OracleKind::Tlp,
            engines: vec![engine.name().to_string()],
            queries: vec![
                queries.base,
                queries.branch_true,
                queries.branch_false,
                queries.branch_error,
            ],
            detail: format!("partition union differs from base ({detail})"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlp_queries_builds_the_three_branch_filters() {
        let queries = tlp_queries("?s <http://example.org/v> ?v", "?v < 5");
        assert_eq!(queries.base, "SELECT * WHERE { ?s <http://example.org/v> ?v }");
        assert!(queries.branch_true.contains("FILTER( ?v < 5 )"));
        assert!(queries.branch_false.contains("FILTER( !( ?v < 5 ) )"));
        assert!(queries
            .branch_error
            .contains("FILTER( COALESCE( IF( ?v < 5 , false, false), true) )"));
    }

    #[test]
    fn run_select_rejects_a_boolean_shape() {
        struct AskOnly;
        impl SparqlEngine for AskOnly {
            fn name(&self) -> &str {
                "ask-only"
            }
            fn select(&self, _sparql: &str) -> Result<QueryResults, EngineFailure> {
                Ok(QueryResults::Boolean(true))
            }
        }
        let err = run_select(&AskOnly, "SELECT * WHERE { ?s ?p ?o }").unwrap_err();
        assert_eq!(err.kind, FailureKind::InvalidResults);
    }
}

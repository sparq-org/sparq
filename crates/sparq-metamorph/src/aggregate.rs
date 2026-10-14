//! **Aggregate-aware partitioning: `COUNT`/`SUM` recombined over the three TLP
//! branches.** sq-gum8.12
//!
//! TLP for SQL has aggregate variants (Rigger & Su, OOPSLA 2020 §3.3): partition the
//! table, aggregate each partition, and recombine — `COUNT` and `SUM` by addition. The
//! *partitioning* transfers to SPARQL unchanged (it is the same three-branch trichotomy
//! [`crate::tlp`] derives), but the *recombination* does not: SQL aggregates ignore
//! `NULL`s, while a SPARQL aggregate has to answer for an evaluation **error**, and the
//! spec's answer is neither "ignore it" nor "fail the query". This module re-derives the
//! law for SPARQL, in a form that is **invariant** under the one place where engines and
//! readings of the spec legitimately differ.
//!
//! # Setup
//!
//! With no `GROUP BY`, a query with an aggregate treats the whole solution bag as a
//! single group and returns exactly **one** solution (SPARQL 1.1 §18.5.1 `Group`, the
//! implicit single-group case; §18.5.1.1 also fixes the empty-group values —
//! `Count({}) = 0`, `Sum({}) = 0`). So each of the four queries below returns one row
//! whose aggregate cell is either **bound** or **unbound**; [`check_tlp_aggregate`]
//! treats any other row count as a violation, since the spec leaves no latitude there.
//!
//! Let `agg` be the aggregate under test and `Ω = Ω_t ⊎ Ω_f ⊎ Ω_e` the trichotomy of
//! [`crate::tlp`] (branch filters built by the shared `tlp::branch_filters`).
//!
//! # The three per-row outcomes, and why they recombine
//!
//! For each solution `μ` in the group, the aggregated expression `e` evaluates to a
//! value, to **unbound**, or to an **error**. SPARQL treats them differently, and — the
//! load-bearing point — it treats each of them **row-locally**:
//!
//! * *value* — contributes to the fold.
//! * *unbound* — the row is **removed** from the aggregation ("aggregate over the bound
//!   values": `SUM(?x)` over a column that is unbound on some rows still sums the rows
//!   that do have a value, rather than collapsing). Removal is per row.
//! * *error* — this is where readings differ. Under the **fatal** reading an erroring
//!   member makes the *whole* aggregate a type error, and an aggregate that errors leaves
//!   its projected variable **unbound** for that row (an error in an `Extend` expression
//!   leaves the variable unbound while keeping the solution, §18.5). Under the **drop**
//!   reading erroring members are removed from the multiset like unbound ones and the
//!   aggregate stays bound. sparq is not uniform across the two — `SUM` is fatal-on-error
//!   while `COUNT` drops the erroring member — and that is *measured*, not assumed:
//!   `tests/oracle_self_tests.rs` pins both behaviours against the real engine. (sparq's
//!   evaluator attributes its `SUM` choice to the W3C `agg-err-01` test; that attribution
//!   is quoted from `crates/sparq-engine/src/exec.rs` and was not independently
//!   re-derived here — which is exactly why the law below is written to be indifferent
//!   to which reading an engine under test picked.)
//!
//! Either way the *error status* of a group is a row-local predicate lifted by an
//! **`OR` over the group's rows** (fatal reading: "some row errored") or is identically
//! false (drop reading). Both distribute over a partition, which is what lets the
//! relation below be stated without committing to one reading:
//!
//! ```text
//! (1)  cell(base) is unbound   ⟺   cell of SOME branch is unbound
//! (2)  all four cells bound    ⇒   cell(base) = cell(t) + cell(f) + cell(e)
//! ```
//!
//! *Why (1).* Under the fatal reading, the offending row lies in exactly one branch
//! (the trichotomy is a partition), so base errors iff that branch errors; under the drop
//! reading both sides are always false. *Why (2).* `Count` and `Sum` are folds of a
//! commutative, associative operation with an identity over the (row-locally filtered)
//! member multiset, and the member multisets of the branches partition the base's, so
//! the folds add. An **empty** branch contributes the identity: `Count({}) = 0` and
//! `Sum({}) = 0` (§18.5.1.1) — not unbound — which is exactly what makes (2) hold when
//! a partition is empty. (`MIN`/`MAX` over an empty group *are* unbound, and `AVG` is
//! not additive at all; both are therefore out of scope here — see below.)
//!
//! # The exactness precondition (why `SumInteger`, not `Sum`)
//!
//! (2) is an *exact* equation, so the fold must be exact. SPARQL `SUM` uses XPath
//! numeric addition with type promotion: over `xsd:integer` (and over `xsd:decimal`) it
//! is exact and associative, but as soon as one member promotes the fold to
//! `xsd:double`, floating-point addition is **no longer associative** and the base sum
//! may legitimately differ from the sum of the branch sums in the last bits — a false
//! violation. [`PartitionedAggregate::SumInteger`] therefore carries the precondition
//! that the aggregated expression yields `xsd:integer`, unbound, or an error on every
//! solution, and the checker enforces it: a bound cell that is not an `xsd:integer`
//! literal is reported as a [`crate::verdict::FailureKind::Harness`] failure — never as
//! a wrong-result claim. Integer-valued *error fuel* is easy to write inside the
//! precondition (`SUM(?w + 1)` errors on an unbound `?w`; a `xsd:integer(?v)` cast
//! errors on a non-castable term), so the error law (1) stays exercised.
//!
//! # Scope
//!
//! * In scope: `COUNT(*)`, `COUNT(e)`, `SUM(e)` with `e` integer-valued (above).
//! * Out of scope: `AVG` (not additive — recombining it needs the paired counts and
//!   reintroduces division rounding), `MIN`/`MAX` (recombinable in principle, but their
//!   empty-group value is *unbound*, which collides with the error-status law (1) and
//!   would make an empty branch indistinguishable from an errored one),
//!   `GROUP_CONCAT`/`SAMPLE` (order/choice are implementation-defined), `GROUP BY` (the
//!   law generalises per group, but comparing grouped results needs a key-wise join and
//!   is a separate oracle), and `DISTINCT` inside the aggregate (`SUM(DISTINCT ?x)` is
//!   *not* additive over a partition: a value occurring in two branches is counted twice
//!   by the branch sums and once by the base).
//! * Preconditions 1, 2 and 4 of [`crate::tlp`] apply unchanged. In particular `EXISTS`
//!   stays excluded — pending the SPARQL 1.2 resolution of its substitution semantics —
//!   in `c` *and* in the aggregated expression.

use sparq_difftest::{Solution, Term};

use crate::engine::SparqlEngine;
use crate::tlp::{branch_filters, run_select};
use crate::verdict::{EngineFailure, FailureKind, OracleKind, Verdict, Violation};

/// `xsd:integer` — the only bound cell datatype the exactness precondition admits.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// The aggregate whose partition recombination is under test.
///
/// Every variant is a fold that is additive over a partition of the group; see the
/// module docs for the derivation and for what is deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionedAggregate {
    /// `COUNT(*)` — the group's row cardinality. Never unbound, never errors.
    CountStar,
    /// `COUNT(e)` — counts the members of the group on which `e` contributes.
    Count(String),
    /// `SUM(e)` where `e` yields `xsd:integer`, unbound, or an error on every solution
    /// (the exactness precondition — see the module docs).
    SumInteger(String),
}

impl PartitionedAggregate {
    /// The SPARQL aggregate expression text (`COUNT(*)`, `COUNT( e )`, `SUM( e )`).
    pub fn render(&self) -> String {
        match self {
            PartitionedAggregate::CountStar => "COUNT(*)".to_string(),
            PartitionedAggregate::Count(expr) => format!("COUNT( {expr} )"),
            PartitionedAggregate::SumInteger(expr) => format!("SUM( {expr} )"),
        }
    }

    /// The aggregated expression, if any (`COUNT(*)` has none). Used to keep the
    /// projected aggregate variable fresh.
    fn expr(&self) -> Option<&str> {
        match self {
            PartitionedAggregate::CountStar => None,
            PartitionedAggregate::Count(expr) | PartitionedAggregate::SumInteger(expr) => {
                Some(expr)
            }
        }
    }
}

/// The four queries of one aggregate-partitioning instance, plus the projected
/// aggregate variable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlpAggregateQueries {
    /// `SELECT ( agg AS ?<var> ) WHERE { P }` — the unpartitioned base.
    pub base: String,
    /// The same aggregate over the `ebv(c, μ) = true` partition.
    pub branch_true: String,
    /// The same aggregate over the `ebv(c, μ) = false` partition.
    pub branch_false: String,
    /// The same aggregate over the `ebv(c, μ) = error` partition.
    pub branch_error: String,
    /// The fresh aggregate variable name (no leading `?`).
    pub agg_var: String,
}

/// Build the four aggregate-partitioning queries for pattern `P`, filter expression `c`
/// and aggregate `agg`. The aggregate variable is chosen fresh: `tlpAgg`, suffixed with
/// `x` until it occurs in none of `pattern`, `predicate`, or the aggregated expression.
pub fn tlp_aggregate_queries(
    pattern: &str,
    predicate: &str,
    agg: &PartitionedAggregate,
) -> TlpAggregateQueries {
    let mut agg_var = String::from("tlpAgg");
    let expr = agg.expr().unwrap_or("");
    while pattern.contains(&agg_var) || predicate.contains(&agg_var) || expr.contains(&agg_var) {
        agg_var.push('x');
    }
    let projection = format!("SELECT ( {} AS ?{agg_var} )", agg.render());
    let [keep_true, keep_false, keep_error] = branch_filters(predicate);
    TlpAggregateQueries {
        base: format!("{projection} WHERE {{ {pattern} }}"),
        branch_true: format!("{projection} WHERE {{ {pattern} {keep_true} }}"),
        branch_false: format!("{projection} WHERE {{ {pattern} {keep_false} }}"),
        branch_error: format!("{projection} WHERE {{ {pattern} {keep_error} }}"),
        agg_var,
    }
}

/// One aggregate cell: bound to an exact integer, or unbound (the aggregate errored, or
/// the engine left the variable unbound for any other reason).
type Cell = Option<i128>;

/// Read the single aggregate cell of one query's result, enforcing the two things the
/// derivation needs: exactly one solution, and a bound cell that is an `xsd:integer`.
/// The `Err` arm carries the ready-made non-pass verdict (a violation for a row-count
/// breach, a fail-closed harness failure for a precondition breach).
fn read_cell(
    engine: &dyn SparqlEngine,
    queries: &TlpAggregateQueries,
    query: &str,
    solutions: &[Solution],
) -> Result<Cell, Verdict> {
    if solutions.len() != 1 {
        return Err(Verdict::Violation(Violation {
            oracle: OracleKind::TlpAggregate,
            engines: vec![engine.name().to_string()],
            queries: vec![query.to_string()],
            detail: format!(
                "an aggregate query without GROUP BY must return exactly one solution \
                 (SPARQL 1.1 §18.5.1), got {}",
                solutions.len()
            ),
        }));
    }
    match solutions[0].get(&queries.agg_var) {
        None => Ok(None),
        Some(Term::Literal {
            lexical,
            datatype,
            lang: None,
        }) if datatype == XSD_INTEGER => match lexical.trim().parse::<i128>() {
            Ok(value) => Ok(Some(value)),
            Err(e) => Err(Verdict::EngineFailure(EngineFailure {
                engine: engine.name().to_string(),
                query: query.to_string(),
                kind: FailureKind::Harness,
                message: format!("aggregate cell {lexical:?} is not a readable xsd:integer: {e}"),
            })),
        },
        Some(other) => Err(Verdict::EngineFailure(EngineFailure {
            engine: engine.name().to_string(),
            query: query.to_string(),
            kind: FailureKind::Harness,
            message: format!(
                "aggregate cell is not an xsd:integer ({other:?}); the exactness \
                 precondition requires an integer-valued aggregate"
            ),
        })),
    }
}

/// Render a cell for the verdict detail.
fn show(cell: Cell) -> String {
    match cell {
        Some(value) => value.to_string(),
        None => "unbound".to_string(),
    }
}

/// Check the aggregate-partitioning relation on one engine: the base aggregate must be
/// unbound exactly when some branch aggregate is, and otherwise equal the sum of the
/// three branch aggregates (see the module docs for the derivation).
///
/// Fail-closed: a query that does not evaluate, or a bound cell outside the exactness
/// precondition, yields [`Verdict::EngineFailure`] — never a pass, never a wrong-result
/// claim.
pub fn check_tlp_aggregate(
    engine: &dyn SparqlEngine,
    pattern: &str,
    predicate: &str,
    agg: &PartitionedAggregate,
) -> Verdict {
    let queries = tlp_aggregate_queries(pattern, predicate, agg);
    let mut cells = Vec::with_capacity(4);
    for query in [
        &queries.base,
        &queries.branch_true,
        &queries.branch_false,
        &queries.branch_error,
    ] {
        let solutions = match run_select(engine, query) {
            Ok(rows) => rows,
            Err(failure) => return Verdict::EngineFailure(failure),
        };
        match read_cell(engine, &queries, query, &solutions) {
            Ok(cell) => cells.push(cell),
            Err(verdict) => return verdict,
        }
    }
    let (base, branches) = cells.split_first().expect("four cells were pushed");

    let detail = format!(
        "aggregate={} base={} true={} false={} error={}",
        agg.render(),
        show(*base),
        show(branches[0]),
        show(branches[1]),
        show(branches[2])
    );
    let violation = |reason: &str| {
        Verdict::Violation(Violation {
            oracle: OracleKind::TlpAggregate,
            engines: vec![engine.name().to_string()],
            queries: vec![
                queries.base.clone(),
                queries.branch_true.clone(),
                queries.branch_false.clone(),
                queries.branch_error.clone(),
            ],
            detail: format!("{reason} ({detail})"),
        })
    };

    // Law (1): the error status of the group is an OR over its rows, so it must agree
    // between the base and the partition.
    let some_branch_unbound = branches.iter().any(Option::is_none);
    if base.is_none() != some_branch_unbound {
        return violation(
            "aggregate error status disagrees: the base aggregate is unbound iff some \
             branch aggregate is",
        );
    }
    // Law (2): with every cell bound, the folds add.
    if let Some(base_value) = *base {
        let mut total: i128 = 0;
        for branch in branches {
            let value = branch.expect("all branches bound in this arm");
            total = match total.checked_add(value) {
                Some(sum) => sum,
                None => {
                    return Verdict::EngineFailure(EngineFailure {
                        engine: engine.name().to_string(),
                        query: queries.base.clone(),
                        kind: FailureKind::Harness,
                        message: format!("branch aggregate sum overflows i128 ({detail})"),
                    })
                }
            };
        }
        if base_value != total {
            return violation(&format!(
                "base aggregate differs from the sum over the partition (sum={total})"
            ));
        }
    }
    Verdict::Pass { detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn queries() -> TlpAggregateQueries {
        tlp_aggregate_queries(
            "?s <http://example.org/w> ?w",
            "?w < 5",
            &PartitionedAggregate::SumInteger("?w".to_string()),
        )
    }

    #[test]
    fn aggregate_queries_project_the_aggregate_and_share_the_branch_filters() {
        let q = queries();
        assert_eq!(
            q.base,
            "SELECT ( SUM( ?w ) AS ?tlpAgg ) WHERE { ?s <http://example.org/w> ?w }"
        );
        assert!(q.branch_true.contains("FILTER( ?w < 5 )"));
        assert!(q.branch_false.contains("FILTER( !( ?w < 5 ) )"));
        assert!(q
            .branch_error
            .contains("FILTER( COALESCE( IF( ?w < 5 , false, false), true) )"));
        assert_eq!(q.agg_var, "tlpAgg");
    }

    #[test]
    fn count_variants_render_as_sparql() {
        assert_eq!(PartitionedAggregate::CountStar.render(), "COUNT(*)");
        assert_eq!(
            PartitionedAggregate::Count("?v".into()).render(),
            "COUNT( ?v )"
        );
        assert!(
            tlp_aggregate_queries("?s ?p ?o", "true", &PartitionedAggregate::CountStar)
                .base
                .starts_with("SELECT ( COUNT(*) AS ?tlpAgg )")
        );
    }

    #[test]
    fn the_aggregate_variable_is_kept_fresh_against_pattern_predicate_and_expression() {
        assert_eq!(
            tlp_aggregate_queries("?tlpAgg ?p ?o", "true", &PartitionedAggregate::CountStar)
                .agg_var,
            "tlpAggx"
        );
        assert_eq!(
            tlp_aggregate_queries(
                "?s ?p ?o",
                "true",
                &PartitionedAggregate::SumInteger("?tlpAgg".into())
            )
            .agg_var,
            "tlpAggx"
        );
    }

    /// The exactness precondition is enforced fail-closed: a non-integer bound cell is a
    /// harness failure, not a wrong-result claim.
    #[test]
    fn a_non_integer_bound_cell_is_a_harness_failure() {
        struct Named;
        impl SparqlEngine for Named {
            fn name(&self) -> &str {
                "named"
            }
            fn select(&self, _sparql: &str) -> Result<sparq_difftest::QueryResults, EngineFailure> {
                unreachable!("read_cell is called directly in this test")
            }
        }
        let q = queries();
        let decimal = Term::Literal {
            lexical: "3.5".into(),
            datatype: "http://www.w3.org/2001/XMLSchema#decimal".into(),
            lang: None,
        };
        let solution: Solution = BTreeMap::from([(q.agg_var.clone(), decimal)]);
        match read_cell(&Named, &q, &q.base, std::slice::from_ref(&solution)) {
            Err(Verdict::EngineFailure(f)) => assert_eq!(f.kind, FailureKind::Harness),
            other => panic!("expected a harness failure, got {other:?}"),
        }
        // An unbound cell is a legitimate outcome (the aggregate errored), not a failure.
        let empty: Solution = BTreeMap::new();
        assert_eq!(
            read_cell(&Named, &q, &q.base, std::slice::from_ref(&empty)),
            Ok(None)
        );
        // Anything other than exactly one solution breaks the §18.5.1 single-group rule.
        match read_cell(&Named, &q, &q.base, &[]) {
            Err(Verdict::Violation(v)) => assert_eq!(v.oracle, OracleKind::TlpAggregate),
            other => panic!("expected a violation, got {other:?}"),
        }
    }

    #[test]
    fn integer_cells_accept_the_non_canonical_lexicals_xsd_integer_allows() {
        struct Named;
        impl SparqlEngine for Named {
            fn name(&self) -> &str {
                "named"
            }
            fn select(&self, _sparql: &str) -> Result<sparq_difftest::QueryResults, EngineFailure> {
                unreachable!("read_cell is called directly in this test")
            }
        }
        let q = queries();
        for (lexical, expected) in [("7", 7i128), ("+7", 7), ("007", 7), ("-7", -7)] {
            let term = Term::Literal {
                lexical: lexical.into(),
                datatype: XSD_INTEGER.into(),
                lang: None,
            };
            let solution: Solution = BTreeMap::from([(q.agg_var.clone(), term)]);
            assert_eq!(
                read_cell(&Named, &q, &q.base, std::slice::from_ref(&solution)),
                Ok(Some(expected)),
                "lexical {lexical}"
            );
        }
    }
}

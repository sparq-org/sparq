//! **Cross-engine differential oracle** — the same query and data on two or more
//! engines, results compared with sparq-difftest's engine-independent comparators.
//! sq-gum8.6
//!
//! Unlike TLP/NoREC (which need only one engine's *internal* consistency), the
//! differential oracle needs an agreement standard across engines. That standard is
//! deliberately **not** any engine's own value code: results are compared through
//! [`sparq_difftest`]'s independent normalisation (see that crate's load-bearing
//! independence constraint), so a value bug shared by harness and engine cannot cancel.
//!
//! # Comparison semantics
//!
//! * `SELECT` results: **multiset equality** over value-canonical solution keys
//!   ([`sparq_difftest::multiset_equal`]) — order-insensitive, duplicate-preserving.
//!   Queries with `ORDER BY` are out of scope for [`check_differential`]; the ordered
//!   mode is [`check_differential_ordered`] (see *ORDER BY differential mode* below).
//! * `ASK` results: boolean equality.
//! * Shape mismatch (one engine answers a `SELECT`, another a boolean): classified as
//!   [`FailureKind::InvalidResults`] — a protocol/driver problem to triage, **not** a
//!   wrong-result claim (fail-closed).
//!
//! # Scope limits (documented, generator-enforced)
//!
//! * **Blank nodes**: sparq-difftest compares blank nodes by engine-local label, which
//!   is only meaningful within one engine (cross-engine label agreement is not required
//!   by SPARQL). The query/data generator therefore emits no blank nodes; cross-engine
//!   blank-node isomorphism is a separate difftest DAG node (bead `sq-qcnn.7`).
//! * **Implementation-defined behaviour**: engines may legitimately differ where the
//!   spec leaves latitude (extended-datatype comparisons, some canonical lexical
//!   choices — the value-canonical keying absorbs the known lexical variance). A
//!   reported divergence is a *candidate* bug; campaign triage attributes it to an
//!   engine (or to spec latitude) before it enters the found-bug ledger with an
//!   upstream issue link.
//!
//! # `ORDER BY` differential mode sq-gum8.12
//!
//! [`check_differential`] deliberately throws the row order away, so it cannot see an
//! ordering bug at all: an engine that sorts wrongly returns the same *bag* and passes.
//! [`check_differential_ordered`] closes that gap — and the reason it is a separate
//! entry point rather than "compare the sequences" is that **plain sequence equality is
//! not a sound oracle for `ORDER BY`**.
//!
//! SPARQL's `ORDER BY` is only a *partial* specification of the result sequence (SPARQL
//! 1.1 §15.1 `ORDER BY`, algebra §18.5 `OrderBy`): solutions that compare equal on every
//! sort condition may be returned in **any** relative order, and two conforming engines
//! may differ there. The sound relation is therefore equality **up to permutation within
//! each sort-key equivalence class**, which is exactly
//! [`sparq_difftest::order_by_equal`]: both sequences are cut into maximal runs of
//! consecutive rows sharing the sort key, the runs must line up in order and by key, and
//! each pair of corresponding runs must be multiset-equal. Cross-run reordering — a
//! genuine order bug — is caught; within-run permutation — spec-permitted latitude — is
//! not reported. Both halves of that statement are pinned by the self-tests, because an
//! order oracle that flags legal latitude is worse than none.
//!
//! ## Scope preconditions (ordered mode)
//!
//! 1. **The sort variables must be pairwise comparable across the whole result.** §15.1
//!    fixes an order *between* term kinds (unbound < blank nodes < IRIs < literals) and,
//!    within literals, only for the datatypes the operator mapping orders; for anything
//!    else the ordering "is implementation-defined" while still being a total order. A
//!    mixed-datatype sort column (say integers next to language-tagged strings) therefore
//!    lets two conforming engines disagree, and a "violation" would measure spec latitude
//!    rather than an engine bug. Sort on a single-datatype (or unbound) column.
//! 2. **`sort_vars` must be the query's actual `ORDER BY` variable list, in order.** The
//!    comparator partitions runs by exactly these variables; passing fewer merges runs
//!    (weaker but still sound), passing *more* or different ones splits runs that the
//!    engine was free to order either way and can manufacture a false violation.
//!    [`ordered_query`] builds the query and returns the list, so the two cannot drift.
//! 3. Sort *conditions* are restricted to bare variables (no `ORDER BY DESC(…)` or
//!    expression conditions): the comparator keys runs by variable value, so an
//!    expression condition has no key to partition by. Ascending order is the default
//!    (§15.1) and the run partition is direction-agnostic anyway — it only cares which
//!    rows tie.
//!
//! Note the run-key is [`sparq_difftest::canonical_key`], which distinguishes datatypes:
//! `"1"^^xsd:integer` and `"1.0"^^xsd:decimal` are `=`-equal (so they may tie under
//! `ORDER BY`) yet key apart, splitting a legitimate tie run. Precondition 1 rules that
//! case out; it is the same restriction, seen from the comparator side.

use sparq_difftest::{multiset_equal, order_by_equal, QueryResults};

use crate::engine::SparqlEngine;
use crate::verdict::{EngineFailure, FailureKind, OracleKind, Verdict, Violation};

/// An `ORDER BY` query plus the sort-variable list its comparison must use — returned
/// together so precondition 2 of the ordered mode cannot be violated by drift.
/// sq-gum8.12
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedQuery {
    /// `SELECT * WHERE { P FILTER( c ) } ORDER BY ?a ?b …`.
    pub query: String,
    /// The `ORDER BY` variable names (no leading `?`), in order.
    pub sort_vars: Vec<String>,
}

/// Build the canonical ordered campaign query for pattern `P`, filter expression `c`,
/// and sort variables `sort_vars` (names without `?`, in `ORDER BY` order).
///
/// An empty `sort_vars` emits no `ORDER BY` clause, and the ordered check then degrades
/// to the unordered multiset relation (`order_by_equal` with an empty key is
/// `multiset_equal`) — sound, but pointless; pass at least one variable. See the module
/// docs for the scope preconditions on which variables are admissible.
pub fn ordered_query(pattern: &str, predicate: &str, sort_vars: &[&str]) -> OrderedQuery {
    let mut query = format!("SELECT * WHERE {{ {pattern} FILTER( {predicate} ) }}");
    if !sort_vars.is_empty() {
        let keys = sort_vars
            .iter()
            .map(|v| format!("?{v}"))
            .collect::<Vec<_>>()
            .join(" ");
        query.push_str(&format!(" ORDER BY {keys}"));
    }
    OrderedQuery {
        query,
        sort_vars: sort_vars.iter().map(|v| (*v).to_string()).collect(),
    }
}

/// How two `SELECT` result sequences are compared.
enum Comparison<'a> {
    /// Order-insensitive multiset equality (the default oracle).
    Unordered,
    /// `ORDER BY`-aware: equality up to permutation within each sort-key equivalence
    /// class over these sort variables.
    Ordered(&'a [&'a str]),
}

/// Run `query` on every engine and require agreement (see the module docs for the
/// comparison semantics). At least two engines are required — fewer is a
/// [`FailureKind::Harness`] failure, never a vacuous pass.
///
/// Fail-closed: if any engine fails to evaluate the query, the verdict is
/// [`Verdict::EngineFailure`] for the first failing engine.
pub fn check_differential(engines: &[&dyn SparqlEngine], query: &str) -> Verdict {
    check_differential_with(engines, query, Comparison::Unordered)
}

/// The **`ORDER BY` differential mode**: run the ordered `query` on every engine and
/// require agreement up to permutation within each sort-key equivalence class over
/// `sort_vars` ([`sparq_difftest::order_by_equal`]).
///
/// `sort_vars` must be the query's own `ORDER BY` variable list, in order — use
/// [`ordered_query`] to build the pair. The scope preconditions (comparable sort column,
/// bare-variable sort conditions) are in the module docs; violating them measures spec
/// latitude rather than an engine bug.
///
/// `ASK` results and every failure mode behave exactly as in [`check_differential`].
pub fn check_differential_ordered(
    engines: &[&dyn SparqlEngine],
    query: &str,
    sort_vars: &[&str],
) -> Verdict {
    check_differential_with(engines, query, Comparison::Ordered(sort_vars))
}

fn check_differential_with(
    engines: &[&dyn SparqlEngine],
    query: &str,
    comparison: Comparison<'_>,
) -> Verdict {
    let oracle = match comparison {
        Comparison::Unordered => OracleKind::Differential,
        Comparison::Ordered(_) => OracleKind::DifferentialOrdered,
    };
    if engines.len() < 2 {
        return Verdict::EngineFailure(EngineFailure {
            engine: String::new(),
            query: query.to_string(),
            kind: FailureKind::Harness,
            message: format!(
                "differential check needs at least 2 engines, got {}",
                engines.len()
            ),
        });
    }

    let mut results: Vec<(&str, QueryResults)> = Vec::with_capacity(engines.len());
    for engine in engines {
        match engine.select(query) {
            Ok(r) => results.push((engine.name(), r)),
            Err(failure) => return Verdict::EngineFailure(failure),
        }
    }

    let (reference_name, reference) = &results[0];
    for (name, candidate) in &results[1..] {
        let (agree, detail) = match (reference, candidate) {
            (
                QueryResults::Solutions { solutions: a, .. },
                QueryResults::Solutions { solutions: b, .. },
            ) => {
                let agree = match comparison {
                    Comparison::Unordered => multiset_equal(a, b),
                    Comparison::Ordered(sort_vars) => order_by_equal(a, b, sort_vars),
                };
                (
                    agree,
                    format!("{reference_name}={} rows, {name}={} rows", a.len(), b.len()),
                )
            }
            (QueryResults::Boolean(a), QueryResults::Boolean(b)) => (
                a == b,
                format!("{reference_name}={a}, {name}={b}"),
            ),
            _ => {
                return Verdict::EngineFailure(EngineFailure {
                    engine: (*name).to_string(),
                    query: query.to_string(),
                    kind: FailureKind::InvalidResults,
                    message: format!(
                        "result-shape mismatch vs {reference_name} (SELECT vs ASK shape)"
                    ),
                })
            }
        };
        if !agree {
            return Verdict::Violation(Violation {
                oracle,
                engines: vec![(*reference_name).to_string(), (*name).to_string()],
                queries: vec![query.to_string()],
                detail: format!("results disagree: {detail}"),
            });
        }
    }
    Verdict::Pass {
        detail: format!("{} engines agree", results.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canned(QueryResults);
    impl SparqlEngine for Canned {
        fn name(&self) -> &str {
            "canned"
        }
        fn select(&self, _sparql: &str) -> Result<QueryResults, EngineFailure> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn fewer_than_two_engines_is_a_harness_failure_not_a_pass() {
        let one = Canned(QueryResults::Boolean(true));
        let verdict = check_differential(&[&one], "ASK { ?s ?p ?o }");
        match verdict {
            Verdict::EngineFailure(f) => assert_eq!(f.kind, FailureKind::Harness),
            other => panic!("expected a harness failure, got {other:?}"),
        }
    }

    #[test]
    fn boolean_agreement_passes_and_disagreement_is_a_violation() {
        let yes = Canned(QueryResults::Boolean(true));
        let also_yes = Canned(QueryResults::Boolean(true));
        let no = Canned(QueryResults::Boolean(false));
        assert!(check_differential(&[&yes, &also_yes], "ASK {}").is_pass());
        assert!(check_differential(&[&yes, &no], "ASK {}").is_violation());
    }

    #[test]
    fn shape_mismatch_is_an_engine_failure_not_a_wrong_result() {
        let boolean = Canned(QueryResults::Boolean(true));
        let solutions = Canned(QueryResults::Solutions {
            vars: vec![],
            solutions: vec![],
        });
        let verdict = check_differential(&[&boolean, &solutions], "ASK {}");
        match verdict {
            Verdict::EngineFailure(f) => assert_eq!(f.kind, FailureKind::InvalidResults),
            other => panic!("expected an engine failure, got {other:?}"),
        }
    }
}

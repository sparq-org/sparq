//! **NoREC re-derived for SPARQL: the non-optimizable-rewrite cardinality oracle.**
//! sq-gum8.6
//!
//! NoREC for SQL (Rigger & Su, ESEC/FSE 2020) rewrites `SELECT * FROM t WHERE p` into a
//! form that evaluates `p` **per row in the projection**, where the optimizer cannot use
//! it for index selection or predicate push-down, and cross-checks the cardinalities.
//! A mismatch means the optimized and non-optimized evaluation paths of the *same
//! engine* disagree — a wrong-result logic bug on one of them.
//!
//! # The SPARQL rewrite
//!
//! For pattern `P` and filter expression `c`, [`norec_queries`] builds:
//!
//! * **optimized** — `SELECT * WHERE { P FILTER( c ) }`: the form every engine's
//!   optimizer targets (filter push-down, index-driven evaluation, join reordering
//!   around the selective predicate).
//! * **non-optimizable rewrite** — `SELECT ( IF( c , true, false) AS ?flag ) WHERE { P }`:
//!   the predicate moved to **projection position**. The engine must materialise
//!   `eval(P)` in full and evaluate `c` once per solution to bind `?flag`; the
//!   predicate can no longer prune pattern evaluation.
//!
//! The metamorphic relation, under SPARQL 1.1 semantics:
//!
//! * `FILTER(c)` keeps exactly the solutions with `ebv(c, μ) = true` (§17.2 "Filter
//!   Evaluation"; see the [`crate::tlp`] module docs for the full three-valued case
//!   analysis).
//! * `IF(c, true, false)` returns `true` exactly when `ebv(c, μ) = true` — the same
//!   EBV, by §17.4.1.2 (`IF`). When `ebv(c, μ) = false` it returns `false`; when `c`
//!   **errors** (type error, unbound variable), `IF` errors, and an error in a
//!   projection expression leaves `?flag` **unbound while keeping the row** (algebra
//!   `Extend`, §18.5: when the expression evaluates to an error, `Extend` returns the
//!   solution mapping unchanged, so the variable stays unbound; a `SELECT`
//!   expression is `Extend` + `Project`, §18.2.4.4). So the
//!   rewrite preserves the full trichotomy observably: `true` / `false` / unbound.
//! * `Project` preserves bag cardinality (no `DISTINCT`), so the rewrite returns
//!   exactly `|eval(P)|` rows.
//!
//! Therefore:
//!
//! ```text
//! |eval(optimized)|  =  #{ rows of the rewrite with ?flag bound to boolean true }
//! ```
//!
//! [`check_norec`] counts the `true`-flagged rows **harness-side** (accepting both
//! valid `xsd:boolean` lexical forms, `true` and `1`) rather than wrapping the rewrite
//! in a `COUNT` aggregate — a nested aggregate would hand the expression back to the
//! very optimization machinery the oracle is trying to bypass.
//!
//! # Honesty note: "non-optimizable" is a structural heuristic, not a theorem
//!
//! A semantics-preserving optimizer *may* legally evaluate `c` early even in
//! projection position. The claim — same as the original NoREC paper's — is
//! architectural: in surveyed engine implementations, filter position feeds the
//! plan-level optimization paths (push-down, index selection) while projection
//! expressions are evaluated by a generic per-row expression interpreter after
//! pattern matching, so the two forms take different code paths and a divergence
//! exposes a bug in one of them. The oracle detects the divergence; it does not prove
//! which path is wrong.
//!
//! # Scope preconditions
//!
//! Same as TLP (deterministic `c`; no `EXISTS`; plain `SELECT` without
//! `DISTINCT`/`ORDER`/`LIMIT`/aggregates at the oracle level) — see
//! [`crate::tlp`] module docs, *Scope preconditions*. Additionally the flag variable
//! must not occur in `P` or `c`; [`norec_queries`] picks a fresh name automatically.

use crate::engine::SparqlEngine;
use crate::tlp::run_select;
use crate::verdict::{OracleKind, Verdict, Violation};

/// The two queries of one NoREC instance, plus the projected flag variable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NorecQueries {
    /// `SELECT * WHERE { P FILTER( c ) }` — the optimizer-visible form.
    pub optimized: String,
    /// `SELECT ( IF( c , true, false) AS ?<flag> ) WHERE { P }` — predicate in
    /// projection position.
    pub rewritten: String,
    /// The fresh flag variable name (no leading `?`).
    pub flag_var: String,
}

/// Build the optimized/rewritten query pair for pattern `P` and filter expression `c`
/// (SPARQL text, absolute IRIs). The flag variable is chosen fresh: `norecFlag`,
/// suffixed with `x` until it occurs in neither `pattern` nor `predicate`.
pub fn norec_queries(pattern: &str, predicate: &str) -> NorecQueries {
    let mut flag_var = String::from("norecFlag");
    while pattern.contains(&flag_var) || predicate.contains(&flag_var) {
        flag_var.push('x');
    }
    NorecQueries {
        optimized: format!("SELECT * WHERE {{ {pattern} FILTER( {predicate} ) }}"),
        rewritten: format!(
            "SELECT ( IF( {predicate} , true, false) AS ?{flag_var} ) WHERE {{ {pattern} }}"
        ),
        flag_var,
    }
}

/// True iff `term` is an `xsd:boolean` literal whose lexical form is a valid
/// boolean-true (`true` or `1`).
fn is_boolean_true(term: &sparq_difftest::Term) -> bool {
    match term {
        sparq_difftest::Term::Literal {
            lexical,
            datatype,
            lang: None,
        } => {
            datatype == "http://www.w3.org/2001/XMLSchema#boolean"
                && (lexical == "true" || lexical == "1")
        }
        _ => false,
    }
}

/// Check the NoREC relation on one engine: the optimized query's cardinality must
/// equal the number of `true`-flagged rows of the non-optimizable rewrite.
///
/// Fail-closed: if either query fails to evaluate, the verdict is
/// [`Verdict::EngineFailure`].
pub fn check_norec(engine: &dyn SparqlEngine, pattern: &str, predicate: &str) -> Verdict {
    let queries = norec_queries(pattern, predicate);
    let optimized = match run_select(engine, &queries.optimized) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };
    let rewritten = match run_select(engine, &queries.rewritten) {
        Ok(rows) => rows,
        Err(failure) => return Verdict::EngineFailure(failure),
    };

    let flagged_true = rewritten
        .iter()
        .filter(|solution| solution.get(&queries.flag_var).is_some_and(is_boolean_true))
        .count();
    let detail = format!(
        "optimized={} rewrite-rows={} rewrite-true={}",
        optimized.len(),
        rewritten.len(),
        flagged_true
    );
    if optimized.len() == flagged_true {
        Verdict::Pass { detail }
    } else {
        Verdict::Violation(Violation {
            oracle: OracleKind::Norec,
            engines: vec![engine.name().to_string()],
            queries: vec![queries.optimized, queries.rewritten],
            detail: format!("optimized cardinality differs from rewrite true-count ({detail})"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norec_queries_moves_the_predicate_into_projection() {
        let queries = norec_queries("?s <http://example.org/v> ?v", "?v < 5");
        assert!(queries.optimized.contains("FILTER( ?v < 5 )"));
        assert!(queries
            .rewritten
            .starts_with("SELECT ( IF( ?v < 5 , true, false) AS ?norecFlag )"));
        assert_eq!(queries.flag_var, "norecFlag");
        assert!(!queries.rewritten.contains("FILTER"));
    }

    #[test]
    fn norec_queries_picks_a_fresh_flag_variable() {
        let queries = norec_queries("?norecFlag <http://example.org/v> ?v", "?v < 5");
        assert_eq!(queries.flag_var, "norecFlagx");
    }

    #[test]
    fn is_boolean_true_accepts_both_valid_lexicals_and_nothing_else() {
        let t = |lexical: &str, datatype: &str| sparq_difftest::Term::Literal {
            lexical: lexical.into(),
            datatype: datatype.into(),
            lang: None,
        };
        let xsd_boolean = "http://www.w3.org/2001/XMLSchema#boolean";
        assert!(is_boolean_true(&t("true", xsd_boolean)));
        assert!(is_boolean_true(&t("1", xsd_boolean)));
        assert!(!is_boolean_true(&t("false", xsd_boolean)));
        assert!(!is_boolean_true(&t("0", xsd_boolean)));
        assert!(!is_boolean_true(&t(
            "true",
            "http://www.w3.org/2001/XMLSchema#string"
        )));
        assert!(!is_boolean_true(&sparq_difftest::Term::Iri(
            "http://example.org/true".into()
        )));
    }
}

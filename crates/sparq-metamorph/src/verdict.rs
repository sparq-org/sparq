//! Oracle verdict types — **wrong-result strictly distinguished from engine-error**.
//! sq-gum8.6
//!
//! Every oracle in this crate returns a [`Verdict`] with exactly three top-level states:
//!
//! * [`Verdict::Pass`] — every query the oracle needed evaluated successfully **and** the
//!   metamorphic/differential relation held on this input. `Pass` is only ever constructed
//!   *after* an explicit comparison succeeds; there is no default-pass path.
//! * [`Verdict::Violation`] — every query evaluated successfully but the relation is
//!   **broken**: a wrong-result **logic-bug signal** (the class SQLancer calls a logic bug:
//!   silently incorrect results, the worst failure mode because nothing crashes).
//! * [`Verdict::EngineFailure`] — some query did **not** evaluate (rejected, transport
//!   error, unparseable results, harness precondition unmet). This is *fail-closed*: an
//!   engine error is never reported as `Pass` (it would mask a crash bug) and never as
//!   `Violation` (it is not evidence of a wrong result). Campaign triage decides whether a
//!   given failure is itself ledger-worthy (an internal error on a valid query is a bug of
//!   the [`crate::ledger::BugClass::EngineError`] class — distinct from a wrong result).

use serde::{Deserialize, Serialize};

/// Which oracle produced a verdict / found a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OracleKind {
    /// Ternary logic partitioning re-derived for SPARQL (see [`crate::tlp`]).
    Tlp,
    /// Non-optimizable rewrite cardinality oracle (see [`crate::norec`]).
    Norec,
    /// Cross-engine differential oracle (see [`crate::differential`]).
    Differential,
    /// Ternary logic partitioning under **set** semantics — the `DISTINCT` variant, whose
    /// partitions recombine by set union (see [`crate::distinct`]). sq-gum8.12
    TlpDistinct,
    /// Ternary logic partitioning recombined through an **aggregate** — `COUNT`/`SUM`
    /// over the three branches, with SPARQL's aggregate error semantics (see
    /// [`crate::aggregate`]). sq-gum8.12
    TlpAggregate,
    /// Cross-engine differential oracle in **`ORDER BY` mode** — compared up to
    /// permutation within each sort-key equivalence class (see
    /// [`crate::differential::check_differential_ordered`]). sq-gum8.12
    DifferentialOrdered,
}

impl std::fmt::Display for OracleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleKind::Tlp => write!(f, "tlp"),
            OracleKind::Norec => write!(f, "norec"),
            OracleKind::Differential => write!(f, "differential"),
            OracleKind::TlpDistinct => write!(f, "tlp-distinct"),
            OracleKind::TlpAggregate => write!(f, "tlp-aggregate"),
            OracleKind::DifferentialOrdered => write!(f, "differential-ordered"),
        }
    }
}

/// Why a query did not evaluate. The coarse kind matters for triage (a `Transport`
/// failure is infrastructure; an `HttpStatus(500)` on a valid query may be an
/// engine-error bug); the strict wrong-result/engine-error split is at the
/// [`Verdict`] level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    /// Loading the test data into the engine failed.
    Load,
    /// The engine rejected or failed to evaluate the query (in-process error string,
    /// parse refusal, evaluation error).
    Evaluation,
    /// A protocol endpoint answered with a non-success HTTP status.
    HttpStatus(u16),
    /// Network / IO failure reaching a protocol endpoint.
    Transport,
    /// The engine answered, but the response was unparseable as SPARQL results, or had
    /// the wrong shape (e.g. a boolean result for a `SELECT`). Classified as an engine
    /// failure, not a wrong result: a malformed response is a serialisation/driver
    /// problem to triage, not evidence about query semantics.
    InvalidResults,
    /// An oracle precondition was violated by the caller (e.g. a differential check over
    /// fewer than two engines). Reported as a failure — never silently passed.
    Harness,
}

/// A query that did not evaluate: which engine, which query, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineFailure {
    /// The engine's [`crate::engine::SparqlEngine::name`].
    pub engine: String,
    /// The exact query text that failed (empty when the failure precedes any query,
    /// e.g. [`FailureKind::Load`] / [`FailureKind::Harness`]).
    pub query: String,
    /// Coarse failure class.
    pub kind: FailureKind,
    /// Human-readable reason.
    pub message: String,
}

impl std::fmt::Display for EngineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "engine failure [{:?}] on {}: {}",
            self.kind, self.engine, self.message
        )
    }
}

/// A broken metamorphic/differential relation: the wrong-result logic-bug signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// Which oracle's relation broke.
    pub oracle: OracleKind,
    /// The engine(s) involved (one for TLP/NoREC; the compared pair for differential).
    pub engines: Vec<String>,
    /// The exact queries whose results disagree, for reproduction.
    pub queries: Vec<String>,
    /// Human-readable description of the disagreement (cardinalities / branch counts).
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} violation on [{}]: {}",
            self.oracle,
            self.engines.join(", "),
            self.detail
        )
    }
}

/// The three-state oracle outcome. See the module docs for the strict semantics of
/// each state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Relation held on this input (evidence string records what was compared).
    Pass {
        /// What was compared (branch cardinalities etc.), for campaign logs.
        detail: String,
    },
    /// Relation broken with all queries evaluated: a wrong-result signal.
    Violation(Violation),
    /// A query did not evaluate: fail-closed, neither pass nor wrong-result.
    EngineFailure(EngineFailure),
}

impl Verdict {
    /// True iff the relation held.
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass { .. })
    }

    /// True iff the relation broke (wrong-result signal).
    pub fn is_violation(&self) -> bool {
        matches!(self, Verdict::Violation(_))
    }

    /// True iff some query failed to evaluate.
    pub fn is_engine_failure(&self) -> bool {
        matches!(self, Verdict::EngineFailure(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_kind_displays_and_serialises() {
        // Display and the serde wire form must agree for every kind — the ledger reads
        // back what the campaign logs wrote.
        for (kind, wire) in [
            (OracleKind::Tlp, "tlp"),
            (OracleKind::Norec, "norec"),
            (OracleKind::Differential, "differential"),
            (OracleKind::TlpDistinct, "tlp-distinct"),
            (OracleKind::TlpAggregate, "tlp-aggregate"),
            (OracleKind::DifferentialOrdered, "differential-ordered"),
        ] {
            assert_eq!(kind.to_string(), wire);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            let back: OracleKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn verdict_state_predicates_are_mutually_exclusive() {
        let pass = Verdict::Pass {
            detail: "base=3 true=1 false=1 error=1".into(),
        };
        let violation = Verdict::Violation(Violation {
            oracle: OracleKind::Tlp,
            engines: vec!["sparq".into()],
            queries: vec!["SELECT * WHERE { ?s ?p ?o }".into()],
            detail: "base=3, partition union=2".into(),
        });
        let failure = Verdict::EngineFailure(EngineFailure {
            engine: "sparq".into(),
            query: "SELECT".into(),
            kind: FailureKind::Evaluation,
            message: "parse error".into(),
        });
        assert!(pass.is_pass() && !pass.is_violation() && !pass.is_engine_failure());
        assert!(violation.is_violation() && !violation.is_pass() && !violation.is_engine_failure());
        assert!(failure.is_engine_failure() && !failure.is_pass() && !failure.is_violation());
    }

    #[test]
    fn engine_failure_display_names_kind_and_engine() {
        let failure = EngineFailure {
            engine: "fuseki".into(),
            query: "SELECT * WHERE { ?s ?p ?o }".into(),
            kind: FailureKind::HttpStatus(500),
            message: "internal error".into(),
        };
        let text = failure.to_string();
        assert!(text.contains("fuseki") && text.contains("500") && text.contains("internal error"));
    }

    #[test]
    fn violation_display_names_oracle_and_engines() {
        let violation = Violation {
            oracle: OracleKind::Differential,
            engines: vec!["sparq".into(), "oxigraph".into()],
            queries: vec![],
            detail: "3 vs 2 rows".into(),
        };
        let text = violation.to_string();
        assert!(text.contains("differential") && text.contains("oxigraph") && text.contains("3 vs 2"));
    }
}

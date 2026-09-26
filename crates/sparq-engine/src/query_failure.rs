//! [GPT-6] Typed execution causes, emitted independently of diagnostic text.

use std::fmt;

/// A cooperative execution limit that was actually reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetExceeded {
    /// Materialized row limit.
    Rows,
    /// Estimated working-set byte limit.
    Bytes,
    /// Installed deadline elapsed.
    Deadline,
    /// Installed cancellation flag was observed.
    Cancelled,
}

impl BudgetExceeded {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Rows => "max-rows",
            Self::Bytes => "max-bytes",
            Self::Deadline => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A configured exact-evaluation domain could not represent a valid value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationCapacity {
    /// A numeric consumer or result exceeds the exact numeric representation.
    NumericRepresentation,
    /// A temporal year is outside the configured supported interval.
    TemporalYear,
}

impl fmt::Display for EvaluationCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NumericRepresentation => "numeric-representation",
            Self::TemporalYear => "temporal-year",
        })
    }
}

/// Whole-query failure from the typed prepared-query entry point.
///
/// Ordinary SPARQL expression errors that become unbound or filter out a row
/// remain successful evaluation. `Evaluation` is an actual whole-query error;
/// its diagnostic must never be interpreted as a budget or capacity category.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryFailure {
    /// A cooperative resource, deadline or cancellation limit was reached.
    Budget(BudgetExceeded),
    /// The explicitly configured exact evaluation domain was exceeded.
    Capacity(EvaluationCapacity),
    /// Another whole-query failure, retaining the legacy diagnostic.
    Evaluation(String),
}

impl fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(cause) => write!(f, "query budget exceeded ({cause})"),
            Self::Capacity(cause) => write!(f, "query evaluation capacity exceeded ({cause})"),
            Self::Evaluation(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for QueryFailure {}

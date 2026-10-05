//! [GPT-6] Typed execution causes without private diagnostic text.

use crate::Rejected;
use sparq_engine::{BudgetExceeded, EvaluationCapacity, QueryFailure};
use std::fmt;

/// Detailed rejection from native/guest evaluation, outside the journal format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationError {
    /// An existing request, source, admission or serialization rejection.
    Rejected(Rejected),
    /// A cooperative row/byte limit, deadline or cancellation was reached.
    Budget(BudgetExceeded),
    /// A configured exact numeric or temporal domain was exceeded.
    Capacity(EvaluationCapacity),
    /// Another whole-query execution failure; private engine text is discarded.
    Execution,
}

impl From<Rejected> for EvaluationError {
    fn from(value: Rejected) -> Self {
        Self::Rejected(value)
    }
}

impl From<QueryFailure> for EvaluationError {
    fn from(value: QueryFailure) -> Self {
        match value {
            QueryFailure::Budget(cause) => Self::Budget(cause),
            QueryFailure::Capacity(cause) => Self::Capacity(cause),
            QueryFailure::Evaluation(_) => Self::Execution,
        }
    }
}

impl From<EvaluationError> for Rejected {
    fn from(value: EvaluationError) -> Self {
        match value {
            EvaluationError::Rejected(rejected) => rejected,
            _ => Self("query evaluation or resource budget rejected"),
        }
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => error.fmt(f),
            Self::Budget(cause) => write!(f, "query budget exceeded ({cause})"),
            Self::Capacity(cause) => write!(f, "query evaluation capacity exceeded ({cause})"),
            Self::Execution => f.write_str("query execution rejected"),
        }
    }
}

impl std::error::Error for EvaluationError {}

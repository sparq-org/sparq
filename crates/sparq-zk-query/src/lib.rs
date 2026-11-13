//! Bounded query plans for the research query method. Not externally audited.

pub mod answer;
pub mod capacity;
pub mod codegen;
pub mod credential;
pub mod credential_graphs;
pub mod witness_closed;
pub mod closed_arith_standin;
#[cfg(not(target_arch = "wasm32"))]
pub mod driver;
pub mod envelope_policy;
pub mod interp;
pub mod interp_closed_group;
mod interp_closed_group_plan;
pub mod interp_closed_order;
pub mod lower;
pub mod plan;
pub mod plan_closed;
pub mod presentation;
pub mod request;
pub mod statement;
pub mod witness;

/// Lane B's proof lifecycle primitive; lane E supplies the protocol adapter.
pub mod method {
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::driver::Compiler;
    pub use crate::presentation::{Prepared, Presentation};
}

/// Stable admission and evaluation refusals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid SPARQL syntax.
    Parse(String),
    /// A named construct outside this method.
    Excluded(String),
    /// A named construct outside the implemented stage.
    Unsupported(String),
    /// A request or evaluation exceeds a compiled bound.
    Capacity(String),
    /// The supplied object does not follow the plan contract.
    Invalid(String),
    /// Native tooling or local artifact storage failed.
    Infrastructure(String),
    /// The shared request/challenge contract rejected the operation.
    Protocol(sparq_query_protocol::ProtocolError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(why) => write!(f, "SPARQL parse refused: {why}"),
            Self::Excluded(name) => write!(f, "{name} is excluded by the query method"),
            Self::Unsupported(name) => write!(f, "{name} is not yet supported by the query method"),
            Self::Capacity(name) => write!(f, "capacity refusal: {name}"),
            Self::Invalid(why) => write!(f, "invalid query-method input: {why}"),
            Self::Infrastructure(why) => write!(f, "query-method infrastructure: {why}"),
            Self::Protocol(why) => write!(f, "{why}"),
        }
    }
}
impl std::error::Error for Error {}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support;

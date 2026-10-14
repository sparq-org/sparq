// sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// quickstart fence is a compiled doctest (hidden `#`-scaffolding for the pushed triple).
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

#[cfg(feature = "window-aggregate")]
mod aggregate;
mod budget;
mod eval;
mod multi;
mod query;
mod rspql;
mod stream;
mod window;

#[cfg(feature = "window-aggregate")]
pub use aggregate::{window_aggregate, Agg};
pub use eval::EvalMode;
pub use multi::ContinuousMultiQuery;
pub use query::{
    AskResult, ContinuousAsk, ContinuousConstruct, ContinuousQuery, GraphResult, R2S, WindowResult,
};
pub use rspql::{RspqlQuery, WindowDecl};
// sq-xqu: re-exported so embedders can build per-registered-query
// window-evaluation budgets without a direct sparq-engine dependency.
pub use sparq_engine::QueryBudget;
pub use stream::{Timestamped, TimestampedTriple, TripleStream};
pub use window::{Window, WindowSpec, WindowedStream};

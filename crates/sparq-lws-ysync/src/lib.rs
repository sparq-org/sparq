//! Transport-independent Yjs relay. Adapters supply authorization and ordinary resource writes.
#![forbid(unsafe_code)]

mod log;
mod relay;
mod room;

pub use yrs;

pub use log::{load, LogFile, LogKind};
pub use relay::{Access, Agent, Authorizer, Changes, Limits, Relay, RoomStore, Session};
pub use room::Fault;

#[cfg(test)]
mod tests;

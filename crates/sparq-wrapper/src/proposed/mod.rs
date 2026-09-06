// [FABLE-5] sq-1rg2q: namespace for still-unlanded rdfjs/wrapper proposals.

//! Ports of still-unlanded rdfjs/wrapper proposals, each behind its own
//! default-off `proposed-*` cargo feature.
//!
//! Every submodule here only exists when its feature is enabled; with no
//! `proposed-*` feature this module is empty and contributes no code. The
//! typed focus kinds and bound node factories live in the feature-gated
//! `typed_focus` submodule (cargo feature `proposed-typed-focus`).

#[cfg(feature = "proposed-async-events")]
pub mod async_events;
#[cfg(feature = "proposed-async-node")]
pub mod async_node;
#[cfg(feature = "proposed-async-store")]
pub mod async_store;
#[cfg(feature = "proposed-cardinality")]
pub mod cardinality;
#[cfg(feature = "proposed-codecs")]
pub mod codecs;
#[cfg(feature = "proposed-graph-scope")]
pub mod graph_scope;
#[cfg(feature = "proposed-graph-scope-events")]
pub mod graph_scope_events;
#[cfg(feature = "proposed-json")]
pub mod json;
#[cfg(feature = "proposed-observe")]
pub mod observe;
#[cfg(feature = "proposed-typed-focus")]
pub mod typed_focus;

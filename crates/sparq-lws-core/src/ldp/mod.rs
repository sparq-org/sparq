// AUTHORED-BY Claude Opus 4.8
//! The LDP / Solid Protocol request path (M1 + the M2 verb-completion slice).
//!
//! - [`target`] — request-URL / LDP-target parsing (pure value logic).
//! - [`cors`] — the Solid Protocol CORS middleware (reflective origin + credentials, preflight,
//!   exposed headers, case-sensitive `Vary: Origin`) — a hand-rolled axum middleware.
//! - [`content`] — Turtle / JSON-LD classification, RDF validation, re-serialisation, and `Accept`
//!   content negotiation.
//! - [`conditional`] — `If-Match` / `If-None-Match` precondition evaluation over the strong ETag.
//! - [`range`] — single and multipart `Range: bytes=…` request handling (206 / 416).
//! - [`patch`] — the Solid N3-Patch engine (`text/n3`): insert/delete plus the `solid:where`
//!   variable solver (basic-graph-pattern matching with the spec's exactly-one-solution rule).
//! - [`handler`] — the GET / HEAD / PUT / POST / DELETE / PATCH axum handlers over the
//!   [`crate::store::Store`] seam.
//!
//! M2-next (clearly seamed, not implemented): full WAC authorization (needs the SPARQ access-control
//! design). `application/sparql-update` PATCH now supports the INSERT/DELETE DATA
//! subset (see [`patch`]).

pub mod conditional;
pub mod content;
pub mod cors;
pub mod handler;
pub mod patch;
pub mod public_read_skip;
pub mod range;
pub mod target;

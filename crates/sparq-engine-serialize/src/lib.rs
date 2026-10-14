//! (sq-6vshe.4) Internal RDF-serializer sub-crate of `sparq-engine`.
//!
//! **Unstable-internal.** This crate is an implementation detail of `sparq-engine`
//! (seam 1 of the staged facade split, RFC `research/engine-split-rfc.md` §4 Option A /
//! §7 Phase A1). It is `publish = false` and carries **no stability guarantee** of its
//! own: depend on the re-export `sparq_engine::serialize` instead, whose public surface
//! is unchanged by the split.
//!
//! It holds the RDF writer matrix — Turtle / TriG / N-Quads / JSON-LD (buffered and
//! streaming) — that used to live in `sparq-engine`'s `serialize` module. `sparq-engine`
//! re-exports it verbatim as `sparq_engine::serialize`, so every existing public path
//! (`sparq_engine::serialize::write_turtle`, `…::graph_to_jsonld`, `…::JsonLdForm`, …)
//! and every cargo feature name (`serialize-rdf`, `streaming-serialization`) is preserved.
//!
//! ## Feature gating (mirrors the original in-engine gating)
//!
//! The whole writer matrix lives behind the **`serialize-rdf`** feature, exactly as it did
//! inside `sparq-engine` (`#[cfg(feature = "serialize-rdf")] pub mod serialize;`). The
//! facade pulls this crate in as an **optional** dependency and enables this feature only
//! when its own `serialize-rdf` is on — so the default / wasm builds compile **zero** of it
//! and pull in **zero** new dependencies (byte-identical feature-off contract preserved).
//! `streaming-serialization` implies `serialize-rdf` and additionally enables the streaming
//! Turtle/TriG writers.

#![forbid(unsafe_code)] // sq-6vshe.4: the serializer carries zero `unsafe` (moved verbatim from sparq-engine, which forbids it crate-wide).

// The entire serializer matrix is behind `serialize-rdf`, mirroring the gating this code
// had inside `sparq-engine` (`#[cfg(feature = "serialize-rdf")] pub mod serialize;`). When
// the feature is off (the default), the crate compiles to an empty library and pulls in no
// `sparq-jsonld` dependency. `sparq-engine` re-exports THIS module verbatim as
// `sparq_engine::serialize` (a `pub use sparq_engine_serialize::serialize;`), so every
// existing public path is preserved byte-for-byte.
#[cfg(feature = "serialize-rdf")]
pub mod serialize;

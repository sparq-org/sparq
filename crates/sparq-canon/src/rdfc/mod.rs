//! RDFC-1.0 core, vendored from the zkp-ld `rdf-canon` 0.15.3 crate (MIT, (c) 2023 yamdan;
//! see `LICENSE` in this directory) with sparq's label-independence patches. It lives
//! inside sparq-canon, rather than as a `[patch]`ed dependency, so the patched algorithm
//! ships with every downstream build and with the published crate. Provenance, the
//! patches, and the retirement condition: `SPARQ-PATCHES.md` in this directory.

mod api;
mod canon;
mod counter;
mod error;

pub use api::{
    canonicalize_quads, canonicalize_quads_with, issue_quads, issue_quads_with,
    CanonicalizationOptions,
};
pub use error::CanonicalizationError;

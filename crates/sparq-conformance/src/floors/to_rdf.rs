//! sq-oy1f.40 — the W3C JSON-LD 1.1 `toRdf` lane ratchet floor
//! (relocated from `tests/jsonld_suite.rs` to this lib-side single source).

/// toRdf (JSON-LD → RDF via oxjsonld) pass floor. RATCHET: may only RISE.
/// Measured 413/467 at the pinned revision (the rest are honest divergences:
/// remote/`@import` `@context` URLs — oxjsonld needs a LoadDocumentCallback —
/// `expandContext`/`rdfDirection` options sparq does not apply, base
/// dot-segment normalization edge cases, and a handful of negative tests
/// oxjsonld accepts leniently; see the runner doc-comment + README).
pub const FLOOR: usize = 413;

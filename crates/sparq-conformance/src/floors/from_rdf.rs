//! sq-oy1f.40 — the W3C JSON-LD 1.1 `fromRdf` lane ratchet floor
//! (relocated from `tests/jsonld_suite.rs` to this lib-side single source).

/// fromRdf (RDF → JSON-LD via the NATIVE `sparq_jsonld::from_rdf`, JSON-LD API
/// §8.1) pass floor. RATCHET: may only RISE.
///
/// sq-oy1f.28 ORACLE-STRENGTHENING RE-PIN, side by side:
/// * OLD oracle (engine-writer self-reparse round-trip only): **51/53**
///   (0 skips; the 2 failures were cross-graph shared list cells, fromRdf/0020
///   and /0021, which the writer's `@list` collapsing renamed; the 2
///   NegativeEvaluationTests counted as vacuous round-trip passes).
/// * NEW oracle (normative document-level comparison on every positive case +
///   the round-trip leg where §8.1 is lossless + REAL negative error-code
///   assertions): **52 pass / 0 fail / 1 skip** — every runnable positive case
///   passes both legs, both negatives raise the exact `invalid JSON literal`
///   code, and fromRdf/0008 (`specVersion: json-ld-1.0`, pinning the 1.0
///   algorithm's partial list conversion) flips from a vacuous round-trip pass
///   to an honest 1.1-processor skip.
pub const FLOOR: usize = 52;

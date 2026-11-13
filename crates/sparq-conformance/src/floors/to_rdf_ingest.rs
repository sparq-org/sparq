//! Graph JSON-LD ingest over the toRdf manifest at json-ld-api 8654ac2.
//! This separate oxjsonld measurement never contributes EARL assertions.
//! All 467 entries execute: 429 pass, 38 fail, none skipped. Unsupported options
//! and remote loading are not emulated; generalized RDF has no oxrdf dataset form.
//! Negatives check rejection, not exact W3C codes (Graph returns string errors).
/// Measured Graph-acceptance and canonical-dataset pass count. May only rise.
pub const FLOOR: usize = 429;
/// Maximum failures over all manifest entries. May only fall.
pub const FAIL_CEILING: usize = 38;

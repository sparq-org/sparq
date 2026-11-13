//! W3C JSON-LD compaction ratchet at json-ld-api 8654ac2.
/// Native document comparison and exact negative error codes. RATCHET: may only rise.
/// Measured 244 pass / 1 fail / 1 skip. The remaining failure is the 1.0-only
/// retired `compaction to list of lists` error; the skip is the narrowly pinned
/// legacy case `#t0038`. `IRI confused with prefix` now raises the required error.
pub const FLOOR: usize = 244;

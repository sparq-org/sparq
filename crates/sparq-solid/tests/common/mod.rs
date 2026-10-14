//! sq-t58w.6 — the **single reusable source** for the WAC/ACP parity corpus.
//!
//! The scenario corpus (the `AclBuilder`/`AcrBuilder`-emitted `.acl`/`.acr` data + the
//! `Expect` decision tables, 13 WAC + 13 ACP) lived inline in `conformance_wac.rs` /
//! `conformance_acp.rs`. It is extracted here, unchanged, so it is reachable by a SECOND
//! integration-test target without copy-paste — the differential oracle (`sq-t58w.7`,
//! `tests/differential_oracle.rs`) consumes the IDENTICAL scenarios. "One corpus, two
//! consumers." Design: `research/solid-acp-differential-oracle-design.md` §3.2 / §5.
//!
//! This is a pure test-infra move: NO change to the scenarios, the emitted RDF, or the
//! expected decisions. The `WacScenario` / `AcpScenario` / `Expect` / `Decision` types are
//! already public crate API (`sparq_solid::wac_conformance` / `sparq_solid::conformance`),
//! so any test target can build and run a corpus from them; what was *not* shared — and is
//! shared here — is the scenario builders, the corpus assembly, and the floor constants.
//!
//! Each conformance test file does `mod common;` and calls `common::wac_corpus()` /
//! `common::acp_corpus()`. Cargo compiles this file once per consuming test binary; a
//! `#![allow(dead_code)]` keeps a consumer that uses only part of the surface warning-free.

#![allow(dead_code)]

pub mod acp;
pub mod wac;

// Re-exported flat so consumers call `common::wac_corpus()` / `common::acp_corpus()`. Each
// conformance test binary consumes only one corpus (the other is its sibling's), and the
// differential oracle will consume both — so the re-export a given binary does not call is
// expected-unused; allow it rather than splitting the shared module per consumer.
#[allow(unused_imports)]
pub use acp::acp_corpus;
#[allow(unused_imports)]
pub use wac::wac_corpus;

/// The agreed scenario floors (the guard the parity tests assert and the differential
/// oracle inherits): 13 WAC + 13 ACP minimal-per-construct scenarios.
///
/// sq-z1xv8 — the VALUES now live once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floors and the reported floors are
/// ONE pair of `const`s and cannot drift (replacing the old textual re-read of this
/// file). Raise them THERE; the corpus narrative stays here.
pub const WAC_SCENARIO_FLOOR: usize = sparq_conformance_floors::solid::WAC_SCENARIO_FLOOR;
pub const ACP_SCENARIO_FLOOR: usize = sparq_conformance_floors::solid::ACP_SCENARIO_FLOOR;

/// Test WebIDs / origins shared by both corpora. Kept here so the two corpus modules — and
/// any second consumer — name the same principals.
pub const ALICE: &str = "https://alice.example/card#me";
pub const BOB: &str = "https://bob.example/card#me";
pub const CAROL: &str = "https://carol.example/card#me";
pub const DAVE: &str = "https://dave.example/card#me";
pub const APP: &str = "https://app.example";
pub const EVIL: &str = "https://evil.example";

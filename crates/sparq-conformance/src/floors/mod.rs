//! sq-oy1f.40 — the LIB-SIDE single source of truth for the JSON-LD API
//! and Graph ingest ratchet FLOORS (`toRdf`, `fromRdf`, `expand`, `compact`,
//! `flatten`, `frame`, `html`, `remote-doc`, `toRdf-ingest`).
//!
//! ## Why the floors live in the LIBRARY, not the test binary
//!
//! Before this module each JSON-LD floor was a `pub const … FLOOR: usize = N;`
//! inside the `tests/jsonld_suite.rs` test binary, MIRRORED as a plain `usize` in
//! `scoreboard::SUITES`, and the two were kept in lock-step only TEXTUALLY by the
//! `tests/scoreboard_floors.rs` guard (it read the const out of the test source and
//! `assert_eq!`'d it against the registry copy). That textual sync is the #1463
//! floor-DRIFT class: three independent places (the runner's `assert!`, the
//! registry `ratchet_floor`, and the `ci.yml` grep gate) each spelled the same
//! number and could silently disagree until a guard caught it AFTER the fact.
//!
//! Moving the floors HERE — into the library the test binary AND the scoreboard
//! both depend on — makes the runner's `assert!` and the registry's `ratchet_floor`
//! read the SAME `const` at COMPILE TIME. They cannot drift because there is only
//! one number. The `ci.yml` `jsonld-conformance` job greps each floor from
//! `src/floors/<lane>.rs` at run time (a single-source read, not a hard-coded
//! duplicate), so the third place is bound to the same source too.
//!
//! Each JSON-LD lane owns one `pub const FLOOR: usize`, optional failure ceiling, and
//! honest oracle / SKIP-bucket / measured-count documentation that belongs on the
//! floor it pins. The runner (`tests/jsonld_suite`) imports these as
//! `sparq_conformance::floors::<lane>::FLOOR`; the scoreboard imports them into its
//! `Suite` rows; and the floor-sync guard no longer needs a textual row for these
//! lanes (the compile-time import cannot drift). See `scoreboard::SUITES` and
//! `tests/scoreboard_floors.rs`.
//!
//! RATCHET: every floor may only RISE — never lower one. Each is the ACTUAL
//! measured pass count at the pinned suite revision (see the fetch scripts), NOT an
//! aspirational target.

pub mod compact;
pub mod expand;
pub mod flatten;
pub mod frame;
pub mod from_rdf;
pub mod to_rdf;
pub mod to_rdf_ingest;

pub mod rdf12;

pub mod html;
pub mod remote_doc;

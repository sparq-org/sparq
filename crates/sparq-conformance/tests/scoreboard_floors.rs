//! [OPUS-4.8] sq-ncvq.16 — guard that the CENTRAL conformance scoreboard's
//! declared ratchet floors stay in lock-step with the floors the runners actually
//! enforce.
//!
//! The scoreboard registry ([`sparq_conformance::scoreboard::SUITES`]) is the
//! single index of every conformance ratchet, but it does NOT depend on
//! `sparq-shacl` / `sparq-geo` (those crates must not become deps of this
//! dev-only harness — see `scoreboard.rs`). So a floor enforced elsewhere has to
//! reach the registry somehow. This test reads the crate-local source files
//! TEXTUALLY (no cargo build, no cross-crate dep) and asserts the copied values
//! still equal the `const` floors the runners assert — so the central scoreboard
//! can never silently fall out of sync with what CI enforces. If a runner raises
//! its floor, this test fails until the registry is updated too.
//!
//! # [SONNET-4.6] sq-z1xv8 — the textual read is now CRATE-LOCAL ONLY
//!
//! This guard used to reach OUT of `sparq-conformance` and read sibling crates'
//! test sources (`sparq-shacl`, `sparq-geo`, `sparq-solid`, `sparq-policy`,
//! `sparq-text`, `sparq-rsp`) by joining `../..` and a table-supplied
//! workspace-relative path. Besides being fragile, that was a CI test-selection
//! soundness hole — `scripts/ci_audit_inputs.py` "residual 3": the read escapes
//! this crate's directory, and because the path is assembled at RUNTIME it is
//! statically unresolvable, so neither a reverse-dependency closure nor a
//! `ci/path-ownership.toml` `readers = [...]` entry could attribute it. A change
//! to `sparq-solid`'s floor could skip the very lane that read it, caught only by
//! the nightly FULL run (design `research/change-based-test-selection.md` §6.1).
//!
//! Those floors now live in the zero-dependency `sparq-conformance-floors` crate
//! and are imported by BOTH the runner and the registry row, so they read one
//! compile-time `const` and cannot drift at all — the same shape the six JSON-LD
//! lanes get from `sparq_conformance::floors`. What is left here is the textual
//! guard for floors whose runner lives in THIS crate, and `const_floor_in` is now
//! rooted at `CARGO_MANIFEST_DIR` so it CANNOT address another crate;
//! [`textual_guard_reads_only_crate_local_sources`] pins that invariant.
//!
//! Three buckets, and every crate-test suite is in exactly one:
//! * [`CRATE_LOCAL_FLOORS`] — runner in this crate; floor read textually.
//! * [`SHARED_CRATE_FLOORS`] — floor in `sparq-conformance-floors`, imported by
//!   both sides ([`shared_crate_floors_are_pinned`] catches a silent LOWERING).
//! * [`LIB_SOURCED_FLOORS`] — floor in `src/floors/<lane>.rs`, imported by both
//!   sides ([`lib_sourced_jsonld_floors_are_pinned`] does the same).

use sparq_conformance::scoreboard::{Runner, SUITES};
use std::path::PathBuf;

/// Read `const <NAME>: usize = <N>;` from a source file in THIS crate, hermetically.
///
/// `rel_from_crate` is relative to `CARGO_MANIFEST_DIR` (i.e. to
/// `crates/sparq-conformance/`) and must stay inside it — see the module docs and
/// [`textual_guard_reads_only_crate_local_sources`].
fn const_floor_in(rel_from_crate: &str, const_name: &str) -> usize {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel_from_crate);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for line in src.lines() {
        let line = line.trim();
        // [OPUS-4.8] sq-t58w.6 — tolerate an optional `pub ` visibility prefix: some
        // floor consts are declared `pub const` (so a sibling test module can read
        // them), others bare `const`. Match e.g. `const NT_SYNTAX_FLOOR: usize = 12;`
        // AND `pub const D_ENTAIL_FLOOR: usize = 5;` (ignoring any trailing comment).
        let line = line.strip_prefix("pub ").unwrap_or(line);
        if let Some(rest) = line.strip_prefix("const ") {
            if let Some(after_name) = rest.strip_prefix(const_name) {
                // Guard against a longer const that merely starts with `const_name`
                // (the next char must be `:` or whitespace, i.e. the type ascription).
                if !after_name.starts_with(':') && !after_name.starts_with(char::is_whitespace) {
                    continue;
                }
                if let Some(eq) = after_name.split('=').nth(1) {
                    let digits: String =
                        eq.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
                    if !digits.is_empty() {
                        return digits.parse().expect("floor parses");
                    }
                }
            }
        }
    }
    panic!("did not find `const {const_name}: usize = N;` in {}", path.display());
}

/// (suite label, source file, const name) for each ratchet whose runner lives in
/// THIS crate and whose floor the scoreboard mirrors. Keep this aligned with the
/// `CrateTest` rows in `SUITES`.
///
/// The source file is relative to `CARGO_MANIFEST_DIR` and MUST stay inside this
/// crate — [`textual_guard_reads_only_crate_local_sources`] enforces that, because a
/// read that escapes the crate dir is the CI test-selection hole sq-z1xv8 closed (see
/// the module docs).
const CRATE_LOCAL_FLOORS: &[(&str, &str, &str)] = &[
    // [SONNET-4.6] sq-z1xv8 — the ELEVEN foreign-runner ratchets (W3C SHACL core +
    // SHACL-SPARQL, both OGC GeoSPARQL lanes, Solid WAC/ACP decision parity + the two
    // differential oracles, the SolidLab ODRL suite, the sparq-text BM25 oracle, the
    // sparq-rsp expressivity oracle) are NO LONGER listed here. Their floor consts
    // moved to the zero-dependency `sparq-conformance-floors` crate and are IMPORTED
    // into BOTH the `Suite` rows in `scoreboard::SUITES`
    // (`ratchet_floor: sparq_conformance_floors::<module>::<FLOOR>`) AND the runner in
    // its own crate (which takes the floors crate as a `dev-dependency`). Both read the
    // SAME compile-time constant, so they CANNOT drift — and the guard no longer reads
    // another crate's test source, which is what closed the `ci_audit_inputs.py`
    // "residual 3". These eleven are the `SHARED_CRATE_FLOORS` set below; the
    // `all_crate_test_suites_are_guarded` test exempts exactly them.
    //
    // [FABLE-5] sq-oy1f.40 — the SIX W3C JSON-LD 1.1 ratchets (toRdf, fromRdf,
    // compact, frame, expand, flatten) are NO LONGER listed here. Their floor consts
    // moved LIB-SIDE to `src/floors/<lane>.rs` (`floors::<lane>::FLOOR`) and are
    // IMPORTED directly into the `Suite` rows in `scoreboard::SUITES`
    // (`ratchet_floor: crate::floors::<lane>::FLOOR`) AND into the runner
    // (`tests/jsonld_suite/common.rs` re-exports them). Both therefore read the SAME
    // compile-time constant — they CANNOT drift, so a textual floor-sync row is
    // unnecessary (and would re-introduce the very hard-coded duplicate this bead
    // removed). These six are the `LIB_SOURCED_FLOORS` set below; the
    // `all_crate_test_suites_are_guarded` test exempts exactly them, so no OTHER
    // crate-test suite can silently escape the textual guard. See
    // `crates/sparq-conformance/src/floors/mod.rs`.
    // [FABLE-5] sq-tonhr.2 (epic sq-tonhr) — the W3C rdf-n-triples / rdf-n-quads /
    // rdf-trig syntax-suite ratchets. The three floor consts live in this crate's
    // `tests/rdf_line_syntax_ratchet.rs` (default-on, self-skipping without the fetched
    // w3c/rdf-tests data); the guard reads them textually so the central scoreboard's
    // `ratchet_floor`s can never drift from what the runner asserts.
    (
        "W3C N-Triples syntax (rdf11 rdf-n-triples)",
        "tests/rdf_line_syntax_ratchet.rs",
        "NT_SYNTAX_FLOOR",
    ),
    (
        "W3C N-Quads syntax (rdf11 rdf-n-quads)",
        "tests/rdf_line_syntax_ratchet.rs",
        "NQ_SYNTAX_FLOOR",
    ),
    (
        "W3C TriG syntax + eval (rdf11 rdf-trig)",
        "tests/rdf_line_syntax_ratchet.rs",
        "TRIG_SYNTAX_FLOOR",
    ),
    // [OPUS-4.8] sq-e5atd (epic sq-pbz04) — the W3C SPARQL 1.1 D-entailment ratchet.
    // `pub const D_ENTAIL_FLOOR` lives in this crate's `tests/d_entail_suite.rs`
    // (behind the opt-in `d-entail` feature, inside the `gated` module — the guard
    // reads it TEXTUALLY, so the `#[cfg]`/module nesting do not affect the match);
    // the guard pins the central scoreboard's `ratchet_floor` to it so the two can
    // never silently drift.
    (
        "W3C SPARQL 1.1 D-entailment",
        "tests/d_entail_suite.rs",
        "D_ENTAIL_FLOOR",
    ),
    // [FABLE-5] sq-pbz04.5.5 (epic sq-pbz04.5) — the W3C RIF WG Core test-suite
    // CONFORMANCE ratchet. `pub const RIF_WG_CORE_FLOOR` lives in this crate's
    // `tests/rif_wg_core_suite.rs` (behind the opt-in `rif-wg-core` feature, inside the
    // `gated` module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do
    // not affect the match); the guard pins the central scoreboard's `ratchet_floor` to
    // it so the two can never silently drift. It is a STANDARDS-suite lane (family "W3C
    // RIF") over the real W3C RIF WG Core test cases — DISTINCT from the
    // sparq-EXTENSION `RIF_CORE_FLOOR` expressivity ratchet below.
    (
        "W3C RIF WG Core test suite",
        "tests/rif_wg_core_suite.rs",
        "RIF_WG_CORE_FLOOR",
    ),
    // [FABLE-5] sq-pbz04.6.4 (epic sq-pbz04.6) — the sparq D VALUE-SPACE MATRIX arm's
    // EXTENSION ratchet. `pub const D_VALUE_MATRIX_FLOOR` lives in this crate's
    // `tests/d_entail_suite.rs` (behind the opt-in `d-entail` feature, inside the `gated`
    // module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do not affect
    // the match); the guard pins the central scoreboard's `ratchet_floor` to it so the two
    // can never silently drift. It is a sparq EXTENSION-shaped ratchet (value-space
    // assertions), tallied separately from the W3C D-entailment pass count above.
    (
        "D value-space matrix (integer/decimal/boolean/binary/temporal)",
        "tests/d_entail_suite.rs",
        "D_VALUE_MATRIX_FLOOR",
    ),
    // [OPUS-4.8] sq-ddpgx (epic sq-my8wd) — the W3C SPARQL 1.1 sparql11/service
    // EVALUATION ratchet. `pub const SERVICE_EVAL_FLOOR` lives in this crate's
    // `tests/service_eval_suite.rs` (behind the opt-in `service` feature, inside the
    // `gated` module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting
    // do not affect the match); the guard pins the central scoreboard's
    // `ratchet_floor` to it so the two can never silently drift.
    (
        "W3C SPARQL 1.1 sparql11/service evaluation",
        "tests/service_eval_suite.rs",
        "SERVICE_EVAL_FLOOR",
    ),
    // [OPUS-4.8] sq-jaj38 (epic sq-my8wd) — the W3C SPARQL 1.1 PROTOCOL (HTTP layer) ratchet.
    // `pub const HTTP_PROTOCOL_FLOOR` lives in this crate's `tests/http_protocol_suite.rs`
    // (behind the opt-in `http-protocol` feature, inside the `gated` module — the guard reads
    // it TEXTUALLY, so the `#[cfg]`/module nesting do not affect the match); the guard pins the
    // central scoreboard's `ratchet_floor` to it so the two can never silently drift.
    (
        "W3C SPARQL 1.1 Protocol (HTTP)",
        "tests/http_protocol_suite.rs",
        "HTTP_PROTOCOL_FLOOR",
    ),
    // [OPUS-4.8] sq-1uuxz (epic sq-my8wd) — the SPARQL 1.1 SERVICE-DESCRIPTION +
    // GRAPH-STORE PROTOCOL ratchet. `pub const SD_GSP_FLOOR` lives in this crate's
    // `tests/sd_gsp_suite.rs` (behind the opt-in `federation-descriptors` feature, inside
    // the `gated` module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do
    // not affect the match); the guard pins the central scoreboard's `ratchet_floor` to it
    // so the two can never silently drift.
    (
        "SPARQL 1.1 Service Description + Graph Store Protocol",
        "tests/sd_gsp_suite.rs",
        "SD_GSP_FLOOR",
    ),
    // [OPUS-4.8] sq-rh4gu (epic sq-pbz04) — the RIF-Core EXPRESSIVITY ratchet. The
    // floor const (`pub const RIF_CORE_FLOOR`) lives in THIS crate's
    // `tests/rif_core_suite.rs` (behind the opt-in `rif-core` feature, inside the
    // `gated` module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting
    // do not affect the match); the guard pins the central scoreboard's
    // `ratchet_floor` to it so the two can never silently drift. It is a sparq
    // EXTENSION-shaped ratchet over the RIF-Core (monotone Horn) subset, NOT the
    // normative W3C SPARQL-RIF conformance suite.
    (
        "RIF-Core expressivity (monotone Horn subset)",
        "tests/rif_core_suite.rs",
        "RIF_CORE_FLOOR",
    ),
    // [OPUS-4.8] sq-qo1a9 (epic sq-pbz04, the LAST conformance bead) — the GRADUATED
    // OWL 2 QL (DL-Lite_R) CERTAIN-ANSWER oracle ratchet. The floor const
    // (`pub const QL_DLLITE_FLOOR`) lives in THIS crate's `tests/ql_dllite_suite.rs`
    // (behind the opt-in `ql-experimental` feature, inside the `gated` module — the
    // guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do not affect the
    // match); the guard pins the central scoreboard's `ratchet_floor` to it so the
    // two can never silently drift. It is a sparq EXTENSION-shaped ratchet over the
    // hand-derived DL-Lite_R certain-answer oracle, NOT a full-OWL-2-QL-conformance
    // claim.
    (
        "OWL 2 QL (DL-Lite_R) certain-answer oracle",
        "tests/ql_dllite_suite.rs",
        "QL_DLLITE_FLOOR",
    ),
    // [FABLE-5] sq-pbz04.3.4 (epic sq-pbz04.3) — the OWL 2 QL ENTAILMENT-REGIME
    // GRADUATED-SUBSET ratchet. The floor const (`pub const QL_ENTAILMENT_FLOOR`)
    // lives in THIS crate's `tests/ql_entailment_floor.rs` (behind the opt-in
    // `ql-experimental` feature, inside the `gated` module — the guard reads it
    // TEXTUALLY, so the `#[cfg]`/module nesting do not affect the match); the guard
    // pins the central scoreboard's `ratchet_floor` to it so the two can never
    // silently drift. The floor equals the length of the PINNED NAMED-CASE list
    // (asserted in-runner), a sparq EXTENSION-shaped ratchet over the six-condition
    // sound `pr:QL` subset, NOT an OWL 2 QL / entailment-regime conformance claim.
    (
        "OWL 2 QL entailment-regime graduated subset",
        "tests/ql_entailment_floor.rs",
        "QL_ENTAILMENT_FLOOR",
    ),
    // [SONNET-4.6] sq-pbz04.2.4 (epic sq-pbz04) — the OWL 2 EL classification ratchet.
    // The floor const (`pub const EL_SUITE_FLOOR`) lives in THIS crate's
    // `tests/el_suite.rs` (behind the opt-in `el-suite` feature, inside the `gated`
    // module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do not
    // affect the match); the guard pins the central scoreboard's `ratchet_floor` to it
    // so the two can never silently drift. It is a sparq EXTENSION-shaped ratchet over
    // the EL fragment sparq-reason-el's classifier implements, NOT a full-OWL-2-EL-
    // conformance claim.
    (
        "OWL 2 EL classification (sparq-reason-el)",
        "tests/el_suite.rs",
        "EL_SUITE_FLOOR",
    ),
    // [FABLE-5] sq-pbz04.4.5 (epic sq-pbz04.4) — the OWL 2 DIRECT-SEMANTICS arm's two
    // ratchets. The floor consts (`pub const DL_PROFILE_FLOOR` / `DL_DIRECT_FLOOR`)
    // live in THIS crate's `tests/dl_suite.rs` (behind the opt-in `dl-direct` feature,
    // inside the `gated` module — the guard reads them TEXTUALLY, so the
    // `#[cfg]`/module nesting do not affect the match); the guard pins the central
    // scoreboard's `ratchet_floor`s to them so they can never silently drift. Both are
    // sparq EXTENSION measurements over the SCOPED FRAGMENT the layered
    // `sparq-reason-dl` checker implements — NOT full OWL 2 DL — and are EXACT-pinned
    // in-runner (`==`, not `>=`: the tri-state invariant "an abstention is NEVER a
    // pass" needs the inflation direction caught too).
    (
        "OWL 2 DL profile identification (Direct arm)",
        "tests/dl_suite.rs",
        "DL_PROFILE_FLOOR",
    ),
    (
        "OWL 2 Direct-Semantics consistency + entailment (scoped fragment)",
        "tests/dl_suite.rs",
        "DL_DIRECT_FLOOR",
    ),
    // [FABLE-5] the UFO-SN3 finite-world expressibility ratchet. The floor const
    // (`pub const UFO_SN3_FLOOR`) lives top-level in THIS crate's
    // `tests/ufo_sn3_suite.rs` (UNGATED — the lane calls plain `reason_n3` and runs
    // in ordinary `cargo test --workspace`); the guard reads it TEXTUALLY so the
    // central scoreboard's `ratchet_floor` can never silently drift from what the
    // runner asserts. It is a sparq EXTENSION-shaped ratchet over the finite-world
    // UFO-SN3 reference profile, NOT a UFO/gUFO/OntoUML standards-conformance claim
    // (no normative UFO conformance suite exists).
    (
        "UFO-SN3 finite-world expressibility",
        "tests/ufo_sn3_suite.rs",
        "UFO_SN3_FLOOR",
    ),
    // [KERN] the RDF 1.2 quoted-triple opacity ratchet. The floor const
    // (`pub const QUOTED_OPACITY_FLOOR`) lives top-level in THIS crate's
    // `tests/quoted_triple_opacity.rs` (UNGATED — the lane calls plain
    // `sparq_reason::materialize` on committed fixtures and runs in ordinary
    // `cargo test --workspace`; its feature-gated EL arm does not count toward
    // the floor). A sparq EXTENSION-shaped ratchet pinning the normative
    // RDF 1.2 triple-term semantics (quoting never asserts; closure
    // non-interference), NOT a W3C-suite pass count.
    (
        "RDF 1.2 quoted-triple opacity (reasoning)",
        "tests/quoted_triple_opacity.rs",
        "QUOTED_OPACITY_FLOOR",
    ),
];

/// [SONNET-4.6] sq-z1xv8 — the suite labels whose ratchet floor is sourced at COMPILE
/// TIME from the SHARED `sparq-conformance-floors` crate, imported into BOTH
/// `scoreboard::SUITES` and the enforcing runner in its own crate. These eleven do NOT
/// need — and must NOT have — a textual floor-sync row in `CRATE_LOCAL_FLOORS`: the
/// registry and the runner read the SAME `const`, so they cannot drift, and reading the
/// runner's source would mean reaching back out of this crate (the CI test-selection
/// hole this bead closed). `all_crate_test_suites_are_guarded` exempts exactly this set.
const SHARED_CRATE_FLOORS: &[&str] = &[
    "W3C SHACL core",
    "W3C SHACL-SPARQL",
    "OGC GeoSPARQL topology compliance",
    "OGC GeoSPARQL query-rewrite extension",
    "Solid WAC decision parity",
    "Solid ACP decision parity",
    "Solid WAC differential oracle",
    "Solid ACP differential oracle",
    "SolidLab ODRL Test Suite",
    "text-search differential oracle",
    "RSP expressivity / SRBench correctness",
];

/// [SONNET-4.6] sq-z1xv8 — the pinned floor VALUES the eleven shared-crate registry
/// rows must carry, as `(label, expected_floor)`. Because the registry imports the
/// shared `const` directly, drift is impossible — but a silent LOWERING is not: editing
/// `sparq-conformance-floors` down would drop the registry value with it. A ratchet may
/// only RISE, so this assertion fires on any decrease; a deliberate raise updates the
/// value here in the SAME commit that edits the floors crate (mirroring how a
/// textual-guard floor is bumped, and exactly how `LIB_SOURCED_EXPECTED` pins the six
/// JSON-LD lanes).
const SHARED_CRATE_EXPECTED: &[(&str, usize)] = &[
    ("W3C SHACL core", 98),
    ("W3C SHACL-SPARQL", 5),
    ("OGC GeoSPARQL topology compliance", 197),
    ("OGC GeoSPARQL query-rewrite extension", 48),
    ("Solid WAC decision parity", 13),
    ("Solid ACP decision parity", 13),
    // A divergence count, not a rising ratchet: the only acceptable value is 0.
    ("Solid WAC differential oracle", 0),
    ("Solid ACP differential oracle", 0),
    ("SolidLab ODRL Test Suite", 67),
    ("text-search differential oracle", 18750),
    ("RSP expressivity / SRBench correctness", 317),
];

/// [FABLE-5] sq-oy1f.40 — the suite labels whose ratchet floor is sourced at
/// COMPILE TIME from the LIB-SIDE `sparq_conformance::floors::<lane>::FLOOR` const
/// (imported into BOTH `scoreboard::SUITES` and the `jsonld_suite` runner). These
/// six do NOT need a textual floor-sync row in `CRATE_LOCAL_FLOORS`: the registry
/// and the runner read the SAME `const`, so they cannot drift. The
/// `all_crate_test_suites_are_guarded` test exempts exactly this set (and asserts
/// nothing else escapes the textual guard).
const LIB_SOURCED_FLOORS: &[&str] = &[
    "W3C JSON-LD 1.1 toRdf",
    "W3C JSON-LD 1.1 fromRdf",
    "W3C JSON-LD 1.1 compact",
    "W3C JSON-LD 1.1 frame",
    "W3C JSON-LD 1.1 expand",
    "W3C JSON-LD 1.1 flatten",
];

/// [FABLE-5] sq-oy1f.40 — the pinned floor VALUES the six lib-sourced JSON-LD
/// registry rows must carry, as `(label, expected_floor)`. Because the registry
/// imports the lib-side `const` directly, this compile-time check catches a silent
/// LOWERING of any of the six floors (a ratchet may only RISE): if someone edits
/// `src/floors/<lane>::FLOOR` down, the registry `ratchet_floor` drops with it and
/// this assertion fires. RAISING a floor deliberately updates the value here in the
/// same commit (mirroring how a textual-guard floor is bumped). This is the
/// lib-sourced analogue of the `const_floor_in` textual re-read, giving the six
/// JSON-LD lanes the same "floor cannot silently move" protection without a
/// hard-coded duplicate of the number in two source files.
const LIB_SOURCED_EXPECTED: &[(&str, usize)] = &[
    ("W3C JSON-LD 1.1 toRdf", 413),
    // [FABLE-5] sq-oy1f.28 — raised 51 → 52: the lane flipped from the engine-writer
    // round-trip oracle to the native document-level `sparq_jsonld::from_rdf` (§8.1)
    // oracle (normative expected-doc comparison + scoped round-trip + REAL negative
    // error-code assertions). Bumped in the SAME commit as src/floors/from_rdf.rs::FLOOR
    // and the ci.yml job name (rise-only; side-by-side in src/floors/from_rdf.rs).
    ("W3C JSON-LD 1.1 fromRdf", 52),
    // [FABLE-5] sq-oy1f.27 — oracle-correction re-pin 186 → 228: the compact lane
    // moved from the RDF-writer self-reparse round-trip to the NATIVE document-level
    // Compaction Algorithm compared against the W3C EXPECTED document (see
    // src/floors/compact.rs for the side-by-side). Bumped in the SAME commit as the
    // lib const (rise-only).
    // [OPUS-5] sq-gzsky — raised 228 → 243: the lane now RUNS the 17
    // NegativeEvaluationTests against the manifest's `expectErrorCode` instead of
    // skipping them (a wrong code is a FAIL). Bumped in the SAME commit as the lib
    // const src/floors/compact.rs::FLOOR (rise-only).
    ("W3C JSON-LD 1.1 compact", 243),
    // [FABLE-5] sq-oy1f.29 — raised 61 → 92: the frame lane moved from the RDF-first
    // framer (`graph_to_jsonld_framed`) to the NATIVE Framing pipeline compared against
    // the W3C EXPECTED document under the stronger normative oracle (see
    // src/floors/frame.rs for the side-by-side). Bumped in the SAME commit as the lib
    // const src/floors/frame.rs::FLOOR and the ci.yml grep gate (rise-only).
    ("W3C JSON-LD 1.1 frame", 92),
    // [SONNET-4.6] sq-oy1f.45 — raised 259 → 276 (expand() correctness: FsLoader
    // wiring + @id-null retention + IRI-colon scheme check + @nest scoped ctx
    // propagation + @reverse @index + 1.0-mode round-trip guard). Bumped in the
    // SAME commit as src/floors/expand.rs::FLOOR (rise-only).
    // [OPUS-5] sq-gzsky — raised 276 → 381: the 109-case NegativeEvaluationTest SKIP
    // bucket (the WHOLE expand gap) is closed — the lane RUNS them against the
    // manifest's `expectErrorCode` — plus seven spec-faithful sparq-jsonld fixes
    // (@included arrayification, @type+@direction, datatype-IRI + blank-node-datatype
    // validation, the keyword round-trip check, and the two 1.0-mode restrictions on
    // @container arrays and relative @vocab). Bumped in the SAME commit as
    // src/floors/expand.rs::FLOOR (rise-only).
    ("W3C JSON-LD 1.1 expand", 381),
    // [FABLE-5] sq-oy1f.26 — oracle-change re-pin (RDF-writer 50 → native flatten() 53).
    // The native lane composes over expand() and inherits the sq-oy1f.37 expand raises,
    // so merging main flips its 7 inherited fails to passes and it now MEASURES 53 pass /
    // 0 fail on the merged tree — a net RISE above the old writer oracle's 50 (union of
    // the native oracle AND main's expand fixes — see src/floors/flatten.rs). Rise-only.
    ("W3C JSON-LD 1.1 flatten", 53),
];

/// [FABLE-5] sq-oy1f.40 — the registry's six lib-sourced JSON-LD floors carry the
/// pinned values (a ratchet may only RISE; a silent LOWERING of a
/// `src/floors/<lane>::FLOOR` const drops the registry value and trips this).
#[test]
fn lib_sourced_jsonld_floors_are_pinned() {
    for (label, expected) in LIB_SOURCED_EXPECTED {
        let suite = SUITES
            .iter()
            .find(|s| s.label == *label)
            .unwrap_or_else(|| panic!("scoreboard registry missing lib-sourced suite {label:?}"));
        assert_eq!(
            suite.ratchet_floor, *expected,
            "lib-sourced floor for {label} is {} but the pinned ratchet is {} — a floor may \
             only RISE; if this is a deliberate raise, bump LIB_SOURCED_EXPECTED in the same \
             commit that edits src/floors/<lane>::FLOOR (and the ci.yml grep gate)",
            suite.ratchet_floor, expected
        );
    }
}

/// [SONNET-4.6] sq-z1xv8 — the registry's eleven shared-crate floors carry the pinned
/// values (a ratchet may only RISE; a silent LOWERING of a
/// `sparq_conformance_floors::<module>::<FLOOR>` const drops the registry value and
/// trips this). Also asserts the registry row and the shared const are literally the
/// same number, which is what makes the retired textual guard unnecessary.
#[test]
fn shared_crate_floors_are_pinned() {
    use sparq_conformance_floors as floors;
    // The registry row's value MUST be the shared const, not a copy of it.
    let shared: &[(&str, usize)] = &[
        ("W3C SHACL core", floors::shacl::CORE_FLOOR),
        ("W3C SHACL-SPARQL", floors::shacl::SPARQL_FLOOR),
        ("OGC GeoSPARQL topology compliance", floors::geo::OGC_TOPOLOGY_FLOOR),
        ("OGC GeoSPARQL query-rewrite extension", floors::geo::OGC_QUERY_REWRITE_FLOOR),
        ("Solid WAC decision parity", floors::solid::WAC_SCENARIO_FLOOR),
        ("Solid ACP decision parity", floors::solid::ACP_SCENARIO_FLOOR),
        ("Solid WAC differential oracle", floors::solid::DIVERGENCE_FLOOR),
        ("Solid ACP differential oracle", floors::solid::DIVERGENCE_FLOOR),
        ("SolidLab ODRL Test Suite", floors::policy::ODRL_SUITE_FLOOR),
        ("text-search differential oracle", floors::text::BM25_ORACLE_FLOOR),
        ("RSP expressivity / SRBench correctness", floors::rsp::EXPRESSIVITY_FLOOR),
    ];
    // All three lists must cover the same suites, or a newly shared floor could be
    // listed as exempt from the textual guard while nothing actually pins its value.
    assert_eq!(
        shared.len(),
        SHARED_CRATE_FLOORS.len(),
        "SHARED_CRATE_FLOORS and this test's const table must cover the same suites"
    );
    assert_eq!(
        SHARED_CRATE_EXPECTED.len(),
        SHARED_CRATE_FLOORS.len(),
        "every SHARED_CRATE_FLOORS suite needs a SHARED_CRATE_EXPECTED pin, else a \
         silent floor LOWERING goes uncaught"
    );
    for (label, expected) in SHARED_CRATE_EXPECTED {
        let suite = SUITES
            .iter()
            .find(|s| s.label == *label)
            .unwrap_or_else(|| panic!("scoreboard registry missing shared-crate suite {label:?}"));
        let (_, shared_floor) = shared
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("shared-crate const table missing suite {label:?}"));
        assert_eq!(
            suite.ratchet_floor, *shared_floor,
            "scoreboard floor for {} does not read the shared sparq-conformance-floors \
             const — the registry row must IMPORT the const, never copy the number",
            label
        );
        assert_eq!(
            suite.ratchet_floor, *expected,
            "shared-crate floor for {} is {} but the pinned ratchet is {} — a floor may \
             only RISE; if this is a deliberate raise, bump SHARED_CRATE_EXPECTED in the \
             same commit that edits the sparq-conformance-floors const",
            label, suite.ratchet_floor, expected
        );
    }
}

/// [SONNET-4.6] sq-z1xv8 — the textual guard may only read sources inside THIS crate.
///
/// `const_floor_in` resolves against `CARGO_MANIFEST_DIR`, so a row naming `../<other
/// crate>` would re-open the CI test-selection hole this bead closed: an out-of-crate
/// read on a runtime-built path that neither a reverse-dependency closure nor a
/// `ci/path-ownership.toml` `readers` entry can attribute (`scripts/ci_audit_inputs.py`
/// residual 3). A floor enforced in another crate belongs in `sparq-conformance-floors`
/// (the `SHARED_CRATE_FLOORS` bucket), never in a textual row.
#[test]
fn textual_guard_reads_only_crate_local_sources() {
    for (label, src, _) in CRATE_LOCAL_FLOORS {
        assert!(
            src.starts_with("tests/") && !src.contains(".."),
            "floor-sync row {:?} names {:?}, which escapes crates/sparq-conformance — put \
             the floor in the shared sparq-conformance-floors crate and add the suite to \
             SHARED_CRATE_FLOORS instead of reading another crate's source",
            label, src
        );
        assert!(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(src).is_file(),
            "floor-sync row {:?} names missing source {:?}",
            label, src
        );
    }
}

#[test]
fn central_floors_match_crate_local_sources() {
    for (label, src, const_name) in CRATE_LOCAL_FLOORS {
        let suite = SUITES
            .iter()
            .find(|s| s.label == *label)
            .unwrap_or_else(|| panic!("scoreboard registry missing suite {label:?}"));
        // It must be a crate-test suite (not a binary one). [OPUS-4.8] sq-oy1f.2:
        // the feature-gated JSON-LD lane is a `FeatureGatedCrateTest` — still a
        // crate-local `cargo test` whose floor lives in source, so it is covered
        // by the same textual floor-sync guard.
        assert!(
            matches!(
                suite.runner,
                Runner::CrateTest { .. } | Runner::FeatureGatedCrateTest { .. }
            ),
            "{label} should be a (feature-gated) CrateTest suite in the registry"
        );
        let source_floor = const_floor_in(src, const_name);
        assert_eq!(
            suite.ratchet_floor, source_floor,
            "scoreboard floor for {label} ({}) is out of sync with {const_name} in {src} ({}) \
             — raise the registry value to match, or the central scoreboard under/over-reports \
             the enforced ratchet",
            suite.ratchet_floor, source_floor
        );
    }
}

/// Every crate-test suite in the registry is covered by a sync check above (so a
/// new crate-local ratchet added to `SUITES` cannot escape the guard).
///
/// [FABLE-5] sq-oy1f.40 / [SONNET-4.6] sq-z1xv8 — a crate-test suite is guarded by
/// EXACTLY ONE of three mechanisms:
///
/// 1. a TEXTUAL floor-sync row in `CRATE_LOCAL_FLOORS` — the runner lives in THIS
///    crate, so its `const` is read out of source and `assert_eq!`'d against the
///    registry copy;
/// 2. SHARED-CRATE — the floor lives in `sparq-conformance-floors` and is imported into
///    BOTH the registry row and the runner in its own crate (`SHARED_CRATE_FLOORS`,
///    covered by `shared_crate_floors_are_pinned`);
/// 3. LIB-SOURCED — the floor lives in `src/floors/<lane>::FLOOR` and is imported into
///    BOTH the registry row and the runner (`LIB_SOURCED_FLOORS`, covered by
///    `lib_sourced_jsonld_floors_are_pinned`).
///
/// Buckets 2 and 3 read ONE compile-time const on both sides, so they cannot drift;
/// bucket 1 is the textual fallback for same-crate runners. A new ratchet added to
/// `SUITES` must land in one of them, and in only one — nothing escapes the guard, and
/// nothing carries a redundant hard-coded duplicate.
#[test]
fn all_crate_test_suites_are_guarded() {
    for suite in SUITES {
        // [OPUS-4.8] sq-oy1f.2: cover both the plain and the feature-gated
        // crate-test runners — neither may escape the floor-sync guard.
        if matches!(
            suite.runner,
            Runner::CrateTest { .. } | Runner::FeatureGatedCrateTest { .. }
        ) {
            let textually_guarded =
                CRATE_LOCAL_FLOORS.iter().any(|(label, _, _)| *label == suite.label);
            let shared_crate = SHARED_CRATE_FLOORS.contains(&suite.label);
            let lib_sourced = LIB_SOURCED_FLOORS.contains(&suite.label);
            let buckets = usize::from(textually_guarded)
                + usize::from(shared_crate)
                + usize::from(lib_sourced);
            assert!(
                buckets > 0,
                "registry CrateTest suite {:?} has no floor guard: it is in none of \
                 CRATE_LOCAL_FLOORS, SHARED_CRATE_FLOORS or LIB_SOURCED_FLOORS",
                suite.label
            );
            // A suite must not be in more than one bucket — a textual row for a
            // const-sourced floor re-introduces the hard-coded duplicate sq-oy1f.40 and
            // sq-z1xv8 removed (and, for a foreign runner, the out-of-crate read too).
            assert!(
                buckets == 1,
                "registry CrateTest suite {:?} is in {} floor-guard buckets — keep exactly \
                 one (the imported const is the single source; drop the CRATE_LOCAL_FLOORS \
                 row)",
                suite.label, buckets
            );
        }
    }
}

/// The scoreboard renders and lists every registered suite — smoke check on the
/// binary's output shape.
#[test]
fn scoreboard_renders_all_suites() {
    let md = sparq_conformance::scoreboard::render_scoreboard();
    assert!(md.contains("conformance scoreboard"));
    for suite in SUITES {
        assert!(
            md.contains(suite.label),
            "rendered scoreboard omits suite {:?}",
            suite.label
        );
    }
    // The consolidation claim: SHACL + GeoSPARQL (sq-ncvq.16), Solid WAC/ACP
    // decision parity (sq-j174), and the Solid WAC/ACP differential oracles
    // (sq-t58w.8) now all appear in this central report.
    assert!(md.contains("W3C SHACL core"));
    assert!(md.contains("OGC GeoSPARQL topology compliance"));
    // [OPUS-4.8] sq-wf9qg — the OGC GeoSPARQL query-rewrite extension ratchet.
    assert!(md.contains("OGC GeoSPARQL query-rewrite extension"));
    assert!(md.contains("Solid WAC decision parity"));
    assert!(md.contains("Solid ACP decision parity"));
    assert!(md.contains("Solid WAC differential oracle"));
    assert!(md.contains("Solid ACP differential oracle"));
    // [OPUS-4.8] sq-oy1f.2 — the W3C JSON-LD 1.1 toRdf + fromRdf ratchets.
    assert!(md.contains("W3C JSON-LD 1.1 toRdf"));
    assert!(md.contains("W3C JSON-LD 1.1 fromRdf"));
    // [OPUS-4.8] sq-3uos5 — the W3C JSON-LD 1.1 compact ratchet.
    assert!(md.contains("W3C JSON-LD 1.1 compact"));
    // [OPUS-4.8] sq-oy1f — the W3C JSON-LD 1.1 expand + flatten ratchets.
    assert!(md.contains("W3C JSON-LD 1.1 expand"));
    assert!(md.contains("W3C JSON-LD 1.1 flatten"));
    // [OPUS-4.8] sq-tmsd6 — the SolidLab ODRL Test Suite decision-parity ratchet.
    assert!(md.contains("SolidLab ODRL Test Suite"));
    // [OPUS-4.8] sq-e5atd — the W3C SPARQL 1.1 D-entailment ratchet.
    assert!(md.contains("W3C SPARQL 1.1 D-entailment"));
    // [FABLE-5] sq-pbz04.5.5 — the W3C RIF WG Core test-suite conformance ratchet (a
    // STANDARDS lane, family "W3C RIF" — NOT the sparq-extension RIF-Core expressivity row).
    assert!(md.contains("W3C RIF WG Core test suite"));
    // [OPUS-4.8] sq-ddpgx — the W3C SPARQL 1.1 sparql11/service evaluation ratchet.
    assert!(md.contains("W3C SPARQL 1.1 sparql11/service evaluation"));
    // [OPUS-4.8] sq-jaj38 — the W3C SPARQL 1.1 Protocol (HTTP layer) ratchet.
    assert!(md.contains("W3C SPARQL 1.1 Protocol (HTTP)"));
    // [OPUS-4.8] sq-1uuxz — the SPARQL 1.1 Service Description + Graph Store Protocol ratchet.
    assert!(md.contains("SPARQL 1.1 Service Description + Graph Store Protocol"));
    // [OPUS-4.8] sq-ripcg — the sparq-text BM25 differential oracle, HONESTLY
    // rendered as a sparq EXTENSION (not a conformance claim): the row, its
    // `sparq extension` family, and the explicit "NOT conformance" total-line
    // disclaimer must all appear.
    assert!(md.contains("text-search differential oracle"));
    assert!(md.contains("sparq extension"));
    assert!(md.contains("NOT a standards-conformance claim") || md.contains("NOT conformance"));
    // [OPUS-4.8] sq-mcb3q — the sparq-rsp RSP expressivity / SRBench correctness
    // oracle, HONESTLY rendered as a sparq EXTENSION (not a conformance claim):
    // the row appears, and (now there are TWO extension rows) the total line
    // pluralises to "rows".
    assert!(md.contains("RSP expressivity / SRBench correctness"));
    // [OPUS-4.8] sq-rh4gu — the RIF-Core expressivity ratchet, HONESTLY rendered as
    // a sparq EXTENSION over the RIF-Core (monotone Horn) subset.
    assert!(md.contains("RIF-Core expressivity (monotone Horn subset)"));
    // [OPUS-4.8] sq-qo1a9 — the GRADUATED OWL 2 QL (DL-Lite_R) certain-answer oracle,
    // HONESTLY rendered as a sparq EXTENSION (NOT folded into the conformance total).
    assert!(md.contains("OWL 2 QL (DL-Lite_R) certain-answer oracle"));
    // [FABLE-5] sq-pbz04.3.4 — the OWL 2 QL entailment-regime graduated-subset
    // ratchet (the pinned named-case floor), HONESTLY rendered as a sparq EXTENSION
    // (NOT folded into the conformance total).
    assert!(md.contains("OWL 2 QL entailment-regime graduated subset"));
    // [SONNET-4.6] sq-pbz04.2.4 — the OWL 2 EL classification ratchet, HONESTLY rendered
    // as a sparq EXTENSION (the total line pluralises to "rows", and it is NOT folded
    // into the conformance total).
    assert!(md.contains("OWL 2 EL classification (sparq-reason-el)"));
    // [FABLE-5] sq-pbz04.4.5 — the two OWL 2 Direct-Semantics arm rows, HONESTLY
    // rendered as sparq EXTENSIONS over the scoped fragment (NOT full OWL 2 DL, NOT
    // folded into the conformance total).
    assert!(md.contains("OWL 2 DL profile identification (Direct arm)"));
    assert!(md.contains("OWL 2 Direct-Semantics consistency + entailment (scoped fragment)"));
    assert!(md.contains("NOT full OWL 2 DL"));
    // [FABLE-5] sq-pbz04.6.4 — the sparq D value-space matrix, HONESTLY rendered as a
    // sparq EXTENSION (tallied separately from the W3C D-entailment pass count).
    assert!(md.contains("D value-space matrix (integer/decimal/boolean/binary/temporal)"));
    // [FABLE-5] — the UFO-SN3 finite-world expressibility ratchet, HONESTLY rendered
    // as a sparq EXTENSION (no normative UFO/gUFO conformance suite exists; NOT
    // folded into the conformance total).
    assert!(md.contains("UFO-SN3 finite-world expressibility"));
    // [KERN] — the RDF 1.2 quoted-triple opacity ratchet, HONESTLY rendered as a
    // sparq EXTENSION (self-authored fixtures pinning the normative RDF 1.2
    // triple-term semantics; NOT folded into the conformance total).
    assert!(md.contains("RDF 1.2 quoted-triple opacity (reasoning)"));
    assert!(
        md.contains("sparq-extension (11 rows, NOT conformance)"),
        "eleven extension rows should be tallied separately and pluralised"
    );
}

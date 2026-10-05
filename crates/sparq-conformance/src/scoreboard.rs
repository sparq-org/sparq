//! [OPUS-4.8] sq-ncvq.16 — the CENTRAL conformance SCOREBOARD registry.
//!
//! Before this module the project's conformance ratchets were reported in
//! several disconnected places: the W3C SPARQL suites (this crate's
//! `sparq-conformance` binary), the inference suites (`sparq-inference-conformance`),
//! the W3C SHACL core + SHACL-SPARQL suites (crate-local `cargo test` runners in
//! `sparq-shacl`), the OGC GeoSPARQL topology ratchet (a crate-local `cargo test`
//! in `sparq-geo`), the Solid WAC + ACP library-level decision-parity suites
//! (crate-local `cargo test` runners in `sparq-solid`), and the SolidLab ODRL Test
//! Suite (a crate-local `cargo test` runner in `sparq-policy`, sq-tmsd6). The
//! drift-scanner
//! (`scripts/drift-scan.py` §5.E `conformance-split`) flagged the SHACL + geo
//! ratchets (sq-ncvq.16), then the Solid WAC/ACP ratchets (sq-j174), as living
//! OUTSIDE the central scoreboard, so no single artifact answered "what conformance
//! does sparq claim, and at what floor?".
//!
//! This registry is that single source of truth. It enumerates EVERY conformance
//! suite the project ratchets — across crates — with, for each: the spec family,
//! the runner that executes it (this crate's binaries, or the named crate-local
//! `cargo test`), the CI job that gates it, and its RATCHET FLOOR (a pass /
//! pass+divergence count that may only RISE). The `sparq-conformance-scoreboard`
//! binary renders it as ONE markdown table so CI (and humans) see all suites at
//! a glance.
//!
//! ## Why a registry, not a merged runner
//!
//! The SHACL + geo + Solid runners are NOT re-implemented here. They depend on
//! `sparq-shacl` / `sparq-geo` / `sparq-solid`, and this crate must stay free of
//! those deps — exactly the constraint `crates/sparq-shacl/tests/w3c_core.rs`
//! records in its own header ("Manifest-walking helpers are modelled on
//! `sparq-conformance`'s (copied, not shared — that crate is dev-only and must not
//! become a dependency)"). Pulling sparq-shacl/sparq-geo/sparq-solid into the
//! conformance crate would invert that and couple the SPARQL/inference scoreboard
//! to the SHACL/geo/Solid build.
//! So consolidation happens at the REPORTING layer: the floors live here as the
//! authoritative list, the runners stay where their dependencies are, and a guard
//! test (`tests/scoreboard_floors.rs`) hermetically reads the crate-local
//! sources to prove the centrally-declared floors still match the runner-enforced
//! ones (so the two can never silently diverge).

/// How a suite is executed — the command CI runs to enforce its ratchet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    /// This crate's `sparq-conformance` binary (W3C SPARQL query/update/syntax).
    SparqlBinary,
    /// This crate's `sparq-inference-conformance` binary (rdf-mt / OWL 2 RL / N3 /
    /// entailment / rdf-turtle).
    InferenceBinary,
    /// A crate-local `cargo test` runner: `(crate, test target)`. The runner
    /// asserts its own pinned floor; CI re-checks the printed scoreboard.
    CrateTest {
        /// Crate the test lives in, e.g. `sparq-shacl`.
        krate: &'static str,
        /// `--test <target>` name, e.g. `w3c_core`.
        target: &'static str,
    },
    /// [OPUS-4.8] sq-oy1f.2 — a crate-local `cargo test` runner that is OFF by
    /// default and only runs under an opt-in cargo `feature` (e.g. the JSON-LD
    /// lane behind `jsonld-suite`). Same as [`CrateTest`](Runner::CrateTest) but
    /// the rendered command names the required feature, so the scoreboard's
    /// "how to run" column is correct for a feature-gated ratchet (the lane
    /// compiles out / self-skips when the feature is off — the lean-core posture).
    FeatureGatedCrateTest {
        /// Crate the test lives in, e.g. `sparq-conformance`.
        krate: &'static str,
        /// `--test <target>` name, e.g. `jsonld_suite`.
        target: &'static str,
        /// The cargo `--features` flag the lane requires, e.g. `jsonld-suite`.
        feature: &'static str,
    },
}

impl Runner {
    /// The shell command a maintainer (or CI) runs to enforce this suite's
    /// ratchet, for the report's "how to run" column.
    pub fn command(&self) -> String {
        match self {
            Runner::SparqlBinary => {
                "cargo run --release -p sparq-conformance --bin sparq-conformance".into()
            }
            Runner::InferenceBinary => {
                "cargo run --release -p sparq-conformance --bin sparq-inference-conformance".into()
            }
            Runner::CrateTest { krate, target } => {
                format!("cargo test -p {krate} --test {target}")
            }
            Runner::FeatureGatedCrateTest { krate, target, feature } => {
                format!("cargo test -p {krate} --features {feature} --test {target}")
            }
        }
    }
}

/// One conformance suite the project ratchets.
#[derive(Clone, Copy, Debug)]
pub struct Suite {
    /// Human label, e.g. "W3C SHACL core".
    pub label: &'static str,
    /// Spec family / governing body, e.g. "W3C SHACL", "OGC GeoSPARQL".
    pub family: &'static str,
    /// How it is run.
    pub runner: Runner,
    /// The CI job (in `.github/workflows/ci.yml`) that gates it.
    pub ci_job: &'static str,
    /// The RATCHET FLOOR — the minimum pass (or pass+documented-divergence) count
    /// CI enforces. May only RISE. For the binary-runner suites this is the count
    /// parsed from the report's Overall line; for the crate-test suites it is the
    /// `const` floor the runner asserts (kept in sync by the guard test).
    pub ratchet_floor: usize,
    /// What "floor" counts (for the report footnote) — pass, or pass+divergence.
    pub floor_basis: &'static str,
    /// One-line scope note.
    pub note: &'static str,
}

/// The CENTRAL registry: every conformance suite, in report order. Adding a new
/// ratchet anywhere in the workspace means adding a row HERE (and the drift
/// scanner's `conformance-split` exemption references this list).
///
/// FLOORS — keep each in lock-step with its enforcer (the guard test checks the
/// crate-test ones against source):
/// * SPARQL  1229  — `ci.yml` job `conformance` (`RATCHET=1229`).
/// * inference 1967 — `ci.yml` job `inference-conformance` (`RATCHET=1967`).
///
/// [SONNET-4.6] sq-z1xv8 — every floor whose ENFORCING runner lives in another crate
/// (SHACL core/SPARQL, both OGC GeoSPARQL lanes, Solid WAC/ACP parity + the two
/// differential oracles, the SolidLab ODRL suite, the sparq-text BM25 oracle and the
/// sparq-rsp expressivity oracle) now lives in the zero-dependency
/// `sparq-conformance-floors` crate and is IMPORTED into both the runner's
/// `assert!(count >= FLOOR)` and the `Suite` row here
/// (`ratchet_floor: sparq_conformance_floors::<module>::<FLOOR>`) — the same
/// cannot-drift shape the six JSON-LD lanes get from `crate::floors`. This RETIRED
/// their textual floor-sync rows in `tests/scoreboard_floors.rs`, which used to read
/// those crates' test SOURCES at a runtime-built workspace path: an out-of-crate read
/// that no dependency closure or `ci/path-ownership.toml` `readers` entry could
/// attribute, i.e. CI test-selection "residual 3"
/// (`scripts/ci_audit_inputs.py`; design `research/change-based-test-selection.md`
/// §4.2). Raise such a floor in `sparq-conformance-floors`; the measurement narrative
/// stays with the runner, and `tests/scoreboard_floors.rs`'s `SHARED_CRATE_EXPECTED`
/// pins the value so a silent LOWERING still fails.
///
/// [FABLE-5] sq-oy1f.40 — the SIX JSON-LD floors below now live LIB-SIDE in
/// `src/floors/<lane>.rs` (`floors::<lane>::FLOOR`) and are IMPORTED directly into
/// the `Suite` rows here (`ratchet_floor: crate::floors::<lane>::FLOOR`), so the
/// registry and the runner's `assert!(pass >= FLOOR)` read ONE compile-time
/// constant — they cannot drift (retiring their textual floor-sync rows in
/// `tests/scoreboard_floors.rs`; `ci.yml` greps the same `src/floors/<lane>.rs`).
///
/// * JSON-LD toRdf 413 — `sparq-conformance` `src/floors/to_rdf.rs`
///   `FLOOR = 413` (sq-oy1f.2; opt-in `jsonld-suite` feature).
/// * JSON-LD fromRdf 52 — `sparq-conformance` `src/floors/from_rdf.rs`
///   `FLOOR = 52` (sq-oy1f.2; RAISED 51→52 by sq-oy1f.28 flipping the lane to the
///   native document-level `sparq_jsonld::from_rdf` oracle; opt-in `jsonld-suite`
///   feature).
/// * JSON-LD compact 243 — `sparq-conformance` `src/floors/compact.rs`
///   `FLOOR = 243` (sq-3uos5 163; RAISED →186 by sq-oy1f.16; RE-PINNED →228 by
///   sq-oy1f.27's oracle correction to the native document-level Compaction
///   Algorithm vs the W3C EXPECTED document; RAISED →243 by sq-gzsky RUNNING the
///   17 NegativeEvaluationTests against `expectErrorCode`; opt-in `jsonld-suite`
///   feature).
/// * JSON-LD frame 92 — `sparq-conformance` `src/floors/frame.rs`
///   `FLOOR = 92` (sq-oy1f.19; RE-PINNED 61→92 by sq-oy1f.29 flipping the lane from
///   the RDF-first framer to the NATIVE document-level Framing Algorithm compared to
///   the W3C EXPECTED document with `json_ld_equal` (negatives RUN, not skipped);
///   opt-in `jsonld-suite` feature; over the SEPARATE w3c/json-ld-framing suite).
/// * JSON-LD expand 381 — `sparq-conformance` `src/floors/expand.rs`
///   `FLOOR = 381` (sq-oy1f.37 expand() correctness raise from 240; →276 by
///   sq-oy1f.45; →381 by sq-gzsky RUNNING the 109 NegativeEvaluationTests against
///   `expectErrorCode` plus seven spec-faithful `sparq-jsonld` fixes; opt-in
///   `jsonld-suite` feature; the expand lane now calls `sparq_jsonld::expand()`
///   directly and compares the result to the expected document via `json_ld_equal`
///   — a document-level JSON comparator measuring JSON-LD data-model (semantic)
///   equivalence, NOT structural identity: object key order insignificant, array
///   order significant only inside `@list`, integers compared exactly (i64/u64),
///   non-integral numbers as f64.  ~18 of 240 passes are semantically-equal-but-
///   reordered vs. the W3C reference (strict-ordered count 222).  OLD floor was
///   247 under the RDF-equivalence oracle (sq-oy1f); the rebase reveals a net 7
///   fewer passes (20 flips minus 13 recoveries) and 26 new honest fails).
/// * JSON-LD flatten 46 — `sparq-conformance` `src/floors/flatten.rs`
///   `FLOOR = 46` (sq-oy1f.26; opt-in `jsonld-suite` feature; native
///   `sparq_jsonld::flatten()` Flattening Algorithm §7.1, compared to the
///   normative expected document via the `json_ld_equal` document-level
///   comparator; re-pin 50→46 from the retired RDF-writer oracle — the drop is
///   inherited native-expand gaps owned by sq-oy1f.37).
/// * Solid WAC differential 0 — `sparq-solid` `tests/differential_oracle.rs`
///   `DIVERGENCE_FLOOR = 0` (sq-t58w.8; a divergence-count floor, hard 0 — the WAC
///   and ACP differential rows share this one const).
/// * Solid ACP differential 0 — same `DIVERGENCE_FLOOR = 0` (sq-t58w.8).
/// * SolidLab ODRL 67 — `sparq-policy` `tests/odrl_test_suite.rs`
///   `ODRL_SUITE_FLOOR = 67` (sq-tmsd6 wired it at 59; the constraint-matching batch
///   sq-euhr3/sq-k7itg/sq-a0zef raised it to 67 of 68 cases pass, 1 in a documented
///   not-implemented bucket).
/// * D-entailment 1 — `sparq-conformance` `tests/d_entail_suite.rs`
///   `D_ENTAIL_FLOOR = 1` (sq-e5atd; opt-in `d-entail` feature; the D-only
///   `sparql11/entailment` tests graduated from OutOfScope to Pass through
///   sparq-reason's `Profile::D` — rdfD1 typing + typed value-space equality).
/// * sparql11/service evaluation 6 — `sparq-conformance`
///   `tests/service_eval_suite.rs` `SERVICE_EVAL_FLOOR = 6` (sq-ddpgx + sq-my8wd.1;
///   opt-in `service` feature; the deferred SERVICE/federation tests graduated from the
///   SPARQL binary's skip bucket to Pass by serving each `qt:serviceData` block on
///   a REAL in-process loopback endpoint — the sq-ushvx harness — and driving the
///   federated query end-to-end through the engine's REAL ureq transport; [SONNET-4.6]
///   nested non-SILENT SERVICE now handled by configuring outer-endpoint egress
///   allowlists; a variable SERVICE endpoint is the one remaining documented Skip).
/// * SPARQL 1.1 Protocol (HTTP) 21 — `sparq-conformance`
///   `tests/http_protocol_suite.rs` `HTTP_PROTOCOL_FLOOR = 21` (sq-jaj38; opt-in
///   `http-protocol` feature; RAW HTTP requests at the in-process loopback server
///   exercising the SPARQL 1.1 Protocol contract — GET/POST query+update, the QUERY
///   method, dataset overrides, SRJ/SRX/CSV/TSV negotiation, 200/400/405/406/415; the
///   present-but-unsatisfiable Accept now 406 (Oxigraph parity, sq-406acc); the
///   absent/`*/*` Accept JSON default + ASK-in-CSV are documented divergences, NOT summed in).
/// * SPARQL 1.1 Service Description + Graph Store Protocol 39 — `sparq-conformance`
///   `tests/sd_gsp_suite.rs` `SD_GSP_FLOOR = 39` (sq-1uuxz; opt-in
///   `federation-descriptors` feature; the `GET /sparql` (no query) Service-Description
///   advertises exactly the formats/languages/versions/features the server genuinely
///   implements — no over-advertising — PLUS a GET/PUT/POST/DELETE Graph-Store-Protocol
///   round-trip over RAW HTTP verifying store state after each op; the absent-graph
///   200-empty read is a documented divergence, NOT summed in).
/// * text-search differential oracle 18750 — `sparq-text`
///   `tests/bm25_oracle.rs` `TEXT_ORACLE_FLOOR = 18750` (sq-ripcg; a sparq
///   EXTENSION ratchet, NOT standards conformance — no normative full-text-over-RDF
///   / BM25 suite exists; floor = the MEASURED count of bit-exact BM25 score
///   assertions the fixed-seed corpus battery makes against a from-scratch
///   independent reference scorer; default-on, no opt-in feature required).
/// * RSP expressivity / SRBench correctness 317 — `sparq-rsp`
///   `tests/srbench_oracle.rs` `RSP_EXPRESSIVITY_FLOOR = 317` (sq-2n1q3.3; raised
///   from 303 sq-2n1q3.1; from 149 sq-mcb3q baseline; a sparq EXTENSION ratchet,
///   NOT standards conformance — RSP-QL is a W3C-COMMUNITY spec and SRBench is a
///   benchmark, so there is NO normative RDF-Stream-Processing Recommendation or its
///   conformance suite; floor = the MEASURED count of deterministic per-window
///   correctness assertions the fixed SRBench-shaped battery makes across window
///   types / R2S operators / EvalModes / multi-window joins (incl. 3+-window joins
///   and ISTREAM/DSTREAM over multi-window joins) against an INDEPENDENT
///   batch-rebuild + closed-form oracle; default-on, no opt-in feature required).
///   [SONNET-4.6]
/// * OWL 2 QL (DL-Lite_R) certain-answer oracle 11 — `sparq-conformance`
///   `tests/ql_dllite_suite.rs` `QL_DLLITE_FLOOR = 11` (sq-qo1a9; opt-in
///   `ql-experimental` feature; a sparq EXTENSION ratchet, NOT a
///   full-OWL-2-QL-conformance claim — no runnable normative W3C QL certain-answer
///   suite exists; floor = the MEASURED count of formal DL-Lite_R cases on which
///   `sparq_reason_ql::rewrite_production` is sound AND complete, i.e. the rewritten
///   UCQ evaluated over the unmodified ABox returns EXACTLY the hand-derived certain
///   answers; the broader `pr:QL` entailment-arm intensional gap stays
///   experimental/OutOfScope, never summed in).
/// * OWL 2 QL entailment-regime graduated subset 15 — `sparq-conformance`
///   `tests/ql_entailment_floor.rs` `QL_ENTAILMENT_FLOOR = 15` (sq-pbz04.3.4; opt-in
///   `ql-experimental` feature; a sparq EXTENSION ratchet, NOT an OWL 2 QL /
///   entailment-regime conformance claim — the floor is the PINNED NAMED-CASE list
///   of `pr:QL` `sparql11/entailment` cases passing ALL SIX graduation conditions,
///   exact set equality: regressions AND unpinned additions both fail CI; every
///   non-graduated case carries an exhaustive hold-reason taxonomy; raised 9→11 by
///   sq-pbz04.3.1 B2 literal-object broadening: `lang` + `plainLit` both graduate
///   [SONNET-4.6]; raised 11→15 by sq-pbz04.3.6 body-blank-node lifting +
///   shared-existential join preservation in the emitter: `sparqldl-05`/`-06`
///   (undistinguished-variable ASK, declaration-only TBox so `exists_super` empty)
///   and `sparqldl-07`/`-08` (SHARED-blank-node JOIN SELECT — graduate once the
///   emitter maps a repeated `Unbound` id to ONE variable, identity rewrite
///   result-equivalent to the W3C oracle) all graduate [OPUS-4.8]).
/// * OWL 2 EL classification 67 — `sparq-conformance` `tests/el_suite.rs`
///   `EL_SUITE_FLOOR = 67` (sq-pbz04.2.4 base; sq-pbz04.2.9 raised 50→51; sq-pbz04.2.10
///   raised 51→67: ABox graduation — `el-suite` now also pulls `sparq-reason-el/abox` so
///   the CI lane exercises the full shipped feature set including the two-step
///   `classify_graph` + `realize_graph` composition; 8 inconsistency + 8 positive-entailment
///   tests graduate (disjoint-class ABox clashes, NPA, hasKey, hasSelf, bottomObjectProperty,
///   equivalentProperty via augment_equivalent_properties); a sparq EXTENSION ratchet, NOT
///   a full-OWL-2-EL-conformance claim; floor = the MEASURED count of W3C OWL 2 EL
///   (test:EL ∧ test:RDF-BASED, Approved) check rows on which `sparq_reason_el`'s
///   classifier + ABox realiser compute the expected outcome; 11 audited PERMANENT
///   divergences (property-chain ABox expansion / reflexive properties / annotation
///   propagation / owl:equivalentProperty extraction / owl:unionOf / bottomDataProperty /
///   FunctionalProperty enforcement) are reported separately, never summed in). [SONNET-4.6]
pub const SUITES: &[Suite] = &[
    Suite {
        label: "W3C SPARQL (1.0 / 1.1 / 1.2, query+update+syntax)",
        family: "W3C SPARQL",
        runner: Runner::SparqlBinary,
        ci_job: "conformance",
        ratchet_floor: 1229,
        floor_basis: "pass + documented divergence",
        note: "query/update evaluation + the four mf:*SyntaxTest* kinds",
    },
    Suite {
        label: "Inference (rdf-mt / OWL 2 RL / N3 / entailment / rdf-turtle)",
        family: "W3C RDF Semantics + OWL + N3",
        runner: Runner::InferenceBinary,
        ci_job: "inference-conformance",
        ratchet_floor: 1967,
        floor_basis: "pass + documented divergence",
        note: "reasoning suites run against sparq-reason, plus the Turtle oracle",
    },
    Suite {
        label: "W3C SHACL core",
        family: "W3C SHACL",
        runner: Runner::CrateTest { krate: "sparq-shacl", target: "w3c_core" },
        ci_job: "shacl-conformance",
        ratchet_floor: sparq_conformance_floors::shacl::CORE_FLOOR,
        floor_basis: "pass",
        note: "data-shapes sht:Validate core suite (w3c/data-shapes)",
    },
    Suite {
        label: "W3C SHACL-SPARQL",
        family: "W3C SHACL",
        runner: Runner::CrateTest { krate: "sparq-shacl", target: "w3c_sparql" },
        ci_job: "shacl-conformance",
        ratchet_floor: sparq_conformance_floors::shacl::SPARQL_FLOOR,
        floor_basis: "pass",
        note: "sh:sparql node + property constraint sub-suites",
    },
    Suite {
        label: "OGC GeoSPARQL topology compliance",
        family: "OGC GeoSPARQL",
        runner: Runner::CrateTest { krate: "sparq-geo", target: "ogc_compliance_ratchet" },
        ci_job: "geo-conformance",
        // [OPUS-4.8] sq-cbe4t — raised 119 -> 158: 39 net-new hand-derived DE-9IM
        // assertions (reverse-order/symmetry coverage + MULTI* operands). The
        // crate-local `OGC_RATCHET_FLOOR` const moved in lock-step; the floor-sync
        // guard (`tests/scoreboard_floors.rs`) pins the two together.
        // [SONNET-4.6] sq-lk3aw.1 — raised 158 -> 197: 39 more net-new assertions
        // (edge-adjacent polygons / disjoint line+polygon / parallel lines /
        // point-on-line / multi-pair rcc8/eh disjoint cells).
        ratchet_floor: sparq_conformance_floors::geo::OGC_TOPOLOGY_FLOOR,
        floor_basis: "pass",
        // [OPUS-4.8] sq-cbe4t — DISTANCE-APPROXIMATION HONESTY NOTE. This topology
        // ratchet is exact DE-9IM (no approximation). The `geof:distance` METRIC
        // path (a separate, non-topological surface, scored under the R1-R30
        // requirements probe) is exact HAVERSINE only for point↔point /
        // point↔geometry; between two EXTENDED geometries it uses a LOCAL
        // EQUIRECTANGULAR approximation about mean latitude (accurate locally,
        // degrading at continental scale / near the poles). See the sparq-geo
        // README "Distance accuracy caveat" — no exactness is claimed there.
        note: "hand-curated sf/eh/rcc8 topology + WKT/GML equivalence assertions \
               (exact DE-9IM); geof:distance is exact haversine point↔point, local \
               equirectangular approximation extended↔extended (see sparq-geo README)",
    },
    // [OPUS-4.8] sq-wf9qg — the OGC GeoSPARQL QUERY-REWRITE extension
    // (`/conf/query-rewrite-extension`) ratchet, graduated alongside the topology
    // FILTER-function ratchet above. The runner is crate-local in sparq-geo
    // (`tests/ogc_query_rewrite_ratchet.rs`) but behind the OPT-IN
    // `geosparql_rewrite` feature (OFF by default) so the default + `--workspace`
    // builds never compile the rewrite surface and the STANDARD SPARQL behaviour
    // stays untouched — the lean opt-in posture. Each case drives a topology
    // PROPERTY pattern (`?f geo:sfWithin ?g`) end-to-end through the real rewrite
    // + engine, asserting result-equivalence to the lexical `geof:` oracle AND
    // that the standard entry point binds zero rows (no asserted topology triple).
    // The floor is the MEASURED pass count; kept in lock-step by
    // `tests/scoreboard_floors.rs`.
    Suite {
        label: "OGC GeoSPARQL query-rewrite extension",
        family: "OGC GeoSPARQL",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-geo",
            target: "ogc_query_rewrite_ratchet",
            feature: "geosparql_rewrite",
        },
        ci_job: "geo-conformance",
        ratchet_floor: sparq_conformance_floors::geo::OGC_QUERY_REWRITE_FLOOR,
        floor_basis: "pass",
        note: "topology PROPERTY forms (geo:sf*/eh*/rcc8*) answered end-to-end via the \
               opt-in query-rewrite extension, result-equivalent to the geof: oracle",
    },
    // [OPUS-4.8] sq-j174 — the Solid WAC + ACP library-level decision-parity suites
    // (harness landed under sq-3jtd.8). The runners stay crate-local in sparq-solid
    // (the dev-only conformance crate must not take sparq-solid as a dep — same
    // constraint as SHACL/geo); the central scoreboard REPORTS them with their
    // ratchet floors, kept in lock-step by `tests/scoreboard_floors.rs`. The floor
    // is the scenario COUNT each corpus must cover (a corpus that silently shrinks is
    // a coverage regression), exactly the in-source ratchet shape geo uses.
    Suite {
        label: "Solid WAC decision parity",
        family: "Solid WAC",
        runner: Runner::CrateTest { krate: "sparq-solid", target: "conformance_wac" },
        ci_job: "solid-conformance",
        ratchet_floor: sparq_conformance_floors::solid::WAC_SCENARIO_FLOOR,
        floor_basis: "scenario",
        note: "library-level allow/deny parity over minimal per-construct WAC .acl scenarios",
    },
    Suite {
        label: "Solid ACP decision parity",
        family: "Solid ACP",
        runner: Runner::CrateTest { krate: "sparq-solid", target: "conformance_acp" },
        ci_job: "solid-conformance",
        ratchet_floor: sparq_conformance_floors::solid::ACP_SCENARIO_FLOOR,
        floor_basis: "scenario",
        note: "library-level allow/deny parity over minimal per-construct ACP ACR scenarios",
    },
    // [OPUS-4.8] sq-t58w.8 — the Solid WAC + ACP DIFFERENTIAL ORACLE (harness landed
    // under sq-t58w.7, `crates/sparq-solid/tests/differential_oracle.rs`). It runs the
    // SAME shared parity corpus through THREE independent deciders (the engine, a
    // from-scratch procedural reference evaluator, and the hand `Expect` table) and
    // asserts they never disagree. The ratchet is a DIVERGENCE count whose ONLY
    // acceptable value is 0 — so unlike the scenario-COUNT floors above (which rise as
    // the corpus grows), this floor is the hard `DIVERGENCE_FLOOR = 0` the runner
    // asserts. Both rows share that ONE source const; the floor-sync guard
    // (`tests/scoreboard_floors.rs`) keeps them locked to it. The runner stays
    // crate-local in sparq-solid (this dev-only crate must not take sparq-solid as a
    // dep — same constraint as SHACL/geo/the conformance suites above). `ci_job` is the
    // Solid conformance lane; the dedicated `<WAC|ACP> differential … divergences N`
    // grep-ratchet is sq-t58w.3.
    Suite {
        label: "Solid WAC differential oracle",
        family: "Solid WAC",
        runner: Runner::CrateTest { krate: "sparq-solid", target: "differential_oracle" },
        ci_job: "solid-conformance",
        ratchet_floor: sparq_conformance_floors::solid::DIVERGENCE_FLOOR,
        floor_basis: "0 divergences",
        note: "engine vs an independent reference evaluator vs the hand Expect table, \
               over the WAC parity corpus (zero divergence)",
    },
    Suite {
        label: "Solid ACP differential oracle",
        family: "Solid ACP",
        runner: Runner::CrateTest { krate: "sparq-solid", target: "differential_oracle" },
        ci_job: "solid-conformance",
        ratchet_floor: sparq_conformance_floors::solid::DIVERGENCE_FLOOR,
        floor_basis: "0 divergences",
        note: "engine vs an independent reference evaluator vs the hand Expect table, \
               over the ACP parity corpus (zero divergence)",
    },
    // [OPUS-4.8] sq-oy1f.2 / sq-3uos5 / sq-oy1f.19 / sq-oy1f — the W3C JSON-LD 1.1
    // conformance ratchets. The runner is crate-local here (`tests/jsonld_suite.rs`)
    // but behind the OPT-IN `jsonld-suite` feature (forwards to sparq-core/jsonld +
    // sparq-engine/serialize-rdf) so the default + `--workspace` builds neither link
    // oxjsonld nor go red — the lean-core posture. Six gated categories: toRdf
    // (JSON-LD → RDF through the real oxjsonld parse path), fromRdf (RDF → JSON-LD
    // through the native serialize-rdf writer, re-parse round-trip), compact (RDF →
    // compacted JSON-LD via the native Compaction Algorithm, lossless round-trip),
    // frame (RDF → framed JSON-LD via the native Framing Algorithm over the SEPARATE
    // w3c/json-ld-framing suite, RDF-equivalence to the normative expected output),
    // and expand + flatten (RDF → expanded/flattened JSON-LD via the shipping
    // `graph_to_jsonld` writer, RDF-equivalence to the normative expected document).
    // The floors are the MEASURED pass counts at the pinned suite revisions (NOT
    // 100% — remote-context/option/writer-shape divergences are honest, recorded
    // gaps); they may only RISE. html + remote-doc remain the documented
    // NOT-IMPLEMENTED buckets the runner reports separately (never failed). Floors
    // kept in lock-step
    // by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C JSON-LD 1.1 toRdf",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [FABLE-5] sq-oy1f.40 — sourced from the LIB-SIDE floor const so the
        // registry and the runner's `assert!` read ONE number (no textual drift).
        ratchet_floor: crate::floors::to_rdf::FLOOR,
        floor_basis: "pass",
        note: "JSON-LD → RDF through the real oxjsonld parse path (jsonld feature); \
               compact + frame + expand + flatten are all now gated lanes (html + \
               remote-doc remain the not-implemented buckets)",
    },
    Suite {
        label: "W3C JSON-LD 1.1 fromRdf",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [FABLE-5] sq-oy1f.40 — LIB-SIDE floor const single source.
        ratchet_floor: crate::floors::from_rdf::FLOOR,
        floor_basis: "pass",
        // [FABLE-5] sq-oy1f.28 — lane flipped to the native document-level pipeline.
        note: "RDF → JSON-LD through the native sparq_jsonld::from_rdf (JSON-LD API \
               §8.1), compared document-level against the normative expected docs \
               plus a scoped re-parse round-trip; negatives assert exact error codes",
    },
    // [FABLE-5] sq-oy1f.27 — the W3C JSON-LD 1.1 `compact` ratchet (epic sq-oy1f), on
    // the NATIVE DOCUMENT-LEVEL oracle: each `jld:CompactTest` input is expanded and
    // compacted through the spec Compaction Algorithm (`sparq_jsonld::compact`), then
    // deep-compared against the suite's NORMATIVE EXPECTED document (`json_ld_equal`
    // — the same oracle shape as the expand/flatten lanes). Replaces the old oxjsonld
    // self-reparse round-trip over the engine's RDF-first writer (sq-3uos5), which
    // measured RDF losslessness rather than the Compaction Algorithm; see the
    // side-by-side re-pin on `floors::compact`. The floor is the MEASURED pass count
    // at the pinned revision; the 18 SKIPs (17 negatives + t0038, the 1.0-only
    // positive reclassified from a below-floor fail per sq-uzdw7 — narrowly id-pinned,
    // scope enforced by the runner's t0038_skip_is_narrowly_scoped) are documented there.
    // Floor kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C JSON-LD 1.1 compact",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [OPUS-4.8] sq-oy1f.16 — RAISED 163 → 186 (#978 writer faithfulness fixes).
        // [FABLE-5] sq-oy1f.27 — RE-PINNED 186 → 228 with the oracle correction to
        // the native document-level Compaction Algorithm (see floors::compact).
        // [FABLE-5] sq-oy1f.40 — LIB-SIDE floor const single source.
        ratchet_floor: crate::floors::compact::FLOOR,
        floor_basis: "pass",
        note: "native document-level Compaction Algorithm (sparq-jsonld), compared \
               against the W3C EXPECTED compacted document (json_ld_equal — the \
               normative document oracle); NegativeEvaluationTests RUN against the \
               manifest expectErrorCode since sq-gzsky (a wrong code is a FAIL)",
    },
    // [OPUS-4.8] sq-oy1f.19 — the W3C JSON-LD 1.1 `frame` ratchet (epic sq-oy1f),
    // over the SEPARATE w3c/json-ld-framing suite (fetch-jsonld-framing-tests.sh).
    // Each `jld:FrameTest` input (an arbitrary EXPANDED JSON-LD document) is parsed
    // to RDF (the real oxjsonld path), framed against the case frame document
    // through the native hand-rolled Framing Algorithm (`graph_to_jsonld_framed`,
    // serialize-rdf), then the framed output is re-parsed and required to
    // reconstruct the SAME RDF dataset as the suite's NORMATIVE expected output
    // (`reparse(frame(D, F)) ≡ reparse(expected)` — framing is a SELECT+RESHAPE, so
    // the oracle anchors on the expected document, NOT the input). The floor is the
    // MEASURED pass count; the remaining cases are honest framer divergences (below
    // the floor, to RISE) or documented SKIP (the 3 frame-validation negatives
    // sparq's TOTAL framer does not raise). Floor kept in lock-step by
    // `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C JSON-LD 1.1 frame",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [FABLE-5] sq-oy1f.40 — LIB-SIDE floor const single source.
        ratchet_floor: crate::floors::frame::FLOOR,
        floor_basis: "pass",
        note: "RDF → framed JSON-LD through the native Framing Algorithm \
               (serialize-rdf) over the w3c/json-ld-framing suite, compared by a \
               re-parse RDF-equivalence to the normative expected output",
    },
    // [OPUS-4.8] sq-oy1f — the W3C JSON-LD 1.1 `expand` + `flatten` ratchets (epic
    // sq-oy1f). Each `jld:ExpandTest` / `jld:FlattenTest` input is parsed to RDF (the
    // [SONNET-4.6] sq-kk1mq oracle-correction re-baseline: the expand lane now calls
    // sparq_jsonld::expand() directly and compares the result to the suite's expected
    // document via json_ld_equal (document-level JSON comparator measuring JSON-LD
    // data-model (semantic) equivalence, NOT structural identity; object key order
    // insignificant; array order significant only inside @list; integers compared exactly
    // via i64/u64, non-integral numbers as f64).  ~18 of 240 passes are semantically-
    // equal-but-reordered vs. the W3C reference (strict-ordered count 222).
    // The old floor was 247 under the RDF-equivalence oracle (sq-oy1f); the rebase
    // reveals a net 7 fewer passes (20 old-pass→new-fail flips minus 13 recoveries:
    // 8 old-fail→new-pass via oracle precision + 5 old-skip→new-pass via options
    // forwarding) and 26 new honest failures.  The expand floor was 240 (the MEASURED
    // pass count with the corrected oracle at the pinned suite revision, sq-kk1mq), then
    // RAISED to 259 by sq-oy1f.37 (three expand() correctness fixes: value-object
    // @type collapse, empty-array-property retention, free-floating value/list drop —
    // rise-only ratchet), then to 276 by sq-oy1f.45 (FsLoader wiring + six more fixes).
    // [OPUS-5] sq-gzsky — RAISED 276 → 381: the 109-case NegativeEvaluationTest SKIP
    // bucket (the whole expand gap) is closed — the lane now RUNS the negatives against
    // the manifest's `expectErrorCode` — plus seven spec-faithful sparq-jsonld fixes.
    // See src/floors/expand.rs for the itemised fix list and the 4 remaining fails.
    // [FABLE-5] sq-oy1f.26 — the flatten lane ALSO moved to the native document oracle
    // (sparq_jsonld::flatten() = expand ∘ node-map ∘ fold, compared via json_ld_equal),
    // re-pinned off the old RDF-writer oracle.  It composes over expand(), so it inherits
    // the sq-oy1f.37 expand raises above (the flatten floor is the MEASURED native-oracle
    // pass count on the merged tree) — see src/floors/flatten.rs.
    // Floors kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C JSON-LD 1.1 expand",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [FABLE-5] sq-oy1f.40 — LIB-SIDE floor const single source.
        ratchet_floor: crate::floors::expand::FLOOR,
        floor_basis: "pass",
        note: "native sparq_jsonld::expand() + json_ld_equal semantic-equivalence comparator \
               (sq-kk1mq; NOT structural identity — ~18/240 passes are reordered, \
               strict-ordered count 222; re-baseline from 247 under RDF-equivalence \
               oracle sq-oy1f); options forwarded (base, expandContext, processingMode); \
               NegativeEvaluationTests RUN against the manifest expectErrorCode since \
               sq-gzsky (a wrong code is a FAIL, never a pass)",
    },
    Suite {
        label: "W3C JSON-LD 1.1 flatten",
        family: "W3C JSON-LD",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "jsonld_suite",
            feature: "jsonld-suite",
        },
        ci_job: "jsonld-conformance",
        // [FABLE-5] sq-oy1f.40 — LIB-SIDE floor const single source.
        ratchet_floor: crate::floors::flatten::FLOOR,
        floor_basis: "pass",
        note: "native sparq_jsonld::flatten() (Flattening Algorithm §7.1 = expand ∘ \
               node-map ∘ named-graph fold) + json_ld_equal document-level comparator \
               (sq-oy1f.26; re-pin 50→46 from the RDF-writer oracle — the 4 drop is \
               inherited native-expand gaps owned by sq-oy1f.37, not flatten bugs)",
    },
    // [OPUS-4.8] sq-tmsd6 — the SolidLab ODRL Test Suite, wired as a crate-local
    // decision-parity ratchet in sparq-policy (mirrors the Solid WAC/ACP pattern:
    // the dev-only conformance crate must not take sparq-policy as a dep). Each of
    // the 68 self-describing Turtle cases is driven through the REAL
    // `parse_policy_str` + `evaluate` path; the oracle is the case's expected
    // compliance report (`report:activationState` ⇒ ALLOW/DENY). The floor is the
    // pass COUNT (67 at the pinned revision after the constraint-matching batch
    // sq-euhr3/sq-k7itg/sq-a0zef — `odrl:LogicalConstraint`, party/asset collection
    // membership, and the `odrl:use` action hierarchy now PASS); the 1 remaining
    // case is a documented NOT-IMPLEMENTED divergence (a duty whose discharge state
    // is unknown/`report:NonSet` — sparq is fail-closed) that does not fail the gate.
    // Floor kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "SolidLab ODRL Test Suite",
        family: "SolidLab ODRL",
        runner: Runner::CrateTest { krate: "sparq-policy", target: "odrl_test_suite" },
        ci_job: "odrl-conformance",
        ratchet_floor: sparq_conformance_floors::policy::ODRL_SUITE_FLOOR,
        floor_basis: "scenario",
        note: "library-level allow/deny parity over the SolidLab self-describing ODRL \
               cases through sparq-policy's real evaluate() path",
    },
    // [OPUS-4.8] sq-e5atd (epic sq-pbz04) — the W3C SPARQL 1.1 D-entailment
    // (datatype / value-space) ratchet. The runner is crate-local here
    // (`tests/d_entail_suite.rs`) but behind the OPT-IN `d-entail` feature (forwards
    // to sparq-reason/d-entail) so the default + `--workspace` builds neither link
    // the `Profile::D` materializer nor go red — the lean-core posture, and the
    // inference BINARY keeps these tests OutOfScope when the feature is off so its
    // own ratchet floor is byte-for-byte unchanged. The genuinely D-only
    // `sparql11/entailment` tests (regime `ent:D` without a stronger RDFS/RDF/
    // OWL-RDF-Based regime) GRADUATE from that OutOfScope bucket to Pass here:
    // premise → `sparq_reason::Profile::D` (rdfD1 typing + the typed value-space
    // comparator — "1"^^xsd:integer ≡ "1.0"^^xsd:decimal, NOT an f64 fast path) →
    // query over the closure through the same evaluation path as the SPARQL harness,
    // with the entailment-regime answer restriction applied. The floor is the
    // MEASURED D-only pass count at the pinned revision (NOT 100%; the broader
    // D-inconsistency / value-space-subset surface is tracked-not-asserted — a child
    // of sq-pbz04). Floor kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C SPARQL 1.1 D-entailment",
        family: "W3C RDF Semantics",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "d_entail_suite",
            feature: "d-entail",
        },
        ci_job: "inference-conformance",
        ratchet_floor: 1,
        floor_basis: "pass",
        note: "D-only sparql11/entailment tests graduated from OutOfScope to Pass \
               through sparq-reason's opt-in Profile::D (rdfD1 typing + typed \
               value-space equality)",
    },
    // [FABLE-5] sq-pbz04.5.5 (epic sq-pbz04.5) — the W3C RIF WG test-suite CONFORMANCE
    // arm, Core subset. The runner is crate-local here (`tests/rif_wg_core_suite.rs`)
    // but behind the OPT-IN `rif-wg-core` feature (forwards to sparq-reason/rif-xml — the
    // RIF/XML importer + rif-core model — plus sparq-substrate/numeric for the
    // value-aware conclusion compare) so the default + `--workspace` builds neither link
    // the importer nor go red — the lean-core posture. DISTINCT from the sparq-EXTENSION
    // `rif_core_suite.rs` expressivity ratchet (below): THIS drives the ACTUAL W3C RIF WG
    // test cases (the pinned Core_v1.22 archive) end-to-end through the real path —
    // RIF/XML import → validate → closure → conclusion oracle — as a STANDARDS-suite lane
    // (family "W3C RIF") with an HONEST denominator: the printed per-category skip
    // taxonomy IS the denominator's honesty. The FLOOR is the ACTUAL MEASURED pass count
    // at the pinned archive — MEASURED 5 (3 PositiveEntailment + 1 PositiveSyntax +
    // 1 NegativeSyntax) after sq-n7y15 positional-Atom import and sq-jsgyn multi-slot
    // frame import; arity-3+ atoms, Import-closure and local constants still skip.
    // The load-bearing NET vacuity rule (un-importable premise = SKIP, never a vacuous
    // "not entailed") is enforced + tested. RISE-READY: the floor ratchets up as importer
    // Core coverage grows. The SPARQL RIF entailment regime (sparql11/entailment rif01..rif06)
    // stays tracked-not-asserted out-of-scope. Floor kept in lock-step by
    // `tests/scoreboard_floors.rs`. [SONNET-4.6] sq-n7y15
    Suite {
        label: "W3C RIF WG Core test suite",
        family: "W3C RIF",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "rif_wg_core_suite",
            feature: "rif-wg-core",
        },
        ci_job: "inference-conformance",
        // [SONNET-4.6] sq-n7y15: raised 0 -> 3 by positional-Atom import. [SONNET-4.6]
        // #3469: raised 3 -> the MEASURED 5 after re-running the lane against the fetched
        // Core_v1.22 archive (sq-jsgyn multi-slot frames graduated 2 more tests). Mirror
        // of `RIF_WG_CORE_FLOOR` in tests/rif_wg_core_suite.rs (kept in lock-step).
        ratchet_floor: 5,
        floor_basis: "pass",
        note: "the W3C RIF WG test cases (Core dialect, pinned Core_v1.22 archive) driven \
               end-to-end through the real RIF/XML import -> validate -> closure -> \
               conclusion oracle; an HONEST-denominator standards lane (the printed \
               skip taxonomy is the honesty), NET-vacuity-guarded, floor = the MEASURED \
               pass count (5 at the sq-jsgyn multi-slot-frame importer, rise-ready)",
    },
    // [FABLE-5] sq-pbz04.6.4 (epic sq-pbz04.6) — the sparq D VALUE-SPACE MATRIX arm, a
    // sparq EXTENSION ratchet tallied SEPARATELY from the W3C D-entailment row above
    // (the W3C `sparql11/entailment` corpus is a SINGLE D-only test; the real value-space
    // coverage is sparq's own hand-authored matrix). Mirrors the OWL 2 QL / EL / RIF-Core
    // extension precedent — program honesty rule 4: the standards-conformance count is NOT
    // padded with sparq's own cases. The floor const (`pub const D_VALUE_MATRIX_FLOOR`)
    // lives in `tests/d_entail_suite.rs` (behind the opt-in `d-entail` feature, inside the
    // `gated` module — the guard reads it TEXTUALLY, so the `#[cfg]`/module nesting do not
    // affect the match); `tests/scoreboard_floors.rs` pins this mirror to it so the two can
    // never drift. The runner drives value-equal-distinct-lexical pairs (integer⊂decimal
    // incl. the 2^53+1 non-aliasing guard, boolean true/1, the hex/base64 octet pair),
    // facet-ill-formed negatives (rdfD1 must NOT type 200^^byte / a leading-space token),
    // and disjoint-space negatives (decimal vs double, date vs dateTime) through the REAL
    // `Profile::D` value-space comparator (now on the shared sparq-substrate seam,
    // sq-pbz04.6.3) — plus broadened-map end-to-end cases through the same
    // materialize→answer-restriction→engine-query path as the W3C lane.
    Suite {
        label: "D value-space matrix (integer/decimal/boolean/binary/temporal)",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "d_entail_suite",
            feature: "d-entail",
        },
        ci_job: "inference-conformance",
        ratchet_floor: 24,
        floor_basis: "value-space assertions (sparq EXTENSION over the D datatype map, \
                      NOT the W3C sparql11/entailment conformance count)",
        note: "EXTENSION ratchet — sparq's own hand-authored D value-space matrix over the \
               recognized datatype map (integer⊂decimal incl. 2^53+1 non-aliasing, boolean \
               true/1, hex/base64 octet identity, facet-ill-formed negatives, decimal-vs-\
               double + date-vs-dateTime disjoint-space negatives), driven through the REAL \
               Profile::D value-space comparator + the materialize→answer-restriction→engine \
               end-to-end path; tallied SEPARATELY, never faked as W3C conformance passes",
    },
    // [OPUS-4.8] sq-ddpgx (epic sq-my8wd) — the W3C SPARQL 1.1 `sparql11/service`
    // EVALUATION ratchet. The runner is crate-local here
    // (`tests/service_eval_suite.rs`) but behind the OPT-IN `service` feature
    // (forwards to `service-loopback` → tokio + axum via sparq-server/service AND
    // ureq via sparq-engine/service) so the default + `--workspace` builds neither
    // link the async server stack nor go red — the lean-core posture; the SPARQL
    // BINARY keeps these tests skipped (`unsupported_feature = "SERVICE /
    // federation"`) so its ratchet floor is byte-for-byte unchanged. These tests
    // GRADUATE from that skip bucket to Pass here: each `qt:serviceData` block is
    // served by a REAL in-process `sparq_server::serve` loopback endpoint (the
    // merged sq-ushvx #1291 harness) on an ephemeral 127.0.0.1:0 port, the
    // well-known endpoint IRIs are rewritten to the bound loopback URLs, and the
    // federated SERVICE query runs end-to-end through the engine's REAL ureq
    // transport, compared to the `.srx` oracle. The floor is the MEASURED pass
    // count at the pinned rdf-tests revision (NOT 100%: a variable SERVICE endpoint
    // is the one remaining honest tracked-not-asserted divergence — documented Skip,
    // never skip-laundered into the count; [SONNET-4.6] sq-my8wd.1 raised the floor
    // from 5→6 by handling nested non-SILENT SERVICE via per-endpoint egress config).
    // Floor kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C SPARQL 1.1 sparql11/service evaluation",
        family: "W3C SPARQL",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "service_eval_suite",
            feature: "service",
        },
        ci_job: "service-federation-conformance",
        ratchet_floor: 6,
        floor_basis: "pass",
        note: "sparql11/service federated SERVICE queries run end-to-end through \
               REAL in-process loopback endpoints (the sq-ushvx harness) + the \
               engine's REAL ureq transport, compared to the .srx oracle",
    },
    // [OPUS-4.8] sq-jaj38 (epic sq-my8wd) — the W3C SPARQL 1.1 PROTOCOL (HTTP layer)
    // conformance ratchet. Where the sibling sparql11/service row above graduates the
    // federated SERVICE EVALUATION tests, THIS row covers the HTTP PROTOCOL itself: the
    // request/response contract a SPARQL endpoint MUST honour. The runner is crate-local here
    // (`tests/http_protocol_suite.rs`) but behind the OPT-IN `http-protocol` feature (forwards
    // to `service-loopback` → tokio + axum via sparq-server/server; NO new third-party dep —
    // the raw HTTP client is a std-only TcpStream helper) so the default + `--workspace` builds
    // neither link the async server stack nor go red — the lean-core posture. Each assertion is
    // a RAW HTTP request to the merged sq-ushvx in-process loopback server (an ephemeral
    // 127.0.0.1:0 port) with an exact method / Content-Type / Accept / body, checking the
    // status code, the response Content-Type and the payload shape: query via GET /
    // POST-urlencoded / POST-direct, the HTTP QUERY method (#1304), update via POST, the
    // default-graph-uri / named-graph-uri dataset overrides, result-format content negotiation
    // (SRJ / SRX / CSV / TSV), the present-but-unsatisfiable Accept → 406 (Oxigraph parity,
    // sq-406acc) and the 200/400/405/406/415 status codes. The floor is the MEASURED PASS count;
    // documented protocol divergences (an absent/*/* Accept defaults to JSON per the W3C-permitted
    // default representation; an ASK boolean in CSV/TSV falling back to JSON) are reported
    // separately and NOT summed into it, so a documented gap can never inflate the conformance
    // number — an honest W3C-Protocol claim, scoped to what the server genuinely satisfies. Floor
    // kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "W3C SPARQL 1.1 Protocol (HTTP)",
        family: "W3C SPARQL",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "http_protocol_suite",
            feature: "http-protocol",
        },
        ci_job: "service-federation-conformance",
        ratchet_floor: 21,
        floor_basis: "pass",
        note: "SPARQL 1.1 Protocol operations (GET/POST query+update, the QUERY method, \
               dataset overrides, SRJ/SRX/CSV/TSV negotiation, 200/400/405/406/415) over RAW \
               HTTP against the in-process loopback server; a present-but-unsatisfiable Accept \
               now returns 406 (Oxigraph parity, sq-406acc); the absent/*/* Accept JSON default + \
               ASK-in-CSV are documented divergences, NOT summed into the floor",
    },
    // [OPUS-4.8] sq-1uuxz (epic sq-my8wd) — the SPARQL 1.1 SERVICE-DESCRIPTION + GRAPH-STORE
    // PROTOCOL conformance ratchet. Where the sibling http-protocol row above covers the
    // query/update Protocol contract, THIS row covers the two federation-descriptor + write
    // surfaces the server exposes behind `federation-descriptors`: (A) the `GET /sparql` (no
    // query) Service-Description document — asserting it advertises EXACTLY the result/input
    // formats + query/update languages + SPARQL versions + BasicFederatedQuery the server
    // GENUINELY implements (no over-advertising; each advertised result format is cross-checked
    // against a real SELECT, and JSON-LD — not served in this build — must NOT appear); and (B) a
    // full GET/PUT/POST/DELETE Graph-Store-Protocol round-trip on a named graph (indirect
    // `?graph=` + direct `/graphs/<path>`) and the default graph (`?default`), VERIFYING store
    // state after every op (PUT→GET-back-equal; PUT replaces; POST merges; DELETE removes;
    // 200/201/204/400/404/405/415). The runner is crate-local here (`tests/sd_gsp_suite.rs`) but
    // behind the OPT-IN `federation-descriptors` feature (forwards to `service-loopback` → tokio +
    // axum via sparq-server/server, AND turns on sparq-server/federation-descriptors so the SD
    // endpoint is live; NO new third-party dep — the raw HTTP client is the same std-only
    // TcpStream helper) so the default + `--workspace` builds neither link the async server stack
    // nor go red — the lean-core posture. The floor is the MEASURED PASS count; the one documented
    // divergence (a GSP read of an absent named graph is 200+empty, not 404 — GSP-permitted) is
    // reported separately and NOT summed into it, so a documented gap can never inflate the
    // conformance number. Floor kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "SPARQL 1.1 Service Description + Graph Store Protocol",
        family: "W3C SPARQL",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "sd_gsp_suite",
            feature: "federation-descriptors",
        },
        ci_job: "service-federation-conformance",
        ratchet_floor: 39,
        floor_basis: "pass",
        note: "the GET /sparql (no query) Service-Description advertises exactly the \
               formats/languages/versions/features the server genuinely implements (no \
               over-advertising), PLUS a GET/PUT/POST/DELETE Graph-Store-Protocol round-trip \
               (named + default graph, indirect + direct identification) verifying store state \
               after each op; absent-graph 200-empty read is a documented divergence, NOT summed in",
    },
    // [OPUS-4.8] sq-ripcg (epic sq-lk3aw) — the sparq-text DIFFERENTIAL BM25 ORACLE
    // (runner lives crate-local in `sparq-text/tests/bm25_oracle.rs`). This is
    // HONESTLY a sparq EXTENSION ratchet, NOT a standards-conformance claim: BM25 is
    // an information-retrieval scoring function (Robertson & Spärck Jones), and there
    // is NO normative "full-text search over RDF" recommendation or its accompanying
    // test suite to point at — so `family = "sparq extension"` (the only family in
    // this registry with NO standards body behind it; even the OGC GeoSPARQL
    // query-rewrite EXTENSION row stays `family = OGC GeoSPARQL` because it extends a
    // standard, whereas BM25-over-RDF extends nothing normative). Deliberately
    // distinct so the row never reads as a W3C/OGC/IETF/Solid conformance claim. The
    // runner recomputes
    // the exact BM25 score of every `search`/`search_any` hit with a from-scratch
    // independent reference scorer and asserts the index reproduces it bit-for-bit
    // (`f32::to_bits`) AND in the same best-first order, over a fixed-seed corpus
    // battery. The floor is the ACTUAL measured count of score-exact assertions the
    // battery makes (the `assertions N (floor F)` line the runner prints) — it may
    // only RISE, and a drop is a scoring regression. The runner stays crate-local
    // (the dev-only conformance crate must NOT take sparq-text as a dep — same
    // constraint as SHACL/geo/Solid/ODRL), so this row takes NO new dependency edge
    // on sparq-text; the floor const `TEXT_ORACLE_FLOOR` is mirrored here and kept in
    // lock-step by `tests/scoreboard_floors.rs` (read textually, no cross-crate dep).
    Suite {
        label: "text-search differential oracle",
        family: "sparq extension",
        runner: Runner::CrateTest { krate: "sparq-text", target: "bm25_oracle" },
        ci_job: "text-oracle",
        ratchet_floor: sparq_conformance_floors::text::BM25_ORACLE_FLOOR,
        floor_basis: "score-exact assertions (sparq EXTENSION, NOT standards conformance)",
        note: "EXTENSION ratchet — no normative full-text-over-RDF / BM25 standard \
               exists: sparq-text's BM25 search/ranking vs a from-scratch independent \
               reference scorer, bit-exact per-hit scores + best-first order over a \
               fixed corpus battery",
    },
    // [OPUS-4.8] sq-mcb3q (epic sq-2n1q3) — the sparq-rsp RSP EXPRESSIVITY /
    // SRBench CORRECTNESS oracle (runner lives crate-local in
    // `sparq-rsp/tests/srbench_oracle.rs`). HONESTLY a sparq EXTENSION ratchet,
    // NOT a standards-conformance claim: SRBench (Zhang/Della Valle/Calbimonte et
    // al., ISWC 2012) is the canonical RSP correctness/expressivity BENCHMARK and
    // RSP-QL is a W3C-COMMUNITY spec — there is NO W3C/OGC/IETF Recommendation for
    // RDF Stream Processing and no normative RSP conformance test suite to point
    // at (unlike SPARQL/SHACL/GeoSPARQL/JSON-LD, the real-conformance rows above).
    // So `family = "sparq extension"` (the same NOT-a-standards-body marker the
    // BM25 row carries). The runner drives the crate's REAL public pipeline
    // (ContinuousQuery / ContinuousMultiQuery over WindowSpec windows,
    // EvalMode-materialised, R2S-filtered) across the SRBench expressivity axes —
    // window TYPES (tumbling + sliding time + CQL count windows), stream OPERATORS
    // (RSTREAM/ISTREAM/DSTREAM), all four EvalModes (pinned identical), and
    // multi-window JOINS (observations ⋈ station-metadata) with
    // aggregate-after-join — and asserts every closed window against an INDEPENDENT
    // oracle (a batch-rebuild via `sparq_engine::query` plus closed-form
    // hand-derived row multisets) that shares no code with the streaming closure /
    // materialisation path. Documented RSP-QL gaps (window VARIABLES, textual ROWS
    // windows, relative NOW bounds, I/DSTREAM over a join) are NOT inflated into
    // passes — they are asserted genuinely-rejected. The floor is the ACTUAL
    // measured count of those per-window correctness assertions (the
    // `assertions N (floor F)` line the runner prints) — it may only RISE, and a
    // drop is an S2R/R2R/EvalMode/join/R2S-diff regression. The runner stays
    // crate-local (the dev-only conformance crate must NOT take sparq-rsp as a dep
    // — same constraint as SHACL/geo/Solid/ODRL/text), so this row takes NO new
    // dependency edge on sparq-rsp; `RSP_EXPRESSIVITY_FLOOR` is mirrored here and
    // kept in lock-step by `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "RSP expressivity / SRBench correctness",
        family: "sparq extension",
        runner: Runner::CrateTest { krate: "sparq-rsp", target: "srbench_oracle" },
        ci_job: "rsp-oracle",
        // [SONNET-4.6] sq-2n1q3.3 raised from 303 (from 149 sq-mcb3q baseline).
        ratchet_floor: sparq_conformance_floors::rsp::EXPRESSIVITY_FLOOR,
        floor_basis: "per-window correctness assertions (sparq EXTENSION, NOT standards conformance)",
        note: "EXTENSION ratchet — no normative RDF-Stream-Processing standard / RSP \
               conformance suite exists (RSP-QL is a W3C-community spec; SRBench a \
               benchmark): sparq-rsp's windowed continuous queries vs an INDEPENDENT \
               batch-rebuild + closed-form oracle across window types / R2S operators \
               / EvalModes / multi-window joins; documented RSP-QL gaps asserted \
               genuinely-rejected, not faked as passes",
    },
    // [OPUS-4.8] sq-rh4gu (epic sq-pbz04) — the RIF-Core EXPRESSIVITY ratchet
    // (runner lives crate-local in `sparq-conformance/tests/rif_core_suite.rs`,
    // behind the opt-in `rif-core` feature). HONESTLY tallied as a sparq EXTENSION
    // ratchet, NOT folded into the conformance total — even though RIF-Core is a
    // real W3C dialect, this lane runs sparq's OWN faithful expressivity battery
    // over the RIF-Core (monotone Horn) subset the `sparq_reason::rif` front-end
    // implements, NOT the normative W3C SPARQL-RIF Core Entailment Regime test
    // suite (sparql11/entailment rif01..rif06, which is a strictly larger
    // SPARQL-protocol integration this bead does NOT deliver — it is a documented
    // tracked-not-asserted out-of-scope item in the runner's OUT_OF_SCOPE list).
    // So the unit is per-feature EXPRESSIVITY assertions (not normative-suite
    // pass-counts), and it is tallied SEPARATELY like the BM25 / RSP extension rows
    // — never inflating a conformance number with a non-conformance one. The runner
    // drives the REAL `rif::Document::{validate, closure}` path across the RIF-Core
    // axes (frame/membership/subclass/equality, recursion, the numeric/string/list
    // builtins with range-restriction SAFETY enforced, monotonicity, and the
    // canonical W3C `rif01` uncle rule), asserting every unsafe rule is GENUINELY
    // rejected and the NAF/nonmonotonic surface genuinely-absent. The floor is the
    // MEASURED assertion count (the `RIF-Core expressivity assertions N` line) — it
    // may only RISE; `RIF_CORE_FLOOR` is mirrored here and kept in lock-step by
    // `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "RIF-Core expressivity (monotone Horn subset)",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "rif_core_suite",
            feature: "rif-core",
        },
        ci_job: "inference-conformance",
        // [SONNET-4.6] sq-pbz04.5.2 — raised 47 → 58: 11 new assertions for the 5
        // new soundly-mapped builtins (NumericNotEqual, StringUpperCase, StringLowerCase,
        // StringEncodeForUri, ListConcatenate).
        // [SONNET-4.6] sq-pbz04.5.4 — Equal-atom audit: equal_atom_audit added FIVE
        // assertions (Equal-in-fact-head + closure-refuses + Equal-in-rule-head +
        // ground-identity fires + DistinctGroundEqual fail-closed) and positive_atoms
        // dropped 1 (the removed "equality lowers to owl:sameAs"). NB: the earlier
        // "4 added" note undercounted — it was 5 added, 1 removed.
        // [OPUS-4.8] sq-26vwp — raised to 73: +10 assertions for variable/mixed body
        // Equal resolved by compile-time substitution/unification (V1/V2, ?x=<t>
        // substitution, head-var bind, chained collapse, distinct-ground fail-closed).
        // [SONNET-4.6] sq-anyad — raised to 76: the distinct-ground Equal item of
        // equal_atom_audit grew from 1 fail-closed assertion to 4 (numeric value-equal
        // validates + fires, numeric value-unequal is vacuous, non-numeric still fails
        // closed) when the NUMERIC half of the value-space deferral landed on the
        // sq-v5evr comparator.
        // Mirrors RIF_CORE_FLOOR in rif_core_suite.rs (scoreboard_floors guard checks sync).
        ratchet_floor: 76,
        floor_basis: "expressivity assertions (sparq EXTENSION over the RIF-Core subset, \
                      NOT the normative W3C SPARQL-RIF conformance suite)",
        note: "EXTENSION ratchet — sparq's own faithful expressivity battery over the \
               RIF-CORE (monotone Horn) subset implemented by sparq-reason's rif:: \
               front-end (frame/membership/subclass/equality, recursion, numeric/string/\
               list builtins with range-restriction safety, monotonicity, the canonical \
               rif01 uncle rule); the normative SPARQL-RIF Core Entailment Regime + full \
               RIF-BLD/PRD are documented out-of-scope, asserted genuinely-rejected, \
               never faked as passes",
    },
    // [OPUS-4.8] sq-qo1a9 (epic sq-pbz04, the LAST conformance bead) — the GRADUATED
    // OWL 2 QL (DL-Lite_R) CERTAIN-ANSWER oracle ratchet (runner lives crate-local
    // in `sparq-conformance/tests/ql_dllite_suite.rs`, behind the opt-in
    // `ql-experimental` feature). HONESTLY tallied as a sparq EXTENSION ratchet, NOT
    // folded into the conformance total — even though OWL 2 QL is a real W3C profile,
    // there is NO runnable normative W3C "QL certain-answer conformance suite" to
    // point a harness at (the W3C QL material is structural/classification, not an
    // answer-comparison corpus over the query-rewriting semantics). So — exactly like
    // the RIF-Core / RSP / BM25 extension rows — this lane runs sparq's OWN faithful
    // DL-Lite_R certain-answer oracle (the hand-derived suite from sq-g19x0): each
    // case is a conjunctive query within SOUND DL-Lite_R rewriting, with a
    // certain-answer set derived BY HAND from the DL-Lite_R semantics. The runner
    // rewrites each case with the REAL `sparq_reason_ql::rewrite_production`
    // (PerfectRef ∪ tree-witness ∪ UCQ-min) and evaluates the UCQ over the UNMODIFIED
    // ABox through the REAL engine, asserting it returns EXACTLY the certain answers —
    // sound (no extra) AND complete (no missing) case by case. The floor is the
    // MEASURED count of sound-and-complete cases (the `QL DL-Lite_R sound-and-complete
    // N (floor F)` line) — it may only RISE; a divergence is a soundness/completeness
    // regression. The BROADER `pr:QL` `sparql11/entailment` arm (intensional /
    // non-DL-Lite certain-answer cases the rewriter cannot soundly answer) stays
    // EXPERIMENTAL / OutOfScope (`tests/ql_experimental_arm.rs`), NEVER summed in.
    // `QL_DLLITE_FLOOR` is mirrored here and kept in lock-step by
    // `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "OWL 2 QL (DL-Lite_R) certain-answer oracle",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "ql_dllite_suite",
            feature: "ql-experimental",
        },
        ci_job: "inference-conformance",
        ratchet_floor: 11,
        floor_basis: "sound-and-complete certain-answer cases (sparq EXTENSION over the \
                      DL-Lite_R oracle, NOT a full-OWL-2-QL-conformance claim)",
        note: "EXTENSION ratchet — no runnable normative W3C OWL 2 QL certain-answer \
               conformance suite exists (the W3C QL material is structural): sparq's own \
               faithful DL-Lite_R certain-answer oracle (the hand-derived suite from \
               sq-g19x0), each conjunctive query rewritten by sparq_reason_ql's REAL \
               rewrite_production (PerfectRef ∪ tree-witness ∪ UCQ-min) and evaluated over \
               the UNMODIFIED ABox, asserting EXACTLY the hand-derived certain answers — \
               sound AND complete case by case; the broader pr:QL entailment-arm \
               intensional gap stays experimental/OutOfScope, never faked as a pass",
    },
    // [FABLE-5] sq-pbz04.3.4 (epic sq-pbz04.3) — the OWL 2 QL ENTAILMENT-REGIME
    // GRADUATED-SUBSET ratchet (runner lives crate-local in
    // `sparq-conformance/tests/ql_entailment_floor.rs`, behind the opt-in
    // `ql-experimental` feature). HONESTLY tallied as a sparq EXTENSION ratchet, NOT
    // folded into the conformance total: sparq implements a FRAGMENT of the W3C
    // SPARQL 1.1 QL entailment regime (holding everything else with an exhaustive
    // reason taxonomy), so no full-regime / full-profile OWL 2 QL conformance claim
    // is made anywhere. A `pr:QL` `sparql11/entailment` case is in the floor iff ALL
    // SIX graduation conditions pass — each CHECKED in code by
    // `inference::sparql_entail::run_ql_graduation`: (1) the fail-closed CQ-shape
    // gate accepts it and it carries no intensional schema-vocabulary atom; (2) the
    // DL-Lite_R TBox is totally captured (`fully_captured()`, sq-pbz04.3.3); (3)
    // zero consistency-relevant (negative/disjointness) axioms; (4) default-graph
    // dataset only; (5) the regime-coincidence guard (certain-answer vs
    // solution-mapping semantics provably coincide: all body terms distinguished, or
    // no existential-generating inclusions); (6) the rewritten UCQ evaluated over
    // the UNMODIFIED data is result-equivalent to the W3C oracle. The floor is a
    // PINNED NAMED-CASE list (exact set equality): a pinned case regressing fails
    // CI, and a newly-eligible case fails CI until pinned deliberately with
    // evidence. Distinct from the DL-Lite_R certain-answer oracle row above (the
    // hand-derived sq-g19x0 corpus, untouched); the inference BINARY keeps every QL
    // row OutOfScope so this floor can never leak into a conformance number.
    // `QL_ENTAILMENT_FLOOR` is mirrored here and kept in lock-step by
    // `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "OWL 2 QL entailment-regime graduated subset",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "ql_entailment_floor",
            feature: "ql-experimental",
        },
        ci_job: "inference-conformance",
        // [SONNET-4.6] sq-pbz04.3.1 raised 9 → 11: `lang` + `plainLit` graduate under the
        // B2 literal-object broadening (SELECT ?x WHERE { ?x foaf:name "name"@en }, TBox
        // foaf:name a owl:DatatypeProperty — fully_captured, no existential generators,
        // identity rewrite returns exactly {:b}, result-equivalent to the W3C oracle).
        // [OPUS-4.8] sq-pbz04.3.6 raised 11 → 15: the body-blank-node lifting graduates the four
        // undistinguished-variable `sparqldl` cases. `sparqldl-05` (ASK { _:a rdf:type :Person })
        // + `sparqldl-06` (ASK over a 4-hop blank-node cycle) graduate directly. `sparqldl-07` +
        // `sparqldl-08` (SELECT * with a SHARED body blank node = an existential JOIN) graduate
        // only because the same bead ALSO fixed emit::cq_to_bgp to map a repeated Unbound id to
        // ONE emitted variable (preserving the join; the prior per-occurrence naming emitted a
        // cartesian product that condition (6) correctly held as oracle-divergent). All four TBoxes
        // are declaration-only (exists_super empty, condition (5) holds); each identity rewrite is
        // result-equivalent to the W3C oracle.
        ratchet_floor: 15,
        floor_basis: "graduated named pr:QL cases — six-condition soundness predicate (sparq \
                      EXTENSION over the QL fragment sparq rewrites, NOT an OWL 2 QL / \
                      entailment-regime conformance claim)",
        note: "EXTENSION ratchet — the pr:QL sparql11/entailment cases that pass ALL SIX \
               graduation conditions (CQ-shape gate + intensional guard, total TBox capture, \
               the consistency condition [zero negative axioms OR the sq-p6yb7 DL-Lite_R \
               violation-query check proves the KB consistent], default-graph dataset, the \
               regime-coincidence guard, and empirical result-equivalence to the W3C oracle \
               through the REAL rewrite_production + engine), pinned as an exact named-case \
               list; every non-graduated case is held with an exhaustive reason taxonomy \
               (permanently-outside / pending-gate / pending-capture / pending-consistency / \
               inconsistent-kb / pending-coincidence / oracle-divergent), never faked as a pass",
    },
    // [SONNET-4.6] sq-pbz04.2.4 (epic sq-pbz04) — the OWL 2 EL classification ratchet
    // (runner lives crate-local in `sparq-conformance/tests/el_suite.rs`, behind the
    // opt-in `el-suite` feature). HONESTLY tallied as a sparq EXTENSION ratchet, NOT
    // folded into the conformance total — even though OWL 2 EL is a real W3C profile,
    // this lane compares each W3C OWL 2 EL test (test:EL ∧ test:RDF-BASED, Approved,
    // no-imports) against what `sparq-reason-el`'s consequence-based classifier +
    // ABox realiser genuinely computes.
    // [SONNET-4.6] sq-pbz04.2.9: the `el-suite` feature also forwards
    // `sparq-reason-el/rbox` + `sparq-reason-el/cdomain` so the CI lane exercises the
    // FULL shipped feature set; the mutual-subsumption → owl:equivalentClass
    // output-vocabulary completion graduates WebOnt-equivalentClass-003.
    // [SONNET-4.6] sq-pbz04.2.10: ABox graduation — `el-suite` now ALSO forwards
    // `sparq-reason-el/abox`; the runner uses the two-step `classify_graph` (TBox
    // closure) + `realize_graph` (ABox rows + whole-ontology inconsistency verdict)
    // composition; 16 tests graduate (8 inconsistency + 8 positive-entailment) dropping
    // the audited divergence list from 27 to 11 permanent entries; floor raised 51→67.
    // `EL_SUITE_FLOOR` is mirrored here and kept in lock-step by
    // `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "OWL 2 EL classification (sparq-reason-el)",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "el_suite",
            feature: "el-suite",
        },
        ci_job: "inference-conformance",
        // [SONNET-4.6] sq-pbz04.2.10 — RAISED 51 → 67: ABox graduation (16 tests pass
        // via realize_graph + augment_equivalent_properties); rbox + cdomain + abox now
        // all on in the el-suite CI lane (full shipped feature set).
        ratchet_floor: 67,
        floor_basis: "pass — classifier + ABox realiser compute the expected outcome (sparq \
                      EXTENSION over the EL fragment implemented, NOT a full-OWL-2-EL-conformance \
                      claim)",
        note: "EXTENSION ratchet — the W3C OWL 2 EL suite checks classification/subsumption \
               semantics and this runner compares against what sparq-reason-el's classifier + \
               ABox realiser compute over the EL fragment (CR1–CR6 + rbox + cdomain + abox): \
               each premise classified through classify_graph (TBox closure) then realize_graph \
               (ABox rows + inconsistency verdict), then consistency / inconsistency / positive- \
               / negative-entailment via the shared bnode-homomorphism check (output-vocabulary \
               completions: datatypes + equivalentClass + equivalentProperty); the 11 audited \
               permanent divergences (property-chain ABox / reflexive property / annotation \
               propagation / equivalentProperty extraction / unionOf / bottomDataProperty / \
               FunctionalProperty) are reported separately, never faked as passes",
    },
    // [FABLE-5] sq-pbz04.4.5 (epic sq-pbz04.4) — the OWL 2 DIRECT-SEMANTICS arm's two
    // ratchets (runner: `inference::dl_suite` + the crate-local `tests/dl_suite.rs`,
    // behind the opt-in `dl-direct` feature → `sparq-reason-dl/dispatch` + the default-off
    // `sparq-reason-dl/dl_transitive` extension). HONESTLY
    // tallied as sparq EXTENSION rows over the SCOPED FRAGMENT the layered
    // `sparq-reason-dl` checker implements — **scoped fragment, NOT full OWL 2 DL** —
    // and never folded into the standards-conformance total. TRI-STATE accounting
    // {Pass, Fail, OutOfFragment(reason)}: an ABSTENTION IS NEVER A PASS, every
    // divergent definitive verdict is pinned BY NAME with an audited mechanism in the
    // runner (exact set equality), and the floors are EXACT-pinned in-runner (`==`,
    // not `>=`, so abstention-inflation and regression BOTH fail CI). The
    // profile-identification row checks POSITIVE `test:profile` tags only (the design
    // record §4 fallback — L2's `In` is fragment-grammar membership and cannot refute
    // full-profile membership, a limit the lane MEASURED); the Direct row runs
    // consistency / inconsistency / positive- / negative-entailment through the L4
    // `DirectChecker` dispatch under a PINNED deterministic count budget (wall-clock
    // budgets banned). The dual-tagged tests' RDF-Based runs stay in the inference
    // binary's RL `owl_suite` / the `el-suite` lane — one test may appear in both
    // tallies because the two runs test DIFFERENT semantics (record §4). Floors read
    // TEXTUALLY by `tests/scoreboard_floors.rs`.
    Suite {
        label: "OWL 2 DL profile identification (Direct arm)",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "dl_suite",
            feature: "dl-direct",
        },
        ci_job: "inference-conformance",
        ratchet_floor: 93,
        floor_basis: "positive-tag membership passes, EXACT-pinned (sparq EXTENSION over the \
                      L1/L2 ALCH-fragment checker — scoped fragment, NOT full OWL 2 DL and NOT \
                      a W3C ProfileIdentificationTest conformance claim); re-pinned by \
                      sq-pbz04.4.16 (M7 singleton-intersection normalization: +27, 68 -> 95); \
                      re-pinned by sq-pbz04.4.9 (L1 datatype-map-IRI refusal: -1, 95 -> 94 — the \
                      WebOnt-I5.3-015 EL profile row whose premise carries xsd:integer/xsd:string \
                      ranges now refuses extraction and honestly abstains, was a pass under the \
                      old opaque-datatype reading); re-pinned by sq-pbz04.4.8 (L1 built-in \
                      fixed-extension property refusal: -1, 94 -> 93 — the \
                      New-Feature-BottomObjectProperty-001 EL profile row, whose premise uses \
                      owl:bottomObjectProperty, now refuses extraction and honestly abstains)",
        note: "EXTENSION ratchet — the DIRECT-arm ProfileIdentificationTest cases whose \
               POSITIVE test:profile tags the L2 syntactic checker reproduces through the \
               REAL fail-closed L1 extraction + grammar walk; abstentions are never passes. \
               The 27 singleton-intersection (M7) divergences are FIXED by sq-pbz04.4.16 (L1 \
               normalizes a 1-ary owl:intersectionOf to its member) and now pass; the \
               positive PROFILE_DIVERGENCES pin is empty. The EXPLICIT-NEGATIVE direction is \
               a SEPARATE lane (sq-pbz04.4.16): the export's owl:NegativePropertyAssertion \
               profile negations refuted where L2 can (134 after sq-pbz04.4.8 moved four \
               built-in-property rows out of the checkable set into honest extraction \
               abstention), with an honest measured In-gap (181 of 315 checkable) where axiom-grammar membership \
               over the ALCH shadow cannot refute full-profile membership (deferred \
               restrictions); species assertions remain unchecked (documented)",
    },
    Suite {
        label: "OWL 2 Direct-Semantics consistency + entailment (scoped fragment)",
        family: "sparq extension",
        runner: Runner::FeatureGatedCrateTest {
            krate: "sparq-conformance",
            target: "dl_suite",
            feature: "dl-direct",
        },
        ci_job: "inference-conformance",
        ratchet_floor: 227,
        floor_basis: "definitive expected verdicts through the L4 dispatch, EXACT-pinned \
                      (sparq EXTENSION over the scoped fragment — NOT full OWL 2 DL); \
                      re-pinned by sq-pbz04.4.11 (M1 named-composite fix, net +8); \
                      re-pinned by sq-pbz04.4.12 (M4 orphan/cyclic fix: -3, 192 -> 189); \
                      re-pinned by sq-pbz04.4.13 (M2 conclusion-bnode existential-reading fix: \
                      +1, 189 -> 190); re-pinned by sq-pbz04.4.16 (M7 singleton-intersection \
                      normalization: -8, 190 -> 182 — 8 consistency cases re-route from the \
                      ALCH tableau to the RL branch and honestly abstain via the documented \
                      disjointWith divergence guard, never a wrong verdict; fail set unchanged); \
                      unchanged at 182 by sq-pbz04.4.9 (SubObjectPropertyOf conclusion encoding \
                      + L1 datatype-map-IRI refusal net to zero on the pass total, composition \
                      97+69 -> 96+70; fail set still 5, M3/M5/M6); re-pinned by sq-zfwzq \
                      (ALCHS transitive roles: +2 consistency and +2 positive-entailment \
                      passes, 182 -> 186; composition 98+14+72+2, fail set unchanged); \
                      re-pinned by sq-pbz04.4.8 (guard-abstention tableau fall-through: +41, \
                      186 -> 227; composition 136+17+72+2 — an abstaining RL/EL/QL branch now \
                      re-asks the ALCH tableau, which is complete for every L1-extracted \
                      ontology, so the 43 guard-abstained rows are decided instead of dropped; \
                      paired with an L1 refusal of owl:top/bottomObjectProperty, whose fixed \
                      extensions L1 had been reading away — without it the fall-through exposed \
                      2 wrong verdicts; fail set unchanged at 5)",
        note: "EXTENSION ratchet — the DIRECT-arm consistency / inconsistency / positive- / \
               negative-entailment tests decided by the REAL sparq-reason-dl L4 dispatch \
               (RL guarded / EL guarded / QL deferred / ALCH tableau) under a pinned \
               deterministic count budget; fail-closed abstentions are reported, never \
               passes, and all 5 remaining wrong-verdict divergences are pinned by name \
               with audited mechanisms (M1 FIXED sq-pbz04.4.11; M4 FIXED sq-pbz04.4.12; \
               M2 FIXED sq-pbz04.4.13; M7 FIXED sq-pbz04.4.16 — those rows now pass/abstain; \
               remaining: M3/M5/M6). The default-off dl_transitive extension graduates \
               transitive-role inputs only through the tableau whose termination and \
               soundness argument is written in sparq-reason-dl::tableau §5a",
    },
    // [FABLE-5] the UFO-SN3 finite-world expressibility ratchet (runner lives
    // crate-local in `sparq-conformance/tests/ufo_sn3_suite.rs`, UNGATED — it calls
    // plain `reason_n3`, links no opt-in code, and runs in ordinary
    // `cargo test --workspace`). HONESTLY tallied as a sparq EXTENSION ratchet, NOT
    // folded into the conformance total: UFO (Unified Foundational Ontology) is a
    // research foundational ontology with NO normative conformance test suite
    // (gUFO, its lightweight OWL implementation, ships no entailment corpus), so —
    // exactly like the BM25 / RSP / RIF-Core-expressivity rows — this lane runs
    // sparq's OWN reference profile: UFO-SN3, a finite-world, function-free,
    // range-restricted, monotone N3 projection of representative UFO-A/B/C
    // concepts (rigidity + identity criteria, relators, events/participation,
    // dispositions, commitments/norms, situations/worlds/accessibility, closed
    // validation). Each committed fixture case is concatenated with the committed
    // ruleset and driven through the REAL `reason_n3` forward closure; the oracle
    // is superset entailment of the case's answer.n3 (the eye_cases shape) PLUS
    // per-case negative-entailment guards (open-world absence is never falsity;
    // anti-rigid memberships do not propagate; `ufo:sameContinuant` never becomes
    // `owl:sameAs`; the reification-node projection never asserts the encoded
    // triple — the honest stand-in for RDF 1.2 triple-term matching the N3
    // engine's Term model lacks, a tracked feature gap, never faked). The floor is
    // the MEASURED assertion count (the `UFO-SN3 expressibility assertions N` line)
    // — it may only RISE; `UFO_SN3_FLOOR` is mirrored here and kept in lock-step by
    // `tests/scoreboard_floors.rs` (read textually).
    Suite {
        label: "UFO-SN3 finite-world expressibility",
        family: "sparq extension",
        runner: Runner::CrateTest { krate: "sparq-conformance", target: "ufo_sn3_suite" },
        ci_job: "test",
        ratchet_floor: 42,
        floor_basis: "expressibility assertions — answer-triple superset entailments + \
                      negative-entailment guards (sparq EXTENSION over the UFO-SN3 \
                      reference profile, NOT a UFO/gUFO/OntoUML standards-conformance \
                      claim)",
        note: "EXTENSION ratchet — no normative UFO/gUFO conformance suite exists: \
               sparq's own finite-world UFO-SN3 reference profile (a function-free, \
               range-restricted, monotone N3 projection of UFO-A/B/C rigidity, \
               identity, relators, events, dispositions, norms, and situations) \
               driven through the REAL reason_n3 closure over committed vocab + rules \
               + fixture cases, with per-case negative-entailment guards; the \
               reification-node projection stands in for RDF 1.2 triple-term \
               matching (a tracked sparq-reason gap), never faked as native support",
    },
    // [KERN] the RDF 1.2 quoted-triple OPACITY ratchet (runner lives crate-local
    // in `sparq-conformance/tests/quoted_triple_opacity.rs`, UNGATED — it calls
    // plain `sparq_reason::materialize` on committed Turtle 1.2 fixtures, links
    // no opt-in code, fetches no data, and runs in ordinary
    // `cargo test --workspace`; only its EL arm is behind the existing
    // `el-suite` feature and does NOT count toward the floor). HONESTLY tallied
    // as a sparq EXTENSION ratchet, NOT folded into the conformance total: the
    // fixtures are self-authored (the W3C rdf-tests 1.2 entailment corpus does
    // not yet cover reasoner-side opacity), but the property they pin is the
    // NORMATIVE RDF 1.2 semantics of triple terms — quoting never asserts: a
    // reified triple `<< s p o >>` (any surface form) entails neither `s p o`
    // nor any consequence of it; the RL closure of a base graph is
    // BYTE-IDENTICAL with or without quoted triples referring to it (pinned
    // against committed expected-answer files); and a reifier's own annotations
    // are reasoned over normally without leaking the quoted content. The
    // annotation form `s p o {| … |}` — which RDF 1.2 DOES assert — is the
    // in-fixture positive control proving the negative guards are meaningful.
    // The floor is the MEASURED assertion count (the `quoted-triple opacity
    // assertions N` line) — it may only RISE; `QUOTED_OPACITY_FLOOR` is
    // mirrored here and kept in lock-step by `tests/scoreboard_floors.rs`.
    Suite {
        label: "RDF 1.2 quoted-triple opacity (reasoning)",
        family: "sparq extension",
        runner: Runner::CrateTest {
            krate: "sparq-conformance",
            target: "quoted_triple_opacity",
        },
        ci_job: "test",
        ratchet_floor: 84,
        floor_basis: "opacity assertions — quoting-never-asserts negative-entailment guards, \
                      byte-identical closure non-interference vs committed expected-answer \
                      files, and normal reifier-annotation reasoning, per profile (RDFS + \
                      OWL 2 RL); self-authored fixtures pinning the normative RDF 1.2 \
                      triple-term semantics, NOT a W3C-suite pass count",
        note: "EXTENSION ratchet — RDF 1.2 quoted/reified triples are TERMS: the lane pins \
               that the REAL reasoning profiles never assert quoted triples (no entailment \
               of the quoted triple nor its domain/range/subproperty/subclass consequences), \
               that RL closures are byte-identical with or without quoted triples referring \
               to them, and that reifier annotations reason normally; the asserting \
               annotation form is the fixture's positive control; the feature-gated \
               EL arm re-checks non-interference through the sparq-reason-el classifier \
               without counting toward the floor",
    },
    // [FABLE-5] sq-tonhr.2 (epic sq-tonhr) — the W3C rdf-n-triples / rdf-n-quads /
    // rdf-trig SYNTAX-suite ratchets, wired BEFORE any rdf-shuttle generated candidate
    // parser lands so the incumbent bar is pinned (only rdf-turtle was ratcheted until
    // now). The runner is crate-local (`tests/rdf_line_syntax_ratchet.rs`), default-on
    // (no new deps — it drives the REAL default-feature ingest paths: the native
    // chunk-parallel `nt.rs` N-Triples parser, the chunk-parallel N-Quads dataset
    // loader, the with-base TriG dataset loader) and self-skips when the pinned
    // w3c/rdf-tests clone is not fetched; the `conformance` CI job fetches it
    // explicitly and runs the ratchet. Floors are the MEASURED pass counts at the
    // pinned revision — NT 60/70 and NQ 76/87 honestly record the native byte-level
    // parser's audited divergences (bead sq-w64x5: no IRI/blank-node-label/lang-tag
    // validation = 9+1 lenient accepts of negative cases, plus one over-strict reject
    // of `minimal_whitespace`; the companion differential gate
    // `tests/parser_differential.rs` pins the SAME cases as an exact adjudicated set),
    // TriG passes all 356. Floors may only RISE (fixing sq-w64x5 raises NT/NQ).
    Suite {
        label: "W3C N-Triples syntax (rdf11 rdf-n-triples)",
        family: "W3C RDF",
        runner: Runner::CrateTest { krate: "sparq-conformance", target: "rdf_line_syntax_ratchet" },
        ci_job: "conformance",
        ratchet_floor: 60,
        floor_basis: "pass",
        note: "positive+negative syntax through the REAL native chunk-parallel nt.rs \
               path; the 10 recorded FAILs are the audited sq-w64x5 validation \
               divergences, never summed in",
    },
    Suite {
        label: "W3C N-Quads syntax (rdf11 rdf-n-quads)",
        family: "W3C RDF",
        runner: Runner::CrateTest { krate: "sparq-conformance", target: "rdf_line_syntax_ratchet" },
        ci_job: "conformance",
        ratchet_floor: 76,
        floor_basis: "pass",
        note: "positive+negative syntax through the REAL chunk-parallel N-Quads dataset \
               loader (named graphs preserved); the 11 recorded FAILs are the shared \
               nt.rs sq-w64x5 divergences plus the graph-position IRI case",
    },
    Suite {
        label: "W3C TriG syntax + eval (rdf11 rdf-trig)",
        family: "W3C RDF",
        runner: Runner::CrateTest { krate: "sparq-conformance", target: "rdf_line_syntax_ratchet" },
        ci_job: "conformance",
        ratchet_floor: 356,
        floor_basis: "pass",
        note: "positive+negative syntax AND eval (quad-SET blank-node-bijection identity \
               to the N-Quads expectation, graph names included) through the with-base \
               TriG dataset loader — all 356 manifest entries pass",
    },
];

/// Render the registry as one markdown scoreboard. This is a STATIC view of what
/// the project ratchets and where each suite is enforced — it does not re-run the
/// suites (each runner owns its own data fetch + execution), so it is hermetic and
/// fast, and serves as the single index CI surfaces alongside the per-suite
/// reports.
pub fn render_scoreboard() -> String {
    use std::fmt::Write;
    let mut md = String::new();
    let _ = writeln!(md, "# sparq conformance scoreboard\n");
    let _ = writeln!(
        md,
        "The single index of EVERY conformance suite sparq ratchets, across crates. \
         Each suite has a FLOOR that CI enforces: a pass-count (or pass+divergence) \
         floor that may only RISE, or — for the differential oracles — a hard \
         divergence-count floor of 0. The per-suite detail reports are produced by the \
         runners in the *run* column; this table is the consolidated map (sq-ncvq.16 \
         brought the SHACL + GeoSPARQL ratchets in; sq-j174 the Solid WAC + ACP \
         decision-parity ones; sq-t58w.8 the Solid WAC + ACP differential oracles — \
         all previously lived outside this scoreboard).\n\n\
         The `family = sparq extension` rows are HONESTLY sparq-extension ratchets, \
         NOT standards-conformance claims — there is no normative body or test suite \
         behind them (sq-ripcg's BM25 differential oracle: no full-text-over-RDF / BM25 \
         standard exists; sq-mcb3q's RSP expressivity / SRBench correctness oracle: no \
         normative RDF-Stream-Processing standard exists — RSP-QL is a W3C-community \
         spec and SRBench a benchmark). Their floors are in a different UNIT \
         (per-window / per-hit correctness assertions, not spec pass-counts), so the \
         consolidated total below counts ONLY the standards-conformance suites; the \
         extension rows are reported separately.\n"
    );
    let _ = writeln!(
        md,
        "| suite | family | floor | basis | CI job | run |"
    );
    let _ = writeln!(md, "|---|---|---:|---|---|---|");
    for s in SUITES {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | `{}` | `{}` |",
            s.label,
            s.family,
            s.ratchet_floor,
            s.floor_basis,
            s.ci_job,
            s.runner.command(),
        );
    }
    // The consolidated total counts ONLY standards-conformance suites — the
    // `sparq extension` rows (sq-ripcg) carry a different-unit floor (score-exact
    // assertions, not spec pass-counts), so summing them in would inflate a
    // "conformance" number with a non-conformance one. They are reported on their
    // own line. [OPUS-4.8] sq-ripcg
    let is_extension = |s: &&Suite| s.family == "sparq extension";
    let conformance: Vec<&Suite> = SUITES.iter().filter(|s| !is_extension(s)).collect();
    let extensions: Vec<&Suite> = SUITES.iter().filter(is_extension).collect();
    let total: usize = conformance.iter().map(|s| s.ratchet_floor).sum();
    let _ = writeln!(
        md,
        "| **conformance total ({} suites)** | | **{}** | | | |",
        conformance.len(),
        total
    );
    if !extensions.is_empty() {
        let ext_total: usize = extensions.iter().map(|s| s.ratchet_floor).sum();
        // [OPUS-4.8] sq-mcb3q: pluralise now there is more than one extension row.
        let row_word = if extensions.len() == 1 { "row" } else { "rows" };
        let _ = writeln!(
            md,
            "| **sparq-extension ({} {}, NOT conformance)** | | {} | (assertions) | | |\n",
            extensions.len(),
            row_word,
            ext_total
        );
    } else {
        let _ = writeln!(md);
    }
    let _ = writeln!(
        md,
        "Notes:\n\n{}",
        SUITES
            .iter()
            .map(|s| format!("- **{}**: {}", s.label, s.note))
            .collect::<Vec<_>>()
            .join("\n")
    );
    md
}

/// Render the registry as the MACHINE-READABLE JSON scoreboard — the exact
/// content of the committed `bench/conformance-scoreboard.generated.json`
/// artifact (sq-gum8.14), so conformance-class paper-evidence bindings can
/// reference suite rows / ratchet floors by json-pointer instead of a Rust
/// source anchor.
///
/// A pure DERIVATION of [`SUITES`]: same rows, same floors, same totals-split
/// (standards-conformance vs the `sparq extension` rows, which are NEVER summed
/// into the conformance total — the same honesty rule [`render_scoreboard`]
/// applies). Deliberately DETERMINISTIC: no timestamps, commit hashes, or other
/// provenance, and every object's keys are emitted in sorted (alphabetical)
/// order, so regenerating at the same source state is byte-identical — the
/// property the drift-guard test (`tests/scoreboard_export.rs`) enforces by
/// byte-comparing the committed artifact against a fresh render.
///
/// Regenerate the committed artifact from the repo root with:
///
/// ```text
/// cargo run -p sparq-conformance --bin sparq-conformance-scoreboard -- \
///   --report /tmp/conformance-scoreboard.md \
///   --json bench/conformance-scoreboard.generated.json
/// ```
pub fn scoreboard_json() -> String {
    use serde_json::{json, Value};

    // NB: every `json!` object below writes its keys in ALPHABETICAL order on
    // purpose. serde_json's default map is a BTreeMap (sorted keys), but its
    // opt-in `preserve_order` feature — which cargo feature-unification could
    // switch on from anywhere in a build graph — preserves insertion order
    // instead. Alphabetical insertion makes the two modes byte-identical, so
    // the committed artifact can never flap on an unrelated dependency change.
    let runner_json = |r: Runner| -> Value {
        match r {
            Runner::SparqlBinary => json!({ "kind": "sparql-binary" }),
            Runner::InferenceBinary => json!({ "kind": "inference-binary" }),
            Runner::CrateTest { krate, target } => json!({
                "crate": krate,
                "kind": "crate-test",
                "target": target,
            }),
            Runner::FeatureGatedCrateTest { krate, target, feature } => json!({
                "crate": krate,
                "feature": feature,
                "kind": "feature-gated-crate-test",
                "target": target,
            }),
        }
    };

    let is_extension = |s: &Suite| s.family == "sparq extension";
    let suites: Vec<Value> = SUITES
        .iter()
        .map(|s| {
            json!({
                "ci_job": s.ci_job,
                "command": s.runner.command(),
                "family": s.family,
                "floor_basis": s.floor_basis,
                "is_extension": is_extension(s),
                "label": s.label,
                "note": s.note,
                "ratchet_floor": s.ratchet_floor,
                "runner": runner_json(s.runner),
            })
        })
        .collect();

    let conformance_floor_total: usize =
        SUITES.iter().filter(|s| !is_extension(s)).map(|s| s.ratchet_floor).sum();
    let conformance_suites = SUITES.iter().filter(|s| !is_extension(s)).count();
    let extension_assertion_total: usize =
        SUITES.iter().filter(|s| is_extension(s)).map(|s| s.ratchet_floor).sum();
    let extension_suites = SUITES.iter().filter(|s| is_extension(s)).count();

    let doc = json!({
        "description": "Machine-readable derivation of the central conformance \
                        registry (crates/sparq-conformance/src/scoreboard.rs \
                        scoreboard::SUITES): every conformance suite sparq \
                        ratchets, with its ratchet floor (may only RISE), floor \
                        basis, CI job, and runner command. Rows with \
                        is_extension=true are HONESTLY sparq-extension ratchets, \
                        NOT standards-conformance claims — their floors are in a \
                        different unit (assertions, not spec pass-counts) and are \
                        NEVER summed into totals.conformance_floor_total. \
                        Committed as bench/conformance-scoreboard.generated.json \
                        and drift-guarded: tests/scoreboard_export.rs regenerates \
                        and byte-compares, so this mirror cannot silently drift \
                        from the Rust source of truth.",
        "schema": "sparq.conformance-scoreboard/v1",
        "suites": suites,
        "title": "sparq conformance scoreboard",
        "totals": {
            "conformance_floor_total": conformance_floor_total,
            "conformance_suites": conformance_suites,
            "extension_assertion_total": extension_assertion_total,
            "extension_suites": extension_suites,
        },
    });
    let mut out = serde_json::to_string_pretty(&doc).expect("scoreboard JSON serialises");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct unit test for [`scoreboard_json`]: the export is valid JSON,
    /// carries EVERY registry row verbatim (label / floor / command, in
    /// registry order), and its totals reproduce an independent fold over
    /// [`SUITES`] with the extension rows split out — the same honesty rule
    /// `render_scoreboard` applies.
    #[test]
    fn scoreboard_json_mirrors_the_registry() {
        let out = scoreboard_json();
        assert!(out.ends_with('\n'), "artifact ends with a trailing newline");

        let doc: serde_json::Value = serde_json::from_str(&out).expect("export is valid JSON");
        assert_eq!(doc["schema"], "sparq.conformance-scoreboard/v1");

        let rows = doc["suites"].as_array().expect("suites is an array");
        assert_eq!(rows.len(), SUITES.len(), "one JSON row per registry row");
        for (row, s) in rows.iter().zip(SUITES) {
            assert_eq!(row["label"], s.label);
            assert_eq!(row["family"], s.family);
            assert_eq!(row["ci_job"], s.ci_job);
            assert_eq!(row["floor_basis"], s.floor_basis);
            assert_eq!(row["note"], s.note);
            assert_eq!(row["command"], s.runner.command());
            assert_eq!(
                row["ratchet_floor"].as_u64().expect("floor is an integer") as usize,
                s.ratchet_floor
            );
            assert_eq!(
                row["is_extension"].as_bool().expect("is_extension is a bool"),
                s.family == "sparq extension"
            );
        }

        // Totals: independent fold, extension rows never summed into the
        // conformance total.
        let ext: usize = SUITES
            .iter()
            .filter(|s| s.family == "sparq extension")
            .map(|s| s.ratchet_floor)
            .sum();
        let conf: usize = SUITES
            .iter()
            .filter(|s| s.family != "sparq extension")
            .map(|s| s.ratchet_floor)
            .sum();
        let totals = &doc["totals"];
        assert_eq!(totals["conformance_floor_total"].as_u64().unwrap() as usize, conf);
        assert_eq!(totals["extension_assertion_total"].as_u64().unwrap() as usize, ext);
        assert_eq!(
            totals["conformance_suites"].as_u64().unwrap() as usize
                + totals["extension_suites"].as_u64().unwrap() as usize,
            SUITES.len()
        );
    }
}

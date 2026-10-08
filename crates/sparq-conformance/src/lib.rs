//! Shared machinery for the W3C conformance harnesses: manifest walking, RDF
//! plumbing, result parsing/comparison, and per-test execution. Three binaries
//! sit on top:
//!
//! - `sparq-conformance` (src/main.rs) — the W3C SPARQL suites (query/update
//!   evaluation + syntax), gated in CI by a pass-count ratchet.
//! - `sparq-inference-conformance` (src/bin/inference.rs) — the reasoning
//!   suites (RDF Semantics rdf-mt, OWL 2 RL, N3, SPARQL entailment regimes)
//!   run against `sparq-reason`.
//! - `sparq-conformance-scoreboard` (src/bin/scoreboard.rs) — [OPUS-4.8]
//!   sq-ncvq.16: the CONSOLIDATED index of EVERY conformance ratchet across the
//!   workspace (the two binaries above PLUS the crate-local W3C SHACL and OGC
//!   GeoSPARQL ratchets), rendered from the central `scoreboard` registry.
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

pub mod compare;
pub mod inference;
pub mod manifest;
pub mod rdf;
pub mod results;
pub mod run;
// [OPUS-4.8] sq-ncvq.16 — the CENTRAL conformance scoreboard registry: every
// ratchet (W3C SPARQL, inference, W3C SHACL, OGC GeoSPARQL) in one place. The
// `sparq-conformance-scoreboard` binary renders it; a guard test keeps the
// crate-local SHACL/geo floors in sync.
pub mod scoreboard;
// [FABLE-5] sq-oy1f.40 — the LIB-SIDE single source of truth for the six W3C
// JSON-LD 1.1 conformance-lane ratchet floors. The `jsonld_suite` test binary's
// `assert!(pass >= FLOOR)` AND `scoreboard::SUITES`' `ratchet_floor` both read the
// SAME `const` at compile time, so they cannot drift (kills the #1463 floor-drift
// class structurally). `ci.yml`'s `jsonld-conformance` job greps each floor from
// `src/floors/<lane>.rs`, binding the CI grep gate to the same single source.
pub mod floors;
// [FABLE-5] sq-hmd7l.15 — shared JSON-LD comparison helpers (the document-level
// `json_ld_equal` comparator + the canonical-dataset bridges), used by BOTH the
// `tests/jsonld_suite/` conformance lanes and the `bench/jsonld` comparative
// harness (the `bench_jsonld` example), so the bench's output-equality gate is
// the SAME comparator the conformance ratchet trusts. Behind the OPT-IN
// `jsonld-suite` feature (optional `sparq-jsonld` / `oxjsonld` deps),
// `#[cfg]`-stripped from the default build — the lean opt-in posture.
#[cfg(feature = "jsonld-suite")]
pub mod jsonld_bench;
// [OPUS-4.8] sq-ushvx (epic sq-my8wd) — the in-process SERVICE-federation test harness.
// A reusable fixture that stands up a REAL `sparq_server::serve` endpoint on an ephemeral
// `127.0.0.1:0` loopback port and drives a federated SERVICE query through the engine's
// REAL `ureq` transport. Behind the OPT-IN `service-loopback` feature (tokio + axum +
// ureq), `#[cfg]`-stripped from the default build. The federation keystone the later
// `sparql11/service` + HTTP-protocol conformance lanes (sq-my8wd) wire onto.
#[cfg(feature = "service-loopback")]
pub mod service_loopback;
// [OPUS-4.8] sq-ddpgx (epic sq-my8wd) — the W3C `sparql11/service` EVALUATION
// conformance lane, driven through the loopback harness above. For each manifest
// test it stands up a REAL loopback endpoint per `qt:serviceData` block, rewrites
// the well-known endpoint IRIs to the bound loopback URLs, runs the federated
// query through the engine's REAL `ureq` transport, and compares to the `.srx`
// oracle. Behind the same opt-in `service-loopback` deps (the crate `service`
// feature forwards to it); the gating runner is `tests/service_eval_suite.rs`.
#[cfg(feature = "service-loopback")]
pub mod service_eval;
// [OPUS-4.8] sq-jaj38 (epic sq-my8wd) — the W3C SPARQL 1.1 **Protocol** (HTTP layer)
// conformance lane. Drives raw HTTP requests (GET/POST query+update, the QUERY method,
// dataset overrides, result-format content negotiation, the documented status codes)
// DIRECTLY at the in-process loopback server's bound `127.0.0.1:0` port — exercising the
// SPARQL Protocol request/response contract, not the federated SERVICE transport. Behind the
// same opt-in `service-loopback` deps (the crate `http-protocol` feature forwards to it); the
// gating runner is `tests/http_protocol_suite.rs`. Connects a plain std `TcpStream` to the
// loopback port (no engine egress involved), so it does not widen the SSRF allowlist.
#[cfg(feature = "service-loopback")]
pub mod http_protocol;
// [OPUS-4.8] sq-1uuxz (epic sq-my8wd) — the SPARQL 1.1 **Service-Description** (SD) +
// **Graph-Store Protocol** (GSP) conformance lane. Drives raw HTTP requests DIRECTLY at the
// in-process loopback server's bound `127.0.0.1:0` port — (A) fetching the `GET /sparql` (no
// query) Service-Description document and asserting it advertises exactly the formats / languages
// / versions / features the server genuinely implements (no over-advertising), and (B) a full
// GET/PUT/POST/DELETE Graph-Store-Protocol round-trip on a named graph (indirect `?graph=` + direct
// `/graphs/<path>`) and the default graph (`?default`), verifying store state after each op. Behind
// the opt-in `federation-descriptors` feature (forwards to `service-loopback` for the loopback
// harness/HTTP client AND turns on `sparq-server/federation-descriptors` so the SD endpoint is
// live); the gating runner is `tests/sd_gsp_suite.rs`. Connects a plain std `TcpStream` to the
// loopback port (no engine egress involved), so it does not widen the SSRF allowlist.
#[cfg(feature = "federation-descriptors")]
pub mod sd_gsp;
// [OPUS-4.8] (B4) W3C rdf-turtle suite run THROUGH the sparq Turtle parser
// (`Graph::parse_to_triples`) — the rejection/acceptance oracle for the Turtle T1 spike,
// distinct from the oxttl-differential chunked-vs-serial test and the N3-parser TurtleTests.
pub mod turtle_suite;
// [FABLE-5] (sq-tonhr.2, epic sq-tonhr) The W3C rdf-n-triples / rdf-n-quads / rdf-trig
// syntax suites run THROUGH the real sparq parse paths (native nt.rs N-Triples, the
// chunk-parallel N-Quads dataset loader, the with-base TriG dataset loader) — pins the
// bar every rdf-shuttle generated candidate parser is differential-gated against.
// Gating runner: `tests/rdf_line_syntax_ratchet.rs`.
pub mod line_syntax;
// [FABLE-5] (sq-tonhr.2) Shared quad-set rendering + blank-node-bijection set comparison
// for the line-syntax suites and the differential harness (never line-by-line identity).
pub mod quadset;
// [FABLE-5] (sq-tonhr.2) The reusable candidate-vs-incumbent parser DIFFERENTIAL harness
// (identical input -> identical accept/reject verdict + quad set) over any W3C suite's
// actions or any corpus directory (e.g. the committed fuzz seeds), with minimal-repro
// line shrinking — the epic's zero-regression gate for generated parsers. Non-vacuity is
// proven by seeded mutants in `tests/parser_differential.rs`.
pub mod differential;
// Issue #6467: runner logic for the community notation3tests suite (codeberg
// phochste/notation3tests, fetched at run time) — binary `sparq-notation3tests`,
// advisory lane `.github/workflows/notation3tests.yml`.
pub mod notation3tests;

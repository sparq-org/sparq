//! The inference/reasoning conformance suites (run by the
//! `sparq-inference-conformance` binary):
//!
//! 1. **RDF Semantics** (`rdfmt`) — w3c/rdf-tests `rdf/rdf11/rdf-mt`: simple /
//!    RDF / RDFS entailment + datatype (D) entailment and inconsistency.
//! 2. **OWL 2 RL** (`owl_suite`) — the W3C OWL WG test-case export, filtered
//!    to RDF-based-semantics tests applicable to the RL profile, run through
//!    `sparq_reason::materialize_owl_rl` + `inconsistencies`.
//! 3. **N3** (`n3_suite`) — the w3c/N3 community-group manifests (reasoner +
//!    parser + extended), run through the `sparq_reason::n3` engine.
//! 4. **SPARQL entailment regimes** (`sparql_entail`) — `sparql11/entailment`,
//!    evaluated as query-over-materialized-closure.
//!
//! Shared machinery: `entail` (regime closures, the bnode-homomorphism
//! entailment check, D-inconsistency), `report` (the honest four-bucket
//! scoreboard: pass / fail / documented divergence / out-of-scope-with-reason).

pub mod entail;
// sq-pbz04.4.5 — the OWL 2 Direct-Semantics arm of the OWL WG export
// (`sparq-reason-dl` L2 profile checker + L4 dispatch checker), compiled ONLY under the
// opt-in `dl-direct` feature so the default build links zero Direct-Semantics code. NOT
// run by the inference binary: its floors are a sparq EXTENSION ratchet (scoped fragment
// — NOT full OWL 2 DL), gated by the crate-local `tests/dl_suite.rs`.
#[cfg(feature = "dl-direct")]
pub mod dl_suite;
pub mod n3_suite;
pub mod owl_suite;
pub mod rdfmt;
pub mod report;
pub mod sparql_entail;

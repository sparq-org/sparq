//! [SONNET-4.6] sq-z1xv8 — the **shared ratchet-floor constants** for every
//! conformance / sparq-extension ratchet whose enforcing test runner lives in a
//! DIFFERENT crate from the central scoreboard that reports it.
//!
//! # Why this crate exists
//!
//! `sparq-conformance`'s [`scoreboard::SUITES`] registry is the single index of
//! every ratchet in the workspace, but the dev-only harness deliberately takes no
//! dependency on the crates it indexes (`sparq-shacl`, `sparq-geo`,
//! `sparq-solid`, `sparq-policy`, `sparq-text`, `sparq-rsp`). So each floor used
//! to be spelled TWICE — once as a `const` in the runner's `tests/*.rs`, once as
//! a plain `usize` in the registry row — and kept in step by
//! `sparq-conformance/tests/scoreboard_floors.rs` READING the sibling crate's
//! test source at a runtime-computed workspace path.
//!
//! That textual re-read was also a **CI test-selection soundness residual**
//! (`scripts/ci_audit_inputs.py`, design `research/change-based-test-selection.md`
//! §4.2 "residual 3"): the read escapes the reading crate's directory, and
//! because the path is assembled at runtime it is statically unresolvable, so
//! neither a reverse-dependency closure nor a `ci/path-ownership.toml`
//! `readers = [...]` entry could attribute it. A change to `sparq-solid`'s floor
//! could therefore skip the very test lane that reads it, caught only by the
//! nightly full run.
//!
//! Moving the numbers HERE fixes both at once:
//!
//! * **No drift.** The runner's `assert!(pass >= FLOOR)` and the registry's
//!   `ratchet_floor` read the SAME `const`. There is one number, so there is
//!   nothing to keep in sync — the same guarantee the six JSON-LD lanes already
//!   get from `sparq_conformance::floors::<lane>::FLOOR`.
//! * **No out-of-crate read.** The guard test no longer reads foreign sources, so
//!   residual 3 is closed rather than acknowledged. The cargo edges (a
//!   `dev-dependency` from each runner crate, a plain dependency from
//!   `sparq-conformance`) put every affected crate in this crate's
//!   reverse-dependency closure, which is exactly what the selector needs to
//!   prove it may not skip them.
//!
//! # The ratchet rule
//!
//! Every constant below is the **actual measured** pass / scenario / assertion
//! count at the pinned suite revision — not an aspirational target — and may only
//! **RISE**. Never lower one. The measurement narrative (what was added, which
//! bead raised it, which cases are honestly excluded) stays with the runner that
//! performs the measurement; this crate holds the value plus a one-line pointer
//! to its enforcer. A silent LOWERING is additionally caught by
//! `sparq-conformance/tests/scoreboard_floors.rs`
//! (`SHARED_CRATE_EXPECTED`), which pins each value the same way
//! `LIB_SOURCED_EXPECTED` pins the JSON-LD lanes.
//!
//! [`scoreboard::SUITES`]: https://github.com/sparq-org/sparq/blob/main/crates/sparq-conformance/src/scoreboard.rs

/// W3C SHACL ratchets — enforced by `sparq-shacl`'s `tests/w3c_core.rs` and
/// `tests/w3c_sparql.rs` (CI job `shacl-conformance`).
pub mod shacl {
    /// Pass floor for the `sht:Validate` **core** suite (`w3c/data-shapes`).
    /// Enforced as `BASELINE_PASS` in `sparq-shacl/tests/w3c_core.rs`.
    pub const CORE_FLOOR: usize = 98;

    /// Pass floor for the `sh:sparql` node + property constraint sub-suites.
    /// Enforced as `SHACL_SPARQL_FLOOR` in `sparq-shacl/tests/w3c_sparql.rs`.
    pub const SPARQL_FLOOR: usize = 5;
}

/// OGC GeoSPARQL ratchets — enforced by `sparq-geo`'s
/// `tests/ogc_compliance_ratchet.rs` and `tests/ogc_query_rewrite_ratchet.rs`
/// (CI job `geo-conformance`).
pub mod geo {
    /// Pass floor for the hand-derived DE-9IM / simple-features topology
    /// assertions. Enforced as `OGC_RATCHET_FLOOR` in
    /// `sparq-geo/tests/ogc_compliance_ratchet.rs`, which documents the raises.
    pub const OGC_TOPOLOGY_FLOOR: usize = 197;

    /// Pass floor for the topology PROPERTY forms answered end-to-end through the
    /// opt-in query-rewrite extension (`geosparql_rewrite` feature). Enforced as
    /// `OGC_QUERY_REWRITE_FLOOR` in
    /// `sparq-geo/tests/ogc_query_rewrite_ratchet.rs`.
    pub const OGC_QUERY_REWRITE_FLOOR: usize = 48;
}

/// Solid WAC / ACP ratchets — enforced by `sparq-solid`'s
/// `tests/common/mod.rs` (the shared parity corpus, read by both
/// `conformance_wac` and `conformance_acp`) and `tests/differential_oracle.rs`
/// (CI job `solid-conformance`).
pub mod solid {
    /// Scenario-count floor for the minimal per-construct WAC `.acl` corpus.
    /// Enforced as `WAC_SCENARIO_FLOOR` in `sparq-solid/tests/common/mod.rs`.
    pub const WAC_SCENARIO_FLOOR: usize = 13;

    /// Scenario-count floor for the minimal per-construct ACP ACR corpus.
    /// Enforced as `ACP_SCENARIO_FLOOR` in `sparq-solid/tests/common/mod.rs`.
    pub const ACP_SCENARIO_FLOOR: usize = 13;

    /// Divergence-count floor for the WAC + ACP differential oracles (engine vs
    /// an independent reference evaluator vs the hand `Expect` table). Unlike a
    /// pass count this is NOT a rising ratchet: the only acceptable divergence
    /// count is **zero**, so the floor is the hard `0`. Enforced as
    /// `DIVERGENCE_FLOOR` in `sparq-solid/tests/differential_oracle.rs`.
    pub const DIVERGENCE_FLOOR: usize = 0;
}

/// SolidLab ODRL Test Suite ratchet — enforced by `sparq-policy`'s
/// `tests/odrl_test_suite.rs` (CI job `odrl-conformance`).
pub mod policy {
    /// Pass floor for the SolidLab self-describing ODRL cases driven through the
    /// real `parse_policy_str` + `evaluate` path. Enforced as `ODRL_SUITE_FLOOR`
    /// in `sparq-policy/tests/odrl_test_suite.rs`, which documents the raises and
    /// the one remaining documented semantic divergence.
    pub const ODRL_SUITE_FLOOR: usize = 67;
}

/// sparq-text BM25 differential-oracle ratchet — enforced by `sparq-text`'s
/// `tests/bm25_oracle.rs` (CI job `text-oracle`). HONESTLY a sparq EXTENSION
/// ratchet: no normative full-text-over-RDF / BM25 conformance suite exists.
pub mod text {
    /// Floor for the number of score-exact differential assertions the fixed
    /// query battery makes against the independent reference BM25 scorer.
    /// Enforced as `TEXT_ORACLE_FLOOR` in `sparq-text/tests/bm25_oracle.rs`.
    pub const BM25_ORACLE_FLOOR: usize = 18750;
}

/// sparq-rsp RSP-expressivity / SRBench-correctness ratchet — enforced by
/// `sparq-rsp`'s `tests/srbench_oracle.rs` (CI job `rsp-oracle`). HONESTLY a
/// sparq EXTENSION ratchet: RSP-QL is a W3C-community spec and SRBench a
/// benchmark, so no normative RSP conformance suite exists.
pub mod rsp {
    /// Floor for the per-window correctness assertions the oracle makes.
    /// Enforced as `RSP_EXPRESSIVITY_FLOOR` in
    /// `sparq-rsp/tests/srbench_oracle.rs`, which documents the raises.
    pub const EXPRESSIVITY_FLOOR: usize = 317;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A **pass/scenario/assertion-count** floor of zero would be vacuous — the
    /// runner's `assert!(count >= FLOOR)` could never fail — so every counting
    /// ratchet here must be strictly positive. (The divergence floor below is the
    /// deliberate exception and is asserted separately.)
    #[test]
    fn every_counting_floor_is_non_vacuous() {
        for (name, floor) in [
            ("shacl::CORE_FLOOR", shacl::CORE_FLOOR),
            ("shacl::SPARQL_FLOOR", shacl::SPARQL_FLOOR),
            ("geo::OGC_TOPOLOGY_FLOOR", geo::OGC_TOPOLOGY_FLOOR),
            ("geo::OGC_QUERY_REWRITE_FLOOR", geo::OGC_QUERY_REWRITE_FLOOR),
            ("solid::WAC_SCENARIO_FLOOR", solid::WAC_SCENARIO_FLOOR),
            ("solid::ACP_SCENARIO_FLOOR", solid::ACP_SCENARIO_FLOOR),
            ("policy::ODRL_SUITE_FLOOR", policy::ODRL_SUITE_FLOOR),
            ("text::BM25_ORACLE_FLOOR", text::BM25_ORACLE_FLOOR),
            ("rsp::EXPRESSIVITY_FLOOR", rsp::EXPRESSIVITY_FLOOR),
        ] {
            assert!(floor > 0, "counting ratchet {} must be > 0, got {}", name, floor);
        }
    }

    /// The differential-oracle floor is a DIVERGENCE count: the only acceptable
    /// value is zero. Raising it would silently license a known-wrong decision,
    /// so pin it exactly rather than leaving it to the rising-ratchet rule.
    #[test]
    fn divergence_floor_is_exactly_zero() {
        assert_eq!(
            solid::DIVERGENCE_FLOOR, 0,
            "a divergence-count floor above zero licenses a known engine/reference \
             disagreement — the WAC/ACP differential oracles must stay at 0"
        );
    }
}

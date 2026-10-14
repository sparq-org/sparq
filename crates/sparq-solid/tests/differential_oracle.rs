//! sq-t58w.7 — THE differential oracle: a three-way agreement check proving the
//! engine's WAC/ACP authorization decisions match an INDEPENDENT reference evaluator AND the
//! hand-written `Expect` decision table, with **zero divergence**.
//!
//! Design record: `research/solid-acp-differential-oracle-design.md` §3-4. The oracle runs
//! the SHARED parity corpus (`tests/common/{wac,acp}.rs` from sq-t58w.6 — `wac_corpus()` /
//! `acp_corpus()`, the IDENTICAL scenarios `conformance_{wac,acp}.rs` assert) through THREE
//! deciders for every `(agent, client, mode, resource)` request in every scenario's table:
//!
//! 1. **the engine** — `materialize_{wac,acp}` + [`AuthIndex::accessible`] (the N3-rules
//!    paradigm), exactly as the conformance suites drive it;
//! 2. **the reference evaluator** — `tests/reference/{wac,acp}.rs`, a from-scratch
//!    PROCEDURAL reading of the spec over a hand-parsed model (a DIFFERENT paradigm — it
//!    shares no code with `materialize.rs`/`rules/*.n3`, so a shared bug cannot hide in
//!    both);
//! 3. **the hand `Expect` table** — the human-authored expected decision baked into each
//!    corpus scenario.
//!
//! A **divergence** is recorded whenever any two of the three disagree on a request. An
//! unclassifiable or erroring request (a corpus that fails to load/materialize, or a
//! reference parse failure) is itself counted as a divergence — **fail-closed**. The oracle
//! asserts `divergences == 0` and prints, in the SHACL/geo runner shape:
//!
//! ```text
//! WAC differential pairs <N> / divergences 0 (floor 0)
//! ACP differential pairs <N> / divergences 0 (floor 0)
//! ```
//!
//! so a later CI ratchet (`sq-t58w.3`) can grep it. Constraints honoured: no JS toolchain,
//! no network, no clock, no Docker — pure in-crate Rust.

mod common;
mod reference;

use common::{acp_corpus, wac_corpus};
use reference::{RefDecision, RefMode};
use sparq_core::Graph;
use sparq_solid::conformance::{AcpScenario, Decision, Expect};
use sparq_solid::wac_conformance::WacScenario;
use sparq_solid::{materialize_acp, materialize_wac, AuthIndex, Mode, Session};

/// The floor for the divergence count — it is and must stay ZERO. A ratchet that only ever
/// goes UP would be meaningless for a divergence count (the only acceptable value is 0), so
/// the "floor" printed for grep-parity with the SHACL/geo runners is the hard 0.
/// sq-z1xv8 — the VALUE now lives once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floor and the reported floor are
/// ONE `const` and cannot drift (replacing the old textual re-read of this file).
/// Raise it THERE; the measurement narrative stays here.
const DIVERGENCE_FLOOR: usize = sparq_conformance_floors::solid::DIVERGENCE_FLOOR;

/// Map the engine's [`Mode`] to the reference evaluator's independent [`RefMode`].
fn ref_mode(mode: Mode) -> RefMode {
    match mode {
        Mode::Read => RefMode::Read,
        Mode::Write => RefMode::Write,
        Mode::Append => RefMode::Append,
        Mode::Control => RefMode::Control,
    }
}

/// Map a reference [`RefDecision`] to the shared [`Decision`] vocabulary so all three
/// deciders are compared in one type.
fn from_ref(d: RefDecision) -> Decision {
    match d {
        RefDecision::Allow => Decision::Allow,
        RefDecision::Deny => Decision::Deny,
    }
}

/// The engine's verdict for one request over a materialized graph (the exact path
/// `WacScenario::run`/`AcpScenario::run` use): the resource is in the session's accessible
/// set for the mode ⇒ Allow.
fn engine_decide(index: &AuthIndex, e: &Expect) -> Decision {
    let session = Session {
        agent: e.req_agent(),
        client: e.req_client(),
        issuer: None,
        now: None,
    };
    let granted = index
        .accessible(&session, e.req_mode())
        .iter()
        .any(|g| g.as_str() == e.req_resource());
    if granted {
        Decision::Allow
    } else {
        Decision::Deny
    }
}

/// One recorded divergence: which scenario + request, and the three deciders' verdicts (an
/// `Err` string when a decider could not classify the request — counted fail-closed).
#[derive(Debug)]
struct Divergence {
    scenario: String,
    request: String,
    engine: Result<Decision, String>,
    reference: Result<Decision, String>,
    expect: Decision,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |r: &Result<Decision, String>| match r {
            Ok(d) => d.to_string(),
            Err(e) => format!("ERROR({e})"),
        };
        write!(
            f,
            "  DIVERGENCE [{}] {}: engine={}, reference={}, expect={}",
            self.scenario,
            self.request,
            show(&self.engine),
            show(&self.reference),
            self.expect,
        )
    }
}

/// Render an `Expect` request for the divergence report.
fn request_label(e: &Expect) -> String {
    let who = match (e.req_agent(), e.req_client()) {
        (Some(a), Some(c)) => format!("(agent {}, client {})", a, c),
        (Some(a), None) => format!("agent {}", a),
        (None, _) => "anonymous".to_owned(),
    };
    format!("{} {:?} {}", who, e.req_mode(), e.req_resource())
}

/// Compare three deciders for one request and, if they do not all agree, return a recorded
/// [`Divergence`]. An `Err` from either real decider is a divergence on its own (fail-closed).
fn check_request(
    scenario: &str,
    e: &Expect,
    engine: Result<Decision, String>,
    reference: Result<Decision, String>,
) -> Option<Divergence> {
    let expect = e.decision();
    let agree = matches!((&engine, &reference), (Ok(en), Ok(rf)) if *en == expect && *rf == expect);
    if agree {
        None
    } else {
        Some(Divergence {
            scenario: scenario.to_owned(),
            request: request_label(e),
            engine,
            reference,
            expect,
        })
    }
}

/// Run the WAC corpus through the three deciders. Returns `(pairs, divergences)`.
fn run_wac(scenarios: &[WacScenario]) -> (usize, Vec<Divergence>) {
    let mut pairs = 0usize;
    let mut divergences = Vec::new();
    for scenario in scenarios {
        // Decider 1: the engine. Materialize ONCE per scenario (the conformance path).
        let engine_index = materialize_wac_index(scenario.nquads_str());
        for e in scenario.expects() {
            pairs += 1;
            let engine = engine_index
                .as_ref()
                .map(|ix| engine_decide(ix, e))
                .map_err(|err| err.clone());
            // Decider 2: the independent reference evaluator (fresh parse per request keeps
            // it dependency-free of the engine state; cheap on this corpus).
            let reference = reference::wac::decide(
                scenario.nquads_str(),
                &reference::wac::Request {
                    agent: e.req_agent(),
                    client: e.req_client(),
                    mode: ref_mode(e.req_mode()),
                    resource: e.req_resource(),
                },
            )
            .map(from_ref);
            if let Some(d) = check_request(scenario.name(), e, engine, reference) {
                divergences.push(d);
            }
        }
    }
    (pairs, divergences)
}

/// Run the ACP corpus through the three deciders. Returns `(pairs, divergences)`.
fn run_acp(scenarios: &[AcpScenario]) -> (usize, Vec<Divergence>) {
    let mut pairs = 0usize;
    let mut divergences = Vec::new();
    for scenario in scenarios {
        let engine_index = materialize_acp_index(scenario.nquads_str());
        for e in scenario.expects() {
            pairs += 1;
            let engine = engine_index
                .as_ref()
                .map(|ix| engine_decide(ix, e))
                .map_err(|err| err.clone());
            let reference = reference::acp::decide(
                scenario.nquads_str(),
                &reference::acp::Request {
                    agent: e.req_agent(),
                    client: e.req_client(),
                    mode: ref_mode(e.req_mode()),
                    resource: e.req_resource(),
                },
            )
            .map(from_ref);
            if let Some(d) = check_request(scenario.name(), e, engine, reference) {
                divergences.push(d);
            }
        }
    }
    (pairs, divergences)
}

/// Materialize the WAC engine's auth index for a scenario's N-Quads (the engine decider's
/// setup). An `Err` here makes every request in the scenario diverge (fail-closed).
fn materialize_wac_index(nquads: &str) -> Result<AuthIndex, String> {
    let mut graph = Graph::load_dataset(nquads, "nquads").map_err(|e| format!("load: {}", e))?;
    materialize_wac(&mut graph).map_err(|e| format!("materialize_wac: {}", e))?;
    Ok(AuthIndex::from_graph(&graph))
}

/// Materialize the ACP engine's auth index for a scenario's N-Quads.
fn materialize_acp_index(nquads: &str) -> Result<AuthIndex, String> {
    let mut graph = Graph::load_dataset(nquads, "nquads").map_err(|e| format!("load: {}", e))?;
    materialize_acp(&mut graph).map_err(|e| format!("materialize_acp: {}", e))?;
    Ok(AuthIndex::from_graph(&graph))
}

#[test]
fn wac_differential_oracle_zero_divergence() {
    let scenarios = wac_corpus();
    let (pairs, divergences) = run_wac(&scenarios);

    // The runner-shape summary line (grep-parity with SHACL/geo + the
    // `WAC scenarios pass …` conformance line) so the sq-t58w.3 ratchet can re-check it.
    println!(
        "WAC differential pairs {} / divergences {} (floor {})",
        pairs,
        divergences.len(),
        DIVERGENCE_FLOOR
    );

    // Guard against a corpus that silently checks nothing: the pinned scenario floor
    // scenarios × multiple requests the conformance suite asserts.
    assert_eq!(
        scenarios.len(),
        common::WAC_SCENARIO_FLOOR,
        "expected the pinned WAC corpus"
    );
    assert!(
        pairs >= 40,
        "expected a substantive WAC request table, got {}",
        pairs
    );

    if !divergences.is_empty() {
        let report: String = divergences.iter().map(|d| format!("{}\n", d)).collect();
        panic!(
            "WAC differential oracle found {} divergence(s) across {} pairs (floor {}):\n{}",
            divergences.len(),
            pairs,
            DIVERGENCE_FLOOR,
            report
        );
    }
}

#[test]
fn acp_differential_oracle_zero_divergence() {
    let scenarios = acp_corpus();
    let (pairs, divergences) = run_acp(&scenarios);

    println!(
        "ACP differential pairs {} / divergences {} (floor {})",
        pairs,
        divergences.len(),
        DIVERGENCE_FLOOR
    );

    assert_eq!(
        scenarios.len(),
        common::ACP_SCENARIO_FLOOR,
        "expected the pinned ACP corpus"
    );
    assert!(
        pairs >= 35,
        "expected a substantive ACP request table, got {}",
        pairs
    );

    if !divergences.is_empty() {
        let report: String = divergences.iter().map(|d| format!("{}\n", d)).collect();
        panic!(
            "ACP differential oracle found {} divergence(s) across {} pairs (floor {}):\n{}",
            divergences.len(),
            pairs,
            DIVERGENCE_FLOOR,
            report
        );
    }
}

/// A NEGATIVE control: the three-way checker MUST flag a disagreement when one decider is
/// wrong. This proves the oracle actually compares (it would be worthless if `check_request`
/// silently passed). It does not assert any real decider is wrong — it feeds a synthetic
/// disagreeing verdict and asserts a divergence is recorded.
#[test]
fn oracle_flags_a_synthetic_disagreement() {
    let doc = "https://pod.example/neg/d1";
    let e = Expect::agent("https://bob.example/card#me")
        .read(doc)
        .is(Decision::Deny);
    // engine says Allow but expect says Deny → must be a divergence.
    let d = check_request("synthetic", &e, Ok(Decision::Allow), Ok(Decision::Deny));
    assert!(d.is_some(), "oracle must flag engine≠expect");

    // An Err from a decider is itself a divergence (fail-closed on an unclassifiable request).
    let d2 = check_request("synthetic", &e, Err("boom".to_owned()), Ok(Decision::Deny));
    assert!(
        d2.is_some(),
        "oracle must count an erroring decider as a divergence"
    );

    // All three agreeing → no divergence.
    let agree = Expect::agent("https://bob.example/card#me")
        .read(doc)
        .is(Decision::Deny);
    let none = check_request("synthetic", &agree, Ok(Decision::Deny), Ok(Decision::Deny));
    assert!(
        none.is_none(),
        "full agreement must not record a divergence"
    );
}

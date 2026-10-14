//! **Opt-in live conformance probe for the `driver` presets** (bead `sq-gum8.10`).
//!
//! This is the step that turns *one concrete configuration's*
//! `PresetEvidence::UpstreamDocs` into `LiveInstance`: it points a preset at the running
//! instance named by an environment variable, checks that the URL shape, transmission
//! method, and content negotiation the preset asserts actually produce a parseable
//! SPARQL-Results-JSON response, and only then calls `EndpointConfig::confirmed_live` on
//! the config that just answered. The promotion is in-process and per-run — it says
//! nothing about any other deployment, and nothing is written back into the presets.
//!
//! **CI never runs it.** The whole file compiles away without the `protocol-drivers`
//! feature, every test is `#[ignore]`d, and each one skips itself unless its endpoint
//! environment variable is set — so `cargo test --all-features` on a network-free
//! runner stays green and silent. A skipped test is *not* a probe: it reports "not
//! checked", never "confirmed".
//!
//! On a campaign box with the engines up:
//!
//! ```sh
//! export SPARQ_METAMORPH_BLAZEGRAPH=http://localhost:9999/blazegraph
//! export SPARQ_METAMORPH_BLAZEGRAPH_NAMESPACE=kb          # optional, defaults to kb
//! export SPARQ_METAMORPH_GRAPHDB=http://localhost:7200
//! export SPARQ_METAMORPH_GRAPHDB_REPOSITORY=campaign      # required with GRAPHDB
//! export SPARQ_METAMORPH_QLEVER=http://localhost:7001
//! export SPARQ_METAMORPH_MILLENNIUMDB=http://localhost:1234
//! export SPARQ_METAMORPH_FUSEKI=http://localhost:3030
//! export SPARQ_METAMORPH_FUSEKI_DATASET=campaign          # required with FUSEKI
//! export SPARQ_METAMORPH_OXIGRAPH=http://localhost:7878
//! export SPARQ_METAMORPH_VIRTUOSO=http://localhost:8890
//! cargo test -p sparq-metamorph --features protocol-drivers \
//!     --test preset_live_conformance -- --ignored --nocapture
//! ```
//!
//! A pass means the preset reached the engine and got results back; it is **not** a
//! statement about the engine's correctness (that is what the oracles are for), and a
//! failure is a misconfiguration report, not a found bug.

#![cfg(feature = "protocol-drivers")]

use sparq_difftest::QueryResults;
use sparq_metamorph::driver::{EndpointConfig, HttpSparqlEngine, PresetEvidence};
use sparq_metamorph::SparqlEngine;

/// The probe query: legal in every SPARQL 1.1 engine, cheap, and correct over an empty
/// store (zero solutions is a pass — we are checking the transport, not the data).
const PROBE: &str = "SELECT * WHERE { ?s ?p ?o } LIMIT 1";

/// Run `PROBE` through `config`, assert the response parsed as SELECT results, and
/// return the same configuration promoted to `PresetEvidence::LiveInstance` — the
/// promotion is earned by the exchange this function just completed, so it happens here
/// and nowhere else.
fn probe(config: EndpointConfig) -> EndpointConfig {
    println!(
        "probing {} at {} ({:?}, {:?}, extra={:?})",
        config.name, config.query_url, config.method, config.evidence, config.extra_params
    );
    assert_ne!(
        config.evidence,
        PresetEvidence::LiveInstance,
        "{}: a freshly built config claims live evidence before anything was sent",
        config.name
    );
    let name = config.name.clone();
    let engine = HttpSparqlEngine::new(config.clone());
    match engine.select(PROBE) {
        Ok(QueryResults::Solutions { solutions, .. }) => {
            let rows = solutions.len();
            println!("  ok: {name} preset reached the endpoint, {rows} row(s)");
            config.confirmed_live()
        }
        Ok(QueryResults::Boolean(_)) => panic!(
            "{name}: SELECT probe returned a boolean — the endpoint is not answering \
             the protocol the way the preset assumes"
        ),
        Err(failure) => panic!(
            "{name}: preset did not reach a working endpoint ({:?}): {}",
            failure.kind, failure.message
        ),
    }
}

/// Read `key`, or print why the probe is being skipped and return `None`.
fn endpoint(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            println!("skipping: {key} is not set");
            None
        }
    }
}

/// A companion variable that is *required* once the base URL is set — an endpoint whose
/// preset needs a dataset/repository/namespace has no safe default to guess.
fn required_with(base_key: &str, key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{base_key} is set, so {key} must be set too"))
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn blazegraph_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_BLAZEGRAPH") else {
        return;
    };
    let namespace = std::env::var("SPARQ_METAMORPH_BLAZEGRAPH_NAMESPACE")
        .unwrap_or_else(|_| "kb".to_string());
    let confirmed = probe(EndpointConfig::blazegraph(&base, &namespace));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn graphdb_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_GRAPHDB") else {
        return;
    };
    let repository = required_with(
        "SPARQ_METAMORPH_GRAPHDB",
        "SPARQ_METAMORPH_GRAPHDB_REPOSITORY",
    );
    let confirmed = probe(EndpointConfig::graphdb(&base, &repository));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn qlever_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_QLEVER") else {
        return;
    };
    let confirmed = probe(EndpointConfig::qlever(&base));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn millenniumdb_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_MILLENNIUMDB") else {
        return;
    };
    let confirmed = probe(EndpointConfig::millenniumdb(&base));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn fuseki_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_FUSEKI") else {
        return;
    };
    let dataset = required_with("SPARQ_METAMORPH_FUSEKI", "SPARQ_METAMORPH_FUSEKI_DATASET");
    let confirmed = probe(EndpointConfig::fuseki(&base, &dataset));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn oxigraph_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_OXIGRAPH") else {
        return;
    };
    let confirmed = probe(EndpointConfig::oxigraph(&base));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

#[test]
#[ignore = "opt-in: needs a live endpoint, see the module docs"]
fn virtuoso_preset_reaches_a_live_endpoint() {
    let Some(base) = endpoint("SPARQ_METAMORPH_VIRTUOSO") else {
        return;
    };
    let confirmed = probe(EndpointConfig::virtuoso(&base));
    assert_eq!(confirmed.evidence, PresetEvidence::LiveInstance);
}

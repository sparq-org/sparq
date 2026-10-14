//! CORE API v3 connector integration tests (sq-tzars.1) — exercise the REAL, SANITIZED
//! `core-batch.json` fixture + the paging/retry logic over an injected fake transport, with
//! ZERO network. Gated on `literature-live` (which implies `literature`), matching the
//! acceptance command `cargo test -p sparq-kb --features literature,literature-live`.
//! 🤖 SPARQ agent — research-KB live ingestion.
#![cfg(feature = "literature-live")]

use std::cell::RefCell;
use std::time::Duration;

use sparq_kb::literature::connector_core::{
    backoff_delay, fetch_paginated, parse_core_batch, CoreClient, HttpResponse, RetryPolicy,
    Transport,
};
use sparq_kb::literature::FIXTURE_CORE_BATCH;

// -------------------------------------------------------------------------------------
// parse_core_batch over the REAL recorded fixture.
// -------------------------------------------------------------------------------------

#[test]
fn real_fixture_parses_to_two_stubs_one_skipped() {
    let (stubs, skipped) = parse_core_batch(FIXTURE_CORE_BATCH).expect("fixture parses");
    // 3 records: 2 carry a DOI, 1 has doi:null (skipped, never silently dropped).
    assert_eq!(stubs.len(), 2, "two DOI-bearing works");
    assert_eq!(skipped, 1, "the doi:null record is counted as skipped");
    // DOIs are normalised (bare, lower-cased) and content-addressed.
    let sparql = stubs
        .iter()
        .find(|s| s.doi == "10.1145/1754239.1754244")
        .expect("SPARQL rewriting work present");
    assert_eq!(sparql.year, Some(2010));
    assert_eq!(
        sparql.source_iri(),
        "https://doi.org/10.1145/1754239.1754244"
    );
}

#[test]
fn real_fixture_every_stub_has_unknown_license_fail_closed() {
    let (stubs, _) = parse_core_batch(FIXTURE_CORE_BATCH).unwrap();
    // CORE v3's search schema carries no per-record license, so every stub is UNKNOWN
    // (None) — which the downstream tiering treats as NON-REDISTRIBUTABLE (fail-closed).
    assert!(
        stubs.iter().all(|s| s.license.is_none()),
        "unknown license captured as None"
    );
}

// -------------------------------------------------------------------------------------
// DOI normalisation + license capture (present + absent) on the CORE path.
// -------------------------------------------------------------------------------------

#[test]
fn core_doi_is_normalised_from_resolver_prefixed_and_uppercase() {
    let json = r#"{ "results": [
        { "doi": "https://doi.org/10.5281/Zenodo.ABC", "title": "Prefixed + mixed case" }
    ] }"#;
    let (stubs, _) = parse_core_batch(json).unwrap();
    assert_eq!(stubs.len(), 1);
    assert_eq!(stubs[0].doi, "10.5281/zenodo.abc");
}

#[test]
fn core_license_present_is_captured_absent_is_none() {
    let json = r#"{ "results": [
        { "doi": "10.1/a", "title": "licensed", "license": "cc-by" },
        { "doi": "10.1/b", "title": "unlicensed" },
        { "doi": "10.1/c", "title": "blank license", "license": "   " }
    ] }"#;
    let (stubs, _) = parse_core_batch(json).unwrap();
    let by = |d: &str| stubs.iter().find(|s| s.doi == d).unwrap();
    assert_eq!(by("10.1/a").license.as_deref(), Some("cc-by")); // present
    assert_eq!(by("10.1/b").license, None); // absent => fail-closed
    assert_eq!(by("10.1/c").license, None); // blank => treated as absent
}

// -------------------------------------------------------------------------------------
// Retry / backoff / rate-limit discipline via an injected fake transport (no sockets).
// -------------------------------------------------------------------------------------

/// A scripted fake transport — replays a queue of responses, counting calls. No network.
struct FakeTransport {
    queue: RefCell<Vec<HttpResponse>>,
    calls: RefCell<usize>,
}
impl FakeTransport {
    fn new(queue: Vec<HttpResponse>) -> Self {
        Self {
            queue: RefCell::new(queue),
            calls: RefCell::new(0),
        }
    }
}
impl Transport for FakeTransport {
    fn get(&self, _url: &str) -> Result<HttpResponse, String> {
        *self.calls.borrow_mut() += 1;
        let mut q = self.queue.borrow_mut();
        if q.is_empty() {
            Ok(resp(200, None, r#"{ "totalHits": 0, "results": [] }"#))
        } else {
            Ok(q.remove(0))
        }
    }
}

fn resp(status: u16, retry_after: Option<u64>, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        retry_after,
        body: body.to_string(),
    }
}

#[test]
fn retries_on_429_respecting_retry_after_then_succeeds() {
    let good = r#"{ "totalHits": 1, "results": [ { "doi": "10.1/a", "title": "A" } ] }"#;
    let fake = FakeTransport::new(vec![resp(429, Some(2), ""), resp(200, None, good)]);
    let mut slept: Vec<Duration> = Vec::new();
    let out = fetch_paginated(
        &fake,
        "https://x.test/works/",
        "zk sparql",
        &RetryPolicy::default(),
        &mut |d| slept.push(d),
    )
    .expect("succeeds after one retry");
    assert_eq!(out.stubs.len(), 1);
    assert_eq!(out.requests_made, 2, "one throttled + one success");
    assert_eq!(*fake.calls.borrow(), 2);
    // Honoured the server's Retry-After (2s), not the exponential fallback.
    assert_eq!(slept, vec![Duration::from_secs(2)]);
}

#[test]
fn exponential_backoff_when_no_retry_after_header() {
    let p = RetryPolicy {
        base_delay: Duration::from_millis(250),
        max_delay: Duration::from_secs(60),
        ..RetryPolicy::default()
    };
    assert_eq!(backoff_delay(0, None, &p), Duration::from_millis(250));
    assert_eq!(backoff_delay(1, None, &p), Duration::from_millis(500));
    assert_eq!(backoff_delay(3, None, &p), Duration::from_millis(2000));
}

#[test]
fn hard_request_cap_stops_a_persistently_throttled_endpoint() {
    let p = RetryPolicy {
        max_requests_per_run: 3,
        max_retries: 100,
        base_delay: Duration::from_millis(1),
        ..RetryPolicy::default()
    };
    // Endpoint always 429s; the cap must halt it (never an unbounded retry loop).
    let fake = FakeTransport::new(vec![
        resp(429, None, ""),
        resp(429, None, ""),
        resp(429, None, ""),
        resp(429, None, ""),
    ]);
    let err = fetch_paginated(&fake, "https://x.test/works/", "q", &p, &mut |_| {})
        .expect_err("cap trips");
    assert!(err.contains("hard request cap"), "got: {}", err);
    assert_eq!(*fake.calls.borrow(), 3, "stopped exactly at the request cap");
}

// -------------------------------------------------------------------------------------
// CoreClient::from_env — key read from the environment, NEVER logged. No network.
// -------------------------------------------------------------------------------------

#[test]
fn from_env_errors_without_key_and_succeeds_with_a_dummy_key() {
    // This is the only test that touches CORE_API_KEY; keep the mutation confined here.
    let saved = std::env::var("CORE_API_KEY").ok();

    std::env::remove_var("CORE_API_KEY");
    // NB: `CoreClient` deliberately does NOT derive `Debug` (it holds the key), so use
    // `.err()` rather than `expect_err` (which would require `T: Debug`).
    let err = CoreClient::from_env()
        .err()
        .expect("missing key is an error");
    assert!(err.contains("CORE_API_KEY"));
    // The error must NOT leak any value (there is none, but assert the shape stays generic).
    assert!(!err.to_lowercase().contains("bearer"));

    // A dummy value (NOT the real key) proves the read path without any network call.
    std::env::set_var("CORE_API_KEY", "dummy-test-value-not-a-real-key");
    assert!(CoreClient::from_env().is_ok(), "a set key constructs a client");
    // with_policy / policy accessor (no network): the override is reflected.
    let client = CoreClient::from_env()
        .unwrap()
        .with_policy(RetryPolicy {
            max_pages: 1,
            ..RetryPolicy::default()
        });
    assert_eq!(client.policy().max_pages, 1);

    std::env::set_var("CORE_API_KEY", "   ");
    assert!(
        CoreClient::from_env().is_err(),
        "a blank key is rejected (fail-closed)"
    );

    // Restore the environment for any other test in this binary.
    match saved {
        Some(v) => std::env::set_var("CORE_API_KEY", v),
        None => std::env::remove_var("CORE_API_KEY"),
    }
}

//! The **CORE API v3 connector** — a live (`literature-live`) HTTP adapter that turns the
//! CORE `/v3/search/works` search response into the same DOI-keyed `SourceStub`s the
//! `connector` module's `parse_openalex_batch` produces, so the downstream pipeline is
//! source-agnostic (§4.2). [SONNET-4.6] sq-tzars.1 (epic sq-tzars, design record
//! `research/research-kb-program.md`). 🤖 SPARQ agent — research-KB live ingestion.
//!
//! ## Two layers, two features
//!
//! - The **pure parse + retry discipline** (`parse_core_batch`, `RetryPolicy`,
//!   `backoff_delay`, the `Transport` trait, `fetch_paginated`) is behind the default-OFF
//!   `literature` feature. It makes NO network call itself — it drives an injected
//!   `Transport`, so CI exercises the paging + rate-limit + retry logic over a fake
//!   transport and the committed `core-batch.json` fixture with ZERO sockets.
//! - The **live socket layer** (`UreqTransport`, `CoreClient`) is behind the default-OFF
//!   `literature-live` feature (which implies `literature`) and pulls the one blocking HTTP
//!   client (`ureq`, already vendored for the engine's SERVICE transport). It is the ONLY
//!   part that touches the network, and it is NEVER driven in CI.
//!
//! ## Security posture (HARD constraints)
//!
//! - The CORE API key is read from the `CORE_API_KEY` environment variable at run time
//!   only (`CoreClient::from_env`). It is NEVER committed, logged, echoed, or placed in a
//!   URL — it travels solely in the `Authorization: Bearer …` request header, and no error
//!   message in this module interpolates it.
//! - License capture is **fail-closed**: `parse_core_batch` reads a per-record `license`
//!   when the CORE record carries one, else `None`. CORE v3's search schema does not
//!   include a per-record license, so in practice every stub is `license: None` = UNKNOWN,
//!   which the downstream dump tiering (`sq-tzars.7`) treats as NON-REDISTRIBUTABLE.
//! - Rate-limit discipline: the paged fetch honours HTTP `429`/`503` with the `Retry-After`
//!   header (bounded), falls back to bounded exponential backoff, and enforces a HARD cap
//!   on the total number of requests per run so a runaway query cannot hammer the API.

use std::time::Duration;

use serde_json::Value;

use super::connector::{normalise_doi, SourceStub};

/// The CORE v3 works-search endpoint (note the trailing slash — the API `301`-redirects the
/// slash-less form). Used only by the live `CoreClient`; tests use a fake base URL.
pub const CORE_SEARCH_WORKS_URL: &str = "https://api.core.ac.uk/v3/search/works/";

// --------------------------------------------------------------------------------------
// Pure parse (feature = "literature") — no network.
// --------------------------------------------------------------------------------------

/// Parse a recorded CORE API v3 `/v3/search/works` response (`{ "totalHits": N, "results":
/// [ { doi, title, abstract, yearPublished, … }, … ] }`) into normalised `SourceStub`s,
/// mirroring `parse_openalex_batch`. Records with no DOI or no (non-blank) title are skipped
/// (they cannot be content-addressed); the skipped count is returned so the caller can
/// report it (never silently lost).
///
/// Each stub's `license` is captured from the record's `license` field when present, else
/// `None` (UNKNOWN = non-redistributable downstream). Pure + deterministic; reads the
/// committed JSON string only — NO network.
pub fn parse_core_batch(json: &str) -> Result<(Vec<SourceStub>, usize), String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("CORE JSON: {}", e))?;
    let results = root
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "CORE JSON: missing `results` array".to_string())?;
    Ok(stubs_from_results(results))
}

/// Normalise a slice of CORE `results` records into `SourceStub`s (+ skipped count). Shared
/// by `parse_core_batch` and the paged fetch so both apply identical field mapping.
fn stubs_from_results(results: &[Value]) -> (Vec<SourceStub>, usize) {
    let mut stubs = Vec::new();
    let mut skipped = 0usize;
    for rec in results {
        let doi = rec.get("doi").and_then(Value::as_str).and_then(normalise_doi);
        let title = rec.get("title").and_then(Value::as_str).map(str::to_string);
        let (Some(doi), Some(title)) = (doi, title) else {
            skipped += 1;
            continue;
        };
        if title.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let abstract_text = rec
            .get("abstract")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // CORE v3 uses `yearPublished` (OpenAlex uses `publication_year`).
        let year = rec.get("yearPublished").and_then(Value::as_i64);
        let license = core_license(rec);
        stubs.push(SourceStub {
            doi,
            title,
            abstract_text,
            year,
            license,
        });
    }
    (stubs, skipped)
}

/// Capture the CORE record's license, fail-closed to `None`. A blank string is treated as
/// absent. CORE v3's search schema omits per-record licensing, so this is `None` in
/// practice — which the tiering treats as non-redistributable. [SONNET-4.6] sq-tzars.1
fn core_license(rec: &Value) -> Option<String> {
    rec.get("license")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// --------------------------------------------------------------------------------------
// Rate-limit + retry discipline (feature = "literature") — transport-agnostic, testable.
// --------------------------------------------------------------------------------------

/// One HTTP response the retry loop inspects: the numeric status, the parsed `Retry-After`
/// header (seconds, when present), and the response body.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The HTTP status code (e.g. `200`, `429`).
    pub status: u16,
    /// The `Retry-After` header parsed as whole seconds, when the server sent one.
    pub retry_after: Option<u64>,
    /// The response body (empty for a status-only response).
    pub body: String,
}

/// The blocking HTTP transport the paged fetch drives. Implemented by `UreqTransport` in
/// production and by a fake in tests, so the paging/retry logic runs with ZERO sockets.
pub trait Transport {
    /// Perform one `GET` and return the response (or a transport-level error string). The
    /// implementation must NOT turn a non-2xx status into an error — the retry loop needs
    /// the status + `Retry-After` header to decide whether to back off.
    fn get(&self, url: &str) -> Result<HttpResponse, String>;
}

/// Rate-limit + retry + pagination policy. Defaults are conservative discipline knobs (not
/// tuned/benchmarked numbers): honour a small burst of retries with bounded backoff, and
/// HARD-cap the total requests per run so a query cannot hammer the API.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum retries per URL after a retryable status before giving up.
    pub max_retries: u32,
    /// Base backoff delay; the exponential schedule is `base * 2^attempt`, capped.
    pub base_delay: Duration,
    /// Upper bound on any single backoff delay (also caps a huge `Retry-After`).
    pub max_delay: Duration,
    /// HARD cap on the total number of HTTP requests (incl. retries) across the whole run.
    pub max_requests_per_run: usize,
    /// Page size (CORE `limit` parameter).
    pub page_size: usize,
    /// HARD cap on the number of pages fetched per run.
    pub max_pages: usize,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            max_requests_per_run: 50,
            page_size: 25,
            max_pages: 20,
        }
    }
}

/// Whether an HTTP status is retryable: `429 Too Many Requests` and the transient `5xx`
/// gateway/overload statuses (`500`, `502`, `503`, `504`). A `4xx` other than `429` is a
/// permanent client error and is NOT retried.
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Compute the backoff delay for a retry attempt. Prefers the server's `Retry-After`
/// (capped at `max_delay`); otherwise bounded exponential backoff `base * 2^attempt`, capped
/// at `max_delay`. Pure — the caller does the sleeping (so tests never actually sleep).
pub fn backoff_delay(attempt: u32, retry_after: Option<u64>, policy: &RetryPolicy) -> Duration {
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs).min(policy.max_delay);
    }
    let factor = 2u64.saturating_pow(attempt);
    let base_ms = policy.base_delay.as_millis() as u64;
    Duration::from_millis(base_ms.saturating_mul(factor)).min(policy.max_delay)
}

/// Fetch one URL with retry discipline. Increments `requests` per attempt and refuses to
/// exceed `policy.max_requests_per_run` (the HARD cap). On a retryable status it sleeps the
/// `backoff_delay` (via the injected `sleep` sink) and retries up to `policy.max_retries`;
/// a non-retryable `>= 400` status is a hard error; a `< 400` status returns the response.
fn fetch_with_retry<T: Transport>(
    transport: &T,
    url: &str,
    policy: &RetryPolicy,
    requests: &mut usize,
    sleep: &mut dyn FnMut(Duration),
) -> Result<HttpResponse, String> {
    let mut attempt = 0u32;
    loop {
        if *requests >= policy.max_requests_per_run {
            return Err(format!(
                "CORE: hard request cap of {} reached before completing the fetch",
                policy.max_requests_per_run
            ));
        }
        *requests += 1;
        let resp = transport.get(url)?;
        if is_retryable_status(resp.status) {
            if attempt >= policy.max_retries {
                return Err(format!(
                    "CORE: gave up after {} retries; last HTTP status {}",
                    policy.max_retries, resp.status
                ));
            }
            let delay = backoff_delay(attempt, resp.retry_after, policy);
            sleep(delay);
            attempt += 1;
            continue;
        }
        if resp.status >= 400 {
            return Err(format!("CORE: non-retryable HTTP status {}", resp.status));
        }
        return Ok(resp);
    }
}

/// The result of a paged CORE search: the accumulated stubs, the skipped-record count, and
/// how many pages / HTTP requests the run actually made (for honest reporting).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreSearchResult {
    /// The normalised `SourceStub`s accumulated across every fetched page.
    pub stubs: Vec<SourceStub>,
    /// Records skipped across all pages (no DOI / no title) — surfaced, not lost.
    pub skipped: usize,
    /// Number of pages fetched.
    pub pages_fetched: usize,
    /// Total HTTP requests made (including retries) — bounded by `max_requests_per_run`.
    pub requests_made: usize,
}

/// Drive a paged CORE search over an injected `Transport`, honouring the `RetryPolicy`
/// (rate-limit backoff + HARD request/page caps). Stops when a page is empty, the reported
/// `totalHits` is reached, an under-full page arrives, or a HARD cap is hit. The `sleep`
/// sink receives each backoff delay (production passes `thread::sleep`; tests pass a
/// recorder). This is the load-bearing logic and is fully exercised with ZERO sockets.
pub fn fetch_paginated<T: Transport>(
    transport: &T,
    base_url: &str,
    query: &str,
    policy: &RetryPolicy,
    sleep: &mut dyn FnMut(Duration),
) -> Result<CoreSearchResult, String> {
    let mut out = CoreSearchResult::default();
    let mut offset = 0usize;
    loop {
        if out.pages_fetched >= policy.max_pages {
            break;
        }
        let url = build_search_url(base_url, query, policy.page_size, offset);
        let resp = fetch_with_retry(transport, &url, policy, &mut out.requests_made, sleep)?;
        let root: Value =
            serde_json::from_str(&resp.body).map_err(|e| format!("CORE JSON: {}", e))?;
        let results = root
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| "CORE JSON: missing `results` array".to_string())?;
        let got = results.len();
        let (page_stubs, page_skipped) = stubs_from_results(results);
        out.stubs.extend(page_stubs);
        out.skipped += page_skipped;
        out.pages_fetched += 1;

        let total = root.get("totalHits").and_then(Value::as_u64).map(|n| n as usize);
        offset += policy.page_size;
        if got == 0 {
            break;
        }
        if let Some(total) = total {
            if offset >= total {
                break;
            }
        }
        if got < policy.page_size {
            break;
        }
        if out.requests_made >= policy.max_requests_per_run {
            break;
        }
    }
    Ok(out)
}

/// Build a CORE search URL: `{base}?q={encoded}&limit={page_size}&offset={offset}`. The API
/// key is NEVER placed here — it travels only in the `Authorization` header.
fn build_search_url(base: &str, query: &str, page_size: usize, offset: usize) -> String {
    format!(
        "{}?q={}&limit={}&offset={}",
        base,
        percent_encode_query(query),
        page_size,
        offset
    )
}

/// Percent-encoding for a query-string value: the RFC 3986 unreserved set is kept verbatim,
/// everything else is uppercase `%XX` per UTF-8 byte. The ONE encoder for the literature
/// path (the `literature_pilot` bin reuses it) — `percent-encoding` keeps it sparq-core-free
/// under `literature` alone (#3712).
pub fn percent_encode_query(s: &str) -> String {
    const UNRESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, UNRESERVED).to_string()
}

// --------------------------------------------------------------------------------------
// Live socket layer (feature = "literature-live") — the ONLY networked part; never in CI.
// --------------------------------------------------------------------------------------

/// Max bytes read from a CORE response body — a finite cap so a runaway endpoint cannot OOM
/// the process (mirrors the engine's SERVICE transport discipline).
#[cfg(feature = "literature-live")]
const CORE_MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

/// The live blocking HTTP transport over `ureq` (rustls). Holds the API key (read from the
/// environment) and a per-request timeout. The key is sent ONLY in the `Authorization`
/// header and is never logged.
#[cfg(feature = "literature-live")]
pub struct UreqTransport {
    api_key: String,
    timeout: Duration,
}

#[cfg(feature = "literature-live")]
impl UreqTransport {
    /// Construct the transport from an API key + per-request timeout. The key is not logged.
    pub fn new(api_key: String, timeout: Duration) -> Self {
        Self { api_key, timeout }
    }
}

#[cfg(feature = "literature-live")]
impl Transport for UreqTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, String> {
        // `http_status_as_error(false)` so a 429/5xx returns the Response (with its
        // Retry-After header) instead of an Err — the retry loop needs to inspect it.
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .http_status_as_error(false)
            .user_agent(concat!("sparq-kb/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut resp = agent
            .get(url)
            // The key travels ONLY here, never in the URL or any log line.
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .call()
            .map_err(|e| format!("CORE: request failed: {}", e))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(CORE_MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|e| format!("CORE: reading response body: {}", e))?;
        Ok(HttpResponse {
            status,
            retry_after,
            body,
        })
    }
}

/// The live CORE API v3 client: a `UreqTransport` + a `RetryPolicy`. Construct it with
/// `from_env` (reads `CORE_API_KEY`), then call `search`. NETWORK — never driven in CI.
#[cfg(feature = "literature-live")]
pub struct CoreClient {
    transport: UreqTransport,
    policy: RetryPolicy,
}

#[cfg(feature = "literature-live")]
impl CoreClient {
    /// Build the client, reading the CORE API key from the `CORE_API_KEY` environment
    /// variable (loaded locally from `~/.config/sparq/core-api.env`). The key value is NEVER
    /// included in the returned error, logged, or echoed. Errors if the variable is unset or
    /// empty.
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("CORE_API_KEY").map_err(|_| {
            "CORE: CORE_API_KEY is not set in the environment (load it locally from \
             ~/.config/sparq/core-api.env; it is never committed or logged)"
                .to_string()
        })?;
        if api_key.trim().is_empty() {
            return Err("CORE: CORE_API_KEY is set but empty".to_string());
        }
        Ok(Self {
            transport: UreqTransport::new(api_key, Duration::from_secs(30)),
            policy: RetryPolicy::default(),
        })
    }

    /// Override the retry/rate-limit policy (e.g. a smaller request cap for a probe run).
    pub fn with_policy(mut self, policy: RetryPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The active retry/rate-limit policy (accessor).
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    /// Run a paged live CORE search for `query`, honouring the rate-limit + retry policy.
    /// NETWORK — this is the sole networked entry point and is never called in CI.
    pub fn search(&self, query: &str) -> Result<CoreSearchResult, String> {
        fetch_paginated(
            &self.transport,
            CORE_SEARCH_WORKS_URL,
            query,
            &self.policy,
            &mut |d| std::thread::sleep(d),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A fake transport that replays a scripted queue of responses (no sockets), recording
    /// how many times it was called. Used to exercise pagination + retry with ZERO network.
    struct FakeTransport {
        responses: RefCell<Vec<HttpResponse>>,
        calls: RefCell<usize>,
    }
    impl FakeTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: RefCell::new(responses),
                calls: RefCell::new(0),
            }
        }
    }
    impl Transport for FakeTransport {
        fn get(&self, _url: &str) -> Result<HttpResponse, String> {
            *self.calls.borrow_mut() += 1;
            let mut q = self.responses.borrow_mut();
            if q.is_empty() {
                // Exhausted script => an empty page (stops pagination).
                Ok(HttpResponse {
                    status: 200,
                    retry_after: None,
                    body: r#"{ "totalHits": 0, "results": [] }"#.to_string(),
                })
            } else {
                Ok(q.remove(0))
            }
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            retry_after: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn parse_core_batch_reads_doi_title_year_and_absent_license() {
        let json = r#"{ "totalHits": 1, "results": [
            { "doi": "10.1/A", "title": "T", "abstract": "abs", "yearPublished": 2021 }
        ] }"#;
        let (stubs, skipped) = parse_core_batch(json).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(stubs.len(), 1);
        assert_eq!(stubs[0].doi, "10.1/a"); // normalised, lower-cased
        assert_eq!(stubs[0].year, Some(2021));
        assert_eq!(stubs[0].license, None); // absent license => fail-closed None
    }

    #[test]
    fn parse_core_batch_captures_present_license_and_skips_doi_less() {
        let json = r#"{ "results": [
            { "doi": "https://doi.org/10.2/B", "title": "Licensed", "license": "cc-by" },
            { "title": "no doi", "abstract": "x" }
        ] }"#;
        let (stubs, skipped) = parse_core_batch(json).unwrap();
        assert_eq!(stubs.len(), 1);
        assert_eq!(skipped, 1);
        assert_eq!(stubs[0].license.as_deref(), Some("cc-by"));
    }

    #[test]
    fn parse_core_batch_malformed_is_error() {
        assert!(parse_core_batch("{ not json").is_err());
        assert!(parse_core_batch(r#"{ "no_results": 1 }"#).is_err());
    }

    #[test]
    fn is_retryable_status_covers_429_and_transient_5xx() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(404));
    }

    #[test]
    fn backoff_delay_prefers_retry_after_and_caps() {
        let p = RetryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            ..RetryPolicy::default()
        };
        // Retry-After honoured, capped at max_delay.
        assert_eq!(backoff_delay(0, Some(2), &p), Duration::from_secs(2));
        assert_eq!(backoff_delay(0, Some(999), &p), Duration::from_secs(5));
        // Exponential when no Retry-After: 100, 200, 400 ms …
        assert_eq!(backoff_delay(0, None, &p), Duration::from_millis(100));
        assert_eq!(backoff_delay(2, None, &p), Duration::from_millis(400));
    }

    #[test]
    fn retry_policy_default_is_conservative() {
        let p = RetryPolicy::default();
        assert!(p.max_retries >= 1);
        assert!(p.max_requests_per_run >= p.page_size.min(p.max_pages));
        assert!(p.base_delay <= p.max_delay);
    }

    #[test]
    fn fetch_paginated_single_page_stops_on_totalhits() {
        let body = r#"{ "totalHits": 2, "results": [
            { "doi": "10.1/a", "title": "A" },
            { "doi": "10.1/b", "title": "B" }
        ] }"#;
        let fake = FakeTransport::new(vec![ok(body)]);
        let mut slept: Vec<Duration> = Vec::new();
        let p = RetryPolicy {
            page_size: 25,
            ..RetryPolicy::default()
        };
        let res = fetch_paginated(&fake, "https://x.test/works/", "q", &p, &mut |d| {
            slept.push(d)
        })
        .unwrap();
        assert_eq!(res.stubs.len(), 2);
        assert_eq!(res.pages_fetched, 1);
        assert_eq!(res.requests_made, 1);
        assert!(slept.is_empty(), "no retry => no sleep");
        assert_eq!(*fake.calls.borrow(), 1);
    }

    #[test]
    fn fetch_paginated_walks_multiple_pages() {
        let page0 = r#"{ "totalHits": 4, "results": [
            { "doi": "10.1/a", "title": "A" },
            { "doi": "10.1/b", "title": "B" }
        ] }"#;
        let page1 = r#"{ "totalHits": 4, "results": [
            { "doi": "10.1/c", "title": "C" },
            { "doi": "10.1/d", "title": "D" }
        ] }"#;
        let fake = FakeTransport::new(vec![ok(page0), ok(page1)]);
        let p = RetryPolicy {
            page_size: 2,
            ..RetryPolicy::default()
        };
        let res =
            fetch_paginated(&fake, "https://x.test/works/", "q", &p, &mut |_| {}).unwrap();
        assert_eq!(res.stubs.len(), 4);
        assert_eq!(res.pages_fetched, 2);
        assert_eq!(*fake.calls.borrow(), 2);
    }

    #[test]
    fn fetch_paginated_retries_on_429_then_succeeds() {
        let mut throttled = ok("");
        throttled.status = 429;
        throttled.retry_after = Some(1);
        let good = r#"{ "totalHits": 1, "results": [ { "doi": "10.1/a", "title": "A" } ] }"#;
        let fake = FakeTransport::new(vec![throttled, ok(good)]);
        let mut slept: Vec<Duration> = Vec::new();
        let res = fetch_paginated(&fake, "https://x.test/works/", "q", &RetryPolicy::default(), &mut |d| slept.push(d)).unwrap();
        assert_eq!(res.stubs.len(), 1);
        assert_eq!(res.requests_made, 2, "one 429 + one success");
        assert_eq!(slept, vec![Duration::from_secs(1)], "slept the Retry-After");
    }

    #[test]
    fn fetch_with_retry_honours_hard_request_cap() {
        let mut throttled = ok("");
        throttled.status = 429;
        // Always 429 => never succeeds; the cap must stop it.
        let fake = FakeTransport::new(vec![throttled.clone(), throttled.clone(), throttled]);
        let p = RetryPolicy {
            max_requests_per_run: 2,
            max_retries: 10,
            base_delay: Duration::from_millis(1),
            ..RetryPolicy::default()
        };
        let mut requests = 0usize;
        let err = fetch_with_retry(&fake, "u", &p, &mut requests, &mut |_| {}).unwrap_err();
        assert!(err.contains("hard request cap"), "got: {}", err);
        assert_eq!(requests, 2, "stopped exactly at the cap");
    }

    #[test]
    fn fetch_with_retry_gives_up_after_max_retries() {
        let mut throttled = ok("");
        throttled.status = 503;
        let fake = FakeTransport::new(vec![
            throttled.clone(),
            throttled.clone(),
            throttled.clone(),
            throttled,
        ]);
        let p = RetryPolicy {
            max_retries: 2,
            max_requests_per_run: 100,
            base_delay: Duration::from_millis(1),
            ..RetryPolicy::default()
        };
        let mut requests = 0usize;
        let err = fetch_with_retry(&fake, "u", &p, &mut requests, &mut |_| {}).unwrap_err();
        assert!(err.contains("gave up after 2 retries"), "got: {}", err);
    }

    #[test]
    fn fetch_with_retry_non_retryable_4xx_is_hard_error() {
        let mut forbidden = ok("");
        forbidden.status = 403;
        let fake = FakeTransport::new(vec![forbidden]);
        let mut requests = 0usize;
        let err =
            fetch_with_retry(&fake, "u", &RetryPolicy::default(), &mut requests, &mut |_| {})
                .unwrap_err();
        assert!(err.contains("non-retryable HTTP status 403"), "got: {}", err);
        assert_eq!(requests, 1);
    }

    #[test]
    fn build_search_url_encodes_query_and_never_carries_a_key() {
        let url = build_search_url("https://api.core.ac.uk/v3/search/works/", "zero knowledge", 10, 20);
        assert_eq!(
            url,
            "https://api.core.ac.uk/v3/search/works/?q=zero%20knowledge&limit=10&offset=20"
        );
        assert!(!url.contains("Bearer"));
        assert!(!url.to_lowercase().contains("key"));
    }

    #[test]
    fn percent_encode_query_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(percent_encode_query("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(percent_encode_query("a b&c"), "a%20b%26c");
    }

    /// #3712 tripwire: every printable ASCII byte + a control + 2-/4-byte UTF-8 — exactly the
    /// RFC 3986 unreserved set passes through; the rest is uppercase `%XX` per UTF-8 byte.
    #[test]
    fn percent_encode_query_pins_the_unreserved_set() {
        let all: String = (0x20u8..0x7f)
            .map(char::from)
            .chain("\t\né😀".chars())
            .collect();
        assert_eq!(
            percent_encode_query(&all),
            "%20%21%22%23%24%25%26%27%28%29%2A%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%09%0A%C3%A9%F0%9F%98%80"
        );
    }
}

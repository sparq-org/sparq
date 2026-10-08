//! [OPUS-4.8] sq-jaj38 (epic sq-my8wd) — the W3C SPARQL 1.1 **Protocol** conformance lane.
//!
//! Where the sibling `service_eval` lane (sq-ddpgx) graduates the federated `SERVICE`
//! EVALUATION tests by driving the engine's `SERVICE` transport over the wire, THIS lane
//! exercises the **HTTP layer itself** — the W3C *SPARQL 1.1 Protocol* (the request/response
//! contract a SPARQL endpoint MUST honour) — against the same merged in-process loopback
//! server (sq-ushvx, #1291). Each assertion is a raw HTTP request to the bound
//! `127.0.0.1:0` endpoint with an EXACT method / `Content-Type` / `Accept` / body, checking
//! the status code, the `Content-Type` of the response and (where it matters) the payload
//! shape — exactly the bytes an Oxigraph / rdflib / Comunica client would send.
//!
//! ## What is exercised (the genuine protocol surface)
//!
//! Driven through the REAL `sparq_server::serve` HTTP handler ([`crate::http_protocol`]):
//!
//! * **query via GET** (`?query=…`), the read-only operation (Protocol §2.1.1);
//! * **query via POST-urlencoded** (`application/x-www-form-urlencoded`, `query=` body field, §2.1.2);
//! * **query via POST-direct** (`application/sparql-query`, the raw query IS the body, §2.1.3);
//! * the **HTTP `QUERY` method** (#1304 / sq-b3df9, w3c/sparql-protocol#40) — a query-ONLY
//!   verb behaving like a direct POST query but rejecting updates;
//! * **update via POST** (`application/sparql-update`, §2.2.3) — the mutating operation;
//! * **dataset overrides** — `default-graph-uri` / `named-graph-uri` (§2.1.4) re-scope the
//!   active dataset from the URL query string;
//! * **result-format content negotiation** for SELECT/ASK — `application/sparql-results+json`,
//!   `application/sparql-results+xml`, `text/csv`, `text/tab-separated-values` (the four W3C
//!   result-format registrations); and
//! * **status codes** — 200 (ok), 400 (malformed request / parse error), 405 (method not
//!   allowed, with an `Allow` header), 415 (unsupported request media type).
//!
//! ## Honest labelling (W3C-standard claim, scoped to what the server genuinely does)
//!
//! This is a **W3C SPARQL Protocol** conformance lane (family `W3C SPARQL`), distinct from the
//! experimental `sparq extension` ratchets. Each assertion that PASSES reflects a behaviour the
//! server genuinely implements end-to-end over real HTTP. [OPUS-4.8] sq-406acc: the server now
//! returns **406 Not Acceptable** for a present-but-unsatisfiable `Accept` (a SELECT/ASK request
//! whose `Accept` names no supported result format and no wildcard), matching Oxigraph
//! (w3c/sparql-protocol#40) — previously the documented sparq-vs-Oxigraph fallback divergence,
//! now a genuine PASS that raises the floor 20→21. The protocol features the server does **NOT**
//! implement are recorded as DOCUMENTED DIVERGENCES (an [`Outcome::Divergence`]), never a faked
//! pass:
//!
//! * **ASK boolean in CSV/TSV** — CSV and TSV have no boolean serialisation, so an ASK with
//!   `Accept: text/csv` correctly falls back to a JSON boolean (`ask_content_type`); the lane
//!   asserts that genuine fallback (a divergence from a naive "every Accept yields its own
//!   media type" expectation), not a CSV ASK body the server never produces.
//! * **absent / `*/*` Accept defaults to SRJ** — the W3C Protocol permits a default
//!   representation when `Accept` is absent (or a wildcard), so sparq keeps serving
//!   SPARQL-results JSON in that case rather than 406; the lane records this default as a
//!   divergence (asserting the converse of the 406 strictness) so the floor counts only the one
//!   genuine 406 pass.
//!
//! The FLOOR (`HTTP_PROTOCOL_FLOOR`, in the gating runner `tests/http_protocol_suite.rs`) is
//! the MEASURED count of PASS assertions — divergences are reported separately and are NOT
//! summed into it (so a documented gap can never inflate the conformance number). A genuine
//! FAILURE (a status/`Content-Type`/payload that contradicts what the server is supposed to do)
//! always fails the gate.
//!
//! ## Egress / SSRF posture (load-bearing — unchanged from the harness)
//!
//! This lane connects a plain `std::net::TcpStream` DIRECTLY to the bound loopback port the
//! [`crate::service_loopback::LoopbackEndpoint`] reports — it does **not** route through the
//! engine's `SERVICE` transport, so the engine's SSRF egress allowlist is not consulted or
//! widened at all (no `with_service_egress_allow`). The connection is scoped to exactly the
//! ephemeral `127.0.0.1:<port>` the harness bound; nothing else is reachable. Adding this lane
//! does not broaden egress in any way.
//!
//! ## Feature gating (both states)
//!
//! Behind this crate's opt-in `http-protocol` feature, which simply forwards to
//! `service-loopback` (tokio + axum via `sparq-server/server`) — the SAME heavy deps the
//! `service` lane already declares, so NO new third-party dependency is added. With the feature
//! OFF this module is `#[cfg]`-stripped and `tests/http_protocol_suite.rs` compiles to a single
//! self-SKIP `#[test]`, so the bare `cargo test -p sparq-conformance` (and the default
//! `--workspace` shards) never link tokio/axum here — the lean opt-in posture.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use sparq_core::Graph;

use crate::service_loopback::LoopbackEndpoint;

/// A minimal HTTP/1.1 response, parsed from the raw bytes the loopback server returns.
///
/// Only the fields the protocol lane asserts on are surfaced: the numeric status code, the
/// `Content-Type` header (lower-cased name lookup) and the response body as a `String`. This
/// is a deliberately tiny std-only parser — it is NOT a general HTTP client (no chunked
/// transfer, no keep-alive reuse), but the loopback server answers each request with a single
/// `Content-Length`-delimited response, which is all this lane needs.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The numeric HTTP status code (e.g. `200`, `400`, `405`, `415`).
    pub status: u16,
    /// The response headers, as `(lower-cased-name, value)` pairs in wire order.
    pub headers: Vec<(String, String)>,
    /// The response body (decoded lossily as UTF-8 — the result formats this lane checks are
    /// all UTF-8 text).
    pub body: String,
}

impl HttpResponse {
    /// The value of a response header by (case-insensitive) name, e.g. `content-type`.
    pub fn header(&self, name: &str) -> Option<&str> {
        let want = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == want)
            .map(|(_, v)| v.as_str())
    }

    /// The `Content-Type` of the response (lower-cased name lookup), if present.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }
}

/// A single raw HTTP request to drive against the loopback endpoint: the method token (any
/// string, so the non-standard `QUERY` verb is expressible), the path-and-query, the headers
/// and an optional body.
///
/// The `Host`, `Content-Length` and `Connection: close` headers are added by [`send`]; callers
/// supply only `Content-Type` / `Accept` (and any others a test needs).
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// The HTTP method token, e.g. `"GET"`, `"POST"`, or the custom `"QUERY"`.
    pub method: String,
    /// The request target (path + optional `?query-string`), e.g. `"/sparql?query=…"`.
    pub target: String,
    /// Request headers as `(name, value)` pairs (excluding `Host` / `Content-Length` /
    /// `Connection`, which [`send`] sets).
    pub headers: Vec<(String, String)>,
    /// The request body (may be empty).
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// A request with the given method + target and no headers/body.
    pub fn new(method: &str, target: &str) -> Self {
        HttpRequest {
            method: method.to_string(),
            target: target.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Add a header (builder style).
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Set the request body (builder style).
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

/// Send a raw [`HttpRequest`] to `addr` over a fresh `TcpStream`, returning the parsed
/// [`HttpResponse`]. Adds `Host`, `Content-Length` and `Connection: close` so the server
/// answers with one `Content-Length`-delimited response and closes — keeping the parser
/// trivial and each request fully independent.
///
/// `addr` is always the ephemeral `127.0.0.1:<port>` the loopback harness bound, so the
/// connection is loopback-scoped (see the module SSRF note). A short read/connect timeout
/// keeps a wedged server from hanging the suite.
pub fn send(addr: SocketAddr, req: &HttpRequest) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("connect {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("set_write_timeout: {e}"))?;

    let mut raw = Vec::new();
    // Request line + the mandatory Host header (HTTP/1.1).
    raw.extend_from_slice(format!("{} {} HTTP/1.1\r\n", req.method, req.target).as_bytes());
    raw.extend_from_slice(format!("Host: {addr}\r\n").as_bytes());
    raw.extend_from_slice(b"Connection: close\r\n");
    for (k, v) in &req.headers {
        raw.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
    }
    // Always send a Content-Length (0 for an empty body) so a body-less request is unambiguous.
    raw.extend_from_slice(format!("Content-Length: {}\r\n", req.body.len()).as_bytes());
    raw.extend_from_slice(b"\r\n");
    raw.extend_from_slice(&req.body);

    stream
        .write_all(&raw)
        .map_err(|e| format!("write request: {e}"))?;
    stream.flush().map_err(|e| format!("flush: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("read response: {e}"))?;
    parse_response(&buf)
}

/// Parse a raw HTTP/1.1 response (status line + headers + `Content-Length`-delimited body)
/// into an [`HttpResponse`]. Tolerant of a body shorter than the advertised length (a closed
/// connection): it returns whatever body bytes arrived.
pub fn parse_response(raw: &[u8]) -> Result<HttpResponse, String> {
    // Split head from body at the first CRLFCRLF.
    let sep = b"\r\n\r\n";
    let head_end = raw
        .windows(sep.len())
        .position(|w| w == sep)
        .ok_or_else(|| "malformed HTTP response: no header/body separator".to_string())?;
    let head = std::str::from_utf8(&raw[..head_end])
        .map_err(|e| format!("response head is not UTF-8: {e}"))?;
    let body_bytes = &raw[head_end + sep.len()..];

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "empty HTTP response".to_string())?;
    // "HTTP/1.1 200 OK" -> 200
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("malformed status line: {status_line:?}"))?
        .parse()
        .map_err(|e| format!("status code parse: {e}"))?;

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    let body = String::from_utf8_lossy(body_bytes).into_owned();
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// The outcome of one protocol assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The server genuinely satisfied the W3C-Protocol behaviour this assertion checks.
    Pass,
    /// A documented divergence: a permitted behaviour distinct from a naive expectation (e.g. an
    /// absent / `*/*` Accept defaulting to SPARQL-results JSON rather than 406 — the W3C-permitted
    /// default representation). Reported separately, NEVER summed into the floor. The string is
    /// the human-readable note.
    Divergence(String),
    /// A genuine failure: the server's response contradicts what it is supposed to do. Always
    /// fails the gate.
    Fail(String),
}

/// The result of running the whole protocol suite: the per-assertion outcomes plus the
/// rolled-up pass / divergence / fail counts.
#[derive(Debug, Clone, Default)]
pub struct SuiteReport {
    /// `(assertion label, outcome)` for every assertion, in run order.
    pub outcomes: Vec<(String, Outcome)>,
}

impl SuiteReport {
    fn record(&mut self, label: &str, outcome: Outcome) {
        self.outcomes.push((label.to_string(), outcome));
    }

    /// The number of PASS assertions — the ratchet count.
    pub fn pass(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| *o == Outcome::Pass)
            .count()
    }

    /// The number of documented DIVERGENCE assertions (reported, never summed into the floor).
    pub fn divergences(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Divergence(_)))
            .count()
    }

    /// The genuine FAILURES, as `(label, why)` — a non-empty list fails the gate.
    pub fn failures(&self) -> Vec<(String, String)> {
        self.outcomes
            .iter()
            .filter_map(|(label, o)| match o {
                Outcome::Fail(why) => Some((label.clone(), why.clone())),
                _ => None,
            })
            .collect()
    }
}

/// The fixture graph the protocol suite runs against: a tiny default graph plus two named
/// graphs (`g1`, `g2`) so the `default-graph-uri` / `named-graph-uri` dataset overrides have
/// something to re-scope.
pub fn fixture_graph() -> Graph {
    const NQ: &str = "\
        <http://ex/d> <http://ex/p> <http://ex/in_default> .\n\
        <http://ex/a> <http://ex/p> <http://ex/in_g1> <http://ex/g1> .\n\
        <http://ex/b> <http://ex/p> <http://ex/in_g2> <http://ex/g2> .\n";
    Graph::load_dataset(NQ, "nquads").expect("load fixture dataset")
}

/// The number of JSON result `bindings` in a SPARQL-results-JSON body (0 if it cannot be
/// parsed as such) — a tiny shape check the SELECT assertions use.
pub fn json_binding_count(body: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("results")
                .and_then(|r| r.get("bindings"))
                .and_then(|b| b.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0)
}

/// `true` if a SPARQL-results-JSON body encodes the boolean `{ "head": {}, "boolean": <b> }`.
pub fn json_boolean_is(body: &str, expected: bool) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("boolean").and_then(|b| b.as_bool()))
        == Some(expected)
}

/// Whether `content_type` starts with `expected` (a `Content-Type` may carry a `; charset=…`
/// parameter, so a prefix match is the right comparison).
pub fn content_type_is(content_type: Option<&str>, expected: &str) -> bool {
    content_type
        .map(|ct| ct.starts_with(expected))
        .unwrap_or(false)
}

/// Run the whole W3C SPARQL 1.1 Protocol conformance suite against a fresh in-process loopback
/// server over the [`fixture_graph`], returning the [`SuiteReport`].
///
/// Each protocol operation / status code / result format is one assertion; the loopback
/// endpoint is bound once and every assertion is an independent raw HTTP request to it. A PASS
/// means the server genuinely satisfied the W3C behaviour; an [`Outcome::Divergence`] is a
/// documented gap (reported, not counted in the floor); an [`Outcome::Fail`] is a genuine
/// contradiction (fails the gate).
pub fn run_protocol_suite() -> SuiteReport {
    let endpoint = LoopbackEndpoint::serve(fixture_graph());
    let addr = endpoint.addr();
    let mut report = SuiteReport::default();

    // A SELECT over the store default graph: the default-graph triple is the only one in the
    // unnamed graph (the g1/g2 triples are quads), so a plain SELECT binds exactly 1 row.
    let select_all = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    let urlenc_select = format!("/sparql?query={}", url_encode(select_all));

    // --- Operation: query via GET (Protocol §2.1.1) ---
    assert_status_ct_rows(
        &mut report,
        "query via GET (?query=…) → 200 + SRJ, 1 default-graph row",
        addr,
        &HttpRequest::new("GET", &urlenc_select)
            .header("Accept", "application/sparql-results+json"),
        200,
        "application/sparql-results+json",
        Some(1),
    );

    // --- Operation: query via POST url-encoded (§2.1.2) ---
    let form_body = format!("query={}", url_encode(select_all));
    assert_status_ct_rows(
        &mut report,
        "query via POST urlencoded (query= body) → 200 + SRJ",
        addr,
        &HttpRequest::new("POST", "/sparql")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/sparql-results+json")
            .body(form_body.clone()),
        200,
        "application/sparql-results+json",
        Some(1),
    );

    // --- Operation: query via POST direct (application/sparql-query, §2.1.3) ---
    assert_status_ct_rows(
        &mut report,
        "query via POST direct (application/sparql-query) → 200 + SRJ",
        addr,
        &HttpRequest::new("POST", "/sparql")
            .header("Content-Type", "application/sparql-query")
            .header("Accept", "application/sparql-results+json")
            .body(select_all),
        200,
        "application/sparql-results+json",
        Some(1),
    );

    // --- Operation: the HTTP QUERY method (#1304 / w3c/sparql-protocol#40) ---
    assert_status_ct_rows(
        &mut report,
        "query via the HTTP QUERY method (application/sparql-query) → 200 + SRJ",
        addr,
        &HttpRequest::new("QUERY", "/sparql")
            .header("Content-Type", "application/sparql-query")
            .header("Accept", "application/sparql-results+json")
            .body(select_all),
        200,
        "application/sparql-results+json",
        Some(1),
    );

    // QUERY is query-ONLY: an application/sparql-update body is 415, an update= form is 400.
    assert_status(
        &mut report,
        "QUERY method rejects application/sparql-update body → 415",
        addr,
        &HttpRequest::new("QUERY", "/sparql")
            .header("Content-Type", "application/sparql-update")
            .body("INSERT DATA { <http://ex/x> <http://ex/y> <http://ex/z> }"),
        415,
    );
    assert_status(
        &mut report,
        "QUERY method rejects update= form field → 400",
        addr,
        &HttpRequest::new("QUERY", "/sparql")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("update=INSERT%20DATA%20%7B%20%3Chttp%3A%2F%2Fex%2Fx%3E%20%3Chttp%3A%2F%2Fex%2Fy%3E%20%3Chttp%3A%2F%2Fex%2Fz%3E%20%7D"),
        400,
    );

    // --- Result-format content negotiation for SELECT (the four W3C registrations) ---
    assert_status_ct(
        &mut report,
        "SELECT Accept application/sparql-results+json → 200 + SRJ",
        addr,
        &select_get(&urlenc_select, "application/sparql-results+json"),
        200,
        "application/sparql-results+json",
    );
    assert_status_ct(
        &mut report,
        "SELECT Accept application/sparql-results+xml → 200 + SRX",
        addr,
        &select_get(&urlenc_select, "application/sparql-results+xml"),
        200,
        "application/sparql-results+xml",
    );
    assert_status_ct(
        &mut report,
        "SELECT Accept text/csv → 200 + text/csv",
        addr,
        &select_get(&urlenc_select, "text/csv"),
        200,
        "text/csv",
    );
    assert_status_ct(
        &mut report,
        "SELECT Accept text/tab-separated-values → 200 + TSV",
        addr,
        &select_get(&urlenc_select, "text/tab-separated-values"),
        200,
        "text/tab-separated-values",
    );

    // --- Result-format content negotiation for ASK (JSON + XML genuine; CSV/TSV fall back) ---
    let ask = "ASK { <http://ex/d> <http://ex/p> <http://ex/in_default> }";
    let ask_get = format!("/sparql?query={}", url_encode(ask));
    assert_ask(
        &mut report,
        "ASK Accept application/sparql-results+json → 200 + SRJ boolean true",
        addr,
        &select_get(&ask_get, "application/sparql-results+json"),
        "application/sparql-results+json",
        true,
    );
    assert_status_ct(
        &mut report,
        "ASK Accept application/sparql-results+xml → 200 + SRX",
        addr,
        &select_get(&ask_get, "application/sparql-results+xml"),
        200,
        "application/sparql-results+xml",
    );

    // --- Dataset overrides (§2.1.4) — re-scope the active dataset from the URL ---
    let select_g = format!(
        "/sparql?default-graph-uri=http://ex/g1&query={}",
        url_encode(select_all)
    );
    assert_status_ct_rows(
        &mut report,
        "default-graph-uri=g1 re-scopes the default graph → 1 row (g1's triple)",
        addr,
        &HttpRequest::new("GET", &select_g).header("Accept", "application/sparql-results+json"),
        200,
        "application/sparql-results+json",
        Some(1),
    );
    let named_q = "SELECT ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }";
    let named_g = format!(
        "/sparql?named-graph-uri=http://ex/g1&query={}",
        url_encode(named_q)
    );
    assert_status_ct_rows(
        &mut report,
        "named-graph-uri=g1 exposes exactly g1 as a named graph → 1 row",
        addr,
        &HttpRequest::new("GET", &named_g).header("Accept", "application/sparql-results+json"),
        200,
        "application/sparql-results+json",
        Some(1),
    );

    // --- Update via POST (§2.2.3): INSERT DATA, then prove the triple is visible ---
    assert_status(
        &mut report,
        "update via POST (application/sparql-update INSERT DATA) → 2xx",
        addr,
        &HttpRequest::new("POST", "/sparql")
            .header("Content-Type", "application/sparql-update")
            .body("INSERT DATA { <http://ex/new> <http://ex/p> <http://ex/v> }"),
        // The server answers a successful update with 200/204; accept either via the helper.
        200,
    );
    // The INSERTed triple is now visible — proves the update REALLY mutated the store (not a
    // no-op accepted-and-dropped). ASK over the new triple must be true.
    let ask_new = "ASK { <http://ex/new> <http://ex/p> <http://ex/v> }";
    let ask_new_get = format!("/sparql?query={}", url_encode(ask_new));
    assert_ask(
        &mut report,
        "POST update genuinely mutated the store (ASK new triple → true)",
        addr,
        &select_get(&ask_new_get, "application/sparql-results+json"),
        "application/sparql-results+json",
        true,
    );

    // --- Status codes ---
    // 400: a GET with no `query` parameter is a malformed protocol request.
    assert_status(
        &mut report,
        "GET /sparql with no query parameter → 400",
        addr,
        &HttpRequest::new("GET", "/sparql"),
        400,
    );
    // 400: a syntactically-malformed query.
    let bad_q = format!("/sparql?query={}", url_encode("SELECT WHERE {"));
    assert_status(
        &mut report,
        "GET /sparql with a malformed query → 400",
        addr,
        &HttpRequest::new("GET", &bad_q).header("Accept", "application/sparql-results+json"),
        400,
    );
    // 415: an unknown request Content-Type on POST.
    assert_status(
        &mut report,
        "POST /sparql with an unsupported Content-Type (text/plain) → 415",
        addr,
        &HttpRequest::new("POST", "/sparql")
            .header("Content-Type", "text/plain")
            .body(select_all),
        415,
    );
    // 405: an unsupported verb (DELETE) on /sparql, with an `Allow` header.
    assert_405_with_allow(
        &mut report,
        "DELETE /sparql → 405 with an Allow header",
        addr,
        &HttpRequest::new("DELETE", "/sparql"),
    );

    // --- 406 on a present-but-unsatisfiable Accept (SELECT) — now a genuine PASS ---
    // [OPUS-4.8] sq-406acc: sparq now returns 406 Not Acceptable when an `Accept` header is
    // present and names no supported SELECT/ASK result format (and no wildcard), matching
    // Oxigraph (w3c/sparql-protocol#40). This was the documented sparq-vs-Oxigraph divergence;
    // closing it raises HTTP_PROTOCOL_FLOOR 20→21 honestly (the server genuinely emits 406).
    {
        let label = "unsatisfiable Accept (image/png) on SELECT → 406 Not Acceptable \
                     (Oxigraph parity, sq-406acc)";
        match send(addr, &select_get(&urlenc_select, "image/png")) {
            Ok(resp) if resp.status == 406 => report.record(label, Outcome::Pass),
            Ok(resp) => report.record(
                label,
                Outcome::Fail(format!(
                    "expected 406 for a present-but-unsatisfiable Accept, got status {} ct {:?}",
                    resp.status,
                    resp.content_type()
                )),
            ),
            Err(e) => report.record(label, Outcome::Fail(e)),
        }
    }

    // --- The converse: an absent / `*/*` Accept still defaults to SRJ (NOT a 406) ---
    // [OPUS-4.8] sq-406acc: the strictness above must NOT regress the W3C-permitted default
    // representation when `Accept` is absent or a wildcard. This is the load-bearing
    // answer-safety check for the change; recorded as a DIVERGENCE so the floor stays at the
    // honest single +1 (the 406 pass) while still exercising the converse on every run.
    {
        let label = "absent / */* Accept on SELECT still defaults to SRJ, not 406 (sq-406acc)";
        // No Accept header at all → JSON default.
        let no_accept = HttpRequest::new("GET", &urlenc_select);
        let wildcard = select_get(&urlenc_select, "*/*");
        let absent_ok = matches!(
            send(addr, &no_accept),
            Ok(ref r) if r.status == 200 && content_type_is(r.content_type(), "application/sparql-results+json")
        );
        let wildcard_ok = matches!(
            send(addr, &wildcard),
            Ok(ref r) if r.status == 200 && content_type_is(r.content_type(), "application/sparql-results+json")
        );
        if absent_ok && wildcard_ok {
            report.record(
                label,
                Outcome::Divergence(
                    "absent / */* Accept defaults to SPARQL-results JSON (W3C-permitted default \
                     representation); only a present-but-unsatisfiable Accept is 406"
                        .to_string(),
                ),
            );
        } else {
            report.record(
                label,
                Outcome::Fail(format!(
                    "absent/*/* Accept must default to 200+SRJ (absent_ok={}, wildcard_ok={})",
                    absent_ok, wildcard_ok
                )),
            );
        }
    }

    // --- Documented divergence: ASK in CSV correctly falls back to a JSON boolean ---
    {
        let label = "ASK Accept text/csv falls back to a JSON boolean (CSV has no boolean form)";
        match send(addr, &select_get(&ask_get, "text/csv")) {
            Ok(resp)
                if resp.status == 200
                    && content_type_is(resp.content_type(), "application/sparql-results+json")
                    && json_boolean_is(&resp.body, true) =>
            {
                report.record(
                    label,
                    Outcome::Divergence(
                        "CSV/TSV have no SPARQL boolean serialisation, so an ASK with \
                         Accept: text/csv falls back to a JSON boolean (ask_content_type)"
                            .to_string(),
                    ),
                );
            }
            Ok(resp) => report.record(
                label,
                Outcome::Fail(format!(
                    "expected the documented ASK CSV→JSON-boolean fallback, got status {} ct {:?}",
                    resp.status,
                    resp.content_type()
                )),
            ),
            Err(e) => report.record(label, Outcome::Fail(e)),
        }
    }

    report
}

/// A GET to a `?query=…` target with the given `Accept`.
fn select_get(target: &str, accept: &str) -> HttpRequest {
    HttpRequest::new("GET", target).header("Accept", accept)
}

/// Assert the response to `req` has exactly `status`. Records Pass / Fail into `report`.
fn assert_status(
    report: &mut SuiteReport,
    label: &str,
    addr: SocketAddr,
    req: &HttpRequest,
    status: u16,
) {
    match send(addr, req) {
        Ok(resp) if status_ok(resp.status, status) => report.record(label, Outcome::Pass),
        Ok(resp) => report.record(
            label,
            Outcome::Fail(format!("expected status {}, got {}", status, resp.status)),
        ),
        Err(e) => report.record(label, Outcome::Fail(e)),
    }
}

/// Assert status + a `Content-Type` prefix.
fn assert_status_ct(
    report: &mut SuiteReport,
    label: &str,
    addr: SocketAddr,
    req: &HttpRequest,
    status: u16,
    ct: &str,
) {
    match send(addr, req) {
        Ok(resp) if resp.status == status && content_type_is(resp.content_type(), ct) => {
            report.record(label, Outcome::Pass)
        }
        Ok(resp) => report.record(
            label,
            Outcome::Fail(format!(
                "expected {} + Content-Type {ct:?}, got status {} ct {:?}",
                status,
                resp.status,
                resp.content_type()
            )),
        ),
        Err(e) => report.record(label, Outcome::Fail(e)),
    }
}

/// Assert status + a `Content-Type` prefix + an exact SELECT row count (when `rows` is `Some`).
fn assert_status_ct_rows(
    report: &mut SuiteReport,
    label: &str,
    addr: SocketAddr,
    req: &HttpRequest,
    status: u16,
    ct: &str,
    rows: Option<usize>,
) {
    match send(addr, req) {
        Ok(resp) => {
            if !status_ok(resp.status, status) {
                report.record(
                    label,
                    Outcome::Fail(format!("expected status {}, got {}", status, resp.status)),
                );
                return;
            }
            if !content_type_is(resp.content_type(), ct) {
                report.record(
                    label,
                    Outcome::Fail(format!(
                        "expected Content-Type {ct:?}, got {:?}",
                        resp.content_type()
                    )),
                );
                return;
            }
            if let Some(want) = rows {
                let got = json_binding_count(&resp.body);
                if got != want {
                    report.record(
                        label,
                        Outcome::Fail(format!("expected {want} rows, got {got}: {}", resp.body)),
                    );
                    return;
                }
            }
            report.record(label, Outcome::Pass);
        }
        Err(e) => report.record(label, Outcome::Fail(e)),
    }
}

/// Assert an ASK response: 200 + the given `Content-Type` + a JSON boolean equal to `expected`.
fn assert_ask(
    report: &mut SuiteReport,
    label: &str,
    addr: SocketAddr,
    req: &HttpRequest,
    ct: &str,
    expected: bool,
) {
    match send(addr, req) {
        Ok(resp)
            if resp.status == 200
                && content_type_is(resp.content_type(), ct)
                && (ct.contains("xml") || json_boolean_is(&resp.body, expected)) =>
        {
            report.record(label, Outcome::Pass)
        }
        Ok(resp) => report.record(
            label,
            Outcome::Fail(format!(
                "expected 200 + {ct:?} + boolean {expected}, got status {} ct {:?} body {}",
                resp.status,
                resp.content_type(),
                resp.body
            )),
        ),
        Err(e) => report.record(label, Outcome::Fail(e)),
    }
}

/// Assert a 405 response that carries the protocol-mandated `Allow` header.
fn assert_405_with_allow(
    report: &mut SuiteReport,
    label: &str,
    addr: SocketAddr,
    req: &HttpRequest,
) {
    match send(addr, req) {
        Ok(resp) if resp.status == 405 && resp.header("allow").is_some() => {
            report.record(label, Outcome::Pass)
        }
        Ok(resp) => report.record(
            label,
            Outcome::Fail(format!(
                "expected 405 + Allow header, got status {} allow {:?}",
                resp.status,
                resp.header("allow")
            )),
        ),
        Err(e) => report.record(label, Outcome::Fail(e)),
    }
}

/// A successful update may answer 200 or 204; treat a 2xx where 200 was asked for a write as OK.
/// Every other comparison is exact.
fn status_ok(got: u16, want: u16) -> bool {
    if want == 200 {
        (200..300).contains(&got)
    } else {
        got == want
    }
}

/// Percent-encode a string for inclusion in a URL query value (the subset this lane needs:
/// it escapes everything that is not an unreserved character, so SPARQL queries embed safely).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_escapes_reserved() {
        // Unreserved chars pass through; spaces, braces and slashes are percent-escaped.
        assert_eq!(url_encode("aZ0-_.~"), "aZ0-_.~");
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("{ }"), "%7B%20%7D");
        assert_eq!(url_encode("http://ex/g1"), "http%3A%2F%2Fex%2Fg1");
    }

    #[test]
    fn parse_response_extracts_status_headers_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/sparql-results+json\r\nContent-Length: 2\r\n\r\n{}";
        let resp = parse_response(raw).expect("parse");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type(), Some("application/sparql-results+json"));
        assert_eq!(resp.header("content-length"), Some("2"));
        assert_eq!(resp.body, "{}");
    }

    #[test]
    fn parse_response_rejects_headless() {
        assert!(parse_response(b"garbage-with-no-separator").is_err());
    }

    #[test]
    fn content_type_is_prefix_matches() {
        assert!(content_type_is(Some("text/csv; charset=utf-8"), "text/csv"));
        assert!(content_type_is(
            Some("application/sparql-results+json"),
            "application/sparql-results+json"
        ));
        assert!(!content_type_is(Some("text/plain"), "text/csv"));
        assert!(!content_type_is(None, "text/csv"));
    }

    #[test]
    fn json_helpers_read_shape() {
        let bindings =
            r#"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"x"}}]}}"#;
        assert_eq!(json_binding_count(bindings), 1);
        assert_eq!(json_binding_count("not json"), 0);
        let boolean = r#"{"head":{},"boolean":true}"#;
        assert!(json_boolean_is(boolean, true));
        assert!(!json_boolean_is(boolean, false));
        assert!(!json_boolean_is("not json", true));
    }

    #[test]
    fn http_request_builder_sets_fields() {
        let req = HttpRequest::new("QUERY", "/sparql")
            .header("Content-Type", "application/sparql-query")
            .body("SELECT * WHERE { ?s ?p ?o }");
        assert_eq!(req.method, "QUERY");
        assert_eq!(req.target, "/sparql");
        assert_eq!(req.headers.len(), 1);
        assert_eq!(req.headers[0].0, "Content-Type");
        assert_eq!(req.body, b"SELECT * WHERE { ?s ?p ?o }".to_vec());
    }

    #[test]
    fn fixture_graph_default_graph_has_one_triple() {
        // `Graph::len()` counts the DEFAULT-graph triples only (named-graph quads are held
        // in a separate overlay), so the fixture's single unnamed triple is the count. The
        // two named graphs (g1, g2) are exercised separately by the dataset-override
        // assertions in `run_protocol_suite` (which the live server resolves correctly).
        let g = fixture_graph();
        assert_eq!(
            g.len(),
            1,
            "the fixture's default graph holds exactly the one unnamed triple"
        );
    }

    #[test]
    fn suite_report_counts_outcomes() {
        let mut r = SuiteReport::default();
        r.record("a", Outcome::Pass);
        r.record("b", Outcome::Pass);
        r.record("c", Outcome::Divergence("doc gap".to_string()));
        r.record("d", Outcome::Fail("boom".to_string()));
        assert_eq!(r.pass(), 2);
        assert_eq!(r.divergences(), 1);
        let fails = r.failures();
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0], ("d".to_string(), "boom".to_string()));
    }

    #[test]
    fn status_ok_is_exact_except_2xx_write() {
        // A write asked-for-200 accepts any 2xx (200/204).
        assert!(status_ok(200, 200));
        assert!(status_ok(204, 200));
        assert!(!status_ok(400, 200));
        // Non-200 expectations are exact.
        assert!(status_ok(415, 415));
        assert!(!status_ok(400, 415));
    }
}

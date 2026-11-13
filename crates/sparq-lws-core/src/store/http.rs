// AUTHORED-BY Claude Opus 4.8
//! The **live** [`SparqClient`] over HTTP — the M3 data path.
//!
//! [`HttpSparqClient`] implements the authoritative-RDF seam by talking to a SPARQ SPARQL endpoint
//! over the **SPARQL 1.1 Protocol**: a query is `POST <endpoint> Content-Type:
//! application/sparql-query` (results `application/sparql-results+json` for SELECT/ASK,
//! `text/turtle`/N-Triples for CONSTRUCT); an update is `POST <endpoint> Content-Type:
//! application/sparql-update` (success → `204 No Content`). Every untrusted IRI/literal flows through
//! the injection-safe builders in [`super::sparql`] — never string-concatenated.
//!
//! ## Verified SPARQ HTTP surface (against `sparq-org/sparq` `crates/sparq-server`, local `52224c4`)
//!
//! `sparq-server` is described as a *"W3C-conformant HTTP server exposing the sparq query engine
//! (SPARQL 1.1 Protocol + Graph Store HTTP Protocol read side)."* Its `/sparql` route accepts:
//! - **GET/POST query** — `query=` (GET / urlencoded body) or `application/sparql-query` (POST body).
//! - **POST update** — `application/sparql-update` body; success → **204**, failure → 400 JSON.
//! - **Results** — `Accept`-negotiated; `application/sparql-results+json` is the default + native
//!   path for SELECT/ASK (`{"head":{"vars":[…]},"results":{"bindings":[…]}}` / `{"boolean":…}`);
//!   CONSTRUCT/DESCRIBE negotiate `application/n-triples` (default) or `text/turtle`.
//! - **Errors** — a structured `{"error":"…"}` JSON body with the status (400/413/500/503/501).
//!
//! ### DEVIATIONS / ASSUMPTIONS this client codes to
//! 1. **Named-graph-per-resource isolation is enforced at the ENGINE, not yet over HTTP.** The
//!    engine's `query` / `update_in_place` fully support `GRAPH <g> { … }` (named graphs preserved
//!    across updates, visible to queries), so the WAC-design model — graph IRI == resource IRI — is
//!    expressed in every query/update this client builds. BUT `sparq-server` today serves a
//!    *triple-only default-graph* `Graph` for its **Graph Store HTTP Protocol** read side, and folds
//!    TriG/N-Quads named graphs into the default graph on load. Whether the live `/sparql` POST path
//!    materialises true per-graph isolation depends on the server's store wiring, which is in flux
//!    (the `sparq-solid` named-graph + auth-view crate is NOT yet wired into `sparq-server` — FR-4 in
//!    `solid-server-rs-wac.md`). This client therefore codes to the *target* (per-resource named
//!    graphs over `/sparql`); the live integration test (`#[ignore]`) is where that assumption is
//!    confirmed against a running SPARQ. If the live surface deviates, only the builders in
//!    [`super::sparql`] change — the HTTP plumbing here is surface-stable.
//! 2. **No SPARQL Graph-Store write API is used** — GSP write verbs return 501 on `sparq-server`;
//!    all writes go through `/sparql` POST `application/sparql-update`, which is implemented.
//! 3. **`create_child` atomicity** relies on the guarded `DELETE/INSERT … WHERE { container-record
//!    EXISTS }` being one atomic SPARQL update on the server (it is — `apply_update` commits a whole
//!    update as one generation). A missing container ⇒ the WHERE yields nothing ⇒ nothing inserted;
//!    this client then confirms via a per-operation **create-marker** nonce written atomically with
//!    the guarded insert (a signal no concurrent `remove_child`/sibling create ever touches, so the
//!    confirm is immune to a containment-edge race) and maps "marker absent" to
//!    [`SparqError::NotFound`], mirroring the in-memory atomic impl's container-EXISTS guard.
//!
//! ## Operational posture
//! The endpoint URL is operator-configured (a trusted internal service), but the client is still
//! defensive: a per-request **timeout**, a **bounded** response read, and **typed** errors that
//! distinguish a *retryable* transport/5xx failure (the typed [`SparqHttpError`], whose
//! [`is_retryable`](SparqHttpError::is_retryable) records retryability before it folds into the
//! opaque [`SparqError::Backend`]) from a *fatal* 4xx/malformed-response failure. It **fails
//! closed**: any ambiguity surfaces as an error, never a silent success or a fabricated
//! "exists/empty".
//!
//! M3-next seams (left explicitly unimplemented): WAC / access-controlled query (gated on sparq#992),
//! notifications (subscription socket), the reconciler GC, and **TLS** (this client is plain HTTP to
//! a trusted internal endpoint; an `https`/rustls connector is the next adapter behind the same type).

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::{header, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

use super::sparq::{DeleteOutcome, ReadPlan, ResourceMeta, SparqClient, SparqError};
use super::sparql;
use super::timestamp;

/// The maximum SPARQL response body this client will buffer (fail-closed bound — a runaway response
/// is an error, never an OOM). 16 MiB comfortably covers index records + a single resource's RDF.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Default per-request timeout. The whole request/response cycle (connect + send + read) must finish
/// inside this, else the request is abandoned with a retryable [`SparqHttpError::Timeout`].
const DEFAULT_TIMEOUT: Duration = super::limits::STORE_CALL_TIMEOUT;

const CT_SPARQL_QUERY: &str = "application/sparql-query";
const CT_SPARQL_UPDATE: &str = "application/sparql-update";
const ACCEPT_RESULTS_JSON: &str = "application/sparql-results+json";
const ACCEPT_NTRIPLES: &str = "application/n-triples";

/// A unique nonce for one write: the race-resistant create and delete markers, and the fence of a
/// write that may still commit ([`sparql::update_fence`]). Fences share one graph across every
/// resource and every client process, and stay, so a nonce must not repeat across processes or
/// their restarts: it is 128 bits from the OS RNG. When that is unavailable, the write fails
/// closed (as `CompositeStore::mint_blob_key` does) rather than use a nonce that could collide
/// and let one write's fence suppress another's.
fn next_nonce() -> Result<String, SparqError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| SparqError::Backend(format!("fatal: OS RNG unavailable for a nonce: {e}")))?;
    Ok(bytes.iter().fold(String::from("op-"), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    }))
}

/// A typed SPARQ-over-HTTP error, carrying whether the failure is worth retrying.
///
/// The split is load-bearing for the composite store's crash-consistency model: a *retryable*
/// failure (connect refused, a 5xx, a timeout) is a transient backend blip the caller may re-issue;
/// a *fatal* failure (a 4xx, a malformed/unparseable response) is a bug or a permanent rejection that
/// retrying cannot fix. Query errors map onto [`SparqError::Backend`] with the retryability hint.
/// A guarded update whose outcome cannot yet be settled carries [`SparqError::Unsettled`] and
/// its nonce fence instead.
#[derive(Debug, thiserror::Error)]
pub enum SparqHttpError {
    /// The request could not be built (a programmer error — never from untrusted input). Fatal.
    #[error("request build error: {0}")]
    Build(String),
    /// A transport failure (connect refused, reset, DNS) — retryable.
    #[error("transport error: {0}")]
    Transport(String),
    /// The per-request deadline elapsed — retryable.
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    /// The server returned a 5xx — retryable (a transient backend condition).
    #[error("server error (HTTP {status})")]
    ServerStatus { status: u16 },
    /// The server returned a 4xx — fatal (a malformed query / rejected request; retrying won't help).
    #[error("client error (HTTP {status})")]
    ClientStatus { status: u16 },
    /// The response body could not be read (the connection failed part way) — fatal.
    #[error("response body unreadable")]
    Body,
    /// The response body passed the size it may take (`MAX_RESPONSE_BYTES`, or a caller's
    /// smaller cap) — fatal.
    #[error("response too large")]
    TooLarge,
    /// The response was not the expected shape (e.g. not valid sparql-results JSON) — fatal.
    #[error("malformed response: {0}")]
    Malformed(String),
}

impl SparqHttpError {
    /// Whether re-issuing the request might succeed (transient transport/5xx/timeout failures).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SparqHttpError::Transport(_)
                | SparqHttpError::Timeout(_)
                | SparqHttpError::ServerStatus { .. }
        )
    }

    /// Whether an UPDATE that failed this way may still commit after the client has answered:
    /// its outcome is not known. A timeout or a transport failure can leave the request in a
    /// socket or queued on the server (the client gives up; `sparq-server` does not stop an update
    /// it has received), a `5xx` can be `sparq-server`'s own wait elapsing while its writer goes on
    /// (`503`), and an unreadable `2xx` body follows a commit. Only a refusal (`4xx`, `501`), or a
    /// request never built, is final.
    pub fn may_commit_later(&self) -> bool {
        matches!(
            self,
            SparqHttpError::Transport(_)
                | SparqHttpError::Timeout(_)
                | SparqHttpError::ServerStatus { .. }
                | SparqHttpError::Body
                | SparqHttpError::TooLarge
        )
    }

    /// Fold into the opaque trait error, preserving the retryability hint in the message so a future
    /// retry layer (and the logs) can distinguish transient from permanent.
    fn into_sparq(self) -> SparqError {
        let retry = if self.is_retryable() {
            "retryable"
        } else {
            "fatal"
        };
        SparqError::Backend(format!("{retry}: {self}"))
    }
}

/// A live [`SparqClient`] over a SPARQ SPARQL HTTP endpoint.
///
/// Cheap to clone (the inner hyper client is `Arc`-backed connection pool); construct once and share.
#[derive(Clone)]
pub struct HttpSparqClient {
    client: Client<HttpConnector, Full<Bytes>>,
    /// The absolute `/sparql` endpoint URL (operator-configured, trusted internal service).
    endpoint: String,
    lifecycle: std::sync::Arc<super::Lifecycle>,
    timeout: Duration,
    /// Fences writes from earlier processes. Clones and service instances using this client in
    /// the same process share the active epoch; it is not an instance ownership mechanism.
    epoch: Arc<AtomicU64>,
}

impl HttpSparqClient {
    /// Build a client targeting `endpoint` (the SPARQ `/sparql` URL, e.g.
    /// `http://sparq.internal:8080/sparql`) with the default timeout.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self::with_timeout(endpoint, DEFAULT_TIMEOUT)
    }

    /// Build a client with an explicit per-request timeout.
    pub fn with_timeout(endpoint: impl Into<String>, timeout: Duration) -> Self {
        // Plain-HTTP connector to a trusted internal endpoint (TLS is an M3-next adapter). The pool
        // is kept small + idle-bounded so the client never holds connections open indefinitely.
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(timeout));
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .build(connector);
        Self {
            client,
            endpoint: endpoint.into(),
            lifecycle: Default::default(),
            timeout,
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The children `q` selects, or `None` once their IRIs pass `max_bytes`.
    ///
    /// With `rows` (the query asks for one more), the response is read only up to a size such a
    /// listing could take, and more rows than `rows` is a listing that does not fit: `None`.
    async fn children(
        &self,
        q: String,
        max_bytes: usize,
        rows: Option<usize>,
    ) -> Result<Option<Vec<String>>, SparqError> {
        let cap = match rows {
            // Each binding's JSON around its IRI is well under 256 bytes; an IRI may be escaped
            // to at most twice its length.
            Some(n) => max_bytes
                .saturating_mul(2)
                .saturating_add(n.saturating_add(1).saturating_mul(256))
                .saturating_add(4096)
                .min(MAX_RESPONSE_BYTES as usize),
            None => MAX_RESPONSE_BYTES as usize,
        };
        let body = match self.query_raw_within(&q, ACCEPT_RESULTS_JSON, cap).await {
            Ok((body, _ct)) => body,
            // Only a response past the cap is a listing that does not fit; one that could not
            // be read is a failure, whatever its size.
            Err(SparqHttpError::TooLarge) if rows.is_some() => return Ok(None),
            Err(e) => return Err(e.into_sparq()),
        };
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        if rows.is_some_and(|n| result.rows.len() > n) {
            return Ok(None);
        }
        let mut children = Vec::with_capacity(result.rows.len());
        let mut bytes = 0usize;
        for row in result.rows {
            // A row of a `SELECT ?child` MUST carry the `child` binding. A row missing it is a
            // malformed backend response, NOT an empty/partial list — surface it as a fatal error
            // rather than silently dropping it, because list_children feeds the fail-closed
            // empty-container DELETE check (a silently-shortened list could wrongly let a non-empty
            // container be deleted).
            match row.get("child") {
                Some(child) => {
                    bytes = bytes.saturating_add(child.len());
                    if bytes > max_bytes {
                        return Ok(None);
                    }
                    children.push(child.clone())
                }
                None => {
                    return Err(SparqHttpError::Malformed(
                        "select-children row missing the 'child' binding".into(),
                    )
                    .into_sparq())
                }
            }
        }
        Ok(Some(children))
    }

    /// Issue a SPARQL **query**, returning the raw body bytes + the response `Content-Type`. The
    /// `accept` header drives the result serialisation.
    async fn query_raw(
        &self,
        sparql: &str,
        accept: &str,
    ) -> Result<(Bytes, String), SparqHttpError> {
        self.query_raw_within(sparql, accept, MAX_RESPONSE_BYTES as usize)
            .await
    }

    /// [`query_raw`](Self::query_raw), its response read up to `cap` bytes
    /// ([`SparqHttpError::TooLarge`] past that).
    async fn query_raw_within(
        &self,
        sparql: &str,
        accept: &str,
        cap: usize,
    ) -> Result<(Bytes, String), SparqHttpError> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(&self.endpoint)
            .header(header::CONTENT_TYPE, CT_SPARQL_QUERY)
            .header(header::ACCEPT, accept)
            .body(Full::new(Bytes::from(sparql.to_string())))
            .map_err(|e| SparqHttpError::Build(e.to_string()))?;
        self.send_within(req, cap).await
    }

    /// Issue a SPARQL **update** (`application/sparql-update`); a 2xx (the server uses 204) is
    /// success. The body is read + discarded (errors are conveyed by status).
    async fn update_raw(&self, sparql: &str) -> Result<(), SparqHttpError> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(&self.endpoint)
            .header(header::CONTENT_TYPE, CT_SPARQL_UPDATE)
            .body(Full::new(Bytes::from(sparql.to_string())))
            .map_err(|e| SparqHttpError::Build(e.to_string()))?;
        let _ = self.send(req).await?;
        Ok(())
    }

    /// Send the fenced update `u` (tagged `nonce`, see [`sparql::update_create_child_fenced`]):
    /// `Ok(false)` when it was applied, `Ok(true)` when it failed but may still commit and has
    /// now been fenced ([`sparql::update_fence`]), so its outcome is final and a read tells it.
    /// When the fence fails too, the outcome is not known: [`SparqError::Unsettled`], with what
    /// settles it later.
    async fn update_fenced(
        &self,
        u: &str,
        nonce: &str,
        touches: &[&str],
    ) -> Result<bool, SparqError> {
        match self.update_raw(u).await {
            Ok(()) => Ok(false),
            Err(e) if e.may_commit_later() => {
                match self.update_raw(&sparql::update_fence(nonce)).await {
                    Ok(()) => Ok(true),
                    Err(fence) => Err(SparqError::Unsettled {
                        detail: format!("{e}; fencing it failed: {fence}"),
                        fence: super::sparq::Fence::new(
                            nonce,
                            touches.iter().map(|t| t.to_string()).collect(),
                        ),
                    }),
                }
            }
            Err(e) => Err(e.into_sparq()),
        }
    }

    /// Send a request under the timeout, classify the status, and read the bounded body. Returns the
    /// body bytes + `Content-Type` for a 2xx; a typed error otherwise.
    ///
    /// The timeout wraps the WHOLE exchange — connect, header receipt, AND the bounded body read —
    /// via one inner async block, so a server that sends headers then stalls the body cannot hang the
    /// client past the deadline (the headers-only timeout was the round-2 finding). The status is
    /// classified from the response HEADERS *before* the body is read, so a non-2xx returns its typed
    /// status error regardless of whether the (ignored) error body reads cleanly — a 5xx with an
    /// oversized/unreadable body stays a retryable `ServerStatus`, never a fatal `Body`.
    async fn send(&self, req: Request<Full<Bytes>>) -> Result<(Bytes, String), SparqHttpError> {
        self.send_within(req, MAX_RESPONSE_BYTES as usize).await
    }

    /// [`send`](Self::send), its response body read up to `cap` bytes.
    async fn send_within(
        &self,
        req: Request<Full<Bytes>>,
        cap: usize,
    ) -> Result<(Bytes, String), SparqHttpError> {
        let client = self.client.clone();
        let exchange = async move {
            let resp = client
                .request(req)
                .await
                .map_err(|e| SparqHttpError::Transport(e.to_string()))?;
            let status = resp.status();
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            // Classify the status FIRST (header-only) so a non-2xx's typed error does not depend on a
            // clean body read. 2xx → ok (read the body, which IS the result); 501 → fatal (a permanent
            // "not implemented", checked BEFORE the generic 5xx branch else `is_server_error()` would
            // mark it retryable); other 5xx → retryable; everything else (4xx, an unexpected 3xx — the
            // client follows no redirects to a trusted internal endpoint) → fatal. The error body is
            // not needed (errors are conveyed by status), so it is dropped unread.
            if status.is_success() {
                let body = read_bounded(resp.into_body(), cap).await?;
                Ok((body, content_type))
            } else if status == StatusCode::NOT_IMPLEMENTED {
                Err(SparqHttpError::ClientStatus {
                    status: status.as_u16(),
                })
            } else if status.is_server_error() {
                Err(SparqHttpError::ServerStatus {
                    status: status.as_u16(),
                })
            } else {
                Err(SparqHttpError::ClientStatus {
                    status: status.as_u16(),
                })
            }
        };
        match tokio::time::timeout(self.timeout, exchange).await {
            Ok(result) => result,
            Err(_elapsed) => Err(SparqHttpError::Timeout(self.timeout)),
        }
    }
}

/// Read an HTTP response body into [`Bytes`], failing closed if it exceeds [`MAX_RESPONSE_BYTES`].
///
/// `B` is the concrete incoming body the hyper-util legacy client yields ([`hyper::body::Incoming`]);
/// [`http_body_util::Limited`] bounds the read, so a runaway response is a fatal
/// [`SparqHttpError::TooLarge`] rather than an OOM; a body that fails part way is
/// [`SparqHttpError::Body`], never taken for one too large.
async fn read_bounded<B>(body: B, cap: usize) -> Result<Bytes, SparqHttpError>
where
    B: hyper::body::Body,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let limited = http_body_util::Limited::new(body, cap);
    match limited.collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(e) if e.is::<http_body_util::LengthLimitError>() => Err(SparqHttpError::TooLarge),
        Err(_) => Err(SparqHttpError::Body),
    }
}

/// Parse a SPARQL-results-JSON ASK boolean (`{"head":{},"boolean":true}`).
fn parse_ask_json(body: &[u8]) -> Result<bool, SparqHttpError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| SparqHttpError::Malformed(format!("ask json: {e}")))?;
    v.get("boolean")
        .and_then(Value::as_bool)
        .ok_or_else(|| SparqHttpError::Malformed("ask json: missing boolean".into()))
}

/// Parse a SPARQL-results-JSON SELECT into rows of (var → binding value-string).
///
/// Returns the bindings as `Vec<HashMap<var, BindingValue>>`. We only need the `value` (+ `type`) of
/// each binding for the data path (metadata literals + child IRIs), so the shape is kept minimal.
fn parse_select_json(body: &[u8]) -> Result<SelectResult, SparqHttpError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| SparqHttpError::Malformed(format!("select json: {e}")))?;
    let bindings = v
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(Value::as_array)
        .ok_or_else(|| SparqHttpError::Malformed("select json: missing results.bindings".into()))?;
    let mut rows = Vec::with_capacity(bindings.len());
    for b in bindings {
        let obj = b.as_object().ok_or_else(|| {
            SparqHttpError::Malformed("select json: binding not an object".into())
        })?;
        let mut row = std::collections::HashMap::with_capacity(obj.len());
        for (var, cell) in obj {
            let value = cell
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SparqHttpError::Malformed(format!("select json: binding '{var}' has no value"))
                })?
                .to_string();
            row.insert(var.clone(), value);
        }
        rows.push(row);
    }
    Ok(SelectResult { rows })
}

/// A minimal parsed SELECT result — just the binding value strings per row.
#[derive(Debug)]
struct SelectResult {
    rows: Vec<std::collections::HashMap<String, String>>,
}

impl HttpSparqClient {
    async fn put_meta_unchecked(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        let nonce = next_nonce()?;
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let u = sparql::update_put_meta_snapshot_fenced(
            iri,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
            &nonce,
            self.epoch.load(Ordering::Acquire),
            meta.append_seq,
        )?;
        self.write_and_confirm(&u, iri, &nonce, &[iri]).await?;
        self.clear_completion(iri, &nonce).await;
        Ok(())
    }

    /// Confirm every write, including an acknowledged update whose epoch guard matched nothing.
    async fn write_and_confirm(
        &self,
        u: &str,
        iri: &str,
        nonce: &str,
        touches: &[&str],
    ) -> Result<(), SparqError> {
        let fenced = self.update_fenced(u, nonce, touches).await?;
        let (body, _) = self
            .query_raw(&sparql::ask_delete_marker(iri, nonce)?, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        if !parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            return Err(SparqError::Backend(if fenced {
                "fatal: the write did not commit".into()
            } else {
                "fatal: writer epoch superseded".into()
            }));
        }
        Ok(())
    }

    async fn clear_completion(&self, iri: &str, nonce: &str) {
        if let Ok(update) = sparql::update_clear_completion(iri, nonce) {
            // Only this nonce is removed. Its write is final and any required receipt has been read.
            let _ = self.update_raw(&update).await;
        }
    }

    async fn require_epoch(&self, epoch: u64) -> Result<(), SparqError> {
        if self.read_epoch().await? != epoch {
            return Err(SparqError::Backend("fatal: writer epoch superseded".into()));
        }
        Ok(())
    }

    async fn read_epoch(&self) -> Result<u64, SparqError> {
        let (body, _) = self
            .query_raw(&sparql::select_writer_epoch(), ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let rows = parse_select_json(&body)
            .map_err(SparqHttpError::into_sparq)?
            .rows;
        match rows.as_slice() {
            [] => Ok(0),
            [row] => row
                .get("epoch")
                .and_then(|e| e.parse().ok())
                .ok_or_else(|| SparqError::Backend("fatal: invalid writer epoch".into())),
            _ => Err(SparqError::Backend("fatal: multiple writer epochs".into())),
        }
    }
}

#[async_trait]
impl SparqClient for HttpSparqClient {
    fn lifecycle(&self) -> std::sync::Arc<super::Lifecycle> {
        self.lifecycle.clone()
    }

    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        let q = sparql::select_meta(iri)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        // No row ⇒ the resource is not indexed (fail-closed: never invent metadata).
        let row = result.rows.into_iter().next().ok_or(SparqError::NotFound)?;
        let content_type = row
            .get("ct")
            .cloned()
            .ok_or_else(|| SparqError::Backend("fatal: meta row missing contentType".into()))?;
        let blob_key = row
            .get("bk")
            .cloned()
            .ok_or_else(|| SparqError::Backend("fatal: meta row missing blobKey".into()))?;
        let etag = row
            .get("etag")
            .cloned()
            .ok_or_else(|| SparqError::Backend("fatal: meta row missing etag".into()))?;
        // `?mod` is OPTIONAL: absent ⇒ no recorded modification time; present-but-unparseable ⇒
        // `None` too (fail OPEN — never a spurious 304, per `timestamp::from_xsd_datetime`).
        let last_modified = row.get("mod").and_then(|m| timestamp::from_xsd_datetime(m));
        Ok(ResourceMeta {
            content_type,
            blob_key,
            etag,
            last_modified,
            append_seq: row
                .get("appendSeq")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| SparqError::Backend("invalid append sequence".into()))?
                .unwrap_or(0),
        })
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri)])?;
        self.put_meta_unchecked(iri, meta).await
    }

    async fn reset_marker(
        &self,
        iri: &str,
        value: i64,
        lease: &super::LifecycleGuard,
    ) -> Result<(), SparqError> {
        if !lease.belongs_to(&self.lifecycle) {
            return Err(SparqError::Backend(
                "marker reset requires this backend's lifecycle lease".into(),
            ));
        }
        self.put_meta_unchecked(iri, super::sparq::marker_meta(&value.to_string()))
            .await
    }

    async fn fold_appends(
        &self,
        iri: &str,
        expected: &ResourceMeta,
        blob_key: &str,
    ) -> Result<bool, SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri)])?;
        let nonce = next_nonce()?;
        let receipt = format!("urn:x-sparq-lws:append-results/{nonce}");
        let epoch = self.epoch.load(Ordering::Acquire);
        let update = sparql::update_fold_appends(iri, expected, blob_key, &receipt, &nonce, epoch)?;
        let fenced = self.update_fenced(&update, &nonce, &[iri]).await?;
        let (body, _) = self
            .query_raw(
                &sparql::select_append_receipt(&receipt)?,
                ACCEPT_RESULTS_JSON,
            )
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        if result.rows.is_empty() {
            self.require_epoch(epoch).await?;
            return if fenced {
                Err(SparqError::Backend("append fold did not commit".into()))
            } else {
                Ok(false)
            };
        }
        let _ = self
            .update_raw(&sparql::update_delete_resource(&receipt)?)
            .await;
        Ok(true)
    }

    async fn publish_append(&self, iri: &str, key: &str, blob_key: &str) -> Result<(), SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri)])?;
        let nonce = next_nonce()?;
        let receipt = format!("urn:x-sparq-lws:append-results/{nonce}");
        let modified = timestamp::to_xsd_datetime(crate::clock::now())
            .ok_or_else(|| SparqError::Backend("invalid append timestamp".into()))?;
        let epoch = self.epoch.load(Ordering::Acquire);
        let update =
            sparql::update_publish_append(iri, key, blob_key, &nonce, &modified, Some(epoch))?;
        let dir = super::append_dir(key);
        let fenced = self.update_fenced(&update, &nonce, &[iri, &dir]).await?;
        let (body, _) = self
            .query_raw(
                &sparql::select_append_receipt(&receipt)?,
                ACCEPT_RESULTS_JSON,
            )
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        if result.rows.is_empty() {
            self.require_epoch(epoch).await?;
            return Err(if fenced {
                SparqError::Backend("append publication did not commit".into())
            } else {
                SparqError::NotFound
            });
        }
        let _ = self
            .update_raw(&sparql::update_delete_resource(&receipt)?)
            .await;
        Ok(())
    }

    async fn begin_reclaim_with_appends(&self, blob_key: &str) -> Result<(bool, bool), SparqError> {
        self.update_raw(&sparql::update_begin_reclaim(blob_key))
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let (body, _) = self
            .query_raw(
                &sparql::select_reclaim_with_appends(blob_key),
                ACCEPT_RESULTS_JSON,
            )
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        Ok((
            !result.rows.is_empty(),
            result
                .rows
                .iter()
                .any(|row| row.get("hasAppends").is_some_and(|s| s == "true")),
        ))
    }

    async fn begin_reclaim(&self, blob_key: &str) -> Result<bool, SparqError> {
        // One guarded update, then read back (an HTTP update does not say whether its WHERE
        // matched). Read back named by another reclaim of the same bytes is fine: they are
        // unreferenced all the same.
        self.update_raw(&sparql::update_begin_reclaim(blob_key))
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let (body, _ct) = self
            .query_raw(&sparql::ask_reclaiming(blob_key), ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        parse_ask_json(&body).map_err(SparqHttpError::into_sparq)
    }

    async fn end_reclaim(&self, blob_key: &str) -> Result<(), SparqError> {
        self.update_raw(&sparql::update_end_reclaim(blob_key))
            .await
            .map_err(SparqHttpError::into_sparq)
    }

    async fn reclaiming(&self) -> Result<Vec<String>, SparqError> {
        let (body, _ct) = self
            .query_raw(&sparql::select_reclaiming(), ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        Ok(result
            .rows
            .into_iter()
            .filter_map(|row| row.get("k").cloned())
            .collect())
    }

    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        let q = sparql::ask_exists(iri)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        parse_ask_json(&body).map_err(SparqHttpError::into_sparq)
    }

    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri)])?;
        let nonce = next_nonce()?;
        let u =
            sparql::update_delete_resource_fenced(iri, &nonce, self.epoch.load(Ordering::Acquire))?;
        self.write_and_confirm(&u, iri, &nonce, &[iri]).await?;
        self.clear_completion(iri, &nonce).await;
        Ok(())
    }

    async fn delete_resource(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<Option<String>, SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri), parent])?;
        let nonce = next_nonce()?;
        let u = sparql::update_delete_resource_with_parent_fenced(
            iri,
            parent,
            &nonce,
            self.epoch.load(Ordering::Acquire),
        )?;
        let touches: Vec<&str> = std::iter::once(iri).chain(parent).collect();
        self.write_and_confirm(&u, iri, &nonce, &touches).await?;
        // The update captured the prior key atomically; no pre-read can prevent fencing the write.
        let (body, _) = self
            .query_raw(
                &sparql::select_deleted_blob_key(&nonce)?,
                ACCEPT_RESULTS_JSON,
            )
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        let key = result
            .rows
            .first()
            .map(|row| {
                row.get("bk").cloned().ok_or_else(|| {
                    SparqError::Backend("fatal: delete receipt missing blob key".into())
                })
            })
            .transpose()?;
        self.clear_completion(iri, &nonce).await;
        Ok(key)
    }

    async fn begin_epoch(&self, lease: &super::LifecycleGuard) -> Result<u64, SparqError> {
        if !lease.belongs_to(&self.lifecycle) {
            return Err(SparqError::Backend(
                "writer epoch requires this backend's lifecycle lease".into(),
            ));
        }
        let epoch = self
            .read_epoch()
            .await?
            .checked_add(1)
            .ok_or_else(|| SparqError::Backend("fatal: writer epoch exhausted".into()))?;
        // Only raises: even a timed-out start's delayed update cannot lower a later epoch.
        let owner = next_nonce()?;
        self.update_raw(&sparql::update_begin_epoch(epoch, &owner))
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let (body, _) = self
            .query_raw(
                &sparql::ask_writer_epoch_owner(epoch, &owner),
                ACCEPT_RESULTS_JSON,
            )
            .await
            .map_err(SparqHttpError::into_sparq)?;
        if !parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            return Err(SparqError::Backend("fatal: writer epoch superseded".into()));
        }
        self.epoch.store(epoch, Ordering::Release);
        Ok(epoch)
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        super::limits::refuse_marker_operands(&[Some(iri), parent])?;
        // A per-operation nonce so the success confirm is RACE-RESISTANT — no concurrent op writes or
        // removes THIS delete marker, so a containment mutation between the update and the confirm
        // cannot flip the result (the same pattern as the create-marker).
        let nonce = next_nonce()?;

        // ONE SPARQL 1.1 `DELETE { } INSERT { } WHERE { }` modify: its WHERE (container-EXISTS AND
        // `ldp:contains`-empty) is evaluated ONCE pre-modification, then the multi-graph DELETE (the
        // container's own graph AND, when `parent` is given, the `<parent> ldp:contains <container>`
        // edge in the parent's graph) and the marker INSERT apply atomically against that one solution.
        // A non-empty (or absent) container ⇒ the WHERE yields nothing ⇒ NOTHING is deleted or detached
        // (the safety invariant). Folding the parent-edge detach in here (rather than a separate later
        // `remove_child`) closes the window where a concurrent POST recreates the child and a stale
        // detach orphans it.
        //
        // An update that fails but may still commit is fenced ([`Self::update_fenced`]), so what
        // the ASKs below read is final.
        let epoch = self.epoch.load(Ordering::Acquire);
        let u = sparql::update_delete_container_if_empty_fenced(iri, parent, &nonce, epoch)?;
        let touches: Vec<&str> = std::iter::once(iri).chain(parent).collect();
        let fenced = self.update_fenced(&u, &nonce, &touches).await?;

        // Disambiguate the outcome with bounded follow-up ASKs (the protocol can't return rows from an
        // UPDATE; documented). FIRST: did the guard match (⇒ the delete ran)? ASK for OUR marker — a
        // nonce nothing else touches, so this is immune to a concurrent containment mutation.
        let marker_q = sparql::ask_delete_marker(iri, &nonce)?;
        let (body, _ct) = self
            .query_raw(&marker_q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        if parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            self.clear_completion(iri, &nonce).await;
            return Ok(DeleteOutcome::Deleted);
        }
        self.require_epoch(epoch).await?;
        // Marker absent ⇒ the delete did NOT run (container was non-empty OR absent). Split the two by
        // a single ASK on the container's own record: present ⇒ NotEmpty (it had members, so the empty
        // guard blocked the delete), absent ⇒ NotFound (it was never indexed). Fail-closed.
        let exists_q = sparql::ask_exists(iri)?;
        let (body, _ct) = self
            .query_raw(&exists_q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        match parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            // A fenced delete that did not run may have been fenced before it ran, not refused
            // for members: what is known is only that it did not run (and never will).
            true if fenced => Err(SparqError::Backend(
                "fatal: the delete failed and did not run".into(),
            )),
            true => Ok(DeleteOutcome::NotEmpty),
            false => Ok(DeleteOutcome::NotFound),
        }
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        super::limits::refuse_marker_operands(&[Some(container), Some(child)])?;
        // A per-operation unique nonce written atomically with the guarded create, so the success
        // confirm is RACE-RESISTANT: no concurrent remove_child / sibling create ever touches this
        // marker, so a containment-edge mutation between the update and the confirm cannot flip the
        // result (the round-4 finding — confirming on the mutable containment edge was racy).
        let nonce = next_nonce()?;

        // ONE atomic update: DELETE any stale child record + marker, INSERT the child record + the
        // containment edge + THIS operation's marker, guarded by the container-record EXISTS in the
        // WHERE clause. A missing container ⇒ the WHERE yields nothing ⇒ nothing inserted.
        //
        // An update that fails but may still commit is fenced ([`Self::update_fenced`]), so what
        // the ASK below reads is final.
        let epoch = self.epoch.load(Ordering::Acquire);
        let u = sparql::update_create_child_meta(container, child, &meta, &nonce, Some(epoch))?;
        let fenced = self.update_fenced(&u, &nonce, &[container, child]).await?;

        // CONFIRM the guard matched by ASKing for OUR marker (not the shared containment edge):
        // the marker is present iff the guarded INSERT ran (container existed) AND no later op removed
        // it — and nothing else writes/removes this nonce, so the check is immune to a concurrent
        // containment mutation. A missing marker ⇒ the container was absent ⇒ NotFound. Fail closed.
        let q = sparql::ask_create_marker(child, &nonce)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let created = parse_ask_json(&body).map_err(SparqHttpError::into_sparq)?;
        if created {
            self.clear_completion(child, &nonce).await;
            Ok(())
        } else {
            self.require_epoch(epoch).await?;
            if fenced {
                // Fenced before it ran, or refused for a missing container: it did not commit, and
                // never will.
                Err(SparqError::Backend(
                    "fatal: the create failed and did not commit".into(),
                ))
            } else {
                Err(SparqError::NotFound)
            }
        }
    }

    async fn fence(&self, fence: &super::sparq::Fence) -> Result<(), SparqError> {
        self.update_raw(&sparql::update_fence(fence.nonce()))
            .await
            .map_err(SparqHttpError::into_sparq)
    }

    /// The per-request timeout this client was built with ([`HttpSparqClient::with_timeout`]).
    fn call_timeout(&self) -> Duration {
        self.timeout
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        super::limits::refuse_marker_operands(&[Some(container), Some(child)])?;
        let nonce = next_nonce()?;
        let u = sparql::update_remove_child_fenced(
            container,
            child,
            &nonce,
            self.epoch.load(Ordering::Acquire),
        )?;
        self.write_and_confirm(&u, child, &nonce, &[container, child])
            .await?;
        self.clear_completion(child, &nonce).await;
        Ok(())
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        let children = self.children(sparql::select_children(container)?, usize::MAX, None);
        Ok(children.await?.unwrap_or_default())
    }

    async fn list_children_within(
        &self,
        container: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<String>>, SparqError> {
        let rows = super::sparq::max_rows(container, max_bytes);
        let q = sparql::select_children_limited(container, rows.saturating_add(1))?;
        self.children(q, max_bytes, Some(rows)).await
    }

    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        // ONE `SELECT DISTINCT ?bk` over every graph's `pss:blobKey` — the whole referenced set in a
        // single round-trip (see the builder doc). No untrusted input, so the query is infallible.
        let q = sparql::select_referenced_blob_keys();
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        let mut keys = std::collections::HashSet::with_capacity(result.rows.len());
        for row in result.rows {
            // A `SELECT ?bk` row MUST carry the `bk` binding. A row missing it is a malformed backend
            // response — surface it as a FATAL error, never silently drop it: a silently-shortened
            // referenced set would make the reconciler think a still-referenced blob is an orphan and
            // DELETE live bytes. Fail-closed (the reconciler aborts the whole sweep on this error).
            match row.get("bk") {
                Some(bk) => {
                    keys.insert(bk.clone());
                }
                None => {
                    return Err(SparqHttpError::Malformed(
                        "referenced-blob-keys row missing the 'bk' binding".into(),
                    )
                    .into_sparq())
                }
            }
        }
        Ok(keys)
    }

    async fn read_plan(
        &self,
        target: &str,
        acl_candidates: &[String],
    ) -> Result<ReadPlan, SparqError> {
        // read-2 (§3.1): the ENTIRE per-read metadata chain — the target's record + every ACL
        // candidate's presence/etag — in ONE combined SELECT (balanced branches or VALUES), replacing the
        // k+2 sequential per-IRI queries. Fail-closed: a transport/backend/malformed-response
        // failure fails the WHOLE plan (no per-candidate partial degrade — design invariant 4).
        let q = sparql::select_read_plan(target, acl_candidates)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;

        // Key the rows by graph IRI. Every row of this SELECT must carry all four bindings — a row
        // missing one is a malformed backend response (fatal), never silently dropped (dropping
        // could hide a PRESENT own-ACL and wrongly inherit an ancestor's grants — fail-closed).
        let mut by_graph: std::collections::HashMap<String, ResourceMeta> =
            std::collections::HashMap::with_capacity(result.rows.len());
        for row in result.rows {
            let bind = |var: &str| -> Result<String, SparqError> {
                row.get(var).cloned().ok_or_else(|| {
                    SparqHttpError::Malformed(format!("read-plan row missing the '{var}' binding"))
                        .into_sparq()
                })
            };
            let g = bind("g")?;
            // `?mod` is OPTIONAL: absent/unparseable ⇒ `None` (fail open — the ACL candidates only
            // need the etag anyway; the target's `last_modified` feeds `If-Modified-Since`).
            let last_modified = row.get("mod").and_then(|m| timestamp::from_xsd_datetime(m));
            let meta = ResourceMeta {
                content_type: bind("ct")?,
                blob_key: bind("bk")?,
                etag: bind("etag")?,
                last_modified,
                append_seq: row
                    .get("appendSeq")
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| SparqError::Backend("invalid append sequence".into()))?
                    .unwrap_or(0),
            };
            // First row per graph wins (a well-formed index holds exactly one record per graph —
            // `update_put_meta`/`update_create_child` keep the record single-valued; this mirrors
            // `select_meta`'s LIMIT 1 determinism).
            by_graph.entry(g).or_insert(meta);
        }

        Ok(ReadPlan {
            target: by_graph.get(target).cloned(),
            acls: acl_candidates
                .iter()
                .map(|c| (c.clone(), by_graph.get(c).map(|m| m.etag.clone())))
                .collect(),
        })
    }
}

impl HttpSparqClient {
    /// CONSTRUCT a resource's RDF body as bytes (N-Triples), bypassing the metadata record. Exposed
    /// for the LDP read path that wants the resource graph (not just the byte-pointer). The body is
    /// the resource's triples serialised as N-Triples (`application/n-triples`).
    ///
    /// This is part of the data path the trait does not (yet) surface — the composite store reads
    /// bytes from the blob store today — but the live client implements it so a future SPARQ-native
    /// read (CONSTRUCT over the named graph) plugs in without re-deriving the query. M3-next: surface
    /// this on the [`SparqClient`] trait when the read path moves from blob bytes to SPARQ CONSTRUCT.
    pub async fn construct_resource_ntriples(&self, iri: &str) -> Result<Bytes, SparqError> {
        let q = sparql::construct_resource(iri)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_NTRIPLES)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        Ok(body)
    }

    /// INSERT a resource's parsed body triples (already-validated RDF) into its named graph, as one
    /// `INSERT DATA`. Each term flows through the injection-safe builders. An empty triple set is a
    /// no-op (no request issued).
    ///
    /// This is the write companion to [`HttpSparqClient::construct_resource_ntriples`]: together they are a real
    /// SPARQ-native body round-trip (insert → construct), exercising the [`sparql::insert_body_data`]
    /// builder end-to-end. The composite store still keeps resource BYTES in the blob store today, so
    /// the trait does not yet route writes through here; this is the seam a future SPARQ-native body
    /// store plugs into. M3-next: surface on the [`SparqClient`] trait when the write path moves from
    /// blob bytes to SPARQ INSERT DATA.
    pub async fn insert_body(
        &self,
        iri: &str,
        triples: &[(String, String, super::BodyObject)],
    ) -> Result<(), SparqError> {
        let update = sparql::insert_body_data(iri, triples)?;
        if update.is_empty() {
            return Ok(()); // nothing to insert
        }
        self.update_raw(&update)
            .await
            .map_err(SparqHttpError::into_sparq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> ResourceMeta {
        ResourceMeta {
            content_type: "text/plain".into(),
            blob_key: "candidate".into(),
            append_seq: 0,
            etag: "e".into(),
            last_modified: None,
        }
    }

    #[tokio::test]
    async fn timed_out_index_writes_retain_their_fences_and_touched_resources() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = HttpSparqClient::with_timeout(
            format!("http://{}/sparql", listener.local_addr().unwrap()),
            Duration::from_millis(20),
        );
        let (iri, parent) = ("https://pod.example/c/d", "https://pod.example/c/");
        for operation in [
            "put",
            "delete",
            "detach",
            "delete_resource",
            "append",
            "fold",
        ] {
            let result = match operation {
                "put" => client.put_meta(iri, meta()).await,
                "delete" => client.delete_meta(iri).await,
                "detach" => client.remove_child(parent, iri).await,
                "append" => client.publish_append(iri, "candidate", "insert").await,
                "fold" => client
                    .fold_appends(iri, &meta(), "folded")
                    .await
                    .map(|_| ()),
                "delete_resource" => client.delete_resource(iri, Some(parent)).await.map(|_| ()),
                _ => unreachable!(),
            };
            let Err(SparqError::Unsettled { fence, .. }) = result else {
                panic!("{operation}: {result:?}");
            };
            let mut touches = fence.touches().to_vec();
            touches.sort();
            let mut expected = vec![iri.to_string()];
            if matches!(operation, "detach" | "delete_resource") {
                expected.push(parent.to_string());
            }
            if operation == "append" {
                expected.push(super::super::append_dir("candidate"));
            }
            expected.sort();
            assert_eq!(touches, expected, "{operation}");
            assert!(!fence.nonce().is_empty());
        }
    }

    #[tokio::test]
    async fn an_http_write_names_the_clients_epoch() {
        use axum::{extract::State, response::IntoResponse, routing::post, Router};
        let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        async fn endpoint(
            State(requests): State<Arc<std::sync::Mutex<Vec<String>>>>,
            headers: http::HeaderMap,
            body: Bytes,
        ) -> axum::response::Response {
            requests
                .lock()
                .unwrap()
                .push(String::from_utf8(body.to_vec()).unwrap());
            if headers[header::CONTENT_TYPE] == CT_SPARQL_UPDATE {
                StatusCode::NO_CONTENT.into_response()
            } else {
                r#"{"boolean":true}"#.into_response()
            }
        }
        let app = Router::new()
            .route("/sparql", post(endpoint))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = HttpSparqClient::with_timeout(
            format!("http://{}/sparql", listener.local_addr().unwrap()),
            Duration::from_secs(2),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        client.epoch.store(7, Ordering::Release);
        let clone = client.clone();
        clone
            .put_meta("https://pod.example/d", meta())
            .await
            .unwrap();
        server.abort();
        let requests = requests.lock().unwrap();
        let request = &requests[0];
        let where_clause = request.split("WHERE {").last().unwrap();
        assert!(
            where_clause.contains("<urn:x-sparq-lws:writer-epoch> <urn:pss:index#epoch> 7"),
            "{request}"
        );
    }

    #[cfg(feature = "embedded-sparq")]
    async fn engine_endpoint() -> (
        HttpSparqClient,
        Arc<std::sync::Mutex<sparq_core::Graph>>,
        tokio::task::JoinHandle<()>,
    ) {
        engine_endpoint_with_epoch_barrier(None).await
    }

    #[cfg(feature = "embedded-sparq")]
    async fn engine_endpoint_with_epoch_barrier(
        barrier: Option<Arc<tokio::sync::Barrier>>,
    ) -> (
        HttpSparqClient,
        Arc<std::sync::Mutex<sparq_core::Graph>>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{extract::State, response::IntoResponse, routing::post, Router};
        type Graph = Arc<std::sync::Mutex<sparq_core::Graph>>;
        async fn endpoint(
            State((graph, barrier)): State<(Graph, Option<Arc<tokio::sync::Barrier>>)>,
            headers: http::HeaderMap,
            body: Bytes,
        ) -> axum::response::Response {
            let text = std::str::from_utf8(&body).unwrap();
            let response = {
                let mut graph = graph.lock().unwrap();
                if headers[header::CONTENT_TYPE] == CT_SPARQL_UPDATE {
                    sparq_engine::update_in_place_atomic(&mut graph, text).unwrap();
                    StatusCode::NO_CONTENT.into_response()
                } else {
                    sparq_engine::query_json(&graph, text)
                        .unwrap()
                        .into_response()
                }
            };
            // Both starts read the old epoch before either is allowed to send its raise.
            if text.starts_with("SELECT ?epoch") {
                if let Some(barrier) = barrier {
                    barrier.wait().await;
                }
            }
            response
        }
        let graph = Arc::new(std::sync::Mutex::new(
            sparq_core::Graph::load_str("", "ntriples").unwrap(),
        ));
        let app = Router::new()
            .route("/sparql", post(endpoint))
            .with_state((graph.clone(), barrier));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = HttpSparqClient::with_timeout(
            format!("http://{}/sparql", listener.local_addr().unwrap()),
            Duration::from_secs(2),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (client, graph, server)
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn append_and_fold_follow_the_writer_epoch_and_clear_their_receipts() {
        let (client, graph, server) = engine_endpoint().await;
        let (parent, child) = ("https://pod.example/c/", "https://pod.example/c/d");
        client.put_meta(parent, meta()).await.unwrap();
        let lease = client.lifecycle().acquire().unwrap();
        let epoch = client.begin_epoch(&lease).await.unwrap();
        let original = ResourceMeta {
            content_type: "application/n-triples".into(),
            append_seq: 3,
            ..meta()
        };
        client
            .create_child(parent, child, original.clone())
            .await
            .unwrap();
        assert_eq!(client.get_meta(child).await.unwrap(), original.clone());
        // A complete snapshot written through either path retains its sequence.
        client.put_meta(child, original.clone()).await.unwrap();
        assert_eq!(client.get_meta(child).await.unwrap(), original);
        client
            .publish_append(child, "candidate", "insert")
            .await
            .unwrap();
        let appended = client.get_meta(child).await.unwrap();
        assert!(appended.append_seq > 3);
        assert_ne!(appended.etag, "e");
        assert!(client
            .fold_appends(child, &appended, "folded")
            .await
            .unwrap());
        let folded = client.get_meta(child).await.unwrap();
        assert_eq!(
            folded,
            ResourceMeta {
                blob_key: "folded".into(),
                append_seq: 0,
                ..appended.clone()
            }
        );
        assert!(!client
            .fold_appends(child, &appended, "stale-fold")
            .await
            .unwrap());
        let snapshot = || {
            sparq_engine::query_json(
                &graph.lock().unwrap(),
                "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g ?s ?p ?o",
            )
            .unwrap()
        };
        assert!(
            !snapshot().contains("append-results/"),
            "confirmed receipts cleaned up"
        );
        client
            .update_raw(&sparql::update_begin_epoch(epoch + 1, "later-start"))
            .await
            .unwrap();
        let before = snapshot();
        for operation in ["append", "fold", "absent-append", "stale-fold"] {
            let result = match operation {
                "append" => client.publish_append(child, "folded", "later-insert").await,
                "fold" => client
                    .fold_appends(child, &folded, "later-fold")
                    .await
                    .map(|_| ()),
                "absent-append" => {
                    client
                        .publish_append(child, "absent-key", "absent-insert")
                        .await
                }
                "stale-fold" => client
                    .fold_appends(child, &appended, "stale-fold")
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            };
            assert!(
                matches!(result, Err(SparqError::Backend(ref error)) if error == "fatal: writer epoch superseded"),
                "{result:?}"
            );
            assert_eq!(snapshot(), before);
        }
        server.abort();
    }

    #[cfg(feature = "embedded-sparq")]
    async fn refuse_superseded(operations: &[&str]) {
        for epoch in [0, 1] {
            let (client, graph, server) = engine_endpoint().await;
            let (parent, child, absent) = (
                "https://pod.example/c/",
                "https://pod.example/c/d",
                "https://pod.example/absent/",
            );
            client.put_meta(parent, meta()).await.unwrap();
            client.create_child(parent, child, meta()).await.unwrap();
            client.epoch.store(epoch, Ordering::Release);
            client
                .update_raw(&sparql::update_begin_epoch(epoch + 1, "later-start"))
                .await
                .unwrap();
            let snapshot = || {
                sparq_engine::query_json(
                    &graph.lock().unwrap(),
                    "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g ?s ?p ?o",
                )
                .unwrap()
            };
            let before = snapshot();
            for &operation in operations {
                let result = match operation {
                    "put" => {
                        client
                            .put_meta(
                                child,
                                ResourceMeta {
                                    blob_key: "replacement".into(),
                                    ..meta()
                                },
                            )
                            .await
                    }
                    "delete" => client.delete_meta(child).await,
                    "detach" => client.remove_child(parent, child).await,
                    "delete_resource" => client
                        .delete_resource(child, Some(parent))
                        .await
                        .map(|_| ()),
                    "delete_resource_parentless" => {
                        client.delete_resource(child, None).await.map(|_| ())
                    }
                    "empty_delete_nonempty" => {
                        client.delete_meta_if_empty(parent, None).await.map(|_| ())
                    }
                    "create" => client.create_child(parent, child, meta()).await,
                    "create_absent" => client.create_child(absent, child, meta()).await,
                    "empty_delete" => client
                        .delete_meta_if_empty(child, Some(parent))
                        .await
                        .map(|_| ()),
                    "empty_delete_absent" => {
                        client.delete_meta_if_empty(absent, None).await.map(|_| ())
                    }
                    _ => unreachable!(),
                };
                assert!(
                    matches!(result, Err(SparqError::Backend(ref error)) if error == "fatal: writer epoch superseded"),
                    "{operation}: {result:?}"
                );
                assert_eq!(snapshot(), before, "{operation} changed the index");
            }
            server.abort();
        }
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn superseded_writers_refuse_every_index_mutation_without_changing_the_index() {
        refuse_superseded(&[
            "put",
            "delete",
            "detach",
            "delete_resource",
            "create",
            "empty_delete",
            "create_absent",
            "empty_delete_absent",
            "empty_delete_nonempty",
            "delete_resource_parentless",
        ])
        .await;
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn superseded_child_creates_report_refusal_before_missing_container() {
        refuse_superseded(&["create", "create_absent"]).await;
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn superseded_container_deletes_report_refusal_before_empty_or_absent() {
        refuse_superseded(&[
            "empty_delete",
            "empty_delete_absent",
            "empty_delete_nonempty",
        ])
        .await;
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn confirmed_writes_remove_their_markers_and_delete_receipts() {
        let (client, graph, server) = engine_endpoint().await;
        let (parent, child) = ("https://pod.example/c/", "https://pod.example/c/d");
        let assert_clean = || {
            assert!(!sparq_engine::ask(&graph.lock().unwrap(),
                "ASK { GRAPH ?g { ?s ?p ?o } FILTER (?p IN (<urn:pss:index#createMarker>, <urn:pss:index#deleteMarker>, <urn:pss:index#deletedBlobKey>)) }").unwrap());
        };
        client.put_meta(parent, meta()).await.unwrap();
        assert_clean();
        client.create_child(parent, child, meta()).await.unwrap();
        assert_clean();
        client.remove_child(parent, child).await.unwrap();
        assert_clean();
        client.delete_meta(child).await.unwrap();
        assert_clean();
        client.create_child(parent, child, meta()).await.unwrap();
        assert_eq!(
            client.delete_resource(child, Some(parent)).await.unwrap(),
            Some("candidate".into())
        );
        assert_clean();
        client.create_child(parent, child, meta()).await.unwrap();
        assert_eq!(
            client
                .delete_meta_if_empty(child, Some(parent))
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert_clean();
        server.abort();
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn completion_cleanup_retains_other_operations_evidence() {
        let (client, graph, server) = engine_endpoint().await;
        let (parent, child) = ("https://pod.example/c/", "https://pod.example/c/d");
        client.put_meta(parent, meta()).await.unwrap();
        client
            .update_raw(
                &sparql::update_create_child_fenced(
                    parent,
                    child,
                    "text/plain",
                    "candidate",
                    "e",
                    None,
                    "pending-create",
                    0,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        client.create_child(parent, child, meta()).await.unwrap();
        assert!(sparq_engine::ask(
            &graph.lock().unwrap(),
            &sparql::ask_create_marker(child, "pending-create").unwrap()
        )
        .unwrap());
        client
            .update_raw(&sparql::update_delete_resource_fenced(child, "pending-delete", 0).unwrap())
            .await
            .unwrap();
        assert_eq!(
            client.delete_resource(child, Some(parent)).await.unwrap(),
            None
        );
        assert!(sparq_engine::ask(
            &graph.lock().unwrap(),
            &sparql::ask_delete_marker(child, "pending-delete").unwrap()
        )
        .unwrap());
        let receipt = sparq_engine::query(
            &graph.lock().unwrap(),
            &sparql::select_deleted_blob_key("pending-delete").unwrap(),
        )
        .unwrap();
        assert_eq!(
            receipt.rows,
            vec![vec![Some(
                oxrdf::Literal::new_simple_literal("candidate").into()
            )]]
        );
        server.abort();
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn simultaneous_starts_confirm_only_the_owner_of_the_epoch() {
        use crate::store::{CompositeStore, InMemoryBlobStore, Store};
        let (first, graph, server) =
            engine_endpoint_with_epoch_barrier(Some(Arc::new(tokio::sync::Barrier::new(2)))).await;
        let second = HttpSparqClient::new(first.endpoint.clone());
        let first_store = CompositeStore::new(first.clone(), InMemoryBlobStore::new());
        let second_store = CompositeStore::new(second.clone(), InMemoryBlobStore::new());
        let first_lease = first_store.lifecycle().acquire().unwrap();
        let second_lease = second_store.lifecycle().acquire().unwrap();
        let (a, b) = tokio::join!(
            first.begin_epoch(&first_lease),
            second.begin_epoch(&second_lease)
        );
        assert_ne!(a.is_ok(), b.is_ok(), "{a:?}; {b:?}");
        let (winner, loser, refusal) = if a.is_ok() {
            (&first, &second, b)
        } else {
            (&second, &first, a)
        };
        assert!(
            matches!(refusal, Err(SparqError::Backend(ref error)) if error == "fatal: writer epoch superseded")
        );
        assert_eq!(winner.epoch.load(Ordering::Acquire), 1);
        assert_eq!(loser.epoch.load(Ordering::Acquire), 0);
        let iri = "https://pod.example/d";
        winner.put_meta(iri, meta()).await.unwrap();
        assert!(matches!(
            loser.put_meta(iri, meta()).await,
            Err(SparqError::Backend(_))
        ));
        assert_eq!(winner.get_meta(iri).await.unwrap().blob_key, "candidate");
        let rows = sparq_engine::query(&graph.lock().unwrap(), &sparql::select_writer_epoch())
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 1);
        server.abort();
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn resource_deletes_return_the_key_captured_by_their_own_update() {
        let (client, _, server) = engine_endpoint().await;
        let (parent, child, sibling) = (
            "https://pod.example/c/",
            "https://pod.example/c/d",
            "https://pod.example/c/sibling",
        );
        client.put_meta(parent, meta()).await.unwrap();
        client.create_child(parent, sibling, meta()).await.unwrap();
        for key in ["first-key", "second-key"] {
            client
                .create_child(
                    parent,
                    child,
                    ResourceMeta {
                        blob_key: key.into(),
                        ..meta()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                client.delete_resource(child, Some(parent)).await.unwrap(),
                Some(key.into())
            );
            assert!(!client.exists(child).await.unwrap());
            assert_eq!(client.list_children(parent).await.unwrap(), [sibling]);
            assert_eq!(
                client.delete_resource(child, Some(parent)).await.unwrap(),
                None,
                "an absent resource must not reuse a prior operation's receipt"
            );
        }
        client.create_child(parent, child, meta()).await.unwrap();
        client.delete_meta(child).await.unwrap();
        assert!(client
            .list_children(parent)
            .await
            .unwrap()
            .iter()
            .any(|c| c == child));
        assert_eq!(
            client.delete_resource(child, Some(parent)).await.unwrap(),
            None,
            "a legacy edge has no resource key"
        );
        assert_eq!(client.list_children(parent).await.unwrap(), [sibling]);
        assert_eq!(client.get_meta(parent).await.unwrap().blob_key, "candidate");
        assert_eq!(
            client.get_meta(sibling).await.unwrap().blob_key,
            "candidate"
        );
        server.abort();
    }

    #[tokio::test]
    async fn beginning_an_epoch_requires_its_backends_lease_before_any_http_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = HttpSparqClient::with_timeout(
            format!("http://{}/sparql", listener.local_addr().unwrap()),
            Duration::from_millis(50),
        );
        let other = HttpSparqClient::new(client.endpoint.clone());
        let lease = other.lifecycle().acquire().unwrap();
        assert!(matches!(
            client.begin_epoch(&lease).await,
            Err(SparqError::Backend(ref message))
                if message == "writer epoch requires this backend's lifecycle lease"
        ));
        assert_eq!(client.epoch.load(Ordering::Acquire), 0);
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    }

    #[cfg(feature = "embedded-sparq")]
    #[tokio::test]
    async fn beginning_an_epoch_through_store_wrappers_updates_clones_and_finalizes_prior_writes() {
        use crate::store::{CompositeStore, CountingSparqClient, InMemoryBlobStore, Store};
        let (client, graph, server) = engine_endpoint().await;
        let clone = client.clone();
        let store = CompositeStore::new(
            CountingSparqClient::new(client, Arc::default()),
            InMemoryBlobStore::new(),
        );
        let lease = store.lifecycle().acquire().unwrap();
        assert_eq!(store.begin_epoch(&lease).await.unwrap(), 1);
        assert_eq!(clone.epoch.load(Ordering::Acquire), 1);
        let iri = "https://pod.example/d";
        let prior = sparql::update_put_meta_fenced(
            iri,
            "text/plain",
            "old-process",
            "old",
            None,
            "op-old",
            1,
        )
        .unwrap();
        drop(lease);
        let lease = store.lifecycle().acquire().unwrap();
        assert_eq!(store.begin_epoch(&lease).await.unwrap(), 2);
        assert_eq!(clone.epoch.load(Ordering::Acquire), 2);
        clone.put_meta(iri, meta()).await.unwrap();
        clone.update_raw(&prior).await.unwrap();
        assert_eq!(clone.get_meta(iri).await.unwrap().blob_key, "candidate");
        assert!(!sparq_engine::ask(
            &graph.lock().unwrap(),
            &sparql::ask_delete_marker(iri, "op-old").unwrap()
        )
        .unwrap());
        server.abort();
    }

    /// Review of #7174: a background fence's attempts were capped at the default call timeout
    /// whatever the client was built with. The client, and a store over it, say the timeout it
    /// was built with.
    #[test]
    fn the_call_timeout_is_the_one_configured() {
        use crate::store::{CompositeStore, InMemoryBlobStore, Store};
        let default = HttpSparqClient::new("http://127.0.0.1:9/sparql");
        assert_eq!(default.call_timeout(), super::super::STORE_CALL_TIMEOUT);
        let longer = Duration::from_secs(90);
        let client = HttpSparqClient::with_timeout("http://127.0.0.1:9/sparql", longer);
        assert_eq!(client.call_timeout(), longer);
        let store = CompositeStore::new(client, InMemoryBlobStore::new());
        assert_eq!(Store::call_timeout(&store), longer);
    }

    /// Review finding: a body past the cap and a body that failed part way were one error, so a
    /// connection reset on a small listing read as a listing too large to keep (`507`). They are
    /// told apart.
    #[tokio::test]
    async fn a_body_past_the_cap_is_not_a_body_that_failed() {
        struct Reset;
        impl hyper::body::Body for Reset {
            type Data = Bytes;
            type Error = std::io::Error;
            fn poll_frame(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>>
            {
                std::task::Poll::Ready(Some(Err(std::io::Error::other("connection reset"))))
            }
        }
        assert!(matches!(
            read_bounded(Reset, 1 << 20).await,
            Err(SparqHttpError::Body)
        ));
        let big = Full::new(Bytes::from(vec![b'x'; 64]));
        assert!(matches!(
            read_bounded(big, 16).await,
            Err(SparqHttpError::TooLarge)
        ));
        let small = Full::new(Bytes::from_static(b"ok"));
        assert_eq!(
            read_bounded(small, 16).await.unwrap(),
            Bytes::from_static(b"ok")
        );
    }

    #[test]
    fn ask_json_parses_true_and_false() {
        assert!(parse_ask_json(br#"{"head":{},"boolean":true}"#).unwrap());
        assert!(!parse_ask_json(br#"{"head":{},"boolean":false}"#).unwrap());
    }

    #[test]
    fn ask_json_missing_boolean_is_malformed() {
        let err = parse_ask_json(br#"{"head":{}}"#).unwrap_err();
        assert!(matches!(err, SparqHttpError::Malformed(_)));
        assert!(!err.is_retryable());
    }

    #[test]
    fn select_json_parses_bindings() {
        let body = br#"{"head":{"vars":["ct","bk","etag"]},"results":{"bindings":[
            {"ct":{"type":"literal","value":"text/turtle"},
             "bk":{"type":"literal","value":"k1"},
             "etag":{"type":"literal","value":"\"e1\""}}
        ]}}"#;
        let r = parse_select_json(body).unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].get("ct").unwrap(), "text/turtle");
        assert_eq!(r.rows[0].get("bk").unwrap(), "k1");
        assert_eq!(r.rows[0].get("etag").unwrap(), "\"e1\"");
    }

    #[test]
    fn select_json_empty_bindings_is_zero_rows() {
        let body = br#"{"head":{"vars":["child"]},"results":{"bindings":[]}}"#;
        let r = parse_select_json(body).unwrap();
        assert!(r.rows.is_empty());
    }

    #[test]
    fn select_json_missing_results_is_malformed() {
        let err = parse_select_json(br#"{"head":{"vars":[]}}"#).unwrap_err();
        assert!(matches!(err, SparqHttpError::Malformed(_)));
    }

    #[test]
    fn retryability_classification() {
        assert!(SparqHttpError::Transport("refused".into()).is_retryable());
        assert!(SparqHttpError::Timeout(Duration::from_secs(1)).is_retryable());
        assert!(SparqHttpError::ServerStatus { status: 503 }.is_retryable());
        assert!(!SparqHttpError::ClientStatus { status: 400 }.is_retryable());
        assert!(!SparqHttpError::Malformed("x".into()).is_retryable());
        assert!(!SparqHttpError::Body.is_retryable());
        assert!(!SparqHttpError::TooLarge.is_retryable());
        assert!(!SparqHttpError::Build("x".into()).is_retryable());
    }

    /// Issue #6836: an update that timed out (or whose connection failed, or that
    /// `sparq-server` answered `503` when its own wait elapsed) may still commit; only a refusal
    /// is final.
    #[test]
    fn an_update_that_may_still_commit_is_told_from_a_refused_one() {
        assert!(SparqHttpError::Timeout(Duration::from_secs(1)).may_commit_later());
        assert!(SparqHttpError::Transport("reset".into()).may_commit_later());
        assert!(SparqHttpError::ServerStatus { status: 503 }.may_commit_later());
        assert!(SparqHttpError::ServerStatus { status: 500 }.may_commit_later());
        assert!(SparqHttpError::Body.may_commit_later());
        assert!(SparqHttpError::TooLarge.may_commit_later());
        assert!(!SparqHttpError::ClientStatus { status: 400 }.may_commit_later());
        assert!(!SparqHttpError::ClientStatus { status: 501 }.may_commit_later());
        assert!(!SparqHttpError::Build("x".into()).may_commit_later());
        assert!(!SparqHttpError::Malformed("x".into()).may_commit_later());
    }

    #[test]
    fn into_sparq_preserves_retryability_hint() {
        let s = SparqHttpError::ServerStatus { status: 500 }.into_sparq();
        match s {
            SparqError::Backend(msg) => assert!(msg.starts_with("retryable:"), "got: {msg}"),
            _ => panic!("expected Backend"),
        }
        let f = SparqHttpError::ClientStatus { status: 400 }.into_sparq();
        match f {
            SparqError::Backend(msg) => assert!(msg.starts_with("fatal:"), "got: {msg}"),
            _ => panic!("expected Backend"),
        }
    }
}

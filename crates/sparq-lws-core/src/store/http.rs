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
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const CT_SPARQL_QUERY: &str = "application/sparql-query";
const CT_SPARQL_UPDATE: &str = "application/sparql-update";
const ACCEPT_RESULTS_JSON: &str = "application/sparql-results+json";
const ACCEPT_NTRIPLES: &str = "application/n-triples";

/// A process-unique nonce for the race-resistant create marker: a monotonic counter combined with
/// the boot-time nanos, so two `create_child`s (even same-IRI) get distinct markers and the success
/// confirm is operation-scoped. Not security-sensitive (it gates no auth) — only uniqueness matters.
fn next_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("op-{nanos:x}-{n:x}")
}

/// A typed SPARQ-over-HTTP error, carrying whether the failure is worth retrying.
///
/// The split is load-bearing for the composite store's crash-consistency model: a *retryable*
/// failure (connect refused, a 5xx, a timeout) is a transient backend blip the caller may re-issue;
/// a *fatal* failure (a 4xx, a malformed/unparseable response) is a bug or a permanent rejection that
/// retrying cannot fix. Both map onto [`SparqError::Backend`] at the trait boundary (the trait keeps
/// the backend opaque), but the retryability is preserved in the message + queryable here so a future
/// retry layer (M3-next) can act on it.
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
    /// The response body exceeded `MAX_RESPONSE_BYTES` or could not be read — fatal.
    #[error("response too large or unreadable")]
    Body,
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
    timeout: Duration,
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
            timeout,
        }
    }

    /// Issue a SPARQL **query**, returning the raw body bytes + the response `Content-Type`. The
    /// `accept` header drives the result serialisation.
    async fn query_raw(
        &self,
        sparql: &str,
        accept: &str,
    ) -> Result<(Bytes, String), SparqHttpError> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(&self.endpoint)
            .header(header::CONTENT_TYPE, CT_SPARQL_QUERY)
            .header(header::ACCEPT, accept)
            .body(Full::new(Bytes::from(sparql.to_string())))
            .map_err(|e| SparqHttpError::Build(e.to_string()))?;
        self.send(req).await
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
                let body = read_bounded(resp.into_body()).await?;
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
/// [`http_body_util::Limited`] bounds the read, so a runaway response is a fatal [`SparqHttpError::Body`]
/// rather than an OOM.
async fn read_bounded<B>(body: B) -> Result<Bytes, SparqHttpError>
where
    B: hyper::body::Body,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let limited = http_body_util::Limited::new(body, MAX_RESPONSE_BYTES as usize);
    match limited.collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
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

#[async_trait]
impl SparqClient for HttpSparqClient {
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
        })
    }

    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let u = sparql::update_put_meta(
            iri,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
        )?;
        self.update_raw(&u)
            .await
            .map_err(SparqHttpError::into_sparq)
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
        // DROP SILENT the resource's whole graph (idempotent on an absent graph).
        let u = sparql::update_delete_resource(iri)?;
        self.update_raw(&u)
            .await
            .map_err(SparqHttpError::into_sparq)
    }

    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        // A per-operation nonce so the success confirm is RACE-RESISTANT — no concurrent op writes or
        // removes THIS delete marker, so a containment mutation between the update and the confirm
        // cannot flip the result (the same pattern as the create-marker).
        let nonce = next_nonce();

        // ONE SPARQL 1.1 `DELETE { } INSERT { } WHERE { }` modify: its WHERE (container-EXISTS AND
        // `ldp:contains`-empty) is evaluated ONCE pre-modification, then the multi-graph DELETE (the
        // container's own graph AND, when `parent` is given, the `<parent> ldp:contains <container>`
        // edge in the parent's graph) and the marker INSERT apply atomically against that one solution.
        // A non-empty (or absent) container ⇒ the WHERE yields nothing ⇒ NOTHING is deleted or detached
        // (the safety invariant). Folding the parent-edge detach in here (rather than a separate later
        // `remove_child`) closes the window where a concurrent POST recreates the child and a stale
        // detach orphans it.
        let u = sparql::update_delete_container_if_empty(iri, parent, &nonce)?;
        self.update_raw(&u)
            .await
            .map_err(SparqHttpError::into_sparq)?;

        // Disambiguate the outcome with bounded follow-up ASKs (the protocol can't return rows from an
        // UPDATE; documented). FIRST: did the guard match (⇒ the delete ran)? ASK for OUR marker — a
        // nonce nothing else touches, so this is immune to a concurrent containment mutation.
        let marker_q = sparql::ask_delete_marker(iri, &nonce)?;
        let (body, _ct) = self
            .query_raw(&marker_q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        if parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            return Ok(DeleteOutcome::Deleted);
        }
        // Marker absent ⇒ the delete did NOT run (container was non-empty OR absent). Split the two by
        // a single ASK on the container's own record: present ⇒ NotEmpty (it had members, so the empty
        // guard blocked the delete), absent ⇒ NotFound (it was never indexed). Fail-closed.
        let exists_q = sparql::ask_exists(iri)?;
        let (body, _ct) = self
            .query_raw(&exists_q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        if parse_ask_json(&body).map_err(SparqHttpError::into_sparq)? {
            Ok(DeleteOutcome::NotEmpty)
        } else {
            Ok(DeleteOutcome::NotFound)
        }
    }

    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        // A per-operation unique nonce written atomically with the guarded create, so the success
        // confirm is RACE-RESISTANT: no concurrent remove_child / sibling create ever touches this
        // marker, so a containment-edge mutation between the update and the confirm cannot flip the
        // result (the round-4 finding — confirming on the mutable containment edge was racy).
        let nonce = next_nonce();

        // ONE atomic update: DELETE any stale child record + marker, INSERT the child record + the
        // containment edge + THIS operation's marker, guarded by the container-record EXISTS in the
        // WHERE clause. A missing container ⇒ the WHERE yields nothing ⇒ nothing inserted.
        let modified = meta.last_modified.and_then(timestamp::to_xsd_datetime);
        let u = sparql::update_create_child(
            container,
            child,
            &meta.content_type,
            &meta.blob_key,
            &meta.etag,
            modified.as_deref(),
            &nonce,
        )?;
        self.update_raw(&u)
            .await
            .map_err(SparqHttpError::into_sparq)?;

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
            Ok(())
        } else {
            Err(SparqError::NotFound)
        }
    }

    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        let u = sparql::update_remove_child(container, child)?;
        self.update_raw(&u)
            .await
            .map_err(SparqHttpError::into_sparq)
    }

    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        let q = sparql::select_children(container)?;
        let (body, _ct) = self
            .query_raw(&q, ACCEPT_RESULTS_JSON)
            .await
            .map_err(SparqHttpError::into_sparq)?;
        let result = parse_select_json(&body).map_err(SparqHttpError::into_sparq)?;
        let mut children = Vec::with_capacity(result.rows.len());
        for row in result.rows {
            // A row of a `SELECT ?child` MUST carry the `child` binding. A row missing it is a
            // malformed backend response, NOT an empty/partial list — surface it as a fatal error
            // rather than silently dropping it, because list_children feeds the fail-closed
            // empty-container DELETE check (a silently-shortened list could wrongly let a non-empty
            // container be deleted).
            match row.get("child") {
                Some(child) => children.push(child.clone()),
                None => {
                    return Err(SparqHttpError::Malformed(
                        "select-children row missing the 'child' binding".into(),
                    )
                    .into_sparq())
                }
            }
        }
        Ok(children)
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
        // candidate's presence/etag — in ONE combined SELECT (`VALUES ?g { … }`), replacing the
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
        assert!(!SparqHttpError::Build("x".into()).is_retryable());
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

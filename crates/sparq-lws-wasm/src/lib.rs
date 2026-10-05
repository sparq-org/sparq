// [GPT-5.6] sq-6xasp.3: wasm host adapter over the real LWS router.
// [SONNET-4.6] sq-250si: wasm panic hook + host trap-recovery boundary.
// [SONNET-4.6] sq-wubkf: bounded linear-memory accounting behind the `memory` module. The crate
// root is `deny(unsafe_code)` rather than `forbid` for exactly one reason — a `#[global_allocator]`
// requires `unsafe impl GlobalAlloc`, which has no safe substitute. `memory` carries the only
// `#[allow(unsafe_code)]` in the crate; every other module still fails to compile on `unsafe`.
#![deny(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(clippy::undocumented_unsafe_blocks)]
//! WebAssembly request entry for the in-memory Solid/LDP server core.
//!
//! Building this dedicated crate is the opt-in boundary. On wasm32, `SolidServer` owns the real axum
//! router over the in-memory SPARQ metadata and blob stores. JavaScript remains responsible for the
//! HTTP listener and OIDC verification; each request supplies only an already-authenticated WebID.
//!
//! # Panic and trap behaviour
//!
//! The release profile uses `panic=abort`, so a Rust panic lowers to a `wasm32` `unreachable`
//! trap rather than unwinding. The `lws_init` function registered via `#[wasm_bindgen(start)]`
//! installs a `console_error_panic_hook` that forwards the panic message to `console.error`
//! before the trap fires — giving the host a readable diagnostic in environments where the
//! hook can run (debug builds, `wasm-bindgen-test`). In `panic=abort` release builds the hook
//! fires before the abort and the message appears in the browser or Node console.
//!
//! The Node host (`@sparq-org/solid-server`) catches the resulting `WebAssembly.RuntimeError` and
//! recreates the `SolidServer` before the next request, so a single trap does not permanently
//! brick the process. See `packages/solid-server/src/index.js`.
//!
//! # Linear-memory ceiling
//!
//! Allocation failure at the linear-memory ceiling reaches that same trap, but — unlike a panic —
//! it is predictable from the pod's own live-byte total. [`memory`] installs a `System`-delegating
//! global allocator that tracks live bytes, and `handleRequest` refuses a request whose projected
//! peak would cross the configured ceiling with a clean HTTP 507 before the router ever runs. See
//! the [`memory`] module documentation for what that does and does not cover.
//!
//! # Persistence
//!
//! A pod built with `SolidServer::new` is ephemeral: its contents live in linear memory and are
//! gone when the host drops the instance. `SolidServer::withSnapshot` opts into the [`persist`]
//! seam instead — the same pod behind a journaling `Store`
//! decorator, restored from bytes the host kept and re-exportable through `snapshot()`. The host
//! owns the durable medium (`node:fs`, IndexedDB); this crate owns only the byte format and the
//! replay. See the [`persist`] module documentation, including why the store cannot call
//! JavaScript per operation.

// The `#[allow]` also has to cover the `#[global_allocator]` registration, whose expansion emits
// `unsafe` allocator shims — hence the whole module, not a single item, is the exemption.
#[allow(unsafe_code)]
pub mod memory;

// [GPT-5.6] sq-6xasp.7: the opt-in persistence seam. Nothing here is reachable from
// `SolidServer::new`, which stays purely in-memory; only `SolidServer::withSnapshot` journals.
pub mod persist;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body, Bytes};
    use axum::response::{IntoResponse, Response};
    use http::header::{HeaderName, HeaderValue};
    use http::{Method, Request, Uri};
    use oxrdf::{NamedNode, Triple};
    use sparq_lws_core::auth::VerifiedToken;
    use sparq_lws_core::ldp::content::{serialize_triples, RdfFormat};
    use sparq_lws_core::ldp::handler::LdpState;
    use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
    use sparq_lws_core::{build_router, AppState, ServerError};
    use tower::ServiceExt;
    use wasm_bindgen::prelude::*;

    use crate::memory;
    use crate::persist::{self, Journal, PersistentStore};

    type MemoryStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";

    /// Called once by the wasm-bindgen runtime when the module is first loaded.
    ///
    /// Installs a panic hook that writes the panic message to `console.error` before the
    /// `unreachable` trap propagates to the host as a `WebAssembly.RuntimeError`. The hook
    /// is idempotent — safe to call from any context, and a no-op if already installed.
    // [SONNET-4.6] sq-250si: set_once is idempotent; safe for concurrent (future) init paths.
    #[wasm_bindgen(start)]
    pub fn lws_init() {
        console_error_panic_hook::set_once();
    }

    /// Bytes currently held by live allocations inside the module's linear memory.
    // [SONNET-4.6] sq-wubkf: `f64` rather than `u64` so the host reads a plain JS number; byte
    // totals stay far inside the 2^53 exactly-representable range.
    #[wasm_bindgen(js_name = lwsMemoryLiveBytes)]
    pub fn lws_memory_live_bytes() -> f64 {
        memory::live_bytes() as f64
    }

    /// The largest live-byte total observed since the module was loaded.
    #[wasm_bindgen(js_name = lwsMemoryPeakBytes)]
    pub fn lws_memory_peak_bytes() -> f64 {
        memory::peak_bytes() as f64
    }

    /// The linear-memory ceiling above which a request is refused with 507; `0` means unbounded.
    #[wasm_bindgen(js_name = lwsMemoryCeilingBytes)]
    pub fn lws_memory_ceiling_bytes() -> f64 {
        memory::ceiling_bytes() as f64
    }

    /// Replace the linear-memory ceiling. Pass `0` to disable the bound.
    ///
    /// A host that runs several pods in one module should lower this so one pod cannot consume the
    /// whole linear memory. Rejects a negative or non-finite argument rather than silently
    /// unbounding.
    #[wasm_bindgen(js_name = lwsSetMemoryCeilingBytes)]
    pub fn lws_set_memory_ceiling_bytes(bytes: f64) -> Result<(), JsError> {
        let ceiling = memory::ceiling_from_js(bytes)
            .ok_or_else(|| JsError::new("memory ceiling must be a finite, non-negative number"))?;
        memory::set_ceiling_bytes(ceiling);
        Ok(())
    }

    /// A response returned across the wasm boundary.
    ///
    /// Headers are a flat `[name, value, name, value, ...]` array. Repeated header names remain
    /// repeated, which lets a Node host reconstruct the original HTTP response without joining them.
    #[wasm_bindgen]
    pub struct LwsResponse {
        status: u16,
        headers: Vec<String>,
        body: Vec<u8>,
    }

    #[wasm_bindgen]
    impl LwsResponse {
        /// Numeric HTTP status code.
        #[wasm_bindgen(getter)]
        pub fn status(&self) -> u16 {
            self.status
        }

        /// Flat name/value response-header pairs.
        #[wasm_bindgen(getter)]
        pub fn headers(&self) -> Vec<String> {
            self.headers.clone()
        }

        /// Complete response bytes.
        #[wasm_bindgen(getter)]
        pub fn body(&self) -> Vec<u8> {
            self.body.clone()
        }
    }

    /// A single-threaded, in-memory Solid/LDP request handler.
    ///
    /// The owner WebID passed to the constructor receives Read, Write, and Control on the storage
    /// root and descendants. That is initial provisioning, not OIDC verification. The JavaScript
    /// host verifies every real credential and passes the resulting WebID to `handleRequest`.
    #[wasm_bindgen]
    pub struct SolidServer {
        router: axum::Router,
        /// `Some` only for a pod built by `withSnapshot`; `new` journals nothing at all.
        journal: Option<Arc<Journal>>,
    }

    #[wasm_bindgen]
    impl SolidServer {
        /// Create an isolated in-memory pod and provision its owner ACL.
        #[wasm_bindgen(constructor)]
        pub fn new(base_url: String, owner_webid: String) -> Result<SolidServer, JsError> {
            let base_url = normalize_base_url(base_url)?;

            let store = MemoryStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
            futures_executor::block_on(ensure_owner_acl(&store, &base_url, &owner_webid))
                .map_err(|error| JsError::new(&error))?;
            let ldp = LdpState::new(store, base_url);
            Ok(Self {
                router: build_router(AppState::new(ldp)),
                journal: None,
            })
        }

        /// Create a pod that can be snapshotted, restoring `snapshot` if it carries one.
        ///
        /// This is the opt-in persistent constructor: the pod is the same in-memory store behind a
        /// journaling decorator, so a host that keeps the bytes from `snapshot()` in `node:fs` or
        /// IndexedDB can rebuild the pod's contents after a listener restart. Pass an empty array
        /// for a fresh persistent pod.
        ///
        /// The owner ACL is provisioned only when the restored pod does not already carry one, so a
        /// restart never overwrites an ACL the owner edited. Rejects a snapshot it cannot decode or
        /// replay rather than silently booting a pod with part of its data.
        // [GPT-5.6] sq-6xasp.7: `Vec<u8>` copies the host's `Uint8Array` in; the encoded journal is
        // bounded by the store's own 64 MiB blob quota.
        #[wasm_bindgen(js_name = withSnapshot)]
        pub fn with_snapshot(
            base_url: String,
            owner_webid: String,
            snapshot: Vec<u8>,
        ) -> Result<SolidServer, JsError> {
            let base_url = normalize_base_url(base_url)?;
            let records = if snapshot.is_empty() {
                Vec::new()
            } else {
                persist::decode(&snapshot)
                    .map_err(|error| JsError::new(&format!("invalid pod snapshot: {}", error)))?
            };

            let journal = Arc::new(Journal::new());
            let store = PersistentStore::new(
                MemoryStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new()),
                Arc::clone(&journal),
            );
            futures_executor::block_on(async {
                // Replay through the journaling decorator, so the restored pod's journal is rebuilt
                // as a side effect and it is immediately snapshottable again.
                persist::replay(&store, records)
                    .await
                    .map_err(|error| error.to_string())?;
                ensure_owner_acl(&store, &base_url, &owner_webid).await
            })
            .map_err(|error| JsError::new(&error))?;

            let ldp = LdpState::new(store, base_url);
            Ok(Self {
                router: build_router(AppState::new(ldp)),
                journal: Some(journal),
            })
        }

        /// The pod's contents as bytes to persist, or `undefined` for a non-persistent pod.
        ///
        /// Feed the result back to `withSnapshot` on the next boot. The encoding folds superseded
        /// writes and deleted resources away, so it tracks what the pod currently holds rather than
        /// everything that ever happened to it.
        #[wasm_bindgen(js_name = snapshot)]
        pub fn snapshot(&self) -> Option<Vec<u8>> {
            self.journal.as_ref().map(|journal| journal.encode())
        }

        /// How many mutations this pod has recorded; `0` for a non-persistent pod.
        ///
        /// Monotonic, so a host can skip rewriting its snapshot file while the value is unchanged.
        #[wasm_bindgen(js_name = snapshotRevision)]
        pub fn snapshot_revision(&self) -> f64 {
            self.journal
                .as_ref()
                .map_or(0, |journal| journal.revision()) as f64
        }

        /// Drive one request through the real axum router and its LDP, WAC, CORS, and body-limit
        /// layers.
        ///
        /// `headers` is a flat `[name, value, ...]` array and must contain an even number of strings.
        /// `authenticated_webid` is trusted only because the JavaScript host is responsible for OIDC;
        /// omit it for an anonymous request. The returned Promise needs no Tokio reactor.
        ///
        /// A request whose projected peak footprint would cross the linear-memory ceiling is
        /// refused with HTTP 507 before the router runs, so sustained memory pressure produces an
        /// HTTP answer instead of an `unreachable` trap. See [`crate::memory`].
        #[wasm_bindgen(js_name = handleRequest)]
        pub async fn handle_request(
            &self,
            method: String,
            path: String,
            headers: Vec<String>,
            body: Vec<u8>,
            authenticated_webid: Option<String>,
        ) -> Result<LwsResponse, JsError> {
            if !memory::admits_request(body.len() as u64) {
                // Reuse the core's own 507 so an admission refusal is byte-identical to the
                // store-quota refusal the host already handles.
                return into_lws_response(ServerError::InsufficientStorage.into_response()).await;
            }
            let request = build_request(method, path, headers, body, authenticated_webid)
                .map_err(|error| JsError::new(&error))?;
            let response = self
                .router
                .clone()
                .oneshot(request)
                .await
                .map_err(|error| JsError::new(&error.to_string()))?;
            into_lws_response(response).await
        }
    }

    async fn into_lws_response(response: Response) -> Result<LwsResponse, JsError> {
        let status = response.status().as_u16();
        let headers = flatten_headers(response.headers())?;
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|error| JsError::new(&error.to_string()))?
            .to_vec();
        Ok(LwsResponse {
            status,
            headers,
            body,
        })
    }

    fn build_request(
        method: String,
        path: String,
        headers: Vec<String>,
        body: Vec<u8>,
        authenticated_webid: Option<String>,
    ) -> Result<Request<Body>, String> {
        if !headers.len().is_multiple_of(2) {
            return Err("headers must contain name/value pairs".to_owned());
        }

        let method = Method::from_bytes(method.as_bytes())
            .map_err(|error| format!("invalid HTTP method: {}", error))?;
        let uri: Uri = path
            .parse()
            .map_err(|error| format!("invalid request path: {}", error))?;
        if uri.scheme().is_some() || uri.authority().is_some() || !uri.path().starts_with('/') {
            return Err("path must be an origin-form URI beginning with '/'".to_owned());
        }

        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::from(body))
            .map_err(|error| format!("could not build request: {}", error))?;
        for pair in headers.chunks_exact(2) {
            let name = HeaderName::from_bytes(pair[0].as_bytes())
                .map_err(|error| format!("invalid header name {:?}: {}", pair[0], error))?;
            let value = HeaderValue::from_str(&pair[1])
                .map_err(|error| format!("invalid value for header {}: {}", pair[0], error))?;
            request.headers_mut().append(name, value);
        }
        if let Some(webid) = authenticated_webid {
            request
                .extensions_mut()
                .insert(VerifiedToken::from_authenticated_webid(webid));
        }
        Ok(request)
    }

    fn flatten_headers(headers: &http::HeaderMap) -> Result<Vec<String>, JsError> {
        let mut flat = Vec::with_capacity(headers.len() * 2);
        for (name, value) in headers {
            flat.push(name.as_str().to_owned());
            flat.push(
                value
                    .to_str()
                    .map_err(|_| JsError::new("response contains a non-text header value"))?
                    .to_owned(),
            );
        }
        Ok(flat)
    }

    fn normalize_base_url(base_url: String) -> Result<String, JsError> {
        let base_url = base_url.trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Err(JsError::new("base_url must not be empty"));
        }
        Ok(base_url)
    }

    /// Provision the owner ACL unless the pod already carries one.
    ///
    /// A fresh pod has none, so this is the constructor's seeding step. A pod restored from a
    /// snapshot already replayed whichever ACL it last held — possibly one the owner edited — so
    /// re-seeding it would silently revert that edit on every restart.
    async fn ensure_owner_acl<S: Store>(
        store: &S,
        base_url: &str,
        owner_webid: &str,
    ) -> Result<(), String> {
        let acl_doc = format!("{}/.acl", base_url);
        let present = store
            .exists(&acl_doc)
            .await
            .map_err(|error| format!("could not read the owner ACL: {}", error))?;
        if present {
            return Ok(());
        }
        seed_owner_acl(store, base_url, owner_webid).await
    }

    async fn seed_owner_acl<S: Store>(
        store: &S,
        base_url: &str,
        owner_webid: &str,
    ) -> Result<(), String> {
        let root = format!("{}/", base_url);
        let acl_doc = format!("{}.acl", root);
        let auth = named_node(&format!("{}#owner", acl_doc))?;
        let root_node = named_node(&root)?;
        let mode = |local: &str| named_node(&format!("{}{}", ACL, local));
        let triples = vec![
            Triple::new(auth.clone(), named_node(RDF_TYPE)?, mode("Authorization")?),
            Triple::new(auth.clone(), mode("agent")?, named_node(owner_webid)?),
            Triple::new(auth.clone(), mode("accessTo")?, root_node.clone()),
            Triple::new(auth.clone(), mode("default")?, root_node),
            Triple::new(auth.clone(), mode("mode")?, mode("Read")?),
            Triple::new(auth.clone(), mode("mode")?, mode("Write")?),
            Triple::new(auth, mode("mode")?, mode("Control")?),
        ];
        let bytes = serialize_triples(RdfFormat::Turtle, &triples)
            .map_err(|error| format!("could not serialize owner ACL: {}", error))?;
        store
            .write(&acl_doc, Bytes::from(bytes), RdfFormat::Turtle.media_type())
            .await
            .map_err(|error| format!("could not provision owner ACL: {}", error))?;
        Ok(())
    }

    fn named_node(iri: &str) -> Result<NamedNode, String> {
        NamedNode::new(iri).map_err(|error| format!("invalid IRI {:?}: {}", iri, error))
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{LwsResponse, SolidServer};

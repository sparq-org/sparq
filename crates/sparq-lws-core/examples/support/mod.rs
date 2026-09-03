// AUTHORED-BY Claude Opus 4.8
//! Shared harness plumbing for the `bench_harness` + `adversarial_bench` examples.
//!
//! This module is `#[path]`-included by each example (examples are independent binary crates, so a
//! `#[path = "support/mod.rs"] mod support;` include is the DRY-est way to share the JOSE/DPoP
//! minting, the in-memory-double router assembly, the counting global allocator, and the JSON report
//! schema between them without adding anything to the library crate (`src/` is untouched — every
//! seam used here is already public: [`build_router`], [`AuthContext::with_cache`], the in-memory
//! store/replay doubles).
//!
//! ## The deterministic-vs-advisory split (PSS charter perf-gate rule)
//! The report schema tags every metric block with a `mode`:
//!  - `"deterministic"` — reproducible integer counts (HTTP status, response byte length, and — via
//!    the [`CountingAllocator`] — allocations + bytes allocated for ONE request in isolation on a
//!    single-threaded runtime). These are strict/comparable floor metrics.
//!  - `"timing_advisory"` — wall-clock-derived throughput + latency percentiles under concurrency.
//!    Shared-runner wall-clock variance exceeds any useful band, so these are ADVISORY: measured,
//!    reported, never a merge gate. Every timing block carries a `disclaimer`.
//!
//! No perf NUMBERS live in markdown — the docs point at the JSON these examples generate.

#![warn(clippy::undocumented_unsafe_blocks)] // [FABLE-5] every unsafe site needs a // SAFETY:
#![allow(dead_code)]

#[cfg(not(feature = "ac-query-scale-instrumentation"))]
use std::alloc::System;
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::Request;
use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};
use rand_core::OsRng;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use solid_oidc_verifier::config::{StaticJwksProvider, VerifierConfig};
use solid_oidc_verifier::jwk::Jwk;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::auth_cache::{ProofPolicy, SharedReplay, VerifiedTokenCache};
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};

// ---------------------------------------------------------------------------------------------
// Fixed identities — mirror `tests/common/mod.rs` so the doubles agree on issuer/audience/webid.
// ---------------------------------------------------------------------------------------------

pub const ISSUER: &str = "https://idp.example/realms/solid";
pub const WEBID: &str = "https://pod.example/alice/profile/card#me";
/// A DIFFERENT authenticated WebID with NO ACL grant — the adversarial "foreign reader".
pub const FOREIGN_WEBID: &str = "https://pod.example/mallory/profile/card#me";
/// The server base URL == verifier audience == DPoP `htu` origin.
pub const BASE_URL: &str = "https://pod.example";
pub const CLIENT_ID: &str = "solid-app";

// ---------------------------------------------------------------------------------------------
// Counting global allocator — the deterministic allocation metric source.
// ---------------------------------------------------------------------------------------------

pub static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
pub static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

/// A backing-allocator wrapper that counts allocation OPS + bytes (Relaxed atomics — cheap).
/// Each example installs it as its `#[global_allocator]`. During a single-threaded deterministic
/// probe (no other runtime threads alive) the counter delta around one request is a reproducible,
/// wall-clock-independent measure of that request's allocation cost. Existing examples delegate to
/// [`System`]; the paper-only `ac-query-scale-instrumentation` feature delegates to mimalloc, which
/// is what the production `sparq-lws-core` binary uses. The counters perturb wall time, so paper
/// timings use an uninstrumented build.
pub struct CountingAllocator;

// SAFETY: pure pass-through wrapper over a sound backing allocator (`System` for the established
// harnesses, `mimalloc::MiMalloc` for the paper instrumentation profile), adding Relaxed atomic
// counters around it; pointer/layout contracts are forwarded unchanged and no returned pointer is
// dereferenced, retained, or aliased. Registered in the unsafe-count ratchet:
// compliance/memsafety/unsafe-register.md (sq-gg0qq.2). [FABLE-5]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        #[cfg(feature = "ac-query-scale-instrumentation")]
        {
            mimalloc::MiMalloc.alloc(layout)
        }
        #[cfg(not(feature = "ac-query-scale-instrumentation"))]
        {
            System.alloc(layout)
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        #[cfg(feature = "ac-query-scale-instrumentation")]
        {
            mimalloc::MiMalloc.dealloc(ptr, layout);
        }
        #[cfg(not(feature = "ac-query-scale-instrumentation"))]
        {
            System.dealloc(ptr, layout);
        }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        #[cfg(feature = "ac-query-scale-instrumentation")]
        {
            mimalloc::MiMalloc.realloc(ptr, layout, new_size)
        }
        #[cfg(not(feature = "ac-query-scale-instrumentation"))]
        {
            System.realloc(ptr, layout, new_size)
        }
    }
}

/// `(count, bytes)` of allocations observed so far.
pub fn alloc_snapshot() -> (u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------------------------------
// ES256 key + JOSE/DPoP minting (trimmed port of tests/common — cannot `use` a test-only module).
// ---------------------------------------------------------------------------------------------

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_json(v: &Value) -> String {
    b64url(serde_json::to_vec(v).unwrap().as_slice())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

use std::sync::atomic::AtomicU64 as Ctr;
static COUNTER: Ctr = Ctr::new(0);

/// A per-PROCESS random nonce mixed into every jti so a fresh run never collides with a prior run's
/// jti space against a long-lived replay store.
fn jti_nonce() -> &'static str {
    use std::sync::OnceLock;
    static NONCE: OnceLock<String> = OnceLock::new();
    NONCE.get_or_init(|| {
        let mut bytes = [0u8; 12];
        getrandom::getrandom(&mut bytes).expect("OS randomness for the jti nonce");
        b64url(&bytes)
    })
}

/// A globally-unique DPoP jti (per-process nonce + monotonic counter).
pub fn next_jti() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", jti_nonce(), n)
}

/// An ES256 key pair + public JWK + RFC 7638 thumbprint.
pub struct BenchKey {
    pub signing: SigningKey,
    pub public_jwk: Value,
    pub thumbprint: String,
}

impl BenchKey {
    pub fn generate() -> Self {
        let signing = SigningKey::random(&mut OsRng);
        let verifying: VerifyingKey = *signing.verifying_key();
        let point = verifying.to_encoded_point(false);
        let x = b64url(point.x().unwrap());
        let y = b64url(point.y().unwrap());
        let public_jwk = json!({ "kty": "EC", "crv": "P-256", "x": x, "y": y });
        let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        let thumbprint = b64url(&Sha256::digest(canonical.as_bytes()));
        Self {
            signing,
            public_jwk,
            thumbprint,
        }
    }

    pub fn jwk(&self) -> Jwk {
        serde_json::from_value(self.public_jwk.clone()).unwrap()
    }

    fn sign(&self, header: &Value, claims: &Value) -> String {
        let signing_input = format!("{}.{}", b64url_json(header), b64url_json(claims));
        let sig: Signature = self.signing.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", b64url(&sig.to_bytes()))
    }
}

/// A static JWKS provider over the issuer's key.
pub fn jwks_provider(issuer_key: &BenchKey) -> StaticJwksProvider {
    StaticJwksProvider::new().with_issuer(ISSUER.to_string(), vec![issuer_key.jwk()])
}

/// Mint an RFC-9068 access token for `webid`, bound to `cnf_jkt`, signed by `issuer_key`.
pub fn mint_access_token_webid(issuer_key: &BenchKey, cnf_jkt: &str, webid: &str) -> String {
    mint_access_token_webid_with_ttl(issuer_key, cnf_jkt, webid, 300)
}

/// Mint an RFC-9068 access token with an explicit lifetime for a long-running benchmark cell.
///
/// The ordinary harness token remains five minutes. A scaling cell may deliberately run longer,
/// so callers can make the credential valid beyond the cell without changing the verifier cache's
/// independent validation-freshness bound.
pub fn mint_access_token_webid_with_ttl(
    issuer_key: &BenchKey,
    cnf_jkt: &str,
    webid: &str,
    ttl_seconds: i64,
) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = unix_now();
    let claims = json!({
        "iss": ISSUER,
        "sub": webid,
        "jti": format!("at-{}", COUNTER.fetch_add(1, Ordering::Relaxed)),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": webid,
        "cnf": { "jkt": cnf_jkt },
        "iat": iat,
        "exp": iat.saturating_add(ttl_seconds.max(1)),
    });
    issuer_key.sign(&header, &claims)
}

/// The owner-WebID access token (the common case).
pub fn mint_access_token(issuer_key: &BenchKey, cnf_jkt: &str) -> String {
    mint_access_token_webid(issuer_key, cnf_jkt, WEBID)
}

/// base64url(SHA-256(token)) — the DPoP `ath`.
pub fn ath(token: &str) -> String {
    b64url(&Sha256::digest(token.as_bytes()))
}

/// Mint a fresh DPoP proof (unique jti) for `method`+`url`, bound to `access_token` via `ath`.
pub fn mint_dpop_proof(
    client_key: &BenchKey,
    method: &str,
    url: &str,
    access_token: &str,
) -> String {
    let header = json!({ "alg": "ES256", "typ": "dpop+jwt", "jwk": client_key.public_jwk });
    let claims = json!({
        "htm": method,
        "htu": url,
        "jti": next_jti(),
        "iat": unix_now(),
        "ath": ath(access_token),
    });
    client_key.sign(&header, &claims)
}

/// Mint a DPoP proof REUSING a fixed jti (for the replay-storm arm — every call replays the same jti).
pub fn mint_dpop_proof_fixed_jti(
    client_key: &BenchKey,
    method: &str,
    url: &str,
    access_token: &str,
    jti: &str,
) -> String {
    let header = json!({ "alg": "ES256", "typ": "dpop+jwt", "jwk": client_key.public_jwk });
    let claims = json!({
        "htm": method,
        "htu": url,
        "jti": jti,
        "iat": unix_now(),
        "ath": ath(access_token),
    });
    client_key.sign(&header, &claims)
}

// ---------------------------------------------------------------------------------------------
// Router assembly over the in-memory doubles (production posture: verified-token cache ON).
// ---------------------------------------------------------------------------------------------

pub type BenchStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

/// A fresh in-memory composite store.
pub fn make_store() -> BenchStore {
    CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
}

/// Seed a ROOT `<base>/.acl` granting `owner_webid` Read/Write/Control on the root AND all descendants
/// (`acl:default`) — the pod-root owner-default the real conformance seed writes per user. Turtle
/// string (a test fixture, not production RDF construction).
pub async fn seed_owner_root_acl(store: &BenchStore, owner_webid: &str) {
    let base = BASE_URL.trim_end_matches('/');
    let root = format!("{base}/");
    let acl_iri = format!("{root}.acl");
    let acl_body = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{owner_webid}>;
         acl:accessTo <{root}>;
         acl:default <{root}>;
         acl:mode acl:Read, acl:Write, acl:Control."#
    );
    store
        .write(&acl_iri, Bytes::from(acl_body), "text/turtle")
        .await
        .expect("seed root owner acl");
}

/// Seed a resource with the given body + content type.
pub async fn seed_resource(store: &BenchStore, iri: &str, body: impl Into<Bytes>, ctype: &str) {
    store
        .write(iri, body.into(), ctype)
        .await
        .expect("seed resource");
}

/// Seed a CHILD resource AND record its `ldp:contains` membership under `container` (the POST
/// containment path). Plain [`seed_resource`]/`write` does NOT add the membership edge, so a
/// container-listing GET would see an empty container; use this to make the listing scenario actually
/// list its children.
pub async fn seed_child(
    store: &BenchStore,
    container: &str,
    child: &str,
    body: impl Into<Bytes>,
    ctype: &str,
) {
    store
        .create_in_container(container, child, body.into(), ctype)
        .await
        .expect("seed child in container");
}

/// Seed a PUBLIC-Read `.acl` for `resource_iri` (`acl:agentClass foaf:Agent`) so an anonymous GET is
/// authorized — the pre-crypto public-read fast-path scenario.
pub async fn seed_public_read_acl(store: &BenchStore, resource_iri: &str) {
    let acl_iri = format!("{resource_iri}.acl");
    let acl_body = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#pub> a acl:Authorization;
       acl:agentClass foaf:Agent;
       acl:accessTo <{resource_iri}>;
       acl:mode acl:Read."#
    );
    store
        .write(&acl_iri, Bytes::from(acl_body), "text/turtle")
        .await
        .expect("seed public read acl");
}

/// Assemble the router with the verified-token cache ENABLED over the real verifier (production
/// posture). `cache_capacity` sizes the token cache. The `store` is moved into the LDP state.
pub fn assemble_app<S: Store + 'static>(
    store: S,
    issuer_key: &BenchKey,
    cache_capacity: usize,
) -> axum::Router {
    let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
    let policy = ProofPolicy {
        clock_tolerance_secs: config.clock_tolerance_secs,
        allow_missing_ath: config.allow_missing_ath,
        replay_fail_closed: config.replay_fail_closed,
    };
    let shared = SharedReplay::new(Arc::new(InMemoryReplayStore::with_window(
        config.replay_ttl(),
    )));
    let cache_replay = Arc::new(shared.clone());
    let verifier = Verifier::new(config, jwks_provider(issuer_key), shared).expect("valid config");
    let cache = VerifiedTokenCache::new(cache_capacity, policy);
    let ctx = AuthContext::with_cache(verifier, BASE_URL, cache, cache_replay);
    let ldp = LdpState::new(store, BASE_URL);
    build_router(AppState::new(ctx, ldp))
}

// ---------------------------------------------------------------------------------------------
// Pre-built request parts — signed OUTSIDE the timed window so timing measures the SERVER, not the
// client's ES256 signing cost.
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct PreReq {
    pub method: String,
    pub path: String,
    pub authz: Option<String>,
    pub dpop: Option<String>,
    pub content_type: Option<String>,
    pub extra: Vec<(String, String)>,
    pub body: Bytes,
}

impl PreReq {
    pub fn to_request(&self) -> Request<Body> {
        let mut b = Request::builder()
            .method(self.method.as_str())
            .uri(&self.path);
        if let Some(a) = &self.authz {
            b = b.header("authorization", a);
        }
        if let Some(d) = &self.dpop {
            b = b.header("dpop", d);
        }
        if let Some(c) = &self.content_type {
            b = b.header("content-type", c);
        }
        for (k, v) in &self.extra {
            b = b.header(k.as_str(), v.as_str());
        }
        b.body(Body::from(self.body.clone())).unwrap()
    }
}

/// Build an authenticated `PreReq` (mints + attaches a fresh DPoP proof for `access_token`).
#[allow(clippy::too_many_arguments)]
pub fn authed_prereq(
    client_key: &BenchKey,
    access_token: &str,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    extra: &[(&str, &str)],
    body: Bytes,
) -> PreReq {
    let htu = format!("{BASE_URL}{path}");
    let dpop = mint_dpop_proof(client_key, method, &htu, access_token);
    PreReq {
        method: method.to_string(),
        path: path.to_string(),
        authz: Some(format!("DPoP {access_token}")),
        dpop: Some(dpop),
        content_type: content_type.map(str::to_string),
        extra: extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body,
    }
}

/// Build an anonymous (no-credential) `PreReq`.
pub fn anon_prereq(method: &str, path: &str) -> PreReq {
    PreReq {
        method: method.to_string(),
        path: path.to_string(),
        authz: None,
        dpop: None,
        content_type: None,
        extra: Vec::new(),
        body: Bytes::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Driving requests + measuring.
// ---------------------------------------------------------------------------------------------

/// Drive one request to completion (status + response byte length). Consumes the body so buffering
/// cost is counted.
pub async fn drive_once(app: &axum::Router, req: Request<Body>) -> (u16, usize) {
    use tower::ServiceExt;
    let resp = app.clone().oneshot(req).await.expect("router service");
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .map(|b| b.len())
        .unwrap_or(0);
    (status, bytes)
}

/// DETERMINISTIC probe: run one request in isolation on a single-threaded runtime, taking the MINIMUM
/// allocation delta over `iters` warm iterations (the stable floor — filters one-time lazy inits).
///
/// `mk` MUST mint a FRESH request each call (a new jti and, for a create, a new path): reusing one
/// pre-signed request would replay its jti, so after the warmup every measured iteration would be a
/// 401 replay-reject (and a `put_create` would stop being a fresh create) — i.e. we'd measure the
/// wrong path. Each fresh request is built OUTSIDE the allocation snapshot window (the client-side
/// ES256 signing in `mk` is not counted), so the delta reflects the SERVER's per-request allocation.
/// The measured status is taken from the measured iterations (not just the warmup).
pub fn deterministic_probe(
    app: &axum::Router,
    mk: &mut dyn FnMut() -> PreReq,
    iters: u32,
) -> DeterministicMetric {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let mut status = 0u16;
    let mut bytes = 0usize;
    let mut min_count = u64::MAX;
    let mut min_bytes = u64::MAX;
    rt.block_on(async {
        // Warm once (discard) so lazy statics / caches are populated before we measure.
        let _ = drive_once(app, mk().to_request()).await;
        for _ in 0..iters {
            // Build the fresh request BEFORE the snapshot so the client-side signing is not counted.
            let req = mk().to_request();
            let (c0, b0) = alloc_snapshot();
            let (s, b) = drive_once(app, req).await;
            let (c1, b1) = alloc_snapshot();
            status = s;
            bytes = b;
            min_count = min_count.min(c1.saturating_sub(c0));
            min_bytes = min_bytes.min(b1.saturating_sub(b0));
        }
    });
    DeterministicMetric {
        mode: "deterministic",
        status,
        response_bytes: bytes,
        alloc_count_per_op: if min_count == u64::MAX { 0 } else { min_count },
        alloc_bytes_per_op: if min_bytes == u64::MAX { 0 } else { min_bytes },
    }
}

/// TIMING sweep (ADVISORY): on a multi-thread runtime, replay a pool of pre-built requests across
/// `concurrency` workers, recording per-request elapsed micros. Wall-clock derived — never a gate.
pub fn timing_sweep(
    rt: &tokio::runtime::Runtime,
    app: &axum::Router,
    reqs: Arc<Vec<PreReq>>,
    concurrency: usize,
    expected_status: u16,
) -> TimingLevel {
    use std::sync::atomic::AtomicUsize;
    use tokio::time::Instant;

    let total = reqs.len();
    let cursor = Arc::new(AtomicUsize::new(0));

    let (wall, mut samples, errors) = rt.block_on(async {
        let start = Instant::now();
        let mut handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let app = app.clone();
            let reqs = reqs.clone();
            let cursor = cursor.clone();
            handles.push(tokio::spawn(async move {
                let mut local: Vec<u64> = Vec::new();
                let mut errs = 0u64;
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= reqs.len() {
                        break;
                    }
                    let req = reqs[i].to_request();
                    let t0 = Instant::now();
                    let (status, _len) = drive_once(&app, req).await;
                    local.push(t0.elapsed().as_micros() as u64);
                    if status != expected_status {
                        errs += 1;
                    }
                }
                (local, errs)
            }));
        }
        let mut samples: Vec<u64> = Vec::with_capacity(total);
        let mut errors = 0u64;
        for h in handles {
            let (local, errs) = h.await.expect("worker join");
            samples.extend(local);
            errors += errs;
        }
        (start.elapsed(), samples, errors)
    });

    samples.sort_unstable();
    let requests = samples.len() as u64;
    let throughput = if wall.as_secs_f64() > 0.0 {
        requests as f64 / wall.as_secs_f64()
    } else {
        0.0
    };
    TimingLevel {
        mode: "timing_advisory",
        concurrency,
        requests,
        errors,
        success_rate: if requests > 0 {
            (requests - errors) as f64 / requests as f64
        } else {
            0.0
        },
        throughput_rps: round2(throughput),
        latency_us: Percentiles {
            p50: pct(&samples, 50.0),
            p90: pct(&samples, 90.0),
            p99: pct(&samples, 99.0),
            p999: pct(&samples, 99.9),
            max: samples.last().copied().unwrap_or(0),
        },
    }
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Percentiles over an already-sorted micros slice.
pub fn percentiles(sorted: &[u64]) -> Percentiles {
    Percentiles {
        p50: pct(sorted, 50.0),
        p90: pct(sorted, 90.0),
        p99: pct(sorted, 99.0),
        p999: pct(sorted, 99.9),
        max: sorted.last().copied().unwrap_or(0),
    }
}

/// Detailed result of replaying a request pool at one concurrency: sorted per-request micros, a status
/// histogram (for the adversarial invariants — WHICH statuses came back), and wall/throughput.
pub struct PoolResult {
    pub latencies_us: Vec<u64>,
    pub status_counts: std::collections::BTreeMap<u16, u64>,
    pub wall_secs: f64,
    pub throughput_rps: f64,
    pub requests: u64,
}

/// Replay a pre-built request pool across `concurrency` workers, capturing the status histogram +
/// per-request latency. The adversarial arms inspect the histogram to assert invariants (e.g. NEVER a
/// 200 for a bogus credential; a replayed jti is rejected).
pub fn run_pool_detailed(
    rt: &tokio::runtime::Runtime,
    app: &axum::Router,
    reqs: Arc<Vec<PreReq>>,
    concurrency: usize,
) -> PoolResult {
    use std::sync::atomic::AtomicUsize;
    use tokio::time::Instant;

    let cursor = Arc::new(AtomicUsize::new(0));
    let (wall, mut samples, statuses) = rt.block_on(async {
        let start = Instant::now();
        let mut handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let app = app.clone();
            let reqs = reqs.clone();
            let cursor = cursor.clone();
            handles.push(tokio::spawn(async move {
                let mut local: Vec<u64> = Vec::new();
                let mut counts: std::collections::BTreeMap<u16, u64> =
                    std::collections::BTreeMap::new();
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= reqs.len() {
                        break;
                    }
                    let req = reqs[i].to_request();
                    let t0 = Instant::now();
                    let (status, _len) = drive_once(&app, req).await;
                    local.push(t0.elapsed().as_micros() as u64);
                    *counts.entry(status).or_insert(0) += 1;
                }
                (local, counts)
            }));
        }
        let mut samples: Vec<u64> = Vec::new();
        let mut statuses: std::collections::BTreeMap<u16, u64> = std::collections::BTreeMap::new();
        for h in handles {
            let (local, counts) = h.await.expect("worker join");
            samples.extend(local);
            for (k, v) in counts {
                *statuses.entry(k).or_insert(0) += v;
            }
        }
        (start.elapsed(), samples, statuses)
    });

    samples.sort_unstable();
    let requests = samples.len() as u64;
    let throughput = if wall.as_secs_f64() > 0.0 {
        requests as f64 / wall.as_secs_f64()
    } else {
        0.0
    };
    PoolResult {
        latencies_us: samples,
        status_counts: statuses,
        wall_secs: wall.as_secs_f64(),
        throughput_rps: round2(throughput),
        requests,
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------------------------
// Report schema (serde) — every metric block carries its `mode`.
// ---------------------------------------------------------------------------------------------

pub const TIMING_DISCLAIMER: &str =
    "ADVISORY: wall-clock-derived under an in-process oneshot driver (no socket/TLS); \
     shared-runner variance exceeds any useful band — NEVER gate a merge on these. \
     Only the deterministic block (status/response_bytes/alloc_*) is strict/comparable.";

#[derive(Serialize)]
pub struct DeterministicMetric {
    pub mode: &'static str,
    pub status: u16,
    pub response_bytes: usize,
    pub alloc_count_per_op: u64,
    pub alloc_bytes_per_op: u64,
}

#[derive(Serialize)]
pub struct Percentiles {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

#[derive(Serialize)]
pub struct TimingLevel {
    pub mode: &'static str,
    pub concurrency: usize,
    pub requests: u64,
    pub errors: u64,
    pub success_rate: f64,
    pub throughput_rps: f64,
    pub latency_us: Percentiles,
}

#[derive(Serialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub deterministic: DeterministicMetric,
    pub timing_advisory: TimingBlock,
}

#[derive(Serialize)]
pub struct TimingBlock {
    pub mode: &'static str,
    pub disclaimer: &'static str,
    pub levels: Vec<TimingLevel>,
}

impl TimingBlock {
    pub fn new(levels: Vec<TimingLevel>) -> Self {
        Self {
            mode: "timing_advisory",
            disclaimer: TIMING_DISCLAIMER,
            levels,
        }
    }
}

#[derive(Serialize)]
pub struct Report {
    pub harness: String,
    pub generated_unix: i64,
    pub build_profile: String,
    pub driver: String,
    pub notes: String,
    pub scenarios: Vec<Scenario>,
}

/// Read the current build profile (debug vs release) — recorded in the report for honesty.
pub fn build_profile() -> String {
    if cfg!(debug_assertions) {
        "debug".to_string()
    } else {
        "release".to_string()
    }
}

pub fn generated_unix() -> i64 {
    unix_now()
}

/// Write `value` as pretty JSON to `path`, creating parent dirs.
pub fn write_json(path: &str, value: &impl Serialize) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).expect("create report dir");
    }
    let json = serde_json::to_string_pretty(value).expect("serialize report");
    std::fs::write(path, json).expect("write report");
}

/// Parse simple `--flag value` CLI args into a map (std-only, no clap).
pub fn parse_args() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        if let Some(flag) = args[i].strip_prefix("--") {
            if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                map.insert(flag.to_string(), args[i + 1].clone());
                i += 2;
            } else {
                map.insert(flag.to_string(), "true".to_string());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    #[test]
    fn explicit_access_token_lifetime_is_encoded_exactly() {
        let issuer_key = BenchKey::generate();
        let token =
            mint_access_token_webid_with_ttl(&issuer_key, "client-thumbprint", WEBID, 43_200);
        let payload = token.split('.').nth(1).expect("JWT payload");
        let payload = URL_SAFE_NO_PAD.decode(payload).expect("base64url payload");
        let claims: Value = serde_json::from_slice(&payload).expect("JSON claims");

        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            43_200
        );
    }
}

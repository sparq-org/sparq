// AUTHORED-BY Claude Opus 4.8
//! Authenticated DPoP load client for the EXPERIMENTAL solid-server-rs HTTPS benchmark (round 2).
//!
//! This is the **auth-path** companion to `bench/run.sh`'s auth-free `oha` sweep. `oha` cannot mint a
//! fresh RFC 9449 DPoP proof per request, so the authenticated GET — the realistic production path,
//! where the server pays token-verify + JWKS + the single ACL read/parse — was deferred in rounds
//! 0–1. This binary closes that gap: it obtains a DPoP-bound RFC 9068 access token from the SAME
//! Keycloak `solid` realm the conformance harness uses (the `conformance-alice` client-credentials
//! service account), then drives a concurrency sweep of authenticated GETs, each carrying a freshly
//! minted DPoP proof (`htu`=exact URL, `htm`=GET, unique `jti`, `iat`, and `ath`=base64url(SHA-256(
//! access_token)) because the server ENFORCES `ath`).
//!
//! It does **not** change any server request-handling behaviour — it is a load *client*. Run it via
//! `bench/run-auth.sh`, which boots the server (conformance-seeded so alice's WebID + pod exist), has
//! this client PUT a private fixture into alice's pod, asserts a single authed GET returns 200 BEFORE
//! the sweep (a misbuilt proof would otherwise measure the 401 path), then sweeps concurrency.
//!
//! ## Token flow (REUSED from conformance — no new auth flow invented)
//! The Keycloak `solid` realm requires a DPoP proof ON THE TOKEN REQUEST (DPoP-bound client
//! credentials). So:
//!   1. mint a token-endpoint DPoP proof (htu=token endpoint, htm=POST, NO `ath`), POST a
//!      `grant_type=client_credentials` to the realm token endpoint with `DPoP:` header → a
//!      `cnf.jkt`-bound `at+jwt` access token whose `webid` is alice + `aud` the server base URL;
//!   2. per resource request, mint a FRESH proof from the SAME key (htu=request URL, htm=GET, unique
//!      jti, `ath` over the token).
//!
//! The proof key is a single ES256 key generated at startup (the token's `cnf.jkt` binds to it); each
//! request gets a fresh proof (unique jti) so the server's replay store never rejects a re-used jti.
//!
//! ## Metrics
//! Per concurrency level it records: requests completed, success rate (HTTP 200 = success), max
//! sustained RPS over the measurement window, and p50/p99/p999 latency. Timing uses
//! `tokio::time::Instant` deltas around the in-flight request only (NOT wall-clock `Date.now`-style
//! absolute timestamps) — every percentile is a measured per-request elapsed duration. Results are
//! written as one JSON object per scenario+concurrency to the `--out` dir, mirroring the `oha` JSON
//! shape the existing harness parses, so the same downstream reporting applies.
//!
//! ## Why a client_credentials service account (not auth-code)?
//! The conformance harness drives the SAME client-credentials DPoP flow (see
//! `conformance/config/solid-server-rs.env`). We reuse the exact client id/secret/token endpoint —
//! the realistic Solid-OIDC token-verify hot path on the server is identical regardless of how the
//! token was obtained (the server verifies the at+jwt + the DPoP proof; it does not know or care that
//! the token came from a service account vs an auth-code login).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};
use rand_core::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

// --- small JOSE/DPoP helpers (ES256), mirroring tests/common/mod.rs ----------------------------

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

static JTI_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A per-PROCESS random nonce mixed into every jti. The server's in-memory replay store remembers a
/// jti for its whole lifetime, so a monotonic counter alone (which restarts at 0 each process run)
/// would collide with a PRIOR run's jtis against a long-lived server → those proofs rejected as
/// replays (a 401). The random prefix makes each run's jti space disjoint, so a fresh client run
/// against an already-running server still mints never-before-seen jtis. (Built once, lazily.)
fn jti_nonce() -> &'static str {
    use std::sync::OnceLock;
    static NONCE: OnceLock<String> = OnceLock::new();
    NONCE.get_or_init(|| {
        let mut bytes = [0u8; 12];
        getrandom::getrandom(&mut bytes).expect("OS randomness for the jti nonce");
        b64url(&bytes)
    })
}

/// A globally-unique DPoP jti: a per-process random nonce + a monotonic counter (unique within the
/// run). `next_jti` returns the full string so no two proofs — within OR across runs — ever collide.
fn next_jti() -> String {
    let n = JTI_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", jti_nonce(), n)
}

/// An ES256 client key + its public JWK (the DPoP proof key the token is `cnf.jkt`-bound to).
struct ProofKey {
    signing: SigningKey,
    public_jwk: Value,
}

impl ProofKey {
    fn generate() -> Self {
        let signing = SigningKey::random(&mut OsRng);
        let verifying: VerifyingKey = *signing.verifying_key();
        let point = verifying.to_encoded_point(false);
        let x = b64url(point.x().unwrap());
        let y = b64url(point.y().unwrap());
        let public_jwk = json!({ "kty": "EC", "crv": "P-256", "x": x, "y": y });
        Self {
            signing,
            public_jwk,
        }
    }

    fn sign(&self, header: &Value, claims: &Value) -> String {
        let signing_input = format!("{}.{}", b64url_json(header), b64url_json(claims));
        let sig: Signature = self.signing.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", b64url(&sig.to_bytes()))
    }

    /// A DPoP proof for `method`+`url`. `ath` is included iff an access token is given (token requests
    /// have no `ath`; resource requests do). A unique `jti` per proof so the server's replay store
    /// never rejects a re-used jti under load.
    fn dpop_proof(&self, method: &str, url: &str, access_token: Option<&str>) -> String {
        let header = json!({ "alg": "ES256", "typ": "dpop+jwt", "jwk": self.public_jwk });
        let mut claims = json!({
            "htm": method,
            "htu": url,
            "jti": next_jti(),
            "iat": unix_now(),
        });
        if let Some(tok) = access_token {
            claims["ath"] = json!(b64url(&Sha256::digest(tok.as_bytes())));
        }
        self.sign(&header, &claims)
    }
}

// --- CLI config (env-driven, mirrors bench/run.sh's CONFIG block) -------------------------------

struct Config {
    /// The server base used as the DPoP `htu` origin + audience identity (e.g. https://localhost:3000).
    base_url: String,
    /// The IPv4 dial target (the server binds IPv4; localhost→::1 first on macOS — see bench/README).
    connect_base: String,
    /// The Keycloak realm token endpoint.
    token_endpoint: String,
    client_id: String,
    client_secret: String,
    /// The private resource path to GET under load (relative to base, e.g. /alice/test/bench-private).
    target_path: String,
    /// The container path for the authenticated-listing scenario (optional; empty ⇒ skip scenario d).
    listing_path: String,
    /// When true, PUT a small private Turtle document to `target_path` at startup (so scenario (c)
    /// measures a genuine owner-private DOCUMENT GET). The owner (alice) has Read/Write under
    /// `/alice/test/` via the conformance seed's pod-root `acl:default`, so the resource is private
    /// (no public grant) yet readable by the token's WebID — exactly the auth-path fixture we want.
    put_fixture: bool,
    /// Number of child documents to PUT into `listing_path` at startup (the authed-listing scenario d
    /// — auth + render combined). 0 ⇒ don't seed children (the container is rendered as-is). Each child
    /// is an owner-private document; under load the OWNER reads the container so render cost scales
    /// with the membership.
    listing_children: usize,
    /// ANONYMOUS mode: send NO Authorization/DPoP headers (the public-read comparison baseline). The
    /// pre-flight then asserts 200 anonymously (a public resource) instead of the authed 200/anon-401
    /// privacy assertion. Used by run-auth.sh's apples-to-apples auth-overhead comparison sweep.
    anon: bool,
    /// AUTHED-PUBLIC mode (EXPLORATORY only — `research/lws-design-records.md` §5): send a
    /// DPoP-bound token but target a PUBLIC resource. This INVERTS the privacy assertion — the
    /// pre-flight asserts the target IS public (anonymous GET 200) — so the sweep measures a
    /// PROOF-CARRYING GET of a public document.
    ///
    /// NOTE: the implemented opt-3 skip does NOT short-circuit this path — the middleware falls
    /// through whenever an `Authorization`/`DPoP` header is present (the credentialed variant is
    /// unsafe: `WAC-Allow user=` is identity-dependent and a forged proof is indistinguishable from
    /// an owner's; see `research/lws-design-records.md` §5). So this mode measures the UNCHANGED
    /// proof-carrying auth path; it is retained ONLY to demonstrate that the ES256 verify is the
    /// cost, NOT as a shippable result. The scenario is named `authed-public-doc`. Ignored in
    /// `anon` mode (the path opt-3 actually changes).
    expect_public: bool,
    duration: Duration,
    warmup: Duration,
    concurrency: Vec<usize>,
    out_dir: String,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Config {
    fn from_env() -> Self {
        let base_url = env_or("AUTH_BASE_URL", "https://localhost:3000");
        let connect_host = env_or("AUTH_CONNECT_HOST", "127.0.0.1");
        let port = base_url
            .rsplit(':')
            .next()
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
            .unwrap_or(3000);
        let connect_base = format!("https://{connect_host}:{port}");
        let issuer = env_or("AUTH_ISSUER", "http://localhost:8080/realms/solid");
        let token_endpoint = env_or(
            "AUTH_TOKEN_ENDPOINT",
            &format!(
                "{}/protocol/openid-connect/token",
                issuer.trim_end_matches('/')
            ),
        );
        let duration =
            Duration::from_secs(env_or("AUTH_DURATION_SECS", "10").parse().unwrap_or(10));
        let warmup = Duration::from_secs(env_or("AUTH_WARMUP_SECS", "3").parse().unwrap_or(3));
        let concurrency = env_or("AUTH_CONCURRENCY", "1 8 16 32 64 128 256 512")
            .split_whitespace()
            .filter_map(|s| s.parse::<usize>().ok())
            .collect();
        Self {
            base_url,
            connect_base,
            token_endpoint,
            client_id: env_or("AUTH_CLIENT_ID", "conformance-alice"),
            client_secret: env_or("AUTH_CLIENT_SECRET", "conformance-alice-secret"),
            target_path: env_or("AUTH_TARGET_PATH", "/alice/test/bench-private"),
            listing_path: env_or("AUTH_LISTING_PATH", "/alice/test/"),
            put_fixture: matches!(
                env_or("AUTH_PUT_FIXTURE", "1").trim(),
                "1" | "true" | "TRUE" | "True"
            ),
            listing_children: env_or("AUTH_LISTING_CHILDREN", "0").parse().unwrap_or(0),
            anon: matches!(
                env_or("AUTH_ANON", "0").trim(),
                "1" | "true" | "TRUE" | "True"
            ),
            expect_public: matches!(
                env_or("AUTH_EXPECT_PUBLIC", "0").trim(),
                "1" | "true" | "TRUE" | "True"
            ),
            duration,
            warmup,
            concurrency,
            out_dir: env_or("AUTH_OUT_DIR", "bench/results"),
        }
    }
}

// --- token mint (client-credentials + token-endpoint DPoP) --------------------------------------

/// Obtain a DPoP-bound access token from the realm. The token endpoint requires a DPoP proof (htu =
/// the token endpoint, htm = POST, no `ath`); the returned token is `cnf.jkt`-bound to `key`.
async fn obtain_token(
    http: &reqwest::Client,
    cfg: &Config,
    key: &ProofKey,
) -> Result<String, String> {
    let proof = key.dpop_proof("POST", &cfg.token_endpoint, None);
    let resp = http
        .post(&cfg.token_endpoint)
        .header("DPoP", proof)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("token body read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("token endpoint {status}: {body}"));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("token JSON parse: {e}"))?;
    v.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("no access_token in token response: {body}"))
}

/// PUT a small private Turtle document to `target_path` (htu = base+path; dialled at the IPv4 base).
/// Uses the SAME owner token + a fresh DPoP proof (htm = PUT). Idempotent: a 201 (create) or 204
/// (overwrite) are both fine. This makes scenario (c) a genuine owner-private DOCUMENT — readable by
/// the token's WebID (alice owns `/alice/test/`) but NOT public (no public grant anywhere on it).
async fn put_fixture(
    http: &reqwest::Client,
    key: &ProofKey,
    token: &str,
    htu: &str,
    dial_url: &str,
) -> Result<(), String> {
    // A tiny, valid Turtle body. We never hand-build ACL/RDF on the SERVER (the house rule); this is
    // a load-CLIENT writing an opaque document body the server stores byte-exact — a benign fixture.
    let body = "<#it> <http://www.w3.org/2000/01/rdf-schema#label> \"bench private fixture\" .\n";
    let proof = key.dpop_proof("PUT", htu, Some(token));
    let resp = http
        .put(dial_url)
        .header("Authorization", format!("DPoP {token}"))
        .header("DPoP", proof)
        .header("Content-Type", "text/turtle")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("PUT fixture request failed: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 201 || status.as_u16() == 204 || status.as_u16() == 205 {
        eprintln!(">> PUT fixture {htu} → {status} (private document created/updated).");
        Ok(())
    } else {
        let b = resp.text().await.unwrap_or_default();
        Err(format!(
            "PUT fixture {htu} returned {status} (expected 201/204): {b}"
        ))
    }
}

/// PUT `count` owner-private children into `listing_path` (so scenario (d)'s authed-listing render
/// has a meaningful membership). Each child is a tiny Turtle document at
/// `<listing_path>auth-item-NNNN`, owned by alice (inherits her pod-root owner ACL). Idempotent: a
/// repeat run overwrites (204). The children make the OWNER-rendered container a real
/// auth+render-combined measurement, scaling render cost with N exactly like the public `oha` listing.
async fn seed_listing_children(
    http: &reqwest::Client,
    key: &ProofKey,
    token: &str,
    cfg: &Config,
    count: usize,
) -> Result<(), String> {
    eprintln!(
        ">> Seeding {count} owner-private children into {} (authed-listing scenario d) ...",
        cfg.listing_path
    );
    let body = "<#it> <http://www.w3.org/2000/01/rdf-schema#label> \"authed listing child\" .\n";
    for i in 0..count {
        let path = format!("{}auth-item-{i:04}", cfg.listing_path);
        let htu = format!("{}{}", cfg.base_url, path);
        let dial = format!("{}{}", cfg.connect_base, path);
        let proof = key.dpop_proof("PUT", &htu, Some(token));
        let resp = http
            .put(&dial)
            .header("Authorization", format!("DPoP {token}"))
            .header("DPoP", proof)
            .header("Content-Type", "text/turtle")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("PUT listing child {htu} failed: {e}"))?;
        let s = resp.status().as_u16();
        if s != 201 && s != 204 && s != 205 {
            let b = resp.text().await.unwrap_or_default();
            return Err(format!(
                "PUT listing child {htu} returned {s} (expected 201/204): {b}"
            ));
        }
    }
    eprintln!(">> Seeded {count} listing children.");
    Ok(())
}

// --- the sweep ----------------------------------------------------------------------------------

/// A single GET. When `token` is `Some`, it is an AUTHED GET: a fresh DPoP proof +
/// `Authorization: DPoP <token>` + `DPoP: <proof>` (the auth-verify hot path). When `token` is
/// `None` it is an ANONYMOUS GET (no headers) — the public-read comparison baseline. Returns the
/// per-request elapsed time and whether it was a 200. Connection reuse comes from the shared
/// `reqwest::Client` pool (keep-alive), so TLS/handshake is amortised exactly like `oha`.
async fn authed_get(
    http: &reqwest::Client,
    key: &ProofKey,
    token: Option<&str>,
    htu: &str,
    dial_url: &str,
) -> (Duration, bool) {
    let mut req = http.get(dial_url);
    if let Some(tok) = token {
        let proof = key.dpop_proof("GET", htu, Some(tok));
        req = req
            .header("Authorization", format!("DPoP {tok}"))
            .header("DPoP", proof);
    }
    let started = Instant::now();
    let result = req.send().await;
    let elapsed = started.elapsed();
    let ok = match result {
        Ok(r) => {
            let status_ok = r.status().as_u16() == 200;
            // Drain the body so the connection returns to the keep-alive pool (else it is closed and
            // the next request re-handshakes — which would inflate latency and mismeasure the path).
            let _ = r.bytes().await;
            status_ok
        }
        Err(_) => false,
    };
    (elapsed, ok)
}

/// Drive `concurrency` worker tasks issuing back-to-back authed GETs against `htu`/`dial_url` for
/// `dur`. Returns (completed, succeeded, sorted per-request latencies in seconds).
async fn run_level(
    http: Arc<reqwest::Client>,
    key: Arc<ProofKey>,
    token: Option<Arc<String>>,
    htu: Arc<String>,
    dial_url: Arc<String>,
    concurrency: usize,
    dur: Duration,
) -> (u64, u64, Vec<f64>) {
    let deadline = Instant::now() + dur;
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let http = http.clone();
        let key = key.clone();
        let token = token.clone();
        let htu = htu.clone();
        let dial_url = dial_url.clone();
        handles.push(tokio::spawn(async move {
            let mut lat: Vec<f64> = Vec::new();
            let mut ok = 0u64;
            while Instant::now() < deadline {
                let tok = token.as_deref().map(String::as_str);
                let (elapsed, success) = authed_get(&http, &key, tok, &htu, &dial_url).await;
                lat.push(elapsed.as_secs_f64());
                if success {
                    ok += 1;
                }
            }
            (lat, ok)
        }));
    }
    let mut all_lat: Vec<f64> = Vec::new();
    let mut succeeded = 0u64;
    for h in handles {
        let (lat, ok) = h.await.expect("worker task panicked");
        succeeded += ok;
        all_lat.extend(lat);
    }
    let completed = all_lat.len() as u64;
    all_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (completed, succeeded, all_lat)
}

/// Percentile (0.0..=1.0) of a SORTED ascending slice, in seconds. Nearest-rank.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Write a result row as a JSON object shaped like the `oha` output the existing harness parses
/// (`summary.requestsPerSec`/`successRate`/`slowest` + `latencyPercentiles.{p50,p99,p99.9}`), so the
/// same downstream reporting (`bench/run.sh`'s `parse_and_record`) applies unchanged. Latencies are
/// in SECONDS (oha's unit), converted to ms by the parser.
fn write_result_json(
    out_dir: &str,
    scenario: &str,
    concurrency: usize,
    completed: u64,
    succeeded: u64,
    sorted: &[f64],
    window: Duration,
) -> Value {
    let rps = completed as f64 / window.as_secs_f64();
    let success_rate = if completed == 0 {
        0.0
    } else {
        succeeded as f64 / completed as f64
    };
    let slowest = sorted.last().copied().unwrap_or(f64::NAN);
    let v = json!({
        "summary": {
            "requestsPerSec": rps,
            "successRate": success_rate,
            "total": completed,
            "slowest": slowest,
        },
        "latencyPercentiles": {
            "p50": percentile(sorted, 0.50),
            "p90": percentile(sorted, 0.90),
            "p99": percentile(sorted, 0.99),
            "p99.9": percentile(sorted, 0.999),
        }
    });
    let path = format!("{out_dir}/{scenario}-c{concurrency}.json");
    if let Err(e) = std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()) {
        eprintln!("  WARN: failed to write {path}: {e}");
    }
    v
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let cfg = Config::from_env();
    std::fs::create_dir_all(&cfg.out_dir).map_err(|e| format!("mkdir out_dir: {e}"))?;

    // One reqwest client with a connection pool (keep-alive) + the self-signed cert accepted (the
    // bench server uses a throwaway localhost cert, exactly as `oha --insecure` does). HTTP/1.1 only
    // (the server has no h2 ALPN) — reqwest negotiates http/1.1 over this rustls stack.
    let http = Arc::new(
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .http1_only()
            .pool_max_idle_per_host(1024)
            .build()
            .map_err(|e| format!("build http client: {e}"))?,
    );

    let key = Arc::new(ProofKey::generate());

    // ANONYMOUS mode: no token (the public-read comparison baseline). AUTHED mode: mint a DPoP-bound
    // token from the realm (the realistic auth path).
    let token: Option<Arc<String>> = if cfg.anon {
        eprintln!(">> ANONYMOUS mode: no token (public-read comparison baseline).");
        None
    } else {
        eprintln!(
            ">> Minting a DPoP-bound token from {} ({})",
            cfg.token_endpoint, cfg.client_id
        );
        let t = Arc::new(obtain_token(&http, &cfg, &key).await?);
        eprintln!(">> Token obtained (len={} chars).", t.len());
        if std::env::var("AUTH_DEBUG").is_ok() {
            if let Some(claims_b64) = t.split('.').nth(1) {
                let pad = (4 - claims_b64.len() % 4) % 4;
                let padded = format!("{claims_b64}{}", "=".repeat(pad));
                if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE.decode(&padded) {
                    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                        eprintln!(
                            "   DEBUG token claims: iss={} aud={} webid={} cnf={} exp={}",
                            v.get("iss").unwrap_or(&Value::Null),
                            v.get("aud").unwrap_or(&Value::Null),
                            v.get("webid").unwrap_or(&Value::Null),
                            v.get("cnf").unwrap_or(&Value::Null),
                            v.get("exp").unwrap_or(&Value::Null),
                        );
                    }
                }
            }
        }
        Some(t)
    };

    // The DPoP htu is the BASE_URL identity (localhost:PORT), but we DIAL the IPv4 literal (see the
    // IPv6-localhost trap in bench/README). htu == base_url + path; dial == connect_base + path.
    let target_htu = Arc::new(format!("{}{}", cfg.base_url, cfg.target_path));
    let target_dial = Arc::new(format!("{}{}", cfg.connect_base, cfg.target_path));

    // --- FIXTURE setup (authed mode only). Create the owner-private DOCUMENT scenario (c) measures,
    // and (if requested) PUT `listing_children` owner-private children into the listing container so
    // scenario (d) renders a meaningful membership. The owner (alice) has Read/Write under
    // `/alice/test/` via the conformance seed's pod-root `acl:default`, so these are private (no
    // public grant) yet readable by the token's WebID.
    if let Some(tok) = token.as_deref() {
        if cfg.put_fixture {
            put_fixture(&http, &key, tok, &target_htu, &target_dial).await?;
        }
        if cfg.listing_children > 0 && !cfg.listing_path.is_empty() {
            seed_listing_children(&http, &key, tok, &cfg, cfg.listing_children).await?;
        }
    }

    // --- PRE-FLIGHT: a single GET MUST land on the path we intend to measure (a misbuilt proof would
    // otherwise measure the 401 path, not the auth path). In AUTHED mode the target must be 200 AND
    // 401-anonymous (genuinely private); in ANON mode it must be 200 anonymously (a public resource).
    {
        let tok = token.as_deref().map(String::as_str);
        let (elapsed, ok) = authed_get(&http, &key, tok, &target_htu, &target_dial).await;
        let _ = elapsed;
        if !ok {
            return Err(format!(
                "PRE-FLIGHT FAILED: {} GET {target_htu} was not 200. A non-200 means the sweep \
                 would measure the wrong path. (Re-run with AUTH_DEBUG=1 to inspect the token.)",
                if cfg.anon { "anonymous" } else { "authed" }
            ));
        }
        eprintln!(
            ">> Pre-flight OK: {} GET {target_htu} → 200.",
            if cfg.anon { "anonymous" } else { "authed" }
        );
    }
    if !cfg.anon && !cfg.expect_public {
        // PRIVACY ASSERTION: the authed target MUST be 401 anonymously, else we are measuring a
        // PUBLIC resource (the wrong path). An anonymous GET (no Authorization/DPoP) must be rejected.
        let (_, anon_ok) = authed_get(&http, &key, None, &target_htu, &target_dial).await;
        if anon_ok {
            return Err(format!(
                "PRIVACY CHECK FAILED: anonymous GET {target_htu} was 200 (expected 401). The \
                 target is not owner-private — the sweep would measure a public read, not the auth path."
            ));
        }
        eprintln!(
            ">> Privacy OK: anonymous GET {target_htu} → not-200 (resource is owner-private)."
        );
    } else if cfg.expect_public {
        // AUTHED-PUBLIC mode (EXPLORATORY — NOT the implemented skip path): the INVERSE assertion —
        // the target MUST be 200 anonymously (a genuinely public resource), so the proof-carrying sweep
        // measures the UNCHANGED auth path (the middleware never skips a credentialed read).
        let (_, anon_ok) = authed_get(&http, &key, None, &target_htu, &target_dial).await;
        if !anon_ok {
            return Err(format!(
                "PUBLIC CHECK FAILED: anonymous GET {target_htu} was not 200 (expected a public \
                 resource). AUTH_EXPECT_PUBLIC=1 measures a PROOF-CARRYING GET of a PUBLIC document."
            ));
        }
        eprintln!(
            ">> Public OK: anonymous GET {target_htu} → 200 (resource is public; measuring the \
             EXPLORATORY proof-carrying authed-public path — NOT skipped by the middleware)."
        );
    }

    // Scenario naming: authed vs anon prefix so the two runs' JSON files never collide in one out dir.
    // `authed-public-doc` is the EXPLORATORY proof-carrying public read (the middleware does NOT skip
    // a credentialed request — see `research/lws-design-records.md` §5); `anon-public-doc` is the
    // path opt-3 implements.
    let doc_name = if cfg.anon {
        "anon-public-doc"
    } else if cfg.expect_public {
        "authed-public-doc"
    } else {
        "authed-private-doc"
    };
    let listing_name = if cfg.anon {
        "anon-listing"
    } else {
        "authed-listing"
    };

    // Scenarios to run: (c) the document; (d) the container listing if configured.
    let mut scenarios: Vec<(&str, Arc<String>, Arc<String>)> =
        vec![(doc_name, target_htu.clone(), target_dial.clone())];
    if !cfg.listing_path.is_empty() {
        scenarios.push((
            listing_name,
            Arc::new(format!("{}{}", cfg.base_url, cfg.listing_path)),
            Arc::new(format!("{}{}", cfg.connect_base, cfg.listing_path)),
        ));
    }

    eprintln!(
        ">> Sweep: concurrency={:?} duration={:?} warmup={:?}",
        cfg.concurrency, cfg.duration, cfg.warmup
    );
    println!("scenario\tconcurrency\trps\tsuccess_rate\tp50_ms\tp99_ms\tp999_ms\tslowest_ms");

    for (name, htu, dial) in &scenarios {
        eprintln!("\n>> Scenario ({name}): {htu}");
        // One discarded warm-up at mid concurrency to prime the keep-alive pool.
        let _ = run_level(
            http.clone(),
            key.clone(),
            token.clone(),
            htu.clone(),
            dial.clone(),
            16,
            cfg.warmup,
        )
        .await;

        for &c in &cfg.concurrency {
            let (completed, succeeded, sorted) = run_level(
                http.clone(),
                key.clone(),
                token.clone(),
                htu.clone(),
                dial.clone(),
                c,
                cfg.duration,
            )
            .await;
            let v = write_result_json(
                &cfg.out_dir,
                name,
                c,
                completed,
                succeeded,
                &sorted,
                cfg.duration,
            );
            let s = &v["summary"];
            let lp = &v["latencyPercentiles"];
            let ms = |x: &Value| x.as_f64().map(|f| f * 1000.0).unwrap_or(f64::NAN);
            println!(
                "{name}\t{c}\t{:.1}\t{:.4}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
                s["requestsPerSec"].as_f64().unwrap_or(0.0),
                s["successRate"].as_f64().unwrap_or(0.0),
                ms(&lp["p50"]),
                ms(&lp["p99"]),
                ms(&lp["p99.9"]),
                ms(&s["slowest"]),
            );
            eprintln!(
                "  [{name} c={c}] rps={:.0} success={:.3} p50={:.3}ms p99={:.3}ms p999={:.3}ms",
                s["requestsPerSec"].as_f64().unwrap_or(0.0),
                s["successRate"].as_f64().unwrap_or(0.0),
                ms(&lp["p50"]),
                ms(&lp["p99"]),
                ms(&lp["p99.9"]),
            );
        }
    }

    eprintln!("\n>> DONE. Per-level JSON in {}/", cfg.out_dir);
    Ok(())
}

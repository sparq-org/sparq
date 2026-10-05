// AUTHORED-BY Claude Opus 4.8
//! Config-gated TLS termination for the experimental server.
//!
//! ## What this does
//! When `SOLID_SERVER_TLS_CERT` **and** `SOLID_SERVER_TLS_KEY` (PEM file paths) are BOTH set, the
//! binary terminates TLS itself (HTTPS) using [`axum_server`] over the house rustls/aws-lc-rs stack.
//! When NEITHER is set, the binary keeps its plain-TCP listener (unchanged dev/test behaviour, and
//! the "terminate TLS at a reverse proxy" posture). Setting exactly ONE is a configuration error and
//! is rejected at boot — a half-configured TLS server is never silently downgraded to plaintext.
//!
//! ## Why PEM file paths via env (the config-shape decision)
//! TLS material is supplied as two PEM files referenced by absolute-or-relative path through the env
//! vars above: a cert chain file and a private-key file. This matches the existing `SOLID_SERVER_*`
//! env-driven configuration style, keeps secrets OUT of the process arguments / the repo, and is the
//! shape every TLS-terminating proxy (Caddy/nginx/Envoy) already speaks, so an operator can point at
//! the same files. We deliberately do NOT do auto-cert / ACME / in-process Let's Encrypt in this
//! slice — that is a future seam: an ACME provider would produce the same in-memory rustls
//! `ServerConfig` this module already builds, so it can be added behind a third env var without
//! reshaping the serve path. Cert reload-on-rotation (axum-server's `RustlsConfig::reload_from_*`)
//! is likewise a future seam.
//!
//! ## Crypto provider
//! `axum-server`'s `tls-rustls-no-provider` feature is used on purpose: it does NOT install its own
//! rustls crypto provider, so the process-wide aws-lc-rs default provider installed in `main` (also
//! used by the SSRF-guarded fetcher) is the single provider in the tree. The `RustlsConfig` builder
//! picks that provider up. We validate at boot that a provider is installed before building a config,
//! so a misorder surfaces as a clear error rather than a runtime panic on the first handshake.
//!
//! ## ALPN — HTTP/2 (`h2`) + HTTP/1.1, owned here (not inherited)
//! The `ServerConfig.alpn_protocols` advertised in the TLS handshake is set EXPLICITLY by this module
//! to [`ALPN_PROTOCOLS`] = `["h2", "http/1.1"]`, in preference order. ALPN is a NEGOTIATION: an
//! `h2`-capable client gets HTTP/2 (multiplexed streams + header compression over a single connection
//! — fewer TLS handshakes per client, a real authed-RPS/latency win for many small requests); an
//! HTTP/1.1-only client offers no `h2` and negotiates down to `http/1.1` transparently. h2 is purely
//! ADDITIVE — it changes the TRANSPORT, never the LDP/auth/WAC SEMANTICS (the handler layer is
//! version-agnostic: it sees an `http::Request` either way), so conformance (an HTTP/1.1 harness) is
//! unaffected. axum-server's [`auto::Builder`](https://docs.rs/hyper-util) serves whichever protocol
//! ALPN selected.
//!
//! We set this OURSELVES rather than relying on axum-server's `RustlsConfig::from_pem` default
//! (which today also sets `[h2, http/1.1]`) on purpose: the ALPN set is a load-bearing transport
//! contract, so it must be a documented, TESTED invariant of THIS crate — not a transitive
//! implementation detail of a dependency that a version bump (or a future swap to an ACME /
//! `from_config` cert path) could silently drop. [`build_rustls_config`] re-asserts it after building
//! the config, so the advertised protocols are always exactly what this module declares.
//!
//! ## TLS session resumption — env-tunable cache size (throughput lever), 0-RTT stays OFF
//! rustls's `ServerConfig` defaults to a **256-session** in-memory resumption cache — only a handful
//! of concurrent clients before eviction forces expensive full handshakes on the anonymous-read hot
//! path. [`ENV_TLS_SESSION_CACHE_SIZE`] makes that cache size env-tunable (default
//! [`DEFAULT_TLS_SESSION_CACHE_SIZE`] = 10 240; `0` disables resumption entirely). A larger cache lets
//! more returning clients complete an ABBREVIATED (resumed) handshake — skipping the asymmetric key
//! exchange — which is the connection-amortization half of the beyond-50k throughput plan
//! (`research/lws-design-records.md` §7). This is a pure PERFORMANCE knob: it changes no
//! LDP/auth/WAC semantics, and it does **NOT** enable TLS 0-RTT early data — `max_early_data_size`
//! stays `0`. 0-RTT is replayable by design, which is incoherent under this server's anti-replay DPoP
//! (`jti`) model, so it is never turned on here (RSS `docs/design/beyond-50k-throughput.md` §5,
//! not in this tree). The tuning is installed uniformly on both build paths (default + mTLS) by
//! `apply_transport_tuning`.
//!
//! ## TLS session resumption — the stateless-`Ticketer` half (opt-in), 0-RTT STILL OFF
//! By default rustls keeps its `NeverProducesTickets` ticketer, so TLS 1.3 resumption is STATEFUL
//! (backed by the `session_storage` cache above; `cache_size == 0` disables it). [`ENV_TLS_STATELESS_TICKETS`]
//! (affirmative opt-in, default OFF) instead installs the aws-lc-rs RFC 5077 [`rustls::crypto::aws_lc_rs::Ticketer`]
//! — AES-256-CBC + HMAC-SHA256, RANDOM per-process keys rotated every ~6h (≈12h ticket life, forward-secret
//! by key erasure) — so a returning client resumes STATELESSLY from an encrypted ticket, with NO server-side
//! per-session memory. This is the second half of the beyond-50k P1.3 connection-amortization lever
//! (`research/lws-design-records.md` §7). It stays OPT-IN and default OFF on purpose: a
//! per-process ticket key is NOT shared across a horizontally-scaled fleet, so tickets issued by one node
//! do not resume on another (a cross-node full-handshake fallback — a perf, never a correctness, effect;
//! and the shared-key design belongs with the horizontal-scale / shared-replay work). Crucially, a
//! ticketer does **NOT** enable 0-RTT: [`max_early_data_size`](rustls::ServerConfig) is still forced to
//! `0` in `apply_transport_tuning` even when the ticketer is on, so the anti-replay DPoP invariant holds
//! regardless. Building the ticketer is FAIL-SAFE — an RNG error logs and falls back to the stateful cache,
//! never a boot failure.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum_server::tls_rustls::RustlsConfig;
// [OPUS-5] sq-5ah3p: PEM decoding comes from `rustls-pki-types`' `PemObject` trait, reached through
// rustls' own `pki_types` re-export rather than a separate direct dependency — that spelling makes
// version skew between the DER types rustls accepts and the ones we decode structurally impossible,
// and it retires the archived `rustls-pemfile` shim (RUSTSEC-2025-0134), which since 2.x has been a
// thin legacy wrapper over exactly these APIs.
use rustls::pki_types::pem::{Error as PemError, PemObject};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Env var naming the PEM **certificate chain** file (leaf first). Set together with [`ENV_TLS_KEY`].
pub const ENV_TLS_CERT: &str = "SOLID_SERVER_TLS_CERT";
/// Env var naming the PEM **private key** file (PKCS#8 or PKCS#1). Set together with [`ENV_TLS_CERT`].
pub const ENV_TLS_KEY: &str = "SOLID_SERVER_TLS_KEY";

/// Env var enabling the PoP Tier-1b RFC 8705 mTLS-bound-token path: when set truthy AND in-process
/// TLS is on, the TLS handshake REQUESTS (but does not REQUIRE) a client certificate, so a cert-bound
/// token can be matched against the presented cert's `cnf.x5t#S256` thumbprint. **Default OFF** — when
/// unset/falsy the TLS + plain serve paths are byte-identical to the pre-Tier-1b behaviour (no client
/// cert requested, no confirmation dispatch). See [`mtls_bound_tokens_from_env`] +
/// [`build_rustls_config`]. Design: `research/lws-design-records.md` §7 (bead 2).
pub const ENV_MTLS_BOUND_TOKENS: &str = "SOLID_SERVER_MTLS_BOUND_TOKENS";

/// Env var tuning the size of the in-memory TLS **session-resumption cache** (rustls's
/// `ServerSessionMemoryCache`): the maximum number of resumable TLS sessions the server remembers so
/// a returning client can complete an ABBREVIATED (resumed) handshake — skipping the asymmetric key
/// exchange — instead of a full one. Parsed as a non-negative integer:
/// - unset / empty / unparseable ⇒ [`DEFAULT_TLS_SESSION_CACHE_SIZE`] (the tuned default),
/// - `0` ⇒ resumption DISABLED (a [`rustls::server::NoServerSessionStorage`] is installed, so every
///   handshake is full),
/// - `N` ⇒ remember up to ≈`N` sessions, clamped to [`MAX_TLS_SESSION_CACHE_SIZE`].
///
/// This is a pure THROUGHPUT knob — the connection-amortization half of the beyond-50k plan
/// (`research/lws-design-records.md` §7): a larger cache lets more distinct concurrent
/// clients resume before eviction forces a full handshake, exactly as a connection-bound PoP check
/// amortizes per-request asymmetric verifies. It changes NO LDP/auth/WAC semantics and, crucially,
/// does NOT enable TLS 0-RTT early data — `max_early_data_size` stays `0`, because 0-RTT is replayable
/// by design and this server's auth model is anti-replay (DPoP `jti`). See the module docs.
pub const ENV_TLS_SESSION_CACHE_SIZE: &str = "SOLID_SERVER_TLS_SESSION_CACHE_SIZE";

/// Default TLS session-resumption cache size when [`ENV_TLS_SESSION_CACHE_SIZE`] is unset. Chosen well
/// above rustls's own 256-session default (only a handful of concurrent clients before eviction forces
/// full handshakes) so a realistic concurrent-client population resumes. Each stored session is a few
/// hundred bytes, so ~10k sessions is a low-single-digit-MB memory ceiling — cheap insurance against
/// the eviction cliff on the anonymous-read hot path.
pub const DEFAULT_TLS_SESSION_CACHE_SIZE: usize = 10_240;

/// Upper clamp on [`ENV_TLS_SESSION_CACHE_SIZE`]. `ServerSessionMemoryCache::new(n)` PRE-ALLOCATES a
/// map + deque sized to `n` at boot, so an absurd operator value (a typo'd `1000000000`) would try to
/// reserve gigabytes before serving a single request. The cap bounds boot-time allocation while still
/// permitting a very large (≈1M-session) cache for a big single-node deployment.
pub const MAX_TLS_SESSION_CACHE_SIZE: usize = 1 << 20; // 1,048,576

/// Env var enabling STATELESS TLS resumption tickets (the deferred half of beyond-50k P1.3). Affirmative
/// opt-in — truthy (`1`/`true`/`yes`/`on`, case-insensitive, trimmed) ⇒ install the aws-lc-rs RFC 5077
/// [`rustls::crypto::aws_lc_rs::Ticketer`] as the server's `ticketer`, so TLS 1.3 resumption becomes
/// STATELESS (an encrypted ticket, no server-side per-session memory) instead of the default STATEFUL
/// `session_storage`-backed path. Everything else (absent, empty, `0`, any other string) ⇒ **OFF** (the
/// default), keeping the byte-identical stateful-cache posture. Default OFF because a per-process ticket
/// key is not shared across a horizontally-scaled fleet (cross-node resumption falls back to a full
/// handshake — a perf, not correctness, effect) and belongs with the shared-replay / horizontal-scale
/// design. Enabling it does NOT turn on 0-RTT early data (`max_early_data_size` stays `0`), so the
/// anti-replay DPoP invariant is preserved either way. See [`stateless_tickets_from_env`] +
/// `apply_transport_tuning` and the module docs.
pub const ENV_TLS_STATELESS_TICKETS: &str = "SOLID_SERVER_TLS_STATELESS_TICKETS";

/// The shared affirmative-opt-in grammar for the boolean security/mechanism env flags: truthy =
/// `1`/`true`/`yes`/`on` (case-insensitive, surrounding-whitespace-trimmed); EVERYTHING else — absent
/// (`None`), empty, `0`, `false`, or any other string — is `false`. A typo therefore NEVER silently
/// enables a gated path (fail-safe to OFF). Used by both [`mtls_bound_tokens_from_env`] (requesting
/// client certs) and [`stateless_tickets_from_env`] so the grammar is defined once.
fn is_affirmative(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim),
        Some(s) if s.eq_ignore_ascii_case("1")
            || s.eq_ignore_ascii_case("true")
            || s.eq_ignore_ascii_case("yes")
            || s.eq_ignore_ascii_case("on")
    )
}

/// Whether the RFC 8705 mTLS-bound-token path is enabled ([`ENV_MTLS_BOUND_TOKENS`]). Truthy =
/// `1`/`true`/`yes`/`on` (case-insensitive, trimmed); everything else (absent, empty, `0`, `false`,
/// any other string) ⇒ **OFF** (the fail-safe default — a typo never silently enables a security path
/// that requests client certs). Uses the shared `is_affirmative` opt-in grammar.
pub fn mtls_bound_tokens_from_env() -> bool {
    is_affirmative(std::env::var(ENV_MTLS_BOUND_TOKENS).ok().as_deref())
}

/// Whether STATELESS TLS resumption tickets are enabled ([`ENV_TLS_STATELESS_TICKETS`]). Truthy ⇒ install
/// the aws-lc-rs RFC 5077 ticketer; everything else ⇒ **OFF** (the default, stateful-cache resumption).
/// Uses the shared `is_affirmative` opt-in grammar — this is a mechanism CHANGE (stateless vs stateful
/// resumption) so, like the mTLS flag, a typo must fail safe to the conservative default (OFF).
pub fn stateless_tickets_from_env() -> bool {
    is_affirmative(std::env::var(ENV_TLS_STATELESS_TICKETS).ok().as_deref())
}

/// Read + parse [`ENV_TLS_SESSION_CACHE_SIZE`] into the session-cache size to install.
pub fn session_cache_size_from_env() -> usize {
    parse_session_cache_size(std::env::var(ENV_TLS_SESSION_CACHE_SIZE).ok().as_deref())
}

/// The testable core of [`session_cache_size_from_env`]. `None` / empty / unparseable ⇒ the tuned
/// [`DEFAULT_TLS_SESSION_CACHE_SIZE`]; a parsed value is clamped to [`MAX_TLS_SESSION_CACHE_SIZE`];
/// `0` is honoured verbatim as "resumption disabled".
///
/// FAIL-SAFE (not fail-closed): a garbage value falls back to the good DEFAULT rather than breaking
/// boot. This is deliberate and the OPPOSITE of [`mtls_bound_tokens_from_env`]'s affirmative-opt-in
/// grammar — the mTLS flag gates a SECURITY posture (requesting client certs), so a typo must fail
/// safe to OFF; this is a non-security THROUGHPUT knob, so a typo must never take the server down, it
/// just reverts to the sensible default cache size. Resumption is a performance optimisation whose
/// worst case (a smaller/absent cache) is only "more full handshakes", never a correctness or
/// security change.
pub fn parse_session_cache_size(raw: Option<&str>) -> usize {
    match raw.map(str::trim) {
        None | Some("") => DEFAULT_TLS_SESSION_CACHE_SIZE,
        Some(s) => match s.parse::<usize>() {
            Ok(n) => n.min(MAX_TLS_SESSION_CACHE_SIZE),
            Err(_) => DEFAULT_TLS_SESSION_CACHE_SIZE,
        },
    }
}

/// The transport-layer TLS resumption tuning applied uniformly to a built config by
/// `apply_transport_tuning`. Carries BOTH beyond-50k P1.3 halves so the two build paths converge on one
/// shape: the stateful [`session_cache_size`](Self::session_cache_size) and the opt-in stateless
/// [`stateless_tickets`](Self::stateless_tickets). Neither touches LDP/auth/WAC semantics and neither
/// enables 0-RTT (kept OFF unconditionally in `apply_transport_tuning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportTuning {
    /// Max resumable TLS sessions kept in the in-memory `ServerSessionMemoryCache` (`0` ⇒ resumption
    /// disabled — a `NoServerSessionStorage`). See [`ENV_TLS_SESSION_CACHE_SIZE`].
    pub session_cache_size: usize,
    /// Install the aws-lc-rs RFC 5077 stateless resumption [`rustls::crypto::aws_lc_rs::Ticketer`]
    /// (encrypted ticket, no server-side per-session memory) instead of the default stateful cache-backed
    /// path. See [`ENV_TLS_STATELESS_TICKETS`]. FAIL-SAFE: a ticketer build error falls back to the cache.
    pub stateless_tickets: bool,
}

impl TransportTuning {
    /// Resolve the tuning entirely from the environment: [`session_cache_size_from_env`] +
    /// [`stateless_tickets_from_env`]. This is what [`build_rustls_config`] uses.
    pub fn from_env() -> Self {
        Self {
            session_cache_size: session_cache_size_from_env(),
            stateless_tickets: stateless_tickets_from_env(),
        }
    }
}

/// The ALPN protocols advertised in the TLS handshake, in server preference order: HTTP/2 (`h2`)
/// FIRST, then HTTP/1.1. An `h2`-capable client negotiates HTTP/2 (multiplexing + header
/// compression); an HTTP/1.1-only client negotiates down to `http/1.1` (h2 is additive, never
/// required). The byte strings are the IANA ALPN protocol IDs (RFC 7301 / RFC 9113 §3.1). This is the
/// owned, tested transport contract — see the module docs.
pub const ALPN_PROTOCOLS: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// The resolved TLS configuration intent, derived from the two env vars.
///
/// `Plain` ⇒ neither var set ⇒ keep the plain-TCP listener. `Tls` ⇒ both set ⇒ terminate HTTPS.
/// "Exactly one set" never produces a value — it is a [`TlsConfigError::Incomplete`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsMode {
    /// No TLS env configured — serve plaintext over TCP (dev/test, or TLS-at-a-proxy).
    Plain,
    /// Both PEM paths configured — terminate TLS in-process.
    Tls {
        cert_path: PathBuf,
        key_path: PathBuf,
    },
}

/// A boot-time TLS configuration error. Each variant carries enough context for a clear operator
/// message (which var, which path, the underlying cause).
#[derive(Debug)]
pub enum TlsConfigError {
    /// Exactly one of the cert/key env vars is set — both-or-neither is required.
    Incomplete {
        present: &'static str,
        missing: &'static str,
    },
    /// A referenced PEM file is missing or unreadable.
    Unreadable {
        which: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    /// A referenced PEM file is present but empty / contains no usable PEM material.
    Empty { which: &'static str, path: PathBuf },
    /// The cert+key were read but rustls could not build a server config from them (malformed PEM,
    /// key/cert mismatch, unsupported key type, …).
    Malformed { source: std::io::Error },
    /// No rustls crypto provider is installed in the process (install it before building TLS config).
    NoCryptoProvider,
}

impl fmt::Display for TlsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete { present, missing } => write!(
                f,
                "TLS misconfigured: {present} is set but {missing} is not — set BOTH (PEM cert + key \
                 file paths) to enable HTTPS, or NEITHER for plain HTTP"
            ),
            Self::Unreadable { which, path, source } => write!(
                f,
                "TLS {which} file is missing or unreadable: {} ({source})",
                path.display()
            ),
            Self::Empty { which, path } => write!(
                f,
                "TLS {which} file is empty / contains no PEM material: {}",
                path.display()
            ),
            Self::Malformed { source } => write!(
                f,
                "TLS cert/key could not be loaded (malformed PEM, key/cert mismatch, or unsupported \
                 key type): {source}"
            ),
            Self::NoCryptoProvider => write!(
                f,
                "no rustls crypto provider installed — install the aws-lc-rs default provider before \
                 building TLS config"
            ),
        }
    }
}

impl std::error::Error for TlsConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } | Self::Malformed { source } => Some(source),
            _ => None,
        }
    }
}

/// Resolve the TLS mode from the two env vars, applying the both-or-neither rule.
///
/// This is pure config parsing — it does NOT touch the filesystem (that is [`build_rustls_config`]'s
/// job), so a caller can distinguish "you set one of the pair" (a fast, dependency-free error) from
/// "the file is bad". A present-but-blank value is treated as set (an operator who exports
/// `SOLID_SERVER_TLS_CERT=` clearly intends TLS and should get the incomplete/empty error, not a
/// silent plaintext downgrade).
///
/// We read with [`std::env::var_os`] (NOT [`std::env::var`]) on purpose: `var` returns `Err` — which
/// `.ok()` would flatten to `None`, i.e. "absent" — for a value that is PRESENT but not valid Unicode.
/// Treating a present-but-non-Unicode TLS path as absent would let TWO non-Unicode paths silently
/// fall back to plaintext, violating the both-or-neither fail-closed rule. `var_os` returns the raw
/// `OsString` so a present path is honoured regardless of encoding (a path is an `OsStr`, not a
/// `String`, anyway — so this is also the correct type for a filesystem path).
pub fn mode_from_env() -> Result<TlsMode, TlsConfigError> {
    let cert = std::env::var_os(ENV_TLS_CERT);
    let key = std::env::var_os(ENV_TLS_KEY);
    mode_from_values(cert.as_deref(), key.as_deref())
}

/// The testable core of [`mode_from_env`]: resolve the mode from explicit option values. `None`
/// means the var is absent; `Some("")` (or whitespace) means it is set-but-blank (still "set", which
/// makes the both-or-neither rule fail closed rather than downgrading to plaintext).
///
/// Takes `Option<&OsStr>` (not `Option<&str>`) so a present-but-non-Unicode path is honoured — never
/// mistaken for "absent" and silently downgraded to plaintext (the fail-closed invariant). The
/// `OsString` is carried straight into a `PathBuf`, which is exactly its target type.
pub fn mode_from_values(
    cert: Option<&OsStr>,
    key: Option<&OsStr>,
) -> Result<TlsMode, TlsConfigError> {
    match (cert, key) {
        (None, None) => Ok(TlsMode::Plain),
        (Some(c), Some(k)) => Ok(TlsMode::Tls {
            cert_path: PathBuf::from(trim_os(c)),
            key_path: PathBuf::from(trim_os(k)),
        }),
        (Some(_), None) => Err(TlsConfigError::Incomplete {
            present: ENV_TLS_CERT,
            missing: ENV_TLS_KEY,
        }),
        (None, Some(_)) => Err(TlsConfigError::Incomplete {
            present: ENV_TLS_KEY,
            missing: ENV_TLS_CERT,
        }),
    }
}

/// Trim leading/trailing ASCII whitespace from an `OsStr` without requiring it to be valid Unicode.
///
/// We can't call `str::trim` (the value may be non-Unicode), but a path's leading/trailing ASCII
/// whitespace is byte-identifiable on every platform whose `OsStr` is byte-based; on platforms where
/// it is not (e.g. Windows' WTF-8), a Unicode value still trims via the lossy round-trip and a
/// non-Unicode value is returned verbatim (no silent corruption). The common operator case — a path
/// with stray surrounding whitespace from a shell export — is handled, while a present non-Unicode
/// path is preserved intact rather than dropped.
fn trim_os(value: &OsStr) -> OsString {
    match value.to_str() {
        // Valid Unicode: trim like before (covers the common shell-export-with-whitespace case).
        Some(s) => OsString::from(s.trim()),
        // Non-Unicode: cannot safely byte-trim across platforms — honour the path verbatim. The
        // fail-closed point is that it is USED, never dropped.
        None => value.to_os_string(),
    }
}

/// Read + validate the PEM files referenced by a [`TlsMode::Tls`] and build the rustls config.
///
/// Validation is explicit and ordered so the boot error names the precise problem:
/// 1. each file is readable (missing/permission → [`TlsConfigError::Unreadable`]),
/// 2. each file is non-empty ([`TlsConfigError::Empty`]),
/// 3. a crypto provider is installed ([`TlsConfigError::NoCryptoProvider`]),
/// 4. rustls can build a `ServerConfig` from the bytes ([`TlsConfigError::Malformed`]).
///
/// On [`TlsMode::Plain`] this returns `Ok(None)` — there is nothing to build.
///
/// `mtls_bound_tokens` selects the client-certificate posture (PoP Tier-1b, [`ENV_MTLS_BOUND_TOKENS`]):
/// - `false` (**the default**) — the byte-identical pre-Tier-1b path: `RustlsConfig::from_pem`, which
///   builds a `ServerConfig` with `with_no_client_auth()` (no `CertificateRequest` is sent, so no
///   client presents a cert). Nothing about the handshake changes.
/// - `true` — build the `ServerConfig` ourselves with a client-certificate verifier that REQUESTS (but
///   does NOT require) a client certificate and does NOT chain-validate it (RFC 8705 §2.2 self-signed
///   flavour: trust is the `cnf.x5t#S256` thumbprint match enforced downstream, NOT the chain). Key
///   POSSESSION is still proven by the handshake `CertificateVerify` signature (verified against the
///   presented cert's own key). Plain-DPoP clients that present NO cert are unaffected (optional auth).
///
/// The resumption tuning (session-cache size + stateless-ticket opt-in) is read from the environment
/// via [`TransportTuning::from_env`] ([`ENV_TLS_SESSION_CACHE_SIZE`] + [`ENV_TLS_STATELESS_TICKETS`]);
/// [`build_rustls_config_with_tuning`] is the explicit-tuning core (used by the resumption-count tests to
/// build configs deterministically without touching process-global env), and
/// [`build_rustls_config_with_session_cache_size`] is a thin cache-size-only wrapper over it.
pub async fn build_rustls_config(
    mode: &TlsMode,
    mtls_bound_tokens: bool,
) -> Result<Option<RustlsConfig>, TlsConfigError> {
    build_rustls_config_with_tuning(mode, mtls_bound_tokens, TransportTuning::from_env()).await
}

/// Thin cache-size-only wrapper over [`build_rustls_config_with_tuning`]: keeps the pre-ticketer public
/// signature (used by the resumption-count integration tests) by supplying `stateless_tickets: false`, so
/// its behaviour is byte-identical to before the ticketer half landed — the default STATEFUL,
/// `session_storage`-backed resumption. `session_cache_size` is the maximum number of resumable TLS
/// sessions to remember (`0` disables resumption); see [`ENV_TLS_SESSION_CACHE_SIZE`].
pub async fn build_rustls_config_with_session_cache_size(
    mode: &TlsMode,
    mtls_bound_tokens: bool,
    session_cache_size: usize,
) -> Result<Option<RustlsConfig>, TlsConfigError> {
    build_rustls_config_with_tuning(
        mode,
        mtls_bound_tokens,
        TransportTuning {
            session_cache_size,
            stateless_tickets: false,
        },
    )
    .await
}

/// The explicit-[`TransportTuning`] core of [`build_rustls_config`] (which reads the tuning from env via
/// [`TransportTuning::from_env`]). `tuning` carries the session-cache size (`0` disables the stateful
/// cache) and the stateless-ticket opt-in; both are applied by `apply_transport_tuning`. Exposed so
/// tests can build configs with a chosen tuning deterministically, without mutating process-global env
/// (which would race other parallel tests in the same binary).
pub async fn build_rustls_config_with_tuning(
    mode: &TlsMode,
    mtls_bound_tokens: bool,
    tuning: TransportTuning,
) -> Result<Option<RustlsConfig>, TlsConfigError> {
    let (cert_path, key_path) = match mode {
        TlsMode::Plain => return Ok(None),
        TlsMode::Tls {
            cert_path,
            key_path,
        } => (cert_path, key_path),
    };

    let cert = read_pem("certificate", cert_path).await?;
    let key = read_pem("private key", key_path).await?;

    // Guard: building a rustls ServerConfig requires an installed crypto provider. Checking here
    // turns a first-handshake panic into a clear boot error.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        return Err(TlsConfigError::NoCryptoProvider);
    }

    let config = if mtls_bound_tokens {
        // Tier-1b: build our own ServerConfig with the optional self-signed client-cert verifier. This
        // path cannot use `RustlsConfig::from_pem` (it hard-codes `with_no_client_auth`), so we parse
        // the PEM ourselves and hand the built config to `RustlsConfig::from_config`.
        let server_config = build_mtls_server_config(&cert, &key)?;
        RustlsConfig::from_config(Arc::new(server_config))
    } else {
        // Default path — byte-identical to pre-Tier-1b. `from_pem` builds a rustls ServerConfig (using
        // the installed default provider, `with_no_client_auth`) and surfaces a malformed-PEM /
        // key-mismatch as an io::Error — mapped to a clear Malformed boot error.
        RustlsConfig::from_pem(cert, key)
            .await
            .map_err(|source| TlsConfigError::Malformed { source })?
    };

    // Finalize the transport tuning on the built config, uniformly across both build paths above:
    // (1) own the ALPN advertisement explicitly (do not inherit axum-server's `from_pem` default) —
    // `[h2, http/1.1]` so an h2-capable client gets HTTP/2 and an h1-only client negotiates down;
    // (2) install the session-resumption cache of `tuning.session_cache_size` (0 ⇒ stateful cache
    // disabled); and (3) optionally install the stateless RFC 5077 ticketer (`tuning.stateless_tickets`).
    // Re-asserting all here means a dependency bump or a future ACME/`from_config` cert path (incl.
    // the mTLS branch above) can never silently change the advertised protocol set OR the resumption
    // posture. 0-RTT early data stays OFF (`max_early_data_size` forced to 0 — see the module docs).
    apply_transport_tuning(&config, tuning);
    Ok(Some(config))
}

/// Build the Tier-1b mTLS `ServerConfig`: our own leaf cert/key + the optional self-signed
/// client-certificate verifier ([`SelfSignedOptionalClientCertVerifier`]). Uses the process-wide
/// default crypto provider (installed in `main`) both for the config builder and for the verifier's
/// handshake-signature algorithms, so exactly one provider is in play.
fn build_mtls_server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<rustls::ServerConfig, TlsConfigError> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .ok_or(TlsConfigError::NoCryptoProvider)?
        .clone();

    let cert_chain = load_cert_chain(cert_pem)?;
    let key = load_private_key(key_pem)?;

    let verifier = Arc::new(SelfSignedOptionalClientCertVerifier {
        provider: provider.clone(),
    });

    // `builder_with_provider(..).with_safe_default_protocol_versions()` pins the SAME provider the
    // verifier uses. `.with_client_cert_verifier` installs the optional cert request; `.with_single_cert`
    // supplies our leaf cert + key. Any mismatch/malformed input surfaces as a clear Malformed error.
    rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsConfigError::Malformed {
            source: io::Error::other(e),
        })?
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, key)
        .map_err(|e| TlsConfigError::Malformed {
            source: io::Error::other(e),
        })
}

/// Parse a PEM certificate chain (leaf first) into DER. A PEM with no CERTIFICATE block is a Malformed
/// boot error (an empty chain would fail deep inside rustls with an opaque message).
///
/// `pem_slice_iter` yields ONLY `CERTIFICATE` sections, in file order, skipping any other section
/// kind — notably a `PRIVATE KEY` bundled into the same file, which operators commonly do. That is
/// the same filtering the retired `rustls_pemfile::certs` did, because that function was itself a
/// wrapper over this parser.
fn load_cert_chain(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, TlsConfigError> {
    let certs = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsConfigError::Malformed {
            source: io::Error::other(e),
        })?;
    if certs.is_empty() {
        return Err(TlsConfigError::Malformed {
            source: io::Error::other("TLS certificate PEM contains no CERTIFICATE block"),
        });
    }
    Ok(certs)
}

/// Parse the FIRST PEM private key (PKCS#8 / PKCS#1 / SEC1) into DER. Absent ⇒ a Malformed boot error.
///
/// `PrivateKeyDer::from_pem_slice` reports "no key here" as [`PemError::NoItemsFound`], where the
/// retired `rustls_pemfile::private_key` reported it as `Ok(None)`; that one variant is mapped back
/// to the same operator-facing message so the boot diagnostic is unchanged by the migration.
fn load_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, TlsConfigError> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|e| TlsConfigError::Malformed {
        source: match e {
            PemError::NoItemsFound => {
                io::Error::other("TLS private-key PEM contains no PRIVATE KEY block")
            }
            other => io::Error::other(other),
        },
    })
}

/// The RFC 8705 §2.2 **self-signed-flavour** optional client-certificate verifier.
///
/// Two deliberate, load-bearing choices:
/// - **Optional** ([`offer_client_auth`](Self) = true, [`client_auth_mandatory`](Self) = false): the
///   handshake REQUESTS a client cert but a client presenting NONE is still accepted — so plain-DPoP
///   clients (the vast majority) are completely unaffected when the mTLS flag is on. A cert-bound token
///   presented on a connection with no client cert is rejected LATER, at the auth layer (fail-closed),
///   not by refusing the handshake.
/// - **No chain validation** ([`verify_client_cert`](Self) accepts any well-formed cert): per RFC 8705
///   §2.2, trust is NOT the certificate chain — it is the `cnf.x5t#S256` thumbprint match, enforced
///   downstream in [`crate::pop`]. Accepting any presented cert here is therefore correct AND minimises
///   surface (no CA/PKI lifecycle). Crucially, key **possession is STILL proven**: the TLS handshake's
///   `CertificateVerify` signature is verified against the presented cert's OWN public key by
///   [`verify_tls12_signature`](Self)/[`verify_tls13_signature`](Self) below (delegated to the crypto
///   provider's real verifiers) — only chain/PKI validation is skipped, never the possession proof.
#[derive(Debug)]
struct SelfSignedOptionalClientCertVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::server::danger::ClientCertVerifier for SelfSignedOptionalClientCertVerifier {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        // No CA roots hinted — self-signed flavour trusts the thumbprint, not a CA.
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        // Accept any well-formed presented certificate: trust is the downstream `cnf.x5t#S256`
        // thumbprint match (RFC 8705 §2.2), NOT the chain. Possession is proven by the signature
        // checks below, which rustls always runs when a cert is presented.
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        // Real possession proof: verify the CertificateVerify signature against the presented cert's
        // own public key using the crypto provider's algorithms (NOT a blanket accept).
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }

    fn offer_client_auth(&self) -> bool {
        true // REQUEST a client cert (so a cert-bound client can present one)…
    }

    fn client_auth_mandatory(&self) -> bool {
        false // …but do NOT require it — plain-DPoP clients presenting none are still admitted.
    }
}

/// Finalize the transport tuning on a built [`RustlsConfig`]: re-assert the advertised ALPN protocols
/// ([`ALPN_PROTOCOLS`]) AND install the session-resumption cache (`cache_size`). Both build paths in
/// [`build_rustls_config_with_session_cache_size`] (`from_pem` default and the mTLS builder) converge
/// here, so the tuning is applied UNIFORMLY regardless of which path produced the config.
///
/// `RustlsConfig` wraps an `ArcSwap<ServerConfig>`; the inner `ServerConfig` is immutable behind the
/// `Arc`, so we clone it ONCE, set both fields, and swap the new config back in via
/// `reload_from_config` (the same swap path axum-server itself uses for cert reload). At boot there
/// are no in-flight handshakes, so the swap is contention-free.
///
/// **This knob governs TLS 1.3 resumption too** (not just TLS 1.2 session IDs). With the DEFAULT
/// (`tuning.stateless_tickets == false`) we keep rustls's default `ticketer` (`NeverProducesTickets`),
/// so rustls performs *stateful* TLS 1.3 resumption backed by `session_storage`: on issue it stores the
/// session and hands the client a random 32-byte id as the ticket (`server/tls13.rs`: `let stateless =
/// ticketer.enabled(); … else { session_storage.put(id, plain) }` — and if `put` returns `false` it logs
/// "resumption not available; not issuing ticket"), and on resume it looks the id up via
/// `session_storage.take(ticket)`. Therefore, IN THE DEFAULT MODE, `cache_size == 0` (a
/// [`rustls::server::NoServerSessionStorage`] whose `put` returns `false`) genuinely DISABLES TLS 1.3
/// resumption — no ticket is even issued — and a bounded cache bounds the number of resumable TLS 1.3
/// sessions (proven by the ignored `tls_session_cache_size_*` test, which asserts the resumed handshakes
/// it counts are TLS 1.3).
///
/// **The stateless-ticket half (`tuning.stateless_tickets == true`, opt-in — P1.3's deferred half, now
/// landed).** Installing the aws-lc-rs RFC 5077 [`rustls::crypto::aws_lc_rs::Ticketer`] flips TLS 1.3
/// resumption to the STATELESS path: `ticketer.enabled()` becomes `true`, so rustls issues an ENCRYPTED
/// ticket (AES-256-CBC + HMAC-SHA256, per-process keys rotated ~6h) and resumes from it WITHOUT
/// `session_storage`. Consequently, when tickets are on, TLS 1.3 resumption no longer depends on the
/// cache — `cache_size == 0` still disables the *stateful* (TLS 1.2 / session-id) path but ticket-based
/// TLS 1.3 resumption remains available. The install is FAIL-SAFE ([`make_ticketer`]): an RNG error logs
/// and leaves the stateful cache in place, never failing the boot.
///
/// **0-RTT stays OFF (security invariant) — even with a ticketer.** [`max_early_data_size`](rustls::ServerConfig)
/// is FORCED to `0` here on every path, regardless of the ticketer. A ticketer makes 0-RTT *possible* only
/// if `max_early_data_size > 0`; keeping it `0` means installing the ticketer never opens the 0-RTT
/// replay window. 0-RTT early data is replayable by design, which is incoherent under this server's
/// anti-replay DPoP `jti` model (`research/lws-design-records.md` §7). The `debug_assert` pins that
/// the value was already `0` so a future rustls default change surfaces in tests.
fn apply_transport_tuning(config: &RustlsConfig, tuning: TransportTuning) {
    let mut server_config = (*config.get_inner()).clone();
    server_config.alpn_protocols = ALPN_PROTOCOLS.iter().map(|p| p.to_vec()).collect();
    server_config.session_storage = make_session_storage(tuning.session_cache_size);
    // Opt-in STATELESS resumption tickets (P1.3 ticketer half). Only overwrites the default
    // `NeverProducesTickets` when the flag is on AND the ticketer built; a build failure keeps the
    // stateful cache (fail-safe). This is orthogonal to 0-RTT, which stays off below.
    if let Some(ticketer) = make_ticketer(tuning.stateless_tickets) {
        server_config.ticketer = ticketer;
    }
    // HARD-ENFORCE 0-RTT OFF, in release too — do not merely assert. It is already `0` by construction
    // (the rustls builder default, preserved by `.clone()`), but setting it explicitly makes the
    // anti-replay invariant hold even if a future rustls default or a `from_config` cert path arrived
    // with a nonzero value; the `debug_assert` then just documents that this line changed nothing today.
    let prior_early_data = server_config.max_early_data_size;
    server_config.max_early_data_size = 0;
    debug_assert_eq!(
        prior_early_data, 0,
        "0-RTT early data must stay disabled (anti-replay invariant) — a nonzero default appeared"
    );
    config.reload_from_config(std::sync::Arc::new(server_config));
}

/// Build the rustls server session store for a given resumption-cache `size`:
/// - `size == 0` ⇒ [`rustls::server::NoServerSessionStorage`] — resumption disabled (every handshake
///   is full; `can_cache()` is `false`, so the server never even issues resumption tickets);
/// - `size > 0` ⇒ a [`rustls::server::ServerSessionMemoryCache`] bounded to ≈`size` sessions (oldest
///   evicted on overflow).
///
/// Factored out (a) so both the config-build path and the always-run unit tests exercise the SAME
/// selection logic, and (b) to keep the `if`/`else` arms coercing to one `dyn` trait object cleanly.
fn make_session_storage(size: usize) -> Arc<dyn rustls::server::StoresServerSessions> {
    if size == 0 {
        Arc::new(rustls::server::NoServerSessionStorage {})
    } else {
        rustls::server::ServerSessionMemoryCache::new(size)
    }
}

/// Build the STATELESS TLS resumption ticketer when [`TransportTuning::stateless_tickets`] is on.
///
/// Returns the aws-lc-rs RFC 5077 [`rustls::crypto::aws_lc_rs::Ticketer`] — "Recommended Ticket
/// Construction": AES-256-CBC encryption + HMAC-SHA256 authentication, RANDOM per-process keys ROTATED
/// every ~6h (≈12h effective ticket life; forward secrecy is provided by key erasure, NOT by a lifetime
/// baked into the ticket). Installing it makes TLS 1.3 resumption stateless (an encrypted ticket, no
/// server-side per-session memory) — the deferred half of beyond-50k P1.3.
///
/// FAIL-SAFE: `Ticketer::new()` can only fail on an RNG error; on failure we LOG (matching `main`'s
/// startup diagnostics) and return `None`, leaving the caller's stateful session cache in place. A
/// resumption-mechanism error is a PERFORMANCE fallback (stateful resumption / full handshakes), NEVER a
/// correctness or security change, so it must never take the server down. Returns `None` when the flag is
/// off (the default), so the caller keeps rustls's `NeverProducesTickets` default.
fn make_ticketer(enabled: bool) -> Option<Arc<dyn rustls::server::ProducesTickets>> {
    if !enabled {
        return None;
    }
    match rustls::crypto::aws_lc_rs::Ticketer::new() {
        Ok(ticketer) => Some(ticketer),
        Err(err) => {
            eprintln!(
                "  WARNING: failed to build the TLS stateless-ticket generator ({err}); \
                 falling back to the stateful session cache"
            );
            None
        }
    }
}

/// Read a PEM file, mapping a missing/unreadable file and an empty file to clear errors.
async fn read_pem(which: &'static str, path: &Path) -> Result<Vec<u8>, TlsConfigError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|source| TlsConfigError::Unreadable {
            which,
            path: path.to_path_buf(),
            source,
        })?;
    // An empty (or whitespace-only) file would otherwise fail deep inside rustls with an opaque
    // "no keys/certs found" — catch it here with the offending path.
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(TlsConfigError::Empty {
            which,
            path: path.to_path_buf(),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `Option<&OsStr>` from a `&str` for the mode-resolution tests.
    fn os(s: &str) -> Option<&OsStr> {
        Some(OsStr::new(s))
    }

    #[test]
    fn neither_set_is_plain() {
        assert_eq!(mode_from_values(None, None).unwrap(), TlsMode::Plain);
    }

    #[test]
    fn both_set_is_tls() {
        let mode = mode_from_values(os("/etc/tls/cert.pem"), os("/etc/tls/key.pem")).unwrap();
        assert_eq!(
            mode,
            TlsMode::Tls {
                cert_path: PathBuf::from("/etc/tls/cert.pem"),
                key_path: PathBuf::from("/etc/tls/key.pem"),
            }
        );
    }

    #[test]
    fn both_set_trims_whitespace() {
        let mode = mode_from_values(os("  /c.pem  "), os("\t/k.pem\n")).unwrap();
        assert_eq!(
            mode,
            TlsMode::Tls {
                cert_path: PathBuf::from("/c.pem"),
                key_path: PathBuf::from("/k.pem"),
            }
        );
    }

    #[test]
    fn cert_only_is_incomplete() {
        let err = mode_from_values(os("/c.pem"), None).unwrap_err();
        match err {
            TlsConfigError::Incomplete { present, missing } => {
                assert_eq!(present, ENV_TLS_CERT);
                assert_eq!(missing, ENV_TLS_KEY);
            }
            other => panic!("expected Incomplete, got {other:?}"),
        }
        // The message names both vars so the operator knows exactly what to fix.
        let msg = err.to_string();
        assert!(msg.contains(ENV_TLS_CERT), "msg: {msg}");
        assert!(msg.contains(ENV_TLS_KEY), "msg: {msg}");
    }

    #[test]
    fn key_only_is_incomplete() {
        let err = mode_from_values(None, os("/k.pem")).unwrap_err();
        match err {
            TlsConfigError::Incomplete { present, missing } => {
                assert_eq!(present, ENV_TLS_KEY);
                assert_eq!(missing, ENV_TLS_CERT);
            }
            other => panic!("expected Incomplete, got {other:?}"),
        }
    }

    #[test]
    fn blank_value_counts_as_set_so_one_blank_is_incomplete() {
        // An exported-but-empty var means "I intended TLS" — must NOT silently downgrade to plain.
        let err = mode_from_values(os(""), None).unwrap_err();
        assert!(matches!(err, TlsConfigError::Incomplete { .. }));
    }

    #[test]
    fn non_unicode_paths_do_not_downgrade_to_plaintext() {
        // FAIL-CLOSED: a present-but-non-Unicode pair must resolve to TLS (the path is HONOURED),
        // never be mistaken for "absent" and silently downgraded to plaintext. This is the regression
        // guard for the `var`/`.ok()` bug: `var` would have returned Err for a non-Unicode value,
        // `.ok()` would have flattened it to None ("absent"), and two such values would have produced
        // `TlsMode::Plain` — a silent plaintext downgrade. `var_os` + `OsStr` carry the bytes through.
        let (cert, key) = non_unicode_pair();
        let mode = mode_from_values(Some(&cert), Some(&key)).unwrap();
        match mode {
            TlsMode::Tls {
                cert_path,
                key_path,
            } => {
                // The exact non-Unicode bytes survived into the PathBuf (not dropped/lossily mangled).
                assert_eq!(cert_path.as_os_str(), cert.as_os_str());
                assert_eq!(key_path.as_os_str(), key.as_os_str());
            }
            TlsMode::Plain => panic!("non-Unicode TLS paths silently downgraded to plaintext"),
        }
    }

    #[test]
    fn one_non_unicode_path_is_incomplete_not_plain() {
        // Exactly one non-Unicode path set is still the both-or-neither error, NOT a plaintext
        // downgrade — the present (non-Unicode) value must be SEEN as present.
        let (cert, _key) = non_unicode_pair();
        let err = mode_from_values(Some(&cert), None).unwrap_err();
        assert!(
            matches!(err, TlsConfigError::Incomplete { .. }),
            "one non-Unicode path should be Incomplete, got {err:?}"
        );
    }

    /// A cert/key `OsString` pair containing bytes that are NOT valid Unicode, on platforms where
    /// `OsString` is byte-based (Unix) — the exact case `std::env::var` rejects. On other platforms
    /// fall back to a valid-Unicode pair (still exercising the `OsStr` path; the non-Unicode-specific
    /// downgrade bug is Unix-shaped where env values are arbitrary bytes).
    fn non_unicode_pair() -> (OsString, OsString) {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            // 0x80/0xFF are invalid as standalone UTF-8 — `String::from_utf8`/`std::env::var` reject.
            (
                OsString::from_vec(vec![b'/', 0x80, b'c', b'.', b'p', b'e', b'm']),
                OsString::from_vec(vec![b'/', 0xFF, b'k', b'.', b'p', b'e', b'm']),
            )
        }
        #[cfg(not(unix))]
        {
            (OsString::from("/c.pem"), OsString::from("/k.pem"))
        }
    }

    #[tokio::test]
    async fn plain_mode_builds_no_config() {
        assert!(build_rustls_config(&TlsMode::Plain, false)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn missing_cert_file_is_unreadable() {
        let mode = TlsMode::Tls {
            cert_path: PathBuf::from("/nonexistent/does-not-exist-cert.pem"),
            key_path: PathBuf::from("/nonexistent/does-not-exist-key.pem"),
        };
        let err = build_rustls_config(&mode, false).await.unwrap_err();
        match err {
            TlsConfigError::Unreadable { which, path, .. } => {
                assert_eq!(which, "certificate");
                assert!(path.to_string_lossy().contains("does-not-exist-cert"));
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_cert_file_is_empty_error() {
        let dir = std::env::temp_dir().join(format!("ssrs-tls-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert = dir.join("empty-cert.pem");
        let key = dir.join("empty-key.pem");
        tokio::fs::write(&cert, b"   \n\t  ").await.unwrap();
        tokio::fs::write(&key, b"   ").await.unwrap();
        let mode = TlsMode::Tls {
            cert_path: cert.clone(),
            key_path: key.clone(),
        };
        let err = build_rustls_config(&mode, false).await.unwrap_err();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        match err {
            TlsConfigError::Empty { which, .. } => assert_eq!(which, "certificate"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn built_config_advertises_h2_then_http11_alpn() {
        // The built TLS config must advertise ALPN = [h2, http/1.1], in that preference order, so an
        // h2-capable client negotiates HTTP/2 and an h1-only client negotiates down. This is the
        // owned transport contract (set_alpn_protocols) — a regression here would silently drop h2.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (cert, key) = self_signed_localhost_pem();
        let dir = std::env::temp_dir().join(format!("ssrs-tls-alpn-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        tokio::fs::write(&cert_path, &cert).await.unwrap();
        tokio::fs::write(&key_path, &key).await.unwrap();
        let mode = TlsMode::Tls {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
        };
        let config = build_rustls_config(&mode, false)
            .await
            .expect("build config")
            .expect("tls mode yields a config");
        let inner = config.get_inner();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        assert_eq!(
            inner.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            "ALPN must be [h2, http/1.1] in preference order (h2 first, h1 fallback)"
        );
        // And the public constant matches what is advertised (so callers/tests can rely on it).
        let from_const: Vec<Vec<u8>> = ALPN_PROTOCOLS.iter().map(|p| p.to_vec()).collect();
        assert_eq!(inner.alpn_protocols, from_const);
    }

    /// Mint a throwaway self-signed P-256 cert+key for `localhost`/`127.0.0.1` IN-MEMORY via
    /// `aws-lc-rs`, returning `(cert_pem, key_pem)`. Used only by the ALPN unit test; never a real
    /// credential (generated fresh per run, discarded immediately). Uses the same crypto backend the
    /// server uses, so it needs no external `openssl`/`rcgen` dependency.
    fn self_signed_localhost_pem() -> (Vec<u8>, Vec<u8>) {
        // The `aws-lc-rs` provider is already a (test-)dependency via rustls; generate a minimal
        // self-signed cert with the system `openssl` if available, else fall back to the checked-in
        // fixture cert. To keep this dependency-free and deterministic we shell out to openssl, which
        // is present on the dev/CI boxes (the bench/conformance cert scripts already require it).
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};
        // UNIQUE per call (not just per pid): several tests mint a cert CONCURRENTLY, so a pid-only dir
        // races (one caller's `remove_dir_all` deletes another's cert mid-read). A monotonic counter
        // gives each call its own dir.
        static MINT_SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ssrs-tls-mint-{}-{}",
            std::process::id(),
            MINT_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("c.pem");
        let key = dir.join("k.pem");
        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-nodes",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args(["-days", "1", "-subj", "/CN=localhost"])
            .args(["-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"])
            .output();
        let (cert_bytes, key_bytes) = match status {
            Ok(out) if out.status.success() => {
                (std::fs::read(&cert).unwrap(), std::fs::read(&key).unwrap())
            }
            _ => {
                // openssl unavailable — fall back to the checked-in throwaway test fixture so the test
                // still exercises the ALPN-set path without a hard openssl requirement.
                let fcert = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cert.pem");
                let fkey = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key.pem");
                (std::fs::read(fcert).unwrap(), std::fs::read(fkey).unwrap())
            }
        };
        let _ = std::fs::remove_dir_all(&dir);
        (cert_bytes, key_bytes)
    }

    #[tokio::test]
    async fn malformed_pem_is_malformed_error() {
        // Install the provider (idempotent) so we reach the malformed-parse path, not NoCryptoProvider.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("ssrs-tls-bad-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert = dir.join("bad-cert.pem");
        let key = dir.join("bad-key.pem");
        // Non-empty, but not valid PEM cert/key material.
        tokio::fs::write(
            &cert,
            b"-----BEGIN CERTIFICATE-----\nnot base64!!!\n-----END CERTIFICATE-----\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            &key,
            b"-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----\n",
        )
        .await
        .unwrap();
        let mode = TlsMode::Tls {
            cert_path: cert.clone(),
            key_path: key.clone(),
        };
        let err = build_rustls_config(&mode, false).await.unwrap_err();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        assert!(
            matches!(err, TlsConfigError::Malformed { .. }),
            "expected Malformed, got {err:?}"
        );
    }

    #[test]
    fn mtls_flag_parses_affirmative_opt_in_only() {
        // Truthy tokens (case-insensitive, trimmed) enable; everything else stays OFF (fail-safe).
        for on in ["1", "true", "TRUE", "Yes", " on ", "On"] {
            std::env::set_var(ENV_MTLS_BOUND_TOKENS, on);
            assert!(mtls_bound_tokens_from_env(), "{on:?} should enable mTLS");
        }
        for off in [
            "", " ", "0", "false", "no", "off", "enabled", "2", "garbage",
        ] {
            std::env::set_var(ENV_MTLS_BOUND_TOKENS, off);
            assert!(
                !mtls_bound_tokens_from_env(),
                "{off:?} must NOT enable mTLS"
            );
        }
        std::env::remove_var(ENV_MTLS_BOUND_TOKENS);
        assert!(
            !mtls_bound_tokens_from_env(),
            "absent ⇒ OFF (the default posture)"
        );
    }

    /// [OPUS-5] sq-5ah3p: the behavioural contract of the two PEM loaders, pinned independently of
    /// which crate implements the decode. These four properties are exactly what the retired
    /// `rustls-pemfile` calls provided, so they are what the `PemObject` migration must preserve:
    /// (1) EVERY `CERTIFICATE` section is returned, in file order, leaf first; (2) a non-certificate
    /// section sharing the file (the common "cert and key concatenated" layout) is SKIPPED rather
    /// than erroring or being mistaken for a cert; (3) a PEM carrying no certificate is a `Malformed`
    /// boot error with the operator-facing message, not an empty chain handed to rustls; (4) a key
    /// file with no key section is likewise `Malformed` with its own message — the one case where
    /// `PemObject` reports `NoItemsFound` and the old API reported `Ok(None)`.
    #[test]
    fn pem_loaders_split_chain_from_key_and_reject_empty_input() {
        let (cert_pem, key_pem) = self_signed_localhost_pem();

        // (1) A two-element chain round-trips in order. The second element is the checked-in fixture
        // CA — a certificate that is definitely NOT the minted leaf — so a reversed or deduplicated
        // iterator cannot satisfy the index assertions below.
        let ca_pem = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/test-ca.pem"
        ))
        .expect("read fixture CA");
        let mut chain_pem = cert_pem.clone();
        chain_pem.extend_from_slice(&ca_pem);
        let chain = load_cert_chain(&chain_pem).expect("a two-certificate chain parses");
        assert_eq!(chain.len(), 2, "both CERTIFICATE sections must be returned");
        let leaf_only = load_cert_chain(&cert_pem).expect("single-cert PEM parses");
        let ca_only = load_cert_chain(&ca_pem).expect("fixture CA parses");
        assert_ne!(leaf_only[0], ca_only[0], "the two fixtures must differ");
        assert_eq!(
            chain[0], leaf_only[0],
            "the FIRST section of the file must be the FIRST (leaf) entry of the chain"
        );
        assert_eq!(chain[1], ca_only[0], "the issuer must follow the leaf");

        // (2) A cert file that also carries the key yields the certs only — and the key loader
        // reads that same bundle happily, skipping the certificate section ahead of the key.
        let mut bundle = cert_pem.clone();
        bundle.extend_from_slice(&key_pem);
        assert_eq!(
            load_cert_chain(&bundle).expect("bundle parses").len(),
            1,
            "the PRIVATE KEY section must be skipped, not counted as a certificate"
        );
        load_private_key(&bundle).expect("the key is found past the leading certificate");

        // (3) No CERTIFICATE section ⇒ Malformed, with the operator-facing message.
        let err = load_cert_chain(&key_pem).expect_err("a key-only PEM has no certificate");
        let TlsConfigError::Malformed { source } = err else {
            panic!("a certificate-less PEM must be Malformed");
        };
        assert!(
            source.to_string().contains("no CERTIFICATE block"),
            "unexpected diagnostic: {source}"
        );

        // (4) No key section ⇒ Malformed. This is the `NoItemsFound` remap: without it the operator
        // would see `PemObject`'s bare "no items found" instead of the actionable message.
        let err = load_private_key(&cert_pem).expect_err("a cert-only PEM has no private key");
        let TlsConfigError::Malformed { source } = err else {
            panic!("a key-less PEM must be Malformed");
        };
        assert!(
            source.to_string().contains("no PRIVATE KEY block"),
            "unexpected diagnostic: {source}"
        );
    }

    #[tokio::test]
    async fn mtls_config_builds_optional_client_auth_and_keeps_alpn() {
        // With the mTLS flag ON, the built ServerConfig must (a) still advertise ALPN [h2, http/1.1]
        // exactly (transport contract preserved on the new build path), and (b) request client auth
        // OPTIONALLY — `client_auth_mandatory() == false` so a plain-DPoP client presenting NO cert is
        // still admitted (the fail-closed reject for a cert-bound-token-without-cert happens at the
        // AUTH layer, never by refusing the handshake). We assert the verifier's optional posture
        // directly (a full handshake is covered by the ignored live TLS integration test).
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (cert, key) = self_signed_localhost_pem();
        // The raw builder succeeds (valid leaf cert/key + the optional self-signed client verifier).
        build_mtls_server_config(&cert, &key).expect("mTLS config builds");

        // The verifier posture is optional (not mandatory) — a plain client presenting no cert is
        // admitted; assert directly on the verifier type.
        let provider = rustls::crypto::CryptoProvider::get_default()
            .expect("provider installed")
            .clone();
        let verifier = SelfSignedOptionalClientCertVerifier { provider };
        use rustls::server::danger::ClientCertVerifier as _;
        assert!(
            verifier.offer_client_auth(),
            "must REQUEST a client cert so a cert-bound client can present one"
        );
        assert!(
            !verifier.client_auth_mandatory(),
            "must NOT require a client cert — plain-DPoP clients are unaffected (fail-closed happens at auth)"
        );

        // The full path builds a config and advertises the owned ALPN set.
        let dir = std::env::temp_dir().join(format!("ssrs-mtls-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert_path = dir.join("c.pem");
        let key_path = dir.join("k.pem");
        tokio::fs::write(&cert_path, &cert).await.unwrap();
        tokio::fs::write(&key_path, &key).await.unwrap();
        let mode = TlsMode::Tls {
            cert_path,
            key_path,
        };
        // mtls = true path: builds a config and advertises the owned ALPN set.
        let config = build_rustls_config(&mode, true)
            .await
            .expect("build mTLS config")
            .expect("tls mode yields a config");
        let inner = config.get_inner();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        assert_eq!(
            inner.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            "mTLS build path must still advertise ALPN [h2, http/1.1]"
        );
    }

    #[tokio::test]
    async fn mtls_off_and_on_both_build_a_usable_config() {
        // The default (flag-off) path and the flag-on path both yield a Some(config) for a valid
        // cert/key — the flag never breaks the ability to serve TLS, it only changes the client-auth
        // posture. (Byte-identical-when-off for the DPoP path is asserted at the auth layer; here we
        // assert the TLS build succeeds either way.)
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (cert, key) = self_signed_localhost_pem();
        let dir = std::env::temp_dir().join(format!("ssrs-mtls2-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert_path = dir.join("c.pem");
        let key_path = dir.join("k.pem");
        tokio::fs::write(&cert_path, &cert).await.unwrap();
        tokio::fs::write(&key_path, &key).await.unwrap();
        let mode = TlsMode::Tls {
            cert_path,
            key_path,
        };
        assert!(build_rustls_config(&mode, false).await.unwrap().is_some());
        assert!(build_rustls_config(&mode, true).await.unwrap().is_some());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // --- TLS session-resumption cache size (beyond-50k P1.3) -----------------------------------------

    #[test]
    fn parse_session_cache_size_table() {
        // Unset / empty / whitespace ⇒ the tuned default.
        assert_eq!(
            parse_session_cache_size(None),
            DEFAULT_TLS_SESSION_CACHE_SIZE
        );
        assert_eq!(
            parse_session_cache_size(Some("")),
            DEFAULT_TLS_SESSION_CACHE_SIZE
        );
        assert_eq!(
            parse_session_cache_size(Some("   ")),
            DEFAULT_TLS_SESSION_CACHE_SIZE
        );
        // A valid non-negative integer is honoured (with surrounding whitespace trimmed).
        assert_eq!(parse_session_cache_size(Some("256")), 256);
        assert_eq!(parse_session_cache_size(Some("  512 ")), 512);
        assert_eq!(parse_session_cache_size(Some("1")), 1);
        // `0` is honoured VERBATIM as "disable resumption" (mapped to NoServerSessionStorage).
        assert_eq!(parse_session_cache_size(Some("0")), 0);
        // Over-large values are CLAMPED so boot never tries to pre-allocate an unbounded map.
        assert_eq!(
            parse_session_cache_size(Some("1000000000")),
            MAX_TLS_SESSION_CACHE_SIZE
        );
        // Garbage / negative / non-integer ⇒ FAIL-SAFE to the default (never a boot break — perf knob).
        for bad in ["garbage", "-1", "12.5", "1e6", "0x10", "  ", "abc123"] {
            assert_eq!(
                parse_session_cache_size(Some(bad)),
                DEFAULT_TLS_SESSION_CACHE_SIZE,
                "{bad:?} must fall back to the default cache size"
            );
        }
    }

    #[test]
    fn make_session_storage_disables_at_zero_and_bounds_above() {
        // (`StoresServerSessions`'s methods are callable directly on the `dyn` trait object.)
        // size 0 ⇒ resumption DISABLED: the store advertises it cannot cache, and a put is dropped.
        let disabled = make_session_storage(0);
        assert!(
            !disabled.can_cache(),
            "size 0 must install a store that cannot cache (resumption disabled)"
        );
        assert!(
            !disabled.put(b"k".to_vec(), b"v".to_vec()),
            "a disabled store must not accept a session"
        );
        assert_eq!(disabled.get(b"k"), None);

        // A bounded store caches, round-trips, AND evicts the oldest past its capacity. `new(4)` keeps
        // ~3 sessions (rustls evicts the oldest on reaching capacity), so after 10 distinct inserts the
        // earliest is gone and the most-recent survives — the exact bound this lever raises from 256.
        let bounded = make_session_storage(4);
        assert!(bounded.can_cache(), "a positive size must cache");
        for i in 0..10u8 {
            assert!(bounded.put(vec![i], vec![i]));
        }
        assert_eq!(
            bounded.get(&[0]),
            None,
            "the oldest session must be evicted past the bound"
        );
        assert_eq!(
            bounded.get(&[9]),
            Some(vec![9]),
            "the most-recent session must survive"
        );
    }

    #[tokio::test]
    async fn built_config_installs_session_cache_and_keeps_0rtt_off() {
        // End-to-end through the real build path: a config built with an explicit cache size must carry
        // a matching session store AND keep 0-RTT early data OFF (the anti-replay invariant). This runs
        // in the standard gate (no socket I/O) and is the deterministic proof that the lever reaches the
        // live ServerConfig, complementing the ignored resumed-vs-full handshake-count integration test.
        // (`StoresServerSessions::can_cache` is callable directly on the `dyn` trait object.)
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (cert, key) = self_signed_localhost_pem();
        let dir = std::env::temp_dir().join(format!("ssrs-tls-scache-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert_path = dir.join("c.pem");
        let key_path = dir.join("k.pem");
        tokio::fs::write(&cert_path, &cert).await.unwrap();
        tokio::fs::write(&key_path, &key).await.unwrap();
        let mode = TlsMode::Tls {
            cert_path,
            key_path,
        };

        // size 0 ⇒ the built config's store cannot cache (resumption disabled).
        let disabled = build_rustls_config_with_session_cache_size(&mode, false, 0)
            .await
            .expect("build config")
            .expect("tls mode yields a config");
        {
            let inner = disabled.get_inner();
            assert!(
                !inner.session_storage.can_cache(),
                "cache size 0 must disable resumption on the built config"
            );
            assert_eq!(
                inner.max_early_data_size, 0,
                "0-RTT early data must be OFF (anti-replay invariant)"
            );
        }

        // Positive sizes install a caching store; 0-RTT stays off. Use the EXPLICIT-size builder (not
        // the env-reading `build_rustls_config`) so this assertion is independent of any ambient
        // `SOLID_SERVER_TLS_SESSION_CACHE_SIZE` (e.g. an operator/CI env setting it to `0` would
        // otherwise disable resumption and fail the `can_cache()` assertion).
        for cfg in [
            build_rustls_config_with_session_cache_size(&mode, false, 4096)
                .await
                .unwrap()
                .unwrap(),
            build_rustls_config_with_session_cache_size(
                &mode,
                false,
                DEFAULT_TLS_SESSION_CACHE_SIZE,
            )
            .await
            .unwrap()
            .unwrap(),
        ] {
            let inner = cfg.get_inner();
            assert!(
                inner.session_storage.can_cache(),
                "a positive cache size must enable resumption"
            );
            assert_eq!(
                inner.max_early_data_size, 0,
                "0-RTT early data must stay OFF on every build path"
            );
            // ALPN is still owned + advertised (tuning did not clobber the transport contract).
            assert_eq!(
                inner.alpn_protocols,
                vec![b"h2".to_vec(), b"http/1.1".to_vec()]
            );
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // --- TLS stateless resumption tickets (beyond-50k P1.3 — the ticketer half) ---------------------

    #[test]
    fn is_affirmative_grammar_matches_opt_in_only() {
        // The shared opt-in grammar behind BOTH the mTLS flag and the stateless-tickets flag: only the
        // truthy tokens enable; everything else (incl. absent) is OFF. A typo never silently opts in.
        for on in ["1", "true", "TRUE", "Yes", " on ", "On", "\tyes\n"] {
            assert!(is_affirmative(Some(on)), "{on:?} must be affirmative");
        }
        for off in [
            "", " ", "0", "false", "no", "off", "enabled", "2", "garbage", "onn", "yeah",
        ] {
            assert!(
                !is_affirmative(Some(off)),
                "{off:?} must NOT be affirmative"
            );
        }
        assert!(!is_affirmative(None), "absent ⇒ OFF (the default)");
    }

    #[test]
    fn make_ticketer_is_none_when_off_and_enabled_when_on() {
        // OFF ⇒ no ticketer (caller keeps rustls's NeverProducesTickets default).
        assert!(
            make_ticketer(false).is_none(),
            "flag off must not build a ticketer"
        );
        // ON ⇒ a real, enabled ticketer (the RFC 5077 aws-lc-rs one). `enabled()` distinguishes it from
        // the default NeverProducesTickets (which reports false). `Ticketer::new()` uses SystemRandom,
        // not the installed provider, but install it idempotently for parity with the other tests.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let ticketer = make_ticketer(true).expect("aws-lc-rs ticketer builds");
        assert!(
            ticketer.enabled(),
            "an installed ticketer must report enabled() == true (stateless resumption active)"
        );
    }

    #[tokio::test]
    async fn built_config_installs_ticketer_when_enabled_and_keeps_0rtt_off() {
        // End-to-end through the real build path: with `stateless_tickets: true` the built ServerConfig
        // must carry an ENABLED ticketer (stateless TLS 1.3 resumption) AND still keep 0-RTT early data
        // OFF (the anti-replay invariant holds even with a ticketer) AND still own the ALPN set. Crucially
        // this uses the EXPLICIT tuning builder (not the env-reading entry) so it is independent of any
        // ambient SOLID_SERVER_TLS_STATELESS_TICKETS. (`ProducesTickets::enabled` is callable directly on
        // the `dyn` trait object, like `StoresServerSessions::can_cache` in the cache test above.)
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (cert, key) = self_signed_localhost_pem();
        let dir = std::env::temp_dir().join(format!("ssrs-tls-ticket-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cert_path = dir.join("c.pem");
        let key_path = dir.join("k.pem");
        tokio::fs::write(&cert_path, &cert).await.unwrap();
        tokio::fs::write(&key_path, &key).await.unwrap();
        let mode = TlsMode::Tls {
            cert_path,
            key_path,
        };

        // Tickets ON (with resumption cache also 0, to prove the ticketer is independent of the cache):
        // the built config's ticketer is enabled and 0-RTT stays off.
        let with_tickets = build_rustls_config_with_tuning(
            &mode,
            false,
            TransportTuning {
                session_cache_size: 0,
                stateless_tickets: true,
            },
        )
        .await
        .expect("build config")
        .expect("tls mode yields a config");
        {
            let inner = with_tickets.get_inner();
            assert!(
                inner.ticketer.enabled(),
                "stateless_tickets: true must install an ENABLED ticketer"
            );
            assert_eq!(
                inner.max_early_data_size, 0,
                "0-RTT early data must stay OFF even with a ticketer (anti-replay invariant)"
            );
            assert_eq!(
                inner.alpn_protocols,
                vec![b"h2".to_vec(), b"http/1.1".to_vec()],
                "ALPN transport contract preserved on the ticketer path"
            );
        }

        // Tickets OFF (the default): rustls's NeverProducesTickets stays, so `enabled()` is false — the
        // byte-identical pre-ticketer posture. Both the explicit tuning path and the cache-size wrapper
        // must leave the ticketer disabled.
        let default_tuning = build_rustls_config_with_tuning(
            &mode,
            false,
            TransportTuning {
                session_cache_size: DEFAULT_TLS_SESSION_CACHE_SIZE,
                stateless_tickets: false,
            },
        )
        .await
        .expect("build config")
        .expect("config");
        let cache_only = build_rustls_config_with_session_cache_size(&mode, false, 4096)
            .await
            .expect("build config")
            .expect("config");
        for cfg in [&default_tuning, &cache_only] {
            assert!(
                !cfg.get_inner().ticketer.enabled(),
                "with tickets OFF the ticketer must be the default NeverProducesTickets (enabled() == false)"
            );
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn env_flags_round_trip_and_transport_tuning_from_env_composes_them() {
        // ONE test owns the env-mutating assertions for ENV_TLS_STATELESS_TICKETS + ENV_TLS_SESSION_CACHE_SIZE
        // so there is a SINGLE writer of those process-global vars (two parallel writers would race — Rust
        // runs tests multi-threaded). It exercises both the shared-grammar reader and `from_env`'s
        // composition, then restores the environment.
        //
        // (a) the tickets-flag env reader is the shared affirmative grammar over ENV_TLS_STATELESS_TICKETS:
        std::env::set_var(ENV_TLS_STATELESS_TICKETS, "on");
        assert!(stateless_tickets_from_env(), "'on' ⇒ tickets enabled");
        std::env::set_var(ENV_TLS_STATELESS_TICKETS, "0");
        assert!(!stateless_tickets_from_env(), "'0' ⇒ tickets OFF");

        // (b) from_env() composes BOTH readers (cache size + tickets flag) into one TransportTuning:
        std::env::set_var(ENV_TLS_SESSION_CACHE_SIZE, "777");
        std::env::set_var(ENV_TLS_STATELESS_TICKETS, "true");
        let tuning = TransportTuning::from_env();
        assert_eq!(tuning.session_cache_size, 777);
        assert!(tuning.stateless_tickets);

        // (c) absent env ⇒ the conservative defaults (tuned cache size, tickets OFF).
        std::env::remove_var(ENV_TLS_SESSION_CACHE_SIZE);
        std::env::remove_var(ENV_TLS_STATELESS_TICKETS);
        assert!(
            !stateless_tickets_from_env(),
            "absent ⇒ tickets OFF (the conservative default)"
        );
        let default_tuning = TransportTuning::from_env();
        assert_eq!(
            default_tuning.session_cache_size,
            DEFAULT_TLS_SESSION_CACHE_SIZE
        );
        assert!(
            !default_tuning.stateless_tickets,
            "absent env ⇒ tickets OFF by default"
        );
    }
}

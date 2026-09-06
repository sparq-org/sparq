// AUTHORED-BY Claude Fable 5
//! PoP **Tier 2 — DPoP-SK**: negotiated symmetric session keys for DPoP-bound requests.
//!
//! Design: `research/lws-design-records.md` §7 + §8 (RSS `docs/design/high-throughput-pop-auth.md`
//! §4–§6, not in this tree); normative spec: **DPoP-SK** (<https://jeswr.github.io/dpop-sk-spec/> —
//! the CG-shaped draft whose Appendix-A worked example this implementation reproduces byte-for-byte
//! in tests).
//!
//! ## What it does
//! Removes the per-request ES256 verify floor for opted-in clients: ONE ordinary RFC 9449 DPoP
//! proof at session establishment (`POST /.pop/session`, behind the unchanged verifier) → a
//! short-lived 32-byte symmetric session key — HKDF-derived from TLS Exported Keying Material
//! (`cb=tls-exporter`, native clients; no key bytes on the wire, useless off its connection) or
//! server-generated + TLS-delivered once (`cb=none`, browsers) → per-request RFC 9421
//! `hmac-sha256` message-signature verification (1 HMAC + an O(1) RFC 4303 sliding-window replay
//! check per request).
//!
//! ## What it does NOT change (the no-downgrade contract)
//! - **DPoP stays the mandatory, always-accepted Solid-OIDC baseline.** A request with a valid
//!   DPoP proof and no `dpop-sk` signature is processed as plain DPoP on every build.
//! - **A token is never accepted bare.** Stripping the `Signature*` fields leaves a PoP-less
//!   request ⇒ 401 with the standard DPoP challenge (an attacker stripping negotiation only ever
//!   forces the client back to full DPoP).
//! - **Env-gated OFF by default** (`SOLID_SERVER_DPOP_SK`, mirroring the Tier-1b
//!   `SOLID_SERVER_MTLS_BOUND_TOKENS` gating): flag-off builds run the pre-Tier-2 path
//!   byte-identically — no routes, no metadata member, Signature headers ignored.
//! - **Fail-closed everywhere**: unknown/expired session, wrong connection, replayed counter,
//!   mutated component, alg/flavour mismatch, exporter unavailable ⇒ deny (the one generic
//!   challenge) — never a bearer fallback, never a substituted flavour.
//!
//! ## Module layout
//! - `derive` — exporter label, HKDF key derivation, `confirm`, HMAC primitives (aws-lc-rs).
//! - [`window`] — the RFC 4303 §3.4.3 sliding-window anti-replay (W = 1024).
//! - [`sig`] — strict RFC 8941 parsing + RFC 9421 signature-base reconstruction.
//! - [`store`] — the bounded, expiring session store.
//! - [`verify`] — the per-request verification pipeline (spec §"Verification", in order).
//! - [`handlers`] — the `/.pop/session` establishment/termination endpoint + RFC 9728 metadata.
//!
//! The TLS-exporter plumbing is a SEAM: the acceptor ([`crate::pop::conn`]) exports keying
//! material under [`derive::EXPORTER_LABEL`] once per connection (TLS 1.3 only, in-process
//! termination only) and injects a [`ConnSk`]; where that context is absent — plain HTTP, a
//! TLS-terminating proxy (the current Caddy deployment), TLS 1.2 — `cb=tls-exporter` is neither
//! advertised nor establishable (400), and `cb=none` remains the browser-path offer. The binding
//! is never faked from weaker material.

pub mod derive;
pub mod handlers;
pub mod sig;
pub mod store;
pub mod verify;
pub mod window;

use solid_oidc_verifier::config::{DEFAULT_CLOCK_TOLERANCE_SECS, DPOP_PROOF_MAX_AGE_SECS};

use derive::Ekm;
use store::SessionStore;

/// The session establishment endpoint path (spec §"Discovery"; static, so it precedes the LDP
/// wildcard route).
pub const SESSION_ENDPOINT_PATH: &str = "/.pop/session";

/// The RFC 9728 protected-resource-metadata well-known path.
pub const OAUTH_PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";

/// The env flag enabling the DPoP-SK tier (`1`/`true`). Default OFF — flag-off is byte-identical
/// to the pre-Tier-2 server (mirrors `SOLID_SERVER_MTLS_BOUND_TOKENS`).
pub const ENV_DPOP_SK: &str = "SOLID_SERVER_DPOP_SK";

/// Read [`ENV_DPOP_SK`].
pub fn dpop_sk_from_env() -> bool {
    matches!(
        std::env::var(ENV_DPOP_SK).ok().as_deref().map(str::trim),
        Some("1") | Some("true")
    )
}

/// DPoP-SK configuration.
#[derive(Debug, Clone)]
pub struct SkConfig {
    /// Whether the `cb=tls-exporter` flavour is available: REQUIRES in-process TLS termination
    /// (the acceptor exports keying material). `false` behind any TLS-terminating proxy — the
    /// flavour is then neither advertised nor establishable (spec §"Discovery" MUST NOT).
    pub exporter_available: bool,
    /// Session-store capacity (bounded soft state; eviction is safe).
    pub capacity: usize,
    /// Maximum session lifetime in seconds (spec SHOULD ≤ 300 — the JWKS/revocation window).
    pub max_ttl_secs: i64,
    /// The `created` acceptance half-width — the SAME symmetric window the verifier applies to
    /// DPoP `iat` (spec: "servers SHOULD apply the same window they apply to DPoP iat").
    pub created_window_secs: i64,
}

impl Default for SkConfig {
    fn default() -> Self {
        Self {
            exporter_available: false,
            capacity: 4096,
            max_ttl_secs: 300,
            created_window_secs: (DPOP_PROOF_MAX_AGE_SECS + DEFAULT_CLOCK_TOLERANCE_SECS) as i64,
        }
    }
}

/// The shared DPoP-SK state: the bounded session store + configuration. Held by
/// [`crate::auth::AuthContext`] (the attestation dispatch) and the `/.pop/session` route state.
pub struct SkState {
    store: SessionStore,
    config: SkConfig,
}

impl SkState {
    pub fn new(config: SkConfig) -> Self {
        Self {
            store: SessionStore::new(config.capacity),
            config,
        }
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    pub fn exporter_available(&self) -> bool {
        self.config.exporter_available
    }

    pub fn max_ttl_secs(&self) -> i64 {
        self.config.max_ttl_secs
    }

    pub fn created_window_secs(&self) -> i64 {
        self.config.created_window_secs
    }
}

/// Per-connection DPoP-SK context, injected once per TLS connection by the acceptor
/// ([`crate::pop::conn::ConnPopAcceptor`]) — NEVER trusted from client-controlled request data
/// (the per-connection service overwrites any smuggled extension).
///
/// `conn_id` identifies the TLS connection (for the `cb=tls-exporter` session pin — verification
/// step 3); `ekm` is the 32-byte exporter output under [`derive::EXPORTER_LABEL`], present ONLY
/// on the establishment path of a TLS 1.3 connection (it is a per-connection secret and no other
/// request needs it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnSk {
    pub conn_id: u64,
    pub ekm: Option<Ekm>,
}

impl ConnSk {
    pub fn new(conn_id: u64, ekm: Option<Ekm>) -> Self {
        Self { conn_id, ekm }
    }
}

/// Request extension marking that THIS request was authenticated by a DPoP-SK attestation (not a
/// DPoP proof), naming the authenticating session. Read by the termination handler (`DELETE`
/// names the session to delete) and by the establishment handler (which REFUSES attested
/// establishment — a session must not mint a session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkSession {
    pub session_id: String,
}

/// Current UNIX time in seconds (matches the verifier's clock).
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The spec's execution-verified Appendix-A worked-example vectors (generated by
/// `dpop-sk-spec/tools/compute-examples.mjs`), shared by the unit tests across submodules so the
/// implementation is pinned to the published example byte-for-byte.
#[cfg(test)]
pub(crate) mod test_vectors {
    use super::derive::SessionKey;

    /// The illustrative access token (only its ASCII bytes matter, via `ath`).
    pub const EXAMPLE_TOKEN: &str =
        "eyJhbGciOiJFUzI1NiIsInR5cCI6ImF0K2p3dCJ9.eyJpc3MiOiJodHRwczovL2lkcC5leGFt\
         cGxlIiwic3ViIjoiaHR0cHM6Ly9pZC5leGFtcGxlL2FsaWNlI21lIn0.EXAMPLE-AS-SIGNATURE";
    /// `ath` = base64url(SHA-256(ASCII(access token))) — published value.
    pub const EXAMPLE_ATH: &str = "WKXpcs5xM2Dyko2M0gv0NC8vjApybvIBdn17lZ7izG0";
    /// The `jkt` RFC 9449 §6.1 publishes for its own example key.
    pub const EXAMPLE_JKT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";
    /// `session_id` = base64url(bytes 00..0f).
    pub const EXAMPLE_SESSION_ID: &str = "AAECAwQFBgcICQoLDA0ODw";

    /// The `cb=none` example key: bytes 0x10..0x2f.
    pub fn example_k_none() -> SessionKey {
        let mut b = [0u8; 32];
        for (i, v) in b.iter_mut().enumerate() {
            *v = 0x10 + i as u8;
        }
        SessionKey::new(b)
    }

    /// The example EKM: bytes 0x40..0x5f.
    pub fn example_ekm() -> super::derive::Ekm {
        let mut b = [0u8; 32];
        for (i, v) in b.iter_mut().enumerate() {
            *v = 0x40 + i as u8;
        }
        super::derive::Ekm::new(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flag_default_off() {
        // Serialize env mutation within this test only (no other test reads this var).
        std::env::remove_var(ENV_DPOP_SK);
        assert!(!dpop_sk_from_env());
        std::env::set_var(ENV_DPOP_SK, "1");
        assert!(dpop_sk_from_env());
        std::env::set_var(ENV_DPOP_SK, "true");
        assert!(dpop_sk_from_env());
        std::env::set_var(ENV_DPOP_SK, "0");
        assert!(!dpop_sk_from_env());
        std::env::remove_var(ENV_DPOP_SK);
    }

    #[test]
    fn default_config_mirrors_the_verifier_windows() {
        let c = SkConfig::default();
        assert!(!c.exporter_available, "exporter requires explicit wiring");
        assert_eq!(c.max_ttl_secs, 300);
        assert_eq!(
            c.created_window_secs,
            (DPOP_PROOF_MAX_AGE_SECS + DEFAULT_CLOCK_TOLERANCE_SECS) as i64
        );
    }
}

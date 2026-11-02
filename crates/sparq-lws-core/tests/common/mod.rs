// AUTHORED-BY Claude Opus 4.8
//! Test plumbing — mints the ES256 keys, RFC-9068 access tokens, JWKS, and DPoP proofs needed to
//! drive `solid-server-rs`'s auth middleware against the real `solid-oidc-verifier`.
//!
//! This is a trimmed ES256-only port of the verifier crate's own `tests/common/mod.rs` (we cannot
//! `use` another crate's test-only module). Keys are freshly generated per test; everything is
//! in-process and deterministic.

#![allow(dead_code)]

use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};
use rand_core::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solid_oidc_verifier::config::StaticJwksProvider;
use solid_oidc_verifier::jwk::Jwk;

pub const ISSUER: &str = "https://idp.example/realms/solid";
pub const WEBID: &str = "https://pod.example/alice/profile/card#me";
/// The server's public base URL == the verifier audience == the DPoP htu origin.
pub const BASE_URL: &str = "https://pod.example";
pub const CLIENT_ID: &str = "solid-app";

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_json(v: &Value) -> String {
    b64url(serde_json::to_vec(v).unwrap().as_slice())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

use std::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0);
fn next_id() -> u64 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// An ES256 key pair + its public JWK + the RFC 7638 thumbprint.
pub struct KeyKit {
    pub signing: SigningKey,
    pub public_jwk: Value,
    pub thumbprint: String,
}

impl KeyKit {
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

    pub fn sign(&self, header: &Value, claims: &Value) -> String {
        let signing_input = format!("{}.{}", b64url_json(header), b64url_json(claims));
        let sig: Signature = self.signing.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", b64url(&sig.to_bytes()))
    }
}

/// Mint a well-formed RFC-9068 access token bound to `cnf_jkt`, signed by `issuer_key`.
pub fn mint_access_token(issuer_key: &KeyKit, cnf_jkt: &str) -> String {
    mint_access_token_for(issuer_key, cnf_jkt, WEBID)
}

/// As [`mint_access_token`] but for an ARBITRARY `webid` — needed whenever a test must
/// distinguish TWO different authenticated agents (e.g. the demo playground's disclosed
/// cross-visitor overwrite/delete, where the grant is `acl:AuthenticatedAgent` so the
/// identities must genuinely differ for the assertion to mean anything).
pub fn mint_access_token_for(issuer_key: &KeyKit, cnf_jkt: &str, webid: &str) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = now();
    let claims = json!({
        "iss": ISSUER,
        "sub": webid,
        "jti": format!("at-{}", next_id()),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": webid,
        "cnf": { "jkt": cnf_jkt },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// The RFC 8705 §3.1 `cnf.x5t#S256` for a certificate: base64url-no-pad(SHA-256(cert DER)). Matches
/// `sparq_lws_core::pop::cert_bound::CertThumbprint::from_cert_der(der).to_base64url()`, so a token
/// minted with this value binds to exactly that DER.
pub fn cert_x5t_s256(cert_der: &[u8]) -> String {
    b64url(&Sha256::digest(cert_der))
}

/// Mint a well-formed RFC-9068 access token whose confirmation is an RFC 8705 mTLS certificate binding
/// (`cnf.x5t#S256` ONLY — no `cnf.jkt`), signed by `issuer_key`. Used by the PoP Tier-1b fail-closed
/// tests (needs a verifier configured with `require_dpop(false)` so a Bearer-presented cert-bound token
/// is verified rather than rejected at the DPoP-scheme gate).
pub fn mint_cert_bound_access_token(issuer_key: &KeyKit, x5t_s256: &str) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = now();
    let claims = json!({
        "iss": ISSUER,
        "sub": WEBID,
        "jti": format!("at-{}", next_id()),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": WEBID,
        "cnf": { "x5t#S256": x5t_s256 },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// Mint a well-formed RFC-9068 access token with NO confirmation (`cnf`) claim at all — a plain,
/// UNBOUND bearer token. Under the Solid `require_dpop(true)` posture the verifier rejects this as
/// `Bearer` (DPoP is mandatory for any non-cert-bound token), which the PoP Tier-1 LIVE tests assert
/// stays true.
pub fn mint_unbound_access_token(issuer_key: &KeyKit) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = now();
    let claims = json!({
        "iss": ISSUER,
        "sub": WEBID,
        "jti": format!("at-{}", next_id()),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": WEBID,
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// Mint an RFC-9068 access token carrying BOTH a DPoP (`cnf.jkt`) and an mTLS (`cnf.x5t#S256`)
/// confirmation — the multi-binding case the Tier-1b dispatch refuses fail-closed.
pub fn mint_dual_bound_access_token(issuer_key: &KeyKit, cnf_jkt: &str, x5t_s256: &str) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = now();
    let claims = json!({
        "iss": ISSUER,
        "sub": WEBID,
        "jti": format!("at-{}", next_id()),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": WEBID,
        "cnf": { "jkt": cnf_jkt, "x5t#S256": x5t_s256 },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// Mint an RFC-9068 access token whose `cnf.x5t#S256` is PRESENT but MALFORMED (not a valid 32-byte
/// base64url thumbprint) — the fail-closed "malformed binding" case. `cnf.jkt` is absent.
pub fn mint_malformed_cert_bound_access_token(issuer_key: &KeyKit) -> String {
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = now();
    let claims = json!({
        "iss": ISSUER,
        "sub": WEBID,
        "jti": format!("at-{}", next_id()),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": WEBID,
        "cnf": { "x5t#S256": "not-a-valid-thumbprint" },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// base64url(SHA-256(token)) — the DPoP `ath`.
pub fn ath(token: &str) -> String {
    b64url(&Sha256::digest(token.as_bytes()))
}

/// Mint a well-formed DPoP proof for `method`+`url`, bound to `access_token` via `ath`, embedding
/// `client_key`'s public JWK.
pub fn mint_dpop_proof(client_key: &KeyKit, method: &str, url: &str, access_token: &str) -> String {
    let header = json!({ "alg": "ES256", "typ": "dpop+jwt", "jwk": client_key.public_jwk });
    let claims = json!({
        "htm": method,
        "htu": url,
        "jti": format!("jti-{}", next_id()),
        "iat": now(),
        "ath": ath(access_token),
    });
    client_key.sign(&header, &claims)
}

/// A static JWKS provider over the issuer's key.
pub fn jwks_provider(issuer_key: &KeyKit) -> StaticJwksProvider {
    StaticJwksProvider::new().with_issuer(ISSUER.to_string(), vec![issuer_key.jwk()])
}

#[cfg(not(target_arch = "wasm32"))]
pub mod process;

//! Minimal JOSE for LWS: compact JWS parsing, ES256 signing and verification, RS256
//! verification, and P-256 JWKs.
//!
//! LWS access tokens (RFC 9068) are signed ES256 by this server's authorization server. Subject
//! tokens presented at the token endpoint are verified here too: did:key and controlled identifier
//! credentials are ES256 (or EdDSA for an Ed25519 key), OpenID Connect ID Tokens are ES256 or
//! RS256. `alg: none` and every other algorithm are refused.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, PublicKey, SecretKey};
use serde_json::{json, Map, Value};

/// base64url without padding (RFC 7515 section 2).
pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decode base64url, tolerating trailing padding.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

/// Standard base64 with padding, as RFC 9530 digests and RFC 9421 signatures use.
pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// `n` fresh random bytes from the OS.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("OS randomness");
    buf
}

/// A fresh random identifier: 16 random bytes, base64url.
pub fn random_id() -> String {
    b64url(&random_bytes(16))
}

/// A P-256 key pair with a key id: signs access tokens and notification deliveries.
#[derive(Clone)]
pub struct EcKey {
    secret: SecretKey,
    kid: String,
}

impl std::fmt::Debug for EcKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcKey").field("kid", &self.kid).finish_non_exhaustive()
    }
}

impl EcKey {
    /// A new random key.
    pub fn generate(kid: impl Into<String>) -> Self {
        loop {
            if let Ok(secret) = SecretKey::from_slice(&random_bytes(32)) {
                return Self { secret, kid: kid.into() };
            }
        }
    }

    /// A key from a private P-256 JWK (`kty EC`, `crv P-256`, with `d`). The JWK's `kid` is kept;
    /// without one the RFC 7638 thumbprint is used.
    pub fn from_jwk(jwk: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(jwk).map_err(|e| format!("not a private P-256 JWK: {e}"))?;
        // Only the key members: the JWK parser refuses members it does not know (`alg`, `use`).
        let mut core = Map::new();
        for k in ["kty", "crv", "x", "y", "d"] {
            if let Some(v) = value.get(k) {
                core.insert(k.into(), v.clone());
            }
        }
        let secret = SecretKey::from_jwk_str(&Value::Object(core).to_string()).map_err(|e| format!("not a private P-256 JWK: {e}"))?;
        let kid = value
            .get("kid")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| thumbprint(&public_jwk_of(&secret.public_key())));
        Ok(Self { secret, kid })
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub fn public_key(&self) -> PublicKey {
        self.secret.public_key()
    }

    /// The public JWK, with `kid`, `alg` and `use`.
    pub fn public_jwk(&self) -> Value {
        let mut jwk = public_jwk_of(&self.public_key());
        if let Some(map) = jwk.as_object_mut() {
            map.insert("kid".into(), Value::String(self.kid.clone()));
            map.insert("alg".into(), Value::String("ES256".into()));
            map.insert("use".into(), Value::String("sig".into()));
        }
        jwk
    }

    /// The private JWK (for handing to a conformance harness, never served).
    pub fn private_jwk(&self) -> Value {
        let mut jwk = self.public_jwk();
        if let Some(map) = jwk.as_object_mut() {
            map.insert("d".into(), Value::String(b64url(&self.secret.to_bytes())));
        }
        jwk
    }

    /// A raw ES256 signature (r || s, 64 bytes) over `message`.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let key = SigningKey::from(&self.secret);
        let sig: Signature = key.sign(message);
        sig.to_bytes().to_vec()
    }

    /// A compact JWS: `header` gets `alg ES256` and this key's `kid` added.
    pub fn sign_jws(&self, mut header: Map<String, Value>, claims: &Value) -> String {
        header.insert("alg".into(), Value::String("ES256".into()));
        header.insert("kid".into(), Value::String(self.kid.clone()));
        let input = format!(
            "{}.{}",
            b64url(Value::Object(header).to_string().as_bytes()),
            b64url(claims.to_string().as_bytes())
        );
        let sig = self.sign(input.as_bytes());
        format!("{input}.{}", b64url(&sig))
    }
}

/// The public JWK of a P-256 key, without `kid`.
pub fn public_jwk_of(key: &PublicKey) -> Value {
    let point = key.to_encoded_point(false);
    json!({
        "kty": "EC",
        "crv": "P-256",
        "x": b64url(point.x().map(|x| x.as_slice()).unwrap_or_default()),
        "y": b64url(point.y().map(|y| y.as_slice()).unwrap_or_default()),
    })
}

/// The RFC 7638 thumbprint of an EC or RSA public JWK.
pub fn thumbprint(jwk: &Value) -> String {
    let get = |k: &str| jwk.get(k).and_then(Value::as_str).unwrap_or_default();
    let canonical = match get("kty") {
        "RSA" => format!(r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#, get("e"), get("n")),
        "OKP" => format!(r#"{{"crv":"{}","kty":"OKP","x":"{}"}}"#, get("crv"), get("x")),
        _ => format!(r#"{{"crv":"{}","kty":"EC","x":"{}","y":"{}"}}"#, get("crv"), get("x"), get("y")),
    };
    use sha2::Digest;
    b64url(&sha2::Sha256::digest(canonical.as_bytes()))
}

/// A P-256 public key from a JWK, or `None` when it is not one.
pub fn ec_public_from_jwk(jwk: &Value) -> Option<PublicKey> {
    if jwk.get("kty")?.as_str()? != "EC" || jwk.get("crv")?.as_str()? != "P-256" {
        return None;
    }
    let x = b64url_decode(jwk.get("x")?.as_str()?)?;
    let y = b64url_decode(jwk.get("y")?.as_str()?)?;
    if x.len() != 32 || y.len() != 32 {
        return None;
    }
    let point = EncodedPoint::from_affine_coordinates(x.as_slice().into(), y.as_slice().into(), false);
    Option::from(PublicKey::from_encoded_point(&point))
}

/// A P-256 public key from its SEC1 encoding (compressed or not), as did:key carries it.
pub fn ec_public_from_sec1(bytes: &[u8]) -> Option<PublicKey> {
    PublicKey::from_sec1_bytes(bytes).ok()
}

/// A parsed compact JWS. Nothing about it is trusted until a `verify_*` call succeeds.
#[derive(Debug, Clone)]
pub struct Jws {
    pub header: Map<String, Value>,
    pub claims: Map<String, Value>,
    signing_input: String,
    signature: Vec<u8>,
}

impl Jws {
    /// Parse `header.payload.signature`. The header and payload must be JSON objects.
    pub fn parse(compact: &str) -> Option<Self> {
        let mut parts = compact.trim().split('.');
        let (h, p, s) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let header = match serde_json::from_slice::<Value>(&b64url_decode(h)?).ok()? {
            Value::Object(m) => m,
            _ => return None,
        };
        let claims = match serde_json::from_slice::<Value>(&b64url_decode(p)?).ok()? {
            Value::Object(m) => m,
            _ => return None,
        };
        Some(Self { header, claims, signing_input: format!("{h}.{p}"), signature: b64url_decode(s)? })
    }

    pub fn alg(&self) -> Option<&str> {
        self.header.get("alg").and_then(Value::as_str)
    }

    pub fn kid(&self) -> Option<&str> {
        self.header.get("kid").and_then(Value::as_str)
    }

    pub fn typ(&self) -> Option<&str> {
        self.header.get("typ").and_then(Value::as_str)
    }

    pub fn claim_str(&self, name: &str) -> Option<&str> {
        self.claims.get(name).and_then(Value::as_str)
    }

    /// A NumericDate claim (seconds since the epoch).
    pub fn claim_time(&self, name: &str) -> Option<i64> {
        self.claims.get(name).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    }

    /// The `aud` claim as a list (a single string is a one-element list).
    pub fn audiences(&self) -> Vec<String> {
        match self.claims.get("aud") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        }
    }

    /// Verify an ES256 signature with `key`. Fails for any other `alg`.
    pub fn verify_es256(&self, key: &PublicKey) -> bool {
        if self.alg() != Some("ES256") || self.signature.len() != 64 {
            return false;
        }
        let Ok(sig) = Signature::from_slice(&self.signature) else {
            return false;
        };
        VerifyingKey::from(key).verify(self.signing_input.as_bytes(), &sig).is_ok()
    }

    /// Verify with a public JWK: ES256 for a P-256 key, RS256 for an RSA key. The JWK's own `alg`,
    /// when present, must agree with the token's.
    pub fn verify_jwk(&self, jwk: &Value) -> bool {
        let alg = self.alg().unwrap_or_default();
        if let Some(declared) = jwk.get("alg").and_then(Value::as_str) {
            if declared != alg {
                return false;
            }
        }
        match (alg, jwk.get("kty").and_then(Value::as_str)) {
            ("ES256", Some("EC")) => ec_public_from_jwk(jwk).is_some_and(|k| self.verify_es256(&k)),
            ("RS256", Some("RSA")) => self.verify_rs256(jwk),
            ("EdDSA" | "Ed25519", Some("OKP")) => {
                jwk.get("crv").and_then(Value::as_str) == Some("Ed25519")
                    && jwk.get("x").and_then(Value::as_str).and_then(b64url_decode).is_some_and(|x| self.verify_ed25519(&x))
            }
            _ => false,
        }
    }

    /// Verify an EdDSA (Ed25519) signature with a raw 32-byte public key. Fails for any other
    /// `alg`.
    pub fn verify_ed25519(&self, key: &[u8]) -> bool {
        if !matches!(self.alg(), Some("EdDSA" | "Ed25519")) || key.len() != 32 || self.signature.len() != 64 {
            return false;
        }
        aws_lc_rs::signature::UnparsedPublicKey::new(&aws_lc_rs::signature::ED25519, key)
            .verify(self.signing_input.as_bytes(), &self.signature)
            .is_ok()
    }

    fn verify_rs256(&self, jwk: &Value) -> bool {
        let (Some(n), Some(e)) = (
            jwk.get("n").and_then(Value::as_str).and_then(b64url_decode),
            jwk.get("e").and_then(Value::as_str).and_then(b64url_decode),
        ) else {
            return false;
        };
        let key = aws_lc_rs::signature::RsaPublicKeyComponents { n: &n, e: &e };
        key.verify(
            &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
            self.signing_input.as_bytes(),
            &self.signature,
        )
        .is_ok()
    }
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es256_round_trip_and_tamper() {
        let key = EcKey::generate("k1");
        let token = key.sign_jws(Map::new(), &json!({"sub": "a"}));
        let jws = Jws::parse(&token).unwrap();
        assert!(jws.verify_es256(&key.public_key()));
        assert!(jws.verify_jwk(&key.public_jwk()));
        let other = EcKey::generate("k2");
        assert!(!jws.verify_es256(&other.public_key()));
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = b64url(br#"{"sub":"b"}"#);
        parts[1] = &forged;
        assert!(!Jws::parse(&parts.join(".")).unwrap().verify_es256(&key.public_key()));
    }

    #[test]
    fn private_jwk_round_trips() {
        let key = EcKey::generate("abc");
        let again = EcKey::from_jwk(&key.private_jwk().to_string()).unwrap();
        assert_eq!(again.kid(), "abc");
        assert_eq!(again.public_jwk(), key.public_jwk());
    }

    #[test]
    fn alg_none_never_verifies() {
        let key = EcKey::generate("k");
        let token = format!("{}.{}.", b64url(br#"{"alg":"none"}"#), b64url(br#"{"sub":"a"}"#));
        let jws = Jws::parse(&token).unwrap();
        assert!(!jws.verify_es256(&key.public_key()));
        assert!(!jws.verify_jwk(&key.public_jwk()));
    }
}

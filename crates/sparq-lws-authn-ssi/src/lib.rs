//! The self-issued authentication suites of the Linked Web Storage authorization server (LWS 1.0
//! core section 5.2.3): a JWT the agent signs itself, as the subject token. Each is checked the way
//! its specification and the Touchstone reference authorization server check it:
//!
//! - **did:key** (`lws10-authn-ssi-did-key`): a self-issued JWT whose subject is a did:key. The key
//!   comes from the identifier itself (P-256, multicodec `0x1200`, signed ES256; or Ed25519,
//!   multicodec `0xed`, signed EdDSA), and `kid` must name the one verification method of the DID
//!   document the identifier derives (`did:key:z…#z…`, or just its fragment).
//! - **Controlled identifier** (`lws10-authn-ssi-cid`): a self-issued JWT whose subject is an HTTPS
//!   URI. The controlled identifier document is fetched from the subject, its `id` must be the
//!   subject, and `kid` must name a method of its `authentication` relationship (embedded or
//!   referenced) that the subject controls and that is neither revoked nor expired.
//!
//! Self-issued credentials need `sub = iss = client_id`, an `aud` that includes this authorization
//! server, an `exp` in the future and an `iat` not ahead of now. `alg: none` is always refused.
//!
//! Documents are fetched under the server's fetch policy (see [`sparq_lws_authn::fetch`]).
//! [`Ssi`] is the suite, for a server's [`Suites`](sparq_lws_authn::Suites).

#![forbid(unsafe_code)]

use serde_json::Value;

use sparq_lws_authn::fetch;
use sparq_lws_authn::jose::{self, Jws};
pub use sparq_lws_authn::JWT_TOKEN_TYPE;
use sparq_lws_authn::{
    check_times, jwt_expires, parse_datetime, parse_jwt, Exchange, Suite, Verified,
};

/// The self-issued suites: did:key and controlled identifier subjects of a JWT subject token.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ssi;

#[async_trait::async_trait]
impl Suite for Ssi {
    fn name(&self) -> &'static str {
        "ssi"
    }

    fn token_types(&self) -> &[&'static str] {
        &[JWT_TOKEN_TYPE]
    }

    fn identifier_types(&self) -> &[&'static str] {
        &["https", "did:key"]
    }

    async fn verify(&self, exchange: &Exchange<'_>) -> Result<Verified, String> {
        let jws = parse_jwt(exchange.subject_token)?;
        if jws
            .claim_str("sub")
            .is_some_and(|s| s.starts_with("did:key:"))
        {
            did_key(exchange.authorization_server, &jws, exchange.now)
        } else {
            cid(exchange, &jws, &jose::now_secs).await
        }
    }
}

/// The self-issued credential checks shared by did:key and CID subjects, for the authorization
/// server `issuer`; the subject on success.
pub fn self_issued(issuer: &str, jws: &Jws, now: i64) -> Result<String, String> {
    let sub = jws.claim_str("sub").filter(|s| !s.is_empty());
    let Some(sub) = sub else {
        return Err("sub, iss and client_id must be the same identifier (sub is missing)".into());
    };
    if jws.claim_str("iss") != Some(sub) || jws.claim_str("client_id") != Some(sub) {
        return Err("sub, iss and client_id must be the same identifier".into());
    }
    check_times(jws, now)?;
    if !sparq_lws_authn::addressed_to(issuer, jws) {
        return Err("aud does not include this authorization server".into());
    }
    Ok(sub.to_string())
}

// ---------------------------------------------------------------- did:key

/// A public key a did:key or a multibase verification method carries.
#[derive(Debug, Clone)]
pub enum MultikeyPublic {
    P256(p256::PublicKey),
    Ed25519([u8; 32]),
}

impl MultikeyPublic {
    /// Verify `jws` with this key, insisting on the algorithm the key type implies.
    pub fn verify(&self, jws: &Jws) -> Result<(), String> {
        let ok = match self {
            MultikeyPublic::P256(k) => {
                if jws.alg() != Some("ES256") {
                    return Err(format!(
                        "a P-256 key signs ES256, not {}",
                        jws.alg().unwrap_or_default()
                    ));
                }
                jws.verify_es256(k)
            }
            MultikeyPublic::Ed25519(k) => {
                if !matches!(jws.alg(), Some("EdDSA" | "Ed25519")) {
                    return Err(format!(
                        "an Ed25519 key signs EdDSA, not {}",
                        jws.alg().unwrap_or_default()
                    ));
                }
                jws.verify_ed25519(k)
            }
        };
        if ok {
            Ok(())
        } else {
            Err("the signature does not verify".into())
        }
    }
}

/// The longest base58btc multikey [`decode_multikey`] considers, in characters: well past the 48 of
/// the largest supported key (a compressed P-256 point with its 2-byte multicodec prefix).
pub const MAX_MULTIKEY_CHARS: usize = 128;

/// Decode a multibase base58btc (`z…`) multikey: P-256 (multicodec `0x1200`, the compressed SEC1
/// point) or Ed25519 (multicodec `0xed`, the raw 32-byte key).
///
/// The supported keys are at most 35 bytes (48 base58 characters), so a longer value is refused
/// before it is decoded (base58 decoding is quadratic in the input), and the decode goes into a
/// fixed buffer that a longer value overflows.
pub fn decode_multikey(multibase: &str) -> Result<MultikeyPublic, String> {
    let b58 = multibase
        .strip_prefix('z')
        .ok_or("not a base58btc multibase value")?;
    if b58.len() > MAX_MULTIKEY_CHARS {
        return Err("the multikey is longer than any supported key".into());
    }
    let mut buf = [0u8; 64];
    let len = bs58::decode(b58)
        .onto(&mut buf)
        .map_err(|_| "not valid base58btc, or longer than any supported key")?;
    match &buf[..len] {
        [0x80, 0x24, point @ ..] if point.len() == 33 && matches!(point[0], 2 | 3) => {
            jose::ec_public_from_sec1(point)
                .map(MultikeyPublic::P256)
                .ok_or_else(|| "the point is not on P-256".into())
        }
        [0xed, 0x01, raw @ ..] if raw.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(raw);
            Ok(MultikeyPublic::Ed25519(k))
        }
        _ => Err("the key is neither a compressed P-256 nor an Ed25519 multikey".into()),
    }
}

/// The key a `did:key:z…` identifier names.
pub fn did_key_public(did: &str) -> Result<MultikeyPublic, String> {
    let mb = did.strip_prefix("did:key:").ok_or("not a did:key")?;
    if mb.contains(['#', '?', '/', ':']) {
        return Err("a did:key subject carries no path, query or fragment".into());
    }
    decode_multikey(mb)
}

/// Whether `kid` names the one verification method of the DID document `did` derives:
/// `did:key:z…#z…`, or its fragment alone (with or without the `#`).
pub fn did_key_kid_matches(did: &str, kid: Option<&str>) -> bool {
    let Some(mb) = did.strip_prefix("did:key:") else {
        return false;
    };
    let method = format!("{did}#{mb}");
    kid.is_some_and(|k| k == method || k == mb || k.strip_prefix('#') == Some(mb))
}

fn did_key(issuer: &str, jws: &Jws, now: i64) -> Result<Verified, String> {
    let did = self_issued(issuer, jws, now)?;
    if !did_key_kid_matches(&did, jws.kid()) {
        return Err(format!(
            "the kid names no verification method of the DID document {did} derives"
        ));
    }
    did_key_public(&did)?.verify(jws)?;
    Ok(Verified {
        subject: did.clone(),
        client: did,
        expires: jwt_expires(jws)?,
    })
}

// ---------------------------------------------------------------- controlled identifiers

/// A controlled identifier subject. The method's revocation and expiry are checked at `clock`'s
/// time once its document is fetched, not at the start of the exchange.
async fn cid(
    ex: &Exchange<'_>,
    jws: &Jws,
    clock: &(dyn Fn() -> i64 + Sync),
) -> Result<Verified, String> {
    let subject = self_issued(ex.authorization_server, jws, ex.now)?;
    if !(subject.starts_with("https://") || subject.starts_with("http://")) {
        return Err("a controlled identifier credential needs an https subject".into());
    }
    let kid = jws
        .kid()
        .filter(|k| !k.is_empty())
        .ok_or("a CID credential names its verification method in kid, and this one has none")?;
    let (_, body) = fetch::fetch(
        &ex.fetch,
        ex.http,
        &subject,
        "application/cid+json, application/ld+json, application/json",
    )
    .await?;
    let doc: Value = serde_json::from_slice(&body)
        .map_err(|_| format!("{subject} is not a JSON controlled identifier document"))?;
    let key = cid_method_key(&doc, &subject, kid, clock())?;
    key.verify(jws)?;
    Ok(Verified {
        subject: subject.clone(),
        client: subject,
        expires: jwt_expires(jws)?,
    })
}

/// A key a CID verification method publishes.
#[derive(Debug, Clone)]
pub enum MethodKey {
    Jwk(Value),
    Multikey(MultikeyPublic),
}

impl MethodKey {
    pub fn verify(&self, jws: &Jws) -> Result<(), String> {
        match self {
            MethodKey::Multikey(k) => k.verify(jws),
            MethodKey::Jwk(jwk) => {
                // The key type fixes the algorithm; verify_jwk refuses any other pairing.
                if jws.verify_jwk(jwk) {
                    Ok(())
                } else {
                    Err("the signature does not verify with the verification method's key".into())
                }
            }
        }
    }
}

/// Resolve a reference in the subject's document: `#frag` against the document URL, anything
/// else as it is.
fn resolve_ref(r: &str, subject: &str) -> String {
    match r.strip_prefix('#') {
        Some(frag) => format!("{}#{frag}", subject.split('#').next().unwrap_or(subject)),
        None => r.to_string(),
    }
}

/// The key of the authentication method `kid` names in the controlled identifier document `doc`
/// of `subject` (CID 1.0 section 3.3): the document must be the subject's, the method must be in
/// the `authentication` relationship (embedded, or referenced and defined under
/// `verificationMethod`), controlled by the subject, and neither revoked nor expired at `now`.
pub fn cid_method_key(
    doc: &Value,
    subject: &str,
    kid: &str,
    now: i64,
) -> Result<MethodKey, String> {
    if doc.get("id").and_then(Value::as_str) != Some(subject) {
        return Err(format!(
            "the controlled identifier document at {subject} names another id"
        ));
    }
    let method_id = if kid.contains(':') {
        kid.to_string()
    } else {
        resolve_ref(&format!("#{}", kid.trim_start_matches('#')), subject)
    };
    let id_of = |m: &Value| {
        m.get("id")
            .and_then(Value::as_str)
            .map(|i| resolve_ref(i, subject))
    };
    let as_list = |v: Option<&Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(a)) => a.clone(),
            Some(v @ (Value::Object(_) | Value::String(_))) => vec![v.clone()],
            _ => Vec::new(),
        }
    };
    // Only `method_id` is looked for: the method it names, defined once, and a reference to it
    // (or an embedded method with its id) in `authentication`. Each list is read once, however
    // long a document makes them.
    let named = |m: &Value| m.is_object() && id_of(m).as_deref() == Some(method_id.as_str());
    let defined = as_list(doc.get("verificationMethod"))
        .into_iter()
        .find(|c| named(c));
    let mut method = None;
    for entry in as_list(doc.get("authentication")) {
        match &entry {
            Value::String(r) if defined.is_some() && resolve_ref(r, subject) == method_id => {
                method = defined;
                break;
            }
            Value::Object(_) if named(&entry) => {
                method = Some(entry);
                break;
            }
            _ => {}
        }
    }
    let method = method.ok_or_else(|| {
        format!("the controlled identifier document names no authentication method {method_id}")
    })?;
    if method
        .get("controller")
        .and_then(Value::as_str)
        .map(|c| resolve_ref(c, subject))
        .as_deref()
        != Some(subject)
    {
        return Err(format!(
            "verification method {method_id} is controlled by another identifier"
        ));
    }
    for bound in ["revoked", "expires"] {
        if let Some(v) = method.get(bound).filter(|v| !v.is_null()) {
            let at = v.as_str().and_then(parse_datetime).ok_or_else(|| {
                format!("verification method {method_id} has an unreadable {bound} time")
            })?;
            if now >= at {
                return Err(format!("verification method {method_id} is {bound}"));
            }
        }
    }
    if let Some(jwk) = method.get("publicKeyJwk").filter(|j| j.is_object()) {
        if jwk.get("d").is_some() {
            return Err(format!(
                "verification method {method_id} publishes a private key"
            ));
        }
        return Ok(MethodKey::Jwk(jwk.clone()));
    }
    if let Some(mb) = method.get("publicKeyMultibase").and_then(Value::as_str) {
        return decode_multikey(mb).map(MethodKey::Multikey);
    }
    Err(format!(
        "verification method {method_id} carries no usable public key"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    /// What the tests need of the server's configuration: its authorization server's id.
    struct LwsConfig(&'static str);

    impl LwsConfig {
        fn issuer(&self) -> &str {
            self.0
        }
    }

    fn cfg() -> LwsConfig {
        LwsConfig("http://localhost:3000/.lws/as")
    }

    fn did_for(key: &jose::EcKey) -> String {
        let point =
            p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&key.public_key(), true);
        let mut bytes = vec![0x80, 0x24];
        bytes.extend_from_slice(point.as_bytes());
        format!("did:key:z{}", bs58::encode(bytes).into_string())
    }

    fn sign(key: &jose::EcKey, kid: &str, claims: Value) -> Jws {
        // sign_jws sets the key's own kid; build the header by hand to choose it.
        let header = json!({"alg": "ES256", "typ": "JWT", "kid": kid});
        let input = format!(
            "{}.{}",
            jose::b64url(header.to_string().as_bytes()),
            jose::b64url(claims.to_string().as_bytes())
        );
        let sig = key.sign(input.as_bytes());
        Jws::parse(&format!("{input}.{}", jose::b64url(&sig))).unwrap()
    }

    fn claims(did: &str, cfg: &LwsConfig) -> Value {
        let now = jose::now_secs();
        json!({"sub": did, "iss": did, "client_id": did, "aud": [cfg.issuer()], "iat": now, "exp": now + 300})
    }

    #[test]
    fn did_key_p256_decodes_and_verifies() {
        let cfg = cfg();
        let key = jose::EcKey::generate("x");
        let did = did_for(&key);
        let mb = did.strip_prefix("did:key:").unwrap().to_string();
        match did_key_public(&did).unwrap() {
            MultikeyPublic::P256(k) => assert_eq!(k, key.public_key()),
            other => panic!("{other:?}"),
        }
        let jws = sign(&key, &mb, claims(&did, &cfg));
        assert_eq!(
            did_key(cfg.issuer(), &jws, jose::now_secs()).unwrap(),
            Verified {
                subject: did.clone(),
                client: did.clone(),
                expires: jwt_expires(&jws).unwrap(),
            }
        );
        let jws = sign(&key, &format!("{did}#{mb}"), claims(&did, &cfg));
        assert!(did_key(cfg.issuer(), &jws, jose::now_secs()).is_ok());
        // Wrong kid, wrong key.
        assert!(did_key(
            cfg.issuer(),
            &sign(&key, "other", claims(&did, &cfg)),
            jose::now_secs()
        )
        .is_err());
        let other = jose::EcKey::generate("y");
        assert!(did_key(
            cfg.issuer(),
            &sign(&other, &mb, claims(&did, &cfg)),
            jose::now_secs()
        )
        .is_err());
    }

    #[test]
    fn ed25519_did_key_decodes() {
        // did:key test vector from the did:key specification.
        let did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        assert!(matches!(
            did_key_public(did).unwrap(),
            MultikeyPublic::Ed25519(_)
        ));
        assert!(did_key_public("did:key:zQ3s").is_err());
        assert!(did_key_public("did:web:example.org").is_err());
        // Review finding: an oversized multikey was base58-decoded in full (quadratic) before any
        // length check. It is refused up front, and a value under the cap that decodes past the
        // largest key overflows the fixed buffer instead of growing one.
        let huge = format!("did:key:z{}", "2".repeat(100_000));
        let started = std::time::Instant::now();
        let err = did_key_public(&huge).unwrap_err();
        assert!(err.contains("longer"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        let long = format!("z{}", "2".repeat(MAX_MULTIKEY_CHARS));
        assert!(decode_multikey(&long).unwrap_err().contains("longer"));
        assert!(did_key_kid_matches(
            did,
            Some("z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK")
        ));
        assert!(!did_key_kid_matches(did, None));
    }

    #[test]
    fn self_issued_claim_faults() {
        let cfg = cfg();
        let key = jose::EcKey::generate("x");
        let did = did_for(&key);
        let now = jose::now_secs();
        let check = |edit: &dyn Fn(&mut Map<String, Value>)| {
            let mut c = claims(&did, &cfg);
            edit(c.as_object_mut().unwrap());
            self_issued(cfg.issuer(), &sign(&key, "k", c), now)
        };
        assert!(check(&|_| {}).is_ok());
        assert!(check(&|c| {
            c.remove("sub");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("iss");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("client_id");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("client_id".into(), json!("https://other.example/"));
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("exp");
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("iat");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("exp".into(), json!(now - 3600));
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("iat".into(), json!(now + 3600));
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("aud".into(), json!(["https://elsewhere.example"]));
        })
        .is_err());
        assert!(check(&|c| {
            c.remove("aud");
        })
        .is_err());
        assert!(check(&|c| {
            c.insert("aud".into(), json!(format!("{}/", cfg.issuer())));
        })
        .is_ok());
    }

    /// Review finding: a method's expiry was checked at the time the exchange began, so one that
    /// expired while its document was being fetched still verified. It is checked after the
    /// fetch.
    #[tokio::test]
    async fn cid_methods_are_checked_when_their_document_arrives() {
        let key = jose::EcKey::generate("k");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let subject = format!("http://{}/id", listener.local_addr().unwrap());
        let start = jose::now_secs();
        let mut doc = cid_doc(&subject, jose::public_jwk_of(&key.public_key()));
        doc["authentication"][0]["expires"] = json!("2001-01-01T00:00:10Z");
        let served = doc.clone();
        let app = axum::Router::new().route(
            "/id",
            axum::routing::get(move || async move { axum::Json(served) }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let claims = json!({"sub": subject, "iss": subject, "client_id": subject,
            "aud": ["http://localhost:3000"], "iat": start, "exp": start + 300});
        let token = sign(&key, "k", claims);
        let policy = fetch::Policy {
            allow_insecure_fetch: true,
        };
        let http = fetch::fetch_client(&policy).unwrap();
        let ex = Exchange {
            subject_token: "",
            subject_token_type: JWT_TOKEN_TYPE,
            authorization_server: "http://localhost:3000",
            token_endpoint: "http://localhost:3000/token",
            dpop: None,
            now: start,
            fetch: policy,
            http: &http,
        };
        // 2001-01-01T00:00:10Z is 978307210: valid just before, expired from then on.
        assert!(cid(&ex, &token, &|| 978_307_209).await.is_ok());
        let err = cid(&ex, &token, &|| 978_307_210).await.unwrap_err();
        assert!(err.contains("expires"), "{err}");
    }

    fn cid_doc(subject: &str, jwk: Value) -> Value {
        json!({
            "id": subject,
            "authentication": [{"id": format!("{subject}#k"), "type": "JsonWebKey", "controller": subject, "publicKeyJwk": jwk}],
        })
    }

    /// Sweep finding: each `authentication` reference was looked up by scanning every
    /// `verificationMethod`, so a document of a hundred thousand of each, fetched before any
    /// signature is checked, cost billions of comparisons. Each list is now read once.
    #[test]
    fn cid_lookup_reads_each_list_once() {
        let s = "https://alice.example/id";
        let key = jose::EcKey::generate("k");
        let jwk = jose::public_jwk_of(&key.public_key());
        let n = 50_000;
        let mut refs: Vec<Value> = (0..n).map(|i| json!(format!("#x{i}"))).collect();
        let mut methods: Vec<Value> = (0..n)
            .map(|i| json!({"id": format!("#y{i}"), "controller": s, "publicKeyJwk": jwk}))
            .collect();
        refs.push(json!("#k"));
        methods.push(json!({"id": "#k", "controller": s, "publicKeyJwk": jwk}));
        let doc = json!({"id": s, "authentication": refs, "verificationMethod": methods});
        let started = std::time::Instant::now();
        assert!(cid_method_key(&doc, s, "k", jose::now_secs()).is_ok());
        assert!(cid_method_key(&doc, s, "nope", jose::now_secs()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A reference to a method the document does not define gives way to an embedded one.
        let doc = json!({"id": s, "authentication": ["#k", {"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", jose::now_secs()).is_ok());
    }

    #[test]
    fn cid_method_rules() {
        let s = "https://alice.example/id";
        let key = jose::EcKey::generate("k");
        let jwk = jose::public_jwk_of(&key.public_key());
        let now = jose::now_secs();
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "k", now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "#k", now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, &format!("{s}#k"), now).is_ok());
        assert!(cid_method_key(&cid_doc(s, jwk.clone()), s, "nope", now).is_err());
        // Another id.
        assert!(
            cid_method_key(&cid_doc("https://bob.example/id", jwk.clone()), s, "k", now).is_err()
        );
        // Referenced method.
        let doc = json!({"id": s, "authentication": ["#k"], "verificationMethod": [{"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", now).is_ok());
        // Listed only for assertion.
        let doc = json!({"id": s, "assertionMethod": ["#k"], "verificationMethod": [{"id": "#k", "controller": s, "publicKeyJwk": jwk}]});
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        // Foreign controller, revoked, expired in the future is fine.
        let mut doc = cid_doc(s, jwk.clone());
        doc["authentication"][0]["controller"] = json!("https://mallory.example/id");
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        let mut doc = cid_doc(s, jwk.clone());
        doc["authentication"][0]["revoked"] = json!("2020-01-01T00:00:00Z");
        assert!(cid_method_key(&doc, s, "k", now).is_err());
        let mut doc = cid_doc(s, jwk);
        doc["authentication"][0]["expires"] = json!("2999-01-01T00:00:00Z");
        assert!(cid_method_key(&doc, s, "k", now).is_ok());
    }
}

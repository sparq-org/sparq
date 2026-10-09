//! Subject token verification for the LWS authorization server (LWS 1.0 core section 5.2.3,
//! "validate before issuing"). Each authentication suite is checked the way its specification and
//! the Touchstone reference authorization server check it:
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
//! Documents are fetched with the server's HTTP client. Unless `allow_insecure_fetch` is set, only
//! `https:` URLs are fetched, and never from loopback, private, link-local or otherwise non-global
//! addresses: every redirect hop is checked before it is requested (the client follows none
//! itself), the client's resolver hands out public addresses only, so the connection goes where
//! the check looked, and no proxy is used.

use std::net::IpAddr;

use serde_json::Value;

use super::jose::{self, Jws};
use super::LwsConfig;

pub const JWT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";

/// Allowed clock skew, in seconds, for subject token times (the reference's).
pub const SKEW_SECS: i64 = 60;

/// Largest identity document, discovery document or JWKS read.
const MAX_DOC: usize = 1024 * 1024;

/// Who a valid subject token authenticates, and the client presenting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub subject: String,
    pub client: String,
}

/// Verify a JWT-shaped subject token of `token_type` for an exchange at this authorization server.
/// The error is the reason, for `error_description`.
pub async fn verify(
    cfg: &LwsConfig,
    http: &reqwest::Client,
    token: &str,
    token_type: &str,
) -> Result<Verified, String> {
    match token_type {
        JWT_TOKEN_TYPE => {
            let jws = parse(token)?;
            if jws
                .claim_str("sub")
                .is_some_and(|s| s.starts_with("did:key:"))
            {
                did_key(cfg, &jws)
            } else {
                cid(cfg, http, &jws).await
            }
        }
        other => Err(format!("unsupported subject_token_type {other}")),
    }
}

/// A signed JWT: three parts, JSON header and claims, and an `alg` that is not `none`.
pub fn parse(token: &str) -> Result<Jws, String> {
    let jws = Jws::parse(token).ok_or("the subject token is not a compact JWS")?;
    match jws.alg() {
        None => Err("the subject token names no alg".into()),
        Some(a) if a.eq_ignore_ascii_case("none") => Err("alg none is refused".into()),
        Some(_) => Ok(jws),
    }
}

/// `exp` present and not past, `iat` present and not ahead of now, `nbf` (when present) reached.
pub fn check_times(jws: &Jws, now: i64) -> Result<(), String> {
    let exp = jws.claim_time("exp").ok_or("exp is required")?;
    if exp <= now - SKEW_SECS {
        return Err("the credential has expired".into());
    }
    let iat = jws.claim_time("iat").ok_or("iat is required")?;
    if iat > now + SKEW_SECS {
        return Err("iat lies in the future".into());
    }
    if jws
        .claim_time("nbf")
        .is_some_and(|nbf| nbf > now + SKEW_SECS)
    {
        return Err("the credential is not valid yet".into());
    }
    Ok(())
}

/// Whether `aud` names this authorization server (its issuer identifier, with or without the
/// trailing slash).
fn addressed_to_us(cfg: &LwsConfig, jws: &Jws) -> bool {
    let issuer = cfg.issuer();
    jws.audiences()
        .iter()
        .any(|a| a == issuer || a.strip_suffix('/') == Some(issuer))
}

/// The self-issued credential checks shared by did:key and CID subjects; the subject on success.
pub fn self_issued(cfg: &LwsConfig, jws: &Jws, now: i64) -> Result<String, String> {
    let sub = jws.claim_str("sub").filter(|s| !s.is_empty());
    let Some(sub) = sub else {
        return Err("sub, iss and client_id must be the same identifier (sub is missing)".into());
    };
    if jws.claim_str("iss") != Some(sub) || jws.claim_str("client_id") != Some(sub) {
        return Err("sub, iss and client_id must be the same identifier".into());
    }
    check_times(jws, now)?;
    if !addressed_to_us(cfg, jws) {
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

fn did_key(cfg: &LwsConfig, jws: &Jws) -> Result<Verified, String> {
    let did = self_issued(cfg, jws, jose::now_secs())?;
    if !did_key_kid_matches(&did, jws.kid()) {
        return Err(format!(
            "the kid names no verification method of the DID document {did} derives"
        ));
    }
    did_key_public(&did)?.verify(jws)?;
    Ok(Verified {
        subject: did.clone(),
        client: did,
    })
}

// ---------------------------------------------------------------- controlled identifiers

async fn cid(cfg: &LwsConfig, http: &reqwest::Client, jws: &Jws) -> Result<Verified, String> {
    let subject = self_issued(cfg, jws, jose::now_secs())?;
    if !(subject.starts_with("https://") || subject.starts_with("http://")) {
        return Err("a controlled identifier credential needs an https subject".into());
    }
    let kid = jws
        .kid()
        .filter(|k| !k.is_empty())
        .ok_or("a CID credential names its verification method in kid, and this one has none")?;
    let (_, body) = fetch(
        cfg,
        http,
        &subject,
        "application/cid+json, application/ld+json, application/json",
    )
    .await?;
    let doc: Value = serde_json::from_slice(&body)
        .map_err(|_| format!("{subject} is not a JSON controlled identifier document"))?;
    let key = cid_method_key(&doc, &subject, kid, jose::now_secs())?;
    key.verify(jws)?;
    Ok(Verified {
        subject: subject.clone(),
        client: subject,
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

/// An xsd:dateTime / RFC 3339 timestamp (`YYYY-MM-DDThh:mm:ss[.frac](Z|±hh:mm)`; no zone is UTC)
/// as seconds since the epoch.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't'])?;
    // Every field must be ASCII digits of a bounded width (so no sign, no overflow on huge years,
    // and nothing for the arithmetic below to wrap on).
    let field = |f: &str, min: usize, max: usize| -> Option<i64> {
        if f.len() < min || f.len() > max || !f.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        f.parse().ok()
    };
    let mut d = date.splitn(3, '-');
    let (y, mo, da) = (
        field(d.next()?, 4, 4)?,
        field(d.next()?, 2, 2)?,
        field(d.next()?, 2, 2)?,
    );
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(zone_at);
    let mut time = time.splitn(2, '.');
    let (time, frac) = (time.next()?, time.next());
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut t = time.splitn(3, ':');
    let (h, mi, se) = (
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 24 || mi > 59 || se > 60 {
        return None;
    }
    let offset = match zone {
        "" | "Z" | "z" => 0,
        z => {
            let sign = match z.as_bytes().first() {
                Some(b'-') => -1,
                Some(b'+') => 1,
                _ => return None,
            };
            let (oh, om) = z.get(1..)?.split_once(':')?;
            let (oh, om) = (field(oh, 2, 2)?, field(om, 2, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

// ---------------------------------------------------------------- fetching

pub use super::is_forbidden_ip;

/// The scheme and host checks of the fetch policy, without name resolution.
pub fn check_url_static(cfg: &LwsConfig, raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw).map_err(|_| format!("not a URL: {raw}"))?;
    match u.scheme() {
        "https" => {}
        "http" if cfg.allow_insecure_fetch => {}
        _ => {
            return Err(format!(
                "refusing to fetch {raw}: only https URLs are dereferenced"
            ))
        }
    }
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    match u.host() {
        Some(url::Host::Ipv4(ip)) if is_forbidden_ip(IpAddr::V4(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Ipv6(ip)) if is_forbidden_ip(IpAddr::V6(ip)) => {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        Some(url::Host::Domain(d))
            if d.eq_ignore_ascii_case("localhost")
                || d.to_ascii_lowercase().ends_with(".localhost") =>
        {
            Err(format!("refusing to fetch {raw}: non-public address"))
        }
        None => Err(format!("not a URL with a host: {raw}")),
        _ => Ok(u),
    }
}

/// The full fetch policy: the static checks, then every address the host resolves to.
async fn check_url(cfg: &LwsConfig, raw: &str) -> Result<url::Url, String> {
    let u = check_url_static(cfg, raw)?;
    if cfg.allow_insecure_fetch {
        return Ok(u);
    }
    if let Some(url::Host::Domain(d)) = u.host() {
        let port = u.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host((d, port))
            .await
            .map_err(|_| format!("cannot resolve {d}"))?;
        let mut any = false;
        for a in addrs {
            any = true;
            if is_forbidden_ip(a.ip()) {
                return Err(format!(
                    "refusing to fetch {raw}: {d} resolves to a non-public address"
                ));
            }
        }
        if !any {
            return Err(format!("cannot resolve {d}"));
        }
    }
    Ok(u)
}

/// Redirects a fetch follows, each hop checked against the fetch policy before it is requested.
pub const MAX_REDIRECTS: usize = 3;

/// Where a redirect from `from` with `location` goes, resolved against `from`.
pub fn redirect_target(from: &url::Url, location: Option<&str>) -> Result<url::Url, String> {
    let location = location.ok_or_else(|| format!("{from} redirects without a Location"))?;
    from.join(location)
        .map_err(|_| format!("{from} redirects to an invalid location"))
}

/// GET `url` under the fetch policy: a 200 answer's media type and body (at most 1 MiB).
///
/// `http` must be built by [`super::fetch_client`]: it follows no redirects itself, so each hop is
/// checked (scheme, host and resolved addresses) before it is requested, and its resolver only
/// connects to public addresses, so a name cannot be re-pointed between check and connect.
pub async fn fetch(
    cfg: &LwsConfig,
    http: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<(String, Vec<u8>), String> {
    let mut u = check_url(cfg, url).await?;
    u.set_fragment(None);
    let mut hops = 0;
    let mut resp = loop {
        let resp = http
            .get(u.clone())
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(|e| format!("cannot dereference {url}: {}", e.without_url()))?;
        if !resp.status().is_redirection() {
            break resp;
        }
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(format!("{url} redirects too many times"));
        }
        let next = redirect_target(
            &u,
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )?;
        u = check_url(cfg, next.as_str()).await?;
        u.set_fragment(None);
    };
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("GET {url} answered {}", resp.status().as_u16()));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if resp.content_length().is_some_and(|n| n as usize > MAX_DOC) {
        return Err(format!("{url} is too large"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|_| format!("cannot read {url}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DOC {
            return Err(format!("{url} is too large"));
        }
    }
    Ok((content_type, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    fn cfg() -> LwsConfig {
        LwsConfig::new("http://localhost:3000")
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
    fn redirect_hops_resolve_against_the_current_url() {
        let from = url::Url::parse("https://id.example/a/b").unwrap();
        assert_eq!(
            redirect_target(&from, Some("/c")).unwrap().as_str(),
            "https://id.example/c"
        );
        assert_eq!(
            redirect_target(&from, Some("http://127.0.0.1/x"))
                .unwrap()
                .as_str(),
            "http://127.0.0.1/x"
        );
        assert!(redirect_target(&from, None).is_err());
        // Each hop then faces the policy: a redirect into loopback is refused before it is
        // requested.
        let cfg = cfg();
        assert!(check_url_static(&cfg, "http://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/x").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/x").is_err());
    }

    /// A redirect to a loopback address is never requested: the fetch client follows no
    /// redirect, and the hop fails the policy check before any request goes out.
    #[tokio::test]
    async fn fetch_refuses_a_redirect_into_loopback() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // The internal target: counts the requests that reach it.
        let internal = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal_addr = internal.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = internal.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        });
        // The "attacker's" server redirects there.
        let outer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let outer_addr = outer.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = outer.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nlocation: http://{internal_addr}/secret\r\ncontent-length: 0\r\n\r\n"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        // The first hop is let through (as the insecure escape hatch would), but the policy is the
        // secure one for every hop after: model that by checking the redirect against a secure
        // configuration with the client the insecure one builds.
        let mut insecure = cfg();
        insecure.allow_insecure_fetch = true;
        let client = super::super::fetch_client(&insecure).unwrap();
        let resp = client
            .get(format!("http://{outer_addr}/me"))
            .send()
            .await
            .unwrap();
        // The client does not follow the redirect itself.
        assert_eq!(resp.status().as_u16(), 302);
        let next = redirect_target(
            &url::Url::parse(&format!("http://{outer_addr}/me")).unwrap(),
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
        )
        .unwrap();
        assert!(check_url(&cfg(), next.as_str()).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // And a secure fetch of a non-https or loopback URL never leaves the process.
        assert!(
            fetch(&cfg(), &client, &format!("http://{outer_addr}/me"), "*/*")
                .await
                .is_err()
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_resolver_keeps_public_addresses_only() {
        let addrs: Vec<std::net::SocketAddr> = [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:80",
            "[::1]:443",
            "93.184.216.34:443",
        ]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
        assert_eq!(
            super::super::public_addrs(addrs),
            vec!["93.184.216.34:443".parse::<std::net::SocketAddr>().unwrap()]
        );
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
            did_key(&cfg, &jws).unwrap(),
            Verified {
                subject: did.clone(),
                client: did.clone()
            }
        );
        let jws = sign(&key, &format!("{did}#{mb}"), claims(&did, &cfg));
        assert!(did_key(&cfg, &jws).is_ok());
        // Wrong kid, wrong key.
        assert!(did_key(&cfg, &sign(&key, "other", claims(&did, &cfg))).is_err());
        let other = jose::EcKey::generate("y");
        assert!(did_key(&cfg, &sign(&other, &mb, claims(&did, &cfg))).is_err());
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
            self_issued(&cfg, &sign(&key, "k", c), now)
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

    #[test]
    fn alg_none_is_refused() {
        let t = format!(
            "{}.{}.",
            jose::b64url(br#"{"alg":"none"}"#),
            jose::b64url(br#"{"sub":"a"}"#)
        );
        assert!(parse(&t).is_err());
        assert!(parse("not.a").is_err());
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

    #[test]
    fn datetimes() {
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_datetime("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_datetime("2000-03-01T01:00:00.123+01:00"),
            Some(951_868_800)
        );
        assert_eq!(
            parse_datetime("2000-02-29T23:00:00-01:00"),
            Some(951_868_800)
        );
        assert_eq!(parse_datetime("yesterday"), None);
        // Malformed (non-ASCII, signed, oversized) fields are `None`, never a panic or overflow.
        for bad in [
            "2000-03-01T00:00:00+0\u{e9}:00",
            "2000-03-01T00:00:00Zjunk",
            "2000-03-01T00:00:00.Z",
            "2000-+3-01T00:00:00Z",
            "99999999999999999999-03-01T00:00:00Z",
            "9223372036854775807-03-01T00:00:00Z",
            "2000-03-01T00:00:00+05",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn fetch_policy() {
        let mut cfg = cfg();
        assert!(check_url_static(&cfg, "http://example.org/").is_err());
        assert!(check_url_static(&cfg, "https://127.0.0.1/").is_err());
        assert!(check_url_static(&cfg, "https://[::1]/").is_err());
        assert!(check_url_static(&cfg, "https://10.1.2.3/").is_err());
        assert!(check_url_static(&cfg, "https://169.254.169.254/").is_err());
        assert!(check_url_static(&cfg, "https://localhost/").is_err());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
        assert!(check_url_static(&cfg, "https://example.org/").is_ok());
        assert!(check_url_static(&cfg, "https://8.8.8.8/").is_ok());
        cfg.allow_insecure_fetch = true;
        assert!(check_url_static(&cfg, "http://localhost:3918/agents/alice").is_ok());
        assert!(check_url_static(&cfg, "file:///etc/passwd").is_err());
    }
}

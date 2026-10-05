//! LWS access tokens (RFC 9068): minted by this server's authorization server, validated on every
//! storage request (LWS 1.0 core section 5.2.4).
//!
//! A token is accepted only when it is an ES256 JWS signed by the current authorization server key,
//! typed `at+jwt` (or plain `JWT`), issued by this server, for this storage alone (`aud` names the
//! realm and nothing else), unexpired, already valid (`nbf`), not issued in the future (`iat`), and
//! names a subject. Anything else is `invalid_token`.

use axum::http::header;
use serde_json::{json, Map, Value};

use super::jose::{self, Jws};
use super::{Agent, LwsConfig, LwsRequest, LwsState};
use crate::store::Store;

/// Allowed clock skew, in seconds, when checking `exp`, `nbf` and `iat`.
pub const LEEWAY_SECS: i64 = 30;

/// The agent the request authenticates as. No `Authorization` header (or a non-Bearer one) is
/// anonymous; a Bearer token that does not validate is an error, answered with
/// `error="invalid_token"`. In open mode nobody is authenticated and nothing is refused.
pub fn authenticate<S: Store>(state: &LwsState<S>, req: &LwsRequest) -> Result<Agent, &'static str> {
    let Some(value) = req.header(header::AUTHORIZATION) else {
        return Ok(Agent::anonymous());
    };
    let Some(token) = bearer(value) else {
        return Ok(Agent::anonymous());
    };
    match validate(&state.cfg, token) {
        Ok(agent) => Ok(agent),
        Err(_) if state.cfg.open => Ok(Agent::anonymous()),
        Err(_) => Err("invalid_token"),
    }
}

/// The token of a `Bearer` authorization value.
pub fn bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Validate an access token; the reason it fails otherwise.
pub fn validate(cfg: &LwsConfig, token: &str) -> Result<Agent, String> {
    let jws = Jws::parse(token).ok_or("not a JWS")?;
    match jws.typ() {
        Some(t) if t.eq_ignore_ascii_case("at+jwt") || t.eq_ignore_ascii_case("application/at+jwt") => {}
        Some(t) if t.eq_ignore_ascii_case("jwt") => {}
        None => {}
        Some(_) => return Err("wrong typ".into()),
    }
    if jws.alg() != Some("ES256") {
        return Err("unsupported alg".into());
    }
    if jws.kid().is_some_and(|k| k != cfg.as_key.kid()) {
        return Err("unknown key".into());
    }
    if !jws.verify_es256(&cfg.as_key.public_key()) {
        return Err("bad signature".into());
    }
    if jws.claim_str("iss") != Some(cfg.issuer()) {
        return Err("wrong issuer".into());
    }
    let aud = jws.audiences();
    if aud.len() != 1 || aud[0] != cfg.realm() {
        return Err("wrong audience".into());
    }
    let now = jose::now_secs();
    match jws.claim_time("exp") {
        Some(exp) if exp + LEEWAY_SECS > now => {}
        _ => return Err("expired".into()),
    }
    if jws.claim_time("nbf").is_some_and(|nbf| nbf > now + LEEWAY_SECS) {
        return Err("not yet valid".into());
    }
    if jws.claim_time("iat").is_some_and(|iat| iat > now + LEEWAY_SECS) {
        return Err("issued in the future".into());
    }
    let subject = jws.claim_str("sub").filter(|s| !s.is_empty()).ok_or("no subject")?.to_string();
    let client = jws.claim_str("client_id").map(str::to_string);
    Ok(Agent { subject: Some(subject), client })
}

/// Mint an access token for `subject` (and `client`) to the storage `resource`.
pub fn mint(cfg: &LwsConfig, subject: &str, client: Option<&str>, resource: &str) -> String {
    let now = jose::now_secs();
    let mut claims = json!({
        "iss": cfg.issuer(),
        "aud": resource,
        "sub": subject,
        "iat": now,
        "nbf": now,
        "exp": now + cfg.token_ttl_secs,
        "jti": jose::random_id(),
    });
    if let Some(c) = client {
        claims["client_id"] = Value::String(c.to_string());
    }
    let mut header = Map::new();
    header.insert("typ".into(), Value::String("at+jwt".into()));
    cfg.as_key.sign_jws(header, &claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LwsConfig {
        LwsConfig::new("http://localhost:3000")
    }

    #[test]
    fn minted_tokens_validate() {
        let cfg = cfg();
        let t = mint(&cfg, "https://alice.example/#me", Some("app"), &cfg.realm());
        let agent = validate(&cfg, &t).unwrap();
        assert_eq!(agent.subject.as_deref(), Some("https://alice.example/#me"));
        assert_eq!(agent.client.as_deref(), Some("app"));
    }

    #[test]
    fn wrong_audience_issuer_or_key_fail() {
        let cfg = cfg();
        assert!(validate(&cfg, &mint(&cfg, "a", None, "http://other/")).is_err());
        let other = LwsConfig::new("http://localhost:3000");
        assert!(validate(&cfg, &mint(&other, "a", None, &cfg.realm())).is_err());
        let mut elsewhere = cfg.clone();
        elsewhere.base_url = "http://elsewhere".into();
        assert!(validate(&cfg, &mint(&elsewhere, "a", None, &cfg.realm())).is_err());
    }

    #[test]
    fn expired_tokens_fail() {
        let mut cfg = cfg();
        cfg.token_ttl_secs = -(LEEWAY_SECS + 5);
        let t = mint(&cfg, "a", None, &cfg.realm());
        assert!(validate(&cfg, &t).is_err());
    }

    #[test]
    fn bearer_scheme() {
        assert_eq!(bearer("Bearer abc"), Some("abc"));
        assert_eq!(bearer("bearer  abc "), Some("abc"));
        assert_eq!(bearer("DPoP abc"), None);
        assert_eq!(bearer("Bearer "), None);
    }
}

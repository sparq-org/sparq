//! LWS access tokens (RFC 9068): minted by this server's authorization server, validated on every
//! storage request (LWS 1.0 core section 5.2.4).
//!
//! A token is accepted only when it is an ES256 JWS signed by the authorization server's current
//! key or, during a key rotation, its previous one (picked by `kid`), typed `at+jwt` (or plain
//! `JWT`), issued by this server, for this storage alone (`aud` names the realm and nothing else),
//! unexpired, already valid (`nbf`), not issued in the future (`iat`), and names a subject that is
//! a URI. Anything else is `invalid_token`. The tokens this server mints always carry a `sub` and a
//! `client_id` that are URIs (section 5.2.3): the token endpoint refuses a subject token whose
//! subject or client is not one.

use axum::http::header;
use serde_json::{json, Map, Value};

use super::jose::{self, Jws};
use super::{is_uri, Agent, LwsConfig, LwsRequest, LwsState};
use crate::store::Store;

/// Allowed clock skew, in seconds, when checking `exp`, `nbf` and `iat`.
pub const LEEWAY_SECS: i64 = 30;

/// The agent the request authenticates as. No `Authorization` header (or a non-Bearer one) is
/// anonymous; a Bearer token that does not validate is an error, answered with
/// `error="invalid_token"`. In open mode nobody is authenticated and nothing is refused.
pub fn authenticate<S: Store>(
    state: &LwsState<S>,
    req: &LwsRequest,
) -> Result<Agent, &'static str> {
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
        Some(t)
            if t.eq_ignore_ascii_case("at+jwt") || t.eq_ignore_ascii_case("application/at+jwt") => {
        }
        Some(t) if t.eq_ignore_ascii_case("jwt") => {}
        None => {}
        Some(_) => return Err("wrong typ".into()),
    }
    if jws.alg() != Some("ES256") {
        return Err("unsupported alg".into());
    }
    // Key rotation: a kid picks the current or the previous key; a token without one may verify
    // against either.
    let keys = cfg.as_verify_keys();
    let candidates: Vec<_> = match jws.kid() {
        Some(kid) => keys.iter().filter(|k| k.kid() == kid).collect(),
        None => keys.iter().collect(),
    };
    if candidates.is_empty() {
        return Err("unknown key".into());
    }
    if !candidates.iter().any(|k| jws.verify_es256(&k.public_key())) {
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
    if jws
        .claim_time("nbf")
        .is_some_and(|nbf| nbf > now + LEEWAY_SECS)
    {
        return Err("not yet valid".into());
    }
    if jws
        .claim_time("iat")
        .is_some_and(|iat| iat > now + LEEWAY_SECS)
    {
        return Err("issued in the future".into());
    }
    // sub MUST be a URI (section 5.2.3). client_id is checked where tokens are made: the
    // validation checks the storage server MUST make (section 5.2.4) do not include it, and a
    // trusted issuer's token is not refused over it.
    let subject = jws
        .claim_str("sub")
        .filter(|s| is_uri(s))
        .ok_or("sub is missing or not a URI")?
        .to_string();
    let client = jws.claim_str("client_id").map(str::to_string);
    Ok(Agent {
        subject: Some(subject),
        client,
    })
}

/// Mint an access token for `subject` and `client` (both URIs) to the storage `resource`.
pub fn mint(cfg: &LwsConfig, subject: &str, client: &str, resource: &str) -> String {
    let now = jose::now_secs();
    let claims = json!({
        "iss": cfg.issuer(),
        "aud": resource,
        "sub": subject,
        "iat": now,
        "nbf": now,
        "exp": now + cfg.token_ttl_secs,
        "jti": jose::random_id(),
        "client_id": client,
    });
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

    const ALICE: &str = "https://alice.example/#me";
    const APP: &str = "https://app.example/id";

    #[test]
    fn minted_tokens_validate() {
        let cfg = cfg();
        let t = mint(&cfg, ALICE, APP, &cfg.realm());
        let agent = validate(&cfg, &t).unwrap();
        assert_eq!(agent.subject.as_deref(), Some(ALICE));
        assert_eq!(agent.client.as_deref(), Some(APP));
    }

    #[test]
    fn the_subject_must_be_a_uri() {
        let cfg = cfg();
        assert!(validate(&cfg, &mint(&cfg, "alice", APP, &cfg.realm())).is_err());
        assert!(validate(&cfg, &mint(&cfg, "", APP, &cfg.realm())).is_err());
    }

    #[test]
    fn wrong_audience_issuer_or_key_fail() {
        let cfg = cfg();
        assert!(validate(&cfg, &mint(&cfg, ALICE, APP, "http://other/")).is_err());
        let other = LwsConfig::new("http://localhost:3000");
        assert!(validate(&cfg, &mint(&other, ALICE, APP, &cfg.realm())).is_err());
        let mut elsewhere = cfg.clone();
        elsewhere.base_url = "http://elsewhere".into();
        assert!(validate(&cfg, &mint(&elsewhere, ALICE, APP, &cfg.realm())).is_err());
    }

    #[test]
    fn tokens_of_the_previous_key_validate_after_a_rotation() {
        let old = cfg();
        let issued_before = mint(&old, ALICE, APP, &old.realm());
        // Rotate: a new current key, the old one kept as the previous key.
        let mut rotated = old.clone();
        rotated.as_key = jose::EcKey::generate("lws-as-2");
        rotated.as_previous_key = Some(jose::VerifyKey::from(&old.as_key));
        assert!(validate(&rotated, &issued_before).is_ok());
        assert!(validate(&rotated, &mint(&rotated, ALICE, APP, &rotated.realm())).is_ok());
        // Once the previous key is dropped its tokens fail.
        rotated.as_previous_key = None;
        assert!(validate(&rotated, &issued_before).is_err());
        // A token naming the previous kid but signed by another key fails.
        let mut forged_cfg = old.clone();
        forged_cfg.as_key = jose::EcKey::generate("lws-as-1");
        let forged = mint(&forged_cfg, ALICE, APP, &old.realm());
        let mut with_previous = rotated.clone();
        with_previous.as_previous_key = Some(jose::VerifyKey::from(&old.as_key));
        assert!(validate(&with_previous, &forged).is_err());
    }

    #[test]
    fn expired_tokens_fail() {
        let mut cfg = cfg();
        cfg.token_ttl_secs = -(LEEWAY_SECS + 5);
        let t = mint(&cfg, ALICE, APP, &cfg.realm());
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

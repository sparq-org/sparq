//! The LWS authorization server (LWS 1.0 core section 5.2): RFC 8414 metadata at
//! `/.well-known/lws-configuration`, the JWKS its access tokens verify against, and an RFC 8693
//! token endpoint that exchanges a subject token from one of the LWS authentication suites for an
//! RFC 9068 access token to this storage.

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use super::subject_tokens;
pub use super::subject_tokens::JWT_TOKEN_TYPE;
use super::{
    is_uri, json_response, method_not_allowed, set, tokens, LwsRequest, LwsState, AS_JWKS_PATH,
    AS_TOKEN_PATH, JSON,
};
use crate::store::Store;

pub const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

pub async fn handle<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest) -> Response {
    if req.path == AS_TOKEN_PATH {
        if req.method != Method::POST {
            return method_not_allowed("POST");
        }
        return token(state, req).await;
    }
    if !matches!(req.method, Method::GET | Method::HEAD) {
        return method_not_allowed("GET, HEAD");
    }
    if req.path == AS_JWKS_PATH {
        let mut r = json_response(
            StatusCode::OK,
            "application/jwk-set+json",
            &jwks(&state.cfg),
        );
        set(r.headers_mut(), header::CACHE_CONTROL, "max-age=300");
        return r;
    }
    json_response(StatusCode::OK, JSON, &metadata(state))
}

/// The JWKS: the current signing key and, during a rotation, the previous one.
pub fn jwks(cfg: &super::LwsConfig) -> Value {
    let keys: Vec<Value> = cfg
        .as_verify_keys()
        .iter()
        .map(super::jose::VerifyKey::public_jwk)
        .collect();
    json!({ "keys": keys })
}

/// RFC 8414 metadata. Token exchange is listed among the grant types because the RFC 8414 default
/// excludes it, and the subject token and identifier types say which suites this server accepts.
pub fn metadata<S: Store>(state: &LwsState<S>) -> Value {
    let cfg = &state.cfg;
    let token_types = vec![JWT_TOKEN_TYPE];
    json!({
        "issuer": cfg.issuer(),
        "token_endpoint": cfg.absolute(AS_TOKEN_PATH),
        "jwks_uri": cfg.absolute(AS_JWKS_PATH),
        "grant_types_supported": [TOKEN_EXCHANGE],
        "subject_token_types_supported": token_types,
        "subject_identifier_types_supported": ["https", "did:key"],
        "token_endpoint_auth_methods_supported": ["none"],
        "response_types_supported": [],
    })
}

/// An RFC 6749 error response.
pub fn oauth_error(error: &str, description: &str) -> Response {
    let mut r = json_response(
        StatusCode::BAD_REQUEST,
        JSON,
        &json!({"error": error, "error_description": description}),
    );
    set(r.headers_mut(), header::CACHE_CONTROL, "no-store");
    r
}

/// The token endpoint: RFC 8693 token exchange of a subject token from one of the LWS
/// authentication suites for an access token to this storage. Every refusal is an RFC 6749 error;
/// a subject token that does not verify is `invalid_request` (RFC 8693 section 2.2.2).
async fn token<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest) -> Response {
    let form = match parse_token_request(req) {
        Ok(f) => f,
        Err((error, description)) => return oauth_error(error, &description),
    };
    if form.resource != state.cfg.realm() {
        return oauth_error(
            "invalid_target",
            &format!("this server issues no tokens for {}", form.resource),
        );
    }
    let verified = match form.subject_token_type.as_str() {
        JWT_TOKEN_TYPE => {
            subject_tokens::verify(
                &state.cfg,
                &state.http,
                &form.subject_token,
                &form.subject_token_type,
            )
            .await
        }
        other => Err(format!("unsupported subject_token_type {other}")),
    };
    let verified = match verified {
        Ok(v) => v,
        Err(reason) => {
            return oauth_error(
                "invalid_request",
                &format!("invalid subject token: {reason}"),
            )
        }
    };
    // The access token's sub and client_id MUST be URIs (section 5.2.3).
    if !is_uri(&verified.subject) || !is_uri(&verified.client) {
        return oauth_error(
            "invalid_request",
            "invalid subject token: the subject and the client must be URIs",
        );
    }
    let token = tokens::mint(
        &state.cfg,
        &verified.subject,
        &verified.client,
        &form.resource,
    );
    let body = json!({
        "access_token": token,
        "issued_token_type": ACCESS_TOKEN_TYPE,
        "token_type": "Bearer",
        "expires_in": state.cfg.token_ttl_secs,
    });
    let mut r = json_response(StatusCode::OK, JSON, &body);
    set(r.headers_mut(), header::CACHE_CONTROL, "no-store");
    set(r.headers_mut(), header::PRAGMA, "no-cache");
    r
}

/// The parameters of a token exchange request.
#[derive(Debug, PartialEq, Eq)]
pub struct TokenRequest {
    pub subject_token: String,
    pub subject_token_type: String,
    pub resource: String,
}

/// Read a token request: form-encoded, the token exchange grant, and `subject_token`,
/// `subject_token_type` and `resource` each given once (RFC 6749 section 3.2: parameters MUST NOT
/// repeat). The error is an RFC 6749 error code and description.
pub fn parse_token_request(req: &LwsRequest) -> Result<TokenRequest, (&'static str, String)> {
    if req.content_type().as_deref() != Some("application/x-www-form-urlencoded") {
        return Err((
            "invalid_request",
            "a token request is application/x-www-form-urlencoded".into(),
        ));
    }
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(&req.body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let one = |name: &str| -> Result<Option<String>, (&'static str, String)> {
        let mut values = pairs
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.trim());
        match (values.next(), values.next()) {
            (_, Some(_)) => Err((
                "invalid_request",
                format!("the {name} parameter is repeated"),
            )),
            (Some(v), None) if !v.is_empty() => Ok(Some(v.to_string())),
            _ => Ok(None),
        }
    };
    match one("grant_type")? {
        Some(g) if g == TOKEN_EXCHANGE => {}
        Some(_) => {
            return Err((
                "unsupported_grant_type",
                "only token exchange is supported".into(),
            ))
        }
        None => return Err(("invalid_request", "grant_type is required".into())),
    }
    let (Some(subject_token), Some(subject_token_type)) =
        (one("subject_token")?, one("subject_token_type")?)
    else {
        return Err((
            "invalid_request",
            "subject_token and subject_token_type are required".into(),
        ));
    };
    let resource =
        one("resource")?.ok_or(("invalid_request", "resource is required".to_string()))?;
    Ok(TokenRequest {
        subject_token,
        subject_token_type,
        resource,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn req(content_type: Option<&str>, body: &str) -> LwsRequest {
        let mut headers = HeaderMap::new();
        if let Some(ct) = content_type {
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(ct).unwrap());
        }
        LwsRequest {
            method: Method::POST,
            path: AS_TOKEN_PATH.into(),
            query: None,
            headers,
            body: body.as_bytes().to_vec().into(),
            admission: None,
        }
    }

    const FORM: Option<&str> = Some("application/x-www-form-urlencoded; charset=UTF-8");
    const GRANT: &str = "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange";

    #[test]
    fn token_requests() {
        let ok = format!("{GRANT}&subject_token=t&subject_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Ajwt&resource=http%3A%2F%2Fh%2F");
        let parsed = parse_token_request(&req(FORM, &ok)).unwrap();
        assert_eq!(parsed.subject_token, "t");
        assert_eq!(parsed.subject_token_type, JWT_TOKEN_TYPE);
        assert_eq!(parsed.resource, "http://h/");
        let code = |ct, body: &str| parse_token_request(&req(ct, body)).unwrap_err().0;
        assert_eq!(code(Some("application/json"), &ok), "invalid_request");
        assert_eq!(code(None, &ok), "invalid_request");
        assert_eq!(
            code(FORM, "grant_type=password&subject_token=t"),
            "unsupported_grant_type"
        );
        assert_eq!(code(FORM, "subject_token=t"), "invalid_request");
        assert_eq!(
            code(FORM, &format!("{GRANT}&subject_token_type=x&resource=r")),
            "invalid_request"
        );
        assert_eq!(
            code(
                FORM,
                &format!("{GRANT}&subject_token=t&subject_token_type=x")
            ),
            "invalid_request"
        );
        assert_eq!(
            code(FORM, &format!("{ok}&resource=again")),
            "invalid_request"
        );
    }
}

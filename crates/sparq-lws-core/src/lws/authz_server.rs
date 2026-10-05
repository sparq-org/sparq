//! The LWS authorization server (LWS 1.0 core section 5.2): RFC 8414 metadata at
//! `/.well-known/lws-configuration`, the JWKS its access tokens verify against, and an RFC 8693
//! token endpoint that exchanges a subject token from one of the LWS authentication suites for an
//! RFC 9068 access token to this storage.

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use super::{json_response, method_not_allowed, set, LwsRequest, LwsState, AS_JWKS_PATH, AS_TOKEN_PATH, JSON};
use crate::store::Store;

pub const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
pub const JWT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";
pub const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";
pub const SAML2_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:saml2";

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
        let mut r = json_response(StatusCode::OK, "application/jwk-set+json", &json!({"keys": [state.cfg.as_key.public_jwk()]}));
        set(r.headers_mut(), header::CACHE_CONTROL, "max-age=300");
        return r;
    }
    json_response(StatusCode::OK, JSON, &metadata(state))
}

/// RFC 8414 metadata. Token exchange is listed among the grant types because the RFC 8414 default
/// excludes it, and the subject token and identifier types say which suites this server accepts.
pub fn metadata<S: Store>(state: &LwsState<S>) -> Value {
    let cfg = &state.cfg;
    let mut token_types = vec![JWT_TOKEN_TYPE, ID_TOKEN_TYPE];
    if !cfg.saml_idps.is_empty() {
        token_types.push(SAML2_TOKEN_TYPE);
    }
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
    let mut r = json_response(StatusCode::BAD_REQUEST, JSON, &json!({"error": error, "error_description": description}));
    set(r.headers_mut(), header::CACHE_CONTROL, "no-store");
    r
}

async fn token<S: Store + 'static>(_state: &LwsState<S>, _req: &LwsRequest) -> Response {
    oauth_error("unsupported_grant_type", "token exchange is not implemented yet")
}

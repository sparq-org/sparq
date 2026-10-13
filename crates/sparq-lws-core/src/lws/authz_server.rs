//! The LWS authorization server (LWS 1.0 core section 5.2): RFC 8414 metadata at
//! `/.well-known/lws-configuration`, the JWKS its access tokens verify against, and an RFC 8693
//! token endpoint that exchanges a subject token from one of the LWS authentication suites for an
//! RFC 9068 access token to this storage.

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use super::{
    is_uri, json_response, method_not_allowed, set, tokens, LwsRequest, LwsState, AS_JWKS_PATH,
    AS_TOKEN_PATH, JSON,
};
use crate::store::Store;
pub use sparq_lws_authn::{ID_TOKEN_TYPE, JWT_TOKEN_TYPE};

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
    let mut token_types: Vec<&str> = Vec::new();
    let mut identifier_types = Vec::new();
    for suite in cfg.suites().iter() {
        token_types.extend(suite.token_types());
        for t in suite.identifier_types() {
            if !identifier_types.contains(t) {
                identifier_types.push(t);
            }
        }
    }
    json!({
        "issuer": cfg.issuer(),
        "token_endpoint": cfg.absolute(AS_TOKEN_PATH),
        "jwks_uri": cfg.absolute(AS_JWKS_PATH),
        "grant_types_supported": [TOKEN_EXCHANGE],
        "subject_token_types_supported": token_types,
        "subject_identifier_types_supported": identifier_types,
        "token_endpoint_auth_methods_supported": ["none"],
        "response_types_supported": [],
    })
}

/// The request's `DPoP` header, when it carries exactly one (RFC 9449 section 4.3).
fn dpop_header(req: &LwsRequest) -> Option<&str> {
    (req.headers.get_all("dpop").iter().count() == 1)
        .then(|| req.header("dpop"))
        .flatten()
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
    let token_type = form.subject_token_type.as_str();
    let verified = match state.cfg.suites().find(token_type) {
        Some(suite) => {
            suite
                .verify(&sparq_lws_authn::Exchange {
                    subject_token: &form.subject_token,
                    subject_token_type: token_type,
                    authorization_server: state.cfg.issuer(),
                    token_endpoint: &state.cfg.absolute(AS_TOKEN_PATH),
                    dpop: dpop_header(req),
                    now: super::jose::now_secs(),
                    fetch: state.cfg.fetch_policy(),
                    http: &state.http,
                })
                .await
        }
        None => Err(format!("unsupported subject_token_type {token_type}")),
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
    // Checked again now, in one place for every suite: a suite that fetched documents took time,
    // and the subject token may have expired since it began.
    if !verified.still_valid(super::jose::now_secs()) {
        return oauth_error(
            "invalid_request",
            "invalid subject token: the credential expired while it was checked",
        );
    }
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

    /// A suite in its own package, as the server sees one: it takes one made-up token type and
    /// records what it was asked.
    struct Fake(std::sync::Mutex<Vec<(String, String, Option<String>)>>);

    #[async_trait::async_trait]
    impl sparq_lws_authn::Suite for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn token_types(&self) -> &[&'static str] {
            &["urn:example:fake"]
        }

        fn identifier_types(&self) -> &[&'static str] {
            &["urn"]
        }

        async fn verify(
            &self,
            x: &sparq_lws_authn::Exchange<'_>,
        ) -> Result<sparq_lws_authn::Verified, String> {
            self.0.lock().unwrap().push((
                x.subject_token.to_string(),
                x.authorization_server.to_string(),
                x.dpop.map(str::to_string),
            ));
            match x.subject_token {
                "good" => Ok(sparq_lws_authn::Verified {
                    subject: "https://alice.example/#me".into(),
                    client: "https://app.example/".into(),
                    expires: x.now + 300,
                }),
                "no-uri" => Ok(sparq_lws_authn::Verified {
                    subject: "alice".into(),
                    client: "https://app.example/".into(),
                    expires: x.now + 300,
                }),
                // Valid when the exchange began, expired (past the skew) by the time it ended.
                "stale" => Ok(sparq_lws_authn::Verified {
                    subject: "https://alice.example/#me".into(),
                    client: "https://app.example/".into(),
                    expires: x.now - sparq_lws_authn::SKEW_SECS,
                }),
                _ => Err("not good".into()),
            }
        }
    }

    /// Review finding: a subject token's expiry was checked before the suite fetched documents,
    /// so one that expired during a slow fetch still got an access token. A controlled identifier
    /// token valid (within the skew) when the exchange begins, whose document takes longer to
    /// arrive than it has left, is refused; the same token with a prompt document is not.
    /// Deterministic under load: the prompt token has a minute left, and the slow document is
    /// served only once its token's time is up, however long the exchange took to reach it.
    #[cfg(feature = "authn-ssi")]
    #[tokio::test]
    async fn a_token_that_expires_during_its_fetch_is_refused() {
        use super::super::jose::{self, Jws};
        use super::super::test_store;
        let key = jose::EcKey::generate("k");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let doc = |subject: &str| {
            json!({"id": subject, "authentication": [{"id": format!("{subject}#k"),
                "type": "JsonWebKey", "controller": subject,
                "publicKeyJwk": jose::public_jwk_of(&key.public_key())}]})
        };
        let (prompt, slow) = (doc(&format!("{base}/prompt")), doc(&format!("{base}/slow")));
        // When the slow token's time is up: set as it is minted.
        let slow_until = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(i64::MAX));
        let until = slow_until.clone();
        let app = axum::Router::new()
            .route(
                "/prompt",
                axum::routing::get(move || async move { axum::Json(prompt) }),
            )
            .route(
                "/slow",
                axum::routing::get(move || async move {
                    while jose::now_secs() < until.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    axum::Json(slow)
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.allow_insecure_fetch = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let resource =
            url::form_urlencoded::byte_serialize(state.cfg.realm().as_bytes()).collect::<String>();
        // A token valid (within the skew) for `left` more seconds: expired, but not by more than
        // the skew allows, until then.
        let exchange = |path: &str, left: i64| {
            let subject = format!("{base}{path}");
            let now = jose::now_secs();
            if path == "/slow" {
                slow_until.store(now + left, std::sync::atomic::Ordering::SeqCst);
            }
            let claims = json!({"sub": subject, "iss": subject, "client_id": subject,
                "aud": [state.cfg.issuer()], "iat": now - 120,
                "exp": now - sparq_lws_authn::SKEW_SECS + left});
            let header = json!({"alg": "ES256", "typ": "JWT", "kid": "k"});
            let input = format!(
                "{}.{}",
                jose::b64url(header.to_string().as_bytes()),
                jose::b64url(claims.to_string().as_bytes())
            );
            let token = format!("{input}.{}", jose::b64url(&key.sign(input.as_bytes())));
            assert!(Jws::parse(&token).is_some());
            let jwt =
                url::form_urlencoded::byte_serialize(JWT_TOKEN_TYPE.as_bytes()).collect::<String>();
            test_store::request(
                Method::POST,
                AS_TOKEN_PATH,
                &[("content-type", "application/x-www-form-urlencoded")],
                &format!(
                    "{GRANT}&subject_token={token}&subject_token_type={jwt}&resource={resource}"
                ),
            )
        };
        let ok = handle(&state, &exchange("/prompt", 60)).await;
        assert_eq!(ok.status(), StatusCode::OK);
        // Long enough for the exchange to begin and check it; the document then waits it out.
        let late = handle(&state, &exchange("/slow", 5)).await;
        assert_eq!(late.status(), StatusCode::BAD_REQUEST);
        let body = test_store::body_json(late).await;
        assert!(
            body["error_description"]
                .as_str()
                .unwrap()
                .contains("expired while it was checked"),
            "{body}"
        );
    }

    /// Review finding: a SAML-only server advertised no subject identifier types. The metadata
    /// lists exactly what the offered suites take: nothing for none, SAML's for SAML alone.
    #[tokio::test]
    async fn the_metadata_lists_what_the_offered_suites_take() {
        use super::super::test_store;
        let meta_of = |cfg| async {
            let state = LwsState::new(test_store::FlakyStore::new(), cfg)
                .await
                .unwrap();
            metadata(&state)
        };
        let none =
            meta_of(super::super::LwsConfig::with_suites("http://localhost:3000", []).unwrap())
                .await;
        assert_eq!(none["subject_token_types_supported"], json!([]));
        assert_eq!(none["subject_identifier_types_supported"], json!([]));
        #[cfg(feature = "authn-saml")]
        {
            let idp = super::super::jose::EcKey::generate("idp")
                .public_jwk()
                .to_string();
            let saml =
                sparq_lws_authn_saml::Saml::new([("https://idp.example".to_string(), idp)].into())
                    .unwrap();
            let only = meta_of(
                super::super::LwsConfig::with_suites(
                    "http://localhost:3000",
                    [std::sync::Arc::new(saml) as std::sync::Arc<dyn sparq_lws_authn::Suite>],
                )
                .unwrap(),
            )
            .await;
            assert_eq!(
                only["subject_token_types_supported"],
                json!([sparq_lws_authn_saml::SAML2_TOKEN_TYPE])
            );
            assert_eq!(
                only["subject_identifier_types_supported"],
                json!(["https", "http"])
            );
        }
    }

    /// A registered suite is offered in the metadata and checks the tokens of its type; the
    /// server still issues the token, and still requires URIs of it.
    #[tokio::test]
    async fn the_token_endpoint_offers_registered_suites() {
        use super::super::test_store;
        let fake = std::sync::Arc::new(Fake(Default::default()));
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.register_suite(fake.clone()).unwrap();
        // A token type an offered suite takes (with the default features, the self-issued JWT)
        // is never another's.
        struct Jwt;
        #[async_trait::async_trait]
        impl sparq_lws_authn::Suite for Jwt {
            fn name(&self) -> &'static str {
                "jwt"
            }
            fn token_types(&self) -> &[&'static str] {
                &[JWT_TOKEN_TYPE]
            }
            async fn verify(
                &self,
                _: &sparq_lws_authn::Exchange<'_>,
            ) -> Result<sparq_lws_authn::Verified, String> {
                unreachable!()
            }
        }
        assert_eq!(
            cfg.register_suite(std::sync::Arc::new(Jwt)).is_err(),
            cfg!(feature = "authn-ssi")
        );
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let meta = metadata(&state);
        let types = meta["subject_token_types_supported"].as_array().unwrap();
        assert!(types.iter().any(|t| t == "urn:example:fake"));
        // Offered by the self-issued suite, or by the test's own JWT suite without it.
        assert!(types.iter().any(|t| t == JWT_TOKEN_TYPE));
        let ids = meta["subject_identifier_types_supported"]
            .as_array()
            .unwrap();
        assert!(ids.iter().any(|t| t == "urn"));
        if cfg!(feature = "authn-ssi") {
            assert!(ids.iter().any(|t| t == "did:key"));
        }

        let resource =
            url::form_urlencoded::byte_serialize(state.cfg.realm().as_bytes()).collect::<String>();
        let exchange = |token: &str, token_type: &str| {
            let body = format!(
                "{GRANT}&subject_token={token}&subject_token_type={}&resource={resource}",
                url::form_urlencoded::byte_serialize(token_type.as_bytes()).collect::<String>()
            );
            test_store::request(
                Method::POST,
                AS_TOKEN_PATH,
                &[
                    ("content-type", "application/x-www-form-urlencoded"),
                    ("dpop", "proof"),
                ],
                &body,
            )
        };
        let ok = handle(&state, &exchange("good", "urn:example:fake")).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let token = test_store::body_json(ok).await["access_token"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = tokens::validate(&state.cfg, &token).unwrap();
        assert_eq!(agent.subject.as_deref(), Some("https://alice.example/#me"));
        assert_eq!(agent.client.as_deref(), Some("https://app.example/"));
        assert_eq!(
            fake.0.lock().unwrap()[0],
            (
                "good".to_string(),
                state.cfg.issuer().to_string(),
                Some("proof".to_string())
            )
        );
        for (token, token_type) in [
            ("bad", "urn:example:fake"),
            ("no-uri", "urn:example:fake"),
            ("stale", "urn:example:fake"),
            ("good", "urn:example:unknown"),
        ] {
            let r = handle(&state, &exchange(token, token_type)).await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{token} {token_type}");
            assert_eq!(test_store::body_json(r).await["error"], "invalid_request");
        }
        // A suite never takes a type another one takes.
        let mut suites = sparq_lws_authn::Suites::default();
        suites.register(fake.clone()).unwrap();
        assert!(suites.register(fake).is_err());
    }
}

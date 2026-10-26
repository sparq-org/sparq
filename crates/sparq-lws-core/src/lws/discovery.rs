//! RFC 9728 protected resource metadata for the LWS storage root.

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use serde_json::json;
use url::Url;

use super::{
    etag_of, json_response, method_not_allowed, problem, resources, set, LwsRequest, LwsState, JSON,
};
use crate::store::Store;

/// Insert the RFC 9728 well-known path before the resource path, omitting its query.
/// Fragments are forbidden in resource identifiers (RFC 9728 section 2).
pub(super) fn prm_url(resource: &str) -> Result<Url, String> {
    let mut url = Url::parse(resource).map_err(|e| e.to_string())?;
    if url.fragment().is_some()
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
    {
        return Err("a resource must be an HTTP(S) URL without a fragment".into());
    }
    let path = format!(
        "/.well-known/oauth-protected-resource{}",
        url.path().trim_end_matches('/')
    );
    url.set_path(&path);
    url.set_query(None);
    Ok(url)
}

pub(super) fn handle<S: Store>(state: &LwsState<S>, req: &LwsRequest) -> Response {
    if !matches!(req.method, Method::GET | Method::HEAD) {
        return method_not_allowed("GET, HEAD");
    }
    // Storage requests accept bearer presentation only. The OpenID subject-token exchange's
    // DPoP proof does not make this resource server a DPoP-bound access-token verifier.
    let doc = json!({
        "resource": state.cfg.storage(),
        "authorization_servers": [state.cfg.issuer()],
        "jlws_storage_description": state.cfg.storage(),
        "bearer_methods_supported": ["header"],
    });
    let etag = etag_of([doc.to_string().as_str()]);
    let mut response = match resources::read_refusal(req, &etag) {
        Some(status) => problem(status, None),
        None => json_response(StatusCode::OK, JSON, &doc),
    };
    set(response.headers_mut(), header::ETAG, &etag);
    // Public discovery is identical for every caller; revalidate after this short freshness window.
    set(
        response.headers_mut(),
        header::CACHE_CONTROL,
        "public, max-age=300",
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_metadata_path_insertion() {
        for (resource, expected) in [
            (
                "https://h/",
                "https://h/.well-known/oauth-protected-resource",
            ),
            (
                "https://h/alice",
                "https://h/.well-known/oauth-protected-resource/alice",
            ),
            (
                "https://h/alice/",
                "https://h/.well-known/oauth-protected-resource/alice",
            ),
            (
                "https://h/alice/?view=private",
                "https://h/.well-known/oauth-protected-resource/alice",
            ),
            (
                "https://h/a%20b/",
                "https://h/.well-known/oauth-protected-resource/a%20b",
            ),
        ] {
            assert_eq!(prm_url(resource).unwrap().as_str(), expected);
        }
        for resource in [
            "https://h/alice#fragment",
            "https://h/#",
            "not a URL",
            "urn:alice",
        ] {
            assert!(prm_url(resource).is_err(), "{resource}");
        }
    }
    use super::super::{parse_links, router, test_store::FlakyStore, LwsConfig, LWS_CID, LWS_NS};
    use axum::{body::Body, http::Request, Router};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use tower::ServiceExt;

    const BASE: &str = "https://storage.example";
    const STORAGE: &str = "https://storage.example/";
    const PRM: &str = "https://storage.example/.well-known/oauth-protected-resource";

    async fn app() -> Router {
        let store = FlakyStore::new();
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        router(store, cfg).await.unwrap()
    }

    async fn call(
        app: &Router,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        let mut req = Request::builder().method(method).uri(url);
        for (key, value) in headers {
            req = req.header(*key, *value);
        }
        app.clone()
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }

    async fn document(response: Response) -> Value {
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// RFC 9110 auth-param grammar: tokens, BWS, quoted strings, and quoted-pair escapes.
    /// Commas delimit parameters only outside a quoted string.
    fn challenge_params(value: &str) -> BTreeMap<String, String> {
        use super::super::{quoted_string, skip_ows, token, unescape};
        let (scheme, mut rest) = token(value).unwrap();
        assert_eq!(scheme, "Bearer");
        assert!(rest.starts_with(' '));
        let mut params = BTreeMap::new();
        loop {
            let (name, after_name) = token(skip_ows(rest)).unwrap();
            let value = skip_ows(after_name).strip_prefix('=').unwrap();
            let value = skip_ows(value);
            let (value, tail) = if value.starts_with('"') {
                quoted_string(value).unwrap()
            } else {
                token(value).unwrap()
            };
            assert!(params
                .insert(name.to_ascii_lowercase(), unescape(value).into_owned())
                .is_none());
            rest = skip_ows(tail);
            if rest.is_empty() {
                break;
            }
            rest = rest.strip_prefix(',').unwrap();
        }
        params
    }

    #[test]
    fn auth_params_preserve_quoted_commas_and_escapes() {
        let params = challenge_params(r#"Bearer realm="a,b\"c\\d", error = invalid_token"#);
        assert_eq!(params["realm"], "a,b\"c\\d");
        assert_eq!(params["error"], "invalid_token");
    }

    fn storage_link(response: &Response) -> String {
        response
            .headers()
            .get_all(header::LINK)
            .iter()
            .find_map(|value| {
                parse_links(value.to_str().unwrap()).find_map(|link| {
                    (link.param("rel").as_deref() == Some(format!("{LWS_NS}storage").as_str()))
                        .then(|| link.target.to_owned())
                })
            })
            .expect("storage relation")
    }

    #[tokio::test]
    async fn anonymous_and_rejected_token_challenges_discover_metadata() {
        let app = app().await;
        for path in [
            "/private",
            "/.lws/grants/",
            "/.lws/requests/",
            "/.lws/subscriptions/",
            "/.lws/types/index",
        ] {
            for headers in [vec![], vec![("authorization", "Bearer invalid")]] {
                let response = call(&app, "GET", path, &headers, "").await;
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
                let challenge = response.headers()[header::WWW_AUTHENTICATE]
                    .to_str()
                    .unwrap();
                assert!(challenge.starts_with("Bearer resource_metadata="));
                let params = challenge_params(challenge);
                assert_eq!(params["resource_metadata"], PRM);
                assert_eq!(params["realm"], STORAGE);
                assert_eq!(params["as_uri"], BASE);
                assert_eq!(
                    params.get("error").map(String::as_str),
                    (!headers.is_empty()).then_some("invalid_token")
                );
                assert_eq!(params.len(), if headers.is_empty() { 3 } else { 4 });
                assert_eq!(storage_link(&response), STORAGE);
            }
        }
    }

    #[tokio::test]
    async fn public_metadata_has_exact_members_and_matches_the_issuer_and_description() {
        let app = app().await;
        let response = call(&app, "GET", PRM, &[], "").await;
        assert_eq!(response.headers()[header::CONTENT_TYPE], JSON);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=300"
        );
        let etag = response.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'));
        let prm = document(response).await;
        assert_eq!(
            prm,
            json!({
                "resource": STORAGE,
                "authorization_servers": [BASE],
                "jlws_storage_description": STORAGE,
                "bearer_methods_supported": ["header"],
            })
        );
        let challenge = call(&app, "GET", "/private", &[], "").await;
        assert_eq!(
            prm["resource"],
            challenge_params(
                challenge.headers()[header::WWW_AUTHENTICATE]
                    .to_str()
                    .unwrap()
            )["realm"]
        );
        for path in [
            "/.well-known/lws-configuration",
            "/.well-known/oauth-authorization-server",
        ] {
            let metadata = document(call(&app, "GET", path, &[], "").await).await;
            assert_eq!(prm["authorization_servers"][0], metadata["issuer"]);
        }
        let description = call(
            &app,
            "GET",
            prm["jlws_storage_description"].as_str().unwrap(),
            &[("accept", LWS_CID)],
            "",
        )
        .await;
        assert_eq!(description.headers()[header::CONTENT_TYPE], LWS_CID);
        assert_eq!(document(description).await["id"], STORAGE);
        let response = call(&app, "GET", PRM, &[("if-none-match", &etag)], "").await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers()[header::ETAG], etag);
        assert!(axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty());
        let with_rejected_credentials =
            call(&app, "GET", PRM, &[("authorization", "Bearer invalid")], "").await;
        assert_eq!(with_rejected_credentials.headers()[header::ETAG], etag);
        assert_eq!(document(with_rejected_credentials).await, prm);
        let head = call(&app, "HEAD", PRM, &[], "").await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::ETAG], etag);
        assert!(axum::body::to_bytes(head.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            call(&app, "POST", PRM, &[], "").await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        for path in [
            "/.well-known/oauth-protected-resource/",
            "/.well-known/oauth-protected-resource/alice",
            "/.well-known/oauth-protected-resource-extra",
        ] {
            assert_eq!(
                call(&app, "GET", path, &[], "").await.status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn only_the_computed_metadata_path_is_routed_for_a_storage_with_a_path() {
        let app = router(
            FlakyStore::new(),
            LwsConfig::new("https://storage.example/alice"),
        )
        .await
        .unwrap();
        let prm = document(
            call(
                &app,
                "GET",
                "/.well-known/oauth-protected-resource/alice",
                &[],
                "",
            )
            .await,
        )
        .await;
        assert_eq!(prm["resource"], "https://storage.example/alice/");
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/bob",
            "/.well-known/oauth-protected-resource/alice/",
        ] {
            assert_eq!(
                call(&app, "GET", path, &[], "").await.status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn discovery_documents_support_cors_without_credentials() {
        let app = app().await;
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/lws-configuration",
            "/.well-known/oauth-authorization-server",
        ] {
            let response = call(&app, "GET", path, &[("origin", "https://app.example")], "").await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "https://app.example"
            );
            assert!(response.headers()[header::VARY]
                .to_str()
                .unwrap()
                .contains("Origin"));
            let response = call(
                &app,
                "OPTIONS",
                path,
                &[
                    ("origin", "https://app.example"),
                    ("access-control-request-method", "GET"),
                ],
                "",
            )
            .await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "https://app.example"
            );
        }
    }

    /// SQ-6, using the same local OP discovery/JWKS and controlled-identifier fixture shape as
    /// the authn-openid suite. Every storage/AS request goes through the production router.
    #[cfg(feature = "authn-openid")]
    #[tokio::test]
    async fn openid_client_walk_from_challenge_to_private_resource() {
        use super::super::jose;

        // The suite's provider helper is private to that crate's unit tests.
        let op_key = jose::EcKey::generate("op-key");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let op = format!("http://{}", listener.local_addr().unwrap());
        let subject = format!("{op}/bob");
        let identity =
            json!({"id": subject, "service": [{"type": "OpenIdProvider", "serviceEndpoint": op}]});
        let discovery = json!({"issuer": op, "jwks_uri": format!("{op}/jwks")});
        let jwks = json!({"keys": [op_key.public_jwk()]});
        let provider = Router::new()
            .route(
                "/bob",
                axum::routing::get(move || async move { axum::Json(identity) }),
            )
            .route(
                "/.well-known/openid-configuration",
                axum::routing::get(move || async move { axum::Json(discovery) }),
            )
            .route(
                "/jwks",
                axum::routing::get(move || async move { axum::Json(jwks) }),
            );
        let provider_task =
            tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
        // Abort on unwind as well as success, so a failed walk cannot leave a server running.
        struct ProviderTask(tokio::task::JoinHandle<()>);
        impl Drop for ProviderTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _provider_task = ProviderTask(provider_task);
        let store = FlakyStore::new();
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some(subject.clone());
        cfg.allow_insecure_fetch = true;
        let bootstrap =
            super::super::tokens::mint(&cfg, &subject, "https://app.example/id", STORAGE);
        let app = router(store, cfg).await.unwrap();
        let created = call(
            &app,
            "POST",
            "/",
            &[
                ("authorization", &format!("Bearer {bootstrap}")),
                ("content-type", "text/plain"),
                ("slug", "private"),
            ],
            "private content",
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);

        let request_url = Url::parse("https://storage.example/private").unwrap();
        let response = call(&app, "GET", request_url.as_str(), &[], "").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let params = challenge_params(
            response.headers()[header::WWW_AUTHENTICATE]
                .to_str()
                .unwrap(),
        );
        let storage = storage_link(&response);
        // D4 check 1: same-origin metadata. oneshot never follows redirects or attaches credentials.
        let metadata_url = Url::parse(&params["resource_metadata"]).unwrap();
        assert_eq!(metadata_url.origin(), request_url.origin());
        let prm = document(call(&app, "GET", metadata_url.as_str(), &[], "").await).await;
        // D4 check 2: the brief deliberately uses storage-root containment for a child request.
        let resource = prm["resource"].as_str().unwrap();
        assert_eq!(resource, storage);
        assert_eq!(resource, params["realm"]);
        assert!(resource.ends_with('/'));
        assert!(request_url.as_str().starts_with(resource));
        let description = document(
            call(
                &app,
                "GET",
                prm["jlws_storage_description"].as_str().unwrap(),
                &[("accept", LWS_CID)],
                "",
            )
            .await,
        )
        .await;
        let storage_root = description["service"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["type"] == "StorageRoot")
            .unwrap()["serviceEndpoint"]
            .as_str()
            .unwrap();
        // D4 check 3: the advertised storage root must have the request's origin.
        assert_eq!(
            Url::parse(storage_root).unwrap().origin(),
            request_url.origin()
        );
        assert_eq!(storage_root, resource);
        // RFC 8414 insertion, rather than appending a well-known segment to an issuer path.
        let mut as_metadata_url =
            Url::parse(prm["authorization_servers"][0].as_str().unwrap()).unwrap();
        let path = format!(
            "/.well-known/lws-configuration{}",
            as_metadata_url.path().trim_end_matches('/')
        );
        as_metadata_url.set_path(&path);
        let metadata = document(call(&app, "GET", as_metadata_url.as_str(), &[], "").await).await;
        assert_eq!(metadata["issuer"], prm["authorization_servers"][0]);
        let now = jose::now_secs();
        let id_token = op_key.sign_jws(serde_json::Map::new(), &json!({
            "iss": op, "sub": subject, "aud": metadata["issuer"], "azp": "https://app.example/id",
            "iat": now, "exp": now + 300,
        }));
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", super::super::authz_server::TOKEN_EXCHANGE)
            .append_pair("resource", &storage)
            .append_pair("subject_token", &id_token)
            .append_pair(
                "subject_token_type",
                super::super::authz_server::ID_TOKEN_TYPE,
            )
            .append_pair("client_id", "https://app.example/id")
            .finish();
        let token = document(
            call(
                &app,
                "POST",
                metadata["token_endpoint"].as_str().unwrap(),
                &[("content-type", "application/x-www-form-urlencoded")],
                &form,
            )
            .await,
        )
        .await;
        assert_eq!(token["token_type"], "Bearer");
        let access_token = token["access_token"].as_str().unwrap();
        assert_eq!(
            jose::Jws::parse(access_token).unwrap().claims["aud"],
            resource
        );
        let response = call(
            &app,
            "GET",
            request_url.as_str(),
            &[("authorization", &format!("Bearer {access_token}"))],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
            "private content"
        );
    }
}

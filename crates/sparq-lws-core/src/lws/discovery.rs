//! RFC 9728 protected resource metadata for storage resources and services.

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
pub(super) fn prm_url(resource: &str) -> Result<String, String> {
    let url = Url::parse(resource).map_err(|e| e.to_string())?;
    if url.fragment().is_some()
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
    {
        return Err("a resource must be an HTTP(S) URL without a fragment".into());
    }
    // Read the HTTP path without URL normalization: encoded octets and the trailing slash
    // distinguish resources. Only the origin root has an empty well-known suffix.
    let uri: axum::http::Uri = resource
        .parse()
        .map_err(|e: axum::http::uri::InvalidUri| e.to_string())?;
    let path = if uri.path() == "/" { "" } else { uri.path() };
    Ok(format!(
        "{}/.well-known/oauth-protected-resource{path}",
        url.origin().ascii_serialization()
    ))
}

/// The configured origin and the HTTP path as sent, without a query or path normalization.
pub(super) fn resource_uri(cfg: &super::LwsConfig, path: &str) -> Option<String> {
    let url = Url::parse(&cfg.base_url).ok()?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return None;
    }
    let uri: axum::http::Uri = cfg.base_url.parse().ok()?;
    Some(format!(
        "{}://{}{path}",
        uri.scheme_str()?,
        uri.authority()?
    ))
}

/// Recognize metadata using only the request path and fixed configuration, never stored state.
pub(super) fn resource(cfg: &super::LwsConfig, path: &str) -> Option<String> {
    let suffix = path.strip_prefix("/.well-known/oauth-protected-resource")?;
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return None;
    }
    let path = if suffix.is_empty() { "/" } else { suffix };
    let resource = resource_uri(cfg, path)?;
    let service = path.starts_with(super::SUBSCRIPTIONS_PATH)
        || path.starts_with(super::GRANTS_PATH)
        || path.starts_with(super::REQUESTS_PATH)
        || matches!(path, super::TYPE_INDEX_PATH | super::TYPE_SEARCH_PATH);
    let storage: axum::http::Uri = cfg.storage().parse().ok()?;
    (path.starts_with(storage.path()) || service).then_some(resource)
}

pub(super) fn handle<S: Store>(state: &LwsState<S>, req: &LwsRequest, resource: &str) -> Response {
    if !matches!(req.method, Method::GET | Method::HEAD) {
        return method_not_allowed("GET, HEAD");
    }
    // Storage requests accept bearer presentation only. The OpenID subject-token exchange's
    // DPoP proof does not make this resource server a DPoP-bound access-token verifier.
    let doc = json!({
        "resource": resource,
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
                "https://h/.well-known/oauth-protected-resource/alice/",
            ),
            (
                "https://h/alice/?view=private",
                "https://h/.well-known/oauth-protected-resource/alice/",
            ),
            (
                "https://h/a%20b/",
                "https://h/.well-known/oauth-protected-resource/a%20b/",
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
                assert_eq!(params["resource_metadata"], format!("{PRM}{path}"));
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
        let challenge = call(&app, "GET", "/", &[], "").await;
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
        for path in ["/.well-known/oauth-protected-resource-extra"] {
            assert_eq!(
                call(&app, "GET", path, &[], "").await.status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn touchstone_metadata_paths_are_scoped_to_storage_and_services() {
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
                "/.well-known/oauth-protected-resource/alice/",
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
            "/.well-known/oauth-protected-resource/alice",
            "/.well-known/oauth-protected-resource/alice-other/",
            "/.well-known/oauth-protected-resource/%61lice/",
            "/.well-known/oauth-protected-resource/.lws/unknown",
        ] {
            assert_eq!(
                call(&app, "GET", path, &[], "").await.status(),
                StatusCode::NOT_FOUND
            );
        }
        for (path, resource) in [
            ("/alice/", "https://storage.example/alice/"),
            ("/alice/absent", "https://storage.example/alice/absent"),
            (
                "/.lws/subscriptions/",
                "https://storage.example/.lws/subscriptions/",
            ),
        ] {
            let doc = document(
                call(
                    &app,
                    "GET",
                    &format!("/.well-known/oauth-protected-resource{path}"),
                    &[],
                    "",
                )
                .await,
            )
            .await;
            assert_eq!(doc["resource"], resource);
            assert_eq!(
                doc["jlws_storage_description"],
                "https://storage.example/alice/"
            );
        }
    }

    #[tokio::test]
    async fn touchstone_root_metadata_keeps_the_configured_resource_identifier() {
        let app = router(
            FlakyStore::new(),
            LwsConfig::new("https://STORAGE.example:443"),
        )
        .await
        .unwrap();
        let response = call(&app, "GET", "/", &[], "").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let params = challenge_params(
            response.headers()[header::WWW_AUTHENTICATE]
                .to_str()
                .unwrap(),
        );
        assert_eq!(params["resource_metadata"], PRM);
        let doc = document(call(&app, "GET", PRM, &[], "").await).await;
        assert_eq!(doc["resource"], "https://STORAGE.example:443/");
        assert_eq!(
            doc["jlws_storage_description"],
            "https://STORAGE.example:443/"
        );
    }

    #[tokio::test]
    async fn touchstone_per_resource_metadata_matches_the_exact_requested_uri() {
        let store = FlakyStore::new();
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        let token = super::super::tokens::mint(
            &cfg,
            "https://owner.example/#me",
            "https://app.example/",
            STORAGE,
        );
        let bearer = format!("Bearer {token}");
        let app = router(store.clone(), cfg).await.unwrap();
        for (slug, content, link) in [
            (
                "c",
                "",
                "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
            ),
            ("c", "protected", ""),
        ] {
            let created = call(
                &app,
                "POST",
                "/",
                &[
                    ("authorization", &bearer),
                    ("slug", slug),
                    ("content-type", "text/plain"),
                    ("link", link),
                ],
                content,
            )
            .await;
            assert_eq!(created.status(), StatusCode::CREATED);
        }
        // Touchstone challenges a container and a created text/plain resource separately.
        // Also pin root, query removal, encoded octets and a resource that was never created.
        for (path, metadata_path) in [
            ("/", ""),
            ("/c/", "/c/"),
            ("/c", "/c"),
            ("/c?view=private", "/c"),
            ("/absent", "/absent"),
            ("/a%2fb%2F%20/", "/a%2fb%2F%20/"),
            ("/a/%2e%2e/", "/a/%2e%2e/"),
        ] {
            let response = call(&app, "GET", path, &[("accept", super::super::LWS_JSON)], "").await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(storage_link(&response), STORAGE);
            let params = challenge_params(
                response.headers()[header::WWW_AUTHENTICATE]
                    .to_str()
                    .unwrap(),
            );
            assert_eq!(params["resource_metadata"], format!("{PRM}{metadata_path}"));
            let metadata = call(
                &app,
                "GET",
                &params["resource_metadata"],
                &[("accept", JSON)],
                "",
            )
            .await;
            assert_eq!(metadata.status(), StatusCode::OK);
            let etag = metadata.headers()[header::ETAG].clone();
            assert_eq!(metadata.headers()[header::CONTENT_TYPE], JSON);
            assert_eq!(
                metadata.headers()[header::CACHE_CONTROL],
                "public, max-age=300"
            );
            let expected = json!({
                "resource": format!("{BASE}{}", path.split('?').next().unwrap()),
                "authorization_servers": [BASE],
                "jlws_storage_description": STORAGE,
                "bearer_methods_supported": ["header"],
            });
            assert_eq!(document(metadata).await, expected);
            for authorization in ["Bearer invalid", bearer.as_str()] {
                let response = call(
                    &app,
                    "GET",
                    &params["resource_metadata"],
                    &[("authorization", authorization)],
                    "",
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.headers()[header::ETAG], etag);
                assert_eq!(document(response).await, expected);
            }
            let head = call(&app, "HEAD", &params["resource_metadata"], &[], "").await;
            assert_eq!(head.status(), StatusCode::OK);
            assert_eq!(head.headers()[header::ETAG], etag);
            assert!(axum::body::to_bytes(head.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty());
            let cached = call(
                &app,
                "GET",
                &params["resource_metadata"],
                &[("if-none-match", etag.to_str().unwrap())],
                "",
            )
            .await;
            assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        }
        // Metadata is served without consulting the store, even during a backend outage.
        *store.trace.lock().unwrap() = Some(Vec::new());
        store
            .fail_meta
            .store(true, std::sync::atomic::Ordering::SeqCst);
        store
            .fail_list
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for path in ["/c", "/absent"] {
            let doc = document(call(&app, "GET", &format!("{PRM}{path}"), &[], "").await).await;
            assert_eq!(doc["resource"], format!("{BASE}{path}"));
        }
        assert!(store.trace.lock().unwrap().as_ref().unwrap().is_empty());
    }

    #[tokio::test]
    async fn touchstone_notification_post_challenges_for_the_service_then_accepts_storage_token() {
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        let token = super::super::tokens::mint(
            &cfg,
            "https://owner.example/#me",
            "https://app.example/",
            STORAGE,
        );
        let bearer = format!("Bearer {token}");
        let app = router(FlakyStore::new(), cfg).await.unwrap();
        let description = document(call(&app, "GET", "/", &[("accept", LWS_CID)], "").await).await;
        let service = description["service"]
            .as_array()
            .unwrap()
            .iter()
            .find(|service| service["type"] == "NotificationService")
            .unwrap();
        let endpoint = service["serviceEndpoint"].as_str().unwrap();
        assert_eq!(endpoint, "https://storage.example/.lws/subscriptions/");
        assert_eq!(
            call(
                &app,
                "POST",
                "/",
                &[
                    ("authorization", &bearer),
                    ("slug", "topic"),
                    (
                        "link",
                        "<https://www.w3.org/ns/lws#Container>; rel=\"type\""
                    )
                ],
                ""
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        let body = json!({"@context": ["https://www.w3.org/ns/lws/v1"], "type": "WebhookSubscription", "topic": ["https://storage.example/topic/"], "inbox": "https://inbox.example/in"}).to_string();
        let response = call(
            &app,
            "POST",
            endpoint,
            &[("content-type", super::super::LWS_JSON)],
            &body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(storage_link(&response), STORAGE);
        let params = challenge_params(
            response.headers()[header::WWW_AUTHENTICATE]
                .to_str()
                .unwrap(),
        );
        assert_eq!(
            params["resource_metadata"],
            format!("{PRM}/.lws/subscriptions/")
        );
        let doc = document(
            call(
                &app,
                "GET",
                &params["resource_metadata"],
                &[("accept", JSON)],
                "",
            )
            .await,
        )
        .await;
        assert_eq!(doc["resource"], endpoint);
        assert_eq!(doc["authorization_servers"], json!([BASE]));
        let accepted = call(
            &app,
            "POST",
            endpoint,
            &[
                ("content-type", super::super::LWS_JSON),
                ("authorization", &bearer),
            ],
            &body,
        )
        .await;
        assert!(accepted.status().is_success());
        let bytes = axum::body::to_bytes(accepted.into_body(), usize::MAX)
            .await
            .unwrap();
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        let subscription = doc["subscription"].as_str().unwrap();
        assert!(call(
            &app,
            "DELETE",
            subscription,
            &[("authorization", &bearer)],
            ""
        )
        .await
        .status()
        .is_success());
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
        // RFC 9728 requires the exact resource requested, while the token realm is the storage.
        let resource = prm["resource"].as_str().unwrap();
        assert_eq!(resource, request_url.as_str());
        assert_eq!(storage, params["realm"]);
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
        assert_eq!(storage_root, storage);
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
            storage
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

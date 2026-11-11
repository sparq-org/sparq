//! Public URLs through the real native executable and a test prefix-stripping proxy.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Router;
use common::process::Process;
use serde_json::Value;
use sparq_lws_core::lws::{jose::EcKey, tokens, LwsConfig, LWS_CID, LWS_JSON};
use tower::ServiceExt;

const ORIGIN: &str = "https://storage.example";
const OWNER: &str = "https://owner.example/#me";
const KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/storage-description/test-key.json"
);

struct Server {
    _process: Process,
    app: Router,
    client: reqwest::Client,
    endpoint: String,
    base: String,
    token: String,
}

impl Server {
    async fn start(base: &str, mount: Option<&str>, proxy: bool) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let bind = listener.local_addr().unwrap().to_string();
        drop(listener);
        let endpoint = format!("http://{bind}");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut values = vec![
            ("SOLID_SERVER_BIND", bind.as_str()),
            ("SOLID_SERVER_BASE_URL", base),
            ("SOLID_SERVER_PROTOCOL", "lws"),
            ("SOLID_SERVER_LWS_OWNER", OWNER),
            ("SOLID_SERVER_LWS_AS_KEY_FILE", KEY),
            ("SOLID_SERVER_LWS_NOTIFY_KEY_FILE", KEY),
            ("SOLID_SERVER_RATE_PER_IP", "off"),
        ];
        if let Some(mount) = mount {
            values.push(("SOLID_SERVER_LWS_MOUNT_BASE_PATH", mount));
        }
        let mut process = Process::start(env!("CARGO_BIN_EXE_sparq-lws-core"), &values);
        process.wait_ready(&client, &endpoint).await;
        let mut cfg = LwsConfig::new(base);
        cfg.as_key =
            EcKey::from_jwk(include_str!("fixtures/storage-description/test-key.json")).unwrap();
        let token = tokens::mint(&cfg, OWNER, "https://app.example/", &cfg.realm());

        // The test proxy forwards the request path it receives to the native listener.
        // nest_service strips precisely one prefix and accepts both root slash forms.
        let forward = Router::new().fallback({
            let client = client.clone();
            let endpoint = endpoint.clone();
            move |req: Request| {
                let client = client.clone();
                let endpoint = endpoint.clone();
                async move {
                    let (parts, body) = req.into_parts();
                    let body = to_bytes(body, 1 << 20).await.unwrap();
                    let response = client
                        .request(
                            parts.method,
                            format!("{endpoint}{}", parts.uri.path_and_query().unwrap()),
                        )
                        .headers(parts.headers)
                        .body(body)
                        .send()
                        .await
                        .unwrap();
                    let mut result = Response::builder().status(response.status());
                    *result.headers_mut().unwrap() = response.headers().clone();
                    result
                        .body(Body::from(response.bytes().await.unwrap()))
                        .unwrap()
                }
            }
        });
        let prefix = cfg.base_url.strip_prefix(ORIGIN).unwrap();
        let app = if proxy && !prefix.is_empty() {
            Router::new()
                .nest_service(prefix, forward.clone())
                // Origin-level discovery is forwarded unchanged, outside the storage mount.
                .route_service(
                    "/.well-known/oauth-protected-resource",
                    forward.clone().into_service(),
                )
                .route_service(
                    "/.well-known/oauth-protected-resource/{*path}",
                    forward.into_service(),
                )
        } else {
            forward
        };
        Self {
            _process: process,
            app,
            client,
            endpoint,
            base: cfg.base_url,
            token,
        }
    }

    async fn call(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        let path = url.strip_prefix(ORIGIN).unwrap_or(url);
        let path = if path.is_empty() { "/" } else { path };
        let mut request = Request::builder().method(method).uri(path);
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }
}

async fn json(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

async fn exercise_advertised_urls(server: &Server) {
    let root = format!("{}/", server.base);
    let auth = format!("Bearer {}", server.token);
    let description = json(server.call("GET", &root, &[("accept", LWS_CID)], "").await).await;
    assert_eq!(description["id"], root);
    if server.base != ORIGIN {
        let bare = json(
            server
                .call("GET", &server.base, &[("accept", LWS_CID)], "")
                .await,
        )
        .await;
        assert_eq!(
            bare, description,
            "both root forms select the storage description"
        );
    }
    let subscriptions = description["service"]
        .as_array()
        .unwrap()
        .iter()
        .find(|service| service["type"] == "NotificationService")
        .unwrap()["serviceEndpoint"]
        .as_str()
        .unwrap();
    assert_eq!(
        subscriptions,
        format!("{}/.lws/subscriptions/", server.base)
    );

    // Follow Location from both a resource create and a container create.
    for (slug, container) in [("child", false), ("alice", true)] {
        let mut headers = vec![
            ("authorization", auth.as_str()),
            ("slug", slug),
            ("content-type", "text/plain"),
        ];
        if container {
            headers.push((
                "link",
                "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
            ));
        }
        let response = server
            .call(
                "POST",
                &server.base,
                &headers,
                if container { "" } else { "outer" },
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED, "create {slug}");
        let location = response.headers()[header::LOCATION].to_str().unwrap();
        assert_eq!(
            location,
            format!("{root}{slug}{}", if container { "/" } else { "" })
        );
        let response = server
            .call(
                "GET",
                location,
                &[
                    ("authorization", &auth),
                    ("accept", if container { LWS_JSON } else { "text/plain" }),
                ],
                "",
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        if container {
            assert_eq!(json(response).await["id"], location);
        } else {
            assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "outer");
        }
    }
    // A legitimate child starts with the same segment as the mount. It must survive one strip.
    let nested = format!("{root}alice/child");
    assert_eq!(
        server
            .call(
                "POST",
                &format!("{root}alice/"),
                &[
                    ("authorization", &auth),
                    ("content-type", "text/plain"),
                    ("slug", "child")
                ],
                "inner"
            )
            .await
            .status(),
        StatusCode::CREATED
    );
    let response = server
        .call("GET", &nested, &[("authorization", &auth)], "")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "inner");
    assert_eq!(
        server
            .call(
                "GET",
                &format!("{root}?page=0"),
                &[("authorization", &auth), ("accept", LWS_JSON)],
                ""
            )
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "the query must reach the listing handler"
    );

    for target in [
        subscriptions.to_owned(),
        format!("{root}a%2fb%20/"),
        format!("{root}a//b/?view=private"),
    ] {
        let response = server.call("GET", &target, &[], "").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap();
        let metadata = challenge
            .split("resource_metadata=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let resource = target.split('?').next().unwrap();
        let suffix = resource.strip_prefix(ORIGIN).unwrap();
        assert_eq!(
            metadata,
            format!("{ORIGIN}/.well-known/oauth-protected-resource{suffix}")
        );
        assert_eq!(
            json(server.call("GET", metadata, &[], "").await).await["resource"],
            resource
        );
    }
    let body = serde_json::json!({"@context": ["https://www.w3.org/ns/lws/v1"], "type": "WebhookSubscription", "topic": [format!("{root}alice/")], "inbox": "https://inbox.example/in"}).to_string();
    let response = server
        .call(
            "POST",
            subscriptions,
            &[("authorization", &auth), ("content-type", LWS_JSON)],
            &body,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(
        server
            .call(
                "DELETE",
                created["subscription"].as_str().unwrap(),
                &[("authorization", &auth)],
                ""
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );

    let as_metadata = json(
        server
            .call(
                "GET",
                &format!("{}/.well-known/lws-configuration", server.base),
                &[],
                "",
            )
            .await,
    )
    .await;
    assert_eq!(as_metadata["issuer"], server.base);
    let jwks = json(
        server
            .call("GET", as_metadata["jwks_uri"].as_str().unwrap(), &[], "")
            .await,
    )
    .await;
    assert!(!jwks["keys"].as_array().unwrap().is_empty());
    assert_eq!(
        server
            .call(
                "GET",
                as_metadata["token_endpoint"].as_str().unwrap(),
                &[],
                ""
            )
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn native_mount_follows_public_urls_and_rejects_outside_paths() {
    for (base, mount) in [
        (ORIGIN.to_string(), "1"),
        (format!("{ORIGIN}/"), "true"),
        (format!("{ORIGIN}/alice"), "1"),
        (format!("{ORIGIN}/alice/"), "True"),
        (format!("{ORIGIN}/team/alice/"), "TRUE"),
    ] {
        let server = Server::start(&base, Some(mount), false).await;
        exercise_advertised_urls(&server).await;
        if server.base != ORIGIN {
            for path in [
                "/",
                "/child",
                "/alice-other/child",
                "/.lws/subscriptions/",
                "/.well-known/lws-configuration",
                "/.well-known/oauth-protected-resource/elsewhere/",
            ] {
                assert_eq!(
                    server
                        .call(
                            "GET",
                            path,
                            &[("authorization", &format!("Bearer {}", server.token))],
                            ""
                        )
                        .await
                        .status(),
                    StatusCode::NOT_FOUND,
                    "outside mount: {base}{path}"
                );
            }
        }
        for probe in ["/livez", "/readyz"] {
            assert_eq!(
                server.call("GET", probe, &[], "").await.status(),
                StatusCode::OK
            );
        }
    }
}

#[tokio::test]
async fn default_native_mapping_follows_urls_through_a_stripping_proxy() {
    for mount in [None, Some("0"), Some("false")] {
        for base in [
            ORIGIN.to_string(),
            format!("{ORIGIN}/"),
            format!("{ORIGIN}/alice"),
            format!("{ORIGIN}/alice/"),
        ] {
            let server = Server::start(&base, mount, true).await;
            exercise_advertised_urls(&server).await;
            if server.base != ORIGIN {
                // Bypass the proxy: default mode MUST NOT strip even a matching path prefix.
                let response = server
                    .client
                    .get(format!("{}/alice/child", server.endpoint))
                    .bearer_auth(&server.token)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(
                    response.text().await.unwrap(),
                    "inner",
                    "default native requests keep every path segment"
                );
            }
        }
    }
}

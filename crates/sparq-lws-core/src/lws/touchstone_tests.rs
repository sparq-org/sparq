//! HTTP regressions matching Touchstone's protected-resource and moving scenarios.

use super::*;
use axum::http::Request;
use test_store::{body_json, links, response_bytes, FlakyStore};
use tower::ServiceExt;

const BASE: &str = "https://storage.example";
const OWNER: &str = "https://owner.example/#me";
const WRITER: &str = "https://writer.example/#me";
const PARENT: &str = "https://storage.example/c/";
const RESOURCE: &str = "https://storage.example/c/moved";
const DESTINATION: &str = "https://storage.example/c/destination/";
const CONTAINER: &str = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";

struct Fixture {
    app: Router,
    store: FlakyStore,
    owner: String,
    writer: String,
}

impl Fixture {
    async fn new() -> Self {
        let store = FlakyStore::new();
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some(OWNER.into());
        let bearer = |subject| {
            format!(
                "Bearer {}",
                tokens::mint(&cfg, subject, "https://app.example/", &cfg.storage())
            )
        };
        let owner = bearer(OWNER);
        let writer = bearer(WRITER);
        let app = router(store.clone(), cfg).await.unwrap();
        let fixture = Self {
            app,
            store,
            owner,
            writer,
        };
        for (parent, slug, link, body) in [
            ("/", "c", CONTAINER, ""),
            ("/c/", "destination", CONTAINER, ""),
            ("/c/", "moved", "", "a resource that stays where it is"),
        ] {
            let created = fixture
                .call(
                    "POST",
                    parent,
                    Some(&fixture.owner),
                    &[
                        ("slug", slug),
                        ("link", link),
                        ("content-type", "text/plain"),
                    ],
                    body,
                )
                .await;
            assert_eq!(created.status(), StatusCode::CREATED);
        }
        fixture
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }

    async fn get(&self, path: &str) -> Response {
        self.call("GET", path, Some(&self.owner), &[], "").await
    }

    async fn snapshot(&self) -> Vec<(StatusCode, HeaderMap, Bytes)> {
        let mut snapshot = Vec::new();
        for path in ["/c/moved", "/c/moved.meta", "/c/", "/c/destination/"] {
            snapshot.push(response_bytes(self.get(path).await).await);
        }
        snapshot
    }

    fn assert_no_writes(&self, before: crate::store::CounterSnapshot) {
        let after = self.store.counters.snapshot();
        assert_eq!(
            after.sparql_updates, before.sparql_updates,
            "no index writes"
        );
        assert_eq!(
            after.blob_puts, before.blob_puts,
            "no content or metadata writes"
        );
    }
}

fn up_patch(target: &str) -> String {
    json!([{"op": "add", "path": "/linkset/0/up", "value": [{"href": target}]}]).to_string()
}

#[tokio::test]
async fn touchstone_up_changes_to_forged_and_existing_parents_are_refused() {
    let f = Fixture::new().await;
    let description = body_json(f.call("GET", "/", None, &[("accept", LWS_CID)], "").await).await;
    for capability in description["capability"].as_array().unwrap() {
        let types = &capability["type"];
        let terms = types
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(types));
        assert!(!terms
            .iter()
            .filter_map(Value::as_str)
            .any(|term| { term.rsplit(['#', '/']).next() == Some("MoveResource") }));
    }
    for (destination, conditional) in [
        ("https://linkset.invalid/forged-parent/", false),
        (DESTINATION, true),
    ] {
        let resource = f.get("/c/moved").await;
        assert_eq!(resource.status(), StatusCode::OK);
        let linkset = links(&resource, "linkset").pop().unwrap();
        let current = f.get(&linkset).await;
        let etag = current.headers()[header::ETAG].to_str().unwrap().to_owned();
        let mut headers = vec![("content-type", JSON_PATCH)];
        if conditional {
            headers.push(("if-match", etag.as_str()));
        }
        let before = f.snapshot().await;
        let counters = f.store.counters.snapshot();
        let refused = f
            .call(
                "PATCH",
                &linkset,
                Some(&f.owner),
                &headers,
                &up_patch(destination),
            )
            .await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert_eq!(refused.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        assert_eq!(body_json(refused).await["status"], 409);
        f.assert_no_writes(counters);
        assert_eq!(f.snapshot().await, before);
        assert_eq!(links(&f.get("/c/moved").await, "up"), vec![PARENT]);
        assert_eq!(
            body_json(
                f.call(
                    "GET",
                    DESTINATION,
                    Some(&f.owner),
                    &[("accept", LWS_JSON)],
                    ""
                )
                .await
            )
            .await["items"],
            json!([])
        );
    }
    let accepted = f
        .call(
            "PATCH",
            "/c/moved.meta",
            Some(&f.owner),
            &[("content-type", JSON_PATCH)],
            &up_patch(PARENT),
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    assert_eq!(links(&f.get("/c/moved").await, "up"), vec![PARENT]);
}

#[tokio::test]
async fn up_refusal_covers_whole_linksets_and_link_headers_before_any_write() {
    let f = Fixture::new().await;
    for (method, path, content_type, body, extra) in [
        ("PUT", "/c/moved.meta", LINKSET_JSON, json!({"linkset": [{"anchor": RESOURCE, "up": [{"href": DESTINATION}], "license": [{"href": "https://example.org/new"}]}]}).to_string(), vec![]),
        // A second own-context entry and case-insensitive relation must not bypass the check.
        ("PATCH", "/c/moved.meta", JSON_PATCH, json!([{"op": "add", "path": "/linkset/-", "value": {"anchor": RESOURCE, "UP": [{"href": DESTINATION}]}}]).to_string(), vec![]),
        ("PUT", "/c/moved", "text/plain", "replacement".into(), vec![("link", "<https://storage.example/c/destination/>; rel=\"up license\""), ("prefer", "set-linkset")]),
    ] {
        let before = f.snapshot().await;
        let counters = f.store.counters.snapshot();
        let mut headers = vec![("content-type", content_type)];
        headers.extend(extra);
        let response = f.call(method, path, Some(&f.owner), &headers, &body).await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{method} {path}");
        f.assert_no_writes(counters);
        assert_eq!(f.snapshot().await, before);
    }
    for target in [PARENT, "./"] {
        let body = json!({"linkset": [{"anchor": RESOURCE, "up": [{"href": target}]}]}).to_string();
        assert_eq!(
            f.call(
                "PUT",
                "/c/moved.meta",
                Some(&f.owner),
                &[("content-type", LINKSET_JSON)],
                &body
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let link = format!("<{target}>; rel=\"up\"");
        assert_eq!(
            f.call(
                "PUT",
                "/c/moved",
                Some(&f.owner),
                &[
                    ("content-type", "text/plain"),
                    ("link", &link),
                    ("prefer", "set-linkset")
                ],
                "updated"
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }
    // Another context's up is an ordinary link; the resource's parent remains fixed.
    let body = json!({"linkset": [{"anchor": "https://other.example/resource", "up": [{"href": DESTINATION}]}]}).to_string();
    assert_eq!(
        f.call(
            "PUT",
            "/c/moved.meta",
            Some(&f.owner),
            &[("content-type", LINKSET_JSON)],
            &body
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(links(&f.get("/c/moved").await, "up"), vec![PARENT]);
    // A different trailing slash denotes a different destination, even at the real parent.
    assert_eq!(
        f.call(
            "PATCH",
            "/c/moved.meta",
            Some(&f.owner),
            &[("content-type", JSON_PATCH)],
            &up_patch("https://storage.example/c")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn up_conflict_is_decided_after_authorization_and_preconditions() {
    let f = Fixture::new().await;
    // This agent can modify but cannot read. All writes below still travel over the router.
    let grant = json!({"@context": [LWS_CONTEXT], "type": ["AccessGrant"], "storage": format!("{BASE}/"), "access": [{"type": ["AccessPolicy"], "action": ["modify"], "assignee": WRITER}]}).to_string();
    let granted = f
        .call(
            "POST",
            GRANTS_PATH,
            Some(&f.owner),
            &[("content-type", LWS_JSON)],
            &grant,
        )
        .await;
    assert_eq!(granted.status(), StatusCode::CREATED);
    for (method, path, content_type, forged, ordinary, extra) in [
        (
            "PATCH",
            "/c/moved.meta",
            JSON_PATCH,
            up_patch(DESTINATION),
            up_patch(PARENT),
            vec![],
        ),
        (
            "PUT",
            "/c/moved.meta",
            LINKSET_JSON,
            json!({"linkset": [{"anchor": RESOURCE, "up": [{"href": DESTINATION}]}]}).to_string(),
            json!({"linkset": []}).to_string(),
            vec![],
        ),
        (
            "PUT",
            "/c/moved",
            "text/plain",
            "replacement".into(),
            "replacement".into(),
            vec![
                (
                    "link",
                    "<https://storage.example/c/destination/>; rel=\"up\"",
                ),
                ("prefer", "set-linkset"),
            ],
        ),
    ] {
        let mut headers = vec![("content-type", content_type)];
        headers.extend(extra);
        let counters = f.store.counters.snapshot();
        for authorization in [None, Some(f.writer.as_str())] {
            let denied =
                response_bytes(f.call(method, path, authorization, &headers, &forged).await).await;
            let normal = response_bytes(
                f.call(
                    method,
                    path,
                    authorization,
                    &[("content-type", content_type)],
                    &ordinary,
                )
                .await,
            )
            .await;
            assert_eq!(
                denied, normal,
                "same authorization outcome for {method} {path}"
            );
            assert_eq!(
                denied.0,
                if authorization.is_some() {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
        }
        headers.push(("if-match", "\"stale\""));
        assert_eq!(
            f.call(method, path, Some(&f.owner), &headers, &forged)
                .await
                .status(),
            StatusCode::PRECONDITION_FAILED,
            "{method} {path}"
        );
        f.assert_no_writes(counters);
    }
}

#[tokio::test]
async fn plain_put_ignores_up_link_headers() {
    let f = Fixture::new().await;
    let mut before = body_json(f.get("/c/moved.meta").await).await;
    before["linkset"][0].as_object_mut().unwrap().remove("self");
    let response = f
        .call(
            "PUT",
            "/c/moved",
            Some(&f.owner),
            &[
                ("content-type", "text/plain"),
                (
                    "link",
                    "<https://storage.example/c/destination/>; rel=\"up\"",
                ),
            ],
            "replacement",
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let resource = f.get("/c/moved").await;
    assert_eq!(links(&resource, "up"), vec![PARENT]);
    assert_eq!(response_bytes(resource).await.2, "replacement");
    let mut after = body_json(f.get("/c/moved.meta").await).await;
    after["linkset"][0].as_object_mut().unwrap().remove("self");
    assert_eq!(after, before);
}

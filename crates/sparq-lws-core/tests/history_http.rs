//! Version/trash lifecycle through the deployed HTTP routers.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use axum::{
    body::{to_bytes, Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use solid_oidc_verifier::{
    config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
};
use sparq_lws_core::{
    app::{build_router, AppState},
    auth::AuthContext,
    ldp::handler::LdpState,
    lws::{self, LwsConfig},
    store::{
        history::HistoryConfig, CompositeStore, InMemoryBlobStore, InMemorySparqClient,
        SparqClient, Store,
    },
};
use tower::ServiceExt;
const BASE: &str = common::BASE_URL;
struct Harness {
    app: Router,
    issuer: common::KeyKit,
    client: common::KeyKit,
    lws: bool,
    lws_config: Option<LwsConfig>,
}
impl Harness {
    async fn new<S: SparqClient + 'static>(lws: bool, index: S, enabled: bool) -> Self {
        Self::with_policy(
            lws,
            index,
            HistoryConfig {
                versions: enabled,
                trash: enabled,
                ..Default::default()
            },
        )
        .await
    }
    async fn with_policy<S: SparqClient + 'static>(
        lws: bool,
        index: S,
        policy: HistoryConfig,
    ) -> Self {
        let store = CompositeStore::new(index, InMemoryBlobStore::new())
            .with_history(policy)
            .unwrap();
        store
            .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let issuer = common::KeyKit::generate();
        let client = common::KeyKit::generate();
        let app = if lws {
            let mut cfg = LwsConfig::new(BASE);
            cfg.open = true;
            lws::router(store, cfg).await.unwrap()
        } else {
            let acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#a> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control.",common::WEBID);
            store
                .write(&format!("{BASE}/.acl"), Bytes::from(acl), "text/turtle")
                .await
                .unwrap();
            let cfg = VerifierConfig::new(vec![common::ISSUER.into()], BASE);
            let replay = InMemoryReplayStore::with_window(cfg.replay_ttl());
            let verifier = Verifier::new(cfg, common::jwks_provider(&issuer), replay).unwrap();
            build_router(AppState::new(
                AuthContext::new(verifier, BASE),
                LdpState::new(store, BASE),
            ))
        };
        Self {
            app,
            issuer,
            client,
            lws,
            lws_config: None,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
        body: &str,
    ) -> (StatusCode, HeaderMap, Bytes) {
        self.call_as(common::WEBID, method, path, extra, body).await
    }
    async fn call_as(
        &self,
        agent: &str,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
        body: &str,
    ) -> (StatusCode, HeaderMap, Bytes) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(cfg) = &self.lws_config {
            req = req.header(
                "authorization",
                format!(
                    "Bearer {}",
                    lws::tokens::mint(cfg, agent, "https://app.example/", &format!("{BASE}/"))
                ),
            );
        }
        if !self.lws {
            let access =
                common::mint_access_token_for(&self.issuer, &self.client.thumbprint, agent);
            let proof =
                common::mint_dpop_proof(&self.client, method, &format!("{BASE}{path}"), &access);
            req = req
                .header("authorization", format!("DPoP {access}"))
                .header("dpop", proof);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let response = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        (
            parts.status,
            parts.headers,
            to_bytes(body, 1024 * 1024).await.unwrap(),
        )
    }
    async fn create(&self, body: &str) -> String {
        let (status, headers, bytes) = if self.lws {
            self.call("POST", "/", &[("content-type", "text/plain")], body)
                .await
        } else {
            self.call("PUT", "/note", &[("content-type", "text/plain")], body)
                .await
        };
        assert!(
            status.is_success(),
            "{status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if self.lws {
            headers["location"]
                .to_str()
                .unwrap()
                .strip_prefix(BASE)
                .unwrap()
                .to_string()
        } else {
            "/note".into()
        }
    }
    fn versions(&self, resource: &str) -> String {
        format!(
            "{}?resource={BASE}{resource}",
            if self.lws {
                "/.lws/versions/"
            } else {
                "/.versions/"
            }
        )
    }
    fn trash(&self) -> &'static str {
        if self.lws {
            "/.lws/trash/"
        } else {
            "/.trash/"
        }
    }
    fn accept(&self) -> &'static str {
        if self.lws {
            "application/lws+json"
        } else {
            "application/ld+json"
        }
    }
    async fn versions_list(&self, path: &str) -> Vec<(String, String)> {
        let (status, _, body) = self
            .call(
                "GET",
                &self.versions(path),
                &[("accept", self.accept())],
                "",
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let v: Value = serde_json::from_slice(&body).unwrap();
        if self.lws {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| {
                    (
                        i["id"].as_str().unwrap().into(),
                        i["etag"].as_str().unwrap().into(),
                    )
                })
                .collect()
        } else {
            v.as_array()
                .unwrap()
                .iter()
                .filter_map(|i| {
                    Some((
                        i["@id"].as_str()?.to_string(),
                        i["https://jeswr.org/ns/solid-sd#etag"][0]["@value"]
                            .as_str()?
                            .to_string(),
                    ))
                })
                .collect()
        }
    }
    async fn trash_list(&self) -> Vec<String> {
        let (status, _, body) = self
            .call("GET", self.trash(), &[("accept", self.accept())], "")
            .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let v: Value = serde_json::from_slice(&body).unwrap();
        if self.lws {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["id"].as_str().unwrap().to_string())
                .collect()
        } else {
            v.as_array().unwrap()[0]["http://www.w3.org/ns/ldp#contains"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["@id"].as_str().unwrap().into())
                .collect()
        }
    }
}
async fn sq8<S: SparqClient + 'static>(lws: bool, index: S) {
    let h = Harness::new(lws, index, true).await;
    let resource = h.create("one").await;
    let mut live_tag = String::new();
    for text in ["two", "three"] {
        let (status, headers, body) = h
            .call("PUT", &resource, &[("content-type", "text/plain")], text)
            .await;
        assert!(
            status.is_success(),
            "{status}: {}",
            String::from_utf8_lossy(&body)
        );
        live_tag = headers["etag"].to_str().unwrap().to_string();
    }
    let versions = h.versions_list(&resource).await;
    assert_eq!(versions.len(), 3);
    let first = versions
        .iter()
        .find(|(id, _)| id != &format!("{BASE}{resource}"))
        .unwrap();
    let path = first.0.strip_prefix(BASE).unwrap();
    let (status, headers, bytes) = h.call("GET", path, &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key("memento-datetime"));
    assert_eq!(headers["etag"], first.1);
    assert!(headers["link"]
        .to_str()
        .unwrap()
        .contains("rel=\"original\""));
    let (status, _, _) = h
        .call("PUT", path, &[("content-type", "text/plain")], "change")
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _, _) = h
        .call(
            "PUT",
            &resource,
            &[("content-type", "text/plain"), ("if-match", &live_tag)],
            std::str::from_utf8(&bytes).unwrap(),
        )
        .await;
    assert!(status.is_success());
    assert_eq!(h.versions_list(&resource).await.len(), 4);
    let (status, headers, _) = h.call("GET", &resource, &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get_all("link")
        .iter()
        .any(|v| v.to_str().unwrap().contains("version-history")));
    assert_eq!(
        h.call("DELETE", &resource, &[], "").await.0,
        StatusCode::NO_CONTENT
    );
    let entries = h.trash_list().await;
    assert_eq!(entries.len(), 1);
    let trashed = entries[0].strip_prefix(BASE).unwrap();
    assert_eq!(h.call("GET", trashed, &[], "").await.0, StatusCode::OK);
    let (status, headers, _) = h
        .call(
            "POST",
            h.trash(),
            &[("content-type", "application/json")],
            &json!({"restore":entries[0]}).to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let target = headers["location"].to_str().unwrap();
    if lws {
        assert_ne!(target, format!("{BASE}{resource}"));
    } else {
        assert_eq!(target, format!("{BASE}{resource}"));
    }
    assert_eq!(
        h.call("GET", target.strip_prefix(BASE).unwrap(), &[], "")
            .await
            .0,
        StatusCode::OK
    );
    assert!(h.trash_list().await.is_empty());
}
#[tokio::test]
async fn sq8_solid_memory() {
    sq8(false, InMemorySparqClient::new()).await;
}
#[tokio::test]
async fn sq8_lws_memory() {
    sq8(true, InMemorySparqClient::new()).await;
}
#[cfg(feature = "embedded-sparq")]
#[tokio::test]
async fn sq8_solid_embedded() {
    sq8(
        false,
        sparq_lws_core::store::EmbeddedSparqClient::in_memory().unwrap(),
    )
    .await;
}
#[cfg(feature = "embedded-sparq")]
#[tokio::test]
async fn sq8_lws_embedded() {
    sq8(
        true,
        sparq_lws_core::store::EmbeddedSparqClient::in_memory().unwrap(),
    )
    .await;
}
#[tokio::test]
async fn purge_removes_history_before_iri_reuse_in_both_modes() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        let resource = h.create("one").await;
        assert!(h
            .call("PUT", &resource, &[("content-type", "text/plain")], "two")
            .await
            .0
            .is_success());
        assert_eq!(h.versions_list(&resource).await.len(), 2);
        assert_eq!(
            h.call("DELETE", &resource, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        let entries = h.trash_list().await;
        assert_eq!(
            h.call("DELETE", entries[0].strip_prefix(BASE).unwrap(), &[], "")
                .await
                .0,
            StatusCode::NO_CONTENT
        );
        assert!(h.versions_list(&resource).await.is_empty());
        if !lws {
            assert!(h
                .call("PUT", &resource, &[("content-type", "text/plain")], "fresh")
                .await
                .0
                .is_success());
            assert_eq!(h.versions_list(&resource).await.len(), 1);
        }
    }
}
#[tokio::test]
async fn disabled_routes_catalog_and_delete_match_the_baseline() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), false).await;
        let resource = h.create("body").await;
        assert_eq!(
            h.call("DELETE", &resource, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        assert!(h.call("GET", &resource, &[], "").await.0.is_client_error());
        assert!(h
            .call("GET", h.trash(), &[("accept", h.accept())], "")
            .await
            .0
            .is_client_error());
        assert!(h
            .call("GET", &h.versions(&resource), &[("accept", h.accept())], "")
            .await
            .0
            .is_client_error());
        let path = if lws { "/" } else { "/.well-known/solid" };
        let (_, _, body) = h.call("GET", path, &[], "").await;
        let body = String::from_utf8_lossy(&body);
        assert!(!body.contains("#trash\"") && !body.contains("#versions\""));
    }
}

async fn matrix(lws_mode: bool) -> Harness {
    use sparq_lws_core::store::history::TrashFields;
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
        .with_history(HistoryConfig {
            versions: true,
            trash: true,
            ..Default::default()
        })
        .unwrap();
    store
        .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
        .await
        .unwrap();
    for (name, could_read) in [("readable", true), ("unreadable", false)] {
        let iri = format!("{BASE}/{name}");
        store
            .create_in_container(
                &format!("{BASE}/"),
                &iri,
                Bytes::from_static(b"retained bytes"),
                "text/plain",
            )
            .await
            .unwrap();
        store
            .trash_resource(
                &iri,
                Some(&format!("{BASE}/")),
                TrashFields {
                    original: iri.clone(),
                    trashed_at: std::time::SystemTime::now(),
                    deleted_by: Some("https://deleter.example/#me".into()),
                    deleter_could_read: could_read,
                },
            )
            .await
            .unwrap();
    }
    let issuer = common::KeyKit::generate();
    let client = common::KeyKit::generate();
    let mut lws_config = None;
    let app = if lws_mode {
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some(common::WEBID.into());
        lws_config = Some(cfg.clone());
        lws::router(store, cfg).await.unwrap()
    } else {
        store.write(&format!("{BASE}/.acl"),Bytes::from(format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#a> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control.",common::WEBID)),"text/turtle").await.unwrap();
        let cfg = VerifierConfig::new(vec![common::ISSUER.into()], BASE);
        let replay = InMemoryReplayStore::with_window(cfg.replay_ttl());
        build_router(AppState::new(
            AuthContext::new(
                Verifier::new(cfg, common::jwks_provider(&issuer), replay).unwrap(),
                BASE,
            ),
            LdpState::new(store, BASE),
        ))
    };
    Harness {
        app,
        issuer,
        client,
        lws: lws_mode,
        lws_config,
    }
}
fn trash_ids(lws: bool, body: &[u8]) -> Vec<String> {
    let v: Value = serde_json::from_slice(body).unwrap();
    if lws {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    } else {
        v.as_array().unwrap()[0]["http://www.w3.org/ns/ldp#contains"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["@id"].as_str().unwrap().into())
            .collect()
    }
}
#[tokio::test]
async fn trash_authorization_owner_reading_deleter_nonreading_deleter_and_stranger() {
    for lws in [false, true] {
        let h = matrix(lws).await;
        let all = h.trash_list().await;
        assert_eq!(all.len(), 2);
        let deleter = "https://deleter.example/#me";
        let (status, _, body) = h
            .call_as(deleter, "GET", h.trash(), &[("accept", h.accept())], "")
            .await;
        assert_eq!(status, StatusCode::OK);
        let own = trash_ids(lws, &body);
        assert_eq!(own.len(), 1);
        let allowed = &own[0];
        let denied = all.iter().find(|id| *id != allowed).unwrap();
        for method in ["GET", "DELETE"] {
            assert_eq!(
                h.call_as(deleter, method, denied.strip_prefix(BASE).unwrap(), &[], "")
                    .await
                    .0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            h.call_as(deleter, "GET", allowed.strip_prefix(BASE).unwrap(), &[], "")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            h.call_as(
                deleter,
                "POST",
                h.trash(),
                &[("content-type", "application/json")],
                &json!({"restore":allowed}).to_string()
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "restore also needs parent create permission"
        );
        let stranger = "https://stranger.example/#me";
        let (status, _, body) = h
            .call_as(stranger, "GET", h.trash(), &[("accept", h.accept())], "")
            .await;
        assert_eq!(status, StatusCode::OK);
        assert!(trash_ids(lws, &body).is_empty());
        for method in ["GET", "DELETE"] {
            assert_eq!(
                h.call_as(
                    stranger,
                    method,
                    allowed.strip_prefix(BASE).unwrap(),
                    &[],
                    ""
                )
                .await
                .0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            h.call(
                "POST",
                h.trash(),
                &[("content-type", "application/json")],
                &json!({"restore":denied}).to_string()
            )
            .await
            .0,
            StatusCode::CREATED
        );
        assert_eq!(
            h.call_as(
                deleter,
                "DELETE",
                allowed.strip_prefix(BASE).unwrap(),
                &[],
                ""
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
}

#[tokio::test]
async fn retained_records_are_absent_from_live_discovery_in_both_modes() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        let resource = h.create("one").await;
        assert!(h
            .call("PUT", &resource, &[("content-type", "text/plain")], "two")
            .await
            .0
            .is_success());
        assert_eq!(
            h.call("DELETE", &resource, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        let mut requests = vec![
            ("/", h.accept()),
            (
                if lws { "/" } else { "/.well-known/solid" },
                if lws {
                    "application/lws+cid"
                } else {
                    "text/turtle"
                },
            ),
        ];
        if lws {
            requests.push(("/.lws/types/index", "application/lws+json"));
        }
        for (path, accept) in requests {
            let (status, _, body) = h.call("GET", path, &[("accept", accept)], "").await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let body = String::from_utf8_lossy(&body);
            assert!(
                !body.contains("urn:x-sparq-lws:versions/")
                    && !body.contains("urn:x-sparq-lws:trash/"),
                "{path}: {body}"
            );
        }
        if !lws {
            #[cfg(feature = "sparql-endpoint")]
            {
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair(
                        "query",
                        "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }",
                    )
                    .finish();
                let (status, _, body) = h
                    .call(
                        "GET",
                        &format!("/sparql?{query}"),
                        &[("accept", "application/sparql-results+json")],
                        "",
                    )
                    .await;
                assert_eq!(status, StatusCode::OK);
                let body = String::from_utf8_lossy(&body);
                assert!(
                    !body.contains("urn:x-sparq-lws:versions/")
                        && !body.contains("urn:x-sparq-lws:trash/")
                );
            }
        }
    }
}

#[tokio::test]
async fn enabled_catalog_advertises_template_and_duration_in_both_modes() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        let (status, _, body) = h
            .call(
                "GET",
                if lws { "/" } else { "/.well-known/solid" },
                &[(
                    "accept",
                    if lws {
                        "application/lws+cid"
                    } else {
                        "text/turtle"
                    },
                )],
                "",
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("VersioningService"));
        assert!(body.contains("TrashService"));
        assert!(body.contains("{resource}"));
        assert!(body.contains("retention"));
    }
}

#[tokio::test]
async fn deletion_captures_the_deleters_read_permission_in_both_modes() {
    for lws in [false, true] {
        for may_read in [false, true] {
            let h = matrix(lws).await;
            let deleter = "https://writer.example/#me";
            let resource = h.create("body").await;
            if lws {
                let mut actions = vec!["delete", "create"];
                if may_read {
                    actions.push("read");
                }
                let grant = json!({"@context":["https://www.w3.org/ns/lws/v1"],"type":["AccessGrant"],"storage":format!("{BASE}/"),"access":[{"type":["AccessPolicy"],"action":actions,"assignee":deleter}]});
                let (status, _, body) = h
                    .call(
                        "POST",
                        "/.lws/grants/",
                        &[("content-type", "application/lws+json")],
                        &grant.to_string(),
                    )
                    .await;
                assert_eq!(
                    status,
                    StatusCode::CREATED,
                    "{}",
                    String::from_utf8_lossy(&body)
                );
            } else {
                let extra = if may_read { ", acl:Read" } else { "" };
                let acl=format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#owner> a acl:Authorization; acl:agent <{}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Read, acl:Write, acl:Control. <#writer> a acl:Authorization; acl:agent <{deleter}>; acl:accessTo <{BASE}/>; acl:default <{BASE}/>; acl:mode acl:Write{extra}.",common::WEBID);
                assert!(h
                    .call("PUT", "/.acl", &[("content-type", "text/turtle")], &acl)
                    .await
                    .0
                    .is_success());
            }
            assert_eq!(
                h.call_as(deleter, "GET", &resource, &[], "")
                    .await
                    .0
                    .is_success(),
                may_read
            );
            assert_eq!(
                h.call_as(deleter, "DELETE", &resource, &[], "").await.0,
                StatusCode::NO_CONTENT
            );
            let (status, _, body) = h
                .call_as(deleter, "GET", h.trash(), &[("accept", h.accept())], "")
                .await;
            assert_eq!(status, StatusCode::OK);
            let ids = trash_ids(lws, &body);
            assert_eq!(ids.len(), usize::from(may_read));
            assert_eq!(h.trash_list().await.len(), 3);
            if may_read {
                assert_eq!(
                    h.call_as(
                        deleter,
                        "POST",
                        h.trash(),
                        &[("content-type", "application/json")],
                        &json!({"restore":ids[0]}).to_string()
                    )
                    .await
                    .0,
                    StatusCode::CREATED
                );
            }
        }
    }
}

#[tokio::test]
async fn restore_conflicts_with_an_existing_target_or_a_missing_parent() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        if !lws {
            let resource = h.create("one").await;
            assert_eq!(
                h.call("DELETE", &resource, &[], "").await.0,
                StatusCode::NO_CONTENT
            );
            let old = h.trash_list().await.remove(0);
            h.create("occupied").await;
            assert_eq!(
                h.call(
                    "POST",
                    h.trash(),
                    &[("content-type", "application/json")],
                    &json!({"restore":old}).to_string()
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
        }
    }
}

#[tokio::test]
async fn missing_parent_conflicts_and_recursive_delete_bypasses_trash() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        let parent = if lws {
            let (status, headers, _) = h
                .call(
                    "POST",
                    "/",
                    &[(
                        "link",
                        "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
                    )],
                    "",
                )
                .await;
            assert_eq!(status, StatusCode::CREATED);
            headers["location"]
                .to_str()
                .unwrap()
                .strip_prefix(BASE)
                .unwrap()
                .to_string()
        } else {
            assert!(h
                .call("PUT", "/c/", &[("content-type", "text/turtle")], "")
                .await
                .0
                .is_success());
            "/c/".into()
        };
        let resource = if lws {
            let (status, headers, _) = h
                .call("POST", &parent, &[("content-type", "text/plain")], "body")
                .await;
            assert_eq!(status, StatusCode::CREATED);
            headers["location"]
                .to_str()
                .unwrap()
                .strip_prefix(BASE)
                .unwrap()
                .to_string()
        } else {
            let resource = format!("{parent}note");
            assert!(h
                .call("PUT", &resource, &[("content-type", "text/plain")], "body")
                .await
                .0
                .is_success());
            resource
        };
        assert_eq!(
            h.call("DELETE", &resource, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        let copy = h.trash_list().await.remove(0);
        assert_eq!(
            h.call("DELETE", &parent, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            h.call(
                "POST",
                h.trash(),
                &[("content-type", "application/json")],
                &json!({"restore":copy}).to_string()
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        if lws {
            let (status, headers, _) = h
                .call(
                    "POST",
                    "/",
                    &[(
                        "link",
                        "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
                    )],
                    "",
                )
                .await;
            assert_eq!(status, StatusCode::CREATED);
            let parent = headers["location"]
                .to_str()
                .unwrap()
                .strip_prefix(BASE)
                .unwrap();
            assert_eq!(
                h.call(
                    "POST",
                    parent,
                    &[("content-type", "text/plain")],
                    "subtree body"
                )
                .await
                .0,
                StatusCode::CREATED
            );
            assert_eq!(
                h.call("DELETE", parent, &[("depth", "infinity")], "")
                    .await
                    .0,
                StatusCode::NO_CONTENT
            );
            assert_eq!(
                h.trash_list().await.len(),
                1,
                "recursive deletion adds no trash entries"
            );
        }
    }
}

#[test]
fn boot_refuses_http_index_with_either_service_enabled() {
    for flag in ["SOLID_SERVER_VERSIONS", "SOLID_SERVER_TRASH"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_sparq-lws-core"))
            .env_clear()
            .env("PSS_SPARQ_BACKEND", "http")
            .env(flag, "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("versions and trash require the memory or embedded index backend"),
            "{error}"
        );
    }
}

#[cfg(all(feature = "embedded-sparq", feature = "sparql-endpoint"))]
#[tokio::test]
async fn restored_rdf_is_present_in_the_live_solid_query_dataset() {
    let h = Harness::new(
        false,
        sparq_lws_core::store::EmbeddedSparqClient::in_memory().unwrap(),
        true,
    )
    .await;
    let turtle = "<urn:document> <urn:predicate> \"restored\" .";
    assert!(h
        .call("PUT", "/note", &[("content-type", "text/turtle")], turtle)
        .await
        .0
        .is_success());
    assert_eq!(
        h.call("DELETE", "/note", &[], "").await.0,
        StatusCode::NO_CONTENT
    );
    let copy = h.trash_list().await.remove(0);
    assert_eq!(
        h.call(
            "POST",
            h.trash(),
            &[("content-type", "application/json")],
            &json!({"restore":copy}).to_string()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let query=url::form_urlencoded::Serializer::new(String::new()).append_pair("query","SELECT ?o WHERE { GRAPH <https://pod.example/note> { <urn:document> <urn:predicate> ?o } }").finish();
    let (status, _, body) = h
        .call(
            "GET",
            &format!("/sparql?{query}"),
            &[("accept", "application/sparql-results+json")],
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["results"]["bindings"][0]["o"]["value"], "restored");
}

#[tokio::test(start_paused = true)]
async fn retention_worker_expires_records_without_another_write_in_both_modes() {
    for lws in [false, true] {
        let policy = HistoryConfig {
            versions: true,
            trash: true,
            versions_retention: std::time::Duration::ZERO,
            trash_retention: std::time::Duration::ZERO,
            ..Default::default()
        };
        let h = Harness::with_policy(lws, InMemorySparqClient::new(), policy).await;
        let resource = h.create("one").await;
        assert!(h
            .call("PUT", &resource, &[("content-type", "text/plain")], "two")
            .await
            .0
            .is_success());
        assert_eq!(
            h.call("DELETE", &resource, &[], "").await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(h.trash_list().await.len(), 1);
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        for _ in 0..20 {
            if h.trash_list().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(h.trash_list().await.is_empty(), "the background worker ran");
        assert!(h.versions_list(&resource).await.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn retention_worker_bounds_each_pass_in_both_modes() {
    for lws in [false, true] {
        let policy = HistoryConfig {
            trash: true,
            trash_retention: std::time::Duration::ZERO,
            ..Default::default()
        };
        let h = Harness::with_policy(lws, InMemorySparqClient::new(), policy).await;
        for _ in 0..65 {
            let resource = h.create("body").await;
            assert_eq!(
                h.call("DELETE", &resource, &[], "").await.0,
                StatusCode::NO_CONTENT
            );
        }
        assert_eq!(h.trash_list().await.len(), 65);
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        for _ in 0..20 {
            if h.trash_list().await.len() < 65 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            h.trash_list().await.len(),
            1,
            "one pass has a fixed record bound"
        );
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        for _ in 0..20 {
            if h.trash_list().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(h.trash_list().await.is_empty());
    }
}

#[cfg(feature = "http-sparq")]
#[test]
fn policy_builder_refuses_a_remote_index_before_router_assembly() {
    use sparq_lws_core::store::HttpSparqClient;
    for (versions, trash) in [(true, false), (false, true)] {
        let result = CompositeStore::new(
            HttpSparqClient::new("http://127.0.0.1:9/sparql"),
            InMemoryBlobStore::new(),
        )
        .with_history(HistoryConfig {
            versions,
            trash,
            ..Default::default()
        });
        assert!(result.is_err());
    }
    assert!(CompositeStore::new(
        HttpSparqClient::new("http://127.0.0.1:9/sparql"),
        InMemoryBlobStore::new()
    )
    .with_history(HistoryConfig::default())
    .is_ok());
}

#[test]
fn disabled_settings_preserve_the_baseline_boot_response() {
    let run = |configure: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_sparq-lws-core"));
        command.env_clear().env("PSS_SPARQ_BACKEND", "http");
        if configure {
            command
                .env("SOLID_SERVER_VERSIONS_MAX", "invalid")
                .env("SOLID_SERVER_VERSIONS_RETENTION", "invalid")
                .env("SOLID_SERVER_TRASH_RETENTION", "invalid");
        }
        command.output().unwrap()
    };
    let baseline = run(false);
    let off = run(true);
    assert_eq!(baseline.status.code(), off.status.code());
    assert_eq!(baseline.stdout, off.stdout);
    assert_eq!(baseline.stderr, off.stderr);
}

#[tokio::test]
async fn history_listings_honor_quality_values_and_explicit_refusal() {
    for lws in [false, true] {
        let h = Harness::new(lws, InMemorySparqClient::new(), true).await;
        let resource = h.create("body").await;
        for path in [h.trash().to_string(), h.versions(&resource)] {
            let refused = if lws {
                "application/lws+json;q=0"
            } else {
                "application/ld+json;q=0, text/turtle;q=0"
            };
            assert_eq!(
                h.call("GET", &path, &[("accept", refused)], "").await.0,
                StatusCode::NOT_ACCEPTABLE
            );
            if !lws {
                let (status, headers, _) = h
                    .call(
                        "GET",
                        &path,
                        &[("accept", "application/ld+json;q=0, text/turtle;q=1")],
                        "",
                    )
                    .await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(headers["content-type"], "text/turtle");
            }
        }
    }
}

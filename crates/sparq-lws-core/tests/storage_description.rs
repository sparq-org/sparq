//! Public discovery over the real routers, without a remote JSON-LD loader.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::Router;
use oxrdf::graph::CanonicalizationAlgorithm;
use oxrdf::{Graph, NamedNode, Triple};
use serde_json::{json, Value};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router_with_catalog, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::describe::{ServiceCatalog, ServiceEntry};
use sparq_lws_core::ldp::content::{parse_to_triples, serialize_triples, RdfFormat};
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::lws::{self, LwsConfig};
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use tower::ServiceExt;

const ROOT: &str = "https://pod.example/";
const SSD: &str = "https://jeswr.org/ns/solid-sd#";
const LWS: &str = "https://www.w3.org/ns/lws#";
const DID: &str = "https://www.w3.org/ns/did#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACQ: &str = "http://www.w3.org/ns/solid/sparql#endpoint";
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/storage-description/"
);
type Memory = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

fn store() -> Memory {
    Memory::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
}

async fn solid(catalog: ServiceCatalog, public: bool) -> Router {
    let store = store();
    store
        .write(ROOT, Bytes::new(), "text/turtle")
        .await
        .unwrap();
    if public {
        store.write(&format!("{ROOT}.acl"), Bytes::from(format!(
            "@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#public> a acl:Authorization; acl:agentClass <http://xmlns.com/foaf/0.1/Agent>; acl:accessTo <{ROOT}>; acl:default <{ROOT}>; acl:mode acl:Read."
        )), "text/turtle").await.unwrap();
    }
    let issuer = common::KeyKit::generate();
    let cfg = VerifierConfig::new(vec![common::ISSUER.into()], common::BASE_URL);
    let replay = InMemoryReplayStore::with_window(cfg.replay_ttl());
    let verifier = Verifier::new(cfg, common::jwks_provider(&issuer), replay).unwrap();
    build_router_with_catalog(
        AppState::new(
            AuthContext::new(verifier, common::BASE_URL),
            LdpState::new(store, common::BASE_URL),
        ),
        catalog,
    )
}

async fn lws(catalog: ServiceCatalog, open: bool) -> Router {
    let mut cfg = LwsConfig::new(common::BASE_URL);
    cfg.open = open;
    cfg.notify_key =
        lws::jose::EcKey::from_jwk(include_str!("fixtures/storage-description/test-key.json"))
            .unwrap();
    lws::router_with_catalog(store(), cfg, catalog)
        .await
        .unwrap()
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        to_bytes(body, 1024 * 1024).await.unwrap().to_vec(),
    )
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}{name}")).unwrap()
}

// Explicit substitution rather than a network-capable context loader. An unknown URL fails.
fn offline(mut value: Value) -> Value {
    let contexts = value
        .get_mut("@context")
        .expect("context on compact fixture");
    let list = match contexts.take() {
        Value::Array(a) => a,
        other => vec![other],
    };
    *contexts = list
        .into_iter()
        .map(|c| {
            let Some(url) = c.as_str() else {
                return c;
            };
            let name = match url {
                "https://www.w3.org/ns/cid/v1" => "cid-context.jsonld",
                "https://www.w3.org/ns/lws/v1" => "lws-context.jsonld",
                "https://www.w3.org/ns/solid/notifications-context/v1" => {
                    return json!({
                        "id": "@id", "type": "@type",
                        "channelType": {
                            "@id": "http://www.w3.org/ns/solid/notifications#channelType",
                            "@type": "@vocab"
                        },
                        "feature": {
                            "@id": "http://www.w3.org/ns/solid/notifications#feature",
                            "@type": "@vocab"
                        }
                    });
                }
                other => panic!("unvendored context: {other}"),
            };
            serde_json::from_slice::<Value>(&fixture(name)).unwrap()["@context"].clone()
        })
        .collect();
    value
}

fn triples(format: RdfFormat, body: &[u8]) -> Vec<Triple> {
    parse_to_triples(format, body, ROOT).unwrap()
}

fn canonical(triples: &[Triple]) -> Graph {
    let mut graph = Graph::new();
    for triple in triples {
        graph.insert(triple);
    }
    graph.canonicalize(CanonicalizationAlgorithm::Unstable);
    graph
}

fn has(graph: &[Triple], subject: &str, predicate: &str, object: &str) -> bool {
    graph.contains(&Triple::new(
        NamedNode::new(subject).unwrap(),
        NamedNode::new(predicate).unwrap(),
        NamedNode::new(object).unwrap(),
    ))
}

fn assert_service(graph: &[Triple], fragment: &str, ty: &str, endpoint: &str) {
    let id = format!("{ROOT}#{fragment}");
    assert!(
        has(graph, ROOT, &format!("{DID}service"), &id),
        "unlinked {fragment}"
    );
    assert!(has(graph, &id, RDF_TYPE, ty), "missing type {ty}");
    assert!(
        has(graph, &id, &format!("{DID}serviceEndpoint"), endpoint),
        "wrong endpoint {fragment}"
    );
}

#[tokio::test]
async fn lws_public_catalog_and_shape() {
    let app = lws(ServiceCatalog::default(), false).await;
    let (status, headers, body) =
        call(&app, "GET", "/", &[("accept", "application/lws+cid")], "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/lws+cid");
    let doc: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(doc["@context"][0], "https://www.w3.org/ns/cid/v1");
    assert_eq!(doc["@context"][1], "https://www.w3.org/ns/lws/v1");
    assert!(doc["@context"][2].is_object());
    assert_eq!(
        doc["conformsTo"],
        json!(["https://www.w3.org/TR/lws10-core/"])
    );
    for (ty, endpoint) in [
        ("StorageRoot", ROOT),
        ("StorageDescription", ROOT),
        ("AuthorizationServer", "https://pod.example"),
        ("AccessGrantService", "https://pod.example/.lws/grants/"),
        ("AccessRequestService", "https://pod.example/.lws/requests/"),
        (
            "NotificationService",
            "https://pod.example/.lws/subscriptions/",
        ),
        ("TypeIndexService", "https://pod.example/.lws/types/index"),
        ("TypeSearchService", "https://pod.example/.lws/types/search"),
    ] {
        let service = doc["service"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["type"] == ty)
            .unwrap_or_else(|| panic!("missing {ty}"));
        assert_eq!(service["serviceEndpoint"], endpoint, "{ty}");
    }
    let caps = doc["capability"].as_array().unwrap();
    let patch = caps
        .iter()
        .find(|c| c["type"] == format!("{SSD}PatchSupport"))
        .unwrap();
    assert_eq!(
        patch["mediaType"]["application/json"],
        json!(["application/json-patch+json"])
    );
    assert_eq!(
        patch["mediaType"]["application/ld+json"],
        json!(["application/json-patch+json"])
    );
    assert_eq!(
        patch["mediaType"]["application/linkset+json"],
        json!(["application/json-patch+json"])
    );
    assert_eq!(patch["mediaType"].as_object().unwrap().len(), 3);
    assert!(patch["mediaType"]
        .as_object()
        .unwrap()
        .keys()
        .all(|ty| !ty.contains('*')));
    assert!(caps
        .iter()
        .any(|c| c["type"] == format!("{SSD}RecursiveDelete")));
    assert!(!String::from_utf8_lossy(&body).contains("MoveResource"));
    assert_eq!(
        doc,
        serde_json::from_slice::<Value>(&fixture(if cfg!(feature = "sparql-endpoint") {
            "lws.jsonld"
        } else {
            "lws-no-sparql.jsonld"
        }))
        .unwrap()
    );
    let open = lws(ServiceCatalog::default(), true).await;
    assert_eq!(
        call(&open, "GET", "/", &[("accept", "application/lws+cid")], "")
            .await
            .2,
        body
    );
    let notification = doc["service"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["type"] == "NotificationService")
        .unwrap();
    assert_eq!(
        notification["subscriptionType"],
        json!(["WebhookSubscription"])
    );
    for absent in ["SparqlQueryService", "CrdtSyncService"] {
        assert!(!doc["service"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["type"] == format!("{SSD}{absent}")));
    }
    let expanded = triples(
        RdfFormat::JsonLd,
        &serde_json::to_vec(&offline(doc)).unwrap(),
    );
    let turtle = serialize_triples(RdfFormat::Turtle, &expanded).unwrap();
    let data =
        sparq_core::Graph::load_str(std::str::from_utf8(&turtle).unwrap(), "turtle").unwrap();
    // The upstream shape's JLWS identifiers differ from the pinned W3C profile.
    // Retain every constraint, mapping only vocabulary/profile identifiers.
    let shape = include_str!("fixtures/jlws-storage-description.ttl")
        .replace("https://w3id.org/jeswr/lws#", LWS)
        .replace(
            "https://w3id.org/jeswr/lws/access-profile/odrl-1",
            &format!("{LWS}AccessProfile"),
        );
    let shapes = sparq_core::Graph::load_str(&shape, "turtle").unwrap();
    let report = sparq_shacl::validate(&data, &shapes);
    assert!(report.conforms, "{}", report.to_text());
    // A negative control proves this is a targeted validation, rather than an empty report.
    let invalid = expanded
        .iter()
        .filter(|t| t.predicate.as_str() != "http://purl.org/dc/terms/conformsTo")
        .cloned()
        .collect::<Vec<_>>();
    let invalid = serialize_triples(RdfFormat::Turtle, &invalid).unwrap();
    let invalid =
        sparq_core::Graph::load_str(std::str::from_utf8(&invalid).unwrap(), "turtle").unwrap();
    assert!(!sparq_shacl::validate(&invalid, &shapes).conforms);
    let (status, _, body) = call(
        &app,
        "GET",
        "/",
        &[
            ("accept", "application/lws+cid"),
            ("if-none-match", headers["etag"].to_str().unwrap()),
        ],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}

#[tokio::test]
async fn solid_cached_descriptions_are_stable_per_format() {
    let app = solid(ServiceCatalog::default(), false).await;
    let mut previous: Option<String> = None;
    for media in ["application/ld+json", "text/turtle", "application/ld+json"] {
        let (status, headers, body) =
            call(&app, "GET", "/.well-known/solid", &[("accept", media)], "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], media);
        assert!(!body.is_empty());
        // A cache entry must contain the bytes of its negotiated format.
        if media == "application/ld+json" {
            serde_json::from_slice::<Value>(&body).unwrap();
        } else {
            assert!(!triples(RdfFormat::Turtle, &body).is_empty());
        }
        let (status, again_headers, again_body) =
            call(&app, "GET", "/.well-known/solid", &[("accept", media)], "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again_body, body);
        assert_eq!(again_headers["etag"], headers["etag"]);
        let tag = headers["etag"].to_str().unwrap();
        let (status, cached_headers, cached_body) = call(
            &app,
            "GET",
            "/.well-known/solid",
            &[("accept", media), ("if-none-match", tag)],
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert_eq!(cached_headers["etag"], headers["etag"]);
        assert!(cached_body.is_empty());
        if let Some(other_tag) = previous {
            assert_ne!(tag, other_tag);
            let (status, _, other_body) = call(
                &app,
                "GET",
                "/.well-known/solid",
                &[("accept", media), ("if-none-match", &other_tag)],
                "",
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(other_body, body);
        }
        previous = Some(tag.to_owned());
    }
}

#[tokio::test]
#[should_panic(expected = "render static JSON-LD storage description before boot")]
async fn solid_rejects_an_unrenderable_catalog_at_boot() {
    let mut catalog = extra();
    catalog.service(ServiceEntry {
        fragment: "invalid-service".into(),
        type_term: "urn:test:Service".into(),
        endpoint: "https://pod.example/invalid".into(),
        extras: serde_json::from_value(json!({"@value": "invalid node object"})).unwrap(),
    });
    let _ = solid(catalog, false).await;
}

#[tokio::test]
async fn solid_negotiation_graph_and_goldens() {
    let app = solid(ServiceCatalog::default(), false).await;
    let (status, json_headers, json_body) = call(&app, "GET", "/.well-known/solid", &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_headers["content-type"], "application/ld+json");
    assert_eq!(json_headers["vary"], "Accept");
    let (status, ttl_headers, ttl_body) = call(
        &app,
        "GET",
        "/.well-known/solid",
        &[("accept", "text/turtle")],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ttl_headers["content-type"], "text/turtle");
    assert_eq!(ttl_headers["vary"], "Accept");
    let document: Value = serde_json::from_slice(&json_body).unwrap();
    assert_eq!(
        document["@context"][0],
        "https://www.w3.org/ns/solid/notifications-context/v1"
    );
    assert_eq!(
        document["notificationChannel"],
        json!([{
            "id": "https://pod.example/.notifications/WebSocketChannel2023/",
            "channelType": "http://www.w3.org/ns/solid/notifications#WebSocketChannel2023",
            "subscriptionService": "https://pod.example/.notifications/WebSocketChannel2023/"
        }])
    );
    let graph = triples(
        RdfFormat::JsonLd,
        &serde_json::to_vec(&offline(document)).unwrap(),
    );
    assert_eq!(
        canonical(&graph),
        canonical(&triples(RdfFormat::Turtle, &ttl_body))
    );
    let suffix = if cfg!(feature = "sparql-endpoint") {
        ""
    } else {
        "-no-sparql"
    };
    assert_eq!(
        canonical(&graph),
        canonical(&triples(
            RdfFormat::JsonLd,
            &serde_json::to_vec(&offline(
                serde_json::from_slice(&fixture(&format!("solid{suffix}.jsonld"))).unwrap()
            ))
            .unwrap()
        ))
    );
    assert_eq!(
        canonical(&graph),
        canonical(&triples(
            RdfFormat::Turtle,
            &fixture(&format!("solid{suffix}.ttl"))
        ))
    );
    assert_service(&graph, "storage-root", &format!("{LWS}StorageRoot"), ROOT);
    assert_service(
        &graph,
        "storage-description",
        &format!("{LWS}StorageDescription"),
        "https://pod.example/.well-known/solid",
    );
    assert_service(
        &graph,
        "notifications",
        &format!("{SSD}SolidNotificationService"),
        "https://pod.example/.notifications/WebSocketChannel2023/",
    );
    assert!(has(
        &graph,
        &format!("{ROOT}#notifications"),
        &format!("{SSD}channelType"),
        "http://www.w3.org/ns/solid/notifications#WebSocketChannel2023"
    ));
    for capability in ["PatchSupport", "AccessControl"] {
        assert!(graph.iter().any(|t| t.predicate.as_str() == RDF_TYPE
            && t.object
                == oxrdf::Term::NamedNode(NamedNode::new(format!("{SSD}{capability}")).unwrap())));
    }
    for absent in [
        format!("{LWS}MoveResource"),
        format!("{SSD}RecursiveDelete"),
    ] {
        assert!(!graph.iter().any(|t| t.predicate.as_str() == RDF_TYPE
            && t.object == oxrdf::Term::NamedNode(NamedNode::new(&absent).unwrap())));
    }
    assert_eq!(
        has(&graph, ROOT, ACQ, "https://pod.example/sparql"),
        cfg!(feature = "sparql-endpoint")
    );
    if cfg!(feature = "sparql-endpoint") {
        assert_service(
            &graph,
            "sparql",
            &format!("{SSD}SparqlQueryService"),
            "https://pod.example/sparql",
        );
    } else {
        assert!(!graph.iter().any(|t| t.predicate.as_str() == RDF_TYPE
            && t.object
                == oxrdf::Term::NamedNode(
                    NamedNode::new(format!("{SSD}SparqlQueryService")).unwrap()
                )));
    }
    for accept in ["*/*", "text/turtle;q=0.3, application/ld+json;q=0.8"] {
        assert_eq!(
            call(&app, "GET", "/.well-known/solid", &[("accept", accept)], "")
                .await
                .2,
            json_body
        );
    }
    for accept in [
        "image/png",
        "application/lws+cid",
        "application/ld+json;q=0, text/turtle;q=0",
    ] {
        let (status, headers, _) =
            call(&app, "GET", "/.well-known/solid", &[("accept", accept)], "").await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE, "{accept}");
        assert_eq!(headers["vary"], "Accept");
    }
    let (status, headers, body) = call(&app, "HEAD", "/.well-known/solid", &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["etag"], json_headers["etag"]);
    assert!(body.is_empty());
    let (status, _, body) = call(
        &app,
        "GET",
        "/.well-known/solid",
        &[("if-none-match", json_headers["etag"].to_str().unwrap())],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    let (status, _, body) = call(
        &app,
        "GET",
        "/.well-known/solid",
        &[
            ("if-match", "\"different-catalog\""),
            ("if-none-match", json_headers["etag"].to_str().unwrap()),
        ],
        "",
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert!(body.is_empty());
    assert_ne!(ttl_headers["etag"], json_headers["etag"]);
    // Public discovery ignores credentials and resource visibility.
    assert_eq!(
        call(
            &app,
            "GET",
            "/.well-known/solid",
            &[("authorization", "DPoP invalid")],
            ""
        )
        .await
        .2,
        json_body
    );
    let public = solid(ServiceCatalog::default(), true).await;
    let (query_status, _, _) = call(
        &public,
        "GET",
        "/sparql?query=ASK%7B%7D",
        &[("accept", "application/sparql-results+json")],
        "",
    )
    .await;
    assert_eq!(
        query_status,
        if cfg!(feature = "sparql-endpoint") {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        }
    );
    assert_eq!(
        call(&public, "GET", "/.well-known/solid", &[], "").await.2,
        json_body
    );
}

#[tokio::test]
async fn root_query_link_tracks_mounted_feature_for_get_and_head() {
    for public in [false, true] {
        let app = solid(ServiceCatalog::default(), public).await;
        for method in ["GET", "HEAD"] {
            let (status, headers, _) = call(&app, method, "/", &[], "").await;
            assert_eq!(
                status,
                if public {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
            let links = headers
                .get_all("link")
                .iter()
                .map(|h| h.to_str().unwrap())
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(links.contains("<https://pod.example/sparql>; rel=\"http://www.w3.org/ns/solid/sparql#endpoint\""), cfg!(feature = "sparql-endpoint"));
        }
    }
}

#[tokio::test]
async fn notification_discovery_triples_are_preserved() {
    let app = solid(ServiceCatalog::default(), false).await;
    for (accept, format) in [
        ("text/turtle", RdfFormat::Turtle),
        ("application/ld+json", RdfFormat::JsonLd),
    ] {
        let (_, _, body) = call(&app, "GET", "/.well-known/solid", &[("accept", accept)], "").await;
        let body = if format == RdfFormat::JsonLd {
            serde_json::to_vec(&offline(serde_json::from_slice(&body).unwrap())).unwrap()
        } else {
            body
        };
        let graph = triples(format, &body);
        assert!(!graph.is_empty(), "discovery graph must not be empty");
        let notify = "http://www.w3.org/ns/solid/notifications#";
        let endpoint = "https://pod.example/.notifications/WebSocketChannel2023/";
        assert!(has(
            &graph,
            ROOT,
            &format!("{notify}subscription"),
            endpoint
        ));
        assert!(has(
            &graph,
            endpoint,
            &format!("{notify}channelType"),
            &format!("{notify}WebSocketChannel2023")
        ));
        assert!(has(
            &graph,
            endpoint,
            &format!("{notify}subscriptionService"),
            endpoint
        ));
        assert!(has(
            &graph,
            &format!("{ROOT}#notifications"),
            &format!("{SSD}channelType"),
            "http://www.w3.org/ns/solid/notifications#WebSocketChannel2023"
        ));
    }
}

fn extra() -> ServiceCatalog {
    let mut catalog = ServiceCatalog::default();
    catalog.service(ServiceEntry {
        fragment: "test-service".into(),
        type_term: "urn:test:Service".into(),
        endpoint: "https://pod.example/test-service".into(),
        extras: serde_json::from_value(json!({"urn:test:property": "custom"})).unwrap(),
    });
    catalog
}

#[tokio::test]
async fn extra_registration_reaches_both_descriptions_and_changes_etags() {
    for is_lws in [false, true] {
        let original = if is_lws {
            lws(ServiceCatalog::default(), false).await
        } else {
            solid(ServiceCatalog::default(), false).await
        };
        let app = if is_lws {
            lws(extra(), false).await
        } else {
            solid(extra(), false).await
        };
        // Match registration with an actual mounted test endpoint.
        let app = app.route("/test-service", axum::routing::get(|| async { "test" }));
        assert_eq!(
            call(&app, "GET", "/test-service", &[], "").await.0,
            StatusCode::OK
        );
        let (path, accept) = if is_lws {
            ("/", "application/lws+cid")
        } else {
            ("/.well-known/solid", "application/ld+json")
        };
        let (_, old_headers, _) = call(&original, "GET", path, &[("accept", accept)], "").await;
        let (status, headers, body) = call(&app, "GET", path, &[("accept", accept)], "").await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(old_headers["etag"], headers["etag"]);
        let mut doc: Value = serde_json::from_slice(&body).unwrap();
        doc = offline(doc);
        let graph = triples(RdfFormat::JsonLd, &serde_json::to_vec(&doc).unwrap());
        assert_service(
            &graph,
            "test-service",
            "urn:test:Service",
            "https://pod.example/test-service",
        );
        assert!(graph
            .iter()
            .any(|t| t.predicate.as_str() == "urn:test:property"
                && t.object == oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal("custom"))));
    }
}

#[tokio::test]
async fn advertised_patch_and_recursive_delete_operations_are_served() {
    let app = lws(ServiceCatalog::default(), true).await;
    for media in [
        "application/json",
        "application/ld+json",
        "application/vnd.test+json",
        "text/vnd.test+json",
    ] {
        let (status, headers, _) = call(
            &app,
            "POST",
            "/",
            &[("content-type", media)],
            "{\"value\":1}",
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{media}");
        let path = headers["location"]
            .to_str()
            .unwrap()
            .strip_prefix(common::BASE_URL)
            .unwrap();
        let (status, _, _) = call(
            &app,
            "PATCH",
            path,
            &[("content-type", "application/json-patch+json")],
            "[{\"op\":\"replace\",\"path\":\"/value\",\"value\":2}]",
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{media}");
        let (status, headers, _) = call(&app, "OPTIONS", path, &[], "").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(headers["accept-patch"], "application/json-patch+json");
        let (_, _, body) = call(&app, "GET", path, &[], "").await;
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["value"], 2);
        let (status, _, _) = call(
            &app,
            "PATCH",
            &format!("{path}.meta"),
            &[("content-type", "application/json-patch+json")],
            "[]",
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let (status, headers, _) = call(
        &app,
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
    let container = headers["location"]
        .to_str()
        .unwrap()
        .strip_prefix(common::BASE_URL)
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            container,
            &[("content-type", "text/plain")],
            "child"
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(&app, "DELETE", container, &[("depth", "infinity")], "")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", container, &[], "").await.0,
        StatusCode::NOT_FOUND
    );

    let app = solid(ServiceCatalog::default(), true).await;
    let (status, headers, _) = call(&app, "HEAD", "/", &[], "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["accept-patch"],
        "text/n3, application/sparql-update"
    );
}

#[cfg(not(feature = "sparql-endpoint"))]
#[tokio::test]
async fn lws_query_route_is_absent_without_its_feature() {
    let app = lws(ServiceCatalog::default(), true).await;
    let (status, _, _) = call(&app, "GET", "/.lws/sparql?query=ASK%7B%7D", &[], "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

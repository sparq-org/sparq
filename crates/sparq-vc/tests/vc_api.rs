#![cfg(feature = "vc-api")]

use std::collections::BTreeMap;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use rand_core::RngCore as _;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256, Sha384};
use sparq_vc::{
    api::{router, OfflineContexts, Service, CONTEXT_FILES},
    SigningKey,
};
use tower::ServiceExt as _;

fn contexts() -> OfflineContexts {
    let documents = CONTEXT_FILES
        .iter()
        .map(|(url, _)| {
            let text = if url.contains("examples") {
                include_str!("api-fixtures/examples.jsonld")
            } else {
                include_str!("api-fixtures/base.jsonld")
            };
            let mut document: Value = serde_json::from_str(text).unwrap();
            if *url == "https://www.w3.org/2018/credentials/v1" {
                document["@context"]
                    .as_object_mut()
                    .unwrap()
                    .remove("DataIntegrityProof");
            } else if url.contains("data-integrity") {
                document = json!({"@context": {
                    "@protected":true,"id":"@id","type":"@type",
                    "DataIntegrityProof":document["@context"]["DataIntegrityProof"]
                }});
            }
            let text = if *url == "https://www.w3.org/2018/credentials/v1"
                || url.contains("data-integrity")
            {
                document.to_string()
            } else {
                text.into()
            };
            ((*url).into(), text)
        })
        .collect::<BTreeMap<_, _>>();
    OfflineContexts::from_documents(documents).unwrap()
}

fn app() -> Router {
    let mut seed = [0; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    router(Service::new(&seed, contexts()))
}

fn credential() -> Value {
    json!({"@context": ["https://www.w3.org/ns/credentials/v2"],
        "type": ["VerifiableCredential"],
        "credentialSubject": {"id": "did:example:alice", "name": "Alice"}})
}

async fn post(app: &Router, path: &str, value: Value) -> (StatusCode, Value) {
    request(app, "POST", path, value.to_string(), "application/json").await
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    body: String,
    content_type: &str,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["content-type"], "application/json");
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn issue(app: &Router, vc: Value) -> Value {
    let (status, result) = post(app, "/credentials/issue", json!({"credential":vc})).await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    let vc = result["verifiableCredential"].clone();
    assert!(vc.is_object());
    assert_eq!(vc["proof"]["type"], "DataIntegrityProof");
    assert_eq!(vc["proof"]["cryptosuite"], "eddsa-rdfc-2022");
    assert_eq!(vc["proof"]["proofPurpose"], "assertionMethod");
    assert!(vc["proof"]["proofValue"].as_str().unwrap().starts_with('z'));
    vc
}

fn rejected(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["verified"], false);
    assert!(!body["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn issue_verify_round_trip_and_tamper_detection() {
    let app = app();
    let vc = issue(&app, credential()).await;
    let (status, result) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc,
        "options":{"checks":["proof"]}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        result,
        json!({"verified":true,"checks":["proof"],"warnings":[],"errors":[]})
    );
    let mut tampered = vc.clone();
    tampered["credentialSubject"]["name"] = "Mallory".into();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":tampered}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(body["errors"][0]["message"], "proof verification failed");
    let mut reordered = vc.clone();
    reordered["proof"] = json!([vc["proof"]]);
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":reordered}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn jsonld_hashing_matches_the_existing_rdf_cryptosuite() {
    use oxrdf::{Literal, NamedNode, Term, Triple};
    use sparq_vc::{did::DidKeyResolver, DataIntegrityProof, ProofConfig};
    let app = app();
    let mut input = credential();
    input["id"] = "urn:vc-api:independent-vector".into();
    let vc = issue(&app, input).await;
    let node = |s: &str| NamedNode::new(s).unwrap();
    let root = node("urn:vc-api:independent-vector");
    let subject = node("did:example:alice");
    let predicate = |s: &str| node(&format!("https://www.w3.org/2018/credentials#{s}"));
    let mut triples = vec![
        Triple::new(
            root.clone(),
            node("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            node("https://www.w3.org/2018/credentials#VerifiableCredential"),
        ),
        Triple::new(
            root.clone(),
            predicate("credentialSubject"),
            subject.clone(),
        ),
        Triple::new(
            root,
            predicate("issuer"),
            node(vc["issuer"].as_str().unwrap()),
        ),
        Triple::new(
            subject,
            node("https://schema.org/name"),
            Term::Literal(Literal::new_simple_literal("Alice")),
        ),
    ];
    let config = ProofConfig::new(vc["proof"]["verificationMethod"].as_str().unwrap())
        .with_created(vc["proof"]["created"].as_str().unwrap());
    let proof = DataIntegrityProof {
        proof_type: "DataIntegrityProof".into(),
        cryptosuite: "eddsa-rdfc-2022".into(),
        config,
        proof_value: vc["proof"]["proofValue"].as_str().unwrap().into(),
    };
    sparq_vc::verify(&triples, &proof, &DidKeyResolver)
        .expect("HTTP JSON-LD signature verifies through RDF API");
    // Verify an independently signed credential whose issuer is unrelated to
    // this server's own key. No network DID resolution is involved.
    let external_key = SigningKey::generate();
    let did = external_key.did_key();
    let config = ProofConfig::new(format!("{did}#{}", did.strip_prefix("did:key:").unwrap()));
    triples[2].object = node(&did).into();
    let proof = sparq_vc::sign(&triples, &external_key, &config).unwrap();
    let mut external = credential();
    external["id"] = "urn:vc-api:independent-vector".into();
    external["issuer"] = did.into();
    external["proof"] = json!({"type":"DataIntegrityProof","cryptosuite":"eddsa-rdfc-2022",
        "verificationMethod":config.verification_method,"proofPurpose":"assertionMethod",
        "proofValue":proof.proof_value});
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":external}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn data_model_suite_fixtures_on_issue_and_verify() {
    let app = app();
    let valid = issue(&app, credential()).await;
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("api-fixtures/data-model.json")).unwrap();
    assert_eq!(fixtures.len(), 92);
    for fixture in fixtures {
        let name = fixture["name"].as_str().unwrap();
        let mut vc = fixture["credential"].clone();
        // Mirror mock.data.js createRequestBody's issuer injection, retaining
        // explicit invalid values (including null) for the negative cases.
        if vc.get("issuer").is_none() {
            vc["issuer"] = valid["issuer"].clone();
        }
        if vc["issuer"].is_object() && vc["issuer"].get("id").is_none() {
            vc["issuer"]["id"] = valid["issuer"].clone();
        }
        let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc})).await;
        if fixture["valid"] == true {
            assert_eq!(status, StatusCode::CREATED, "{name}: {body}");
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":body["verifiableCredential"]}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{name}: {body}");
            assert_eq!(body["verified"], true, "{name}");
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
            rejected(status, &body);
            // An invalid model must be rejected before signature checking. Pin
            // the validation error too, so a crypto failure cannot mask a
            // missing verifier validation call.
            vc["proof"] = valid["proof"].clone();
            let (verify_status, verify_body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":vc}),
            )
            .await;
            rejected(verify_status, &verify_body);
            assert_eq!(
                verify_body["errors"], body["errors"],
                "{name}: verifier validation differs"
            );
        }
    }
}

#[tokio::test]
async fn proof_errors_and_requested_options() {
    let app = app();
    let vc = issue(&app, credential()).await;
    for (field, value) in [
        ("type", json!("UnknownProofType")),
        ("cryptosuite", json!("eddsa-jcs-2022")),
        ("proofPurpose", json!("authentication")),
        ("created", json!("2023-02-29T00:00:00Z")),
        ("verificationMethod", json!("did:key:@invalidVm@")),
        ("proofValue", json!("z123")),
        ("domain", json!(123)),
        ("challenge", json!(false)),
        ("previousProof", json!("urn:proof:1")),
        ("id", Value::Null),
        ("id", json!("relative-proof-id")),
    ] {
        let mut changed = vc.clone();
        changed["proof"][field] = value;
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":changed}),
        )
        .await;
        rejected(status, &body);
    }
    for field in [
        "type",
        "cryptosuite",
        "proofPurpose",
        "verificationMethod",
        "proofValue",
    ] {
        let mut changed = vc.clone();
        changed["proof"].as_object_mut().unwrap().remove(field);
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":changed}),
        )
        .await;
        rejected(status, &body);
    }
    for proof in [
        Value::Null,
        json!([]),
        json!([vc["proof"], {}]),
        json!("proof"),
    ] {
        let mut changed = vc.clone();
        changed["proof"] = proof;
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":changed}),
        )
        .await;
        rejected(status, &body);
    }
    let options =
        json!({"created":"2023-02-24T23:36:38Z","domain":"example.org","challenge":"test"});
    let (status, body) = post(
        &app,
        "/credentials/issue",
        json!({"credential":credential(),"options":options}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let bound = body["verifiableCredential"].clone();
    for (challenge, expected) in [("test", StatusCode::OK), ("other", StatusCode::BAD_REQUEST)] {
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":bound,
            "options":{"domain":"example.org","challenge":challenge}}),
        )
        .await;
        assert_eq!(status, expected, "{body}");
    }
    let other_key = SigningKey::generate();
    let mut changed = vc;
    changed["issuer"] = other_key.did_key().into();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":changed}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(
        body["errors"][0]["message"],
        "verificationMethod controller must be the did:key issuer"
    );
}

#[tokio::test]
async fn offline_contexts_include_nested_references_and_protected_terms() {
    let app = app();
    for context in [
        json!("https://unavailable.example/context"),
        json!({"@import":"https://unavailable.example/context"}),
        json!({"name":{"@id":"https://schema.org/name","@context":"https://unavailable.example/context"}}),
    ] {
        let mut vc = credential();
        vc["@context"].as_array_mut().unwrap().push(context);
        let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc})).await;
        rejected(status, &body);
    }
    let (status, doc) = request(&app, "GET", "/did.json", String::new(), "application/json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["testOnly"], true);
    assert!(doc["verificationMethod"][0]
        .get("secretKeyMultibase")
        .is_none());
    assert!(OfflineContexts::from_documents(BTreeMap::new()).is_err());
    assert!(
        OfflineContexts::from_directory(std::path::Path::new("/nonexistent/vc-api-cache")).is_err()
    );
}

#[tokio::test]
async fn related_resource_shape_and_cached_byte_digests() {
    let app = app();
    let bytes = include_str!("api-fixtures/base.jsonld").as_bytes();
    let resource = json!({"id":"https://www.w3.org/ns/credentials/v2",
        "digestSRI":format!("sha384-{}", STANDARD.encode(Sha384::digest(bytes))),
        "digestMultibase":format!("u{}", URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)))});
    let mut vc = credential();
    vc["relatedResource"] = resource.clone();
    issue(&app, vc).await;
    for resources in [
        json!([resource["id"]]),
        json!({"digestSRI":resource["digestSRI"]}),
        json!({"id":resource["id"]}),
        json!([resource, resource]),
        json!({"id":resource["id"],"digestSRI":"sha384-invalid"}),
        json!({"id":resource["id"],"digestMultibase":"uInvalid"}),
        json!({"id":"https://unavailable.example/resource","digestSRI":resource["digestSRI"]}),
    ] {
        let mut vc = credential();
        vc["relatedResource"] = resources;
        let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc})).await;
        rejected(status, &body);
    }
}

#[tokio::test]
async fn malformed_requests_and_unsupported_options() {
    let app = app();
    for body in ["{", "[]", "null", "{}", "{\"credential\":null}"] {
        let (status, body) = request(
            &app,
            "POST",
            "/credentials/issue",
            body.into(),
            "application/json",
        )
        .await;
        rejected(status, &body);
    }
    let (status, body) = request(
        &app,
        "POST",
        "/credentials/issue",
        "{}".into(),
        "text/plain",
    )
    .await;
    rejected(status, &body);
    let (status, body) = request(
        &app,
        "POST",
        "/credentials/issue",
        " ".repeat(256 * 1024 + 1),
        "application/json",
    )
    .await;
    rejected(status, &body);
    for options in [
        json!(null),
        json!([]),
        json!({"cryptosuite":"unknown"}),
        json!({"type":"unknown"}),
        json!({"proofPurpose":"authentication"}),
        json!({"created":"2023-02-29T00:00:00Z"}),
        json!({"verificationMethod":"did:key:other"}),
        json!({"unknown":true}),
        json!({"domain":5}),
    ] {
        let (status, body) = post(
            &app,
            "/credentials/issue",
            json!({"credential":credential(),"options":options}),
        )
        .await;
        rejected(status, &body);
    }
    let vc = issue(&app, credential()).await;
    for options in [
        json!({"checks":["status"]}),
        json!({"checks":[]}),
        json!({"created":"2023-01-01T00:00:00Z"}),
    ] {
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":vc,"options":options}),
        )
        .await;
        rejected(status, &body);
    }
}

#[tokio::test]
async fn unmapped_claims_are_rejected_and_named_graph_claims_are_bound() {
    let app = app();
    let mut vc = credential();
    vc["credentialSubject"]["unmappedClaim"] = "unmapped".into();
    let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc})).await;
    rejected(status, &body);
    assert_eq!(body["errors"][0]["message"], "unmapped JSON-LD property");
    let mut vc = credential();
    vc["credentialSubject"]["@graph"] = json!([{"id":"urn:graph:subject","name":"Graph claim"}]);
    let mut vc = issue(&app, vc).await;
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    vc["credentialSubject"]["@graph"][0]["name"] = "Changed graph claim".into();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    rejected(status, &body);
}

#[tokio::test]
async fn vc_v1_adds_the_data_integrity_context() {
    let app = app();
    let mut vc = credential();
    vc["@context"] = json!(["https://www.w3.org/2018/credentials/v1"]);
    vc["issuanceDate"] = "2023-01-01T00:00:00Z".into();
    let vc = issue(&app, vc).await;
    assert_eq!(
        vc["@context"][1],
        "https://w3id.org/security/data-integrity/v2"
    );
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut vc = credential();
    vc["@context"] = json!([
        "https://www.w3.org/2018/credentials/v1",
        "https://w3id.org/security/data-integrity/v1"
    ]);
    let vc = issue(&app, vc).await;
    assert_eq!(vc["@context"].as_array().unwrap().len(), 2);
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

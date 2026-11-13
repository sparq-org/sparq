use super::*;
use ed25519_dalek::Signer as _;

const DOMAIN: &str = "github.com/w3c/vc-data-model-2.0-test-suite";
const CHALLENGE: &str = "uTestEndpointsChallenge";

fn context_documents() -> BTreeMap<String, String> {
    let texts = [
        include_str!("../api-fixtures/contexts/credentials-v2.jsonld"),
        include_str!("../api-fixtures/contexts/credentials-v1.jsonld"),
        include_str!("../api-fixtures/contexts/examples-v2.jsonld"),
        include_str!("../api-fixtures/contexts/data-integrity-v2.jsonld"),
        include_str!("../api-fixtures/contexts/data-integrity-v1.jsonld"),
        include_str!("../api-fixtures/contexts/multikey-v1.jsonld"),
        include_str!("../api-fixtures/contexts/did-v1.jsonld"),
    ];
    CONTEXT_FILES
        .iter()
        .zip(texts)
        .map(|((url, _), text)| ((*url).into(), text.into()))
        .collect()
}

fn app() -> Router {
    let mut seed = [0; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    router(Service::new(
        &seed,
        OfflineContexts::from_documents(context_documents()).unwrap(),
    ))
}

fn canonical(value: &Value) -> String {
    // The test signer uses an independent JSON-LD expansion implementation.
    let text = value.to_string();
    let documents = context_documents();
    let quads = oxjsonld::JsonLdParser::new()
        .for_slice(text.as_bytes())
        .with_load_document_callback(move |url, _| {
            Ok(oxjsonld::JsonLdRemoteDocument {
                document: documents
                    .get(url)
                    .expect("static test context")
                    .as_bytes()
                    .to_vec(),
                document_url: url.into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    sparq_canon::canonicalize_quads_with::<Sha256>(&quads).unwrap()
}

struct TestSigner {
    key: ed25519_dalek::SigningKey,
    did: String,
}

impl TestSigner {
    fn new() -> Self {
        let mut seed = [0; 32];
        rand_core::OsRng.fill_bytes(&mut seed);
        Self {
            key: ed25519_dalek::SigningKey::from_bytes(&seed),
            did: SigningKey::from_seed(&seed).did_key(),
        }
    }

    fn proof(&self, purpose: &str) -> Value {
        json!({"type":"DataIntegrityProof","cryptosuite":"eddsa-rdfc-2022",
            "verificationMethod":format!("{}#{}", self.did, self.did.strip_prefix("did:key:").unwrap()),
            "proofPurpose":purpose,"created":"2023-02-26T22:06:38Z"})
    }

    fn sign(&self, document: &Value, mut proof: Value) -> Value {
        let mut config = proof.clone();
        config["@context"] = document["@context"].clone();
        let mut hash = Sha256::digest(canonical(&config).as_bytes()).to_vec();
        hash.extend(Sha256::digest(canonical(document).as_bytes()));
        proof["proofValue"] = format!(
            "z{}",
            bs58::encode(self.key.sign(&hash).to_bytes()).into_string()
        )
        .into();
        proof
    }

    fn presentation(&self, mut vp: Value) -> Value {
        let mut proof = self.proof("authentication");
        proof["domain"] = DOMAIN.into();
        proof["challenge"] = CHALLENGE.into();
        vp["proof"] = self.sign(&vp, proof);
        vp
    }
}

fn presentation() -> Value {
    json!({"@context":["https://www.w3.org/ns/credentials/v2"], "type":["VerifiablePresentation"]})
}

async fn verify_vp(app: &Router, vp: Value) -> (StatusCode, Value) {
    post(
        app,
        "/presentations/verify",
        json!({"verifiablePresentation":vp,
        "options":{"domain":DOMAIN,"challenge":CHALLENGE}}),
    )
    .await
}

fn accepted(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        &json!({"verified":true,"checks":["proof"],"warnings":[],"errors":[]})
    );
}

#[tokio::test]
async fn suite_presentation_requests_verify_all_proofs() {
    let app = app();
    let signer = TestSigner::new();
    let vp = signer.presentation(presentation());
    let (status, body) = verify_vp(&app, vp.clone()).await;
    accepted(status, &body);
    for (field, value, message) in [
        (
            "proofValue",
            json!("z5nmMBw2u9TGKd9NxxHJtyZKgjN2Eh"),
            "invalid proofValue",
        ),
        ("type", json!("UnknownProof"), "unsupported proof type"),
        (
            "cryptosuite",
            json!("eddsa-jcs-2022"),
            "unsupported cryptosuite",
        ),
        (
            "proofPurpose",
            json!("assertionMethod"),
            "unsupported proofPurpose",
        ),
        (
            "challenge",
            json!("other"),
            "proof does not match requested options",
        ),
        (
            "domain",
            json!("other.example"),
            "proof does not match requested options",
        ),
    ] {
        let mut changed = vp.clone();
        changed["proof"][field] = value;
        let (status, body) = verify_vp(&app, changed).await;
        rejected(status, &body);
        assert_eq!(body["message"], message);
    }
    for field in ["domain", "challenge"] {
        let mut options = json!({"domain":DOMAIN,"challenge":CHALLENGE});
        options[field] = "different-request".into();
        let (status, body) = post(
            &app,
            "/presentations/verify",
            json!({"verifiablePresentation":vp,"options":options}),
        )
        .await;
        rejected(status, &body);
        assert_eq!(body["message"], "proof does not match requested options");
    }
    let mut changed = vp.clone();
    changed.as_object_mut().unwrap().remove("proof");
    let (status, body) = verify_vp(&app, changed).await;
    rejected(status, &body);
    assert_eq!(body["message"], "proof is required");

    let mut second = signer.proof("authentication");
    second["domain"] = DOMAIN.into();
    second["challenge"] = CHALLENGE.into();
    second["created"] = "2023-02-26T22:16:38Z".into();
    let second = signer.sign(&presentation(), second);
    let mut set = vp.clone();
    set["proof"] = json!([vp["proof"], second]);
    let (status, body) = verify_vp(&app, set.clone()).await;
    accepted(status, &body);
    set["proof"][1]["proofValue"] = vp["proof"]["proofValue"].clone();
    let (status, body) = verify_vp(&app, set).await;
    rejected(status, &body);
    assert_eq!(body["message"], "proof verification failed");
}

#[tokio::test]
async fn suite_presentation_model_inputs() {
    let app = app();
    let signer = TestSigner::new();
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../api-fixtures/presentations.json")).unwrap();
    for fixture in fixtures {
        let name = fixture["name"].as_str().unwrap();
        // The pinned tests exercise these signed forms, without JWT enveloping
        // or self-asserted credential operations.
        if name.contains("enveloped") || name.contains("self-asserted") {
            continue;
        }
        let mut vp = fixture["presentation"].clone();
        if fixture["valid"] == true {
            if vp.get("holder").is_some() {
                if vp["holder"].is_object() {
                    vp["holder"]["id"] = signer.did.clone().into();
                } else {
                    vp["holder"] = signer.did.clone().into();
                }
            }
            if let Some(credentials) = vp
                .get_mut("verifiableCredential")
                .and_then(Value::as_array_mut)
            {
                for vc in credentials {
                    *vc = issue(&app, vc.clone()).await;
                }
            }
            vp = signer.presentation(vp);
            let (status, body) = verify_vp(&app, vp).await;
            assert_eq!(status, StatusCode::OK, "{name}: {body}");
            accepted(status, &body);
        } else {
            // Model checks precede crypto. A valid signature on another VP
            // must not substitute for the rejected document's model checks.
            vp["proof"] = signer.presentation(presentation())["proof"].clone();
            let (status, body) = verify_vp(&app, vp).await;
            rejected(status, &body);
            assert_ne!(body["message"], "proof verification failed", "{name}");
        }
    }
}

#[tokio::test]
async fn embedded_credentials_use_the_credential_verifier() {
    let app = app();
    let signer = TestSigner::new();
    let vc = issue(&app, credential()).await;
    let mut second_input = credential();
    second_input["credentialSubject"]["id"] = "did:example:bob".into();
    let second = issue(&app, second_input).await;
    for credentials in [json!(vc), json!([vc, second])] {
        let mut vp = presentation();
        vp["verifiableCredential"] = credentials;
        let (status, body) = verify_vp(&app, signer.presentation(vp)).await;
        accepted(status, &body);
    }
    let mut inputs = Vec::new();
    let mut invalid = vc.clone();
    invalid["credentialSubject"]["name"] = "Changed".into();
    inputs.push(invalid);
    let mut invalid = vc.clone();
    invalid.as_object_mut().unwrap().remove("proof");
    inputs.push(invalid);
    let mut invalid = vc.clone();
    invalid["proof"] = json!([vc["proof"], {"type":"UnsupportedProof"}]);
    inputs.push(invalid);
    let mut invalid = vc.clone();
    invalid["issuer"] = signer.did.clone().into();
    inputs.push(invalid);
    let mut invalid = vc.clone();
    invalid["type"] = json!(["https://example.org/Other"]);
    inputs.push(invalid);
    for invalid in inputs {
        let (_, direct) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":invalid}),
        )
        .await;
        let mut vp = presentation();
        vp["verifiableCredential"] = json!([vc, invalid]);
        let (status, body) = verify_vp(&app, signer.presentation(vp)).await;
        rejected(status, &body);
        assert_eq!(body, direct);
    }
    let mut vp = presentation();
    vp["holder"] = TestSigner::new().did.into();
    let (status, body) = verify_vp(&app, signer.presentation(vp)).await;
    rejected(status, &body);
    assert_eq!(
        body["message"],
        "verificationMethod controller must be the did:key issuer or holder"
    );
}

#[tokio::test]
async fn previous_proof_suite_vector_and_dependency_order() {
    let app = app();
    let vc: Value =
        serde_json::from_str(include_str!("../api-fixtures/previous-proof.json")).unwrap();
    for list in [false, true] {
        for reverse in [false, true] {
            let mut document = vc.clone();
            if list {
                document["proof"][1]["previousProof"] = json!([vc["proof"][1]["previousProof"]]);
            }
            if reverse {
                document["proof"].as_array_mut().unwrap().reverse();
            }
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":document}),
            )
            .await;
            accepted(status, &body);
        }
    }
    let mut sibling_context = vc.clone();
    sibling_context["@context"][1]["discarded"] = "https://example.org/discarded".into();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":sibling_context}),
    )
    .await;
    accepted(status, &body);
    let mut changed = vc.clone();
    changed["proof"][0]["proofValue"] = vc["proof"][1]["proofValue"].clone();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":changed}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(body["message"], "proof verification failed");
}

fn chain(signer: &TestSigner, count: usize) -> Value {
    let mut vc = credential();
    vc["issuer"] = signer.did.clone().into();
    let mut proofs = Vec::new();
    for index in 0..count {
        let mut proof = signer.proof("assertionMethod");
        proof["id"] = format!("urn:proof:{index}").into();
        let mut input = vc.clone();
        if index > 0 {
            proof["previousProof"] = format!("urn:proof:{}", index - 1).into();
            input["proof"] = json!([proofs[index - 1]]);
        }
        proofs.push(signer.sign(&input, proof));
    }
    vc["proof"] = proofs.into();
    vc
}

#[tokio::test]
async fn proof_chain_lists_require_every_dependency() {
    let app = app();
    let signer = TestSigner::new();
    let other = TestSigner::new();
    let mut vc = credential();
    vc["issuer"] = signer.did.clone().into();
    let mut first = other.proof("assertionMethod");
    first["id"] = "urn:proof:first".into();
    first = other.sign(&vc, first);
    let mut second = other.proof("assertionMethod");
    second["id"] = "urn:proof:second".into();
    second = other.sign(&vc, second);
    for corrupt in [false, true] {
        let mut previous = first.clone();
        if corrupt {
            previous["proofValue"] = second["proofValue"].clone();
        }
        let mut input = vc.clone();
        input["proof"] = json!([previous, second]);
        let mut terminal = signer.proof("assertionMethod");
        terminal["previousProof"] = json!(["urn:proof:first", "urn:proof:second"]);
        terminal = signer.sign(&input, terminal);
        let mut document = vc.clone();
        document["proof"] = json!([terminal, second, previous]);
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":document}),
        )
        .await;
        if corrupt {
            rejected(status, &body);
            assert_eq!(body["message"], "proof verification failed");
        } else {
            accepted(status, &body);
        }
    }
    // Pin dependency processing order using distinct outcomes for the two proofs.
    let mut ordered = vc.clone();
    ordered["issuer"] = "did:key:zInvalid".into();
    let mut terminal = signer.proof("assertionMethod");
    terminal["verificationMethod"] = "did:key:zInvalid#zInvalid".into();
    terminal["previousProof"] = "urn:proof:first".into();
    ordered["proof"] = json!([terminal, first]);
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":ordered}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(body["message"], "proof verification failed");
    // An independent proof set still requires an issuer proof for each member.
    let mut document = vc.clone();
    document["proof"] = first;
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":document}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(
        body["message"],
        "verificationMethod controller must be the did:key issuer or holder"
    );
}

#[tokio::test]
async fn proof_reference_errors_are_explicit() {
    let app = app();
    let signer = TestSigner::new();
    let vc = chain(&signer, 2);
    let mut cases = Vec::new();
    for (value, expected) in [
        (json!("urn:proof:missing"), "unknown previousProof id"),
        (json!("urn:proof:1"), "previousProof self-reference"),
        (json!([]), "previousProof must not be empty"),
        (json!(null), "previousProof must contain strings"),
        (json!([1]), "previousProof must contain strings"),
        (
            json!(["urn:proof:0", "urn:proof:0"]),
            "duplicate previousProof reference",
        ),
        (
            json!(vec!["urn:proof:0"; 65]),
            "previousProof count exceeds 64",
        ),
    ] {
        let mut changed = vc.clone();
        changed["proof"][1]["previousProof"] = value;
        cases.push((changed, expected));
    }
    let mut changed = vc.clone();
    changed["proof"][0].as_object_mut().unwrap().remove("id");
    cases.push((changed, "unknown previousProof id"));
    let mut changed = vc.clone();
    changed["proof"][0]["previousProof"] = "urn:proof:1".into();
    cases.push((changed, "cyclic previousProof references"));
    let mut changed = vc.clone();
    changed["proof"][1]["id"] = "urn:proof:0".into();
    cases.push((changed, "duplicate proof id"));
    for (changed, expected) in cases {
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":changed}),
        )
        .await;
        rejected(status, &body);
        assert_eq!(body["message"], expected);
    }
}

#[tokio::test]
async fn proof_count_and_chain_depth_limits() {
    let app = app();
    let signer = TestSigner::new();
    for (count, expected) in [(16, None), (17, Some("proof chain depth exceeds 16"))] {
        let vc = chain(&signer, count);
        for reverse in [false, true] {
            let mut vc = vc.clone();
            if reverse {
                vc["proof"].as_array_mut().unwrap().reverse();
            }
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":vc}),
            )
            .await;
            if let Some(expected) = expected {
                rejected(status, &body);
                assert_eq!(body["message"], expected);
            } else {
                accepted(status, &body);
            }
        }
    }
    let mut vc = credential();
    vc["issuer"] = signer.did.clone().into();
    let proofs: Vec<_> = (0..65)
        .map(|i| {
            let mut proof = signer.proof("assertionMethod");
            proof["id"] = format!("urn:set:{i}").into();
            signer.sign(&vc, proof)
        })
        .collect();
    for count in [64, 65] {
        let mut vc = vc.clone();
        vc["proof"] = json!(proofs[..count]);
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":vc}),
        )
        .await;
        if count == 64 {
            accepted(status, &body);
        } else {
            rejected(status, &body);
            assert_eq!(body["message"], "proof count exceeds 64");
        }
    }
}

#[tokio::test]
async fn verification_routes_share_request_errors_and_limits() {
    let app = app();
    for path in [
        "/credentials/issue",
        "/credentials/verify",
        "/presentations/verify",
    ] {
        for body in [
            "{".into(),
            "null".into(),
            "[]".into(),
            " ".repeat(256 * 1024 + 1),
            format!("{}0{}", "[".repeat(65), "]".repeat(65)),
        ] {
            let (status, body) = request(&app, "POST", path, body, "application/json").await;
            rejected(status, &body);
            assert!(!body["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("[object Object]"));
        }
        let (status, body) = request(&app, "POST", path, "{}".into(), "text/plain").await;
        rejected(status, &body);
    }
    let signer = TestSigner::new();
    let vp = signer.presentation(presentation());
    for (path, field, document) in [
        ("/credentials/issue", "credential", credential()),
        (
            "/credentials/verify",
            "verifiableCredential",
            issue(&app, credential()).await,
        ),
        (
            "/presentations/verify",
            "verifiablePresentation",
            vp.clone(),
        ),
    ] {
        let mut oversized = json!({field:document});
        oversized["padding"] = "x".repeat(256 * 1024).into();
        let (status, body) = post(&app, path, oversized).await;
        rejected(status, &body);
        assert_eq!(
            body["message"],
            "expected a JSON request body within 256 KiB"
        );
        for levels in [63, 64] {
            let mut nested = json!(0);
            for _ in 0..levels {
                nested = json!([nested]);
            }
            let mut input = json!({field:document});
            input["depth"] = nested;
            let (status, body) = post(&app, path, input).await;
            if levels == 63 {
                assert!(status.is_success(), "{body}");
            } else {
                rejected(status, &body);
                assert_eq!(body["message"], "JSON nesting depth exceeds 64");
            }
        }
    }
    for option in [
        json!(null),
        json!({"checks":[]}),
        json!({"challenge":5}),
        json!({"proofPurpose":"assertionMethod"}),
        json!({"unknown":"proof"}),
    ] {
        let (status, body) = post(
            &app,
            "/presentations/verify",
            json!({"verifiablePresentation":vp,"options":option}),
        )
        .await;
        rejected(status, &body);
    }
    let vc = issue(&app, credential()).await;
    for count in [32, 33] {
        let mut vp = presentation();
        let mut credentials = Vec::new();
        for i in 0..count {
            let mut input = credential();
            input["id"] = format!("urn:credential:{i}").into();
            credentials.push(issue(&app, input).await);
        }
        vp["verifiableCredential"] = credentials.into();
        let (status, body) = verify_vp(&app, signer.presentation(vp)).await;
        if count == 32 {
            accepted(status, &body);
        } else {
            rejected(status, &body);
            assert_eq!(body["message"], "presentation exceeds 32 credentials");
        }
    }
    for value in [
        json!([]),
        json!([vc, "https://example.org/credential"]),
        json!(null),
    ] {
        let mut vp = presentation();
        vp["verifiableCredential"] = value;
        let (status, body) = verify_vp(&app, vp).await;
        rejected(status, &body);
        assert_eq!(body["message"], "expected one or more objects");
    }
}

#[tokio::test]
async fn route_errors_use_the_same_readable_problem_shape() {
    let app = app();
    for (method, path, expected, message) in [
        (
            "GET",
            "/credentials/issue",
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ),
        (
            "GET",
            "/credentials/verify",
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ),
        (
            "GET",
            "/presentations/verify",
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ),
        (
            "POST",
            "/did.json",
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ),
        ("POST", "/unknown", StatusCode::NOT_FOUND, "route not found"),
    ] {
        let (status, body) = request(&app, method, path, "{}".into(), "application/json").await;
        assert_eq!(status, expected);
        assert_eq!(body["message"], message);
        assert_eq!(body["errors"][0]["message"], message);
        assert_eq!(body["errors"][0]["detail"], message);
        assert_eq!(body["errors"][0]["status"], expected.as_u16());
        assert_eq!(body["errors"][0]["title"], "ValidationError");
        assert_eq!(
            body["type"],
            "https://sparq.jeswr.org/ns/vc-api#ValidationError"
        );
        assert_eq!(body["errors"][0]["type"], body["type"]);
        assert_eq!(body["verified"], false);
    }
}

#[tokio::test]
async fn held_resources_check_every_cached_document_and_digest() {
    let mut documents = context_documents();
    documents.insert("urn:held:fixture".into(), "fixture-held bytes\n".into());
    let mut seed = [0; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let app = router(Service::new(
        &seed,
        OfflineContexts::from_documents(documents.clone()).unwrap(),
    ));
    let signer = TestSigner::new();
    for (id, bytes) in &documents {
        for (field, matching, wrong, message) in [
            (
                "digestSRI",
                format!(
                    "sha256-{}",
                    STANDARD.encode(Sha256::digest(bytes.as_bytes()))
                ),
                format!("sha256-{}", STANDARD.encode([0; 32])),
                "relatedResource digestSRI mismatch for held resource",
            ),
            (
                "digestSRI",
                format!(
                    "sha384-{}",
                    STANDARD.encode(Sha384::digest(bytes.as_bytes()))
                ),
                format!("sha384-{}", STANDARD.encode([0; 48])),
                "relatedResource digestSRI mismatch for held resource",
            ),
            (
                "digestSRI",
                format!(
                    "sha512-{}",
                    STANDARD.encode(Sha512::digest(bytes.as_bytes()))
                ),
                format!("sha512-{}", STANDARD.encode([0; 64])),
                "relatedResource digestSRI mismatch for held resource",
            ),
            (
                "digestMultibase",
                format!(
                    "u{}",
                    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes.as_bytes()))
                ),
                format!("u{}", URL_SAFE_NO_PAD.encode([0; 32])),
                "relatedResource digestMultibase mismatch for held resource",
            ),
        ] {
            let mut input = credential();
            input["relatedResource"] =
                json!({"id":id, field:matching, "mediaType":"application/ld+json"});
            let signed = issue(&app, input.clone()).await;
            assert_eq!(signed["relatedResource"], input["relatedResource"]);
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":signed}),
            )
            .await;
            accepted(status, &body);
            input["relatedResource"][field] = wrong.clone().into();
            let (status, body) =
                post(&app, "/credentials/issue", json!({"credential":input})).await;
            rejected(status, &body);
            assert_eq!(body["message"], message, "{id}");
            input["issuer"] = signer.did.clone().into();
            input["proof"] = signer.sign(&input, signer.proof("assertionMethod"));
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":input}),
            )
            .await;
            rejected(status, &body);
            assert_eq!(body["message"], message, "{id}");
            let mut unknown = credential();
            unknown["relatedResource"] =
                json!({"id":"https://unavailable.example/resource", field:wrong});
            let signed = issue(&app, unknown.clone()).await;
            assert_eq!(signed["relatedResource"], unknown["relatedResource"]);
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":signed}),
            )
            .await;
            accepted(status, &body);
        }
    }
    // Both advertised digests must match, even when the other one is correct.
    for field in ["digestSRI", "digestMultibase"] {
        let mut input = credential();
        let bytes = documents["https://www.w3.org/ns/credentials/v2"].as_bytes();
        input["relatedResource"] = json!({"id":"https://www.w3.org/ns/credentials/v2",
            "digestSRI": format!("sha384-{}", STANDARD.encode(Sha384::digest(bytes))),
            "digestMultibase": format!("u{}", URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)))});
        input["relatedResource"][field] = if field == "digestSRI" {
            format!("sha384-{}", STANDARD.encode([0; 48]))
        } else {
            format!("u{}", URL_SAFE_NO_PAD.encode([0; 32]))
        }
        .into();
        let (status, body) = post(&app, "/credentials/issue", json!({"credential":input})).await;
        rejected(status, &body);
        assert!(body["message"]
            .as_str()
            .unwrap()
            .contains("mismatch for held resource"));
    }
}

#[tokio::test]
async fn github_context_bytes_match_the_suite_digests_and_preserve_jsonld() {
    let app = app();
    for resource in [
        json!({"id":"https://www.w3.org/ns/credentials/v2", "digestSRI":"sha384-l/HrjlBCNWyAX91hr6LFV2Y3heB5Tcr6IeE4/Tje8YyzYBM8IhqjHWiWpr8+ZbYU"}),
        json!({"id":"https://www.w3.org/ns/credentials/v2", "digestMultibase":"uWZVc7WaX1h4D8rJVb-vlMIqxaEKEb1tYbX8fet7JJzQ"}),
    ] {
        let mut vc = credential();
        vc["relatedResource"] = resource;
        issue(&app, vc).await;
    }
    let mut vc = credential();
    vc["relatedResource"] = json!({"id":"https://www.w3.org/ns/credentials/v2", "digestMultibase":"uM4RgWQc3RUDtjJCSgTJtTfvpZ7SPEg_LNO0ESlovQC0"});
    let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc})).await;
    rejected(status, &body);
    assert_eq!(
        body["message"],
        "relatedResource digestMultibase mismatch for held resource"
    );
    let documents = context_documents();
    let manifest: Value =
        serde_json::from_str(include_str!("../api-fixtures/contexts/manifest.json")).unwrap();
    for (url, filename) in CONTEXT_FILES {
        assert_eq!(
            hex::encode(Sha256::digest(documents[*url].as_bytes())),
            manifest[*filename]["sha256"]
        );
    }
    let reformatted: BTreeMap<_, _> = documents
        .iter()
        .map(|(url, text)| {
            let value: Value = serde_json::from_str(text).unwrap();
            (url.clone(), serde_json::to_string(&value).unwrap())
        })
        .collect();
    let mut seed = [0; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let reformatted_app = router(Service::new(
        &seed,
        OfflineContexts::from_documents(reformatted).unwrap(),
    ));
    let vc = issue(&app, credential()).await;
    let (status, body) = post(
        &reformatted_app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    accepted(status, &body);
    let signer = TestSigner::new();
    let mut vp = presentation();
    vp["verifiableCredential"] = json!([vc]);
    let (status, body) = verify_vp(&reformatted_app, signer.presentation(vp)).await;
    accepted(status, &body);
}

#[tokio::test]
async fn inline_context_entries_apply_identically_on_issue_and_verify() {
    let app = app();
    let signer = TestSigner::new();
    let mapping = json!({"rating":"https://example.org/rating"});
    for siblings in [false, true] {
        let mut plain = credential();
        plain["id"] = "urn:credential:rating".into();
        plain["@context"]
            .as_array_mut()
            .unwrap()
            .push(mapping.clone());
        plain["credentialSubject"]["rating"] = "excellent".into();
        let mut wrapped = plain.clone();
        let mut entry = json!({"@context":mapping});
        if siblings {
            entry["rating"] = "https://ignored.example/rating".into();
            entry["@protected"] = "ignored sibling".into();
        }
        wrapped["@context"][1] = entry;
        let issued = issue(&app, wrapped.clone()).await;
        assert_eq!(issued["@context"], wrapped["@context"]);
        // The same signature must verify with either context spelling.
        let mut equivalent = issued.clone();
        equivalent["@context"] = plain["@context"].clone();
        for vc in [issued, equivalent] {
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":vc}),
            )
            .await;
            accepted(status, &body);
        }
        // Sign the ordinary document through an independent expansion implementation.
        plain["issuer"] = signer.did.clone().into();
        wrapped["issuer"] = plain["issuer"].clone();
        wrapped["proof"] = signer.sign(&plain, signer.proof("assertionMethod"));
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":wrapped}),
        )
        .await;
        accepted(status, &body);
    }
}

#[tokio::test]
async fn nested_and_scoped_context_documents_follow_the_resolver_rule() {
    let app = app();
    let signer = TestSigner::new();
    let mapping = json!({"rating":"https://example.org/rating"});
    for scoped in [false, true] {
        for direct in [false, true] {
            let mut plain = credential();
            plain["issuer"] = signer.did.clone().into();
            let mut wrapped = plain.clone();
            let context = if direct {
                json!({"@context":mapping,"rating":"https://ignored.example/rating"})
            } else {
                json!([{"@context":mapping,"rating":"https://ignored.example/rating"}])
            };
            if scoped {
                plain["@context"].as_array_mut().unwrap().push(json!({
                    "claim":{"@id":"https://example.org/claim","@context":mapping}
                }));
                plain["credentialSubject"]["claim"] = json!({"rating":"excellent"});
                wrapped = plain.clone();
                wrapped["@context"][1]["claim"]["@context"] = context;
            } else {
                plain["credentialSubject"]["@context"] = mapping.clone();
                plain["credentialSubject"]["rating"] = "excellent".into();
                wrapped["credentialSubject"] = plain["credentialSubject"].clone();
                wrapped["credentialSubject"]["@context"] = context;
            }
            wrapped["proof"] = signer.sign(&plain, signer.proof("assertionMethod"));
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":wrapped.clone()}),
            )
            .await;
            accepted(status, &body);
            wrapped.as_object_mut().unwrap().remove("proof");
            issue(&app, wrapped).await;
        }
    }
}

#[tokio::test]
async fn resolved_context_entries_require_an_object_after_one_unwrap() {
    let app = app();
    let signer = TestSigner::new();
    let mut plain = credential();
    plain["issuer"] = signer.did.clone().into();
    let proof = signer.sign(&plain, signer.proof("assertionMethod"));
    for inner in [
        json!("https://www.w3.org/ns/credentials/v2"),
        json!([]),
        json!([{}]),
        Value::Null,
        json!(7),
        json!(false),
        json!({"@context":{"rating":"https://example.org/rating"}}),
    ] {
        let mut vc = plain.clone();
        vc["@context"]
            .as_array_mut()
            .unwrap()
            .push(json!({"@context":inner}));
        let (status, body) =
            post(&app, "/credentials/issue", json!({"credential":vc.clone()})).await;
        rejected(status, &body);
        assert!(
            body["message"].as_str().unwrap().contains("JSON-LD"),
            "{body}"
        );
        vc["proof"] = proof.clone();
        let (status, body) = post(
            &app,
            "/credentials/verify",
            json!({"verifiableCredential":vc}),
        )
        .await;
        rejected(status, &body);
        assert!(
            body["message"].as_str().unwrap().contains("JSON-LD"),
            "{body}"
        );
    }
    // A direct wrapper containing null differs from an ordinary null context.
    let mut vc = plain;
    vc["credentialSubject"]["@context"] = json!({"@context":null});
    let (status, body) = post(&app, "/credentials/issue", json!({"credential":vc.clone()})).await;
    rejected(status, &body);
    assert_eq!(
        body["message"],
        "JSON-LD context entry must unwrap to an object"
    );
    vc["proof"] = proof;
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":vc}),
    )
    .await;
    rejected(status, &body);
    assert_eq!(
        body["message"],
        "JSON-LD context entry must unwrap to an object"
    );
}

#[tokio::test]
async fn held_and_direct_context_documents_share_inline_processing() {
    let signer = TestSigner::new();
    let mapping = json!({"rating":"https://example.org/rating"});
    for held in [
        json!({"@context":mapping}),
        json!({"@context":[{"@context":mapping}]}),
    ] {
        let mut documents = context_documents();
        documents.insert("urn:held:rating-context".into(), held.to_string());
        let mut seed = [0; 32];
        rand_core::OsRng.fill_bytes(&mut seed);
        let app = router(Service::new(
            &seed,
            OfflineContexts::from_documents(documents).unwrap(),
        ));
        // Direct local wrappers can resolve a URL or array before entry processing.
        for inner in [json!("urn:held:rating-context"), json!([mapping])] {
            let mut plain = credential();
            plain["issuer"] = signer.did.clone().into();
            plain["credentialSubject"]["@context"] = mapping.clone();
            plain["credentialSubject"]["rating"] = "excellent".into();
            let mut wrapped = plain.clone();
            wrapped["credentialSubject"]["@context"] = json!({"@context":inner});
            wrapped["proof"] = signer.sign(&plain, signer.proof("assertionMethod"));
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":wrapped.clone()}),
            )
            .await;
            accepted(status, &body);
            wrapped.as_object_mut().unwrap().remove("proof");
            issue(&app, wrapped).await;
        }
    }
}

#[tokio::test]
async fn terminal_controller_policy_rejects_notary_and_cosigner_shapes() {
    let app = app();
    let issuer = TestSigner::new();
    let notary = TestSigner::new();
    let mut vc = credential();
    vc["issuer"] = issuer.did.clone().into();
    let mut first = issuer.proof("assertionMethod");
    first["id"] = "urn:proof:issuer".into();
    first = issuer.sign(&vc, first);
    let mut single = vc.clone();
    single["proof"] = first.clone();
    let (status, body) = post(
        &app,
        "/credentials/verify",
        json!({"verifiableCredential":single}),
    )
    .await;
    accepted(status, &body);
    for countersignature in [false, true] {
        let mut input = vc.clone();
        let mut proof = notary.proof("assertionMethod");
        if countersignature {
            proof["previousProof"] = "urn:proof:issuer".into();
            input["proof"] = json!([first]);
        }
        let proof = notary.sign(&input, proof);
        for reverse in [false, true] {
            let mut document = vc.clone();
            document["proof"] = if reverse {
                json!([proof, first])
            } else {
                json!([first, proof])
            };
            let (status, body) = post(
                &app,
                "/credentials/verify",
                json!({"verifiableCredential":document}),
            )
            .await;
            rejected(status, &body);
            assert_eq!(
                body["message"],
                "verificationMethod controller must be the did:key issuer or holder"
            );
        }
    }
}

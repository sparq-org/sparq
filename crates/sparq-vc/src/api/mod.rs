//! Opt-in VC-API credential handlers for offline interoperability testing.
//!
//! See `crates/sparq-vc/vercel/README.md` for cache preparation, environment variables,
//! local suite execution, and the separate Vercel adapter. Issuer keys are test-only.

mod cache;
mod model;
mod proofs;

use std::{fmt, sync::Arc};

use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use ed25519_dalek::{Signer as _, Verifier as _};
use serde_json::{json, Value};
use sha2::Sha256;
use sparq_jsonld::{Json as LdJson, JsonLdOptions};

use crate::{
    did::{DidKeyResolver, DidResolver},
    SigningKey,
};
use model::{require, string};

pub use cache::{OfflineContexts, CONTEXT_FILES};

/// A rejected VC-API request or startup configuration.
#[derive(Debug)]
pub struct ApiError(String);

impl ApiError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "type": "ValidationError", "message": self.0,
                "verified": false, "checks": [], "warnings": [],
                "errors": [self.0]
            })),
        )
            .into_response()
    }
}

/// Shared native HTTP service state. It contains only an in-memory key and cache.
pub struct Service {
    key: SigningKey,
    contexts: OfflineContexts,
}

impl Service {
    /// Build a test issuer from an externally supplied Ed25519 seed and offline cache.
    pub fn new(seed: &[u8; 32], contexts: OfflineContexts) -> Self {
        Self {
            key: SigningKey::from_seed(seed),
            contexts,
        }
    }

    /// Read `SPARQ_VC_TEST_SEED` (64 hex digits) and `SPARQ_VC_CONTEXT_DIR`
    /// (default `crates/sparq-vc/vercel/contexts`). Missing or malformed seeds are errors.
    pub fn from_env() -> Result<Self, ApiError> {
        let directory = std::env::var_os("SPARQ_VC_CONTEXT_DIR")
            .unwrap_or_else(|| "crates/sparq-vc/vercel/contexts".into());
        Self::from_env_with_contexts(OfflineContexts::from_directory(std::path::Path::new(
            &directory,
        ))?)
    }

    /// Read `SPARQ_VC_TEST_SEED` with an already-loaded cache (e.g. embedded by a host).
    pub fn from_env_with_contexts(contexts: OfflineContexts) -> Result<Self, ApiError> {
        let text = std::env::var("SPARQ_VC_TEST_SEED")
            .map_err(|_| ApiError::new("SPARQ_VC_TEST_SEED is required"))?;
        Ok(Self::new(&seed_from_hex(&text)?, contexts))
    }

    /// A diagnostic DID document containing the public key and `testOnly: true`.
    pub fn did_document(&self) -> Value {
        let did = self.key.did_key();
        let multikey = did.strip_prefix("did:key:").expect("did:key prefix");
        let vm = format!("{did}#{multikey}");
        json!({
            "@context": ["https://www.w3.org/ns/did/v1", "https://w3id.org/security/multikey/v1",
                {"testOnly": "https://sparq.org/ns/vc-api#testOnly"}],
            "id": did, "testOnly": true,
            "verificationMethod": [{"id": vm, "type": "Multikey", "controller": did,
                "publicKeyMultibase": multikey}],
            "assertionMethod": [vm]
        })
    }

    fn issue(&self, request: &Value) -> Result<Value, ApiError> {
        let options = options(request, true, "assertionMethod")?;
        let mut vc = request
            .get("credential")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| ApiError::new("credential must be an object"))?;
        // VC-API permits supplying the issuer. Validate it before replacing it
        // with this service's identity, preserving optional issuer metadata.
        if vc.get("issuer").is_none() {
            vc["issuer"] = self.key.did_key().into();
        }
        model::validate(&vc, &self.contexts)?;
        require(vc.get("proof").is_none(), "credential already has a proof")?;
        if vc["issuer"].is_object() {
            vc["issuer"]["id"] = self.key.did_key().into();
        } else {
            vc["issuer"] = self.key.did_key().into();
        }
        if vc["@context"][0] == model::V1 {
            let contexts = vc["@context"].as_array_mut().expect("validated context");
            let di = json!("https://w3id.org/security/data-integrity/v2");
            if !contexts.contains(&di)
                && !contexts.contains(&json!("https://w3id.org/security/data-integrity/v1"))
            {
                contexts.push(di);
            }
        }
        let did = self.key.did_key();
        let vm = format!(
            "{did}#{}",
            did.strip_prefix("did:key:").expect("did:key prefix")
        );
        if let Some(value) = options.get("verificationMethod") {
            require(
                value == &vm,
                "verificationMethod must name the test issuer key",
            )?;
        }
        let mut proof = json!({
            "type": "DataIntegrityProof", "cryptosuite": "eddsa-rdfc-2022",
            "verificationMethod": vm, "proofPurpose": "assertionMethod",
            "created": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
        });
        for field in ["created", "domain", "challenge"] {
            if let Some(value) = options.get(field) {
                proof[field] = value.clone();
            }
        }
        validate_proof(&proof, "assertionMethod")?;
        let hash_data = self.hash_data(&vc, &proof)?;
        let signature = self.key.inner.sign(&hash_data);
        proof["proofValue"] = super::encode_proof_value(&signature).into();
        vc["proof"] = proof;
        Ok(json!({"verifiableCredential": vc}))
    }

    fn verify_request(&self, request: &Value, presentation: bool) -> Result<Value, ApiError> {
        let purpose = if presentation {
            "authentication"
        } else {
            "assertionMethod"
        };
        let options = options(request, false, purpose)?;
        let field = if presentation {
            "verifiablePresentation"
        } else {
            "verifiableCredential"
        };
        let document = request
            .get(field)
            .filter(|v| v.is_object())
            .ok_or_else(|| ApiError::new(format!("{field} must be an object")))?;
        if presentation {
            model::presentation(document, &self.contexts)?;
            if let Some(credentials) = document.get("verifiableCredential") {
                for credential in model::objects(credentials)? {
                    self.verify_credential(credential, &json!({}))?;
                }
            }
            let holder = document.get("holder").map(|h| {
                h.as_str()
                    .or_else(|| h["id"].as_str())
                    .expect("validated holder")
            });
            self.verify_proofs(document, &options, purpose, holder)?;
        } else {
            self.verify_credential(document, &options)?;
        }
        Ok(json!({"verified": true, "checks": ["proof"], "warnings": [], "errors": []}))
    }

    fn verify_credential(&self, credential: &Value, options: &Value) -> Result<(), ApiError> {
        model::validate(credential, &self.contexts)?;
        let issuer = credential["issuer"]
            .as_str()
            .or_else(|| credential["issuer"]["id"].as_str())
            .expect("validated issuer");
        self.verify_proofs(credential, options, "assertionMethod", Some(issuer))
    }

    fn verify_proofs(
        &self,
        document: &Value,
        options: &Value,
        purpose: &str,
        controller: Option<&str>,
    ) -> Result<(), ApiError> {
        let mut unsecured = document.clone();
        let proofs = unsecured
            .as_object_mut()
            .expect("validated document")
            .remove("proof")
            .ok_or_else(|| ApiError::new("proof is required"))?;
        let proofs: Vec<_> = proofs
            .as_array()
            .map_or_else(|| vec![&proofs], |a| a.iter().collect());
        let plan = proofs::Plan::new(&proofs, purpose)?;
        for index in plan.order {
            let proof = proofs[index];
            let vm = string(proof, "verificationMethod")?;
            if !plan.referenced[index] {
                if let Some(controller) = controller {
                    require(
                        vm.split('#').next() == Some(controller)
                            && controller.starts_with("did:key:"),
                        "verificationMethod controller must be the did:key issuer or holder",
                    )?;
                }
            }
            for field in ["domain", "challenge"] {
                if let Some(value) = options.get(field) {
                    require(
                        proof.get(field) == Some(value),
                        "proof does not match requested options",
                    )?;
                }
            }
            let key = DidKeyResolver
                .resolve_str(vm)
                .map_err(|_| ApiError::new("unresolvable verificationMethod"))?;
            let signature = super::decode_proof_value(string(proof, "proofValue")?)
                .map_err(|_| ApiError::new("invalid proofValue"))?;
            let mut input = unsecured.clone();
            if !plan.dependencies[index].is_empty() {
                input["proof"] = Value::Array(
                    plan.dependencies[index]
                        .iter()
                        .map(|&i| proofs[i].clone())
                        .collect(),
                );
            }
            let hash_data = self.hash_data(&input, proof)?;
            key.verify(&hash_data, &signature)
                .map_err(|_| ApiError::new("proof verification failed"))?;
        }
        Ok(())
    }

    fn hash_data(&self, vc: &Value, proof: &Value) -> Result<Vec<u8>, ApiError> {
        let mut config = proof.clone();
        config
            .as_object_mut()
            .expect("validated proof")
            .remove("proofValue");
        if let Some(context) = config.get("@context") {
            require(
                context == &vc["@context"],
                "proof context differs from credential context",
            )?;
        }
        config["@context"] = vc["@context"].clone();
        let proof_nquads = self.canonicalize(&config)?;
        let document_nquads = self.canonicalize(vc)?;
        Ok(super::hash_data_from_canonical(
            &proof_nquads,
            &document_nquads,
        ))
    }

    fn canonicalize(&self, value: &Value) -> Result<String, ApiError> {
        let mut value = value.clone();
        inline_context_documents(&mut value);
        self.check_terms(&value, &sparq_jsonld::context::ActiveContext::new(None))?;
        let input =
            LdJson::parse(&value.to_string()).map_err(|_| ApiError::new("invalid JSON-LD"))?;
        let expanded =
            sparq_jsonld::expand::expand(&input, &JsonLdOptions::default(), &self.contexts)
                .map_err(|e| ApiError::new(format!("JSON-LD expansion failed: {e}")))?;
        let mut expanded_text = String::new();
        expanded.write(&mut expanded_text);
        let expanded = expanded_text;
        let mut expanded: Value = serde_json::from_str(&expanded)
            .map_err(|_| ApiError::new("invalid expanded JSON-LD"))?;
        model::expanded_types(&expanded)?;
        i18n_datatypes(&mut expanded);
        let expanded = expanded.to_string();
        // sparq-jsonld's native toRdf is a stub. Convert its expanded output
        // with oxjsonld, which receives no remote contexts or network loader.
        let quads = oxjsonld::JsonLdParser::new()
            .for_slice(expanded.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApiError::new("JSON-LD to RDF conversion failed"))?;
        sparq_canon::canonicalize_quads_bounded_with::<Sha256>(
            &quads,
            &sparq_canon::CanonicalizationLimits {
                max_quads: 16_384,
                max_input_bytes: 4 * 1024 * 1024,
                max_output_bytes: 4 * 1024 * 1024,
                max_hndq_calls: 1_000,
                max_permutation_steps: 10_000_000,
            },
        )
        .map_err(|error| ApiError::new(format!("RDFC-1.0 canonicalization failed: {error}")))
    }

    // Refuse unmapped properties instead of silently excluding them from the
    // signed RDF. Process local, type-scoped and property-scoped contexts.
    fn check_terms(
        &self,
        value: &Value,
        inherited: &sparq_jsonld::context::ActiveContext,
    ) -> Result<(), ApiError> {
        match value {
            Value::Object(object) => {
                let process = |active: &sparq_jsonld::context::ActiveContext, local: &LdJson| {
                    active
                        .process(local, None, &self.contexts, &JsonLdOptions::default())
                        .map_err(|e| ApiError::new(format!("JSON-LD expansion failed: {e}")))
                };
                let mut active = inherited.clone();
                if let Some(context) = object.get("@context") {
                    let context = LdJson::parse(&context.to_string())
                        .map_err(|_| ApiError::new("invalid context"))?;
                    active = process(&active, &context)?;
                }
                let types = object.get("type").or_else(|| object.get("@type"));
                if let Some(types) = types {
                    let types = types
                        .as_array()
                        .map_or_else(|| vec![types], |a| a.iter().collect());
                    let mut terms: Vec<_> = types.iter().filter_map(|t| t.as_str()).collect();
                    terms.sort_unstable();
                    let before_types = active.clone();
                    for term in terms {
                        if let Some(context) =
                            before_types.term_definition(term).and_then(|d| d.context())
                        {
                            active = process(&active, context)?;
                        }
                    }
                }
                for (key, child) in object {
                    if key == "@context" {
                        continue;
                    }
                    require(
                        matches!(
                            key.as_str(),
                            "@id"
                                | "@type"
                                | "@graph"
                                | "@included"
                                | "@value"
                                | "@language"
                                | "@direction"
                                | "@list"
                                | "@set"
                                | "@index"
                                | "@reverse"
                                | "@nest"
                        ) || active.term_definition(key).and_then(|t| t.iri()).is_some()
                            || active.vocabulary_mapping().is_some()
                            || oxrdf::NamedNode::new(key).is_ok(),
                        "unmapped JSON-LD property",
                    )?;
                    if key == "@value" {
                        continue;
                    }
                    let child_context = active.term_definition(key).and_then(|d| d.context());
                    let scoped = match child_context {
                        Some(LdJson::Raw(raw)) if raw == "null" => {
                            sparq_jsonld::context::ActiveContext::new(None)
                        }
                        Some(context) => process(&active, context)?,
                        None => active.clone(),
                    };
                    self.check_terms(child, &scoped)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.check_terms(child, inherited)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}

// Accept a complete context document as an inline context entry, as used by
// the fixed Data Integrity chain vectors. Only a sole @context is unwrapped.
fn inline_context_documents(value: &mut Value) {
    fn context(value: &mut Value) {
        while value
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.contains_key("@context"))
        {
            *value = value
                .as_object_mut()
                .expect("context document")
                .remove("@context")
                .expect("context");
        }
        if let Value::Array(values) = value {
            for value in values {
                context(value);
            }
        }
    }
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get_mut("@context") {
                context(value);
            }
            for value in object.values_mut() {
                inline_context_documents(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                inline_context_documents(value);
            }
        }
        _ => (),
    }
}

// JSON-LD 1.1 rdfDirection = i18n-datatype, retaining direction in RDF 1.1
// rather than passing RDF 1.2 directional literals to standard RDFC-1.0.
fn i18n_datatypes(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(direction)) = object.remove("@direction") {
                let language = object
                    .remove("@language")
                    .and_then(|v| v.as_str().map(str::to_lowercase))
                    .unwrap_or_default();
                object.insert(
                    "@type".into(),
                    format!("https://www.w3.org/ns/i18n#{language}_{direction}").into(),
                );
            }
            for child in object.values_mut() {
                i18n_datatypes(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                i18n_datatypes(child);
            }
        }
        _ => (),
    }
}

fn seed_from_hex(text: &str) -> Result<[u8; 32], ApiError> {
    require(
        text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "SPARQ_VC_TEST_SEED must contain 64 hex digits",
    )?;
    let mut seed = [0; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| ApiError::new("invalid seed encoding"))?;
    }
    Ok(seed)
}

fn options(request: &Value, issue: bool, purpose: &str) -> Result<Value, ApiError> {
    require(request.is_object(), "request must be an object")?;
    let options = request.get("options").cloned().unwrap_or_else(|| json!({}));
    let object = options
        .as_object()
        .ok_or_else(|| ApiError::new("options must be an object"))?;
    for (field, value) in object {
        require(
            matches!(
                field.as_str(),
                "type"
                    | "cryptosuite"
                    | "proofPurpose"
                    | "created"
                    | "verificationMethod"
                    | "domain"
                    | "challenge"
                    | "checks"
            ),
            "unsupported option",
        )?;
        if field == "checks" {
            require(
                !issue && value == &json!(["proof"]),
                "only the proof check is supported",
            )?;
        } else {
            require(value.is_string(), "option must be a string")?;
            match field.as_str() {
                "type" => require(value == "DataIntegrityProof", "unsupported proof type")?,
                "cryptosuite" => require(value == "eddsa-rdfc-2022", "unsupported cryptosuite")?,
                "proofPurpose" => require(value == purpose, "unsupported proofPurpose")?,
                "created" | "verificationMethod" => {
                    require(issue, "unsupported verification option")?
                }
                _ => (),
            }
        }
    }
    Ok(options)
}

fn validate_proof(proof: &Value, purpose: &str) -> Result<(), ApiError> {
    let object = proof
        .as_object()
        .ok_or_else(|| ApiError::new("proof must be an object"))?;
    if proof.get("id").is_some() {
        require(
            oxrdf::NamedNode::new(string(proof, "id")?).is_ok(),
            "invalid proof id",
        )?;
    }
    require(
        object.keys().all(|k| {
            matches!(
                k.as_str(),
                "@context"
                    | "id"
                    | "type"
                    | "cryptosuite"
                    | "verificationMethod"
                    | "proofPurpose"
                    | "created"
                    | "domain"
                    | "challenge"
                    | "proofValue"
                    | "previousProof"
            )
        }),
        "unsupported proof option",
    )?;
    require(
        string(proof, "type")? == "DataIntegrityProof",
        "unsupported proof type",
    )?;
    require(
        string(proof, "cryptosuite")? == "eddsa-rdfc-2022",
        "unsupported cryptosuite",
    )?;
    require(
        string(proof, "proofPurpose")? == purpose,
        "unsupported proofPurpose",
    )?;
    let mut config = crate::ProofConfig::new(string(proof, "verificationMethod")?);
    for field in ["created", "domain", "challenge"] {
        if proof.get(field).is_some() {
            string(proof, field)?;
        }
    }
    config.created = proof
        .get("created")
        .and_then(Value::as_str)
        .map(str::to_owned);
    config
        .validate()
        .map_err(|_| ApiError::new("invalid proof options"))?;
    Ok(())
}

type RequestJson = Result<Json<Value>, JsonRejection>;

fn request_json(input: RequestJson) -> Result<Value, ApiError> {
    let Json(value) =
        input.map_err(|_| ApiError::new("expected a JSON request body within 256 KiB"))?;
    fn depth(value: &Value, level: usize) -> Result<(), ApiError> {
        require(level <= 64, "JSON nesting depth exceeds 64")?;
        match value {
            Value::Array(values) => {
                for child in values {
                    depth(child, level + 1)?;
                }
            }
            Value::Object(values) => {
                for child in values.values() {
                    depth(child, level + 1)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    depth(&value, 0)?;
    Ok(value)
}

async fn issue(
    State(service): State<Arc<Service>>,
    input: RequestJson,
) -> Result<impl IntoResponse, ApiError> {
    let request = request_json(input)?;
    Ok((StatusCode::CREATED, Json(service.issue(&request)?)))
}

async fn verify(
    State(service): State<Arc<Service>>,
    input: RequestJson,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(service.verify_request(&request_json(input)?, false)?))
}

async fn verify_presentation(
    State(service): State<Arc<Service>>,
    input: RequestJson,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(service.verify_request(&request_json(input)?, true)?))
}

async fn method_not_allowed() -> Response {
    let mut response = ApiError::new("method not allowed").into_response();
    *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
    response
}

async fn not_found() -> Response {
    let mut response = ApiError::new("route not found").into_response();
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

async fn did(State(service): State<Arc<Service>>) -> Json<Value> {
    Json(service.did_document())
}

/// POST `/credentials/issue` (201), POST `/credentials/verify` (200), and
/// POST `/presentations/verify` (200), and GET `/did.json`. Rejections return 400
/// with a readable `message`, `errors`, and
/// `verified: false`. Request bodies are limited to 256 KiB.
pub fn router(service: Service) -> Router {
    Router::new()
        .route("/credentials/issue", post(issue))
        .route("/credentials/verify", post(verify))
        .route("/presentations/verify", post(verify_presentation))
        .route("/did.json", get(did))
        .method_not_allowed_fallback(method_not_allowed)
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(Arc::new(service))
}

#[cfg(test)]
mod tests {
    use super::seed_from_hex;
    use rand_core::RngCore as _;

    #[test]
    fn seed_encoding_is_strict_and_lossless() {
        for text in ["", "1234", "not-a-seed", &"g".repeat(64), &"é".repeat(32)] {
            assert!(seed_from_hex(text).is_err());
        }
        let mut seed = [0; 32];
        rand_core::OsRng.fill_bytes(&mut seed);
        assert_eq!(seed_from_hex(&hex::encode(seed)).unwrap(), seed);
        assert_eq!(seed_from_hex(&hex::encode_upper(seed)).unwrap(), seed);
    }
}

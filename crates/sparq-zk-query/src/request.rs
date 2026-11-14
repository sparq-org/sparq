//! Verifier-stored query and issuer policy for the research suite profile.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use oxrdf::{Literal, NamedNode};
use sha2::{Digest, Sha256};
use sparq_query_protocol::{
    ArtifactIdentity, DatasetAssembly, Digest32, EvaluationMode, HolderPolicy, Identifier,
    MethodDescriptor, QueryForm, ResultContract, SourceEvidence, StatusPolicy, StoredRequest,
};

use crate::{
    capacity::Bounds,
    lower::lower,
    plan::{Plan, ResultForm},
    Error,
};

pub const METHOD: &str = "urn:sparq:vcq:method:noir-query:v1";
pub const DIALECT: &str = "urn:sparq:sparql11";
pub const PROFILE: &str = "urn:sparq:vcq:noir-query:poseidon2-cells:v1";
pub const SUITE: &str = "urn:sparq:cryptosuite:schnorr-poseidon2-merkle-2026";
const BACKEND: &str = "urn:sparq:noir:beta.21:bb:5.0.0-nightly.20260324";
const VCQ: &str = "https://w3id.org/sparq/vcq#";

/// Immutable verifier request. No field is obtained from the presentation.
#[derive(Clone, Debug)]
pub struct Request {
    stored: StoredRequest,
    bounds: Bounds,
    key: Vec<u8>,
    digest: [u8; 32],
    canonical_rdf: String,
    envelope: Option<crate::statement::EnvelopeInputs>,
    envelope_policy: Option<crate::envelope_policy::PolicyInput>,
    credential_quad_bounds: Vec<u32>,
}

pub fn descriptor(plan: &Plan, key: &[u8]) -> Result<MethodDescriptor, Error> {
    let mut parameters = plan.hash().to_vec();
    let b = plan.bounds();
    for n in [b.credentials, b.quads_per_credential, b.rows_per_table]
        .into_iter()
        .chain(plan.credential_quad_bounds())
    {
        parameters.extend(n.to_be_bytes());
    }
    MethodDescriptor::new(
        Identifier::new(METHOD).map_err(Error::Protocol)?,
        1,
        Identifier::new(PROFILE).map_err(Error::Protocol)?,
        Digest32::new(Sha256::digest(parameters).into()).map_err(Error::Protocol)?,
        ArtifactIdentity::VerificationKey {
            vk_digest: Digest32::new(Sha256::digest(key).into()).map_err(Error::Protocol)?,
        },
        Identifier::new(BACKEND).map_err(Error::Protocol)?,
    )
    .map_err(Error::Protocol)
}

impl Request {
    pub fn new(stored: StoredRequest, bounds: Bounds, key: Vec<u8>) -> Result<Self, Error> {
        Self::new_inner(stored, bounds, key, None, None)
    }
    /// Store the verifier's authoritative issuer-set root, never a presentation value.
    pub fn new_authenticated(
        stored: StoredRequest,
        bounds: Bounds,
        key: Vec<u8>,
        issuer_root: [u8; 32],
    ) -> Result<Self, Error> {
        crate::credential::field(issuer_root)?;
        Self::new_inner(
            stored,
            bounds,
            key,
            Some(crate::statement::EnvelopeInputs::issuer_only(issuer_root)),
            None,
        )
    }
    /// Store distinct public quad limits for the ordered credential slots.
    pub fn new_authenticated_with_quad_bounds(
        stored: StoredRequest,
        bounds: Bounds,
        key: Vec<u8>,
        issuer_root: [u8; 32],
        quad_bounds: &[u32],
    ) -> Result<Self, Error> {
        crate::credential::field(issuer_root)?;
        Self::new_inner(
            stored,
            bounds,
            key,
            Some(crate::statement::EnvelopeInputs::issuer_only(issuer_root)),
            Some(quad_bounds),
        )
    }
    /// Admit typed policy values supplied by the verifier's envelope adapter.
    pub fn new_authenticated_with_policy(
        stored: StoredRequest,
        bounds: Bounds,
        key: Vec<u8>,
        envelope: crate::statement::EnvelopeInputs,
        quad_bounds: Option<&[u32]>,
    ) -> Result<Self, Error> {
        envelope.validate()?;
        Self::new_inner(stored, bounds, key, Some(envelope), quad_bounds)
    }
    /// Derive policy roots and public words from verifier-stored typed properties.
    /// Validity and freshness instants are Unix epoch milliseconds.
    pub fn new_authenticated_with_envelope(
        stored: StoredRequest,
        bounds: Bounds,
        key: Vec<u8>,
        policy: crate::envelope_policy::PolicyInput,
        quad_bounds: Option<&[u32]>,
    ) -> Result<Self, Error> {
        let envelope = crate::envelope_policy::derive(&policy)?.into();
        let mut request = Self::new_inner(stored, bounds, key, Some(envelope), quad_bounds)?;
        request.envelope_policy = Some(policy);
        Ok(request)
    }
    fn new_inner(
        stored: StoredRequest,
        bounds: Bounds,
        key: Vec<u8>,
        envelope: Option<crate::statement::EnvelopeInputs>,
        quad_bounds: Option<&[u32]>,
    ) -> Result<Self, Error> {
        if key.is_empty() {
            return Err(Error::Invalid("verification key".into()));
        }
        let mut plan = lower(stored.query(), bounds)?;
        if let Some(quad_bounds) = quad_bounds {
            plan = plan.with_credential_quad_bounds(quad_bounds)?;
        }
        let credential_quad_bounds = plan.credential_quad_bounds();
        let requirements = stored.requirements();
        if stored.base_iri().is_some() {
            return Err(Error::Excluded("external base IRI".into()));
        }
        if requirements.mode() != EvaluationMode::ExactBounded {
            return Err(Error::Unsupported("evaluation mode".into()));
        }
        if envelope.is_some() {
            if requirements.source_evidence() != SourceEvidence::IssuerAuthenticated
                || !requirements
                    .accepted_suites()
                    .iter()
                    .any(|s| s.as_str() == SUITE)
            {
                return Err(Error::Unsupported(
                    "authenticated credential suite policy".into(),
                ));
            }
        } else if requirements.source_evidence() != SourceEvidence::None {
            return Err(Error::Unsupported("credential suite opening".into()));
        }
        if let Some(policy) = &envelope {
            policy.validate()?;
            if policy.holder_required != (requirements.holder() == HolderPolicy::Required)
                || policy.status_required != (requirements.status() == StatusPolicy::Required)
            {
                return Err(Error::Invalid("request and envelope policy".into()));
            }
        } else if requirements.status() != StatusPolicy::NotRequested {
            return Err(Error::Unsupported("credential status envelope".into()));
        } else if requirements.holder() != HolderPolicy::BearerAccepted {
            return Err(Error::Unsupported("holder envelope".into()));
        }
        if requirements.assembly() != DatasetAssembly::UnionDefaultGraph {
            return Err(Error::Unsupported("dataset assembly".into()));
        }
        if requirements.query_profile().dialect.as_str() != DIALECT
            || requirements.query_profile().fragment.as_str() != PROFILE
            || requirements
                .accepted_mappings()
                .iter()
                .all(|p| p.as_str() != PROFILE)
            || requirements
                .accepted_linking()
                .iter()
                .all(|p| p.as_str() != PROFILE)
        {
            return Err(Error::Unsupported(
                "query mapping and linking profile".into(),
            ));
        }
        let expected = match plan.form() {
            ResultForm::Select => (QueryForm::Select, ResultContract::SelectSequence),
            ResultForm::Ask => (QueryForm::Ask, ResultContract::AskBoolean),
        };
        if stored.form() != expected.0 || requirements.contract() != expected.1 {
            return Err(Error::Invalid("query form and result contract".into()));
        }
        if requirements.methods() != [descriptor(&plan, &key)?] {
            return Err(Error::Invalid("method descriptor".into()));
        }
        let canonical_rdf = canonical_request(&stored, bounds, &plan, &key, envelope.as_ref())?;
        let digest = Sha256::digest(canonical_rdf.as_bytes()).into();
        Ok(Self {
            stored,
            bounds,
            key,
            digest,
            canonical_rdf,
            envelope,
            envelope_policy: None,
            credential_quad_bounds,
        })
    }
    pub fn issuer_root(&self) -> Result<[u8; 32], Error> {
        Ok(self.envelope_inputs()?.issuer_root)
    }
    /// Single trusted seam for envelope public-input derivation.
    pub fn envelope_inputs(&self) -> Result<crate::statement::EnvelopeInputs, Error> {
        let policy = if let Some(input) = &self.envelope_policy {
            crate::envelope_policy::derive(input)?.into()
        } else {
            self.envelope
                .clone()
                .ok_or_else(|| Error::Unsupported("authenticated issuer policy required".into()))?
        };
        policy.validate()?;
        Ok(policy)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn require_envelope_consumer(
        &self,
        envelope: &crate::statement::EnvelopeInputs,
    ) -> Result<(), Error> {
        envelope.require_consumer(self.envelope_policy.is_some())
    }
    pub fn stored(&self) -> &StoredRequest {
        &self.stored
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn key(&self) -> &[u8] {
        &self.key
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    /// Normative digest input: RDFC canonical N-Quads of this method's request.
    pub fn canonical_rdf(&self) -> &str {
        &self.canonical_rdf
    }
    /// Rebuild the trusted plan and credential limits from this stored request.
    pub fn plan(&self) -> Result<Plan, Error> {
        lower(self.stored.query(), self.bounds)?
            .with_credential_quad_bounds(&self.credential_quad_bounds)
    }
}

fn canonical_request(
    stored: &StoredRequest,
    b: Bounds,
    plan: &Plan,
    key: &[u8],
    envelope: Option<&crate::statement::EnvelopeInputs>,
) -> Result<String, Error> {
    let mut rdf = String::new();
    let mut triple = |subject: &str, predicate: &str, object: String| {
        rdf.push_str(&format!("{subject} <{predicate}> {object} .\n"));
    };
    let literal = |s: &str| Literal::new_simple_literal(s).to_string();
    let integer = |n: u32| {
        Literal::new_typed_literal(
            n.to_string(),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#nonNegativeInteger"),
        )
        .to_string()
    };
    let bytes = |value: &[u8]| {
        Literal::new_typed_literal(
            format!("u{}", URL_SAFE_NO_PAD.encode(value)),
            NamedNode::new_unchecked("https://w3id.org/security#multibase"),
        )
        .to_string()
    };
    triple(
        "_:r",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        format!("<{VCQ}QueryRequest>"),
    );
    triple("_:r", &format!("{VCQ}query"), literal(stored.query()));
    triple("_:r", &format!("{VCQ}proofMethod"), "_:m".into());
    triple("_:m", &format!("{VCQ}method"), format!("<{METHOD}>"));
    triple("_:m", &format!("{VCQ}verificationKey"), bytes(key));
    // The local structural encoding is an explicitly named method parameter,
    // never substituted for the request's RDF canonicalization.
    let mut parameters = serde_json::json!({
        "bounds": {"credentials":b.credentials, "quadsPerCredential":b.quads_per_credential,"rowsPerTable":b.rows_per_table},
        "planHash": URL_SAFE_NO_PAD.encode(plan.hash()), "profile":PROFILE,
        "credentialQuadBounds":plan.credential_quad_bounds(),
        "storedRequestEncoding":sparq_query_protocol::LOCAL_ENCODING_PROFILE,
        "storedRequest":URL_SAFE_NO_PAD.encode(sparq_query_protocol::encode_stored_request(stored)),
    });
    if let Some(policy) = envelope {
        parameters["issuerRoot"] = serde_json::json!(URL_SAFE_NO_PAD.encode(policy.issuer_root));
        parameters["envelope"] = serde_json::json!(policy
            .words()?
            .iter()
            .map(|w| URL_SAFE_NO_PAD.encode(w))
            .collect::<Vec<_>>());
    }
    // This parameter schema contains only strings and bounded integers. Sort
    // both objects explicitly for its canonical RDF JSON lexical form.
    parameters["bounds"].as_object_mut().unwrap().sort_keys();
    parameters.as_object_mut().unwrap().sort_keys();
    triple(
        "_:m",
        &format!("{VCQ}parameters"),
        Literal::new_typed_literal(
            parameters.to_string(),
            NamedNode::new_unchecked("http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON"),
        )
        .to_string(),
    );
    let resources = stored.requirements().resources();
    triple(
        "_:r",
        &format!("{VCQ}maxPresentationBytes"),
        integer(resources.presentation_bytes()),
    );
    triple(
        "_:r",
        &format!("{VCQ}maxResultSize"),
        integer(resources.released_rows()),
    );
    triple(
        "_:r",
        "https://w3id.org/security#challenge",
        literal(&format!(
            "u{}",
            URL_SAFE_NO_PAD.encode(stored.challenge().as_bytes())
        )),
    );
    triple(
        "_:r",
        "https://w3id.org/security#domain",
        literal(stored.audience().as_str()),
    );
    for (name, seconds) in [
        ("validFrom", stored.not_before()),
        ("validUntil", stored.not_after()),
    ] {
        let date = i64::try_from(seconds)
            .ok()
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .ok_or_else(|| Error::Invalid("request validity date".into()))?;
        triple(
            "_:r",
            &format!("https://www.w3.org/2018/credentials#{name}"),
            Literal::new_typed_literal(
                date.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime"),
            )
            .to_string(),
        );
    }
    if let Some(anchor) = stored.requirements().anchor() {
        triple(
            "_:r",
            &format!("{VCQ}inputCommitment"),
            bytes(anchor.as_bytes()),
        );
    }
    sparq_canon::canonicalize_nquads(&rdf).map_err(|e| Error::Invalid(format!("request RDF: {e}")))
}

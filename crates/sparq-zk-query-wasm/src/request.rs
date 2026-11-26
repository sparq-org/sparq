use crate::codec::{bytes, digest, err, hex, word, Result};
use crate::coverage::CoverageWire;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_query_protocol::*;
use sparq_zk::sig;
use sparq_zk_query::{
    capacity::Bounds,
    capacity::OpeningProfile,
    coverage::Coverage,
    envelope_policy::{AcceptedSnapshots, HolderBinding, PolicyInput, TrustMember},
    lower::lower,
    plan::{EnvelopeFeatures, Plan, ResultForm},
    request::{self, Request},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TrustMemberWire {
    pub issuer: String,
    pub verification_method: String,
    pub public_key: String,
}

/// Holder binding: omit `issuers` to bind every credential; a present list is nonempty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HolderBindingWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuers: Option<Vec<String>>,
}
/// Verifier-curated status snapshot digests (see `statusSnapshot`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SnapshotsWire {
    pub digests: Vec<String>,
    #[serde(default)]
    pub allow_missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Limits {
    pub credentials: u32,
    pub quads_per_credential: u32,
    pub rows_per_table: u32,
}
impl Limits {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            credentials: self.credentials,
            quads_per_credential: self.quads_per_credential,
            rows_per_table: self.rows_per_table,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Policy {
    pub bounds: Limits,
    pub trusted_issuers: Vec<String>,
    #[serde(default)]
    pub trust_members: Vec<TrustMemberWire>,
    pub audience: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_quad_bounds: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_commitment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_presentation_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_result_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_binding: Option<HolderBindingWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_snapshots: Option<SnapshotsWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_created: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_row_bounds: Option<Vec<u32>>,
    /// Lexical opening limits; holder keys need `long-objects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_profile: Option<String>,
}
impl Policy {
    pub fn normalize(&self) -> Result<Self> {
        let mut p = self.clone();
        if p.opening_profile.as_deref() == Some(OpeningProfile::default().name()) {
            p.opening_profile = None;
        }
        p.profile()?;
        if let Some(HolderBindingWire {
            issuers: Some(issuers),
        }) = &mut p.holder_binding
        {
            for issuer in issuers.iter() {
                oxrdf::NamedNode::new(issuer.as_str()).map_err(err)?;
            }
            issuers.sort();
            issuers.dedup();
        }
        if let Some(snapshots) = &mut p.accepted_snapshots {
            snapshots.digests = snapshots
                .digests
                .iter()
                .map(|d| word(d).map(|w| hex(&w)))
                .collect::<Result<_>>()?;
            snapshots.digests.sort();
            snapshots.digests.dedup();
        }
        if self.envelope_features() != EnvelopeFeatures::default() && self.trust_members.is_empty()
        {
            return Err("credential envelope features need canonical trust members".into());
        }
        p.trusted_issuers = self.keys()?.iter().map(sig::public_key_to_hex).collect();
        p.trusted_issuers.sort();
        p.trusted_issuers.dedup();
        if p.trusted_issuers.is_empty() {
            return Err("trusted issuer set is empty".into());
        }
        for member in &mut p.trust_members {
            let key =
                sig::public_key_from_hex(&member.public_key).ok_or("trust member key rejected")?;
            member.public_key = sig::public_key_to_hex(&key);
        }
        p.trust_members.sort();
        p.trust_members.dedup();
        if !p.trust_members.is_empty() {
            p.envelope_policy()?;
        }
        if let Some(coverage) = &mut p.coverage {
            coverage.normalize()?;
        }
        Ok(p)
    }
    fn anchor(&self) -> Result<Option<Digest32>> {
        self.input_commitment
            .as_deref()
            .map(word)
            .transpose()?
            .map(Digest32::new)
            .transpose()
            .map_err(err)
    }
    fn validated_coverage(&self) -> Result<Option<Coverage>> {
        self.coverage
            .as_ref()
            .map(|coverage| coverage.validate(self.anchor()?))
            .transpose()
    }
    pub fn envelope_policy(&self) -> Result<PolicyInput> {
        if self.trust_members.is_empty() {
            return Err("canonical trust members required".into());
        }
        let mut keys: Vec<_> = self
            .trust_members
            .iter()
            .map(|m| m.public_key.clone())
            .collect();
        keys.sort();
        keys.dedup();
        if keys != self.trusted_issuers {
            return Err("trust member keys differ from trusted issuers".into());
        }
        let trust_members = self
            .trust_members
            .iter()
            .map(|m| {
                Ok(TrustMember {
                    issuer: oxrdf::NamedNode::new(m.issuer.clone()).map_err(err)?,
                    verification_method: oxrdf::NamedNode::new(m.verification_method.clone())
                        .map_err(err)?,
                    key: sig::public_key_from_hex(&m.public_key)
                        .ok_or("trust member key rejected")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let policy = PolicyInput {
            trust_members,
            holder_binding: self
                .holder_binding
                .as_ref()
                .map(|h| {
                    Ok::<_, String>(HolderBinding {
                        issuers: h
                            .issuers
                            .as_ref()
                            .map(|list| {
                                list.iter()
                                    .map(|i| oxrdf::NamedNode::new(i.as_str()).map_err(err))
                                    .collect::<Result<Vec<_>>>()
                            })
                            .transpose()?,
                    })
                })
                .transpose()?,
            accepted_snapshots: self
                .accepted_snapshots
                .as_ref()
                .map(|a| {
                    Ok::<_, String>(AcceptedSnapshots {
                        digests: a.digests.iter().map(|d| word(d)).collect::<Result<_>>()?,
                        allow_missing: a.allow_missing,
                    })
                })
                .transpose()?,
            validity_time: self.validity_time,
            earliest_created: self.earliest_created,
        };
        sparq_zk_query::envelope_policy::derive(&policy).map_err(err)?;
        Ok(policy)
    }
    pub fn keys(&self) -> Result<Vec<sig::PublicKey>> {
        self.trusted_issuers
            .iter()
            .map(|key| {
                sig::public_key_from_hex(key).ok_or_else(|| "trusted issuer key rejected".into())
            })
            .collect()
    }
    pub fn profile(&self) -> Result<OpeningProfile> {
        match self.opening_profile.as_deref() {
            None => Ok(OpeningProfile::default()),
            Some(name) => [
                OpeningProfile::Solid,
                OpeningProfile::LongObjects,
                OpeningProfile::Extended,
            ]
            .into_iter()
            .find(|p| p.name() == name)
            .ok_or_else(|| "opening profile rejected".into()),
        }
    }
    /// Circuit consumers follow the requested policy properties, as in the native request.
    pub fn envelope_features(&self) -> EnvelopeFeatures {
        EnvelopeFeatures {
            holder: self.holder_binding.is_some(),
            status: self.accepted_snapshots.is_some(),
            validity: self.validity_time.is_some(),
            freshness: self.earliest_created.is_some(),
        }
    }
    /// The same plan shaping as the query crate's request reconstruction: lower, opening
    /// profile, envelope features, per-table rows, then per-credential quad bounds.
    pub fn plan(&self, query: &str) -> Result<Plan> {
        let bounds = self.bounds.bounds();
        let mut plan = lower(query, bounds)
            .map_err(err)?
            .with_opening_profile(self.profile()?)
            .with_envelope_features(self.envelope_features());
        if let Some(rows) = &self.table_row_bounds {
            plan = sparq_zk_query::plan_closed::with_row_bounds(plan, bounds, rows).map_err(err)?;
        }
        if let Some(limits) = &self.credential_quad_bounds {
            plan = plan.with_credential_quad_bounds(limits).map_err(err)?;
        }
        Ok(plan)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequestWire {
    pub query: String,
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_checked_at: Option<u64>,
    pub nonce: String,
    pub plan_hash: String,
    pub verification_key: String,
    pub digest: String,
    pub request_rdf: String,
}
fn id(s: &str) -> Result<Identifier> {
    Identifier::new(s).map_err(err)
}
fn native(
    query: &str,
    p: &Policy,
    nonce: &str,
    key: &[u8],
    coverage: Option<(Coverage, u64)>,
) -> Result<Request> {
    let plan = p.plan(query)?;
    let envelope = p.envelope_policy()?;
    let anchor = p.anchor()?;
    let is_ask = plan.form() == ResultForm::Ask;
    let requirements = QueryRequirements::new(RequirementsSpec {
        contract: if is_ask {
            ResultContract::AskBoolean
        } else {
            ResultContract::SelectSequence
        },
        mode: EvaluationMode::ExactBounded,
        authority: if anchor.is_some() {
            ScopeAuthority::VerifierAgreedAnchor
        } else {
            ScopeAuthority::HolderDeclared
        },
        anchor,
        source_evidence: Some(SourceEvidence::IssuerAuthenticated),
        status: Some(if p.accepted_snapshots.is_some() {
            StatusPolicy::Required
        } else {
            StatusPolicy::NotRequested
        }),
        holder: Some(if p.holder_binding.is_some() {
            HolderPolicy::Required
        } else {
            HolderPolicy::BearerAccepted
        }),
        assembly: DatasetAssembly::UnionDefaultGraph,
        query_profile: QueryProfile {
            dialect: id(request::DIALECT)?,
            fragment: id(request::PROFILE)?,
        },
        accepted_suites: vec![id(request::SUITE)?],
        accepted_mappings: vec![id(request::PROFILE)?],
        accepted_linking: vec![id(request::PROFILE)?],
        resources: ResourceBounds::new(
            p.max_result_size.unwrap_or(p.bounds.rows_per_table),
            p.max_presentation_bytes.unwrap_or(1 << 24),
        )
        .map_err(err)?,
        methods: vec![request::descriptor(&plan, key).map_err(err)?],
    })
    .map_err(err)?;
    let stored = StoredRequest::new(StoredRequestSpec {
        requirements,
        query: query.into(),
        challenge: Challenge32::new(word(nonce)?).map_err(err)?,
        audience: id(&p.audience)?,
        not_before: p
            .not_before
            .ok_or("stored request needs its validity start")?,
        not_after: p.not_after.ok_or("stored request needs its validity end")?,
        form: if is_ask {
            QueryForm::Ask
        } else {
            QueryForm::Select
        },
        base_iri: None,
        describe_policy: None,
    })
    .map_err(err)?;
    let request = Request::new_authenticated_with_envelope_profile_and_table_rows(
        stored,
        p.bounds.bounds(),
        key.to_vec(),
        envelope,
        p.credential_quad_bounds.as_deref(),
        p.profile()?,
        p.table_row_bounds.as_deref(),
    )
    .map_err(err)?;
    match coverage {
        Some((coverage, issued_at)) => request.with_coverage(coverage, issued_at).map_err(err),
        None => Ok(request),
    }
}
pub(crate) fn checked_key(plan: &Plan, key: &[u8]) -> Result<()> {
    let table: Value = serde_json::from_str(crate::CATALOGUE).map_err(err)?;
    let h = hex(&plan.hash());
    let entry = table["entries"]
        .as_array()
        .ok_or("catalogue rejected")?
        .iter()
        .find(|e| e["planHash"] == h)
        .ok_or("missing catalogue plan")?;
    if entry["vkSha256"].as_str() != Some(digest(key).as_str()) {
        return Err("verification key mismatch".into());
    }
    Ok(())
}
pub(crate) fn create(input: &str) -> Result<RequestWire> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Input {
        query: String,
        policy: Policy,
        nonce: String,
        verification_key: String,
        clock: u64,
    }
    let input: Input = crate::codec::parse(input)?;
    let mut policy = input.policy.normalize()?;
    let coverage = policy.validated_coverage()?;
    let mut default_end = input
        .clock
        .checked_add(3600)
        .ok_or("request time overflow")?;
    if let Some(evidence) = &coverage {
        evidence.check_valid_at(input.clock).map_err(err)?;
        default_end = default_end.min(evidence.claims().expires_at);
    }
    policy.not_before.get_or_insert(input.clock);
    policy.not_after.get_or_insert(default_end);
    let key = bytes(&input.verification_key)?;
    let plan = policy.plan(&input.query)?;
    checked_key(&plan, &key)?;
    let nonce = hex(&word(&input.nonce)?);
    let coverage_checked_at = coverage.as_ref().map(|_| input.clock);
    let request = native(
        &input.query,
        &policy,
        &nonce,
        &key,
        coverage.zip(coverage_checked_at),
    )?;
    Ok(RequestWire {
        query: input.query,
        policy,
        coverage_checked_at,
        nonce,
        plan_hash: hex(&plan.hash()),
        verification_key: hex(&key),
        digest: hex(&request.digest()),
        request_rdf: request.canonical_rdf().into(),
    })
}
pub(crate) fn restore(wire: &RequestWire) -> Result<Request> {
    let policy = wire.policy.normalize()?;
    if policy != wire.policy {
        return Err("noncanonical stored policy".into());
    }
    let key = bytes(&wire.verification_key)?;
    let coverage = policy.validated_coverage()?;
    if coverage.is_some() != wire.coverage_checked_at.is_some() {
        return Err("stored request reconstruction differs".into());
    }
    let request = native(
        &wire.query,
        &policy,
        &wire.nonce,
        &key,
        coverage.zip(wire.coverage_checked_at),
    )?;
    if wire.plan_hash != hex(&request.plan().map_err(err)?.hash())
        || wire.digest != hex(&request.digest())
        || wire.request_rdf != request.canonical_rdf()
        || wire.nonce != hex(request.stored().challenge().as_bytes())
        || wire.verification_key != hex(&key)
    {
        return Err("stored request reconstruction differs".into());
    }
    Ok(request)
}
pub(crate) fn plan_info(input: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        query: String,
        policy: Policy,
    }
    let input: Input = crate::codec::parse(input)?;
    let policy = input.policy.normalize()?;
    let plan = policy.plan(&input.query)?;
    let package = sparq_zk_query::codegen::generate(&plan).map_err(err)?;
    let capacity = plan.capacity();
    Ok(
        json!({"planHash":hex(&plan.hash()),"packageName":package.name,"files":package.files,
        "capacity":{"credentials":capacity.credentials,"quadsPerCredential":capacity.quads_per_credential,"rowsPerTable":capacity.rows_per_table},
        "publicInputDeclarations":sparq_zk_query::statement::Layout::new(&plan).declarations_authenticated()}),
    )
}
pub(crate) fn freshness(request: &Request, clock: u64) -> Result<()> {
    if clock < request.stored().not_before() || clock >= request.stored().not_after() {
        Err("request validity window".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    fn fixture() -> RequestWire {
        let input: Value = serde_json::from_str(include_str!(
            "../../../js/zk-query/test/fixtures/authenticated.json"
        ))
        .unwrap();
        serde_json::from_value(input["request"].clone()).unwrap()
    }

    fn policy(wire: &RequestWire, signing_seed: u8) -> Policy {
        let mut policy = serde_json::to_value(&wire.policy).unwrap();
        let anchor = hex(&[8; 32]);
        let member = json!({
            "issuer": "urn:authority",
            "verificationMethod": "urn:authority:key",
            "publicKey": crate::issuance::test_keygen(&hex(&[1; 32])).unwrap().public_key
        });
        let scope = json!({
            "subject": "urn:subject", "registry": "urn:registry", "claimTypes": ["urn:claim"],
            "periodStart": 1, "periodEnd": 2, "method": "urn:method", "profile": "urn:profile"
        });
        let signing_key = crate::issuance::test_keygen(&hex(&[signing_seed; 32])).unwrap();
        let mut documents = crate::coverage::document(
            &json!({
                "issuer": member["issuer"], "verificationMethod": member["verificationMethod"],
                "scope": scope, "asOf": 2, "expiresAt": 6, "inputCommitment": anchor
            })
            .to_string(),
        )
        .unwrap();
        documents["issuer"] = json!(signing_key);
        documents["seedHex"] = json!(hex(&[3; 32]));
        let credential = crate::issuance::issue(&documents.to_string(), true).unwrap();
        let mut credential = serde_json::to_value(credential).unwrap();
        credential["issuer"] = member["publicKey"].clone();
        policy["coverage"] = json!({"authority": member, "scope": scope, "credential": credential});
        policy["inputCommitment"] = json!(anchor);
        policy["notBefore"] = json!(2);
        policy["notAfter"] = json!(6);
        serde_json::from_value(policy).unwrap()
    }

    #[test]
    fn coverage_request_rejects_wrong_signature_before_storage() {
        let fixture = fixture();
        let input = |signer| {
            json!({
                "query": fixture.query, "nonce": fixture.nonce, "clock": 2,
                "policy": policy(&fixture, signer), "verificationKey": fixture.verification_key
            })
            .to_string()
        };
        let stored = create(&input(1)).unwrap();
        let mut uncovered: Value = serde_json::from_str(&input(1)).unwrap();
        uncovered["policy"].as_object_mut().unwrap().remove("coverage");
        let uncovered = create(&uncovered.to_string()).unwrap();
        assert_ne!(stored.digest, uncovered.digest);
        let request = restore(&stored).unwrap();
        let attestation = request.coverage().unwrap();
        assert_eq!(attestation.claims().issuer.as_str(), "urn:authority");
        let stored_coverage = serde_json::to_value(&stored.policy.coverage).unwrap();
        assert_eq!(
            Some(attestation.credential().document.as_str()),
            stored_coverage["credential"]["graphNQuads"].as_str()
        );
        assert_eq!(
            request.stored().coverage_digest(),
            Some(&attestation.digest())
        );
        assert!(request.check_dataset_agreement([8; 32]).is_ok());
        assert!(request.check_dataset_agreement([9; 32]).is_err());
        assert!(create(&input(2))
            .err()
            .unwrap()
            .contains("coverage signature"));
    }

    #[test]
    fn coverage_cannot_be_attached_to_a_different_anchor() {
        let fixture = fixture();
        let policy = policy(&fixture, 1).normalize().unwrap();
        let coverage = policy.validated_coverage().unwrap().unwrap();
        let mut plain = policy;
        plain.coverage = None;
        plain.input_commitment = Some(hex(&[7; 32]));
        let key = bytes(&fixture.verification_key).unwrap();
        let request = native(&fixture.query, &plain, &fixture.nonce, &key, None).unwrap();
        assert!(
            request
                .with_coverage(coverage, 2)
                .unwrap_err()
                .to_string()
                .contains("coverage dataset commitment")
        );
    }

    #[test]
    fn native_coverage_binding_requires_validation_at_request_creation() {
        let fixture = fixture();
        let mut policy = policy(&fixture, 1).normalize().unwrap();
        let coverage = policy.validated_coverage().unwrap().unwrap();
        policy.coverage = None;
        policy.not_before = Some(3);
        let key = bytes(&fixture.verification_key).unwrap();
        let request = native(&fixture.query, &policy, &fixture.nonce, &key, None).unwrap();
        for issued_at in [1, 6] {
            assert!(coverage.check_valid_at(issued_at).is_err());
            assert!(
                request
                    .clone()
                    .with_coverage(coverage.clone(), issued_at)
                    .unwrap_err()
                    .to_string()
                    .contains("coverage validity at request creation")
            );
        }
        assert!(request.clone().with_coverage(coverage.clone(), 2).is_ok());
        assert!(request.with_coverage(coverage, 5).is_ok());
    }

    #[test]
    fn native_request_rejects_a_bare_coverage_digest() {
        let fixture = fixture();
        let mut policy = policy(&fixture, 1).normalize().unwrap();
        policy.coverage = None;
        let key = bytes(&fixture.verification_key).unwrap();
        let plain = native(&fixture.query, &policy, &fixture.nonce, &key, None).unwrap();
        let build = |stored| {
            Request::new_authenticated(
                stored,
                plain.bounds(),
                key.clone(),
                plain.issuer_root().unwrap(),
            )
        };
        assert!(build(plain.stored().clone()).is_ok());
        let digest_only = plain
            .stored()
            .clone()
            .with_coverage_digest(Digest32::new([3; 32]).unwrap())
            .unwrap();
        assert!(
            build(digest_only)
                .unwrap_err()
                .to_string()
                .contains("coverage credential required")
        );
    }
}

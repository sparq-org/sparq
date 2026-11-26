use oxrdf::NamedNode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_query_protocol::Digest32;
use sparq_zk::sig;
use sparq_zk_query::{
    coverage::{Coverage, CoverageClaims, CoverageScope},
    envelope_policy::TrustMember,
};

use crate::{
    codec::{err, hex, word, Result},
    issuance::Credential,
    request::TrustMemberWire,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeWire {
    registry: String,
    subject: String,
    claim_types: Vec<String>,
    period_start: u64,
    period_end: u64,
    method: String,
    profile: String,
}

impl ScopeWire {
    fn native(&self) -> Result<CoverageScope> {
        let iri =
            |value: &str| NamedNode::new(value).map_err(|e| format!("coverage scope IRI: {e}"));
        Ok(CoverageScope {
            registry: iri(&self.registry)?,
            subject: iri(&self.subject)?,
            claim_types: self
                .claim_types
                .iter()
                .map(|s| iri(s))
                .collect::<Result<_>>()?,
            period_start: self.period_start,
            period_end: self.period_end,
            method: iri(&self.method)?,
            profile: iri(&self.profile)?,
        })
    }

    fn from_scope(scope: &CoverageScope) -> Self {
        Self {
            registry: scope.registry.as_str().into(),
            subject: scope.subject.as_str().into(),
            claim_types: scope
                .claim_types
                .iter()
                .map(|t| t.as_str().into())
                .collect(),
            period_start: scope.period_start,
            period_end: scope.period_end,
            method: scope.method.as_str().into(),
            profile: scope.profile.as_str().into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoverageWire {
    authority: TrustMemberWire,
    scope: ScopeWire,
    credential: Credential,
}

impl CoverageWire {
    pub fn normalize(&mut self) -> Result<()> {
        self.scope.claim_types.sort();
        self.scope.claim_types.dedup();
        self.authority.public_key = sig::public_key_to_hex(&key(&self.authority.public_key)?);
        self.credential.issuer = sig::public_key_to_hex(&key(&self.credential.issuer)?);
        Ok(())
    }

    pub fn validate(&self, anchor: Option<Digest32>) -> Result<Coverage> {
        let anchor = anchor.ok_or("coverage dataset commitment required")?;
        let authority = TrustMember {
            issuer: signer_iri(&self.authority.issuer)?,
            verification_method: signer_iri(&self.authority.verification_method)?,
            key: key(&self.authority.public_key)?,
        };
        if key(&self.credential.issuer)? != authority.key {
            return Err("coverage signer key differs".into());
        }
        Coverage::verify(
            self.credential
                .decode()
                .map_err(|e| format!("coverage signature: {e}"))?,
            &authority,
            &self.scope.native()?,
            anchor,
        )
        .map_err(err)
    }
}

pub(crate) fn document(input: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Input {
        issuer: String,
        verification_method: String,
        scope: ScopeWire,
        as_of: u64,
        expires_at: u64,
        input_commitment: String,
    }

    let input: Input = crate::codec::parse(input)?;
    let claims = CoverageClaims {
        issuer: signer_iri(&input.issuer)?,
        verification_method: signer_iri(&input.verification_method)?,
        scope: input.scope.native()?,
        as_of: input.as_of,
        expires_at: input.expires_at,
        input_commitment: Digest32::new(word(&input.input_commitment)?).map_err(err)?,
    };
    let (document, config) = claims.issuance_documents().map_err(err)?;
    Ok(json!({"graphNQuads": document, "status": {"proofConfigNQuads": config}}))
}

pub(crate) fn report(coverage: &Coverage) -> Value {
    let claims = coverage.claims();
    json!({
        "digest": hex(coverage.digest().as_bytes()),
        "issuer": claims.issuer.as_str(),
        "verificationMethod": claims.verification_method.as_str(),
        "scope": ScopeWire::from_scope(&claims.scope),
        "asOf": claims.as_of,
        "expiresAt": claims.expires_at,
        "inputCommitment": hex(claims.input_commitment.as_bytes())
    })
}

fn signer_iri(value: &str) -> Result<NamedNode> {
    NamedNode::new(value).map_err(|e| format!("coverage signer IRI: {e}"))
}

fn key(value: &str) -> Result<sig::PublicKey> {
    sig::public_key_from_hex(value).ok_or_else(|| "coverage signer key rejected".into())
}

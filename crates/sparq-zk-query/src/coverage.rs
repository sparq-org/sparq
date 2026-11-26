//! Detached, authority-signed coverage for an agreed credential dataset.

use std::collections::{BTreeMap, BTreeSet};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use oxrdf::{
    vocab::{rdf, xsd},
    GraphName, Literal, NamedNode, Term,
};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::{metadata, poseidon as suite};
use sparq_query_protocol::Digest32;

use crate::{envelope_policy::TrustMember, Error};

const VCQ: &str = "https://w3id.org/sparq/vcq#";
const VERIFICATION_METHOD: &str = "https://w3id.org/security#verificationMethod";
const MULTIBASE: &str = "https://w3id.org/security#multibase";
const MAX_TIME: u64 = (1 << 53) - 1;
const MAX_CLAIM_TYPES: usize = 64;
const MAX_RDF_BYTES: usize = 64 * 1024;

/// Verifier-selected scope and collection method for an authority's claim.
/// The historical interval is half-open and expressed in Unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageScope {
    pub registry: NamedNode,
    pub subject: NamedNode,
    pub claim_types: BTreeSet<NamedNode>,
    pub period_start: u64,
    pub period_end: u64,
    pub method: NamedNode,
    pub profile: NamedNode,
}

/// Public, signed claims for one ordered, salted dataset commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageClaims {
    pub issuer: NamedNode,
    pub verification_method: NamedNode,
    pub scope: CoverageScope,
    pub as_of: u64,
    pub expires_at: u64,
    pub input_commitment: Digest32,
}

impl CoverageClaims {
    /// Builds the claims document and suite proof configuration for issuance.
    pub fn issuance_documents(&self) -> Result<(String, String), Error> {
        self.check_times()?;
        check_types(&self.scope)?;
        let mut document = String::new();
        for (predicate, term, _) in fixed_claims(
            &self.issuer,
            &self.verification_method,
            &self.scope,
            &self.input_commitment,
        ) {
            document.push_str(&format!("_:coverage <{predicate}> {term} .\n"));
        }
        for claim in &self.scope.claim_types {
            document.push_str(&format!("_:coverage <{VCQ}coverageClaimType> {claim} .\n"));
        }
        for (predicate, instant) in [
            ("coverageAsOf", self.as_of),
            ("coverageExpiresAt", self.expires_at),
        ] {
            let term = seconds(instant);
            document.push_str(&format!("_:coverage <{VCQ}{predicate}> {term} .\n"));
        }
        let config = format!(
            "_:proof <https://w3id.org/security#cryptosuite> \"{}\" .\n\
             _:proof <{VERIFICATION_METHOD}> {} .\n",
            suite::SCHNORR_POSEIDON2_MERKLE_2026,
            self.verification_method,
        );
        check_size(&document, &config)?;
        Ok((document, config))
    }

    fn check_times(&self) -> Result<(), Error> {
        if self.scope.period_start >= self.scope.period_end || self.scope.period_end > self.as_of {
            return Err(invalid("period"));
        }
        if self.as_of >= self.expires_at || self.expires_at > MAX_TIME {
            return Err(invalid("validity"));
        }
        Ok(())
    }
}

/// Validated detached coverage evidence. Its signature is checked against a
/// verifier-authorized issuer, method and key for the exact expected scope.
#[derive(Clone, Debug)]
pub struct Coverage {
    credential: suite::Credential,
    claims: CoverageClaims,
    digest: Digest32,
}

impl Coverage {
    /// Checks the credential signature, authority, scope and dataset commitment.
    /// The request separately checks that its validity window fits the claims.
    pub fn verify(
        credential: suite::Credential,
        authority: &TrustMember,
        scope: &CoverageScope,
        anchor: Digest32,
    ) -> Result<Self, Error> {
        check_size(&credential.document, &credential.proof_config)?;
        check_types(scope)?;
        let mut properties = properties(&credential.document)?;
        for (predicate, expected, field) in fixed_claims(
            &authority.issuer,
            &authority.verification_method,
            scope,
            &anchor,
        ) {
            if properties.remove(&predicate).as_deref() != Some(&[expected]) {
                return Err(invalid(field));
            }
        }
        let types = properties
            .remove(&format!("{VCQ}coverageClaimType"))
            .ok_or_else(|| invalid("scope"))?;
        let types_len = types.len();
        let types = types
            .into_iter()
            .map(|term| match term {
                Term::NamedNode(iri) => Some(iri),
                _ => None,
            })
            .collect::<Option<BTreeSet<_>>>();
        if types_len != scope.claim_types.len() || types.as_ref() != Some(&scope.claim_types) {
            return Err(invalid("scope"));
        }
        let claims = CoverageClaims {
            issuer: authority.issuer.clone(),
            verification_method: authority.verification_method.clone(),
            scope: scope.clone(),
            as_of: take_seconds(&mut properties, "coverageAsOf")?,
            expires_at: take_seconds(&mut properties, "coverageExpiresAt")?,
            input_commitment: anchor,
        };
        if !properties.is_empty() {
            return Err(invalid("document"));
        }
        claims.check_times()?;
        let metadata = metadata::from_configuration(&credential.proof_config, &authority.key)
            .map_err(|_| invalid("signature configuration"))?;
        if metadata.verification_method != Some(authority.verification_method.clone().into()) {
            return Err(invalid("signer verification method"));
        }
        if !suite::verify(&credential, &authority.key) {
            return Err(invalid("signature"));
        }
        let canonical = sparq_canon::canonicalize_nquads(&credential.document)
            .map_err(|_| invalid("canonical claims"))?;
        let digest =
            Digest32::new(Sha256::digest(canonical.as_bytes()).into()).map_err(Error::Protocol)?;
        Ok(Self {
            credential,
            claims,
            digest,
        })
    }

    /// Returns the verified coverage claims.
    pub fn claims(&self) -> &CoverageClaims {
        &self.claims
    }

    /// Returns the original detached credential for storage with the request.
    pub fn credential(&self) -> &suite::Credential {
        &self.credential
    }

    /// Returns the SHA-256 digest of the RDFC-1.0 canonical signed claims.
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
}

fn fixed_claims(
    issuer: &NamedNode,
    verification_method: &NamedNode,
    scope: &CoverageScope,
    anchor: &Digest32,
) -> [(String, Term, &'static str); 10] {
    [
        (
            rdf::TYPE.as_str().into(),
            NamedNode::new_unchecked(format!("{VCQ}CoverageCredential")).into(),
            "document",
        ),
        (
            format!("{VCQ}coverageAuthority"),
            issuer.clone().into(),
            "signer",
        ),
        (
            VERIFICATION_METHOD.into(),
            verification_method.clone().into(),
            "signer",
        ),
        (
            format!("{VCQ}coverageRegistry"),
            scope.registry.clone().into(),
            "scope",
        ),
        (
            format!("{VCQ}coverageSubject"),
            scope.subject.clone().into(),
            "scope",
        ),
        (
            format!("{VCQ}coverageMethod"),
            scope.method.clone().into(),
            "method",
        ),
        (
            format!("{VCQ}coverageProfile"),
            scope.profile.clone().into(),
            "profile",
        ),
        (
            format!("{VCQ}coveragePeriodStart"),
            seconds(scope.period_start),
            "period",
        ),
        (
            format!("{VCQ}coveragePeriodEnd"),
            seconds(scope.period_end),
            "period",
        ),
        (
            format!("{VCQ}inputCommitment"),
            commitment_term(anchor),
            "dataset commitment",
        ),
    ]
}

fn properties(document: &str) -> Result<BTreeMap<String, Vec<Term>>, Error> {
    let mut properties = BTreeMap::<String, Vec<Term>>::new();
    let mut subject = None;
    for quad in oxttl::NQuadsParser::new().for_slice(document.as_bytes()) {
        let quad = quad.map_err(|_| invalid("document"))?;
        if quad.graph_name != GraphName::DefaultGraph
            || subject.get_or_insert(quad.subject.clone()) != &quad.subject
        {
            return Err(invalid("document"));
        }
        properties
            .entry(quad.predicate.into_string())
            .or_default()
            .push(quad.object);
    }
    Ok(properties)
}

fn take_seconds(
    properties: &mut BTreeMap<String, Vec<Term>>,
    predicate: &str,
) -> Result<u64, Error> {
    let value = properties.remove(&format!("{VCQ}{predicate}"));
    let Some([Term::Literal(literal)]) = value.as_deref() else {
        return Err(invalid("validity"));
    };
    let time = literal.value().parse().map_err(|_| invalid("validity"))?;
    if value.as_deref() != Some(&[seconds(time)]) {
        return Err(invalid("validity"));
    }
    Ok(time)
}

fn seconds(time: u64) -> Term {
    Literal::new_typed_literal(time.to_string(), xsd::NON_NEGATIVE_INTEGER).into()
}

fn commitment_term(anchor: &Digest32) -> Term {
    Literal::new_typed_literal(
        format!("u{}", URL_SAFE_NO_PAD.encode(anchor.as_bytes())),
        NamedNode::new_unchecked(MULTIBASE),
    )
    .into()
}

fn check_types(scope: &CoverageScope) -> Result<(), Error> {
    if scope.claim_types.is_empty() || scope.claim_types.len() > MAX_CLAIM_TYPES {
        return Err(invalid("scope claim types"));
    }
    Ok(())
}

fn check_size(document: &str, config: &str) -> Result<(), Error> {
    if document.len() > MAX_RDF_BYTES || config.len() > MAX_RDF_BYTES {
        return Err(invalid("document size"));
    }
    Ok(())
}

fn invalid(field: &str) -> Error {
    Error::Invalid(format!("coverage {field}"))
}

// [GPT-6] A request-bound NOW context, never an observed clock or freshness claim.
//! Versioned deterministic evaluation with an independently expected dateTime.
use crate::{DatasetAuthority, MAX_QUERY_BYTES, ProofContract, Provenance, Rejected, v3};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "graph-results")]
mod evaluate;
#[cfg(feature = "graph-results")]
pub use evaluate::{admit, evaluate};

pub use v3::{CanonicalResult, Policy, PrivateDataset};

/// Separate wire version; earlier requests never acquire an implicit clock.
pub const VERSION: u32 = 4;
/// Bounds validation and each cloned NOW literal, including fractional precision.
pub const MAX_NOW_BYTES: usize = 1024;

/// The exact lexical form of one xsd:dateTime with a required timezone.
///
/// The guest validates calendar, timezone and positive-year capacity. The context
/// is an agreed input; it does not attest to wall-clock time or freshness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NowContext {
    pub datetime: String,
}

/// Explicit evaluation context independently included in the expected request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContext {
    pub now: NowContext,
}

/// SPARQL 1.1 read forms with only NOW specialized from the bound context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    SparqSparql11NowContextV4,
}

/// Verifier-owned query, context, source authority, capacity and challenge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub contract: ProofContract,
    pub dialect: Dialect,
    pub query: String,
    pub context: ExecutionContext,
    pub authority: DatasetAuthority,
    pub policy: Policy,
    pub nonce: [u8; 32],
}

/// Complete private source; the caller supplies no evaluated output.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub request: Request,
    pub dataset: PrivateDataset,
}

/// Exact result and explicit context, bound to the original query and request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub request_digest: [u8; 32],
    pub dataset_commitment: [u8; 32],
    pub provenance: Provenance,
    pub context: ExecutionContext,
    pub result: CanonicalResult,
}

/// Checks structural request bounds before context parsing or source cloning.
///
/// Full dateTime validation occurs during admission/evaluation inside the guest.
///
/// # Errors
/// Rejects wrong versions, unsupported contracts, missing challenges or excess bytes.
pub fn validate_request(request: &Request) -> Result<(), Rejected> {
    if request.version != VERSION || request.contract != ProofContract::ExactDataset {
        return Err(Rejected("unsupported V4 proof contract or version"));
    }
    if request.query.len() > MAX_QUERY_BYTES || request.nonce == [0; 32] {
        return Err(Rejected("V4 query size or challenge rejected"));
    }
    if request.context.now.datetime.is_empty() || request.context.now.datetime.len() > MAX_NOW_BYTES
    {
        return Err(Rejected("V4 NOW context byte capacity"));
    }
    v3::validate_policy(&request.policy)
}

/// Commits to original query bytes and the complete independently expected context.
///
/// # Errors
/// Rejects structural request bounds or serialization failures.
pub fn request_digest(request: &Request) -> Result<[u8; 32], Rejected> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(|_| Rejected("V4 request serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:request:now-context:v4\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(hash.finalize().into())
}

/// Commits to the complete source/catalog and policy under a separate V4 domain.
///
/// The reusable source anchor is independent of the query's NOW context. The
/// request/journal bind time separately; neither anchor implies source authenticity.
///
/// # Errors
/// Rejects invalid source/catalog, salt and public capacity limits.
pub fn dataset_commitment(dataset: &PrivateDataset, policy: &Policy) -> Result<[u8; 32], Rejected> {
    let inner = v3::dataset_commitment(dataset, policy)?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:dataset:now-context:v4\0");
    hash.update(inner);
    Ok(hash.finalize().into())
}

/// Checks context, original request and source authority after receipt verification.
///
/// # Errors
/// Rejects substituted contexts/requests, cross-version outputs and stronger provenance.
pub fn bind_journal(journal: &Journal, expected: &Request) -> Result<(), Rejected> {
    if journal.version != VERSION
        || journal.request_digest != request_digest(expected)?
        || journal.context != expected.context
    {
        return Err(Rejected("V4 journal request or context mismatch"));
    }
    match expected.authority {
        DatasetAuthority::VerifierAgreed { commitment } => {
            if journal.dataset_commitment != commitment
                || journal.provenance != Provenance::VerifierAcceptedCommitment
            {
                return Err(Rejected("V4 journal dataset authority mismatch"));
            }
        }
        DatasetAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderDeclaredOnly {
                return Err(Rejected("V4 journal holder-declared provenance mismatch"));
            }
        }
    }
    Ok(())
}

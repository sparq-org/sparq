// [GPT-6] Experimental versioned blank-node and graph-result relation.
//! A separate request and result schema for bounded RDF graph production.
//!
//! V1/V2 remain distinct relations. Source labels are not copied as identifiers;
//! published blank nodes use result-local RDFC-1.0 identities across the table.

use crate::{DatasetAuthority, MAX_QUERY_BYTES, ProofContract, Provenance, Rejected, RowOrder, v2};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "graph-results")]
mod evaluate;
#[cfg(feature = "graph-results")]
mod result;
#[cfg(feature = "graph-results")]
pub use evaluate::{admit, evaluate};

/// Wire version for the blank-node and graph-result relation.
pub const VERSION: u32 = 3;

/// Explicit, deterministic implementation policy for DESCRIBE.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DescribePolicy {
    /// Outgoing triples from selected resources, recursively visiting blank objects.
    /// Inbound triples and reification closure are not included.
    OutgoingBlankNodeClosure,
}

/// Public algorithm and encoding limits; exceeding any bound rejects the query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalizationPolicy {
    pub max_quads: u32,
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,
    pub max_hndq_calls: u32,
    pub max_permutation_steps: u32,
}

impl Default for CanonicalizationPolicy {
    fn default() -> Self {
        // [GPT-6] Static profile ceilings bound table encoding as well as RDFC.
        Self {
            max_quads: 32_768,
            max_input_bytes: 1_048_576,
            max_output_bytes: 1_048_576,
            max_hndq_calls: 64,
            max_permutation_steps: 1_048_576,
        }
    }
}

/// Complete source, result and canonicalization capacities for V3.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub dataset: v2::Policy,
    pub canonicalization: CanonicalizationPolicy,
    pub describe: DescribePolicy,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            dataset: v2::Policy::default(),
            canonicalization: CanonicalizationPolicy::default(),
            describe: DescribePolicy::OutgoingBlankNodeClosure,
        }
    }
}

/// Pinned query syntax, dataset identity and result-encoding profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    SparqSparql11GraphResultsV3,
}

/// Independent verifier expectations; an accepted anchor is not source authentication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub contract: ProofContract,
    pub dialect: Dialect,
    pub query: String,
    pub authority: DatasetAuthority,
    pub policy: Policy,
    pub nonce: [u8; 32],
}

/// Exact N-Quads bytes, complete IRI graph catalog and hiding salt.
///
/// The source container is shared with V2; its V3 interpretation and commitment
/// domain are separate. Blank-node graph names and triple terms remain excluded.
pub type PrivateDataset = v2::PrivateDataset;

/// Private input; the evaluator constructs the result itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub request: Request,
    pub dataset: PrivateDataset,
}

/// Result-local identities preserve duplicates, unbound cells and cross-row identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalResult {
    Select {
        variables: Vec<String>,
        order: RowOrder,
        rows: Vec<Vec<Option<String>>>,
    },
    Ask(bool),
    /// RDFC-1.0 output for one default graph, hence canonical N-Triples.
    Graph {
        ntriples: String,
    },
}

/// Public V3 relation output, bound to the independently expected request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub request_digest: [u8; 32],
    pub dataset_commitment: [u8; 32],
    pub provenance: Provenance,
    pub result: CanonicalResult,
}

fn validate_policy(policy: &Policy) -> Result<(), Rejected> {
    v2::validate_policy(&policy.dataset)?;
    let c = &policy.canonicalization;
    let ceiling = CanonicalizationPolicy::default();
    if c.max_quads == 0
        || c.max_quads > ceiling.max_quads
        || c.max_input_bytes == 0
        || c.max_input_bytes > ceiling.max_input_bytes
        || c.max_output_bytes == 0
        || c.max_output_bytes > ceiling.max_output_bytes
        || c.max_hndq_calls == 0
        || c.max_hndq_calls > ceiling.max_hndq_calls
        || c.max_permutation_steps == 0
        || c.max_permutation_steps > ceiling.max_permutation_steps
    {
        return Err(Rejected(
            "V3 canonicalization policy exceeds program bounds",
        ));
    }
    Ok(())
}

/// Validates version, contract, challenge and every public capacity.
///
/// # Errors
/// Rejects unsupported contracts, missing challenges and excessive capacities.
pub fn validate_request(request: &Request) -> Result<(), Rejected> {
    if request.version != VERSION || request.contract != ProofContract::ExactDataset {
        return Err(Rejected("unsupported V3 proof contract or version"));
    }
    if request.query.len() > MAX_QUERY_BYTES || request.nonce == [0; 32] {
        return Err(Rejected("V3 query size or challenge rejected"));
    }
    validate_policy(&request.policy)
}

/// Hashes the complete independent request under the V3 domain.
///
/// # Errors
/// Rejects invalid requests and serialization failure.
pub fn request_digest(request: &Request) -> Result<[u8; 32], Rejected> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(|_| Rejected("V3 request serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:request:v3\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(hash.finalize().into())
}

/// Commits to the complete source, empty-graph catalog and V3 result policy.
///
/// The inner V2 source-byte commitment is reused as an explicitly domain-separated
/// component; the outer V3 domain also binds canonicalization and DESCRIBE policy.
/// Salt reuse remains publicly linkable. No issuer authenticity is established.
///
/// # Errors
/// Rejects source/catalog capacity violations, duplicate graph names and zero salt.
pub fn dataset_commitment(dataset: &PrivateDataset, policy: &Policy) -> Result<[u8; 32], Rejected> {
    validate_policy(policy)?;
    let inner = v2::dataset_commitment(dataset, &policy.dataset)?;
    let bytes = serde_json::to_vec(policy).map_err(|_| Rejected("V3 policy serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:dataset:nquads:catalog:source-bytes:graph-results:v3\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    hash.update(inner);
    Ok(hash.finalize().into())
}

/// Checks request and authority binding after cryptographic receipt verification.
///
/// # Errors
/// Rejects cross-version journals, substituted requests and stronger provenance claims.
pub fn bind_journal(journal: &Journal, expected: &Request) -> Result<(), Rejected> {
    if journal.version != VERSION || journal.request_digest != request_digest(expected)? {
        return Err(Rejected("V3 journal request mismatch"));
    }
    match &expected.authority {
        DatasetAuthority::VerifierAgreed { commitment } => {
            if *commitment != journal.dataset_commitment
                || journal.provenance != Provenance::VerifierAcceptedCommitment
            {
                return Err(Rejected("V3 journal dataset authority mismatch"));
            }
        }
        DatasetAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderDeclaredOnly {
                return Err(Rejected("V3 journal holder-declared provenance mismatch"));
            }
        }
    }
    Ok(())
}

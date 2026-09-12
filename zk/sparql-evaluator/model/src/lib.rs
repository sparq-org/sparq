// [GPT-6] Experimental exact-dataset relation, not externally audited.
//! Versioned requests, hiding input anchors, and exact result serialization.
//!
//! This crate's host evaluation is only a differential oracle. Cryptographic
//! verification lives in the companion host crate and requires a genuine receipt.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Complete default/named dataset relation with a separately versioned schema.
pub mod v2;

#[cfg(feature = "evaluate")]
mod aggregate_profile;
#[cfg(feature = "evaluate")]
mod evaluate;
#[cfg(feature = "evaluate")]
pub use evaluate::{admit, evaluate};

/// Wire version for this bounded default-graph experiment.
pub const VERSION: u32 = 1;
/// Maximum input source bytes admitted by the program, including whitespace.
pub const MAX_DATASET_BYTES: u32 = 65_536;
/// Maximum source triples, counted before RDF graph deduplication.
pub const MAX_TRIPLES: u32 = 256;
/// Maximum query bytes; the parser also enforces its own nesting limit.
pub const MAX_QUERY_BYTES: usize = 8_192;
/// Maximum materialized rows permitted by the bounded evaluator profile.
pub const MAX_ROWS: u32 = 4_096;
/// Byte ceiling for the pinned SDK's V1/V2 witness serialization, including framing.
///
/// Source and query strings are packed bytes. The margin bounds graph-catalog
/// lengths, resource fields, authority, salt, nonce and alignment padding.
pub const MAX_WITNESS_BYTES: usize = MAX_DATASET_BYTES as usize + MAX_QUERY_BYTES + 4_096;

/// Mathematical statement requested by a relying party.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofContract {
    /// Only released answers have supporting witnesses; use the Noir result API.
    SelectedSupport,
    /// The complete result of evaluation on the explicitly scoped input dataset.
    ExactDataset,
}

/// Versioned engine semantics; the method ID additionally pins its implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    /// Restricted SPARQL 1.1 surface using the foundation engine snapshot.
    SparqSparql11SnapshotV1,
}

/// Who chooses the input dataset, independent of evaluation exactness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatasetAuthority {
    /// The verifier supplies its independently accepted hiding commitment.
    VerifierAgreed { commitment: [u8; 32] },
    /// The holder chooses the input; no external completeness or authenticity.
    HolderDeclared,
}

/// Public resource limits committed into both the request and dataset anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub max_dataset_bytes: u32,
    pub max_triples: u32,
    pub max_rows: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_dataset_bytes: MAX_DATASET_BYTES,
            max_triples: MAX_TRIPLES,
            max_rows: MAX_ROWS,
        }
    }
}

/// A verifier-owned query request; never derive this from a presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub contract: ProofContract,
    pub dialect: Dialect,
    pub query: String,
    pub authority: DatasetAuthority,
    pub policy: Policy,
    /// Fresh application challenge; persistent single-use checks belong to verification.
    pub nonce: [u8; 32],
}

/// Private source and blinding; debug output intentionally redacts both.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateDataset {
    /// Exact UTF-8 N-Triples bytes for one complete default graph.
    pub ntriples: String,
    /// Independently sampled blinding; callers must use cryptographic entropy.
    pub salt: [u8; 32],
}

impl fmt::Debug for PrivateDataset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateDataset([REDACTED])")
    }
}

/// Private guest input; no caller-supplied result enters the relation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub request: Request,
    pub dataset: PrivateDataset,
}

/// Serialization mode preserves either multiplicities or outer query order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowOrder {
    /// Lexicographically sorted encoded rows, with every duplicate retained.
    Bag,
    /// Engine result order, including ORDER BY and its implementation tie policy.
    Sequence,
}

/// Released exact result; None cells are unbound, never the empty string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalResult {
    Select {
        variables: Vec<String>,
        order: RowOrder,
        rows: Vec<Vec<Option<String>>>,
    },
    Ask(bool),
}

/// Provenance is deliberately weaker than issuer authentication in this first slice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    /// Authenticity rests on the verifier's independently accepted dataset anchor.
    VerifierAcceptedCommitment,
    /// Only computation on the holder's chosen bytes is asserted.
    HolderDeclaredOnly,
}

/// Public guest output, bound to its complete request and dataset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub request_digest: [u8; 32],
    pub dataset_commitment: [u8; 32],
    pub provenance: Provenance,
    pub result: CanonicalResult,
}

/// A bounded-relation rejection, without private source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected(pub &'static str);

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Rejected {}

/// Validates resource limits and supported wire-level contract.
///
/// # Errors
/// Rejects unknown versions, selected-support routing, empty nonces and excessive limits.
pub fn validate_request(request: &Request) -> Result<(), Rejected> {
    if request.version != VERSION || request.contract != ProofContract::ExactDataset {
        return Err(Rejected("unsupported proof contract or version"));
    }
    if request.query.len() > MAX_QUERY_BYTES || request.nonce == [0; 32] {
        return Err(Rejected("query size or challenge rejected"));
    }
    let p = &request.policy;
    if p.max_dataset_bytes > MAX_DATASET_BYTES
        || p.max_triples > MAX_TRIPLES
        || p.max_rows == 0
        || p.max_rows > MAX_ROWS
    {
        return Err(Rejected("resource policy exceeds program bounds"));
    }
    Ok(())
}

/// Hashes the exact request under a versioned domain separator.
///
/// # Errors
/// Rejects unsupported wire-level requests or serialization failure.
pub fn request_digest(request: &Request) -> Result<[u8; 32], Rejected> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(|_| Rejected("request serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:request:v1\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(hash.finalize().into())
}

/// Commits to complete source bytes, blinding, format, and capacity policy.
///
/// The source is exact N-Triples, not RDF-isomorphism canonicalization. Equivalent
/// documents may have different anchors. Reusing an anchor is publicly linkable.
///
/// # Errors
/// Rejects source bytes beyond the declared capacity or an all-zero blinding.
pub fn dataset_commitment(dataset: &PrivateDataset, policy: &Policy) -> Result<[u8; 32], Rejected> {
    if dataset.ntriples.len() > policy.max_dataset_bytes as usize || dataset.salt == [0; 32] {
        return Err(Rejected("dataset byte capacity or blinding rejected"));
    }
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:default-graph:ntriples:source-bytes:v1\0");
    hash.update(policy.max_dataset_bytes.to_le_bytes());
    hash.update(policy.max_triples.to_le_bytes());
    hash.update(policy.max_rows.to_le_bytes());
    hash.update(dataset.salt);
    hash.update((dataset.ntriples.len() as u64).to_le_bytes());
    hash.update(dataset.ntriples.as_bytes());
    Ok(hash.finalize().into())
}

/// Checks the public journal against independent verifier expectations.
///
/// This is request binding only, not proof verification. The host crate verifies
/// the receipt and locally pinned method ID before accepting this output.
///
/// # Errors
/// Rejects substituted requests, dataset anchors and provenance modes.
pub fn bind_journal(journal: &Journal, expected: &Request) -> Result<(), Rejected> {
    if journal.version != VERSION || journal.request_digest != request_digest(expected)? {
        return Err(Rejected("journal request mismatch"));
    }
    match &expected.authority {
        DatasetAuthority::VerifierAgreed { commitment } => {
            if *commitment != journal.dataset_commitment
                || journal.provenance != Provenance::VerifierAcceptedCommitment
            {
                return Err(Rejected("journal dataset authority mismatch"));
            }
        }
        DatasetAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderDeclaredOnly {
                return Err(Rejected("journal holder-declared provenance mismatch"));
            }
        }
    }
    Ok(())
}

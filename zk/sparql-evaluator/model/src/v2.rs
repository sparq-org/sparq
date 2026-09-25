// [GPT-6] Versioned complete default/named dataset relation; not externally audited.
//! Complete N-Quads source and an explicit catalog of IRI-named graphs.
//!
//! Empty named graphs must appear in the catalog even though N-Quads cannot
//! represent them. This schema is separate from the V1 default-graph relation;
//! V1 requests, source commitments and journals retain their existing meaning.

use crate::{
    CanonicalResult, DatasetAuthority, MAX_DATASET_BYTES, MAX_QUERY_BYTES, MAX_ROWS, MAX_TRIPLES,
    ProofContract, Provenance, Rejected,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

#[cfg(feature = "evaluate")]
mod evaluate;
#[cfg(feature = "evaluate")]
pub use evaluate::{admit, evaluate};

/// Wire version for the complete default/named graph relation.
pub const VERSION: u32 = 2;
/// Maximum named graphs, including empty graphs, in the complete input catalog.
pub const MAX_NAMED_GRAPHS: u32 = 16;

/// A pinned SPARQL 1.1 dataset profile, further identified by the guest image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    SparqSparql11DatasetV2,
}

/// Public source and execution capacities included in the dataset commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Sum of exact N-Quads source bytes and catalog IRI bytes.
    pub max_dataset_bytes: u32,
    /// Source quads across all graphs, counted before per-graph deduplication.
    pub max_triples: u32,
    pub max_rows: u32,
    /// Complete input catalog capacity; query-derived dataset clauses have a
    /// separate static reference bound in the pinned admission profile.
    pub max_named_graphs: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_dataset_bytes: MAX_DATASET_BYTES,
            max_triples: MAX_TRIPLES,
            max_rows: MAX_ROWS,
            max_named_graphs: MAX_NAMED_GRAPHS,
        }
    }
}

/// Independent verifier expectations for the V2 dataset relation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The version remains the first wire word, as in V1.
    pub version: u32,
    pub contract: ProofContract,
    pub dialect: Dialect,
    pub query: String,
    pub authority: DatasetAuthority,
    pub policy: Policy,
    pub nonce: [u8; 32],
}

/// Complete private source, named-graph catalog and hiding salt.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateDataset {
    /// Complete exact UTF-8 N-Quads source, including default-graph statements.
    pub nquads: String,
    /// Every IRI-named graph, including empty graphs. Duplicates are rejected.
    ///
    /// The commitment sorts exact IRI strings; input catalog order has no meaning.
    /// The evaluator validates IRI syntax and rejects a source quad whose named
    /// graph is missing from this catalog. Blank-node graph names are unsupported.
    pub named_graphs: Vec<String>,
    /// Independently sampled cryptographic salt; never reuse fixture salts.
    pub salt: [u8; 32],
}

impl fmt::Debug for PrivateDataset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateDatasetV2([REDACTED])")
    }
}

/// Private guest input; evaluation constructs its own result.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub request: Request,
    pub dataset: PrivateDataset,
}

/// Exact public result bound to a V2 request and complete input dataset.
///
/// V1 and V2 currently share a journal field layout. Successful decoding does
/// not select a relation: after receipt verification, `bind_journal` must check
/// the version and the separately domain-bound expected request.
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
    if policy.max_dataset_bytes > MAX_DATASET_BYTES
        || policy.max_triples > MAX_TRIPLES
        || policy.max_rows == 0
        || policy.max_rows > MAX_ROWS
        || policy.max_named_graphs > MAX_NAMED_GRAPHS
    {
        return Err(Rejected("V2 resource policy exceeds program bounds"));
    }
    Ok(())
}

/// Validates the version, contract, challenge and resource capacities.
///
/// # Errors
/// Rejects unsupported versions, selected-support routing and excessive inputs.
pub fn validate_request(request: &Request) -> Result<(), Rejected> {
    if request.version != VERSION || request.contract != ProofContract::ExactDataset {
        return Err(Rejected("unsupported V2 proof contract or version"));
    }
    if request.query.len() > MAX_QUERY_BYTES || request.nonce == [0; 32] {
        return Err(Rejected("V2 query size or challenge rejected"));
    }
    validate_policy(&request.policy)
}

/// Hashes an independent V2 request under a distinct versioned domain.
///
/// # Errors
/// Rejects invalid requests or serialization failure.
pub fn request_digest(request: &Request) -> Result<[u8; 32], Rejected> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request).map_err(|_| Rejected("V2 request serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:request:v2\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(hash.finalize().into())
}

fn canonical_catalog<'a>(
    dataset: &'a PrivateDataset,
    policy: &Policy,
) -> Result<Vec<&'a str>, Rejected> {
    validate_policy(policy)?;
    if dataset.named_graphs.len() > policy.max_named_graphs as usize || dataset.salt == [0; 32] {
        return Err(Rejected("V2 graph capacity or blinding rejected"));
    }
    let bytes = dataset
        .named_graphs
        .iter()
        .try_fold(dataset.nquads.len(), |size, name| {
            size.checked_add(name.len())
                .ok_or(Rejected("V2 dataset byte capacity rejected"))
        })?;
    if bytes > policy.max_dataset_bytes as usize {
        return Err(Rejected("V2 dataset byte capacity rejected"));
    }
    let mut names: Vec<_> = dataset.named_graphs.iter().map(String::as_str).collect();
    names.sort_unstable();
    if names.iter().any(|name| name.is_empty()) || names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Rejected("V2 named graph catalog is not unique"));
    }
    Ok(names)
}

/// Commits to the complete source and catalog, including all empty named graphs.
///
/// Catalog IRI strings are sorted without IRI normalization. N-Quads bytes remain
/// exact source bytes, not RDF canonicalization. The private salt hides the input;
/// reusing a commitment is publicly linkable. This commitment establishes no
/// source authenticity or wallet completeness by itself.
///
/// # Errors
/// Rejects capacity violations, duplicate names and all-zero blinding. Syntax and
/// source-to-catalog membership are checked during guest evaluation.
pub fn dataset_commitment(dataset: &PrivateDataset, policy: &Policy) -> Result<[u8; 32], Rejected> {
    let catalog = canonical_catalog(dataset, policy)?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:dataset:nquads:sorted-iri-catalog:source-bytes:v2\0");
    hash.update(policy.max_dataset_bytes.to_le_bytes());
    hash.update(policy.max_triples.to_le_bytes());
    hash.update(policy.max_rows.to_le_bytes());
    hash.update(policy.max_named_graphs.to_le_bytes());
    hash.update(dataset.salt);
    hash.update((catalog.len() as u32).to_le_bytes());
    for name in catalog {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
    }
    hash.update((dataset.nquads.len() as u64).to_le_bytes());
    hash.update(dataset.nquads.as_bytes());
    Ok(hash.finalize().into())
}

/// Checks independent request and authority expectations after receipt verification.
///
/// # Errors
/// Rejects cross-version journals, request substitution and authority mismatches.
pub fn bind_journal(journal: &Journal, expected: &Request) -> Result<(), Rejected> {
    if journal.version != VERSION || journal.request_digest != request_digest(expected)? {
        return Err(Rejected("V2 journal request mismatch"));
    }
    match &expected.authority {
        DatasetAuthority::VerifierAgreed { commitment } => {
            if *commitment != journal.dataset_commitment
                || journal.provenance != Provenance::VerifierAcceptedCommitment
            {
                return Err(Rejected("V2 journal dataset authority mismatch"));
            }
        }
        DatasetAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderDeclaredOnly {
                return Err(Rejected("V2 journal holder-declared provenance mismatch"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset() -> PrivateDataset {
        PrivateDataset {
            nquads: "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/full> .\n".into(),
            named_graphs: vec!["http://ex/full".into(), "http://ex/empty".into()],
            salt: [17; 32],
        }
    }

    #[test]
    fn catalog_order_is_irrelevant_but_empty_graph_membership_is_committed() {
        let mut dataset = dataset();
        let policy = Policy::default();
        let expected = dataset_commitment(&dataset, &policy).unwrap();
        dataset.named_graphs.reverse();
        assert_eq!(dataset_commitment(&dataset, &policy).unwrap(), expected);
        dataset
            .named_graphs
            .retain(|name| name != "http://ex/empty");
        assert_ne!(dataset_commitment(&dataset, &policy).unwrap(), expected);
        dataset.named_graphs.push("http://ex/renamed-empty".into());
        assert_ne!(dataset_commitment(&dataset, &policy).unwrap(), expected);
    }

    #[test]
    fn capacity_source_and_salt_are_bound() {
        let dataset = dataset();
        let policy = Policy::default();
        let expected = dataset_commitment(&dataset, &policy).unwrap();
        let mut modified = dataset.clone();
        modified.nquads.push('\n');
        assert_ne!(dataset_commitment(&modified, &policy).unwrap(), expected);
        modified = dataset.clone();
        modified.salt[0] ^= 1;
        assert_ne!(dataset_commitment(&modified, &policy).unwrap(), expected);
        for field in 0..4 {
            let mut policy = policy.clone();
            match field {
                0 => policy.max_dataset_bytes -= 1,
                1 => policy.max_triples -= 1,
                2 => policy.max_rows -= 1,
                _ => policy.max_named_graphs -= 1,
            }
            assert_ne!(dataset_commitment(&dataset, &policy).unwrap(), expected);
        }
        let v1 = crate::PrivateDataset {
            ntriples: dataset.nquads.clone(),
            salt: dataset.salt,
        };
        assert_ne!(
            crate::dataset_commitment(&v1, &crate::Policy::default()).unwrap(),
            expected
        );
    }

    #[test]
    fn duplicate_names_and_capacity_exhaustion_fail_before_evaluation() {
        let mut dataset = dataset();
        dataset.named_graphs.push(dataset.named_graphs[0].clone());
        assert!(dataset_commitment(&dataset, &Policy::default()).is_err());
        dataset.named_graphs.pop();
        let mut policy = Policy {
            max_named_graphs: 1,
            ..Policy::default()
        };
        assert!(dataset_commitment(&dataset, &policy).is_err());
        policy = Policy::default();
        // Catalog bytes participate in the same bounded private-input capacity.
        policy.max_dataset_bytes = dataset.nquads.len() as u32;
        assert!(dataset_commitment(&dataset, &policy).is_err());
        dataset.salt = [0; 32];
        assert!(dataset_commitment(&dataset, &Policy::default()).is_err());
    }

    #[test]
    fn both_authorities_bind_version_request_and_provenance() {
        let policy = Policy::default();
        let commitment = dataset_commitment(&dataset(), &policy).unwrap();
        let mut request = Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11DatasetV2,
            query: "ASK {}".into(),
            authority: DatasetAuthority::VerifierAgreed { commitment },
            policy,
            nonce: [29; 32],
        };
        let mut journal = Journal {
            version: VERSION,
            request_digest: request_digest(&request).unwrap(),
            dataset_commitment: commitment,
            provenance: Provenance::VerifierAcceptedCommitment,
            result: CanonicalResult::Ask(true),
        };
        bind_journal(&journal, &request).unwrap();
        journal.dataset_commitment[0] ^= 1;
        assert!(bind_journal(&journal, &request).is_err());
        journal.dataset_commitment = commitment;
        request.authority = DatasetAuthority::HolderDeclared;
        assert!(bind_journal(&journal, &request).is_err());
        journal.request_digest = request_digest(&request).unwrap();
        assert!(bind_journal(&journal, &request).is_err());
        journal.provenance = Provenance::HolderDeclaredOnly;
        bind_journal(&journal, &request).unwrap();
        journal.version = crate::VERSION;
        assert!(bind_journal(&journal, &request).is_err());
    }
}

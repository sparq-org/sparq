//! TEE attestation: `urn:sparq:vcq:method:tee-attestation` v1, platform `aws-nitro`.
//!
//! The holder sends the request, its credentials and the commitment salt to a
//! program running in an AWS Nitro Enclave. The program runs the same relation
//! as `disclosed-reevaluation` and the `risc0-authenticated-rdf` v5 guest
//! ([`auth::evaluate`]): it checks every `eddsa-rdfc-2022` signature against
//! the request's key table, builds the input dataset and evaluates the query.
//! It then asks the Nitro Secure Module for an attestation document whose
//! `user_data` is the [statement digest](statement_digest) and whose `nonce` is
//! the request's nonce. The `proof` member is that document.
//!
//! The verifier checks the document's certificate chain up to the pinned AWS
//! Nitro Enclaves root, its COSE_Sign1 ES384 signature, that PCR0 (the
//! measurement of the enclave image) is one the verifier accepts, that the
//! document is fresh, and that `user_data` equals the digest of the stated
//! statement. See [`verify`].
//!
//! What the verifier must trust: AWS's attestation keys and Nitro hypervisor,
//! the measured enclave program (identified by PCR0, which a verifier obtains
//! by building the image reproducibly or from a party it trusts), and the
//! enclave's isolation, including against side channels. The proof is not
//! zero-knowledge in the cryptographic sense: the verifier learns the statement
//! and the attestation document, which names the enclave module and the PCRs;
//! the credentials stay with the holder and the enclave. The evidence is
//! publicly verifiable: anyone holding it, the request and the trust policy
//! can check it, so it is transferable.
//!
//! Research prototype, not externally audited. The relation's limits are those
//! of [`auth`].

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Journal, Request};

pub use sparq_proved_evaluator_model::Rejected;

pub mod nitro;
pub mod protocol;

/// Proof-method identifier.
pub const METHOD_ID: &str = "urn:sparq:vcq:method:tee-attestation";
/// Proof-method version.
pub const METHOD_VERSION: u32 = 1;
/// Kind of evidence, as named in the answer specification.
pub const EVIDENCE_KIND: &str = "attestation";
/// The `parameters` platform this crate implements.
pub const PLATFORM: &str = "aws-nitro";

/// What a presentation states: the evaluation result and its input commitment.
pub type Statement = Journal;

/// The `proof` member of a presentation under this method.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeeProof {
    /// [`METHOD_VERSION`].
    pub version: u32,
    /// [`PLATFORM`].
    pub platform: String,
    /// The Nitro attestation document: a CBOR-encoded COSE_Sign1.
    pub attestation: Vec<u8>,
}

/// What the verifier accepts.
#[derive(Clone, Debug)]
pub struct TrustPolicy {
    /// DER of the trust anchor; [`nitro::aws_root_g1`] for AWS.
    pub root: Vec<u8>,
    /// Accepted PCR0 values (SHA-384 measurements of the enclave image).
    pub pcr0: Vec<[u8; 48]>,
    /// Oldest accepted document, in milliseconds before the verifier's clock.
    pub max_age_ms: u64,
}

/// SHA-256 over a domain tag and the compact JSON encoding of `statement`.
///
/// The encoding is deterministic: [`Journal`] is a struct of fixed field order
/// with no maps, and byte arrays encode as arrays of numbers.
pub fn statement_digest(statement: &Statement) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:tee-attestation:statement:v1\0");
    hash.update(serde_json::to_vec(statement).expect("Journal serializes"));
    hash.finalize().into()
}

/// Verifier side: checks the presentation's `statement` and `proof` against
/// the verifier's stored `request` and its trust `policy`, at time `now_ms`
/// (milliseconds since the Unix epoch).
///
/// `request` must be the verifier's own stored request. Replay protection
/// (consuming the request's challenge) is the caller's step after this
/// returns `Ok`.
///
/// # Errors
/// Another version or platform; a request in another signature mode than
/// hidden; a statement whose request digest does not
/// match `request`; an attestation document that is malformed, not signed by a
/// chain to `policy.root`, too old, from an enclave whose PCR0 is not
/// accepted, or whose `user_data` or `nonce` does not bind this statement and
/// request.
pub fn verify(
    request: &Request,
    statement: &Statement,
    proof: &TeeProof,
    policy: &TrustPolicy,
    now_ms: u64,
) -> Result<Statement, Rejected> {
    if proof.version != METHOD_VERSION {
        return Err(Rejected("unsupported TEE proof version"));
    }
    if proof.platform != PLATFORM {
        return Err(Rejected("unsupported TEE platform"));
    }
    if request.policy.signature_mode != auth::SignatureMode::Hidden {
        return Err(Rejected(
            "TEE attestation supports the hidden signature mode only",
        ));
    }
    auth::bind_journal(statement, request)?;
    let document = nitro::verify(&proof.attestation, &policy.root, now_ms, policy.max_age_ms)?;
    let pcr0 = document
        .pcrs
        .iter()
        .find(|(index, _)| *index == 0)
        .map(|(_, value)| value.as_slice())
        .ok_or(Rejected("attestation has no PCR0"))?;
    if pcr0.iter().all(|byte| *byte == 0) {
        return Err(Rejected("attestation is from a debug-mode enclave"));
    }
    if !policy
        .pcr0
        .iter()
        .any(|accepted| accepted.as_slice() == pcr0)
    {
        return Err(Rejected("enclave image is not accepted"));
    }
    if document.user_data.as_deref() != Some(statement_digest(statement).as_slice()) {
        return Err(Rejected("attestation does not bind the stated statement"));
    }
    if document.nonce.as_deref() != Some(request.nonce.as_slice()) {
        return Err(Rejected("attestation does not bind the request nonce"));
    }
    Ok(statement.clone())
}

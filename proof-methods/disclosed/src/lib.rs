//! Disclosed-credential re-evaluation: `urn:sparq:vcq:method:disclosed-reevaluation` v1.
//!
//! The evidence is the credentials themselves. The holder sends every
//! credential in the input dataset together with the salt of the dataset
//! commitment. The verifier checks each `eddsa-rdfc-2022` signature against its
//! own key table, builds the input dataset and evaluates the query. It accepts
//! the presentation only when its own result and dataset commitment equal the
//! ones the presentation states.
//!
//! This is not a zero-knowledge method. The verifier learns every disclosed
//! credential, including its signature, which is the same in every
//! presentation and so links them. A verifier lists the `disclosed` signature
//! mode only when it may see that data.
//!
//! The relation is exactly the one the `risc0-authenticated-rdf` v5 guest
//! proves ([`auth::evaluate`]), run natively by the verifier instead of inside
//! a zkVM. Both methods therefore compute the same dataset commitment
//! ([`auth::dataset_commitment`]) for the same credentials and salt, so a
//! verifier-agreed commitment can be answered by either method. The input
//! commitment of a presentation is that value.
//!
//! Where SPARQL 1.1 permits more than one result (OFFSET or LIMIT without a
//! strict ORDER BY, REDUCED, SAMPLE, GROUP_CONCAT, floating-point aggregates),
//! the holder makes the choice through the set of credentials it sends (the
//! relation orders them by document hash, so their order does not matter), and
//! the verifier accepts exactly the result the shared evaluator computes for
//! that set. The method publishes no rule for which permitted
//! result that is, so a verifier that needs one particular result writes a
//! query whose result is unique.
//!
//! This crate is an evaluation baseline over the internal v5 [`Request`] and
//! [`Journal`]. It does not yet implement the full proof-method profile of the
//! answer specification: the protocol request (audience, acceptance window,
//! accepted methods, signature mode) is not bound into the statement, and the
//! canonical statement and proof encodings are not defined.
//!
//! Research prototype, not externally audited. The bounded canonical RDF
//! profile and its limits are those of [`auth`]: no JSON-LD processing, no
//! credential status, no holder binding and no validity-period check.

use serde::{Deserialize, Serialize};
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, Journal, PrivateCredentials, Request, SignedCredential, Witness,
};

pub use sparq_proved_evaluator_model::Rejected;

mod admission;
#[cfg(feature = "fixtures")]
pub mod fixtures;

pub use admission::MAX_PROOF_JSON_BYTES;

/// Proof-method identifier.
pub const METHOD_ID: &str = "urn:sparq:vcq:method:disclosed-reevaluation";
/// Proof-method version.
pub const METHOD_VERSION: u32 = 1;
/// Kind of evidence, as named in the answer specification.
pub const EVIDENCE_KIND: &str = "disclosed credentials";
/// Signature mode this method uses.
pub const SIGNATURE_MODE: &str = "disclosed";

/// The `proof` member of a presentation under this method.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosedProof {
    /// [`METHOD_VERSION`].
    pub version: u32,
    /// Every credential in the input dataset, in the holder's order.
    pub credentials: Vec<SignedCredential>,
    /// Salt of the dataset commitment.
    pub salt: [u8; 32],
}

/// Decodes a JSON `proof` member, rejecting an encoding longer than
/// [`MAX_PROOF_JSON_BYTES`] before parsing it.
///
/// # Errors
/// An oversized or malformed encoding, or an unknown field.
pub fn decode_proof(bytes: &[u8]) -> Result<DisclosedProof, Rejected> {
    if bytes.len() > MAX_PROOF_JSON_BYTES {
        return Err(Rejected("disclosed proof encoding capacity"));
    }
    serde_json::from_slice(bytes).map_err(|_| Rejected("malformed disclosed proof"))
}

/// What a presentation states: the evaluation result and its input commitment.
pub type Statement = Journal;

/// Holder side: evaluates the request over the credentials it is about to
/// disclose and returns the statement and the proof member.
///
/// The holder runs the verifier's own check first, so it never sends a
/// presentation that the verifier would reject and sees exactly what it
/// discloses.
///
/// # Errors
/// A query this method does not admit, a request or credentials over the
/// relation's capacities (checked before either is copied), or any rejection
/// of [`auth::evaluate`]: an invalid request, a credential whose
/// signature, issuer or key does not match the request's table, a commitment
/// unequal to a verifier-agreed one, or exceeded capacities.
pub fn present(
    request: &Request,
    credentials: Vec<SignedCredential>,
    salt: [u8; 32],
) -> Result<(Statement, DisclosedProof), Rejected> {
    admission::admit_request(request)?;
    admission::admit_sizes(&credentials)?;
    let witness = Witness {
        request: request.clone(),
        dataset: PrivateCredentials { credentials, salt },
    };
    let statement = auth::evaluate(&witness)?;
    let proof = DisclosedProof {
        version: METHOD_VERSION,
        credentials: witness.dataset.credentials,
        salt: witness.dataset.salt,
    };
    Ok((statement, proof))
}

/// Verifier side: re-evaluates `request` over the disclosed credentials and
/// checks the presentation's `statement`.
///
/// `request` must be the verifier's own stored request, never one taken from
/// the presentation. Replay protection (consuming the request's challenge) is
/// the caller's step after this returns `Ok`.
///
/// # Errors
/// Another proof version, a query this method does not admit, a request or
/// credentials over the relation's capacities (checked before either is
/// copied), any
/// rejection of [`auth::evaluate`], or a stated result, commitment, provenance or request digest that differs from the
/// verifier's own evaluation.
pub fn verify(
    request: &Request,
    statement: &Statement,
    proof: &DisclosedProof,
) -> Result<Statement, Rejected> {
    if proof.version != METHOD_VERSION {
        return Err(Rejected("unsupported disclosed proof version"));
    }
    admission::admit_request(request)?;
    admission::admit_sizes(&proof.credentials)?;
    let dataset = PrivateCredentials {
        credentials: proof.credentials.clone(),
        salt: proof.salt,
    };
    let own = auth::evaluate(&Witness {
        request: request.clone(),
        dataset,
    })?;
    auth::bind_journal(&own, request)?;
    if own != *statement {
        return Err(Rejected(
            "stated answer differs from the verifier's evaluation",
        ));
    }
    Ok(own)
}

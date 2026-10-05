// [GPT-6] Explicit V2 APIs preserve V1 wire and commitment semantics.
//! Complete named-dataset receipts bound to independently supplied V2 requests.

use crate::{AcceptedGuest, Error, Nonces, Presentation, prove_serialized, verify_receipt};
use sparq_proved_evaluator_methods::{SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID};
use sparq_proved_evaluator_model::v2::{
    Journal, Request, Witness, bind_journal, dataset_commitment, validate_request,
};
use std::path::Path;

/// Proves a complete V2 dataset using the locally embedded guest.
///
/// # Errors
/// Rejects invalid requests/data, failed execution and non-Succinct receipts.
pub fn prove(witness: &Witness, r0vm: &Path) -> Result<Presentation, Error> {
    prove_program(witness, r0vm, SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID)
}

/// Proves V2 evaluation with an independently accepted deployment artifact.
///
/// An accepted V1-only guest cannot satisfy this V2 relation. The application
/// must approve the program's versioned semantics, as well as its artifact pin.
///
/// # Errors
/// Rejects invalid requests/data, unsupported guest versions and failed proofs.
pub fn prove_with_artifact(
    witness: &Witness,
    r0vm: &Path,
    guest: &AcceptedGuest,
) -> Result<Presentation, Error> {
    prove_program(witness, r0vm, &guest.artifact, guest.image_id)
}

fn prove_program(
    witness: &Witness,
    r0vm: &Path,
    artifact: &[u8],
    image_id: [u32; 8],
) -> Result<Presentation, Error> {
    validate_request(&witness.request).map_err(|_| Error("invalid V2 proof request"))?;
    dataset_commitment(&witness.dataset, &witness.request.policy)
        .map_err(|_| Error("V2 private input capacity rejected"))?;
    let presentation = prove_serialized(witness, r0vm, artifact)?;
    checked_journal(&presentation, &witness.request, image_id)?;
    Ok(presentation)
}

fn checked_journal(
    presentation: &Presentation,
    expected: &Request,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    validate_request(expected).map_err(|_| Error("invalid V2 expected request"))?;
    verify_receipt(presentation, image_id)?;
    let journal: Journal = presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("V2 journal decoding rejected"))?;
    bind_journal(&journal, expected)
        .map_err(|_| Error("independent V2 request binding rejected"))?;
    Ok(journal)
}

/// Verifies a V2 receipt, independent request and single-use challenge.
///
/// Request and program expectations are verifier-owned configuration. Preserve
/// returned provenance: holder-declared exact evaluation does not establish
/// issuer authentication, status validity or wallet completeness.
///
/// # Errors
/// Rejects program/request/catalog tampering, replay and nonce-storage failure.
pub fn verify(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
) -> Result<Journal, Error> {
    verify_program(presentation, expected, nonces, SPARQ_EXACT_GUEST_ID)
}

/// Verifies V2 evaluation against an independently accepted common artifact.
///
/// # Errors
/// Rejects wrong versions, identities, request bindings, replay and storage failure.
pub fn verify_with_artifact(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
) -> Result<Journal, Error> {
    verify_program(presentation, expected, nonces, guest.image_id)
}

fn verify_program(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    let journal = checked_journal(presentation, expected, image_id)?;
    if !nonces.consume(expected.nonce)? {
        return Err(Error("challenge already consumed"));
    }
    Ok(journal)
}

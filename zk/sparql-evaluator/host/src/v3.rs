// [GPT-6] Explicit V3 APIs preserve earlier versioned wire and commitment semantics.
//! Blank-node and graph-result receipts bound to independently supplied V3 requests.

use crate::{AcceptedGuest, Error, Nonces, Presentation, prove_serialized, verify_receipt};
use sparq_proved_evaluator_methods::{SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID};
use sparq_proved_evaluator_model::v3::{
    Journal, Request, Witness, bind_journal, dataset_commitment, validate_request,
};
use std::{convert::Infallible, path::Path};

/// Proves a complete V3 dataset using the locally embedded guest.
///
/// # Errors
/// Rejects invalid requests/data, failed execution and non-Succinct receipts.
pub fn prove(witness: &Witness, r0vm: &Path) -> Result<Presentation, Error> {
    prove_program(witness, r0vm, SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID)
}

/// Proves V3 evaluation with an independently accepted deployment artifact.
///
/// An accepted V1/V2-only guest cannot satisfy this V3 relation. The application
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
    validate_request(&witness.request).map_err(|_| Error("invalid V3 proof request"))?;
    dataset_commitment(&witness.dataset, &witness.request.policy)
        .map_err(|_| Error("V3 private input capacity rejected"))?;
    let presentation = prove_serialized(witness, r0vm, artifact)?;
    checked_journal(&presentation, &witness.request, image_id)?;
    Ok(presentation)
}

fn checked_journal(
    presentation: &Presentation,
    expected: &Request,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    validate_request(expected).map_err(|_| Error("invalid V3 expected request"))?;
    verify_receipt(presentation, image_id)?;
    let journal: Journal = presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("V3 journal decoding rejected"))?;
    bind_journal(&journal, expected)
        .map_err(|_| Error("independent V3 request binding rejected"))?;
    Ok(journal)
}

/// Verifies a V3 receipt, independent request and single-use challenge.
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

/// Verifies V3 evaluation against an independently accepted common artifact.
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
    // [OPUS-5.5] The public APIs run no extra check; behavior is unchanged.
    let no_check = |_: &Journal| Ok::<(), Infallible>(());
    match verify_checked_program(presentation, expected, nonces, image_id, no_check) {
        Ok((journal, ())) => Ok(journal),
        Err(CheckedFailure::Verification(error)) => Err(error),
        Err(CheckedFailure::Check(never)) => match never {},
    }
}

/// [OPUS-5.5] Failure of a checked V3 verification.
pub(crate) enum CheckedFailure<E> {
    /// Receipt, request binding or nonce consumption failed.
    Verification(Error),
    /// The caller's check rejected the verified, request-bound journal.
    Check(E),
}

/// [OPUS-5.5] Verifies like [`verify_with_artifact`], plus a caller check.
///
/// `check` sees the journal only after Succinct receipt verification, journal
/// decoding and independent request binding, and runs BEFORE `nonces` is
/// called. A rejected check therefore never consumes the challenge.
#[cfg(feature = "vcq")]
pub(crate) fn verify_checked_with_artifact<T, E>(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
    check: impl FnOnce(&Journal) -> Result<T, E>,
) -> Result<(Journal, T), CheckedFailure<E>> {
    verify_checked_program(presentation, expected, nonces, guest.image_id, check)
}

fn verify_checked_program<T, E>(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    image_id: [u32; 8],
    check: impl FnOnce(&Journal) -> Result<T, E>,
) -> Result<(Journal, T), CheckedFailure<E>> {
    let journal =
        checked_journal(presentation, expected, image_id).map_err(CheckedFailure::Verification)?;
    let checked = check(&journal).map_err(CheckedFailure::Check)?;
    if !nonces
        .consume(expected.nonce)
        .map_err(CheckedFailure::Verification)?
    {
        return Err(CheckedFailure::Verification(Error(
            "challenge already consumed",
        )));
    }
    Ok((journal, checked))
}

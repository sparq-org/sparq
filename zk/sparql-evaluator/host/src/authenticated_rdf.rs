// [OPUS-5.5] zkp-14.5: low-level V5 host API over the separately pinned guest.
// Rust guideline compliant 2026-02-21
//! Issuer-authenticated RDF (V5) receipts bound to independent verifier requests.
//!
//! Compiled only with the off-by-default `authenticated-rdf` feature. The relation
//! is [`sparq_proved_evaluator_model::authenticated_rdf`], unchanged. It runs in
//! its own guest image (see [`crate::embedded_authrdf_pin`]), separate from the
//! exact V1–V3 guest. Neither guest accepts the other's input; direct execution
//! at source `42d13fed` observed both rejections. Both functions
//! take an [`AcceptedGuest`] loaded from an independently approved V5 artifact
//! pin. There is no embedded-default shortcut.
//!
//! The guest itself parses, canonicalizes and authenticates every credential.
//! It checks the verifier's table and any agreed commitment, then evaluates the
//! query. The host supplies no precomputed dataset, journal or verdict. Proving
//! first runs native request validation and credential authentication only so
//! that invalid input fails early; nothing relies on that check.
//!
//! Both `DatasetAuthority` variants are supported. [`verify_with_artifact`]
//! accepts a journal only after Succinct receipt verification with dev mode
//! disabled, journal decoding and `bind_journal` against the verifier's own
//! request. Only then does it consume the nonce, once. No earlier failure consumes
//! it.
//!
//! Proving shares the exact APIs' prover-local session ceiling. The valid
//! synthetic V5 witnesses of the direct execution tests fit it; that does not
//! show every valid witness fits, and exceeding it yields no presentation.
//!
//! The one low-level genuine-receipt job, at `42d13fed`, timed out without a
//! receipt. Separately, the generic VCQ adapter (`vcq_authenticated`) at frozen
//! source `7fe88955` produced one verified genuine V5 receipt with the same
//! approved guest `c35f5e4b`. It covers only one public synthetic
//! verifier-agreed SELECT bag case. It shows neither that every valid witness
//! fits the ceiling nor that the other five declared tuples have retained
//! proofs.
//!
//! Experimental, not externally audited. Neither provenance establishes
//! credential status, holder binding, or wallet or world completeness.

use crate::v3::CheckedFailure;
use crate::{AcceptedGuest, Error, Nonces, Presentation, prove_serialized, verify_receipt};
use sparq_proved_evaluator_model::authenticated_rdf::{
    Journal, Request, Witness, bind_journal, dataset_commitment, validate_request,
};
use std::{convert::Infallible, path::Path};

/// Proves V5 authentication and evaluation with an independently accepted guest.
///
/// `guest` must be the approved V5 image; the exact V1–V3 image rejects V5
/// input. The returned receipt has already been verified and bound to
/// `witness.request`, without consuming any nonce.
///
/// # Errors
/// Rejects invalid requests or credentials, failed execution or proving
/// (including executions above the shared session ceiling), non-Succinct
/// receipts and receipts that do not bind to `witness.request`.
pub fn prove_with_artifact(
    witness: &Witness,
    r0vm: &Path,
    guest: &AcceptedGuest,
) -> Result<Presentation, Error> {
    validate_request(&witness.request).map_err(|_| Error("invalid V5 proof request"))?;
    dataset_commitment(&witness.dataset, &witness.request.policy)
        .map_err(|_| Error("V5 private credentials rejected"))?;
    let presentation = prove_serialized(witness, r0vm, &guest.artifact)?;
    checked_journal(&presentation, &witness.request, guest.image_id)?;
    Ok(presentation)
}

fn checked_journal(
    presentation: &Presentation,
    expected: &Request,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    validate_request(expected).map_err(|_| Error("invalid V5 expected request"))?;
    verify_receipt(presentation, image_id)?;
    let journal: Journal = presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("V5 journal decoding rejected"))?;
    bind_journal(&journal, expected)
        .map_err(|_| Error("independent V5 request binding rejected"))?;
    Ok(journal)
}

/// Verifies a V5 receipt, the independent request and a single-use nonce.
///
/// Obtain `expected`, including its authorization table and any agreed
/// commitment, from verifier-owned configuration, never from the presentation.
/// Preserve the returned provenance: holder-selected credentials are
/// authenticated but may omit others.
///
/// # Errors
/// Rejects fake or non-Succinct receipts, another image, journal or request
/// substitution, authority or anchor mismatch, replay, and nonce-storage failure.
/// Only replay and storage failure are reported after the nonce store is called.
pub fn verify_with_artifact(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
) -> Result<Journal, Error> {
    let no_check = |_: &Journal| Ok::<(), Infallible>(());
    match verify_checked(presentation, expected, nonces, guest.image_id, no_check) {
        Ok((journal, ())) => Ok(journal),
        Err(CheckedFailure::Verification(error)) => Err(error),
        Err(CheckedFailure::Check(never)) => match never {},
    }
}

// [OPUS-5.5] zkp-14.6: crate-private hook used by `crate::vcq_authenticated`.
// `check` sees only a verified, request-bound journal and runs before the nonce
// is consumed; a rejected check never reaches `nonces`. Behavior is unchanged.
pub(crate) fn verify_checked<T, E>(
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

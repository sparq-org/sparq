// zkp-14.5: low-level V5 host API over the separately pinned guest.
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
//! [`check_query_profile`] rejects `DESCRIBE` and dataset clauses (`FROM`,
//! `FROM NAMED`), which version 1 of the ZK SPARQL answers specification
//! excludes but the relation evaluates. Proving and both verification paths
//! apply it to the request before any other work; the guest does not.
//!
//! Experimental, not externally audited. Neither provenance establishes
//! credential status, holder binding, or wallet or world completeness.

use crate::v3::CheckedFailure;
use crate::{AcceptedGuest, Error, Nonces, Presentation, prove_serialized, verify_receipt};
use serde::{Deserialize, Serialize};
use sparq_proved_evaluator_model::authenticated_rdf::{
    Journal, Request, SignatureMode, Witness, bind_journal, check_revealed_signatures,
    dataset_commitment, validate_request,
};
use std::{
    convert::Infallible,
    path::Path,
    time::{Duration, Instant},
};

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
    check_query_profile(&witness.request)?;
    validate_request(&witness.request).map_err(|_| Error("invalid V5 proof request"))?;
    dataset_commitment(&witness.dataset, &witness.request.policy)
        .map_err(|_| Error("V5 private credentials rejected"))?;
    let presentation = prove_serialized(witness, r0vm, &guest.artifact)?;
    checked_journal(&presentation, &witness.request, guest.image_id)?;
    Ok(presentation)
}

/// Rejects a request whose query uses `DESCRIBE` or a dataset clause (`FROM` or
/// `FROM NAMED`).
///
/// Version 1 of the ZK SPARQL answers specification excludes these features, but
/// the V5 relation evaluates them: `DESCRIBE` with the evaluator's own
/// description, and dataset clauses against credentials that form only the
/// default graph. The check runs on the host, on the verifier's own request, so
/// the accepted V5 image is unchanged.
///
/// # Errors
/// Rejects an unparsable query, `DESCRIBE` and any dataset clause.
pub fn check_query_profile(request: &Request) -> Result<(), Error> {
    let query = spargebra::SparqlParser::new()
        .parse_query(&request.query)
        .map_err(|_| Error("V5 query parse rejected"))?;
    if matches!(query, spargebra::Query::Describe { .. }) {
        return Err(Error("DESCRIBE is outside the V5 query profile"));
    }
    if query.dataset().is_some() {
        return Err(Error(
            "FROM and FROM NAMED are outside the V5 query profile",
        ));
    }
    Ok(())
}

fn checked_journal(
    presentation: &Presentation,
    expected: &Request,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    validate_expected(expected)?;
    let journal = receipt_journal(presentation, image_id)?;
    bind_expected(&journal, expected)?;
    Ok(journal)
}

// These are the production stages. Request work surrounds the receipt and
// signature stages to preserve error precedence and nonce-store behavior.
fn validate_expected(expected: &Request) -> Result<(), Error> {
    check_query_profile(expected)?;
    validate_request(expected).map_err(|_| Error("invalid V5 expected request"))
}

fn receipt_journal(presentation: &Presentation, image_id: [u32; 8]) -> Result<Journal, Error> {
    verify_receipt(presentation, image_id)?;
    presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("V5 journal decoding rejected"))
}

fn bind_expected(journal: &Journal, expected: &Request) -> Result<(), Error> {
    bind_journal(journal, expected).map_err(|_| Error("independent V5 request binding rejected"))
}

fn consume_nonce(expected: &Request, nonces: &mut impl Nonces) -> Result<(), Error> {
    if !nonces.consume(expected.nonce)? {
        return Err(Error("challenge already consumed"));
    }
    Ok(())
}

/// JSON transport envelope for the low-level V5 API, including detached signatures.
///
/// `signatures` is `None` (JSON null) for the hidden entry point and `Some`
/// (journal order) for the revealed entry point. The verifier's independent
/// request supplies the suite, mode and policy; the receipt carries the journal.
/// This is a local encoding, not the VCQ adapter or a standardized wire format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerPresentation {
    /// The complete low-level presentation, including receipt metadata and journal.
    pub presentation: Presentation,
    /// Detached Ed25519 signatures for the revealed entry point, otherwise absent.
    pub signatures: Option<Vec<Vec<u8>>>,
}

/// Disjoint work intervals from one successful low-level V5 verification.
///
/// Request time sums the mode guard, query/profile validation, request validation,
/// journal binding and nonce consumption, in their original order. Receipt time
/// includes Succinct verification and journal decoding. Signature time covers the
/// strict revealed-mode checks and is absent in hidden mode. Timers do not nest;
/// caller/envelope decoding, timer bookkeeping and return construction are outside.
#[derive(Clone, Debug, Default)]
pub struct VerificationTimings {
    /// Receipt verification and journal decoding.
    pub receipt: Duration,
    /// Revealed-mode signature checks, or `None` in hidden mode.
    pub signatures: Option<Duration>,
    /// All request checks, including nonce consumption.
    pub request: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Receipt,
    Signatures,
    Request,
}

trait Stages {
    fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T;
}

impl Stages for () {
    fn run<T>(&mut self, _: Stage, work: impl FnOnce() -> T) -> T {
        work()
    }
}

impl Stages for VerificationTimings {
    fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let result = work();
        let elapsed = start.elapsed();
        match stage {
            Stage::Receipt => self.receipt += elapsed,
            Stage::Signatures => *self.signatures.get_or_insert(Duration::ZERO) += elapsed,
            Stage::Request => self.request += elapsed,
        }
        result
    }
}

/// Verifies the complete local envelope through the existing low-level entry points.
///
/// # Errors
/// Exactly the errors of the selected hidden or revealed entry point. Selection
/// uses the presence of detached signatures and still checks the independent mode.
pub fn verify_answer_with_artifact(
    answer: &AnswerPresentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
) -> Result<Journal, Error> {
    verify_answer(answer, expected, nonces, guest, &mut ())
}

/// As [`verify_answer_with_artifact`], recording the same production stages once.
///
/// # Errors
/// Exactly as [`verify_answer_with_artifact`]; failed verifications return no sample.
pub fn verify_answer_with_artifact_timed(
    answer: &AnswerPresentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
) -> Result<(Journal, VerificationTimings), Error> {
    let mut timings = VerificationTimings::default();
    let journal = verify_answer(answer, expected, nonces, guest, &mut timings)?;
    Ok((journal, timings))
}

fn verify_answer(
    answer: &AnswerPresentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
    stages: &mut impl Stages,
) -> Result<Journal, Error> {
    match &answer.signatures {
        None => verify_hidden(&answer.presentation, expected, nonces, guest, stages),
        Some(signatures) => verify_revealed(
            &answer.presentation,
            expected,
            signatures,
            nonces,
            guest,
            stages,
        ),
    }
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
    verify_hidden(presentation, expected, nonces, guest, &mut ())
}

fn verify_hidden(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
    stages: &mut impl Stages,
) -> Result<Journal, Error> {
    // A revealed-mode journal is unauthenticated until its signatures are checked.
    stages.run(Stage::Request, || {
        if expected.policy.signature_mode == SignatureMode::Revealed {
            return Err(Error(
                "revealed-mode V5 requests use verify_revealed_with_artifact",
            ));
        }
        Ok(())
    })?;
    let no_check = |_: &Journal| Ok::<(), Infallible>(());
    match verify_staged(
        presentation,
        expected,
        nonces,
        guest.image_id,
        no_check,
        stages,
        receipt_journal,
    ) {
        Ok((journal, ())) => Ok(journal),
        Err(CheckedFailure::Verification(error)) => Err(error),
        Err(CheckedFailure::Check(never)) => match never {},
    }
}

/// Verifies a revealed-mode V5 receipt and the presented issuer signatures.
///
/// As [`verify_with_artifact`], and before the nonce is consumed, checks each
/// presented `proofValue` (journal order, 64 bytes each) with strict Ed25519
/// against the journal's signed message and the request table key it names.
///
/// # Errors
/// Everything [`verify_with_artifact`] rejects, hidden-mode requests, and any
/// missing, extra, malformed or invalid signature.
pub fn verify_revealed_with_artifact(
    presentation: &Presentation,
    expected: &Request,
    signatures: &[Vec<u8>],
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
) -> Result<Journal, Error> {
    verify_revealed(presentation, expected, signatures, nonces, guest, &mut ())
}

fn verify_revealed(
    presentation: &Presentation,
    expected: &Request,
    signatures: &[Vec<u8>],
    nonces: &mut impl Nonces,
    guest: &AcceptedGuest,
    stages: &mut impl Stages,
) -> Result<Journal, Error> {
    stages.run(Stage::Request, || {
        if expected.policy.signature_mode != SignatureMode::Revealed {
            return Err(Error("hidden-mode V5 requests use verify_with_artifact"));
        }
        Ok(())
    })?;
    let check = |journal: &Journal| check_revealed_signatures(journal, expected, signatures);
    match verify_staged(
        presentation,
        expected,
        nonces,
        guest.image_id,
        check,
        stages,
        receipt_journal,
    ) {
        Ok((journal, ())) => Ok(journal),
        Err(CheckedFailure::Verification(error)) => Err(error),
        Err(CheckedFailure::Check(_)) => Err(Error("revealed issuer signature rejected")),
    }
}

// zkp-14.6: crate-private hook used by `crate::vcq_authenticated`.
// `check` sees only a verified, request-bound journal and runs before the nonce
// is consumed; a rejected check never reaches `nonces`. Behavior is unchanged.
#[cfg(feature = "vcq-authenticated")]
pub(crate) fn verify_checked<T, E>(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    image_id: [u32; 8],
    check: impl FnOnce(&Journal) -> Result<T, E>,
) -> Result<(Journal, T), CheckedFailure<E>> {
    verify_staged(
        presentation,
        expected,
        nonces,
        image_id,
        check,
        &mut (),
        receipt_journal,
    )
}

// Receipt injection is private and monomorphized. Tests substitute only this
// stage to exercise post-receipt behavior without producing a cryptographic proof.
fn verify_staged<T, E>(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    image_id: [u32; 8],
    check: impl FnOnce(&Journal) -> Result<T, E>,
    stages: &mut impl Stages,
    receipt: impl FnOnce(&Presentation, [u32; 8]) -> Result<Journal, Error>,
) -> Result<(Journal, T), CheckedFailure<E>> {
    stages
        .run(Stage::Request, || validate_expected(expected))
        .map_err(CheckedFailure::Verification)?;
    let journal = stages
        .run(Stage::Receipt, || receipt(presentation, image_id))
        .map_err(CheckedFailure::Verification)?;
    stages
        .run(Stage::Request, || bind_expected(&journal, expected))
        .map_err(CheckedFailure::Verification)?;
    let checked = if expected.policy.signature_mode == SignatureMode::Revealed {
        stages.run(Stage::Signatures, || check(&journal))
    } else {
        check(&journal)
    }
    .map_err(CheckedFailure::Check)?;
    stages
        .run(Stage::Request, || consume_nonce(expected, nonces))
        .map_err(CheckedFailure::Verification)?;
    Ok((journal, checked))
}

#[cfg(test)]
#[path = "authenticated_rdf_tests.rs"]
mod tests;

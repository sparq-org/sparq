// [GPT-6] Genuine local RISC Zero receipts only; experimental, not externally audited.
//! Proves exact bounded Sparq evaluation and verifies independent request binding.
//!
//! This opt-in detached crate is not part of the lean engine or WASM dependency
//! graph. It uses an explicitly supplied local r0vm executable, never a cloud
//! prover. The executable sees the private witness, as does its local caller.

use risc0_zkvm::{
    ExecutorEnv, ExternalProver, InnerReceipt, Prover, ProverOpts, Receipt, VerifierContext,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_methods::{SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID};
use sparq_proved_evaluator_model::{
    Journal, Request, Witness, bind_journal, dataset_commitment, validate_request,
};
use std::{fmt, path::Path};

/// Public presentation contains only the cryptographic receipt and its journal.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub receipt: Receipt,
}

/// Verification/proving failure; never contains private source data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

/// Atomic persistent nonce acceptance supplied by the relying party.
pub trait Nonces {
    /// Consumes an unused challenge, returning false for an already consumed one.
    ///
    /// # Errors
    /// Return an error on storage failure; verification fails closed.
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error>;
}

/// Returns the identity of this binary's locally compiled exact-evaluator guest.
pub fn method_id() -> [u32; 8] {
    SPARQ_EXACT_GUEST_ID
}

/// The exact guest artifact embedded in this build, suitable for deployment export.
pub fn embedded_artifact() -> &'static [u8] {
    SPARQ_EXACT_GUEST_ELF
}

/// Trusted deployment configuration, obtained independently of a presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPin {
    /// Hash of all artifact bytes, checked before parsing ELF memory declarations.
    pub sha256: [u8; 32],
    /// Expected RISC Zero execution identity of that approved artifact.
    pub image_id: [u32; 8],
}

/// Exports a pin for this locally compiled artifact; release operators must approve it.
pub fn embedded_pin() -> ArtifactPin {
    ArtifactPin {
        sha256: Sha256::digest(SPARQ_EXACT_GUEST_ELF).into(),
        image_id: SPARQ_EXACT_GUEST_ID,
    }
}

/// An executable guest checked against a relying party's independently approved ID.
///
/// This is not deserializable from a presentation. Acceptance of the expected ID
/// is an application trust decision about the program's semantics; matching a
/// holder-provided ID alone establishes no such trust.
pub struct AcceptedGuest {
    artifact: Vec<u8>,
    image_id: [u32; 8],
}

impl fmt::Debug for AcceptedGuest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcceptedGuest")
            .field("image_id", &self.image_id)
            .finish_non_exhaustive()
    }
}

impl AcceptedGuest {
    /// Checks supplied program bytes against an independently pinned digest and ID.
    ///
    /// Obtain `expected` from trusted deployment configuration, never from
    /// the proof sender. Prover and verifier can load the same approved artifact
    /// even when their own local source builds produce different image IDs.
    ///
    /// # Errors
    /// Rejects oversized/malformed programs and a mismatching expected identity.
    pub fn from_artifact(artifact: Vec<u8>, expected: &ArtifactPin) -> Result<Self, Error> {
        if artifact.len() > 32 * 1024 * 1024 {
            return Err(Error("guest artifact capacity rejected"));
        }
        // Reject unapproved bytes before SDK ELF parsing: small hostile ELF files
        // can otherwise request enormous BSS allocations while computing an ID.
        let digest: [u8; 32] = Sha256::digest(&artifact).into();
        if digest != expected.sha256 {
            return Err(Error("independent guest artifact digest rejected"));
        }
        let image =
            risc0_zkvm::compute_image_id(&artifact).map_err(|_| Error("invalid guest artifact"))?;
        if image.as_words() != expected.image_id {
            return Err(Error("independent guest artifact identity rejected"));
        }
        Ok(Self {
            artifact,
            image_id: expected.image_id,
        })
    }

    /// The independently accepted execution identity, not an identity from a receipt.
    pub fn image_id(&self) -> [u32; 8] {
        self.image_id
    }
}

/// Proves the entire relation with an explicitly supplied local executable.
///
/// # Errors
/// Rejects bad requests, missing/incompatible tools, failed relations, mock
/// receipts, and executions above the prover-local cycle ceiling. No host result
/// or host execution verdict is accepted as a substitute.
pub fn prove(witness: &Witness, r0vm: &Path) -> Result<Presentation, Error> {
    prove_program(witness, r0vm, SPARQ_EXACT_GUEST_ELF, SPARQ_EXACT_GUEST_ID)
}

/// Proves with an independently accepted deployment artifact.
///
/// # Errors
/// Has the same fail-closed input, execution and receipt checks as [`prove`].
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
    validate_request(&witness.request).map_err(|_| Error("invalid proof request"))?;
    dataset_commitment(&witness.dataset, &witness.request.policy)
        .map_err(|_| Error("private input capacity rejected"))?;
    let env = ExecutorEnv::builder()
        // Prover-local denial-of-service guard; exhaustion produces no presentation.
        .session_limit(Some(1 << 25))
        .segment_limit_po2(20)
        .write(witness)
        .map_err(|_| Error("private witness serialization failed"))?
        .build()
        .map_err(|_| Error("executor environment failed"))?;
    let prover = ExternalProver::new("sparq-local-r0vm", r0vm);
    let context = VerifierContext::default().with_dev_mode(false);
    let info = prover
        .prove_with_ctx(
            env,
            &context,
            artifact,
            &ProverOpts::succinct().with_dev_mode(false),
        )
        .map_err(|_| Error("real local proof failed"))?;
    let presentation = Presentation {
        receipt: info.receipt,
    };
    checked_journal(&presentation, &witness.request, image_id)?;
    Ok(presentation)
}

fn checked_journal(
    presentation: &Presentation,
    expected: &Request,
    image_id: [u32; 8],
) -> Result<Journal, Error> {
    validate_request(expected).map_err(|_| Error("invalid expected request"))?;
    if !matches!(presentation.receipt.inner, InnerReceipt::Succinct { .. }) {
        return Err(Error("only succinct receipts are accepted"));
    }
    presentation
        .receipt
        .verify_with_context(&VerifierContext::default().with_dev_mode(false), image_id)
        .map_err(|_| Error("proof or program identity rejected"))?;
    let journal: Journal = presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("journal decoding rejected"))?;
    bind_journal(&journal, expected).map_err(|_| Error("independent request binding rejected"))?;
    Ok(journal)
}

/// Verifies the real receipt, expected request, and atomic single-use challenge.
///
/// Obtain `expected` independently from the relying party's own request. Never
/// reconstruct it from the prover. Returned provenance must be preserved by users
/// of this API; holder-declared computation is not authenticated credential data.
///
/// # Errors
/// Rejects tampering, mismatched program/request/dataset/policy, replay, and storage failure.
pub fn verify(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
) -> Result<Journal, Error> {
    verify_program(presentation, expected, nonces, SPARQ_EXACT_GUEST_ID)
}

/// Verifies against the same independently accepted artifact deployed by the prover.
///
/// This supports artifact-pinned deployment across non-identical local builds.
/// The application must independently approve the program ID before constructing
/// `guest`; never construct it from an ID supplied in the presentation.
///
/// # Errors
/// Rejects bad receipts, request mismatch, replay and nonce storage failure.
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

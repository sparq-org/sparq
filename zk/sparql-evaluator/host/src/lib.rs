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

/// Proves the entire relation with an explicitly supplied local executable.
///
/// # Errors
/// Rejects bad requests, missing/incompatible tools, failed relations, mock
/// receipts, and executions above the prover-local cycle ceiling. No host result
/// or host execution verdict is accepted as a substitute.
pub fn prove(witness: &Witness, r0vm: &Path) -> Result<Presentation, Error> {
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
            SPARQ_EXACT_GUEST_ELF,
            &ProverOpts::succinct().with_dev_mode(false),
        )
        .map_err(|_| Error("real local proof failed"))?;
    let presentation = Presentation {
        receipt: info.receipt,
    };
    checked_journal(&presentation, &witness.request)?;
    Ok(presentation)
}

fn checked_journal(presentation: &Presentation, expected: &Request) -> Result<Journal, Error> {
    validate_request(expected).map_err(|_| Error("invalid expected request"))?;
    if !matches!(presentation.receipt.inner, InnerReceipt::Succinct { .. }) {
        return Err(Error("only succinct receipts are accepted"));
    }
    presentation
        .receipt
        .verify_with_context(
            &VerifierContext::default().with_dev_mode(false),
            SPARQ_EXACT_GUEST_ID,
        )
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
    let journal = checked_journal(presentation, expected)?;
    if !nonces.consume(expected.nonce)? {
        return Err(Error("challenge already consumed"));
    }
    Ok(journal)
}

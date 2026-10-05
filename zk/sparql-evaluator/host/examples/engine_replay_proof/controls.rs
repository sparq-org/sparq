// [OPUS-5.5] Verifier-side substitution and tamper controls; not malicious-witness checks.
//! Verification-only rejection controls shared by the example and its ignored test.
//!
//! The prover never receives a result: V3 evaluates inside the guest. Editing the
//! public journal after proving therefore checks receipt integrity, not a
//! malicious witness. Malicious-witness checks change the prover's input instead.

use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use sparq_proved_evaluator::v3::verify_with_artifact;
use sparq_proved_evaluator::{AcceptedGuest, Error, Nonces, Presentation};
use sparq_proved_evaluator_model::{DatasetAuthority, MAX_QUERY_BYTES, v3};
use std::collections::BTreeSet;

/// Expected host error for a verifier expectation that the journal does not bind.
pub const BINDING_REJECTED: &str = "independent V3 request binding rejected";
/// Expected host error for altered receipt or journal bytes.
pub const RECEIPT_REJECTED: &str = "proof or program identity rejected";

/// In-process single-use challenges; not persistent replay protection.
#[derive(Default)]
pub struct MemoryNonces(BTreeSet<[u8; 32]>);

impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

/// Requires the exact host error value; success or another error fails.
pub fn require_rejection<T>(actual: Result<T, Error>, expected: &'static str) -> Result<(), String> {
    match actual {
        Err(error) if error == Error(expected) => Ok(()),
        Err(error) => Err(format!(
            "wrong rejection class: expected {expected}; received {error}"
        )),
        Ok(_) => Err(format!("control unexpectedly accepted: {expected}")),
    }
}

/// Returns a result of the same form that always differs from `result`.
pub fn tampered_result(result: &v3::CanonicalResult) -> v3::CanonicalResult {
    let mut changed = result.clone();
    match &mut changed {
        v3::CanonicalResult::Ask(answer) => *answer = !*answer,
        v3::CanonicalResult::Select {
            variables, rows, ..
        } => {
            let extra = rows
                .first()
                .cloned()
                .unwrap_or_else(|| vec![None; variables.len()]);
            rows.push(extra);
        }
        v3::CanonicalResult::Graph { ntriples } => {
            ntriples.push_str("<urn:sparq:tampered> <urn:sparq:tampered> <urn:sparq:tampered> .\n");
        }
    }
    changed
}

fn with_journal(presentation: &Presentation, journal: &v3::Journal) -> Result<Presentation, String> {
    let words = risc0_zkvm::serde::to_vec(journal).map_err(|_| "journal re-encoding failed")?;
    let mut changed = presentation.clone();
    changed.receipt.journal.bytes = words.into_iter().flat_map(u32::to_le_bytes).collect();
    Ok(changed)
}

fn fresh(presentation: &Presentation, request: &v3::Request, guest: &AcceptedGuest) -> Result<v3::Journal, Error> {
    verify_with_artifact(presentation, request, &mut MemoryNonces::default(), guest)
}

/// Runs verification-only rejection controls against one genuine presentation.
///
/// `consumed` must be the store that already accepted `presentation`. Returns
/// the passed control names; each control requires its exact host error.
///
/// # Errors
/// Returns a description when any control is accepted or rejected differently.
pub fn verifier_controls(
    presentation: &Presentation,
    request: &v3::Request,
    journal: &v3::Journal,
    guest: &AcceptedGuest,
    consumed: &mut MemoryNonces,
) -> Result<Vec<&'static str>, String> {
    let mut passed = Vec::new();
    let mut substitutions = Vec::new();
    let mut nonce = request.clone();
    nonce.nonce[0] ^= 1;
    if nonce.nonce == [0; 32] {
        nonce.nonce[1] ^= 1;
    }
    substitutions.push(("expected_nonce_substitution", nonce));
    let mut query = request.clone();
    if query.query.len() < MAX_QUERY_BYTES {
        query.query.push(' ');
    } else {
        query.query.pop();
    }
    substitutions.push(("expected_query_substitution", query));
    let mut anchor = request.clone();
    let mut commitment = journal.dataset_commitment;
    commitment[0] ^= 1;
    anchor.authority = DatasetAuthority::VerifierAgreed { commitment };
    substitutions.push(("expected_commitment_substitution", anchor));
    let mut authority = request.clone();
    authority.authority = match request.authority {
        DatasetAuthority::VerifierAgreed { .. } => DatasetAuthority::HolderDeclared,
        DatasetAuthority::HolderDeclared => DatasetAuthority::VerifierAgreed {
            commitment: journal.dataset_commitment,
        },
    };
    substitutions.push(("expected_authority_substitution", authority));
    for (name, wrong) in substitutions {
        require_rejection(fresh(presentation, &wrong, guest), BINDING_REJECTED)?;
        passed.push(name);
    }
    require_rejection(
        verify_with_artifact(presentation, request, consumed, guest),
        "challenge already consumed",
    )?;
    passed.push("consumed_nonce_replay");

    // Public-output integrity: the journal is edited after proving.
    let mut result = journal.clone();
    result.result = tampered_result(&journal.result);
    require_rejection(
        fresh(&with_journal(presentation, &result)?, request, guest),
        RECEIPT_REJECTED,
    )?;
    passed.push("public_journal_result_tamper");
    let mut root = journal.clone();
    root.dataset_commitment[0] ^= 1;
    require_rejection(
        fresh(&with_journal(presentation, &root)?, request, guest),
        RECEIPT_REJECTED,
    )?;
    passed.push("public_journal_commitment_tamper");

    let mut seal = presentation.clone();
    let InnerReceipt::Succinct(receipt) = &mut seal.receipt.inner else {
        return Err("control requires a Succinct presentation".into());
    };
    *receipt.seal.first_mut().ok_or("empty Succinct seal")? ^= 1;
    require_rejection(fresh(&seal, request, guest), RECEIPT_REJECTED)?;
    passed.push("succinct_seal_tamper");
    let bytes = presentation.receipt.journal.bytes.clone();
    let fake = Presentation {
        receipt: Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
                guest.image_id(),
                bytes.clone(),
            ))),
            bytes,
        ),
    };
    require_rejection(
        fresh(&fake, request, guest),
        "only succinct receipts are accepted",
    )?;
    passed.push("fake_receipt");
    Ok(passed)
}

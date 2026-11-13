// Fixture tests of the stage seam do not certify a cryptographic receipt.
#[path = "../tests/support/authenticated_rdf.rs"]
mod fixture;
use super::*;
use fixture::CountingNonces;
use std::cell::Cell;

#[derive(Default)]
struct Trace {
    stages: Vec<Stage>,
    active: bool,
}
impl Stages for Trace {
    fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T {
        assert!(!self.active, "verification intervals must not nest");
        self.active = true;
        self.stages.push(stage);
        let result = work();
        self.active = false;
        result
    }
}

fn bytes(journal: &Journal) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(journal)
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

// Frozen pre-refactor control flow. Only the receipt verifier is substituted in
// fixture cases; real rejection cases below pass the production receipt stage.
fn legacy<T, E>(
    presentation: &Presentation,
    expected: &Request,
    nonces: &mut impl Nonces,
    check: impl FnOnce(&Journal) -> Result<T, E>,
    receipt: impl FnOnce(&Presentation, [u32; 8]) -> Result<Journal, Error>,
) -> Result<(Journal, T), CheckedFailure<E>> {
    check_query_profile(expected).map_err(CheckedFailure::Verification)?;
    validate_request(expected)
        .map_err(|_| CheckedFailure::Verification(Error("invalid V5 expected request")))?;
    let journal = receipt(presentation, [0; 8]).map_err(CheckedFailure::Verification)?;
    bind_journal(&journal, expected).map_err(|_| {
        CheckedFailure::Verification(Error("independent V5 request binding rejected"))
    })?;
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

fn comparable<T>(
    result: Result<(Journal, T), CheckedFailure<Rejected>>,
) -> Result<Vec<u8>, String> {
    match result {
        Ok((journal, _)) => Ok(bytes(&journal)),
        Err(CheckedFailure::Verification(error)) => Err(format!("verification:{}", error.0)),
        Err(CheckedFailure::Check(error)) => Err(format!("check:{}", error.0)),
    }
}

use sparq_proved_evaluator_model::{Rejected, authenticated_rdf as auth};

#[test]
fn staged_verifier_matches_legacy_decisions_journal_bytes_and_nonce_effects() {
    for mode in [SignatureMode::Hidden, SignatureMode::Revealed] {
        let mut witness = fixture::witness(
            fixture::ASK_ISSUER,
            sparq_proved_evaluator_model::DatasetAuthority::HolderDeclared,
            [0x3c; 32],
        );
        witness.request.policy.signature_mode = mode;
        let signature = witness.dataset.credentials[0].signature.clone();
        if mode == SignatureMode::Revealed {
            witness.dataset.credentials[0].signature.clear();
        }
        let journal = auth::evaluate(&witness).unwrap();
        let fake = fixture::fake_receipt([0; 8], bytes(&journal));
        for case in [
            "valid",
            "wrong_nonce",
            "wrong_request",
            "invalid_request",
            "profile",
            "bad_signature",
            "missing_signature",
            "extra_signature",
            "replay",
            "store",
            "receipt",
            "decode",
        ] {
            let mut expected = witness.request.clone();
            let mut signatures = vec![signature.clone()];
            match case {
                "wrong_nonce" => expected.nonce[0] ^= 1,
                "wrong_request" => expected.query = fixture::ASK_OTHER_ISSUER.into(),
                "invalid_request" => expected.version = 0,
                "profile" => expected.query = "DESCRIBE ?s WHERE { ?s ?p ?o }".into(),
                "bad_signature" => signatures[0][0] ^= 1,
                "missing_signature" => signatures.clear(),
                "extra_signature" => signatures.push(signature.clone()),
                _ => {}
            }
            let setup = || {
                let mut nonces = if case == "store" {
                    CountingNonces::broken()
                } else {
                    CountingNonces::default()
                };
                if case == "replay" {
                    nonces.consume(expected.nonce).unwrap();
                }
                nonces
            };
            let mut old_nonces = setup();
            let mut new_nonces = setup();
            let receipt_calls = Cell::new(0);
            let receipt = |p: &Presentation, image| {
                receipt_calls.set(receipt_calls.get() + 1);
                if case == "receipt" {
                    return receipt_journal(p, image);
                }
                if case == "decode" {
                    return Err(Error("V5 journal decoding rejected"));
                }
                // Explicit test-only seam, never available through a public API.
                Ok(journal.clone())
            };
            let check_calls = Cell::new(0);
            let check = |j: &Journal| -> Result<(), Rejected> {
                check_calls.set(check_calls.get() + 1);
                if mode == SignatureMode::Revealed {
                    check_revealed_signatures(j, &expected, &signatures)
                } else {
                    Ok(())
                }
            };
            let old = comparable(legacy(&fake, &expected, &mut old_nonces, check, receipt));
            let mut trace = Trace::default();
            let new = comparable(verify_staged(
                &fake,
                &expected,
                &mut new_nonces,
                [0; 8],
                check,
                &mut trace,
                receipt,
            ));
            assert_eq!(new, old, "{mode:?}/{case}");
            assert_eq!(new_nonces.calls, old_nonces.calls, "{mode:?}/{case}");
            // Each path attempts the receipt/check at most once, and has the
            // same early exits. The successful fixture is one verified journal.
            assert_eq!(receipt_calls.get() % 2, 0);
            assert_eq!(check_calls.get() % 2, 0);
            assert_eq!(
                trace
                    .stages
                    .iter()
                    .filter(|&&s| s == Stage::Receipt)
                    .count(),
                receipt_calls.get() / 2
            );
            assert_eq!(
                trace
                    .stages
                    .iter()
                    .filter(|&&s| s == Stage::Signatures)
                    .count(),
                if mode == SignatureMode::Revealed {
                    check_calls.get() / 2
                } else {
                    0
                }
            );
            if case == "valid" {
                assert_eq!(new.unwrap(), bytes(&journal));
                assert_eq!(new_nonces.calls, 1);
                let mut expected_stages = vec![Stage::Request, Stage::Receipt, Stage::Request];
                if mode == SignatureMode::Revealed {
                    expected_stages.push(Stage::Signatures);
                }
                expected_stages.push(Stage::Request);
                assert_eq!(trace.stages, expected_stages);
                assert_eq!(receipt_calls.get(), 2);
                assert_eq!(check_calls.get(), 2);
            }
        }
    }
}

#[test]
fn production_envelope_roundtrips_and_matches_legacy_receipt_rejections() {
    let guest = AcceptedGuest::from_artifact(
        crate::embedded_authrdf_artifact().to_vec(),
        &crate::embedded_authrdf_pin(),
    )
    .unwrap();
    for mode in [SignatureMode::Hidden, SignatureMode::Revealed] {
        let mut witness = fixture::witness(
            fixture::ASK_ISSUER,
            sparq_proved_evaluator_model::DatasetAuthority::HolderDeclared,
            [0x3c; 32],
        );
        witness.request.policy.signature_mode = mode;
        let signature = witness.dataset.credentials[0].signature.clone();
        if mode == SignatureMode::Revealed {
            witness.dataset.credentials[0].signature.clear();
        }
        let journal = auth::evaluate(&witness).unwrap();
        let answer = AnswerPresentation {
            presentation: fixture::fake_receipt(guest.image_id(), bytes(&journal)),
            signatures: (mode == SignatureMode::Revealed).then(|| vec![signature]),
        };
        let encoded = serde_json::to_vec(&answer).unwrap();
        let decoded: AnswerPresentation = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);
        let object: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            object["presentation"],
            serde_json::to_value(&answer.presentation).unwrap()
        );
        assert_eq!(
            object["signatures"],
            serde_json::to_value(&answer.signatures).unwrap()
        );
        for case in [
            "fake",
            "tampered_journal",
            "wrong_mode",
            "invalid_request",
            "profile",
        ] {
            let mut decoded = decoded.clone();
            let mut expected = witness.request.clone();
            match case {
                "tampered_journal" => decoded.presentation.receipt.journal.bytes[0] ^= 1,
                "wrong_mode" => {
                    expected.policy.signature_mode = if mode == SignatureMode::Hidden {
                        SignatureMode::Revealed
                    } else {
                        SignatureMode::Hidden
                    }
                }
                "invalid_request" => expected.version = 0,
                "profile" => expected.query = "DESCRIBE ?s WHERE { ?s ?p ?o }".into(),
                _ => {}
            }
            let mut old_nonces = CountingNonces::default();
            let old = match &decoded.signatures {
                None => {
                    verify_with_artifact(&decoded.presentation, &expected, &mut old_nonces, &guest)
                }
                Some(sigs) => verify_revealed_with_artifact(
                    &decoded.presentation,
                    &expected,
                    sigs,
                    &mut old_nonces,
                    &guest,
                ),
            };
            let mut new_nonces = CountingNonces::default();
            let new =
                verify_answer_with_artifact_timed(&decoded, &expected, &mut new_nonces, &guest)
                    .map(|(j, _)| j);
            assert_eq!(new, old, "{mode:?}/{case}");
            assert!(new.is_err());
            assert_eq!(new_nonces.calls, 0);
            assert_eq!(new_nonces.calls, old_nonces.calls);
        }
    }
}

#[test]
fn stage_timers_partition_one_verification_and_leave_hidden_signatures_absent() {
    for mode in [SignatureMode::Hidden, SignatureMode::Revealed] {
        let mut witness = fixture::witness(
            fixture::ASK_ISSUER,
            sparq_proved_evaluator_model::DatasetAuthority::HolderDeclared,
            [0x3c; 32],
        );
        witness.request.policy.signature_mode = mode;
        let signatures = vec![witness.dataset.credentials[0].signature.clone()];
        if mode == SignatureMode::Revealed {
            witness.dataset.credentials[0].signature.clear();
        }
        let journal = auth::evaluate(&witness).unwrap();
        let fake = fixture::fake_receipt([0; 8], bytes(&journal));
        let mut nonces = CountingNonces::default();
        let mut timings = VerificationTimings::default();
        let mut receipt_calls = 0;
        let mut signature_calls = 0;
        let start = Instant::now();
        let result = verify_staged(
            &fake,
            &witness.request,
            &mut nonces,
            [0; 8],
            |j| {
                if mode == SignatureMode::Revealed {
                    signature_calls += 1;
                    check_revealed_signatures(j, &witness.request, &signatures)?;
                }
                Ok::<(), Rejected>(())
            },
            &mut timings,
            |_, _| {
                receipt_calls += 1;
                Ok(journal.clone())
            },
        );
        let elapsed = start.elapsed();
        assert_eq!(comparable(result).unwrap(), bytes(&journal));
        assert_eq!(receipt_calls, 1);
        assert_eq!(
            signature_calls,
            usize::from(mode == SignatureMode::Revealed)
        );
        assert_eq!(
            timings.signatures.is_some(),
            mode == SignatureMode::Revealed
        );
        assert!(
            timings.receipt + timings.request + timings.signatures.unwrap_or_default() <= elapsed
        );
        assert_eq!(nonces.calls, 1);
    }
}

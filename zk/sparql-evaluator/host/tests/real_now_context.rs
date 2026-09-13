// [GPT-6] Genuine V4 receipts bind the original query and independently agreed context.
#[path = "support/evidence.rs"]
mod evidence;
#[path = "support/now_context.rs"]
mod fixtures;
use sparq_proved_evaluator::v4::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, Error, Nonces, embedded_artifact, embedded_pin};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3, v4};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Default)]
struct MemoryNonces(BTreeSet<[u8; 32]>);
impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

fn prove_case(id: &str, agreed: bool, name: &str, nonce: u8) {
    let corpus = fixtures::corpus();
    let case = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    let input = fixtures::input(case, agreed, nonce);
    let guest =
        AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin()).unwrap();
    let r0vm = PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("local r0vm required"));
    let proof = prove_with_artifact(&input, &r0vm, &guest).expect("genuine V4 Succinct receipt");
    let mut nonces = MemoryNonces::default();
    let journal = verify_with_artifact(&proof, &input.request, &mut nonces, &guest).unwrap();
    let expected: v4::CanonicalResult =
        serde_json::from_value(case["expected_result"].clone()).unwrap();
    assert_eq!(journal.result, expected);
    assert_eq!(journal.context, input.request.context);
    assert_eq!(journal, v4::evaluate(&input).unwrap());
    assert_eq!(
        verify_with_artifact(&proof, &input.request, &mut nonces, &guest).unwrap_err(),
        Error("challenge already consumed")
    );
    for field in 0..7 {
        let mut changed = input.request.clone();
        match field {
            0 => changed.context.now.datetime = "2026-09-13T00:02:03.000000001Z".into(),
            1 => changed.context.now.datetime = "2027-01-01T00:00:00Z".into(),
            2 => changed.query.push(' '),
            3 => changed.nonce[0] ^= 1,
            4 => changed.policy.dataset.max_rows -= 1,
            5 => {
                changed.authority = DatasetAuthority::VerifierAgreed {
                    commitment: [79; 32],
                }
            }
            _ => {
                changed.authority = if agreed {
                    DatasetAuthority::HolderDeclared
                } else {
                    DatasetAuthority::VerifierAgreed {
                        commitment: journal.dataset_commitment,
                    }
                }
            }
        }
        assert_eq!(
            verify_with_artifact(&proof, &changed, &mut MemoryNonces::default(), &guest)
                .unwrap_err(),
            Error("independent V4 request binding rejected"),
            "field {field}"
        );
    }
    let mut altered = proof.clone();
    altered.receipt.journal.bytes[0] ^= 1;
    assert_eq!(
        verify_with_artifact(
            &altered,
            &input.request,
            &mut MemoryNonces::default(),
            &guest
        )
        .unwrap_err(),
        Error("proof or program identity rejected")
    );
    let fake = sparq_proved_evaluator::Presentation {
        receipt: risc0_zkvm::Receipt::new(
            risc0_zkvm::InnerReceipt::Fake(risc0_zkvm::FakeReceipt::new(
                risc0_zkvm::ReceiptClaim::ok(guest.image_id(), proof.receipt.journal.bytes.clone()),
            )),
            proof.receipt.journal.bytes.clone(),
        ),
    };
    assert_eq!(
        verify_with_artifact(&fake, &input.request, &mut MemoryNonces::default(), &guest)
            .unwrap_err(),
        Error("only succinct receipts are accepted")
    );
    let old = v3::Request {
        version: v3::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
        query: input.request.query.clone(),
        authority: input.request.authority.clone(),
        policy: input.request.policy.clone(),
        nonce: input.request.nonce,
    };
    assert!(
        sparq_proved_evaluator::v3::verify_with_artifact(
            &proof,
            &old,
            &mut MemoryNonces::default(),
            &guest
        )
        .is_err(),
        "V4 must not acquire V3 semantics"
    );
    evidence::record(name, &input.request, &proof.receipt);
}

#[test]
fn real_v4_holder_repeated_now_table() {
    prove_case("repeated-nested", false, "v4-holder-now-bag", 103);
}

#[test]
fn real_v4_verifier_exact_time_construct() {
    prove_case("construct", true, "v4-verifier-now-construct", 107);
}

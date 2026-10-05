// [GPT-6] Genuine V3 graph-result receipts with independent request verification.
#[path = "support/evidence.rs"]
mod evidence;
#[path = "support/graph_results.rs"]
mod fixtures;
use fixtures::*;
use sparq_proved_evaluator::v3::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, Error, Nonces, embedded_artifact, embedded_pin};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, Provenance, v2, v3};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Default)]
struct MemoryNonces(BTreeSet<[u8; 32]>);
impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

fn prove_and_check(input: &v3::Witness, name: &str) -> v3::Journal {
    let guest =
        AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin()).unwrap();
    let r0vm =
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("real local r0vm required"));
    let proof = prove_with_artifact(input, &r0vm, &guest).expect("genuine V3 Succinct receipt");
    let mut nonces = MemoryNonces::default();
    let journal = verify_with_artifact(&proof, &input.request, &mut nonces, &guest).unwrap();
    assert_eq!(journal, v3::evaluate(input).unwrap());
    assert!(verify_with_artifact(&proof, &input.request, &mut nonces, &guest).is_err());
    for field in 0..8 {
        let mut changed = input.request.clone();
        match field {
            0 => changed.version = v2::VERSION,
            1 => changed.query.push(' '),
            2 => changed.nonce[0] ^= 1,
            3 => changed.policy.canonicalization.max_quads -= 1,
            4 => changed.policy.dataset.max_rows -= 1,
            5 => changed.contract = ProofContract::SelectedSupport,
            6 => {
                changed.authority = DatasetAuthority::VerifierAgreed {
                    commitment: [79; 32],
                }
            }
            _ => {
                changed.authority = match input.request.authority {
                    DatasetAuthority::HolderDeclared => DatasetAuthority::VerifierAgreed {
                        commitment: journal.dataset_commitment,
                    },
                    DatasetAuthority::VerifierAgreed { .. } => DatasetAuthority::HolderDeclared,
                }
            }
        }
        assert!(
            verify_with_artifact(&proof, &changed, &mut MemoryNonces::default(), &guest).is_err()
        );
    }
    let mut tampered = proof.clone();
    tampered.receipt.journal.bytes[0] ^= 1;
    assert!(
        verify_with_artifact(
            &tampered,
            &input.request,
            &mut MemoryNonces::default(),
            &guest
        )
        .is_err()
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
    let v2_request = v2::Request {
        version: v2::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v2::Dialect::SparqSparql11DatasetV2,
        query: input.request.query.clone(),
        authority: input.request.authority.clone(),
        policy: input.request.policy.dataset.clone(),
        nonce: input.request.nonce,
    };
    assert!(
        sparq_proved_evaluator::v2::verify_with_artifact(
            &proof,
            &v2_request,
            &mut MemoryNonces::default(),
            &guest
        )
        .is_err(),
        "V3 receipts cannot be interpreted as V2"
    );
    evidence::record(name, &input.request, &proof.receipt);
    journal
}

#[test]
fn real_v3_holder_table_retains_duplicate_unbound_and_cross_row_identity() {
    let input = witness(BAG, SHARED_NODE, &[]);
    let journal = prove_and_check(&input, "v3-holder-bag");
    assert_eq!(journal.provenance, Provenance::HolderDeclaredOnly);
    let v3::CanonicalResult::Select { rows, .. } = journal.result else {
        panic!("table")
    };
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[2], rows[3]);
    assert!(
        rows.iter()
            .all(|row| row[0] == rows[0][0] && row[2].is_none())
    );
}

#[test]
fn real_v3_verifier_construct_proves_fresh_nodes_and_graph_set_semantics() {
    let mut input = witness(CONSTRUCT, CONSTRUCT_SOURCE, &["http://ex/empty"]);
    accepted(&mut input);
    input.request.nonce = [73; 32];
    let journal = prove_and_check(&input, "v3-verifier-construct");
    assert_eq!(journal.provenance, Provenance::VerifierAcceptedCommitment);
    let v3::CanonicalResult::Graph { ntriples } = journal.result else {
        panic!("graph")
    };
    assert_eq!(ntriples.lines().count(), 5);
    assert!(!ntriples.contains("tc0_0_0"));
}

#[test]
fn real_v3_holder_describe_binds_the_explicit_blank_closure_policy() {
    let mut input = witness("DESCRIBE ex:a", DESCRIBE_SOURCE, &[]);
    input.request.nonce = [83; 32];
    let journal = prove_and_check(&input, "v3-holder-describe");
    let v3::CanonicalResult::Graph { ntriples } = journal.result else {
        panic!("graph")
    };
    assert_eq!(ntriples.lines().count(), 4);
    assert!(!ntriples.contains("inbound") && !ntriples.contains("outside"));
}

#[test]
fn v3_wire_version_and_maximum_input_fit_without_reinterpreting_v2() {
    let mut input = witness("ASK {}", "", &[]);
    input.dataset.named_graphs = (0..v2::MAX_NAMED_GRAPHS)
        .map(|i| format!("http://ex/{i}"))
        .collect();
    let names: usize = input.dataset.named_graphs.iter().map(String::len).sum();
    input.dataset.nquads =
        " ".repeat(sparq_proved_evaluator_model::MAX_DATASET_BYTES as usize - names);
    input.request.query = " ".repeat(sparq_proved_evaluator_model::MAX_QUERY_BYTES);
    let encoded = risc0_zkvm::serde::to_vec(&input).unwrap();
    assert_eq!(encoded[0], v3::VERSION);
    assert!(encoded.len() * 4 <= sparq_proved_evaluator_model::MAX_WITNESS_BYTES);
    let decoded: v3::Witness = risc0_zkvm::serde::from_slice(&encoded).unwrap();
    assert_eq!(decoded.request, input.request);
    assert_eq!(decoded.dataset.nquads, input.dataset.nquads);
    assert_eq!(decoded.dataset.named_graphs, input.dataset.named_graphs);
}

// [GPT-6] No ignored tests, mock mode, network prover, or tool-presence skip.
use risc0_zkvm::{
    Executor, ExecutorEnv, ExternalProver, FakeReceipt, InnerReceipt, Receipt, ReceiptClaim,
    VerifierContext,
};
use sparq_proved_evaluator::{
    AcceptedGuest, ArtifactPin, Error, Nonces, Presentation, embedded_artifact, embedded_pin,
    method_id, prove, prove_with_artifact, verify, verify_with_artifact,
};
use sparq_proved_evaluator_methods::SPARQ_EXACT_GUEST_ELF;
use sparq_proved_evaluator_model::*;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Default)]
struct MemoryNonces(BTreeSet<[u8; 32]>);
impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

fn witness(query: &str) -> Witness {
    let dataset = PrivateDataset {
        ntriples: include_str!("../../fixtures/default.nt").into(),
        salt: [17; 32],
    };
    let policy = Policy::default();
    Witness {
        request: Request {
            version: VERSION,
            contract: ProofContract::ExactDataset,
            dialect: Dialect::SparqSparql11SnapshotV1,
            query: query.into(),
            authority: DatasetAuthority::VerifierAgreed {
                commitment: dataset_commitment(&dataset, &policy).unwrap(),
            },
            policy,
            nonce: [29; 32],
        },
        dataset,
    }
}

fn r0vm() -> PathBuf {
    std::env::var_os("RISC0_SERVER_PATH")
        .map(PathBuf::from)
        .expect("RISC0_SERVER_PATH must identify the installed real r0vm 3.0.6 executable")
}

fn accepted_guest() -> AcceptedGuest {
    if let Some(path) = std::env::var_os("SPARQ_ACCEPTED_GUEST_ARTIFACT") {
        // Test/deployment-owner configuration, deliberately separate from the presentation.
        let pin: ArtifactPin = serde_json::from_slice(
            &std::fs::read(
                std::env::var_os("SPARQ_ACCEPTED_GUEST_PIN")
                    .expect("a separately accepted artifact pin is required"),
            )
            .unwrap(),
        )
        .unwrap();
        AcceptedGuest::from_artifact(std::fs::read(path).unwrap(), &pin).unwrap()
    } else {
        AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin()).unwrap()
    }
}

#[test]
fn real_exact_result_rejects_all_public_binding_tampering_and_replay() {
    let witness = witness(include_str!("../../fixtures/combined.rq"));
    let guest = accepted_guest();
    let verify =
        |p: &Presentation, r: &Request, n: &mut MemoryNonces| verify_with_artifact(p, r, n, &guest);
    eprintln!(
        "proving authenticated OPTIONAL/MINUS/COUNT/subquery/ORDER/LIMIT/path/EXISTS fixture"
    );
    let presentation = prove_with_artifact(&witness, &r0vm(), &guest).expect("genuine local proof");
    if guest.image_id() != method_id() {
        assert!(
            sparq_proved_evaluator::verify(
                &presentation,
                &witness.request,
                &mut MemoryNonces::default()
            )
            .is_err(),
            "a different local build is not the accepted guest"
        );
    }
    let journal = verify(
        &presentation,
        &witness.request,
        &mut MemoryNonces::default(),
    )
    .unwrap();
    assert_eq!(
        journal,
        evaluate(&witness).unwrap(),
        "real guest vs native semantic oracle"
    );
    let CanonicalResult::Select { order, rows, .. } = &journal.result else {
        panic!("SELECT");
    };
    assert_eq!(*order, RowOrder::Sequence);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1][2], None, "unmatched OPTIONAL is unbound");
    assert_eq!(
        rows[0][1].as_deref(),
        Some("\"4\"^^<http://www.w3.org/2001/XMLSchema#integer>")
    );
    let mut nonces = MemoryNonces::default();
    verify(&presentation, &witness.request, &mut nonces).unwrap();
    assert!(verify(&presentation, &witness.request, &mut nonces).is_err());
    for selector in 0..5 {
        let mut expected = witness.request.clone();
        match selector {
            0 => expected.query.push(' '),
            1 => expected.nonce[0] ^= 1,
            2 => expected.policy.max_rows -= 1,
            3 => expected.authority = DatasetAuthority::HolderDeclared,
            _ => {
                expected.authority = DatasetAuthority::VerifierAgreed {
                    commitment: [42; 32],
                }
            }
        }
        assert!(verify(&presentation, &expected, &mut MemoryNonces::default()).is_err());
    }
    let mut tampered = presentation.clone();
    tampered.receipt.journal.bytes[0] ^= 1;
    assert!(verify(&tampered, &witness.request, &mut MemoryNonces::default()).is_err());
    let mut wrong_id = guest.image_id();
    wrong_id[0] ^= 1;
    assert!(
        presentation
            .receipt
            .verify_with_context(&VerifierContext::default().with_dev_mode(false), wrong_id)
            .is_err()
    );
    let fake = Presentation {
        receipt: Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
                guest.image_id(),
                presentation.receipt.journal.bytes.clone(),
            ))),
            presentation.receipt.journal.bytes.clone(),
        ),
    };
    assert!(verify(&fake, &witness.request, &mut MemoryNonces::default()).is_err());
    let receipt_bytes = serde_json::to_vec(&presentation).unwrap();
    eprintln!(
        "real exact-evaluator proof verified; presentation_bytes={}; accepted_image_id={:?}",
        receipt_bytes.len(),
        guest.image_id()
    );
}

#[test]
fn actual_guest_rejects_bad_anchors_nondeterminism_and_exhausted_capacity() {
    let base = witness("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }");
    let mut cases = Vec::new();
    let mut w = base.clone();
    w.dataset.ntriples.clear();
    cases.push(w);
    let mut w = base.clone();
    w.request.query = "SELECT (NOW() AS ?x) WHERE {}".into();
    cases.push(w);
    let mut w = base.clone();
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.request.policy.max_triples = 1;
    cases.push(w);
    let mut w = base.clone();
    w.request.contract = ProofContract::SelectedSupport;
    cases.push(w);
    let mut w = base.clone();
    w.request.authority = DatasetAuthority::HolderDeclared;
    w.request.query = "SELECT ?s WHERE { ?s <http://ex/score> ?v }".into();
    w.request.policy.max_rows = 1;
    cases.push(w);
    let mut w = base;
    w.request.query = " ".repeat(MAX_QUERY_BYTES + 1);
    cases.push(w);
    // Execute the exact same exclusion corpus in the actual guest. Native
    // rejection alone is not evidence that the guest enforces this boundary.
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/conformance/cases.json")).unwrap();
    let excluded = corpus["cases"].as_array().unwrap().iter().filter(|case| {
        case["features"].as_array().unwrap().iter().any(|feature| {
            feature == "exists_complex_bodies" || feature == "nullable_path_composition"
        })
    });
    let mut excluded_count = 0;
    for case in excluded {
        cases.push(witness(case["query"].as_str().unwrap()));
        excluded_count += 1;
    }
    assert!(
        excluded_count > 0,
        "actual guest exclusion corpus is required"
    );
    let executor = ExternalProver::new("real-sparq-guest-negative", r0vm());
    // A broken local executor must not count as successful relation rejection.
    let positive = ExecutorEnv::builder()
        .session_limit(Some(1 << 24))
        .write(&witness("ASK {}"))
        .unwrap()
        .build()
        .unwrap();
    executor
        .execute(positive, SPARQ_EXACT_GUEST_ELF)
        .expect("positive actual-guest execution before negative cases");
    for w in cases {
        let env = ExecutorEnv::builder()
            .session_limit(Some(1 << 24))
            .write(&w)
            .unwrap()
            .build()
            .unwrap();
        assert!(
            executor.execute(env, SPARQ_EXACT_GUEST_ELF).is_err(),
            "rejected relation must not reach Halted(0)"
        );
    }
}

#[test]
fn real_holder_declared_bag_preserves_duplicates_unbound_and_provenance() {
    let mut witness = witness(
        "SELECT ?s ?missing WHERE { \
         { ?s <http://ex/value> ?v } UNION \
         { VALUES (?s ?missing) { (<http://ex/alice> UNDEF) } } }",
    );
    witness.request.authority = DatasetAuthority::HolderDeclared;
    witness.request.nonce = [30; 32];
    eprintln!("proving holder-declared UNION/VALUES/bag fixture");
    let presentation = prove(&witness, &r0vm()).expect("genuine holder-declared proof");
    let journal = verify(
        &presentation,
        &witness.request,
        &mut MemoryNonces::default(),
    )
    .unwrap();
    assert_eq!(journal.provenance, Provenance::HolderDeclaredOnly);
    assert_eq!(journal, evaluate(&witness).unwrap());
    let CanonicalResult::Select { order, rows, .. } = journal.result else {
        panic!("SELECT");
    };
    assert_eq!(order, RowOrder::Bag);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[1], rows[2], "three copies must survive bag encoding");
    assert!(rows.iter().all(|r| r[1].is_none()));
    let mut stronger_claim = witness.request.clone();
    stronger_claim.authority = DatasetAuthority::VerifierAgreed {
        commitment: journal.dataset_commitment,
    };
    assert!(verify(&presentation, &stronger_claim, &mut MemoryNonces::default()).is_err());
}

#[test]
fn real_false_ask_proves_absence_from_the_accepted_complete_graph() {
    let mut witness = witness("ASK { ?s <http://ex/score> ?v FILTER(?v > 100) }");
    witness.request.nonce = [31; 32];
    eprintln!("proving false ASK on the accepted complete graph");
    let presentation = prove(&witness, &r0vm()).expect("genuine false-ASK proof");
    let journal = verify(
        &presentation,
        &witness.request,
        &mut MemoryNonces::default(),
    )
    .unwrap();
    assert_eq!(journal.result, CanonicalResult::Ask(false));
    assert_eq!(journal, evaluate(&witness).unwrap());
}

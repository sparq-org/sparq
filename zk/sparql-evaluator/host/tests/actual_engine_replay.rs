// [OPUS-5.5] Ignored genuine V3 receipts for one explicitly supplied retained replay cell.
//! Driver only: set `SPARQ_ENGINE_REPLAY_PROOF_JOB` and `RISC0_SERVER_PATH`, then
//! run this ignored test by name. See `bench/zk-bindings/engine-proof-replay.md`.
//! A missing job, tool or input fails; it never counts as a proof run.
#[path = "../examples/engine_replay_proof/controls.rs"]
mod controls;

use controls::{MemoryNonces, verifier_controls};
use serde::Deserialize;
use sparq_proved_evaluator::v3::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error};
use sparq_proved_evaluator_model::replay::{
    self, ExpectedManifest, HolderManifest, Originals, VerifierManifest,
};
use sparq_proved_evaluator_model::{DatasetAuthority, EvaluationError, Provenance, Rejected, v3};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const JOB_SCHEMA: &str = "sparq.engine-replay-proof.real-test.v1";

/// Explicit driver job; every input is independently supplied by the caller.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema: String,
    replay_directory: PathBuf,
    holder: PathBuf,
    holder_declared_request: PathBuf,
    verifier_agreed_request: PathBuf,
    expected: PathBuf,
    guest: PathBuf,
    pin: PathBuf,
}

fn read(path: &Path, limit: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    File::open(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(bytes.len() <= limit, "{} exceeds its read bound", path.display());
    bytes
}

fn json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    serde_json::from_slice(&read(path, 4 << 20))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
#[ignore = "genuine proofs: needs SPARQ_ENGINE_REPLAY_PROOF_JOB, RISC0_SERVER_PATH and an accepted artifact"]
fn real_engine_replay_cell_proves_both_authorities_and_rejects_substitutions() {
    assert!(
        std::env::var_os("RISC0_DEV_MODE").is_none(),
        "RISC0_DEV_MODE must be unset"
    );
    let job: Job = json(Path::new(
        &std::env::var_os("SPARQ_ENGINE_REPLAY_PROOF_JOB").expect("explicit replay proof job"),
    ));
    assert_eq!(job.schema, JOB_SCHEMA);
    let r0vm =
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("real local r0vm required"));
    let pin: ArtifactPin = json(&job.pin);
    let guest = AcceptedGuest::from_artifact(read(&job.guest, 32 << 20), &pin)
        .expect("independently pinned guest artifact");
    let directory = &job.replay_directory;
    let record = read(&directory.join("record.json"), 16 << 20);
    let query = read(&directory.join("query.rq"), 16 << 20);
    let data = read(&directory.join("data.ttl"), 16 << 20);
    let originals = Originals {
        record: &record,
        query: &query,
        data: &data,
    };
    let holder: HolderManifest = json(&job.holder);
    let expected: ExpectedManifest = json(&job.expected);
    let holder_request: VerifierManifest = json(&job.holder_declared_request);
    let agreed_request: VerifierManifest = json(&job.verifier_agreed_request);
    assert_eq!(holder_request.request.authority, DatasetAuthority::HolderDeclared);
    assert!(matches!(
        agreed_request.request.authority,
        DatasetAuthority::VerifierAgreed { .. }
    ));
    assert_eq!(holder_request.originals, agreed_request.originals, "one original cell");
    assert_ne!(
        holder_request.request.nonce, agreed_request.request.nonce,
        "distinct verifier challenges"
    );

    let mut agreed_witness = None;
    for (verifier, provenance) in [
        (&holder_request, Provenance::HolderDeclaredOnly),
        (&agreed_request, Provenance::VerifierAcceptedCommitment),
    ] {
        let prepared = replay::prepare(originals, verifier, &holder).expect("prepared original");
        let native = prepared
            .evaluate_against(&expected)
            .expect("native V3 agrees with the independent expectation");
        eprintln!("engine replay: genuine {provenance:?} proof starts");
        let presentation = prove_with_artifact(prepared.witness(), &r0vm, &guest)
            .expect("genuine V3 Succinct receipt");
        assert!(matches!(
            presentation.receipt.inner,
            risc0_zkvm::InnerReceipt::Succinct(_)
        ));
        let mut consumed = MemoryNonces::default();
        let journal =
            verify_with_artifact(&presentation, &verifier.request, &mut consumed, &guest)
                .expect("independent verification");
        assert_eq!(journal.provenance, provenance);
        assert!(expected.matches(&journal.result).expect("bounded comparison"));
        assert_eq!(journal, native, "differential: verified and native journals");
        assert_eq!(journal.dataset_commitment, prepared.dataset_commitment());
        let controls = verifier_controls(
            &presentation,
            &verifier.request,
            &journal,
            &guest,
            &mut consumed,
        )
        .expect("verifier substitution and tamper controls");
        assert_eq!(
            controls,
            [
                "expected_nonce_substitution",
                "expected_query_substitution",
                "expected_commitment_substitution",
                "expected_authority_substitution",
                "consumed_nonce_replay",
                "public_journal_result_tamper",
                "public_journal_commitment_tamper",
                "succinct_seal_tamper",
                "fake_receipt",
            ]
        );
        if provenance == Provenance::VerifierAcceptedCommitment {
            agreed_witness = Some(prepared.witness().clone());
        }
    }

    // Malicious witness: the prover omits one original statement under the
    // verifier-agreed anchor. The guest aborts, so no presentation exists.
    let mut omitted = agreed_witness.expect("verifier-agreed request exercised");
    let end = omitted
        .dataset
        .nquads
        .find('\n')
        .expect("the original fixture needs at least one statement")
        + 1;
    omitted.dataset.nquads.replace_range(..end, "");
    assert_eq!(
        v3::evaluate_detailed(&omitted).unwrap_err(),
        EvaluationError::Rejected(Rejected("V3 complete dataset anchor mismatch"))
    );
    assert_eq!(
        prove_with_artifact(&omitted, &r0vm, &guest).unwrap_err(),
        Error("real local proof failed")
    );
}

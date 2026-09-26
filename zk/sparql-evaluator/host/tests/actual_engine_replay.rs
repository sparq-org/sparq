// [OPUS-5.5] Ignored genuine V3 receipts for one explicitly supplied retained replay cell.
//! Driver only: set `SPARQ_ENGINE_REPLAY_PROOF_JOB` and `RISC0_SERVER_PATH`, then
//! run this ignored test by name. See `bench/zk-bindings/engine-proof-replay.md`.
//! A missing job, tool or input fails; it never counts as a proof run. Public
//! receipts persist under the job's new output directory, which is never removed;
//! `summary.json` is written only after every proof, control and negative check.
#[path = "../examples/engine_replay_proof/controls.rs"]
mod controls;

use controls::{MemoryNonces, verifier_controls};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::v3::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error};
use sparq_proved_evaluator_model::replay::{
    self, ExpectedManifest, HolderManifest, Originals, VerifierManifest,
};
use sparq_proved_evaluator_model::{DatasetAuthority, EvaluationError, Provenance, Rejected, v3};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const JOB_SCHEMA: &str = "sparq.engine-replay-proof.real-test.v1";
const SUMMARY_SCHEMA: &str = "sparq.engine-replay-proof.real-test-summary.v1";
/// Exact verifier-only control names, in `verifier_controls` order.
const CONTROLS: [&str; 9] = [
    "expected_nonce_substitution",
    "expected_query_substitution",
    "expected_commitment_substitution",
    "expected_authority_substitution",
    "consumed_nonce_replay",
    "public_journal_result_tamper",
    "public_journal_commitment_tamper",
    "succinct_seal_tamper",
    "fake_receipt",
];

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
    /// Absolute and absent; outside the source checkout and the replay directory.
    new_output_directory: PathBuf,
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

fn parse<T: DeserializeOwned>(bytes: &[u8], path: &Path) -> T {
    serde_json::from_slice(bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Reads one bounded input and records the SHA-256 of its exact bytes.
fn input(digests: &mut Map<String, Value>, name: &str, path: &Path, limit: usize) -> Vec<u8> {
    let bytes = read(path, limit);
    digests.insert(format!("{name}_sha256"), json!(sha256(&bytes)));
    bytes
}

/// Creates one new directory, owner-only on Unix; an existing path is an error.
fn create_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Writes one new file, owner-only on Unix; never replaces existing evidence.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// Creates the evidence directory below a canonicalized parent.
///
/// `replay` must already be canonical. Nothing here or later removes evidence.
fn fresh_output(requested: &Path, replay: &Path) -> Result<PathBuf, String> {
    if !requested.is_absolute() {
        return Err("new_output_directory must be absolute".into());
    }
    let (Some(parent), Some(name)) = (requested.parent(), requested.file_name()) else {
        return Err("new_output_directory needs a parent and a final component".into());
    };
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("output parent: {error}"))?;
    let output = parent.join(name);
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .map_err(|error| format!("source checkout: {error}"))?;
    if output.starts_with(&checkout) {
        return Err("new_output_directory must be outside the source checkout".into());
    }
    if output.starts_with(replay) {
        return Err("new_output_directory must be outside the replay directory".into());
    }
    create_dir(&output).map_err(|error| format!("{}: {error}", output.display()))?;
    Ok(output)
}

fn pretty(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("typed public JSON")
}

#[test]
#[ignore = "genuine proofs: needs SPARQ_ENGINE_REPLAY_PROOF_JOB, RISC0_SERVER_PATH and an accepted artifact"]
fn real_engine_replay_cell_proves_both_authorities_and_rejects_substitutions() {
    assert!(
        std::env::var_os("RISC0_DEV_MODE").is_none(),
        "RISC0_DEV_MODE must be unset"
    );
    let job_path = PathBuf::from(
        std::env::var_os("SPARQ_ENGINE_REPLAY_PROOF_JOB").expect("explicit replay proof job"),
    );
    let job_bytes = read(&job_path, 4 << 20);
    let job: Job = parse(&job_bytes, &job_path);
    assert_eq!(job.schema, JOB_SCHEMA);
    let r0vm =
        PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("real local r0vm required"));
    let mut digests = Map::new();
    let pin: ArtifactPin = parse(&input(&mut digests, "pin", &job.pin, 4 << 20), &job.pin);
    let guest = AcceptedGuest::from_artifact(input(&mut digests, "guest", &job.guest, 32 << 20), &pin)
        .expect("independently pinned guest artifact");
    let directory = job.replay_directory.canonicalize().expect("replay directory");
    let record = input(&mut digests, "record", &directory.join("record.json"), 16 << 20);
    let query = input(&mut digests, "query", &directory.join("query.rq"), 16 << 20);
    let data = input(&mut digests, "data", &directory.join("data.ttl"), 16 << 20);
    let originals = Originals {
        record: &record,
        query: &query,
        data: &data,
    };
    let (holder_path, expected_path) = (&job.holder, &job.expected);
    let holder: HolderManifest = parse(&input(&mut digests, "holder", holder_path, 4 << 20), holder_path);
    let expected: ExpectedManifest =
        parse(&input(&mut digests, "expected", expected_path, 4 << 20), expected_path);
    let [holder_request, agreed_request]: [VerifierManifest; 2] = [
        ("holder_declared_request", &job.holder_declared_request),
        ("verifier_agreed_request", &job.verifier_agreed_request),
    ]
    .map(|(name, path)| parse(&input(&mut digests, name, path, 4 << 20), path));
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
    let output = fresh_output(&job.new_output_directory, &directory).expect("new evidence directory");
    eprintln!("engine replay: evidence directory {}", output.display());

    let mut finished = Vec::new();
    let mut agreed_witness = None;
    for (verifier, provenance, name) in [
        (&holder_request, Provenance::HolderDeclaredOnly, "holder_declared"),
        (&agreed_request, Provenance::VerifierAcceptedCommitment, "verifier_agreed"),
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

        // Public outputs only; the private witness is never serialized.
        let child = output.join(name);
        create_dir(&child).expect("new authority directory");
        let mut evidence = json!({
            "authority": name,
            "provenance": provenance,
            "verified": true,
            "request_digest": hex(&journal.request_digest),
            "dataset_commitment": hex(&journal.dataset_commitment),
            "receipt_journal_sha256": sha256(&presentation.receipt.journal.bytes),
            "checks": ["independent_verification", "expected_result", "native_journal", "dataset_commitment"],
            "files": {},
        });
        for (file, bytes) in [
            ("presentation.json", pretty(&presentation)),
            ("journal.json", pretty(&journal)),
            ("request.json", pretty(&verifier.request)),
        ] {
            write_new(&child.join(file), &bytes).expect("new evidence file");
            evidence["files"][file] = json!(sha256(&bytes));
        }
        write_new(&child.join("verified.json"), &pretty(&evidence)).expect("partial evidence");
        let controls = verifier_controls(
            &presentation,
            &verifier.request,
            &journal,
            &guest,
            &mut consumed,
        )
        .unwrap_or_else(|error| {
            write_new(&child.join("control-failure.json"), &pretty(&json!({ "diagnostic": error })))
                .expect("control diagnostic");
            panic!("verifier substitution and tamper controls: {error}")
        });
        assert_eq!(controls, CONTROLS);
        evidence["controls"] = json!(controls);
        finished.push(evidence);
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
    let native_cause = v3::evaluate_detailed(&omitted).unwrap_err();
    assert_eq!(
        native_cause,
        EvaluationError::Rejected(Rejected("V3 complete dataset anchor mismatch"))
    );
    let observed = prove_with_artifact(&omitted, &r0vm, &guest).unwrap_err();
    assert_eq!(observed, Error("real local proof failed"));

    let proof_count = finished.len();
    let verified_count = finished.iter().filter(|evidence| evidence["verified"] == true).count();
    assert_eq!((proof_count, verified_count), (2, 2));
    let summary = json!({
        "schema": SUMMARY_SCHEMA,
        "job_sha256": sha256(&job_bytes),
        "inputs": digests,
        "accepted_guest": {
            "pin": pin,
            "artifact_sha256": hex(&pin.sha256),
            "image_id": guest.image_id(),
        },
        "authorities": finished,
        "controls_per_authority": CONTROLS,
        "proof_count": proof_count,
        "verified_proof_count": verified_count,
        "production_authentication_claims": 0,
        "omission_negative": {
            "native_expected_cause": format!("{native_cause:?}"),
            "observed_host_error": observed.0,
            "guest_abort_certified": false,
            "note": "the host returns this generic error for infrastructure failures and guest aborts alike; an independent log inspection must confirm the guest abort",
        },
        "scope": "synthetic experiment evidence; not externally audited; no source credential is authenticated",
    });
    write_new(&output.join("summary.json"), &pretty(&summary)).expect("final summary");
}

#[test]
fn job_requires_a_new_output_directory_and_rejects_unknown_fields() {
    let mut fields = Map::new();
    fields.insert("schema".into(), json!(JOB_SCHEMA));
    for key in [
        "replay_directory",
        "holder",
        "holder_declared_request",
        "verifier_agreed_request",
        "expected",
        "guest",
        "pin",
        "new_output_directory",
    ] {
        fields.insert(key.into(), json!(format!("/abs/{key}")));
    }
    let job = |fields: &Map<String, Value>| serde_json::from_value::<Job>(Value::Object(fields.clone()));
    let complete = job(&fields).expect("complete job");
    assert_eq!(complete.new_output_directory, Path::new("/abs/new_output_directory"));
    let mut missing = fields.clone();
    missing.remove("new_output_directory");
    let error = job(&missing).err().expect("missing output rejected").to_string();
    assert!(error.contains("missing field `new_output_directory`"), "{error}");
    fields.insert("output".into(), json!("/abs/output"));
    let error = job(&fields).err().expect("unknown field rejected").to_string();
    assert!(error.contains("unknown field `output`"), "{error}");
}

#[test]
fn output_directory_must_be_new_owner_only_and_outside_checkout_and_replay() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("sparq-replay-guard-{}-{nanos}", std::process::id()));
    create_dir(&base).unwrap();
    let base = base.canonicalize().unwrap();
    let replay = base.join("replay");
    create_dir(&replay).unwrap();
    create_dir(&base.join("taken")).unwrap();
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("new-evidence-guard");
    for (requested, reason) in [
        (PathBuf::from("relative-evidence"), "absolute"),
        (checkout.clone(), "source checkout"),
        (replay.join("evidence"), "replay directory"),
        (replay.clone(), "replay directory"),
        (base.join("missing/evidence"), "output parent"),
        (base.join("taken"), "taken"),
    ] {
        let error = fresh_output(&requested, &replay).unwrap_err();
        assert!(error.contains(reason), "{}: {error}", requested.display());
    }
    assert!(!checkout.exists() && !replay.join("evidence").exists());
    let output = fresh_output(&base.join("evidence"), &replay).unwrap();
    assert_eq!(output, base.join("evidence"));
    write_new(&output.join("summary.json"), b"{}").unwrap();
    assert!(write_new(&output.join("summary.json"), b"[]").is_err());
    assert_eq!(fs::read(output.join("summary.json")).unwrap(), b"{}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&output), mode(&output.join("summary.json"))), (0o700, 0o600));
    }
    fs::remove_dir_all(&base).unwrap();
}

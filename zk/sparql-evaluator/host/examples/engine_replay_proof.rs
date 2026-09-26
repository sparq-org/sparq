// [OPUS-5.5] Retained engine-replay cell to genuine exact V3 proof bridge.
//! Prepare, or prove and independently verify, one retained engine-replay cell.
//!
//! See `bench/zk-bindings/engine-proof-replay.md` for inputs, outputs and scope.
//! Experimental and not externally audited; no source credential is authenticated.
#[path = "engine_replay_proof/controls.rs"]
mod controls;

use controls::{MemoryNonces, verifier_controls};
use risc0_zkvm::InnerReceipt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::v3::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin};
use sparq_proved_evaluator_model::replay::{
    self, ExpectedManifest, HolderManifest, Originals, Prepared, ReplayError, VerifierManifest,
};
use sparq_proved_evaluator_model::{DatasetAuthority, v3};
use std::{
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn StdError>>;

const HELP: &str = "Usage:
  engine_replay_proof prepare REPLAY_DIR HOLDER.json VERIFIER.json EXPECTED.json NEW_OUTPUT_DIR
  engine_replay_proof real REPLAY_DIR HOLDER.json VERIFIER.json EXPECTED.json ACCEPTED_GUEST.bin INDEPENDENT_PIN.json LOCAL_R0VM NEW_OUTPUT_DIR
REPLAY_DIR is one retained native cell with record.json, query.rq and data.ttl.
VERIFIER.json and EXPECTED.json are independently supplied; accept the pin
independently of the artifact. Only explicitly synthetic inputs with a published
fixed salt are admitted. NEW_OUTPUT_DIR must not exist and must be outside the
source checkout and the replay directory.
Exit 0: prepared, or proved and independently verified. Exit 1: typed rejection
recorded in status.json. Exit 2: usage or infrastructure failure.";

const STATUS_SCHEMA: &str = "sparq.engine-replay-proof.status.v1";
const PRIVATE_RECORD: &str = "private/witness-summary.json";
/// Adapter read bound per retained file; matches the native controller's record bound.
const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;
/// Manifest read bound; an expected V3 result is at most 1 MiB before JSON wrapping.
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
/// Pin read bound; an `ArtifactPin` is a small fixed-shape JSON object.
const MAX_PIN_BYTES: usize = 4096;
/// Guest read bound; equal to `AcceptedGuest`'s own artifact capacity.
const MAX_GUEST_BYTES: usize = 32 * 1024 * 1024;

struct Invocation {
    mode: &'static str,
    replay: PathBuf,
    holder: PathBuf,
    verifier: PathBuf,
    expected: PathBuf,
    real: Option<RealPaths>,
    output: PathBuf,
}

struct RealPaths {
    guest: PathBuf,
    pin: PathBuf,
    r0vm: PathBuf,
}

struct Inputs {
    replay: PathBuf,
    record: Vec<u8>,
    query: Vec<u8>,
    data: Vec<u8>,
    holder: HolderManifest,
    verifier: VerifierManifest,
    expected: ExpectedManifest,
    digests: Value,
}

struct RealTools {
    guest: AcceptedGuest,
    r0vm: PathBuf,
    r0vm_sha256: String,
    identity: Value,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn file_digest(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let len = input.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        hash.update(&buffer[..len]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn bounded_read(path: &Path, max: usize) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(format!("{} must be a regular file", path.display()).into());
    }
    let mut bytes = Vec::new();
    file.take((max + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(format!("{} exceeds its adapter read bound", path.display()).into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8], protected: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if protected {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = protected;
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

fn create_protected_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(path)?;
    Ok(())
}

fn command_text(program: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        return Err("tool identity command failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn parse(args: &[PathBuf]) -> Result<Invocation> {
    let (mode, real) = match (args.first().and_then(|mode| mode.to_str()), args.len()) {
        (Some("prepare"), 6) => ("prepare", None),
        (Some("real"), 9) => (
            "real",
            Some(RealPaths {
                guest: args[5].clone(),
                pin: args[6].clone(),
                r0vm: args[7].clone(),
            }),
        ),
        _ => return Err(HELP.into()),
    };
    Ok(Invocation {
        mode,
        replay: args[1].clone(),
        holder: args[2].clone(),
        verifier: args[3].clone(),
        expected: args[4].clone(),
        real,
        output: args[args.len() - 1].clone(),
    })
}

fn load_inputs(invocation: &Invocation) -> Result<Inputs> {
    let replay = invocation.replay.canonicalize()?;
    if !replay.is_dir() {
        return Err("REPLAY_DIR must be a directory".into());
    }
    let record = bounded_read(&replay.join("record.json"), MAX_RETAINED_BYTES)?;
    let query = bounded_read(&replay.join("query.rq"), MAX_RETAINED_BYTES)?;
    let data = bounded_read(&replay.join("data.ttl"), MAX_RETAINED_BYTES)?;
    let holder_bytes = bounded_read(&invocation.holder, MAX_MANIFEST_BYTES)?;
    let verifier_bytes = bounded_read(&invocation.verifier, MAX_MANIFEST_BYTES)?;
    let expected_bytes = bounded_read(&invocation.expected, MAX_MANIFEST_BYTES)?;
    let digests = json!({
        "record_sha256": sha256(&record),
        "query_sha256": sha256(&query),
        "data_sha256": sha256(&data),
        "holder_manifest_sha256": sha256(&holder_bytes),
        "verifier_manifest_sha256": sha256(&verifier_bytes),
        "expected_manifest_sha256": sha256(&expected_bytes),
    });
    Ok(Inputs {
        replay,
        record,
        query,
        data,
        holder: serde_json::from_slice(&holder_bytes)?,
        verifier: serde_json::from_slice(&verifier_bytes)?,
        expected: serde_json::from_slice(&expected_bytes)?,
        digests,
    })
}

fn load_tools(paths: &RealPaths) -> Result<RealTools> {
    if std::env::var_os("RISC0_DEV_MODE").is_some() {
        return Err("RISC0_DEV_MODE must be unset; only genuine Succinct receipts are accepted".into());
    }
    let artifact = bounded_read(&paths.guest, MAX_GUEST_BYTES)?;
    let pin_bytes = bounded_read(&paths.pin, MAX_PIN_BYTES)?;
    let pin: ArtifactPin = serde_json::from_slice(&pin_bytes)?;
    let artifact_sha256 = sha256(&artifact);
    let guest = AcceptedGuest::from_artifact(artifact, &pin)?;
    let r0vm = paths.r0vm.canonicalize()?;
    let r0vm_sha256 = file_digest(&r0vm)?;
    let version = command_text(&r0vm, &["--version"])?;
    if version.split_whitespace().last() != Some("3.0.6") {
        return Err("r0vm must report exactly version 3.0.6".into());
    }
    let identity = json!({
        "accepted_artifact_sha256": artifact_sha256,
        "accepted_pin": pin,
        "accepted_pin_file_sha256": sha256(&pin_bytes),
        "pin_source": "caller-supplied independent trust input; never derived from the artifact",
        "r0vm": r0vm.display().to_string(),
        "r0vm_sha256": r0vm_sha256,
        "r0vm_version_output": version,
        "adapter_executable_sha256": file_digest(&std::env::current_exe()?)?,
        "receipt_policy": "Succinct only; dev mode disabled by host feature and verifier context; RISC0_DEV_MODE unset",
    });
    Ok(RealTools {
        guest,
        r0vm,
        r0vm_sha256,
        identity,
    })
}

fn fresh_output(requested: &Path, replay: &Path) -> Result<PathBuf> {
    let name = requested
        .file_name()
        .ok_or("NEW_OUTPUT_DIR needs a final path component")?;
    let parent = match requested.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.canonicalize()?,
        _ => std::env::current_dir()?.canonicalize()?,
    };
    let output = parent.join(name);
    // Witness summaries must never land in tracked sources or retained originals.
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if checkout
        .canonicalize()
        .is_ok_and(|checkout| output.starts_with(checkout))
    {
        return Err("NEW_OUTPUT_DIR must be outside the source checkout".into());
    }
    if output.starts_with(replay) {
        return Err("NEW_OUTPUT_DIR must be outside the retained replay directory".into());
    }
    fs::create_dir(&output)?;
    fs::create_dir(output.join("public"))?;
    create_protected_dir(&output.join("private"))?;
    Ok(output)
}

fn authority(request: &v3::Request) -> &'static str {
    match request.authority {
        DatasetAuthority::VerifierAgreed { .. } => "verifier_agreed",
        DatasetAuthority::HolderDeclared => "holder_declared",
    }
}

fn initial_status(invocation: &Invocation, inputs: &Inputs, tools: Option<&RealTools>) -> Result<Value> {
    let request = &inputs.verifier.request;
    Ok(json!({
        "schema": STATUS_SCHEMA,
        "mode": invocation.mode,
        "status": "started",
        "accepted": false,
        "stage": null,
        "disposition": null,
        "diagnostic": null,
        "cell": inputs.verifier.cell,
        "replay_directory": inputs.replay.display().to_string(),
        "inputs": inputs.digests,
        "salt": "synthetic-fixed: published test salt, not production blinding",
        "oracle": inputs.expected.oracle,
        "verifier_request": {
            "authority": authority(request),
            "public_request_sha256": sha256(&serde_json::to_vec(request)?),
            "request_digest": v3::request_digest(request).ok().map(|digest| hex(&digest)),
        },
        "proof_bridge": {
            "backend": "exact_v3",
            "dataset_commitment": null,
            "public_statement_sha256": null,
            "private_witness_record": null,
            "reuse_allowed": false,
        },
        "native_replay_observation": null,
        "native_evaluation": null,
        "proof_count": 0,
        "verified_proof_count": 0,
        "harness_checks": [],
        "controls": [],
        "tools": tools.map(|tools| tools.identity.clone()),
        "scope": {
            "statement": "exact V3 evaluation of the retained original query over the retained original Turtle, converted to exact N-Quads",
            "host_harness_checks": "Turtle-to-N-Quads conversion and original-file digests are host checks, not proved by the guest; only a verifier-agreed anchor independently prepared from the originals binds the converted inputs",
            "status_record": "experiment provenance, not a public proof claim",
            "holder_declared": "computation over holder-chosen bytes only; no dataset authenticity or wallet completeness",
            "verifier_agreed": "the input opens the verifier-supplied anchor; that anchor is test-setup trust input, not source authentication",
            "source_credentials": "not authenticated; a separate upcoming method contract; no W3C VC signature support",
            "storage_and_profile": "not evidence about native storage or feature-profile execution; no proof reuse",
            "native_classification": "unchanged; count-only and other non-agreement cells remain non-agreement",
            "assurance": "experimental and not externally audited",
        },
    }))
}

fn reject(status: &mut Value, error: &ReplayError) -> bool {
    status["status"] = json!("rejected");
    status["stage"] = json!(error.stage());
    status["disposition"] = json!(error.disposition());
    status["diagnostic"] = json!(error.to_string());
    false
}

fn fail(status: &mut Value, stage: &str, disposition: &str, diagnostic: &str) -> bool {
    status["status"] = json!("rejected");
    status["stage"] = json!(stage);
    status["disposition"] = json!(disposition);
    status["diagnostic"] = json!(diagnostic);
    false
}

fn record_prepared(output: &Path, status: &mut Value, prepared: &Prepared) -> Result<()> {
    let words = risc0_zkvm::serde::to_vec(prepared.witness())?;
    let guest_input: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let summary = json!({
        "schema": "sparq.engine-replay-proof.private-witness.v1",
        "warning": "sensitive local test artifact; digests of low-entropy data can be matched by guessing; never publish",
        "summary": prepared.private_summary(),
        "guest_input_sha256": sha256(&guest_input),
        "guest_input_bytes": guest_input.len(),
    });
    write_new(
        &output.join(PRIVATE_RECORD),
        &serde_json::to_vec_pretty(&summary)?,
        true,
    )?;
    status["proof_bridge"]["dataset_commitment"] = json!(hex(&prepared.dataset_commitment()));
    status["proof_bridge"]["private_witness_record"] = json!(PRIVATE_RECORD);
    status["native_replay_observation"] = json!({
        "comparison_status": prepared.native_comparison(),
        "label": "native differential observation from record.json; not an independent or normative golden",
    });
    Ok(())
}

fn prove_and_verify(
    output: &Path,
    status: &mut Value,
    inputs: &Inputs,
    prepared: &Prepared,
    native: &v3::Journal,
    tools: &RealTools,
) -> Result<bool> {
    let request = &inputs.verifier.request;
    eprintln!("engine replay proof: real local proof starts");
    let presentation = match prove_with_artifact(prepared.witness(), &tools.r0vm, &tools.guest) {
        Ok(presentation) => presentation,
        Err(error) => return Ok(fail(status, "prover", "ProofFailure", error.0)),
    };
    let InnerReceipt::Succinct(receipt) = &presentation.receipt.inner else {
        return Ok(fail(status, "receipt", "Unexpected", "receipt is not Succinct"));
    };
    status["proof_count"] = json!(1);
    let encoded = serde_json::to_vec(&presentation)?;
    status["receipt"] = json!({
        "kind": "Succinct",
        "seal_bytes": receipt.seal.len().checked_mul(4).ok_or("seal size overflow")?,
        "outer_control_id": receipt.control_id,
        "journal_bytes": presentation.receipt.journal.bytes.len(),
        "presentation_sha256": sha256(&encoded),
        "journal_sha256": sha256(&presentation.receipt.journal.bytes),
    });
    // Persist the actual receipt before verification so failures retain it.
    write_new(&output.join("public/presentation.json"), &encoded, false)?;
    eprintln!("engine replay proof: independent verification starts");
    let mut consumed = MemoryNonces::default();
    let journal = match verify_with_artifact(&presentation, request, &mut consumed, &tools.guest) {
        Ok(journal) => journal,
        Err(error) => return Ok(fail(status, "verifier", "VerificationFailure", error.0)),
    };
    status["verified_proof_count"] = json!(1);
    write_new(
        &output.join("public/journal.json"),
        &serde_json::to_vec_pretty(&journal)?,
        false,
    )?;
    status["proof_bridge"]["public_statement_sha256"] =
        json!(sha256(&presentation.receipt.journal.bytes));
    match inputs.expected.matches(&journal.result) {
        Ok(true) => {}
        Ok(false) => {
            return Ok(fail(
                status,
                "oracle",
                "ContractViolation",
                "verified result differs from the independent expectation",
            ));
        }
        Err(error) => return Ok(reject(status, &error)),
    }
    if journal != *native {
        return Ok(fail(
            status,
            "differential",
            "Unexpected",
            "verified journal differs from native evaluation of the same witness",
        ));
    }
    if journal.dataset_commitment != prepared.dataset_commitment() {
        return Ok(fail(
            status,
            "commitment",
            "Unexpected",
            "verified commitment differs from the prepared witness",
        ));
    }
    status["harness_checks"] = json!([
        "verified_result_matches_independent_expectation",
        "verified_journal_equals_native_evaluation",
        "verified_commitment_equals_prepared_witness",
    ]);
    let controls = match verifier_controls(&presentation, request, &journal, &tools.guest, &mut consumed) {
        Ok(controls) => controls,
        Err(error) => return Ok(fail(status, "controls", "ControlFailure", &error)),
    };
    status["controls"] = json!(controls);
    if file_digest(&tools.r0vm)? != tools.r0vm_sha256 {
        return Ok(fail(status, "tools", "Unexpected", "r0vm changed during the run"));
    }
    status["status"] = json!("proved-and-verified");
    Ok(true)
}

fn run(args: &[PathBuf]) -> Result<(PathBuf, bool)> {
    let invocation = parse(args)?;
    let inputs = load_inputs(&invocation)?;
    let tools = invocation.real.as_ref().map(load_tools).transpose()?;
    let output = fresh_output(&invocation.output, &inputs.replay)?;
    let mut status = initial_status(&invocation, &inputs, tools.as_ref())?;
    write_new(
        &output.join("public/request.json"),
        &serde_json::to_vec_pretty(&inputs.verifier.request)?,
        false,
    )?;
    let originals = Originals {
        record: &inputs.record,
        query: &inputs.query,
        data: &inputs.data,
    };
    let accepted = match replay::prepare(originals, &inputs.verifier, &inputs.holder) {
        Err(error) => reject(&mut status, &error),
        Ok(prepared) => {
            record_prepared(&output, &mut status, &prepared)?;
            match prepared.evaluate_against(&inputs.expected) {
                Err(error) => reject(&mut status, &error),
                Ok(native) => {
                    status["native_evaluation"] = json!({
                        "request_digest": hex(&native.request_digest),
                        "provenance": native.provenance,
                        "result_matches_independent_expectation": true,
                        "label": "native V3 evaluation of the prepared witness; differential evidence, not a proof",
                    });
                    match &tools {
                        None => {
                            status["status"] = json!("prepared");
                            true
                        }
                        Some(tools) => {
                            prove_and_verify(&output, &mut status, &inputs, &prepared, &native, tools)?
                        }
                    }
                }
            }
        }
    };
    status["accepted"] = json!(accepted);
    let path = output.join("status.json");
    write_new(&path, &serde_json::to_vec_pretty(&status)?, false)?;
    Ok((path, accepted))
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() == 1 && args[0] == Path::new("--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok((status, true)) => {
            println!("{}", status.display());
            ExitCode::SUCCESS
        }
        Ok((status, false)) => {
            eprintln!("engine replay proof: typed rejection in {}", status.display());
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("engine replay proof: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::controls::{require_rejection, tampered_result};
    use super::*;
    use sparq_proved_evaluator::Error;
    use sparq_proved_evaluator_model::RowOrder;

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn invocation_requires_an_explicit_mode_and_complete_arguments() {
        for rejected in [
            paths(&[]),
            paths(&["prepare", "r", "h", "v", "e"]),
            paths(&["prove", "r", "h", "v", "e", "o"]),
            paths(&["real", "r", "h", "v", "e", "o"]),
            paths(&["prepare", "r", "h", "v", "e", "g", "p", "x", "o"]),
        ] {
            assert!(matches!(parse(&rejected), Err(error) if error.to_string() == HELP));
        }
        let prepare = parse(&paths(&["prepare", "r", "h", "v", "e", "o"])).unwrap();
        assert!(prepare.real.is_none());
        assert_eq!((prepare.mode, prepare.output), ("prepare", PathBuf::from("o")));
        let real = parse(&paths(&["real", "r", "h", "v", "e", "g", "p", "x", "o"])).unwrap();
        let tools = real.real.unwrap();
        assert_eq!(
            (tools.guest, tools.pin, tools.r0vm, real.output),
            ("g".into(), "p".into(), "x".into(), "o".into())
        );
    }

    #[test]
    fn tampered_results_always_differ_from_their_source() {
        let select = |rows: Vec<Vec<Option<String>>>, width: usize| v3::CanonicalResult::Select {
            variables: (0..width).map(|n| format!("v{n}")).collect(),
            order: RowOrder::Bag,
            rows,
        };
        for result in [
            v3::CanonicalResult::Ask(true),
            v3::CanonicalResult::Ask(false),
            select(vec![], 0),
            select(vec![], 2),
            select(vec![vec![]], 0),
            select(vec![vec![Some("<http://ex/a>".into()), None]], 2),
            v3::CanonicalResult::Graph {
                ntriples: String::new(),
            },
            v3::CanonicalResult::Graph {
                ntriples: "<http://ex/a> <http://ex/p> <http://ex/b> .\n".into(),
            },
        ] {
            assert_ne!(tampered_result(&result), result);
        }
    }

    #[test]
    fn controls_require_the_exact_rejection_class() {
        let receipt = "proof or program identity rejected";
        require_rejection::<()>(Err(Error(receipt)), receipt).unwrap();
        assert!(require_rejection::<()>(Err(Error("real local proof failed")), receipt).is_err());
        assert!(require_rejection(Ok(()), receipt).is_err());
    }
}

// [GPT-6] Genuine local exact-result measurements with independently accepted artifacts.
//! Run fixed synthetic V2 contracts; see `experiments/README.md` for measurement scope.
#[path = "exact_experiment/fixtures.rs"]
mod fixtures;

use fixtures::{Case, Manifest, expected, nonce, witness};
use risc0_zkvm::InnerReceipt;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::v2::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error, Nonces, Presentation};
use sparq_proved_evaluator_model::v2::{Journal, Request, dataset_commitment};
use sparq_proved_evaluator_model::{CanonicalResult, DatasetAuthority};
use std::{
    collections::BTreeSet,
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn StdError>>;
const HELP: &str = "Usage: exact_experiment MANIFEST.json ACCEPTED_GUEST.bin INDEPENDENT_PIN.json LOCAL_R0VM NEW_OUTPUT_DIRECTORY\nRun only validated fixed or generated synthetic profiles. The pin must be accepted independently; never derive it from the supplied artifact. Output must be outside the source checkout and must not exist. Local timings are NONcanonical.";

#[derive(Default)]
struct MemoryNonces(BTreeSet<[u8; 32]>);
impl Nonces for MemoryNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> std::result::Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
        return Err("input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((max + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err("input byte capacity rejected".into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_new(path, &serde_json::to_vec_pretty(value)?)
}

fn command_text(program: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        return Err("provenance command failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn observed_checkout() -> Result<Value> {
    // Runtime observations, not a claim that this executable was built at this HEAD.
    let root = command_text(
        Path::new("git"),
        &[
            "-C",
            env!("CARGO_MANIFEST_DIR"),
            "rev-parse",
            "--show-toplevel",
        ],
    )?;
    let status = command_text(
        Path::new("git"),
        &[
            "-C",
            &root,
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    )?;
    if !status.is_empty() {
        return Err("observed source checkout must be clean".into());
    }
    Ok(json!({
        "head": command_text(Path::new("git"), &["-C", &root, "rev-parse", "HEAD"] )?,
        "tree": command_text(Path::new("git"), &["-C", &root, "rev-parse", "HEAD^{tree}"] )?,
        "clean": true,
        "is_build_attestation": false,
        "root": root,
    }))
}

fn elapsed_ns(start: Instant) -> Result<u64> {
    Ok(start.elapsed().as_nanos().try_into()?)
}

fn require_rejection<T>(
    actual: std::result::Result<T, Error>,
    expected: &'static str,
) -> Result<()> {
    match actual {
        Err(error) if error == Error(expected) => Ok(()),
        Err(error) => {
            Err(format!("wrong rejection class: expected {expected}; received {error}").into())
        }
        Ok(_) => Err(format!("tamper control unexpectedly accepted: {expected}").into()),
    }
}

fn artifact_controls(bytes: &[u8], pin: &ArtifactPin) -> Result<Vec<&'static str>> {
    let mut corrupt = bytes.to_vec();
    *corrupt.first_mut().ok_or("empty artifact")? ^= 1;
    require_rejection(
        AcceptedGuest::from_artifact(corrupt, pin),
        "independent guest artifact digest rejected",
    )?;
    let mut wrong_digest = pin.clone();
    wrong_digest.sha256[0] ^= 1;
    require_rejection(
        AcceptedGuest::from_artifact(bytes.to_vec(), &wrong_digest),
        "independent guest artifact digest rejected",
    )?;
    let mut wrong_image = pin.clone();
    wrong_image.image_id[0] ^= 1;
    require_rejection(
        AcceptedGuest::from_artifact(bytes.to_vec(), &wrong_image),
        "independent guest artifact identity rejected",
    )?;
    Ok(vec![
        "artifact_bytes_digest",
        "independent_pin_digest",
        "independent_pin_image_id",
    ])
}

fn acceptance_controls(
    presentation: &Presentation,
    request: &Request,
    journal: &Journal,
    guest: &AcceptedGuest,
    consumed: &mut MemoryNonces,
) -> Result<Vec<&'static str>> {
    let mut changed = Vec::new();
    let mut query = request.clone();
    query.query.push(' ');
    changed.push(("expected_query_bytes", query));
    let mut challenge = request.clone();
    challenge.nonce[0] ^= 1;
    changed.push(("expected_nonce", challenge));
    let mut authority = request.clone();
    authority.authority = match request.authority {
        DatasetAuthority::VerifierAgreed { .. } => DatasetAuthority::HolderDeclared,
        DatasetAuthority::HolderDeclared => DatasetAuthority::VerifierAgreed {
            commitment: journal.dataset_commitment,
        },
    };
    changed.push(("expected_authority", authority));
    let mut root = request.clone();
    let mut commitment = journal.dataset_commitment;
    commitment[0] ^= 1;
    root.authority = DatasetAuthority::VerifierAgreed { commitment };
    changed.push(("expected_dataset_root", root));
    let mut names = Vec::new();
    for (name, wrong) in changed {
        require_rejection(
            verify_with_artifact(presentation, &wrong, &mut MemoryNonces::default(), guest),
            "independent V2 request binding rejected",
        )?;
        names.push(name);
    }
    require_rejection(
        verify_with_artifact(presentation, request, consumed, guest),
        "challenge already consumed",
    )?;
    names.push("consumed_nonce_replay");

    // Alter the actual authenticated journal result, not an unauthenticated report copy.
    let mut wrong = journal.clone();
    match &mut wrong.result {
        CanonicalResult::Ask(answer) => *answer = !*answer,
        CanonicalResult::Select { rows, .. } => {
            *rows
                .first_mut()
                .and_then(|row| row.first_mut())
                .ok_or("nonempty SELECT control required")? = Some("<http://ex/tampered>".into());
        }
    }
    let mut changed_presentation = presentation.clone();
    changed_presentation.receipt.journal.bytes = risc0_zkvm::serde::to_vec(&wrong)?
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    require_rejection(
        verify_with_artifact(
            &changed_presentation,
            request,
            &mut MemoryNonces::default(),
            guest,
        ),
        "proof or program identity rejected",
    )?;
    names.push("authenticated_returned_result");

    // A holder-declared root is a proved output, not an independent expected root.
    let mut changed_root = journal.clone();
    changed_root.dataset_commitment[0] ^= 1;
    changed_presentation.receipt.journal.bytes = risc0_zkvm::serde::to_vec(&changed_root)?
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    require_rejection(
        verify_with_artifact(
            &changed_presentation,
            request,
            &mut MemoryNonces::default(),
            guest,
        ),
        "proof or program identity rejected",
    )?;
    names.push("authenticated_dataset_root");
    let mut damaged_seal = presentation.clone();
    let InnerReceipt::Succinct(receipt) = &mut damaged_seal.receipt.inner else {
        return Err("non-succinct control receipt".into());
    };
    *receipt.seal.first_mut().ok_or("empty succinct seal")? ^= 1;
    require_rejection(
        verify_with_artifact(&damaged_seal, request, &mut MemoryNonces::default(), guest),
        "proof or program identity rejected",
    )?;
    names.push("succinct_seal_bytes");
    Ok(names)
}

fn contract_identity(case: &Case) -> Result<Value> {
    let input = witness(case, [1; 32])?;
    Ok(json!({
        "backend_contract": "exact_dataset_v2",
        "fixture": case.fixture,
        "authority_mode": case.authority,
        "query": input.request.query,
        "contract": input.request.contract,
        "dialect": input.request.dialect,
        "policy": input.request.policy,
        "dataset_commitment": dataset_commitment(&input.dataset, &input.request.policy)?,
        "nquads_sha256": digest(input.dataset.nquads.as_bytes()),
        "named_graph_catalog": input.dataset.named_graphs,
        "synthetic_salt": input.dataset.salt,
        "expected_result": expected(case),
        "issuer_authentication": "none",
        "scope": "complete committed bounded dataset; local FROM/FROM NAMED snapshots",
        "disclosure": "canonical complete SELECT result or exact ASK boolean, dataset commitment and request binding",
    }))
}

fn run_sample(
    case: &Case,
    challenge: [u8; 32],
    r0vm: &Path,
    guest: &AcceptedGuest,
    output: &Path,
) -> Result<Value> {
    let started = Instant::now();
    let input = witness(case, challenge)?;
    let prepare_ns = elapsed_ns(started)?;
    let started = Instant::now();
    let presentation = prove_with_artifact(&input, r0vm, guest)?;
    let prove_api_ns = elapsed_ns(started)?;
    let mut consumed = MemoryNonces::default();
    let started = Instant::now();
    let journal = verify_with_artifact(&presentation, &input.request, &mut consumed, guest)?;
    let independent_verify_ns = elapsed_ns(started)?;
    if journal.result != expected(case) {
        return Err("genuine result disagrees with fixed golden".into());
    }
    let controls = acceptance_controls(
        &presentation,
        &input.request,
        &journal,
        guest,
        &mut consumed,
    )?;
    let InnerReceipt::Succinct(receipt) = &presentation.receipt.inner else {
        return Err("non-succinct experiment receipt".into());
    };
    let encoded = serde_json::to_vec(&presentation)?;
    fs::create_dir(output)?;
    write_new(&output.join("presentation.json"), &encoded)?;
    write_json(&output.join("expected-request.json"), &input.request)?;
    write_json(&output.join("journal.json"), &journal)?;
    Ok(json!({
        "status": "proved_and_independently_verified",
        "nonce": challenge,
        "request_digest": journal.request_digest,
        "dataset_commitment": journal.dataset_commitment,
        "provenance": journal.provenance,
        "result": journal.result,
        "stages_ns": {
            "fixture_request_prepare": prepare_ns,
            "prove_api_including_internal_verification": prove_api_ns,
            "independent_receipt_verify": independent_verify_ns,
            "guest_compilation": null,
            "witness_execution_only": null,
            "proof_generation_only": null,
            "receipt_compression_only": null,
        },
        "unavailable_stage_reason": "host API exposes inclusive proof call only; build and artifact acceptance occur outside sample timers",
        "peak_rss_bytes": null,
        "peak_rss_reason": "not instrumented across local r0vm child processes",
        "receipt_kind": "Succinct",
        "seal_bytes": receipt.seal.len().checked_mul(4).ok_or("seal size overflow")?,
        "journal_bytes": presentation.receipt.journal.bytes.len(),
        "presentation_json_bytes": encoded.len(),
        "presentation_sha256": digest(&encoded),
        "expected_request_json_bytes": serde_json::to_vec(&input.request)?.len(),
        "canonical_result_json_bytes": serde_json::to_vec(&journal.result)?.len(),
        "private_witness_risc0_words": risc0_zkvm::serde::to_vec(&input)?.len(),
        "outer_control_id": receipt.control_id,
        "typed_acceptance_controls_passed": controls,
    }))
}

fn run(args: &[PathBuf]) -> Result<()> {
    let manifest_bytes = bounded_read(&args[0], 1024 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate()?;
    let artifact = bounded_read(&args[1], 32 * 1024 * 1024)?;
    let pin_bytes = bounded_read(&args[2], 4096)?;
    let pin: ArtifactPin = serde_json::from_slice(&pin_bytes)?;
    let guest = AcceptedGuest::from_artifact(artifact.clone(), &pin)?;
    let artifact_controls = artifact_controls(&artifact, &pin)?;
    let r0vm = args[3].canonicalize()?;
    let r0vm_hash = file_digest(&r0vm)?;
    let r0vm_version = command_text(&r0vm, &["--version"])?;
    if r0vm_version.split_whitespace().last() != Some("3.0.6") {
        return Err("r0vm must report exactly version 3.0.6".into());
    }
    let executable = std::env::current_exe()?;
    let executable_hash = file_digest(&executable)?;
    let checkout = observed_checkout()?;
    let parent = args[4]
        .parent()
        .ok_or("output parent required")?
        .canonicalize()?;
    let output = parent.join(
        args[4]
            .file_name()
            .ok_or("output directory name required")?,
    );
    if output.starts_with(checkout["root"].as_str().ok_or("checkout root")?) {
        return Err("output must be outside the source checkout".into());
    }
    fs::create_dir(&output)?;
    write_new(&output.join("manifest.json"), &manifest_bytes)?;
    write_new(&output.join("accepted-pin.json"), &pin_bytes)?;
    let mut samples = Vec::new();
    for (case_index, case) in manifest.cases.iter().enumerate() {
        let identity = contract_identity(case)?;
        let identity_sha256 = digest(&serde_json::to_vec(&identity)?);
        for ordinal in 0..manifest.warmups + manifest.repetitions {
            eprintln!(
                "exact experiment: case {case_index}, sample {ordinal}: real local proof starts"
            );
            let sample_directory = format!("case-{case_index}-sample-{ordinal}");
            let mut sample = run_sample(
                case,
                nonce(&manifest.run_id, case_index, ordinal),
                &r0vm,
                &guest,
                &output.join(&sample_directory),
            )?;
            sample["warmup"] = json!(ordinal < manifest.warmups);
            sample["ordinal"] = json!(ordinal);
            sample["artifact_directory"] = json!(sample_directory);
            sample["semantic_contract_sha256"] = json!(identity_sha256);
            sample["semantic_contract"] = identity.clone();
            samples.push(sample);
            eprintln!(
                "exact experiment: case {case_index}, sample {ordinal}: verified and controls passed"
            );
        }
    }
    if observed_checkout()? != checkout
        || file_digest(&r0vm)? != r0vm_hash
        || file_digest(&executable)? != executable_hash
    {
        return Err("source/tool/executable changed during experiment".into());
    }
    let report = json!({
        "schema": "sparq.exact-evaluator-experiment.v1",
        "complete": true,
        "canonical_performance": false,
        "manifest_sha256": digest(&manifest_bytes),
        "run_id": manifest.run_id,
        "samples": samples,
        "artifact_acceptance_controls_passed": artifact_controls,
        "provenance": {
            "observed_runtime_checkout": checkout,
            "compiled_in_cargo_lock_sha256": digest(include_bytes!("../../Cargo.lock")),
            "adapter_executable_sha256": executable_hash,
            "accepted_artifact_pin": pin,
            "accepted_pin_file_sha256": digest(&pin_bytes),
            "accepted_artifact_source": null,
            "artifact_source_reason": "artifact pin is caller-supplied trust input; establish source provenance independently",
            "r0vm_sha256": r0vm_hash,
            "r0vm_version_output": r0vm_version,
            "rustc_observed_at_runtime": command_text(Path::new("rustc"), &["--version"] )?,
            "host_os": std::env::consts::OS,
            "host_arch": std::env::consts::ARCH,
            "adapter_debug_assertions": cfg!(debug_assertions),
            "prover": "explicit local ExternalProver; Succinct; dev-mode disabled in compile-time feature and verifier context",
            "hal_observed": null,
            "hal_reason": "adapter does not capture backend target events; hardware is not evidence of selected HAL",
        },
        "cache_conditions": "sequential in one process; existing r0vm/toolchain OS caches retained; no cold-cache claim",
        "unsupported": ["selected-support comparison without matched statement", "arbitrary queries or datasets", "issuer signatures and credential status", "internal stage or RSS measurement", "production nonce generation", "VC suite or optimization ablations"],
        "assurance": "experimental and not externally audited; upstream RISC Zero assurance and outer receipt metadata limits apply",
    });
    write_json(&output.join("report.json"), &report)?;
    println!("{}", output.join("report.json").display());
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() == 1 && args[0] == Path::new("--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.len() != 5 {
        return Err(HELP.into());
    }
    run(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_reject_wrong_error_classes_and_success() {
        assert!(
            require_rejection::<()>(
                Err(Error("real local proof failed")),
                "proof or program identity rejected"
            )
            .is_err()
        );
        assert!(require_rejection(Ok(()), "proof or program identity rejected").is_err());
        require_rejection::<()>(
            Err(Error("proof or program identity rejected")),
            "proof or program identity rejected",
        )
        .unwrap();
    }

    #[test]
    fn identity_excludes_run_nonce_but_includes_authority_and_fixture() {
        let mut identities = BTreeSet::new();
        for fixture in [
            fixtures::Fixture::DefaultSelect,
            fixtures::Fixture::FalseAsk,
            fixtures::Fixture::NamedCatalog,
        ] {
            for authority in [
                fixtures::Authority::VerifierAgreed,
                fixtures::Authority::HolderDeclared,
            ] {
                identities.insert(digest(
                    &serde_json::to_vec(
                        &contract_identity(&Case {
                            fixture,
                            authority,
                            organization: None,
                        })
                        .unwrap(),
                    )
                    .unwrap(),
                ));
            }
        }
        assert_eq!(identities.len(), 6);
    }
}

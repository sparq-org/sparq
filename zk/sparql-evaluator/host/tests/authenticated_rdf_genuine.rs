// [OPUS-5.5] zkp-14.5: genuine V5 receipts over the published W3C vector.
// Rust guideline compliant 2026-02-21
//! Compiled only with `--features authenticated-rdf`.
//!
//! One ignored test is a driver only: set `SPARQ_AUTHRDF_PROOF_JOB` to an
//! explicit job file and `RISC0_SERVER_PATH` to the real local `r0vm`, then run
//! it by name; see `skills/zk-query-proofs/references/authenticated-rdf-guest.md`.
//! A missing job, tool or input fails; nothing is skipped or counted as a run.
//! Six cases are defined: bag SELECT, ASK and CONSTRUCT, each under
//! verifier-agreed and holder-declared authority. The job declares which of them
//! to prove, one to six distinct known IDs, checked before any proof; a subset
//! run claims only its declared cases. Every control reuses those receipts and
//! creates no proof. Evidence goes to the job's new output directory, which is
//! never removed; `summary.json` is written last. Every nonce store here is an
//! in-memory test double, never a production store. Not yet run at the current
//! checkpoint: no V5 receipt exists.
#![cfg(feature = "authenticated-rdf")]

#[path = "support/authenticated_rdf_evidence.rs"]
mod evidence;
#[path = "support/authenticated_rdf.rs"]
mod fixture;

use evidence::{Evidence, GUEST_PACKAGE, hex, pretty, read_input, sha256};
use fixture::{CountingNonces, Expect, STORE_FAILURE};
use risc0_zkvm::{ExitCode, InnerReceipt};
use serde::Deserialize;
use serde_json::{Value, json};
use sparq_proved_evaluator::authenticated_rdf::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error, Presentation, v3 as host_v3};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Provenance, Request};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3};
use std::path::{Path, PathBuf};

/// V2 adds the required `cases` selection; no V1 job was deployed.
const JOB_SCHEMA: &str = "sparq.authrdf-genuine-proof.test-job.v2";
const METADATA_SCHEMA: &str = "sparq.authrdf-genuine-proof.test-metadata.v2";
const SUMMARY_SCHEMA: &str = "sparq.authrdf-genuine-proof.test-summary.v2";
/// Package name of the exact V1–V3 guest, used only for cross-image controls.
const EXACT_GUEST_PACKAGE: &str = "sparq-exact-guest";
/// Domain separator for test-only nonces derived from the public job seed.
const NONCE_DOMAIN: &[u8] = b"sparq:authrdf-genuine-test:nonce:v1\0";
const STORES: &str = "in-memory test doubles only; not durable, not production stores";

/// Read bounds for explicit job inputs.
const MAX_JOB_BYTES: usize = 64 << 10;
const MAX_PIN_BYTES: usize = 4 << 10;
const MAX_GUEST_BYTES: usize = 32 << 20;

/// Exact host errors the controls must produce.
const BINDING: &str = "independent V5 request binding rejected";
const IDENTITY: &str = "proof or program identity rejected";
const NOT_SUCCINCT: &str = "only succinct receipts are accepted";
const REPLAYED: &str = "challenge already consumed";
/// The two correct V3-API outcomes for a genuine V5 receipt under the V5 image.
///
/// Which one occurs depends on whether the pinned SDK decodes the V5 journal
/// words as a V3 journal. Either way the V3 relation is not accepted.
const V3_API_REJECTIONS: [&str; 2] = [
    "V3 journal decoding rejected",
    "independent V3 request binding rejected",
];

/// Explicit prove-mode job; unknown fields reject.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema: String,
    /// Independently approved V5 guest artifact and its deployment-approved pin.
    guest: PathBuf,
    pin: PathBuf,
    /// Independently approved exact V1–V3 guest and pin, for cross-image controls.
    exact_guest: PathBuf,
    exact_pin: PathBuf,
    /// Case IDs to prove, in run order: one to six distinct IDs from `CASES`.
    cases: Vec<String>,
    /// Fresh public synthetic seed, 64 hex characters; not a secret.
    challenge_seed32: String,
    /// Absolute, absent, outside the checkout.
    new_output_directory: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Authority {
    Agreed,
    Holder,
}

impl Authority {
    fn dataset(self, anchor: [u8; 32]) -> DatasetAuthority {
        match self {
            Self::Agreed => DatasetAuthority::VerifierAgreed { commitment: anchor },
            Self::Holder => DatasetAuthority::HolderDeclared,
        }
    }

    fn provenance(self) -> Provenance {
        match self {
            Self::Agreed => Provenance::VerifierAgreedAuthenticated,
            Self::Holder => Provenance::HolderSelectedAuthenticated,
        }
    }
}

struct Case {
    id: &'static str,
    authority: Authority,
    query: &'static str,
    /// Another admitted query of the same form.
    changed_query: &'static str,
    expect: Expect,
}

const CHANGED_SELECT: &str = "SELECT ?name WHERE { ?c <https://schema.org/name> ?name }";
const CHANGED_CONSTRUCT: &str = "CONSTRUCT { ?s <http://ex/other> ?o } \
     WHERE { ?s <https://www.w3.org/ns/credentials/examples#alumniOf> ?o }";

/// Three forms x two authorities; ASK covers both boolean values.
static CASES: [Case; 6] = [
    Case {
        id: "select-bag-verifier-agreed",
        authority: Authority::Agreed,
        query: fixture::SELECT_BAG,
        changed_query: CHANGED_SELECT,
        expect: Expect::Bag,
    },
    Case {
        id: "select-bag-holder-declared",
        authority: Authority::Holder,
        query: fixture::SELECT_BAG,
        changed_query: CHANGED_SELECT,
        expect: Expect::Bag,
    },
    Case {
        id: "ask-true-verifier-agreed",
        authority: Authority::Agreed,
        query: fixture::ASK_ISSUER,
        changed_query: fixture::ASK_OTHER_ISSUER,
        expect: Expect::Ask(true),
    },
    Case {
        id: "ask-false-holder-declared",
        authority: Authority::Holder,
        query: fixture::ASK_OTHER_ISSUER,
        changed_query: fixture::ASK_ISSUER,
        expect: Expect::Ask(false),
    },
    Case {
        id: "construct-verifier-agreed",
        authority: Authority::Agreed,
        query: fixture::CONSTRUCT,
        changed_query: CHANGED_CONSTRUCT,
        expect: Expect::Graph,
    },
    Case {
        id: "construct-holder-declared",
        authority: Authority::Holder,
        query: fixture::CONSTRUCT,
        changed_query: CHANGED_CONSTRUCT,
        expect: Expect::Graph,
    },
];

/// Resolves the declared case IDs, in declared order, before any proof.
///
/// Rejects an empty or longer-than-`CASES` list, an unknown ID and a duplicate.
fn select_cases(declared: &[String]) -> Result<Vec<&'static Case>, String> {
    if declared.is_empty() || declared.len() > CASES.len() {
        return Err(format!(
            "cases must list 1 to {} case IDs, not {}",
            CASES.len(),
            declared.len()
        ));
    }
    let mut selected: Vec<&'static Case> = Vec::with_capacity(declared.len());
    for id in declared {
        let case = CASES
            .iter()
            .find(|case| case.id == id)
            .ok_or_else(|| format!("unknown case `{id}`"))?;
        if selected.iter().any(|chosen| chosen.id == case.id) {
            return Err(format!("duplicate case `{id}`"));
        }
        selected.push(case);
    }
    Ok(selected)
}

fn parse_seed(text: &str) -> Result<[u8; 32], &'static str> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("challenge_seed32 must be 64 hex characters");
    }
    let seed: [u8; 32] = fixture::hex(text);
    if seed == [0; 32] {
        return Err("challenge_seed32 must be nonzero");
    }
    Ok(seed)
}

/// Test-only nonce: SHA-256(domain || seed || u64-be label length || label).
fn nonce_for(seed: &[u8; 32], label: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(NONCE_DOMAIN);
    hash.update(seed);
    hash.update((label.len() as u64).to_be_bytes());
    hash.update(label.as_bytes());
    hash.finalize().into()
}

/// Validated job inputs; both guests come from approved pins, never embedded defaults.
struct Setup {
    job_sha256: String,
    inputs: Value,
    seed: [u8; 32],
    seed_text: String,
    /// The job's declared cases, in declared order; nothing else is proved.
    cases: Vec<&'static Case>,
    r0vm: PathBuf,
    pin: ArtifactPin,
    exact_pin: ArtifactPin,
    guest: AcceptedGuest,
    exact_guest: AcceptedGuest,
    evidence: Evidence,
    /// The verifier's anchor over its own copy of the published credential.
    anchor: [u8; 32],
}

fn load_pin(path: &Path) -> (ArtifactPin, Vec<u8>) {
    let bytes = read_input(path, MAX_PIN_BYTES).expect("pin");
    (serde_json::from_slice(&bytes).expect("pin JSON"), bytes)
}

fn load_guest(path: &Path, pin: &ArtifactPin) -> (AcceptedGuest, Vec<u8>) {
    let bytes = read_input(path, MAX_GUEST_BYTES).expect("guest artifact");
    let guest = AcceptedGuest::from_artifact(bytes.clone(), pin)
        .expect("guest matches its deployment-approved pin");
    assert_eq!(guest.image_id(), pin.image_id);
    (guest, bytes)
}

impl Setup {
    fn load() -> Self {
        assert!(
            std::env::var_os("RISC0_DEV_MODE").is_none(),
            "RISC0_DEV_MODE must be unset"
        );
        let path = PathBuf::from(
            std::env::var_os("SPARQ_AUTHRDF_PROOF_JOB").expect("explicit SPARQ_AUTHRDF_PROOF_JOB"),
        );
        let job_bytes = read_input(&path, MAX_JOB_BYTES).expect("job");
        let job: Job = serde_json::from_slice(&job_bytes).expect("job JSON");
        assert_eq!(job.schema, JOB_SCHEMA);
        // Before any guest load, output directory or proof.
        let cases = select_cases(&job.cases).expect("declared cases");
        let seed = parse_seed(&job.challenge_seed32).expect("challenge seed");
        let r0vm = PathBuf::from(
            std::env::var_os("RISC0_SERVER_PATH")
                .expect("RISC0_SERVER_PATH must name the installed real r0vm 3.0.6"),
        );
        assert!(r0vm.is_absolute(), "RISC0_SERVER_PATH must be absolute");
        let r0vm = r0vm.canonicalize().expect("r0vm");
        assert!(r0vm.is_file(), "r0vm must be an executable file");
        let (pin, pin_bytes) = load_pin(&job.pin);
        let (exact_pin, exact_pin_bytes) = load_pin(&job.exact_pin);
        assert_ne!(pin.sha256, exact_pin.sha256, "V5 and exact artifacts must differ");
        assert_ne!(pin.image_id, exact_pin.image_id, "V5 and exact image IDs must differ");
        let (guest, guest_bytes) = load_guest(&job.guest, &pin);
        let (exact_guest, exact_bytes) = load_guest(&job.exact_guest, &exact_pin);
        let inputs = json!({
            "pin_sha256": sha256(&pin_bytes),
            "guest_sha256": sha256(&guest_bytes),
            "exact_pin_sha256": sha256(&exact_pin_bytes),
            "exact_guest_sha256": sha256(&exact_bytes),
        });
        let evidence =
            Evidence::create(&job.new_output_directory, &pin).expect("new evidence directory");
        Self {
            job_sha256: sha256(&job_bytes),
            inputs,
            seed,
            seed_text: job.challenge_seed32,
            cases,
            r0vm,
            pin,
            exact_pin,
            guest,
            exact_guest,
            evidence,
            anchor: fixture::anchor(fixture::SALT),
        }
    }
}

/// Reads receipt status; call only after checked verification succeeded.
fn genuine_status(presentation: &Presentation) -> Value {
    let InnerReceipt::Succinct(succinct) = &presentation.receipt.inner else {
        panic!("a Succinct receipt is required");
    };
    assert!(!succinct.seal.is_empty(), "nonempty Succinct seal");
    let claim = succinct.claim.as_value().expect("unpruned receipt claim");
    assert_eq!(claim.exit_code, ExitCode::Halted(0));
    json!({
        "inner": "Succinct",
        "seal_words": succinct.seal.len(),
        "exit_code": "Halted(0)",
        "dev_mode": false,
    })
}

/// Verifies with a fresh counting store and returns the outcome and store calls.
fn attempt(
    presentation: &Presentation,
    expected: &Request,
    guest: &AcceptedGuest,
) -> (Result<auth::Journal, Error>, usize) {
    let mut nonces = CountingNonces::default();
    let outcome = verify_with_artifact(presentation, expected, &mut nonces, guest);
    (outcome, nonces.calls)
}

/// Asserts one exact rejection with zero nonce-store calls and records it.
fn refused(
    case: &Case,
    name: &str,
    (outcome, calls): (Result<auth::Journal, Error>, usize),
    error: &'static str,
    passed: &mut Vec<Value>,
) {
    assert_eq!(outcome.err(), Some(Error(error)), "{}: {name}", case.id);
    assert_eq!(calls, 0, "{}: {name} reached the nonce store", case.id);
    passed.push(json!({ "control": name, "error": error, "nonce_calls": 0 }));
}

/// Replaces the result in an already verified receipt's journal.
fn altered_result(presentation: &Presentation, journal: &auth::Journal) -> Presentation {
    let mut changed = journal.clone();
    match &mut changed.result {
        v3::CanonicalResult::Ask(value) => *value = !*value,
        v3::CanonicalResult::Select { rows, .. } => {
            rows.pop();
        }
        v3::CanonicalResult::Graph { ntriples } => ntriples.clear(),
    }
    assert_ne!(changed, *journal);
    let words = risc0_zkvm::serde::to_vec(&changed).expect("journal re-encoding");
    let mut tampered = presentation.clone();
    tampered.receipt.journal.bytes = words.into_iter().flat_map(u32::to_le_bytes).collect();
    tampered
}

/// The same receipt through the V3 API under the V5 image; never accepted.
fn v3_api_control(s: &Setup, case: &Case, original: &Request, presentation: &Presentation) -> Value {
    let v3_request = v3::Request {
        version: v3::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
        query: original.query.clone(),
        authority: original.authority.clone(),
        policy: original.policy.evaluation.clone(),
        nonce: original.nonce,
    };
    // A valid V3 request, so the rejection comes after receipt verification.
    v3::validate_request(&v3_request)
        .unwrap_or_else(|error| panic!("{}: V3 control request: {error}", case.id));
    let mut nonces = CountingNonces::default();
    let outcome = host_v3::verify_with_artifact(presentation, &v3_request, &mut nonces, &s.guest);
    let Err(Error(error)) = outcome else {
        panic!("{}: the V3 API accepted a V5 receipt", case.id);
    };
    assert!(V3_API_REJECTIONS.contains(&error), "{}: v3_api_same_image: {error}", case.id);
    assert_eq!(nonces.calls, 0, "{}: v3_api_same_image reached the store", case.id);
    json!({ "control": "v3_api_same_image", "error": error, "nonce_calls": 0 })
}

/// Verifier-expectation substitutions on one genuine receipt; none reaches a store.
fn binding_controls(
    s: &Setup,
    case: &Case,
    original: &Request,
    presentation: &Presentation,
    journal: &auth::Journal,
) -> Vec<Value> {
    let mut passed = Vec::new();
    let mut substituted: Vec<(&str, Request)> = Vec::new();
    let mut edit = |name: &'static str, change: &dyn Fn(&mut Request)| {
        let mut request = original.clone();
        change(&mut request);
        // Each substitute is itself a valid request, so the rejection is the binding.
        auth::validate_request(&request).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_ne!(&request, original, "{name}");
        substituted.push((name, request));
    };
    edit("changed_query_same_form", &|r| r.query = case.changed_query.into());
    let wrong = nonce_for(&s.seed, &format!("{}/wrong-nonce", case.id));
    edit("wrong_nonce", &|r| r.nonce = wrong);
    edit("policy_rows_tightened", &|r| r.policy.evaluation.dataset.max_rows -= 1);
    // Key-table controls change the verifier's own expectation, never a holder table.
    edit("key_table_widened", &|r| {
        r.policy.authorization.push(fixture::authorized(
            fixture::OTHER_ISSUER,
            fixture::OTHER_VM,
            fixture::W3C_PUBLIC_KEY,
        ));
    });
    edit("key_table_other_issuer", &|r| {
        r.policy.authorization[0].issuer = fixture::OTHER_ISSUER.into();
    });
    edit("key_table_other_method", &|r| {
        r.policy.authorization[0].verification_method = fixture::OTHER_VM.into();
    });
    edit("key_table_other_key", &|r| {
        r.policy.authorization[0].public_key = fixture::hex(fixture::RFC8032_TEST1_PUBLIC_KEY);
    });
    match case.authority {
        Authority::Agreed => {
            edit("authority_agreed_to_holder", &|r| {
                r.authority = DatasetAuthority::HolderDeclared;
            });
            let mut flipped = s.anchor;
            flipped[0] ^= 1;
            edit("wrong_agreed_anchor", &|r| {
                r.authority = DatasetAuthority::VerifierAgreed { commitment: flipped };
            });
            let resalted = fixture::anchor(fixture::OTHER_SALT);
            edit("anchor_under_other_salt", &|r| {
                r.authority = DatasetAuthority::VerifierAgreed { commitment: resalted };
            });
        }
        Authority::Holder => {
            // The journal's own commitment does not upgrade holder selection.
            let published = journal.dataset_commitment;
            edit("authority_holder_to_agreed", &|r| {
                r.authority = DatasetAuthority::VerifierAgreed { commitment: published };
            });
        }
    }
    for (name, request) in &substituted {
        refused(case, name, attempt(presentation, request, &s.guest), BINDING, &mut passed);
    }

    let tampered = altered_result(presentation, journal);
    let outcome = attempt(&tampered, original, &s.guest);
    refused(case, "altered_journal_result", outcome, IDENTITY, &mut passed);
    let mut flipped = presentation.clone();
    flipped.receipt.journal.bytes[0] ^= 1;
    let outcome = attempt(&flipped, original, &s.guest);
    refused(case, "flipped_journal_byte", outcome, IDENTITY, &mut passed);
    let outcome = attempt(presentation, original, &s.exact_guest);
    refused(case, "cross_image_exact_guest", outcome, IDENTITY, &mut passed);
    let fake = fixture::fake_receipt(s.guest.image_id(), presentation.receipt.journal.bytes.clone());
    let outcome = attempt(&fake, original, &s.guest);
    refused(case, "fake_receipt", outcome, NOT_SUCCINCT, &mut passed);
    passed.push(v3_api_control(s, case, original, presentation));
    passed
}

/// Replay and store failure after the one accepted verification.
fn nonce_controls(
    s: &Setup,
    case: &Case,
    request: &Request,
    presentation: &Presentation,
    mut used: CountingNonces,
) -> Vec<Value> {
    let replay = verify_with_artifact(presentation, request, &mut used, &s.guest);
    assert_eq!(replay.err(), Some(Error(REPLAYED)), "{}: replay", case.id);
    assert_eq!(used.calls, 2, "{}: replay store calls", case.id);
    let mut broken = CountingNonces::broken();
    let failure = verify_with_artifact(presentation, request, &mut broken, &s.guest);
    assert_eq!(failure.err(), Some(STORE_FAILURE), "{}: store failure propagates", case.id);
    assert_eq!(broken.calls, 1, "{}: store failure calls", case.id);
    vec![
        json!({ "control": "replay_same_store", "error": REPLAYED, "nonce_calls": 2 }),
        json!({ "control": "broken_store", "error": STORE_FAILURE.0, "nonce_calls": 1 }),
    ]
}

fn run_case(s: &Setup, case: &Case) -> Value {
    let nonce = nonce_for(&s.seed, case.id);
    let witness = fixture::witness(case.query, case.authority.dataset(s.anchor), nonce);
    let request = witness.request.clone();
    let native = auth::evaluate(&witness).expect("native oracle");
    assert_eq!(native.result, case.expect.result(), "{}: hand-defined result", case.id);
    s.evidence.start(case.id, &request);
    eprintln!("authrdf genuine: {} proof starts", case.id);
    let presentation = prove_with_artifact(&witness, &s.r0vm, &s.guest)
        .unwrap_or_else(|error| panic!("{}: genuine proof: {error}", case.id));
    s.evidence.retain(case.id, &presentation);

    let mut nonces = CountingNonces::default();
    let journal = verify_with_artifact(&presentation, &request, &mut nonces, &s.guest)
        .unwrap_or_else(|error| panic!("{}: verification: {error}", case.id));
    assert_eq!(nonces.calls, 1, "{}: nonce consumed once", case.id);
    let status = genuine_status(&presentation);
    assert_eq!(journal, native, "{}: guest vs native model", case.id);
    assert_eq!(journal.result, case.expect.result(), "{}", case.id);
    assert_eq!(journal.provenance, case.authority.provenance(), "{}", case.id);
    assert_eq!(journal.dataset_commitment, s.anchor, "{}", case.id);
    eprintln!("authrdf genuine: {} verified", case.id);

    let mut controls = binding_controls(s, case, &request, &presentation, &journal);
    controls.extend(nonce_controls(s, case, &request, &presentation, nonces));
    let details = json!({
        "authority": format!("{:?}", case.authority),
        "provenance": format!("{:?}", journal.provenance),
        "query": case.query,
        "nonce": hex(&request.nonce),
        "expected": case.expect.result(),
        "journal": journal,
        "status": status,
        "controls": controls,
        "controls_create_proofs": false,
    });
    s.evidence.record(
        case.id,
        "published W3C vc-di-eddsa eddsa-rdfc-2022 vector; public data only",
        &request, &presentation, details)
}

#[test]
#[ignore = "genuine proofs: needs SPARQ_AUTHRDF_PROOF_JOB, a real RISC0_SERVER_PATH and approved guests and pins"]
fn genuine_authrdf_receipts_verify_declared_cases_and_reject_controls() {
    fixture::check_published_vector();
    let s = Setup::load();
    eprintln!("authrdf genuine: evidence directory {}", s.evidence.root().display());
    let declared: Vec<&str> = s.cases.iter().map(|case| case.id).collect();
    eprintln!("authrdf genuine: declared cases {declared:?}");
    let expected: serde_json::Map<String, Value> = s
        .cases
        .iter()
        .map(|case| (case.id.to_owned(), json!(case.expect.result())))
        .collect();
    let metadata = pretty(&json!({
        "schema": METADATA_SCHEMA,
        "status": "started; not a success record",
        "job_sha256": s.job_sha256,
        "inputs": s.inputs,
        "accepted_guest": {
            "package": GUEST_PACKAGE,
            "relation_version": auth::VERSION,
            "pin": s.pin,
            "image_id": s.guest.image_id(),
        },
        "exact_guest": {
            "package": EXACT_GUEST_PACKAGE,
            "use": "cross-image controls only",
            "pin": s.exact_pin,
            "image_id": s.exact_guest.image_id(),
        },
        "r0vm": s.r0vm,
        "challenge_seed32": s.seed_text,
        "declared_cases": declared,
        "all_defined_cases_declared": declared.len() == CASES.len(),
        "nonce_derivation": "SHA-256(domain || seed || u64-be label length || case label)",
        "fixture": {
            "source": "W3C vc-di-eddsa REC 2025-05-15, eddsa-rdfc-2022 examples 7, 9, 10, 12, 13, 15",
            "document_sha256": fixture::W3C_DOCUMENT_SHA256,
            "proof_config_sha256": fixture::W3C_PROOF_SHA256,
            "authorized_key": {
                "issuer": fixture::W3C_ISSUER,
                "verification_method": fixture::W3C_VM,
                "public_key": fixture::W3C_PUBLIC_KEY,
            },
            "salt": hex(&fixture::SALT),
            "agreed_anchor": hex(&s.anchor),
            "expected": expected,
            "expected_source": "hand-defined from the published document and checked against its statements; also compared with the native model",
        },
        "nonce_stores": STORES,
    }));
    let metadata_file = s.evidence.write("metadata.json", &metadata);

    let cases: Vec<Value> = s.cases.iter().map(|case| run_case(&s, case)).collect();
    // A run reports exactly its declared cases: IDs, order, results and receipt count.
    let completed: Vec<&str> = cases
        .iter()
        .map(|record| record["case"].as_str().expect("case ID"))
        .collect();
    assert_eq!(completed, declared, "completed cases differ from the declared cases");
    let completed_expected: serde_json::Map<String, Value> = cases
        .iter()
        .map(|record| {
            let id = record["case"].as_str().expect("case ID").to_owned();
            (id, record["details"]["expected"].clone())
        })
        .collect();
    assert_eq!(completed_expected, expected, "expected results differ from the declared cases");
    let controls: usize = cases
        .iter()
        .map(|record| record["details"]["controls"].as_array().map_or(0, Vec::len))
        .sum();
    let summary = json!({
        "schema": SUMMARY_SCHEMA,
        "job_sha256": s.job_sha256,
        "metadata": metadata_file,
        "guest": GUEST_PACKAGE,
        "declared_cases": declared,
        "completed_cases": completed,
        "all_defined_cases_run": completed.len() == CASES.len(),
        "genuine_receipts": cases.len(),
        "controls_on_existing_receipts": controls,
        "controls_create_proofs": false,
        "cases": cases,
        "nonce_stores": STORES,
        "scope": "published W3C vector only; experimental, not externally audited; no credential status, holder binding, or wallet or world completeness is established; no benchmark",
    });
    s.evidence.write("summary.json", &pretty(&summary));
}

#[test]
fn genuine_job_rejects_unknown_fields_and_invalid_seeds() {
    let job = json!({
        "schema": JOB_SCHEMA,
        "guest": "/abs/authrdf/guest.bin",
        "pin": "/abs/authrdf/pin.json",
        "exact_guest": "/abs/exact/guest.bin",
        "exact_pin": "/abs/exact/pin.json",
        "cases": ["ask-true-verifier-agreed"],
        "challenge_seed32": "01".repeat(32),
        "new_output_directory": "/abs/out",
    });
    let parsed: Job = serde_json::from_value(job.clone()).unwrap();
    assert_eq!(parsed.schema, JOB_SCHEMA);
    assert_eq!(parsed.cases, ["ask-true-verifier-agreed"]);
    let mut extra = job.clone();
    extra["r0vm"] = json!("/abs/r0vm");
    let error = serde_json::from_value::<Job>(extra).err().unwrap().to_string();
    assert!(error.contains("unknown field `r0vm`"), "{error}");
    // Case selection is required; there is no implicit default set.
    let mut missing = job.clone();
    assert!(missing.as_object_mut().unwrap().remove("cases").is_some());
    let error = serde_json::from_value::<Job>(missing).err().unwrap().to_string();
    assert!(error.contains("missing field `cases`"), "{error}");
    let mut scalar = job;
    scalar["cases"] = json!("ask-true-verifier-agreed");
    assert!(serde_json::from_value::<Job>(scalar).is_err(), "cases must be a list");
    for bad in ["00".repeat(32), "0g".repeat(32), "01".into()] {
        assert!(parse_seed(&bad).is_err(), "{bad}");
    }
    let seed = parse_seed(&"01".repeat(32)).unwrap();
    let nonces: std::collections::BTreeSet<[u8; 32]> = CASES
        .iter()
        .flat_map(|case| [case.id.to_owned(), format!("{}/wrong-nonce", case.id)])
        .map(|label| nonce_for(&seed, &label))
        .collect();
    assert_eq!(nonces.len(), 12, "distinct original and wrong nonces");
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|id| (*id).to_owned()).collect()
}

fn selected(declared: &[&str]) -> Vec<&'static str> {
    select_cases(&ids(declared))
        .unwrap_or_else(|error| panic!("{declared:?}: {error}"))
        .iter()
        .map(|case| case.id)
        .collect()
}

#[test]
fn genuine_job_case_selection_is_bounded_known_and_distinct() {
    let all: Vec<&str> = CASES.iter().map(|case| case.id).collect();
    // Complete coverage only by listing all six.
    assert_eq!(selected(&all), all);
    let mut reversed = all.clone();
    reversed.reverse();
    assert_eq!(selected(&reversed), reversed, "declared order is kept");
    for &id in &all {
        assert_eq!(selected(&[id]), [id], "single case");
    }
    let subset = ["construct-holder-declared", "ask-true-verifier-agreed"];
    assert_eq!(selected(&subset), subset, "a subset selects nothing else");

    let mut seven = all.clone();
    seven.push(all[0]);
    let rejected: [(Vec<&str>, &str); 7] = [
        (vec![], "cases must list 1 to 6 case IDs, not 0"),
        (seven, "cases must list 1 to 6 case IDs, not 7"),
        (vec![all[0], all[0]], "duplicate case `select-bag-verifier-agreed`"),
        (vec![all[5], all[2], all[5]], "duplicate case `construct-holder-declared`"),
        (vec!["select-bag"], "unknown case `select-bag`"),
        (vec![all[1], "SELECT-BAG-HOLDER-DECLARED"], "unknown case `SELECT-BAG-HOLDER-DECLARED`"),
        (vec![""], "unknown case ``"),
    ];
    for (declared, error) in rejected {
        let outcome = select_cases(&ids(&declared)).err();
        assert_eq!(outcome.as_deref(), Some(error), "{declared:?}");
    }
}

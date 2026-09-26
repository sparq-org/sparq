// [OPUS-5.5] Ignored genuine vcq receipts: six accepted tuples plus one row-bound rejection.
// Rust guideline compliant 2026-02-21
//! Driver only: set `SPARQ_VCQ_PROOF_JOB` to an explicit job file and run one
//! ignored test by name; see `skills/zk-query-proofs/references/vcq-exact-adapter.md`.
//! A missing job, tool or input fails; it never counts as a proof run. Prove mode
//! writes public evidence under the job's new output directory, which is never
//! removed; `summary.json` is written only after every proof, control and
//! negative check. Verify-only mode proves nothing and writes nothing. Every
//! challenge store here is an in-memory test double, never a production store.
#![cfg(feature = "vcq")]

use risc0_zkvm::{ExitCode, InnerReceipt};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::v3 as host_v3;
use sparq_proved_evaluator::vcq::{self, ReleasedResult, Risc0ExactV3, V3Output, VcqPresentation};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error, Nonces, Presentation};
use sparq_proved_evaluator_model::{Provenance, RowOrder, v3};
use sparq_query_protocol::{
    CapacityBound, Challenge32, ChallengeOutcome, ChallengeStore, ChallengeStoreError, ClaimScope,
    Completeness, DatasetAssembly, Digest32, Enforcer, ErrorCode, EvaluationMode, FailureClass,
    HolderPolicy, Identifier, MethodDescriptor, Obligation, ObligationOutcome, Phase,
    ProtocolError, QueryForm, QueryMethod, QueryProfile, QueryRequirements, RequirementsSpec,
    ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest,
    StoredRequestSpec, VerifiedClaim, encode_method_descriptor, encode_stored_request,
};
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

const JOB_SCHEMA: &str = "sparq.vcq-genuine-proof.test-job.v1";
const METADATA_SCHEMA: &str = "sparq.vcq-genuine-proof.test-metadata.v1";
const SUMMARY_SCHEMA: &str = "sparq.vcq-genuine-proof.test-summary.v1";
/// Domain separator for test-only original challenges derived from the job seed.
const CHALLENGE_DOMAIN: &[u8] = b"sparq:vcq-genuine-test:original-challenge:v1\0";
const STORES: &str = "in-memory test doubles only; not durable, not production stores";

/// Fixed synthetic verifier context, disclosed in the evidence; no ambient clock or credential.
const AUDIENCE: &str = "urn:example:sparq-vcq-genuine-test:verifier";
const OTHER_AUDIENCE: &str = "urn:example:sparq-vcq-genuine-test:other-verifier";
const NOT_BEFORE: u64 = 1_800_000_000;
const NOT_AFTER: u64 = 1_800_000_600;
const NOW: u64 = 1_800_000_100;

/// PUBLIC SYNTHETIC fixture: two subjects share one object, so the bag holds a duplicate row.
const NQUADS: &str = "<http://ex/a> <http://ex/p> \"1\" .\n<http://ex/b> <http://ex/p> \"1\" .\n";
/// Fixed nonzero synthetic salt; public here, never a deployment salt.
const SALT: [u8; 32] = [0x5a; 32];
const ONE: &str = "\"1\"";
/// Hand-written canonical N-Triples of the CONSTRUCT result, sorted, no blank nodes.
const GRAPH: &str = "<http://ex/a> <http://ex/q> \"1\" .\n<http://ex/b> <http://ex/q> \"1\" .\n";
const BAG_QUERY: &str = "SELECT ?o WHERE { ?s <http://ex/p> ?o }";
const CONSTRUCT_QUERY: &str = "CONSTRUCT { ?s <http://ex/q> ?o } WHERE { ?s <http://ex/p> ?o }";

/// Read bounds. A retained `VcqPresentation` JSON writes each receipt byte as a
/// decimal number (up to four characters) within the 16 MiB receipt ceiling.
const MAX_JOB_BYTES: usize = 64 << 10;
const MAX_PIN_BYTES: usize = 4 << 10;
const MAX_GUEST_BYTES: usize = 32 << 20;
const MAX_RETAINED_PRESENTATION_BYTES: usize = 72 << 20;

/// Hand-defined expected result; never derived from the adapter or an evaluator.
#[derive(Clone, Copy)]
enum Expect {
    Bag,
    Ask(bool),
    Graph,
}

impl Expect {
    fn released(self) -> ReleasedResult {
        match self {
            Self::Bag => ReleasedResult::SelectBag {
                variables: vec!["o".to_owned()],
                rows: vec![vec![Some(ONE.to_owned())], vec![Some(ONE.to_owned())]],
            },
            Self::Ask(value) => ReleasedResult::Ask(value),
            Self::Graph => ReleasedResult::Graph {
                ntriples: GRAPH.to_owned(),
            },
        }
    }

    fn journal(self) -> v3::CanonicalResult {
        match self {
            Self::Bag => v3::CanonicalResult::Select {
                variables: vec!["o".to_owned()],
                order: RowOrder::Bag,
                rows: vec![vec![Some(ONE.to_owned())], vec![Some(ONE.to_owned())]],
            },
            Self::Ask(value) => v3::CanonicalResult::Ask(value),
            Self::Graph => v3::CanonicalResult::Graph {
                ntriples: GRAPH.to_owned(),
            },
        }
    }

    fn json(self) -> Value {
        match self {
            Self::Bag => json!({ "select_bag": { "variables": ["o"], "rows": [[ONE], [ONE]] } }),
            Self::Ask(value) => json!({ "ask": value }),
            Self::Graph => json!({ "graph_ntriples": GRAPH }),
        }
    }
}

struct Case {
    id: &'static str,
    contract: ResultContract,
    form: QueryForm,
    authority: ScopeAuthority,
    query: &'static str,
    /// Another admitted query of the same actual form.
    changed_query: &'static str,
    released_rows: u32,
    expect: Expect,
}

const AGREED: ScopeAuthority = ScopeAuthority::VerifierAgreedAnchor;
const HOLDER: ScopeAuthority = ScopeAuthority::HolderDeclared;

/// Three contracts x two authorities; ASK covers both boolean values.
const CASES: [Case; 6] = [
    Case {
        id: "select-bag-verifier-agreed",
        contract: ResultContract::SelectBag,
        form: QueryForm::Select,
        authority: AGREED,
        query: BAG_QUERY,
        changed_query: "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Bag,
    },
    Case {
        id: "select-bag-holder-declared",
        contract: ResultContract::SelectBag,
        form: QueryForm::Select,
        authority: HOLDER,
        query: BAG_QUERY,
        changed_query: "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Bag,
    },
    Case {
        id: "ask-true-verifier-agreed",
        contract: ResultContract::AskBoolean,
        form: QueryForm::Ask,
        authority: AGREED,
        query: "ASK { <http://ex/a> <http://ex/p> ?o }",
        changed_query: "ASK { <http://ex/b> <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Ask(true),
    },
    Case {
        id: "ask-false-holder-declared",
        contract: ResultContract::AskBoolean,
        form: QueryForm::Ask,
        authority: HOLDER,
        query: "ASK { <http://ex/c> <http://ex/p> ?o }",
        changed_query: "ASK { <http://ex/d> <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Ask(false),
    },
    Case {
        id: "construct-verifier-agreed",
        contract: ResultContract::GraphRdfc10,
        form: QueryForm::Construct,
        authority: AGREED,
        query: CONSTRUCT_QUERY,
        changed_query: "CONSTRUCT { ?s <http://ex/r> ?o } WHERE { ?s <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Graph,
    },
    Case {
        id: "construct-holder-declared",
        contract: ResultContract::GraphRdfc10,
        form: QueryForm::Construct,
        authority: HOLDER,
        query: CONSTRUCT_QUERY,
        changed_query: "CONSTRUCT { ?s <http://ex/r> ?o } WHERE { ?s <http://ex/p> ?o }",
        released_rows: 4,
        expect: Expect::Graph,
    },
];

/// Seventh receipt: the two-row bag under a one-row request bound.
const ROW_BOUND: Case = Case {
    id: "select-bag-row-bound",
    contract: ResultContract::SelectBag,
    form: QueryForm::Select,
    authority: AGREED,
    query: BAG_QUERY,
    changed_query: BAG_QUERY,
    released_rows: 1,
    expect: Expect::Bag,
};

/// Explicit driver job. Exactly one mode: prove (`r0vm` and
/// `new_output_directory`) or verify-only (`verify_only_row_bound` alone).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema: String,
    /// Independently approved guest artifact and its deployment-approved pin.
    guest: PathBuf,
    pin: PathBuf,
    /// Fresh public synthetic seed, 64 hex characters; not a secret.
    challenge_seed32: String,
    /// Real local `r0vm` (prove mode only).
    r0vm: Option<PathBuf>,
    /// Absolute, absent, outside the checkout (prove mode only).
    new_output_directory: Option<PathBuf>,
    /// Retained row-bound `presentation.json` (verify-only mode only).
    verify_only_row_bound: Option<PathBuf>,
}

enum Mode {
    Prove { r0vm: PathBuf, output: PathBuf },
    VerifyOnly { row_bound: PathBuf },
}

fn mode(job: &Job) -> Result<Mode, &'static str> {
    match (
        &job.r0vm,
        &job.new_output_directory,
        &job.verify_only_row_bound,
    ) {
        (Some(r0vm), Some(output), None) => Ok(Mode::Prove {
            r0vm: r0vm.clone(),
            output: output.clone(),
        }),
        (None, None, Some(row_bound)) => Ok(Mode::VerifyOnly {
            row_bound: row_bound.clone(),
        }),
        _ => Err(
            "job needs prove mode (r0vm, new_output_directory) or verify-only mode (verify_only_row_bound alone)",
        ),
    }
}

fn parse_seed(text: &str) -> Result<[u8; 32], &'static str> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("challenge_seed32 must be 64 hex characters");
    }
    let mut seed = [0; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * index..2 * index + 2], 16)
            .map_err(|_| "challenge_seed32 must be 64 hex characters")?;
    }
    if seed == [0; 32] {
        return Err("challenge_seed32 must be nonzero");
    }
    Ok(seed)
}

/// Test-only original challenge: SHA-256(domain || seed || u64-be label length || label).
fn challenge_for(seed: &[u8; 32], label: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(CHALLENGE_DOMAIN);
    hash.update(seed);
    hash.update((label.len() as u64).to_be_bytes());
    hash.update(label.as_bytes());
    hash.finalize().into()
}

/// Reads one bounded explicit input: absolute, no `.`/`..`, a regular file, not a symlink.
fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let dotted = path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir));
    if !path.is_absolute() || dotted {
        return Err(format!(
            "{shown}: inputs must be absolute paths without . or .."
        ));
    }
    let kind = fs::symlink_metadata(path)
        .map_err(|error| format!("{shown}: {error}"))?
        .file_type();
    if !kind.is_file() {
        return Err(format!(
            "{shown}: inputs must be regular files, not symlinks or directories"
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("{shown}: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("{shown} exceeds its read bound of {limit} bytes"));
    }
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn pretty(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("typed public JSON")
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

/// Creates the absent evidence directory below a canonicalized parent outside the checkout.
fn fresh_output(requested: &Path) -> Result<PathBuf, String> {
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
    create_dir(&output).map_err(|error| format!("{}: {error}", output.display()))?;
    Ok(output)
}

/// Writes one retained file and records its size and SHA-256.
fn save(dir: &Path, files: &mut Map<String, Value>, name: &str, bytes: &[u8]) {
    write_new(&dir.join(name), bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
    files.insert(
        name.into(),
        json!({ "bytes": bytes.len(), "sha256": sha256(bytes) }),
    );
}

fn id(text: &str) -> Identifier {
    Identifier::new(text).expect("fixed identifier")
}

fn backend(code: &'static str) -> ErrorCode {
    ErrorCode::Backend(code)
}

fn dataset() -> v3::PrivateDataset {
    v3::PrivateDataset {
        nquads: NQUADS.to_owned(),
        named_graphs: vec![],
        salt: SALT,
    }
}

/// In-memory test double with a call counter; never a production store.
#[derive(Default)]
struct Store {
    seen: Mutex<HashSet<[u8; 32]>>,
    calls: AtomicUsize,
    broken: bool,
}

impl ChallengeStore for Store {
    fn consume(&self, challenge: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.broken {
            return Err(ChallengeStoreError::new("test-broken"));
        }
        Ok(
            if self
                .seen
                .lock()
                .expect("test store lock")
                .insert(*challenge.as_bytes())
            {
                ChallengeOutcome::Fresh
            } else {
                ChallengeOutcome::AlreadyConsumed
            },
        )
    }
}

impl Store {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

/// Test-only V3 nonce set, distinct from every adapter challenge store.
#[derive(Default)]
struct TestNonces(BTreeSet<[u8; 32]>);

impl Nonces for TestNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        Ok(self.0.insert(nonce))
    }
}

/// Every stored-request field a control may substitute.
#[derive(Clone)]
struct Spec {
    contract: ResultContract,
    form: QueryForm,
    authority: ScopeAuthority,
    anchor: Option<[u8; 32]>,
    query: String,
    released_rows: u32,
    challenge: [u8; 32],
    audience: &'static str,
    not_before: u64,
    not_after: u64,
}

impl Spec {
    fn original(case: &Case, seed: &[u8; 32], anchor: [u8; 32]) -> Self {
        Self {
            contract: case.contract,
            form: case.form,
            authority: case.authority,
            anchor: (case.authority == AGREED).then_some(anchor),
            query: case.query.to_owned(),
            released_rows: case.released_rows,
            challenge: challenge_for(seed, case.id),
            audience: AUDIENCE,
            not_before: NOT_BEFORE,
            not_after: NOT_AFTER,
        }
    }

    fn stored(&self, descriptor: &MethodDescriptor) -> StoredRequest {
        let requirements = QueryRequirements::new(RequirementsSpec {
            contract: self.contract,
            mode: EvaluationMode::ExactBounded,
            authority: self.authority,
            anchor: self
                .anchor
                .map(|anchor| Digest32::new(anchor).expect("nonzero anchor")),
            source_evidence: Some(SourceEvidence::None),
            status: Some(StatusPolicy::NotRequested),
            holder: Some(HolderPolicy::BearerAccepted),
            assembly: DatasetAssembly::ExactSourceCatalog,
            query_profile: QueryProfile {
                dialect: id(vcq::QUERY_DIALECT),
                fragment: id(vcq::QUERY_FRAGMENT),
            },
            accepted_suites: vec![],
            accepted_mappings: vec![id(vcq::MAPPING_PROFILE)],
            accepted_linking: vec![id(vcq::LINKING_PROFILE)],
            resources: ResourceBounds::new(self.released_rows, vcq::MAX_PRESENTATION_BYTES)
                .expect("request bounds"),
            methods: vec![descriptor.clone()],
        })
        .expect("requirements");
        StoredRequest::new(StoredRequestSpec {
            requirements,
            query: self.query.clone(),
            challenge: Challenge32::new(self.challenge).expect("nonzero challenge"),
            audience: id(self.audience),
            not_before: self.not_before,
            not_after: self.not_after,
            form: self.form,
            base_iri: None,
            describe_policy: None,
        })
        .expect("stored request")
    }
}

/// Validated job inputs shared by both ignored tests.
struct Setup {
    job: Job,
    job_sha256: String,
    inputs: Map<String, Value>,
    seed: [u8; 32],
    pin: ArtifactPin,
    guest: Vec<u8>,
}

impl Setup {
    fn load() -> Self {
        assert!(
            std::env::var_os("RISC0_DEV_MODE").is_none(),
            "RISC0_DEV_MODE must be unset"
        );
        let path = PathBuf::from(
            std::env::var_os("SPARQ_VCQ_PROOF_JOB").expect("explicit SPARQ_VCQ_PROOF_JOB"),
        );
        let job_bytes = read_input(&path, MAX_JOB_BYTES).expect("job");
        let job: Job = serde_json::from_slice(&job_bytes).expect("job JSON");
        assert_eq!(job.schema, JOB_SCHEMA);
        let seed = parse_seed(&job.challenge_seed32).expect("challenge seed");
        let pin_bytes = read_input(&job.pin, MAX_PIN_BYTES).expect("pin");
        let pin: ArtifactPin = serde_json::from_slice(&pin_bytes).expect("pin JSON");
        let guest = read_input(&job.guest, MAX_GUEST_BYTES).expect("guest artifact");
        let mut inputs = Map::new();
        inputs.insert("pin_sha256".into(), json!(sha256(&pin_bytes)));
        inputs.insert("guest_sha256".into(), json!(sha256(&guest)));
        Self {
            job_sha256: sha256(&job_bytes),
            job,
            inputs,
            seed,
            pin,
            guest,
        }
    }

    /// Builds the adapter and an independent checker from the approved pin, never the embedded guest.
    fn verifier(&self, r0vm: Option<PathBuf>) -> Verifier {
        let accept = || {
            AcceptedGuest::from_artifact(self.guest.clone(), &self.pin)
                .expect("guest matches the deployment-approved pin")
        };
        let mut method = Risc0ExactV3::new(&self.pin, accept()).expect("adapter");
        if let Some(r0vm) = r0vm {
            method = method.with_r0vm(r0vm);
        }
        Verifier {
            descriptor: method.descriptor().clone(),
            method,
            checker: accept(),
            seed: self.seed,
            // Agreed anchor: the exact fixture under the default V3 policy.
            anchor: v3::dataset_commitment(&dataset(), &v3::Policy::default())
                .expect("fixture commitment"),
        }
    }
}

struct Verifier {
    method: Risc0ExactV3,
    checker: AcceptedGuest,
    descriptor: MethodDescriptor,
    seed: [u8; 32],
    anchor: [u8; 32],
}

impl Verifier {
    /// Adapter verification of `spec` with a verifier-recomputed admission.
    fn verify(
        &self,
        spec: &Spec,
        audience: &str,
        now: u64,
        presentation: &VcqPresentation,
        store: &Store,
    ) -> Result<VerifiedClaim<V3Output>, ProtocolError> {
        let request = spec.stored(&self.descriptor);
        let admission = self
            .method
            .admit(request.requirements(), &self.descriptor)
            .expect("verifier admission");
        self.method.verify_at(
            &request,
            &id(audience),
            now,
            &admission,
            presentation,
            store,
        )
    }

    /// SDK-checked V3 verification against the independently derived request, with test nonces.
    fn underlying(
        &self,
        spec: &Spec,
        presentation: &VcqPresentation,
    ) -> (Presentation, v3::Journal, Value) {
        let request = spec.stored(&self.descriptor);
        let expected = vcq::expected_v3_request(&request, &self.descriptor, Phase::Verify)
            .expect("derived V3 request");
        let receipt: Presentation =
            serde_json::from_slice(presentation.receipt_bytes()).expect("receipt transport JSON");
        let journal = host_v3::verify_with_artifact(
            &receipt,
            &expected,
            &mut TestNonces::default(),
            &self.checker,
        )
        .expect("SDK-checked Succinct V3 verification");
        let status = genuine_status(&receipt);
        (receipt, journal, status)
    }
}

/// Reads receipt status; call only after checked verification succeeded.
fn genuine_status(receipt: &Presentation) -> Value {
    let InnerReceipt::Succinct(succinct) = &receipt.receipt.inner else {
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
        "journal_sha256": sha256(&receipt.receipt.journal.bytes),
    })
}

/// Prepares and proves one case, then retains its public material before any verification.
fn prove_and_retain(
    v: &Verifier,
    case: &Case,
    spec: &Spec,
    dir: &Path,
) -> (VcqPresentation, Map<String, Value>) {
    let request = spec.stored(&v.descriptor);
    let admission = v
        .method
        .admit(request.requirements(), &v.descriptor)
        .expect("admission");
    let witness = v
        .method
        .prepare(&request, &admission, dataset())
        .expect("prepared witness");
    let marker = json!({ "case": case.id, "progress": "proving", "claims": "none" });
    write_new(&dir.join("started.json"), &pretty(&marker)).expect("progress marker");
    eprintln!("vcq genuine: {} proof starts", case.id);
    let presentation = v
        .method
        .prove(witness)
        .unwrap_or_else(|error| panic!("{}: genuine proof: {error:?}", case.id));
    let expected = vcq::expected_v3_request(&request, &v.descriptor, Phase::Verify)
        .expect("derived V3 request");
    let mut files = Map::new();
    for (name, bytes) in [
        (
            "presentation.json",
            serde_json::to_vec(&presentation).expect("presentation JSON"),
        ),
        ("receipt.json", presentation.receipt_bytes().to_vec()),
        ("v3-request.json", pretty(&expected)),
        (
            "stored-request.local-struct-v1.bin",
            encode_stored_request(&request),
        ),
        (
            "descriptor.local-struct-v1.bin",
            encode_method_descriptor(&v.descriptor),
        ),
        ("expected-result.json", pretty(&case.expect.json())),
    ] {
        save(dir, &mut files, name, &bytes);
    }
    (presentation, files)
}

/// Checks the verified claim against hand-defined expectations and preserved scope.
fn check_claim(
    v: &Verifier,
    case: &Case,
    spec: &Spec,
    claim: &VerifiedClaim<V3Output>,
    receipt: &Presentation,
    journal: &v3::Journal,
) {
    let agreed = case.authority == AGREED;
    let output = claim.result();
    assert_eq!(
        output.result,
        case.expect.released(),
        "{}: hand-defined result",
        case.id
    );
    assert_eq!(
        journal.result,
        case.expect.journal(),
        "{}: verified journal",
        case.id
    );
    let provenance = if agreed {
        Provenance::VerifierAcceptedCommitment
    } else {
        Provenance::HolderDeclaredOnly
    };
    assert_eq!(
        (&output.provenance, &journal.provenance),
        (&provenance, &provenance)
    );
    assert_eq!(
        (output.dataset_commitment, journal.dataset_commitment),
        (v.anchor, v.anchor)
    );
    assert_eq!(output.v3_request_digest, journal.request_digest);
    assert_eq!(claim.descriptor(), &v.descriptor);
    assert_eq!(
        (claim.contract(), claim.mode()),
        (case.contract, EvaluationMode::ExactBounded)
    );
    let statement = vcq::statement_digest(
        &spec.stored(&v.descriptor),
        &v.descriptor,
        &receipt.receipt.journal.bytes,
    );
    assert_eq!(claim.statement_digest().as_bytes(), &statement);
    let anchor = agreed.then(|| Digest32::new(v.anchor).expect("nonzero anchor"));
    assert_eq!(
        *claim.scope(),
        ClaimScope {
            authority: case.authority,
            anchor,
            source_evidence: SourceEvidence::None,
            completeness: Completeness::RelativeToScope,
        }
    );
    let tuple = claim.tuple();
    assert_eq!(
        (tuple.source_evidence, tuple.status, tuple.holder),
        (
            SourceEvidence::None,
            StatusPolicy::NotRequested,
            HolderPolicy::BearerAccepted
        )
    );
    let relation = ObligationOutcome::Established(Enforcer::Relation(id(vcq::RELATION)));
    let anchored = if agreed {
        ObligationOutcome::Established(Enforcer::HostPublic(id(vcq::ANCHOR_CHECK)))
    } else {
        ObligationOutcome::NotEstablished
    };
    for (obligation, expected) in [
        (Obligation::Authenticity, ObligationOutcome::NotEstablished),
        (Obligation::Status, ObligationOutcome::NotEstablished),
        (Obligation::HolderBinding, ObligationOutcome::NotEstablished),
        (Obligation::Mapping, relation.clone()),
        (Obligation::Linking, relation.clone()),
        (Obligation::Query, relation),
        (Obligation::Anchor, anchored),
    ] {
        assert_eq!(
            claim.outcome(obligation),
            &expected,
            "{}: {obligation:?}",
            case.id
        );
    }
}

/// Replaces the journal result of an already verified receipt.
fn altered_result(
    presentation: &VcqPresentation,
    receipt: &Presentation,
    journal: &v3::Journal,
) -> VcqPresentation {
    let mut changed = journal.clone();
    match &mut changed.result {
        v3::CanonicalResult::Ask(value) => *value = !*value,
        // Dropping one of the two duplicate rows.
        v3::CanonicalResult::Select { rows, .. } => {
            rows.pop();
        }
        v3::CanonicalResult::Graph { ntriples } => {
            let first = ntriples.find('\n').map_or(0, |end| end + 1);
            ntriples.truncate(first);
        }
    }
    assert_ne!(changed, *journal);
    let words = risc0_zkvm::serde::to_vec(&changed).expect("journal re-encoding");
    let mut tampered = receipt.clone();
    tampered.receipt.journal.bytes = words.into_iter().flat_map(u32::to_le_bytes).collect();
    VcqPresentation::new(
        *presentation.descriptor_digest(),
        serde_json::to_vec(&tampered).expect("tampered receipt JSON"),
    )
}

/// Reuses one genuine receipt for rejection controls; none reaches the store.
fn binding_controls(
    v: &Verifier,
    case: &Case,
    original: &Spec,
    presentation: &VcqPresentation,
    receipt: &Presentation,
    journal: &v3::Journal,
) -> Vec<Value> {
    // Fresh store: an accepted control would succeed here, never fail as replay.
    let untouched = Store::default();
    let mut passed = Vec::new();
    let mut expect = |name: &str,
                      outcome: Result<VerifiedClaim<V3Output>, ProtocolError>,
                      class: FailureClass,
                      code: ErrorCode| {
        let error = outcome
            .err()
            .unwrap_or_else(|| panic!("{}: control {name} accepted", case.id));
        assert_eq!(
            (error.class(), error.phase(), error.code()),
            (class, Phase::Verify, code),
            "{}: {name}",
            case.id
        );
        passed.push(
            json!({ "control": name, "class": format!("{class:?}"), "code": format!("{code:?}") }),
        );
    };
    let invalid = FailureClass::Invalid;
    let rejected = backend("vcq-proof-rejected");
    let at = |spec: &Spec, audience: &str, now: u64, presented: &VcqPresentation| {
        v.verify(spec, audience, now, presented, &untouched)
    };

    let mut query = original.clone();
    query.query = case.changed_query.to_owned();
    expect(
        "changed_query_same_form",
        at(&query, AUDIENCE, NOW, presentation),
        invalid,
        rejected,
    );
    let mut challenge = original.clone();
    challenge.challenge = challenge_for(&v.seed, &format!("{}/wrong-challenge", case.id));
    expect(
        "wrong_challenge",
        at(&challenge, AUDIENCE, NOW, presentation),
        invalid,
        rejected,
    );
    let mut audience = original.clone();
    audience.audience = OTHER_AUDIENCE;
    let bound = at(&audience, OTHER_AUDIENCE, NOW, presentation);
    expect(
        "changed_stored_audience_matching_supplied",
        bound,
        invalid,
        rejected,
    );
    let mut window = original.clone();
    window.not_before -= 1; // NOW is still inside the window.
    expect(
        "changed_window_containing_now",
        at(&window, AUDIENCE, NOW, presentation),
        invalid,
        rejected,
    );
    let wrong_audience = at(original, OTHER_AUDIENCE, NOW, presentation);
    expect(
        "wrong_verifier_audience",
        wrong_audience,
        invalid,
        backend("vcq-audience-mismatch"),
    );
    let expired = at(original, AUDIENCE, NOT_AFTER, presentation);
    expect("expired", expired, invalid, backend("vcq-request-expired"));
    let early = at(original, AUDIENCE, NOT_BEFORE - 1, presentation);
    expect(
        "not_yet_valid",
        early,
        invalid,
        backend("vcq-request-not-yet-valid"),
    );
    let mut digest = *presentation.descriptor_digest();
    digest[0] ^= 1;
    let foreign = VcqPresentation::new(digest, presentation.receipt_bytes().to_vec());
    let foreign = at(original, AUDIENCE, NOW, &foreign);
    expect(
        "wrong_descriptor_digest",
        foreign,
        invalid,
        backend("vcq-descriptor-digest-mismatch"),
    );
    let tampered = altered_result(presentation, receipt, journal);
    expect(
        "altered_journal_result",
        at(original, AUDIENCE, NOW, &tampered),
        invalid,
        rejected,
    );
    let mut scope = original.clone();
    let name = if case.authority == AGREED {
        scope.authority = HOLDER;
        scope.anchor = None;
        "scope_substitution_agreed_to_holder"
    } else {
        scope.authority = AGREED;
        scope.anchor = Some(v.anchor);
        "scope_substitution_holder_to_agreed"
    };
    expect(
        name,
        at(&scope, AUDIENCE, NOW, presentation),
        invalid,
        rejected,
    );
    if case.authority == AGREED {
        let mut anchor = original.clone();
        let mut wrong = v.anchor;
        wrong[0] ^= 1;
        anchor.anchor = Some(wrong);
        expect(
            "wrong_agreed_anchor",
            at(&anchor, AUDIENCE, NOW, presentation),
            invalid,
            rejected,
        );
    }
    assert_eq!(
        untouched.calls(),
        0,
        "{}: no binding control reached the store",
        case.id
    );
    passed
}

/// Replay, store failure and one concurrent race on memory-only test stores.
fn challenge_controls(
    v: &Verifier,
    case: &Case,
    spec: &Spec,
    presentation: &VcqPresentation,
    used: &Store,
) -> Vec<Value> {
    let replay = v
        .verify(spec, AUDIENCE, NOW, presentation, used)
        .expect_err("second verification on the same store");
    assert_eq!(
        (replay.class(), replay.code(), used.calls()),
        (FailureClass::Invalid, ErrorCode::ChallengeReplayed, 2),
        "{}: replay",
        case.id
    );
    let broken = Store {
        broken: true,
        ..Store::default()
    };
    let failure = v
        .verify(spec, AUDIENCE, NOW, presentation, &broken)
        .expect_err("broken store never accepts");
    assert_eq!(
        (failure.class(), failure.code(), broken.calls()),
        (
            FailureClass::Infrastructure,
            ErrorCode::ChallengeStoreFailure("test-broken"),
            1
        ),
        "{}: store failure",
        case.id
    );
    let shared = Store::default();
    let barrier = Barrier::new(2);
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let racers: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    v.verify(spec, AUDIENCE, NOW, presentation, &shared)
                })
            })
            .collect();
        racers
            .into_iter()
            .map(|racer| racer.join().expect("verifier thread"))
            .collect()
    });
    let accepted: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok())
        .collect();
    let replays = outcomes
        .iter()
        .filter(
            |outcome| matches!(outcome, Err(error) if error.code() == ErrorCode::ChallengeReplayed),
        )
        .count();
    assert_eq!(
        (accepted.len(), replays, shared.calls()),
        (1, 1, 2),
        "{}: race",
        case.id
    );
    assert_eq!(accepted[0].result().result, case.expect.released());
    vec![
        json!({ "control": "replay_same_store", "code": format!("{:?}", ErrorCode::ChallengeReplayed), "store_calls": 2 }),
        json!({ "control": "broken_store", "class": "Infrastructure", "code": "ChallengeStoreFailure(\"test-broken\")", "store_calls": 1 }),
        json!({ "control": "concurrent_verifications", "accepted": 1, "replayed": 1, "store_calls": 2 }),
    ]
}

fn accepted_case(v: &Verifier, case: &Case, output: &Path) -> Value {
    let dir = output.join(case.id);
    create_dir(&dir).expect("new case directory");
    let spec = Spec::original(case, &v.seed, v.anchor);
    let (presentation, mut files) = prove_and_retain(v, case, &spec, &dir);
    let store = Store::default();
    let claim = v
        .verify(&spec, AUDIENCE, NOW, &presentation, &store)
        .unwrap_or_else(|error| panic!("{}: adapter verification: {error:?}", case.id));
    assert_eq!(
        store.calls(),
        1,
        "{}: original challenge consumed once",
        case.id
    );
    let (receipt, journal, genuine) = v.underlying(&spec, &presentation);
    check_claim(v, case, &spec, &claim, &receipt, &journal);
    save(&dir, &mut files, "journal.json", &pretty(&journal));
    let verified = json!({ "case": case.id, "progress": "verified", "protocol_accepted": true, "genuine": genuine });
    write_new(&dir.join("verified.json"), &pretty(&verified)).expect("progress marker");
    eprintln!("vcq genuine: {} verified", case.id);

    let mut controls = binding_controls(v, case, &spec, &presentation, &receipt, &journal);
    controls.extend(challenge_controls(v, case, &spec, &presentation, &store));
    let record = json!({
        "case": case.id,
        "contract": format!("{:?}", case.contract),
        "authority": format!("{:?}", case.authority),
        "query": case.query,
        "released_rows": case.released_rows,
        "challenge": hex(&spec.challenge),
        "expected": case.expect.json(),
        "genuine": genuine,
        "protocol_accepted": true,
        "scope": {
            "source_evidence": "None",
            "status": "NotRequested (NotEstablished)",
            "holder": "BearerAccepted (holder binding NotEstablished)",
            "anchor_established": case.authority == AGREED,
        },
        "controls": controls,
        "files": files,
    });
    write_new(&dir.join("record.json"), &pretty(&record)).expect("case record");
    record
}

/// Asserts the exact post-proof, pre-consumption row-bound rejection of the seventh receipt.
fn check_row_bound(v: &Verifier, presentation: &VcqPresentation) -> (Value, v3::Journal) {
    let spec = Spec::original(&ROW_BOUND, &v.seed, v.anchor);
    let store = Store::default();
    let error = v
        .verify(&spec, AUDIENCE, NOW, presentation, &store)
        .expect_err("the protocol row bound rejects the genuine receipt");
    let bound = ErrorCode::CapacityExceeded(CapacityBound::Backend {
        name: "vcq-released-rows",
        requested: 2,
        ceiling: 1,
    });
    assert_eq!(
        (error.class(), error.phase(), error.code()),
        (FailureClass::Capacity, Phase::Verify, bound)
    );
    assert_eq!(
        store.calls(),
        0,
        "rejected before the original challenge is consumed"
    );
    // Establish the cryptographic acceptance separately, with test nonces only.
    let (_, journal, genuine) = v.underlying(&spec, presentation);
    assert_eq!(
        journal.result,
        ROW_BOUND.expect.journal(),
        "hand-defined duplicate bag"
    );
    assert_eq!(journal.provenance, Provenance::VerifierAcceptedCommitment);
    assert_eq!(journal.dataset_commitment, v.anchor);
    let rejection = json!({
        "class": "Capacity",
        "phase": "Verify",
        "code": format!("{bound:?}"),
        "original_challenge_store_calls": 0,
        "underlying_v3_verified_with_test_nonces": true,
        "genuine": genuine,
    });
    (rejection, journal)
}

#[test]
#[ignore = "genuine proofs: needs a prove-mode SPARQ_VCQ_PROOF_JOB, a real r0vm and an approved guest and pin"]
fn genuine_vcq_receipts_verify_every_tuple_and_reject_controls() {
    let setup = Setup::load();
    let Ok(Mode::Prove { r0vm, output }) = mode(&setup.job) else {
        panic!("this test needs a prove-mode job");
    };
    let r0vm = r0vm.canonicalize().expect("r0vm");
    assert!(r0vm.is_file(), "r0vm must be an executable file");
    let output = fresh_output(&output).expect("new evidence directory");
    eprintln!("vcq genuine: evidence directory {}", output.display());
    let v = setup.verifier(Some(r0vm.clone()));

    let expected: Map<String, Value> = CASES
        .iter()
        .chain([&ROW_BOUND])
        .map(|case| (case.id.to_owned(), case.expect.json()))
        .collect();
    let metadata = json!({
        "schema": METADATA_SCHEMA,
        "status": "started; not a success record",
        "job_sha256": setup.job_sha256,
        "inputs": setup.inputs,
        "accepted_guest": { "pin": setup.pin, "image_id": v.checker.image_id() },
        "descriptor_sha256": hex(&vcq::descriptor_digest(&v.descriptor)),
        "r0vm": r0vm,
        "challenge_seed32": setup.job.challenge_seed32,
        "challenge_derivation": "SHA-256(domain || seed || u64-be label length || case label)",
        "fixture": {
            "nquads_sha256": sha256(NQUADS.as_bytes()),
            "named_graphs": [],
            "salt": hex(&SALT),
            "agreed_anchor": hex(&v.anchor),
            "anchor_policy": "v3::Policy::default()",
            "expected": expected,
            "expected_sha256": sha256(&pretty(&expected)),
            "expected_source": "hand-defined from the fixture; not derived from adapter or evaluator output",
        },
        "verifier": { "audience": AUDIENCE, "not_before": NOT_BEFORE, "not_after": NOT_AFTER, "now": NOW, "clock": "fixed synthetic, not ambient" },
        "challenge_stores": STORES,
    });
    let metadata = pretty(&metadata);
    write_new(&output.join("metadata.json"), &metadata).expect("initial metadata");

    let accepted: Vec<Value> = CASES
        .iter()
        .map(|case| accepted_case(&v, case, &output))
        .collect();

    let dir = output.join(ROW_BOUND.id);
    create_dir(&dir).expect("new case directory");
    let spec = Spec::original(&ROW_BOUND, &v.seed, v.anchor);
    let (presentation, mut files) = prove_and_retain(&v, &ROW_BOUND, &spec, &dir);
    let (rejection, journal) = check_row_bound(&v, &presentation);
    save(&dir, &mut files, "journal.json", &pretty(&journal));
    let row_bound = json!({
        "case": ROW_BOUND.id,
        "released_rows": 1,
        "challenge": hex(&spec.challenge),
        "genuine_receipt": true,
        "protocol_accepted": false,
        "rejection": rejection,
        "verify_only_input": "presentation.json",
        "files": files,
    });
    write_new(&dir.join("record.json"), &pretty(&row_bound)).expect("row-bound record");

    let control_count: usize = accepted
        .iter()
        .map(|record| record["controls"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(accepted.len(), 6);
    let summary = json!({
        "schema": SUMMARY_SCHEMA,
        "job_sha256": setup.job_sha256,
        "metadata_sha256": sha256(&metadata),
        "genuine_receipts": 7,
        "protocol_accepted_receipts": 6,
        "protocol_rejected_genuine_receipts": { "released_row_bound": 1 },
        "controls_on_existing_receipts": control_count,
        "controls_create_proofs": false,
        "cases": accepted,
        "row_bound": row_bound,
        "challenge_stores": STORES,
        "registry_adapter_available": false,
        "scope": "public synthetic fixture; experimental, not externally audited; no source credential, status or holder key is authenticated",
    });
    write_new(&output.join("summary.json"), &pretty(&summary)).expect("final summary");
}

#[test]
#[ignore = "verify-only: needs a SPARQ_VCQ_PROOF_JOB naming a retained row-bound presentation"]
fn retained_row_bound_receipt_rejects_after_proof_checks_before_consumption() {
    let setup = Setup::load();
    let Ok(Mode::VerifyOnly { row_bound }) = mode(&setup.job) else {
        panic!("this test needs a verify-only job");
    };
    let bytes =
        read_input(&row_bound, MAX_RETAINED_PRESENTATION_BYTES).expect("retained presentation");
    let presentation: VcqPresentation =
        serde_json::from_slice(&bytes).expect("retained presentation JSON");
    check_row_bound(&setup.verifier(None), &presentation);
    eprintln!(
        "vcq genuine: verify-only row bound held; no proof created, nothing written ({})",
        sha256(&bytes)
    );
}

#[test]
fn job_modes_are_exclusive_and_unknown_fields_rejected() {
    let base = json!({
        "schema": JOB_SCHEMA,
        "guest": "/abs/guest.bin",
        "pin": "/abs/pin.json",
        "challenge_seed32": "01".repeat(32),
    });
    let job = |extra: Value| {
        let mut fields = base.clone();
        fields
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<Job>(fields)
    };
    let prove = job(json!({ "r0vm": "/abs/r0vm", "new_output_directory": "/abs/out" })).unwrap();
    assert!(matches!(mode(&prove), Ok(Mode::Prove { .. })));
    let verify = job(json!({ "verify_only_row_bound": "/abs/presentation.json" })).unwrap();
    assert!(matches!(mode(&verify), Ok(Mode::VerifyOnly { .. })));
    for mixed in [
        json!({}),
        json!({ "r0vm": "/abs/r0vm" }),
        json!({ "r0vm": "/abs/r0vm", "new_output_directory": "/abs/out", "verify_only_row_bound": "/abs/p" }),
        json!({ "new_output_directory": "/abs/out", "verify_only_row_bound": "/abs/p" }),
    ] {
        assert!(mode(&job(mixed).unwrap()).is_err());
    }
    let error = job(json!({ "output": "/abs/out" }))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("unknown field `output`"), "{error}");
    assert!(parse_seed(&"00".repeat(32)).is_err());
    assert!(parse_seed(&"0g".repeat(32)).is_err());
    assert!(parse_seed("01").is_err());
    let seed = parse_seed(&"01".repeat(32)).unwrap();
    let labels: BTreeSet<[u8; 32]> = CASES
        .iter()
        .chain([&ROW_BOUND])
        .flat_map(|case| [case.id.to_owned(), format!("{}/wrong-challenge", case.id)])
        .map(|label| challenge_for(&seed, &label))
        .collect();
    assert_eq!(labels.len(), 14, "distinct original and wrong challenges");
}

#[test]
fn inputs_and_output_directory_are_guarded() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("sparq-vcq-guard-{}-{nanos}", std::process::id()));
    create_dir(&base).unwrap();
    let base = base.canonicalize().unwrap();
    let file = base.join("pin.json");
    write_new(&file, b"0123456789").unwrap();
    assert_eq!(read_input(&file, 10).unwrap(), b"0123456789");
    assert!(read_input(&file, 9).unwrap_err().contains("read bound"));
    assert!(
        read_input(Path::new("pin.json"), 10)
            .unwrap_err()
            .contains("absolute")
    );
    assert!(
        read_input(&base.join("x/../pin.json"), 10)
            .unwrap_err()
            .contains("absolute")
    );
    assert!(read_input(&base, 10).unwrap_err().contains("regular files"));
    #[cfg(unix)]
    {
        let link = base.join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_input(&link, 10).unwrap_err().contains("regular files"));
    }
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("new-vcq-evidence-guard");
    assert!(
        fresh_output(Path::new("relative"))
            .unwrap_err()
            .contains("absolute")
    );
    assert!(
        fresh_output(&checkout)
            .unwrap_err()
            .contains("source checkout")
    );
    assert!(
        fresh_output(&base.join("missing/evidence"))
            .unwrap_err()
            .contains("output parent")
    );
    assert!(
        fresh_output(&file).is_err(),
        "an existing path is never reused"
    );
    assert!(!checkout.exists());
    let output = fresh_output(&base.join("evidence")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    fs::remove_dir_all(&base).unwrap();
}

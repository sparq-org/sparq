// [OPUS-5.5] zkp-14.6: ignored genuine-receipt driver for the V5 vcq adapter.
// Rust guideline compliant 2026-02-21
//! Compiled only with `--features vcq-authenticated`.
//!
//! One ignored test is a driver only: set `SPARQ_VCQ_AUTHRDF_PROOF_JOB` to an
//! explicit job file and `RISC0_SERVER_PATH` to the real local `r0vm`, then run
//! it by name; see
//! `skills/zk-query-proofs/references/vcq-authenticated-rdf-adapter.md`. A
//! missing job, tool or input fails; nothing is skipped or counted as a run.
//!
//! Seven cases are defined over the published W3C vector: bag SELECT, ASK and
//! CONSTRUCT, each under verifier-agreed and holder-declared authority, plus a
//! bag whose request allows one released row and must be rejected. The job
//! declares which of them to prove, one to seven distinct known IDs, checked
//! before any proof; a subset run claims only its declared cases, and the
//! summary says whether every defined case ran. Every control reuses an
//! existing receipt and creates no proof. Evidence goes to the job's new output
//! directory, which is never removed; `summary.json` is written last. Every
//! challenge store here is an in-memory test double, never a production store.
//! Not yet run at the current checkpoint: no V5 adapter receipt exists.
#![cfg(feature = "vcq-authenticated")]

#[path = "support/authenticated_rdf_evidence.rs"]
mod evidence;
#[path = "support/authenticated_rdf.rs"]
mod fixture;

use evidence::{Evidence, GUEST_PACKAGE, hex, pretty, read_input, sha256};
use fixture::{CountingNonces, Expect};
use risc0_zkvm::{ExitCode, InnerReceipt};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::authenticated_rdf::verify_with_artifact;
use sparq_proved_evaluator::vcq::{self, ReleasedResult, VcqPresentation};
use sparq_proved_evaluator::vcq_authenticated::{
    self as vcq5, AuthenticatedOutput, Risc0AuthenticatedRdfV5,
};
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Presentation};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Policy, Provenance};
use sparq_proved_evaluator_model::v3;
use sparq_query_protocol::{
    CapacityBound, Challenge32, ChallengeOutcome, ChallengeStore, ChallengeStoreError, ClaimScope,
    Completeness, DatasetAssembly, Digest32, Enforcer, ErrorCode, EvaluationMode, FailureClass,
    HolderPolicy, Identifier, MethodDescriptor, Obligation, ObligationOutcome, Phase,
    ProtocolError, QueryForm, QueryMethod, QueryProfile, QueryRequirements, RequirementsSpec,
    ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest,
    StoredRequestSpec, VerifiedClaim, encode_method_descriptor, encode_stored_request,
};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

const JOB_SCHEMA: &str = "sparq.vcq-authrdf-genuine-proof.test-job.v1";
const METADATA_SCHEMA: &str = "sparq.vcq-authrdf-genuine-proof.test-metadata.v1";
const SUMMARY_SCHEMA: &str = "sparq.vcq-authrdf-genuine-proof.test-summary.v1";
/// Package name of the exact V1–V3 guest, used only for the cross-image control.
const EXACT_GUEST_PACKAGE: &str = "sparq-exact-guest";
/// Domain separator for test-only original challenges derived from the job seed.
const CHALLENGE_DOMAIN: &[u8] = b"sparq:vcq-authrdf-genuine-test:original-challenge:v1\0";
const STORES: &str = "in-memory test doubles only; not durable, not production stores";

/// Fixed synthetic verifier context, disclosed in the evidence; no ambient clock.
const AUDIENCE: &str = "urn:example:sparq-vcq-authrdf-genuine-test:verifier";
const OTHER_AUDIENCE: &str = "urn:example:sparq-vcq-authrdf-genuine-test:other-verifier";
const NOT_BEFORE: u64 = 1_800_000_000;
const NOT_AFTER: u64 = 1_800_000_600;
const NOW: u64 = 1_800_000_100;

/// Read bounds for explicit job inputs.
const MAX_JOB_BYTES: usize = 64 << 10;
const MAX_PIN_BYTES: usize = 4 << 10;
const MAX_GUEST_BYTES: usize = 32 << 20;

const CHANGED_SELECT: &str = "SELECT ?name WHERE { ?c <https://schema.org/name> ?name }";
const CHANGED_CONSTRUCT: &str = "CONSTRUCT { ?s <http://ex/other> ?o } \
     WHERE { ?s <https://www.w3.org/ns/credentials/examples#alumniOf> ?o }";

/// Explicit prove-mode job; unknown fields reject.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema: String,
    /// Independently approved V5 guest artifact and its deployment-approved pin.
    guest: PathBuf,
    pin: PathBuf,
    /// Independently approved exact V1–V3 guest and pin, for the cross-image control.
    exact_guest: PathBuf,
    exact_pin: PathBuf,
    /// Case IDs to prove, in run order: one to seven distinct IDs from `CASES`.
    cases: Vec<String>,
    /// Fresh public synthetic seed, 64 hex characters; not a secret.
    challenge_seed32: String,
    /// Absolute, absent, outside the checkout.
    new_output_directory: PathBuf,
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
    /// False only for the row-bound case, which the protocol must reject.
    accepted: bool,
}

const AGREED: ScopeAuthority = ScopeAuthority::VerifierAgreedAnchor;
const HOLDER: ScopeAuthority = ScopeAuthority::HolderDeclared;

/// Three contracts x two authorities (ASK covers both values), plus the row bound.
static CASES: [Case; 7] = [
    Case {
        id: "select-bag-verifier-agreed",
        contract: ResultContract::SelectBag,
        form: QueryForm::Select,
        authority: AGREED,
        query: fixture::SELECT_BAG,
        changed_query: CHANGED_SELECT,
        released_rows: 4,
        expect: Expect::Bag,
        accepted: true,
    },
    Case {
        id: "select-bag-holder-declared",
        contract: ResultContract::SelectBag,
        form: QueryForm::Select,
        authority: HOLDER,
        query: fixture::SELECT_BAG,
        changed_query: CHANGED_SELECT,
        released_rows: 4,
        expect: Expect::Bag,
        accepted: true,
    },
    Case {
        id: "ask-true-verifier-agreed",
        contract: ResultContract::AskBoolean,
        form: QueryForm::Ask,
        authority: AGREED,
        query: fixture::ASK_ISSUER,
        changed_query: fixture::ASK_OTHER_ISSUER,
        released_rows: 4,
        expect: Expect::Ask(true),
        accepted: true,
    },
    Case {
        id: "ask-false-holder-declared",
        contract: ResultContract::AskBoolean,
        form: QueryForm::Ask,
        authority: HOLDER,
        query: fixture::ASK_OTHER_ISSUER,
        changed_query: fixture::ASK_ISSUER,
        released_rows: 4,
        expect: Expect::Ask(false),
        accepted: true,
    },
    Case {
        id: "construct-verifier-agreed",
        contract: ResultContract::GraphRdfc10,
        form: QueryForm::Construct,
        authority: AGREED,
        query: fixture::CONSTRUCT,
        changed_query: CHANGED_CONSTRUCT,
        released_rows: 4,
        expect: Expect::Graph,
        accepted: true,
    },
    Case {
        id: "construct-holder-declared",
        contract: ResultContract::GraphRdfc10,
        form: QueryForm::Construct,
        authority: HOLDER,
        query: fixture::CONSTRUCT,
        changed_query: CHANGED_CONSTRUCT,
        released_rows: 4,
        expect: Expect::Graph,
        accepted: true,
    },
    Case {
        id: "select-bag-row-bound",
        contract: ResultContract::SelectBag,
        form: QueryForm::Select,
        authority: AGREED,
        query: fixture::SELECT_BAG,
        changed_query: CHANGED_SELECT,
        released_rows: 1,
        expect: Expect::Bag,
        accepted: false,
    },
];

/// Hand-defined released result; never derived from the adapter or an evaluator.
fn released(expect: Expect) -> ReleasedResult {
    match expect {
        Expect::Bag => ReleasedResult::SelectBag {
            variables: vec!["name".to_owned()],
            rows: vec![vec![Some(fixture::NAME_CELL.to_owned())]; 2],
        },
        Expect::Ask(value) => ReleasedResult::Ask(value),
        Expect::Graph => ReleasedResult::Graph {
            ntriples: fixture::GRAPH.to_owned(),
        },
    }
}

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

/// Test-only original challenge: SHA-256(domain || seed || u64-be label length || label).
fn challenge_for(seed: &[u8; 32], label: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(CHALLENGE_DOMAIN);
    hash.update(seed);
    hash.update((label.len() as u64).to_be_bytes());
    hash.update(label.as_bytes());
    hash.finalize().into()
}

/// Verifier-owned policy substitutions; each builds its own valid descriptor.
fn replaced_policies() -> Vec<(&'static str, Policy)> {
    let base = fixture::policy();
    let edit = |change: &dyn Fn(&mut Policy)| {
        let mut policy = base.clone();
        change(&mut policy);
        policy
    };
    vec![
        (
            "policy_other_key",
            edit(&|p| {
                p.authorization[0].public_key = fixture::hex(fixture::RFC8032_TEST1_PUBLIC_KEY);
            }),
        ),
        (
            "policy_other_issuer",
            edit(&|p| p.authorization[0].issuer = fixture::OTHER_ISSUER.into()),
        ),
        (
            "policy_other_method",
            edit(&|p| p.authorization[0].verification_method = fixture::OTHER_VM.into()),
        ),
        (
            "policy_table_widened",
            edit(&|p| {
                p.authorization.push(fixture::authorized(
                    fixture::OTHER_ISSUER,
                    fixture::OTHER_VM,
                    fixture::W3C_PUBLIC_KEY,
                ));
            }),
        ),
        (
            "policy_rows_tightened",
            edit(&|p| p.evaluation.dataset.max_rows -= 1),
        ),
    ]
}

fn id(text: &str) -> Identifier {
    Identifier::new(text).expect("fixed identifier")
}

fn backend(code: &'static str) -> ErrorCode {
    ErrorCode::Backend(code)
}

fn credentials() -> auth::PrivateCredentials {
    fixture::credentials(vec![fixture::credential()], fixture::SALT)
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
        let fresh = self
            .seen
            .lock()
            .expect("test store lock")
            .insert(*challenge.as_bytes());
        Ok(if fresh {
            ChallengeOutcome::Fresh
        } else {
            ChallengeOutcome::AlreadyConsumed
        })
    }
}

impl Store {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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
            source_evidence: Some(SourceEvidence::IssuerAuthenticated),
            status: Some(StatusPolicy::NotRequested),
            holder: Some(HolderPolicy::BearerAccepted),
            assembly: DatasetAssembly::UnionDefaultGraph,
            query_profile: QueryProfile {
                dialect: id(vcq5::QUERY_DIALECT),
                fragment: id(vcq5::QUERY_FRAGMENT),
            },
            accepted_suites: vec![id(vcq5::SUITE)],
            accepted_mappings: vec![id(vcq5::MAPPING_PROFILE)],
            accepted_linking: vec![id(vcq5::LINKING_PROFILE)],
            resources: ResourceBounds::new(self.released_rows, vcq5::MAX_PRESENTATION_BYTES)
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

/// Adapter verification with a verifier-recomputed admission for `method`'s own descriptor.
fn verify_with(
    method: &Risc0AuthenticatedRdfV5,
    spec: &Spec,
    audience: &str,
    now: u64,
    presentation: &VcqPresentation,
    store: &Store,
) -> Result<VerifiedClaim<AuthenticatedOutput>, ProtocolError> {
    let descriptor = method.descriptor();
    let request = spec.stored(descriptor);
    let admission = method
        .admit(request.requirements(), descriptor)
        .expect("verifier admission");
    method.verify_at(&request, &id(audience), now, &admission, presentation, store)
}

/// The adapters and independent checker, all from approved pins, never embedded guests.
struct Verifier {
    method: Risc0AuthenticatedRdfV5,
    descriptor: MethodDescriptor,
    /// Same approved V5 guest, for SDK-checked verification with test nonces.
    checker: AcceptedGuest,
    /// The same approved V5 guest under verifier-owned substituted policies.
    replaced: Vec<(&'static str, Risc0AuthenticatedRdfV5)>,
    /// The approved exact guest and pin with the original policy.
    cross_image: Risc0AuthenticatedRdfV5,
}

/// Validated job inputs.
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
    verifier: Verifier,
    evidence: Evidence,
    /// The verifier's anchor over its own copy of the published credential.
    anchor: [u8; 32],
}

fn load_pin(path: &Path) -> (ArtifactPin, Vec<u8>) {
    let bytes = read_input(path, MAX_PIN_BYTES).expect("pin");
    (serde_json::from_slice(&bytes).expect("pin JSON"), bytes)
}

fn accept(bytes: &[u8], pin: &ArtifactPin) -> AcceptedGuest {
    let guest = AcceptedGuest::from_artifact(bytes.to_vec(), pin)
        .expect("guest matches its deployment-approved pin");
    assert_eq!(guest.image_id(), pin.image_id);
    guest
}

impl Setup {
    fn load() -> Self {
        assert!(
            std::env::var_os("RISC0_DEV_MODE").is_none(),
            "RISC0_DEV_MODE must be unset"
        );
        let path = PathBuf::from(
            std::env::var_os("SPARQ_VCQ_AUTHRDF_PROOF_JOB")
                .expect("explicit SPARQ_VCQ_AUTHRDF_PROOF_JOB"),
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
        let guest_bytes = read_input(&job.guest, MAX_GUEST_BYTES).expect("V5 guest artifact");
        let exact_bytes = read_input(&job.exact_guest, MAX_GUEST_BYTES).expect("exact guest");
        let method = Risc0AuthenticatedRdfV5::new(&pin, accept(&guest_bytes, &pin), fixture::policy())
            .expect("adapter from the approved V5 pin")
            .with_r0vm(r0vm.clone());
        let replaced = replaced_policies()
            .into_iter()
            .map(|(name, policy)| {
                let adapter = Risc0AuthenticatedRdfV5::new(&pin, accept(&guest_bytes, &pin), policy)
                    .expect("adapter under a substituted verifier policy");
                (name, adapter)
            })
            .collect();
        let cross_image = Risc0AuthenticatedRdfV5::new(
            &exact_pin,
            accept(&exact_bytes, &exact_pin),
            fixture::policy(),
        )
        .expect("adapter pinned to the exact guest");
        let verifier = Verifier {
            descriptor: method.descriptor().clone(),
            method,
            checker: accept(&guest_bytes, &pin),
            replaced,
            cross_image,
        };
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
            verifier,
            evidence,
            anchor: fixture::anchor(fixture::SALT),
        }
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

/// SDK-checked V5 verification against the independently derived request, with test nonces.
fn underlying(v: &Verifier, receipt: &Presentation, expected: &auth::Request) -> auth::Journal {
    let mut nonces = CountingNonces::default();
    let journal = verify_with_artifact(receipt, expected, &mut nonces, &v.checker)
        .expect("SDK-checked Succinct V5 verification");
    assert_eq!(nonces.calls, 1, "test nonce set consulted once");
    journal
}

/// One proved case: its public material and the native oracle's journal.
struct Proved {
    presentation: VcqPresentation,
    receipt: Presentation,
    expected: auth::Request,
    native: auth::Journal,
    files: Map<String, Value>,
}

/// Prepares and proves one case, retaining public material before any verification.
fn prove_and_retain(s: &Setup, case: &Case, spec: &Spec) -> Proved {
    let v = &s.verifier;
    let request = spec.stored(&v.descriptor);
    let admission = v
        .method
        .admit(request.requirements(), &v.descriptor)
        .expect("admission");
    let expected = vcq5::expected_v5_request(&request, &v.descriptor, v.method.policy(), Phase::Verify)
        .expect("derived V5 request");
    // Native model only, to compare with the verified journal; never a proof.
    let native = auth::evaluate(&auth::Witness {
        request: expected.clone(),
        dataset: credentials(),
    })
    .expect("native oracle");
    assert_eq!(native.result, case.expect.result(), "{}: hand-defined result", case.id);
    let witness = v
        .method
        .prepare(&request, &admission, credentials())
        .expect("prepared witness");
    s.evidence.start(case.id, &expected);
    eprintln!("vcq authrdf genuine: {} proof starts", case.id);
    let presentation = v
        .method
        .prove(witness)
        .unwrap_or_else(|error| panic!("{}: genuine proof: {error:?}", case.id));
    let receipt: Presentation =
        serde_json::from_slice(presentation.receipt_bytes()).expect("receipt transport JSON");
    s.evidence.retain(case.id, &receipt);
    let mut files = Map::new();
    for (name, bytes) in [
        (
            "vcq-presentation.json",
            serde_json::to_vec(&presentation).expect("presentation JSON"),
        ),
        (
            "stored-request.local-struct-v1.bin",
            encode_stored_request(&request),
        ),
        (
            "descriptor.local-struct-v1.bin",
            encode_method_descriptor(&v.descriptor),
        ),
        ("expected-result.json", pretty(&case.expect.result())),
    ] {
        let written = s.evidence.write(&format!("{}/{name}", case.id), &bytes);
        files.insert(name.into(), written);
    }
    Proved {
        presentation,
        receipt,
        expected,
        native,
        files,
    }
}

/// Checks the verified claim against hand-defined expectations and preserved scope.
fn check_claim(
    s: &Setup,
    case: &Case,
    spec: &Spec,
    claim: &VerifiedClaim<AuthenticatedOutput>,
    proved: &Proved,
    journal: &auth::Journal,
) {
    let v = &s.verifier;
    let agreed = case.authority == AGREED;
    let output = claim.result();
    assert_eq!(output.result, released(case.expect), "{}: hand-defined result", case.id);
    assert_eq!(journal.result, case.expect.result(), "{}: verified journal", case.id);
    let provenance = if agreed {
        Provenance::VerifierAgreedAuthenticated
    } else {
        Provenance::HolderSelectedAuthenticated
    };
    assert_eq!((output.provenance, journal.provenance), (provenance, provenance));
    // Both authorities evaluate the same credential and salt.
    assert_eq!(
        (output.dataset_commitment, journal.dataset_commitment),
        (s.anchor, s.anchor)
    );
    let request_digest = auth::request_digest(&proved.expected).expect("V5 request digest");
    assert_eq!(
        (output.v5_request_digest, journal.request_digest),
        (request_digest, request_digest)
    );
    assert_eq!(claim.descriptor(), &v.descriptor);
    assert_eq!(
        (claim.contract(), claim.mode()),
        (case.contract, EvaluationMode::ExactBounded)
    );
    let statement = vcq5::statement_digest(
        &spec.stored(&v.descriptor),
        &v.descriptor,
        &proved.receipt.receipt.journal.bytes,
    );
    assert_eq!(claim.statement_digest().as_bytes(), &statement);
    let anchor = agreed.then(|| Digest32::new(s.anchor).expect("nonzero anchor"));
    assert_eq!(
        *claim.scope(),
        ClaimScope {
            authority: case.authority,
            anchor,
            source_evidence: SourceEvidence::IssuerAuthenticated,
            completeness: Completeness::RelativeToScope,
        }
    );
    let tuple = claim.tuple();
    assert_eq!(tuple.suite, Some(id(vcq5::SUITE)));
    assert_eq!(
        (tuple.status, tuple.holder),
        (StatusPolicy::NotRequested, HolderPolicy::BearerAccepted)
    );
    let relation = ObligationOutcome::Established(Enforcer::Relation(id(vcq5::RELATION)));
    let anchored = if agreed {
        ObligationOutcome::Established(Enforcer::HostPublic(id(vcq5::ANCHOR_CHECK)))
    } else {
        ObligationOutcome::NotEstablished
    };
    for (obligation, expected) in [
        (Obligation::Authenticity, relation.clone()),
        (Obligation::Mapping, relation.clone()),
        (Obligation::Linking, relation.clone()),
        (Obligation::Query, relation),
        (Obligation::Anchor, anchored),
        (Obligation::Status, ObligationOutcome::NotEstablished),
        (Obligation::HolderBinding, ObligationOutcome::NotEstablished),
    ] {
        assert_eq!(claim.outcome(obligation), &expected, "{}: {obligation:?}", case.id);
    }
}

/// Replaces the result in an already verified receipt's journal.
fn altered_result(proved: &Proved, journal: &auth::Journal) -> VcqPresentation {
    let mut changed = journal.clone();
    match &mut changed.result {
        v3::CanonicalResult::Ask(value) => *value = !*value,
        // Dropping one of the two duplicate rows.
        v3::CanonicalResult::Select { rows, .. } => {
            rows.pop();
        }
        v3::CanonicalResult::Graph { ntriples } => ntriples.clear(),
    }
    assert_ne!(changed, *journal);
    let words = risc0_zkvm::serde::to_vec(&changed).expect("journal re-encoding");
    let mut tampered = proved.receipt.clone();
    tampered.receipt.journal.bytes = words.into_iter().flat_map(u32::to_le_bytes).collect();
    wrap(*proved.presentation.descriptor_digest(), &tampered)
}

fn wrap(descriptor_digest: [u8; 32], receipt: &Presentation) -> VcqPresentation {
    VcqPresentation::new(
        descriptor_digest,
        serde_json::to_vec(receipt).expect("receipt JSON"),
    )
}

/// Reuses one genuine receipt for rejection controls; none reaches a store.
fn binding_controls(
    s: &Setup,
    case: &Case,
    original: &Spec,
    proved: &Proved,
    journal: &auth::Journal,
) -> Vec<Value> {
    let v = &s.verifier;
    // Fresh store: an accepted control would succeed here, never fail as replay.
    let untouched = Store::default();
    let mut passed = Vec::new();
    let mut expect = |name: &str,
                      outcome: Result<VerifiedClaim<AuthenticatedOutput>, ProtocolError>,
                      code: ErrorCode| {
        let error = outcome
            .err()
            .unwrap_or_else(|| panic!("{}: control {name} accepted", case.id));
        assert_eq!(
            (error.class(), error.phase(), error.code()),
            (FailureClass::Invalid, Phase::Verify, code),
            "{}: {name}",
            case.id
        );
        passed.push(json!({ "control": name, "class": "Invalid", "code": format!("{code:?}") }));
    };
    let presentation = &proved.presentation;
    let rejected = backend("vcq-proof-rejected");
    let at = |method: &Risc0AuthenticatedRdfV5,
              spec: &Spec,
              audience: &str,
              now: u64,
              shown: &VcqPresentation| {
        verify_with(method, spec, audience, now, shown, &untouched)
    };
    let own = &v.method;

    let mut query = original.clone();
    query.query = case.changed_query.to_owned();
    expect("changed_query_same_form", at(own, &query, AUDIENCE, NOW, presentation), rejected);
    let mut challenge = original.clone();
    challenge.challenge = challenge_for(&s.seed, &format!("{}/wrong-challenge", case.id));
    expect("wrong_challenge", at(own, &challenge, AUDIENCE, NOW, presentation), rejected);
    let mut audience = original.clone();
    audience.audience = OTHER_AUDIENCE;
    let bound = at(own, &audience, OTHER_AUDIENCE, NOW, presentation);
    expect("changed_stored_audience_matching_supplied", bound, rejected);
    let mut window = original.clone();
    window.not_before -= 1; // NOW is still inside the window.
    expect("changed_window_containing_now", at(own, &window, AUDIENCE, NOW, presentation), rejected);
    let wrong_audience = at(own, original, OTHER_AUDIENCE, NOW, presentation);
    expect("wrong_verifier_audience", wrong_audience, backend("vcq-audience-mismatch"));
    let expired = at(own, original, AUDIENCE, NOT_AFTER, presentation);
    expect("expired", expired, backend("vcq-request-expired"));
    let early = at(own, original, AUDIENCE, NOT_BEFORE - 1, presentation);
    expect("not_yet_valid", early, backend("vcq-request-not-yet-valid"));
    let mut digest = *presentation.descriptor_digest();
    digest[0] ^= 1;
    let foreign = VcqPresentation::new(digest, presentation.receipt_bytes().to_vec());
    let foreign = at(own, original, AUDIENCE, NOW, &foreign);
    expect("wrong_descriptor_digest", foreign, backend("vcq-descriptor-digest-mismatch"));

    // Receipt tampering.
    let tampered = altered_result(proved, journal);
    expect("altered_journal_result", at(own, original, AUDIENCE, NOW, &tampered), rejected);
    let mut flipped = proved.receipt.clone();
    flipped.receipt.journal.bytes[0] ^= 1;
    let flipped = wrap(*presentation.descriptor_digest(), &flipped);
    expect("flipped_journal_byte", at(own, original, AUDIENCE, NOW, &flipped), rejected);
    let fake = fixture::fake_receipt(
        v.checker.image_id(),
        proved.receipt.receipt.journal.bytes.clone(),
    );
    let fake = wrap(*presentation.descriptor_digest(), &fake);
    expect("fake_receipt", at(own, original, AUDIENCE, NOW, &fake), rejected);

    // Scope substitution.
    let mut scope = original.clone();
    let name = if case.authority == AGREED {
        scope.authority = HOLDER;
        scope.anchor = None;
        "scope_substitution_agreed_to_holder"
    } else {
        scope.authority = AGREED;
        scope.anchor = Some(s.anchor);
        "scope_substitution_holder_to_agreed"
    };
    expect(name, at(own, &scope, AUDIENCE, NOW, presentation), rejected);
    if case.authority == AGREED {
        let mut anchor = original.clone();
        let mut wrong = s.anchor;
        wrong[0] ^= 1;
        anchor.anchor = Some(wrong);
        expect("wrong_agreed_anchor", at(own, &anchor, AUDIENCE, NOW, presentation), rejected);
        let mut resalted = original.clone();
        resalted.anchor = Some(fixture::anchor(fixture::OTHER_SALT));
        expect("anchor_under_other_salt", at(own, &resalted, AUDIENCE, NOW, presentation), rejected);
    }

    // Verifier-owned policy or key replacement: first as presented, then with
    // the presentation's descriptor digest spliced to the substitute's.
    for (policy, method) in &v.replaced {
        let as_presented = at(method, original, AUDIENCE, NOW, presentation);
        expect(
            &format!("{policy}_descriptor_digest"),
            as_presented,
            backend("vcq-descriptor-digest-mismatch"),
        );
        let spliced = VcqPresentation::new(
            vcq::descriptor_digest(method.descriptor()),
            presentation.receipt_bytes().to_vec(),
        );
        let spliced = at(method, original, AUDIENCE, NOW, &spliced);
        expect(&format!("{policy}_spliced"), spliced, rejected);
    }
    // Another approved image: the exact guest's pin with the original policy.
    let cross = &v.cross_image;
    let spliced = VcqPresentation::new(
        vcq::descriptor_digest(cross.descriptor()),
        presentation.receipt_bytes().to_vec(),
    );
    let spliced = at(cross, original, AUDIENCE, NOW, &spliced);
    expect("cross_image_exact_guest_spliced", spliced, rejected);
    assert_eq!(untouched.calls(), 0, "{}: no binding control reached the store", case.id);
    passed
}

/// Replay, store failure and one concurrent race on memory-only test stores.
fn challenge_controls(
    s: &Setup,
    case: &Case,
    spec: &Spec,
    presentation: &VcqPresentation,
    used: &Store,
) -> Vec<Value> {
    let own = &s.verifier.method;
    let replay = verify_with(own, spec, AUDIENCE, NOW, presentation, used)
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
    let failure = verify_with(own, spec, AUDIENCE, NOW, presentation, &broken)
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
                    verify_with(own, spec, AUDIENCE, NOW, presentation, &shared)
                })
            })
            .collect();
        racers
            .into_iter()
            .map(|racer| racer.join().expect("verifier thread"))
            .collect()
    });
    let accepted: Vec<_> = outcomes.iter().filter_map(|outcome| outcome.as_ref().ok()).collect();
    let replays = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Err(error) if error.code() == ErrorCode::ChallengeReplayed))
        .count();
    assert_eq!(
        (accepted.len(), replays, shared.calls()),
        (1, 1, 2),
        "{}: race",
        case.id
    );
    assert_eq!(accepted[0].result().result, released(case.expect));
    vec![
        json!({ "control": "replay_same_store", "code": "ChallengeReplayed", "store_calls": 2 }),
        json!({ "control": "broken_store", "class": "Infrastructure", "code": "ChallengeStoreFailure(\"test-broken\")", "store_calls": 1 }),
        json!({ "control": "concurrent_verifications", "accepted": 1, "replayed": 1, "store_calls": 2 }),
    ]
}

/// A protocol-accepted case: one verification, claim checks, then controls.
fn accepted_details(s: &Setup, case: &Case, spec: &Spec, proved: &Proved) -> Value {
    let store = Store::default();
    let claim = verify_with(&s.verifier.method, spec, AUDIENCE, NOW, &proved.presentation, &store)
        .unwrap_or_else(|error| panic!("{}: adapter verification: {error:?}", case.id));
    assert_eq!(store.calls(), 1, "{}: original challenge consumed once", case.id);
    let journal = underlying(&s.verifier, &proved.receipt, &proved.expected);
    let status = genuine_status(&proved.receipt);
    assert_eq!(journal, proved.native, "{}: guest vs native model", case.id);
    check_claim(s, case, spec, &claim, proved, &journal);
    eprintln!("vcq authrdf genuine: {} verified", case.id);
    let mut controls = binding_controls(s, case, spec, proved, &journal);
    controls.extend(challenge_controls(s, case, spec, &proved.presentation, &store));
    json!({
        "protocol_accepted": true,
        "journal": journal,
        "status": status,
        "scope": {
            "source_evidence": "IssuerAuthenticated (suite, mapping, linking and query by the V5 relation)",
            "status": "NotRequested (NotEstablished)",
            "holder": "BearerAccepted (holder binding NotEstablished)",
            "anchor_established": case.authority == AGREED,
            "completeness": "relative to the verifier-agreed commitment or to the holder's selection only",
        },
        "controls": controls,
    })
}

/// The row-bound case: a genuine receipt the protocol must reject before consumption.
fn row_bound_details(s: &Setup, case: &Case, spec: &Spec, proved: &Proved) -> Value {
    let store = Store::default();
    let error = verify_with(&s.verifier.method, spec, AUDIENCE, NOW, &proved.presentation, &store)
        .expect_err("the protocol row bound rejects the genuine receipt");
    let bound = ErrorCode::CapacityExceeded(CapacityBound::Backend {
        name: "vcq-released-rows",
        requested: 2,
        ceiling: u64::from(case.released_rows),
    });
    assert_eq!(
        (error.class(), error.phase(), error.code()),
        (FailureClass::Capacity, Phase::Verify, bound),
        "{}",
        case.id
    );
    assert_eq!(store.calls(), 0, "rejected before the original challenge is consumed");
    // Establish cryptographic acceptance separately, with test nonces only.
    let journal = underlying(&s.verifier, &proved.receipt, &proved.expected);
    let status = genuine_status(&proved.receipt);
    assert_eq!(journal, proved.native, "{}: guest vs native model", case.id);
    assert_eq!(journal.result, case.expect.result(), "hand-defined duplicate bag");
    assert_eq!(journal.provenance, Provenance::VerifierAgreedAuthenticated);
    assert_eq!(journal.dataset_commitment, s.anchor);
    json!({
        "protocol_accepted": false,
        "journal": journal,
        "status": status,
        "rejection": {
            "class": "Capacity",
            "phase": "Verify",
            "code": format!("{bound:?}"),
            "original_challenge_store_calls": 0,
            "underlying_v5_verified_with_test_nonces": true,
        },
    })
}

fn run_case(s: &Setup, case: &Case) -> Value {
    let spec = Spec::original(case, &s.seed, s.anchor);
    let proved = prove_and_retain(s, case, &spec);
    let outcome = if case.accepted {
        accepted_details(s, case, &spec, &proved)
    } else {
        row_bound_details(s, case, &spec, &proved)
    };
    let details = json!({
        "contract": format!("{:?}", case.contract),
        "authority": format!("{:?}", case.authority),
        "query": case.query,
        "released_rows": case.released_rows,
        "challenge": hex(&spec.challenge),
        "expected": case.expect.result(),
        "outcome": outcome,
        "files": proved.files,
        "controls_create_proofs": false,
    });
    s.evidence.record(case.id, &proved.expected, &proved.receipt, details)
}

#[test]
#[ignore = "genuine proofs: needs SPARQ_VCQ_AUTHRDF_PROOF_JOB, a real RISC0_SERVER_PATH and approved guests and pins"]
fn genuine_vcq_authrdf_receipts_verify_declared_cases_and_reject_controls() {
    fixture::check_published_vector();
    let s = Setup::load();
    eprintln!("vcq authrdf genuine: evidence directory {}", s.evidence.root().display());
    let declared: Vec<&str> = s.cases.iter().map(|case| case.id).collect();
    eprintln!("vcq authrdf genuine: declared cases {declared:?}");
    let expected: Map<String, Value> = s
        .cases
        .iter()
        .map(|case| (case.id.to_owned(), json!(case.expect.result())))
        .collect();
    let policy = s.verifier.method.policy();
    let metadata = pretty(&json!({
        "schema": METADATA_SCHEMA,
        "status": "started; not a success record",
        "job_sha256": s.job_sha256,
        "inputs": s.inputs,
        "accepted_guest": {
            "package": GUEST_PACKAGE,
            "relation_version": auth::VERSION,
            "pin": s.pin,
            "image_id": s.verifier.checker.image_id(),
        },
        "exact_guest": {
            "package": EXACT_GUEST_PACKAGE,
            "use": "cross-image control only",
            "pin": s.exact_pin,
        },
        "descriptor_sha256": hex(&vcq::descriptor_digest(&s.verifier.descriptor)),
        "parameter_digest": hex(&vcq5::parameter_digest(policy).expect("valid policy")),
        "policy_digest": hex(&vcq5::policy_digest(policy).expect("valid policy")),
        "verifier_policy": policy,
        "replaced_policies": s.verifier.replaced.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        "r0vm": s.r0vm,
        "challenge_seed32": s.seed_text,
        "challenge_derivation": "SHA-256(domain || seed || u64-be label length || case label)",
        "declared_cases": declared,
        "all_defined_cases_declared": declared.len() == CASES.len(),
        "fixture": {
            "source": "W3C vc-di-eddsa REC 2025-05-15, eddsa-rdfc-2022 examples 7, 9, 10, 12, 13, 15",
            "document_sha256": fixture::W3C_DOCUMENT_SHA256,
            "proof_config_sha256": fixture::W3C_PROOF_SHA256,
            "salt": hex(&fixture::SALT),
            "agreed_anchor": hex(&s.anchor),
            "expected": expected,
            "expected_source": "hand-defined from the published document and checked against its statements; also compared with the native model",
        },
        "verifier": { "audience": AUDIENCE, "not_before": NOT_BEFORE, "not_after": NOT_AFTER, "now": NOW, "clock": "fixed synthetic, not ambient" },
        "challenge_stores": STORES,
    }));
    let metadata_file = s.evidence.write("metadata.json", &metadata);

    let records: Vec<Value> = s.cases.iter().map(|case| run_case(&s, case)).collect();
    // A run reports exactly its declared cases: IDs, order, results and receipt count.
    let completed: Vec<&str> = records
        .iter()
        .map(|record| record["case"].as_str().expect("case ID"))
        .collect();
    assert_eq!(completed, declared, "completed cases differ from the declared cases");
    let completed_expected: Map<String, Value> = records
        .iter()
        .map(|record| {
            let id = record["case"].as_str().expect("case ID").to_owned();
            (id, record["details"]["expected"].clone())
        })
        .collect();
    assert_eq!(completed_expected, expected, "expected results differ from the declared cases");
    let accepted = s.cases.iter().filter(|case| case.accepted).count();
    let controls: usize = records
        .iter()
        .map(|record| {
            record["details"]["outcome"]["controls"]
                .as_array()
                .map_or(0, Vec::len)
        })
        .sum();
    let summary = json!({
        "schema": SUMMARY_SCHEMA,
        "job_sha256": s.job_sha256,
        "metadata": metadata_file,
        "guest": GUEST_PACKAGE,
        "declared_cases": declared,
        "completed_cases": completed,
        "all_defined_cases_run": completed.len() == CASES.len(),
        "genuine_receipts": records.len(),
        "protocol_accepted_receipts": accepted,
        "protocol_rejected_genuine_receipts": { "released_row_bound": records.len() - accepted },
        "controls_on_existing_receipts": controls,
        "controls_create_proofs": false,
        "cases": records,
        "challenge_stores": STORES,
        "registry_adapter_availability": "not tested; this test reads no registry entry",
        "scope": "published W3C vector only; experimental, not externally audited; no credential status, holder binding, JSON-LD processing, or wallet or world completeness is established; no benchmark",
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
    let challenges: BTreeSet<[u8; 32]> = CASES
        .iter()
        .flat_map(|case| [case.id.to_owned(), format!("{}/wrong-challenge", case.id)])
        .map(|label| challenge_for(&seed, &label))
        .collect();
    assert_eq!(challenges.len(), 14, "distinct original and wrong challenges");
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
    // Complete coverage only by listing all seven.
    assert_eq!(selected(&all), all);
    let mut reversed = all.clone();
    reversed.reverse();
    assert_eq!(selected(&reversed), reversed, "declared order is kept");
    for &id in &all {
        assert_eq!(selected(&[id]), [id], "single case");
    }
    let subset = ["construct-holder-declared", "select-bag-row-bound"];
    assert_eq!(selected(&subset), subset, "a subset selects nothing else");

    let mut eight = all.clone();
    eight.push(all[0]);
    let rejected: [(Vec<&str>, &str); 6] = [
        (vec![], "cases must list 1 to 7 case IDs, not 0"),
        (eight, "cases must list 1 to 7 case IDs, not 8"),
        (vec![all[0], all[0]], "duplicate case `select-bag-verifier-agreed`"),
        (vec![all[6], all[2], all[6]], "duplicate case `select-bag-row-bound`"),
        (vec!["select-bag"], "unknown case `select-bag`"),
        (vec![""], "unknown case ``"),
    ];
    for (declared, error) in rejected {
        let outcome = select_cases(&ids(&declared)).err();
        assert_eq!(outcome.as_deref(), Some(error), "{declared:?}");
    }
    // Exactly one defined case is a protocol rejection, and it has its own ID.
    let rejections: Vec<&str> = CASES
        .iter()
        .filter(|case| !case.accepted)
        .map(|case| case.id)
        .collect();
    assert_eq!(rejections, ["select-bag-row-bound"]);
}

#[test]
fn substituted_verifier_policies_are_valid_and_distinct() {
    // Native only: each substitute must itself be a valid verifier policy, so a
    // genuine-run rejection comes from the binding, not from policy validation.
    let mut digests = BTreeSet::new();
    assert!(digests.insert(vcq5::parameter_digest(&fixture::policy()).unwrap()));
    for (name, policy) in replaced_policies() {
        let digest = vcq5::parameter_digest(&policy)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert!(digests.insert(digest), "{name}: parameter digest unchanged");
    }
    assert_eq!(digests.len(), 6);
}

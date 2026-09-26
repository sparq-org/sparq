// [OPUS-5.5] Native vcq adapter gates. Unit doubles only: no proof is created,
// and the only receipt used is a FAKE one that must be rejected.
#![cfg(feature = "vcq")]

use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::vcq::{self, Risc0ExactV3, VcqPresentation};
use sparq_proved_evaluator::{
    AcceptedGuest, ArtifactPin, Presentation, embedded_artifact, embedded_pin, method_id,
};
use sparq_proved_evaluator_model::v3;
use sparq_query_protocol::{
    ArtifactIdentity, CapacityBound, Challenge32, ChallengeConsumption, ChallengeOutcome,
    ChallengeOwner, ChallengeStore, ChallengeStoreError, DatasetAssembly, Enforcer, ErrorCode,
    EvaluationMode, FailureClass, HolderPolicy, Identifier, MethodDescriptor, Obligation, Phase,
    ProtocolError, QueryForm, QueryMethod, QueryProfile, QueryRequirements, RequirementsSpec,
    ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest,
    StoredRequestSpec, encode_method_descriptor, encode_stored_request,
};
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const AUDIENCE: &str = "urn:example:verifier";
const NOT_BEFORE: u64 = 1_800_000_000;
const NOT_AFTER: u64 = 1_800_000_600;
const NOW: u64 = 1_800_000_100;
/// Public synthetic fixture: two subjects share one object, so the bag has a duplicate row.
const BAG_NQUADS: &str = "<http://ex/a> <http://ex/p> \"1\" .\n<http://ex/b> <http://ex/p> \"1\" .\n";
const BAG_QUERY: &str = "SELECT ?o WHERE { ?s <http://ex/p> ?o }";

fn id(text: &str) -> Identifier {
    Identifier::new(text).unwrap()
}

fn adapter() -> &'static Risc0ExactV3 {
    static ADAPTER: OnceLock<Risc0ExactV3> = OnceLock::new();
    ADAPTER.get_or_init(|| {
        let pin = embedded_pin();
        let guest = AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &pin)
            .expect("embedded guest matches its own pin");
        Risc0ExactV3::new(&pin, guest).expect("adapter")
    })
}

fn dataset() -> v3::PrivateDataset {
    v3::PrivateDataset {
        nquads: BAG_NQUADS.to_owned(),
        named_graphs: vec![],
        salt: [0x5a; 32],
    }
}

/// In-memory test double; never a production store.
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
        Ok(if self.seen.lock().unwrap().insert(*challenge.as_bytes()) {
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

#[derive(Clone)]
struct Spec {
    contract: ResultContract,
    form: QueryForm,
    query: String,
    authority: ScopeAuthority,
    anchor: Option<[u8; 32]>,
    source_evidence: SourceEvidence,
    suites: Vec<&'static str>,
    status: StatusPolicy,
    holder: HolderPolicy,
    mapping: &'static str,
    dialect: &'static str,
    rows: u32,
    bytes: u32,
    methods: Vec<MethodDescriptor>,
    challenge: [u8; 32],
    audience: &'static str,
    not_before: u64,
    not_after: u64,
    base_iri: Option<&'static str>,
    describe_policy: Option<&'static str>,
}

impl Spec {
    fn bag() -> Self {
        Self {
            contract: ResultContract::SelectBag,
            form: QueryForm::Select,
            query: BAG_QUERY.to_owned(),
            authority: ScopeAuthority::HolderDeclared,
            anchor: None,
            source_evidence: SourceEvidence::None,
            suites: vec![],
            status: StatusPolicy::NotRequested,
            holder: HolderPolicy::BearerAccepted,
            mapping: vcq::MAPPING_PROFILE,
            dialect: vcq::QUERY_DIALECT,
            rows: 4,
            bytes: vcq::MAX_PRESENTATION_BYTES,
            methods: vec![adapter().descriptor().clone()],
            challenge: [0x11; 32],
            audience: AUDIENCE,
            not_before: NOT_BEFORE,
            not_after: NOT_AFTER,
            base_iri: None,
            describe_policy: None,
        }
    }

    fn with(mut self, contract: ResultContract, form: QueryForm, query: &str) -> Self {
        self.contract = contract;
        self.form = form;
        self.query = query.to_owned();
        self
    }

    fn requirements(&self) -> QueryRequirements {
        QueryRequirements::new(RequirementsSpec {
            contract: self.contract,
            mode: EvaluationMode::ExactBounded,
            authority: self.authority,
            anchor: self.anchor.map(|anchor| sparq_query_protocol::Digest32::new(anchor).unwrap()),
            source_evidence: Some(self.source_evidence),
            status: Some(self.status),
            holder: Some(self.holder),
            assembly: DatasetAssembly::ExactSourceCatalog,
            query_profile: QueryProfile {
                dialect: id(self.dialect),
                fragment: id(vcq::QUERY_FRAGMENT),
            },
            accepted_suites: self.suites.iter().map(|suite| id(suite)).collect(),
            accepted_mappings: vec![id(self.mapping)],
            accepted_linking: vec![id(vcq::LINKING_PROFILE)],
            resources: ResourceBounds::new(self.rows, self.bytes).unwrap(),
            methods: self.methods.clone(),
        })
        .unwrap()
    }

    fn stored(&self) -> StoredRequest {
        StoredRequest::new(StoredRequestSpec {
            requirements: self.requirements(),
            query: self.query.clone(),
            challenge: Challenge32::new(self.challenge).unwrap(),
            audience: id(self.audience),
            not_before: self.not_before,
            not_after: self.not_after,
            form: self.form,
            base_iri: self
                .base_iri
                .map(|base| sparq_query_protocol::BaseIri::new(base).unwrap()),
            describe_policy: self.describe_policy.map(id),
        })
        .unwrap()
    }
}

fn backend(code: &'static str) -> ErrorCode {
    ErrorCode::Backend(code)
}

fn placeholder() -> VcqPresentation {
    VcqPresentation::new(
        vcq::descriptor_digest(adapter().descriptor()),
        b"not a receipt".to_vec(),
    )
}

fn verify(
    request: &StoredRequest,
    presentation: &VcqPresentation,
    store: &Store,
) -> Result<(), ProtocolError> {
    let admission = adapter().admit(request.requirements(), adapter().descriptor())?;
    adapter()
        .verify_at(request, &id(AUDIENCE), NOW, &admission, presentation, store)
        .map(|_| ())
}

#[test]
fn descriptor_and_capabilities_pin_the_exact_v3_method() {
    let descriptor = adapter().descriptor();
    assert_eq!(descriptor.method().as_str(), vcq::METHOD_ID);
    assert_eq!(descriptor.version(), 3);
    assert_eq!(descriptor.parameter_set().as_str(), vcq::PARAMETER_SET);
    assert_eq!(descriptor.parameter_digest().as_bytes(), &vcq::parameter_digest());
    let artifact: [u8; 32] = Sha256::digest(embedded_artifact()).into();
    assert_eq!(
        *descriptor.artifact(),
        ArtifactIdentity::ZkvmGuest {
            artifact_digest: sparq_query_protocol::Digest32::new(artifact).unwrap(),
            image_id: sparq_query_protocol::Digest32::new(vcq::image_id_bytes(method_id()))
                .unwrap(),
        }
    );
    assert_eq!(descriptor.backend().as_str(), vcq::BACKEND_PIN);
    assert_eq!(vcq::descriptor(&embedded_pin()).unwrap(), *descriptor);

    let capabilities = adapter().capabilities();
    assert!(capabilities.is_executable());
    assert_eq!(capabilities.challenge().owner, ChallengeOwner::Method);
    assert_eq!(
        capabilities.challenge().consumption,
        ChallengeConsumption::ConsumeOnSuccess
    );
    let tuples = capabilities.tuples();
    assert_eq!(tuples.len(), 6);
    let mut seen = BTreeSet::new();
    for tuple in tuples {
        assert!(matches!(
            tuple.contract,
            ResultContract::SelectBag | ResultContract::AskBoolean | ResultContract::GraphRdfc10
        ));
        assert_eq!(tuple.mode, EvaluationMode::ExactBounded);
        assert_eq!(tuple.source_evidence, SourceEvidence::None);
        assert_eq!(tuple.status, StatusPolicy::NotRequested);
        assert_eq!(tuple.holder, HolderPolicy::BearerAccepted);
        assert_eq!(tuple.assembly, DatasetAssembly::ExactSourceCatalog);
        assert_eq!(tuple.suite, None);
        for obligation in [Obligation::Authenticity, Obligation::Status, Obligation::HolderBinding] {
            assert_eq!(*tuple.enforcement.route(obligation), Enforcer::Absent);
        }
        let anchored = tuple.authority == ScopeAuthority::VerifierAgreedAnchor;
        assert_eq!(
            matches!(tuple.enforcement.anchor, Enforcer::HostPublic(_)),
            anchored
        );
        seen.insert(format!("{:?}/{:?}", tuple.contract, tuple.authority));
    }
    assert_eq!(seen.len(), 6, "whole, distinct contract/authority tuples");
}

#[test]
fn parameter_digest_follows_the_documented_encoding() {
    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-exact:parameters:v3\0");
    // Hand-copied V3 defaults: version, dataset, then canonicalization bounds.
    for value in [3_u32, 65_536, 256, 4_096, 16, 32_768, 1_048_576, 1_048_576, 64, 1_048_576] {
        hash.update(value.to_be_bytes());
    }
    hash.update([0x01]);
    hash.update(8_192_u64.to_be_bytes());
    let expected: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq::parameter_digest(), expected);
}

#[test]
fn nonce_and_statement_follow_the_documented_composition() {
    let request = Spec::bag().stored();
    let descriptor = adapter().descriptor();
    let request_digest: [u8; 32] = Sha256::digest(encode_stored_request(&request)).into();
    let descriptor_digest: [u8; 32] = Sha256::digest(encode_method_descriptor(descriptor)).into();
    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-exact:v3-nonce:local-struct-v1\0");
    hash.update(request_digest);
    hash.update(descriptor_digest);
    let nonce: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq::derive_nonce(&request, descriptor), nonce);
    assert_eq!(vcq::derive_nonce(&request, descriptor), nonce, "deterministic");
    assert_ne!(&nonce, request.challenge().as_bytes(), "never the original challenge");

    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-exact:statement:v3\0");
    hash.update(request_digest);
    hash.update(descriptor_digest);
    hash.update(Sha256::digest(b"journal"));
    let statement: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq::statement_digest(&request, descriptor, b"journal"), statement);

    let expected = vcq::expected_v3_request(&request, descriptor, Phase::Verify).unwrap();
    assert_eq!(expected.nonce, nonce);
    assert_eq!(expected.query, BAG_QUERY);
    assert_eq!(expected.policy, v3::Policy::default());
    assert_eq!(expected.authority, sparq_proved_evaluator_model::DatasetAuthority::HolderDeclared);
    v3::validate_request(&expected).unwrap();
}

#[test]
fn every_stored_request_field_changes_the_nonce() {
    let base = Spec::bag();
    let other_pin = ArtifactPin {
        sha256: [1; 32],
        image_id: [1; 8],
    };
    let other = vcq::descriptor(&other_pin).unwrap();
    let mut cases: Vec<(&'static str, Spec)> = vec![("base", base.clone())];
    let mut push = |name: &'static str, change: &dyn Fn(&mut Spec)| {
        let mut spec = base.clone();
        change(&mut spec);
        cases.push((name, spec));
    };
    push("query", &|s| s.query = "SELECT ?s WHERE { ?s <http://ex/p> ?o }".into());
    push("contract", &|s| {
        *s = s.clone().with(ResultContract::AskBoolean, QueryForm::Ask, "ASK { ?s ?p ?o }");
    });
    push("authority", &|s| {
        s.authority = ScopeAuthority::VerifierAgreedAnchor;
        s.anchor = Some([5; 32]);
    });
    push("method-list", &|s| s.methods.push(other.clone()));
    push("audience", &|s| s.audience = "urn:example:other-verifier");
    push("not-before", &|s| s.not_before += 1);
    push("not-after", &|s| s.not_after += 1);
    push("challenge", &|s| s.challenge[0] ^= 1);
    push("rows", &|s| s.rows += 1);
    push("bytes", &|s| s.bytes -= 1);
    let mut anchored = base.clone();
    anchored.authority = ScopeAuthority::VerifierAgreedAnchor;
    anchored.anchor = Some([6; 32]);
    cases.push(("anchor-value", anchored));

    let descriptor = adapter().descriptor();
    let mut nonces = BTreeSet::new();
    let mut named = Vec::new();
    for (name, spec) in &cases {
        if nonces.insert(vcq::derive_nonce(&spec.stored(), descriptor)) {
            named.push(*name);
        }
    }
    assert_eq!(nonces.len(), cases.len(), "distinct nonces only for: {named:?}");
    let stored = base.stored();
    assert_ne!(
        vcq::derive_nonce(&stored, descriptor),
        vcq::derive_nonce(&stored, &other),
        "selected descriptor"
    );
}

#[test]
fn unsupported_policies_and_profiles_fail_admission() {
    let admit = |spec: Spec| {
        adapter()
            .admit(spec.stored().requirements(), adapter().descriptor())
            .unwrap_err()
    };
    let tuple = |spec: Spec| {
        let error = admit(spec);
        assert_eq!(error.class(), FailureClass::Unsupported);
        error.code()
    };
    let mut spec = Spec::bag();
    spec.source_evidence = SourceEvidence::IssuerAuthenticated;
    spec.suites = vec!["urn:example:suite"];
    assert_eq!(tuple(spec), ErrorCode::TupleUnsupported);
    let mut spec = Spec::bag();
    spec.status = StatusPolicy::Required;
    assert_eq!(tuple(spec), ErrorCode::TupleUnsupported);
    let mut spec = Spec::bag();
    spec.holder = HolderPolicy::Required;
    assert_eq!(tuple(spec), ErrorCode::TupleUnsupported);
    let mut spec = Spec::bag();
    spec.mapping = "urn:example:map:other";
    assert_eq!(tuple(spec), ErrorCode::TupleUnsupported);
    let mut spec = Spec::bag();
    spec.dialect = "urn:example:dialect:other";
    assert_eq!(tuple(spec), ErrorCode::QueryProfileUnsupported);
    for contract in [ResultContract::SelectSequence, ResultContract::SelectDistinctSet] {
        let spec = Spec::bag().with(contract, QueryForm::Select, BAG_QUERY);
        assert_eq!(tuple(spec), ErrorCode::TupleUnsupported);
    }
    let mut spec = Spec::bag();
    spec.rows = 4_097;
    assert_eq!(admit(spec).class(), FailureClass::Capacity);
}

#[test]
fn actual_query_form_is_checked_before_any_challenge_use() {
    let describe = "DESCRIBE ?s WHERE { ?s ?p ?o }";
    let long = format!("ASK {{ }}{}", " ".repeat(8_192));
    let mut described = Spec::bag().with(ResultContract::GraphRdfc10, QueryForm::Describe, describe);
    described.describe_policy = Some("urn:example:describe:outgoing");
    let mut based = Spec::bag();
    based.base_iri = Some("http://ex/base/");
    let cases: Vec<(Spec, FailureClass, ErrorCode)> = vec![
        (
            Spec::bag().with(ResultContract::GraphRdfc10, QueryForm::Construct, describe),
            FailureClass::Invalid,
            backend("vcq-query-form-mismatch"),
        ),
        (
            Spec::bag().with(ResultContract::SelectBag, QueryForm::Select, "ASK { ?s ?p ?o }"),
            FailureClass::Invalid,
            backend("vcq-query-form-mismatch"),
        ),
        (
            Spec::bag().with(ResultContract::AskBoolean, QueryForm::Ask, BAG_QUERY),
            FailureClass::Invalid,
            backend("vcq-query-form-mismatch"),
        ),
        (
            Spec::bag().with(
                ResultContract::SelectBag,
                QueryForm::Select,
                "SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o",
            ),
            FailureClass::Unsupported,
            backend("vcq-select-sequence-unsupported"),
        ),
        (described, FailureClass::Unsupported, backend("vcq-describe-unsupported")),
        (based, FailureClass::Unsupported, backend("vcq-base-iri-unsupported")),
        (
            Spec::bag().with(
                ResultContract::SelectBag,
                QueryForm::Select,
                "SELECT * WHERE { SERVICE <http://ex/s> { ?s ?p ?o } }",
            ),
            FailureClass::Unsupported,
            backend("vcq-query-not-admitted"),
        ),
        (
            Spec::bag().with(ResultContract::AskBoolean, QueryForm::Ask, "ASK {"),
            FailureClass::Invalid,
            backend("vcq-query-parse"),
        ),
        (
            Spec::bag().with(ResultContract::AskBoolean, QueryForm::Ask, &long),
            FailureClass::Capacity,
            ErrorCode::CapacityExceeded(CapacityBound::Backend {
                name: "v3-query-bytes",
                requested: long.len() as u64,
                ceiling: 8_192,
            }),
        ),
    ];
    let store = Store::default();
    for (spec, class, code) in cases {
        let request = spec.stored();
        let admission = adapter()
            .admit(request.requirements(), adapter().descriptor())
            .expect("structurally admitted");
        let prepared = adapter().prepare(&request, &admission, dataset()).unwrap_err();
        let verified = verify(&request, &placeholder(), &store).unwrap_err();
        for (error, phase) in [(prepared, Phase::Prepare), (verified, Phase::Verify)] {
            assert_eq!((error.class(), error.code(), error.phase()), (class, code, phase), "{}", spec.query);
        }
    }
    assert_eq!(store.calls(), 0);
}

#[test]
fn admission_from_another_request_is_rejected() {
    let holder = Spec::bag().stored();
    let mut agreed = Spec::bag();
    agreed.authority = ScopeAuthority::VerifierAgreedAnchor;
    agreed.anchor = Some([5; 32]);
    let agreed = agreed.stored();
    let admission = adapter()
        .admit(holder.requirements(), adapter().descriptor())
        .unwrap();
    let error = adapter().prepare(&agreed, &admission, dataset()).unwrap_err();
    assert_eq!(error.code(), backend("vcq-admission-mismatch"));
    let store = Store::default();
    let error = adapter()
        .verify_at(&agreed, &id(AUDIENCE), NOW, &admission, &placeholder(), &store)
        .unwrap_err();
    assert_eq!(error.code(), backend("vcq-admission-mismatch"));
    assert_eq!(store.calls(), 0);
}

#[test]
fn prepare_checks_the_private_dataset_and_anchor_opening() {
    let expected_commitment = v3::dataset_commitment(&dataset(), &v3::Policy::default()).unwrap();
    for (anchor, outcome) in [
        (expected_commitment, None),
        ([7; 32], Some((FailureClass::Unsatisfiable, backend("vcq-anchor-not-opened")))),
    ] {
        let mut spec = Spec::bag();
        spec.authority = ScopeAuthority::VerifierAgreedAnchor;
        spec.anchor = Some(anchor);
        let request = spec.stored();
        let admission = adapter()
            .admit(request.requirements(), adapter().descriptor())
            .unwrap();
        let result = adapter().prepare(&request, &admission, dataset());
        match outcome {
            None => {
                let witness = format!("{:?}", result.expect("anchor opens"));
                assert!(!witness.contains("http://ex/"), "witness Debug is redacted");
            }
            Some((class, code)) => {
                let error = result.unwrap_err();
                assert_eq!((error.class(), error.code()), (class, code));
            }
        }
    }
    let request = Spec::bag().stored();
    let admission = adapter()
        .admit(request.requirements(), adapter().descriptor())
        .unwrap();
    let mut zero_salt = dataset();
    zero_salt.salt = [0; 32];
    let error = adapter().prepare(&request, &admission, zero_salt).unwrap_err();
    assert_eq!(error.code(), backend("vcq-private-dataset-rejected"));
}

#[test]
fn verify_gates_run_before_receipt_decoding_and_consumption() {
    let store = Store::default();
    let request = Spec::bag().stored();
    let admission = adapter()
        .admit(request.requirements(), adapter().descriptor())
        .unwrap();
    let at = |audience: &str, now: u64, presentation: &VcqPresentation| {
        adapter()
            .verify_at(&request, &id(audience), now, &admission, presentation, &store)
            .unwrap_err()
    };
    let placeholder = placeholder();
    assert_eq!(
        at("urn:example:other", NOW, &placeholder).code(),
        backend("vcq-audience-mismatch")
    );
    assert_eq!(
        at(AUDIENCE, NOT_BEFORE - 1, &placeholder).code(),
        backend("vcq-request-not-yet-valid")
    );
    assert_eq!(
        at(AUDIENCE, NOT_AFTER, &placeholder).code(),
        backend("vcq-request-expired")
    );
    // At `not_before` the window is open, so the next gate (decoding) rejects.
    assert_eq!(
        at(AUDIENCE, NOT_BEFORE, &placeholder).code(),
        backend("vcq-receipt-decoding")
    );
    let mut digest = vcq::descriptor_digest(adapter().descriptor());
    digest[0] ^= 1;
    let foreign = VcqPresentation::new(digest, b"not a receipt".to_vec());
    assert_eq!(
        at(AUDIENCE, NOW, &foreign).code(),
        backend("vcq-descriptor-digest-mismatch")
    );

    // The byte bound is checked BEFORE decoding: undecodable bytes over the
    // bound report capacity, not a decoding failure.
    let mut small = Spec::bag();
    small.bytes = 64;
    let small = small.stored();
    let oversize = VcqPresentation::new(
        vcq::descriptor_digest(adapter().descriptor()),
        vec![b'x'; 33],
    );
    let error = verify(&small, &oversize, &store).unwrap_err();
    assert_eq!(
        error.code(),
        ErrorCode::CapacityExceeded(CapacityBound::Backend {
            name: "vcq-presentation-bytes",
            requested: 65,
            ceiling: 64,
        })
    );
    let fits = VcqPresentation::new(vcq::descriptor_digest(adapter().descriptor()), vec![b'x'; 32]);
    assert_eq!(
        verify(&small, &fits, &store).unwrap_err().code(),
        backend("vcq-receipt-decoding")
    );

    let unconfigured = adapter()
        .verify(&request, &admission, &placeholder, &store)
        .unwrap_err();
    assert_eq!(unconfigured.code(), backend("vcq-verifier-context-missing"));
    assert_eq!(store.calls(), 0);
}

#[test]
fn fake_receipt_with_a_correctly_bound_journal_is_rejected_without_consumption() {
    let request = Spec::bag().stored();
    let expected =
        vcq::expected_v3_request(&request, adapter().descriptor(), Phase::Verify).unwrap();
    let witness = v3::Witness {
        request: expected,
        dataset: dataset(),
    };
    // Native differential journal for the fixture; not a proof.
    let journal = v3::evaluate(&witness).expect("native V3 evaluation");
    let one = Some("\"1\"".to_owned());
    assert_eq!(
        journal.result,
        v3::CanonicalResult::Select {
            variables: vec!["o".to_owned()],
            order: sparq_proved_evaluator_model::RowOrder::Bag,
            rows: vec![vec![one.clone()], vec![one]],
        },
        "hand-defined duplicate bag"
    );
    let bytes: Vec<u8> = risc0_zkvm::serde::to_vec(&journal)
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let fake = Presentation {
        receipt: Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(method_id(), bytes.clone()))),
            bytes,
        ),
    };
    let presentation = VcqPresentation::new(
        vcq::descriptor_digest(adapter().descriptor()),
        serde_json::to_vec(&fake).unwrap(),
    );
    for store in [Store::default(), Store { broken: true, ..Store::default() }] {
        let error = verify(&request, &presentation, &store).unwrap_err();
        assert_eq!(
            (error.class(), error.code()),
            (FailureClass::Invalid, backend("vcq-proof-rejected"))
        );
        assert_eq!(store.calls(), 0, "a rejected proof never reaches the store");
    }
}

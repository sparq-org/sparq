//! [OPUS-5.5] Stored-request, local-encoding and shared challenge-store tests.
//!
//! These are contract and mock tests. Every store and simulated method here
//! is a test double, and these tests generate and verify ZERO proofs.

use std::collections::HashSet;
use std::sync::{Barrier, Mutex};
use std::thread;

use sparq_query_protocol::{
    admit, consume_challenge, encode_method_descriptor, encode_stored_request, ArtifactIdentity,
    BaseIri, Capabilities, CapabilitiesSpec, CapabilityTuple, Challenge32, ChallengeConsumption,
    ChallengeOutcome, ChallengeOwner, ChallengePolicy, ChallengeStore, ChallengeStoreError,
    ComponentStatus, DatasetAssembly, Digest32, Enforcement, Enforcer, ErrorCode, EvaluationMode,
    FailureClass, HolderPolicy, Identifier, MethodDescriptor, Phase, ProtocolError, QueryForm,
    QueryMethod, QueryProfile, QueryRequirements, RequirementsSpec, ResourceBounds,
    ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest,
    StoredRequestSpec, LOCAL_ENCODING_PROFILE, MAX_BASE_IRI_LEN, MAX_QUERY_LEN,
    METHOD_DESCRIPTOR_DOMAIN, STORED_REQUEST_DOMAIN,
};

fn id(value: &str) -> Identifier {
    Identifier::new(value).expect("valid test identifier")
}

fn digest(byte: u8) -> Digest32 {
    Digest32::new([byte; 32]).expect("nonzero test digest")
}

fn challenge(byte: u8) -> Challenge32 {
    Challenge32::new([byte; 32]).expect("nonzero test challenge")
}

fn unhex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn vk(byte: u8) -> ArtifactIdentity {
    ArtifactIdentity::VerificationKey {
        vk_digest: digest(byte),
    }
}

fn desc(
    method: &str,
    version: u32,
    params: &str,
    params_digest: u8,
    artifact: ArtifactIdentity,
    backend: &str,
) -> MethodDescriptor {
    MethodDescriptor::new(
        id(method),
        version,
        id(params),
        digest(params_digest),
        artifact,
        id(backend),
    )
    .expect("valid test descriptor")
}

// ---- golden vectors -------------------------------------------------------

macro_rules! golden_descriptor_hex {
    () => {
        concat!(
            "73706172713a7663713a",                 // "sparq:vcq:"
            "6d6574686f642d64657363726970746f723a", // "method-descriptor:"
            "6c6f63616c2d7374727563742d7631",       // "local-struct-v1"
            "00",
            "01", "0000000000000003", "613a62", // method "a:b"
            "02", "00000001",                   // version 1
            "03", "0000000000000003", "703a71", // parameter set "p:q"
            "04", // parameter digest 0x11 x 32
            "11111111111111111111111111111111",
            "11111111111111111111111111111111",
            "05", "01", // artifact VerificationKey, vk 0x22 x 32
            "22222222222222222222222222222222",
            "22222222222222222222222222222222",
            "06", "0000000000000003", "623a63", // backend "b:c"
        )
    };
}

const GOLDEN_STORED_REQUEST_HEX: &str = concat!(
    "73706172713a7663713a",           // "sparq:vcq:"
    "73746f7265642d726571756573743a", // "stored-request:"
    "6c6f63616c2d7374727563742d7631", // "local-struct-v1"
    "00",
    "01", "0000000000000005", "41534b7b7d", // query "ASK{}"
    "02", "00",                             // base IRI absent
    "03", // challenge 0x33 x 32
    "33333333333333333333333333333333",
    "33333333333333333333333333333333",
    "04", "0000000000000003", "763a61", // audience "v:a"
    "05", "0000000000000001",           // not before 1
    "06", "0000000000000002",           // not after 2
    "07", "02",                         // form Ask
    "08", "00",                         // DESCRIBE policy absent
    "09", "04",                         // contract AskBoolean
    "0a", "02",                         // mode ExactBounded
    "0b", "02",                         // authority HolderDeclared
    "0c", "00",                         // anchor absent
    "0d", "01",                         // source evidence None
    "0e", "02",                         // status NotRequested
    "0f", "02",                         // holder BearerAccepted
    "10", "01",                         // assembly UnionDefaultGraph
    "11", "0000000000000003", "643a31", // dialect "d:1"
    "12", "0000000000000003", "663a31", // fragment "f:1"
    "13", "0000000000000000",           // accepted suites: none
    "14", "0000000000000001", "0000000000000003", "6d3a31", // mappings ["m:1"]
    "15", "0000000000000001", "0000000000000003", "6c3a31", // linking ["l:1"]
    "16", "00000001",                   // released rows 1
    "17", "00000002",                   // presentation bytes 2
    "18", "0000000000000001", "0000000000000098", // one descriptor, 152 bytes
    golden_descriptor_hex!(),
);

fn golden_descriptor() -> MethodDescriptor {
    desc("a:b", 1, "p:q", 0x11, vk(0x22), "b:c")
}

fn golden_request() -> StoredRequest {
    let requirements = QueryRequirements::new(RequirementsSpec {
        contract: ResultContract::AskBoolean,
        mode: EvaluationMode::ExactBounded,
        authority: ScopeAuthority::HolderDeclared,
        anchor: None,
        source_evidence: Some(SourceEvidence::None),
        status: Some(StatusPolicy::NotRequested),
        holder: Some(HolderPolicy::BearerAccepted),
        assembly: DatasetAssembly::UnionDefaultGraph,
        query_profile: QueryProfile {
            dialect: id("d:1"),
            fragment: id("f:1"),
        },
        accepted_suites: vec![],
        accepted_mappings: vec![id("m:1")],
        accepted_linking: vec![id("l:1")],
        resources: ResourceBounds::new(1, 2).expect("nonzero bounds"),
        methods: vec![golden_descriptor()],
    })
    .expect("valid golden requirements");
    StoredRequest::new(StoredRequestSpec {
        requirements,
        query: "ASK{}".to_owned(),
        challenge: challenge(0x33),
        audience: id("v:a"),
        not_before: 1,
        not_after: 2,
        form: QueryForm::Ask,
        base_iri: None,
        describe_policy: None,
    })
    .expect("valid golden request")
}

#[test]
fn descriptor_matches_golden_bytes() {
    let expected = unhex(golden_descriptor_hex!());
    assert_eq!(expected.len(), 152);
    assert_eq!(encode_method_descriptor(&golden_descriptor()), expected);
    assert!(expected.starts_with(METHOD_DESCRIPTOR_DOMAIN));
}

#[test]
fn stored_request_matches_golden_bytes() {
    let expected = unhex(GOLDEN_STORED_REQUEST_HEX);
    assert_eq!(expected.len(), 392);
    let request = golden_request();
    assert_eq!(encode_stored_request(&request), expected);
    assert_eq!(encode_stored_request(&request.clone()), expected);
    assert!(expected.starts_with(STORED_REQUEST_DOMAIN));
    assert_eq!(LOCAL_ENCODING_PROFILE, "local-struct-v1");
}

// ---- sensitivity ----------------------------------------------------------

fn method_a() -> MethodDescriptor {
    desc("urn:t:m", 1, "urn:t:p", 1, vk(2), "urn:t:b")
}

fn method_b() -> MethodDescriptor {
    desc("urn:t:m", 2, "urn:t:p", 1, vk(2), "urn:t:b")
}

fn base_requirements() -> RequirementsSpec {
    RequirementsSpec {
        contract: ResultContract::SelectDistinctSet,
        mode: EvaluationMode::ExactBounded,
        authority: ScopeAuthority::HolderDeclared,
        anchor: None,
        source_evidence: Some(SourceEvidence::None),
        status: Some(StatusPolicy::NotRequested),
        holder: Some(HolderPolicy::BearerAccepted),
        assembly: DatasetAssembly::UnionDefaultGraph,
        query_profile: QueryProfile {
            dialect: id("d:1"),
            fragment: id("f:1"),
        },
        accepted_suites: vec![id("s:1")],
        accepted_mappings: vec![id("m:1"), id("m:2")],
        accepted_linking: vec![id("l:1"), id("l:2")],
        resources: ResourceBounds::new(10, 1000).expect("nonzero bounds"),
        methods: vec![method_a(), method_b()],
    }
}

fn base_spec(requirements: QueryRequirements) -> StoredRequestSpec {
    StoredRequestSpec {
        requirements,
        query: "SELECT * {}".to_owned(),
        challenge: challenge(0x33),
        audience: id("v:a"),
        not_before: 10,
        not_after: 20,
        form: QueryForm::Select,
        base_iri: None,
        describe_policy: None,
    }
}

fn build(
    edit_requirements: impl FnOnce(&mut RequirementsSpec),
    edit: impl FnOnce(&mut StoredRequestSpec),
) -> Result<StoredRequest, ProtocolError> {
    let mut requirements = base_requirements();
    edit_requirements(&mut requirements);
    let mut spec = base_spec(QueryRequirements::new(requirements).expect("valid requirements"));
    edit(&mut spec);
    StoredRequest::new(spec)
}

fn encode(
    edit_requirements: impl FnOnce(&mut RequirementsSpec),
    edit: impl FnOnce(&mut StoredRequestSpec),
) -> Vec<u8> {
    encode_stored_request(&build(edit_requirements, edit).expect("valid variant"))
}

fn req(edit: impl FnOnce(&mut RequirementsSpec)) -> Vec<u8> {
    encode(edit, |_| {})
}

fn top(edit: impl FnOnce(&mut StoredRequestSpec)) -> Vec<u8> {
    encode(|_| {}, edit)
}

fn assert_all_distinct(base: &[u8], variants: Vec<(&str, Vec<u8>)>) {
    let mut seen = HashSet::from([base.to_vec()]);
    for (name, bytes) in variants {
        assert!(seen.insert(bytes), "variant `{name}` aliases another encoding");
    }
}

#[test]
fn every_stored_request_field_changes_the_encoding() {
    let base = top(|_| {});
    let graph = |r: &mut RequirementsSpec| r.contract = ResultContract::GraphRdfc10;
    let describe = |policy: &'static str| {
        move |s: &mut StoredRequestSpec| {
            s.form = QueryForm::Describe;
            s.describe_policy = Some(id(policy));
        }
    };
    let composite = ArtifactIdentity::Composite {
        circuit_digest: digest(2),
        setup_digest: digest(3),
    };
    let variants = vec![
        ("query", top(|s| s.query = "SELECT * { }".to_owned())),
        ("base present", top(|s| s.base_iri = Some(BaseIri::new("x:/a").unwrap()))),
        ("base value", top(|s| s.base_iri = Some(BaseIri::new("x:/b").unwrap()))),
        ("challenge", top(|s| s.challenge = challenge(0x34))),
        ("audience", top(|s| s.audience = id("v:b"))),
        ("not before", top(|s| s.not_before = 11)),
        ("not after", top(|s| s.not_after = 21)),
        ("form construct", encode(graph, |s| s.form = QueryForm::Construct)),
        ("form describe", encode(graph, describe("urn:t:describe:cbd"))),
        ("describe policy", encode(graph, describe("urn:t:describe:scbd"))),
        ("contract", req(|r| r.contract = ResultContract::SelectBag)),
        ("mode", req(|r| r.mode = EvaluationMode::SelectedSupport)),
        (
            "authority",
            req(|r| {
                r.authority = ScopeAuthority::VerifierAgreedAnchor;
                r.anchor = Some(digest(9));
            }),
        ),
        (
            "anchor",
            req(|r| {
                r.authority = ScopeAuthority::VerifierAgreedAnchor;
                r.anchor = Some(digest(10));
            }),
        ),
        ("evidence issuer", req(|r| r.source_evidence = Some(SourceEvidence::IssuerAuthenticated))),
        ("evidence reattested", req(|r| r.source_evidence = Some(SourceEvidence::ReAttested))),
        ("status", req(|r| r.status = Some(StatusPolicy::Required))),
        ("holder", req(|r| r.holder = Some(HolderPolicy::Required))),
        ("assembly named", req(|r| r.assembly = DatasetAssembly::CredentialNamedGraphs)),
        ("assembly catalog", req(|r| r.assembly = DatasetAssembly::ExactSourceCatalog)),
        ("dialect", req(|r| r.query_profile.dialect = id("d:2"))),
        ("fragment", req(|r| r.query_profile.fragment = id("f:2"))),
        ("suites added", req(|r| r.accepted_suites.push(id("s:2")))),
        ("suites empty", req(|r| r.accepted_suites.clear())),
        ("suites value", req(|r| r.accepted_suites = vec![id("s:2")])),
        ("mappings order", req(|r| r.accepted_mappings.reverse())),
        ("mappings removed", req(|r| r.accepted_mappings.truncate(1))),
        ("linking order", req(|r| r.accepted_linking.reverse())),
        ("linking removed", req(|r| r.accepted_linking.truncate(1))),
        ("rows", req(|r| r.resources = ResourceBounds::new(11, 1000).unwrap())),
        ("bytes", req(|r| r.resources = ResourceBounds::new(10, 1001).unwrap())),
        ("methods order", req(|r| r.methods.reverse())),
        ("methods removed", req(|r| r.methods.truncate(1))),
        (
            "nested artifact",
            req(|r| r.methods[1] = desc("urn:t:m", 2, "urn:t:p", 1, composite, "urn:t:b")),
        ),
        (
            "nested params digest",
            req(|r| r.methods[1] = desc("urn:t:m", 2, "urn:t:p", 7, vk(2), "urn:t:b")),
        ),
        (
            "nested backend",
            req(|r| r.methods[1] = desc("urn:t:m", 2, "urn:t:p", 1, vk(2), "urn:t:c")),
        ),
    ];
    assert_all_distinct(&base, variants);
}

#[test]
fn every_descriptor_field_changes_the_encoding() {
    let zkvm = |a, b| ArtifactIdentity::ZkvmGuest {
        artifact_digest: digest(a),
        image_id: digest(b),
    };
    let base = encode_method_descriptor(&method_a());
    let variants = [
        ("method", desc("urn:t:n", 1, "urn:t:p", 1, vk(2), "urn:t:b")),
        ("version", desc("urn:t:m", 3, "urn:t:p", 1, vk(2), "urn:t:b")),
        ("parameter set", desc("urn:t:m", 1, "urn:t:q", 1, vk(2), "urn:t:b")),
        ("parameter digest", desc("urn:t:m", 1, "urn:t:p", 9, vk(2), "urn:t:b")),
        ("vk", desc("urn:t:m", 1, "urn:t:p", 1, vk(3), "urn:t:b")),
        ("zkvm", desc("urn:t:m", 1, "urn:t:p", 1, zkvm(2, 3), "urn:t:b")),
        ("zkvm swapped", desc("urn:t:m", 1, "urn:t:p", 1, zkvm(3, 2), "urn:t:b")),
        ("zkvm image", desc("urn:t:m", 1, "urn:t:p", 1, zkvm(2, 4), "urn:t:b")),
        (
            "composite",
            desc(
                "urn:t:m",
                1,
                "urn:t:p",
                1,
                ArtifactIdentity::Composite {
                    circuit_digest: digest(2),
                    setup_digest: digest(3),
                },
                "urn:t:b",
            ),
        ),
        ("backend", desc("urn:t:m", 1, "urn:t:p", 1, vk(2), "urn:t:c")),
    ];
    let encoded = variants
        .into_iter()
        .map(|(name, d)| (name, encode_method_descriptor(&d)))
        .collect();
    assert_all_distinct(&base, encoded);
}

#[test]
fn length_prefixes_prevent_concatenation_aliasing() {
    // One list entry versus two entries with the same concatenated text.
    assert_ne!(
        req(|r| r.accepted_linking = vec![id("l:1l:2")]),
        req(|r| r.accepted_linking = vec![id("l:1"), id("l:2")]),
    );
    // An entry moved across the boundary between two adjacent lists.
    assert_ne!(
        req(|r| {
            r.accepted_mappings = vec![id("m:1"), id("m:2")];
            r.accepted_linking = vec![id("l:1")];
        }),
        req(|r| {
            r.accepted_mappings = vec![id("m:1")];
            r.accepted_linking = vec![id("m:2"), id("l:1")];
        }),
    );
    // Query text absorbing the following optional base IRI.
    assert_ne!(
        top(|s| s.base_iri = Some(BaseIri::new("x:y").unwrap())),
        top(|s| s.query = "SELECT * {}x:y".to_owned()),
    );
    // Explicit none versus present.
    assert_ne!(top(|_| {}), top(|s| s.base_iri = Some(BaseIri::new("x").unwrap())));
}

// ---- validation -----------------------------------------------------------

fn assert_request_invalid(result: Result<StoredRequest, ProtocolError>, code: ErrorCode) {
    let error = result.expect_err("request must be rejected");
    assert_eq!(
        (error.class(), error.phase(), error.code()),
        (FailureClass::Invalid, Phase::Request, code)
    );
}

#[test]
fn rejects_zero_challenge() {
    let error = Challenge32::new([0; 32]).expect_err("zero challenge");
    assert_eq!(error.code(), ErrorCode::ZeroChallenge);
    assert_eq!(error.class(), FailureClass::Invalid);
    assert_eq!(format!("{:?}", challenge(0xab)), format!("Challenge32({})", "ab".repeat(32)));
}

#[test]
fn rejects_empty_or_oversize_query_at_the_exact_bound() {
    assert_request_invalid(build(|_| {}, |s| s.query.clear()), ErrorCode::MalformedQuery);
    let at_bound = build(|_| {}, |s| s.query = "a".repeat(MAX_QUERY_LEN)).expect("at bound");
    assert_eq!(at_bound.query_bytes().len(), MAX_QUERY_LEN);
    assert_request_invalid(
        build(|_| {}, |s| s.query = "a".repeat(MAX_QUERY_LEN + 1)),
        ErrorCode::MalformedQuery,
    );
}

#[test]
fn rejects_empty_or_inverted_validity_window() {
    let window = |nb, na| build(|_| {}, move |s| (s.not_before, s.not_after) = (nb, na));
    assert_request_invalid(window(5, 5), ErrorCode::InvalidValidityWindow);
    assert_request_invalid(window(6, 5), ErrorCode::InvalidValidityWindow);
    assert_request_invalid(window(u64::MAX, 0), ErrorCode::InvalidValidityWindow);
    let ok = window(0, u64::MAX).expect("widest window");
    assert_eq!((ok.not_before(), ok.not_after()), (0, u64::MAX));
}

#[test]
fn form_and_contract_must_agree() {
    use QueryForm::{Ask, Construct, Describe, Select};
    use ResultContract::{
        AskBoolean, AskTrueOnly, GraphRdfc10, SelectBag, SelectDistinctSet, SelectSequence,
    };
    let table = [
        (SelectDistinctSet, [true, false, false, false]),
        (SelectBag, [true, false, false, false]),
        (SelectSequence, [true, false, false, false]),
        (AskBoolean, [false, true, false, false]),
        (AskTrueOnly, [false, true, false, false]),
        (GraphRdfc10, [false, false, true, true]),
    ];
    for (contract, expected) in table {
        for (form, want) in [Select, Ask, Construct, Describe].into_iter().zip(expected) {
            assert_eq!(form.supports_contract(contract), want, "{form:?} x {contract:?}");
        }
    }
    let with = |contract, form| {
        build(
            move |r| r.contract = contract,
            move |s| {
                s.form = form;
                s.describe_policy = (form == Describe).then(|| id("urn:t:describe:cbd"));
            },
        )
    };
    assert_request_invalid(with(AskBoolean, Select), ErrorCode::FormContractMismatch);
    assert_request_invalid(with(SelectBag, Ask), ErrorCode::FormContractMismatch);
    assert_request_invalid(with(SelectDistinctSet, Construct), ErrorCode::FormContractMismatch);
    assert_request_invalid(with(AskBoolean, Describe), ErrorCode::FormContractMismatch);
    assert!(with(GraphRdfc10, Describe).is_ok());
}

#[test]
fn describe_policy_is_required_exactly_for_describe() {
    let graph = |r: &mut RequirementsSpec| r.contract = ResultContract::GraphRdfc10;
    let policy = || Some(id("urn:t:describe:cbd"));
    assert_request_invalid(
        build(graph, |s| s.form = QueryForm::Describe),
        ErrorCode::DescribePolicyMismatch,
    );
    assert_request_invalid(
        build(graph, |s| {
            s.form = QueryForm::Construct;
            s.describe_policy = policy();
        }),
        ErrorCode::DescribePolicyMismatch,
    );
    assert_request_invalid(
        build(|_| {}, |s| s.describe_policy = policy()),
        ErrorCode::DescribePolicyMismatch,
    );
}

#[test]
fn base_iri_is_a_bounded_lexical_shape_only() {
    for bad in ["", " x:/", "x:/a b", "x:\t/", "x:/\u{7f}", "x:/\u{85}", "x:/\u{2028}"] {
        let error = BaseIri::new(bad).expect_err("malformed base");
        assert_eq!(error.code(), ErrorCode::MalformedBaseIri, "{bad:?}");
    }
    assert!(BaseIri::new(&"a".repeat(MAX_BASE_IRI_LEN)).is_ok());
    assert!(BaseIri::new(&"a".repeat(MAX_BASE_IRI_LEN + 1)).is_err());
    // `?` is legal in IRIs, unlike in `Identifier`; no RFC 3987 check runs.
    assert!(Identifier::new("http://example.org/a?b").is_err());
    let base = BaseIri::new("http://example.org/a?b#c").expect("query and fragment");
    assert_eq!(base.as_str(), "http://example.org/a?b#c");
    assert!(BaseIri::new("relative/path").is_ok());
}

#[test]
fn getters_return_the_stored_values() {
    let request = build(
        |_| {},
        |s| s.base_iri = Some(BaseIri::new("http://example.org/").unwrap()),
    )
    .expect("valid");
    assert_eq!(request.query(), "SELECT * {}");
    assert_eq!(request.query_bytes(), b"SELECT * {}");
    assert_eq!(request.challenge(), &challenge(0x33));
    assert_eq!(request.audience(), &id("v:a"));
    assert_eq!(request.form(), QueryForm::Select);
    assert_eq!(request.base_iri().map(BaseIri::as_str), Some("http://example.org/"));
    assert_eq!(request.describe_policy(), None);
    let requirements = request.requirements();
    assert_eq!(requirements.accepted_mappings(), &[id("m:1"), id("m:2")]);
    assert_eq!(requirements.methods(), &[method_a(), method_b()]);
    assert_eq!(requirements.assembly(), DatasetAssembly::UnionDefaultGraph);
}

#[test]
fn exact_source_catalog_is_its_own_assembly_axis() {
    let tuple = CapabilityTuple {
        query_profile: QueryProfile {
            dialect: id("d:1"),
            fragment: id("f:1"),
        },
        contract: ResultContract::SelectDistinctSet,
        mode: EvaluationMode::ExactBounded,
        authority: ScopeAuthority::HolderDeclared,
        source_evidence: SourceEvidence::None,
        status: StatusPolicy::NotRequested,
        holder: HolderPolicy::BearerAccepted,
        assembly: DatasetAssembly::UnionDefaultGraph,
        suite: None,
        mapping: id("m:1"),
        linking: vec![id("l:1")],
        enforcement: Enforcement {
            authenticity: Enforcer::Absent,
            mapping: Enforcer::Relation(id("urn:t:relation:r")),
            status: Enforcer::Absent,
            holder_binding: Enforcer::Absent,
            linking: Enforcer::Relation(id("urn:t:relation:r")),
            query: Enforcer::Relation(id("urn:t:relation:r")),
            anchor: Enforcer::Absent,
        },
    };
    let capabilities = Capabilities::new(CapabilitiesSpec {
        descriptor: method_a(),
        status: ComponentStatus::ImplementedExperimental,
        adapter_available: true,
        tuples: vec![tuple],
        ceilings: ResourceBounds::new(10, 1000).expect("nonzero"),
        challenge: ChallengePolicy {
            owner: ChallengeOwner::Adapter,
            consumption: ChallengeConsumption::BurnOnAttempt,
        },
        disclosure: vec![],
    })
    .expect("valid capabilities");
    let union = QueryRequirements::new(base_requirements()).expect("valid");
    assert!(admit(&union, &method_a(), &capabilities).is_ok());
    let catalog = QueryRequirements::new(RequirementsSpec {
        assembly: DatasetAssembly::ExactSourceCatalog,
        ..base_requirements()
    })
    .expect("valid");
    let error = admit(&catalog, &method_a(), &capabilities).expect_err("assembly mismatch");
    assert_eq!(error.code(), ErrorCode::TupleUnsupported);
}

// ---- shared challenge store (contract and mock only, zero proofs) --------

/// Test double: in memory and NOT durable; never a production store.
#[derive(Default)]
struct TestStore(Mutex<HashSet<[u8; 32]>>);

impl ChallengeStore for TestStore {
    fn consume(&self, challenge: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError> {
        let mut seen = self.0.lock().expect("test store lock");
        Ok(if seen.insert(*challenge.as_bytes()) {
            ChallengeOutcome::Fresh
        } else {
            ChallengeOutcome::AlreadyConsumed
        })
    }
}

struct FailingStore;

impl ChallengeStore for FailingStore {
    fn consume(&self, _: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError> {
        Err(ChallengeStoreError::new("io"))
    }
}

/// Test-only stand-in for a per-method nonce derivation; not a hash.
fn derived_nonce(tag: u8, request: &StoredRequest) -> [u8; 32] {
    let mut nonce = *request.challenge().as_bytes();
    nonce[0] ^= tag;
    nonce
}

/// Simulated verify: computes a test-only stand-in nonce, consumes the original, proves nothing.
fn simulated_verify(
    tag: u8,
    store: &dyn ChallengeStore,
    request: &StoredRequest,
) -> Result<[u8; 32], ProtocolError> {
    let nonce = derived_nonce(tag, request);
    consume_challenge(store, request)?;
    Ok(nonce)
}

#[test]
fn shared_store_accepts_one_original_challenge_once_across_methods() {
    let request = golden_request();
    assert_ne!(derived_nonce(1, &request), derived_nonce(2, &request));
    for _ in 0..64 {
        let store = TestStore::default();
        let barrier = Barrier::new(2);
        let (store_ref, request_ref, barrier_ref) = (&store, &request, &barrier);
        let results = thread::scope(|scope| {
            [1_u8, 2]
                .map(|tag| {
                    scope.spawn(move || {
                        barrier_ref.wait();
                        simulated_verify(tag, store_ref, request_ref)
                    })
                })
                .map(|handle| handle.join().expect("simulated method panicked"))
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let replay = results
            .iter()
            .find_map(|r| r.as_ref().err())
            .expect("one replay");
        assert_eq!(
            (replay.class(), replay.phase(), replay.code()),
            (FailureClass::Invalid, Phase::Verify, ErrorCode::ChallengeReplayed)
        );
        assert_eq!(store.consume(request.challenge()), Ok(ChallengeOutcome::AlreadyConsumed));
    }
}

#[test]
fn store_failure_is_infrastructure_not_replay() {
    let error = consume_challenge(&FailingStore, &golden_request()).expect_err("store failure");
    assert_eq!(
        (error.class(), error.phase(), error.code()),
        (
            FailureClass::Infrastructure,
            Phase::Verify,
            ErrorCode::ChallengeStoreFailure("io")
        )
    );
}

/// Compile-time check: `QueryMethod` accepts the stored request and a shared dyn store.
#[expect(dead_code, reason = "compile-time bound check only")]
fn query_method_accepts_shared_store<M>()
where
    M: QueryMethod<Request = StoredRequest, ChallengeStore = dyn ChallengeStore>,
{
}

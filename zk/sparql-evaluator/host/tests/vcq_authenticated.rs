// zkp-14.6: native gates of the optional V5 vcq adapter.
// Rust guideline compliant 2026-02-21
//! Compiled only with `--features vcq-authenticated`.
//!
//! These gates create no proof and never run an executor or `r0vm`. The only
//! receipts they build are FAKE ones, which must be rejected before any
//! challenge-store call. They are native adapter checks and never count as
//! proof evidence; genuine receipts belong to the ignored driver in
//! `vcq_authenticated_genuine.rs`. Every challenge store here is an in-memory
//! test double, never a production store. The tests link the embedded V5 guest
//! artifact, so they need the `authenticated-rdf` guest build.
#![cfg(feature = "vcq-authenticated")]

#[path = "support/authenticated_rdf.rs"]
mod fixture;

use fixture::Expect;
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::vcq::{self, VcqPresentation};
use sparq_proved_evaluator::vcq_authenticated::{self as vcq5, Risc0AuthenticatedRdfV5};
use sparq_proved_evaluator::{
    AcceptedGuest, ArtifactPin, embedded_artifact, embedded_authrdf_artifact,
    embedded_authrdf_pin, embedded_pin, method_id,
};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Policy, PrivateCredentials};
use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_query_protocol::{
    ArtifactIdentity, BaseIri, CapacityBound, Challenge32, ChallengeConsumption, ChallengeOutcome,
    ChallengeOwner, ChallengeStore, ChallengeStoreError, ComponentStatus, DatasetAssembly,
    Digest32, Enforcer, ErrorCode, EvaluationMode, FailureClass, HolderPolicy, Identifier,
    MethodDescriptor, Obligation, Phase, ProtocolError, QueryForm, QueryMethod, QueryProfile,
    QueryRequirements, RequirementsSpec, ResourceBounds, ResultContract, ScopeAuthority,
    SourceEvidence, StatusPolicy, StoredRequest, StoredRequestSpec, encode_method_descriptor,
    encode_stored_request,
};
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const AUDIENCE: &str = "urn:example:verifier";
const NOT_BEFORE: u64 = 1_800_000_000;
const NOT_AFTER: u64 = 1_800_000_600;
const NOW: u64 = 1_800_000_100;
/// The adapter's policy-invalid code, hand-copied from its documentation.
const POLICY_INVALID: &str = "vcq-authrdf-policy-invalid";

fn id(text: &str) -> Identifier {
    Identifier::new(text).unwrap()
}

fn backend(code: &'static str) -> ErrorCode {
    ErrorCode::Backend(code)
}

fn v5_guest() -> AcceptedGuest {
    AcceptedGuest::from_artifact(embedded_authrdf_artifact().to_vec(), &embedded_authrdf_pin())
        .expect("embedded V5 guest matches its own pin")
}

/// The adapter under the verifier's one-entry table for the published vector.
fn adapter() -> &'static Risc0AuthenticatedRdfV5 {
    static ADAPTER: OnceLock<Risc0AuthenticatedRdfV5> = OnceLock::new();
    ADAPTER.get_or_init(|| {
        Risc0AuthenticatedRdfV5::new(&embedded_authrdf_pin(), v5_guest(), fixture::policy())
            .expect("adapter")
    })
}

fn credentials(salt: [u8; 32]) -> PrivateCredentials {
    fixture::credentials(vec![fixture::credential()], salt)
}

/// A second valid table entry: synthetic issuer and method, RFC 8032 public key.
fn two_entry_policy() -> Policy {
    Policy::new(vec![
        fixture::authorized(fixture::W3C_ISSUER, fixture::W3C_VM, fixture::W3C_PUBLIC_KEY),
        fixture::authorized(
            fixture::OTHER_ISSUER,
            fixture::OTHER_VM,
            fixture::RFC8032_TEST1_PUBLIC_KEY,
        ),
    ])
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

/// Every verifier-stored field a test may substitute.
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
    assembly: DatasetAssembly,
    mapping: &'static str,
    linking: &'static str,
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
            query: fixture::SELECT_BAG.to_owned(),
            authority: ScopeAuthority::HolderDeclared,
            anchor: None,
            source_evidence: SourceEvidence::IssuerAuthenticated,
            suites: vec![vcq5::SUITE],
            status: StatusPolicy::NotRequested,
            holder: HolderPolicy::BearerAccepted,
            assembly: DatasetAssembly::UnionDefaultGraph,
            mapping: vcq5::MAPPING_PROFILE,
            linking: vcq5::LINKING_PROFILE,
            dialect: vcq5::QUERY_DIALECT,
            rows: 4,
            bytes: vcq5::MAX_PRESENTATION_BYTES,
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

    fn agreed(mut self, anchor: [u8; 32]) -> Self {
        self.authority = ScopeAuthority::VerifierAgreedAnchor;
        self.anchor = Some(anchor);
        self
    }

    fn requirements(&self) -> QueryRequirements {
        QueryRequirements::new(RequirementsSpec {
            contract: self.contract,
            mode: EvaluationMode::ExactBounded,
            authority: self.authority,
            anchor: self.anchor.map(|anchor| Digest32::new(anchor).unwrap()),
            source_evidence: Some(self.source_evidence),
            status: Some(self.status),
            holder: Some(self.holder),
            assembly: self.assembly,
            query_profile: QueryProfile {
                dialect: id(self.dialect),
                fragment: id(vcq5::QUERY_FRAGMENT),
            },
            accepted_suites: self.suites.iter().map(|suite| id(suite)).collect(),
            accepted_mappings: vec![id(self.mapping)],
            accepted_linking: vec![id(self.linking)],
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
            base_iri: self.base_iri.map(|base| BaseIri::new(base).unwrap()),
            describe_policy: self.describe_policy.map(id),
        })
        .unwrap()
    }
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

fn journal_bytes(journal: &auth::Journal) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(journal)
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

#[test]
fn descriptor_and_capabilities_pin_the_separate_v5_method() {
    fixture::check_published_vector();
    let descriptor = adapter().descriptor();
    assert_eq!(
        descriptor.method().as_str(),
        "urn:sparq:vcq:method:risc0-authenticated-rdf"
    );
    assert_eq!(descriptor.version(), 5);
    assert_eq!(descriptor.parameter_set().as_str(), vcq5::PARAMETER_SET);
    assert_eq!(
        descriptor.parameter_digest().as_bytes(),
        &vcq5::parameter_digest(&fixture::policy()).unwrap()
    );
    let artifact: [u8; 32] = Sha256::digest(embedded_authrdf_artifact()).into();
    assert_eq!(
        *descriptor.artifact(),
        ArtifactIdentity::ZkvmGuest {
            artifact_digest: Digest32::new(artifact).unwrap(),
            image_id: Digest32::new(vcq::image_id_bytes(embedded_authrdf_pin().image_id)).unwrap(),
        }
    );
    assert_eq!(descriptor.backend().as_str(), vcq5::BACKEND_PIN);
    assert_eq!(
        vcq5::descriptor(&embedded_authrdf_pin(), &fixture::policy()).unwrap(),
        *descriptor
    );
    assert_eq!(adapter().policy(), &fixture::policy());

    // Separate from the exact V3 adapter's descriptor.
    let exact = vcq::descriptor(&embedded_pin()).unwrap();
    assert_ne!(exact.method(), descriptor.method());
    assert_ne!(exact.version(), descriptor.version());
    assert_ne!(exact.parameter_set(), descriptor.parameter_set());
    assert_ne!(exact.parameter_digest(), descriptor.parameter_digest());
    assert_ne!(exact.artifact(), descriptor.artifact());
    assert_ne!(vcq::descriptor_digest(&exact), vcq::descriptor_digest(descriptor));

    let capabilities = adapter().capabilities();
    // A local implementation declaration; not validated availability.
    assert!(capabilities.is_executable());
    assert_eq!(capabilities.status(), ComponentStatus::ImplementedExperimental);
    assert_eq!(capabilities.challenge().owner, ChallengeOwner::Method);
    assert_eq!(
        capabilities.challenge().consumption,
        ChallengeConsumption::ConsumeOnSuccess
    );
    let ceilings = capabilities.ceilings();
    assert_eq!(
        (ceilings.released_rows(), ceilings.presentation_bytes()),
        (4_096, 16 << 20)
    );
    assert_eq!(capabilities.disclosure().to_vec(), vec![id(vcq5::DISCLOSURE)]);
    let relation = Enforcer::Relation(id(vcq5::RELATION));
    let tuples = capabilities.tuples();
    assert_eq!(tuples.len(), 6);
    let mut seen = BTreeSet::new();
    for tuple in tuples {
        assert!(matches!(
            tuple.contract,
            ResultContract::SelectBag | ResultContract::AskBoolean | ResultContract::GraphRdfc10
        ));
        assert_eq!(tuple.mode, EvaluationMode::ExactBounded);
        assert_eq!(tuple.source_evidence, SourceEvidence::IssuerAuthenticated);
        assert_eq!(tuple.suite, Some(id(vcq5::SUITE)));
        assert_eq!(tuple.status, StatusPolicy::NotRequested);
        assert_eq!(tuple.holder, HolderPolicy::BearerAccepted);
        assert_eq!(tuple.assembly, DatasetAssembly::UnionDefaultGraph);
        assert_eq!(tuple.mapping, id(vcq5::MAPPING_PROFILE));
        assert_eq!(tuple.linking, vec![id(vcq5::LINKING_PROFILE)]);
        assert_eq!(
            tuple.query_profile,
            QueryProfile {
                dialect: id(vcq5::QUERY_DIALECT),
                fragment: id(vcq5::QUERY_FRAGMENT),
            }
        );
        for obligation in [
            Obligation::Authenticity,
            Obligation::Mapping,
            Obligation::Linking,
            Obligation::Query,
        ] {
            assert_eq!(*tuple.enforcement.route(obligation), relation, "{obligation:?}");
        }
        for obligation in [Obligation::Status, Obligation::HolderBinding] {
            assert_eq!(*tuple.enforcement.route(obligation), Enforcer::Absent);
        }
        let anchor = match tuple.authority {
            ScopeAuthority::VerifierAgreedAnchor => Enforcer::HostPublic(id(vcq5::ANCHOR_CHECK)),
            ScopeAuthority::HolderDeclared => Enforcer::Absent,
        };
        assert_eq!(tuple.enforcement.anchor, anchor);
        seen.insert(format!("{:?}/{:?}", tuple.contract, tuple.authority));
    }
    assert_eq!(seen.len(), 6, "whole, distinct contract/authority tuples");
}

#[test]
fn policy_and_parameter_digests_follow_the_documented_composition() {
    let policy = fixture::policy();
    // Hand-built sentinel request; only the policy varies between descriptors.
    let sentinel = auth::Request {
        version: 5,
        query: "ASK {}".to_owned(),
        authority: DatasetAuthority::HolderDeclared,
        policy: policy.clone(),
        nonce: *b"sparq/vcq/authrdf/policy-binding",
    };
    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-authenticated-rdf:policy:v5\0");
    hash.update(auth::request_digest(&sentinel).unwrap());
    let policy_digest: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq5::policy_digest(&policy).unwrap(), policy_digest);

    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-authenticated-rdf:parameters:v5\0");
    hash.update(5_u32.to_be_bytes());
    hash.update(policy_digest);
    for id in [
        "urn:sparq:vcq:suite:di-eddsa-rdfc-2022:v5-bounded-canonical-rdf",
        "urn:sparq:vcq:map:v5-scoped-canonical-union",
        "urn:sparq:vcq:link:authenticated-dataset-evaluation",
        "urn:sparq:vcq:relation:risc0-authenticated-rdf-v5-guest",
    ] {
        hash.update((id.len() as u64).to_be_bytes());
        hash.update(id.as_bytes());
    }
    // Hand-copied bounds: query bytes, then the V5 model bounds in declaration
    // order. The witness bound is 8192 + 32768 + 16 * (2 * 512 + 128) + 4 * 256 + 4096.
    for bound in [
        8_192_u64, 4, 16, 512, 8_192, 2_048, 32_768, 128, 8, 256, 64_512,
    ] {
        hash.update(bound.to_be_bytes());
    }
    for ceiling in [4_096_u32, 16 << 20] {
        hash.update(ceiling.to_be_bytes());
    }
    let expected: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq5::parameter_digest(&policy).unwrap(), expected);
    assert_eq!(auth::MAX_WITNESS_BYTES, 64_512);
}

#[test]
fn every_policy_field_changes_every_binding_but_table_order_does_not() {
    // The model policy has no `created` or controller field: `created` is a
    // signed proof option inside each credential (covered by the signature and
    // the commitment's config hash), and the entry's issuer is the only
    // issuer-to-key authorization. The suite, mapping and DESCRIBE policy have
    // one variant each, so they cannot be mutated here.
    let base = two_entry_policy();
    let mut cases: Vec<(&'static str, Policy)> = vec![("base", base.clone())];
    let mut push = |name: &'static str, change: &dyn Fn(&mut Policy)| {
        let mut policy = base.clone();
        change(&mut policy);
        cases.push((name, policy));
    };
    push("entry0-issuer", &|p| {
        p.authorization[0].issuer = "https://vc.example/issuers/1111".into();
    });
    push("entry0-method", &|p| {
        p.authorization[0].verification_method = "did:example:w3c#key-2".into();
    });
    push("entry0-key", &|p| {
        p.authorization[0].public_key = fixture::hex(fixture::RFC8032_TEST1_PUBLIC_KEY);
    });
    push("entry1-issuer", &|p| {
        p.authorization[1].issuer = "https://vc.example/issuers/2222".into();
    });
    push("entry1-method", &|p| {
        p.authorization[1].verification_method = "did:example:other-issuer#key-2".into();
    });
    push("entry1-key", &|p| {
        p.authorization[1].public_key = fixture::hex(fixture::W3C_PUBLIC_KEY);
    });
    push("entry-removed", &|p| {
        p.authorization.pop();
    });
    push("entry-added", &|p| {
        p.authorization.push(fixture::authorized(
            "https://vc.example/issuers/3333",
            "did:example:third#key-1",
            fixture::W3C_PUBLIC_KEY,
        ));
    });
    push("dataset-bytes", &|p| p.evaluation.dataset.max_dataset_bytes -= 1);
    push("dataset-triples", &|p| p.evaluation.dataset.max_triples -= 1);
    push("dataset-rows", &|p| p.evaluation.dataset.max_rows -= 1);
    push("dataset-named-graphs", &|p| p.evaluation.dataset.max_named_graphs -= 1);
    push("canon-quads", &|p| p.evaluation.canonicalization.max_quads -= 1);
    push("canon-input", &|p| p.evaluation.canonicalization.max_input_bytes -= 1);
    push("canon-output", &|p| p.evaluation.canonicalization.max_output_bytes -= 1);
    push("canon-hndq", &|p| p.evaluation.canonicalization.max_hndq_calls -= 1);
    push("canon-steps", &|p| {
        p.evaluation.canonicalization.max_permutation_steps -= 1;
    });

    let pin = ArtifactPin {
        sha256: [3; 32],
        image_id: [3; 8],
    };
    let bindings = |policy: &Policy| {
        let descriptor = vcq5::descriptor(&pin, policy).unwrap();
        let mut spec = Spec::bag();
        spec.methods = vec![descriptor.clone()];
        let stored = spec.stored();
        let expected =
            vcq5::expected_v5_request(&stored, &descriptor, policy, Phase::Verify).unwrap();
        [
            vcq5::policy_digest(policy).unwrap(),
            vcq5::parameter_digest(policy).unwrap(),
            vcq::descriptor_digest(&descriptor),
            vcq5::derive_nonce(&stored, &descriptor),
            auth::request_digest(&expected).unwrap(),
        ]
    };
    let mut distinct: [BTreeSet<[u8; 32]>; 5] = Default::default();
    for (name, policy) in &cases {
        for (set, digest) in distinct.iter_mut().zip(bindings(policy)) {
            assert!(set.insert(digest), "{name}: a binding did not change");
        }
    }
    for set in &distinct {
        assert_eq!(set.len(), cases.len());
    }
    let mut reordered = base.clone();
    reordered.authorization.reverse();
    assert_ne!(reordered, base);
    assert_eq!(bindings(&reordered), bindings(&base), "table order is not significant");
}

#[test]
fn stored_policy_is_the_canonically_ordered_verifier_copy() {
    let mut reordered = two_entry_policy();
    reordered.authorization.reverse();
    let method =
        Risc0AuthenticatedRdfV5::new(&embedded_authrdf_pin(), v5_guest(), reordered).unwrap();
    let methods: Vec<&str> = method
        .policy()
        .authorization
        .iter()
        .map(|entry| entry.verification_method.as_str())
        .collect();
    let mut sorted = methods.clone();
    sorted.sort_unstable();
    assert_eq!(methods, sorted);
    assert_eq!(
        method.descriptor(),
        &vcq5::descriptor(&embedded_authrdf_pin(), &two_entry_policy()).unwrap()
    );
}

#[test]
fn invalid_policies_reject_before_any_descriptor() {
    let valid = || fixture::authorized(fixture::W3C_ISSUER, fixture::W3C_VM, fixture::W3C_PUBLIC_KEY);
    let seventeen = (0..17)
        .map(|i| {
            fixture::authorized(
                fixture::W3C_ISSUER,
                &format!("did:example:k{i}#key-1"),
                fixture::W3C_PUBLIC_KEY,
            )
        })
        .collect();
    let mut relative = valid();
    relative.issuer = "relative-issuer".into();
    let mut oversized = valid();
    oversized.verification_method = format!("did:example:{}", "a".repeat(600));
    let mut identity = valid();
    // The Ed25519 identity point: a valid encoding of small order.
    identity.public_key = std::array::from_fn(|i| u8::from(i == 0));
    let mut no_rows = fixture::policy();
    no_rows.evaluation.dataset.max_rows = 0;
    let mut excess_rows = fixture::policy();
    excess_rows.evaluation.dataset.max_rows = 4_097;
    let mut no_quads = fixture::policy();
    no_quads.evaluation.canonicalization.max_quads = 0;
    let cases = [
        ("empty-table", Policy::new(Vec::new())),
        ("seventeen-keys", Policy::new(seventeen)),
        ("repeated-method", Policy::new(vec![valid(), valid()])),
        ("relative-iri", Policy::new(vec![relative])),
        ("oversized-iri", Policy::new(vec![oversized])),
        ("small-order-key", Policy::new(vec![identity])),
        ("zero-rows", no_rows),
        ("excess-rows", excess_rows),
        ("zero-canonical-quads", no_quads),
    ];
    let pin = embedded_authrdf_pin();
    for (name, policy) in &cases {
        let errors = [
            vcq5::policy_digest(policy).unwrap_err(),
            vcq5::parameter_digest(policy).unwrap_err(),
            vcq5::descriptor(&pin, policy).unwrap_err(),
        ];
        for error in errors {
            assert_eq!(
                (error.class(), error.phase(), error.code()),
                (FailureClass::Invalid, Phase::Request, backend(POLICY_INVALID)),
                "{name}"
            );
        }
    }
    let error =
        Risc0AuthenticatedRdfV5::new(&pin, v5_guest(), Policy::new(Vec::new())).unwrap_err();
    assert_eq!(error.code(), backend(POLICY_INVALID));
}

#[test]
fn nonce_statement_and_expected_request_follow_the_documented_composition() {
    let request = Spec::bag().stored();
    let descriptor = adapter().descriptor();
    let request_digest: [u8; 32] = Sha256::digest(encode_stored_request(&request)).into();
    let descriptor_digest: [u8; 32] = Sha256::digest(encode_method_descriptor(descriptor)).into();
    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-authenticated-rdf:v5-nonce:local-struct-v1\0");
    hash.update(request_digest);
    hash.update(descriptor_digest);
    let nonce: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq5::derive_nonce(&request, descriptor), nonce);
    assert_ne!(&nonce, request.challenge().as_bytes(), "never the original challenge");
    assert_ne!(
        vcq::derive_nonce(&request, descriptor),
        nonce,
        "domain-separated from the V3 adapter"
    );

    let mut hash = Sha256::new();
    hash.update(b"sparq:vcq:risc0-authenticated-rdf:statement:v5\0");
    hash.update(request_digest);
    hash.update(descriptor_digest);
    hash.update(Sha256::digest(b"journal"));
    let statement: [u8; 32] = hash.finalize().into();
    assert_eq!(vcq5::statement_digest(&request, descriptor, b"journal"), statement);

    let expected =
        vcq5::expected_v5_request(&request, descriptor, adapter().policy(), Phase::Verify)
            .unwrap();
    assert_eq!(expected.version, 5);
    assert_eq!(expected.query, fixture::SELECT_BAG);
    assert_eq!(expected.authority, DatasetAuthority::HolderDeclared);
    assert_eq!(expected.policy, fixture::policy());
    assert_eq!(expected.nonce, nonce);
    auth::validate_request(&expected).unwrap();

    let anchor = fixture::anchor(fixture::SALT);
    let agreed = Spec::bag().agreed(anchor).stored();
    let expected =
        vcq5::expected_v5_request(&agreed, descriptor, adapter().policy(), Phase::Verify)
            .unwrap();
    assert_eq!(
        expected.authority,
        DatasetAuthority::VerifierAgreed { commitment: anchor }
    );
    // The verifier's own policy is required; a substituted one is refused.
    let error =
        vcq5::expected_v5_request(&agreed, descriptor, &two_entry_policy(), Phase::Verify)
            .unwrap_err();
    assert_eq!(error.code(), backend("vcq-authrdf-policy-descriptor-mismatch"));
}

#[test]
fn representative_stored_request_fields_change_the_nonce() {
    // A subset; full field coverage lives in the shared encoding tests.
    let base = Spec::bag();
    let other = vcq5::descriptor(&embedded_authrdf_pin(), &two_entry_policy()).unwrap();
    let mut cases: Vec<(&'static str, Spec)> = vec![("base", base.clone())];
    let mut push = |name: &'static str, change: &dyn Fn(&mut Spec)| {
        let mut spec = base.clone();
        change(&mut spec);
        cases.push((name, spec));
    };
    push("query", &|s| {
        s.query = "SELECT ?name WHERE { ?c <https://schema.org/name> ?name }".into();
    });
    push("contract", &|s| {
        *s = s
            .clone()
            .with(ResultContract::AskBoolean, QueryForm::Ask, fixture::ASK_ISSUER);
    });
    push("authority", &|s| *s = s.clone().agreed([5; 32]));
    push("anchor-value", &|s| *s = s.clone().agreed([6; 32]));
    push("method-list", &|s| s.methods.push(other.clone()));
    push("accepted-suites", &|s| s.suites.push("urn:example:suite:other"));
    push("audience", &|s| s.audience = "urn:example:other-verifier");
    push("not-before", &|s| s.not_before += 1);
    push("not-after", &|s| s.not_after += 1);
    push("challenge", &|s| s.challenge[0] ^= 1);
    push("rows", &|s| s.rows += 1);
    push("bytes", &|s| s.bytes -= 1);

    let descriptor = adapter().descriptor();
    let nonces: BTreeSet<[u8; 32]> = cases
        .iter()
        .map(|(_, spec)| vcq5::derive_nonce(&spec.stored(), descriptor))
        .collect();
    let names: Vec<&str> = cases.iter().map(|(name, _)| *name).collect();
    assert_eq!(nonces.len(), cases.len(), "distinct nonces for {names:?}");
    let stored = base.stored();
    assert_ne!(
        vcq5::derive_nonce(&stored, descriptor),
        vcq5::derive_nonce(&stored, &other),
        "selected descriptor, and through it the policy"
    );
}

#[test]
fn stronger_weaker_and_foreign_requests_fail_admission() {
    let admit = |spec: &Spec, selected: &MethodDescriptor| {
        adapter()
            .admit(spec.stored().requirements(), selected)
            .unwrap_err()
    };
    let own = adapter().descriptor().clone();
    let tuple = |change: &dyn Fn(&mut Spec)| {
        let mut spec = Spec::bag();
        change(&mut spec);
        let error = admit(&spec, &own);
        assert_eq!(error.class(), FailureClass::Unsupported);
        error.code()
    };
    let unsupported = ErrorCode::TupleUnsupported;
    assert_eq!(
        tuple(&|s| {
            s.source_evidence = SourceEvidence::None;
            s.suites.clear();
        }),
        unsupported
    );
    assert_eq!(tuple(&|s| s.source_evidence = SourceEvidence::ReAttested), unsupported);
    assert_eq!(tuple(&|s| s.suites = vec!["urn:example:suite:other"]), unsupported);
    assert_eq!(tuple(&|s| s.status = StatusPolicy::Required), unsupported);
    assert_eq!(tuple(&|s| s.holder = HolderPolicy::Required), unsupported);
    assert_eq!(
        tuple(&|s| s.assembly = DatasetAssembly::CredentialNamedGraphs),
        unsupported
    );
    assert_eq!(
        tuple(&|s| s.assembly = DatasetAssembly::ExactSourceCatalog),
        unsupported
    );
    assert_eq!(tuple(&|s| s.mapping = "urn:example:map:other"), unsupported);
    assert_eq!(tuple(&|s| s.linking = "urn:example:link:other"), unsupported);
    for contract in [ResultContract::SelectSequence, ResultContract::SelectDistinctSet] {
        assert_eq!(
            tuple(&|s| *s = s.clone().with(contract, QueryForm::Select, fixture::SELECT_BAG)),
            unsupported
        );
    }
    assert_eq!(
        tuple(&|s| s.dialect = "urn:example:dialect:other"),
        ErrorCode::QueryProfileUnsupported
    );
    let mut spec = Spec::bag();
    spec.rows = 4_097;
    assert_eq!(admit(&spec, &own).class(), FailureClass::Capacity);

    // A foreign descriptor, even when listed, is not this backend.
    let exact = vcq::descriptor(&embedded_pin()).unwrap();
    let replaced = vcq5::descriptor(&embedded_authrdf_pin(), &two_entry_policy()).unwrap();
    for foreign in [exact, replaced] {
        let mut spec = Spec::bag();
        spec.methods = vec![foreign.clone()];
        let error = admit(&spec, &foreign);
        assert_eq!(
            (error.class(), error.phase(), error.code()),
            (
                FailureClass::Unsupported,
                Phase::Negotiation,
                ErrorCode::DescriptorUnavailable
            )
        );
        let error = admit(&spec, &own);
        assert_eq!(error.code(), ErrorCode::DescriptorNotRequested);
    }
}

#[test]
fn typed_query_shape_rejections_precede_any_proof_or_challenge_use() {
    let describe = "DESCRIBE ?s WHERE { ?s ?p ?o }";
    let long = format!("ASK {{ }}{}", " ".repeat(8_192));
    let mut described =
        Spec::bag().with(ResultContract::GraphRdfc10, QueryForm::Describe, describe);
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
            Spec::bag().with(ResultContract::SelectBag, QueryForm::Select, fixture::ASK_ISSUER),
            FailureClass::Invalid,
            backend("vcq-query-form-mismatch"),
        ),
        (
            Spec::bag().with(ResultContract::AskBoolean, QueryForm::Ask, fixture::SELECT_BAG),
            FailureClass::Invalid,
            backend("vcq-query-form-mismatch"),
        ),
        (
            Spec::bag().with(
                ResultContract::SelectBag,
                QueryForm::Select,
                "SELECT ?name WHERE { ?c <https://schema.org/name> ?name } ORDER BY ?name",
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
        let prepared = adapter()
            .prepare(&request, &admission, credentials(fixture::SALT))
            .unwrap_err();
        let verified = verify(&request, &placeholder(), &store).unwrap_err();
        for (error, phase) in [(prepared, Phase::Prepare), (verified, Phase::Verify)] {
            assert_eq!(
                (error.class(), error.code(), error.phase()),
                (class, code, phase),
                "{}",
                spec.query
            );
        }
    }
    assert_eq!(store.calls(), 0);
}

#[test]
fn admission_from_another_request_is_rejected() {
    let holder = Spec::bag().stored();
    let agreed = Spec::bag().agreed(fixture::anchor(fixture::SALT)).stored();
    let admission = adapter()
        .admit(holder.requirements(), adapter().descriptor())
        .unwrap();
    let error = adapter()
        .prepare(&agreed, &admission, credentials(fixture::SALT))
        .unwrap_err();
    assert_eq!(error.code(), backend("vcq-admission-mismatch"));
    let store = Store::default();
    let error = adapter()
        .verify_at(&agreed, &id(AUDIENCE), NOW, &admission, &placeholder(), &store)
        .unwrap_err();
    assert_eq!(error.code(), backend("vcq-admission-mismatch"));
    assert_eq!(store.calls(), 0);
}

#[test]
fn prepare_authenticates_natively_and_checks_the_anchor_opening() {
    let prepare = |spec: Spec, private: PrivateCredentials| {
        let request = spec.stored();
        let admission = adapter()
            .admit(request.requirements(), adapter().descriptor())
            .unwrap();
        adapter().prepare(&request, &admission, private)
    };
    let opened = prepare(
        Spec::bag().agreed(fixture::anchor(fixture::SALT)),
        credentials(fixture::SALT),
    )
    .expect("the verifier's anchor opens");
    let shown = format!("{opened:?}");
    assert!(!shown.contains("Alumni") && !shown.contains("did:example"), "{shown}");
    prepare(Spec::bag(), credentials(fixture::SALT)).expect("holder-declared");

    // The same credential under another salt does not open the anchor.
    let error = prepare(
        Spec::bag().agreed(fixture::anchor(fixture::SALT)),
        credentials(fixture::OTHER_SALT),
    )
    .unwrap_err();
    assert_eq!(
        (error.class(), error.code()),
        (FailureClass::Unsatisfiable, backend("vcq-anchor-not-opened"))
    );
    let mut forged = credentials(fixture::SALT);
    forged.credentials[0].signature[0] ^= 1;
    let mut zero_salt = credentials(fixture::SALT);
    zero_salt.salt = [0; 32];
    let empty = fixture::credentials(Vec::new(), fixture::SALT);
    for private in [forged, zero_salt, empty] {
        let error = prepare(Spec::bag(), private).unwrap_err();
        assert_eq!(
            (error.class(), error.phase(), error.code()),
            (
                FailureClass::Invalid,
                Phase::Prepare,
                backend("vcq-authrdf-credentials-rejected")
            )
        );
    }
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
            .code()
    };
    let placeholder = placeholder();
    assert_eq!(
        at("urn:example:other", NOW, &placeholder),
        backend("vcq-audience-mismatch")
    );
    assert_eq!(
        at(AUDIENCE, NOT_BEFORE - 1, &placeholder),
        backend("vcq-request-not-yet-valid")
    );
    assert_eq!(at(AUDIENCE, NOT_AFTER, &placeholder), backend("vcq-request-expired"));
    assert_eq!(at(AUDIENCE, NOT_BEFORE, &placeholder), backend("vcq-receipt-decoding"));
    let mut digest = vcq::descriptor_digest(adapter().descriptor());
    digest[0] ^= 1;
    let foreign = VcqPresentation::new(digest, b"not a receipt".to_vec());
    assert_eq!(at(AUDIENCE, NOW, &foreign), backend("vcq-descriptor-digest-mismatch"));

    // The byte bound is checked BEFORE decoding.
    let mut small = Spec::bag();
    small.bytes = 64;
    let small = small.stored();
    let own = vcq::descriptor_digest(adapter().descriptor());
    let oversize = VcqPresentation::new(own, vec![b'x'; 33]);
    assert_eq!(
        verify(&small, &oversize, &store).unwrap_err().code(),
        ErrorCode::CapacityExceeded(CapacityBound::Backend {
            name: "vcq-presentation-bytes",
            requested: 65,
            ceiling: 64,
        })
    );
    let fits = VcqPresentation::new(own, vec![b'x'; 32]);
    assert_eq!(
        verify(&small, &fits, &store).unwrap_err().code(),
        backend("vcq-receipt-decoding")
    );

    let unconfigured = adapter()
        .verify(&request, &admission, &placeholder, &store)
        .unwrap_err();
    assert_eq!(unconfigured.code(), backend("vcq-verifier-context-missing"));
    let configured =
        Risc0AuthenticatedRdfV5::new(&embedded_authrdf_pin(), v5_guest(), fixture::policy())
            .unwrap()
            .with_verifier(id(AUDIENCE), || NOW);
    let error = configured
        .verify(&request, &admission, &placeholder, &store)
        .unwrap_err();
    assert_eq!(error.code(), backend("vcq-receipt-decoding"));
    assert_eq!(store.calls(), 0);
}

#[test]
fn fake_and_foreign_receipts_are_rejected_without_consumption() {
    let anchor = fixture::anchor(fixture::SALT);
    let request = Spec::bag().agreed(anchor).stored();
    let expected = vcq5::expected_v5_request(
        &request,
        adapter().descriptor(),
        adapter().policy(),
        Phase::Verify,
    )
    .unwrap();
    let witness = auth::Witness {
        request: expected,
        dataset: credentials(fixture::SALT),
    };
    // Native differential journal for the published vector; not a proof.
    let journal = auth::evaluate(&witness).expect("native V5 evaluation");
    auth::bind_journal(&journal, &witness.request).unwrap();
    assert_eq!(journal.result, Expect::Bag.result(), "hand-defined duplicate bag");
    assert_eq!(journal.provenance, auth::Provenance::VerifierAgreedAuthenticated);
    assert_eq!(journal.dataset_commitment, anchor);
    let bytes = journal_bytes(&journal);
    let own = vcq::descriptor_digest(adapter().descriptor());
    // Fake receipts claiming the V5 image and the exact V1-V3 image.
    for image in [embedded_authrdf_pin().image_id, method_id()] {
        let fake = serde_json::to_vec(&fixture::fake_receipt(image, bytes.clone())).unwrap();
        let presentation = VcqPresentation::new(own, fake);
        for store in [
            Store::default(),
            Store {
                broken: true,
                ..Store::default()
            },
        ] {
            let error = verify(&request, &presentation, &store).unwrap_err();
            assert_eq!(
                (error.class(), error.phase(), error.code()),
                (
                    FailureClass::Invalid,
                    Phase::Verify,
                    backend("vcq-proof-rejected")
                )
            );
            assert_eq!(store.calls(), 0, "a rejected proof never reaches the store");
        }
    }
    // The same bytes presented for the exact V3 adapter's descriptor.
    let exact = vcq::descriptor_digest(&vcq::descriptor(&embedded_pin()).unwrap());
    let fake = fixture::fake_receipt(embedded_authrdf_pin().image_id, bytes);
    let presentation = VcqPresentation::new(exact, serde_json::to_vec(&fake).unwrap());
    let store = Store::default();
    assert_eq!(
        verify(&request, &presentation, &store).unwrap_err().code(),
        backend("vcq-descriptor-digest-mismatch")
    );
    assert_eq!(store.calls(), 0);
}

#[test]
fn mismatched_guest_and_pin_are_rejected() {
    let exact = AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin())
        .expect("embedded exact guest matches its own pin");
    let error = Risc0AuthenticatedRdfV5::new(&embedded_authrdf_pin(), exact, fixture::policy())
        .unwrap_err();
    assert_eq!(
        (error.class(), error.phase(), error.code()),
        (
            FailureClass::Invalid,
            Phase::Request,
            backend("vcq-guest-pin-mismatch")
        )
    );
    let zero = ArtifactPin {
        sha256: [0; 32],
        image_id: [0; 8],
    };
    assert_eq!(
        vcq5::descriptor(&zero, &fixture::policy()).unwrap_err().code(),
        ErrorCode::ZeroDigest
    );
    // Guest bytes never become accepted under another artifact's pin.
    assert!(
        AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_authrdf_pin())
            .is_err()
    );
}

#[test]
fn native_oracle_matches_every_hand_defined_expectation() {
    // Native model only; these journals are never presented as proofs.
    let anchor = fixture::anchor(fixture::SALT);
    let cases = [
        (ResultContract::SelectBag, QueryForm::Select, fixture::SELECT_BAG, Expect::Bag),
        (ResultContract::AskBoolean, QueryForm::Ask, fixture::ASK_ISSUER, Expect::Ask(true)),
        (
            ResultContract::AskBoolean,
            QueryForm::Ask,
            fixture::ASK_OTHER_ISSUER,
            Expect::Ask(false),
        ),
        (ResultContract::GraphRdfc10, QueryForm::Construct, fixture::CONSTRUCT, Expect::Graph),
    ];
    for (contract, form, query, expect) in cases {
        for agreed in [true, false] {
            let mut spec = Spec::bag().with(contract, form, query);
            if agreed {
                spec = spec.agreed(anchor);
            }
            let request = spec.stored();
            let admission = adapter()
                .admit(request.requirements(), adapter().descriptor())
                .unwrap();
            adapter()
                .prepare(&request, &admission, credentials(fixture::SALT))
                .expect("prepared");
            let expected = vcq5::expected_v5_request(
                &request,
                adapter().descriptor(),
                adapter().policy(),
                Phase::Verify,
            )
            .unwrap();
            let journal = auth::evaluate(&auth::Witness {
                request: expected,
                dataset: credentials(fixture::SALT),
            })
            .expect("native V5 evaluation");
            assert_eq!(journal.result, expect.result(), "{query}");
        }
    }
}

//! [OPUS-5.5] Negotiation and method-contract tests for vcq draft 0.
//!
//! Every method and capability declaration here is a unit-test fixture. None
//! is a registered backend, and these tests generate and verify ZERO proofs.

use std::fmt::Debug;

use sparq_query_protocol::{
    admit, Admission, ArtifactIdentity, BindingRoute, Capabilities, CapabilitiesSpec,
    CapabilityTuple, CapacityBound, ChallengeConsumption, ChallengeOwner, ChallengePolicy,
    ClaimEvidence, Completeness, ComponentStatus, DatasetAssembly, Digest32, Enforcement,
    Enforcer, ErrorCode, EvaluationMode, FailureClass, HolderPolicy, Identifier,
    MethodDescriptor, Obligation, ObligationOutcome, Phase, PolicyField, PreparedWitness,
    ProtocolError, QueryMethod, QueryProfile, QueryRequirements, RequirementsSpec,
    ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, VerifiedClaim,
};

fn id(value: &str) -> Identifier {
    Identifier::new(value).expect("valid test identifier")
}

fn digest(byte: u8) -> Digest32 {
    Digest32::new([byte; 32]).expect("nonzero test digest")
}

fn descriptor(version: u32, vk: u8) -> MethodDescriptor {
    MethodDescriptor::new(
        id("urn:test:method:m"),
        version,
        id("urn:test:params:p"),
        digest(1),
        ArtifactIdentity::VerificationKey {
            vk_digest: digest(vk),
        },
        id("urn:test:backend:b"),
    )
    .expect("valid test descriptor")
}

fn profile() -> QueryProfile {
    QueryProfile {
        dialect: id("urn:test:dialect:d"),
        fragment: id("urn:test:fragment:bgp"),
    }
}

fn relation(name: &str) -> Enforcer {
    Enforcer::Relation(id(&format!("urn:test:relation:{name}")))
}

fn enforcing_all() -> Enforcement {
    Enforcement {
        authenticity: relation("r"),
        mapping: relation("r"),
        status: relation("r"),
        holder_binding: relation("r"),
        linking: relation("r"),
        query: relation("r"),
        anchor: Enforcer::HostPublic(id("urn:test:host:anchor")),
    }
}

/// Holder-declared scope with issuer-authenticated evidence and required status.
fn holder_authenticated() -> CapabilityTuple {
    CapabilityTuple {
        query_profile: profile(),
        contract: ResultContract::SelectDistinctSet,
        mode: EvaluationMode::SelectedSupport,
        authority: ScopeAuthority::HolderDeclared,
        source_evidence: SourceEvidence::IssuerAuthenticated,
        status: StatusPolicy::Required,
        holder: HolderPolicy::BearerAccepted,
        assembly: DatasetAssembly::UnionDefaultGraph,
        suite: Some(id("urn:test:suite:a")),
        mapping: id("urn:test:map:m"),
        linking: vec![id("urn:test:link:same-relation")],
        enforcement: enforcing_all(),
    }
}

/// Verifier-agreed scope with no source evidence and no status.
fn agreed_unauthenticated() -> CapabilityTuple {
    CapabilityTuple {
        authority: ScopeAuthority::VerifierAgreedAnchor,
        source_evidence: SourceEvidence::None,
        status: StatusPolicy::NotRequested,
        suite: None,
        ..holder_authenticated()
    }
}

fn capabilities(descriptor: MethodDescriptor, tuples: Vec<CapabilityTuple>) -> CapabilitiesSpec {
    CapabilitiesSpec {
        descriptor,
        status: ComponentStatus::ImplementedExperimental,
        adapter_available: true,
        tuples,
        ceilings: ResourceBounds::new(4, 4096).expect("nonzero ceilings"),
        challenge: ChallengePolicy {
            owner: ChallengeOwner::Method,
            consumption: ChallengeConsumption::BurnOnAttempt,
        },
        disclosure: vec![id("urn:test:disclosure:released-terms")],
    }
}

fn build(spec: CapabilitiesSpec) -> Capabilities {
    Capabilities::new(spec).expect("valid test capabilities")
}

fn holder_authenticated_request(methods: Vec<MethodDescriptor>) -> RequirementsSpec {
    RequirementsSpec {
        contract: ResultContract::SelectDistinctSet,
        mode: EvaluationMode::SelectedSupport,
        authority: ScopeAuthority::HolderDeclared,
        anchor: None,
        source_evidence: Some(SourceEvidence::IssuerAuthenticated),
        status: Some(StatusPolicy::Required),
        holder: Some(HolderPolicy::BearerAccepted),
        assembly: DatasetAssembly::UnionDefaultGraph,
        query_profile: profile(),
        accepted_suites: vec![id("urn:test:suite:a")],
        accepted_mappings: vec![id("urn:test:map:m")],
        accepted_linking: vec![id("urn:test:link:same-relation")],
        resources: ResourceBounds::new(4, 4096).expect("nonzero bounds"),
        methods,
    }
}

fn agreed_unauthenticated_request(methods: Vec<MethodDescriptor>) -> RequirementsSpec {
    RequirementsSpec {
        authority: ScopeAuthority::VerifierAgreedAnchor,
        anchor: Some(digest(9)),
        source_evidence: Some(SourceEvidence::None),
        status: Some(StatusPolicy::NotRequested),
        accepted_suites: vec![],
        ..holder_authenticated_request(methods)
    }
}

fn requirements(spec: RequirementsSpec) -> QueryRequirements {
    QueryRequirements::new(spec).expect("valid test requirements")
}

fn assert_rejects<T: Debug>(
    result: Result<T, ProtocolError>,
    class: FailureClass,
    phase: Phase,
    code: ErrorCode,
) {
    let error = result.expect_err("must reject");
    assert_eq!((error.class(), error.phase(), error.code()), (class, phase, code));
}

#[test]
fn admits_the_single_matching_tuple() {
    let selected = descriptor(2, 7);
    let caps = build(capabilities(
        selected.clone(),
        vec![holder_authenticated(), agreed_unauthenticated()],
    ));
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));
    let admission = admit(&request, &selected, &caps).expect("supported tuple admits");

    assert_eq!(admission.descriptor(), &selected);
    assert_eq!(admission.tuple(), &holder_authenticated());
    assert_eq!(admission.query_profile(), &profile());
    assert_eq!(admission.completeness(), Completeness::None);
    let owed: Vec<Obligation> = admission.obligations().iter().map(|(o, _)| *o).collect();
    assert_eq!(
        owed,
        [
            Obligation::Authenticity,
            Obligation::Mapping,
            Obligation::Status,
            Obligation::Linking,
            Obligation::Query,
        ]
    );
    assert!(admission.enforcer(Obligation::HolderBinding).is_none());
    assert!(admission.enforcer(Obligation::Anchor).is_none());
}

/// Guard test for the per-tuple match. Suggested mutation: delete the
/// `tuple.authority == self.authority` conjunct (or check each axis against
/// ANY tuple) in `QueryRequirements::matches`; this test must then fail.
#[test]
fn product_of_declared_tuples_is_not_authorized() {
    let selected = descriptor(2, 7);
    let caps = build(capabilities(
        selected.clone(),
        vec![holder_authenticated(), agreed_unauthenticated()],
    ));
    let each = [
        holder_authenticated_request(vec![selected.clone()]),
        agreed_unauthenticated_request(vec![selected.clone()]),
    ];
    for spec in each {
        admit(&requirements(spec), &selected, &caps).expect("each declared tuple admits");
    }

    // Verifier-agreed authority from one tuple with issuer-authenticated
    // evidence and required status from the other: never declared together.
    let mut mixed = holder_authenticated_request(vec![selected.clone()]);
    mixed.authority = ScopeAuthority::VerifierAgreedAnchor;
    mixed.anchor = Some(digest(9));
    assert_rejects(
        admit(&requirements(mixed), &selected, &caps),
        FailureClass::Unsupported,
        Phase::Admit,
        ErrorCode::TupleUnsupported,
    );
}

/// Guard test for the per-tuple query profile. Suggested mutation: check the
/// profile against ANY tuple (or drop the `tuple.query_profile` conjunct in
/// `QueryRequirements::matches`); this test must then fail.
#[test]
fn query_profile_does_not_cross_declared_tuples() {
    let selected = descriptor(2, 7);
    let optional = QueryProfile {
        fragment: id("urn:test:fragment:optional"),
        ..profile()
    };
    let agreed_optional = CapabilityTuple {
        query_profile: optional.clone(),
        ..agreed_unauthenticated()
    };
    let caps = build(capabilities(
        selected.clone(),
        vec![holder_authenticated(), agreed_optional],
    ));

    // Each declared (tuple, profile) pair admits and keeps its exact profile.
    let holder_bgp = holder_authenticated_request(vec![selected.clone()]);
    let admission = admit(&requirements(holder_bgp), &selected, &caps).expect("declared pair");
    assert_eq!(admission.query_profile(), &profile());
    let mut agreed_opt = agreed_unauthenticated_request(vec![selected.clone()]);
    agreed_opt.query_profile = optional.clone();
    let admission = admit(&requirements(agreed_opt), &selected, &caps).expect("declared pair");
    assert_eq!(admission.query_profile(), &optional);
    assert_eq!(admission.tuple().authority, ScopeAuthority::VerifierAgreedAnchor);

    // Both profiles are known, but only under the other tuple.
    let mut holder_opt = holder_authenticated_request(vec![selected.clone()]);
    holder_opt.query_profile = optional;
    let agreed_bgp = agreed_unauthenticated_request(vec![selected.clone()]);
    for crossed in [holder_opt, agreed_bgp] {
        assert_rejects(
            admit(&requirements(crossed), &selected, &caps),
            FailureClass::Unsupported,
            Phase::Admit,
            ErrorCode::TupleUnsupported,
        );
    }
}

#[test]
fn presentation_must_select_an_exactly_requested_descriptor() {
    let requested = descriptor(2, 7);
    for selected in [descriptor(3, 7), descriptor(1, 7), descriptor(2, 8)] {
        // Even a backend that implements the selected descriptor is not used.
        let caps = build(capabilities(selected.clone(), vec![holder_authenticated()]));
        let request = requirements(holder_authenticated_request(vec![requested.clone()]));
        assert_rejects(
            admit(&request, &selected, &caps),
            FailureClass::Invalid,
            Phase::Negotiation,
            ErrorCode::DescriptorNotRequested,
        );
    }
}

#[test]
fn backend_with_another_version_or_artifact_is_unavailable() {
    let requested = descriptor(2, 7);
    let request = requirements(holder_authenticated_request(vec![requested.clone()]));
    for local in [descriptor(1, 7), descriptor(3, 7), descriptor(2, 8)] {
        let caps = build(capabilities(local, vec![holder_authenticated()]));
        assert_rejects(
            admit(&request, &requested, &caps),
            FailureClass::Unsupported,
            Phase::Negotiation,
            ErrorCode::DescriptorUnavailable,
        );
    }
}

/// Guard test for enforcement. Suggested mutation: remove the
/// `enforcer.discharges(obligation)` check in `admit`; this test must fail.
#[test]
fn challenge_binding_is_not_enforcement() {
    let selected = descriptor(2, 7);
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));

    let mut challenge_only = holder_authenticated();
    challenge_only.enforcement.status = Enforcer::BoundOnly(BindingRoute::Challenge);
    let caps = build(capabilities(selected.clone(), vec![challenge_only]));
    assert_rejects(
        admit(&request, &selected, &caps),
        FailureClass::Unsupported,
        Phase::Admit,
        ErrorCode::UnenforcedObligation(Obligation::Status),
    );

    let mut public_host = holder_authenticated();
    public_host.enforcement.authenticity = Enforcer::HostPublic(id("urn:test:host:issuer-list"));
    let caps = build(capabilities(selected.clone(), vec![public_host]));
    assert_rejects(
        admit(&request, &selected, &caps),
        FailureClass::Unsupported,
        Phase::Admit,
        ErrorCode::UnenforcedObligation(Obligation::Authenticity),
    );
}

#[test]
fn unavailable_adapter_is_unsupported_whatever_the_component_status() {
    let selected = descriptor(2, 7);
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));
    let cases = [
        (ComponentStatus::ImplementedExperimental, false),
        (ComponentStatus::Proposed, true),
        (ComponentStatus::Planned, true),
    ];
    for (status, adapter_available) in cases {
        let mut spec = capabilities(selected.clone(), vec![holder_authenticated()]);
        spec.status = status;
        spec.adapter_available = adapter_available;
        assert_rejects(
            admit(&request, &selected, &build(spec)),
            FailureClass::Unsupported,
            Phase::Request,
            ErrorCode::AdapterUnavailable,
        );
    }
}

#[test]
fn capacity_names_the_exceeded_bound() {
    let selected = descriptor(2, 7);
    let caps = build(capabilities(selected.clone(), vec![holder_authenticated()]));

    let mut rows = holder_authenticated_request(vec![selected.clone()]);
    rows.resources = ResourceBounds::new(5, 4096).expect("nonzero bounds");
    let bound = CapacityBound::ReleasedRows {
        requested: 5,
        ceiling: 4,
    };
    assert_rejects(
        admit(&requirements(rows), &selected, &caps),
        FailureClass::Capacity,
        Phase::Admit,
        ErrorCode::CapacityExceeded(bound),
    );

    let mut bytes = holder_authenticated_request(vec![selected.clone()]);
    bytes.resources = ResourceBounds::new(4, 8192).expect("nonzero bounds");
    let bound = CapacityBound::PresentationBytes {
        requested: 8192,
        ceiling: 4096,
    };
    assert_rejects(
        admit(&requirements(bytes), &selected, &caps),
        FailureClass::Capacity,
        Phase::Admit,
        ErrorCode::CapacityExceeded(bound),
    );
}

#[test]
fn weaker_capability_never_serves_a_stronger_request() {
    let selected = descriptor(2, 7);
    let caps = build(capabilities(selected.clone(), vec![holder_authenticated()]));

    let mut exact = holder_authenticated_request(vec![selected.clone()]);
    exact.mode = EvaluationMode::ExactBounded;
    let mut re_attested = holder_authenticated_request(vec![selected.clone()]);
    re_attested.source_evidence = Some(SourceEvidence::ReAttested);
    let mut holder_bound = holder_authenticated_request(vec![selected.clone()]);
    holder_bound.holder = Some(HolderPolicy::Required);
    let mut other_suite = holder_authenticated_request(vec![selected.clone()]);
    other_suite.accepted_suites = vec![id("urn:test:suite:b")];

    for spec in [exact, re_attested, holder_bound, other_suite] {
        assert_rejects(
            admit(&requirements(spec), &selected, &caps),
            FailureClass::Unsupported,
            Phase::Admit,
            ErrorCode::TupleUnsupported,
        );
    }
}

#[test]
fn ambiguous_tuples_fail_closed() {
    let selected = descriptor(2, 7);
    let suite_b = CapabilityTuple {
        suite: Some(id("urn:test:suite:b")),
        ..holder_authenticated()
    };
    let caps = build(capabilities(
        selected.clone(),
        vec![holder_authenticated(), suite_b],
    ));

    let mut both = holder_authenticated_request(vec![selected.clone()]);
    both.accepted_suites = vec![id("urn:test:suite:a"), id("urn:test:suite:b")];
    assert_rejects(
        admit(&requirements(both), &selected, &caps),
        FailureClass::Unsupported,
        Phase::Admit,
        ErrorCode::TupleAmbiguous,
    );

    let only_a = requirements(holder_authenticated_request(vec![selected.clone()]));
    let admission = admit(&only_a, &selected, &caps).expect("one tuple matches");
    assert_eq!(admission.tuple().suite, Some(id("urn:test:suite:a")));
}

#[test]
fn unsupported_query_profile_is_rejected_at_admit() {
    let selected = descriptor(2, 7);
    let caps = build(capabilities(selected.clone(), vec![holder_authenticated()]));
    let mut spec = holder_authenticated_request(vec![selected.clone()]);
    spec.query_profile.fragment = id("urn:test:fragment:optional");
    assert_rejects(
        admit(&requirements(spec), &selected, &caps),
        FailureClass::Unsupported,
        Phase::Admit,
        ErrorCode::QueryProfileUnsupported,
    );
}

#[test]
fn request_validation_fails_closed() {
    let selected = descriptor(2, 7);
    let base = || holder_authenticated_request(vec![selected.clone()]);
    let invalid = |spec, code| {
        assert_rejects(QueryRequirements::new(spec), FailureClass::Invalid, Phase::Request, code);
    };

    let mut spec = base();
    spec.status = None;
    invalid(spec, ErrorCode::MissingPolicy(PolicyField::Status));
    let mut spec = base();
    spec.source_evidence = None;
    invalid(spec, ErrorCode::MissingPolicy(PolicyField::SourceEvidence));
    let mut spec = base();
    spec.holder = None;
    invalid(spec, ErrorCode::MissingPolicy(PolicyField::Holder));
    let mut spec = base();
    spec.methods = vec![];
    invalid(spec, ErrorCode::EmptyMethodList);
    let mut spec = base();
    spec.methods = vec![selected.clone(), descriptor(3, 7), selected.clone()];
    invalid(spec, ErrorCode::DuplicateDescriptor);
    let mut spec = base();
    spec.authority = ScopeAuthority::VerifierAgreedAnchor;
    invalid(spec, ErrorCode::AnchorMismatch);
    let mut spec = base();
    spec.anchor = Some(digest(9));
    invalid(spec, ErrorCode::AnchorMismatch);
    let mut spec = base();
    spec.accepted_suites = vec![];
    invalid(spec, ErrorCode::EmptyAcceptedSet);
    let mut spec = base();
    spec.accepted_linking = vec![];
    invalid(spec, ErrorCode::EmptyAcceptedSet);

    for contract in [ResultContract::SelectBag, ResultContract::AskBoolean] {
        let mut spec = base();
        spec.contract = contract;
        assert_rejects(
            QueryRequirements::new(spec),
            FailureClass::Unsupported,
            Phase::Request,
            ErrorCode::ContractModeMismatch,
        );
    }
}

#[test]
fn malformed_identifiers_versions_digests_and_bounds_are_invalid() {
    let long = format!("urn:{}", "x".repeat(300));
    for bad in [
        "", "no-colon", ":x", "urn:", "urn:a b", "urn:*", "urn:a?", "1urn:x", long.as_str(),
    ] {
        assert_rejects(
            Identifier::new(bad),
            FailureClass::Invalid,
            Phase::Request,
            ErrorCode::MalformedIdentifier,
        );
    }
    assert_eq!(id("urn:sparq:vcq:method:x").as_str(), "urn:sparq:vcq:method:x");

    let zero_version = MethodDescriptor::new(
        id("urn:test:method:m"),
        0,
        id("urn:test:params:p"),
        digest(1),
        ArtifactIdentity::VerificationKey { vk_digest: digest(7) },
        id("urn:test:backend:b"),
    );
    let invalid = (FailureClass::Invalid, Phase::Request);
    assert_rejects(zero_version, invalid.0, invalid.1, ErrorCode::ZeroVersion);
    assert_rejects(Digest32::new([0; 32]), invalid.0, invalid.1, ErrorCode::ZeroDigest);
    assert_rejects(ResourceBounds::new(0, 1), invalid.0, invalid.1, ErrorCode::ZeroBound);
    assert_rejects(ResourceBounds::new(1, 0), invalid.0, invalid.1, ErrorCode::ZeroBound);
}

#[test]
fn malformed_capabilities_are_rejected() {
    let selected = descriptor(2, 7);
    let suite_without_evidence = CapabilityTuple {
        suite: Some(id("urn:test:suite:a")),
        ..agreed_unauthenticated()
    };
    let evidence_without_suite = CapabilityTuple {
        suite: None,
        ..holder_authenticated()
    };
    let selected_bag = CapabilityTuple {
        contract: ResultContract::SelectBag,
        ..holder_authenticated()
    };
    let unlinked = CapabilityTuple {
        linking: vec![],
        ..holder_authenticated()
    };
    let cases = [
        vec![],
        vec![suite_without_evidence],
        vec![evidence_without_suite],
        vec![selected_bag],
        vec![unlinked],
    ];
    for tuples in cases {
        let error = Capabilities::new(capabilities(selected.clone(), tuples))
            .expect_err("malformed capabilities");
        assert_eq!((error.class(), error.phase()), (FailureClass::Invalid, Phase::Negotiation));
        assert!(matches!(error.code(), ErrorCode::MalformedCapabilities(_)));
    }
}

/// Unit-test fixture: echoes one private row and generates ZERO proofs.
struct EchoMethod {
    capabilities: Capabilities,
    omit: Option<Obligation>,
}

struct SecretWitness(String);

impl QueryMethod for EchoMethod {
    type Request = str;
    type PrivateInputs = String;
    type Witness = SecretWitness;
    type Presentation = Vec<String>;
    type Output = Vec<String>;
    type ChallengeStore = ();

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn prepare(
        &self,
        _request: &str,
        admission: &Admission,
        private: String,
    ) -> Result<PreparedWitness<SecretWitness>, ProtocolError> {
        Ok(PreparedWitness::new(admission, SecretWitness(private)))
    }

    fn prove(
        &self,
        witness: PreparedWitness<SecretWitness>,
    ) -> Result<Self::Presentation, ProtocolError> {
        let SecretWitness(row) = witness.into_inner_for(self.descriptor())?;
        Ok(vec![row])
    }

    fn verify(
        &self,
        _request: &str,
        admission: &Admission,
        presentation: &Self::Presentation,
        _challenges: &(),
    ) -> Result<VerifiedClaim<Self::Output>, ProtocolError> {
        // A real backend would check a proof here. The fixture reports every
        // owed obligation (minus `omit`) plus one that was never owed.
        let established = admission
            .obligations()
            .iter()
            .map(|(obligation, _)| *obligation)
            .filter(|obligation| Some(*obligation) != self.omit)
            .chain([Obligation::HolderBinding])
            .collect();
        let evidence = ClaimEvidence {
            result: presentation.clone(),
            statement_digest: digest(5),
            established,
        };
        VerifiedClaim::new(admission, evidence)
    }
}

fn echo(selected: &MethodDescriptor, omit: Option<Obligation>) -> EchoMethod {
    EchoMethod {
        capabilities: build(capabilities(selected.clone(), vec![holder_authenticated()])),
        omit,
    }
}

#[test]
fn mock_method_round_trip_preserves_claim_metadata() {
    let selected = descriptor(2, 7);
    let method = echo(&selected, None);
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));
    let admission = method.admit(&request, &selected).expect("admits");

    let witness = method
        .prepare("request", &admission, "Alice-secret".to_owned())
        .expect("prepares");
    assert!(!format!("{witness:?}").contains("Alice-secret"));
    let presentation = method.prove(witness).expect("fixture proves nothing");
    let claim = method
        .verify("request", &admission, &presentation, &())
        .expect("fixture claim");

    assert_eq!(claim.result(), &vec!["Alice-secret".to_owned()]);
    assert_eq!(claim.descriptor(), &selected);
    assert_eq!(claim.scope().authority, ScopeAuthority::HolderDeclared);
    assert_eq!(claim.scope().source_evidence, SourceEvidence::IssuerAuthenticated);
    assert_eq!(claim.scope().completeness, Completeness::None);
    assert_eq!(
        claim.outcome(Obligation::Status),
        &ObligationOutcome::Established(relation("r"))
    );
    // Not owed by the request, so the backend's report is not evidence.
    assert_eq!(claim.outcome(Obligation::HolderBinding), &ObligationOutcome::NotEstablished);
    assert_eq!(claim.disclosure(), [id("urn:test:disclosure:released-terms")]);
}

#[test]
fn claim_missing_an_owed_obligation_is_invalid() {
    let selected = descriptor(2, 7);
    let method = echo(&selected, Some(Obligation::Status));
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));
    let admission = method.admit(&request, &selected).expect("admits");
    assert_rejects(
        method.verify("request", &admission, &vec!["row".to_owned()], &()),
        FailureClass::Invalid,
        Phase::Verify,
        ErrorCode::ObligationNotEstablished(Obligation::Status),
    );
}

#[test]
fn witness_prepared_for_another_descriptor_is_refused() {
    let selected = descriptor(2, 7);
    let method = echo(&selected, None);
    let other = echo(&descriptor(3, 7), None);
    let request = requirements(holder_authenticated_request(vec![selected.clone()]));
    let admission = method.admit(&request, &selected).expect("admits");
    let witness = method
        .prepare("request", &admission, "row".to_owned())
        .expect("prepares");
    assert_rejects(
        other.prove(witness),
        FailureClass::Invalid,
        Phase::Prove,
        ErrorCode::AdmissionMismatch,
    );
}

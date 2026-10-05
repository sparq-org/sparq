//! Method descriptors, capability tuples and enforcement routes (draft §6.1, §7.3).
//!
//! A [`MethodDescriptor`] pins one exact method version, parameter set,
//! artifact and backend. A method's [`Capabilities`] list whole
//! [`CapabilityTuple`]s: each tuple is one allowed combination, and admission
//! never combines axes from different tuples. Each tuple names an [`Enforcer`]
//! per [`Obligation`]; binding a field into a challenge
//! ([`Enforcer::BoundOnly`]) is recorded but never discharges an obligation.
//! [OPUS-5.5]

use crate::error::{ErrorCode, FailureClass, Phase, ProtocolError};
use crate::ids::{Digest32, Identifier, QueryProfile};

/// Pinned artifact identity loaded from verifier configuration (draft §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtifactIdentity {
    /// A circuit verification-key digest.
    VerificationKey {
        /// Verification-key digest.
        vk_digest: Digest32,
    },
    /// A zkVM guest artifact and image id.
    ZkvmGuest {
        /// Guest artifact digest.
        artifact_digest: Digest32,
        /// Guest image id.
        image_id: Digest32,
    },
    /// A composite proof's circuit and setup digests.
    Composite {
        /// Circuit digest.
        circuit_digest: Digest32,
        /// Setup digest.
        setup_digest: Digest32,
    },
}

/// Exact method tuple: id, version, parameters, artifact and backend pin.
///
/// Equality compares every field. There is no version range, parameter
/// wildcard or artifact fallback.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MethodDescriptor {
    // `pub(crate)` only so the local encoder can destructure exhaustively.
    pub(crate) method: Identifier,
    pub(crate) version: u32,
    pub(crate) parameter_set: Identifier,
    pub(crate) parameter_digest: Digest32,
    pub(crate) artifact: ArtifactIdentity,
    pub(crate) backend: Identifier,
}

impl MethodDescriptor {
    /// Builds a descriptor from configured, already validated parts.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with [`ErrorCode::ZeroVersion`]
    /// when `version` is zero.
    pub fn new(
        method: Identifier,
        version: u32,
        parameter_set: Identifier,
        parameter_digest: Digest32,
        artifact: ArtifactIdentity,
        backend: Identifier,
    ) -> Result<Self, ProtocolError> {
        if version == 0 {
            return Err(ProtocolError::new(
                FailureClass::Invalid,
                Phase::Request,
                ErrorCode::ZeroVersion,
            ));
        }
        Ok(Self {
            method,
            version,
            parameter_set,
            parameter_digest,
            artifact,
            backend,
        })
    }

    /// Returns the method id.
    #[must_use]
    pub fn method(&self) -> &Identifier {
        &self.method
    }

    /// Returns the method version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Returns the parameter-set id.
    #[must_use]
    pub fn parameter_set(&self) -> &Identifier {
        &self.parameter_set
    }

    /// Returns the configured parameter digest.
    #[must_use]
    pub fn parameter_digest(&self) -> &Digest32 {
        &self.parameter_digest
    }

    /// Returns the pinned artifact identity.
    #[must_use]
    pub fn artifact(&self) -> &ArtifactIdentity {
        &self.artifact
    }

    /// Returns the backend pin (toolchain or prover build).
    #[must_use]
    pub fn backend(&self) -> &Identifier {
        &self.backend
    }
}

/// Result semantics (draft §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResultContract {
    /// SELECT with set semantics.
    SelectDistinctSet,
    /// SELECT with multiset semantics.
    SelectBag,
    /// SELECT with outer order and slicing.
    SelectSequence,
    /// ASK returning `true` or `false`.
    AskBoolean,
    /// ASK that can only establish `true`.
    AskTrueOnly,
    /// CONSTRUCT or DESCRIBE as an RDFC-1.0 canonical graph.
    GraphRdfc10,
}

impl ResultContract {
    /// Reports whether this contract is defined under `mode` (draft §5.2).
    ///
    /// Selected support asserts no completeness, so it serves only distinct
    /// sets and true-only ASK; the other contracts need exact evaluation.
    #[must_use]
    pub fn supports_mode(self, mode: EvaluationMode) -> bool {
        match self {
            Self::SelectDistinctSet => true,
            Self::AskTrueOnly => mode == EvaluationMode::SelectedSupport,
            Self::SelectBag | Self::SelectSequence | Self::AskBoolean | Self::GraphRdfc10 => {
                mode == EvaluationMode::ExactBounded
            }
        }
    }
}

/// Evaluation mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvaluationMode {
    /// Each released answer is supported; no completeness.
    SelectedSupport,
    /// The full bounded result relative to scope.
    ExactBounded,
}

/// Who chose the dataset (draft §5.3); orthogonal to [`SourceEvidence`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScopeAuthority {
    /// The verifier accepted a dataset commitment before the request.
    VerifierAgreedAnchor,
    /// The holder chose the inputs at presentation time.
    HolderDeclared,
}

/// What authenticates the inputs (draft §5.3); orthogonal to authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceEvidence {
    /// Nothing authenticates the inputs.
    None,
    /// A trusted issuer's suite under a verifier-authorized key.
    IssuerAuthenticated,
    /// A trusted importer's re-signature; names the importer, not the issuer.
    ReAttested,
}

/// Credential status policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusPolicy {
    /// Status must be checked within the verifier's window.
    Required,
    /// Status is explicitly not requested.
    NotRequested,
}

/// Holder-binding policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HolderPolicy {
    /// Proof of possession of an issuer-bound holder key is required.
    Required,
    /// Bearer presentation is explicitly accepted.
    BearerAccepted,
}

/// Dataset assembly (draft §4.2 rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DatasetAssembly {
    /// All triples in the default graph, blank nodes kept apart.
    UnionDefaultGraph,
    /// One named graph per credential under an opaque name.
    CredentialNamedGraphs,
    /// Exact source N-Quads bytes with an exact graph-name catalog.
    ///
    /// [OPUS-5.5] Not a credential assembly: the dataset is the source bytes as
    /// given, not graphs built from imported credentials. The choice says
    /// nothing about authenticity, which [`SourceEvidence`] states separately.
    ExactSourceCatalog,
}

/// Completeness a claim carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Completeness {
    /// No absence or completeness is implied.
    None,
    /// Complete relative to the stated scope only.
    RelativeToScope,
}

impl EvaluationMode {
    /// Returns the completeness this mode can carry.
    #[must_use]
    pub fn completeness(self) -> Completeness {
        match self {
            Self::SelectedSupport => Completeness::None,
            Self::ExactBounded => Completeness::RelativeToScope,
        }
    }
}

/// Component source status from the registry; never evidence of an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentStatus {
    /// Component source exists behind an opt-in feature or detached workspace.
    ImplementedExperimental,
    /// Executable experiment with a fixed relation.
    StandalonePrototype,
    /// Verification delegated to a component sparq does not supply.
    DelegationSeam,
    /// Existing verifier path outside this protocol.
    ExistingNotAdapted,
    /// Named in design records; not implemented.
    Planned,
    /// Introduced by the draft; not implemented.
    Proposed,
}

/// How a statement field reaches the proof (draft §7.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingRoute {
    /// A public input.
    Proved,
    /// Inside a proved output.
    Journaled,
    /// Selects the verification key or program.
    Derived,
    /// Through the request-derived challenge.
    Challenge,
    /// Checked only against the stored request.
    Host,
}

/// Obligation a presentation or request owes (draft §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Obligation {
    /// Issuer or re-attester authenticity of each credential.
    Authenticity,
    /// Authenticated bytes map to the evaluated RDF terms.
    Mapping,
    /// Credential status within the verifier window.
    Status,
    /// Holder proof of possession.
    HolderBinding,
    /// Shared hidden values across obligations are the same values.
    Linking,
    /// The result meets the result contract for the query over scope.
    Query,
    /// The evaluated dataset equals the verifier-agreed anchor.
    Anchor,
}

impl Obligation {
    /// Every obligation, in a fixed order.
    pub const ALL: [Self; 7] = [
        Self::Authenticity,
        Self::Mapping,
        Self::Status,
        Self::HolderBinding,
        Self::Linking,
        Self::Query,
        Self::Anchor,
    ];
}

/// What checks an obligation (draft §7.3).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Enforcer {
    /// Checked inside the named relation over the witness.
    Relation(Identifier),
    /// Host check whose hidden inputs arrive through an authenticated opening.
    HostWithOpening(Identifier),
    /// Host check over public values only.
    HostPublic(Identifier),
    /// The field is bound by this route but no predicate is enforced.
    BoundOnly(BindingRoute),
    /// Not enforced.
    Absent,
}

impl Enforcer {
    /// Reports whether this enforcer discharges `obligation`.
    ///
    /// A host check over public values can discharge only the anchor
    /// comparison; every other obligation concerns hidden values and needs a
    /// relation or an authenticated opening. Binding alone discharges nothing.
    #[must_use]
    pub fn discharges(&self, obligation: Obligation) -> bool {
        match self {
            Self::Relation(_) | Self::HostWithOpening(_) => true,
            Self::HostPublic(_) => obligation == Obligation::Anchor,
            Self::BoundOnly(_) | Self::Absent => false,
        }
    }
}

/// One enforcer per [`Obligation`] for a capability tuple.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Enforcement {
    /// Enforcer for [`Obligation::Authenticity`].
    pub authenticity: Enforcer,
    /// Enforcer for [`Obligation::Mapping`].
    pub mapping: Enforcer,
    /// Enforcer for [`Obligation::Status`].
    pub status: Enforcer,
    /// Enforcer for [`Obligation::HolderBinding`].
    pub holder_binding: Enforcer,
    /// Enforcer for [`Obligation::Linking`].
    pub linking: Enforcer,
    /// Enforcer for [`Obligation::Query`].
    pub query: Enforcer,
    /// Enforcer for [`Obligation::Anchor`].
    pub anchor: Enforcer,
}

impl Enforcement {
    /// Returns the enforcer declared for `obligation`.
    #[must_use]
    pub fn route(&self, obligation: Obligation) -> &Enforcer {
        match obligation {
            Obligation::Authenticity => &self.authenticity,
            Obligation::Mapping => &self.mapping,
            Obligation::Status => &self.status,
            Obligation::HolderBinding => &self.holder_binding,
            Obligation::Linking => &self.linking,
            Obligation::Query => &self.query,
            Obligation::Anchor => &self.anchor,
        }
    }
}

/// One allowed combination of result, scope, policy and profile axes.
///
/// A method supporting two tuples authorizes exactly those two, never their
/// Cartesian product. The query profile is one of the axes: a profile declared
/// under one tuple does not extend to another. `suite` is present exactly
/// when `source_evidence` is not [`SourceEvidence::None`];
/// [`Capabilities::new`] checks this.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CapabilityTuple {
    /// Exact dialect and fragment pair supported under this tuple.
    pub query_profile: QueryProfile,
    /// Result contract.
    pub contract: ResultContract,
    /// Evaluation mode.
    pub mode: EvaluationMode,
    /// Scope authority.
    pub authority: ScopeAuthority,
    /// Source evidence.
    pub source_evidence: SourceEvidence,
    /// Status policy.
    pub status: StatusPolicy,
    /// Holder policy.
    pub holder: HolderPolicy,
    /// Dataset assembly.
    pub assembly: DatasetAssembly,
    /// Authenticity suite, when source evidence is authenticated.
    pub suite: Option<Identifier>,
    /// Mapping profile.
    pub mapping: Identifier,
    /// Linking profiles the proof uses; all must be accepted by the request.
    pub linking: Vec<Identifier>,
    /// Enforcer per obligation under this tuple.
    pub enforcement: Enforcement,
}

/// Verifier-side resource bounds or method ceilings; never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceBounds {
    // `pub(crate)` only so the local encoder can destructure exhaustively.
    pub(crate) released_rows: u32,
    pub(crate) presentation_bytes: u32,
}

impl ResourceBounds {
    /// Builds bounds on released rows and presentation bytes.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with [`ErrorCode::ZeroBound`]
    /// when either bound is zero.
    pub fn new(released_rows: u32, presentation_bytes: u32) -> Result<Self, ProtocolError> {
        if released_rows == 0 || presentation_bytes == 0 {
            return Err(ProtocolError::new(
                FailureClass::Invalid,
                Phase::Request,
                ErrorCode::ZeroBound,
            ));
        }
        Ok(Self {
            released_rows,
            presentation_bytes,
        })
    }

    /// Returns the released-row bound.
    #[must_use]
    pub fn released_rows(&self) -> u32 {
        self.released_rows
    }

    /// Returns the presentation-size bound in bytes.
    #[must_use]
    pub fn presentation_bytes(&self) -> u32 {
        self.presentation_bytes
    }
}

/// Party that consumes the challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChallengeOwner {
    /// The wrapped method consumes it; the adapter must not consume again.
    Method,
    /// The adapter consumes it.
    Adapter,
}

/// When the challenge owner consumes the challenge (draft §6.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChallengeConsumption {
    /// Consumed before proof checking; failures also spend it.
    BurnOnAttempt,
    /// Consumed after every other check; failures leave it retryable.
    ConsumeOnSuccess,
}

/// Declared challenge owner and consumption policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChallengePolicy {
    /// Consuming party.
    pub owner: ChallengeOwner,
    /// Consumption point.
    pub consumption: ChallengeConsumption,
}

/// Unvalidated capability declaration passed to [`Capabilities::new`].
#[derive(Clone, Debug)]
pub struct CapabilitiesSpec {
    /// Exact descriptor this backend implements.
    pub descriptor: MethodDescriptor,
    /// Registry component status.
    pub status: ComponentStatus,
    /// Whether an executable vcq adapter exists for this descriptor.
    pub adapter_available: bool,
    /// Allowed capability tuples, each with its own query profile.
    pub tuples: Vec<CapabilityTuple>,
    /// Resource ceilings.
    pub ceilings: ResourceBounds,
    /// Challenge owner and policy.
    pub challenge: ChallengePolicy,
    /// What a successful presentation discloses.
    pub disclosure: Vec<Identifier>,
}

/// Validated capabilities of one exact method descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    descriptor: MethodDescriptor,
    status: ComponentStatus,
    adapter_available: bool,
    tuples: Vec<CapabilityTuple>,
    ceilings: ResourceBounds,
    challenge: ChallengePolicy,
    disclosure: Vec<Identifier>,
}

impl Capabilities {
    /// Validates a capability declaration.
    ///
    /// This checks structure only. The declaration is trusted local
    /// configuration; validating it does not show that the backend enforces
    /// what it declares.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Negotiation`] with
    /// [`ErrorCode::MalformedCapabilities`] when there are no tuples, a
    /// tuple's contract is undefined under its mode, a tuple's suite presence
    /// disagrees with its source evidence, or a tuple names no linking profile.
    pub fn new(spec: CapabilitiesSpec) -> Result<Self, ProtocolError> {
        let malformed = |why: &'static str| {
            Err(ProtocolError::new(
                FailureClass::Invalid,
                Phase::Negotiation,
                ErrorCode::MalformedCapabilities(why),
            ))
        };
        if spec.tuples.is_empty() {
            return malformed("no capability tuples");
        }
        for tuple in &spec.tuples {
            if !tuple.contract.supports_mode(tuple.mode) {
                return malformed("contract undefined under mode");
            }
            if (tuple.source_evidence == SourceEvidence::None) != tuple.suite.is_none() {
                return malformed("suite presence disagrees with source evidence");
            }
            if tuple.linking.is_empty() {
                return malformed("tuple names no linking profile");
            }
        }
        Ok(Self {
            descriptor: spec.descriptor,
            status: spec.status,
            adapter_available: spec.adapter_available,
            tuples: spec.tuples,
            ceilings: spec.ceilings,
            challenge: spec.challenge,
            disclosure: spec.disclosure,
        })
    }

    /// Returns the exact descriptor these capabilities describe.
    #[must_use]
    pub fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Returns the registry component status.
    #[must_use]
    pub fn status(&self) -> ComponentStatus {
        self.status
    }

    /// Reports whether this descriptor may execute through vcq.
    ///
    /// True only when an adapter is available and the component is
    /// implemented; component status alone never suffices.
    #[must_use]
    pub fn is_executable(&self) -> bool {
        self.adapter_available && self.status == ComponentStatus::ImplementedExperimental
    }

    /// Returns the allowed capability tuples, each with its query profile.
    #[must_use]
    pub fn tuples(&self) -> &[CapabilityTuple] {
        &self.tuples
    }

    /// Returns the resource ceilings.
    #[must_use]
    pub fn ceilings(&self) -> ResourceBounds {
        self.ceilings
    }

    /// Returns the challenge owner and policy.
    #[must_use]
    pub fn challenge(&self) -> ChallengePolicy {
        self.challenge
    }

    /// Returns the disclosure inventory.
    #[must_use]
    pub fn disclosure(&self) -> &[Identifier] {
        &self.disclosure
    }
}

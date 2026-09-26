//! Verifier-owned requirements and deterministic, fail-closed admission (draft §5, §6.2).
//!
//! [`QueryRequirements`] is the negotiation subset of the draft §5.1 request:
//! result, scope, policy, profile, resource and exact-method fields. It omits
//! the query bytes, challenge, audience and validity window, which have no wire
//! encoding yet and stay in the adapter's own request type.
//!
//! [`admit`] is a pure function of the requirements, the descriptor a
//! presentation selected and the local backend's [`Capabilities`]. It has no
//! fallback, performs no lookup and reads no result. It checks trusted local
//! capability DECLARATIONS against the requirements; it runs no enforcer and
//! cannot show that any declared check exists or ran. [`Admission`] has
//! private fields and is built only by [`admit`], which guarantees only that
//! the value passed this structural admission. It is not an unforgeable
//! security token: anyone can declare local capabilities and admit against
//! them. [OPUS-5.5]

use crate::descriptor::{
    Capabilities, CapabilityTuple, ChallengePolicy, Completeness, DatasetAssembly, Enforcer,
    EvaluationMode, HolderPolicy, MethodDescriptor, Obligation, ResourceBounds, ResultContract,
    ScopeAuthority, SourceEvidence, StatusPolicy,
};
use crate::error::{CapacityBound, ErrorCode, FailureClass, Phase, PolicyField, ProtocolError};
use crate::ids::{Digest32, Identifier, QueryProfile};

/// Unvalidated verifier requirements passed to [`QueryRequirements::new`].
///
/// `source_evidence`, `status` and `holder` are options only so that an
/// omitted policy is reported as `invalid` rather than defaulted.
#[derive(Clone, Debug)]
pub struct RequirementsSpec {
    /// Result contract.
    pub contract: ResultContract,
    /// Evaluation mode.
    pub mode: EvaluationMode,
    /// Scope authority.
    pub authority: ScopeAuthority,
    /// Dataset anchor; present exactly under anchor authority.
    pub anchor: Option<Digest32>,
    /// Source evidence; no default.
    pub source_evidence: Option<SourceEvidence>,
    /// Status policy; no default.
    pub status: Option<StatusPolicy>,
    /// Holder policy; no default.
    pub holder: Option<HolderPolicy>,
    /// Dataset assembly.
    pub assembly: DatasetAssembly,
    /// Pinned dialect and admitted fragment.
    pub query_profile: QueryProfile,
    /// Accepted authenticity suites.
    pub accepted_suites: Vec<Identifier>,
    /// Accepted mapping profiles.
    pub accepted_mappings: Vec<Identifier>,
    /// Accepted linking profiles.
    pub accepted_linking: Vec<Identifier>,
    /// Verifier resource bounds.
    pub resources: ResourceBounds,
    /// Exact acceptable descriptors, preference-ordered.
    pub methods: Vec<MethodDescriptor>,
}

/// Validated, verifier-owned negotiation requirements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRequirements {
    contract: ResultContract,
    mode: EvaluationMode,
    authority: ScopeAuthority,
    anchor: Option<Digest32>,
    source_evidence: SourceEvidence,
    status: StatusPolicy,
    holder: HolderPolicy,
    assembly: DatasetAssembly,
    query_profile: QueryProfile,
    accepted_suites: Vec<Identifier>,
    accepted_mappings: Vec<Identifier>,
    accepted_linking: Vec<Identifier>,
    resources: ResourceBounds,
    methods: Vec<MethodDescriptor>,
}

fn reject(class: FailureClass, phase: Phase, code: ErrorCode) -> ProtocolError {
    ProtocolError::new(class, phase, code)
}

impl QueryRequirements {
    /// Validates verifier requirements.
    ///
    /// # Errors
    /// At [`Phase::Request`]: `invalid` for a missing policy, an empty or
    /// duplicated method list, an anchor that disagrees with the authority,
    /// or an empty accepted mapping, linking or (for authenticated evidence)
    /// suite set; `unsupported` when the contract is undefined under the mode.
    pub fn new(spec: RequirementsSpec) -> Result<Self, ProtocolError> {
        let invalid = |code| reject(FailureClass::Invalid, Phase::Request, code);
        let missing = |field| invalid(ErrorCode::MissingPolicy(field));
        let source_evidence = spec
            .source_evidence
            .ok_or_else(|| missing(PolicyField::SourceEvidence))?;
        let status = spec.status.ok_or_else(|| missing(PolicyField::Status))?;
        let holder = spec.holder.ok_or_else(|| missing(PolicyField::Holder))?;
        if spec.methods.is_empty() {
            return Err(invalid(ErrorCode::EmptyMethodList));
        }
        for (index, method) in spec.methods.iter().enumerate() {
            if spec.methods[..index].contains(method) {
                return Err(invalid(ErrorCode::DuplicateDescriptor));
            }
        }
        if !spec.contract.supports_mode(spec.mode) {
            return Err(reject(
                FailureClass::Unsupported,
                Phase::Request,
                ErrorCode::ContractModeMismatch,
            ));
        }
        if (spec.authority == ScopeAuthority::VerifierAgreedAnchor) != spec.anchor.is_some() {
            return Err(invalid(ErrorCode::AnchorMismatch));
        }
        let suites_missing =
            source_evidence != SourceEvidence::None && spec.accepted_suites.is_empty();
        if suites_missing || spec.accepted_mappings.is_empty() || spec.accepted_linking.is_empty()
        {
            return Err(invalid(ErrorCode::EmptyAcceptedSet));
        }
        Ok(Self {
            contract: spec.contract,
            mode: spec.mode,
            authority: spec.authority,
            anchor: spec.anchor,
            source_evidence,
            status,
            holder,
            assembly: spec.assembly,
            query_profile: spec.query_profile,
            accepted_suites: spec.accepted_suites,
            accepted_mappings: spec.accepted_mappings,
            accepted_linking: spec.accepted_linking,
            resources: spec.resources,
            methods: spec.methods,
        })
    }

    /// Returns the obligations these requirements owe, in [`Obligation::ALL`] order.
    ///
    /// Mapping, linking and query are always owed. Authenticity is owed unless
    /// source evidence is `none`, status and holder binding when required, and
    /// the anchor comparison under anchor authority.
    #[must_use]
    pub fn required_obligations(&self) -> Vec<Obligation> {
        Obligation::ALL
            .into_iter()
            .filter(|obligation| match obligation {
                Obligation::Authenticity => self.source_evidence != SourceEvidence::None,
                Obligation::Status => self.status == StatusPolicy::Required,
                Obligation::HolderBinding => self.holder == HolderPolicy::Required,
                Obligation::Anchor => self.authority == ScopeAuthority::VerifierAgreedAnchor,
                Obligation::Mapping | Obligation::Linking | Obligation::Query => true,
            })
            .collect()
    }

    /// Returns the exact acceptable descriptors, preference-ordered.
    #[must_use]
    pub fn methods(&self) -> &[MethodDescriptor] {
        &self.methods
    }

    /// Returns the verifier resource bounds.
    #[must_use]
    pub fn resources(&self) -> ResourceBounds {
        self.resources
    }

    /// Returns the dataset anchor, present under anchor authority only.
    #[must_use]
    pub fn anchor(&self) -> Option<&Digest32> {
        self.anchor.as_ref()
    }

    fn matches(&self, tuple: &CapabilityTuple) -> bool {
        // Every axis is compared against ONE tuple. Checking each axis against
        // any tuple would authorize the Cartesian product of declared tuples.
        // The query profile is one of those axes.
        tuple.query_profile == self.query_profile
            && tuple.contract == self.contract
            && tuple.mode == self.mode
            && tuple.authority == self.authority
            && tuple.source_evidence == self.source_evidence
            && tuple.status == self.status
            && tuple.holder == self.holder
            && tuple.assembly == self.assembly
            && tuple
                .suite
                .as_ref()
                .is_none_or(|suite| self.accepted_suites.contains(suite))
            && self.accepted_mappings.contains(&tuple.mapping)
            && tuple
                .linking
                .iter()
                .all(|link| self.accepted_linking.contains(link))
    }
}

/// Result of successful admission: the selected descriptor and owed obligations.
///
/// Admission is negotiation over public data and trusted local declarations.
/// It establishes no obligation, runs no enforcer, verifies nothing
/// cryptographically and carries no result. Its private fields ensure only
/// that it came from [`admit`], not that the declarations it was admitted
/// against are honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    descriptor: MethodDescriptor,
    tuple: CapabilityTuple,
    anchor: Option<Digest32>,
    obligations: Vec<(Obligation, Enforcer)>,
    resources: ResourceBounds,
    challenge: ChallengePolicy,
    disclosure: Vec<Identifier>,
}

impl Admission {
    /// Returns the selected exact descriptor.
    #[must_use]
    pub fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Returns the single capability tuple that matched.
    #[must_use]
    pub fn tuple(&self) -> &CapabilityTuple {
        &self.tuple
    }

    /// Returns the exact query profile of the matched tuple.
    #[must_use]
    pub fn query_profile(&self) -> &QueryProfile {
        &self.tuple.query_profile
    }

    /// Returns the dataset anchor the verifier supplied, if any.
    #[must_use]
    pub fn anchor(&self) -> Option<&Digest32> {
        self.anchor.as_ref()
    }

    /// Returns each owed obligation with the enforcer that must discharge it.
    #[must_use]
    pub fn obligations(&self) -> &[(Obligation, Enforcer)] {
        &self.obligations
    }

    /// Returns the enforcer owed for `obligation`, or `None` if it is not owed.
    #[must_use]
    pub fn enforcer(&self, obligation: Obligation) -> Option<&Enforcer> {
        self.obligations
            .iter()
            .find(|(owed, _)| *owed == obligation)
            .map(|(_, enforcer)| enforcer)
    }

    /// Returns the verifier's resource bounds, which fit the method ceilings.
    #[must_use]
    pub fn resources(&self) -> ResourceBounds {
        self.resources
    }

    /// Returns the declared challenge owner and consumption policy.
    #[must_use]
    pub fn challenge(&self) -> ChallengePolicy {
        self.challenge
    }

    /// Returns the method's disclosure inventory.
    #[must_use]
    pub fn disclosure(&self) -> &[Identifier] {
        &self.disclosure
    }

    /// Returns the completeness the admitted mode can carry.
    #[must_use]
    pub fn completeness(&self) -> Completeness {
        self.tuple.mode.completeness()
    }
}

/// Admits `selected` against `requirements` and the local `capabilities`.
///
/// Checks run in this order, each failing closed: the selected descriptor is
/// exactly one the request lists; the local backend implements exactly that
/// descriptor; an adapter is available; some tuple declares the query
/// profile; exactly one declared tuple matches every requested axis,
/// including the query profile; each owed obligation has a declared
/// discharging enforcer; the requested bounds fit the ceilings.
///
/// Every check reads declarations only. `admit` does not execute an enforcer
/// or show that a declared check exists; the adapter's `verify` owes that.
///
/// # Errors
/// - `invalid`, [`Phase::Negotiation`]: [`ErrorCode::DescriptorNotRequested`].
/// - `unsupported`, [`Phase::Negotiation`]: [`ErrorCode::DescriptorUnavailable`].
/// - `unsupported`, [`Phase::Request`]: [`ErrorCode::AdapterUnavailable`].
/// - `unsupported`, [`Phase::Admit`]: [`ErrorCode::QueryProfileUnsupported`],
///   [`ErrorCode::TupleUnsupported`], [`ErrorCode::TupleAmbiguous`] or
///   [`ErrorCode::UnenforcedObligation`].
/// - `capacity`, [`Phase::Admit`]: [`ErrorCode::CapacityExceeded`] naming the bound.
pub fn admit(
    requirements: &QueryRequirements,
    selected: &MethodDescriptor,
    capabilities: &Capabilities,
) -> Result<Admission, ProtocolError> {
    let unsupported = |phase, code| reject(FailureClass::Unsupported, phase, code);
    if !requirements.methods.contains(selected) {
        return Err(reject(
            FailureClass::Invalid,
            Phase::Negotiation,
            ErrorCode::DescriptorNotRequested,
        ));
    }
    if capabilities.descriptor() != selected {
        return Err(unsupported(Phase::Negotiation, ErrorCode::DescriptorUnavailable));
    }
    if !capabilities.is_executable() {
        return Err(unsupported(Phase::Request, ErrorCode::AdapterUnavailable));
    }
    // This only classifies the failure for a profile no tuple declares. It
    // authorizes nothing: a profile declared under another tuple still fails
    // the per-tuple match below as `TupleUnsupported`.
    if !capabilities
        .tuples()
        .iter()
        .any(|tuple| tuple.query_profile == requirements.query_profile)
    {
        return Err(unsupported(Phase::Admit, ErrorCode::QueryProfileUnsupported));
    }
    let mut matching = capabilities
        .tuples()
        .iter()
        .filter(|tuple| requirements.matches(tuple));
    let tuple = match (matching.next(), matching.next()) {
        (Some(tuple), None) => tuple,
        (None, _) => return Err(unsupported(Phase::Admit, ErrorCode::TupleUnsupported)),
        (Some(_), Some(_)) => return Err(unsupported(Phase::Admit, ErrorCode::TupleAmbiguous)),
    };
    let mut obligations = Vec::new();
    for obligation in requirements.required_obligations() {
        let enforcer = tuple.enforcement.route(obligation);
        if !enforcer.discharges(obligation) {
            return Err(unsupported(
                Phase::Admit,
                ErrorCode::UnenforcedObligation(obligation),
            ));
        }
        obligations.push((obligation, enforcer.clone()));
    }
    let (wanted, ceiling) = (requirements.resources, capabilities.ceilings());
    if wanted.released_rows() > ceiling.released_rows() {
        let bound = CapacityBound::ReleasedRows {
            requested: wanted.released_rows(),
            ceiling: ceiling.released_rows(),
        };
        return Err(ProtocolError::capacity(Phase::Admit, bound));
    }
    if wanted.presentation_bytes() > ceiling.presentation_bytes() {
        let bound = CapacityBound::PresentationBytes {
            requested: wanted.presentation_bytes(),
            ceiling: ceiling.presentation_bytes(),
        };
        return Err(ProtocolError::capacity(Phase::Admit, bound));
    }
    Ok(Admission {
        descriptor: selected.clone(),
        tuple: tuple.clone(),
        anchor: requirements.anchor,
        obligations,
        resources: wanted,
        challenge: capabilities.challenge(),
        disclosure: capabilities.disclosure().to_vec(),
    })
}

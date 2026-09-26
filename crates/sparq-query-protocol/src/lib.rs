//! Typed negotiation and method contract for Sparq credential query proofs.
//!
//! [OPUS-5.5] Experimental, unpublished, dependency-free contract layer for
//! vcq draft 0 (`research/vc-query-protocol.md`, registry
//! `research/vc-query-methods.json`). It contains shared types, the
//! [`QueryMethod`] trait and deterministic, fail-closed capability negotiation.
//! It contains no proof backend, no wire encoding, no canonicalization and no
//! cryptography, and **it proves no cryptographic claim**. Nothing here is
//! externally audited (sq-qhy4).
//!
//! # Mapping to the draft
//!
//! | Draft | Here |
//! |---|---|
//! | §5.1 request (negotiation subset) | [`RequirementsSpec`], [`QueryRequirements`] |
//! | §5.2 result contracts and modes | [`ResultContract`], [`EvaluationMode`] |
//! | §5.3 authority and source evidence | [`ScopeAuthority`], [`SourceEvidence`] |
//! | §6.1 descriptor and capabilities | [`MethodDescriptor`], [`Capabilities`], [`CapabilityTuple`] |
//! | §6.2 `admit` | [`admit`], [`Admission`] |
//! | §6.2 operations | [`QueryMethod`], [`PreparedWitness`] |
//! | §6.3 verified claim | [`VerifiedClaim`], [`ClaimEvidence`] |
//! | §6.4 failures | [`ProtocolError`], [`FailureClass`], [`Phase`], [`ErrorCode`] |
//! | §7.3 routes and enforcers | [`BindingRoute`], [`Enforcer`], [`Enforcement`] |
//!
//! The query bytes, base IRI, challenge, audience, validity window and status
//! window of §5.1 are NOT modelled: they need the §7.2 wire profile, which
//! does not exist. Adapters carry them in [`QueryMethod::Request`].
//!
//! # Negotiation rules
//!
//! - A descriptor matches only by exact equality of every field.
//! - A method declares whole [`CapabilityTuple`]s; admission needs exactly one
//!   tuple to match every requested axis, so declared tuples never combine.
//!   Each tuple carries its own [`QueryProfile`], and [`Admission::query_profile`]
//!   returns the matched one.
//! - [`admit`] checks trusted local capability DECLARATIONS. It runs no
//!   enforcer and cannot show that a declared check exists or ran.
//!   [`Admission`]'s private fields show only that it passed [`admit`]; anyone
//!   can declare local capabilities, so it is not an unforgeable token.
//! - Every owed [`Obligation`] needs an [`Enforcer`] that discharges it.
//!   [`Enforcer::BoundOnly`] (for example, a policy hashed into a challenge)
//!   and public host checks over hidden values discharge nothing.
//! - A descriptor is executable only when its adapter is available and its
//!   component is implemented ([`Capabilities::is_executable`]).
//!
//! # What adapters still owe
//!
//! An [`Admission`] and a [`VerifiedClaim`] certify no cryptographic fact.
//! A real adapter's `verify` must itself bind the request and result into the
//! proved statement, check audience and validity against the stored request,
//! use a named wire encoding and hash, atomically consume the challenge per
//! its declared owner and policy, check issuer key authorization from verifier
//! trust material (draft §4.2 rule 9), and link hidden witnesses across
//! obligations (§8). A digest an adapter supplies does not show that any
//! hidden predicate was checked.
//!
//! # Examples
//!
//! ```
//! use sparq_query_protocol::{
//!     admit, ArtifactIdentity, Capabilities, CapabilitiesSpec, CapabilityTuple,
//!     ChallengeConsumption, ChallengeOwner, ChallengePolicy, ComponentStatus,
//!     DatasetAssembly, Digest32, Enforcement, Enforcer, EvaluationMode, HolderPolicy,
//!     Identifier, MethodDescriptor, Obligation, ProtocolError, QueryProfile,
//!     QueryRequirements, RequirementsSpec, ResourceBounds, ResultContract,
//!     ScopeAuthority, SourceEvidence, StatusPolicy,
//! };
//!
//! # fn main() -> Result<(), ProtocolError> {
//! let id = Identifier::new;
//! let descriptor = MethodDescriptor::new(
//!     id("urn:example:method:exact")?,
//!     1,
//!     id("urn:example:params:default")?,
//!     Digest32::new([1; 32])?,
//!     ArtifactIdentity::ZkvmGuest {
//!         artifact_digest: Digest32::new([2; 32])?,
//!         image_id: Digest32::new([3; 32])?,
//!     },
//!     id("urn:example:backend:pinned")?,
//! )?;
//! let profile = QueryProfile {
//!     dialect: id("urn:example:dialect:1")?,
//!     fragment: id("urn:example:fragment:bgp")?,
//! };
//! let guest = Enforcer::Relation(id("urn:example:relation:guest")?);
//! // Illustrative declaration for this example only; no such adapter exists.
//! let capabilities = Capabilities::new(CapabilitiesSpec {
//!     descriptor: descriptor.clone(),
//!     status: ComponentStatus::ImplementedExperimental,
//!     adapter_available: true,
//!     tuples: vec![CapabilityTuple {
//!         query_profile: profile.clone(),
//!         contract: ResultContract::SelectBag,
//!         mode: EvaluationMode::ExactBounded,
//!         authority: ScopeAuthority::VerifierAgreedAnchor,
//!         source_evidence: SourceEvidence::None,
//!         status: StatusPolicy::NotRequested,
//!         holder: HolderPolicy::BearerAccepted,
//!         assembly: DatasetAssembly::UnionDefaultGraph,
//!         suite: None,
//!         mapping: id("urn:example:map:exact-bytes")?,
//!         linking: vec![id("urn:example:link:committed-dataset")?],
//!         enforcement: Enforcement {
//!             authenticity: Enforcer::Absent,
//!             mapping: guest.clone(),
//!             status: Enforcer::Absent,
//!             holder_binding: Enforcer::Absent,
//!             linking: guest.clone(),
//!             query: guest,
//!             anchor: Enforcer::HostPublic(id("urn:example:host:anchor-equality")?),
//!         },
//!     }],
//!     ceilings: ResourceBounds::new(4096, 1 << 20)?,
//!     challenge: ChallengePolicy {
//!         owner: ChallengeOwner::Method,
//!         consumption: ChallengeConsumption::ConsumeOnSuccess,
//!     },
//!     disclosure: vec![id("urn:example:disclosure:journal")?],
//! })?;
//! let requirements = QueryRequirements::new(RequirementsSpec {
//!     contract: ResultContract::SelectBag,
//!     mode: EvaluationMode::ExactBounded,
//!     authority: ScopeAuthority::VerifierAgreedAnchor,
//!     anchor: Some(Digest32::new([9; 32])?),
//!     source_evidence: Some(SourceEvidence::None),
//!     status: Some(StatusPolicy::NotRequested),
//!     holder: Some(HolderPolicy::BearerAccepted),
//!     assembly: DatasetAssembly::UnionDefaultGraph,
//!     query_profile: profile.clone(),
//!     accepted_suites: vec![],
//!     accepted_mappings: vec![id("urn:example:map:exact-bytes")?],
//!     accepted_linking: vec![id("urn:example:link:committed-dataset")?],
//!     resources: ResourceBounds::new(16, 65_536)?,
//!     methods: vec![descriptor.clone()],
//! })?;
//!
//! let admission = admit(&requirements, &descriptor, &capabilities)?;
//! assert_eq!(admission.query_profile(), &profile);
//! assert!(admission.enforcer(Obligation::Anchor).is_some());
//! assert!(admission.enforcer(Obligation::Authenticity).is_none());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod descriptor;
mod error;
mod ids;
mod method;
mod negotiate;

pub use descriptor::{
    ArtifactIdentity, BindingRoute, Capabilities, CapabilitiesSpec, CapabilityTuple,
    ChallengeConsumption, ChallengeOwner, ChallengePolicy, Completeness, ComponentStatus,
    DatasetAssembly, Enforcement, Enforcer, EvaluationMode, HolderPolicy, MethodDescriptor,
    Obligation, ResourceBounds, ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy,
};
pub use error::{
    BackendFailure, CapacityBound, ErrorCode, FailureClass, Phase, PolicyField, ProtocolError,
};
pub use ids::{Digest32, Identifier, QueryProfile, MAX_IDENTIFIER_LEN};
pub use method::{
    ClaimEvidence, ClaimScope, ObligationOutcome, PreparedWitness, QueryMethod, VerifiedClaim,
};
pub use negotiate::{admit, Admission, QueryRequirements, RequirementsSpec};

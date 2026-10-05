//! Typed negotiation and method contract for Sparq credential query proofs.
//!
//! [OPUS-5.5] Experimental, unpublished, dependency-free contract layer for
//! vcq draft 0 (`research/vc-query-protocol.md`, registry
//! `research/vc-query-methods.json`). It contains shared types, the
//! [`QueryMethod`] trait, deterministic, fail-closed capability negotiation,
//! a validated [`StoredRequest`], a named LOCAL structural byte encoding and
//! a shared [`ChallengeStore`] contract. It contains no proof backend, no
//! interoperable wire encoding or transport, no RDF or SPARQL
//! canonicalization, no query parser and no cryptography or hash, and **it
//! proves no cryptographic claim**. Nothing here is externally audited
//! (sq-qhy4).
//!
//! # Mapping to the draft
//!
//! | Draft | Here |
//! |---|---|
//! | §5.1 request (negotiation subset) | [`RequirementsSpec`], [`QueryRequirements`] |
//! | §5.1 request (stored subset) | [`StoredRequestSpec`], [`StoredRequest`], [`Challenge32`], [`BaseIri`], [`QueryForm`] |
//! | §6.4 challenge consumption | [`ChallengeStore`], [`consume_challenge`] |
//! | §7.2 (local stand-in only) | [`encode_stored_request`], [`encode_method_descriptor`] |
//! | §5.2 result contracts and modes | [`ResultContract`], [`EvaluationMode`] |
//! | §5.3 authority and source evidence | [`ScopeAuthority`], [`SourceEvidence`] |
//! | §6.1 descriptor and capabilities | [`MethodDescriptor`], [`Capabilities`], [`CapabilityTuple`] |
//! | §6.2 `admit` | [`admit`], [`Admission`] |
//! | §6.2 operations | [`QueryMethod`], [`PreparedWitness`] |
//! | §6.3 verified claim | [`VerifiedClaim`], [`ClaimEvidence`] |
//! | §6.4 failures | [`ProtocolError`], [`FailureClass`], [`Phase`], [`ErrorCode`] |
//! | §7.3 routes and enforcers | [`BindingRoute`], [`Enforcer`], [`Enforcement`] |
//!
//! [`StoredRequest`] models the query text, base IRI (shape only), original
//! challenge, audience, validity window, form and DESCRIBE policy of §5.1. It
//! is still a subset: issuer sets, status windows, trust-policy roots,
//! disclosure policy, the rest of the evaluation context and graph-size bounds
//! are not modelled, so not every credential policy is represented, and none
//! is enforced here. The local encoding ([`LOCAL_ENCODING_PROFILE`]) is
//! byte-exact over these typed values but is not the §7.2 wire profile.
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
//! proved statement, check audience and validity against the [`StoredRequest`],
//! name its hash over a named encoding, consume the ORIGINAL challenge once
//! through the verifier's shared [`ChallengeStore`] per its declared owner and
//! policy, check that the parsed query has the declared form and that the base
//! IRI is a supported IRI, check issuer key authorization from verifier
//! trust material (draft §4.2 rule 9), and link hidden witnesses across
//! obligations (§8). A digest an adapter supplies does not show that any
//! hidden predicate was checked.
//!
//! # Examples
//!
//! ```
//! use sparq_query_protocol::{
//!     admit, encode_stored_request, ArtifactIdentity, Capabilities, CapabilitiesSpec,
//!     CapabilityTuple, Challenge32, ChallengeConsumption, ChallengeOwner, ChallengePolicy,
//!     ComponentStatus, DatasetAssembly, Digest32, Enforcement, Enforcer, EvaluationMode,
//!     HolderPolicy, Identifier, MethodDescriptor, Obligation, ProtocolError, QueryForm,
//!     QueryProfile, QueryRequirements, RequirementsSpec, ResourceBounds, ResultContract,
//!     ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest, StoredRequestSpec,
//!     STORED_REQUEST_DOMAIN,
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
//!
//! // The verifier stores the full request subset and encodes it locally.
//! let stored = StoredRequest::new(StoredRequestSpec {
//!     requirements,
//!     query: "SELECT ?s WHERE { ?s ?p ?o }".to_owned(),
//!     challenge: Challenge32::new([5; 32])?,
//!     audience: id("urn:example:verifier")?,
//!     not_before: 1_800_000_000,
//!     not_after: 1_800_000_600,
//!     form: QueryForm::Select,
//!     base_iri: None,
//!     describe_policy: None,
//! })?;
//! assert!(encode_stored_request(&stored).starts_with(STORED_REQUEST_DOMAIN));
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod challenge;
mod descriptor;
mod encoding;
mod error;
mod ids;
mod method;
mod negotiate;
mod request;

pub use challenge::{consume_challenge, ChallengeOutcome, ChallengeStore, ChallengeStoreError};
pub use encoding::{
    encode_method_descriptor, encode_stored_request, LOCAL_ENCODING_PROFILE,
    METHOD_DESCRIPTOR_DOMAIN, STORED_REQUEST_DOMAIN,
};
pub use request::{
    BaseIri, Challenge32, QueryForm, StoredRequest, StoredRequestSpec, MAX_BASE_IRI_LEN,
    MAX_QUERY_LEN,
};

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

// zkp-14.6 optional vcq adapter over the V5 authenticated-RDF relation.
// Experimental source, not externally audited. At frozen source 7fe88955 one
// audited genuine receipt covers only the select-bag-verifier-agreed case.
// Rust guideline compliant 2026-02-21
//! `sparq-query-protocol` adapter over the issuer-authenticated RDF (V5) relation.
//!
//! [`Risc0AuthenticatedRdfV5`] implements [`QueryMethod`] for one exact
//! descriptor, `urn:sparq:vcq:method:risc0-authenticated-rdf` version 5. It is
//! built from three verifier-owned inputs, none of which a presentation can
//! select: an independently approved V5 [`ArtifactPin`], the [`AcceptedGuest`]
//! loaded under that pin, and an immutable [`Policy`] (the authorization table
//! plus the V3 evaluation capacities). Proving uses
//! [`crate::authenticated_rdf::prove_with_artifact`]; verification uses the
//! existing checked V5 path (Succinct receipt, dev mode disabled, journal
//! decoded only after the proof checks, then independent request binding).
//! Compiled only with the off-by-default `vcq-authenticated` feature.
//!
//! # What the relation checks
//!
//! Each private credential is a pair of N-Quads inputs (unsecured document and
//! proof configuration without `proofValue`) plus 64 raw Ed25519 signature
//! bytes. Inside the pinned V5 guest, per credential and within fixed bounds,
//! the relation parses both inputs (default graph only), canonicalizes each with
//! bounded RDFC-1.0/SHA-256 and reads every check from those canonical bytes:
//! one `DataIntegrityProof` node whose typed `cryptosuite` is `eddsa-rdfc-2022`,
//! one IRI `verificationMethod`, `proofPurpose assertionMethod`, and at most one
//! `created` in a restricted whole-second UTC profile (never compared with a
//! clock). The verification method must appear in the verifier's table and the
//! document's single issuer must equal that entry's pinned issuer. Strict
//! Ed25519 is then verified over `SHA-256(canonical config) ||
//! SHA-256(canonical document)`. The hashed canonical documents are unioned
//! into one default graph with per-credential blank-node scopes, and the V3
//! query is evaluated over that union. See
//! [`sparq_proved_evaluator_model::authenticated_rdf`] for the exact relation.
//!
//! This is a bounded canonical RDF profile of W3C `eddsa-rdfc-2022`, not a
//! Data Integrity processor. There is no JSON-LD expansion or context handling,
//! no `proofValue` multibase decoding, no DID or controller resolution, no
//! validity-period or `created` clock check, no credential status and no holder
//! binding. The verifier's table is the only trust input: the table entry's
//! `issuer` is the only issuer-to-key authorization, and there is no separate
//! controller field.
//!
//! # Capabilities
//!
//! Six whole tuples: result contracts `SelectBag`, `AskBoolean` and
//! `GraphRdfc10` (CONSTRUCT only), each under `VerifierAgreedAnchor` and
//! `HolderDeclared`. Every tuple is `ExactBounded`, query profile
//! ([`QUERY_DIALECT`], [`QUERY_FRAGMENT`]), `UnionDefaultGraph` assembly, source
//! evidence `IssuerAuthenticated` with suite [`SUITE`], status `NotRequested`,
//! holder `BearerAccepted`, mapping [`MAPPING_PROFILE`] and linking
//! [`LINKING_PROFILE`]. Authenticity, mapping, linking and query are enforced
//! by the guest relation [`RELATION`]. The anchor comparison is the public host
//! check [`ANCHOR_CHECK`], owed only under verifier-agreed authority; the guest
//! also rejects an unequal agreed commitment. Status and holder binding are
//! `Absent`. Requests for source evidence `None` or `ReAttested`, another
//! suite, mapping or linking profile, required status or holder binding,
//! credential named graphs or exact source catalogs, SELECT sequences, sets,
//! DESCRIBE or an explicit base IRI are rejected at admission or by the typed
//! query-shape check, before any proof work or challenge use.
//!
//! `Capabilities::is_executable` is true because this build contains the
//! adapter. That is an implementation declaration only. One independently
//! audited genuine receipt, at frozen source `7fe88955`, covers only the
//! verifier-agreed `SelectBag` tuple on one public synthetic fixture. The other
//! five tuples have not run, so the research registry keeps
//! `adapter_available: false` for this method; this code never reads the
//! registry.
//!
//! # Policy binding
//!
//! The descriptor's [`parameter_digest`] binds [`policy_digest`], the suite,
//! mapping, linking and relation identifiers and every fixed capacity.
//! [`policy_digest`] reuses the model's own request digest rather than a second
//! policy encoder: it hashes [`POLICY_DOMAIN`] and
//! `authenticated_rdf::request_digest` of a fixed sentinel request (version 5,
//! query [`POLICY_SENTINEL_QUERY`], holder-declared authority, nonce
//! [`POLICY_SENTINEL_NONCE`]) carrying the policy. With every other field
//! fixed, that digest depends only on the model's framed policy digest: the
//! suite and mapping profile tags, every fixed model bound, the serialized V3
//! evaluation policy and each table entry's issuer, verification method and key,
//! sorted by verification method. Table order is therefore not significant,
//! while every other field change changes the digest. An invalid policy (empty
//! or oversized table, invalid or oversized IRI, invalid or small-order key,
//! repeated verification method, or evaluation capacities outside the program
//! ceilings) is rejected.
//!
//! # Request binding
//!
//! The V5 nonce is [`derive_nonce`]: SHA-256 over [`NONCE_DOMAIN`], the
//! SHA-256 of the `local-struct-v1` stored-request bytes and the SHA-256 of the
//! `local-struct-v1` descriptor bytes. The V5 request both sides derive
//! ([`expected_v5_request`]) carries the verifier's own policy, the exact query,
//! the authority and anchor, and that nonce; its model request digest is
//! journaled by the guest. The original challenge, audience, validity window,
//! accepted suites, descriptor (and through it the policy) and query are thus
//! bound cryptographically. The verifier also checks audience and validity
//! against its own [`StoredRequest`] and clock. No policy is ever inferred from
//! a presentation.
//!
//! # Challenge and bounds
//!
//! As in [`crate::vcq`]: the method owns the challenge and consumes the
//! ORIGINAL [`StoredRequest::challenge`] once, through the shared
//! [`ChallengeStore`], after the presentation-byte bound, receipt decoding,
//! Succinct verification, request binding, anchor, provenance, result form and
//! released-row checks. Replay and store failure are classified from the typed
//! store outcome.
//!
//! # Limits
//!
//! Neither provenance implies wallet or world completeness: under
//! holder-declared authority the holder chooses which authenticated credentials
//! to include, and under verifier-agreed authority completeness is relative to
//! the agreed commitment only. Salt reuse makes commitments linkable.

use crate::authenticated_rdf::{prove_with_artifact, verify_checked};
use crate::vcq::{
    OriginalChallenge, ReleasedResult, VcqPresentation, check_size, check_verifier_context,
    checked_admission, descriptor_digest, fail, image_id_bytes, invalid, released_result,
    stored_request_digest, system_unix_seconds,
};
use crate::{AcceptedGuest, ArtifactPin, Presentation};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, Cryptosuite, Mapping, Policy, PrivateCredentials, Provenance,
};
use sparq_proved_evaluator_model::{DatasetAuthority, MAX_QUERY_BYTES, MAX_ROWS};
use sparq_query_protocol::{
    Admission, ArtifactIdentity, BackendFailure, Capabilities, CapabilitiesSpec, CapabilityTuple,
    ChallengeConsumption, ChallengeOwner, ChallengePolicy, ChallengeStore, ClaimEvidence,
    ComponentStatus, DatasetAssembly, Digest32, Enforcement, Enforcer, EvaluationMode,
    HolderPolicy, Identifier, MethodDescriptor, Obligation, Phase, PreparedWitness,
    ProtocolError, QueryMethod, QueryProfile, ResourceBounds, ResultContract, ScopeAuthority,
    SourceEvidence, StatusPolicy, StoredRequest, VerifiedClaim,
};
use std::fmt;
use std::path::PathBuf;

/// Method id of the issuer-authenticated RDF RISC Zero evaluator.
pub const METHOD_ID: &str = "urn:sparq:vcq:method:risc0-authenticated-rdf";
/// Method version; equals the V5 relation's wire version.
pub const METHOD_VERSION: u32 = 5;
const _: () = assert!(METHOD_VERSION == auth::VERSION);
/// Parameter-set id: a verifier-owned V5 policy, pinned by [`parameter_digest`].
pub const PARAMETER_SET: &str = "urn:sparq:vcq:params:risc0-authenticated-rdf:v5-verifier-policy";
/// Backend pin: the vendored RISC Zero SDK version and the Succinct receipt kind.
///
/// Declared separately from the V3 adapter's pin. Both guests use the same SDK
/// and receipt kind; the method id, version and artifact pin tell them apart.
pub const BACKEND_PIN: &str = "urn:sparq:vcq:backend:risc0-zkvm:3.0.6:succinct";
/// Query dialect; the V5 relation evaluates the unchanged V3 query language.
pub const QUERY_DIALECT: &str = crate::vcq::QUERY_DIALECT;
/// Query fragment; queries the V3 static admission accepts.
///
/// Admission is necessary, not sufficient: evaluation over mapped credentials
/// with blank nodes applies the V3 evaluator's stricter blank-node profile.
pub const QUERY_FRAGMENT: &str = crate::vcq::QUERY_FRAGMENT;
/// Authenticity suite: the bounded canonical RDF profile of `eddsa-rdfc-2022`.
pub const SUITE: &str = "urn:sparq:vcq:suite:di-eddsa-rdfc-2022:v5-bounded-canonical-rdf";
/// Mapping profile: hashed canonical documents in one default-graph union.
///
/// Blank nodes are scoped per credential; literal lexical forms are unchanged.
pub const MAPPING_PROFILE: &str = "urn:sparq:vcq:map:v5-scoped-canonical-union";
/// Linking profile: one relation authenticates, maps and evaluates one witness.
pub const LINKING_PROFILE: &str = "urn:sparq:vcq:link:authenticated-dataset-evaluation";
/// Relation enforcer: the separately pinned V5 guest program.
pub const RELATION: &str = "urn:sparq:vcq:relation:risc0-authenticated-rdf-v5-guest";
/// Public host check comparing the journaled authenticated commitment with the anchor.
pub const ANCHOR_CHECK: &str = "urn:sparq:vcq:host:authenticated-anchor-equality";
/// Disclosure: the V5 journal (request digest, commitment, provenance, result).
pub const DISCLOSURE: &str = "urn:sparq:vcq:disclosure:risc0-authenticated-rdf-v5-journal";

/// Method ceiling on presentation bytes; the same 16 MiB as the V3 adapter.
pub const MAX_PRESENTATION_BYTES: u32 = crate::vcq::MAX_PRESENTATION_BYTES;

/// Domain separator of [`parameter_digest`].
pub const PARAMETER_DOMAIN: &[u8] = b"sparq:vcq:risc0-authenticated-rdf:parameters:v5\0";
/// Domain separator of [`policy_digest`].
pub const POLICY_DOMAIN: &[u8] = b"sparq:vcq:risc0-authenticated-rdf:policy:v5\0";
/// Domain separator of [`derive_nonce`].
pub const NONCE_DOMAIN: &[u8] = b"sparq:vcq:risc0-authenticated-rdf:v5-nonce:local-struct-v1\0";
/// Domain separator of [`statement_digest`].
pub const STATEMENT_DOMAIN: &[u8] = b"sparq:vcq:risc0-authenticated-rdf:statement:v5\0";

/// Fixed query of the sentinel request hashed by [`policy_digest`].
///
/// Never parsed or evaluated; the model's request validation checks only its
/// length. Changing it changes every descriptor of this method.
pub const POLICY_SENTINEL_QUERY: &str = "ASK {}";
/// Fixed nonzero nonce of the sentinel request hashed by [`policy_digest`].
///
/// A public constant, never a challenge. Changing it changes every descriptor.
pub const POLICY_SENTINEL_NONCE: [u8; 32] = *b"sparq/vcq/authrdf/policy-binding";

/// Error code for a policy the V5 model rejects.
const POLICY_INVALID: &str = "vcq-authrdf-policy-invalid";

// Exhaustive on purpose: a new model suite or mapping variant must fail to
// compile here until it is given its own identifiers.
fn profile_ids(policy: &Policy) -> (&'static str, &'static str) {
    let suite = match policy.cryptosuite {
        Cryptosuite::EddsaRdfc2022 => SUITE,
    };
    let mapping = match policy.mapping {
        Mapping::ScopedCanonicalUnion => MAPPING_PROFILE,
    };
    (suite, mapping)
}

fn sentinel_request(policy: &Policy) -> auth::Request {
    auth::Request {
        version: auth::VERSION,
        query: POLICY_SENTINEL_QUERY.to_owned(),
        authority: DatasetAuthority::HolderDeclared,
        policy: policy.clone(),
        nonce: POLICY_SENTINEL_NONCE,
    }
}

fn policy_digest_at(policy: &Policy, phase: Phase) -> Result<[u8; 32], ProtocolError> {
    // This adapter verifies no presented signature, so only hidden mode is offered.
    if policy.signature_mode != auth::SignatureMode::Hidden {
        return Err(invalid(phase, POLICY_INVALID));
    }
    // The model diagnostic is never classified; every rejection is one code.
    let inner = auth::request_digest(&sentinel_request(policy))
        .map_err(|_| invalid(phase, POLICY_INVALID))?;
    let mut hash = Sha256::new();
    hash.update(POLICY_DOMAIN);
    hash.update(inner);
    Ok(hash.finalize().into())
}

/// Validates `policy` and returns its order-independent binding digest.
///
/// SHA-256 over [`POLICY_DOMAIN`] || the V5 model `request_digest` of the
/// sentinel request described in the module docs.
///
/// # Errors
/// Returns `invalid` at [`Phase::Request`] (`vcq-authrdf-policy-invalid`) for
/// any policy the V5 model rejects.
pub fn policy_digest(policy: &Policy) -> Result<[u8; 32], ProtocolError> {
    policy_digest_at(policy, Phase::Request)
}

fn parameter_digest_at(policy: &Policy, phase: Phase) -> Result<[u8; 32], ProtocolError> {
    let policy_digest = policy_digest_at(policy, phase)?;
    let (suite, mapping) = profile_ids(policy);
    let mut hash = Sha256::new();
    hash.update(PARAMETER_DOMAIN);
    hash.update(METHOD_VERSION.to_be_bytes());
    hash.update(policy_digest);
    for id in [suite, mapping, LINKING_PROFILE, RELATION] {
        hash.update((id.len() as u64).to_be_bytes());
        hash.update(id.as_bytes());
    }
    for bound in [
        MAX_QUERY_BYTES,
        auth::MAX_CREDENTIALS,
        auth::MAX_AUTHORIZED_KEYS,
        auth::MAX_IRI_BYTES,
        auth::MAX_DOCUMENT_BYTES,
        auth::MAX_PROOF_CONFIG_BYTES,
        auth::MAX_TOTAL_BYTES,
        auth::MAX_DOCUMENT_QUADS,
        auth::MAX_PROOF_CONFIG_QUADS,
        auth::MAX_TOTAL_QUADS,
        auth::MAX_WITNESS_BYTES,
    ] {
        hash.update((bound as u64).to_be_bytes());
    }
    for ceiling in [MAX_ROWS, MAX_PRESENTATION_BYTES] {
        hash.update(ceiling.to_be_bytes());
    }
    Ok(hash.finalize().into())
}

/// Digest of the verifier policy, profile identifiers and fixed capacities.
///
/// SHA-256 over [`PARAMETER_DOMAIN`]; the method version as big-endian `u32`;
/// [`policy_digest`]; the [`SUITE`], [`MAPPING_PROFILE`], [`LINKING_PROFILE`]
/// and [`RELATION`] identifiers, each as a big-endian `u64` length and bytes;
/// then as big-endian `u64` the query byte bound and the V5 model bounds
/// `MAX_CREDENTIALS`, `MAX_AUTHORIZED_KEYS`, `MAX_IRI_BYTES`,
/// `MAX_DOCUMENT_BYTES`, `MAX_PROOF_CONFIG_BYTES`, `MAX_TOTAL_BYTES`,
/// `MAX_DOCUMENT_QUADS`, `MAX_PROOF_CONFIG_QUADS`, `MAX_TOTAL_QUADS` and
/// `MAX_WITNESS_BYTES`; and last, as big-endian `u32`, the released-row and
/// presentation-byte ceilings.
///
/// # Errors
/// Returns `invalid` at [`Phase::Request`] for a policy the model rejects.
pub fn parameter_digest(policy: &Policy) -> Result<[u8; 32], ProtocolError> {
    parameter_digest_at(policy, Phase::Request)
}

/// Builds the exact descriptor for an approved `pin` and verifier `policy`.
///
/// # Errors
/// Returns `invalid` at [`Phase::Request`] for a policy the model rejects or an
/// all-zero pin digest or ID.
pub fn descriptor(pin: &ArtifactPin, policy: &Policy) -> Result<MethodDescriptor, ProtocolError> {
    MethodDescriptor::new(
        Identifier::new(METHOD_ID)?,
        METHOD_VERSION,
        Identifier::new(PARAMETER_SET)?,
        Digest32::new(parameter_digest(policy)?)?,
        ArtifactIdentity::ZkvmGuest {
            artifact_digest: Digest32::new(pin.sha256)?,
            image_id: Digest32::new(image_id_bytes(pin.image_id))?,
        },
        Identifier::new(BACKEND_PIN)?,
    )
}

fn capabilities(descriptor: MethodDescriptor) -> Result<Capabilities, ProtocolError> {
    let id = Identifier::new;
    let profile = QueryProfile {
        dialect: id(QUERY_DIALECT)?,
        fragment: id(QUERY_FRAGMENT)?,
    };
    let relation = Enforcer::Relation(id(RELATION)?);
    let mut tuples = Vec::with_capacity(6);
    for contract in [
        ResultContract::SelectBag,
        ResultContract::AskBoolean,
        ResultContract::GraphRdfc10,
    ] {
        for authority in [
            ScopeAuthority::VerifierAgreedAnchor,
            ScopeAuthority::HolderDeclared,
        ] {
            let anchor = match authority {
                ScopeAuthority::VerifierAgreedAnchor => Enforcer::HostPublic(id(ANCHOR_CHECK)?),
                ScopeAuthority::HolderDeclared => Enforcer::Absent,
            };
            tuples.push(CapabilityTuple {
                query_profile: profile.clone(),
                contract,
                mode: EvaluationMode::ExactBounded,
                authority,
                source_evidence: SourceEvidence::IssuerAuthenticated,
                status: StatusPolicy::NotRequested,
                holder: HolderPolicy::BearerAccepted,
                assembly: DatasetAssembly::UnionDefaultGraph,
                suite: Some(id(SUITE)?),
                mapping: id(MAPPING_PROFILE)?,
                linking: vec![id(LINKING_PROFILE)?],
                enforcement: Enforcement {
                    authenticity: relation.clone(),
                    mapping: relation.clone(),
                    status: Enforcer::Absent,
                    holder_binding: Enforcer::Absent,
                    linking: relation.clone(),
                    query: relation.clone(),
                    anchor,
                },
            });
        }
    }
    Capabilities::new(CapabilitiesSpec {
        descriptor,
        status: ComponentStatus::ImplementedExperimental,
        // Local implementation declaration, not validated availability; the
        // research registry keeps this method's `adapter_available` false.
        adapter_available: true,
        tuples,
        ceilings: ResourceBounds::new(MAX_ROWS, MAX_PRESENTATION_BYTES)?,
        challenge: ChallengePolicy {
            owner: ChallengeOwner::Method,
            consumption: ChallengeConsumption::ConsumeOnSuccess,
        },
        disclosure: vec![id(DISCLOSURE)?],
    })
}

/// Derives the V5 nonce from the stored request and selected descriptor.
///
/// SHA-256 over [`NONCE_DOMAIN`] || [`stored_request_digest`] ||
/// [`descriptor_digest`]. Both inner digests are fixed-width. The original
/// challenge is one stored-request field; the derived nonce is never consumed
/// by the store. The domain differs from the V3 adapter's, so the two methods
/// never derive the same nonce for one stored request.
#[must_use]
pub fn derive_nonce(request: &StoredRequest, descriptor: &MethodDescriptor) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(NONCE_DOMAIN);
    hash.update(stored_request_digest(request));
    hash.update(descriptor_digest(descriptor));
    hash.finalize().into()
}

/// Statement digest a verified claim carries.
///
/// SHA-256 over [`STATEMENT_DOMAIN`] || [`stored_request_digest`] ||
/// [`descriptor_digest`] || SHA-256 of the exact verified receipt journal
/// bytes, which are not re-encoded.
#[must_use]
pub fn statement_digest(
    request: &StoredRequest,
    descriptor: &MethodDescriptor,
    journal_bytes: &[u8],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(STATEMENT_DOMAIN);
    hash.update(stored_request_digest(request));
    hash.update(descriptor_digest(descriptor));
    hash.update(Sha256::digest(journal_bytes));
    hash.finalize().into()
}

/// Builds the V5 request both sides derive from a stored request and `policy`.
///
/// `policy` must be the verifier's own policy, the one `descriptor` was built
/// from; it is never taken from a presentation. The request carries the exact
/// query, the authority and anchor (an authenticated V5 commitment from
/// `authenticated_rdf::dataset_commitment`, not a V3 anchor), `policy` and the
/// [`derive_nonce`] value.
///
/// # Errors
/// Returns `invalid` at `phase` when `policy` is invalid
/// (`vcq-authrdf-policy-invalid`) or does not produce `descriptor`'s parameter
/// digest (`vcq-authrdf-policy-descriptor-mismatch`), when anchor authority
/// lacks an anchor, or when the model rejects the resulting request.
pub fn expected_v5_request(
    request: &StoredRequest,
    descriptor: &MethodDescriptor,
    policy: &Policy,
    phase: Phase,
) -> Result<auth::Request, ProtocolError> {
    if *descriptor.parameter_digest().as_bytes() != parameter_digest_at(policy, phase)? {
        return Err(invalid(phase, "vcq-authrdf-policy-descriptor-mismatch"));
    }
    let requirements = request.requirements();
    let authority = match requirements.authority() {
        ScopeAuthority::VerifierAgreedAnchor => {
            let anchor = requirements
                .anchor()
                .ok_or_else(|| invalid(phase, "vcq-anchor-missing"))?;
            DatasetAuthority::VerifierAgreed {
                commitment: *anchor.as_bytes(),
            }
        }
        ScopeAuthority::HolderDeclared => DatasetAuthority::HolderDeclared,
    };
    let expected = auth::Request {
        version: auth::VERSION,
        query: request.query().to_owned(),
        authority,
        policy: policy.clone(),
        nonce: derive_nonce(request, descriptor),
    };
    auth::validate_request(&expected).map_err(|_| invalid(phase, "vcq-authrdf-request-invalid"))?;
    Ok(expected)
}

/// Typed output of a verified V5 claim; produced only by verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedOutput {
    /// The released result, checked against the admitted contract and row bound.
    pub result: ReleasedResult,
    /// Journaled V5 provenance; agrees with the claim's scope authority.
    ///
    /// Neither variant implies wallet or world completeness, status or holder binding.
    pub provenance: Provenance,
    /// Journaled authenticated commitment; publicly linkable when a salt is reused.
    pub dataset_commitment: [u8; 32],
    /// Journaled V5 request digest, which binds the verifier's policy.
    pub v5_request_digest: [u8; 32],
}

/// Prepared V5 witness; never cloned, serialized or shown by `Debug`.
///
/// The model's `PrivateCredentials` input is itself `Clone`; this wrapper is
/// not, so a prepared witness can only be moved into [`QueryMethod::prove`].
pub struct AuthenticatedWitness {
    witness: auth::Witness,
    resources: ResourceBounds,
}

impl fmt::Debug for AuthenticatedWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthenticatedWitness([REDACTED])")
    }
}

/// The issuer-authenticated RDF (V5) RISC Zero query method.
#[derive(Debug)]
pub struct Risc0AuthenticatedRdfV5 {
    guest: AcceptedGuest,
    policy: Policy,
    capabilities: Capabilities,
    descriptor_digest: [u8; 32],
    r0vm: Option<PathBuf>,
    audience: Option<Identifier>,
    clock: fn() -> u64,
}

impl Risc0AuthenticatedRdfV5 {
    /// Builds the method from an approved V5 pin, accepted guest and verifier policy.
    ///
    /// `guest` must have been accepted under `pin`, and both, like `policy`,
    /// come from verifier or deployment configuration, never from a
    /// presentation. The stored policy is a copy with the authorization table
    /// sorted by verification method; the binding digests do not depend on
    /// table order. The policy cannot be changed afterwards.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] when `guest` does not match
    /// `pin` (`vcq-guest-pin-mismatch`), for an invalid policy
    /// (`vcq-authrdf-policy-invalid`) or for an all-zero pin.
    pub fn new(
        pin: &ArtifactPin,
        guest: AcceptedGuest,
        mut policy: Policy,
    ) -> Result<Self, ProtocolError> {
        let artifact: [u8; 32] = Sha256::digest(&guest.artifact).into();
        if artifact != pin.sha256 || guest.image_id() != pin.image_id {
            return Err(invalid(Phase::Request, "vcq-guest-pin-mismatch"));
        }
        policy
            .authorization
            .sort_by(|a, b| a.verification_method.cmp(&b.verification_method));
        let descriptor = descriptor(pin, &policy)?;
        Ok(Self {
            guest,
            policy,
            descriptor_digest: descriptor_digest(&descriptor),
            capabilities: capabilities(descriptor)?,
            r0vm: None,
            audience: None,
            clock: system_unix_seconds,
        })
    }

    /// Returns the verifier-owned policy, with its table sorted by verification method.
    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Sets the local `r0vm` executable that [`QueryMethod::prove`] runs.
    #[must_use]
    pub fn with_r0vm(mut self, r0vm: PathBuf) -> Self {
        self.r0vm = Some(r0vm);
        self
    }

    /// Sets the verifier's own audience and clock for [`QueryMethod::verify`].
    #[must_use]
    pub fn with_verifier(mut self, audience: Identifier, clock: fn() -> u64) -> Self {
        self.audience = Some(audience);
        self.clock = clock;
        self
    }

    /// Verifies with an explicitly supplied audience and Unix time.
    ///
    /// `audience` and `now_unix` must come from the verifier itself. Checks run
    /// in this order, all before the original challenge is consumed:
    /// recomputed admission and actual query form; audience; validity window;
    /// challenge policy; descriptor digest; presentation-byte bound (before any
    /// receipt decoding); receipt decoding; the V5 request rebuilt from the
    /// stored request and this method's own policy; Succinct proof and request
    /// binding; anchor, provenance, result contract and released-row bound on
    /// the verified journal; claim construction. Only then is the original
    /// challenge consumed once.
    ///
    /// # Errors
    /// `invalid`, `unsupported` or `capacity` at [`Phase::Verify`], or an
    /// admission error; `ChallengeReplayed` on replay and `infrastructure`
    /// `ChallengeStoreFailure` on store failure.
    pub fn verify_at(
        &self,
        request: &StoredRequest,
        audience: &Identifier,
        now_unix: u64,
        admission: &Admission,
        presentation: &VcqPresentation,
        challenges: &dyn ChallengeStore,
    ) -> Result<VerifiedClaim<AuthenticatedOutput>, ProtocolError> {
        let phase = Phase::Verify;
        let admission = checked_admission(&self.capabilities, request, admission, phase)?;
        check_verifier_context(request, audience, now_unix, &admission, phase)?;
        if *presentation.descriptor_digest() != self.descriptor_digest {
            return Err(invalid(phase, "vcq-descriptor-digest-mismatch"));
        }
        check_size(presentation, admission.resources(), phase)?;
        let receipt: Presentation = serde_json::from_slice(presentation.receipt_bytes())
            .map_err(|_| invalid(phase, "vcq-receipt-decoding"))?;
        let expected = expected_v5_request(request, admission.descriptor(), &self.policy, phase)?;
        let mut bridge = OriginalChallenge::new(expected.nonce, request, challenges);
        let journal_bytes = &receipt.receipt.journal.bytes;
        let outcome = verify_checked(
            &receipt,
            &expected,
            &mut bridge,
            self.guest.image_id(),
            |journal| claim(request, &admission, journal, journal_bytes),
        );
        bridge.finish(outcome.map(|(_, claim)| claim))
    }
}

/// Builds the claim from a VERIFIED, request-bound V5 journal; runs before consumption.
fn claim(
    request: &StoredRequest,
    admission: &Admission,
    journal: &auth::Journal,
    journal_bytes: &[u8],
) -> Result<VerifiedClaim<AuthenticatedOutput>, ProtocolError> {
    let phase = Phase::Verify;
    if journal.version != auth::VERSION {
        return Err(invalid(phase, "vcq-authrdf-journal-version"));
    }
    let requirements = request.requirements();
    let mut established = vec![
        Obligation::Authenticity,
        Obligation::Mapping,
        Obligation::Linking,
        Obligation::Query,
    ];
    match requirements.authority() {
        ScopeAuthority::VerifierAgreedAnchor => {
            let anchor = requirements
                .anchor()
                .ok_or_else(|| invalid(phase, "vcq-anchor-missing"))?;
            if journal.provenance != Provenance::VerifierAgreedAuthenticated
                || journal.dataset_commitment != *anchor.as_bytes()
            {
                return Err(invalid(phase, "vcq-anchor-mismatch"));
            }
            established.push(Obligation::Anchor);
        }
        ScopeAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderSelectedAuthenticated {
                return Err(invalid(phase, "vcq-provenance-mismatch"));
            }
        }
    }
    let result = released_result(
        admission.tuple().contract,
        &journal.result,
        admission.resources().released_rows(),
    )?;
    let statement = statement_digest(request, admission.descriptor(), journal_bytes);
    let evidence = ClaimEvidence {
        result: AuthenticatedOutput {
            result,
            provenance: journal.provenance,
            dataset_commitment: journal.dataset_commitment,
            v5_request_digest: journal.request_digest,
        },
        statement_digest: Digest32::new(statement)
            .map_err(|_| invalid(phase, "vcq-zero-statement-digest"))?,
        established,
    };
    VerifiedClaim::new(admission, evidence)
}

impl QueryMethod for Risc0AuthenticatedRdfV5 {
    type Request = StoredRequest;
    type PrivateInputs = PrivateCredentials;
    type Witness = AuthenticatedWitness;
    type Presentation = VcqPresentation;
    type Output = AuthenticatedOutput;
    type ChallengeStore = dyn ChallengeStore;

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Checks the request, authenticates the credentials natively and builds the witness.
    ///
    /// The native check only fails early; the guest repeats every check.
    ///
    /// # Errors
    /// Admission and query-shape errors as in verification (at
    /// [`Phase::Prepare`]); `invalid` (`vcq-authrdf-credentials-rejected`) when
    /// the model rejects the credentials for any reason, whose diagnostic is not
    /// classified; `unsatisfiable` when they do not open the agreed anchor.
    fn prepare(
        &self,
        request: &StoredRequest,
        admission: &Admission,
        private: PrivateCredentials,
    ) -> Result<PreparedWitness<AuthenticatedWitness>, ProtocolError> {
        let phase = Phase::Prepare;
        let admission = checked_admission(&self.capabilities, request, admission, phase)?;
        let expected = expected_v5_request(request, admission.descriptor(), &self.policy, phase)?;
        let commitment = auth::dataset_commitment(&private, &self.policy)
            .map_err(|_| invalid(phase, "vcq-authrdf-credentials-rejected"))?;
        let opens_anchor = match &expected.authority {
            DatasetAuthority::VerifierAgreed { commitment: anchor } => *anchor == commitment,
            DatasetAuthority::HolderDeclared => true,
        };
        if !opens_anchor {
            return Err(fail(
                BackendFailure::Unsatisfiable,
                phase,
                "vcq-anchor-not-opened",
            ));
        }
        let inner = AuthenticatedWitness {
            witness: auth::Witness {
                request: expected,
                dataset: private,
            },
            resources: admission.resources(),
        };
        Ok(PreparedWitness::new(&admission, inner))
    }

    /// Proves the witness with the accepted V5 guest and the configured `r0vm`.
    ///
    /// # Errors
    /// `invalid` for a witness of another descriptor; `infrastructure` for a
    /// missing `r0vm` or any proving failure (never classified as capacity);
    /// `capacity` when the presentation exceeds the request's byte bound.
    fn prove(
        &self,
        witness: PreparedWitness<AuthenticatedWitness>,
    ) -> Result<VcqPresentation, ProtocolError> {
        let phase = Phase::Prove;
        let inner = witness.into_inner_for(self.capabilities.descriptor())?;
        let r0vm = self
            .r0vm
            .as_deref()
            .ok_or_else(|| fail(BackendFailure::Infrastructure, phase, "vcq-r0vm-missing"))?;
        let presentation = prove_with_artifact(&inner.witness, r0vm, &self.guest)
            .map_err(|_| fail(BackendFailure::Infrastructure, phase, "vcq-proof-failed"))?;
        let receipt = serde_json::to_vec(&presentation)
            .map_err(|_| fail(BackendFailure::Infrastructure, phase, "vcq-receipt-encoding"))?;
        let presentation = VcqPresentation::new(self.descriptor_digest, receipt);
        check_size(&presentation, inner.resources, phase)?;
        Ok(presentation)
    }

    /// Verifies with the configured audience and clock; see [`Risc0AuthenticatedRdfV5::verify_at`].
    ///
    /// # Errors
    /// `invalid` when no verifier audience is configured; otherwise as
    /// [`Risc0AuthenticatedRdfV5::verify_at`].
    fn verify(
        &self,
        request: &StoredRequest,
        admission: &Admission,
        presentation: &VcqPresentation,
        challenges: &Self::ChallengeStore,
    ) -> Result<VerifiedClaim<AuthenticatedOutput>, ProtocolError> {
        let audience = self
            .audience
            .as_ref()
            .ok_or_else(|| invalid(Phase::Verify, "vcq-verifier-context-missing"))?;
        self.verify_at(
            request,
            audience,
            (self.clock)(),
            admission,
            presentation,
            challenges,
        )
    }
}

#[cfg(test)]
mod tests {
    //! Claim conversion over hand-built journals. These tests create and
    //! accept no proof: `claim` is only ever reached after receipt verification.
    use super::*;
    use sparq_proved_evaluator_model::{RowOrder, v3};
    use sparq_query_protocol::{
        CapacityBound, Challenge32, ClaimScope, Completeness, ErrorCode, ObligationOutcome,
        QueryForm, QueryRequirements, RequirementsSpec, StoredRequestSpec, admit,
    };

    /// Published W3C vc-di-eddsa example key and identifiers; public data only.
    const ISSUER: &str = "https://vc.example/issuers/5678";
    const METHOD: &str = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";
    const KEY: &str = "b00d8d938e7f773d51565aad36a623f5344f7f5d1960f9cf3e8e12620ea2810f";
    const ANCHOR: [u8; 32] = [0x42; 32];
    const CELL: &str = "\"Alumni Credential\"";

    fn key() -> [u8; 32] {
        std::array::from_fn(|i| u8::from_str_radix(&KEY[2 * i..2 * i + 2], 16).unwrap())
    }

    fn policy() -> Policy {
        Policy::new(vec![auth::AuthorizedKey {
            issuer: ISSUER.to_owned(),
            verification_method: METHOD.to_owned(),
            public_key: key(),
        }])
    }

    fn setup(
        contract: ResultContract,
        form: QueryForm,
        query: &str,
        authority: ScopeAuthority,
        rows: u32,
    ) -> (StoredRequest, Admission) {
        let id = |text| Identifier::new(text).unwrap();
        let pin = ArtifactPin {
            sha256: [1; 32],
            image_id: [1; 8],
        };
        let descriptor = descriptor(&pin, &policy()).unwrap();
        let capabilities = capabilities(descriptor.clone()).unwrap();
        let requirements = QueryRequirements::new(RequirementsSpec {
            contract,
            mode: EvaluationMode::ExactBounded,
            authority,
            anchor: (authority == ScopeAuthority::VerifierAgreedAnchor)
                .then(|| Digest32::new(ANCHOR).unwrap()),
            source_evidence: Some(SourceEvidence::IssuerAuthenticated),
            status: Some(StatusPolicy::NotRequested),
            holder: Some(HolderPolicy::BearerAccepted),
            assembly: DatasetAssembly::UnionDefaultGraph,
            query_profile: QueryProfile {
                dialect: id(QUERY_DIALECT),
                fragment: id(QUERY_FRAGMENT),
            },
            accepted_suites: vec![id(SUITE)],
            accepted_mappings: vec![id(MAPPING_PROFILE)],
            accepted_linking: vec![id(LINKING_PROFILE)],
            resources: ResourceBounds::new(rows, 1 << 20).unwrap(),
            methods: vec![descriptor.clone()],
        })
        .unwrap();
        let request = StoredRequest::new(StoredRequestSpec {
            requirements,
            query: query.to_owned(),
            challenge: Challenge32::new([9; 32]).unwrap(),
            audience: id("urn:example:verifier"),
            not_before: 10,
            not_after: 20,
            form,
            base_iri: None,
            describe_policy: None,
        })
        .unwrap();
        let admission = admit(request.requirements(), &descriptor, &capabilities).unwrap();
        (request, admission)
    }

    fn bag(rows: usize) -> v3::CanonicalResult {
        v3::CanonicalResult::Select {
            variables: vec!["name".to_owned()],
            order: RowOrder::Bag,
            rows: vec![vec![Some(CELL.to_owned())]; rows],
        }
    }

    fn journal(
        provenance: Provenance,
        commitment: [u8; 32],
        result: v3::CanonicalResult,
    ) -> auth::Journal {
        auth::Journal {
            version: auth::VERSION,
            request_digest: [7; 32],
            dataset_commitment: commitment,
            provenance,
            result,
            signed_messages: Vec::new(),
        }
    }

    fn code<T: fmt::Debug>(result: Result<T, ProtocolError>) -> ErrorCode {
        result.unwrap_err().code()
    }

    #[test]
    fn agreed_claim_reports_authenticated_scope_and_established_obligations() {
        let query = "SELECT ?name WHERE { ?c <https://schema.org/name> ?name }";
        let (request, admission) = setup(
            ResultContract::SelectBag,
            QueryForm::Select,
            query,
            ScopeAuthority::VerifierAgreedAnchor,
            2,
        );
        let journal = journal(Provenance::VerifierAgreedAuthenticated, ANCHOR, bag(2));
        let claim = claim(&request, &admission, &journal, b"journal").unwrap();
        let output = claim.result();
        assert_eq!(
            output.result,
            ReleasedResult::SelectBag {
                variables: vec!["name".to_owned()],
                rows: vec![vec![Some(CELL.to_owned())]; 2],
            }
        );
        assert_eq!(output.provenance, Provenance::VerifierAgreedAuthenticated);
        assert_eq!(output.dataset_commitment, ANCHOR);
        assert_eq!(output.v5_request_digest, [7; 32]);
        assert_eq!(
            claim.statement_digest().as_bytes(),
            &statement_digest(&request, admission.descriptor(), b"journal")
        );
        assert_eq!(
            *claim.scope(),
            ClaimScope {
                authority: ScopeAuthority::VerifierAgreedAnchor,
                anchor: Some(Digest32::new(ANCHOR).unwrap()),
                source_evidence: SourceEvidence::IssuerAuthenticated,
                completeness: Completeness::RelativeToScope,
            }
        );
        assert_eq!(claim.tuple().suite, Some(Identifier::new(SUITE).unwrap()));
        let relation = ObligationOutcome::Established(Enforcer::Relation(
            Identifier::new(RELATION).unwrap(),
        ));
        let anchor = ObligationOutcome::Established(Enforcer::HostPublic(
            Identifier::new(ANCHOR_CHECK).unwrap(),
        ));
        let absent = ObligationOutcome::NotEstablished;
        for (obligation, expected) in [
            (Obligation::Authenticity, &relation),
            (Obligation::Mapping, &relation),
            (Obligation::Linking, &relation),
            (Obligation::Query, &relation),
            (Obligation::Anchor, &anchor),
            (Obligation::Status, &absent),
            (Obligation::HolderBinding, &absent),
        ] {
            assert_eq!(claim.outcome(obligation), expected, "{obligation:?}");
        }
    }

    #[test]
    fn holder_claim_keeps_holder_selected_provenance_without_anchor() {
        let query = "ASK { ?c ?p ?o }";
        let (request, admission) = setup(
            ResultContract::AskBoolean,
            QueryForm::Ask,
            query,
            ScopeAuthority::HolderDeclared,
            1,
        );
        let journal = journal(
            Provenance::HolderSelectedAuthenticated,
            [3; 32],
            v3::CanonicalResult::Ask(false),
        );
        let claim = claim(&request, &admission, &journal, b"journal").unwrap();
        assert_eq!(claim.result().result, ReleasedResult::Ask(false));
        assert_eq!(claim.result().provenance, Provenance::HolderSelectedAuthenticated);
        assert_eq!(claim.scope().anchor, None);
        assert_eq!(claim.outcome(Obligation::Anchor), &ObligationOutcome::NotEstablished);
        assert!(matches!(
            claim.outcome(Obligation::Authenticity),
            ObligationOutcome::Established(Enforcer::Relation(_))
        ));
    }

    #[test]
    fn provenance_anchor_version_form_and_rows_reject_before_consumption() {
        let bad = |code: &'static str| ErrorCode::Backend(code);
        let query = "SELECT ?name WHERE { ?c <https://schema.org/name> ?name }";
        let agreed = setup(
            ResultContract::SelectBag,
            QueryForm::Select,
            query,
            ScopeAuthority::VerifierAgreedAnchor,
            4,
        );
        let holder = setup(
            ResultContract::SelectBag,
            QueryForm::Select,
            query,
            ScopeAuthority::HolderDeclared,
            1,
        );
        let run = |(request, admission): &(StoredRequest, Admission), journal: auth::Journal| {
            code(claim(request, admission, &journal, b"journal"))
        };
        // Holder-selected provenance never satisfies agreed authority, even with the anchor.
        let upgraded = journal(Provenance::HolderSelectedAuthenticated, ANCHOR, bag(1));
        assert_eq!(run(&agreed, upgraded), bad("vcq-anchor-mismatch"));
        let other = journal(Provenance::VerifierAgreedAuthenticated, [5; 32], bag(1));
        assert_eq!(run(&agreed, other), bad("vcq-anchor-mismatch"));
        let claimed = journal(Provenance::VerifierAgreedAuthenticated, ANCHOR, bag(1));
        assert_eq!(run(&holder, claimed), bad("vcq-provenance-mismatch"));
        let mut earlier = journal(Provenance::VerifierAgreedAuthenticated, ANCHOR, bag(1));
        earlier.version = v3::VERSION;
        assert_eq!(run(&agreed, earlier), bad("vcq-authrdf-journal-version"));
        let ask = journal(
            Provenance::VerifierAgreedAuthenticated,
            ANCHOR,
            v3::CanonicalResult::Ask(true),
        );
        assert_eq!(run(&agreed, ask), bad("vcq-result-kind-mismatch"));
        let two = journal(Provenance::HolderSelectedAuthenticated, [3; 32], bag(2));
        assert_eq!(
            run(&holder, two),
            ErrorCode::CapacityExceeded(CapacityBound::Backend {
                name: "vcq-released-rows",
                requested: 2,
                ceiling: 1,
            })
        );
    }

    #[test]
    fn policy_binding_rejects_a_mismatched_descriptor_and_invalid_policy() {
        let query = "ASK { ?c ?p ?o }";
        let (request, admission) = setup(
            ResultContract::AskBoolean,
            QueryForm::Ask,
            query,
            ScopeAuthority::HolderDeclared,
            1,
        );
        let descriptor = admission.descriptor();
        let expected = expected_v5_request(&request, descriptor, &policy(), Phase::Verify).unwrap();
        assert_eq!(expected.policy, policy());
        assert_eq!(expected.nonce, derive_nonce(&request, descriptor));
        let mut other = policy();
        other.authorization[0].issuer = "https://vc.example/issuers/9999".to_owned();
        assert_eq!(
            code(expected_v5_request(&request, descriptor, &other, Phase::Verify)),
            ErrorCode::Backend("vcq-authrdf-policy-descriptor-mismatch")
        );
        let empty = Policy::new(Vec::new());
        assert_eq!(
            code(expected_v5_request(&request, descriptor, &empty, Phase::Verify)),
            ErrorCode::Backend(POLICY_INVALID)
        );
        // Revealed mode needs presented signatures this adapter never checks.
        let revealed = policy().with_signature_mode(auth::SignatureMode::Revealed);
        assert_eq!(code(policy_digest(&revealed)), ErrorCode::Backend(POLICY_INVALID));
        assert_eq!(
            code(expected_v5_request(&request, descriptor, &revealed, Phase::Verify)),
            ErrorCode::Backend(POLICY_INVALID)
        );
    }
}

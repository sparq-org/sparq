// [OPUS-5.5] zkp-16.2 first executable vcq adapter; experimental, not externally audited.
//! `sparq-query-protocol` adapter over the exact RISC Zero V3 relation.
//!
//! [`Risc0ExactV3`] implements [`QueryMethod`] for one exact descriptor,
//! `urn:sparq:vcq:method:risc0-exact` version 3, built from an independently
//! approved [`ArtifactPin`] and [`AcceptedGuest`]. Both are verifier
//! configuration; a presentation never selects them. Proving uses
//! [`crate::v3::prove_with_artifact`] and verification uses the existing checked
//! V3 path (Succinct receipt, dev mode disabled, journal decoded only after the
//! proof checks, then independent request binding).
//!
//! # Capabilities
//!
//! Six whole tuples: result contracts `SelectBag`, `AskBoolean` and
//! `GraphRdfc10` (CONSTRUCT only), each under `VerifierAgreedAnchor` and
//! `HolderDeclared`. Every tuple is `ExactBounded`, query profile
//! ([`QUERY_DIALECT`], [`QUERY_FRAGMENT`]), `ExactSourceCatalog` assembly,
//! source evidence `None`, status `NotRequested`, holder `BearerAccepted`,
//! mapping [`MAPPING_PROFILE`] and linking [`LINKING_PROFILE`]. Mapping,
//! linking and query are enforced by the guest relation [`RELATION`]; the
//! anchor comparison is the public host check [`ANCHOR_CHECK`] and is owed
//! only under verifier-agreed authority. Authenticity, status and holder
//! binding are never established. SELECT sequences, DESCRIBE and explicit base
//! IRIs are rejected. Only the fixed default V3 policy is used; the
//! [`parameter_digest`] pins it.
//!
//! # Request binding
//!
//! The V3 nonce is [`derive_nonce`]: SHA-256 over [`NONCE_DOMAIN`], the
//! SHA-256 of the `local-struct-v1` stored-request bytes and the SHA-256 of the
//! `local-struct-v1` bytes of the selected descriptor. Every stored-request
//! field therefore reaches the V3 request digest the guest journals. The
//! verifier also checks audience and validity (`not_before <= now < not_after`)
//! against its own [`StoredRequest`] and clock.
//!
//! # Challenge
//!
//! The method owns the challenge and consumes it on success. The V3 verifier
//! calls its nonce store once, after every other check; this adapter's bridge
//! accepts exactly that one call with the expected derived nonce and consumes
//! the ORIGINAL [`StoredRequest::challenge`] through the shared
//! [`ChallengeStore`]. Replay and store failure are classified from the typed
//! store outcome, never from error text.
//!
//! # Resource bounds
//!
//! Two distinct checks, each `capacity` at [`Phase::Verify`]. The encoded
//! presentation length is checked against `presentation_bytes` before the
//! receipt is decoded (`vcq-presentation-bytes`). The released-row count is a
//! semantic bound on the result: it is read from the verified, request-bound
//! journal and checked against `released_rows` after the proof checks and
//! before the original challenge is consumed (`vcq-released-rows`). SELECT rows
//! and CONSTRUCT N-Triples lines count; ASK has no row bound.
//!
//! # Limits
//!
//! This adapter authenticates no source credential and checks no status or
//! holder key. Holder-declared results assert only computation over the
//! holder's chosen bytes. The research registry records the `vcq_adapter` of
//! `method:risc0-exact` as available for version 3 and for exactly the six
//! tuples above; every other version and contract has no adapter. This code
//! never reads the registry: admission checks only the local [`Capabilities`].

use crate::v3::{CheckedFailure, prove_with_artifact, verify_checked_with_artifact};
use crate::{AcceptedGuest, ArtifactPin, Error, Nonces, Presentation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::v3::{self, QueryShape, ShapeError, query_shape};
use sparq_proved_evaluator_model::{
    DatasetAuthority, MAX_QUERY_BYTES, MAX_ROWS, ProofContract, Provenance, RowOrder, v2,
};
use sparq_query_protocol::{
    Admission, ArtifactIdentity, BackendFailure, Capabilities, CapabilitiesSpec, CapabilityTuple,
    CapacityBound, ChallengeConsumption, ChallengeOwner, ChallengePolicy, ChallengeStore,
    ClaimEvidence, ComponentStatus, DatasetAssembly, Digest32, Enforcement, Enforcer, ErrorCode,
    EvaluationMode, HolderPolicy, Identifier, MethodDescriptor, Obligation, Phase,
    PreparedWitness, ProtocolError, QueryForm, QueryMethod, QueryProfile, ResourceBounds,
    ResultContract, ScopeAuthority, SourceEvidence, StatusPolicy, StoredRequest, VerifiedClaim,
    admit, consume_challenge, encode_method_descriptor, encode_stored_request,
};
use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Method id of the exact RISC Zero evaluator.
pub const METHOD_ID: &str = "urn:sparq:vcq:method:risc0-exact";
/// Method version; equals the V3 relation's wire version.
pub const METHOD_VERSION: u32 = 3;
const _: () = assert!(METHOD_VERSION == v3::VERSION);
/// Parameter-set id for the fixed default V3 policy.
pub const PARAMETER_SET: &str = "urn:sparq:vcq:params:risc0-exact:v3-default-policy";
/// Backend pin: the vendored RISC Zero SDK version and the Succinct receipt kind.
pub const BACKEND_PIN: &str = "urn:sparq:vcq:backend:risc0-zkvm:3.0.6:succinct";
/// Query dialect: SPARQL 1.1 under the pinned V3 graph-result engine snapshot.
pub const QUERY_DIALECT: &str = "urn:sparq:vcq:dialect:sparq-sparql11-graph-results:v3";
/// Query fragment: queries the V3 static admission accepts.
pub const QUERY_FRAGMENT: &str = "urn:sparq:vcq:fragment:v3-static-admission";
/// Mapping profile; the V3 interpretation is selected by the descriptor version.
pub const MAPPING_PROFILE: &str = "urn:sparq:vcq:map:exact-source-bytes";
/// Linking profile: the proof evaluates exactly the bytes it commits to.
pub const LINKING_PROFILE: &str = "urn:sparq:vcq:link:committed-dataset-evaluation";
/// Relation enforcer: the pinned V3 guest program.
pub const RELATION: &str = "urn:sparq:vcq:relation:risc0-exact-v3-guest";
/// Public host check comparing the journaled commitment with the anchor.
pub const ANCHOR_CHECK: &str = "urn:sparq:vcq:host:anchor-equality";
/// Disclosure: the V3 journal (request digest, commitment, provenance, result).
pub const DISCLOSURE: &str = "urn:sparq:vcq:disclosure:risc0-exact-v3-journal";

/// Method ceiling on presentation bytes (16 MiB).
///
/// Chosen to hold one Succinct receipt serialized as JSON with a wide margin.
/// Requests may set a lower bound; the verifier checks it before decoding.
pub const MAX_PRESENTATION_BYTES: u32 = 16 << 20;

/// Domain separator of [`parameter_digest`].
pub const PARAMETER_DOMAIN: &[u8] = b"sparq:vcq:risc0-exact:parameters:v3\0";
/// Domain separator of [`derive_nonce`].
pub const NONCE_DOMAIN: &[u8] = b"sparq:vcq:risc0-exact:v3-nonce:local-struct-v1\0";
/// Domain separator of [`statement_digest`].
pub const STATEMENT_DOMAIN: &[u8] = b"sparq:vcq:risc0-exact:statement:v3\0";

fn fail(class: BackendFailure, phase: Phase, code: &'static str) -> ProtocolError {
    ProtocolError::backend(class, phase, code)
}

fn invalid(phase: Phase, code: &'static str) -> ProtocolError {
    fail(BackendFailure::Invalid, phase, code)
}

fn unsupported(phase: Phase, code: &'static str) -> ProtocolError {
    fail(BackendFailure::Unsupported, phase, code)
}

/// Digest of the fixed default V3 policy and query bound.
///
/// SHA-256 over [`PARAMETER_DOMAIN`], then big-endian `u32` values in this
/// order: V3 version; dataset `max_dataset_bytes`, `max_triples`, `max_rows`,
/// `max_named_graphs`; canonicalization `max_quads`, `max_input_bytes`,
/// `max_output_bytes`, `max_hndq_calls`, `max_permutation_steps`; then one
/// DESCRIBE-policy byte (`OutgoingBlankNodeClosure` = `0x01`) and the query
/// byte bound as big-endian `u64`.
#[must_use]
pub fn parameter_digest() -> [u8; 32] {
    let v3::Policy {
        dataset,
        canonicalization,
        describe,
    } = v3::Policy::default();
    let v2::Policy {
        max_dataset_bytes,
        max_triples,
        max_rows,
        max_named_graphs,
    } = dataset;
    let v3::CanonicalizationPolicy {
        max_quads,
        max_input_bytes,
        max_output_bytes,
        max_hndq_calls,
        max_permutation_steps,
    } = canonicalization;
    let describe = match describe {
        v3::DescribePolicy::OutgoingBlankNodeClosure => 0x01_u8,
    };
    let mut hash = Sha256::new();
    hash.update(PARAMETER_DOMAIN);
    for value in [
        v3::VERSION,
        max_dataset_bytes,
        max_triples,
        max_rows,
        max_named_graphs,
        max_quads,
        max_input_bytes,
        max_output_bytes,
        max_hndq_calls,
        max_permutation_steps,
    ] {
        hash.update(value.to_be_bytes());
    }
    hash.update([describe]);
    hash.update((MAX_QUERY_BYTES as u64).to_be_bytes());
    hash.finalize().into()
}

/// RISC Zero image-id bytes: each word little-endian, in word order.
#[must_use]
pub fn image_id_bytes(words: [u32; 8]) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// Builds the exact descriptor for an independently approved `pin`.
///
/// # Errors
/// Returns `invalid` at [`Phase::Request`] for an all-zero pin digest or ID.
pub fn descriptor(pin: &ArtifactPin) -> Result<MethodDescriptor, ProtocolError> {
    MethodDescriptor::new(
        Identifier::new(METHOD_ID)?,
        METHOD_VERSION,
        Identifier::new(PARAMETER_SET)?,
        Digest32::new(parameter_digest())?,
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
                source_evidence: SourceEvidence::None,
                status: StatusPolicy::NotRequested,
                holder: HolderPolicy::BearerAccepted,
                assembly: DatasetAssembly::ExactSourceCatalog,
                suite: None,
                mapping: id(MAPPING_PROFILE)?,
                linking: vec![id(LINKING_PROFILE)?],
                enforcement: Enforcement {
                    authenticity: Enforcer::Absent,
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

/// SHA-256 of the `local-struct-v1` encoding of `descriptor`.
#[must_use]
pub fn descriptor_digest(descriptor: &MethodDescriptor) -> [u8; 32] {
    Sha256::digest(encode_method_descriptor(descriptor)).into()
}

/// SHA-256 of the `local-struct-v1` encoding of `request`.
#[must_use]
pub fn stored_request_digest(request: &StoredRequest) -> [u8; 32] {
    Sha256::digest(encode_stored_request(request)).into()
}

/// Derives the V3 nonce from the stored request and selected descriptor.
///
/// SHA-256 over [`NONCE_DOMAIN`] || [`stored_request_digest`] ||
/// [`descriptor_digest`]. Both inner digests are fixed-width, so the
/// concatenation is unambiguous. The original challenge is one of the
/// stored-request fields; the derived nonce is never consumed by the store.
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
/// bytes. The journal bytes are the guest's committed output under the pinned
/// RISC Zero serde encoding; they are not re-encoded here.
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

/// Builds the V3 request both sides derive from the stored request.
///
/// The V3 request's own JSON digest keeps its existing pinned format; this
/// adapter only fills its fields: the exact query, the authority and anchor,
/// the default policy and the [`derive_nonce`] value.
///
/// # Errors
/// Returns `invalid` at `phase` when anchor authority lacks an anchor.
pub fn expected_v3_request(
    request: &StoredRequest,
    descriptor: &MethodDescriptor,
    phase: Phase,
) -> Result<v3::Request, ProtocolError> {
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
    Ok(v3::Request {
        version: v3::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
        query: request.query().to_owned(),
        authority,
        policy: v3::Policy::default(),
        nonce: derive_nonce(request, descriptor),
    })
}

/// Current Unix time in seconds; a pre-epoch clock reads as `u64::MAX`.
///
/// `u64::MAX` is never below an exclusive `not_after`, so every request
/// window rejects it as expired. Zero would pass a `not_before = 0` window.
#[must_use]
pub fn system_unix_seconds() -> u64 {
    unix_seconds(SystemTime::now())
}

// [OPUS-5.5] Failed epoch conversion must not read as a valid instant.
fn unix_seconds(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |elapsed| elapsed.as_secs())
}

/// Descriptor digest plus bounded, serialized receipt bytes.
///
/// The receipt bytes are the existing host [`Presentation`] as JSON, used only
/// as transport; no protocol hash is taken over them. Transports should bound
/// their own input before building this value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VcqPresentation {
    descriptor_digest: [u8; 32],
    receipt: Vec<u8>,
}

impl VcqPresentation {
    /// Wraps `receipt` bytes produced for the descriptor with `descriptor_digest`.
    #[must_use]
    pub fn new(descriptor_digest: [u8; 32], receipt: Vec<u8>) -> Self {
        Self {
            descriptor_digest,
            receipt,
        }
    }

    /// Returns the selected descriptor's digest.
    #[must_use]
    pub fn descriptor_digest(&self) -> &[u8; 32] {
        &self.descriptor_digest
    }

    /// Returns the serialized receipt bytes.
    #[must_use]
    pub fn receipt_bytes(&self) -> &[u8] {
        &self.receipt
    }

    /// Returns the size counted against the presentation-byte bound.
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        u64::try_from(self.receipt.len())
            .unwrap_or(u64::MAX)
            .saturating_add(32)
    }
}

impl fmt::Debug for VcqPresentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VcqPresentation")
            .field("receipt_len", &self.receipt.len())
            .finish_non_exhaustive()
    }
}

/// Released result taken from the verified V3 journal.
///
/// Cells and graph lines are the journal's already canonical N-Triples term
/// strings; they are not re-parsed. `None` is an unbound cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleasedResult {
    /// SELECT bag: sorted rows, duplicates retained.
    SelectBag {
        /// Unique projected variable names.
        variables: Vec<String>,
        /// Rows of exactly `variables.len()` cells.
        rows: Vec<Vec<Option<String>>>,
    },
    /// ASK value.
    Ask(bool),
    /// RDFC-1.0 canonical N-Triples of the CONSTRUCT graph.
    Graph {
        /// Canonical N-Triples, one triple per line.
        ntriples: String,
    },
}

/// Typed output of a verified claim; produced only by verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V3Output {
    /// The released result.
    pub result: ReleasedResult,
    /// Journaled provenance; agrees with the claim's scope authority.
    pub provenance: Provenance,
    /// Journaled V3 dataset commitment.
    pub dataset_commitment: [u8; 32],
    /// Journaled V3 request digest.
    pub v3_request_digest: [u8; 32],
}

/// Prepared V3 witness; never cloned, serialized or shown by `Debug`.
pub struct V3Witness {
    witness: v3::Witness,
    resources: ResourceBounds,
}

impl fmt::Debug for V3Witness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("V3Witness([REDACTED])")
    }
}

/// The exact RISC Zero V3 query method.
#[derive(Debug)]
pub struct Risc0ExactV3 {
    guest: AcceptedGuest,
    capabilities: Capabilities,
    descriptor_digest: [u8; 32],
    r0vm: Option<PathBuf>,
    audience: Option<Identifier>,
    clock: fn() -> u64,
}

impl Risc0ExactV3 {
    /// Builds the method from an independently approved pin and accepted guest.
    ///
    /// `guest` must have been accepted under `pin`; both come from verifier
    /// or deployment configuration, never from a presentation.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] when `guest` does not match
    /// `pin`, or for an all-zero pin.
    pub fn new(pin: &ArtifactPin, guest: AcceptedGuest) -> Result<Self, ProtocolError> {
        let artifact: [u8; 32] = Sha256::digest(&guest.artifact).into();
        if artifact != pin.sha256 || guest.image_id != pin.image_id {
            return Err(invalid(Phase::Request, "vcq-guest-pin-mismatch"));
        }
        let descriptor = descriptor(pin)?;
        Ok(Self {
            guest,
            descriptor_digest: descriptor_digest(&descriptor),
            capabilities: capabilities(descriptor)?,
            r0vm: None,
            audience: None,
            clock: system_unix_seconds,
        })
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

    /// Recomputes admission and checks the actual query against the request.
    fn check_request(
        &self,
        request: &StoredRequest,
        admission: &Admission,
        phase: Phase,
    ) -> Result<Admission, ProtocolError> {
        let recomputed = admit(
            request.requirements(),
            admission.descriptor(),
            &self.capabilities,
        )?;
        if recomputed != *admission {
            return Err(invalid(phase, "vcq-admission-mismatch"));
        }
        if request.base_iri().is_some() {
            return Err(unsupported(phase, "vcq-base-iri-unsupported"));
        }
        let shape = query_shape(request.query()).map_err(|error| shape_error(&error, phase))?;
        let actual = match shape {
            QueryShape::SelectBag | QueryShape::SelectSequence => QueryForm::Select,
            QueryShape::Ask => QueryForm::Ask,
            QueryShape::Construct => QueryForm::Construct,
            QueryShape::Describe => QueryForm::Describe,
        };
        if actual != request.form() {
            return Err(invalid(phase, "vcq-query-form-mismatch"));
        }
        match (recomputed.tuple().contract, shape) {
            (ResultContract::SelectBag, QueryShape::SelectBag)
            | (ResultContract::AskBoolean, QueryShape::Ask)
            | (ResultContract::GraphRdfc10, QueryShape::Construct) => Ok(recomputed),
            (_, QueryShape::Describe) => Err(unsupported(phase, "vcq-describe-unsupported")),
            (_, QueryShape::SelectSequence) => {
                Err(unsupported(phase, "vcq-select-sequence-unsupported"))
            }
            _ => Err(invalid(phase, "vcq-contract-shape-mismatch")),
        }
    }

    /// Verifies with an explicitly supplied audience and Unix time.
    ///
    /// `audience` and `now_unix` must come from the verifier itself. Checks run
    /// in this order, all before the original challenge is consumed:
    /// recomputed admission and actual query form; audience; validity window;
    /// descriptor digest; presentation-byte bound (before any receipt
    /// decoding); receipt decoding; Succinct proof and request binding; the
    /// anchor, provenance and result contract on the verified journal; claim
    /// construction. Only then is the original challenge consumed once.
    ///
    /// # Errors
    /// `invalid`, `unsupported` or `capacity` at [`Phase::Verify`], or an
    /// admission error; [`ErrorCode::ChallengeReplayed`] on replay and
    /// `infrastructure` [`ErrorCode::ChallengeStoreFailure`] on store failure.
    pub fn verify_at(
        &self,
        request: &StoredRequest,
        audience: &Identifier,
        now_unix: u64,
        admission: &Admission,
        presentation: &VcqPresentation,
        challenges: &dyn ChallengeStore,
    ) -> Result<VerifiedClaim<V3Output>, ProtocolError> {
        let phase = Phase::Verify;
        let admission = self.check_request(request, admission, phase)?;
        if request.audience() != audience {
            return Err(invalid(phase, "vcq-audience-mismatch"));
        }
        if now_unix < request.not_before() {
            return Err(invalid(phase, "vcq-request-not-yet-valid"));
        }
        if now_unix >= request.not_after() {
            return Err(invalid(phase, "vcq-request-expired"));
        }
        if admission.challenge()
            != (ChallengePolicy {
                owner: ChallengeOwner::Method,
                consumption: ChallengeConsumption::ConsumeOnSuccess,
            })
        {
            return Err(invalid(phase, "vcq-challenge-policy"));
        }
        if presentation.descriptor_digest != self.descriptor_digest {
            return Err(invalid(phase, "vcq-descriptor-digest-mismatch"));
        }
        check_size(presentation, admission.resources(), phase)?;
        let receipt: Presentation = serde_json::from_slice(&presentation.receipt)
            .map_err(|_| invalid(phase, "vcq-receipt-decoding"))?;
        let expected = expected_v3_request(request, admission.descriptor(), phase)?;
        let mut bridge = OriginalChallenge::new(expected.nonce, request, challenges);
        let journal_bytes = &receipt.receipt.journal.bytes;
        let outcome = verify_checked_with_artifact(
            &receipt,
            &expected,
            &mut bridge,
            &self.guest,
            |journal| claim(request, &admission, journal, journal_bytes),
        );
        bridge.finish(outcome.map(|(_, claim)| claim))
    }
}

fn shape_error(error: &ShapeError, phase: Phase) -> ProtocolError {
    match error {
        ShapeError::QueryBytes { len } => ProtocolError::capacity(
            phase,
            CapacityBound::Backend {
                name: "v3-query-bytes",
                requested: u64::try_from(*len).unwrap_or(u64::MAX),
                ceiling: MAX_QUERY_BYTES as u64,
            },
        ),
        ShapeError::Parse => invalid(phase, "vcq-query-parse"),
        ShapeError::Version => unsupported(phase, "vcq-query-version"),
        // The diagnostic text is never read: an unadmitted query is unsupported.
        ShapeError::NotAdmitted(_) => unsupported(phase, "vcq-query-not-admitted"),
    }
}

/// Checks the encoded presentation length; runs before the receipt is decoded.
fn check_size(
    presentation: &VcqPresentation,
    bounds: ResourceBounds,
    phase: Phase,
) -> Result<(), ProtocolError> {
    let requested = presentation.encoded_len();
    let ceiling = u64::from(bounds.presentation_bytes());
    if requested > ceiling {
        return Err(ProtocolError::capacity(
            phase,
            CapacityBound::Backend {
                name: "vcq-presentation-bytes",
                requested,
                ceiling,
            },
        ));
    }
    Ok(())
}

/// Checks released rows from the verified journal; runs before challenge consumption.
fn check_rows(rows: usize, bound: u32) -> Result<(), ProtocolError> {
    let requested = u64::try_from(rows).unwrap_or(u64::MAX);
    if requested > u64::from(bound) {
        return Err(ProtocolError::capacity(
            Phase::Verify,
            CapacityBound::Backend {
                name: "vcq-released-rows",
                requested,
                ceiling: u64::from(bound),
            },
        ));
    }
    Ok(())
}

/// Checks the verified journal result against the admitted contract.
fn released_result(
    contract: ResultContract,
    result: &v3::CanonicalResult,
    released_rows: u32,
) -> Result<ReleasedResult, ProtocolError> {
    let invalid = |code| invalid(Phase::Verify, code);
    match (contract, result) {
        (
            ResultContract::SelectBag,
            v3::CanonicalResult::Select {
                variables,
                order: RowOrder::Bag,
                rows,
            },
        ) => {
            if (1..variables.len()).any(|index| variables[..index].contains(&variables[index])) {
                return Err(invalid("vcq-duplicate-variable"));
            }
            if rows.iter().any(|row| row.len() != variables.len()) {
                return Err(invalid("vcq-row-arity"));
            }
            if rows.windows(2).any(|pair| pair[0] > pair[1]) {
                return Err(invalid("vcq-bag-order"));
            }
            check_rows(rows.len(), released_rows)?;
            Ok(ReleasedResult::SelectBag {
                variables: variables.clone(),
                rows: rows.clone(),
            })
        }
        (ResultContract::AskBoolean, v3::CanonicalResult::Ask(value)) => {
            Ok(ReleasedResult::Ask(*value))
        }
        (ResultContract::GraphRdfc10, v3::CanonicalResult::Graph { ntriples }) => {
            if !ntriples.is_empty() && !ntriples.ends_with('\n') {
                return Err(invalid("vcq-graph-framing"));
            }
            check_rows(ntriples.bytes().filter(|byte| *byte == b'\n').count(), released_rows)?;
            Ok(ReleasedResult::Graph {
                ntriples: ntriples.clone(),
            })
        }
        _ => Err(invalid("vcq-result-kind-mismatch")),
    }
}

/// Builds the claim from a VERIFIED, request-bound journal; runs before consumption.
fn claim(
    request: &StoredRequest,
    admission: &Admission,
    journal: &v3::Journal,
    journal_bytes: &[u8],
) -> Result<VerifiedClaim<V3Output>, ProtocolError> {
    let phase = Phase::Verify;
    let requirements = request.requirements();
    let mut established = vec![Obligation::Mapping, Obligation::Linking, Obligation::Query];
    match requirements.authority() {
        ScopeAuthority::VerifierAgreedAnchor => {
            let anchor = requirements
                .anchor()
                .ok_or_else(|| invalid(phase, "vcq-anchor-missing"))?;
            if journal.provenance != Provenance::VerifierAcceptedCommitment
                || journal.dataset_commitment != *anchor.as_bytes()
            {
                return Err(invalid(phase, "vcq-anchor-mismatch"));
            }
            established.push(Obligation::Anchor);
        }
        ScopeAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderDeclaredOnly {
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
        result: V3Output {
            result,
            provenance: journal.provenance.clone(),
            dataset_commitment: journal.dataset_commitment,
            v3_request_digest: journal.request_digest,
        },
        statement_digest: Digest32::new(statement)
            .map_err(|_| invalid(phase, "vcq-zero-statement-digest"))?,
        established,
    };
    VerifiedClaim::new(admission, evidence)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BridgeState {
    NotCalled,
    NonceMismatch,
    Consumed(Result<(), ProtocolError>),
}

/// Private [`Nonces`] bridge to the shared store's ORIGINAL challenge.
///
/// Accepts one call with the expected derived nonce, then consumes the stored
/// request's original challenge once and records the typed outcome.
struct OriginalChallenge<'a> {
    expected: [u8; 32],
    request: &'a StoredRequest,
    store: &'a dyn ChallengeStore,
    state: BridgeState,
    repeated: bool,
}

impl<'a> OriginalChallenge<'a> {
    fn new(expected: [u8; 32], request: &'a StoredRequest, store: &'a dyn ChallengeStore) -> Self {
        Self {
            expected,
            request,
            store,
            state: BridgeState::NotCalled,
            repeated: false,
        }
    }

    /// Classifies the V3 outcome from the recorded store outcome only.
    fn finish<T>(
        self,
        outcome: Result<T, CheckedFailure<ProtocolError>>,
    ) -> Result<T, ProtocolError> {
        let violation = invalid(Phase::Verify, "vcq-nonce-bridge-violation");
        if self.repeated {
            return Err(violation);
        }
        match (outcome, self.state) {
            (Ok(value), BridgeState::Consumed(Ok(()))) => Ok(value),
            (Err(CheckedFailure::Check(error)), BridgeState::NotCalled) => Err(error),
            (Err(CheckedFailure::Verification(_)), BridgeState::NotCalled) => {
                Err(invalid(Phase::Verify, "vcq-proof-rejected"))
            }
            (Err(CheckedFailure::Verification(_)), BridgeState::Consumed(Err(error))) => {
                Err(error)
            }
            _ => Err(violation),
        }
    }
}

impl Nonces for OriginalChallenge<'_> {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        if self.state != BridgeState::NotCalled {
            self.repeated = true;
            return Err(Error("vcq nonce bridge called more than once"));
        }
        if nonce != self.expected {
            self.state = BridgeState::NonceMismatch;
            return Err(Error("vcq derived nonce mismatch"));
        }
        let outcome = consume_challenge(self.store, self.request);
        self.state = BridgeState::Consumed(outcome);
        match outcome {
            Ok(()) => Ok(true),
            Err(error) if error.code() == ErrorCode::ChallengeReplayed => Ok(false),
            Err(_) => Err(Error("challenge store failure")),
        }
    }
}

impl QueryMethod for Risc0ExactV3 {
    type Request = StoredRequest;
    type PrivateInputs = v3::PrivateDataset;
    type Witness = V3Witness;
    type Presentation = VcqPresentation;
    type Output = V3Output;
    type ChallengeStore = dyn ChallengeStore;

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Checks the request and private dataset and builds the V3 witness.
    ///
    /// # Errors
    /// Admission and query-shape errors as in verification (at
    /// [`Phase::Prepare`]); `invalid` when V3 rejects the private dataset;
    /// `unsatisfiable` when it does not open the verifier-agreed anchor.
    fn prepare(
        &self,
        request: &StoredRequest,
        admission: &Admission,
        private: v3::PrivateDataset,
    ) -> Result<PreparedWitness<V3Witness>, ProtocolError> {
        let phase = Phase::Prepare;
        let admission = self.check_request(request, admission, phase)?;
        let expected = expected_v3_request(request, admission.descriptor(), phase)?;
        // The V3 diagnostic is not classified: capacity is never inferred from it.
        let commitment = v3::dataset_commitment(&private, &expected.policy)
            .map_err(|_| invalid(phase, "vcq-private-dataset-rejected"))?;
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
        let inner = V3Witness {
            witness: v3::Witness {
                request: expected,
                dataset: private,
            },
            resources: admission.resources(),
        };
        Ok(PreparedWitness::new(&admission, inner))
    }

    /// Proves the witness with the accepted guest and the configured `r0vm`.
    ///
    /// # Errors
    /// `invalid` for a witness of another descriptor; `infrastructure` for a
    /// missing `r0vm` or any proving failure (never classified as capacity);
    /// `capacity` when the presentation exceeds the request's byte bound.
    fn prove(&self, witness: PreparedWitness<V3Witness>) -> Result<VcqPresentation, ProtocolError> {
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

    /// Verifies with the configured audience and clock; see [`Risc0ExactV3::verify_at`].
    ///
    /// # Errors
    /// `invalid` when no verifier audience is configured; otherwise as
    /// [`Risc0ExactV3::verify_at`].
    fn verify(
        &self,
        request: &StoredRequest,
        admission: &Admission,
        presentation: &VcqPresentation,
        challenges: &Self::ChallengeStore,
    ) -> Result<VerifiedClaim<V3Output>, ProtocolError> {
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
    //! Unit doubles only: these tests create and accept no proof.
    use super::*;
    use sparq_query_protocol::{
        Challenge32, ChallengeOutcome, ChallengeStoreError, FailureClass, QueryRequirements,
        RequirementsSpec, StoredRequestSpec,
    };
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory test double; never a production store.
    #[derive(Default)]
    struct Store {
        seen: Mutex<HashSet<[u8; 32]>>,
        calls: AtomicUsize,
        broken: bool,
    }

    impl ChallengeStore for Store {
        fn consume(
            &self,
            challenge: &Challenge32,
        ) -> Result<ChallengeOutcome, ChallengeStoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.broken {
                return Err(ChallengeStoreError::new("test-broken"));
            }
            let mut seen = self.seen.lock().expect("test store lock");
            Ok(if seen.insert(*challenge.as_bytes()) {
                ChallengeOutcome::Fresh
            } else {
                ChallengeOutcome::AlreadyConsumed
            })
        }
    }

    fn pin(seed: u8) -> ArtifactPin {
        ArtifactPin {
            sha256: [seed; 32],
            image_id: [u32::from(seed); 8],
        }
    }

    fn stored(descriptor: &MethodDescriptor) -> StoredRequest {
        let id = |text| Identifier::new(text).unwrap();
        let requirements = QueryRequirements::new(RequirementsSpec {
            contract: ResultContract::AskBoolean,
            mode: EvaluationMode::ExactBounded,
            authority: ScopeAuthority::HolderDeclared,
            anchor: None,
            source_evidence: Some(SourceEvidence::None),
            status: Some(StatusPolicy::NotRequested),
            holder: Some(HolderPolicy::BearerAccepted),
            assembly: DatasetAssembly::ExactSourceCatalog,
            query_profile: QueryProfile {
                dialect: id(QUERY_DIALECT),
                fragment: id(QUERY_FRAGMENT),
            },
            accepted_suites: vec![],
            accepted_mappings: vec![id(MAPPING_PROFILE)],
            accepted_linking: vec![id(LINKING_PROFILE)],
            resources: ResourceBounds::new(4, 1 << 20).unwrap(),
            methods: vec![descriptor.clone()],
        })
        .unwrap();
        StoredRequest::new(StoredRequestSpec {
            requirements,
            query: "ASK { ?s ?p ?o }".to_owned(),
            challenge: Challenge32::new([9; 32]).unwrap(),
            audience: id("urn:example:verifier"),
            not_before: 10,
            not_after: 20,
            form: QueryForm::Ask,
            base_iri: None,
            describe_policy: None,
        })
        .unwrap()
    }

    fn verification_error() -> CheckedFailure<ProtocolError> {
        CheckedFailure::Verification(Error("challenge already consumed"))
    }

    #[test]
    fn bridge_consumes_the_original_challenge_exactly_once() {
        let descriptor = descriptor(&pin(1)).unwrap();
        let request = stored(&descriptor);
        let nonce = derive_nonce(&request, &descriptor);
        let store = Store::default();
        let mut bridge = OriginalChallenge::new(nonce, &request, &store);
        assert_eq!(bridge.consume(nonce), Ok(true));
        assert!(bridge.consume(nonce).is_err(), "second call rejected");
        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
        assert!(store.seen.lock().unwrap().contains(request.challenge().as_bytes()));
        assert!(!store.seen.lock().unwrap().contains(&nonce), "derived nonce never stored");
        let error = bridge.finish(Ok(())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Backend("vcq-nonce-bridge-violation"));
    }

    #[test]
    fn bridge_rejects_a_foreign_nonce_without_touching_the_store() {
        let descriptor = descriptor(&pin(1)).unwrap();
        let request = stored(&descriptor);
        let nonce = derive_nonce(&request, &descriptor);
        let store = Store::default();
        let mut bridge = OriginalChallenge::new(nonce, &request, &store);
        let mut other = nonce;
        other[0] ^= 1;
        assert!(bridge.consume(other).is_err());
        assert_eq!(store.calls.load(Ordering::SeqCst), 0);
        let error = bridge.finish::<()>(Err(verification_error())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Backend("vcq-nonce-bridge-violation"));
    }

    #[test]
    fn replay_and_store_failure_use_typed_outcomes() {
        let descriptor = descriptor(&pin(1)).unwrap();
        let request = stored(&descriptor);
        let nonce = derive_nonce(&request, &descriptor);
        let store = Store::default();
        store.consume(request.challenge()).unwrap();
        let mut bridge = OriginalChallenge::new(nonce, &request, &store);
        assert_eq!(bridge.consume(nonce), Ok(false));
        let replay = bridge.finish::<()>(Err(verification_error())).unwrap_err();
        assert_eq!(replay.code(), ErrorCode::ChallengeReplayed);
        assert_eq!(replay.class(), FailureClass::Invalid);

        let broken = Store {
            broken: true,
            ..Store::default()
        };
        let mut bridge = OriginalChallenge::new(nonce, &request, &broken);
        assert!(bridge.consume(nonce).is_err());
        let failure = bridge
            .finish::<()>(Err(CheckedFailure::Verification(Error("any text"))))
            .unwrap_err();
        assert_eq!(failure.code(), ErrorCode::ChallengeStoreFailure("test-broken"));
        assert_eq!(failure.class(), FailureClass::Infrastructure);
    }

    #[test]
    fn two_methods_share_one_original_challenge() {
        let (first, second) = (descriptor(&pin(1)).unwrap(), descriptor(&pin(2)).unwrap());
        let request = stored(&first);
        let (a, b) = (derive_nonce(&request, &first), derive_nonce(&request, &second));
        assert_ne!(a, b, "per-method nonces differ");
        let store = Store::default();
        assert_eq!(OriginalChallenge::new(a, &request, &store).consume(a), Ok(true));
        assert_eq!(OriginalChallenge::new(b, &request, &store).consume(b), Ok(false));
    }

    #[test]
    fn check_failure_before_consumption_keeps_the_challenge() {
        let descriptor = descriptor(&pin(1)).unwrap();
        let request = stored(&descriptor);
        let store = Store::default();
        let bridge = OriginalChallenge::new(derive_nonce(&request, &descriptor), &request, &store);
        let rejected = invalid(Phase::Verify, "vcq-result-kind-mismatch");
        let error = bridge.finish::<()>(Err(CheckedFailure::Check(rejected))).unwrap_err();
        assert_eq!(error, rejected);
        let bridge = OriginalChallenge::new([1; 32], &request, &store);
        let error = bridge.finish::<()>(Err(verification_error())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Backend("vcq-proof-rejected"));
        assert_eq!(store.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn pre_epoch_clock_reads_as_expired_not_zero() {
        use std::time::Duration;
        let pre_epoch = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
        assert_eq!(unix_seconds(pre_epoch), u64::MAX);
        assert_eq!(unix_seconds(UNIX_EPOCH + Duration::from_secs(7)), 7);
    }

    fn select(order: RowOrder, variables: &[&str], rows: &[&[Option<&str>]]) -> v3::CanonicalResult {
        v3::CanonicalResult::Select {
            variables: variables.iter().map(|v| (*v).to_owned()).collect(),
            order,
            rows: rows
                .iter()
                .map(|row| row.iter().map(|cell| cell.map(str::to_owned)).collect())
                .collect(),
        }
    }

    fn code(result: Result<ReleasedResult, ProtocolError>) -> ErrorCode {
        result.unwrap_err().code()
    }

    #[test]
    fn result_must_match_the_exact_contract_form_and_bounds() {
        let a = Some("<http://ex/a>");
        let bag = select(RowOrder::Bag, &["s"], &[&[a], &[a]]);
        let released = released_result(ResultContract::SelectBag, &bag, 2).unwrap();
        assert!(matches!(released, ReleasedResult::SelectBag { ref rows, .. } if rows.len() == 2));
        let bad = |code: &'static str| ErrorCode::Backend(code);
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &bag, 1)),
            ErrorCode::CapacityExceeded(CapacityBound::Backend {
                name: "vcq-released-rows",
                requested: 2,
                ceiling: 1,
            })
        );
        let sequence = select(RowOrder::Sequence, &["s"], &[&[a]]);
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &sequence, 4)),
            bad("vcq-result-kind-mismatch")
        );
        let duplicate = select(RowOrder::Bag, &["s", "s"], &[&[a, a]]);
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &duplicate, 4)),
            bad("vcq-duplicate-variable")
        );
        let arity = select(RowOrder::Bag, &["s", "o"], &[&[a]]);
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &arity, 4)),
            bad("vcq-row-arity")
        );
        let unsorted = select(RowOrder::Bag, &["s"], &[&[Some("<http://ex/b>")], &[a]]);
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &unsorted, 4)),
            bad("vcq-bag-order")
        );
        let ask = v3::CanonicalResult::Ask(false);
        assert_eq!(
            released_result(ResultContract::AskBoolean, &ask, 1),
            Ok(ReleasedResult::Ask(false))
        );
        assert_eq!(
            code(released_result(ResultContract::SelectBag, &ask, 4)),
            bad("vcq-result-kind-mismatch")
        );
        assert_eq!(
            code(released_result(ResultContract::AskBoolean, &bag, 4)),
            bad("vcq-result-kind-mismatch")
        );
        let graph = v3::CanonicalResult::Graph {
            ntriples: "<http://ex/a> <http://ex/p> _:c14n0 .\n_:c14n0 <http://ex/p> \"x\" .\n"
                .to_owned(),
        };
        assert!(released_result(ResultContract::GraphRdfc10, &graph, 2).is_ok());
        assert!(matches!(
            code(released_result(ResultContract::GraphRdfc10, &graph, 1)),
            ErrorCode::CapacityExceeded(_)
        ));
        assert_eq!(
            code(released_result(ResultContract::AskBoolean, &graph, 4)),
            bad("vcq-result-kind-mismatch")
        );
        let unframed = v3::CanonicalResult::Graph {
            ntriples: "<http://ex/a> <http://ex/p> <http://ex/b> .".to_owned(),
        };
        assert_eq!(
            code(released_result(ResultContract::GraphRdfc10, &unframed, 4)),
            bad("vcq-graph-framing")
        );
    }
}

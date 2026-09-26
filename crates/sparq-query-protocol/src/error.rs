//! Typed failures carrying a class, a phase and a code (draft §6.4).
//!
//! Classes are chosen by the code path that detects the failure, never by
//! parsing message text. Backend failures use [`ProtocolError::backend`], whose
//! class type has no capacity variant: only [`ProtocolError::capacity`], which
//! must name the exhausted bound, can report `capacity`. [OPUS-5.5]

use core::fmt;

use crate::descriptor::Obligation;

/// Failure class from the draft §6.4 table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureClass {
    /// Outside declared capabilities; decided from public inputs.
    Unsupported,
    /// Admissible but beyond a named, declared bound.
    Capacity,
    /// Prover side only: no witness supports the result.
    Unsatisfiable,
    /// Trust policy excludes an input.
    PolicyRejected,
    /// A request, configuration or presentation fails a check.
    Invalid,
    /// Environment failure; never acceptance and never `invalid`.
    Infrastructure,
}

/// Protocol phase in which a failure was detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Verifier-side request validation.
    Request,
    /// Descriptor selection against the request's exact method list.
    Negotiation,
    /// Credential import (adapter-owned; unused by this crate).
    Import,
    /// Deterministic admission over public data.
    Admit,
    /// Witness preparation by the backend.
    Prepare,
    /// Proof generation by the backend.
    Prove,
    /// Presentation verification by the backend.
    Verify,
}

/// Request policy field that has no default (draft §5.1 rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolicyField {
    /// `scope.source_evidence`.
    SourceEvidence,
    /// `trust.status`.
    Status,
    /// `trust.holder`.
    Holder,
}

/// The declared bound a `capacity` failure exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapacityBound {
    /// Requested released-row bound exceeds the method ceiling.
    ReleasedRows {
        /// Verifier request bound.
        requested: u32,
        /// Method ceiling.
        ceiling: u32,
    },
    /// Requested presentation-size bound exceeds the method ceiling.
    PresentationBytes {
        /// Verifier request bound.
        requested: u32,
        /// Method ceiling.
        ceiling: u32,
    },
    /// A backend-specific bound such as a circuit bucket or search budget.
    Backend {
        /// Stable bound name chosen by the backend.
        name: &'static str,
        /// Amount the input required.
        requested: u64,
        /// Declared ceiling.
        ceiling: u64,
    },
}

/// Machine-readable failure code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// An identifier is empty, too long, wildcarded or not `scheme:rest`.
    MalformedIdentifier,
    /// A method version is zero.
    ZeroVersion,
    /// A digest is all zero bytes, the usual unset placeholder.
    ZeroDigest,
    /// A resource bound or ceiling is zero.
    ZeroBound,
    /// A policy with no default was omitted.
    MissingPolicy(PolicyField),
    /// The request lists no acceptable descriptor.
    EmptyMethodList,
    /// The request lists one descriptor twice.
    DuplicateDescriptor,
    /// A required accepted-suite, mapping or linking set is empty.
    EmptyAcceptedSet,
    /// A dataset anchor is present without anchor authority, or missing with it.
    AnchorMismatch,
    /// The result contract is not defined under the evaluation mode (§5.2).
    ContractModeMismatch,
    /// A method's capability declaration violates a structural rule.
    MalformedCapabilities(&'static str),
    /// The selected descriptor is not exactly one the request lists.
    DescriptorNotRequested,
    /// The local backend implements a different descriptor.
    DescriptorUnavailable,
    /// No executable vcq adapter exists for the descriptor.
    AdapterUnavailable,
    /// No declared tuple names the request's exact query profile.
    QueryProfileUnsupported,
    /// No single declared capability tuple matches the request.
    TupleUnsupported,
    /// More than one declared tuple matches, so the choice is not determined.
    TupleAmbiguous,
    /// A required obligation has no enforcer in the matching tuple.
    UnenforcedObligation(Obligation),
    /// A backend claim omits a required obligation.
    ObligationNotEstablished(Obligation),
    /// A witness or claim was produced under another admission.
    AdmissionMismatch,
    /// A declared bound was exceeded.
    CapacityExceeded(CapacityBound),
    /// A backend-defined stable code.
    Backend(&'static str),
    /// [OPUS-5.5] A request challenge is all zero bytes.
    ZeroChallenge,
    /// A query is empty or longer than [`MAX_QUERY_LEN`](crate::MAX_QUERY_LEN).
    MalformedQuery,
    /// A base IRI is empty, too long, or has whitespace or control characters.
    MalformedBaseIri,
    /// A validity window does not satisfy `not_before < not_after`.
    InvalidValidityWindow,
    /// The query form does not take the requested result contract.
    FormContractMismatch,
    /// A DESCRIBE policy is missing for DESCRIBE or present for another form.
    DescribePolicyMismatch,
    /// The original challenge was already consumed (replay).
    ChallengeReplayed,
    /// The challenge store failed; carries the store's stable code.
    ChallengeStoreFailure(&'static str),
}

/// Backend failure classes other than `capacity`.
///
/// Capacity must name its bound and is reported with [`ProtocolError::capacity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendFailure {
    /// See [`FailureClass::Unsupported`].
    Unsupported,
    /// See [`FailureClass::Unsatisfiable`].
    Unsatisfiable,
    /// See [`FailureClass::PolicyRejected`].
    PolicyRejected,
    /// See [`FailureClass::Invalid`].
    Invalid,
    /// See [`FailureClass::Infrastructure`].
    Infrastructure,
}

/// A rejected protocol step: class, phase and code.
///
/// Every outcome other than a [`VerifiedClaim`](crate::VerifiedClaim) rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolError {
    class: FailureClass,
    phase: Phase,
    code: ErrorCode,
}

impl ProtocolError {
    pub(crate) fn new(class: FailureClass, phase: Phase, code: ErrorCode) -> Self {
        Self { class, phase, code }
    }

    /// Reports a `capacity` failure that names the exhausted `bound`.
    #[must_use]
    pub fn capacity(phase: Phase, bound: CapacityBound) -> Self {
        Self::new(FailureClass::Capacity, phase, ErrorCode::CapacityExceeded(bound))
    }

    /// Reports a backend failure with a stable backend `code`.
    ///
    /// An error whose cause the backend cannot classify should use the least
    /// specific applicable class; it can never certify `capacity`.
    #[must_use]
    pub fn backend(class: BackendFailure, phase: Phase, code: &'static str) -> Self {
        let class = match class {
            BackendFailure::Unsupported => FailureClass::Unsupported,
            BackendFailure::Unsatisfiable => FailureClass::Unsatisfiable,
            BackendFailure::PolicyRejected => FailureClass::PolicyRejected,
            BackendFailure::Invalid => FailureClass::Invalid,
            BackendFailure::Infrastructure => FailureClass::Infrastructure,
        };
        Self::new(class, phase, ErrorCode::Backend(code))
    }

    /// Returns the failure class.
    #[must_use]
    pub fn class(&self) -> FailureClass {
        self.class
    }

    /// Returns the phase that detected the failure.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Returns the failure code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at {:?}: {:?}", self.class, self.phase, self.code)
    }
}

impl std::error::Error for ProtocolError {}

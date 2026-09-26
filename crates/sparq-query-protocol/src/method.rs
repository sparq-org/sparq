//! Query-method trait, opaque prepared witnesses and verified claims (draft §6.2, §6.3).
//!
//! Implementations of [`QueryMethod`] are TRUSTED local backends. Implementing
//! the trait proves nothing about their correctness: this crate cannot check
//! that `verify` runs a real proof check, and a [`VerifiedClaim`] is exactly
//! as trustworthy as the backend that built it. This crate itself never builds a
//! claim, computes a digest or verifies a proof. [OPUS-5.5]

use core::fmt;

use crate::descriptor::{
    CapabilityTuple, Capabilities, Completeness, Enforcer, EvaluationMode, MethodDescriptor,
    Obligation, ResultContract, ScopeAuthority, SourceEvidence,
};
use crate::error::{ErrorCode, FailureClass, Phase, ProtocolError};
use crate::ids::{Digest32, Identifier};
use crate::negotiate::{admit, Admission, QueryRequirements};

/// Backend-owned witness, created under an admission and consumed by `prove`.
///
/// It implements neither `Clone` nor serialization, and its `Debug` output
/// shows only the descriptor, never the inner witness.
pub struct PreparedWitness<W> {
    descriptor: MethodDescriptor,
    inner: W,
}

impl<W> PreparedWitness<W> {
    /// Wraps backend witness `inner` prepared under `admission`.
    #[must_use]
    pub fn new(admission: &Admission, inner: W) -> Self {
        Self {
            descriptor: admission.descriptor().clone(),
            inner,
        }
    }

    /// Returns the descriptor the witness was prepared for.
    #[must_use]
    pub fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Consumes the wrapper, releasing the witness only to `descriptor`'s backend.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Prove`] with
    /// [`ErrorCode::AdmissionMismatch`] when the witness was prepared for
    /// another descriptor; the witness is dropped.
    pub fn into_inner_for(self, descriptor: &MethodDescriptor) -> Result<W, ProtocolError> {
        if self.descriptor != *descriptor {
            return Err(ProtocolError::new(
                FailureClass::Invalid,
                Phase::Prove,
                ErrorCode::AdmissionMismatch,
            ));
        }
        Ok(self.inner)
    }
}

impl<W> fmt::Debug for PreparedWitness<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedWitness")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// Per-obligation outcome in a claim.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ObligationOutcome {
    /// The backend reports discharging it with this admitted enforcer.
    Established(Enforcer),
    /// Not owed by the request, so nothing is asserted.
    NotEstablished,
}

/// Scope metadata carried by a claim (draft §5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClaimScope {
    /// Who chose the dataset.
    pub authority: ScopeAuthority,
    /// Verifier anchor, under anchor authority.
    pub anchor: Option<Digest32>,
    /// What authenticates the inputs.
    pub source_evidence: SourceEvidence,
    /// Completeness relative to scope, or none.
    pub completeness: Completeness,
}

/// What a backend reports after its own cryptographic verification.
#[derive(Clone, Debug)]
pub struct ClaimEvidence<R> {
    /// Typed result taken from the proved statement.
    pub result: R,
    /// Statement digest the backend recomputed under its local encoding.
    pub statement_digest: Digest32,
    /// Obligations the backend's checks discharged.
    pub established: Vec<Obligation>,
}

/// Typed result plus scope, evidence, enforcement and disclosure metadata.
///
/// Scope, source evidence and completeness come from the admitted tuple, and
/// each owed obligation must appear in the backend's evidence. Obligations the
/// request did not owe are always `NotEstablished`, whatever the backend says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedClaim<R> {
    descriptor: MethodDescriptor,
    contract: ResultContract,
    mode: EvaluationMode,
    result: R,
    statement_digest: Digest32,
    scope: ClaimScope,
    tuple: CapabilityTuple,
    obligations: Vec<(Obligation, ObligationOutcome)>,
    disclosure: Vec<Identifier>,
}

impl<R> VerifiedClaim<R> {
    /// Builds a claim from a backend's `evidence` under `admission`.
    ///
    /// Only a backend `verify` that has checked the proof should call this.
    /// The function checks coverage of owed obligations, not the proof.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Verify`] with
    /// [`ErrorCode::ObligationNotEstablished`] when an owed obligation is
    /// missing from `evidence.established`.
    pub fn new(admission: &Admission, evidence: ClaimEvidence<R>) -> Result<Self, ProtocolError> {
        let mut obligations = Vec::with_capacity(Obligation::ALL.len());
        for obligation in Obligation::ALL {
            let outcome = match admission.enforcer(obligation) {
                Some(enforcer) if evidence.established.contains(&obligation) => {
                    ObligationOutcome::Established(enforcer.clone())
                }
                Some(_) => {
                    return Err(ProtocolError::new(
                        FailureClass::Invalid,
                        Phase::Verify,
                        ErrorCode::ObligationNotEstablished(obligation),
                    ))
                }
                None => ObligationOutcome::NotEstablished,
            };
            obligations.push((obligation, outcome));
        }
        let tuple = admission.tuple();
        Ok(Self {
            descriptor: admission.descriptor().clone(),
            contract: tuple.contract,
            mode: tuple.mode,
            result: evidence.result,
            statement_digest: evidence.statement_digest,
            scope: ClaimScope {
                authority: tuple.authority,
                anchor: admission.anchor().copied(),
                source_evidence: tuple.source_evidence,
                completeness: admission.completeness(),
            },
            tuple: tuple.clone(),
            obligations,
            disclosure: admission.disclosure().to_vec(),
        })
    }

    /// Returns the typed result.
    #[must_use]
    pub fn result(&self) -> &R {
        &self.result
    }

    /// Consumes the claim, returning the typed result.
    #[must_use]
    pub fn into_result(self) -> R {
        self.result
    }

    /// Returns the descriptor that produced the claim.
    #[must_use]
    pub fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Returns the result contract.
    #[must_use]
    pub fn contract(&self) -> ResultContract {
        self.contract
    }

    /// Returns the evaluation mode.
    #[must_use]
    pub fn mode(&self) -> EvaluationMode {
        self.mode
    }

    /// Returns the backend-supplied statement digest.
    #[must_use]
    pub fn statement_digest(&self) -> &Digest32 {
        &self.statement_digest
    }

    /// Returns scope, source evidence and completeness.
    #[must_use]
    pub fn scope(&self) -> &ClaimScope {
        &self.scope
    }

    /// Returns the admitted tuple, including suite, mapping and linking profiles.
    #[must_use]
    pub fn tuple(&self) -> &CapabilityTuple {
        &self.tuple
    }

    /// Returns the outcome for `obligation`.
    #[must_use]
    pub fn outcome(&self, obligation: Obligation) -> &ObligationOutcome {
        // `new` records every obligation, so the fallback is never taken.
        const NOT_ESTABLISHED: &ObligationOutcome = &ObligationOutcome::NotEstablished;
        self.obligations
            .iter()
            .find(|(held, _)| *held == obligation)
            .map_or(NOT_ESTABLISHED, |(_, outcome)| outcome)
    }

    /// Returns every obligation outcome in [`Obligation::ALL`] order.
    #[must_use]
    pub fn obligations(&self) -> &[(Obligation, ObligationOutcome)] {
        &self.obligations
    }

    /// Returns the disclosure inventory.
    #[must_use]
    pub fn disclosure(&self) -> &[Identifier] {
        &self.disclosure
    }
}

/// A pluggable query proof method (draft §6.2 *QueryMethod*).
///
/// `Request` is the verifier-stored request. [OPUS-5.5] Adapters should use
/// [`StoredRequest`](crate::StoredRequest), or a type wrapping it, so the
/// query text, original challenge, audience and validity window come from one
/// validated value. `ChallengeStore` should be `dyn`
/// [`ChallengeStore`](crate::ChallengeStore) so every method of one verifier
/// shares one store namespace without tying backends to one storage library.
/// Implementations are trusted: nothing here checks that `prove` or `verify`
/// does what it claims.
pub trait QueryMethod {
    /// Verifier-stored request; recommended [`StoredRequest`](crate::StoredRequest).
    type Request: ?Sized;
    /// Holder-private inputs to `prepare`.
    type PrivateInputs;
    /// Backend witness wrapped in [`PreparedWitness`].
    type Witness;
    /// Presentation produced by `prove`.
    type Presentation;
    /// Typed result carried in a [`VerifiedClaim`].
    type Output;
    /// Shared store the declared owner consumes the ORIGINAL challenge from.
    ///
    /// Recommended: `dyn` [`ChallengeStore`](crate::ChallengeStore).
    type ChallengeStore: ?Sized;

    /// Returns the backend's validated capabilities.
    fn capabilities(&self) -> &Capabilities;

    /// Returns the exact descriptor this backend implements.
    fn descriptor(&self) -> &MethodDescriptor {
        self.capabilities().descriptor()
    }

    /// Deterministically admits `selected` against `requirements`.
    ///
    /// # Errors
    /// As [`admit`].
    fn admit(
        &self,
        requirements: &QueryRequirements,
        selected: &MethodDescriptor,
    ) -> Result<Admission, ProtocolError> {
        admit(requirements, selected, self.capabilities())
    }

    /// Prepares an opaque witness from `private` inputs.
    ///
    /// # Errors
    /// `unsupported`, `unsatisfiable`, `capacity`, `policy-rejected` or
    /// `infrastructure` at [`Phase::Prepare`].
    fn prepare(
        &self,
        request: &Self::Request,
        admission: &Admission,
        private: Self::PrivateInputs,
    ) -> Result<PreparedWitness<Self::Witness>, ProtocolError>;

    /// Consumes `witness` and produces a presentation.
    ///
    /// # Errors
    /// `capacity` or `infrastructure` at [`Phase::Prove`], or `invalid` for a
    /// witness prepared for another descriptor.
    fn prove(
        &self,
        witness: PreparedWitness<Self::Witness>,
    ) -> Result<Self::Presentation, ProtocolError>;

    /// Verifies `presentation` against the verifier's stored `request`.
    ///
    /// `admission` must be recomputed by the verifier, never taken from a holder.
    /// When this method owns the challenge it consumes the request's original
    /// challenge from `challenges` exactly once, never a derived nonce.
    ///
    /// # Errors
    /// `invalid`, `unsupported`, `capacity` or `infrastructure` at [`Phase::Verify`].
    fn verify(
        &self,
        request: &Self::Request,
        admission: &Admission,
        presentation: &Self::Presentation,
        challenges: &Self::ChallengeStore,
    ) -> Result<VerifiedClaim<Self::Output>, ProtocolError>;
}

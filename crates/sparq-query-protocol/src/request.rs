//! Verifier-stored request: requirements plus query, challenge, audience and window (draft §5.1).
//!
//! [OPUS-5.5] [`StoredRequest`] joins the negotiation [`QueryRequirements`]
//! with the request fields a verifier checks against its own stored copy: the
//! exact query text, the original single-use [`Challenge32`], the audience,
//! the validity window, the declared [`QueryForm`], an optional base IRI and
//! an optional DESCRIBE policy. A presentation never supplies these values.
//!
//! This is still a SUBSET of the draft request. Issuer sets, status windows,
//! trust-policy roots, disclosure policy, the evaluation context other than
//! the DESCRIBE policy, and graph-size bounds are not modelled, and no
//! credential policy is enforced here. The first planned backend supports only
//! source evidence `none`, status `not-requested` and bearer holders.
//!
//! Every check is a SHAPE check. The query is not parsed, so [`QueryForm`] is
//! the verifier's declaration and an adapter must confirm that the parsed
//! query has that form. [`BaseIri`] is a bounded lexical string, not a
//! validated IRI; adapters owe IRI validation and support. Nothing here
//! computes a digest.

use core::fmt;

use crate::descriptor::ResultContract;
use crate::error::{ErrorCode, FailureClass, Phase, ProtocolError};
use crate::ids::Identifier;
use crate::negotiate::QueryRequirements;

/// Longest accepted query text, in bytes (1 MiB).
///
/// The bound keeps a stored request small enough to encode and compare in
/// memory. Raising it changes no other invariant.
pub const MAX_QUERY_LEN: usize = 1 << 20;

/// Longest accepted base-IRI string, in bytes.
///
/// Generous for real base IRIs while keeping documents out of the field.
/// Raising it changes no other invariant.
pub const MAX_BASE_IRI_LEN: usize = 4096;

fn invalid(code: ErrorCode) -> ProtocolError {
    ProtocolError::new(FailureClass::Invalid, Phase::Request, code)
}

/// Verifier-random, single-use 32-byte request challenge (draft §5.1).
///
/// A distinct type from [`Digest32`](crate::Digest32): a challenge is not a
/// digest. A method's per-method derived nonce must never be wrapped as a
/// `Challenge32`; the shared [`ChallengeStore`](crate::ChallengeStore)
/// consumes only the ORIGINAL challenge held in the [`StoredRequest`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Challenge32([u8; 32]);

impl Challenge32 {
    /// Wraps verifier-random challenge `bytes`.
    ///
    /// This does not check randomness; the verifier's generator owes that.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with [`ErrorCode::ZeroChallenge`]
    /// for all-zero bytes, the usual placeholder for an unset challenge.
    pub fn new(bytes: [u8; 32]) -> Result<Self, ProtocolError> {
        if bytes == [0; 32] {
            return Err(invalid(ErrorCode::ZeroChallenge));
        }
        Ok(Self(bytes))
    }

    /// Returns the challenge bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Challenge32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Challenge32(")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

/// An exact base-IRI string, checked for shape only.
///
/// Accepted values are 1 to [`MAX_BASE_IRI_LEN`] bytes with no whitespace and
/// no control characters. This is NOT RFC 3987 validation: no scheme, syntax
/// or resolution check runs, and adapters must validate and support the IRI
/// themselves. Comparison and encoding are byte-exact, with no normalization.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BaseIri(String);

impl BaseIri {
    /// Checks the shape of `value` and keeps it unchanged.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with
    /// [`ErrorCode::MalformedBaseIri`] when `value` is empty, too long, or
    /// contains whitespace or a control character.
    pub fn new(value: &str) -> Result<Self, ProtocolError> {
        let bad_char = value.chars().any(|c| c.is_whitespace() || c.is_control());
        if value.is_empty() || value.len() > MAX_BASE_IRI_LEN || bad_char {
            return Err(invalid(ErrorCode::MalformedBaseIri));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the exact base-IRI text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaseIri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Declared SPARQL query form (draft §5.1 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueryForm {
    /// SELECT.
    Select,
    /// ASK.
    Ask,
    /// CONSTRUCT.
    Construct,
    /// DESCRIBE; needs an explicit DESCRIBE policy.
    Describe,
}

impl QueryForm {
    /// Reports whether `contract` is a result contract for this form (draft §5.2).
    ///
    /// SELECT takes the three SELECT contracts, ASK the two ASK contracts, and
    /// CONSTRUCT and DESCRIBE the canonical-graph contract. Anything else fails.
    #[must_use]
    pub fn supports_contract(self, contract: ResultContract) -> bool {
        match contract {
            ResultContract::SelectDistinctSet
            | ResultContract::SelectBag
            | ResultContract::SelectSequence => self == Self::Select,
            ResultContract::AskBoolean | ResultContract::AskTrueOnly => self == Self::Ask,
            ResultContract::GraphRdfc10 => matches!(self, Self::Construct | Self::Describe),
        }
    }
}

/// Unvalidated stored-request fields passed to [`StoredRequest::new`].
#[derive(Clone, Debug)]
pub struct StoredRequestSpec {
    /// Validated negotiation requirements.
    pub requirements: QueryRequirements,
    /// Exact query text, never normalized.
    pub query: String,
    /// Original verifier-random challenge.
    pub challenge: Challenge32,
    /// Exact verifier audience.
    pub audience: Identifier,
    /// Start of validity, Unix seconds.
    pub not_before: u64,
    /// End of validity, Unix seconds; greater than `not_before`.
    pub not_after: u64,
    /// Declared query form.
    pub form: QueryForm,
    /// Explicit base IRI, or explicit none.
    pub base_iri: Option<BaseIri>,
    /// DESCRIBE policy; present exactly for [`QueryForm::Describe`].
    pub describe_policy: Option<Identifier>,
}

/// Validated, immutable verifier-stored request (subset of draft §5.1).
///
/// Adapters check audience, validity and challenge against this stored copy,
/// and consume [`StoredRequest::challenge`] through the shared store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRequest {
    // `pub(crate)` only so the local encoder can destructure exhaustively.
    pub(crate) requirements: QueryRequirements,
    pub(crate) query: String,
    pub(crate) challenge: Challenge32,
    pub(crate) audience: Identifier,
    pub(crate) not_before: u64,
    pub(crate) not_after: u64,
    pub(crate) form: QueryForm,
    pub(crate) base_iri: Option<BaseIri>,
    pub(crate) describe_policy: Option<Identifier>,
}

impl StoredRequest {
    /// Validates the stored-request fields in `spec`.
    ///
    /// Checks run in this order: query length, validity window, form against
    /// result contract, then DESCRIBE-policy presence.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with
    /// - [`ErrorCode::MalformedQuery`] for an empty query or one longer than
    ///   [`MAX_QUERY_LEN`] bytes;
    /// - [`ErrorCode::InvalidValidityWindow`] unless `not_before < not_after`;
    /// - [`ErrorCode::FormContractMismatch`] when the form does not take the
    ///   requirements' result contract;
    /// - [`ErrorCode::DescribePolicyMismatch`] when a DESCRIBE policy is
    ///   missing for DESCRIBE or present for another form.
    pub fn new(spec: StoredRequestSpec) -> Result<Self, ProtocolError> {
        if spec.query.is_empty() || spec.query.len() > MAX_QUERY_LEN {
            return Err(invalid(ErrorCode::MalformedQuery));
        }
        if spec.not_before >= spec.not_after {
            return Err(invalid(ErrorCode::InvalidValidityWindow));
        }
        if !spec.form.supports_contract(spec.requirements.contract()) {
            return Err(invalid(ErrorCode::FormContractMismatch));
        }
        if (spec.form == QueryForm::Describe) != spec.describe_policy.is_some() {
            return Err(invalid(ErrorCode::DescribePolicyMismatch));
        }
        Ok(Self {
            requirements: spec.requirements,
            query: spec.query,
            challenge: spec.challenge,
            audience: spec.audience,
            not_before: spec.not_before,
            not_after: spec.not_after,
            form: spec.form,
            base_iri: spec.base_iri,
            describe_policy: spec.describe_policy,
        })
    }

    /// Returns the negotiation requirements.
    #[must_use]
    pub fn requirements(&self) -> &QueryRequirements {
        &self.requirements
    }

    /// Returns the exact query text.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Returns the exact UTF-8 query bytes.
    #[must_use]
    pub fn query_bytes(&self) -> &[u8] {
        self.query.as_bytes()
    }

    /// Returns the original challenge, the only value the store consumes.
    #[must_use]
    pub fn challenge(&self) -> &Challenge32 {
        &self.challenge
    }

    /// Returns the exact audience.
    #[must_use]
    pub fn audience(&self) -> &Identifier {
        &self.audience
    }

    /// Returns the start of validity in Unix seconds.
    #[must_use]
    pub fn not_before(&self) -> u64 {
        self.not_before
    }

    /// Returns the end of validity in Unix seconds.
    #[must_use]
    pub fn not_after(&self) -> u64 {
        self.not_after
    }

    /// Returns the declared query form.
    #[must_use]
    pub fn form(&self) -> QueryForm {
        self.form
    }

    /// Returns the explicit base IRI, or `None` for explicit none.
    #[must_use]
    pub fn base_iri(&self) -> Option<&BaseIri> {
        self.base_iri.as_ref()
    }

    /// Returns the DESCRIBE policy, present only for [`QueryForm::Describe`].
    #[must_use]
    pub fn describe_policy(&self) -> Option<&Identifier> {
        self.describe_policy.as_ref()
    }
}

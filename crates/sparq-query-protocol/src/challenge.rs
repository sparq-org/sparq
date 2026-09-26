//! Shared single-use challenge-store contract (draft §6.4).
//!
//! [OPUS-5.5] One verifier shares ONE [`ChallengeStore`] namespace across all
//! of its query methods. Each verification has exactly one owner (the method
//! or the adapter, per [`ChallengePolicy`](crate::ChallengePolicy)) that calls
//! [`ChallengeStore::consume`] at most once, with the ORIGINAL
//! [`Challenge32`] from the [`StoredRequest`]. A method that derives its own
//! per-method nonce from the challenge must still consume the original, so
//! two methods cannot each accept one challenge under different nonces.
//!
//! The trait takes `&self`, so a store shared by concurrent verifications
//! implements consumption as an atomic check-and-set, typically in a durable
//! database. The trait itself enforces neither atomicity nor persistence:
//! implementations owe both. This crate ships NO production store; the
//! in-memory store below is a test double only.
//!
//! # Examples
//!
//! ```
//! use std::collections::HashSet;
//! use std::sync::Mutex;
//!
//! use sparq_query_protocol::{
//!     Challenge32, ChallengeOutcome, ChallengeStore, ChallengeStoreError,
//! };
//!
//! /// Test double: in memory, not durable, never for production.
//! struct TestStore(Mutex<HashSet<[u8; 32]>>);
//!
//! impl ChallengeStore for TestStore {
//!     fn consume(&self, challenge: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError> {
//!         let mut seen = self.0.lock().map_err(|_| ChallengeStoreError::new("poisoned"))?;
//!         Ok(if seen.insert(*challenge.as_bytes()) {
//!             ChallengeOutcome::Fresh
//!         } else {
//!             ChallengeOutcome::AlreadyConsumed
//!         })
//!     }
//! }
//!
//! let store = TestStore(Mutex::new(HashSet::new()));
//! let challenge = Challenge32::new([7; 32]).unwrap();
//! assert_eq!(store.consume(&challenge), Ok(ChallengeOutcome::Fresh));
//! assert_eq!(store.consume(&challenge), Ok(ChallengeOutcome::AlreadyConsumed));
//! ```

use core::fmt;

use crate::error::{ErrorCode, FailureClass, Phase, ProtocolError};
use crate::request::{Challenge32, StoredRequest};

/// Result of one atomic check-and-consume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChallengeOutcome {
    /// Not seen before; now durably recorded as consumed.
    Fresh,
    /// Consumed earlier; the presentation is a replay.
    AlreadyConsumed,
}

/// Challenge-store infrastructure failure with a stable store code.
///
/// A failure is never acceptance and never replay: the challenge state is
/// unknown, so the verification rejects as `infrastructure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChallengeStoreError {
    code: &'static str,
}

impl ChallengeStoreError {
    /// Builds a failure with stable store `code`.
    #[must_use]
    pub fn new(code: &'static str) -> Self {
        Self { code }
    }

    /// Returns the stable store code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ChallengeStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "challenge store failure: {}", self.code)
    }
}

impl std::error::Error for ChallengeStoreError {}

/// A verifier's shared, single-use challenge store.
///
/// `Send + Sync` so one store can serve concurrent verifications, typically
/// as `&dyn ChallengeStore` or `Arc<dyn ChallengeStore>`.
pub trait ChallengeStore: Send + Sync {
    /// Atomically checks and consumes the ORIGINAL request `challenge`.
    ///
    /// Implementations must record the challenge durably before returning
    /// [`ChallengeOutcome::Fresh`], and of concurrent calls with one challenge
    /// at most one may return `Fresh`. Never pass a method-derived nonce.
    ///
    /// # Errors
    /// Returns a [`ChallengeStoreError`] when the store cannot decide, for
    /// example on I/O failure. The challenge state is then unknown.
    fn consume(&self, challenge: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError>;
}

/// Consumes `request`'s original challenge once from the shared `store`.
///
/// The declared challenge owner calls this exactly once per verification, at
/// the point its [`ChallengeConsumption`](crate::ChallengeConsumption) names.
///
/// # Errors
/// At [`Phase::Verify`]: `invalid` with [`ErrorCode::ChallengeReplayed`] when
/// the challenge was already consumed; `infrastructure` with
/// [`ErrorCode::ChallengeStoreFailure`] when the store fails.
pub fn consume_challenge<S: ChallengeStore + ?Sized>(
    store: &S,
    request: &StoredRequest,
) -> Result<(), ProtocolError> {
    match store.consume(request.challenge()) {
        Ok(ChallengeOutcome::Fresh) => Ok(()),
        Ok(ChallengeOutcome::AlreadyConsumed) => Err(ProtocolError::new(
            FailureClass::Invalid,
            Phase::Verify,
            ErrorCode::ChallengeReplayed,
        )),
        Err(error) => Err(ProtocolError::new(
            FailureClass::Infrastructure,
            Phase::Verify,
            ErrorCode::ChallengeStoreFailure(error.code()),
        )),
    }
}

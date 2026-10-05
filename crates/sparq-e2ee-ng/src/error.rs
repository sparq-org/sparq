//! The single error type for the E2EE-NG primitives.

use core::fmt;

/// Every fallible primitive returns this.
///
/// The variants deliberately do **not** carry secret material or plaintext, so
/// an `Err(_)` is safe to log. Parsing/validation is **fail-closed**: any
/// deviation from the canonical/expected form is an error, never a best-effort
/// recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A CBOR item was not in RFC 8949 core-deterministic form (indefinite
    /// length, non-shortest integer, out-of-order or duplicate map key, …).
    NonCanonical(&'static str),
    /// The encoded item was structurally malformed or truncated.
    Malformed(&'static str),
    /// A declared length exceeded a configured parser limit (§8.1 deployment
    /// limits): message/block size, array length, map entry count, or nesting.
    LimitExceeded(&'static str),
    /// A required (positive-key) field was absent, or an unknown mandatory
    /// (positive-key) field was present. Unknown *negative* keys are extensions
    /// and are skipped, never an error.
    Schema(&'static str),
    /// The bound suite identifier is unknown or was substituted. Implementations
    /// MUST NOT silently substitute algorithms (§8.1).
    UnknownSuite,
    /// Two capability fields that must be mutually exclusive were both present
    /// (e.g. a publisher private key and an admin private key in one capability),
    /// or an authority/constraint separation rule was violated.
    Separation(&'static str),
    /// A delegation attempted to widen a constraint beyond its parent grant.
    Delegation(&'static str),
    /// An AEAD open failed: wrong key, tampered ciphertext, or wrong associated
    /// data (wrong repo/branch/epoch/object/block/chunk binding).
    Decrypt,
    /// A signature (author, publisher, or admin) did not verify.
    BadSignature,
    /// An authenticated structural link did not check out: a Merkle child's
    /// digest, level, or bound position disagreed with what its encrypted parent
    /// recorded. Distinguished from [`Error::Malformed`] because the bytes
    /// parsed fine — they simply are not the bytes that were linked.
    Integrity(&'static str),
    /// A cryptographic key or point encoding was invalid.
    BadKey(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NonCanonical(w) => write!(f, "non-canonical CBOR: {w}"),
            Error::Malformed(w) => write!(f, "malformed encoding: {w}"),
            Error::LimitExceeded(w) => write!(f, "parser limit exceeded: {w}"),
            Error::Schema(w) => write!(f, "schema violation: {w}"),
            Error::UnknownSuite => write!(f, "unknown or substituted crypto suite"),
            Error::Separation(w) => write!(f, "capability separation violation: {w}"),
            Error::Delegation(w) => write!(f, "invalid delegation: {w}"),
            Error::Decrypt => write!(f, "AEAD open failed (key / tag / associated-data mismatch)"),
            Error::BadSignature => write!(f, "signature did not verify"),
            Error::Integrity(w) => write!(f, "authenticated link mismatch: {w}"),
            Error::BadKey(w) => write!(f, "invalid key material: {w}"),
        }
    }
}

impl std::error::Error for Error {}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;

//! Validated identifiers, configured digests and query-profile names.
//!
//! These are exact names loaded from verifier configuration. Comparison is
//! byte-exact; there is no case folding, prefix matching or wildcard. This
//! module validates shape only and computes no digest. [OPUS-5.5]

use core::fmt;

use crate::error::{ErrorCode, FailureClass, Phase, ProtocolError};

/// Longest accepted identifier, in bytes.
///
/// Registry identifiers are short URNs; the bound keeps pasted documents or
/// keys from being accepted as names. Raising it changes no other invariant.
pub const MAX_IDENTIFIER_LEN: usize = 256;

/// An exact, case-sensitive `scheme:rest` identifier.
///
/// Accepted values are 1 to [`MAX_IDENTIFIER_LEN`] bytes of printable,
/// non-space ASCII with a scheme (a letter, then letters, digits, `+`, `-` or
/// `.`), a `:` and a non-empty remainder. `*` and `?` are rejected so that no
/// identifier can read as a wildcard.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// Validates `value` as an identifier.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with
    /// [`ErrorCode::MalformedIdentifier`] when `value` breaks the rules above.
    pub fn new(value: &str) -> Result<Self, ProtocolError> {
        let malformed = || {
            ProtocolError::new(
                FailureClass::Invalid,
                Phase::Request,
                ErrorCode::MalformedIdentifier,
            )
        };
        if value.is_empty() || value.len() > MAX_IDENTIFIER_LEN {
            return Err(malformed());
        }
        if !value.bytes().all(|b| b.is_ascii_graphic() && b != b'*' && b != b'?') {
            return Err(malformed());
        }
        let (scheme, rest) = value.split_once(':').ok_or_else(malformed)?;
        let mut scheme_bytes = scheme.bytes();
        let scheme_ok = scheme_bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
            && scheme_bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
        if !scheme_ok || rest.is_empty() {
            return Err(malformed());
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A 32-byte digest supplied by verifier configuration or a backend.
///
/// This type only carries bytes. It does not say which hash or encoding
/// produced them, and this crate never computes one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest32([u8; 32]);

impl Digest32 {
    /// Wraps configured digest `bytes`.
    ///
    /// # Errors
    /// Returns `invalid` at [`Phase::Request`] with [`ErrorCode::ZeroDigest`]
    /// for all-zero bytes, the usual placeholder for an unset pin.
    pub fn new(bytes: [u8; 32]) -> Result<Self, ProtocolError> {
        if bytes == [0; 32] {
            return Err(ProtocolError::new(
                FailureClass::Invalid,
                Phase::Request,
                ErrorCode::ZeroDigest,
            ));
        }
        Ok(Self(bytes))
    }

    /// Returns the digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Digest32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digest32(")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

/// Pinned dialect plus admitted query fragment (draft §5.1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryProfile {
    /// Pinned syntax, semantics and engine snapshot.
    pub dialect: Identifier,
    /// Admitted query fragment within the dialect.
    pub fragment: Identifier,
}

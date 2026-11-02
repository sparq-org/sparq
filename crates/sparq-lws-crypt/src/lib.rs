//! Native key providers and the versioned e1 blob envelope.
//!
//! See the crate README for the binary formats, key lifecycle and threat model.
//! This crate has no server or environment-variable wiring.

#![cfg(not(target_arch = "wasm32"))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod envelope;
mod key;
mod local;
mod wire;

pub use envelope::{is_envelope, open, parse_header, seal, Dek, Header, MAX_SEALS_PER_DEK};
pub use key::{KeyContext, KeyProvider, SecretKey, WrappedKey};
pub use local::LocalKeyProvider;

/// An error that carries no caller-supplied data or key material.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid framing, lengths, UTF-8, or key-file size.
    MalformedInput,
    /// The framing identifies an unsupported format version.
    UnsupportedVersion,
    /// The wrapped key names another provider.
    WrongProvider,
    /// The wrapped key names a different KEK; decryption was not attempted.
    WrongKek,
    /// Authentication failed, including a different DEK or context.
    AuthenticationFailed,
    /// Key-file I/O, random generation, or cryptographic setup failed.
    ProviderUnavailable,
    /// A Unix KEK file grants permissions to group or other users.
    InsecureKeyFilePermissions,
    /// The DEK has exhausted its seal-attempt budget.
    SealLimitReached,
    /// A length cannot be represented by this format.
    InputTooLong,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MalformedInput => "malformed encryption input",
            Self::UnsupportedVersion => "unsupported encryption format version",
            Self::WrongProvider => "wrong key provider",
            Self::WrongKek => "wrong KEK",
            Self::AuthenticationFailed => "encryption authentication failed",
            Self::ProviderUnavailable => "key provider unavailable",
            Self::InsecureKeyFilePermissions => "KEK file must grant no group or other permissions",
            Self::SealLimitReached => "DEK seal limit reached; rotate the data key",
            Self::InputTooLong => "input exceeds encryption format length limit",
        })
    }
}

impl std::error::Error for Error {}

/// Result type for key-provider and envelope operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;

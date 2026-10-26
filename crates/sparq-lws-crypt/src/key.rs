use std::fmt;

use async_trait::async_trait;
use zeroize::{Zeroize, Zeroizing};

use crate::{wire, Error, Result};

/// A non-cloneable 256-bit key, zeroized on drop with redacted diagnostics.
///
/// ```compile_fail
/// let secret = sparq_lws_crypt::SecretKey::from_bytes([0; 32]);
/// let copy = secret.clone();
/// ```
pub struct SecretKey(pub(crate) Zeroizing<[u8; 32]>);

impl SecretKey {
    /// Take ownership of key bytes. Callers must erase any copies they retain.
    pub fn from_bytes(mut bytes: [u8; 32]) -> Self {
        let key = Self(Zeroizing::new(bytes));
        bytes.zeroize();
        key
    }

    /// Borrow the key for a provider implementation; never log these bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey([REDACTED])")
    }
}

/// The exact pod identity and storage scheme that own a data key.
///
/// Values are UTF-8 bytes, without URI or Unicode normalization. Callers must
/// choose one stable identity per pod and never reuse its DEK for another pod.
pub struct KeyContext {
    /// Stable pod identity.
    pub pod: String,
    /// Stable storage scheme identifier.
    pub scheme: String,
}

impl KeyContext {
    /// Encode `SQLC || 01 || len(pod) || pod || len(scheme) || scheme`.
    /// Lengths are unsigned 32-bit big-endian UTF-8 byte counts.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = b"SQLC\x01".to_vec();
        wire::push_field(&mut bytes, self.pod.as_bytes())?;
        wire::push_field(&mut bytes, self.scheme.as_bytes())?;
        Ok(bytes)
    }
}

impl fmt::Debug for KeyContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyContext([REDACTED])")
    }
}

/// A provider's opaque wrapped data key, suitable for durable blob storage.
#[derive(Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// Stable provider identifier; `local` for the local provider.
    pub provider_id: String,
    /// Provider-specific KEK identifier (lowercase hex for the local provider).
    pub kek_id: String,
    /// Opaque provider payload; local v1 uses nonce followed by ciphertext and tag.
    pub bytes: Vec<u8>,
}

impl WrappedKey {
    /// Encode version 1: `SQLK || 01` followed by length-prefixed provider id,
    /// KEK id and payload, each using an unsigned 32-bit big-endian length.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = b"SQLK\x01".to_vec();
        wire::push_field(&mut bytes, self.provider_id.as_bytes())?;
        wire::push_field(&mut bytes, self.kek_id.as_bytes())?;
        wire::push_field(&mut bytes, &self.bytes)?;
        Ok(bytes)
    }

    /// Decode exactly one wrapped key; reject unknown versions and trailing bytes.
    pub fn from_bytes(mut bytes: &[u8]) -> Result<Self> {
        wire::prefix(&mut bytes, b"SQLK")?;
        let provider_id =
            std::str::from_utf8(wire::field(&mut bytes)?).map_err(|_| Error::MalformedInput)?;
        let kek_id =
            std::str::from_utf8(wire::field(&mut bytes)?).map_err(|_| Error::MalformedInput)?;
        let payload = wire::field(&mut bytes)?;
        if !bytes.is_empty() {
            return Err(Error::MalformedInput);
        }
        Ok(Self {
            provider_id: provider_id.to_owned(),
            kek_id: kek_id.to_owned(),
            bytes: payload.to_vec(),
        })
    }
}

impl fmt::Debug for WrappedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WrappedKey([REDACTED])")
    }
}

/// Provider boundary for per-pod data keys. Implementations must bind the exact
/// canonical context on both generate and unwrap and redact all errors.
#[async_trait]
pub trait KeyProvider: Send + Sync {
    /// Stable provider identifier.
    fn id(&self) -> &str;

    /// Generate a fresh random data key and wrap it for this pod and scheme.
    async fn generate_data_key(&self, context: &KeyContext) -> Result<(SecretKey, WrappedKey)>;

    /// Unwrap a data key only under the same pod and scheme.
    async fn unwrap(&self, wrapped: &WrappedKey, context: &KeyContext) -> Result<SecretKey>;
}

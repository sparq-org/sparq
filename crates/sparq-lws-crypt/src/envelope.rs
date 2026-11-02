use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use aws_lc_rs::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand,
};
use zeroize::Zeroizing;

use crate::{wire, Error, Result, SecretKey};

const MAGIC: &[u8; 4] = b"SQLB";
const HEADER_LEN: usize = 4 + 1 + 16 + 12;
const TAG_LEN: usize = 16;

/// Maximum seal attempts per DEK with random 96-bit nonces. Rotate well before it.
pub const MAX_SEALS_PER_DEK: u64 = 1 << 32;

/// One pod's data key, immutable id and shared seal-attempt counter.
///
/// No Clone implementation: share one instance across writers. Persist the
/// counter conservatively or rotate on restart; importing the same key twice
/// does not share its budget. Reads do not consume the budget.
pub struct Dek {
    key: SecretKey,
    id: [u8; 16],
    seals: AtomicU64,
}

impl Dek {
    /// Restore a pod's key and unique id with its conservative lifetime count.
    /// New keys start at zero. Reject counts above the hard limit.
    pub fn new(key: SecretKey, id: [u8; 16], seal_count: u64) -> Result<Self> {
        if seal_count > MAX_SEALS_PER_DEK {
            return Err(Error::SealLimitReached);
        }
        Ok(Self {
            key,
            id,
            seals: AtomicU64::new(seal_count),
        })
    }

    /// Public, immutable key id. The caller must allocate a unique id per pod DEK.
    pub fn id(&self) -> &[u8; 16] {
        &self.id
    }

    /// Seal attempts reserved so far, including attempts that failed after reservation.
    pub fn seal_count(&self) -> u64 {
        self.seals.load(Ordering::Relaxed)
    }

    fn reserve_seal(&self) -> Result<()> {
        self.seals
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < MAX_SEALS_PER_DEK).then_some(n + 1)
            })
            .map(|_| ())
            .map_err(|_| Error::SealLimitReached)
    }
}

impl fmt::Debug for Dek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dek([REDACTED])")
    }
}

/// Unauthenticated e1 routing information. Trust it only after successful `open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Data key identifier.
    pub dek_id: [u8; 16],
    /// AES-GCM nonce.
    pub nonce: [u8; 12],
}

/// Recognize the reserved envelope magic, including truncated/unknown versions.
/// Callers must send matches to `open` and propagate errors, never retry as plaintext.
/// Legacy b1 blobs with other prefixes remain plaintext candidates.
pub fn is_envelope(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Read a complete v1 header without authenticating it or requiring a payload.
pub fn parse_header(mut bytes: &[u8]) -> Option<Header> {
    wire::prefix(&mut bytes, MAGIC).ok()?;
    Some(Header {
        dek_id: wire::take(&mut bytes, 16).ok()?.try_into().ok()?,
        nonce: wire::take(&mut bytes, 12).ok()?.try_into().ok()?,
    })
}

/// Seal a blob with a fresh random nonce, reserving one lifetime seal attempt.
/// RNG/crypto failures and an exhausted budget return errors. The context is
/// bound indirectly through the unique DEK id; see the crate README.
pub fn seal(dek: &Dek, blob_key: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let aad = blob_aad(&dek.id, blob_key)?;
    dek.reserve_seal()?;
    seal_impl(dek, &aad, plaintext, random_nonce()?)
}

fn blob_aad(id: &[u8; 16], blob_key: &str) -> Result<Vec<u8>> {
    let mut aad = b"e1".to_vec();
    aad.extend_from_slice(id);
    wire::push_field(&mut aad, blob_key.as_bytes())?;
    Ok(aad)
}

fn seal_impl(dek: &Dek, aad: &[u8], plaintext: &[u8], nonce: [u8; 12]) -> Result<Vec<u8>> {
    let mut envelope = MAGIC.to_vec();
    envelope.push(1);
    envelope.extend_from_slice(&dek.id);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&encrypt(&dek.key, nonce, aad, plaintext)?);
    Ok(envelope)
}

#[cfg(test)]
pub(crate) fn seal_with_nonce(
    dek: &Dek,
    blob_key: &str,
    plaintext: &[u8],
    nonce: [u8; 12],
) -> Result<Vec<u8>> {
    let aad = blob_aad(&dek.id, blob_key)?;
    dek.reserve_seal()?;
    seal_impl(dek, &aad, plaintext, nonce)
}

/// Authenticate and decrypt exactly one e1 envelope for this blob key and DEK.
pub fn open(dek: &Dek, blob_key: &str, mut envelope: &[u8]) -> Result<Vec<u8>> {
    let bytes = envelope;
    wire::prefix(&mut envelope, MAGIC)?;
    let header = parse_header(bytes).ok_or(Error::MalformedInput)?;
    if bytes.len() < HEADER_LEN + TAG_LEN {
        return Err(Error::MalformedInput);
    }
    if header.dek_id != dek.id {
        return Err(Error::AuthenticationFailed);
    }
    let plaintext = decrypt(
        &dek.key,
        header.nonce,
        &blob_aad(&dek.id, blob_key)?,
        &bytes[HEADER_LEN..],
    )?;
    Ok(plaintext.to_vec())
}

pub(crate) fn random_nonce() -> Result<[u8; 12]> {
    let mut nonce = [0; 12];
    rand::fill(&mut nonce).map_err(|_| Error::ProviderUnavailable)?;
    Ok(nonce)
}

fn aes_key(key: &SecretKey) -> Result<LessSafeKey> {
    UnboundKey::new(&aead::AES_256_GCM, key.as_bytes())
        .map(LessSafeKey::new)
        .map_err(|_| Error::ProviderUnavailable)
}

pub(crate) fn encrypt(
    key: &SecretKey,
    nonce: [u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let mut bytes = Zeroizing::new(plaintext.to_vec());
    aes_key(key)?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut *bytes,
        )
        .map_err(|_| Error::InputTooLong)?;
    Ok(bytes.to_vec())
}

pub(crate) fn decrypt(
    key: &SecretKey,
    nonce: [u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(ciphertext.to_vec());
    let len = aes_key(key)?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut bytes,
        )
        .map_err(|_| Error::AuthenticationFailed)?
        .len();
    // Erase the tag before truncating so all initialized bytes are wiped.
    zeroize::Zeroize::zeroize(&mut bytes[len..]);
    bytes.truncate(len);
    Ok(bytes)
}

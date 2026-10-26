use std::{fmt, fs::File, io::Read, path::Path};

use async_trait::async_trait;
use aws_lc_rs::{digest, rand};
use zeroize::Zeroizing;

use crate::{envelope, Error, KeyContext, KeyProvider, Result, SecretKey, WrappedKey};

const KEK_ID_DOMAIN: &[u8] = b"sparq-lws-crypt/local-kek-id/v1\0";
const LOCAL_PAYLOAD_LEN: usize = 12 + 32 + 16;

/// An AES-256-GCM key provider backed by one operator-managed local KEK file.
pub struct LocalKeyProvider {
    kek: SecretKey,
    kek_id: String,
}

impl LocalKeyProvider {
    /// Load exactly 32 raw bytes from a regular file. On Unix, the opened file
    /// must grant no group/other permissions (for example mode 0600 or 0400).
    /// Errors never include the path or file contents.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path).map_err(|_| Error::ProviderUnavailable)?;
        let metadata = file.metadata().map_err(|_| Error::ProviderUnavailable)?;
        if !metadata.is_file() {
            return Err(Error::MalformedInput);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::InsecureKeyFilePermissions);
            }
        }
        // Bounded read also detects a file that grows after the metadata check.
        let mut bytes = Zeroizing::new(Vec::with_capacity(33));
        file.take(33)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::ProviderUnavailable)?;
        if bytes.len() != 32 {
            return Err(Error::MalformedInput);
        }
        let mut kek = SecretKey::from_bytes([0; 32]);
        kek.0.copy_from_slice(&bytes);
        let mut hash = digest::Context::new(&digest::SHA256);
        hash.update(KEK_ID_DOMAIN);
        hash.update(kek.as_bytes());
        let kek_id = hash.finish().as_ref()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self { kek, kek_id })
    }

    fn wrap_impl(
        &self,
        key: &SecretKey,
        context: &KeyContext,
        nonce: [u8; 12],
    ) -> Result<WrappedKey> {
        let aad = context.to_bytes()?;
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&envelope::encrypt(&self.kek, nonce, &aad, key.as_bytes())?);
        Ok(WrappedKey {
            provider_id: self.id().to_owned(),
            kek_id: self.kek_id.clone(),
            bytes,
        })
    }

    #[cfg(test)]
    pub(crate) fn wrap_with_nonce(
        &self,
        key: &SecretKey,
        context: &KeyContext,
        nonce: [u8; 12],
    ) -> Result<WrappedKey> {
        self.wrap_impl(key, context, nonce)
    }
}

#[async_trait]
impl KeyProvider for LocalKeyProvider {
    fn id(&self) -> &str {
        "local"
    }

    async fn generate_data_key(&self, context: &KeyContext) -> Result<(SecretKey, WrappedKey)> {
        let mut key = SecretKey::from_bytes([0; 32]);
        rand::fill(key.0.as_mut()).map_err(|_| Error::ProviderUnavailable)?;
        let wrapped = self.wrap_impl(&key, context, envelope::random_nonce()?)?;
        Ok((key, wrapped))
    }

    async fn unwrap(&self, wrapped: &WrappedKey, context: &KeyContext) -> Result<SecretKey> {
        if wrapped.provider_id != self.id() {
            return Err(Error::WrongProvider);
        }
        if wrapped.kek_id != self.kek_id {
            return Err(Error::WrongKek);
        }
        if wrapped.bytes.len() != LOCAL_PAYLOAD_LEN {
            return Err(Error::MalformedInput);
        }
        let nonce = wrapped.bytes[..12]
            .try_into()
            .map_err(|_| Error::MalformedInput)?;
        let plaintext =
            envelope::decrypt(&self.kek, nonce, &context.to_bytes()?, &wrapped.bytes[12..])?;
        if plaintext.len() != 32 {
            return Err(Error::MalformedInput);
        }
        let mut key = SecretKey::from_bytes([0; 32]);
        key.0.copy_from_slice(&plaintext);
        Ok(key)
    }
}

impl fmt::Debug for LocalKeyProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalKeyProvider([REDACTED])")
    }
}

//! Research-grade Ed25519/SHA-256 suite with private per-leaf salts.
use crate::{canonical_config_with, comparison_key, sha256, Error, SignedCredential, SALT_BYTES};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, Mac};
use oxrdf::{Quad, Term};
use sha2::{Digest, Sha256};

/// Data Integrity suite identifier and signed-message domain.
pub const EDDSA_SHA256_MERKLE_SALTED_2026: &str = "eddsa-sha256-merkle-salted-2026";

/// Unsalted digest of a canonical RDFC-1.0 quad for private ordering.
/// Returns `None` for triple terms and directional language-tagged literals.
/// Literal frames bind both the RDF term bytes and the extended key.
#[must_use]
pub fn quad_hash(quad: &Quad) -> Option<[u8; 32]> {
    if matches!(&quad.object, Term::Literal(literal) if literal.direction().is_some()) {
        return None;
    }
    let hash = Sha256::new_with_prefix(EDDSA_SHA256_MERKLE_SALTED_2026).chain_update(b"/quad\0");
    sha256::quad_digest(quad, hash, comparison_key)
}

/// Derive a private leaf salt from the issuance salt and zero-based index in
/// strictly increasing quad-hash order.
#[must_use]
pub fn leaf_salt(salt: &[u8; SALT_BYTES], index: u32) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(EDDSA_SHA256_MERKLE_SALTED_2026.as_bytes());
    mac.update(b"/leaf-salt\0");
    mac.update(&index.to_be_bytes());
    mac.finalize().into_bytes().into()
}

/// Hash a quad digest with its private salt at the given canonical index.
#[must_use]
pub fn leaf(quad_hash: &[u8; 32], salt: &[u8; SALT_BYTES], index: u32) -> [u8; 32] {
    Sha256::new_with_prefix(EDDSA_SHA256_MERKLE_SALTED_2026)
        .chain_update(b"/leaf\0")
        .chain_update(quad_hash)
        .chain_update(leaf_salt(salt, index))
        .finalize()
        .into()
}

/// Build the minimal binary tree with zero padding. Returns `None` unless
/// the quad hashes are nonempty, strictly increasing, and fit a u32 count.
/// The salted leaves retain quad-hash order.
#[must_use]
pub fn root(quads: &[[u8; 32]], salt: &[u8; SALT_BYTES]) -> Option<[u8; 32]> {
    if quads.is_empty()
        || u32::try_from(quads.len()).is_err()
        || quads.windows(2).any(|p| p[0] >= p[1])
    {
        return None;
    }
    let row = quads
        .iter()
        .enumerate()
        .map(|(i, quad)| leaf(quad, salt, i as u32))
        .collect();
    sha256::merkle_root(row)
}

/// The Ed25519 message, binding the suite, issuance salt, exact quad count,
/// salted root, and SHA-256 of the RDFC-1.0 canonical proof configuration.
#[must_use]
pub fn signed_message(
    salt: &[u8; SALT_BYTES],
    count: u32,
    root: &[u8; 32],
    config_digest: &[u8; 32],
) -> [u8; 32] {
    sha256::message_for_suite(
        EDDSA_SHA256_MERKLE_SALTED_2026,
        salt,
        count,
        root,
        config_digest,
    )
}

fn canonical_document(document: &str) -> Result<String, Error> {
    sparq_canon::canonicalize_nquads(document)
        .map_err(|_| Error("document canonicalization rejected"))
}

fn canonical_config(proof_config: &str) -> Result<String, Error> {
    canonical_config_with(proof_config, |literal| {
        literal.value() == EDDSA_SHA256_MERKLE_SALTED_2026
            && matches!(
                literal.datatype().as_str(),
                "http://www.w3.org/2001/XMLSchema#string"
                    | "https://w3id.org/security#cryptosuiteString"
            )
    })
}

fn quad_hashes(document: &str) -> impl Iterator<Item = Result<[u8; 32], Error>> + '_ {
    oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .map(|quad| {
            let quad = quad.map_err(|_| Error("quad parsing rejected"))?;
            quad_hash(&quad).ok_or(Error("quad object is outside the RDFC-1.0 profile"))
        })
}

fn document_message(
    hashes: &[[u8; 32]],
    canonical_config: &str,
    salt: &[u8; SALT_BYTES],
) -> Result<[u8; 32], Error> {
    let count =
        u32::try_from(hashes.len()).map_err(|_| Error("quad count exceeds member range"))?;
    let root = root(hashes, salt).ok_or(Error(
        "quad hashes must be nonempty and strictly increasing",
    ))?;
    let digest = Sha256::digest(canonical_config.as_bytes()).into();
    Ok(signed_message(salt, count, &root, &digest))
}

/// Issue an ordered credential with `signature || salt` as its proof value bytes.
/// The issuer supplies a fresh uniformly random 32-byte salt and shares it
/// only with the holder. The proof configuration must name this suite with an
/// `xsd:string` or `sec:cryptosuiteString` literal.
///
/// # Errors
/// Refuses malformed or unsupported documents, empty trees, and invalid proof
/// configurations.
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &ed25519_dalek::SigningKey,
    salt: [u8; SALT_BYTES],
) -> Result<SignedCredential, Error> {
    use ed25519_dalek::Signer;

    let document = canonical_document(document)?;
    let proof_config = canonical_config(proof_config)?;
    let mut rows = quad_hashes(&document)
        .zip(document.split_inclusive('\n'))
        .map(|(hash, line)| Ok((hash?, line)))
        .collect::<Result<Vec<_>, Error>>()?;
    rows.sort_unstable_by_key(|(hash, _)| *hash);
    let hashes: Vec<_> = rows.iter().map(|(hash, _)| *hash).collect();
    let message = document_message(&hashes, &proof_config, &salt)?;
    let mut signature = key.sign(&message).to_bytes().to_vec();
    signature.extend_from_slice(&salt);
    Ok(SignedCredential {
        document: rows.into_iter().map(|(_, line)| line).collect(),
        proof_config,
        signature,
    })
}

/// Verify a whole credential under this suite using its private issuance salt.
#[must_use]
pub fn verify(credential: &SignedCredential, key: &VerifyingKey) -> bool {
    let check = || -> Result<(), Error> {
        let configuration = canonical_config(&credential.proof_config)?;
        if configuration != credential.proof_config {
            return Err(Error("configuration is not canonical"));
        }
        let canonical = canonical_document(&credential.document)?;
        let mut lines: Vec<_> = credential.document.split_inclusive('\n').collect();
        lines.sort_unstable();
        if lines.concat() != canonical {
            return Err(Error("document is not canonical"));
        }
        let (signature, salt) = credential
            .signature
            .split_at_checked(64)
            .ok_or(Error("signature and salt length"))?;
        let signature =
            Signature::from_slice(signature).map_err(|_| Error("signature encoding"))?;
        let salt = salt.try_into().map_err(|_| Error("salt length"))?;
        let hashes = quad_hashes(&credential.document).collect::<Result<Vec<_>, _>>()?;
        let message = document_message(&hashes, &configuration, salt)?;
        key.verify_strict(&message, &signature)
            .map_err(|_| Error("signature rejected"))
    };
    check().is_ok()
}

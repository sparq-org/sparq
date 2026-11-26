//! Research-grade Ed25519/SHA-256 suite with private per-leaf salts.
use crate::{
    check_cryptosuite, config_quads, keys::salted_sha256_comparison_key, sha256, Error,
    SignedCredential, SALT_BYTES,
};
use ed25519_dalek::{Signature, VerifyingKey, SIGNATURE_LENGTH};
use hmac::{Hmac, Mac};
use oxrdf::{GraphName, NamedOrBlankNode, Quad, Term};
use sha2::{Digest, Sha256};
use sparq_canon::{canonicalize_nquads_bounded, CanonicalizationLimits};

/// Data Integrity suite identifier and signed-message domain.
pub const EDDSA_SHA256_MERKLE_SALTED_2026: &str = "eddsa-sha256-merkle-salted-2026";

const CANONICALIZATION_LIMITS: CanonicalizationLimits = CanonicalizationLimits {
    max_quads: 16_384,
    max_input_bytes: 4 * 1024 * 1024,
    // Leave space for the canonicalizer's conservative blank node relabeling allowance.
    max_output_bytes: 5 * 1024 * 1024,
    max_hndq_calls: 4_000,
    max_permutation_steps: 20_000,
};

/// Unsalted digest of a canonical RDFC-1.0 quad for private ordering.
/// Returns `None` for triple terms, directional language-tagged literals,
/// and components whose byte length exceeds `u32::MAX`.
/// Literal frames bind both the RDF term bytes and the extended key.
#[must_use]
pub fn quad_hash(quad: &Quad) -> Option<[u8; 32]> {
    if !quad_components_fit(quad) {
        return None;
    }
    let hash = Sha256::new_with_prefix(EDDSA_SHA256_MERKLE_SALTED_2026).chain_update(b"/quad\0");
    sha256::quad_digest(quad, hash, salted_sha256_comparison_key)
}

fn quad_components_fit(quad: &Quad) -> bool {
    let subject = match &quad.subject {
        NamedOrBlankNode::NamedNode(node) => node.as_str(),
        NamedOrBlankNode::BlankNode(node) => node.as_str(),
    };
    let graph = match &quad.graph_name {
        GraphName::DefaultGraph => "",
        GraphName::NamedNode(node) => node.as_str(),
        GraphName::BlankNode(node) => node.as_str(),
    };
    [subject, quad.predicate.as_str(), graph]
        .iter()
        .all(|text| component_length_fits(text.len()))
        && match &quad.object {
            Term::NamedNode(node) => component_length_fits(node.as_str().len()),
            Term::BlankNode(node) => component_length_fits(node.as_str().len()),
            Term::Literal(literal) => {
                literal.direction().is_none()
                    && [
                        literal.value(),
                        literal.datatype().as_str(),
                        literal.language().unwrap_or(""),
                    ]
                    .iter()
                    .all(|text| component_length_fits(text.len()))
            }
            Term::Triple(_) => false,
        }
}

fn component_length_fits(len: usize) -> bool {
    u32::try_from(len).is_ok()
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

fn canonicalize(
    text: &str,
    limits: &CanonicalizationLimits,
    rejected: Error,
) -> Result<String, Error> {
    let canonical = canonicalize_nquads_bounded(text, limits).map_err(|_| rejected)?;
    if canonical.len() > limits.max_input_bytes {
        return Err(rejected);
    }
    Ok(canonical)
}

fn canonical_document(document: &str, limits: &CanonicalizationLimits) -> Result<String, Error> {
    canonicalize(
        document,
        limits,
        Error("document canonicalization rejected"),
    )
}

fn canonical_config(proof_config: &str, limits: &CanonicalizationLimits) -> Result<String, Error> {
    let text = canonicalize(
        proof_config,
        limits,
        Error("configuration canonicalization rejected"),
    )?;
    let quads = config_quads(&text)?;
    if quads
        .iter()
        .any(|quad| quad.predicate.as_str() == "https://w3id.org/security#proofValue")
    {
        return Err(Error("configuration contains proofValue"));
    }
    check_cryptosuite(&quads, |literal| {
        literal.value() == EDDSA_SHA256_MERKLE_SALTED_2026
            && matches!(
                literal.datatype().as_str(),
                "http://www.w3.org/2001/XMLSchema#string"
                    | "https://w3id.org/security#cryptosuiteString"
            )
    })?;
    Ok(text)
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
/// `xsd:string` or `sec:cryptosuiteString` literal and omit `sec:proofValue`.
///
/// # Errors
/// Refuses malformed or unsupported documents, empty trees, and invalid proof
/// configurations. Each input must fit the canonicalization size and work limits.
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &ed25519_dalek::SigningKey,
    salt: [u8; SALT_BYTES],
) -> Result<SignedCredential, Error> {
    use ed25519_dalek::Signer;

    let document = canonical_document(document, &CANONICALIZATION_LIMITS)?;
    let proof_config = canonical_config(proof_config, &CANONICALIZATION_LIMITS)?;
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
/// Checks the proof value length and key encoding before bounded canonicalization.
#[must_use]
pub fn verify(credential: &SignedCredential, key: &VerifyingKey) -> bool {
    check_credential(credential, key, &CANONICALIZATION_LIMITS).is_ok()
}

fn check_credential(
    credential: &SignedCredential,
    key: &VerifyingKey,
    limits: &CanonicalizationLimits,
) -> Result<(), Error> {
    let (signature, salt) = credential
        .signature
        .split_at_checked(SIGNATURE_LENGTH)
        .ok_or(Error("signature and salt length"))?;
    let salt = salt
        .try_into()
        .map_err(|_| Error("signature and salt length"))?;
    let signature = Signature::from_slice(signature).map_err(|_| Error("signature encoding"))?;
    check_key_encoding(key)?;

    let configuration = canonical_config(&credential.proof_config, limits)?;
    if configuration != credential.proof_config {
        return Err(Error("configuration is not canonical"));
    }
    let canonical = canonical_document(&credential.document, limits)?;
    let mut lines = credential.document.split_inclusive('\n');
    let mut sorted: Vec<_> = lines.by_ref().take(limits.max_quads).collect();
    if lines.next().is_some() {
        return Err(Error("document line capacity"));
    }
    sorted.sort_unstable();
    if !sorted.into_iter().eq(canonical.split_inclusive('\n')) {
        return Err(Error("document is not canonical"));
    }
    let hashes = quad_hashes(&credential.document).collect::<Result<Vec<_>, _>>()?;
    let message = document_message(&hashes, &configuration, salt)?;
    key.verify_strict(&message, &signature)
        .map_err(|_| Error("signature rejected"))
}

fn check_key_encoding(key: &VerifyingKey) -> Result<(), Error> {
    if key.to_edwards().compress().as_bytes() != key.as_bytes() {
        return Err(Error("public key encoding"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    const DOCUMENT: &str = "<urn:s> <urn:p> <urn:o> .\n";
    const CONFIG: &str =
        "<urn:p> <https://w3id.org/security#cryptosuite> \"eddsa-sha256-merkle-salted-2026\" .\n";
    const CYCLE: &str = "_:c14n0 <urn:p> _:c14n1 .\n_:c14n1 <urn:p> _:c14n0 .\n";

    #[test]
    fn proof_value_length_precedes_canonicalization() {
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key();
        let mut credential = SignedCredential {
            document: "invalid document".into(),
            proof_config: "invalid configuration".into(),
            signature: Vec::new(),
        };
        for len in [0, 63, 64, 95, 96, 97] {
            credential.signature.resize(len, 0);
            let error = if len == 96 {
                Error("configuration canonicalization rejected")
            } else {
                Error("signature and salt length")
            };
            assert_eq!(
                check_credential(&credential, &key, &CANONICALIZATION_LIMITS),
                Err(error)
            );
        }
    }

    #[test]
    fn verifier_bounds_document_canonicalization_work() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let credential = issue(CYCLE, CONFIG, &key, [9; 32]).unwrap();
        let pk = key.verifying_key();
        assert!(verify(&credential, &pk));
        let limits = CanonicalizationLimits {
            max_permutation_steps: 7,
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(
            check_credential(&credential, &pk, &limits),
            Err(Error("document canonicalization rejected"))
        );
        let limits = CanonicalizationLimits {
            max_hndq_calls: 1,
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(
            check_credential(&credential, &pk, &limits),
            Err(Error("document canonicalization rejected"))
        );
    }

    #[test]
    fn verifier_bounds_configuration_canonicalization_work() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let credential = issue(DOCUMENT, &format!("{CONFIG}{CYCLE}"), &key, [9; 32]).unwrap();
        let pk = key.verifying_key();
        assert!(verify(&credential, &pk));
        let limits = CanonicalizationLimits {
            max_permutation_steps: 7,
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(
            check_credential(&credential, &pk, &limits),
            Err(Error("configuration canonicalization rejected"))
        );
        let limits = CanonicalizationLimits {
            max_hndq_calls: 1,
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(
            check_credential(&credential, &pk, &limits),
            Err(Error("configuration canonicalization rejected"))
        );
    }

    #[test]
    fn canonicalization_bounds_raw_and_canonical_nquads_bytes() {
        let input = format!("\n{DOCUMENT}");
        let limits = CanonicalizationLimits {
            max_input_bytes: DOCUMENT.len(),
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(
            canonical_document(DOCUMENT, &limits).as_deref(),
            Ok(DOCUMENT)
        );
        assert_eq!(
            canonical_document(&input, &limits),
            Err(Error("document canonicalization rejected"))
        );

        let input = "_:a <urn:p> <urn:o> .\n";
        let canonical = "_:c14n0 <urn:p> <urn:o> .\n";
        let limits = CanonicalizationLimits {
            max_input_bytes: canonical.len(),
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(canonical_document(input, &limits).as_deref(), Ok(canonical));
        let limits = CanonicalizationLimits {
            max_input_bytes: input.len(),
            ..limits
        };
        assert_eq!(
            canonical_document(input, &limits),
            Err(Error("document canonicalization rejected"))
        );
    }

    #[test]
    fn verifier_bounds_physical_document_lines_before_sorting() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let key = signer.verifying_key();
        let mut credential = issue(DOCUMENT, CONFIG, &signer, [9; 32]).unwrap();
        let limits = CanonicalizationLimits {
            max_quads: 1,
            ..CANONICALIZATION_LIMITS
        };
        assert_eq!(check_credential(&credential, &key, &limits), Ok(()));
        credential.document.insert(0, '\n');
        assert_eq!(
            check_credential(&credential, &key, &limits),
            Err(Error("document line capacity"))
        );
        let limits = CanonicalizationLimits {
            max_quads: 2,
            ..limits
        };
        assert_eq!(
            check_credential(&credential, &key, &limits),
            Err(Error("document is not canonical"))
        );
    }

    #[test]
    fn canonical_key_admission_checks_the_supplied_bytes() {
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key();
        assert_eq!(check_key_encoding(&key), Ok(()));

        let mut noncanonical = [0; 32];
        noncanonical[0] = 1;
        noncanonical[31] = 0x80;
        let key = VerifyingKey::from_bytes(&noncanonical).unwrap();
        assert_eq!(key.to_bytes(), noncanonical);
        assert_eq!(check_key_encoding(&key), Err(Error("public key encoding")));
        let credential = SignedCredential {
            document: DOCUMENT.into(),
            proof_config: CONFIG.into(),
            signature: vec![0; SIGNATURE_LENGTH + SALT_BYTES],
        };
        assert_eq!(
            check_credential(&credential, &key, &CANONICALIZATION_LIMITS),
            Err(Error("public key encoding"))
        );
    }

    #[test]
    fn quad_component_length_boundary() {
        assert!(component_length_fits(0));
        assert!(component_length_fits(u32::MAX as usize));
        if let Ok(oversized) = usize::try_from(u64::from(u32::MAX) + 1) {
            assert!(!component_length_fits(oversized));
            assert!(!component_length_fits(usize::MAX));
        }
    }
}

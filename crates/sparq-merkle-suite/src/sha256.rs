//! SHA-256 member. Existing vectors retain their byte encodings.
use crate::{term_kind, SALT_BYTES};
use oxrdf::{GraphName, Quad, Term};
use sha2::{Digest, Sha256};
/// Data Integrity suite identifier.
pub const EDDSA_SHA256_MERKLE_2026: &str = "eddsa-sha256-merkle-2026";
/// Frozen SHA-member dispatch. The field-native member owns the extended date/subtype profile.
pub fn comparison_key(literal: &oxrdf::Literal) -> [u8; crate::KEY_BYTES] {
    let datatype = literal.datatype().as_str();
    let local = datatype.strip_prefix("http://www.w3.org/2001/XMLSchema#");
    if matches!(local, Some("date")) || local.is_some_and(|name| name != "integer" && crate::keys::integer_type(name)) {
        [0; crate::KEY_BYTES]
    } else { crate::comparison_key(literal) }
}
/// Appends the typed byte encoding of one term (or graph name) to `out`.
pub fn encode_term(term: &Term, out: &mut Vec<u8>) {
    match term {
        Term::NamedNode(node) => tagged(out, term_kind::IRI, node.as_str()),
        Term::BlankNode(node) => tagged(out, term_kind::BLANK_NODE, node.as_str()),
        Term::Literal(literal) => {
            tagged(out, term_kind::LITERAL, literal.value());
            framed(out, literal.datatype().as_str());
            framed(out, literal.language().unwrap_or(""));
            out.extend_from_slice(&comparison_key(literal));
        }
        Term::Triple(_) => unreachable!("triple terms are rejected before encoding"),
    }
}

/// `SHA-256(0x00 || enc(s) || enc(p) || enc(o) || enc(g))` for one quad.
///
/// Returns `None` for a quad with an RDF 1.2 triple term, which the suite excludes.
#[must_use]
pub fn leaf(quad: &Quad) -> Option<[u8; 32]> {
    if matches!(quad.object, Term::Triple(_)) {
        return None;
    }
    let mut bytes = vec![0x00];
    encode_term(&Term::from(quad.subject.clone()), &mut bytes);
    tagged(&mut bytes, term_kind::IRI, quad.predicate.as_str());
    encode_term(&quad.object, &mut bytes);
    match &quad.graph_name {
        GraphName::DefaultGraph => bytes.push(term_kind::DEFAULT_GRAPH),
        GraphName::NamedNode(node) => tagged(&mut bytes, term_kind::IRI, node.as_str()),
        GraphName::BlankNode(node) => tagged(&mut bytes, term_kind::BLANK_NODE, node.as_str()),
    }
    Some(Sha256::digest(&bytes).into())
}

/// The Merkle root over leaves in strictly increasing byte order.
///
/// The leaf row is padded with all-zero leaves to the next power of two; an inner
/// node is `SHA-256(0x01 || left || right)`. One leaf is its own root. Returns
/// `None` for no leaves or leaves that are not strictly increasing.
#[must_use]
pub fn root(leaves: &[[u8; 32]]) -> Option<[u8; 32]> {
    if leaves.is_empty() || leaves.windows(2).any(|pair| pair[0] >= pair[1]) {
        return None;
    }
    let mut row = leaves.to_vec();
    row.resize(leaves.len().next_power_of_two(), [0; 32]);
    while row.len() > 1 {
        row = row
            .chunks_exact(2)
            .map(|pair| {
                let mut hash = Sha256::new();
                hash.update([0x01]);
                hash.update(pair[0]);
                hash.update(pair[1]);
                hash.finalize().into()
            })
            .collect();
    }
    Some(row[0])
}

/// The 32-byte message the issuer signs with Ed25519.
///
/// `SHA-256("eddsa-sha256-merkle-2026" || 0x00 || salt || u32be(n) || root || config_digest)`,
/// where `n` is the number of quads and `config_digest` is SHA-256 of the
/// RDFC-1.0 canonical proof configuration.
#[must_use]
pub fn signed_message(
    salt: &[u8; SALT_BYTES],
    quads: u32,
    root: &[u8; 32],
    config_digest: &[u8; 32],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(EDDSA_SHA256_MERKLE_2026.as_bytes());
    hash.update([0x00]);
    hash.update(salt);
    hash.update(quads.to_be_bytes());
    hash.update(root);
    hash.update(config_digest);
    hash.finalize().into()
}

fn framed(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn tagged(out: &mut Vec<u8>, tag: u8, text: &str) {
    out.push(tag);
    framed(out, text);
}

/// Issues a credential under this suite, outside any proof.
///
/// Canonicalizes `document` and `proof_config` with RDFC-1.0, orders the
/// document quads by leaf, signs the salted root message with `key`, and returns
/// the witness form: the leaf-ordered document, the canonical configuration, and
/// `signature || salt` as the `proofValue` bytes. `proof_config` must name this
/// suite; issuance and verification both check that configuration.
///
/// # Errors
/// Rejects input that does not parse or canonicalize, or that has a triple term.
#[cfg(not(target_os = "zkvm"))]
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &ed25519_dalek::SigningKey,
    salt: [u8; SALT_BYTES],
) -> Result<crate::SignedCredential, crate::Error> {
    use ed25519_dalek::Signer;
    let canonical = |text: &str| {
        sparq_canon::canonicalize_nquads(text)
            .map_err(|_| crate::Error("issuer canonicalization rejected"))
    };
    let document = canonical(document)?;
    let proof_config = crate::canonical_config(proof_config, EDDSA_SHA256_MERKLE_2026)?;
    let mut leaves = Vec::new();
    for (quad, line) in oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .zip(document.lines())
    {
        let quad = quad.map_err(|_| crate::Error("issuer N-Quads parse rejected"))?;
        let leaf = leaf(&quad).ok_or(crate::Error("RDF 1.2 triple terms are not admitted"))?;
        leaves.push((leaf, line));
    }
    leaves.sort_unstable();
    let row: Vec<[u8; 32]> = leaves.iter().map(|(leaf, _)| *leaf).collect();
    let root = root(&row).ok_or(crate::Error(
        "Merkle leaves must be non-empty and strictly increasing",
    ))?;
    let config_digest: [u8; 32] = Sha256::digest(proof_config.as_bytes()).into();
    let message = signed_message(&salt, row.len() as u32, &root, &config_digest);
    let mut proof_value = key.sign(&message).to_bytes().to_vec();
    proof_value.extend_from_slice(&salt);
    Ok(crate::SignedCredential {
        document: leaves.iter().map(|(_, line)| format!("{line}\n")).collect(),
        proof_config,
        signature: proof_value,
    })
}

/// Verify an ordered SHA-256 credential, including its private salt.
pub fn verify(credential: &crate::SignedCredential, key: &ed25519_dalek::VerifyingKey) -> bool {
    let check = || -> Result<(), crate::Error> {
        let configuration =
            crate::canonical_config(&credential.proof_config, EDDSA_SHA256_MERKLE_2026)?;
        if configuration != credential.proof_config {
            return Err(crate::Error("configuration is not canonical"));
        }
        let canonical = sparq_canon::canonicalize_nquads(&credential.document)
            .map_err(|_| crate::Error("document canonicalization rejected"))?;
        let mut ordered = credential.document.lines().collect::<Vec<_>>();
        ordered.sort_unstable();
        if ordered.join("\n") + "\n" != canonical {
            return Err(crate::Error("document is not canonical"));
        }
        if credential.signature.len() != 64 + SALT_BYTES {
            return Err(crate::Error("signature and salt length"));
        }
        let leaves = oxttl::NQuadsParser::new()
            .for_slice(credential.document.as_bytes())
            .map(|q| {
                leaf(&q.map_err(|_| crate::Error("quad parsing rejected"))?)
                    .ok_or(crate::Error("triple term"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let root = root(&leaves).ok_or(crate::Error("leaf order or count"))?;
        let count = u32::try_from(leaves.len())
            .map_err(|_| crate::Error("quad count exceeds member range"))?;
        let salt: [u8; SALT_BYTES] = credential.signature[64..]
            .try_into()
            .map_err(|_| crate::Error("salt length"))?;
        let digest: [u8; 32] = Sha256::digest(credential.proof_config.as_bytes()).into();
        let message = signed_message(&salt, count, &root, &digest);
        let signature = ed25519_dalek::Signature::from_slice(&credential.signature[..64])
            .map_err(|_| crate::Error("signature encoding"))?;
        key.verify_strict(&message, &signature)
            .map_err(|_| crate::Error("signature rejected"))
    };
    check().is_ok()
}

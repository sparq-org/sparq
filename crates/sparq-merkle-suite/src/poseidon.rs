//! Field-native member. All primitive operations reuse `sparq-zk`.
use crate::{comparison_key, Error, Fr, KEY_BYTES};
use ark_ff::{BigInteger, PrimeField};
use oxrdf::{GraphName, Quad, Term};
use sha2::{Digest, Sha256};
use sparq_zk::{poseidon2, sig};

/// Data Integrity cryptosuite name.
pub const SCHNORR_POSEIDON2_MERKLE_2026: &str = "schnorr-poseidon2-merkle-2026";
/// Byte chunks are unsigned, big-endian integers strictly below the field modulus.
pub const PACK_BYTES: usize = 31;
/// Domains are the big-endian integers of the stated ASCII labels.
pub const TERM_DOMAIN: u64 = u64::from_be_bytes(*b"MS26TERM");
/// Unsalted quad digest domain (private inside a presentation).
pub const QUAD_DOMAIN: u64 = u64::from_be_bytes(*b"MS26QUAD");
/// Derivation of the leaf salt from the tree salt and index.
pub const SALT_DOMAIN: u64 = u64::from_be_bytes(*b"MS26SALT");
/// Salted leaf digest domain.
pub const LEAF_DOMAIN: u64 = u64::from_be_bytes(*b"MS26LEAF");
/// Binary tree node domain.
pub const NODE_DOMAIN: u64 = u64::from_be_bytes(*b"MS26NODE");
/// Signed credential message domain.
pub const MESSAGE_DOMAIN: u64 = u64::from_be_bytes(*b"MS26MESG");
/// Datatype identifier domain.
pub const DATATYPE_DOMAIN: u64 = u64::from_be_bytes(*b"MS26TYPE");
/// Language identifier domain.
pub const LANGUAGE_DOMAIN: u64 = u64::from_be_bytes(*b"MS26LANG");
/// Suite identifier domain.
pub const SUITE_DOMAIN: u64 = u64::from_be_bytes(*b"MS26SUIT");

/// Length-bound byte sponge: `P(domain, byte_length, packed_chunks...)`.
/// The last chunk retains its actual width; no trailing zero bytes are added.
pub fn hash_bytes(domain: u64, bytes: &[u8]) -> Fr {
    let mut fields = Vec::with_capacity(2 + bytes.len().div_ceil(PACK_BYTES));
    fields.extend([Fr::from(domain), Fr::from(bytes.len() as u64)]);
    fields.extend(bytes.chunks(PACK_BYTES).map(Fr::from_be_bytes_mod_order));
    poseidon2::hash(&fields)
}

/// Field identifier of this cryptosuite, using the suite-specific byte domain.
pub fn suite_id() -> Fr {
    hash_bytes(SUITE_DOMAIN, SCHNORR_POSEIDON2_MERKLE_2026.as_bytes())
}

/// A validated RDF term and its bound cell components for the Noir host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermCell {
    /// Whether the cell is bound. An unbound cell has every other slot zero.
    pub bound: bool,
    /// Default graph, IRI, blank node, or literal, using the suite byte tag.
    pub kind: u8,
    /// `P(kind, term_hash, key)`; equality uses this slot.
    pub enc: Fr,
    /// The key class, also the most significant byte of `key`.
    pub key_class: u8,
    /// The complete comparison key interpreted big-endian.
    pub key: Fr,
    /// The length-bound datatype identifier, or zero for non-literals.
    pub dt: Fr,
    /// The length-bound language identifier, or zero for non-literals.
    pub lang: Fr,
    /// Credential-local blank-node scope; zero for every other kind.
    pub credential: u32,
    /// Opening bytes, without the comparison key.
    pub bytes: Vec<u8>,
}

/// The field encoding of a term. Triple terms are outside this member.
/// Blank-node scope belongs to the dataset cell, not the issuer's RDF encoding.
pub fn term_cell(term: &Term, credential: u32) -> Result<TermCell, Error> {
    if matches!(term, Term::Triple(_)) {
        return Err(Error("triple terms are not admitted"));
    }
    let mut bytes = Vec::new();
    crate::sha256::encode_term(term, &mut bytes);
    let (key, dt, lang) = if let Term::Literal(literal) = term {
        bytes.truncate(bytes.len() - KEY_BYTES);
        (
            comparison_key(literal),
            hash_bytes(DATATYPE_DOMAIN, literal.datatype().as_str().as_bytes()),
            hash_bytes(LANGUAGE_DOMAIN, literal.language().unwrap_or("").as_bytes()),
        )
    } else {
        ([0; KEY_BYTES], Fr::from(0u64), Fr::from(0u64))
    };
    Ok(cell(
        bytes,
        key,
        dt,
        lang,
        if matches!(term, Term::BlankNode(_)) {
            credential
        } else {
            0
        },
    ))
}

fn cell(bytes: Vec<u8>, key: [u8; KEY_BYTES], dt: Fr, lang: Fr, credential: u32) -> TermCell {
    let kind = bytes[0];
    let k = Fr::from_be_bytes_mod_order(&key);
    TermCell {
        bound: true,
        kind,
        enc: poseidon2::hash(&[Fr::from(kind), hash_bytes(TERM_DOMAIN, &bytes), k]),
        key_class: key[0],
        key: k,
        dt,
        lang,
        credential,
        bytes,
    }
}

/// The default graph's bound cell. This is distinct from an unbound table cell.
pub fn default_graph() -> TermCell {
    cell(vec![0], [0; KEY_BYTES], Fr::from(0u64), Fr::from(0u64), 0)
}

/// Every quad's terms, ordered subject, predicate, object, graph.
pub fn quad_cells(quad: &Quad, credential: u32) -> Result<[TermCell; 4], Error> {
    Ok([
        term_cell(&Term::from(quad.subject.clone()), credential)?,
        term_cell(&Term::NamedNode(quad.predicate.clone()), credential)?,
        term_cell(&quad.object, credential)?,
        match &quad.graph_name {
            GraphName::DefaultGraph => default_graph(),
            GraphName::NamedNode(n) => term_cell(&Term::NamedNode(n.clone()), credential)?,
            GraphName::BlankNode(n) => term_cell(&Term::BlankNode(n.clone()), credential)?,
        },
    ])
}

/// Unsalted quad digest, used only for internal ordering and duplicate detection.
pub fn quad_hash(quad: &Quad) -> Result<Fr, Error> {
    let cells = quad_cells(quad, 0)?;
    Ok(hash_quad(cells.map(|c| c.enc)))
}

/// Quad fold over validated term encodings.
pub fn hash_quad(enc: [Fr; 4]) -> Fr {
    poseidon2::hash(&[Fr::from(QUAD_DOMAIN), enc[0], enc[1], enc[2], enc[3]])
}
/// Per-leaf salt. The index is zero-based in strictly increasing quad order.
pub fn leaf_salt(salt: Fr, index: u32) -> Fr {
    poseidon2::hash(&[Fr::from(SALT_DOMAIN), salt, Fr::from(index)])
}
/// Salted leaf.
pub fn leaf(quad_hash: Fr, salt: Fr, index: u32) -> Fr {
    poseidon2::hash(&[Fr::from(LEAF_DOMAIN), quad_hash, leaf_salt(salt, index)])
}

/// Minimal binary tree over salted leaves, with zero leaves to the next power of
/// two. `quads` must be nonempty and strictly increasing as canonical integers.
pub fn root(quads: &[Fr], salt: Fr) -> Result<Fr, Error> {
    if quads.is_empty()
        || quads.len() > u32::MAX as usize
        || quads
            .windows(2)
            .any(|p| p[0].into_bigint() >= p[1].into_bigint())
    {
        return Err(Error(
            "quad hashes must be nonempty and strictly increasing",
        ));
    }
    let mut row: Vec<_> = quads
        .iter()
        .enumerate()
        .map(|(i, q)| leaf(*q, salt, i as u32))
        .collect();
    row.resize(row.len().next_power_of_two(), Fr::from(0u64));
    while row.len() > 1 {
        row = row
            .chunks_exact(2)
            .map(|p| poseidon2::hash(&[Fr::from(NODE_DOMAIN), p[0], p[1]]))
            .collect();
    }
    Ok(row[0])
}

/// Signed message, binding suite, private salt, exact count, salted root, and
/// both unsigned halves of the SHA-256 canonical configuration digest.
pub fn signed_message(salt: Fr, count: u32, root: Fr, configuration: &[u8; 32]) -> Fr {
    message_for_suite(suite_id(), salt, count, root, configuration)
}

/// Explicit suite slot for cross-member negative controls and future members.
pub fn message_for_suite(
    suite: Fr,
    salt: Fr,
    count: u32,
    root: Fr,
    configuration: &[u8; 32],
) -> Fr {
    poseidon2::hash(&[
        Fr::from(MESSAGE_DOMAIN),
        suite,
        salt,
        Fr::from(count),
        root,
        Fr::from_be_bytes_mod_order(&configuration[..16]),
        Fr::from_be_bytes_mod_order(&configuration[16..]),
    ])
}

/// A complete field-native credential. Salt is private prover material.
#[derive(Debug, Clone)]
pub struct Credential {
    /// Canonical N-Quads in strictly increasing quad-hash order.
    pub document: String,
    /// Canonical Data Integrity configuration.
    pub proof_config: String,
    /// Private, canonical field salt. Issuers must draw a fresh uniform salt.
    pub salt: Fr,
    /// The signed quad count.
    pub count: u32,
    /// Salted Merkle root, private inside the query method.
    pub root: Fr,
    /// Signature over the member's salted message.
    pub signature: sig::Signature,
}

/// Issue after RDFC-1.0 canonicalization. The signer is the existing Baby Jubjub
/// signer; the caller supplies fresh private field randomness.
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &sig::SecretKey,
    salt: Fr,
) -> Result<Credential, Error> {
    let document = sparq_canon::canonicalize_nquads(document)
        .map_err(|_| Error("document canonicalization rejected"))?;
    let proof_config = crate::canonical_config(proof_config, SCHNORR_POSEIDON2_MERKLE_2026)?;
    let mut rows = Vec::new();
    for (q, line) in oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .zip(document.lines())
    {
        rows.push((
            quad_hash(&q.map_err(|_| Error("quad parsing rejected"))?)?,
            line,
        ));
    }
    rows.sort_unstable_by_key(|(q, _)| q.into_bigint());
    let count = u32::try_from(rows.len()).map_err(|_| Error("quad count exceeds member range"))?;
    let root = root(&rows.iter().map(|(q, _)| *q).collect::<Vec<_>>(), salt)?;
    let digest: [u8; 32] = Sha256::digest(proof_config.as_bytes()).into();
    let signature = sig::sign_deterministic(key, &signed_message(salt, count, root, &digest));
    Ok(Credential {
        document: rows.iter().map(|(_, line)| format!("{line}\n")).collect(),
        proof_config,
        salt,
        count,
        root,
        signature,
    })
}

/// Verify a whole credential held with its private salt, recomputing every quad,
/// canonical label, key, order, tree, count, configuration, and signed message.
pub fn verify(credential: &Credential, key: &sig::PublicKey) -> bool {
    let check = || -> Result<Fr, Error> {
        let proof_config =
            crate::canonical_config(&credential.proof_config, SCHNORR_POSEIDON2_MERKLE_2026)?;
        if proof_config != credential.proof_config {
            return Err(Error("configuration is not canonical"));
        }
        let canonical = sparq_canon::canonicalize_nquads(&credential.document)
            .map_err(|_| Error("document canonicalization rejected"))?;
        let mut original = credential.document.lines().collect::<Vec<_>>();
        original.sort_unstable();
        if original.join("\n") + "\n" != canonical {
            return Err(Error("document is not canonical"));
        }
        let qs = oxttl::NQuadsParser::new()
            .for_slice(credential.document.as_bytes())
            .map(|q| quad_hash(&q.map_err(|_| Error("quad parsing rejected"))?))
            .collect::<Result<Vec<_>, _>>()?;
        if qs.len() != credential.count as usize || root(&qs, credential.salt)? != credential.root {
            return Err(Error("count or root differs"));
        }
        let digest: [u8; 32] = Sha256::digest(proof_config.as_bytes()).into();
        Ok(signed_message(
            credential.salt,
            credential.count,
            credential.root,
            &digest,
        ))
    };
    check().is_ok_and(|message| sig::verify(key, &message, &credential.signature))
}

/// Canonical byte representation for witnesses and vector files.
pub fn field_bytes(field: Fr) -> [u8; 32] {
    field
        .into_bigint()
        .to_bytes_be()
        .try_into()
        .expect("BN254 field byte width")
}

//! Authenticated proof metadata and the suite's private-use Multikey profile.
use crate::{poseidon as p, Error, Fr};
use oxrdf::{Literal, Term};
use sparq_zk::{poseidon2, sig};

/// Proof metadata domain, big-endian ASCII.
pub const METADATA_DOMAIN: u64 = u64::from_be_bytes(*b"MS26META");
/// Synthetic per-credential graph name domain, big-endian ASCII.
pub const GRAPH_DOMAIN: u64 = u64::from_be_bytes(*b"MS26GRPH");
/// Private-use multicodec for uncompressed canonical Baby Jubjub coordinates.
pub const MULTIKEY_CODEC: u32 = 0x300001;
/// Unsigned varint encoding of the private-use codec.
pub const MULTIKEY_PREFIX: [u8; 4] = [0x81, 0x80, 0xc0, 0x01];
/// Byte length of the lower-case base16 Multibase key string.
pub const MULTIKEY_LENGTH: usize = 137;
/// Supported lexical capacity buckets.
pub const LEXICAL_BUCKETS: [usize; 3] = [32, 128, 512];
/// Datatype opening capacity.
pub const DATATYPE_CAPACITY: usize = 128;
/// Language opening capacity.
pub const LANGUAGE_CAPACITY: usize = 64;

/// Suite-specific Multikey: `f` + hex(varint(codec) || x_be32 || y_be32).
/// This private-use codec is not a registered interoperable key type.
pub fn multikey(key: &sig::PublicKey) -> Result<String, Error> {
    if !key.is_prime_order() {
        return Err(Error("key is not a nonidentity prime-order point"));
    }
    let (x, y) = key.coords().ok_or(Error("identity key is not admitted"))?;
    let mut text = String::from("f");
    for byte in MULTIKEY_PREFIX
        .into_iter()
        .chain(p::field_bytes(x))
        .chain(p::field_bytes(y))
    {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("string formatting");
    }
    Ok(text)
}
/// Literal key cell, derived from the actual signature verification key.
pub fn key_cell(key: &sig::PublicKey) -> Result<p::TermCell, Error> {
    p::term_cell(
        &Term::Literal(Literal::new_simple_literal(multikey(key)?)),
        0,
    )
}
/// Proof metadata extracted from the exact canonical configuration.
#[derive(Clone, Debug)]
pub struct Metadata {
    /// Absent creation is None; unsupported lexical values remain bound terms.
    pub created: Option<Term>,
    /// The verification key's Multikey literal.
    pub verification_key: p::TermCell,
    /// Verification-method IRI from the proof, when present.
    pub verification_method: Option<Term>,
    /// Signed metadata commitment.
    pub digest: Fr,
}
/// Extract metadata before issuance/verification. This helper alone does not verify a signature.
pub fn from_configuration(configuration: &str, key: &sig::PublicKey) -> Result<Metadata, Error> {
    let quads = oxttl::NQuadsParser::new()
        .for_slice(configuration.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error("configuration parsing rejected"))?;
    let proof = quads
        .iter()
        .find(|q| q.predicate.as_str() == "https://w3id.org/security#cryptosuite")
        .ok_or(Error("proof suite absent"))?;
    let creation = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "http://purl.org/dc/terms/created")
        .collect::<Vec<_>>();
    if creation.len() > 1
        || creation.iter().any(|q| {
            q.subject != proof.subject
                || q.graph_name != proof.graph_name
                || !matches!(q.object, Term::Literal(_))
        })
    {
        return Err(Error("creation metadata is not a singleton proof literal"));
    }
    let created = creation.first().map(|q| q.object.clone());
    let methods = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "https://w3id.org/security#verificationMethod")
        .collect::<Vec<_>>();
    if methods.len() > 1
        || methods.iter().any(|q| {
            q.subject != proof.subject
                || q.graph_name != proof.graph_name
                || !matches!(q.object, Term::NamedNode(_))
        })
    {
        return Err(Error("verification method is not a singleton proof IRI"));
    }
    let verification_method = methods.first().map(|q| q.object.clone());
    let method_enc = verification_method
        .as_ref()
        .map(|t| p::term_cell(t, 0).map(|c| c.enc))
        .transpose()?
        .unwrap_or(Fr::from(0u64));
    let key = key_cell(key)?;
    let created_enc = created
        .as_ref()
        .map(|t| p::term_cell(t, 0).map(|c| c.enc))
        .transpose()?
        .unwrap_or(Fr::from(0u64));
    let digest = poseidon2::hash(&[Fr::from(METADATA_DOMAIN), created_enc, key.enc, method_enc]);
    Ok(Metadata {
        created,
        verification_key: key,
        verification_method,
        digest,
    })
}
/// Synthetic blank graph name, separate from issuer-supplied blank-node labels.
pub fn graph_name(request: [u128; 2], index: u32) -> Result<p::TermCell, Error> {
    let scope = index
        .checked_add(1)
        .ok_or(Error("credential scope capacity"))?;
    let mut cell = p::default_graph();
    cell.kind = crate::term_kind::BLANK_NODE;
    cell.enc = poseidon2::hash(&[
        Fr::from(GRAPH_DOMAIN),
        Fr::from(request[0]),
        Fr::from(request[1]),
        Fr::from(index),
    ]);
    // Synthetic graph names already have a dedicated, key-free encoding.
    cell.identity = cell.enc;
    cell.credential = scope;
    cell.bytes.clear();
    Ok(cell)
}

/// RDF vocabulary used by credential-envelope wiring, with native term encodings.
pub const VOCABULARY: &[(&str, &str)] = &[
    (
        "RDF_TYPE",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
    ),
    (
        "VC_TYPE",
        "https://www.w3.org/2018/credentials#VerifiableCredential",
    ),
    (
        "VALID_FROM",
        "https://www.w3.org/2018/credentials#validFrom",
    ),
    (
        "VALID_UNTIL",
        "https://www.w3.org/2018/credentials#validUntil",
    ),
    ("ISSUER", "https://www.w3.org/2018/credentials#issuer"),
    (
        "SUBJECT",
        "https://www.w3.org/2018/credentials#credentialSubject",
    ),
    ("MULTIKEY", "https://w3id.org/security#Multikey"),
    ("PUBLIC_KEY", "https://w3id.org/security#publicKeyMultibase"),
    (
        "CREDENTIAL_STATUS",
        "https://www.w3.org/2018/credentials#credentialStatus",
    ),
    (
        "STATUS_ENTRY",
        "https://www.w3.org/ns/credentials/status#BitstringStatusListEntry",
    ),
    (
        "STATUS_LIST",
        "https://www.w3.org/ns/credentials/status#statusListCredential",
    ),
    (
        "STATUS_INDEX",
        "https://www.w3.org/ns/credentials/status#statusListIndex",
    ),
    (
        "STATUS_PURPOSE",
        "https://www.w3.org/ns/credentials/status#statusPurpose",
    ),
    (
        "STATUS_SIZE",
        "https://www.w3.org/ns/credentials/status#statusSize",
    ),
    (
        "ASSERTION_METHOD",
        "https://w3id.org/security#assertionMethod",
    ),
    (
        "CONFIDENCE_METHOD",
        "https://www.w3.org/2018/credentials#confidenceMethod",
    ),
    ("VERIFIED_ISSUER", "https://w3id.org/sparq/vcq#verifiedIssuer"),
];

/// Exact UTC milliseconds of a representable zoned date-time; otherwise unknown.
pub fn timestamp(term: Option<&Term>) -> Option<i64> {
    let Term::Literal(literal) = term? else {
        return None;
    };
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#dateTime" {
        return None;
    }
    let key = crate::comparison_key(literal);
    (key[0] == crate::key_class::DATE_TIME).then(|| {
        (u64::from_be_bytes(key[1..9].try_into().expect("key payload")) ^ (1u64 << 63)) as i64
    })
}
/// Parse a W3C status index string without accepting a sign, whitespace or overflow.
pub fn status_index(literal: &Literal) -> Result<u64, Error> {
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#string"
        || literal.value().is_empty()
        || !literal.value().bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Error("status index is not a decimal string"));
    }
    literal
        .value()
        .parse()
        .map_err(|_| Error("status index exceeds range"))
}

/// Open a literal Multikey against an already resolved public key.
pub fn open_multikey(literal: &Literal, key: &sig::PublicKey) -> Result<p::TermCell, Error> {
    if literal.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#string"
        || literal.value() != multikey(key)?
    {
        return Err(Error("verification key opening differs"));
    }
    p::term_cell(&Term::Literal(literal.clone()), 0)
}

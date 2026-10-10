//! AWS Nitro Enclaves attestation documents: parsing and verification.
//!
//! A document is a CBOR COSE_Sign1 (RFC 9052) signed with ES384 by a leaf
//! certificate that chains to the AWS Nitro Enclaves root G1. Its payload is
//! the CBOR map described in the AWS Nitro Enclaves user guide
//! ("Verifying the root of trust").

use ciborium::Value;
use der::{Decode, Encode};
use p384::ecdsa::signature::Verifier;
use p384::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;

use crate::Rejected;

/// The AWS Nitro Enclaves root certificate G1, as published at
/// <https://aws-nitro-enclaves.amazonaws.com/AWS_NitroEnclaves_Root-G1.zip>.
pub const AWS_ROOT_G1_PEM: &str = include_str!("aws_nitro_root_g1.pem");
/// SHA-256 of the DER of [`AWS_ROOT_G1_PEM`], as published by AWS.
pub const AWS_ROOT_G1_SHA256: [u8; 32] = [
    0x64, 0x1a, 0x03, 0x21, 0xa3, 0xe2, 0x44, 0xef, 0xe4, 0x56, 0x46, 0x31, 0x95, 0xd6, 0x06, 0x31,
    0x7e, 0xd7, 0xcd, 0xcc, 0x3c, 0x17, 0x56, 0xe0, 0x98, 0x93, 0xf3, 0xc6, 0x8f, 0x79, 0xbb, 0x5b,
];

/// Largest attestation document parsed.
pub const MAX_DOCUMENT_BYTES: usize = 16_384;
const MAX_CHAIN: usize = 8;
const MAX_PCRS: usize = 32;
/// Accepted clock skew for a document dated after the verifier's clock.
const MAX_FUTURE_MS: u64 = 60_000;
const ECDSA_WITH_SHA384: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const COSE_ES384: i64 = -35;
const COSE_SIGN1_TAG: u64 = 18;

/// DER of the AWS Nitro Enclaves root G1, checked against its published
/// fingerprint.
pub fn aws_root_g1() -> Vec<u8> {
    let body: String = AWS_ROOT_G1_PEM
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = base64_decode(&body).expect("embedded root is base64");
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(&der)),
        AWS_ROOT_G1_SHA256,
        "embedded AWS root does not match its published fingerprint"
    );
    der
}

/// The verified payload of an attestation document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub module_id: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: u64,
    /// Locked PCRs, by index.
    pub pcrs: Vec<(u64, Vec<u8>)>,
    pub user_data: Option<Vec<u8>>,
    pub nonce: Option<Vec<u8>>,
    pub public_key: Option<Vec<u8>>,
}

/// Verifies `bytes` as an attestation document signed by a chain to `root`,
/// dated no more than `max_age_ms` before `now_ms`, and returns its payload.
///
/// Checks, in order: size; COSE_Sign1 shape and an ES384 protected header;
/// payload fields; `cabundle[0]` equal to `root`; each certificate of
/// `cabundle[1..]` and the leaf signed with ECDSA P-384 / SHA-384 by the one
/// before it, with every non-leaf certificate a CA; every certificate valid at
/// the document's timestamp; the COSE signature under the leaf key; and the
/// timestamp's freshness.
///
/// # Errors
/// Any failed check.
pub fn verify(
    bytes: &[u8],
    root: &[u8],
    now_ms: u64,
    max_age_ms: u64,
) -> Result<Document, Rejected> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Rejected("attestation document capacity"));
    }
    let (protected, payload, signature) = cose_sign1(bytes)?;
    let fields = Fields::parse(&payload)?;

    let bundle = &fields.cabundle;
    if bundle.is_empty() || bundle.len() > MAX_CHAIN {
        return Err(Rejected("attestation CA bundle length"));
    }
    if bundle[0] != root {
        return Err(Rejected(
            "attestation chain does not start at the trusted root",
        ));
    }
    let mut chain = Vec::with_capacity(bundle.len() + 1);
    for der in bundle.iter().chain(std::iter::once(&fields.certificate)) {
        chain.push(Certificate::from_der(der).map_err(|_| Rejected("malformed certificate"))?);
    }
    let seconds = fields.timestamp / 1000;
    for (index, cert) in chain.iter().enumerate() {
        check_validity(cert, seconds)?;
        let is_leaf = index + 1 == chain.len();
        if !is_leaf && !is_ca(cert) {
            return Err(Rejected("attestation chain certificate is not a CA"));
        }
        let issuer = if index == 0 { cert } else { &chain[index - 1] };
        check_signed_by(cert, issuer)?;
    }
    let leaf = key_of(chain.last().expect("chain is non-empty"))?;

    let to_be_signed = sig_structure(&protected, &payload)?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| Rejected("malformed attestation signature"))?;
    leaf.verify(&to_be_signed, &signature)
        .map_err(|_| Rejected("attestation signature does not verify"))?;

    if fields.timestamp > now_ms.saturating_add(MAX_FUTURE_MS) {
        return Err(Rejected("attestation is dated in the future"));
    }
    if now_ms.saturating_sub(fields.timestamp) > max_age_ms {
        return Err(Rejected("attestation is too old"));
    }
    Ok(Document {
        module_id: fields.module_id,
        timestamp: fields.timestamp,
        pcrs: fields.pcrs,
        user_data: fields.user_data,
        nonce: fields.nonce,
        public_key: fields.public_key,
    })
}

/// Protected header bytes, payload and signature of a COSE_Sign1.
type CoseParts = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Splits a COSE_Sign1 into its protected header bytes, payload and signature,
/// and requires the protected header to be `{1: -35}` (ES384).
fn cose_sign1(bytes: &[u8]) -> Result<CoseParts, Rejected> {
    let value: Value =
        ciborium::from_reader(bytes).map_err(|_| Rejected("attestation is not CBOR"))?;
    let value = match value {
        Value::Tag(COSE_SIGN1_TAG, inner) => *inner,
        Value::Tag(..) => return Err(Rejected("attestation has an unexpected CBOR tag")),
        other => other,
    };
    let Value::Array(items) = value else {
        return Err(Rejected("attestation is not a COSE_Sign1"));
    };
    let [protected, unprotected, payload, signature] =
        <[Value; 4]>::try_from(items).map_err(|_| Rejected("attestation is not a COSE_Sign1"))?;
    let (Value::Bytes(protected), Value::Map(_), Value::Bytes(payload), Value::Bytes(signature)) =
        (protected, unprotected, payload, signature)
    else {
        return Err(Rejected("attestation is not a COSE_Sign1"));
    };
    let header: Value = ciborium::from_reader(protected.as_slice())
        .map_err(|_| Rejected("malformed COSE protected header"))?;
    let Value::Map(entries) = header else {
        return Err(Rejected("malformed COSE protected header"));
    };
    let algorithm = entries.iter().find_map(|(key, value)| match key {
        Value::Integer(key) if i128::from(*key) == 1 => Some(value),
        _ => None,
    });
    match algorithm {
        Some(Value::Integer(alg)) if i128::from(*alg) == i128::from(COSE_ES384) => {}
        _ => return Err(Rejected("attestation is not signed with ES384")),
    }
    Ok((protected, payload, signature))
}

/// `Sig_structure = ["Signature1", protected, external_aad = h'', payload]`.
fn sig_structure(protected: &[u8], payload: &[u8]) -> Result<Vec<u8>, Rejected> {
    let value = Value::Array(vec![
        Value::Text("Signature1".into()),
        Value::Bytes(protected.to_vec()),
        Value::Bytes(Vec::new()),
        Value::Bytes(payload.to_vec()),
    ]);
    let mut out = Vec::new();
    ciborium::into_writer(&value, &mut out).map_err(|_| Rejected("COSE encoding"))?;
    Ok(out)
}

struct Fields {
    module_id: String,
    timestamp: u64,
    pcrs: Vec<(u64, Vec<u8>)>,
    certificate: Vec<u8>,
    cabundle: Vec<Vec<u8>>,
    public_key: Option<Vec<u8>>,
    user_data: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
}

impl Fields {
    fn parse(payload: &[u8]) -> Result<Self, Rejected> {
        let value: Value = ciborium::from_reader(payload)
            .map_err(|_| Rejected("attestation payload is not CBOR"))?;
        let Value::Map(entries) = value else {
            return Err(Rejected("attestation payload is not a map"));
        };
        let mut module_id = None;
        let mut timestamp = None;
        let mut digest = None;
        let mut pcrs = None;
        let mut certificate = None;
        let mut cabundle = None;
        let mut public_key = None;
        let mut user_data = None;
        let mut nonce = None;
        for (key, value) in entries {
            let Value::Text(key) = key else {
                return Err(Rejected("attestation payload key is not text"));
            };
            let slot_taken = match key.as_str() {
                "module_id" => module_id.replace(text(value)?).is_some(),
                "timestamp" => timestamp.replace(uint(&value)?).is_some(),
                "digest" => digest.replace(text(value)?).is_some(),
                "pcrs" => pcrs.replace(pcr_map(value)?).is_some(),
                "certificate" => certificate.replace(bytes(value, 1024)?).is_some(),
                "cabundle" => cabundle.replace(cert_list(value)?).is_some(),
                "public_key" => public_key.replace(optional_bytes(value)?).is_some(),
                "user_data" => user_data.replace(optional_bytes(value)?).is_some(),
                "nonce" => nonce.replace(optional_bytes(value)?).is_some(),
                _ => return Err(Rejected("unknown attestation payload field")),
            };
            if slot_taken {
                return Err(Rejected("duplicate attestation payload field"));
            }
        }
        if digest.as_deref() != Some("SHA384") {
            return Err(Rejected("attestation PCR digest is not SHA384"));
        }
        Ok(Self {
            module_id: module_id.ok_or(Rejected("attestation has no module_id"))?,
            timestamp: timestamp.ok_or(Rejected("attestation has no timestamp"))?,
            pcrs: pcrs.ok_or(Rejected("attestation has no PCRs"))?,
            certificate: certificate.ok_or(Rejected("attestation has no certificate"))?,
            cabundle: cabundle.ok_or(Rejected("attestation has no CA bundle"))?,
            public_key: public_key.flatten(),
            user_data: user_data.flatten(),
            nonce: nonce.flatten(),
        })
    }
}

fn text(value: Value) -> Result<String, Rejected> {
    match value {
        Value::Text(text) => Ok(text),
        _ => Err(Rejected("attestation field is not text")),
    }
}

fn uint(value: &Value) -> Result<u64, Rejected> {
    match value {
        Value::Integer(n) => u64::try_from(*n).map_err(|_| Rejected("attestation integer range")),
        _ => Err(Rejected("attestation field is not an integer")),
    }
}

fn bytes(value: Value, max: usize) -> Result<Vec<u8>, Rejected> {
    match value {
        Value::Bytes(bytes) if !bytes.is_empty() && bytes.len() <= max => Ok(bytes),
        _ => Err(Rejected("attestation byte field is missing or too long")),
    }
}

fn optional_bytes(value: Value) -> Result<Option<Vec<u8>>, Rejected> {
    match value {
        Value::Null => Ok(None),
        Value::Bytes(bytes) if bytes.len() <= 1024 => Ok(Some(bytes)),
        _ => Err(Rejected(
            "attestation user field is not bytes of at most 1024",
        )),
    }
}

fn cert_list(value: Value) -> Result<Vec<Vec<u8>>, Rejected> {
    let Value::Array(items) = value else {
        return Err(Rejected("attestation CA bundle is not an array"));
    };
    if items.len() > MAX_CHAIN {
        return Err(Rejected("attestation CA bundle length"));
    }
    items.into_iter().map(|item| bytes(item, 1024)).collect()
}

fn pcr_map(value: Value) -> Result<Vec<(u64, Vec<u8>)>, Rejected> {
    let Value::Map(entries) = value else {
        return Err(Rejected("attestation PCRs are not a map"));
    };
    if entries.len() > MAX_PCRS {
        return Err(Rejected("attestation PCR count"));
    }
    let mut pcrs = Vec::with_capacity(entries.len());
    for (index, value) in entries {
        let index = uint(&index)?;
        if index >= MAX_PCRS as u64 || pcrs.iter().any(|(i, _)| *i == index) {
            return Err(Rejected("attestation PCR index"));
        }
        let value = bytes(value, 64)?;
        if ![32, 48, 64].contains(&value.len()) {
            return Err(Rejected("attestation PCR length"));
        }
        pcrs.push((index, value));
    }
    Ok(pcrs)
}

fn check_validity(cert: &Certificate, unix_seconds: u64) -> Result<(), Rejected> {
    let validity = &cert.tbs_certificate.validity;
    let not_before = validity.not_before.to_unix_duration().as_secs();
    let not_after = validity.not_after.to_unix_duration().as_secs();
    if unix_seconds < not_before || unix_seconds > not_after {
        return Err(Rejected(
            "attestation certificate is not valid at the document's time",
        ));
    }
    Ok(())
}

fn is_ca(cert: &Certificate) -> bool {
    use x509_cert::ext::pkix::BasicConstraints;
    let Some(extensions) = &cert.tbs_certificate.extensions else {
        return false;
    };
    extensions.iter().any(|ext| {
        ext.extn_id == const_oid::db::rfc5280::ID_CE_BASIC_CONSTRAINTS
            && BasicConstraints::from_der(ext.extn_value.as_bytes()).is_ok_and(|bc| bc.ca)
    })
}

fn key_of(cert: &Certificate) -> Result<VerifyingKey, Rejected> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let point = spki
        .subject_public_key
        .as_bytes()
        .ok_or(Rejected("malformed certificate key"))?;
    VerifyingKey::from_sec1_bytes(point).map_err(|_| Rejected("certificate key is not P-384"))
}

fn check_signed_by(cert: &Certificate, issuer: &Certificate) -> Result<(), Rejected> {
    if cert.signature_algorithm.oid != ECDSA_WITH_SHA384
        || cert.tbs_certificate.signature.oid != ECDSA_WITH_SHA384
    {
        return Err(Rejected(
            "certificate is not signed with ECDSA P-384 / SHA-384",
        ));
    }
    if cert.tbs_certificate.issuer != issuer.tbs_certificate.subject {
        return Err(Rejected("certificate issuer does not match the chain"));
    }
    let tbs = cert
        .tbs_certificate
        .to_der()
        .map_err(|_| Rejected("certificate encoding"))?;
    let signature = cert
        .signature
        .as_bytes()
        .ok_or(Rejected("malformed certificate signature"))?;
    let signature =
        Signature::from_der(signature).map_err(|_| Rejected("malformed certificate signature"))?;
    key_of(issuer)?
        .verify(&tbs, &signature)
        .map_err(|_| Rejected("certificate signature does not verify"))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for byte in text.bytes().filter(|b| !b.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|a| *a == byte)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

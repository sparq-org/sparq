//! The `eddsa-rdfc-2022` cryptosuite: Ed25519 signatures over the RDFC-1.0
//! canonical form, per [W3C vc-di-eddsa].
//!
//! [W3C vc-di-eddsa]: https://www.w3.org/TR/vc-di-eddsa/
//!
//! # The hashing algorithm (vc-di-eddsa §3 / vc-data-integrity §4)
//!
//! 1. **Transform** the secured document to RDF and **canonicalize** it with
//!    RDFC-1.0 ([`sparq_canon`]) → the canonical N-Quads `transformedDocument`.
//! 2. **Canonicalize** the proof configuration (the proof node *without*
//!    `proofValue`) the same way → the canonical N-Quads `proofConfig`.
//! 3. `transformedDocumentHash = SHA-256(transformedDocument)`,
//!    `proofConfigHash = SHA-256(proofConfig)`.
//! 4. `hashData = proofConfigHash ‖ transformedDocumentHash` (proof config first).
//! 5. **Sign / verify** an Ed25519 signature (RFC 8032) over `hashData`.
//!
//! The signature value is multibase `z`-base58btc (the `proofValue`).
//!
//! # Honest scope
//!
//! This signs/verifies over the **RDF dataset** form of the document — exactly the
//! form RDFC-1.0 canonicalizes and the proof binds to. Transforming a JSON-LD
//! credential to RDF is the caller's job (see the crate-level docs); doing it here
//! would force a JSON-LD context processor onto the lean build. The proof
//! configuration is supplied as a typed [`ProofConfig`] (not extracted from a
//! JSON-LD `proof` node), and its canonical RDF form uses the standard
//! `https://w3id.org/security#` vocabulary.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest as _, Sha256};

pub use ed25519_dalek::VerifyingKey;
use ed25519_dalek::{Signature, Signer, SigningKey as DalekSigningKey, Verifier};

use crate::did::{did_key_for, DidError, DidResolver};

/// The `type` of a W3C Data Integrity proof.
pub const PROOF_TYPE: &str = "DataIntegrityProof";

/// The cryptosuite identifier this crate implements.
pub const CRYPTOSUITE: &str = "eddsa-rdfc-2022";

// The `https://w3id.org/security#` vocabulary terms the proof config RDF uses.
const SEC: &str = "https://w3id.org/security#";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// A signing key — a vetted Ed25519 (RFC 8032) keypair.
///
/// The secret half never leaves the struct; [`SigningKey::did_key`] /
/// [`SigningKey::verifying_key`] expose only the public half.
pub struct SigningKey {
    inner: DalekSigningKey,
}

impl SigningKey {
    /// Generate a fresh Ed25519 signing key from the OS CSPRNG.
    pub fn generate() -> SigningKey {
        let mut seed = [0u8; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut seed);
        SigningKey {
            inner: DalekSigningKey::from_bytes(&seed),
        }
    }

    /// Reconstruct a signing key from its 32-byte Ed25519 seed.
    pub fn from_seed(seed: &[u8; 32]) -> SigningKey {
        SigningKey {
            inner: DalekSigningKey::from_bytes(seed),
        }
    }

    /// The public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.inner.verifying_key()
    }

    /// The canonical `did:key` string (`did:key:z6Mk…`) for this key's public half.
    pub fn did_key(&self) -> String {
        did_key_for(&self.inner.verifying_key())
    }
}

/// The proof configuration (the proof options, minus the signature) supplied to a
/// [`sign`] / [`verify`] call. Its canonical RDF form is hashed into `hashData`, so
/// a verifier and a signer that disagree on any field produce a different hash and
/// the signature fails — binding the signature to the verification method, purpose,
/// time, and domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofConfig {
    /// The `verificationMethod` IRI — the DID (`did:key:…`) or fragment IRI
    /// (`did:web:…#key`) that resolves to the Ed25519 key. **Required.**
    pub verification_method: String,
    /// The proof purpose (`sec:proofPurpose`). Defaults to `assertionMethod`.
    pub proof_purpose: String,
    /// The proof creation time, an `xsd:dateTime` string (`sec:created`). Optional.
    pub created: Option<String>,
    /// An optional domain the proof is bound to (`sec:domain`).
    pub domain: Option<String>,
    /// An optional challenge nonce (`sec:challenge`) — a verifier-supplied
    /// single-use value for replay defence.
    pub challenge: Option<String>,
}

impl ProofConfig {
    /// A proof config with the given `verificationMethod` and the default
    /// `assertionMethod` purpose; no time, domain, or challenge.
    pub fn new(verification_method: impl Into<String>) -> ProofConfig {
        ProofConfig {
            verification_method: verification_method.into(),
            proof_purpose: "assertionMethod".to_string(),
            created: None,
            domain: None,
            challenge: None,
        }
    }

    /// Set the `xsd:dateTime` `created` time.
    pub fn with_created(mut self, created: impl Into<String>) -> ProofConfig {
        self.created = Some(created.into());
        self
    }

    /// Bind the proof to a `domain`.
    pub fn with_domain(mut self, domain: impl Into<String>) -> ProofConfig {
        self.domain = Some(domain.into());
        self
    }

    /// Bind the proof to a verifier `challenge` nonce.
    pub fn with_challenge(mut self, challenge: impl Into<String>) -> ProofConfig {
        self.challenge = Some(challenge.into());
        self
    }
}

/// A completed `eddsa-rdfc-2022` `DataIntegrityProof`: the proof configuration plus
/// the multibase-encoded Ed25519 `proofValue`. This is the portable, self-describing
/// attestation a W3C-conformant verifier checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataIntegrityProof {
    /// Always [`PROOF_TYPE`] (`DataIntegrityProof`).
    pub proof_type: String,
    /// Always [`CRYPTOSUITE`] (`eddsa-rdfc-2022`).
    pub cryptosuite: String,
    /// The proof configuration the signature is bound to.
    pub config: ProofConfig,
    /// The `proofValue`: multibase `z`-base58btc over the 64-byte Ed25519 signature.
    pub proof_value: String,
}

impl DataIntegrityProof {
    /// The `verificationMethod` IRI of the proof config (convenience accessor).
    pub fn verification_method(&self) -> &str {
        &self.config.verification_method
    }
}

/// The outcome of a successful [`verify`]: which verification method's key the
/// signature checked against, and the proof config it was bound to. A returned
/// `VerifiedProof` means *this signer asserted exactly these triples, unmodified*
/// — nothing about confidentiality or zero-knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProof {
    /// The `verificationMethod` IRI whose resolved key verified the signature.
    pub verification_method: String,
    /// The proof configuration the signature was bound to.
    pub config: ProofConfig,
}

/// `eddsa-rdfc-2022` failure.
#[derive(Debug)]
pub enum VcError {
    /// RDFC-1.0 canonicalization of the document or proof config failed.
    Canon(sparq_canon::CanonError),
    /// The `verificationMethod` DID did not resolve to an Ed25519 key.
    Did(DidError),
    /// The `proofValue` did not decode as multibase `z`-base58btc / 64-byte sig.
    BadProofValue(String),
    /// The Ed25519 signature did not verify over `hashData` — the document was
    /// modified, signed by a different key, or the proof config disagrees.
    SignatureInvalid,
    /// The proof's cryptosuite/type is not `eddsa-rdfc-2022` / `DataIntegrityProof`.
    UnsupportedProof(String),
}

impl std::fmt::Display for VcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcError::Canon(e) => write!(f, "RDFC-1.0 canonicalization failed: {}", e),
            VcError::Did(e) => write!(f, "verificationMethod resolution failed: {}", e),
            VcError::BadProofValue(s) => write!(f, "bad proofValue: {}", s),
            VcError::SignatureInvalid => write!(
                f,
                "Ed25519 signature did not verify over the RDFC-1.0 canonical hash data"
            ),
            VcError::UnsupportedProof(s) => write!(f, "unsupported proof: {}", s),
        }
    }
}

impl std::error::Error for VcError {}

impl From<sparq_canon::CanonError> for VcError {
    fn from(e: sparq_canon::CanonError) -> Self {
        VcError::Canon(e)
    }
}

// ---------------------------------------------------------------------------
// Sign
// ---------------------------------------------------------------------------

/// Sign a slice of triples (one graph's content) under `eddsa-rdfc-2022`, producing
/// a portable [`DataIntegrityProof`]. The signature binds to the **RDFC-1.0
/// canonical form** of the triples, so an RDF-isomorphic re-encoding (blank-node
/// relabelling, reordering) verifies identically, and any content change breaks it.
///
/// The proof config's `verificationMethod` must resolve (at verify time) to the
/// public half of `key`; mismatch makes [`verify`] return [`VcError::Did`] or
/// [`VcError::SignatureInvalid`].
pub fn sign(
    triples: &[Triple],
    key: &SigningKey,
    config: &ProofConfig,
) -> Result<DataIntegrityProof, VcError> {
    let hash_data = hash_data(triples, config)?;
    let sig: Signature = key.inner.sign(&hash_data);
    // proofValue: multibase z-base58btc over the 64-byte signature.
    let proof_value = format!("z{}", bs58::encode(sig.to_bytes()).into_string());
    Ok(DataIntegrityProof {
        proof_type: PROOF_TYPE.to_string(),
        cryptosuite: CRYPTOSUITE.to_string(),
        config: config.clone(),
        proof_value,
    })
}

/// Sign the content of a [`sparq_core::Graph`] (a named graph or a whole store's
/// default graph) under `eddsa-rdfc-2022`. Materializes the graph's triples through
/// [`sparq_canon::graph_triples`] and delegates to [`sign`] — the resulting proof is
/// the portable *"this endpoint asserted exactly this graph"* attestation.
pub fn sign_graph(
    graph: &sparq_core::Graph,
    key: &SigningKey,
    config: &ProofConfig,
) -> Result<DataIntegrityProof, VcError> {
    let triples = sparq_canon::graph_triples(graph)?;
    sign(&triples, key, config)
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

/// Verify an `eddsa-rdfc-2022` [`DataIntegrityProof`] over a slice of triples,
/// resolving the proof's `verificationMethod` through `resolver` (e.g.
/// [`crate::did::DidKeyResolver`]).
///
/// On success the returned [`VerifiedProof`] means: *the key the proof names
/// asserted exactly these triples, unmodified.* It fails closed
/// ([`VcError::SignatureInvalid`]) if the triples were changed, a different key
/// signed, or the proof config disagrees with what was signed.
pub fn verify<R: DidResolver>(
    triples: &[Triple],
    proof: &DataIntegrityProof,
    resolver: &R,
) -> Result<VerifiedProof, VcError> {
    if proof.proof_type != PROOF_TYPE {
        return Err(VcError::UnsupportedProof(proof.proof_type.clone()));
    }
    if proof.cryptosuite != CRYPTOSUITE {
        return Err(VcError::UnsupportedProof(proof.cryptosuite.clone()));
    }
    // Resolve the verificationMethod to an Ed25519 key.
    let vk = resolver
        .resolve_str(&proof.config.verification_method)
        .map_err(VcError::Did)?;
    // Decode the proofValue.
    let sig = decode_proof_value(&proof.proof_value)?;
    // Reconstruct hashData from the SAME proof config and verify.
    let hash_data = hash_data(triples, &proof.config)?;
    vk.verify(&hash_data, &sig)
        .map_err(|_| VcError::SignatureInvalid)?;
    Ok(VerifiedProof {
        verification_method: proof.config.verification_method.clone(),
        config: proof.config.clone(),
    })
}

/// Verify an `eddsa-rdfc-2022` proof over the content of a [`sparq_core::Graph`].
/// The store-side counterpart of [`verify`]: materializes the graph's triples and
/// checks the proof against them.
pub fn verify_graph<R: DidResolver>(
    graph: &sparq_core::Graph,
    proof: &DataIntegrityProof,
    resolver: &R,
) -> Result<VerifiedProof, VcError> {
    let triples = sparq_canon::graph_triples(graph)?;
    verify(&triples, proof, resolver)
}

// ---------------------------------------------------------------------------
// Hashing (the shared sign/verify core)
// ---------------------------------------------------------------------------

/// `hashData = SHA-256(canon(proofConfig)) ‖ SHA-256(canon(document))` — the
/// 64-byte input the Ed25519 signature covers (vc-di-eddsa §3, proof config first).
fn hash_data(triples: &[Triple], config: &ProofConfig) -> Result<Vec<u8>, VcError> {
    let doc_canon = sparq_canon::canonicalize_triples(triples)?;
    let cfg_triples = proof_config_triples(config);
    let cfg_canon = sparq_canon::canonicalize_triples(&cfg_triples)?;

    let doc_hash = Sha256::digest(doc_canon.to_nquads().as_bytes());
    let cfg_hash = Sha256::digest(cfg_canon.to_nquads().as_bytes());

    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&cfg_hash); // proof config hash first
    out.extend_from_slice(&doc_hash);
    Ok(out)
}

/// Build the RDF triples representing the proof configuration, over a single fixed
/// blank-node subject (`_:proof`). RDFC-1.0 relabels the blank node deterministically,
/// so the canonical form depends only on the field VALUES — exactly the binding we
/// want. Uses the standard `https://w3id.org/security#` vocabulary.
fn proof_config_triples(config: &ProofConfig) -> Vec<Triple> {
    let subject = || NamedOrBlankNode::BlankNode(oxrdf::BlankNode::new_unchecked("proof"));
    let pred = |local: &str| NamedNode::new_unchecked(format!("{}{}", SEC, local));

    // The four mandatory proof-config statements (rdf:type, cryptosuite,
    // verificationMethod, proofPurpose). Optional fields are pushed below.
    let mut t = vec![
        // rdf:type DataIntegrityProof
        Triple::new(
            subject(),
            NamedNode::new_unchecked(RDF_TYPE),
            Term::NamedNode(pred(PROOF_TYPE)),
        ),
        // sec:cryptosuite "eddsa-rdfc-2022"
        Triple::new(
            subject(),
            pred("cryptosuite"),
            Term::Literal(Literal::new_simple_literal(CRYPTOSUITE)),
        ),
        // sec:verificationMethod <iri>
        Triple::new(
            subject(),
            pred("verificationMethod"),
            Term::NamedNode(NamedNode::new_unchecked(&config.verification_method)),
        ),
        // sec:proofPurpose <iri> (relative to the sec: vocabulary, per VC DI)
        Triple::new(
            subject(),
            pred("proofPurpose"),
            Term::NamedNode(pred(&config.proof_purpose)),
        ),
    ];
    if let Some(created) = &config.created {
        t.push(Triple::new(
            subject(),
            pred("created"),
            Term::Literal(Literal::new_typed_literal(
                created.clone(),
                NamedNode::new_unchecked(XSD_DATETIME),
            )),
        ));
    }
    if let Some(domain) = &config.domain {
        t.push(Triple::new(
            subject(),
            pred("domain"),
            Term::Literal(Literal::new_simple_literal(domain)),
        ));
    }
    if let Some(challenge) = &config.challenge {
        t.push(Triple::new(
            subject(),
            pred("challenge"),
            Term::Literal(Literal::new_simple_literal(challenge)),
        ));
    }
    t
}

/// Decode a multibase `z`-base58btc `proofValue` into a 64-byte Ed25519 signature.
fn decode_proof_value(pv: &str) -> Result<Signature, VcError> {
    let b58 = pv
        .strip_prefix('z')
        .ok_or_else(|| VcError::BadProofValue(format!("not multibase-z: {}", pv)))?;
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| VcError::BadProofValue(format!("base58: {}", e)))?;
    let arr: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| VcError::BadProofValue(format!("expected 64-byte signature: {}", pv)))?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::did::DidKeyResolver;
    use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};

    fn vm_for(key: &SigningKey) -> String {
        let did = key.did_key();
        format!("{did}#{}", did.strip_prefix("did:key:").unwrap())
    }

    fn graph() -> Vec<Triple> {
        vec![
            Triple::new(
                NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://ex/s")),
                NamedNode::new_unchecked("http://ex/p"),
                Term::Literal(Literal::new_simple_literal("v")),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("b0")),
                NamedNode::new_unchecked("http://ex/q"),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/o")),
            ),
        ]
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let key = SigningKey::from_seed(&[1u8; 32]);
        let cfg = ProofConfig::new(vm_for(&key)).with_created("2026-06-22T00:00:00Z");
        let proof = sign(&graph(), &key, &cfg).unwrap();
        assert_eq!(proof.proof_type, PROOF_TYPE);
        assert_eq!(proof.cryptosuite, CRYPTOSUITE);
        let v = verify(&graph(), &proof, &DidKeyResolver).unwrap();
        assert_eq!(v.verification_method, cfg.verification_method);
    }

    /// The load-bearing invariant: an isomorphic re-encoding (different blank-node
    /// labels + permuted order) of the SAME graph verifies under the SAME proof.
    #[test]
    fn verify_is_isomorphism_stable() {
        let key = SigningKey::from_seed(&[2u8; 32]);
        let cfg = ProofConfig::new(vm_for(&key));
        let proof = sign(&graph(), &key, &cfg).unwrap();

        // Re-encode: rename the blank node and swap triple order.
        let reencoded = vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("renamed")),
                NamedNode::new_unchecked("http://ex/q"),
                Term::NamedNode(NamedNode::new_unchecked("http://ex/o")),
            ),
            Triple::new(
                NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://ex/s")),
                NamedNode::new_unchecked("http://ex/p"),
                Term::Literal(Literal::new_simple_literal("v")),
            ),
        ];
        assert!(verify(&reencoded, &proof, &DidKeyResolver).is_ok());
    }

    /// Tamper-evidence: changing one object literal makes verify fail closed.
    #[test]
    fn tamper_fails_closed() {
        let key = SigningKey::from_seed(&[3u8; 32]);
        let cfg = ProofConfig::new(vm_for(&key));
        let proof = sign(&graph(), &key, &cfg).unwrap();

        let mut tampered = graph();
        tampered[0] = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("http://ex/s")),
            NamedNode::new_unchecked("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("DIFFERENT")),
        );
        assert!(matches!(
            verify(&tampered, &proof, &DidKeyResolver),
            Err(VcError::SignatureInvalid)
        ));
    }

    /// Non-repudiation binding: a proof config the signer did not sign (a swapped
    /// challenge) fails closed even with the same key + graph.
    #[test]
    fn config_binding_fails_closed() {
        let key = SigningKey::from_seed(&[4u8; 32]);
        let cfg = ProofConfig::new(vm_for(&key)).with_challenge("nonce-A");
        let mut proof = sign(&graph(), &key, &cfg).unwrap();
        // Verifier expects a different challenge than was signed.
        proof.config.challenge = Some("nonce-B".to_string());
        assert!(matches!(
            verify(&graph(), &proof, &DidKeyResolver),
            Err(VcError::SignatureInvalid)
        ));
    }

    /// A different key's signature does not verify under the proof's named method.
    #[test]
    fn wrong_key_fails_closed() {
        let signer = SigningKey::from_seed(&[5u8; 32]);
        let other = SigningKey::from_seed(&[6u8; 32]);
        // Sign with `signer` but name `other`'s verificationMethod.
        let cfg = ProofConfig::new(vm_for(&other));
        let proof = sign(&graph(), &signer, &cfg).unwrap();
        assert!(matches!(
            verify(&graph(), &proof, &DidKeyResolver),
            Err(VcError::SignatureInvalid)
        ));
    }

    #[test]
    fn rejects_wrong_cryptosuite() {
        let key = SigningKey::from_seed(&[7u8; 32]);
        let cfg = ProofConfig::new(vm_for(&key));
        let mut proof = sign(&graph(), &key, &cfg).unwrap();
        proof.cryptosuite = "bbs-2023".to_string();
        assert!(matches!(
            verify(&graph(), &proof, &DidKeyResolver),
            Err(VcError::UnsupportedProof(_))
        ));
    }
}

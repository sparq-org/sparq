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
//!
//! # Proof-configuration RDF mapping
//!
//! [OPUS-5.5] zkp-14.2: the typed [`ProofConfig`] maps to the same RDF the W3C
//! vc-di-eddsa test vectors canonicalize (<https://www.w3.org/TR/vc-di-eddsa/#test-vectors>,
//! `eddsa-rdfc-2022` representation): `created` is
//! `<http://purl.org/dc/terms/created>` typed `xsd:dateTime`, and `cryptosuite`
//! is a literal typed `<https://w3id.org/security#cryptosuiteString>`. The
//! published proof-configuration hash and `proofValue` are regression-tested.
//!
//! **Incompatibility:** earlier releases used `sec:created` and a plain
//! `cryptosuite` literal. Proofs produced by that mapping do **not** verify
//! under this one, and there is deliberately no legacy fallback — re-sign them.
//!
//! Only the typed subset `ProofConfig` carries is represented (type,
//! cryptosuite, verificationMethod, proofPurpose, created, domain, challenge);
//! any other proof option or `@context` a JSON-LD proof may carry is not
//! preserved.
//!
//! [OPUS-5.5] zkp-14.3: every entry point lexically validates the `ProofConfig`
//! (absolute-IRI `verificationMethod`, supported `proofPurpose`, XSD 1.1
//! `created`) through one seam before any graph materialization,
//! canonicalization, signing, or DID resolution, failing with
//! [`VcError::InvalidProofOption`]. See [`ProofOptionError`] and
//! [`ProofConfig::validate`] for the exact contract and what it does not check.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest as _, Sha256};

pub use ed25519_dalek::VerifyingKey;
use ed25519_dalek::{Signature, Signer, SigningKey as DalekSigningKey, Verifier};

use crate::did::{DidError, DidResolver, did_key_for};
use crate::proof_options::{self, CheckedProofConfig, ProofOptionError};

/// The `type` of a W3C Data Integrity proof.
pub const PROOF_TYPE: &str = "DataIntegrityProof";

/// The cryptosuite identifier this crate implements.
pub const CRYPTOSUITE: &str = "eddsa-rdfc-2022";

// The `https://w3id.org/security#` vocabulary terms the proof config RDF uses.
pub(crate) const SEC: &str = "https://w3id.org/security#";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
// [OPUS-5.5] The Data Integrity `@context` maps `created` to Dublin Core, not
// `sec:created`. Changing this breaks every existing proof and the W3C vectors.
const DCTERMS_CREATED: &str = "http://purl.org/dc/terms/created";
// [OPUS-5.5] Datatype of the `cryptosuite` literal in the Data Integrity
// `@context`. Changing it breaks every existing proof and the W3C vectors.
const CRYPTOSUITE_STRING: &str = "https://w3id.org/security#cryptosuiteString";

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
///
/// Fields are public and unchecked at construction; [`sign`], [`verify`], and
/// their graph wrappers run [`ProofConfig::validate`] first and reject invalid
/// options with [`VcError::InvalidProofOption`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofConfig {
    /// The `verificationMethod` — an absolute IRI such as a DID (`did:key:…`) or
    /// fragment IRI (`did:web:…#key`) that resolves to the Ed25519 key. **Required.**
    /// Hashed verbatim.
    pub verification_method: String,
    /// The proof purpose (`sec:proofPurpose`). Defaults to `assertionMethod`.
    ///
    /// Either one of the compact terms in [`SUPPORTED_PURPOSE_TERMS`] (hashed as
    /// `https://w3id.org/security#<term>`) or, if the value contains `:`, an
    /// absolute IRI hashed verbatim. Other bare terms are rejected; compact IRIs
    /// such as `sec:assertionMethod` are not expanded.
    ///
    /// [`SUPPORTED_PURPOSE_TERMS`]: crate::SUPPORTED_PURPOSE_TERMS
    pub proof_purpose: String,
    /// The proof creation time (`dcterms:created`). Optional.
    ///
    /// Must be an XSD 1.1 `xsd:dateTime` lexical form (year zero, negative years,
    /// `24:00:00`, and an absent timezone are allowed). Hashed exactly as given.
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

    /// Lexically validates the proof options, as every sign/verify call does first.
    ///
    /// Checks that `verification_method` is an absolute IRI, that `proof_purpose`
    /// is a supported compact term or an absolute IRI, and that `created` (if set)
    /// is an XSD 1.1 `xsd:dateTime`. `domain` and `challenge` accept any string.
    ///
    /// Passing is **lexical** well-formedness only. It does not establish that the
    /// verification method is authorized for the purpose, nor that the purpose,
    /// `domain`, `challenge`, or `created` match what a verifier expects; the
    /// signature-only API checks none of those, so enforce them on
    /// [`VerifiedProof::config`].
    ///
    /// # Examples
    ///
    /// ```
    /// use sparq_vc::{ProofConfig, ProofOptionError};
    ///
    /// let vm = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";
    /// assert!(ProofConfig::new(vm).with_created("0000-02-29T24:00:00Z").validate().is_ok());
    ///
    /// let bad = ProofConfig::new(vm).with_created("2023-02-29T00:00:00Z");
    /// assert!(matches!(bad.validate(), Err(ProofOptionError::Created { .. })));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the [`ProofOptionError`] for the first invalid option, in the
    /// order `verification_method`, `proof_purpose`, `created`.
    pub fn validate(&self) -> Result<(), ProofOptionError> {
        proof_options::check(self).map(|_| ())
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
    /// A [`ProofConfig`] option failed lexical validation. Raised before any
    /// canonicalization, signing, or `verificationMethod` resolution. [OPUS-5.5]
    InvalidProofOption(ProofOptionError),
}

impl std::fmt::Display for VcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcError::InvalidProofOption(e) => write!(f, "{}", e),
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

impl From<ProofOptionError> for VcError {
    fn from(e: ProofOptionError) -> Self {
        VcError::InvalidProofOption(e)
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
///
/// # Errors
///
/// [`VcError::InvalidProofOption`] if `config` fails [`ProofConfig::validate`]
/// (checked before canonicalizing anything), or [`VcError::Canon`] if the
/// triples cannot be RDFC-1.0 canonicalized.
pub fn sign(
    triples: &[Triple],
    key: &SigningKey,
    config: &ProofConfig,
) -> Result<DataIntegrityProof, VcError> {
    let checked = proof_options::check(config)?;
    sign_checked(triples, key, &checked)
}

/// Sign the content of a [`sparq_core::Graph`] (a named graph or a whole store's
/// default graph) under `eddsa-rdfc-2022`. Materializes the graph's triples through
/// [`sparq_canon::graph_triples`] and signs them as [`sign`] does — the resulting
/// proof is the portable *"this endpoint asserted exactly this graph"* attestation.
///
/// # Errors
///
/// As [`sign`]; the proof options are validated before the graph is materialized.
pub fn sign_graph(
    graph: &sparq_core::Graph,
    key: &SigningKey,
    config: &ProofConfig,
) -> Result<DataIntegrityProof, VcError> {
    let checked = proof_options::check(config)?;
    let triples = sparq_canon::graph_triples(graph)?;
    sign_checked(&triples, key, &checked)
}

/// Signs `triples` under an already-validated proof configuration.
fn sign_checked(
    triples: &[Triple],
    key: &SigningKey,
    checked: &CheckedProofConfig<'_>,
) -> Result<DataIntegrityProof, VcError> {
    let hash_data = hash_data(triples, checked)?;
    let sig: Signature = key.inner.sign(&hash_data);
    // proofValue: multibase z-base58btc over the 64-byte signature.
    let proof_value = format!("z{}", bs58::encode(sig.to_bytes()).into_string());
    Ok(DataIntegrityProof {
        proof_type: PROOF_TYPE.to_string(),
        cryptosuite: CRYPTOSUITE.to_string(),
        config: checked.config.clone(),
        proof_value,
    })
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
///
/// A successful verify is a signature check only: it does not decide whether the
/// key is authorized for the purpose, or whether the purpose, `domain`,
/// `challenge`, or `created` are the ones the caller expects. Check those on the
/// returned [`VerifiedProof::config`].
///
/// # Errors
///
/// In check order: [`VcError::UnsupportedProof`] for a foreign type/cryptosuite;
/// [`VcError::InvalidProofOption`] if the proof's config fails
/// [`ProofConfig::validate`] (before `resolver` is called or anything is
/// canonicalized); [`VcError::Did`] if resolution fails;
/// [`VcError::BadProofValue`]; [`VcError::Canon`]; [`VcError::SignatureInvalid`].
pub fn verify<R: DidResolver>(
    triples: &[Triple],
    proof: &DataIntegrityProof,
    resolver: &R,
) -> Result<VerifiedProof, VcError> {
    let checked = check_proof(proof)?;
    verify_checked(triples, proof, &checked, resolver)
}

/// Verify an `eddsa-rdfc-2022` proof over the content of a [`sparq_core::Graph`].
/// The store-side counterpart of [`verify`]: materializes the graph's triples and
/// checks the proof against them.
///
/// # Errors
///
/// As [`verify`]; the proof type and options are checked before the graph is
/// materialized.
pub fn verify_graph<R: DidResolver>(
    graph: &sparq_core::Graph,
    proof: &DataIntegrityProof,
    resolver: &R,
) -> Result<VerifiedProof, VcError> {
    let checked = check_proof(proof)?;
    let triples = sparq_canon::graph_triples(graph)?;
    verify_checked(&triples, proof, &checked, resolver)
}

/// Checks the proof's type and cryptosuite, then validates its proof options.
fn check_proof(proof: &DataIntegrityProof) -> Result<CheckedProofConfig<'_>, VcError> {
    if proof.proof_type != PROOF_TYPE {
        return Err(VcError::UnsupportedProof(proof.proof_type.clone()));
    }
    if proof.cryptosuite != CRYPTOSUITE {
        return Err(VcError::UnsupportedProof(proof.cryptosuite.clone()));
    }
    proof_options::check(&proof.config).map_err(VcError::from)
}

/// Verifies `proof` over `triples`, given its already-validated configuration.
fn verify_checked<R: DidResolver>(
    triples: &[Triple],
    proof: &DataIntegrityProof,
    checked: &CheckedProofConfig<'_>,
    resolver: &R,
) -> Result<VerifiedProof, VcError> {
    // Resolve the verificationMethod to an Ed25519 key.
    let vk = resolver
        .resolve_str(&proof.config.verification_method)
        .map_err(VcError::Did)?;
    // Decode the proofValue.
    let sig = decode_proof_value(&proof.proof_value)?;
    // Reconstruct hashData from the SAME proof config and verify.
    let hash_data = hash_data(triples, checked)?;
    vk.verify(&hash_data, &sig)
        .map_err(|_| VcError::SignatureInvalid)?;
    Ok(VerifiedProof {
        verification_method: proof.config.verification_method.clone(),
        config: proof.config.clone(),
    })
}

// ---------------------------------------------------------------------------
// Hashing (the shared sign/verify core)
// ---------------------------------------------------------------------------

/// `hashData = SHA-256(canon(proofConfig)) ‖ SHA-256(canon(document))` — the
/// 64-byte input the Ed25519 signature covers (vc-di-eddsa §3, proof config first).
fn hash_data(triples: &[Triple], checked: &CheckedProofConfig<'_>) -> Result<Vec<u8>, VcError> {
    let doc_canon = sparq_canon::canonicalize_triples(triples)?;
    let cfg_triples = proof_config_triples(checked);
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
/// want. Uses the standard `https://w3id.org/security#` vocabulary, with
/// `dcterms:created` and a `sec:cryptosuiteString`-typed cryptosuite (see the
/// module-level mapping notes).
///
/// Takes only a [`CheckedProofConfig`]: the two option IRIs come pre-parsed from
/// the validation seam, and `pred` is applied to fixed vocabulary terms only —
/// never to caller input. [OPUS-5.5] zkp-14.3
fn proof_config_triples(checked: &CheckedProofConfig<'_>) -> Vec<Triple> {
    let config = checked.config;
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
        // sec:cryptosuite "eddsa-rdfc-2022"^^sec:cryptosuiteString
        Triple::new(
            subject(),
            pred("cryptosuite"),
            Term::Literal(Literal::new_typed_literal(
                CRYPTOSUITE,
                NamedNode::new_unchecked(CRYPTOSUITE_STRING),
            )),
        ),
        // sec:verificationMethod <iri> (validated absolute IRI, verbatim)
        Triple::new(
            subject(),
            pred("verificationMethod"),
            Term::NamedNode(checked.verification_method.clone()),
        ),
        // sec:proofPurpose <iri> (a supported term under sec:, or a verbatim
        // absolute IRI — see `proof_options`)
        Triple::new(
            subject(),
            pred("proofPurpose"),
            Term::NamedNode(checked.proof_purpose.clone()),
        ),
    ];
    if let Some(created) = &config.created {
        // dcterms:created "…"^^xsd:dateTime
        t.push(Triple::new(
            subject(),
            NamedNode::new_unchecked(DCTERMS_CREATED),
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

    /// [OPUS-5.5] zkp-14.2: the proof-config RDF mapping reproduces the W3C
    /// vc-di-eddsa `eddsa-rdfc-2022` test vector byte-for-byte — canonical
    /// proof-config N-Quads (Example 12) and their published SHA-256 (Example 13).
    /// Source: <https://www.w3.org/TR/vc-di-eddsa/#test-vectors> (Recommendation,
    /// 15 May 2025, Appendix B.1). Guards against reverting to `sec:created` or a
    /// plain `cryptosuite` literal.
    #[test]
    fn proof_config_matches_w3c_eddsa_rdfc_2022_vector() {
        const VM: &str = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2\
                          #z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";
        const EXPECTED_NQUADS: &str = concat!(
            "_:c14n0 <http://purl.org/dc/terms/created> \"2023-02-24T23:36:38Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
            "_:c14n0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#DataIntegrityProof> .\n",
            "_:c14n0 <https://w3id.org/security#cryptosuite> \"eddsa-rdfc-2022\"^^<https://w3id.org/security#cryptosuiteString> .\n",
            "_:c14n0 <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> .\n",
            "_:c14n0 <https://w3id.org/security#verificationMethod> <did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2#z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2> .\n",
        );
        const EXPECTED_SHA256: &str =
            "bea7b7acfbad0126b135104024a5f1733e705108f42d59668b05c0c50004c6b0";

        let cfg = ProofConfig::new(VM).with_created("2023-02-24T23:36:38Z");
        // [OPUS-5.5] zkp-14.3: the published options pass validation, and the
        // expanded `sec:assertionMethod` IRI hashes to the same published digest,
        // so the published proofValue verifies under either spelling.
        let expanded = ProofConfig {
            proof_purpose: format!("{SEC}assertionMethod"),
            ..cfg.clone()
        };
        for cfg in [&cfg, &expanded] {
            let checked = proof_options::check(cfg).unwrap();
            let canon = sparq_canon::canonicalize_triples(&proof_config_triples(&checked)).unwrap();
            let nquads = canon.to_nquads();
            assert_eq!(nquads, EXPECTED_NQUADS);
            assert_eq!(
                hex::encode(<Sha256 as sha2::Digest>::digest(nquads.as_bytes())),
                EXPECTED_SHA256
            );
        }
    }

    /// The pre-zkp-14.3 builder: every option value went in through
    /// `new_unchecked`, and the purpose was always appended to `sec:`.
    fn legacy_proof_config_triples(config: &ProofConfig) -> Vec<Triple> {
        let subject = || NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("proof"));
        let pred = |local: &str| NamedNode::new_unchecked(format!("{SEC}{local}"));
        let mut t = vec![
            Triple::new(
                subject(),
                NamedNode::new_unchecked(RDF_TYPE),
                Term::NamedNode(pred(PROOF_TYPE)),
            ),
            Triple::new(
                subject(),
                pred("cryptosuite"),
                Term::Literal(Literal::new_typed_literal(
                    CRYPTOSUITE,
                    NamedNode::new_unchecked(CRYPTOSUITE_STRING),
                )),
            ),
            Triple::new(
                subject(),
                pred("verificationMethod"),
                Term::NamedNode(NamedNode::new_unchecked(&config.verification_method)),
            ),
            Triple::new(
                subject(),
                pred("proofPurpose"),
                Term::NamedNode(pred(&config.proof_purpose)),
            ),
        ];
        if let Some(created) = &config.created {
            t.push(Triple::new(
                subject(),
                NamedNode::new_unchecked(DCTERMS_CREATED),
                Term::Literal(Literal::new_typed_literal(
                    created.clone(),
                    NamedNode::new_unchecked(XSD_DATETIME),
                )),
            ));
        }
        for (local, value) in [("domain", &config.domain), ("challenge", &config.challenge)] {
            if let Some(value) = value {
                t.push(Triple::new(
                    subject(),
                    pred(local),
                    Term::Literal(Literal::new_simple_literal(value)),
                ));
            }
        }
        t
    }

    /// [OPUS-5.5] zkp-14.3: for every config that was already valid, the validated
    /// builder hashes exactly what the unchecked one did, so deterministic Ed25519
    /// produces byte-identical signatures for existing valid inputs.
    #[test]
    fn validated_builder_matches_legacy_mapping_for_valid_configs() {
        let key = SigningKey::from_seed(&[8u8; 32]);
        let vm = vm_for(&key);
        let configs = [
            ProofConfig::new(vm.clone()),
            ProofConfig::new(key.did_key()).with_created("2023-02-24T23:36:38Z"),
            ProofConfig::new(vm.clone())
                .with_created("2026-06-22T12:00:00.250+05:30")
                .with_domain("vc.example")
                .with_challenge("nonce-A"),
            ProofConfig {
                proof_purpose: "authentication".to_string(),
                ..ProofConfig::new(vm.clone())
            },
            ProofConfig {
                proof_purpose: "capabilityDelegation".to_string(),
                ..ProofConfig::new("https://issuer.example/keys/1")
            },
        ];
        for cfg in &configs {
            let checked = proof_options::check(cfg).unwrap();
            let new = sparq_canon::canonicalize_triples(&proof_config_triples(&checked)).unwrap();
            let old = sparq_canon::canonicalize_triples(&legacy_proof_config_triples(cfg)).unwrap();
            assert_eq!(new.to_nquads(), old.to_nquads(), "{cfg:?}");
        }
    }

    /// An absolute purpose IRI is hashed verbatim, never appended to `sec:`.
    #[test]
    fn absolute_purpose_iri_is_not_prefixed_with_sec() {
        let cfg = ProofConfig {
            proof_purpose: "https://example.test/purposes#audit".to_string(),
            ..ProofConfig::new("did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2")
        };
        let checked = proof_options::check(&cfg).unwrap();
        let purpose = proof_config_triples(&checked)
            .into_iter()
            .find(|t| t.predicate.as_str() == format!("{SEC}proofPurpose"))
            .unwrap();
        assert_eq!(
            purpose.object,
            Term::NamedNode(NamedNode::new_unchecked(
                "https://example.test/purposes#audit"
            ))
        );
    }
}

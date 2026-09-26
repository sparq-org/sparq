// [OPUS-5.5] Native-only issuer-authenticated RDF relation; no guest, receipt or proof.
//! Issuer-authenticated bounded RDF queries over W3C `eddsa-rdfc-2022` credentials.
//!
//! This is relation version 5, a native model only. It has no guest adapter, host
//! API, receipt or proof, and it makes no conformance or performance claim. It is
//! not a complete Data Integrity processor and is not externally audited.
//!
//! # Relation
//!
//! A verifier-owned [`Request`] binds a V3 query, V3 evaluation policy, dataset
//! authority and nonce plus a bounded [`AuthorizedKey`] table and the fixed
//! [`Cryptosuite`] and [`Mapping`] profile. The private [`Witness`] holds only 1–4
//! signed credentials (document N-Quads, proof-configuration N-Quads, 64 signature
//! bytes) and a salt. There is no separately supplied query dataset.
//!
//! For each credential, [`evaluate`] and [`dataset_commitment`]:
//!
//! 1. enforce byte and statement bounds before any parsing;
//! 2. parse both inputs, admitting only the default graph and rejecting triple
//!    terms and directional literals, then canonicalize each with bounded
//!    RDFC-1.0/SHA-256;
//! 3. read every check from the exact canonical bytes: one proof node with exactly
//!    `rdf:type sec:DataIntegrityProof`, a `sec:cryptosuite` literal typed
//!    `sec:cryptosuiteString`, one IRI `sec:verificationMethod`,
//!    `sec:proofPurpose sec:assertionMethod` and at most one `dcterms:created`
//!    typed `xsd:dateTime`, with no other option; the document has no `sec:proof`
//!    or `sec:proofValue`, one `VerifiableCredential` node and one IRI issuer;
//! 4. require the verification method to be in the table and the document issuer
//!    to equal that entry's pinned issuer;
//! 5. verify strict Ed25519 over `SHA-256(config) || SHA-256(document)`.
//!
//! Verified credentials are sorted by document hash, and duplicates reject. The
//! exact hashed canonical document quads are unioned into one default graph,
//! with blank nodes renamed `k{index}_{label}` so that credentials never share a
//! blank node. Literal lexical forms are copied unchanged. V3 then evaluates that
//! graph under an internal `HolderDeclared` authority; only its result is kept.
//!
//! # Commitment
//!
//! [`dataset_commitment`] is SHA-256 over a version-5 domain tag, the policy
//! digest, the salt, a `u32` credential count and, per sorted credential, the
//! 32-byte canonical document and proof-configuration hashes that were signed.
//! The policy digest frames the suite and mapping profile tags, every fixed bound,
//! the serialized V3 policy and the table, sorted by verification method. A
//! commitment under one policy therefore cannot stand for another. Salt reuse is
//! publicly linkable.
//!
//! # Not established
//!
//! Neither [`Provenance`] implies wallet or world completeness: the holder may
//! omit credentials. Credential status, holder binding, validity periods,
//! `created` against a clock, and DID or controller resolution are not checked.
//! The table is the verifier's only trust input.
//!
//! `created` accepts only a restricted lexical profile: positive four-digit years
//! from `0001`, whole seconds and `Z`. Other valid XML Schema 1.1
//! `dateTimeStamp` values reject as unsupported, and malformed values reject as
//! malformed. Full typed proof-option handling is left to a later integration.

use crate::{DatasetAuthority, ProofContract, Rejected, v3};
use ed25519_dalek::{Signature, VerifyingKey};
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Quad, Term};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparq_canon::{CanonicalizationLimits, canonicalize_quads_bounded_with};
use std::fmt::{self, Write};

/// Wire version, distinct from V1–V3 and from historical V4 work.
pub const VERSION: u32 = 5;
/// Maximum signed credentials per witness; at least one is required.
pub const MAX_CREDENTIALS: usize = 4;
/// Maximum verifier-authorized keys in one request.
pub const MAX_AUTHORIZED_KEYS: usize = 16;
/// Maximum bytes of each issuer or verification-method IRI in the table.
pub const MAX_IRI_BYTES: usize = 512;
/// Maximum source bytes of one document input.
pub const MAX_DOCUMENT_BYTES: usize = 8_192;
/// Maximum source bytes of one proof-configuration input.
pub const MAX_PROOF_CONFIG_BYTES: usize = 2_048;
/// Maximum document plus proof-configuration bytes across the witness.
pub const MAX_TOTAL_BYTES: usize = 32_768;
/// Maximum statements in one document.
pub const MAX_DOCUMENT_QUADS: usize = 128;
/// Maximum statements in one proof configuration before option checks.
pub const MAX_PROOF_CONFIG_QUADS: usize = 8;
/// Maximum document statements across the witness; equals the V3 default ceiling.
pub const MAX_TOTAL_QUADS: usize = 256;

// Per-input RDFC-1.0 limits. Input and output allow re-serialization escapes and
// canonical labels; exceeding them rejects and never truncates. HNDQ and
// permutation ceilings equal the V3 defaults.
const RDFC_LIMITS: CanonicalizationLimits = CanonicalizationLimits {
    max_quads: MAX_DOCUMENT_QUADS,
    max_input_bytes: 2 * MAX_DOCUMENT_BYTES,
    max_output_bytes: 2 * MAX_DOCUMENT_BYTES + 4_096,
    max_hndq_calls: 64,
    max_permutation_steps: 1_048_576,
};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const VC_TYPE: &str = "https://www.w3.org/2018/credentials#VerifiableCredential";
const VC_ISSUER: &str = "https://www.w3.org/2018/credentials#issuer";
const SEC_PROOF: &str = "https://w3id.org/security#proof";
const SEC_PROOF_VALUE: &str = "https://w3id.org/security#proofValue";
const SEC_DATA_INTEGRITY_PROOF: &str = "https://w3id.org/security#DataIntegrityProof";
const SEC_CRYPTOSUITE: &str = "https://w3id.org/security#cryptosuite";
const SEC_CRYPTOSUITE_STRING: &str = "https://w3id.org/security#cryptosuiteString";
const SEC_VERIFICATION_METHOD: &str = "https://w3id.org/security#verificationMethod";
const SEC_PROOF_PURPOSE: &str = "https://w3id.org/security#proofPurpose";
const SEC_ASSERTION_METHOD: &str = "https://w3id.org/security#assertionMethod";
const DC_CREATED: &str = "http://purl.org/dc/terms/created";
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const EDDSA_RDFC_2022: &str = "eddsa-rdfc-2022";

/// Fixed cryptosuite profile; a holder cannot substitute another algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cryptosuite {
    /// W3C `eddsa-rdfc-2022`: RDFC-1.0, SHA-256, strict Ed25519, `assertionMethod`.
    EddsaRdfc2022,
}

/// Fixed mapping from verified credentials to the private V3 dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mapping {
    /// Default-graph union of hashed canonical documents, blank nodes scoped per credential.
    ScopedCanonicalUnion,
}

/// One verifier-pinned Ed25519 key authorized for an issuer's assertions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizedKey {
    /// Absolute issuer IRI that the credential's single `issuer` must equal.
    pub issuer: String,
    /// Absolute IRI the proof configuration must name; unique within a table.
    pub verification_method: String,
    /// Compressed Ed25519 key; invalid and small-order points reject.
    pub public_key: [u8; 32],
}

/// Complete verifier-owned policy bound into the request and commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Unchanged V3 source, result and canonicalization capacities.
    pub evaluation: v3::Policy,
    /// 1–16 entries; order is not significant and digests sort by method IRI.
    pub authorization: Vec<AuthorizedKey>,
    pub cryptosuite: Cryptosuite,
    pub mapping: Mapping,
}

impl Policy {
    /// Creates the fixed-profile policy with default V3 capacities.
    pub fn new(authorization: Vec<AuthorizedKey>) -> Self {
        Self {
            evaluation: v3::Policy::default(),
            authorization,
            cryptosuite: Cryptosuite::EddsaRdfc2022,
            mapping: Mapping::ScopedCanonicalUnion,
        }
    }
}

/// Independent verifier expectations; never derive these from a presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub query: String,
    /// `VerifierAgreed` names a commitment from [`dataset_commitment`], not a V3 anchor.
    pub authority: DatasetAuthority,
    pub policy: Policy,
    pub nonce: [u8; 32],
}

/// One secured credential split into its signed parts.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCredential {
    /// N-Quads of the unsecured document; canonicalized before hashing.
    pub document: String,
    /// N-Quads of the proof configuration, without `proofValue`.
    pub proof_config: String,
    /// The 64 raw Ed25519 signature bytes decoded from `proofValue`.
    pub signature: Vec<u8>,
}

impl fmt::Debug for SignedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignedCredential([REDACTED])")
    }
}

/// Private credentials and blinding; debug output redacts both.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateCredentials {
    pub credentials: Vec<SignedCredential>,
    /// Independently sampled cryptographic salt; never reuse fixture salts.
    pub salt: [u8; 32],
}

impl fmt::Debug for PrivateCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateCredentials([REDACTED])")
    }
}

/// Private input; the relation builds its own query dataset and result.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub request: Request,
    pub dataset: PrivateCredentials,
}

/// Source assurance; neither variant implies completeness, status or holder binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    /// Pinned-key-authenticated credentials matching the verifier's accepted commitment.
    VerifierAgreedAuthenticated,
    /// Pinned-key-authenticated credentials that the holder chose to include.
    HolderSelectedAuthenticated,
}

/// Public relation output, bound to the independently expected request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub request_digest: [u8; 32],
    /// Authenticated commitment; it publishes no credential count or issuer list.
    pub dataset_commitment: [u8; 32],
    pub provenance: Provenance,
    pub result: v3::CanonicalResult,
}

impl Cryptosuite {
    fn profile(self) -> &'static str {
        match self {
            Self::EddsaRdfc2022 => "eddsa-rdfc-2022:rdfc-1.0:sha-256:ed25519-strict:assertionMethod",
        }
    }
}

impl Mapping {
    fn profile(self) -> &'static str {
        match self {
            Self::ScopedCanonicalUnion => "hashed-canonical-default-graph-union:per-credential-blank-scope",
        }
    }
}

type Entry<'a> = (&'a AuthorizedKey, VerifyingKey);

fn checked_table(keys: &[AuthorizedKey]) -> Result<Vec<Entry<'_>>, Rejected> {
    if keys.is_empty() || keys.len() > MAX_AUTHORIZED_KEYS {
        return Err(Rejected("authorization table must hold 1 to 16 keys"));
    }
    let mut table = Vec::with_capacity(keys.len());
    for entry in keys {
        for iri in [&entry.issuer, &entry.verification_method] {
            if iri.len() > MAX_IRI_BYTES || NamedNode::new(iri.as_str()).is_err() {
                return Err(Rejected("authorization table IRI rejected"));
            }
        }
        let key = VerifyingKey::from_bytes(&entry.public_key)
            .map_err(|_| Rejected("authorized key is not a valid Ed25519 point"))?;
        if key.is_weak() {
            return Err(Rejected("authorized key has small order"));
        }
        table.push((entry, key));
    }
    table.sort_unstable_by(|a, b| a.0.verification_method.cmp(&b.0.verification_method));
    if table
        .windows(2)
        .any(|pair| pair[0].0.verification_method == pair[1].0.verification_method)
    {
        return Err(Rejected("authorization table verification methods are not unique"));
    }
    Ok(table)
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn policy_digest(policy: &Policy, table: &[Entry<'_>]) -> Result<[u8; 32], Rejected> {
    let evaluation = serde_json::to_vec(&policy.evaluation)
        .map_err(|_| Rejected("authenticated evaluation policy serialization"))?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:authenticated-rdf:policy:v5\0");
    frame(&mut hash, policy.cryptosuite.profile().as_bytes());
    frame(&mut hash, policy.mapping.profile().as_bytes());
    for bound in [
        MAX_CREDENTIALS,
        MAX_AUTHORIZED_KEYS,
        MAX_IRI_BYTES,
        MAX_DOCUMENT_BYTES,
        MAX_PROOF_CONFIG_BYTES,
        MAX_TOTAL_BYTES,
        MAX_DOCUMENT_QUADS,
        MAX_PROOF_CONFIG_QUADS,
        MAX_TOTAL_QUADS,
        RDFC_LIMITS.max_quads,
        RDFC_LIMITS.max_input_bytes,
        RDFC_LIMITS.max_output_bytes,
        RDFC_LIMITS.max_hndq_calls,
        RDFC_LIMITS.max_permutation_steps,
    ] {
        hash.update((bound as u64).to_le_bytes());
    }
    frame(&mut hash, &evaluation);
    hash.update((table.len() as u32).to_le_bytes());
    for (entry, _) in table {
        frame(&mut hash, entry.issuer.as_bytes());
        frame(&mut hash, entry.verification_method.as_bytes());
        hash.update(entry.public_key);
    }
    Ok(hash.finalize().into())
}

fn inner_request(request: &Request) -> v3::Request {
    v3::Request {
        version: v3::VERSION,
        contract: ProofContract::ExactDataset,
        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
        query: request.query.clone(),
        authority: DatasetAuthority::HolderDeclared,
        policy: request.policy.evaluation.clone(),
        nonce: request.nonce,
    }
}

/// Validates the version, embedded V3 request fields and authorization table.
///
/// # Errors
/// Rejects other versions, V3 query/nonce/policy violations, and tables that are
/// empty, oversized, hold invalid IRIs or keys, or repeat a verification method.
pub fn validate_request(request: &Request) -> Result<(), Rejected> {
    if request.version != VERSION {
        return Err(Rejected("unsupported authenticated RDF version"));
    }
    v3::validate_request(&inner_request(request))?;
    checked_table(&request.policy.authorization).map(drop)
}

/// Hashes every request and policy field under a framed version-5 domain.
///
/// The inner V3 contract and dialect are fixed by this version and domain.
///
/// # Errors
/// Rejects requests that fail [`validate_request`].
pub fn request_digest(request: &Request) -> Result<[u8; 32], Rejected> {
    validate_request(request)?;
    let table = checked_table(&request.policy.authorization)?;
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:authenticated-rdf:request:v5\0");
    hash.update(request.version.to_le_bytes());
    hash.update(policy_digest(&request.policy, &table)?);
    frame(&mut hash, request.query.as_bytes());
    match &request.authority {
        DatasetAuthority::HolderDeclared => hash.update([0u8]),
        DatasetAuthority::VerifierAgreed { commitment } => {
            hash.update([1u8]);
            hash.update(commitment);
        }
    }
    hash.update(request.nonce);
    Ok(hash.finalize().into())
}

/// Authenticates every credential and returns the blinded authenticated commitment.
///
/// A verifier computes this from an independently obtained credential set and
/// salt before selecting `VerifierAgreed`. The module docs list what is committed.
///
/// # Errors
/// Rejects evaluation policies outside the V3 program ceilings (with the same
/// rejection as [`validate_request`]), invalid tables, zero salt, capacity
/// violations, malformed or unsupported RDF or proof options, unauthorized
/// methods or issuers, failed signatures and duplicate canonical documents.
pub fn dataset_commitment(dataset: &PrivateCredentials, policy: &Policy) -> Result<[u8; 32], Rejected> {
    authenticate(dataset, policy).map(|authenticated| authenticated.commitment)
}

/// Authenticates the credentials and evaluates the V3 query on their mapped union.
///
/// # Errors
/// Rejects invalid requests, any authentication failure, an unequal
/// verifier-agreed commitment, and every V3 evaluation or capacity rejection.
pub fn evaluate(witness: &Witness) -> Result<Journal, Rejected> {
    let request = &witness.request;
    validate_request(request)?;
    let authenticated = authenticate(&witness.dataset, &request.policy)?;
    let provenance = match &request.authority {
        DatasetAuthority::VerifierAgreed { commitment } if *commitment == authenticated.commitment => {
            Provenance::VerifierAgreedAuthenticated
        }
        DatasetAuthority::VerifierAgreed { .. } => {
            return Err(Rejected("authenticated dataset anchor mismatch"));
        }
        DatasetAuthority::HolderDeclared => Provenance::HolderSelectedAuthenticated,
    };
    // The inner V3 journal's request digest and source commitment are discarded;
    // only this outer journal binds the request and authenticated commitment.
    let inner = v3::evaluate(&v3::Witness {
        request: inner_request(request),
        dataset: v3::PrivateDataset {
            nquads: authenticated.nquads,
            named_graphs: Vec::new(),
            salt: witness.dataset.salt,
        },
    })?;
    if inner.version != v3::VERSION || inner.provenance != crate::Provenance::HolderDeclaredOnly {
        return Err(Rejected("internal V3 provenance mismatch"));
    }
    Ok(Journal {
        version: VERSION,
        request_digest: request_digest(request)?,
        dataset_commitment: authenticated.commitment,
        provenance,
        result: inner.result,
    })
}

/// Checks request and authority binding after cryptographic receipt verification.
///
/// This verifies no proof. `VerifierAgreed` requires the journal commitment to
/// equal the request's authenticated commitment; both authorities require their
/// own provenance variant.
///
/// # Errors
/// Rejects other versions, substituted requests or policies, unequal
/// commitments and provenance from the other authority.
pub fn bind_journal(journal: &Journal, expected: &Request) -> Result<(), Rejected> {
    if journal.version != VERSION || journal.request_digest != request_digest(expected)? {
        return Err(Rejected("authenticated journal request mismatch"));
    }
    match &expected.authority {
        DatasetAuthority::VerifierAgreed { commitment } => {
            if *commitment != journal.dataset_commitment
                || journal.provenance != Provenance::VerifierAgreedAuthenticated
            {
                return Err(Rejected("authenticated journal dataset authority mismatch"));
            }
        }
        DatasetAuthority::HolderDeclared => {
            if journal.provenance != Provenance::HolderSelectedAuthenticated {
                return Err(Rejected("authenticated journal holder-selected provenance mismatch"));
            }
        }
    }
    Ok(())
}

struct Authenticated {
    commitment: [u8; 32],
    nquads: String,
}

struct Verified {
    document_hash: [u8; 32],
    config_hash: [u8; 32],
    quads: Vec<Quad>,
}

fn authenticate(dataset: &PrivateCredentials, policy: &Policy) -> Result<Authenticated, Rejected> {
    // [OPUS-5.5] Same V3 program ceilings as `validate_request`, before any source work.
    v3::validate_policy(&policy.evaluation)?;
    let table = checked_table(&policy.authorization)?;
    let policy_digest = policy_digest(policy, &table)?;
    if dataset.salt == [0; 32] {
        return Err(Rejected("authenticated dataset blinding rejected"));
    }
    admit_sizes(&dataset.credentials)?;
    let mut verified = dataset
        .credentials
        .iter()
        .map(|credential| verify(credential, &table))
        .collect::<Result<Vec<_>, _>>()?;
    verified.sort_unstable_by_key(|a| a.document_hash);
    if verified
        .windows(2)
        .any(|pair| pair[0].document_hash == pair[1].document_hash)
    {
        return Err(Rejected("duplicate authenticated credential document"));
    }
    let mut hash = Sha256::new();
    hash.update(b"sparq:proved-evaluator:authenticated-rdf:eddsa-rdfc-2022:scoped-union:v5\0");
    hash.update(policy_digest);
    hash.update(dataset.salt);
    hash.update((verified.len() as u32).to_le_bytes());
    let mut nquads = String::new();
    for (scope, credential) in verified.iter().enumerate() {
        hash.update(credential.document_hash);
        hash.update(credential.config_hash);
        map_scoped(&mut nquads, scope, &credential.quads)?;
    }
    Ok(Authenticated {
        commitment: hash.finalize().into(),
        nquads,
    })
}

// N-Quads admits at most one statement per EOL-separated line, so this counts
// an upper bound without parsing. Comment-only lines only overestimate.
fn statement_bound(text: &str) -> usize {
    text.split(['\n', '\r'])
        .filter(|line| !line.trim().is_empty())
        .count()
}

fn admit_sizes(credentials: &[SignedCredential]) -> Result<(), Rejected> {
    if credentials.is_empty() || credentials.len() > MAX_CREDENTIALS {
        return Err(Rejected("authenticated credential count must be 1 to 4"));
    }
    let (mut bytes, mut quads) = (0, 0);
    for credential in credentials {
        if credential.signature.len() != 64 {
            return Err(Rejected("Ed25519 signature must be 64 bytes"));
        }
        if credential.document.len() > MAX_DOCUMENT_BYTES
            || credential.proof_config.len() > MAX_PROOF_CONFIG_BYTES
        {
            return Err(Rejected("authenticated credential input capacity"));
        }
        let statements = statement_bound(&credential.document);
        if statements > MAX_DOCUMENT_QUADS
            || statement_bound(&credential.proof_config) > MAX_PROOF_CONFIG_QUADS
        {
            return Err(Rejected("authenticated credential input capacity"));
        }
        bytes += credential.document.len() + credential.proof_config.len();
        quads += statements;
    }
    if bytes > MAX_TOTAL_BYTES || quads > MAX_TOTAL_QUADS {
        return Err(Rejected("authenticated credential total capacity"));
    }
    Ok(())
}

fn parse(text: &str, max_quads: usize) -> Result<Vec<Quad>, Rejected> {
    let mut quads = Vec::new();
    for quad in oxttl::NQuadsParser::new().for_slice(text.as_bytes()) {
        if quads.len() >= max_quads {
            return Err(Rejected("authenticated RDF statement capacity"));
        }
        let quad = quad.map_err(|_| Rejected("authenticated RDF N-Quads parse rejected"))?;
        if !quad.graph_name.is_default_graph() {
            return Err(Rejected("only the default graph is admitted"));
        }
        match &quad.object {
            Term::Triple(_) => return Err(Rejected("RDF 1.2 triple terms are not admitted")),
            Term::Literal(literal) if literal.direction().is_some() => {
                return Err(Rejected("directional language strings are not admitted"));
            }
            _ => {}
        }
        quads.push(quad);
    }
    Ok(quads)
}

fn canonical(quads: &[Quad]) -> Result<String, Rejected> {
    canonicalize_quads_bounded_with::<Sha256>(quads, &RDFC_LIMITS)
        .map_err(|_| Rejected("bounded RDFC-1.0 canonicalization rejected"))
}

fn verify(credential: &SignedCredential, table: &[Entry<'_>]) -> Result<Verified, Rejected> {
    let document = canonical(&parse(&credential.document, MAX_DOCUMENT_QUADS)?)?;
    let config = canonical(&parse(&credential.proof_config, MAX_PROOF_CONFIG_QUADS)?)?;
    // Every later check reads the exact canonical bytes that are hashed and signed.
    let quads = parse(&document, MAX_DOCUMENT_QUADS)?;
    let method = proof_method(&parse(&config, MAX_PROOF_CONFIG_QUADS)?)?;
    let (entry, key) = table
        .binary_search_by(|(entry, _)| entry.verification_method.as_str().cmp(method.as_str()))
        .map(|index| &table[index])
        .map_err(|_| Rejected("verification method is not authorized"))?;
    check_document(&quads, &entry.issuer)?;
    let config_hash: [u8; 32] = Sha256::digest(config.as_bytes()).into();
    let document_hash: [u8; 32] = Sha256::digest(document.as_bytes()).into();
    let mut hash_data = [0; 64];
    hash_data[..32].copy_from_slice(&config_hash);
    hash_data[32..].copy_from_slice(&document_hash);
    let signature: [u8; 64] = credential
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| Rejected("Ed25519 signature must be 64 bytes"))?;
    key.verify_strict(&hash_data, &Signature::from_bytes(&signature))
        .map_err(|_| Rejected("Ed25519 signature verification failed"))?;
    Ok(Verified {
        document_hash,
        config_hash,
        quads,
    })
}

fn is_iri(term: &Term, iri: &str) -> bool {
    matches!(term, Term::NamedNode(node) if node.as_str() == iri)
}

fn proof_method(quads: &[Quad]) -> Result<String, Rejected> {
    let subject = quads.first().map(|quad| &quad.subject);
    if quads.iter().any(|quad| Some(&quad.subject) != subject) {
        return Err(Rejected("proof configuration must describe one proof node"));
    }
    let mut seen = [false; 5];
    let mut method = None;
    for quad in quads {
        let slot = match quad.predicate.as_str() {
            RDF_TYPE => 0,
            SEC_CRYPTOSUITE => 1,
            SEC_VERIFICATION_METHOD => 2,
            SEC_PROOF_PURPOSE => 3,
            DC_CREATED => 4,
            _ => return Err(Rejected("unsupported proof configuration option")),
        };
        if std::mem::replace(&mut seen[slot], true) {
            return Err(Rejected("duplicate proof configuration predicate"));
        }
        match (slot, &quad.object) {
            (0, object) if is_iri(object, SEC_DATA_INTEGRITY_PROOF) => {}
            (0, _) => return Err(Rejected("proof type must be DataIntegrityProof")),
            (1, Term::Literal(literal))
                if literal.value() == EDDSA_RDFC_2022
                    && literal.datatype().as_str() == SEC_CRYPTOSUITE_STRING => {}
            (1, _) => return Err(Rejected("cryptosuite must be the typed eddsa-rdfc-2022 value")),
            (2, Term::NamedNode(node)) => method = Some(node.as_str().to_owned()),
            (2, _) => return Err(Rejected("verification method must be an IRI")),
            (3, object) if is_iri(object, SEC_ASSERTION_METHOD) => {}
            (3, _) => return Err(Rejected("proof purpose must be assertionMethod")),
            (_, Term::Literal(literal)) if literal.datatype().as_str() == XSD_DATE_TIME => {
                match classify_created(literal.value()) {
                    Created::Supported => {}
                    Created::Unsupported => {
                        return Err(Rejected(
                            "created is outside the restricted whole-second UTC profile",
                        ));
                    }
                    Created::Malformed => {
                        return Err(Rejected("created is not a valid XSD 1.1 dateTimeStamp"));
                    }
                }
            }
            _ => return Err(Rejected("created must be an xsd:dateTime literal")),
        }
    }
    match method {
        Some(method) if seen[..4].iter().all(|present| *present) => Ok(method),
        _ => Err(Rejected("proof configuration is missing a required field")),
    }
}

fn check_document(quads: &[Quad], issuer: &str) -> Result<(), Rejected> {
    if quads
        .iter()
        .any(|quad| matches!(quad.predicate.as_str(), SEC_PROOF | SEC_PROOF_VALUE))
    {
        return Err(Rejected("embedded proofs are not admitted"));
    }
    let mut credentials = quads
        .iter()
        .filter(|quad| quad.predicate.as_str() == RDF_TYPE && is_iri(&quad.object, VC_TYPE))
        .map(|quad| &quad.subject);
    let (Some(credential), None) = (credentials.next(), credentials.next()) else {
        return Err(Rejected("document must have exactly one VerifiableCredential node"));
    };
    let mut issuers = quads
        .iter()
        .filter(|quad| quad.subject == *credential && quad.predicate.as_str() == VC_ISSUER);
    let (Some(stated), None) = (issuers.next(), issuers.next()) else {
        return Err(Rejected("credential must have exactly one issuer"));
    };
    if !is_iri(&stated.object, issuer) {
        return Err(Rejected("credential issuer does not match the authorized issuer"));
    }
    Ok(())
}

fn map_scoped(out: &mut String, scope: usize, quads: &[Quad]) -> Result<(), Rejected> {
    // Canonical labels start with `c14n`, so each scope prefix yields disjoint labels.
    let relabel = |node: &BlankNode| BlankNode::new_unchecked(format!("k{scope}_{}", node.as_str()));
    for quad in quads {
        let subject = match &quad.subject {
            NamedOrBlankNode::BlankNode(node) => NamedOrBlankNode::from(relabel(node)),
            subject => subject.clone(),
        };
        let object = match &quad.object {
            Term::BlankNode(node) => Term::from(relabel(node)),
            object => object.clone(),
        };
        writeln!(out, "{subject} {} {object} .", quad.predicate)
            .map_err(|_| Rejected("authenticated mapping serialization"))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Created {
    Supported,
    Unsupported,
    Malformed,
}

fn classify_created(lexical: &str) -> Created {
    match date_time_stamp(lexical) {
        Some(true) => Created::Supported,
        Some(false) => Created::Unsupported,
        None => Created::Malformed,
    }
}

fn two_digits(text: &str) -> Option<u32> {
    let &[high, low] = text.as_bytes() else {
        return None;
    };
    (high.is_ascii_digit() && low.is_ascii_digit())
        .then(|| u32::from(high - b'0') * 10 + u32::from(low - b'0'))
}

// Recognizes the XML Schema 1.1 dateTimeStamp lexical space, which requires a
// timezone and, unlike XSD 1.0, admits year 0000 (a leap year). Returns whether
// the value is in the restricted profile: positive four-digit year from 0001,
// hour below 24, whole seconds and `Z`.
fn date_time_stamp(lexical: &str) -> Option<bool> {
    let (negative, text) = lexical
        .strip_prefix('-')
        .map_or((false, lexical), |rest| (true, rest));
    let (year, rest) = text.split_once('-')?;
    let (month, rest) = rest.split_once('-')?;
    let (day, rest) = rest.split_once('T')?;
    let (time, zone) = rest.split_at(rest.find(['Z', '+', '-'])?);
    let mut fields = time.split(':');
    let (hour, minute, second) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some() {
        return None;
    }
    let (whole, fraction) = second
        .split_once('.')
        .map_or((second, None), |(whole, fraction)| (whole, Some(fraction)));
    if year.len() < 4
        || (year.len() > 4 && year.starts_with('0'))
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|digits| digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let (month, day) = (two_digits(month)?, two_digits(day)?);
    let (hour, minute, whole) = (two_digits(hour)?, two_digits(minute)?, two_digits(whole)?);
    // Divisibility by 4, 100 and 400 survives reduction modulo 400 and negation.
    let cycle = year
        .bytes()
        .fold(0, |acc, byte| (acc * 10 + u32::from(byte - b'0')) % 400);
    let leap = cycle == 0 || (cycle % 4 == 0 && cycle % 100 != 0);
    let days = match month {
        2 => 28 + u32::from(leap),
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    let zero_fraction = fraction.is_none_or(|digits| digits.bytes().all(|byte| byte == b'0'));
    let end_of_day = hour == 24 && minute == 0 && whole == 0 && zero_fraction;
    if day == 0 || day > days || !(end_of_day || (hour < 24 && minute < 60 && whole < 60)) {
        return None;
    }
    let utc = zone == "Z";
    if !utc {
        let (sign, offset_hour, colon, offset_minute) =
            (zone.as_bytes().first()?, zone.get(1..3)?, zone.as_bytes().get(3)?, zone.get(4..)?);
        let (offset_hour, offset_minute) = (two_digits(offset_hour)?, two_digits(offset_minute)?);
        if !matches!(*sign, b'+' | b'-')
            || *colon != b':'
            || offset_minute > 59
            || offset_hour > 14
            || (offset_hour == 14 && offset_minute != 0)
        {
            return None;
        }
    }
    Some(!negative && year.len() == 4 && year != "0000" && hour < 24 && fraction.is_none() && utc)
}

#[cfg(test)]
mod tests {
    use super::{Created, classify_created};

    #[test]
    fn created_profile_separates_supported_unsupported_and_malformed() {
        for value in [
            "2023-02-24T23:36:38Z",
            "2024-02-29T00:00:00Z",
            "2000-02-29T12:00:00Z",
            "0001-01-01T00:00:00Z",
            "9999-12-31T23:59:59Z",
        ] {
            assert_eq!(classify_created(value), Created::Supported, "{value}");
        }
        // Valid XSD 1.1 dateTimeStamp values outside the restricted profile,
        // including the XSD 1.1 year zero, which is a leap year.
        for value in [
            "0000-02-29T00:00:00Z",
            "-0001-01-01T00:00:00Z",
            "10000-01-01T00:00:00Z",
            "2023-02-24T23:36:38.5Z",
            "2023-02-24T23:36:38.000Z",
            "2023-02-24T24:00:00Z",
            "2023-02-24T23:36:38+01:00",
            "2023-02-24T23:36:38-00:00",
            "2023-02-24T23:36:38+14:00",
        ] {
            assert_eq!(classify_created(value), Created::Unsupported, "{value}");
        }
        for value in [
            "",
            "2023-02-24T23:36:38",
            "2023-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "2023-04-31T00:00:00Z",
            "2023-13-01T00:00:00Z",
            "2023-02-24T24:00:01Z",
            "2023-02-24T24:00:00.1Z",
            "2023-02-24T23:60:00Z",
            "2023-02-24T23:36:60Z",
            "2023-02-24T23:36:38.Z",
            "2023-02-24T23:36:38ZZ",
            "2023-02-24T23:36:38+14:01",
            "2023-02-24T23:36:38+1:00",
            "2023-02-24 23:36:38Z",
            "02023-01-01T00:00:00Z",
            "+2023-01-01T00:00:00Z",
            "123-01-01T00:00:00Z",
            "2023-1-01T00:00:00Z",
            "\u{ff12}023-02-24T23:36:38Z",
        ] {
            assert_eq!(classify_created(value), Created::Malformed, "{value}");
        }
    }
}

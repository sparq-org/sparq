//! Authenticate reconstructed public triples with verifier-owned signature statements.
//!
//! [GPT-6] Every BGP variable must be public. No hidden RDF preimage or numeric
//! witness relation is inferred from a shared nonce. See the native RDF reference.

use ark_bls12_381::{Bls12_381, Fr};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::rand::{CryptoRng, RngCore};
use bbs_plus::{
    prelude::{KeypairG2, PublicKeyG2, SignatureG1, SignatureParamsG1},
    proof::PoKOfSignatureG1Proof,
};
use blake2::{Blake2b512, Digest};
use dock_crypto_utils::hashing_utils::hash_to_field;
use oxrdf::{GraphName, NamedOrBlankNode, Term, Triple};
use proof_system::{
    prelude::{MetaStatements, Proof, ProofSpec, StatementProof, Witnesses},
    statement::{
        bbs_plus::{PoKBBSSignatureG1Prover, PoKBBSSignatureG1Verifier},
        Statements,
    },
    witness::PoKBBSSignatureG1,
};
use serde_json::json;
use spargebra::{
    algebra::GraphPattern,
    term::{NamedNodePattern, TermPattern, TriplePattern},
    Query, SparqlParser,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Fixed canonical triple-slot capacity, public in this versioned protocol.
pub const TRIPLE_SLOTS: usize = 16;
// Protocol and status occupy two distinct, always disclosed signature messages.
const MESSAGE_COUNT: usize = TRIPLE_SLOTS + 2;
// Canonical compressed BLS12-381 G2 public key; fixed in this wire version.
const PUBLIC_KEY_BYTES: usize = 96;
const MAX_ROLES: usize = 4;
const MAX_ROWS: usize = 8;
const MAX_PATTERNS: usize = 8;
const MAX_TERM_BYTES: usize = 1024;
const MAX_DOCUMENT_BYTES: usize = 32_768;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_STATUS_BYTES: usize = 131_072;
const MAX_STATEMENT_BYTES: usize = 8192;
const MAX_NONCES: usize = 4096;
const PROTOCOL: &[u8] = b"sparq/native-rdf/public-bgp/v1";
const WIRE_MAGIC: &[u8; 8] = b"SPNRDF01";
const HASH_DST: &[u8] = b"sparq-native-rdf-v1-bls12381-fr-blake2b512";

/// A typed rejection, with no private input or backend diagnostic attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An input exceeded a public protocol capacity.
    Capacity,
    /// A query uses an unsupported algebra shape or a hidden variable.
    QueryProfile,
    /// An RDF term, graph, or mapping violates the admitted RDF profile.
    RdfProfile,
    /// Public rows or slot references do not cover exactly the requested BGP.
    Support,
    /// A status reference is invalid, outside the accepted snapshot, or revoked.
    Status,
    /// Key generation, signing, or presentation construction failed.
    Construction,
    /// The bounded native BBS+ proof envelope is malformed.
    Decoding,
    /// The proof does not authenticate the independently expected statement.
    Verification,
    /// This verifier has already consumed the challenge.
    Replay,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native RDF rejection: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

/// A public binding maps variable names to canonical RDF term spellings.
///
/// Values use RDF term syntax (for example `<urn:alice>` or `"Alice"`), not
/// arbitrary JSON strings. Parsing and role validation occur before proof work.
pub type Mapping = BTreeMap<String, String>;

/// Public status reference authenticated as one signature message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReference {
    /// Absolute IRI of the verifier-accepted status list.
    pub list: String,
    /// Signed issuance epoch label, required to equal the verifier's expectation.
    ///
    /// A different accepted epoch requires a credential signed for that label;
    /// this API does not discover freshness or rebind an existing signature.
    pub epoch: u64,
    /// Raw-byte bit index: byte `index / 8`, mask `1 << (index % 8)`.
    ///
    /// Within each byte, the least significant bit comes first; a set bit is revoked.
    pub index: u64,
}

/// A verifier-owned snapshot and its exact expected signed reference.
#[derive(Clone, Debug)]
pub struct AcceptedStatus {
    /// The verifier's independently selected reference, not a holder trust hint.
    pub reference: StatusReference,
    /// Complete raw byte vector in the index convention above.
    ///
    /// This API does not decode a W3C encodedList or authenticate the list source.
    pub bits: Vec<u8>,
}

/// A trusted issuer key with a public, context-bound role identifier.
#[derive(Clone, Debug)]
pub struct TrustedIssuer {
    id: String,
    key: PublicKeyG2<Bls12_381>,
}

impl TrustedIssuer {
    /// Import an independently trusted public key in canonical compressed form.
    ///
    /// Decoding does not establish who owns the key; the caller supplies that trust.
    ///
    /// # Errors
    /// Rejects invalid identifiers, wrong length, invalid subgroup points and infinity.
    pub fn from_bytes(id: &str, encoded: &[u8]) -> Result<Self> {
        iri(id)?;
        if encoded.len() != PUBLIC_KEY_BYTES {
            return Err(Error::Decoding);
        }
        let mut bytes = encoded;
        let key = PublicKeyG2::<Bls12_381>::deserialize_compressed(&mut bytes)
            .map_err(|_| Error::Decoding)?;
        if !bytes.is_empty() || !key.is_valid() {
            return Err(Error::Decoding);
        }
        Ok(Self {
            id: id.to_owned(),
            key,
        })
    }

    /// Export this public key in canonical compressed form.
    ///
    /// # Errors
    /// Returns `Construction` if canonical serialization fails.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(PUBLIC_KEY_BYTES);
        self.key
            .serialize_compressed(&mut bytes)
            .map_err(|_| Error::Construction)?;
        Ok(bytes)
    }
}

/// An issuer's secret key; Debug intentionally redacts all key material.
pub struct Issuer {
    trusted: TrustedIssuer,
    keys: KeypairG2<Bls12_381>,
}
impl fmt::Debug for Issuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Issuer([REDACTED])")
    }
}
impl Issuer {
    /// Generate a fresh issuer key for the fixed experimental encoding.
    ///
    /// # Errors
    /// Returns `RdfProfile` for an invalid identifier and `Capacity` if oversized.
    pub fn generate<R: RngCore + CryptoRng>(rng: &mut R, id: &str) -> Result<Self> {
        iri(id)?;
        let keys = KeypairG2::generate_using_rng(rng, &parameters());
        if !keys.public_key.is_valid() {
            return Err(Error::Construction);
        }
        Ok(Self {
            trusted: TrustedIssuer {
                id: id.to_owned(),
                key: keys.public_key.clone(),
            },
            keys,
        })
    }

    /// Export the public key for an independent trust decision.
    pub fn public(&self) -> TrustedIssuer {
        self.trusted.clone()
    }
}

/// One verifier-owned issuer role and status requirement.
#[derive(Clone, Debug)]
pub struct RolePolicy {
    /// Trusted issuer chosen independently of the presentation.
    pub issuer: TrustedIssuer,
    /// Accepted list, epoch, index and complete snapshot.
    pub status: AcceptedStatus,
}

/// The verifier's exact query, released rows, policies and fresh challenge.
#[derive(Clone, Debug)]
pub struct Request {
    /// Exact SELECT DISTINCT query bytes, bound even when another spelling is equivalent.
    pub query: String,
    /// Nonempty distinct released mappings, in presentation order.
    pub rows: Vec<Mapping>,
    /// Each role must contribute at least one reconstructed triple.
    pub roles: Vec<RolePolicy>,
    /// Verifier-generated challenge; use a durable replay store in an application.
    pub nonce: [u8; 32],
}

/// A holder's signed RDF vector, including private undisclosed triples.
#[derive(Clone)]
pub struct Credential {
    lines: Vec<String>,
    messages: Vec<Fr>,
    signature: SignatureG1<Bls12_381>,
}
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential([REDACTED])")
    }
}

/// Public location of a signed supporting triple.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// Index into the independently accepted issuer-role list.
    pub role: usize,
    /// Zero-based canonical triple rank; protocol/status message offsets are internal.
    pub triple: usize,
}

/// Row-major, then BGP-pattern-major support locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Support {
    /// Exactly one signed slot for each reconstructed triple pattern in every row.
    pub rows: Vec<Vec<Slot>>,
}

/// A bounded native presentation; it carries no verifier trust specification.
#[derive(Clone, Debug)]
pub struct Presentation {
    /// Public slot indices leak placement/rank in the fixed canonical vector.
    pub support: Support,
    /// Versioned framing containing only canonical native BBS+ proof statements.
    pub proof: Vec<u8>,
}

/// A bounded process-local consumed-challenge store.
///
/// Dropping this store loses replay history. Production callers must retain
/// challenges durably; this research helper makes no cross-process replay claim.
#[derive(Default, Debug)]
pub struct ConsumedNonces(BTreeSet<[u8; 32]>);

fn parameters() -> SignatureParamsG1<Bls12_381> {
    SignatureParamsG1::new::<Blake2b512>(PROTOCOL, MESSAGE_COUNT as u32)
}

fn field(tag: &[u8], bytes: &[u8]) -> Fr {
    // Fixed tag length and payload length prevent concatenation ambiguity.
    let mut preimage = Vec::with_capacity(16 + tag.len() + bytes.len());
    preimage.extend_from_slice(&(tag.len() as u64).to_le_bytes());
    preimage.extend_from_slice(tag);
    preimage.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    preimage.extend_from_slice(bytes);
    hash_to_field::<Fr, Blake2b512>(HASH_DST, &preimage)
}

fn iri(value: &str) -> Result<()> {
    if value.len() > MAX_TERM_BYTES {
        return Err(Error::Capacity);
    }
    oxrdf::NamedNode::new(value)
        .map(|_| ())
        .map_err(|_| Error::RdfProfile)
}

fn status_bytes(reference: &StatusReference) -> Result<Vec<u8>> {
    iri(&reference.list)?;
    serde_json::to_vec(&json!([reference.list, reference.epoch, reference.index]))
        .map_err(|_| Error::Construction)
}

fn check_status(status: &AcceptedStatus) -> Result<()> {
    status_bytes(&status.reference)?;
    if status.bits.is_empty() || status.bits.len() > MAX_STATUS_BYTES {
        return Err(Error::Capacity);
    }
    let index = usize::try_from(status.reference.index).map_err(|_| Error::Status)?;
    let byte = status.bits.get(index / 8).ok_or(Error::Status)?;
    if byte & (1 << (index % 8)) != 0 {
        return Err(Error::Status);
    }
    Ok(())
}

fn validate_triple(triple: &Triple) -> Result<()> {
    let NamedOrBlankNode::NamedNode(subject) = &triple.subject else {
        return Err(Error::RdfProfile);
    };
    iri(subject.as_str())?;
    iri(triple.predicate.as_str())?;
    match &triple.object {
        Term::NamedNode(node) => iri(node.as_str())?,
        Term::Literal(value) if value.direction().is_none() => {
            if value.value().len() > MAX_TERM_BYTES {
                return Err(Error::Capacity);
            }
            iri(value.datatype().as_str())?;
        }
        _ => return Err(Error::RdfProfile),
    }
    Ok(())
}

fn canonical_lines(document: &str) -> Result<Vec<String>> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Capacity);
    }
    let quads = sparq_canon::parse_nquads(document).map_err(|_| Error::RdfProfile)?;
    if quads.is_empty() || quads.len() > TRIPLE_SLOTS * 4 {
        return Err(Error::Capacity);
    }
    let mut triples = Vec::with_capacity(quads.len());
    for quad in quads {
        if quad.graph_name != GraphName::DefaultGraph {
            return Err(Error::RdfProfile);
        }
        let triple = Triple::new(quad.subject, quad.predicate, quad.object);
        validate_triple(&triple)?;
        triples.push(triple);
    }
    let mut lines = sparq_canon::canonicalize_triples(&triples)
        .map_err(|_| Error::RdfProfile)?
        .lines;
    lines.sort();
    lines.dedup(); // This protocol signs an RDF graph set, never input multiplicities.
    if lines.len() > TRIPLE_SLOTS {
        return Err(Error::Capacity);
    }
    Ok(lines)
}

/// Sign a bounded blank-node-free RDF graph in the experimental native encoding.
///
/// # Errors
/// Rejects invalid RDF, named graphs, excess capacity, and signing failures.
/// The issuer authorizes the supplied status reference; this function does not
/// claim that a holder-provided list or epoch is independently trusted.
pub fn issue_rdf<R: RngCore + CryptoRng>(
    rng: &mut R,
    issuer: &Issuer,
    document: &str,
    status: &StatusReference,
) -> Result<Credential> {
    let lines = canonical_lines(document)?;
    let mut messages = vec![
        field(b"protocol", PROTOCOL),
        field(b"status", &status_bytes(status)?),
    ];
    for slot in 0..TRIPLE_SLOTS {
        messages.push(match lines.get(slot) {
            Some(line) => field(b"triple", line.as_bytes()),
            None => field(b"padding", &(slot as u64).to_le_bytes()),
        });
    }
    let signature = SignatureG1::new(rng, &messages, &issuer.keys.secret_key, &parameters())
        .map_err(|_| Error::Construction)?;
    signature
        .verify(&messages, issuer.trusted.key.clone(), parameters())
        .map_err(|_| Error::Construction)?;
    Ok(Credential {
        lines,
        messages,
        signature,
    })
}

fn mapped_term(value: &str) -> Result<Term> {
    if value.len() > MAX_TERM_BYTES {
        return Err(Error::Capacity);
    }
    let quads = sparq_canon::parse_nquads(&format!(
        "<urn:sparq:native:s> <urn:sparq:native:p> {value} ."
    ))
    .map_err(|_| Error::RdfProfile)?;
    if quads.len() != 1 {
        return Err(Error::RdfProfile);
    }
    let quad = &quads[0];
    let triple = Triple::new(
        quad.subject.clone(),
        quad.predicate.clone(),
        quad.object.clone(),
    );
    validate_triple(&triple)?;
    if quad.graph_name != GraphName::DefaultGraph || quad.object.to_string() != value {
        return Err(Error::RdfProfile);
    }
    Ok(quad.object.clone())
}

fn term(pattern: &TermPattern, row: &Mapping) -> Result<Term> {
    match pattern {
        TermPattern::NamedNode(node) => Ok(node.clone().into()),
        TermPattern::Literal(value) => Ok(value.clone().into()),
        TermPattern::Variable(variable) => {
            mapped_term(row.get(variable.as_str()).ok_or(Error::Support)?)
        }
        _ => Err(Error::QueryProfile),
    }
}

fn reconstructed(pattern: &TriplePattern, row: &Mapping) -> Result<String> {
    let Term::NamedNode(subject) = term(&pattern.subject, row)? else {
        return Err(Error::RdfProfile);
    };
    let predicate = match &pattern.predicate {
        NamedNodePattern::NamedNode(node) => node.clone(),
        NamedNodePattern::Variable(variable) => {
            let Term::NamedNode(node) =
                mapped_term(row.get(variable.as_str()).ok_or(Error::Support)?)?
            else {
                return Err(Error::RdfProfile);
            };
            node
        }
    };
    let triple = Triple::new(subject, predicate, term(&pattern.object, row)?);
    validate_triple(&triple)?;
    let lines = sparq_canon::canonicalize_triples(&[triple])
        .map_err(|_| Error::RdfProfile)?
        .lines;
    lines.into_iter().next().ok_or(Error::RdfProfile)
}

fn required_lines(request: &Request) -> Result<Vec<Vec<String>>> {
    if request.query.len() > MAX_QUERY_BYTES
        || request.rows.is_empty()
        || request.rows.len() > MAX_ROWS
        || request.roles.is_empty()
        || request.roles.len() > MAX_ROLES
    {
        return Err(Error::Capacity);
    }
    for role in &request.roles {
        iri(&role.issuer.id)?;
        check_status(&role.status)?;
    }
    let Query::Select {
        dataset: None,
        pattern: GraphPattern::Distinct { inner },
        ..
    } = SparqlParser::new()
        .parse_query(&request.query)
        .map_err(|_| Error::QueryProfile)?
    else {
        return Err(Error::QueryProfile);
    };
    let GraphPattern::Project { inner, variables } = *inner else {
        return Err(Error::QueryProfile);
    };
    let GraphPattern::Bgp { patterns } = *inner else {
        return Err(Error::QueryProfile);
    };
    if patterns.is_empty()
        || patterns.len() > MAX_PATTERNS
        || variables.is_empty()
        || variables.len() > MAX_PATTERNS * 3
    {
        return Err(Error::QueryProfile);
    }
    let projected: BTreeSet<_> = variables.iter().map(|v| v.as_str()).collect();
    if projected.len() != variables.len() {
        return Err(Error::QueryProfile);
    }
    let mut used = BTreeSet::new();
    for pattern in &patterns {
        for endpoint in [&pattern.subject, &pattern.object] {
            match endpoint {
                TermPattern::Variable(v) => {
                    used.insert(v.as_str());
                }
                TermPattern::NamedNode(_) | TermPattern::Literal(_) => {}
                _ => return Err(Error::QueryProfile),
            }
        }
        if let NamedNodePattern::Variable(v) = &pattern.predicate {
            used.insert(v.as_str());
        }
    }
    if projected != used {
        return Err(Error::QueryProfile);
    }
    let mut distinct = BTreeSet::new();
    let mut result = Vec::with_capacity(request.rows.len());
    for row in &request.rows {
        if row.len() != projected.len()
            || row.keys().map(String::as_str).collect::<BTreeSet<_>>() != projected
        {
            return Err(Error::Support);
        }
        if row.values().any(|value| value.len() > MAX_TERM_BYTES) {
            return Err(Error::Capacity);
        }
        if !distinct.insert(row.clone()) {
            return Err(Error::Support);
        }
        result.push(
            patterns
                .iter()
                .map(|p| reconstructed(p, row))
                .collect::<Result<_>>()?,
        );
    }
    Ok(result)
}

type Disclosures = Vec<BTreeMap<usize, Fr>>;

fn relation(request: &Request, support: &Support) -> Result<(Disclosures, Vec<u8>)> {
    let lines = required_lines(request)?;
    if support.rows.len() != lines.len() {
        return Err(Error::Support);
    }
    let mut disclosed = request
        .roles
        .iter()
        .map(|r| {
            Ok(BTreeMap::from([
                (0, field(b"protocol", PROTOCOL)),
                (1, field(b"status", &status_bytes(&r.status.reference)?)),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    for (references, triples) in support.rows.iter().zip(&lines) {
        if references.len() != triples.len() {
            return Err(Error::Support);
        }
        for (slot, triple) in references.iter().zip(triples) {
            if slot.triple >= TRIPLE_SLOTS {
                return Err(Error::Support);
            }
            let messages = disclosed.get_mut(slot.role).ok_or(Error::Support)?;
            let message = field(b"triple", triple.as_bytes());
            if let Some(previous) = messages.insert(slot.triple + 2, message) {
                if previous != message {
                    return Err(Error::Support);
                }
            }
        }
    }
    if disclosed.iter().any(|d| d.len() <= 2) {
        return Err(Error::Support);
    }
    let roles = request
        .roles
        .iter()
        .map(|role| {
            let mut key = Vec::new();
            role.issuer
                .key
                .serialize_compressed(&mut key)
                .map_err(|_| Error::Construction)?;
            Ok(json!([
                role.issuer.id,
                key,
                role.status.reference.list,
                role.status.reference.epoch,
                role.status.reference.index,
                Blake2b512::digest(&role.status.bits).to_vec()
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let slots: Vec<Vec<_>> = support
        .rows
        .iter()
        .map(|r| r.iter().map(|s| [s.role, s.triple]).collect())
        .collect();
    let context = serde_json::to_vec(&json!({"protocol": PROTOCOL, "query": request.query, "rows": request.rows, "roles": roles, "support": slots, "semantics": "nonempty-distinct-successful-support"})).map_err(|_| Error::Construction)?;
    Ok((disclosed, context))
}

fn specification(
    request: &Request,
    support: &Support,
    prover: bool,
) -> Result<ProofSpec<Bls12_381>> {
    let (disclosed, context) = relation(request, support)?;
    let mut statements = Statements::new();
    for (role, messages) in request.roles.iter().zip(disclosed) {
        if prover {
            statements.add(PoKBBSSignatureG1Prover::new_statement_from_params(
                parameters(),
                messages,
            ));
        } else {
            statements.add(PoKBBSSignatureG1Verifier::new_statement_from_params(
                parameters(),
                role.issuer.key.clone(),
                messages,
            ));
        }
    }
    // Every needed preimage is public: there is no residual circuit or hidden
    // cross-statement equality claim in this version.
    let spec = ProofSpec::new(statements, MetaStatements::new(), vec![], Some(context));
    spec.validate().map_err(|_| Error::Construction)?;
    Ok(spec)
}

/// Export the exact public proof context derived from an accepted request.
///
/// This includes trusted key bytes, snapshot digests and disclosed slot indices.
/// The verifier nonce is a separate proof input. No private credential is read.
///
/// # Errors
/// Rejects the same profile, status, capacity and support violations as verification.
pub fn public_context(request: &Request, support: &Support) -> Result<Vec<u8>> {
    relation(request, support).map(|(_, context)| context)
}

fn encode(proof: Proof<Bls12_381>) -> Result<Vec<u8>> {
    if proof.aggregated_groth16.is_some()
        || proof.aggregated_legogroth16.is_some()
        || proof.statement_proofs.len() > MAX_ROLES
    {
        return Err(Error::Construction);
    }
    let mut output = WIRE_MAGIC.to_vec();
    output.push(proof.statement_proofs.len() as u8);
    for statement in proof.statement_proofs {
        let StatementProof::PoKBBSSignatureG1(statement) = statement else {
            return Err(Error::Construction);
        };
        let mut bytes = Vec::new();
        statement
            .serialize_compressed(&mut bytes)
            .map_err(|_| Error::Construction)?;
        if bytes.len() > MAX_STATEMENT_BYTES {
            return Err(Error::Capacity);
        }
        output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        output.extend_from_slice(&bytes);
    }
    Ok(output)
}

fn decode(encoded: &[u8], count: usize) -> Result<Proof<Bls12_381>> {
    if encoded.len() > 9 + MAX_ROLES * (4 + MAX_STATEMENT_BYTES)
        || encoded.get(..8) != Some(WIRE_MAGIC.as_slice())
        || encoded.get(8).copied().map(usize::from) != Some(count)
    {
        return Err(Error::Decoding);
    }
    let mut input = &encoded[9..];
    let mut statements = Vec::with_capacity(count);
    for _ in 0..count {
        let length = u32::from_le_bytes(
            input
                .get(..4)
                .ok_or(Error::Decoding)?
                .try_into()
                .map_err(|_| Error::Decoding)?,
        ) as usize;
        input = &input[4..];
        if length > MAX_STATEMENT_BYTES {
            return Err(Error::Decoding);
        }
        let mut bytes = input.get(..length).ok_or(Error::Decoding)?;
        // Decode only BBS+ statements. No generic proof-system variant or
        // aggregation payload can select an unrelated deserializer.
        let statement = PoKOfSignatureG1Proof::<Bls12_381>::deserialize_compressed(&mut bytes)
            .map_err(|_| Error::Decoding)?;
        if !bytes.is_empty() {
            return Err(Error::Decoding);
        }
        statements.push(StatementProof::PoKBBSSignatureG1(statement));
        input = &input[length..];
    }
    if !input.is_empty() {
        return Err(Error::Decoding);
    }
    Ok(Proof {
        statement_proofs: statements,
        aggregated_groth16: None,
        aggregated_legogroth16: None,
    })
}

// [GPT-6] An occurrence can have support in several issuer roles. Track every
// reachable role set so first-match choices cannot starve a required role. The
// request bounds this to 16 masks and 64 occurrences; traversal is deterministic.
fn covering_slots(candidates: &[Vec<Slot>], roles: usize) -> Result<Vec<Slot>> {
    if roles == 0 || roles > MAX_ROLES || candidates.len() > MAX_ROWS * MAX_PATTERNS {
        return Err(Error::Capacity);
    }
    let states = 1 << roles;
    let mut reachable: Vec<Option<Vec<Slot>>> = vec![None; states];
    reachable[0] = Some(Vec::new());
    for choices in candidates {
        let mut next = vec![None; states];
        for (mask, prefix) in reachable.iter().enumerate() {
            let Some(prefix) = prefix else { continue };
            for slot in choices {
                if slot.role >= roles || slot.triple >= TRIPLE_SLOTS {
                    return Err(Error::Support);
                }
                let target = mask | (1 << slot.role);
                if next[target].is_none() {
                    let mut allocation = prefix.clone();
                    allocation.push(*slot);
                    next[target] = Some(allocation);
                }
            }
        }
        reachable = next;
    }
    reachable[states - 1].take().ok_or(Error::Support)
}

// [GPT-6] Shared honest construction path; native preparation creates no proof.
fn prepare_relation(request: &Request, credentials: &[Credential]) -> Result<(Support, Disclosures)> {
    if credentials.len() != request.roles.len() {
        return Err(Error::Support);
    }
    let required = required_lines(request)?;
    let candidates = required
        .iter()
        .flatten()
        .map(|line| {
            credentials
                .iter()
                .enumerate()
                .filter_map(|(role, credential)| {
                    credential.lines.iter().position(|s| s == line)
                        .map(|triple| Slot { role, triple })
                })
                .collect()
        })
        .collect::<Vec<_>>();
    let selected = covering_slots(&candidates, request.roles.len())?;
    let mut offset = 0;
    let support = Support {
        rows: required.iter().map(|row| {
            let slots = selected[offset..offset + row.len()].to_vec();
            offset += row.len();
            slots
        }).collect(),
    };
    let (disclosed, _) = relation(request, &support)?;
    for (credential, public) in credentials.iter().zip(&disclosed) {
        if public.iter().any(|(i, v)| credential.messages.get(*i) != Some(v)) {
            return Err(Error::Support);
        }
    }
    Ok((support, disclosed))
}

/// Prepare the exact public support used by the honest native prover.
///
/// This validates the same query, policy, allocation and disclosed messages as
/// proof construction. It does not generate or verify a zero-knowledge proof.
///
/// # Errors
/// Rejects unsupported requests, missing role-covering support, mismatched signed
/// public messages, invalid status policy and public capacity violations.
pub fn prepare_public_bgp(request: &Request, credentials: &[Credential]) -> Result<Support> {
    prepare_relation(request, credentials).map(|(support, _)| support)
}

/// Find public support and prove possession of the corresponding issuer signatures.
///
/// # Errors
/// Rejects unsupported requests, missing matching triples, invalid policy,
/// excess capacity, or signature-proof construction failures.
pub fn prove_public_bgp<R: RngCore + CryptoRng>(
    rng: &mut R,
    request: &Request,
    credentials: &[Credential],
) -> Result<Presentation> {
    let (support, disclosed) = prepare_relation(request, credentials)?;
    let mut witnesses = Witnesses::new();
    for (credential, public) in credentials.iter().zip(disclosed) {
        witnesses.add(PoKBBSSignatureG1::new_as_witness(
            credential.signature.clone(),
            credential
                .messages
                .iter()
                .enumerate()
                .filter(|(i, _)| !public.contains_key(i))
                .map(|(i, m)| (i, *m))
                .collect(),
        ));
    }
    let (proof, _) = Proof::new::<R, Blake2b512>(
        rng,
        specification(request, &support, true)?,
        witnesses,
        Some(request.nonce.to_vec()),
        Default::default(),
    )
    .map_err(|_| Error::Construction)?;
    Ok(Presentation {
        support,
        proof: encode(proof)?,
    })
}

/// Verify against independent expectations and consume the challenge after success.
///
/// # Errors
/// Rejects malformed or weaker proofs, mismatched RDF/query/result/policy,
/// revoked status, reused challenges, or a full process-local replay store.
pub fn verify_public_bgp<R: RngCore + CryptoRng>(
    rng: &mut R,
    request: &Request,
    presentation: &Presentation,
    consumed: &mut ConsumedNonces,
) -> Result<()> {
    if consumed.0.contains(&request.nonce) {
        return Err(Error::Replay);
    }
    if consumed.0.len() >= MAX_NONCES {
        return Err(Error::Capacity);
    }
    let spec = specification(request, &presentation.support, false)?;
    let proof = decode(&presentation.proof, request.roles.len())?;
    proof
        .verify::<R, Blake2b512>(rng, spec, Some(request.nonce.to_vec()), Default::default())
        .map_err(|_| Error::Verification)?;
    consumed.0.insert(request.nonce);
    Ok(())
}

#[cfg(test)]
mod tests;

/// Local synthetic-fixture controls, excluded without the explicit test feature.
#[cfg(feature = "native-binding")]
pub mod binding_tests;

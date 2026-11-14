//! Credential-suite adapter. Authentication and term encodings have one source.

use crate::{
    interp::{Cell, CredentialCells, Dataset, EncodeTerm, Word},
    Error,
};
use oxrdf::{GraphName, Quad, Term};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::{poseidon as suite, Fr};
use sparq_zk::{
    field::{field_from_hex_str_strict, field_to_be_bytes_32},
    sig,
};

/// The initial issuer registry uses the shared membership relation.
pub const ISSUER_DEPTH: u32 = 4;
/// Maximum object lexical backing in the extended opening profile.
pub const LEXICAL_CAPACITY: usize = 160;
/// Maximum resource lexical backing in the extended opening profile.
pub const RESOURCE_CAPACITY: usize = 128;
/// Scalar lexical backing used by value-key checks.
pub const SCALAR_CAPACITY: usize = 32;
/// Datatype and language backing in the bounded credential profile.
pub const DATATYPE_CAPACITY: usize = 41;
pub const LANGUAGE_CAPACITY: usize = 1;
/// Creation and verification-method witness backing capacities.
pub const CREATION_CAPACITY: usize = 32;
pub const METHOD_CAPACITY: usize = 128;
/// Reserved method-generated issuer assertion predicate.
pub const VERIFIED_ISSUER: &str = "https://w3id.org/sparq/vcq#verifiedIssuer";
pub(crate) const DATASET_DOMAIN: &[u8; 16] = b"NQ/SUITE/V1\0\0\0\0\0";

/// Fixed suite encoding for request parameters and released RDF terms.
pub struct SuiteEncoder;
impl EncodeTerm for SuiteEncoder {
    fn encode(&self, term: &Term) -> Result<Cell, Error> {
        Ok(cell(suite::term_cell(term, 0).map_err(suite_error)?))
    }
}
fn cell(c: suite::TermCell) -> Cell {
    Cell {
        bound: c.bound,
        kind: c.kind,
        enc: table_identity(&c),
        key_class: c.key_class,
        key: suite::field_bytes(c.key),
        dt: suite::field_bytes(c.dt),
        lang: suite::field_bytes(c.lang),
        scope: c.credential,
    }
}
// One conversion is shared by source cells, request parameters and released RDF.
fn table_identity(cell: &suite::TermCell) -> Word {
    suite::field_bytes(cell.identity)
}
fn suite_error(e: sparq_merkle_suite::Error) -> Error {
    Error::Invalid(e.to_string())
}

/// Complete suite credential and its private issuer-set membership witness.
#[derive(Clone, Debug)]
pub struct Credential {
    pub credential: suite::Credential,
    pub issuer: sig::PublicKey,
    pub issuer_index: u64,
    pub issuer_siblings: Vec<Fr>,
}
impl Credential {
    /// Enforce verifier-selected opening bounds without selecting a profile from data.
    pub fn cells_with_profile(
        &self,
        credential_index: u32,
        profile: crate::capacity::OpeningProfile,
    ) -> Result<CredentialCells, Error> {
        let cells = self.cells(credential_index)?;
        let limits = profile.limits();
        for quad in self.quads()? {
            if lexical_len(&quad.subject.into())? > limits.resource as usize
                || quad.predicate.as_str().len() > limits.resource as usize
                || lexical_len(&quad.object)? > limits.object as usize
            {
                return Err(Error::Capacity("opening profile lexical bytes".into()));
            }
        }
        let metadata = self.metadata()?;
        if metadata
            .created
            .as_ref()
            .map(lexical_len)
            .transpose()?
            .is_some_and(|n| n > limits.created as usize)
            || metadata
                .verification_method
                .as_ref()
                .map(lexical_len)
                .transpose()?
                .is_some_and(|n| n > limits.method as usize)
        {
            return Err(Error::Capacity("opening profile metadata bytes".into()));
        }
        Ok(cells)
    }
    /// Verify before generating a witness; the circuit independently checks it.
    pub fn cells(&self, credential_index: u32) -> Result<CredentialCells, Error> {
        if !suite::verify(&self.credential, &self.issuer) {
            return Err(Error::Invalid(
                "credential suite signature or opening".into(),
            ));
        }
        if self.issuer_siblings.len() != ISSUER_DEPTH as usize
            || self.issuer_index >= (1 << ISSUER_DEPTH)
        {
            return Err(Error::Invalid("issuer membership shape".into()));
        }
        let scope = credential_index
            .checked_add(1)
            .ok_or_else(|| Error::Capacity("credential scope".into()))?;
        let mut quads = vec![];
        for quad in self.quads()? {
            if quad.predicate.as_str() == VERIFIED_ISSUER {
                return Err(Error::Excluded(
                    "credential verifiedIssuer assertion".into(),
                ));
            }
            if quad.graph_name != GraphName::DefaultGraph {
                return Err(Error::Unsupported("credential named graph".into()));
            }
            let terms = [
                Term::from(quad.subject.clone()),
                Term::NamedNode(quad.predicate.clone()),
                quad.object.clone(),
            ];
            for (position, term) in terms.iter().enumerate() {
                let lexical = match term {
                    Term::NamedNode(n) => n.as_str(),
                    Term::BlankNode(n) => n.as_str(),
                    Term::Literal(l) => l.value(),
                    Term::Triple(_) => return Err(Error::Excluded("triple term".into())),
                };
                let capacity = if position < 2 {
                    RESOURCE_CAPACITY
                } else {
                    LEXICAL_CAPACITY
                };
                if lexical.len() > capacity {
                    return Err(Error::Capacity("opened lexical bytes".into()));
                }
            }
            let encoded = suite::quad_cells(&quad, scope).map_err(suite_error)?;
            quads.push([
                cell(encoded[0].clone()),
                cell(encoded[1].clone()),
                cell(encoded[2].clone()),
                Cell::default(),
            ]);
        }
        Ok(CredentialCells { quads })
    }
    pub(crate) fn quads(&self) -> Result<Vec<Quad>, Error> {
        oxttl::NQuadsParser::new()
            .for_slice(self.credential.document.as_bytes())
            .map(|q| q.map_err(|e| Error::Invalid(e.to_string())))
            .collect()
    }
    pub fn metadata(&self) -> Result<sparq_merkle_suite::metadata::Metadata, Error> {
        sparq_merkle_suite::metadata::from_configuration(
            &self.credential.proof_config,
            &self.issuer,
        )
        .map_err(suite_error)
    }
    pub fn signed_message(&self) -> Result<Fr, Error> {
        let config: [u8; 32] = Sha256::digest(self.credential.proof_config.as_bytes()).into();
        Ok(suite::signed_message(
            self.credential.salt,
            self.credential.count,
            self.credential.root,
            &config,
            self.metadata()?.digest,
        ))
    }
    pub fn issuer_root(&self) -> Result<Word, Error> {
        let mut node =
            sig::key_set_leaf(&self.issuer).ok_or_else(|| Error::Invalid("issuer key".into()))?;
        if self.issuer_siblings.len() != ISSUER_DEPTH as usize
            || self.issuer_index >= (1 << ISSUER_DEPTH)
        {
            return Err(Error::Invalid("issuer membership shape".into()));
        }
        for (i, sibling) in self.issuer_siblings.iter().enumerate() {
            node = if (self.issuer_index >> i) & 1 == 0 {
                sparq_zk::poseidon2::hash(&[node, *sibling])
            } else {
                sparq_zk::poseidon2::hash(&[*sibling, node])
            };
        }
        Ok(field_to_be_bytes_32(&node))
    }
}

fn lexical_len(term: &Term) -> Result<usize, Error> {
    match term {
        Term::NamedNode(n) => Ok(n.as_str().len()),
        Term::BlankNode(n) => Ok(n.as_str().len()),
        Term::Literal(l) => Ok(l.value().len()),
        Term::Triple(_) => Err(Error::Excluded("triple term".into())),
    }
}

/// Presentation-local graph identity from the suite's shared constructor.
pub fn credential_graph(request_digest: Word, index: u32) -> Result<Cell, Error> {
    let request = [
        u128::from_be_bytes(request_digest[..16].try_into().unwrap()),
        u128::from_be_bytes(request_digest[16..].try_into().unwrap()),
    ];
    Ok(cell(
        sparq_merkle_suite::metadata::graph_name(request, index).map_err(suite_error)?,
    ))
}

/// Assemble credential graph views and default issuer assertions.
/// The envelope supplies issuer cells authenticated against the stored trust set.
pub fn assemble_dataset(
    credentials: &[CredentialCells],
    issuers: &[Cell],
    request_digest: Word,
) -> Result<Dataset, Error> {
    if credentials.len() != issuers.len() {
        return Err(Error::Invalid("verified issuer count".into()));
    }
    let predicate =
        SuiteEncoder.encode(&oxrdf::NamedNode::new_unchecked(VERIFIED_ISSUER).into())?;
    let mut quads = vec![];
    let mut counts = vec![];
    for (i, (credential, issuer)) in credentials.iter().zip(issuers).enumerate() {
        issuer.validate()?;
        if !issuer.bound || issuer.kind != 1 {
            return Err(Error::Invalid("verified issuer is an IRI".into()));
        }
        let graph = credential_graph(
            request_digest,
            u32::try_from(i).map_err(|_| Error::Capacity("credential scope".into()))?,
        )?;
        counts.push(credential.quads.len());
        for quad in &credential.quads {
            for cell in quad {
                cell.validate()?;
            }
            if quad[3].bound {
                return Err(Error::Unsupported("credential named graph".into()));
            }
            if quad[1].enc == predicate.enc {
                return Err(Error::Excluded(
                    "credential verifiedIssuer assertion".into(),
                ));
            }
            if quad[..3].iter().any(|c| !c.bound) || quad[0].kind == 3 || quad[1].kind != 1 {
                return Err(Error::Invalid("RDF triple positions".into()));
            }
            if quad[..3]
                .iter()
                .any(|c| c.kind == 2 && c.scope != graph.scope)
            {
                return Err(Error::Invalid("credential blank-node scope".into()));
            }
            quads.push([quad[0], quad[1], quad[2], graph]);
        }
        quads.push([graph, predicate, *issuer, Cell::default()]);
    }
    Ok(Dataset { quads, counts })
}

/// Assemble graph views from signed credentials and verifier-selected trust members.
/// This host helper checks issuer identity, verification method and signing key.
/// Other envelope conditions belong to their respective policy consumers.
pub fn assemble_issuer_dataset(
    credentials: &[Credential],
    trust_members: &[crate::envelope_policy::TrustMember],
    request_digest: Word,
) -> Result<Dataset, Error> {
    use crate::envelope_policy::{canonical_set_root, trust_member_digest};
    let members = trust_members
        .iter()
        .map(trust_member_digest)
        .collect::<Result<Vec<_>, _>>()?;
    canonical_set_root(&members)?;
    let mut opened = Vec::with_capacity(credentials.len());
    let mut issuers = Vec::with_capacity(credentials.len());
    for (index, credential) in credentials.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| Error::Capacity("credential scope".into()))?;
        opened.push(credential.cells(index)?);
        let quads = credential.quads()?;
        let roots: Vec<_> = quads
            .iter()
            .filter(|q| {
                q.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                    && q.object
                        == Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                            "https://www.w3.org/2018/credentials#VerifiableCredential",
                        ))
            })
            .map(|q| &q.subject)
            .collect();
        let [subject] = roots.as_slice() else {
            return Err(Error::Invalid("credential root cardinality".into()));
        };
        let claims: Vec<_> = quads
            .iter()
            .filter(|q| {
                &q.subject == *subject
                    && q.predicate.as_str() == "https://www.w3.org/2018/credentials#issuer"
            })
            .map(|q| &q.object)
            .collect();
        let [Term::NamedNode(issuer)] = claims.as_slice() else {
            return Err(Error::Invalid(
                "credential issuer cardinality or kind".into(),
            ));
        };
        let Some(Term::NamedNode(method)) = credential.metadata()?.verification_method else {
            return Err(Error::Invalid("credential verification method".into()));
        };
        if !trust_members.iter().any(|member| {
            member.issuer == *issuer
                && member.verification_method == method
                && member.key == credential.issuer
        }) {
            return Err(Error::Invalid("issuer authorization set".into()));
        }
        issuers.push(SuiteEncoder.encode(&Term::NamedNode(issuer.clone()))?);
    }
    assemble_dataset(&opened, &issuers, request_digest)
}

/// Salted commitment to the ordered, authenticated suite messages.
pub fn dataset_commitment(credentials: &[Credential], salt: Word) -> Result<Word, Error> {
    let mut bytes = DATASET_DOMAIN.to_vec();
    bytes.extend(salt);
    for credential in credentials {
        bytes.extend(suite::field_bytes(credential.signed_message()?));
    }
    Ok(Sha256::digest(bytes).into())
}

pub(crate) fn field(word: Word) -> Result<Fr, Error> {
    let text = word.iter().map(|b| format!("{b:02x}")).collect::<String>();
    field_from_hex_str_strict(&text).ok_or_else(|| Error::Invalid("noncanonical field word".into()))
}

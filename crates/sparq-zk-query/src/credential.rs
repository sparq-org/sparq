//! Credential-suite adapter. Authentication and term encodings have one source.

use crate::{
    interp::{Cell, CredentialCells, EncodeTerm, Word},
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
/// Opened lexical capacity of the initial query profile.
pub const LEXICAL_CAPACITY: usize = 32;
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
        enc: suite::field_bytes(c.enc),
        key_class: c.key_class,
        key: suite::field_bytes(c.key),
        dt: suite::field_bytes(c.dt),
        lang: suite::field_bytes(c.lang),
        scope: c.credential,
    }
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
        let mut quads = vec![];
        for quad in self.quads()? {
            if quad.graph_name != GraphName::DefaultGraph {
                return Err(Error::Unsupported("credential named graph".into()));
            }
            let terms = [
                Term::from(quad.subject.clone()),
                Term::NamedNode(quad.predicate.clone()),
                quad.object.clone(),
            ];
            for term in &terms {
                let lexical = match term {
                    Term::NamedNode(n) => n.as_str(),
                    Term::BlankNode(n) => n.as_str(),
                    Term::Literal(l) => l.value(),
                    Term::Triple(_) => return Err(Error::Excluded("triple term".into())),
                };
                if lexical.len() > LEXICAL_CAPACITY {
                    return Err(Error::Capacity("opened lexical bytes".into()));
                }
                if let Term::Literal(l) = term {
                    if !matches!(
                        l.datatype().as_str(),
                        "http://www.w3.org/2001/XMLSchema#integer"
                            | "http://www.w3.org/2001/XMLSchema#decimal"
                            | "http://www.w3.org/2001/XMLSchema#date"
                            | "http://www.w3.org/2001/XMLSchema#dateTime"
                            | "http://www.w3.org/2001/XMLSchema#boolean"
                            | "http://www.w3.org/2001/XMLSchema#string"
                    ) {
                        return Err(Error::Unsupported("N0 credential literal datatype".into()));
                    }
                }
            }
            let encoded = suite::quad_cells(&quad, credential_index + 1).map_err(suite_error)?;
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
    pub fn signed_message(&self) -> Fr {
        let config: [u8; 32] = Sha256::digest(self.credential.proof_config.as_bytes()).into();
        suite::signed_message(
            self.credential.salt,
            self.credential.count,
            self.credential.root,
            &config,
        )
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

/// Salted commitment to the ordered, authenticated suite messages.
pub fn dataset_commitment(credentials: &[Credential], salt: Word) -> Word {
    let mut bytes = DATASET_DOMAIN.to_vec();
    bytes.extend(salt);
    for credential in credentials {
        bytes.extend(suite::field_bytes(credential.signed_message()));
    }
    Sha256::digest(bytes).into()
}

pub(crate) fn field(word: Word) -> Result<Fr, Error> {
    let text = word.iter().map(|b| format!("{b:02x}")).collect::<String>();
    field_from_hex_str_strict(&text).ok_or_else(|| Error::Invalid("noncanonical field word".into()))
}

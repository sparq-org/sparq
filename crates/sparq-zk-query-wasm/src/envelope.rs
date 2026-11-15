//! Holder keys, status snapshots and private envelope evidence for the Node package.
//! Research grade and not externally audited. Public policy always comes from the
//! stored request; this module only assembles the holder's private evidence.
use crate::codec::{err, hex, word, Result};
use oxrdf::{Literal, NamedNode, Term};
use serde::{Deserialize, Serialize};
use sparq_merkle_suite::{poseidon as suite, Fr};
use sparq_zk::{field::field_to_be_bytes_32, poseidon2, sig};
use sparq_zk_query::{
    envelope_policy::{self, PolicyInput},
    interp::Word,
    witness::{CredentialEvidence, EnvelopeWitness, HolderWitness, StatusWitness},
};

const CREDENTIAL_STATUS: &str = "https://www.w3.org/2018/credentials#credentialStatus";
const STATUS_LIST: &str = "https://www.w3.org/ns/credentials/status#statusListCredential";
const STATUS_PURPOSE: &str = "https://www.w3.org/ns/credentials/status#statusPurpose";
const STATUS_INDEX: &str = "https://www.w3.org/ns/credentials/status#statusListIndex";
/// The compiled status-path bound.
const MAX_STATUS_DEPTH: u32 = 32;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HolderKey {
    pub secret_key_hex: String,
    pub public_key: String,
    pub multikey: String,
}
fn holder_key(secret: [u8; 32]) -> Result<HolderKey> {
    let key = suite::keypair_from_seed(&secret).public_key();
    Ok(HolderKey {
        secret_key_hex: hex(&secret),
        public_key: sig::public_key_to_hex(&key),
        multikey: sparq_merkle_suite::metadata::multikey(&key).map_err(err)?,
    })
}
pub(crate) fn holder_keygen() -> Result<HolderKey> {
    let mut secret = [0; 32];
    getrandom_latest::fill(&mut secret).map_err(err)?;
    holder_key(secret)
}
/// Deterministic holder keys for tests only.
pub(crate) fn test_holder_keygen(seed: &str) -> Result<HolderKey> {
    holder_key(word(seed)?)
}

/// One status list as published by its issuer: list IRI, purpose and bits, one
/// character per entry ("0" valid, "1" set).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StatusList {
    pub list: String,
    pub purpose: String,
    pub bits: String,
}
struct Tree {
    bits: Vec<bool>,
    depth: u32,
    /// levels[0] are the leaves; zero pads past the list length.
    levels: Vec<Vec<Fr>>,
    list: Word,
    purpose: Word,
}
impl StatusList {
    fn tree(&self) -> Result<Tree> {
        let bits = self
            .bits
            .chars()
            .map(|c| match c {
                '0' => Ok(false),
                '1' => Ok(true),
                _ => Err("status bits must be 0 or 1".to_string()),
            })
            .collect::<Result<Vec<_>>>()?;
        if bits.is_empty() {
            return Err("status list is empty".into());
        }
        let depth = (bits.len() as u64).next_power_of_two().trailing_zeros();
        if depth > MAX_STATUS_DEPTH {
            return Err("status list exceeds the compiled depth".into());
        }
        let mut level: Vec<Fr> = bits
            .iter()
            .map(|b| envelope_policy::status_leaf(*b))
            .collect();
        level.resize(1 << depth, Fr::from(0u64));
        let mut levels = vec![level];
        while levels.last().unwrap().len() > 1 {
            let next = levels
                .last()
                .unwrap()
                .chunks(2)
                .map(|p| poseidon2::hash(&[p[0], p[1]]))
                .collect();
            levels.push(next);
        }
        let list = NamedNode::new(self.list.as_str()).map_err(err)?;
        let list = suite::term_cell(&Term::NamedNode(list), 0).map_err(err)?;
        let purpose = Literal::new_simple_literal(self.purpose.as_str());
        let purpose = suite::term_cell(&purpose.into(), 0).map_err(err)?;
        Ok(Tree {
            bits,
            depth,
            levels,
            list: field_to_be_bytes_32(&list.enc),
            purpose: field_to_be_bytes_32(&purpose.enc),
        })
    }
}
impl Tree {
    fn root(&self) -> Word {
        field_to_be_bytes_32(&self.levels.last().unwrap()[0])
    }
    fn digest(&self) -> Result<Word> {
        envelope_policy::snapshot_digest(
            self.list,
            self.purpose,
            self.bits.len() as u32,
            self.depth,
            self.root(),
        )
        .map_err(err)
    }
    fn path(&self, index: usize) -> Vec<Word> {
        (0..self.depth as usize)
            .map(|level| field_to_be_bytes_32(&self.levels[level][(index >> level) ^ 1]))
            .collect()
    }
}
/// The snapshot digest a verifier lists in `acceptedSnapshots`.
pub(crate) fn status_snapshot(input: &str) -> Result<serde_json::Value> {
    let list: StatusList = crate::codec::parse(input)?;
    let tree = list.tree()?;
    Ok(
        serde_json::json!({"digest":hex(&tree.digest()?),"root":hex(&tree.root()),
        "depth":tree.depth,"bitLength":tree.bits.len()}),
    )
}

fn object<'a>(quads: &'a [oxrdf::Quad], subject: &Term, predicate: &str) -> Result<&'a Term> {
    let mut found = quads
        .iter()
        .filter(|q| Term::from(q.subject.clone()) == *subject && q.predicate.as_str() == predicate);
    match (found.next(), found.next()) {
        (Some(q), None) => Ok(&q.object),
        _ => Err(format!("status entry needs exactly one {predicate}")),
    }
}
fn lexical(term: &Term) -> Result<&str> {
    match term {
        Term::Literal(l) => Ok(l.value()),
        _ => Err("status literal expected".into()),
    }
}

/// Build the holder's private evidence for the request's policy. The holder key signs
/// the complete request digest; each status entry opens its bit in a supplied list
/// whose snapshot the verifier accepted. A set bit refuses here, before proving.
pub(crate) fn evidence(
    policy: &PolicyInput,
    digest: &[u8; 32],
    documents: &[&str],
    holder_secret: Option<&str>,
    lists: &[StatusList],
) -> Result<EnvelopeWitness> {
    let mut evidence = EnvelopeWitness::default();
    if policy.holder_binding.is_some() {
        let secret = word(holder_secret.ok_or("holder key required by the request")?)?;
        let key = suite::keypair_from_seed(&secret);
        evidence.holders.push(HolderWitness {
            key: key.public_key(),
            signature: sig::sign_deterministic(&key, &envelope_policy::holder_message(digest)),
        });
    } else if holder_secret.is_some() {
        return Err("the request does not ask for holder binding".into());
    }
    let Some(accepted) = &policy.accepted_snapshots else {
        if !lists.is_empty() {
            return Err("the request does not ask for credential status".into());
        }
        evidence.credentials = documents
            .iter()
            .map(|_| CredentialEvidence::default())
            .collect();
        return Ok(evidence);
    };
    let trees = lists
        .iter()
        .map(StatusList::tree)
        .collect::<Result<Vec<_>>>()?;
    for document in documents {
        let quads = oxttl::NQuadsParser::new()
            .for_slice(document.as_bytes())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let mut statuses = vec![];
        for (quad_index, quad) in quads.iter().enumerate() {
            if quad.predicate.as_str() != CREDENTIAL_STATUS {
                continue;
            }
            let entry = &quad.object;
            let Term::NamedNode(list) = object(&quads, entry, STATUS_LIST)? else {
                return Err("status list must be an IRI".into());
            };
            let purpose = lexical(object(&quads, entry, STATUS_PURPOSE)?)?;
            let index: usize = lexical(object(&quads, entry, STATUS_INDEX)?)?
                .parse()
                .map_err(|_| "status index rejected")?;
            let (_, tree) = lists
                .iter()
                .zip(&trees)
                .find(|(l, _)| l.list == list.as_str() && l.purpose == purpose)
                .ok_or("no status list supplied for a credential status entry")?;
            if index >= tree.bits.len() {
                return Err("status index outside the supplied list".into());
            }
            if tree.bits[index] {
                return Err(format!("credential status is set ({purpose})"));
            }
            let (_, accepted_index, accepted_siblings) =
                envelope_policy::canonical_set_membership(&accepted.digests, tree.digest()?)
                    .map_err(|_| "status snapshot is not accepted by the request")?;
            statuses.push(StatusWitness {
                quad_index,
                bit_len: tree.bits.len() as u32,
                depth: tree.depth,
                root: tree.root(),
                accepted_index,
                accepted_siblings,
                bit: false,
                path_index: index as u64,
                path: tree.path(index),
            });
        }
        evidence.credentials.push(CredentialEvidence {
            holder_slot: 0,
            statuses,
        });
    }
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_zk_query::envelope_policy::{AcceptedSnapshots, HolderBinding};

    fn list(bits: &str) -> StatusList {
        StatusList {
            list: "urn:list".into(),
            purpose: "revocation".into(),
            bits: bits.into(),
        }
    }
    fn document(index: usize) -> String {
        format!(
            "<urn:vc> <urn:p> \"1\" .\n<urn:vc> <{CREDENTIAL_STATUS}> <urn:entry> .\n\
             <urn:entry> <{STATUS_LIST}> <urn:list> .\n<urn:entry> <{STATUS_PURPOSE}> \"revocation\" .\n\
             <urn:entry> <{STATUS_INDEX}> \"{index}\" .\n"
        )
    }
    fn policy(holder: bool, lists: &[&str]) -> PolicyInput {
        PolicyInput {
            holder_binding: holder.then_some(HolderBinding { issuers: None }),
            accepted_snapshots: (!lists.is_empty()).then(|| AcceptedSnapshots {
                digests: lists
                    .iter()
                    .map(|b| list(b).tree().unwrap().digest().unwrap())
                    .collect(),
                allow_missing: false,
            }),
            ..Default::default()
        }
    }
    fn run(
        policy: &PolicyInput,
        index: usize,
        holder: Option<&str>,
        lists: &[&str],
    ) -> Result<EnvelopeWitness> {
        let lists: Vec<_> = lists.iter().map(|b| list(b)).collect();
        evidence(policy, &[7; 32], &[&document(index)], holder, &lists)
    }

    #[test]
    fn status_paths_open_every_leaf_to_the_root() {
        for bits in ["0", "01", "011", "01001", "0000000011"] {
            let tree = list(bits).tree().unwrap();
            assert_eq!(1usize << tree.depth, bits.len().next_power_of_two());
            for index in 0..bits.len() {
                let mut node = envelope_policy::status_leaf(bits.as_bytes()[index] == b'1');
                for (level, sibling) in tree.path(index).iter().enumerate() {
                    let sibling =
                        sparq_zk::field::field_from_hex_str_strict(&hex(sibling)).unwrap();
                    node = if (index >> level) & 1 == 0 {
                        poseidon2::hash(&[node, sibling])
                    } else {
                        poseidon2::hash(&[sibling, node])
                    };
                }
                assert_eq!(field_to_be_bytes_32(&node), tree.root());
            }
        }
        assert!(list("").tree().is_err());
        assert!(list("012").tree().is_err());
    }

    #[test]
    fn evidence_opens_the_accepted_snapshot_and_names_each_refusal() {
        let secret = "11".repeat(32);
        let full = policy(true, &["0100", "1"]);
        let honest = run(&full, 0, Some(&secret), &["0100"]).unwrap();
        assert_eq!(honest.holders.len(), 1);
        let status = &honest.credentials[0].statuses[0];
        assert_eq!((status.quad_index, status.depth, status.bit_len), (1, 2, 4));
        assert!(!status.bit);
        let refusal = |r: Result<EnvelopeWitness>| r.err().unwrap();
        assert_eq!(
            refusal(run(&full, 1, Some(&secret), &["0100"])),
            "credential status is set (revocation)"
        );
        assert_eq!(
            refusal(run(&full, 4, Some(&secret), &["0100"])),
            "status index outside the supplied list"
        );
        assert_eq!(
            refusal(run(&full, 0, Some(&secret), &["0110"])),
            "status snapshot is not accepted by the request"
        );
        assert_eq!(
            refusal(run(&full, 0, Some(&secret), &[])),
            "no status list supplied for a credential status entry"
        );
        assert_eq!(
            refusal(run(&full, 0, None, &["0100"])),
            "holder key required by the request"
        );
        let bearer = policy(false, &[]);
        assert_eq!(
            refusal(run(&bearer, 0, Some(&secret), &[])),
            "the request does not ask for holder binding"
        );
        assert_eq!(
            refusal(run(&bearer, 0, None, &["0100"])),
            "the request does not ask for credential status"
        );
        let plain = run(&bearer, 0, None, &[]).unwrap();
        assert!(plain.holders.is_empty() && plain.credentials[0].statuses.is_empty());
    }
}

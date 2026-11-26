//! Raw canonical-envelope evidence. Sealed suite observations stay inside Noir.
use super::{json_toml, prover_toml_authenticated_mode, Authentication};
use crate::{
    credential::{field, Credential, SuiteEncoder},
    envelope_policy::{self, PolicyInput},
    interp::{Cell, EncodeTerm, Trace, Word},
    plan::{EnvelopeFeatures, Plan},
    statement::EnvelopeInputs,
    Error,
};
use oxrdf::{NamedOrBlankNode, Term};
use serde_json::{json, Value};
use sparq_zk::{field::field_to_hex, sig};

/// Shared holder signature over the complete request digest.
#[derive(Clone, Debug)]
pub struct HolderWitness {
    pub key: sig::PublicKey,
    pub signature: sig::Signature,
}
/// One accepted status snapshot and the bit opening at a signed status row.
#[derive(Clone, Debug)]
pub struct StatusWitness {
    pub quad_index: usize,
    pub bit_len: u32,
    /// Actual bit-tree depth, bounded independently of the accepted-set depth.
    pub depth: u32,
    pub root: Word,
    pub accepted_index: u32,
    pub accepted_siblings: Vec<Word>,
    pub bit: bool,
    pub path_index: u64,
    /// Leaf-to-root siblings, with exactly the actual bit-tree depth entries.
    pub path: Vec<Word>,
}
/// Evidence slots are ordered like the presented credentials.
#[derive(Clone, Debug, Default)]
pub struct CredentialEvidence {
    pub holder_slot: u32,
    pub statuses: Vec<StatusWitness>,
}
/// Private evidence; public policy is always derived from the stored request.
#[derive(Clone, Debug, Default)]
pub struct EnvelopeWitness {
    pub holders: Vec<HolderWitness>,
    pub credentials: Vec<CredentialEvidence>,
}

/// Canonical policy and private evidence for one rendering operation.
pub struct EnvelopeInput<'a> {
    pub policy: &'a PolicyInput,
    pub evidence: &'a EnvelopeWitness,
    pub revealed: Option<&'a crate::statement::RevealedInputs>,
}

fn bytes(text: &str, capacity: usize) -> Result<(Vec<u8>, usize), Error> {
    if text.len() > capacity {
        return Err(Error::Capacity("envelope lexical bytes".into()));
    }
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize(capacity, 0);
    Ok((bytes, text.len()))
}
fn lexical(c: &Credential, s: &NamedOrBlankNode, p: &str) -> Result<String, Error> {
    match c.property(s, p)? {
        None => Ok(String::new()),
        Some(Term::Literal(l)) => Ok(l.value().into()),
        Some(_) => Err(Error::Invalid("envelope property literal".into())),
    }
}
fn membership(index: u64, path: &[Word], capacity: usize) -> Result<Value, Error> {
    if path.len() > capacity || index >= (1u64 << path.len()) {
        return Err(Error::Capacity("envelope membership path".into()));
    }
    let mut siblings = path
        .iter()
        .copied()
        .map(|w| field(w).map(|f| field_to_hex(&f)))
        .collect::<Result<Vec<_>, _>>()?;
    siblings.resize(capacity, "0".into());
    Ok(json!({"index":index.to_string(),"siblings":siblings}))
}
fn property_row(
    quads: &[oxrdf::Quad],
    subject: &NamedOrBlankNode,
    predicate: &str,
) -> Result<usize, Error> {
    let rows: Vec<_> = quads
        .iter()
        .enumerate()
        .filter(|(_, q)| {
            &q.subject == subject
                && q.predicate.as_str() == predicate
                && q.graph_name.is_default_graph()
        })
        .map(|(i, _)| i)
        .collect();
    let [row] = rows.as_slice() else {
        return Err(Error::Invalid("status property cardinality".into()));
    };
    Ok(*row)
}

fn empty_status() -> Value {
    json!({"row":0,"list_row":0,"purpose_row":0,"depth":0,"present":false,"bit_len":0,"root":"0","accepted":{"index":"0","siblings":vec!["0";4]},"bit":false,
        "path":{"index":"0","siblings":vec!["0";32]},"purpose":vec![0;16],"purpose_len":0,
        "index":vec![0;20],"index_len":0,"size":vec![0;20],"size_len":0})
}
fn raw_credential(
    c: Option<&Credential>,
    e: Option<&CredentialEvidence>,
    capacity: usize,
    features: EnvelopeFeatures,
) -> Result<Value, Error> {
    let mut status = vec![empty_status(); capacity];
    let mut from = String::new();
    let mut until = String::new();
    if let Some(c) = c {
        let root = c.vc_subject()?;
        if features.validity {
            from = lexical(c, &root, "https://www.w3.org/2018/credentials#validFrom")?;
            until = lexical(c, &root, "https://www.w3.org/2018/credentials#validUntil")?;
        }
        let quads = c.quads()?;
        for proof in e.into_iter().flat_map(|e| &e.statuses) {
            if proof.depth > 32 || proof.path.len() != proof.depth as usize {
                return Err(Error::Capacity("status bit tree depth".into()));
            }
            if proof.quad_index >= quads.len() || proof.quad_index >= capacity {
                return Err(Error::Invalid("status evidence row".into()));
            }
            if status[proof.quad_index]["present"] == true {
                return Err(Error::Invalid("duplicate status evidence row".into()));
            }
            let entry = NamedOrBlankNode::try_from(quads[proof.quad_index].object.clone())
                .map_err(|_| Error::Invalid("status entry kind".into()))?;
            let list_row = property_row(
                &quads,
                &entry,
                "https://www.w3.org/ns/credentials/status#statusListCredential",
            )?;
            let purpose_row = property_row(
                &quads,
                &entry,
                "https://www.w3.org/ns/credentials/status#statusPurpose",
            )?;
            let (purpose, purpose_len) = bytes(
                &lexical(
                    c,
                    &entry,
                    "https://www.w3.org/ns/credentials/status#statusPurpose",
                )?,
                16,
            )?;
            let (index, index_len) = bytes(
                &lexical(
                    c,
                    &entry,
                    "https://www.w3.org/ns/credentials/status#statusListIndex",
                )?,
                20,
            )?;
            let (size, size_len) = bytes(
                &lexical(
                    c,
                    &entry,
                    "https://www.w3.org/ns/credentials/status#statusSize",
                )?,
                20,
            )?;
            status[proof.quad_index] = json!({"list_row":list_row,"purpose_row":purpose_row,"depth":proof.depth,"present":true,"bit_len":proof.bit_len,"root":field_to_hex(&field(proof.root)?),
                "accepted":membership(proof.accepted_index as u64,&proof.accepted_siblings,4)?,"bit":proof.bit,
                "path":membership(proof.path_index,&proof.path,32)?,"purpose":purpose,"purpose_len":purpose_len,
                "index":index,"index_len":index_len,"size":size,"size_len":size_len});
        }
    }
    // The circuit checks entries through STATUS_SLOTS selected rows in tree
    // order and proves the count covers every signed entry; more is no proof.
    let mut slots = Vec::with_capacity(crate::plan::STATUS_SLOTS);
    for (row, mut entry) in status.into_iter().enumerate() {
        if entry["present"] == true {
            entry["row"] = json!(row);
            slots.push(entry);
        }
    }
    if slots.len() > crate::plan::STATUS_SLOTS {
        return Err(Error::Capacity("status entry slots".into()));
    }
    slots.resize(crate::plan::STATUS_SLOTS, empty_status());
    let (from, from_len) = bytes(&from, 32)?;
    let (until, until_len) = bytes(&until, 32)?;
    Ok(
        json!({"holder_slot":e.map_or(0,|e|e.holder_slot),"status":slots,
        "validity":{"from":from,"from_len":from_len,"until":until,"until_len":until_len}}),
    )
}

/// Authorize complete signed credentials with the policy's owned canonical builder.
pub(crate) fn authorize(
    credentials: &[Credential],
    policy: &PolicyInput,
    envelope: &EnvelopeInputs,
) -> Result<(Vec<Credential>, Vec<Cell>), Error> {
    credentials
        .iter()
        .map(|credential| {
            let member = credential.trust_member()?;
            let (set, index, path) =
                Credential::authorization_path_for_member(&member, &policy.trust_members)?;
            if set.root != envelope.issuer_root || set.depth != envelope.issuer_depth {
                return Err(Error::Invalid("issuer membership root".into()));
            }
            let mut c = credential.clone();
            c.issuer_index = index as u64;
            c.issuer_siblings = path.into_iter().map(field).collect::<Result<Vec<_>, _>>()?;
            Ok((c, SuiteEncoder.encode(&member.issuer.into())?))
        })
        .collect()
}

/// Render inputs for `codegen::generate_enveloped`, using canonical authorization paths.
pub fn prover_toml_enveloped(
    plan: &Plan,
    credentials: &[Credential],
    trace: &Trace,
    digest: Word,
    salt: Word,
    input: EnvelopeInput<'_>,
) -> Result<String, Error> {
    let EnvelopeInput {
        policy,
        evidence,
        revealed,
    } = input;
    let envelope: EnvelopeInputs = envelope_policy::derive(policy)?.into();
    envelope.validate()?;
    let (credentials, issuers) = authorize(credentials, policy, &envelope)?;
    render_authorized(
        plan,
        &credentials,
        &issuers,
        trace,
        digest,
        salt,
        CanonicalInput {
            public: &envelope,
            evidence,
            revealed,
        },
    )
}

pub(crate) struct CanonicalInput<'a> {
    pub public: &'a EnvelopeInputs,
    pub evidence: &'a EnvelopeWitness,
    pub revealed: Option<&'a crate::statement::RevealedInputs>,
}

/// The root-only seam consumes canonical trust-member witnesses, never key-only paths.
pub(crate) fn render_canonical(
    plan: &Plan,
    credentials: &[Credential],
    trace: &Trace,
    digest: Word,
    salt: Word,
    input: CanonicalInput<'_>,
) -> Result<String, Error> {
    plan.envelope_features().check(input.public)?;
    let issuers = authenticated_issuers(credentials, input.public)?;
    render_authorized(plan, credentials, &issuers, trace, digest, salt, input)
}

pub(crate) fn authenticated_issuers(
    credentials: &[Credential],
    envelope: &EnvelopeInputs,
) -> Result<Vec<Cell>, Error> {
    credentials
        .iter()
        .map(|credential| {
            let (root, issuer) = credential.canonical_issuer(envelope.issuer_depth)?;
            if root != envelope.issuer_root {
                return Err(Error::Invalid("issuer membership root".into()));
            }
            SuiteEncoder.encode(&issuer.into())
        })
        .collect()
}

pub(crate) fn render_authorized(
    plan: &Plan,
    credentials: &[Credential],
    issuers: &[Cell],
    trace: &Trace,
    digest: Word,
    salt: Word,
    input: CanonicalInput<'_>,
) -> Result<String, Error> {
    let CanonicalInput {
        public: envelope,
        evidence,
        revealed,
    } = input;
    plan.envelope_features().check(envelope)?;
    let cap = plan.capacity().credentials as usize;
    if evidence.holders.len() > cap || evidence.credentials.len() > credentials.len() {
        return Err(Error::Capacity("envelope evidence slots".into()));
    }
    let mut out = prover_toml_authenticated_mode(
        plan,
        credentials,
        trace,
        digest,
        salt,
        envelope,
        Authentication {
            revealed,
            canonical_issuers: Some(issuers),
        },
    )?;
    let empty = json!({"pk":{"x":"0","y":"0"},"sig":{"present":false,"r":{"x":"0","y":"0"},"s":"0","e":"0","e_k":"0"}});
    let mut holders = vec![empty; cap];
    let message = envelope_policy::holder_message(&digest);
    for (i, holder) in evidence.holders.iter().enumerate() {
        if !sig::verify(&holder.key, &message, &holder.signature) {
            return Err(Error::Invalid("holder signature".into()));
        }
        let w = sig::in_circuit_witness(&holder.key, &message, &holder.signature)
            .ok_or_else(|| Error::Invalid("holder signature witness".into()))?;
        holders[i] = json!({"pk":{"x":field_to_hex(&w.pk_x),"y":field_to_hex(&w.pk_y)},
            "sig":{"present":true,"r":{"x":field_to_hex(&w.r_x),"y":field_to_hex(&w.r_y)},"s":field_to_hex(&w.s),"e":field_to_hex(&w.e),"e_k":field_to_hex(&w.e_k)}});
    }
    out.push_str(&format!(
        "holder_evidence = {}\n",
        json_toml(&Value::Array(holders))
    ));
    for (i, n) in plan.credential_quad_capacities().into_iter().enumerate() {
        out.push_str(&format!(
            "envelope_{i} = {}\n",
            json_toml(&raw_credential(
                credentials.get(i),
                evidence.credentials.get(i),
                n as usize,
                plan.envelope_features(),
            )?)
        ));
    }
    crate::witness_closed::append_authenticated(plan, credentials, trace, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capacity::Bounds,
        credential::{assemble_issuer_dataset, TRUST_MEMBER_CHECKS, VERIFIED_ISSUER},
        interp::evaluate_dataset,
        lower::lower,
    };

    fn canonical_fixture() -> ([Credential; 1], PolicyInput) {
        let key = sig::SecretKey::from_seed(19);
        let data = [Credential {
            credential: sparq_merkle_suite::poseidon::issue(
                concat!(
                    "<urn:vc> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ",
                    "<https://www.w3.org/2018/credentials#VerifiableCredential> .\n",
                    "<urn:vc> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n",
                ),
                concat!(
                    "_:p <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n",
                    "_:p <https://w3id.org/security#verificationMethod> <urn:method> .\n",
                ),
                &key,
                101u64.into(),
            )
            .unwrap(),
            issuer: key.public_key(),
            issuer_index: 0,
            issuer_siblings: vec![],
        }];
        let policy = PolicyInput {
            trust_members: vec![data[0].trust_member().unwrap()],
            ..Default::default()
        };
        (data, policy)
    }

    #[test]
    fn canonical_witness_without_a_scan_skips_dataset_build() {
        use crate::witness::SCAN_DATASET_BUILDS;

        let (data, policy) = canonical_fixture();
        let plan = lower(
            "ASK {}",
            Bounds {
                credentials: 1,
                quads_per_credential: 16,
                rows_per_table: 2,
            },
        )
        .unwrap();
        let digest = [7; 32];
        let dataset = assemble_issuer_dataset(&data, &policy.trust_members, digest).unwrap();
        let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
        let before = SCAN_DATASET_BUILDS.get();
        let input = prover_toml_enveloped(
            &plan,
            &data,
            &trace,
            digest,
            [3; 32],
            EnvelopeInput {
                policy: &policy,
                evidence: &EnvelopeWitness::default(),
                revealed: None,
            },
        )
        .unwrap();
        assert_eq!(SCAN_DATASET_BUILDS.get(), before);
        assert!(input.lines().any(|line| line == "result = true"));
        assert!(input.lines().any(|line| line.starts_with("envelope_0 = ")));
        assert!(!input
            .lines()
            .any(|line| line.starts_with("target_") || line.starts_with("source_")));
    }

    #[test]
    fn canonical_scan_reuses_the_issuer_from_membership_verification() {
        let (data, policy) = canonical_fixture();
        let digest = [7; 32];
        let salt = [3; 32];
        let plan = lower(
            &format!("ASK {{?g <{VERIFIED_ISSUER}> <urn:issuer>}}"),
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 2,
            },
        )
        .unwrap();
        let before = TRUST_MEMBER_CHECKS.get();
        let dataset = assemble_issuer_dataset(&data, &policy.trust_members, digest).unwrap();
        assert_eq!(TRUST_MEMBER_CHECKS.get() - before, data.len());
        let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
        let evidence = EnvelopeWitness::default();
        let envelope = envelope_policy::derive(&policy).unwrap().into();

        let before = TRUST_MEMBER_CHECKS.get();
        let from_policy = prover_toml_enveloped(
            &plan,
            &data,
            &trace,
            digest,
            salt,
            EnvelopeInput {
                policy: &policy,
                evidence: &evidence,
                revealed: None,
            },
        )
        .unwrap();
        assert_eq!(TRUST_MEMBER_CHECKS.get() - before, data.len());
        assert!(from_policy.lines().any(|line| line == "source_0 = [4, 0]"));

        let (with_paths, _) = authorize(&data, &policy, &envelope).unwrap();
        let before = TRUST_MEMBER_CHECKS.get();
        let from_root = render_canonical(
            &plan,
            &with_paths,
            &trace,
            digest,
            salt,
            CanonicalInput {
                public: &envelope,
                evidence: &evidence,
                revealed: None,
            },
        )
        .unwrap();
        assert_eq!(TRUST_MEMBER_CHECKS.get() - before, data.len());
        assert_eq!(from_root, from_policy);
    }
}

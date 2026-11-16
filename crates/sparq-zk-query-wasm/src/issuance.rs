use crate::codec::{err, field, hex, word, Result};
use serde::{Deserialize, Serialize};
use sparq_merkle_suite::poseidon as suite;
use sparq_zk::sig;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TestIssuer {
    pub seed_hex: String,
    pub public_key: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Issuer {
    pub secret_key_hex: String,
    pub public_key: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum IssuerInput {
    Test(TestIssuer),
    Production(Issuer),
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configuration {
    proof_config_n_quads: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IssueInput {
    issuer: IssuerInput,
    graph_n_quads: String,
    status: Option<Configuration>,
    seed_hex: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Credential {
    pub graph_n_quads: String,
    pub proof_config_n_quads: String,
    pub salt_hex: String,
    pub root_hex: String,
    pub count: u32,
    pub issuer: String,
    pub signature_hex: String,
}
impl Credential {
    pub(crate) fn open(&self) -> Result<(suite::Credential, sig::PublicKey)> {
        let pk = sig::public_key_from_hex(&self.issuer).ok_or("issuer key rejected")?;
        let credential = suite::Credential {
            document: self.graph_n_quads.clone(),
            proof_config: self.proof_config_n_quads.clone(),
            salt: field(&self.salt_hex)?,
            root: field(&self.root_hex)?,
            count: self.count,
            signature: sig::signature_from_hex(&self.signature_hex)
                .ok_or("signature encoding rejected")?,
        };
        if !suite::verify(&credential, &pk) {
            return Err("credential suite signature or opening rejected".into());
        }
        Ok((credential, pk))
    }
}
pub(crate) fn keygen() -> Result<Issuer> {
    let mut secret = [0; 32];
    getrandom_latest::fill(&mut secret).map_err(err)?;
    let key = suite::keypair_from_seed(&secret);
    Ok(Issuer {
        secret_key_hex: hex(&secret),
        public_key: sig::public_key_to_hex(&key.public_key()),
    })
}
pub(crate) fn test_keygen(seed: &str) -> Result<TestIssuer> {
    let seed = word(seed)?;
    let key = suite::keypair_from_seed(&seed);
    Ok(TestIssuer {
        seed_hex: hex(&seed),
        public_key: sig::public_key_to_hex(&key.public_key()),
    })
}
pub(crate) fn issue(input: &str, deterministic: bool) -> Result<Credential> {
    let input: IssueInput = crate::codec::parse(input)?;
    let (material, expected_key) = match &input.issuer {
        IssuerInput::Test(key) => (&key.seed_hex, &key.public_key),
        IssuerInput::Production(_) if deterministic => {
            return Err("test issuance requires a test issuer".into())
        }
        IssuerInput::Production(key) => (&key.secret_key_hex, &key.public_key),
    };
    let key = suite::keypair_from_seed(&word(material)?);
    let public = sig::public_key_to_hex(&key.public_key());
    if &public != expected_key {
        return Err("issuer key differs from key material".into());
    }
    let configuration = input
        .status
        .map(|s| s.proof_config_n_quads)
        .unwrap_or_else(|| {
            format!(
                "_:proof <https://w3id.org/security#cryptosuite> \"{}\" .\n",
                suite::SCHNORR_POSEIDON2_MERKLE_2026
            )
        });
    let credential = if deterministic {
        let seed = word(
            input
                .seed_hex
                .as_deref()
                .ok_or("test issuance requires a seed")?,
        )?;
        suite::issue_deterministic(&input.graph_n_quads, &configuration, &key, &seed)
            .map_err(err)?
    } else {
        if input.seed_hex.is_some() {
            return Err("ordinary issuance does not accept a seed".into());
        }
        sparq_merkle_suite::random::issue(&input.graph_n_quads, &configuration, &key)
            .map_err(err)?
    };
    Ok(Credential {
        graph_n_quads: credential.document,
        proof_config_n_quads: credential.proof_config,
        salt_hex: hex(&suite::field_bytes(credential.salt)),
        root_hex: hex(&suite::field_bytes(credential.root)),
        count: credential.count,
        issuer: public,
        signature_hex: sig::signature_to_hex(&credential.signature),
    })
}

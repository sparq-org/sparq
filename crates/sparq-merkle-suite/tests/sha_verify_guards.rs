//! Verification controls for the frozen-compatible SHA member.
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::{sha256 as s, SignedCredential};
const CONFIG: &str =
    "<urn:p> <https://w3id.org/security#cryptosuite> \"eddsa-sha256-merkle-2026\" .\n";
fn fixture() -> (SignedCredential, SigningKey) {
    let key = SigningKey::from_bytes(&[19; 32]);
    (
        s::issue(
            "<urn:a> <urn:p> <urn:o> .\n<urn:b> <urn:p> <urn:o> .\n",
            CONFIG,
            &key,
            [7; 32],
        )
        .unwrap(),
        key,
    )
}
fn leaves(c: &SignedCredential) -> Vec<[u8; 32]> {
    oxttl::NQuadsParser::new()
        .for_slice(c.document.as_bytes())
        .map(|q| s::leaf(&q.unwrap()).unwrap())
        .collect()
}
fn unchecked_root(mut row: Vec<[u8; 32]>) -> [u8; 32] {
    row.resize(row.len().next_power_of_two(), [0; 32]);
    while row.len() > 1 {
        row = row
            .chunks_exact(2)
            .map(|p| {
                let mut h = Sha256::new();
                h.update([1]);
                h.update(p[0]);
                h.update(p[1]);
                h.finalize().into()
            })
            .collect();
    }
    row[0]
}
fn sign(c: &mut SignedCredential, key: &SigningKey, suite: &str, count: u32, root: [u8; 32]) {
    let mut hash = Sha256::new();
    hash.update(suite.as_bytes());
    hash.update([0]);
    hash.update(&c.signature[64..]);
    hash.update(count.to_be_bytes());
    hash.update(root);
    hash.update(Sha256::digest(c.proof_config.as_bytes()));
    let m: [u8; 32] = hash.finalize().into();
    c.signature[..64].copy_from_slice(&key.sign(&m).to_bytes());
}
#[test]
fn sha_verify_rejects_resigned_order() {
    let (mut c, k) = fixture();
    let mut lines = c.document.lines().collect::<Vec<_>>();
    lines.reverse();
    c.document = lines.join("\n") + "\n";
    let root = unchecked_root(leaves(&c));
    sign(&mut c, &k, s::EDDSA_SHA256_MERKLE_2026, 2, root);
    assert!(!s::verify(&c, &k.verifying_key()));
}
#[test]
fn sha_verify_rejects_resigned_count() {
    let (mut c, k) = fixture();
    let root = unchecked_root(leaves(&c));
    sign(&mut c, &k, s::EDDSA_SHA256_MERKLE_2026, 1, root);
    assert!(!s::verify(&c, &k.verifying_key()));
}
#[test]
fn sha_verify_rejects_salt() {
    let (mut c, k) = fixture();
    c.signature[64] ^= 1;
    assert!(!s::verify(&c, &k.verifying_key()));
}
#[test]
fn sha_verify_rejects_suite() {
    let (mut c, k) = fixture();
    let root = unchecked_root(leaves(&c));
    sign(&mut c, &k, "another-suite", 2, root);
    assert!(!s::verify(&c, &k.verifying_key()));
}
#[test]
fn sha_verify_rejects_configuration() {
    let (mut c, k) = fixture();
    c.proof_config.push_str("<urn:z> <urn:p> <urn:o> .\n");
    assert!(!s::verify(&c, &k.verifying_key()));
}
#[test]
fn sha_verify_rejects_root() {
    let (mut c, k) = fixture();
    sign(&mut c, &k, s::EDDSA_SHA256_MERKLE_2026, 2, [0; 32]);
    assert!(!s::verify(&c, &k.verifying_key()));
}

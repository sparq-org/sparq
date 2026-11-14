//! Research grade and not externally audited. Independent suite reference for test vectors.
use sparq_merkle_suite::poseidon;
use std::io::{self, Read};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let v: serde_json::Value = serde_json::from_str(&input)?;
    fn bytes(s: &str) -> [u8; 32] {
        std::array::from_fn(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
    }
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    let key =
        poseidon::keypair_from_seed(&bytes(v["issuerSeedHex"].as_str().ok_or("issuer seed")?));
    let credential = poseidon::issue_deterministic(
        v["graphNQuads"].as_str().ok_or("document")?,
        v["proofConfigNQuads"].as_str().ok_or("configuration")?,
        &key,
        &bytes(v["seedHex"].as_str().ok_or("salt seed")?),
    )?;
    println!(
        "{}",
        serde_json::json!({"graphNQuads":credential.document,"proofConfigNQuads":credential.proof_config,"issuer":sparq_zk::sig::public_key_to_hex(&key.public_key()),"saltHex":hex(&poseidon::field_bytes(credential.salt)),"rootHex":hex(&poseidon::field_bytes(credential.root)),"count":credential.count,"signatureHex":sparq_zk::sig::signature_to_hex(&credential.signature)})
    );
    Ok(())
}

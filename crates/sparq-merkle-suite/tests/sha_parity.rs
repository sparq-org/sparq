//! Required-CI compatibility with the frozen source, included directly, never copied.
use oxrdf::{Literal, NamedNode, Term};
use sparq_merkle_suite::{sha256 as current, SignedCredential};
pub use sparq_merkle_suite::Error as Rejected;
mod authenticated_rdf { pub type SignedCredential = sparq_merkle_suite::SignedCredential; }
#[allow(dead_code)]
#[path = "../../../zk/sparql-evaluator/model/src/merkle_suite.rs"]
mod frozen;
use sha2::{Digest, Sha256};

// The frozen merkle module has no verify function. This reference adapter drives
// its actual leaf/root/message functions and the same strict Ed25519 primitive.
fn frozen_verify(c: &SignedCredential, pk: &ed25519_dalek::VerifyingKey) -> bool {
    let check = || -> Option<()> {
        let leaves = oxttl::NQuadsParser::new().for_slice(c.document.as_bytes())
            .map(|q| frozen::leaf(&q.ok()?)).collect::<Option<Vec<_>>>()?;
        let root = frozen::root(&leaves)?;
        if c.signature.len() != 96 { return None; }
        let salt = c.signature[64..].try_into().ok()?;
        let digest: [u8;32] = Sha256::digest(c.proof_config.as_bytes()).into();
        let message = frozen::signed_message(&salt, leaves.len().try_into().ok()?, &root, &digest);
        let signature = ed25519_dalek::Signature::from_slice(&c.signature[..64]).ok()?;
        pk.verify_strict(&message, &signature).ok()
    }; check().is_some()
}
#[test]
fn frozen_source_and_current_suite_match_vectors_and_generated_inputs() {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[19;32]);
    let base_config = "<urn:p> <https://w3id.org/security#cryptosuite> \"eddsa-sha256-merkle-2026\" .\n";
    let mut seed = 0x324d_5678_92ab_cdefu64;
    let mut classes = std::collections::BTreeSet::new();
    for generation in 0..24 {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        let values = [
            ("integer", (seed as i64).to_string()),
            ("decimal", format!("-{}.125", seed % 100000)),
            ("double", format!("{}e-10", seed)),
            ("float", format!("{}.5", seed % 997)),
            ("dateTime", format!("2026-01-{:02}T01:02:03Z", 1+seed%28)),
            ("dateTime", format!("2026-01-{:02}T01:02:03", 1+seed%28)),
            ("boolean", if seed & 1 == 0 { "true" } else { "false" }.to_owned()),
            ("string", format!("short-{seed}")),
            ("string", format!("long-{seed}-{}", "λ".repeat(24))),
            ("double", "NaN".to_owned()),
            ("decimal", "0.0000000000000000001".to_owned()),
            ("other", format!("unknown-{seed}")),
            ("date", "2026-01-01".to_owned()),
            ("unsignedByte", (seed % 256).to_string()),
        ];
        let mut document = String::new();
        for (i, (dt, text)) in values.iter().enumerate() {
            let literal = Literal::new_typed_literal(text, NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{dt}")));
            let a = current::comparison_key(&literal); let b = frozen::comparison_key(&literal);
            assert_eq!(a,b,"SHA profile key differs: {literal}"); classes.insert(a[0]);
            let term = Term::Literal(literal);
            let mut a = Vec::new(); let mut b = Vec::new(); current::encode_term(&term,&mut a); frozen::encode_term(&term,&mut b); assert_eq!(a,b);
            document.push_str(&format!("_:s{i} <urn:p> {term} .\n"));
        }
        let config=format!("{base_config}<urn:p> <urn:option> \"{generation}\" .\n");
        let salt = [generation as u8;32];
        let a = current::issue(&document,&config,&sk,salt).unwrap();
        let b = frozen::issue(&document,&config,&sk,salt).unwrap();
        assert_eq!(a,b,"issued bytes differ");
        let leaves = |text: &str| oxttl::NQuadsParser::new().for_slice(text.as_bytes()).map(|q| q.unwrap()).collect::<Vec<_>>();
        let mut row = Vec::new();
        for q in leaves(&a.document) { let left=current::leaf(&q);let right=frozen::leaf(&q); assert_eq!(left,right);row.push(left.unwrap()); }
        let root=current::root(&row).unwrap();assert_eq!(Some(root),frozen::root(&row));
        let digest: [u8;32]=Sha256::digest(a.proof_config.as_bytes()).into();
        assert_eq!(current::signed_message(&salt,row.len() as u32,&root,&digest), frozen::signed_message(&salt,row.len() as u32,&root,&digest));
        assert!(current::verify(&a,&sk.verifying_key()));assert!(frozen_verify(&b,&sk.verifying_key()));
        for index in [0,63,64,95] { let mut altered=a.clone();altered.signature[index]^=1;assert!(!current::verify(&altered,&sk.verifying_key()));assert!(!frozen_verify(&altered,&sk.verifying_key())); }
    }
    assert_eq!(classes, [0,1,2,3,4,5,6,7,127].into_iter().collect());
}

#[test]
fn existing_sha_vectors_are_byte_identical() {
    let source=include_str!("../../../zk/sparql-evaluator/model/tests/merkle_suite.rs");
    let constant=|name:&str|source.split(&format!("const {name}: &str = \"")).nth(1).unwrap().split('"').next().unwrap();
    let hex=|bytes:&[u8]|bytes.iter().map(|b|format!("{b:02x}")).collect::<String>();
    let parse=|text:&str|oxttl::NQuadsParser::new().for_slice(text.as_bytes()).next().unwrap().unwrap();
    let a=parse("_:c14n0 <http://ex/balance> \"1250.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .");
    let b=parse("<urn:vc:1> <http://ex/name> \"Alice\"@en .");
    let leaf=current::leaf(&a).unwrap();assert_eq!(hex(&leaf),constant("LEAF"));assert_eq!(Some(leaf),frozen::leaf(&a));
    let mut row=[leaf,current::leaf(&b).unwrap()];row.sort();let root=current::root(&row).unwrap();assert_eq!(hex(&root),constant("ROOT"));assert_eq!(Some(root),frozen::root(&row));
    for bad in [Vec::new(),vec![row[1],row[0]],vec![leaf,leaf]] {assert_eq!(current::root(&bad),None);assert_eq!(frozen::root(&bad),None);}
    let message=current::signed_message(&[7;32],2,&root,&[9;32]);assert_eq!(hex(&message),constant("MESSAGE"));assert_eq!(message,frozen::signed_message(&[7;32],2,&root,&[9;32]));
    let text=constant("TEST1_SECRET");let bytes=std::array::from_fn(|i|u8::from_str_radix(&text[2*i..2*i+2],16).unwrap());let key=ed25519_dalek::SigningKey::from_bytes(&bytes);
    let signature=ed25519_dalek::Signer::sign(&key,&message);assert_eq!(hex(&key.verifying_key().to_bytes()),constant("TEST1_PUBLIC"));assert_eq!(hex(&signature.to_bytes()),constant("SIGNATURE"));key.verifying_key().verify_strict(&message,&signature).unwrap();
}

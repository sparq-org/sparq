//! Behavioural controls for every native verification input binding.
use sparq_merkle_suite::{metadata, poseidon as p, Fr};
use sparq_zk::{poseidon2, sig};
use sha2::{Digest,Sha256};
const DOC: &str="<urn:a> <urn:p> <urn:o> .\n<urn:b> <urn:p> <urn:o> .\n";
const CONFIG: &str="<urn:p> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
fn fixture()->(p::Credential,sig::SecretKey) {let key=sig::SecretKey::from_seed(19);(p::issue(DOC,CONFIG,&key,Fr::from(7u64)).unwrap(),key)}
fn message(c:&p::Credential,key:&sig::SecretKey,suite:Fr)->Fr {
 let config: [u8;32]=Sha256::digest(c.proof_config.as_bytes()).into();
 p::message_for_suite(suite,c.salt,c.count,c.root,&config,metadata::from_configuration(&c.proof_config,&key.public_key()).unwrap().digest)
}
fn unchecked_root(qs:&[Fr],salt:Fr)->Fr {
 let mut leaves=qs.iter().enumerate().map(|(i,q)|p::leaf(*q,salt,i as u32)).collect::<Vec<_>>();leaves.resize(leaves.len().next_power_of_two(),Fr::from(0u64));
 while leaves.len()>1 {leaves=leaves.chunks_exact(2).map(|p|poseidon2::hash(&[Fr::from(p::NODE_DOMAIN),p[0],p[1]])).collect();}leaves[0]
}
#[test]
fn verify_rejects_resigned_out_of_order_quads() {
 let(mut c,key)=fixture();let mut rows=c.document.lines().collect::<Vec<_>>();rows.reverse();c.document=rows.join("\n")+"\n";
 let qs=oxttl::NQuadsParser::new().for_slice(c.document.as_bytes()).map(|q|p::quad_hash(&q.unwrap()).unwrap()).collect::<Vec<_>>();
 c.root=unchecked_root(&qs,c.salt);c.signature=sig::sign_deterministic(&key,&message(&c,&key,p::suite_id()));assert!(!p::verify(&c,&key.public_key()));
}
#[test]
fn verify_rejects_resigned_wrong_count() {let(mut c,key)=fixture();c.count+=1;c.signature=sig::sign_deterministic(&key,&message(&c,&key,p::suite_id()));assert!(!p::verify(&c,&key.public_key()));}
#[test]
fn verify_rejects_wrong_salt() {let(mut c,key)=fixture();c.salt=Fr::from(8u64);assert!(!p::verify(&c,&key.public_key()));}
#[test]
fn verify_rejects_another_suite_message() {let(mut c,key)=fixture();c.signature=sig::sign_deterministic(&key,&message(&c,&key,Fr::from(1u64)));assert!(!p::verify(&c,&key.public_key()));}
#[test]
fn verify_rejects_another_configuration() {let(mut c,key)=fixture();c.proof_config.push_str("<urn:z> <urn:p> <urn:o> .\n");assert!(!p::verify(&c,&key.public_key()));}
#[test]
fn creation_metadata_is_verified_before_exposure() {
 let key=sig::SecretKey::from_seed(19);
 let config=format!("{CONFIG}<urn:p> <http://purl.org/dc/terms/created> \"2026-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n");
 let mut c=p::issue(DOC,&config,&key,Fr::from(7u64)).unwrap();let opened=p::verify_and_open(&c,&key.public_key()).unwrap();
 assert_eq!(metadata::timestamp(opened.created.as_ref()),Some(1767225600000));assert_eq!(metadata::multikey(&key.public_key()).unwrap().len(),metadata::MULTIKEY_LENGTH);
 assert!(p::verify_and_open(&c,&sig::SecretKey::from_seed(20).public_key()).is_err());
 let literal=oxrdf::Literal::new_simple_literal(metadata::multikey(&key.public_key()).unwrap());
 assert!(metadata::open_multikey(&literal,&key.public_key()).is_ok());
 assert!(metadata::open_multikey(&literal,&sig::SecretKey::from_seed(20).public_key()).is_err());
 let mut codec=0u32;for (i,b) in metadata::MULTIKEY_PREFIX.iter().enumerate() { codec |= u32::from(b & 127) << (7*i); }assert_eq!(codec,metadata::MULTIKEY_CODEC);
 c.proof_config=c.proof_config.replace("2026-01-01","2026-01-02");assert!(p::verify_and_open(&c,&key.public_key()).is_err());
 let unknown=p::issue(DOC,&format!("{CONFIG}<urn:p> <http://purl.org/dc/terms/created> \"unknown\" .\n"),&key,Fr::from(7u64)).unwrap();
 assert_eq!(metadata::timestamp(p::verify_and_open(&unknown,&key.public_key()).unwrap().created.as_ref()),None);
}
#[test]
fn status_index_uses_decimal_strings() {
 use oxrdf::Literal;
 assert_eq!(metadata::status_index(&Literal::new_simple_literal("0007")),Ok(7));
 for bad in ["","+7","-1"," 7","7.0","18446744073709551616"] {assert!(metadata::status_index(&Literal::new_simple_literal(bad)).is_err());}
}
#[test]
fn verify_rejects_resigned_wrong_root() {let(mut c,key)=fixture();c.root=Fr::from(0u64);c.signature=sig::sign_deterministic(&key,&message(&c,&key,p::suite_id()));assert!(!p::verify(&c,&key.public_key()));}
#[test]
fn verify_rejects_another_key() {let(c,_key)=fixture();assert!(!p::verify(&c,&sig::SecretKey::from_seed(20).public_key()));}

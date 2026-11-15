use std::collections::bounded_vec::BoundedVec;
use crate::term::{TermCell, open_term, recompute_key, padding, field_lt, hash_bytes};
use crate::suite::{quad_hash, leaf_salt, leaf, credential_root, signed_message, check_credential_signature};
use crate::suite_constants::TERM_DOMAIN;
use poseidon::poseidon2::Poseidon2;
use compose::issuer::Point;

fn empty() -> BoundedVec<u8, 1> { BoundedVec::new() }
fn iri<let N: u32>(text: [u8; N]) -> TermCell { open_term(1, BoundedVec::<u8, N>::from_array(text), empty(), empty(), [0; 31], 0) }
fn blank<let N: u32>(text: [u8; N]) -> TermCell { open_term(2, BoundedVec::<u8, N>::from_array(text), empty(), empty(), [0; 31], 0) }
fn decimal_key() -> [u8; 31] {
    let field: Field = 0x000180000000000000000000000000000000000000000043ca2c749129fa0000;
    let bytes = field.to_be_bytes::<32>();
    let mut key = [0; 31]; for i in 0..31 { key[i] = bytes[i + 1]; } key
}
fn decimal(key: [u8; 31]) -> TermCell {
    open_term(3, BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("1250.50".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()), empty(), key, 0)
}
fn vector_quad() -> [TermCell; 4] {
    [blank("c14n0".as_bytes()), iri("http://ex/balance".as_bytes()), decimal(decimal_key()), open_term(0, empty(), empty(), empty(), [0; 31], 0)]
}
fn other_quad() -> [TermCell; 4] {
    [iri("urn:vc:1".as_bytes()), iri("http://ex/name".as_bytes()),
        open_term(3, BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("Alice".as_bytes()),
            BoundedVec::<u8, 64>::from_array("http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes()),
            BoundedVec::<u8, 8>::from_array("en".as_bytes()), [0; 31], 0),
        open_term(0, empty(), empty(), empty(), [0; 31], 0)]
}
fn q(quad: [TermCell; 4]) -> Field { quad_hash([quad[0].enc(), quad[1].enc(), quad[2].enc(), quad[3].enc()]) }
fn sorted() -> [[TermCell; 4]; 2] {
    let a = vector_quad(); let b = other_quad();
    if field_lt(q(a), q(b)) { [a, b] } else { [b, a] }
}
fn config() -> [u128; 2] { [0x09090909090909090909090909090909; 2] }

#[test]
fn rust_noir_vectors_agree() {
    let quad = vector_quad();
    assert(quad[2].bound() & (quad[2].kind() == 3) & (quad[2].key_class() == 1));
    assert(quad[2].datatype() == crate::suite_constants::DT_DECIMAL);
    assert(quad[2].language() == 0x1a3838163a3bfceec91def0fb1c526eed692b8c508b6c0198aa96104499158ea);
    assert(quad[2].credential() == 0);
    assert(other_quad()[2].language() == 0x17d59821989b11bc15307e5a2c15991c2b3fadcbc218819dda6531c1300bcc7f);
    let checked = recompute_key(quad[2], BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("1250.50".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()), empty());
    assert(checked.cell().key() == 0x000180000000000000000000000000000000000000000043ca2c749129fa0000);
    assert(quad[2].enc() == 0x257a37dea6179751be35123aba062b9ab79b8f26a4a6aa49901f1f313c0e6861);
    assert(q(quad) == 0x18b29490f68c4af6b3b2d96e0824344927f4f541e6dec5bb2802f3e288c5c1a2);
    let l = leaf(q(quad), 7, 0);
    assert(l == 0x0a6bb130a5e4691b0f13a1abe83604e81f62a55e3c0f91ef15c859dee31c83ff);
    assert(credential_root([quad], [leaf_salt(7, 0)], 7, 1) == l);
    assert(signed_message(7, 1, l, config(), 0) == 0x11d3772b6e07a2d71fb63ccef479fc94c43e6c8837cf7e387737043504aed322);
}

#[test]
fn hash_is_bucket_independent_and_length_bound() {
    let a = hash_bytes(TERM_DOMAIN, BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("abc".as_bytes()));
    let b = hash_bytes(TERM_DOMAIN, BoundedVec::<u8, crate::suite_constants::LEXICAL_MEDIUM>::from_array("abc".as_bytes()));
    let c = hash_bytes(TERM_DOMAIN, BoundedVec::<u8, crate::suite_constants::LEXICAL_LONG>::from_array("abc".as_bytes()));
    assert((a == b) & (b == c));
    assert(a != hash_bytes(TERM_DOMAIN, BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array([97, 98, 99, 0])));
}
#[test]
fn tree_root_is_capacity_independent() {
    let qs = sorted();
    let root = credential_root(qs, [leaf_salt(7, 0), leaf_salt(7, 1)], 7, 2);
    let mut larger = [[padding(); 4]; 4]; larger[0] = qs[0]; larger[1] = qs[1];
    assert(credential_root(larger, [leaf_salt(7, 0), leaf_salt(7, 1), 0, 0], 7, 2) == root);
}
#[test(should_fail_with = "key disagrees with lexical form")]
fn key_disagrees_with_lexical_form() {
    let mut key = decimal_key(); key[30] = key[30] + 1;
    let _checked = recompute_key(decimal(key), BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("1250.50".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()), empty());
}
#[test(should_fail_with = "class disagrees with datatype")]
fn class_disagrees_with_datatype() {let mut key=[0;31];key[0]=5;key[1]=1;let _cell=decimal(key);}

#[test(should_fail_with = "quads not strictly increasing")]
fn duplicated_quad() { let _root = credential_root([vector_quad(); 2], [leaf_salt(7, 0), leaf_salt(7, 1)], 7, 2); }
#[test(should_fail_with = "quads not strictly increasing")]
fn quads_out_of_order() { let qs = sorted(); let _root = credential_root([qs[1], qs[0]], [leaf_salt(7, 0), leaf_salt(7, 1)], 7, 2); }
#[test(should_fail_with = "non-zero padding")]
fn non_zero_padding() { let _root = credential_root([vector_quad(), other_quad()], [leaf_salt(7, 0), 0], 7, 1); }
#[test(should_fail_with = "wrong count")]
fn wrong_count() { let _root = credential_root([vector_quad()], [leaf_salt(7, 0)], 7, 2); }
#[test(should_fail_with = "leaf salt not derived from tree salt")]
fn leaf_salt_not_derived_from_tree_salt() { let _root = credential_root([vector_quad()], [leaf_salt(8, 0)], 7, 1); }
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn message_for_another_suite() {
    // A valid host signature over the same credential under SUITE_ID + 1.
    let pk = Point { x: 0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9, y: 0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55 };
    check_credential_signature([vector_quad()], [leaf_salt(7, 0)], 7, 1, config(), pk,
        Point { x: 0x226941183795847bf36b37ef2b456218f28cbf533914a6f4194d7f4a45a82f09, y: 0x002c9988ac826801903fa122e21f9777cc5ad9bbbcb5f9f85f8297a9a22dac4b },
        0x0061773cf382ce799abc4f52f1cc2dfb9260a3fcd0efbce60a47174a8a7c45a1, 0x013bd5e108900a7a980c539b88989cba0304fca9478cbafb797c2dccbb8b7d89, 0x0000000000000000000000000000000000000000000000000000000000000000, 0);
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn message_for_another_configuration() {
    let c = config();
    signature_check_config(false, [c[0], c[1] + 1]);
}
#[test(should_fail_with = "term kinds are distinct")]
fn blank_node_label_reused_across_term_kinds() {
    assert(blank("same".as_bytes()).enc() == iri("same".as_bytes()).enc(), "term kinds are distinct");
}

fn signature_check(other: bool) { signature_check_config(other, config()); }
fn signature_check_config(other: bool, configuration: [u128; 2]) {
    let pk = if other { Point { x: 0x10e2b0752a9934ad28c9b07eeaa19a511df7bd67051d2b03c0e9e4b51f33c1f6, y: 0x0ac6e1123840c217fcf07ef14dacccb4381e5d732458f79a425dfe6af6c17d75 } } else { Point { x: 0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9, y: 0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55 } };
    check_credential_signature([vector_quad()], [leaf_salt(7, 0)], 7, 1, configuration, pk, Point { x: 0x170300f0746f4d6c1f4422a8cac16b22790035ecddbad2c8abe7cf5b936d29af, y: 0x0b35e5d8ef4bd612bb739350ffe310a331c4a7502cab97d9af61b8ed601a8ed7 }, 0x054a151286c38c28dc21978b687264e580f3353e656e4a379eb612f9f5e36951, 0x0372c4cfb3cc6840517166be45877f505be4f3163d22ef4888f4eb334b557d30, 0x0000000000000000000000000000000000000000000000000000000000000005, 0);
}
#[test]
fn honest_hidden_signature() { signature_check(false); }
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn signature_under_another_key() { signature_check(true); }

#[test]
fn odd_count_uses_zero_leaf_inside_the_minimal_tree() {
    let a = vector_quad();
    let b = other_quad();
    let mut c = b; c[2] = iri("urn:extra".as_bytes());
    let mut qs = [a, b, c, [padding(); 4]];
    for _i in 0..3 {
        for j in 0..2 {
            if field_lt(q(qs[j + 1]), q(qs[j])) { let swap = qs[j]; qs[j] = qs[j + 1]; qs[j + 1] = swap; }
        }
    }
    assert(credential_root(qs, [leaf_salt(7, 0), leaf_salt(7, 1), leaf_salt(7, 2), 0], 7, 3) == 0x23cc71e43d022f09c9184c75f607ca041f0c3d530ab726c141352cb821eedd5e);
    let c: [u128;2] = [0x000102030405060708090a0b0c0d0e0f, 0x101112131415161718191a1b1c1d1e1f];
    let r11 = credential_root(qs, [leaf_salt(11, 0),leaf_salt(11, 1),leaf_salt(11, 2),0],11,3);
    assert(signed_message(11,3,r11,c, 0) == 0x2daebcd97f9087cf1bc7a36c09a5ecebe46dfb520b1ae6a56d7bb7672e0b22fb);
    let l0 = leaf(q(qs[0]), 7, 0);
    let l1 = leaf(q(qs[1]), 7, 1);
    let l2 = leaf(q(qs[2]), 7, 2);
    let node = crate::suite_constants::NODE_DOMAIN;
    let left = Poseidon2::hash([node, l0, l1], 3);
    let right = Poseidon2::hash([node, l2, 0], 3);
    assert(credential_root(qs, [leaf_salt(7, 0), leaf_salt(7, 1), leaf_salt(7, 2), 0], 7, 3) == Poseidon2::hash([node, left, right], 3));
}
#[test(should_fail_with = "term kinds are distinct")]
fn blank_node_label_reused_as_literal() {
    let mut key = [0; 31]; key[0] = 6; key[1] = 115; key[2] = 97; key[3] = 109; key[4] = 101;
    let literal = open_term(3, BoundedVec::<u8, crate::suite_constants::LEXICAL_SHORT>::from_array("same".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()), empty(), key, 0);
    assert(blank("same".as_bytes()).enc() == literal.enc(), "term kinds are distinct");
}

#[test]
fn salt_and_configuration_halves_match_independent_host_vectors() {
    let q = q(vector_quad());
    assert(leaf(q, 11, 0) == 0x2f7b881412bf40ffb82670cdfa90b9bbe843cdc885573ba25fc66a619450f3f8);
    let c: [u128;2] = [0x000102030405060708090a0b0c0d0e0f, 0x101112131415161718191a1b1c1d1e1f];
    let root = leaf(q, 7, 0);
    assert(signed_message(7, 1, root, c, 0) == 0x1057f46ab8be80dbf2268dc2211e11eee6c52e9d424088882ac324c7b206288e);
    assert(signed_message(7, 1, root, c, 0) != signed_message(7, 1, root, [c[1],c[0]], 0));
}
#[test(should_fail_with = "count differs")]
fn message_for_another_count() {
    let root = leaf(q(vector_quad()), 7, 0);
    assert(signed_message(7, 1, root, config(), 0) == signed_message(7, 2, root, config(), 0), "count differs");
}

#[test(should_fail_with = "wrong count")]
fn count_exceeds_capacity() { let _root = credential_root([vector_quad()], [leaf_salt(7,0)], 7, 2); }
#[test(should_fail_with = "wrong count")]
fn count_is_zero() { let _root = credential_root([[padding();4]], [0], 7, 0); }
#[test]
fn blank_node_scope_is_credential_local() {
    let text: BoundedVec<u8,32> = BoundedVec::from_array("same".as_bytes());
    let a = open_term(2,text,empty(),empty(),[0;31],1);
    let b = open_term(2,text,empty(),empty(),[0;31],2);
    assert(a.enc() == b.enc());
    assert((a.credential() == 1) & (b.credential() == 2));
    assert((a.kind() == 2) & (a.key() == 0) & (a.datatype() == 0) & (a.language() == 0));
    let iri = open_term(1,text,empty(),empty(),[0;31],2);
    assert(iri.credential() == 0);
}

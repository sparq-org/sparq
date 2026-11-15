use std::collections::bounded_vec::BoundedVec;
use crate::term::{TermCell, open_term, same_term};
use crate::suite_entry::BoundedTermOpening;

fn empty() -> BoundedVec<u8, 1> { BoundedVec::new() }
fn resource<let L: u32>(kind: u8, text: [u8; L], scope: u32) -> TermCell {
    open_term(kind, BoundedVec::<u8, L>::from_array(text), empty(), empty(), [0; 31], scope)
}
fn literal<let L: u32, let D: u32, let G: u32>(
    text: [u8; L], datatype: [u8; D], language: [u8; G], key: [u8; 31],
) -> TermCell {
    open_term(3, BoundedVec::<u8, L>::from_array(text), BoundedVec::<u8, D>::from_array(datatype),
        BoundedVec::<u8, G>::from_array(language), key, 0)
}
fn string_key<let L: u32>(text: [u8; L]) -> [u8; 31] {
    std::static_assert(L <= 30, "test string capacity");
    let mut key = [0; 31]; key[0] = 6;
    for i in 0..L { key[i + 1] = text[i]; } key
}
fn decimal_key(extra: u8) -> [u8; 31] {
    let value: Field = 0x000180000000000000000000000000000000000000000043ca2c749129fa0000;
    let bytes = value.to_be_bytes::<32>();
    let mut key = [0; 31]; for i in 0..31 { key[i] = bytes[i + 1]; }
    key[30] = key[30] + extra; key
}
fn decimal(extra: u8) -> TermCell {
    literal("1250.50".as_bytes(), "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(),
        "".as_bytes(), decimal_key(extra))
}

#[test]
fn rust_noir_identity_vectors_agree() {
    let iri = resource(1, "urn:identity".as_bytes(), 0);
    let blank = resource(2, "urn:identity".as_bytes(), 1);
    let plain = literal("example".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
        "".as_bytes(), string_key("example".as_bytes()));
    let typed = literal("7".as_bytes(), "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(),
        "".as_bytes(), [0; 31]);
    let lang = literal("example".as_bytes(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(),
        "en".as_bytes(), [0; 31]);
    let default = open_term(0, empty(), empty(), empty(), [0; 31], 0);
    assert(iri.identity() == 0x2a5e074c81ce77c212de6b53d7d58a8efe587f3a3aba6a0be14513ad46b8d815);
    assert(blank.identity() == 0x1b0322149f623004e1e54012986c7420547211a68d62fe4e4341e084d5525713);
    assert(plain.identity() == 0x218d80174c06b3298f25e7676009a5146d2f68e1dbbdc09b0f65ca8cbd6926da);
    assert(typed.identity() == 0x1762ddd77dfb6cdc515b4054df4aa83eeceb93f9affca790c49057ed5af01f48);
    assert(lang.identity() == 0x0ba25beef8d00e064985c6f38ae3edc9e56581a55c5c8e8197b9294840e15b40);
    assert(default.identity() == 0x14acf45f4341c42ed4a746a67fefb355e7f484690e941d3d778167a9457090aa);
}

#[test]
fn identity_binds_datatype() {
    let a = literal("example".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
        "".as_bytes(), string_key("example".as_bytes()));
    let b = literal("example".as_bytes(), "http://www.w3.org/2001/XMLSchema#other".as_bytes(),
        "".as_bytes(), [0; 31]);
    assert(a.identity() != b.identity()); assert(!same_term(a, b));
}
#[test]
fn identity_binds_language() {
    let a = literal("example".as_bytes(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(),
        "en".as_bytes(), [0; 31]);
    let b = literal("example".as_bytes(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(),
        "fr".as_bytes(), [0; 31]);
    assert(a.identity() != b.identity()); assert(!same_term(a, b));
}
#[test]
fn identity_binds_kind() {
    let iri = resource(1, "urn:identity".as_bytes(), 0);
    let blank = resource(2, "urn:identity".as_bytes(), 0);
    assert(iri.identity() != blank.identity()); assert(!same_term(iri, blank));
    assert(iri.identity() == 0x2a5e074c81ce77c212de6b53d7d58a8efe587f3a3aba6a0be14513ad46b8d815);
}
#[test]
fn identity_binds_lexical() {
    assert(resource(1, "urn:identity".as_bytes(), 0).identity()
        != resource(1, "urn:Identity".as_bytes(), 0).identity());
    let a = literal("example".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
        "".as_bytes(), string_key("example".as_bytes()));
    let b = literal("Example".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
        "".as_bytes(), string_key("Example".as_bytes()));
    assert(a.identity() != b.identity());
}
#[test]
fn identity_preserves_scope_and_canonical_padding() {
    let a = resource(2, "urn:identity".as_bytes(), 1);
    let b = resource(2, "urn:identity".as_bytes(), 2);
    assert(a.identity() == b.identity()); assert(!same_term(a, b));
    let pad = crate::term::padding(); pad.assert_padding(); assert(pad.identity() == 0);
    let default = open_term(0, empty(), empty(), empty(), [0; 31], 0);
    assert(default.identity() == crate::suite_constants::DEFAULT_GRAPH_ENC);
    assert(!same_term(default, pad));
    let graph = crate::term::credential_graph([11, 13], 1);
    assert(graph.identity() == graph.enc());
    assert(graph.identity() != crate::term::credential_graph([11, 13], 2).identity());
}
fn raw_integer() -> BoundedTermOpening<8, 64, 1> {
    let mut lexical = [0; 8]; lexical[0] = 55;
    let dt = "http://www.w3.org/2001/XMLSchema#integer".as_bytes();
    let mut datatype = [0; 64]; for i in 0..dt.len() { datatype[i] = dt[i]; }
    let (_, field) = crate::term_key::recompute(BoundedVec::<u8, 8>::from_array([55]), crate::suite_constants::DT_INTEGER);
    let bytes = field.to_be_bytes::<32>(); let mut key = [0; 31];
    for i in 0..31 { key[i] = bytes[i + 1]; }
    BoundedTermOpening { bound: true, kind: 3, lexical, lexical_len: 1, datatype, datatype_len: dt.len(),
        language: [0; 1], language_len: 0, key }
}
#[test]
fn bounded_and_checked_cells_keep_the_constructed_identity() {
    let raw = raw_integer(); let opened = crate::suite_entry::open_bounded_cell(raw, 0);
    let checked = crate::term::open_key_checked(raw, 0, crate::suite_constants::DT_INTEGER).cell();
    let recomputed = crate::term::recompute_key_for(opened, BoundedVec::<u8, 8>::from_array([55]),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#integer".as_bytes()),
        empty(), crate::suite_constants::DT_INTEGER).cell();
    assert((opened.identity() == checked.identity()) & (checked.identity() == recomputed.identity()));
    let resource = resource(1, "urn:identity".as_bytes(), 0);
    let specialised = crate::term::open_nonliteral_term::<12, 1, 1, 0>(1,
        BoundedVec::from_array("urn:identity".as_bytes()), empty(), empty(), [0; 31], 0);
    assert(resource.identity() == specialised.identity());
    let graph = crate::term::open_nonliteral_term::<1, 1, 1, 3>(0, empty(), empty(), empty(), [0; 31], 0);
    assert(graph.identity() == crate::suite_constants::DEFAULT_GRAPH_ENC);
}
#[test(should_fail_with = "key disagrees with lexical form")]
fn identity_does_not_mark_a_key_checked() {
    let cell = decimal(1);
    let _checked = crate::term::recompute_key_for(cell, BoundedVec::<u8, 8>::from_array("1250.50".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()),
        empty(), crate::suite_constants::DT_DECIMAL);
}

#[test]
fn issuer_signed_keys_do_not_change_term_identity() {
    let a = decimal(0); let b = decimal(1);
    assert(a.key() != b.key()); assert(a.enc() != b.enc());
    assert(a.identity() == b.identity()); assert(same_term(a, b));
    let subject = resource(2, "c14n0".as_bytes(), 1);
    let predicate = resource(1, "http://ex/balance".as_bytes(), 0);
    let graph = open_term(0, empty(), empty(), empty(), [0; 31], 0);
    crate::suite::check_credential_signature([[subject, predicate, a, graph]],
        [crate::suite::leaf_salt(7, 0)], 7, 1, [0x09090909090909090909090909090909; 2],
        compose::issuer::Point { x: 0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9, y: 0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55 },
        compose::issuer::Point { x: 0x170300f0746f4d6c1f4422a8cac16b22790035ecddbad2c8abe7cf5b936d29af, y: 0x0b35e5d8ef4bd612bb739350ffe310a331c4a7502cab97d9af61b8ed601a8ed7 },
        0x054a151286c38c28dc21978b687264e580f3353e656e4a379eb612f9f5e36951,
        0x0372c4cfb3cc6840517166be45877f505be4f3163d22ef4888f4eb334b557d30,
        0x0000000000000000000000000000000000000000000000000000000000000005, 0);
    crate::suite::check_credential_signature([[subject, predicate, b, graph]],
        [crate::suite::leaf_salt(7, 0)], 7, 1, [0x09090909090909090909090909090909; 2],
        compose::issuer::Point { x: 0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9, y: 0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55 },
        compose::issuer::Point { x: 0x1e0ea2e3a8e4119b95780e2d7acbc917988d5c2a7a3c30931622323be50ac729, y: 0x00e262260f92f96730dac27edd9b23c6f9a9be2a13121113264fca50af3e72d4 },
        0x0244eda829f9d7f03674848b58db367c1d5ea630190a74373e3fe941f418c955,
        0x01d283f8bc59997c7a0eb5132f49b1fa0184cf3ef36b27e293c82a8289b8d21b,
        0x0000000000000000000000000000000000000000000000000000000000000002, 0);
}
fn assert_fixed<let L: u32>(fixed: TermCell, text: [u8; L]) {
    let opened = resource(1, text, 0);
    assert((fixed.identity() == opened.identity()) & (fixed.enc() == opened.enc()));
}
#[test]
fn fixed_vocabulary_identities_match_constrained_openings() {
    assert_fixed(crate::term::iri_rdf_type(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes());
    assert_fixed(crate::term::iri_vc_type(), "https://www.w3.org/2018/credentials#VerifiableCredential".as_bytes());
    assert_fixed(crate::term::iri_valid_from(), "https://www.w3.org/2018/credentials#validFrom".as_bytes());
    assert_fixed(crate::term::iri_valid_until(), "https://www.w3.org/2018/credentials#validUntil".as_bytes());
    assert_fixed(crate::term::iri_issuer(), "https://www.w3.org/2018/credentials#issuer".as_bytes());
    assert_fixed(crate::term::iri_subject(), "https://www.w3.org/2018/credentials#credentialSubject".as_bytes());
    assert_fixed(crate::term::iri_multikey(), "https://w3id.org/security#Multikey".as_bytes());
    assert_fixed(crate::term::iri_public_key(), "https://w3id.org/security#publicKeyMultibase".as_bytes());
    assert_fixed(crate::term::iri_credential_status(), "https://www.w3.org/2018/credentials#credentialStatus".as_bytes());
    assert_fixed(crate::term::iri_status_entry(), "https://www.w3.org/ns/credentials/status#BitstringStatusListEntry".as_bytes());
    assert_fixed(crate::term::iri_status_list(), "https://www.w3.org/ns/credentials/status#statusListCredential".as_bytes());
    assert_fixed(crate::term::iri_status_index(), "https://www.w3.org/ns/credentials/status#statusListIndex".as_bytes());
    assert_fixed(crate::term::iri_status_purpose(), "https://www.w3.org/ns/credentials/status#statusPurpose".as_bytes());
    assert_fixed(crate::term::iri_status_size(), "https://www.w3.org/ns/credentials/status#statusSize".as_bytes());
    assert_fixed(crate::term::iri_assertion_method(), "https://w3id.org/security#assertionMethod".as_bytes());
    assert_fixed(crate::term::iri_confidence_method(), "https://www.w3.org/2018/credentials#confidenceMethod".as_bytes());
    assert_fixed(crate::term::iri_verified_issuer(), "https://w3id.org/sparq/vcq#verifiedIssuer".as_bytes());
}

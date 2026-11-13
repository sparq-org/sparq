//! Host-reproducible authenticated metadata vectors; no free metadata witnesses.
use std::collections::bounded_vec::BoundedVec;
use crate::suite_entry::TermOpening;
use crate::suite_metadata::{open_credential_hidden, OpenedCredential, multikey_cell, open_multikey, open_status_index, open_status_purpose};
use compose::issuer::Point;
fn raw<let C:u32,let L:u32,let D:u32>(kind:u8,lex:[u8;L],dt:[u8;D],key:Field)->TermOpening<C> {
    let mut lexical=[0;C];for i in 0..L { lexical[i]=lex[i]; }
    let mut datatype=[0;128];for i in 0..D { datatype[i]=dt[i]; }
    let full=key.to_be_bytes::<32>();let mut bytes=[0;31];for i in 0..31 {bytes[i]=full[i+1];}
    TermOpening {bound:true,kind,lexical,lexical_len:L,datatype,datatype_len:D,language:[0;64],language_len:0,key:bytes}
}
fn absent<let C:u32>()->TermOpening<C> { TermOpening {bound:false,kind:0,lexical:[0;C],lexical_len:0,datatype:[0;128],datatype_len:0,language:[0;64],language_len:0,key:[0;31]} }
fn quads()->[[TermOpening<32>;4];1] { [[raw::<32,5,0>(2,"c14n0".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<32,17,0>(1,"http://ex/balance".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<32,7,40>(3,"1250.50".as_bytes(),"http://www.w3.org/2001/XMLSchema#decimal".as_bytes(),0x0180000000000000000000000000000000000000000043ca2c749129fa0000),raw::<32,0,0>(0,"".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000)]] }
fn created()->TermOpening<128> {raw::<128,20,41>(3,"2026-01-01T00:00:00Z".as_bytes(),"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(),0x038000019b76daa80000000000000000000000000000000000000000000000)}
fn method()->TermOpening<128> {raw::<128,10,0>(1,"urn:key:19".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000)}
fn pk()->Point {Point {x:0x1bb873421a895ca8c8d9fc0b44dd35be1d65592564942523f49590264f03d5a9,y:0x0d9fd8a464e1fc3e79175f0729533a1b865b70848c2f90d9c037d4d6949c9b55}}
fn fixture_method(created_raw:TermOpening<128>,key:Point,method_raw:TermOpening<128>)->OpenedCredential<1> { open_credential_hidden(quads(),[0x10d27ae5747a328634d7dd2941ebeeb79f1e938054c9c7e1cab289b2adf5bbb7],7,1,[0x09090909090909090909090909090909;2],created_raw,method_raw,key,0,Point { x:0x270d497e79fbe3b7574d1fc62b211e0009ba391e1c8e12c1175632f2fb5afdba,y:0x164b448b1476688eeddfc1359220d79e0c2ed96a694bdd84114fcc63d0a01d0b },0x03203196df02ebfe53e3fa449595f2c334699ac289cb3597b8a1dade2f18df0f,0x03b86d67c45349ac68136ab7c56ee819e85f3c2e4a60f2f1150c18ab98ade013,0x0000000000000000000000000000000000000000000000000000000000000004) }
fn fixture(created_raw:TermOpening<128>,key:Point)->OpenedCredential<1> {fixture_method(created_raw,key,method())}

#[test]
fn authenticated_metadata_matches_host() {
 let c=fixture(created(),pk());assert(c.created().known());assert(c.created().millis()==1767225600000);
 assert(c.message()==0x0f25cc526305bf6fbfe61ec480784974fb3c2d81ab429a91e3bbdc96f601ef9c);assert(c.verification_key().enc()==0x04c7c69ab89257002a1e343b3c466c7416ca11e9edef3714567f27bad5494b29);
 assert(c.root()==0x0a6bb130a5e4691b0f13a1abe83604e81f62a55e3c0f91ef15c859dee31c83ff);
 assert(c.count()==1);assert(c.quads()[0][0].credential()==1);
 assert(!c.valid_from().bound() & !c.valid_until().bound());
}
#[test]
fn absent_creation_is_authenticated_unknown() {
 let c=open_credential_hidden(quads(),[0x10d27ae5747a328634d7dd2941ebeeb79f1e938054c9c7e1cab289b2adf5bbb7],7,1,[0x09090909090909090909090909090909;2],absent::<128>(),method(),pk(),0,Point { x:0x22f1b44ef76c0c0ae40f472e6a9bf9988991890608e8d6f2f2ed8b9d5ffce315,y:0x03923aabf53092773ca694720811abc2b142aa31825a037d82a1db2d04ddfd6e },0x045d91d1b1df398992b0f7d7fa594d364cf5f7a33850494cae713339f6f08d15,0x00d66638d3a7340cf7a9394012e5bbdcf48da5b7c81fcbc3941a908062725be4,0x0000000000000000000000000000000000000000000000000000000000000004);
 assert(!c.created().known() & (c.created().millis()==0));assert(!c.created_at().bound());assert(c.message()==0x12b7346eadbc2bf1aef18f1d3c6603e0ad41ec46e7b1d2e2f23c4cb2765707a9);
}
#[test(should_fail_with="challenge reduction not bound to its preimage")]
fn wrong_created_instant() { let mut changed=created();changed.lexical[9]=50;
 let key=crate::term_key::recompute(BoundedVec::<u8,128>::from_parts_unchecked(changed.lexical,changed.lexical_len),crate::suite_constants::DT_DATETIME).1.to_be_bytes::<32>();
 for i in 0..31 {changed.key[i]=key[i+1];} let _opened=fixture(changed,pk()); }
#[test(should_fail_with="challenge reduction not bound to its preimage")]
fn wrong_authenticated_key() { let _opened=fixture(created(),Point {x:0x10e2b0752a9934ad28c9b07eeaa19a511df7bd67051d2b03c0e9e4b51f33c1f6,y:0x0ac6e1123840c217fcf07ef14dacccb4381e5d732458f79a425dfe6af6c17d75}); }
#[test(should_fail_with="verification key opening differs")]
fn wrong_multikey_opening() {open_multikey(multikey_cell(pk()),Point {x:0x10e2b0752a9934ad28c9b07eeaa19a511df7bd67051d2b03c0e9e4b51f33c1f6,y:0x0ac6e1123840c217fcf07ef14dacccb4381e5d732458f79a425dfe6af6c17d75});}
#[test]
fn scoped_graph_matches_host() {let g=crate::term::credential_graph([11,13],1);assert(g.enc()==crate::suite_constants::GRAPH_VECTOR);assert((g.kind()==2)&(g.credential()==2));assert(!crate::term::same_term(g,crate::term::credential_graph([11,13],2)));}
fn string<let L:u32>(lex:[u8;L])->crate::term::TermCell {
 let text=BoundedVec::<u8,L>::from_array(lex);
 let bytes=crate::term_key::recompute(text,crate::suite_constants::DT_STRING).1.to_be_bytes::<32>();let mut key=[0;31];for i in 0..31 {key[i]=bytes[i+1];}
 crate::term::open_term(3,text,BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()),BoundedVec::<u8,1>::new(),key,0)
}
#[test]
fn status_strings_have_exact_values() {
 assert(open_status_index(string("0007".as_bytes()),BoundedVec::<u8,32>::from_array("0007".as_bytes()))==7);
 assert(!open_status_purpose(string("revocation".as_bytes()),BoundedVec::<u8,32>::from_array("revocation".as_bytes())));
 assert(open_status_purpose(string("suspension".as_bytes()),BoundedVec::<u8,32>::from_array("suspension".as_bytes())));
}
#[test(should_fail_with="lexical opening differs")]
fn wrong_index_string() { let _index=open_status_index(string("0007".as_bytes()),BoundedVec::<u8,32>::from_array("0008".as_bytes())); }
#[test(should_fail_with="lexical opening differs")]
fn wrong_purpose_string() { let _purpose=open_status_purpose(string("revocation".as_bytes()),BoundedVec::<u8,32>::from_array("suspension".as_bytes())); }
#[test(should_fail)]
fn index_string_overflows() { let _index=open_status_index(string("18446744073709551616".as_bytes()),BoundedVec::<u8,32>::from_array("18446744073709551616".as_bytes())); }
#[test(should_fail_with="status index is not decimal")]
fn index_string_is_signed() { let _index=open_status_index(string("+7".as_bytes()),BoundedVec::<u8,32>::from_array("+7".as_bytes())); }
#[test(should_fail_with="lexical opening differs")]
fn lexical_opening_differs_beyond_key_prefix() {
 let cell=string("abcdefghijklmnopqrstuvwxyz1234-A".as_bytes());
 let _checked=crate::term::recompute_key_for(cell,BoundedVec::<u8,64>::from_array("abcdefghijklmnopqrstuvwxyz1234-B".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()),BoundedVec::<u8,1>::new(),crate::suite_constants::DT_STRING);
}
fn bound_quads()->[[TermOpening<128>;4];4] { [[raw::<128,6,0>(1,"urn:vc".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,47,0>(1,"http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,56,0>(1,"https://www.w3.org/2018/credentials#VerifiableCredential".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,0,0>(0,"".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000)],[raw::<128,6,0>(1,"urn:vc".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,46,0>(1,"https://www.w3.org/2018/credentials#validUntil".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,20,41>(3,"2027-01-01T00:00:00Z".as_bytes(),"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(),0x03800001a2ce8bd40000000000000000000000000000000000000000000000),raw::<128,0,0>(0,"".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000)],[raw::<128,6,0>(1,"urn:vc".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,45,0>(1,"https://www.w3.org/2018/credentials#validFrom".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000),raw::<128,20,41>(3,"2026-01-01T00:00:00Z".as_bytes(),"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(),0x038000019b76daa80000000000000000000000000000000000000000000000),raw::<128,0,0>(0,"".as_bytes(),"".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000)],[absent::<128>(),absent::<128>(),absent::<128>(),absent::<128>()]] }
#[test]
fn validity_bounds_and_credential_subject_are_authenticated() {
let c=crate::suite_metadata::open_credential_revealed(bound_quads(),[0x10d27ae5747a328634d7dd2941ebeeb79f1e938054c9c7e1cab289b2adf5bbb7,0x1bf8956e78ed38c2f76fcddb9047792ee4569cc3ac28d2c56d4590097788e51e,0x2b2a75bf91866bde22a7cc04877899db42fc6d48553d2c8077e2d9ea8c00d0ee,0x0000000000000000000000000000000000000000000000000000000000000000],7,3,[0x09090909090909090909090909090909;2],absent::<128>(),method(),pk(),0);
assert(c.message()==0x2eb2ed8596261940cde244965300022d3985b5e8809b36035ef1150db786e925);assert(c.credential_subject().bound());
assert(crate::suite_metadata::open_time(c.valid_from(),BoundedVec::<u8,32>::from_array("2026-01-01T00:00:00Z".as_bytes())).millis()==1767225600000);
assert(crate::suite_metadata::open_time(c.valid_until(),BoundedVec::<u8,32>::from_array("2027-01-01T00:00:00Z".as_bytes())).millis()==1798761600000);
}

#[test(should_fail_with="non-zero padding")]
fn non_zero_padding_salt() {
    let a=quads();let cells=crate::suite_entry::open_quads(a,1);
    let _root=crate::suite::credential_root([cells[0],[crate::term::padding();4]],[crate::suite::leaf_salt(7,0),1],7,1);
}
#[test(should_fail_with="non-zero byte padding")]
fn non_zero_lexical_padding() {
    let _cell=crate::term::open_term(1,BoundedVec::<u8,2>::from_parts_unchecked([65,66],1),BoundedVec::<u8,1>::new(),BoundedVec::<u8,1>::new(),[0;31],0);
}
#[test(should_fail_with="non-zero padding")]
fn non_zero_raw_padding() {let mut a=absent::<32>();a.lexical[0]=65;let _cell=crate::suite_entry::open_cell(a,0);}
#[test(should_fail_with="wrong count")]
fn unbound_graph_in_counted_quad() {let mut a=crate::suite_entry::open_quads(quads(),1);a[0][3]=crate::term::padding();let _root=crate::suite::credential_root(a,[crate::suite::leaf_salt(7,0)],7,1);}
#[test(should_fail_with="salt differs")]
fn signed_message_binds_tree_salt() {assert(crate::suite::signed_message(7,1,99,[9,9],0)==crate::suite::signed_message(8,1,99,[9,9],0),"salt differs");}
#[test(should_fail_with="datatype specialization differs")]
fn datatype_specialization_must_match() {
    let text=BoundedVec::<u8,32>::from_array("1".as_bytes());
    let full=crate::term_key::recompute(text,crate::suite_constants::DT_DECIMAL).1.to_be_bytes::<32>();
    let mut key=[0;31];for i in 0..31 {key[i]=full[i+1];}
    let dt=BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#decimal".as_bytes());let lang=BoundedVec::<u8,1>::new();
    let cell=crate::term::open_term(3,text,dt,lang,key,0);
    let _checked=crate::term::recompute_key_for(cell,text,dt,lang,crate::suite_constants::DT_INTEGER);
}
#[test(should_fail_with="non-literal metadata")]
fn iri_cannot_carry_a_key() {let mut key=[0;31];key[1]=1;let _cell=crate::term::open_term(1,BoundedVec::<u8,1>::from_array([65]),BoundedVec::<u8,1>::new(),BoundedVec::<u8,1>::new(),key,0);}
#[test(should_fail_with="language datatype")]
fn language_identifier_requires_language_tag() {
 let _cell=crate::term::open_term(3,BoundedVec::<u8,3>::from_array("abc".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes()),BoundedVec::<u8,1>::new(),[0;31],0);
}
#[test(should_fail_with="status index is not decimal")]
fn index_string_is_non_decimal() {let _index=open_status_index(string("a".as_bytes()),BoundedVec::<u8,32>::from_array("a".as_bytes()));}
#[test(should_fail_with="empty status index")]
fn empty_status_index() {let _index=open_status_index(string("".as_bytes()),BoundedVec::<u8,32>::new());}
#[test(should_fail_with="unknown status purpose")]
fn unknown_status_purpose() {let _purpose=open_status_purpose(string("other".as_bytes()),BoundedVec::<u8,32>::from_array("other".as_bytes()));}
#[test]
fn graph_name_binds_both_digest_halves() {
 let a=crate::term::credential_graph([11,13],1);
 assert(a.enc()!=crate::term::credential_graph([12,13],1).enc());
 assert(a.enc()!=crate::term::credential_graph([11,14],1).enc());
}
#[test(should_fail_with="challenge reduction not bound to its preimage")]
fn changed_term_cannot_keep_the_signature() {
 let mut raw=quads();raw[0][2].lexical[3]=49;
 let bytes=crate::term_key::recompute(BoundedVec::<u8,32>::from_parts_unchecked(raw[0][2].lexical,raw[0][2].lexical_len),crate::suite_constants::DT_DECIMAL).1.to_be_bytes::<32>();
 for i in 0..31 {raw[0][2].key[i]=bytes[i+1];}
 let _c=open_credential_hidden(raw,[crate::suite::leaf_salt(7,0)],7,1,[0x09090909090909090909090909090909;2],created(),method(),pk(),0,
Point {x:0x270d497e79fbe3b7574d1fc62b211e0009ba391e1c8e12c1175632f2fb5afdba,y:0x164b448b1476688eeddfc1359220d79e0c2ed96a694bdd84114fcc63d0a01d0b},0x03203196df02ebfe53e3fa449595f2c334699ac289cb3597b8a1dade2f18df0f,0x03b86d67c45349ac68136ab7c56ee819e85f3c2e4a60f2f1150c18ab98ade013,0x0000000000000000000000000000000000000000000000000000000000000004);
}

#[test(should_fail_with="challenge reduction not bound to its preimage")]
fn wrong_verification_method_iri() {let mut m=method();m.lexical[9]=48;let _c=fixture_method(created(),pk(),m);}
#[test]
fn unknown_creation_is_not_zero_instant() {
 let created_raw=raw::<128,7,41>(3,"unknown".as_bytes(),"http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(),0x00000000000000000000000000000000000000000000000000000000000000);
 let c=open_credential_hidden(quads(),[0x10d27ae5747a328634d7dd2941ebeeb79f1e938054c9c7e1cab289b2adf5bbb7],7,1,[0x09090909090909090909090909090909;2],created_raw,method(),pk(),0,Point {x:0x1c6a7e6bb3be453d697e951c9c01ab40f4c8bfb3aafe147232c3adb24f925332,y:0x2637e6ce5123a2a68ebc84110c827d09eca54a9dbae1bc4b346863be6bd4f9bf},0x04824b4e450c9910177fbdbddccc3551579829fd6dd78a8d699345659021e854,0x035022704be0aa9dc66a3e0be0f4b90fc995b201ce281a0ebff9d132191ffe2d,0x0000000000000000000000000000000000000000000000000000000000000000);
 assert(c.created_at().bound());assert(!c.created().known() & (c.created().millis()==0));assert(c.message()==0x0d7c301527404dec67435e82efae23774433564ec93f68c7c612c2fc8e51fc54);
}
#[test]
fn metadata_frame_binds_the_verification_key() {
 let a=multikey_cell(pk());let b=multikey_cell(Point {x:0x10e2b0752a9934ad28c9b07eeaa19a511df7bd67051d2b03c0e9e4b51f33c1f6,y:0x0ac6e1123840c217fcf07ef14dacccb4381e5d732458f79a425dfe6af6c17d75});
 assert(crate::suite_metadata::metadata_digest(crate::term::padding(),a,crate::term::padding()) != crate::suite_metadata::metadata_digest(crate::term::padding(),b,crate::term::padding()));
}

#[test]
fn status_index_maximum_is_exact() {
 assert(open_status_index(string("18446744073709551615".as_bytes()),BoundedVec::<u8,32>::from_array("18446744073709551615".as_bytes()))==18446744073709551615);
}
#[test]
fn time_before_epoch_stays_signed() {
 let lexical=BoundedVec::<u8,32>::from_array("1969-12-31T00:00:00Z".as_bytes());
 let key=crate::term_key::recompute(lexical,crate::suite_constants::DT_DATETIME).1.to_be_bytes::<32>();let mut bytes=[0;31];for i in 0..31 {bytes[i]=key[i+1];}
 let cell=crate::term::open_term(3,lexical,BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#dateTime".as_bytes()),BoundedVec::<u8,1>::new(),bytes,0);
 let instant=crate::suite_metadata::open_time(cell,lexical);assert(instant.known());assert(instant.millis()==-86400000);
}

#[test(should_fail_with="empty key payload")]
fn unrepresentable_payload_is_zero() {
 let mut key=[0;31];key[0]=127;key[30]=1;
 let _cell=crate::term::open_term(3,BoundedVec::<u8,3>::from_array("NaN".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#double".as_bytes()),BoundedVec::<u8,1>::new(),key,0);
}
#[test(should_fail_with="empty key payload")]
fn absent_key_payload_is_zero() {
 let mut key=[0;31];key[30]=1;
 let _cell=crate::term::open_term(3,BoundedVec::<u8,3>::from_array("bad".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#other".as_bytes()),BoundedVec::<u8,1>::new(),key,0);
}
#[test(should_fail_with="boolean key payload")]
fn boolean_payload_is_boolean() {
 let mut key=[0;31];key[0]=5;key[1]=2;
 let _cell=crate::term::open_term(3,BoundedVec::<u8,4>::from_array("true".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#boolean".as_bytes()),BoundedVec::<u8,1>::new(),key,0);
}
#[test(should_fail_with="non-zero key tail")]
fn numeric_key_tail_is_zero() {
 let mut key=[0;31];key[0]=2;key[1]=128;key[30]=1;
 let _cell=crate::term::open_term(3,BoundedVec::<u8,1>::from_array("0".as_bytes()),BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#double".as_bytes()),BoundedVec::<u8,1>::new(),key,0);
}

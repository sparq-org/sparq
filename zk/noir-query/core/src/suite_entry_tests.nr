//! Compatibility and rejection controls for caller-selected raw bounds.
use std::collections::bounded_vec::BoundedVec;
use crate::suite_entry::{BoundedTermOpening, TermOpening, open_bounded_cell, open_bounded_quads, open_cell, open_quads};
use crate::term::{TermCell, open_key_checked, recompute_key_for, open_literal_for_key_check, check_literal_key_for};
use crate::suite_constants::{DATATYPE_CAPACITY, LANGUAGE_CAPACITY};

fn raw<let L: u32, let D: u32, let G: u32, let A: u32, let B: u32, let C: u32>(
    kind: u8, text: [u8; A], dt: [u8; B], lang: [u8; C], key: [u8; 31],
) -> BoundedTermOpening<L, D, G> {
    let mut lexical = [0; L]; let mut datatype = [0; D]; let mut language = [0; G];
    for i in 0..A { lexical[i] = text[i]; }
    for i in 0..B { datatype[i] = dt[i]; }
    for i in 0..C { language[i] = lang[i]; }
    BoundedTermOpening { bound: true, kind, lexical, lexical_len: A,
        datatype, datatype_len: B, language, language_len: C, key }
}
fn absent() -> BoundedTermOpening<32,64,8> {
    BoundedTermOpening { bound: false, kind: 0, lexical: [0;32], lexical_len: 0,
        datatype: [0;64], datatype_len: 0, language: [0;8], language_len: 0, key: [0;31] }
}
fn legacy<let L: u32, let D: u32, let G: u32>(a: BoundedTermOpening<L,D,G>) -> TermOpening<L> {
    let mut datatype = [0;DATATYPE_CAPACITY]; let mut language = [0;LANGUAGE_CAPACITY];
    for i in 0..D { datatype[i] = a.datatype[i]; }
    for i in 0..G { language[i] = a.language[i]; }
    TermOpening { bound: a.bound, kind: a.kind, lexical: a.lexical, lexical_len: a.lexical_len,
        datatype, datatype_len: a.datatype_len, language, language_len: a.language_len, key: a.key }
}
fn equal_cell(a: TermCell, b: TermCell) {
    assert((a.bound() == b.bound()) & (a.kind() == b.kind()) & (a.enc() == b.enc())
        & (a.key_class() == b.key_class()) & (a.key() == b.key())
        & (a.datatype() == b.datatype()) & (a.language() == b.language())
        & (a.credential() == b.credential()), "cell differs");
}
fn compare<let L: u32, let D: u32, let G: u32>(a: BoundedTermOpening<L,D,G>) {
    equal_cell(open_bounded_cell(a, 5), open_cell(legacy(a), 5));
}
fn checked<let L: u32, let D: u32, let G: u32>(a: BoundedTermOpening<L,D,G>, dt: Field, class: u8) {
    let fresh = open_key_checked(a, 5, dt).cell();
    let prior = recompute_key_for(open_bounded_cell(a, 5),
        BoundedVec::from_parts_unchecked(a.lexical, a.lexical_len),
        BoundedVec::from_parts_unchecked(a.datatype, a.datatype_len),
        BoundedVec::from_parts_unchecked(a.language, a.language_len), dt).cell();
    equal_cell(fresh, prior);
    let opened = open_literal_for_key_check(a, 5);
    equal_cell(opened.cell(), open_bounded_cell(a, 5));
    let split = check_literal_key_for(opened, dt).cell();
    equal_cell(split, fresh);
    assert(split.identity() == fresh.identity(), "checked identity differs");
    assert(fresh.key_class() == class, "checked class differs");
}
fn integer() -> BoundedTermOpening<32,64,8> {
    let mut key = [0;31]; key[0] = 1;
    let payload = (0x800000000000000000000000000000000000000000000000000000000000 + 7000000000000000000 as Field).to_be_bytes::<30>();
    for i in 0..30 { key[i+1] = payload[i]; }
    raw(3, "7".as_bytes(), "http://www.w3.org/2001/XMLSchema#integer".as_bytes(), "".as_bytes(), key)
}

#[test]
fn bounded_cell_matches_legacy_across_capacities() {
    compare(raw::<8,1,1,5,0,0>(1, "urn:a".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]));
    compare(raw::<32,64,8,5,0,0>(2, "label".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]));
    compare(integer());
    compare(raw::<64,96,8,5,53,2>(3, "hello".as_bytes(), "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(), "en".as_bytes(), [0;31]));
    compare(raw::<128,128,64,0,0,0>(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]));
    compare(absent());
}
#[test]
fn bounded_quads_match_legacy() {
    let a = integer(); let b = absent(); let raw = [[a,b,a,b];2];
    let bounded = open_bounded_quads(raw, 5);
    let old = open_quads([[legacy(a),legacy(b),legacy(a),legacy(b)];2], 5);
    for i in 0..2 { for j in 0..4 { equal_cell(bounded[i][j], old[i][j]); } }
}

#[test]
fn credential_role_cells_match_general_constructors() {
    let subject = raw::<32,64,8,5,0,0>(2, "label".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]);
    let predicate = raw::<32,64,8,5,0,0>(1, "urn:p".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]);
    let graph = raw::<32,64,8,0,0,0>(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]);
    let named = raw::<32,64,8,5,0,0>(2, "graph".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]);
    let rows = [[subject,predicate,integer(),graph],[predicate,predicate,predicate,named],[absent();4]];
    let specialized = crate::suite_entry::open_credential_bounded_quads(rows, 5);
    let general = open_bounded_quads(rows, 5);
    for i in 0..3 { for j in 0..4 { equal_cell(specialized[i][j], general[i][j]); } }
}
#[test(should_fail_with = "predicate kind")]
fn credential_predicate_refuses_literal() {
    let p=raw::<32,64,8,5,0,0>(1,"urn:p".as_bytes(),"".as_bytes(),"".as_bytes(),[0;31]);
    let _cells=crate::suite_entry::open_credential_bounded_quads([[p,integer(),p,p]],5);
}
#[test(should_fail_with = "predicate kind")]
fn credential_predicate_refuses_blank_node() {
    let p=raw::<32,64,8,5,0,0>(1,"urn:p".as_bytes(),"".as_bytes(),"".as_bytes(),[0;31]);
    let mut changed=p;changed.kind=2;
    let _cells=crate::suite_entry::open_credential_bounded_quads([[p,changed,p,p]],5);
}
#[test(should_fail_with = "nonliteral position")]
fn credential_subject_refuses_literal() {
    let p=raw::<32,64,8,5,0,0>(1,"urn:p".as_bytes(),"".as_bytes(),"".as_bytes(),[0;31]);
    let _cells=crate::suite_entry::open_credential_bounded_quads([[integer(),p,p,p]],5);
}
#[test(should_fail_with = "nonliteral position")]
fn credential_graph_refuses_literal() {
    let p=raw::<32,64,8,5,0,0>(1,"urn:p".as_bytes(),"".as_bytes(),"".as_bytes(),[0;31]);
    let _cells=crate::suite_entry::open_credential_bounded_quads([[p,p,p,integer()]],5);
}
#[test(should_fail_with = "non-literal metadata")]
fn credential_role_nonliteral_key_is_checked() {
    let p=raw::<32,64,8,5,0,0>(1,"urn:p".as_bytes(),"".as_bytes(),"".as_bytes(),[0;31]);
    let mut changed=p;changed.key[1]=1;
    let _cells=crate::suite_entry::open_credential_bounded_quads([[changed,p,p,p]],5);
}
#[test(should_fail_with = "non-zero padding")]
fn credential_role_absent_padding_is_checked() {
    let mut changed=absent();changed.key[1]=1;
    let _cells=crate::suite_entry::open_credential_bounded_quads([[changed,absent(),absent(),absent()]],5);
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn bounded_lexical_padding_is_rejected() { let mut a=integer();a.lexical[1]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "call to assert_max_bit_size")]
fn bounded_datatype_padding_is_rejected() { let mut a=integer();a.datatype[63]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "call to assert_max_bit_size")]
fn bounded_language_padding_is_rejected() { let mut a=integer();a.language[7]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "non-zero padding")]
fn absent_bounded_lexical_padding_is_rejected() { let mut a=absent();a.lexical[0]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "non-zero padding")]
fn absent_bounded_datatype_padding_is_rejected() { let mut a=absent();a.datatype[0]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "non-zero padding")]
fn absent_bounded_language_padding_is_rejected() { let mut a=absent();a.language[0]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "non-zero padding")]
fn absent_bounded_key_padding_is_rejected() { let mut a=absent();a.key[1]=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "non-zero padding")]
fn absent_bounded_lengths_are_rejected() { let mut a=absent();a.datatype_len=1;let _cell=open_bounded_cell(a,5); }
#[test(should_fail)]
fn bounded_lexical_length_is_checked() { let mut a=integer();a.lexical_len=33;let _cell=open_bounded_cell(a,5); }
#[test(should_fail)]
fn bounded_datatype_length_is_checked() { let mut a=integer();a.datatype_len=65;let _cell=open_bounded_cell(a,5); }
#[test(should_fail)]
fn bounded_language_length_is_checked() { let mut a=integer();a.language_len=9;let _cell=open_bounded_cell(a,5); }
#[test(should_fail_with = "datatype specialization differs")]
fn checked_opening_rejects_another_datatype() { let _cell=open_key_checked(integer(),5,crate::suite_constants::DT_DECIMAL); }
#[test(should_fail_with = "key disagrees with lexical form")]
fn checked_opening_rejects_another_key() { let mut a=integer();a.lexical[0]=56;let _cell=open_key_checked(a,5,crate::suite_constants::DT_INTEGER); }
#[test(should_fail_with = "key requires literal")]
fn checked_opening_requires_a_literal() { let _cell=open_key_checked(absent(),5,0); }

#[test(should_fail_with = "datatype specialization differs")]
fn opened_literal_rejects_another_datatype() {
    let opened = open_literal_for_key_check(integer(), 5);
    let _checked = check_literal_key_for(opened, crate::suite_constants::DT_DECIMAL);
}
#[test(should_fail_with = "key disagrees with lexical form")]
fn opened_literal_rejects_another_key() {
    let mut raw = integer(); raw.lexical[0] = 56;
    let _checked = check_literal_key_for(open_literal_for_key_check(raw, 5), crate::suite_constants::DT_INTEGER);
}
#[test(should_fail_with = "key requires literal")]
fn opened_literal_requires_a_literal() {
    let iri = raw::<32,64,8,5,0,0>(1, "urn:a".as_bytes(), "".as_bytes(), "".as_bytes(), [0;31]);
    let _opened = open_literal_for_key_check(iri, 5);
}
#[test(should_fail_with = "key requires literal")]
fn opened_literal_refuses_absence() { let _opened = open_literal_for_key_check(absent(), 5); }
#[test(should_fail_with = "call to assert_max_bit_size")]
fn opened_literal_checks_canonical_padding() {
    let mut raw = integer(); raw.lexical[31] = 1;
    let _opened = open_literal_for_key_check(raw, 5);
}
#[test]
fn opened_literals_keep_independent_lexical_associations() {
    let a = integer(); let mut b = a; b.lexical[0] = 56;
    let mut key: Field = 0; for i in 0..31 { key = key * 256 + b.key[i] as Field; }
    let bytes = (key + 1000000000000000000).to_be_bytes::<32>();
    for i in 0..31 { b.key[i] = bytes[i + 1]; }
    let first = open_literal_for_key_check(a, 5);
    let second = open_literal_for_key_check(b, 5);
    let ca = check_literal_key_for(first, crate::suite_constants::DT_INTEGER).cell();
    let cb = check_literal_key_for(second, crate::suite_constants::DT_INTEGER).cell();
    assert(ca.key() != cb.key()); assert(ca.identity() != cb.identity());
    assert(ca.key() == first.cell().key()); assert(cb.key() == second.cell().key());
}

#[test]
fn checked_opening_matches_recompute_decimal() {
    let a=raw::<32,64,8,5,40,0>(3, "61743".as_bytes(), "http://www.w3.org/2001/XMLSchema#integer".as_bytes(), "".as_bytes(), [1,128,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,13,19,24,64,171,144,223,92,0,0]);
    checked(a,crate::suite_constants::DT_INTEGER,1);
}

#[test]
fn checked_opening_matches_recompute_double() {
    let a=raw::<32,64,8,9,39,0>(3, "61743.666".as_bytes(), "http://www.w3.org/2001/XMLSchema#double".as_bytes(), "".as_bytes(), [2,192,238,37,245,79,223,59,100,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_DOUBLE,2);
}

#[test]
fn checked_opening_matches_recompute_date_time() {
    let a=raw::<32,64,8,20,41,0>(3, "6743-12-20T00:00:00Z".as_bytes(), "http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(), "".as_bytes(), [3,128,0,137,4,90,140,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_DATETIME,3);
}

#[test]
fn checked_opening_matches_recompute_local_date_time() {
    let a=raw::<32,64,8,19,41,0>(3, "6743-12-20T00:00:00".as_bytes(), "http://www.w3.org/2001/XMLSchema#dateTime".as_bytes(), "".as_bytes(), [4,128,0,137,4,90,140,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_DATETIME,4);
}

#[test]
fn checked_opening_matches_recompute_date() {
    let a=raw::<32,64,8,10,37,0>(3, "6743-12-20".as_bytes(), "http://www.w3.org/2001/XMLSchema#date".as_bytes(), "".as_bytes(), [8,128,0,0,0,0,26,155,39,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_DATE,8);
}

#[test]
fn checked_opening_matches_recompute_boolean() {
    let a=raw::<32,64,8,4,40,0>(3, "true".as_bytes(), "http://www.w3.org/2001/XMLSchema#boolean".as_bytes(), "".as_bytes(), [5,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_BOOLEAN,5);
}

#[test]
fn checked_opening_matches_recompute_string() {
    let a=raw::<32,64,8,18,39,0>(3, "é3ab297e131c9cd6f".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(), "".as_bytes(), [6,195,169,51,97,98,50,57,55,101,49,51,49,99,57,99,100,54,102,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_STRING,6);
}

#[test]
fn checked_opening_matches_recompute_string_prefix() {
    let a=raw::<64,64,8,36,39,0>(3, "3ab297e131c9cd6fzzzzzzzzzzzzzzzzzzzz".as_bytes(), "http://www.w3.org/2001/XMLSchema#string".as_bytes(), "".as_bytes(), [7,51,97,98,50,57,55,101,49,51,49,99,57,99,100,54,102,122,122,122,122,122,122,122,122,122,122,122,122,122,122]);
    checked(a,crate::suite_constants::DT_STRING,7);
}

#[test]
fn checked_opening_matches_recompute_unrepresentable() {
    let a=raw::<32,64,8,21,40,0>(3, "0.4229609993513061743".as_bytes(), "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(), "".as_bytes(), [127,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_DECIMAL,127);
}

#[test]
fn checked_opening_matches_recompute_none() {
    let a=raw::<32,64,8,25,38,0>(3, "value-4229609993513061743".as_bytes(), "http://www.w3.org/2001/XMLSchema#other".as_bytes(), "".as_bytes(), [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_OTHER,0);
}

#[test]
fn checked_opening_matches_recompute_decimal_datatype() {
    let a=raw::<32,64,8,20,40,0>(3, "42296099935130617.43".as_bytes(), "http://www.w3.org/2001/XMLSchema#decimal".as_bytes(), "".as_bytes(), [1,128,0,0,0,0,0,0,0,0,0,0,0,0,0,0,8,37,91,213,188,242,6,237,214,86,37,1,175,0,0]);
    checked(a,crate::suite_constants::DT_DECIMAL,1);
}

#[test]
fn checked_opening_matches_recompute_float_datatype() {
    let a=raw::<32,64,8,11,38,0>(3, "1743.666e31".as_bytes(), "http://www.w3.org/2001/XMLSchema#float".as_bytes(), "".as_bytes(), [2,199,10,221,140,192,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
    checked(a,crate::suite_constants::DT_FLOAT,2);
}

#[test]
fn checked_opening_matches_recompute_unsigned_byte_datatype() {
    let a=raw::<32,64,8,3,45,0>(3, "+42".as_bytes(), "http://www.w3.org/2001/XMLSchema#unsignedByte".as_bytes(), "".as_bytes(), [1,128,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,70,221,249,121,118,104,0,0]);
    checked(a,crate::suite_constants::DT_UNSIGNEDBYTE,1);
}

#[test]
fn checked_opening_matches_recompute_language_literal() {
    let a=raw::<32,64,8,5,53,2>(3,"hello".as_bytes(),"http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(),"en".as_bytes(),[0;31]);
    checked(a,crate::suite_constants::DT_LANGSTRING,0);
}

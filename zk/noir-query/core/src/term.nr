//! Authenticated RDF term cells. Construct cells through `open_term`; never use
//! TermCell itself as an entry-point witness type. Lexical hashing dominates the
//! constructor, and is shared when the same opening is used by key recomputation.
use std::collections::bounded_vec::BoundedVec;
use poseidon::poseidon2::Poseidon2;
use crate::term_bytes::{BytePacker, check_bytes};
use crate::suite_constants::{DT_LANGSTRING, KIND_DEFAULT_GRAPH, KIND_BLANK_NODE, KIND_LITERAL, DEFAULT_GRAPH_ENC, GRAPH_DOMAIN};

pub struct TermCell {
    // Empty storage is valid internally and deliberately invalid in an ABI.
    // Field privacy alone does not prevent a caller from witnessing a struct.
    constructed: [u8; 0],
    bound: bool,
    kind: u8,
    enc: Field,
    identity: Field,
    key_class: u8,
    key: Field,
    dt: Field,
    lang: Field,
    credential: u32,
}

impl TermCell {
    pub fn bound(self) -> bool { self.bound }
    pub fn kind(self) -> u8 { self.kind }
    pub fn enc(self) -> Field { self.enc }
    pub fn identity(self) -> Field { self.identity }
    pub fn key_class(self) -> u8 { self.key_class }
    pub fn key(self) -> Field { self.key }
    pub fn datatype(self) -> Field { self.dt }
    pub fn language(self) -> Field { self.lang }
    pub fn credential(self) -> u32 { self.credential }
    pub fn assert_padding(self) {
        // Only padding constructs an unbound cell, and fixes its identity to zero.
        // Keep that constructor invariant instead of hashing unused identities here.
        assert(!self.bound & (self.kind == 0) & (self.enc == 0) & (self.key_class == 0)
            & (self.key == 0) & (self.dt == 0) & (self.lang == 0) & (self.credential == 0), "non-zero padding");
    }
}

/// Canonical table padding. A bound default-graph cell is different.
pub fn padding() -> TermCell {
    TermCell { constructed: [], bound: false, kind: 0, enc: 0, identity: 0, key_class: 0, key: 0, dt: 0, lang: 0, credential: 0 }
}

/// The byte hash has the same value in every capacity bucket. Zero padding is
/// checked explicitly because BoundedVec's private backing storage is still ABI data.
pub fn hash_bytes<let L: u32>(domain: Field, bytes: BoundedVec<u8, L>) -> Field {
    crate::term_bytes::straight_hash(domain, bytes)
}

/// Build every component from an opening. Non-literals carry no key or metadata.
/// `kind` uses the suite byte tag: default graph, IRI, blank node, literal.
pub fn open_term<let L: u32, let D: u32, let G: u32>(
    kind: u8, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>,
    language: BoundedVec<u8, G>, key: [u8; 31], credential: u32,
) -> TermCell {
    if D == 0 {
        open_nonliteral_term::<L, D, G, 3>(kind, lexical, datatype, language, key, credential)
    } else {
        open_metadata_term(kind, lexical, datatype, language, key, credential)
    }
}

fn open_metadata_term<let L: u32, let D: u32, let G: u32>(
    kind: u8, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>,
    language: BoundedVec<u8, G>, key: [u8; 31], credential: u32,
) -> TermCell {
    assert(kind <= KIND_LITERAL, "term kind");
    let length = 1 + if kind == KIND_DEFAULT_GRAPH { 0 } else { 4 + lexical.len() }
        + if kind == KIND_LITERAL { 8 + datatype.len() + language.len() } else { 0 };
    if kind == KIND_DEFAULT_GRAPH { assert(lexical.len() == 0, "default graph lexical"); }
    let mut encoded: BytePacker<L + D + G + 13> =
        BytePacker::framed_prefix(kind, lexical, if kind == KIND_DEFAULT_GRAPH { 5 } else { length });
    let mut key_field: Field = 0;
    for i in 0..31 { key_field = key_field * 256 + key[i] as Field; }
    let mut dt: Field = 0;
    let mut lang: Field = 0;
    if kind == KIND_LITERAL {
        assert(datatype.len() > 0, "literal datatype");
        encoded.framed(datatype);
        encoded.framed(language);
        dt = crate::term_ids::datatype(datatype);
        assert(crate::term_key::class_matches(key[0], dt), "class disagrees with datatype");
        crate::term_key::check_key_shape(key);
        assert((language.len() > 0) == (dt == DT_LANGSTRING), "language datatype");
        lang = crate::term_ids::language(language);
    } else {
        assert((datatype.len() == 0) & (language.len() == 0) & (key_field == 0), "non-literal metadata");
        // The helpers check backing storage even for an empty opening.
        check_bytes(datatype);
        assert((language.len() > 0) == (dt == DT_LANGSTRING), "language datatype");
        check_bytes(language);
    }
    // The framed prefix checks lexical backing storage in every term kind.
    let (enc, identity) = if kind == KIND_DEFAULT_GRAPH { (DEFAULT_GRAPH_ENC, DEFAULT_GRAPH_ENC) } else {
        // The shared frame includes kind, lexical bytes, datatype and language.
        let framed = encoded.finish();
        (Poseidon2::hash([kind as Field, framed, key_field], 3), Poseidon2::hash([kind as Field, framed], 2))
    };
    TermCell { constructed: [], bound: true, kind, enc, identity, key_class: key[0], key: key_field, dt, lang,
        credential: if kind == KIND_BLANK_NODE { credential } else { 0 } }
}

/// Bind optional lexical bytes and metadata to a cell's term identity.
pub fn open_lexical<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>,
) {
    assert(cell.bound, "unbound lexical opening");
    let key = cell.key.to_be_bytes::<32>();
    assert(key[0] == 0, "key width");
    let mut bytes = [0; 31];
    for i in 0..31 { bytes[i] = key[i + 1]; }
    let rebuilt = open_term(cell.kind, lexical, datatype, language, bytes, cell.credential);
    assert((rebuilt.enc == cell.enc) & (rebuilt.identity == cell.identity) & (rebuilt.dt == cell.dt) & (rebuilt.lang == cell.lang)
        & (rebuilt.key_class == cell.key_class), "lexical opening differs");
}

/// Its private payload is minted only after lexical key recomputation.
pub struct KeyCheckedCell { cell: TermCell }
impl KeyCheckedCell { pub fn cell(self) -> TermCell { self.cell } }

/// Opening and recomputation are inseparable: a caller cannot mark an arbitrary
/// cell checked by presenting a key alone.
pub fn recompute_key<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>,
) -> KeyCheckedCell {
    recompute_key_for(cell, lexical, datatype, language, cell.dt)
}

/// Datatype-specialized form. The expected identifier is still constrained to
/// the opened datatype; a compiled query can fold unrelated lexical parsers.
pub fn recompute_key_for<let L: u32, let D: u32, let G: u32>(
    cell: TermCell, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>, language: BoundedVec<u8, G>, expected_datatype: Field,
) -> KeyCheckedCell {
    open_lexical(cell, lexical, datatype, language);
    check_opened_key(cell, lexical, expected_datatype)
}

/// Build and check a raw literal with one lexical hash. Pass a fixed datatype
/// identifier so unrelated value parsers fold away in the generated circuit.
pub fn open_key_checked<let L: u32, let D: u32, let G: u32>(
    raw: crate::suite_entry::BoundedTermOpening<L, D, G>, credential: u32, expected_dt: Field,
) -> KeyCheckedCell {
    let cell = crate::suite_entry::open_bounded_cell(raw, credential);
    let lexical = BoundedVec::from_parts_unchecked(raw.lexical, raw.lexical_len);
    check_opened_key(cell, lexical, expected_dt)
}

// Only fresh construction or a checked full lexical opening reaches this helper.
fn check_opened_key<let L: u32>(cell: TermCell, lexical: BoundedVec<u8, L>, expected_datatype: Field) -> KeyCheckedCell {
    assert(cell.kind == KIND_LITERAL, "key requires literal");
    assert(cell.dt == expected_datatype, "datatype specialization differs");
    let (_class, key) = crate::term_key::recompute(lexical, expected_datatype);
    assert(key == cell.key, "key disagrees with lexical form");
    KeyCheckedCell { cell }
}

/// Canonical unsigned field order, including values above the signed bit range.
pub fn field_lt(a: Field, b: Field) -> bool {
    a.lt(b)
}

/// RDF equality uses key-independent identity and issuer blank-node scope.
pub fn same_term(a: TermCell, b: TermCell) -> bool {
    (a.bound == b.bound) & (a.identity == b.identity) & (a.kind == b.kind)
        & (a.credential == b.credential)
}
fn fixed_iri(enc: Field, identity: Field) -> TermCell {
    TermCell { constructed: [], bound: true, kind: 1, enc, identity, key_class: 0, key: 0, dt: 0, lang: 0, credential: 0 }
}
/// A synthetic graph name has its own domain and cannot equal an issuer's label.
/// Request digest and dataset index must be fixed by generated request wiring.
pub fn credential_graph(request: [u128;2], index: u32) -> TermCell {
    assert(index < 0xffffffff, "credential scope capacity");
    let enc = Poseidon2::hash([GRAPH_DOMAIN, request[0] as Field, request[1] as Field, index as Field], 4);
    TermCell { constructed: [], bound: true, kind: 2, enc, identity: enc,
        key_class: 0, key: 0, dt: 0, lang: 0, credential: index + 1 }
}
pub fn iri_rdf_type() -> TermCell { fixed_iri(crate::suite_constants::IRI_RDF_TYPE, crate::suite_constants::IRI_RDF_TYPE_IDENTITY) }
pub fn iri_vc_type() -> TermCell { fixed_iri(crate::suite_constants::IRI_VC_TYPE, crate::suite_constants::IRI_VC_TYPE_IDENTITY) }
pub fn iri_valid_from() -> TermCell { fixed_iri(crate::suite_constants::IRI_VALID_FROM, crate::suite_constants::IRI_VALID_FROM_IDENTITY) }
pub fn iri_valid_until() -> TermCell { fixed_iri(crate::suite_constants::IRI_VALID_UNTIL, crate::suite_constants::IRI_VALID_UNTIL_IDENTITY) }
pub fn iri_issuer() -> TermCell { fixed_iri(crate::suite_constants::IRI_ISSUER, crate::suite_constants::IRI_ISSUER_IDENTITY) }
pub fn iri_subject() -> TermCell { fixed_iri(crate::suite_constants::IRI_SUBJECT, crate::suite_constants::IRI_SUBJECT_IDENTITY) }
pub fn iri_multikey() -> TermCell { fixed_iri(crate::suite_constants::IRI_MULTIKEY, crate::suite_constants::IRI_MULTIKEY_IDENTITY) }
pub fn iri_public_key() -> TermCell { fixed_iri(crate::suite_constants::IRI_PUBLIC_KEY, crate::suite_constants::IRI_PUBLIC_KEY_IDENTITY) }
pub fn iri_credential_status() -> TermCell { fixed_iri(crate::suite_constants::IRI_CREDENTIAL_STATUS, crate::suite_constants::IRI_CREDENTIAL_STATUS_IDENTITY) }
pub fn iri_status_entry() -> TermCell { fixed_iri(crate::suite_constants::IRI_STATUS_ENTRY, crate::suite_constants::IRI_STATUS_ENTRY_IDENTITY) }
pub fn iri_status_list() -> TermCell { fixed_iri(crate::suite_constants::IRI_STATUS_LIST, crate::suite_constants::IRI_STATUS_LIST_IDENTITY) }
pub fn iri_status_index() -> TermCell { fixed_iri(crate::suite_constants::IRI_STATUS_INDEX, crate::suite_constants::IRI_STATUS_INDEX_IDENTITY) }
pub fn iri_status_purpose() -> TermCell { fixed_iri(crate::suite_constants::IRI_STATUS_PURPOSE, crate::suite_constants::IRI_STATUS_PURPOSE_IDENTITY) }
pub fn iri_status_size() -> TermCell { fixed_iri(crate::suite_constants::IRI_STATUS_SIZE, crate::suite_constants::IRI_STATUS_SIZE_IDENTITY) }
pub fn iri_assertion_method() -> TermCell { fixed_iri(crate::suite_constants::IRI_ASSERTION_METHOD, crate::suite_constants::IRI_ASSERTION_METHOD_IDENTITY) }
pub fn iri_confidence_method() -> TermCell { fixed_iri(crate::suite_constants::IRI_CONFIDENCE_METHOD, crate::suite_constants::IRI_CONFIDENCE_METHOD_IDENTITY) }
pub fn iri_verified_issuer() -> TermCell { fixed_iri(crate::suite_constants::IRI_VERIFIED_ISSUER, crate::suite_constants::IRI_VERIFIED_ISSUER_IDENTITY) }

/// Credential-only constructor for positions whose RDF role excludes literals.
/// All raw metadata and backing padding are still checked before minting a cell.
pub(crate) fn open_nonliteral_term<let L: u32, let D: u32, let G: u32, let P: u32>(
    kind: u8, lexical: BoundedVec<u8, L>, datatype: BoundedVec<u8, D>,
    language: BoundedVec<u8, G>, key: [u8; 31], credential: u32,
) -> TermCell {
    std::static_assert((P == 0) | (P == 1) | (P == 3), "nonliteral RDF position");
    assert(kind < KIND_LITERAL, "nonliteral position");
    if P != 3 { assert(kind > KIND_DEFAULT_GRAPH, "subject kind"); }
    assert((datatype.len() == 0) & (language.len() == 0) & (crate::term_bytes::byte_sum(key) == 0), "non-literal metadata");
    check_bytes(datatype); check_bytes(language);
    let default_graph = (P == 3) & (kind == KIND_DEFAULT_GRAPH);
    if default_graph { assert(lexical.len() == 0, "default graph lexical"); check_bytes(lexical); }
    let (enc, identity) = if default_graph { (DEFAULT_GRAPH_ENC, DEFAULT_GRAPH_ENC) } else {
        let framed = crate::term_bytes::nonliteral_hash(kind, lexical);
        (Poseidon2::hash([kind as Field, framed, 0], 3), Poseidon2::hash([kind as Field, framed], 2))
    };
    TermCell { constructed: [], bound: true, kind, enc, identity, key_class: 0, key: 0, dt: 0, lang: 0,
        credential: if kind == KIND_BLANK_NODE { credential } else { 0 } }
}


#[test]
fn field_order_is_canonical_across_limb_and_modulus_boundaries() {
    let values: [Field; 9] = [0, 1, 0x7fffffffffffffffffffffffffffffff,
        0x80000000000000000000000000000000, 0xffffffffffffffffffffffffffffffff,
        0x100000000000000000000000000000000,
        0x2000000000000000000000000000000000000000000000000000000000000000,
        0 - 2, 0 - 1];
    for i in 0..9 { for j in 0..9 { assert(field_lt(values[i], values[j]) == (i < j)); } }
}

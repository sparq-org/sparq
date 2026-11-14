//! Authenticated envelope interface. Raw openings enter; derived values leave.
use std::collections::bounded_vec::BoundedVec;
use compose::issuer::{Point, schnorr_verify, assert_on_curve, assert_in_prime_order_subgroup};
use poseidon::poseidon2::Poseidon2;
use crate::term::{TermCell, padding, same_term, open_term, recompute_key_for};
use crate::suite_entry::{TermOpening, open_cell, BoundedTermOpening, open_bounded_cell, open_credential_quads, open_credential_bounded_quads};
use crate::suite::{credential_root, signed_message};
use crate::suite_constants::{METADATA_DOMAIN, MULTIKEY_LENGTH, MULTIKEY_PREFIX_0, MULTIKEY_PREFIX_1, MULTIKEY_PREFIX_2, MULTIKEY_PREFIX_3, CLASS_STRING_PREFIX, DT_DATETIME, DT_STRING, CLASS_DATE_TIME};

pub struct Timestamp { known: bool, millis: i64 }
impl Timestamp {
    pub fn known(self) -> bool { self.known }
    pub fn millis(self) -> i64 { self.millis }
}
fn unknown() -> Timestamp { Timestamp { known: false, millis: 0 } }

/// Exact UTC milliseconds; absent, ill-typed, local and unrepresentable are unknown.
/// The cell must come from a signed opening/property. No rounded time is returned.
pub fn open_time<let L: u32>(cell: TermCell, lexical: BoundedVec<u8,L>) -> Timestamp {
    if !cell.bound() {
        assert(lexical.len() == 0, "absent time lexical");
        crate::term_bytes::check_bytes(lexical);
        unknown()
    } else if (cell.kind() == 3) & (cell.datatype() == DT_DATETIME) {
        let dt = BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#dateTime".as_bytes());
        let checked = recompute_key_for(cell, lexical, dt, BoundedVec::<u8,1>::new(), DT_DATETIME).cell();
        if checked.key_class() == CLASS_DATE_TIME {
            let bytes = checked.key().to_be_bytes::<32>();
            let mut biased: u64 = 0;
            for i in 0..8 { biased = biased * 256 + bytes[i + 2] as u64; }
            Timestamp { known: true, millis: (biased ^ 0x8000000000000000) as i64 }
        } else { unknown() }
    } else {
        // Unknown datatype metadata is still bound by its full signed term cell.
        unknown()
    }
}

fn hex_digit(nibble: u8) -> u8 { if nibble < 10 { 48 + nibble } else { 87 + nibble } }

/// Canonical lower-case base16 Multibase with the suite's private-use multicodec.
/// Coordinate validation reuses the existing Baby Jubjub implementation.
pub fn multikey_cell(pk: Point) -> TermCell {
    assert_on_curve(pk);
    assert_in_prime_order_subgroup(pk);
    assert((pk.x != 0) | (pk.y != 1), "identity key is not admitted");
    let mut binary: [u8;68] = [0;68];
    binary[0] = MULTIKEY_PREFIX_0; binary[1] = MULTIKEY_PREFIX_1;
    binary[2] = MULTIKEY_PREFIX_2; binary[3] = MULTIKEY_PREFIX_3;
    let x = pk.x.to_be_bytes::<32>(); let y = pk.y.to_be_bytes::<32>();
    for i in 0..32 { binary[i + 4] = x[i]; binary[i + 36] = y[i]; }
    let mut lexical: [u8;MULTIKEY_LENGTH] = [0;MULTIKEY_LENGTH]; lexical[0] = 102;
    for i in 0..68 {
        lexical[2 * i + 1] = hex_digit(binary[i] / 16);
        lexical[2 * i + 2] = hex_digit(binary[i] % 16);
    }
    let mut key: [u8;31] = [0;31]; key[0] = CLASS_STRING_PREFIX;
    for i in 0..30 { key[i + 1] = lexical[i]; }
    open_term(3, BoundedVec::<u8,MULTIKEY_LENGTH>::from_array(lexical),
        BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()),
        BoundedVec::<u8,1>::new(), key, 0)
}
pub fn open_multikey(cell: TermCell, pk: Point) {
    assert(same_term(cell, multikey_cell(pk)), "verification key opening differs");
}
pub fn metadata_digest(created: TermCell, key: TermCell, method: TermCell) -> Field {
    Poseidon2::hash([METADATA_DOMAIN, created.enc(), key.enc(), method.enc()], 4)
}

pub struct OpenedCredential<let N: u32> {
    quads: [[TermCell;4];N], count: u32, root: Field, message: Field,
    created_at: TermCell, created: Timestamp, verification_key: TermCell, verification_method: TermCell,
}
impl<let N: u32> OpenedCredential<N> {
    pub fn quads(self) -> [[TermCell;4];N] { self.quads }
    pub fn count(self) -> u32 { self.count }
    pub fn root(self) -> Field { self.root }
    pub fn message(self) -> Field { self.message }
    pub fn created_at(self) -> TermCell { self.created_at }
    pub fn created(self) -> Timestamp { self.created }
    pub fn verification_key(self) -> TermCell { self.verification_key }
    pub fn verification_method(self) -> TermCell { self.verification_method }
    /// Complete singleton lookup over the authenticated table, never a supplied subset.
    pub fn property(self, subject: TermCell, predicate: TermCell) -> TermCell {
        let mut value = padding(); let mut matches: u32 = 0;
        for i in 0..N {
            let row = self.quads[i];
            if (i < self.count) & same_term(row[0], subject) & same_term(row[1], predicate) & (row[3].kind() == 0) {
                value = row[2]; matches = matches + 1;
            }
        }
        assert(matches <= 1, "property cardinality"); value
    }
    /// The unique default-graph VerifiableCredential subject, or canonical absence.
    pub fn credential_subject(self) -> TermCell {
        let mut value = padding(); let mut matches: u32 = 0;
        for i in 0..N {
            let row = self.quads[i];
            if (i < self.count) & same_term(row[1], crate::term::iri_rdf_type())
                & same_term(row[2], crate::term::iri_vc_type()) & (row[3].kind() == 0) {
                value = row[0]; matches = matches + 1;
            }
        }
        assert(matches <= 1, "credential subject cardinality"); value
    }
    pub fn valid_from(self) -> TermCell { self.property(self.credential_subject(), crate::term::iri_valid_from()) }
    pub fn valid_until(self) -> TermCell { self.property(self.credential_subject(), crate::term::iri_valid_until()) }
}

/// Revealed signature mode: external verification uses this exact salted message.
/// Entry points expose only that message and the agreed configuration digest.
pub fn open_credential_revealed<let N: u32, let L: u32, let C: u32, let V: u32>(
    raw: [[TermOpening<L>;4];N], salts: [Field;N], salt: Field, count: u32,
    configuration: [u128;2], created_raw: TermOpening<C>, method_raw: TermOpening<V>, pk: Point, credential_index: u32,
) -> OpenedCredential<N> {
    assert(credential_index < 0xffffffff, "credential scope capacity");
    open_credential_revealed_from_quads(open_credential_quads(raw, credential_index + 1), salts, salt, count,
        configuration, created_raw, method_raw, pk, credential_index)
}

fn finish_opening<let N: u32, let C: u32>(quads: [[TermCell; 4]; N], salts: [Field; N],
    salt: Field, count: u32, configuration: [u128; 2], created_at: TermCell,
    created_lexical: BoundedVec<u8, C>, method: TermCell, pk: Point, credential_index: u32,
) -> OpenedCredential<N> {
    assert(credential_index < 0xffffffff, "credential scope capacity");
    let scope = credential_index + 1;
    for i in 0..N {
        for j in 0..4 {
            let cell = quads[i][j];
            if cell.bound() & (cell.kind() == crate::suite_constants::KIND_BLANK_NODE) {
                assert(cell.credential() == scope, "credential blank-node scope");
            }
        }
    }
    let root = credential_root(quads, salts, salt, count);
    assert(!created_at.bound() | (created_at.kind() == 3), "created must be a literal");
    let created = open_time(created_at, created_lexical);
    let key = multikey_cell(pk);
    assert(!method.bound() | (method.kind() == 1), "verification method must be an IRI");
    let message = signed_message(salt, count, root, configuration, metadata_digest(created_at, key, method));
    OpenedCredential { quads, count, root, message, created_at, created, verification_key: key, verification_method: method }
}

pub fn open_credential_hidden<let N: u32, let L: u32, let C: u32, let V: u32>(
    raw: [[TermOpening<L>;4];N], salts: [Field;N], salt: Field, count: u32,
    configuration: [u128;2], created_raw: TermOpening<C>, method_raw: TermOpening<V>, pk: Point, credential_index: u32,
    r: Point, s: Field, e: Field, e_k: Field,
) -> OpenedCredential<N> {
    assert(credential_index < 0xffffffff, "credential scope capacity");
    open_credential_hidden_from_quads(open_credential_quads(raw, credential_index + 1), salts, salt, count,
        configuration, created_raw, method_raw, pk, credential_index, r, s, e, e_k)
}

/// Authenticate constructor-produced cells without reopening their lexical bytes.
/// Cells must come from constrained constructors, never an ABI or a hint.
pub fn open_credential_revealed_from_quads<let N: u32, let C: u32, let V: u32>(
    quads: [[TermCell; 4]; N], salts: [Field; N], salt: Field, count: u32,
    configuration: [u128; 2], created_raw: TermOpening<C>, method_raw: TermOpening<V>,
    pk: Point, credential_index: u32,
) -> OpenedCredential<N> {
    finish_opening(quads, salts, salt, count, configuration, open_cell(created_raw, 0),
        BoundedVec::from_parts_unchecked(created_raw.lexical, created_raw.lexical_len), open_cell(method_raw, 0), pk, credential_index)
}
/// Hidden counterpart of the same complete table authentication relation.
pub fn open_credential_hidden_from_quads<let N: u32, let C: u32, let V: u32>(
    quads: [[TermCell; 4]; N], salts: [Field; N], salt: Field, count: u32,
    configuration: [u128; 2], created_raw: TermOpening<C>, method_raw: TermOpening<V>,
    pk: Point, credential_index: u32, r: Point, s: Field, e: Field, e_k: Field,
) -> OpenedCredential<N> {
    let opened = open_credential_revealed_from_quads(quads, salts, salt, count, configuration,
        created_raw, method_raw, pk, credential_index);
    authenticate_signature(opened, pk, r, s, e, e_k)
}
fn authenticate_signature<let N: u32>(opened: OpenedCredential<N>, pk: Point,
    r: Point, s: Field, e: Field, e_k: Field,
) -> OpenedCredential<N> {
    schnorr_verify(pk, r, s, e, e_k, opened.message);
    opened
}

/// Bind the W3C decimal index string to its exact unsigned value.
pub fn open_status_index<let L: u32>(cell: TermCell, lexical: BoundedVec<u8,L>) -> u64 {
    let dt = BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes());
    let lang = BoundedVec::<u8,1>::new();
    let _checked = recompute_key_for(cell, lexical, dt, lang, DT_STRING);
    assert(lexical.len() > 0, "empty status index");
    let mut value: u64 = 0;
    for i in 0..L {
        if i < lexical.len() {
            let byte = lexical.get_unchecked(i);
            assert((byte >= 48) & (byte <= 57), "status index is not decimal");
            value = value * 10 + (byte - 48) as u64;
        }
    }
    value
}
/// W3C status purposes are string literals. Return suspension after exact opening.
pub fn open_status_purpose<let L: u32>(cell: TermCell, lexical: BoundedVec<u8,L>) -> bool {
    let dt = BoundedVec::<u8,64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes());
    let _checked = recompute_key_for(cell, lexical, dt, BoundedVec::<u8,1>::new(), DT_STRING);
    let suspended = crate::term_key::equals(lexical, "suspension".as_bytes());
    assert(suspended | crate::term_key::equals(lexical, "revocation".as_bytes()), "unknown status purpose");
    suspended
}

/// Complete opening with caller-selected lexical, datatype and language bounds.
/// Raw metadata shares the datatype/language bounds; its lexical bounds are separate.
pub fn open_bounded_credential_revealed<let N: u32, let L: u32, let C: u32, let V: u32, let D: u32, let G: u32>(
    raw: [[BoundedTermOpening<L, D, G>; 4]; N], salts: [Field; N], salt: Field, count: u32,
    configuration: [u128; 2], created_raw: BoundedTermOpening<C, D, G>, method_raw: BoundedTermOpening<V, D, G>,
    pk: Point, credential_index: u32,
) -> OpenedCredential<N> {
    assert(credential_index < 0xffffffff, "credential scope capacity");
    finish_opening(open_credential_bounded_quads(raw, credential_index + 1), salts, salt, count, configuration,
        open_bounded_cell(created_raw, 0), BoundedVec::from_parts_unchecked(created_raw.lexical, created_raw.lexical_len),
        open_bounded_cell(method_raw, 0), pk, credential_index)
}
/// Hidden signature mode over the same bounded complete opening.
pub fn open_bounded_credential_hidden<let N: u32, let L: u32, let C: u32, let V: u32, let D: u32, let G: u32>(
    raw: [[BoundedTermOpening<L, D, G>; 4]; N], salts: [Field; N], salt: Field, count: u32,
    configuration: [u128; 2], created_raw: BoundedTermOpening<C, D, G>, method_raw: BoundedTermOpening<V, D, G>,
    pk: Point, credential_index: u32, r: Point, s: Field, e: Field, e_k: Field,
) -> OpenedCredential<N> {
    let opened = open_bounded_credential_revealed(raw, salts, salt, count, configuration, created_raw, method_raw, pk, credential_index);
    authenticate_signature(opened, pk, r, s, e, e_k)
}

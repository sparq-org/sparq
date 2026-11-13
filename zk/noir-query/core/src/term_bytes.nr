//! Pack fixed input chunks before concatenating their bounded byte streams.
use poseidon::poseidon2::Poseidon2;
use std::collections::bounded_vec::BoundedVec;

pub(crate) struct BytePacker<let N: u32> {
    words: [Field; (N + 30) / 31 + 2],
    partial: Field,
    used: u32,
    filled: u32,
    written: u32,
    length: u32,
}
impl<let N: u32> BytePacker<N> {
    pub(crate) fn new(domain: Field, length: u32) -> Self {
        assert(length <= N, "byte capacity");
        let mut words = [0; (N + 30) / 31 + 2];
        words[0] = domain;
        words[1] = length as Field;
        Self { words, partial: 0, used: 0, filled: 0, written: 0, length }
    }
    pub(crate) fn framed_prefix<let L: u32>(kind: u8, bytes: BoundedVec<u8, L>, length: u32) -> Self {
        std::static_assert(N >= L + 5, "framed byte capacity");
        check_bytes(bytes);
        let prefix_length = 5 + bytes.len();
        assert(prefix_length <= length, "byte prefix length");
        let mut packed = Self::new(crate::suite_constants::TERM_DOMAIN, length);
        let words = nonliteral_words(kind, bytes);
        for i in 0..(L + 35) / 31 { packed.words[i + 2] = words[i]; }
        packed.used = prefix_length % 31;
        packed.filled = prefix_length / 31;
        packed.partial = if packed.used > 0 { packed.words[packed.filled + 2] } else { 0 };
        packed.written = prefix_length;
        packed
    }
    pub(crate) fn push(&mut self, byte: u8) {
        self.append(byte as Field, 1);
    }
    // Short chunks keep the split products below the field modulus. The partial
    // word still follows the suite's full word width and preserves leading zeros.
    fn append(&mut self, value: Field, length: u32) {
        let total = self.used + length;
        let full = total >= 31;
        let remainder = if full { total - 31 } else { 0 };
        let mut powers: [Field; 32] = [1; 32];
        for i in 1..32 {
            powers[i] = powers[i - 1] * 256;
        }
        // Safety: the remainder is constrained by the exact split relation,
        // canonical short ranges and the shifted-remainder range below.
        let low = unsafe { split_remainder(value, remainder) };
        let high = verify_split(value, low, remainder, powers);
        let shift = if full { 31 - self.used } else { length };
        let next = self.partial * powers[shift] + high;
        let index = if length > 0 { self.filled + 2 } else { 2 };
        self.words[index] = if length > 0 { next } else { self.words[index] };
        self.partial = if full { low } else { next };
        self.used = if full { remainder } else { total };
        self.filled = self.filled + full as u32;
        self.written = self.written + length;
    }
    pub(crate) fn extend<let L: u32>(&mut self, bytes: BoundedVec<u8, L>) {
        check_bytes(bytes);
        for chunk in 0..(L + 14) / 15 {
            let start = chunk * 15;
            let remaining = if start < bytes.len() {
                bytes.len() - start
            } else {
                0
            };
            let take = if remaining < 15 { remaining } else { 15 };
            let mut value: Field = 0;
            for j in 0..15 {
                let byte = if start + j < L {
                    bytes.get_unchecked(start + j)
                } else {
                    0
                };
                value = value * 256 + byte as Field;
            }
            let mut powers: [Field; 16] = [1; 16];
            for i in 1..16 {
                powers[i] = powers[i - 1] * 256;
            }
            let trimmed = value / powers[15 - take];
            self.append(trimmed, take);
        }
    }
    pub(crate) fn framed<let L: u32>(&mut self, bytes: BoundedVec<u8, L>) {
        self.append(bytes.len() as Field, 4);
        self.extend(bytes);
    }
    pub(crate) fn finish(mut self) -> Field {
        assert(self.written == self.length, "encoded byte length");
        let index = if self.used > 0 { self.filled + 2 } else { 2 };
        self.words[index] = if self.used > 0 {
            self.partial
        } else {
            self.words[index]
        };
        Poseidon2::hash(self.words, 2 + (self.length + 30) / 31)
    }
}
fn short(value: Field) {
    value.assert_max_bit_size::<120>();
}
pub(crate) fn check_bytes<let L: u32>(bytes: BoundedVec<u8, L>) {
    assert(bytes.len() <= L, "byte capacity");
    let mut powers: [Field; 16] = [1; 16];
    for i in 1..16 {
        powers[i] = powers[i - 1] * 256;
    }
    let boundary = bytes.len() / 15;
    let remainder = bytes.len() % 15;
    let mut partial: Field = 0;
    let mut padding: Field = 0;
    for chunk in 0..(L + 14) / 15 {
        let mut value: Field = 0;
        for j in 0..15 {
            let i = chunk * 15 + j;
            value = value * 256 + if i < L { bytes.get_unchecked(i) as Field } else { 0 };
        }
        partial = partial + if chunk == boundary { value } else { 0 };
        padding = padding + if chunk > boundary { value } else { 0 };
    }
    // Unused full chunks are nonnegative and their sum cannot wrap the field.
    padding.assert_max_bit_size::<0>();
    // Only the boundary chunk needs the exact short integer division check.
    short(partial / powers[15 - remainder]);

}

// The prefix and every source index are static; only the final word width varies.
pub(crate) fn nonliteral_hash<let L: u32>(kind: u8, bytes: BoundedVec<u8, L>) -> Field {
    check_bytes(bytes);
    let length = bytes.len() + 5;
    let mut words = [0; (L + 35) / 31 + 2];
    words[0] = crate::suite_constants::TERM_DOMAIN;
    words[1] = length as Field;
    let encoded = nonliteral_words(kind, bytes);
    for i in 0..(L + 35) / 31 { words[i + 2] = encoded[i]; }
    Poseidon2::hash(words, 2 + (length + 30) / 31)
}

pub(crate) fn straight_hash<let L: u32>(domain: Field, bytes: BoundedVec<u8, L>) -> Field {
    check_bytes(bytes);
    let mut words = [0; (L + 30) / 31 + 2];
    words[0] = domain; words[1] = bytes.len() as Field;
    for chunk in 0..(L + 30) / 31 {
        let start = chunk * 31;
        let width = if L - start < 31 { L - start } else { 31 };
        let mut value: Field = 0;
        for offset in 0..31 {
            if offset < width { value = value * 256 + bytes.get_unchecked(start + offset) as Field; }
        }
        let remaining = if start < bytes.len() { bytes.len() - start } else { 0 };
        let take = if remaining < width { remaining } else { width };
        words[chunk + 2] = value / power256(width - take);
    }
    Poseidon2::hash(words, 2 + (bytes.len() + 30) / 31)
}

unconstrained fn split_remainder(value: Field, bytes: u32) -> Field {
    let value = value as u128;
    let scale = 1u128 << ((bytes * 8) as u128);
    (value % scale) as Field
}
fn verify_split(value: Field, low: Field, bytes: u32, powers: [Field; 32]) -> Field {
    assert(bytes <= 15, "chunk split width");
    short(value);
    short(low);
    short(low * powers[15 - bytes]);
    // The shifted-low bound fixes low < powers[bytes]. With both quotient and
    // remainder short, the division relation cannot wrap in the field.
    let high = (value - low) / powers[bytes];
    short(high);
    high
}

fn reference_hash<let N: u32>(bytes: [u8; N], length: u32) -> Field {
    let mut words = [0; (N + 30) / 31 + 2];
    words[0] = crate::suite_constants::TERM_DOMAIN;
    words[1] = length as Field;
    for chunk in 0..(N + 30) / 31 {
        let mut value: Field = 0;
        for offset in 0..31 {
            let i = chunk * 31 + offset;
            if (i < N) & (i < length) {
                value = value * 256 + bytes[i] as Field;
            }
        }
        words[chunk + 2] = value;
    }
    Poseidon2::hash(words, 2 + (length + 30) / 31)
}
#[test]
fn chunked_bytes_match_reference_at_word_boundaries() {
    for length in [0, 1, 14, 15, 16, 30, 31, 32, 45, 46, 61, 62, 63, 95, 96] {
        let mut bytes = [0; 96];
        for i in 0..96 {
            if i < length {
                bytes[i] = ((i * 17 + length) % 256) as u8;
            }
        }
        let mut packed: BytePacker<96> =
            BytePacker::new(crate::suite_constants::TERM_DOMAIN, length);
        packed.extend(BoundedVec::from_parts_unchecked(bytes, length));
        assert(packed.finish() == reference_hash(bytes, length), "packed bytes differ");
    }
}
#[test]
fn chunked_frames_match_reference_with_empty_segments() {
    for length in [0, 1, 14, 15, 16, 29, 30, 31, 32] {
        let mut lexical = [0; 32];
        for i in 0..32 {
            if i < length {
                lexical[i] = (i + 1) as u8;
            }
        }
        let mut raw = [0; 64];
        raw[0] = 3;
        raw[4] = length as u8;
        for i in 0..32 {
            if i < length {
                raw[5 + i] = lexical[i];
            }
        }
        raw[8 + length] = 2;
        raw[9 + length] = 65;
        raw[10 + length] = 66;
        let mut packed: BytePacker<64> =
            BytePacker::new(crate::suite_constants::TERM_DOMAIN, 15 + length);
        packed.push(3);
        packed.framed(BoundedVec::from_parts_unchecked(lexical, length));
        packed.framed(BoundedVec::<u8, 2>::from_array([65, 66]));
        packed.framed(BoundedVec::<u8, 1>::new());
        assert(packed.finish() == reference_hash(raw, 15 + length), "packed frames differ");
    }
}
fn split_powers() -> [Field; 32] {
    let mut powers = [1; 32];
    for i in 1..32 {
        powers[i] = powers[i - 1] * 256;
    }
    powers
}
#[test]
fn chunk_split_matches_its_integer_quotient() {
    assert(verify_split(0x1234, 0x34, 1, split_powers()) == 0x12);
    assert(verify_split(0x1234, 0, 0, split_powers()) == 0x1234);
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn chunk_split_rejects_wrong_remainder() {
    let _high = verify_split(0x1234, 0x35, 1, split_powers());
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn chunk_split_rejects_noncanonical_remainder() {
    let _high = verify_split(0x1234, 0x134, 1, split_powers());
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn chunk_split_rejects_out_of_range_input() {
    let _high = verify_split(0x1000000000000000000000000000000, 0, 1, split_powers());
}
#[test(should_fail_with = "chunk split width")]
fn chunk_split_rejects_out_of_range_width() {
    let _high = verify_split(0, 0, 16, split_powers());
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn default_graph_byte_padding_is_rejected() {
    let _cell = crate::term::open_term(
        0,
        BoundedVec::<u8, 2>::from_parts_unchecked([65, 0], 0),
        BoundedVec::<u8, 1>::new(),
        BoundedVec::<u8, 1>::new(),
        [0; 31],
        0,
    );
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn nonliteral_metadata_byte_padding_is_rejected() {
    let _cell = crate::term::open_term(
        1,
        BoundedVec::<u8, 5>::from_array("urn:a".as_bytes()),
        BoundedVec::<u8, 2>::from_parts_unchecked([65, 0], 0),
        BoundedVec::<u8, 1>::new(),
        [0; 31],
        0,
    );
}

#[test(should_fail_with = "encoded byte length")]
fn encoded_byte_length_is_checked() {
    let mut packed: BytePacker<32> = BytePacker::new(crate::suite_constants::TERM_DOMAIN, 0);
    packed.push(1);
    let _hash = packed.finish();
}

fn power256(exponent: u32) -> Field {
    let bits = (exponent as Field).to_le_bits::<5>();
    let mut result: Field = 1;
    let mut power: Field = 256;
    for i in 0..5 {
        result = result * if bits[i] { power } else { 1 };
        power = power * power;
    }
    result
}
// Each word is a fixed linear combination of byte witnesses. Canonical backing
// zeros make division by its unused-byte scale exact in the integer encoding.
fn nonliteral_words<let L: u32>(kind: u8, bytes: BoundedVec<u8,L>) -> [Field; (L + 35) / 31] {
    let mut words = [0; (L + 35) / 31];
    for chunk in 0..(L + 35) / 31 {
        let mut value = if chunk == 0 { (kind as Field) * 4294967296 + bytes.len() as Field } else { 0 };
        let start = if chunk == 0 { 0 } else { chunk * 31 - 5 };
        let width = if chunk == 0 { if L < 26 { L } else { 26 } }
            else { if L - start < 31 { L - start } else { 31 } };
        for j in 0..31 {
            if j < width { value = value * 256 + bytes.get_unchecked(start + j) as Field; }
        }
        let remaining = if start < bytes.len() { bytes.len() - start } else { 0 };
        let take = if remaining < width { remaining } else { width };
        words[chunk] = value / power256(width - take);
    }
    words
}


#[test]
fn fixed_words_match_reference_at_framed_boundaries() {
    for length in [0, 1, 25, 26, 27, 30, 31, 32, 56, 57, 58, 61, 62, 63, 88, 89, 90, 127, 128] {
        let mut lexical = [0; 128];
        let mut raw = [0; 160];
        raw[0] = 3;
        raw[4] = length as u8;
        for i in 0..128 {
            if i < length {
                lexical[i] = ((i * 17 + length) % 256) as u8;
                raw[i + 5] = lexical[i];
            }
        }
        let bytes = BoundedVec::from_parts_unchecked(lexical, length);
        assert(nonliteral_hash(3, bytes) == reference_hash(raw, length + 5), "fixed words differ");
        raw[8 + length] = 2;
        raw[9 + length] = 65;
        raw[10 + length] = 66;
        let mut framed: BytePacker<160> = BytePacker::framed_prefix(3, bytes, length + 15);
        framed.framed(BoundedVec::<u8, 2>::from_array([65, 66]));
        framed.framed(BoundedVec::<u8, 1>::new());
        assert(framed.finish() == reference_hash(raw, length + 15), "fixed frame suffix differs");
        assert(straight_hash(crate::suite_constants::TERM_DOMAIN, bytes)
            == reference_hash(lexical, length), "fixed raw words differ");
    }
}

#[test(should_fail_with = "byte prefix length")]
fn framed_prefix_cannot_discard_lexical_bytes() {
    let _packed: BytePacker<32> = BytePacker::framed_prefix(1, BoundedVec::<u8, 1>::from_array([65]), 5);
}

#[test(should_fail_with = "call to assert_max_bit_size")]
fn fixed_prefix_rejects_backing_padding() {
    let _packed: BytePacker<32> = BytePacker::framed_prefix(1,
        BoundedVec::<u8, 2>::from_parts_unchecked([65, 66], 1), 6);
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn fixed_raw_hash_rejects_backing_padding() {
    let _hash = straight_hash(crate::suite_constants::TERM_DOMAIN,
        BoundedVec::<u8, 2>::from_parts_unchecked([65, 66], 1));
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn empty_metadata_resource_rejects_backing_padding() {
    let _cell = crate::term::open_term(1,
        BoundedVec::<u8, 2>::from_parts_unchecked([65, 66], 1),
        BoundedVec::<u8, 0>::new(), BoundedVec::<u8, 0>::new(), [0; 31], 1);
}
#[test(should_fail_with = "nonliteral position")]
fn empty_datatype_capacity_rejects_literals() {
    let _cell = crate::term::open_term(3, BoundedVec::<u8, 1>::from_array([65]),
        BoundedVec::<u8, 0>::new(), BoundedVec::<u8, 0>::new(), [0; 31], 1);
}


pub(crate) fn byte_sum<let L: u32>(bytes: [u8; L]) -> Field {
    let mut sum: Field = 0;
    for i in 0..L { sum = sum + bytes[i] as Field; }
    sum
}

#[test]
fn padding_boundary_accepts_every_canonical_length() {
    for length in 0..65 {
        let mut bytes = [0; 64];
        for i in 0..64 { if i < length { bytes[i] = ((i * 13 + 7) % 256) as u8; } }
        check_bytes(BoundedVec::from_parts_unchecked(bytes, length));
    }
}
#[test(should_fail_with = "call to assert_max_bit_size")]
fn full_padding_chunks_must_be_zero() {
    let mut bytes = [0; 64];
    bytes[0] = 65;
    bytes[45] = 1;
    check_bytes(BoundedVec::from_parts_unchecked(bytes, 1));
}

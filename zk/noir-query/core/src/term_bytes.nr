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
            short(trimmed);
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
            value = value * 256
                + if start + j < L {
                    bytes.get_unchecked(start + j) as Field
                } else {
                    0
                };
        }
        // Packed input and the bounded quotient make this an integer division:
        // their possible product is below the modulus. Every discarded byte
        // must therefore be zero, including the wholly unused chunks.
        short(value / powers[15 - take]);
    }
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

//! Streaming typed bytes into field words avoids a variable-index byte buffer.
//! Poseidon remains the pinned dependency; this adapter only packs its inputs.
use std::collections::bounded_vec::BoundedVec;
use poseidon::poseidon2::Poseidon2;

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
        words[0] = domain; words[1] = length as Field;
        Self { words, partial: 0, used: 0, filled: 0, written: 0, length }
    }
    pub(crate) fn push(&mut self, byte: u8) { self.push_if(byte, true); }
    pub(crate) fn push_if(&mut self, byte: u8, enabled: bool) {
        let index = if enabled { self.filled + 2 } else { 2 };
        let value = self.partial * 256 + byte as Field;
        // Store outside the predicate: selecting a whole array at each byte is quadratic.
        self.words[index] = if enabled { value } else { self.words[index] };
        self.partial = if enabled { value } else { self.partial };
        self.used = self.used + enabled as u32;
        self.written = self.written + enabled as u32;
        if self.used == 31 {
            self.filled = self.filled + 1;
            self.partial = 0; self.used = 0;
        }
    }
    pub(crate) fn framed<let L: u32>(&mut self, bytes: BoundedVec<u8, L>) {
        check_bytes(bytes);
        let length = (bytes.len() as Field).to_be_bytes::<4>();
        for i in 0..4 { self.push(length[i]); }
        for i in 0..L { self.push_if(bytes.get_unchecked(i), i < bytes.len()); }
    }
    pub(crate) fn finish(self) -> Field {
        assert(self.written == self.length, "encoded byte length");
        Poseidon2::hash(self.words, 2 + (self.length + 30) / 31)
    }
}
pub(crate) fn check_bytes<let L: u32>(bytes: BoundedVec<u8, L>) {
    assert(bytes.len() <= L, "byte capacity");
    for i in 0..L { if i >= bytes.len() { assert(bytes.get_unchecked(i) == 0, "non-zero byte padding"); } }
}

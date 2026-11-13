// Temporary lane-D contract. Lane A replaces this file with term.nr.
// Research grade; not externally audited. No credential encoding is defined here.
use crate::expr_core::Scalar;

pub struct TermCell {
    pub bound: bool,
    pub kind: u8,
    pub enc: Field,
    pub key_class: u8,
    pub key: Field,
    pub dt: Field,
    pub lang: Field,
    pub credential: u32,
}

// There is deliberately no implementation for a bare TermCell. The implementation
// supplied by the term module must open enc, validate the datatype/lexical form,
// recompute key_class/key, validate UTF-8 and padding, and derive ALL Scalar
// fields from that opening. IEEE bits come from the lexical opening, retaining
// signed zero even if a comparison key normalizes it. Blank
// node enc must include the credential scope. This trait is a compile-time seam,
// never a witness-supplied `key_checked` flag.
pub trait CheckedOpening<let L: u32> {
    fn open(self) -> Scalar<L>;
}

// Literal contract mirror of lane A; replace these imports at integration.
pub global CLASS_NONE: u8 = 0;
pub global CLASS_DECIMAL: u8 = 1;
pub global CLASS_DOUBLE: u8 = 2;
pub global CLASS_DATE_TIME: u8 = 3;
pub global CLASS_LOCAL_DATE_TIME: u8 = 4;
pub global CLASS_BOOLEAN: u8 = 5;
pub global CLASS_STRING: u8 = 6;
pub global CLASS_STRING_PREFIX: u8 = 7;
pub global CLASS_DATE: u8 = 8;
pub global CLASS_UNREPRESENTABLE: u8 = 127;
pub global DT_INTEGER: Field = 0x0a7208b76d85ea2290a33b1932ddeca10e8acaa3191915e26db178f0bdafd2fa;
pub global DT_DECIMAL: Field = 0x1c84b6b9f9826b96b83e5ca9ab437970720413d23db6f52cf393c88981520a14;
pub global DT_DOUBLE: Field = 0x1e5a392e58db9a4d3aaead8b0744b68b53f21edcc5293ca067c9220fe3a06a13;
pub global DT_FLOAT: Field = 0x1d69a2854fbaa3966f764069594c0ce5315ce523b566e72cf12bd6fc1e8554ae;
pub global DT_DATETIME: Field = 0x2b8b41b4eef8c53c46d958b1c936f2ab2a07cad42cf188d80bea026a9021d099;
pub global DT_BOOLEAN: Field = 0x032641e10feb6a65ebe1791f64db183511a9701e7f8cbb027ad0b282bbae19e8;
pub global DT_STRING: Field = 0x20bd98b85f0a5afe4c816b2dd66017170c61876fa819f53ad74c1ba905961400;
pub global DT_DATE: Field = 0x2b913f1d3cce00d157d3a2f242e06b78ba4690f3169768c2840d99d79e5521d0;
pub global DT_NONPOSITIVEINTEGER: Field = 0x20e4426505c45a6c3bc45e90c7b82558b333c6340eeafbc4c13d4375875b3cac;
pub global DT_NEGATIVEINTEGER: Field = 0x235d0da2e08119f7b497007c6795cb49cace0627fbeff11667c614e3b267b9ba;
pub global DT_LONG: Field = 0x1426a6c54ab80ea2bcefe5d99be1314f2c9bf16be0c36f7cac1b78ce4eceef4a;
pub global DT_INT: Field = 0x2d2a3f74288d70d666e33ec27c24f43592914e0f3d6555ece5d9733fb81ef3dd;
pub global DT_SHORT: Field = 0x27da187a883e40bbe595538de26e05d5ddf526a58eb750e4ee5c2d429b9db364;
pub global DT_BYTE: Field = 0x14e3920b464d072c15081b34d016b8d23a36c503dc230fc52dc8b68511d5e6a6;
pub global DT_NONNEGATIVEINTEGER: Field = 0x1233b193aaff724d2ef2558a340af79c2ac12ecdbf03f9f309699f8a5f8686a2;
pub global DT_UNSIGNEDLONG: Field = 0x14caf20016b20bafe266909204f879042e3f00e78c12e0e43f2fecd3d883a544;
pub global DT_UNSIGNEDINT: Field = 0x27c4e02a2fe79c1285caa087e535e68de17b248633888b9eeb98c1d214ab3eaf;
pub global DT_UNSIGNEDSHORT: Field = 0x2bde6c508aa67fc6323860931eb36a8f1a569ec77bd89b839c94d27438429ca6;
pub global DT_UNSIGNEDBYTE: Field = 0x0bbd090cd528cd9095b7d23b0c3a70b568c389e5d8459788ec819024c040cafa;
pub global DT_POSITIVEINTEGER: Field = 0x02b0b0cea3b1390254029161fc49193254f5705d4a1b6b115269718e204c108b;
pub global DT_LANGSTRING: Field = 0x0eb6478ded5db6ac415603573fafac6836b76d48c2315d3b206ee24b3c31ffd5;
pub global KIND_DEFAULT_GRAPH: u8 = 0;
pub global KIND_IRI: u8 = 1;
pub global KIND_BLANK_NODE: u8 = 2;
pub global KIND_LITERAL: u8 = 3;

impl TermCell {
    pub fn bound(self) -> bool { self.bound }
    pub fn kind(self) -> u8 { self.kind }
    pub fn enc(self) -> Field { self.enc }
    pub fn key_class(self) -> u8 { self.key_class }
    pub fn key(self) -> Field { self.key }
    pub fn datatype(self) -> Field { self.dt }
    pub fn language(self) -> Field { self.lang }
    pub fn credential(self) -> u32 { self.credential }
}

// The adapter must call lane A's open_lexical and, for literals, recompute_key.
// All bytes here are returned by that constrained adapter, never independent
// arguments to a function which reads a term's characters.
use std::collections::bounded_vec::BoundedVec;
pub struct LexicalOpening<let L: u32, let D: u32, let G: u32> {
    pub cell: TermCell,
    pub lexical: BoundedVec<u8,L>,
    pub datatype: BoundedVec<u8,D>,
    pub language: BoundedVec<u8,G>,
}
pub trait CheckedLexicalOpening<let L: u32, let D: u32, let G: u32> {
    fn open_lexical(self) -> LexicalOpening<L,D,G>;
}
pub fn check_bytes<let L: u32>(bytes: BoundedVec<u8,L>) {
    assert(bytes.len() <= L,"opening capacity");
    for i in 0..L { if i >= bytes.len() { assert(bytes.get_unchecked(i)==0,"opening padding"); } }
}
pub fn read<T,let L:u32,let D:u32,let G:u32>(value:T) -> LexicalOpening<L,D,G>
where T:CheckedLexicalOpening<L,D,G> {
    let out=value.open_lexical();
    check_bytes(out.lexical);check_bytes(out.datatype);check_bytes(out.language);
    assert(out.cell.kind <= KIND_LITERAL,"term kind");
    assert((out.cell.kind==KIND_BLANK_NODE) | (out.cell.credential==0),"credential scope");
    out
}
// Key includes its class byte: one class byte followed by thirty payload bytes.
pub fn key_payload(cell:TermCell) -> Field {
    cell.key.assert_max_bit_size::<248>();
    let bytes=cell.key.to_be_bytes::<31>();
    assert(bytes[0]==cell.key_class,"key class byte");
    cell.key-cell.key_class as Field*0x1000000000000000000000000000000000000000000000000000000000000
}

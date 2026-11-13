//! STAND-IN: lane D owns the typed arithmetic and numeric-result adapter.
//! This bounded adapter hashes the canonical lexical result through lane A.
use std::collections::bounded_vec::BoundedVec;
use crate::table::TermCell;
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL};
pub global OFFSET: Field = 0x800000000000000000000000000000000000000000000000000000000000;
pub global PREFIX: Field = 0x1000000000000000000000000000000000000000000000000000000000000;
pub fn encode(key: Field, decimal: bool) -> TermCell {
    key.assert_max_bit_size::<240>();
    let negative = key.lt(OFFSET);
    let magnitude = if negative { OFFSET-key } else { key-OFFSET };
    magnitude.assert_max_bit_size::<127>();
    let scaled = magnitude as u128;
    let scale: u128 = 1000000000000000000;
    let mut integer = scaled / scale;
    let fraction = scaled % scale;
    assert(decimal | (fraction == 0), "integer result precision");
    let mut digits = [0; 39];
    let mut length: u32 = 0;
    for i in 0..39 {
        digits[i] = (integer % 10) as u8 + 48;
        if (integer > 0) | (i == 0) { length = length + 1; }
        integer = integer / 10;
    }
    let mut lexical: BoundedVec<u8, 64> = BoundedVec::new();
    if negative { lexical.push(45); }
    for i in 0..39 { if i < length { lexical.push(digits[length-1-i]); } }
    if decimal {
        lexical.push(46);
        let mut remaining = fraction; let mut divisor: u128 = 100000000000000000;
        for i in 0..18 {
            if (remaining > 0) | (i == 0) { lexical.push((remaining / divisor) as u8 + 48); }
            remaining = remaining % divisor; divisor = divisor / 10;
        }
    }
    let mut dt: BoundedVec<u8, 41> = BoundedVec::new();
    let datatype = if decimal { "http://www.w3.org/2001/XMLSchema#decimal" } else { "http://www.w3.org/2001/XMLSchema#integer" };
    for b in datatype.as_bytes() { dt.push(b); }
    let bytes = (PREFIX + key).to_be_bytes::<32>();
    let mut raw = [0;31]; for i in 0..31 { raw[i]=bytes[i+1]; }
    let cell=crate::term::open_term(3,lexical,dt,BoundedVec::<u8,0>::new(),raw,0);
    assert(cell.datatype() == if decimal { DT_DECIMAL } else { DT_INTEGER }, "numeric datatype");
    TermCell {bound:true,kind:3,enc:cell.enc(),key_class:1,key,dt:cell.datatype(),lang:0,scope:0}
}

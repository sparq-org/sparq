//! Bridge between suite-prefixed keys and the complete grouping relation.
use crate::table::{TermCell,Table,check_table};
use crate::ops_closed_group::{Aggregate, NumericTypes, Having};
use crate::ops_closed_numeric_standin::{PREFIX,encode};
use crate::suite_constants::{DT_INTEGER, DT_DECIMAL, LANGUAGE_DOMAIN};
use std::collections::bounded_vec::BoundedVec;
fn normalize(mut c: TermCell) -> TermCell {
    if c.bound & (c.kind == 3) & (c.key_class == 1) {
        let bytes=c.key.to_be_bytes::<32>();
        assert((bytes[0] == 0) & (bytes[1] == 1), "suite numeric key prefix");
        let lang=crate::term::hash_bytes(LANGUAGE_DOMAIN,BoundedVec::<u8,0>::new());
        assert(c.lang == lang, "numeric language");
        c.key=c.key-PREFIX;c.lang=0;
    }
    c
}
pub fn group<let A:u32,let R:u32,let V:u32,let G:u32,let M:u32,let W:u32>(
    input:Table<A,V>,out:Table<R,W>,columns:[u32;G],specs:[Aggregate;M],
) {
    check_table(input);check_table(out);
    let mut normalized=input;let mut expected=out;
    for i in 0..A {for v in 0..V {normalized.rows[i][v]=normalize(input.rows[i][v]);}}
    for i in 0..R {for v in 0..W {expected.rows[i][v]=normalize(out.rows[i][v]);}}
    let types=NumericTypes {integer_dt:DT_INTEGER,decimal_dt:DT_DECIMAL};
    let having=Having {enabled:false,column:0,comparison:0,rhs:crate::table::unbound()};
    if (G == 0) & (R > 0) {
        assert(out.len <= 1, "operator length");
        let singleton=Table {rows:[expected.rows[0]],len:expected.len};
        crate::ops_closed_group::group(normalized,singleton,columns,specs,[[false;M];A],types,having,encode);
    } else {
        crate::ops_closed_group::group(normalized,expected,columns,specs,[[false;M];A],types,having,encode);
    }
}

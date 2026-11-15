//! Multiset UNION projects both entire inputs into the combined schema.
use crate::table::{Table, empty, append, canonicalize, check_table, assert_table};
use crate::ops_closed_common::project_row;
pub fn union<let A: u32, let B: u32, let R: u32, let VA: u32, let VB: u32, let V: u32>(
    left: Table<A, VA>, right: Table<B, VB>, ac: [u32; V], bc: [u32; V], out: Table<R, V>,
) {
    check_table(left); check_table(right);
    let mut expected = empty();
    for i in 0..A {expected = append(expected, project_row(left.rows[i], ac), i < left.len);}
    for i in 0..B {expected = append(expected, project_row(right.rows[i], bc), i < right.len);}
    assert_table(canonicalize(expected), out);
}

use crate::table::{TermCell, Table, unbound, empty, equal_row, canonicalize};
use std::ops::{WrappingMul, WrappingAdd};

fn iri(enc: Field) -> TermCell {
    TermCell {bound:true,kind:1,enc,key_class:0,key:0,dt:0,lang:0,scope:0}
}

#[test]
fn same_subject_different_object_is_not_a_duplicate() {
    crate::ops_core::scan([[iri(1),iri(9),iri(2)],[iri(1),iri(9),iri(3)]],
        [true;2],[0,2,1],[unbound(),iri(9),unbound()],
        Table {rows:[[iri(1),iri(2)],[iri(1),iri(3)],[unbound();2],[unbound();2]],len:2});
}

#[test]
fn blank_node_scope_is_part_of_duplicate_identity() {
    let mut a=iri(1);a.kind=2;a.scope=1;
    let mut b=a;b.scope=2;
    crate::ops_core::scan([[a,iri(9),iri(10)],[b,iri(9),iri(10)]],
        [true;2],[0,1,1],[unbound(),iri(9),iri(10)],
        Table {rows:[[a],[b],[unbound()],[unbound()]],len:2});
}

#[test]
fn all_constant_scan_emits_the_empty_binding_once() {
    crate::ops_core::scan([[iri(1),iri(9),iri(10)];3],
        [true;3],[1;3],[iri(1),iri(9),iri(10)],
        Table {rows:[[unbound()];4],len:1});
}

#[test]
fn inactive_and_nonmatching_rows_do_not_populate_the_cache() {
    crate::ops_core::scan([
        [iri(7),iri(9),iri(10)],[iri(8),iri(8),iri(10)],
        [iri(1),iri(9),iri(2)],[iri(1),iri(9),iri(1)],
    ],[false,true,true,true],[0,1,0],[unbound(),iri(9),unbound()],
        Table {rows:[[iri(1)],[unbound()],[unbound()],[unbound()]],len:1});
}

#[test]
fn first_matching_occurrence_retains_the_complete_payload() {
    let mut early=iri(1);early.key=11;
    let mut matched=early;matched.key=22;
    let mut later=matched;later.key=33;
    crate::ops_core::scan([
        [early,iri(8),iri(10)],[matched,iri(9),iri(10)],[later,iri(9),iri(10)],
    ],[true;3],[0,1,1],[unbound(),iri(9),iri(10)],
        Table {rows:[[matched],[unbound()],[unbound()],[unbound()]],len:1});
}

#[test(should_fail_with="scan completeness or capacity")]
fn scan_cannot_evict_an_emitted_row_to_fit_another_match() {
    crate::ops_core::scan([
        [iri(1),iri(9),iri(10)],[iri(2),iri(9),iri(10)],[iri(3),iri(9),iri(10)],
    ],[true;3],[0,1,1],[unbound(),iri(9),iri(10)],empty::<2,1>());
}

#[test]
fn stable_equal_identity_keeps_payload_order() {
    let mut a=iri(1);a.key=11;
    let mut b=a;b.key=22;
    let sorted=canonicalize(Table {rows:[[a],[b],[iri(0)],[iri(2)]],len:4});
    assert(equal_row(sorted.rows[0],[iri(0)]));
    assert(equal_row(sorted.rows[1],[a]));
    assert(equal_row(sorted.rows[2],[b]));
    assert(equal_row(sorted.rows[3],[iri(2)]));
}

fn original_sort<let R:u32,let V:u32>(mut table:Table<R,V>) -> Table<R,V> {
    for _ in 0..R {for i in 0..R-1 {
        if (i+1 < table.len) & crate::table::row_less(table.rows[i+1],table.rows[i]) {
            let row=table.rows[i];table.rows[i]=table.rows[i+1];table.rows[i+1]=row;
        }
    }}
    table
}

#[test]
fn network_matches_stable_sort_with_duplicates_and_padding(seed:u64) {
    let mut state=seed;
    let mut table=empty::<4,2>();table.len=(seed%5) as u32;
    for r in 0..4 {for c in 0..2 {
        state=state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if r < table.len {
            let mut cell=iri((state%4) as Field);
            if state%3==0 {cell.kind=2;cell.scope=((state/4)%3) as u32;}
            cell.key=(state%97) as Field;
            table.rows[r][c]=cell;
        }
    }}
    let expected=original_sort(table);let actual=canonicalize(table);
    assert(actual.len==expected.len);
    for i in 0..4 {assert(equal_row(actual.rows[i],expected.rows[i]));}
}

#[test]
fn other_capacity_buckets_keep_the_stable_fallback() {
    let mut table=empty::<3,1>();table.len=3;
    table.rows[0]=[iri(3)];table.rows[1]=[iri(1)];table.rows[2]=[iri(2)];
    let expected=original_sort(table);let actual=canonicalize(table);
    assert(actual.len==expected.len);
    for i in 0..3 {assert(equal_row(actual.rows[i],expected.rows[i]));}
}

fn compare_larger_sort<let R:u32>(seed:u64) {
    let mut state=seed;let mut table=empty::<R,2>();table.len=(seed%((R+1) as u64)) as u32;
    for r in 0..R {for c in 0..2 {
        state=state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if r < table.len {
            let mut cell=iri((state%4) as Field);
            if state%3==0 {cell.kind=2;cell.scope=((state/4)%3) as u32;}
            cell.key=(state%97) as Field;table.rows[r][c]=cell;
        }
    }}
    let expected=original_sort(table);let actual=canonicalize(table);
    assert(actual.len==expected.len);
    for i in 0..R {assert(equal_row(actual.rows[i],expected.rows[i]));}
}
#[test]
fn larger_network_matches_stable_sort_eight(seed:u64) {compare_larger_sort::<8>(seed);}
#[test]
fn larger_network_matches_stable_sort_sixteen(seed:u64) {compare_larger_sort::<16>(seed);}
#[test]
fn larger_network_matches_stable_sort_thirty_two(seed:u64) {compare_larger_sort::<32>(seed);}
#[test]
fn larger_network_moves_active_rows_before_padding() {compare_larger_sort::<8>(5);}
#[test]
fn larger_network_needs_descending_intermediate_runs() {
    let values:[u32;8]=[0,1,2,3,4,6,5,7];let mut table=empty::<8,1>();table.len=8;
    for i in 0..8 {table.rows[i]=[iri(values[i] as Field)];}
    let sorted=canonicalize(table);
    for i in 0..8 {assert(equal_row(sorted.rows[i],[iri(i as Field)]));}
}
#[test]
fn larger_network_moves_equal_identity_payloads_stably() {
    let mut table=empty::<8,1>();table.len=8;
    for i in 0..8 {let mut cell=iri(((7-i)/2) as Field);cell.key=i as Field;table.rows[i]=[cell];}
    let actual=canonicalize(table);let expected=original_sort(table);
    for i in 0..8 {assert(equal_row(actual.rows[i],expected.rows[i]));}
}
fn reverse_unique<let R:u32>() {
    let mut table=empty::<R,1>();table.len=R;
    for i in 0..R {let mut cell=iri((R-1-i) as Field);cell.key=i as Field;table.rows[i]=[cell];}
    let sorted=canonicalize(table);
    for i in 0..R {let mut expected=iri(i as Field);expected.key=(R-1-i) as Field;
        assert(equal_row(sorted.rows[i],[expected]));}
}
#[test]
fn larger_network_sorts_unique_keys_across_every_admitted_stage() {
    reverse_unique::<8>();reverse_unique::<16>();reverse_unique::<32>();
}
#[test]
fn adjacent_distinct_preserves_full_binding_and_first_payload() {
    let mut a=iri(1);a.key=11;let mut copy=a;copy.key=22;
    crate::ops_core::distinct(Table {rows:[[a,iri(10)],[copy,iri(10)],[a,iri(11)],[copy,iri(11)]],len:4},
        Table {rows:[[a,iri(10)],[a,iri(11)],[unbound();2],[unbound();2]],len:2});
}
#[test]
fn adjacent_distinct_keeps_the_first_empty_binding() {
    crate::ops_core::distinct(Table {rows:[[unbound()];8],len:3},Table {rows:[[unbound()];4],len:1});
}
#[test]
fn adjacent_distinct_keeps_separate_blank_scopes() {
    let mut a=iri(1);a.kind=2;a.scope=1;let mut b=a;b.scope=2;
    crate::ops_core::distinct(Table {rows:[[a],[a],[b],[b]],len:4},
        Table {rows:[[a],[b],[unbound()],[unbound()]],len:2});
}
#[test(should_fail_with="table capacity")]
fn adjacent_distinct_retains_input_validation() {
    crate::ops_core::distinct(Table {rows:[[iri(1)];4],len:5},
        Table {rows:[[iri(1)],[unbound()],[unbound()],[unbound()]],len:1});
}

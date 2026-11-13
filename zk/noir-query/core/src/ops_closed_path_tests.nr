use crate::table::{TermCell, Table, unbound, empty, append, canonicalize, assert_table};
use crate::ops_closed_path::{edges, identity, optional, closure, op_edges, op_inverse, op_sequence, op_alternative, op_optional, op_closure};
fn cell(n: Field) -> TermCell { TermCell {bound: true,kind: 1,enc:n,key_class:0,key:0,dt:0,lang:0,scope:0} }
fn pairs<let R: u32>() -> Table<R,2> { canonicalize(append(append(empty(),[cell(1),cell(2)],true),[cell(2),cell(3)],true)) }
fn right<let R:u32>() -> Table<R,2> { canonicalize(append(append(empty(),[cell(2),cell(4)],true),[cell(3),cell(5)],true)) }
fn graph() -> [[TermCell;3];3] { [[cell(1),cell(9),cell(2)],[cell(2),cell(9),cell(3)],[cell(3),cell(8),cell(1)]] }
fn honest<let R: u32>(op: u32) -> Table<R,2> {
    let mut out = empty();
    if (op == 0) | (op == 4) | (op == 7) { out = pairs(); }
    if op == 1 { out = append(append(out,[cell(2),cell(1)],true),[cell(3),cell(2)],true); }
    if op == 2 { out = append(append(out,[cell(1),cell(4)],true),[cell(2),cell(5)],true); }
    if op == 3 { out = append(append(pairs(),[cell(1),cell(2)],true),[cell(2),cell(3)],true); }
    if (op == 5) | (op == 6) { out = append(pairs(),[cell(1),cell(3)],true); }
    if op == 6 { for i in 1..4 { out = append(out,[cell(i as Field),cell(i as Field)],true); } }
    canonicalize(out)
}
fn check<let R: u32>(op: u32, out: Table<R,2>) {
    if op == 0 { op_edges(graph(),[true;3],[cell(9)],1,false,out); }
    else if op == 1 { op_inverse(pairs::<R>(),out); }
    else if op == 2 { op_sequence(pairs(),right(),out); }
    else if op == 3 { op_alternative(pairs(),pairs(),out); }
    else if op == 4 { op_optional(pairs(),empty(),out); }
    else if op == 5 { op_closure::<2,R>(pairs(),empty(),false,2,out); }
    else if op == 6 { op_closure::<2,R>(pairs(),identity(graph(),[true;3],[unbound();2]),true,2,out); }
    else { op_edges(graph(),[true;3],[cell(8)],1,true,out); }
}
fn changed(op:u32, mutation:u32) {
    let mut out: Table<8,2> = honest(op);
    if mutation == 0 { out.len = out.len-1; out.rows[out.len] = [unbound();2]; }
    if mutation == 1 { out.rows[out.len] = [cell(6),cell(7)]; out.len = out.len+1; }
    if mutation == 2 {
        if out.len > 1 { let row = out.rows[0]; out.rows[0]=out.rows[out.len-1];out.rows[out.len-1]=row; }
        else { out.rows[0]=[cell(3),cell(1)]; }
    }
    if mutation == 3 { out.rows[out.len]=[cell(6),cell(7)]; }
    check(op,out);
}

#[test]
fn link_honest() { check::<8>(0,honest(0)); }
#[test(should_fail)]
fn link_missing() { changed(0,0); }
#[test(should_fail)]
fn link_extra() { changed(0,1); }
#[test(should_fail)]
fn link_reordered() { changed(0,2); }
#[test(should_fail)]
fn link_padding() { changed(0,3); }
#[test(should_fail)]
fn link_overflow() { check::<1>(0,empty()); }

#[test]
fn inverse_honest() { check::<8>(1,honest(1)); }
#[test(should_fail)]
fn inverse_missing() { changed(1,0); }
#[test(should_fail)]
fn inverse_extra() { changed(1,1); }
#[test(should_fail)]
fn inverse_reordered() { changed(1,2); }
#[test(should_fail)]
fn inverse_padding() { changed(1,3); }
#[test(should_fail)]
fn inverse_overflow() { op_inverse(pairs::<2>(),empty::<1,2>()); }

#[test]
fn sequence_honest() { check::<8>(2,honest(2)); }
#[test(should_fail)]
fn sequence_missing() { changed(2,0); }
#[test(should_fail)]
fn sequence_extra() { changed(2,1); }
#[test(should_fail)]
fn sequence_reordered() { changed(2,2); }
#[test(should_fail)]
fn sequence_padding() { changed(2,3); }
#[test(should_fail)]
fn sequence_overflow() { let a: Table<2,2> = append(append(empty(),[cell(1),cell(2)],true),[cell(2),cell(2)],true); let b: Table<2,2> = append(append(empty(),[cell(2),cell(3)],true),[cell(2),cell(4)],true); op_sequence(a,b,empty()); }

#[test]
fn alternative_honest() { check::<8>(3,honest(3)); }
#[test(should_fail)]
fn alternative_missing() { changed(3,0); }
#[test(should_fail)]
fn alternative_extra() { changed(3,1); }
#[test(should_fail)]
fn alternative_reordered() { changed(3,2); }
#[test(should_fail)]
fn alternative_padding() { changed(3,3); }
#[test(should_fail)]
fn alternative_overflow() { check::<2>(3,empty()); }

#[test]
fn optional_honest() { check::<8>(4,honest(4)); }
#[test(should_fail)]
fn optional_missing() { changed(4,0); }
#[test(should_fail)]
fn optional_extra() { changed(4,1); }
#[test(should_fail)]
fn optional_reordered() { changed(4,2); }
#[test(should_fail)]
fn optional_padding() { changed(4,3); }
#[test(should_fail)]
fn optional_overflow() { let a: Table<2,2> = pairs(); let z: Table<2,2> = append(empty(),[cell(4),cell(4)],true); op_optional(a,z,empty()); }

#[test]
fn plus_honest() { check::<8>(5,honest(5)); }
#[test(should_fail)]
fn plus_missing() { changed(5,0); }
#[test(should_fail)]
fn plus_extra() { changed(5,1); }
#[test(should_fail)]
fn plus_reordered() { changed(5,2); }
#[test(should_fail)]
fn plus_padding() { changed(5,3); }
#[test(should_fail)]
fn plus_overflow() { check::<2>(5,empty()); }

#[test]
fn star_honest() { check::<8>(6,honest(6)); }
#[test(should_fail)]
fn star_missing() { changed(6,0); }
#[test(should_fail)]
fn star_extra() { changed(6,1); }
#[test(should_fail)]
fn star_reordered() { changed(6,2); }
#[test(should_fail)]
fn star_padding() { changed(6,3); }
#[test(should_fail)]
fn star_overflow() { check::<2>(6,empty()); }

#[test]
fn negated_honest() { check::<8>(7,honest(7)); }
#[test(should_fail)]
fn negated_missing() { changed(7,0); }
#[test(should_fail)]
fn negated_extra() { changed(7,1); }
#[test(should_fail)]
fn negated_reordered() { changed(7,2); }
#[test(should_fail)]
fn negated_padding() { changed(7,3); }
#[test(should_fail)]
fn negated_overflow() { check::<1>(7,empty()); }

#[test(should_fail)]
fn closure_depth_refuses_extension() { let input: Table<8,2> = pairs(); op_closure::<1,8>(input,empty(),false,1,input); }
#[test(should_fail)]
fn closure_public_depth_is_bound() { let input: Table<8,2> = pairs(); op_closure::<2,8>(input,empty(),false,1,honest(5)); }
#[test(should_fail)]
fn negated_hidden_eligible_edge() { let out: Table<8,2> = edges(graph(),[true,false,true],[cell(8)],1,true); op_edges(graph(),[true;3],[cell(8)],1,true,out); }
#[test(should_fail)]
fn negated_hidden_excluded_edge_changes_predicate_claim() { let mut data = graph(); data[0][1]=cell(8); let out: Table<8,2> = edges(data,[true;3],[cell(8)],1,true); op_edges(graph(),[true;3],[cell(8)],1,true,out); }
#[test(should_fail)]
fn fixpoint_hidden_extending_edge() { let out: Table<8,2> = edges(graph(),[true,false,false],[cell(9)],1,false); op_edges(graph(),[true;3],[cell(9)],1,false,out); op_closure::<1,8>(out,empty(),false,1,out); }
#[test]
fn zero_includes_object_literals_and_absent_anchor_excludes_predicates() { let mut literal = cell(2); literal.kind = 3; let data = [[cell(1),cell(9),literal]]; let z: Table<4,2> = identity(data,[true],[cell(7),unbound()]); assert(z.len == 3); assert_table(canonicalize(append(append(append(empty(),[cell(1),cell(1)],true),[literal,literal],true),[cell(7),cell(7)],true)),z); }
#[test]
fn cycles_and_diamond_deduplicate_endpoints() { let input: Table<8,2> = canonicalize(append(append(append(pairs(),[cell(1),cell(3)],true),[cell(3),cell(1)],true),[cell(1),cell(2)],true)); let out = closure::<3,16>(widen(input),empty(),false,3); assert(out.len == 9); }
fn widen(input: Table<8,2>) -> Table<16,2> { let mut out=empty(); for i in 0..8 {out=append(out,input.rows[i],i<input.len);} out }
#[test]
fn optional_is_distinct_including_self_loop() { let input: Table<4,2> = append(append(empty(),[cell(1),cell(1)],true),[cell(1),cell(1)],true); let z = append(empty(),[cell(1),cell(1)],true); let out = optional(input,z); assert(out.len == 1); }

#[test]
fn zero_distinguishes_credential_scoped_blank_nodes() { let mut a=cell(1);a.kind=2;a.scope=1;let mut b=a;b.scope=2;let z: Table<4,2> = identity([[a,cell(9),b]],[true],[unbound();2]);assert(z.len==2);assert_table(append(append(empty(),[a,a],true),[b,b],true),z); }

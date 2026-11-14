//! Exact bounded join with a private inverse mapping for the canonical output.
//! Complete pair counting and ordered equal-binding runs constrain a bijection.
use crate::table::{TermCell,Table,check_table,equal_row,same_row};
use crate::ops_closed_common::compatible;

pub fn join<let A:u32,let B:u32,let R:u32,let VA:u32,let VB:u32,let V:u32>(
    left:Table<A,VA>,right:Table<B,VB>,ac:[u32;V],bc:[u32;V],
    pairs:[[u32;2];R],out:Table<R,V>) {
    std::static_assert((A<=32)&(B<=32),"closed join capacity profile");
    check_table(left);check_table(right);check_table(out);
    for v in 0..V {assert((ac[v]<=VA)&(bc[v]<=VB),"join column");}
    let mut count:u32=0;
    // Count every compatible pair, independently of the inverse-map witness.
    for i in 0..A {for j in 0..B {
        let (matches,_,_)=compatible(left.rows[i],right.rows[j],ac,bc);
        count=count+((i<left.len)&(j<right.len)&matches) as u32;
    }}
    assert(count<=R,"operator capacity");
    assert(out.len==count,"operator length");
    for r in 0..R {
        let active=r<out.len;
        let valid=(pairs[r][0]<left.len)&(pairs[r][1]<right.len);
        assert(!active|valid,"join pair index");
        assert(active|((pairs[r][0]==0)&(pairs[r][1]==0)),"join pair padding");
        if (A>0)&(B>0) {
            let ai=if active & (pairs[r][0]<A) {pairs[r][0]}else{0};
            let bi=if active & (pairs[r][1]<B) {pairs[r][1]}else{0};
            let (matches,_,row)=compatible(left.rows[ai],right.rows[bi],ac,bc);
            assert(!active|matches,"join pair compatibility");
            assert(!active|equal_row(row,out.rows[r]),"operator rows");
        }
        if r>0 {
            // Canonical identity order makes equal bindings contiguous. Strict
            // pair order in each run prevents reuse and fixes stable payloads.
            let previous=pairs[r-1][0] as u64*B as u64+pairs[r-1][1] as u64;
            let current=pairs[r][0] as u64*B as u64+pairs[r][1] as u64;
            assert(!active|!same_row(out.rows[r-1],out.rows[r])|(previous<current),"join pair order");
        }
    }
}

fn iri(n:Field)->TermCell {TermCell{bound:true,kind:1,enc:n,key_class:0,key:0,dt:0,lang:0,scope:0}}
fn left()->Table<4,2>{crate::table::append(crate::table::append(crate::table::empty(),[iri(1),iri(10)],true),[iri(2),iri(11)],true)}
fn right()->Table<4,2>{crate::table::append(crate::table::append(crate::table::empty(),[iri(1),iri(20)],true),[iri(2),iri(21)],true)}
fn expected()->Table<4,3>{crate::table::append(crate::table::append(crate::table::empty(),[iri(1),iri(10),iri(20)],true),[iri(2),iri(11),iri(21)],true)}
fn relation(out:Table<4,3>){join(left(),right(),[0,1,2],[0,2,1],[[0,0],[1,1],[0,0],[0,0]],out);}
#[test] fn closed_join_honest(){relation(expected());}
#[test(should_fail_with="operator length")] fn closed_join_missing(){let mut out=expected();out.len=1;out.rows[1]=[crate::table::unbound();3];join(left(),right(),[0,1,2],[0,2,1],[[0,0];4],out);}
#[test(should_fail_with="operator length")] fn closed_join_extra(){let out=expected();relation(crate::table::append(out,out.rows[1],true));}
#[test(should_fail_with="table order")] fn closed_join_reordered(){let mut out=expected();let a=out.rows[0];out.rows[0]=out.rows[1];out.rows[1]=a;relation(out);}
#[test(should_fail_with="table padding")] fn closed_join_padding(){let mut out=expected();out.rows[3]=out.rows[0];relation(out);}
#[test(should_fail_with="operator capacity")] fn closed_join_overflow(){join(left(),right(),[0,1,2],[0,2,1],[[0,0]],crate::table::empty::<1,3>());}
#[test(should_fail_with="join pair index")] fn closed_join_cannot_select_padding(){join(left(),right(),[0,1,2],[0,2,1],[[2,0],[1,1],[0,0],[0,0]],expected());}
#[test(should_fail_with="join pair padding")] fn closed_join_pair_padding_is_canonical(){join(left(),right(),[0,1,2],[0,2,1],[[0,0],[1,1],[1,0],[0,0]],expected());}
#[test(should_fail_with="join pair order")] fn closed_join_cannot_repeat_a_matching_pair(){
    let a=crate::table::append(crate::table::append(crate::table::empty::<2,1>(),[iri(1)],true),[iri(1)],true);
    join(a,crate::table::append(crate::table::empty::<1,1>(),[iri(1)],true),[0],[0],[[0,0],[0,0]],a);
}
#[test] fn closed_join_preserves_bag_multiplicity(){
    let a=crate::table::append(crate::table::append(crate::table::empty::<2,1>(),[iri(1)],true),[iri(1)],true);
    join(a,crate::table::append(crate::table::empty::<1,1>(),[iri(1)],true),[0],[0],[[0,0],[1,0]],a);
}

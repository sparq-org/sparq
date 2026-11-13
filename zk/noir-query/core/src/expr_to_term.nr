// Research grade; not externally audited.
use std::collections::bounded_vec::BoundedVec;
use crate::expr_core::{Lane, Numeric, check_numeric, INTEGER, DECIMAL, SCALE};
use crate::expr_term_contract::{TermCell, KIND_LITERAL, CLASS_BOOLEAN, CLASS_DECIMAL, DT_BOOLEAN, DT_INTEGER, DT_DECIMAL};

// Static integration seam: implement ONLY by calling lane A's term::open_term
// with these bytes, empty language, credential zero and the supplied key.
// The live integration test instantiates that constructor from a pinned snapshot.
// Replace this seam with a direct term::open_term call when the lanes are combined.
pub trait ComputedTermEncoder {
    fn literal(lexical: BoundedVec<u8,64>, datatype: BoundedVec<u8,64>, key: [u8;31]) -> TermCell;
}
pub fn unbound() -> TermCell {
    TermCell { bound:false, kind:0, enc:0, key_class:0, key:0, dt:0, lang:0, credential:0 }
}
fn padded<let N:u32>(bytes:[u8;N]) -> BoundedVec<u8,64> {
    assert(N<=64,"computed lexical capacity");
    let mut out=BoundedVec::new();
    for b in bytes { out.push(b); }
    out
}
fn encode<E>(lexical:BoundedVec<u8,64>,datatype:BoundedVec<u8,64>,key:Field,dt:Field,class:u8)->TermCell
where E:ComputedTermEncoder {
    let cell=E::literal(lexical,datatype,key.to_be_bytes::<31>());
    assert(cell.bound() & (cell.kind()==KIND_LITERAL) & (cell.key_class()==class)
        & (cell.key()==key) & (cell.datatype()==dt) & (cell.credential()==0),"computed term contract");
    cell
}
pub fn boolean<E>(input:(Lane<bool>,bool))->(Lane<TermCell>,bool)
where E:ComputedTermEncoder {
    let (a,error)=input;
    let key=CLASS_BOOLEAN as Field*0x1000000000000000000000000000000000000000000000000000000000000
        + if a.value {0x10000000000000000000000000000000000000000000000000000000000} else {0};
    let lexical=if a.value {padded("true".as_bytes())} else {padded("false".as_bytes())};
    let value=if error | a.capacity {unbound()} else {
        encode::<E>(lexical,padded("http://www.w3.org/2001/XMLSchema#boolean".as_bytes()),key,DT_BOOLEAN,CLASS_BOOLEAN)
    };
    (Lane {value,capacity:a.capacity},error)
}
// No lexical witness: repeated quotient/remainder constraints determine every
// digit. At most 34 significant scaled digits plus sign/dot fit this bucket.
pub fn numeric_lexical(input:Numeric)->BoundedVec<u8,64> {
    check_numeric(input);
    assert(input.ty<=DECIMAL,"computed numeric term type");
    let magnitude=input.magnitude as u128;
    let mut out=BoundedVec::new();
    if input.negative {out.push(45);}
    let mut whole=magnitude/(SCALE as u128);
    let mut reverse=[0;34];
    let mut len:u32=0;
    for i in 0..34 {
        if (whole!=0) | (i==0) {
            reverse[i]=(whole%10) as u8+48;
            whole=whole/10;len=len+1;
        }
    }
    assert(whole==0,"computed numeric lexical capacity");
    for i in 0..34 {if i<len {out.push(reverse[len-1-i]);}}
    if input.ty==DECIMAL {
        out.push(46);
        let mut fraction=magnitude%(SCALE as u128);
        let mut divisor:u128=100000000000000000;
        for i in 0..18 {
            if (fraction!=0) | (i==0) {
                out.push((fraction/divisor) as u8+48);
                fraction=fraction%divisor;
            }
            divisor=divisor/10;
        }
    }
    out
}
pub fn numeric<E>(input:(Lane<Numeric>,bool))->(Lane<TermCell>,bool)
where E:ComputedTermEncoder {
    let (a,error)=input;
    // Floating term serialization is outside this checkpoint's numeric bucket.
    let capacity=a.capacity | (!error & (a.value.ty>DECIMAL));
    let value=if error | capacity {unbound()} else {
        let n=a.value;
        let bias:Field=0x800000000000000000000000000000000000000000000000000000000000;
        let key=CLASS_DECIMAL as Field*0x1000000000000000000000000000000000000000000000000000000000000
            + if n.negative {bias-n.magnitude} else {bias+n.magnitude};
        let (datatype,dt)=if n.ty==INTEGER {
            (padded("http://www.w3.org/2001/XMLSchema#integer".as_bytes()),DT_INTEGER)
        } else {(padded("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()),DT_DECIMAL)};
        encode::<E>(numeric_lexical(n),datatype,key,dt,CLASS_DECIMAL)
    };
    (Lane {value,capacity},error)
}

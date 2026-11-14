//! Checked-key transport into the shared exact numeric and result gadgets.
use crate::table::TermCell;
use crate::expr_core::{Lane,Numeric,DECIMAL,INTEGER,EXACT_LIMIT};
pub global OFFSET:Field=0x800000000000000000000000000000000000000000000000000000000000;
pub global PREFIX:Field=0x1000000000000000000000000000000000000000000000000000000000000;
pub fn read(key:Field,decimal:bool)->(Lane<Numeric>,bool){
    key.assert_max_bit_size::<240>();let negative=key.lt(OFFSET);
    let magnitude=if negative {OFFSET-key}else{key-OFFSET};
    let capacity=!magnitude.lt(EXACT_LIMIT);
    let value=crate::expr_core::exact(if decimal {DECIMAL}else{INTEGER},negative & !capacity,if capacity {0}else{magnitude});
    (Lane{value,capacity},false)
}
pub fn encode(key:Field,decimal:bool)->TermCell {
    let (mut input,error)=read(key,decimal);input.value.ty=if decimal {DECIMAL}else{INTEGER};
    let (value,_error)=crate::expr_to_term::numeric::<crate::ops_closed_extend::SuiteEncoder>((input,error));
    assert(!value.capacity,"aggregate numeric capacity");
    let mut c=crate::ops_closed_extend::table_cell(value.value);
    c.key=c.key-PREFIX;c.lang=0;c
}

// Keep exact inputs statically exact at the shared arithmetic boundary. This
// lane admits no floating operands; preserve their error/capacity channels.
fn exact_lane(a:(Lane<Numeric>,bool))->(Lane<Numeric>,bool){
    let (a,e)=a;let n=a.value;let absent=e|a.capacity;
    (Lane{value:Numeric{ty:DECIMAL,negative:n.negative & !absent,magnitude:if absent {0}else{n.magnitude},bits:0},capacity:a.capacity},e)
}
pub fn binary(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool),op:u8)->(Lane<Numeric>,bool){
    let (x,xe)=exact_lane(a);let (y,ye)=exact_lane(b);
    let (out,error)=crate::expr_arith::binary((x,false),(y,false),op);
    (out,error|xe|ye)
}
pub fn compare(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool),op:u8)->(Lane<bool>,bool){
    let (x,xe)=exact_lane(a);let (y,ye)=exact_lane(b);
    let (out,error)=crate::expr_compare::numeric_compare((x,false),(y,false),op);
    (out,error|xe|ye)
}

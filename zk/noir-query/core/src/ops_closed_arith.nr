//! Numeric bytecode wiring over the shared arithmetic and comparison gadgets.
use crate::table::{TermCell,Table};
use crate::ops_core::Boolean;
use crate::expr_core::{Lane,Numeric};
use crate::ops_closed_numeric::{PREFIX};
pub struct Step {pub op:u8,pub a:u32,pub b:u32,pub parameter:u32}
pub fn integer(c:TermCell)->(Lane<Numeric>,bool){
    if !c.bound {(crate::expr_core::lane(crate::expr_core::zero_numeric()),true)} else {
        let numeric=(c.kind==3)&(c.key_class==1)&((c.dt==crate::suite_constants::DT_INTEGER)|(c.dt==crate::suite_constants::DT_DECIMAL));
        assert(c.key_class!=127,"numeric capacity");
        if numeric {crate::ops_closed_numeric::read(c.key-PREFIX,c.dt==crate::suite_constants::DT_DECIMAL)}
        else {(crate::expr_core::lane(crate::expr_core::zero_numeric()),true)}
    }
}
pub fn add(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool))->(Lane<Numeric>,bool){crate::ops_closed_numeric::binary(a,b,crate::expr_arith::ADD)}
pub fn subtract(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool))->(Lane<Numeric>,bool){crate::ops_closed_numeric::binary(a,b,crate::expr_arith::SUB)}
pub fn multiply(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool))->(Lane<Numeric>,bool){crate::ops_closed_numeric::binary(a,b,crate::expr_arith::MUL)}
pub fn compare(a:(Lane<Numeric>,bool),b:(Lane<Numeric>,bool),op:u8)->Boolean{
    let (v,e)=crate::ops_closed_numeric::compare(a,b,op);
    assert(!v.capacity,"numeric capacity");Boolean{value:v.value,is_error:e}
}
pub fn evaluate<let R:u32,let V:u32,let P:u32,let E:u32>(input:Table<R,V>,parameters:[TermCell;P],steps:[Step;E],op:u8,left:u32,right:u32)->[Boolean;R]{
    crate::table::check_table(input);
    assert((left<E)&(right<E),"integer roots");
    for s in 0..E {let step=steps[s];
        if step.op==0 {assert(step.parameter<=V,"integer column");}
        else if step.op==1 {assert(step.parameter<P,"integer parameter");}
        else {assert((step.a<s)&(step.b<s),"integer topology");assert((step.op>=2)&(step.op<=4),"integer opcode");}
    }
    let mut output=[Boolean{value:false,is_error:true};R];
    for r in 0..R {if r<input.len {
        let mut values=[(crate::expr_core::lane(crate::expr_core::zero_numeric()),true);E];
        for s in 0..E {
            let step=steps[s];
            if step.op==0 {let mut c=crate::table::unbound();for v in 0..V {if v==step.parameter {c=input.rows[r][v];}}values[s]=integer(c);}
            else if step.op==1 {assert(step.parameter<P,"integer parameter");values[s]=integer(parameters[step.parameter]);}
            else {assert((step.a<s)&(step.b<s),"integer topology");let a=values[step.a];let b=values[step.b];
                if step.op==2 {values[s]=add(a,b);}else if step.op==3 {values[s]=subtract(a,b);}else {assert(step.op==4,"integer opcode");values[s]=multiply(a,b);}}
        }
        assert((left<E)&(right<E),"integer roots");output[r]=compare(values[left],values[right],op);
    }}
    output
}

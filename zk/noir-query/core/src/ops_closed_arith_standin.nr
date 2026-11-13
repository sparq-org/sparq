//! STAND-IN: lane D replaces these typed integer value/error functions.
use crate::table::{TermCell,Table};
use crate::ops_core::Boolean;
use crate::ops_closed_numeric_standin::{OFFSET,PREFIX};
use crate::suite_constants::DT_INTEGER;
pub struct Step {pub op:u8,pub a:u32,pub b:u32,pub parameter:u32}
fn checked(value:Field)->Field {
    (value*1000000000000000000+OFFSET).assert_max_bit_size::<240>();value
}
pub fn integer(c:TermCell)->(Field,bool){
    let mut value:Field=0;
    if c.bound {
        assert((c.kind==3) & (c.key_class==1) & (c.dt==DT_INTEGER),"lane D integer arithmetic operands");
        let payload=c.key-PREFIX;payload.assert_max_bit_size::<240>();
        let magnitude=if payload.lt(OFFSET) {OFFSET-payload} else {payload-OFFSET};
        (magnitude/1000000000000000000).assert_max_bit_size::<180>();
        value=(payload-OFFSET)/1000000000000000000;
    }
    (value,!c.bound)
}
pub fn add(a:(Field,bool),b:(Field,bool))->(Field,bool){let error=a.1|b.1;(if error {0}else{checked(a.0+b.0)},error)}
pub fn subtract(a:(Field,bool),b:(Field,bool))->(Field,bool){let error=a.1|b.1;(if error {0}else{checked(a.0-b.0)},error)}
pub fn multiply(a:(Field,bool),b:(Field,bool))->(Field,bool){
    let error=a.1|b.1;
    // Restrict operands before multiplication so field reduction cannot imitate overflow.
    if !error {
        let ao=a.0*1000000000000000000+OFFSET;let bo=b.0*1000000000000000000+OFFSET;
        let am=if ao.lt(OFFSET) {OFFSET-ao}else{ao-OFFSET};let bm=if bo.lt(OFFSET) {OFFSET-bo}else{bo-OFFSET};
        let ai=am/1000000000000000000;let bi=bm/1000000000000000000;
        ai.assert_max_bit_size::<120>();bi.assert_max_bit_size::<120>();
        (ai*bi).assert_max_bit_size::<180>();
    }
    (if error {0}else{checked(a.0*b.0)},error)
}
pub fn compare(a:(Field,bool),b:(Field,bool),op:u8)->Boolean{
    assert(op<=4,"integer comparison");
    let av=a.0*1000000000000000000+OFFSET;let bv=b.0*1000000000000000000+OFFSET;
    let lt=av.lt(bv);let gt=bv.lt(av);
    Boolean {value:if op==0 {av==bv}else if op==1 {lt}else if op==2 {!gt}else if op==3 {gt}else{!lt},is_error:a.1|b.1}
}
pub fn evaluate<let R:u32,let V:u32,let P:u32,let E:u32>(input:Table<R,V>,parameters:[TermCell;P],steps:[Step;E],op:u8,left:u32,right:u32)->[Boolean;R]{
    let mut output=[Boolean{value:false,is_error:true};R];
    for r in 0..R {
        let mut values=[(0,true);E];
        for s in 0..E {
            let step=steps[s];
            if step.op==0 {let mut c=crate::table::unbound();for v in 0..V {if v==step.parameter {c=input.rows[r][v];}}values[s]=integer(c);}
            else if step.op==1 {assert(step.parameter<P,"integer parameter");values[s]=integer(parameters[step.parameter]);}
            else {assert((step.a<s)&(step.b<s),"integer topology");let a=values[step.a];let b=values[step.b];
                if step.op==2 {values[s]=add(a,b);}else if step.op==3 {values[s]=subtract(a,b);}else {assert(step.op==4,"integer opcode");values[s]=multiply(a,b);}}
        }
        assert((left<E)&(right<E),"integer roots");output[r]=compare(values[left],values[right],op);
    }
    output
}

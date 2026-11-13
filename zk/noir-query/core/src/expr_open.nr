// Research grade; not externally audited.
// Narrow adapter for the target-query value bucket. The static opening trait
// must call lane A's open_lexical and recompute_key; a bare cell is not accepted.
use crate::expr_core::{Scalar, Lane, zero_numeric, StringLane, TemporalLane,
    TERM, NUMERIC, BOOLEAN, STRING, DATE, DATETIME, INTEGER, DECIMAL, exact};
use crate::expr_term_contract::{CheckedLexicalOpening, read, key_payload, CLASS_NONE,
    CLASS_UNREPRESENTABLE, CLASS_DECIMAL, CLASS_BOOLEAN, CLASS_STRING, CLASS_STRING_PREFIX,
    CLASS_DATE, CLASS_DATE_TIME, CLASS_LOCAL_DATE_TIME, DT_INTEGER, DT_DECIMAL, DT_BOOLEAN,
    DT_STRING, DT_DATE, DT_DATETIME, DT_DOUBLE, DT_FLOAT, DT_NONPOSITIVEINTEGER,
    DT_NEGATIVEINTEGER, DT_LONG, DT_INT, DT_SHORT, DT_BYTE, DT_NONNEGATIVEINTEGER,
    DT_UNSIGNEDLONG, DT_UNSIGNEDINT, DT_UNSIGNEDSHORT, DT_UNSIGNEDBYTE, DT_POSITIVEINTEGER,
    KIND_LITERAL};

pub fn variable<T,let L:u32,let D:u32,let G:u32>(opening:T)->(Lane<Scalar<L>>,bool)
where T:CheckedLexicalOpening<L,D,G> {
    let o=read(opening);
    let cell=o.cell;
    let mut s=Scalar {tag:TERM,valid:true,boolean:false,number:zero_numeric(),
        string:StringLane {bytes:[0;L],len:0,lang:0},
        temporal:TemporalLane {key_class:0,key:0},term:cell};
    let mut capacity=false;
    if cell.bound() & (cell.kind()==KIND_LITERAL) {
        let dt=cell.datatype();
        let integer=(dt==DT_INTEGER)|(dt==DT_NONPOSITIVEINTEGER)|(dt==DT_NEGATIVEINTEGER)
            |(dt==DT_LONG)|(dt==DT_INT)|(dt==DT_SHORT)|(dt==DT_BYTE)|(dt==DT_NONNEGATIVEINTEGER)
            |(dt==DT_UNSIGNEDLONG)|(dt==DT_UNSIGNEDINT)|(dt==DT_UNSIGNEDSHORT)
            |(dt==DT_UNSIGNEDBYTE)|(dt==DT_POSITIVEINTEGER);
        let class=cell.key_class();
        let payload=key_payload(cell);
        if integer | (dt==DT_DECIMAL) {
            s.tag=NUMERIC;s.valid=class!=CLASS_NONE;
            capacity=class==CLASS_UNREPRESENTABLE;
            if class==CLASS_DECIMAL {
                let bias:Field=0x800000000000000000000000000000000000000000000000000000000000;
                let negative=payload.lt(bias);
                let magnitude=if negative {bias-payload} else {payload-bias};
                capacity=!magnitude.lt(crate::expr_core::EXACT_LIMIT);
                if !capacity {s.number=exact(if integer {INTEGER} else {DECIMAL},negative,magnitude);}
            } else {assert((class==CLASS_NONE)|(class==CLASS_UNREPRESENTABLE),"numeric key class");}
        } else if dt==DT_BOOLEAN {
            s.tag=BOOLEAN;s.valid=class==CLASS_BOOLEAN;
            assert((class==CLASS_BOOLEAN)|(class==CLASS_NONE),"boolean key class");
            let true_key:Field=0x10000000000000000000000000000000000000000000000000000000000;
            assert((payload==0)|((class==CLASS_BOOLEAN)&(payload==true_key)),"boolean key payload");
            s.boolean=payload!=0;
        } else if dt==DT_STRING {
            assert((class==CLASS_STRING)|(class==CLASS_STRING_PREFIX),"string key class");
            let mut bytes=[0;L];
            for i in 0..L {bytes[i]=o.lexical.get_unchecked(i);}
            s.tag=STRING;s.string=StringLane {bytes,len:o.lexical.len(),lang:0};
        } else if (dt==DT_DATE)|(dt==DT_DATETIME) {
            s.tag=if dt==DT_DATE {DATE} else {DATETIME};
            s.valid=class!=CLASS_NONE;
            capacity=class==CLASS_UNREPRESENTABLE;
            if !capacity & s.valid {
                assert(if dt==DT_DATE {class==CLASS_DATE} else {(class==CLASS_DATE_TIME)|(class==CLASS_LOCAL_DATE_TIME)},"temporal key class");
                let bytes=payload.to_be_bytes::<30>();
                let mut key:Field=0;
                for i in 0..30 {if i<8 {key=key*256+bytes[i] as Field;} else {assert(bytes[i]==0,"temporal key padding");}}
                s.temporal=TemporalLane {key_class:class,key};
            }
        } else if (dt==DT_FLOAT)|(dt==DT_DOUBLE) {
            // The full IEEE lexical adapter follows this target-query checkpoint.
            capacity=true;
        }
    }
    (Lane {value:s,capacity},!cell.bound())
}

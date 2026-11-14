//! Adapts authenticated suite keys to the shared expression gadgets.
//! The adapter uses only checked keys; it accepts no private scalar payload.

use crate::table::{TermCell, Table, unbound};
use crate::ops_core::Boolean;
use crate::suite_constants::{DT_STRING, DT_LANGSTRING, DT_INTEGER, DT_DECIMAL, DT_DOUBLE,
    DT_FLOAT, DT_BOOLEAN, DT_NONPOSITIVEINTEGER, DT_NEGATIVEINTEGER,
    DT_LONG, DT_INT, DT_SHORT, DT_BYTE, DT_NONNEGATIVEINTEGER, DT_UNSIGNEDLONG,
    DT_UNSIGNEDINT, DT_UNSIGNEDSHORT, DT_UNSIGNEDBYTE, DT_POSITIVEINTEGER};
use crate::expr_core::{Lane, Scalar, Numeric, StringLane, TemporalLane, truth,
    zero_numeric, TERM, BOOLEAN, NUMERIC, STRING, DATE, DATETIME, DECIMAL};
use crate::{expr_core, expr_compare};

pub struct Step {
    pub op: u8,
    pub a: u32,
    pub b: u32,
    pub parameter: u32,
    pub comparison: u8,
}
struct Value {
    term: TermCell,
    boolean: (Lane<bool>, bool),
    is_boolean: bool,
}
fn error() -> Value { boolean(truth(false, true)) }
fn boolean(value: (Lane<bool>, bool)) -> Value {
    Value { term: unbound(), boolean: value, is_boolean: true }
}
fn term(value: TermCell) -> Value {
    Value { term: value, boolean: truth(false, true), is_boolean: false }
}

fn numeric_or_boolean(dt:Field) -> bool {
    (dt == DT_INTEGER) | (dt == DT_DECIMAL) | (dt == DT_DOUBLE) | (dt == DT_FLOAT)
        | (dt == DT_BOOLEAN) | (dt == DT_NONPOSITIVEINTEGER) | (dt == DT_NEGATIVEINTEGER)
        | (dt == DT_LONG) | (dt == DT_INT) | (dt == DT_SHORT) | (dt == DT_BYTE)
        | (dt == DT_NONNEGATIVEINTEGER) | (dt == DT_UNSIGNEDLONG) | (dt == DT_UNSIGNEDINT)
        | (dt == DT_UNSIGNEDSHORT) | (dt == DT_UNSIGNEDBYTE) | (dt == DT_POSITIVEINTEGER)
}
fn is_string(cell:TermCell) -> bool {
    cell.bound & (cell.kind == 3) & ((cell.key_class == 6) | (cell.key_class == 7))
}

/// Leading scalar word of a checked suite temporal key, with canonical padding.
/// The input includes the suite class header; the result is its biased time word.
pub fn temporal_payload(key:Field) -> Field {
    let bytes=key.to_be_bytes::<32>();
    let mut instant:u64=0;
    for i in 2..10 {instant=instant*256+bytes[i] as u64;}
    for i in 10..32 {assert(bytes[i]==0,"temporal key zero tail");}
    instant as Field
}

fn term_contract(cell:TermCell) -> crate::expr_term_contract::TermCell {
    crate::expr_term_contract::TermCell {bound:cell.bound,kind:cell.kind,enc:cell.enc,
        key_class:cell.key_class,key:cell.key,dt:cell.dt,lang:cell.lang,credential:cell.scope}
}

// Ordering uses the checked short-string key, whose zero padding preserves order.
// Equality of strings with the same datatype delegates directly to shared term
// identity. Other pairs retain the shared class and error dispatch.
fn scalar(cell: TermCell, effective_boolean: bool, class_only:bool) -> (Lane<Scalar<32>>, bool) {
    let mut value = Scalar {
        tag: TERM, valid: true, boolean: false, number: zero_numeric(),
        string: StringLane { bytes: [0; 32], len: 0, lang: 0 },
        temporal: TemporalLane { key_class: 0, key: 0 },
        term: term_contract(cell),
    };
    let mut capacity = false;
    let invalid_ebv = cell.bound & (cell.kind == 3) & (cell.key_class == 0)
        & numeric_or_boolean(cell.dt);
    if cell.bound & (cell.kind == 3) {
        if effective_boolean {
            assert(cell.key_class != 0x7f, "literal value range");
            assert(cell.key_class != 2, "floating-point EBV not yet supported");
        } else {
            assert(cell.key_class != 0x7f, "literal comparison range");

        }
        let prefix: Field = cell.key_class as Field * 0x1000000000000000000000000000000000000000000000000000000000000;
        if invalid_ebv & effective_boolean {
            value.tag = if cell.dt == DT_BOOLEAN { BOOLEAN } else { NUMERIC };
            value.valid = false;
        } else if cell.key_class == 1 {
            let zero: Field = 0x1800000000000000000000000000000000000000000000000000000000000;
            let negative = cell.key.lt(zero);
            let magnitude = if negative { zero - cell.key } else { cell.key - zero };
            capacity = !magnitude.lt(expr_core::EXACT_LIMIT);
            value.tag = NUMERIC;
            value.number = Numeric { ty: DECIMAL, negative: negative & !capacity,
                magnitude: if capacity { 0 } else { magnitude }, bits: 0 };
        } else if cell.key_class == 5 {
            value.tag = BOOLEAN;
            value.boolean = cell.key != prefix;
        } else if (cell.key_class == 3) | (cell.key_class == 4) | (cell.key_class == 8) {
            value.tag = if cell.key_class == 8 { DATE } else { DATETIME };
            value.temporal = TemporalLane { key_class: cell.key_class, key: temporal_payload(cell.key) };
        } else if (cell.key_class == 6) | (cell.key_class == 7) {
            value.tag = STRING;
            value.string.lang = if cell.dt == DT_STRING { 0 } else { cell.lang };
            if class_only {
                value.string.lang = if cell.dt == DT_STRING {0} else {1};
            } else if effective_boolean {
                let nonempty = (cell.key_class == 7) | (cell.key != prefix);
                value.string.len = nonempty as u32;
                value.string.bytes[0] = nonempty as u8;
            } else {
                assert(cell.key_class == 6, "string comparison capacity");
                let bytes = cell.key.to_be_bytes::<32>();
                for i in 0..30 {
                    value.string.bytes[i] = bytes[i + 2];
                    if bytes[i + 2] != 0 { value.string.len = i + 1; }
                }
            }
        }
    }
    (Lane { value, capacity }, !cell.bound)
}
fn ebv(value: Value) -> (Lane<bool>, bool) {
    if value.is_boolean { value.boolean }
    else {
        assert(!value.term.bound | (value.term.dt != DT_LANGSTRING), "language-tagged effective boolean value not yet supported");
        expr_core::ebv(scalar(value.term, true, false))
    }
}
fn compare(op: u8, a: Value, b: Value, membership: bool) -> (Lane<bool>, bool) {
    assert(op <= 4, "comparison instruction");
    assert(!membership | (op == 0), "membership comparison instruction");
    if a.is_boolean & b.is_boolean {
        let (av, ae) = a.boolean; let (bv, be) = b.boolean;
        let mut left = scalar(unbound(), false, false);
        let mut right = left;
        let (mut x, _) = left; let (mut y, _) = right;
        x.value.tag = BOOLEAN; x.value.boolean = av.value; x.value.term.bound = true;
        x.capacity = av.capacity;
        y.value.tag = BOOLEAN; y.value.boolean = bv.value; y.value.term.bound = true;
        y.capacity = bv.capacity;
        left = (x, ae); right = (y, be);
        if membership { expr_compare::in_list(left, [right]) }
        else { expr_compare::value_compare(left, right, op) }
    } else if !a.is_boolean & !b.is_boolean {
        let a = a.term; let b = b.term;
        if !a.bound | !b.bound { truth(false, true) } else {
        if a.bound & b.bound & (a.kind == 3) & (b.kind == 3) {
            assert((a.key_class != 2) & (b.key_class != 2), "floating-point comparison not yet supported");
            assert((op != 0) | ((a.key_class != 0) & (b.key_class != 0)) | is_string(a) | is_string(b) | (a.dt == DT_LANGSTRING) | (b.dt == DT_LANGSTRING), "literal equality without a supported value key");
            let strings = is_string(a) & is_string(b);
            assert((op == 0) | !strings | ((a.dt != DT_LANGSTRING) & (b.dt != DT_LANGSTRING)
                & ((a.lang == 0) | (a.dt == DT_STRING)) & ((b.lang == 0) | (b.dt == DT_STRING))), "language-tagged string ordering not yet supported");
            let temporal_a = (a.key_class == 3) | (a.key_class == 4) | (a.key_class == 8);
            let temporal_b = (b.key_class == 3) | (b.key_class == 4) | (b.key_class == 8);
            assert(!(temporal_a & temporal_b) | (a.dt == b.dt), "mixed temporal datatypes not yet supported");
            assert(!(temporal_a & temporal_b) | (a.key_class == b.key_class), "mixed timezone comparison not yet supported");
        }
        let string_pair = is_string(a) & is_string(b);
        let same_string_type = string_pair & (a.dt == b.dt);
        if (op == 0) & same_string_type {
            expr_compare::same_term((Lane {value:term_contract(a),capacity:false},false),
                (Lane {value:term_contract(b),capacity:false},false))
        } else {
        // Under equality, same-type strings have already taken the identity arm.
        // The remaining scalar dispatch needs only string classes, never bytes.
        let class_only = !string_pair | (op == 0);
        let (mut x, xe) = scalar(a, false, class_only);
        let (mut y, ye) = scalar(b, false, class_only);
        let result = if (x.value.tag == NUMERIC) & (y.value.tag == NUMERIC) {
            // Keep the exact-only bucket statically typed. Error and capacity
            // remain separate, and no IEEE conversion is instantiated here.
            let xn = Numeric { ty: DECIMAL, negative: x.value.number.negative,
                magnitude: x.value.number.magnitude, bits: 0 };
            let yn = Numeric { ty: DECIMAL, negative: y.value.number.negative,
                magnitude: y.value.number.magnitude, bits: 0 };
            expr_compare::numeric_compare((Lane { value: xn, capacity: x.capacity }, false),
                (Lane { value: yn, capacity: y.capacity }, false), op)
        } else {
            x.value.number = zero_numeric(); y.value.number = zero_numeric();
            if membership { expr_compare::in_list((x, xe), [(y, ye)]) }
            else { expr_compare::value_compare((x, xe), (y, ye), op) }
        };
        // Credential-local blank labels retain their distinct scopes.
        if op == 0 { expr_core::logical_and(result, truth(a.scope == b.scope, false)) }
        else { result }
        }
        }
    } else {
        assert(false, "comparison of boolean expression and term not yet supported");
        truth(false, true)
    }
}

pub fn evaluate<let R: u32, let V: u32, let P: u32, let E: u32>(
    input: Table<R, V>, parameters: [TermCell; P], steps: [Step; E],
) -> [Boolean; R] {
    evaluate_indexed(input,parameters,[0],steps)
}
fn check_steps<let P:u32,let I:u32,let E:u32>(steps:[Step;E],indices:[u32;I]) {
    for s in 0..E {
        let step=steps[s];
        if step.op == 1 {
            assert(step.parameter < P, "expression parameter");
        } else if step.op == 8 {
            assert(step.a < s, "expression topology");
            assert(step.b < I, "IN length parameter");
            assert((step.parameter as u64 + 16) <= P as u64, "IN term parameters");
            let count=indices[step.b];
            assert(count <= 16, "IN list capacity");
        } else if (step.op != 0) & (step.op != 7) {
            assert((step.a < s) & (step.b < s), "expression topology");
            assert((step.op >= 2) & (step.op <= 6), "expression opcode");
            if (step.op == 2) | (step.op == 6) {
                assert(step.comparison <= 4, "comparison instruction");
                assert((step.op != 6) | (step.comparison == 0), "membership comparison instruction");
            }
        }
    }
}
pub fn evaluate_indexed<let R:u32, let V:u32, let P:u32, let I:u32, let E:u32>(
    input:Table<R,V>, parameters:[TermCell;P], indices:[u32;I], steps:[Step;E],
) -> [Boolean;R] {
    std::static_assert(E > 0, "expression root");
    check_steps::<P,I,E>(steps,indices);
    let mut results = [Boolean {value: false, is_error: true}; R];
    for r in 0..R {
        let mut result=Boolean {value:false,is_error:true};
        if r < input.len {
        let mut values = [error(); E];
        for s in 0..E {
            let step = steps[s];
            if step.op == 0 {
                let mut cell = unbound();
                for v in 0..V { if step.parameter == v { cell = input.rows[r][v]; } }
                values[s] = term(cell);
            } else if step.op == 1 {
                values[s] = term(parameters[step.parameter]);
            } else if step.op == 7 {
                values[s] = boolean(truth(false, false));
            } else if step.op == 8 {
                let count = indices[step.b];
                let mut result = truth(false,false);
                for i in 0..16 {
                    if i < count {
                        result = expr_core::logical_or(result,
                            compare(0,values[step.a],term(parameters[step.parameter+i]),true));
                    }
                }
                values[s] = boolean(result);
            } else {
                let a = values[step.a]; let b = values[step.b];
                if (step.op == 2) | (step.op == 6) {
                    values[s] = boolean(compare(step.comparison, a, b, step.op == 6));
                } else if step.op == 3 {
                    values[s] = boolean(expr_core::logical_and(ebv(a), ebv(b)));
                } else if step.op == 4 {
                    values[s] = boolean(expr_core::logical_or(ebv(a), ebv(b)));
                } else {
                    values[s] = boolean(expr_core::logical_not(ebv(a)));
                }
            }
        }
        let root = ebv(values[E - 1]);
        let (_, is_error) = root;
        result = Boolean { value: expr_core::filter_accept(root), is_error };
        }
        results[r]=result;
    }
    results
}

//! Adapts authenticated suite keys to the shared expression gadgets.
//! The adapter uses only checked keys; it accepts no private scalar payload.

use crate::table::{TermCell, Table, unbound};
use crate::ops_core::Boolean;
use crate::suite_constants::{DT_STRING, DT_LANGSTRING};
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

// Equality uses a term-identity token for strings. Ordering uses the checked
// short-string key, whose zero padding preserves the lexical order. Neither
// representation is exposed as a lexical opening to other expression gadgets.
fn scalar(cell: TermCell, equality: bool, effective_boolean: bool) -> (Lane<Scalar<32>>, bool) {
    let mut value = Scalar {
        tag: TERM, valid: true, boolean: false, number: zero_numeric(),
        string: StringLane { bytes: [0; 32], len: 0, lang: 0 },
        temporal: TemporalLane { key_class: 0, key: 0 },
        term: crate::expr_term_contract::TermCell {
            bound: cell.bound, kind: if cell.kind > 0 { cell.kind - 1 } else { 0 },
            enc: cell.enc, key_class: cell.key_class, key: cell.key, dt: cell.dt, lang: cell.lang,
        },
    };
    let mut capacity = false;
    if cell.bound & (cell.kind == 3) {
        if effective_boolean {
            assert(cell.key_class != 0x7f, "literal value range");
            assert(cell.key_class != 2, "floating-point EBV not yet supported");
        } else {
            assert(cell.key_class != 0x7f, "literal comparison range");

        }
        let prefix: Field = cell.key_class as Field * 0x1000000000000000000000000000000000000000000000000000000000000;
        if cell.key_class == 1 {
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
            value.temporal = TemporalLane { key_class: cell.key_class, key: cell.key - prefix };
        } else if (cell.key_class == 6) | (cell.key_class == 7) {
            value.tag = STRING;
            value.string.lang = if cell.dt == DT_STRING { 0 } else { cell.lang };
            if effective_boolean {
                let nonempty = (cell.key_class == 7) | (cell.key != prefix);
                value.string.len = nonempty as u32;
                value.string.bytes[0] = nonempty as u8;
            } else if equality {
                value.string.bytes = cell.enc.to_be_bytes::<32>();
                value.string.len = 32;
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
        expr_core::ebv(scalar(value.term, false, true))
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
        if !a.bound | !b.bound { return truth(false, true); }
        if a.bound & b.bound & (a.kind == 3) & (b.kind == 3) {
            assert((a.key_class != 2) & (b.key_class != 2), "floating-point comparison not yet supported");
            assert((op != 0) | ((a.key_class != 0) & (b.key_class != 0)), "literal equality without a supported value key");
            let strings = ((a.key_class == 6) | (a.key_class == 7))
                & ((b.key_class == 6) | (b.key_class == 7));
            assert((op == 0) | !strings | (((a.lang == 0) | (a.dt == DT_STRING))
                & ((b.lang == 0) | (b.dt == DT_STRING))), "language-tagged string ordering not yet supported");
            let temporal_a = (a.key_class == 3) | (a.key_class == 4) | (a.key_class == 8);
            let temporal_b = (b.key_class == 3) | (b.key_class == 4) | (b.key_class == 8);
            assert(!(temporal_a & temporal_b) | (a.dt == b.dt), "mixed temporal datatypes not yet supported");
            assert(!(temporal_a & temporal_b) | (a.key_class == b.key_class), "mixed timezone comparison not yet supported");
        }
        let string_pair = ((a.key_class == 6) | (a.key_class == 7))
            & ((b.key_class == 6) | (b.key_class == 7));
        let (mut x, xe) = scalar(a, (op == 0) | !string_pair, false);
        let (mut y, ye) = scalar(b, (op == 0) | !string_pair, false);
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
    } else {
        assert(false, "comparison of boolean expression and term not yet supported");
        truth(false, true)
    }
}

pub fn evaluate<let R: u32, let V: u32, let P: u32, let E: u32>(
    input: Table<R, V>, parameters: [TermCell; P], steps: [Step; E],
) -> [Boolean; R] {
    std::static_assert(E > 0, "expression root");
    let mut results = [Boolean {value: false, is_error: true}; R];
    for r in 0..R {
        let mut values = [error(); E];
        for s in 0..E {
            let step = steps[s];
            if step.op == 0 {
                let mut cell = unbound();
                for v in 0..V { if step.parameter == v { cell = input.rows[r][v]; } }
                values[s] = term(cell);
            } else if step.op == 1 {
                assert(step.parameter < P, "expression parameter");
                values[s] = term(parameters[step.parameter]);
            } else if step.op == 7 {
                values[s] = boolean(truth(false, false));
            } else {
                assert((step.a < s) & (step.b < s), "expression topology");
                let a = values[step.a]; let b = values[step.b];
                if (step.op == 2) | (step.op == 6) {
                    values[s] = boolean(compare(step.comparison, a, b, step.op == 6));
                } else if step.op == 3 {
                    values[s] = boolean(expr_core::logical_and(ebv(a), ebv(b)));
                } else if step.op == 4 {
                    values[s] = boolean(expr_core::logical_or(ebv(a), ebv(b)));
                } else {
                    assert(step.op == 5, "expression opcode");
                    values[s] = boolean(expr_core::logical_not(ebv(a)));
                }
            }
        }
        let root = ebv(values[E - 1]);
        let (_, is_error) = root;
        results[r] = Boolean { value: expr_core::filter_accept(root), is_error };
    }
    results
}

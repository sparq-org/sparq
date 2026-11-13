//! Initial key-comparison, boolean and membership expression contract.
//! Lane D's typed expression gadgets replace this dispatch at integration.

use crate::table::{TermCell, Table, unbound, same_term};
use crate::ops_core::Boolean;

pub struct Step {
    pub op: u8,
    pub a: u32,
    pub b: u32,
    pub parameter: u32,
    pub comparison: u8,
}

struct Value {
    term: TermCell,
    boolean: bool,
    is_boolean: bool,
    is_error: bool,
}

fn error() -> Value { Value {term: unbound(), boolean: false, is_boolean: true, is_error: true} }
fn boolean(value: bool, is_error: bool) -> Value { Value {term: unbound(), boolean: value, is_boolean: true, is_error} }

fn ebv(value: Value) -> Boolean {
    let mut result = Boolean {value: false, is_error: true};
    if value.is_boolean {
        result = Boolean {value: value.boolean, is_error: value.is_error};
    } else if value.term.bound & (value.term.kind == 3) {
        assert(value.term.key_class != 0x7f, "literal value range");
        assert(value.term.key_class != 2, "floating-point EBV not yet supported");
        if value.term.key_class == 1 {
            result = Boolean {value: value.term.key != 0x800000000000000000000000000000000000000000000000000000000000, is_error: false};
        } else if (value.term.key_class == 5) | (value.term.key_class == 6) {
            result = Boolean {value: value.term.key != 0, is_error: false};
        } else if value.term.key_class == 7 {
            result = Boolean {value: true, is_error: false};
        }
    }
    result
}

fn compare(op: u8, a: Value, b: Value) -> Value {
    assert(op <= 4, "comparison instruction");
    let mut result = error();
    if a.is_boolean & b.is_boolean {
        let av = a.boolean as u8; let bv = b.boolean as u8;
        let value = ((op == 0) & (av == bv)) | ((op == 1) & (av < bv))
            | ((op == 2) & (av <= bv)) | ((op == 3) & (av > bv)) | ((op == 4) & (av >= bv));
        result = boolean(value, a.is_error | b.is_error);
    } else if !a.is_boolean & !b.is_boolean {
        if a.term.bound & b.term.bound {
            assert((a.term.key_class != 0x7f) & (b.term.key_class != 0x7f), "literal comparison range");
            if (a.term.kind == 3) & (b.term.kind == 3) {
                assert((a.term.key_class != 2) & (b.term.key_class != 2), "floating-point comparison not yet supported");
                let temporal_a = (a.term.key_class == 3) | (a.term.key_class == 4);
                let temporal_b = (b.term.key_class == 3) | (b.term.key_class == 4);
                assert(!(temporal_a & temporal_b) | (a.term.dt == b.term.dt), "mixed temporal datatypes not yet supported");
                assert(!(temporal_a & temporal_b) | (a.term.key_class == b.term.key_class), "mixed timezone comparison not yet supported");
                assert((op != 0) | ((a.term.key_class != 0) & (b.term.key_class != 0)), "literal equality without a supported value key");
                let strings = ((a.term.key_class == 6) | (a.term.key_class == 7))
                    & ((b.term.key_class == 6) | (b.term.key_class == 7));
                assert((op == 0) | !strings | ((a.term.lang == 0) & (b.term.lang == 0)), "language-tagged string ordering not yet supported");
                assert(((op == 0) & strings) | ((a.term.key_class != 7) & (b.term.key_class != 7)), "string comparison capacity");
            }
            let class = a.term.key_class;
            let keyed = (a.term.kind == 3) & (b.term.kind == 3) & (class == b.term.key_class)
                & ((class == 1) | (class == 3) | (class == 4) | (class == 5) | (class == 6));
            let strings = (a.term.kind == 3) & (b.term.kind == 3)
                & ((a.term.key_class == 6) | (a.term.key_class == 7))
                & ((b.term.key_class == 6) | (b.term.key_class == 7));
            if (op == 0) & strings {
                result = boolean(same_term(a.term, b.term), false);
            } else if keyed {
                let av = a.term.key; let bv = b.term.key;
                let less = av.lt(bv); let greater = bv.lt(av);
                let value = ((op == 0) & (av == bv)) | ((op == 1) & less)
                    | ((op == 2) & !greater) | ((op == 3) & greater) | ((op == 4) & !less);
                result = boolean(value, false);
            } else if op == 0 {
                if (a.term.kind != 3) | (b.term.kind != 3) {
                    result = boolean(same_term(a.term, b.term), false);
                } else if same_term(a.term, b.term) {
                    result = boolean(true, false);
                }
            }
        }
    } else {
        assert(false, "comparison of boolean expression and term not yet supported");
    }
    result
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
                let mut term = unbound();
                for v in 0..V { if step.parameter == v { term = input.rows[r][v]; } }
                values[s] = Value {term, boolean: false, is_boolean: false, is_error: false};
            } else if step.op == 1 {
                assert(step.parameter < P, "expression parameter");
                values[s] = Value {term: parameters[step.parameter], boolean: false, is_boolean: false, is_error: false};
            } else if step.op == 7 {
                values[s] = boolean(false, false);
            } else {
                assert((step.a < s) & (step.b < s), "expression topology");
                let a = values[step.a]; let b = values[step.b];
                if step.op == 2 {
                    values[s] = compare(step.comparison, a, b);
                } else {
                    let av = ebv(a); let bv = ebv(b);
                    if step.op == 3 {
                        values[s] = boolean(av.value & bv.value, (av.is_error | bv.is_error)
                            & !((!av.value & !av.is_error) | (!bv.value & !bv.is_error)));
                    } else if step.op == 4 {
                        values[s] = boolean(av.value | bv.value, (av.is_error | bv.is_error)
                            & !((av.value & !av.is_error) | (bv.value & !bv.is_error)));
                    } else {
                        assert(step.op == 5, "expression opcode");
                        values[s] = boolean(!av.value, av.is_error);
                    }
                }
            }
        }
        results[r] = ebv(values[E - 1]);
    }
    results
}

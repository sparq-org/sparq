// Research grade; not externally audited.
use crate::expr_arith::{promoted_type, to_double, to_float};
use crate::expr_core::{
    BOOLEAN, check_numeric, check_string, DATE, DATETIME, DECIMAL, FLOAT, Lane, lane, logical_not,
    logical_or, NUMERIC, Numeric, Scalar, STRING, StringLane, truth,
};
use crate::expr_term_contract::TermCell;
use sparq_ieee754::{f32, f64};

pub global EQ: u8 = 0;
pub global LT: u8 = 1;
pub global LE: u8 = 2;
pub global GT: u8 = 3;
pub global GE: u8 = 4;

fn relation(eq: bool, lt: bool, gt: bool, op: u8) -> bool {
    assert(op <= GE, "comparison name");
    if op == EQ {
        eq
    } else if op == LT {
        lt
    } else if op == LE {
        lt | eq
    } else if op == GT {
        gt
    } else {
        gt | eq
    }
}
pub fn numeric_compare(
    a: (Lane<Numeric>, bool),
    b: (Lane<Numeric>, bool),
    op: u8,
) -> (Lane<bool>, bool) {
    let (a, ae) = a;
    let (b, be) = b;
    let error = ae | be;
    // Error payloads are never evaluated as a number.
    let x = if error {
        crate::expr_core::zero_numeric()
    } else {
        a.value
    };
    let y = if error {
        crate::expr_core::zero_numeric()
    } else {
        b.value
    };
    check_numeric(x);
    check_numeric(y);
    let ty = promoted_type(x.ty, y.ty, false);
    let mut value = false;
    if ty <= DECIMAL {
        let eq = (x.magnitude == y.magnitude) & (x.negative == y.negative);
        let lt = if x.negative != y.negative {
            x.negative
        } else if x.negative {
            y.magnitude.lt(x.magnitude)
        } else {
            x.magnitude.lt(y.magnitude)
        };
        value = relation(eq, lt, !eq & !lt, op);
    } else if ty == FLOAT {
        let x = f32::new(to_float(x));
        let y = f32::new(to_float(y));
        value = relation(x.eq(y), x.lt(y), x.gt(y), op);
    } else {
        let x = f64::new(to_double(x));
        let y = f64::new(to_double(y));
        value = relation(x.eq(y), x.lt(y), x.gt(y), op);
    }
    (Lane { value: value & !error, capacity: a.capacity | b.capacity }, error)
}
pub fn same_term(a: (Lane<TermCell>, bool), b: (Lane<TermCell>, bool)) -> (Lane<bool>, bool) {
    let (a, ae) = a;
    let (b, be) = b;
    let error = ae | be | !a.value.bound() | !b.value.bound();
    (Lane { value: identity(a.value, b.value) & !error, capacity: a.capacity | b.capacity }, error)
}

// Explicit byte lengths preserve U+0000. The upstream str<N> comparison infers
// length from NUL padding, so it cannot express this calling convention.
pub fn string_compare<let L: u32>(a: StringLane<L>, b: StringLane<L>, op: u8) -> bool {
    check_string(a);
    check_string(b);
    let mut equal = true;
    let mut less = false;
    for i in 0..L {
        if equal & (i < a.len) & (i < b.len) {
            less = a.bytes[i] < b.bytes[i];
            equal = a.bytes[i] == b.bytes[i];
        }
    }
    if equal {
        less = a.len < b.len;
        equal = a.len == b.len;
    }
    relation(equal, less, !equal & !less, op)
}

// Scalars reach this function only through CheckedOpening or computed typed
// lanes. Key comparisons are valid within a class, never across class tags.
pub fn value_compare<let L: u32>(
    a: (Lane<Scalar<L>>, bool),
    b: (Lane<Scalar<L>>, bool),
    op: u8,
) -> (Lane<bool>, bool) {
    assert(op <= GE, "comparison name");
    let (a, ae) = a;
    let (b, be) = b;
    let x = a.value;
    let y = b.value;
    let mut error = ae | be | !x.term.bound() | !y.term.bound();
    let mut capacity = a.capacity | b.capacity;
    let mut value = false;
    if (x.tag == NUMERIC) & (y.tag == NUMERIC) {
        error = error | !x.valid | !y.valid;
        let (v, e) = numeric_compare((lane(x.number), error), (lane(y.number), error), op);
        value = v.value;
        error = e;
    } else if (x.tag == BOOLEAN) & (y.tag == BOOLEAN) {
        error = error | !x.valid | !y.valid;
        value = relation(
            x.boolean == y.boolean,
            !x.boolean & y.boolean,
            x.boolean & !y.boolean,
            op,
        );
    } else if (x.tag == STRING) & (y.tag == STRING) {
        error = error | !x.valid | !y.valid;
        if op == EQ {
            value = (x.string.lang == y.string.lang) & string_compare(x.string, y.string, EQ);
        } else {
            error = error | (x.string.lang != 0) | (y.string.lang != 0);
            value = string_compare(x.string, y.string, op);
        }
    } else if ((x.tag == DATE) | (x.tag == DATETIME)) & (x.tag == y.tag) {
        error = error | !x.valid | !y.valid;
        // A missing timezone would require an agreed implicit timezone. Different
        // classes have no order here; this is a method capacity refusal.
        capacity = capacity | (!error & (x.temporal.key_class != y.temporal.key_class));
        x.temporal.key.assert_max_bit_size::<64>();
        y.temporal.key.assert_max_bit_size::<64>();
        value = relation(
            x.temporal.key == y.temporal.key,
            x.temporal.key.lt(y.temporal.key),
            y.temporal.key.lt(x.temporal.key),
            op,
        );
    } else if op == EQ {
        let identical = identity(x.term, y.term);
        // Specific datatype mappings above take precedence, including NaN and
        // invalid lexical forms. Only the RDF-term fallback uses identity.
        value = identical;
        error = error | ((x.term.kind() == 3) & (y.term.kind() == 3) & !identical);
    } else {
        error = true;
    }
    (Lane { value: value & !error & !capacity, capacity }, error)
}
pub fn in_list<let L: u32, let N: u32>(
    lhs: (Lane<Scalar<L>>, bool),
    args: [(Lane<Scalar<L>>, bool); N],
) -> (Lane<bool>, bool) {
    let mut result = truth(false, false);
    for i in 0..N {
        result = logical_or(result, value_compare(lhs, args[i], EQ));
    }
    result
}
pub fn not_in_list<let L: u32, let N: u32>(
    lhs: (Lane<Scalar<L>>, bool),
    args: [(Lane<Scalar<L>>, bool); N],
) -> (Lane<bool>, bool) {
    logical_not(in_list(lhs, args))
}

fn identity(a: TermCell, b: TermCell) -> bool {
    (a.bound() == b.bound())
        & (a.kind() == b.kind())
        & (a.enc() == b.enc())
        & (a.credential() == b.credential())
}

// Research grade; not externally audited.
use crate::expr_term_contract::{CheckedOpening, TermCell};

pub global SCALE: Field = 1000000000000000000;
pub global EXACT_LIMIT: Field = 5192296858534827628530496329220096; // 2^112
pub global INTEGER: u8 = 0;
pub global DECIMAL: u8 = 1;
pub global FLOAT: u8 = 2;
pub global DOUBLE: u8 = 3;
pub global TERM: u8 = 0;
pub global BOOLEAN: u8 = 1;
pub global NUMERIC: u8 = 2;
pub global STRING: u8 = 3;
pub global DATE: u8 = 4;
pub global DATETIME: u8 = 5;

// Capacity is distinct from a SPARQL error: COALESCE must not turn a capacity
// refusal into a fallback value. Callers constrain !capacity at the selected root.
pub struct Lane<T> {
    pub value: T,
    pub capacity: bool,
}

// Integer/decimal magnitude is abs(value * 10^18), a bounded subset of class 01.
// Float/double bits retain signed zero and NaN. Inactive fields are canonical zero.
pub struct Numeric {
    pub ty: u8,
    pub negative: bool,
    pub magnitude: Field,
    pub bits: u64,
}

pub struct StringLane<let L: u32> {
    pub bytes: [u8; L],
    pub len: u32,
    pub lang: Field,
}

pub struct TemporalLane {
    pub key_class: u8,
    pub key: Field,
}

// Run-time union for a query whose datatype is not fixed statically. `valid` is
// derived by CheckedOpening, including lexical validation and derived-type facets.
pub struct Scalar<let L: u32> {
    pub tag: u8,
    pub valid: bool,
    pub boolean: bool,
    pub number: Numeric,
    pub string: StringLane<L>,
    pub temporal: TemporalLane,
    pub term: TermCell,
}

pub fn lane<T>(value: T) -> Lane<T> {
    Lane { value, capacity: false }
}
pub fn truth(value: bool, is_error: bool) -> (Lane<bool>, bool) {
    (lane(value & !is_error), is_error)
}
pub fn zero_numeric() -> Numeric {
    Numeric { ty: INTEGER, negative: false, magnitude: 0, bits: 0 }
}
pub fn exact(ty: u8, negative: bool, magnitude: Field) -> Numeric {
    let value = Numeric { ty, negative: negative & (magnitude != 0), magnitude, bits: 0 };
    check_numeric(value);
    value
}
pub fn floating(ty: u8, bits: u64) -> Numeric {
    let value = Numeric { ty, negative: false, magnitude: 0, bits };
    check_numeric(value);
    value
}
pub fn check_numeric(value: Numeric) {
    assert(value.ty <= DOUBLE, "numeric tag");
    if value.ty <= DECIMAL {
        value.magnitude.assert_max_bit_size::<112>();
        assert(value.bits == 0, "inactive bits");
        assert(!value.negative | (value.magnitude != 0), "exact signed zero");
        if value.ty == INTEGER {
            assert((value.magnitude as u128) % (SCALE as u128) == 0, "integer scale");
        }
    } else {
        assert(!value.negative & (value.magnitude == 0), "inactive exact lane");
        if value.ty == FLOAT {
            (value.bits as Field).assert_max_bit_size::<32>();
        }
    }
}
pub fn check_string<let L: u32>(s: StringLane<L>) {
    assert(s.len <= L, "string capacity");
    for i in 0..L {
        if i >= s.len {
            assert(s.bytes[i] == 0, "string padding");
        }
    }
}
pub fn variable<T, let L: u32>(opening: T) -> (Lane<Scalar<L>>, bool)
where
    T: CheckedOpening<L>,
{
    let value = opening.open();
    let is_error = !value.term.bound();
    assert(value.tag <= DATETIME, "scalar tag");
    (lane(value), is_error)
}
pub fn as_numeric<let L: u32>(a: (Lane<Scalar<L>>, bool)) -> (Lane<Numeric>, bool) {
    let (a, e) = a;
    let error = e | !a.value.term.bound() | !a.value.valid | (a.value.tag != NUMERIC);
    let value = if error {
        zero_numeric()
    } else {
        a.value.number
    };
    check_numeric(value);
    (Lane { value, capacity: a.capacity }, error)
}
pub fn ebv<let L: u32>(a: (Lane<Scalar<L>>, bool)) -> (Lane<bool>, bool) {
    let (a, e) = a;
    let v = a.value;
    let admitted = (v.tag == BOOLEAN) | (v.tag == NUMERIC) | (v.tag == STRING);
    let error = e | !v.term.bound() | !admitted;
    let mut value = false;
    if v.tag == BOOLEAN {
        value = v.valid & v.boolean;
    }
    if v.tag == STRING {
        check_string(v.string);
        value = v.string.len != 0;
    }
    if (v.tag == NUMERIC) & v.valid {
        check_numeric(v.number);
        if v.number.ty <= DECIMAL {
            value = v.number.magnitude != 0;
        }
        if v.number.ty == FLOAT {
            let b = v.number.bits;
            value = ((b & 0x7fffffff) != 0)
                & !(((b & 0x7f800000) == 0x7f800000) & ((b & 0x007fffff) != 0));
        }
        if v.number.ty == DOUBLE {
            let b = v.number.bits;
            value = ((b & 0x7fffffffffffffff) != 0)
                & !(
                    ((b & 0x7ff0000000000000) == 0x7ff0000000000000)
                        & ((b & 0x000fffffffffffff) != 0)
                );
        }
    }
    (Lane { value: value & !error, capacity: a.capacity }, error)
}
pub fn logical_not(a: (Lane<bool>, bool)) -> (Lane<bool>, bool) {
    let (a, e) = a;
    (Lane { value: !a.value & !e, capacity: a.capacity }, e)
}
pub fn logical_and(a: (Lane<bool>, bool), b: (Lane<bool>, bool)) -> (Lane<bool>, bool) {
    let (a, ae) = a;
    let (b, be) = b;
    let is_false = (!ae & !a.value) | (!be & !b.value);
    let error = (ae | be) & !is_false;
    (
        Lane {
            value: sparq_xpath::logical_and(a.value, b.value) & !error,
            capacity: a.capacity | b.capacity,
        }, error,
    )
}
pub fn logical_or(a: (Lane<bool>, bool), b: (Lane<bool>, bool)) -> (Lane<bool>, bool) {
    let (a, ae) = a;
    let (b, be) = b;
    let is_true = (!ae & a.value) | (!be & b.value);
    let error = (ae | be) & !is_true;
    (Lane { value: is_true, capacity: a.capacity | b.capacity }, error)
}
pub fn choose<T>(
    condition: (Lane<bool>, bool),
    yes: (Lane<T>, bool),
    no: (Lane<T>, bool),
) -> (Lane<T>, bool) {
    let (c, ce) = condition;
    let (selected, se) = if c.value { yes } else { no };
    (Lane { value: selected.value, capacity: c.capacity | (!ce & selected.capacity) }, ce | se)
}
pub fn coalesce<T, let N: u32>(args: [(Lane<T>, bool); N], error_value: T) -> (Lane<T>, bool) {
    let mut found = false;
    let mut selected = lane(error_value);
    for i in 0..N {
        let (arg, error) = args[i];
        if !found & (!error | arg.capacity) {
            selected = arg;
            found = true;
        }
    }
    (selected, !found)
}
pub fn bound(cell: TermCell) -> (Lane<bool>, bool) {
    truth(cell.bound(), false)
}
pub fn filter_accept(a: (Lane<bool>, bool)) -> bool {
    let (a, error) = a;
    assert(!a.capacity, "expression capacity");
    a.value & !error
}

pub fn as_string<let L: u32>(input: (Lane<Scalar<L>>, bool)) -> (Lane<StringLane<L>>, bool) {
    let (a, e) = input;
    let error = e | !a.value.term.bound() | !a.value.valid | (a.value.tag != STRING);
    let value = if error {
        StringLane { bytes: [0; L], len: 0, lang: 0 }
    } else {
        a.value.string
    };
    check_string(value);
    (Lane { value, capacity: a.capacity }, error)
}

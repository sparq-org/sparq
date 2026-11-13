// Research grade; not externally audited.
// Narrow adapter for the target-query value bucket. The static opening trait
// must call lane A's open_lexical and recompute_key; a bare cell is not accepted.
use crate::expr_core::{
    BOOLEAN, DATE, DATETIME, DECIMAL, exact, INTEGER, Lane, NUMERIC, Scalar, STRING, StringLane,
    TemporalLane, TERM, zero_numeric,
};
use crate::expr_term_contract::{
    CheckedLexicalOpening, CLASS_BOOLEAN, CLASS_DATE, CLASS_DATE_TIME, CLASS_DECIMAL, CLASS_DOUBLE,
    CLASS_LOCAL_DATE_TIME, CLASS_NONE, CLASS_STRING, CLASS_STRING_PREFIX, CLASS_UNREPRESENTABLE,
    DT_BOOLEAN, DT_BYTE, DT_DATE, DT_DATETIME, DT_DECIMAL, DT_DOUBLE, DT_FLOAT, DT_INT, DT_INTEGER,
    DT_LANGSTRING, DT_LONG, DT_NEGATIVEINTEGER, DT_NONNEGATIVEINTEGER, DT_NONPOSITIVEINTEGER,
    DT_POSITIVEINTEGER, DT_SHORT, DT_STRING, DT_UNSIGNEDBYTE, DT_UNSIGNEDINT, DT_UNSIGNEDLONG,
    DT_UNSIGNEDSHORT, key_payload, KIND_LITERAL, read,
};

pub fn variable<T, let L: u32, let D: u32, let G: u32>(opening: T) -> (Lane<Scalar<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let o = read(opening);
    let cell = o.cell;
    let mut s = Scalar {
        tag: TERM,
        valid: true,
        boolean: false,
        number: zero_numeric(),
        string: StringLane { bytes: [0; L], len: 0, lang: 0 },
        temporal: TemporalLane { key_class: 0, key: 0 },
        term: cell,
    };
    let mut capacity = false;
    if cell.bound() & (cell.kind() == KIND_LITERAL) {
        let dt = cell.datatype();
        let integer = (dt == DT_INTEGER)
            | (dt == DT_NONPOSITIVEINTEGER)
            | (dt == DT_NEGATIVEINTEGER)
            | (dt == DT_LONG)
            | (dt == DT_INT)
            | (dt == DT_SHORT)
            | (dt == DT_BYTE)
            | (dt == DT_NONNEGATIVEINTEGER)
            | (dt == DT_UNSIGNEDLONG)
            | (dt == DT_UNSIGNEDINT)
            | (dt == DT_UNSIGNEDSHORT)
            | (dt == DT_UNSIGNEDBYTE)
            | (dt == DT_POSITIVEINTEGER);
        let class = cell.key_class();
        let payload = key_payload(cell);
        if integer | (dt == DT_DECIMAL) {
            s.tag = NUMERIC;
            s.valid = class != CLASS_NONE;
            capacity = class == CLASS_UNREPRESENTABLE;
            if class == CLASS_DECIMAL {
                let bias: Field = 0x800000000000000000000000000000000000000000000000000000000000;
                let negative = payload.lt(bias);
                let magnitude = if negative {
                    bias - payload
                } else {
                    payload - bias
                };
                capacity = !magnitude.lt(crate::expr_core::EXACT_LIMIT);
                if !capacity {
                    s.number = exact(if integer { INTEGER } else { DECIMAL }, negative, magnitude);
                }
            } else {
                assert(
                    (class == CLASS_NONE) | (class == CLASS_UNREPRESENTABLE),
                    "numeric key class",
                );
            }
        } else if dt == DT_BOOLEAN {
            s.tag = BOOLEAN;
            s.valid = class == CLASS_BOOLEAN;
            assert((class == CLASS_BOOLEAN) | (class == CLASS_NONE), "boolean key class");
            let true_key: Field = 0x10000000000000000000000000000000000000000000000000000000000;
            assert(
                (payload == 0) | ((class == CLASS_BOOLEAN) & (payload == true_key)),
                "boolean key payload",
            );
            s.boolean = payload != 0;
        } else if (dt == DT_STRING) | (dt == DT_LANGSTRING) {
            assert(
                if dt == DT_LANGSTRING {
                    class == CLASS_NONE
                } else {
                    (class == CLASS_STRING) | (class == CLASS_STRING_PREFIX)
                },
                "string key class",
            );
            let mut bytes = [0; L];
            for i in 0..L {
                bytes[i] = o.lexical.get_unchecked(i);
            }
            s.tag = STRING;
            s.string = StringLane {
                bytes,
                len: o.lexical.len(),
                lang: if dt == DT_LANGSTRING {
                    cell.language()
                } else {
                    0
                },
            };
        } else if (dt == DT_DATE) | (dt == DT_DATETIME) {
            s.tag = if dt == DT_DATE { DATE } else { DATETIME };
            s.valid = class != CLASS_NONE;
            capacity = class == CLASS_UNREPRESENTABLE;
            if !capacity & s.valid {
                assert(
                    if dt == DT_DATE {
                        class == CLASS_DATE
                    } else {
                        (class == CLASS_DATE_TIME) | (class == CLASS_LOCAL_DATE_TIME)
                    },
                    "temporal key class",
                );
                let bytes = payload.to_be_bytes::<30>();
                let mut key: Field = 0;
                for i in 0..30 {
                    if i < 8 {
                        key = key * 256 + bytes[i] as Field;
                    } else {
                        assert(bytes[i] == 0, "temporal key padding");
                    }
                }
                s.temporal = TemporalLane { key_class: class, key };
            }
        } else if (dt == DT_FLOAT) | (dt == DT_DOUBLE) {
            s.tag = NUMERIC;
            s.valid = class != CLASS_NONE;
            assert(
                (class == CLASS_NONE) | (class == CLASS_DOUBLE) | (class == CLASS_UNREPRESENTABLE),
                "IEEE key class",
            );
            let ty = if dt == DT_FLOAT {
                crate::expr_core::FLOAT
            } else {
                crate::expr_core::DOUBLE
            };
            let mut bits: u64 = 0;
            if class == CLASS_UNREPRESENTABLE {
                bits = if ty == crate::expr_core::FLOAT {
                    0x7fc00000
                } else {
                    0x7ff8000000000000
                };
            } else if class == CLASS_DOUBLE {
                let order = word(payload);
                let mut wide = if order >> 63 == 1 {
                    order ^ 0x8000000000000000
                } else {
                    !order
                };
                assert(
                    ((wide & 0x7ff0000000000000) != 0x7ff0000000000000)
                        | ((wide & 0xfffffffffffff) == 0),
                    "NaN key class",
                );
                if wide == 0 {
                    let mut negative = false;
                    for i in 0..L {
                        if (i == 0) & (i < o.lexical.len()) {
                            negative = o.lexical.get_unchecked(i) == 45;
                        }
                    }
                    if negative {
                        wide = 0x8000000000000000;
                    }
                }
                bits = if ty == crate::expr_core::FLOAT {
                    single_bits(wide) as u64
                } else {
                    wide
                };
            }
            s.number = crate::expr_core::floating(ty, bits);
        }
    }
    // The current suite lexical key parsers do not collapse XML whitespace.
    // Refuse that key-based representation instead of reporting a SPARQL error
    // for a potentially valid value. Character/metadata functions use the opening.
    if cell.bound()
        & (cell.kind() == KIND_LITERAL)
        & (cell.key_class() == CLASS_NONE)
        & ((s.tag == NUMERIC) | (s.tag == BOOLEAN) | (s.tag == DATE) | (s.tag == DATETIME)) {
        let mut edge = false;
        for i in 0..L {
            if (i < o.lexical.len()) & ((i == 0) | (i + 1 == o.lexical.len())) {
                let b = o.lexical.get_unchecked(i);
                edge = edge | (b == 32) | (b == 9) | (b == 10) | (b == 13);
            }
        }
        if edge {
            s.valid = true;
            capacity = true;
        }
    }
    (Lane { value: s, capacity }, !cell.bound())
}

fn word(payload: Field) -> u64 {
    let b = payload.to_be_bytes::<30>();
    let mut out: u64 = 0;
    for i in 0..30 {
        if i < 8 {
            out = out * 256 + b[i] as u64;
        } else {
            assert(b[i] == 0, "IEEE key padding");
        }
    }
    out
}
// Inverse of the suite's exact binary32-to-binary64 key widening. The pinned
// IEEE library has no cross-format conversion API. This is bit decoding only.
fn single_bits(wide: u64) -> u32 {
    let sign = (wide >> 63) as u32 * 0x80000000;
    let e = (wide >> 52) & 2047;
    let m = wide & 0xfffffffffffff;
    let mut out: u32 = 0;
    if (e == 0) & (m == 0) {
        out = sign;
    } else if e == 2047 {
        assert_eq(m, 0, "float key mantissa");
        out = sign | 0x7f800000;
    } else if e >= 897 {
        assert(e <= 1150, "float key exponent");
        assert((m & 0x1fffffff) == 0, "float key precision");
        out = sign | ((e - 896) as u32 << 23) | (m >> 29) as u32;
    } else {
        assert(e >= 874, "float key exponent");
        let shift = 926 - e;
        let sig = 0x10000000000000 | m;
        let fraction = (sig >> shift) as u32;
        assert_eq((fraction as u64) << shift, sig, "float key precision");
        out = sign | fraction;
    }
    assert_eq(crate::expr_arith::widen_float(out as u64), wide, "float key widening");
    out
}

// Computed term edges retain their parent's channels; raw variable slots use
// variable() directly. The opening's implementation remains statically checked.
pub fn value<T, let L: u32, let D: u32, let G: u32>(
    input: (Lane<T>, bool),
) -> (Lane<Scalar<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let (input, e) = input;
    let (mut out, ve) = variable(input.value);
    out.capacity = out.capacity | input.capacity;
    (out, e | ve)
}

// Research grade; not externally audited.
use crate::expr_core::{
    check_numeric, DECIMAL, DOUBLE, EXACT_LIMIT, FLOAT, floating, INTEGER, Lane, Numeric, SCALE,
    zero_numeric,
};
use sparq_ieee754::{f32, f64};

pub global ADD: u8 = 0;
pub global SUB: u8 = 1;
pub global MUL: u8 = 2;
pub global DIV: u8 = 3;

pub fn promoted_type(a: u8, b: u8, divide: bool) -> u8 {
    assert((a <= DOUBLE) & (b <= DOUBLE), "numeric tag");
    let ty = if a > b { a } else { b };
    if divide & (ty == INTEGER) {
        DECIMAL
    } else {
        ty
    }
}

// Long division is fully constrained. n < 2^224, d < 2^112, so the
// remainder/doubling fits u128 and quotient accumulation never wraps Field.
// Its fixed bit loop dominates exact multiplication/division cost.
fn divmod(n: Field, d: Field) -> (Field, Field) {
    n.assert_max_bit_size::<224>();
    d.assert_max_bit_size::<112>();
    assert(d != 0, "internal divisor");
    let bits = n.to_le_bits::<224>();
    let divisor = d as u128;
    let mut remainder: u128 = 0;
    let mut quotient: Field = 0;
    for i in 0..224 {
        remainder = remainder * 2 + bits[223 - i] as u128;
        let digit = remainder >= divisor;
        remainder = remainder - (digit as u128) * divisor;
        quotient = quotient * 2 + digit as Field;
    }
    (quotient, remainder as Field)
}

// One round-to-nearest-even conversion of the EXACT rational magnitude/10^18.
// Converting numerator and denominator separately through binary64 would round
// twice. Values in this bucket are normal in binary32 and binary64.
pub fn exact_to_bits<let M: u32, let B: u32>(a: Numeric) -> u64 {
    check_numeric(a);
    assert(a.ty <= DECIMAL, "exact promotion");
    let mut n = if a.magnitude == 0 {
        1
    } else {
        a.magnitude as u128
    };
    let mut d: u128 = 1000000000000000000;
    let mut exponent = B as u64;
    for _ in 0..60 {
        if n < d {
            n = n * 2;
            exponent = exponent - 1;
        }
    }
    for _ in 0..53 {
        if n >= d * 2 {
            d = d * 2;
            exponent = exponent + 1;
        }
    }
    let mut remainder = n - d;
    let mut significand: u64 = 1;
    for _ in 0..M {
        remainder = remainder * 2;
        let digit = remainder >= d;
        remainder = remainder - (digit as u128) * d;
        significand = significand * 2 + digit as u64;
    }
    let round_up = (remainder * 2 > d) | ((remainder * 2 == d) & ((significand & 1) != 0));
    significand = significand + round_up as u64;
    let implicit: u64 = (1 as u64) << (M as u64);
    if significand == implicit * 2 {
        significand = implicit;
        exponent = exponent + 1;
    }
    let sign_bit = if M == 23 {
        0x80000000 as u64
    } else {
        0x8000000000000000 as u64
    };
    if a.magnitude == 0 {
        0
    } else {
        (a.negative as u64) * sign_bit + exponent * implicit + significand - implicit
    }
}

// binary32 -> binary64 is exact, including subnormals. The external library has
// no cross-format conversion entry point at the pinned tag.
pub fn widen_float(bits: u64) -> u64 {
    (bits as Field).assert_max_bit_size::<32>();
    let sign = (bits >> 31) * 0x8000000000000000;
    let exp = (bits >> 23) & 255;
    let mant = bits & 0x7fffff;
    let mut m = mant;
    let mut e: u64 = 897;
    for _ in 0..23 {
        if (m != 0) & (m < 0x800000) {
            m = m * 2;
            e = e - 1;
        }
    }
    if exp == 255 {
        sign + 0x7ff0000000000000 + (if mant == 0 { 0 } else { 0x8000000000000 })
    } else if exp == 0 {
        if mant == 0 {
            sign
        } else {
            sign + e * 0x10000000000000 + (m - 0x800000) * 0x20000000
        }
    } else {
        sign + (exp + 896) * 0x10000000000000 + mant * 0x20000000
    }
}
pub fn to_float(a: Numeric) -> u32 {
    check_numeric(a);
    assert(a.ty <= FLOAT, "float promotion");
    if a.ty <= DECIMAL {
        exact_to_bits::<23, 127>(a) as u32
    } else {
        a.bits as u32
    }
}
pub fn to_double(a: Numeric) -> u64 {
    check_numeric(a);
    if a.ty <= DECIMAL {
        exact_to_bits::<52, 1023>(a)
    } else if a.ty == FLOAT {
        widen_float(a.bits)
    } else {
        a.bits
    }
}
pub fn unary(a: (Lane<Numeric>, bool), minus: bool) -> (Lane<Numeric>, bool) {
    let (a, error) = a;
    let mut value = if error { zero_numeric() } else { a.value };
    check_numeric(value);
    if minus {
        if value.ty <= DECIMAL {
            value.negative = !value.negative & (value.magnitude != 0);
        } else {
            value.bits = value.bits
                ^ (
                    if value.ty == FLOAT {
                        0x80000000
                    } else {
                        0x8000000000000000
                    }
                );
        }
    }
    (Lane { value, capacity: a.capacity }, error)
}
pub fn binary(a: (Lane<Numeric>, bool), b: (Lane<Numeric>, bool), op: u8) -> (Lane<Numeric>, bool) {
    assert(op <= DIV, "arithmetic name");
    let (a, ae) = a;
    let (b, be) = b;
    let mut error = ae | be;
    let x = if error { zero_numeric() } else { a.value };
    let y = if error { zero_numeric() } else { b.value };
    check_numeric(x);
    check_numeric(y);
    let ty = promoted_type(x.ty, y.ty, op == DIV);
    let mut value = zero_numeric();
    let mut capacity = a.capacity | b.capacity;
    if ty <= DECIMAL {
        let yn = y.negative ^ (op == SUB);
        let x_large = !(x.magnitude.lt(y.magnitude));
        let mut magnitude = if x.negative == yn {
            x.magnitude + y.magnitude
        } else if x_large {
            x.magnitude - y.magnitude
        } else {
            y.magnitude - x.magnitude
        };
        let mut negative = if x_large { x.negative } else { yn };
        if (op == MUL) | (op == DIV) {
            let n = if op == MUL {
                x.magnitude * y.magnitude
            } else {
                x.magnitude * SCALE
            };
            let d = if op == MUL {
                SCALE
            } else {
                if y.magnitude == 0 {
                    1
                } else {
                    y.magnitude
                }
            };
            let (q, r) = divmod(n, d);
            magnitude = q;
            negative = x.negative ^ y.negative;
            // This method admits exact results only; excess fractional precision
            // (including a repeating quotient) is a capacity refusal.
            capacity = capacity | (r != 0);
            error = error | ((op == DIV) & (y.magnitude == 0));
        }
        capacity = capacity | !magnitude.lt(EXACT_LIMIT);
        value = Numeric { ty, negative: negative & (magnitude != 0), magnitude, bits: 0 };
    } else if ty == FLOAT {
        let x = f32::new(to_float(x));
        let y = f32::new(to_float(y));
        let z = if op == ADD {
            x + y
        } else if op == SUB {
            x - y
        } else if op == MUL {
            x * y
        } else {
            x / y
        };
        value = floating(FLOAT, z.bits() as u64);
    } else {
        let x = f64::new(to_double(x));
        let y = f64::new(to_double(y));
        let z = if op == ADD {
            x + y
        } else if op == SUB {
            x - y
        } else if op == MUL {
            x * y
        } else {
            x / y
        };
        value = floating(DOUBLE, z.bits());
    }
    // Canonical payload on error/refusal; capacity must survive to the root.
    if error | capacity {
        value = zero_numeric();
    }
    (Lane { value, capacity: a.capacity | b.capacity | (capacity & !error) }, error)
}

// Unary plus is the original RDF term, including its derived datatype and lexical
// spelling. Numeric validation happens before identity is retained for sameTerm.
pub fn unary_plus_term<let L: u32>(
    a: (Lane<crate::expr_core::Scalar<L>>, bool),
) -> (Lane<crate::expr_core::Scalar<L>>, bool) {
    let (_, error) = crate::expr_core::as_numeric(a);
    (a.0, error)
}

// Numeric functions over constrained input lanes.
// Research grade; not externally audited.
use crate::expr_core::{
    check_numeric, DECIMAL, EXACT_LIMIT, FLOAT, floating, INTEGER, Lane, Numeric, SCALE,
    zero_numeric,
};
use sparq_ieee754::{f32, f64};
pub global ABS: u8 = 0;
pub global ROUND: u8 = 1;
pub global CEIL: u8 = 2;
pub global FLOOR: u8 = 3;
pub fn numeric(a: (Lane<Numeric>, bool), function: u8) -> (Lane<Numeric>, bool) {
    assert(function <= FLOOR, "numeric function");
    let (a, error) = a;
    let mut n = if error { zero_numeric() } else { a.value };
    check_numeric(n);
    let mut capacity = a.capacity;
    if n.ty == INTEGER {
        // This bucket's unscaled integer is below 2^53, safely inside i64.
        let m = (n.magnitude as u128 / (SCALE as u128)) as i64;
        let value = if n.negative { -m } else { m };
        let out = if function == ABS {
            sparq_xpath::abs_int(value)
        } else if function == ROUND {
            sparq_xpath::round_int(value)
        } else if function == CEIL {
            sparq_xpath::ceil_int(value)
        } else {
            sparq_xpath::floor_int(value)
        };
        n.negative = out < 0;
        n.magnitude = (if out < 0 { -out } else { out }) as u64 as Field * SCALE;
    } else if n.ty == DECIMAL {
        if function == ABS {
            n.negative = false;
        } else if n.ty == DECIMAL {
            let mag = n.magnitude as u128;
            let scale = SCALE as u128;
            let q = mag / scale;
            let r = mag % scale;
            let increment = if function == ROUND {
                (r * 2 > scale) | ((r * 2 == scale) & !n.negative)
            } else if function == CEIL {
                (!n.negative) & (r != 0)
            } else {
                n.negative & (r != 0)
            };
            n.magnitude = (q + increment as u128) as Field * SCALE;
            capacity = capacity | !n.magnitude.lt(EXACT_LIMIT);
            n.negative = n.negative & (n.magnitude != 0);
        }
    } else if n.ty == FLOAT {
        let value = f32::new(n.bits as u32);
        let bits = if function == ABS {
            value.abs().bits()
        } else if function == CEIL {
            value.ceil().bits()
        } else if function == FLOOR {
            value.floor().bits()
        } else {
            sparq_xpath::round_float(sparq_xpath::XsdFloat::from_bits(n.bits as u32)).to_bits()
        };
        n = floating(FLOAT, bits as u64);
    } else {
        let value = f64::new(n.bits);
        let bits = if function == ABS {
            value.abs().bits()
        } else if function == CEIL {
            value.ceil().bits()
        } else if function == FLOOR {
            value.floor().bits()
        } else {
            sparq_xpath::round_double(sparq_xpath::XsdDouble::from_bits(n.bits)).to_bits()
        };
        n = floating(3, bits);
    }
    if error | capacity {
        n = zero_numeric();
    }
    (Lane { value: n, capacity }, error)
}

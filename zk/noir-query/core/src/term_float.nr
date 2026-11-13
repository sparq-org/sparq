//! Exact decimal-to-IEEE rounding for bounded lexical bytes. Integer numerator
//! and denominator arithmetic avoids intermediate floating-point rounding.
//! This is deliberately a direct constrained relation, with no parse hints.
use std::collections::bounded_vec::BoundedVec;
use crate::term_key::{at, digit, equals};

fn mul10<let W: u32>(a: [u32; W], digit: u32) -> [u32; W] {
    let mut out = [0; W];
    let mut carry = digit as u64;
    for i in 0..W {
        let n = a[i] as u64 * 10 + carry;
        out[i] = n as u32;
        carry = n >> 32;
    }
    assert(carry == 0, "float rational capacity");
    out
}
fn ge<let W: u32>(a: [u32; W], b: [u32; W]) -> bool {
    let mut equal = true;
    let mut greater = false;
    for i in 0..W {
        let j = W - 1 - i;
        greater = greater | (equal & (a[j] > b[j]));
        equal = equal & (a[j] == b[j]);
    }
    equal | greater
}
fn subtract<let W: u32>(a: [u32; W], b: [u32; W]) -> [u32; W] {
    let mut out = [0; W];
    let mut borrow: u64 = 0;
    for i in 0..W {
        let n = 0x100000000 + a[i] as u64 - b[i] as u64 - borrow;
        out[i] = n as u32;
        borrow = 1 - (n >> 32);
    }
    out
}
fn shift<let W: u32>(a: [u32; W], bits: u32) -> [u32; W] {
    let mut out = [0; W];
    let words = bits / 32;
    let rem = bits % 32;
    for i in 0..W {
        let low = if i >= words { (a[i - words] as u64 << rem as u64) as u32 } else { 0 };
        let high = if i > words { (a[i - words - 1] as u64 >> (32 - rem) as u64) as u32 } else { 0 };
        out[i] = low | high;
    }
    out
}
fn log2<let W: u32>(a: [u32; W]) -> i32 {
    let mut result: u32 = 0;
    for i in 0..W {
        let bits = (a[i] as Field).to_le_bits::<32>();
        for b in 0..32 { if bits[b] { result = i * 32 + b; } }
    }
    result as i32
}
fn rounded_bits<let W: u32>(numerator: [u32; W], denominator: [u32; W], single: bool) -> u64 {
    let mut exponent = log2(numerator) - log2(denominator);
    let below = if exponent >= 0 { !ge(numerator, shift(denominator, exponent as u32)) }
        else { !ge(shift(numerator, (-exponent) as u32), denominator) };
    if below { exponent = exponent - 1; }
    let mantissa: i32 = if single { 23 } else { 52 };
    let bias: i32 = if single { 127 } else { 1023 };
    let min_exp: i32 = if single { -126 } else { -1022 };
    let max_exp: i32 = if single { 127 } else { 1023 };
    let e = if exponent < min_exp { min_exp } else if exponent > max_exp { max_exp } else { exponent };
    let scale = mantissa - e;
    let mut r = if scale >= 0 { shift(numerator, scale as u32) } else { numerator };
    let d = if scale < 0 { shift(denominator, (-scale) as u32) } else { denominator };
    let mut quotient: u64 = 0;
    // A normalized binary significand has at most mantissa+1 bits. One extra
    // bit handles the carry at an exact rounding boundary.
    for j in 0..54 {
        let bit = 53 - j;
        let trial = shift(d, bit);
        if ge(r, trial) { r = subtract(r, trial); quotient = quotient + (1_u64 << bit as u64); }
    }
    let twice = shift(r, 1);
    let above_half = ge(twice, d) & (twice != d);
    let tie = twice == d;
    if above_half | (tie & ((quotient & 1) == 1)) { quotient = quotient + 1; }
    let implicit: u64 = 1_u64 << mantissa as u64;
    let mut final_exp = e;
    if quotient >= implicit * 2 { quotient = quotient >> 1; final_exp = final_exp + 1; }
    let infinity: u64 = if single { 0x7f800000 } else { 0x7ff0000000000000 };
    if (exponent > max_exp) | (final_exp > max_exp) { infinity }
    else if quotient < implicit { quotient }
    else { ((final_exp + bias) as u64 << mantissa as u64) | (quotient - implicit) }
}
fn widen(bits: u32) -> u64 {
    let sign = (bits as u64 & 0x80000000) << 32;
    let exponent = (bits >> 23) & 255;
    let mantissa = bits & 0x7fffff;
    let mut high: u32 = 0;
    for i in 0..23 { if (mantissa >> i) != 0 { high = i; } }
    if exponent == 255 { sign | 0x7ff0000000000000 }
    else if exponent != 0 { sign | ((exponent as u64 + 896) << 52) | (mantissa as u64 << 29) }
    else if mantissa == 0 { sign }
    else { sign | ((high as u64 + 874) << 52) | ((mantissa as u64 - (1_u64 << high as u64)) << (52 - high) as u64) }
}

pub(crate) fn parse<let L: u32>(text: BoundedVec<u8, L>, single: bool) -> (bool, bool, u64) {
    let negative = at(text, 0) == 45;
    let start = if negative | (at(text, 0) == 43) { 1 } else { 0 };
    let mut valid = text.len() > start;
    let mut in_exp = false;
    let mut exp_start: u32 = 0;
    let mut exp_negative = false;
    let mut exp_digits: u32 = 0;
    let mut exp: u32 = 0;
    let mut dot = false;
    let mut fraction: u32 = 0;
    let mut mantissa_digits: u32 = 0;
    let mut significant: u32 = 0;
    let mut nonzero = false;
    let mut n: [u32; (4 * L + 2048) / 32] = [0; (4 * L + 2048) / 32];
    for i in 0..L {
        if (i >= start) & (i < text.len()) {
            let b = at(text, i);
            if in_exp {
                if (i == exp_start) & ((b == 43) | (b == 45)) { exp_negative = b == 45; }
                else {
                    valid = valid & digit(b);
                    exp_digits = exp_digits + 1;
                    let next = exp * 10 + if digit(b) { (b - 48) as u32 } else { 0 };
                    exp = if next > L + 400 { L + 400 } else { next };
                }
            } else if (b == 101) | (b == 69) { in_exp = true; exp_start = i + 1; }
            else if b == 46 { valid = valid & !dot; dot = true; }
            else {
                valid = valid & digit(b);
                let d = if digit(b) { (b - 48) as u32 } else { 0 };
                n = mul10(n, d);
                mantissa_digits = mantissa_digits + 1;
                if dot { fraction = fraction + 1; }
                nonzero = nonzero | (d != 0);
                if nonzero { significant = significant + 1; }
            }
        }
    }
    valid = valid & (mantissa_digits > 0) & (!in_exp | (exp_digits > 0));
    let effective = (if exp_negative { -(exp as i32) } else { exp as i32 }) - fraction as i32;
    let order = effective + significant as i32;
    let too_large = order > 309;
    let too_small = order < -325;
    let effective = if valid & nonzero & !too_large & !too_small { effective } else { 0 };
    let mut denominator: [u32; (4 * L + 2048) / 32] = [0; (4 * L + 2048) / 32];
    denominator[0] = 1;
    for i in 0..L + 400 {
        if (i as i32) < effective { n = mul10(n, 0); }
        if (i as i32) < -effective { denominator = mul10(denominator, 0); }
    }
    let raw = rounded_bits(n, denominator, single);
    let raw = if too_large & nonzero { if single { 0x7f800000 } else { 0x7ff0000000000000 } }
        else if too_small | !nonzero { 0 } else { raw };
    let raw = if single { widen(raw as u32) } else { raw };
    let raw = raw | if negative { 0x8000000000000000 } else { 0 };
    let inf = equals(text, "INF".as_bytes()) | equals(text, "+INF".as_bytes()) | equals(text, "-INF".as_bytes());
    let nan = equals(text, "NaN".as_bytes());
    (valid | inf | nan, nan, if inf { 0x7ff0000000000000 | if negative { 0x8000000000000000 } else { 0 } } else { raw })
}

//! Lexical-to-key relations. No issuer-provided parse result is trusted.
use std::collections::bounded_vec::BoundedVec;
use crate::suite_constants::{CLASS_NONE, CLASS_DECIMAL, CLASS_DOUBLE, CLASS_DATE_TIME, CLASS_LOCAL_DATE_TIME, CLASS_BOOLEAN, CLASS_STRING, CLASS_STRING_PREFIX, CLASS_DATE, CLASS_UNREPRESENTABLE, DT_INTEGER, DT_DECIMAL, DT_DOUBLE, DT_FLOAT, DT_DATETIME, DT_BOOLEAN, DT_STRING, DT_DATE, DT_NONPOSITIVEINTEGER, DT_NEGATIVEINTEGER, DT_LONG, DT_INT, DT_SHORT, DT_BYTE, DT_NONNEGATIVEINTEGER, DT_UNSIGNEDLONG, DT_UNSIGNEDINT, DT_UNSIGNEDSHORT, DT_UNSIGNEDBYTE, DT_POSITIVEINTEGER};
use crate::term::field_lt;

pub(crate) fn at<let L: u32>(text: BoundedVec<u8, L>, i: u32) -> u8 {
    if i < text.len() { text.get(i) } else { 0 }
}
pub(crate) fn digit(b: u8) -> bool { (b >= 48) & (b <= 57) }
fn class_key(class: u8, payload: Field) -> (u8, Field) {
    (class, class as Field * 0x1000000000000000000000000000000000000000000000000000000000000 + payload)
}
pub(crate) fn recompute<let L: u32>(text: BoundedVec<u8, L>, dt: Field) -> (u8, Field) {
    let integer = (dt == DT_INTEGER) | (dt == DT_NONPOSITIVEINTEGER) | (dt == DT_NEGATIVEINTEGER)
        | (dt == DT_LONG) | (dt == DT_INT) | (dt == DT_SHORT) | (dt == DT_BYTE)
        | (dt == DT_NONNEGATIVEINTEGER) | (dt == DT_UNSIGNEDLONG) | (dt == DT_UNSIGNEDINT)
        | (dt == DT_UNSIGNEDSHORT) | (dt == DT_UNSIGNEDBYTE) | (dt == DT_POSITIVEINTEGER);
    if integer | (dt == DT_DECIMAL) { decimal(text, dt, integer) }
    else if (dt == DT_DOUBLE) | (dt == DT_FLOAT) {
        let (valid, nan, bits) = crate::term_float::parse(text, dt == DT_FLOAT);
        if !valid { class_key(CLASS_NONE, 0) }
        else if nan { class_key(CLASS_UNREPRESENTABLE, 0) }
        else {
            let bits = if (bits & 0x7fffffffffffffff) == 0 { 0 } else { bits };
            let key = if (bits >> 63) == 1 { !bits } else { bits ^ 0x8000000000000000 };
            class_key(CLASS_DOUBLE, key as Field * 0x100000000000000000000000000000000000000000000)
        }
    }
    else if dt == DT_BOOLEAN {
        let yes = equals(text, "true".as_bytes()) | equals(text, "1".as_bytes());
        let no = equals(text, "false".as_bytes()) | equals(text, "0".as_bytes());
        class_key(if yes | no { CLASS_BOOLEAN } else { CLASS_NONE }, if yes { 0x10000000000000000000000000000000000000000000000000000000000 } else { 0 })
    }
    else if dt == DT_STRING {
        let mut prefix: Field = 0;
        let mut nul = false;
        for i in 0..L { if i < text.len() { nul = nul | (text.get_unchecked(i) == 0); } }
        for i in 0..30 { prefix = prefix * 256 + at(text, i) as Field; }
        class_key(if (text.len() > 30) | nul { CLASS_STRING_PREFIX } else { CLASS_STRING }, prefix)
    }
    else if (dt == DT_DATETIME) | (dt == DT_DATE) { temporal(text, dt == DT_DATE) }
    else { class_key(CLASS_NONE, 0) }
}

pub(crate) fn equals<let L: u32, let N: u32>(text: BoundedVec<u8, L>, expected: [u8; N]) -> bool {
    let mut same = text.len() == N;
    for i in 0..N { same = same & (at(text, i) == expected[i]); }
    same
}

fn decimal<let L: u32>(text: BoundedVec<u8, L>, dt: Field, integer: bool) -> (u8, Field) {
    let negative = at(text, 0) == 45;
    let start = if negative | (at(text, 0) == 43) { 1 } else { 0 };
    let mut valid = text.len() > start;
    let mut dot = false;
    let mut digits: u32 = 0;
    let mut fraction: u32 = 0;
    let mut place: u64 = 1000000000000000000;
    let mut magnitude: Field = 0;
    let mut nonzero = false;
    let mut overflow = false;
    let limit: Field = 0x800000000000000000000000000000000000000000000000000000000000;
    for i in 0..L {
        if (i >= start) & (i < text.len()) {
            let b = text.get_unchecked(i);
            let is_digit = digit(b);
            valid = valid & (is_digit | ((b == 46) & !dot & !integer));
            if is_digit {
                digits = digits + 1;
                nonzero = nonzero | (b != 48);
                let d = (b - 48) as Field;
                let mut next = magnitude;
                if dot {
                    fraction = fraction + 1;
                    place = place / 10;
                    if fraction <= 18 { next = magnitude + d * place as Field; }
                    else { overflow = overflow | (b != 48); }
                } else { next = magnitude * 10 + d * 1000000000000000000; }
                let in_range = field_lt(next, limit);
                overflow = overflow | !in_range;
                magnitude = if in_range { next } else { 0 };
            }
            dot = dot | (b == 46);
        }
    }
    valid = valid & (digits > 0);
    let neg = negative & nonzero;
    if dt == DT_NONPOSITIVEINTEGER { valid = valid & (neg | !nonzero); }
    if dt == DT_NEGATIVEINTEGER { valid = valid & neg; }
    if dt == DT_NONNEGATIVEINTEGER { valid = valid & !neg; }
    if dt == DT_POSITIVEINTEGER { valid = valid & !neg & nonzero; }
    let mut bound: Field = 0;
    if dt == DT_LONG { bound = if neg { 9223372036854775808 } else { 9223372036854775807 }; }
    if dt == DT_INT { bound = if neg { 2147483648 } else { 2147483647 }; }
    if dt == DT_SHORT { bound = if neg { 32768 } else { 32767 }; }
    if dt == DT_BYTE { bound = if neg { 128 } else { 127 }; }
    if dt == DT_UNSIGNEDLONG { bound = 18446744073709551615; valid = valid & !neg; }
    if dt == DT_UNSIGNEDINT { bound = 4294967295; valid = valid & !neg; }
    if dt == DT_UNSIGNEDSHORT { bound = 65535; valid = valid & !neg; }
    if dt == DT_UNSIGNEDBYTE { bound = 255; valid = valid & !neg; }
    if bound != 0 { valid = valid & !overflow & !field_lt(bound * 1000000000000000000, magnitude); }
    if !valid { class_key(CLASS_NONE, 0) }
    else if overflow { class_key(CLASS_UNREPRESENTABLE, 0) }
    else { class_key(CLASS_DECIMAL, if negative { limit - magnitude } else { limit + magnitude }) }
}

fn pair<let L: u32>(text: BoundedVec<u8, L>, i: u32) -> (bool, u32) {
    let a = at(text, i);
    let b = at(text, i + 1);
    (digit(a) & digit(b), if digit(a) & digit(b) { (a - 48) as u32 * 10 + (b - 48) as u32 } else { 0 })
}
fn temporal<let L: u32>(text: BoundedVec<u8, L>, date: bool) -> (u8, Field) {
    let negative = at(text, 0) == 45;
    let start = if negative { 1 } else { 0 };
    let mut year_end = L;
    let mut year_len: u32 = 0;
    let mut year: u32 = 0;
    let mut modulo: u32 = 0;
    let mut valid = true;
    for i in 0..L {
        if (i >= start) & (i < text.len()) & (year_end == L) {
            let b = at(text, i);
            if b == 45 { year_end = i; }
            else {
                valid = valid & digit(b);
                let d = if digit(b) { (b - 48) as u32 } else { 0 };
                modulo = (modulo * 10 + d) % 400;
                if year_len < 4 { year = year * 10 + d; }
                year_len = year_len + 1;
            }
        }
    }
    valid = valid & (year_len >= 4) & ((year_len == 4) | (at(text, start) != 48)) & (year_end < L);
    let (m_ok, month) = pair(text, year_end + 1);
    let (d_ok, day) = pair(text, year_end + 4);
    let leap = (modulo % 4 == 0) & ((modulo % 100 != 0) | (modulo == 0));
    let mut days: u32 = 31;
    if (month == 4) | (month == 6) | (month == 9) | (month == 11) { days = 30; }
    if month == 2 { days = if leap { 29 } else { 28 }; }
    valid = valid & m_ok & d_ok & (at(text, year_end + 3) == 45) & (month >= 1) & (month <= 12) & (day >= 1) & (day <= days);
    let date_end = year_end + 6;
    let mut zone = date_end;
    let mut hour: u32 = 0;
    let mut minute: u32 = 0;
    let mut second: u32 = 0;
    let mut millis: u32 = 0;
    let mut fraction_digits: u32 = 0;
    let mut precise = true;
    if !date {
        let clock = date_end + 1;
        let (h_ok, h) = pair(text, clock);
        let (m_ok, m) = pair(text, clock + 3);
        let (s_ok, s) = pair(text, clock + 6);
        hour = h; minute = m; second = s;
        valid = valid & (at(text, date_end) == 84) & h_ok & m_ok & s_ok
            & (at(text, clock + 2) == 58) & (at(text, clock + 5) == 58);
        zone = clock + 8;
        if at(text, zone) == 46 {
            let first = zone + 1;
            zone = first;
            let mut ended = false;
            for i in 0..L {
                if (i >= first) & (i < text.len()) & !ended {
                    let b = at(text, i);
                    if digit(b) {
                        fraction_digits = fraction_digits + 1;
                        if fraction_digits <= 3 { millis = millis * 10 + (b - 48) as u32; }
                        else { precise = precise & (b == 48); }
                        zone = i + 1;
                    } else { ended = true; }
                }
            }
            valid = valid & (fraction_digits > 0);
            if fraction_digits == 1 { millis = millis * 100; }
            if fraction_digits == 2 { millis = millis * 10; }
        }
        let end_of_day = (hour == 24) & (minute == 0) & (second == 0) & (millis == 0) & precise;
        valid = valid & ((hour <= 23) | end_of_day) & (minute <= 59) & (second <= 59);
    }
    let zoned = zone < text.len();
    let mut offset: i64 = 0;
    if zoned {
        if at(text, zone) == 90 { valid = valid & (zone + 1 == text.len()); }
        else {
            let (h_ok, h) = pair(text, zone + 1);
            let (m_ok, m) = pair(text, zone + 4);
            valid = valid & ((at(text, zone) == 43) | (at(text, zone) == 45)) & h_ok & m_ok
                & (at(text, zone + 3) == 58) & (zone + 6 == text.len()) & (m <= 59) & (h * 60 + m <= 840);
            offset = (h as i64 * 60 + m as i64) * if at(text, zone) == 45 { -1 } else { 1 };
        }
    } else { valid = valid & (zone == text.len()); }
    let representable = !negative & (year_len == 4) & (year > 0) & (hour != 24) & precise & !(date & zoned);
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    // The representable year is positive; the sole negative intermediate is
    // the refused year-zero January/February case. Euclidean era is explicit.
    let era = if y < 0 { (y - 399) / 400 } else { y / 400 };
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let civil = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + day as i64 - 1 - 719468;
    let value = if date { civil } else { (civil * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64 - offset * 60) * 1000 + millis as i64 };
    let biased = (value as u64) ^ 0x8000000000000000;
    if !valid { class_key(CLASS_NONE, 0) }
    else if !representable { class_key(CLASS_UNREPRESENTABLE, 0) }
    else { class_key(if date { CLASS_DATE } else if zoned { CLASS_DATE_TIME } else { CLASS_LOCAL_DATE_TIME }, biased as Field * 0x100000000000000000000000000000000000000000000) }
}

// Pass-two date components; casts remain outside this checkpoint.
// Research grade; not externally audited.
use crate::expr_core::{DECIMAL, exact, INTEGER, Lane, Numeric, SCALE, StringLane};
use crate::expr_term::{empty, from_bytes};
use crate::expr_term_contract::{
    CheckedLexicalOpening, DT_DATE, DT_DATETIME, KIND_LITERAL, read_input,
};
use sparq_xpath::{XsdDateTime, XsdDayTimeDuration};
use std::collections::bounded_vec::BoundedVec;
pub global YEAR: u8 = 0;
pub global MONTH: u8 = 1;
pub global DAY: u8 = 2;
pub global HOURS: u8 = 3;
pub global MINUTES: u8 = 4;
pub global SECONDS: u8 = 5;
pub struct TemporalValue {
    pub value: XsdDateTime,
    pub fraction: Field,
    pub has_timezone: bool,
    pub zone: StringLane<6>,
    pub is_date: bool,
}
fn at<let L: u32>(a: BoundedVec<u8, L>, i: u32) -> u8 {
    if i < a.len() {
        a.get(i)
    } else {
        0
    }
}
fn digit(b: u8) -> bool {
    (b >= 48) & (b <= 57)
}
fn pair<let L: u32>(a: BoundedVec<u8, L>, i: u32) -> (u32, bool) {
    let x = at(a, i);
    let y = at(a, i + 1);
    let ok = digit(x) & digit(y);
    (
        if ok {
            (x - 48) as u32 * 10 + (y - 48) as u32
        } else {
            0
        }, ok,
    )
}
// Input validity is checked independently of the key's representability. This
// permits component extraction from fractional seconds finer than milliseconds.
pub fn read_date<T, let L: u32, let D: u32, let G: u32>(
    input: (Lane<T>, bool),
) -> (Lane<TemporalValue>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let (a, upstream_error, upstream_capacity) = read_input(input);
    let mut start: u32 = 0;
    let mut end = a.lexical.len();
    let mut leading = true;
    for i in 0..L {
        if i < a.lexical.len() {
            let b = a.lexical.get_unchecked(i);
            let ws = (b == 32) | (b == 9) | (b == 10) | (b == 13);
            if leading & ws {
                start = i + 1;
            } else {
                leading = false;
            }
        }
    }
    let mut trailing = true;
    for j in 0..L {
        let i = L - 1 - j;
        if (i >= start) & (i < a.lexical.len()) {
            let b = a.lexical.get_unchecked(i);
            let ws = (b == 32) | (b == 9) | (b == 10) | (b == 13);
            if trailing & ws {
                end = i;
            } else {
                trailing = false;
            }
        }
    }
    let mut text: BoundedVec<u8, L> = BoundedVec::new();
    for i in 0..L {
        if (i >= start) & (i < end) {
            text.push(a.lexical.get_unchecked(i));
        }
    }
    let is_date = a.cell.datatype() == DT_DATE;
    let type_ok = (a.cell.datatype() == DT_DATE) | (a.cell.datatype() == DT_DATETIME);
    let negative = at(text, 0) == 45;
    let start = negative as u32;
    let mut end = L;
    let mut count: u32 = 0;
    let mut year: u32 = 0;
    let mut modulo: u32 = 0;
    let mut nonzero = false;
    let mut valid = true;
    for i in 0..L {
        if (i >= start) & (i < text.len()) & (end == L) {
            let b = at(text, i);
            if b == 45 {
                end = i;
            } else {
                valid = valid & digit(b);
                let d = if digit(b) { (b - 48) as u32 } else { 0 };
                modulo = (modulo * 10 + d) % 400;
                nonzero = nonzero | (d != 0);
                if count < 4 {
                    year = year * 10 + d;
                }
                count = count + 1;
            }
        }
    }
    valid = valid & nonzero & (count >= 4) & ((count == 4) | (at(text, start) != 48));
    let (month, mok) = pair(text, end + 1);
    let (day, dok) = pair(text, end + 4);
    let calendar_mod = if negative {
        (401 - modulo) % 400
    } else {
        modulo
    };
    let leap = (calendar_mod % 4 == 0) & ((calendar_mod % 100 != 0) | (calendar_mod == 0));
    let days = if month == 2 {
        if leap {
            29
        } else {
            28
        }
    } else if (month == 4) | (month == 6) | (month == 9) | (month == 11) {
        30
    } else {
        31
    };
    valid = valid
        & mok
        & dok
        & (at(text, end + 3) == 45)
        & (month >= 1)
        & (month <= 12)
        & (day >= 1)
        & (day <= days);
    let mut zone = end + 6;
    let mut hour: u32 = 0;
    let mut minute: u32 = 0;
    let mut second: u32 = 0;
    let mut fraction: Field = 0;
    let mut precision = false;
    if !is_date {
        let (h, hok) = pair(text, zone + 1);
        let (m, mok) = pair(text, zone + 4);
        let (s, sok) = pair(text, zone + 7);
        valid = valid
            & (at(text, zone) == 84)
            & hok
            & mok
            & sok
            & (at(text, zone + 3) == 58)
            & (at(text, zone + 6) == 58);
        hour = h;
        minute = m;
        second = s;
        zone = zone + 9;
        if at(text, zone) == 46 {
            let first = zone + 1;
            let mut finished = false;
            let mut digits: u32 = 0;
            let mut place: Field = SCALE;
            zone = first;
            for i in 0..L {
                if (i >= first) & (i < text.len()) & !finished {
                    let b = at(text, i);
                    if digit(b) {
                        digits = digits + 1;
                        if digits <= 18 {
                            place = place / 10;
                            fraction = fraction + (b - 48) as Field * place;
                        } else {
                            precision = precision | (b != 48);
                        }
                        zone = i + 1;
                    } else {
                        finished = true;
                    }
                }
            }
            valid = valid & (digits > 0);
        }
        valid = valid
            & (minute < 60)
            & (second < 60)
            & (
                (hour < 24)
                    | ((hour == 24) & (minute == 0) & (second == 0) & (fraction == 0) & !precision)
            );
    }
    let has_timezone = zone < text.len();
    let mut offset: i16 = 0;
    let mut tz = empty();
    if has_timezone {
        if at(text, zone) == 90 {
            valid = valid & (zone + 1 == text.len());
            tz = from_bytes([90, 0, 0, 0, 0, 0], 1);
        } else {
            let (h, hok) = pair(text, zone + 1);
            let (m, mok) = pair(text, zone + 4);
            valid = valid
                & hok
                & mok
                & ((at(text, zone) == 43) | (at(text, zone) == 45))
                & (at(text, zone + 3) == 58)
                & (zone + 6 == text.len())
                & (m < 60)
                & (h * 60 + m <= 840);
            let n = (h * 60 + m) as i16;
            offset = if at(text, zone) == 45 { -n } else { n };
            tz.len = 6;
            for i in 0..6 {
                tz.bytes[i] = at(text, zone + i);
            }
        }
    } else {
        valid = valid & (zone == text.len());
    }
    let error =
        upstream_error | !a.cell.bound() | (a.cell.kind() != KIND_LITERAL) | !type_ok | !valid;
    let mut capacity = upstream_capacity | (!error & (negative | (count != 4) | precision));
    let active = !error & !capacity;
    let value = sparq_xpath::datetime_from_components_with_tz(
        if active { year as i32 } else { 1970 },
        if active { month as u8 } else { 1 },
        if active { day as u8 } else { 1 },
        if active { hour as u8 } else { 0 },
        if active { minute as u8 } else { 0 },
        if active { second as u8 } else { 0 },
        0,
        if active { offset } else { 0 },
    );
    capacity = capacity | (sparq_xpath::year_from_datetime(value) > 9999);
    (
        Lane {
            value: TemporalValue {
                value,
                fraction: if active { fraction } else { 0 },
                has_timezone,
                zone: tz,
                is_date,
            },
            capacity,
        }, error,
    )
}
pub fn component(a: (Lane<TemporalValue>, bool), function: u8) -> (Lane<Numeric>, bool) {
    assert(function <= SECONDS, "date component");
    let (a, e) = a;
    let err = e | a.value.is_date;
    let d = a.value.value;
    let n = if function == YEAR {
        sparq_xpath::year_from_datetime(d) as u32
    } else if function == MONTH {
        sparq_xpath::month_from_datetime(d) as u32
    } else if function == DAY {
        sparq_xpath::day_from_datetime(d) as u32
    } else if function == HOURS {
        sparq_xpath::hours_from_datetime(d) as u32
    } else if function == MINUTES {
        sparq_xpath::minutes_from_datetime(d) as u32
    } else {
        sparq_xpath::seconds_from_datetime(d) as u32
    };
    let value = exact(
        if function == SECONDS {
            DECIMAL
        } else {
            INTEGER
        },
        false,
        n as Field * SCALE
            + if function == SECONDS {
                a.value.fraction
            } else {
                0
            },
    );
    (Lane { value, capacity: a.capacity }, err)
}
pub fn timezone(a: (Lane<TemporalValue>, bool)) -> (Lane<XsdDayTimeDuration>, bool) {
    let (a, e) = a;
    (
        Lane { value: sparq_xpath::timezone_from_datetime(a.value.value), capacity: a.capacity },
        e | a.value.is_date | !a.value.has_timezone,
    )
}
pub fn tz(a: (Lane<TemporalValue>, bool)) -> (Lane<StringLane<6>>, bool) {
    let (a, e) = a;
    (Lane { value: a.value.zone, capacity: a.capacity }, e | a.value.is_date)
}
pub fn compare(
    a: (Lane<TemporalValue>, bool),
    b: (Lane<TemporalValue>, bool),
    op: u8,
) -> (Lane<bool>, bool) {
    assert(op <= 4, "date comparison");
    let (a, ae) = a;
    let (b, be) = b;
    let error = ae | be | (a.value.is_date != b.value.is_date);
    let capacity =
        a.capacity | b.capacity | (!error & (a.value.has_timezone != b.value.has_timezone));
    let whole = sparq_xpath::datetime_equal(a.value.value, b.value.value);
    let eq = whole & (a.value.fraction == b.value.fraction);
    let less = sparq_xpath::datetime_less_than(a.value.value, b.value.value)
        | (whole & a.value.fraction.lt(b.value.fraction));
    let result = if op == 0 {
        eq
    } else if op == 1 {
        less
    } else if op == 2 {
        less | eq
    } else if op == 3 {
        !less & !eq
    } else {
        !less
    };
    (Lane { value: result & !error & !capacity, capacity }, error)
}

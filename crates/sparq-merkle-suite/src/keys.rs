//! Comparison keys shared by the credential suite family.
use crate::KEY_BYTES;
use oxrdf::Literal;

/// Comparison-key classes. Keys compare in value order only within one class.
pub mod key_class {
    /// No comparison key: `rdf:langString`, other datatypes and ill-typed literals.
    pub const NONE: u8 = 0x00;
    /// `xsd:integer` or `xsd:decimal`, scaled by 10^18 and offset by 2^239.
    pub const DECIMAL: u8 = 0x01;
    /// `xsd:double` or `xsd:float`, as the IEEE 754 total-order key of the f64 value.
    pub const DOUBLE: u8 = 0x02;
    /// `xsd:dateTime` with a timezone: UTC milliseconds offset by 2^63.
    pub const DATE_TIME: u8 = 0x03;
    /// `xsd:dateTime` without a timezone, encoded as if it were UTC.
    pub const LOCAL_DATE_TIME: u8 = 0x04;
    /// Timezone-free `xsd:date`, as signed Gregorian days offset by 2^63.
    pub const DATE: u8 = 0x08;
    /// `xsd:boolean`: 0 or 1.
    pub const BOOLEAN: u8 = 0x05;
    /// `xsd:string` of at most 30 bytes and no U+0000.
    pub const STRING: u8 = 0x06;
    /// A longer `xsd:string`, or one containing U+0000: its first 30 bytes.
    pub const STRING_PREFIX: u8 = 0x07;
    /// A valid value outside the key's range or precision, or NaN.
    pub const UNREPRESENTABLE: u8 = 0x7f;
}

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const PAYLOAD: usize = KEY_BYTES - 1;
const DECIMAL_SCALE: usize = 18;

/// The 31-byte comparison key of `literal`.
#[must_use]
pub fn comparison_key(literal: &Literal) -> [u8; KEY_BYTES] {
    let datatype = literal.datatype().as_str();
    let lexical = literal.value();
    let mut out = [0; KEY_BYTES];
    let local = datatype.strip_prefix(XSD);
    let class = match local {
        Some(local) if integer_type(local) && valid_integer_subtype(local, lexical) => {
            decimal_key(lexical, &mut out)
        }
        Some("decimal") if is_decimal(lexical) => decimal_key(lexical, &mut out),
        Some("double") => double_key(lexical, false, &mut out),
        Some("float") => double_key(lexical, true, &mut out),
        Some("dateTime") => date_time_key(lexical, &mut out),
        Some("date") => date_key(lexical, &mut out),
        Some("boolean") => match lexical {
            "true" | "1" => {
                out[1] = 1;
                key_class::BOOLEAN
            }
            "false" | "0" => key_class::BOOLEAN,
            _ => key_class::NONE,
        },
        // SPARQL orders only simple literals and `xsd:string`; language-tagged
        // strings get no key.
        Some("string") => string_key(lexical, &mut out),
        _ => key_class::NONE,
    };
    if class == key_class::NONE || class == key_class::UNREPRESENTABLE {
        out = [0; KEY_BYTES];
    }
    out[0] = class;
    out
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn unsigned(text: &str) -> (bool, &str) {
    match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

fn is_integer(lexical: &str) -> bool {
    digits(unsigned(lexical).1)
}

fn is_decimal(lexical: &str) -> bool {
    match unsigned(lexical).1.split_once('.') {
        None => is_integer(lexical),
        Some((whole, fraction)) => {
            (whole.is_empty() || digits(whole))
                && (fraction.is_empty() || digits(fraction))
                && !(whole.is_empty() && fraction.is_empty())
        }
    }
}

// Big-endian base-256 multiply-add; false on overflow of the payload width.
fn mul_add(acc: &mut [u8; PAYLOAD], mul: u32, add: u32) -> bool {
    let mut carry = add;
    for byte in acc.iter_mut().rev() {
        let value = u32::from(*byte) * mul + carry;
        *byte = value as u8;
        carry = value >> 8;
    }
    carry == 0
}

fn decimal_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    let (negative, body) = unsigned(lexical);
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let fraction = fraction.trim_end_matches('0');
    if fraction.len() > DECIMAL_SCALE {
        return key_class::UNREPRESENTABLE;
    }
    let mut magnitude = [0u8; PAYLOAD];
    let scaled = whole
        .bytes()
        .chain(fraction.bytes())
        .chain(std::iter::repeat_n(b'0', DECIMAL_SCALE - fraction.len()));
    for digit in scaled {
        if !mul_add(&mut magnitude, 10, u32::from(digit - b'0')) {
            return key_class::UNREPRESENTABLE;
        }
    }
    // The magnitude must stay below the 2^239 offset.
    if magnitude[0] & 0x80 != 0 {
        return key_class::UNREPRESENTABLE;
    }
    let zero = magnitude.iter().all(|b| *b == 0);
    let mut offset = [0u8; PAYLOAD];
    offset[0] = 0x80;
    let payload = if negative && !zero {
        subtract(&offset, &magnitude)
    } else {
        add(&offset, &magnitude)
    };
    out[1..].copy_from_slice(&payload);
    key_class::DECIMAL
}

fn add(a: &[u8; PAYLOAD], b: &[u8; PAYLOAD]) -> [u8; PAYLOAD] {
    let mut out = [0; PAYLOAD];
    let mut carry = 0u16;
    for i in (0..PAYLOAD).rev() {
        let sum = u16::from(a[i]) + u16::from(b[i]) + carry;
        out[i] = sum as u8;
        carry = sum >> 8;
    }
    out
}

fn subtract(a: &[u8; PAYLOAD], b: &[u8; PAYLOAD]) -> [u8; PAYLOAD] {
    let mut out = [0; PAYLOAD];
    let mut borrow = 0i16;
    for i in (0..PAYLOAD).rev() {
        let mut diff = i16::from(a[i]) - i16::from(b[i]) - borrow;
        borrow = i16::from(diff < 0);
        if diff < 0 {
            diff += 256;
        }
        out[i] = diff as u8;
    }
    out
}

fn is_xsd_double(lexical: &str) -> bool {
    let body = unsigned(lexical).1;
    if lexical == "NaN" || body == "INF" {
        return true;
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        None => digits(mantissa),
        Some((whole, fraction)) => {
            (whole.is_empty() || digits(whole))
                && (fraction.is_empty() || digits(fraction))
                && !(whole.is_empty() && fraction.is_empty())
        }
    };
    mantissa_ok && exponent.is_none_or(|exponent| digits(unsigned(exponent).1))
}

fn double_key(lexical: &str, single: bool, out: &mut [u8; KEY_BYTES]) -> u8 {
    if !is_xsd_double(lexical) {
        return key_class::NONE;
    }
    let text = match lexical {
        "INF" | "+INF" => "inf",
        "-INF" => "-inf",
        "NaN" => return key_class::UNREPRESENTABLE,
        text => text,
    };
    let value = if single {
        text.parse::<f32>().map(f64::from)
    } else {
        text.parse::<f64>()
    };
    let Ok(value) = value else {
        return key_class::NONE;
    };
    // Negative zero compares equal to zero.
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    let key = if bits >> 63 == 1 {
        !bits
    } else {
        bits ^ (1 << 63)
    };
    out[1..9].copy_from_slice(&key.to_be_bytes());
    key_class::DOUBLE
}

fn number(text: &str, len: usize) -> Option<i64> {
    (text.len() == len && digits(text))
        .then(|| text.parse().ok())
        .flatten()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn date_time_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    match date_time_millis(lexical) {
        Ok((class, millis)) => {
            let key = (millis as u64) ^ (1 << 63);
            out[1..9].copy_from_slice(&key.to_be_bytes());
            class
        }
        Err(class) => class,
    }
}

// Parses an XSD 1.1 dateTime. Ill-typed input is `Err(NONE)`; a valid value
// outside the four-digit-year, millisecond, before-24:00 profile is
// `Err(UNREPRESENTABLE)`.
fn date_time_millis(lexical: &str) -> Result<(u8, i64), u8> {
    let invalid = key_class::NONE;
    let (date, time) = lexical.split_once('T').ok_or(invalid)?;
    let (negative, date) = match date.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, date),
    };
    let mut parts = date.split('-');
    let (year_text, month_text, day_text) = (
        parts.next().ok_or(invalid)?,
        parts.next().ok_or(invalid)?,
        parts.next().ok_or(invalid)?,
    );
    if parts.next().is_some()
        || year_text.len() < 4
        || !digits(year_text)
        || (year_text.len() > 4 && year_text.starts_with('0'))
    {
        return Err(invalid);
    }
    let (month, day) = (
        number(month_text, 2).ok_or(invalid)?,
        number(day_text, 2).ok_or(invalid)?,
    );
    // Leap years follow the proleptic Gregorian calendar of the actual year, of any
    // length; only years of four digits can be represented.
    let modulo = |m: u32| {
        year_text
            .bytes()
            .fold(0, |acc, digit| (acc * 10 + u32::from(digit - b'0')) % m)
    };
    let leap = modulo(4) == 0 && (modulo(100) != 0 || modulo(400) == 0);
    let year = if year_text.len() == 4 {
        year_text.parse::<i64>().map_err(|_| invalid)?
    } else {
        0
    };
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month) || day < 1 || day > month_days[(month - 1) as usize] {
        return Err(invalid);
    }
    let (clock, zone) = match time.find(['Z', '+', '-']) {
        Some(at) => (&time[..at], Some(&time[at..])),
        None => (time, None),
    };
    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) if digits(fraction) => (hms, fraction),
        Some(_) => return Err(invalid),
        None => (clock, ""),
    };
    let mut fields = hms.split(':');
    let (hour, minute, second) = (
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
    );
    let fraction = fraction.trim_end_matches('0');
    let end_of_day = hour == 24 && minute == 0 && second == 0 && fraction.is_empty();
    if fields.next().is_some() || (hour > 23 && !end_of_day) || minute > 59 || second > 59 {
        return Err(invalid);
    }
    let offset_minutes = match zone {
        None | Some("Z") => 0,
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (h, m) = zone[1..].split_once(':').ok_or(invalid)?;
            let (h, m) = (number(h, 2).ok_or(invalid)?, number(m, 2).ok_or(invalid)?);
            if m > 59 || h * 60 + m > 14 * 60 {
                return Err(invalid);
            }
            sign * (h * 60 + m)
        }
    };
    if negative || year == 0 || end_of_day || fraction.len() > 3 {
        return Err(key_class::UNREPRESENTABLE);
    }
    let millis_part = format!("{fraction:0<3}")
        .parse::<i64>()
        .map_err(|_| invalid)?;
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset_minutes * 60;
    let class = if zone.is_some() {
        key_class::DATE_TIME
    } else {
        key_class::LOCAL_DATE_TIME
    };
    Ok((class, seconds * 1_000 + millis_part))
}

fn string_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    let bytes = lexical.as_bytes();
    let take = bytes.len().min(PAYLOAD);
    out[1..=take].copy_from_slice(&bytes[..take]);
    if bytes.len() <= PAYLOAD && !bytes.contains(&0) {
        key_class::STRING
    } else {
        key_class::STRING_PREFIX
    }
}

/// Whether an XSD local name denotes integer or one of its built-in derived types.
pub fn integer_type(local: &str) -> bool {
    matches!(
        local,
        "integer"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
            | "positiveInteger"
    )
}

fn valid_integer_subtype(local: &str, text: &str) -> bool {
    if !is_integer(text) {
        return false;
    }
    let (negative, body) = unsigned(text);
    let body = body.trim_start_matches('0');
    let negative = negative && !body.is_empty();
    let within =
        |bound: &str| body.len() < bound.len() || (body.len() == bound.len() && body <= bound);
    match local {
        "nonPositiveInteger" => negative || body.is_empty(),
        "negativeInteger" => negative,
        "nonNegativeInteger" => !negative,
        "positiveInteger" => !negative && !body.is_empty(),
        "long" => within(if negative {
            "9223372036854775808"
        } else {
            "9223372036854775807"
        }),
        "int" => within(if negative { "2147483648" } else { "2147483647" }),
        "short" => within(if negative { "32768" } else { "32767" }),
        "byte" => within(if negative { "128" } else { "127" }),
        "unsignedLong" => !negative && within("18446744073709551615"),
        "unsignedInt" => !negative && within("4294967295"),
        "unsignedShort" => !negative && within("65535"),
        "unsignedByte" => !negative && within("255"),
        "integer" => true,
        _ => false,
    }
}

fn date_key(text: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    // Preserve XSD validation from dateTime, including arbitrary year lengths.
    // The calendar-day class deliberately excludes timezoned dates. The engine
    // normalizes zoned values to UTC and gives mixed timezone presence a partial
    // order; a shared day ordinal cannot encode those distinctions.
    let start = usize::from(text.starts_with('-'));
    let zone_at = text
        .char_indices()
        .skip(start)
        .filter(|(_, c)| *c == '-')
        .nth(2)
        .map(|(i, _)| i)
        .or_else(|| text.find(['Z', '+']));
    let (date, zone) = zone_at.map_or((text, ""), |i| (&text[..i], &text[i..]));
    match date_time_millis(&format!("{date}T00:00:00{zone}")) {
        Ok((_, millis)) if zone.is_empty() => {
            let days = millis / 86_400_000;
            let key = (days as u64) ^ (1 << 63);
            out[1..9].copy_from_slice(&key.to_be_bytes());
            key_class::DATE
        }
        Ok(_) | Err(key_class::UNREPRESENTABLE) => key_class::UNREPRESENTABLE,
        Err(_) => key_class::NONE,
    }
}

//! `xsd:dateTime` lexical forms (XSD 1.1 Part 2 §3.3.7) mapped to points on the time
//! line, in seconds since 1970-01-01T00:00:00 as exact rationals.

use super::rat::Rat;
use num_bigint::BigInt;
use sparq_core::civil::days_from_civil;

/// A parsed `xsd:dateTime`: the local time as seconds (read as if it were UTC) and the
/// time-zone offset in minutes, when there is one.
pub(crate) struct DateTime {
    pub(crate) local: Rat,
    pub(crate) offset: Option<i16>,
}

impl DateTime {
    /// The instant on the time line (`timeOnTimeline`); for a value without an offset,
    /// the local time read as UTC.
    pub(crate) fn instant(&self) -> Rat {
        let shift = -i64::from(self.offset.unwrap_or(0)) * 60;
        self.local.add(&Rat::int(shift))
    }
}

/// The ±14:00 bound on offsets, in seconds: how far a value without an offset may sit
/// from the instant it denotes under some offset.
pub(crate) const MAX_OFFSET_SECS: i64 = 14 * 3600;

/// Parses an `xsd:dateTime` lexical. `Err` = well-formed but with a year too large for
/// this implementation; `Ok(None)` = not in the lexical space.
pub(crate) fn parse(s: &str) -> Result<Option<DateTime>, ()> {
    let b = s.as_bytes();
    let neg = b.first() == Some(&b'-');
    let rest = &s[usize::from(neg)..];
    let Some(dash) = rest.find('-') else {
        return Ok(None);
    };
    let year_digits = &rest[..dash];
    if year_digits.len() < 4
        || (year_digits.len() > 4 && year_digits.starts_with('0'))
        || !year_digits.bytes().all(|c| c.is_ascii_digit())
    {
        return Ok(None);
    }
    let Ok(year) = year_digits.parse::<i64>() else {
        return Err(());
    };
    let year = if neg { -year } else { year };
    let r = &rest[dash..];
    // "-MM-DDThh:mm:ss" is fixed width.
    let rb = r.as_bytes();
    if rb.len() < 15
        || rb[0] != b'-'
        || rb[3] != b'-'
        || rb[6] != b'T'
        || rb[9] != b':'
        || rb[12] != b':'
    {
        return Ok(None);
    }
    let two = |i: usize| -> Option<u32> {
        let d = &rb[i..i + 2];
        d.iter()
            .all(u8::is_ascii_digit)
            .then(|| u32::from(d[0] - b'0') * 10 + u32::from(d[1] - b'0'))
    };
    let (Some(month), Some(day), Some(hour), Some(minute), Some(sec)) =
        (two(1), two(4), two(7), two(10), two(13))
    else {
        return Ok(None);
    };
    let mut tail = &r[15..];
    let mut frac = "";
    if let Some(f) = tail.strip_prefix('.') {
        let n = f.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return Ok(None);
        }
        frac = &f[..n];
        tail = &f[n..];
    }
    let offset = match tail {
        "" => None,
        "Z" => Some(0),
        _ => {
            let t = tail.as_bytes();
            if t.len() != 6 || !matches!(t[0], b'+' | b'-') || t[3] != b':' {
                return Ok(None);
            }
            let (Some(h), Some(m)) = (
                t[1..3]
                    .iter()
                    .all(u8::is_ascii_digit)
                    .then(|| u32::from(t[1] - b'0') * 10 + u32::from(t[2] - b'0')),
                t[4..6]
                    .iter()
                    .all(u8::is_ascii_digit)
                    .then(|| u32::from(t[4] - b'0') * 10 + u32::from(t[5] - b'0')),
            ) else {
                return Ok(None);
            };
            if m > 59 || h > 14 || (h == 14 && m != 0) {
                return Ok(None);
            }
            let mins = (h * 60 + m) as i16;
            Some(if t[0] == b'-' { -mins } else { mins })
        }
    };
    let leap = year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0);
    let month_len = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    let end_of_day = hour == 24 && minute == 0 && sec == 0 && frac.bytes().all(|c| c == b'0');
    if !(1..=12).contains(&month)
        || day == 0
        || day > month_len
        || minute > 59
        || sec > 59
        || (hour > 23 && !end_of_day)
    {
        return Ok(None);
    }
    let days = BigInt::from(days_from_civil(year, month, day));
    let whole = days * 86_400 + BigInt::from(hour * 3600 + minute * 60 + sec);
    let fraction = Rat::parse_decimal(&format!("0.{frac}")).expect("digits");
    Ok(Some(DateTime {
        local: Rat::int(whole).add(&fraction),
        offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Rat {
        parse(s).unwrap().unwrap().instant()
    }

    #[test]
    fn instants_are_exact_and_offsets_apply() {
        assert_eq!(at("1970-01-01T00:00:00Z"), Rat::int(0));
        assert_eq!(at("1970-01-01T01:00:00+01:00"), Rat::int(0));
        assert_eq!(at("1969-12-31T24:00:00Z"), Rat::int(0));
        assert_eq!(
            at("2000-02-29T00:00:00.5Z"),
            Rat::int(951_782_400).add(&Rat::parse_decimal("0.5").unwrap())
        );
        assert!(at("-0001-01-01T00:00:00Z") < at("0000-01-01T00:00:00Z"));
        assert!(
            parse("2001-02-29T00:00:00").unwrap().is_none(),
            "not a leap year"
        );
        for bad in [
            "01-01-01T00:00:00",
            "2000-01-01T24:00:01",
            "2000-01-01T00:00:00+14:01",
            "02000-01-01T00:00:00",
            "2000-01-01T00:00:00.",
            "2000-01-01",
        ] {
            assert!(parse(bad).unwrap().is_none(), "{bad}");
        }
        assert!(parse("12000-01-01T00:00:00").unwrap().is_some());
        assert!(parse("99999999999999999999-01-01T00:00:00").is_err());
    }
}

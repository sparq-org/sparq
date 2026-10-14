//! HTTP-dates (RFC 9110 §5.6.7): the one strict parser every precondition and validator in this
//! crate reads dates with, and the IMF-fixdate formatter for the dates it sends.
//!
//! A recipient MUST accept all three forms, and a date that fails to parse is ignored by the
//! conditional it carries. Ignoring a valid `If-Unmodified-Since` would let a lost update through,
//! so the parser accepts exactly the valid dates: the full calendar (leap years, leap seconds,
//! pre-1970 years) and the actual weekday, with the RFC 850 two-digit year resolved against `now`.
//! The grammar is fixed-width and the epoch arithmetic is the standard days-from-civil algorithm, so
//! a small self-contained parser is exact and needs no date crate.

use std::time::{SystemTime, UNIX_EPOCH};

use http::{HeaderMap, HeaderName};

/// The value of `name` when the request carries exactly one field line of it, else `None`. A date
/// field is a single HTTP-date, so a repeated one is not a valid date and its conditional is
/// ignored.
pub(crate) fn single_line(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
    let mut lines = headers.get_all(name).into_iter();
    match (lines.next(), lines.next()) {
        (Some(v), None) => v.to_str().ok(),
        _ => None,
    }
}

/// Format a [`SystemTime`] as an IMF-fixdate (RFC 9110 §5.6.7, the form a sender MUST produce), e.g.
/// `Sun, 06 Nov 1994 08:49:37 GMT`. Truncated to whole seconds; `None` before the Unix epoch.
pub(crate) fn format(t: SystemTime) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = system_time_to_unix(t)?;
    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{}, {day:02} {} {year:04} {:02}:{:02}:{:02} GMT",
        day_name_for(days),
        MONTHS[month as usize - 1],
        time / 3_600,
        time / 60 % 60,
        time % 60,
    ))
}

/// A [`SystemTime`] as whole seconds since the Unix epoch, or `None` before it.
fn system_time_to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Parse an HTTP-date in any of the three forms a recipient MUST accept (RFC 9110 §5.6.7) to whole
/// seconds since the Unix epoch (negative before it): IMF-fixdate ([`parse_imf_fixdate`]), the
/// obsolete RFC 850 form `Sunday, 06-Nov-94 08:49:37 GMT`, and the obsolete asctime form
/// `Sun Nov  6 08:49:37 1994` (UTC). Each form is fixed-width and checked strictly, calendar and
/// weekday included; `None` is "not a valid HTTP-date", which every caller ignores as RFC 9110
/// §13.1.3 and §13.1.4 require.
///
/// An RFC 850 two-digit year that would be more than 50 years after `now` is taken as the most
/// recent past year with those digits, as §5.6.7 requires.
pub(crate) fn parse(value: &str, now: SystemTime) -> Option<i64> {
    parse_imf_fixdate(value)
        .or_else(|| parse_rfc850_date(value, now))
        .or_else(|| parse_asctime_date(value))
}

/// `Sunday, 06-Nov-94 08:49:37 GMT`: a full day-name, then a fixed-width 22-byte tail.
fn parse_rfc850_date(value: &str, now: SystemTime) -> Option<i64> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    let (day_name, rest) = v.split_once(", ")?;
    let b = rest.as_bytes();
    if b.len() != 22
        || b[2] != b'-'
        || b[6] != b'-'
        || b[9] != b' '
        || b[12] != b':'
        || b[15] != b':'
        || &b[18..22] != b" GMT"
    {
        return None;
    }
    let day = two_digits(b, 0)?;
    let month = month_from_abbrev(&rest[3..6])?;
    let yy = i64::from(two_digits(b, 7)?);
    let (hh, mm, ss) = (two_digits(b, 10)?, two_digits(b, 13)?, two_digits(b, 16)?);
    // §5.6.7: a timestamp that appears MORE than 50 years in the future means the most recent past
    // year with the same last two digits. "50 years" is calendar years from `now`, compared on the
    // full timestamp, so the boundary year resolves by month, day and time too.
    let now_secs = system_time_to_unix(now).unwrap_or(0);
    let (now_year, now_month, now_day) = civil_from_days(now_secs.div_euclid(86_400));
    let horizon_day = now_day.min(days_in_month(now_year + 50, now_month));
    let horizon = days_from_civil(now_year + 50, now_month, horizon_day) * 86_400
        + now_secs.rem_euclid(86_400);
    let mut year = now_year - now_year.rem_euclid(100) + yy;
    if civil_to_unix(year, month, day, hh, mm, ss).is_some_and(|(secs, _)| secs > horizon) {
        year -= 100;
    }
    let (secs, days) = civil_to_unix(year, month, day, hh, mm, ss)?;
    let short = day_name_for(days);
    (day_name.len() > 3 && day_name.starts_with(short) && full_day_name(short) == day_name)
        .then_some(secs)
}

/// `Sun Nov  6 08:49:37 1994`: 24 bytes, the day of month space-padded to two characters.
fn parse_asctime_date(value: &str) -> Option<i64> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    let b = v.as_bytes();
    if b.len() != 24
        || b[3] != b' '
        || b[7] != b' '
        || b[10] != b' '
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b' '
    {
        return None;
    }
    let month = month_from_abbrev(&v[4..7])?;
    let day = match (b[8], b[9]) {
        (b' ', d) if d.is_ascii_digit() => u32::from(d - b'0'),
        (b' ', _) => return None,
        _ => two_digits(b, 8)?,
    };
    let (secs, days) = civil_to_unix(
        four_digits(b, 20)?,
        month,
        day,
        two_digits(b, 11)?,
        two_digits(b, 14)?,
        two_digits(b, 17)?,
    )?;
    (day_name_for(days) == &v[0..3]).then_some(secs)
}

/// Validate a civil UTC date-time and return (seconds, days) since the Unix epoch.
///
/// A leap second (`23:59:60`, RFC 5322 §3.3, which HTTP-date inherits) is accepted and counts as the
/// following midnight, the instant it precedes on the POSIX timeline; `:60` at any other minute is
/// not a real UTC time and is rejected. So is a year before 1900, which RFC 5322 §3.3 rules out.
fn civil_to_unix(year: i64, month: u32, day: u32, hh: u32, mm: u32, ss: u32) -> Option<(i64, i64)> {
    let leap_second = ss == 60 && hh == 23 && mm == 59;
    if year < 1900
        || day == 0
        || day > days_in_month(year, month)
        || hh > 23
        || mm > 59
        || (ss > 59 && !leap_second)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some((
        days * 86_400 + i64::from(hh) * 3_600 + i64::from(mm) * 60 + i64::from(ss),
        days,
    ))
}

/// The proleptic Gregorian (year, month, day) of `days` since the Unix epoch (the inverse of
/// [`days_from_civil`], same era arithmetic).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The RFC 850 full day-name for an IMF-fixdate abbreviation.
fn full_day_name(short: &str) -> &'static str {
    match short {
        "Mon" => "Monday",
        "Tue" => "Tuesday",
        "Wed" => "Wednesday",
        "Thu" => "Thursday",
        "Fri" => "Friday",
        "Sat" => "Saturday",
        _ => "Sunday",
    }
}

/// Parse an HTTP **IMF-fixdate** (RFC 9110 §5.6.7 — the format a sender MUST produce), e.g.
/// `Sun, 06 Nov 1994 08:49:37 GMT`, to whole seconds since the Unix epoch.
///
/// STRICT: the grammar is fixed-width (exactly 29 ASCII bytes after trimming), every field is
/// enforced at its exact position and width (two-digit day/time components, four-digit year, the
/// literal `", "` / `" "` / `":"` separators, the `GMT` zone), the calendar is validated for real
/// (day within the month's true length, leap years included), and the leading day-name must be the
/// ACTUAL weekday of the date (a wrong or arbitrary day-name token is malformed — RFC 9110 §5.6.7
/// fixes `day-name` to the day of week). Anything else (non-fixed-width fields, negative
/// components, a `:60` second anywhere but `23:59`) returns `None`; the obsolete forms are
/// [`parse`]'s other branches.
fn parse_imf_fixdate(value: &str) -> Option<i64> {
    // `Sun, 06 Nov 1994 08:49:37 GMT` — positions:
    //  0123456789012345678901234567 8
    //  Sun ,  06 Nov  1994 08:49:37  GMT
    let v = value.trim();
    if !v.is_ascii() {
        return None; // also guarantees the byte slicing below is char-safe
    }
    let b = v.as_bytes();
    if b.len() != 29
        || &b[3..5] != b", "
        || b[7] != b' '
        || b[11] != b' '
        || b[16] != b' '
        || b[19] != b':'
        || b[22] != b':'
        || &b[25..29] != b" GMT"
    {
        return None;
    }
    let day = two_digits(b, 5)?;
    let month = month_from_abbrev(&v[8..11])?;
    let year = four_digits(b, 12)?;
    let (secs, days) = civil_to_unix(
        year,
        month,
        day,
        two_digits(b, 17)?,
        two_digits(b, 20)?,
        two_digits(b, 23)?,
    )?;
    if day_name_for(days) != &v[0..3] {
        return None; // day-name inconsistent with the actual date (or not a day-name at all)
    }
    Some(secs)
}

/// Exactly two ASCII digits at `b[at..at+2]`, as a number.
fn two_digits(b: &[u8], at: usize) -> Option<u32> {
    let (d1, d2) = (b[at], b[at + 1]);
    if d1.is_ascii_digit() && d2.is_ascii_digit() {
        Some(u32::from(d1 - b'0') * 10 + u32::from(d2 - b'0'))
    } else {
        None
    }
}

/// Exactly four ASCII digits at `b[at..at+4]`, as a year.
fn four_digits(b: &[u8], at: usize) -> Option<i64> {
    let mut acc: i64 = 0;
    for &d in &b[at..at + 4] {
        if !d.is_ascii_digit() {
            return None;
        }
        acc = acc * 10 + i64::from(d - b'0');
    }
    Some(acc)
}

/// The true length of `month` in `year` (proleptic Gregorian, leap years included).
pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// The IMF-fixdate day-name for a day count since the Unix epoch (1970-01-01 was a Thursday).
fn day_name_for(days_since_epoch: i64) -> &'static str {
    const NAMES: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    NAMES[days_since_epoch.rem_euclid(7) as usize]
}

/// Map a three-letter English month abbreviation to its 1-based number.
fn month_from_abbrev(m: &str) -> Option<u32> {
    Some(match m {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

/// Days from the Unix epoch (1970-01-01) to `y-m-d` (proleptic Gregorian), by Howard Hinnant's
/// `days_from_civil` algorithm. Exact for all civil dates; negative for dates before the epoch.
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn format_round_trips_through_parse() {
        let now = at(1_800_000_000);
        for secs in [0, 784_111_777, 951_782_400, 1_783_254_896, 4_102_444_799] {
            let text = format(at(secs)).unwrap();
            assert_eq!(text.len(), 29, "{text}");
            assert_eq!(parse(&text, now), Some(secs as i64), "{text}");
        }
        assert_eq!(
            format(at(784_111_777)).unwrap(),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        assert_eq!(format(at(0)).unwrap(), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(format(UNIX_EPOCH - Duration::from_secs(1)), None);
    }

    #[test]
    fn only_a_single_field_line_is_a_date() {
        let mut headers = HeaderMap::new();
        let name = http::header::IF_UNMODIFIED_SINCE;
        assert_eq!(single_line(&headers, name.clone()), None);
        headers.append(
            name.clone(),
            "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap(),
        );
        assert_eq!(
            single_line(&headers, name.clone()),
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        );
        headers.append(
            name.clone(),
            "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap(),
        );
        assert_eq!(single_line(&headers, name), None);
    }

    #[test]
    fn obsolete_http_date_forms_are_accepted_for_writes() {
        // RFC 9110 §5.6.7's own example, in all three forms.
        let now = at(1_800_000_000); // 2027
        for v in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(parse(v, now), Some(784_111_777), "{v}");
        }
        assert_eq!(parse("Thu Jan  1 00:33:20 1970", now), Some(2000));
        // asctime allows a zero-padded day as well as a space-padded one.
        assert_eq!(parse("Sun Nov 06 08:49:37 1994", now), Some(784_111_777));
        // Wrong weekdays, a missing pad, a short day-name and an impossible date are rejected.
        for v in [
            "Monday, 06-Nov-94 08:49:37 GMT",
            "Sun, 06-Nov-94 08:49:37 GMT",
            "Mon Nov  6 08:49:37 1994",
            "Sun Nov 6 08:49:37 1994",
            "Thursday, 31-Nov-94 08:49:37 GMT",
        ] {
            assert_eq!(parse(v, now), None, "{v}");
        }
        // Two-digit years more than 50 years ahead belong to the previous century.
        let in_2027 = parse("Wednesday, 01-Jan-76 00:00:00 GMT", now).unwrap();
        assert_eq!(
            in_2027,
            parse_imf_fixdate("Wed, 01 Jan 2076 00:00:00 GMT").unwrap()
        );
        let late = at(4_102_444_800); // 2100-01-01
        assert_eq!(
            parse("Saturday, 06-Nov-94 08:49:37 GMT", late),
            parse_imf_fixdate("Sat, 06 Nov 2094 08:49:37 GMT")
        );
    }

    #[test]
    fn rfc850_century_cutoff_compares_the_full_timestamp() {
        // Now: 2026-10-14 10:00:00 UTC. The cutoff is 2076-10-14 10:00:00.
        let now = at(1_791_972_000);
        let date = |v| parse(v, now);
        // December 2076 is past the cutoff, so `76` there means 1976.
        assert_eq!(
            date("Friday, 31-Dec-76 12:00:00 GMT"),
            parse_imf_fixdate("Fri, 31 Dec 1976 12:00:00 GMT")
        );
        assert_eq!(date("Thursday, 31-Dec-76 12:00:00 GMT"), None);
        // The cutoff instant itself is exactly 50 years ahead, not more: it stays in 2076.
        assert_eq!(
            date("Wednesday, 14-Oct-76 10:00:00 GMT"),
            Some(3_369_895_200)
        );
        // One second later it rolls back a century (and must then carry 1976's weekday).
        assert_eq!(
            date("Thursday, 14-Oct-76 10:00:01 GMT"),
            parse_imf_fixdate("Thu, 14 Oct 1976 10:00:01 GMT")
        );
    }

    #[test]
    fn a_leap_second_is_the_following_midnight_in_every_form() {
        let now = at(1_791_972_000);
        let midnight = Some(1_483_228_800); // 2017-01-01T00:00:00Z
        for v in [
            "Sat, 31 Dec 2016 23:59:60 GMT",
            "Saturday, 31-Dec-16 23:59:60 GMT",
            "Sat Dec 31 23:59:60 2016",
        ] {
            assert_eq!(parse(v, now), midnight, "{v}");
        }
        assert_eq!(parse_imf_fixdate("Sat, 31 Dec 2016 23:58:60 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sat, 31 Dec 2016 23:59:61 GMT"), None);
    }

    #[test]
    fn imf_fixdate_known_values() {
        // The canonical RFC 9110 example: 784111777 seconds since the epoch.
        assert_eq!(
            parse_imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        // The epoch itself.
        assert_eq!(parse_imf_fixdate("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        // A round-2000s value used by the tests above.
        assert_eq!(
            parse_imf_fixdate("Thu, 01 Jan 1970 00:33:20 GMT"),
            Some(2_000)
        );
    }

    #[test]
    fn imf_fixdate_rejects_malformed() {
        assert_eq!(parse_imf_fixdate(""), None);
        assert_eq!(parse_imf_fixdate("garbage"), None);
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 1994 08:49:37 UTC"), None); // not GMT
        assert_eq!(parse_imf_fixdate("Sun, 32 Nov 1994 08:49:37 GMT"), None); // bad day
        assert_eq!(parse_imf_fixdate("Sun, 06 Foo 1994 08:49:37 GMT"), None); // bad month
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 1994 25:49:37 GMT"), None); // bad hour
        assert_eq!(
            parse_imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMT extra"),
            None
        );
        // trailing junk
    }

    #[test]
    fn imf_fixdate_enforces_strict_imf_fixdate_grammar() {
        // Non-fixed-width day (single digit) — the grammar is exactly 2DIGIT.
        assert_eq!(parse_imf_fixdate("Sun, 6 Nov 1994 08:49:37 GMT"), None);
        // Negative / non-digit components.
        assert_eq!(parse_imf_fixdate("Sun, -6 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 1994 08:4x:37 GMT"), None);
        // The obsolete RFC 850 / two-digit-year / asctime forms.
        assert_eq!(parse_imf_fixdate("Sunday, 06-Nov-94 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 94 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sun Nov  6 08:49:37 1994"), None);
        // Wrong separators at the fixed positions.
        assert_eq!(parse_imf_fixdate("Sun; 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 1994 08-49-37 GMT"), None);
        // Case-sensitive tokens.
        assert_eq!(parse_imf_fixdate("sun, 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Sun, 06 NOV 1994 08:49:37 GMT"), None);
        // `:60` outside 23:59 is not a real leap second and is rejected (fails open to a 200).
        assert_eq!(parse_imf_fixdate("Sun, 06 Nov 1994 08:49:60 GMT"), None);
        // Non-ASCII input of plausible shape must not panic (byte-slicing guard) and must fail.
        assert_eq!(parse_imf_fixdate("Sün, 06 Nov 1994 08:49:37 GMT"), None);
    }

    #[test]
    fn a_year_before_1900_is_not_an_http_date() {
        // RFC 5322 §3.3, whose year HTTP-date inherits: 1900 or later, in every form.
        let now = at(1_700_000_000);
        for invalid in [
            "Sun, 31 Dec 1899 23:59:59 GMT",
            "Sun, 31 Dec 1899 23:59:60 GMT",
            "Sun Dec 31 23:59:59 1899",
        ] {
            assert_eq!(parse(invalid, now), None, "{invalid}");
        }
        assert_eq!(
            parse("Mon, 01 Jan 1900 00:00:00 GMT", now),
            Some(-2_208_988_800)
        );
        assert_eq!(parse("Mon Jan  1 00:00:00 1900", now), Some(-2_208_988_800));
        // An RFC 850 year is within a century of `now`, never before 1900 for any `now` from
        // the epoch on (an earlier clock reads as the epoch): at the epoch, `00` is 1900 itself.
        let monday = "Monday, 01-Jan-00 00:00:00 GMT";
        assert_eq!(parse(monday, UNIX_EPOCH), Some(-2_208_988_800));
        let before_epoch = UNIX_EPOCH - Duration::from_secs(86_400 * 365 * 30);
        assert_eq!(parse(monday, before_epoch), Some(-2_208_988_800));
    }

    #[test]
    fn imf_fixdate_validates_the_real_calendar() {
        // Day zero / day beyond the month's true length.
        assert_eq!(parse_imf_fixdate("Wed, 00 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_imf_fixdate("Thu, 31 Nov 1994 08:49:37 GMT"), None); // Nov has 30 days
        // Feb 29 exists only in a leap year.
        assert_eq!(parse_imf_fixdate("Mon, 29 Feb 1999 00:00:00 GMT"), None);
        assert_eq!(parse_imf_fixdate("Tue, 29 Feb 1900 00:00:00 GMT"), None); // century, not leap
        assert_eq!(
            parse_imf_fixdate("Tue, 29 Feb 2000 00:00:00 GMT"), // 400-year rule: leap
            Some(951_782_400)
        );
    }

    #[test]
    fn imf_fixdate_requires_the_correct_weekday() {
        // 1994-11-06 was a Sunday: the correct day-name parses…
        assert!(parse_imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMT").is_some());
        // …a real-but-WRONG day-name is malformed…
        assert_eq!(parse_imf_fixdate("Mon, 06 Nov 1994 08:49:37 GMT"), None);
        // …and an arbitrary token is malformed.
        assert_eq!(parse_imf_fixdate("Xyz, 06 Nov 1994 08:49:37 GMT"), None);
    }

    // --- Representation-variant tags (`"<state>+<variant>"`) ---------------------------------------
}

//! `xsd:dateTime` / `xsd:date` / `xsd:duration` value comparison, independent of sparq.
//! sq-qcnn.4
//!
//! Everything here is built from a proleptic-Gregorian day count (Howard Hinnant's `days_from_civil`)
//! plus arbitrary-precision arithmetic ([`bigdecimal`]) — no `chrono`/`time` and, crucially, no sparq
//! temporal code, so a bug in the engine's own temporal handling cannot cancel out of the differential.
//!
//! The comparisons are **three-valued** ([`TemporalOrder`]) because XSD temporal order is *partial*:
//!
//! * A timezone-less `dateTime` compared with a timezoned one falls in the XPath **±14h indeterminate
//!   window** — an implicit timezone could place it either side, so the result may be `Indeterminate`
//!   rather than a definite order. This is a legitimate cross-engine divergence source (engines pick
//!   different implicit timezones), so the harness must triage it, not auto-fail.
//! * `xsd:duration` is only *partially* ordered — `P1M` (one month) and `P30D` (30 days) are
//!   **incomparable** — so [`duration_compare`] returns `Indeterminate` whenever the month and second
//!   components move in opposite directions (the conservative XSD monotone order).

use std::cmp::Ordering;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use num_bigint::BigInt;

use crate::term::XSD;

/// A three-valued temporal order: XSD temporal comparison is partial, so a definite `<`/`=`/`>` is not
/// always available (the ±14h dateTime window; the duration month-vs-second partial order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalOrder {
    Less,
    Equal,
    Greater,
    /// No definite order (the ±14h window, or an incomparable duration pair). Route to triage.
    Indeterminate,
}

/// An `xsd:dateTime`/`xsd:date` value: seconds from the Unix epoch, plus whether the lexical carried a
/// timezone. A timezoned value's `seconds` is normalised to **UTC**; a timezone-less value's `seconds`
/// is its **local** wall-clock reading (compared under the ±14h rule).
#[derive(Debug, Clone)]
pub struct DateTimeValue {
    seconds: BigDecimal,
    has_tz: bool,
}

impl DateTimeValue {
    /// A stable multiset-keying string. Timezoned values key on their UTC instant (so two lexical
    /// spellings of the same instant collide); timezone-less values key on their local reading under a
    /// distinct tag (so a timezone-less value never collides with a timezoned one — the ±14h
    /// indeterminacy is preserved as "not equal" for keying, the conservative choice).
    pub(crate) fn canonical_key(&self) -> String {
        let tag = if self.has_tz { 'Z' } else { 'L' };
        format!("{}{}", tag, self.seconds.normalized())
    }
}

/// An `xsd:duration` value decomposed into its two independent axes: `months` (year+month) and
/// `seconds` (day+time). They are compared component-wise (the XSD monotone partial order).
#[derive(Debug, Clone)]
pub struct Duration {
    months: BigInt,
    seconds: BigDecimal,
}

impl Duration {
    /// A stable multiset-keying string: two durations with equal month and second components key the
    /// same (so `P1Y` and `P12M` collide by value), an incomparable pair keys differently.
    pub(crate) fn canonical_key(&self) -> String {
        format!("{}\u{1f}{}", self.months, self.seconds.normalized())
    }
}

/// Days in a proleptic-Gregorian month, using astronomical year numbering (year 0 exists), so the leap
/// rule applies directly to year 0 and negative (BCE) years. `None` for a month outside 1–12.
fn days_in_month(year: i64, month: i64) -> Option<i64> {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

/// Days from 1970-01-01 for a proleptic-Gregorian date, using astronomical year numbering (year 0
/// exists). Howard Hinnant's `days_from_civil`; valid across the whole integer year range.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Parse an `xsd:dateTime` or `xsd:date` `(lexical, datatype-IRI)` into a comparable value.
pub fn parse_datetime(lexical: &str, datatype: &str) -> Option<DateTimeValue> {
    let local = datatype.strip_prefix(XSD)?;
    let has_time = match local {
        "dateTime" => true,
        "date" => false,
        _ => return None,
    };
    let s = lexical.trim();
    // Leading '-' is a negative (BCE) year sign; strip it before the internal date separators.
    let (neg_year, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };

    let (date_str, time_str, tz) = if has_time {
        let (dpart, tpart) = rest.split_once('T')?;
        let (tstr, tz) = split_timezone(tpart)?;
        (dpart, Some(tstr), tz)
    } else {
        let (dstr, tz) = split_date_timezone(rest)?;
        (dstr, None, tz)
    };

    let mut dparts = date_str.split('-');
    let year: i64 = dparts.next()?.parse().ok()?;
    let month: i64 = dparts.next()?.parse().ok()?;
    let day: i64 = dparts.next()?.parse().ok()?;
    if dparts.next().is_some() {
        return None;
    }
    let signed_year = if neg_year { -year } else { year };
    // Reject an out-of-range calendar date (month 0/13, day 0/32, 30 Feb, 29 Feb in a non-leap year,
    // …). An invalid lexical must NOT be value-canonicalised: it falls through to exact-lexical keying
    // so a genuine cross-engine divergence is never masked by an accidental value collision.
    if !(1..=days_in_month(signed_year, month)?).contains(&day) {
        return None;
    }
    let days = days_from_civil(signed_year, month, day);

    let (hh, mm, frac) = match time_str {
        Some(t) => parse_time(t)?,
        None => (0i64, 0i64, BigDecimal::from(0)),
    };

    let base = days * 86_400 + hh * 3_600 + mm * 60;
    let mut seconds = BigDecimal::from(base) + frac;
    if let Some(offset) = tz {
        // local = UTC + offset  =>  UTC = local - offset.
        seconds -= BigDecimal::from(offset);
    }
    Some(DateTimeValue {
        seconds,
        has_tz: tz.is_some(),
    })
}

/// Split a `"hh:mm:ss[.frac]"` (dateTime time part) trailing timezone off; returns `(time, offset)`
/// where `offset` is `Some(seconds)` for a present timezone (`Z` -> 0) and `None` otherwise.
fn split_timezone(tpart: &str) -> Option<(&str, Option<i64>)> {
    if let Some(t) = tpart.strip_suffix('Z') {
        return Some((t, Some(0)));
    }
    // The time "hh:mm:ss.fff" contains no '+' or '-', so the first one marks the timezone.
    if let Some(pos) = tpart.find(['+', '-']) {
        let (t, tzs) = tpart.split_at(pos);
        return Some((t, Some(parse_tz_offset(tzs)?)));
    }
    Some((tpart, None))
}

/// Split a trailing timezone off an `xsd:date` `"yyyy-mm-dd[tz]"` body (the year sign already removed,
/// so the date contains exactly two '-' separators; a third marks a negative timezone).
fn split_date_timezone(rest: &str) -> Option<(&str, Option<i64>)> {
    if let Some(d) = rest.strip_suffix('Z') {
        return Some((d, Some(0)));
    }
    if let Some(pos) = rest.find('+') {
        return Some((&rest[..pos], Some(parse_tz_offset(&rest[pos..])?)));
    }
    // Count '-' separators: 2 => no tz, 3 => the last group is a negative tz.
    let dash_count = rest.matches('-').count();
    if dash_count <= 2 {
        return Some((rest, None));
    }
    let pos = rest.rmatch_indices('-').next()?.0;
    Some((&rest[..pos], Some(parse_tz_offset(&rest[pos..])?)))
}

/// Parse a `"±hh:mm"` timezone into signed seconds.
fn parse_tz_offset(tz: &str) -> Option<i64> {
    let (sign, body) = match tz.strip_prefix('+') {
        Some(b) => (1, b),
        None => (-1, tz.strip_prefix('-')?),
    };
    let (h, m) = body.split_once(':')?;
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    // XSD `timezoneFrag` range: minutes 0–59, hours 0–14, and at the ±14:00 bound the minutes must be
    // 0 (14:01..14:59 is out of range). An out-of-range offset is an invalid lexical and must NOT be
    // value-canonicalised — it falls through to exact-lexical keying so a genuine cross-engine
    // divergence is never masked by an accidental value collision.
    if !(0..=59).contains(&m) || !(0..=14).contains(&h) || (h == 14 && m != 0) {
        return None;
    }
    Some(sign * (h * 3_600 + m * 60))
}

/// Parse a `"hh:mm:ss[.frac]"` time into `(hours, minutes, seconds-with-fraction)`.
fn parse_time(t: &str) -> Option<(i64, i64, BigDecimal)> {
    let mut parts = t.split(':');
    let hh: i64 = parts.next()?.parse().ok()?;
    let mm: i64 = parts.next()?.parse().ok()?;
    let ss = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let frac = BigDecimal::from_str(ss).ok()?;
    // XSD time-of-day ranges: minutes 0–59; seconds in [0, 60) (XSD has no leap seconds, so 60 is
    // invalid, and a negative seconds component such as `00:00:-1` is likewise invalid). Hours are
    // 0–23, plus the special end-of-day `24:00:00` (equal to 00:00:00 of the next day). Anything else
    // is an invalid lexical and must return `None` so the caller falls back to exact-lexical keying.
    let zero = BigDecimal::from(0);
    let sixty = BigDecimal::from(60);
    if !(0..=59).contains(&mm) || frac < zero || frac >= sixty {
        return None;
    }
    match hh {
        0..=23 => {}
        24 if mm == 0 && frac.cmp(&zero) == Ordering::Equal => {}
        _ => return None,
    }
    Some((hh, mm, frac))
}

/// Compare two `xsd:dateTime`/`date` values. Two timezoned (or two timezone-less) values compare
/// definitely; a timezoned-vs-timezone-less pair uses the XPath ±14h rule and may be `Indeterminate`.
pub fn dt_compare(a: &DateTimeValue, b: &DateTimeValue) -> TemporalOrder {
    match (a.has_tz, b.has_tz) {
        (true, true) | (false, false) => cmp_to_order(a.seconds.cmp(&b.seconds)),
        (true, false) => tz_vs_local(&a.seconds, &b.seconds),
        (false, true) => flip(tz_vs_local(&b.seconds, &a.seconds)),
    }
}

/// The ±14h rule: `u` is a timezoned value's UTC instant, `l` a timezone-less local reading whose true
/// instant could be anywhere in `[l - 14h, l + 14h]`. Definite only outside that window.
fn tz_vs_local(u: &BigDecimal, l: &BigDecimal) -> TemporalOrder {
    let fourteen_h = BigDecimal::from(14 * 3_600);
    if *u < l - &fourteen_h {
        TemporalOrder::Less
    } else if *u > l + &fourteen_h {
        TemporalOrder::Greater
    } else {
        TemporalOrder::Indeterminate
    }
}

fn flip(o: TemporalOrder) -> TemporalOrder {
    match o {
        TemporalOrder::Less => TemporalOrder::Greater,
        TemporalOrder::Greater => TemporalOrder::Less,
        other => other,
    }
}

fn cmp_to_order(o: Ordering) -> TemporalOrder {
    match o {
        Ordering::Less => TemporalOrder::Less,
        Ordering::Equal => TemporalOrder::Equal,
        Ordering::Greater => TemporalOrder::Greater,
    }
}

/// The XSD duration lexical subspace selected by the datatype — i.e. which unit letters are legal
/// (XSD 1.1 §3.4.26–28). `xsd:yearMonthDuration` and `xsd:dayTimeDuration` are *restrictions* of
/// `xsd:duration`, so a lexical carrying a unit outside its subset is invalid and must not be
/// value-canonicalised (it falls through to exact-lexical keying instead).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DurKind {
    /// `xsd:duration` — all of `Y M D H M S`.
    Full,
    /// `xsd:yearMonthDuration` — only `Y` and the date-side `M`; no day/time part.
    YearMonth,
    /// `xsd:dayTimeDuration` — only `D H` and the time-side `M S`; no year/month.
    DayTime,
}

/// Parse an `xsd:duration` / `xsd:yearMonthDuration` / `xsd:dayTimeDuration` into a comparable value,
/// enforcing each datatype's unit subset (a `dayTimeDuration` rejects `Y`/`M`, a `yearMonthDuration`
/// rejects any day/time component) and the XSD rule that only the seconds component may be fractional.
pub fn parse_duration(lexical: &str, datatype: &str) -> Option<Duration> {
    let local = datatype.strip_prefix(XSD)?;
    let kind = match local {
        "duration" => DurKind::Full,
        "yearMonthDuration" => DurKind::YearMonth,
        "dayTimeDuration" => DurKind::DayTime,
        _ => return None,
    };
    let s = lexical.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let s = s.strip_prefix('P')?;
    let (dpart, tpart) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    // `xsd:yearMonthDuration` has no day/time part at all (not even an empty `T`).
    if kind == DurKind::YearMonth && tpart.is_some() {
        return None;
    }

    let mut months = BigInt::from(0);
    let mut seconds = BigDecimal::from(0);
    let mut any = false;
    read_components(dpart, false, kind, &mut months, &mut seconds, &mut any)?;
    if let Some(t) = tpart {
        // A 'T' with nothing after it is invalid.
        if t.is_empty() {
            return None;
        }
        read_components(t, true, kind, &mut months, &mut seconds, &mut any)?;
    }
    if !any {
        return None;
    }
    if neg {
        months = -months;
        seconds = -seconds;
    }
    Some(Duration { months, seconds })
}

/// Read a run of `<number><unit>` components into the month/second accumulators. `is_time` selects the
/// time-side unit set (`H`/`M`/`S`) versus the date-side set (`Y`/`M`/`D`); `kind` restricts the legal
/// units to the datatype's lexical subspace (see [`DurKind`]). Only the seconds component may be
/// fractional; a fraction on any other unit is an invalid lexical and is rejected.
fn read_components(
    part: &str,
    is_time: bool,
    kind: DurKind,
    months: &mut BigInt,
    seconds: &mut BigDecimal,
    any: &mut bool,
) -> Option<()> {
    let mut cur = part;
    // XSD duration requires each unit to appear at most once and in the fixed order Y,M,D (date part) /
    // H,M,S (time part). A strictly increasing rank enforces both: an out-of-order or repeated unit
    // (e.g. `P1D2Y`, `P1Y1Y`) is an invalid lexical and must be rejected — it falls through to
    // exact-lexical keying rather than being value-canonicalised (which could mask a divergence).
    let mut last_rank: i32 = -1;
    while !cur.is_empty() {
        let numend = cur.find(|c: char| !c.is_ascii_digit() && c != '.')?;
        let (num, rest) = cur.split_at(numend);
        let unit = rest.chars().next()?;
        cur = &rest[unit.len_utf8()..];
        if num.is_empty() {
            return None;
        }
        // In XSD duration lexical space only the seconds component may carry a fraction.
        if num.contains('.') && !(is_time && unit == 'S') {
            return None;
        }
        let rank = match (is_time, unit) {
            (false, 'Y') => 0,
            (false, 'M') => 1,
            (false, 'D') => 2,
            (true, 'H') => 0,
            (true, 'M') => 1,
            (true, 'S') => 2,
            _ => return None,
        };
        if rank <= last_rank {
            return None;
        }
        last_rank = rank;
        match (is_time, unit) {
            // Year/month axis — illegal in a `dayTimeDuration`.
            (false, 'Y') if kind != DurKind::DayTime => {
                *months += BigInt::from_str(num).ok()? * BigInt::from(12)
            }
            (false, 'M') if kind != DurKind::DayTime => *months += BigInt::from_str(num).ok()?,
            // Day/time axis — illegal in a `yearMonthDuration`.
            (false, 'D') if kind != DurKind::YearMonth => {
                *seconds = seconds.clone() + BigDecimal::from_str(num).ok()? * BigDecimal::from(86_400)
            }
            (true, 'H') if kind != DurKind::YearMonth => {
                *seconds = seconds.clone() + BigDecimal::from_str(num).ok()? * BigDecimal::from(3_600)
            }
            (true, 'M') if kind != DurKind::YearMonth => {
                *seconds = seconds.clone() + BigDecimal::from_str(num).ok()? * BigDecimal::from(60)
            }
            (true, 'S') if kind != DurKind::YearMonth => {
                *seconds = seconds.clone() + BigDecimal::from_str(num).ok()?
            }
            _ => return None,
        }
        *any = true;
    }
    Some(())
}

/// Compare two `xsd:duration` values under the XSD monotone partial order: definite only when the month
/// and second components agree in direction; `Indeterminate` when they diverge (`P1M` vs `P30D`).
pub fn duration_compare(a: &Duration, b: &Duration) -> TemporalOrder {
    let mc = a.months.cmp(&b.months);
    let sc = a.seconds.cmp(&b.seconds);
    use Ordering::{Equal, Greater, Less};
    match (mc, sc) {
        (Equal, Equal) => TemporalOrder::Equal,
        (Less, Less) | (Less, Equal) | (Equal, Less) => TemporalOrder::Less,
        (Greater, Greater) | (Greater, Equal) | (Equal, Greater) => TemporalOrder::Greater,
        _ => TemporalOrder::Indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
    const DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
    const DUR: &str = "http://www.w3.org/2001/XMLSchema#duration";
    const YM: &str = "http://www.w3.org/2001/XMLSchema#yearMonthDuration";
    const DAYT: &str = "http://www.w3.org/2001/XMLSchema#dayTimeDuration";

    fn dt(s: &str) -> DateTimeValue {
        parse_datetime(s, DT).unwrap()
    }

    #[test]
    fn parse_datetime_forms() {
        // Z, offset, timezone-less, fractional seconds, xsd:date, negative year.
        assert!(parse_datetime("2020-01-01T00:00:00Z", DT).is_some());
        assert!(parse_datetime("2020-01-01T00:00:00+01:00", DT).is_some());
        assert!(parse_datetime("2020-01-01T00:00:00", DT).is_some());
        assert!(parse_datetime("2020-01-01T00:00:00.500Z", DT).is_some());
        assert!(parse_datetime("2020-01-01", DATE).is_some());
        assert!(parse_datetime("2020-01-01-05:00", DATE).is_some());
        assert!(parse_datetime("-0044-03-15T00:00:00Z", DT).is_some());
        // wrong datatype / malformed.
        assert!(parse_datetime("2020-01-01T00:00:00Z", DUR).is_none());
        assert!(parse_datetime("not-a-date", DT).is_none());
    }

    #[test]
    fn dt_compare_timezone_equivalence() {
        // Same instant, different lexical -> Equal.
        assert_eq!(
            dt_compare(&dt("2020-01-01T13:00:00Z"), &dt("2020-01-01T14:00:00+01:00")),
            TemporalOrder::Equal
        );
        // Different instant -> definite order.
        assert_eq!(
            dt_compare(&dt("2020-01-01T13:00:00Z"), &dt("2020-01-01T13:00:00-05:00")),
            TemporalOrder::Less
        );
        // Timezone-less vs timezoned, well outside ±14h -> definite.
        assert_eq!(
            dt_compare(
                &parse_datetime("2020-01-05T00:00:00", DT).unwrap(),
                &dt("2020-01-01T00:00:00Z")
            ),
            TemporalOrder::Greater
        );
        // Timezone-less vs timezoned, within ±14h -> Indeterminate.
        assert_eq!(
            dt_compare(
                &parse_datetime("2020-01-01T06:00:00", DT).unwrap(),
                &dt("2020-01-01T00:00:00Z")
            ),
            TemporalOrder::Indeterminate
        );
        // Two timezone-less values compare directly.
        assert_eq!(
            dt_compare(
                &parse_datetime("2020-01-01T00:00:00", DT).unwrap(),
                &parse_datetime("2020-01-01T00:00:01", DT).unwrap()
            ),
            TemporalOrder::Less
        );
    }

    #[test]
    fn dt_compare_is_symmetric_under_argument_reversal() {
        // sq-dfoik: exercise every timezone pairing and both mixed-pair outcomes.
        let cases = [
            (
                "both timezoned",
                dt("2020-01-01T00:00:00Z"),
                dt("2020-01-02T00:00:00Z"),
                TemporalOrder::Less,
            ),
            (
                "both timezone-less",
                dt("2020-01-01T00:00:00"),
                dt("2020-01-02T00:00:00"),
                TemporalOrder::Less,
            ),
            (
                "mixed outside the 14-hour window",
                dt("2020-01-01T00:00:00Z"),
                dt("2020-01-03T00:00:00"),
                TemporalOrder::Less,
            ),
            (
                "mixed within the 14-hour window",
                dt("2020-01-01T00:00:00Z"),
                dt("2020-01-01T06:00:00"),
                TemporalOrder::Indeterminate,
            ),
        ];

        for (case, a, b, expected) in cases {
            let forward = dt_compare(&a, &b);
            assert_eq!(forward, expected, "unexpected forward order for {case}");
            assert_eq!(
                flip(forward),
                dt_compare(&b, &a),
                "argument reversal did not flip the order for {case}"
            );
        }

        for value in [dt("2020-01-01T00:00:00Z"), dt("2020-01-01T00:00:00")] {
            assert_eq!(dt_compare(&value, &value), TemporalOrder::Equal);
        }
    }

    #[test]
    fn parse_datetime_rejects_invalid_calendar_fields() {
        // Out-of-range month/day must NOT value-canonicalise — they fall back to exact-lexical keying.
        assert!(parse_datetime("2020-00-01T00:00:00Z", DT).is_none(), "month 0");
        assert!(parse_datetime("2020-13-01T00:00:00Z", DT).is_none(), "month 13");
        assert!(parse_datetime("2020-01-00T00:00:00Z", DT).is_none(), "day 0");
        assert!(parse_datetime("2020-01-32T00:00:00Z", DT).is_none(), "day 32");
        assert!(parse_datetime("2019-04-31", DATE).is_none(), "April has 30 days");
        assert!(parse_datetime("2021-02-29", DATE).is_none(), "29 Feb in a non-leap year");
        // Real dates are still accepted, including the leap day of a leap year.
        assert!(parse_datetime("2020-02-29", DATE).is_some(), "2020 is a leap year");
        assert!(parse_datetime("2019-04-30", DATE).is_some());
    }

    #[test]
    fn parse_time_rejects_out_of_range_and_accepts_end_of_day() {
        // Hour/minute ranges and a non-negative seconds component in [0, 60).
        assert!(parse_datetime("2020-01-01T25:00:00Z", DT).is_none(), "hour 25");
        assert!(parse_datetime("2020-01-01T00:60:00Z", DT).is_none(), "minute 60");
        assert!(parse_datetime("2020-01-01T00:00:60Z", DT).is_none(), "second 60 (no leap seconds)");
        assert!(parse_datetime("2020-01-01T00:00:-1Z", DT).is_none(), "negative second");
        // The XSD end-of-day form 24:00:00 equals 00:00:00 of the next day (so it must canonicalise).
        assert_eq!(
            dt_compare(&dt("2020-01-01T24:00:00Z"), &dt("2020-01-02T00:00:00Z")),
            TemporalOrder::Equal
        );
        // …but a non-zero minute/second alongside hour 24 is invalid.
        assert!(parse_datetime("2020-01-01T24:00:01Z", DT).is_none());
        assert!(parse_datetime("2020-01-01T24:30:00Z", DT).is_none());
    }

    #[test]
    fn parse_datetime_rejects_out_of_range_timezone() {
        // XSD timezone range is ±14:00; the bound itself is inclusive but must have zero minutes.
        assert!(parse_datetime("2020-01-01T00:00:00+14:00", DT).is_some(), "±14:00 is the bound");
        assert!(parse_datetime("2020-01-01T00:00:00-14:00", DT).is_some());
        assert!(parse_datetime("2020-01-01T00:00:00+15:00", DT).is_none(), "beyond ±14:00");
        assert!(parse_datetime("2020-01-01T00:00:00+14:30", DT).is_none(), "14:30 exceeds the bound");
        assert!(parse_datetime("2020-01-01T00:00:00+00:60", DT).is_none(), "tz minute 60");
        assert!(parse_datetime("2020-01-01-15:00", DATE).is_none(), "date tz beyond ±14:00");
    }

    #[test]
    fn parse_duration_forms() {
        assert!(parse_duration("P1Y2M3DT4H5M6S", DUR).is_some());
        assert!(parse_duration("-P1Y", YM).is_some());
        assert!(parse_duration("PT1.5S", DAYT).is_some());
        assert!(parse_duration("P30D", DAYT).is_some());
        // invalid: empty, no P, bad unit, fractional year.
        assert!(parse_duration("P", DUR).is_none());
        assert!(parse_duration("1Y", DUR).is_none());
        assert!(parse_duration("PT", DUR).is_none());
        assert!(parse_duration("P1.5Y", DUR).is_none());
        assert!(parse_duration("P1Y", DT).is_none());
    }

    #[test]
    fn parse_duration_enforces_datatype_unit_subsets() {
        // `xsd:yearMonthDuration`: only Y and (date) M; every day/time unit is rejected.
        assert!(parse_duration("P1M", YM).is_some());
        assert!(parse_duration("P1Y2M", YM).is_some());
        assert!(parse_duration("P30D", YM).is_none(), "day is not a yearMonth unit");
        assert!(parse_duration("PT1H", YM).is_none(), "a time part is illegal in yearMonth");
        assert!(parse_duration("P1YT0S", YM).is_none(), "even a zero time part is illegal");
        // `xsd:dayTimeDuration`: only D and (time) H/M/S; year and (date) month are rejected.
        assert!(parse_duration("PT1H", DAYT).is_some());
        assert!(parse_duration("P1DT2H3M4.5S", DAYT).is_some());
        assert!(parse_duration("P1Y", DAYT).is_none(), "year is not a dayTime unit");
        assert!(parse_duration("P1M", DAYT).is_none(), "date-side month is not a dayTime unit");
    }

    #[test]
    fn parse_duration_only_seconds_may_be_fractional() {
        // Seconds may carry a fraction; nothing else may.
        assert!(parse_duration("PT1.5S", DUR).is_some());
        assert!(parse_duration("P1.5D", DUR).is_none(), "fractional day is invalid");
        assert!(parse_duration("P1.5M", DUR).is_none(), "fractional (date) month is invalid");
        assert!(parse_duration("PT1.5H", DUR).is_none(), "fractional hour is invalid");
        assert!(parse_duration("PT1.5M", DUR).is_none(), "fractional minute is invalid");
    }

    #[test]
    fn parse_duration_requires_ordered_unique_units() {
        // Correct order is accepted.
        assert!(parse_duration("P1Y2M3DT4H5M6S", DUR).is_some());
        // Out-of-order units are an invalid lexical (must fall back to exact-lexical keying).
        assert!(parse_duration("P1D2Y", DUR).is_none(), "date units out of order");
        assert!(parse_duration("PT1M2H", DUR).is_none(), "time units out of order");
        // A repeated unit is invalid.
        assert!(parse_duration("P1Y1Y", DUR).is_none(), "repeated year");
        assert!(parse_duration("PT1S2S", DAYT).is_none(), "repeated second");
    }

    #[test]
    fn duration_compare_partial_order() {
        let p1m = parse_duration("P1M", DUR).unwrap();
        let p30d = parse_duration("P30D", DUR).unwrap();
        // one-month vs thirty-days is the canonical incomparable pair.
        assert_eq!(duration_compare(&p1m, &p30d), TemporalOrder::Indeterminate);
        // equal value via different lexical (P1Y == P12M).
        assert_eq!(
            duration_compare(
                &parse_duration("P1Y", YM).unwrap(),
                &parse_duration("P12M", YM).unwrap()
            ),
            TemporalOrder::Equal
        );
        // monotone: adding days keeps the same month component -> definite.
        assert_eq!(
            duration_compare(
                &parse_duration("P1M15D", DUR).unwrap(),
                &parse_duration("P1M", DUR).unwrap()
            ),
            TemporalOrder::Greater
        );
        assert_eq!(
            duration_compare(&p30d, &parse_duration("P31D", DAYT).unwrap()),
            TemporalOrder::Less
        );
    }

    #[test]
    fn canonical_keys_are_value_stable() {
        // Same instant, different lexical -> same key.
        assert_eq!(
            dt("2020-01-01T13:00:00Z").canonical_key(),
            dt("2020-01-01T14:00:00+01:00").canonical_key()
        );
        // Timezone-less never collides with timezoned.
        assert_ne!(
            parse_datetime("2020-01-01T13:00:00", DT).unwrap().canonical_key(),
            dt("2020-01-01T13:00:00Z").canonical_key()
        );
        // P1Y and P12M key the same by value.
        assert_eq!(
            parse_duration("P1Y", YM).unwrap().canonical_key(),
            parse_duration("P12M", YM).unwrap().canonical_key()
        );
        assert_ne!(
            parse_duration("P1M", DUR).unwrap().canonical_key(),
            parse_duration("P30D", DUR).unwrap().canonical_key()
        );
    }
}

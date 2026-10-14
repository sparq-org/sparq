// AUTHORED-BY Claude Fable 5
//! A tiny, **dependency-free** round-trip between [`SystemTime`] and the `xsd:dateTime` lexical form
//! the index stores for a resource's modification time (`pss:modified`).
//!
//! # Why this exists / why no date crate
//! The SPARQ index records a resource's server-side write time as an `xsd:dateTime` literal (see
//! [`super::sparql`]); the conditional-GET evaluator ([`crate::ldp::conditional::evaluate_read`])
//! compares that time — as a [`SystemTime`] — against an `If-Modified-Since` header. So the store
//! must serialise a `SystemTime` INTO the index on write and parse it BACK on read.
//!
//! We do NOT pull in a date crate for this: `chrono` is only a *transitive* dependency (via
//! `object_store`), not a direct one, and adding a direct date dependency for one canonical-UTC
//! round-trip is not worth the supply-chain surface — the same reasoning the HTTP-date parser in
//! `crate::http_date` documents, whose exact civil-date arithmetic (Howard Hinnant's
//! `days_from_civil` / `civil_from_days`, valid for all proleptic-Gregorian dates) this reuses.
//!
//! # The correctness contract (fail-OPEN on read)
//! Both directions are TOTAL and never panic. On the READ side a value that cannot be parsed (a
//! backend that stored a form we do not recognise, a pre-epoch or otherwise nonsensical instant)
//! yields `None`, and the conditional evaluator then treats the modification time as *unknown* and
//! serves a fresh 200 — **never** a spurious 304. Strictness therefore costs at most a cache miss,
//! never stale content. We only ever *emit* the canonical `YYYY-MM-DDTHH:MM:SSZ` form, so the parser
//! only has to be lenient enough to also accept the equivalents a standards-conformant SPARQL engine
//! might hand back (fractional seconds, an explicit `+00:00`/`-HH:MM` offset).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::http_date::{civil_from_days, days_from_civil, days_in_month};

/// Seconds in one day.
const SECS_PER_DAY: i64 = 86_400;

/// Format a [`SystemTime`] as the canonical UTC `xsd:dateTime` lexical form
/// (`YYYY-MM-DDTHH:MM:SSZ`), truncated to whole seconds. Returns `None` for a pre-epoch instant (no
/// stored resource realistically has one) — the caller then records no modification time (⇒ read
/// fails open to a fresh 200).
///
/// Sub-second precision is intentionally dropped: `If-Modified-Since` (RFC 9110 §5.6.7) has
/// whole-second granularity and the evaluator compares whole seconds, so a canonical whole-second
/// lexical form is exactly what the comparison needs.
pub fn to_xsd_datetime(t: SystemTime) -> Option<String> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(SECS_PER_DAY);
    let tod = secs.rem_euclid(SECS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    let hh = tod / 3_600;
    let mm = (tod % 3_600) / 60;
    let ss = tod % 60;
    Some(format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"))
}

/// Parse an `xsd:dateTime` lexical form back to a [`SystemTime`], or `None` if it is not a form we
/// recognise / cannot represent (the caller then treats the modification time as unknown — fail
/// OPEN to a fresh 200, never a spurious 304).
///
/// Accepts `YYYY-MM-DDTHH:MM:SS` with an OPTIONAL fractional-seconds part (ignored — truncated to
/// whole seconds) and an OPTIONAL timezone: `Z`, `+HH:MM`, or `-HH:MM` (a bare, timezone-less form
/// is read as UTC, which is what we emit). The calendar is validated for real (month range, the
/// month's true length incl. leap years); anything malformed ⇒ `None`.
pub fn from_xsd_datetime(lexical: &str) -> Option<SystemTime> {
    let v = lexical.trim();
    if !v.is_ascii() {
        return None;
    }
    let b = v.as_bytes();
    // The fixed-width date+time head is exactly `YYYY-MM-DDTHH:MM:SS` = 19 bytes.
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let year = digits(b, 0, 4)?;
    let month = digits(b, 5, 2)? as u32;
    let day = digits(b, 8, 2)? as u32;
    let hh = digits(b, 11, 2)?;
    let mm = digits(b, 14, 2)?;
    let ss = digits(b, 17, 2)?;
    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    // `hh == 24` is only valid as the "end of day" midnight in XSD; we do not emit it and a real
    // instant never needs it, so reject anything past a normal clock reading. Leap-second `:60` is
    // likewise rejected (we never emit it).
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }

    // Whatever follows the seconds: an optional `.fraction`, then an optional timezone. The fraction
    // is discarded (whole-second granularity); the timezone offset is applied to reach UTC.
    let mut rest = &v[19..];
    if let Some(after_dot) = rest.strip_prefix('.') {
        // A run of ASCII digits is the fractional part; it must be non-empty and is ignored.
        let frac_len = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if frac_len == 0 {
            return None;
        }
        rest = &after_dot[frac_len..];
    }
    let offset_secs = parse_timezone(rest)?;

    // Days since the Unix epoch for this civil date, then the wall-clock seconds, then remove the
    // offset to land on UTC.
    let days = days_from_civil(year, month, day);
    let secs = days * SECS_PER_DAY + hh * 3_600 + mm * 60 + ss - offset_secs;
    if secs < 0 {
        return None; // pre-epoch — not representable as a `Duration` from `UNIX_EPOCH`.
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// Parse the trailing timezone designator to an offset in seconds EAST of UTC (so UTC = local −
/// offset): empty or `Z` ⇒ 0; `+HH:MM` / `-HH:MM` ⇒ the signed offset. Anything else ⇒ `None`.
///
/// The magnitude is bounded to the XSD `dateTime` timezone range of `±14:00` (XSD Part 2 §3.2.7):
/// an out-of-range offset like `+23:59` is a malformed value and REJECTED (⇒ the caller fails open
/// to a fresh 200), not silently accepted as a wrong instant.
fn parse_timezone(tz: &str) -> Option<i64> {
    if tz.is_empty() || tz == "Z" {
        return Some(0);
    }
    let b = tz.as_bytes();
    // `±HH:MM` is exactly 6 bytes.
    if b.len() != 6 || b[3] != b':' {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let oh = digits(b, 1, 2)?;
    let om = digits(b, 4, 2)?;
    // Field ranges AND the XSD ±14:00 magnitude cap (so `+14:00` is the largest valid offset;
    // `+14:01` and anything beyond is malformed ⇒ None).
    if om > 59 || oh * 60 + om > 14 * 60 {
        return None;
    }
    Some(sign * (oh * 3_600 + om * 60))
}

/// Parse exactly `n` ASCII digits at `b[at..at+n]` as a non-negative integer, or `None`.
fn digits(b: &[u8], at: usize, n: usize) -> Option<i64> {
    if at + n > b.len() {
        return None;
    }
    let mut acc: i64 = 0;
    for &d in &b[at..at + n] {
        if !d.is_ascii_digit() {
            return None;
        }
        acc = acc * 10 + i64::from(d - b'0');
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known instant: 2026-07-05T12:34:56Z is 1_783_254_896 seconds since the epoch.
    fn known() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_783_254_896)
    }

    #[test]
    fn formats_canonical_utc() {
        assert_eq!(to_xsd_datetime(known()).unwrap(), "2026-07-05T12:34:56Z");
    }

    #[test]
    fn epoch_formats_as_1970() {
        assert_eq!(to_xsd_datetime(UNIX_EPOCH).unwrap(), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn round_trips_a_range_of_instants() {
        // Cover leap years, month boundaries, and a leap day across a wide span.
        for secs in [
            0i64,
            1,
            59,
            86_399,
            86_400,
            951_782_400, // 2000-02-29 (leap day) 00:00:00Z
            1_078_012_800,
            1_783_254_896,
            4_102_444_799, // 2099-12-31T23:59:59Z
        ] {
            let t = UNIX_EPOCH + Duration::from_secs(secs as u64);
            let lex = to_xsd_datetime(t).unwrap();
            let back = from_xsd_datetime(&lex).unwrap();
            assert_eq!(back, t, "round-trip failed for {secs} ({lex})");
        }
    }

    #[test]
    fn parses_the_canonical_form() {
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56Z"), Some(known()));
    }

    #[test]
    fn tolerates_fractional_seconds_truncating_to_whole() {
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56.789Z"), Some(known()));
    }

    #[test]
    fn tolerates_a_bare_no_timezone_form_as_utc() {
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56"), Some(known()));
    }

    #[test]
    fn applies_a_numeric_offset_to_reach_utc() {
        // 14:34:56+02:00 is the SAME instant as 12:34:56Z.
        assert_eq!(
            from_xsd_datetime("2026-07-05T14:34:56+02:00"),
            Some(known())
        );
        // 10:34:56-02:00 is likewise 12:34:56Z.
        assert_eq!(
            from_xsd_datetime("2026-07-05T10:34:56-02:00"),
            Some(known())
        );
    }

    #[test]
    fn rejects_malformed_and_out_of_range_forms_as_none() {
        for bad in [
            "",
            "not-a-date",
            "2026-07-05",               // date only, no time head
            "2026-13-05T00:00:00Z",     // month 13
            "2026-02-29T00:00:00Z",     // 2026 is not a leap year
            "2026-07-05T24:00:00Z",     // hour 24
            "2026-07-05T12:60:00Z",     // minute 60
            "2026-07-05T12:34:60Z",     // leap-second style :60
            "2026-07-05T12:34:56.Z",    // empty fraction
            "2026-07-05T12:34:56+2:00", // non-fixed-width offset
            "2026-07-05 12:34:56Z",     // space instead of 'T'
        ] {
            assert_eq!(from_xsd_datetime(bad), None, "expected None for {bad:?}");
        }
    }

    #[test]
    fn rejects_timezone_offsets_beyond_the_xsd_14h_cap() {
        // `+14:00` is the largest valid XSD offset ⇒ accepted; anything beyond ⇒ None (fail open).
        assert!(from_xsd_datetime("2026-07-05T12:34:56+14:00").is_some());
        assert!(from_xsd_datetime("2026-07-05T12:34:56-14:00").is_some());
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56+14:01"), None);
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56-14:01"), None);
        assert_eq!(from_xsd_datetime("2026-07-05T12:34:56+23:59"), None);
    }

    #[test]
    fn pre_epoch_is_none() {
        // 1969-12-31T23:59:59Z is before the epoch — not representable, must be None (never a
        // wrong 304 basis).
        assert_eq!(from_xsd_datetime("1969-12-31T23:59:59Z"), None);
        assert!(to_xsd_datetime(UNIX_EPOCH - Duration::from_secs(1)).is_none());
    }
}

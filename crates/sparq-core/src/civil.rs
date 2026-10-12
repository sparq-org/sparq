//! Proleptic-Gregorian civil dates and UTC `xsd:dateTime` formatting, dependency-free.
//!
//! The workspace has no `chrono`/`time`, so crates that stamp or compare dates use these
//! instead of keeping their own copy. The conversions are Howard Hinnant's algorithms
//! (the same calendar [`crate::temporal::parse_civil_date`] uses), so a formatted lexical
//! parses back to the same instant.

/// Days since 1970-01-01 of a proleptic-Gregorian civil date (`month` 1–12, `day` 1–31).
/// The inverse of [`civil_from_days`]. Returned as `i128`, so every `i64` year is exact
/// (the day count of a year near `i64::MAX` does not fit an `i64`).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i128 {
    let y = i128::from(year) - i128::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i128::from(if month > 2 { month - 3 } else { month + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i128::from(day) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Epoch seconds (UTC) → an `xsd:dateTime` lexical `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let tod = epoch_secs.rem_euclid(86_400); // 0..86400, always non-negative
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Days-from-epoch (1970-01-01 = 0) → `(year, month, day)`. The inverse of
/// [`days_from_civil`]; from Howard Hinnant's `chrono`-compatible algorithms. Exact for
/// every `i64` (the arithmetic runs in `i128`; the year always fits an `i64`).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = i128::from(z) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    ((if m <= 2 { y + 1 } else { y }) as i64, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temporal::Timeline;

    #[test]
    fn epoch_zero_is_unix_epoch() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_instants() {
        // 2001-09-09T01:46:40Z is epoch 1_000_000_000.
        assert_eq!(format_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap-day instant.
        assert_eq!(format_utc(1_582_934_400), "2020-02-29T00:00:00Z");
    }

    #[test]
    fn round_trips_through_core_parser() {
        // The formatted lexical must parse back (via sparq-core's xsd:dateTime
        // parser) to exactly the same instant — proving the two are inverse.
        for &secs in &[0, 1, 86_400, 1_000_000_000, 1_700_000_000, 1_582_934_400] {
            let lex = format_utc(secs);
            let tl = Timeline::parse_datetime(&lex).expect("formatted lexical must parse");
            assert_eq!(tl.instant() as i64, secs, "round-trip mismatch for {lex}");
        }
    }

    /// Epoch seconds at 2100-03-01T00:00:00Z — the first date where `doe / 36524 = 1`
    /// in the Hinnant calendar algorithm (doe = 36524 relative to the start of the
    /// 400-year Gregorian era that began ~2000-03-01). In the formula:
    ///   `yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365`
    /// the `+ doe/36524` term changes from 0 to 1 at this date; a mutation that
    /// replaces it with `- doe/36524` would produce `yoe=99` (year 2099) instead of
    /// `yoe=100` (year 2100), yielding the nonexistent date "2100-02-29". This test
    /// pins the exact output to catch that arithmetic mutation. [OPUS-4.8] sq-qcnn.20
    #[test]
    fn year_2100_first_century_correction_date() {
        // 2100-03-01 00:00:00 UTC: epoch secs = 47541 * 86400 = 4_107_542_400.
        // 2100 is NOT a leap year (divisible by 100 but not by 400), so
        // 2100-02-29 does not exist — formatting to that date is the mutation's tell.
        assert_eq!(format_utc(4_107_542_400), "2100-03-01T00:00:00Z");
        // One day earlier is still February (year 2100, not 2099).
        assert_eq!(format_utc(4_107_456_000), "2100-02-28T00:00:00Z");
    }

    /// In `civil_from_days`, the month index `mp` is computed as
    ///   `mp = (5 * doy + 2) / 153`
    /// The `+ 2` offset is required because the integer-division boundary for
    /// `153 * k` falls two short of the correct month-start day-of-year in the
    /// Hinnant March-based calendar. There are exactly two `doy` values where
    /// dropping the `+ 2` (mutation: `5 * doy + 2 → 5 * doy`) shifts `mp` by 1,
    /// producing the wrong month:
    ///
    /// * **doy = 122** (July 1): `(5×122 + 2)/153 = 4` (July) vs `610/153 = 3` (June).
    ///   July 1 at 2001 = epoch 993_945_600. Mutation gives "2001-06-31" (invalid).
    /// * **doy = 275** (December 1): `(5×275 + 2)/153 = 9` (December) vs `8` (November).
    ///   December 1 at 2001 = epoch 1_007_164_800. Mutation gives "2001-11-31" (invalid).
    ///
    /// [SONNET-4.6] sq-qcnn.39
    #[test]
    fn month_boundary_july_and_december() {
        // doy=122: 2001-07-01T00:00:00Z, epoch = 11504 * 86_400 = 993_945_600.
        assert_eq!(format_utc(993_945_600), "2001-07-01T00:00:00Z");
        // doy=275: 2001-12-01T00:00:00Z, epoch = 11657 * 86_400 = 1_007_164_800.
        assert_eq!(format_utc(1_007_164_800), "2001-12-01T00:00:00Z");
    }

    /// Round-trip the doy-boundary instants through the sparq-core xsd:dateTime
    /// parser to confirm they are valid calendar dates (not "June 31" or "November 31"
    /// artefacts produced by the mutated algorithm). [SONNET-4.6] sq-qcnn.39
    #[test]
    fn month_boundary_instants_round_trip() {
        for &secs in &[993_945_600_i64, 1_007_164_800] {
            let lex = format_utc(secs);
            let tl = Timeline::parse_datetime(&lex)
                .unwrap_or_else(|| panic!("month-boundary lexical must parse: {lex}"));
            assert_eq!(tl.instant() as i64, secs, "round-trip mismatch for {}", lex);
        }
    }

    #[test]
    fn days_and_civil_dates_are_inverse() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2021, 1, 1), 18_628);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // Every day across several 400-year eras, including pre-epoch and year 0.
        for days in (-800_000..800_000).step_by(7) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), i128::from(days), "{y}-{m}-{d}");
        }
        // The whole i64 domain is exact: no overflow at either end.
        for days in [i64::MIN, i64::MIN + 1, i64::MAX - 1, i64::MAX] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), i128::from(days), "{y}-{m}-{d}");
        }
        for year in [i64::MIN, i64::MAX] {
            let days = days_from_civil(year, 6, 15);
            assert_eq!(days.signum(), year.signum() as i128);
        }
    }
}

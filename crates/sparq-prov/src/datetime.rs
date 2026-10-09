//! `xsd:dateTime` formatting from epoch seconds, dependency-free.
//!
//! The workspace has no `chrono`/`time`, and pulling one in for two lines would
//! bloat this opt-in crate, so we format the canonical UTC lexical ourselves.
//! The civil-date conversion is Howard Hinnant's algorithm (the same one
//! `sparq-core::temporal::parse_civil_date` inverts), so a formatted lexical
//! parses back to the same instant. [OPUS-4.8] sq-ntcg.

/// Epoch seconds (UTC) → an `xsd:dateTime` lexical `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let tod = epoch_secs.rem_euclid(86_400); // 0..86400, always non-negative
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Days-from-epoch (1970-01-01 = 0) → `(year, month, day)`. The inverse of
/// `days_from_civil`; from Howard Hinnant's `chrono`-compatible algorithms.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::temporal::Timeline;

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
}

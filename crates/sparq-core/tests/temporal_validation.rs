// XSD 1.0 lexical/value boundaries shared by cached and scalar evaluation.
use sparq_core::temporal::{Temporal, Timeline, parse_civil_date, parse_tz};

#[test]
fn invalid_datetimes_do_not_enter_the_comparison_cache() {
    for text in [
        "2023-02-29T01:02:03Z",
        "1900-02-29T01:02:03Z",
        "2024-04-31T01:02:03Z",
        "2024-13-01T01:02:03Z",
        "2024-00-01T01:02:03Z",
        "2024-01-00T01:02:03Z",
        "2024-1-01T01:02:03Z",
        "2024-01-01T1:02:03Z",
        "2024-01-01T25:02:03Z",
        "2024-01-01T24:00:01Z",
        "2024-01-01T24:00:00.0000000000000000000000000000000000000001Z",
        "2024-01-01T01:60:03Z",
        "2024-01-01T01:02:60Z",
        "2024-01-01T01:02:NaNZ",
        "2024-01-01T01:02:03.Z",
        "2024-01-01T01:02:03Ztail",
        "2024-01-01T01:02:03+14:01",
        "2024-01-01T01:02:03+15:00",
        "2024-01-01T01:02:03+00:60",
        "-0000-01-01T01:02:03Z",
        "02024-01-01T01:02:03Z",
        "+2024-01-01T01:02:03Z",
        "9223372036854775807-01-01T01:02:03Z",
        "-9223372036854775807-01-01T01:02:03Z",
        "2024-é-01T01:02:03Z",
        "2024-01-01T01:02:03+é:00",
        "🕑2024-01-01T00:00:00Z",
    ] {
        assert!(Timeline::parse_datetime(text).is_none(), "{text}");
        assert!(
            Temporal::of_lit(text, "http://www.w3.org/2001/XMLSchema#dateTime").is_none(),
            "{text}"
        );
    }
}

#[test]
fn valid_boundaries_keep_their_timeline_values() {
    for text in [
        "2000-02-29T01:02:03Z",
        "2024-02-29T01:02:03+14:00",
        "2024-02-29T01:02:03-14:00",
        "2024-02-29T01:02:03",
        "-0001-01-01T00:00:00Z",
        "0000-01-01T01:02:03Z",
    ] {
        assert!(Timeline::parse_datetime(text).is_some(), "{text}");
    }
    let midnight = Timeline::parse_datetime("2024-02-29T24:00:00.000Z").unwrap();
    let next_day = Timeline::parse_datetime("2024-03-01T00:00:00Z").unwrap();
    assert_eq!(midnight.instant(), next_day.instant());
    // Raw lexicals are not XML-preprocessed: boundary padding is rejected
    // (single-char `Temporal::of_lit` pads are covered in `temporal_lexical.rs`).
    let unpadded = "2024-03-01T00:00:00Z";
    assert!(Temporal::of_lit(unpadded, "http://www.w3.org/2001/XMLSchema#dateTime").is_some());
    for pad in [" ", "\t", "\r", "\n"] {
        for padded in [format!("{pad}{unpadded}"), format!("{unpadded}{pad}")] {
            assert!(Timeline::parse_datetime(&padded).is_none(), "{padded:?}");
        }
    }
    let padded = format!(" \t{unpadded}\r\n");
    assert!(Timeline::parse_datetime(&padded).is_none());
    assert!(Temporal::of_lit(&padded, "http://www.w3.org/2001/XMLSchema#dateTime").is_none());
    // Capacity control only: the f64 cache collapses these distinct XSD
    // instants. This is not normative value equality or exact fractional support.
    assert_eq!(
        Timeline::parse_datetime("2024-03-01T00:00:59.999999999999999999Z")
            .unwrap()
            .instant(),
        Timeline::parse_datetime("2024-03-01T00:01:00Z")
            .unwrap()
            .instant()
    );
}

#[test]
fn public_calendar_parsers_reject_malformed_and_overflowing_inputs() {
    for text in [
        "",
        "é",
        "x05:30",
        "+5:30",
        "+15:00",
        "-14:01",
        "+00:60",
        "+999999999999999999:00",
    ] {
        assert!(parse_tz(text).is_none(), "{text}");
    }
    for text in [
        "2023-02-29",
        "1900-02-29",
        "2024-04-31",
        "-0000-01-01",
        "0001-02-29",
        "9223372036854775807-01-01",
        "-9223372036854775807-01-01",
    ] {
        assert!(parse_civil_date(text).is_none(), "{text}");
        assert!(Timeline::parse_date(text).is_none(), "{text}");
    }
    assert_eq!(
        parse_civil_date("2000-03-01").unwrap() - parse_civil_date("2000-02-29").unwrap(),
        1
    );
}

#[test]
fn civil_day_numbers_stay_contiguous_across_the_wide_arithmetic_boundary() {
    let date = |y: i64, m: u32, d: u32| {
        let sign = if y < 0 { "-" } else { "" };
        sparq_core::temporal::parse_civil_date(&format!("{sign}{:04}-{m:02}-{d:02}", y.unsigned_abs())).unwrap()
    };
    assert_eq!(date(1970, 1, 1), 0);
    let edge = 1_i64 << 40;
    for y in [1, 1969, 1999, 2000, 2023, 2024, edge - 3, edge - 2, edge - 1, edge, edge + 1, -(edge + 1), -edge, -(edge - 1), -(edge - 2), -401, -2] {
        // The day count applies the proleptic leap rule to the signed year itself.
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        assert_eq!(date(y + 1, 1, 1) - date(y, 1, 1), if leap { 366 } else { 365 }, "year {y}");
        assert_eq!(date(y, 3, 1) - date(y, 2, 28), if leap { 2 } else { 1 }, "year {y}");
    }
}

/// XSD 1.1 has a year zero (1 BCE), a leap year, and numbers earlier years
/// astronomically: `-0001-12-31` is the day before `0000-01-01`.
#[test]
fn year_zero_is_one_bce() {
    let day = |text: &str| parse_civil_date(text).unwrap_or_else(|| panic!("{text}"));
    assert_eq!(day("0000-01-01"), day("-0001-12-31") + 1);
    assert_eq!(day("0001-01-01"), day("0000-12-31") + 1);
    assert_eq!(day("0000-03-01"), day("0000-02-29") + 1);
    assert_eq!(day("0001-01-01") - day("0000-01-01"), 366);
    assert_eq!(
        Timeline::parse_date("0000-01-01").unwrap().instant(),
        Timeline::parse_datetime("0000-01-01T00:00:00Z").unwrap().instant()
    );
    let midnight = Timeline::parse_datetime("-0001-12-31T24:00:00Z").unwrap();
    let new_year = Timeline::parse_datetime("0000-01-01T00:00:00Z").unwrap();
    assert_eq!(midnight.instant(), new_year.instant());
}

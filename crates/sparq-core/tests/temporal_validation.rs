// [GPT-6] XSD 1.0 lexical/value boundaries shared by cached and scalar evaluation.
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
        "0000-01-01T01:02:03Z",
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
    ] {
        assert!(Timeline::parse_datetime(text).is_some(), "{text}");
    }
    let midnight = Timeline::parse_datetime("2024-02-29T24:00:00.000Z").unwrap();
    let next_day = Timeline::parse_datetime("2024-03-01T00:00:00Z").unwrap();
    assert_eq!(midnight.instant(), next_day.instant());
    assert_eq!(
        Timeline::parse_datetime(" \t2024-03-01T00:00:00Z\r\n")
            .unwrap()
            .instant(),
        next_day.instant()
    );
    // [GPT-6] Capacity control only: the f64 cache collapses these distinct XSD
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
        "0000-01-01",
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

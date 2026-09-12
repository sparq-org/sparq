// [GPT-6] Exact comparison expectations independent of the approximate cache.
use sparq_core::temporal::{ExactTemporal, ExactTimeline};
use std::cmp::Ordering::{Equal, Greater, Less};

fn time(value: &str) -> ExactTimeline<'_> {
    ExactTimeline::parse_datetime(value).unwrap()
}

#[test]
fn fractional_digits_and_large_epochs_remain_distinct() {
    for (left, right) in [
        ("2024-01-01T00:00:00Z", "2024-01-01T00:00:00.000000001Z"),
        (
            "2024-01-01T00:00:59.999999999999999999Z",
            "2024-01-01T00:01:00Z",
        ),
        (
            "9999-12-31T23:59:59.0000000000000000000000000001Z",
            "9999-12-31T23:59:59.0000000000000000000000000002Z",
        ),
        ("1000000000-01-01T00:00:00Z", "1000000000-01-01T00:00:01Z"),
        ("-1000000000-01-01T00:00:00Z", "-1000000000-01-01T00:00:01Z"),
    ] {
        assert_eq!(time(left).compare(time(right)), Some(Less), "{left}");
        assert_eq!(time(right).compare(time(left)), Some(Greater), "{left}");
        assert_eq!(time(left).compare_total(time(right)), Less);
    }
}

#[test]
fn lexical_fraction_normalization_and_offsets_preserve_value_equality() {
    for (left, right) in [
        ("2024-01-01T00:00:00.1000Z", "2024-01-01T00:00:00.1Z"),
        ("2024-01-01T00:00:00.0000Z", "2024-01-01T00:00:00Z"),
        (
            "2024-01-01T00:00:00.000000001+14:00",
            "2023-12-31T10:00:00.000000001Z",
        ),
        ("2024-01-01T24:00:00.000Z", "2024-01-02T00:00:00Z"),
        ("0001-01-01T00:00:00Z", "0001-01-01T14:00:00+14:00"),
    ] {
        assert_eq!(time(left).compare(time(right)), Some(Equal), "{left}");
        assert_eq!(time(left).compare_total(time(right)), Equal);
    }
}

#[test]
fn fractional_uncertainty_boundaries_are_exact_and_total_order_is_transitive() {
    let floating = time("2024-01-01T00:00:00");
    let boundary = time("2024-01-01T14:00:00Z");
    let outside = time("2024-01-01T14:00:00.00000000000000000001Z");
    assert_eq!(floating.compare(boundary), None);
    assert_eq!(floating.compare(outside), Some(Less));
    assert_eq!(outside.compare(floating), Some(Greater));
    let values = [floating, boundary, outside, time("2024-01-01T00:00:00Z")];
    for a in values {
        for b in values {
            for c in values {
                if a.compare_total(b) != Greater && b.compare_total(c) != Greater {
                    assert_ne!(a.compare_total(c), Greater);
                }
            }
        }
    }
}

#[test]
fn datatype_family_and_stamp_timezone_are_checked() {
    let date =
        ExactTemporal::of_lit("2024-01-01Z", "http://www.w3.org/2001/XMLSchema#date").unwrap();
    let datetime = ExactTemporal::of_lit(
        "2024-01-01T00:00:00Z",
        "http://www.w3.org/2001/XMLSchema#dateTime",
    )
    .unwrap();
    assert_eq!(date.compare(datetime), None);
    assert_eq!(date.compare_total(datetime), Greater);
    assert!(
        ExactTemporal::of_lit(
            "2024-01-01T00:00:00",
            "http://www.w3.org/2001/XMLSchema#dateTimeStamp"
        )
        .is_none()
    );
    assert!(ExactTimeline::parse_datetime("2024-01-01T24:00:00.000000000000000001Z").is_none());
    assert!(ExactTimeline::parse_datetime("9223372036854775807-01-01T00:00:00Z").is_none());
}

#[test]
fn explicit_year_capacity_preserves_ordinary_lexical_errors() {
    use sparq_core::temporal::year_within_capacity;
    let datatype = "http://www.w3.org/2001/XMLSchema#dateTime";
    for value in [
        "0001-01-01T00:00:00Z",
        "1000000000-01-01T00:00:00.000000001Z",
    ] {
        assert!(
            year_within_capacity(value, datatype, 1, 1_000_000_000),
            "{value}"
        );
    }
    // XSD 1.1 year zero is valid-shaped but outside this positive-year lane.
    for value in [
        "0000-01-01T00:00:00Z",
        "-0001-01-01T00:00:00Z",
        "1000000001-01-01T00:00:00Z",
        "99999999999999999999999-01-01T00:00:00Z",
    ] {
        assert!(
            !year_within_capacity(value, datatype, 1, 1_000_000_000),
            "{value}"
        );
    }
    for value in [
        "not a date",
        "01234-01-01T00:00:00Z",
    ] {
        assert!(
            year_within_capacity(value, datatype, 1, 1_000_000_000),
            "{value}"
        );
    }
    assert!(year_within_capacity(
        "-0001-01-01T00:00:00Z",
        datatype,
        -10,
        10
    ));
    assert!(!year_within_capacity(
        "0001-01-01T00:00:00Z",
        datatype,
        10,
        1
    ));
}

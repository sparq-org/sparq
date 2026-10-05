// [GPT-6] Raw RDF temporal lexicals are validated without construction normalization.
use sparq_core::temporal::{
    ExactTemporal, ExactTimeline, Temporal, Timeline, year_within_capacity,
};

const DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const STAMP: &str = "http://www.w3.org/2001/XMLSchema#dateTimeStamp";
const DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

#[test]
fn raw_temporal_lexicals_do_not_apply_xml_preprocessing() {
    for (value, datatype) in [
        ("2024-02-29T12:34:56.123456789Z", DATETIME),
        ("2024-02-29T12:34:56.123456789Z", STAMP),
        ("2024-02-29Z", DATE),
    ] {
        assert!(Temporal::of_lit(value, datatype).is_some());
        assert!(ExactTemporal::of_lit(value, datatype).is_some());
        for pad in [" ", "\t", "\r", "\n", "\u{a0}"] {
            for padded in [format!("{pad}{value}"), format!("{value}{pad}")] {
                assert!(
                    Temporal::of_lit(&padded, datatype).is_none(),
                    "approx {padded:?}"
                );
                assert!(
                    ExactTemporal::of_lit(&padded, datatype).is_none(),
                    "exact {padded:?}"
                );
            }
        }
    }
    assert!(Timeline::parse_date(" 2024-02-29Z ").is_none());
    assert!(ExactTimeline::parse_datetime(" 2024-02-29T00:00:00Z ").is_none());
}

#[test]
fn every_temporal_representation_checks_the_stamp_timezone_facet() {
    let value = "2024-02-29T12:34:56.123456789";
    assert!(Temporal::of_lit(value, DATETIME).is_some());
    assert!(ExactTemporal::of_lit(value, DATETIME).is_some());
    assert!(Temporal::of_lit(value, STAMP).is_none());
    assert!(ExactTemporal::of_lit(value, STAMP).is_none());
}

#[test]
fn malformed_padded_years_do_not_masquerade_as_capacity_failures() {
    for datatype in [DATE, DATETIME, STAMP] {
        for value in [
            " 1000000001-01-01T00:00:00Z",
            "1000000001-01-01T00:00:00Z\n",
        ] {
            assert!(year_within_capacity(value, datatype, 1, 1_000_000_000));
        }
        assert!(!year_within_capacity(
            "1000000001-01-01T00:00:00Z",
            datatype,
            1,
            1_000_000_000
        ));
        assert!(!year_within_capacity(
            "0000-01-01T00:00:00Z",
            datatype,
            1,
            1_000_000_000
        ));
    }
}

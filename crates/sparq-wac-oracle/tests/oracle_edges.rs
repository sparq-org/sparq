// sq-bif.38 — integration-level edge contracts for the dev-only oracle.

use sparq_wac_oracle::escalation_corpus::{canonicalize_child, is_acl_shaped, naive_acl_guard};
use sparq_wac_oracle::{build_store, fixtures, run_vectors};

#[test]
fn empty_vector_report_is_a_pass() {
    let store = build_store(&fixtures()[0]).expect("WAC fixture builds");
    let report = run_vectors(&store, &[]);

    assert_eq!(report.total, 0);
    assert!(report.failures.is_empty());
    assert!(report.passed());
}

#[test]
fn acl_shape_detection_pins_boundary_names() {
    let cases = [
        ("", false),
        ("/", false),
        ("FOO.ACL", true),
        ("dir/x.acl/", true),
        (".acl", true),
    ];

    for (name, expected) in cases {
        assert_eq!(is_acl_shaped(name), expected, "name: {name:?}");
    }
}

#[test]
fn child_canonicalization_pins_dot_and_trailing_character_rules() {
    let cases = [
        ("foo.acl.", "foo.acl"),
        ("foo.acl ", "foo.acl"),
        ("foo\u{FF0E}acl", "foo.acl"),
        ("foo%2Eacl", "foo.acl"),
        (".foo", ".foo"),
    ];

    for (raw, expected) in cases {
        assert_eq!(canonicalize_child(raw), expected, "raw child: {raw:?}");
    }
}

#[test]
fn canonicalization_exposes_encoded_acl_shape_missed_by_naive_guard() {
    let raw = "foo%2Eacl";

    assert!(!naive_acl_guard(raw));
    assert!(is_acl_shaped(&canonicalize_child(raw)));
}

//! Edge-contract integration tests for deterministic generation and fail-closed
//! window reporting. sq-bif.32

use sparq_metamorph::{generate_case, run_window, InProcessSparq, WindowReport};

#[test]
fn generated_case_is_reproducible_for_seed_42() {
    let first = generate_case(42);
    let second = generate_case(42);

    assert_eq!(first, second);
    assert_eq!(first.seed, 42);
    assert!(!first.data_ntriples.is_empty());
    assert!(first.pattern.contains("OPTIONAL"));
}

#[test]
fn window_report_truth_table_fails_closed_and_summary_carries_counts() {
    let empty = WindowReport {
        checked: 0,
        pass: 0,
        ..WindowReport::default()
    };
    assert!(!empty.all_pass());

    let green = WindowReport {
        checked: 2,
        pass: 2,
        ..WindowReport::default()
    };
    assert!(green.all_pass());

    let red = WindowReport {
        checked: 2,
        pass: 1,
        ..WindowReport::default()
    };
    assert!(!red.all_pass());

    let summary_report = WindowReport {
        checked: 7,
        pass: 5,
        ..WindowReport::default()
    };
    let summary = summary_report.summary_line();
    assert!(summary.contains("metamorph seeds"));
    assert!(summary.contains("checked=7"));
    assert!(summary.contains("pass=5"));
}

#[test]
fn run_window_reports_the_requested_seed_range() {
    let mut sink = Vec::new();
    let report = run_window(0, 2, false, &mut sink).expect("two-seed window runs");

    assert_eq!(report.start, 0);
    assert_eq!(report.checked, 2);
}

#[test]
fn in_process_engine_rejects_invalid_ntriples() {
    let result = InProcessSparq::from_ntriples("engine-under-test", "this is not n-triples");

    assert!(result.is_err());
}

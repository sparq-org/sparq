//! (sq-fggn) The human-readable / identifier contracts that callers
//! (sparq-server's HTTP layer, logs, metrics) depend on but that the behavioural
//! writer/ring tests never invoke: `WriteError`'s `Display`/`Error`, and `PodId`'s
//! `as_str` round-trip plus its `Display`.

use sparq_serve::{PodId, WriteError};

/// Each `WriteError` variant renders a distinct, non-empty message, and the
/// retryable `Unavailable` is clearly marked retryable (the 503 contract). The
/// `Rejected`/`Unavailable` payloads are surfaced in the message.
#[test]
fn write_error_display_is_distinct_and_carries_payload() {
    let rejected = format!("{}", WriteError::Rejected("parse failed at line 2".into()));
    let shutdown = format!("{}", WriteError::Shutdown);
    let unavailable = format!("{}", WriteError::Unavailable("ENOSPC".into()));

    assert!(
        rejected.contains("parse failed at line 2"),
        "got: {rejected}"
    );
    assert!(!shutdown.is_empty());
    assert!(unavailable.contains("ENOSPC"), "got: {unavailable}");
    assert!(
        unavailable.to_lowercase().contains("retry"),
        "Unavailable must signal retryability: {unavailable}"
    );

    // All three render differently.
    assert_ne!(rejected, shutdown);
    assert_ne!(rejected, unavailable);
    assert_ne!(shutdown, unavailable);

    // It is a real std::error::Error with no nested source.
    let err: &dyn std::error::Error = &WriteError::Shutdown;
    assert!(err.source().is_none());
    assert!(!err.to_string().is_empty());
}

/// `PodId` round-trips its identifier through `as_str`, and `Display` prints that
/// same identifier (used in epoch logging / debug output).
#[test]
fn pod_id_as_str_and_display_round_trip() {
    let id = PodId::from("http://pod/alice");
    assert_eq!(id.as_str(), "http://pod/alice");
    assert_eq!(format!("{id}"), "http://pod/alice");

    // The other constructors agree.
    let from_string = PodId::from(String::from("http://pod/bob"));
    assert_eq!(from_string.as_str(), "http://pod/bob");
    let via_new = PodId::new("http://pod/carol");
    assert_eq!(format!("{via_new}"), via_new.as_str());
}

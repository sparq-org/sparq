// [GPT-6] Reject unapproved bytes before the SDK reads ELF memory declarations.
use sparq_proved_evaluator::{AcceptedGuest, embedded_artifact, embedded_pin, method_id};

#[test]
fn reviewed_artifact_loads_with_both_independent_pins() {
    let guest = AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &embedded_pin()).unwrap();
    assert_eq!(guest.image_id(), method_id());
}

#[test]
fn malformed_unapproved_artifact_is_rejected_before_elf_parsing() {
    let error = AcceptedGuest::from_artifact(vec![0; 64], &embedded_pin()).unwrap_err();
    assert_eq!(error.0, "independent guest artifact digest rejected");
}

#[test]
fn oversized_artifact_is_rejected_before_hashing_or_parsing() {
    let error =
        AcceptedGuest::from_artifact(vec![0; 32 * 1024 * 1024 + 1], &embedded_pin()).unwrap_err();
    assert_eq!(error.0, "guest artifact capacity rejected");
}

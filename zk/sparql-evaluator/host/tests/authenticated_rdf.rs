// [OPUS-5.5] zkp-14.5: native V5 host API gates over the published W3C vector.
// Rust guideline compliant 2026-02-21
//! Compiled only with `--features authenticated-rdf` and run by default with it.
//!
//! These gates create no proof and use no executor or `r0vm`. The only receipt
//! they build is a FAKE one, which must be rejected before any nonce use. Direct
//! guest execution is in `actual_authenticated_rdf.rs` and genuine receipts are
//! in `authenticated_rdf_genuine.rs`; both are ignored drivers. Every nonce store
//! here is an in-memory test double, never a production store.
//!
//! They still link the embedded V5 artifact, so they need the feature's guest
//! build and its not-yet-generated lock; at the current checkpoint they are unrun.
#![cfg(feature = "authenticated-rdf")]

#[path = "support/authenticated_rdf.rs"]
mod fixture;

use fixture::{CountingNonces, Expect};
use sparq_proved_evaluator::authenticated_rdf::{prove_with_artifact, verify_with_artifact};
use sparq_proved_evaluator::{
    AcceptedGuest, ArtifactPin, Error, embedded_artifact, embedded_authrdf_artifact,
    embedded_authrdf_pin, embedded_pin,
};
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, Provenance};
use sparq_proved_evaluator_model::{DatasetAuthority, v3};
use std::path::Path;

const NONCE: [u8; 32] = [0x3c; 32];

fn auth_guest() -> AcceptedGuest {
    AcceptedGuest::from_artifact(embedded_authrdf_artifact().to_vec(), &embedded_authrdf_pin())
        .expect("embedded V5 guest matches its own pin")
}

fn agreed() -> DatasetAuthority {
    DatasetAuthority::VerifierAgreed {
        commitment: fixture::anchor(fixture::SALT),
    }
}

#[test]
fn fixture_is_the_published_vector_and_the_native_oracle_matches_expectations() {
    fixture::check_published_vector();
    let cases = [
        (fixture::SELECT_BAG, Expect::Bag),
        (fixture::ASK_ISSUER, Expect::Ask(true)),
        (fixture::ASK_OTHER_ISSUER, Expect::Ask(false)),
        (fixture::CONSTRUCT, Expect::Graph),
    ];
    for (authority, provenance) in [
        (agreed(), Provenance::VerifierAgreedAuthenticated),
        (DatasetAuthority::HolderDeclared, Provenance::HolderSelectedAuthenticated),
    ] {
        for (query, expect) in cases {
            let witness = fixture::witness(query, authority.clone(), NONCE);
            let journal = auth::evaluate(&witness).expect("native oracle");
            auth::bind_journal(&journal, &witness.request).unwrap();
            assert_eq!(journal.result, expect.result(), "{query}");
            assert_eq!(journal.provenance, provenance);
            assert_eq!(journal.dataset_commitment, fixture::anchor(fixture::SALT));
        }
    }
}

#[test]
fn published_witness_encoding_stays_below_the_guest_input_bound() {
    for query in [fixture::SELECT_BAG, fixture::CONSTRUCT] {
        let witness = fixture::witness(query, agreed(), NONCE);
        let words = risc0_zkvm::serde::to_vec(&witness).expect("typed witness encoding");
        assert_eq!(words[0], auth::VERSION, "the version is the first word");
        assert!(4 * words.len() < auth::MAX_WITNESS_BYTES, "{query}");
    }
}

#[test]
fn separate_guest_pins_are_distinct_and_checked_independently() {
    let (exact, authrdf) = (embedded_pin(), embedded_authrdf_pin());
    assert_ne!(exact.sha256, authrdf.sha256, "distinct artifacts");
    assert_ne!(exact.image_id, authrdf.image_id, "distinct image IDs");
    assert_eq!(auth_guest().image_id(), authrdf.image_id);

    let digest = Error("independent guest artifact digest rejected");
    let swapped = [
        AcceptedGuest::from_artifact(embedded_artifact().to_vec(), &authrdf),
        AcceptedGuest::from_artifact(embedded_authrdf_artifact().to_vec(), &exact),
    ];
    for result in swapped {
        assert_eq!(result.unwrap_err(), digest);
    }
    let wrong_identity = ArtifactPin {
        sha256: authrdf.sha256,
        image_id: exact.image_id,
    };
    assert_eq!(
        AcceptedGuest::from_artifact(embedded_authrdf_artifact().to_vec(), &wrong_identity)
            .unwrap_err(),
        Error("independent guest artifact identity rejected")
    );
}

#[test]
fn prove_rejects_invalid_requests_and_credentials_before_any_executor() {
    // Never executed: each case must fail before the executable is used. The
    // guest enforces the same checks itself; see `actual_authenticated_rdf.rs`.
    let absent = Path::new("/nonexistent/sparq-authrdf-host-test/r0vm");
    let guest = auth_guest();
    let mut earlier = fixture::witness(fixture::ASK_ISSUER, agreed(), NONCE);
    earlier.request.version = v3::VERSION;
    let mut zero_nonce = fixture::witness(fixture::ASK_ISSUER, agreed(), NONCE);
    zero_nonce.request.nonce = [0; 32];
    let mut forged = fixture::witness(fixture::ASK_ISSUER, DatasetAuthority::HolderDeclared, NONCE);
    forged.dataset.credentials[0].signature[0] ^= 1;
    let mut unauthorized =
        fixture::witness(fixture::ASK_ISSUER, DatasetAuthority::HolderDeclared, NONCE);
    unauthorized.request.policy.authorization[0].issuer = fixture::OTHER_ISSUER.into();
    let cases = [
        (earlier, "invalid V5 proof request"),
        (zero_nonce, "invalid V5 proof request"),
        (forged, "V5 private credentials rejected"),
        (unauthorized, "V5 private credentials rejected"),
    ];
    for (witness, expected) in cases {
        assert_eq!(
            prove_with_artifact(&witness, absent, &guest).unwrap_err(),
            Error(expected)
        );
    }
}

#[test]
fn verification_failures_never_consume_the_nonce() {
    let guest = auth_guest();
    let witness = fixture::witness(fixture::ASK_ISSUER, agreed(), NONCE);
    let journal = auth::evaluate(&witness).expect("native oracle");
    let bytes: Vec<u8> = risc0_zkvm::serde::to_vec(&journal)
        .unwrap()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    // Claims the accepted image ID and a journal the relation would produce.
    let fake = fixture::fake_receipt(guest.image_id(), bytes);
    let mut earlier = witness.request.clone();
    earlier.version = v3::VERSION;
    let cases = [
        (&witness.request, CountingNonces::default(), "only succinct receipts are accepted"),
        (&witness.request, CountingNonces::broken(), "only succinct receipts are accepted"),
        (&earlier, CountingNonces::default(), "invalid V5 expected request"),
    ];
    for (expected, mut nonces, error) in cases {
        assert_eq!(
            verify_with_artifact(&fake, expected, &mut nonces, &guest).unwrap_err(),
            Error(error)
        );
        assert_eq!(nonces.calls, 0, "{error}: the nonce store was reached");
    }
}

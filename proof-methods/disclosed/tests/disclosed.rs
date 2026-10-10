//! Holder and verifier round trips and rejections for disclosed re-evaluation.

use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, PrivateCredentials};
use sparq_proved_evaluator_model::v3::CanonicalResult;
use sparq_vcq_disclosed::fixtures::{self, QUERIES};
use sparq_vcq_disclosed::{present, verify};

const SALT: [u8; 32] = [0xa5; 32];
const NONCE: [u8; 32] = [7; 32];

fn query(id: &str) -> &'static str {
    QUERIES.iter().find(|q| q.id == id).expect("query id").text
}

#[test]
fn every_query_round_trips_for_one_and_four_credentials() {
    for n in [1, 4] {
        for q in QUERIES {
            let request = fixtures::request(q.text, DatasetAuthority::HolderDeclared, NONCE);
            let (statement, proof) = present(&request, fixtures::credentials(n), SALT)
                .unwrap_or_else(|e| panic!("{} n={n}: {}", q.id, e.0));
            assert_eq!(
                verify(&request, &statement, &proof).expect("verifies"),
                statement
            );
        }
    }
}

#[test]
fn q1_is_false_and_q4_selects_only_amounts_above_the_bound() {
    let request = fixtures::request(query("Q1"), DatasetAuthority::HolderDeclared, NONCE);
    let (statement, _) = present(&request, fixtures::credentials(4), SALT).expect("presents");
    assert_eq!(statement.result, CanonicalResult::Ask(false));

    // Each credential has amounts 1250.00, 1250.00 and 1310.50: one exceeds 1300.00.
    for (n, expected) in [(1, 1), (4, 4)] {
        let request = fixtures::request(query("Q4"), DatasetAuthority::HolderDeclared, NONCE);
        let (statement, _) = present(&request, fixtures::credentials(n), SALT).expect("presents");
        let CanonicalResult::Select { rows, .. } = statement.result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), expected, "Q4 n={n}");
    }

    // Q2 keeps the duplicate amount; Q5 matches only the 2026-07 payment of the first credential.
    for (id, expected) in [("Q2", 12), ("Q5", 1)] {
        let request = fixtures::request(query(id), DatasetAuthority::HolderDeclared, NONCE);
        let (statement, _) = present(&request, fixtures::credentials(4), SALT).expect("presents");
        let CanonicalResult::Select { rows, .. } = statement.result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), expected, "{id}");
    }
}

#[test]
fn first_credential_is_the_recorded_payment_fixture() {
    let credential = fixtures::sign(fixtures::document(0));
    let hex: String = credential
        .signature
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, fixtures::PAYMENT_SIGNATURE);
    for i in 1..4 {
        let copy = fixtures::document(i);
        assert_eq!(
            copy.lines().count(),
            fixtures::PAYMENT_DOCUMENT.lines().count()
        );
        assert!(
            !copy.contains("abcdefgh"),
            "copy {i} keeps the original subject"
        );
    }
}

#[test]
fn verifier_agreed_commitment_is_the_zkvm_method_commitment() {
    let credentials = fixtures::credentials(2);
    let commitment = auth::dataset_commitment(
        &PrivateCredentials {
            credentials: credentials.clone(),
            salt: SALT,
        },
        &fixtures::policy(),
    )
    .expect("commitment");
    let request = fixtures::request(
        query("Q2"),
        DatasetAuthority::VerifierAgreed { commitment },
        NONCE,
    );
    let (statement, proof) = present(&request, credentials, SALT).expect("presents");
    assert_eq!(statement.dataset_commitment, commitment);
    verify(&request, &statement, &proof).expect("verifies");
}

#[test]
fn verifier_rejects_a_changed_result_document_signature_or_commitment() {
    let request = fixtures::request(query("Q4"), DatasetAuthority::HolderDeclared, NONCE);
    let (statement, proof) = present(&request, fixtures::credentials(1), SALT).expect("presents");

    let mut wrong = statement.clone();
    wrong.result = CanonicalResult::Ask(true);
    assert!(verify(&request, &wrong, &proof).is_err(), "changed result");

    let mut forged = proof.clone();
    forged.credentials[0].document = forged.credentials[0]
        .document
        .replace("Settled", "Returned");
    assert!(
        verify(&request, &statement, &forged).is_err(),
        "changed document"
    );

    let mut bad_sig = proof.clone();
    bad_sig.credentials[0].signature[0] ^= 1;
    assert!(
        verify(&request, &statement, &bad_sig).is_err(),
        "changed signature"
    );

    let mut other_salt = proof.clone();
    other_salt.salt = [0x5a; 32];
    assert!(
        verify(&request, &statement, &other_salt).is_err(),
        "changed salt"
    );

    let other = fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, NONCE);
    assert!(verify(&other, &statement, &proof).is_err(), "other request");
}

#[test]
fn verifier_agreed_rejects_other_credentials() {
    let agreed = auth::dataset_commitment(
        &PrivateCredentials {
            credentials: fixtures::credentials(1),
            salt: SALT,
        },
        &fixtures::policy(),
    )
    .expect("commitment");
    let request = fixtures::request(
        query("Q1"),
        DatasetAuthority::VerifierAgreed { commitment: agreed },
        NONCE,
    );
    assert!(present(&request, fixtures::credentials(2), SALT).is_err());
}

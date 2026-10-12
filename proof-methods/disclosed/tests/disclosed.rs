//! Holder and verifier round trips and rejections for disclosed re-evaluation.

use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_proved_evaluator_model::authenticated_rdf::{self as auth, PrivateCredentials};
use sparq_proved_evaluator_model::v3::CanonicalResult;
use sparq_vcq_disclosed::fixtures::{self, QUERIES};
use sparq_vcq_disclosed::{MAX_PROOF_JSON_BYTES, decode_proof, present, verify};

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

#[test]
fn describe_and_dataset_clauses_are_rejected() {
    let (statement, proof) = present(
        &fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, NONCE),
        fixtures::credentials(1),
        SALT,
    )
    .expect("presents");
    for text in [
        "DESCRIBE <https://bank.example/payments/2026-06>",
        "SELECT ?s FROM <https://bank.example/g> WHERE { ?s ?p ?o }",
        "SELECT ?s FROM NAMED <https://bank.example/g> WHERE { ?s ?p ?o }",
    ] {
        let request = fixtures::request(text, DatasetAuthority::HolderDeclared, NONCE);
        assert!(
            present(&request, fixtures::credentials(1), SALT).is_err(),
            "present admitted {text}"
        );
        // The verifier checks the query form itself, even for a statement the
        // shared relation computes for that query.
        let own = auth::evaluate(&auth::Witness {
            request: request.clone(),
            dataset: PrivateCredentials {
                credentials: proof.credentials.clone(),
                salt: SALT,
            },
        });
        let stated = own.as_ref().unwrap_or(&statement);
        assert!(
            verify(&request, stated, &proof).is_err(),
            "verify admitted {text}"
        );
    }
}

#[test]
fn a_query_with_more_than_one_permitted_result_verifies_only_the_evaluators_choice() {
    let text = "PREFIX bank: <https://bank.example/vocab#>\n\
                SELECT ?p WHERE { ?p a bank:Payment } LIMIT 1\n";
    let request = fixtures::request(text, DatasetAuthority::HolderDeclared, NONCE);
    let (statement, proof) = present(&request, fixtures::credentials(1), SALT).expect("presents");
    assert_eq!(
        verify(&request, &statement, &proof).expect("verifies"),
        statement
    );
    // Another permitted solution for the same request is not accepted.
    let mut other = statement.clone();
    let sparq_proved_evaluator_model::v3::CanonicalResult::Select { rows, .. } = &mut other.result
    else {
        panic!("select result")
    };
    assert_eq!(rows.len(), 1);
    let other_row = present(
        &fixtures::request(
            "PREFIX bank: <https://bank.example/vocab#>\n\
             SELECT ?p WHERE { ?p a bank:Payment }\n",
            DatasetAuthority::HolderDeclared,
            NONCE,
        ),
        fixtures::credentials(1),
        SALT,
    )
    .expect("presents");
    let CanonicalResult::Select { rows: all, .. } = other_row.0.result else {
        panic!("select result")
    };
    let replacement = all
        .into_iter()
        .find(|r| *r != rows[0])
        .expect("another payment");
    rows[0] = replacement;
    assert!(verify(&request, &other, &proof).is_err());
}

#[test]
fn oversized_evidence_is_rejected_before_evaluation() {
    let request = fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, NONCE);
    let (statement, proof) = present(&request, fixtures::credentials(1), SALT).expect("presents");

    let mut many = proof.clone();
    many.credentials = vec![proof.credentials[0].clone(); auth::MAX_CREDENTIALS + 1];
    assert_eq!(
        verify(&request, &statement, &many).unwrap_err().0,
        "disclosed credential count must be 1 to 4"
    );

    let mut large = proof.clone();
    large.credentials[0].document = "x".repeat(auth::MAX_DOCUMENT_BYTES + 1);
    assert_eq!(
        verify(&request, &statement, &large).unwrap_err().0,
        "disclosed credential capacity"
    );

    let encoded = serde_json::to_vec(&proof).expect("serializes");
    let decoded = decode_proof(&encoded).expect("decodes");
    assert_eq!(
        verify(&request, &statement, &decoded).expect("verifies"),
        statement
    );
    let oversized = vec![b' '; MAX_PROOF_JSON_BYTES + 1];
    assert_eq!(
        decode_proof(&oversized).unwrap_err().0,
        "disclosed proof encoding capacity"
    );
}

#[test]
fn holder_checks_capacities_before_evaluating() {
    let request = fixtures::request(query("Q2"), DatasetAuthority::HolderDeclared, NONCE);
    // The method's own messages show its check ran before the relation's.
    assert_eq!(
        present(
            &request,
            fixtures::credentials(auth::MAX_CREDENTIALS + 1),
            SALT
        )
        .unwrap_err()
        .0,
        "disclosed credential count must be 1 to 4"
    );
    let mut large = fixtures::credentials(1);
    large[0].document = "x".repeat(auth::MAX_DOCUMENT_BYTES + 1);
    assert_eq!(
        present(&request, large, SALT).unwrap_err().0,
        "disclosed credential capacity"
    );

    let mut wide = request.clone();
    let key = wide.policy.authorization[0].clone();
    wide.policy.authorization = vec![key.clone(); auth::MAX_AUTHORIZED_KEYS + 1];
    let mut long_iri = request.clone();
    long_iri.policy.authorization[0].issuer = "x".repeat(auth::MAX_IRI_BYTES + 1);
    let (statement, proof) = present(&request, fixtures::credentials(1), SALT).expect("presents");
    for bad in [wide, long_iri] {
        assert_eq!(
            present(&bad, fixtures::credentials(1), SALT).unwrap_err().0,
            "disclosed request key table capacity"
        );
        assert_eq!(
            verify(&bad, &statement, &proof).unwrap_err().0,
            "disclosed request key table capacity"
        );
    }
}

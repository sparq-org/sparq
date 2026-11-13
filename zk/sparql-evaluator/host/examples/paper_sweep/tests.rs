use super::*;

#[test]
fn native_records_include_complete_credential_sizes_without_receipt_work() {
    let guest = AcceptedGuest::from_artifact(
        embedded_authrdf_artifact().to_vec(),
        &embedded_authrdf_pin(),
    )
    .unwrap();
    let args = Args {
        set: "main".into(),
        execute: false,
        phases: false,
        prove: 0,
        only: None,
        suite: None,
        mode: None,
        n: None,
        size: None,
        blank_free: false,
    };
    for suite in [
        Cryptosuite::EddsaRdfc2022,
        Cryptosuite::EddsaSha256Merkle2026,
    ] {
        for mode in [SignatureMode::Hidden, SignatureMode::Revealed] {
            let record = run(
                &args,
                "main",
                "native-fixture",
                "ASK { ?s ?p ?o }",
                &Config {
                    suite,
                    mode,
                    n: 1,
                    size: 16,
                },
                &guest,
            );
            assert_eq!(record["schema"], "sparq.zk-paper-sweep.v2");
            assert_eq!(record["admitted"], true);
            assert_eq!(record["verifier_split_status"], "not_measured_no_receipt");
            assert!(record.get("proofs").is_none());
            assert_eq!(record["issuer_sign_ns"].as_array().unwrap().len(), 1);
            assert_eq!(
                record["issuer_signature_primitive_ns"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                record["issuer_merkle_tree_ns"][0].is_null(),
                suite == Cryptosuite::EddsaRdfc2022
            );
            let (credential, _) = issue(
                &payment_document(0),
                &proof_config(PAYMENT_VM, suite),
                suite,
                [0x40; 32],
            );
            assert_eq!(
                record["credential_bytes"][0],
                credential.document.len() + credential.proof_config.len()
            );
            assert_eq!(
                record["credential_encoded_bytes"][0],
                serde_json::to_vec(&credential).unwrap().len()
            );
            assert_eq!(record["signature_bytes"], 64);
        }
    }
}

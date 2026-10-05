// [GPT-6] Opt-in export of synthetic, already verified integration-test receipts.
use risc0_zkvm::{InnerReceipt, Receipt, VerifierContext};
use serde::Serialize;
use sparq_proved_evaluator::{ArtifactPin, embedded_pin};
use std::{fs::OpenOptions, io::Write, path::PathBuf};

pub fn record(name: &str, request: &impl Serialize, receipt: &Receipt) {
    let Some(directory) = std::env::var_os("SPARQ_EVALUATOR_EVIDENCE_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let pin: ArtifactPin = serde_json::from_slice(
        &std::fs::read(directory.join("artifact/pin.json")).expect("exported artifact pin"),
    )
    .expect("typed exported artifact pin");
    assert_eq!(pin, embedded_pin(), "tested and exported guest must match");
    assert!(matches!(receipt.inner, InnerReceipt::Succinct(_)));
    receipt
        .verify_with_context(
            &VerifierContext::default().with_dev_mode(false),
            pin.image_id,
        )
        .expect("exported receipt independently verifies with the exported guest");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema": "sparq-synthetic-receipt-evidence-v1",
        "fixture": name,
        "synthetic_inputs": true,
        "pin": pin,
        "request": request,
        "receipt": receipt,
    }))
    .unwrap();
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("receipts").join(format!("{name}.json")))
        .expect("new receipt evidence file")
        .write_all(&bytes)
        .expect("complete receipt evidence write");
}

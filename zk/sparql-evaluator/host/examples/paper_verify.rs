//! Measure an already retained answer without generating a proof.
//!
//! `paper_verify REQUEST.json ANSWER.json` verifies against an independently
//! supplied request and the embedded approved V5 pin. `paper_verify --size-only
//! ANSWER.json` measures encoding only, without any verification claim.
use serde_json::{Value, json};
use sparq_proved_evaluator::authenticated_rdf::{
    AnswerPresentation, verify_answer_with_artifact_timed,
};
use sparq_proved_evaluator::{
    AcceptedGuest, Error, Nonces, embedded_authrdf_artifact, embedded_authrdf_pin,
};
use sparq_proved_evaluator_model::authenticated_rdf::Request;
use std::{fs, hint::black_box, time::Instant};
#[path = "paper_sweep/encoding.rs"]
mod encoding;

#[derive(Default)]
struct Once(bool);
impl Nonces for Once {
    fn consume(&mut self, _: [u8; 32]) -> Result<bool, Error> {
        Ok(!std::mem::replace(&mut self.0, true))
    }
}

fn size_record(answer: &AnswerPresentation, received: usize) -> Value {
    json!({
        "schema": "sparq.zk-paper-verification.v2",
        "presentation_encoding": "serde-json-answer-presentation-v1",
        "presentation_received_bytes": received,
        "presentation_bincode_bytes": encoding::bincode_bytes(answer).len(),
        "presentation_compact_encoding": encoding::BINCODE_ENCODING,
        "presentation_encoded_bytes": serde_json::to_vec(answer).expect("complete answer encoding").len(),
        "presentation_bytes": serde_json::to_vec(&answer.presentation.receipt).expect("receipt encoding").len()
            + answer.signatures.as_ref().map_or(0, |signatures| signatures.iter().map(Vec::len).sum::<usize>()),
        "verified": false,
    })
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        2,
        "paper_verify REQUEST.json ANSWER.json | --size-only ANSWER.json"
    );
    let bytes = fs::read(&args[1]).expect("answer file");
    let answer: AnswerPresentation =
        serde_json::from_slice(&bytes).expect("complete answer decoding");
    let mut record = size_record(&answer, bytes.len());
    if args[0] != "--size-only" {
        let expected: Request =
            serde_json::from_slice(&fs::read(&args[0]).expect("independent request file"))
                .expect("request decoding");
        let guest = AcceptedGuest::from_artifact(
            embedded_authrdf_artifact().to_vec(),
            &embedded_authrdf_pin(),
        )
        .expect("approved V5 pin");
        let mut nonces = Once::default();
        let inputs = black_box((&answer, &expected, &mut nonces, &guest));
        let start = Instant::now();
        let (journal, timings) = black_box(verify_answer_with_artifact_timed(
            inputs.0, inputs.1, inputs.2, inputs.3,
        ))
        .expect("receipt verification");
        let aggregate = start.elapsed();
        record["verified"] = json!(true);
        record["verify_ns"] = json!(aggregate.as_nanos());
        record["verifier_receipt_ns"] = json!(timings.receipt.as_nanos());
        record["verifier_signature_ns"] = json!(timings.signatures.map(|time| time.as_nanos()));
        record["verifier_request_ns"] = json!(timings.request.as_nanos());
        record["verifier_stages_ns"] = json!(
            (timings.receipt + timings.signatures.unwrap_or_default() + timings.request).as_nanos()
        );
        record["journal"] = json!(journal);
    }
    println!("{record}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};

    #[test]
    fn complete_answer_sizing_needs_no_proof_and_counts_detached_signatures() {
        for signatures in [None, Some(vec![vec![7; 64]])] {
            let answer = AnswerPresentation {
                presentation: sparq_proved_evaluator::Presentation {
                    receipt: Receipt::new(
                        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok([0; 8], vec![]))),
                        vec![],
                    ),
                },
                signatures,
            };
            let encoded = serde_json::to_vec(&answer).unwrap();
            let record = size_record(&answer, encoded.len());
            assert_eq!(record["presentation_encoded_bytes"], encoded.len());
            assert_eq!(record["presentation_received_bytes"], encoded.len());
            let binary = encoding::bincode_bytes(&answer);
            assert_eq!(record["presentation_bincode_bytes"], binary.len());
            let binary_decoded: AnswerPresentation = bincode::deserialize(&binary).unwrap();
            assert_eq!(serde_json::to_vec(&binary_decoded).unwrap(), encoded);
            assert!(binary.len() < encoded.len());
            assert_eq!(record["verified"], false);
            assert!(record.get("verifier_receipt_ns").is_none());
            assert!(
                record["presentation_encoded_bytes"].as_u64().unwrap()
                    > record["presentation_bytes"].as_u64().unwrap()
            );
            let decoded: AnswerPresentation = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded.signatures, answer.signatures);
            let mut object: Value = serde_json::from_slice(&encoded).unwrap();
            object["extra"] = json!(true);
            assert!(serde_json::from_value::<AnswerPresentation>(object).is_err());
        }
    }
}

//! Portable prepared-program and presentation values.

use crate::{answer::Answer, plan::Plan, Error};
use sparq_query_protocol::MethodDescriptor;

/// A locally compiled shape and its exact verification key.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: Plan,
    pub verification_key: Vec<u8>,
    pub descriptor: MethodDescriptor,
    /// Locally compiled artifact, for reproducibility checks and export.
    pub artifact: serde_json::Value,
}

/// A presentation carries RDF results and evidence, never a plan, program,
/// verification key, ABI, capacity or public-input blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub request_digest: [u8; 32],
    pub input_commitment: [u8; 32],
    pub signature_mode: u8,
    pub result: Answer,
    pub proof: Vec<u8>,
}
impl Presentation {
    /// Canonical local envelope for this draft primitive. The public adapter
    /// additionally applies its complete transport envelope's resource bound.
    pub fn encoded(&self) -> Result<Vec<u8>, Error> {
        let result = match &self.result {
            Answer::Ask(value) => serde_json::json!({"boolean":value}),
            Answer::Select { variables, rows } => serde_json::json!({"variables":variables,
                "rows":rows.iter().map(|r|r.iter().map(|t|t.as_ref().map(ToString::to_string)).collect::<Vec<_>>()).collect::<Vec<_>>() }),
        };
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        serde_json::to_vec(&serde_json::json!({"requestDigest":URL_SAFE_NO_PAD.encode(self.request_digest),
            "inputCommitment":URL_SAFE_NO_PAD.encode(self.input_commitment),"signatureMode":self.signature_mode,
            "result":result,"proof":URL_SAFE_NO_PAD.encode(&self.proof)})).map_err(|e| Error::Infrastructure(e.to_string()))
    }
}

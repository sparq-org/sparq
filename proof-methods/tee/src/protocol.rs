//! Holder ↔ enclave messages over vsock, and the enclave's request handler.
//!
//! Each message is a 4-byte big-endian length followed by that many bytes of
//! JSON. The holder sends one [`EnclaveRequest`]; the enclave answers with one
//! [`EnclaveResponse`].

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use sparq_proved_evaluator_model::authenticated_rdf::{Request, SignedCredential};

use crate::{METHOD_VERSION, PLATFORM, Rejected, Statement, TeeProof, statement_digest};

/// vsock port the enclave listens on.
pub const PORT: u32 = 5005;
/// Largest message accepted in either direction.
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;

/// What the holder sends the enclave.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnclaveRequest {
    pub request: Request,
    pub credentials: Vec<SignedCredential>,
    pub salt: [u8; 32],
}

/// What the enclave answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EnclaveResponse {
    Presentation {
        statement: Statement,
        proof: TeeProof,
    },
    Rejected(String),
}

/// Produces an attestation document over `user_data` and `nonce`: the Nitro
/// Secure Module inside an enclave, or a test double.
pub trait Attester {
    fn attest(&mut self, user_data: &[u8; 32], nonce: &[u8; 32]) -> Result<Vec<u8>, Rejected>;
}

/// Enclave side: evaluates the request over the credentials with the same
/// admission checks and relation as `disclosed-reevaluation`, then attests
/// the statement digest.
pub fn handle(message: &[u8], attester: &mut impl Attester) -> EnclaveResponse {
    let request: EnclaveRequest = match serde_json::from_slice(message) {
        Ok(request) => request,
        Err(_) => return EnclaveResponse::Rejected("malformed enclave request".into()),
    };
    let result = sparq_vcq_disclosed::present(&request.request, request.credentials, request.salt)
        .and_then(|(statement, _)| {
            let attestation =
                attester.attest(&statement_digest(&statement), &request.request.nonce)?;
            Ok(EnclaveResponse::Presentation {
                statement,
                proof: TeeProof {
                    version: METHOD_VERSION,
                    platform: PLATFORM.into(),
                    attestation,
                },
            })
        });
    result.unwrap_or_else(|Rejected(reason)| EnclaveResponse::Rejected(reason.into()))
}

/// Writes one length-prefixed message.
pub fn write_message(stream: &mut impl Write, body: &[u8]) -> std::io::Result<()> {
    let length = u32::try_from(body.len())
        .ok()
        .filter(|n| *n as usize <= MAX_MESSAGE_BYTES)
        .ok_or_else(|| std::io::Error::other("message too long"))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Reads one length-prefixed message, rejecting one longer than
/// [`MAX_MESSAGE_BYTES`] before allocating for it.
pub fn read_message(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::other("message too long"));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}

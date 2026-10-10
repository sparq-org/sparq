//! The program that runs inside the Nitro Enclave: serves one presentation per
//! vsock connection on [`protocol::PORT`].

use aws_nitro_enclaves_nsm_api::api::{Request as NsmRequest, Response as NsmResponse};
use aws_nitro_enclaves_nsm_api::driver::{nsm_exit, nsm_init, nsm_process_request};
use serde_bytes::ByteBuf;
use sparq_vcq_tee::Rejected;
use sparq_vcq_tee::protocol::{self, Attester};
use vsock::{VMADDR_CID_ANY, VsockListener};

struct Nsm(i32);

impl Attester for Nsm {
    fn attest(&mut self, user_data: &[u8; 32], nonce: &[u8; 32]) -> Result<Vec<u8>, Rejected> {
        let request = NsmRequest::Attestation {
            user_data: Some(ByteBuf::from(user_data.to_vec())),
            nonce: Some(ByteBuf::from(nonce.to_vec())),
            public_key: None,
        };
        match nsm_process_request(self.0, request) {
            NsmResponse::Attestation { document } => Ok(document),
            _ => Err(Rejected("Nitro Secure Module refused the attestation")),
        }
    }
}

fn main() -> std::io::Result<()> {
    let fd = nsm_init();
    if fd < 0 {
        return Err(std::io::Error::other("cannot open the Nitro Secure Module"));
    }
    let mut nsm = Nsm(fd);
    let listener = VsockListener::bind_with_cid_port(VMADDR_CID_ANY, protocol::PORT)?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        let response = match protocol::read_message(&mut stream) {
            Ok(message) => protocol::handle(&message, &mut nsm),
            Err(_) => protocol::EnclaveResponse::Rejected("unreadable message".into()),
        };
        let body = serde_json::to_vec(&response).expect("response serializes");
        let _ = protocol::write_message(&mut stream, &body);
    }
    nsm_exit(fd);
    Ok(())
}

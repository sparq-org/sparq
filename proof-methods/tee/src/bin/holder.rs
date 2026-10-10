//! Holder-side driver: sends Q1–Q5 over 1 and 4 credentials to the enclave,
//! verifies each presentation as a verifier would, and prints one JSON object
//! per (query, n) with holder round-trip and verifier times.
//!
//! `sparq-vcq-tee-holder CID PCR0_HEX [reps]`

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_vcq_disclosed::fixtures::{self, QUERIES};
use sparq_vcq_tee::protocol::{self, EnclaveRequest, EnclaveResponse};
use sparq_vcq_tee::{TrustPolicy, nitro, verify};
use vsock::{VsockAddr, VsockStream};

fn median_iqr(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.25), at(0.75))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_millis() as u64
}

fn parse_pcr0(hex: &str) -> [u8; 48] {
    assert_eq!(hex.len(), 96, "PCR0 is 48 bytes of hex");
    let mut out = [0u8; 48];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("PCR0 is hex");
    }
    out
}

/// Connects to the enclave, waiting up to a minute for it to start listening.
fn connect(cid: u32) -> VsockStream {
    let address = VsockAddr::new(cid, protocol::PORT);
    for _ in 0..600 {
        if let Ok(stream) = VsockStream::connect(&address) {
            return stream;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("enclave {cid} is not listening on port {}", protocol::PORT);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cid: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .expect("usage: CID PCR0_HEX [reps]");
    let pcr0 = parse_pcr0(&args.next().expect("usage: CID PCR0_HEX [reps]"));
    let reps: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(21);
    let policy = TrustPolicy {
        root: nitro::aws_root_g1(),
        pcr0: vec![pcr0],
        max_age_ms: 5 * 60 * 1000,
    };
    drop(connect(cid));
    let salt = [0xa5; 32];
    let mut counter = 0u64;
    for n in [1, 4] {
        let credentials = fixtures::credentials(n);
        for q in QUERIES {
            let (mut holder, mut verifier) = (Vec::new(), Vec::new());
            let mut attestation_bytes = 0;
            for _ in 0..reps {
                counter += 1;
                let nonce: [u8; 32] = Sha256::digest(counter.to_le_bytes()).into();
                let request = fixtures::request(q.text, DatasetAuthority::HolderDeclared, nonce);
                let message = serde_json::to_vec(&EnclaveRequest {
                    request: request.clone(),
                    credentials: credentials.clone(),
                    salt,
                })
                .expect("request serializes");

                let start = Instant::now();
                let mut stream =
                    VsockStream::connect(&VsockAddr::new(cid, protocol::PORT)).expect("connects");
                protocol::write_message(&mut stream, &message).expect("sends");
                let reply = protocol::read_message(&mut stream).expect("receives");
                holder.push(start.elapsed().as_secs_f64() * 1e3);

                let (statement, proof) = match serde_json::from_slice(&reply).expect("decodes") {
                    EnclaveResponse::Presentation { statement, proof } => (statement, proof),
                    EnclaveResponse::Rejected(reason) => panic!("{} n={n}: {reason}", q.id),
                };
                attestation_bytes = proof.attestation.len();
                let start = Instant::now();
                verify(&request, &statement, &proof, &policy, now_ms()).expect("verifies");
                verifier.push(start.elapsed().as_secs_f64() * 1e3);
            }
            let (h, v) = (median_iqr(holder), median_iqr(verifier));
            println!(
                "{}",
                serde_json::json!({
                    "method": sparq_vcq_tee::METHOD_ID,
                    "platform": sparq_vcq_tee::PLATFORM,
                    "query": q.id,
                    "credentials": n,
                    "reps": reps,
                    "unit": "milliseconds",
                    "holder_round_trip": {"median": h.0, "q1": h.1, "q3": h.2},
                    "verifier_verify": {"median": v.0, "q1": v.1, "q3": v.2},
                    "attestation_bytes": attestation_bytes,
                })
            );
        }
    }
}

//! Measures issuer, holder and verifier cost of disclosed re-evaluation over
//! Q1–Q5 for n credentials, and prints one JSON object per (query, n).
//!
//! `cargo run --release --example measure --features fixtures -- [reps]`

use std::time::Instant;

use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_vcq_disclosed::fixtures::{self, QUERIES};
use sparq_vcq_disclosed::{present, verify};

const PAYMENTS: usize = 3;

fn median_iqr(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.25), at(0.75))
}

fn time<T>(reps: usize, mut f: impl FnMut() -> T) -> (f64, f64, f64) {
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let start = Instant::now();
        std::hint::black_box(f());
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    median_iqr(samples)
}

fn main() {
    let reps: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(21);
    let salt = [0xa5; 32];
    for n in [1, 4] {
        let issuer = time(reps, || fixtures::credentials(n, PAYMENTS));
        let credentials = fixtures::credentials(n, PAYMENTS);
        let raw_bytes: usize = credentials
            .iter()
            .map(|c| c.document.len() + c.proof_config.len() + c.signature.len())
            .sum();
        let triples: usize = credentials.iter().map(|c| c.document.lines().count()).sum();
        for q in QUERIES {
            let request = fixtures::request(q.text, DatasetAuthority::HolderDeclared, [7; 32]);
            let holder = time(reps, || {
                present(&request, credentials.clone(), salt).expect("presents")
            });
            let (statement, proof) =
                present(&request, credentials.clone(), salt).expect("presents");
            let verifier = time(reps, || {
                verify(&request, &statement, &proof).expect("verifies")
            });
            let proof_json = serde_json::to_vec(&proof).expect("serializes").len();
            println!(
                "{}",
                serde_json::json!({
                    "method": sparq_vcq_disclosed::METHOD_ID,
                    "query": q.id,
                    "credentials": n,
                    "triples": triples,
                    "reps": reps,
                    "unit": "microseconds",
                    "issuer_sign_all": {"median": issuer.0, "q1": issuer.1, "q3": issuer.2},
                    "holder_present": {"median": holder.0, "q1": holder.1, "q3": holder.2},
                    "verifier_verify": {"median": verifier.0, "q1": verifier.1, "q3": verifier.2},
                    "proof_raw_bytes": raw_bytes,
                    "proof_json_bytes": proof_json,
                })
            );
        }
    }
}

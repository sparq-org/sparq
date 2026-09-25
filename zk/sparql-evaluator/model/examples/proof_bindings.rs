// [GPT-6] Native evaluation adapter; never generates or counts cryptographic proofs.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::{self as model, DatasetAuthority, ProofContract, v2};
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::PathBuf,
};

fn authority(job: &Value, commitment: [u8; 32]) -> Result<DatasetAuthority, Box<dyn Error>> {
    match job["authority"].as_str() {
        Some("verifier_agreed") => Ok(DatasetAuthority::VerifierAgreed { commitment }),
        Some("holder_declared") => Ok(DatasetAuthority::HolderDeclared),
        _ => Err("unknown dataset authority".into()),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: proof_bindings INPUT_JSON NEW_OUTPUT_DIRECTORY".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&args[0])?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("job byte capacity".into());
    }
    let job: Value = serde_json::from_slice(&bytes)?;
    if job["schema"] != "sparq.proof-binding-job.v1"
        || job["tier"] != "native"
        || !matches!(job["operation"].as_str(), Some("result" | "admission"))
    {
        return Err("native adapter schema, tier or operation rejected".into());
    }
    let query = job["query"].as_str().ok_or("missing query")?.to_owned();
    let mut hash = Sha256::new();
    hash.update(b"sparq-proof-binding-native-nonce-v1\0");
    hash.update(job["id"].as_str().ok_or("missing job identity")?.as_bytes());
    let nonce: [u8; 32] = hash.finalize().into();
    let result = match job["backend"].as_str() {
        Some("exact_v1") => {
            let mut policy = model::Policy::default();
            if let Some(limit) = job["policy_overrides"]["max_rows"].as_u64() {
                policy.max_rows = limit.try_into()?;
            }
            let dataset = model::PrivateDataset {
                ntriples: job["dataset"]["ntriples"]
                    .as_str()
                    .ok_or("missing default dataset")?
                    .to_owned(),
                salt: [91; 32],
            };
            let commitment = model::dataset_commitment(&dataset, &policy)?;
            let witness = model::Witness {
                request: model::Request {
                    version: model::VERSION,
                    contract: ProofContract::ExactDataset,
                    dialect: model::Dialect::SparqSparql11SnapshotV1,
                    query,
                    authority: authority(&job, commitment)?,
                    policy,
                    nonce,
                },
                dataset,
            };
            model::evaluate(&witness).map(|journal| journal.result)
        }
        Some("exact_v2") => {
            let mut policy = v2::Policy::default();
            if let Some(limit) = job["policy_overrides"]["max_rows"].as_u64() {
                policy.max_rows = limit.try_into()?;
            }
            let dataset = v2::PrivateDataset {
                nquads: job["dataset"]["nquads"]
                    .as_str()
                    .ok_or("missing complete dataset")?
                    .to_owned(),
                named_graphs: serde_json::from_value(job["dataset"]["named_graphs"].clone())?,
                salt: [91; 32],
            };
            let commitment = v2::dataset_commitment(&dataset, &policy)?;
            let witness = v2::Witness {
                request: v2::Request {
                    version: v2::VERSION,
                    contract: ProofContract::ExactDataset,
                    dialect: v2::Dialect::SparqSparql11DatasetV2,
                    query,
                    authority: authority(&job, commitment)?,
                    policy,
                    nonce,
                },
                dataset,
            };
            v2::evaluate(&witness).map(|journal| journal.result)
        }
        _ => return Err("adapter backend unavailable; never classified as unsupported".into()),
    };
    let (observed, value, error_class) = match result {
        Ok(value) => ("accepted", serde_json::to_value(value)?, None),
        Err(error) => ("rejected", Value::Null, Some(error.to_string())),
    };
    let outcome = json!({"schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
        "case_sha256":job["case_sha256"], "backend":job["backend"], "tier":"native",
        "observed":observed,"stage":"native","proof_count":0,"verified_count":0,
        "error_class":error_class,"artifacts":[],"controls":[],"result":value});
    let output = PathBuf::from(&args[1]);
    fs::create_dir(&output)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("outcome.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&outcome)?)?;
    file.write_all(b"\n")?;
    Ok(())
}

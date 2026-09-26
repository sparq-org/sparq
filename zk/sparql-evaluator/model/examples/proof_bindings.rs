// [GPT-6] Native evaluation adapter; never generates or counts cryptographic proofs.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_engine::{BudgetExceeded, EvaluationCapacity};
use sparq_proved_evaluator_model::{self as model, DatasetAuthority, ProofContract, v2};
#[cfg(feature = "graph-results")]
use sparq_proved_evaluator_model::v3;
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

// [GPT-6] The model exposes static diagnostics, including one deliberately
// ambiguous evaluation/budget error. Classify only exact reviewed producers;
// never let expected job data or a substring decide what actually failed.
fn rejection(error: &model::Rejected, phase: &str) -> Option<Value> {
    let classes: Value = serde_json::from_str(include_str!(
        "../../../../bench/zk-bindings/rejections.json"
    ))
    .expect("committed rejection classifications");
    let mut classified = classes["diagnostics"].get(error.0)?.clone();
    if phase == "admission" {
        classified["phase"] = json!("admission");
    }
    classified["diagnostic"] = json!(error.0);
    Some(classified)
}

// [GPT-6] Only actual enum values emitted by the owning query frame certify
// execution capacity. Neither a legacy string nor the job's expectation does.
fn detailed_rejection(error: &model::EvaluationError, phase: &str) -> Option<Value> {
    use model::EvaluationError;
    let cause = match error {
        EvaluationError::Rejected(error) => return rejection(error, phase),
        EvaluationError::Budget(BudgetExceeded::Rows) => "budget_rows",
        EvaluationError::Budget(BudgetExceeded::Bytes) => "budget_bytes",
        EvaluationError::Budget(BudgetExceeded::Deadline) => "budget_deadline",
        EvaluationError::Budget(BudgetExceeded::Cancelled) => "budget_cancelled",
        EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation) => "numeric_representation",
        EvaluationError::Capacity(EvaluationCapacity::TemporalYear) => "temporal_year",
        EvaluationError::Execution => "execution",
    };
    let classes: Value = serde_json::from_str(include_str!(
        "../../../../bench/zk-bindings/rejections.json"
    )).expect("committed rejection classifications");
    let mut classified = classes["typed_causes"][cause].clone();
    classified["cause"] = json!(cause);
    Some(classified)
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
    let (phase, result) = match job["backend"].as_str() {
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
            match model::admit(&witness.request) {
                Err(error) => ("admission", Err(error.into())),
                Ok(()) => (
                    "evaluation",
                    model::evaluate_detailed(&witness).map(|journal| json!(journal.result)),
                ),
            }
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
            match v2::admit(&witness.request) {
                Err(error) => ("admission", Err(error.into())),
                Ok(()) => (
                    "evaluation",
                    v2::evaluate_detailed(&witness).map(|journal| json!(journal.result)),
                ),
            }
        }
        #[cfg(feature = "graph-results")]
        Some("exact_v3") => {
            let mut policy = v3::Policy::default();
            if let Some(limit) = job["policy_overrides"]["max_rows"].as_u64() {
                policy.dataset.max_rows = limit.try_into()?;
            }
            let dataset = v3::PrivateDataset {
                nquads: job["dataset"]["nquads"].as_str()
                    .ok_or("missing complete dataset")?.to_owned(),
                named_graphs: serde_json::from_value(job["dataset"]["named_graphs"].clone())?,
                salt: [91; 32],
            };
            let commitment = v3::dataset_commitment(&dataset, &policy)?;
            let witness = v3::Witness {
                request: v3::Request {
                    version: v3::VERSION,
                    contract: ProofContract::ExactDataset,
                    dialect: v3::Dialect::SparqSparql11GraphResultsV3,
                    query,
                    authority: authority(&job, commitment)?,
                    policy,
                    nonce,
                },
                dataset,
            };
            match v3::admit(&witness.request) {
                Err(error) => ("admission", Err(error.into())),
                Ok(()) => ("evaluation", v3::evaluate_detailed(&witness)
                    .map(|journal| json!(journal.result))),
            }
        }
        _ => return Err("adapter backend unavailable; never classified as unsupported".into()),
    };
    let (observed, value, error_class, classified, diagnostic) = match result {
        Ok(value) => (
            "accepted",
            serde_json::to_value(value)?,
            Value::Null,
            Value::Null,
            Value::Null,
        ),
        Err(error) => match detailed_rejection(&error, phase) {
            Some(classified) => (
                "rejected",
                Value::Null,
                classified["category"].clone(),
                classified,
                json!(error.to_string()),
            ),
            None => (
                "error",
                Value::Null,
                json!("unclassified_model_rejection"),
                Value::Null,
                json!(error.to_string()),
            ),
        },
    };
    let outcome = json!({"schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
        "case_sha256":job["case_sha256"], "backend":job["backend"], "tier":"native",
        "observed":observed,"stage":"native","proof_count":0,"verified_count":0,
        "error_class":error_class,"rejection":classified,"notes":diagnostic,
        "artifacts":[],"controls":[],"result":value});
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

#[test]
fn unrelated_and_ambiguous_errors_are_not_capacity_evidence() {
    for message in [
        "query evaluation or resource budget rejected",
        "unexpected capacity panic",
        "dataset byte capacity or blinding rejected",
    ] {
        assert_eq!(rejection(&model::Rejected(message), "evaluation"), None);
    }
    assert_eq!(
        rejection(&model::Rejected("SPARQL parse rejected"), "admission").unwrap()["category"],
        "parse"
    );
    let actual = rejection(&model::Rejected("temporal year capacity"), "admission").unwrap();
    assert_eq!(actual["category"], "capacity");
    assert_eq!(actual["phase"], "admission");
}

#[test]
fn only_typed_capacity_and_resource_limits_classify_as_capacity() {
    use model::EvaluationError;
    for error in [EvaluationError::Budget(BudgetExceeded::Rows),
                  EvaluationError::Budget(BudgetExceeded::Bytes),
                  EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation),
                  EvaluationError::Capacity(EvaluationCapacity::TemporalYear)] {
        let actual = detailed_rejection(&error, "evaluation").unwrap();
        assert_eq!(actual["category"], "capacity");
        assert_eq!(actual["phase"], "evaluation");
    }
    for (error, category) in [(EvaluationError::Budget(BudgetExceeded::Deadline), "deadline"),
                              (EvaluationError::Budget(BudgetExceeded::Cancelled), "cancelled"),
                              (EvaluationError::Execution, "execution")] {
        assert_eq!(detailed_rejection(&error, "evaluation").unwrap()["category"], category);
    }
    assert_eq!(detailed_rejection(&EvaluationError::Rejected(model::Rejected(
        "query evaluation or resource budget rejected")), "evaluation"), None);
}

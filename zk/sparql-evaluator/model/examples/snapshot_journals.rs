// Native full-journal comparison harness; never executes a prover.
use serde_json::{Value, json};
use sparq_proved_evaluator_model::{self as model, DatasetAuthority, ProofContract, v2, v3};
use std::{error::Error, fs, io::Write, path::Path};

fn outcome<T: serde::Serialize>(result: Result<T, model::EvaluationError>) -> Value {
    match result {
        Ok(journal) => json!({"journal": journal}),
        Err(error) => json!({"error": format!("{error:?}")}),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("usage: snapshot_journals NEW_JSONL")?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let fixtures = root.join("zk/sparql-evaluator/fixtures/conformance");
    let mut sources = fs::read_dir(&fixtures)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    for file in [
        "builtin_edges.json",
        "exact_temporal.json",
        "temporal_lexical.json",
    ] {
        sources.push(root.join("crates/sparq-engine/tests/fixtures").join(file));
    }
    sources.sort();
    for source in sources {
        if source.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let corpus: Value = serde_json::from_slice(&fs::read(&source)?)?;
        let Some(cases) = corpus["cases"].as_array() else {
            continue;
        };
        for case in cases {
            let Some(query) = case["query"].as_str() else {
                continue;
            };
            let data = case["dataset_ntriples"]
                .as_str()
                .or_else(|| case["dataset"].as_str())
                .or_else(|| corpus["dataset_ntriples"].as_str());
            let default = if corpus["default_dataset"] == "core.nt" {
                fs::read_to_string(fixtures.join("core.nt"))?
            } else {
                String::new()
            };
            let data = data.unwrap_or(&default);
            for holder in [false, true] {
                let authority = |commitment| {
                    if holder {
                        DatasetAuthority::HolderDeclared
                    } else {
                        DatasetAuthority::VerifierAgreed { commitment }
                    }
                };
                let policy = model::Policy::default();
                let dataset = model::PrivateDataset {
                    ntriples: data.into(),
                    salt: [17; 32],
                };
                let input = model::Witness {
                    request: model::Request {
                        version: model::VERSION,
                        contract: ProofContract::ExactDataset,
                        dialect: model::Dialect::SparqSparql11SnapshotV1,
                        query: query.into(),
                        authority: authority(model::dataset_commitment(&dataset, &policy)?),
                        policy,
                        nonce: [29; 32],
                    },
                    dataset,
                };
                let v1 = outcome(model::evaluate_detailed(&input));
                let policy = v2::Policy::default();
                let dataset = v2::PrivateDataset {
                    nquads: data.into(),
                    named_graphs: vec![],
                    salt: [17; 32],
                };
                let input = v2::Witness {
                    request: v2::Request {
                        version: v2::VERSION,
                        contract: ProofContract::ExactDataset,
                        dialect: v2::Dialect::SparqSparql11DatasetV2,
                        query: query.into(),
                        authority: authority(v2::dataset_commitment(&dataset, &policy)?),
                        policy,
                        nonce: [29; 32],
                    },
                    dataset,
                };
                let v2 = outcome(v2::evaluate_detailed(&input));
                let policy = v3::Policy::default();
                let dataset = v3::PrivateDataset {
                    nquads: data.into(),
                    named_graphs: vec![],
                    salt: [17; 32],
                };
                let input = v3::Witness {
                    request: v3::Request {
                        version: v3::VERSION,
                        contract: ProofContract::ExactDataset,
                        dialect: v3::Dialect::SparqSparql11GraphResultsV3,
                        query: query.into(),
                        authority: authority(v3::dataset_commitment(&dataset, &policy)?),
                        policy,
                        nonce: [29; 32],
                    },
                    dataset,
                };
                let record = json!({
                    "source": source.strip_prefix(&root)?.to_str().ok_or("non-UTF8 source")?,
                    "id": case["id"], "holder": holder,
                    "v1": v1, "v2": v2, "v3": outcome(v3::evaluate_detailed(&input)),
                });
                serde_json::to_writer(&mut output, &record)?;
                writeln!(output)?;
            }
        }
    }
    Ok(())
}

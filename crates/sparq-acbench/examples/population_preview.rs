//! [GPT-6] Emit deterministic size observations; this is not a serving benchmark.

use sparq_acbench::population::{PolicyModel, PopulationConfig, write_pod};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Eight complete records streams per language provide an early serialization
    // sizing sample. They are discarded after emission and are not hosted Pods.
    let config = PopulationConfig::service_history();
    let mut summaries = Vec::new();
    for model in [PolicyModel::Wac, PolicyModel::Acp] {
        for pod in 0..8 {
            let summary = write_pod(&config, pod, model, std::io::sink())?;
            summaries.push(serde_json::json!({"policy_model": model, "summary": summary}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "kind": "deterministic_serialization_sizing",
            "canonical_timing": false,
            "persisted_pods": 0,
            "config": config,
            "observations": summaries
        }))?
    );
    Ok(())
}

//! Test-only operator inputs; no credential-authentication assertion.
use crate::codec::{err, noir_numbers, witness_json, Result};
use serde_json::{json, Value};
use sparq_zk_query::{
    capacity::Bounds,
    codegen,
    credential::SuiteEncoder,
    interp,
    lower::lower,
    statement::{self, Layout, Statement},
    witness,
};
pub(crate) fn operator(input: &str) -> Result<Value> {
    let input: Value = crate::codec::parse(input)?;
    let b = &input["bounds"];
    let bounds = Bounds {
        credentials: b["credentials"].as_u64().unwrap_or(1) as u32,
        quads_per_credential: b["quadsPerCredential"].as_u64().unwrap_or(4) as u32,
        rows_per_table: b["rowsPerTable"].as_u64().unwrap_or(4) as u32,
    };
    let plan = lower(input["query"].as_str().ok_or("query required")?, bounds).map_err(err)?;
    let package = codegen::generate_operator_fixture(&plan).map_err(err)?;
    let mut result = json!({"name":package.name,"files":package.files});
    if input["generateOnly"] == true {
        return Ok(result);
    }
    let data = vec![interp::CredentialCells { quads: vec![] }];
    let trace = interp::evaluate(&plan, &data, &SuiteEncoder).map_err(err)?;
    let digest = [7; 32];
    let salt = [8; 32];
    let commitment = statement::dataset_commitment(&data, salt).map_err(err)?;
    let statement = Statement {
        request_digest: digest,
        input_commitment: commitment,
        signature_mode: 0,
        result: trace.result.clone(),
    };
    let public = Layout::new(&plan)
        .reconstruct(&plan, &statement, &SuiteEncoder)
        .map_err(err)?;
    let toml =
        witness::prover_toml(&plan, &data, &SuiteEncoder, &trace, digest, salt).map_err(err)?;
    result["witnessInputs"] = json!({"toml":toml,"json":noir_numbers(witness_json(&toml)?)});
    result["publicInputs"] = json!(public.into_iter().map(statement::hex).collect::<Vec<_>>());
    Ok(result)
}

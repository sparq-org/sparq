//! Manual native/Node interoperability evidence; not a required proof check.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::Value;
    use sparq_core::Graph;
    use sparq_zk::{commit::commit_graph_content, field::field_from_hex_str_strict, sig};
    use sparq_zk_compose::{
        driver::CircuitProver,
        manifest::StatusListSnapshot,
        result::{
            prepare_result, verify_result, ResultCredential, ResultPolicy, ResultPresentation,
        },
        InMemorySeenNonces, VerifierNonce,
    };
    use std::{collections::BTreeMap, path::PathBuf};

    let args = std::env::args().collect::<Vec<_>>();
    let mode = &args[1];
    let request: Value = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let string = |v: &Value| v.as_str().map(str::to_owned).ok_or("expected string");
    let query = string(&request["query"])?;
    let n = VerifierNonce::from_hex(&string(&request["nonce"])?).ok_or("nonce")?;
    let p = &request["policy"];
    let policy = ResultPolicy {
        trusted_issuers: p["trustedIssuers"]
            .as_array()
            .ok_or("keys")?
            .iter()
            .map(|v| sig::public_key_from_hex(v.as_str().unwrap()).ok_or("key"))
            .collect::<Result<_, _>>()?,
        snapshots: serde_json::from_value::<Vec<StatusListSnapshot>>(p["snapshots"].clone())?,
        min_version: p["minVersion"].as_u64().ok_or("min")?,
        max_version: p["maxVersion"].as_u64().ok_or("max")?,
        admit_unbound_legacy: false,
    };
    let driver = CircuitProver::from_crate_root();
    let dir = PathBuf::from(&args[3]);
    let presentation: ResultPresentation = if mode == "verify" {
        serde_json::from_value(request["presentation"].clone())?
    } else {
        let creds = request["credentials"]
            .as_array()
            .ok_or("credentials")?
            .iter()
            .map(|c| {
                let graph = Graph::load_str(c["graphNQuads"].as_str().unwrap(), "nquads")?;
                Ok::<_, String>(ResultCredential {
                    graph: commit_graph_content(
                        &graph,
                        field_from_hex_str_strict(c["salt"].as_str().unwrap()).ok_or("salt")?,
                    )
                    .map_err(|e| e.to_string())?,
                    issuer: sig::public_key_from_hex(c["issuer"].as_str().unwrap())
                        .ok_or("issuer")?,
                    signature: sig::signature_from_hex(c["signature"].as_str().unwrap())
                        .ok_or("signature")?,
                    status_list: c["statusRef"]["status_list"].as_str().unwrap().into(),
                    status_version: c["statusRef"]["version"].as_u64().unwrap(),
                    status_index: c["statusRef"]["index"].as_u64().unwrap(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared_json: Value = serde_json::from_str(&sparq_zk_result_wasm::prepare(
            &query,
            request["credentials"]
                .as_array()
                .unwrap()
                .iter()
                .map(Value::to_string)
                .collect(),
            &p.to_string(),
            &string(&request["nonce"])?,
            "{}",
        )?)?;
        let skeleton: ResultPresentation =
            serde_json::from_value(prepared_json["presentationSkeleton"].clone())?;
        let rows = skeleton
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|(k, v)| Ok((k.clone(), v.to_term().ok_or("released term")?)))
                    .collect::<Result<BTreeMap<_, _>, &str>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        prepare_result(&query, &creds, &rows, &policy, &n)?.prove(
            &driver,
            &dir,
            "native_crosscheck",
        )?
    };
    let verified = verify_result(
        &query,
        &presentation,
        &policy,
        &n,
        &InMemorySeenNonces::new(),
        &driver,
        &dir,
    )?;
    let mut output = request;
    output["presentation"] = serde_json::to_value(&presentation)?;
    std::fs::write(dir.join("exchange.json"), serde_json::to_vec(&output)?)?;
    println!(
        "Native verification accepted {} released mappings",
        verified.rows.len()
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}

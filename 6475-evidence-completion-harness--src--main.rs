// [GPT-6 Astra] Scratch-only baseline driver; upstream packages are unmodified.
use oxrdf::Quad;
use oxttl::NQuadsParser;
use rdf_canon::{CanonicalizationError, CanonicalizationOptions};
use serde_json::{Value, json};
use sha2::{Sha256, Sha384};
use std::{env, fs::File};

fn result_json<T: serde::Serialize>(result: Result<T, CanonicalizationError>) -> Value {
    match result {
        Ok(value) => json!({"value": value}),
        Err(error) => json!({"error": error.to_string(), "error_debug": format!("{error:?}")}),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        return Err("expected input.nq output|map SHA256|SHA384 default|limit".into());
    }
    let quads: Vec<Quad> = NQuadsParser::new()
        .for_reader(File::open(&args[1])?)
        .collect::<Result<_, _>>()?;
    let options = CanonicalizationOptions {
        hndq_call_limit: if args[4] == "default" {
            None
        } else {
            Some(args[4].parse()?)
        },
    };
    let result = match (args[2].as_str(), args[3].as_str()) {
        ("output", "SHA256") => result_json(rdf_canon::canonicalize_quads_with::<Sha256>(
            &quads, &options,
        )),
        ("output", "SHA384") => result_json(rdf_canon::canonicalize_quads_with::<Sha384>(
            &quads, &options,
        )),
        ("map", "SHA256") => result_json(rdf_canon::issue_quads_with::<Sha256>(&quads, &options)),
        ("map", "SHA384") => result_json(rdf_canon::issue_quads_with::<Sha384>(&quads, &options)),
        _ => return Err("unsupported mode/hash".into()),
    };
    println!("{result}");
    Ok(())
}

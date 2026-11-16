//! Research-grade JSON reference bridge; not externally audited.
use std::io::Read;
fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let value: serde_json::Value = serde_json::from_str(&input).unwrap();
    let operation = value["operation"].as_str().unwrap();
    match sparq_zk_query_wasm::host_call(operation, &value["input"].to_string()) {
        Ok(value) => println!("{value}"),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

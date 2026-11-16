//! Research grade and not externally audited. Test-only supplied-cell program.
use std::io::{self, Read};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    println!(
        "{}",
        sparq_zk_query_wasm::host_call("testOperatorFixture", &input)?
    );
    Ok(())
}

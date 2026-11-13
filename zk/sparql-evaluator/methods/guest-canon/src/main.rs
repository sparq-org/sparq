// Regression image only; never accepted as an authenticated-RDF evaluator.
#![no_main]

risc0_zkvm::guest::entry!(main);

fn main() {
    let (text, sha384): (String, bool) = risc0_zkvm::guest::env::read();
    let output = sparq_canon::parse_nquads(&text)
        .and_then(|quads| {
            if sha384 {
                sparq_canon::canonicalize_quads_with::<sha2::Sha384>(&quads)
            } else {
                sparq_canon::canonicalize_quads_with::<sha2::Sha256>(&quads)
            }
        })
        .map_err(|error| error.to_string());
    risc0_zkvm::guest::env::commit(&output);
}

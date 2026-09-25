// [GPT-6] Private source authentication, parsing and SPARQL evaluation are proved.
#![no_main]

risc0_zkvm::guest::entry!(main);

use std::io::Read;

fn main() {
    use sparq_proved_evaluator_model as model;
    // Preserve the original V1 witness wire form: the request version is its
    // first word, not a newly inserted wrapper enum. Both versions have bounded
    // byte input before their typed deserialization and graph construction.
    let mut bytes = Vec::new();
    if risc0_zkvm::guest::env::stdin()
        .take((model::MAX_WITNESS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > model::MAX_WITNESS_BYTES
        || bytes.len() < 4
        || bytes.len() % 4 != 0
    {
        reject();
    }
    let version = u32::from_le_bytes(bytes[..4].try_into().unwrap_or_else(|_| reject()));
    match version {
        model::VERSION => {
            let witness: model::Witness =
                risc0_zkvm::serde::from_slice(&bytes).unwrap_or_else(|_| reject());
            canonical_input(
                &bytes,
                risc0_zkvm::serde::to_vec(&witness).unwrap_or_else(|_| reject()),
            );
            let journal = model::evaluate(&witness).unwrap_or_else(|_| reject());
            risc0_zkvm::guest::env::commit(&journal);
        }
        model::v2::VERSION => {
            let witness: model::v2::Witness =
                risc0_zkvm::serde::from_slice(&bytes).unwrap_or_else(|_| reject());
            canonical_input(
                &bytes,
                risc0_zkvm::serde::to_vec(&witness).unwrap_or_else(|_| reject()),
            );
            let journal = model::v2::evaluate(&witness).unwrap_or_else(|_| reject());
            risc0_zkvm::guest::env::commit(&journal);
        }
        _ => reject(),
    }
}

fn canonical_input(bytes: &[u8], words: Vec<u32>) {
    // SDK from_slice permits trailing input. Require exactly the typed witness
    // encoding so extra words, noncanonical padding and narrowing aliases cannot
    // become an alternative transport form. The raw input was already bounded.
    if words.len() != bytes.len() / 4
        || !words
            .iter()
            .zip(bytes.chunks_exact(4))
            .all(|(word, bytes)| word.to_le_bytes().as_slice() == bytes)
    {
        reject();
    }
}

fn reject() -> ! {
    risc0_zkvm::guest::abort("bounded exact-dataset relation rejected")
}

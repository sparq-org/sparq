// zkp-14.5: V5 credential authentication and evaluation are proved here.
// Rust guideline compliant 2026-02-21
#![no_main]

risc0_zkvm::guest::entry!(main);

use sparq_proved_evaluator_model::authenticated_rdf as auth;
use std::io::Read;

fn main() {
    // As in the V1-V3 wire forms, the request version is the first word. Input is
    // bounded by `auth::MAX_WITNESS_BYTES` before typed deserialization,
    // credential parsing or evaluation.
    let mut bytes = Vec::new();
    if risc0_zkvm::guest::env::stdin()
        .take((auth::MAX_WITNESS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > auth::MAX_WITNESS_BYTES
        || bytes.len() < 4
        || bytes.len() % 4 != 0
    {
        reject();
    }
    // V1-V3 belong to the exact guest image; this image accepts only V5.
    let version = u32::from_le_bytes(bytes[..4].try_into().unwrap_or_else(|_| reject()));
    if version != auth::VERSION {
        reject();
    }
    let witness: auth::Witness = risc0_zkvm::serde::from_slice(&bytes).unwrap_or_else(|_| reject());
    canonical_input(
        &bytes,
        risc0_zkvm::serde::to_vec(&witness).unwrap_or_else(|_| reject()),
    );
    // Parsing, canonicalization, table and signature checks, the agreed-anchor
    // comparison and V3 evaluation all run here; no host verdict is an input.
    #[cfg(not(feature = "phase-cycles"))]
    let journal = auth::evaluate(&witness).unwrap_or_else(|_| reject());
    #[cfg(feature = "phase-cycles")]
    let journal = observed(&witness);
    risc0_zkvm::guest::env::commit(&journal);
}

// Measurement image only: the cycle count after input decoding (`None`) and at
// the end of each phase, written to the host's stdout, never to the journal.
#[cfg(feature = "phase-cycles")]
fn observed(witness: &auth::Witness) -> auth::Journal {
    use risc0_zkvm::guest::env;
    let mut marks: Vec<(Option<auth::Phase>, u64)> = vec![(None, env::cycle_count())];
    let journal = auth::evaluate_observed(witness, &mut |phase| marks.push((Some(phase), env::cycle_count())))
        .unwrap_or_else(|_| reject());
    env::write(&marks);
    journal
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
    risc0_zkvm::guest::abort("bounded authenticated-RDF relation rejected")
}

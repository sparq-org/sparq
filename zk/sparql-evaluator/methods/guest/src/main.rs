// [GPT-6] Private source authentication, parsing and SPARQL evaluation are proved.
#![no_main]

risc0_zkvm::guest::entry!(main);

fn main() {
    let witness: sparq_proved_evaluator_model::Witness = risc0_zkvm::guest::env::read();
    match sparq_proved_evaluator_model::evaluate(&witness) {
        Ok(journal) => risc0_zkvm::guest::env::commit(&journal),
        Err(_) => panic!("bounded exact-dataset relation rejected"),
    }
}

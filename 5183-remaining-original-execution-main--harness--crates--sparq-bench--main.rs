// [GPT-6 ASTRA] Fixed-list driver for the unchanged original differential module.
#[path = "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5183/remaining-original-execution/source/original-update_fuzz.rs"]
mod update_fuzz;
fn main() {
    const SEEDS: [u64; 7] = [4141222535, 4141222571, 4141222576, 4141222599, 4141222612, 4141222617, 4141222622];
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("expected exactly one fixed-list seed");
        std::process::exit(2);
    }
    match args[1].parse::<u64>() {
        Ok(seed) if SEEDS.contains(&seed) => update_fuzz::run(seed, 1),
        _ => {
            eprintln!("seed is outside the declared fixed list");
            std::process::exit(2);
        }
    }
}

//! sq-xg6uu — corpus triage helper: print sparq-reason's NORMALIZED
//! canonical OWL 2 RL closure for one N-Triples file (the exact left-hand side the
//! differential test compares), so a divergence can be re-derived by eye while
//! updating the corpus or the disposition ledger.
//!
//! ```text
//! cargo run -p sparq-reason-diff --features rl-oracle --example dump_closure -- <file.nt>
//! ```

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_closure <file.nt>");
    let text = std::fs::read_to_string(&path).expect("readable input file");
    let lines = sparq_reason_diff::rl::sparq_rl_closure_lines(&text).expect("closure");
    for l in &lines {
        println!("{l}");
    }
    eprintln!("({} normalized triples)", lines.len());
}

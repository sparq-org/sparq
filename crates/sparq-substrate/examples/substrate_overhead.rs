//! Runnable zero-overhead DELTA harness — the substrate half of the
//! sparq-engine-systems paper's §8 protocol.
//!
//! # sq-atjue
//!
//! Measures each shared substrate kernel (`join` / `numeric` / `compare`) against a
//! hand-rolled pre-extraction equivalent and emits the house JSON envelope with one
//! `substrate.overhead_<kernel>` record per kernel. The number is a MEASUREMENT of
//! the paper's title-level "zero measured marginal overhead" claim, never an
//! assertion of it: if a kernel shows a non-negligible delta the honest disposition
//! (fixed in the paper before the measurement) is to report it as measured.
//!
//! Run:
//! ```text
//! # work box (non-canonical — PR body only, never a paper headline):
//! cargo run -p sparq-substrate --example substrate_overhead --features overhead --release
//!
//! # JSON envelope only:
//! cargo run -p sparq-substrate --example substrate_overhead --features overhead --release -- --json
//!
//! # CANONICAL run (a dedicated QUIET EC2 box, one process): mark it headline-eligible.
//! cargo run -p sparq-substrate --example substrate_overhead --features overhead --release -- \
//!     --canonical --reps 15 --json
//! ```
//!
//! `--canonical` sets `environment="canonical"` and `canonical=true` in the
//! envelope; a headline may cite the number ONLY from such a run on a dedicated
//! quiet host. Omitting it yields `environment="indicative"`.

use sparq_substrate::overhead::OverheadReport;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json_only = args.iter().any(|a| a == "--json");
    let canonical = args.iter().any(|a| a == "--canonical");

    // --reps N (default 7; the min-of-N is the robust fastest-path estimator).
    let reps = args
        .iter()
        .position(|a| a == "--reps")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(7);

    // --host "note" for the envelope provenance (default a generic marker).
    let host_note = args
        .iter()
        .position(|a| a == "--host")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| {
            if canonical {
                "canonical dedicated quiet EC2 box (pass --host to record the instance type)"
                    .to_string()
            } else {
                "non-canonical work box (indicative)".to_string()
            }
        });

    let environment = if canonical { "canonical" } else { "indicative" };
    let report = OverheadReport::run(reps, environment, &host_note);

    if json_only {
        print!("{}", report.to_json());
    } else {
        print!("{}", report.to_table());
        println!();
        println!("--- house JSON envelope (substrate.overhead_<kernel>; consume this, not the table) ---");
        print!("{}", report.to_json());
    }

    // A disagreement between a kernel's two implementations invalidates its ratio;
    // exit non-zero so a CI/canonical run cannot silently emit a bogus envelope.
    if !report.all_agree() {
        eprintln!(
            "ERROR: a kernel's substrate and hand-rolled results disagree; the overhead ratio is \
             INVALID. Do not cite this envelope."
        );
        std::process::exit(1);
    }
}

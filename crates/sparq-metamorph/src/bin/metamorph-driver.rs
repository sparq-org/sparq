//! Thin CLI over [`sparq_metamorph::run_window`] — the nightly metamorphic CI lane's
//! entry point (`.github/workflows/metamorph.yml`, bead `sq-3dyje.9`).
//!
//! ```text
//! metamorph-driver <seed-start> <seed-count> [--inject-filter-drops-row]
//! ```
//!
//! Exit code 0 iff every seed in the window passed both the TLP and NoREC oracles
//! (fail-closed: violations, engine failures, and an empty window all exit 1). The
//! `--inject-filter-drops-row` switch wraps the engine in the crate's seeded
//! wrong-result mutant to demonstrate the red path end-to-end; the standing lane
//! never sets it. All driver logic (and its tests) lives in the library's `harness`
//! module — this binary only parses arguments and maps the report to an exit code.

use std::process::ExitCode;

use sparq_metamorph::run_window;

fn usage() -> ExitCode {
    eprintln!("usage: metamorph-driver <seed-start> <seed-count> [--inject-filter-drops-row]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut inject = false;
    let mut positional = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "--inject-filter-drops-row" => inject = true,
            _ => positional.push(arg.as_str()),
        }
    }
    let (Some(start), Some(count), None) = (
        positional.first().and_then(|s| s.parse::<u64>().ok()),
        positional.get(1).and_then(|s| s.parse::<u64>().ok()),
        positional.get(2),
    ) else {
        return usage();
    };
    if inject {
        println!("NOTE: --inject-filter-drops-row set — the wrong-result mutant is active; this run MUST fail.");
    }
    let mut stdout = std::io::stdout().lock();
    match run_window(start, count, inject, &mut stdout) {
        Ok(report) if report.all_pass() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("metamorph-driver: io error writing the log: {e}");
            ExitCode::FAILURE
        }
    }
}

//! [OPUS-4.8] sq-ncvq.16 — `sparq-conformance-scoreboard`: emits the CONSOLIDATED
//! conformance scoreboard (stdout + `conformance-scoreboard.md`).
//!
//! ONE artifact listing every conformance suite the project ratchets — W3C
//! SPARQL, inference, W3C SHACL core, W3C SHACL-SPARQL, and OGC GeoSPARQL — each
//! with its ratchet floor and the CI job that enforces it. The per-suite detail
//! reports are still produced by their own runners (the SPARQL/inference binaries
//! here, the crate-local `cargo test` runners in sparq-shacl / sparq-geo); this
//! binary is the index that pulls them into a single view, closing the
//! drift-scanner's `conformance-split` finding (§5.E).
//!
//! Hermetic + fast: it renders the static central registry
//! ([`sparq_conformance::scoreboard::SUITES`]); it does NOT re-run the suites or
//! need any fetched test data. Always exits 0 (except on a write error).
//!
//! [FABLE-5] sq-gum8.14 — `--json FILE` additionally writes the MACHINE-READABLE
//! JSON scoreboard ([`sparq_conformance::scoreboard::scoreboard_json`]), the
//! committed `bench/conformance-scoreboard.generated.json` artifact that the
//! drift-guard test (`tests/scoreboard_export.rs`) byte-compares against a fresh
//! render. Regenerate it from the repo root with:
//!
//!   cargo run -p sparq-conformance --bin sparq-conformance-scoreboard -- \
//!     --report /tmp/conformance-scoreboard.md \
//!     --json bench/conformance-scoreboard.generated.json
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use sparq_conformance::scoreboard;
use std::path::PathBuf;

fn main() {
    let mut report_path = PathBuf::from("conformance-scoreboard.md");
    let mut json_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--report" => report_path = PathBuf::from(args.next().expect("--report needs a path")),
            "--json" => {
                json_path = Some(PathBuf::from(args.next().expect("--json needs a path")));
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: sparq-conformance-scoreboard [--report FILE] [--json FILE]");
                std::process::exit(2);
            }
        }
    }

    let md = scoreboard::render_scoreboard();
    print!("{md}");
    if let Err(e) = std::fs::write(&report_path, &md) {
        eprintln!("could not write {}: {e}", report_path.display());
    } else {
        eprintln!("\nscoreboard written to {}", report_path.display());
    }
    if let Some(json_path) = json_path {
        // A silently-missing JSON artifact would defeat the committed-generated
        // pattern (the drift guard byte-compares it), so a write error here is
        // fatal — unlike the best-effort markdown copy above.
        if let Err(e) = std::fs::write(&json_path, scoreboard::scoreboard_json()) {
            eprintln!("could not write {}: {e}", json_path.display());
            std::process::exit(1);
        }
        eprintln!(
            "machine-readable scoreboard written to {}",
            json_path.display()
        );
    }
}

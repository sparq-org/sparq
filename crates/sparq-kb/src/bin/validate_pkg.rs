//! Simple validation binary for PKG instances against SHACL shapes.
//!
//! Usage: `cargo run --bin validate-pkg --features validate -- <ttl-file>`
//!
//! Reads a Turtle file, validates it against the PKG SHACL shapes, and reports
//! the validation result. Exits with 0 if valid, 1 if invalid.
//!
//! sq-tzars.4 — PKG ingest workflow validation

use std::env;
use std::fs;
use std::process;

fn main() {
    #[cfg(not(feature = "validate"))]
    {
        eprintln!("error: this binary requires the 'validate' feature");
        process::exit(1);
    }

    #[cfg(feature = "validate")]
    {
        let args: Vec<String> = env::args().collect();
        if args.len() < 2 {
            eprintln!("usage: {} <ttl-file>", args.first().unwrap_or(&"validate-pkg".to_string()));
            process::exit(1);
        }

        let ttl_file = &args[1];
        let content = match fs::read_to_string(ttl_file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error reading {}: {}", ttl_file, e);
                process::exit(1);
            }
        };

        match sparq_kb::validate::validate_instances(&[&content]) {
            Ok(report) => {
                eprintln!(
                    "[validate-pkg] SHACL validation: {} violations (conforms={})",
                    report.results.len(),
                    report.conforms
                );
                if !report.results.is_empty() {
                    eprintln!("{}", report.to_text());
                }
                if report.conforms && report.results.is_empty() {
                    eprintln!("[validate-pkg] ✓ PKG instances conform to SHACL shapes");
                    process::exit(0);
                } else {
                    eprintln!("[validate-pkg] ✗ PKG instances FAILED SHACL validation");
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error validating {}: {}", ttl_file, e);
                process::exit(1);
            }
        }
    }
}

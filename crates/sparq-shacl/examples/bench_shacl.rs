//! (sq-7iai) SHACL validation benchmark runner — the G1 TSV-emitting
//! runner for `bench/shacl/`. It is the SHACL analogue of `sparq-cli bench`:
//! validate one data graph against a directory of shape graphs and emit, per
//! workload, the deterministic gate metrics + an advisory timing.
//!
//! ```sh
//! cargo run -p sparq-shacl --release --example bench_shacl -- <data.nt> <fmt> <shapes-dir> [iters]
//! ```
//!
//! Output: one TSV line per `*.ttl` in `<shapes-dir>` (sorted by workload name):
//!
//! ```text
//! <workload>\t<violations>\t<validate_us>\t<conforms>\t<focus_nodes>\t<load_us>
//! ```
//!
//! - `<workload>`     — the shape file's stem (e.g. `cardinality`).
//! - `<violations>`   — `report.results.len()` (primary deterministic gate; == the contract `count`).
//! - `<validate_us>`  — best-of-`iters` validate time in microseconds (ADVISORY timing).
//! - `<conforms>`     — `report.conforms` as 0/1 (deterministic gate).
//! - `<focus_nodes>`  — distinct focus nodes the shapes' targets select (deterministic gate).
//! - `<load_us>`      — data-graph parse+build time in microseconds (ADVISORY timing).
//!
//! `bench/shacl/run.sh` parses this, asserts the deterministic columns against
//! `bench/shacl/expected.tsv` (exit 1 on drift), and forwards the
//! `<workload>\t<violations>\t<validate_us>` 3-column contract to the ci-bench hook.
//! Correctness lives in `expected.tsv`, NOT in the binary — exactly like LUBM.

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (data_path, fmt, shapes_dir) = match (args.first(), args.get(1), args.get(2)) {
        (Some(d), Some(f), Some(s)) => (d.as_str(), f.as_str(), s.as_str()),
        _ => {
            eprintln!(
                "usage: bench_shacl <data-file> <format> <shapes-dir> [iters]\n\
                 emits per shapes/*.ttl: name\\tviolations\\tvalidate_us\\tconforms\\tfocus_nodes\\tload_us"
            );
            std::process::exit(2);
        }
    };
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    // Reject iters=0 explicitly: 0 makes both `0..iters` loops skip, leaving load_us/validate_us
    // at INFINITY and tripping `data.expect("iters >= 1")` — a non-deterministic footgun. The
    // gate contract requires finite `name\tcount\tus`, so demand a real sample count up front.
    if iters == 0 {
        eprintln!("error: iters must be >= 1 (got 0); need at least one sample for finite timings");
        std::process::exit(2);
    }

    // ---- load the data graph once (timed: the ADVISORY load metric) -------------------
    let data_text = std::fs::read_to_string(data_path)
        .unwrap_or_else(|e| panic!("read data {data_path}: {e}"));
    // Best-of-iters load so the advisory load_us is the least-noisy sample.
    let mut load_us = f64::INFINITY;
    let mut data = None;
    for _ in 0..iters {
        let t = Instant::now();
        let g = sparq_core::Graph::load_str(&data_text, fmt)
            .unwrap_or_else(|e| panic!("parse data {data_path}: {e}"));
        load_us = load_us.min(t.elapsed().as_secs_f64() * 1e6);
        data = Some(g);
    }
    let data = data.expect("iters >= 1");

    // ---- enumerate shapes/*.ttl (sorted, deterministic order) -------------------------
    let mut shape_files: Vec<std::path::PathBuf> = std::fs::read_dir(shapes_dir)
        .unwrap_or_else(|e| panic!("read shapes dir {shapes_dir}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ttl").unwrap_or(false))
        .collect();
    shape_files.sort();

    for path in &shape_files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let shapes_text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read shapes {path:?}: {e}"));
        let shapes = sparq_core::Graph::load_str(&shapes_text, "turtle")
            .unwrap_or_else(|e| panic!("parse shapes {path:?}: {e}"));
        // Parse the shapes model once; reuse it for the focus-node count and every
        // validate iteration (amortises shape parsing, mirroring validate_with_model).
        let model = sparq_shacl::ShapesModel::parse(&shapes);
        let focus_nodes = sparq_shacl::count_focus_nodes(&data, &model);

        let mut validate_us = f64::INFINITY;
        let mut violations = 0usize;
        let mut conforms = true;
        for _ in 0..iters {
            let t = Instant::now();
            let report = sparq_shacl::validate_with_model(&data, &model);
            validate_us = validate_us.min(t.elapsed().as_secs_f64() * 1e6);
            violations = report.results.len();
            conforms = report.conforms;
        }
        println!(
            "{name}\t{violations}\t{validate_us:.1}\t{}\t{focus_nodes}\t{load_us:.1}",
            u8::from(conforms),
        );
    }
}

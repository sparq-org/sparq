//! Throughput measurement for the README: pushes synthetic sensor readings
//! through a registered continuous query and reports triples/s and windows/s,
//! for each window-materialisation strategy ([`EvalMode`]).
//!
//! The LIBRARY never reads a clock; this harness (an example, not the crate)
//! times the run from outside. Run with:
//!
//! ```sh
//! cargo run --release -p sparq-rsp --example throughput
//! # strictly-additive machine-readable emit (STDOUT unchanged):
//! cargo run --release -p sparq-rsp --example throughput -- --json /tmp/rsp.json
//! ```
//!
//! [OPUS-4.8] (sq-cxji) `--json <path>` writes the SAME rows STDOUT prints — the
//! deterministic `windows`/`rows` counts plus the advisory throughput timings — as
//! a stable, DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON convention
//! of `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json` and
//! `crates/sparq-text/examples/bench_text.rs`. No serde dep is added (serde_json is a
//! TEST-only dev-dep used to parse the emit back). STDOUT is byte-for-byte unchanged
//! whether or not the flag is present; timings are ADVISORY + NON-CANONICAL (this dev
//! box) and nothing is committed.

use std::time::Instant;

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousQuery, EvalMode, WindowSpec, WindowedStream};

const N_TRIPLES: u64 = 1_000_000;
const N_SENSORS: u64 = 100;

/// One captured measurement row. `windows`/`rows` are DETERMINISTIC (a pure function
/// of the synthetic stream + window spec — runner-noise-immune); `mtriples_per_s` and
/// `windows_per_s` are best-effort throughput (ADVISORY, NON-CANONICAL on this box).
struct Row {
    label: String,
    mode: String,
    mtriples_per_s: f64,
    windows_per_s: f64,
    windows: u64,
    rows: u64,
    secs: f64,
}

fn reading(sensor: u64, v: u64) -> [Term; 3] {
    [
        NamedNode::new_unchecked(format!("http://ex/sensor/{sensor}")).into(),
        NamedNode::new_unchecked("http://ex/value").into(),
        Literal::from(v as i64).into(),
    ]
}

/// One scenario × one strategy: windows of `range`/`step`, one triple per tick.
fn run(label: &str, range: u64, step: u64, sparql: &str, mode: EvalMode) -> Row {
    let mut q = ContinuousQuery::register(sparql, WindowSpec::time(range, step))
        .expect("valid query")
        .with_mode(mode);
    let mut windows = 0u64;
    let mut rows = 0u64;
    let start = Instant::now();
    for ts in 0..N_TRIPLES {
        q.push(reading(ts % N_SENSORS, ts % 50), ts, |r| {
            windows += 1;
            rows += r.rows.len() as u64;
        })
        .expect("eval");
    }
    q.flush(|r| {
        windows += 1;
        rows += r.rows.len() as u64;
    })
    .expect("eval");
    let dt = start.elapsed().as_secs_f64();
    let mtriples_per_s = N_TRIPLES as f64 / dt / 1e6;
    let windows_per_s = windows as f64 / dt;
    println!(
        "{label:<48} {:<16} {mtriples_per_s:>7.2} Mtriples/s  {windows_per_s:>9.0} windows/s  ({windows} windows, {rows} rows, {dt:.2}s)",
        format!("{mode:?}"),
    );
    Row {
        label: label.to_string(),
        mode: format!("{mode:?}"),
        mtriples_per_s,
        windows_per_s,
        windows,
        rows,
        secs: dt,
    }
}

fn run_all_modes(label: &str, range: u64, step: u64, sparql: &str, out: &mut Vec<Row>) {
    for mode in [
        EvalMode::Rebuild,
        EvalMode::PersistentDict,
        EvalMode::Delta,
        EvalMode::Snapshot,
    ] {
        out.push(run(label, range, step, sparql, mode));
    }
    println!();
}

/// Bare parse cost of a registered query — the per-window work the
/// prepared-query seam removes (queries are parsed once at `register`, each
/// window executes the prepared algebra). Only matters at very high window
/// rates: see the RANGE 10 scenario.
fn run_parse_cost(label: &str, sparql: &str) {
    const N: u32 = 100_000;
    let start = Instant::now();
    for _ in 0..N {
        std::hint::black_box(sparq_engine::PreparedQuery::parse(sparql)).expect("valid query");
    }
    let ns = start.elapsed().as_nanos() as f64 / f64::from(N);
    println!(
        "parse cost ({label}): {ns:.0} ns/parse — saved per window by parse-once registration\n"
    );
}

/// Windowing only (no query): the bare S2R operator cost.
fn run_windowing_only(window_ticks: u64, out: &mut Vec<Row>) {
    let mut ws = WindowedStream::empty(WindowSpec::time(window_ticks, window_ticks));
    let mut windows = 0u64;
    let start = Instant::now();
    for ts in 0..N_TRIPLES {
        ws.push(reading(ts % N_SENSORS, ts % 50), ts);
        windows += ws.take_closed().len() as u64;
    }
    windows += ws.flush().len() as u64;
    let dt = start.elapsed().as_secs_f64();
    let mtriples_per_s = N_TRIPLES as f64 / dt / 1e6;
    let windows_per_s = windows as f64 / dt;
    let label = format!("windowing only (no query), RANGE {window_ticks}");
    println!(
        "{label:<48} {:<16} {mtriples_per_s:>7.2} Mtriples/s  {windows_per_s:>9.0} windows/s  ({windows} windows, {dt:.2}s)\n",
        "-",
    );
    out.push(Row {
        label,
        mode: "-".to_string(),
        mtriples_per_s,
        windows_per_s,
        windows,
        rows: 0,
        secs: dt,
    });
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-cxji) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// Extracts `--json <path>` (and its value) from argv, returning argv WITHOUT the
/// flag pair so any positional parsing is unchanged. A bare `--json` is a usage
/// error (exit 2), mirroring `sparq-cli` / `bench_text`'s identical flag.
fn take_json_flag(args: Vec<String>) -> (Vec<String>, Option<String>) {
    let mut out = Vec::with_capacity(args.len());
    let mut json_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--json" {
            match args.get(i + 1) {
                Some(p) => {
                    json_path = Some(p.clone());
                    i += 2;
                    continue;
                }
                None => {
                    eprintln!("`--json` requires a path argument: --json <path>");
                    std::process::exit(2);
                }
            }
        }
        out.push(args[i].clone());
        i += 1;
    }
    (out, json_path)
}

/// Minimal JSON string escaper (the dependency-free emit). Workload labels are static
/// ASCII, so this covers the realistic input; anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialises the run to stable, dependency-free JSON. Each `rows` object carries the
/// SAME fields the STDOUT line prints; `windows`/`rows` are the DETERMINISTIC counts,
/// the throughput numbers are ADVISORY + NON-CANONICAL (stated in `note`).
fn results_json(rows: &[Row]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-rsp throughput\",\n");
    s.push_str(&format!("  \"n_triples\": {N_TRIPLES},\n"));
    s.push_str(&format!("  \"n_sensors\": {N_SENSORS},\n"));
    s.push_str(
        "  \"note\": \"`windows`/`rows` are DETERMINISTIC (a pure function of the synthetic \
         stream + window spec); `mtriples_per_s`/`windows_per_s`/`secs` are best-effort \
         throughput MEASURED on the running host — ADVISORY, NON-CANONICAL (this dev box) — \
         do not bake into committed files\",\n",
    );
    s.push_str("  \"rows\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 < rows.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"label\": {}, \"mode\": {}, \"windows\": {}, \"rows\": {}, \
             \"mtriples_per_s\": {:.4}, \"windows_per_s\": {:.1}, \"secs\": {:.4} }}{comma}\n",
            json_str(&r.label),
            json_str(&r.mode),
            r.windows,
            r.rows,
            r.mtriples_per_s,
            r.windows_per_s,
            r.secs,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());

    println!("{N_TRIPLES} triples, {N_SENSORS} sensors, 1 triple/tick, time windows\n");
    let mut rows: Vec<Row> = Vec::new();
    run_windowing_only(1_000, &mut rows);
    let avg = "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }";
    run_parse_cost("AVG", avg);
    run_all_modes(
        "AVG, RANGE 10 STEP 10 (tumbling, parse-sensitive)",
        10,
        10,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG, RANGE 100 STEP 100 (tumbling)",
        100,
        100,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG, RANGE 1000 STEP 1000 (tumbling)",
        1_000,
        1_000,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG, RANGE 10000 STEP 10000 (tumbling)",
        10_000,
        10_000,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG, RANGE 1000 STEP 100 (sliding 10x overlap)",
        1_000,
        100,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG, RANGE 10000 STEP 1000 (sliding 10x overlap)",
        10_000,
        1_000,
        avg,
        &mut rows,
    );
    run_all_modes(
        "AVG per sensor (GROUP BY), RANGE 1000 STEP 1000",
        1_000,
        1_000,
        "SELECT ?s (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v } GROUP BY ?s",
        &mut rows,
    );

    // [OPUS-4.8] (sq-cxji) Strictly-additive JSON emit: only when `--json <path>` was
    // given. STDOUT above is the unchanged human/README table.
    if let Some(path) = json_path {
        let doc = results_json(&rows);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} result rows to {path}", rows.len());
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-cxji) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["throughput", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["throughput"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        // Absent flag -> argv unchanged, no path.
        let plain: Vec<String> = ["throughput"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("Rebuild"), "\"Rebuild\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let rows = vec![
            Row {
                label: "windowing only".into(),
                mode: "-".into(),
                mtriples_per_s: 12.5,
                windows_per_s: 999.0,
                windows: 1000,
                rows: 0,
                secs: 0.08,
            },
            Row {
                label: "AVG tumbling".into(),
                mode: "Rebuild".into(),
                mtriples_per_s: 3.25,
                windows_per_s: 42.0,
                windows: 100,
                rows: 100,
                secs: 0.3,
            },
        ];
        let doc = results_json(&rows);
        // The dependency-free emit must round-trip through a REAL serde_json parse —
        // this catches a malformed document (e.g. a missing inter-row comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-rsp throughput");
        assert_eq!(v["n_triples"], N_TRIPLES);
        assert_eq!(v["n_sensors"], N_SENSORS);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        let arr = v["rows"].as_array().expect("rows is an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["label"], "windowing only");
        assert_eq!(arr[0]["mode"], "-");
        assert_eq!(arr[0]["windows"], 1000);
        assert_eq!(arr[0]["rows"], 0);
        assert!(arr[0]["mtriples_per_s"].is_number());
        assert_eq!(arr[1]["label"], "AVG tumbling");
        assert_eq!(arr[1]["mode"], "Rebuild");
        assert_eq!(arr[1]["windows"], 100);
        assert_eq!(arr[1]["rows"], 100);
    }
}

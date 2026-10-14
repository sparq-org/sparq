//! Build-time + output measurement for `sparq-introspect` on the real benchmark
//! datasets (the performance gate from `research/genai-design.md` §4):
//!
//! - olympics (1.78M triples): `bench/qlever-olympics/olympics.nt`, override with
//!   `SPARQ_OLYMPICS_NT`;
//! - qlever-synthetic (8M triples, optional): `bench/qlever-synthetic/synthetic.nt`,
//!   override with `SPARQ_SYNTHETIC_NT` — measured when present.
//!
//! Reported per dataset: load time, `Introspection::build` time, `to_json` /
//! `to_text_summary` time and sizes, plus the first part of the text summary.
//!
//! Run: `cargo run -p sparq-introspect --example olympics_introspect --release`
//!
//! (sq-cxji) `--json <path>` writes the SAME measurements STDOUT prints —
//! the deterministic structural counts (triples, subjects, classes, predicates,
//! characteristic sets, namespaces, byte sizes) plus the advisory build/serialise
//! timings — as a stable, DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON
//! convention of `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`. (The crate
//! already depends on serde_json for `Introspection::to_json`, but the bench emit is
//! deliberately hand-built so the convention matches the other harnesses.) STDOUT is
//! byte-for-byte unchanged whether or not the flag is present; timings are ADVISORY +
//! NON-CANONICAL (this dev box) and nothing is committed.

use sparq_core::Graph;
use sparq_introspect::Introspection;
use std::time::Instant;

/// One dataset's measured row (the per-dataset emit object).
struct DatasetResult {
    name: String,
    path: String,
    triples: usize,
    load_secs: f64,
    build_secs: f64,
    subjects: u64,
    classes: usize,
    predicates: usize,
    characteristic_sets: u64,
    namespaces: u64,
    json_ms: f64,
    json_bytes: usize,
    summary_ms: f64,
    summary_chars: usize,
}

fn path_of(env: &str, rel: &str) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var(env) {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    p.exists().then_some(p)
}

fn measure(name: &str, path: &std::path::Path) -> DatasetResult {
    println!("==== {name} ({}) ====", path.display());
    let t0 = Instant::now();
    let text = std::fs::read_to_string(path).expect("read dataset");
    let g = Graph::load_str(&text, "ntriples").expect("parse dataset");
    drop(text);
    let load_secs = t0.elapsed().as_secs_f64();
    let triples = g.len();
    println!("loaded {triples} triples in {load_secs:.2}s");

    let t1 = Instant::now();
    let ix = Introspection::build(&g);
    let build_secs = t1.elapsed().as_secs_f64();
    println!(
        "Introspection::build: {build_secs:.2}s — {} subjects, {} classes, {} predicates, {} characteristic sets, {} namespaces",
        ix.subjects,
        ix.classes.len(),
        ix.predicates.len(),
        ix.characteristic_sets.distinct,
        ix.vocabularies.distinct
    );

    let t2 = Instant::now();
    let json = ix.to_json();
    let json_ms = t2.elapsed().as_secs_f64() * 1000.0;
    let json_bytes = json.len();
    println!("to_json: {json_ms:.0}ms, {json_bytes} bytes");

    let t3 = Instant::now();
    let summary = ix.to_text_summary(2500);
    let summary_ms = t3.elapsed().as_secs_f64() * 1000.0;
    let summary_chars = summary.chars().count();
    println!("to_text_summary(2500): {summary_ms:.1}ms, {summary_chars} chars");
    println!("---- text summary (budget 2500) ----\n{summary}");

    DatasetResult {
        name: name.to_string(),
        path: path.display().to_string(),
        triples,
        load_secs,
        build_secs,
        subjects: ix.subjects,
        classes: ix.classes.len(),
        predicates: ix.predicates.len(),
        characteristic_sets: ix.characteristic_sets.distinct,
        namespaces: ix.vocabularies.distinct,
        json_ms,
        json_bytes,
        summary_ms,
        summary_chars,
    }
}

// ---------------------------------------------------------------------------
// (sq-cxji) --json <path> machine-readable results emit
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

/// Minimal JSON string escaper (the dependency-free emit). Dataset names + paths are
/// ASCII in practice; anything else still yields valid `\uXXXX` escapes.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialises the per-dataset measurements to stable, dependency-free JSON. The
/// structural counts + byte sizes are DETERMINISTIC (pure functions of the dataset);
/// the `*_secs`/`*_ms` timings are wall-clock — ADVISORY + NON-CANONICAL (this dev
/// box), stated in `note`.
fn results_json(rows: &[DatasetResult]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-introspect olympics_introspect\",\n");
    s.push_str(
        "  \"note\": \"structural counts (`triples`/`subjects`/`classes`/`predicates`/\
         `characteristic_sets`/`namespaces`) and byte sizes are DETERMINISTIC (pure \
         functions of the dataset); `*_secs`/`*_ms` are wall-clock MEASURED on the running \
         host — ADVISORY, NON-CANONICAL (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str("  \"datasets\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 < rows.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"name\": {}, \"path\": {}, \"triples\": {}, \"subjects\": {}, \
             \"classes\": {}, \"predicates\": {}, \"characteristic_sets\": {}, \
             \"namespaces\": {}, \"json_bytes\": {}, \"summary_chars\": {}, \
             \"load_secs\": {:.4}, \"build_secs\": {:.4}, \"json_ms\": {:.2}, \
             \"summary_ms\": {:.2} }}{comma}\n",
            json_str(&r.name),
            json_str(&r.path),
            r.triples,
            r.subjects,
            r.classes,
            r.predicates,
            r.characteristic_sets,
            r.namespaces,
            r.json_bytes,
            r.summary_chars,
            r.load_secs,
            r.build_secs,
            r.json_ms,
            r.summary_ms,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());

    let mut rows: Vec<DatasetResult> = Vec::new();
    match path_of(
        "SPARQ_OLYMPICS_NT",
        "../../bench/qlever-olympics/olympics.nt",
    ) {
        Some(p) => rows.push(measure("olympics 1.78M", &p)),
        None => eprintln!("olympics.nt not found (set SPARQ_OLYMPICS_NT) — skipping"),
    }
    match path_of(
        "SPARQ_SYNTHETIC_NT",
        "../../bench/qlever-synthetic/synthetic.nt",
    ) {
        Some(p) => rows.push(measure("qlever-synthetic 8M", &p)),
        None => eprintln!("synthetic.nt not found (set SPARQ_SYNTHETIC_NT) — skipping"),
    }

    // (sq-cxji) Strictly-additive JSON emit: only when `--json <path>` was
    // given. STDOUT above is the unchanged human report. An empty `datasets` array
    // (neither dataset present) is still valid JSON, so the file is always written.
    if let Some(path) = json_path {
        let doc = results_json(&rows);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {} dataset result(s) to {path}", rows.len());
    }
}

// ---------------------------------------------------------------------------
// (sq-cxji) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, triples: usize) -> DatasetResult {
        DatasetResult {
            name: name.to_string(),
            path: format!("/data/{name}.nt"),
            triples,
            load_secs: 1.25,
            build_secs: 0.5,
            subjects: 100,
            classes: 6,
            predicates: 40,
            characteristic_sets: 12,
            namespaces: 8,
            json_ms: 3.5,
            json_bytes: 2048,
            summary_ms: 0.75,
            summary_chars: 2400,
        }
    }

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["olympics_introspect", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["olympics_introspect"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["olympics_introspect"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("olympics 1.78M"), "\"olympics 1.78M\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let rows = vec![
            sample("olympics", 1_780_000),
            sample("synthetic", 8_000_000),
        ];
        let doc = results_json(&rows);
        // The dependency-free emit must round-trip through a REAL serde_json parse —
        // this catches a malformed document (e.g. a missing inter-row comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-introspect olympics_introspect");
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        let arr = v["datasets"].as_array().expect("datasets is an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "olympics");
        assert_eq!(arr[0]["triples"], 1_780_000);
        assert_eq!(arr[0]["classes"], 6);
        assert_eq!(arr[0]["characteristic_sets"], 12);
        assert!(arr[0]["build_secs"].is_number());
        assert_eq!(arr[1]["name"], "synthetic");
        assert_eq!(arr[1]["triples"], 8_000_000);
    }

    #[test]
    fn empty_datasets_is_valid_json() {
        // Neither dataset present -> an empty `datasets` array is still valid JSON.
        let doc = results_json(&[]);
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["datasets"].as_array().unwrap().len(), 0);
    }
}

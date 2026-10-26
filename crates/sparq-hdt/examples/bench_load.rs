//! Load-throughput sketch: HDT archive vs gzipped N-Triples on ~1M synthetic triples.
//!
//! HDT's win is the compact on-disk size plus no-text-parse loading: the dictionary
//! is decompressed term-by-term (each distinct term once) and the triples arrive as
//! ids, versus tokenizing + interning ~100 MB of N-Triples text.
//!
//! Usage (generates `crates/sparq-hdt/bench-data/` on first run, gitignored):
//! ```sh
//! cargo run --release -p sparq-hdt --example bench_load
//! # strictly-additive machine-readable emit (STDOUT unchanged):
//! cargo run --release -p sparq-hdt --example bench_load -- --json /tmp/hdt.json
//! ```
//!
//! (sq-cxji) `--json <path>` writes the SAME measurements STDOUT prints —
//! the deterministic file sizes + triple count, plus the advisory best-of-RUNS load
//! times — as a stable, DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON
//! convention of `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json` and
//! `crates/sparq-text/examples/bench_text.rs`. No serde dep is added (serde_json is a
//! TEST-only dev-dep used to parse the emit back). STDOUT is byte-for-byte unchanged
//! whether or not the flag is present; the timings are ADVISORY + NON-CANONICAL (this
//! dev box) and nothing is committed.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

const N_TRIPLES: usize = 1_000_000;
const RUNS: usize = 3;

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

/// Serialises the measured load comparison to stable, dependency-free JSON. The byte
/// sizes + triple count are DETERMINISTIC (a pure function of the seeded corpus); the
/// `*_load_secs` / `*_triples_per_s` / `speedup` numbers are best-of-RUNS wall-clock —
/// ADVISORY + NON-CANONICAL (this dev box), stated in `note`. No serde dependency.
fn results_json(
    nt_bytes: u64,
    gz_bytes: u64,
    hdt_bytes: u64,
    triples: usize,
    hdt_secs: f64,
    gz_secs: f64,
) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-hdt bench_load\",\n");
    s.push_str(&format!("  \"triples\": {triples},\n"));
    s.push_str(&format!("  \"runs\": {RUNS},\n"));
    s.push_str(
        "  \"note\": \"file `*_bytes` + `triples` are DETERMINISTIC (a pure function of the \
         seeded synthetic corpus); `*_load_secs`/`*_triples_per_s`/`speedup` are best-of-runs \
         wall-clock MEASURED on the running host — ADVISORY, NON-CANONICAL (this dev box) — \
         do not bake into committed files\",\n",
    );
    s.push_str(&format!("  \"nt_bytes\": {nt_bytes},\n"));
    s.push_str(&format!("  \"gz_bytes\": {gz_bytes},\n"));
    s.push_str(&format!("  \"hdt_bytes\": {hdt_bytes},\n"));
    s.push_str(&format!("  \"hdt_load_secs\": {hdt_secs:.4},\n"));
    s.push_str(&format!(
        "  \"hdt_triples_per_s\": {:.1},\n",
        triples as f64 / hdt_secs
    ));
    s.push_str(&format!("  \"gz_load_secs\": {gz_secs:.4},\n"));
    s.push_str(&format!(
        "  \"gz_triples_per_s\": {:.1},\n",
        triples as f64 / gz_secs
    ));
    s.push_str(&format!("  \"speedup\": {:.4}\n", gz_secs / hdt_secs));
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench-data");
    std::fs::create_dir_all(&dir).unwrap();
    let nt_path = dir.join("bench.nt");
    let gz_path = dir.join("bench.nt.gz");
    let hdt_path = dir.join("bench.hdt");

    if !nt_path.exists() || !gz_path.exists() || !hdt_path.exists() {
        generate(&nt_path, &gz_path, &hdt_path);
    }

    let size = |p: &Path| std::fs::metadata(p).unwrap().len();
    let (nt_bytes, gz_bytes, hdt_bytes) = (size(&nt_path), size(&gz_path), size(&hdt_path));
    println!("file sizes:");
    println!("  bench.nt     {nt_bytes:>10} bytes");
    println!("  bench.nt.gz  {gz_bytes:>10} bytes");
    println!("  bench.hdt    {hdt_bytes:>10} bytes");

    // HDT -> Graph.
    let mut best_hdt = f64::MAX;
    let mut n = 0;
    for _ in 0..RUNS {
        let t = Instant::now();
        let g = sparq_hdt::load(&hdt_path).unwrap();
        let dt = t.elapsed().as_secs_f64();
        n = g.store().len();
        best_hdt = best_hdt.min(dt);
    }
    println!(
        "HDT    load: {n} triples in {best_hdt:.3}s  ({:.0} triples/s)",
        n as f64 / best_hdt
    );

    // .nt.gz -> Graph (streaming decompress + parse).
    let mut best_gz = f64::MAX;
    for _ in 0..RUNS {
        let t = Instant::now();
        let f = std::fs::File::open(&gz_path).unwrap();
        let g =
            sparq_core::Graph::load_reader(flate2::read::GzDecoder::new(f), "ntriples").unwrap();
        let dt = t.elapsed().as_secs_f64();
        assert_eq!(g.store().len(), n);
        best_gz = best_gz.min(dt);
    }
    println!(
        ".nt.gz load: {n} triples in {best_gz:.3}s  ({:.0} triples/s)",
        n as f64 / best_gz
    );
    println!("speedup: {:.2}x", best_gz / best_hdt);

    // (sq-cxji) Strictly-additive JSON emit: only when `--json <path>` was
    // given. STDOUT above is the unchanged human table.
    if let Some(path) = json_path {
        let doc = results_json(nt_bytes, gz_bytes, hdt_bytes, n, best_hdt, best_gz);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote load comparison to {path}");
    }
}

/// Deterministic synthetic graph: clustered subjects, a small predicate set, and a
/// mix of IRI objects (drawn from the subject pool, exercising HDT's shared
/// section) and plain / language-tagged / numeric literals.
fn generate(nt_path: &Path, gz_path: &Path, hdt_path: &Path) {
    eprintln!("generating {N_TRIPLES} synthetic triples (one-time)...");
    let mut nt = String::with_capacity(N_TRIPLES * 100);
    let mut st = 0x243f6a88u64;
    let mut rng = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };
    for _ in 0..N_TRIPLES {
        let s = rng() % 100_000;
        let p = rng() % 50;
        nt.push_str(&format!(
            "<http://bench.example/e{s}> <http://bench.example/vocab/p{p}> "
        ));
        match rng() % 5 {
            0 => nt.push_str(&format!("<http://bench.example/e{}>", rng() % 100_000)),
            1 => nt.push_str(&format!("\"label {}\"", rng() % 1_000_000)),
            2 => nt.push_str(&format!("\"etikett {}\"@de", rng() % 1_000_000)),
            3 => nt.push_str(&format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                rng() % 100_000
            )),
            _ => nt.push_str(&format!(
                "\"{}.{:02}\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                rng() % 1000,
                rng() % 100
            )),
        }
        nt.push_str(" .\n");
    }
    std::fs::write(nt_path, &nt).unwrap();

    let gz_file = std::fs::File::create(gz_path).unwrap();
    let mut enc = flate2::write::GzEncoder::new(
        std::io::BufWriter::new(gz_file),
        flate2::Compression::default(),
    );
    enc.write_all(nt.as_bytes()).unwrap();
    enc.finish().unwrap().flush().unwrap();

    eprintln!("converting to HDT (one-time)...");
    let hdt = hdt::Hdt::read_nt(nt_path).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(hdt_path).unwrap());
    hdt.write(&mut out).unwrap();
}

// ---------------------------------------------------------------------------
// (sq-cxji) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["bench_load", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["bench_load"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        // Absent flag -> argv unchanged, no path.
        let plain: Vec<String> = ["bench_load"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn results_json_shape_and_keys() {
        let doc = results_json(100_000_000, 20_000_000, 12_000_000, 1_000_000, 0.25, 1.5);
        // The dependency-free emit must round-trip through a REAL serde_json parse —
        // this catches a malformed document (e.g. a missing comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-hdt bench_load");
        assert_eq!(v["triples"], 1_000_000);
        assert_eq!(v["runs"], RUNS);
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        assert_eq!(v["nt_bytes"], 100_000_000u64);
        assert_eq!(v["gz_bytes"], 20_000_000u64);
        assert_eq!(v["hdt_bytes"], 12_000_000u64);
        assert!((v["hdt_load_secs"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        assert!((v["gz_load_secs"].as_f64().unwrap() - 1.5).abs() < 1e-9);
        // speedup = gz_secs / hdt_secs = 1.5 / 0.25 = 6.0.
        assert!((v["speedup"].as_f64().unwrap() - 6.0).abs() < 1e-9);
        // triples_per_s = triples / secs.
        assert!((v["hdt_triples_per_s"].as_f64().unwrap() - 4_000_000.0).abs() < 1.0);
    }
}

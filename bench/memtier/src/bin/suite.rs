//! RESEARCH SPIKE — open an index dir (mmap), optionally eager-decompress, run a query
//! battery, and report: open time, heap breakdown, RSS at each stage, per-query
//! latency (min/median/max of N), and per-file page-cache residency after the battery.
//!
//! usage: suite <index-dir> <queries-dir> [iters] [decompress] [hist]
//!
//! Build twice for the perm-dropping comparison:
//!   cargo build --release                      # 6 permutations searched
//!   cargo build --release --features compact   # 3 permutations (the wasm set)

use memtier_spikes::{file_residency, residency_histogram, rss_kb, run_suite_mode};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (dir, qdir) = match (args.get(1), args.get(2)) {
        (Some(d), Some(q)) => (d.clone(), q.clone()),
        _ => {
            eprintln!("usage: suite <index-dir> <queries-dir> [iters] [decompress] [hist]");
            std::process::exit(2);
        }
    };
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    let decompress = args.iter().any(|a| a == "decompress");
    let hist = args.iter().any(|a| a == "hist");
    let mode = if args.iter().any(|a| a == "json") { "json" } else { "count" };
    let perm_names = ["SPO", "SOP", "PSO", "POS", "OSP", "OPS"];

    println!(
        "config\tperms={}\titers={iters}\tdecompress={decompress}\tmode={mode}",
        if cfg!(feature = "compact") { 3 } else { 6 }
    );
    println!("rss_kb\tstart\t{}", rss_kb());

    let t = Instant::now();
    let mut g = sparq_core::Graph::open(std::path::Path::new(&dir)).unwrap_or_else(|e| {
        eprintln!("open error: {e}");
        std::process::exit(1);
    });
    println!("open_s\t{:.3}", t.elapsed().as_secs_f64());
    println!("triples\t{}", g.len());
    println!("heap_bytes\ttotal_after_open\t{}", g.heap_bytes());
    println!("heap_bytes\tdict_after_open\t{}", g.dict().heap_bytes());
    println!("rss_kb\tafter_open\t{}", rss_kb());

    if decompress {
        let t = Instant::now();
        g.decompress_indexes();
        println!("decompress_s\t{:.3}", t.elapsed().as_secs_f64());
        println!("heap_bytes\ttotal_after_decompress\t{}", g.heap_bytes());
        println!("rss_kb\tafter_decompress\t{}", rss_kb());
    }

    let results = run_suite_mode(&g, &qdir, iters, mode);
    let mut total_min = 0.0;
    for r in &results {
        match &r.err {
            None => {
                println!(
                    "query\t{}\t{}\t{:.1}\t{:.1}\t{:.1}",
                    r.name,
                    r.rows.unwrap_or(0),
                    r.min_us,
                    r.median_us,
                    r.max_us
                );
                total_min += r.min_us;
            }
            Some(e) => println!("query\t{}\tERROR\t{e}", r.name),
        }
    }
    println!("suite_total_min_us\t{total_min:.1}");
    println!("rss_kb\tafter_battery\t{}", rss_kb());
    println!("heap_bytes\ttotal_after_battery\t{}", g.heap_bytes());

    // Per-file page-cache residency: which permutations (and dict/numeric files) the
    // battery actually paged in. The differential between touched and untouched perm
    // files is both the hot/cold signal and the validation of the technique.
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64;
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        match file_residency(&f) {
            Ok((res, tot)) if tot > 0 => {
                let label = name
                    .strip_prefix("perm")
                    .and_then(|s| s.strip_suffix(".bin"))
                    .and_then(|s| s.parse::<usize>().ok())
                    .map(|i| format!("{name}({})", perm_names[i]))
                    .unwrap_or(name);
                println!(
                    "residency\t{label}\t{res}\t{tot}\t{:.1}%\t{:.1} MB resident",
                    100.0 * res as f64 / tot as f64,
                    res as f64 * page / 1e6
                );
                if hist && label.starts_with("perm") {
                    let h = residency_histogram(&f, 20).unwrap();
                    let s: Vec<String> = h.iter().map(|v| format!("{:.0}", v * 100.0)).collect();
                    println!("residency_hist\t{label}\t{}", s.join(","));
                }
            }
            Ok(_) => {}
            Err(e) => println!("residency\t{name}\tERROR\t{e}"),
        }
    }
}

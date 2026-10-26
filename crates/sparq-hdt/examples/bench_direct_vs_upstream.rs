//! A/B throughput sketch: the direct sparq-side decoder (default
//! `load_reader`, plan levers H1–H4) vs the upstream-backed oracle
//! (`load_reader_via_upstream`, i.e. `hdt::Hdt::read` + per-id translation) on the
//! same in-memory HDT bytes. Median-of-N, both producing identical graphs.
//!
//! HEADLINE NUMBERS ARE NOT TAKEN HERE: the orchestrator re-measures MB/s + peak
//! RSS on a quiet box. This is a structural A/B (same machine, same bytes, same
//! process) to show the direction; absolute numbers on a contended box are noise.
//!
//! Usage: `cargo run --release -p sparq-hdt --example bench_direct_vs_upstream [N]`
//! (N = triple count, default 1_000_000; data is in-memory, nothing written).

use std::time::Instant;

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let runs = 3;

    eprintln!("generating {n} synthetic triples + HDT (in-memory, one-time)...");
    let bytes = generate_hdt_bytes(n);
    eprintln!(
        "hdt bytes: {} ({:.1} MiB)",
        bytes.len(),
        bytes.len() as f64 / (1024.0 * 1024.0)
    );

    // Confirm identical output before timing.
    let g_direct = sparq_hdt::load_reader(std::io::Cursor::new(bytes.clone())).unwrap();
    let g_upstream =
        sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(bytes.clone())).unwrap();
    assert_eq!(
        g_direct.store().len(),
        g_upstream.store().len(),
        "graphs must match"
    );
    let triples = g_direct.store().len();
    println!("loaded {triples} triples (direct == upstream store len)\n");

    let mut best_direct = f64::MAX;
    let mut best_upstream = f64::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        let g = sparq_hdt::load_reader(std::io::Cursor::new(bytes.clone())).unwrap();
        std::hint::black_box(&g);
        best_direct = best_direct.min(t.elapsed().as_secs_f64());

        let t = Instant::now();
        let g = sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(bytes.clone())).unwrap();
        std::hint::black_box(&g);
        best_upstream = best_upstream.min(t.elapsed().as_secs_f64());
    }

    println!("CONTENDED / PROVISIONAL (orchestrator re-measures on a quiet box):");
    println!(
        "  direct   (H1–H4): {best_direct:.4}s  ({:.0} triples/s)",
        triples as f64 / best_direct
    );
    println!(
        "  upstream (oracle): {best_upstream:.4}s  ({:.0} triples/s)",
        triples as f64 / best_upstream
    );
    println!("  speedup: {:.2}x", best_upstream / best_direct);
}

/// Deterministic synthetic graph -> HDT bytes, mirroring `bench_load.rs`'s shape
/// (clustered subjects, small predicate set, shared-section IRI objects + a mix of
/// literal kinds) so the dictionary has realistic term reuse across PFC blocks.
fn generate_hdt_bytes(n: usize) -> Vec<u8> {
    use std::fmt::Write;
    let mut nt = String::with_capacity(n * 100);
    let mut st = 0x243f6a88u64;
    let mut rng = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };
    for _ in 0..n {
        let s = rng() % 100_000;
        let p = rng() % 50;
        let _ = write!(
            nt,
            "<http://bench.example/e{s}> <http://bench.example/vocab/p{p}> "
        );
        match rng() % 5 {
            0 => {
                let _ = write!(nt, "<http://bench.example/e{}>", rng() % 100_000);
            }
            1 => {
                let _ = write!(nt, "\"label {}\"", rng() % 1_000_000);
            }
            2 => {
                let _ = write!(nt, "\"etikett {}\"@de", rng() % 1_000_000);
            }
            3 => {
                let _ = write!(
                    nt,
                    "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                    rng() % 100_000
                );
            }
            _ => {
                let _ = write!(
                    nt,
                    "\"{}.{:02}\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                    rng() % 1000,
                    rng() % 100
                );
            }
        }
        nt.push_str(" .\n");
    }
    // N-Triples -> HDT via the (test/dev) writer, then serialize to bytes.
    let dir = std::env::temp_dir().join("sparq-hdt-bench-ab");
    std::fs::create_dir_all(&dir).unwrap();
    let nt_path = dir.join("ab.nt");
    std::fs::write(&nt_path, &nt).unwrap();
    let hdt = hdt::Hdt::read_nt(&nt_path).unwrap();
    let mut buf = Vec::new();
    hdt.write(&mut buf).unwrap();
    let _ = std::fs::remove_file(&nt_path); // clean /tmp scratch (disk discipline)
    buf
}

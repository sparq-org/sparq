//! In-crate micro-bench for the generated SHACL-CS parsers +
//! residual printer (gate G1 registration; see `bench/benchmarks.toml`
//! `sparq-shaclc`). Deterministic synthetic corpus generated in-process;
//! prints `metric_us` lines (min-of-3), NOT a committed regression number.
//!
//! usage: cargo run -p sparq-shaclc --release --example bench_shaclc -- [shapes=2000]

use sparq_shaclc::{parse, write, Profile, DEFAULT_BASE};

/// One representative shape per index: target, counts, nodeKind, datatype
/// atom, or-chain, nested not — the constructs the corpus exercises most.
fn corpus(shapes: usize) -> String {
    let mut doc = String::with_capacity(shapes * 200 + 64);
    doc.push_str("PREFIX ex: <http://example.org/bench#>\n\n");
    for i in 0..shapes {
        doc.push_str(&format!(
            "shape ex:S{i} -> ex:C{i} {{\n\
             \tex:name{i} [1..3] xsd:string minLength=2 .\n\
             \tex:kind{i} IRI .\n\
             \tex:val{i} class=ex:T{i}|!datatype=xsd:integer .\n\
             }}\n\n"
        ));
    }
    doc
}

fn min_of_3_us(mut f: impl FnMut() -> u64) -> (f64, u64) {
    let mut best = f64::INFINITY;
    let mut count = 0;
    for _ in 0..3 {
        let t0 = std::time::Instant::now();
        count = f();
        best = best.min(t0.elapsed().as_secs_f64() * 1e6);
    }
    (best, count)
}

fn main() {
    let shapes: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let doc = corpus(shapes);
    println!("corpus: {} shapes, {} bytes", shapes, doc.len());

    for (label, profile) in [("strict", Profile::Strict), ("extended", Profile::Extended)] {
        let (us, n) = min_of_3_us(|| {
            let (t, _) = parse(&doc, DEFAULT_BASE, profile).expect("parse");
            t.len() as u64
        });
        println!("metric_us parse_{label} {us:.0} triples {n}");
    }

    let (triples, outcome) = parse(&doc, DEFAULT_BASE, Profile::Strict).expect("parse");
    let (us, n) = min_of_3_us(|| {
        let text = write(&triples, Some(DEFAULT_BASE), &outcome.prefixes, Profile::Strict)
            .expect("write");
        text.len() as u64
    });
    println!("metric_us write_strict {us:.0} bytes {n}");
}

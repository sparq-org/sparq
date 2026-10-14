//! (sq-hmd7l.16) RDFC-1.0 comparative-bench runner for `bench/canon/`
//! — canonicalizes one N-Quads file through the PUBLIC sparq-canon API so the
//! same-box comparison panel (vs rdf-canonize / the rdf-canon crate driven
//! directly) exercises exactly the surface sparq ships, bridge included.
//!
//! ```sh
//! cargo build --release -p sparq-canon --example canon_bench
//! target/release/examples/canon_bench canon [--sha384] <input.nq>
//! target/release/examples/canon_bench gen-clique <n>
//! ```
//!
//! Modes:
//! - `canon [--sha384] <input.nq>` — canonical N-Quads on **stdout** (byte-exact,
//!   the cross-engine parity artifact); `parse_us=<n>` / `canon_us=<n>` advisory
//!   timings on stderr (work-box timings are non-canonical).
//! - `gen-clique <n>` — emit the classic RDFC-1.0 worst case on stdout: an
//!   n-node blank-node clique (every ordered pair related by one predicate),
//!   fully symmetric so Hash-N-Degree-Quads explodes. Deterministic; lets the
//!   poison sweep scale beyond the vendored 10-node W3C fixture without
//!   committing datasets.
//!
//! Exit codes (the `bench/canon/run.sh` outcome contract):
//! - `0` — canonicalized OK.
//! - `2` — canonicalization REJECTED fail-closed (the HNDQ call-limit guard on
//!   poison graphs, or an RDF 1.2 triple term outside the RDFC-1.0 data model).
//!   On the DoS-resistance axis this is the *good* outcome, not a crash.
//! - `3` — input is not valid N-Quads.
//! - `1` — usage / IO error.

use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "usage: canon_bench canon [--sha384] <input.nq>   # canonical N-Quads on stdout\n\
         \x20      canon_bench gen-clique <n>              # n-node blank-node clique on stdout\n\
         exit: 0 ok | 2 canonicalization rejected (fail-closed) | 3 parse error | 1 usage/IO"
    );
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("canon") => run_canon(&args[1..]),
        Some("gen-clique") => run_gen_clique(&args[1..]),
        _ => usage(),
    }
}

fn run_canon(args: &[String]) -> ! {
    let (sha384, path) = match args {
        [flag, p] if flag == "--sha384" => (true, p),
        [p] => (false, p),
        _ => usage(),
    };
    let input = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("canon_bench: read {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let t = Instant::now();
    let quads = match sparq_canon::parse_nquads(&input) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("canon_bench: parse {}: {}", path, e);
            std::process::exit(3);
        }
    };
    let parse_us = t.elapsed().as_micros();

    let t = Instant::now();
    let result = if sha384 {
        sparq_canon::canonicalize_quads_with::<sha2::Sha384>(&quads)
    } else {
        sparq_canon::canonicalize_quads(&quads)
    };
    let canon_us = t.elapsed().as_micros();

    match result {
        Ok(canonical) => {
            eprintln!("parse_us={}", parse_us);
            eprintln!("canon_us={}", canon_us);
            print!("{}", canonical);
            std::process::exit(0);
        }
        // Fail-closed rejection (HNDQ call-limit guard / triple term): the
        // honest DoS-resistance outcome — bounded refusal, not unbounded work.
        Err(e) => {
            eprintln!("canon_bench: rejected (fail-closed): {}", e);
            eprintln!("canon_us={}", canon_us);
            std::process::exit(2);
        }
    }
}

fn run_gen_clique(args: &[String]) -> ! {
    let n: usize = match args {
        [n] => n.parse().unwrap_or(0),
        _ => usage(),
    };
    if !(2..=1000).contains(&n) {
        eprintln!("canon_bench: gen-clique n must be in 2..=1000 (got {:?})", args);
        std::process::exit(1);
    }
    // Every ordered pair of distinct blank nodes, one predicate: maximal
    // blank-node symmetry (the shape of the W3C suite's negative test074).
    let mut out = String::new();
    for i in 0..n {
        for j in 0..n {
            if i != j {
                out.push_str(&format!("_:b{} <http://example.org/p> _:b{} .\n", i, j));
            }
        }
    }
    print!("{}", out);
    std::process::exit(0);
}

//! Flat-graph hot-read microbenchmark — the before/after gauge for the structural-fork
//! work: a graph with NO pending overlay and NO fork layers must read at (within noise
//! of) the pre-fork baseline. Run manually:
//!   cargo test -p sparq-engine --release --test flat_read_bench -- --ignored --nocapture
//!
//! Three probes, chosen to stress the paths the fork layer touches:
//! 1. join-heavy SELECT (merge/bind joins over scans — `scan_sorted` + `estimate`),
//! 2. point-pattern probe storm (many tiny `scan`s + `contains`-style lookups, the
//!    bind-join inner loop shape — most sensitive to per-call overhead),
//! 3. full-scan aggregate (`iter_ids`-shaped single big scan + dict term resolution).

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use std::time::Instant;

fn build_graph(n: usize) -> Graph {
    let nsubj = (n / 10).max(1);
    let mut dict = Dict::new();
    let preds: Vec<Id> = (0..10).map(|p| dict.intern_iri(&format!("http://ex/p{p}"))).collect();
    let subjs: Vec<Id> = (0..nsubj).map(|s| dict.intern_iri(&format!("http://ex/s{s}"))).collect();
    let mix = |i: usize| -> usize {
        let mut h = i as u64;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        (h % nsubj as u64) as usize
    };
    let triples: Vec<[Id; 3]> =
        (0..n).map(|i| [subjs[i % nsubj], preds[i % 10], subjs[mix(i)]]).collect();
    Graph::from_parts(dict, triples)
}

#[test]
#[ignore = "benchmark — run explicitly in --release with --nocapture"]
fn flat_read_hot_paths() {
    let n: usize =
        std::env::var("SPARQ_BENCH_TRIPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(1_000_000);
    let t0 = Instant::now();
    let g = build_graph(n);
    eprintln!("graph: {} triples, built in {:.2?}", g.len(), t0.elapsed());

    // 1. Join-heavy query (two-hop join through p0/p1), repeated. Parsed ONCE so the
    // loop times store reads + execution, not SPARQL parse/planning (roborev 1927).
    let q_join = "PREFIX : <http://ex/> SELECT * WHERE { ?s :p0 ?m . ?m :p1 ?o }";
    let prepared = sparq_engine::PreparedQuery::parse(q_join).unwrap();
    let reps = 20;
    let mut rows = 0;
    let t = Instant::now();
    for _ in 0..reps {
        rows = sparq_engine::count_prepared(&g, &prepared).unwrap();
    }
    let join_avg = t.elapsed() / reps;
    eprintln!("join query:   {join_avg:?}/run ({rows} rows, {reps} reps)");

    // 2. Point-probe storm: bound-subject scans + estimates (bind-join inner shape).
    // Subject ids resolved through the dictionary, not assumed from interning order
    // (roborev 1927), over the SAME subject breadth the original bench probed —
    // all nsubj subjects (capped at the probe count), not a small hot working set
    // that could mask read-path regressions (roborev 1945).
    let nsubj = (n / 10).max(1);
    let probes = 200_000usize;
    let subj_ids: Vec<Id> = (0..nsubj.min(probes))
        .map(|i| {
            g.id_of(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!(
                "http://ex/s{i}"
            ))))
            .expect("benchmark subject must be present")
        })
        .collect();
    let t = Instant::now();
    let mut found = 0usize;
    for i in 0..probes {
        let sid = subj_ids[i % subj_ids.len()];
        let pat = [Some(sid), None, None];
        found += g.store().estimate(&pat);
        let scan = g.store().scan(&pat);
        found += scan.rows.len();
    }
    let probe = t.elapsed();
    eprintln!("probe storm:  {:?} total / {probes} probes = {:.0} ns/probe (acc {found})", probe, probe.as_nanos() as f64 / probes as f64);

    // 3. Full-scan aggregate + dict resolution sample.
    let t = Instant::now();
    let mut acc = 0u64;
    for t3 in g.iter_ids() {
        acc = acc.wrapping_add(t3[0] as u64 ^ t3[2] as u64);
    }
    let full = t.elapsed();
    eprintln!("full scan:    {full:?} (acc {acc})");

    // dict term materialisation (result-serialisation shape).
    let t = Instant::now();
    let mut bytes = 0usize;
    for id in 1..=(g.dict().len() as Id) {
        bytes += g.dict().term(id).to_string().len();
    }
    let dict_t = t.elapsed();
    eprintln!("dict terms:   {dict_t:?} ({bytes} bytes)");
}

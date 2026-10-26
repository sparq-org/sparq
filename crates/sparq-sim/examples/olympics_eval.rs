//! The sparq-sim accuracy + latency evaluation on the real olympics dataset
//! (`bench/qlever-olympics/olympics.nt`, 1.78M triples) — the benchmark gate from
//! `research/genai-design.md` §4.
//!
//! Ground truth: `rdf:type` (foaf:Person, dbo:SportsTeam, dbo:SportsEvent, dbo:Sport,
//! dbo:Olympics, dbo:City). **Type triples are EXCLUDED from the signatures** so the
//! similarity signal cannot trivially read off the label it is judged against (the
//! leakage rule, design doc §5.5).
//!
//! Reported:
//! - **AUC**: over stratified same-class vs cross-class entity pairs (the class
//!   distribution is 98% Person, so pairs are sampled per class, not uniformly);
//! - **precision@10**: fraction of `most_similar(seed, 10)` results sharing a class
//!   with the seed (untyped results count as misses — conservative);
//! - **latency**: `most_similar(k=10)` wall-clock per seed (mean / p50 / p95 / max).
//!
//! Run: `cargo run -p sparq-sim --example olympics_eval --release`
//! Data path: `$SPARQ_OLYMPICS_NT` or `bench/qlever-olympics/olympics.nt` at the repo root.
//!
//! (sq-cxji) `--json <path>` writes the SAME measurements STDOUT prints —
//! the deterministic accuracy metrics (per-mode AUC, overall precision@10, pair/seed
//! counts) plus the advisory `most_similar` latency distribution — as a stable,
//! DEPENDENCY-FREE JSON document, mirroring the `format!`-JSON convention of
//! `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`. No serde dep is added
//! (serde_json is a TEST-only dev-dep used to parse the emit back). STDOUT is
//! byte-for-byte unchanged whether or not the flag is present; accuracy metrics are
//! deterministic, the latency numbers are ADVISORY + NON-CANONICAL (this dev box), and
//! nothing is committed.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_sim::{Sim, SimConfig};
use std::collections::HashMap;
use std::time::Instant;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// Per-class sample sizes (the dataset is 98% foaf:Person — stratify, don't sample uniformly).
const PER_CLASS: usize = 40;
/// Only classes with at least this many members participate (skips the 2-member tails).
const MIN_CLASS: usize = 40;

/// The captured accuracy + latency metrics for the `--json` emit. Every field here is
/// printed to STDOUT verbatim; the AUC/precision/count fields are DETERMINISTIC (pure
/// functions of the dataset + stratified sample), the latency fields are ADVISORY.
#[derive(Default)]
struct EvalResult {
    triples: usize,
    classes_kept: usize,
    sample_size: usize,
    /// (mode-label, auc, n_pos_pairs, n_neg_pairs) per signature mode.
    auc_by_mode: Vec<(String, f64, usize, usize)>,
    precision_at_10: f64,
    precision_hits: usize,
    precision_scored: usize,
    latency_calls: usize,
    latency_mean_ms: f64,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_max_ms: f64,
}

fn data_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SPARQ_OLYMPICS_NT") {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/qlever-olympics/olympics.nt");
    p.exists().then_some(p)
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

/// Minimal JSON string escaper (the dependency-free emit). Mode labels are ASCII;
/// anything else still yields valid `\uXXXX` escapes.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialises the evaluation to stable, dependency-free JSON. The accuracy metrics
/// (`auc`, `precision_at_10`, all counts) are DETERMINISTIC (pure functions of the
/// dataset + stratified sample); the `latency_*_ms` numbers are wall-clock — ADVISORY
/// + NON-CANONICAL (this dev box), stated in `note`.
fn results_json(r: &EvalResult) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"sparq-sim olympics_eval\",\n");
    s.push_str(
        "  \"note\": \"accuracy metrics (`auc`, `precision_at_10`, pair/seed counts) are \
         DETERMINISTIC (pure functions of the dataset + stratified sample); `latency_*_ms` are \
         `most_similar` wall-clock MEASURED on the running host — ADVISORY, NON-CANONICAL \
         (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str(&format!("  \"triples\": {},\n", r.triples));
    s.push_str(&format!("  \"classes_kept\": {},\n", r.classes_kept));
    s.push_str(&format!("  \"sample_size\": {},\n", r.sample_size));
    s.push_str("  \"auc_by_mode\": [\n");
    for (i, (label, auc, pos, neg)) in r.auc_by_mode.iter().enumerate() {
        let comma = if i + 1 < r.auc_by_mode.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"mode\": {}, \"auc\": {auc:.6}, \"pos_pairs\": {pos}, \"neg_pairs\": {neg} }}{comma}\n",
            json_str(label.trim()),
        ));
    }
    s.push_str("  ],\n");
    s.push_str(&format!(
        "  \"precision_at_10\": {:.6},\n",
        r.precision_at_10
    ));
    s.push_str(&format!("  \"precision_hits\": {},\n", r.precision_hits));
    s.push_str(&format!(
        "  \"precision_scored\": {},\n",
        r.precision_scored
    ));
    s.push_str(&format!("  \"latency_calls\": {},\n", r.latency_calls));
    s.push_str(&format!(
        "  \"latency_mean_ms\": {:.4},\n",
        r.latency_mean_ms
    ));
    s.push_str(&format!("  \"latency_p50_ms\": {:.4},\n", r.latency_p50_ms));
    s.push_str(&format!("  \"latency_p95_ms\": {:.4},\n", r.latency_p95_ms));
    s.push_str(&format!("  \"latency_max_ms\": {:.4}\n", r.latency_max_ms));
    s.push_str("}\n");
    s
}

fn main() {
    let (_args, json_path) = take_json_flag(std::env::args().collect());

    let Some(path) = data_path() else {
        eprintln!("olympics.nt not found (set SPARQ_OLYMPICS_NT or place it under bench/qlever-olympics/) — skipping");
        return;
    };
    let mut metrics = EvalResult::default();
    let t0 = Instant::now();
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let g = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    metrics.triples = g.len();
    println!(
        "loaded {} triples in {:.1}s",
        g.len(),
        t0.elapsed().as_secs_f64()
    );

    let rdf_type = NamedNode::new(RDF_TYPE).unwrap();
    let sim = Sim::with_config(
        &g,
        SimConfig {
            exclude_predicates: vec![rdf_type.clone()],
            ..SimConfig::default()
        },
    );

    // ---- Ground truth: entity id -> class ids, via one POS scan of the rdf:type block.
    let type_id = g
        .id_of(&Term::NamedNode(rdf_type))
        .expect("rdf:type present");
    let scan = g.store().scan(&[None, Some(type_id), None]);
    let mut classes_of: HashMap<Id, Vec<Id>> = HashMap::new();
    let mut members: HashMap<Id, Vec<Id>> = HashMap::new();
    for row in scan.rows.iter() {
        let [s, _, o] = scan.to_spo(row);
        classes_of.entry(s).or_default().push(o);
        members.entry(o).or_default().push(s);
    }
    let mut classes: Vec<(Id, Vec<Id>)> = members
        .into_iter()
        .filter(|(_, m)| m.len() >= MIN_CLASS)
        .collect();
    classes.sort_by_key(|(c, _)| *c);
    metrics.classes_kept = classes.len();
    println!("classes with >= {MIN_CLASS} members: {}", classes.len());

    // ---- Stratified sample: PER_CLASS entities per class, deterministic stride.
    let mut sample: Vec<(Id, Id)> = Vec::new(); // (entity, class)
    for (c, m) in &classes {
        let mut m = m.clone();
        m.sort_unstable();
        let stride = (m.len() / PER_CLASS).max(1);
        for e in m.iter().step_by(stride).take(PER_CLASS) {
            sample.push((*e, *c));
        }
        let name = g.dict().term(*c);
        println!(
            "  class {name}: {} members, sampled {}",
            m.len(),
            m.len().div_ceil(stride).min(PER_CLASS)
        );
    }
    metrics.sample_size = sample.len();

    // ---- AUC over all sampled pairs (signatures cached), in BOTH signature modes:
    // PredicateNeighbor measures shared concrete context (the ranking signal used by
    // most_similar); Predicates measures shared role/profile (the class-separation
    // signal — e.g. two Sports share no concrete neighbour, every event names exactly
    // one sport, but their predicate profiles are near-identical).
    let same_class = |a: Id, b: Id| -> bool {
        let (ca, cb) = (&classes_of[&a], &classes_of[&b]);
        ca.iter().any(|c| cb.contains(c))
    };
    let rdf_type_nn = NamedNode::new(RDF_TYPE).unwrap();
    for (label, mode) in [
        (
            "predicate+neighbor",
            sparq_sim::SignatureMode::PredicateNeighbor,
        ),
        ("predicate-profile ", sparq_sim::SignatureMode::Predicates),
    ] {
        let s = Sim::with_config(
            &g,
            SimConfig {
                mode,
                exclude_predicates: vec![rdf_type_nn.clone()],
                ..SimConfig::default()
            },
        );
        let sigs: Vec<(Id, sparq_sim::Signature)> = sample
            .iter()
            .map(|&(e, _)| {
                (
                    e,
                    s.signature(&g.dict().term(e))
                        .expect("sampled entity in dict"),
                )
            })
            .collect();
        let mut pos: Vec<f64> = Vec::new();
        let mut neg: Vec<f64> = Vec::new();
        let t_auc = Instant::now();
        for i in 0..sigs.len() {
            for j in (i + 1)..sigs.len() {
                let v = sparq_sim::weighted_jaccard(&sigs[i].1, &sigs[j].1);
                if same_class(sigs[i].0, sigs[j].0) {
                    pos.push(v);
                } else {
                    neg.push(v);
                }
            }
        }
        let auc_value = auc(&pos, &neg);
        println!(
            "AUC [{label}] (same-class above cross-class): {:.4}   [{} pos / {} neg pairs, {:.1}s]",
            auc_value,
            pos.len(),
            neg.len(),
            t_auc.elapsed().as_secs_f64()
        );
        metrics
            .auc_by_mode
            .push((label.to_string(), auc_value, pos.len(), neg.len()));
    }

    // ---- precision@10 + most_similar latency, per class and overall.
    let mut latencies: Vec<f64> = Vec::new();
    let mut overall = (0usize, 0usize); // (hits, scored)
    for (c, _) in &classes {
        let seeds: Vec<Id> = sample
            .iter()
            .filter(|(_, sc)| sc == c)
            .map(|(e, _)| *e)
            .collect();
        let mut hits = 0;
        let mut scored = 0;
        for &e in &seeds {
            let term = g.dict().term(e);
            let t = Instant::now();
            let top = sim.most_similar(&term, 10);
            latencies.push(t.elapsed().as_secs_f64() * 1000.0);
            for (cand, _) in &top {
                scored += 1;
                if let Some(cid) = g.id_of(cand) {
                    if classes_of.contains_key(&cid) && same_class(e, cid) {
                        hits += 1;
                    }
                }
            }
        }
        overall.0 += hits;
        overall.1 += scored;
        let name = g.dict().term(*c);
        println!(
            "  precision@10 {name}: {:.3}  ({hits}/{scored}, {} seeds)",
            hits as f64 / scored.max(1) as f64,
            seeds.len()
        );
    }
    metrics.precision_at_10 = overall.0 as f64 / overall.1.max(1) as f64;
    metrics.precision_hits = overall.0;
    metrics.precision_scored = overall.1;
    println!(
        "precision@10 overall: {:.3}  ({}/{})",
        overall.0 as f64 / overall.1.max(1) as f64,
        overall.0,
        overall.1
    );

    latencies.sort_by(f64::total_cmp);
    let pct = |p: f64| latencies[((latencies.len() - 1) as f64 * p) as usize];
    let mean = latencies.iter().sum::<f64>() / latencies.len() as f64;
    println!(
        "most_similar(k=10) latency over {} calls: mean {:.2} ms, p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        latencies.len(),
        mean,
        pct(0.50),
        pct(0.95),
        latencies.last().unwrap()
    );
    metrics.latency_calls = latencies.len();
    metrics.latency_mean_ms = mean;
    metrics.latency_p50_ms = pct(0.50);
    metrics.latency_p95_ms = pct(0.95);
    metrics.latency_max_ms = *latencies.last().unwrap();

    // (sq-cxji) Strictly-additive JSON emit: only when `--json <path>` was
    // given. STDOUT above is the unchanged human report.
    if let Some(path) = json_path {
        let doc = results_json(&metrics);
        if let Err(e) = std::fs::write(&path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote evaluation metrics to {path}");
    }
}

/// Exact AUC by pairwise comparison (ties count ½).
fn auc(pos: &[f64], neg: &[f64]) -> f64 {
    // Rank-sum (Mann-Whitney) over the merged score list — O((P+N) log (P+N)).
    let mut all: Vec<(f64, bool)> = pos
        .iter()
        .map(|&s| (s, true))
        .chain(neg.iter().map(|&s| (s, false)))
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut rank_sum = 0.0;
    let mut i = 0;
    let mut rank = 1.0;
    while i < all.len() {
        let mut j = i;
        while j < all.len() && all[j].0 == all[i].0 {
            j += 1;
        }
        let avg_rank = (rank + (rank + (j - i) as f64 - 1.0)) / 2.0;
        for item in &all[i..j] {
            if item.1 {
                rank_sum += avg_rank;
            }
        }
        rank += (j - i) as f64;
        i = j;
    }
    let (p, n) = (pos.len() as f64, neg.len() as f64);
    (rank_sum - p * (p + 1.0) / 2.0) / (p * n)
}

// ---------------------------------------------------------------------------
// (sq-cxji) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["olympics_eval", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["olympics_eval"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["olympics_eval"].iter().map(|s| s.to_string()).collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("predicate+neighbor"), "\"predicate+neighbor\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn results_json_shape_and_keys() {
        let metrics = EvalResult {
            triples: 1_780_000,
            classes_kept: 6,
            sample_size: 160,
            // The label intentionally has a trailing space (mirrors the live "predicate-profile ")
            // to assert the emit trims it.
            auc_by_mode: vec![
                ("predicate+neighbor".to_string(), 0.91, 1000, 8000),
                ("predicate-profile ".to_string(), 0.97, 1000, 8000),
            ],
            precision_at_10: 0.85,
            precision_hits: 136,
            precision_scored: 160,
            latency_calls: 160,
            latency_mean_ms: 0.42,
            latency_p50_ms: 0.4,
            latency_p95_ms: 0.8,
            latency_max_ms: 1.5,
        };
        let doc = results_json(&metrics);
        // The dependency-free emit must round-trip through a REAL serde_json parse —
        // this catches a malformed document (e.g. a missing inter-row comma).
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "sparq-sim olympics_eval");
        assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
        assert_eq!(v["triples"], 1_780_000);
        assert_eq!(v["classes_kept"], 6);
        assert_eq!(v["sample_size"], 160);
        let arr = v["auc_by_mode"]
            .as_array()
            .expect("auc_by_mode is an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["mode"], "predicate+neighbor");
        assert!((arr[0]["auc"].as_f64().unwrap() - 0.91).abs() < 1e-6);
        assert_eq!(arr[0]["pos_pairs"], 1000);
        // The trailing space on the second label must be trimmed in the emit.
        assert_eq!(arr[1]["mode"], "predicate-profile");
        assert!((v["precision_at_10"].as_f64().unwrap() - 0.85).abs() < 1e-9);
        assert_eq!(v["precision_hits"], 136);
        assert_eq!(v["latency_calls"], 160);
        assert!(v["latency_p95_ms"].is_number());
    }
}

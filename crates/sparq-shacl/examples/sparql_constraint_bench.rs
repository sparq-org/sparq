//! (sq-pymgf, epic sq-hmd7l) SPARQL-constraint SLICE sub-panel runner —
//! validates ONLY the `sh:sparql`-heavy shape set
//! (`bench/shacl/shapes-sparql/sparql_heavy.ttl`) over the deterministic LUBM(1)
//! ABox and reports the slice PER SHAPE, so the SHACL-SPARQL evaluation path
//! (engine-native `sh:select` with `$this` batched into one `VALUES` execution,
//! sq-7d3dj.33.1) is benchmarked as its own axis instead of being averaged into
//! the whole-suite mix `scripts/bench/shacl-same-box.sh` runs.
//!
//! ```sh
//! cargo run -p sparq-shacl --release --example sparql_constraint_bench -- --smoke   # iters=1
//! cargo run -p sparq-shacl --release --example sparql_constraint_bench -- [iters]   # default 3
//! ```
//!
//! HARD GATE BEFORE ANY TIMING (the sq-pymgf invariant): every shape's
//! `conforms` / `violations` / `focus_nodes` is asserted against the pinned
//! oracle `bench/shacl/expected-sparql.tsv` (derived from the report itself and
//! independently cross-checked against the raw ABox — see that file's header).
//! Any drift prints the observed-vs-pinned values and exits 1 WITHOUT emitting a
//! single timing row: a drift is a recorded correctness finding, never rounded.
//! Two internal consistency checks back the per-shape split: each shape's
//! single-shape validation count must equal its `sh:sourceShape`-grouped count
//! from the full-set validation, and the per-shape counts must sum to the
//! full-set total.
//!
//! Output (stdout, only after the gate passes) is the 3-column contract
//! `bench/shacl/run.sh` forwards for the committed suite:
//!
//! ```text
//! <workload>\t<violations>\t<validate_us>
//! ```
//!
//! one row per shape (workload = the shape IRI's local name, best-of-`iters`
//! single-shape validate time) plus a `TOTAL` row (all three shapes validated
//! together, one model). Timing is ADVISORY (non-canonical on the shared work
//! box); correctness lives in `expected-sparql.tsv`, NOT in the binary — exactly
//! like `bench/shacl/expected.tsv`.
//!
//! The external competitor column (pySHACL via
//! `scripts/bench-adapters/shacl_report_count.py`) is a documented GATHER step —
//! see `research/gap-shacl-sparql-2026-07.md` — so this harness stays
//! single-crate and self-asserting.
//!
//! Data: `$BENCH_SHACL_DATA` (must still be the pinned LUBM(1) seed-0 ABox — the
//! gate asserts it) or `bench/shacl/gen.sh 1 0` (cached after the first run).
//! The shapes + oracle are read from the repo checkout via `CARGO_MANIFEST_DIR`,
//! so the run is cwd-independent.

use oxrdf::{NamedOrBlankNode, Term, Triple};
use sparq_shacl::{count_focus_nodes, graph_from_triples, validate_with_model, ShapesModel};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SH_NODE_SHAPE: &str = "http://www.w3.org/ns/shacl#NodeShape";
/// The whole-slice row: all three shapes validated together as ONE model.
const TOTAL: &str = "TOTAL";

/// Pinned oracle row from `bench/shacl/expected-sparql.tsv`
/// (`<workload>\t<conforms>\t<violations>\t<focus_nodes>`).
struct Expected {
    conforms: u8,
    violations: usize,
    focus_nodes: usize,
}

/// Observed deterministic gate values + the advisory timing for one workload.
struct Observed {
    conforms: u8,
    violations: usize,
    focus_nodes: usize,
    validate_us: f64,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root from CARGO_MANIFEST_DIR")
}

/// Locate the pinned LUBM(1) seed-0 ABox: `$BENCH_SHACL_DATA` override, else
/// `bench/shacl/gen.sh 1 0` (first stdout line; cached + deterministic).
fn ensure_data(root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("BENCH_SHACL_DATA") {
        return PathBuf::from(p);
    }
    let gen = root.join("bench/shacl/gen.sh");
    let out = std::process::Command::new(&gen)
        .args(["1", "0"])
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", gen.display()));
    assert!(
        out.status.success(),
        "bench/shacl/gen.sh 1 0 failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let data = stdout.lines().next().unwrap_or_default().trim();
    assert!(!data.is_empty(), "gen.sh emitted no data path");
    PathBuf::from(data)
}

/// Parse the pinned oracle TSV (comment lines `#` and blanks skipped).
fn parse_expected(text: &str) -> BTreeMap<String, Expected> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert!(
            cols.len() == 4,
            "malformed expected-sparql.tsv row: {line:?}"
        );
        let parse = |i: usize| -> usize {
            cols[i]
                .parse()
                .unwrap_or_else(|e| panic!("bad number in expected-sparql.tsv row {line:?}: {e}"))
        };
        map.insert(
            cols[0].to_string(),
            Expected {
                conforms: u8::try_from(parse(1)).expect("conforms is 0/1"),
                violations: parse(2),
                focus_nodes: parse(3),
            },
        );
    }
    map
}

/// The forward-reachable sub-graph of `triples` from the shape node `start` —
/// the shape's own triples plus everything it references (its `sh:sparql`
/// constraint bnode, the shared `sh:prefixes` declaration node, …). This slices
/// the committed multi-shape file into per-shape shape graphs WITHOUT committing
/// derived copies of it.
fn shape_closure(triples: &[Triple], start: &str) -> Vec<Triple> {
    // Key subjects/objects by their N-Triples form so NamedOrBlankNode and Term
    // compare through one representation.
    let subject_key = |s: &NamedOrBlankNode| s.to_string();
    let mut by_subject: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, t) in triples.iter().enumerate() {
        by_subject
            .entry(subject_key(&t.subject))
            .or_default()
            .push(i);
    }
    let mut reached: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let start_key = format!("<{start}>");
    reached.insert(start_key.clone());
    queue.push_back(start_key);
    let mut keep: Vec<usize> = Vec::new();
    while let Some(key) = queue.pop_front() {
        for &i in by_subject.get(&key).into_iter().flatten() {
            keep.push(i);
            let obj = &triples[i].object;
            if matches!(obj, Term::NamedNode(_) | Term::BlankNode(_)) {
                let k = obj.to_string();
                if reached.insert(k.clone()) {
                    queue.push_back(k);
                }
            }
        }
    }
    keep.sort_unstable();
    keep.dedup();
    keep.into_iter().map(|i| triples[i].clone()).collect()
}

/// `http://…#GradCourseShape` → `GradCourseShape` (the TSV workload key).
fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/']).next().unwrap_or(iri)
}

/// Best-of-`iters` validation: (violations, conforms, best µs).
fn timed_validate(
    data: &sparq_core::Graph,
    model: &ShapesModel,
    iters: usize,
) -> (usize, bool, f64) {
    let mut best_us = f64::INFINITY;
    let mut violations = 0usize;
    let mut conforms = true;
    for _ in 0..iters {
        let t = Instant::now();
        let report = validate_with_model(data, model);
        best_us = best_us.min(t.elapsed().as_secs_f64() * 1e6);
        violations = report.results.len();
        conforms = report.conforms;
    }
    (violations, conforms, best_us)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let iters: usize = match args.first().map(String::as_str) {
        None => 3,
        Some("--smoke") => 1,
        Some(n) => n.parse().unwrap_or_else(|_| {
            eprintln!(
                "usage: sparql_constraint_bench [--smoke | iters>=1]\n\
                 asserts bench/shacl/expected-sparql.tsv, then emits per-shape\n\
                 <workload>\\t<violations>\\t<validate_us> (+ a TOTAL row)"
            );
            std::process::exit(2);
        }),
    };
    if iters == 0 {
        eprintln!("error: iters must be >= 1 (got 0); need at least one sample for finite timings");
        std::process::exit(2);
    }

    let root = repo_root();
    let shapes_path = root.join("bench/shacl/shapes-sparql/sparql_heavy.ttl");
    let expected_path = root.join("bench/shacl/expected-sparql.tsv");

    // ---- data graph (pinned LUBM(1) seed-0 ABox; load timing is ADVISORY) ------------
    let data_path = ensure_data(&root);
    let data_text = std::fs::read_to_string(&data_path)
        .unwrap_or_else(|e| panic!("read data {}: {e}", data_path.display()));
    let t = Instant::now();
    let data = sparq_core::Graph::load_str(&data_text, "ntriples")
        .unwrap_or_else(|e| panic!("parse data {}: {e}", data_path.display()));
    let load_us = t.elapsed().as_secs_f64() * 1e6;
    eprintln!(
        "[shacl-sparql] data={} load_us={load_us:.1} (advisory) iters={iters}",
        data_path.display()
    );

    // ---- shapes: full set + per-shape slices of the SAME committed file ---------------
    let shapes_text = std::fs::read_to_string(&shapes_path)
        .unwrap_or_else(|e| panic!("read shapes {}: {e}", shapes_path.display()));
    let triples: Vec<Triple> = oxttl::TurtleParser::new()
        .for_slice(shapes_text.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("parse shapes {}: {e}", shapes_path.display()));

    // Discover the shape nodes (named subjects typed sh:NodeShape), sorted by IRI.
    let mut shape_iris: Vec<String> = triples
        .iter()
        .filter_map(|t| match (&t.subject, &t.predicate, &t.object) {
            (NamedOrBlankNode::NamedNode(s), p, Term::NamedNode(o))
                if p.as_str() == RDF_TYPE && o.as_str() == SH_NODE_SHAPE =>
            {
                Some(s.as_str().to_string())
            }
            _ => None,
        })
        .collect();
    shape_iris.sort();
    shape_iris.dedup();
    assert!(
        !shape_iris.is_empty(),
        "no sh:NodeShape found in {}",
        shapes_path.display()
    );

    // Full-set validation (the TOTAL row) + per-source-shape grouped counts.
    let full_graph = graph_from_triples(triples.iter().cloned());
    let full_model = ShapesModel::parse(&full_graph);
    let full_focus = count_focus_nodes(&data, &full_model);
    let (total_violations, total_conforms, total_us) = timed_validate(&data, &full_model, iters);
    let mut grouped: BTreeMap<String, usize> = BTreeMap::new();
    {
        // One extra (untimed) run to attribute results per sh:sourceShape.
        let report = validate_with_model(&data, &full_model);
        for r in &report.results {
            let key = match &r.source_shape {
                Term::NamedNode(n) => n.as_str().to_string(),
                other => other.to_string(),
            };
            *grouped.entry(key).or_default() += 1;
        }
    }

    // Per-shape single-shape validations.
    let mut observed: BTreeMap<String, Observed> = BTreeMap::new();
    let mut fail = false;
    for iri in &shape_iris {
        let slice = shape_closure(&triples, iri);
        let graph = graph_from_triples(slice);
        let model = ShapesModel::parse(&graph);
        let focus_nodes = count_focus_nodes(&data, &model);
        let (violations, conforms, validate_us) = timed_validate(&data, &model, iters);
        // Internal consistency: the single-shape count must equal the
        // sh:sourceShape-grouped count from the full-set run (two routes, one answer).
        let from_group = grouped.get(iri).copied().unwrap_or(0);
        if violations != from_group {
            eprintln!(
                "[shacl-sparql] ERROR: {} single-shape violations={violations} but \
                 full-set sourceShape-grouped count={from_group} (per-shape split unsound)",
                local_name(iri)
            );
            fail = true;
        }
        observed.insert(
            local_name(iri).to_string(),
            Observed {
                conforms: u8::from(conforms),
                violations,
                focus_nodes,
                validate_us,
            },
        );
    }
    let per_shape_sum: usize = observed.values().map(|o| o.violations).sum();
    if per_shape_sum != total_violations {
        eprintln!(
            "[shacl-sparql] ERROR: per-shape violations sum {per_shape_sum} != full-set \
             total {total_violations} (per-shape split unsound)"
        );
        fail = true;
    }
    observed.insert(
        TOTAL.to_string(),
        Observed {
            conforms: u8::from(total_conforms),
            violations: total_violations,
            focus_nodes: full_focus,
            validate_us: total_us,
        },
    );

    // ---- HARD GATE: assert every workload vs the pinned oracle BEFORE any timing ------
    let expected_text = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("read oracle {}: {e}", expected_path.display()));
    let expected = parse_expected(&expected_text);
    for (name, exp) in &expected {
        let Some(obs) = observed.get(name) else {
            eprintln!(
                "[shacl-sparql] ERROR: pinned workload {name} did not run (a workload vanished)"
            );
            fail = true;
            continue;
        };
        if obs.violations != exp.violations {
            eprintln!(
                "[shacl-sparql] ERROR: {name} violations={} expected={} (correctness regression)",
                obs.violations, exp.violations
            );
            fail = true;
        }
        if obs.conforms != exp.conforms {
            eprintln!(
                "[shacl-sparql] ERROR: {name} conforms={} expected={} (correctness regression)",
                obs.conforms, exp.conforms
            );
            fail = true;
        }
        if obs.focus_nodes != exp.focus_nodes {
            eprintln!(
                "[shacl-sparql] ERROR: {name} focus_nodes={} expected={} (target-selection regression)",
                obs.focus_nodes, exp.focus_nodes
            );
            fail = true;
        }
    }
    for name in observed.keys() {
        if !expected.contains_key(name) {
            eprintln!("[shacl-sparql] ERROR: workload {name} has no expected-sparql.tsv entry");
            fail = true;
        }
    }
    if fail {
        eprintln!(
            "[shacl-sparql] FAILED: divergence from bench/shacl/expected-sparql.tsv — \
             no timings emitted (a drift is a correctness finding, never rounded)"
        );
        std::process::exit(1);
    }

    // ---- gate passed: emit the <workload>\t<violations>\t<us> contract ----------------
    // Per-shape rows (sorted), then the whole-slice TOTAL row.
    for (name, obs) in observed.iter().filter(|(n, _)| n.as_str() != TOTAL) {
        println!("{name}\t{}\t{:.1}", obs.violations, obs.validate_us);
    }
    let total = &observed[TOTAL];
    println!("{TOTAL}\t{}\t{:.1}", total.violations, total.validate_us);
    eprintln!(
        "[shacl-sparql] OK: all {} workloads match expected-sparql.tsv \
         (violations + conforms + focus_nodes); timings are ADVISORY (non-canonical box)",
        observed.len()
    );
}

// [OPUS-4.8] (sq-b4lo / gh-52) written while Fable 5 unavailable — re-review when Fable returns.
//! SINGLE-WRITER WRITE-THROUGHPUT BENCHMARK — the PSS (Solid pod-server) update set.
//!
//! ## What this measures and why it exists
//!
//! gh-52 (the PSS↔SPARQ horizontal-scale thread) locked sparq's Phase-2 serving contract
//! to **"N readers against 1 sequenced writer + external coordination"** (documented in
//! `crates/sparq-server/README.md` → "Concurrency contract"). The binding acceptance
//! criterion the PSS agent set for that single writer is **parity-or-better vs the
//! QLever-over-HTTP write path on PSS's actual update set** (NOT bulk ingest); the
//! reference starting targets are *sustained ≥ a few hundred small updates/sec* and
//! *p99 write-commit < ~50 ms*.
//!
//! This harness drives sparq's PRODUCTION single sequenced writer (`AppState`, the same
//! `apply_update` the HTTP `application/sparql-update` path calls) through the **exact
//! PSS named-graph update shapes** that `crates/sparq-server/tests/named_graphs.rs`
//! pins for correctness (gh-47) — so throughput here is measured over the operations PSS
//! actually issues, not a synthetic stand-in:
//!
//! - `put_document` — `DELETE WHERE { GRAPH <r> { <r> ?p ?o } } ; INSERT DATA { GRAPH <r> { … } }`:
//!   replace a resource's ~8 metadata triples in its own per-resource named graph + the
//!   parent container's `ldp:contains` link (PSS `putDocument`).
//! - `set_acl` — `DELETE { … } INSERT { … } WHERE { OPTIONAL { … } }`: idempotently replace a
//!   single-valued `acl:accessControl` pointer in the container graph (PSS `setAclPointer` /
//!   `putContainer`, the gh-47 shape (3)).
//! - `delete_document` — `DROP GRAPH <r> ; DELETE WHERE { GRAPH <parent> { <parent> ldp:contains <r> } }`:
//!   remove a resource graph and unlink it from its container (PSS DELETE).
//! - `provision` — the heaviest interactive burst: ONE multi-operation UPDATE that creates a
//!   pod root container + a handful of child resources + their `.acl` pointers in a single
//!   sequenced commit (PSS pod provisioning).
//!
//! ## Profiles
//!
//! - `crud` (default) — a sustained interleaved CRUD stream over a resource pool, round-robin
//!   so DELETEs really hit (an insert-only overlay would flatter the number). Reports sustained
//!   writes/s + commit-latency p50/p99/max per 1k window + total + final RSS.
//! - `provision` — repeated provisioning bursts (the worst-case interactive flow): writes/s and
//!   per-burst commit latency.
//!
//! ## Parity gate (`--qlever-baseline-ms <p99_ms>`)
//!
//! The binding criterion is *relative to QLever*, and QLever's write path is an external,
//! Docker-on-host process; this harness therefore does NOT embed a QLever number (that
//! would bake a machine-specific absolute into a tracked file — forbidden by repo
//! hygiene). Instead `bench/pss-update-set/compare.py` runs BOTH engines over the same
//! update set on one box and asserts parity; for a standalone sparq-only check this binary
//! accepts a recorded QLever p99 (ms) and FAILS (exit 1) if sparq's p99 regresses past it
//! by more than the tolerance (`--tolerance`, default 1.0 = exact parity-or-better).
//! Without the flag it is a pure measurement (exit 0 always) — re-run on the target box.
//!
//! Run: `cargo run --release --bin pss_update_throughput` in `bench/serve`.
//! Flags: `--profile crud|provision`, `--updates N`, `--resources N`, `--readers N`,
//! `--children N`, `--qlever-baseline-ms F`, `--tolerance F`.
//!   `… -- --json /tmp/pss.json`  # strictly-additive
//!
//! [OPUS-4.8] (sq-5vm.1) `--json <path>` writes the SAME per-window + TOTAL + parity-gate
//! figures STDOUT prints as a stable, DEPENDENCY-FREE JSON document, mirroring writer_spike's
//! emit (sq-k5qq). No serde dep is added to the harness (serde_json is a TEST-only dev-dep that
//! parses the emit back). STDOUT is byte-for-byte unchanged whether or not the flag is present;
//! every number is best-effort, MEASURED on the running host — ADVISORY + NON-CANONICAL (stated
//! in the emitted `note`) — nothing is committed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_server::{AppState, ServerConfig};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0)
        / 1024.0
}

// --- PSS update shapes (mirroring crates/sparq-server/tests/named_graphs.rs) --------

const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";
const ACL_ACCESS_CONTROL: &str = "http://www.w3.org/ns/auth/acl#accessControl";

fn resource_iri(pod: usize, res: usize) -> String {
    format!("http://pod.ex/p{pod}/r{res}.ttl")
}
fn container_iri(pod: usize) -> String {
    format!("http://pod.ex/p{pod}/")
}

/// PSS `putDocument`: replace the resource's metadata in its OWN named graph + (re)link it
/// into the parent container graph. `DELETE WHERE` clears the prior version; `INSERT DATA`
/// writes ~8 fresh triples. Resource and container are distinct named graphs.
fn put_document(pod: usize, res: usize, version: usize) -> String {
    let r = resource_iri(pod, res);
    let c = container_iri(pod);
    format!(
        "DELETE WHERE {{ GRAPH <{r}> {{ <{r}> ?p ?o }} }} ; \
         INSERT DATA {{ \
           GRAPH <{r}> {{ \
             <{r}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> . \
             <{r}> <https://pss.dev/ns#contentType> \"text/turtle\" . \
             <{r}> <http://www.w3.org/ns/posix/stat#size> {version} . \
             <{r}> <http://www.w3.org/ns/posix/stat#mtime> {version} . \
             <{r}> <http://purl.org/dc/terms/modified> \"2026-06-14T00:00:{ss:02}\" . \
             <{r}> <https://pss.dev/ns#etag> \"etag-{pod}-{res}-{version}\" . \
             <{r}> <https://pss.dev/ns#s3Key> \"k/{pod}/{res}/{version}\" . \
           }} \
           GRAPH <{c}> {{ <{c}> <{LDP_CONTAINS}> <{r}> . }} \
         }}",
        ss = version % 60
    )
}

/// PSS `setAclPointer` / `putContainer` (gh-47 shape 3): idempotently replace a
/// single-valued `acl:accessControl` pointer in the container graph, whether or not one
/// already exists. `OPTIONAL` binds the prior pointer (if any); `DELETE` clears it; the
/// `INSERT` sets the new one — exactly one value remains, no accumulation.
fn set_acl(pod: usize, res: usize, version: usize) -> String {
    let r = resource_iri(pod, res);
    let c = container_iri(pod);
    let acl = format!("http://pod.ex/p{pod}/v{version}.acl");
    format!(
        "DELETE {{ GRAPH <{c}> {{ <{r}> <{ACL_ACCESS_CONTROL}> ?old }} }} \
         INSERT {{ GRAPH <{c}> {{ <{r}> <{ACL_ACCESS_CONTROL}> <{acl}> }} }} \
         WHERE  {{ OPTIONAL {{ GRAPH <{c}> {{ <{r}> <{ACL_ACCESS_CONTROL}> ?old }} }} }}"
    )
}

/// PSS resource DELETE: drop the resource's own graph and unlink it from its container.
fn delete_document(pod: usize, res: usize) -> String {
    let r = resource_iri(pod, res);
    let c = container_iri(pod);
    format!(
        "DROP SILENT GRAPH <{r}> ; \
         DELETE WHERE {{ GRAPH <{c}> {{ <{c}> <{LDP_CONTAINS}> <{r}> }} }}"
    )
}

/// PSS pod provisioning — the heaviest interactive burst: ONE multi-operation UPDATE that
/// creates the pod-root container + `n_children` child resources (each in its own graph,
/// linked from the container) + an `.acl` pointer for each, all in a single sequenced
/// commit. This is the "handful of resources + ACLs in one flow" shape gh-52 calls out.
fn provision(pod: usize, n_children: usize) -> String {
    let c = container_iri(pod);
    let mut s = String::new();
    s.push_str(&format!(
        "INSERT DATA {{ GRAPH <{c}> {{ <{c}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Container> ."
    ));
    for res in 0..n_children {
        let r = resource_iri(pod, res);
        s.push_str(&format!(
            " <{c}> <{LDP_CONTAINS}> <{r}> . <{r}> <{ACL_ACCESS_CONTROL}> <http://pod.ex/p{pod}/v0.acl> ."
        ));
    }
    s.push_str(" } ");
    for res in 0..n_children {
        let r = resource_iri(pod, res);
        s.push_str(&format!(
            "GRAPH <{r}> {{ <{r}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> . \
             <{r}> <http://www.w3.org/ns/posix/stat#size> 0 . }} "
        ));
    }
    s.push('}');
    s
}

// --- metrics ----------------------------------------------------------------

fn pct(sorted_us: &[u64], p: f64) -> u64 {
    if sorted_us.is_empty() {
        return 0;
    }
    let idx = ((sorted_us.len() as f64 * p) as usize).min(sorted_us.len() - 1);
    sorted_us[idx]
}

struct Run {
    /// all per-update commit latencies (µs), unsorted until summarised in `main`
    lat_us: Vec<u64>,
    updates: usize,
    wall: Duration,
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json <path> machine-readable results emit
// ---------------------------------------------------------------------------

/// One 1k-window of the CRUD stream (the `crud` profile only emits these).
struct WindowRow {
    from: usize,
    to: usize,
    throughput_per_s: f64,
    p50_us: u64,
    p99_us: u64,
    max_us: u64,
    rss_mb: f64,
}

/// Run-level summary (params + the TOTAL line + reference / parity-gate verdicts).
#[derive(Default)]
struct Summary {
    profile: String,
    updates: usize,
    resources: usize,
    readers: usize,
    children: usize,
    base_triples_target: usize,
    base_triples: usize,
    base_rss_mb: f64,
    committed: usize,
    wall_secs: f64,
    throughput_per_s: f64,
    p50_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    final_graph_triples: usize,
    reader_queries: u64,
    final_rss_mb: f64,
    /// the gh-52 reference verdict on p99 (< ~50ms): "PASS" or "OVER".
    reference_p99_verdict: String,
    /// when `--qlever-baseline-ms` is supplied: the recorded QLever p99 + the parity bar.
    qlever_baseline_ms: Option<f64>,
    tolerance: f64,
    parity_bar_ms: Option<f64>,
    /// "PASS"/"FAIL" when a baseline was supplied, else "n/a".
    parity_verdict: String,
}

/// Extracts `--json <path>` from argv, returning argv WITHOUT the flag pair. A bare
/// `--json` is a usage error (exit 2), mirroring `sparq-cli` / `writer_spike`'s flag.
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

/// Minimal JSON string escaper (the dependency-free emit). Labels are static ASCII, so
/// this covers the realistic input; anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Render an `Option<f64>` as a JSON number or `null` (parity fields are absent unless a
/// QLever baseline was supplied).
fn opt_num(v: Option<f64>) -> String {
    match v {
        Some(n) => format!("{:.3}", n),
        None => "null".to_string(),
    }
}

/// Serialise the run to stable, dependency-free JSON. Every metric is ADVISORY +
/// NON-CANONICAL (stated in `note`).
fn results_json(sum: &Summary, windows: &[WindowRow]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"harness\": \"serve-spikes pss_update_throughput\",\n");
    s.push_str(
        "  \"note\": \"single-writer write-throughput on the PSS update set (gh-52); the binding \
         metric is p99 commit latency. This harness uses ServerConfig::default(), whose \
         adaptive_commit is ON, so single-client throughput is engine-bound and is not the \
         concurrent write ceiling measured by writer_spike. Every µs/ms-latency, updates/s and \
         RSS figure is best-effort, MEASURED on the running host — ADVISORY, NON-CANONICAL \
         (this dev box) — do not bake into committed files\",\n",
    );
    s.push_str(&format!("  \"profile\": {},\n", json_str(&sum.profile)));
    s.push_str(&format!("  \"updates\": {},\n", sum.updates));
    s.push_str(&format!("  \"resources\": {},\n", sum.resources));
    s.push_str(&format!("  \"readers\": {},\n", sum.readers));
    s.push_str(&format!("  \"children\": {},\n", sum.children));
    s.push_str(&format!("  \"base_triples_target\": {},\n", sum.base_triples_target));
    s.push_str(&format!("  \"base_triples\": {},\n", sum.base_triples));
    s.push_str(&format!("  \"base_rss_mb\": {:.0},\n", sum.base_rss_mb));
    s.push_str(&format!("  \"committed\": {},\n", sum.committed));
    s.push_str(&format!("  \"wall_secs\": {:.3},\n", sum.wall_secs));
    s.push_str(&format!("  \"throughput_per_s\": {:.1},\n", sum.throughput_per_s));
    s.push_str(&format!("  \"p50_ms\": {:.3},\n", sum.p50_ms));
    s.push_str(&format!("  \"p99_ms\": {:.3},\n", sum.p99_ms));
    s.push_str(&format!("  \"max_ms\": {:.3},\n", sum.max_ms));
    s.push_str(&format!("  \"final_graph_triples\": {},\n", sum.final_graph_triples));
    s.push_str(&format!("  \"reader_queries\": {},\n", sum.reader_queries));
    s.push_str(&format!("  \"final_rss_mb\": {:.0},\n", sum.final_rss_mb));
    s.push_str(&format!(
        "  \"reference_p99_verdict\": {},\n",
        json_str(&sum.reference_p99_verdict)
    ));
    s.push_str(&format!(
        "  \"qlever_baseline_ms\": {},\n",
        opt_num(sum.qlever_baseline_ms)
    ));
    s.push_str(&format!("  \"tolerance\": {:.3},\n", sum.tolerance));
    s.push_str(&format!("  \"parity_bar_ms\": {},\n", opt_num(sum.parity_bar_ms)));
    s.push_str(&format!("  \"parity_verdict\": {},\n", json_str(&sum.parity_verdict)));
    s.push_str("  \"windows\": [\n");
    for (i, w) in windows.iter().enumerate() {
        let comma = if i + 1 < windows.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"from\": {}, \"to\": {}, \"throughput_per_s\": {:.1}, \"p50_us\": {}, \
             \"p99_us\": {}, \"max_us\": {}, \"rss_mb\": {:.0} }}{comma}\n",
            w.from, w.to, w.throughput_per_s, w.p50_us, w.p99_us, w.max_us, w.rss_mb,
        ));
    }
    s.push_str("  ]\n");
    s.push_str("}\n");
    s
}

/// CRUD stream: round-robin over a resource pool, cycling put → set_acl → put → delete so
/// every shape (and a real DELETE) is exercised; latency is the synchronous `apply_update`
/// commit time (group-commit window + delta apply + publish) on the single writer.
///
/// HONESTY — this is a SINGLE synchronous client (one in-flight update at a time), which is
/// exactly the interactive PSS shape: each LDP request blocks on its own commit. So the
/// **binding metric is p99 commit latency** (it must sit inside the LDP request budget); the
/// per-client *throughput* is `1/group-commit-window`-bounded only when adaptive commit is
/// disabled. [SONNET-4.6] (sq-p7kk5) This harness uses `ServerConfig::default()`, whose
/// `adaptive_commit` is ON and is carried into the writer config, so a serial client commits
/// in engine time and is therefore engine-bound. Aggregate write throughput rises with
/// CONCURRENT clients filling the window — that ceiling is `writer_spike` (group-commit
/// throughput at `max_batch` 1/16/256), not this binary. Do NOT read the single-client
/// `updates/s` here as the writer's concurrent capacity.
fn crud_stream(state: &AppState, updates: usize, resources: usize, windows: &mut Vec<WindowRow>) -> Run {
    let pod = 0usize;
    let mut lat_us = Vec::with_capacity(updates);
    let window = 1_000usize;
    let mut window_lat = Vec::with_capacity(window);
    let start = Instant::now();
    let mut window_start = Instant::now();
    for i in 0..updates {
        let res = i % resources;
        let version = i / resources + 1;
        let upd = match i % 4 {
            0 | 2 => put_document(pod, res, version),
            1 => set_acl(pod, res, version),
            _ => delete_document(pod, res),
        };
        let t = Instant::now();
        state.apply_update(&upd).expect("update committed");
        let us = t.elapsed().as_micros() as u64;
        lat_us.push(us);
        window_lat.push(us);
        if window_lat.len() == window {
            window_lat.sort_unstable();
            let from = i + 1 - window;
            let to = i + 1;
            let throughput = window as f64 / window_start.elapsed().as_secs_f64();
            let p50 = window_lat[window / 2];
            let p99 = window_lat[window * 99 / 100];
            let max = window_lat[window - 1];
            let rss = rss_mb();
            println!(
                "updates {:>7}-{:>7}: thr={:>7.0}/s  p50={:>5}us  p99={:>6}us  max={:>8}us  RSS={:>5.0}MB",
                from, to, throughput, p50, p99, max, rss
            );
            windows.push(WindowRow {
                from,
                to,
                throughput_per_s: throughput,
                p50_us: p50,
                p99_us: p99,
                max_us: max,
                rss_mb: rss,
            });
            window_lat.clear();
            window_start = Instant::now();
        }
    }
    Run {
        lat_us,
        updates,
        wall: start.elapsed(),
    }
}

/// Provisioning bursts: one provisioning UPDATE per pod, `updates` pods. Each commit is the
/// heaviest single interactive write PSS issues.
fn provision_bursts(state: &AppState, updates: usize, children: usize) -> Run {
    let mut lat_us = Vec::with_capacity(updates);
    let start = Instant::now();
    for pod in 0..updates {
        let upd = provision(pod, children);
        let t = Instant::now();
        state.apply_update(&upd).expect("provision committed");
        lat_us.push(t.elapsed().as_micros() as u64);
    }
    Run {
        lat_us,
        updates,
        wall: start.elapsed(),
    }
}

fn main() {
    let mut profile = String::from("crud");
    let mut updates = 20_000usize;
    let mut resources = 2_000usize;
    let mut readers = 0usize;
    let mut children = 8usize;
    // Base graph: the pre-existing working set the writer's delta-overlay folds against. The
    // 1M default is a STRESS case (a large pod-server tenant) that exercises the periodic
    // O(graph) compaction; PSS's real per-pod working set is KBs–MBs, so lower it (e.g.
    // `--base-triples 50000`) to model a realistic interactive tenant.
    let mut base_triples_target = 1_000_000usize;
    let mut qlever_baseline_ms: Option<f64> = None;
    let mut tolerance = 1.0f64;
    // Strictly-additive: pull `--json <path>` out before the spike's own flag parse, so the
    // existing flag loop is unaffected when the flag is absent.
    let (argv, json_path) = take_json_flag(std::env::args().collect());
    let mut args = argv.into_iter().skip(1);
    while let Some(a) = args.next() {
        let mut next = || args.next().expect("flag value");
        match a.as_str() {
            "--profile" => profile = next(),
            "--updates" => updates = next().parse().expect("number"),
            "--resources" => resources = next().parse().expect("number"),
            "--readers" => readers = next().parse().expect("number"),
            "--children" => children = next().parse().expect("number"),
            "--base-triples" => base_triples_target = next().parse().expect("number"),
            "--qlever-baseline-ms" => qlever_baseline_ms = Some(next().parse().expect("float")),
            "--tolerance" => tolerance = next().parse().expect("float"),
            other => panic!("unknown arg {other}"),
        }
    }
    // Honest reader count clamp: 0 readers (writer-only ceiling) is the default; readers
    // exercise the "readers never block the writer" invariant under concurrent write load.
    resources = resources.max(1);

    let mut sum = Summary {
        profile: profile.clone(),
        updates,
        resources,
        readers,
        children,
        base_triples_target,
        qlever_baseline_ms,
        tolerance,
        parity_verdict: "n/a".to_string(),
        ..Summary::default()
    };
    let mut windows: Vec<WindowRow> = Vec::new();

    // Build the pre-existing working set (`--base-triples`) of unrelated data so the
    // delta-overlay's periodic O(graph) compaction is exercised against a real graph.
    let mut nt = String::with_capacity(base_triples_target.saturating_mul(48));
    let distinct_subjects = (base_triples_target / 5).max(1);
    for i in 0..base_triples_target {
        nt.push_str(&format!(
            "<http://ex/s{}> <http://ex/p{}> \"v{}\" .\n",
            i % distinct_subjects,
            i % 50,
            i
        ));
    }
    let graph = Graph::load_str(&nt, "ntriples").expect("base graph load");
    let base_triples = graph.len();
    sum.base_triples = base_triples;

    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(5)),
        ..ServerConfig::default()
    };
    let state = AppState::with_config(graph, config);
    let stop = Arc::new(AtomicBool::new(false));

    sum.base_rss_mb = rss_mb();
    println!(
        "# pss_update_throughput — single-writer write-throughput on the PSS update set (gh-52)"
    );
    println!(
        "# profile={profile}  updates={updates}  resources={resources}  readers={readers}  children={children}  base_triples={base_triples_target}"
    );
    println!(
        "# base graph: {base_triples} triples, RSS {:.0}MB\n",
        sum.base_rss_mb
    );

    // Optional concurrent readers: snapshot + point query in a loop (the N-readers side of
    // the contract). They must not stall the writer; their throughput is reported, not gated.
    let state = Arc::new(state);
    let reader_handles: Vec<_> = (0..readers)
        .map(|i| {
            let st = state.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let b = QueryBudget::unlimited();
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    // Pin the current generation (lock-free ~10–20ns); never blocked by the
                    // writer — the N-readers side of the contract.
                    let pin = st.current();
                    let g = pin.snapshot();
                    let s = (n as usize).wrapping_mul(2654435761).wrapping_add(i) % 200_000;
                    let q = format!("SELECT ?o WHERE {{ <http://ex/s{s}> ?p ?o }} LIMIT 5");
                    std::hint::black_box(sparq_engine::query_json_with_budget(g, &q, &b).ok());
                    n += 1;
                }
                n
            })
        })
        .collect();

    let mut run = match profile.as_str() {
        "crud" => crud_stream(&state, updates, resources, &mut windows),
        "provision" => provision_bursts(&state, updates, children),
        other => panic!("unknown profile {other} (use crud|provision)"),
    };

    stop.store(true, Ordering::Relaxed);
    let reads: u64 = reader_handles.into_iter().map(|h| h.join().unwrap()).sum();
    sum.reader_queries = reads;

    run.lat_us.sort_unstable();
    let thr = run.updates as f64 / run.wall.as_secs_f64();
    let p50 = pct(&run.lat_us, 0.50) as f64 / 1000.0;
    let p99 = pct(&run.lat_us, 0.99) as f64 / 1000.0;
    let max = *run.lat_us.last().unwrap_or(&0) as f64 / 1000.0;
    sum.committed = run.updates;
    sum.wall_secs = run.wall.as_secs_f64();
    sum.throughput_per_s = thr;
    sum.p50_ms = p50;
    sum.p99_ms = p99;
    sum.max_ms = max;
    let pin = state.current();
    let g = pin.snapshot();
    sum.final_graph_triples = g.len();
    sum.final_rss_mb = rss_mb();
    println!(
        "\nTOTAL: {} commits in {:.2}s = {:.0} updates/s (SINGLE-client, engine-bound with ServerConfig adaptive_commit ON; not the concurrent write ceiling); p50={:.3}ms p99={:.3}ms max={:.3}ms",
        run.updates,
        run.wall.as_secs_f64(),
        thr,
        p50,
        p99,
        max
    );
    println!(
        "       final graph {} triples (base {base_triples}); concurrent reader queries: {reads}; final RSS {:.0}MB",
        g.len(),
        sum.final_rss_mb
    );

    // Reference targets from gh-52 (NOT the gate — the gate is QLever-parity). The BINDING
    // interactive metric is p99 commit latency (< ~50 ms). This harness uses
    // ServerConfig::default(), whose adaptive_commit is ON, so the single-client result is
    // engine-bound; writer_spike measures the concurrent write ceiling.
    sum.reference_p99_verdict = if p99 < 50.0 { "PASS" } else { "OVER" }.to_string();
    println!(
        "# reference (gh-52, non-gating): p99 {} < ~50ms (BINDING interactive metric); single-client thr {:.0}/s is engine-bound with ServerConfig adaptive_commit ON (writer_spike = concurrent ceiling)",
        sum.reference_p99_verdict,
        thr
    );

    // Parity gate: only when a recorded QLever p99 is supplied. parity-or-better means
    // sparq p99 ≤ QLever p99 × tolerance.
    if let Some(q) = qlever_baseline_ms {
        let bar = q * tolerance;
        sum.parity_bar_ms = Some(bar);
        println!(
            "# parity gate: sparq p99 {p99:.3}ms vs QLever baseline {q:.3}ms × tol {tolerance} = {bar:.3}ms"
        );
        if p99 > bar {
            sum.parity_verdict = "FAIL".to_string();
            // Emit the optional JSON BEFORE exiting non-zero so a recorded run is still captured.
            emit_json(&json_path, &sum, &windows);
            eprintln!(
                "PARITY FAIL: sparq single-writer p99 {p99:.3}ms regressed past QLever-parity bar {bar:.3}ms"
            );
            std::process::exit(1);
        }
        sum.parity_verdict = "PASS".to_string();
        println!(
            "PARITY OK: single-writer p99 is parity-or-better vs QLever on the PSS update set"
        );
    }

    // [OPUS-4.8] (sq-5vm.1) Strictly-additive JSON emit: only when `--json <path>` was given.
    // STDOUT above (the per-window + TOTAL + reference + parity lines) is the unchanged output.
    emit_json(&json_path, &sum, &windows);
}

/// Write the optional `--json` results document, or do nothing when the flag is absent.
/// Factored out so the parity-FAIL early-exit path still captures the recorded run.
fn emit_json(json_path: &Option<String>, sum: &Summary, windows: &[WindowRow]) {
    if let Some(path) = json_path {
        let doc = results_json(sum, windows);
        if let Err(e) = std::fs::write(path, doc) {
            eprintln!("error writing --json results to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote pss_update_throughput results ({} window rows) to {path}", windows.len());
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5vm.1) --json emit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_extraction() {
        let argv: Vec<String> = ["pss_update_throughput", "--profile", "crud", "--json", "/tmp/o.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (positional, path) = take_json_flag(argv);
        assert_eq!(positional, vec!["pss_update_throughput", "--profile", "crud"]);
        assert_eq!(path.as_deref(), Some("/tmp/o.json"));
        let plain: Vec<String> = ["pss_update_throughput", "--profile", "crud"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (p2, none) = take_json_flag(plain.clone());
        assert_eq!(p2, plain);
        assert!(none.is_none());
    }

    #[test]
    fn json_str_escapes_and_opt_num() {
        assert_eq!(json_str("crud"), "\"crud\"");
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(opt_num(None), "null");
        assert_eq!(opt_num(Some(12.5)), "12.500");
    }

    #[test]
    fn results_json_round_trips() {
        // crud run WITH a parity baseline: parity fields are present and numeric.
        let sum = Summary {
            profile: "crud".to_string(),
            updates: 20_000,
            resources: 2_000,
            readers: 0,
            children: 8,
            base_triples_target: 1_000_000,
            base_triples: 1_000_000,
            base_rss_mb: 250.0,
            committed: 20_000,
            wall_secs: 60.0,
            throughput_per_s: 333.0,
            p50_ms: 3.0,
            p99_ms: 4.5,
            max_ms: 12.0,
            final_graph_triples: 1_010_000,
            reader_queries: 0,
            final_rss_mb: 280.0,
            reference_p99_verdict: "PASS".to_string(),
            qlever_baseline_ms: Some(6.0),
            tolerance: 1.0,
            parity_bar_ms: Some(6.0),
            parity_verdict: "PASS".to_string(),
        };
        let windows = vec![
            WindowRow {
                from: 0,
                to: 1_000,
                throughput_per_s: 333.0,
                p50_us: 3_000,
                p99_us: 4_500,
                max_us: 12_000,
                rss_mb: 260.0,
            },
        ];
        let doc = results_json(&sum, &windows);
        // The dependency-free emit must round-trip through a REAL serde_json parse.
        let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
        assert_eq!(v["harness"], "serve-spikes pss_update_throughput");
        let note = v["note"].as_str().unwrap();
        assert!(note.contains("NON-CANONICAL"));
        assert!(note.contains("ServerConfig::default()"));
        assert!(note.contains("adaptive_commit is ON"));
        assert!(note.contains("single-client throughput is engine-bound"));
        assert!(note.contains("not the concurrent write ceiling measured by writer_spike"));
        assert_eq!(v["profile"], "crud");
        assert_eq!(v["updates"], 20_000);
        assert!(v["p99_ms"].is_number());
        assert_eq!(v["reference_p99_verdict"], "PASS");
        assert!(v["qlever_baseline_ms"].is_number());
        assert!(v["parity_bar_ms"].is_number());
        assert_eq!(v["parity_verdict"], "PASS");
        let w = v["windows"].as_array().expect("windows is an array");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0]["from"], 0);
        assert_eq!(w[0]["p99_us"], 4_500);

        // provision run with NO parity baseline: parity numerics are JSON null, verdict "n/a",
        // and the window array is empty — still valid JSON.
        let bare = Summary {
            profile: "provision".to_string(),
            tolerance: 1.0,
            parity_verdict: "n/a".to_string(),
            ..Summary::default()
        };
        let doc2 = results_json(&bare, &[]);
        let v2: serde_json::Value = serde_json::from_str(&doc2).expect("bare is valid JSON");
        assert_eq!(v2["profile"], "provision");
        assert!(v2["qlever_baseline_ms"].is_null());
        assert!(v2["parity_bar_ms"].is_null());
        assert_eq!(v2["parity_verdict"], "n/a");
        assert!(v2["windows"].as_array().unwrap().is_empty());
    }
}

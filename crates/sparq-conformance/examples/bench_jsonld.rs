//! sq-hmd7l.15 — the sparq-side runner for the `bench/jsonld`
//! comparative harness (see `bench/jsonld/README.md`). Drives the REAL shipped
//! JSON-LD paths:
//!
//! * `expand` / `flatten` — the native document-level algorithms
//!   (`sparq_jsonld::expand` / `sparq_jsonld::flatten`).
//! * `tordf` — the real oxjsonld ingest path (the same parser `sparq-core`'s
//!   `jsonld` feature calls), emitting label-canonicalized sorted N-Quads.
//! * `compact` — sparq's shipped document→compacted-document pipeline:
//!   toRdf (oxjsonld) → `Graph` → `graph_to_jsonld_compact` (the engine's
//!   hand-rolled Compaction Algorithm over RDF). NOTE: this is a DIFFERENT
//!   algorithm from the W3C document-level Compaction peers run — the harness
//!   compares the two at the TASK level (same input + context → a compacted
//!   document that must round-trip to the same RDF dataset) and says so on
//!   every emitted row.
//!
//! The `equal` / `dataset-equal` / `roundtrip-compact` subcommands are the
//! harness's OUTPUT-EQUALITY GATE (exit 0/1): no throughput row is emitted for
//! any engine pair until the corresponding gate has passed on the same inputs.
//! `time` prints one advisory TSV row (wall-clock; NEVER canonical numbers).
//!
//! Build: `cargo build --release -p sparq-conformance --features jsonld-suite
//! --example bench_jsonld` (the example is `required-features`-gated; the
//! default build compiles none of this).

use sparq_conformance::jsonld_bench::{
    dataset_to_nquads, json_ld_equal, jsonld_to_canonical_dataset, nquads_to_canonical_dataset,
    read_context_member, sparq_json_to_serde,
};
use sparq_core::Graph;
use sparq_engine::serialize::graph_to_jsonld_compact;
use sparq_jsonld::{expand, flatten, Json, JsonLdOptions, NoopLoader};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// Fixture documents carry only absolute IRIs, so the base never shapes the
/// output — but oxjsonld requires one, and the peer adapters pin the SAME
/// constant so the comparison stays base-faithful by construction.
const DEFAULT_BASE: &str = "https://w3id.org/sparq/bench/jsonld/";

fn usage() -> String {
    [
        "bench_jsonld — sparq runner for bench/jsonld (requires --features jsonld-suite)",
        "",
        "USAGE:",
        "  bench_jsonld equal <a.json> <b.json>",
        "      JSON-LD data-model deep-equality (the conformance comparator:",
        "      object key order insignificant, array order significant only",
        "      inside @list, numeric-guarded). Exit 0 equal / 1 not equal.",
        "  bench_jsonld dataset-equal <a> <b> [--base IRI]",
        "      Canonical-RDF-dataset equality, blank-node-blind (.nq/.nquads",
        "      parsed as N-Quads, anything else as JSON-LD). Exit 0/1.",
        "  bench_jsonld roundtrip-compact <in.jsonld> <ctx.jsonld> [--base IRI]",
        "      The compact losslessness gate: reparse(compact(D, ctx)) must",
        "      equal D (the input's RDF dataset). Exit 0/1.",
        "  bench_jsonld expand|flatten <in.jsonld> [--base IRI] [--out F]",
        "  bench_jsonld compact <in.jsonld> <ctx.jsonld> [--base IRI] [--out F]",
        "  bench_jsonld tordf <in.jsonld> [--base IRI] [--out F]",
        "  bench_jsonld counts <in.jsonld> [--base IRI]",
        "      Deterministic anchors: `expand_nodes<TAB>k` + `tordf_quads<TAB>n`.",
        "  bench_jsonld time <op> <in.jsonld> [<ctx.jsonld>] [--base IRI]",
        "                    [--iters N] [--warmup W]",
        "      One advisory TSV row: op file bytes iters us_per_op docs_per_s mb_per_s.",
    ]
    .join("\n")
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))
}

fn write_out(out: Option<&str>, text: &str) -> Result<(), String> {
    match out {
        Some(f) => std::fs::write(f, text).map_err(|e| format!("write {}: {}", f, e)),
        None => {
            println!("{}", text);
            Ok(())
        }
    }
}

/// Parse a file into a canonicalized dataset — N-Quads by extension, else JSON-LD.
fn file_to_canonical_dataset(path: &str, base: &str) -> Result<oxrdf::Dataset, String> {
    let text = read(path)?;
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if ext.eq_ignore_ascii_case("nq") || ext.eq_ignore_ascii_case("nquads") {
        nquads_to_canonical_dataset(&text)
    } else {
        jsonld_to_canonical_dataset(&text, base)
    }
}

/// Native document-level expand → serialized expanded document.
fn run_expand(text: &str, base: &str) -> Result<Json, String> {
    let input = Json::parse(text).map_err(|e| format!("parse input JSON: {:?}", e))?;
    let mut opts = JsonLdOptions::default();
    opts.base = Some(base.to_string());
    expand(&input, &opts, &NoopLoader).map_err(|e| format!("expand: {}", e))
}

/// Native document-level flatten (returns the flattened EXPANDED form).
fn run_flatten(text: &str, base: &str) -> Result<Json, String> {
    let input = Json::parse(text).map_err(|e| format!("parse input JSON: {:?}", e))?;
    let mut opts = JsonLdOptions::default();
    opts.base = Some(base.to_string());
    flatten(&input, &opts, &NoopLoader).map_err(|e| format!("flatten: {}", e))
}

/// sparq's document→compacted-document pipeline: toRdf (oxjsonld, streaming,
/// NO canonicalization — that is oracle work, not task work) → `Graph` →
/// `graph_to_jsonld_compact`.
fn run_compact(
    text: &str,
    ctx: &sparq_engine::serialize::JsonLdValue,
    base: &str,
) -> Result<String, String> {
    let parser = oxjsonld::JsonLdParser::new()
        .with_base_iri(base)
        .map_err(|e| format!("invalid base {}: {}", base, e))?;
    let mut nq = String::new();
    for q in parser.for_slice(text.as_bytes()) {
        let q = q.map_err(|e| format!("toRdf: {}", e))?;
        nq.push_str(&format!("{} .\n", q));
    }
    let graph = Graph::load_dataset(&nq, "nquads")?;
    Ok(graph_to_jsonld_compact(&graph, ctx))
}

/// Label-canonicalized, line-sorted N-Quads (the cross-engine toRdf artifact).
fn run_tordf(text: &str, base: &str) -> Result<String, String> {
    let ds = jsonld_to_canonical_dataset(text, base)?;
    let mut lines: Vec<String> = dataset_to_nquads(&ds).lines().map(str::to_string).collect();
    lines.sort_unstable();
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

fn json_to_string(j: &Json) -> String {
    let mut buf = String::new();
    j.write(&mut buf);
    buf
}

/// Flag parsing: strips `--base/--out/--iters/--warmup` pairs out of `args`,
/// returning (positional, base, out, iters, warmup).
#[allow(clippy::type_complexity)]
fn split_args(args: &[String]) -> Result<(Vec<String>, String, Option<String>, u32, u32), String> {
    let mut pos = Vec::new();
    let mut base = DEFAULT_BASE.to_string();
    let mut out = None;
    let mut iters: u32 = 20;
    let mut warmup: u32 = 3;
    let mut i = 0;
    while i < args.len() {
        let take = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[*i - 1]))
        };
        match args[i].as_str() {
            "--base" => base = take(&mut i)?,
            "--out" => out = Some(take(&mut i)?),
            "--iters" => {
                iters = take(&mut i)?
                    .parse()
                    .map_err(|e| format!("--iters: {}", e))?
            }
            "--warmup" => {
                warmup = take(&mut i)?
                    .parse()
                    .map_err(|e| format!("--warmup: {}", e))?
            }
            other => pos.push(other.to_string()),
        }
        i += 1;
    }
    Ok((pos, base, out, iters, warmup))
}

fn run() -> Result<bool, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        return Err(usage());
    };
    let (pos, base, out, iters, warmup) = split_args(&argv[1..])?;

    match cmd.as_str() {
        "equal" => {
            let [a, b] = &pos[..] else {
                return Err("equal <a.json> <b.json>".into());
            };
            let va: serde_json::Value =
                serde_json::from_str(&read(a)?).map_err(|e| format!("parse {}: {}", a, e))?;
            let vb: serde_json::Value =
                serde_json::from_str(&read(b)?).map_err(|e| format!("parse {}: {}", b, e))?;
            let eq = json_ld_equal(&va, &vb);
            println!(
                "json-ld-equal\t{}\t{}\t{}",
                a,
                b,
                if eq { "EQUAL" } else { "NOT-EQUAL" }
            );
            Ok(eq)
        }
        "dataset-equal" => {
            let [a, b] = &pos[..] else {
                return Err("dataset-equal <a> <b> [--base IRI]".into());
            };
            let da = file_to_canonical_dataset(a, &base)?;
            let db = file_to_canonical_dataset(b, &base)?;
            let eq = da == db;
            println!(
                "dataset-equal\t{}\t{}\t{}\t{}quads",
                a,
                b,
                if eq { "EQUAL" } else { "NOT-EQUAL" },
                da.len()
            );
            Ok(eq)
        }
        "roundtrip-compact" => {
            let [input, ctx_file] = &pos[..] else {
                return Err("roundtrip-compact <in.jsonld> <ctx.jsonld> [--base IRI]".into());
            };
            let text = read(input)?;
            let want = jsonld_to_canonical_dataset(&text, &base)?;
            let ctx = read_context_member(Path::new(ctx_file))?;
            let compacted = run_compact(&text, &ctx, &base)?;
            let got = jsonld_to_canonical_dataset(&compacted, &base)?;
            let eq = got == want;
            println!(
                "roundtrip-compact\t{}\t{}\t{}quads",
                input,
                if eq { "LOSSLESS" } else { "NOT-LOSSLESS" },
                want.len()
            );
            Ok(eq)
        }
        "expand" | "flatten" => {
            let [input] = &pos[..] else {
                return Err(format!("{} <in.jsonld> [--out F]", cmd));
            };
            let text = read(input)?;
            let doc = if cmd == "expand" {
                run_expand(&text, &base)?
            } else {
                run_flatten(&text, &base)?
            };
            write_out(out.as_deref(), &json_to_string(&doc))?;
            Ok(true)
        }
        "compact" => {
            let [input, ctx_file] = &pos[..] else {
                return Err("compact <in.jsonld> <ctx.jsonld> [--out F]".into());
            };
            let text = read(input)?;
            let ctx = read_context_member(Path::new(ctx_file))?;
            write_out(out.as_deref(), &run_compact(&text, &ctx, &base)?)?;
            Ok(true)
        }
        "tordf" => {
            let [input] = &pos[..] else {
                return Err("tordf <in.jsonld> [--out F]".into());
            };
            let text = read(input)?;
            write_out(out.as_deref(), &run_tordf(&text, &base)?)?;
            Ok(true)
        }
        "counts" => {
            let [input] = &pos[..] else {
                return Err("counts <in.jsonld> [--base IRI]".into());
            };
            let text = read(input)?;
            let expanded = run_expand(&text, &base)?;
            let v = sparq_json_to_serde(&expanded)?;
            let nodes = v
                .as_array()
                .map(Vec::len)
                .ok_or("expanded form is not an array")?;
            let quads = jsonld_to_canonical_dataset(&text, &base)?.len();
            println!("expand_nodes\t{}", nodes);
            println!("tordf_quads\t{}", quads);
            Ok(true)
        }
        "time" => {
            let (op, input, ctx_file) = match &pos[..] {
                [op, input] => (op.clone(), input.clone(), None),
                [op, input, ctx] => (op.clone(), input.clone(), Some(ctx.clone())),
                _ => {
                    return Err(
                        "time <op> <in.jsonld> [<ctx.jsonld>] [--iters N] [--warmup W]".into(),
                    )
                }
            };
            let text = read(&input)?;
            let bytes = text.len();
            let ctx = match (&op[..], &ctx_file) {
                ("compact", Some(f)) => Some(read_context_member(Path::new(f))?),
                ("compact", None) => return Err("time compact needs <ctx.jsonld>".into()),
                _ => None,
            };
            // The timed task: in-memory JSON text → operation result (JSON parse
            // included on every iteration — the peer adapters do the same).
            let run_once = || -> Result<(), String> {
                match op.as_str() {
                    "expand" => run_expand(&text, &base).map(|_| ()),
                    "flatten" => run_flatten(&text, &base).map(|_| ()),
                    "compact" => run_compact(&text, ctx.as_ref().unwrap(), &base).map(|_| ()),
                    "tordf" => run_tordf(&text, &base).map(|_| ()),
                    other => Err(format!("unknown op {}", other)),
                }
            };
            for _ in 0..warmup {
                run_once()?;
            }
            let start = Instant::now();
            for _ in 0..iters.max(1) {
                run_once()?;
            }
            let total = start.elapsed();
            let us_per_op = total.as_micros() as f64 / f64::from(iters.max(1));
            let docs_per_s = 1_000_000.0 / us_per_op;
            let mb_per_s = (bytes as f64 / 1_000_000.0) * docs_per_s;
            // Advisory TSV (work-box wall-clock — NEVER a canonical number).
            println!(
                "jsonld_{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.2}",
                op, input, bytes, iters, us_per_op, docs_per_s, mb_per_s
            );
            Ok(true)
        }
        _ => Err(usage()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(msg) => {
            eprintln!("{}", msg);
            ExitCode::from(2)
        }
    }
}

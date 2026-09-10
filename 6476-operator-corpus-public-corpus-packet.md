# PR6478 registered operator corpus execution

Exact source: `d07ca79f89e3a8945be46b516c3cd2f770ccd618`, clean. GPT-6 Astra performed this bounded local validation.

The unchanged registered 28-query corpus completed in count, materialize and JSON modes, three executions per query/mode: **84 records / 252 successful invocations**. The exact repository generator created 2,000 entities, 16,000 N-Triples statements; the loaded graph contained 15,993 distinct triples in all modes. Returned sizes agreed across all three iterations, count/materialize agreed for all queries, and graph-form triple counts agreed across modes. This is corpus/API execution, not the full CLI benchmark workflow or an independent answer oracle.

The harness preserves the CLI graph-form classification and dispatches to public count/query/query_json/construct_or_describe APIs. JSON lengths are bytes; graph forms use triple counts in every mode. No timing or allocation result is reported. Default engine features and System allocator with one Rayon worker differ from the CLI's unified dependency feature set and mimalloc.

The task-private invalid-q13 copy failed with exit2, an exact q13 error, 12 preceding success records, and no completion marker. The original registered files stayed unchanged.

Production binary provenance:

- Harness SHA256: `3dd2c00eb92db02b75aaa84ff4cef727563b394d90effa8431cd0b82a51f74cb`
- Fresh engine rlib SHA256: `2f17e066de1c77dd975ec2367c6e140175a31710685b05d8aadccd04f3b7b395`
- Engine git tree: `d793fe496cf292f9994f58302af32d334a38d135`
- Generator SHA256: `d39caebec4dad6fe0446812d4dac8bc2ab228abf48f19d3f29823407872a8d77`
- Dataset SHA256: `7d9987d12b85351aecd471c0f6777b288d6142fb921e0c77c282ebc11edd8e28`
- Engine compiled features: `default,digest,parallel,regex`

The build log records fresh engine and unique harness rustc invocations and Finished in32.41s. A resource-monitor wrapper failed on a disappearing rustc temporary file before collecting Cargo's exit status; that status is unknown, and the remaining build interval was not continuously observed. No build was repeated. The completed binary and all executed command outcomes are preserved. Final observed aggregate task growth was287,457,280B under536,870,912B; free disk was5,647,593,472B above2GiB. Full local diagnostics retain the exact limitation.

## Reproducible local harness

Build with the existing pinned lock/toolchain, offline, jobs1, incremental off, opt-level3, unwind, no LTO, codegen-units16. The local Cargo manifest uses repository path dependencies; no production manifest or dependency was modified. This is a documented reproduction command, not a recovered top-level Cargo exit receipt:

```sh
cargo build --release --locked --offline --manifest-path <task-harness>/Cargo.toml -vv
operator-corpus generate dataset.nt
operator-corpus run <registered-corpus> dataset.nt count
operator-corpus run <registered-corpus> dataset.nt materialize
operator-corpus run <registered-corpus> dataset.nt json
```

Exact source follows; generator source is supplied verbatim separately as `harness/src/dataset.rs`. CLI source is repository-relative `crates/sparq-cli/src/main.rs`, lines2199–2236 and1440–1476, reproduced in `cli-dispatch-source.txt`. Raw compiler/environment logs and absolute local paths are omitted from this public packet; local evidence contains them.

```rust
//! [GPT-6 Astra] Registered corpus execution without timing or allocation claims.

#[allow(dead_code)]
mod dataset;

use sparq_core::Graph;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "generate" {
        let triples = dataset::write_nt(2000, &args[2]).map_err(|e| e.to_string())?;
        println!("GENERATED\t2000\t{triples}");
        return Ok(());
    }
    if args.len() != 5 || args[1] != "run" {
        return Err("expected generate <dataset> or run <corpus-dir> <dataset> <mode>".into());
    }
    let mode = args[4].as_str();
    if !["count", "materialize", "json"].contains(&mode) {
        return Err("unknown mode".into());
    }
    // Bound local work independently of runner hardware; this is not a latency comparison.
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .map_err(|e| e.to_string())?;
    let mut entries = std::fs::read_dir(&args[2])
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.retain(|path| path.extension().is_some_and(|ext| ext == "rq"));
    entries.sort();
    let names: Vec<_> = entries
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let expected: Vec<_> = include_str!("expected-names.txt").lines().collect();
    if names != expected || entries.len() != 28 {
        return Err("registered query-name set is incomplete or changed".into());
    }
    // Same streaming N-Triples loader as the CLI's raw-format branch, raw store profile.
    let reader = BufReader::new(File::open(&args[3]).map_err(|e| e.to_string())?);
    let graph = Graph::load_reader_parallel(reader, "ntriples")?;
    eprintln!("LOADED\t{}", graph.len());
    for path in entries {
        let name = Path::new(&path).file_stem().unwrap().to_string_lossy();
        let sparql = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        // Match the CLI's graph-form classification and public-API dispatch below.
        let graph_form = sparq_engine::PreparedQuery::parse(&sparql)
            .map(|p| p.is_graph_form())
            .unwrap_or(false);
        let mut values = [0; 3];
        for value in &mut values {
            let r: Result<usize, String> = if graph_form {
                sparq_engine::construct_or_describe(&graph, &sparql).map(|ts| ts.len())
            } else {
                match mode {
                    "count" => sparq_engine::count(&graph, &sparql),
                    "json" => sparq_engine::query_json(&graph, &sparql).map(|s| {
                        let n = s.len();
                        std::hint::black_box(s);
                        n
                    }),
                    _ => sparq_engine::query(&graph, &sparql).map(|r| r.len()),
                }
            };
            *value = r.map_err(|error| format!("{mode}/{name}: {error}"))?;
        }
        if values.iter().any(|value| *value != values[0]) {
            return Err(format!(
                "{mode}/{name}: returned-size disagreement across iterations"
            ));
        }
        let unit = if graph_form {
            "triples"
        } else if mode == "json" {
            "json_bytes"
        } else if mode == "count" {
            "count_result"
        } else {
            "solution_rows"
        };
        println!(
            "RESULT\t{mode}\t{name}\t{unit}\t{}\t{}\t{}",
            values[0], values[1], values[2]
        );
    }
    println!("COMPLETED\t{mode}\t28");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("OPERATOR_CORPUS_ERROR: {error}");
        std::process::exit(2);
    }
}
```

## All returned sizes

Each cell is the identical value observed in each of three iterations. JSON columns count bytes except the two graph forms, which count triples.

| Query | Count | Materialize | JSON bytes / graph triples |
|---|---:|---:|---:|
| q01_bgp | 1 | 1 | 95 |
| q02_star3 | 1 | 1 | 247 |
| q03_chain | 16 | 16 | 2236 |
| q04_triangle | 1 | 1 | 156 |
| q05_union | 2 | 2 | 139 |
| q06_optional | 2 | 2 | 417 |
| q07_optional_notbound | 10 | 10 | 498 |
| q08_minus | 3 | 3 | 183 |
| q09_filter_numeric | 75 | 75 | 10440 |
| q10_filter_string | 2 | 2 | 227 |
| q11_filter_in | 3 | 3 | 319 |
| q12_filter_exists | 3 | 3 | 183 |
| q13_bind | 1 | 1 | 295 |
| q14_values | 3 | 3 | 319 |
| q15_agg_group_having | 3 | 3 | 1569 |
| q16_distinct | 4 | 4 | 244 |
| q17_orderby_limit_offset | 5 | 5 | 749 |
| q18_path_plus | 1 | 1 | 26 |
| q19_path_star | 1 | 1 | 150 |
| q20_path_opt | 5 | 5 | 278 |
| q21_path_seq | 4 | 4 | 244 |
| q22_path_alt | 2 | 2 | 182 |
| q23_path_inverse | 1 | 1 | 153 |
| q24_path_negated_pset | 4 | 4 | 275 |
| q25_subquery | 3 | 3 | 492 |
| q26_ask | 1 | 1 | 26 |
| q27_construct | 4 | 4 | 4 |
| q28_describe | 16 | 16 | 16 |

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

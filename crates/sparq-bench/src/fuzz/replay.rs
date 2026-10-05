//! [GPT-6] Retain existing fuzz seeds and native storage observations, never proofs.

use std::collections::BTreeMap;
use std::path::Path;

use oxigraph::store::Store;
use serde_json::{json, Value};
use sparq_core::Graph;

use super::{
    compare_ask, compare_graph, compare_select, gen_graph, gen_query, is_bnode_iri_inequality,
    oxi_count, query_form, spec_filter_count, Compared, DivergenceAllowlist, Form, Rng, CATEGORIES,
};

const MODES: &[&str] = &["dense", "fork", "compressed", "mmap", "mmap-compressed", "dict-spill"];

fn diagnostic(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn dataset(g: &Graph) -> Vec<String> {
    let scan = g.store.scan(&[None, None, None]);
    let mut triples: Vec<_> = scan.rows.iter().map(|row| {
        let spo = scan.to_spo(row);
        format!("{} {} {} .", g.dict.term(spo[0]), g.dict.term(spo[1]), g.dict.term(spo[2]))
    }).collect();
    triples.sort();
    triples
}

fn storage(ttl: &str, mode: &str, path: &Path) -> Result<Graph, String> {
    let graph = Graph::load_str(ttl, "turtle").map_err(diagnostic)?;
    match mode {
        "dense" => Ok(graph),
        "fork" => Ok(graph.fork()),
        "compressed" => Ok(graph.into_compressed()),
        "mmap" | "mmap-compressed" => {
            if mode == "mmap" {
                graph.save(path).map_err(diagnostic)?;
            } else {
                graph.save_compressed(path).map_err(diagnostic)?;
            }
            Graph::open(path).map_err(diagnostic)
        }
        "dict-spill" => {
            // Same tiny dictionary-spill budget as the original fuzz mode.
            let nt = dataset(&graph).join("\n") + "\n";
            let config = sparq_core::dictspill::SpillConfig { mem_budget: 1, disk_floor: 0 };
            Graph::build_external_spill(nt.as_bytes(), "ntriples", path, 64, &config)
                .map_err(diagnostic)?;
            Graph::open(path).map_err(diagnostic)
        }
        _ => Err(format!("unknown storage mode: {mode}")),
    }
}

// Preserve exact RDF spelling, row multiplicity, unbound cells and observed order.
// This is intentionally separate from the neutral comparator's value normalization.
fn native_result(graph: &Graph, query: &str) -> Result<Value, String> {
    match query_form(query) {
        Form::Select => {
            let result = sparq_engine::query(graph, query).map_err(diagnostic)?;
            let variables: Vec<_> = result.vars.iter().map(|v| v.as_str()).collect();
            let rows: Vec<BTreeMap<_, _>> = result.rows.iter().map(|row| {
                variables.iter().zip(row).map(|(v, t)| {
                    (*v, t.as_ref().map(ToString::to_string))
                }).collect()
            }).collect();
            Ok(json!({"kind":"select", "variables":variables, "rows":rows}))
        }
        Form::Ask => Ok(json!({"kind":"ask", "value":sparq_engine::ask(graph, query).map_err(diagnostic)?})),
        Form::Graph => {
            let mut triples: Vec<_> = sparq_engine::construct_or_describe(graph, query)
                .map_err(diagnostic)?.iter().map(ToString::to_string).collect();
            triples.sort();
            Ok(json!({"kind":"graph", "triples":triples}))
        }
    }
}

// The legacy Store API is also the original fuzz oracle. Keep that pinned behavior.
#[expect(deprecated, reason = "reuse the existing differential oracle API")]
fn oracle_result(store: &Store, query: &str) -> Result<Value, String> {
    match store.query(query).map_err(diagnostic)? {
        oxigraph::sparql::QueryResults::Solutions(solutions) => {
            let variables: Vec<String> = solutions.variables().iter().map(|v| v.as_str().to_owned()).collect();
            let mut rows = Vec::new();
            for solution in solutions {
                let solution = solution.map_err(diagnostic)?;
                let row: BTreeMap<_, _> = variables.iter().map(|v| {
                    (v, solution.get(v.as_str()).map(ToString::to_string))
                }).collect();
                rows.push(row);
            }
            Ok(json!({"kind":"select", "variables":variables, "rows":rows}))
        }
        oxigraph::sparql::QueryResults::Boolean(value) => Ok(json!({"kind":"ask", "value":value})),
        oxigraph::sparql::QueryResults::Graph(triples) => {
            let mut rows = Vec::new();
            for triple in triples {
                rows.push(triple.map_err(diagnostic)?.to_string());
            }
            rows.sort();
            Ok(json!({"kind":"graph", "triples":rows}))
        }
    }
}

fn verdict(result: Result<Compared, String>) -> Value {
    match result {
        Ok(Compared::Multiset) => json!({"status":"agreement", "comparator":"multiset"}),
        Ok(Compared::Ordered) => json!({"status":"agreement", "comparator":"order-equivalence-classes"}),
        Ok(Compared::Isomorphic) => json!({"status":"agreement", "comparator":"global-blank-isomorphism-order-unchecked"}),
        Ok(Compared::AskBoolean) => json!({"status":"agreement", "comparator":"ask-boolean"}),
        Ok(Compared::GraphIsomorphic) => json!({"status":"agreement", "comparator":"graph-isomorphism"}),
        Ok(Compared::SkippedRowChoice) => json!({"status":"no-agreement", "reason":"arbitrary-window-row-choice"}),
        Ok(Compared::TriageIso(reason)) => json!({"status":"no-agreement", "reason":"isomorphism-triage", "detail":reason}),
        // No caller is allowed to relabel a missing implementation as profile rejection.
        Ok(Compared::Unsupported) => json!({"status":"unclassified-execution", "reason":"existing-comparator-could-not-decode"}),
        Err(detail) => json!({"status":"mismatch", "detail":detail}),
    }
}

fn compare(graph: &Graph, store: &Store, query: &str, allow: &DivergenceAllowlist) -> Value {
    match query_form(query) {
        Form::Ask => verdict(compare_ask(graph, store, query)),
        Form::Graph => verdict(compare_graph(graph, store, query)),
        Form::Select => {
            let native = sparq_engine::query(graph, query).map(|r| r.len());
            let reference = oxi_count(store, query);
            let (Ok(native), Ok(reference)) = (native, reference) else {
                return json!({"status":"unclassified-execution", "reason":"cardinality-evaluation-failed"});
            };
            match sparq_engine::count(graph, query) {
                Ok(count) if count == native => {}
                Ok(count) => return json!({"status":"mismatch", "reason":"count-path", "native_count":native, "count":count}),
                Err(error) => return json!({"status":"unclassified-execution", "reason":"count-path", "detail":error}),
            }
            if native != reference {
                let spec = allow.cross_family_eq_type_error.then(|| spec_filter_count(store, query)).flatten();
                if spec == Some(native) {
                    return json!({"status":"adjudicated-count-only", "class":"cross-family-eq-type-error",
                        "native_count":native, "reference_count":reference, "spec_count":spec});
                }
                if spec.is_none() && allow.bnode_iri_inequality && is_bnode_iri_inequality(store, query) {
                    return json!({"status":"no-agreement", "reason":"adjudicated-bnode-iri-inequality"});
                }
                return json!({"status":"mismatch", "native_count":native, "reference_count":reference, "spec_count":spec});
            }
            verdict(compare_select(graph, store, query))
        }
    }
}

/// Run one original seed through one explicit storage mode.
///
/// Errors retain partial files and never report native execution as a proof.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 4 {
        return Err("usage: fuzz-replay SEED CATEGORY MODE NEW_OUTPUT_DIRECTORY".into());
    }
    let seed: u64 = args[0].parse().map_err(diagnostic)?;
    let category = args[1].as_str();
    let mode = args[2].as_str();
    if category != "all" && !CATEGORIES.contains(&category) {
        return Err(format!("unknown original category: {category}"));
    }
    if !MODES.contains(&mode) {
        return Err(format!("unknown storage mode: {mode}"));
    }
    let output = Path::new(&args[3]);
    std::fs::create_dir(output).map_err(diagnostic)?;
    let mut rng = Rng::new(seed);
    let ttl = gen_graph(&mut rng);
    let query = gen_query(&mut rng, category);
    std::fs::write(output.join("data.ttl"), &ttl).map_err(diagnostic)?;
    std::fs::write(output.join("query.rq"), &query).map_err(diagnostic)?;
    let path = output.join("store");
    let graph = storage(&ttl, mode, &path)?;
    let data = dataset(&graph);
    let store = Store::new().map_err(diagnostic)?;
    store.load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes()).map_err(diagnostic)?;
    let allow = DivergenceAllowlist::load();
    let native = native_result(&graph, &query);
    let reference = oracle_result(&store, &query);
    let comparison = compare(&graph, &store, &query, &allow);
    let record = json!({
        "schema":"sparq.engine-seed-replay.v1", "seed":seed, "category":category, "storage":mode,
        "input":{"query":query, "dataset":ttl, "format":"turtle", "named_graph_catalog":[]},
        "observed_ntriples":data, "native_result":native.as_ref().ok(), "reference_result":reference.as_ref().ok(),
        "native_error":native.as_ref().err(), "reference_error":reference.as_ref().err(),
        "comparison":comparison, "divergence_registry":{
            "source":allow.source, "cross_family_eq_type_error":allow.cross_family_eq_type_error,
            "bnode_iri_inequality":allow.bnode_iri_inequality},
        "proof_bridge":{"status":"not-prepared", "backend":null, "authority":null,
            "public_statement_sha256":null, "private_witness_sha256":null, "reuse_allowed":false},
        "proof_count":0, "verified_proof_count":0
    });
    std::fs::write(output.join("record.json"), serde_json::to_vec_pretty(&record).map_err(diagnostic)?)
        .map_err(diagnostic)?;
    drop(graph);
    // Remove only the store created inside this new output; retain replay inputs/results.
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(diagnostic)?;
    }
    if native.is_err() || reference.is_err() || matches!(comparison["status"].as_str(), Some("mismatch" | "unclassified-execution")) {
        return Err("retained native/oracle failure; inspect record.json".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_never_become_full_result_agreement() {
        for compared in [Compared::SkippedRowChoice, Compared::Unsupported, Compared::TriageIso("budget".into())] {
            assert_ne!(verdict(Ok(compared))["status"], "agreement");
        }
        assert_eq!(verdict(Err("wrong result".into()))["status"], "mismatch");
    }

    #[test]
    fn every_storage_mode_retains_original_seed_and_raw_dataset() {
        // Small original fixture, including raw typed literals; no new oracle.
        let mut rng = Rng::new(0);
        let ttl = gen_graph(&mut rng);
        let query = gen_query(&mut rng, "bgp");
        let original = Graph::load_str(&ttl, "turtle").unwrap();
        let expected = dataset(&original);
        let base = std::env::temp_dir().join(format!("sparq_seed_replay_test_{}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        for mode in MODES {
            let output = base.join(mode);
            run(&["0".into(), "bgp".into(), (*mode).into(), output.display().to_string()]).unwrap();
            let record: Value = serde_json::from_slice(&std::fs::read(output.join("record.json")).unwrap()).unwrap();
            assert_eq!(record["input"]["query"], query);
            assert_eq!(record["input"]["dataset"], ttl);
            assert_eq!(record["observed_ntriples"], json!(expected));
            assert_eq!(record["comparison"]["status"], "agreement");
            assert_eq!(record["proof_count"], 0);
            assert!(!output.join("store").exists());
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}

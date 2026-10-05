// [GPT-6] Export existing W3C manifests and goldens without evaluating queries.
//! Run `proof_corpus MANIFEST SUITE_ROOT OUTPUT_JSON` against an existing suite checkout.
use serde_json::{Value, json};
use spargebra::{Query, SparqlParser, algebra::GraphPattern};
use sparq_conformance::{manifest, rdf, results};
use std::{collections::BTreeSet, error::Error, fs, io::Write, path::Path};

fn ordered(pattern: &GraphPattern) -> bool {
    match pattern {
        GraphPattern::OrderBy { .. } => true,
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => ordered(inner),
        _ => false,
    }
}

fn source(path: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(json!({"path":path, "bytes":fs::read(path)?}))
}

fn export(entry: manifest::TestEntry) -> Result<Value, Box<dyn Error>> {
    let mut record = json!({"id":entry.id, "name":entry.name, "suite":entry.suite,
        "oracle":{"kind":"w3c_manifest_golden"}, "template":"existing_w3c",
        "candidate_terms":[], "triples":[], "original_fixture":{"withdrawn":entry.withdrawn}});
    let manifest::EntryKind::QueryEval = entry.kind else {
        record["classification"] = json!({"status":"excluded", "reason":format!("W3C entry kind {:?} is not a read-query evaluation", entry.kind)});
        return Ok(record);
    };
    if entry.withdrawn || entry.action.unsupported_feature.is_some() {
        record["classification"] = json!({"status":"excluded", "reason":if entry.withdrawn {
            "withdrawn W3C test".into()
        } else {format!("external entailment or service context: {:?}",entry.action.unsupported_feature)}});
    }
    let query_path = entry
        .action
        .query
        .ok_or("query action missing query file")?;
    let result_path = entry
        .result_file
        .ok_or("evaluation test missing original golden")?;
    let query_text = fs::read_to_string(&query_path)?;
    let query = SparqlParser::new()
        .with_base_iri(rdf::file_iri(&query_path))?
        .parse_query(&query_text)?;
    record["query"] = json!(query_text);
    record["original_fixture"]["query"] = source(&query_path)?;
    record["original_fixture"]["golden"] = source(&result_path)?;
    record["original_fixture"]["query_base"] = json!(rdf::file_iri(&query_path));
    if SparqlParser::new().parse_query(&query_text).is_err() {
        record["classification"] = json!({"status":"requires_profile_classification",
            "reason":"Original query requires its external file base; current evaluator request has no external-base field. Query bytes remain unchanged."});
    }
    let mut nquads = String::new();
    let mut triples = Vec::new();
    let mut catalog = BTreeSet::new();
    let mut sources = Vec::new();
    // W3C data documents can contain source-scoped blank labels. Keep each
    // document separately in the export; exact V3 import must standardize those
    // scopes apart rather than silently joining equal source labels.
    let mut source_blank = false;
    for path in &entry.action.data {
        sources.push(json!({"graph":null,"source":source(path)?}));
        for t in rdf::parse_file(path)? {
            source_blank |= t.subject.is_blank_node() || t.object.is_blank_node();
            triples.push([
                t.subject.to_string(),
                t.predicate.to_string(),
                t.object.to_string(),
            ]);
            nquads.push_str(&format!("{t} .\n"));
        }
    }
    let ntriples = nquads.clone();
    for (name, path) in &entry.action.graph_data {
        catalog.insert(name.clone());
        sources.push(json!({"graph":name,"source":source(path)?}));
        for t in rdf::parse_file(path)? {
            source_blank |= t.subject.is_blank_node() || t.object.is_blank_node();
            nquads.push_str(&format!("{t} <{name}> .\n"));
        }
    }
    if source_blank && sources.len() > 1 {
        record["classification"] = json!({"status":"requires_profile_classification",
            "reason":"Multiple RDF source documents contain blank nodes; source-scope-preserving adapter import is required."});
    }
    record["original_fixture"]["data"] = json!(sources);
    record["triples"] = json!(triples);
    record["dataset"] = json!({"ntriples":ntriples,"nquads":nquads,"named_graphs":catalog});
    record["expected"] = match query {
        Query::Select { ref pattern, .. } => {
            let results::Expected::Bindings {
                mut vars,
                rows,
                indexed,
            } = results::parse_expected(&result_path)?
            else {
                return Err("SELECT golden has the wrong result form".into());
            };
            if vars.is_empty() {
                vars = rows
                    .iter()
                    .flat_map(|r| r.iter().map(|(v, _)| v.clone()))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
            }
            let aligned: Vec<Vec<Option<String>>> = rows
                .iter()
                .map(|row| {
                    vars.iter()
                        .map(|v| {
                            row.iter()
                                .find(|(name, _)| name == v)
                                .map(|(_, term)| term.to_string())
                        })
                        .collect()
                })
                .collect();
            json!({"Select":{"variables":vars,"order":if indexed || ordered(pattern) {"Sequence"} else {"Bag"},"rows":aligned}})
        }
        Query::Ask { .. } => {
            let results::Expected::Boolean(value) = results::parse_expected(&result_path)? else {
                return Err("ASK golden has the wrong result form".into());
            };
            json!({"Ask":value})
        }
        Query::Construct { .. } | Query::Describe { .. } => {
            let expected = rdf::parse_file(&result_path)?;
            record["expected_graph_triples"] = json!(
                expected
                    .iter()
                    .map(|t| [
                        t.subject.to_string(),
                        t.predicate.to_string(),
                        t.object.to_string()
                    ])
                    .collect::<Vec<_>>()
            );
            json!({"Graph":{"ntriples":expected.iter().map(|t| format!("{t} .\n")).collect::<String>()}})
        }
    };
    Ok(record)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: proof_corpus MANIFEST SUITE_ROOT NEW_OUTPUT_JSON".into());
    }
    let root = fs::canonicalize(&args[1])?;
    let mut entries = Vec::new();
    manifest::collect(Path::new(&args[0]), &root, &mut entries)?;
    if entries.is_empty() {
        return Err("empty W3C manifest replay".into());
    }
    let mut ids = BTreeSet::new();
    let records = entries
        .into_iter()
        .map(|entry| {
            if !ids.insert(entry.id.clone()) {
                return Err("duplicate W3C fixture identifier".into());
            }
            export(entry)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&args[2])?;
    file.write_all(&serde_json::to_vec_pretty(&json!({
        "schema":"sparq.proof-bindings.imported-w3c.v1",
        "root_manifest":source(Path::new(&args[0]))?, "suite_root":root,
        "cases":records
    }))?)?;
    file.write_all(b"\n")?;
    Ok(())
}

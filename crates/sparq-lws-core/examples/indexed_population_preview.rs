//! Compare existing indexed persistence APIs on a bounded personal-data population.
//!
//! [GPT-6] This is an exploratory storage diagnostic, not an HTTP capacity benchmark.
//! Every open retains the engine's dictionary and compressed-block validation.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sparq_acbench::population::{self, PolicyModel, PopulationConfig, PopulationQuery};
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Settings {
    output: PathBuf,
    config: PopulationConfig,
    model: PolicyModel,
    pods: u64,
    max_pod_bytes: usize,
}

impl Settings {
    fn parse() -> Result<Self> {
        let mut flags = BTreeMap::new();
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            if flag == "--help" {
                println!("indexed_population_preview --output-dir NEW_DIRECTORY [--profile smoke|history|entropy] [--model wac|acp] [--pods 8] [--max-pod-bytes 536870912]");
                std::process::exit(0);
            }
            if !matches!(
                flag.as_str(),
                "--output-dir" | "--profile" | "--model" | "--pods" | "--max-pod-bytes"
            ) {
                return Err(format!("unknown argument: {flag}").into());
            }
            let value = args.next().ok_or("missing argument value")?;
            if flags.insert(flag, value).is_some() {
                return Err("duplicate argument".into());
            }
        }
        let config = match flags
            .get("--profile")
            .map(String::as_str)
            .unwrap_or("smoke")
        {
            "smoke" => PopulationConfig::smoke(),
            "history" => PopulationConfig::service_history(),
            "entropy" => PopulationConfig::service_history_entropy(),
            _ => return Err("profile must be smoke, history, or entropy".into()),
        };
        let model = match flags.get("--model").map(String::as_str).unwrap_or("wac") {
            "wac" => PolicyModel::Wac,
            "acp" => PolicyModel::Acp,
            _ => return Err("model must be wac or acp".into()),
        };
        let settings = Self {
            output: flags
                .remove("--output-dir")
                .ok_or("--output-dir is required")?
                .into(),
            config,
            model,
            pods: flags
                .get("--pods")
                .map(String::as_str)
                .unwrap_or("8")
                .parse()?,
            max_pod_bytes: flags
                .get("--max-pod-bytes")
                .map(String::as_str)
                .unwrap_or("536870912")
                .parse()?,
        };
        if !(1..=16).contains(&settings.pods)
            || !(1..=2_147_483_648).contains(&settings.max_pod_bytes)
        {
            return Err("pods must be1..16; max-pod-bytes must be1..2147483648".into());
        }
        Ok(settings)
    }
}

#[derive(Default, Debug, Serialize)]
struct DiskStats {
    files: u64,
    directories: u64,
    logical_bytes: u64,
    allocated_bytes: Option<u64>,
}

fn disk_stats(directory: &Path) -> io::Result<DiskStats> {
    let mut stats = DiskStats::default();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::other("unexpected symlink in generated index"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            *stats.allocated_bytes.get_or_insert(0) += metadata.blocks() * 512;
        }
        if metadata.is_dir() {
            stats.directories += 1;
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            stats.files += 1;
            stats.logical_bytes += metadata.len();
        } else {
            return Err(io::Error::other(
                "unexpected special file in generated index",
            ));
        }
    }
    Ok(stats)
}

fn materialize(store: &mut PodStore, model: PolicyModel) -> Result<u128> {
    let start = Instant::now();
    match model {
        PolicyModel::Wac => store.materialize_wac()?,
        PolicyModel::Acp => store.materialize_acp()?,
    };
    Ok(start.elapsed().as_nanos())
}

#[derive(Debug, PartialEq, Eq)]
struct Rows {
    variables: Vec<String>,
    bindings: Vec<String>,
}

fn rows(source: &str, ordered: bool) -> Result<Rows> {
    let value: Value = serde_json::from_str(source)?;
    let mut variables = value
        .pointer("/head/vars")
        .and_then(Value::as_array)
        .ok_or("missing result variables")?
        .iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or("invalid variable name"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    variables.sort();
    let mut bindings = value
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .ok_or("missing result bindings")?
        .iter()
        .map(|binding| {
            let fields: BTreeMap<_, _> = binding
                .as_object()
                .ok_or("binding is not an object")?
                .iter()
                .collect();
            Ok(serde_json::to_string(&fields)?)
        })
        .collect::<Result<Vec<_>>>()?;
    if !ordered {
        bindings.sort();
    }
    Ok(Rows {
        variables,
        bindings,
    })
}

struct Reference {
    role: &'static str,
    agent: Option<String>,
    query: PopulationQuery,
    result: Rows,
}

fn reference_queries(
    config: &PopulationConfig,
    pod: u64,
    store: &PodStore,
) -> Result<Vec<Reference>> {
    let mut references = Vec::new();
    for (role, agent) in [
        ("owner", Some(population::owner_webid(config, pod))),
        (
            "recipient",
            Some(population::recipient_webid(config, pod, 0)),
        ),
        ("anonymous", None),
    ] {
        let session = Session {
            agent: agent.as_deref(),
            client: None,
            issuer: None,
            now: None,
        };
        for query in population::benchmark_queries(config, pod)
            .into_iter()
            .filter(|q| {
                matches!(
                    q.id.as_str(),
                    "q1-point" | "q2-count" | "q3-star" | "q4-join" | "q7-optional" | "q11-graphs"
                )
            })
        {
            let start = Instant::now();
            let result = store.query_json_as(&session, Mode::Read, &query.sparql)?;
            let query_ns = start.elapsed().as_nanos();
            let normalized = rows(&result, query.sparql.contains("ORDER BY"))?;
            if query.id == "q2-count" {
                let value: Value = serde_json::from_str(&result)?;
                let count: u64 = value
                    .pointer("/results/bindings/0/count/value")
                    .and_then(Value::as_str)
                    .ok_or("count missing")?
                    .parse()?;
                if count != population::expected_record_count(config, pod, agent.as_deref())? {
                    return Err(
                        "memory reference disagrees with neutral record-count oracle".into(),
                    );
                }
            }
            emit(
                &json!({"record_type":"query", "pod":pod, "storage":"memory", "role":role,
                "query":query.id, "query_and_serialization_ns":query_ns, "rows":normalized.bindings.len(),
                "reference":true}),
            )?;
            references.push(Reference {
                role,
                agent: agent.clone(),
                query,
                result: normalized,
            });
        }
    }
    Ok(references)
}

fn compare_queries(
    store: &PodStore,
    references: &[Reference],
    pod: u64,
    storage: &str,
) -> Result<()> {
    for reference in references {
        let session = Session {
            agent: reference.agent.as_deref(),
            client: None,
            issuer: None,
            now: None,
        };
        let start = Instant::now();
        let result = store.query_json_as(&session, Mode::Read, &reference.query.sparql)?;
        let query_ns = start.elapsed().as_nanos();
        let normalized = rows(&result, reference.query.sparql.contains("ORDER BY"))?;
        if normalized != reference.result {
            return Err(format!(
                "{storage}: Pod{pod} {} {} differs from memory reference",
                reference.role, reference.query.id
            )
            .into());
        }
        emit(
            &json!({"record_type":"query", "pod":pod, "storage":storage, "role":reference.role,
            "query":reference.query.id, "query_and_serialization_ns":query_ns,
            "rows":normalized.bindings.len(), "exact_result_match":true}),
        )?;
    }
    Ok(())
}

fn graph_objects(graph: &Graph) -> u64 {
    1 + graph
        .named
        .iter()
        .map(|(_, sub)| graph_objects(sub))
        .sum::<u64>()
}

fn run_pod(settings: &Settings, pod: u64) -> Result<()> {
    let start = Instant::now();
    let mut source = LimitedBuffer {
        bytes: Vec::new(),
        limit: settings.max_pod_bytes,
    };
    let summary = population::write_pod(&settings.config, pod, settings.model, &mut source)?;
    let generate_ns = start.elapsed().as_nanos();
    let source_sha256 = format!("{:x}", Sha256::digest(&source.bytes));
    let source = String::from_utf8(source.bytes)?;
    let start = Instant::now();
    let graph = Graph::load_dataset(&source, "nquads")?;
    let prepare_ns = start.elapsed().as_nanos();
    drop(source);
    let objects = graph_objects(&graph);
    emit(
        &json!({"record_type":"prepare", "pod":pod, "summary":summary, "source_sha256":source_sha256,
        "generate_ns":generate_ns, "parse_and_index_ns":prepare_ns, "graph_objects":objects}),
    )?;
    for (storage, compressed) in [("raw", false), ("compressed", true)] {
        let path = settings.output.join(format!("pod-{pod}-{storage}"));
        let start = Instant::now();
        if compressed {
            graph.save_compressed(&path)?;
        } else {
            graph.save(&path)?;
        }
        let save_ns = start.elapsed().as_nanos();
        emit(
            &json!({"record_type":"save", "pod":pod, "storage":storage, "save_ns":save_ns,
            "files":disk_stats(&path)?, "path":path,
            "durability":"existing Graph::save API completion; no extra fsync or crash-durability claim"}),
        )?;
    }
    let ready_start = Instant::now();
    let mut store = PodStore::new(graph);
    let constructor_ns = ready_start.elapsed().as_nanos();
    let materialize_ns = materialize(&mut store, settings.model)?;
    let constructor_to_authorized_ready_ns = ready_start.elapsed().as_nanos();
    let reference_graph_objects = graph_objects(&store.graph);
    emit(
        &json!({"record_type":"materialize", "pod":pod, "storage":"memory",
            "constructor_ns":constructor_ns, "materialize_ns":materialize_ns,
            "constructor_to_authorized_ready_ns":constructor_to_authorized_ready_ns}),
    )?;
    let references = reference_queries(&settings.config, pod, &store)?;
    drop(store);
    for storage in ["raw", "compressed"] {
        let path = settings.output.join(format!("pod-{pod}-{storage}"));
        let start = Instant::now();
        let graph = Graph::open(&path)?;
        let open_ns = start.elapsed().as_nanos();
        let constructor_start = Instant::now();
        let mut store = PodStore::new(graph);
        let constructor_ns = constructor_start.elapsed().as_nanos();
        let materialize_ns = materialize(&mut store, settings.model)?;
        let open_to_authorized_ready_ns = start.elapsed().as_nanos();
        if graph_objects(&store.graph) != reference_graph_objects {
            return Err("persisted graph structure changed".into());
        }
        emit(
            &json!({"record_type":"open", "pod":pod, "storage":storage, "open_and_validation_ns":open_ns,
            "constructor_ns":constructor_ns, "materialize_ns":materialize_ns,
            "open_to_authorized_ready_ns":open_to_authorized_ready_ns,
            "files":disk_stats(&path)?, "graph_objects":objects,
            "retained_wal_and_journal_fd_estimate":2 * objects,
            "validation":"unchanged Graph::open: dictionary records and compressed blocks validated",
            "os_page_cache":"uncontrolled; files were just generated in this process"}),
        )?;
        compare_queries(&store, &references, pod, storage)?;
    }
    emit(
        &json!({"record_type":"pod-complete", "pod":pod, "exact_result_comparisons":references.len() * 2}),
    )?;
    Ok(())
}

fn emit(value: &Value) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::other(
                "Pod source exceeds max-pod-bytes; diagnostic rejected this Pod",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn main() -> Result<()> {
    let settings = Settings::parse()?;
    fs::create_dir(&settings.output)?;
    emit(
        &json!({"record_type":"configuration", "schema_version":1, "config":settings.config,
        "model":settings.model, "pods":settings.pods, "max_pod_bytes":settings.max_pod_bytes,
        "scope":"bounded sequential prefix diagnostic; not representative population or HTTP capacity evidence",
        "order":"memory then raw then compressed; one prepared Pod at a time; no cache flushing",
        "results":"exact ordered rows or bags against the same engine over in-memory data; independent count oracle"}),
    )?;
    for pod in 0..settings.pods {
        run_pod(&settings, pod)?;
    }
    emit(
        &json!({"record_type":"diagnostic-complete", "pods":settings.pods, "exact_result_comparisons":settings.pods * 36}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_preserves_duplicates_and_requested_order() -> Result<()> {
        let result = |values: &[&str]| {
            json!({"head":{"vars":["v"]},"results":{"bindings":values.iter().map(|v| json!({"v":{"type":"literal","value":v}})).collect::<Vec<_>>()}}).to_string()
        };
        assert_eq!(
            rows(&result(&["a", "b"]), false)?,
            rows(&result(&["b", "a"]), false)?
        );
        assert_ne!(
            rows(&result(&["a", "b"]), true)?,
            rows(&result(&["b", "a"]), true)?
        );
        assert_ne!(
            rows(&result(&["a", "a"]), false)?,
            rows(&result(&["a"]), false)?
        );
        Ok(())
    }

    #[test]
    fn buffer_rejects_oversize_before_accepting_bytes() {
        let mut buffer = LimitedBuffer {
            bytes: Vec::new(),
            limit: 3,
        };
        buffer.write_all(b"abc").unwrap();
        assert!(buffer.write_all(b"d").is_err());
        assert_eq!(buffer.bytes, b"abc");
    }
}

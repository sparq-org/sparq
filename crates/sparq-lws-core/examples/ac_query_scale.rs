//! Raw-observation runner for the many-Pod access-controlled SPARQL paper.
//!
//! Two deliberately separate lanes share one corpus and correctness oracle:
//!
//! - `materialized`: one `sparq_solid::PodStore` per Pod, routed before `query_as`;
//! - `http`: the real `sparq_lws_core` axum `/sparql` route, including authentication,
//!   server-root containment enumeration, WAC planning, dataset assembly, evaluation,
//!   and SPARQL Results JSON serialization.
//!
//! Every timed result is compared with evaluation over an independently and physically
//! filtered reference dataset before its observation is emitted. Output is JSON Lines on
//! stdout; progress and errors go to stderr.

#![warn(clippy::undocumented_unsafe_blocks)]

#[path = "support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::env;
use std::io::{BufWriter, Write};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{to_bytes, Body, Bytes};
use axum::http::Request;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sparq_acbench::deployment::{
    generate, BenchmarkQuery, DeploymentAudienceMix, DeploymentCorpus, DeploymentDomain,
    DeploymentParams, OriginTopology, PrincipalClass,
};
use sparq_engine::QueryResult;
use sparq_lws_core::store::counting::BackendCounters;
#[cfg(feature = "ac-query-scale-instrumentation")]
use sparq_lws_core::store::counting::{CountingBlobStore, CountingSparqClient};
use sparq_lws_core::store::{
    CompositeStore, InMemoryBlobStore, InMemorySparqClient, InMemoryStoreLimits, Store,
};
use sparq_solid::{Mode, PodStore, Session};
use tower::ServiceExt;

#[cfg(feature = "ac-query-scale-instrumentation")]
#[global_allocator]
static GLOBAL: support::CountingAllocator = support::CountingAllocator;

#[cfg(not(feature = "ac-query-scale-instrumentation"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SCHEMA_VERSION: u8 = 6;
const STRANGER_WEBID: &str = "https://identities.example/stranger#me";

const fn measurement_profile() -> &'static str {
    if cfg!(feature = "ac-query-scale-instrumentation") {
        "instrumentation"
    } else {
        "timing"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Materialized,
    Http,
}

impl Lane {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "materialized" => Ok(Self::Materialized),
            "http" => Ok(Self::Http),
            _ => Err(format!(
                "unknown lane {value:?}; expected materialized or http"
            )),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Materialized => "materialized-routed",
            Self::Http => "native-http-assembly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrincipalKind {
    Owner,
    Recipient,
    Stranger,
    Anonymous,
}

impl PrincipalKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "owner" => Ok(Self::Owner),
            "recipient" => Ok(Self::Recipient),
            "stranger" => Ok(Self::Stranger),
            "anonymous" => Ok(Self::Anonymous),
            _ => Err(format!(
                "unknown principal {value:?}; expected owner, recipient, stranger, or anonymous"
            )),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Recipient => "named-recipient",
            Self::Stranger => "authenticated-stranger",
            Self::Anonymous => "anonymous",
        }
    }

    const fn oracle(self) -> PrincipalClass {
        match self {
            Self::Owner => PrincipalClass::Owner(0),
            Self::Recipient => PrincipalClass::Recipient(0),
            Self::Stranger => PrincipalClass::Stranger,
            Self::Anonymous => PrincipalClass::Anonymous,
        }
    }
}

#[derive(Debug, Clone)]
struct Config {
    lane: Lane,
    campaign: String,
    cell_label: String,
    params: DeploymentParams,
    principal: PrincipalKind,
    warmups: u32,
    repetitions: u32,
    process_block: u32,
    configuration_order: Option<u32>,
    configuration_order_seed: Option<u64>,
    run_id: String,
    query_filters: Vec<String>,
}

impl Config {
    fn parse() -> Result<Option<Self>, String> {
        let args: Vec<String> = env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            print_help();
            return Ok(None);
        }

        let mut config = Self {
            lane: Lane::Materialized,
            campaign: "ad-hoc".to_owned(),
            cell_label: "ad-hoc".to_owned(),
            params: DeploymentParams {
                seed: 42,
                pods: 1,
                documents_per_pod: 16,
                triples_per_document: 8,
                container_depth: 3,
                own_acl_coverage: 250,
                audience_mix: DeploymentAudienceMix::controlled(),
                topology: OriginTopology::SharedOrigin,
                domain: DeploymentDomain::Social,
            },
            principal: PrincipalKind::Owner,
            warmups: 10,
            repetitions: 30,
            process_block: 0,
            configuration_order: None,
            configuration_order_seed: None,
            run_id: default_run_id(),
            query_filters: Vec::new(),
        };

        let mut index = 0;
        while index < args.len() {
            let flag = &args[index];
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("missing value after {flag}"))?;
            index += 1;
            match flag.as_str() {
                "--lane" => config.lane = Lane::parse(value)?,
                "--campaign" => config.campaign.clone_from(value),
                "--cell-label" => config.cell_label.clone_from(value),
                "--pods" => config.params.pods = parse_value(flag, value)?,
                "--documents" => {
                    config.params.documents_per_pod = parse_value(flag, value)?;
                }
                "--triples" => {
                    config.params.triples_per_document = parse_value(flag, value)?;
                }
                "--depth" => config.params.container_depth = parse_value(flag, value)?,
                "--acl-coverage" => {
                    config.params.own_acl_coverage = parse_value(flag, value)?;
                }
                "--public" => config.params.audience_mix.public = parse_value(flag, value)?,
                "--private" => config.params.audience_mix.private = parse_value(flag, value)?,
                "--shared" => config.params.audience_mix.shared = parse_value(flag, value)?,
                "--seed" => config.params.seed = parse_value(flag, value)?,
                "--domain" => {
                    config.params.domain = match value.as_str() {
                        "social" => DeploymentDomain::Social,
                        "health" => DeploymentDomain::Health,
                        _ => return Err(format!("unknown domain {value:?}")),
                    };
                }
                "--topology" => {
                    config.params.topology = match value.as_str() {
                        "shared-origin" => OriginTopology::SharedOrigin,
                        "origin-per-pod" => OriginTopology::OriginPerPod,
                        _ => return Err(format!("unknown topology {value:?}")),
                    };
                }
                "--principal" => config.principal = PrincipalKind::parse(value)?,
                "--warmups" => config.warmups = parse_value(flag, value)?,
                "--repetitions" => config.repetitions = parse_value(flag, value)?,
                "--process-block" => config.process_block = parse_value(flag, value)?,
                "--configuration-order" => {
                    config.configuration_order = Some(parse_value(flag, value)?);
                }
                "--configuration-order-seed" => {
                    config.configuration_order_seed = Some(parse_value(flag, value)?);
                }
                "--run-id" => config.run_id.clone_from(value),
                "--query" => {
                    for query_id in value.split(',').map(str::trim) {
                        if query_id.is_empty() {
                            return Err("--query contains an empty query identifier".to_owned());
                        }
                        if !config.query_filters.iter().any(|known| known == query_id) {
                            config.query_filters.push(query_id.to_owned());
                        }
                    }
                }
                _ => return Err(format!("unknown argument {flag:?}; use --help")),
            }
        }
        config.params.validate()?;
        if config.repetitions == 0 {
            return Err("repetitions must be greater than zero".to_owned());
        }
        if config.campaign.is_empty() || config.cell_label.is_empty() {
            return Err("campaign and cell-label must be non-empty".to_owned());
        }
        if config.lane == Lane::Http && config.params.topology != OriginTopology::SharedOrigin {
            return Err(
                "the native HTTP lane has one server authority and requires shared-origin topology"
                    .to_owned(),
            );
        }
        Ok(Some(config))
    }
}

fn parse_value<T>(flag: &str, value: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| format!("invalid {flag} value {value:?}: {error}"))
}

fn print_help() {
    println!(
        "ac_query_scale options:\n\
         --lane materialized|http [--campaign NAME --cell-label NAME]\n\
         --pods N --documents N --triples N --depth N\n\
         --acl-coverage PER_MILLE --public PER_MILLE --private PER_MILLE --shared PER_MILLE\n\
         --domain social|health --topology shared-origin|origin-per-pod --seed N\n\
         --principal owner|recipient|stranger|anonymous\n\
         --warmups N --repetitions N --process-block N --run-id ID\n\
         [--query q1-point[,q8-graph-scan]]\n\
         [--configuration-order N --configuration-order-seed N]"
    );
}

#[derive(Debug, Clone, Serialize)]
struct CommonRecord {
    schema_version: u8,
    run_id: String,
    run_uuid: String,
    source_commit: String,
    source_dirty: bool,
    host: String,
    instance_id: Option<String>,
    instance_type: Option<String>,
    cloud_region: Option<String>,
    os: String,
    architecture: String,
    rustc: String,
    profile: &'static str,
    features: &'static str,
    measurement_profile: &'static str,
    campaign: String,
    cell_label: String,
    lane: &'static str,
    domain: &'static str,
    topology: &'static str,
    pods: u32,
    documents_per_pod: u32,
    triples_per_document: u32,
    container_depth: u8,
    own_acl_coverage_per_mille: u16,
    public_per_mille: u16,
    private_per_mille: u16,
    shared_per_mille: u16,
    principal: &'static str,
    corpus_seed: u64,
    corpus_hash_sha256: String,
    process_block: u32,
    configuration_order: Option<u32>,
    configuration_order_seed: Option<u64>,
    warmups_configured: u32,
    repetitions_configured: u32,
    concurrency: u32,
    rayon_threads: Option<String>,
    cpu_affinity: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CorpusCounts {
    content_documents: usize,
    content_triples: u64,
    container_graphs: usize,
    control_documents: usize,
    control_triples: u64,
    total_source_graphs: usize,
    target_readable_documents: usize,
    evaluation_readable_documents: usize,
}

#[derive(Serialize)]
struct ConstructionRecord<'a> {
    #[serde(flatten)]
    common: &'a CommonRecord,
    record_type: &'static str,
    utc_unix_ns: u128,
    #[serde(flatten)]
    counts: &'a CorpusCounts,
    corpus_generation_ns: u64,
    graph_load_ns: Option<u64>,
    wac_materialization_ns: Option<u64>,
    route_index_ns: Option<u64>,
    lws_seed_ns: Option<u64>,
    auth_triples_total: Option<u64>,
    http_store_max_total_bytes: Option<u64>,
    http_store_max_resource_count: Option<u64>,
    construction_allocations: Option<u64>,
    construction_allocated_bytes: Option<u64>,
    resident_bytes: Option<u64>,
    peak_resident_bytes: Option<u64>,
}

#[derive(Serialize)]
struct ApplicabilityRecord<'a> {
    #[serde(flatten)]
    common: &'a CommonRecord,
    record_type: &'static str,
    utc_unix_ns: u128,
    query_id: &'a str,
    query_family: &'a str,
    query_hash_sha256: String,
    minimum_triples_per_document: u32,
    applicable: bool,
    selected: bool,
    reason: Option<&'static str>,
}

#[derive(Serialize)]
struct CorrectnessRecord<'a> {
    #[serde(flatten)]
    common: &'a CommonRecord,
    record_type: &'static str,
    utc_unix_ns: u128,
    gate: &'static str,
    principals_checked: u32,
    queries_checked: u32,
    exact_result_bags: bool,
}

#[derive(Serialize)]
struct ObservationRecord<'a> {
    #[serde(flatten)]
    common: &'a CommonRecord,
    record_type: &'static str,
    utc_unix_ns: u128,
    query_id: &'a str,
    query_family: &'a str,
    query_hash_sha256: String,
    operation: &'static str,
    pair_id: String,
    repetition: u32,
    warmup: bool,
    order_in_pair: u8,
    wall_ns: u64,
    process_cpu_ns: Option<u64>,
    allocation_operations: Option<u64>,
    allocated_bytes: Option<u64>,
    response_bytes: Option<usize>,
    result_rows: usize,
    result_hash_sha256: String,
    correctness: bool,
    http_status: Option<u16>,
    backend_sparql_queries: Option<u64>,
    backend_sparql_updates: Option<u64>,
    backend_blob_gets: Option<u64>,
    backend_blob_puts: Option<u64>,
    backend_blob_other: Option<u64>,
    backend_total_operations: Option<u64>,
    backend_max_in_flight: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalResult {
    variables: Vec<String>,
    rows: Vec<String>,
    hash: String,
}

impl CanonicalResult {
    fn from_query(result: &QueryResult) -> Self {
        let variables: Vec<_> = result.vars.iter().map(ToString::to_string).collect();
        let mut rows: Vec<_> = result
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        Some(term) => length_prefixed(&term.to_string()),
                        None => "-".to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect();
        rows.sort_unstable();
        let hash = result_hash(&variables, &rows);
        Self {
            variables,
            rows,
            hash,
        }
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn from_sparql_json(bytes: &[u8]) -> Result<Self, String> {
        let document: Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid SPARQL Results JSON: {error}"))?;
        let variables: Vec<String> = document
            .pointer("/head/vars")
            .and_then(Value::as_array)
            .ok_or_else(|| "SPARQL Results JSON has no head.vars array".to_owned())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "SPARQL Results JSON variable is not a string".to_owned())
            })
            .collect::<Result<_, _>>()?;
        let bindings = document
            .pointer("/results/bindings")
            .and_then(Value::as_array)
            .ok_or_else(|| "SPARQL Results JSON has no results.bindings array".to_owned())?;
        let mut rows = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let object = binding
                .as_object()
                .ok_or_else(|| "SPARQL Results JSON binding is not an object".to_owned())?;
            let mut row = String::new();
            for variable in &variables {
                let Some(cell) = object.get(variable) else {
                    row.push('-');
                    continue;
                };
                let cell = cell
                    .as_object()
                    .ok_or_else(|| "SPARQL Results JSON cell is not an object".to_owned())?;
                for key in ["type", "value", "datatype", "xml:lang"] {
                    let value = cell.get(key).and_then(Value::as_str).unwrap_or("");
                    row.push_str(&length_prefixed(value));
                }
            }
            rows.push(row);
        }
        rows.sort_unstable();
        let hash = result_hash(&variables, &rows);
        Ok(Self {
            variables,
            rows,
            hash,
        })
    }
}

struct Timed<T> {
    value: T,
    wall_ns: u64,
    process_cpu_ns: Option<u64>,
    allocation_operations: Option<u64>,
    allocated_bytes: Option<u64>,
}

struct ProcessCpuClock;

impl ProcessCpuClock {
    fn discover() -> Self {
        Self
    }

    fn now_ns(&self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            let mut timestamp = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: `timestamp` is valid writable storage for one `timespec`, and
            // `CLOCK_PROCESS_CPUTIME_ID` requires no additional pointer or lifetime
            // invariants. A non-zero return is handled as an unavailable measurement.
            if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut timestamp) } != 0 {
                return None;
            }
            let seconds = u64::try_from(timestamp.tv_sec).ok()?;
            let nanoseconds = u64::try_from(timestamp.tv_nsec).ok()?;
            seconds
                .checked_mul(1_000_000_000)
                .and_then(|value| value.checked_add(nanoseconds))
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    fn elapsed_ns(&self, start: Option<u64>, end: Option<u64>) -> Option<u64> {
        end?.checked_sub(start?)
    }
}

fn timed<T>(clock: &ProcessCpuClock, operation: impl FnOnce() -> T) -> Timed<T> {
    let (alloc_start, bytes_start) = support::alloc_snapshot();
    let cpu_start = clock.now_ns();
    let start = Instant::now();
    let value = operation();
    let wall_ns = duration_ns(start.elapsed());
    let cpu_end = clock.now_ns();
    let (alloc_end, bytes_end) = support::alloc_snapshot();
    Timed {
        value,
        wall_ns,
        process_cpu_ns: clock.elapsed_ns(cpu_start, cpu_end),
        allocation_operations: cfg!(feature = "ac-query-scale-instrumentation")
            .then(|| alloc_end.saturating_sub(alloc_start)),
        allocated_bytes: cfg!(feature = "ac-query-scale-instrumentation")
            .then(|| bytes_end.saturating_sub(bytes_start)),
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn utc_unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn default_run_id() -> String {
    format!("local-{}-{}", std::process::id(), utc_unix_ns())
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn result_hash(variables: &[String], rows: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"sparq-ac-result-v1\0");
    for variable in variables {
        hash_field(&mut hasher, variable.as_bytes());
    }
    hasher.update([0xff]);
    for row in rows {
        hash_field(&mut hasher, row.as_bytes());
    }
    hex_digest(hasher.finalize().as_slice())
}

fn query_hash(query: &str) -> String {
    hex_digest(Sha256::digest(query.as_bytes()).as_slice())
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn corpus_hash(corpus: &DeploymentCorpus) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"sparq-ac-deployment-corpus-v1\0");
    for pod in &corpus.pods {
        hash_field(&mut hasher, pod.root_iri.as_bytes());
        hash_field(&mut hasher, pod.owner_webid.as_bytes());
        hash_field(&mut hasher, pod.recipient_webid.as_bytes());
    }
    for container in &corpus.containers {
        hash_field(&mut hasher, container.iri.as_bytes());
        hash_field(
            &mut hasher,
            container.parent_iri.as_deref().unwrap_or("").as_bytes(),
        );
    }
    for document in &corpus.documents {
        hash_field(&mut hasher, document.iri.as_bytes());
        hash_field(&mut hasher, format!("{:?}", document.audience).as_bytes());
        hasher.update([u8::from(document.has_own_acl)]);
        hash_field(&mut hasher, document.ntriples.as_bytes());
    }
    for control in &corpus.control_documents {
        hash_field(&mut hasher, control.iri.as_bytes());
        hash_field(&mut hasher, control.governed_iri.as_bytes());
        hash_field(&mut hasher, control.ntriples.as_bytes());
    }
    hex_digest(hasher.finalize().as_slice())
}

fn corpus_counts(corpus: &DeploymentCorpus, principal: PrincipalKind, lane: Lane) -> CorpusCounts {
    let content_triples = u64::try_from(corpus.documents.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(corpus.params.triples_per_document));
    let control_triples = corpus
        .control_documents
        .iter()
        .map(|document| document.ntriples.lines().count() as u64)
        .sum();
    let target_readable_documents = corpus
        .documents
        .iter()
        .filter(|document| document.pod_index == 0 && corpus.can_read(principal.oracle(), document))
        .count();
    let evaluation_readable_documents = match lane {
        Lane::Materialized => target_readable_documents,
        Lane::Http => corpus
            .documents
            .iter()
            .filter(|document| corpus.can_read(principal.oracle(), document))
            .count(),
    };
    CorpusCounts {
        content_documents: corpus.documents.len(),
        content_triples,
        container_graphs: corpus.containers.len(),
        control_documents: corpus.control_documents.len(),
        control_triples,
        total_source_graphs: corpus.documents.len()
            + corpus.containers.len()
            + corpus.control_documents.len(),
        target_readable_documents,
        evaluation_readable_documents,
    }
}

fn common_record(config: &Config, corpus_hash_sha256: String) -> CommonRecord {
    CommonRecord {
        schema_version: SCHEMA_VERSION,
        run_id: config.run_id.clone(),
        run_uuid: uuid::Uuid::new_v4().to_string(),
        source_commit: command_output("git", &["rev-parse", "HEAD"])
            .unwrap_or_else(|| "unknown".to_owned()),
        source_dirty: command_output("git", &["status", "--porcelain"])
            .is_some_and(|output| !output.is_empty()),
        host: env::var("SPARQ_BENCH_HOST")
            .ok()
            .or_else(|| command_output("hostname", &[]))
            .unwrap_or_else(|| "unknown".to_owned()),
        instance_id: env::var("SPARQ_BENCH_INSTANCE_ID").ok(),
        instance_type: env::var("SPARQ_BENCH_INSTANCE_TYPE").ok(),
        cloud_region: env::var("SPARQ_BENCH_REGION").ok(),
        os: env::consts::OS.to_owned(),
        architecture: env::consts::ARCH.to_owned(),
        rustc: command_output("rustc", &["--version"]).unwrap_or_else(|| "unknown".to_owned()),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        features: if cfg!(feature = "ac-query-scale-instrumentation") {
            "sparq-lws-core/default+ac-query-scale-instrumentation; sparq-solid/default"
        } else {
            "sparq-lws-core/default; sparq-solid/default"
        },
        measurement_profile: measurement_profile(),
        campaign: config.campaign.clone(),
        cell_label: config.cell_label.clone(),
        lane: config.lane.label(),
        domain: match config.params.domain {
            DeploymentDomain::Social => "social",
            DeploymentDomain::Health => "health",
        },
        topology: match config.params.topology {
            OriginTopology::SharedOrigin => "shared-origin",
            OriginTopology::OriginPerPod => "origin-per-pod",
        },
        pods: config.params.pods,
        documents_per_pod: config.params.documents_per_pod,
        triples_per_document: config.params.triples_per_document,
        container_depth: config.params.container_depth,
        own_acl_coverage_per_mille: config.params.own_acl_coverage,
        public_per_mille: config.params.audience_mix.public,
        private_per_mille: config.params.audience_mix.private,
        shared_per_mille: config.params.audience_mix.shared,
        principal: config.principal.label(),
        corpus_seed: config.params.seed,
        corpus_hash_sha256,
        process_block: config.process_block,
        configuration_order: config.configuration_order,
        configuration_order_seed: config.configuration_order_seed,
        warmups_configured: config.warmups,
        repetitions_configured: config.repetitions,
        concurrency: 1,
        rayon_threads: env::var("RAYON_NUM_THREADS").ok(),
        cpu_affinity: current_cpu_affinity(),
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn current_cpu_affinity() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn resident_memory() -> (Option<u64>, Option<u64>) {
    if cfg!(target_os = "linux") {
        let status = std::fs::read_to_string("/proc/self/status").ok();
        let current = status
            .as_deref()
            .and_then(|value| proc_status_bytes(value, "VmRSS:"));
        let peak = status
            .as_deref()
            .and_then(|value| proc_status_bytes(value, "VmHWM:"));
        (current, peak)
    } else {
        let pid = std::process::id().to_string();
        let current = command_output("ps", &["-o", "rss=", "-p", &pid])
            .and_then(|value| value.trim().parse::<u64>().ok())
            .and_then(|kibibytes| kibibytes.checked_mul(1024));
        (current, None)
    }
}

fn proc_status_bytes(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.split_whitespace().next()?;
        value.parse::<u64>().ok()?.checked_mul(1024)
    })
}

fn random_guarded_first(config: &Config, query_index: usize, repetition: u32) -> bool {
    let mut value = config.params.seed
        ^ u64::from(config.process_block).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (query_index as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ u64::from(repetition).wrapping_mul(0x94d0_49bb_1331_11eb);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (value ^ (value >> 31)) & 1 == 0
}

fn selected_queries<'a>(
    config: &Config,
    queries: &'a [BenchmarkQuery],
) -> Result<Vec<&'a BenchmarkQuery>, String> {
    let unknown: Vec<_> = config
        .query_filters
        .iter()
        .filter(|filter| !queries.iter().any(|query| query.id == filter.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(format!("unknown query identifiers: {}", unknown.join(", ")));
    }
    let selected: Vec<_> = queries
        .iter()
        .filter(|query| query_selected(config, query.id))
        .collect();
    if selected.is_empty() {
        return Err(format!(
            "query filter {:?} selected no workload query",
            config.query_filters
        ));
    }
    Ok(selected)
}

fn query_selected(config: &Config, query_id: &str) -> bool {
    config.query_filters.is_empty() || config.query_filters.iter().any(|filter| filter == query_id)
}

fn emit<T: Serialize>(output: &mut impl Write, record: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *output, record).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())
}

fn emit_applicability(
    output: &mut impl Write,
    common: &CommonRecord,
    config: &Config,
    queries: &[BenchmarkQuery],
) -> Result<(), String> {
    for query in queries {
        let applicable = config.params.triples_per_document >= query.minimum_triples_per_document;
        let selected = query_selected(config, query.id);
        emit(
            output,
            &ApplicabilityRecord {
                common,
                record_type: "applicability",
                utc_unix_ns: utc_unix_ns(),
                query_id: query.id,
                query_family: query.family,
                query_hash_sha256: query_hash(&query.sparql),
                minimum_triples_per_document: query.minimum_triples_per_document,
                applicable,
                selected,
                reason: (!applicable).then_some("required generated predicates are absent"),
            },
        )?;
    }
    Ok(())
}

fn session_for<'a>(
    principal: PrincipalKind,
    owner_webid: &'a str,
    recipient_webid: &'a str,
) -> Session<'a> {
    let agent = match principal {
        PrincipalKind::Owner => Some(owner_webid),
        PrincipalKind::Recipient => Some(recipient_webid),
        PrincipalKind::Stranger => Some(STRANGER_WEBID),
        PrincipalKind::Anonymous => None,
    };
    Session {
        agent,
        client: None,
        issuer: None,
        now: None,
    }
}

fn verify_materialized_all_principals(
    store: &PodStore,
    corpus: &DeploymentCorpus,
    queries: &[&BenchmarkQuery],
) -> Result<u32, String> {
    let pod = &corpus.pods[0];
    let principals = [
        PrincipalKind::Owner,
        PrincipalKind::Recipient,
        PrincipalKind::Stranger,
        PrincipalKind::Anonymous,
    ];
    let mut checks = 0_u32;
    for principal in principals {
        let reference = sparq_core::Graph::load_dataset(
            &corpus.pod_readable_content_nquads(0, principal.oracle())?,
            "nquads",
        )?;
        let session = session_for(principal, &pod.owner_webid, &pod.recipient_webid);
        for query in queries {
            let actual = CanonicalResult::from_query(&store.query_as(
                &session,
                Mode::Read,
                &query.sparql,
            )?);
            let expected =
                CanonicalResult::from_query(&sparq_engine::query(&reference, &query.sparql)?);
            if actual != expected {
                return Err(format!(
                    "materialized correctness mismatch: principal={}, query={}",
                    principal.label(),
                    query.id
                ));
            }
            checks += 1;
        }
    }
    Ok(checks)
}

fn run_materialized(
    output: &mut impl Write,
    config: &Config,
    corpus: DeploymentCorpus,
    corpus_generation_ns: u64,
) -> Result<(), String> {
    let hash = corpus_hash(&corpus);
    let counts = corpus_counts(&corpus, config.principal, config.lane);
    let common = common_record(config, hash);
    let workload = corpus.benchmark_queries(0)?;
    emit_applicability(output, &common, config, &workload)?;
    let selected = selected_queries(config, &workload)?;
    let applicable: Vec<_> = selected
        .into_iter()
        .filter(|query| config.params.triples_per_document >= query.minimum_triples_per_document)
        .collect();
    if applicable.is_empty() {
        return Err(
            "no selected query is applicable to this triples-per-document setting".to_owned(),
        );
    }

    let mut graph_load_ns = 0_u64;
    let mut materialization_ns = 0_u64;
    let mut auth_triples_total = 0_u64;
    let (vector_alloc_start, vector_bytes_start) = support::alloc_snapshot();
    let mut stores = Vec::with_capacity(config.params.pods as usize);
    let (vector_alloc_end, vector_bytes_end) = support::alloc_snapshot();
    let mut construction_allocations = vector_alloc_end.saturating_sub(vector_alloc_start);
    let mut construction_allocated_bytes = vector_bytes_end.saturating_sub(vector_bytes_start);
    for pod_index in 0..config.params.pods {
        // Rendering the already-generated fixture into an interchange string is harness
        // work. Count only allocations made while the service parses, materializes, and
        // retains the Pod store.
        let nquads = corpus.pod_dataset_nquads(pod_index)?;
        let (pod_alloc_start, pod_bytes_start) = support::alloc_snapshot();
        let start = Instant::now();
        let graph = sparq_core::Graph::load_dataset(&nquads, "nquads")?;
        graph_load_ns = graph_load_ns.saturating_add(duration_ns(start.elapsed()));
        let mut store = PodStore::new(graph);
        let start = Instant::now();
        let stats = store.materialize_wac()?;
        materialization_ns = materialization_ns.saturating_add(duration_ns(start.elapsed()));
        auth_triples_total = auth_triples_total.saturating_add(stats.auth_triples as u64);
        stores.push(store);
        let (pod_alloc_end, pod_bytes_end) = support::alloc_snapshot();
        construction_allocations =
            construction_allocations.saturating_add(pod_alloc_end.saturating_sub(pod_alloc_start));
        construction_allocated_bytes = construction_allocated_bytes
            .saturating_add(pod_bytes_end.saturating_sub(pod_bytes_start));
    }
    let (route_alloc_start, route_bytes_start) = support::alloc_snapshot();
    let route_start = Instant::now();
    let routes: HashMap<_, _> = corpus
        .pods
        .iter()
        .enumerate()
        .map(|(index, pod)| (pod.root_iri.clone(), index))
        .collect();
    let route_index_ns = duration_ns(route_start.elapsed());
    let (route_alloc_end, route_bytes_end) = support::alloc_snapshot();
    construction_allocations =
        construction_allocations.saturating_add(route_alloc_end.saturating_sub(route_alloc_start));
    construction_allocated_bytes = construction_allocated_bytes
        .saturating_add(route_bytes_end.saturating_sub(route_bytes_start));
    // All service-construction allocation scopes have now closed. The correctness
    // oracle remains live during paired measurements but is benchmark instrumentation,
    // not part of the routed service.
    let target_root = corpus.pods[0].root_iri.clone();
    let owner_webid = corpus.pods[0].owner_webid.clone();
    let recipient_webid = corpus.pods[0].recipient_webid.clone();

    let checks = verify_materialized_all_principals(&stores[0], &corpus, &applicable)?;
    emit(
        output,
        &CorrectnessRecord {
            common: &common,
            record_type: "correctness-gate",
            utc_unix_ns: utc_unix_ns(),
            gate: "all four principal classes against physically filtered reference",
            principals_checked: 4,
            queries_checked: checks,
            exact_result_bags: true,
        },
    )?;

    let reference_nquads = corpus.pod_readable_content_nquads(0, config.principal.oracle())?;
    let reference = sparq_core::Graph::load_dataset(&reference_nquads, "nquads")?;
    drop(reference_nquads);
    drop(corpus);
    let (resident_bytes, peak_resident_bytes) = resident_memory();
    emit(
        output,
        &ConstructionRecord {
            common: &common,
            record_type: "construction",
            utc_unix_ns: utc_unix_ns(),
            counts: &counts,
            corpus_generation_ns,
            graph_load_ns: Some(graph_load_ns),
            wac_materialization_ns: Some(materialization_ns),
            route_index_ns: Some(route_index_ns),
            lws_seed_ns: None,
            auth_triples_total: Some(auth_triples_total),
            http_store_max_total_bytes: None,
            http_store_max_resource_count: None,
            construction_allocations: cfg!(feature = "ac-query-scale-instrumentation")
                .then_some(construction_allocations),
            construction_allocated_bytes: cfg!(feature = "ac-query-scale-instrumentation")
                .then_some(construction_allocated_bytes),
            resident_bytes,
            peak_resident_bytes,
        },
    )?;

    let session = session_for(config.principal, &owner_webid, &recipient_webid);
    let clock = ProcessCpuClock::discover();
    for (query_index, query) in applicable.iter().enumerate() {
        let expected =
            CanonicalResult::from_query(&sparq_engine::query(&reference, &query.sparql)?);
        for _ in 0..config.warmups {
            let store_index = *routes
                .get(&target_root)
                .ok_or_else(|| "target Pod route is absent".to_owned())?;
            let actual = CanonicalResult::from_query(&stores[store_index].query_as(
                &session,
                Mode::Read,
                &query.sparql,
            )?);
            if actual != expected {
                return Err(format!("warm-up mismatch for {}", query.id));
            }
            let plain =
                CanonicalResult::from_query(&sparq_engine::query(&reference, &query.sparql)?);
            if plain != expected {
                return Err(format!("plain warm-up mismatch for {}", query.id));
            }
        }

        for repetition in 0..config.repetitions {
            let guarded_first = random_guarded_first(config, query_index, repetition);
            for order_in_pair in 0..2_u8 {
                let guarded = (order_in_pair == 0) == guarded_first;
                let measured = if guarded {
                    timed(&clock, || {
                        let store_index = *routes
                            .get(&target_root)
                            .expect("preflight established target Pod route");
                        stores[store_index].query_as(&session, Mode::Read, &query.sparql)
                    })
                } else {
                    timed(&clock, || sparq_engine::query(&reference, &query.sparql))
                };
                let result = measured.value?;
                let actual = CanonicalResult::from_query(&result);
                if actual != expected {
                    return Err(format!(
                        "timed {} result mismatch for {} at repetition {repetition}",
                        if guarded { "guarded" } else { "plain" },
                        query.id
                    ));
                }
                emit(
                    output,
                    &ObservationRecord {
                        common: &common,
                        record_type: "observation",
                        utc_unix_ns: utc_unix_ns(),
                        query_id: query.id,
                        query_family: query.family,
                        query_hash_sha256: query_hash(&query.sparql),
                        operation: if guarded {
                            "guarded-query-as"
                        } else {
                            "plain-engine-reference"
                        },
                        pair_id: format!(
                            "{}:{}:{}:{}",
                            config.run_id, config.process_block, query.id, repetition
                        ),
                        repetition,
                        warmup: false,
                        order_in_pair,
                        wall_ns: measured.wall_ns,
                        process_cpu_ns: measured.process_cpu_ns,
                        allocation_operations: measured.allocation_operations,
                        allocated_bytes: measured.allocated_bytes,
                        response_bytes: None,
                        result_rows: actual.row_count(),
                        result_hash_sha256: actual.hash,
                        correctness: true,
                        http_status: None,
                        backend_sparql_queries: None,
                        backend_sparql_updates: None,
                        backend_blob_gets: None,
                        backend_blob_puts: None,
                        backend_blob_other: None,
                        backend_total_operations: None,
                        backend_max_in_flight: None,
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn run_http(
    output: &mut impl Write,
    config: &Config,
    corpus: DeploymentCorpus,
    corpus_generation_ns: u64,
) -> Result<(), String> {
    let store_limits = benchmark_store_limits(&corpus)?;
    let store_limit_bytes = u64::try_from(store_limits.max_total_bytes).unwrap_or(u64::MAX);
    let store_limit_resources = u64::try_from(store_limits.max_resource_count).unwrap_or(u64::MAX);

    #[cfg(feature = "ac-query-scale-instrumentation")]
    {
        type CountingStore = CompositeStore<
            CountingSparqClient<InMemorySparqClient>,
            CountingBlobStore<InMemoryBlobStore>,
        >;
        let counters = BackendCounters::new();
        let sparq = CountingSparqClient::new(
            InMemorySparqClient::with_limits(store_limits),
            std::sync::Arc::clone(&counters),
        );
        let blob = CountingBlobStore::new(
            InMemoryBlobStore::with_limits(store_limits),
            std::sync::Arc::clone(&counters),
        );
        let store: CountingStore = CompositeStore::new(sparq, blob);
        run_http_with_store(
            output,
            config,
            corpus,
            corpus_generation_ns,
            store,
            Some(counters),
            store_limit_bytes,
            store_limit_resources,
        )
    }

    #[cfg(not(feature = "ac-query-scale-instrumentation"))]
    {
        let store = CompositeStore::new(
            InMemorySparqClient::with_limits(store_limits),
            InMemoryBlobStore::with_limits(store_limits),
        );
        run_http_with_store(
            output,
            config,
            corpus,
            corpus_generation_ns,
            store,
            None,
            store_limit_bytes,
            store_limit_resources,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn run_http_with_store<S: Store + 'static>(
    output: &mut impl Write,
    config: &Config,
    corpus: DeploymentCorpus,
    corpus_generation_ns: u64,
    store: S,
    counters: Option<std::sync::Arc<BackendCounters>>,
    store_limit_bytes: u64,
    store_limit_resources: u64,
) -> Result<(), String> {
    let hash = corpus_hash(&corpus);
    let counts = corpus_counts(&corpus, config.principal, config.lane);
    let common = common_record(config, hash);
    let workload = corpus.benchmark_queries(0)?;
    emit_applicability(output, &common, config, &workload)?;
    let selected = selected_queries(config, &workload)?;
    let applicable: Vec<_> = selected
        .into_iter()
        .filter(|query| config.params.triples_per_document >= query.minimum_triples_per_document)
        .collect();
    if applicable.is_empty() {
        return Err(
            "no selected query is applicable to this triples-per-document setting".to_owned(),
        );
    }

    let owner_webid = corpus.pods[0].owner_webid.clone();
    let recipient_webid = corpus.pods[0].recipient_webid.clone();
    let webid = match config.principal {
        PrincipalKind::Owner => Some(owner_webid.as_str()),
        PrincipalKind::Recipient => Some(recipient_webid.as_str()),
        PrincipalKind::Stranger => Some(STRANGER_WEBID),
        PrincipalKind::Anonymous => None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot construct Tokio runtime: {error}"))?;

    let issuer_key = support::BenchKey::generate();
    let client_key = support::BenchKey::generate();
    let access_token = webid
        .map(|webid| support::mint_access_token_webid(&issuer_key, &client_key.thumbprint, webid));

    // Runtime creation and client-side key/token minting are harness work. The counted
    // construction boundary begins when the service is seeded and ends after the LWS
    // application is assembled.
    let (alloc_start, bytes_start) = support::alloc_snapshot();
    let seed_start = Instant::now();
    runtime.block_on(seed_http_corpus(&store, &corpus))?;
    let lws_seed_ns = duration_ns(seed_start.elapsed());
    let app = support::assemble_app(store, &issuer_key, 1_024);
    let (alloc_end, bytes_end) = support::alloc_snapshot();

    // The current endpoint is server-wide: it starts from the shared server root and
    // admits every readable descendant, not only Pod 0. The physical oracle must model
    // that exact active dataset. In the primary all-private intervention, background
    // Pods remain unreadable and this reduces to Pod 0's fixed slice. Construct it only
    // after the server-allocation interval has closed.
    let reference_nquads = corpus.readable_content_nquads(config.principal.oracle());
    let reference = sparq_core::Graph::load_dataset(&reference_nquads, "nquads")?;
    drop(reference_nquads);

    let mut expected = HashMap::new();
    for query in &applicable {
        let json = sparq_engine::query_json(&reference, &query.sparql)?;
        expected.insert(
            query.id,
            CanonicalResult::from_sparql_json(json.as_bytes())?,
        );
    }

    let mut checks = 0_u32;
    for query in &applicable {
        let request =
            query_request(&client_key, access_token.as_deref(), &query.sparql).to_request();
        let (status, body) = runtime.block_on(drive_http(&app, request))?;
        if status != 200 {
            return Err(format!(
                "HTTP correctness request for {} returned {status}: {}",
                query.id,
                String::from_utf8_lossy(&body)
            ));
        }
        let actual = CanonicalResult::from_sparql_json(&body)?;
        if actual != expected[query.id] {
            return Err(format!("HTTP correctness mismatch for {}", query.id));
        }
        checks += 1;
    }
    emit(
        output,
        &CorrectnessRecord {
            common: &common,
            record_type: "correctness-gate",
            utc_unix_ns: utc_unix_ns(),
            gate: "primary principal through real HTTP route against physically filtered reference",
            principals_checked: 1,
            queries_checked: checks,
            exact_result_bags: true,
        },
    )?;

    drop(corpus);
    let (resident_bytes, peak_resident_bytes) = resident_memory();
    emit(
        output,
        &ConstructionRecord {
            common: &common,
            record_type: "construction",
            utc_unix_ns: utc_unix_ns(),
            counts: &counts,
            corpus_generation_ns,
            graph_load_ns: None,
            wac_materialization_ns: None,
            route_index_ns: None,
            lws_seed_ns: Some(lws_seed_ns),
            auth_triples_total: None,
            http_store_max_total_bytes: Some(store_limit_bytes),
            http_store_max_resource_count: Some(store_limit_resources),
            construction_allocations: cfg!(feature = "ac-query-scale-instrumentation")
                .then(|| alloc_end.saturating_sub(alloc_start)),
            construction_allocated_bytes: cfg!(feature = "ac-query-scale-instrumentation")
                .then(|| bytes_end.saturating_sub(bytes_start)),
            resident_bytes,
            peak_resident_bytes,
        },
    )?;
    let clock = ProcessCpuClock::discover();
    for (query_index, query) in applicable.iter().enumerate() {
        let expected = expected
            .get(query.id)
            .ok_or_else(|| format!("missing expected result for {}", query.id))?;
        for _ in 0..config.warmups {
            let request =
                query_request(&client_key, access_token.as_deref(), &query.sparql).to_request();
            let (status, body) = runtime.block_on(drive_http(&app, request))?;
            if status != 200 || CanonicalResult::from_sparql_json(&body)? != *expected {
                return Err(format!("HTTP warm-up mismatch for {}", query.id));
            }
            let plain = sparq_engine::query_json(&reference, &query.sparql)?;
            if CanonicalResult::from_sparql_json(plain.as_bytes())? != *expected {
                return Err(format!("plain warm-up mismatch for {}", query.id));
            }
        }

        for repetition in 0..config.repetitions {
            let guarded_first = random_guarded_first(config, query_index, repetition);
            for order_in_pair in 0..2_u8 {
                let guarded = (order_in_pair == 0) == guarded_first;
                if guarded {
                    // Proof minting, request construction, and body cloning are client-side
                    // operations and deliberately occur before both measurement windows.
                    let request =
                        query_request(&client_key, access_token.as_deref(), &query.sparql)
                            .to_request();
                    let backend_scope = counters.as_ref().map(|value| value.measure());
                    let measured = timed(&clock, || runtime.block_on(drive_http(&app, request)));
                    let backend = backend_scope.map(|scope| scope.delta());
                    let (status, body) = measured.value?;
                    if status != 200 {
                        return Err(format!(
                            "timed HTTP request for {} returned {status}: {}",
                            query.id,
                            String::from_utf8_lossy(&body)
                        ));
                    }
                    let actual = CanonicalResult::from_sparql_json(&body)?;
                    if actual != *expected {
                        return Err(format!(
                            "timed HTTP result mismatch for {} at repetition {repetition}",
                            query.id
                        ));
                    }
                    emit(
                        output,
                        &ObservationRecord {
                            common: &common,
                            record_type: "observation",
                            utc_unix_ns: utc_unix_ns(),
                            query_id: query.id,
                            query_family: query.family,
                            query_hash_sha256: query_hash(&query.sparql),
                            operation: "native-http-request",
                            pair_id: format!(
                                "{}:{}:{}:{}",
                                config.run_id, config.process_block, query.id, repetition
                            ),
                            repetition,
                            warmup: false,
                            order_in_pair,
                            wall_ns: measured.wall_ns,
                            process_cpu_ns: measured.process_cpu_ns,
                            allocation_operations: measured.allocation_operations,
                            allocated_bytes: measured.allocated_bytes,
                            response_bytes: Some(body.len()),
                            result_rows: actual.row_count(),
                            result_hash_sha256: actual.hash,
                            correctness: true,
                            http_status: Some(status),
                            backend_sparql_queries: backend
                                .as_ref()
                                .map(|value| value.sparql_queries),
                            backend_sparql_updates: backend
                                .as_ref()
                                .map(|value| value.sparql_updates),
                            backend_blob_gets: backend.as_ref().map(|value| value.blob_gets),
                            backend_blob_puts: backend.as_ref().map(|value| value.blob_puts),
                            backend_blob_other: backend.as_ref().map(|value| value.blob_others),
                            backend_total_operations: backend
                                .as_ref()
                                .map(|value| value.total_backend_ops()),
                            backend_max_in_flight: backend
                                .as_ref()
                                .map(|value| value.max_in_flight),
                        },
                    )?;
                } else {
                    let measured = timed(&clock, || {
                        sparq_engine::query_json(&reference, &query.sparql)
                    });
                    let body = measured.value?;
                    let actual = CanonicalResult::from_sparql_json(body.as_bytes())?;
                    if actual != *expected {
                        return Err(format!(
                            "timed plain result mismatch for {} at repetition {repetition}",
                            query.id
                        ));
                    }
                    emit(
                        output,
                        &ObservationRecord {
                            common: &common,
                            record_type: "observation",
                            utc_unix_ns: utc_unix_ns(),
                            query_id: query.id,
                            query_family: query.family,
                            query_hash_sha256: query_hash(&query.sparql),
                            operation: "plain-engine-json-reference",
                            pair_id: format!(
                                "{}:{}:{}:{}",
                                config.run_id, config.process_block, query.id, repetition
                            ),
                            repetition,
                            warmup: false,
                            order_in_pair,
                            wall_ns: measured.wall_ns,
                            process_cpu_ns: measured.process_cpu_ns,
                            allocation_operations: measured.allocation_operations,
                            allocated_bytes: measured.allocated_bytes,
                            response_bytes: Some(body.len()),
                            result_rows: actual.row_count(),
                            result_hash_sha256: actual.hash,
                            correctness: true,
                            http_status: None,
                            backend_sparql_queries: None,
                            backend_sparql_updates: None,
                            backend_blob_gets: None,
                            backend_blob_puts: None,
                            backend_blob_other: None,
                            backend_total_operations: None,
                            backend_max_in_flight: None,
                        },
                    )?;
                }
            }
        }
    }
    Ok(())
}

async fn seed_http_corpus<S: Store>(store: &S, corpus: &DeploymentCorpus) -> Result<(), String> {
    let server_root = format!("{}/", support::BASE_URL.trim_end_matches('/'));
    store
        .write(&server_root, Bytes::new(), "text/turtle")
        .await
        .map_err(|error| format!("seed server root: {error}"))?;
    for container in &corpus.containers {
        let parent = container.parent_iri.as_deref().ok_or_else(|| {
            format!(
                "HTTP container {} has no shared-origin parent",
                container.iri
            )
        })?;
        store
            .create_in_container(parent, &container.iri, Bytes::new(), "text/turtle")
            .await
            .map_err(|error| format!("seed container {}: {error}", container.iri))?;
    }
    for document in &corpus.documents {
        let slash = document
            .iri
            .rfind('/')
            .ok_or_else(|| format!("content IRI has no parent: {}", document.iri))?;
        let parent = &document.iri[..=slash];
        store
            .create_in_container(
                parent,
                &document.iri,
                Bytes::from(document.ntriples.clone()),
                "text/turtle",
            )
            .await
            .map_err(|error| format!("seed content {}: {error}", document.iri))?;
    }
    // ACL resources are auxiliaries and intentionally do not enter ldp:contains.
    for control in &corpus.control_documents {
        store
            .write(
                &control.iri,
                Bytes::from(control.ntriples.clone()),
                "text/turtle",
            )
            .await
            .map_err(|error| format!("seed ACL {}: {error}", control.iri))?;
    }
    Ok(())
}

fn benchmark_store_limits(corpus: &DeploymentCorpus) -> Result<InMemoryStoreLimits, String> {
    let max_total_bytes = corpus
        .documents
        .iter()
        .map(|document| document.ntriples.len())
        .chain(
            corpus
                .control_documents
                .iter()
                .map(|document| document.ntriples.len()),
        )
        .try_fold(0_usize, |total, bytes| total.checked_add(bytes))
        .ok_or_else(|| "HTTP corpus byte count overflows usize".to_owned())?;
    let max_resource_count = 1_usize
        .checked_add(corpus.containers.len())
        .and_then(|total| total.checked_add(corpus.documents.len()))
        .and_then(|total| total.checked_add(corpus.control_documents.len()))
        .ok_or_else(|| "HTTP corpus resource count overflows usize".to_owned())?;
    Ok(InMemoryStoreLimits::new(
        max_total_bytes,
        max_resource_count,
    ))
}

fn query_request(
    client_key: &support::BenchKey,
    access_token: Option<&str>,
    query: &str,
) -> support::PreReq {
    if let Some(access_token) = access_token {
        support::authed_prereq(
            client_key,
            access_token,
            "POST",
            "/sparql",
            Some("application/sparql-query"),
            &[("accept", "application/sparql-results+json")],
            Bytes::from(query.to_owned()),
        )
    } else {
        support::PreReq {
            method: "POST".to_owned(),
            path: "/sparql".to_owned(),
            authz: None,
            dpop: None,
            content_type: Some("application/sparql-query".to_owned()),
            extra: vec![(
                "accept".to_owned(),
                "application/sparql-results+json".to_owned(),
            )],
            body: Bytes::from(query.to_owned()),
        }
    }
}

async fn drive_http(app: &axum::Router, request: Request<Body>) -> Result<(u16, Bytes), String> {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| format!("router service error: {error}"))?;
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| format!("response buffering failed: {error}"))?;
    Ok((status, body))
}

fn main() -> Result<(), String> {
    let Some(config) = Config::parse()? else {
        return Ok(());
    };
    let generation_start = Instant::now();
    let corpus = generate(&config.params)?;
    let corpus_generation_ns = duration_ns(generation_start.elapsed());
    let stdout = std::io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    match config.lane {
        Lane::Materialized => {
            run_materialized(&mut output, &config, corpus, corpus_generation_ns)?;
        }
        Lane::Http => run_http(&mut output, &config, corpus, corpus_generation_ns)?,
    }
    output.flush().map_err(|error| error.to_string())
}

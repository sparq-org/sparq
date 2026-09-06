// [GPT-6] Packed independent Pod snapshots and a bounded active dataset cache.
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Instant;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sparq_acbench::population::{self, PolicyModel, PopulationConfig};
use sparq_lws_core::auth::VerifiedToken;
use sparq_solid::{Mode, PodStore, Session};

use super::{Result, Settings};

const INDEX_BYTES: u64 = 24;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Policy {
    Wac,
    Acp,
}

impl Policy {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "wac" => Ok(Self::Wac),
            "acp" => Ok(Self::Acp),
            _ => Err("model must be wac or acp".into()),
        }
    }

    fn materialize(self, store: &mut PodStore) -> Result<()> {
        match self {
            Self::Wac => store.materialize_wac(),
            Self::Acp => store.materialize_acp(),
        }
        .map_err(|e| format!("materialization: {e}"))?;
        Ok(())
    }

    fn update(self, store: &mut PodStore, session: &Session<'_>, query: &str) -> Result<()> {
        match self {
            Self::Wac => store.update_as(session, query),
            Self::Acp => store.update_as_acp(session, query),
        }
        .map_err(|e| format!("update: {e}"))?;
        Ok(())
    }
}

struct Entry {
    store: PodStore,
    source_bytes: u64,
    used: u64,
}

pub(super) struct PodCache {
    directory: PathBuf,
    index: File,
    packed: File,
    pods: u64,
    model: Policy,
    config: PopulationConfig,
    entries: HashMap<u64, Entry>,
    maximum_entries: usize,
    maximum_bytes: u64,
    maximum_pod_bytes: u64,
    maximum_journal_bytes: u64,
    bytes: u64,
    clock: u64,
}

pub(super) struct Outcome {
    pub(super) body: String,
    pub(super) cache_hit: bool,
    pub(super) load_us: u64,
    pub(super) materialize_us: u64,
    pub(super) cache_entries: usize,
    pub(super) cache_bytes: u64,
    pub(super) policy_triple_delta: i64,
}

pub(super) struct OperationError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

#[derive(Deserialize, Serialize)]
struct JournalEntry {
    agent: Option<String>,
    client: Option<String>,
    issuer: Option<String>,
    query: String,
    #[serde(default)]
    policy_administration: bool,
}

impl JournalEntry {
    fn session(&self) -> Session<'_> {
        Session {
            agent: self.agent.as_deref(),
            client: self.client.as_deref(),
            issuer: self.issuer.as_deref(),
            now: None,
        }
    }
}

impl PodCache {
    pub(super) fn open(settings: &Settings, workers: usize) -> Result<Self> {
        let directory = settings.path("corpus", "population-corpus");
        let manifest: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)?;
        if manifest["format"] != "sparq-pod-pack-zstd-v1" {
            return Err("unrecognized corpus format".into());
        }
        let pods = manifest["pods"].as_u64().ok_or("manifest missing pods")?;
        let model = Policy::parse(manifest["model"].as_str().ok_or("manifest missing model")?)?;
        let config = serde_json::from_value(manifest["config"].clone())?;
        let index = File::open(directory.join("pods.index"))?;
        if index.metadata()?.len() != pods.checked_mul(INDEX_BYTES).ok_or("index overflow")? {
            return Err("index length does not match populated Pod count".into());
        }
        let maximum_entries = usize::try_from(settings.number("cache-pods", 64)?)? / workers;
        let maximum_bytes = settings.number("cache-bytes", 256 * 1024 * 1024)? / workers as u64;
        if maximum_entries == 0 || maximum_bytes == 0 {
            return Err("each worker needs a positive cache allocation".into());
        }
        let packed = File::open(directory.join("pods.nqpack"))?;
        Ok(Self {
            directory,
            index,
            packed,
            pods,
            model,
            config,
            entries: HashMap::new(),
            maximum_entries,
            maximum_bytes,
            maximum_pod_bytes: settings.number("max-pod-bytes", 128 * 1024 * 1024)?,
            maximum_journal_bytes: settings.number("max-journal-bytes", 8 * 1024 * 1024)?,
            bytes: 0,
            clock: 0,
        })
    }

    fn evict(&mut self, pod: u64) {
        if let Some(entry) = self.entries.remove(&pod) {
            self.bytes -= entry.source_bytes;
        }
    }

    fn journal_path(&self, pod: u64) -> PathBuf {
        self.directory
            .join("updates")
            .join((pod / 1000).to_string())
            .join(format!("{pod}.jsonl"))
    }

    fn get(&mut self, pod: u64) -> Result<(bool, u64, u64)> {
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&pod) {
            entry.used = self.clock;
            return Ok((true, 0, 0));
        }
        if pod >= self.pods {
            return Err("Pod outside persisted corpus".into());
        }
        let start = Instant::now();
        let mut record = [0_u8; INDEX_BYTES as usize];
        self.index.seek(SeekFrom::Start(pod * INDEX_BYTES))?;
        self.index.read_exact(&mut record)?;
        let offset = u64::from_le_bytes(record[0..8].try_into()?);
        let packed_bytes = u64::from_le_bytes(record[8..16].try_into()?);
        let raw_bytes = u64::from_le_bytes(record[16..24].try_into()?);
        if raw_bytes == 0 || raw_bytes > self.maximum_pod_bytes || raw_bytes > self.maximum_bytes {
            return Err(
                format!("Pod source size {raw_bytes} exceeds configured admission budget").into(),
            );
        }
        if packed_bytes == 0
            || offset
                .checked_add(packed_bytes)
                .ok_or("packed offset overflow")?
                > self.packed.metadata()?.len()
        {
            return Err("invalid packed Pod extent".into());
        }
        while self.entries.len() >= self.maximum_entries
            || self.bytes + raw_bytes > self.maximum_bytes
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
                .ok_or("cache budget invariant")?;
            self.evict(oldest);
        }
        self.packed.seek(SeekFrom::Start(offset))?;
        let decoder = zstd::stream::read::Decoder::new((&mut self.packed).take(packed_bytes))?;
        let mut source = String::with_capacity(usize::try_from(raw_bytes)?);
        decoder.take(raw_bytes + 1).read_to_string(&mut source)?;
        if source.len() as u64 != raw_bytes {
            return Err("decoded Pod length differs from committed index".into());
        }
        let graph = sparq_core::Graph::load_dataset(&source, "nquads")?;
        drop(source);
        let mut store = PodStore::new(graph);
        let load_us = start.elapsed().as_micros() as u64;
        let start = Instant::now();
        self.model.materialize(&mut store)?;
        let journal_path = self.journal_path(pod);
        if journal_path.exists() {
            let file = File::open(journal_path)?;
            if file.metadata()?.len() > self.maximum_journal_bytes {
                return Err("update journal exceeds configured admission budget".into());
            }
            for line in BufReader::new(file).lines() {
                let entry: JournalEntry = serde_json::from_str(&line?)?;
                if entry.policy_administration {
                    administer_policy(
                        &mut store,
                        self.model,
                        &self.config,
                        pod,
                        &entry.session(),
                        &entry.query,
                    )?;
                } else {
                    self.model
                        .update(&mut store, &entry.session(), &entry.query)?;
                }
            }
        }
        let materialize_us = start.elapsed().as_micros() as u64;
        self.entries.insert(
            pod,
            Entry {
                store,
                source_bytes: raw_bytes,
                used: self.clock,
            },
        );
        self.bytes += raw_bytes;
        Ok((false, load_us, materialize_us))
    }

    pub(super) fn execute(
        &mut self,
        pod: u64,
        token: &VerifiedToken,
        query: &str,
        update: bool,
        administration: bool,
    ) -> std::result::Result<Outcome, OperationError> {
        if pod >= self.pods {
            return Err(OperationError {
                status: StatusCode::NOT_FOUND,
                message: "unknown Pod".into(),
            });
        }
        let result = self.execute_inner(pod, token, query, update, administration);
        result.map_err(|error| {
            // An unsuccessful mutation may have changed an in-memory graph: discard it,
            // then reconstruct solely from the durable snapshot and accepted journal.
            if update {
                self.evict(pod);
            }
            OperationError {
                status: if update {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::BAD_REQUEST
                },
                message: error.to_string(),
            }
        })
    }

    fn execute_inner(
        &mut self,
        pod: u64,
        token: &VerifiedToken,
        query: &str,
        update: bool,
        administration: bool,
    ) -> Result<Outcome> {
        let (cache_hit, load_us, materialize_us) = self.get(pod)?;
        let session = Session {
            agent: token.web_id.as_deref(),
            client: token.client_id.as_deref(),
            issuer: token.issuer.as_deref(),
            now: None,
        };
        let journal_path = self.journal_path(pod);
        let entry = self
            .entries
            .get_mut(&pod)
            .ok_or("loaded Pod absent from cache")?;
        let policy_count = |store: &PodStore| -> u64 {
            store
                .graph
                .named
                .iter()
                .filter(|(name, _)| {
                    let name = name.to_string();
                    name.ends_with(".acl>") || name.ends_with(".acr>")
                })
                .map(|(_, graph)| graph.len() as u64)
                .sum()
        };
        let policies_before = if administration {
            policy_count(&entry.store)
        } else {
            0
        };
        let body = if update {
            if token.is_public() {
                return Err("authenticated update required".into());
            }
            let journal = JournalEntry {
                agent: token.web_id.clone(),
                client: token.client_id.clone(),
                issuer: token.issuer.clone(),
                query: query.into(),
                policy_administration: administration,
            };
            let serialized = serde_json::to_vec(&journal)?;
            let current = fs::metadata(&journal_path).map_or(0, |metadata| metadata.len());
            if current + serialized.len() as u64 + 1 > self.maximum_journal_bytes {
                return Err("journal is full".into());
            }
            if administration {
                administer_policy(
                    &mut entry.store,
                    self.model,
                    &self.config,
                    pod,
                    &session,
                    query,
                )?;
            } else {
                self.model.update(&mut entry.store, &session, query)?;
            }
            commit_journal(&journal_path, &serialized)?;
            json!({"updated":true,"policy_administration":administration}).to_string()
        } else {
            match spargebra::SparqlParser::new().parse_query(query)? {
                spargebra::Query::Select { .. } | spargebra::Query::Ask { .. } => (),
                _ => return Err("benchmark route accepts SELECT and ASK only".into()),
            }
            entry.store.query_json_as(&session, Mode::Read, query)?
        };
        let policy_triple_delta = if administration {
            policy_count(&entry.store) as i64 - policies_before as i64
        } else {
            0
        };
        Ok(Outcome {
            body,
            cache_hit,
            load_us,
            materialize_us,
            policy_triple_delta,
            cache_entries: self.entries.len(),
            cache_bytes: self.bytes,
        })
    }
}

fn commit_journal(path: &std::path::Path, serialized: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("journal parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".journal-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        match File::open(path) {
            Ok(mut previous) => {
                std::io::copy(&mut previous, &mut file)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        file.write_all(serialized)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        // Persist newly created journal directory entries through their ancestors.
        if let Some(grandparent) = parent.parent() {
            File::open(grandparent)?.sync_all()?;
        }
        if let Some(corpus) = parent.parent().and_then(std::path::Path::parent) {
            File::open(corpus)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn administer_policy(
    store: &mut PodStore,
    model: Policy,
    config: &PopulationConfig,
    pod: u64,
    session: &Session<'_>,
    query: &str,
) -> Result<()> {
    if session.agent != Some(population::owner_webid(config, pod).as_str()) {
        return Err(
            "policy administration requires the manifest-bound authenticated Pod owner".into(),
        );
    }
    let root = population::pod_root(config, pod);
    let suffix = match model {
        Policy::Wac => ".acl",
        Policy::Acp => ".acr",
    };
    let update = spargebra::SparqlParser::new().parse_update(query)?;
    let validate = |graph: &spargebra::term::GraphName| -> Result<()> {
        let spargebra::term::GraphName::NamedNode(name) = graph else {
            return Err("policy administration requires explicit named policy graphs".into());
        };
        if !name.as_str().starts_with(&root)
            || !name.as_str().ends_with(suffix)
            || !store
                .graph
                .named
                .iter()
                .any(|(existing, _)| existing.to_string() == name.to_string())
        {
            return Err(
                "policy administration target is not an existing policy of this Pod".into(),
            );
        }
        Ok(())
    };
    for operation in &update.operations {
        match operation {
            spargebra::GraphUpdateOperation::InsertData { data } => {
                for quad in data {
                    validate(&quad.graph_name)?;
                }
            }
            spargebra::GraphUpdateOperation::DeleteData { data } => {
                for quad in data {
                    validate(&quad.graph_name)?;
                }
            }
            _ => {
                return Err(
                    "policy administration accepts only explicit INSERT DATA / DELETE DATA".into(),
                )
            }
        }
    }
    sparq_engine::update_in_place(&mut store.graph, query)?;
    model.materialize(store)?;
    Ok(())
}

fn digest_file(path: PathBuf) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn pack(settings: &Settings) -> Result<()> {
    let directory = settings.path("corpus", "population-corpus");
    let pods = settings.number("pods", 8)?;
    if pods == 0 {
        return Err("cannot pack an empty population".into());
    }
    let config: PopulationConfig = if settings.0.contains_key("config-file") {
        serde_json::from_slice(&fs::read(settings.path("config-file", ""))?)?
    } else {
        match settings.text("profile", "smoke").as_str() {
            "smoke" => PopulationConfig::smoke(),
            "history" => PopulationConfig::service_history(),
            _ => return Err("profile must be smoke or history".into()),
        }
    };
    let model = Policy::parse(&settings.text("model", "wac"))?;
    let model_name = match model {
        Policy::Wac => "wac",
        Policy::Acp => "acp",
    };
    let policy = match model {
        Policy::Wac => PolicyModel::Wac,
        Policy::Acp => PolicyModel::Acp,
    };
    fs::create_dir_all(&directory)?;
    let mut packed = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("pods.nqpack"))?,
    );
    let mut index = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("pods.index"))?,
    );
    let mut summaries = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("pod-summaries.jsonl"))?,
    );
    let started = Instant::now();
    let mut raw_bytes = 0_u64;
    let mut quads = 0_u64;
    let mut records = 0_u64;
    let mut maximum_pod_bytes = 0_u64;
    let mut maximum_compressed_bytes = 0_u64;
    let mut last_progress = Instant::now();
    for pod in 0..pods {
        let offset = packed.stream_position()?;
        let mut encoder = zstd::stream::write::Encoder::new(&mut packed, 3)?;
        let summary = population::write_pod(&config, pod, policy, &mut encoder)?;
        encoder.finish()?;
        let end = packed.stream_position()?;
        index.write_all(&offset.to_le_bytes())?;
        index.write_all(&(end - offset).to_le_bytes())?;
        index.write_all(&summary.bytes.to_le_bytes())?;
        let mut value = serde_json::to_value(&summary)?;
        value["compressed_bytes"] = json!(end - offset);
        serde_json::to_writer(&mut summaries, &value)?;
        summaries.write_all(b"\n")?;
        raw_bytes += summary.bytes;
        quads += summary.quads;
        records += summary.records;
        maximum_pod_bytes = maximum_pod_bytes.max(summary.bytes);
        maximum_compressed_bytes = maximum_compressed_bytes.max(end - offset);
        if last_progress.elapsed().as_secs() >= 30 || pod + 1 == pods {
            println!(
                "{}",
                json!({"record_type":"pack-progress","pods":pod+1,
                "target_pods":pods,"packed_bytes":end,"source_bytes":raw_bytes,
                "elapsed_us":started.elapsed().as_micros() as u64})
            );
            last_progress = Instant::now();
        }
    }
    packed.flush()?;
    index.flush()?;
    summaries.flush()?;
    packed.get_ref().sync_all()?;
    index.get_ref().sync_all()?;
    summaries.get_ref().sync_all()?;
    let generation_us = started.elapsed().as_micros() as u64;
    let packed_bytes = packed.get_ref().metadata()?.len();
    let manifest = json!({"format":"sparq-pod-pack-zstd-v1", "pods":pods,
        "model":model_name,"config":config,"source_bytes":raw_bytes,"packed_bytes":packed_bytes,
        "index_bytes":pods*INDEX_BYTES,"quads":quads,"records":records,
        "maximum_pod_source_bytes":maximum_pod_bytes,"maximum_pod_compressed_bytes":maximum_compressed_bytes,
        "generation_us":generation_us,"compression":"zstd independent frames level3",
        "packed_sha256":digest_file(directory.join("pods.nqpack"))?,
        "index_sha256":digest_file(directory.join("pods.index"))?,
        "binary_payloads_included":false,"populated":true});
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "{}",
        json!({"record_type":"pack-complete","manifest":manifest})
    );
    Ok(())
}

pub(super) fn verify(settings: &Settings) -> Result<()> {
    let mut cache = PodCache::open(settings, 1)?;
    let manifest: Value =
        serde_json::from_slice(&fs::read(cache.directory.join("manifest.json"))?)?;
    let config: PopulationConfig = serde_json::from_value(manifest["config"].clone())?;
    let count = settings.number("verify-pods", cache.pods)?.min(cache.pods);
    let mut checked = 0_u64;
    for sample in 0..count {
        // Spread verification throughout the persisted population, including its endpoint.
        let pod = if count == 1 {
            0
        } else {
            sample * (cache.pods - 1) / (count - 1)
        };
        let owner = population::owner_webid(&config, pod);
        let recipient = population::recipient_webid(&config, pod, 0);
        for agent in [
            Some(owner.as_str()),
            Some(recipient.as_str()),
            Some("https://outsider.benchmark.example/profile#me"),
            None,
        ] {
            let token = VerifiedToken {
                web_id: agent.map(str::to_owned),
                ..VerifiedToken::default()
            };
            let mut readable = Vec::new();
            population::write_readable_content(&config, pod, agent, &mut readable)?;
            let reference =
                sparq_core::Graph::load_dataset(std::str::from_utf8(&readable)?, "nquads")?;
            drop(readable);
            for query in population::benchmark_queries(&config, pod) {
                let outcome = cache
                    .execute(pod, &token, &query.sparql, false, false)
                    .map_err(|error| error.message)?;
                let expected_json = sparq_engine::query_json(&reference, &query.sparql)?;
                let actual: Value = serde_json::from_str(&outcome.body)?;
                let expected: Value = serde_json::from_str(&expected_json)?;
                let ordered = query.sparql.contains("ORDER BY");
                if normalized_rows(&actual, ordered)? != normalized_rows(&expected, ordered)? {
                    return Err(format!(
                        "solution mismatch Pod {pod}, agent {agent:?}, query {}",
                        query.id
                    )
                    .into());
                }
                if query.id == "q2-count" {
                    let count: u64 = actual
                        .pointer("/results/bindings/0/count/value")
                        .and_then(Value::as_str)
                        .ok_or("count result absent")?
                        .parse()?;
                    let expected_count = population::expected_record_count(&config, pod, agent)?;
                    if count != expected_count {
                        return Err(format!(
                            "count oracle mismatch Pod{pod}, got{count}, expected{expected_count}"
                        )
                        .into());
                    }
                }
                checked += 1;
            }
        }
        if sample % 100 == 0 {
            println!(
                "{}",
                json!({"record_type":"verify-progress","sampled_pods":sample+1,"checks":checked})
            );
        }
    }
    println!(
        "{}",
        json!({"record_type":"verification-complete","sampled_pods":count,
        "checks":checked,"oracle":"policy-neutral physically filtered content; same SPARQ query evaluator; exact bags or ordered rows plus independent counts",
        "authentication":"trusted-session deterministic check; HTTP authentication checked separately"})
    );
    Ok(())
}

fn normalized_rows(value: &Value, ordered: bool) -> Result<Vec<String>> {
    if let Some(boolean) = value.get("boolean") {
        return Ok(vec![boolean.to_string()]);
    }
    let bindings = value
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .ok_or("result bindings missing")?;
    let mut rows = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let fields: std::collections::BTreeMap<_, _> = binding
            .as_object()
            .ok_or("binding is not an object")?
            .iter()
            .collect();
        rows.push(serde_json::to_string(&fields)?);
    }
    if !ordered {
        rows.sort();
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "https://pods.example/p/0/data";
    const OWNER: &str = "https://pods.example/p/0/profile/card#me";
    const RECIPIENT: &str = "https://recipient.example/#me";
    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";
    const QUERY: &str =
        "SELECT (COUNT(*) AS ?count) WHERE { GRAPH ?g { ?s <https://example.org/value> ?value } }";

    fn token(agent: &str) -> VerifiedToken {
        VerifiedToken {
            web_id: Some(agent.into()),
            ..VerifiedToken::default()
        }
    }

    fn source(model: Policy) -> String {
        let content = format!("<{DOC}#it> <https://example.org/value> \"private\" <{DOC}> .\n");
        match model {
            Policy::Wac => {
                let graph = format!("{DOC}.acl");
                let mut source = content;
                for (id, agent, modes) in [
                    ("owner", OWNER, vec!["Read", "Write", "Control"]),
                    ("recipient", RECIPIENT, vec!["Read"]),
                ] {
                    let subject = format!("{graph}#{id}");
                    for (predicate, object) in [
                        (
                            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".into(),
                            format!("{ACL}Authorization"),
                        ),
                        (format!("{ACL}accessTo"), DOC.into()),
                        (format!("{ACL}agent"), agent.into()),
                    ] {
                        source.push_str(&format!(
                            "<{subject}> <{predicate}> <{object}> <{graph}> .\n"
                        ));
                    }
                    for mode in modes {
                        source.push_str(&format!(
                            "<{subject}> <{ACL}mode> <{ACL}{mode}> <{graph}> .\n"
                        ));
                    }
                }
                source
            }
            Policy::Acp => {
                let mut builder = sparq_solid::conformance::AcrBuilder::new();
                builder.access_control(DOC, |p| {
                    p.allow(Mode::Read)
                        .allow(Mode::Write)
                        .allow(Mode::Control)
                        .any_of_agent(OWNER)
                });
                builder.access_control(DOC, |p| p.allow(Mode::Read).any_of_agent(RECIPIENT));
                format!("{content}{}", builder.into_nquads())
            }
        }
    }

    fn fixture(model: Policy) -> Result<(Settings, PathBuf)> {
        let directory =
            std::env::temp_dir().join(format!("sparq-pop-storage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory)?;
        let source = source(model);
        let compressed = zstd::stream::encode_all(source.as_bytes(), 1)?;
        let mut data = File::create(directory.join("pods.nqpack"))?;
        let mut index = File::create(directory.join("pods.index"))?;
        for pod in 0..2 {
            index.write_all(&(pod * compressed.len() as u64).to_le_bytes())?;
            index.write_all(&(compressed.len() as u64).to_le_bytes())?;
            index.write_all(&(source.len() as u64).to_le_bytes())?;
            data.write_all(&compressed)?;
        }
        fs::write(directory.join("manifest.json"), json!({"format":"sparq-pod-pack-zstd-v1","pods":2,"model":model,"config":PopulationConfig::smoke()}).to_string())?;
        let settings = Settings(HashMap::from([
            ("corpus".into(), directory.to_string_lossy().into_owned()),
            ("cache-pods".into(), "1".into()),
        ]));
        Ok((settings, directory))
    }

    fn count(cache: &mut PodCache, pod: u64, agent: &str) -> Result<u64> {
        let outcome = cache
            .execute(pod, &token(agent), QUERY, false, false)
            .map_err(|e| e.message)?;
        let value: Value = serde_json::from_str(&outcome.body)?;
        Ok(value
            .pointer("/results/bindings/0/count/value")
            .and_then(Value::as_str)
            .ok_or("missing count")?
            .parse()?)
    }

    #[test]
    fn revocation_survives_eviction_and_restart_in_both_languages() -> Result<()> {
        for model in [Policy::Wac, Policy::Acp] {
            let (settings, directory) = fixture(model)?;
            let mut cache = PodCache::open(&settings, 1)?;
            assert_eq!(count(&mut cache, 0, RECIPIENT)?, 1);
            assert_eq!(count(&mut cache, 0, "https://outsider.example/#me")?, 0);
            let (graph, subject, predicate, object) = match model {
                Policy::Wac => (
                    format!("{DOC}.acl"),
                    format!("{DOC}.acl#recipient"),
                    format!("{ACL}agent"),
                    RECIPIENT.to_owned(),
                ),
                Policy::Acp => (
                    format!("{DOC}.acr"),
                    format!("{DOC}.acr#ctl1"),
                    format!("{ACP}apply"),
                    format!("{DOC}.acr#pol1"),
                ),
            };
            let revoke = format!(
                "DELETE DATA {{ GRAPH <{graph}> {{ <{subject}> <{predicate}> <{object}> }} }}"
            );
            assert!(cache
                .execute(0, &token(RECIPIENT), &revoke, true, true)
                .is_err());
            assert!(cache
                .execute(0, &VerifiedToken::default(), &revoke, true, true)
                .is_err());
            assert_eq!(count(&mut cache, 0, RECIPIENT)?, 1);
            assert!(cache
                .execute(1, &token(OWNER), &revoke, true, true)
                .is_err());
            let cross_pod = revoke.replace("/p/0/", "/p/999/");
            assert!(cache
                .execute(0, &token(OWNER), &cross_pod, true, true)
                .is_err());
            let content_write = format!("INSERT DATA {{ GRAPH <{DOC}> {{ <{DOC}#it> <https://example.org/value> \"forged\" }} }}");
            assert!(cache
                .execute(0, &token(OWNER), &content_write, true, true)
                .is_err());
            assert!(cache
                .execute(
                    0,
                    &token(OWNER),
                    &format!("{revoke}; {content_write}"),
                    true,
                    true
                )
                .is_err());
            assert_eq!(count(&mut cache, 0, RECIPIENT)?, 1);
            cache
                .execute(0, &token(OWNER), &revoke, true, true)
                .map_err(|e| e.message)?;
            assert_eq!(count(&mut cache, 0, RECIPIENT)?, 0);
            assert_eq!(count(&mut cache, 1, RECIPIENT)?, 1);
            assert!(!cache.entries.contains_key(&0));
            assert_eq!(count(&mut cache, 0, RECIPIENT)?, 0);
            drop(cache);
            let mut restarted = PodCache::open(&settings, 1)?;
            assert_eq!(count(&mut restarted, 0, RECIPIENT)?, 0);
            assert_eq!(count(&mut restarted, 0, OWNER)?, 1);
            fs::remove_dir_all(directory)?;
        }
        Ok(())
    }

    #[test]
    fn corrupt_index_and_oversize_pod_fail_closed() -> Result<()> {
        let (mut settings, directory) = fixture(Policy::Wac)?;
        settings.0.insert("max-pod-bytes".into(), "1".into());
        let mut cache = PodCache::open(&settings, 1)?;
        assert!(count(&mut cache, 0, OWNER).is_err());
        assert!(cache.entries.is_empty());
        settings.0.remove("max-pod-bytes");
        drop(cache);
        let mut index = OpenOptions::new()
            .write(true)
            .open(directory.join("pods.index"))?;
        index.write_all(&u64::MAX.to_le_bytes())?;
        let mut cache = PodCache::open(&settings, 1)?;
        assert!(count(&mut cache, 0, OWNER).is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn failed_journal_creation_does_not_replace_acknowledged_state() -> Result<()> {
        let (settings, directory) = fixture(Policy::Wac)?;
        let mut cache = PodCache::open(&settings, 1)?;
        assert_eq!(count(&mut cache, 0, RECIPIENT)?, 1);
        fs::write(
            directory.join("updates"),
            b"injected directory creation failure",
        )?;
        let revoke = format!("DELETE DATA {{ GRAPH <{DOC}.acl> {{ <{DOC}.acl#recipient> <{ACL}agent> <{RECIPIENT}> }} }}");
        assert!(cache
            .execute(0, &token(OWNER), &revoke, true, true)
            .is_err());
        fs::remove_file(directory.join("updates"))?;
        drop(cache);
        let mut restarted = PodCache::open(&settings, 1)?;
        assert_eq!(count(&mut restarted, 0, RECIPIENT)?, 1);
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}

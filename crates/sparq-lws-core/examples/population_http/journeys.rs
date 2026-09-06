// [GPT-6] Prospective journey-weighted demand with inventory-derived mutations.
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sparq_acbench::population::{self, mutation, PopulationConfig, Service};
use tokio::task::JoinSet;

use super::auth::Credentials;
use super::load::{emit, send, unit, Outgoing};
use super::{Result, Settings};

// Seed is an independent key domain, never added to the counter: nearby frozen
// seeds must not replay shifted copies of the same arrival or request stream.
fn random(seed: u64, sequence: u64, stream: u64) -> u64 {
    let mut hash = Sha256::new();
    hash.update(b"sparq-journeys-v2\0");
    hash.update(seed.to_le_bytes());
    hash.update(sequence.to_le_bytes());
    hash.update(stream.to_le_bytes());
    u64::from_le_bytes(hash.finalize()[..8].try_into().expect("eight digest bytes"))
}

fn number(value: &Value) -> Result<f64> {
    let n = value.as_f64().ok_or("missing workload number")?;
    if !n.is_finite() || n < 0.0 {
        return Err("invalid workload number".into());
    }
    Ok(n)
}
fn weighted(weights: &[f64], draw: u64) -> usize {
    let target = unit(draw) * weights.iter().sum::<f64>();
    let mut cumulative = 0.0;
    weights
        .iter()
        .position(|weight| {
            cumulative += weight;
            target < cumulative
        })
        .unwrap_or(weights.len() - 1)
}
fn service_index(service: Service) -> usize {
    Service::ALL
        .iter()
        .position(|v| *v == service)
        .expect("all services")
}

struct Inventory {
    cumulative: [Vec<u64>; 8],
    pods: u64,
    active: u64,
}
impl Inventory {
    fn load(settings: &Settings, pods: u64, active: u64) -> Result<Self> {
        let mut cumulative: [Vec<u64>; 8] = std::array::from_fn(|_| vec![0]);
        let input = BufReader::new(File::open(
            settings
                .path("corpus", "population-corpus")
                .join("pod-summaries.jsonl"),
        )?);
        for line in input.lines() {
            let row: Value = serde_json::from_str(&line?)?;
            let id = row["pod_id"].as_u64().ok_or("summary Pod ID")?;
            if id != cumulative[0].len() as u64 - 1 || id >= pods {
                return Err("noncontiguous or extra inventory row".into());
            }
            for service in Service::ALL {
                let counts = &mut cumulative[service_index(service)];
                let n = row["records_by_service"][service.name()]
                    .as_u64()
                    .ok_or("summary service count")?;
                counts.push(
                    counts
                        .last()
                        .ok_or("inventory accumulator")?
                        .checked_add(n)
                        .ok_or("inventory overflow")?,
                );
            }
        }
        if cumulative[0].len() as u64 != pods + 1 {
            return Err("incomplete persisted inventory".into());
        }
        Ok(Self {
            cumulative,
            pods,
            active,
        })
    }
    fn total(&self, service: Service, active: bool) -> u64 {
        self.cumulative[service_index(service)]
            [if active { self.active } else { self.pods } as usize]
    }
    fn count(&self, service: Service, pod: u64) -> u64 {
        let values = &self.cumulative[service_index(service)];
        values[pod as usize + 1] - values[pod as usize]
    }
    fn choose(&self, service: Service, active: bool, draw: u64) -> Result<u64> {
        let total = self.total(service, active);
        if total == 0 {
            return Err("empty selected service inventory".into());
        }
        let target = draw % total;
        let values = &self.cumulative[service_index(service)];
        Ok((values.partition_point(|v| *v <= target) - 1) as u64)
    }
    fn manifest(&self) -> Value {
        json!({"pods":self.pods,"active_pods":self.active,"realized_active_fraction":self.active as f64/self.pods as f64,
          "retained_records":Service::ALL.iter().map(|s|(s.name(),self.total(*s,false))).collect::<HashMap<_,_>>(),
          "active_retained_records":Service::ALL.iter().map(|s|(s.name(),self.total(*s,true))).collect::<HashMap<_,_>>(),
          "driver_inventory_bytes":self.cumulative.iter().map(|v|v.capacity()*8).sum::<usize>()})
    }
}

#[derive(Clone)]
enum Kind {
    Query(usize),
    Background,
    Insert(Service, u32, bool),
    Delete(Service, u32),
    Modify(Service, u32),
    Policy,
}
#[derive(Clone)]
struct Channel {
    id: String,
    daily: f64,
    kind: Kind,
}
fn channels(
    workload: &Value,
    config: &PopulationConfig,
    inventory: &Inventory,
) -> Result<Vec<Channel>> {
    let v = &workload["execution_v2"];
    let mut channels = Vec::new();
    for (index, journey) in workload["journeys"]
        .as_array()
        .ok_or("journeys missing")?
        .iter()
        .enumerate()
    {
        let daily = inventory.active as f64
            * number(&journey["actions_per_active_person_day"])?
            * number(&journey["logical_reads_per_action"])?
            * (1.0 - number(&journey["client_cache_hit_fraction"])?)
            * number(&journey["pod_fanout"])?
            / number(&journey["batch_factor"])?;
        let templates = journey["query_templates"]
            .as_array()
            .ok_or("query templates missing")?;
        let weights = templates
            .iter()
            .map(|q| number(&q["weight"]))
            .collect::<Result<Vec<_>>>()?;
        if templates.is_empty() || (weights.iter().sum::<f64>() - 1.0).abs() > 1e-9 {
            return Err("template weights must sum to one".into());
        }
        channels.push(Channel {
            id: journey["id"].as_str().ok_or("journey ID")?.into(),
            daily,
            kind: Kind::Query(index),
        });
    }
    channels.push(Channel {
        id: "background-read".into(),
        daily: inventory.pods as f64 * number(&v["background_read_requests_per_hosted_day"])?,
        kind: Kind::Background,
    });
    let horizon = f64::from(config.history_months) * number(&v["retention_days_per_month"])?;
    if horizon <= 0.0 {
        return Err("positive retention horizon required".into());
    }
    let background = number(&v["background_ingestion_fraction"])?;
    if background > 1.0 {
        return Err("background fraction exceeds one".into());
    }
    for batch in v["mutation_batches"]
        .as_array()
        .ok_or("mutation batches missing")?
    {
        let service: Service = serde_json::from_value(batch["service"].clone())?;
        let records_per_day = inventory.total(service, false) as f64 / horizon;
        let batch_size = |name: &str| -> Result<u32> {
            let n = batch[name].as_u64().ok_or("missing batch count")?;
            if !(1..=8).contains(&n) {
                return Err("journey batch must contain 1..8 records".into());
            }
            Ok(n as u32)
        };
        let insert = batch_size("ingest_records")?;
        let delete = batch_size("expire_records")?;
        let modify = batch_size("modify_records")?;
        for (id, daily, kind) in [
            (
                "ingest-background",
                records_per_day * background / f64::from(insert),
                Kind::Insert(service, insert, false),
            ),
            (
                "ingest-foreground",
                records_per_day * (1.0 - background) / f64::from(insert),
                Kind::Insert(service, insert, true),
            ),
            (
                "expire",
                records_per_day / f64::from(delete),
                Kind::Delete(service, delete),
            ),
            (
                "modify",
                records_per_day * number(&batch["modifications_per_ingested_record"])?
                    / f64::from(modify),
                Kind::Modify(service, modify),
            ),
        ] {
            channels.push(Channel {
                id: format!("{}-{id}", service.name()),
                daily,
                kind,
            });
        }
    }
    channels.push(Channel {
        id: "contacts-modify".into(),
        daily: inventory.total(Service::Contacts, false) as f64
            * number(&v["contacts_modifications_per_retained_record_year"])?
            / number(&v["days_per_year"])?,
        kind: Kind::Modify(Service::Contacts, 1),
    });
    channels.push(Channel {
        id: "policy-attempt".into(),
        daily: inventory.pods as f64 * number(&v["policy_attempts_per_hosted_day"])?,
        kind: Kind::Policy,
    });
    if channels
        .iter()
        .any(|c| !c.daily.is_finite() || c.daily < 0.0)
        || channels.iter().map(|c| c.daily).sum::<f64>() <= 0.0
    {
        return Err("invalid derived channel rates".into());
    }
    Ok(channels)
}
fn read_target(pods: u64, selection: &str, draw: u64) -> u64 {
    if selection == "skew80-20" && pods > 1 {
        let hot = (pods / 5).max(1);
        if !draw.is_multiple_of(5) {
            (draw / 5) % hot
        } else {
            hot + (draw / 5) % (pods - hot)
        }
    } else {
        draw % pods
    }
}

struct Planner {
    inventory: Inventory,
    config: PopulationConfig,
    workload: Value,
    model: String,
    channels: Vec<Channel>,
    expiry: HashMap<(u64, usize), u64>,
    policy: HashMap<u64, u64>,
    seed: u64,
    epoch: u64,
    selection: String,
}
impl Planner {
    fn plan(&mut self, sequence: u64, scheduled: Instant, scheduled_us: u64) -> Result<Outgoing> {
        let draw = |stream| random(self.seed, sequence, stream);
        let channel = self.channels[weighted(
            &self.channels.iter().map(|c| c.daily).collect::<Vec<_>>(),
            draw(21),
        )]
        .clone();
        let v = &self.workload["execution_v2"];
        let mut principal = "owner";
        if matches!(channel.kind, Kind::Query(_)) {
            let weights = [
                number(&v["read_requester_weights"]["owner"])?,
                number(&v["read_requester_weights"]["recipient"])?,
                number(&v["read_requester_weights"]["public"])?,
            ];
            if (weights.iter().sum::<f64>() - 1.0).abs() > 1e-9 {
                return Err("requester weights must sum to one".into());
            }
            principal = ["owner", "recipient", "public"][weighted(&weights, draw(22))];
        }
        let pod = match channel.kind {
            Kind::Query(_) => read_target(
                if principal == "owner" {
                    self.inventory.active
                } else {
                    self.inventory.pods
                },
                &self.selection,
                draw(23),
            ),
            Kind::Insert(service, _, active) => self.inventory.choose(service, active, draw(23))?,
            Kind::Delete(service, _) | Kind::Modify(service, _) => {
                self.inventory.choose(service, false, draw(23))?
            }
            _ => draw(23) % self.inventory.pods,
        };
        let root = population::pod_root(&self.config, pod);
        let mut record = json!({"record_type":"request","sequence":sequence,"pod":pod,"principal":principal,"channel":channel.id,"scheduled_us":scheduled_us});
        let mut observation = None;
        let (suffix, query_text) = match channel.kind {
            Kind::Query(index) => {
                let journey = &self.workload["journeys"][index];
                let templates = journey["query_templates"]
                    .as_array()
                    .ok_or("query templates")?;
                let selected = weighted(
                    &templates
                        .iter()
                        .map(|q| number(&q["weight"]))
                        .collect::<Result<Vec<_>>>()?,
                    draw(24),
                );
                record["operation"] = json!("query");
                record["journey"] = journey["id"].clone();
                record["query_family"] = templates[selected]["family"].clone();
                record["query_id"] = templates[selected]["id"].clone();
                (
                    "sparql",
                    templates[selected]["sparql"]
                        .as_str()
                        .ok_or("query text")?
                        .replace("{root}", &root),
                )
            }
            Kind::Background => {
                record["operation"] = json!("background-read");
                record["query_id"] = v["background_read_template"]["id"].clone();
                (
                    "sparql",
                    v["background_read_template"]["sparql"]
                        .as_str()
                        .ok_or("background query")?
                        .replace("{root}", &root),
                )
            }
            Kind::Policy => {
                let changes = self.policy.entry(pod).or_default();
                let insert = !(*changes).is_multiple_of(2);
                *changes += 1;
                let action = if insert { "INSERT" } else { "DELETE" };
                let (graph, subject, predicate, object) = if self.model == "wac" {
                    (
                        format!("{root}calendar/.acl"),
                        format!("{root}calendar/.acl#reader"),
                        "http://www.w3.org/ns/auth/acl#agent",
                        population::recipient_webid(&self.config, pod, 0),
                    )
                } else {
                    (
                        format!("{root}calendar/.acr"),
                        format!("{root}calendar/.acr#reader-control"),
                        "http://www.w3.org/ns/solid/acp#apply",
                        format!("{root}calendar/.acr#reader-policy"),
                    )
                };
                record["operation"] = json!("policy-attempt");
                record["desired_grant"] = json!(insert);
                record["mutation_id"] = json!(format!("{}-{sequence}", self.epoch));
                observation = Some(json!({"id":format!("{}-{sequence}",self.epoch),"records":[]}));
                ("policy",format!("{action} DATA {{ GRAPH <{graph}> {{ <{subject}> <{predicate}> <{object}> }} }}"))
            }
            kind => {
                let (service, request, operation) = match kind {
                    Kind::Insert(service, count, _) => (
                        service,
                        mutation::PopulationMutationRequest::Insert {
                            batch_id: (self.epoch << 30) | sequence,
                            count,
                        },
                        "ingest",
                    ),
                    Kind::Delete(service, count) => {
                        let offset = self
                            .expiry
                            .entry((pod, service_index(service)))
                            .or_default();
                        let request = mutation::PopulationMutationRequest::Delete {
                            offset: *offset,
                            count,
                        };
                        *offset += u64::from(count);
                        (service, request, "expire")
                    }
                    Kind::Modify(service, count) => {
                        let total = self.inventory.count(service, pod);
                        let half = total / 2;
                        let offset = half + draw(25) % (total - half).max(1);
                        (
                            service,
                            mutation::PopulationMutationRequest::Modify {
                                offset,
                                count,
                                revision: sequence + 1,
                            },
                            "modify",
                        )
                    }
                    _ => unreachable!(),
                };
                let batch = mutation::emit_mutation_batch(&self.config, pod, service, request)?;
                record["operation"] = json!(operation);
                record["service"] = json!(service);
                record["mutation_request"] = json!(batch.request);
                record["planned_records"] = json!(batch.record_refs.len());
                record["expected_inserted_triples"] = json!(batch.expected_inserted_triples);
                record["expected_deleted_triples"] = json!(batch.expected_deleted_triples);
                let id = format!("{}-{sequence}", self.epoch);
                record["mutation_id"] = json!(id);
                observation = Some(
                    json!({"id":id,"records":batch.record_refs.iter().map(|r|json!([r.graph,r.subject])).collect::<Vec<_>>()}),
                );
                ("update", batch.sparql)
            }
        };
        record["request_bytes"] = json!(query_text.len());
        record["query_sha256"] = json!(format!("{:x}", Sha256::digest(query_text.as_bytes())));
        let webid = match principal {
            "public" => None,
            "recipient" => Some(population::recipient_webid(&self.config, pod, 0)),
            _ => Some(population::owner_webid(&self.config, pod)),
        };
        Ok(Outgoing {
            path: format!("/pods/{pod}/{suffix}"),
            query_text,
            webid,
            update: suffix != "sparql",
            observation,
            record,
            scheduled,
        })
    }
}

pub(super) fn query_templates(
    workload: &Value,
    config: &PopulationConfig,
    pod: u64,
) -> Result<Vec<population::PopulationQuery>> {
    let root = population::pod_root(config, pod);
    workload["journeys"]
        .as_array()
        .ok_or("journeys missing")?
        .iter()
        .flat_map(|j| j["query_templates"].as_array().into_iter().flatten())
        .map(|q| {
            Ok(population::PopulationQuery {
                id: q["id"].as_str().ok_or("query ID")?.into(),
                sparql: q["sparql"]
                    .as_str()
                    .ok_or("query text")?
                    .replace("{root}", &root),
            })
        })
        .collect()
}

pub(super) async fn run(settings: Settings) -> Result<()> {
    let corpus = settings.path("corpus", "population-corpus");
    let manifest: Value = serde_json::from_slice(&std::fs::read(corpus.join("manifest.json"))?)?;
    let pods = manifest["pods"].as_u64().ok_or("manifest pods")?;
    if settings.0.contains_key("pods") && settings.number("pods", pods)? != pods {
        return Err("journeys requires the full persisted population; use a separate corpus for a smaller population".into());
    }
    let config: PopulationConfig = serde_json::from_value(manifest["config"].clone())?;
    let workload_bytes =
        std::fs::read(settings.path("workload-file", "bench/ac/million/workload.json"))?;
    let workload: Value = serde_json::from_slice(&workload_bytes)?;
    if workload["schema_version"] != 2 {
        return Err("journeys requires workload schema 2".into());
    }
    let active_fraction = number(&workload["population"]["daily_active_fraction"])?;
    if active_fraction <= 0.0 || active_fraction > 1.0 || pods == 0 {
        return Err("active fraction must be in (0,1] and population positive".into());
    }
    let inventory = Inventory::load(
        &settings,
        pods,
        ((pods as f64 * active_fraction).ceil() as u64)
            .min(pods)
            .max(1),
    )?;
    let channels = channels(&workload, &config, &inventory)?;
    let scenario = settings.text("scenario", "busy-period");
    let multiplier = number(&workload["offered_multiplier_scenarios"][&scenario])?;
    let derived_rate = channels.iter().map(|c| c.daily).sum::<f64>() / 86400.0 * multiplier;
    let rate = settings
        .text("rate", &derived_rate.to_string())
        .parse::<f64>()?;
    let requests = settings.number("requests", 1000)?;
    let seed = settings.number("seed", 2026090601)?;
    let epoch = settings.number("mutation-epoch", seed % (1 << 20))?;
    let max_inflight = settings.number("max-inflight", 256)? as usize;
    let selection = settings.text("selection", "uniform");
    if !["uniform", "skew80-20"].contains(&selection.as_str()) {
        return Err("journeys read selection must be uniform or skew80-20".into());
    }
    if !(rate > 0.0 && rate.is_finite())
        || requests == 0
        || requests >= (1 << 30)
        || epoch >= (1 << 20)
        || max_inflight == 0
    {
        return Err("invalid rate, requests, mutation epoch or max-inflight".into());
    }
    let output = Arc::new(Mutex::new(BufWriter::new(File::create(
        settings.path("out", "population-journeys.jsonl"),
    )?)));
    emit(
        &output,
        json!({"record_type":"load-start","mode":"journeys","settings":settings.0,"workload_sha256":format!("{:x}",Sha256::digest(&workload_bytes)),"corpus_manifest":manifest,"inventory":inventory.manifest(),"channels_daily":channels.iter().map(|c|json!({"id":c.id,"requests":c.daily})).collect::<Vec<_>>(),"derived_rate":derived_rate,"offered_rate":rate,"mutation_epoch":epoch,"arrival":"precomputed Poisson; no retry","read_target_selection":selection,"write_target_selection":"actual retained service inventory weighted; foreground ingestion active cohort","receipt_boundary":workload["execution_v2"]["receipt_boundary"],"unix_seconds":super::auth::now()}),
    )?;
    if settings.text("plan-only", "false") == "true" {
        output.lock().expect("output lock").flush()?;
        return Ok(());
    }
    let credentials = Arc::new(Credentials::load(&settings)?);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(settings.number("timeout-ms", 5000)?))
        .build()?;
    let connect = settings.text("connect", "http://127.0.0.1:3100");
    let mut planner = Planner {
        inventory,
        config,
        workload,
        model: manifest["model"].as_str().ok_or("model")?.into(),
        channels,
        expiry: HashMap::new(),
        policy: HashMap::new(),
        seed,
        epoch,
        selection,
    };
    let mut seconds = 0.0;
    let minimum_seconds = settings.number("duration-seconds", 0)? as f64;
    let mut schedule = Vec::new();
    while schedule.len() < requests as usize || seconds < minimum_seconds {
        let seq = schedule.len() as u64;
        if seq >= (1 << 30) {
            return Err("duration would overflow mutation sequence space".into());
        }
        seconds += -unit(random(seed, seq, 17)).ln() / rate;
        schedule.push(Duration::from_secs_f64(seconds));
    }
    let requests = schedule.len() as u64;
    let start = Instant::now();
    let mut running = JoinSet::new();
    let mut dropped = 0;
    let mut exhausted = 0;
    for (sequence, offset) in schedule.into_iter().enumerate() {
        while let Some(result) = running.try_join_next() {
            result??;
        }
        let scheduled = start + offset;
        tokio::time::sleep_until(tokio::time::Instant::from_std(scheduled)).await;
        let outgoing = planner.plan(sequence as u64, scheduled, offset.as_micros() as u64)?;
        if outgoing.query_text.is_empty() || running.len() >= max_inflight {
            let mut record = outgoing.record;
            record["status"] = Value::Null;
            record["outcome"] = json!(if outgoing.query_text.is_empty() {
                exhausted += 1;
                "client-plan-exhausted"
            } else {
                dropped += 1;
                "client-admission-drop"
            });
            record["scheduled_latency_us"] = json!(scheduled.elapsed().as_micros() as u64);
            emit(&output, record)?;
            continue;
        }
        let client = client.clone();
        let credentials = Arc::clone(&credentials);
        let connect = connect.clone();
        let output = Arc::clone(&output);
        running.spawn(async move {
            let mut record = send(&client, &credentials, &connect, outgoing).await?;
            if record["status"] == 200 && record.get("planned_records").is_some() {
                let receipt = &record["mutation_receipt"];
                let present = receipt.is_object();
                let insert_matches =
                    receipt["inserted_triples"] == record["expected_inserted_triples"];
                let delete_matches =
                    receipt["deleted_triples"] == record["expected_deleted_triples"];
                record["mutation_receipt_present"] = json!(present);
                record["mutation_matches_plan"] =
                    json!(present && insert_matches && delete_matches);
            }
            emit(&output, record)
        });
    }
    while let Some(result) = running.join_next().await {
        result??;
    }
    emit(
        &output,
        json!({"record_type":"load-complete","offered":requests,"client_dropped":dropped,"plan_exhausted":exhausted,"elapsed_us":start.elapsed().as_micros() as u64}),
    )?;
    output.lock().expect("output lock").flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearby_replicate_seeds_are_not_shifted_counters() {
        for stream in [17, 21, 22, 23, 24, 25] {
            assert_ne!(
                (0..32)
                    .map(|i| random(2026090611, i + 12, stream))
                    .collect::<Vec<_>>(),
                (0..32)
                    .map(|i| random(2026090623, i, stream))
                    .collect::<Vec<_>>()
            );
            assert_eq!(random(2026090611, 7, stream), random(2026090611, 7, stream));
        }
    }
    #[test]
    fn weighted_inventory_and_stationary_rate() {
        let config = PopulationConfig::smoke();
        let mut inventory = Inventory {
            cumulative: std::array::from_fn(|_| vec![0]),
            pods: 4,
            active: 2,
        };
        for pod in 0..4 {
            let counts = population::planned_record_counts(&config, pod).expect("counts");
            for service in Service::ALL {
                let values = &mut inventory.cumulative[service_index(service)];
                values.push(values.last().unwrap() + counts[&service]);
            }
        }
        let workload: Value =
            serde_json::from_str(include_str!("../../../../bench/ac/million/workload.json"))
                .expect("workload");
        let channels = channels(&workload, &config, &inventory).expect("channels");
        for service in Service::ALL {
            if service == Service::Contacts {
                continue;
            }
            let mut ingested = 0.0;
            let mut expired = 0.0;
            for c in &channels {
                match c.kind {
                    Kind::Insert(s, n, _) if s == service => ingested += c.daily * f64::from(n),
                    Kind::Delete(s, n) if s == service => expired += c.daily * f64::from(n),
                    _ => {}
                }
            }
            assert!((ingested - expired).abs() < 1e-9);
            assert!((ingested - inventory.total(service, false) as f64 / 60.0).abs() < 1e-9);
        }
        for service in Service::ALL {
            for draw in 0..inventory.total(service, true) {
                assert!(inventory.choose(service, true, draw).unwrap() < 2);
            }
        }
    }
    #[test]
    fn journey_templates_match_neutral_results_and_nonempty_domains() -> Result<()> {
        let config = PopulationConfig::smoke();
        let workload: Value =
            serde_json::from_str(include_str!("../../../../bench/ac/million/workload.json"))?;
        for model in [population::PolicyModel::Wac, population::PolicyModel::Acp] {
            let mut source = Vec::new();
            population::write_pod(&config, 0, model, &mut source)?;
            let graph = sparq_core::Graph::load_dataset(std::str::from_utf8(&source)?, "nquads")?;
            let mut store = sparq_solid::PodStore::new(graph);
            match model {
                population::PolicyModel::Wac => store.materialize_wac()?,
                population::PolicyModel::Acp => store.materialize_acp()?,
            };
            let owner = population::owner_webid(&config, 0);
            let recipient = population::recipient_webid(&config, 0, 0);
            for agent in [
                Some(owner.as_str()),
                Some(recipient.as_str()),
                Some("https://outsider.example/#me"),
                None,
            ] {
                let mut source = Vec::new();
                population::write_readable_content(&config, 0, agent, &mut source)?;
                let reference =
                    sparq_core::Graph::load_dataset(std::str::from_utf8(&source)?, "nquads")?;
                for query in query_templates(&workload, &config, 0)? {
                    let actual: Value = serde_json::from_str(&store.query_json_as(
                        &sparq_solid::Session {
                            agent,
                            ..Default::default()
                        },
                        sparq_solid::Mode::Read,
                        &query.sparql,
                    )?)?;
                    let expected: Value = serde_json::from_str(&sparq_engine::query_json(
                        &reference,
                        &query.sparql,
                    )?)?;
                    assert_eq!(actual, expected, "{} {agent:?}", query.id);
                    if agent == Some(owner.as_str()) && query.id != "calendar-not-exists" {
                        assert!(
                            !actual["results"]["bindings"].as_array().unwrap().is_empty(),
                            "empty owner {}",
                            query.id
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

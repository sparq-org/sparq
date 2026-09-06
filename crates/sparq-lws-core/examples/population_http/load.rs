// [GPT-6] Scheduled arrivals include client dispatch delay and complete body receipt.
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sparq_acbench::population::{self, PopulationConfig};
use tokio::task::JoinSet;

use super::auth::Credentials;
use super::{Result, Settings};

pub(super) fn emit(output: &Mutex<BufWriter<File>>, value: Value) -> Result<()> {
    let mut output = output.lock().expect("output lock poisoned");
    serde_json::to_writer(&mut *output, &value)?;
    output.write_all(b"\n")?;
    Ok(())
}

pub(super) fn random(seed: u64, sequence: u64, stream: u64) -> u64 {
    let mut value = sequence
        .wrapping_add(seed)
        .wrapping_add(stream.wrapping_mul(0x9e3779b97f4a7c15));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

pub(super) fn unit(value: u64) -> f64 {
    ((value >> 11) as f64 + 0.5) / ((1_u64 << 53) as f64)
}

fn workload_weights(settings: &Settings) -> Result<[f64; 4]> {
    if settings.text("mix", "reads") == "reads" {
        return Ok([1.0, 0.0, 0.0, 0.0]);
    }
    if settings.text("mix", "reads") != "population" {
        return Err("mix must be reads or population".into());
    }
    let workload: Value = serde_json::from_slice(&std::fs::read(
        settings.path("workload-file", "bench/ac/million/workload.json"),
    )?)?;
    let n = |value: &Value| -> Result<f64> {
        value
            .as_f64()
            .ok_or_else(|| "workload number missing".into())
    };
    let active = n(&workload["population"]["daily_active_fraction"])?;
    let mut query = 0.0;
    for journey in workload["journeys"]
        .as_array()
        .ok_or("workload journeys missing")?
    {
        query += active
            * n(&journey["actions_per_active_person_day"])?
            * n(&journey["logical_reads_per_action"])?
            * (1.0 - n(&journey["client_cache_hit_fraction"])?)
            * n(&journey["pod_fanout"])?
            / n(&journey["batch_factor"])?;
    }
    let background = &workload["background"];
    let weights = [
        query,
        n(&background["read_requests_per_hosted_person_day"])?,
        active * n(&background["content_write_requests_per_active_person_day"])?,
        n(&background["policy_changes_per_hosted_person_day"])?,
    ];
    if weights.iter().any(|v| !v.is_finite() || *v < 0.0) || weights.iter().sum::<f64>() <= 0.0 {
        return Err("invalid workload weights".into());
    }
    Ok(weights)
}

pub(super) async fn run(settings: Settings) -> Result<()> {
    if settings.text("mix", "reads") == "journeys" {
        return super::journeys::run(settings).await;
    }
    let credentials = Arc::new(Credentials::load(&settings)?);
    let requests = settings.number("requests", 1000)?;
    let rate: f64 = settings.text("rate", "10").parse()?;
    let pods = settings.number("pods", 1)?;
    let maximum_inflight = usize::try_from(settings.number("max-inflight", 256)?)?;
    if !rate.is_finite() || rate <= 0.0 || requests == 0 || pods == 0 || maximum_inflight == 0 {
        return Err("rate, requests, Pods and max-inflight must be positive".into());
    }
    let output_path = settings.path("out", "population-http.jsonl");
    let output = Arc::new(Mutex::new(BufWriter::new(File::create(output_path)?)));
    let query = Arc::new(if settings.0.contains_key("query-file") {
        std::fs::read_to_string(settings.path("query-file", ""))?
    } else {
        "SELECT (COUNT(?s) AS ?count) WHERE { GRAPH ?g { ?s a <https://sparq.dev/bench/personal#Record> } }".into()
    });
    let connect = settings.text("connect", "http://127.0.0.1:3100");
    let selection = settings.text("selection", "sequential");
    if !["sequential", "uniform", "hot", "skew80-20"].contains(&selection.as_str()) {
        return Err("selection must be sequential, uniform, hot or skew80-20".into());
    }
    let principal = settings.text("principal", "owner");
    if !["owner", "recipient", "outsider", "public"].contains(&principal.as_str()) {
        return Err("principal must be owner, recipient, outsider or public".into());
    }
    let seed = settings.number("seed", 1)?;
    let weights = workload_weights(&settings)?;
    let population_queries = settings.text("query-set", "count") == "population";
    let population = if population_queries || weights[2] > 0.0 || weights[3] > 0.0 {
        let manifest: Value = serde_json::from_slice(&std::fs::read(
            settings
                .path("corpus", "population-corpus")
                .join("manifest.json"),
        )?)?;
        let config: PopulationConfig = serde_json::from_value(manifest["config"].clone())?;
        Some((
            Arc::new(config),
            manifest["model"]
                .as_str()
                .ok_or("model missing")?
                .to_owned(),
        ))
    } else {
        None
    };
    let arrival = settings.text("arrival", "constant");
    if !["constant", "poisson"].contains(&arrival.as_str()) {
        return Err("arrival must be constant or poisson".into());
    }
    // Freeze the full arrival clock before sending, independent of server progress.
    let mut accumulated = 0.0;
    let schedule: Vec<Duration> = (0..requests)
        .map(|sequence| {
            if arrival == "poisson" {
                accumulated += -unit(random(seed, sequence, 17)).ln() / rate;
            } else {
                accumulated = sequence as f64 / rate;
            }
            Duration::from_secs_f64(accumulated)
        })
        .collect();
    let timeout = Duration::from_millis(settings.number("timeout-ms", 30_000)?);
    let client = reqwest::Client::builder().timeout(timeout).build()?;
    emit(
        &output,
        json!({"record_type":"load-start", "settings":settings.0,
        "arrival_model":format!("{arrival} scheduled open-loop"), "latency_origin":"scheduled arrival",
        "operation_weights":weights,"query_selection":"uniform across declared population query templates when selected",
        "server_timer":"request admitted through response body serialization; socket transfer measured by client",
        "unix_seconds":super::auth::now()}),
    )?;
    let start = Instant::now();
    let mut running = JoinSet::new();
    let mut dropped = 0_u64;
    let mut policy_changes = std::collections::HashMap::<u64, u64>::new();
    for sequence in 0..requests {
        while let Some(result) = running.try_join_next() {
            result??;
        }
        let scheduled = start + schedule[sequence as usize];
        tokio::time::sleep_until(tokio::time::Instant::from_std(scheduled)).await;
        let random = random(seed, sequence, 1);
        let pod = match selection.as_str() {
            "uniform" => random % pods,
            // Explicit stress profile: 90% of arrivals address the first 1% of Pods.
            "hot" if !random.is_multiple_of(10) => (random / 10) % (pods / 100).max(1),
            "hot" => (random / 10) % pods,
            "skew80-20" if !random.is_multiple_of(5) => (random / 5) % (pods / 5).max(1),
            "skew80-20" => (pods / 5).max(1) + (random / 5) % (pods - (pods / 5).max(1)).max(1),
            _ => sequence % pods,
        } % pods;
        let choice = unit(self::random(seed, sequence, 2)) * weights.iter().sum::<f64>();
        let mut cumulative = 0.0;
        let operation = weights
            .iter()
            .position(|weight| {
                cumulative += weight;
                choice < cumulative
            })
            .unwrap_or(3);
        let mut query_text = query.replace("{pod}", &pod.to_string());
        let mut query_id = "custom-count".to_owned();
        if let Some((config, model)) = &population {
            let root = population::pod_root(config, pod);
            if operation <= 1 && population_queries {
                let queries = population::benchmark_queries(config, pod);
                let selected = if operation == 1 {
                    0
                } else {
                    (self::random(seed, sequence, 3) % queries.len() as u64) as usize
                };
                query_text = queries[selected].sparql.clone();
                query_id = queries[selected].id.clone();
            } else if operation == 2 {
                let graph = format!("{root}communication/m0000.ttl");
                query_text = format!("DELETE {{ GRAPH <{graph}> {{ <{graph}#r0> <https://sparq.dev/bench/personal#value> ?old }} }} INSERT {{ GRAPH <{graph}> {{ <{graph}#r0> <https://sparq.dev/bench/personal#value> {sequence} }} }} WHERE {{ OPTIONAL {{ GRAPH <{graph}> {{ <{graph}#r0> <https://sparq.dev/bench/personal#value> ?old }} }} }}");
                query_id = "replace-one-record-value".into();
            } else if operation == 3 {
                let changes = policy_changes.entry(pod).or_default();
                let action = if (*changes).is_multiple_of(2) {
                    "DELETE"
                } else {
                    "INSERT"
                };
                *changes += 1;
                let recipient = population::recipient_webid(config, pod, 0);
                let (graph, subject, predicate, object) = if model == "wac" {
                    (
                        format!("{root}calendar/.acl"),
                        format!("{root}calendar/.acl#reader"),
                        "http://www.w3.org/ns/auth/acl#agent",
                        recipient,
                    )
                } else {
                    (
                        format!("{root}calendar/.acr"),
                        format!("{root}calendar/.acr#reader-control"),
                        "http://www.w3.org/ns/solid/acp#apply",
                        format!("{root}calendar/.acr#reader-policy"),
                    )
                };
                query_text = format!("{action} DATA {{ GRAPH <{graph}> {{ <{subject}> <{predicate}> <{object}> }} }}");
                query_id = format!("{action}-calendar-recipient");
            }
        }
        if running.len() >= maximum_inflight {
            dropped += 1;
            emit(
                &output,
                json!({"record_type":"request", "sequence":sequence,
                "pod":pod,"status":null,"outcome":"client-admission-drop",
                "scheduled_us":scheduled.duration_since(start).as_micros() as u64,
                "scheduled_latency_us":scheduled.elapsed().as_micros() as u64}),
            )?;
            continue;
        }
        let client = client.clone();
        let output = Arc::clone(&output);
        let credentials = Arc::clone(&credentials);
        let connect = connect.clone();
        let principal = if operation >= 2 {
            "owner".to_owned()
        } else {
            principal.clone()
        };
        let population = population.as_ref().map(|(config, _)| Arc::clone(config));
        let owner_template = settings.text(
            "owner-template",
            "https://pods.example/p/{pod}/profile/card#me",
        );
        let recipient_template = settings.text(
            "recipient-template",
            "https://pods.example/p/{pod}/people/0#me",
        );
        running.spawn(async move {
            let suffix = if operation == 3 { "policy" } else if operation == 2 { "update" } else { "sparql" };
            let path = format!("/pods/{pod}/{suffix}");
            let webid = match (principal.as_str(), population.as_ref()) {
                ("public", _) => None,
                ("owner", Some(config)) => Some(population::owner_webid(config,pod)),
                ("recipient", Some(config)) => Some(population::recipient_webid(config,pod,0)),
                ("recipient", _) => Some(recipient_template.replace("{pod}", &pod.to_string())),
                ("outsider", _) => Some("https://outsider.benchmark.example/profile#me".into()),
                _ => Some(owner_template.replace("{pod}", &pod.to_string())),
            };
            let record = json!({"record_type":"request", "sequence":sequence,
                "pod":pod, "scheduled_us":scheduled.duration_since(start).as_micros() as u64,
                "operation":(["query","background-read","content-write","policy-write"][operation]),"query_id":query_id});
            let record = send(&client, &credentials, &connect, Outgoing {path, query_text, webid, update:operation>=2, observation:None, record, scheduled}).await?;
            emit(&output, record)
        });
    }
    while let Some(result) = running.join_next().await {
        result??;
    }
    emit(
        &output,
        json!({"record_type":"load-complete", "offered":requests,
        "client_dropped":dropped,"elapsed_us":start.elapsed().as_micros() as u64}),
    )?;
    output.lock().expect("output lock poisoned").flush()?;
    Ok(())
}

// Shared request transport: both pilot and journey schedules retain all offered outcomes.
pub(super) struct Outgoing {
    pub(super) path: String,
    pub(super) query_text: String,
    pub(super) webid: Option<String>,
    pub(super) update: bool,
    pub(super) observation: Option<Value>,
    pub(super) record: Value,
    pub(super) scheduled: Instant,
}

pub(super) async fn send(
    client: &reqwest::Client,
    credentials: &Credentials,
    connect: &str,
    outgoing: Outgoing,
) -> Result<Value> {
    let Outgoing {
        path,
        query_text,
        webid,
        update,
        observation,
        mut record,
        scheduled,
    } = outgoing;
    let prepare = Instant::now();
    let mut request = client
        .post(format!("{connect}{path}"))
        .header(
            "content-type",
            if update {
                "application/sparql-update"
            } else {
                "application/sparql-query"
            },
        )
        .body(query_text);
    if let Some(webid) = webid {
        let (access, proof) = credentials.headers(&webid, &path);
        request = request
            .header("authorization", access)
            .header("dpop", proof);
    }
    if let Some(observation) = observation {
        request = request.header("x-benchmark-records", serde_json::to_string(&observation)?);
    }
    let sent = Instant::now();
    let result = request.send().await;
    record["dispatch_lag_us"] = json!(prepare.duration_since(scheduled).as_micros() as u64);
    record["credential_preparation_us"] = json!(sent.duration_since(prepare).as_micros() as u64);
    match result {
        Ok(response) => {
            record["status"] = json!(response.status().as_u16());
            if let Some(delta) = response
                .headers()
                .get("x-policy-triple-delta")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
            {
                record["policy_triple_delta"] = json!(delta);
            }
            for name in [
                "queue",
                "auth",
                "operation",
                "load",
                "materialize",
                "server",
            ] {
                if let Some(value) = response
                    .headers()
                    .get(format!("x-{name}-us"))
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    record[format!("{name}_us")] = json!(value);
                }
            }
            for name in ["cache-hit", "cache-entries", "cache-source-bytes"] {
                if let Some(value) = response
                    .headers()
                    .get(format!("x-{name}"))
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    record[name.replace('-', "_")] = json!(value);
                }
            }
            match response.bytes().await {
                Ok(bytes) => {
                    record["response_bytes"] = json!(bytes.len());
                    record["outcome"] = json!(if record["status"] == 200 {
                        "ok"
                    } else {
                        "http-error"
                    });
                    if record["status"] != 200 {
                        record["error"] = json!(String::from_utf8_lossy(&bytes)
                            .chars()
                            .take(512)
                            .collect::<String>());
                    }
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                        if let Some(receipt) = value.get("mutation_receipt") {
                            record["mutation_receipt"] = receipt.clone();
                        }
                        if let Some(count) = value.pointer("/results/bindings/0/count/value") {
                            record["count"] = count.clone();
                        }
                    }
                }
                Err(error) => {
                    record["outcome"] = json!("body-error");
                    record["error"] = json!(format!("{error:?}"));
                    record["is_timeout"] = json!(error.is_timeout());
                    record["commit_status"] = json!(if update {
                        "unknown-until-journal-reconciliation"
                    } else {
                        "not-a-mutation"
                    });
                }
            }
        }
        Err(error) => {
            record["status"] = Value::Null;
            record["outcome"] = json!("transport-error");
            record["error"] = json!(format!("{error:?}"));
            record["is_timeout"] = json!(error.is_timeout());
            record["is_connect"] = json!(error.is_connect());
            record["commit_status"] = json!(if update {
                "unknown-until-journal-reconciliation"
            } else {
                "not-a-mutation"
            });
        }
    }
    record["http_latency_us"] = json!(sent.elapsed().as_micros() as u64);
    record["scheduled_latency_us"] = json!(scheduled.elapsed().as_micros() as u64);
    Ok(record)
}

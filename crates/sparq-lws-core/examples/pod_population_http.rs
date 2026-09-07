// [GPT-6] Research HTTP path for persisted, independently authorized Pods.
//! Run a cached or fully preloaded Pod query experiment with real token verification.
//!
//! This example is not the native LDP server: it isolates Pod-local SPARQL and
//! durable update journals. Its pinned benchmark issuer omits live OIDC discovery.

#[path = "population_http/auth.rs"]
mod auth;
#[path = "population_http/churn.rs"]
mod churn;
#[path = "population_http/journeys.rs"]
mod journeys;
#[path = "population_http/load.rs"]
mod load;
#[path = "population_http/storage.rs"]
mod storage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Router};
use serde_json::json;
use tokio::sync::{oneshot, RwLock};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Failure = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Failure>;

#[derive(Clone)]
struct Settings(HashMap<String, String>);

impl Settings {
    fn parse() -> Result<Self> {
        let mut values = HashMap::new();
        let mut args = std::env::args().skip(1);
        values.insert("mode".into(), args.next().unwrap_or_else(|| "help".into()));
        while let Some(key) = args.next() {
            if !key.starts_with("--") {
                return Err(format!("expected --name, got {key}").into());
            }
            let value = args.next().ok_or("missing argument value")?;
            if values.insert(key[2..].to_owned(), value).is_some() {
                return Err(format!("duplicate {key}").into());
            }
        }
        Ok(Self(values))
    }

    fn text(&self, key: &str, default: &str) -> String {
        self.0.get(key).map_or_else(|| default.into(), Clone::clone)
    }

    fn number(&self, key: &str, default: u64) -> Result<u64> {
        self.0.get(key).map_or(Ok(default), |v| Ok(v.parse()?))
    }

    fn path(&self, key: &str, default: &str) -> PathBuf {
        self.text(key, default).into()
    }
}

struct Work {
    pod: u64,
    path: String,
    headers: HeaderMap,
    body: String,
    update: bool,
    policy_administration: bool,
    received: Instant,
    reply: oneshot::Sender<Response>,
}

struct HttpState {
    workers: Vec<mpsc::SyncSender<WorkerCommand>>,
    admission: RwLock<()>,
    control_token: Option<String>,
}

// [GPT-6] FIFO fences include work whose HTTP receiver has timed out or closed.
enum WorkerCommand {
    Request(Work),
    Fence(oneshot::Sender<serde_json::Value>),
}

fn control_token(settings: &Settings) -> Result<Option<String>> {
    if !settings.0.contains_key("control-token-file") {
        return Ok(None);
    }
    let path = settings.path("control-token-file", "");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&path)?.permissions().mode() & 0o077 != 0 {
            return Err("control token file must be readable only by its owner".into());
        }
    }
    let value = std::fs::read_to_string(path)?.trim().to_owned();
    if !(32..=256).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("control token must be 32..256 random alphanumeric bytes".into());
    }
    Ok(Some(value))
}

async fn drain(State(state): State<Arc<HttpState>>, headers: HeaderMap) -> Response {
    let Some(expected) = &state.control_token else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get("x-benchmark-control")
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    // Fixed-size comparison for this independent operator capability; no caller
    // WebID can grant itself permission to stop admission across other Pods.
    let equal = supplied.len() == expected.len()
        && supplied
            .iter()
            .zip(expected.as_bytes())
            .fold(0_u8, |v, (a, b)| v | (a ^ b))
            == 0;
    if !equal {
        return StatusCode::FORBIDDEN.into_response();
    }
    let start = Instant::now();
    let _exclusive = state.admission.write().await;
    let workers = state.workers.clone();
    let fences = tokio::task::spawn_blocking(
        move || -> Result<Vec<oneshot::Receiver<serde_json::Value>>> {
            let mut responses = Vec::new();
            for worker in workers {
                let (tx, rx) = oneshot::channel();
                worker
                    .send(WorkerCommand::Fence(tx))
                    .map_err(|_| "worker unavailable at drain")?;
                responses.push(rx);
            }
            Ok(responses)
        },
    )
    .await;
    let receivers = match fences {
        Ok(Ok(receivers)) => receivers,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker fence admission failed",
            )
                .into_response()
        }
    };
    let mut reports = Vec::new();
    for receiver in receivers {
        match receiver.await {
            Ok(report) => reports.push(report),
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "worker fence acknowledgement missing",
                )
                    .into_response()
            }
        }
    }
    let valid = reports.iter().all(|r: &serde_json::Value| {
        r["state"]["poisoned"] == false
            && r["state"]["activations"]["attempted_activations_after_ready"] == 0
    });
    let body = json!({"record_type":"worker-drain-complete","passed":valid,
        "elapsed_us":start.elapsed().as_micros(),"workers":reports,
        "scope":"previously admitted HTTP handlers completed body extraction and handling; all worker operations finished before their FIFO fences; never-admitted transport connections are outside this barrier"});
    (
        if valid {
            StatusCode::OK
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        },
        axum::Json(body),
    )
        .into_response()
}

async fn request_drain(settings: &Settings) -> Result<()> {
    let token = control_token(settings)?.ok_or("drain requires control-token-file")?;
    let connect = settings.text("connect", "http://127.0.0.1:3100");
    let response = reqwest::Client::new()
        .post(format!(
            "{}/__benchmark/drain",
            connect.trim_end_matches('/')
        ))
        .header("x-benchmark-control", token)
        .timeout(std::time::Duration::from_secs(
            settings.number("drain-timeout-seconds", 600)?,
        ))
        .send()
        .await?;
    let status = response.status();
    let body: serde_json::Value = response.json().await?;
    println!("{body}");
    if !status.is_success()
        || body["record_type"] != "worker-drain-complete"
        || body["passed"] != true
    {
        return Err("worker drain did not establish a valid barrier".into());
    }
    Ok(())
}

async fn query(
    State(state): State<Arc<HttpState>>,
    Path(pod): Path<u64>,
    Extension(received): Extension<Instant>,
    headers: HeaderMap,
    body: String,
) -> Response {
    dispatch(state, pod, headers, body, false, false, received).await
}

async fn update(
    State(state): State<Arc<HttpState>>,
    Path(pod): Path<u64>,
    Extension(received): Extension<Instant>,
    headers: HeaderMap,
    body: String,
) -> Response {
    dispatch(state, pod, headers, body, true, false, received).await
}

async fn policy(
    State(state): State<Arc<HttpState>>,
    Path(pod): Path<u64>,
    Extension(received): Extension<Instant>,
    headers: HeaderMap,
    body: String,
) -> Response {
    dispatch(state, pod, headers, body, true, true, received).await
}

async fn dispatch(
    state: Arc<HttpState>,
    pod: u64,
    headers: HeaderMap,
    body: String,
    update: bool,
    policy_administration: bool,
    received: Instant,
) -> Response {
    let suffix = if policy_administration {
        "policy"
    } else if update {
        "update"
    } else {
        "sparql"
    };
    let (reply, response) = oneshot::channel();
    let work = Work {
        pod,
        path: format!("/pods/{pod}/{suffix}"),
        headers,
        body,
        update,
        policy_administration,
        received,
        reply,
    };
    let worker = &state.workers[(pod % state.workers.len() as u64) as usize];
    if worker.try_send(WorkerCommand::Request(work)).is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "bounded worker queue full").into_response();
    }
    response.await.unwrap_or_else(|_| {
        (StatusCode::INTERNAL_SERVER_ERROR, "worker unavailable").into_response()
    })
}

async fn mark_received(
    State(state): State<Arc<HttpState>>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(Instant::now());
    // [GPT-6] Track routed requests before body extraction. A slow warmup body
    // must finish or fail before a drain can fence workers. The independently
    // authenticated control route cannot take the read lock it needs to drain.
    let _admitted = if request.uri().path() == "/__benchmark/drain" {
        None
    } else {
        Some(state.admission.read().await)
    };
    next.run(request).await
}

fn worker_loop(
    input: mpsc::Receiver<WorkerCommand>,
    mut pods: storage::PodCache,
    auth: Arc<auth::Authentication>,
) {
    let mut processed = 0_u64;
    while let Ok(command) = input.recv() {
        let work = match command {
            WorkerCommand::Request(work) => work,
            WorkerCommand::Fence(reply) => {
                let _ = reply.send(json!({"processed_requests":processed,"state":pods.state()}));
                continue;
            }
        };
        let queue_us = work.received.elapsed().as_micros() as u64;
        let start = Instant::now();
        let token = auth.authenticate(&work.headers, &work.path);
        let auth_us = start.elapsed().as_micros() as u64;
        let response = match token {
            Err(error) => error.into_response(),
            Ok(token) => {
                let start = Instant::now();
                match pods.execute_observed(
                    work.pod,
                    &token,
                    &work.body,
                    work.update,
                    work.policy_administration,
                    work.headers
                        .get("x-benchmark-records")
                        .and_then(|v| v.to_str().ok()),
                ) {
                    Ok(outcome) => {
                        let operation_us = start.elapsed().as_micros() as u64;
                        let mut response = (
                            StatusCode::OK,
                            [("content-type", "application/sparql-results+json")],
                            outcome.body,
                        )
                            .into_response();
                        for (name, value) in [
                            ("x-queue-us", queue_us),
                            ("x-auth-us", auth_us),
                            ("x-operation-us", operation_us),
                            ("x-load-us", outcome.load_us),
                            ("x-materialize-us", outcome.materialize_us),
                            ("x-cache-hit", u64::from(outcome.cache_hit)),
                            ("x-cache-entries", outcome.cache_entries as u64),
                            ("x-cache-source-bytes", outcome.cache_bytes),
                            ("x-server-us", work.received.elapsed().as_micros() as u64),
                        ] {
                            if let Ok(value) = value.to_string().parse() {
                                response.headers_mut().insert(name, value);
                            }
                        }
                        if let Ok(value) = outcome.policy_triple_delta.to_string().parse() {
                            response
                                .headers_mut()
                                .insert("x-policy-triple-delta", value);
                        }
                        response
                    }
                    Err(error) => (error.status, error.message).into_response(),
                }
            }
        };
        let _ = work.reply.send(response);
        processed += 1;
    }
}

async fn serve(settings: Settings) -> Result<()> {
    let workers = usize::try_from(settings.number("workers", 1)?)?;
    if workers == 0 || workers > 64 {
        return Err("workers must be between 1 and 64".into());
    }
    let archive_verification = storage::validate_native(&settings)?;
    if let Some(record) = &archive_verification {
        println!("{record}");
    }
    let auth = Arc::new(auth::Authentication::load(&settings)?);
    let control_token = control_token(&settings)?;
    let preloaded = settings.text("storage-mode", "cached") != "cached";
    if preloaded && control_token.is_none() {
        return Err(
            "preloaded evaluation requires an independent control-token-file for draining".into(),
        );
    }
    let mut senders = Vec::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    for worker in 0..workers {
        let capacity = usize::try_from(settings.number("queue-capacity", 256)?)?;
        let (tx, rx) = mpsc::sync_channel(capacity);
        let auth = Arc::clone(&auth);
        let settings = settings.clone();
        let ready = ready_tx.clone();
        std::thread::Builder::new()
            .name(format!("pod-worker-{worker}"))
            .spawn(move || {
                let initialize = || -> Result<(storage::PodCache, serde_json::Value)> {
                    let mut cache = storage::PodCache::open(&settings, workers)?;
                    let report = cache.preload(worker, workers)?;
                    Ok((cache, report))
                };
                match initialize() {
                    Ok((cache, report)) => {
                        if ready.send(Ok(report)).is_ok() {
                            worker_loop(rx, cache, auth);
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                    }
                }
            })?;
        senders.push(tx);
    }
    drop(ready_tx);
    let startup = tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>> {
        let mut reports = Vec::new();
        for result in ready_rx {
            reports.push(result.map_err(|e| format!("worker preload failed: {e}"))?);
            if reports.len() == workers {
                break;
            }
        }
        if reports.len() != workers {
            return Err("population initialization worker missing".into());
        }
        Ok(reports)
    })
    .await??;
    if preloaded {
        let population = startup
            .first()
            .and_then(|r| r["state"]["population"].as_u64())
            .ok_or("ready population missing")?;
        let retained: u64 = startup
            .iter()
            .map(|r| r["assigned_pods"].as_u64().unwrap_or(0))
            .sum();
        if retained != population
            || !startup
                .iter()
                .all(|r| r["state"]["ready"] == true && r["state"]["poisoned"] == false)
        {
            return Err("all-population-ready barrier failed".into());
        }
        println!(
            "{}",
            json!({"record_type":"all-population-ready","population":population,
            "retained_pods":retained,"worker_reports":startup,"settings":settings.0,
            "archive_verification":archive_verification})
        );
    }
    let state = Arc::new(HttpState {
        workers: senders,
        admission: RwLock::new(()),
        control_token,
    });
    let router = Router::new()
        .route("/pods/{pod}/sparql", post(query))
        .route("/pods/{pod}/update", post(update))
        .route("/pods/{pod}/policy", post(policy))
        .route("/__benchmark/drain", post(drain))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            mark_received,
        ))
        .with_state(state);
    let bind = settings.text("bind", "127.0.0.1:3100");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!(
        "{}",
        json!({"record_type":"server-ready", "bind":bind,
        "authentication":"pinned-issuer-es256-dpop", "workers":workers,
        "cache_budget":"encoded-source-bytes, not measured heap bytes",
        "settings":settings.0})
    );
    axum::serve(listener, router).await?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let settings = Settings::parse()?;
    match settings.text("mode", "help").as_str() {
        "serve" => serve(settings).await,
        "auth" => auth::provision(&settings),
        "load" => load::run(settings).await,
        "pack" => storage::pack(&settings),
        "prepare-native" => storage::prepare_native(&settings),
        "drain" => request_drain(&settings).await,
        "verify" => storage::verify(&settings),
        "audit" => storage::audit(&settings),
        "churn" => churn::run(settings).await,
        _ => {
            println!("pod_population_http auth|pack|prepare-native|serve|drain|load|verify|audit|churn --name value\nSee skills/solid-lws-server/SKILL.md for the research-only interface.");
            Ok(())
        }
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;
    use std::time::Duration;
    use tower::ServiceExt;

    #[tokio::test]
    async fn drain_waits_for_body_extraction_before_fencing_workers() -> Result<()> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let secret = "a".repeat(32);
        let state = Arc::new(HttpState {
            workers: vec![sender],
            admission: RwLock::new(()),
            control_token: Some(secret.clone()),
        });
        let router = Router::new()
            .route("/slow", post(|_body: String| async { StatusCode::OK }))
            .route("/__benchmark/drain", post(drain))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                mark_received,
            ))
            .with_state(Arc::clone(&state));
        let (started, wait_started) = oneshot::channel();
        let (release, wait_release) = oneshot::channel();
        let body = axum::body::Body::from_stream(futures_util::stream::once(async move {
            let _ = started.send(());
            wait_release.await.map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"complete"))
        }));
        let request = Request::builder().method("POST").uri("/slow").body(body)?;
        let client = tokio::spawn(router.clone().oneshot(request));
        wait_started.await?;
        let (fenced, mut wait_fenced) = oneshot::channel();
        let worker = std::thread::spawn(move || {
            let WorkerCommand::Fence(reply) = receiver.recv().expect("fence") else {
                panic!("expected fence")
            };
            let _ = fenced.send(());
            let _ = reply.send(json!({"processed_requests":0,"state":{"poisoned":false,
                "activations":{"attempted_activations_after_ready":0}}}));
        });
        let request = Request::builder()
            .method("POST")
            .uri("/__benchmark/drain")
            .header("x-benchmark-control", secret)
            .body(axum::body::Body::empty())?;
        let controller = tokio::spawn(router.oneshot(request));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.admission.try_read().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        assert!(!controller.is_finished());
        assert!(matches!(
            wait_fenced.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).map_err(|_| "body reader disappeared")?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), client)
                .await???
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), controller)
                .await???
                .status(),
            StatusCode::OK
        );
        worker.join().map_err(|_| "fake worker panicked")?;
        Ok(())
    }

    #[tokio::test]
    async fn drain_waits_for_timed_out_work_and_pauses_admission() -> Result<()> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let secret = "a".repeat(32);
        let state = Arc::new(HttpState {
            workers: vec![sender.clone()],
            admission: RwLock::new(()),
            control_token: Some(secret.clone()),
        });
        let (reply, abandoned) = oneshot::channel();
        drop(abandoned);
        sender
            .send(WorkerCommand::Request(Work {
                pod: 0,
                path: String::new(),
                headers: HeaderMap::new(),
                body: String::new(),
                update: false,
                policy_administration: false,
                received: Instant::now(),
                reply,
            }))
            .map_err(|_| "fake worker queue unavailable")?;
        let (started, wait_started) = oneshot::channel();
        let (release, wait_release) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let WorkerCommand::Request(work) = receiver.recv().expect("queued work") else {
                panic!("work must precede fence")
            };
            let _ = started.send(());
            wait_release.recv().expect("release work");
            assert!(work.reply.send(StatusCode::OK.into_response()).is_err());
            let WorkerCommand::Fence(reply) = receiver.recv().expect("queued fence") else {
                panic!("expected fence")
            };
            let _ = reply.send(json!({"processed_requests":1,"state":{"poisoned":false,
                "activations":{"attempted_activations_after_ready":0}}}));
        });
        wait_started.await?;
        let mut headers = HeaderMap::new();
        headers.insert("x-benchmark-control", secret.parse()?);
        let controller = tokio::spawn(drain(State(Arc::clone(&state)), headers));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.admission.try_read().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        assert!(!controller.is_finished());
        release.send(())?;
        let response = tokio::time::timeout(Duration::from_secs(2), controller).await??;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.admission.try_read().is_ok());
        worker.join().map_err(|_| "fake worker panicked")?;
        Ok(())
    }

    #[tokio::test]
    async fn drain_rejects_non_operator_without_enqueuing_a_fence() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let state = Arc::new(HttpState {
            workers: vec![sender],
            admission: RwLock::new(()),
            control_token: Some("a".repeat(32)),
        });
        assert_eq!(
            drain(State(Arc::clone(&state)), HeaderMap::new())
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(state.admission.try_read().is_ok());
    }
}

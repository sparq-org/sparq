// [GPT-6] Research HTTP path for persisted, independently authorized Pods.
//! Run a bounded-cache Pod query experiment with real token and DPoP verification.
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
use tokio::sync::oneshot;

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
    workers: Vec<mpsc::SyncSender<Work>>,
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
    if worker.try_send(work).is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "bounded worker queue full").into_response();
    }
    response.await.unwrap_or_else(|_| {
        (StatusCode::INTERNAL_SERVER_ERROR, "worker unavailable").into_response()
    })
}

async fn mark_received(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(Instant::now());
    next.run(request).await
}

fn worker_loop(
    input: mpsc::Receiver<Work>,
    mut pods: storage::PodCache,
    auth: Arc<auth::Authentication>,
) {
    while let Ok(work) = input.recv() {
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
    }
}

async fn serve(settings: Settings) -> Result<()> {
    let workers = usize::try_from(settings.number("workers", 1)?)?;
    if workers == 0 || workers > 64 {
        return Err("workers must be between 1 and 64".into());
    }
    let auth = Arc::new(auth::Authentication::load(&settings)?);
    let mut senders = Vec::new();
    for worker in 0..workers {
        let cache = storage::PodCache::open(&settings, workers)?;
        let capacity = usize::try_from(settings.number("queue-capacity", 256)?)?;
        let (tx, rx) = mpsc::sync_channel(capacity);
        let auth = Arc::clone(&auth);
        std::thread::Builder::new()
            .name(format!("pod-worker-{worker}"))
            .spawn(move || worker_loop(rx, cache, auth))?;
        senders.push(tx);
    }
    let router = Router::new()
        .route("/pods/{pod}/sparql", post(query))
        .route("/pods/{pod}/update", post(update))
        .route("/pods/{pod}/policy", post(policy))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(mark_received))
        .with_state(Arc::new(HttpState { workers: senders }));
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
        "verify" => storage::verify(&settings),
        "audit" => storage::audit(&settings),
        "churn" => churn::run(settings).await,
        _ => {
            println!("pod_population_http auth|pack|serve|load|verify|audit|churn --name value\nSee skills/solid-lws-server/SKILL.md for the research-only interface.");
            Ok(())
        }
    }
}

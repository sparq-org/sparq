//! Manual P0-9 measurement through the native durable HTTP server.
#![cfg(all(unix, feature = "embedded-sparq", feature = "blob-fs"))]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::process::{Directory, Process};
use reqwest::{Method, StatusCode};
use serde_json::json;
use sparq_lws_core::store::{blob_fs::FsBlobStore, CompositeStore, EmbeddedSparqClient, Store};

struct Fixture {
    directory: Directory,
    base: String,
    client: reqwest::Client,
}

impl Fixture {
    async fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let fixture = Self {
            directory: Directory::new(),
            base: format!("http://{}", listener.local_addr().unwrap()),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        };
        drop(listener);
        std::fs::create_dir(fixture.data()).unwrap();
        // Match common/durable.rs: boot owns format initialization; provision only root + ACL
        // offline. All measured resources go through HTTP with the server's real storage config.
        boot(&fixture).await.0.stop();
        let index = EmbeddedSparqClient::open_or_create(&fixture.data().join("index")).unwrap();
        let store = CompositeStore::new(
            index.clone(),
            FsBlobStore::open(fixture.data().join("blobs")).unwrap(),
        );
        store
            .write(
                &format!("{}/", fixture.base),
                bytes::Bytes::new(),
                "text/turtle",
            )
            .await
            .unwrap();
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<#public> a acl:Authorization; acl:accessTo <{0}/>; acl:default <{0}/>;
    acl:agentClass <http://xmlns.com/foaf/0.1/Agent>;
    acl:mode acl:Read, acl:Write, acl:Append, acl:Control ."#,
            fixture.base
        );
        store
            .write(
                &format!("{}/.acl", fixture.base),
                bytes::Bytes::from(acl),
                "text/turtle",
            )
            .await
            .unwrap();
        index.compact().await.unwrap();
        fixture
    }

    fn data(&self) -> std::path::PathBuf {
        self.directory.0.join("data")
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.client.request(method, format!("{}{path}", self.base))
    }
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    assert!(!sorted.is_empty());
    assert!((1..=100).contains(&percent));
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn file_count(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                file_count(&entry.path())
            } else {
                assert!(kind.is_file(), "unexpected index entry: {:?}", entry.path());
                1
            }
        })
        .sum()
}

async fn boot(fixture: &Fixture) -> (Process, Duration) {
    let data = fixture.data();
    let started = Instant::now();
    let process = Process::start(
        env!("CARGO_BIN_EXE_sparq-lws-core"),
        &[
            (
                "SOLID_SERVER_BIND",
                fixture.base.strip_prefix("http://").unwrap(),
            ),
            ("SOLID_SERVER_BASE_URL", &fixture.base),
            ("SOLID_SERVER_DATA_DIR", data.to_str().unwrap()),
            ("SOLID_SERVER_BODY_CACHE_BYTES", "0"),
        ],
    );
    loop {
        if let Ok(response) = fixture
            .client
            .get(format!("{}/readyz", fixture.base))
            .timeout(Duration::from_millis(250))
            .send()
            .await
        {
            if response.status() == StatusCode::OK {
                return (process, started.elapsed());
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "restart readiness timed out: {}",
            process.log()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn body(id: usize) -> Vec<u8> {
    format!("{id:016x}{}", "x".repeat(112)).into_bytes()
}

#[tokio::test]
#[ignore = "manual disk measurement; see solid-lws-server skill P0-9 instructions"]
async fn durable_resource_measurement() {
    let count: usize = std::env::var("P09_RESOURCES")
        .expect("set P09_RESOURCES to a positive resource count")
        .parse()
        .unwrap();
    assert!(count > 0 && count <= usize::MAX / 100);
    let scratch = std::env::var("TMPDIR").expect("set TMPDIR to a real-disk directory");
    assert!(Path::new(&scratch).is_absolute() && Path::new(&scratch).is_dir());
    let fixture = Fixture::new().await;
    assert!(fixture.directory.0.starts_with(&scratch));
    let (server, _) = boot(&fixture).await;
    let mut samples = Vec::with_capacity(count);
    for id in 0..count {
        let path = format!("/resource-{id:06}.bin");
        let request = fixture
            .request(Method::PUT, &path)
            .header("Content-Type", "application/octet-stream")
            .header("If-None-Match", "*")
            .body(body(id));
        let started = Instant::now();
        let response = request.send().await.unwrap();
        let status = response.status();
        let response_body = response.bytes().await.unwrap();
        let elapsed = started.elapsed();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "create {id}: {response_body:?}; server: {}",
            server.log()
        );
        samples.push(elapsed);
        if (id + 1) % 1000 == 0 {
            eprintln!("P0-9: created {} / {count}", id + 1);
        }
    }
    // kill() waits for the child and asserts its exit signal is SIGKILL, without compaction.
    server.kill();
    let index = fixture.data().join("index");
    let files = file_count(&index);
    assert!(files > 0 && index.join("sparq.lock").is_file());
    let (server, restart) = boot(&fixture).await;
    // Check acknowledged bytes through the reopened index and filesystem blob store.
    for id in [0, count / 2, count - 1] {
        let response = fixture
            .request(Method::GET, &format!("/resource-{id:06}.bin"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "recovered {id}");
        assert_eq!(response.bytes().await.unwrap().as_ref(), body(id));
    }
    samples.sort_unstable();
    println!(
        "{}",
        json!({
            "schema": 1,
            "debug_assertions": cfg!(debug_assertions),
            "resources": count,
            "samples": samples.len(),
            "payload_bytes": 128,
            "concurrency": 1,
            "protocol": "solid",
            "backend": "embedded-sparq+fs-blob-store",
            "create_metric": "HTTP PUT through complete response body",
            "percentile_method": "nearest-rank",
            "p50_create_ms": percentile(&samples, 50).as_secs_f64() * 1000.0,
            "p99_create_ms": percentile(&samples, 99).as_secs_f64() * 1000.0,
            "index_dir": index,
            "index_file_count": files,
            "restart_metric": "spawn-to-readyz after SIGKILL; warm OS page cache",
            "post_sigkill_open_ms": restart.as_secs_f64() * 1000.0,
            "kill_signal": 9,
            "recovered_samples_checked": 3,
        })
    );
    server.kill();
}

#[test]
fn nearest_rank_percentiles() {
    let samples: Vec<_> = (1..=200).map(Duration::from_millis).collect();
    assert_eq!(percentile(&samples, 50), Duration::from_millis(100));
    assert_eq!(percentile(&samples, 99), Duration::from_millis(198));
    assert_eq!(percentile(&samples[..3], 50), Duration::from_millis(2));
    assert_eq!(percentile(&samples[..3], 99), Duration::from_millis(3));
    assert_eq!(percentile(&samples[..1], 99), Duration::from_millis(1));
}

#[test]
fn counts_only_files_below_index() {
    let directory = common::process::Directory::new();
    let index = directory.0.join("index");
    std::fs::create_dir_all(index.join("nested")).unwrap();
    std::fs::write(index.join("base"), "").unwrap();
    std::fs::write(index.join("nested/journal"), "").unwrap();
    std::fs::write(index.join("nested/snapshot"), "").unwrap();
    std::fs::write(directory.0.join("blob"), "").unwrap();
    assert_eq!(file_count(&index), 3);
}

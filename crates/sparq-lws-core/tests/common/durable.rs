//! HTTP corpus and offline inspection for native durability tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde_json::json;
use sparq_lws_core::lws::{jose::EcKey, tokens, LwsConfig};
use sparq_lws_core::store::{
    reconcile_orphans, BlobStore, CompositeStore, EmbeddedSparqClient, ReconcileOptions,
    SparqClient, Store,
};

use super::common::process::{Directory, Process};
use sparq_lws_core::store::blob_fs::FsBlobStore;

pub const OWNER: &str = "https://fixtures.example/agents/alice";
pub const READER: &str = "https://fixtures.example/agents/bob";
pub const LWS_JSON: &str = "application/lws+json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Solid,
    Lws,
}

pub fn local_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

pub struct Fixture {
    pub directory: Directory,
    pub base: String,
    pub endpoint: String,
    pub mode: Mode,
    pub client: Client,
    pub key: EcKey,
}

impl Fixture {
    pub async fn new(mode: Mode) -> Self {
        let directory = Directory::new();
        std::fs::create_dir(directory.0.join("data")).unwrap();
        let key = EcKey::generate_thumbprinted();
        std::fs::write(
            directory.0.join("as-key.json"),
            key.private_jwk().to_string(),
        )
        .unwrap();
        // Let the OS choose a port. Reuse the origin across restarts: IRIs are persisted.
        let base = local_endpoint();
        let fixture = Self {
            directory,
            endpoint: base.clone(),
            base,
            mode,
            key,
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        };
        // The first boot owns format/lock initialization. Provision only the Solid root and its
        // test ACL offline; all scenario writes (including the descendant ACL) go through HTTP.
        fixture.boot().await.stop();
        if mode == Mode::Solid {
            let index = EmbeddedSparqClient::open_or_create(&fixture.data().join("index")).unwrap();
            let store = CompositeStore::new(
                index.clone(),
                FsBlobStore::open(fixture.data().join("blobs")).unwrap(),
            );
            store
                .write(
                    &format!("{}/", fixture.base),
                    bytes::Bytes::from_static(b""),
                    "text/turtle",
                )
                .await
                .unwrap();
            store
                .write(
                    &format!("{}/.acl", fixture.base),
                    bytes::Bytes::from(fixture.acl("/")),
                    "text/turtle",
                )
                .await
                .unwrap();
            index.compact().await.unwrap();
        }
        fixture
    }

    pub fn data(&self) -> std::path::PathBuf {
        self.directory.0.join("data")
    }

    pub async fn boot(&self) -> Process {
        let address = self.endpoint.strip_prefix("http://").unwrap();
        let data = self.data();
        let key = self.directory.0.join("as-key.json");
        let notify_key = self.directory.0.join("notify-key.json");
        let mut values = vec![
            ("SOLID_SERVER_BIND", address),
            ("SOLID_SERVER_BASE_URL", self.base.as_str()),
            ("SOLID_SERVER_DATA_DIR", data.to_str().unwrap()),
            ("SOLID_SERVER_BODY_CACHE_BYTES", "0"),
            ("SOLID_SERVER_MAX_BODY_BYTES", "8388608"),
            ("SOLID_SERVER_MAX_CONCURRENCY", "512"),
            ("SOLID_SERVER_REQUEST_TIMEOUT_SECS", "120"),
        ];
        if self.mode == Mode::Lws {
            values.extend([
                ("SOLID_SERVER_PROTOCOL", "lws"),
                ("SOLID_SERVER_LWS_OWNER", OWNER),
                ("SOLID_SERVER_LWS_AS_KEY_FILE", key.to_str().unwrap()),
                (
                    "SOLID_SERVER_LWS_NOTIFY_KEY_FILE",
                    notify_key.to_str().unwrap(),
                ),
                ("SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH", "1"),
                ("SOLID_SERVER_LWS_PAGE_SIZE", "1000"),
            ]);
        }
        let mut process = Process::start(env!("CARGO_BIN_EXE_sparq-lws-core"), &values);
        process.wait_ready(&self.client, &self.endpoint).await;
        process
    }

    pub fn token(&self, subject: &str) -> String {
        // Same HarnessIssuedTokens approach as conformance/lws/touchstone.sh: hold the AS key.
        let mut config = LwsConfig::new(&self.base);
        config.as_key = EcKey::from_jwk(&self.key.private_jwk().to_string()).unwrap();
        tokens::mint(
            &config,
            subject,
            "https://fixtures.example/app",
            &config.realm(),
        )
    }

    pub fn request_as(&self, method: Method, path: &str, subject: Option<&str>) -> RequestBuilder {
        let request = self.client.request(method, format!("{}{path}", self.endpoint));
        match subject {
            Some(subject) if self.mode == Mode::Lws => request.bearer_auth(self.token(subject)),
            _ => request,
        }
    }

    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.request_as(method, path, Some(OWNER))
    }

    pub async fn put(&self, path: &str, body: Vec<u8>, content_type: &str) -> Response {
        let create = self.mode == Mode::Lws
            && self
                .request(Method::HEAD, path)
                .send()
                .await
                .unwrap()
                .status()
                == StatusCode::NOT_FOUND;
        let response = self
            .write_request(path, create)
            .header("Content-Type", content_type)
            .body(body)
            .send()
            .await
            .unwrap();
        success(response, path).await
    }

    pub fn write_request(&self, path: &str, create: bool) -> RequestBuilder {
        if self.mode == Mode::Lws && create {
            let (parent, slug) = split_member(path);
            self.request(Method::POST, &parent).header("Slug", slug)
        } else {
            self.request(Method::PUT, path)
        }
    }

    pub async fn container(&self, path: &str) {
        let response = if self.mode == Mode::Solid {
            self.request(Method::PUT, path)
                .header(
                    "Link",
                    "<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\"",
                )
                .header("Content-Type", "text/turtle")
                .body("")
                .send()
                .await
                .unwrap()
        } else {
            let (parent, slug) = split_member(path.trim_end_matches('/'));
            self.request(Method::POST, &parent)
                .header("Slug", slug)
                .header("Link", "<http://www.w3.org/ns/lws#Container>; rel=\"type\"")
                .header("Content-Type", "text/turtle")
                .body("")
                .send()
                .await
                .unwrap()
        };
        success(response, path).await;
        assert_eq!(self.snapshot(path).await.status, 200, "container {path}");
    }

    pub fn acl(&self, path: &str) -> String {
        format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
<#public> a acl:Authorization; acl:accessTo <{0}{1}>; acl:default <{0}{1}>;
    acl:agentClass foaf:Agent; acl:mode acl:Read, acl:Write, acl:Append, acl:Control .
"#,
            self.base, path
        )
    }

    pub async fn grant_read(&self, path: &str) -> String {
        let body = json!({"@context": ["https://www.w3.org/ns/lws/v1"], "type": ["AccessGrant"],
            "storage": format!("{}/", self.base),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": READER,
                "target": {"type": "StorageResource", "value": [format!("{}{path}", self.base)]}}]});
        let response = self
            .request(Method::POST, "/.lws/grants/")
            .header("Content-Type", LWS_JSON)
            .json(&body)
            .send()
            .await
            .unwrap();
        let response = success(response, "grant").await;
        assert_eq!(response.status(), 201);
        response.headers()["location"].to_str().unwrap().to_owned()
    }

    pub async fn snapshot(&self, path: &str) -> Snapshot {
        let response = self
            .request(Method::GET, path)
            .header(
                "Accept",
                if self.mode == Mode::Solid {
                    "text/turtle"
                } else {
                    LWS_JSON
                },
            )
            .send()
            .await
            .unwrap();
        Snapshot::read(response).await
    }
}

pub async fn success(response: Response, context: &str) -> Response {
    if !response.status().is_success() {
        let status = response.status();
        panic!("{context}: {status}: {}", response.text().await.unwrap());
    }
    response
}

pub fn split_member(path: &str) -> (String, String) {
    let (parent, name) = path.rsplit_once('/').unwrap();
    (format!("{parent}/"), name.to_string())
}

#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub last_modified: Option<String>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use sha2::{Digest, Sha256};
        f.debug_struct("Snapshot")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .field("body_sha256", &Sha256::digest(&self.body))
            .field("etag", &self.etag)
            .field("content_type", &self.content_type)
            .field("last_modified", &self.last_modified)
            .finish()
    }
}

impl Snapshot {
    pub async fn read(response: Response) -> Self {
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_owned())
        };
        Self {
            status: response.status().as_u16(),
            etag: header("etag"),
            content_type: header("content-type"),
            last_modified: header("last-modified"),
            body: response.bytes().await.unwrap().to_vec(),
        }
    }

    pub fn assert_resource(&self, path: &str) {
        assert_eq!(self.status, 200, "{path}: {self:?}");
        assert!(self.etag.is_some(), "ETag missing: {path}");
        assert!(self.content_type.is_some(), "Content-Type missing: {path}");
        assert!(
            self.last_modified.is_some(),
            "Last-Modified missing: {path}"
        );
    }
}

pub fn payload(id: usize, length: usize) -> Vec<u8> {
    (0..length)
        .map(|offset| ((offset.wrapping_mul(31) + id * 17) % 251) as u8)
        .collect()
}

pub struct Corpus {
    pub snapshots: BTreeMap<String, Snapshot>,
    pub grant: Option<String>,
}

impl Corpus {
    pub async fn write(f: &Fixture) -> Self {
        for path in ["/corpus/", "/corpus/nested/", "/corpus/nested/deep/"] {
            f.container(path).await;
        }
        for (path, body, content_type) in [
            (
                "/corpus/nested/deep/data.ttl",
                b"<urn:s> <urn:p> <urn:o> .\n".to_vec(),
                "text/turtle",
            ),
            ("/corpus/empty.bin", vec![], "application/octet-stream"),
            (
                "/corpus/large.bin",
                payload(9, 4 * 1024 * 1024),
                "application/octet-stream",
            ),
            (
                "/corpus/overwrite.bin",
                payload(2, 1024),
                "application/octet-stream",
            ),
            (
                "/corpus/deleted.bin",
                payload(3, 64),
                "application/octet-stream",
            ),
        ] {
            f.put(path, body.clone(), content_type).await;
            if content_type == "application/octet-stream" {
                assert_eq!(
                    f.snapshot(path).await.body,
                    body,
                    "initial binary bytes: {path}"
                );
            }
        }
        f.put(
            "/corpus/overwrite.bin",
            payload(4, 8193),
            "application/octet-stream",
        )
        .await;
        success(
            f.request(Method::DELETE, "/corpus/deleted.bin")
                .send()
                .await
                .unwrap(),
            "delete",
        )
        .await;
        let grant = if f.mode == Mode::Solid {
            f.put(
                "/corpus/.acl",
                f.acl("/corpus/").into_bytes(),
                "text/turtle",
            )
            .await;
            None
        } else {
            Some(f.grant_read("/corpus/large.bin").await)
        };
        let mut snapshots = BTreeMap::new();
        for path in [
            "/corpus/",
            "/corpus/nested/",
            "/corpus/nested/deep/",
            "/corpus/nested/deep/data.ttl",
            "/corpus/empty.bin",
            "/corpus/large.bin",
            "/corpus/overwrite.bin",
        ] {
            let snapshot = f.snapshot(path).await;
            snapshot.assert_resource(path);
            snapshots.insert(path.into(), snapshot);
        }
        if f.mode == Mode::Solid {
            snapshots.insert("/corpus/.acl".into(), f.snapshot("/corpus/.acl").await);
        }
        assert_eq!(f.snapshot("/corpus/deleted.bin").await.status, 404);
        Self { snapshots, grant }
    }

    pub async fn verify(&self, f: &Fixture) {
        for (path, expected) in &self.snapshots {
            assert_eq!(&f.snapshot(path).await, expected, "restart changed {path}");
        }
        assert_eq!(f.snapshot("/corpus/deleted.bin").await.status, 404);
        if let Some(grant) = &self.grant {
            let path = grant.strip_prefix(&f.base).unwrap();
            assert_eq!(f.snapshot(path).await.status, 200);
            assert_eq!(
                f.request_as(Method::GET, "/corpus/large.bin", Some(READER))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
    }
}

// Inspect only while the real server is stopped: the embedded client holds the exclusive lock.
// The public index and reconciler APIs avoid depending on the on-disk index format.
pub async fn inspect_blobs(f: &Fixture, sweep: bool) {
    assert!(
        std::fs::read_dir(f.data().join("blobs/tmp"))
            .unwrap()
            .next()
            .is_none(),
        "shutdown must leave tmp empty before any adapter open can clean it"
    );
    let index = EmbeddedSparqClient::open_or_create(&f.data().join("index")).unwrap();
    let blob = FsBlobStore::open(f.data().join("blobs")).unwrap();
    let referenced = index.referenced_blob_keys().await.unwrap();
    assert!(
        !referenced.is_empty(),
        "empty index cannot witness blob durability"
    );
    let mut bodies = BTreeMap::new();
    for key in &referenced {
        bodies.insert(key.clone(), BlobStore::get(&blob, key).await.unwrap());
    }
    if sweep {
        // No writers can race this zero-grace sweep: the server is stopped and this owns the lock.
        // A known unreferenced object makes invocation of the zero-grace sweep observable even
        // when this particular crash happened to leave no natural orphan.
        let canary = "b1-00000000000000000000000000000000";
        BlobStore::put(&blob, canary, bytes::Bytes::from_static(b"orphan canary"))
            .await
            .unwrap();
        let report = reconcile_orphans(
            &index,
            &blob,
            &ReconcileOptions {
                grace: Duration::ZERO,
                dry_run: false,
            },
        )
        .await
        .unwrap();
        assert!(
            report.deleted >= 1,
            "zero-grace sweep must reclaim the orphan canary"
        );
        assert_eq!(report.delete_errors, 0);
        assert_eq!(report.skipped_unknown_age, 0);
    }
    let mut files = BTreeSet::new();
    for plane in ["objects", "pool"] {
        walk_files(&f.data().join("blobs").join(plane), &mut files);
    }
    assert_eq!(
        files,
        referenced.into_iter().collect(),
        "blob plane differs from index"
    );
    assert!(
        std::fs::read_dir(f.data().join("blobs/tmp"))
            .unwrap()
            .next()
            .is_none(),
        "tmp must be empty"
    );
    assert!(
        files.iter().any(|key| !key.starts_with("$pool$")),
        "object plane must be exercised"
    );
    if f.mode == Mode::Lws && !sweep {
        assert!(
            files.iter().any(|key| key.starts_with("$pool$")),
            "service pool must be exercised"
        );
    }
    for (key, expected) in bodies {
        assert_eq!(
            BlobStore::get(&blob, &key).await.unwrap(),
            expected,
            "sweep removed live {key}"
        );
    }
}

fn walk_files(path: &Path, files: &mut BTreeSet<String>) {
    if !path.exists() {
        return;
    }
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            walk_files(&entry.path(), files);
        } else {
            assert!(
                entry.file_type().unwrap().is_file(),
                "unexpected blob entry"
            );
            assert!(
                files.insert(entry.file_name().into_string().unwrap()),
                "duplicate blob key"
            );
        }
    }
}

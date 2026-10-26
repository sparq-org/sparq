//! Offline archives of the native data-directory contract, restored through the real binary.
#![cfg(all(unix, feature = "embedded-sparq", feature = "blob-fs"))]

mod common;
#[path = "common/durable.rs"]
mod durable;

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::process::{Directory, Process};
use durable::{
    inspect_blobs, local_endpoint, payload, success, Corpus, Fixture, Mode, Snapshot, LWS_JSON,
    READER,
};
use reqwest::Method;
use serde_json::{json, Value};
use sparq_lws_core::ldp::content::{parse_to_triples, RdfFormat};
use sparq_lws_core::lws::jose::EcKey;
use sparq_lws_core::store::EmbeddedSparqClient;

const TEST_DEADLINE: Duration = Duration::from_secs(240);

// Poll even the archive subprocess against a deadline; a timeout must reap it.
fn tar(command: &mut Command) {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("system tar is required");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "tar failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("tar exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn archive(data: &Path, output: &Path) {
    assert!(data.join("format.json").is_file());
    assert!(data.join("index").is_dir());
    assert!(data.join("blobs").is_dir());
    tar(Command::new("tar")
        .arg("-cpf")
        .arg(output)
        .arg("-C")
        .arg(data)
        .arg("."));
}

fn unpack(archive: &Path, destination: &Path) -> std::io::Result<()> {
    if std::fs::read_dir(destination)?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "restore requires an empty directory",
        ));
    }
    tar(Command::new("tar")
        .arg("-xpf")
        .arg(archive)
        .arg("-C")
        .arg(destination));
    Ok(())
}

// Compare all archive bytes and directory entries, including advisory lock files.
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                files.insert(relative, None);
                visit(root, &path, files);
            } else {
                assert!(kind.is_file(), "unexpected archive entry: {path:?}");
                files.insert(relative, Some(std::fs::read(path).unwrap()));
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn refuses(data: &Path, mode: Mode, expected: &str) {
    let before = tree(data);
    let mut values = vec![("SOLID_SERVER_DATA_DIR", data.to_str().unwrap())];
    if mode == Mode::Lws {
        values.push(("SOLID_SERVER_PROTOCOL", "lws"));
    }
    let output = Process::start(env!("CARGO_BIN_EXE_sparq-lws-core"), &values).finish();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "boot refusal exit: {stderr}");
    let diagnostic = format!("Error: Custom {{ kind: InvalidInput, error: {expected:?} }}");
    assert_eq!(
        stderr.lines().last(),
        Some(diagnostic.as_str()),
        "wrong boot refusal: {stderr}"
    );
    assert_eq!(
        tree(data),
        before,
        "refused boot changed the restore target"
    );
}

fn incompatible_majors(data: &Path, mode: Mode) {
    let marker = data.join("format.json");
    let original = std::fs::read(&marker).unwrap();
    let format: Value = serde_json::from_slice(&original).unwrap();
    for plane in ["index", "blobs"] {
        let mut altered = format.clone();
        altered[plane]["major"] = json!(2);
        std::fs::write(&marker, serde_json::to_vec(&altered).unwrap()).unwrap();
        refuses(
            data,
            mode,
            "unsupported format.json: expected schema=1, index.major=1, blobs.major=1",
        );
    }
    std::fs::write(marker, original).unwrap();
}

// Reuse the harness with the public origin and external signing configuration, without
// provisioning or opening the restored store. Only the transport listener changes.
fn restored_fixture(original: &Fixture, directory: Directory) -> Fixture {
    for name in ["as-key.json", "notify-key.json"] {
        let source = original.directory.0.join(name);
        if source.is_file() {
            std::fs::copy(source, directory.0.join(name)).unwrap();
        }
    }
    Fixture {
        directory,
        base: original.base.clone(),
        endpoint: local_endpoint(),
        mode: original.mode,
        client: original.client.clone(),
        key: EcKey::from_jwk(&original.key.private_jwk().to_string()).unwrap(),
    }
}

fn assert_listing(f: &Fixture, path: &str, snapshot: &Snapshot, expected_paths: &[&str]) {
    let parent = format!("{}{path}", f.base);
    let members: BTreeSet<String> = if f.mode == Mode::Lws {
        let listing: Value = serde_json::from_slice(&snapshot.body).unwrap();
        assert_eq!(listing["id"].as_str(), Some(parent.as_str()));
        let items = listing["items"].as_array().unwrap();
        assert_eq!(
            listing["totalItems"].as_u64(),
            Some(expected_paths.len() as u64)
        );
        assert_eq!(items.len(), expected_paths.len(), "complete corpus listing");
        items
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect()
    } else {
        parse_to_triples(RdfFormat::Turtle, &snapshot.body, &parent)
            .unwrap()
            .into_iter()
            .filter(|triple| triple.predicate.as_str() == "http://www.w3.org/ns/ldp#contains")
            .map(|triple| {
                assert_eq!(triple.subject.to_string(), format!("<{parent}>"));
                match triple.object {
                    oxrdf::Term::NamedNode(iri) => iri.into_string(),
                    other => panic!("non-IRI corpus member: {other}"),
                }
            })
            .collect()
    };
    let expected = expected_paths
        .iter()
        .map(|path| format!("{}{path}", f.base))
        .collect();
    assert_eq!(members, expected, "corpus listing membership: {path}");
}

fn assert_corpus(corpus: &Corpus, f: &Fixture) {
    let mut paths: BTreeSet<_> = [
        "/corpus/",
        "/corpus/nested/",
        "/corpus/nested/deep/",
        "/corpus/nested/deep/data.ttl",
        "/corpus/empty.bin",
        "/corpus/large.bin",
        "/corpus/overwrite.bin",
    ]
    .into_iter()
    .collect();
    if f.mode == Mode::Solid {
        paths.insert("/corpus/.acl");
    } else {
        assert!(
            corpus.grant.is_some(),
            "restore must exercise a persisted LWS grant"
        );
    }
    assert_eq!(
        corpus
            .snapshots
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        paths,
        "restore must verify the complete corpus"
    );
    for (path, snapshot) in &corpus.snapshots {
        snapshot.assert_resource(path, f.mode);
        for (name, value) in [
            ("ETag", snapshot.etag.as_deref()),
            ("Content-Type", snapshot.content_type.as_deref()),
        ] {
            assert!(
                value.is_some_and(|value| !value.trim().is_empty()),
                "empty corpus {name}: {path}"
            );
        }
        if f.mode == Mode::Lws || !path.ends_with('/') {
            assert!(
                snapshot
                    .last_modified
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
                "empty corpus Last-Modified: {path}"
            );
        }
    }
    assert_eq!(
        corpus.snapshots["/corpus/large.bin"].body.len(),
        4 * 1024 * 1024
    );
    assert_eq!(
        corpus.snapshots["/corpus/large.bin"].body,
        payload(9, 4 * 1024 * 1024)
    );
    assert!(corpus.snapshots["/corpus/empty.bin"].body.is_empty());
    assert_eq!(
        corpus.snapshots["/corpus/overwrite.bin"].body,
        payload(4, 8193)
    );
    let rdf_path = "/corpus/nested/deep/data.ttl";
    let triples = parse_to_triples(
        RdfFormat::Turtle,
        &corpus.snapshots[rdf_path].body,
        &format!("{}{rdf_path}", f.base),
    )
    .unwrap();
    assert_eq!(triples.len(), 1, "restore must exercise the RDF resource");
    assert_eq!(triples[0].subject.to_string(), "<urn:s>");
    assert_eq!(triples[0].predicate.as_str(), "urn:p");
    assert_eq!(triples[0].object.to_string(), "<urn:o>");
    for (parent, members) in [
        (
            "/corpus/",
            [
                "/corpus/nested/",
                "/corpus/empty.bin",
                "/corpus/large.bin",
                "/corpus/overwrite.bin",
            ]
            .as_slice(),
        ),
        ("/corpus/nested/", ["/corpus/nested/deep/"].as_slice()),
        (
            "/corpus/nested/deep/",
            ["/corpus/nested/deep/data.ttl"].as_slice(),
        ),
    ] {
        assert_listing(f, parent, &corpus.snapshots[parent], members);
    }
}

fn assert_distinct_storage(original: &Path, restored: &Path) {
    for relative in ["", "index", "blobs", "index.lock", "index/sparq.lock"] {
        let original = original.join(relative);
        let restored = restored.join(relative);
        assert_ne!(
            std::fs::canonicalize(&original).unwrap(),
            std::fs::canonicalize(&restored).unwrap(),
            "aliased restore path: {relative}"
        );
        let original = std::fs::metadata(original).unwrap();
        let restored = std::fs::metadata(restored).unwrap();
        assert_ne!(
            (original.dev(), original.ino()),
            (restored.dev(), restored.ino()),
            "shared restore directory or lock: {relative}"
        );
    }
}

fn assert_index_locked(f: &Fixture) {
    match EmbeddedSparqClient::open_or_create(&f.data().join("index")) {
        Ok(_) => panic!(
            "server does not own its configured index lock: {:?}",
            f.data()
        ),
        Err(error) => assert_eq!(
            error.kind(),
            std::io::ErrorKind::WouldBlock,
            "index lock probe: {error}"
        ),
    }
}

async fn verify(corpus: &Corpus, f: &Fixture) {
    // Includes bodies, ETags, Content-Type, Last-Modified, all nested listings and the delete.
    corpus.verify(f).await;
    if f.mode == Mode::Lws {
        let response = f
            .request_as(Method::GET, "/corpus/large.bin", Some(READER))
            .header("Accept", LWS_JSON)
            .send()
            .await
            .unwrap();
        assert_eq!(
            Snapshot::read(response).await,
            corpus.snapshots["/corpus/large.bin"],
            "grant must serve the restored bytes and validators to the reader"
        );
        assert_eq!(
            f.request_as(Method::GET, "/corpus/overwrite.bin", Some(READER))
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "reader must not have access outside the grant"
        );
    }
}

async fn offline_backup_and_restore(mode: Mode) -> (Fixture, Fixture) {
    let original = Fixture::new(mode).await;
    let server = original.boot().await;
    let corpus = Corpus::write(&original).await;
    assert_corpus(&corpus, &original);
    verify(&corpus, &original).await;
    server.stop(); // SIGTERM, exit zero, drained and compacted before tar starts.
    let expected_tree = tree(&original.data());

    let backup = Directory::new();
    let archive_path = backup.0.join("data.tar");
    archive(&original.data(), &archive_path);
    let destination = Directory::new();
    let data = destination.0.join("data");
    std::fs::create_dir(&data).unwrap();
    assert_ne!(
        std::fs::canonicalize(&data).unwrap(),
        std::fs::canonicalize(original.data()).unwrap()
    );
    unpack(&archive_path, &data).unwrap();
    assert_distinct_storage(&original.data(), &data);
    assert_eq!(tree(&data), expected_tree, "archive lost data or locks");
    assert_eq!(
        tree(&original.data()),
        expected_tree,
        "archive or extraction changed the source"
    );

    // Start the original first so its listener is occupied when choosing the copy's port.
    // Configuration and signing keys live outside the data archive in this harness.
    let original_server = original.boot().await;
    let restored = restored_fixture(&original, destination);
    assert_ne!(restored.data(), original.data());
    assert_ne!(restored.endpoint, original.endpoint);
    assert_ne!(
        reqwest::Url::parse(&restored.endpoint).unwrap().port(),
        reqwest::Url::parse(&original.endpoint).unwrap().port()
    );
    assert_eq!(
        restored.base, original.base,
        "stored IRIs retain their origin"
    );
    let restored_server = restored.boot().await;
    assert_ne!(original_server.id(), restored_server.id());
    assert_index_locked(&original);
    assert_index_locked(&restored);
    tokio::join!(verify(&corpus, &original), verify(&corpus, &restored));

    // Witness that requests reached independent stores, not the original twice.
    let body = payload(23, 16385);
    restored
        .put(
            "/corpus/copy-only.bin",
            body.clone(),
            "application/octet-stream",
        )
        .await;
    assert_eq!(restored.snapshot("/corpus/copy-only.bin").await.body, body);
    assert_eq!(original.snapshot("/corpus/copy-only.bin").await.status, 404);
    verify(&corpus, &original).await;
    if let Some(grant) = &corpus.grant {
        success(
            restored
                .request(Method::DELETE, grant.strip_prefix(&restored.base).unwrap())
                .send()
                .await
                .unwrap(),
            "revoke restored grant",
        )
        .await;
        assert_eq!(
            restored
                .request_as(Method::GET, "/corpus/large.bin", Some(READER))
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "restored reader must depend on the persisted grant"
        );
        verify(&corpus, &original).await;
    }
    restored_server.stop();
    assert_index_locked(&original);
    drop(EmbeddedSparqClient::open_or_create(&restored.data().join("index")).unwrap());
    verify(&corpus, &original).await;
    original_server.stop();
    incompatible_majors(&restored.data(), mode);
    (original, restored)
}

// Keep exact orphan checks independently failing: an unreferenced blob must not prevent
// the archive, HTTP equality, grant, refusal and independent-directory checks from executing.
async fn backup_blob_planes_match_index(mode: Mode) {
    let (original, restored) = offline_backup_and_restore(mode).await;
    inspect_blobs(&original, false).await;
    inspect_blobs(&restored, false).await;
}

#[tokio::test]
async fn offline_backup_and_restore_solid() {
    tokio::time::timeout(TEST_DEADLINE, offline_backup_and_restore(Mode::Solid))
        .await
        .expect("Solid backup/restore exceeded deadline");
}

#[tokio::test]
async fn offline_backup_and_restore_lws() {
    tokio::time::timeout(TEST_DEADLINE, offline_backup_and_restore(Mode::Lws))
        .await
        .expect("LWS backup/restore exceeded deadline");
}

#[tokio::test]
async fn backup_blob_planes_match_index_solid() {
    tokio::time::timeout(TEST_DEADLINE, backup_blob_planes_match_index(Mode::Solid))
        .await
        .expect("Solid backup blob inspection exceeded deadline");
}

#[tokio::test]
async fn backup_blob_planes_match_index_lws() {
    tokio::time::timeout(TEST_DEADLINE, backup_blob_planes_match_index(Mode::Lws))
        .await
        .expect("LWS backup blob inspection exceeded deadline");
}

fn foreign_restore_target_is_refused_without_changes(mode: Mode) {
    let source = archive_fixture();
    let backup = Directory::new();
    let archive_path = backup.0.join("data.tar");
    archive(&source.0, &archive_path);
    for plane in ["index", "blobs"] {
        let destination = Directory::new();
        std::fs::create_dir(destination.0.join(plane)).unwrap();
        std::fs::write(destination.0.join(plane).join("foreign"), b"preserve me").unwrap();
        let before = tree(&destination.0);
        assert_eq!(
            unpack(&archive_path, &destination.0).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            tree(&destination.0),
            before,
            "unpack overwrote foreign data"
        );
        // Pre-extraction boot guard: never overlay this directory with a valid marker from tar.
        refuses(
            &destination.0,
            mode,
            &format!("refusing foreign data directory {}: format.json is absent but {plane}/ has content", destination.0.display()),
        );
    }
}

#[test]
fn foreign_restore_target_is_refused_without_changes_solid() {
    foreign_restore_target_is_refused_without_changes(Mode::Solid);
}

#[test]
fn foreign_restore_target_is_refused_without_changes_lws() {
    foreign_restore_target_is_refused_without_changes(Mode::Lws);
}

// Synthetic on-disk bytes for archive fidelity and guards that run before either plane opens.
fn archive_fixture() -> Directory {
    let source = Directory::new();
    for path in ["index", "blobs/objects/ab", "blobs/pool", "blobs/tmp"] {
        std::fs::create_dir_all(source.0.join(path)).unwrap();
    }
    for (path, bytes) in [
        (
            "format.json",
            b"{\"schema\":1,\"index\":{\"major\":1},\"blobs\":{\"major\":1}}\n".as_slice(),
        ),
        ("index/sparq.lock", b"".as_slice()),
        ("index.lock", b"".as_slice()),
        ("index/snapshot", b"index fixture\0".as_slice()),
        ("blobs/objects/ab/body", b"\0binary\xff".as_slice()),
        ("blobs/pool/service", b"grant fixture".as_slice()),
    ] {
        std::fs::write(source.0.join(path), bytes).unwrap();
    }
    source
}

fn archive_layout_locks_and_restored_format_guards(mode: Mode) {
    // The HTTP tests above additionally alter both majors in a restored real corpus.
    let source = archive_fixture();
    let backup = Directory::new();
    let archive_path = backup.0.join("data.tar");
    archive(&source.0, &archive_path);
    let destination = Directory::new();
    unpack(&archive_path, &destination.0).unwrap();
    assert_eq!(tree(&destination.0), tree(&source.0));
    incompatible_majors(&destination.0, mode);
    assert_eq!(tree(&destination.0), tree(&source.0));
}

#[test]
fn archive_layout_locks_and_restored_format_guards_solid() {
    archive_layout_locks_and_restored_format_guards(Mode::Solid);
}

#[test]
fn archive_layout_locks_and_restored_format_guards_lws() {
    archive_layout_locks_and_restored_format_guards(Mode::Lws);
}

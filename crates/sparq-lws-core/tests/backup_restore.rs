//! Offline archives of the native data-directory contract, restored through the real binary.
#![cfg(all(unix, feature = "embedded-sparq", feature = "blob-fs"))]

mod common;
#[path = "common/durable.rs"]
mod durable;

use std::collections::BTreeMap;
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
use sparq_lws_core::lws::jose::EcKey;

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

fn refuses(data: &Path, expected: &str) {
    let before = tree(data);
    let output = Process::start(
        env!("CARGO_BIN_EXE_sparq-lws-core"),
        &[("SOLID_SERVER_DATA_DIR", data.to_str().unwrap())],
    )
    .finish();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "boot unexpectedly succeeded: {stderr}"
    );
    assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
    assert_eq!(
        tree(data),
        before,
        "refused boot changed the restore target"
    );
}

fn incompatible_majors(data: &Path) {
    let marker = data.join("format.json");
    let original = std::fs::read(&marker).unwrap();
    let format: Value = serde_json::from_slice(&original).unwrap();
    for plane in ["index", "blobs"] {
        let mut altered = format.clone();
        altered[plane]["major"] = json!(2);
        std::fs::write(&marker, serde_json::to_vec(&altered).unwrap()).unwrap();
        refuses(data, "unsupported format.json");
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

async fn offline_backup_and_restore(mode: Mode) {
    let original = Fixture::new(mode).await;
    let server = original.boot().await;
    let corpus = Corpus::write(&original).await;
    verify(&corpus, &original).await;
    server.stop(); // SIGTERM, exit zero, drained and compacted before tar starts.
    inspect_blobs(&original, false).await;

    let backup = Directory::new();
    let archive_path = backup.0.join("data.tar");
    archive(&original.data(), &archive_path);
    let destination = Directory::new();
    let data = destination.0.join("data");
    std::fs::create_dir(&data).unwrap();
    unpack(&archive_path, &data).unwrap();
    assert_eq!(
        tree(&data),
        tree(&original.data()),
        "archive lost data or locks"
    );

    // Start the original first so its listener is occupied when choosing the copy's port.
    // Configuration and signing keys live outside the data archive in this harness.
    let original_server = original.boot().await;
    let restored = restored_fixture(&original, destination);
    assert_ne!(restored.data(), original.data());
    assert_ne!(restored.endpoint, original.endpoint);
    assert_eq!(
        restored.base, original.base,
        "stored IRIs retain their origin"
    );
    let restored_server = restored.boot().await;
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
    original_server.stop();
    inspect_blobs(&restored, false).await;
    incompatible_majors(&restored.data());
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

#[test]
fn foreign_restore_target_is_refused_without_changes() {
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
        refuses(&destination.0, "refusing foreign data directory");
    }
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

#[test]
fn archive_layout_locks_and_restored_format_guards() {
    // The HTTP tests above additionally alter both majors in a restored real corpus.
    let source = archive_fixture();
    let backup = Directory::new();
    let archive_path = backup.0.join("data.tar");
    archive(&source.0, &archive_path);
    let destination = Directory::new();
    unpack(&archive_path, &destination.0).unwrap();
    assert_eq!(tree(&destination.0), tree(&source.0));
    incompatible_majors(&destination.0);
    assert_eq!(tree(&destination.0), tree(&source.0));
}

//! Exercise native boot decisions through the real executable and literal environment names.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let dir = Self(std::env::temp_dir().join(format!("lws-boot-{suffix}")));
        std::fs::create_dir(&dir.0).unwrap();
        dir
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Process(Option<Child>);

impl Process {
    fn start(values: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sparq-lws-core"));
        command
            .env_clear()
            .env("SOLID_SERVER_BIND", "127.0.0.1:0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (key, value) in values {
            command.env(key, value);
        }
        Self(Some(command.spawn().unwrap()))
    }

    fn finish(mut self) -> Output {
        let started = Instant::now();
        let child = self.0.as_mut().unwrap();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(10) {
                child.kill().unwrap();
                let output = self.0.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "boot did not exit: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn refuses(values: &[(&str, &str)], expected: &str) {
    let output = Process::start(values).finish();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "boot unexpectedly succeeded: {stderr}"
    );
    assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
}

#[cfg(all(target_os = "linux", feature = "embedded-sparq", feature = "blob-fs"))]
fn journal_lengths(dir: &std::path::Path) -> Vec<u64> {
    let mut lengths = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            lengths.extend(journal_lengths(&path));
        } else if matches!(
            path.file_name().unwrap().to_str(),
            Some("wal.log" | "txn.log")
        ) {
            lengths.push(std::fs::metadata(path).unwrap().len());
        }
    }
    lengths
}

#[test]
fn native_boot_refuses_conflicting_paths_and_backend_selection() {
    let dir = Directory::new();
    let path = dir.0.to_str().unwrap();
    refuses(
        &[
            ("SOLID_SERVER_DATA_DIR", path),
            ("SOLID_SERVER_SPARQ_DIR", "old"),
        ],
        "cannot be set together",
    );
    for backend in ["memory", "http", "unknown", ""] {
        refuses(
            &[
                ("SOLID_SERVER_DATA_DIR", path),
                ("PSS_SPARQ_BACKEND", backend),
            ],
            "requires PSS_SPARQ_BACKEND=embedded",
        );
    }
    assert!(std::fs::read_dir(&dir.0).unwrap().next().is_none());
}

#[test]
fn native_boot_keeps_the_durable_index_ephemeral_blob_guard() {
    let dir = Directory::new();
    for backend in ["embedded", "http"] {
        for allow in ["0", "1"] {
            refuses(
                &[
                    ("PSS_SPARQ_BACKEND", backend),
                    ("SOLID_SERVER_SPARQ_DIR", dir.0.to_str().unwrap()),
                    ("SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY", allow),
                ],
                "requires a durable blob store",
            );
        }
    }
    assert!(std::fs::read_dir(&dir.0).unwrap().next().is_none());
}

#[test]
fn native_boot_refuses_invalid_lock_wait_values() {
    for value in ["-1", "bad", "0.5"] {
        refuses(
            &[("SOLID_SERVER_WAIT_FOR_LOCK_SECS", value)],
            "must be a non-negative integer",
        );
    }
}

#[cfg(all(feature = "embedded-sparq", not(feature = "blob-fs")))]
#[test]
fn data_directory_boot_refuses_the_missing_filesystem_adapter() {
    let dir = Directory::new();
    refuses(
        &[("SOLID_SERVER_DATA_DIR", dir.0.to_str().unwrap())],
        "filesystem blob store is not built in",
    );
    assert!(dir.0.join("format.json").is_file());
    assert!(!dir.0.join("blobs").exists());
    // The failed blob constructor must drop and join the index actor.
    drop(sparq_lws_core::store::EmbeddedSparqClient::open_or_create(&dir.0.join("index")).unwrap());
}

#[cfg(not(feature = "embedded-sparq"))]
#[test]
fn data_directory_boot_requires_the_embedded_feature() {
    let dir = Directory::new();
    refuses(
        &[("SOLID_SERVER_DATA_DIR", dir.0.to_str().unwrap())],
        "requires the `embedded-sparq` build feature",
    );
    assert!(std::fs::read_dir(&dir.0).unwrap().next().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn native_server_drains_on_terminate_and_interrupt() {
    use std::io::{BufRead, BufReader};
    #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
    let dir = Directory::new();
    let configurations: &[&[(&str, &str)]] = &[
        &[],
        #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
        &[
            ("SOLID_SERVER_DATA_DIR", dir.0.to_str().unwrap()),
            // Only this disposable directory permits fixtures in a durable backend.
            ("SOLID_SERVER_SEED_DEMO", "1"),
            ("SOLID_SERVER_ALLOW_SEED_NONMEMORY", "1"),
        ],
    ];
    for values in configurations {
        for signal in ["-TERM", "-INT"] {
            let mut process = Process::start(values);
            let stderr = process.0.as_mut().unwrap().stderr.take().unwrap();
            let (ready, received) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                let mut ready = Some(ready);
                for line in BufReader::new(stderr).lines() {
                    let line = line.unwrap();
                    if line.contains("WARNING: experimental parallel track") {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    }
                }
            });
            received.recv_timeout(Duration::from_secs(10)).unwrap();
            // Observe registration, rather than racing the listener's first poll.
            let registered = Instant::now();
            loop {
                let status = std::fs::read_to_string(format!(
                    "/proc/{}/status",
                    process.0.as_ref().unwrap().id()
                ))
                .unwrap();
                let caught = status
                    .lines()
                    .find_map(|line| line.strip_prefix("SigCgt:\t"))
                    .unwrap();
                let caught = u64::from_str_radix(caught.trim(), 16).unwrap();
                if caught & ((1 << 1) | (1 << 14)) == ((1 << 1) | (1 << 14)) {
                    break;
                }
                assert!(
                    registered.elapsed() < Duration::from_secs(10),
                    "signal handlers were not registered"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
            if !values.is_empty() {
                assert!(journal_lengths(&dir.0.join("index"))
                    .iter()
                    .any(|len| *len > 0));
            }
            let status = Command::new("kill")
                .arg(signal)
                .arg(process.0.as_ref().unwrap().id().to_string())
                .status()
                .unwrap();
            assert!(status.success());
            assert!(process.finish().status.success());
            reader.join().unwrap();
            #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
            if !values.is_empty() {
                let lengths = journal_lengths(&dir.0.join("index"));
                assert!(!lengths.is_empty());
                assert!(lengths.iter().all(|len| *len == 0));
                drop(
                    sparq_lws_core::store::EmbeddedSparqClient::open_or_create(
                        &dir.0.join("index"),
                    )
                    .unwrap(),
                );
            }
        }
    }
    #[cfg(all(feature = "embedded-sparq", feature = "blob-fs"))]
    {
        assert!(dir.0.join("format.json").is_file());
        assert!(dir.0.join("index").is_dir());
        for path in ["blobs/objects", "blobs/pool", "blobs/tmp"] {
            assert!(dir.0.join(path).is_dir(), "missing {path}");
        }
    }
}

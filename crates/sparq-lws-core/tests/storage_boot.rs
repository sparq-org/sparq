//! Exercise native boot decisions through the real executable and literal environment names.
#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::process::{Directory, Process};

fn refuses(values: &[(&str, &str)], expected: &str) {
    let output = Process::start(env!("CARGO_BIN_EXE_sparq-lws-core"), values).finish();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "boot unexpectedly succeeded: {stderr}"
    );
    assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
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
    for signal in ["-TERM", "-INT"] {
        let mut process = Process::start(env!("CARGO_BIN_EXE_sparq-lws-core"), &[]);
        process.wait_signal_handlers();
        process.signal(signal);
        assert_eq!(process.finish().status.code(), Some(0));
    }
}

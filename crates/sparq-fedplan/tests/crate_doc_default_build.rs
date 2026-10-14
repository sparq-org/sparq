//! Regression guard for bead **sq-gxx7** (sibling of sq-tiq4 / PR #470): the crate-level
//! rustdoc must build cleanly in the DEFAULT (`fedplan` OFF) build under
//! `-D rustdoc::broken_intra_doc_links`.
//!
//! Why this exists: the crate-level narrative in `lib.rs` intra-doc-links items
//! (`SourceDescriptor` / `select_sources` / `plan_bgp` / `JoinTree` / `StreamJoin` / …) that
//! are EVERY one `#[cfg(feature = "fedplan")]`-gated, so in the default build none of them are
//! in scope. Before sq-gxx7, `cargo doc --no-deps -p sparq-fedplan` (default features) reported
//! 13 `unresolved link` errors under that lint; no CI lane builds docs, so `main` stayed green
//! while the default-build docs mis-rendered. The fix serves a concise, link-free crate doc in
//! the OFF build (and the rich linked one only when `fedplan` is ON, where the targets exist,
//! linking PUBLIC re-exports rather than the private `selection`/`plan`/`stream` modules) and
//! adds `#![deny(rustdoc::broken_intra_doc_links)]`.
//!
//! `broken_intra_doc_links` is a rustdoc-only lint — it fires under `cargo doc`, never under
//! `cargo build`/`clippy`/`test` — so the crate's `#![deny]` alone is not exercised by the
//! normal gates. This test shells out to `cargo doc` (the same shell-out style the wider
//! workspace uses for `cargo metadata`/`cargo` guard tests) so a regression is caught IN THE
//! TEST SUITE, not silently.
//!
//! sq-gxx7 — flagged for Fable re-review when available.

use std::path::PathBuf;
use std::process::Command;

/// Build the crate's own docs in the default (fedplan OFF) feature state with the
/// broken-intra-doc-links lint promoted to an error, and assert it succeeds.
///
/// Hermetic notes:
/// * `--no-deps` documents only this crate, so the run is fast and the assertion is about
///   THIS crate's doc comments, not a dependency's.
/// * A dedicated `--target-dir` keeps this nested `cargo doc` off the outer `cargo test`'s
///   build lock (no deadlock / no contention with the in-flight test build).
/// * `RUSTDOCFLAGS` carries the deny flag belt-and-braces alongside the crate's own
///   `#![deny(...)]`, so the test still fails loud even if the inner attribute is ever removed.
#[test]
fn crate_doc_builds_clean_in_default_off_build() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    // Isolate the doc build's target dir under the per-test temp dir so it does not race the
    // build lock the outer `cargo test` already holds on the shared target dir.
    let mut doc_target = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    doc_target.push("sq-gxx7-doc-guard");

    let output = Command::new(&cargo)
        .args([
            "doc",
            "--no-deps",
            "-p",
            "sparq-fedplan",
            // DEFAULT features only — this is the build the bug lived in. Do NOT pass
            // `--features fedplan`.
            "--target-dir",
        ])
        .arg(&doc_target)
        .env("RUSTDOCFLAGS", "-D rustdoc::broken_intra_doc_links")
        .output()
        .expect("failed to spawn `cargo doc`");

    assert!(
        output.status.success(),
        "BROKEN CRATE DOC (sq-gxx7 regression): `cargo doc --no-deps -p sparq-fedplan` in the \
         DEFAULT (fedplan OFF) build failed under `-D rustdoc::broken_intra_doc_links`. The \
         crate-level doc must not intra-doc-link `#[cfg(feature = \"fedplan\")]`-gated items in \
         the OFF build — gate the rich linked narrative behind `feature = \"fedplan\"` and keep \
         the OFF doc link-free.\n--- cargo doc stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

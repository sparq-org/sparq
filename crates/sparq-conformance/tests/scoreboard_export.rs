//! sq-gum8.14 — DRIFT GUARD for the committed machine-readable
//! conformance scoreboard, `bench/conformance-scoreboard.generated.json`.
//!
//! The committed artifact is a pure derivation of the central registry
//! (`scoreboard::SUITES` — the suite rows + ratchet floors), exported so
//! conformance-class paper-evidence bindings can point at suite rows / floors
//! by json-pointer instead of a Rust source anchor (the same
//! committed-generated-artifact pattern as `served-conformance.generated.json`,
//! `src/bin/served-conformance-report.rs`). The INVARIANT this test enforces is
//! result-equivalence: the committed bytes MUST equal a fresh regeneration from
//! the `SUITES` consts — any divergence (a floor raise, a new suite row, a note
//! edit, a stale hand-edit of the JSON) goes red here, so the JSON mirror the
//! evidence bindings reference can never silently drift from the Rust source of
//! truth.
//!
//! Hermetic and feature-independent: `scoreboard_json()` reads only the static
//! registry (no fetched suite data, no opt-in features), so this test behaves
//! identically in every feature state.
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

use sparq_conformance::scoreboard;
use std::path::PathBuf;

/// Repo-root path of the committed artifact (this crate lives at
/// `crates/sparq-conformance`, so the repo root is two levels up).
fn committed_artifact_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/conformance-scoreboard.generated.json")
}

const REGEN_CMD: &str = "cargo run -p sparq-conformance --bin sparq-conformance-scoreboard -- \
                         --report /tmp/conformance-scoreboard.md \
                         --json bench/conformance-scoreboard.generated.json";

#[test]
fn committed_scoreboard_json_byte_matches_regeneration() {
    let path = committed_artifact_path();
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "could not read the committed artifact {} ({e}) — regenerate it from the repo \
             root with: {REGEN_CMD}",
            path.display()
        )
    });
    let fresh = scoreboard::scoreboard_json();

    if committed != fresh {
        // Point at the first divergent line so a floor bump / row add is easy to
        // locate without diffing the whole document by hand.
        let first_divergence = committed
            .lines()
            .zip(fresh.lines())
            .enumerate()
            .find(|(_, (c, f))| c != f)
            .map(|(i, (c, f))| {
                format!(
                    "first divergent line {}:\n  committed: {c}\n  fresh:     {f}",
                    i + 1
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "one document is a prefix of the other (committed {} bytes, \
                     fresh {} bytes)",
                    committed.len(),
                    fresh.len()
                )
            });
        panic!(
            "bench/conformance-scoreboard.generated.json has DRIFTED from the registry \
             (scoreboard::SUITES in crates/sparq-conformance/src/scoreboard.rs).\n\
             {first_divergence}\n\
             The committed JSON mirror must byte-match a fresh regeneration — never \
             hand-edit it; regenerate from the repo root with: {REGEN_CMD}"
        );
    }
}

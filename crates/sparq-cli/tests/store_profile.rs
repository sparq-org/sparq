//! (sq-7d3dj.32.2.1) Contract tests for `SPARQ_STORE_PROFILE` — the runtime in-RAM
//! store-profile selection on sparq-cli's shared load path — and the re-landed `memstat`
//! subcommand.
//!
//! Why a *subprocess* test (like `cli_contract.rs`): the profile hook lives in the shared load
//! helper behind `fn main()`'s positional dispatch and is driven by an ENVIRONMENT VARIABLE, so
//! the only way to exercise the REAL path (env → `store_profile_from_env` → `into_compressed`) is
//! to spawn the built binary with the env var set. Each test builds a tiny self-contained fixture
//! under `CARGO_TARGET_TMPDIR`.
//!
//! The three load-bearing invariants (bead sq-7d3dj.32.2.1 ACCEPTANCE):
//!   1. `SPARQ_STORE_PROFILE=compressed` yields IDENTICAL query solutions to raw
//!      (result-equivalence) — the profile is a memory trade, never a semantic change.
//!   2. An unknown `SPARQ_STORE_PROFILE` value is a HARD ERROR (fail-closed, exit 2) — a typo
//!      must never silently fall through to raw.
//!   3. The `compressed` profile reports a LOWER store heap (`store_b_per_triple` via `memstat`)
//!      than raw on the same corpus — the memory saving actually happens.
//!
//! An extra invariant guards byte-identity of the default path: unset ≡ `raw`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh, unique scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("store-profile-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run the built binary with `args` and an explicit `SPARQ_STORE_PROFILE` value
/// (`None` leaves it UNSET, so the process inherits nothing — the default path).
fn run_profile(profile: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sparq-cli"));
    cmd.args(args);
    // Always clear any inherited value first so the harness is deterministic.
    cmd.env_remove("SPARQ_STORE_PROFILE");
    if let Some(p) = profile {
        cmd.env("SPARQ_STORE_PROFILE", p);
    }
    cmd.output().expect("spawning sparq-cli")
}

/// (exit code, stdout, stderr); a signal-terminated child reports code -1.
fn run3(profile: Option<&str>, args: &[&str]) -> (i32, String, String) {
    let out = run_profile(profile, args);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write fixture");
    p
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A small N-Triples fixture with enough distinct terms + rows that the block-compressed
/// perms measurably beat the raw six-permutation layout on `store_b_per_triple`. Each subject
/// links to two objects with two predicates → dense, lexicographically-ordered ids.
fn corpus() -> String {
    let mut nt = String::new();
    for i in 0..4000u32 {
        nt.push_str(&format!(
            "<http://ex/s{i}> <http://ex/knows> <http://ex/s{}> .\n",
            (i + 1) % 4000
        ));
        nt.push_str(&format!("<http://ex/s{i}> <http://ex/age> \"{}\" .\n", 20 + (i % 60)));
    }
    nt
}

/// Pull a `name\tvalue` line's value out of `memstat`'s TSV stdout.
fn memstat_field<'a>(stdout: &'a str, key: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('\t')))
        .unwrap_or_else(|| panic!("memstat output missing `{key}` line:\n{stdout}"))
}

// ---------------------------------------------------------------------------
// Invariant 1 — result-equivalence (raw vs compressed yield identical solutions)
// ---------------------------------------------------------------------------

#[test]
fn compressed_profile_yields_identical_solutions() {
    let dir = scratch("equiv");
    let data = write(&dir, "data.nt", &corpus());
    // A SELECT with an explicit ORDER BY so the row ORDER is deterministic across profiles —
    // then a byte-for-byte stdout comparison is a true result-equivalence check.
    let q = "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o } ORDER BY ?s ?o";

    let (c_raw, out_raw, e_raw) = run3(None, &["query", s(&data), "ntriples", q]);
    let (c_cmp, out_cmp, e_cmp) = run3(Some("compressed"), &["query", s(&data), "ntriples", q]);

    assert_eq!(c_raw, 0, "raw query failed: {e_raw}");
    assert_eq!(c_cmp, 0, "compressed query failed: {e_cmp}");
    assert_eq!(
        out_raw, out_cmp,
        "compressed profile changed the query solutions — result-equivalence violated"
    );
    // Sanity: the query actually returned rows (guards against both-empty vacuity).
    assert!(out_raw.contains("row(s)") && out_raw.contains("<http://ex/s0>"), "stdout: {out_raw}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explicit_raw_equals_unset_default() {
    // Byte-identity of the default path: `SPARQ_STORE_PROFILE=raw` must be indistinguishable
    // from leaving it unset.
    let dir = scratch("raw-default");
    let data = write(&dir, "data.nt", &corpus());
    let q = "SELECT ?s ?o WHERE { ?s <http://ex/knows> ?o } ORDER BY ?s ?o";

    let (_, out_unset, _) = run3(None, &["query", s(&data), "ntriples", q]);
    let (_, out_raw, _) = run3(Some("raw"), &["query", s(&data), "ntriples", q]);
    let (_, out_raw_case, _) = run3(Some("RAW"), &["query", s(&data), "ntriples", q]);

    assert_eq!(out_unset, out_raw, "explicit `raw` diverged from the unset default");
    assert_eq!(out_unset, out_raw_case, "`RAW` (case) diverged from the unset default");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Invariant 2 — fail-closed on an unknown profile value
// ---------------------------------------------------------------------------

#[test]
fn unknown_profile_fails_closed() {
    let dir = scratch("fail-closed");
    let data = write(&dir, "data.nt", &corpus());
    let q = "SELECT ?s WHERE { ?s <http://ex/knows> ?o }";

    // A plausible typo must NOT silently fall through to raw — it exits 2 with a message.
    for bogus in ["compresed", "compress", "gzip", "1", "true", "raw-ish"] {
        let (code, stdout, stderr) = run3(Some(bogus), &["query", s(&data), "ntriples", q]);
        assert_eq!(code, 2, "profile `{bogus}` should exit 2 (fail-closed), got {code}\nstderr: {stderr}");
        assert!(
            stderr.contains("SPARQ_STORE_PROFILE"),
            "the error should name the env var for `{bogus}`; stderr: {stderr}"
        );
        assert!(!stdout.contains("row(s)"), "no results should be emitted for `{bogus}`; stdout: {stdout}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Invariant 3 — the compressed profile actually reduces the reported store heap
// ---------------------------------------------------------------------------

#[test]
fn compressed_profile_reports_lower_store_heap() {
    let dir = scratch("heap");
    let data = write(&dir, "data.nt", &corpus());

    // `memstat` reports `store_b_per_triple` (the six-permutation heap over the triple count).
    let (c_raw, raw_out, e_raw) = run3(None, &["memstat", s(&data), "ntriples"]);
    let (c_cmp, cmp_out, e_cmp) = run3(Some("compressed"), &["memstat", s(&data), "ntriples"]);
    assert_eq!(c_raw, 0, "raw memstat failed: {e_raw}");
    assert_eq!(c_cmp, 0, "compressed memstat failed: {e_cmp}");

    // The `mode` line reflects the applied profile.
    assert_eq!(memstat_field(&raw_out, "mode"), "raw", "raw memstat mode line");
    assert_eq!(memstat_field(&cmp_out, "mode"), "compressed", "compressed memstat mode line");

    let raw_bpt: f64 = memstat_field(&raw_out, "store_b_per_triple").parse().expect("raw B/triple");
    let cmp_bpt: f64 = memstat_field(&cmp_out, "store_b_per_triple").parse().expect("compressed B/triple");
    assert!(
        cmp_bpt < raw_bpt,
        "compressed store_b_per_triple ({cmp_bpt}) should be lower than raw ({raw_bpt})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn memstat_positional_compressed_matches_env() {
    // The re-landed positional `compressed` flag and `SPARQ_STORE_PROFILE=compressed` select the
    // SAME mode (and, since memstat OR's them and applies `into_compressed` exactly once, the
    // reported store heap agrees).
    let dir = scratch("memstat-pos");
    let data = write(&dir, "data.nt", &corpus());

    let (c_pos, pos_out, e_pos) = run3(None, &["memstat", s(&data), "ntriples", "compressed"]);
    let (c_env, env_out, e_env) = run3(Some("compressed"), &["memstat", s(&data), "ntriples"]);
    assert_eq!(c_pos, 0, "positional-compressed memstat failed: {e_pos}");
    assert_eq!(c_env, 0, "env-compressed memstat failed: {e_env}");

    assert_eq!(memstat_field(&pos_out, "mode"), "compressed");
    assert_eq!(memstat_field(&env_out, "mode"), "compressed");
    assert_eq!(
        memstat_field(&pos_out, "store_b_per_triple"),
        memstat_field(&env_out, "store_b_per_triple"),
        "positional and env compression should report the same store heap (single encode)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn memstat_raw_emits_the_composition_breakdown() {
    // The re-landed `memstat` prints its deterministic `name\tvalue` composition lines.
    let dir = scratch("memstat-raw");
    let data = write(&dir, "data.nt", &corpus());
    let (code, stdout, stderr) = run3(None, &["memstat", s(&data), "ntriples"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for key in [
        "memstat_version",
        "mode",
        "triples",
        "heap_total_bytes",
        "heap_store_bytes",
        "store_b_per_triple",
        "dict_b_per_term",
    ] {
        assert!(stdout.contains(&format!("{key}\t")), "memstat missing `{key}`:\n{stdout}");
    }
    assert_eq!(memstat_field(&stdout, "memstat_version"), "1");
    let _ = std::fs::remove_dir_all(&dir);
}

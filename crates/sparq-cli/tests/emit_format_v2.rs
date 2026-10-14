//! (sq-kmve2) Contract tests for the SPQCPRM2 **emit flag** — `save …
//! compressed --format-v2` and `recompress … --v2`, the ergonomic replacement for exporting
//! `SPARQ_EMIT_FORMAT=v2` into the process environment.
//!
//! Why a *subprocess* test (like `store_profile.rs` / `cli_contract.rs`): the flag is parsed
//! inside `fn main()`'s positional dispatch and reports via `eprintln!`/`std::process::exit`,
//! and the thing it selects (the on-disk block-stream version) is only observable as bytes on
//! disk, so the only way to exercise the REAL path (argv → `take_emit_v2` →
//! `compress::with_emit_format` → `save_compressed`) is to run the built binary and read the
//! 8-byte file magic of the permutations it wrote.
//!
//! The invariants, in both feature states:
//!   1. **Default is unchanged.** No flag ⇒ every compressed perm still carries the `SPQCPRM1`
//!      magic, byte-for-byte as before this change.
//!   2. **Fail-closed, never silently-V1.** `--format-v2` without the `compressed` positional,
//!      an unknown `--flag`, and (on a build without the opt-in `spqcprm2` feature) the flag
//!      itself are all hard usage errors (exit 2) — the one outcome worse than an error here is
//!      writing the *other* format while the caller believes they got V2.
//!
//! With `--features spqcprm2` additionally:
//!   3. **The flag actually emits V2** from both `save` and `recompress` (`SPQCPRM2` magic), and
//!      it BEATS the `SPARQ_EMIT_FORMAT` env var (the per-thread override wins).
//!   4. **Result-equivalence:** a V2 directory answers a query identically to a V1 one — the
//!      format is a byte-layout trade, never a semantic change.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh, unique scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("emit-v2-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

const NT: &str = "\
<http://ex/alice> <http://ex/knows> <http://ex/bob> .
<http://ex/alice> <http://ex/age> \"30\" .
<http://ex/bob> <http://ex/age> \"25\" .
";

fn write_nt(dir: &Path) -> PathBuf {
    let p = dir.join("data.nt");
    std::fs::write(&p, NT).expect("write fixture");
    p
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

/// Run the built binary with `args` and an explicit `SPARQ_EMIT_FORMAT` value (`None` leaves it
/// UNSET, so the child inherits nothing — the default emit path).
fn run_emit(env: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sparq-cli"));
    cmd.args(args);
    // Always clear any inherited value first so the harness is deterministic.
    cmd.env_remove("SPARQ_EMIT_FORMAT");
    if let Some(v) = env {
        cmd.env("SPARQ_EMIT_FORMAT", v);
    }
    cmd.output().expect("spawning sparq-cli")
}

/// (exit code, stdout, stderr); a signal-terminated child reports code -1.
fn run3(env: Option<&str>, args: &[&str]) -> (i32, String, String) {
    let out = run_emit(env, args);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The 8-byte file magic of every BLOCK-COMPRESSED permutation in `dir`. Unbuilt (empty) perms
/// are written raw-empty and carry no magic, so they are skipped; the returned vec must be
/// non-empty for the assertion to mean anything (checked by the callers).
fn perm_magics(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..6 {
        let path = dir.join(format!("perm{i}.bin"));
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if bytes.len() < 8 {
            continue;
        }
        let magic = String::from_utf8_lossy(&bytes[..8]).into_owned();
        if magic.starts_with("SPQCPRM") {
            out.push(magic);
        }
    }
    out
}

/// Asserts every compressed perm in `dir` carries `expect`, and that there was at least one.
fn assert_all_magics(dir: &Path, expect: &str) {
    let magics = perm_magics(dir);
    assert!(!magics.is_empty(), "no block-compressed perm written to {}", dir.display());
    for m in &magics {
        assert_eq!(m, expect, "perm magic in {}: {:?}", dir.display(), magics);
    }
}

// ---------------------------------------------------------------------------
// 1. The default path is unchanged (both feature states).
// ---------------------------------------------------------------------------

#[test]
fn save_compressed_without_flag_still_emits_v1() {
    let dir = scratch("default-v1");
    let data = write_nt(&dir);
    let idx = dir.join("idx");

    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples", s(&idx), "compressed"]);
    assert_eq!(code, 0, "save stderr: {err}");
    assert_all_magics(&idx, "SPQCPRM1");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Fail-closed usage errors (both feature states).
// ---------------------------------------------------------------------------

#[test]
fn format_v2_without_compressed_exits_2() {
    let dir = scratch("no-compressed");
    let data = write_nt(&dir);
    let idx = dir.join("idx");

    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples", s(&idx), "--format-v2"]);
    assert_eq!(code, 2, "raw perms have no block-stream version: {err}");
    assert!(err.contains("compressed"), "stderr: {err}");
    assert!(!idx.exists(), "the flag must be rejected BEFORE anything is written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_flag_exits_2() {
    let dir = scratch("unknown-flag");
    let data = write_nt(&dir);
    let idx = dir.join("idx");

    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples", s(&idx), "compressed", "--frmat-v2"]);
    assert_eq!(code, 2, "a typo'd flag must never be silently ignored: {err}");
    assert!(err.contains("unknown flag --frmat-v2"), "stderr: {err}");

    let (rcode, _rout, rerr) = run3(None, &["recompress", s(&idx), s(&dir.join("dst")), "--vv2"]);
    assert_eq!(rcode, 2, "recompress stderr: {rerr}");
    assert!(rerr.contains("unknown flag --vv2"), "stderr: {rerr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pre-existing usage contracts (`error_paths_cli` / `cli_contract`) must survive the new
/// flag parsing: too-few positionals and `src == dst` are still exit-2 with the same shapes.
#[test]
fn existing_usage_contracts_survive_flag_parsing() {
    let dir = scratch("usage-shapes");
    let data = write_nt(&dir);

    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples"]);
    assert_eq!(code, 2);
    assert!(err.contains("usage: sparq-cli save"), "stderr: {err}");

    let same = dir.join("same");
    let (rcode, _rout, rerr) = run3(None, &["recompress", s(&same), s(&same)]);
    assert_eq!(rcode, 2);
    assert!(rerr.contains("dirs must differ"), "stderr: {rerr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2b. Feature OFF: the flag is a loud error, never a silent SPQCPRM1 write.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "spqcprm2"))]
#[test]
fn flag_without_the_feature_exits_2() {
    let dir = scratch("no-feature");
    let data = write_nt(&dir);
    let idx = dir.join("idx");

    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples", s(&idx), "compressed", "--format-v2"]);
    assert_eq!(code, 2, "a build without `spqcprm2` cannot emit V2: {err}");
    assert!(err.contains("spqcprm2"), "the error must name the missing feature: {err}");
    assert!(!idx.exists(), "nothing may be written on the rejected path");

    // Same for `recompress`, on a directory that really exists (so this is the FLAG failing,
    // not the open).
    let raw = dir.join("raw");
    assert_eq!(run3(None, &["save", s(&data), "ntriples", s(&raw)]).0, 0);
    let cmp = dir.join("cmp");
    let (rcode, _rout, rerr) = run3(None, &["recompress", s(&raw), s(&cmp), "--v2"]);
    assert_eq!(rcode, 2, "recompress stderr: {rerr}");
    assert!(rerr.contains("spqcprm2"), "stderr: {rerr}");
    assert!(!cmp.exists(), "nothing may be written on the rejected path");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3/4. Feature ON: the flag emits V2, beats the env var, and preserves results.
// ---------------------------------------------------------------------------

#[cfg(feature = "spqcprm2")]
#[test]
fn save_format_v2_emits_v2_and_queries_identically() {
    let dir = scratch("save-v2");
    let data = write_nt(&dir);
    let v1 = dir.join("v1");
    let v2 = dir.join("v2");

    assert_eq!(run3(None, &["save", s(&data), "ntriples", s(&v1), "compressed"]).0, 0);
    let (code, _out, err) = run3(None, &["save", s(&data), "ntriples", s(&v2), "compressed", "--format-v2"]);
    assert_eq!(code, 0, "save stderr: {err}");
    assert!(err.contains("SPQCPRM2"), "the report must name the format written: {err}");

    assert_all_magics(&v1, "SPQCPRM1");
    assert_all_magics(&v2, "SPQCPRM2");

    // Result-equivalence: the V2 directory answers the query exactly as the V1 one does.
    let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o } ORDER BY ?s ?p ?o";
    let (c1, o1, _) = run3(None, &["query-mmap", s(&v1), q]);
    let (c2, o2, e2) = run3(None, &["query-mmap", s(&v2), q]);
    assert_eq!((c1, c2), (0, 0), "query-mmap stderr: {e2}");
    assert_eq!(o1, o2, "SPQCPRM2 must be result-equivalent to SPQCPRM1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "spqcprm2")]
#[test]
fn recompress_v2_emits_v2_and_queries_identically() {
    let dir = scratch("recompress-v2");
    let data = write_nt(&dir);
    let raw = dir.join("raw");
    let v1 = dir.join("v1");
    let v2 = dir.join("v2");

    assert_eq!(run3(None, &["save", s(&data), "ntriples", s(&raw)]).0, 0);
    assert_eq!(run3(None, &["recompress", s(&raw), s(&v1)]).0, 0);
    let (code, _out, err) = run3(None, &["recompress", s(&raw), s(&v2), "--v2"]);
    assert_eq!(code, 0, "recompress stderr: {err}");
    assert!(err.contains("SPQCPRM2"), "the report must name the format written: {err}");

    assert_all_magics(&v1, "SPQCPRM1");
    assert_all_magics(&v2, "SPQCPRM2");

    let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o } ORDER BY ?s ?p ?o";
    let (c1, o1, _) = run3(None, &["query-mmap", s(&v1), q]);
    let (c2, o2, e2) = run3(None, &["query-mmap", s(&v2), q]);
    assert_eq!((c1, c2), (0, 0), "query-mmap stderr: {e2}");
    assert_eq!(o1, o2, "SPQCPRM2 must be result-equivalent to SPQCPRM1");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The flag is the PER-THREAD override, so it wins over `SPARQ_EMIT_FORMAT` — and the env var
/// still works on its own, unchanged, for callers that already use it.
#[cfg(feature = "spqcprm2")]
#[test]
fn flag_beats_env_var_and_env_var_still_works() {
    let dir = scratch("flag-vs-env");
    let data = write_nt(&dir);

    // Env alone selects V2 (the pre-existing path).
    let env_only = dir.join("env-only");
    let (c, _o, e) = run3(Some("v2"), &["save", s(&data), "ntriples", s(&env_only), "compressed"]);
    assert_eq!(c, 0, "save stderr: {e}");
    assert_all_magics(&env_only, "SPQCPRM2");

    // Flag wins over an env value that does NOT select V2.
    let flag_wins = dir.join("flag-wins");
    let (c, _o, e) = run3(Some("v1"), &["save", s(&data), "ntriples", s(&flag_wins), "compressed", "--format-v2"]);
    assert_eq!(c, 0, "save stderr: {e}");
    assert_all_magics(&flag_wins, "SPQCPRM2");
    let _ = std::fs::remove_dir_all(&dir);
}

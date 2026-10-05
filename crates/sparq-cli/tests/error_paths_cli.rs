//! [GPT-5.6] Exit-code contracts for previously uncovered CLI usage and I/O failures.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(args)
        .output()
        .expect("spawning sparq-cli")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("error-paths-cli-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_nt(dir: &Path) -> PathBuf {
    let path = dir.join("data.nt");
    std::fs::write(&path, "<http://ex/s> <http://ex/p> <http://ex/o> .\n")
        .expect("write N-Triples fixture");
    path
}

fn s(path: &Path) -> &str {
    path.to_str().expect("UTF-8 fixture path")
}

fn assert_contract(args: &[&str], expected_exit: i32, stderr_substring: &str) {
    let output = run(args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(expected_exit),
        "argv {args:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains(stderr_substring),
        "argv {args:?}; expected stderr to contain {stderr_substring:?}; stderr: {stderr}"
    );
}

#[test]
fn build_nonexistent_input_exits_1() {
    let dir = scratch("build-nonexistent-input");
    let output_dir = dir.join("store");
    assert_contract(
        &["build", "/no/such/file.nt", "ntriples", s(&output_dir)],
        1,
        "open /no/such/file.nt",
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn query_format_without_value_exits_2() {
    let dir = scratch("query-format-without-value");
    let data = write_nt(&dir);
    assert_contract(
        &["query", s(&data), "ntriples", "ASK {}", "--format"],
        2,
        "--format needs a value",
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recompress_nonexistent_source_exits_1() {
    let dir = scratch("recompress-nonexistent-source");
    let output_dir = dir.join("store");
    assert_contract(
        &["recompress", "/no/such/src", s(&output_dir)],
        1,
        "open error",
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn compact_without_directory_exits_2() {
    assert_contract(&["compact"], 2, "usage: sparq-cli compact");
}

#[test]
fn compact_nonexistent_directory_exits_1() {
    assert_contract(&["compact", "/no/such/dir"], 1, "open error");
}

#[test]
fn save_too_few_arguments_exits_2() {
    let dir = scratch("save-too-few-arguments");
    let data = write_nt(&dir);
    assert_contract(&["save", s(&data), "ntriples"], 2, "usage: sparq-cli save");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn query_mmap_without_arguments_exits_2() {
    assert_contract(&["query-mmap"], 2, "usage: sparq-cli query-mmap");
}

#[test]
fn reason_too_few_arguments_exits_2() {
    let dir = scratch("reason-too-few-arguments");
    let data = write_nt(&dir);
    assert_contract(
        &["reason", s(&data), "ntriples"],
        2,
        "usage: sparq-cli reason",
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// [OPUS-5] (sq-2ch27) The feature-OFF half of the `--reason el` contract: a build without the
/// opt-in `el` feature must FAIL LOUDLY naming the feature, never silently fall back to `owl`
/// (which is sound but INCOMPLETE for class classification). The feature-ON half lives in
/// `el_cli.rs`. Gated `not(feature = "el")` so the `sparq-cli (el)` matrix leg does not run it.
#[cfg(not(feature = "el"))]
#[test]
fn reason_el_without_the_feature_exits_2() {
    let dir = scratch("reason-el-without-feature");
    let data = write_nt(&dir);
    assert_contract(
        &["reason", s(&data), "ntriples", "el"],
        2,
        "opt-in `el` cargo feature",
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// [OPUS-5] (sq-2ch27) …and the `classify` subcommand is simply ABSENT without the feature, so
/// it falls through to the top-level usage block (exit 2) rather than a half-wired stub.
#[cfg(not(feature = "el"))]
#[test]
fn classify_subcommand_absent_without_the_feature() {
    assert_contract(&["classify"], 2, "usage:\n  sparq-cli query");
}

/// [SONNET-4.6] (sq-p4zci) The feature-OFF half of the `--reason datalog:<rules.dlog>` contract:
/// a build without the opt-in `datalog` feature must FAIL LOUDLY naming the feature. There is no
/// fall-back profile at all here — RDFS/OWL-RL are monotone, so quietly running one of them
/// would drop negation as failure and aggregation and change the answer set. The feature-ON half
/// lives in `datalog_cli.rs`. Gated `not(feature = "datalog")` so the `sparq-cli (datalog)`
/// matrix leg does not run it.
#[cfg(not(feature = "datalog"))]
#[test]
fn reason_datalog_without_the_feature_exits_2() {
    let dir = scratch("reason-datalog-without-feature");
    let data = write_nt(&dir);
    assert_contract(
        &["reason", s(&data), "ntriples", "datalog:rules.dlog"],
        2,
        "opt-in `datalog` cargo feature",
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// [SONNET-4.6] (sq-p4zci) A bare `datalog` with no rules file is a USAGE error naming the
/// syntax, in BOTH feature states — the rules-file argument is not optional, and falling through
/// to the profile parser would report the far less useful "unknown reasoning profile". The
/// argument check runs before the feature check, so this assertion is feature-independent.
#[test]
fn reason_datalog_without_a_rules_file_exits_2() {
    let dir = scratch("reason-datalog-no-rules");
    let data = write_nt(&dir);
    assert_contract(
        &["reason", s(&data), "ntriples", "datalog"],
        2,
        "--reason datalog:<rules.dlog>",
    );
    let _ = std::fs::remove_dir_all(dir);
}

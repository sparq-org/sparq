//! (sq-vczh2, epic sq-2m6zm) The `terse <terse-query>` subcommand end-to-end.
//!
//! Spawns the *built* `sparq-cli` binary (via `CARGO_BIN_EXE_sparq-cli`, the same mechanism as
//! `dump_cli.rs` / `hdt_cli.rs`). The `terse` subcommand lives behind the opt-in `terse` feature,
//! so this whole file compiles and runs only under it:
//! `cargo test -p sparq-cli --features terse`.
#![cfg(feature = "terse")]

use std::io::Write;
use std::process::{Command, Stdio};

/// (exit code, stdout, stderr) from the built binary.
fn run3(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(args)
        .output()
        .expect("spawning sparq-cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `terse` with a `K:<name>` keyword expands it to the canonical IRI and echoes the expansion —
/// the actual transpiler runs (not a stub).
#[test]
fn keyword_expands_to_canonical_sparql() {
    let (code, out, err) = run3(&["terse", "SELECT ?s WHERE { ?s K:derivedFrom ?o }"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("<http://www.w3.org/ns/prov#wasDerivedFrom>"),
        "canonical SPARQL should carry the expanded IRI: {out}"
    );
    assert!(out.contains("\"keyword\":\"derivedFrom\""), "{out}");
    assert!(out.contains("\"legendVersion\":\"pkg-keywords/v1\""), "{out}");
    // The lean CLI build has no V() resolution, so resolutions is always empty.
    assert!(out.contains("\"resolutions\":[]"), "{out}");
}

/// Canonical SPARQL with no terse token round-trips unchanged with empty keywords.
#[test]
fn identity_passthrough() {
    let (code, out, err) = run3(&["terse", "SELECT ?s WHERE { ?s ?p ?o }"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("SELECT ?s WHERE { ?s ?p ?o }"), "{out}");
    assert!(out.contains("\"keywords\":[]"), "{out}");
}

/// An unknown keyword is a loud failure (non-zero exit, message on stderr) — never a silent guess.
#[test]
fn unknown_keyword_loud_fails() {
    let (code, out, err) = run3(&["terse", "SELECT ?s WHERE { ?s K:bogus ?o }"]);
    assert_eq!(code, 2, "stdout: {out}");
    assert!(err.contains("unknown keyword"), "stderr: {err}");
}

/// A `V("phrase")` construct loud-fails in the lean (no-`vectors`) CLI build (the V() ambiguity
/// caveat is tracked by sq-26fdp).
#[test]
fn v_construct_loud_fails_in_lean_build() {
    let (code, out, err) = run3(&["terse", "SELECT ?s WHERE { ?s ?p V(\"some concept\") }"]);
    assert_eq!(code, 2, "stdout: {out}");
    assert!(err.contains("vectors"), "stderr: {err}");
}

/// `terse -` reads the query from stdin (so a shell-hostile query can be piped).
#[test]
fn reads_query_from_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(["terse", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning sparq-cli");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"SELECT ?s WHERE { ?s K:type ?o }")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>"),
        "{stdout}"
    );
}

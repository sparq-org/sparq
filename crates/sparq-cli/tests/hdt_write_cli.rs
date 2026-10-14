//! (sq-8ju74) End-to-end tests for the `to-hdt` EXPORT subcommand: the actual
//! sparq-cli binary exporting a loaded RDF document as a standard-layout HDT archive, then
//! loading that archive back through its own `query` HDT path. Compiled and run only with
//! the opt-in `hdt-write` cargo feature (which implies `hdt`, the loader):
//! `cargo test -p sparq-cli --features hdt-write`.
#![cfg(feature = "hdt-write")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A small term zoo exercising the HDT dictionary sections: IRIs, a shared
/// subject+object term, a blank-node subject, and plain / language-tagged /
/// datatyped literals. 6 triples.
const ZOO_NT: &str = "\
<http://example.org/s1> <http://example.org/p> <http://example.org/o1> .\n\
<http://example.org/s1> <http://example.org/p> \"plain\" .\n\
<http://example.org/s1> <http://example.org/p> \"hallo\"@de .\n\
<http://example.org/s1> <http://example.org/p> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
<http://example.org/s2> <http://example.org/q> <http://example.org/s1> .\n\
_:b1 <http://example.org/p> <http://example.org/s1> .\n";

/// TriG dataset: ONE default-graph triple + a named graph carrying TWO more. HDT has no
/// place for named graphs, so `to-hdt` must write only the default graph and say so.
const DATASET_TRIG: &str = "\
<http://example.org/s0> <http://example.org/p> <http://example.org/o0> .\n\
<http://example.org/g1> {\n\
  <http://example.org/s1> <http://example.org/p> <http://example.org/o1> .\n\
  <http://example.org/s2> <http://example.org/p> <http://example.org/o2> .\n\
}\n";

const COUNT_ALL: &str = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";

/// A FRESH per-test scratch dir. `CARGO_TARGET_TMPDIR` persists across `cargo test`
/// invocations, so a leftover archive from a previous run would satisfy the reload
/// assertions even if `to-hdt` stopped writing anything (a stale-fixture vacuity this
/// suite caught once — the mutation witness only goes red because of this cleanup).
fn tmpdir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (std::process::ExitStatus, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(args)
        .output()
        .expect("spawning sparq-cli");
    (
        out.status,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Full loop through the real binary: N-Triples -> `to-hdt` -> `.hdt` -> `query` (the
/// existing HDT loader hook) must see EXACTLY the original triple count. The footer
/// assertion pins the same `(<N> row(s))` contract hdt_cli.rs / cli_contract.rs pin.
#[test]
fn to_hdt_roundtrips_through_query() {
    let dir = tmpdir("to-hdt-roundtrip");
    let nt = dir.join("zoo.nt");
    std::fs::write(&nt, ZOO_NT).unwrap();
    let hdt = dir.join("zoo.hdt");

    let (status, _, stderr) = run(&["to-hdt", nt.to_str().unwrap(), "ntriples", hdt.to_str().unwrap()]);
    assert!(status.success(), "to-hdt failed: {stderr}");
    assert!(
        stderr.contains("wrote 6 triples"),
        "summary line must report the written triple count: {stderr}"
    );
    // A pure-triples input has nothing to drop — the named-graph warning must NOT fire.
    assert!(!stderr.contains("warning:"), "spurious warning on a triples-only input: {stderr}");

    let (status, stdout, stderr) = run(&["query", hdt.to_str().unwrap(), "hdt", COUNT_ALL]);
    assert!(status.success(), "query over the exported archive failed: {stderr}");
    assert!(stdout.contains("(6 row(s))"), "stdout: {stdout}");
}

/// The output container is chosen by the OUTPUT extension: `.hdt.gz` must be genuinely
/// gzip-wrapped (magic bytes — not just a renamed bare archive) and still load back
/// through the reader's magic-byte sniffing.
#[test]
fn to_hdt_gz_extension_writes_gzip_container() {
    let dir = tmpdir("to-hdt-gz");
    let nt = dir.join("zoo.nt");
    std::fs::write(&nt, ZOO_NT).unwrap();
    let gz = dir.join("zoo.hdt.gz");

    let (status, _, stderr) = run(&["to-hdt", nt.to_str().unwrap(), "ntriples", gz.to_str().unwrap()]);
    assert!(status.success(), "to-hdt failed: {stderr}");

    let bytes = std::fs::read(&gz).unwrap();
    assert!(
        bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b,
        "output named .hdt.gz must actually be gzip (magic 1f 8b), got {:02x?}",
        &bytes[..2.min(bytes.len())]
    );

    let (status, stdout, stderr) = run(&["query", gz.to_str().unwrap(), "hdt", COUNT_ALL]);
    assert!(status.success(), "query over the gzipped archive failed: {stderr}");
    assert!(stdout.contains("(6 row(s))"), "stdout: {stdout}");
}

/// HDT carries a single default graph: a TriG dataset's named graphs are DROPPED from
/// the archive — loudly (a stderr warning with the dropped counts), never silently. The
/// re-loaded archive must hold ONLY the default-graph triple.
#[test]
fn to_hdt_drops_named_graphs_loudly() {
    let dir = tmpdir("to-hdt-named");
    let trig = dir.join("dataset.trig");
    std::fs::write(&trig, DATASET_TRIG).unwrap();
    let hdt = dir.join("dataset.hdt");

    let (status, _, stderr) = run(&["to-hdt", trig.to_str().unwrap(), "trig", hdt.to_str().unwrap()]);
    assert!(status.success(), "to-hdt failed: {stderr}");
    assert!(
        stderr.contains("dropping 1 named graph(s) (2 triple(s))"),
        "must warn with the dropped graph/triple counts: {stderr}"
    );

    let (status, stdout, stderr) = run(&["query", hdt.to_str().unwrap(), "hdt", COUNT_ALL]);
    assert!(status.success(), "query over the exported archive failed: {stderr}");
    assert!(stdout.contains("(1 row(s))"), "only the default graph is written: {stdout}");
}

/// Missing positional arguments are a usage error: exit 2 (the CLI-wide contract), with
/// the usage line on stderr.
#[test]
fn to_hdt_usage_error_exits_2() {
    let (status, _, stderr) = run(&["to-hdt", "only-one-arg"]);
    assert_eq!(status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("usage: sparq-cli to-hdt"), "stderr: {stderr}");
}

/// An unknown input format must die loudly (exit 2) BEFORE writing anything — the shared
/// load path's unknown-format guard (bug sq-q50l) applies to the export path too.
#[test]
fn to_hdt_unknown_format_exits_2_and_writes_nothing() {
    let dir = tmpdir("to-hdt-badfmt");
    let nt = dir.join("zoo.nt");
    std::fs::write(&nt, ZOO_NT).unwrap();
    let hdt = dir.join("never.hdt");

    let (status, _, _) = run(&["to-hdt", nt.to_str().unwrap(), "no-such-format", hdt.to_str().unwrap()]);
    assert_eq!(status.code(), Some(2));
    assert!(!hdt.exists(), "no output file may be created on a usage error");
}

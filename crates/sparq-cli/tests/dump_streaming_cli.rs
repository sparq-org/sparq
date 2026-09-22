//! [FABLE-5] (sq-0kq6k) `dump <file> <in> turtle|trig` under the opt-in
//! `streaming-serialization` feature: the document is written STRAIGHT to stdout through the
//! engine's streaming writers instead of being built as one rendered `String` first.
//!
//! The whole point of the feature is that this is a MEMORY-SHAPE change and nothing else, so
//! the tests here are byte-equality tests: the streamed stdout must equal what the buffered
//! writer (`sparq_engine::serialize::graph_to_turtle` / `graph_to_trig`, linked directly) would
//! have produced for the same input. If the two ever diverge the feature is a silent output
//! regression, and that is exactly what these catch.
//!
//! Compiled and run only under the feature:
//! `cargo test -p sparq-cli --features streaming-serialization`.
#![cfg(feature = "streaming-serialization")]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A fresh scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("dump-streaming-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// stdout from the built binary; a non-zero exit fails the test loudly.
fn dump(path: &Path, in_fmt: &str, out_fmt: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(["dump", path.to_str().unwrap(), in_fmt, out_fmt])
        .output()
        .expect("spawning sparq-cli");
    assert!(
        out.status.success(),
        "dump … {out_fmt} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("dump output must be UTF-8")
}

/// A dataset with a default graph and two named graphs, mixing prefix-compactable IRIs,
/// `rdf:type` (which the Turtle writer abbreviates to `a`), a language-tagged literal and a
/// typed literal — the cells where a streamed and a buffered writer could disagree.
const DATASET_TRIG: &str = "\
@prefix ex: <http://example.com/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:alice a foaf:Person ; foaf:name \"Alice\"@en ; foaf:age \"30\"^^xsd:integer .
ex:bob a foaf:Person ; foaf:knows ex:alice .
ex:g1 {
  ex:x ex:p ex:y .
  ex:x ex:q \"literal with \\\"quotes\\\" and \\\\ backslash\" .
}
ex:g2 {
  ex:z ex:p \"tail\" .
}
";

/// Enough subjects that the writer crosses many subject-block boundaries — the seam where a
/// streaming writer that buffered its state wrongly would show up.
fn wide_ttl() -> String {
    let mut ttl = String::from("@prefix ex: <http://example.com/> .\n");
    for i in 0..2000 {
        ttl.push_str(&format!("ex:s{i} ex:p \"value-{i}\" ; ex:q ex:o{i} .\n"));
    }
    ttl
}

/// Loads the fixture exactly the way the CLI's `load_quiet` does — TriG / N-Quads through
/// `load_dataset` so named graphs survive, everything else through `load_str` (fold them into
/// the default graph and the oracle would disagree with the binary for the wrong reason).
fn load(doc: &str, in_fmt: &str) -> sparq_core::Graph {
    if matches!(in_fmt, "trig" | "nquads" | "n-quads") {
        sparq_core::Graph::load_dataset(doc, in_fmt).expect("fixture must parse")
    } else {
        sparq_core::Graph::load_str(doc, in_fmt).expect("fixture must parse")
    }
}

/// Renders the fixture with the BUFFERED writer — the oracle the streamed stdout must match.
fn buffered(doc: &str, in_fmt: &str, out_fmt: &str) -> String {
    let g = load(doc, in_fmt);
    match out_fmt {
        "trig" => sparq_engine::serialize::graph_to_trig(&g),
        _ => sparq_engine::serialize::graph_to_turtle(&g),
    }
}

fn fixture(tag: &str, doc: &str, ext: &str) -> PathBuf {
    let dir = scratch(tag);
    let p = dir.join(format!("in.{ext}"));
    std::fs::write(&p, doc).expect("write fixture");
    p
}

#[test]
fn streamed_turtle_dump_equals_the_buffered_writer() {
    let ttl = wide_ttl();
    let p = fixture("turtle", &ttl, "ttl");
    assert_eq!(dump(&p, "turtle", "turtle"), buffered(&ttl, "turtle", "turtle"));
}

#[test]
fn streamed_turtle_dump_handles_the_term_hazard_cells() {
    let p = fixture("turtle-hazards", DATASET_TRIG, "trig");
    // `turtle` emits the DEFAULT graph only (named graphs need a dataset format).
    assert_eq!(dump(&p, "trig", "turtle"), buffered(DATASET_TRIG, "trig", "turtle"));
}

#[test]
fn the_ttl_alias_streams_the_same_bytes() {
    let ttl = wide_ttl();
    let p = fixture("ttl-alias", &ttl, "ttl");
    assert_eq!(dump(&p, "turtle", "ttl"), dump(&p, "turtle", "turtle"));
}

#[test]
fn streamed_trig_dump_equals_the_buffered_writer() {
    let p = fixture("trig", DATASET_TRIG, "trig");
    let streamed = dump(&p, "trig", "trig");
    assert_eq!(streamed, buffered(DATASET_TRIG, "trig", "trig"));
    // Sanity that the fixture really exercised the named-graph framing.
    assert!(streamed.contains("GRAPH"), "the TriG dump must carry the named graphs: {streamed}");
}

#[test]
fn streamed_trig_dump_over_many_subjects_equals_the_buffered_writer() {
    let ttl = wide_ttl();
    let p = fixture("trig-wide", &ttl, "ttl");
    assert_eq!(dump(&p, "turtle", "trig"), buffered(&ttl, "turtle", "trig"));
}

/// The streamed document is still a valid RDF document: re-parse it and require the same
/// triple count, so a byte-equal-but-both-wrong writer pair cannot pass silently.
#[test]
fn the_streamed_turtle_dump_reparses_to_the_same_triples() {
    let ttl = wide_ttl();
    let p = fixture("turtle-roundtrip", &ttl, "ttl");
    let source = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
    let back = sparq_core::Graph::load_str(&dump(&p, "turtle", "turtle"), "turtle").unwrap();
    assert_eq!(back.len(), source.len());
}

/// A closed downstream pipe (the `dump … | head` shape) is a NORMAL exit, not a panic.
///
/// This is the one externally visible behaviour difference the feature makes, and it is a real
/// improvement: the buffered `print!` path panics with "failed printing to stdout: Broken pipe"
/// and exits 101, because `print!` unwraps its write. The streaming path classifies
/// `ErrorKind::BrokenPipe` and returns quietly — exit 0, nothing on stderr.
///
/// Built WITHOUT a shell: `head` is emulated by reading a 64-byte prefix and then dropping the
/// read end, which is what makes the child's next write fail with `EPIPE`. A shell pipeline
/// would have had to smuggle the child's status back out through `PIPESTATUS` (bash-only; `sh`
/// is dash on Debian/Ubuntu, where it is a fatal "Bad substitution" that never runs the CLI at
/// all), so the status assertion below is only trustworthy because the pipe is built here.
///
/// Deterministic at this fixture size: a 2000-subject document is far larger than the pipe
/// buffer, so the child is still writing — and therefore still has a write left to fail — when
/// the read end closes.
#[test]
fn a_closed_downstream_pipe_is_a_quiet_exit() {
    let ttl = wide_ttl();
    let p = fixture("broken-pipe", &ttl, "ttl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(["dump", p.to_str().unwrap(), "turtle", "turtle"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning sparq-cli");

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut head = [0u8; 64];
    stdout.read_exact(&mut head).expect("the first bytes of the dump must arrive");
    drop(stdout); // the `| head` moment: the reader goes away mid-document.

    let out = child.wait_with_output().expect("waiting for sparq-cli");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "a closed pipe must be a quiet exit 0, got {:?}: {}",
        out.status.code(),
        stderr
    );
    assert!(stderr.is_empty(), "a closed pipe must say nothing on stderr: {}", stderr);
}

/// Formats WITHOUT a streaming writer are untouched by the feature — `nquads` still goes
/// through the buffered arm and prints the same bytes it always did.
#[test]
fn a_non_streaming_out_format_is_unaffected() {
    let p = fixture("nquads", DATASET_TRIG, "trig");
    let g = load(DATASET_TRIG, "trig");
    assert_eq!(dump(&p, "trig", "nquads"), sparq_engine::serialize::graph_to_nquads(&g));
}

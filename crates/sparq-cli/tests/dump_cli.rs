//! (sq-oy1f.5) The `dump <file> <in> <out>` re-serializer end-to-end, with a
//! focus on the new `jsonld-compact` out-format (full W3C JSON-LD 1.1 Compaction against a
//! caller `@context` passed via `--context <file>`).
//!
//! Spawns the *built* `sparq-cli` binary (via `CARGO_BIN_EXE_sparq-cli`, the same mechanism
//! as `hdt_cli.rs` / `cli_contract.rs`). The `dump` subcommand lives behind the opt-in
//! `serialize-rdf` feature, so this whole file compiles and runs only under it:
//! `cargo test -p sparq-cli --features serialize-rdf`.
#![cfg(feature = "serialize-rdf")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("dump-cli-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write fixture");
    p
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

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

/// A small `schema.org` graph so an `@vocab` context can compact every predicate to a bare
/// term (which the prefix-only `jsonld-compacted` form cannot do).
const TTL: &str = "\
@prefix s: <http://schema.org/> .
s:alice s:name \"Alice\" ; s:knows s:bob .
";

/// (sq-g3pji) Dataset fixture for textual dump round-trips: two default-graph
/// triples, three in `ex:g1`, and one in `ex:g2`. The blank node ensures the invariant is
/// about counts rather than serializer-specific blank-node labels.
const DATASET_TRIG: &str = "\
@prefix ex: <http://example.com/> .
ex:d0 ex:p \"default one\" .
ex:d1 ex:p ex:o .
ex:g1 {
  ex:a ex:p ex:b .
  ex:b ex:p \"named two\" .
  _:local ex:p ex:c .
}
ex:g2 { ex:x ex:p ex:y . }
";

/// Run a non-aggregate SELECT through the mmap index and return its solution count.
fn query_mmap_count(index: &Path, query: &str) -> usize {
    let (code, out, err) = run3(&["query-mmap", s(index), query, "--count"]);
    assert_eq!(
        code, 0,
        "query-mmap failed for {query:?}; stderr: {err}"
    );
    out.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("query-mmap returned no solution count for {query:?}: {out}"))
}

/// Dump the shared TriG fixture, reload the emitted format through `save`, and assert the
/// default, whole-dataset, and named-graph counts exposed by `query-mmap`.
fn assert_textual_dump_roundtrip(
    out_format: &str,
    extension: &str,
    expected_total: usize,
    named_counts: &[(&str, usize)],
) {
    let dir = scratch(&format!("roundtrip-{out_format}"));
    let input = write(&dir, "input.trig", DATASET_TRIG);
    let (dump_code, dumped, dump_err) = run3(&["dump", s(&input), "trig", out_format]);
    assert_eq!(
        dump_code, 0,
        "dump to {out_format} failed; stderr: {dump_err}"
    );

    let output = write(&dir, &format!("dump.{extension}"), &dumped);
    let index = dir.join("index");
    let (save_code, _save_out, save_err) = run3(&["save", s(&output), out_format, s(&index)]);
    assert_eq!(
        save_code, 0,
        "reload of dumped {out_format} failed; stderr: {save_err}"
    );

    // Check graph identity first: if a serializer silently flattens named triples into the
    // default graph, this is the assertion that witnesses the regression.
    for &(graph, expected) in named_counts {
        let query = format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{graph}> {{ ?s ?p ?o }} }}");
        assert_eq!(
            query_mmap_count(&index, &query),
            expected,
            "{out_format} changed named graph <{graph}>"
        );
    }

    let default_query = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    assert_eq!(
        query_mmap_count(&index, default_query),
        2,
        "{out_format} changed the default graph"
    );

    // A plain triple pattern is scoped to the default graph. UNION it with GRAPH ?g to count
    // the complete dataset without using COUNT(*), because `--count` counts solution rows.
    let dataset_query =
        "SELECT ?s ?p ?o WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }";
    assert_eq!(
        query_mmap_count(&index, dataset_query),
        expected_total,
        "{out_format} changed the full dataset count"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Turtle is a graph format, so dumping a dataset intentionally preserves only its two
/// default-graph triples; named graphs must neither leak into nor be flattened into that graph.
#[test]
fn dump_turtle_roundtrips_default_graph_count() {
    assert_textual_dump_roundtrip("turtle", "ttl", 2, &[]);
}

/// TriG is a dataset format and must preserve all six triples and both graph identities.
#[test]
fn dump_trig_roundtrips_default_and_named_graph_counts() {
    assert_textual_dump_roundtrip(
        "trig",
        "trig",
        6,
        &[("http://example.com/g1", 3), ("http://example.com/g2", 1)],
    );
}

/// N-Quads is a dataset format and must preserve all six triples and both graph identities.
#[test]
fn dump_nquads_roundtrips_default_and_named_graph_counts() {
    assert_textual_dump_roundtrip(
        "nquads",
        "nq",
        6,
        &[("http://example.com/g1", 3), ("http://example.com/g2", 1)],
    );
}

/// Baseline: `dump … jsonld-expanded` emits a JSON-LD array (no `@context`) — the always-on
/// expanded path still works (a regression guard for the new match arm ordering).
#[test]
fn dump_jsonld_expanded_still_works() {
    let dir = scratch("expanded");
    let data = write(&dir, "g.ttl", TTL);
    let (code, out, err) = run3(&["dump", s(&data), "turtle", "jsonld-expanded"]);
    assert_eq!(code, 0, "stderr: {err}");
    // Expanded with no named graphs is a bare node-object array (full IRIs, no @context).
    assert!(out.trim_start().starts_with('['), "expanded doc is a bare array: {out}");
    assert!(out.contains("http://schema.org/name"), "full-IRI predicate present: {out}");
    assert!(!out.contains("\"@context\""), "expanded form has no @context: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `dump … jsonld-compact --context <ctx>` runs the FULL W3C Compaction Algorithm: an
/// `@vocab` context collapses every predicate to a bare `@vocab`-relative term and echoes the
/// `@context` — the capability the prefix-only `jsonld-compacted` form cannot express.
#[test]
fn dump_jsonld_compact_with_context() {
    let dir = scratch("compact");
    let data = write(&dir, "g.ttl", TTL);
    let ctx = write(&dir, "ctx.jsonld", r#"{"@vocab":"http://schema.org/"}"#);
    let (code, out, err) =
        run3(&["dump", s(&data), "turtle", "jsonld-compact", "--context", s(&ctx)]);
    assert_eq!(code, 0, "compaction must succeed; stderr: {err}");
    assert!(out.contains("\"@vocab\":\"http://schema.org/\""), "context echoed: {out}");
    // Bare `@vocab`-relative predicate keys (NOT `schema:name`) prove the algorithm ran.
    assert!(out.contains("\"name\":"), "predicate is a bare @vocab term: {out}");
    assert!(out.contains("\"knows\":"), "second predicate compacted too: {out}");
    // Minified by default — the pretty pass inserts newlines, so the default output has none.
    assert!(!out.contains('\n'), "minified by default (no pretty newlines): {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `-pretty` variant emits the same document indented (multi-line).
#[test]
fn dump_jsonld_compact_pretty() {
    let dir = scratch("compact-pretty");
    let data = write(&dir, "g.ttl", TTL);
    let ctx = write(&dir, "ctx.jsonld", r#"{"@vocab":"http://schema.org/"}"#);
    let (code, out, err) = run3(&[
        "dump", s(&data), "turtle", "jsonld-compact-pretty", "--context", s(&ctx),
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains('\n'), "pretty compaction is multi-line: {out}");
    assert!(out.contains("\"name\":"), "still compacted: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `jsonld-compact` WITHOUT `--context` is a usage error (exit 2) with a clear message —
/// the `@context` is mandatory for the full algorithm.
#[test]
fn dump_jsonld_compact_missing_context_errors() {
    let dir = scratch("no-ctx");
    let data = write(&dir, "g.ttl", TTL);
    let (code, _out, err) = run3(&["dump", s(&data), "turtle", "jsonld-compact"]);
    assert_eq!(code, 2, "missing --context must exit 2: {err}");
    assert!(err.contains("--context"), "error mentions the flag: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `--context` file that is not a JSON object (a JSON-LD `@context` must be one) is a
/// data error (exit 1), not a silently-wrong document.
#[test]
fn dump_jsonld_compact_non_object_context_errors() {
    let dir = scratch("bad-ctx");
    let data = write(&dir, "g.ttl", TTL);
    let ctx = write(&dir, "ctx.json", r#"["not an object"]"#);
    let (code, _out, err) =
        run3(&["dump", s(&data), "turtle", "jsonld-compact", "--context", s(&ctx)]);
    assert_eq!(code, 1, "a non-object @context must exit 1: {err}");
    assert!(err.contains("@context") || err.contains("JSON object"), "clear error: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--context` with no following value is a usage error (exit 2).
#[test]
fn dump_context_flag_missing_value_errors() {
    let dir = scratch("ctx-no-val");
    let data = write(&dir, "g.ttl", TTL);
    let (code, _out, err) =
        run3(&["dump", s(&data), "turtle", "jsonld-compact", "--context"]);
    assert_eq!(code, 2, "--context with no value must exit 2: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (sq-oy1f.4) DEFAULT-ON JSON-LD round-trip — the load-bearing invariant for making
/// `jsonld` a default feature of the native CLI: with NO `--features` flag, the binary must both
/// READ a JSON-LD input document (the `oxjsonld` parser, `sparq-core/jsonld`) and WRITE one back
/// (the engine's JSON-LD writer, `sparq-engine/serialize-rdf`). This test is gated on the `jsonld`
/// feature — which is in the CLI default set — so the default `cargo test -p sparq-cli` exercises
/// the real JSON-LD parse path, while a `--no-default-features --features serialize-rdf` build (no
/// JSON-LD parser) skips it cleanly. A `dump in.jsonld jsonld jsonld-expanded` is a full
/// parse→intern→re-serialise round-trip through both ends of the JSON-LD surface.
#[cfg(feature = "jsonld")]
#[test]
fn dump_jsonld_input_roundtrips_default_build() {
    let dir = scratch("jsonld-input");
    // An expanded-form JSON-LD input document (one node, two predicates).
    let jsonld_in = r#"[
      {
        "@id": "http://schema.org/alice",
        "http://schema.org/name": [{"@value": "Alice"}],
        "http://schema.org/knows": [{"@id": "http://schema.org/bob"}]
      }
    ]"#;
    let data = write(&dir, "g.jsonld", jsonld_in);
    // Read JSON-LD in (oxjsonld parser), re-emit as expanded JSON-LD (engine writer).
    let (code, out, err) = run3(&["dump", s(&data), "jsonld", "jsonld-expanded"]);
    assert_eq!(code, 0, "JSON-LD in -> JSON-LD out must succeed in the default build; stderr: {err}");
    // The output is a JSON-LD node-object array carrying the parsed triples.
    assert!(out.trim_start().starts_with('['), "re-serialised as a JSON-LD array: {out}");
    assert!(out.contains("http://schema.org/alice"), "subject survived the round-trip: {out}");
    assert!(out.contains("http://schema.org/name"), "name predicate survived: {out}");
    assert!(out.contains("Alice"), "literal value survived: {out}");
    assert!(out.contains("http://schema.org/bob"), "object IRI survived: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (sq-oy1f.4) The default build also accepts the `application/ld+json` media-type token
/// (and `json-ld`) as an input format alias for the JSON-LD parser — same dispatch as the HTTP
/// surface. Proves the alias set is wired in the default (no-flag) build.
#[cfg(feature = "jsonld")]
#[test]
fn dump_jsonld_input_media_type_alias_default_build() {
    let dir = scratch("jsonld-alias");
    let jsonld_in = r#"{"@id":"http://ex/s","http://ex/p":[{"@value":"v"}]}"#;
    let data = write(&dir, "g.jsonld", jsonld_in);
    let (code, out, err) = run3(&["dump", s(&data), "application/ld+json", "ntriples"]);
    assert_eq!(code, 0, "the `application/ld+json` input alias parses in the default build; stderr: {err}");
    assert!(out.contains("<http://ex/s>"), "parsed subject present: {out}");
    assert!(out.contains("<http://ex/p>"), "parsed predicate present: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

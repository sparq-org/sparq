//! [OPUS-4.8] Per-subcommand golden-output + exit-code CONTRACT table for sparq-cli.
//!
//! Spawns the *built* `sparq-cli` binary (via `CARGO_BIN_EXE_sparq-cli`, the same
//! mechanism as `hdt_cli.rs`) and drives it through an invocation table —
//! `(args) -> (expected exit code, stdout/stderr shape)`. Modelled on the server's
//! `protocol.rs` per-input negotiation table, adapted to a CLI process table.
//!
//! Why a *subprocess* test and not unit tests: every subcommand's logic lives behind
//! `fn main()`'s positional dispatch and reports via `println!`/`std::process::exit`,
//! so the only way to test the real exit-code + stdout contract is to run the binary.
//! (This is also why the crate's llvm-cov reads ~0% — the covered code runs in a child
//! process the coverage instrumentation of the test binary never sees.)
//!
//! These tests are intentionally datasets-tiny and self-contained: each builds its own
//! fixtures under `CARGO_TARGET_TMPDIR` and cleans them up.
//!
//! NOTE on the `query` output contract (sq-l4ki — `query` is now a real query CLI). [OPUS-4.8]
//!
//! `query` classifies the parsed query FORM and routes each to its executor, emitting real
//! results (not just a count):
//!
//! - SELECT: the solution bindings. Default a readable ASCII table; `--format
//!   <table|tsv|csv|xml|json|ntriples>` selects a W3C result format (tsv/csv/xml reuse
//!   sparq-server's serialisers; json reuses the engine's direct serialiser).
//! - ASK: a boolean (`true` / `false`); `--format json|xml` emit the W3C boolean documents.
//! - CONSTRUCT/DESCRIBE: the resulting triples as N-Triples (NOT an error any more).
//! - `--count`: restores the historical count-only output ("<n> solutions/triples in
//!   <ms>ms") for scripts that depended on it.
//!
//! Before sq-l4ki, `query` routed everything through `sparq_engine::query` (SELECT/ASK only):
//! SELECT/ASK surfaced only as a solution count (ASK as 1/0), and CONSTRUCT/DESCRIBE errored
//! out with "only SELECT and ASK queries are supported" (exit 1). The cases below assert the
//! FIXED behaviour; the `bench` CONSTRUCT case (run_query_suite routes graph forms through
//! construct_or_describe) is still exercised too.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A fresh, unique scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-contract-{tag}"));
    let _ = std::fs::remove_dir_all(&dir); // clean any prior run
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The canonical 3-triple N-Triples fixture used across the happy-path cases.
const NT: &str = "\
<http://ex/alice> <http://ex/knows> <http://ex/bob> .
<http://ex/alice> <http://ex/age> \"30\" .
<http://ex/bob> <http://ex/age> \"25\" .
";

/// The same graph as Turtle, to exercise the buffered (non-NT) load path + the `ttl` alias.
const TTL: &str = "\
@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob ; ex:age 30 .
ex:bob ex:age 25 .
";

/// Run the built binary with `args`, returning the captured Output.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(args)
        .output()
        .expect("spawning sparq-cli")
}

/// Convenience: (exit code, stdout, stderr). A missing code (signal) is reported as -1.
fn run3(args: &[&str]) -> (i32, String, String) {
    let out = run(args);
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

// ---------------------------------------------------------------------------
// 1. Happy paths — `query`
// ---------------------------------------------------------------------------

#[test]
fn query_select_emits_rows_nt() {
    // [OPUS-4.8] (sq-l4ki) SELECT now prints the solution BINDINGS, not a count. The
    // default format is a readable table: a header row with the projected variables and one
    // row per solution carrying the actual terms.
    let dir = scratch("select-nt");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["query", s(&data), "ntriples", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    // Header carries the projected variables (with the SPARQL `?` sigil).
    assert!(stdout.contains("?s") && stdout.contains("?p") && stdout.contains("?o"), "header: {stdout}");
    // Rows carry actual bound terms — not just a count.
    assert!(stdout.contains("<http://ex/alice>"), "rows should contain alice: {stdout}");
    assert!(stdout.contains("<http://ex/knows>"), "rows should contain knows: {stdout}");
    assert!(stdout.contains("\"30\""), "rows should contain the age literal: {stdout}");
    // Three solutions => the table reports three rows.
    assert!(stdout.contains("3 row(s)"), "row count footer: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_select_turtle_and_ttl_alias_load_identically() {
    let dir = scratch("select-ttl");
    let data = write(&dir, "data.ttl", TTL);
    // canonical name — emits bindings (rows), not a count.
    let (c1, o1, e1) = run3(&["query", s(&data), "turtle", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_eq!(c1, 0, "stderr: {e1}");
    assert!(o1.contains("3 row(s)") && o1.contains("<http://ex/alice>"), "stdout: {o1}");
    // alias loads identically.
    let (c2, o2, _) = run3(&["query", s(&data), "ttl", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_eq!(c2, 0);
    assert!(o2.contains("3 row(s)") && o2.contains("<http://ex/alice>"), "stdout: {o2}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_ask_prints_boolean() {
    // [OPUS-4.8] (sq-l4ki) ASK now prints a boolean (`true`/`false`), not a 1/0 solution
    // count. The two must be unambiguously distinguishable as booleans.
    let dir = scratch("ask");
    let data = write(&dir, "data.nt", NT);
    let (ct, ot, et) = run3(&["query", s(&data), "ntriples", "ASK { ?s <http://ex/knows> ?o }"]);
    assert_eq!(ct, 0, "stderr: {et}");
    assert_eq!(ot.trim(), "true", "ASK-true stdout: {ot}");

    let (cf, of, _) = run3(&["query", s(&data), "ntriples", "ASK { <http://ex/nobody> ?p ?o }"]);
    assert_eq!(cf, 0);
    assert_eq!(of.trim(), "false", "ASK-false stdout: {of}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_with_reason_flag_loads() {
    // `--reason rdfs` on a graph with no schema is a no-op closure, but it must
    // route through the reasoning load path and still answer the query (emit rows).
    let dir = scratch("reason-flag");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&[
        "query",
        s(&data),
        "ntriples",
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
        "--reason",
        "rdfs",
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("3 row(s)"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_count_flag_preserves_legacy_count_output() {
    // [OPUS-4.8] (sq-l4ki) `--count` is the backward-compatible escape hatch: it restores
    // the historical count-only line for both SELECT (solutions) and the graph forms (triples).
    let dir = scratch("count-flag");
    let data = write(&dir, "data.nt", NT);
    let (cs, os, es) = run3(&["query", s(&data), "ntriples", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(cs, 0, "stderr: {es}");
    assert!(os.contains("3 solutions"), "SELECT --count stdout: {os}");
    // No table border / row terms when counting.
    assert!(!os.contains("<http://ex/alice>"), "count mode must not emit rows: {os}");

    let (cc, oc, _) = run3(&["query", s(&data), "ntriples", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(cc, 0);
    assert!(oc.contains("3 triples"), "CONSTRUCT --count stdout: {oc}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_select_format_shapes() {
    // [OPUS-4.8] (sq-l4ki) Each `--format` emits the right SHAPE for SELECT bindings.
    let dir = scratch("select-formats");
    let data = write(&dir, "data.nt", NT);
    let q = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }";

    // TSV: header WITH `?` sigil, TAB-separated; literals in SPARQL term syntax (quoted).
    let (ct, ot, et) = run3(&["query", s(&data), "ntriples", q, "--format", "tsv"]);
    assert_eq!(ct, 0, "tsv stderr: {et}");
    assert_eq!(ot.lines().next().unwrap(), "?s\t?o", "tsv header: {ot}");
    assert!(ot.contains("<http://ex/alice>\t\"30\""), "tsv row: {ot}");

    // CSV: header WITHOUT `?`, comma-separated; literal as bare lexical value.
    let (cc, oc, _) = run3(&["query", s(&data), "ntriples", q, "--format", "csv"]);
    assert_eq!(cc, 0);
    assert_eq!(oc.lines().next().unwrap(), "s,o", "csv header: {oc}");
    assert!(oc.contains("http://ex/alice,30"), "csv row: {oc}");

    // JSON: SPARQL 1.1 Results JSON object with head.vars + results.bindings.
    let (cj, oj, _) = run3(&["query", s(&data), "ntriples", q, "--format", "json"]);
    assert_eq!(cj, 0);
    assert!(oj.contains("\"head\"") && oj.contains("\"vars\"") && oj.contains("\"bindings\""), "json: {oj}");
    assert!(oj.contains("\"value\":\"http://ex/alice\""), "json binding: {oj}");

    // XML: SPARQL Results XML with the sparql-results namespace + <binding> elements.
    let (cx, ox, _) = run3(&["query", s(&data), "ntriples", q, "--format", "xml"]);
    assert_eq!(cx, 0);
    assert!(ox.contains("xmlns=\"http://www.w3.org/2005/sparql-results#\""), "xml ns: {ox}");
    assert!(ox.contains("<uri>http://ex/alice</uri>"), "xml binding: {ox}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_ask_format_json_xml() {
    // [OPUS-4.8] (sq-l4ki) ASK under `--format json|xml` emits the W3C boolean documents.
    let dir = scratch("ask-formats");
    let data = write(&dir, "data.nt", NT);
    let (cj, oj, _) = run3(&["query", s(&data), "ntriples", "ASK { ?s <http://ex/knows> ?o }", "--format", "json"]);
    assert_eq!(cj, 0);
    assert!(oj.contains("\"boolean\":true"), "ask json: {oj}");

    let (cx, ox, _) = run3(&["query", s(&data), "ntriples", "ASK { <http://ex/nobody> ?p ?o }", "--format", "xml"]);
    assert_eq!(cx, 0);
    assert!(ox.contains("<boolean>false</boolean>"), "ask xml: {ox}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_unknown_format_exits_2() {
    // [OPUS-4.8] (sq-l4ki) An unknown `--format` value is a usage error (exit 2), matching
    // the CLI's flag-validation contract.
    let dir = scratch("err-outformat");
    let data = write(&dir, "data.nt", NT);
    let (code, _o, stderr) = run3(&["query", s(&data), "ntriples", "SELECT ?s WHERE { ?s ?p ?o }", "--format", "bogus"]);
    assert_eq!(code, 2, "unknown --format must exit 2; stderr: {stderr}");
    assert!(stderr.contains("unknown --format"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Happy paths — compressed-input auto-detection (gzip / zstd / bzip2)
//    All three codecs of the *same* N-Triples bytes must load identically.
// ---------------------------------------------------------------------------

#[test]
fn compressed_input_autodetect_gzip_zstd_bzip2_load_identically() {
    use std::io::Write;
    let dir = scratch("compressed");
    let raw = write(&dir, "data.nt", NT);

    // gzip
    let gz = dir.join("data.nt.gz");
    {
        let mut enc = flate2::write::GzEncoder::new(
            std::fs::File::create(&gz).unwrap(),
            flate2::Compression::fast(),
        );
        enc.write_all(NT.as_bytes()).unwrap();
        enc.finish().unwrap();
    }
    // bzip2
    let bz = dir.join("data.nt.bz2");
    {
        let mut enc = bzip2::write::BzEncoder::new(
            std::fs::File::create(&bz).unwrap(),
            bzip2::Compression::fast(),
        );
        enc.write_all(NT.as_bytes()).unwrap();
        enc.finish().unwrap();
    }
    // zstd
    let zst = dir.join("data.nt.zst");
    std::fs::write(&zst, zstd::stream::encode_all(NT.as_bytes(), 3).unwrap()).unwrap();

    let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    let mut counts = Vec::new();
    for f in [&raw, &gz, &bz, &zst] {
        // [OPUS-4.8] (sq-l4ki) Use `--count` here so the assertion is on the solution COUNT
        // (codec-equivalence is about how many triples decoded, not the table rendering).
        let (code, stdout, stderr) = run3(&["query", s(f), "ntriples", q, "--count"]);
        assert_eq!(code, 0, "{}: stderr {stderr}", f.display());
        assert!(stdout.contains("3 solutions"), "{}: stdout {stdout}", f.display());
        // capture the solution count token to assert byte-identical results across codecs
        counts.push(
            stdout
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string(),
        );
    }
    assert!(
        counts.iter().all(|c| c == "3"),
        "compressed inputs disagreed on solution count: {counts:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Happy paths — index round-trip: save -> query-mmap ; build -> query-mmap
// ---------------------------------------------------------------------------

#[test]
fn save_then_query_mmap_round_trip() {
    let dir = scratch("save-roundtrip");
    let data = write(&dir, "data.nt", NT);
    let idx = dir.join("idx");

    let (sc, _so, se) = run3(&["save", s(&data), "ntriples", s(&idx)]);
    assert_eq!(sc, 0, "save stderr: {se}");
    assert!(idx.is_dir(), "save did not create the index dir");

    // [OPUS-4.8] (sq-iwyy) These round-trip tests assert the index encodes/decodes the right
    // number of triples — codec/build equivalence, not the table rendering — so they use
    // `--count` to keep the assertion on the solution COUNT (now that a bare SELECT emits a
    // table). The default-format SELECT rendering is exercised by the dedicated parity tests
    // below.
    let (qc, qo, qe) = run3(&["query-mmap", s(&idx), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(qc, 0, "query-mmap stderr: {qe}");
    assert!(qo.contains("3 solutions"), "query-mmap stdout: {qo}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_compressed_then_query_mmap_round_trip() {
    let dir = scratch("save-compressed");
    let data = write(&dir, "data.nt", NT);
    let idx = dir.join("cidx");

    let (sc, _so, se) = run3(&["save", s(&data), "ntriples", s(&idx), "compressed"]);
    assert_eq!(sc, 0, "save stderr: {se}");

    let (qc, qo, qe) = run3(&["query-mmap", s(&idx), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(qc, 0, "query-mmap stderr: {qe}");
    assert!(qo.contains("3 solutions"), "query-mmap stdout: {qo}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_external_then_query_mmap_round_trip() {
    let dir = scratch("build-roundtrip");
    let data = write(&dir, "data.nt", NT);
    let idx = dir.join("bidx");

    let (bc, _bo, be) = run3(&["build", s(&data), "ntriples", s(&idx)]);
    assert_eq!(bc, 0, "build stderr: {be}");
    assert!(idx.is_dir(), "build did not create the index dir");

    let (qc, qo, qe) = run3(&["query-mmap", s(&idx), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(qc, 0, "query-mmap stderr: {qe}");
    assert!(qo.contains("3 solutions"), "query-mmap stdout: {qo}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [OPUS-4.8] (sq-5atq) The CLI `build` of an N-Quads dataset goes through the OUT-OF-CORE
/// quad path (`build_external_quads`), so its NAMED GRAPHS must survive end-to-end: a
/// `GRAPH <g> { ?s ?p ?o }` query over the opened index returns that graph's quads, not a
/// flattened default graph. Before the quad-aware `build` wiring, every quad was silently
/// folded into the default graph and this query returned nothing.
#[test]
fn build_external_nquads_named_graph_round_trip() {
    let dir = scratch("build-nquads-roundtrip");
    let nq = "\
<http://ex/alice> <http://ex/knows> <http://ex/bob> <http://ex/g1> .
<http://ex/bob> <http://ex/age> \"25\" <http://ex/g1> .
<http://ex/x> <http://ex/y> <http://ex/z> <http://ex/g2> .
<http://ex/default> <http://ex/p> <http://ex/o> .
";
    let data = write(&dir, "data.nq", nq);
    let idx = dir.join("nqidx");

    let (bc, _bo, be) = run3(&["build", s(&data), "nquads", s(&idx)]);
    assert_eq!(bc, 0, "build stderr: {be}");
    assert!(idx.join("named.bin").is_file(), "named manifest not written by quad build");

    // g1 has two quads; the GRAPH query must see exactly them (not a flattened default graph).
    let (qc, qo, qe) = run3(&[
        "query-mmap",
        s(&idx),
        "SELECT ?s ?p ?o WHERE { GRAPH <http://ex/g1> { ?s ?p ?o } }",
        "--count",
    ]);
    assert_eq!(qc, 0, "query-mmap stderr: {qe}");
    assert!(qo.contains("2 solutions"), "GRAPH g1 query stdout: {qo}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recompress_raw_index_then_query() {
    let dir = scratch("recompress");
    let data = write(&dir, "data.nt", NT);
    let raw = dir.join("raw");
    let cmp = dir.join("cmp");

    assert_eq!(run3(&["save", s(&data), "ntriples", s(&raw)]).0, 0);
    let (rc, _ro, re) = run3(&["recompress", s(&raw), s(&cmp)]);
    assert_eq!(rc, 0, "recompress stderr: {re}");

    let (qc, qo, _) = run3(&["query-mmap", s(&cmp), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(qc, 0);
    assert!(qo.contains("3 solutions"), "stdout: {qo}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3b. `query-mmap` OUTPUT PARITY with `query` (sq-iwyy).
//
// [OPUS-4.8] query-mmap shares `query`'s exact emission core, so it must emit the same
// SHAPES: SELECT rows (default table + each `--format`), ASK boolean, CONSTRUCT/DESCRIBE
// N-Triples, and the `--count` legacy line. These mirror the `query` happy-path cases but
// drive the mmap path (save -> query-mmap). The unknown-`--format` usage error (exit 2) is
// shared verbatim via `out_format_flag`, so it's covered for both by `query`'s case.
// ---------------------------------------------------------------------------

/// Build an on-disk index from the canonical NT fixture and return (scratch dir, index dir).
/// The caller queries `idx` via `query-mmap` and is responsible for removing `dir`.
fn mmap_index(tag: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(tag);
    let data = write(&dir, "data.nt", NT);
    let idx = dir.join("idx");
    let (sc, _so, se) = run3(&["save", s(&data), "ntriples", s(&idx)]);
    assert_eq!(sc, 0, "save stderr: {se}");
    (dir, idx)
}

#[test]
fn query_mmap_select_emits_rows_default_table() {
    // [OPUS-4.8] (sq-iwyy) A bare SELECT via query-mmap now prints the solution BINDINGS in a
    // readable table (header with ?-sigil vars, one row per solution, `(K row(s))` footer) —
    // identical to `query`, not a count.
    let (dir, idx) = mmap_index("mmap-select-table");
    let (code, stdout, stderr) = run3(&["query-mmap", s(&idx), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("?s") && stdout.contains("?p") && stdout.contains("?o"), "header: {stdout}");
    assert!(stdout.contains("<http://ex/alice>"), "rows should contain alice: {stdout}");
    assert!(stdout.contains("<http://ex/knows>"), "rows should contain knows: {stdout}");
    assert!(stdout.contains("\"30\""), "rows should contain the age literal: {stdout}");
    assert!(stdout.contains("3 row(s)"), "row count footer: {stdout}");
    // Crucially NOT the legacy count line (that is what parity fixed).
    assert!(!stdout.contains("3 solutions"), "default SELECT must not be a count: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_ask_prints_boolean() {
    // [OPUS-4.8] (sq-iwyy) ASK via query-mmap prints a boolean, not a 1/0 solution count.
    let (dir, idx) = mmap_index("mmap-ask");
    let (ct, ot, et) = run3(&["query-mmap", s(&idx), "ASK { ?s <http://ex/knows> ?o }"]);
    assert_eq!(ct, 0, "stderr: {et}");
    assert_eq!(ot.trim(), "true", "ASK-true stdout: {ot}");

    let (cf, of, _) = run3(&["query-mmap", s(&idx), "ASK { <http://ex/nobody> ?p ?o }"]);
    assert_eq!(cf, 0);
    assert_eq!(of.trim(), "false", "ASK-false stdout: {of}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_construct_and_describe_emit_triples() {
    // [OPUS-4.8] (sq-iwyy) The graph forms via query-mmap emit the resulting triples as
    // N-Triples (previously query-mmap could only count).
    let (dir, idx) = mmap_index("mmap-construct");
    let (cc, oc, ec) = run3(&["query-mmap", s(&idx), "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"]);
    assert_eq!(cc, 0, "CONSTRUCT stderr: {ec}");
    let lines: Vec<&str> = oc.lines().filter(|l| l.trim_end().ends_with(" .")).collect();
    assert_eq!(lines.len(), 3, "expected 3 N-Triples lines; stdout: {oc}");
    assert!(
        oc.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob> ."),
        "CONSTRUCT stdout: {oc}"
    );

    let (cd, od, ed) = run3(&["query-mmap", s(&idx), "DESCRIBE <http://ex/alice>"]);
    assert_eq!(cd, 0, "DESCRIBE stderr: {ed}");
    assert!(od.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob> ."), "DESCRIBE stdout: {od}");
    assert!(od.contains("<http://ex/alice> <http://ex/age> \"30\" ."), "DESCRIBE stdout: {od}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_select_format_shapes() {
    // [OPUS-4.8] (sq-iwyy) Each `--format` emits the right SHAPE for SELECT bindings via the
    // mmap path — mirrors `query_select_format_shapes`.
    let (dir, idx) = mmap_index("mmap-formats");
    let q = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }";

    let (ct, ot, et) = run3(&["query-mmap", s(&idx), q, "--format", "tsv"]);
    assert_eq!(ct, 0, "tsv stderr: {et}");
    assert_eq!(ot.lines().next().unwrap(), "?s\t?o", "tsv header: {ot}");
    assert!(ot.contains("<http://ex/alice>\t\"30\""), "tsv row: {ot}");

    let (cc, oc, _) = run3(&["query-mmap", s(&idx), q, "--format", "csv"]);
    assert_eq!(cc, 0);
    assert_eq!(oc.lines().next().unwrap(), "s,o", "csv header: {oc}");
    assert!(oc.contains("http://ex/alice,30"), "csv row: {oc}");

    let (cj, oj, _) = run3(&["query-mmap", s(&idx), q, "--format", "json"]);
    assert_eq!(cj, 0);
    assert!(oj.contains("\"head\"") && oj.contains("\"vars\"") && oj.contains("\"bindings\""), "json: {oj}");
    assert!(oj.contains("\"value\":\"http://ex/alice\""), "json binding: {oj}");

    let (cx, ox, _) = run3(&["query-mmap", s(&idx), q, "--format", "xml"]);
    assert_eq!(cx, 0);
    assert!(ox.contains("xmlns=\"http://www.w3.org/2005/sparql-results#\""), "xml ns: {ox}");
    assert!(ox.contains("<uri>http://ex/alice</uri>"), "xml binding: {ox}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_ask_format_json_xml() {
    // [OPUS-4.8] (sq-iwyy) ASK under `--format json|xml` via query-mmap emits the W3C boolean
    // documents — mirrors `query_ask_format_json_xml`.
    let (dir, idx) = mmap_index("mmap-ask-formats");
    let (cj, oj, _) = run3(&["query-mmap", s(&idx), "ASK { ?s <http://ex/knows> ?o }", "--format", "json"]);
    assert_eq!(cj, 0);
    assert!(oj.contains("\"boolean\":true"), "ask json: {oj}");

    let (cx, ox, _) = run3(&["query-mmap", s(&idx), "ASK { <http://ex/nobody> ?p ?o }", "--format", "xml"]);
    assert_eq!(cx, 0);
    assert!(ox.contains("<boolean>false</boolean>"), "ask xml: {ox}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_count_flag_preserves_legacy_count_output() {
    // [OPUS-4.8] (sq-iwyy) `--count` restores the historical count-only line for both SELECT
    // (solutions) and the graph forms (triples) via the mmap path.
    let (dir, idx) = mmap_index("mmap-count");
    let (cs, os, es) = run3(&["query-mmap", s(&idx), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(cs, 0, "stderr: {es}");
    assert!(os.contains("3 solutions"), "SELECT --count stdout: {os}");
    assert!(!os.contains("<http://ex/alice>"), "count mode must not emit rows: {os}");

    let (cc, oc, _) = run3(&["query-mmap", s(&idx), "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", "--count"]);
    assert_eq!(cc, 0);
    assert!(oc.contains("3 triples"), "CONSTRUCT --count stdout: {oc}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_unknown_format_exits_2() {
    // [OPUS-4.8] (sq-iwyy) An unknown `--format` value is a usage error (exit 2) on query-mmap
    // too — it shares `query`'s `out_format_flag` validation.
    let (dir, idx) = mmap_index("mmap-err-outformat");
    let (code, _o, stderr) = run3(&["query-mmap", s(&idx), "SELECT ?s WHERE { ?s ?p ?o }", "--format", "bogus"]);
    assert_eq!(code, 2, "unknown --format must exit 2; stderr: {stderr}");
    assert!(stderr.contains("unknown --format"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_malformed_query_exits_1() {
    // [OPUS-4.8] (sq-iwyy) A malformed query via query-mmap is a runtime error (exit 1) with a
    // useful stderr — same contract as `query`.
    let (dir, idx) = mmap_index("mmap-err-badquery");
    let (code, _o, stderr) = run3(&["query-mmap", s(&idx), "SELECT ?s WHERE { this is not sparql"]);
    assert_eq!(code, 1, "parse failure is a runtime error (exit 1); stderr: {stderr}");
    assert!(stderr.to_lowercase().contains("error"), "stderr should explain the failure: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Happy paths — ingest / reason / bench (the TSV + summary shapes)
// ---------------------------------------------------------------------------

#[test]
fn ingest_full_reports_triple_and_term_counts() {
    let dir = scratch("ingest");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["ingest", s(&data), "full"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("triples ingested : 3"), "stdout: {stdout}");
    assert!(stdout.contains("distinct terms   : 6"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reason_rdfs_reports_triple_count() {
    let dir = scratch("reason");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["reason", s(&data), "ntriples", "rdfs"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("triples after rdfs reasoning"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #6466: with an output path, stdout may BE the closure (`/dev/stdout`), so the summary goes
/// to stderr and stdout carries only N-Triples.
#[cfg(unix)]
#[test]
fn reason_to_dev_stdout_writes_only_the_closure_to_stdout() {
    let dir = scratch("reason-stdout");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["reason", s(&data), "ntriples", "rdfs", "/dev/stdout"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.contains("triples after rdfs reasoning"), "stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "closure missing from stdout");
    assert!(lines.iter().all(|l| l.ends_with(" .")), "non-N-Triples line on stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bench_emits_one_tsv_line_per_query_including_construct() {
    let dir = scratch("bench");
    let data = write(&dir, "data.nt", NT);
    let qd = dir.join("q");
    std::fs::create_dir_all(&qd).unwrap();
    write(&qd, "a_all.rq", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }");
    write(&qd, "b_ask.rq", "ASK { ?s <http://ex/knows> ?o }");
    // CONSTRUCT works via `bench` (graph forms route through construct_or_describe),
    // even though it errors via the `query` subcommand — assert that contract here.
    write(&qd, "c_con.rq", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }");

    let (code, stdout, stderr) = run3(&["bench", s(&data), "ntriples", s(&qd), "1"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    // One TSV line per query, sorted by name: name<TAB>rows<TAB>micros
    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains('\t')).collect();
    assert_eq!(lines.len(), 3, "expected 3 TSV lines, got: {stdout}");
    assert!(lines[0].starts_with("a_all\t3\t"), "line0: {}", lines[0]);
    assert!(lines[1].starts_with("b_ask\t1\t"), "line1: {}", lines[1]);
    assert!(lines[2].starts_with("c_con\t3\t"), "line2 (CONSTRUCT): {}", lines[2]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// [OPUS-4.8] (sq-d7d) `bench --json <path>` writes a machine-readable results document
/// ALONGSIDE the unchanged STDOUT TSV. Asserts: (a) STDOUT is still exactly one TSV line per
/// query (JSON is additive, not a replacement); (b) the file is genuinely VALID, parseable JSON
/// (parsed with serde_json, not substring-matched); (c) it carries the expected top-level keys +
/// a `queries` array whose entries mirror the TSV fields (`name`, `rows`, `min_micros`).
#[test]
fn bench_json_flag_writes_parseable_results_with_expected_keys() {
    let dir = scratch("bench-json");
    let data = write(&dir, "data.nt", NT);
    let qd = dir.join("q");
    std::fs::create_dir_all(&qd).unwrap();
    write(&qd, "a_all.rq", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }");
    write(&qd, "b_ask.rq", "ASK { ?s <http://ex/knows> ?o }");
    let out = dir.join("results.json");

    let (code, stdout, stderr) =
        run3(&["bench", s(&data), "ntriples", s(&qd), "1", "count", "--json", s(&out)]);
    assert_eq!(code, 0, "stderr: {stderr}");

    // (a) STDOUT TSV is unchanged — still one line per query, byte-for-byte the historical shape.
    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains('\t')).collect();
    assert_eq!(lines.len(), 2, "expected 2 TSV lines on STDOUT, got: {stdout}");
    assert!(lines[0].starts_with("a_all\t3\t"), "line0: {}", lines[0]);
    assert!(lines[1].starts_with("b_ask\t1\t"), "line1: {}", lines[1]);

    // (b) The file exists and is VALID, parseable JSON.
    let raw = std::fs::read_to_string(&out).expect("--json results file should exist");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("results file must be valid JSON");

    // (c) Expected top-level keys + run parameters.
    assert_eq!(doc["harness"], "sparq-cli bench", "harness key: {raw}");
    assert_eq!(doc["mode"], "count", "mode key: {raw}");
    assert_eq!(doc["iters"], 1, "iters key: {raw}");
    assert!(doc["note"].is_string(), "note caveat present: {raw}");

    // The `queries` array mirrors the TSV: one entry per *.rq, sorted by name, each carrying
    // the SAME measured fields (name / rows / min_micros).
    let queries = doc["queries"].as_array().expect("queries must be an array");
    assert_eq!(queries.len(), 2, "one entry per query: {raw}");
    assert_eq!(queries[0]["name"], "a_all", "first query name: {raw}");
    assert_eq!(queries[0]["rows"], 3, "first query rows: {raw}");
    assert!(queries[0]["min_micros"].is_number(), "min_micros numeric: {raw}");
    assert_eq!(queries[1]["name"], "b_ask", "second query name: {raw}");
    assert_eq!(queries[1]["rows"], 1, "second query rows (ASK): {raw}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// [OPUS-4.8] (sq-d7d) A bare `--json` with no path is a usage error (exit 2), mirroring the
/// rest of the CLI's value-flag contract.
#[test]
fn bench_json_flag_requires_a_path() {
    let dir = scratch("bench-json-nopath");
    let data = write(&dir, "data.nt", NT);
    let qd = dir.join("q");
    std::fs::create_dir_all(&qd).unwrap();
    write(&qd, "a_all.rq", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }");

    let (code, _stdout, stderr) = run3(&["bench", s(&data), "ntriples", s(&qd), "1", "--json"]);
    assert_eq!(code, 2, "bare --json is a usage error; stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compare_compress_reports_footprint() {
    let dir = scratch("compare-compress");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["compare-compress", s(&data), "ntriples"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("index footprint"), "stdout: {stdout}");
    assert!(stdout.contains("triples"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. Error / exit-code contract — the previously-untested surface.
//    The contract: bad usage -> exit 2 ; runtime failure -> exit 1 ; success -> 0.
// ---------------------------------------------------------------------------

#[test]
fn malformed_query_exits_nonzero_with_useful_stderr() {
    let dir = scratch("err-badquery");
    let data = write(&dir, "data.nt", NT);
    let (code, _stdout, stderr) = run3(&["query", s(&data), "ntriples", "SELECT ?s WHERE { this is not sparql"]);
    assert_ne!(code, 0, "a malformed query must NOT exit 0");
    assert_eq!(code, 1, "parse failure is a runtime error (exit 1); stderr: {stderr}");
    assert!(
        stderr.to_lowercase().contains("error"),
        "stderr should explain the failure: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_input_file_exits_nonzero() {
    let dir = scratch("err-missing");
    let missing = dir.join("does-not-exist.nt");
    let (code, _stdout, stderr) = run3(&["query", s(&missing), "ntriples", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_ne!(code, 0, "a missing input file must NOT exit 0");
    assert_eq!(code, 1, "missing file is a runtime error (exit 1); stderr: {stderr}");
    assert!(!stderr.is_empty(), "stderr should report the missing file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsupported_format_exits_nonzero() {
    // [OPUS-4.8] Regression guard for the fixed bug: before the load_quiet format
    // guard, an unknown format silently parsed the input as Turtle and exited 0.
    let dir = scratch("err-format");
    let data = write(&dir, "data.nt", NT);
    let (code, _stdout, stderr) = run3(&["query", s(&data), "bogusfmt", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_ne!(code, 0, "an unsupported format must NOT exit 0 (it silently parsed as Turtle before the fix)");
    assert_eq!(code, 2, "unsupported format is a usage error (exit 2); stderr: {stderr}");
    assert!(
        stderr.contains("unknown format"),
        "stderr should name the unknown format: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_unsupported_format_exits_nonzero() {
    // [OPUS-4.8] `build` calls Graph::build_external directly (not load_quiet); guard against
    // the same silent-Turtle-fallback bug there too (sq-q50l).
    let dir = scratch("err-buildformat");
    let data = write(&dir, "data.nt", NT);
    let idx = dir.join("idx");
    let (code, _stdout, stderr) = run3(&["build", s(&data), "bogusfmt", s(&idx)]);
    assert_eq!(code, 2, "build with unsupported format must exit 2; stderr: {stderr}");
    assert!(stderr.contains("unknown format"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn construct_via_query_subcommand_emits_triples() {
    // [OPUS-4.8] (sq-l4ki) CONSTRUCT via `query` now produces the resulting triples as
    // N-Triples (it used to error with "only SELECT and ASK queries are supported", exit 1).
    let dir = scratch("construct");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["query", s(&data), "ntriples", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"]);
    assert_eq!(code, 0, "CONSTRUCT via `query` should succeed; stderr: {stderr}");
    // The three source triples come back as canonical N-Triples lines (`s p o .`).
    let lines: Vec<&str> = stdout.lines().filter(|l| l.trim_end().ends_with(" .")).collect();
    assert_eq!(lines.len(), 3, "expected 3 N-Triples lines; stdout: {stdout}");
    assert!(
        stdout.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob> ."),
        "stdout should contain the constructed triple: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn describe_via_query_subcommand_emits_triples() {
    // [OPUS-4.8] (sq-l4ki) DESCRIBE is the other graph form — it returns the resource's
    // concise bounded description as N-Triples (also previously a runtime error).
    let dir = scratch("describe");
    let data = write(&dir, "data.nt", NT);
    let (code, stdout, stderr) = run3(&["query", s(&data), "ntriples", "DESCRIBE <http://ex/alice>"]);
    assert_eq!(code, 0, "DESCRIBE via `query` should succeed; stderr: {stderr}");
    // alice has two outgoing triples (knows bob, age 30).
    assert!(stdout.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob> ."), "stdout: {stdout}");
    assert!(stdout.contains("<http://ex/alice> <http://ex/age> \"30\" ."), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_subcommand_prints_usage_and_exits_2() {
    let (code, _stdout, stderr) = run3(&[]);
    assert_eq!(code, 2, "no subcommand must exit 2 (usage error)");
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn unknown_subcommand_prints_usage_and_exits_2() {
    let (code, _stdout, stderr) = run3(&["frobnicate"]);
    assert_eq!(code, 2, "unknown subcommand must exit 2 (usage error)");
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn query_too_few_args_exits_2() {
    let dir = scratch("err-args");
    let data = write(&dir, "data.nt", NT);
    let (code, _stdout, stderr) = run3(&["query", s(&data)]); // missing format + sparql
    assert_eq!(code, 2, "too few args is a usage error (exit 2)");
    assert!(stderr.contains("usage: sparq-cli query"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_mmap_missing_dir_exits_1() {
    let dir = scratch("err-mmapdir");
    let missing = dir.join("no-such-index");
    let (code, _stdout, stderr) = run3(&["query-mmap", s(&missing), "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"]);
    assert_eq!(code, 1, "opening a non-existent index dir is a runtime error (exit 1)");
    assert!(stderr.contains("open error"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bench_unknown_mode_exits_2() {
    let dir = scratch("err-benchmode");
    let data = write(&dir, "data.nt", NT);
    let qd = dir.join("q");
    std::fs::create_dir_all(&qd).unwrap();
    write(&qd, "a.rq", "ASK { ?s ?p ?o }");
    let (code, _stdout, stderr) = run3(&["bench", s(&data), "ntriples", s(&qd), "1", "badmode"]);
    assert_eq!(code, 2, "an unknown bench mode is a usage error (exit 2); stderr: {stderr}");
    assert!(stderr.contains("unknown mode"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reason_unknown_profile_exits_2() {
    let dir = scratch("err-profile");
    let data = write(&dir, "data.nt", NT);
    let (code, _stdout, stderr) = run3(&["reason", s(&data), "ntriples", "bogusprofile"]);
    assert_eq!(code, 2, "an unknown reasoning profile is a usage error (exit 2); stderr: {stderr}");
    assert!(
        stderr.to_lowercase().contains("unknown reasoning profile"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recompress_same_src_dst_exits_2() {
    let dir = scratch("err-recompress");
    let same = dir.join("same");
    let (code, _stdout, stderr) = run3(&["recompress", s(&same), s(&same)]);
    assert_eq!(code, 2, "src == dst is a usage error (exit 2)");
    assert!(stderr.contains("dirs must differ"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Algebra-rewrite pass is LIT in this binary (sq-7d3dj.30.13) [FABLE-5]
// ---------------------------------------------------------------------------

/// [FABLE-5] (sq-7d3dj.30.13) COMPILE-TIME + behavioral tripwire that the CLI's
/// `sparq-engine` dependency keeps `algebra-rewrite` enabled (Cargo.toml dep features,
/// like `dp-planner`). `sparq_engine::rewrite` only exists under the feature, so if a
/// future edit drops it from the dep line this test FAILS TO COMPILE in the per-crate
/// lanes (`cargo test -p sparq-cli`). NB: under a whole-`--workspace` build, feature
/// unification could satisfy this via a sibling crate — the per-crate CI lanes
/// (coverage, feature-matrix) are where this tripwire is authoritative.
///
/// Behavioral half: the motivating SP2Bench q03-shape `FILTER(?v = <iri>)` must actually
/// be REWRITTEN (constant folded into the pattern — the algebra changes), while a
/// literal equality must be left VERBATIM (the sq-lr2ii avoidance contract: `=` on
/// literals is value-equality, never substituted).
#[test]
fn algebra_rewrite_pass_is_lit_and_fires_on_iri_equality() {
    use spargebra::SparqlParser;
    let parse = |q: &str| SparqlParser::new().parse_query(q).expect("parse");
    // (a) IRI equality — the rewrite MUST fire (algebra differs from the verbatim parse).
    let iri_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) }";
    let rewritten = sparq_engine::rewrite::rewrite_query(parse(iri_eq));
    assert_ne!(
        format!("{:?}", rewritten),
        format!("{:?}", parse(iri_eq)),
        "FILTER(?v = <iri>) should be constant-substituted by the rewrite pass"
    );
    // (b) literal equality — NEVER rewritten (value-equality; sq-lr2ii avoidance contract).
    let lit_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = 1) }";
    let verbatim = sparq_engine::rewrite::rewrite_query(parse(lit_eq));
    assert_eq!(
        format!("{:?}", verbatim),
        format!("{:?}", parse(lit_eq)),
        "literal equality must be left verbatim (sq-lr2ii avoidance contract)"
    );
}

/// [FABLE-5] (sq-7d3dj.30.13) End-to-end through the BUILT binary: the q03-shape query
/// (`FILTER(?v = <iri>)`) returns exactly the correct rows — i.e. the rewritten plan the
/// shipped CLI now executes is result-correct on the shape that motivated lighting it.
#[test]
fn query_iri_equality_filter_rows_correct_through_rewrite() {
    let dir = scratch("rewrite-iri-eq");
    let data = write(
        &dir,
        "data.nt",
        "<http://ex/a> <http://ex/p> <http://ex/target> .\n\
         <http://ex/b> <http://ex/p> <http://ex/other> .\n\
         <http://ex/c> <http://ex/p> <http://ex/target> .\n",
    );
    let (code, stdout, stderr) = run3(&[
        "query",
        s(&data),
        "ntriples",
        "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) } ORDER BY ?s",
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("<http://ex/a>"), "row a expected: {stdout}");
    assert!(stdout.contains("<http://ex/c>"), "row c expected: {stdout}");
    assert!(!stdout.contains("<http://ex/b>"), "row b must be filtered out: {stdout}");
    assert!(stdout.contains("2 row(s)"), "exactly two rows: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [OPUS-4.8] (sq-7d3dj.30.20) COMPILE-TIME tripwire that the CLI's `sparq-engine` dependency
/// keeps the SP2Bench-q07 `antijoin-static-decline` fix enabled (Cargo.toml dep features, like
/// `algebra-rewrite`/`dp-planner`). `theta_antijoin_testing::early_declined` only exists under
/// that feature, so if a future edit drops it from the dep line this test FAILS TO COMPILE in
/// the per-crate lanes (`cargo test -p sparq-cli`) — the same authoritative-per-crate caveat as
/// `algebra_rewrite_pass_is_lit_and_fires_on_iri_equality` above. It also asserts the counter
/// starts at zero (a live, callable feature surface), never a stale value.
#[test]
fn antijoin_static_decline_is_lit() {
    sparq_engine::theta_antijoin_testing::reset_stats();
    assert_eq!(
        sparq_engine::theta_antijoin_testing::early_declined(),
        0,
        "the antijoin-static-decline early-decline counter is live and reset-to-zero"
    );
}

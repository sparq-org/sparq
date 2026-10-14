//! The HDT loader hook end-to-end: the actual sparq-cli binary loading a real
//! .hdt archive (and its gzipped form) and answering a SPARQL query.
//! Compiled and run only with the opt-in `hdt` cargo feature:
//! `cargo test -p sparq-cli --features hdt`.
#![cfg(feature = "hdt")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sparq-hdt/tests/fixtures/snikmeta.hdt")
}

const COUNT_ALL: &str = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";

/// (sq-w6ri) The default `query` SELECT path (no `--count`) renders a readable
/// ASCII table footed by `(<N> row(s))` — the canonical wording documented in
/// `cli_contract.rs` (sq-l4ki). The snikmeta fixture has 328 triples, so a `SELECT ?s ?p ?o`
/// yields 328 rows. The constant asserts the *full parenthesized* footer `(328 row(s))` —
/// exactly the substring `select_to_table` emits via `writeln!(out, "({} row(s))", ..)` —
/// rather than the bare `328 row(s)`, so the test is pinned to the precise contract.
/// (`solutions` is reserved for the `--count` count-only output, which these tests do not
/// exercise.)
const EXPECTED_FOOTER: &str = "(328 row(s))";

/// Runs `sparq-cli query <path> <format> <sparql>` and returns (status-ok, stdout, stderr).
fn run_query(path: &Path, format: &str) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sparq-cli"))
        .args(["query", path.to_str().unwrap(), format, COUNT_ALL])
        .output()
        .expect("spawning sparq-cli");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn query_loads_hdt_via_explicit_format_and_extension() {
    let hdt = fixture();
    assert!(
        hdt.exists(),
        "snikmeta fixture missing at {}",
        hdt.display()
    );

    // Explicit `hdt` format argument.
    let (ok, stdout, stderr) = run_query(&hdt, "hdt");
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains(EXPECTED_FOOTER),
        "stdout: {stdout}\nstderr: {stderr}"
    );

    // Extension-driven detection: a bogus format argument is overridden by `.hdt`.
    let (ok, stdout, _) = run_query(&hdt, "ntriples");
    assert!(ok);
    assert!(stdout.contains(EXPECTED_FOOTER), "stdout: {stdout}");
}

#[test]
fn query_loads_gzipped_hdt() {
    use std::io::Write;
    let hdt = fixture();
    assert!(hdt.exists());
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sparq-cli-hdt");
    std::fs::create_dir_all(&dir).unwrap();
    let gz_path = dir.join("snikmeta.hdt.gz");
    let mut enc = flate2::write::GzEncoder::new(
        std::fs::File::create(&gz_path).unwrap(),
        flate2::Compression::fast(),
    );
    enc.write_all(&std::fs::read(&hdt).unwrap()).unwrap();
    enc.finish().unwrap();

    let (ok, stdout, stderr) = run_query(&gz_path, "hdt");
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains(EXPECTED_FOOTER), "stdout: {stdout}");
}

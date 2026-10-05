//! [FABLE-5] (sq-lsp7k.8, epic sq-lsp7k) The `tabular` subcommand end-to-end: CSV direct
//! mapping + the R2RML materializing subset over CSV logical tables, driven through the
//! *built* binary (`CARGO_BIN_EXE_sparq-cli`, the same mechanism as `cli_contract.rs`).
//!
//! The whole subcommand lives behind the opt-in `tabular` feature, so this file compiles
//! and runs only under it: `cargo test -p sparq-cli --features tabular`.
//!
//! The R2RML fixture suite under `tests/fixtures/r2rml/<case>/` is data-driven: each case
//! holds a `mapping.ttl`, its CSV logical table(s), and the exact `expected.nt` output
//! (compared as a sorted, deduplicated triple SET — R2RML output is a graph). Cases named
//! `tc….` are CSV-adaptations of the corresponding W3C R2RML test cases (SQL logical
//! tables replaced by CSV files, as the mapping headers document); the rest pin the same
//! constructs the W3C suite exercises (NULLs, percent-encoding, constants, blank nodes,
//! language tags, multiple tables).

#![cfg(feature = "tabular")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sparq-cli")).args(args).output().expect("spawning sparq-cli")
}

/// (exit code, stdout, stderr) from the built binary; a signal death reports as -1.
fn run3(args: &[&str]) -> (i32, String, String) {
    let out = run(args);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tabular-cli-{tag}"));
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

/// Sorted, deduplicated non-empty lines — triple SET comparison.
fn triple_set(nt: &str) -> Vec<String> {
    let mut v: Vec<String> = nt.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect();
    v.sort();
    v.dedup();
    v
}

const PEOPLE_CSV: &str = "name,age\nalice,34\nbob,28\n";

// ---------------------------------------------------------------------------
// Direct mapping
// ---------------------------------------------------------------------------

/// The acceptance one-shot: a plain CSV loads via direct mapping and is queried in the
/// same invocation — datatype inference makes `age` an xsd:integer, so a numeric FILTER
/// works without casts.
#[test]
fn direct_one_shot_query() {
    let dir = scratch("direct-query");
    let csv = write(&dir, "people.csv", PEOPLE_CSV);
    let (code, out, err) = run3(&[
        "tabular",
        s(&csv),
        "--query",
        "SELECT ?n WHERE { ?s <http://example.com/people#age> ?a ; <http://example.com/people#name> ?n . FILTER(?a > 30) }",
        "--format",
        "tsv",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(err.contains("loaded 6 triples"), "2 rows x (2 cols + rdf:type): {err}");
    assert!(out.contains("\"alice\""), "{out}");
    assert!(!out.contains("\"bob\""), "{out}");
}

/// `--out` streams the exact direct-mapping N-Triples (no graph build): full contract —
/// row-template subjects, per-column predicates, rdf:type, inference, NULL skip.
#[test]
fn direct_out_exact_triples() {
    let dir = scratch("direct-out");
    let csv = write(&dir, "emp.csv", "id,name,score\n1,Ann,4.5\n2,,true\n");
    let out_nt = dir.join("out.nt");
    let (code, _, err) = run3(&["tabular", s(&csv), "--out", s(&out_nt)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(err.contains("wrote 7 triples"), "{err}");
    let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
    let expected = triple_set(
        r#"<http://example.com/emp/row/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/emp> .
<http://example.com/emp/row/1> <http://example.com/emp#id> "1"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://example.com/emp/row/1> <http://example.com/emp#name> "Ann" .
<http://example.com/emp/row/1> <http://example.com/emp#score> "4.5"^^<http://www.w3.org/2001/XMLSchema#decimal> .
<http://example.com/emp/row/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/emp> .
<http://example.com/emp/row/2> <http://example.com/emp#id> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://example.com/emp/row/2> <http://example.com/emp#score> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> ."#,
    );
    assert_eq!(got, expected);
}

/// `--template`/`--class none`/`--no-infer`/`--base` reshape the direct mapping.
#[test]
fn direct_flags_template_class_none_no_infer() {
    let dir = scratch("direct-flags");
    let csv = write(&dir, "emp.csv", "ID,age\n7,41\n");
    let out_nt = dir.join("out.nt");
    let (code, _, err) = run3(&[
        "tabular",
        s(&csv),
        "--base",
        "http://ex.org/",
        "--template",
        "http://ex.org/emp/{ID}",
        "--class",
        "none",
        "--no-infer",
        "--out",
        s(&out_nt),
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
    let expected = triple_set(
        r#"<http://ex.org/emp/7> <http://ex.org/emp#ID> "7" .
<http://ex.org/emp/7> <http://ex.org/emp#age> "41" ."#,
    );
    assert_eq!(got, expected);
}

/// Compressed inputs stream through the same transparent decompression as every other
/// ingest path: `.csv.gz` and `.csv.zst` load identically to the plain file.
#[test]
fn direct_compressed_inputs_gz_and_zst() {
    let dir = scratch("direct-compressed");
    let gz = dir.join("people.csv.gz");
    let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::default());
    enc.write_all(PEOPLE_CSV.as_bytes()).unwrap();
    enc.finish().unwrap();
    let zst = dir.join("people.csv.zst");
    let mut enc = zstd::stream::write::Encoder::new(std::fs::File::create(&zst).unwrap(), 0).unwrap();
    enc.write_all(PEOPLE_CSV.as_bytes()).unwrap();
    enc.finish().unwrap();
    for path in [&gz, &zst] {
        let (code, out, err) = run3(&[
            "tabular",
            s(path),
            "--query",
            "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }",
            "--format",
            "tsv",
        ]);
        assert_eq!(code, 0, "{path:?} stderr: {err}");
        // table name = stem WITHOUT the compression + format extensions
        assert!(err.contains("loaded 6 triples"), "{path:?}: {err}");
        assert!(out.contains("6"), "{out}");
    }
}

/// `--out people.nt.gz` writes a valid gzip N-Triples archive (finished trailer, not a
/// truncated stream).
#[test]
fn direct_out_gz_roundtrip() {
    let dir = scratch("direct-out-gz");
    let csv = write(&dir, "t.csv", "a\n1\n");
    let out_gz = dir.join("t.nt.gz");
    let (code, _, err) = run3(&["tabular", s(&csv), "--class", "none", "--out", s(&out_gz)]);
    assert_eq!(code, 0, "stderr: {err}");
    let mut nt = String::new();
    let mut dec = flate2::read::MultiGzDecoder::new(std::fs::File::open(&out_gz).unwrap());
    std::io::Read::read_to_string(&mut dec, &mut nt).unwrap();
    assert_eq!(
        triple_set(&nt),
        triple_set(r#"<http://example.com/t/row/1> <http://example.com/t#a> "1"^^<http://www.w3.org/2001/XMLSchema#integer> ."#)
    );
}

// ---------------------------------------------------------------------------
// R2RML fixture suite
// ---------------------------------------------------------------------------

/// Data-driven R2RML suite: every `tests/fixtures/r2rml/<case>/` runs through the binary
/// (`--mapping … --out …`) and must produce EXACTLY the `expected.nt` triple set.
#[test]
fn r2rml_fixture_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r2rml");
    let scratch = scratch("r2rml-suite");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("fixtures dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(cases.len() >= 10, "expected the full fixture suite, found {}", cases.len());
    for case in cases {
        let name = case.file_name().unwrap().to_str().unwrap().to_owned();
        let mapping = case.join("mapping.ttl");
        let expected = std::fs::read_to_string(case.join("expected.nt")).expect("expected.nt");
        let mut csvs: Vec<PathBuf> = std::fs::read_dir(&case)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "csv"))
            .collect();
        csvs.sort();
        let out_nt = scratch.join(format!("{name}.nt"));
        let mut args: Vec<String> = vec!["tabular".into()];
        args.extend(csvs.iter().map(|p| s(p).to_owned()));
        args.extend(["--mapping".into(), s(&mapping).to_owned(), "--out".into(), s(&out_nt).to_owned()]);
        let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, _, err) = run3(&argrefs);
        assert_eq!(code, 0, "case {name}: stderr: {err}");
        let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
        assert_eq!(got, triple_set(&expected), "case {name} diverged");
    }
}

/// The R2RML path also LOADS and queries in one shot (not just `--out`).
#[test]
fn r2rml_one_shot_query() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r2rml/tc0002a-columns-typed");
    let (code, out, err) = run3(&[
        "tabular",
        s(&root.join("student.csv")),
        "--mapping",
        s(&root.join("mapping.ttl")),
        "--query",
        "SELECT ?n WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?n }",
        "--format",
        "tsv",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("\"Venus\""), "{out}");
}

/// Explicit `<name>=<path>` positional binding overrides the file-stem table name.
#[test]
fn r2rml_explicit_table_binding() {
    let dir = scratch("r2rml-binding");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r2rml/tc0001a-template-class");
    // Copy the CSV under a NON-matching name; bind it explicitly to the `student` table.
    let renamed = dir.join("other-name.csv");
    std::fs::copy(root.join("student.csv"), &renamed).unwrap();
    let binding = format!("student={}", s(&renamed));
    let out_nt = dir.join("out.nt");
    let (code, _, err) =
        run3(&["tabular", &binding, "--mapping", s(&root.join("mapping.ttl")), "--out", s(&out_nt)]);
    assert_eq!(code, 0, "stderr: {err}");
    let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
    assert_eq!(got.len(), 1, "{got:?}");
}

// ---------------------------------------------------------------------------
// [OPUS-5] (sq-u1z86) joins, named graphs, row provenance
// ---------------------------------------------------------------------------

/// The join fixture is executed through the binary by `r2rml_fixture_suite`; here the same
/// mapping is LOADED and queried, so the join survives the ingest as well as the emitter.
#[test]
fn r2rml_join_one_shot_query() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r2rml/join");
    let (code, out, err) = run3(&[
        "tabular",
        s(&root.join("sport.csv")),
        s(&root.join("student.csv")),
        "--mapping",
        s(&root.join("mapping.ttl")),
        "--query",
        "SELECT ?n WHERE { ?s <http://example.com/plays> <http://example.com/sport/100> ; <http://example.com/name> ?n }",
        "--format",
        "tsv",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("\"Venus\""), "{out}");
    assert!(!out.contains("\"Fred\""), "only the joining row: {out}");
}

const GRAPH_MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#T> rr:logicalTable [ rr:tableName "t" ] ;
  rr:subjectMap [ rr:template "http://example.com/r/{id}" ; rr:graphMap [ rr:template "http://example.com/g/{cat}" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:v ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// `rr:graphMap` switches the emitter to N-Quads: `--out` writes real quads (and the summary
/// line says "quads", not "triples").
#[test]
fn r2rml_graph_map_out_nquads() {
    let dir = scratch("r2rml-graphmap-out");
    let csv = write(&dir, "t.csv", "id,v,cat\n1,x,red\n2,y,blue\n");
    let mapping = write(&dir, "m.ttl", GRAPH_MAPPING);
    let out_nq = dir.join("out.nq");
    let (code, _, err) = run3(&["tabular", s(&csv), "--mapping", s(&mapping), "--out", s(&out_nq)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(err.contains("wrote 2 quads"), "quad wording: {err}");
    let got = triple_set(&std::fs::read_to_string(&out_nq).unwrap());
    let expected = triple_set(
        r#"<http://example.com/r/1> <http://example.com/v> "x" <http://example.com/g/red> .
<http://example.com/r/2> <http://example.com/v> "y" <http://example.com/g/blue> ."#,
    );
    assert_eq!(got, expected);
}

/// …and the LOAD path takes the dataset loader, so the named graphs are queryable with
/// `GRAPH ?g { … }` rather than being flattened into the default graph.
#[test]
fn r2rml_graph_map_loads_named_graphs() {
    let dir = scratch("r2rml-graphmap-query");
    let csv = write(&dir, "t.csv", "id,v,cat\n1,x,red\n2,y,blue\n");
    let mapping = write(&dir, "m.ttl", GRAPH_MAPPING);
    let (code, out, err) = run3(&[
        "tabular",
        s(&csv),
        "--mapping",
        s(&mapping),
        "--query",
        "SELECT ?g WHERE { GRAPH ?g { ?s <http://example.com/v> \"x\" } }",
        "--format",
        "tsv",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(err.contains("loaded 2 quads"), "{err}");
    assert!(out.contains("<http://example.com/g/red>"), "named graph must survive the load: {out}");
    assert!(!out.contains("g/blue"), "{out}");
}

/// `--row-provenance` links each subject back to its source row, in both modes.
#[test]
fn row_provenance_direct_and_r2rml() {
    let dir = scratch("row-provenance");
    let csv = write(&dir, "emp.csv", "ID,name\n7,Ann\n");
    let out_nt = dir.join("direct.nt");
    let (code, _, err) = run3(&[
        "tabular",
        s(&csv),
        "--template",
        "http://example.com/emp/{ID}",
        "--class",
        "none",
        "--row-provenance",
        "--out",
        s(&out_nt),
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
    assert!(
        got.contains(
            &"<http://example.com/emp/7> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://example.com/emp/row/1> ."
                .to_string()
        ),
        "{got:?}"
    );

    let mapping = write(
        &dir,
        "m.ttl",
        "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n@prefix ex: <http://example.com/> .\n\
         <#T> rr:logicalTable [ rr:tableName \"emp\" ] ;\n\
           rr:subjectMap [ rr:template \"http://example.com/e/{ID}\" ] ;\n\
           rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column \"name\" ] ] .\n",
    );
    let out_nt = dir.join("r2rml.nt");
    let (code, _, err) =
        run3(&["tabular", s(&csv), "--mapping", s(&mapping), "--row-provenance", "--out", s(&out_nt)]);
    assert_eq!(code, 0, "stderr: {err}");
    let got = triple_set(&std::fs::read_to_string(&out_nt).unwrap());
    assert_eq!(
        got,
        triple_set(
            r#"<http://example.com/e/7> <http://example.com/name> "Ann" .
<http://example.com/e/7> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://example.com/emp/row/1> ."#
        )
    );
}

// ---------------------------------------------------------------------------
// Loud failures
// ---------------------------------------------------------------------------

#[test]
fn errors_are_loud_and_typed() {
    let dir = scratch("errors");
    let csv = write(&dir, "t.csv", "a,b\n1,2\n");

    // usage errors -> exit 2
    let (code, _, _) = run3(&["tabular"]);
    assert_eq!(code, 2, "no files is a usage error");
    let (code, _, err) = run3(&["tabular", s(&csv), "--bogus"]);
    assert_eq!(code, 2, "unknown flag: {err}");
    let mapping = write(&dir, "m.ttl", "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n");
    let (code, _, err) = run3(&["tabular", s(&csv), "--mapping", s(&mapping), "--no-infer"]);
    assert_eq!(code, 2, "direct-mapping flag with --mapping: {err}");
    assert!(err.contains("direct-mapping"), "{err}");

    // data/mapping errors -> exit 1
    let ragged = write(&dir, "ragged.csv", "a,b\n1\n");
    let (code, _, err) = run3(&["tabular", s(&ragged), "--out", s(&dir.join("x.nt"))]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("ragged"), "{err}");

    let bad_map = write(
        &dir,
        "bad.ttl",
        "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n<#T> rr:logicalTable [ rr:tableName \"t\" ] ;\n rr:subjectMap [ rr:template \"http://e/{missing}\" ] .\n",
    );
    let (code, _, err) = run3(&["tabular", s(&csv), "--mapping", s(&bad_map), "--out", s(&dir.join("y.nt"))]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("not found"), "{err}");

    let sql_map = write(
        &dir,
        "sql.ttl",
        "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n<#T> rr:logicalTable [ rr:sqlQuery \"SELECT 1\" ] ;\n rr:subjectMap [ rr:template \"http://e/x\" ] .\n",
    );
    let (code, _, err) = run3(&["tabular", s(&csv), "--mapping", s(&sql_map), "--out", s(&dir.join("z.nt"))]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("rr:sqlQuery"), "{err}");
}

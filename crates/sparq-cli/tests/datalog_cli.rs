//! [SONNET-4.6] (sq-p4zci, design record `research/stratified-datalog-rules.md` §6 item 6) The
//! stratified-Datalog CLI surface end-to-end: `--reason datalog:<rules.dlog>` on `query` and
//! `reason <f> <fmt> datalog:<rules.dlog> [out.nt]`.
//!
//! Spawns the *built* `sparq-cli` binary (via `CARGO_BIN_EXE_sparq-cli`, the same mechanism as
//! `el_cli.rs` / `dump_cli.rs`). The surface lives behind the opt-in `datalog` feature, so this
//! whole file compiles and runs only under it: `cargo test -p sparq-cli --features datalog`.
//! The feature-OFF half of the contract (a `--reason datalog:…` that refuses to fall back to a
//! monotone profile) is pinned separately in `error_paths_cli.rs`, which runs in the DEFAULT
//! build.
#![cfg(feature = "datalog")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh scratch dir under the cargo per-test-target tmp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("datalog-cli-{tag}"));
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
    p.to_str().expect("UTF-8 fixture path")
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

/// Four nodes; `a` has three outgoing edges, `b` has one, `c`/`d` have none.
const GRAPH: &str = "\
<http://ex/a> <http://ex/edge> <http://ex/b> .
<http://ex/a> <http://ex/edge> <http://ex/c> .
<http://ex/a> <http://ex/edge> <http://ex/d> .
<http://ex/b> <http://ex/edge> <http://ex/c> .
<http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Node> .
<http://ex/b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Node> .
<http://ex/c> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Node> .
<http://ex/d> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Node> .
";

/// The design-record §2 headline program: an `AGGREGATE` degree, a `FILTER` threshold on it, and
/// a `NOT` over the class the threshold derives — three strata, exercising exactly the two things
/// (aggregation + negation as failure) that make this NOT expressible as an RDFS/OWL-RL profile.
///
/// Over `GRAPH`: `a` has degree 3 (→ `ex:Hub`), `b` degree 1, `c`/`d` no edges at all (no
/// aggregate row). So `ex:Leaf` = {b, c, d} and `ex:Hub` = {a}.
const RULES: &str = r#"@prefix ex: <http://ex/> .
[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .
[?x, a, ex:Hub]  :- [?x, ex:deg, ?c], FILTER(?c >= 3) .
[?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] .
"#;

const TYPE: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";

/// `reason <f> <fmt> datalog:<rules>` runs the program and writes the closure; the derived
/// `ex:Hub` / `ex:Leaf` facts are exactly the perfect model of the stratified program.
#[test]
fn reason_datalog_writes_the_stratified_closure() {
    let dir = scratch("reason-closure");
    let data = write(&dir, "graph.nt", GRAPH);
    let rules = write(&dir, "rules.dlog", RULES);
    let out = dir.join("closure.nt");
    let profile = format!("datalog:{}", s(&rules));

    let (code, stdout, stderr) = run3(&["reason", s(&data), "ntriples", &profile, s(&out)]);
    assert_eq!(code, 0, "stderr: {stderr}");
    // The stratification report: 3 rules across 3 strata (deg < Hub < Leaf).
    assert!(stderr.contains("3 rule(s) in 3 stratum/strata"), "stderr: {stderr}");
    assert!(stdout.contains("triples after"), "stdout: {stdout}");

    let closure = std::fs::read_to_string(&out).expect("read closure");
    // Aggregation: `a` has three edges.
    assert!(
        closure.contains("<http://ex/a> <http://ex/deg> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> ."),
        "closure: {closure}"
    );
    // FILTER threshold over the aggregate.
    assert!(closure.contains(&format!("<http://ex/a> {TYPE} <http://ex/Hub> .")), "closure: {closure}");
    // Negation as failure against the completed lower stratum: everything that is NOT a Hub.
    for leaf in ["b", "c", "d"] {
        assert!(
            closure.contains(&format!("<http://ex/{leaf}> {TYPE} <http://ex/Leaf> .")),
            "expected {leaf} to be a Leaf; closure: {closure}"
        );
    }
    assert!(!closure.contains(&format!("<http://ex/a> {TYPE} <http://ex/Leaf> .")), "closure: {closure}");
    // The input facts survive into the closure (it is a superset of the EDB).
    assert!(closure.contains("<http://ex/a> <http://ex/edge> <http://ex/b> ."), "closure: {closure}");
    let _ = std::fs::remove_dir_all(dir);
}

/// `query --reason datalog:<rules>` hands the closure to the ordinary query path, so the derived
/// facts are visible to plain BGP evaluation — the point of the surface.
#[test]
fn query_reason_datalog_sees_derived_facts() {
    let dir = scratch("query-derived");
    let data = write(&dir, "graph.nt", GRAPH);
    let rules = write(&dir, "rules.dlog", RULES);
    let profile = format!("datalog:{}", s(&rules));

    let (code, stdout, stderr) = run3(&[
        "query",
        s(&data),
        "ntriples",
        "SELECT ?x WHERE { ?x a <http://ex/Leaf> }",
        "--reason",
        &profile,
        "--format",
        "tsv",
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for leaf in ["b", "c", "d"] {
        assert!(stdout.contains(&format!("<http://ex/{leaf}>")), "stdout: {stdout}");
    }
    assert!(!stdout.contains("<http://ex/a>"), "the Hub must not be a Leaf; stdout: {stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A program with a recursion cycle THROUGH negation has no stratified model. The checker must
/// reject it loudly (exit 1) rather than hand back some derivation-order-dependent fixpoint.
#[test]
fn unstratifiable_program_is_rejected() {
    let dir = scratch("unstratifiable");
    let data = write(&dir, "graph.nt", GRAPH);
    // `ex:P` is derived from the ABSENCE of `ex:Q`, and `ex:Q` from the absence of `ex:P`.
    let rules = write(
        &dir,
        "cycle.dlog",
        "@prefix ex: <http://ex/> .\n\
         [?x, ex:p, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:q, \"y\"] .\n\
         [?x, ex:q, \"y\"] :- [?x, a, ex:Node], NOT [?x, ex:p, \"y\"] .\n",
    );
    let profile = format!("datalog:{}", s(&rules));
    let (code, _stdout, stderr) = run3(&["reason", s(&data), "ntriples", &profile]);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(stderr.contains("datalog stratification error"), "stderr: {stderr}");
    // The checker names a predicate ON the cycle, so the message is actionable.
    assert!(stderr.contains("http://ex/q") || stderr.contains("http://ex/p"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A rule document outside the documented fragment is a loud exit-1 error naming BOTH the rules
/// file and the offending construct — not a silently-empty or silently-truncated program. The
/// fixture is range-restriction (safety): `?z` appears in the head but no positive body atom
/// binds it, so there is no finite set of facts to derive.
#[test]
fn out_of_fragment_rules_exit_1() {
    let dir = scratch("out-of-fragment-rules");
    let data = write(&dir, "graph.nt", GRAPH);
    let rules = write(&dir, "unsafe.dlog", "@prefix ex: <http://ex/> .\n[?x, ex:p, ?z] :- [?x, ex:edge, ?y] .\n");
    let profile = format!("datalog:{}", s(&rules));
    let (code, _stdout, stderr) = run3(&["reason", s(&data), "ntriples", &profile]);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(stderr.contains("datalog rule error in"), "stderr: {stderr}");
    assert!(stderr.contains("unsafe.dlog"), "the rules FILE must be named; stderr: {stderr}");
    assert!(stderr.contains("head variable `?z`"), "the CONSTRUCT must be named; stderr: {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A missing rules file is an exit-1 I/O error naming the rules path — distinguishable from a
/// missing DATA file, which is the far more confusing thing to report.
#[test]
fn missing_rules_file_exits_1() {
    let dir = scratch("missing-rules");
    let data = write(&dir, "graph.nt", GRAPH);
    let (code, _stdout, stderr) = run3(&["reason", s(&data), "ntriples", "datalog:/no/such/rules.dlog"]);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(stderr.contains("error reading datalog rules /no/such/rules.dlog"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

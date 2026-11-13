//! Additional implementation-report lanes. Fetched inputs require explicit roots.
#[path = "support/earl.rs"]
mod earl;
#[path = "support/earl_fixture.rs"]
mod earl_fixture;

use sparq_conformance::inference::{
    n3_suite, rdfmt,
    report::{Outcome, TestResult},
};
use sparq_conformance::{
    line_syntax::{self, LineSuite},
    turtle_suite,
};
use std::path::PathBuf;

// Measured passing counts; XML floors apply with the shipping parser enabled.
const RDF12_XML_FLOOR: usize = 29;
const RDF11_XML_FLOOR: usize = 164;
const RDF12_SEMANTICS_FLOOR: usize = 14;
const RDF11_TURTLE_FLOOR: usize = 313;
const RDF11_SEMANTICS_FLOOR: usize = 48;
const N3_REASONER_FLOOR: usize = 83;
const N3_PARSER_FLOOR: usize = 212;
const N3_EXTENDED_FLOOR: usize = 871;
const N3_TURTLE_FLOOR: usize = 297;

fn root(key: &str) -> Option<PathBuf> {
    let Some(path) = std::env::var_os(key).map(PathBuf::from) else {
        assert!(
            std::env::var_os("SPARQ_EARL_OUT").is_none(),
            "{key} required for EARL"
        );
        eprintln!("SKIP: {key} unset; fetch with earl-report.sh");
        return None;
    };
    Some(path.canonicalize().expect("explicit suite root must exist"))
}

fn tally(rows: &[(String, TestResult)], floor: usize, regressions: &mut Vec<String>) {
    earl::emit(
        rows.iter()
            .map(|(id, r)| (id.as_str(), matches!(r.outcome, Outcome::Pass))),
    );
    let pass = rows
        .iter()
        .filter(|(_, r)| matches!(r.outcome, Outcome::Pass))
        .count();
    let label = &rows.first().expect("suite must contain tests").1.suite;
    println!("TOTAL {label} {pass} {} (floor {floor})", rows.len() - pass);
    for (id, r) in rows {
        if !matches!(r.outcome, Outcome::Pass) {
            eprintln!("FAIL {id}: {:?}", r.outcome);
        }
    }
    if pass < floor {
        regressions.push(format!("{label}: {pass} < {floor}"));
    }
}

fn complete(marker: &str, regressions: Vec<String>) {
    if std::env::var_os("SPARQ_EARL_OUT").is_some() {
        println!("EARL complete {marker}");
    }
    assert!(
        regressions.is_empty(),
        "ratchets regressed: {regressions:?}"
    );
}

fn xml(root: &std::path::Path, version: &str) -> Vec<(String, TestResult)> {
    let lane = if version == "rdf12" {
        "rdf-xml/eval"
    } else {
        "rdf-xml"
    };
    line_syntax::run_manifest(
        LineSuite::RdfXml,
        &root
            .join("rdf")
            .join(version)
            .join(lane)
            .join("manifest.ttl"),
        Some(&format!(
            "https://w3c.github.io/rdf-tests/rdf/{version}/{lane}/"
        )),
    )
    .expect("RDF/XML manifest ran")
}

// Measured at rdf-tests 7b5afb91857a9c282c03d2e45d3f76485797c901.
// Without rdfxml every entry fails, including negative syntax: no parser was run.
fn xml_floor(version: &str) -> usize {
    if !cfg!(feature = "rdfxml") {
        0
    } else if version == "rdf12" {
        RDF12_XML_FLOOR
    } else {
        RDF11_XML_FLOOR
    }
}

#[test]
fn rdfxml_ratchets() {
    let Some(root) = root("SPARQ_RDF_TESTS_ROOT") else {
        return;
    };
    let mut regressions = Vec::new();
    for version in ["rdf12", "rdf11"] {
        tally(&xml(&root, version), xml_floor(version), &mut regressions);
    }
    complete("rdfxml", regressions);
}

#[test]
fn semantics_ratchet() {
    let Some(root) = root("SPARQ_RDF_TESTS_ROOT") else {
        return;
    };
    let rows = rdfmt::run_manifest(
        &root.join("rdf/rdf12/rdf-semantics/manifest.ttl"),
        "https://w3c.github.io/rdf-tests/rdf/rdf12/rdf-semantics/",
    )
    .expect("semantics manifest ran");
    let mut regressions = Vec::new();
    tally(&rows, RDF12_SEMANTICS_FLOOR, &mut regressions);
    complete("semantics", regressions);
}

#[test]
fn rdf11_ratchets() {
    let Some(root) = root("SPARQ_RDF_TESTS_ROOT") else {
        return;
    };
    let mut regressions = Vec::new();
    // Existing RDF 1.1 floors are unchanged.
    for (dir, suite, floor) in [
        ("rdf-turtle", None, RDF11_TURTLE_FLOOR),
        ("rdf-trig", Some(LineSuite::TriG), 356),
        ("rdf-n-triples", Some(LineSuite::NTriples), 70),
        ("rdf-n-quads", Some(LineSuite::NQuads), 87),
    ] {
        let base = format!("https://w3c.github.io/rdf-tests/rdf/rdf11/{dir}/");
        let manifest = root.join("rdf/rdf11").join(dir).join("manifest.ttl");
        let rows = match suite {
            None => turtle_suite::run_manifest(&manifest, Some(&base)),
            Some(suite) => line_syntax::run_manifest(suite, &manifest, Some(&base)),
        }
        .expect("RDF 1.1 syntax manifest ran");
        tally(&rows, floor, &mut regressions);
    }
    tally(&xml(&root, "rdf11"), xml_floor("rdf11"), &mut regressions);
    let rows = rdfmt::run_manifest(
        &root.join("rdf/rdf11/rdf-mt/manifest.ttl"),
        "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-mt/",
    )
    .expect("RDF 1.1 semantics ran");
    tally(&rows, RDF11_SEMANTICS_FLOOR, &mut regressions);
    complete("rdf11", regressions);
}

#[test]
fn n3_ratchets() {
    let Some(root) = root("SPARQ_N3_ROOT") else {
        return;
    };
    let rows = n3_suite::run_suite_with_ids(&root).expect("N3 manifests ran");
    let mut regressions = Vec::new();
    // Measured at N3 1d29dea774d3e5a97883ebbc8ab033a176ac11b5.
    for (suite, floor) in [
        ("n3/reasoner", N3_REASONER_FLOOR),
        ("n3/parser", N3_PARSER_FLOOR),
        ("n3/extended", N3_EXTENDED_FLOOR),
        ("n3/turtle", N3_TURTLE_FLOOR),
    ] {
        let selected: Vec<_> = rows
            .iter()
            .filter(|(_, r)| r.suite == suite)
            .cloned()
            .collect();
        tally(&selected, floor, &mut regressions);
    }
    complete("n3", regressions);
}

#[test]
fn reports_every_new_lane_even_when_ratchets_fail() {
    let fixture = earl_fixture::Fixture::new();
    let mut xml_expected = Vec::new();
    let mut rdf11_expected = Vec::new();
    for (dir, stem) in [
        ("rdf12/rdf-xml/eval", "TestXML"),
        ("rdf11/rdf-xml", "TestXML"),
        ("rdf11/rdf-turtle", "TestTurtle"),
        ("rdf11/rdf-trig", "TestTrig"),
        ("rdf11/rdf-n-triples", "TestNTriples"),
        ("rdf11/rdf-n-quads", "TestNQuads"),
    ] {
        fixture.write(
            &format!("rdf/{dir}/manifest.ttl"),
            format!(
                r#"
            @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
            @prefix rdft: <http://www.w3.org/ns/rdftest#> .
            <> a mf:Manifest; mf:entries (<#ok> <#missing> <#unknown> <#withdrawn>) .
            <#ok> a rdft:{stem}Eval; mf:action <input>; mf:result <expected.nt> .
            <#missing> a rdft:{stem}NegativeSyntax; mf:action <absent> .
            <#unknown> a rdft:FutureTest; mf:action <input> .
            <#withdrawn> a rdft:{stem}Eval; rdft:approval rdft:Withdrawn .
        "#
            ),
        );
        let input = if stem == "TestXML" {
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:e="https://example.test/">
                <rdf:Description rdf:about="https://example.test/s"><e:p rdf:resource="https://example.test/o"/></rdf:Description>
            </rdf:RDF>"#
        } else {
            "<https://example.test/s> <https://example.test/p> <https://example.test/o> ."
        };
        fixture.write(&format!("rdf/{dir}/input"), input);
        fixture.write(
            &format!("rdf/{dir}/expected.nt"),
            "<https://example.test/s> <https://example.test/p> <https://example.test/o> .\n",
        );
        let expected: Vec<_> = [
            ("ok", stem != "TestXML" || cfg!(feature = "rdfxml")),
            ("missing", false),
            ("unknown", false),
        ]
        .into_iter()
        .map(|(id, pass)| {
            (
                format!("https://w3c.github.io/rdf-tests/rdf/{dir}/manifest.ttl#{id}"),
                pass,
            )
        })
        .collect();
        if stem == "TestXML" {
            xml_expected.extend(expected.clone());
        }
        if dir.starts_with("rdf11/") {
            rdf11_expected.extend(expected);
        }
    }
    let mut semantics_expected = Vec::new();
    for dir in ["rdf12/rdf-semantics", "rdf11/rdf-mt"] {
        fixture.write(
            &format!("rdf/{dir}/manifest.ttl"),
            r#"
            @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
            <> a mf:Manifest; mf:entries (<#ok> <#missing> <#unknown>) .
            <#ok> a mf:PositiveEntailmentTest; mf:action <input.ttl>; mf:result <input.ttl> .
            <#missing> a mf:NegativeEntailmentTest; mf:action <absent.ttl>; mf:result false .
            <#unknown> a mf:FutureTest .
        "#,
        );
        fixture.write(
            &format!("rdf/{dir}/input.ttl"),
            "<https://example.test/s> <https://example.test/p> <https://example.test/o> .",
        );
        let expected: Vec<_> = [("ok", true), ("missing", false), ("unknown", false)]
            .into_iter()
            .map(|(id, pass)| {
                (
                    format!("https://w3c.github.io/rdf-tests/rdf/{dir}/manifest.ttl#{id}"),
                    pass,
                )
            })
            .collect();
        if dir.starts_with("rdf11/") {
            rdf11_expected.extend(expected);
        } else {
            semantics_expected.extend(expected);
        }
    }
    let roots = [("SPARQ_RDF_TESTS_ROOT", fixture.root())];
    fixture.assert_report_status(
        "rdfxml_ratchets",
        "EARL complete rdfxml",
        &roots,
        &xml_expected,
        !cfg!(feature = "rdfxml"),
    );
    fixture.assert_report(
        "semantics_ratchet",
        "EARL complete semantics",
        &roots,
        &semantics_expected,
    );
    fixture.assert_report(
        "rdf11_ratchets",
        "EARL complete rdf11",
        &roots,
        &rdf11_expected,
    );

    let mut n3_expected = Vec::new();
    for (manifest, kind) in [
        ("N3Tests/manifest-reasoner.ttl", "test:TestN3Reason"),
        ("N3Tests/manifest-parser.ttl", "test:TestN3Eval"),
        ("N3Tests/manifest-extended.ttl", "test:TestN3Eval"),
        ("TurtleTests/manifest.ttl", "rdft:TestTurtleEval"),
    ] {
        fixture.write(
            &format!("tests/{manifest}"),
            format!(
                r#"
            @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
            @prefix rdft: <http://www.w3.org/ns/rdftest#> .
            @prefix test: <https://w3c.github.io/N3/tests/test.n3#> .
            <> a mf:Manifest; mf:entries (<#ok> <#missing> <#unknown> <#rejected>) .
            <#ok> a {kind}; mf:action <input.n3>; mf:result <input.n3> .
            <#missing> a {kind} .
            <#unknown> a test:FutureTest; mf:action <input.n3> .
            <#rejected> a {kind}; rdft:approval rdft:Rejected .
        "#
            ),
        );
        let dir = manifest.rsplit_once('/').unwrap().0;
        fixture.write(
            &format!("tests/{dir}/input.n3"),
            "<https://example.test/s> <https://example.test/p> <https://example.test/o> .",
        );
        n3_expected.extend(
            [("ok", true), ("missing", false), ("unknown", false)]
                .into_iter()
                .map(|(id, pass)| {
                    (
                        format!("https://w3c.github.io/N3/tests/{manifest}#{id}"),
                        pass,
                    )
                }),
        );
    }
    fixture.assert_report(
        "n3_ratchets",
        "EARL complete n3",
        &[("SPARQ_N3_ROOT", fixture.root())],
        &n3_expected,
    );
}

#[test]
fn missing_or_incomplete_inputs_never_complete_an_earl_run() {
    let fixture = earl_fixture::Fixture::new();
    for manifest in [
        "rdf/rdf12/rdf-xml/eval/manifest.ttl",
        "rdf/rdf11/rdf-xml/manifest.ttl",
        "rdf/rdf12/rdf-semantics/manifest.ttl",
        "rdf/rdf11/rdf-turtle/manifest.ttl",
        "tests/N3Tests/manifest-reasoner.ttl",
        "tests/N3Tests/manifest-parser.ttl",
        "tests/N3Tests/manifest-extended.ttl",
        "tests/TurtleTests/manifest.ttl",
    ] {
        fixture.write(
            manifest,
            r#"
            @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            <> a mf:Manifest; mf:entries [rdf:first <#unrunnable>; rdf:rest <#missing-tail>] .
        "#,
        );
    }
    for (runner, key) in [
        ("rdfxml_ratchets", "SPARQ_RDF_TESTS_ROOT"),
        ("semantics_ratchet", "SPARQ_RDF_TESTS_ROOT"),
        ("rdf11_ratchets", "SPARQ_RDF_TESTS_ROOT"),
        ("n3_ratchets", "SPARQ_N3_ROOT"),
    ] {
        for input in [
            None,
            Some(fixture.root().join("absent")),
            Some(fixture.root().to_owned()),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", runner, "--nocapture"])
                .env_remove(key)
                .env("SPARQ_EARL_OUT", fixture.root().join("outcomes.tsv"));
            if let Some(input) = input {
                command.env(key, input);
            }
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{runner} must require its root");
            assert!(!String::from_utf8_lossy(&output.stdout).contains("EARL complete"));
            assert!(!fixture.root().join("outcomes.tsv").exists());
        }
    }
}

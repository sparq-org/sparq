//! RDF 1.2 manifest outcomes and measured pass-count floors.

#[path = "support/earl.rs"]
mod earl;

use sparq_conformance::inference::report::Outcome;
use sparq_conformance::line_syntax::{self, LineSuite};
use sparq_conformance::turtle_suite;
use std::path::PathBuf;

#[test]
fn reports_every_rdf12_lane_even_when_ratchets_fail() {
    let fixture = earl::Fixture::new();
    let mut expected = Vec::new();
    for &(dir, lane, _) in sparq_conformance::floors::rdf12::LANES {
        let stem = match dir {
            "rdf-turtle" => "TestTurtle",
            "rdf-trig" => "TestTrig",
            "rdf-n-triples" => "TestNTriples",
            _ => "TestNQuads",
        };
        let kind = match lane {
            "eval" => "Eval",
            "c14n" => "PositiveC14N",
            _ => "PositiveSyntax",
        };
        let base = format!("https://w3c.github.io/rdf-tests/rdf/rdf12/{dir}/{lane}");
        fixture.write(
            &format!("{dir}/{lane}/manifest.ttl"),
            format!(
                r#"
            @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
            @prefix rdft: <http://www.w3.org/ns/rdftest#> .
            @prefix : <{base}#> .
            :manifest mf:assumedTestBase <{base}/>; mf:entries (:ok :missing :withdrawn) .
            :ok a rdft:{stem}{kind}; mf:action <input>; mf:result <expected.nq> .
            :missing a rdft:{stem}NegativeSyntax; mf:action <absent> .
            :withdrawn a rdft:{stem}PositiveSyntax; rdft:approval rdft:Withdrawn .
        "#
            ),
        );
        for name in ["input", "expected.nq"] {
            fixture.write(
                &format!("{dir}/{lane}/{name}"),
                "<https://example.test/s> <https://example.test/p> <https://example.test/o> .\n",
            );
        }
        expected.extend([
            (format!("{base}#ok"), true),
            (format!("{base}#missing"), false),
        ]);
    }
    fixture.assert_report(
        "rdf12_syntax_ratchets",
        "EARL complete rdf12",
        &[("SPARQ_RDF12_ROOT", fixture.root())],
        &expected,
    );
}

#[test]
fn rdf12_syntax_ratchets() {
    // The suite is fetched outside the repository (earl-report.sh, CI), so it is only read
    // from an explicit root; without one the ratchet self-skips unless an EARL run needs it.
    let Some(root) = std::env::var_os("SPARQ_RDF12_ROOT").map(PathBuf::from) else {
        assert!(
            std::env::var_os("SPARQ_EARL_OUT").is_none(),
            "RDF 1.2 suite missing"
        );
        eprintln!(
            "SKIP: SPARQ_RDF12_ROOT unset; run crates/sparq-conformance/earl-report.sh rdf12"
        );
        return;
    };
    assert!(root.is_dir(), "RDF 1.2 suite missing at {}", root.display());
    let mut regressions = Vec::new();
    for &(dir, lane, floor) in sparq_conformance::floors::rdf12::LANES {
        let manifest = root.join(dir).join(lane).join("manifest.ttl");
        let results = match dir {
            "rdf-turtle" => turtle_suite::run_manifest(&manifest, None),
            "rdf-trig" => line_syntax::run_manifest(LineSuite::TriG, &manifest, None),
            "rdf-n-triples" => line_syntax::run_manifest(LineSuite::NTriples, &manifest, None),
            _ => line_syntax::run_manifest(LineSuite::NQuads, &manifest, None),
        }
        .expect("RDF 1.2 manifest ran");
        earl::emit(
            results
                .iter()
                .map(|(id, r)| (id.as_str(), matches!(r.outcome, Outcome::Pass))),
        );
        let pass = results
            .iter()
            .filter(|(_, r)| matches!(r.outcome, Outcome::Pass))
            .count();
        println!(
            "TOTAL {dir}/{lane} {pass} {} (floor {floor})",
            results.len() - pass
        );
        for (id, r) in &results {
            if let Outcome::Fail(why) = &r.outcome {
                eprintln!("FAIL {id}: {why}");
            }
        }
        if pass < floor {
            regressions.push(format!("{dir}/{lane}: {pass} < {floor}"));
        }
    }
    if std::env::var_os("SPARQ_EARL_OUT").is_some() {
        println!("EARL complete rdf12");
    }
    assert!(
        regressions.is_empty(),
        "RDF 1.2 ratchets regressed: {regressions:?}"
    );
}

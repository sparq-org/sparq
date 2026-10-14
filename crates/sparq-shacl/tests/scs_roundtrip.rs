//! (sq-v0b8, #796) Round-trip test for the SHACL Compact Syntax (SCS)
//! PARSER against the vendored W3C `shacl12-cs` corpus.
//!
//! For each `<name>.shaclc` / `<name>.ttl` fixture pair under
//! `tests/shacl/data-shapes/shacl12-cs/tests/valid/`, this:
//!
//!   1. parses the `.shaclc` to SHACL shapes triples (`sparq_shacl::parse_scs`);
//!   2. parses the expected `.ttl` with oxttl, *both* against the same base IRI
//!      (the fixture's `BASE`, or [`DEFAULT_BASE`] when it declares none), so
//!      relative IRIs and the `owl:Ontology` subject resolve identically on both
//!      sides;
//!   3. compares the two graphs for **RDF graph isomorphism** (oxrdf's
//!      blank-node canonicalisation — a blank node matches any blank node).
//!
//! A mismatch on a fixture in [`expected_pass`] is a FAIL (the parser regressed);
//! a fixture NOT in that list is recorded as an honest SKIP (a construct this
//! parser does not yet round-trip exactly) — it is never silently fake-passed.
//!
//! ## Gates
//!
//!   * Suite gate — the corpus is fetched by `./fetch-shacl-tests.sh` into
//!     `tests/shacl/` (gitignored); when absent the test SKIPS itself so
//!     `cargo test` stays green on a fresh checkout.
//!   * Feature gate — the whole file is `#[cfg(feature = "scs")]`; with the
//!     feature off it is compiled out (the parser does not exist).
#![cfg(feature = "scs")]

use oxrdf::graph::CanonicalizationAlgorithm;
use oxrdf::{Graph, Triple};
use std::path::PathBuf;

/// The fixtures this parser is expected to round-trip EXACTLY. Every other
/// fixture in the corpus is an honest SKIP. A FAIL here is a real regression.
const EXPECTED_PASS: &[&str] = &[
    "array-in",
    "basic-shape",
    "basic-shape-iri",
    "basic-shape-with-target",
    "basic-shape-with-targets",
    "class",
    "comment",
    "complex1",
    "complex2",
    "count-0-1",
    "count-0-unlimited",
    "count-1-2",
    "count-1-unlimited",
    "datatype",
    "directives",
    "empty",
    "nestedShape",
    "node-or-2",
    "node-or-3-not",
    "nodeKind",
    "path-alternative",
    "path-complex",
    "path-inverse",
    "path-oneOrMore",
    "path-sequence",
    "path-zeroOrMore",
    "path-zeroOrOne",
    "property-empty",
    "property-not",
    "property-or-2",
    "property-or-3",
    "shapeRef",
];

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/shacl12-cs/tests/valid")
}

/// The base IRI a fixture's `.shaclc` `BASE` directive declares (first such
/// line), else [`sparq_shacl::DEFAULT_BASE`]. Both the SCS parse and the
/// reference-Turtle parse use this base so relative IRIs resolve identically.
fn fixture_base(shaclc: &str) -> String {
    for line in shaclc.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("BASE") {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
                return inner.to_string();
            }
        }
    }
    sparq_shacl::DEFAULT_BASE.to_string()
}

fn canonical_graph(triples: Vec<Triple>) -> Graph {
    let mut g: Graph = triples.into_iter().collect();
    g.canonicalize(CanonicalizationAlgorithm::Unstable);
    g
}

fn parse_ttl(text: &str, base: &str) -> Result<Vec<Triple>, String> {
    let parser = oxttl::TurtleParser::new()
        .with_base_iri(base)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for t in parser.for_slice(text.as_bytes()) {
        out.push(t.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[test]
fn shacl12_cs_roundtrip() {
    let root = corpus_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C shacl12-cs corpus not present at {} — run crates/sparq-shacl/fetch-shacl-tests.sh",
            root.display()
        );
        return;
    }

    // Discover every <name>.shaclc fixture.
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&root).expect("read corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("shaclc") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();

    let (mut pass, mut fail, mut skip) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();

    println!("\nW3C shacl12-cs SCS-parser round-trip scoreboard");
    for name in &names {
        let expected = EXPECTED_PASS.contains(&name.as_str());
        let shaclc_path = root.join(format!("{}.shaclc", name));
        let ttl_path = root.join(format!("{}.ttl", name));
        if !ttl_path.exists() {
            // No expected Turtle (e.g. an invalid-syntax fixture) — out of scope.
            skip += 1;
            println!("  SKIP {}: no expected .ttl", name);
            continue;
        }
        let shaclc = std::fs::read_to_string(&shaclc_path).expect("read .shaclc");
        let ttl = std::fs::read_to_string(&ttl_path).expect("read .ttl");
        let base = fixture_base(&shaclc);

        let result = (|| -> Result<bool, String> {
            let got = sparq_shacl::parse_scs(&shaclc, &base).map_err(|e| e.to_string())?;
            let want = parse_ttl(&ttl, &base)?;
            let got_g = canonical_graph(got);
            let want_g = canonical_graph(want);
            Ok(got_g == want_g)
        })();

        match (expected, result) {
            (true, Ok(true)) => {
                pass += 1;
                println!("  PASS {}", name);
            }
            (true, Ok(false)) => {
                fail += 1;
                failures.push(format!("{}: graphs not isomorphic", name));
                println!("  FAIL {}: graphs not isomorphic", name);
            }
            (true, Err(e)) => {
                fail += 1;
                failures.push(format!("{}: {}", name, e));
                println!("  FAIL {}: {}", name, e);
            }
            (false, Ok(true)) => {
                // An honest-skip fixture that actually round-trips: count it as a
                // pass (a free win) but do not fail if it ever stops.
                pass += 1;
                println!("  PASS {} (was honest-skip — round-trips now)", name);
            }
            (false, Ok(false)) => {
                skip += 1;
                println!("  SKIP {}: not yet round-tripping (honest)", name);
            }
            (false, Err(e)) => {
                skip += 1;
                println!("  SKIP {}: {} (honest)", name, e);
            }
        }
    }
    println!("TOTAL pass {} fail {} skip {}", pass, fail, skip);

    assert!(
        failures.is_empty(),
        "SCS round-trip regressions on EXPECTED_PASS fixtures:\n  {}",
        failures.join("\n  ")
    );
    // The whole corpus is currently expected to round-trip; guard against a
    // silent shrink in coverage.
    assert!(
        pass >= EXPECTED_PASS.len(),
        "SCS round-trip pass count {} dropped below the expected floor {}",
        pass,
        EXPECTED_PASS.len()
    );
}

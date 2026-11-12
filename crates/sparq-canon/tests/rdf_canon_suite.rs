//! W3C RDF Dataset Canonicalization (RDFC-1.0) test suite, manifest-driven
//! over the vendored snapshot (`tests/rdf-canon-testdata/`, see PROVENANCE.md).
//!
//! This drives sparq-canon's **public API** (`canonicalize` / `issue_quads` /
//! the `*_with` SHA-384 profile, plus the single-graph `canonicalize_triples`
//! the ZK commitment pipeline calls) over the official vectors — so the same
//! oxrdf-0.3 <-> oxrdf-0.2 bridge the crate ships is what gets validated. A
//! bridge bug fails the suite exactly like a canonicalization bug would.
//!
//! The manifest itself is parsed with sparq (dogfooding the engine).
//!
//! Test types:
//! - `rdfc:RDFC10EvalTest`: canonical N-Quads must match byte-for-byte.
//! - `rdfc:RDFC10MapTest`: the issued identifier map must match.
//! - `rdfc:RDFC10NegativeEvalTest`: canonicalization must FAIL (poison graphs;
//!   the HNDQ call-limit guard).

#[path = "../../sparq-conformance/tests/support/earl.rs"]
mod earl;

use oxrdf::Triple;
use sparq_core::Graph;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn testdata() -> PathBuf {
    if let Some(root) = std::env::var_os("SPARQ_RDF_CANON_ROOT") {
        return root.into();
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rdf-canon-testdata")
}

struct Entry {
    name: String,
    kind: String,
    action: String,
    result: Option<String>,
    /// `rdfc:hashAlgorithm` (test075 is parameterized over SHA-384).
    hash_algorithm: Option<String>,
}

/// Reads the manifest with sparq itself.
fn manifest_entries() -> Vec<Entry> {
    let ttl = std::fs::read_to_string(testdata().join("manifest.ttl")).unwrap();
    // The manifest uses relative IRIs; anchor them at the canonical W3C base
    // (the vendored file is untouched — the base is prepended at read time).
    let ttl = format!("@base <https://w3c.github.io/rdf-canon/tests/> .\n{ttl}");
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let q = r#"
        PREFIX mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#>
        PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
        PREFIX rdfc: <https://w3c.github.io/rdf-canon/tests/vocab#>
        SELECT ?test ?type ?action ?result ?hash WHERE {
            ?manifest mf:entries/rdf:rest*/rdf:first ?test .
            OPTIONAL { ?test a ?type }
            OPTIONAL { ?test mf:action ?action }
            OPTIONAL { ?test mf:result ?result }
            OPTIONAL { ?test rdfc:hashAlgorithm ?hash }
        } ORDER BY ?test
    "#;
    let res = sparq_engine::query(&g, q).unwrap();
    let mut out = Vec::new();
    for row in &res.rows {
        let iri = |i: usize| -> Option<String> {
            match &row[i] {
                Some(oxrdf::Term::NamedNode(n)) => Some(n.as_str().to_string()),
                _ => None,
            }
        };
        let hash_algorithm = match &row[4] {
            Some(oxrdf::Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        };
        out.push(Entry {
            name: iri(0).unwrap(),
            kind: iri(1).unwrap_or_default(),
            action: iri(2).unwrap_or_default(),
            result: iri(3),
            hash_algorithm,
        });
    }
    if std::env::var_os("SPARQ_RDF_CANON_ROOT").is_none() {
        assert_eq!(out.len(), 86, "expected the full 86-entry manifest");
    }
    out
}

/// Loads a test input `.nq` as oxrdf 0.3 quads (suite inputs include both
/// default-graph and named-graph datasets; the public API canonicalizes at the
/// dataset/quad level).
fn load_quads(path: &Path) -> Result<Vec<oxrdf::Quad>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    oxttl::NQuadsParser::new()
        .for_slice(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Canonicalize through the crate's PUBLIC dataset API (default SHA-256, or the
/// `*_with` SHA-384 profile for the parameterized cases).
fn canonicalize(quads: &[oxrdf::Quad], sha384: bool) -> Result<String, String> {
    if sha384 {
        sparq_canon::canonicalize_quads_with::<sha2::Sha384>(quads).map_err(|e| e.to_string())
    } else {
        sparq_canon::canonicalize(quads).map_err(|e| e.to_string())
    }
}

fn issue(quads: &[oxrdf::Quad], sha384: bool) -> Result<HashMap<String, String>, String> {
    if sha384 {
        sparq_canon::issue_quads_with::<sha2::Sha384>(quads).map_err(|e| e.to_string())
    } else {
        sparq_canon::issued_identifiers(quads).map_err(|e| e.to_string())
    }
}

fn local_path(iri: &str) -> PathBuf {
    let tail = iri.rsplit_once("rdfc10/").map(|(_, t)| t).unwrap_or(iri);
    testdata().join("rdfc10").join(tail)
}

fn run_entry(e: &Entry) -> Result<(), String> {
    let input = load_quads(&local_path(&e.action))?;
    let sha384 = match e.hash_algorithm.as_deref() {
        None | Some("SHA256") => false,
        Some("SHA384") => true,
        Some(other) => return Err(format!("unsupported hash algorithm {other}")),
    };
    let expected = || -> Result<String, String> {
        let result = e.result.as_deref().ok_or("missing mf:result")?;
        std::fs::read_to_string(local_path(result)).map_err(|e| e.to_string())
    };
    match e.kind.rsplit('#').next().unwrap_or("") {
        "RDFC10EvalTest" => {
            let want = expected()?;
            let got = canonicalize(&input, sha384)?;
            if got == want {
                Ok(())
            } else {
                Err("canonical output mismatch".into())
            }
        }
        "RDFC10MapTest" => {
            let want: HashMap<String, String> =
                serde_json::from_str(&expected()?).map_err(|e| e.to_string())?;
            if issue(&input, sha384)? == want {
                Ok(())
            } else {
                Err("issued map mismatch".into())
            }
        }
        "RDFC10NegativeEvalTest" => match canonicalize(&input, sha384) {
            Err(_) => Ok(()),
            Ok(_) => Err("poison graph canonicalized (must fail closed)".into()),
        },
        other => Err(format!("unknown test type {other}")),
    }
}

#[test]
fn w3c_rdf_canon_suite() {
    let entries = manifest_entries();
    let (mut eval, mut map, mut neg) = (0, 0, 0);
    let mut failures: Vec<String> = Vec::new();

    let mut outcomes = Vec::new();
    for e in &entries {
        match e.kind.rsplit('#').next().unwrap_or("") {
            "RDFC10EvalTest" => eval += 1,
            "RDFC10MapTest" => map += 1,
            "RDFC10NegativeEvalTest" => neg += 1,
            _ => {}
        }
        // Fixture errors and unexpected panics are failures, including negative tests.
        let outcome = std::panic::catch_unwind(|| run_entry(e))
            .unwrap_or_else(|_| Err("runner panicked".into()));
        if let Err(why) = &outcome {
            failures.push(format!("{}: {why}", e.name));
        }
        outcomes.push((e.name.as_str(), outcome.is_ok()));
    }
    earl::emit(outcomes.iter().copied());
    if std::env::var_os("SPARQ_EARL_OUT").is_some() {
        println!(
            "TOTAL rdf-canon {} {}",
            entries.len() - failures.len(),
            failures.len()
        );
        println!("EARL complete rdf-canon");
    }
    assert_eq!((eval, map, neg), (64, 21, 1), "suite composition drifted");
    assert!(
        failures.is_empty(),
        "{} of {} W3C rdf-canon tests failed:\n{}",
        failures.len(),
        entries.len(),
        failures.join("\n")
    );
}

/// The single-graph API (`canonicalize_triples`, what the ZK commitment
/// pipeline calls) must agree with the suite on every default-graph-only eval
/// test under the default SHA-256.
#[test]
fn default_graph_tests_via_single_graph_api() {
    let entries = manifest_entries();
    let mut covered = 0;
    for e in &entries {
        if !e.kind.ends_with("RDFC10EvalTest") || e.hash_algorithm.is_some() {
            continue; // the commitment pipeline canonicalizes with the default SHA-256
        }
        let quads = load_quads(&local_path(&e.action)).unwrap();
        if !quads
            .iter()
            .all(|q| q.graph_name == oxrdf::GraphName::DefaultGraph)
        {
            continue; // named-graph datasets are not single-graph content
        }
        let triples: Vec<Triple> = quads
            .iter()
            .map(|q| Triple::new(q.subject.clone(), q.predicate.clone(), q.object.clone()))
            .collect();
        let expected = std::fs::read_to_string(local_path(e.result.as_ref().unwrap())).unwrap();
        let got = sparq_canon::canonicalize_triples(&triples)
            .unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert_eq!(got.to_nquads(), expected, "{} via single-graph API", e.name);
        covered += 1;
    }
    assert!(
        covered >= 40,
        "expected most eval tests to be default-graph-only, got {covered}"
    );
}

#[test]
fn unreadable_negative_fixture_is_a_failure() {
    let e = Entry {
        name: "https://example.test/manifest#missing".into(),
        kind: "https://w3c.github.io/rdf-canon/tests/vocab#RDFC10NegativeEvalTest".into(),
        action: format!("rdfc10/absent-{}.nq", std::process::id()),
        result: None,
        hash_algorithm: None,
    };
    let why = run_entry(&e).expect_err("a missing negative fixture must fail");
    assert!(
        why.starts_with("read "),
        "fixture error must not be a canonicalization rejection: {why}"
    );
}

#[test]
fn reports_canon_entries_even_when_ratchet_fails() {
    let fixture = earl::Fixture::new();
    fixture.write(
        "manifest.ttl",
        r#"
        @prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
        @prefix rdfc: <https://w3c.github.io/rdf-canon/tests/vocab#> .
        <> mf:entries (<#ok> <#missing> <#unknown>) .
        <#ok> a rdfc:RDFC10EvalTest; mf:action <rdfc10/input.nq>; mf:result <rdfc10/expected.nq> .
        <#missing> a rdfc:RDFC10NegativeEvalTest; mf:action <rdfc10/absent.nq> .
        <#unknown> a rdfc:UnknownTest; mf:action <rdfc10/input.nq> .
    "#,
    );
    for name in ["input.nq", "expected.nq"] {
        fixture.write(
            &format!("rdfc10/{name}"),
            "<https://example.test/s> <https://example.test/p> <https://example.test/o> .\n",
        );
    }
    fixture.assert_report(
        "w3c_rdf_canon_suite",
        "EARL complete rdf-canon",
        &[("SPARQ_RDF_CANON_ROOT", fixture.root())],
        &[
            ("https://w3c.github.io/rdf-canon/tests/#ok".into(), true),
            (
                "https://w3c.github.io/rdf-canon/tests/#missing".into(),
                false,
            ),
            (
                "https://w3c.github.io/rdf-canon/tests/#unknown".into(),
                false,
            ),
        ],
    );
}

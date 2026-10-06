//! RDF 1.1 Semantics entailment suite (w3c/rdf-tests `rdf/rdf11/rdf-mt`):
//! simple / RDF / RDFS entailment with per-test recognized-datatype sets.
//!
//! Test shape: `mf:action` = premise document; `mf:result` = either a
//! conclusion document (the premise must / must-not entail it) or the literal
//! `false` (the premise must / must-not be INCONSISTENT). `mf:entailmentRegime`
//! ("simple" | "RDF" | "RDFS") picks the closure; `mf:recognizedDatatypes`
//! lists D.

use super::entail::{self, Recognized, Regime, Row};
use super::report::{Outcome, TestResult};
use crate::manifest::MF;
use crate::rdf::{as_node, iri_to_path, MiniGraph};
use oxrdf::Term;
use rustc_hash::FxHashSet;
use std::path::Path;

const RDFT: &str = "http://www.w3.org/ns/rdftest#";

pub fn run_suite(rdf_tests_root: &Path, out: &mut Vec<TestResult>) -> Result<(), String> {
    let manifest_path = rdf_tests_root.join("rdf/rdf11/rdf-mt/manifest.ttl");
    let g = MiniGraph::load(&manifest_path)?;
    let manifests = g.subjects_with_type(&format!("{MF}Manifest"));
    for m in &manifests {
        let Some(entries) = g.object(m, &format!("{MF}entries")) else {
            continue;
        };
        for item in g.list(entries) {
            let Some(node) = as_node(&item) else { continue };
            let name = g
                .str_object(&node, &format!("{MF}name"))
                .unwrap_or_else(|| format!("{node}"));
            let types = g.types_of(&node);
            let positive = types
                .iter()
                .any(|t| t == &format!("{MF}PositiveEntailmentTest"));
            let negative = types
                .iter()
                .any(|t| t == &format!("{MF}NegativeEntailmentTest"));
            if !positive && !negative {
                out.push(result(
                    &name,
                    Outcome::OutOfScope(format!(
                        "not an entailment test (type {})",
                        types.first().cloned().unwrap_or_default()
                    )),
                ));
                continue;
            }
            if matches!(
                g.object(&node, &format!("{RDFT}approval")),
                Some(Term::NamedNode(n)) if n.as_str().ends_with("Rejected") || n.as_str().ends_with("Withdrawn")
            ) {
                out.push(result(
                    &name,
                    Outcome::OutOfScope("test rejected/withdrawn upstream".into()),
                ));
                continue;
            }
            let outcome = run_one(&g, &node, positive);
            out.push(result(&name, outcome));
        }
    }
    Ok(())
}

fn result(name: &str, outcome: Outcome) -> TestResult {
    TestResult {
        suite: "rdf-mt".into(),
        name: name.to_string(),
        outcome,
    }
}

fn run_one(g: &MiniGraph, node: &oxrdf::NamedOrBlankNode, positive: bool) -> Outcome {
    // Regime + recognized datatypes.
    let regime = match g
        .str_object(node, &format!("{MF}entailmentRegime"))
        .unwrap_or_else(|| "simple".into())
        .as_str()
    {
        "simple" => Regime::Simple,
        "RDF" => Regime::Rdf,
        "RDFS" => Regime::Rdfs,
        other => return Outcome::OutOfScope(format!("entailment regime {other} not wired")),
    };
    let mut recognized: FxHashSet<String> = FxHashSet::default();
    if let Some(head) = g.object(node, &format!("{MF}recognizedDatatypes")) {
        for t in g.list(head) {
            if let Term::NamedNode(n) = t {
                recognized.insert(n.as_str().to_string());
            }
        }
    }
    let d = Recognized::with_defaults(recognized);

    // Premise.
    let premise_rows: Vec<Row> = match g.object(node, &format!("{MF}action")) {
        Some(Term::NamedNode(n)) => {
            match iri_to_path(n.as_str()).map(|p| crate::rdf::parse_file(&p)) {
                Some(Ok(triples)) => triples.iter().map(entail::triple_row).collect(),
                Some(Err(e)) => return Outcome::Fail(format!("premise parse error: {e}")),
                None => return Outcome::Fail("non-file premise IRI".into()),
            }
        }
        _ => return Outcome::Fail("manifest entry has no mf:action document".into()),
    };

    // Result: conclusion document or boolean `false` (= consistency check).
    enum Goal {
        Conclusion(Vec<Row>),
        Inconsistency,
    }
    let goal = match g.object(node, &format!("{MF}result")) {
        Some(Term::NamedNode(n)) => match iri_to_path(n.as_str())
            .map(|p| crate::rdf::parse_file(&p))
        {
            Some(Ok(triples)) => Goal::Conclusion(triples.iter().map(entail::triple_row).collect()),
            Some(Err(e)) => return Outcome::Fail(format!("conclusion parse error: {e}")),
            None => return Outcome::Fail("non-file conclusion IRI".into()),
        },
        Some(Term::Literal(l)) if l.value() == "false" => Goal::Inconsistency,
        other => return Outcome::Fail(format!("unsupported mf:result {other:?}")),
    };

    let conclusion_rows: &[Row] = match &goal {
        Goal::Conclusion(rows) => rows,
        Goal::Inconsistency => &[],
    };
    let closure = entail::close(&premise_rows, conclusion_rows, regime, &d);
    let inconsistent = entail::inconsistency(&closure, &d);

    match goal {
        Goal::Inconsistency => match (positive, inconsistent) {
            (true, Some(_)) => Outcome::Pass,
            (true, None) => Outcome::Fail("premise not detected as inconsistent".into()),
            (false, None) => Outcome::Pass,
            (false, Some(why)) => {
                Outcome::Fail(format!("premise wrongly judged inconsistent ({why})"))
            }
        },
        Goal::Conclusion(rows) => {
            // An inconsistent premise entails everything.
            let holds = inconsistent.is_some() || entail::entails(&closure, &rows, &d);
            match (positive, holds) {
                (true, true) => Outcome::Pass,
                (true, false) => {
                    Outcome::Fail("conclusion not entailed by the materialized closure".into())
                }
                (false, false) => Outcome::Pass,
                (false, true) => Outcome::Fail("conclusion wrongly entailed".into()),
            }
        }
    }
}

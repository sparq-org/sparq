//! RDF 1.1 and RDF 1.2 Semantics entailment manifests (w3c/rdf-tests):
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
    out.extend(
        run_manifest(
            &manifest_path,
            "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-mt/",
        )?
        .into_iter()
        .map(|(_, r)| r),
    );
    Ok(())
}

/// Run an RDF Semantics manifest against its published document base.
/// Every active entry retains its test IRI; unsupported kinds/regimes fail.
pub fn run_manifest(manifest: &Path, base: &str) -> Result<Vec<(String, TestResult)>, String> {
    let manifest = manifest.canonicalize().map_err(|e| e.to_string())?;
    let root = manifest.parent().ok_or("manifest has no directory")?;
    let g = MiniGraph::load(&manifest)?;
    let mut out = Vec::new();
    let mut seen = FxHashSet::default();
    for m in g.subjects_with_type(&format!("{MF}Manifest")) {
        let entries = g
            .object(&m, &format!("{MF}entries"))
            .ok_or("missing mf:entries")?;
        for item in g.checked_list(entries)? {
            let node = as_node(&item).ok_or("manifest entry is not a resource")?;
            if !seen.insert(node.clone()) {
                continue;
            }
            if g.objects(&node, &format!("{RDFT}approval")).iter().any(
                |o| matches!(o, Term::NamedNode(n) if matches!(n.as_str().strip_prefix(RDFT), Some("Withdrawn" | "Rejected")))) {
                continue;
            }
            let Term::NamedNode(id) = item else {
                return Err("test has no IRI".into());
            };
            let name = g
                .str_object(&node, &format!("{MF}name"))
                .unwrap_or_else(|| id.to_string());
            let types = g.types_of(&node);
            let positive = types
                .iter()
                .any(|t| t == &format!("{MF}PositiveEntailmentTest"));
            let negative = types
                .iter()
                .any(|t| t == &format!("{MF}NegativeEntailmentTest"));
            let outcome = if positive == negative {
                Outcome::Fail(format!("unsupported entailment test types {types:?}"))
            } else {
                run_one(&g, &node, positive, root, base)
            };
            out.push((
                crate::rdf::published_iri(id.as_str(), root, base),
                TestResult {
                    suite: "rdf-mt".into(),
                    name,
                    outcome,
                },
            ));
        }
    }
    if out.is_empty() {
        return Err("manifest has no active entries".into());
    }
    Ok(out)
}

fn parse_document(path: &Path, root: &Path, base: &str) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let document_base = crate::rdf::published_iri(&crate::rdf::file_iri(path), root, base);
    let format = match path.extension().and_then(|s| s.to_str()) {
        Some("nt") => "ntriples",
        Some("rdf") => "rdfxml",
        _ => "turtle",
    };
    let graph = sparq_core::Graph::load_str_with_base(&text, format, &document_base)?;
    Ok(graph
        .iter_ids()
        .map(|row| row.map(|id| graph.dict().term(id)))
        .collect())
}

fn run_one(
    g: &MiniGraph,
    node: &oxrdf::NamedOrBlankNode,
    positive: bool,
    root: &Path,
    base: &str,
) -> Outcome {
    // Regime + recognized datatypes.
    let regime_term = g.object(node, &format!("{MF}entailmentRegime"));
    let regime = match regime_term {
        None => "simple",
        Some(Term::Literal(l)) => l.value(),
        Some(other) => return Outcome::Fail(format!("unsupported entailment regime {other}")),
    };
    let regime = match regime {
        "simple" => Regime::Simple,
        "RDF" => Regime::Rdf,
        "RDFS" => Regime::Rdfs,
        other => return Outcome::Fail(format!("entailment regime {other} not wired")),
    };
    let mut recognized: FxHashSet<String> = FxHashSet::default();
    if let Some(head) = g.object(node, &format!("{MF}recognizedDatatypes")) {
        let datatypes = match g.checked_list(head) {
            Ok(items) => items,
            Err(e) => return Outcome::Fail(format!("invalid mf:recognizedDatatypes: {e}")),
        };
        for t in datatypes {
            let Term::NamedNode(n) = t else {
                return Outcome::Fail("recognized datatype is not an IRI".into());
            };
            recognized.insert(n.as_str().to_string());
        }
    }
    let d = Recognized::with_defaults(recognized);

    // Premise.
    let premise_rows: Vec<Row> = match g.object(node, &format!("{MF}action")) {
        Some(Term::NamedNode(n)) => {
            match iri_to_path(n.as_str()).map(|p| parse_document(&p, root, base)) {
                Some(Ok(triples)) => triples,
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
        Some(Term::NamedNode(n)) => {
            match iri_to_path(n.as_str()).map(|p| parse_document(&p, root, base)) {
                Some(Ok(triples)) => Goal::Conclusion(triples),
                Some(Err(e)) => return Outcome::Fail(format!("conclusion parse error: {e}")),
                None => return Outcome::Fail("non-file conclusion IRI".into()),
            }
        }
        Some(Term::Literal(l))
            if l.datatype() == oxrdf::vocab::xsd::BOOLEAN && matches!(l.value(), "false" | "0") =>
        {
            Goal::Inconsistency
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_ids_and_unrunnable_semantics_are_truthful() {
        let root = std::env::temp_dir().join(format!("sparq-rdfmt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = root.join("manifest.ttl");
        std::fs::write(
            &manifest,
            format!(
                r#"
            @prefix mf: <{MF}> . @prefix rdft: <{RDFT}> .
            <> a mf:Manifest; mf:entries (<#ok> <#unknown> <#regime> <#missing> <#iri-regime> <#withdrawn> <#datatype> <#datatype-list> <#string-result>) .
            <#ok> a mf:PositiveEntailmentTest; mf:action <a.ttl>; mf:result <b.ttl> .
            <#unknown> a mf:NewEntailmentTest .
            <#regime> a mf:NegativeEntailmentTest; mf:entailmentRegime "future" .
            <#missing> a mf:PositiveEntailmentTest; mf:action <absent>; mf:result false .
            <#iri-regime> a mf:PositiveEntailmentTest; mf:entailmentRegime <https://example.test/future>;
                mf:action <a.ttl>; mf:result <b.ttl> .
            <#withdrawn> a mf:PositiveEntailmentTest; rdft:approval rdft:Withdrawn .
            <#datatype> a mf:PositiveEntailmentTest; mf:action <a.ttl>; mf:result <b.ttl>;
                mf:recognizedDatatypes (false) .
            <#datatype-list> a mf:PositiveEntailmentTest; mf:action <a.ttl>; mf:result <b.ttl>;
                mf:recognizedDatatypes <#incomplete> .
            <#string-result> a mf:NegativeEntailmentTest; mf:action <a.ttl>; mf:result "false" .
        "#
            ),
        )
        .unwrap();
        std::fs::write(root.join("a.ttl"), "<s> <p> [ <p> <o> ] .").unwrap();
        std::fs::write(root.join("b.ttl"), "<https://example.test/semantics/s> <https://example.test/semantics/p> _:x . _:x <https://example.test/semantics/p> <https://example.test/semantics/o> .").unwrap();
        let rows = run_manifest(&manifest, "https://example.test/semantics/").unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(rows.len(), 8, "only withdrawn entries omitted");
        assert_eq!(rows[0].0, "https://example.test/semantics/manifest.ttl#ok");
        assert!(
            matches!(rows[0].1.outcome, Outcome::Pass),
            "shipping parser and bnode entailment"
        );
        for (_, row) in &rows[1..] {
            assert!(matches!(row.outcome, Outcome::Fail(_)), "{row:?}");
        }
        assert!(matches!(&rows[2].1.outcome, Outcome::Fail(e) if e.contains("future")));
    }
}

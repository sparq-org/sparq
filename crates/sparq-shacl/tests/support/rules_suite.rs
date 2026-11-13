//! Manifest traversal and graph oracle for the rules suites.
#![allow(dead_code)]
use super::Outcome;
use oxrdf::graph::CanonicalizationAlgorithm;
use oxrdf::{NamedNode, Term, Triple};
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
pub const SHT: &str = "http://www.w3.org/ns/shacl-test#";
pub const SH: &str = "http://www.w3.org/ns/shacl#";
pub const SRLT: &str = "http://www.w3.org/ns/sparql-rl-tests#";
pub fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(s).into()
}
pub fn root(folder: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shacl/data-shapes/shacl12-test-suite/tests")
        .join(folder)
}
pub fn path(term: &Term) -> Result<PathBuf, String> {
    match term {
        Term::NamedNode(n) => n
            .as_str()
            .strip_prefix("file://")
            .map(PathBuf::from)
            .ok_or_else(|| format!("Non-local resource {n}")),
        _ => Err(format!("Expected file IRI: {term}")),
    }
}
pub fn load(file: &Path) -> Result<Graph, String> {
    let file = file.canonicalize().map_err(|e| e.to_string())?;
    sparq_shacl::load_turtle_with_base(
        &std::fs::read_to_string(&file).map_err(|e| e.to_string())?,
        &format!("file://{}", file.display()),
    )
}
pub fn object(view: &GraphView, subject: &Term, ns: &str, local: &str) -> Result<Term, String> {
    view.object(subject, &format!("{ns}{local}"))
        .ok_or_else(|| format!("Missing {local} on {subject}"))
}
#[derive(Default)]
pub struct Score {
    pub counts: BTreeMap<String, [usize; 3]>,
    seen: HashSet<String>,
}
impl Score {
    pub fn walk(
        &mut self,
        file: &Path,
        group: &str,
        run: fn(&Graph, &GraphView, &Term) -> Result<Outcome, String>,
    ) {
        let file = file.canonicalize().expect("manifest exists");
        let graph = load(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let view = GraphView::new(&graph);
        let manifests = view.subjects(
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            &iri(&format!("{MF}Manifest")),
        );
        assert!(!manifests.is_empty(), "No manifest in {}", file.display());
        for manifest in manifests {
            for include in view.objects(&manifest, &format!("{MF}include")) {
                let next = path(&include).expect("local include");
                let subgroup = if next.file_name().is_some_and(|n| n == "manifest.ttl")
                    && next.parent() != file.parent()
                {
                    format!(
                        "{group}/{}",
                        next.parent()
                            .unwrap()
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                    )
                } else {
                    group.into()
                };
                self.walk(&next, &subgroup, run);
            }
            for entries in view.objects(&manifest, &format!("{MF}entries")) {
                for entry in view
                    .list_strict(&entries)
                    .expect("well-formed entries list")
                {
                    let Term::NamedNode(n) = &entry else {
                        panic!("Entry must have an IRI")
                    };
                    assert!(self.seen.insert(n.as_str().into()), "Duplicate entry {n}");
                    // Only withdrawn entries are inapplicable to this report.
                    if view
                        .objects(&entry, &format!("{MF}status"))
                        .contains(&iri(&format!("{SHT}withdrawn")))
                    {
                        continue;
                    }
                    let outcome = run(&graph, &view, &entry).unwrap_or_else(Outcome::Fail);
                    crate::support::earl(n.as_str(), &outcome);
                    let counts = self.counts.entry(group.into()).or_default();
                    match outcome {
                        Outcome::Pass => counts[0] += 1,
                        Outcome::Fail(why) => {
                            counts[1] += 1;
                            println!("FAIL {n}: {why}");
                        }
                        Outcome::Skip(why) => {
                            counts[2] += 1;
                            println!("SKIP {n}: {why}");
                        }
                    }
                }
            }
        }
    }
    pub fn finish(
        &self,
        expected_total: usize,
        pass_floor: usize,
        fail_ceiling: usize,
        skip_ceiling: usize,
    ) {
        let mut total = [0; 3];
        for (group, counts) in &self.counts {
            println!(
                "{group}: pass={} fail={} skip={}",
                counts[0], counts[1], counts[2]
            );
            for i in 0..3 {
                total[i] += counts[i];
            }
        }
        println!(
            "TOTAL: pass={} fail={} skip={}",
            total[0], total[1], total[2]
        );
        assert_eq!(
            total.iter().sum::<usize>(),
            expected_total,
            "Manifest census changed"
        );
        println!("CENSUS_COMPLETE {expected_total}");
        assert!(
            total[0] >= pass_floor,
            "Pass count below floor {pass_floor}"
        );
        assert!(
            total[1] <= fail_ceiling,
            "Failure count above ceiling {fail_ceiling}"
        );
        assert!(
            total[2] <= skip_ceiling,
            "Skip count above ceiling {skip_ceiling}"
        );
    }
}
pub fn triples(graph: &Graph) -> Vec<Triple> {
    GraphView::new(graph)
        .triples(None, None, None)
        .into_iter()
        .map(|[s, p, o]| {
            let Term::NamedNode(p) = p else {
                panic!("predicate")
            };
            Triple::new(oxrdf::NamedOrBlankNode::try_from(s).expect("subject"), p, o)
        })
        .collect()
}
pub fn isomorphic(actual: Vec<Triple>, expected: Vec<Triple>) -> bool {
    let canon = |triples: Vec<Triple>| {
        let mut graph: oxrdf::Graph = triples.into_iter().collect();
        graph.canonicalize(CanonicalizationAlgorithm::Unstable);
        graph
    };
    canon(actual) == canon(expected)
}
pub fn expected_graph(view: &GraphView, result: &Term) -> Result<Vec<Triple>, String> {
    if let Term::NamedNode(n) = result {
        if n.as_str() != "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil" {
            return Ok(triples(&load(&path(result)?)?));
        }
    }
    view.list_strict(result)
        .ok_or("Expected triple list")?
        .into_iter()
        .map(|row| {
            let terms = view.list_strict(&row).ok_or("Malformed expected triple")?;
            let [s, p, o]: [Term; 3] = terms
                .try_into()
                .map_err(|_| "Expected three triple terms")?;
            let Term::NamedNode(p) = p else {
                return Err("Expected IRI predicate".into());
            };
            Ok(Triple::new(
                oxrdf::NamedOrBlankNode::try_from(s).map_err(|_| "Invalid expected subject")?,
                p,
                o,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_oracle_preserves_blank_identity_and_triple_terms() {
        let parse = |s| triples(&Graph::load_str(s, "turtle").unwrap());
        let a = parse("_:a <urn:p> _:b . _:b <urn:q> <<( _:a <urn:p> _:b )>> .");
        let b = parse("_:x <urn:p> _:y . _:y <urn:q> <<( _:x <urn:p> _:y )>> .");
        assert!(isomorphic(a.clone(), b));
        assert!(!isomorphic(
            a,
            parse("_:x <urn:p> _:y . _:z <urn:q> <<( _:x <urn:p> _:y )>> .")
        ));
        assert!(!isomorphic(
            parse("<urn:a> <urn:p> 1 ."),
            parse("<urn:a> <urn:p> 2 .")
        ));
    }
}

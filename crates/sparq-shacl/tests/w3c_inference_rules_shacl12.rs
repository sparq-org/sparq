//! Pinned SHACL 1.2 inference rules: inferred graph, validation report or failure.
#[path = "support/rules_suite.rs"]
mod suite;
mod support;
use oxrdf::Term;
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use suite::*;
use support::Outcome;

#[test]
fn w3c_inference_rules_shacl12() {
    let root = root("inference-rules");
    if !root.exists() {
        eprintln!("SKIP: suite not fetched: {}", root.display());
        return;
    }
    let mut score = Score::default();
    score.walk(&root.join("manifest.ttl"), "inference-rules", run);
    if cfg!(feature = "shacl-af") {
        score.finish(47, 45, 2, 0);
    } else {
        score.finish(47, 0, 0, 47);
    }
}

fn run(graph: &Graph, view: &GraphView, entry: &Term) -> Result<Outcome, String> {
    let types = view.objects(entry, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    if !types.contains(&iri(&format!("{SHT}Infer")))
        && !types.contains(&iri(&format!("{SHT}Validate")))
    {
        return Err("Unsupported test type".into());
    }
    let action = object(view, entry, MF, "action")?;
    let data = load(&path(&object(view, &action, SHT, "dataGraph")?)?)?;
    let shapes = load(&path(&object(view, &action, SHT, "shapesGraph")?)?)?;
    let expected = object(view, entry, MF, "result")?;
    #[cfg(not(feature = "shacl-af"))]
    {
        let _ = (graph, data, shapes, expected);
        Ok(Outcome::Skip("shacl-af disabled".into()))
    }
    #[cfg(feature = "shacl-af")]
    {
        let _ = graph;
        if types.contains(&iri(&format!("{SHT}Validate"))) {
            let result = sparq_shacl::validate_strict(&data, &shapes);
            if expected == iri(&format!("{SHT}Failure")) {
                return Ok(if result.is_err() {
                    Outcome::Pass
                } else {
                    Outcome::Fail("Expected validation failure".into())
                });
            }
            let report = result.map_err(|e| e.to_string())?;
            // This folder's reports contain no results. Check that, rather than
            // accepting a conforms flag while ignoring unexpected result nodes.
            let expected_results = view.objects(&expected, &format!("{SH}result"));
            if !expected_results.is_empty() {
                return Err("Report result comparison not implemented".into());
            }
            let conforms = object(view, &expected, SH, "conforms")?;
            return Ok(
                if conforms == oxrdf::Literal::from(report.conforms).into()
                    && report.results.is_empty()
                {
                    Outcome::Pass
                } else {
                    Outcome::Fail(format!(
                        "Unexpected validation report: {}",
                        report.to_text()
                    ))
                },
            );
        }
        let set = view.object(&action, &format!("{SHT}ruleSet"));
        let result = sparq_shacl::rules12::apply_rules(&data, &shapes, set.as_ref());
        if expected == iri(&format!("{SHT}Failure")) {
            return Ok(if result.is_err() {
                Outcome::Pass
            } else {
                Outcome::Fail("Expected rule failure".into())
            });
        }
        let inferred = result?;
        let expected = expected_graph(view, &expected)?;
        Ok(if isomorphic(inferred.triples.clone(), expected.clone()) {
            Outcome::Pass
        } else {
            Outcome::Fail(format!(
                "Inferred graphs differ: expected {expected:?}; got {:?}",
                inferred.triples
            ))
        })
    }
}

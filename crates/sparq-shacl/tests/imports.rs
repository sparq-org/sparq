//! (sq-uz0) Acceptance tests for the opt-in `imports` feature:
//! shapes-graph ASSEMBLY per W3C SHACL §§3.1/3.3 (shapes-graph) — `sh:shapesGraph` discovery in the
//! data graph plus the transitive, cycle-guarded `owl:imports` closure of every
//! referenced shapes document. Dereferencing an IRI to a document is the
//! caller's loader callback (this engine is offline by design); the traversal,
//! cycle guard, blank-node standardising-apart and union are the library's.
#![cfg(feature = "imports")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::view::GraphView;
use sparq_shacl::{resolve_imports, resolve_shapes_graph, validate, Path};
use std::collections::HashMap;

const EX: &str = "http://example.org/";

/// A `sh:NodeShape` requiring `ex:name` on `ex:Person` (declared via a labelled
/// blank property shape so cross-document label collisions are exercised).
const NAME_SHAPES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    ex:NameShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property _:b0 .
    _:b0 sh:path ex:name ; sh:minCount 1 .
"#;

/// Same construction for `ex:age` — deliberately reusing the `_:b0` label.
const AGE_SHAPES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    ex:AgeShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property _:b0 .
    _:b0 sh:path ex:age ; sh:minCount 1 .
"#;

fn ttl(text: &str) -> Graph {
    Graph::load_str(text, "turtle").expect("test turtle parses")
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).expect("valid IRI"))
}

/// An in-memory IRI → Turtle loader recording every call, `Ok(None)` on a miss.
fn map_loader<'a>(
    docs: &'a HashMap<&'a str, &'a str>,
    calls: &'a mut Vec<String>,
) -> impl FnMut(&str) -> Result<Option<Graph>, String> + 'a {
    move |iri: &str| {
        calls.push(iri.to_string());
        Ok(docs.get(iri).map(|text| ttl(text)))
    }
}

/// The set of `sh:resultPath` predicate IRIs in a validation report.
fn result_paths(data: &Graph, shapes: &Graph) -> Vec<String> {
    let report = validate(data, shapes);
    let mut paths: Vec<String> = report
        .results
        .iter()
        .filter_map(|r| match &r.path {
            Some(Path::Predicate(p)) => Some(p.clone()),
            _ => None,
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[test]
fn resolves_sh_shapes_graph_reference() {
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 .
        ex:alice a ex:Person .
    "#);
    let docs = HashMap::from([("http://example.org/g1", NAME_SHAPES)]);
    let mut calls = Vec::new();
    let res = resolve_shapes_graph(&data, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g1", EX)]);
    assert!(res.unresolved.is_empty());
    // The assembled shapes graph contains the referenced document…
    let paths = result_paths(&data, &res.shapes);
    assert_eq!(paths, vec![format!("{}name", EX)]);
    // …but NOT the data graph's own triples (the shapes-graph union covers only the referenced graphs).
    let v = GraphView::new(&res.shapes);
    assert!(
        !v.contains(
            &iri(&format!("{}alice", EX)),
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            &iri(&format!("{}Person", EX)),
        ),
        "data-graph triples must not leak into the assembled shapes graph"
    );
}

#[test]
fn follows_owl_imports_transitively_and_guards_cycles() {
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 .
        ex:bob a ex:Person .
    "#);
    // g1 imports g2; g2 imports g1 back (a cycle) — each must load exactly once.
    let g1 = format!(
        "@prefix owl: <http://www.w3.org/2002/07/owl#> . @prefix ex: <http://example.org/> .\n\
         ex:g1 owl:imports ex:g2 .\n{}",
        NAME_SHAPES
    );
    let g2 = format!(
        "@prefix owl: <http://www.w3.org/2002/07/owl#> . @prefix ex: <http://example.org/> .\n\
         ex:g2 owl:imports ex:g1 .\n{}",
        AGE_SHAPES
    );
    let docs = HashMap::from([
        ("http://example.org/g1", g1.as_str()),
        ("http://example.org/g2", g2.as_str()),
    ]);
    let mut calls = Vec::new();
    let res = resolve_shapes_graph(&data, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g1", EX), format!("{}g2", EX)]);
    assert_eq!(calls.len(), 2, "cycle guard: each IRI dereferenced once");
    // Both documents' constraints apply: bob lacks ex:name AND ex:age.
    let paths = result_paths(&data, &res.shapes);
    assert_eq!(paths, vec![format!("{}age", EX), format!("{}name", EX)]);
}

#[test]
fn unresolvable_iris_are_recorded_not_fatal() {
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 .
        ex:bob a ex:Person .
    "#);
    let g1 = format!(
        "@prefix owl: <http://www.w3.org/2002/07/owl#> . @prefix ex: <http://example.org/> .\n\
         ex:g1 owl:imports ex:missing .\n{}",
        NAME_SHAPES
    );
    let docs = HashMap::from([("http://example.org/g1", g1.as_str())]);
    let mut calls = Vec::new();
    let res = resolve_shapes_graph(&data, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g1", EX)]);
    assert_eq!(res.unresolved, vec![format!("{}missing", EX)]);
    // The graphs that DID resolve still validate.
    assert_eq!(
        result_paths(&data, &res.shapes),
        vec![format!("{}name", EX)]
    );
}

#[test]
fn loader_errors_propagate() {
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 .
    "#);
    let err = resolve_shapes_graph(&data, |_: &str| Err::<Option<Graph>, _>("boom".into()))
        .expect_err("loader error propagates");
    assert!(
        err.contains("http://example.org/g1"),
        "names the IRI: {}",
        err
    );
    assert!(err.contains("boom"), "carries the loader error: {}", err);
}

#[test]
fn blank_nodes_are_standardised_apart_across_documents() {
    // Two separately-parsed documents both use the label `_:b0` for their
    // property shape. A naive union would merge them into one property shape
    // with two sh:path values; standardising apart keeps them distinct.
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 , ex:g2 .
        ex:bob a ex:Person .
    "#);
    let docs = HashMap::from([
        ("http://example.org/g1", NAME_SHAPES),
        ("http://example.org/g2", AGE_SHAPES),
    ]);
    let mut calls = Vec::new();
    let res = resolve_shapes_graph(&data, map_loader(&docs, &mut calls)).expect("resolves");

    let paths = result_paths(&data, &res.shapes);
    assert_eq!(
        paths,
        vec![format!("{}age", EX), format!("{}name", EX)],
        "each document's property shape must stay a distinct blank node"
    );
}

#[test]
fn duplicate_references_dereference_once() {
    let data = ttl(r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:doc sh:shapesGraph ex:g1 .
        ex:other sh:shapesGraph ex:g1 .
    "#);
    let docs = HashMap::from([("http://example.org/g1", NAME_SHAPES)]);
    let mut calls = Vec::new();
    let res = resolve_shapes_graph(&data, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g1", EX)]);
    assert_eq!(
        calls,
        vec![format!("{}g1", EX)],
        "deduplicated before loading"
    );
}

#[test]
fn seed_blank_labels_cannot_collide_with_renamed_import_labels() {
    // The seed's property shape is labelled `_:i0_b0` — exactly the label a
    // naive ordinal-prefix rename gives the first imported document's `_:b0`.
    // If the two collapsed into one node, the merged shape would carry two
    // sh:path values (ill-formed: only one survives parsing), so one of the
    // two violations would silently vanish from the report.
    let seed = ttl(r#"
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:seed owl:imports ex:g2 .
        ex:NameShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property _:i0_b0 .
        _:i0_b0 sh:path ex:name ; sh:minCount 1 .
    "#);
    let data = ttl(r#"
        @prefix ex: <http://example.org/> .
        ex:bob a ex:Person .
    "#);
    let docs = HashMap::from([("http://example.org/g2", AGE_SHAPES)]);
    let mut calls = Vec::new();
    let res = resolve_imports(&seed, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g2", EX)]);
    let paths = result_paths(&data, &res.shapes);
    assert_eq!(
        paths,
        vec![format!("{}age", EX), format!("{}name", EX)],
        "the seed's _:i0_b0 and the import's renamed _:b0 must stay distinct shapes"
    );
}

#[test]
fn resolve_imports_unions_seed_with_closure() {
    // The caller already HAS a shapes graph; only its owl:imports closure is
    // assembled on top (the seed's own triples are included, labels intact).
    let seed = ttl(&format!(
        "@prefix owl: <http://www.w3.org/2002/07/owl#> . @prefix ex: <http://example.org/> .\n\
         ex:seed owl:imports ex:g2 .\n{}",
        NAME_SHAPES
    ));
    let data = ttl(r#"
        @prefix ex: <http://example.org/> .
        ex:bob a ex:Person .
    "#);
    let docs = HashMap::from([("http://example.org/g2", AGE_SHAPES)]);
    let mut calls = Vec::new();
    let res = resolve_imports(&seed, map_loader(&docs, &mut calls)).expect("resolves");

    assert_eq!(res.resolved, vec![format!("{}g2", EX)]);
    assert!(res.unresolved.is_empty());
    let paths = result_paths(&data, &res.shapes);
    assert_eq!(paths, vec![format!("{}age", EX), format!("{}name", EX)]);
}

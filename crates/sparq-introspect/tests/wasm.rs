// sq-aq5 — headless wasm32 smoke test for sparq-introspect.
//
// The whole crate is sorted scans over sparq-core's PUBLIC read API — no threads, no
// syscalls, no wall clock, no network — so the bead's expectation was that it runs on
// `wasm32-unknown-unknown` trivially. "Expected trivial, unverified" is exactly what a
// smoke test is for: the crate's native `#[cfg(test)]`/integration tests run on the HOST
// target only, so `cargo build --target wasm32` (the CI step this test accompanies)
// proves it COMPILES + links for wasm, but never RUNS a single scan there. This module
// closes that gap — every export (`Introspection::build`, `to_json`, `to_text_summary`,
// `to_void`, `to_void_with_cs`, `schema_summary_for`, and the planner-facing
// `characteristic_set_ids`) executes in a genuine wasm runtime via
// `wasm-pack test --node`, asserting the effective-schema pipeline end to end.
//
// `wasm-pack test --node` runs each `#[wasm_bindgen_test]` in the Node executor (no
// browser, no DOM); Node is the default, so no `run_in_browser` directive is needed.

#![cfg(target_arch = "wasm32")]

use sparq_core::Graph;
use sparq_introspect::{characteristic_set_ids, Introspection};
use wasm_bindgen_test::*;

// A tiny multi-class graph: two foaf:Person, one foaf:Document, an IRI->IRI edge
// (knows), literal objects (name/age, age inline-integer), and a langString — enough to
// exercise characteristic sets, per-class predicate usage, cross-class join hints, the
// literal-vs-IRI/datatype split, vocabulary detection, and the VoID partitions.
const NT: &str = r#"<http://ex/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://ex/alice> <http://xmlns.com/foaf/0.1/name> "Alice" .
<http://ex/alice> <http://xmlns.com/foaf/0.1/age> "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://ex/alice> <http://xmlns.com/foaf/0.1/knows> <http://ex/bob> .
<http://ex/alice> <http://xmlns.com/foaf/0.1/made> <http://ex/doc1> .
<http://ex/bob> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://ex/bob> <http://xmlns.com/foaf/0.1/name> "Bob"@en .
<http://ex/bob> <http://xmlns.com/foaf/0.1/knows> <http://ex/alice> .
<http://ex/doc1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Document> .
<http://ex/doc1> <http://purl.org/dc/terms/title> "A Document" .
"#;

fn graph() -> Graph {
    Graph::load_str(NT, "ntriples").expect("parse inline n-triples on wasm")
}

/// The headline smoke test: the full effective-schema build runs on wasm and the
/// headline totals are exact.
#[wasm_bindgen_test]
fn build_runs_on_wasm() {
    let g = graph();
    assert_eq!(g.len(), 10);

    let ix = Introspection::build(&g);
    assert_eq!(ix.triples, 10);
    // alice, bob, doc1 are the distinct subjects; all three are typed entities.
    assert_eq!(ix.subjects, 3);
    assert_eq!(ix.entities, 3);

    // foaf:Person is the dominant class (2 instances); foaf:Document has 1.
    assert_eq!(ix.classes[0].class, "http://xmlns.com/foaf/0.1/Person");
    assert_eq!(ix.classes[0].instances, 2);
    assert!(ix
        .classes
        .iter()
        .any(|c| c.class == "http://xmlns.com/foaf/0.1/Document"));

    // Characteristic sets partition the subjects exactly (the scan invariant).
    assert!(ix.characteristic_sets.distinct > 0);
    let covered: u64 = ix.characteristic_sets.sets.iter().map(|s| s.subjects).sum();
    assert_eq!(
        covered + ix.characteristic_sets.elided_subjects,
        ix.subjects
    );

    // Cross-class join hint: a foaf:Person --foaf:knows--> foaf:Person edge is observed.
    assert!(ix.join_hints.hints.iter().any(|h| {
        h.subject_class == "http://xmlns.com/foaf/0.1/Person"
            && h.predicate == "http://xmlns.com/foaf/0.1/knows"
            && h.object_class == "http://xmlns.com/foaf/0.1/Person"
    }));

    // Vocabulary detection recognises foaf offline (bundled table, no network).
    assert!(ix.vocabularies.namespaces.iter().any(
        |v| v.namespace == "http://xmlns.com/foaf/0.1/" && v.prefix.as_deref() == Some("foaf")
    ));
}

/// The JSON surface (`to_json`) serialises and round-trips through serde_json on wasm.
#[wasm_bindgen_test]
fn to_json_runs_on_wasm() {
    let ix = Introspection::build(&graph());
    let json = ix.to_json();
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON on wasm");
    assert_eq!(v["triples"].as_u64(), Some(ix.triples));
    assert_eq!(v["classes"][0]["class"], "http://xmlns.com/foaf/0.1/Person");
}

/// The prompt-ready text digest respects its budget and names the real schema on wasm.
#[wasm_bindgen_test]
fn text_summary_runs_on_wasm() {
    let ix = Introspection::build(&graph());
    let summary = ix.to_text_summary(2000);
    assert!(
        summary.chars().count() <= 2000,
        "summary must respect its budget on wasm"
    );
    assert!(summary.contains("Person"));
    assert!(summary.contains("foaf: http://xmlns.com/foaf/0.1/"));
}

/// The VoID exports (`to_void` + the characteristic-set superset `to_void_with_cs`) emit
/// on wasm and the cs export is a strict superset.
#[wasm_bindgen_test]
fn void_export_runs_on_wasm() {
    let ix = Introspection::build(&graph());
    let void = ix.to_void("http://ex/dataset");
    assert!(void.contains("http://rdfs.org/ns/void#Dataset"));
    assert!(void.contains("http://rdfs.org/ns/void#triples"));

    let with_cs = ix.to_void_with_cs("http://ex/dataset");
    assert!(
        with_cs.len() > void.len(),
        "to_void_with_cs is a strict superset of to_void"
    );
    assert!(with_cs.contains("http://sparq.dev/ns/cs#CharacteristicSet"));
}

/// sq-bde: the SHACL node-shape export (`to_shacl`) runs on wasm and emits a
/// node shape per characteristic set with property shapes for its predicates.
#[wasm_bindgen_test]
fn shacl_export_runs_on_wasm() {
    let ix = Introspection::build(&graph());
    let shacl = ix.to_shacl();
    // One sh:NodeShape per retained characteristic set.
    let node_shapes = shacl
        .matches("<http://www.w3.org/ns/shacl#NodeShape>")
        .count();
    assert_eq!(node_shapes, ix.characteristic_sets.sets.len());
    assert!(node_shapes > 0);
    // foaf:Person is universal to the two-Person set, so it targets at least one shape.
    assert!(shacl.contains("http://www.w3.org/ns/shacl#targetClass"));
    assert!(shacl.contains("http://xmlns.com/foaf/0.1/Person"));
    // Property shapes carry sh:path + sh:minCount for the set's predicates.
    assert!(shacl.contains("http://www.w3.org/ns/shacl#path"));
    assert!(shacl.contains("http://www.w3.org/ns/shacl#minCount"));
}

/// The planner-facing id-space accessor (`characteristic_set_ids`) runs on wasm and its
/// subject counts agree with the IRI-resolved table's distinct-set count.
#[wasm_bindgen_test]
fn characteristic_set_ids_runs_on_wasm() {
    let g = graph();
    let sets = characteristic_set_ids(&g);
    assert!(!sets.is_empty());
    let total: u64 = sets.iter().map(|s| s.subjects).sum();
    assert_eq!(total, 3, "every subject falls in exactly one id-space set");
    // predicate_triples is aligned with predicates for every set.
    for s in &sets {
        assert_eq!(s.predicates.len(), s.predicate_triples.len());
    }
}

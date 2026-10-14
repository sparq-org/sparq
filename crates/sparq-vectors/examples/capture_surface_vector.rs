//! sq-dwdm — capture harness for the /surface/vector site page.
//!
//! Prints the EXACT, deterministic output of the real sparq-vectors pipeline so the
//! site page (site/src/lib/vector.ts) can paste it BYTE-FOR-BYTE rather than fabricate
//! it (the sq-rnwc lesson). Everything here is the answer-EXACT backend over a tiny,
//! declared, in-memory fixture — reproducible on any machine, no downloaded data, no
//! model, no index build. Run with:
//!
//!   cargo run -p sparq-vectors --features vec-predicate --example capture_surface_vector
//!
//! Two captures:
//!   1. Usain Bolt label-embedding nearest-neighbour (deterministic HashEmbedder,
//!      `embed_labels` + `nearest_term_exact`) — the bead's requested example.
//!   2. The `vec:nearest` / `vec:search` magic predicates as REAL SPARQL over a
//!      5-entity unit-circle store, printing the engine's verbatim term serialization.
#![cfg(feature = "vec-predicate")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{
    embed_labels, nearest_term_exact, query_vec, HashEmbedder, QueryResult, VectorStore,
};

fn term(iri: &str) -> Term {
    Term::NamedNode(NamedNode::new(iri).unwrap())
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "sparq-vectors-capture-{}-{}.spqv",
        std::process::id(),
        name
    ))
}

/// Render a QueryResult exactly as the engine emits each cell (oxrdf::Term::Display =
/// N-Triples), one row per line, tab-separated, `UNBOUND` for an unbound cell.
fn render(r: &QueryResult) {
    let header: Vec<String> = r.vars.iter().map(|v| format!("?{}", v.as_str())).collect();
    println!("VARS\t{}", header.join("\t"));
    for row in &r.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|c| match c {
                Some(t) => t.to_string(),
                None => "UNBOUND".to_string(),
            })
            .collect();
        println!("ROW\t{}", cells.join("\t"));
    }
    println!("NROWS\t{}", r.rows.len());
}

fn capture_bolt() {
    // The bead's example: rdfs:label embeddings with the DETERMINISTIC HashEmbedder
    // (lexical n-gram hashing, fixed — no model, no randomness), then exact cosine
    // nearest-neighbour by term. "Usain Bolt"'s nearest LABELED neighbour is
    // "Usain Bolt Junior" (shared n-grams); the seed is excluded.
    let ttl = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix ex:   <http://example.org/> .

ex:bolt      rdfs:label "Usain Bolt" ; a ex:Athlete .
ex:bolt2     rdfs:label "Usain Bolt Junior" ; a ex:Athlete .
ex:blake     rdfs:label "Yohan Blake" ; a ex:Athlete .
ex:powell    rdfs:label "Asafa Powell" ; a ex:Athlete .
ex:coubertin skos:prefLabel "Pierre de Coubertin" ; a ex:Founder .
"#;
    let g = Graph::load_str(ttl, "turtle").unwrap();
    let path = tmp("bolt");
    let embedder = HashEmbedder::new(64); // deterministic, test-only lexical embedder
    let mut store = VectorStore::create(&path, 64).unwrap();
    let n = embed_labels(&g, &mut store, &embedder).unwrap();
    store.finalize().unwrap();

    println!("=== BOLT-CAPTURE-BEGIN ===");
    println!("EMBEDDED\t{}", n);
    let bolt = term("http://example.org/bolt");
    let neighbours = nearest_term_exact(&store, &g, &bolt, 4);
    println!("SEED\t{}", bolt);
    for (t, score) in &neighbours {
        // f32 cosine printed with full default precision; the site rounds for display
        // but the captured value is the engine's exact f32.
        println!("NEIGHBOUR\t{}\t{}", t, score);
    }
    println!("=== BOLT-CAPTURE-END ===");
    let _ = std::fs::remove_file(&path);
}

fn unit_circle_store(name: &str) -> (Graph, VectorStore) {
    // Five entities on the unit circle (mirrors tests/vec_predicate.rs::fixture): a-style
    // +x, b-style +y, c near +x, d -x, e near +y. dim=2 so the geometry is legible.
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/label> "alpha" .
        <http://ex/b> <http://ex/label> "beta" .
        <http://ex/c> <http://ex/label> "gamma" .
        <http://ex/d> <http://ex/label> "delta" .
        <http://ex/e> <http://ex/label> "epsilon" .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| g.id_of(&term(s)).unwrap();
    let mut store = VectorStore::create(tmp(name), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // near +x
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // near +y
    (g, store)
}

fn capture_query(title: &str, name: &str, sparql: &str) {
    let (g, store) = unit_circle_store(name);
    println!("=== QUERY-CAPTURE-BEGIN\t{} ===", title);
    println!("SPARQL-BEGIN");
    print!("{}", sparql);
    println!("\nSPARQL-END");
    let r = query_vec(&g, sparql, &store).unwrap();
    render(&r);
    println!("=== QUERY-CAPTURE-END ===");
}

fn main() {
    capture_bolt();

    // (a) vec:nearest by query vector "1,0" → the two most +x-aligned: a then c.
    capture_query(
        "nearest-by-vector",
        "nearest_vec",
        "PREFIX vec: <http://sparq.dev/vec#>\nSELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
    );

    // (b) vec:nearest by SEED IRI <a> (+x); a itself is excluded → c is nearest.
    capture_query(
        "nearest-by-seed",
        "nearest_seed",
        "PREFIX vec: <http://sparq.dev/vec#>\nSELECT ?node WHERE { ?node vec:nearest ( <http://ex/a> 1 ) }",
    );

    // (c) vec:nearest joined to ordinary triples — neighbours' labels for "0,1".
    capture_query(
        "nearest-joined-to-bgp",
        "nearest_join",
        "PREFIX vec: <http://sparq.dev/vec#>\nSELECT ?label WHERE {\n  ?node vec:nearest ( \"0,1\" 2 ) .\n  ?node <http://ex/label> ?label .\n}",
    );

    // (d) vec:search binds the cosine score; ORDER BY DESC recovers best-first.
    capture_query(
        "search-binds-score",
        "search_score",
        "PREFIX vec: <http://sparq.dev/vec#>\nSELECT ?node ?score WHERE {\n  ( ?node ?score ) vec:search ( \"1,0\" 3 )\n} ORDER BY DESC(?score)",
    );
}

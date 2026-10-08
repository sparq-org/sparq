//! [OPUS-4.8] sq-rt6v — **graph serialiser oracle** for the RDF *graph* writers in
//! `sparq_server::graph` (`triples_to_ntriples` / `triples_to_turtle` / `triples_to_rdfxml`).
//!
//! Mirrors the SELECT/ASK `tests/serializer_oracle.rs` but for the graph syntaxes: OUR
//! serialised output is fed back through an INDEPENDENT reference parser (oxigraph's
//! `oxttl` Turtle parser and `oxrdfxml` RDF/XML parser) and the recovered triple SET is
//! required to equal the source graph exactly. A `contains` check proves a byte is present;
//! this proves the document is well-formed AND lossless.
//!
//! The source graphs come from the REAL engine (CONSTRUCT over a random Turtle graph, the
//! same generator style as the engine's exec tests) so the hazard cells — typed/lang
//! literals, blank nodes, escapable lexical forms, IRIs that the writers must compact or
//! escape — flow through the writers as they would in production.

use oxrdf::Triple;
use sparq_core::Graph;
use sparq_engine::construct_or_describe;
use sparq_server::graph::{parse_rdfxml, triples_to_ntriples, triples_to_rdfxml, triples_to_turtle};

/// An order-independent fingerprint of a triple set: each triple rendered to its canonical
/// N-Triples line, the whole list sorted. Two graphs share a fingerprint iff they are the
/// same set of triples. (Blank-node labels are compared verbatim here; the round-trips
/// below preserve labels, so that is exact for these inputs.)
fn triple_set(triples: &[Triple]) -> Vec<String> {
    let mut out: Vec<String> = triples.iter().map(|t| format!("{} {} {} .", t.subject, t.predicate, t.object)).collect();
    out.sort();
    out
}

/// Re-parses our Turtle output with the reference `oxttl` Turtle parser.
fn reparse_turtle(ttl: &str) -> Vec<Triple> {
    oxttl::TurtleParser::new()
        .for_slice(ttl.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("reference Turtle parser REJECTED our output: {e}\n--- output ---\n{ttl}"))
}

/// Asserts every graph writer round-trips `triples` exactly (well-formed + lossless set).
fn assert_graph_roundtrips(label: &str, triples: &[Triple]) {
    let expect = triple_set(triples);

    // N-Triples (sanity — it is the canonical form, re-parsed as Turtle since NT ⊂ Turtle).
    let nt = triples_to_ntriples(triples).unwrap();
    assert_eq!(triple_set(&reparse_turtle(&nt)), expect, "[{label}/N-Triples] set must round-trip\n{nt}");

    // Prefix-compacting Turtle.
    let ttl = triples_to_turtle(triples).unwrap();
    assert_eq!(triple_set(&reparse_turtle(&ttl)), expect, "[{label}/Turtle] set must round-trip\n{ttl}");

    // RDF/XML.
    let xml = triples_to_rdfxml(triples).unwrap();
    let back = parse_rdfxml(xml.as_bytes(), None)
        .unwrap_or_else(|e| panic!("[{label}/RDF-XML] reference parser REJECTED our output: {e}\n{xml}"));
    assert_eq!(triple_set(&back), expect, "[{label}/RDF-XML] set must round-trip\n{xml}");
}

/// Deterministic LCG → a random Turtle graph mixing IRIs (some under registered prefixes,
/// so the writers must compact them) and hazardous literals (escapable / typed / lang).
fn random_graph_ttl(seed0: u64, n_nodes: u32, n_edges: usize) -> String {
    let mut seed = seed0;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let lits = [
        r#""plain""#,
        "\"line1\\nline2\"",
        "\"q\\\"uote\"",
        r#""café"@fr"#,
        // Raw `<`, `>`, `&` in a literal value — the RDF/XML writer must XML-escape these,
        // and the Turtle writer must keep them intact. A robust escaping check.
        r#""a <angle> & ampersand""#,
        r#""99"^^<http://www.w3.org/2001/XMLSchema#integer>"#,
        r#""2020-01-02T03:04:05Z"^^<http://www.w3.org/2001/XMLSchema#dateTime>"#,
    ];
    // foaf:/rdf: predicates exercise prefix compaction in the writers.
    let mut ttl = String::from(
        "@prefix ex: <http://ex/> .\n\
         @prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
         @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n",
    );
    for _ in 0..n_edges {
        let a = next() % n_nodes;
        match next() % 4 {
            0 => {
                let b = next() % n_nodes;
                ttl.push_str(&format!("ex:n{a} foaf:knows ex:n{b} .\n"));
            }
            1 => {
                let l = lits[(next() as usize) % lits.len()];
                ttl.push_str(&format!("ex:n{a} foaf:name {l} .\n"));
            }
            2 => {
                ttl.push_str(&format!("ex:n{a} rdf:type foaf:Person .\n"));
            }
            _ => {
                ttl.push_str(&format!("ex:n{a} foaf:knows [ foaf:name \"t{}\" ] .\n", next() % 5));
            }
        }
    }
    ttl
}

#[test]
fn graph_writers_roundtrip_engine_constructs() {
    // CONSTRUCT the whole graph back out (`?s ?p ?o`), so the engine produces a real triple
    // list (with the random graph's hazards) that we push through every writer.
    for seed in [1u64, 7, 42, 1234, 98765] {
        let ttl = random_graph_ttl(seed, 20, 140);
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let triples = construct_or_describe(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").unwrap();
        assert_graph_roundtrips(&format!("seed{seed}"), &triples);
    }
}

#[test]
fn graph_writers_roundtrip_empty_graph() {
    // Zero triples: every writer must emit a valid (empty-graph) document.
    assert_graph_roundtrips("empty", &[]);
}

/// RDF/XML cannot use `rdf:about` as a predicate. The buffered writer must refuse the graph
/// with an `Err` rather than return an unterminated document that silently lacks the bad
/// triple and every triple after it. The other writers encode the same graph fine.
#[test]
fn rdfxml_refuses_a_forbidden_predicate_instead_of_truncating() {
    let ttl = "<http://ex/a> <http://ex/p> \"before\" .\n\
               <http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#about> \"bad\" .\n\
               <http://ex/b> <http://ex/p> \"after\" .\n";
    let g = Graph::load_str(ttl, "turtle").unwrap();
    let triples = construct_or_describe(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(triples.len(), 3);

    let err = triples_to_rdfxml(&triples).expect_err("rdf:about as a predicate must be refused");
    assert!(err.contains("serialising the graph"), "unexpected message: {err}");

    assert_eq!(reparse_turtle(&triples_to_ntriples(&triples).unwrap()).len(), 3);
    assert_eq!(reparse_turtle(&triples_to_turtle(&triples).unwrap()).len(), 3);
}

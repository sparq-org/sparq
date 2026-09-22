//! [OPUS-4.8] sq-rt6v — RDF *graph* serialisation (and RDF/XML parsing) for the graph-valued
//! surfaces: CONSTRUCT / DESCRIBE results and the Graph Store HTTP Protocol read/write side.
//!
//! Where [`crate::results`] serialises *SPARQL solution sets* (the SELECT/ASK result
//! formats), this module serialises an *RDF graph* — a list of [`oxrdf::Triple`] — to the
//! three RDF concrete syntaxes the server offers for a graph result:
//!
//! * **N-Triples** (`application/n-triples`) — the canonical line form; a syntactic subset
//!   of Turtle, so it also satisfies a `text/turtle` request when no compaction is wanted.
//! * **Turtle** (`text/turtle`), **prefix-compacting** — uses a registered prefix set so IRIs
//!   under a well-known namespace render as `prefix:local` instead of a full `<…>`. This
//!   closes the readability half of the conformance gap (the previous "Turtle" output was
//!   just N-Triples). Built on `oxttl`'s `TurtleSerializer` (the oxigraph Turtle writer), so
//!   the output is guaranteed well-formed Turtle.
//! * **RDF/XML** (`application/rdf+xml`) — the last result-format gap; built on `oxrdfxml`'s
//!   `RdfXmlSerializer`. The same crate's `RdfXmlParser` powers the *request-body* reader so a
//!   GSP `PUT`/`POST` may carry an `application/rdf+xml` body.
//!
//! These are deliberately pure (no async, no HTTP types), like [`crate::results`], so they
//! are unit-testable and the `server` feature is not required to use them.
//!
//! ## Buffered vs streaming ([FABLE-5] sq-0kq6k)
//!
//! Each syntax has TWO entry points and exactly ONE implementation:
//!
//! * `write_*_to(&[Triple], &mut impl io::Write)` — the primitive. It hands the underlying
//!   `oxttl` / `oxrdfxml` serialiser (or the N-Triples line loop) the caller's sink directly,
//!   so the rendered document is never materialised as one `String`. This is what the HTTP
//!   CONSTRUCT / DESCRIBE path feeds a chunk sink to emit a chunked-transfer response body.
//! * `triples_to_*(&[Triple]) -> String` — the buffered convenience, implemented BY calling
//!   the `write_*_to` primitive over an in-memory `Vec<u8>`. Byte-identity between the two is
//!   therefore structural (one writer, one code path), not a property a test has to police.

use oxrdf::Triple;

/// RDF/XML media type (graph syntax) — request body **and** response.
pub const RDF_XML_MEDIA: &str = "application/rdf+xml";
/// Turtle media type.
pub const TURTLE_MEDIA: &str = "text/turtle";
/// N-Triples media type.
pub const NTRIPLES_MEDIA: &str = "application/n-triples";

/// The prefix set the Turtle / RDF/XML writers register so common namespaces compact to
/// `prefix:local` (Turtle) / a namespaced QName (RDF/XML) for readability. These are the
/// ubiquitous vocabularies a SPARQL endpoint emits constantly; a non-matching IRI simply
/// renders in full, so the set is purely additive (never wrong, only more or less compact).
///
/// `rdf` and `xsd` are load-bearing for RDF/XML too: `rdf:` is the syntax's own namespace
/// and `xsd:` typed-literal datatypes are the most common, so registering them keeps the
/// output tidy. Kept small and stable on purpose — these are not user-configurable (the
/// server has no per-request prefix input), and an over-long list would only bloat every
/// document's header.
pub const COMMON_PREFIXES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("schema", "https://schema.org/"),
];

/// [FABLE-5] sq-0kq6k: Streams an RDF graph as canonical **N-Triples** into `w` — one
/// `s p o .` line per triple in `oxrdf` Display term syntax, written straight to the sink
/// with no whole-document `String`. [`triples_to_ntriples`] is this function over a `Vec<u8>`.
///
/// The only error a caller can observe is `w`'s: over an in-memory buffer that is impossible,
/// over the HTTP chunk sink it means the client went away (and serialisation should stop).
pub fn write_ntriples_to<W: std::io::Write>(triples: &[Triple], w: &mut W) -> std::io::Result<()> {
    for t in triples {
        writeln!(w, "{} {} {} .", t.subject, t.predicate, t.object)?;
    }
    Ok(())
}

/// Serialises an RDF graph as canonical **N-Triples**. Identical contract to the engine's
/// `triples_to_ntriples` (one `s p o .` line per triple, `oxrdf` Display term syntax); kept
/// here so this module is the single graph-serialisation seam the format dispatch routes
/// through.
pub fn triples_to_ntriples(triples: &[Triple]) -> String {
    buffer(triples.len() * 64, |w| write_ntriples_to(triples, w))
}

/// The Turtle serialiser with [`COMMON_PREFIXES`] registered, shared by the streaming and
/// buffered entry points so both compact the same namespaces.
///
/// `with_prefix` only fails on a syntactically invalid prefix IRI; ours are constants and
/// valid, so the error is unreachable — fall back to the un-prefixed serialiser rather than
/// panicking if a future edit breaks one.
fn turtle_serializer() -> oxttl::TurtleSerializer {
    let mut ser = oxttl::TurtleSerializer::new();
    for (name, iri) in COMMON_PREFIXES {
        ser = ser.with_prefix(*name, *iri).unwrap_or_else(|_| oxttl::TurtleSerializer::new());
    }
    ser
}

/// [FABLE-5] sq-0kq6k: Streams an RDF graph as **prefix-compacting Turtle** into `w`.
/// `oxttl`'s `TurtleSerializer` is already an incremental writer — it holds only the current
/// subject/predicate grouping state — so handing it the caller's sink emits the `@prefix`
/// header and early subject blocks before the last triple is rendered, with no whole-document
/// `String`. [`triples_to_turtle`] is this function over a `Vec<u8>`.
pub fn write_turtle_to<W: std::io::Write>(triples: &[Triple], w: &mut W) -> std::io::Result<()> {
    let mut ser = turtle_serializer().for_writer(w);
    for t in triples {
        ser.serialize_triple(t.as_ref())?;
    }
    ser.finish()?;
    Ok(())
}

/// Serialises an RDF graph as **prefix-compacting Turtle**: registers [`COMMON_PREFIXES`] so
/// IRIs under those namespaces render as `prefix:local`, the rest in full. Output is
/// guaranteed well-formed Turtle (it goes through `oxttl`'s `TurtleSerializer`).
pub fn triples_to_turtle(triples: &[Triple]) -> String {
    buffer(triples.len() * 64, |w| write_turtle_to(triples, w))
}

/// [FABLE-5] sq-0kq6k: Streams an RDF graph as **RDF/XML** into `w`. Like the Turtle writer,
/// `oxrdfxml`'s `RdfXmlSerializer` writes incrementally, so no whole-document `String` is
/// built. [`triples_to_rdfxml`] is this function over a `Vec<u8>`.
pub fn write_rdfxml_to<W: std::io::Write>(triples: &[Triple], w: &mut W) -> std::io::Result<()> {
    let mut ser = oxrdfxml::RdfXmlSerializer::new();
    for (name, iri) in COMMON_PREFIXES {
        ser = ser.with_prefix(*name, *iri).unwrap_or_else(|_| oxrdfxml::RdfXmlSerializer::new());
    }
    let mut ser = ser.for_writer(w);
    for t in triples {
        ser.serialize_triple(t.as_ref())?;
    }
    ser.finish()?;
    Ok(())
}

/// Serialises an RDF graph as **RDF/XML** (`application/rdf+xml`) with [`COMMON_PREFIXES`]
/// registered as XML namespaces for readable QNames. Output goes through `oxrdfxml`'s
/// `RdfXmlSerializer`, so it is guaranteed well-formed RDF/XML.
pub fn triples_to_rdfxml(triples: &[Triple]) -> String {
    buffer(triples.len() * 64, |w| write_rdfxml_to(triples, w))
}

/// [OPUS-4.8] sq-oy1f.1: Serialises an RDF graph (triple list) as **JSON-LD 1.1**
/// (`application/ld+json`) in the engine's *flattened* document form: a
/// `{"@graph": [ … node objects … ]}` envelope with subjects merged into one node object
/// each and a stable node ordering. We pick the flattened form (over expanded or compacted)
/// as the negotiated default because it is the safest interop target — node-merged, no
/// `@context` to negotiate against, and round-trips losslessly back to the same RDF graph
/// (the JSON-LD `toRdf` of a `fromRdf`-flattened document reconstructs the source triples,
/// asserted by the server-level round-trip test).
///
/// The result graph for a CONSTRUCT / DESCRIBE / GSP read is a flat list of default-graph
/// triples (no named graphs — GSP scopes one graph per request, named by the URL), so we
/// hand the engine writer a single default-graph view `[(None, triples)]`. The prefixes
/// argument is only consulted by the compacted form, which we do not use here, so the
/// engine's `default_prefixes` are inert.
///
/// OPT-IN: only compiled when the server's `jsonld` feature is on (it links the engine's
/// `serialize-rdf` JSON-LD writer); without the feature this function does not exist and the
/// `GraphFormat::JsonLd` variant it serves is never constructed.
#[cfg(feature = "jsonld")]
pub fn triples_to_jsonld(triples: &[Triple]) -> String {
    use sparq_engine::serialize::{default_prefixes, write_jsonld, JsonLdForm, NamedGraph};
    // One default-graph view over the triple list. `NamedGraph = (Option<&Term>, &[Triple])`;
    // `None` is the default graph.
    let view: [NamedGraph<'_>; 1] = [(None, triples)];
    write_jsonld(&view, JsonLdForm::Flattened, &default_prefixes())
}

/// [FABLE-5] sq-0kq6k: Writes the JSON-LD document into `w`, the sink-shaped twin of
/// [`triples_to_jsonld`] so the graph-format dispatch is uniform.
///
/// HONEST CAVEAT: this one does NOT stream. The engine's `write_jsonld` returns a whole
/// `String` (a JSON-LD document is node-merged, so it cannot be emitted subject-by-subject
/// the way Turtle can), so the rendered document is still materialised in full before it
/// reaches `w`. A `text/turtle` / `application/n-triples` / `application/rdf+xml` CONSTRUCT
/// streams; an `application/ld+json` one does not. Wiring a genuinely incremental JSON-LD
/// writer is follow-up work.
#[cfg(feature = "jsonld")]
pub fn write_jsonld_to<W: std::io::Write>(triples: &[Triple], w: &mut W) -> std::io::Result<()> {
    w.write_all(triples_to_jsonld(triples).as_bytes())
}

/// [FABLE-5] sq-0kq6k: Runs one of the `write_*_to` primitives over an in-memory `Vec<u8>`
/// and returns the rendered document — the single shared implementation of every buffered
/// `triples_to_*` wrapper, so buffered output is byte-identical to streamed output by
/// construction.
///
/// Writing to a `Vec<u8>` is infallible, so the `io::Error` arm is genuinely unreachable; if
/// one ever surfaced (a serialiser-internal invariant) we keep whatever bytes were written
/// rather than panic in a request handler. The oxigraph writers emit valid UTF-8 (Turtle /
/// RDF/XML are UTF-8 here), so the final conversion is lossless.
fn buffer(hint: usize, render: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>) -> String {
    let mut out = Vec::with_capacity(hint);
    let _ = render(&mut out);
    String::from_utf8(out).unwrap_or_default()
}

/// Parses an **RDF/XML** document into a triple list (the GSP write-body reader for
/// `Content-Type: application/rdf+xml`). A syntax error is reported as `Err(message)`; the
/// caller maps that to a `400`. `base` resolves relative IRIs / `rdf:ID` fragments — the GSP
/// graph IRI is a sensible base so a relative reference resolves predictably.
pub fn parse_rdfxml(bytes: &[u8], base: Option<&str>) -> Result<Vec<Triple>, String> {
    let mut parser = oxrdfxml::RdfXmlParser::new();
    if let Some(base) = base {
        parser = parser.with_base_iri(base).map_err(|e| format!("invalid base IRI '{base}': {e}"))?;
    }
    let mut triples = Vec::new();
    for t in parser.for_slice(bytes) {
        triples.push(t.map_err(|e| e.to_string())?);
    }
    Ok(triples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, Term};

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    /// A small mixed graph: a prefixed-namespace triple, a typed literal, a lang literal and
    /// a blank-node object — enough to exercise compaction, datatype and lang handling.
    fn sample() -> Vec<Triple> {
        vec![
            Triple::new(
                iri("http://xmlns.com/foaf/0.1/Person"),
                iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
                iri("http://www.w3.org/2002/07/owl#Class"),
            ),
            Triple::new(
                iri("http://ex/alice"),
                iri("http://xmlns.com/foaf/0.1/age"),
                Term::Literal(Literal::new_typed_literal("30", iri("http://www.w3.org/2001/XMLSchema#integer"))),
            ),
            Triple::new(
                iri("http://ex/alice"),
                iri("http://xmlns.com/foaf/0.1/name"),
                Term::Literal(Literal::new_language_tagged_literal("Alice", "en").unwrap()),
            ),
            Triple::new(iri("http://ex/alice"), iri("http://xmlns.com/foaf/0.1/knows"), BlankNode::new("b0").unwrap()),
        ]
    }

    #[test]
    fn ntriples_is_one_line_per_triple() {
        let nt = triples_to_ntriples(&sample());
        assert_eq!(nt.lines().count(), 4);
        assert!(nt.contains("<http://ex/alice> <http://xmlns.com/foaf/0.1/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> ."));
    }

    #[test]
    fn turtle_compacts_common_prefixes() {
        let ttl = triples_to_turtle(&sample());
        // The header declares the registered prefixes…
        assert!(ttl.contains("@prefix foaf: <http://xmlns.com/foaf/0.1/>"), "missing foaf prefix decl: {ttl}");
        // …and the body uses the compact form rather than the full IRI.
        assert!(ttl.contains("foaf:age") || ttl.contains("foaf:name"), "namespace not compacted: {ttl}");
        // It must still be valid Turtle that re-parses to the same triple count.
        assert_eq!(sparq_core::Graph::load_str(&ttl, "turtle").unwrap().len(), 4, "turtle round-trip lost triples: {ttl}");
    }

    #[test]
    fn rdfxml_is_wellformed_and_roundtrips() {
        let xml = triples_to_rdfxml(&sample());
        assert!(xml.contains("<?xml"), "missing XML declaration: {xml}");
        assert!(xml.contains("rdf:RDF"), "missing rdf:RDF root: {xml}");
        // Re-parse our own RDF/XML output: the graph must come back identical (4 triples).
        let back = parse_rdfxml(xml.as_bytes(), None).unwrap();
        assert_eq!(back.len(), 4, "rdf/xml round-trip changed triple count: {xml}");
    }

    #[test]
    fn parse_rdfxml_rejects_malformed() {
        let err = parse_rdfxml(b"<not> rdf xml", None);
        assert!(err.is_err(), "malformed RDF/XML must be an error");
    }

    #[test]
    fn parse_rdfxml_rejects_an_invalid_base_iri() {
        // [OPUS-4.8] sq-4vao: the `base` (the GSP graph IRI) is fed to `with_base_iri`; a
        // syntactically invalid base must fail FAST with a clear message (the caller maps it
        // to a 400) rather than silently parsing against no base. A bare space is not a valid IRI.
        let err = parse_rdfxml(b"<?xml version=\"1.0\"?><rdf:RDF/>", Some("not a valid iri"))
            .expect_err("an invalid base IRI must be rejected");
        assert!(err.contains("invalid base IRI"), "unexpected message: {err}");
        assert!(err.contains("not a valid iri"), "message should echo the bad base: {err}");
    }

    #[test]
    fn parse_then_serialize_roundtrips_through_turtle() {
        // Parse a hand-written RDF/XML doc, re-serialise as prefix Turtle, and confirm the
        // triples survive (value-equal as a set).
        let doc = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:foaf="http://xmlns.com/foaf/0.1/">
  <rdf:Description rdf:about="http://ex/alice">
    <foaf:name>Alice</foaf:name>
  </rdf:Description>
</rdf:RDF>"#;
        let triples = parse_rdfxml(doc.as_bytes(), None).unwrap();
        assert_eq!(triples.len(), 1);
        let ttl = triples_to_turtle(&triples);
        let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(g.len(), 1);
    }
}

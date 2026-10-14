//! sq-quly (#796): the `Store::parseShaclCompact(text, base?)` binding.
//!
//! Exposes `sparq-shacl`'s existing SHACL Compact Syntax (SCS) parser
//! ([`sparq_shacl::parse_scs_to_graph`] — SCS text → a SHACL shapes [`Graph`]) to
//! JS/WASM consumers, so the site #796 playground can offer an SCS *input* mode:
//! a user types compact-syntax shapes and gets back the equivalent SHACL shapes
//! graph as RDF text (ready to feed [`Store::validate`], or to display / download).
//!
//! It reimplements NOTHING. The parse side is `sparq-shacl`'s `scs` feature (a
//! hand-rolled lexer + recursive-descent parser, no new dependency; 32/32 W3C
//! `shacl12-cs` fixtures round-trip). The serialise side is this crate's EXISTING
//! [`Store::serialize`] engine-writer path ([`crate::serialize`], the
//! `graph_to_turtle*` machinery from #900/#923) — the binding parses SCS into a
//! shapes `Graph`, wraps it in a throwaway [`Store`], and calls straight through to
//! `serialize("turtle", …)`. So the bytes a caller gets are exactly what
//! `Store::serialize` produces over the same graph; there is no second serializer.
//!
//! This module compiles ONLY under the opt-in `scs` feature (which pulls `shacl`
//! for the `sparq-shacl/scs` parser AND `serialize-rdf` for the engine writer it
//! reuses), so the default browser bundle carries zero SCS / serializer code — the
//! lean `wasm_bundle_bytes` baseline is byte-identical to before.
//!
//! ## Surface
//!
//! `parseShaclCompact(text, base?)` → the shapes graph as a **pretty Turtle**
//! string (the default output: a sorted, blank-line-separated `@prefix`-headed
//! document — the same shape `Store.serialize("turtle", true, "  ", true)`
//! emits — chosen so a UI can show a readable shapes graph straight away). Relative
//! IRIs (and the `owl:Ontology` subject) resolve against `base`; when `base` is
//! `undefined`/`null` the SCS no-`BASE` convention ([`sparq_shacl::DEFAULT_BASE`])
//! is used. A document-level `BASE` directive overrides either. An SCS parse error
//! surfaces as the `JsError` Err arm (carrying the 1-based source line), never a
//! silently-empty or mis-parsed document.

use sparq_shacl::{parse_scs_to_graph, DEFAULT_BASE};
use wasm_bindgen::prelude::*;

use crate::Store;

#[wasm_bindgen]
impl Store {
    /// sq-quly (#796): parses a **SHACL Compact Syntax (SCS)** document
    /// into the equivalent SHACL **shapes graph** and returns it as a **pretty
    /// Turtle** string.
    ///
    /// `text` is an SCS document (the W3C compact syntax — `shape`/`shapeClass`,
    /// path expressions, `[min..max]`, `nodeKind`, `@`shape-refs, `param=value`,
    /// `!`/`|`, nested `{…}` and `[…]`, directives). `base` (optional) is the base
    /// IRI that relative IRIs and the `owl:Ontology` subject resolve against; pass
    /// `undefined`/`null` for the SCS no-`BASE` convention
    /// (`urn:x-base:default`). A document-level `BASE` directive overrides it.
    ///
    /// The returned Turtle is byte-for-byte what [`serialize`](Self::serialize)
    /// produces for the same graph with `("turtle", pretty=true, indent="  ",
    /// abbreviate=true)` — a sorted, blank-line-separated, `@prefix`-headed document
    /// (the `sh:` / `rdf:` / `rdfs:` / `xsd:` / `owl:` well-known prefixes are
    /// compacted). It re-parses as standard Turtle, and the shapes it carries
    /// validate data **identically** to the equivalent hand-written Turtle shapes —
    /// it is the same triples [`validate`](Self::validate) consumes. This is the
    /// SCS *input* counterpart for the playground's "Compact → shapes" mode.
    ///
    /// This is a **stateless** one-shot — it does not consult the receiver's stored
    /// triples (build a throwaway store with `Store.load("", "turtle")` to call it).
    /// Errors only when SCS parsing fails (a `JsError` carrying the parser's message
    /// + 1-based line); serialising the parsed graph is infallible. Available only
    /// when the crate is built with the OPT-IN `scs` feature (which implies `shacl`
    /// + `serialize-rdf`) — the site REPL bundle enables it; the lean default bundle
    /// does not.
    #[wasm_bindgen(js_name = parseShaclCompact)]
    pub fn parse_shacl_compact(&self, text: &str, base: Option<String>) -> Result<String, JsError> {
        let base = base.as_deref().unwrap_or(DEFAULT_BASE);
        let graph = parse_scs_to_graph(text, base).map_err(|e| JsError::new(&e.to_string()))?;
        // REUSE the existing engine-writer path: wrap the shapes graph in a throwaway
        // Store and emit it through `Store::serialize` (the #900/#923 serializer),
        // rather than rolling a second serialiser. Pretty Turtle, 2-space indent,
        // abbreviate on; `prefixes = None` (sq-l5kr serialize signature)
        // selects the engine's well-known defaults — the readable default a UI can
        // show straight away, byte-for-byte the prior 4-arg default-prefix output.
        let shapes_store = Store { graph };
        shapes_store.serialize("turtle", true, Some("  ".to_string()), true, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    // A small SCS document exercising the common shapes-graph constructs: a node
    // shape with a target class, a property shape with a path, datatype and a
    // `[min..max]` cardinality, and a `nodeKind`. The parser + serialiser run
    // natively (the `Ok` arm never touches `JsError::new`), so the round-trip
    // contract is tested here without a wasm runtime — mirroring the other wasm
    // bindings' native-test convention. The negative (`Err`) arm (a malformed SCS
    // document) is covered by the wasm32 `tests/web.rs::scs_parse_error_is_err`.
    const SCS: &str = "\
PREFIX ex: <http://example.org/>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>

shapeClass ex:Person {
\tex:name xsd:string [1..1] .
\tex:age xsd:integer .
}
";

    /// `parseShaclCompact` parses an SCS document and returns the shapes graph as a
    /// Turtle string that (a) re-parses, and (b) carries the key shape triples — the
    /// `sh:NodeShape` type, the target class, and the property paths — i.e. the
    /// load-bearing invariant that the SCS *input* yields the real shapes graph.
    #[test]
    fn parse_scs_round_trips_to_shapes_graph() {
        let store = Store::load("", "turtle").unwrap();
        let ttl = store.parse_shacl_compact(SCS, None).unwrap();

        // The output re-parses as standard Turtle.
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        assert!(!g.is_empty(), "shapes graph is non-empty: {ttl}");

        // Key shape triples are present (the SCS produced the real SHACL shapes).
        // `shapeClass` => the node shape IS the target class, typed as both
        // sh:NodeShape and rdfs:Class with an implicit sh:targetClass of itself.
        assert!(
            ttl.contains("sh:NodeShape"),
            "node shape type present: {ttl}"
        );
        assert!(
            ttl.contains("<http://example.org/Person>"),
            "the Person shape IRI is present: {ttl}"
        );
        // The two property paths surface.
        assert!(
            ttl.contains("<http://example.org/name>"),
            "ex:name property path present: {ttl}"
        );
        assert!(
            ttl.contains("<http://example.org/age>"),
            "ex:age property path present: {ttl}"
        );
        // The datatype constraint and the `[1..1]` cardinality reached the graph.
        assert!(ttl.contains("xsd:string"), "sh:datatype present: {ttl}");

        // Parity — the binding reuses `Store::serialize` and adds NO reformatting:
        // running the SAME shapes graph through `Store::serialize("turtle", true, "  ",
        // true)` (the call the binding makes) is byte-identical to the engine writer
        // `graph_to_turtle_pretty_with(.., abbreviate=true)` it documents. (We cannot
        // byte-compare against a SECOND independent SCS parse: the parser mints fresh
        // blank-node labels each call, so two parses are isomorphic but not equal — that
        // is exactly why this parity is asserted over ONE graph object, while the key
        // triples above carry the binding's actual output.)
        use sparq_engine::serialize::{
            default_prefixes, graph_to_turtle_pretty_with, PrettyOptions,
        };
        let shapes = parse_scs_to_graph(SCS, DEFAULT_BASE).unwrap();
        let via_store = Store { graph: shapes };
        let store_ttl = via_store
            .serialize("turtle", true, Some("  ".to_string()), true, None)
            .unwrap();
        let via_engine = graph_to_turtle_pretty_with(
            &via_store.graph,
            &default_prefixes(),
            &PrettyOptions {
                indent: "  ".to_string(),
                abbreviate: true,
            },
        );
        assert_eq!(
            store_ttl, via_engine,
            "the binding's Store::serialize call must equal the documented engine writer"
        );
    }

    /// The parsed shapes graph VALIDATES data identically to hand-written Turtle
    /// shapes — the real end-to-end invariant: an SCS `[1..1]` minCount on ex:name
    /// flags a node missing that property. Drives `sparq_shacl::validate` directly
    /// (the SCS parse + the validate engine), the genuine path the playground wires.
    #[test]
    fn parsed_scs_shapes_drive_validation() {
        let shapes = parse_scs_to_graph(SCS, DEFAULT_BASE).unwrap();
        // ex:bob is an ex:Person with an age but NO name — the [1..1] on ex:name is a
        // minCount 1 violation.
        let data = Graph::load_str(
            "@prefix ex: <http://example.org/> . ex:bob a ex:Person ; ex:age 7 .",
            "turtle",
        )
        .unwrap();
        let report = sparq_shacl::validate(&data, &shapes);
        assert!(
            !report.conforms,
            "missing ex:name must violate the SCS [1..1] minCount"
        );
        assert!(
            report
                .results
                .iter()
                .any(|r| r.source_component.contains("MinCount")),
            "a minCount violation must be present: {:?}",
            report.results
        );
    }

    /// A document-level `BASE` directive (and the `base` argument default) resolve
    /// relative IRIs: with no `base` argument the SCS `BASE` wins for relative refs.
    #[test]
    fn parse_scs_respects_base() {
        let store = Store::load("", "turtle").unwrap();
        let scs = "BASE <http://b.example/>\nshape <#S> {\n}\n";
        let ttl = store.parse_shacl_compact(scs, None).unwrap();
        assert!(
            ttl.contains("<http://b.example/#S>"),
            "document BASE resolves the relative shape IRI: {ttl}"
        );
        // The explicit `base` argument is used when the document declares no BASE.
        let scs2 = "shape <#T> {\n}\n";
        let ttl2 = store
            .parse_shacl_compact(scs2, Some("http://arg.example/".to_string()))
            .unwrap();
        assert!(
            ttl2.contains("<http://arg.example/#T>"),
            "the base argument resolves the relative shape IRI: {ttl2}"
        );
    }
}

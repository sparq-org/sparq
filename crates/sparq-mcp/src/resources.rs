//! [SONNET-4.6] sq-sjey1 (gh #3220) The MCP **resources** surface of the base
//! [`McpServer`](crate::McpServer): the served dataset and each of its graphs, exposed
//! as readable MCP resources.
//!
//! MCP splits agent-facing state into *tools* (model-invoked actions) and *resources*
//! (application-selected, addressable context a client can attach to a conversation).
//! `sparq-mcp` already had the tools; this module is the resource half, and it is a
//! THIN projection of surfaces that already existed — nothing new is computed:
//!
//! - [`DATASET_URI`] → the W3C VoID descriptor `sparq_introspect` already mines (the
//!   same bytes the `void` tool returns for the default `dataset` argument);
//! - [`DEFAULT_GRAPH_URI`] → the default graph's triples, materialised through the SAME
//!   budgeted `sparq_engine` CONSTRUCT path the `construct` tool uses, so a resource
//!   read is bounded by the server's [`QueryBudget`] exactly like a tool call;
//! - one resource per IRI-named, non-reserved NAMED graph, whose `uri` **is** that IRI.
//!
//! The surface is READ-ONLY and always present in the default build. It adds no engine
//! capability and no crate to the build: reading `Graph::named` needs `oxrdf`, which this
//! crate now names directly, but `sparq-core` already depended on it unconditionally, so
//! every `sparq-mcp` build has always compiled it.
//!
//! ## Size
//!
//! `resources/read` on a graph materialises that graph's WHOLE N-Triples serialization in
//! memory. The server's row cap is the only thing bounding that — a blunt ceiling, not a
//! paging mechanism (MCP resources have no pagination). This is the deliberate "hand the
//! agent the whole document" operation resources exist for; for anything selective, use
//! the `query` / `construct` tools instead.

use oxrdf::Term;
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_introspect::Introspection;

/// The whole-dataset resource: a W3C VoID descriptor of the served dataset.
///
/// RESERVED — a named graph carrying this IRI is shadowed by the dataset descriptor.
pub const DATASET_URI: &str = "urn:sparq:dataset";

/// The default (unnamed) graph of the served dataset, as N-Triples.
///
/// RESERVED — a named graph carrying this IRI is shadowed by the default graph.
pub const DEFAULT_GRAPH_URI: &str = "urn:sparq:graph:default";

/// The media type every resource this module serves is delivered as. Both the VoID
/// descriptor and a graph dump are N-Triples.
pub const NTRIPLES_MIME: &str = "application/n-triples";

/// The CONSTRUCT that copies a whole graph. Routing a resource read through the engine
/// (rather than a bespoke serializer) is what makes the read inherit the server's
/// deadline + row cap.
const DUMP_QUERY: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";

/// Why a [`read`] failed. The two cases are distinct on the wire: a URI this server does
/// not serve is MCP's `RESOURCE_NOT_FOUND`, whereas a served resource that could not be
/// materialised (a tripped query budget, say) is an internal error — reporting the latter
/// as "not found" would tell the agent the resource does not exist, which is false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// No resource with this URI is served.
    NotFound(String),
    /// The resource is served but could not be materialised.
    Failed(String),
}

impl ReadError {
    /// The human/agent-facing message.
    pub fn message(&self) -> &str {
        match self {
            ReadError::NotFound(m) | ReadError::Failed(m) => m,
        }
    }
}

/// The `resources/list` entries for `graph`: the dataset descriptor, the default graph,
/// and one entry per named graph (sorted by IRI so the listing is deterministic).
///
/// A named graph whose name is not an IRI (a blank node) has no addressable URI and is
/// omitted rather than given an invented one, as is one carrying a RESERVED URI —
/// [`read`] would resolve that URI to the dataset/default entry, so listing it as a named
/// graph as well would advertise a resource nothing serves.
pub fn descriptors(graph: &Graph) -> Vec<Value> {
    let mut out = vec![
        json!({
            "uri": DATASET_URI,
            "name": "dataset",
            "description": "W3C VoID descriptor of the served dataset \
                            (the `void` tool's default output).",
            "mimeType": NTRIPLES_MIME,
        }),
        json!({
            "uri": DEFAULT_GRAPH_URI,
            "name": "default graph",
            "description": "Every triple of the default (unnamed) graph, as N-Triples.",
            "mimeType": NTRIPLES_MIME,
        }),
    ];
    let mut named: Vec<&str> = graph
        .named
        .iter()
        .filter_map(|(name, _)| match name {
            Term::NamedNode(node) if !is_reserved(node.as_str()) => Some(node.as_str()),
            _ => None,
        })
        .collect();
    named.sort_unstable();
    out.extend(named.into_iter().map(|iri| {
        json!({
            // The graph IRI IS the resource URI — the dataset is the only authority on
            // what a graph is called, so no prettier name is invented.
            "uri": iri,
            "name": iri,
            "description": "Every triple of this named graph, as N-Triples.",
            "mimeType": NTRIPLES_MIME,
        })
    }));
    out
}

/// Materialise the resource `uri` serves, under `budget`.
///
/// [`DATASET_URI`] yields the VoID descriptor; [`DEFAULT_GRAPH_URI`] and any named-graph
/// IRI yield that graph's N-Triples, produced by the same budgeted engine CONSTRUCT the
/// `construct` tool runs. Any other URI is [`ReadError::NotFound`].
pub fn read(graph: &Graph, uri: &str, budget: &QueryBudget) -> Result<String, ReadError> {
    if uri == DATASET_URI {
        return Ok(Introspection::build(graph).to_void(DATASET_URI));
    }
    let target = if uri == DEFAULT_GRAPH_URI {
        graph
    } else {
        named_graph(graph, uri)
            .ok_or_else(|| ReadError::NotFound(format!("no resource is served at <{}>", uri)))?
    };
    sparq_engine::construct_ntriples_with_budget(target, DUMP_QUERY, budget)
        .map_err(ReadError::Failed)
}

/// Whether `uri` is one of the two URIs this module reserves for the dataset-level
/// resources, and so cannot address a named graph.
fn is_reserved(uri: &str) -> bool {
    uri == DATASET_URI || uri == DEFAULT_GRAPH_URI
}

/// The named graph of `graph` whose name is the IRI `iri`, if any.
fn named_graph<'a>(graph: &'a Graph, iri: &str) -> Option<&'a Graph> {
    graph.named.iter().find_map(|(name, g)| match name {
        Term::NamedNode(node) if node.as_str() == iri => Some(g),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIG: &str = r#"@prefix ex: <http://ex/> .
ex:alice ex:name "Alice" .
<http://ex/g1> { ex:bob ex:name "Bob" . }
<http://ex/g0> { ex:carol ex:name "Carol" . }
"#;

    fn dataset() -> Graph {
        Graph::load_dataset(TRIG, "trig").expect("load trig")
    }

    fn uris(graph: &Graph) -> Vec<String> {
        descriptors(graph)
            .iter()
            .map(|d| d["uri"].as_str().expect("uri is a string").to_string())
            .collect()
    }

    #[test]
    fn descriptors_list_dataset_default_and_named_graphs_in_order() {
        assert_eq!(
            uris(&dataset()),
            vec![
                DATASET_URI.to_string(),
                DEFAULT_GRAPH_URI.to_string(),
                // Sorted by IRI, so g0 precedes g1 even though TriG declared g1 first.
                "http://ex/g0".to_string(),
                "http://ex/g1".to_string(),
            ]
        );
    }

    #[test]
    fn descriptors_carry_the_ntriples_media_type() {
        for d in descriptors(&dataset()) {
            assert_eq!(d["mimeType"].as_str(), Some(NTRIPLES_MIME));
        }
    }

    #[test]
    fn read_default_graph_returns_only_default_graph_triples() {
        let text = read(&dataset(), DEFAULT_GRAPH_URI, &QueryBudget::unlimited()).expect("read");
        assert!(text.contains("Alice"), "default graph triple missing: {}", text);
        assert!(!text.contains("Bob"), "named-graph triple leaked: {}", text);
    }

    #[test]
    fn read_named_graph_returns_only_that_graphs_triples() {
        let text = read(&dataset(), "http://ex/g1", &QueryBudget::unlimited()).expect("read");
        assert!(text.contains("Bob"), "named graph triple missing: {}", text);
        assert!(!text.contains("Alice"), "default-graph triple leaked: {}", text);
        assert!(!text.contains("Carol"), "sibling named-graph triple leaked: {}", text);
    }

    #[test]
    fn read_dataset_returns_the_void_descriptor() {
        let text = read(&dataset(), DATASET_URI, &QueryBudget::unlimited()).expect("read");
        assert!(text.contains(DATASET_URI), "VoID subject missing: {}", text);
        assert!(text.contains("rdfs.org/ns/void#"), "not a VoID descriptor: {}", text);
    }

    #[test]
    fn read_unknown_uri_is_not_found() {
        let err = read(&dataset(), "http://ex/nope", &QueryBudget::unlimited())
            .expect_err("unknown URI must not resolve");
        assert!(matches!(err, ReadError::NotFound(_)), "{:?}", err);
        assert!(err.message().contains("http://ex/nope"), "{}", err.message());
    }

    /// A named graph carrying a RESERVED URI is shadowed by the dataset-level resource in
    /// `read`, so it must not be listed a second time as a named graph.
    #[test]
    fn a_named_graph_on_a_reserved_uri_is_not_listed_twice() {
        let trig = format!(
            "<{}> {{ <http://ex/x> <http://ex/p> \"shadowed\" . }}",
            DEFAULT_GRAPH_URI
        );
        let graph = Graph::load_dataset(&trig, "trig").expect("load trig");
        assert_eq!(
            uris(&graph),
            vec![DATASET_URI.to_string(), DEFAULT_GRAPH_URI.to_string()]
        );
        // And the reserved URI still resolves to the DEFAULT graph, not the shadowed one.
        let text = read(&graph, DEFAULT_GRAPH_URI, &QueryBudget::unlimited()).expect("read");
        assert!(!text.contains("shadowed"), "reserved URI resolved to the named graph: {}", text);
    }

    #[test]
    fn read_error_message_reads_both_variants() {
        assert_eq!(ReadError::Failed("boom".into()).message(), "boom");
        assert_eq!(ReadError::NotFound("gone".into()).message(), "gone");
    }
}

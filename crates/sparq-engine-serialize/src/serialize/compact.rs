//! JSON-LD 1.1 **Compaction** and **Framing** of an RDF dataset against a caller-supplied
//! `@context` / frame.
//!
//! Both writers are thin adapters over the native document-level pipeline in
//! [`sparq_jsonld`] — the same algorithms the W3C `compact` and `frame` conformance lanes
//! measure (`crates/sparq-conformance/src/floors/{compact,frame}.rs`):
//!
//! 1. **fromRdf** — the dataset's quads go through [`sparq_jsonld::from_rdf::from_rdf`]
//!    (`@list` collapse, `rdf:type` → `@type`, named graphs as `@graph` nodes). Canonical
//!    `xsd:integer` / `xsd:boolean` literals then become native JSON scalars, exactly as the
//!    expanded/flattened writer in the parent module emits them, so the coercion stays
//!    lossless (a non-canonical lexical such as `"007"` keeps its `@value` string).
//! 2. **Compaction** ([`sparq_jsonld::compact::compact_expanded`]) or **Framing**
//!    ([`sparq_jsonld::frame::frame`]) under default [`JsonLdOptions`] and the deny-by-default
//!    [`NoopLoader`] (a remote `@context` IRI is never fetched).
//!
//! The writers stay total: a `@context` / frame the processor rejects (e.g. an invalid term
//! definition, a remote context) yields the lossless **expanded** document instead, so the
//! output always re-reads as the same RDF.
//!
//! An RDF 1.2 triple term has no JSON-LD encoding; it is carried as an opaque `@id` holding
//! its N-Triples spelling, the same choice the parent writer makes.

use super::{coerce_native, NamedGraph};
use oxrdf::{NamedOrBlankNode, Term, Triple};
use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
use sparq_jsonld::frame::{compact_framed, frame_match_from_rdf, FrameOptions};
use sparq_jsonld::compact::compact_expanded;
use sparq_jsonld::{JsonLdOptions, NoopLoader, ProcessingMode};

pub use sparq_jsonld::{ActiveContext, Json};

fn rdf_term(term: &Term) -> RdfTerm {
    match term {
        Term::NamedNode(n) => RdfTerm::iri(n.as_str()),
        Term::BlankNode(b) => RdfTerm::blank(b.as_str()),
        Term::Literal(l) => match l.language() {
            Some(lang) => RdfTerm::lang_literal(l.value(), lang),
            None => RdfTerm::typed_literal(l.value(), l.datatype().as_str()),
        },
        Term::Triple(_) => RdfTerm::iri(term.to_string()),
    }
}

fn rdf_quads(graphs: &[NamedGraph<'_>]) -> Vec<RdfQuad> {
    let mut quads = Vec::with_capacity(graphs.iter().map(|(_, ts)| ts.len()).sum());
    for (name, triples) in graphs {
        let graph = name.map(rdf_term);
        quads.extend(triples.iter().map(|t: &Triple| {
            RdfQuad::new(
                match &t.subject {
                    NamedOrBlankNode::NamedNode(n) => RdfTerm::iri(n.as_str()),
                    NamedOrBlankNode::BlankNode(b) => RdfTerm::blank(b.as_str()),
                },
                RdfTerm::iri(t.predicate.as_str()),
                rdf_term(&t.object),
                graph.clone(),
            )
        }));
    }
    quads
}

/// Rewrites `{"@value": "<canonical lexical>", "@type": xsd:integer|xsd:boolean}` value
/// objects to native JSON scalars, in place (see the module docs).
fn native_scalars(v: &mut Json) {
    match v {
        Json::Arr(items) => items.iter_mut().for_each(native_scalars),
        Json::Obj(members) => {
            // fromRdf emits typed value objects as exactly [@value, @type].
            if let [(k, Json::Str(lex)), (t, Json::Str(dt))] = members.as_slice() {
                if k == "@value" && t == "@type" {
                    if let Some(raw) = coerce_native(lex, dt) {
                        *members = vec![("@value".to_string(), Json::Raw(raw))];
                    }
                    return;
                }
            }
            // A value object holds no nested node objects (and a @json payload is opaque).
            if members.first().is_some_and(|(k, _)| k == "@value") {
                return;
            }
            members.iter_mut().for_each(|(_, m)| native_scalars(m));
        }
        _ => {}
    }
}

/// The lossless expanded fromRdf document for `graphs`.
///
/// Converted in 1.0 mode so every `rdf:JSON` literal stays a typed string with its
/// exact lexical form: 1.1 `@json` decoding would normalise (or, for malformed JSON,
/// reject) it, and a `@json` array payload cannot be told apart from multiple values
/// once compacted. Typed list cells stay explicit nodes, so their `rdf:type rdf:List`
/// triple survives.
fn expanded(graphs: &[NamedGraph<'_>]) -> Json {
    let mut options = FromRdfOptions::default();
    options.processing_mode = ProcessingMode::JsonLd10;
    options.keep_typed_list_cells = true;
    // With @json decoding and compound literals both off, no term can fail.
    let mut doc =
        from_rdf(&rdf_quads(graphs), &options).expect("1.0-mode fromRdf of RDF terms is total");
    native_scalars(&mut doc);
    add_empty_graphs(&mut doc, graphs);
    doc
}

/// Adds `{"@id": g, "@graph": []}` for each named graph without triples, which the
/// quads alone cannot carry, keeping the nodes in fromRdf's `@id` order.
fn add_empty_graphs(doc: &mut Json, graphs: &[NamedGraph<'_>]) {
    let Json::Arr(nodes) = doc else { return };
    for (name, triples) in graphs {
        let id = match name {
            Some(Term::NamedNode(n)) if triples.is_empty() => n.as_str().to_string(),
            Some(Term::BlankNode(b)) if triples.is_empty() => format!("_:{}", b.as_str()),
            _ => continue,
        };
        let pos = nodes.binary_search_by(|n| n.get("@id").and_then(Json::as_str).unwrap_or("").cmp(&id));
        match pos {
            Ok(i) => {
                if nodes[i].get("@graph").is_none() {
                    nodes[i].set("@graph", Json::Arr(Vec::new()));
                }
            }
            Err(i) => {
                let mut node = Json::obj();
                node.set("@id", Json::Str(id));
                node.set("@graph", Json::Arr(Vec::new()));
                nodes.insert(i, node);
            }
        }
    }
}

fn render(doc: &Json) -> String {
    let mut out = String::new();
    doc.write(&mut out);
    out
}

/// Serialises an RDF dataset (default + named graphs) as a **compacted** JSON-LD 1.1
/// document against the caller-supplied `context` (a `@context` value, or a document whose
/// `@context` member is used), applying the W3C JSON-LD 1.1 Compaction Algorithm.
///
/// Round-tripping the output through a JSON-LD-to-RDF processor reconstructs the same
/// dataset. A `context` the processor rejects yields the expanded document (module docs).
pub fn write_jsonld_compact(graphs: &[NamedGraph<'_>], context: &Json) -> String {
    let doc = expanded(graphs);
    let compact = |ctx: &Json| compact_expanded(&doc, ctx, &JsonLdOptions::default(), &NoopLoader);
    match compact(context) {
        Ok(compacted) => match readable_type_maps(&compacted, context) {
            None => render(&compacted),
            Some(stripped) => render(&compact(&stripped).unwrap_or(compacted)),
        },
        Err(_) => render(&doc),
    }
}

/// `None` when every `@type` map in `out` holds only node references. Otherwise a copy of
/// `context` without `@type` containers: oxjsonld (sparq's JSON-LD reader) cannot read a
/// node object inside a type map, so such output is redone with plain properties.
fn readable_type_maps(out: &Json, context: &Json) -> Option<Json> {
    if !has_type_container(context) {
        return None;
    }
    let mut stripped = context.clone();
    let mut terms = Vec::new();
    strip_type_containers(&mut stripped, &mut terms);
    (!terms.is_empty() && holds_typed_nodes(out, &terms)).then_some(stripped)
}

/// Whether some `@container` in `ctx` (or its scoped contexts) names `@type`.
fn has_type_container(ctx: &Json) -> bool {
    match ctx {
        Json::Arr(items) => items.iter().any(has_type_container),
        Json::Obj(members) => members.iter().any(|(k, v)| match (k.as_str(), v) {
            ("@container", Json::Str(c)) => c == "@type",
            ("@container", Json::Arr(cs)) => cs.iter().any(|c| c.as_str() == Some("@type")),
            _ => has_type_container(v),
        }),
        _ => false,
    }
}

/// Removes `@type` from every term's `@container` in `ctx` (and its scoped contexts),
/// collecting the names of the terms changed.
fn strip_type_containers(ctx: &mut Json, terms: &mut Vec<String>) {
    match ctx {
        Json::Arr(items) => items.iter_mut().for_each(|c| strip_type_containers(c, terms)),
        Json::Obj(members) => {
            for (name, def) in members.iter_mut() {
                if name == "@context" {
                    strip_type_containers(def, terms);
                    continue;
                }
                let Json::Obj(fields) = def else { continue };
                let mut stripped = false;
                for (key, val) in fields.iter_mut() {
                    match (key.as_str(), val) {
                        ("@context", scoped) => strip_type_containers(scoped, terms),
                        ("@container", Json::Arr(cs)) => {
                            let n = cs.len();
                            cs.retain(|c| c.as_str() != Some("@type"));
                            stripped |= cs.len() != n;
                        }
                        ("@container", c) => stripped |= c.as_str() == Some("@type"),
                        _ => {}
                    }
                }
                if stripped {
                    // A container left empty (or that was just "@type") is dropped.
                    fields.retain(|(k, v)| {
                        k != "@container" || matches!(v, Json::Arr(cs) if !cs.is_empty())
                    });
                    terms.push(name.clone());
                }
            }
        }
        _ => {}
    }
}

/// Whether some entry of `doc` named in `terms` is a map holding a node object.
fn holds_typed_nodes(doc: &Json, terms: &[String]) -> bool {
    let is_node = |v: &Json| match v {
        Json::Obj(_) => true,
        Json::Arr(a) => a.iter().any(|x| matches!(x, Json::Obj(_))),
        _ => false,
    };
    match doc {
        Json::Arr(items) => items.iter().any(|d| holds_typed_nodes(d, terms)),
        Json::Obj(members) => members.iter().any(|(k, v)| {
            k != "@context"
                && ((terms.contains(k)
                    && matches!(v, Json::Obj(m) if m.iter().any(|(_, x)| is_node(x))))
                    || holds_typed_nodes(v, terms))
        }),
        _ => false,
    }
}

/// Frames an RDF dataset against a caller-supplied JSON-LD **frame** document, applying the
/// W3C JSON-LD 1.1 Framing Algorithm (node-pattern matching, `@embed`, `@explicit`,
/// `@default` / `@omitDefault`, `@requireAll`, list and named-graph framing), and compacts
/// the result against the frame's `@context`.
///
/// A single matched root collapses to the bare framed node merged with `@context` (the
/// JSON-LD 1.1 `omitGraph` default). A frame the processor rejects (e.g. an invalid
/// `@embed` value) yields the expanded document (module docs).
pub fn write_jsonld_framed(graphs: &[NamedGraph<'_>], frame_doc: &Json) -> String {
    let opts = JsonLdOptions::default();
    let fopts = FrameOptions::default();
    let ctx = frame_doc.get("@context").cloned().unwrap_or_default();
    let out = frame_match_from_rdf(expanded(graphs), frame_doc, &opts, &fopts, &NoopLoader).and_then(|matched| {
        let framed = compact_framed(&matched, &ctx, &opts, &fopts, &NoopLoader)?;
        // The match stands; only its compaction is redone for a readable shape.
        Ok(match readable_type_maps(&framed, &ctx) {
            None => framed,
            Some(stripped) => compact_framed(&matched, &stripped, &opts, &fopts, &NoopLoader)
                .unwrap_or(framed),
        })
    });
    match out {
        Ok(framed) => render(&framed),
        // A frame that cannot be applied leaves the document unframed.
        Err(_) => render(&expanded(graphs)),
    }
}

/// Parses a caller `@context` (or frame) from its JSON text. Returns `None` unless the text
/// is a single JSON object.
pub fn parse_context_json(text: &str) -> Option<Json> {
    Json::parse(text).ok().filter(Json::is_obj)
}

// Framing regressions: each expected string is the pyld W3C reference processor's
// `jsonld.frame` output for the same RDF (sq-oy1f.17).
#[cfg(test)]
mod tests {
    use super::super::{graph_to_jsonld_framed, parse_context_json};
    use sparq_core::Graph;

    /// JSON equality with object member order ignored (pyld sorts members; sparq keeps
    /// the frame's order for filled defaults).
    fn json_eq(a: &str, b: &str) -> bool {
        let p = |s: &str| serde_json::from_str::<serde_json::Value>(s).expect("valid JSON");
        p(a) == p(b)
    }

    /// Frames `ttl` (Turtle) against `frame_json` (a JSON-LD frame) via the REAL public path.
    fn frame_doc(ttl: &str, frame_json: &str) -> String {
        let g = Graph::load_str(ttl, "turtle").expect("load turtle");
        let frame = parse_context_json(frame_json).expect("frame json object");
        graph_to_jsonld_framed(&g, &frame)
    }

    /// Frames `trig` (a dataset) against `frame_json`.
    fn frame_dataset(trig: &str, frame_json: &str) -> String {
        let g = Graph::load_dataset(trig, "trig").expect("load trig");
        let frame = parse_context_json(frame_json).expect("frame json object");
        graph_to_jsonld_framed(&g, &frame)
    }

    // The expected strings below are the EXACT output of the pyld W3C reference
    // processor's `jsonld.frame(input, frame)` on the same RDF (captured via a pyld
    // venv during development, sq-oy1f.17). They pin the spec-correct framed shape as
    // a permanent regression: @type / property-presence / value / wildcard / match-none
    // node matching, @embed @once/@always/@never, @explicit, @default/@omitDefault,
    // @requireAll AND/OR, list framing, and the @graph / omitGraph envelope.

    #[test]
    fn type_frame_embeds_referenced_node() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Library> ; \
             <http://ex/contains> <http://ex/2> .\n\
             <http://ex/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Book> ; \
             <http://ex/title> \"T\" .",
            r#"{"@type":"http://ex/Library","http://ex/contains":{}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/Library","http://ex/contains":{"@id":"http://ex/2","@type":"http://ex/Book","http://ex/title":"T"}}"#
        );
    }

    #[test]
    fn explicit_prunes_unframed_properties() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> ; \
             <http://ex/a> \"A\" ; <http://ex/b> \"B\" .",
            r#"{"@type":"http://ex/T","@explicit":true,"http://ex/a":{}}"#,
        );
        // @explicit:true drops ex:b (not named in the frame).
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/a":"A"}"#
        );
    }

    /// `"@id": []` matches no node; a wildcard in an `@id` list matches every node and
    /// still honours `@explicit`.
    #[test]
    fn id_patterns_match_none_or_any() {
        let ttl = "<http://ex/s> <http://ex/p> \"x\" ; <http://ex/q> \"y\" .";
        let none = frame_doc(ttl, r#"{"@id":[]}"#);
        assert!(!none.contains("http://ex/s"), "{none}");
        let any = frame_doc(ttl, r#"{"@id":[{}],"@explicit":true,"http://ex/p":{}}"#);
        assert_eq!(any, r#"{"@id":"http://ex/s","http://ex/p":"x"}"#);
    }

    /// `@id` and `@type` must both match: a listed node whose type differs is not framed,
    /// whichever member comes first.
    #[test]
    fn id_and_type_patterns_both_constrain() {
        let ttl = "<http://ex/s> a <http://ex/T> ; <http://ex/p> \"x\" .";
        for frame in [
            r#"{"@id":"http://ex/s","@type":"http://ex/Missing"}"#,
            r#"{"@type":"http://ex/Missing","@id":"http://ex/s"}"#,
        ] {
            let out = frame_doc(ttl, frame);
            assert!(!out.contains("http://ex/s"), "{frame}: {out}");
        }
        let out = frame_doc(ttl, r#"{"@type":"http://ex/T","@id":"http://ex/s"}"#);
        assert!(out.contains("http://ex/s"), "{out}");
    }

    /// Value patterns follow the W3C framing suite (#t0045): a pattern that omits
    /// `@language` or `@type` admits only values without one, and naming it (or `{}`)
    /// admits the tagged or typed literal.
    #[test]
    fn value_patterns_constrain_type_and_language() {
        for ttl in [
            "<http://ex/s> <http://ex/p> \"yes\"@en .",
            "<http://ex/s> <http://ex/p> \"yes\"^^<http://ex/T> .",
        ] {
            let out = frame_doc(ttl, r#"{"http://ex/p":{"@value":"yes"}}"#);
            assert!(!out.contains("http://ex/s"), "{ttl}: {out}");
        }
        let out = frame_doc("<http://ex/s> <http://ex/p> \"yes\"@en .", r#"{"http://ex/p":{"@value":"yes","@language":"en"}}"#);
        assert!(out.contains("http://ex/s"), "{out}");
        let out = frame_doc("<http://ex/s> <http://ex/p> \"yes\"^^<http://ex/T> .", r#"{"http://ex/p":{"@value":"yes","@type":{}}}"#);
        assert!(out.contains("http://ex/s"), "{out}");
    }

    #[test]
    fn default_fills_absent_property() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"FB"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/m":"FB"}"#
        );
    }

    #[test]
    fn omit_default_drops_absent_defaulted_property() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"FB","@omitDefault":true}}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","@type":"http://ex/T"}"#);
    }

    #[test]
    fn default_null_emits_preserve_null() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","http://ex/m":{"@default":"@null"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/m":null}"#
        );
    }

    #[test]
    fn embed_never_emits_node_reference() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\n<http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":"@never"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/2"}}"#
        );
    }

    #[test]
    fn embed_once_deduplicates_shared_node() {
        // @once (the default): a node shared by two properties is embedded the FIRST time
        // and referenced thereafter — result-wide dedup, matching pyld.
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/x> ; <http://ex/q> <http://ex/x> .\n\
             <http://ex/x> <http://ex/v> \"X\" .",
            r#"{"@id":"http://ex/1"}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/x","http://ex/v":"X"},"http://ex/q":{"@id":"http://ex/x"}}"#
        );
    }

    #[test]
    fn embed_always_reembeds_shared_node() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/x> ; <http://ex/q> <http://ex/x> .\n\
             <http://ex/x> <http://ex/v> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":"@always"},"http://ex/q":{"@embed":"@always"}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/x","http://ex/v":"X"},"http://ex/q":{"@id":"http://ex/x","http://ex/v":"X"}}"#
        );
    }

    #[test]
    fn require_all_conjoins_frame_properties() {
        // @requireAll:true → only the node carrying BOTH ex:a AND ex:b is selected.
        let out = frame_doc(
            "<http://ex/1> <http://ex/a> \"A\" ; <http://ex/b> \"B\" .\n<http://ex/2> <http://ex/a> \"A\" .",
            r#"{"http://ex/a":{},"http://ex/b":{},"@requireAll":true}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/a":"A","http://ex/b":"B"}"#
        );
    }

    #[test]
    fn require_all_false_disjoins_to_two_roots_with_preserve_null() {
        // The default OR selects BOTH nodes; the framed-but-absent property is preserve-null.
        // Multiple roots → the @graph envelope.
        let out = frame_doc(
            "<http://ex/1> <http://ex/a> \"A\" .\n<http://ex/2> <http://ex/b> \"B\" .",
            r#"{"http://ex/a":{},"http://ex/b":{},"@requireAll":false}"#,
        );
        let want = r#"{"@graph":[{"@id":"http://ex/1","http://ex/a":"A","http://ex/b":null},{"@id":"http://ex/2","http://ex/a":null,"http://ex/b":"B"}]}"#;
        assert!(json_eq(&out, want), "{out}");
    }

    #[test]
    fn value_match_selects_by_literal() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"yes\" .\n<http://ex/2> <http://ex/p> \"no\" .",
            r#"{"http://ex/p":[{"@value":"yes"}]}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","http://ex/p":"yes"}"#);
    }

    #[test]
    fn match_none_requires_property_absent() {
        // The frame names ex:p as match-none `[]` → only the node WITHOUT ex:p is selected.
        // ex:p is then a framed-but-absent property → preserve-null.
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"P\" .\n<http://ex/2> <http://ex/q> \"Q\" .",
            r#"{"http://ex/p":[]}"#,
        );
        let want = r#"{"@id":"http://ex/2","http://ex/p":null,"http://ex/q":"Q"}"#;
        assert!(json_eq(&out, want), "{out}");
    }

    #[test]
    fn wildcard_frame_matches_all() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/p> \"P\" .\n<http://ex/2> <http://ex/q> \"Q\" .",
            r#"{}"#,
        );
        assert_eq!(
            out,
            r#"{"@graph":[{"@id":"http://ex/1","http://ex/p":"P"},{"@id":"http://ex/2","http://ex/q":"Q"}]}"#
        );
    }

    #[test]
    fn circular_reference_terminates() {
        // a → b → a: the embed link table breaks the cycle with a node reference.
        let out = frame_doc(
            "<http://ex/a> <http://ex/next> <http://ex/b> .\n<http://ex/b> <http://ex/next> <http://ex/a> .",
            r#"{"@id":"http://ex/a"}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/a","http://ex/next":{"@id":"http://ex/b","http://ex/next":{"@id":"http://ex/a"}}}"#
        );
    }

    #[test]
    fn blank_node_cycle_terminates() {
        // A genuine blank-node cycle must terminate (no infinite recursion) and produce a
        // valid framed tree whose innermost back-edge is a bare reference. Blank-node labels
        // are processor-internal, so we assert termination + a closing reference structurally.
        let out = frame_doc(
            "_:a <http://ex/next> _:b . _:b <http://ex/next> _:a . _:a <http://ex/v> \"A\" .",
            r#"{"http://ex/v":{}}"#,
        );
        // Selected node a (has ex:v); a embeds b; b's ex:next points back to a as a reference.
        assert!(out.contains(r#""http://ex/v":"A""#), "node a framed: {out}");
        assert!(out.contains(r#""http://ex/next""#), "edge present: {out}");
        // The back-edge to a is a bare {"@id": …} reference (cycle broken), so there is no
        // third level of "http://ex/v" nesting — the document is finite.
        assert_eq!(out.matches(r#""http://ex/v""#).count(), 1, "finite: {out}");
    }

    #[test]
    fn list_framing_preserves_list_wrapper() {
        let out = frame_doc(
            "<http://ex/1> <http://ex/items> ( \"a\" \"b\" ) .",
            r#"{"http://ex/items":{}}"#,
        );
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","http://ex/items":{"@list":["a","b"]}}"#
        );
    }

    #[test]
    fn typed_literal_native_coercion() {
        let out = frame_doc("<http://ex/1> <http://ex/n> 42 .", r#"{"http://ex/n":{}}"#);
        assert_eq!(out, r#"{"@id":"http://ex/1","http://ex/n":42}"#);
    }

    #[test]
    fn context_compaction_applies() {
        // The frame's @context drives the final compaction: @vocab abbreviates the @type
        // and the property key.
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> ; \
             <http://ex/p> \"P\" .",
            r#"{"@context":{"@vocab":"http://ex/"},"@type":"T"}"#,
        );
        assert_eq!(
            out,
            r#"{"@context":{"@vocab":"http://ex/"},"@id":"http://ex/1","@type":"T","p":"P"}"#
        );
    }

    #[test]
    fn named_graph_nodes_match_in_the_merged_graph() {
        // Framing matches over the merged graph of the dataset (the default unless
        // the frame selects `@graph`), so a node in a named graph is framed directly.
        let out = frame_dataset(
            "<http://ex/g> { <http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
             <http://ex/T> ; <http://ex/v> \"V\" . }",
            r#"{"@type":"http://ex/T"}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/v":"V"}"#);
    }

    #[test]
    fn no_match_yields_empty_document() {
        // A frame that matches nothing compacts to an empty document (never panics).
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/Other"}"#,
        );
        assert_eq!(out, "{}");
    }

    // -----------------------------------------------------------------------
    // [OPUS-4.8] sq-qcnn.33 — coverage-raise tests: targeted paths not yet
    // exercised by the framing suite above.
    // -----------------------------------------------------------------------

    /// `Embed::parse` with `Json::Raw("true")` and `Json::Raw("false")` — the legacy
    /// boolean spellings for `@once` and `@never`.  The frame parser stores JSON `true`
    /// / `false` as `Json::Raw`, not `Json::Str`, so these arms were previously uncovered.
    #[test]
    fn embed_boolean_true_false() {
        // @embed: false — the legacy spelling of @never; the referenced node stays a
        // bare {"@id":…} reference, identical to using "@embed":"@never".
        let out_false = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\
             <http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":false}}"#,
        );
        assert_eq!(
            out_false,
            r#"{"@id":"http://ex/1","http://ex/p":{"@id":"http://ex/2"}}"#,
            "embed=false is @never: {}",
            out_false
        );

        // @embed: true — the legacy spelling of @once; the node is embedded the first
        // time it is reached (same result as the default, but exercises the true arm).
        let out_true = frame_doc(
            "<http://ex/1> <http://ex/p> <http://ex/2> .\
             <http://ex/2> <http://ex/q> \"X\" .",
            r#"{"@id":"http://ex/1","http://ex/p":{"@embed":true}}"#,
        );
        assert!(
            out_true.contains(r#""@id":"http://ex/2""#),
            "embed=true embeds the node: {}",
            out_true
        );
        assert!(
            out_true.contains(r#""http://ex/q":"X""#),
            "embedded node properties present: {}",
            out_true
        );
    }

    /// `@omitDefault` is a per-property-frame flag (Framing §4.1 initialises only
    /// `@embed` / `@explicit` / `@requireAll` from the node frame), so a node-level
    /// `@omitDefault` does not suppress a property frame's `@default` fill.
    #[test]
    fn top_level_omit_default_does_not_suppress_fill() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .",
            r#"{"@type":"http://ex/T","@omitDefault":true,"http://ex/m":{"@default":"FB"}}"#,
        );
        assert_eq!(out, r#"{"@id":"http://ex/1","@type":"http://ex/T","http://ex/m":"FB"}"#);
    }

    /// `match_type` wildcard `@type:{}` branch (~line 426): matches any node that has
    /// at least one type; nodes without `@type` are excluded.
    #[test]
    fn type_wildcard_matches_only_typed_nodes() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .\
             <http://ex/2> <http://ex/p> \"X\" .",
            // @type:{} = wildcard — match any node that has a type.
            r#"{"@type":{}}"#,
        );
        // ex:1 has a type and matches; ex:2 has no type and is excluded.
        assert_eq!(
            out,
            r#"{"@id":"http://ex/1","@type":"http://ex/T"}"#,
            "wildcard @type selects typed node only: {}",
            out
        );
    }

    /// `match_type` match-none `@type:[]` branch (~line 423): matches only nodes that
    /// have NO `@type`; typed nodes are excluded.
    #[test]
    fn type_match_none_selects_untyped_nodes() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T> .\
             <http://ex/2> <http://ex/p> \"X\" .",
            // @type:[] = match-none — only untyped nodes match.
            r#"{"@type":[]}"#,
        );
        // ex:2 has no type and matches; ex:1 is excluded.
        assert_eq!(
            out,
            r#"{"@id":"http://ex/2","http://ex/p":"X"}"#,
            "match-none @type selects untyped node only: {}",
            out
        );
    }

    /// `match_type` Json::Arr `wanted` branch (~line 431): an array of type IRIs in the
    /// frame matches a node with ANY of those types (OR semantics).
    #[test]
    fn type_array_frame_matches_either_type() {
        let out = frame_doc(
            "<http://ex/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T1> .\
             <http://ex/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T2> .\
             <http://ex/3> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/T3> .",
            // @type array: matches nodes typed T1 OR T2; T3 is excluded.
            r#"{"@type":["http://ex/T1","http://ex/T2"]}"#,
        );
        // ex:1 and ex:2 match; ex:3 does not.
        assert!(
            out.contains(r#""@id":"http://ex/1""#),
            "T1 node in result: {}",
            out
        );
        assert!(
            out.contains(r#""@id":"http://ex/2""#),
            "T2 node in result: {}",
            out
        );
        assert!(
            !out.contains(r#""@id":"http://ex/3""#),
            "T3 node excluded: {}",
            out
        );
    }
}

//! JSON-LD 1.1 **Compaction** of an RDF dataset against a caller-supplied `@context`.
//!
//! The writer is a thin adapter over the native document-level pipeline in
//! [`sparq_jsonld`] — the same algorithms the W3C `compact` conformance lane measures
//! (`crates/sparq-conformance/src/floors/compact.rs`):
//!
//! 1. **fromRdf** — the dataset's quads go through [`sparq_jsonld::from_rdf::from_rdf`]
//!    (`@list` collapse, `rdf:type` → `@type`, named graphs as `@graph` nodes). Canonical
//!    `xsd:integer` / `xsd:boolean` literals then become native JSON scalars, exactly as the
//!    expanded/flattened writer in the parent module emits them, so the coercion stays
//!    lossless (a non-canonical lexical such as `"007"` keeps its `@value` string).
//! 2. **Compaction** ([`sparq_jsonld::compact::compact_expanded`]) under default
//!    [`JsonLdOptions`] and the deny-by-default [`NoopLoader`] (a remote `@context` IRI is
//!    never fetched).
//!
//! The writer stays total: a `@context` the processor rejects (e.g. an invalid term
//! definition, a remote context) yields the lossless **expanded** document instead, so the
//! output always re-reads as the same RDF. Framing still runs the hand-rolled framer in
//! the sibling `frame` module (over `legacy_compact`) until it moves onto
//! [`sparq_jsonld::frame`] in its own change.
//!
//! An RDF 1.2 triple term has no JSON-LD encoding; it is carried as an opaque `@id` holding
//! its N-Triples spelling, the same choice the parent writer makes.

use super::{coerce_native, NamedGraph};
use oxrdf::{NamedOrBlankNode, Term, Triple};
use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
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

/// Parses a caller `@context` (or frame) from its JSON text. Returns `None` unless the text
/// is a single JSON object.
pub fn parse_context_json(text: &str) -> Option<Json> {
    Json::parse(text).ok().filter(Json::is_obj)
}

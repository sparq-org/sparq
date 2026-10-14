// AUTHORED-BY Claude Opus 4.8
//! RDF content-type handling for the LDP path.
//!
//! M1 supports the two RDF formats the production server allows: **Turtle** and **JSON-LD** (the
//! house rule — `oxttl` for Turtle, `oxjsonld` for JSON-LD, per the spike §4). This module classifies
//! a media type and validates that a body parses as RDF in that format, returning the parsed quad
//! count (proof the body is well-formed RDF) without retaining the graph — the slice stores bytes
//! verbatim and lets SPARQ be authoritative for the triples.
//!
//! M2: content negotiation (an `Accept`-driven serialisation choice) + re-serialisation between the
//! two RDF formats now land here ([`negotiate_accept`] + [`serialize_triples`]). The JSON-LD
//! `noRemoteContextLoader` SSRF posture (oxjsonld is local-only by construction) ports favourably.
//! The Solid read path is hardened via [`negotiate_accept_with_profile`]: q-values, the JSON-LD
//! `profile` parameter (expanded vs compacted, surfaced in [`NegotiatedFormat`]), and an `Accept`
//! naming no producible type degrading to `text/turtle` (the Solid default) instead of a 406.
//! An honoured profile is wired through the read handlers (sq-10ty4): the response `Content-Type`
//! echoes it ([`NegotiatedFormat::content_type`]) and [`serialize_triples_negotiated`] emits the
//! requested document form — `expanded` byte-identically (the serialiser's default output IS the
//! expanded form), `compacted` via a local context-free compaction step that keeps the SSRF
//! posture (nothing is ever fetched). Reads can also be served as N-Triples and N-Quads (a
//! default-graph serialisation, so the N-Quads body carries no graph labels); writes still accept
//! only Turtle and JSON-LD ([`classify`]). N3 is not offered: there is no N3 serialiser, and the
//! Turtle 1.2 output can carry triple terms that N3 cannot express, so an `Accept: text/n3` read
//! falls back to Turtle like any other unproducible type.

use std::collections::HashMap;

use oxjsonld::{JsonLdParser, JsonLdSerializer};
use oxrdf::{BlankNode, GraphNameRef, NamedOrBlankNode, QuadRef, Term, Triple};
use oxttl::{
    NQuadsParser, NQuadsSerializer, NTriplesParser, NTriplesSerializer, TurtleParser,
    TurtleSerializer,
};

use crate::error::ServerError;

/// A supported RDF media type.
///
/// Turtle and JSON-LD are the stored (writable) formats; N-Triples and N-Quads are read-only
/// representations that [`negotiate_accept_with_profile`] can choose and [`serialize_triples`] can
/// produce, but that [`classify`] never accepts for a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdfFormat {
    Turtle,
    JsonLd,
    NTriples,
    NQuads,
}

impl RdfFormat {
    /// Every format a read can be served in, in tie-break preference order: when two formats
    /// share the best `q` and neither is the stored format, the earlier one wins. Turtle and
    /// JSON-LD come first so `*/*` and `application/*` keep choosing what they always did.
    pub const READABLE: [RdfFormat; 4] = [
        RdfFormat::Turtle,
        RdfFormat::JsonLd,
        RdfFormat::NTriples,
        RdfFormat::NQuads,
    ];

    /// The canonical media type string.
    pub fn media_type(self) -> &'static str {
        match self {
            RdfFormat::Turtle => "text/turtle",
            RdfFormat::JsonLd => "application/ld+json",
            RdfFormat::NTriples => "application/n-triples",
            RdfFormat::NQuads => "application/n-quads",
        }
    }
}

/// Classify a `Content-Type` header value into a supported [`RdfFormat`].
///
/// The media-type is matched case-insensitively and any parameters (e.g. `; charset=utf-8`) are
/// ignored. An unsupported or absent type is an [`ServerError::UnsupportedMediaType`] — the LDP
/// surface accepts only RDF on this slice's single-resource PUT path.
pub fn classify(content_type: Option<&str>) -> Result<RdfFormat, ServerError> {
    let raw = content_type.ok_or_else(|| ServerError::UnsupportedMediaType("missing".into()))?;
    let essence = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/turtle" => Ok(RdfFormat::Turtle),
        "application/ld+json" => Ok(RdfFormat::JsonLd),
        other => Err(ServerError::UnsupportedMediaType(other.to_string())),
    }
}

/// Validate that `body` parses as RDF in `format`, returning the number of triples/quads parsed.
///
/// `base_iri` is the resource's own IRI: per the LDP/RDF convention a server resolves relative IRIs
/// in a submitted document against the request URI, so a document that uses relative IRIs is valid.
/// This proves the body is well-formed RDF before the slice stores it. The parsed graph is NOT
/// retained — SPARQ is the authoritative triple store; the blob store keeps the bytes verbatim.
pub fn validate_rdf(format: RdfFormat, body: &[u8], base_iri: &str) -> Result<usize, ServerError> {
    Ok(parse_to_triples(format, body, base_iri)?.len())
}

/// Relabel every blank node in `triples` `b0`, `b1`, … in order of first appearance (subject before
/// object, nested RDF 1.2 triple terms included).
///
/// A parser invents fresh labels for anonymous blank nodes (`[ … ]`, collections) on every parse, so
/// re-serialising the same stored bytes twice would otherwise give different bytes. A read that
/// re-serialises a stored document runs its triples through this first, so one stored state always
/// renders to one byte sequence under one strong validator. Only for output: relabelling two
/// separately parsed documents this way would make their blank nodes collide.
pub fn label_blank_nodes_by_position(triples: &mut [Triple]) {
    fn relabel(node: &mut BlankNode, labels: &mut HashMap<String, BlankNode>) {
        let next = labels.len();
        let label = labels
            .entry(node.as_str().to_owned())
            .or_insert_with(|| BlankNode::new_unchecked(format!("b{next}")));
        *node = label.clone();
    }
    fn visit(triple: &mut Triple, labels: &mut HashMap<String, BlankNode>) {
        if let NamedOrBlankNode::BlankNode(node) = &mut triple.subject {
            relabel(node, labels);
        }
        match &mut triple.object {
            Term::BlankNode(node) => relabel(node, labels),
            Term::Triple(inner) => visit(inner, labels),
            Term::NamedNode(_) | Term::Literal(_) => {}
        }
    }
    let mut labels = HashMap::new();
    for triple in triples {
        visit(triple, &mut labels);
    }
}

/// Parse `body` (in `format`) into its default-graph triples, resolving relative IRIs against
/// `base_iri`.
///
/// Both source formats are reduced to a flat `Vec<Triple>`: the Turtle parser already yields the
/// default graph; the JSON-LD parser yields quads, of which only the default graph is retained (a
/// Solid RDF *resource* is a single graph — named graphs in a submitted JSON-LD document are not
/// part of the resource's triples). This is the shared parse step behind both validation and content
/// negotiation, so an unparseable body is rejected (a 400) before any storage or re-serialisation.
pub fn parse_to_triples(
    format: RdfFormat,
    body: &[u8],
    base_iri: &str,
) -> Result<Vec<Triple>, ServerError> {
    match format {
        RdfFormat::Turtle => {
            let parser = TurtleParser::new()
                .with_base_iri(base_iri)
                .map_err(|e| ServerError::BadRequest(format!("invalid base IRI: {e}")))?;
            let mut triples = Vec::new();
            for triple in parser.for_slice(body) {
                let t =
                    triple.map_err(|e| ServerError::BadRequest(format!("invalid Turtle: {e}")))?;
                triples.push(t);
            }
            Ok(triples)
        }
        RdfFormat::JsonLd => {
            // oxjsonld is local-only by construction (no remote context loader) — the SSRF-safe
            // posture the production server enforces explicitly is the default here.
            let parser = JsonLdParser::new()
                .with_base_iri(base_iri)
                .map_err(|e| ServerError::BadRequest(format!("invalid base IRI: {e}")))?;
            let mut triples = Vec::new();
            for quad in parser.for_slice(body) {
                let q =
                    quad.map_err(|e| ServerError::BadRequest(format!("invalid JSON-LD: {e}")))?;
                // A resource is a single (default) graph; ignore any named-graph quads.
                if q.graph_name == oxrdf::GraphName::DefaultGraph {
                    triples.push(Triple::new(q.subject, q.predicate, q.object));
                }
            }
            Ok(triples)
        }
        RdfFormat::NTriples => {
            // N-Triples has no relative IRIs, so there is no base to resolve against.
            let mut triples = Vec::new();
            for triple in NTriplesParser::new().for_slice(body) {
                let t = triple
                    .map_err(|e| ServerError::BadRequest(format!("invalid N-Triples: {e}")))?;
                triples.push(t);
            }
            Ok(triples)
        }
        RdfFormat::NQuads => {
            let mut triples = Vec::new();
            for quad in NQuadsParser::new().for_slice(body) {
                let q =
                    quad.map_err(|e| ServerError::BadRequest(format!("invalid N-Quads: {e}")))?;
                // A resource is a single (default) graph; ignore any named-graph quads.
                if q.graph_name == oxrdf::GraphName::DefaultGraph {
                    triples.push(Triple::new(q.subject, q.predicate, q.object));
                }
            }
            Ok(triples)
        }
    }
}

/// Serialise a triple set into `format`, returning the bytes.
///
/// The serialisation is unconditioned (no base-IRI abbreviation) so the output is self-contained and
/// stable. Used by content negotiation on read (re-render the stored Turtle as JSON-LD or vice
/// versa) and after a PATCH (re-serialise the patched graph for storage).
pub fn serialize_triples(format: RdfFormat, triples: &[Triple]) -> Result<Vec<u8>, ServerError> {
    match format {
        RdfFormat::Turtle => {
            let mut ser = TurtleSerializer::new().for_writer(Vec::new());
            for t in triples {
                ser.serialize_triple(t)
                    .map_err(|e| ServerError::Storage(format!("turtle serialise: {e}")))?;
            }
            ser.finish()
                .map_err(|e| ServerError::Storage(format!("turtle serialise: {e}")))
        }
        RdfFormat::JsonLd => {
            let mut ser = JsonLdSerializer::new().for_writer(Vec::new());
            for t in triples {
                let q = QuadRef::new(
                    t.subject.as_ref(),
                    t.predicate.as_ref(),
                    t.object.as_ref(),
                    GraphNameRef::DefaultGraph,
                );
                ser.serialize_quad(q)
                    .map_err(|e| ServerError::Storage(format!("json-ld serialise: {e}")))?;
            }
            ser.finish()
                .map_err(|e| ServerError::Storage(format!("json-ld serialise: {e}")))
        }
        RdfFormat::NTriples => {
            let mut ser = NTriplesSerializer::new().for_writer(Vec::new());
            for t in triples {
                ser.serialize_triple(t)
                    .map_err(|e| ServerError::Storage(format!("n-triples serialise: {e}")))?;
            }
            Ok(ser.finish())
        }
        RdfFormat::NQuads => {
            // A resource is one default graph, so every line is a triple with no graph label.
            let mut ser = NQuadsSerializer::new().for_writer(Vec::new());
            for t in triples {
                let q = QuadRef::new(
                    t.subject.as_ref(),
                    t.predicate.as_ref(),
                    t.object.as_ref(),
                    GraphNameRef::DefaultGraph,
                );
                ser.serialize_quad(q)
                    .map_err(|e| ServerError::Storage(format!("n-quads serialise: {e}")))?;
            }
            Ok(ser.finish())
        }
    }
}

/// Serialise a triple set into the negotiated format AND document form, returning the bytes.
///
/// Same as [`serialize_triples`] for Turtle and for JSON-LD with no honoured profile — the
/// serialiser's default output (a top-level array of node objects, full IRIs, no `@context`) is
/// ALREADY the [expanded document form](https://www.w3.org/TR/json-ld11/#expanded-document-form),
/// so the `expanded` profile is honoured byte-identically. The `compacted` profile applies
/// `compact_jsonld`, the local (context-free) compaction step — no context is used or fetched,
/// preserving the local-only no-remote-context SSRF posture.
pub fn serialize_triples_negotiated(
    negotiated: NegotiatedFormat,
    triples: &[Triple],
) -> Result<Vec<u8>, ServerError> {
    let bytes = serialize_triples(negotiated.format, triples)?;
    match (negotiated.format, negotiated.jsonld_profile) {
        (RdfFormat::JsonLd, Some(JsonLdProfileParam::Compacted)) => compact_jsonld(&bytes),
        _ => Ok(bytes),
    }
}

/// Compact the serialiser's own expanded JSON-LD output with an EMPTY context — the JSON-LD 1.1
/// compaction of the document against no context, scoped to the shape [`serialize_triples`]
/// produces (a top-level array of flat node objects whose non-`@id` values are arrays of node
/// references / value objects; no `@list`, no nesting, no named graphs — the serialiser emits
/// none of those for a default-graph triple set):
///
/// - a single-element property array collapses to its (compacted) single value;
/// - a value object holding ONLY `@value` collapses to the bare scalar (there is no default
///   language, so a plain string never needs its object kept); one carrying `@type` / `@language`
///   (/ `@direction`) keeps its object — those entries must survive;
/// - a node reference (`{"@id": …}`) stays an object (an empty context maps no term to `@id`);
/// - a single top-level node object becomes the document itself, an empty document becomes `{}`,
///   and multiple node objects wrap in `{"@graph": […]}` — the `JsonLdProcessor.compact` API's
///   top-level rule.
///
/// Purely local structural rewriting over our own serialiser's bytes: no context document exists,
/// so nothing can be fetched (the SSRF posture is preserved by construction). Key order follows
/// `serde_json`'s deterministic map, so equal inputs give equal bytes (the ETag contract).
fn compact_jsonld(bytes: &[u8]) -> Result<Vec<u8>, ServerError> {
    let doc: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ServerError::Storage(format!("json-ld compaction parse: {e}")))?;
    // The serialiser's context-free output is always a top-level array; anything else means the
    // input was not ours — fail loudly rather than mislabel a non-compacted body.
    let serde_json::Value::Array(nodes) = doc else {
        return Err(ServerError::Storage(
            "json-ld compaction: unexpected serialiser output shape".into(),
        ));
    };
    let mut compacted: Vec<serde_json::Value> = nodes.into_iter().map(compact_node).collect();
    let out = match compacted.len() {
        0 => serde_json::Value::Object(serde_json::Map::new()),
        1 => compacted.pop().unwrap_or_default(),
        _ => {
            let mut map = serde_json::Map::with_capacity(1);
            map.insert("@graph".into(), serde_json::Value::Array(compacted));
            serde_json::Value::Object(map)
        }
    };
    serde_json::to_vec(&out)
        .map_err(|e| ServerError::Storage(format!("json-ld compaction serialise: {e}")))
}

/// Compact one node object: every entry except `@id` (whose value is a plain string) is a
/// property whose array value compacts per the rules on [`compact_jsonld`].
fn compact_node(node: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = node else {
        // Not a node object — never produced by our serialiser; pass through unchanged.
        return node;
    };
    let compacted = map
        .into_iter()
        .map(|(key, value)| {
            let value = if key == "@id" {
                value
            } else {
                compact_property_values(value)
            };
            (key, value)
        })
        .collect();
    serde_json::Value::Object(compacted)
}

/// Compact a property's expanded value array: each member value-compacts (a lone-`@value` object
/// to its bare scalar), and a single-member array collapses to that member (`compactArrays`).
fn compact_property_values(value: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Array(members) = value else {
        return compact_value(value);
    };
    let mut members: Vec<serde_json::Value> = members.into_iter().map(compact_value).collect();
    if members.len() == 1 {
        members.pop().unwrap_or_default()
    } else {
        serde_json::Value::Array(members)
    }
}

/// Value-compact one expanded object: a value object holding ONLY `@value` becomes the bare
/// scalar; everything else (a `@type`/`@language`-carrying value object, a node reference) is
/// kept as-is.
fn compact_value(value: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = value else {
        return value;
    };
    if map.len() == 1 && map.contains_key("@value") {
        return map.into_iter().next().map(|(_, v)| v).unwrap_or_default();
    }
    serde_json::Value::Object(map)
}

/// A JSON-LD document form requested via the `Accept` header's `profile` media-type parameter
/// ([JSON-LD 1.1 IANA registration](https://www.w3.org/TR/json-ld11/#iana-considerations)).
///
/// Only the two READ-relevant document forms are honoured: `expanded` and `compacted`. Other
/// registered profile IRIs (`flattened`, `framed`, …) are ignored — per the registration a profile
/// parameter is a client *preference* a server may decline, never an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLdProfileParam {
    /// `http://www.w3.org/ns/json-ld#expanded`
    Expanded,
    /// `http://www.w3.org/ns/json-ld#compacted`
    Compacted,
}

impl JsonLdProfileParam {
    /// The canonical profile IRI (the JSON-LD 1.1 IANA registration), via
    /// [`oxjsonld::JsonLdProfile::iri`] so the spelling stays pinned to the vendored JSON-LD
    /// implementation. Used to echo the honoured profile back in the response `Content-Type`.
    pub fn iri(self) -> &'static str {
        match self {
            Self::Expanded => oxjsonld::JsonLdProfile::Expanded.iri(),
            Self::Compacted => oxjsonld::JsonLdProfile::Compacted.iri(),
        }
    }

    /// Extract the first honoured document form from a `profile` parameter VALUE (the part after
    /// `profile=`), deterministically.
    ///
    /// The value is a (usually quoted) whitespace-separated list of profile IRIs; the FIRST one
    /// that names an honoured form wins (client order = client preference). Unknown IRIs are
    /// skipped. IRIs match the canonical registration exactly (IRIs are case-sensitive) — matched
    /// via [`oxjsonld::JsonLdProfile::from_iri`] so the accepted spellings stay pinned to the
    /// vendored JSON-LD implementation, not a hand-copied string.
    fn from_param_value(value: &str) -> Option<Self> {
        value
            .trim()
            .trim_matches('"')
            .split_ascii_whitespace()
            .find_map(|iri| match oxjsonld::JsonLdProfile::from_iri(iri) {
                Some(oxjsonld::JsonLdProfile::Expanded) => Some(Self::Expanded),
                Some(oxjsonld::JsonLdProfile::Compacted) => Some(Self::Compacted),
                _ => None,
            })
    }
}

/// The outcome of [`negotiate_accept_with_profile`]: the chosen response format, plus — when that
/// format is JSON-LD and the winning explicit `application/ld+json` range carried a recognised
/// `profile` parameter — the requested document form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedFormat {
    /// The RDF format the response body should be serialised in.
    pub format: RdfFormat,
    /// The JSON-LD document form the client asked for, when [`Self::format`] is
    /// [`RdfFormat::JsonLd`] and an explicit `application/ld+json` range carried one. `None`
    /// otherwise (including a JSON-LD choice reached via an `application/*` / `*/*` wildcard —
    /// wildcard ranges carry no honoured profile). An honoured profile is echoed back in the
    /// response `Content-Type` ([`Self::content_type`]) and drives the document form
    /// ([`serialize_triples_negotiated`]): the serialiser's default output IS the expanded form,
    /// and `compacted` applies the local (context-free, nothing-fetched) compaction step.
    pub jsonld_profile: Option<JsonLdProfileParam>,
}

impl NegotiatedFormat {
    /// The response `Content-Type` value: the negotiated media type, with the honoured JSON-LD
    /// `profile` parameter echoed back (quoted, the canonical IRI) per the JSON-LD 1.1 IANA
    /// registration — so a client can see which document form the server actually honoured.
    pub fn content_type(&self) -> String {
        match self.jsonld_profile {
            Some(profile) => format!("{};profile=\"{}\"", self.format.media_type(), profile.iri()),
            None => self.format.media_type().to_string(),
        }
    }

    /// Whether the stored bytes of a resource stored in `stored` format ARE this negotiation's
    /// representation (the verbatim fast path): same format AND no honoured JSON-LD profile. Any
    /// honoured profile forces a re-serialisation even for a stored-JSON-LD resource — the stored
    /// bytes are whatever document form the client originally wrote, so serving them under a
    /// `profile`-carrying `Content-Type` would be dishonest. The single shared rule keeps the body
    /// path ([`serialize_triples_negotiated`] callers) and the validator path in agreement.
    pub fn serves_stored_verbatim(&self, stored: RdfFormat) -> bool {
        self.format == stored && self.jsonld_profile.is_none()
    }

    /// The short, `+`-free `+<variant>` ETag suffix token for this negotiated representation
    /// (consumed by `conditional::variant_etag`; shared by the LDP and identity read paths so a
    /// variant tag is derived identically on both surfaces).
    ///
    /// The suffix is profile-variant-specific exactly when the BYTES differ: `compacted` output
    /// differs from the default serialisation ⇒ a distinct token, while `expanded` output is
    /// byte-identical to it (the serialiser's default output IS the expanded document form —
    /// [`serialize_triples_negotiated`]) ⇒ the same token as plain JSON-LD.
    pub fn variant_suffix(&self) -> &'static str {
        match (self.format, self.jsonld_profile) {
            (RdfFormat::Turtle, _) => "ttl",
            (RdfFormat::JsonLd, Some(JsonLdProfileParam::Compacted)) => "jsonld-c",
            (RdfFormat::JsonLd, _) => "jsonld",
            (RdfFormat::NTriples, _) => "nt",
            (RdfFormat::NQuads, _) => "nq",
        }
    }
}

/// Negotiate the response RDF format from an `Accept` header against the formats this server can
/// produce ([`RdfFormat::READABLE`]). Thin wrapper over [`negotiate_accept_with_profile`] (the single
/// `Accept` decision point) that drops the JSON-LD profile detail.
pub fn negotiate_accept(accept: Option<&str>, stored: RdfFormat) -> Option<RdfFormat> {
    negotiate_accept_with_profile(accept, stored).map(|n| n.format)
}

/// Negotiate the response RDF format (and JSON-LD document form) from an `Accept` header against
/// the formats this server can produce ([`RdfFormat::READABLE`]: Turtle, JSON-LD, N-Triples, N-Quads).
///
/// - An ABSENT `Accept` means "no preference": the resource's stored format (`stored`) — the most
///   faithful, zero-cost response (as does `*/*`, which covers every producible type).
/// - Quality values (`q=`) are honoured: the highest-q acceptable type wins; a q-tie prefers the
///   stored format (cheapest), then the [`RdfFormat::READABLE`] order (so `application/*` still
///   means JSON-LD).
/// - The `application/ld+json` `profile` parameter is honoured deterministically: the profile of
///   the best-q explicit `ld+json` range is surfaced (first-listed range wins q-ties), reduced to
///   the first honoured document form in its value.
/// - A PRESENT header that names/covers NO producible type (blank, or only unrecognised types like
///   `text/html`) falls back to `text/turtle` — the Solid default representation — rather than
///   failing the read (never a 406, never a 500).
/// - `None` (⇒ the caller's 406) is returned ONLY when the client EXPLICITLY refused every
///   producible type it covered (all applicable ranges at `q=0`) — serving a representation the
///   client q=0-refused would violate RFC 7231 §5.3.1.
///
/// This is a deliberately small, dependency-free `Accept` parser sufficient for the RDF media types
/// the server serves; it is NOT a general RFC 7231 content-negotiation engine (in
/// particular, parameters are split on `;`, so a quoted parameter value containing `;` — which no
/// registered profile IRI does — would mis-parse).
pub fn negotiate_accept_with_profile(
    accept: Option<&str>,
    stored: RdfFormat,
) -> Option<NegotiatedFormat> {
    // ABSENT header — "no preference": the stored format, byte-identical to the stored bytes.
    let Some(raw) = accept else {
        return Some(NegotiatedFormat {
            format: stored,
            jsonld_profile: None,
        });
    };

    // Track the best q for each producible type (indexed like `RdfFormat::READABLE`), plus the
    // type-range wildcards. A `text/*` range covers only Turtle; an `application/*` range covers
    // JSON-LD, N-Triples and N-Quads; `*/*` covers all four. Each is kept separately so a `text/*`
    // request never yields an `application/*` type (the bug roborev flagged).
    let mut q_explicit: [Option<f32>; RdfFormat::READABLE.len()] =
        [None; RdfFormat::READABLE.len()];
    let mut q_text_star: Option<f32> = None;
    let mut q_app_star: Option<f32> = None;
    let mut q_any: Option<f32> = None;

    // The profile carried by the BEST explicit `application/ld+json` range seen so far. Updated
    // only on a STRICTLY greater q, so the first-listed range wins q-ties — deterministic, and
    // consistent with `bump`, under which a later equal-q range changes nothing.
    let mut jsonld_profile: Option<JsonLdProfileParam> = None;

    fn bump(slot: &mut Option<f32>, q: f32) {
        *slot = Some(slot.unwrap_or(0.0).max(q));
    }

    for part in raw.split(',') {
        let mut it = part.split(';');
        let media = it.next().unwrap_or("").trim().to_ascii_lowercase();
        // Parse the parameters: an optional q-value (default 1.0; clamp to [0,1]; a malformed q is
        // treated as 0 — RFC 7231 §5.3.1, an unparseable weight is not "accepted") and an optional
        // `profile` (media-type parameter names are case-insensitive; the IRI value is not).
        let mut q: f32 = 1.0;
        let mut profile: Option<JsonLdProfileParam> = None;
        for param in it {
            let Some((name, value)) = param.split_once('=') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "q" => q = value.trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0),
                "profile" => profile = JsonLdProfileParam::from_param_value(value),
                _ => {}
            }
        }
        match media.as_str() {
            "text/*" => bump(&mut q_text_star, q),
            "application/*" => bump(&mut q_app_star, q),
            "*/*" => bump(&mut q_any, q),
            other => {
                let Some(i) = RdfFormat::READABLE
                    .iter()
                    .position(|f| f.media_type() == other)
                else {
                    continue;
                };
                if RdfFormat::READABLE[i] == RdfFormat::JsonLd
                    && q_explicit[i].is_none_or(|cur| q > cur)
                {
                    jsonld_profile = profile;
                }
                bump(&mut q_explicit[i], q);
            }
        }
    }

    // A header that named/covered NO producible type — blank, or only unrecognised media types
    // (`text/html`, `application/xml`, …). Solid default: degrade to Turtle, don't fail the read.
    if q_explicit.iter().all(Option::is_none)
        && q_text_star.is_none()
        && q_app_star.is_none()
        && q_any.is_none()
    {
        return Some(NegotiatedFormat {
            format: RdfFormat::Turtle,
            jsonld_profile: None,
        });
    }

    // Resolve each concrete type's effective weight: an explicit q wins; else the most specific
    // applicable wildcard (`type/*`), else `*/*`. A type with no applicable range is not accepted.
    let weight = |i: usize| {
        let type_star = match RdfFormat::READABLE[i] {
            RdfFormat::Turtle => q_text_star,
            RdfFormat::JsonLd | RdfFormat::NTriples | RdfFormat::NQuads => q_app_star,
        };
        q_explicit[i].or(type_star).or(q_any).unwrap_or(0.0)
    };
    let weights: [f32; RdfFormat::READABLE.len()] = std::array::from_fn(weight);
    let best = weights.iter().copied().fold(0.0_f32, f32::max);
    if best <= 0.0 {
        return None; // 406 — the client EXPLICITLY refused (q=0) every covered producible type.
    }
    // Highest q wins; on a tie prefer the resource's stored format (cheapest, most faithful), then
    // the `READABLE` order.
    let stored_index = RdfFormat::READABLE.iter().position(|f| *f == stored);
    let index = match stored_index {
        Some(i) if weights[i] >= best => i,
        _ => weights.iter().position(|w| *w >= best).unwrap_or(0),
    };
    let format = RdfFormat::READABLE[index];
    Some(NegotiatedFormat {
        format,
        // The profile is only meaningful for a JSON-LD response, and only when JSON-LD won via an
        // explicit `application/ld+json` range (a wildcard win carries no honoured profile).
        jsonld_profile: match format {
            RdfFormat::JsonLd => jsonld_profile,
            RdfFormat::Turtle | RdfFormat::NTriples | RdfFormat::NQuads => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IRI: &str = "https://pod.example/alice/data";
    const TURTLE: &str =
        "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

    #[test]
    fn absent_or_wildcard_accept_keeps_stored_format() {
        assert_eq!(
            negotiate_accept(None, RdfFormat::Turtle),
            Some(RdfFormat::Turtle)
        );
        assert_eq!(
            negotiate_accept(None, RdfFormat::JsonLd),
            Some(RdfFormat::JsonLd)
        );
        assert_eq!(
            negotiate_accept(Some("*/*"), RdfFormat::Turtle),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn blank_accept_falls_back_to_turtle_the_solid_default() {
        // A PRESENT-but-blank Accept is treated as naming no producible type: the Solid default
        // (text/turtle), even when the stored format is JSON-LD.
        assert_eq!(
            negotiate_accept(Some(""), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
        assert_eq!(
            negotiate_accept(Some("   "), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn explicit_jsonld_wins_over_stored_turtle() {
        assert_eq!(
            negotiate_accept(Some("application/ld+json"), RdfFormat::Turtle),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn explicit_turtle_wins_over_stored_jsonld() {
        assert_eq!(
            negotiate_accept(Some("text/turtle"), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn q_values_are_honoured() {
        // JSON-LD preferred by weight even though Turtle is listed first.
        assert_eq!(
            negotiate_accept(
                Some("text/turtle;q=0.5, application/ld+json;q=0.9"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn q_zero_excludes_a_type() {
        // Turtle explicitly refused (q=0); JSON-LD acceptable ⇒ JSON-LD.
        assert_eq!(
            negotiate_accept(
                Some("text/turtle;q=0, application/ld+json"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn unrecognised_accept_falls_back_to_turtle_not_406() {
        // An Accept naming only types the server can't produce degrades to the Solid default
        // (text/turtle) — the read never fails over an exotic Accept.
        assert_eq!(
            negotiate_accept(Some("application/xml"), RdfFormat::Turtle),
            Some(RdfFormat::Turtle)
        );
        assert_eq!(
            negotiate_accept(Some("text/html"), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn explicit_q_zero_refusal_of_every_covered_type_is_none_406() {
        // The ONLY remaining 406: the client covered our producible types and explicitly refused
        // them all with q=0 — serving one anyway would violate the client's stated refusal.
        assert_eq!(
            negotiate_accept(
                Some("text/turtle;q=0, application/ld+json;q=0"),
                RdfFormat::Turtle
            ),
            None
        );
        assert_eq!(negotiate_accept(Some("*/*;q=0"), RdfFormat::JsonLd), None);
    }

    #[test]
    fn text_star_covers_only_turtle() {
        // `text/*` maps to Turtle, never JSON-LD — even when the stored format is JSON-LD.
        assert_eq!(
            negotiate_accept(Some("text/*"), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
        assert_eq!(
            negotiate_accept(Some("text/*"), RdfFormat::Turtle),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn application_star_prefers_jsonld_and_never_yields_turtle() {
        // `application/*` maps to JSON-LD, never Turtle — even when the stored format is Turtle.
        // It also covers N-Triples and N-Quads, but JSON-LD comes first in `READABLE`.
        assert_eq!(
            negotiate_accept(Some("application/*"), RdfFormat::Turtle),
            Some(RdfFormat::JsonLd)
        );
        assert_eq!(
            negotiate_accept(Some("application/*"), RdfFormat::JsonLd),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn any_wildcard_covers_both_and_keeps_stored() {
        assert_eq!(
            negotiate_accept(Some("*/*"), RdfFormat::Turtle),
            Some(RdfFormat::Turtle)
        );
        assert_eq!(
            negotiate_accept(Some("*/*"), RdfFormat::JsonLd),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn explicit_beats_wildcard() {
        // An explicit application/ld+json at higher q wins over a text/* range.
        assert_eq!(
            negotiate_accept(
                Some("text/*;q=0.3, application/ld+json;q=0.9"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::JsonLd)
        );
    }

    #[test]
    fn explicit_ntriples_and_nquads_are_negotiated() {
        for stored in [RdfFormat::Turtle, RdfFormat::JsonLd] {
            assert_eq!(
                negotiate_accept(Some("application/n-triples"), stored),
                Some(RdfFormat::NTriples)
            );
            assert_eq!(
                negotiate_accept(Some("Application/N-Quads; charset=utf-8"), stored),
                Some(RdfFormat::NQuads)
            );
        }
        // q-values rank them like any other type.
        assert_eq!(
            negotiate_accept(
                Some("text/turtle;q=0.5, application/n-quads;q=0.8, application/n-triples;q=0.7"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::NQuads)
        );
        // Equal q with no stored-format candidate: the READABLE order decides.
        assert_eq!(
            negotiate_accept(
                Some("application/n-quads, application/n-triples"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::NTriples)
        );
        // `application/*` reaches N-Triples once JSON-LD is refused, instead of a 406.
        assert_eq!(
            negotiate_accept(
                Some("application/*, application/ld+json;q=0"),
                RdfFormat::Turtle
            ),
            Some(RdfFormat::NTriples)
        );
        // `text/*` still covers only Turtle.
        assert_eq!(
            negotiate_accept(Some("text/*, application/n-triples;q=0"), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
        // Refusing everything a header covers is still a 406.
        assert_eq!(
            negotiate_accept(
                Some("application/n-triples;q=0, application/n-quads;q=0"),
                RdfFormat::Turtle
            ),
            None
        );
    }

    #[test]
    fn n3_is_not_offered_and_falls_back_to_turtle() {
        assert_eq!(
            negotiate_accept(Some("text/n3"), RdfFormat::JsonLd),
            Some(RdfFormat::Turtle)
        );
    }

    #[test]
    fn classify_does_not_accept_the_read_only_formats() {
        for media in ["application/n-triples", "application/n-quads", "text/n3"] {
            assert!(
                matches!(
                    classify(Some(media)),
                    Err(ServerError::UnsupportedMediaType(_))
                ),
                "{media}"
            );
        }
    }

    #[test]
    fn ntriples_and_nquads_round_trip_the_triples() {
        const DOC: &str = "@prefix ex: <https://pod.example/ns#> .\n\
            <https://pod.example/alice/data#me> ex:name \"Alice\"@en ; ex:knows _:b ; \
            ex:age 42 ; ex:says <<( <https://pod.example/s> ex:p ex:o )>> .";
        let triples = parse_to_triples(RdfFormat::Turtle, DOC.as_bytes(), IRI).unwrap();
        let canonical = |t: &[Triple]| {
            let mut ts: Vec<String> = t.iter().map(ToString::to_string).collect();
            ts.sort();
            ts
        };
        for format in [RdfFormat::NTriples, RdfFormat::NQuads] {
            let bytes = serialize_triples(format, &triples).unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert_eq!(text.lines().count(), triples.len(), "{format:?}: {text}");
            let back = parse_to_triples(format, &bytes, IRI).unwrap();
            // Blank-node labels are fresh per serialisation, so compare with them masked.
            let mask = |v: Vec<String>| -> Vec<String> {
                v.into_iter()
                    .map(|l| {
                        l.split(' ')
                            .map(|w| if w.starts_with("_:") { "_:" } else { w })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            };
            assert_eq!(
                mask(canonical(&back)),
                mask(canonical(&triples)),
                "{format:?}"
            );
        }
        // N-Quads parsing keeps only the default graph, like JSON-LD.
        let nq = "<https://pod.example/a> <https://pod.example/p> \"d\" .\n\
            <https://pod.example/a> <https://pod.example/p> \"n\" <https://pod.example/g> .\n";
        assert_eq!(
            parse_to_triples(RdfFormat::NQuads, nq.as_bytes(), IRI)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn every_readable_format_has_a_distinct_variant_suffix() {
        let suffixes: Vec<&str> = RdfFormat::READABLE
            .iter()
            .map(|&format| {
                NegotiatedFormat {
                    format,
                    jsonld_profile: None,
                }
                .variant_suffix()
            })
            .chain(std::iter::once(
                jsonld_with_profile("http://www.w3.org/ns/json-ld#compacted").variant_suffix(),
            ))
            .collect();
        let mut unique = suffixes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), suffixes.len(), "{suffixes:?}");
    }

    #[test]
    fn turtle_round_trips_through_jsonld_and_back() {
        // Parse Turtle → serialise JSON-LD → parse JSON-LD → same single triple.
        let triples = parse_to_triples(RdfFormat::Turtle, TURTLE.as_bytes(), IRI).unwrap();
        assert_eq!(triples.len(), 1);
        let jsonld = serialize_triples(RdfFormat::JsonLd, &triples).unwrap();
        let reparsed = parse_to_triples(RdfFormat::JsonLd, &jsonld, IRI).unwrap();
        assert_eq!(reparsed, triples);
    }

    /// The negotiated outcome for an explicit `application/ld+json` range carrying `profile`.
    fn jsonld_with_profile(profile_iri: &str) -> NegotiatedFormat {
        negotiate_accept_with_profile(
            Some(&format!("application/ld+json;profile=\"{profile_iri}\"")),
            RdfFormat::Turtle,
        )
        .expect("acceptable")
    }

    #[test]
    fn profile_iris_come_from_the_vendored_implementation() {
        assert_eq!(
            JsonLdProfileParam::Expanded.iri(),
            "http://www.w3.org/ns/json-ld#expanded"
        );
        assert_eq!(
            JsonLdProfileParam::Compacted.iri(),
            "http://www.w3.org/ns/json-ld#compacted"
        );
    }

    #[test]
    fn serves_stored_verbatim_only_without_an_honoured_profile() {
        let plain = negotiate_accept_with_profile(Some("application/ld+json"), RdfFormat::JsonLd)
            .expect("acceptable");
        assert!(plain.serves_stored_verbatim(RdfFormat::JsonLd));
        assert!(!plain.serves_stored_verbatim(RdfFormat::Turtle));
        // An honoured profile always re-serialises, even from a stored-JSON-LD resource.
        let compacted = jsonld_with_profile("http://www.w3.org/ns/json-ld#compacted");
        assert!(!compacted.serves_stored_verbatim(RdfFormat::JsonLd));
        let expanded = jsonld_with_profile("http://www.w3.org/ns/json-ld#expanded");
        assert!(!expanded.serves_stored_verbatim(RdfFormat::JsonLd));
    }

    #[test]
    fn compacting_an_empty_document_yields_the_empty_object() {
        let compacted = jsonld_with_profile("http://www.w3.org/ns/json-ld#compacted");
        let bytes = serialize_triples_negotiated(compacted, &[]).unwrap();
        assert_eq!(bytes, b"{}");
    }

    #[test]
    fn compaction_rejects_a_non_array_document_shape() {
        // Defensive: the serialiser's context-free output is always a top-level array; anything
        // else must fail loudly rather than be mislabelled as compacted.
        assert!(compact_jsonld(b"{\"@graph\":[]}").is_err());
        assert!(compact_jsonld(b"not json").is_err());
    }

    #[test]
    fn serialise_to_turtle_is_reparseable() {
        let triples = parse_to_triples(RdfFormat::Turtle, TURTLE.as_bytes(), IRI).unwrap();
        let ttl = serialize_triples(RdfFormat::Turtle, &triples).unwrap();
        let reparsed = parse_to_triples(RdfFormat::Turtle, &ttl, IRI).unwrap();
        assert_eq!(reparsed, triples);
    }

    #[test]
    fn blank_nodes_are_labelled_by_first_appearance() {
        const DOC: &str =
            "<http://a> <http://p> [ <http://q> [ <http://r> 1 ] ] . _:x <http://p> _:x .";
        let parse = || parse_to_triples(RdfFormat::Turtle, DOC.as_bytes(), IRI).unwrap();
        let (mut one, mut two) = (parse(), parse());
        label_blank_nodes_by_position(&mut one);
        label_blank_nodes_by_position(&mut two);
        assert_eq!(one, two, "two parses label alike");
        let text =
            String::from_utf8(serialize_triples(RdfFormat::NTriples, &one).unwrap()).unwrap();
        for label in ["_:b0", "_:b1", "_:b2"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // Distinct blank nodes stay distinct, and one node keeps one label.
        assert!(!text.contains("_:b3"));
        assert!(text.contains("_:b2 <http://p> _:b2 ."), "{text}");
    }
}

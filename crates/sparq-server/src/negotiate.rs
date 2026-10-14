//! `Accept`-header content negotiation for SPARQL results.
//!
//! Maps an HTTP `Accept` header to the result format the server will produce. We support
//! the four W3C result serialisations for SELECT (JSON/XML/CSV/TSV) and the boolean
//! serialisations for ASK (JSON/XML). The parsing is a pragmatic q-value-aware scan: it
//! collects acceptable media ranges with their q-values and picks the highest-q supported
//! format, defaulting to JSON (the engine's native, fastest path) when nothing matches or
//! the header is absent.

/// The negotiated SELECT/ASK result format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Xml,
    Csv,
    Tsv,
}

impl Format {
    /// The `Content-Type` for a SELECT result in this format (charset annotated where the
    /// W3C registration specifies one).
    pub fn select_content_type(self) -> &'static str {
        match self {
            Format::Json => "application/sparql-results+json",
            Format::Xml => "application/sparql-results+xml",
            Format::Csv => "text/csv; charset=utf-8",
            Format::Tsv => "text/tab-separated-values; charset=utf-8",
        }
    }

    /// The `Content-Type` for an ASK boolean. CSV/TSV have no boolean form, so they fall
    /// back to JSON per common practice.
    pub fn ask_content_type(self) -> &'static str {
        match self {
            Format::Xml => "application/sparql-results+xml",
            _ => "application/sparql-results+json",
        }
    }
}

/// The negotiated RDF graph serialisation for CONSTRUCT / DESCRIBE + Graph Store reads.
/// `Turtle` is now a real prefix-compacting Turtle document, and `RdfXml` is a genuine
/// RDF/XML document (sq-rt6v) — distinct serialisations, not all-N-Triples.
/// Negotiation picks the format (and its `Content-Type`) from the `Accept` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphFormat {
    NTriples,
    Turtle,
    /// `application/rdf+xml` — RDF/XML. sq-rt6v.
    RdfXml,
    /// `application/ld+json` — JSON-LD 1.1 (the engine's flattened serialisation).
    /// sq-oy1f.1. OPT-IN: only a participant in negotiation when the server is
    /// built with the `jsonld` feature (which links the engine's `serialize-rdf` writer);
    /// without it the variant does not exist, so `application/ld+json` is unrecognised and
    /// negotiation never selects it (it falls through to the default, exactly like any other
    /// unsupported media type).
    #[cfg(feature = "jsonld")]
    JsonLd,
}

impl GraphFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            GraphFormat::NTriples => "application/n-triples; charset=utf-8",
            GraphFormat::Turtle => "text/turtle; charset=utf-8",
            // sq-rt6v: RDF/XML registers `charset=utf-8` like the others; the
            // document's own XML declaration also pins UTF-8.
            GraphFormat::RdfXml => "application/rdf+xml; charset=utf-8",
            // sq-oy1f.1: JSON-LD is UTF-8 JSON; the IANA `application/ld+json`
            // registration carries no charset parameter (JSON is UTF-8 by RFC 8259), but we
            // annotate it like the others so a strict client never mis-decodes.
            #[cfg(feature = "jsonld")]
            GraphFormat::JsonLd => "application/ld+json; charset=utf-8",
        }
    }
}

/// Negotiates the CONSTRUCT/DESCRIBE + GSP-read graph serialisation from an `Accept` header:
/// `text/turtle`, `application/n-triples` and — sq-rt6v — `application/rdf+xml`
/// (q-value aware), default N-Triples.
pub fn negotiate_graph(accept: Option<&str>) -> GraphFormat {
    negotiate_graph_or_406(accept).unwrap_or(GraphFormat::NTriples)
}

/// Negotiates the CONSTRUCT/DESCRIBE + GSP-read graph serialisation with Oxigraph-parity
/// strictness (sq-406acc) — the RDF-graph analogue of [`negotiate_or_406`].
///
/// Returns `Ok(format)` when a supported RDF media type (or a wildcard) is named, or when the
/// `Accept` header is ABSENT / empty (the permitted N-Triples default); returns
/// `Err(`[`NotAcceptable`]`)` when the header is PRESENT and non-empty but names no supported
/// RDF media type and no wildcard, which the HTTP layer maps to `406 Not Acceptable`.
pub fn negotiate_graph_or_406(accept: Option<&str>) -> Result<GraphFormat, NotAcceptable> {
    let accept = match accept {
        // Absent or empty `Accept` → the N-Triples default is permitted (never a 406).
        Some(a) if !a.trim().is_empty() => a,
        _ => return Ok(GraphFormat::NTriples),
    };
    let mut best: Option<(GraphFormat, f32, usize)> = None;
    for (media, q) in media_ranges(accept) {
        let (fmt, spec) = match media.as_str() {
            "application/n-triples" => (Some(GraphFormat::NTriples), 2),
            "text/turtle" => (Some(GraphFormat::Turtle), 2),
            // sq-rt6v: RDF/XML (and the older `application/xml`/`text/xml` aliases
            // some clients send for RDF/XML — kept lower-specificity so an exact match wins).
            "application/rdf+xml" => (Some(GraphFormat::RdfXml), 2),
            "application/xml" | "text/xml" => (Some(GraphFormat::RdfXml), 1),
            // sq-oy1f.1: JSON-LD, OPT-IN behind the `jsonld` feature. When the
            // feature is off this arm is compiled out, so `application/ld+json` matches no arm
            // and falls through to the default (N-Triples) like any unsupported type.
            #[cfg(feature = "jsonld")]
            "application/ld+json" => (Some(GraphFormat::JsonLd), 2),
            "*/*" => (Some(GraphFormat::NTriples), 0),
            _ => (None, 0),
        };
        if let Some(fmt) = fmt {
            if q <= 0.0 {
                continue; // q=0 explicitly rejects
            }
            let better = match best {
                None => true,
                Some((_, bq, bspec)) => q > bq || (q == bq && spec > bspec),
            };
            if better {
                best = Some((fmt, q, spec));
            }
        }
    }
    // A present, non-empty `Accept` that matched no supported RDF range (and no wildcard) is
    // unsatisfiable → 406, exactly as Oxigraph does for a CONSTRUCT/DESCRIBE result.
    best.map(|(f, _, _)| f).ok_or(NotAcceptable)
}

/// Splits an `Accept` header into (lowercased media range, q-value) pairs.
fn media_ranges(accept: &str) -> impl Iterator<Item = (String, f32)> + '_ {
    accept.split(',').map(|part| {
        let mut it = part.split(';');
        let media = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut q = 1.0f32;
        for param in it {
            if let Some(v) = param.trim().strip_prefix("q=") {
                q = v.parse().unwrap_or(1.0);
            }
        }
        (media, q)
    })
}

/// A present-but-unsatisfiable `Accept` header — the HTTP layer maps this to `406 Not
/// Acceptable`. sq-406acc: returned only by the `*_or_406` negotiators when an
/// `Accept` header is PRESENT and non-empty yet names no supported media range and no wildcard.
/// An absent / empty / `*/*` `Accept` is NEVER a `NotAcceptable` (the W3C SPARQL Protocol
/// permits a default representation when `Accept` is absent), so the existing default-to-JSON /
/// default-to-N-Triples behaviour is preserved in those cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAcceptable;

/// Negotiates the result format from an optional `Accept` header value.
///
/// This is the LENIENT entry point used by surfaces that never 406 (it always yields a concrete
/// format, defaulting to JSON). The HTTP query path uses [`negotiate_or_406`] instead — see that
/// function for the Oxigraph-parity strict behaviour (sq-406acc).
pub fn negotiate(accept: Option<&str>) -> Format {
    negotiate_or_406(accept).unwrap_or(Format::Json)
}

/// Negotiates the SELECT/ASK result format with Oxigraph-parity strictness (sq-406acc).
///
/// Returns:
/// * `Ok(format)` — a usable format was selected: either an explicitly-named supported type,
///   the wildcard `*/*` (or any matchable range), OR the JSON default when the `Accept` header
///   is ABSENT / empty (the W3C SPARQL Protocol permits a default representation when `Accept`
///   is absent — so an absent header is never a 406).
/// * `Err(`[`NotAcceptable`]`)` — the `Accept` header is PRESENT and non-empty but names NO
///   supported solution media type and NO wildcard, so the server cannot honour it. The HTTP
///   layer turns this into a `406 Not Acceptable`, matching Oxigraph (w3c/sparql-protocol#40).
///
/// The selection logic (q-values, specificity, `q=0` rejection) is identical to [`negotiate`];
/// the only difference is that a present-but-unsatisfiable header is reported as an error
/// instead of silently falling back to JSON.
pub fn negotiate_or_406(accept: Option<&str>) -> Result<Format, NotAcceptable> {
    let accept = match accept {
        // Absent or empty `Accept` → the JSON default is permitted (never a 406).
        Some(a) if !a.trim().is_empty() => a,
        _ => return Ok(Format::Json),
    };

    let mut best: Option<(Format, f32, usize)> = None; // (format, q, specificity)
    for (media, q) in media_ranges(accept) {
        // specificity: an exact supported type beats a wildcard at equal q; the first
        // listed wins remaining ties (the `better` test below keeps the earlier match).
        let (fmt, spec) = match media.as_str() {
            "application/sparql-results+json" | "application/json" => (Some(Format::Json), 2),
            "application/sparql-results+xml" | "application/xml" | "text/xml" => (Some(Format::Xml), 2),
            "text/csv" => (Some(Format::Csv), 2),
            "text/tab-separated-values" => (Some(Format::Tsv), 2),
            "*/*" => (Some(Format::Json), 0),
            _ => (None, 0),
        };
        if let Some(fmt) = fmt {
            if q <= 0.0 {
                continue; // q=0 explicitly rejects
            }
            let better = match best {
                None => true,
                Some((_, bq, bspec)) => q > bq || (q == bq && spec > bspec),
            };
            if better {
                best = Some((fmt, q, spec));
            }
        }
    }
    // A present, non-empty `Accept` that matched no supported range (and no wildcard) is
    // unsatisfiable → 406, exactly as Oxigraph does. `q=0` on a matched type counts as a
    // rejection, not a match, so `Accept: application/sparql-results+json;q=0` is also a 406.
    best.map(|(f, _, _)| f).ok_or(NotAcceptable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_json() {
        assert_eq!(negotiate(None), Format::Json);
        assert_eq!(negotiate(Some("")), Format::Json);
        assert_eq!(negotiate(Some("*/*")), Format::Json);
    }

    #[test]
    fn exact_types() {
        assert_eq!(negotiate(Some("application/sparql-results+json")), Format::Json);
        assert_eq!(negotiate(Some("application/sparql-results+xml")), Format::Xml);
        assert_eq!(negotiate(Some("text/csv")), Format::Csv);
        assert_eq!(negotiate(Some("text/tab-separated-values")), Format::Tsv);
    }

    #[test]
    fn q_values_pick_highest() {
        assert_eq!(
            negotiate(Some("application/sparql-results+json;q=0.5, text/csv;q=0.9")),
            Format::Csv
        );
        // q=0 rejects json, so xml wins
        assert_eq!(
            negotiate(Some("application/sparql-results+json;q=0, application/sparql-results+xml")),
            Format::Xml
        );
    }

    #[test]
    fn unsupported_falls_back_to_json() {
        assert_eq!(negotiate(Some("application/pdf")), Format::Json);
    }

    #[test]
    fn exact_beats_wildcard() {
        assert_eq!(negotiate(Some("text/csv, */*")), Format::Csv);
    }

    // sq-406acc: the Oxigraph-parity strict negotiator. An ABSENT / empty / `*/*`
    // Accept keeps the JSON default (Ok); a PRESENT-but-unsatisfiable Accept is `NotAcceptable`
    // (the HTTP layer maps it to 406), instead of the lenient `negotiate`'s JSON fallback.
    #[test]
    fn or_406_absent_or_wildcard_keeps_json_default() {
        assert_eq!(negotiate_or_406(None), Ok(Format::Json));
        assert_eq!(negotiate_or_406(Some("")), Ok(Format::Json));
        assert_eq!(negotiate_or_406(Some("   ")), Ok(Format::Json));
        assert_eq!(negotiate_or_406(Some("*/*")), Ok(Format::Json));
    }

    #[test]
    fn or_406_supported_types_select_format() {
        assert_eq!(
            negotiate_or_406(Some("application/sparql-results+json")),
            Ok(Format::Json)
        );
        assert_eq!(negotiate_or_406(Some("text/csv")), Ok(Format::Csv));
        assert_eq!(negotiate_or_406(Some("text/tab-separated-values")), Ok(Format::Tsv));
        // A supported type alongside an unsupported one still negotiates the supported one.
        assert_eq!(negotiate_or_406(Some("image/png, text/csv")), Ok(Format::Csv));
        // An explicit wildcard rescues an otherwise-unsupported list.
        assert_eq!(negotiate_or_406(Some("image/png, */*")), Ok(Format::Json));
    }

    #[test]
    fn or_406_present_unsatisfiable_is_not_acceptable() {
        // A present Accept naming ONLY unsupported solution media types → 406 (Oxigraph parity).
        assert_eq!(negotiate_or_406(Some("image/png")), Err(NotAcceptable));
        assert_eq!(negotiate_or_406(Some("application/pdf")), Err(NotAcceptable));
        assert_eq!(
            negotiate_or_406(Some("text/turtle, application/rdf+xml")),
            Err(NotAcceptable)
        );
        // `q=0` rejects the only otherwise-supported type, leaving nothing acceptable.
        assert_eq!(
            negotiate_or_406(Some("application/sparql-results+json;q=0")),
            Err(NotAcceptable)
        );
    }

    #[test]
    fn negotiate_matches_or_406_modulo_default() {
        // The lenient wrapper is exactly `_or_406` with the error collapsed to the JSON default.
        for accept in [
            None,
            Some(""),
            Some("*/*"),
            Some("text/csv"),
            Some("image/png"),
            Some("application/sparql-results+json;q=0"),
        ] {
            assert_eq!(negotiate(accept), negotiate_or_406(accept).unwrap_or(Format::Json));
        }
    }

    // sq-406acc: the graph (CONSTRUCT/DESCRIBE) analogue.
    #[test]
    fn graph_or_406_absent_or_wildcard_keeps_ntriples_default() {
        assert_eq!(negotiate_graph_or_406(None), Ok(GraphFormat::NTriples));
        assert_eq!(negotiate_graph_or_406(Some("")), Ok(GraphFormat::NTriples));
        assert_eq!(negotiate_graph_or_406(Some("*/*")), Ok(GraphFormat::NTriples));
    }

    #[test]
    fn graph_or_406_supported_and_unsatisfiable() {
        assert_eq!(negotiate_graph_or_406(Some("text/turtle")), Ok(GraphFormat::Turtle));
        assert_eq!(
            negotiate_graph_or_406(Some("application/n-triples")),
            Ok(GraphFormat::NTriples)
        );
        // A SELECT/ASK result media type is NOT an RDF graph type → 406 for a graph query.
        assert_eq!(
            negotiate_graph_or_406(Some("text/csv")),
            Err(NotAcceptable)
        );
        assert_eq!(negotiate_graph_or_406(Some("image/png")), Err(NotAcceptable));
    }

    #[test]
    fn graph_formats() {
        assert_eq!(negotiate_graph(None), GraphFormat::NTriples);
        assert_eq!(negotiate_graph(Some("")), GraphFormat::NTriples);
        assert_eq!(negotiate_graph(Some("*/*")), GraphFormat::NTriples);
        assert_eq!(negotiate_graph(Some("text/turtle")), GraphFormat::Turtle);
        assert_eq!(negotiate_graph(Some("application/n-triples")), GraphFormat::NTriples);
        // q-values decide; q=0 rejects.
        assert_eq!(
            negotiate_graph(Some("application/n-triples;q=0.5, text/turtle;q=0.9")),
            GraphFormat::Turtle
        );
        assert_eq!(
            negotiate_graph(Some("application/n-triples;q=0, text/turtle")),
            GraphFormat::Turtle
        );
        // exact beats wildcard; unsupported falls back to N-Triples.
        assert_eq!(negotiate_graph(Some("text/turtle, */*")), GraphFormat::Turtle);
        // sq-rt6v: RDF/XML is now a first-class graph format.
        assert_eq!(negotiate_graph(Some("application/rdf+xml")), GraphFormat::RdfXml);
        // The `application/xml`/`text/xml` aliases also map to RDF/XML, but at lower
        // specificity, so an exact `application/rdf+xml` (or turtle/n-triples) wins a tie.
        assert_eq!(negotiate_graph(Some("application/xml")), GraphFormat::RdfXml);
        assert_eq!(negotiate_graph(Some("text/xml")), GraphFormat::RdfXml);
        assert_eq!(negotiate_graph(Some("text/turtle, application/xml")), GraphFormat::Turtle);
        // q=0 rejects RDF/XML, so Turtle wins.
        assert_eq!(negotiate_graph(Some("application/rdf+xml;q=0, text/turtle")), GraphFormat::Turtle);
        // An unsupported type still falls back to N-Triples.
        assert_eq!(negotiate_graph(Some("application/pdf")), GraphFormat::NTriples);
    }

    // sq-oy1f.1: JSON-LD negotiation is only compiled with the `jsonld` feature.
    #[cfg(feature = "jsonld")]
    #[test]
    fn graph_format_jsonld() {
        assert_eq!(negotiate_graph(Some("application/ld+json")), GraphFormat::JsonLd);
        assert_eq!(GraphFormat::JsonLd.content_type(), "application/ld+json; charset=utf-8");
        // q-values decide between JSON-LD and the other graph formats.
        assert_eq!(
            negotiate_graph(Some("text/turtle;q=0.5, application/ld+json;q=0.9")),
            GraphFormat::JsonLd
        );
        // q=0 rejects JSON-LD, so Turtle wins.
        assert_eq!(
            negotiate_graph(Some("application/ld+json;q=0, text/turtle")),
            GraphFormat::Turtle
        );
        // Exact JSON-LD beats a wildcard.
        assert_eq!(negotiate_graph(Some("application/ld+json, */*")), GraphFormat::JsonLd);
    }

    // sq-oy1f.1: WITHOUT the feature, `application/ld+json` is just another
    // unsupported type — negotiation must fall through to the default (N-Triples).
    #[cfg(not(feature = "jsonld"))]
    #[test]
    fn graph_format_jsonld_unsupported_without_feature() {
        assert_eq!(negotiate_graph(Some("application/ld+json")), GraphFormat::NTriples);
        // Even alongside a supported type, the JSON-LD range is ignored and Turtle is picked.
        assert_eq!(
            negotiate_graph(Some("application/ld+json;q=0.9, text/turtle;q=0.5")),
            GraphFormat::Turtle
        );
    }
}

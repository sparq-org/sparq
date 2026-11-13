//! Shared, test-only GeoSPARQL Benchmark loading and answer comparison.
//! GSBEvaluationModule compares JSON trees: object-key order is immaterial;
//! variable and row arrays, term kind, lexical form, datatype and language matter.

use oxrdf::Term;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use sparq_core::Graph;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

const WKT_LITERAL: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
const GML_LITERAL: &str = "http://www.opengis.net/ont/geosparql#gmlLiteral";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// A SPARQL-results term in the shape the benchmark compares.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    kind: &'static str,
    value: String,
    datatype: Option<String>,
    lang: Option<String>,
}

impl Binding {
    /// The upstream `removeWKTWhiteSpaces` normalisation, plus the RDF 1.1
    /// plain-literal/`xsd:string` identification.
    fn normalized(mut self) -> Self {
        match self.datatype.as_deref() {
            Some(WKT_LITERAL) => {
                self.value = self
                    .value
                    .chars()
                    .filter(|c| *c != ' ' && *c != '\n')
                    .collect::<String>()
                    .trim_matches(|c| c <= '\u{20}')
                    .to_lowercase();
            }
            Some(GML_LITERAL | "http://www.opengis.net/ont/geosparql#kmlLiteral") => {
                // Out of `canonical_xml`'s bounds the literal is compared
                // VERBATIM. The old behaviour — keep whatever prefix had been
                // emitted before the XML error — could equate two DIFFERENT
                // literals that happen to share a prefix; the raw text cannot.
                if let Ok(c) = canonical_xml(&self.value) {
                    self.value = c;
                }
            }
            Some(XSD_STRING) => self.datatype = None,
            _ => {}
        }
        if self.lang.is_some() {
            self.datatype = None;
        }
        self
    }
}

/// An XML name (element or attribute), which the benchmark's literals encode as
/// UTF-8. Lossy decoding is deliberately not used: it would map two different
/// byte sequences onto the same `U+FFFD` name.
fn xml_name(raw: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(raw).map_err(|e| format!("xml name: {}", e))
}

/// Inclusive XML C14N 1.0 without comments, as selected by the Java evaluator.
/// Preserves character-data whitespace and prefixes, expands empty tags, suppresses
/// redundant namespace declarations, and orders attributes by expanded name.
/// DTD-bearing XML is outside the benchmark's GML profile: returning Err leaves
/// the lexical value unchanged, as upstream does on canonicalization failure.
fn canonical_xml(s: &str) -> Result<String, String> {
    fn escaped(s: &str, attr: bool) -> String {
        let mut out = String::new();
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' if !attr => out.push_str("&gt;"),
                '"' if attr => out.push_str("&quot;"),
                '\t' if attr => out.push_str("&#x9;"),
                '\n' if attr => out.push_str("&#xA;"),
                '\r' => out.push_str("&#xD;"),
                _ => out.push(c),
            }
        }
        out
    }
    fn namespace<'a>(name: &str, scope: &'a BTreeMap<String, String>) -> Result<&'a str, String> {
        match name.split_once(':') {
            Some((prefix, _)) => scope
                .get(prefix)
                .map(String::as_str)
                .ok_or_else(|| format!("unbound namespace: {}", prefix)),
            None => Ok(""),
        }
    }

    let input = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut reader = Reader::from_str(&input);
    reader.config_mut().expand_empty_elements = true;
    let mut initial = BTreeMap::new();
    initial.insert("xml".into(), "http://www.w3.org/XML/1998/namespace".into());
    initial.insert(String::new(), String::new());
    let mut scopes = vec![initial];
    let mut seen_root = false;
    let mut out = String::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Eof => break,
            Event::Start(e) => {
                if scopes.len() == 1 && seen_root {
                    return Err("multiple XML roots".into());
                }
                seen_root = true;
                let parent = scopes.last().unwrap();
                let mut scope = parent.clone();
                let mut attrs = Vec::new();
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    let name = xml_name(a.key.as_ref())?.to_string();
                    let value = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    if name == "xmlns" || name.starts_with("xmlns:") {
                        if !value.is_empty() && !value.contains(':') {
                            return Err("relative namespace IRI".into());
                        }
                        let prefix = name.strip_prefix("xmlns:").unwrap_or("");
                        scope.insert(prefix.to_string(), value);
                    } else {
                        attrs.push((name, value));
                    }
                }
                let name = xml_name(e.name().as_ref())?.to_string();
                namespace(&name, &scope)?;
                write!(out, "<{}", name).unwrap();
                // Inclusive namespaces, sorted by prefix; inherited bindings
                // appear only when changed at this element.
                for (prefix, uri) in &scope {
                    if prefix != "xml" && parent.get(prefix) != Some(uri) {
                        let key = if prefix.is_empty() {
                            "xmlns".to_string()
                        } else {
                            format!("xmlns:{}", prefix)
                        };
                        write!(out, " {}=\"{}\"", key, escaped(uri, true)).unwrap();
                    }
                }
                let mut expanded = Vec::new();
                for (name, value) in attrs {
                    let uri = namespace(&name, &scope)?.to_string();
                    let local = name
                        .split_once(':')
                        .map_or(name.as_str(), |(_, l)| l)
                        .to_string();
                    expanded.push((uri, local, name, value));
                }
                expanded.sort();
                for (_, _, name, value) in expanded {
                    write!(out, " {}=\"{}\"", name, escaped(&value, true)).unwrap();
                }
                out.push('>');
                scopes.push(scope);
            }
            Event::End(e) => {
                if scopes.len() <= 1 {
                    return Err("stray XML end tag".into());
                }
                write!(out, "</{}>", xml_name(e.name().as_ref())?).unwrap();
                scopes.pop();
            }
            Event::Text(t) => {
                let text = t.decode().map_err(|e| e.to_string())?;
                if scopes.len() > 1 {
                    out.push_str(&escaped(&text, false));
                } else if !text.trim().is_empty() {
                    return Err("text outside XML root".into());
                }
            }
            Event::CData(t) => {
                out.push_str(&escaped(&t.decode().map_err(|e| e.to_string())?, false))
            }
            Event::GeneralRef(r) => out.push_str(&escaped(&resolve_ref(&r)?, false)),
            Event::PI(pi) => {
                if scopes.len() == 1 && seen_root {
                    out.push('\n');
                }
                write!(out, "<?{}?>", xml_name(pi.as_ref())?).unwrap();
                if scopes.len() == 1 && !seen_root {
                    out.push('\n');
                }
            }
            Event::Decl(_) | Event::Comment(_) => {}
            other => return Err(format!("unsupported XML construct: {:?}", other)),
        }
    }
    if scopes.len() != 1 || !seen_root {
        return Err("incomplete XML document".into());
    }
    Ok(out)
}

/// Resolves one `&name;` / `&#nn;` reference to the character(s) it stands for.
/// An entity the XML predefined set cannot resolve is an error, never a silently
/// dropped character.
fn resolve_ref(r: &quick_xml::events::BytesRef<'_>) -> Result<String, String> {
    let name = r.decode().map_err(|e| format!("xml entity: {}", e))?;
    unescape(&format!("&{};", name))
        .map(|c| c.into_owned())
        .map_err(|e| format!("xml entity &{};: {}", name, e))
}

/// A whole SPARQL result set, in the shape the benchmark compares: the ordered
/// variable list and the ordered row list, each row a name -> binding map with
/// unbound variables absent (the SPARQL-JSON encoding the upstream diffs).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResultSet {
    vars: Vec<String>,
    rows: Vec<BTreeMap<String, Binding>>,
}

fn term_binding(t: &Term) -> Binding {
    match t {
        Term::NamedNode(n) => Binding {
            kind: "uri",
            value: n.as_str().to_string(),
            datatype: None,
            lang: None,
        },
        Term::BlankNode(b) => Binding {
            kind: "bnode",
            value: b.as_str().to_string(),
            datatype: None,
            lang: None,
        },
        Term::Literal(l) => Binding {
            kind: "literal",
            value: l.value().to_string(),
            datatype: Some(l.datatype().as_str().to_string()),
            lang: l.language().map(str::to_string),
        },
        // oxrdf's `Term` is non-exhaustive (RDF-1.2 triple terms); the benchmark
        // never produces one, and an unknown shape must not silently compare
        // equal to anything.
        other => Binding {
            kind: "unknown",
            value: format!("{:?}", other),
            datatype: None,
            lang: None,
        },
    }
}

fn from_query_result(r: &sparq_engine::QueryResult) -> ResultSet {
    let vars: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let rows = r
        .rows
        .iter()
        .map(|row| {
            let mut m = BTreeMap::new();
            for (i, cell) in row.iter().enumerate() {
                if let (Some(name), Some(t)) = (vars.get(i), cell.as_ref()) {
                    m.insert(name.clone(), term_binding(t).normalized());
                }
            }
            m
        })
        .collect();
    ResultSet { vars, rows }
}

/// Parses a SPARQL Results XML (`.srx`) answer file.
fn parse_srx(xml: &str) -> Result<ResultSet, String> {
    let input = xml.replace("\r\n", "\n").replace('\r', "\n");
    let mut reader = Reader::from_str(&input);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut root = false;
    let mut results = false;
    let mut vars: Vec<String> = Vec::new();
    let mut rows: Vec<BTreeMap<String, Binding>> = Vec::new();
    let mut row: BTreeMap<String, Binding> = BTreeMap::new();
    // The binding currently open: its variable name plus the term being read.
    let mut binding_name: Option<String> = None;
    let mut term: Option<Binding> = None;
    let mut text = String::new();

    let attr = |e: &quick_xml::events::BytesStart<'_>, want: &str| -> Option<String> {
        e.attributes().flatten().find_map(|a| {
            if a.key.as_ref() == want.as_bytes() {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            } else {
                None
            }
        })
    };

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("srx: {}", e)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if depth == 0 {
                    if root || e.name().as_ref() != b"sparql" {
                        return Err("invalid SRX root".into());
                    }
                    root = true;
                }
                depth += 1;
                match e.name().as_ref() {
                    b"results" => results = true,
                    b"variable" => {
                        if let Some(n) = attr(&e, "name") {
                            vars.push(n);
                        }
                    }
                    b"result" => row = BTreeMap::new(),
                    b"binding" => binding_name = attr(&e, "name"),
                    b"uri" => {
                        term = Some(Binding {
                            kind: "uri",
                            value: String::new(),
                            datatype: None,
                            lang: None,
                        })
                    }
                    b"bnode" => {
                        term = Some(Binding {
                            kind: "bnode",
                            value: String::new(),
                            datatype: None,
                            lang: None,
                        })
                    }
                    b"literal" => {
                        term = Some(Binding {
                            kind: "literal",
                            value: String::new(),
                            datatype: attr(&e, "datatype"),
                            lang: attr(&e, "xml:lang"),
                        })
                    }
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Text(t)) => {
                text.push_str(&t.decode().map_err(|e| e.to_string())?);
            }
            Ok(Event::CData(t)) => text.push_str(&t.decode().map_err(|e| e.to_string())?),
            // The reader reports `&amp;` / `&#38;` as its own event, so an
            // escaped value — every `geo:gmlLiteral` answer is one — loses
            // characters unless the reference is resolved back into the text.
            Ok(Event::GeneralRef(r)) => text.push_str(&resolve_ref(&r)?),
            Ok(Event::End(e)) => {
                depth = depth.checked_sub(1).ok_or("stray SRX end tag")?;
                match e.name().as_ref() {
                    b"uri" | b"bnode" | b"literal" => {
                        if let Some(b) = term.as_mut() {
                            b.value.clone_from(&text);
                        }
                        text.clear();
                    }
                    b"binding" => {
                        if let (Some(n), Some(b)) = (binding_name.take(), term.take()) {
                            row.insert(n, b.normalized());
                        }
                    }
                    b"result" => rows.push(std::mem::take(&mut row)),
                    _ => {}
                }
            }
            Ok(Event::DocType(_)) => return Err("DTD is outside the SRX fixture profile".into()),
            Ok(_) => {}
        }
    }
    if !root || !results || depth != 0 {
        return Err("incomplete SRX result set".into());
    }
    Ok(ResultSet { vars, rows })
}

/// Loads the benchmark dataset, optionally materialising its RDFS closure.
///
/// The closure is what the RDFSE requirements (R25–R27) ask for, but it is not
/// free: the benchmark's expected answers for the geometry-property requirements
/// (R8/R9) were fixed against a reference that resolves `geo:hasGeometry` to the
/// ASSERTED triple only, so the extra `my:hasPointGeometry rdfs:subPropertyOf
/// geo:hasGeometry` solution the closure adds reads there as a wrong answer.
/// `GSB_RDFS=0` runs the entailment-free configuration so both are measurable.
pub(crate) fn load_dataset(dir: &Path, rdfs: bool) -> Graph {
    let path = dir.join("gsb_dataset/dataset.rdf");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let (mut dict, mut triples) = Graph::parse_to_triples(&text, "rdfxml")
        .unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e));
    if rdfs {
        sparq_reason::materialize_rdfs(&mut dict, &mut triples);
    }
    Graph::from_parts(dict, triples)
}

/// Runs one benchmark query through the opt-in sparq GeoSPARQL stack.
///
/// `rewrite` selects the entry point: the `geosparql_rewrite` one (the QRW
/// requirements R28–R30 are what it exists for) or the standard engine one,
/// where a `geo:sfWithin` triple matches only ASSERTED triples. `GSB_REWRITE=0`
/// picks the latter so the rewrite's net effect on the score is measurable.
pub(crate) fn run_query(graph: &Graph, sparql: &str, rewrite: bool) -> Result<ResultSet, String> {
    let registry = sparq_geo::geof_registry();
    let result = sparq_engine::with_functions(&registry, || {
        if rewrite {
            sparq_engine::query_prepared(graph, &sparq_geo::geosparql_rewrite(sparql)?)
        } else {
            sparq_engine::query(graph, sparql)
        }
    })?;
    Ok(from_query_result(&result))
}

/// The expected answer plus every `-alternative-N.srx` the benchmark accepts.
pub(crate) fn expected_answers(dir: &Path, id: &str) -> Result<Vec<ResultSet>, String> {
    let parse = |path: &Path| {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        parse_srx(&text).map_err(|e| format!("{}: {}", path.display(), e))
    };
    let mut out = vec![parse(&dir.join(format!("gsb_answers/{}.srx", id)))?];
    // GSBSeqTaskGenerator stops at the first absent numbered alternative.
    for k in 1.. {
        let alt = dir.join(format!("gsb_answers/{}-alternative-{}.srx", id, k));
        if !alt.try_exists().map_err(|e| e.to_string())? {
            break;
        }
        out.push(parse(&alt)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SRX: &str = r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
 <head><variable name="f"/><variable name="wkt"/></head>
 <results>
  <result>
   <binding name="f"><uri>http://example.org/ApplicationSchema#A</uri></binding>
   <binding name="wkt"><literal datatype="http://www.opengis.net/ont/geosparql#wktLiteral"><![CDATA[Polygon((-83.6 34.1, -83.2 34.1))]]></literal></binding>
  </result>
 </results>
</sparql>"#;

    #[test]
    fn srx_parses_into_the_compared_shape() {
        let rs = parse_srx(SRX).expect("parse");
        assert_eq!(rs.vars, ["f", "wkt"]);
        assert_eq!(rs.rows.len(), 1);
        assert_eq!(rs.rows[0]["f"].kind, "uri");
        assert_eq!(
            rs.rows[0]["f"].value,
            "http://example.org/ApplicationSchema#A"
        );
        // The WKT normalisation: whitespace removed, lower-cased.
        assert_eq!(rs.rows[0]["wkt"].value, "polygon((-83.634.1,-83.234.1))");
    }

    #[test]
    fn wkt_answers_compare_modulo_whitespace_and_case_only() {
        let spaced = SRX.replace(
            "Polygon((-83.6 34.1, -83.2 34.1))",
            "POLYGON((-83.6  34.1,-83.2 34.1))",
        );
        assert_eq!(parse_srx(SRX).unwrap(), parse_srx(&spaced).unwrap());
        // …but a different COORDINATE is a different answer.
        let moved = SRX.replace("-83.2 34.1", "-83.3 34.1");
        assert_ne!(parse_srx(SRX).unwrap(), parse_srx(&moved).unwrap());
    }

    #[test]
    fn row_order_and_variable_order_are_significant() {
        let second = r#"<result><binding name="f"><uri>urn:second</uri></binding></result>"#;
        let first = SRX
            .split("<result>")
            .nth(1)
            .unwrap()
            .split("</result>")
            .next()
            .unwrap();
        let first = format!("<result>{}</result>", first);
        let two = SRX.replace(&first, &format!("{}{}", first, second));
        let swapped = SRX.replace(&first, &format!("{}{}", second, first));
        assert_ne!(parse_srx(&two).unwrap(), parse_srx(&swapped).unwrap());
        assert_ne!(parse_srx(&two).unwrap(), parse_srx(SRX).unwrap());
        let vars = SRX.replace(
            r#"<variable name="f"/><variable name="wkt"/>"#,
            r#"<variable name="wkt"/><variable name="f"/>"#,
        );
        assert_ne!(parse_srx(&vars).unwrap(), parse_srx(SRX).unwrap());
        // Binding object-key order has no significance in Jackson JSON trees.
        let original = r#"<binding name="f"><uri>urn:f</uri></binding><binding name="wkt"><literal>x</literal></binding>"#;
        let reversed = r#"<binding name="wkt"><literal>x</literal></binding><binding name="f"><uri>urn:f</uri></binding>"#;
        let doc = format!(
            r#"<sparql><head/><results><result>{}</result></results></sparql>"#,
            original
        );
        assert_eq!(
            parse_srx(&doc).unwrap(),
            parse_srx(&doc.replace(original, reversed)).unwrap()
        );
    }

    /// Canonicalize an XML literal in the benchmark's GML profile.
    fn gml(s: &str) -> String {
        canonical_xml(s).unwrap_or_else(|e| panic!("canonical_xml({}): {}", s, e))
    }

    /// The GML literal `s`, put through the comparison exactly as an answer is.
    fn gml_binding(s: &str) -> Binding {
        Binding {
            kind: "literal",
            value: s.to_string(),
            datatype: Some(GML_LITERAL.into()),
            lang: None,
        }
        .normalized()
    }

    #[test]
    fn gml_character_data_whitespace_is_significant() {
        let one = "<gml:Point xmlns:gml=\"http://www.opengis.net/ont/gml\"><gml:pos>1 2</gml:pos></gml:Point>";
        let two = "<gml:Point   xmlns:gml=\"http://www.opengis.net/ont/gml\">\n  <gml:pos>1   2</gml:pos>\n</gml:Point>";
        assert_ne!(gml(one), gml(two));
        // A different coordinate must still differ.
        assert_ne!(gml(one), gml(&one.replace("1 2", "1 3")));
        // …and so must a different namespace URI behind the same prefix.
        assert_ne!(gml(one), gml(&one.replace("ont/gml", "ont/other")));
    }

    #[test]
    fn empty_elements_normalise_to_the_start_end_pair() {
        assert_eq!(gml("<a><b/></a>"), gml("<a><b></b></a>"));
        assert_eq!(
            gml("<a><b attr=\"v\"/></a>"),
            gml("<a><b attr=\"v\"></b></a>")
        );
        // An empty element is still not the same as an absent one.
        assert_ne!(gml("<a><b/></a>"), gml("<a></a>"));
    }

    #[test]
    fn attribute_and_namespace_order_is_not_a_difference() {
        let a = r#"<gml:Point xmlns:gml="urn:G" srsDimension="2" srsName="S"/>"#;
        let b = r#"<gml:Point srsName="S" srsDimension="2" xmlns:gml="urn:G"/>"#;
        assert_eq!(gml(a), gml(b));
        // A namespace declaration sorts ahead of a plain attribute (C14N's
        // order); only determinism is load-bearing, but pin it so a reordering
        // of the emitter is a deliberate change.
        assert!(gml(a).starts_with(r#"<gml:Point xmlns:gml="urn:G" srsDimension="2""#));
        // An attribute VALUE, name, or count is still a difference.
        assert_ne!(gml(a), gml(&a.replace(r#"srsName="S""#, r#"srsName="T""#)));
        assert_ne!(gml(a), gml(&a.replace("srsName", "srsname")));
        assert_ne!(
            gml(a),
            gml(r#"<gml:Point xmlns:gml="urn:G" srsDimension="2"/>"#)
        );
    }

    #[test]
    fn escaped_and_literal_spellings_of_a_character_agree() {
        // In character data…
        assert_eq!(gml("<a>x &amp; y</a>"), gml("<a>x &#38; y</a>"));
        assert_eq!(gml("<a>x &lt; y</a>"), gml("<a><![CDATA[x < y]]></a>"));
        // …and in an attribute value.
        assert_eq!(gml(r#"<a t="&quot;q&quot;"/>"#), gml("<a t='\"q\"'/>"));
        assert_eq!(gml(r#"<a t="x &amp; y"/>"#), gml(r#"<a t="x &#38; y"/>"#));
        // Re-escaping must not let escaped text pass for markup, nor an
        // attribute value break out of its quotes.
        assert_ne!(gml("<a>&lt;b/&gt;</a>"), gml("<a><b/></a>"));
        assert_ne!(
            gml(r#"<a t="x&quot; u=&quot;y"/>"#),
            gml(r#"<a t="x" u="y"/>"#)
        );
    }

    #[test]
    fn out_of_bounds_gml_is_compared_verbatim_never_as_a_partial_parse() {
        // Malformed, and the constructs the normaliser does not model.
        assert!(canonical_xml("<gml:Point><gml:pos>1 2</gml:Point>").is_err());
        assert!(canonical_xml("<a>").is_err());
        assert!(canonical_xml("<!DOCTYPE a><a/>").is_err());
        assert_eq!(gml("<a><!-- c --></a>"), "<a></a>");
        assert_eq!(gml("<a><?pi go?></a>"), "<a><?pi go?></a>");
        assert!(canonical_xml("<a>&noSuchEntity;</a>").is_err());
        // The regression this guards: two literals that agree only on the
        // prefix before the XML error must NOT compare equal.
        assert_ne!(
            gml_binding("<a>x</b>keep-me"),
            gml_binding("<a>x</b>different"),
        );
        // Canonicalization failure leaves the literal unchanged, as in Java.
        assert_eq!(
            gml_binding("<a><!-- c --></a>"),
            gml_binding("<a><!-- c --></a>")
        );
    }

    #[test]
    fn srx_entity_references_survive_the_parse() {
        // Inside an `.srx` answer a GML literal is XML-escaped; dropping those
        // references mangles every expected GML answer before the comparator
        // ever sees it.
        let srx = format!(
            r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
 <head><variable name="g"/></head>
 <results><result><binding name="g"><literal datatype="{}">&lt;gml:Point xmlns:gml="urn:G"&gt;&lt;gml:pos&gt;1 2&lt;/gml:pos&gt;&lt;/gml:Point&gt;</literal></binding></result></results>
</sparql>"#,
            GML_LITERAL
        );
        let rs = parse_srx(&srx).expect("parse");
        assert_eq!(
            rs.rows[0]["g"].value,
            gml(r#"<gml:Point xmlns:gml="urn:G"><gml:pos>1 2</gml:pos></gml:Point>"#),
        );
    }

    #[test]
    fn plain_and_xsd_string_literals_are_the_same_answer() {
        let plain = Binding {
            kind: "literal",
            value: "x".into(),
            datatype: None,
            lang: None,
        };
        let typed = Binding {
            kind: "literal",
            value: "x".into(),
            datatype: Some(XSD_STRING.into()),
            lang: None,
        };
        assert_eq!(plain.clone().normalized(), typed.normalized());
        // A genuinely different datatype is NOT normalised away.
        let integer = Binding {
            datatype: Some("http://www.w3.org/2001/XMLSchema#integer".into()),
            ..plain.clone()
        };
        assert_ne!(plain.normalized(), integer.normalized());
    }

    #[test]
    fn java_wkt_normalization_does_not_remove_internal_tabs() {
        let tab = SRX.replace("-83.6 34.1", "-83.6\t34.1");
        assert_ne!(parse_srx(SRX).unwrap(), parse_srx(&tab).unwrap());
        let newline = SRX.replace("-83.6 34.1", "-83.6\n34.1");
        assert_eq!(parse_srx(SRX).unwrap(), parse_srx(&newline).unwrap());
    }

    #[test]
    fn c14n_namespace_scope_expanded_attribute_order_and_escaping() {
        let xml = "<a xmlns:z=\"urn:a\" xmlns:b=\"urn:z\" b:a=\"2\" z:z=\"1\"><n xmlns:z=\"urn:a\"/>\n</a>";
        assert_eq!(
            gml(xml),
            "<a xmlns:b=\"urn:z\" xmlns:z=\"urn:a\" z:z=\"1\" b:a=\"2\"><n></n>\n</a>"
        );
        assert_eq!(
            gml("<a xmlns=\"urn:a\"><b xmlns=\"\"/></a>"),
            "<a xmlns=\"urn:a\"><b xmlns=\"\"></b></a>"
        );
        assert_eq!(
            gml("<a x=\"&#x9;&#xA;&#xD;\">&#xD; &gt; &lt; &amp;</a>"),
            "<a x=\"&#x9;&#xA;&#xD;\">&#xD; &gt; &lt; &amp;</a>"
        );
        assert_ne!(
            gml("<a xmlns:x=\"urn:a\"><x:b/></a>"),
            gml("<a xmlns:y=\"urn:a\"><y:b/></a>")
        );
        assert_eq!(
            gml("<?before go?><a><!-- omitted -->x</a><?after go?>"),
            "<?before go?>\n<a>x</a>\n<?after go?>"
        );
    }

    #[test]
    fn srx_preserves_lexical_datatype_language_and_empty_binding_structure() {
        let doc = r#"<sparql><head><variable name="x"/></head><results><result><binding name="x"><literal datatype="http://www.w3.org/2001/XMLSchema#double">0.0</literal></binding></result></results></sparql>"#;
        assert_ne!(
            parse_srx(doc).unwrap(),
            parse_srx(&doc.replace("0.0", "0")).unwrap()
        );
        assert_ne!(
            parse_srx(doc).unwrap(),
            parse_srx(&doc.replace("#double", "#decimal")).unwrap()
        );
        let plain = doc.replace(r#" datatype="http://www.w3.org/2001/XMLSchema#double""#, "");
        assert_ne!(
            parse_srx(&plain).unwrap(),
            parse_srx(&plain.replace("<literal>", "<literal xml:lang=\"en\">")).unwrap()
        );
        let empty = plain.replace("<literal>0.0</literal>", "<literal/>");
        assert_eq!(parse_srx(&empty).unwrap().rows[0]["x"].value, "");
        assert_ne!(
            parse_srx(&empty).unwrap(),
            parse_srx(&empty.replace(r#"<binding name="x"><literal/></binding>"#, "")).unwrap()
        );
        assert!(parse_srx("<sparql><head/><results>").is_err());
        assert!(parse_srx("<not-results/>").is_err());
    }

    #[test]
    fn alternatives_accept_a_later_answer_and_reject_broken_fixtures() {
        let dir = std::env::temp_dir().join(format!(
            "sparq-gsb-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("gsb_answers")).unwrap();
        let primary = dir.join("gsb_answers/query-r01.srx");
        fs::write(&primary, SRX).unwrap();
        fs::write(
            dir.join("gsb_answers/query-r01-alternative-1.srx"),
            SRX.replace("#A", "#B"),
        )
        .unwrap();
        fs::write(
            dir.join("gsb_answers/query-r01-alternative-2.srx"),
            SRX.replace("#A", "#C"),
        )
        .unwrap();
        let actual = parse_srx(&SRX.replace("#A", "#C")).unwrap();
        let expected = expected_answers(&dir, "query-r01").unwrap();
        assert_ne!(expected[0], actual);
        assert!(expected.contains(&actual));
        fs::remove_file(dir.join("gsb_answers/query-r01-alternative-1.srx")).unwrap();
        // Upstream stops at the first absent numbered alternative.
        assert!(!expected_answers(&dir, "query-r01")
            .unwrap()
            .contains(&actual));
        fs::write(&primary, "<broken/>").unwrap();
        assert!(expected_answers(&dir, "query-r01").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

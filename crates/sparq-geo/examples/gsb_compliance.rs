//! sq-ql2iy — the GeoSPARQL Compliance Benchmark (GSB) runner.
//!
//! Runs sparq through the **206-query GeoSPARQL Compliance Benchmark**
//! (Jovanovik, Homburg, Spasić — *A GeoSPARQL Compliance Benchmark*, ISPRS IJGI
//! 10(7):487, 2021; arXiv:2102.06139) and reports the benchmark's own two
//! metrics: **correct answers** (of 206) and the **requirements-weighted
//! compliance percentage**.
//!
//! Why: `research/gap-conformance-cross-engine-2026-07.md` §3.7 scored the
//! GeoSPARQL row **NOT-COMPARABLE** — sparq ratchets its own 197-assertion
//! DE-9IM battery, which is a different unit from the published percentages
//! (Fuseki-geosparql 82.75 %, GraphDB 69.75 %, …). This runner puts sparq on the
//! published axis.
//!
//! ```sh
//! GSB="$(bench/geo/gsb.sh)"
//! cargo run --release -p sparq-geo --features geosparql_rewrite,geof_accessors \
//!     --example gsb_compliance -- "$GSB"
//! ```
//!
//! The benchmark artifact is GPL-2.0 and is NEVER vendored — `bench/geo/gsb.sh`
//! is the pinned gather-only recipe (see its header).
//!
//! ## System under test
//!
//! The benchmark scores a *system*, so the runner drives one sparq GeoSPARQL
//! stack uniformly across all 206 queries — nothing is special-cased per query:
//!
//! * `sparq-core` RDF/XML load of the benchmark dataset;
//! * `sparq-reason` RDFS materialisation (the benchmark's RDFSE requirements
//!   R25–R27 test exactly "materialised **and** inferred" triples);
//! * `sparq-geo`'s `geof:` [`FunctionRegistry`](sparq_engine::FunctionRegistry)
//!   including the opt-in `geof_accessors` functions;
//! * the standard `sparq_engine` query entry point, or `sparq-geo`'s opt-in
//!   `geosparql_rewrite` one.
//!
//! Both of those last two axes are runtime toggles (`GSB_RDFS=0`,
//! `GSB_REWRITE=0`) because both change the score materially — a compliance
//! number is meaningless without naming the configuration that produced it. The
//! measured matrix is recorded in
//! `research/gap-conformance-cross-engine-2026-07.md` §3.7.
//!
//! ## Scoring — faithful to the benchmark's own harness
//!
//! * **Weights** (`plan`) reproduce the benchmark's scoring table: each of the
//!   30 requirements is worth `1/30`, split evenly over its query groups, and a
//!   4-query serialisation group splits `1/3, 1/3, 1/6, 1/6` (the WKT/WKT and
//!   GML/GML forms weigh double the two mixed forms). R17 has no query and is
//!   credited iff the system scores above zero — the upstream rule.
//! * **Answer comparison** reproduces the upstream evaluation module: a result
//!   matches iff its variable list and its *ordered* row list equal the expected
//!   answer's, after the upstream normalisations — `geo:wktLiteral` values have
//!   all whitespace removed and are lower-cased, `geo:gmlLiteral` values go
//!   through `canonical_xml`, a **bounded** GML normaliser (NOT an
//!   implementation of XML C14N — its doc comment states exactly what it folds,
//!   what it refuses, and the one fold it does not do). Any of a query's
//!   `-alternative-N.srx` answers counts.
//! * A query whose evaluation **errors** (unsupported function, parse failure)
//!   is simply an incorrect answer, exactly as an endpoint error would be.
//!
//! Output is one TSV row per query (`id`, status, weight) followed by the two
//! summary metrics; the exit status is 0 whenever the run completed, since the
//! score is a measurement, not a gate.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use oxrdf::Term;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use sparq_core::Graph;

/// `geo:wktLiteral` — whitespace-insensitive, case-insensitive under the
/// benchmark's normalisation.
const WKT_LITERAL: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
/// `geo:gmlLiteral` — XML-normalised under the benchmark's normalisation.
const GML_LITERAL: &str = "http://www.opengis.net/ont/geosparql#gmlLiteral";
/// `xsd:string`: RDF 1.1 makes a plain literal and an `xsd:string` literal the
/// same term, but the two serialisations differ, so both sides drop it.
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// One benchmark query: its file stem and its share of the total score.
struct Task {
    id: String,
    weight: f64,
}

/// The benchmark's scoring table, reproduced structurally.
///
/// Every requirement is worth `1/30`. `SERIALISATION_SPLIT` is the weighting the
/// benchmark gives a 4-query group that tests one operation over the WKT/WKT,
/// GML/GML, WKT/GML and GML/WKT serialisation pairs.
const SERIALISATION_SPLIT: [f64; 4] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0];

fn plan() -> Vec<Task> {
    let req = 1.0 / 30.0;
    let mut tasks: Vec<Task> = Vec::new();
    let mut push = |id: String, weight: f64| tasks.push(Task { id, weight });

    // Requirements answered by a single query.
    for n in [1u32, 2, 3, 7, 10, 11, 12, 14, 15, 18, 27] {
        push(format!("query-r{:02}", n), req);
    }
    // Requirements answered by a flat, evenly-weighted group.
    for (n, count) in [
        (4u32, 8u32),
        (5, 8),
        (6, 8),
        (8, 2),
        (9, 6),
        (13, 2),
        (16, 2),
        (20, 2),
        (25, 3),
        (26, 2),
        (28, 8),
        (29, 8),
        (30, 8),
    ] {
        for i in 1..=count {
            push(format!("query-r{:02}-{}", n, i), req / f64::from(count));
        }
    }
    // R21 (geof:relate) is a single serialisation group.
    for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
        push(format!("query-r21-{}", i + 1), req * share);
    }
    // R19 (the non-topological geof: functions): nine groups, four of which take
    // one argument (2 serialisations) and five two (4 serialisation pairs).
    for g in 1u32..=9 {
        let arity2 = matches!(g, 1 | 4 | 5 | 6 | 7);
        if arity2 {
            for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
                push(format!("query-r19-{}-{}", g, i + 1), req / 9.0 * share);
            }
        } else {
            for i in 1..=2 {
                push(format!("query-r19-{}-{}", g, i), req / 9.0 * 0.5);
            }
        }
    }
    // R22/R23/R24 (the geof: sf / eh / rcc8 topology functions): eight relations,
    // each a serialisation group.
    for n in [22u32, 23, 24] {
        for g in 1u32..=8 {
            for (i, share) in SERIALISATION_SPLIT.iter().enumerate() {
                push(format!("query-r{}-{}-{}", n, g, i + 1), req / 8.0 * share);
            }
        }
    }
    tasks
}

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
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase();
            }
            Some(GML_LITERAL) => {
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
        self
    }
}

/// An XML name (element or attribute), which the benchmark's literals encode as
/// UTF-8. Lossy decoding is deliberately not used: it would map two different
/// byte sequences onto the same `U+FFFD` name.
fn xml_name(raw: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(raw).map_err(|e| format!("xml name: {}", e))
}

/// The **bounded** normaliser the answer comparison runs over `geo:gmlLiteral`
/// values. It is not an implementation of XML C14N and is deliberately not
/// described as one: the benchmark corpus is a gather-only GPL-2.0 download, so
/// no differential test against the upstream harness's canonicaliser can be run
/// from this tree, and a C14N claim would be unverified.
///
/// What the comparison actually needs is weaker, and is what this provides: a
/// deterministic function of the XML value, applied identically to both sides,
/// that folds the formatting differences two serialisers legitimately disagree
/// on — and folds nothing else, so two different literals cannot silently
/// collapse onto one string. It folds:
///
/// * the XML declaration (not part of the value);
/// * `<a/>` vs `<a></a>` — the reader expands empty elements, so both spell out
///   the start/end pair, as C14N does;
/// * attribute order — attributes are re-emitted in a fixed order (namespace
///   declarations first, then attributes, each sorted; only determinism is
///   load-bearing, since both sides go through this same function);
/// * entity and character references (`&amp;`, `&#38;`, CDATA) — resolved, then
///   re-escaped with one escape set, in text and in attribute values alike, so
///   the *spelling* of a character is not a difference but a `&lt;` in text
///   still cannot pass for real markup;
/// * pretty-printing — inter-element whitespace disappears and whitespace runs
///   inside character data collapse to one space (a GML coordinate list is
///   whitespace-separated).
///
/// Everything else is OUT OF BOUNDS and returns `Err`: any XML error (a
/// mismatched or missing end tag, non-UTF-8 bytes), a DTD, a processing
/// instruction, a comment, or an unresolvable entity. [`Binding::normalized`]
/// then compares that literal verbatim rather than as a partial parse.
///
/// The one fold it does NOT do, and does not claim: the namespace axis. A prefix
/// is compared lexically together with its `xmlns:` declaration, so re-binding
/// the same namespace URI to a different prefix reads as a difference.
fn canonical_xml(s: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut reader = Reader::from_str(s.trim());
    reader.config_mut().expand_empty_elements = true;

    // One run of character data can arrive as several Text / CData / GeneralRef
    // events, so it is accumulated and written out at the next tag boundary. A
    // run that is only whitespace is pretty-printing and disappears.
    let mut text = String::new();
    // The reader reports a stray end tag, but runs off the end of an UNCLOSED
    // one without complaint, so element depth is tracked here.
    let mut depth: i64 = 0;
    fn flush(out: &mut String, text: &mut String) {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text.clear();
        if !collapsed.is_empty() {
            out.push_str(&escape(collapsed.as_str()));
        }
    }

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("xml: {}", e)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                flush(&mut out, &mut text);
                depth += 1;
                // `rank` puts the namespace declarations first, C14N's order.
                let mut attrs: Vec<(u8, String, String)> = Vec::new();
                for a in e.attributes() {
                    let a = a.map_err(|err| format!("xml attribute: {}", err))?;
                    let key = xml_name(a.key.as_ref())?.to_string();
                    let value = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|err| format!("xml attribute value: {}", err))?
                        .into_owned();
                    attrs.push((u8::from(!key.starts_with("xmlns")), key, value));
                }
                attrs.sort();
                let name = e.name();
                let _ = write!(out, "<{}", xml_name(name.as_ref())?);
                for (_, k, v) in attrs {
                    let _ = write!(out, " {}=\"{}\"", k, escape(v.as_str()));
                }
                out.push('>');
            }
            Ok(Event::End(e)) => {
                flush(&mut out, &mut text);
                depth -= 1;
                let name = e.name();
                let _ = write!(out, "</{}>", xml_name(name.as_ref())?);
            }
            Ok(Event::Text(t)) => {
                text.push_str(&t.decode().map_err(|e| format!("xml text: {}", e))?);
            }
            Ok(Event::CData(t)) => {
                text.push_str(&t.decode().map_err(|e| format!("xml cdata: {}", e))?);
            }
            Ok(Event::GeneralRef(r)) => text.push_str(&resolve_ref(&r)?),
            Ok(Event::Decl(_)) => {}
            // A DTD, a processing instruction or a comment: out of bounds.
            Ok(other) => return Err(format!("unsupported XML construct: {:?}", other)),
        }
    }
    if depth != 0 {
        return Err(format!("xml: {} unclosed element(s)", depth));
    }
    flush(&mut out, &mut text);
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
struct ResultSet {
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
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut vars: Vec<String> = Vec::new();
    let mut rows: Vec<BTreeMap<String, Binding>> = Vec::new();
    let mut row: BTreeMap<String, Binding> = BTreeMap::new();
    // The binding currently open: its variable name plus the term being read.
    let mut binding_name: Option<String> = None;
    let mut term: Option<Binding> = None;
    let mut text = String::new();

    let attr = |e: &quick_xml::events::BytesStart<'_>, want: &str| -> Option<String> {
        e.attributes().flatten().find_map(|a| {
            (a.key.as_ref() == want.as_bytes())
                .then(|| String::from_utf8_lossy(a.value.as_ref()).into_owned())
        })
    };

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("srx: {}", e)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                match e.name().as_ref() {
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
                if let Ok(d) = t.decode() {
                    text.push_str(&d);
                }
            }
            Ok(Event::CData(t)) => text.push_str(&String::from_utf8_lossy(t.as_ref())),
            // The reader reports `&amp;` / `&#38;` as its own event, so an
            // escaped value — every `geo:gmlLiteral` answer is one — loses
            // characters unless the reference is resolved back into the text.
            Ok(Event::GeneralRef(r)) => text.push_str(&resolve_ref(&r)?),
            Ok(Event::End(e)) => match e.name().as_ref() {
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
            },
            Ok(_) => {}
        }
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
fn load_dataset(dir: &Path, rdfs: bool) -> Graph {
    let path = dir.join("gsb_dataset/dataset.rdf");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
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
fn run_query(graph: &Graph, sparql: &str, rewrite: bool) -> Result<ResultSet, String> {
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
fn expected_answers(dir: &Path, id: &str) -> Vec<ResultSet> {
    let mut out = Vec::new();
    let primary = dir.join(format!("gsb_answers/{}.srx", id));
    if let Ok(text) = fs::read_to_string(&primary) {
        match parse_srx(&text) {
            Ok(rs) => out.push(rs),
            Err(e) => eprintln!("[gsb] WARN {}: {}", primary.display(), e),
        }
    }
    for k in 1.. {
        let alt = dir.join(format!("gsb_answers/{}-alternative-{}.srx", id, k));
        let Ok(text) = fs::read_to_string(&alt) else { break };
        match parse_srx(&text) {
            Ok(rs) => out.push(rs),
            Err(e) => eprintln!("[gsb] WARN {}: {}", alt.display(), e),
        }
    }
    out
}

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            eprintln!("usage: gsb_compliance <gsb-resources-dir>   (see bench/geo/gsb.sh)");
            std::process::exit(2);
        }
    };

    let tasks = plan();
    assert_eq!(tasks.len(), 206, "the benchmark is 206 queries");
    let total_weight: f64 = tasks.iter().map(|t| t.weight).sum();
    // 29 of the 30 requirements carry queries; R17 is credited separately.
    assert!(
        (total_weight - 29.0 / 30.0).abs() < 1e-9,
        "scoring table must sum to 29/30, got {}",
        total_weight
    );

    let rdfs = std::env::var("GSB_RDFS").as_deref() != Ok("0");
    let rewrite = std::env::var("GSB_REWRITE").as_deref() != Ok("0");
    let graph = load_dataset(&dir, rdfs);
    eprintln!(
        "[gsb] dataset loaded: {} triples (rdfs={}, rewrite={})",
        graph.len(),
        rdfs,
        rewrite
    );

    // `GSB_DEBUG=<query-id-prefix>` dumps the actual-vs-expected result sets for
    // the matching failures — how a FAIL is triaged into "engine gap" vs
    // "serialisation difference" without re-deriving the harness.
    let debug = std::env::var("GSB_DEBUG").ok();
    let mut correct = 0usize;
    let mut score = 0.0f64;
    println!("query\tstatus\tweight");
    for task in &tasks {
        let qpath = dir.join(format!("gsb_queries/{}.rq", task.id));
        let Ok(sparql) = fs::read_to_string(&qpath) else {
            println!("{}\tMISSING\t{:.6}", task.id, task.weight);
            continue;
        };
        let expected = expected_answers(&dir, &task.id);
        if expected.is_empty() {
            println!("{}\tNO-ANSWER\t{:.6}", task.id, task.weight);
            continue;
        }
        let status = match run_query(&graph, &sparql, rewrite) {
            Err(e) => {
                eprintln!("[gsb] {}: {}", task.id, e);
                "ERROR"
            }
            Ok(actual) => {
                if expected.contains(&actual) {
                    correct += 1;
                    score += task.weight;
                    "PASS"
                } else {
                    if debug.as_deref().is_some_and(|d| task.id.starts_with(d)) {
                        if let Ok(p) = sparq_geo::geosparql_rewrite(&sparql) {
                            eprintln!("[gsb] {} rewritten: {}", task.id, p.query());
                        }
                        eprintln!("[gsb] {} actual:   {:?}", task.id, actual);
                        for e in &expected {
                            eprintln!("[gsb] {} expected: {:?}", task.id, e);
                        }
                    }
                    "FAIL"
                }
            }
        };
        println!("{}\t{}\t{:.6}", task.id, status, task.weight);
    }

    // The upstream rule: R17 has no query, and is credited iff the system scored
    // above zero on the rest.
    if score > 0.0 {
        score += 1.0 / 30.0;
    }
    println!("gsb_correct_answers\t{}\t{}", correct, tasks.len());
    println!("gsb_compliance_pct\t{:.2}", score * 100.0);
}

/// The corpus-free half of the harness — the scoring table and the answer
/// comparator. These run under a plain `cargo test --all-features`, so the parts
/// that decide whether a run is scored CORRECTLY are covered even though the
/// benchmark artifact itself is a gather-only download.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_table_reproduces_the_benchmark() {
        let tasks = plan();
        assert_eq!(tasks.len(), 206, "the benchmark is 206 queries");
        let total: f64 = tasks.iter().map(|t| t.weight).sum();
        // 29 requirements carry queries; R17 has none and is credited separately.
        assert!((total - 29.0 / 30.0).abs() < 1e-12, "table sums to {}", total);

        let w = |id: &str| {
            tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("no task {}", id))
                .weight
        };
        // A whole requirement in one query.
        assert!((w("query-r01") - 1.0 / 30.0).abs() < 1e-12);
        // An evenly split eight-query requirement.
        assert!((w("query-r04-8") - 1.0 / 30.0 / 8.0).abs() < 1e-12);
        // A serialisation group: the two mixed WKT/GML forms weigh half.
        assert!((w("query-r22-1-1") - 1.0 / 30.0 / 8.0 / 3.0).abs() < 1e-12);
        assert!((w("query-r22-1-3") - 1.0 / 30.0 / 8.0 / 6.0).abs() < 1e-12);
        // R19's nine groups: a two-serialisation group splits evenly.
        assert!((w("query-r19-2-1") - 1.0 / 30.0 / 9.0 / 2.0).abs() < 1e-12);
        assert!((w("query-r19-1-3") - 1.0 / 30.0 / 9.0 / 6.0).abs() < 1e-12);
        // The ids must be the benchmark's own file stems.
        assert!(tasks.iter().any(|t| t.id == "query-r30-8"));
        assert!(!tasks.iter().any(|t| t.id.starts_with("query-r17")));
    }

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
        assert_eq!(rs.rows[0]["f"].value, "http://example.org/ApplicationSchema#A");
        // The WKT normalisation: whitespace removed, lower-cased.
        assert_eq!(rs.rows[0]["wkt"].value, "polygon((-83.634.1,-83.234.1))");
    }

    #[test]
    fn wkt_answers_compare_modulo_whitespace_and_case_only() {
        let spaced = SRX.replace("Polygon((-83.6 34.1, -83.2 34.1))", "POLYGON((-83.6  34.1,-83.2 34.1))");
        assert_eq!(parse_srx(SRX).unwrap(), parse_srx(&spaced).unwrap());
        // …but a different COORDINATE is a different answer.
        let moved = SRX.replace("-83.2 34.1", "-83.3 34.1");
        assert_ne!(parse_srx(SRX).unwrap(), parse_srx(&moved).unwrap());
    }

    #[test]
    fn row_order_and_variable_order_are_significant() {
        let two = SRX.replace(
            "</results>",
            r#"<result><binding name="f"><uri>http://example.org/ApplicationSchema#B</uri></binding></result></results>"#,
        );
        let swapped = SRX
            .replace(
                "</results>",
                r#"<result><binding name="f"><uri>http://example.org/ApplicationSchema#B</uri></binding></result></results>"#,
            )
            .replace("#A", "#Z");
        assert_ne!(parse_srx(&two).unwrap(), parse_srx(&swapped).unwrap());
        assert_ne!(parse_srx(&two).unwrap().rows.len(), parse_srx(SRX).unwrap().rows.len());
    }

    /// A GML literal, normalised — panics if it is out of `canonical_xml`'s
    /// bounds, which is what the in-bounds fixtures below are asserting.
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
    fn gml_answers_compare_modulo_xml_formatting() {
        let one = "<gml:Point xmlns:gml=\"http://www.opengis.net/ont/gml\"><gml:pos>1 2</gml:pos></gml:Point>";
        let two = "<gml:Point   xmlns:gml=\"http://www.opengis.net/ont/gml\">\n  <gml:pos>1   2</gml:pos>\n</gml:Point>";
        assert_eq!(gml(one), gml(two));
        // A different coordinate must still differ.
        assert_ne!(gml(one), gml(&one.replace("1 2", "1 3")));
        // …and so must a different namespace URI behind the same prefix.
        assert_ne!(gml(one), gml(&one.replace("ont/gml", "ont/other")));
    }

    #[test]
    fn empty_elements_normalise_to_the_start_end_pair() {
        assert_eq!(gml("<a><b/></a>"), gml("<a><b></b></a>"));
        assert_eq!(gml("<a><b attr=\"v\"/></a>"), gml("<a><b attr=\"v\"></b></a>"));
        // An empty element is still not the same as an absent one.
        assert_ne!(gml("<a><b/></a>"), gml("<a></a>"));
    }

    #[test]
    fn attribute_and_namespace_order_is_not_a_difference() {
        let a = r#"<gml:Point xmlns:gml="G" srsDimension="2" srsName="S"/>"#;
        let b = r#"<gml:Point srsName="S" srsDimension="2" xmlns:gml="G"/>"#;
        assert_eq!(gml(a), gml(b));
        // A namespace declaration sorts ahead of a plain attribute (C14N's
        // order); only determinism is load-bearing, but pin it so a reordering
        // of the emitter is a deliberate change.
        assert!(gml(a).starts_with(r#"<gml:Point xmlns:gml="G" srsDimension="2""#));
        // An attribute VALUE, name, or count is still a difference.
        assert_ne!(gml(a), gml(&a.replace(r#"srsName="S""#, r#"srsName="T""#)));
        assert_ne!(gml(a), gml(&a.replace("srsName", "srsname")));
        assert_ne!(gml(a), gml(r#"<gml:Point xmlns:gml="G" srsDimension="2"/>"#));
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
        assert_ne!(gml(r#"<a t="x&quot; u=&quot;y"/>"#), gml(r#"<a t="x" u="y"/>"#));
    }

    #[test]
    fn out_of_bounds_gml_is_compared_verbatim_never_as_a_partial_parse() {
        // Malformed, and the constructs the normaliser does not model.
        assert!(canonical_xml("<gml:Point><gml:pos>1 2</gml:Point>").is_err());
        assert!(canonical_xml("<a>").is_err());
        assert!(canonical_xml("<!DOCTYPE a><a/>").is_err());
        assert!(canonical_xml("<a><!-- c --></a>").is_err());
        assert!(canonical_xml("<a><?pi go?></a>").is_err());
        assert!(canonical_xml("<a>&noSuchEntity;</a>").is_err());
        // The regression this guards: two literals that agree only on the
        // prefix before the XML error must NOT compare equal.
        assert_ne!(
            gml_binding("<a>x</b>keep-me"),
            gml_binding("<a>x</b>different"),
        );
        // …while an out-of-bounds literal still equals ITSELF, so a corpus that
        // uses a construct outside the bound degrades to exact-string matching
        // rather than to a guaranteed failure.
        assert_eq!(gml_binding("<a><!-- c --></a>"), gml_binding("<a><!-- c --></a>"));
    }

    #[test]
    fn srx_entity_references_survive_the_parse() {
        // Inside an `.srx` answer a GML literal is XML-escaped; dropping those
        // references mangles every expected GML answer before the comparator
        // ever sees it.
        let srx = format!(
            r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#">
 <head><variable name="g"/></head>
 <results><result><binding name="g"><literal datatype="{}">&lt;gml:Point xmlns:gml="G"&gt;&lt;gml:pos&gt;1 2&lt;/gml:pos&gt;&lt;/gml:Point&gt;</literal></binding></result></results>
</sparql>"#,
            GML_LITERAL
        );
        let rs = parse_srx(&srx).expect("parse");
        assert_eq!(
            rs.rows[0]["g"].value,
            gml(r#"<gml:Point xmlns:gml="G"><gml:pos>1 2</gml:pos></gml:Point>"#),
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
}

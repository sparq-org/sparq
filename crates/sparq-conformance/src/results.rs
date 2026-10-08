//! Expected-result parsing: SPARQL XML results (`.srx`), JSON results (`.srj`),
//! TSV results (`.tsv`), and DAWG result-set graphs (`.ttl` / `.rdf` using the
//! `rs:` vocabulary).

use crate::rdf::{as_node, MiniGraph};
use oxrdf::{Literal, NamedNode, Term};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::borrow::Cow;
use std::path::Path;

const RS: &str = "http://www.w3.org/2001/sw/DataAccess/tests/result-set#";

// [OPUS-4.8] quick-xml 0.40 removed `BytesText::unescape()` / deprecated
// `Attribute::unescape_value()`. Both used to decode + resolve the XML
// predefined entities (`&amp;`/`&lt;`/`&gt;`/`&apos;`/`&quot;` and numeric
// char-refs) without applying attribute-value whitespace folding. We replicate
// that EXACT semantics here — `quick_xml::escape::unescape` is
// `unescape_with(resolve_predefined_entity)`, the same resolver the old
// methods called — so the SPARQL-XML-results parse path is byte-for-byte
// unchanged (CONFORMANCE-CRITICAL: this file feeds the W3C ratchet). We
// deliberately do NOT switch to 0.40's `normalized_value`/`xml_content`, which
// would add AVN whitespace folding / EOL normalization the old path never did.
fn unescape_utf8(raw: &[u8]) -> Result<String, String> {
    let s = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
    quick_xml::escape::unescape(s)
        .map(Cow::into_owned)
        .map_err(|e| e.to_string())
}

/// One solution: `(variable name, bound term)` pairs.
pub type Binding = Vec<(String, Term)>;

#[derive(Debug)]
pub enum Expected {
    Bindings {
        /// Declared result variables (may be empty for `rs:` graphs that omit them).
        vars: Vec<String>,
        rows: Vec<Binding>,
        /// True when the result set carried explicit `rs:index` ordering.
        indexed: bool,
    },
    Boolean(bool),
}

pub fn parse_expected(path: &Path) -> Result<Expected, String> {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "srx" => parse_srx(path),
        "srj" => parse_srj(path),
        "tsv" => parse_tsv(path),
        "ttl" | "rdf" | "nt" => parse_rs_graph(path),
        // .csv is intentionally unsupported: CSV results are lossy (no term
        // kinds/datatypes), so equality against them is not meaningful here.
        ext => Err(format!("unsupported result format: .{ext}")),
    }
}

/// SPARQL Query Results TSV Format: header `?v1\t?v2…`, then one row per line,
/// each cell a Turtle-encoded term (or empty = unbound). Cells are parsed by
/// wrapping them as the object of a dummy Turtle triple, which also covers the
/// bare-number abbreviations the format allows.
fn parse_tsv(path: &Path) -> Result<Expected, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| format!("{}: empty TSV", path.display()))?;
    let vars: Vec<String> = header
        .split('\t')
        .map(|v| v.trim().trim_start_matches('?').to_string())
        .filter(|v| !v.is_empty())
        .collect();
    let mut rows: Vec<Binding> = Vec::new();
    for (lineno, line) in lines.enumerate() {
        // An empty cell is an unbound variable, so an all-tabs (or, for a
        // single variable, empty) line is a row with no bindings — `split`
        // yields the empty cells; nothing to special-case.
        let mut binding: Binding = Vec::new();
        for (i, cell) in line.split('\t').enumerate() {
            if cell.is_empty() {
                continue; // unbound
            }
            let var = vars.get(i).ok_or_else(|| {
                format!(
                    "{}: row {} has more cells than variables",
                    path.display(),
                    lineno + 2
                )
            })?;
            binding.push((
                var.clone(),
                parse_tsv_term(cell)
                    .map_err(|e| format!("{}: row {}: {e}", path.display(), lineno + 2))?,
            ));
        }
        rows.push(binding);
    }
    Ok(Expected::Bindings {
        vars,
        rows,
        // `indexed` flags rs:index graphs only; TSV row order is honoured via
        // the runner's extension check (srx/srj/tsv are order-preserving).
        indexed: false,
    })
}

fn parse_tsv_term(cell: &str) -> Result<Term, String> {
    let doc = format!("<urn:tsv:s> <urn:tsv:p> {cell} .");
    let mut triples = Vec::new();
    for t in oxttl::TurtleParser::new().for_slice(doc.as_bytes()) {
        triples.push(t.map_err(|e| format!("bad TSV term `{cell}`: {e}"))?);
    }
    match triples.len() {
        1 => Ok(triples.pop().expect("len checked").object),
        n => Err(format!("bad TSV term `{cell}`: parsed to {n} triples")),
    }
}

/// SPARQL Query Results JSON Format.
fn parse_srj(path: &Path) -> Result<Expected, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(b) = v.get("boolean").and_then(|b| b.as_bool()) {
        return Ok(Expected::Boolean(b));
    }
    let vars: Vec<String> = v
        .pointer("/head/vars")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut rows: Vec<Binding> = Vec::new();
    for sol in v
        .pointer("/results/bindings")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let mut row: Binding = Vec::new();
        for (var, val) in sol.as_object().into_iter().flatten() {
            row.push((var.clone(), srj_term(val)?));
        }
        rows.push(row);
    }
    Ok(Expected::Bindings {
        vars,
        rows,
        indexed: false,
    })
}

/// One SRJ term object, including SPARQL 1.2 `"type": "triple"` whose value is
/// `{subject, predicate, object}` (object may nest another triple term).
fn srj_term(val: &serde_json::Value) -> Result<Term, String> {
    let get = |k: &str| val.get(k).and_then(|s| s.as_str());
    match get("type") {
        Some("uri") => make_term(
            "uri",
            None,
            None,
            None,
            get("value").unwrap_or_default().to_string(),
        ),
        Some("bnode") => make_term(
            "bnode",
            None,
            None,
            None,
            get("value").unwrap_or_default().to_string(),
        ),
        Some("triple") => {
            let v = val
                .get("value")
                .ok_or_else(|| "triple term without value".to_string())?;
            let part = |k: &str| -> Result<Term, String> {
                srj_term(v.get(k).ok_or_else(|| format!("triple term without {k}"))?)
            };
            let subject = match part("subject")? {
                Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
                Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
                other => return Err(format!("invalid triple-term subject: {other}")),
            };
            let predicate = match part("predicate")? {
                Term::NamedNode(n) => n,
                other => return Err(format!("invalid triple-term predicate: {other}")),
            };
            Ok(Term::Triple(Box::new(oxrdf::Triple {
                subject,
                predicate,
                object: part("object")?,
            })))
        }
        _ => make_term(
            "literal",
            get("xml:lang").map(String::from),
            get("its:dir").map(String::from),
            get("datatype").map(String::from),
            get("value").unwrap_or_default().to_string(),
        ),
    }
}

/// SPARQL Query Results XML Format.
fn parse_srx(path: &Path) -> Result<Expected, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);

    let mut vars: Vec<String> = Vec::new();
    let mut rows: Vec<Binding> = Vec::new();
    let mut cur_row: Binding = Vec::new();
    let mut cur_var: Option<String> = None;
    // Value element currently open: (kind, lang, its:dir, datatype, text).
    #[allow(clippy::type_complexity)]
    let mut cur_val: Option<(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = None;
    // SPARQL 1.2 `<triple>` nesting: each frame is (active slot, [s, p, o]).
    let mut triple_stack: Vec<(usize, [Option<Term>; 3])> = Vec::new();
    let mut boolean: Option<bool> = None;
    let mut in_boolean = false;

    // Routes a finished term either into the enclosing `<triple>` frame's
    // active slot or, at top level, into the current row binding.
    fn commit(
        term: Term,
        triple_stack: &mut [(usize, [Option<Term>; 3])],
        cur_row: &mut Binding,
        cur_var: &Option<String>,
    ) {
        if let Some((slot, parts)) = triple_stack.last_mut() {
            parts[*slot] = Some(term);
        } else if let Some(var) = cur_var.clone() {
            cur_row.push((var, term));
        }
    }

    fn set_slot(triple_stack: &mut [(usize, [Option<Term>; 3])], slot: usize) {
        if let Some((s, _)) = triple_stack.last_mut() {
            *s = slot;
        }
    }

    loop {
        match reader
            .read_event()
            .map_err(|e| format!("{}: {e}", path.display()))?
        {
            Event::Eof => break,
            ev @ (Event::Start(_) | Event::Empty(_)) => {
                let is_empty = matches!(ev, Event::Empty(_));
                let e = match &ev {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let name = e.local_name();
                let name = std::str::from_utf8(name.as_ref()).unwrap_or("").to_string();
                let attr = |key: &str| -> Option<String> {
                    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
                        let k = std::str::from_utf8(a.key.as_ref()).ok()?;
                        // Matches both `datatype` and `xml:lang`-style keys.
                        if k == key || k.ends_with(&format!(":{key}")) {
                            Some(unescape_utf8(&a.value).ok()?)
                        } else {
                            None
                        }
                    })
                };
                match name.as_str() {
                    "variable" => {
                        if let Some(v) = attr("name") {
                            vars.push(v);
                        }
                    }
                    "result" => cur_row = Vec::new(),
                    "binding" => cur_var = attr("name"),
                    "uri" | "bnode" => cur_val = Some((name, None, None, None, String::new())),
                    "literal" => {
                        cur_val = Some((
                            name,
                            attr("lang"),
                            attr("dir"),
                            attr("datatype"),
                            String::new(),
                        ))
                    }
                    "triple" => triple_stack.push((0, [None, None, None])),
                    "subject" => set_slot(&mut triple_stack, 0),
                    "predicate" => set_slot(&mut triple_stack, 1),
                    "object" => set_slot(&mut triple_stack, 2),
                    "boolean" => in_boolean = true,
                    _ => {}
                }
                // Self-closing value elements (e.g. `<literal/>`) get no End event:
                // commit the (empty-text) term right away.
                if is_empty {
                    if let Some((kind, lang, dir, dt, text)) = cur_val.take() {
                        commit(
                            make_term(&kind, lang, dir, dt, text)?,
                            &mut triple_stack,
                            &mut cur_row,
                            &cur_var,
                        );
                    }
                }
            }
            Event::Text(t) => {
                // [OPUS-4.8] 0.40: `decode()` (no EOL normalization, matching the
                // old `unescape`) then resolve predefined entities — see `unescape_utf8`.
                let decoded = t.decode().map_err(|e| e.to_string())?;
                let s = quick_xml::escape::unescape(&decoded)
                    .map_err(|e| e.to_string())?
                    .into_owned();
                if in_boolean {
                    boolean = Some(s.trim() == "true");
                } else if let Some(v) = cur_val.as_mut() {
                    v.4.push_str(&s);
                }
            }
            Event::CData(t) => {
                if let Some(v) = cur_val.as_mut() {
                    v.4.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"uri" | b"bnode" | b"literal" => {
                        if let Some((kind, lang, dir, dt, text)) = cur_val.take() {
                            commit(
                                make_term(&kind, lang, dir, dt, text)?,
                                &mut triple_stack,
                                &mut cur_row,
                                &cur_var,
                            );
                        }
                    }
                    b"triple" => {
                        let Some((_, [s, p, o])) = triple_stack.pop() else {
                            return Err(format!("{}: stray </triple>", path.display()));
                        };
                        let subject = match s {
                            Some(Term::NamedNode(n)) => oxrdf::NamedOrBlankNode::NamedNode(n),
                            Some(Term::BlankNode(b)) => oxrdf::NamedOrBlankNode::BlankNode(b),
                            other => {
                                return Err(format!(
                                    "{}: invalid triple-term subject: {other:?}",
                                    path.display()
                                ))
                            }
                        };
                        let predicate = match p {
                            Some(Term::NamedNode(n)) => n,
                            other => {
                                return Err(format!(
                                    "{}: invalid triple-term predicate: {other:?}",
                                    path.display()
                                ))
                            }
                        };
                        let object = o.ok_or_else(|| {
                            format!("{}: triple term without object", path.display())
                        })?;
                        commit(
                            Term::Triple(Box::new(oxrdf::Triple {
                                subject,
                                predicate,
                                object,
                            })),
                            &mut triple_stack,
                            &mut cur_row,
                            &cur_var,
                        );
                    }
                    b"binding" => cur_var = None,
                    b"result" => rows.push(std::mem::take(&mut cur_row)),
                    b"boolean" => in_boolean = false,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if let Some(b) = boolean {
        return Ok(Expected::Boolean(b));
    }
    Ok(Expected::Bindings {
        vars,
        rows,
        indexed: false,
    })
}

fn make_term(
    kind: &str,
    lang: Option<String>,
    dir: Option<String>,
    dt: Option<String>,
    text: String,
) -> Result<Term, String> {
    Ok(match kind {
        "uri" => Term::NamedNode(NamedNode::new(text).map_err(|e| e.to_string())?),
        "bnode" => Term::BlankNode(oxrdf::BlankNode::new(text).map_err(|e| e.to_string())?),
        _ => {
            if let Some(lang) = lang {
                // SPARQL 1.2: `its:dir` makes this an rdf:dirLangString literal —
                // direction is part of the term and must survive into comparison.
                if let Some(dir) = dir {
                    let direction = match dir.as_str() {
                        "ltr" => oxrdf::BaseDirection::Ltr,
                        "rtl" => oxrdf::BaseDirection::Rtl,
                        other => return Err(format!("invalid base direction: {other}")),
                    };
                    Term::Literal(
                        Literal::new_directional_language_tagged_literal(text, lang, direction)
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    Term::Literal(
                        Literal::new_language_tagged_literal(text, lang)
                            .map_err(|e| e.to_string())?,
                    )
                }
            } else if let Some(dt) = dt {
                Term::Literal(Literal::new_typed_literal(
                    text,
                    NamedNode::new(dt).map_err(|e| e.to_string())?,
                ))
            } else {
                Term::Literal(Literal::new_simple_literal(text))
            }
        }
    })
}

/// DAWG result-set graph (`rs:` vocabulary) in Turtle or RDF/XML.
fn parse_rs_graph(path: &Path) -> Result<Expected, String> {
    let g = MiniGraph::load(path)?;
    let sets = g.subjects_with_type(&format!("{RS}ResultSet"));
    let set = sets
        .first()
        .ok_or_else(|| format!("{}: no rs:ResultSet node (graph result?)", path.display()))?;

    if let Some(Term::Literal(l)) = g.object(set, &format!("{RS}boolean")) {
        return Ok(Expected::Boolean(l.value() == "true"));
    }

    let vars: Vec<String> = g
        .objects(set, &format!("{RS}resultVariable"))
        .into_iter()
        .filter_map(|t| match t {
            Term::Literal(l) => Some(l.value().to_string()),
            _ => None,
        })
        .collect();

    // Each rs:solution is a node with rs:binding [rs:variable; rs:value] and optional rs:index.
    let mut rows: Vec<(Option<f64>, Binding)> = Vec::new();
    for sol in g.objects(set, &format!("{RS}solution")) {
        let Some(sol) = as_node(sol) else { continue };
        let mut row: Binding = Vec::new();
        for b in g.objects(&sol, &format!("{RS}binding")) {
            let Some(b) = as_node(b) else { continue };
            let var = g.str_object(&b, &format!("{RS}variable"));
            let val = g.object(&b, &format!("{RS}value"));
            if let (Some(var), Some(val)) = (var, val) {
                row.push((var, val.clone()));
            }
        }
        let index = match g.object(&sol, &format!("{RS}index")) {
            Some(Term::Literal(l)) => l.value().parse::<f64>().ok(),
            _ => None,
        };
        rows.push((index, row));
    }
    let indexed = !rows.is_empty() && rows.iter().all(|(i, _)| i.is_some());
    if indexed {
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }
    Ok(Expected::Bindings {
        vars,
        rows: rows.into_iter().map(|(_, r)| r).collect(),
        indexed,
    })
}

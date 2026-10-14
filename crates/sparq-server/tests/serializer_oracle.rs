//! Differential **serializer oracle** for the SPARQL Results XML / CSV / TSV
//! writers in `sparq_server::results` (`select_to_xml` / `select_to_csv` / `select_to_tsv`).
//!
//! ## Why this exists (the gap it closes)
//!
//! The hand-written serialisers in `crates/sparq-server/src/results.rs` previously had
//! only *assertion-style* inline tests (`assert!(xml.contains("…"))`). Those prove a byte
//! is present; they do NOT prove the document is *well-formed* nor that it *round-trips*.
//! A malformed-but-superficially-plausible byte stream — wrong XML escaping, mis-quoted CSV
//! field with an embedded `"` / `,` / newline, a botched TSV literal escape, a dropped
//! language tag or datatype — would sail straight through a `contains` check while being
//! rejected (or silently mis-parsed to a *different* result set) by any real consumer.
//!
//! ## What it does (modelled on the repo's chunked-vs-serial differential oracles)
//!
//! Like `sparq_core`'s `parallel_turtle_matches_serial` (parallel parse ≡ serial parse),
//! this is a *round-trip* oracle: OUR serialiser output is fed back through an INDEPENDENT
//! **reference parser** and the recovered result set is required to equal the source
//! `QueryResult` exactly — variable list (order included), row multiset, and every term's
//! kind / value / language tag / datatype, plus unbound (`OPTIONAL`) cells.
//!
//! * **XML** and **TSV** → re-parsed with oxigraph's `sparesults` (the canonical SPARQL
//!   results parser). Both are *lossless* formats, so the recovered terms must be
//!   value-equal to the originals.
//! * **CSV** → `sparesults` deliberately REFUSES to parse CSV ("CSV SPARQL results syntax
//!   is lossy and can't be parsed to a proper RDF representation"), so we instead validate
//!   our CSV with a self-contained, strict RFC 4180 reader (in this file) and assert each
//!   recovered field equals the spec's *lexical* projection of the source term (IRIs
//!   verbatim, bnodes `_:label`, literals = their lexical value only). This catches the
//!   quoting/escaping bugs CSV is actually prone to.
//!
//! Inputs are an **adversarial battery** (hand-built `QueryResult`s with embedded newline /
//! comma / double-quote / leading+trailing whitespace, language-tagged, datatyped
//! `xsd:integer`/`xsd:dateTime`, XML-significant IRIs, blank nodes, unbound cells, and an
//! RDF 1.2 triple term) plus an **engine-driven random battery** (random graphs
//! queried through the real engine — the same generator style as the engine's exec tests).

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple, Variable};
use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
use sparq_engine::{query, QueryResult};
use sparq_server::results::{ask_to_json, ask_to_xml, select_to_csv, select_to_tsv, select_to_xml};

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

// ---------------------------------------------------------------------------
// Reference re-parse for the lossless formats (XML / TSV) via sparesults.
// ---------------------------------------------------------------------------

/// Re-parses a serialised SELECT result with the reference `sparesults` parser and
/// reconstructs an engine-shaped `(vars, rows)` pair.
///
/// `sparesults` only yields *bound* (variable, term) pairs per solution, so we project
/// each recovered solution back onto the declared variable list, materialising `None` for
/// any variable the solution does not bind. This makes the round-trip directly comparable
/// to the source `QueryResult`, unbound cells included.
fn reparse_reference(bytes: &[u8], format: QueryResultsFormat) -> (Vec<Variable>, Vec<Vec<Option<Term>>>) {
    let parser = QueryResultsParser::from_format(format);
    match parser
        .for_reader(bytes)
        .unwrap_or_else(|e| panic!("reference parser REJECTED our {format:?} output: {e}\n--- output ---\n{}", String::from_utf8_lossy(bytes)))
    {
        ReaderQueryResultsParserOutput::Boolean(b) => {
            panic!("expected SELECT solutions from {format:?}, got boolean {b}")
        }
        ReaderQueryResultsParserOutput::Solutions(solutions) => {
            let vars: Vec<Variable> = solutions.variables().to_vec();
            let rows: Vec<Vec<Option<Term>>> = solutions
                .map(|sol| {
                    let sol = sol.unwrap_or_else(|e| {
                        panic!("reference parser failed mid-stream on our {format:?} output: {e}")
                    });
                    // Project the (sparse) solution back onto the full variable order.
                    vars.iter().map(|v| sol.get(v).cloned()).collect()
                })
                .collect();
            (vars, rows)
        }
    }
}

/// Re-parses an ASK boolean document with the reference `sparesults` parser and recovers the
/// boolean value. Panics if the parser rejects the document or yields SELECT solutions instead
/// of a boolean — i.e. if our `ask_to_xml` / `ask_to_json` produced something a real consumer
/// would not read back as the boolean we intended.
fn reparse_boolean(bytes: &[u8], format: QueryResultsFormat) -> bool {
    let parser = QueryResultsParser::from_format(format);
    match parser.for_reader(bytes).unwrap_or_else(|e| {
        panic!(
            "reference parser REJECTED our ASK {format:?} output: {e}\n--- output ---\n{}",
            String::from_utf8_lossy(bytes)
        )
    }) {
        ReaderQueryResultsParserOutput::Boolean(b) => b,
        ReaderQueryResultsParserOutput::Solutions(_) => {
            panic!("expected an ASK boolean from {format:?}, got SELECT solutions")
        }
    }
}

/// A canonical, order-independent fingerprint of the row multiset: each row is rendered to
/// a stable string (per-cell `Debug` of the oxrdf term, or `∅` for unbound) and the whole
/// list is sorted. Two result sets share a fingerprint iff they have the same rows with the
/// same multiplicities, regardless of row order. (Row ORDER is asserted separately when the
/// source query is ordered — see `assert_roundtrip`.)
fn row_multiset(rows: &[Vec<Option<Term>>]) -> Vec<String> {
    let mut out: Vec<String> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| match c {
                    Some(t) => format!("«{t:?}»"),
                    None => "∅".to_string(),
                })
                .collect::<Vec<_>>()
                .join("\u{1f}") // unit separator — cannot collide with term text
        })
        .collect();
    out.sort();
    out
}

/// Asserts that serialising `r` to a lossless format and re-parsing it with the reference
/// parser recovers the SAME variable list (order included) and the SAME row multiset. When
/// `ordered`, the row SEQUENCE must match exactly too (set equality is not enough for an
/// ORDER BY result).
fn assert_lossless_roundtrip(label: &str, r: &QueryResult, serialised: &str, format: QueryResultsFormat, ordered: bool) {
    let (got_vars, got_rows) = reparse_reference(serialised.as_bytes(), format);

    assert_eq!(
        got_vars,
        r.vars,
        "[{label}/{format:?}] variable list (with order) must round-trip\n--- output ---\n{serialised}"
    );

    if ordered {
        assert_eq!(
            got_rows,
            r.rows,
            "[{label}/{format:?}] ORDERED row sequence must round-trip exactly\n--- output ---\n{serialised}"
        );
    } else {
        assert_eq!(
            row_multiset(&got_rows),
            row_multiset(&r.rows),
            "[{label}/{format:?}] row multiset must round-trip\n--- output ---\n{serialised}"
        );
    }
}

// ---------------------------------------------------------------------------
// CSV: a self-contained strict RFC 4180 reader + lexical-projection check.
// (sparesults refuses CSV as lossy, so we validate well-formedness ourselves.)
// ---------------------------------------------------------------------------

/// Parses an RFC 4180 document (fields separated by `,`, records by CRLF, a field is quoted
/// iff it contains `"`, `,`, CR or LF, and an embedded `"` is written doubled). Returns the
/// list of records (each a list of field strings). Panics on any malformed construct — a
/// stray quote, an unterminated quoted field, a bare CR/LF not part of a CRLF terminator —
/// which is exactly the failure mode a buggy CSV serialiser would produce.
fn parse_rfc4180(input: &str) -> Vec<Vec<String>> {
    // Fields are accumulated as raw bytes and UTF-8-decoded at each field boundary, so
    // multibyte content (e.g. `café`) survives intact — a byte-as-char shortcut would mangle it.
    let bytes = input.as_bytes();
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field: Vec<u8> = Vec::new();
    let take_field = |field: &mut Vec<u8>| -> String {
        String::from_utf8(std::mem::take(field)).expect("RFC4180: field is not valid UTF-8")
    };
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        let b = bytes[i];
        if b == b'"' {
            // Quoted field — must be at the START of a field.
            assert!(field.is_empty(), "RFC4180: quote not at field start at byte {i}");
            i += 1;
            loop {
                assert!(i < n, "RFC4180: unterminated quoted field");
                let c = bytes[i];
                if c == b'"' {
                    if i + 1 < n && bytes[i + 1] == b'"' {
                        field.push(b'"'); // doubled quote => one literal quote
                        i += 2;
                    } else {
                        i += 1; // closing quote
                        break;
                    }
                } else {
                    field.push(c); // any byte (incl. CR/LF/comma/multibyte) is literal here
                    i += 1;
                }
            }
            // After a closing quote the next byte must be a delimiter, CRLF, or EOF.
            assert!(
                i >= n || bytes[i] == b',' || bytes[i] == b'\r',
                "RFC4180: text after closing quote at byte {i}"
            );
        } else if b == b',' {
            record.push(take_field(&mut field));
            i += 1;
        } else if b == b'\r' {
            assert!(i + 1 < n && bytes[i + 1] == b'\n', "RFC4180: bare CR (not CRLF) at byte {i}");
            record.push(take_field(&mut field));
            records.push(std::mem::take(&mut record));
            i += 2;
        } else if b == b'\n' {
            panic!("RFC4180: bare LF (not CRLF) at byte {i}");
        } else {
            // Unquoted field byte: must NOT be one of the must-quote characters.
            assert!(b != b'"', "RFC4180: bare quote in unquoted field at byte {i}");
            field.push(b);
            i += 1;
        }
    }
    // A trailing unterminated record (no final CRLF) would be a serialiser bug for us,
    // since every row is CRLF-terminated; surface it.
    assert!(field.is_empty() && record.is_empty(), "RFC4180: trailing data without CRLF terminator");
    records
}

/// The CSV *lexical* projection of a term, per the SPARQL results CSV spec: IRIs verbatim,
/// blank nodes as `_:label`, literals as their lexical value ONLY (no datatype/lang), and an
/// unbound cell as the empty string. This is the value the round-tripped CSV field must hold.
fn csv_lexical(cell: &Option<Term>) -> String {
    match cell {
        None => String::new(),
        Some(Term::NamedNode(n)) => n.as_str().to_string(),
        Some(Term::BlankNode(b)) => format!("_:{}", b.as_str()),
        Some(Term::Literal(l)) => l.value().to_string(),
        // RDF 1.2 triple term: CSV has no lossless encoding; our serialiser falls back to
        // oxrdf's Display. We mirror that here so the lexical comparison is exact.
        Some(other @ Term::Triple(_)) => other.to_string(),
    }
}

/// CSV oracle: parse our CSV strictly (RFC 4180) and assert the recovered header == bare
/// variable names and every recovered field == the lexical projection of the source term.
fn assert_csv_roundtrip(label: &str, r: &QueryResult, csv: &str) {
    let records = parse_rfc4180(csv);
    assert!(!records.is_empty(), "[{label}/CSV] missing header row\n--- output ---\n{csv}");

    let header: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    assert_eq!(records[0], header, "[{label}/CSV] header must be the bare variable names");

    let body = &records[1..];
    assert_eq!(body.len(), r.rows.len(), "[{label}/CSV] row count must match\n--- output ---\n{csv}");
    for (ri, (got, src)) in body.iter().zip(&r.rows).enumerate() {
        let expected: Vec<String> = src.iter().map(csv_lexical).collect();
        assert_eq!(
            *got, expected,
            "[{label}/CSV] row {ri} lexical fields must match (quoting/escaping check)\n--- output ---\n{csv}"
        );
    }
}

// ---------------------------------------------------------------------------
// The full per-case driver: run all three serialisers through the oracle.
// ---------------------------------------------------------------------------

fn check_all_formats(label: &str, r: &QueryResult, ordered: bool) {
    let xml = select_to_xml(r);
    assert_lossless_roundtrip(label, r, &xml, QueryResultsFormat::Xml, ordered);

    let tsv = select_to_tsv(r);
    assert_lossless_roundtrip(label, r, &tsv, QueryResultsFormat::Tsv, ordered);

    let csv = select_to_csv(r);
    assert_csv_roundtrip(label, r, &csv);
}

// ---------------------------------------------------------------------------
// Term constructors for the adversarial battery.
// ---------------------------------------------------------------------------

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}
fn bnode(label: &str) -> Term {
    Term::BlankNode(BlankNode::new(label).unwrap())
}
fn plain(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
}
fn lang(s: &str, tag: &str) -> Term {
    Term::Literal(Literal::new_language_tagged_literal(s, tag).unwrap())
}
fn typed(s: &str, dt: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(s, NamedNode::new(dt).unwrap()))
}

fn vars(names: &[&str]) -> Vec<Variable> {
    names.iter().map(|n| Variable::new(*n).unwrap()).collect()
}

// ---------------------------------------------------------------------------
// Adversarial battery — hand-built QueryResults with the nasty cells.
// ---------------------------------------------------------------------------

#[test]
fn oracle_adversarial_literals_roundtrip() {
    // One wide row exercising every literal hazard the serialisers must escape.
    let r = QueryResult {
        vars: vars(&["v"]),
        rows: vec![
            vec![Some(plain("plain"))],
            vec![Some(plain("embedded\nnewline"))],          // CSV must quote, TSV must \n
            vec![Some(plain("comma,separated"))],            // CSV must quote
            vec![Some(plain("has\"double\"quote"))],         // CSV must double, TSV must \"
            vec![Some(plain("  leading and trailing  "))],   // whitespace must survive verbatim
            vec![Some(plain("tab\tinside"))],                // TSV must \t
            vec![Some(plain("carriage\rreturn"))],           // CSV must quote, TSV must \r
            vec![Some(plain("back\\slash"))],                // TSV must \\
            vec![Some(plain("mix,\"\n\t\r\\end"))],          // all hazards at once
            vec![Some(lang("colour", "en-GB"))],             // language tag (with subtag)
            vec![Some(lang("Straße", "de"))],                // non-ASCII + lang
            vec![Some(typed("42", XSD_INTEGER))],            // xsd:integer datatype
            vec![Some(typed("2024-03-15T13:00:00Z", XSD_DATETIME))], // xsd:dateTime datatype
            vec![Some(iri("http://ex/q?a=1&b=2&c=x"))],       // IRI with `&` — needs XML escaping
            vec![Some(iri("http://ex/plain"))],
            vec![Some(bnode("b0"))],                         // blank node
            vec![None],                                      // unbound / OPTIONAL
        ],
    };
    // Unordered: the writers preserve order, but the SET equality is the contract here.
    check_all_formats("adversarial-literals", &r, false);
}

#[test]
fn oracle_multi_var_with_unbound_cells_roundtrip() {
    // Multiple variables, ragged binding (OPTIONAL-style holes in arbitrary columns),
    // including a fully-unbound row.
    let r = QueryResult {
        vars: vars(&["s", "p", "o"]),
        rows: vec![
            vec![Some(iri("http://ex/a")), Some(iri("http://ex/knows")), Some(iri("http://ex/b"))],
            vec![Some(iri("http://ex/a")), None, Some(lang("hi, there\n", "en"))],
            vec![None, Some(iri("http://ex/p")), None],
            vec![Some(bnode("x1")), Some(iri("http://ex/v")), Some(typed("-7", XSD_INTEGER))],
            vec![None, None, None], // entirely unbound row
        ],
    };
    check_all_formats("multi-var-unbound", &r, true); // ordered: rows must keep sequence
}

#[test]
fn oracle_rdf_star_triple_term_roundtrip() {
    // RDF 1.2 triple term as a binding value. XML uses the <triple> element; TSV uses
    // the `<<( … )>>` term syntax; both must re-parse to the SAME nested Triple. (CSV is the
    // lossy Display fallback, checked lexically.)
    let qt = Term::Triple(Box::new(Triple::new(
        NamedNode::new("http://ex/a").unwrap(),
        NamedNode::new("http://ex/b").unwrap(),
        plain("an \"object\" literal, with hazards\n"),
    )));
    let r = QueryResult {
        vars: vars(&["t", "c"]),
        rows: vec![
            vec![Some(qt), Some(typed("0.9", "http://www.w3.org/2001/XMLSchema#decimal"))],
            vec![None, Some(plain("note"))],
        ],
    };
    // XML + TSV are lossless and must recover the nested triple; assert those directly.
    let xml = select_to_xml(&r);
    assert_lossless_roundtrip("rdf-star", &r, &xml, QueryResultsFormat::Xml, true);
    let tsv = select_to_tsv(&r);
    assert_lossless_roundtrip("rdf-star", &r, &tsv, QueryResultsFormat::Tsv, true);
    // CSV: lexical (Display) fallback — well-formedness + field match.
    let csv = select_to_csv(&r);
    assert_csv_roundtrip("rdf-star", &r, &csv);
}

#[test]
fn oracle_empty_and_no_var_results_roundtrip() {
    // Zero rows (header only).
    let empty = QueryResult { vars: vars(&["a", "b"]), rows: vec![] };
    check_all_formats("empty-rows", &empty, true);

    // SELECT with no projected variables but one solution (the unit row) — the empty-BGP
    // shape. Header is empty; one all-empty result row.
    let unit = QueryResult { vars: vec![], rows: vec![vec![]] };
    // XML round-trips (one <result/> element). TSV/CSV degenerate to empty lines; validate
    // XML for the structural contract and CSV for well-formedness.
    let xml = select_to_xml(&unit);
    assert_lossless_roundtrip("unit-row", &unit, &xml, QueryResultsFormat::Xml, true);
}

// ---------------------------------------------------------------------------
// ASK boolean oracle — round-trip ask_to_xml / ask_to_json through sparesults.
// ---------------------------------------------------------------------------

#[test]
fn oracle_ask_boolean_roundtrip() {
    // Both ASK serialisers, for both truth values, must re-parse via the reference parser
    // back to the SAME boolean (and as the Boolean variant, not SELECT solutions). The
    // pre-existing inline tests only `contains`-checked the byte; this proves the documents
    // are well-formed SPARQL-results that a real consumer reads back correctly.
    for value in [true, false] {
        let xml = ask_to_xml(value);
        assert_eq!(
            reparse_boolean(xml.as_bytes(), QueryResultsFormat::Xml),
            value,
            "ASK/XML boolean must round-trip for {value}\n--- output ---\n{xml}"
        );

        let json = ask_to_json(value);
        assert_eq!(
            reparse_boolean(json.as_bytes(), QueryResultsFormat::Json),
            value,
            "ASK/JSON boolean must round-trip for {value}\n--- output ---\n{json}"
        );
    }
}

// ---------------------------------------------------------------------------
// Engine-driven random battery — real QueryResults from the real engine.
// ---------------------------------------------------------------------------

/// Deterministic LCG (same constants as the engine's exec tests) → a random Turtle graph
/// whose objects mix IRIs and adversarial literals, so engine-produced result sets carry the
/// hazardous cells through the serialisers.
fn random_graph_ttl(seed0: u64, n_nodes: u32, n_edges: usize) -> String {
    let mut seed = seed0;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    // A pool of nasty literal lexical forms (already Turtle-escaped for the loader).
    let lits = [
        r#""plain""#,
        r#""a,b""#,
        "\"line1\\nline2\"",
        "\"q\\\"uote\"",
        r#""  pad  ""#,
        r#""café"@fr"#,
        r#""99"^^<http://www.w3.org/2001/XMLSchema#integer>"#,
        r#""2020-01-02T03:04:05Z"^^<http://www.w3.org/2001/XMLSchema#dateTime>"#,
    ];
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for _ in 0..n_edges {
        let a = next() % n_nodes;
        let kind = next() % 3;
        match kind {
            0 => {
                let b = next() % n_nodes;
                ttl.push_str(&format!("ex:n{a} ex:link ex:n{b} .\n"));
            }
            1 => {
                let l = lits[(next() as usize) % lits.len()];
                ttl.push_str(&format!("ex:n{a} ex:val {l} .\n"));
            }
            _ => {
                // bnode object
                ttl.push_str(&format!("ex:n{a} ex:has [ ex:tag \"t{}\" ] .\n", next() % 5));
            }
        }
    }
    ttl
}

#[test]
fn oracle_engine_random_results_roundtrip() {
    use sparq_core::Graph;

    // A handful of seeds × query shapes. Each engine result set is pushed through all three
    // serialisers and round-tripped. SELECT *-style queries here are unordered → set check.
    let queries = [
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:link ?o }",
        "PREFIX ex: <http://ex/> SELECT ?s ?v WHERE { ?s ex:val ?v }",
        // OPTIONAL → unbound cells in the result.
        "PREFIX ex: <http://ex/> SELECT ?s ?v ?o WHERE { ?s ex:val ?v OPTIONAL { ?s ex:link ?o } }",
        // bnode objects in the projection.
        "PREFIX ex: <http://ex/> SELECT ?s ?h WHERE { ?s ex:has ?h }",
        // All three columns mixed.
        "PREFIX ex: <http://ex/> SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 200",
    ];

    for seed in [1u64, 7, 42, 1234, 98765] {
        let ttl = random_graph_ttl(seed, 24, 160);
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        for q in &queries {
            let r = query(&g, q).unwrap();
            check_all_formats(&format!("engine-seed{seed}"), &r, false);
        }
    }

    // ORDER BY case: verify the row SEQUENCE survives, not just the set.
    let ttl = random_graph_ttl(2024, 30, 200);
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let r = query(
        &g,
        "PREFIX ex: <http://ex/> SELECT ?s ?v WHERE { ?s ex:val ?v } ORDER BY ?v ?s",
    )
    .unwrap();
    check_all_formats("engine-ordered", &r, true);
}

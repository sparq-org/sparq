//! SPARQL 1.1 JSON results serialisation (`application/sparql-results+json`),
//! hand-written (no serde) so it is shared by the CLI and the wasm build with no
//! extra dependencies.

use crate::QueryResult;
use oxrdf::{Term, Variable};
use sparq_core::dict::TermParts;

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Serialises a term's JSON binding object directly from its dictionary components,
/// with no intermediate `oxrdf::Term` allocation (the materialise hot path).
///
/// `TermParts::Triple` is NOT handled here (it carries component IDS that need the
/// dictionary to resolve) — the id-level writers in `exec.rs` intercept it and recurse
/// before calling this. A stray call is a logic error, not silent corruption.
pub(crate) fn parts_to_json(s: &mut String, parts: TermParts<'_>) {
    match parts {
        TermParts::Triple(_) => unreachable!("triple-term JSON is written by the id-level writers (exec::write_id_json)"),
        TermParts::Iri { prefix, suffix } => {
            s.push_str("{\"type\":\"uri\",\"value\":\"");
            escape_into(s, prefix);
            escape_into(s, suffix);
            s.push_str("\"}");
        }
        TermParts::Blank(b) => {
            s.push_str("{\"type\":\"bnode\",\"value\":\"");
            escape_into(s, b);
            s.push_str("\"}");
        }
        TermParts::Lit { value, datatype, lang } => {
            s.push_str("{\"type\":\"literal\",\"value\":\"");
            escape_into(s, value);
            s.push('"');
            if let Some(lang) = lang {
                // [OPUS-4.8] sq-bj7o: the stored slot folds an RDF 1.2 base direction into
                // the tag as `lang--dir` (see `dict::lang_with_dir`). The SPARQL 1.2 JSON
                // results format keeps them apart: `xml:lang` is the bare tag and `its:dir`
                // the direction. Splitting here (instead of emitting `xml:lang:"en--ltr"`)
                // makes this path spec-conformant AND match the materialised `oxrdf::Term`
                // path (`term_to_json`) — the two were diverging for directional literals.
                let (tag, dir) = sparq_core::dict::split_lang_dir(lang);
                s.push_str(",\"xml:lang\":\"");
                escape_into(s, tag);
                s.push('"');
                if let Some(dir) = dir {
                    s.push_str(",\"its:dir\":\"");
                    escape_into(s, dir);
                    s.push('"');
                }
            } else if datatype != XSD_STRING {
                s.push_str(",\"datatype\":\"");
                escape_into(s, datatype);
                s.push('"');
            }
            s.push('}');
        }
    }
}

/// JSON for an inline-integer value id (an `xsd:integer`), heap-free.
pub(crate) fn inline_int_json(s: &mut String, mut v: u32) {
    s.push_str("{\"type\":\"literal\",\"value\":\"");
    if v == 0 {
        s.push('0');
    } else {
        let mut digits = [0u8; 10];
        let mut n = 0;
        while v > 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        for &d in digits[..n].iter().rev() {
            s.push(d as char);
        }
    }
    s.push_str("\",\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\"}");
}

/// Serialises a query result to a SPARQL 1.1 JSON results string.
pub fn to_sparql_json(r: &QueryResult) -> String {
    to_sparql_json_rows(&r.vars, &r.rows)
}

/// Serialises a SELECT solution table — given separately as its `vars` and a slice
/// of `rows` — to a **standalone, self-contained** SPARQL 1.1 JSON results document
/// (`{"head":{"vars":…},"results":{"bindings":[…]}}`). Passing a sub-slice of a larger
/// result's rows yields a valid document carrying just those rows, which lets a caller
/// page a big result into independently-parseable batches (the wasm `SolutionCursor`)
/// without ever holding the whole result as one JSON string. `to_sparql_json` is the
/// `rows == r.rows` special case.
pub fn to_sparql_json_rows(vars: &[Variable], rows: &[Vec<Option<Term>>]) -> String {
    let mut s = String::with_capacity(64 + rows.len() * 32);
    s.push_str("{\"head\":{\"vars\":[");
    for (i, v) in vars.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        escape_into(&mut s, v.as_str());
        s.push('"');
    }
    s.push_str("]},\"results\":{\"bindings\":[");
    for (ri, row) in rows.iter().enumerate() {
        if ri > 0 {
            s.push(',');
        }
        s.push('{');
        let mut first = true;
        for (vi, cell) in row.iter().enumerate() {
            if let Some(term) = cell {
                if !first {
                    s.push(',');
                }
                first = false;
                s.push('"');
                escape_into(&mut s, vars[vi].as_str());
                s.push_str("\":");
                term_to_json(&mut s, term);
            }
        }
        s.push('}');
    }
    s.push_str("]}}");
    s
}

pub(crate) fn term_to_json(s: &mut String, t: &Term) {
    match t {
        Term::NamedNode(n) => {
            s.push_str("{\"type\":\"uri\",\"value\":\"");
            escape_into(s, n.as_str());
            s.push_str("\"}");
        }
        Term::BlankNode(b) => {
            s.push_str("{\"type\":\"bnode\",\"value\":\"");
            escape_into(s, b.as_str());
            s.push_str("\"}");
        }
        Term::Literal(l) => {
            s.push_str("{\"type\":\"literal\",\"value\":\"");
            escape_into(s, l.value());
            s.push('"');
            if let Some(lang) = l.language() {
                s.push_str(",\"xml:lang\":\"");
                escape_into(s, lang);
                s.push('"');
                // [OPUS-4.8] sq-bj7o: RDF 1.2 base direction → SPARQL 1.2 `its:dir` (kept
                // separate from `xml:lang`, matching the stored-slot fast path above). Map the
                // two-variant `BaseDirection` enum to a `&'static str` rather than
                // `dir.to_string()` — the latter heap-allocated a fresh `String` per
                // directional literal (a Display call) for no reason.
                if let Some(dir) = l.direction() {
                    let dir = match dir {
                        oxrdf::BaseDirection::Ltr => "ltr",
                        oxrdf::BaseDirection::Rtl => "rtl",
                    };
                    s.push_str(",\"its:dir\":\"");
                    escape_into(s, dir);
                    s.push('"');
                }
            } else {
                let dt = l.datatype();
                if dt.as_str() != "http://www.w3.org/2001/XMLSchema#string" {
                    s.push_str(",\"datatype\":\"");
                    escape_into(s, dt.as_str());
                    s.push('"');
                }
            }
            s.push('}');
        }
        // RDF 1.2 triple term — the SPARQL 1.2 JSON results encoding:
        // {"type":"triple","value":{"subject":…,"predicate":…,"object":…}}.
        Term::Triple(t) => {
            s.push_str("{\"type\":\"triple\",\"value\":{\"subject\":");
            match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => term_to_json(s, &Term::NamedNode(n.clone())),
                oxrdf::NamedOrBlankNode::BlankNode(b) => term_to_json(s, &Term::BlankNode(b.clone())),
            }
            s.push_str(",\"predicate\":");
            term_to_json(s, &Term::NamedNode(t.predicate.clone()));
            s.push_str(",\"object\":");
            term_to_json(s, &t.object);
            s.push_str("}}");
        }
    }
}

/// Appends `s` to `out` with JSON string escaping. Bulk-copies maximal runs of
/// already-safe text and only handles the rare escape byte individually — the common
/// (no-escape) value is a single `push_str`, which matters when serialising millions of
/// result cells. Byte-level is safe: every escaped byte is ASCII (< 0x80), so UTF-8
/// multi-byte sequences (all bytes ≥ 0x80) are never split and pass through in a run.
pub(crate) fn escape_into(out: &mut String, s: &str) {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &str = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => {
                // Other control character → \u00XX.
                out.push_str(&s[start..i]);
                out.push_str("\\u00");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        out.push_str(&s[start..i]);
        out.push_str(esc);
        start = i + 1;
    }
    out.push_str(&s[start..]);
}

#[cfg(test)]
mod rows_tests {
    use super::{to_sparql_json, to_sparql_json_rows};
    use crate::QueryResult;
    use oxrdf::{NamedNode, Term, Variable};

    fn row(iri: &str) -> Vec<Option<Term>> {
        vec![Some(Term::NamedNode(NamedNode::new(iri).unwrap()))]
    }

    #[test]
    fn rows_slice_is_self_contained_and_partitions_whole() {
        let vars = vec![Variable::new("s").unwrap()];
        let rows = vec![row("http://ex/a"), row("http://ex/b"), row("http://ex/c")];
        let r = QueryResult { vars: vars.clone(), rows: rows.clone() };

        // The whole-result wrapper is the rows == r.rows special case.
        assert_eq!(to_sparql_json(&r), to_sparql_json_rows(&vars, &rows));

        // Each slice is an independently valid SPARQL-JSON doc carrying its head vars.
        let first = to_sparql_json_rows(&vars, &rows[0..2]);
        let rest = to_sparql_json_rows(&vars, &rows[2..3]);
        for doc in [&first, &rest] {
            assert!(doc.contains("\"vars\":[\"s\"]"), "missing head vars: {doc}");
            assert!(doc.starts_with("{\"head\":") && doc.ends_with("}}"), "not a full doc: {doc}");
        }
        assert_eq!(first.matches("\"s\":{").count(), 2);
        assert_eq!(rest.matches("\"s\":{").count(), 1);
        assert!(first.contains("http://ex/a") && first.contains("http://ex/b"));
        assert!(rest.contains("http://ex/c"));

        // An empty slice is still a valid doc with empty bindings.
        let empty = to_sparql_json_rows(&vars, &[]);
        assert!(empty.contains("\"bindings\":[]"), "empty slice: {empty}");
    }
}

#[cfg(test)]
mod escape_tests {
    use super::escape_into;

    fn esc(s: &str) -> String {
        let mut out = String::new();
        escape_into(&mut out, s);
        out
    }

    #[test]
    fn escapes_match_json_semantics() {
        assert_eq!(esc(""), "");
        assert_eq!(esc("plain text 123"), "plain text 123");
        assert_eq!(esc("a\"b"), "a\\\"b");
        assert_eq!(esc("a\\b"), "a\\\\b");
        assert_eq!(esc("a\nb\tc\rd"), "a\\nb\\tc\\rd");
        // Other control characters → \u00XX (lowercase hex).
        assert_eq!(esc("\u{0}\u{1f}\u{7}"), "\\u0000\\u001f\\u0007");
        // Multi-byte UTF-8 passes through untouched (bytes are all >= 0x80).
        assert_eq!(esc("café — 日本語 😀"), "café — 日本語 😀");
        // Mixed: clean run, escape, clean run.
        assert_eq!(esc("http://ex/a\"x/é"), "http://ex/a\\\"x/é");
        // DEL (0x7f) is NOT a JSON-required escape — passes through.
        assert_eq!(esc("\u{7f}"), "\u{7f}");
    }
}

/// [OPUS-4.8] (sq-bif) The materialised `term_to_json` writer through the public
/// `to_sparql_json_rows` — the SPARQL-1.1/1.2-results-JSON binding object for EACH
/// term shape. The existing `rows_tests` only exercise the `uri` shape; these pin
/// the `bnode`, plain/typed/language/directional literal and RDF-1.2 triple-term
/// encodings so a regression in any branch of `term_to_json` is caught (e.g. the
/// `its:dir` split or the recursive triple-term writer, which had no public
/// assertion before).
#[cfg(test)]
mod term_shape_tests {
    use super::to_sparql_json_rows;
    use oxrdf::vocab::xsd;
    use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};

    /// Wrap one term as a single `?v` binding and return JUST its binding object
    /// (the depth-matched object after `"v":` in the one-row document), so each
    /// assertion reads as the exact SPARQL-results-JSON for that term.
    fn binding(t: Term) -> String {
        let vars = vec![Variable::new("v").unwrap()];
        let doc = to_sparql_json_rows(&vars, &[vec![Some(t)]]);
        let needle = "\"v\":";
        let start = doc.find(needle).expect("binding present") + needle.len();
        let rest = &doc[start..];
        // The object runs from its opening `{` to the matching `}`.
        let bytes = rest.as_bytes();
        let mut depth = 0usize;
        let mut end = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        rest[..end.expect("object closes")].to_string()
    }

    #[test]
    fn iri_and_bnode_shapes() {
        assert_eq!(
            binding(Term::NamedNode(NamedNode::new("http://ex/o").unwrap())),
            r#"{"type":"uri","value":"http://ex/o"}"#
        );
        assert_eq!(
            binding(Term::BlankNode(BlankNode::new("b0").unwrap())),
            r#"{"type":"bnode","value":"b0"}"#
        );
    }

    #[test]
    fn plain_and_typed_literal_shapes() {
        // A plain (xsd:string) literal omits the datatype key entirely.
        assert_eq!(
            binding(Term::Literal(Literal::new_simple_literal("hi"))),
            r#"{"type":"literal","value":"hi"}"#
        );
        // xsd:string is the default and is STILL omitted even when explicit.
        assert_eq!(
            binding(Term::Literal(Literal::new_typed_literal("hi", xsd::STRING))),
            r#"{"type":"literal","value":"hi"}"#
        );
        // A non-string datatype is emitted.
        let int42 = Term::Literal(Literal::new_typed_literal("42", xsd::INTEGER));
        assert_eq!(
            binding(int42),
            r#"{"type":"literal","value":"42","datatype":"http://www.w3.org/2001/XMLSchema#integer"}"#
        );
        // The value is JSON-escaped (a quote and a newline in the lexical form).
        assert_eq!(
            binding(Term::Literal(Literal::new_simple_literal("a\"b\nc"))),
            r#"{"type":"literal","value":"a\"b\nc"}"#
        );
    }

    #[test]
    fn language_and_directional_literal_shapes() {
        // Directional-literal builder, kept short so the assertions fit.
        let dir = |v: &str, tag: &str, d: BaseDirection| {
            Term::Literal(Literal::new_directional_language_tagged_literal(v, tag, d).unwrap())
        };
        let lang = |v: &str, tag: &str| {
            Term::Literal(Literal::new_language_tagged_literal(v, tag).unwrap())
        };

        // A language-tagged literal: `xml:lang`, no datatype.
        assert_eq!(
            binding(lang("hello", "en")),
            r#"{"type":"literal","value":"hello","xml:lang":"en"}"#
        );
        // RDF 1.2 directional literal: `xml:lang` is the BARE tag and `its:dir`
        // carries the base direction — the two are kept apart (SPARQL 1.2 JSON),
        // never folded into one `xml:lang:"en--ltr"`.
        assert_eq!(
            binding(dir("hello", "en", BaseDirection::Ltr)),
            r#"{"type":"literal","value":"hello","xml:lang":"en","its:dir":"ltr"}"#
        );
        assert_eq!(
            binding(dir("مرحبا", "ar", BaseDirection::Rtl)),
            r#"{"type":"literal","value":"مرحبا","xml:lang":"ar","its:dir":"rtl"}"#
        );
    }

    #[test]
    fn rdf_star_triple_term_shape_recurses() {
        // An RDF 1.2 quoted triple `<< :a :p 7 >>` in object position:
        // `{"type":"triple","value":{"subject":…,"predicate":…,"object":…}}`, each
        // component recursively encoded by the SAME writer (so the integer object
        // carries its datatype and the predicate is a `uri`).
        let t = Triple::new(
            NamedNode::new("http://ex/a").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Literal::new_typed_literal("7", xsd::INTEGER),
        );
        assert_eq!(
            binding(Term::Triple(Box::new(t))),
            concat!(
                r#"{"type":"triple","value":{"#,
                r#""subject":{"type":"uri","value":"http://ex/a"},"#,
                r#""predicate":{"type":"uri","value":"http://ex/p"},"#,
                r#""object":{"type":"literal","value":"7","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}}"#,
            )
        );

        // A triple nested in the OBJECT of another triple recurses one more level
        // (subject-position quoted triples are not representable in oxrdf's term
        // model, so object nesting is the reachable nested case).
        let inner = Triple::new(
            NamedNode::new("http://ex/a").unwrap(),
            NamedNode::new("http://ex/p").unwrap(),
            Term::NamedNode(NamedNode::new("http://ex/b").unwrap()),
        );
        let outer = Triple::new(
            NamedNode::new("http://ex/c").unwrap(),
            NamedNode::new("http://ex/says").unwrap(),
            Term::Triple(Box::new(inner)),
        );
        let json = binding(Term::Triple(Box::new(outer)));
        let has = |needle: &str| assert!(json.contains(needle), "{json}");
        has(r#""object":{"type":"triple","value":{"#);
        has(r#""object":{"type":"uri","value":"http://ex/b"}"#);
        let want_prefix =
            r#"{"type":"triple","value":{"subject":{"type":"uri","value":"http://ex/c"}"#;
        assert!(json.starts_with(want_prefix), "{json}");
    }
}

//! A reader for the neutral wire form each oracle emits — **SPARQL 1.1 Results JSON** — into the
//! neutral `{var → term}` model. Engine-independent (it parses the standard serialisation, not any
//! engine's in-memory types). sq-qcnn.4
//!
//! `SELECT` results become [`QueryResults::Solutions`]; `ASK` results become
//! [`QueryResults::Boolean`]. Graph results (`CONSTRUCT`/`DESCRIBE`, which are N-Triples not JSON and
//! need blank-node canonicalisation) are a separate DAG node (`sq-qcnn.7`) and are not read here.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::term::{Term, RDF_LANGSTRING, XSD_STRING};

/// One solution mapping: projected variable → bound term. A projected-but-**unbound** variable is
/// simply **absent** from the map (distinct from bound-to-empty-string).
pub type Solution = BTreeMap<String, Term>;

/// A parsed SPARQL result set.
#[derive(Debug, Clone)]
pub enum QueryResults {
    /// A `SELECT` result: the header variable order plus the solution multiset.
    Solutions {
        vars: Vec<String>,
        solutions: Vec<Solution>,
    },
    /// An `ASK` result.
    Boolean(bool),
}

/// A SPARQL-Results-JSON parse error, carrying a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SPARQL-Results-JSON parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError(msg.into()))
}

/// Parse a SPARQL-Results-JSON document (`SELECT` bindings or an `ASK` boolean).
pub fn parse_results_json(s: &str) -> Result<QueryResults, ParseError> {
    let root: Value = serde_json::from_str(s).map_err(|e| ParseError(e.to_string()))?;
    let obj = root.as_object().ok_or_else(|| ParseError("root is not an object".into()))?;

    // ASK: a top-level "boolean".
    if let Some(b) = obj.get("boolean") {
        return match b.as_bool() {
            Some(v) => Ok(QueryResults::Boolean(v)),
            None => err("\"boolean\" is not a boolean"),
        };
    }

    // SELECT: head.vars + results.bindings. sq-qcnn.4 — parse `head.vars`
    // STRICTLY (reject, do not silently default): a SELECT Results-JSON must carry
    // `head.vars` as an array of strings (SPARQL 1.1 §Query Results JSON). Silently
    // defaulting a missing/ill-typed header to `[]`, or dropping a non-string element,
    // would let a broken oracle output be compared as if it had no projected vars —
    // exactly the masking of invalid oracle output this differential harness must reject.
    let vars_arr = obj
        .get("head")
        .and_then(|h| h.get("vars"))
        .ok_or_else(|| ParseError("missing head.vars (and no boolean)".into()))?
        .as_array()
        .ok_or_else(|| ParseError("head.vars is not an array".into()))?;
    let mut vars: Vec<String> = Vec::with_capacity(vars_arr.len());
    for v in vars_arr {
        let name = v
            .as_str()
            .ok_or_else(|| ParseError("head.vars element is not a string".into()))?;
        vars.push(name.to_string());
    }

    let bindings = obj
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError("missing results.bindings (and no boolean)".into()))?;

    let mut solutions = Vec::with_capacity(bindings.len());
    for row in bindings {
        let row_obj = row.as_object().ok_or_else(|| ParseError("a binding row is not an object".into()))?;
        let mut sol = Solution::new();
        for (var, cell) in row_obj {
            sol.insert(var.clone(), parse_term(cell)?);
        }
        solutions.push(sol);
    }
    Ok(QueryResults::Solutions { vars, solutions })
}

/// Parse one RDF-term binding cell (`{type, value, datatype?, xml:lang?}`, or a nested `triple`).
fn parse_term(cell: &Value) -> Result<Term, ParseError> {
    let ty = cell
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ParseError("binding cell missing \"type\"".into()))?;
    match ty {
        "uri" | "iri" => {
            let v = str_field(cell, "value")?;
            Ok(Term::Iri(v.to_string()))
        }
        "bnode" => {
            let v = str_field(cell, "value")?;
            Ok(Term::Blank(v.to_string()))
        }
        "literal" | "typed-literal" => {
            let lexical = str_field(cell, "value")?.to_string();
            let lang = cell.get("xml:lang").and_then(Value::as_str).map(str::to_string);
            let explicit_dt = cell.get("datatype").and_then(Value::as_str);
            // `xml:lang` and `datatype` must agree: a language-tagged literal's datatype is
            // `rdf:langString`, and nothing else may carry that datatype. Reject either contradiction
            // rather than silently fabricate/override a term — masking invalid oracle output would let
            // a real divergence slip past strict equality / value keying.
            match (&lang, explicit_dt) {
                // datatype names `rdf:langString` but there is no tag: invalid.
                (None, Some(d)) if d == RDF_LANGSTRING => {
                    return err("literal cell has datatype rdf:langString but no xml:lang");
                }
                // a tag is present but the explicit datatype is not `rdf:langString`: contradictory.
                (Some(_), Some(d)) if d != RDF_LANGSTRING => {
                    return err("language-tagged literal cell has a non-rdf:langString datatype");
                }
                _ => {}
            }
            let datatype = match (&lang, explicit_dt) {
                (Some(_), _) => RDF_LANGSTRING.to_string(),
                (None, Some(d)) => d.to_string(),
                (None, None) => XSD_STRING.to_string(),
            };
            Ok(Term::Literal {
                lexical,
                datatype,
                lang,
            })
        }
        "triple" => {
            let v = cell.get("value").ok_or_else(|| ParseError("triple missing \"value\"".into()))?;
            let s = parse_term(v.get("subject").ok_or_else(|| ParseError("triple missing subject".into()))?)?;
            let p = parse_term(v.get("predicate").ok_or_else(|| ParseError("triple missing predicate".into()))?)?;
            let o = parse_term(v.get("object").ok_or_else(|| ParseError("triple missing object".into()))?)?;
            // RDF triple-term positional constraints (universal across RDF versions): the subject
            // cannot be a literal, and the predicate must be an IRI. Reject a structurally-invalid
            // triple rather than canonicalise non-conformant oracle output, which could mask a
            // divergence. (The object may be any term kind, so it is unconstrained here.)
            if matches!(s, Term::Literal { .. }) {
                return err("triple-term subject cannot be a literal");
            }
            if !matches!(p, Term::Iri(_)) {
                return err("triple-term predicate must be an IRI");
            }
            Ok(Term::Triple(Box::new([s, p, o])))
        }
        other => err(format!("unknown binding type {:?}", other)),
    }
}

fn str_field<'a>(cell: &'a Value, key: &str) -> Result<&'a str, ParseError> {
    cell.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ParseError(format!("binding cell missing string \"{}\"", key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ask_boolean() {
        let r = parse_results_json(r#"{"head":{},"boolean":true}"#).unwrap();
        assert!(matches!(r, QueryResults::Boolean(true)));
        let r = parse_results_json(r#"{"head":{},"boolean":false}"#).unwrap();
        assert!(matches!(r, QueryResults::Boolean(false)));
    }

    #[test]
    fn parse_select_all_term_kinds() {
        let doc = r#"{
          "head": { "vars": [ "i", "s", "b", "n", "t" ] },
          "results": { "bindings": [ {
            "i": { "type": "uri", "value": "http://example.org/x" },
            "s": { "type": "literal", "value": "hello" },
            "b": { "type": "bnode", "value": "b0" },
            "n": { "type": "literal", "value": "42", "datatype": "http://www.w3.org/2001/XMLSchema#integer" },
            "t": { "type": "literal", "value": "chat", "xml:lang": "EN" }
          } ] }
        }"#;
        let r = parse_results_json(doc).unwrap();
        let QueryResults::Solutions { vars, solutions } = r else {
            panic!("expected solutions");
        };
        assert_eq!(vars, vec!["i", "s", "b", "n", "t"]);
        assert_eq!(solutions.len(), 1);
        let sol = &solutions[0];
        assert!(matches!(sol.get("i"), Some(Term::Iri(u)) if u == "http://example.org/x"));
        assert!(matches!(sol.get("b"), Some(Term::Blank(l)) if l == "b0"));
        // a plain literal folds to xsd:string.
        match sol.get("s") {
            Some(Term::Literal { datatype, lang, .. }) => {
                assert_eq!(datatype, XSD_STRING);
                assert!(lang.is_none());
            }
            other => panic!("expected literal, got {:?}", other),
        }
        // a language-tagged literal gets rdf:langString + the tag.
        match sol.get("t") {
            Some(Term::Literal { lang, datatype, .. }) => {
                assert_eq!(lang.as_deref(), Some("EN"));
                assert_eq!(datatype, RDF_LANGSTRING);
            }
            other => panic!("expected lang literal, got {:?}", other),
        }
    }

    #[test]
    fn parse_triple_term_and_errors() {
        let doc = r#"{
          "head": { "vars": [ "t" ] },
          "results": { "bindings": [ {
            "t": { "type": "triple", "value": {
              "subject":   { "type": "uri", "value": "http://s" },
              "predicate": { "type": "uri", "value": "http://p" },
              "object":    { "type": "literal", "value": "1", "datatype": "http://www.w3.org/2001/XMLSchema#integer" }
            } }
          } ] }
        }"#;
        let r = parse_results_json(doc).unwrap();
        let QueryResults::Solutions { solutions, .. } = r else { panic!() };
        assert!(matches!(solutions[0].get("t"), Some(Term::Triple(_))));

        // error paths.
        assert!(parse_results_json("not json").is_err());
        assert!(parse_results_json("[]").is_err());
        assert!(parse_results_json(r#"{"head":{}}"#).is_err());
        assert!(parse_results_json(r#"{"results":{"bindings":[{"x":{"type":"mystery","value":"?"}}]}}"#).is_err());
    }

    #[test]
    fn rejects_langstring_datatype_without_lang() {
        // A literal that names rdf:langString as its datatype but omits xml:lang is an invalid
        // Results-JSON cell (a language-tagged literal must carry a tag); it must be rejected rather
        // than fabricate an inconsistent langString-with-no-tag term.
        let doc = r#"{
          "head": { "vars": [ "x" ] },
          "results": { "bindings": [ {
            "x": { "type": "literal", "value": "hi",
                   "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString" }
          } ] }
        }"#;
        assert!(parse_results_json(doc).is_err());
        // …but the same lexical WITH a tag is a well-formed language-tagged literal.
        let ok = r#"{
          "head": { "vars": [ "x" ] },
          "results": { "bindings": [ {
            "x": { "type": "literal", "value": "hi", "xml:lang": "en" }
          } ] }
        }"#;
        assert!(parse_results_json(ok).is_ok());
    }

    #[test]
    fn rejects_lang_with_conflicting_datatype() {
        // A tag alongside a non-langString datatype is contradictory and must be rejected rather than
        // silently overridden to rdf:langString (which would mask invalid oracle output).
        let doc = r#"{
          "head": { "vars": [ "x" ] },
          "results": { "bindings": [ {
            "x": { "type": "literal", "value": "hi", "xml:lang": "en",
                   "datatype": "http://www.w3.org/2001/XMLSchema#string" }
          } ] }
        }"#;
        assert!(parse_results_json(doc).is_err());
        // A redundant but consistent langString datatype alongside the tag is fine.
        let ok = r#"{
          "head": { "vars": [ "x" ] },
          "results": { "bindings": [ {
            "x": { "type": "literal", "value": "hi", "xml:lang": "en",
                   "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString" }
          } ] }
        }"#;
        assert!(parse_results_json(ok).is_ok());
    }

    #[test]
    fn rejects_structurally_invalid_triple_term() {
        // A literal subject is never valid RDF; reject rather than canonicalise.
        let lit_subject = r#"{
          "head": { "vars": [ "t" ] },
          "results": { "bindings": [ {
            "t": { "type": "triple", "value": {
              "subject":   { "type": "literal", "value": "x" },
              "predicate": { "type": "uri", "value": "http://p" },
              "object":    { "type": "uri", "value": "http://o" }
            } }
          } ] }
        }"#;
        assert!(parse_results_json(lit_subject).is_err());
        // A non-IRI predicate (here a blank node) is likewise invalid.
        let bnode_predicate = r#"{
          "head": { "vars": [ "t" ] },
          "results": { "bindings": [ {
            "t": { "type": "triple", "value": {
              "subject":   { "type": "uri", "value": "http://s" },
              "predicate": { "type": "bnode", "value": "b0" },
              "object":    { "type": "uri", "value": "http://o" }
            } }
          } ] }
        }"#;
        assert!(parse_results_json(bnode_predicate).is_err());
    }

    #[test]
    fn rejects_malformed_head_vars() {
        // sq-qcnn.4 — a SELECT result (no top-level "boolean") must carry a
        // well-formed `head.vars`: rejecting malformed headers rather than defaulting to `[]`
        // keeps a broken oracle output from being compared as if it had no projected vars.
        // (a) head.vars entirely absent.
        assert!(parse_results_json(r#"{"head":{},"results":{"bindings":[]}}"#).is_err());
        // (b) head.vars is not an array.
        assert!(parse_results_json(r#"{"head":{"vars":"x"},"results":{"bindings":[]}}"#).is_err());
        // (c) a non-string element must be rejected, not silently dropped.
        assert!(
            parse_results_json(r#"{"head":{"vars":["x",5]},"results":{"bindings":[]}}"#).is_err()
        );
        // …but an empty projection (valid `SELECT *` over a var-less pattern) is accepted, and
        // a well-formed all-string header round-trips its variable order.
        let empty = parse_results_json(r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#).unwrap();
        assert!(matches!(empty, QueryResults::Solutions { ref vars, .. } if vars.is_empty()));
        let two = parse_results_json(r#"{"head":{"vars":["a","b"]},"results":{"bindings":[]}}"#)
            .unwrap();
        let QueryResults::Solutions { vars, .. } = two else { panic!() };
        assert_eq!(vars, vec!["a", "b"]);
    }
}

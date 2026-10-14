//! (sq-hj4n, gh-916) OPT-IN Solid-style **N3-Patch** parsing for the Graph-Store-
//! Protocol `PATCH` method.
//!
//! 🤖 SPARQ agent — feature-gated capability (`n3-patch`, default OFF).
//!
//! This module is compiled ONLY behind the `n3-patch` cargo feature. With the feature off the
//! whole module — and its single dispatch arm in [`crate::http`] — is `#[cfg]`-stripped, so a
//! `text/n3` `PATCH` body is a plain `415 Unsupported Media Type` and the crate is byte-identical
//! to before. The OTHER `PATCH` dialect (`application/sparql-update` body) is always-on and lives
//! in [`crate::http`], NOT here.
//!
//! ## What an N3-Patch is
//!
//! The Solid Protocol's N3-Patch (`text/n3`) is an N3 document describing exactly ONE
//! `solid:InsertDeletePatch` resource with up to three formula properties:
//!
//! ```text
//! @prefix solid: <http://www.w3.org/ns/solid/terms#>.
//! @prefix ex:    <http://example.org/>.
//! _:patch a solid:InsertDeletePatch;
//!   solid:where   { ?person ex:name "Jane". };
//!   solid:deletes { ?person ex:age 12. };
//!   solid:inserts { ?person ex:age 13. }.
//! ```
//!
//! * `solid:where`   — a graph pattern that binds the variables used in the other two formulas;
//! * `solid:deletes` — the triples to remove (may reference `where`-bound variables);
//! * `solid:inserts` — the triples to add (may reference `where`-bound variables).
//!
//! Each property is OPTIONAL but at least one of `deletes` / `inserts` MUST be present, and each
//! MUST appear at most once (Solid §N3 Patch conformance). A blank node may appear only in
//! `inserts` (it mints a fresh node); `deletes` and `where` may not introduce blank nodes (they
//! would be unsatisfiable / unsafe to delete), so we reject them — fail-closed.
//!
//! ## Translation — one ATOMIC, graph-scoped SPARQL Update
//!
//! We translate the parsed patch into a single SPARQL Update and submit it through the SAME
//! sequenced group-commit writer the `application/sparql-update` PATCH path uses, so the whole
//! delete+insert lands in ONE durable generation (atomic) — never a partial effect. The caller
//! ([`crate::http`]) supplies the graph-scoping (`GRAPH <g> { … }` for a named graph, bare for the
//! default graph), reusing the exact `graph_data_block` helper the GSP write path already uses.
//!
//! With a `where` clause the shape is `DELETE { … } INSERT { … } WHERE { … }` (one atomic
//! pattern-based modify). Without a `where` clause (concrete triples only) the shape is
//! `DELETE DATA { … } ; INSERT DATA { … }` — DATA-form for ground triples, matching the GSP
//! write path's `INSERT DATA` discipline. Either way it is one writer submission.

use oxrdf::GraphName;
use oxttl::n3::{N3Parser, N3Quad, N3Term};
use std::collections::BTreeMap;

/// The Solid terms vocabulary base.
const SOLID: &str = "http://www.w3.org/ns/solid/terms#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// A failure parsing or validating an N3-Patch body. Each maps to a client `400` in the handler
/// (a malformed/invalid patch is the caller's error). The message is SAFE to return: it describes
/// the structural rule that was violated, never echoing attacker-controlled term text (the
/// underlying oxttl parse error — which CAN quote body tokens — is folded into a single generic
/// `Parse` message; the operator gets the detail in the server log via the handler's
/// `sanitized_error`).
#[derive(Debug)]
pub enum N3PatchError {
    /// The N3 body did not parse (syntax error). Detail withheld from the client.
    Parse(String),
    /// No `solid:InsertDeletePatch` resource was found.
    NoPatch,
    /// More than one `solid:InsertDeletePatch` resource was found (the document must describe
    /// exactly one patch).
    MultiplePatches,
    /// A formula property (`solid:where` / `solid:deletes` / `solid:inserts`) appeared more than
    /// once on the patch resource.
    DuplicateProperty(&'static str),
    /// Neither `solid:deletes` nor `solid:inserts` was present (an empty patch is meaningless).
    Empty,
    /// A `solid:deletes` or `solid:where` formula introduced a blank node (only `solid:inserts`
    /// may mint blank nodes; a blank node in a delete/where pattern is unsafe / unsatisfiable).
    BlankNodeInPattern(&'static str),
    /// A formula referenced by `solid:where`/`deletes`/`inserts` was not a `{ … }` block (the
    /// object was a literal or a node that names no formula graph).
    NotAFormula(&'static str),
}

impl std::fmt::Display for N3PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Generic — never interpolates the body's parse-error text (which can quote tokens).
            N3PatchError::Parse(_) => write!(f, "malformed N3-Patch body"),
            N3PatchError::NoPatch => write!(
                f,
                "N3-Patch body has no solid:InsertDeletePatch resource"
            ),
            N3PatchError::MultiplePatches => write!(
                f,
                "N3-Patch body must describe exactly one solid:InsertDeletePatch resource"
            ),
            N3PatchError::DuplicateProperty(p) => {
                write!(f, "N3-Patch property solid:{} appears more than once", p)
            }
            N3PatchError::Empty => write!(
                f,
                "N3-Patch must have at least one of solid:deletes or solid:inserts"
            ),
            N3PatchError::BlankNodeInPattern(p) => write!(
                f,
                "N3-Patch solid:{} must not contain a blank node (only solid:inserts may)",
                p
            ),
            N3PatchError::NotAFormula(p) => {
                write!(f, "N3-Patch solid:{} must reference an {{ … }} formula", p)
            }
        }
    }
}

/// The under-the-hood detail of a parse failure (for the SERVER LOG only — `sanitized_error`
/// withholds it from the client because the oxttl error can echo body tokens). For the
/// structural errors there is no extra detail, so the Display message is reused.
impl N3PatchError {
    /// The detail string the handler logs server-side (never returned to the client).
    pub fn detail(&self) -> String {
        match self {
            N3PatchError::Parse(d) => d.clone(),
            other => other.to_string(),
        }
    }
}

/// A parsed N3-Patch, reduced to the three SPARQL block bodies (one triple-pattern per line, in
/// SPARQL term syntax). Empty when the corresponding formula was absent. The caller assembles
/// these into a graph-scoped SPARQL Update (see [`crate::http`]); keeping the graph-scoping in the
/// handler reuses the exact `graph_data_block` helper the rest of the GSP write path uses.
pub struct N3Patch {
    /// The `solid:deletes` triples (SPARQL term syntax, one per line, may use `where` variables).
    pub deletes: String,
    /// The `solid:inserts` triples (SPARQL term syntax, one per line, may use `where` variables).
    pub inserts: String,
    /// The `solid:where` graph pattern (SPARQL term syntax, one triple per line). Empty when no
    /// `solid:where` was given (then the patch is a ground DATA-form modify).
    pub conditions: String,
    /// Whether a `solid:where` formula was present (distinguishes a ground DATA-form patch from a
    /// pattern-based `DELETE/INSERT … WHERE`).
    pub has_where: bool,
}

/// Parses + validates an N3-Patch body into its three SPARQL block bodies.
///
/// `base` is the IRI relative references in the body resolve against — the caller passes the
/// addressed graph's IRI (or `None` for the default graph), exactly as the GSP write path does for
/// an RDF/XML body. On any structural or syntactic violation returns an [`N3PatchError`] (the
/// caller's `400`).
pub fn parse(body: &[u8], base: Option<&str>) -> Result<N3Patch, N3PatchError> {
    // oxttl's N3Parser flattens nested `{ … }` formulas into quads whose `graph_name` is a fresh
    // blank node, and emits the top-level `?patch solid:inserts _:formula` statements in the
    // DEFAULT graph linking to that blank node — so a single pass collects both the patch's
    // properties and each formula's contents.
    let mut parser = N3Parser::new();
    if let Some(b) = base {
        parser = parser
            .with_base_iri(b)
            .map_err(|e| N3PatchError::Parse(e.to_string()))?;
    }
    let mut quads: Vec<N3Quad> = Vec::new();
    for q in parser.for_slice(body) {
        quads.push(q.map_err(|e| N3PatchError::Parse(e.to_string()))?);
    }

    // 1) Find the single solid:InsertDeletePatch resource and its where/deletes/inserts formula
    //    references. The patch resource term-prints once (blank node or IRI); we key formulas by
    //    the printed graph-name string.
    let mut patch_subject: Option<String> = None;
    // property name -> formula graph-name key (the blank node that names the `{ … }` block).
    let mut where_ref: Option<String> = None;
    let mut deletes_ref: Option<String> = None;
    let mut inserts_ref: Option<String> = None;
    let mut where_seen = false;
    let mut deletes_seen = false;
    let mut inserts_seen = false;

    for q in &quads {
        // Patch metadata lives in the DEFAULT graph (top-level statements).
        if q.graph_name != GraphName::DefaultGraph {
            continue;
        }
        let pred = match &q.predicate {
            N3Term::NamedNode(n) => n.as_str().to_string(),
            _ => continue,
        };
        if pred == RDF_TYPE {
            if let N3Term::NamedNode(obj) = &q.object {
                if obj.as_str() == format!("{}InsertDeletePatch", SOLID) {
                    let subj = term_key(&q.subject);
                    match &patch_subject {
                        Some(existing) if *existing != subj => {
                            return Err(N3PatchError::MultiplePatches)
                        }
                        Some(_) => {}
                        None => patch_subject = Some(subj),
                    }
                }
            }
            continue;
        }
        // The three formula-valued properties.
        let prop = match pred.strip_prefix(SOLID) {
            Some("where") => "where",
            Some("deletes") => "deletes",
            Some("inserts") => "inserts",
            _ => continue,
        };
        let formula_key = formula_object_key(&q.object).ok_or(N3PatchError::NotAFormula(
            match prop {
                "where" => "where",
                "deletes" => "deletes",
                _ => "inserts",
            },
        ))?;
        match prop {
            "where" => {
                if where_seen {
                    return Err(N3PatchError::DuplicateProperty("where"));
                }
                where_seen = true;
                where_ref = Some(formula_key);
            }
            "deletes" => {
                if deletes_seen {
                    return Err(N3PatchError::DuplicateProperty("deletes"));
                }
                deletes_seen = true;
                deletes_ref = Some(formula_key);
            }
            _ => {
                if inserts_seen {
                    return Err(N3PatchError::DuplicateProperty("inserts"));
                }
                inserts_seen = true;
                inserts_ref = Some(formula_key);
            }
        }
    }

    if patch_subject.is_none() {
        return Err(N3PatchError::NoPatch);
    }
    if deletes_ref.is_none() && inserts_ref.is_none() {
        return Err(N3PatchError::Empty);
    }

    // 2) Group every formula's triples by graph-name key (one pass over the non-default quads).
    let mut formulas: BTreeMap<String, Vec<&N3Quad>> = BTreeMap::new();
    for q in &quads {
        if let Some(key) = graph_name_key(&q.graph_name) {
            formulas.entry(key).or_default().push(q);
        }
    }

    // 3) Render each referenced formula to SPARQL term-syntax lines, enforcing the blank-node
    //    rule: `deletes` / `where` may not introduce blank nodes (only `inserts` may).
    let conditions = match &where_ref {
        Some(k) => render_formula(formulas.get(k), false, "where")?,
        None => String::new(),
    };
    let deletes = match &deletes_ref {
        Some(k) => render_formula(formulas.get(k), false, "deletes")?,
        None => String::new(),
    };
    let inserts = match &inserts_ref {
        Some(k) => render_formula(formulas.get(k), true, "inserts")?,
        None => String::new(),
    };

    Ok(N3Patch {
        deletes,
        inserts,
        conditions,
        has_where: where_ref.is_some(),
    })
}

/// Renders one formula's quads to SPARQL term-syntax triple lines (`s p o .`), one per line. When
/// `allow_blanks` is false a blank node anywhere in the formula is a hard error (the delete/where
/// blank-node rule). The N3-term Display is N-Triples/SPARQL term syntax, which is exactly what a
/// SPARQL `DELETE`/`INSERT`/`WHERE` block accepts.
fn render_formula(
    quads: Option<&Vec<&N3Quad>>,
    allow_blanks: bool,
    prop: &'static str,
) -> Result<String, N3PatchError> {
    let mut out = String::new();
    let empty = Vec::new();
    for q in quads.unwrap_or(&empty) {
        if !allow_blanks
            && (matches!(q.subject, N3Term::BlankNode(_))
                || matches!(q.object, N3Term::BlankNode(_)))
        {
            return Err(N3PatchError::BlankNodeInPattern(prop));
        }
        let s = render_term(&q.subject);
        let p = render_term(&q.predicate);
        let o = render_term(&q.object);
        out.push_str(&s);
        out.push(' ');
        out.push_str(&p);
        out.push(' ');
        out.push_str(&o);
        out.push_str(" .\n");
    }
    Ok(out)
}

/// SPARQL/N-Triples term syntax for an [`N3Term`]. oxttl/oxrdf Display already emits canonical
/// term syntax (`<iri>`, `"lit"^^<dt>`, `"lit"@lang`, `_:b`, `?var`, RDF-star `<< … >>`), which a
/// SPARQL block accepts verbatim — no escaping needed beyond what Display already does.
fn render_term(t: &N3Term) -> String {
    t.to_string()
}

/// A stable key for the patch subject (for the single-subject check). Blank nodes and IRIs print
/// distinctly, so the printed form is a sound identity key here.
fn term_key(t: &N3Term) -> String {
    t.to_string()
}

/// If `object` names a formula graph (a blank node minted by oxttl for a `{ … }` block), the
/// graph-name key. A literal / IRI object is NOT a formula → `None`.
fn formula_object_key(object: &N3Term) -> Option<String> {
    match object {
        N3Term::BlankNode(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The graph-name key for a quad's graph (the blank node oxttl assigned to a `{ … }` formula).
/// The default graph (top-level statements) has no key.
fn graph_name_key(g: &GraphName) -> Option<String> {
    match g {
        GraphName::BlankNode(b) => Some(b.to_string()),
        // A formula could in principle be named by an IRI graph; oxttl mints blank nodes for
        // `{ … }` blocks, so this is the live path. Handle the IRI case for completeness.
        GraphName::NamedNode(n) => Some(n.to_string()),
        GraphName::DefaultGraph => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_where_delete_insert() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:patch a solid:InsertDeletePatch;
  solid:where   { ?person ex:name "Jane". };
  solid:deletes { ?person ex:age 12. };
  solid:inserts { ?person ex:age 13. }.
"#;
        let p = parse(doc, None).unwrap();
        assert!(p.has_where);
        assert!(p.conditions.contains("?person"));
        assert!(p.conditions.contains("<http://ex/name>"));
        assert!(p.deletes.contains("<http://ex/age>"));
        assert!(p.inserts.contains("<http://ex/age>"));
        // The integer literal round-trips as a typed literal.
        assert!(p.deletes.contains("12"));
        assert!(p.inserts.contains("13"));
    }

    #[test]
    fn ground_insert_only_no_where() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o. }.
"#;
        let p = parse(doc, None).unwrap();
        assert!(!p.has_where);
        assert!(p.deletes.is_empty());
        assert!(p.inserts.contains("<http://ex/s> <http://ex/p> <http://ex/o> ."));
    }

    #[test]
    fn iri_named_patch_resolves_against_base() {
        // `<#patch>` is relative; the base IRI must resolve it (else oxttl errors "no scheme").
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
<#patch> a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o. }.
"#;
        let p = parse(doc, Some("http://example.org/doc")).unwrap();
        assert!(p.inserts.contains("<http://ex/s>"));
    }

    #[test]
    fn rejects_no_patch() {
        let doc = br#"@prefix ex: <http://ex/>.
ex:s ex:p ex:o.
"#;
        assert!(matches!(parse(doc, None), Err(N3PatchError::NoPatch)));
    }

    #[test]
    fn rejects_empty_patch() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
_:p a solid:InsertDeletePatch.
"#;
        assert!(matches!(parse(doc, None), Err(N3PatchError::Empty)));
    }

    #[test]
    fn rejects_duplicate_inserts() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:inserts { ex:s ex:p ex:o. };
  solid:inserts { ex:s ex:p ex:o2. }.
"#;
        assert!(matches!(
            parse(doc, None),
            Err(N3PatchError::DuplicateProperty("inserts"))
        ));
    }

    #[test]
    fn rejects_blank_node_in_deletes() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:deletes { [ ex:p ex:o ]. }.
"#;
        assert!(matches!(
            parse(doc, None),
            Err(N3PatchError::BlankNodeInPattern("deletes"))
        ));
    }

    #[test]
    fn allows_blank_node_in_inserts() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:inserts { ex:s ex:p [ ex:q ex:r ]. }.
"#;
        let p = parse(doc, None).unwrap();
        assert!(p.inserts.contains("<http://ex/q>"));
    }

    #[test]
    fn rejects_two_patch_resources() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
ex:p1 a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o. }.
ex:p2 a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o2. }.
"#;
        assert!(matches!(
            parse(doc, None),
            Err(N3PatchError::MultiplePatches)
        ));
    }

    /// Load-bearing INJECTION-SAFETY invariant: a literal whose lexical form contains the
    /// characters that would break out of a generated SPARQL block (`"`, newline, `}`) is escaped
    /// by the oxttl term Display, so the rendered triple line cannot inject a second clause. We
    /// assert the raw `}` / unescaped newline do NOT appear bare in the rendered insert.
    #[test]
    fn literal_with_break_chars_is_escaped() {
        let doc = "@prefix solid: <http://www.w3.org/ns/solid/terms#>.\n\
@prefix ex: <http://ex/>.\n\
_:p a solid:InsertDeletePatch;\n\
  solid:inserts { ex:s ex:p \"evil}\\n} INSERT DATA { ex:x ex:y ex:z } #\" . }.\n";
        let p = parse(doc.as_bytes(), None).unwrap();
        // The malicious payload is INSIDE a single escaped literal, so a bare `}` that would close
        // the INSERT block early is not present as an unescaped delimiter, and the injected
        // `INSERT DATA` is part of the literal lexical, not a second operation.
        assert!(p.inserts.contains("<http://ex/s>"));
        // The dangerous `}` and newline survive only in escaped form (`\}` is not a thing; oxttl
        // escapes `}`-in-literal as a literal char inside the quotes, and the newline as `\n`).
        assert!(
            p.inserts.contains("\\n"),
            "newline in literal must be escaped: {}",
            p.inserts
        );
        // The whole insert is exactly ONE triple line (one trailing " .\n"), not two.
        assert_eq!(
            p.inserts.matches(" .\n").count(),
            1,
            "must render exactly one triple: {}",
            p.inserts
        );
    }

    #[test]
    fn rejects_malformed_n3() {
        let doc = br#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
_:p a solid:InsertDeletePatch; solid:inserts { this is not n3 "#;
        assert!(matches!(parse(doc, None), Err(N3PatchError::Parse(_))));
    }
}

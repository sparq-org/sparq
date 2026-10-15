//! sparq-dl-wasm: the OWL 2 DL reasoner (`sparq-reason-dl`'s `owl2dl` module) compiled to
//! WebAssembly — the bundle behind `checkOwlDl` in the `@sparq-org/sparq` npm package.
//!
//! A SEPARATE bundle from the lean `sparq-wasm` triplestore bundle: the npm package loads it
//! with a dynamic `import()` on the first `checkOwlDl` call, so a page that never calls it
//! never fetches it.
//!
//! The one export, [`check_owl_dl`] (`checkOwlDl` in JS), parses a TBox document and an
//! optional ABox document into one graph, reads it as an OWL 2 ontology (the fail-closed
//! reverse RDF mapping), runs the OWL 2 DL profile check, consistency and classification,
//! and returns the result as a JSON string. Every answer is three-valued: `"unknown"` comes
//! with the reason and, where there are any, the offending input triples as N-Triples lines.
//!
//! No serde: the JSON is written by hand (the private `json` module), and all the logic is in
//! [`check_owl_dl_json`], which is plain Rust and unit-tested natively; the
//! `#[wasm_bindgen]` export is a thin error-mapping wrapper.
//!
//! ```js
//! import init, { checkOwlDl } from "./sparq_dl_wasm.js";
//! await init();
//! const result = JSON.parse(checkOwlDl(tboxTurtle, aboxTurtle, "turtle", undefined));
//! ```
#![forbid(unsafe_code)]

mod json;

use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple};
use sparq_core::dict::{Dict, Id, NO_ID};
use sparq_core::Graph;
use sparq_reason_dl::owl2dl::{
    check, extract, Consistency, DlProfileReport, DlReport, ExtractError, ExtractErrorKind,
    UnknownReason, Violation, ViolationKind,
};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// Parses `tbox` (and `abox`, if given) in `format` into ONE dictionary and triple list, the
/// TBox triples first. The two documents are separate RDF documents, so a blank-node label
/// of the ABox that also occurs in the TBox is renamed: blank nodes never join across
/// documents.
fn parse_input(
    tbox: &str,
    abox: Option<&str>,
    format: &str,
) -> Result<(Dict, Vec<[Id; 3]>), String> {
    let (mut dict, mut triples) =
        Graph::parse_to_triples(tbox, format).map_err(|e| format!("TBox parse error: {e}"))?;
    let Some(abox) = abox else {
        return Ok((dict, triples));
    };
    let (adict, atriples) =
        Graph::parse_to_triples(abox, format).map_err(|e| format!("ABox parse error: {e}"))?;
    let mut ids: HashMap<Id, Id> = HashMap::new();
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut fresh = 0usize;
    triples.reserve(atriples.len());
    for t in atriples {
        let mut out = [NO_ID; 3];
        for (slot, id) in out.iter_mut().zip(t) {
            *slot = match ids.get(&id) {
                Some(&mapped) => mapped,
                None => {
                    let term = rename_blanks(adict.term(id), &mut |label| {
                        blank_label(&dict, &adict, &mut labels, &mut fresh, label)
                    });
                    let mapped = dict.intern(&term);
                    ids.insert(id, mapped);
                    mapped
                }
            };
        }
        triples.push(out);
    }
    Ok((dict, triples))
}

/// The label an ABox blank node gets in the merged graph: its own, unless the TBox already
/// uses it; then a fresh one used by neither document. Decided once per label.
fn blank_label(
    merged: &Dict,
    abox: &Dict,
    labels: &mut HashMap<String, String>,
    fresh: &mut usize,
    label: &str,
) -> String {
    if let Some(l) = labels.get(label) {
        return l.clone();
    }
    let used = |d: &Dict, l: &str| d.lookup(&Term::BlankNode(BlankNode::new_unchecked(l))) != NO_ID;
    // The first time a label is seen, `merged` holds the TBox, the ABox labels already kept
    // (all different from this one) and fresh labels (never an ABox label), so `used` here
    // means "used by the TBox".
    let chosen = if used(merged, label) {
        loop {
            *fresh += 1;
            let candidate = format!("{label}_{fresh}");
            if !used(merged, &candidate) && !used(abox, &candidate) {
                break candidate;
            }
        }
    } else {
        label.to_owned()
    };
    labels.insert(label.to_owned(), chosen.clone());
    chosen
}

/// Applies `rename` to every blank node in `term` (inside RDF 1.2 triple terms too).
fn rename_blanks(term: Term, rename: &mut impl FnMut(&str) -> String) -> Term {
    match term {
        Term::BlankNode(b) => Term::BlankNode(BlankNode::new_unchecked(rename(b.as_str()))),
        Term::Triple(t) => {
            let Triple {
                subject,
                predicate,
                object,
            } = *t;
            let subject = match subject {
                NamedOrBlankNode::BlankNode(b) => {
                    NamedOrBlankNode::BlankNode(BlankNode::new_unchecked(rename(b.as_str())))
                }
                named => named,
            };
            Term::Triple(Box::new(Triple {
                subject,
                predicate,
                object: rename_blanks(object, rename),
            }))
        }
        other => other,
    }
}

/// A term as a plain string: an IRI without angle brackets, anything else in N-Triples.
fn plain(dict: &Dict, id: Id) -> String {
    match dict.term(id) {
        Term::NamedNode(n) => n.into_string(),
        other => other.to_string(),
    }
}

/// The input triples at `indices` as N-Triples lines.
fn ntriples(dict: &Dict, triples: &[[Id; 3]], indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .filter_map(|&i| triples.get(i))
        .map(|&[s, p, o]| format!("{} {} {} .", dict.term(s), dict.term(p), dict.term(o)))
        .collect()
}

/// The stable JSON name of a profile-violation kind.
fn violation_kind(kind: ViolationKind) -> &'static str {
    match kind {
        ViolationKind::Undeclared => "Undeclared",
        ViolationKind::IllegalPunning => "IllegalPunning",
        ViolationKind::ReservedVocabulary => "ReservedVocabulary",
        ViolationKind::NonSimpleProperty => "NonSimpleProperty",
        ViolationKind::IrregularPropertyHierarchy => "IrregularPropertyHierarchy",
        ViolationKind::AnonymousIndividualCycle => "AnonymousIndividualCycle",
        ViolationKind::IllTypedLiteral => "IllTypedLiteral",
        ViolationKind::UnknownDatatype => "UnknownDatatype",
        ViolationKind::InvalidFacet => "InvalidFacet",
        ViolationKind::InvalidDatatypeDefinition => "InvalidDatatypeDefinition",
        ViolationKind::TopDataPropertyMisuse => "TopDataPropertyMisuse",
        ViolationKind::ExtractionNote => "ExtractionNote",
        ViolationKind::InsufficientOperands => "InsufficientOperands",
    }
}

/// The stable JSON name of an extraction-refusal kind.
fn extract_kind(kind: ExtractErrorKind) -> &'static str {
    match kind {
        ExtractErrorKind::Unparsed => "Unparsed",
        ExtractErrorKind::Malformed => "Malformed",
        ExtractErrorKind::Ambiguous => "Ambiguous",
        ExtractErrorKind::Unsupported => "Unsupported",
    }
}

/// The stable JSON name of a no-verdict reason.
fn reason_kind(reason: &UnknownReason) -> &'static str {
    match reason {
        UnknownReason::OutOfFragment { .. } => "OutOfFragment",
        UnknownReason::NotInDl(_) => "NotInDl",
        UnknownReason::ResourceBudget => "ResourceBudget",
        UnknownReason::ImportsNotLoaded(_) => "ImportsNotLoaded",
        UnknownReason::DatatypeUnknown { .. } => "DatatypeUnknown",
        UnknownReason::Unconfirmed => "Unconfirmed",
    }
}

/// Writes JSON for the input data the report refers to.
struct Writer<'a> {
    dict: &'a Dict,
    triples: &'a [[Id; 3]],
    out: String,
}

impl Writer<'_> {
    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn key(&mut self, k: &str) {
        json::push_str(&mut self.out, k);
        self.out.push(':');
    }

    fn str(&mut self, s: &str) {
        json::push_str(&mut self.out, s);
    }

    fn triples(&mut self, indices: &[usize]) {
        let lines = ntriples(self.dict, self.triples, indices);
        json::push_str_array(&mut self.out, lines.iter().map(String::as_str));
    }

    fn violation(&mut self, v: &Violation) {
        self.raw("{");
        self.key("kind");
        self.str(violation_kind(v.kind));
        self.raw(",");
        self.key("detail");
        self.str(&v.message);
        self.raw(",");
        self.key("triples");
        self.triples(&v.source);
        self.raw("}");
    }

    fn violations<'v>(&mut self, vs: impl IntoIterator<Item = &'v Violation>) {
        self.raw("[");
        for (i, v) in vs.into_iter().enumerate() {
            if i > 0 {
                self.raw(",");
            }
            self.violation(v);
        }
        self.raw("]");
    }

    fn reason(&mut self, reason: &UnknownReason) {
        self.raw("{");
        self.key("kind");
        self.str(reason_kind(reason));
        match reason {
            UnknownReason::OutOfFragment { construct, source } => {
                self.raw(",");
                self.key("construct");
                self.str(construct);
                self.raw(",");
                self.key("triples");
                self.triples(source);
            }
            UnknownReason::NotInDl(blocking) => {
                self.raw(",");
                self.key("violations");
                self.violations(blocking);
            }
            UnknownReason::ImportsNotLoaded(imports) => {
                let mut iris: Vec<String> = imports.iter().map(|&i| plain(self.dict, i)).collect();
                iris.sort_unstable();
                self.raw(",");
                self.key("imports");
                json::push_str_array(&mut self.out, iris.iter().map(String::as_str));
            }
            UnknownReason::DatatypeUnknown { source } => {
                self.raw(",");
                self.key("triples");
                self.triples(source);
            }
            UnknownReason::ResourceBudget | UnknownReason::Unconfirmed => {}
        }
        self.raw("}");
    }

    fn refusal(&mut self, e: &ExtractError) {
        self.raw("{");
        self.key("kind");
        self.str(extract_kind(e.kind));
        self.raw(",");
        self.key("detail");
        self.str(&e.detail);
        self.raw(",");
        self.key("triples");
        self.triples(&e.triples);
        self.raw("}");
    }

    fn profile(&mut self, p: Option<&DlProfileReport>) {
        self.raw("{");
        self.key("dl");
        match p.and_then(DlProfileReport::verdict) {
            Some(true) => self.raw("true"),
            Some(false) => self.raw("false"),
            None => self.str("unknown"),
        }
        self.raw(",");
        self.key("violations");
        self.violations(p.map_or(&[][..], |p| &p.violations));
        self.raw(",");
        self.key("unchecked");
        self.raw("[");
        for (i, u) in p.map_or(&[][..], |p| &p.unchecked).iter().enumerate() {
            if i > 0 {
                self.raw(",");
            }
            self.raw("{");
            self.key("rule");
            self.str(u.rule);
            self.raw(",");
            self.key("triples");
            self.triples(&u.source);
            self.raw("}");
        }
        self.raw("]}");
    }

    /// The whole result object, for a checked ontology (`Ok`) or an extraction refusal.
    fn result(&mut self, outcome: Result<&DlReport, &ExtractError>) {
        self.raw("{");
        self.key("consistent");
        match outcome {
            Ok(r) => match &r.consistency {
                Consistency::Consistent => self.raw("true,\"reason\":null"),
                Consistency::Inconsistent => self.raw("false,\"reason\":null"),
                Consistency::Unknown(reason) => {
                    self.str("unknown");
                    self.raw(",");
                    self.key("reason");
                    self.reason(reason);
                }
            },
            Err(e) => {
                self.str("unknown");
                self.raw(",");
                self.key("reason");
                self.refusal(e);
            }
        }
        self.raw(",");
        self.key("profile");
        self.profile(outcome.ok().map(|r| &r.profile));

        let classification = outcome.ok().and_then(|r| r.classification.as_ref());
        let mut unsat: Vec<String> = classification
            .map(|c| {
                c.unsatisfiable
                    .iter()
                    .map(|&i| plain(self.dict, i))
                    .collect()
            })
            .unwrap_or_default();
        unsat.sort_unstable();
        let mut inferred: Vec<(String, String)> = classification
            .map(|c| {
                c.inferred
                    .iter()
                    .map(|&(a, b)| (plain(self.dict, a), plain(self.dict, b)))
                    .collect()
            })
            .unwrap_or_default();
        inferred.sort_unstable();
        self.raw(",");
        self.key("unsatisfiable");
        json::push_str_array(&mut self.out, unsat.iter().map(String::as_str));
        self.raw(",");
        self.key("inferredSubClassOf");
        self.raw("[");
        for (i, (a, b)) in inferred.iter().enumerate() {
            if i > 0 {
                self.raw(",");
            }
            json::push_str_array(&mut self.out, [a.as_str(), b.as_str()]);
        }
        self.raw("]");
        self.raw(",");
        self.key("classification");
        match outcome {
            Ok(DlReport {
                classification: Some(_),
                ..
            }) => self.str("complete"),
            Ok(DlReport {
                classification_unknown: Some(reason),
                ..
            }) => {
                self.raw("{");
                self.key("unknown");
                self.str(reason_kind(reason));
                self.raw("}");
            }
            _ => self.raw("null"),
        }
        self.raw("}");
    }
}

/// Checks an OWL 2 ontology and returns the result as a JSON string (the shape is
/// documented in the crate README and in `skills/javascript-wasm/SKILL.md`).
///
/// `tbox` and the optional `abox` are parsed in `format` (`"turtle"` when empty; any format
/// `sparq_core::Graph::parse_to_triples` accepts in this build: `"turtle"`, `"ntriples"`,
/// `"nquads"`, `"trig"` — named graphs are folded into the default graph) and merged into
/// one graph. `options` is `null`/absent or `{"budget":{"maxSteps":N,"maxNodes":N}}`.
///
/// # Errors
/// A parse error in either document, or malformed options. An ontology the reverse mapping
/// refuses is NOT an error: it answers `"consistent":"unknown"` with the refusal as reason.
pub fn check_owl_dl_json(
    tbox: &str,
    abox: Option<&str>,
    format: &str,
    options: Option<&str>,
) -> Result<String, String> {
    let budget = json::budget_from_options(options)?;
    let format = if format.is_empty() { "turtle" } else { format };
    let (dict, triples) = parse_input(tbox, abox, format)?;
    let mut w = Writer {
        dict: &dict,
        triples: &triples,
        out: String::new(),
    };
    match extract(&dict, &triples) {
        Ok(onto) => {
            let report = check(&dict, &onto, budget);
            w.result(Ok(&report));
        }
        Err(e) => w.result(Err(&e)),
    }
    Ok(w.out)
}

/// JS: `checkOwlDl(tbox, abox?, format, optionsJson?) -> string` — the JSON result of
/// [`check_owl_dl_json`]. Throws on a parse error or malformed options.
#[wasm_bindgen(js_name = checkOwlDl)]
pub fn check_owl_dl(
    tbox: &str,
    abox: Option<String>,
    format: &str,
    options_json: Option<String>,
) -> Result<String, JsError> {
    check_owl_dl_json(tbox, abox.as_deref(), format, options_json.as_deref())
        .map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests;

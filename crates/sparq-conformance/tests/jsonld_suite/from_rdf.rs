//! [FABLE-5] sq-oy1f.28 — the W3C JSON-LD 1.1 `fromRdf` lane runner, flipped to the
//! NATIVE document-level pipeline (`sparq_jsonld::from_rdf`, JSON-LD API §8.1).
//!
//! **Oracle (strengthened vs the old engine-writer round-trip lane):**
//!
//! 1. **Document-level comparison (normative, every positive case).** The input
//!    `.nq` is parsed with `oxttl` (labels preserved — §8.1 output is
//!    label-faithful), converted through the REAL native path
//!    `sparq_jsonld::from_rdf` with the case's manifest options (`useNativeTypes`,
//!    `useRdfType`, `rdfDirection`), and deep-compared against the suite's
//!    normative expected expanded document via `json_ld_equal` (key order
//!    insignificant; array order significant only inside `@list`).
//! 2. **Round-trip isomorphism (kept, scoped to where §8.1 is lossless).** The
//!    produced document is re-parsed through `oxjsonld` and required to
//!    reconstruct the input dataset (blank-node-blind canonical comparison).
//!    The leg is skipped — never silently weakened — exactly where the spec
//!    itself is lossy or the re-parser cannot express the construct:
//!    * option-bearing cases (`useNativeTypes` rewrites lexical forms;
//!      `rdfDirection` decodes into `@direction`, which `oxjsonld` does not
//!      re-serialize);
//!    * documents containing `@json` values (re-serializing them to canonical
//!      `rdf:JSON` literals is the `toRdf` side, bead sq-oy1f.30);
//!    * inputs typing collapsed list cells `rdf:type rdf:List` (§8.1 consumes the
//!      redundant type triples by design — fromRdf/0016).
//! 3. **NegativeEvaluationTests are now REAL** (previously vacuously round-trip
//!    "passed"): the raised `JsonLdErrorCode` must equal the manifest's
//!    `expectErrorCode` (`invalid JSON literal` ×2).
//!
//! **Honest SKIP bucket:** `option.specVersion == json-ld-1.0` cases (fromRdf/0008)
//! — those pin the JSON-LD **1.0** algorithm's partial list conversion, which a 1.1
//! processor intentionally does not reproduce (suite convention: 1.0-only tests are
//! run by 1.0 processors). The old lane counted 0008 as a round-trip pass; the
//! re-pinned floor accounts for the flip to an honest skip (see
//! `src/floors/from_rdf.rs` for the side-by-side).

use super::common::*;
use serde_json::Value;
use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
use sparq_jsonld::RdfDirection;
use std::collections::HashMap;
use std::path::Path;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_LIST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#List";

/// The §8.1-specific manifest options (`useNativeTypes`, `useRdfType`,
/// `rdfDirection`) per test id. `common::Entry` carries only the cross-lane
/// fields, so the lane reads its own extras straight from the manifest — keeping
/// this bead's changes inside its own files (fleet contract, sq-oy1f.28).
fn fromrdf_case_options(root: &Path) -> HashMap<String, (bool, bool, RdfDirection)> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("fromRdf-manifest.jsonld")) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return out;
    };
    let Some(seq) = v.get("sequence").and_then(Value::as_array) else {
        return out;
    };
    for e in seq {
        let Some(id) = e.get("@id").and_then(Value::as_str) else {
            continue;
        };
        let opt = e.get("option");
        let flag = |name: &str| {
            opt.and_then(|o| o.get(name))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let direction = match opt
            .and_then(|o| o.get("rdfDirection"))
            .and_then(Value::as_str)
        {
            Some("i18n-datatype") => RdfDirection::I18nDatatype,
            Some("compound-literal") => RdfDirection::CompoundLiteral,
            _ => RdfDirection::None,
        };
        out.insert(
            id.to_string(),
            (flag("useNativeTypes"), flag("useRdfType"), direction),
        );
    }
    out
}

/// Parse the case's N-Quads into the native term model, PRESERVING blank-node
/// labels (the §8.1 node map is label-faithful and the expected documents
/// reference the input labels — no canonicalization here).
fn nquads_to_model(text: &str) -> Result<Vec<RdfQuad>, String> {
    let mut out = Vec::new();
    for q in oxttl::NQuadsParser::new().for_slice(text.as_bytes()) {
        let q = q.map_err(|e| format!("{}", e))?;
        let subject = match q.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => RdfTerm::iri(n.into_string()),
            oxrdf::NamedOrBlankNode::BlankNode(b) => RdfTerm::blank(b.into_string()),
        };
        let predicate = RdfTerm::iri(q.predicate.into_string());
        let object = match q.object {
            oxrdf::Term::NamedNode(n) => RdfTerm::iri(n.into_string()),
            oxrdf::Term::BlankNode(b) => RdfTerm::blank(b.into_string()),
            oxrdf::Term::Literal(lit) => {
                if let Some(lang) = lit.language() {
                    RdfTerm::lang_literal(lit.value(), lang)
                } else {
                    RdfTerm::typed_literal(lit.value(), lit.datatype().as_str())
                }
            }
            other => return Err(format!("unsupported object term: {}", other)),
        };
        let graph = match q.graph_name {
            oxrdf::GraphName::DefaultGraph => None,
            oxrdf::GraphName::NamedNode(n) => Some(RdfTerm::iri(n.into_string())),
            oxrdf::GraphName::BlankNode(b) => Some(RdfTerm::blank(b.into_string())),
        };
        out.push(RdfQuad::new(subject, predicate, object, graph));
    }
    Ok(out)
}

/// Whether the round-trip leg applies (see the module docs: the leg is scoped to
/// where §8.1 is lossless AND `oxjsonld` can re-parse the produced constructs).
fn round_trip_applies(opts: &FromRdfOptions, quads: &[RdfQuad], doc_text: &str) -> bool {
    if opts.use_native_types || opts.rdf_direction != RdfDirection::None {
        return false;
    }
    // @json value objects: canonical rdf:JSON re-serialization is the toRdf side.
    if doc_text.contains(r#""@type":"@json""#) {
        return false;
    }
    // rdf:type rdf:List on collapsed cells is consumed by §8.1 (fromRdf/0016).
    let types_a_list = quads
        .iter()
        .any(|q| q.predicate == RdfTerm::iri(RDF_TYPE) && q.object == RdfTerm::iri(RDF_LIST));
    !(types_a_list && doc_text.contains(r#""@list""#))
}

/// Run the fromRdf category through the NATIVE §8.1 path: document-level
/// comparison on every case, plus the scoped round-trip leg, plus real
/// negative-test error-code assertions.
pub fn run_fromrdf(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "fromRdf") {
        Ok(e) => e,
        Err(why) => {
            s.fail("fromRdf-manifest", why);
            return s;
        }
    };
    let case_options = fromrdf_case_options(root);
    for e in &entries {
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        // JSON-LD-1.0-only cases pin the 1.0 algorithm; a 1.1 processor skips them.
        if e.spec_version.as_deref() == Some("json-ld-1.0") {
            s.skip();
            continue;
        }
        let input_path = root.join(&e.input);
        let nq_text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read input: {}", why));
                continue;
            }
        };
        let quads = match nquads_to_model(&nq_text) {
            Ok(q) => q,
            Err(why) => {
                s.fail(&e.id, format!("parse input nq: {}", why));
                continue;
            }
        };
        let (use_native_types, use_rdf_type, rdf_direction) = case_options
            .get(&e.id)
            .copied()
            .unwrap_or((false, false, RdfDirection::None));
        let mut opts = FromRdfOptions::default();
        opts.use_native_types = use_native_types;
        opts.use_rdf_type = use_rdf_type;
        opts.rdf_direction = rdf_direction;
        opts.ordered = true;

        let outcome = from_rdf(&quads, &opts);

        // NegativeEvaluationTests: the exact spec error code must be raised.
        if e.is_negative {
            match (&outcome, e.expect_error_code.as_deref()) {
                (Err(err), Some(want)) if err.code().as_str() == want => s.pass(),
                (Err(err), want) => s.fail(
                    &e.id,
                    format!("raised {:?}, expected {:?}", err.code().as_str(), want),
                ),
                (Ok(_), want) => {
                    s.fail(&e.id, format!("expected error {:?}, got a document", want))
                }
            }
            continue;
        }

        let doc = match outcome {
            Ok(d) => d,
            Err(err) => {
                s.fail(&e.id, format!("from_rdf error: {}", err));
                continue;
            }
        };
        // 1. Document-level comparison against the normative expected document.
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
        let got: Value = match sparq_json_to_serde(&doc) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("convert output: {}", why));
                continue;
            }
        };
        let want: Value = match std::fs::read_to_string(root.join(expect_rel))
            .map_err(|e| format!("{}", e))
            .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("{}", e)))
        {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("read/parse expect: {}", why));
                continue;
            }
        };
        if !json_ld_equal(&got, &want) {
            s.fail(
                &e.id,
                "document mismatch vs normative expected doc".to_string(),
            );
            continue;
        }
        // 2. Round-trip isomorphism, where §8.1 is lossless (see module docs).
        let mut doc_text = String::new();
        doc.write(&mut doc_text);
        if round_trip_applies(&opts, &quads, &doc_text) {
            let round_trip = jsonld_to_canonical_dataset(&doc_text, SUITE_BASE)
                .and_then(|got_ds| nquads_to_canonical_dataset(&nq_text).map(|w| (got_ds, w)));
            match round_trip {
                Ok((got_ds, want_ds)) if got_ds == want_ds => {}
                Ok((got_ds, want_ds)) => {
                    s.fail(
                        &e.id,
                        format!(
                            "round-trip mismatch ({} vs {} quads)",
                            got_ds.len(),
                            want_ds.len()
                        ),
                    );
                    continue;
                }
                Err(why) => {
                    s.fail(&e.id, format!("round-trip re-parse: {}", why));
                    continue;
                }
            }
        }
        s.pass();
    }
    s
}

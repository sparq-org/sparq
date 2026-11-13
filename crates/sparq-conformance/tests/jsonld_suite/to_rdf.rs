//! sq-oy1f.40 — the W3C JSON-LD 1.1 `toRdf` lane runner (split out of the
//! former monolithic `tests/jsonld_suite.rs`; behaviour byte-identical).
//!
//! JSON-LD 1.1 entries use native sparq-jsonld conversion, with canonical RDF
//! dataset comparison, exact negative codes, and positive syntax acceptance.
//! Legacy 1.0-only entries retain their previous sparq-core/oxjsonld path.

use super::common::*;
use std::path::Path;

/// Run the toRdf category. Returns the scoreboard.
pub fn run_tordf(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "toRdf") {
        Ok(e) => e,
        Err(why) => {
            s.fail("toRdf-manifest", why);
            return s;
        }
    };
    for e in &entries {
        // These legacy generalized-RDF fixtures have no oxrdf dataset representation.
        if e.requires.as_deref() == Some("GeneralizedRdf") {
            s.skip();
            continue;
        }
        let input_path = root.join(&e.input);
        let text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read input: {why}"));
                continue;
            }
        };
        let base = doc_base(e);
        // Keep the legacy compatibility lane; 1.1 uses the native document processor.
        let parsed = if e.spec_version.as_deref() == Some("json-ld-1.0")
            || e.processing_mode.as_deref() == Some("json-ld-1.0")
        {
            parse_jsonld_dataset(&text, &base)
        } else {
            let mut options = sparq_jsonld::JsonLdOptions::default();
            options.base = Some(base);
            options.rdf_direction = match e
                .options
                .get("rdfDirection")
                .and_then(serde_json::Value::as_str)
            {
                Some("i18n-datatype") => sparq_jsonld::RdfDirection::I18nDatatype,
                Some("compound-literal") => sparq_jsonld::RdfDirection::CompoundLiteral,
                _ => sparq_jsonld::RdfDirection::None,
            };
            if let Some(context) = &e.expand_context_path {
                let parsed = std::fs::read_to_string(root.join(context))
                    .map_err(|e| e.to_string())
                    .and_then(|text| sparq_jsonld::Json::parse(&text).map_err(|e| e.to_string()));
                match parsed {
                    Ok(context) => options.expand_context = Some(context),
                    Err(why) => {
                        s.fail(&e.id, format!("expandContext: {why}"));
                        continue;
                    }
                }
            }
            let loader = super::remote::ManifestLoader::new(root, e);
            let outcome = sparq_jsonld::Json::parse(&text)
                .map_err(|e| {
                    sparq_jsonld::JsonLdError::with_detail(
                        sparq_jsonld::JsonLdErrorCode::LoadingDocumentFailed,
                        e.to_string(),
                    )
                })
                .and_then(|input| native_dataset(&input, &options, &loader));
            if e.is_negative {
                let want = e.expect_error_code.as_deref().unwrap_or("");
                match outcome {
                    Err(error) if error.code().as_str() == want => s.pass(&e.id),
                    other => s.fail(&e.id, format!("expected {want}, got {other:?}")),
                }
                continue;
            }
            outcome.map_err(|e| e.to_string())
        };
        if e.is_negative {
            // A NegativeEvaluationTest passes iff the parse fails (sparq
            // rejects the malformed/illegal document).
            match parsed {
                Err(_) => s.pass(&e.id),
                Ok(_) => s.fail(&e.id, "negative test parsed without error".into()),
            }
            continue;
        }
        let Some(expect) = &e.expect else {
            if e.types
                .iter()
                .any(|kind| kind.ends_with(":PositiveSyntaxTest"))
            {
                match parsed {
                    Ok(_) => s.pass(&e.id),
                    Err(why) => s.fail(&e.id, why),
                }
            } else {
                s.fail(&e.id, "positive evaluation has no expected output".into());
            }
            continue;
        };
        let got = match parsed {
            Ok(ds) => ds,
            Err(why) => {
                s.fail(&e.id, format!("parse: {why}"));
                continue;
            }
        };
        let exp_text = match std::fs::read_to_string(root.join(expect)) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read expect: {why}"));
                continue;
            }
        };
        let want = match nquads_to_canonical_dataset(&exp_text) {
            Ok(ds) => ds,
            Err(why) => {
                s.fail(&e.id, format!("parse expect: {why}"));
                continue;
            }
        };
        if got == want {
            s.pass(&e.id);
        } else {
            s.fail(
                &e.id,
                format!("RDF mismatch ({} vs {} quads)", got.len(), want.len()),
            );
        }
    }
    s
}

/// Convert the native RDF model to the canonical dataset used by the W3C oracle.
pub fn native_dataset(
    input: &sparq_jsonld::Json,
    options: &sparq_jsonld::JsonLdOptions,
    loader: &dyn sparq_jsonld::DocumentLoader,
) -> Result<oxrdf::Dataset, sparq_jsonld::JsonLdError> {
    use sparq_jsonld::from_rdf::RdfTerm;
    fn term(t: RdfTerm) -> oxrdf::Term {
        match t {
            RdfTerm::Iri(iri) => oxrdf::NamedNode::new_unchecked(iri).into(),
            RdfTerm::BlankNode(id) => oxrdf::BlankNode::new_unchecked(id).into(),
            RdfTerm::Literal {
                lexical,
                datatype,
                language,
            } => {
                if let Some(lang) = language {
                    oxrdf::Literal::new_language_tagged_literal_unchecked(lexical, lang).into()
                } else if let Some(datatype) = datatype {
                    oxrdf::Literal::new_typed_literal(
                        lexical,
                        oxrdf::NamedNode::new_unchecked(datatype),
                    )
                    .into()
                } else {
                    oxrdf::Literal::new_simple_literal(lexical).into()
                }
            }
        }
    }
    let quads = sparq_jsonld::to_rdf::to_rdf(input, options, loader)?;
    let mut dataset = oxrdf::Dataset::new();
    for q in quads {
        let subject = match term(q.subject) {
            oxrdf::Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
            oxrdf::Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
            _ => unreachable!(),
        };
        let oxrdf::Term::NamedNode(predicate) = term(q.predicate) else {
            return Err(sparq_jsonld::JsonLdError::with_detail(
                sparq_jsonld::JsonLdErrorCode::LoadingDocumentFailed,
                "generalized RDF has no oxrdf dataset representation",
            ));
        };
        let graph = match q.graph.map(term) {
            None => oxrdf::GraphName::DefaultGraph,
            Some(oxrdf::Term::NamedNode(n)) => oxrdf::GraphName::NamedNode(n),
            Some(oxrdf::Term::BlankNode(b)) => oxrdf::GraphName::BlankNode(b),
            _ => unreachable!(),
        };
        let quad = oxrdf::Quad::new(subject, predicate, term(q.object), graph);
        dataset.insert(&quad);
    }
    dataset.canonicalize(oxrdf::dataset::CanonicalizationAlgorithm::Unstable);
    Ok(dataset)
}

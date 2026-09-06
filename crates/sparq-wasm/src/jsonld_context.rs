//! [SONNET-4.6] sq-yz27r (#3251): the opt-in `Store.loadJsonLdWithContexts(text, contexts)`
//! binding — JSON-LD ingest for a document that references its `@context` **by URL**.
//!
//! ## The gap this closes
//!
//! The lean wasm JSON-LD ingest path ([`Store::load`] with `"jsonld"`, sq-dvyi) drives
//! `oxjsonld` with **no** `LoadDocumentCallback`. A document whose `@context` is a URL
//! rather than an inline object — which is how essentially every real Verifiable
//! Credential is written (`"@context": "https://www.w3.org/2018/credentials/v1"`) —
//! therefore fails to parse with `No LoadDocumentCallback has been set to load remote
//! contexts`. That is what the site's VC import surface hit (sq-3p0z / PR #1091) and
//! worked around with an inline-`@context` sample plus an import-as-Turtle path.
//!
//! ## What this module does — and, precisely, what it does not
//!
//! It installs a `LoadDocumentCallback` backed by a **caller-supplied map** of
//! `contextUrl -> contextDocumentText`. Resolution is **fail-closed**: a URL the caller
//! did not supply is refused with an error naming that URL. The supplied map *is* the
//! allowlist, so enabling this feature grants the wasm module **no ambient network** —
//! it never opens a socket, and the bundle links no HTTP/fetch code. This matches the
//! deny-by-default posture `sparq_jsonld::NoopLoader` documents for the native pipeline.
//!
//! **The fetching stays in JS, and that is a constraint, not a preference.** `oxjsonld`'s
//! `LoadDocumentCallback` is a *synchronous* `Fn(&str, &_) -> Result<JsonLdRemoteDocument, _>`
//! (`oxjsonld::SliceJsonLdParser::with_load_document_callback`). The browser's `fetch` is
//! async and cannot be awaited from inside a synchronous Rust callback running on the wasm
//! stack, so a "fetch-backed loader" cannot live on this side of the boundary at all
//! (short of synchronous `XMLHttpRequest`, which blocks the main thread and is deprecated).
//! The workable split is therefore: **the host fetches (async, under whatever same-origin /
//! allowlist / CSP policy it enforces), this binding parses.** A caller that wants zero
//! network keeps working by passing contexts it already has on hand — a bundled asset, a
//! cache, or `IndexedDB`.
//!
//! Behind the non-default `jsonld-contexts` Cargo feature (which implies `jsonld`), so the
//! lean default bundle links none of this and the `wasm_bundle_bytes` baseline is
//! unchanged.

use std::collections::HashMap;

use oxjsonld::{JsonLdParser, JsonLdRemoteDocument};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::Store;

/// Turns the JS `contexts` argument — an ordered `[[url, documentText], …]` array, the same
/// pair-array shape `Store::serialize`'s `prefixes` argument uses — into the lookup map the
/// loader closure captures. A malformed element (not a 2-string array) is rejected rather
/// than silently dropped, so a bad map surfaces instead of producing a wrongly-parsed graph.
/// A duplicate URL takes its LAST value (last write wins), matching `prefixes_from_pairs`.
fn contexts_from_pairs(contexts: &js_sys::Array) -> Result<HashMap<String, String>, JsError> {
    let mut map = HashMap::with_capacity(contexts.length() as usize);
    for (i, entry) in contexts.iter().enumerate() {
        let pair: js_sys::Array = entry.dyn_into().map_err(|_| {
            JsError::new(&format!(
                "contexts[{}] must be a [url, documentText] array of two strings",
                i
            ))
        })?;
        let url = pair.get(0).as_string().ok_or_else(|| {
            JsError::new(&format!("contexts[{}][0] (the context URL) must be a string", i))
        })?;
        let document = pair.get(1).as_string().ok_or_else(|| {
            JsError::new(&format!(
                "contexts[{}][1] (the context document text) must be a string",
                i
            ))
        })?;
        map.insert(url, document);
    }
    Ok(map)
}

/// Parses `text` as JSON-LD, resolving remote `@context` references from `contexts` only.
///
/// The pure, natively-testable core of [`Store::load_jsonld_with_contexts`]: it interns
/// straight into a [`Dict`] + triple vector and hands them to [`Graph::from_parts`], which
/// is the same shape `sparq_core::Graph::parse_to_triples` builds for the no-callback
/// JSON-LD arm. Named graphs (a JSON-LD `@graph` with an outer `@id`) are **folded into the
/// default graph**, matching [`Store::load`].
///
/// A referenced context URL absent from `contexts` makes the parse fail with a message
/// naming that URL — the fail-closed allowlist behaviour, never a silent partial parse.
pub(crate) fn parse_jsonld_with_contexts(
    text: &str,
    contexts: HashMap<String, String>,
) -> Result<Graph, String> {
    let mut dict = Dict::new();
    let mut triples: Vec<[Id; 3]> = Vec::new();

    let parser = JsonLdParser::new()
        .for_slice(text.as_bytes())
        .with_load_document_callback(move |url, _options| {
            match contexts.get(url) {
                Some(document) => Ok(JsonLdRemoteDocument {
                    document: document.as_bytes().to_vec(),
                    document_url: url.to_string(),
                }),
                // Fail-closed: the supplied map IS the allowlist. Name the URL so the
                // caller knows exactly which context it still has to fetch and pass in.
                None => Err(format!(
                    "remote @context <{}> was not supplied to loadJsonLdWithContexts (no network is available to the wasm module; fetch it in JS and pass it in the `contexts` array)",
                    url
                )
                .into()),
            }
        });

    for quad in parser {
        let quad = quad.map_err(|e| e.to_string())?;
        let subject = match &quad.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => oxrdf::Term::NamedNode(n.clone()),
            oxrdf::NamedOrBlankNode::BlankNode(b) => oxrdf::Term::BlankNode(b.clone()),
        };
        let s = dict.intern(&subject);
        let p = dict.intern(&oxrdf::Term::NamedNode(quad.predicate.clone()));
        let o = dict.intern(&quad.object);
        triples.push([s, p, o]);
    }

    Ok(Graph::from_parts(dict, triples))
}

#[wasm_bindgen]
impl Store {
    /// [SONNET-4.6] sq-yz27r (#3251): parses a JSON-LD document whose `@context` is given
    /// **by URL**, resolving those URLs from a caller-supplied `contexts` map.
    ///
    /// `contexts` is an ordered array of `[url, documentText]` string pairs — the context
    /// documents the host has already retrieved:
    ///
    /// ```js
    /// // The host fetches (async, under its OWN same-origin / allowlist policy) …
    /// const url = "https://www.w3.org/2018/credentials/v1";
    /// const ctx = await (await fetch(url)).text();
    /// // … and this binding parses, with no network of its own.
    /// const store = Store.loadJsonLdWithContexts(vcJsonText, [[url, ctx]]);
    /// ```
    ///
    /// **Fail-closed.** A `@context` URL the document references but `contexts` does not
    /// carry is an `Err` (`JsError`) naming that URL — the module has no network, so it
    /// cannot and does not go and get it. The supplied map is the whole allowlist.
    /// Consequently this method never performs I/O; it is the *parse* half of a
    /// fetch-then-parse split forced by `oxjsonld`'s synchronous `LoadDocumentCallback`
    /// (see the module docs). A malformed `contexts` element (not a 2-string array) also
    /// throws rather than being skipped.
    ///
    /// Named graphs are folded into the default graph, exactly as [`load`](Self::load)
    /// does; there is no dataset-preserving variant of this method yet. For a document
    /// whose `@context` is already inline, plain `Store.load(text, "jsonld")` is
    /// unchanged and remains the cheaper path.
    ///
    /// Compiled in only with the opt-in `jsonld-contexts` bundle feature; on a bundle
    /// without it this method is absent.
    #[wasm_bindgen(js_name = loadJsonLdWithContexts)]
    pub fn load_jsonld_with_contexts(
        text: &str,
        contexts: js_sys::Array,
    ) -> Result<Store, JsError> {
        let map = contexts_from_pairs(&contexts)?;
        let graph = parse_jsonld_with_contexts(text, map).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }
}

#[cfg(test)]
mod tests {
    // These run on the NATIVE target: they exercise `parse_jsonld_with_contexts`, the pure
    // core the `#[wasm_bindgen]` export delegates to (the export itself cannot run off-wasm
    // — `JsError::new` and `js_sys::Array` are wasm-bindgen imports). Same convention as
    // the other src/*.rs tests in this crate.
    use super::*;

    const VC_CONTEXT_URL: &str = "https://www.w3.org/2018/credentials/v1";

    /// A minimal stand-in for the real VC v1 context: enough term definitions to give the
    /// document below a fully-resolved set of IRIs.
    const VC_CONTEXT: &str = r#"{
      "@context": {
        "id": "@id",
        "type": "@type",
        "VerifiableCredential": "https://www.w3.org/2018/credentials#VerifiableCredential",
        "credentialSubject": {
          "@id": "https://www.w3.org/2018/credentials#credentialSubject",
          "@type": "@id"
        }
      }
    }"#;

    /// The shape that motivated the bead: a credential that names its `@context` only by
    /// URL. This is the load-bearing assertion — with the context supplied it parses and
    /// the terms resolve to the context's IRIs.
    #[test]
    fn remote_context_url_resolves_from_the_supplied_map() {
        let vc = r#"{
          "@context": "https://www.w3.org/2018/credentials/v1",
          "id": "http://example.org/creds/1",
          "type": "VerifiableCredential",
          "credentialSubject": {"id": "http://example.org/subject/alice"}
        }"#;
        let mut contexts = HashMap::new();
        contexts.insert(VC_CONTEXT_URL.to_string(), VC_CONTEXT.to_string());

        let g = parse_jsonld_with_contexts(vc, contexts).unwrap();
        // rdf:type + credentialSubject.
        assert_eq!(g.len(), 2, "both statements of the credential are interned");

        let nt =
            sparq_engine::construct_ntriples(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
                .unwrap();
        assert!(
            nt.contains("https://www.w3.org/2018/credentials#VerifiableCredential"),
            "the `type` term resolved through the remote context: {}",
            nt
        );
        assert!(
            nt.contains("https://www.w3.org/2018/credentials#credentialSubject"),
            "the `credentialSubject` term resolved through the remote context: {}",
            nt
        );
        assert!(
            nt.contains("http://example.org/subject/alice"),
            "the subject's `id` was coerced to an IRI node: {}",
            nt
        );
    }

    /// Fail-closed: the map is the allowlist. An unsupplied URL must be an `Err` naming
    /// that URL — never a silent partial parse (which would drop every term the context
    /// defines and hand back a plausible-looking but wrong graph).
    #[test]
    fn unsupplied_context_url_is_err_and_names_the_url() {
        let vc = r#"{
          "@context": "https://www.w3.org/2018/credentials/v1",
          "type": "VerifiableCredential"
        }"#;
        // `Graph` is not `Debug`, so unwrap the error arm by matching.
        let Err(err) = parse_jsonld_with_contexts(vc, HashMap::new()) else {
            panic!("an unsupplied remote @context must fail closed, not parse");
        };
        assert!(
            err.contains(VC_CONTEXT_URL),
            "the error names the context that is missing: {}",
            err
        );
    }

    /// A second, unrelated URL in the map does not make an unsupplied one resolve — the
    /// lookup is exact-match, not "any context will do".
    #[test]
    fn a_different_supplied_context_does_not_satisfy_the_reference() {
        let vc = r#"{"@context": "https://example.org/a.jsonld", "type": "Thing"}"#;
        let mut contexts = HashMap::new();
        contexts.insert("https://example.org/b.jsonld".to_string(), VC_CONTEXT.to_string());
        let Err(err) = parse_jsonld_with_contexts(vc, contexts) else {
            panic!("an exact-match miss must fail closed");
        };
        assert!(err.contains("https://example.org/a.jsonld"), "{}", err);
    }

    /// An inline `@context` still parses with an EMPTY map — supplying contexts is only
    /// needed for the by-URL form, so this method is a superset of `load(_, "jsonld")`.
    #[test]
    fn inline_context_needs_no_supplied_documents() {
        let doc = r#"{
          "@context": {"name": "http://schema.org/name"},
          "@id": "http://example.org/alice",
          "name": "Alice"
        }"#;
        let g = parse_jsonld_with_contexts(doc, HashMap::new()).unwrap();
        assert_eq!(g.len(), 1);
    }

    /// A nested/second-level remote context (a supplied context that itself references
    /// another by URL) resolves through the SAME callback — the recursion is oxjsonld's,
    /// which is precisely why this routes through its `LoadDocumentCallback` rather than
    /// rewriting the document's `@context` before parsing.
    #[test]
    fn nested_remote_context_resolves_through_the_same_callback() {
        let outer = r#"{"@context": ["https://example.org/inner.jsonld", {"nick": "http://schema.org/alternateName"}]}"#;
        let doc = r#"{
          "@context": "https://example.org/outer.jsonld",
          "@id": "http://example.org/alice",
          "name": "Alice",
          "nick": "Al"
        }"#;
        let mut contexts = HashMap::new();
        contexts.insert("https://example.org/outer.jsonld".to_string(), outer.to_string());
        contexts.insert(
            "https://example.org/inner.jsonld".to_string(),
            r#"{"@context": {"name": "http://schema.org/name"}}"#.to_string(),
        );
        let g = parse_jsonld_with_contexts(doc, contexts).unwrap();
        assert_eq!(g.len(), 2, "both the inner- and outer-defined terms resolved");
    }

    /// Malformed JSON is still an `Err` (the loader changes context resolution only, not
    /// the document's own syntax handling).
    #[test]
    fn malformed_document_is_err() {
        assert!(parse_jsonld_with_contexts("{ not json", HashMap::new()).is_err());
    }
}

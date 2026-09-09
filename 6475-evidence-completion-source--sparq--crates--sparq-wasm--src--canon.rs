//! [OPUS-4.8] sq-1dd5t (#1047): the opt-in `canonicalizeNQuads(nquads)` wasm
//! binding — RDFC-1.0 (RDF Dataset Canonicalization, the URDNA2015 successor)
//! surfaced to JS so the `@sparq-org/sparq` RDF/JS `Dataset` can implement
//! `toCanonical` / `equals` / `contains` by RDF-dataset **isomorphism** (blank
//! nodes relabelled to a canonical form) rather than by blank-node LABEL.
//!
//! It is a thin wrapper over [`sparq_canon::canonicalize_nquads`] (the single-
//! sourced RDFC-1.0 over the maintained `rdf-canon` crate, W3C-suite validated),
//! so the canonical output is byte-identical to what every other sparq consumer
//! (the ZK commitment pipeline, the native `sparq-canon` API) produces.
//!
//! Behind the non-default `canon` Cargo feature: the lean default bundle links
//! zero canonicalization code (and never pulls `rdf-canon` / its oxrdf-0.2
//! bridge), so the `wasm_bundle_bytes` baseline is byte-identical. The published
//! `@sparq-org/sparq` bundle (js `build:wasm`) turns it on.

use wasm_bindgen::prelude::*;

/// Canonicalizes an **N-Quads document** and returns its RDFC-1.0 canonical
/// N-Quads (canonically sorted, one quad per line, blank nodes relabelled to
/// `_:c14nN`, each line `\n`-terminated). Two N-Quads documents that denote
/// RDF-isomorphic datasets — i.e. differ only in blank-node labels and/or quad
/// order — produce byte-identical output, so a caller can hash / compare the
/// result for an isomorphism-aware dataset `equals` / `contains` / content hash.
///
/// `input` is parsed as N-Quads (the default graph is a 3-term line; named
/// graphs carry their graph term). A malformed document, or one containing an
/// RDF-1.2 triple term (outside the W3C RDFC-1.0 data model), returns the `Err`
/// (`JsError`) arm rather than trapping.
#[wasm_bindgen(js_name = canonicalizeNQuads)]
pub fn canonicalize_nquads(input: &str) -> Result<String, JsError> {
    sparq_canon::canonicalize_nquads(input).map_err(|e| JsError::new(&e.to_string()))
}

#[cfg(test)]
mod tests {
    // These run on the NATIVE target (the `#[wasm_bindgen]` export's `JsError`
    // mapping closure cannot run off-wasm — `JsError::new` is a wasm-bindgen
    // import that panics natively — so, like the other src/lib.rs tests, they
    // assert against the `sparq_canon` function the export delegates to; the real
    // wasm32 export is exercised in tests/web.rs::canonicalize_nquads_isomorphic).

    #[test]
    fn isomorphic_nquads_canonicalize_identically() {
        let a = "_:b0 <http://ex/p> _:b1 .\n_:b1 <http://ex/q> \"v\" .\n";
        let b = "_:y <http://ex/q> \"v\" .\n_:x <http://ex/p> _:y .\n";
        let ca = sparq_canon::canonicalize_nquads(a).unwrap();
        let cb = sparq_canon::canonicalize_nquads(b).unwrap();
        assert_eq!(ca, cb, "isomorphic datasets must canonicalize identically");
        assert!(ca.contains("_:c14n"));
    }

    #[test]
    fn non_isomorphic_differs() {
        let a = "_:x <http://ex/p> _:y .\n";
        let b = "_:x <http://ex/p> _:y .\n_:x <http://ex/r> \"w\" .\n";
        assert_ne!(
            sparq_canon::canonicalize_nquads(a).unwrap(),
            sparq_canon::canonicalize_nquads(b).unwrap()
        );
    }

    #[test]
    fn malformed_is_err() {
        assert!(sparq_canon::canonicalize_nquads("_:b0 <http://ex/p>").is_err());
    }
}

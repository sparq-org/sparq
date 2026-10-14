//! sparq-text-wasm: the sparq **full-text search** layer compiled to WebAssembly —
//! the tier-b "W-text" bundle (sq-jbe6).
//!
//! This is a SEPARATE, lazy-loaded bundle from the lean `sparq-wasm` triplestore bundle.
//! It exposes the owned BM25 inverted index + the `text:` magic predicates of
//! [`sparq_text`] over an RDF document, for in-tab live full-text search on the showcase
//! site's `/surface/full-text` page (the "quick brown fox" example: keyword →
//! BM25-ranked literals; `text:matches` / `text:score` SELECT).
//!
//! Every method is a stateless one-shot — it parses the supplied document, builds the
//! [`TextIndex`] over it, runs the request, and returns a string —
//! so the JS side never holds a long-lived index handle. The bundle ALWAYS builds the
//! index with positions ([`TextIndex::build_with_positions`]) so the positional
//! predicates (`text:phrase` / `text:near`) work alongside the BM25 ones; a tiny demo
//! corpus keeps the extra positional map (the index-build memory risk the bead flags)
//! bounded. The document syntaxes are those [`sparq_core::Graph::parse_to_triples`]
//! accepts (`"turtle"` | `"ntriples"` | `"nquads"` | `"trig"`; named graphs are folded
//! into the default graph, so the index is over every string literal in the document).
//!
//! Like the lean bundle it carries no serde: SELECT bindings are serialised through
//! sparq-engine's canonical SPARQL-1.1-JSON serialiser
//! ([`to_sparql_json`](sparq_engine::json::to_sparql_json)), and the index summary via a
//! hand-built JSON object (the same approach as `sparq-reason-wasm`).
//!
//! ```js
//! import init, { TextSearch } from "./sparq_text_wasm.js";
//! await init();
//! const data = `
//!   <http://ex/a> <http://ex/comment> "The quick brown fox" .
//!   <http://ex/b> <http://ex/comment> "A lazy dog" .`;
//! // text:matches (AND of tokens) + text:score, ranked, as SPARQL-results JSON:
//! const json = TextSearch.query(data, "ntriples",
//!   `PREFIX text: <http://sparq.dev/text#>
//!    SELECT ?s ?score WHERE {
//!      ?s <http://ex/comment> ?lit .
//!      ?lit text:matches "quick fox" ; text:score ?score .
//!    } ORDER BY DESC(?score)`);
//! // Index footprint (the bundle's memory risk), as JSON:
//! const stats = JSON.parse(TextSearch.indexStats(data, "ntriples"));
//! ```
#![forbid(unsafe_code)] // sq-jbe6: crate has zero `unsafe`

use sparq_core::Graph;
use sparq_text::{query_text, TextIndex};
use wasm_bindgen::prelude::*;

/// The minimal in-tab full-text-search entry point: parse an RDF document, build the
/// BM25 inverted index over its string literals, and run a `text:`-predicate SPARQL
/// query (or report the index footprint).
///
/// Stateless one-shots: nothing here holds a long-lived [`TextIndex`]; each call rebuilds
/// the (positions-enabled) index from the document it is given.
#[wasm_bindgen]
pub struct TextSearch;

#[wasm_bindgen]
impl TextSearch {
    /// Parses the RDF `data` document (`format` is `"turtle"` | `"ntriples"` |
    /// `"nquads"` | `"trig"`), builds a positions-enabled BM25 [`TextIndex`] over its
    /// string literals, rewrites the `text:` magic predicates in `sparql` into plain
    /// SPARQL ([`sparq_text::rewrite_query`]), and evaluates it through the standard
    /// engine — returning a SPARQL 1.1 JSON results document
    /// (`application/sparql-results+json`).
    ///
    /// The `text:` vocabulary ([`sparq_text::vocab`], `http://sparq.dev/text#`):
    /// `?lit text:matches "q"` (AND of tokens, `*`-suffix = prefix),
    /// `?lit text:matchesAny "q"` (OR), `?lit text:phrase "foo bar"` (adjacent, in
    /// order), `?lit text:near "foo bar"` (+ optional `text:slop N`, proximity-ranked),
    /// and `?lit text:score ?s` (the relevance score). A query with NO `text:` patterns
    /// runs unchanged. Errors (as a `JsError`) if the document or query fails to parse,
    /// or the query misuses a `text:` predicate (e.g. a non-variable subject).
    pub fn query(data: &str, format: &str, sparql: &str) -> Result<String, JsError> {
        let (dict, triples) =
            Graph::parse_to_triples(data, format).map_err(|e| JsError::new(&e))?;
        let graph = Graph::from_parts(dict, triples);
        // Positions ON so text:phrase / text:near are answerable too; the BM25 path
        // (text:matches / matchesAny / score) is identical either way.
        let index = TextIndex::build_with_positions(&graph);
        let result = query_text(&graph, sparql, &index).map_err(|e| JsError::new(&e))?;
        Ok(sparq_engine::json::to_sparql_json(&result))
    }

    /// Parses the RDF `data` document and reports the footprint of the BM25 index built
    /// over it as a small JSON object
    /// `{"docs":N,"tokens":M,"heapBytes":B,"hasPositions":true}` — the
    /// indexed-literal count, the distinct-token count, the index's estimated in-memory
    /// size in bytes, and whether positions are recorded. This is the bundle's
    /// index-build-memory surface (the risk the bead flags): the page can show it before
    /// (or instead of) running a query. Errors if the document fails to parse.
    #[wasm_bindgen(js_name = indexStats)]
    pub fn index_stats(data: &str, format: &str) -> Result<String, JsError> {
        let (dict, triples) =
            Graph::parse_to_triples(data, format).map_err(|e| JsError::new(&e))?;
        let graph = Graph::from_parts(dict, triples);
        let index = TextIndex::build_with_positions(&graph);
        Ok(index_stats_json(&index))
    }
}

/// Hand-built JSON summary of a [`TextIndex`]'s footprint (the bundle carries no serde,
/// exactly like `sparq-reason-wasm`). Every field is a number or a bare boolean, so no
/// string escaping is needed.
fn index_stats_json(index: &TextIndex) -> String {
    format!(
        "{{\"docs\":{},\"tokens\":{},\"heapBytes\":{},\"hasPositions\":{}}}",
        index.len(),
        index.token_count(),
        index.heap_bytes(),
        index.has_positions(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The quick-brown-fox example the /surface/full-text page demonstrates: two literals,
    // one of which contains both "quick" and "fox".
    const FOX_NT: &str = r#"<http://ex/a> <http://ex/comment> "The quick brown fox" .
        <http://ex/b> <http://ex/comment> "A lazy dog" ."#;

    // Native unit tests exercise the engine + index calls each #[wasm_bindgen] method
    // delegates to. The `JsError`-returning wasm wrappers themselves cannot run natively
    // (`JsError::new` is a wasm-bindgen import that panics off-wasm), so — exactly as the
    // reason bundle's tests do — we assert against the underlying calls. The headless
    // `wasm-pack test --node` suite (tests/web.rs) drives the real #[wasm_bindgen] API.

    /// `text:matches "quick fox"` (AND of tokens) selects only the literal containing
    /// BOTH tokens, and the BM25 score binds — exercising the bundle's core query path.
    #[test]
    fn matches_and_score_selects_the_fox() {
        let graph = Graph::load_str(FOX_NT, "ntriples").unwrap();
        let index = TextIndex::build_with_positions(&graph);
        let r = query_text(
            &graph,
            "PREFIX text: <http://sparq.dev/text#> \
             SELECT ?s ?score WHERE { ?s <http://ex/comment> ?lit . \
             ?lit text:matches \"quick fox\" ; text:score ?score . } ORDER BY DESC(?score)",
            &index,
        )
        .unwrap();
        // Only <http://ex/a> contains both "quick" and "fox".
        assert_eq!(r.rows.len(), 1, "exactly one literal matches both tokens");
        let json = sparq_engine::json::to_sparql_json(&r);
        assert!(
            json.contains("http://ex/a"),
            "the fox subject is bound: {json}"
        );
        assert!(
            !json.contains("http://ex/b"),
            "the lazy-dog subject is excluded: {json}"
        );
        // text:score binds a numeric value (SPARQL-JSON encodes it as an xsd:double).
        assert!(
            json.contains("\"score\""),
            "the score variable is projected: {json}"
        );
    }

    /// `text:phrase` is answerable because the bundle always builds a positions-enabled
    /// index: "quick brown" is an adjacent in-order match in the fox literal, "brown
    /// quick" is not.
    #[test]
    fn phrase_needs_positions_which_the_bundle_provides() {
        let graph = Graph::load_str(FOX_NT, "ntriples").unwrap();
        let index = TextIndex::build_with_positions(&graph);
        assert!(
            index.has_positions(),
            "the bundle's index records positions"
        );
        let hit = query_text(
            &graph,
            "PREFIX text: <http://sparq.dev/text#> \
             SELECT ?s WHERE { ?s <http://ex/comment> ?lit . ?lit text:phrase \"quick brown\" . }",
            &index,
        )
        .unwrap();
        assert_eq!(hit.rows.len(), 1, "adjacent in-order phrase matches once");
        let miss = query_text(
            &graph,
            "PREFIX text: <http://sparq.dev/text#> \
             SELECT ?s WHERE { ?s <http://ex/comment> ?lit . ?lit text:phrase \"brown quick\" . }",
            &index,
        )
        .unwrap();
        assert!(miss.rows.is_empty(), "out-of-order phrase does not match");
    }

    /// The index-stats JSON is well-formed and its counts reflect the corpus: two string
    /// literals (two docs), positions on, a positive heap-byte estimate.
    #[test]
    fn index_stats_reports_the_corpus() {
        let graph = Graph::load_str(FOX_NT, "ntriples").unwrap();
        let index = TextIndex::build_with_positions(&graph);
        let json = index_stats_json(&index);
        assert!(json.contains("\"docs\":2"), "two indexed literals: {json}");
        assert!(
            json.contains("\"hasPositions\":true"),
            "positions recorded: {json}"
        );
        // distinct tokens of "the quick brown fox" + "a lazy dog" = 7 (lowercased).
        assert!(
            json.contains("\"tokens\":7"),
            "seven distinct tokens: {json}"
        );
        assert!(
            !json.contains("\"heapBytes\":0"),
            "non-zero heap estimate: {json}"
        );
        assert_eq!(index.len(), 2);
    }

    /// A query with NO `text:` predicates passes through the rewrite unchanged and runs
    /// as ordinary SPARQL — the bundle is a superset of plain querying.
    #[test]
    fn plain_query_passes_through() {
        let graph = Graph::load_str(FOX_NT, "ntriples").unwrap();
        let index = TextIndex::build_with_positions(&graph);
        let r = query_text(
            &graph,
            "SELECT ?s WHERE { ?s <http://ex/comment> ?o . }",
            &index,
        )
        .unwrap();
        assert_eq!(
            r.rows.len(),
            2,
            "both subjects come back without any text: filter"
        );
    }
}

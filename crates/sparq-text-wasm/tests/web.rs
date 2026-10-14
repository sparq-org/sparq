// sq-jbe6 — headless wasm smoke test of the JS-facing sparq-text-wasm API.
//
// The crate's `#[cfg(test)] mod tests` in src/ runs on the NATIVE target only, so it
// exercises the engine + index functions the wrappers delegate to — NOT the actual
// `#[wasm_bindgen]` exports (`JsError::new` is a wasm-bindgen import that panics off-wasm).
// This module closes that gap: every test drives the REAL exported `TextSearch` API
// compiled to and executed in a genuine wasm32 runtime via `wasm-pack test --node`, so the
// index build + `text:` rewrite + SPARQL-JSON serialisation across the JS boundary is
// asserted end to end — exactly the surface a `cargo build --target wasm32` never runs.
//
// Tiny, inline datasets (the index-build memory risk the bead flags is bounded by a
// small demo corpus). `wasm-pack test --node` runs each `#[wasm_bindgen_test]` in the Node
// executor (no browser, no DOM); Node is the default target, so no
// `wasm_bindgen_test_configure!(run_in_browser)` directive is needed.

#![cfg(target_arch = "wasm32")]

use sparq_text_wasm::TextSearch;
use wasm_bindgen_test::*;

// The quick-brown-fox example the /surface/full-text page demonstrates.
const FOX_NT: &str = r#"<http://ex/a> <http://ex/comment> "The quick brown fox" .
    <http://ex/b> <http://ex/comment> "A lazy dog" ."#;

/// `text:matches` (AND of tokens) + `text:score` returns SPARQL-JSON binding only the
/// literal that contains both tokens — the core BM25 query path, end to end in wasm.
#[wasm_bindgen_test]
fn matches_and_score_in_wasm() {
    let json = TextSearch::query(
        FOX_NT,
        "ntriples",
        "PREFIX text: <http://sparq.dev/text#> \
         SELECT ?s ?score WHERE { ?s <http://ex/comment> ?lit . \
         ?lit text:matches \"quick fox\" ; text:score ?score . } ORDER BY DESC(?score)",
    )
    .expect("text:matches query must succeed in wasm");
    assert!(
        json.contains("http://ex/a"),
        "the fox subject is bound: {json}"
    );
    assert!(
        !json.contains("http://ex/b"),
        "the lazy-dog subject is excluded: {json}"
    );
    assert!(
        json.contains("\"score\""),
        "the score variable is projected: {json}"
    );
}

/// `text:phrase` works because the bundle always builds a positions-enabled index: an
/// in-order adjacent phrase matches, an out-of-order one does not.
#[wasm_bindgen_test]
fn phrase_in_wasm() {
    let hit = TextSearch::query(
        FOX_NT,
        "ntriples",
        "PREFIX text: <http://sparq.dev/text#> \
         SELECT ?s WHERE { ?s <http://ex/comment> ?lit . ?lit text:phrase \"quick brown\" . }",
    )
    .expect("text:phrase query must succeed in wasm");
    assert!(
        hit.contains("http://ex/a"),
        "adjacent in-order phrase matches: {hit}"
    );

    let miss = TextSearch::query(
        FOX_NT,
        "ntriples",
        "PREFIX text: <http://sparq.dev/text#> \
         SELECT ?s WHERE { ?s <http://ex/comment> ?lit . ?lit text:phrase \"brown quick\" . }",
    )
    .expect("out-of-order text:phrase query must still succeed (just empty)");
    assert!(
        !miss.contains("http://ex/a"),
        "out-of-order phrase does not match: {miss}"
    );
}

/// `indexStats` returns a well-formed JSON footprint reflecting the corpus.
#[wasm_bindgen_test]
fn index_stats_in_wasm() {
    let json = TextSearch::index_stats(FOX_NT, "ntriples").expect("indexStats must succeed");
    assert!(json.contains("\"docs\":2"), "two indexed literals: {json}");
    assert!(
        json.contains("\"hasPositions\":true"),
        "positions recorded: {json}"
    );
    assert!(
        json.contains("\"tokens\":7"),
        "seven distinct tokens: {json}"
    );
}

/// A malformed query surfaces as a clean `JsError` (a rejected Promise on the JS side),
/// not a wasm panic/trap.
#[wasm_bindgen_test]
fn bad_query_errors_cleanly() {
    assert!(TextSearch::query(FOX_NT, "ntriples", "SELECT ?s WHERE { not sparql").is_err());
}

/// A `text:` misuse (a non-variable subject) is also a clean error.
#[wasm_bindgen_test]
fn text_misuse_errors_cleanly() {
    assert!(TextSearch::query(
        FOX_NT,
        "ntriples",
        "PREFIX text: <http://sparq.dev/text#> \
         SELECT ?s WHERE { <http://ex/x> text:matches \"fox\" . }",
    )
    .is_err());
}

//! sq-3ul2n.3: the opt-in `Store::loadBytes` / `Store::loadBytesWithBase`
//! byte-ingest bindings.
//!
//! Every ingest today crosses the wasm boundary as a JS **string**: a document
//! fetched with `fetch(url).text()` is materialised as a UTF-16 JS string, which
//! `wasm-bindgen` then re-encodes to UTF-8 and copies into linear memory — two
//! character-set conversions plus an extra full-document JS-heap copy before the
//! parser sees a single byte. But the source is almost always **already bytes**: a
//! `Uint8Array` from `response.arrayBuffer()` or `File.arrayBuffer()`. These bindings
//! take that `Uint8Array` directly (`&[u8]` on the Rust side — `wasm-bindgen` copies it
//! ONCE into linear memory with no UTF-16 detour), validate it as UTF-8 **fail-closed**
//! (the RDF text formats sparq parses are all UTF-8), and feed the EXISTING parse path
//! ([`sparq_core::Graph::load_str`] / `load_str_with_base`) — so the resulting store is
//! byte-for-byte the same store [`Store::load`] / [`Store::load_with_base`] would build
//! from `new TextDecoder().decode(bytes)`, only without the JS-string round-trip.
//!
//! This module reimplements nothing: on a valid-UTF-8 input it is exactly
//! `Store::load(std::str::from_utf8(data)?, format)`. The only added surface is the
//! fail-closed UTF-8 check, which returns the SAME `JsError` result shape as `load`
//! (a `try { … } catch` on the JS side, never a panic / abort) so invalid bytes surface
//! as a catchable error rather than mis-parsing or trapping.
//!
//! It compiles ONLY under the opt-in `bytes-ingest` feature, OFF by default, so the lean
//! default browser bundle (`wasm_bundle_bytes`) carries zero byte-ingest code and stays
//! byte-identical; the published bundle (`js` `build:wasm`) turns it on. This module adds
//! two `#[wasm_bindgen] impl Store` associated functions to the `Store` defined in
//! `lib.rs`. NOTE: this ships the binding only — no speed claim is made here; the
//! wrapper-layer measurement lives in the sibling `js` bead (sq-3ul2n.5).

use sparq_core::Graph;
use wasm_bindgen::prelude::*;

use crate::Store;

/// Validate the raw bytes as UTF-8 **fail-closed**, returning the same `JsError` surface
/// as [`Store::load`] on failure.
///
/// The RDF text formats sparq ingests (Turtle / N-Triples / N-Quads / TriG / JSON-LD) are
/// all UTF-8, so this is the correct decode: invalid bytes are rejected up front with a
/// clear, catchable `JsError` (the byte offset of the first invalid sequence), never a
/// silently-truncated or lossily-replaced document. On success it borrows the bytes as a
/// `&str` with no copy (the bytes already live in linear memory), which is then handed
/// straight to the parser.
fn decode_utf8(data: &[u8]) -> Result<&str, JsError> {
    std::str::from_utf8(data).map_err(|e| {
        JsError::new(&format!(
            "input is not valid UTF-8 (RDF text formats must be UTF-8): {}",
            e
        ))
    })
}

#[wasm_bindgen]
impl Store {
    /// sq-3ul2n.3: like [`load`](Self::load) but ingests **raw bytes** (a JS
    /// `Uint8Array`) instead of a JS string, skipping the UTF-16 string round-trip.
    ///
    /// Pass the `Uint8Array` you already hold — e.g. `new Uint8Array(await
    /// response.arrayBuffer())` or `new Uint8Array(await file.arrayBuffer())`. The bytes
    /// are copied ONCE into wasm linear memory (no intermediate UTF-16 JS string), then
    /// validated as UTF-8 and parsed through the exact same path as [`load`](Self::load),
    /// so the resulting store is identical to `Store.load(new TextDecoder().decode(bytes),
    /// format)`. `format` is the same set [`load`](Self::load) accepts (`"turtle"` |
    /// `"ntriples"` | `"nquads"` | `"trig"` | `"jsonld"` with the opt-in `jsonld` feature).
    /// Named graphs are folded into the default graph (as [`load`](Self::load)).
    ///
    /// Invalid UTF-8 is rejected **fail-closed** with a `JsError` (a `try { … } catch` on
    /// the JS side), the same error surface as a malformed document to [`load`](Self::load)
    /// — never a panic, abort, or lossy decode.
    #[wasm_bindgen(js_name = loadBytes)]
    pub fn load_bytes(data: &[u8], format: &str) -> Result<Store, JsError> {
        let text = decode_utf8(data)?;
        let graph = Graph::load_str(text, format).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// sq-3ul2n.3: the byte-ingest counterpart to
    /// [`loadWithBase`](Self::load_with_base) — ingests raw bytes (a `Uint8Array`) and
    /// resolves the document's RELATIVE IRIs against `base`.
    ///
    /// Identical semantics to [`loadWithBase`](Self::load_with_base) (a document-level
    /// `@base` still overrides the supplied `base`; the line-based formats
    /// `"ntriples"` / `"nquads"` allow only absolute IRIs so `base` has no effect on them;
    /// an invalid `base` is rejected with a `JsError`), only reading bytes instead of a JS
    /// string. Invalid UTF-8 is rejected fail-closed exactly as in
    /// [`loadBytes`](Self::load_bytes).
    #[wasm_bindgen(js_name = loadBytesWithBase)]
    pub fn load_bytes_with_base(data: &[u8], format: &str, base: &str) -> Result<Store, JsError> {
        let text = decode_utf8(data)?;
        let graph =
            Graph::load_str_with_base(text, format, base).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `Ok` arm of `load_bytes` / `load_bytes_with_base` never touches `JsError::new`
    // (that import traps off-wasm), so — mirroring the native-test convention of the other
    // wasm bindings (see `serialize.rs`) — the equivalence invariant is pinned here without
    // a wasm runtime. The fail-closed (`Err`) arm hits `JsError::new`, so its end-to-end
    // assertion lives in the wasm32 `tests/web.rs` under the `bytes-ingest` feature.

    const DATA: &str = r#"@prefix ex: <http://ex/> .
        ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
        ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

    /// `load_bytes(utf8_of(s), fmt)` builds a store equal to `load(s, fmt)` — the
    /// load-bearing result-equivalence invariant (the binding adds no reparsing, only a
    /// UTF-8 decode of already-in-memory bytes).
    #[test]
    fn load_bytes_equivalent_to_load() {
        let via_str = Store::load(DATA, "turtle").unwrap();
        let via_bytes = Store::load_bytes(DATA.as_bytes(), "turtle").unwrap();
        assert_eq!(via_bytes.size(), via_str.size(), "equal triple count");
        // Probe query equality: the same SELECT over both stores yields byte-identical JSON.
        let q =
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n ; ex:age ?a } ORDER BY ?a";
        assert_eq!(via_bytes.query(q).unwrap(), via_str.query(q).unwrap());
    }

    /// A multi-byte UTF-8 fixture (accented + CJK literals) survives the byte path intact
    /// and is equal to the string path — the byte copy is not mangling non-ASCII.
    #[test]
    fn load_bytes_multibyte_utf8_equivalent() {
        let data = r#"@prefix ex: <http://ex/> .
            ex:a ex:name "Ámélie" .
            ex:b ex:name "日本語" .
            ex:c ex:name "emoji é and 🦀" ."#;
        let via_str = Store::load(data, "turtle").unwrap();
        let via_bytes = Store::load_bytes(data.as_bytes(), "turtle").unwrap();
        assert_eq!(via_bytes.size(), via_str.size());
        let q = "SELECT ?n WHERE { ?s <http://ex/name> ?n } ORDER BY ?n";
        assert_eq!(via_bytes.query(q).unwrap(), via_str.query(q).unwrap());
        // The CJK literal really crossed the byte boundary (not replaced / dropped).
        assert!(via_bytes.query(q).unwrap().contains("日本語"));
    }

    /// `load_bytes_with_base` resolves relative IRIs against the base, equal to the string
    /// `load_with_base`.
    #[test]
    fn load_bytes_with_base_equivalent() {
        let doc = "<a> <p> <o> .";
        let via_str = Store::load_with_base(doc, "turtle", "http://ex/dir/").unwrap();
        let via_bytes =
            Store::load_bytes_with_base(doc.as_bytes(), "turtle", "http://ex/dir/").unwrap();
        assert_eq!(via_bytes.size(), via_str.size());
        // `<p>` resolves to `http://ex/dir/p` under the base, so query on the resolved IRI.
        let q = "SELECT ?s ?o WHERE { ?s <http://ex/dir/p> ?o }";
        assert_eq!(via_bytes.query(q).unwrap(), via_str.query(q).unwrap());
        // The base actually resolved: the subject is the absolute IRI, not a relative one.
        assert!(via_bytes.query(q).unwrap().contains("http://ex/dir/a"));
    }

    /// `decode_utf8` accepts valid UTF-8 (the `Ok` arm, native-safe) and — verified by the
    /// zero-copy borrow — returns the exact same string. The invalid-UTF-8 `Err` arm hits
    /// `JsError::new` (which traps off-wasm), so its assertion is deferred to the wasm test
    /// (`tests/web.rs::load_bytes_invalid_utf8_fails_closed`); here we pin only the `Ok` arm.
    #[test]
    fn decode_utf8_accepts_valid() {
        assert_eq!(decode_utf8(b"hello \xC3\xA9").unwrap(), "hello \u{e9}");
        // Sanity on the underlying validator's rejection of a lone lead byte — built at
        // runtime so the `invalid_from_utf8` lint (which only fires on a literal) stays quiet.
        let bad: Vec<u8> = vec![0x80u8, 0x80];
        assert!(std::str::from_utf8(&bad).is_err());
    }
}

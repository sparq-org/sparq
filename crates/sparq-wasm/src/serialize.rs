//! sq-fe1s / sq-ixc3.5: the `Store::serialize(format, …)` RDF-writer binding.
//!
//! Exposes `sparq-engine`'s pretty Turtle / TriG writers
//! ([`sparq_engine::serialize`] — `graph_to_turtle_pretty_with` /
//! `graph_to_trig_pretty_with`, byte-shape-matching the site's `prettyTurtle`
//! reshaper, #805) AND its native JSON-LD 1.1 writers
//! (`graph_to_jsonld_with` / `graph_to_jsonld_pretty_with`, the
//! expanded / flattened / compacted forms, sq-ixc3.3) to JS/WASM consumers, so a
//! browser `Store` can produce a human-readable Turtle / TriG / JSON-LD document of
//! its contents — not just the flat N-Triples that [`Store::query_quads`] returns
//! for a CONSTRUCT/DESCRIBE.
//!
//! This module reimplements nothing: it calls straight through to the engine
//! serialiser the native CLI / HTTP surface uses, so a `Store.serialize("turtle",
//! true, …)` is byte-identical to `sparq_engine::serialize::graph_to_turtle_pretty_with`
//! over the same graph, and `Store.serialize("jsonld-expanded", true, …, None)` is
//! byte-identical to `graph_to_jsonld_pretty_with(.., JsonLdForm::Expanded, ..)`
//! (asserted by the native `src/serialize.rs` parity tests and the `wasm32` headless
//! `tests/web.rs` tests). Routing JSON-LD through this SAME method lets the site
//! consume the engine writer instead of hand-formatting JSON-LD in TypeScript —
//! mirroring the Turtle engine-first path.
//!
//! It compiles ONLY under the opt-in `serialize-rdf` feature (which forwards to
//! `sparq-engine/serialize-rdf`, where the whole writer matrix — Turtle/TriG/JSON-LD —
//! lives), so the default browser bundle carries zero serializer code — the lean
//! `wasm_bundle_bytes` baseline is unchanged. The JSON-LD *serialise-out* path needs
//! NO extra feature: the writers are pure Rust under `serialize-rdf` (the opt-in
//! `jsonld` feature is INGEST-only — it pulls the `oxjsonld` *parser*, unrelated to
//! these writers). The site REPL bundle (`js` `build:wasm`, built
//! `--features shacl,jsonld,serialize-rdf`) turns `serialize-rdf` on.
//!
//! ## Surface
//!
//! `serialize(format, pretty, indent, abbreviate, prefixes?)`:
//!
//! * `format` (case-insensitive) — one of:
//!   * `"turtle"` (aliases `"ttl"`, `"text/turtle"`) — the **default graph only**.
//!   * `"trig"` (alias `"application/trig"`) — the whole dataset (default graph at
//!     top level, named graphs as `GRAPH <g> { … }` blocks).
//!   * `"jsonld"` / `"json-ld"` / `"application/ld+json"` — JSON-LD 1.1, **expanded**
//!     form by default (the safest interop target). To pick a form explicitly use
//!     `"jsonld-expanded"`, `"jsonld-flattened"`, or `"jsonld-compacted"`
//!     (`json-ld-…` accepted too). JSON-LD always emits the whole dataset.
//! * `pretty` — `true` for the indented pretty shape (Turtle/TriG: the
//!   blank-line-separated, sorted `prettyTurtle` shape; JSON-LD: the structurally
//!   re-indented document), `false` for the compact single-pass / minified writer.
//! * `indent` — the indent unit for the pretty writers (e.g. `"  "`, `"\t"`).
//!   `undefined` / `null` uses the two-space default. Ignored when `pretty` is `false`.
//! * `abbreviate` — Turtle/TriG only: `true` to emit an alphabetical `@prefix` header
//!   and compact IRIs to `prefix:local`; `false` to keep every IRI in full `<…>` form.
//!   **Ignored for JSON-LD** — IRI abbreviation there is selected by the
//!   `jsonld-compacted` form (which carries a prefix `@context`), not by this flag.
//! * `prefixes?` — an OPTIONAL caller-supplied prefix map: a JS array of `[prefix, iri]`
//!   pairs (e.g. `[["ex", "http://example.org/"], ["schema", "https://schema.org/"]]`).
//!   When omitted (`undefined` / `null`) the engine's well-known [`default_prefixes`] are
//!   used — **byte-for-byte the prior behaviour**, so every existing caller is unchanged.
//!   When supplied, those prefixes drive Turtle/TriG `@prefix` compaction and the JSON-LD
//!   compacted `@context`, so a caller can match its own prefix policy (the site's
//!   `COMMON_PREFIXES`, a query's declared `PREFIX` lines) and get byte-parity output. A
//!   malformed entry (not a two-string array) is rejected with a `JsError`.
//!
//! An unknown `format` (or a malformed `prefixes`) is rejected with a clear `JsError`
//! (never a silent empty string), so a typo surfaces rather than producing a wrong
//! document.

use sparq_engine::serialize::{
    default_prefixes, graph_to_jsonld_compact, graph_to_jsonld_compact_pretty,
    graph_to_jsonld_pretty_with, graph_to_jsonld_with, graph_to_trig_pretty_with,
    graph_to_trig_with, graph_to_turtle_pretty_with, graph_to_turtle_with, parse_context_json,
    prefixes_from_pairs, JsonLdForm, JsonLdPrettyOptions, Prefixes, PrettyOptions,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::Store;

/// sq-l5kr: turns the optional JS `prefixes` argument into the engine
/// [`Prefixes`] map the writers consume.
///
/// * `None` (the JS caller passed `undefined` / `null` / omitted the argument) reproduces
///   the prior behaviour exactly — the engine's [`default_prefixes`] (`schema` →
///   `http://schema.org/`, the `rdf`/`rdfs`/`xsd`/`owl`/`foaf`/`dc`/`skos`/`sh` set), so
///   every existing caller and golden is byte-for-byte unchanged.
/// * `Some(array)` is read as an ordered `[[prefix, iri], …]` list — each element a
///   2-entry JS array of strings — and turned into the map via [`prefixes_from_pairs`]
///   (last write wins on a duplicate label). This lets a caller serialise under its OWN
///   prefix policy (e.g. the site's `COMMON_PREFIXES`: `schema` → `https://schema.org/`
///   plus `dcterms`/`prov`/`geo`/`void`/`ex` → `http://example.org/`, or a query's parsed
///   `PREFIX` declarations) and get byte-parity output.
///
/// A malformed element (not a 2-string array) is rejected with a clear `JsError` rather
/// than silently dropped, so a bad prefix map surfaces instead of producing a wrong
/// document. An empty array means "no prefixes" (every IRI stays full `<…>`).
fn resolve_prefixes(prefixes: Option<js_sys::Array>) -> Result<Prefixes, JsError> {
    let Some(arr) = prefixes else {
        return Ok(default_prefixes());
    };
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(arr.length() as usize);
    for (i, entry) in arr.iter().enumerate() {
        let pair: js_sys::Array = entry.dyn_into().map_err(|_| {
            JsError::new(&format!(
                "prefixes[{}] must be a [prefix, iri] array of two strings",
                i
            ))
        })?;
        let prefix = pair.get(0).as_string().ok_or_else(|| {
            JsError::new(&format!("prefixes[{}][0] (the prefix label) must be a string", i))
        })?;
        let iri = pair.get(1).as_string().ok_or_else(|| {
            JsError::new(&format!("prefixes[{}][1] (the namespace IRI) must be a string", i))
        })?;
        pairs.push((prefix, iri));
    }
    Ok(prefixes_from_pairs(pairs))
}

/// The output format the wasm `Store::serialize` understands. Kept private — the JS
/// surface takes a `&str` (the same convention as `Store::load`'s `format`).
enum SerFormat {
    Turtle,
    Trig,
    /// JSON-LD 1.1 in the selected document form (expanded / flattened / compacted).
    JsonLd(JsonLdForm),
}

impl SerFormat {
    /// Parses the JS-supplied `format` string (case-insensitive). `None` for an
    /// unrecognised value, which the caller maps to a `JsError`. A bare `"jsonld"`
    /// (or its media-type / `json-ld` aliases) selects the **expanded** form — the
    /// safest interop default; the form is otherwise chosen by the `-expanded` /
    /// `-flattened` / `-compacted` suffix.
    fn parse(format: &str) -> Option<SerFormat> {
        match format.trim().to_ascii_lowercase().as_str() {
            "turtle" | "ttl" | "text/turtle" => Some(SerFormat::Turtle),
            "trig" | "application/trig" => Some(SerFormat::Trig),
            "jsonld" | "json-ld" | "application/ld+json" => {
                Some(SerFormat::JsonLd(JsonLdForm::Expanded))
            }
            "jsonld-expanded" | "json-ld-expanded" => Some(SerFormat::JsonLd(JsonLdForm::Expanded)),
            "jsonld-flattened" | "json-ld-flattened" => {
                Some(SerFormat::JsonLd(JsonLdForm::Flattened))
            }
            "jsonld-compacted" | "json-ld-compacted" => {
                Some(SerFormat::JsonLd(JsonLdForm::Compacted))
            }
            _ => None,
        }
    }
}

#[wasm_bindgen]
impl Store {
    /// sq-fe1s / sq-ixc3.5: serialises the store's contents to a **Turtle**,
    /// **TriG**, or **JSON-LD** document string.
    ///
    /// `format` (case-insensitive) is one of:
    /// * `"turtle"` (aliases `"ttl"`, `"text/turtle"`) — the default graph only.
    /// * `"trig"` (alias `"application/trig"`) — the whole dataset: default graph at
    ///   top level, named graphs as `GRAPH <g> { … }` blocks.
    /// * `"jsonld"` / `"json-ld"` / `"application/ld+json"` — JSON-LD 1.1, **expanded**
    ///   form by default; `"jsonld-expanded"` / `"jsonld-flattened"` /
    ///   `"jsonld-compacted"` pick the form explicitly (`json-ld-…` accepted too).
    ///   JSON-LD always emits the whole dataset.
    ///
    /// When `pretty` is `true` the output is indented: Turtle/TriG use the
    /// blank-line-separated, **sorted** (emission-order-independent) `prettyTurtle`
    /// shape; JSON-LD uses the structurally re-indented document. The `indent` arg is
    /// the indent unit (`undefined`/`null` ⇒ two spaces). When `pretty` is `false` the
    /// compact / minified writer is used and `indent` is ignored.
    ///
    /// `abbreviate` applies to **Turtle/TriG only**: `true` emits a sorted `@prefix`
    /// header and compacts IRIs to `prefix:local`; `false` keeps every IRI in full
    /// `<…>` form. It is **ignored for JSON-LD** — IRI abbreviation there is selected by
    /// the `jsonld-compacted` form (which carries a prefix `@context`), not this flag.
    ///
    /// `prefixes` is an OPTIONAL caller-supplied prefix map: a JS array of
    /// `[prefix, iri]` pairs (e.g. `[["ex", "http://example.org/"], ["schema",
    /// "https://schema.org/"]]`). When omitted (`undefined` / `null`) the engine's
    /// well-known defaults (`rdf`, `rdfs`, `xsd`, `owl`, `schema` → `http://schema.org/`,
    /// `foaf`, `dc`, `skos`, `sh`) are used — **byte-for-byte the prior behaviour**. When
    /// supplied, those prefixes drive Turtle/TriG `@prefix` compaction and the JSON-LD
    /// compacted `@context` instead, so a caller can match its OWN prefix policy (the
    /// site's `COMMON_PREFIXES` with `https://schema.org/` + `dcterms`/`prov`/`geo`/`void`/
    /// `ex`, or a query's declared `PREFIX` lines) and get byte-parity output. A malformed
    /// entry (not a two-string array) is rejected with a `JsError`. Only used for
    /// compaction (Turtle/TriG with `abbreviate=true`, JSON-LD `jsonld-compacted`).
    ///
    /// This is the document-export counterpart to [`query_quads`](Self::query_quads),
    /// which returns a CONSTRUCT/DESCRIBE *result graph* as flat N-Triples: `serialize`
    /// writes the **store itself** in a readable syntax. Errors only if `format` is
    /// not one of the recognised values, or `prefixes` is malformed (a `JsError`);
    /// serialisation itself is infallible. Available only when the crate is built with the
    /// OPT-IN `serialize-rdf` feature — the site REPL bundle enables it; the lean default
    /// bundle does not. (JSON-LD *serialise-out* needs no extra feature: the writers
    /// live under `serialize-rdf`; the `jsonld` feature is INGEST-only.)
    pub fn serialize(
        &self,
        format: &str,
        pretty: bool,
        indent: Option<String>,
        abbreviate: bool,
        prefixes: Option<js_sys::Array>,
    ) -> Result<String, JsError> {
        let fmt = SerFormat::parse(format).ok_or_else(|| {
            JsError::new(&format!(
                "unsupported serialize format {:?} (expected \"turtle\", \"trig\", or \
                 \"jsonld\" / \"jsonld-expanded\" / \"jsonld-flattened\" / \"jsonld-compacted\")",
                format
            ))
        })?;
        // sq-l5kr: the caller's prefix policy (or the engine default when absent).
        let prefixes = resolve_prefixes(prefixes)?;
        // JSON-LD selects its form via the `format` string (not `abbreviate`); the
        // `indent`/`pretty` args carry through identically to the Turtle/TriG path.
        if let SerFormat::JsonLd(form) = fmt {
            let out = if pretty {
                let opts = JsonLdPrettyOptions {
                    indent: indent.unwrap_or_else(|| "  ".to_string()),
                };
                graph_to_jsonld_pretty_with(&self.graph, form, &prefixes, &opts)
            } else {
                graph_to_jsonld_with(&self.graph, form, &prefixes)
            };
            return Ok(out);
        }
        let out = if pretty {
            let opts = PrettyOptions {
                indent: indent.unwrap_or_else(|| "  ".to_string()),
                abbreviate,
            };
            match fmt {
                SerFormat::Turtle => graph_to_turtle_pretty_with(&self.graph, &prefixes, &opts),
                SerFormat::Trig => graph_to_trig_pretty_with(&self.graph, &prefixes, &opts),
                SerFormat::JsonLd(_) => unreachable!("JSON-LD handled above"),
            }
        } else {
            match fmt {
                // The compact writers always compact against the supplied prefixes; the
                // `abbreviate=false` "full IRIs" choice is a PRETTY-only option, so an
                // empty prefix map is passed to suppress CURIE compaction in that case.
                SerFormat::Turtle if abbreviate => graph_to_turtle_with(&self.graph, &prefixes),
                SerFormat::Turtle => graph_to_turtle_with(&self.graph, &Prefixes::new()),
                // sq-l5kr: the compact TriG path now honours the caller's prefix
                // map via `graph_to_trig_with` (previously it always used default_prefixes()).
                // `abbreviate=false` suppresses compaction with an empty map, matching Turtle.
                SerFormat::Trig if abbreviate => graph_to_trig_with(&self.graph, &prefixes),
                SerFormat::Trig => graph_to_trig_with(&self.graph, &Prefixes::new()),
                SerFormat::JsonLd(_) => unreachable!("JSON-LD handled above"),
            }
        };
        Ok(out)
    }

    /// sq-oy1f.5: serialises the store as a **full W3C JSON-LD 1.1 Compaction**
    /// document against a caller-supplied `@context`.
    ///
    /// Where [`serialize`](Self::serialize)`("jsonld-compacted", …)` only abbreviates IRIs
    /// to `prefix:local` CURIEs from a `[prefix, iri]` map (a *prefix-only* `@context`), this
    /// applies the real **W3C JSON-LD 1.1 Compaction Algorithm** against the `@context` JSON
    /// you pass: **term definitions** (`{"name":"http://…/name"}` or the expanded
    /// `{"@id"/"@reverse","@type","@language","@container"}` form), **`@vocab`**, **type
    /// coercion** (a term `@type` matching a datatype collapses the value object;
    /// `@type":"@id"`/`@vocab` collapse a node reference to a bare IRI string), **language
    /// coercion**, **`@container`** (`@set`/`@list`/`@language`/`@index`), **`@reverse`**
    /// terms, and `@id`/`@type` keyword aliasing — value + node + IRI compaction against the
    /// active context. The whole dataset is emitted (named graphs as nested `@graph` nodes).
    ///
    /// `context` is the `@context` **JSON text** — e.g. `'{"@vocab":"http://schema.org/"}'`
    /// or `'{"name":"http://xmlns.com/foaf/0.1/name"}'`. It must be a JSON **object** (a
    /// JSON-LD `@context` value); an empty `{}` yields an expanded-shaped document with no
    /// abbreviation. A non-object or malformed JSON is rejected with a `JsError` (never a
    /// silently-wrong document).
    ///
    /// `pretty` selects the indented multi-line shape (whitespace-only re-indentation of the
    /// minified document); `indent` is the indent unit (`undefined`/`null` ⇒ two spaces,
    /// ignored when `pretty` is `false`).
    ///
    /// The compaction is **lossless** — every coercion it applies is invertible against the
    /// same `@context`, so a JSON-LD-to-RDF round-trip of the output reconstructs the original
    /// triples. Routes through the SAME engine writer
    /// (`sparq_engine::serialize::graph_to_jsonld_compact`) the native CLI surface uses, so
    /// the bytes match. Still **dependency-free** (a hand-rolled `Json` AST — no `serde_json`,
    /// no json-ld crate). Available only when the crate is built with the OPT-IN
    /// `serialize-rdf` feature (the JSON-LD *serialise-out* path needs no `jsonld` feature —
    /// that one is INGEST-only); on the lean default bundle this method is absent.
    #[wasm_bindgen(js_name = serializeCompact)]
    pub fn serialize_compact(
        &self,
        context: &str,
        pretty: bool,
        indent: Option<String>,
    ) -> Result<String, JsError> {
        // The caller `@context` must be a JSON object (a JSON-LD `@context` value). A
        // non-object / malformed string surfaces as an `Err` rather than a wrong document.
        let ctx = parse_context_json(context).ok_or_else(|| {
            JsError::new(
                "the `context` argument must be a JSON object (a JSON-LD @context); \
                 got malformed JSON or a non-object value",
            )
        })?;
        let out = if pretty {
            // The indented multi-line shape, matching `serialize("jsonld-…", true, …)`'s
            // JSON-LD indentation (whitespace-only re-indentation of the minified document).
            let opts = JsonLdPrettyOptions {
                indent: indent.unwrap_or_else(|| "  ".to_string()),
            };
            graph_to_jsonld_compact_pretty(&self.graph, &ctx, &opts)
        } else {
            graph_to_jsonld_compact(&self.graph, &ctx)
        };
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    const DATA: &str = r#"@prefix ex: <http://ex/> .
        ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
        ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

    // The serialiser runs natively (the `Ok` arm never touches `JsError::new`), so the
    // parity contract is tested here without a wasm runtime — mirroring the native-test
    // convention of the other wasm bindings. The negative (`Err`) arm (an unknown format
    // string) stays covered by the wasm32 `tests/web.rs::serialize_unknown_format_is_err`.

    /// Pretty Turtle through the wasm `Store::serialize` is byte-identical to the engine
    /// serialiser it delegates to (`graph_to_turtle_pretty_with`) over the same graph and
    /// options — the load-bearing parity invariant (the binding adds no reformatting).
    #[test]
    fn serialize_turtle_pretty_matches_engine() {
        let store = Store::load(DATA, "turtle").unwrap();
        let got = store
            .serialize("turtle", true, Some("  ".to_string()), true, None)
            .unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let want = graph_to_turtle_pretty_with(&g, &default_prefixes(), &PrettyOptions::default());
        assert_eq!(got, want, "wasm pretty Turtle must equal the engine output");
        // Sanity on the shape: with abbreviation on, the well-known `xsd:` prefix (used by
        // the integer ages) is declared in a header and the integer datatype is compacted.
        // (`ex:` is NOT a well-known prefix, so those IRIs stay in full `<…>` form — the
        // byte-equality above is the real parity check; this just pins the shape.)
        assert!(
            got.contains("@prefix xsd:"),
            "well-known prefix header present: {got}"
        );
        assert!(got.contains("xsd:integer"), "datatype compacted: {got}");
    }

    /// A custom indent flows through to the engine writer (the `{indent}` option is wired,
    /// not dropped): a four-space indent produces a different document than the default.
    #[test]
    fn serialize_turtle_pretty_custom_indent() {
        let store = Store::load(DATA, "turtle").unwrap();
        let two = store
            .serialize("turtle", true, Some("  ".to_string()), true, None)
            .unwrap();
        let four = store
            .serialize("turtle", true, Some("    ".to_string()), true, None)
            .unwrap();
        assert_ne!(two, four, "indent option must affect the output");
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let opts = PrettyOptions {
            indent: "    ".to_string(),
            abbreviate: true,
        };
        assert_eq!(
            four,
            graph_to_turtle_pretty_with(&g, &default_prefixes(), &opts),
            "custom-indent output must equal the engine output with the same opts"
        );
        // `indent` defaulting: passing `None` matches the two-space default explicitly.
        let dflt = store.serialize("turtle", true, None, true, None).unwrap();
        assert_eq!(dflt, two, "None indent uses the two-space default");
    }

    /// `abbreviate=false` keeps every IRI in full `<…>` form with no `@prefix` header.
    #[test]
    fn serialize_turtle_pretty_no_abbreviate() {
        let store = Store::load(DATA, "turtle").unwrap();
        let full = store.serialize("turtle", true, None, false, None).unwrap();
        assert!(
            !full.contains("@prefix"),
            "no prefix header when abbreviate=false: {full}"
        );
        assert!(
            full.contains("<http://ex/alice>"),
            "IRIs stay in full form: {full}"
        );
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let opts = PrettyOptions {
            indent: "  ".to_string(),
            abbreviate: false,
        };
        assert_eq!(
            full,
            graph_to_turtle_pretty_with(&g, &default_prefixes(), &opts)
        );
    }

    /// TriG pretty over a dataset emits a `GRAPH <g> { … }` block for the named graph and
    /// matches the engine `graph_to_trig_pretty_with`.
    #[test]
    fn serialize_trig_pretty_named_graph() {
        let nq = "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
        let store = Store::load_dataset(nq, "nquads").unwrap();
        let got = store.serialize("trig", true, None, true, None).unwrap();
        assert!(got.contains("GRAPH"), "named graph as a GRAPH block: {got}");
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let want = graph_to_trig_pretty_with(&g, &default_prefixes(), &PrettyOptions::default());
        assert_eq!(got, want, "wasm pretty TriG must equal the engine output");
    }

    /// The compact (non-pretty) Turtle path matches the engine compact writer.
    #[test]
    fn serialize_turtle_compact_matches_engine() {
        let store = Store::load(DATA, "turtle").unwrap();
        let got = store.serialize("turtle", false, None, true, None).unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let want = graph_to_turtle_with(&g, &default_prefixes());
        assert_eq!(got, want, "wasm compact Turtle must equal the engine output");
    }

    // ---- sq-l5kr: the optional caller-supplied prefix map ----
    //
    // The `prefixes` arg is a `js_sys::Array`, which can only be CONSTRUCTED in a real
    // wasm runtime (`js_sys::Array::new()` traps off-wasm). So the native tests here pin
    // the `None` (default) arm — the load-bearing **back-compat** invariant — across
    // every format; the `Some(prefix-map)` arm (the new capability: `http://example.org/x`
    // -> `ex:x` and `https://schema.org/`) is exercised across the JS boundary by
    // `tests/web.rs::serialize_with_prefix_map`.

    /// `prefixes = None` (the default arm) reproduces `default_prefixes()` BYTE-FOR-BYTE for
    /// every format — the back-compat guarantee that adding the optional argument changed no
    /// existing output. (Equivalently: every other native test above passes `None` and still
    /// matches the engine `*_with(.., &default_prefixes(), ..)` writers.)
    #[test]
    fn serialize_none_prefixes_reproduces_default_byte_for_byte() {
        let nq = "<http://ex/s> <http://ex/p> <http://schema.org/y> <http://ex/g1> .\n\
                  <http://ex/s> <http://schema.org/q> \"v\" .\n";
        let store = Store::load_dataset(nq, "nquads").unwrap();
        let g = Graph::load_dataset(nq, "nquads").unwrap();
        let dp = default_prefixes();
        // Pretty Turtle (default graph), pretty TriG (whole dataset), compact TriG, and
        // JSON-LD compacted — each `None`-arm output equals the engine default-prefix writer.
        assert_eq!(
            store.serialize("turtle", true, None, true, None).unwrap(),
            graph_to_turtle_pretty_with(&g, &dp, &PrettyOptions::default()),
            "pretty Turtle None arm == default_prefixes",
        );
        assert_eq!(
            store.serialize("trig", true, None, true, None).unwrap(),
            graph_to_trig_pretty_with(&g, &dp, &PrettyOptions::default()),
            "pretty TriG None arm == default_prefixes",
        );
        assert_eq!(
            store.serialize("trig", false, None, true, None).unwrap(),
            graph_to_trig_with(&g, &dp),
            "compact TriG None arm == default_prefixes (now honoured via graph_to_trig_with)",
        );
        assert_eq!(
            store.serialize("jsonld-compacted", false, None, true, None).unwrap(),
            graph_to_jsonld_with(&g, JsonLdForm::Compacted, &dp),
            "JSON-LD compacted None arm == default_prefixes",
        );
        // The default schema namespace is HTTP — so http://schema.org/y compacts.
        assert!(
            store
                .serialize("trig", true, None, true, None)
                .unwrap()
                .contains("schema:y"),
            "default (http) schema namespace compacts http://schema.org/y",
        );
    }

    /// `resolve_prefixes(None)` is exactly `default_prefixes()` — the unit-level back-compat
    /// invariant, independent of any serialiser run.
    #[test]
    fn resolve_prefixes_none_is_default() {
        assert_eq!(resolve_prefixes(None).unwrap(), default_prefixes());
    }

    /// Format parsing is case-insensitive and accepts the media types / `ttl` alias.
    #[test]
    fn serialize_format_aliases() {
        let store = Store::load(DATA, "turtle").unwrap();
        let a = store.serialize("turtle", true, None, true, None).unwrap();
        for alias in ["TURTLE", "Ttl", "text/turtle"] {
            assert_eq!(
                store.serialize(alias, true, None, true, None).unwrap(),
                a,
                "alias {alias} must serialise as turtle"
            );
        }
        // Round-trip: serialising then re-parsing yields the same triple count.
        let reparsed = Graph::load_str(&a, "turtle").unwrap();
        assert_eq!(reparsed.len(), store.size(), "serialise->parse round-trip");
    }

    // ---- sq-ixc3.5: JSON-LD serialise-out parity with the engine writer ----

    use sparq_engine::serialize::{graph_to_jsonld_pretty_with, graph_to_jsonld_with, JsonLdForm};

    /// PRETTY JSON-LD through the wasm `Store::serialize` is byte-identical to the engine
    /// serialiser it delegates to (`graph_to_jsonld_pretty_with`) over the same graph,
    /// form, and indent — the load-bearing parity invariant for the new JSON-LD path.
    #[test]
    fn serialize_jsonld_pretty_matches_engine() {
        let store = Store::load(DATA, "turtle").unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        for (fmt, form) in [
            ("jsonld", JsonLdForm::Expanded), // bare "jsonld" ⇒ expanded default
            ("jsonld-expanded", JsonLdForm::Expanded),
            ("jsonld-flattened", JsonLdForm::Flattened),
            ("jsonld-compacted", JsonLdForm::Compacted),
        ] {
            let got = store
                .serialize(fmt, true, Some("  ".to_string()), true, None)
                .unwrap();
            let opts = JsonLdPrettyOptions {
                indent: "  ".to_string(),
            };
            let want = graph_to_jsonld_pretty_with(&g, form, &default_prefixes(), &opts);
            assert_eq!(got, want, "wasm pretty JSON-LD ({fmt}) must equal the engine output");
            // The pretty document is multi-line JSON (the indenter inserted newlines).
            assert!(got.contains('\n'), "pretty JSON-LD is indented: {got}");
        }
    }

    /// Minified (non-pretty) JSON-LD matches the engine `graph_to_jsonld_with`, and the
    /// pretty/minified pair re-indent to the same document (the pretty pass is
    /// whitespace-only). The `abbreviate` flag is IGNORED for JSON-LD (form selects
    /// compaction), so passing it either way yields the same bytes.
    #[test]
    fn serialize_jsonld_minified_matches_engine_and_ignores_abbreviate() {
        let store = Store::load(DATA, "turtle").unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let got = store.serialize("jsonld-compacted", false, None, true, None).unwrap();
        let want = graph_to_jsonld_with(&g, JsonLdForm::Compacted, &default_prefixes());
        assert_eq!(got, want, "wasm minified JSON-LD must equal the engine output");
        // `abbreviate` is a no-op for JSON-LD: true vs false produces identical bytes.
        let with_abbrev_off = store.serialize("jsonld-compacted", false, None, false, None).unwrap();
        assert_eq!(got, with_abbrev_off, "abbreviate is ignored for JSON-LD");
        // Compacted carries a prefix `@context`; expanded does not.
        assert!(got.contains("@context"), "compacted form has @context: {got}");
        let expanded = store.serialize("jsonld-expanded", false, None, true, None).unwrap();
        assert!(!expanded.contains("@context"), "expanded form has no @context: {expanded}");
    }

    /// A custom indent flows through to the JSON-LD pretty writer (a four-space indent
    /// produces a different — but engine-matching — document than the two-space default),
    /// and `None` defaults to two spaces.
    #[test]
    fn serialize_jsonld_pretty_custom_indent() {
        let store = Store::load(DATA, "turtle").unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let two = store
            .serialize("jsonld-expanded", true, Some("  ".to_string()), true, None)
            .unwrap();
        let four = store
            .serialize("jsonld-expanded", true, Some("    ".to_string()), true, None)
            .unwrap();
        assert_ne!(two, four, "indent option must affect the JSON-LD output");
        let opts = JsonLdPrettyOptions {
            indent: "    ".to_string(),
        };
        assert_eq!(
            four,
            graph_to_jsonld_pretty_with(&g, JsonLdForm::Expanded, &default_prefixes(), &opts),
            "custom-indent JSON-LD must equal the engine output with the same opts"
        );
        let dflt = store.serialize("jsonld-expanded", true, None, true, None).unwrap();
        assert_eq!(dflt, two, "None indent uses the two-space default for JSON-LD");
    }

    /// JSON-LD round-trips: a serialised document re-parses to the same triple set. Guarded
    /// by `jsonld` (the INGEST parser) so it only runs when JSON-LD parsing is available;
    /// the parity tests above are the primary, parser-independent invariant.
    #[cfg(feature = "jsonld")]
    #[test]
    fn serialize_jsonld_round_trips() {
        let store = Store::load(DATA, "turtle").unwrap();
        for fmt in ["jsonld-expanded", "jsonld-flattened", "jsonld-compacted"] {
            let doc = store.serialize(fmt, true, None, true, None).unwrap();
            let reparsed = Graph::load_str(&doc, "jsonld").unwrap();
            assert_eq!(
                reparsed.len(),
                store.size(),
                "JSON-LD ({fmt}) serialise->parse round-trip preserves triple count"
            );
        }
    }

    // ---- sq-oy1f.5: full W3C JSON-LD 1.1 Compaction (caller `@context`) ----
    //
    // `serializeCompact(context, pretty, indent)` exposes the engine's
    // `graph_to_jsonld_compact` — the REAL Compaction Algorithm against a supplied `@context`,
    // not the prefix-only `jsonld-compacted` form of `serialize`. The `Ok` arm runs natively
    // (no `JsError::new`), so the load-bearing parity invariant is pinned here; the `Err` arm
    // (a malformed context) and the wasm32 boundary are covered by `tests/web.rs`.

    use sparq_engine::serialize::{
        graph_to_jsonld_compact, graph_to_jsonld_compact_pretty, parse_context_json,
    };

    /// `serializeCompact` is byte-identical to the engine `graph_to_jsonld_compact` over the
    /// same graph and `@context` — the parity invariant (the binding adds no reformatting),
    /// and the compaction actually fires (an `@vocab` context drops the namespace from the
    /// predicate keys, which the prefix-only `jsonld-compacted` form cannot do).
    #[test]
    fn serialize_compact_matches_engine() {
        let data = r#"<http://schema.org/x> <http://schema.org/name> "Alice" ;
                       <http://schema.org/knows> <http://schema.org/y> ."#;
        let store = Store::load(data, "turtle").unwrap();
        let g = Graph::load_str(data, "turtle").unwrap();
        let ctx_text = r#"{"@vocab":"http://schema.org/"}"#;
        let got = store.serialize_compact(ctx_text, false, None).unwrap();
        let ctx = parse_context_json(ctx_text).unwrap();
        let want = graph_to_jsonld_compact(&g, &ctx);
        assert_eq!(got, want, "serializeCompact must equal the engine writer");
        // The `@vocab` term-compaction the prefix-only form CANNOT do: bare `name`/`knows`
        // predicate keys (not `schema:name`), proving the full algorithm ran.
        assert!(got.contains("\"@vocab\":\"http://schema.org/\""), "context echoed: {got}");
        assert!(got.contains("\"name\":"), "predicate is @vocab-relative bare term: {got}");
        // Contrast: the prefix-only `jsonld-compacted` form keeps a `schema:` CURIE, never a
        // bare `name` key — so the new method genuinely exposes a richer surface.
        let prefix_only = store.serialize("jsonld-compacted", false, None, true, None).unwrap();
        assert!(!prefix_only.contains("\"name\":"), "prefix-only form is not @vocab: {prefix_only}");
    }

    /// The pretty arm equals `graph_to_jsonld_compact_pretty` and re-indents the minified
    /// document (whitespace-only), and a custom indent flows through.
    #[test]
    fn serialize_compact_pretty_matches_engine() {
        let data = r#"<http://ex/s> <http://ex/p> "v" ."#;
        let store = Store::load(data, "turtle").unwrap();
        let g = Graph::load_str(data, "turtle").unwrap();
        let ctx_text = r#"{"@vocab":"http://ex/"}"#;
        let ctx = parse_context_json(ctx_text).unwrap();
        let two = store.serialize_compact(ctx_text, true, Some("  ".to_string())).unwrap();
        let opts = JsonLdPrettyOptions { indent: "  ".to_string() };
        assert_eq!(two, graph_to_jsonld_compact_pretty(&g, &ctx, &opts));
        assert!(two.contains('\n'), "pretty compaction is multi-line: {two}");
        // A four-space indent differs; `None` defaults to two spaces.
        let four = store.serialize_compact(ctx_text, true, Some("    ".to_string())).unwrap();
        assert_ne!(two, four, "indent option must affect the output");
        let dflt = store.serialize_compact(ctx_text, true, None).unwrap();
        assert_eq!(dflt, two, "None indent uses the two-space default");
    }

    /// An empty `{}` IS a valid (no-op) `@context` — the `Ok` arm runs natively, so this
    /// pins that the parser accepts it (an expanded-shaped, full-IRI document). The malformed
    /// `@context` `Err` arm touches `JsError::new` (which traps off-wasm), so it lives in the
    /// wasm32 `tests/web.rs::serialize_compact_round_trips`, not here.
    #[test]
    fn serialize_compact_empty_context_is_ok() {
        let data = r#"<http://ex/s> <http://ex/p> "v" ."#;
        let store = Store::load(data, "turtle").unwrap();
        let g = Graph::load_str(data, "turtle").unwrap();
        let got = store.serialize_compact("{}", false, None).unwrap();
        let ctx = parse_context_json("{}").unwrap();
        assert_eq!(got, graph_to_jsonld_compact(&g, &ctx), "empty @context matches the engine");
        // No abbreviation with an empty context: the predicate stays a full IRI.
        assert!(got.contains("http://ex/p"), "full-IRI predicate with empty @context: {got}");
    }
}

// [OPUS-4.8] sq-9qz6 pt2 — headless wasm test of the JS-facing sparq-wasm API.
//
// The crate's `#[cfg(test)] mod tests` in src/lib.rs runs on the NATIVE target only and
// therefore exercises the engine functions the wrappers delegate to — NOT the actual
// `#[wasm_bindgen]` exports, because `JsError::new` is a wasm-bindgen import that panics
// off-wasm. This module closes that gap: every test here drives the REAL exported API
// (`Store::load`, `query`, `ask`, `count`, `queryQuads`, the cursors, the in-place update
// path) compiled to and executed in a genuine wasm32 runtime via `wasm-pack test --node`,
// so result serialisation across the JS boundary (the SPARQL-1.1-JSON shape returned by
// `query`/`query_json`, the N-Triples shape from `queryQuads`, the `JsError` Err path) is
// asserted end to end — exactly the surface CI's `cargo build --target wasm32` never ran.
//
// Datasets are tiny and inline. `wasm-pack test --node` runs each `#[wasm_bindgen_test]`
// in the Node executor (no browser, no DOM); Node is the default target, so no
// `wasm_bindgen_test_configure!(run_in_browser)` directive is needed (and `run_in_node`
// is not a valid directive in wasm-bindgen-test 0.3.x — the runner flag picks Node).

#![cfg(target_arch = "wasm32")]

use sparq_wasm::Store;
use wasm_bindgen_test::*;

const DATA: &str = r#"@prefix ex: <http://ex/> .
    ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
    ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

// ---- load + basic shape across the JS boundary ----

/// Turtle loads into the wasm Store and reports the deduplicated triple count + a
/// non-zero heap estimate — the two getters JS reads off a freshly loaded store.
#[wasm_bindgen_test]
fn load_turtle_size_and_heap() {
    let store = Store::load(DATA, "turtle").expect("turtle must load in wasm");
    // 2 names + 2 ages + 1 knows = 5 triples.
    assert_eq!(store.size(), 5);
    assert!(store.heap_bytes() > 0, "heap estimate must be non-zero");
}

/// N-Triples also loads (the other primary format JS callers pass).
#[wasm_bindgen_test]
fn load_ntriples() {
    let nt = "<http://ex/a> <http://ex/p> <http://ex/b> .\n<http://ex/a> <http://ex/p> <http://ex/c> .\n";
    let store = Store::load(nt, "ntriples").expect("ntriples must load in wasm");
    assert_eq!(store.size(), 2);
}

/// A malformed document surfaces as the `Err` (JsError) arm across the boundary, not a
/// panic / trap — the error path JS code relies on (`try { Store.load(...) } catch`).
#[wasm_bindgen_test]
fn load_error_is_err_not_trap() {
    let bad = Store::load("@prefix ex: <http://ex/> . ex:a ex:p", "turtle");
    assert!(
        bad.is_err(),
        "a truncated triple must return Err, not panic"
    );
}

// ---- SELECT -> SPARQL 1.1 JSON (the query_json shape) ----

/// `query` returns a well-formed SPARQL-1.1-JSON string: head vars in order, a typed
/// integer literal with its xsd:integer datatype, a language tag on "Bob"@en, a plain
/// string literal without a datatype, and exactly two solution rows — i.e. the full
/// serialisation contract crossing the wasm/JS boundary as a real JS string.
#[wasm_bindgen_test]
fn select_sparql_json_shape() {
    let store = Store::load(DATA, "turtle").unwrap();
    let json = store
        .query("PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a")
        .expect("select must return JSON");
    assert!(json.contains("\"vars\":[\"n\",\"a\"]"), "head vars: {json}");
    assert!(
        json.contains("\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\""),
        "integer datatype: {json}"
    );
    assert!(json.contains("\"xml:lang\":\"en\""), "lang tag: {json}");
    // A plain string literal omits the xsd:string datatype.
    assert!(
        json.contains("\"value\":\"Alice\"}"),
        "plain literal: {json}"
    );
    // Two solutions (one `?a` cell object each).
    assert_eq!(json.matches("\"a\":{").count(), 2, "two rows: {json}");
}

/// A URI-valued cell serialises as a `uri` term with its absolute IRI.
#[wasm_bindgen_test]
fn select_uri_term() {
    let store = Store::load(DATA, "turtle").unwrap();
    let json = store
        .query("PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:knows ?o }")
        .unwrap();
    assert!(
        json.contains("\"type\":\"uri\",\"value\":\"http://ex/bob\""),
        "uri term: {json}"
    );
}

/// A literal with a quote/backslash is JSON-escaped on the way across the boundary.
#[wasm_bindgen_test]
fn select_escaping() {
    let store = Store::load(
        "@prefix ex: <http://ex/> . ex:a ex:p \"q\\\"x\" .",
        "turtle",
    )
    .unwrap();
    let json = store.query("SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(
        json.contains("\"value\":\"q\\\"x\""),
        "escaped literal: {json}"
    );
}

/// An empty SELECT result is still a valid SPARQL-JSON doc with empty bindings.
#[wasm_bindgen_test]
fn select_empty_result() {
    let store = Store::load(DATA, "turtle").unwrap();
    let json = store
        .query("PREFIX ex: <http://ex/> SELECT ?x WHERE { ?s ex:nope ?x }")
        .unwrap();
    assert!(json.contains("\"bindings\":[]"), "empty bindings: {json}");
}

// ---- ASK -> plain boolean across the boundary ----

/// `ask` returns a native JS boolean (not a JSON doc): true when the pattern matches,
/// false when it does not, and FILTERs are evaluated.
#[wasm_bindgen_test]
fn ask_boolean() {
    let store = Store::load(DATA, "turtle").unwrap();
    assert!(store
        .ask("PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o }")
        .unwrap());
    assert!(!store
        .ask("PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }")
        .unwrap());
    assert!(store
        .ask("PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 28) }")
        .unwrap());
    assert!(!store
        .ask("PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 99) }")
        .unwrap());
}

/// A non-ASK query routed to `ask` is rejected as Err across the boundary.
#[wasm_bindgen_test]
fn ask_rejects_select() {
    let store = Store::load(DATA, "turtle").unwrap();
    assert!(store.ask("SELECT ?s WHERE { ?s ?p ?o }").is_err());
}

/// `askWithMaxRows`: a generous cap answers; a zero cap trips the working-set budget
/// (the portable, wasm-safe budget dimension) and returns Err.
#[wasm_bindgen_test]
fn ask_with_max_rows_budget() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o . ?s ex:age ?a FILTER(?a > 28) }";
    assert!(store.ask_with_max_rows(q, 1024).unwrap());
    assert!(
        store.ask_with_max_rows(q, 0).is_err(),
        "zero cap must trip the budget"
    );
}

// ---- count (read from the index, no materialisation) ----

/// `count` returns the solution count as a JS number.
#[wasm_bindgen_test]
fn count_solutions() {
    let store = Store::load(DATA, "turtle").unwrap();
    let n = store
        .count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:name ?n }")
        .unwrap();
    assert_eq!(n, 2);
}

// ---- CONSTRUCT / DESCRIBE -> N-Triples ----

/// `queryQuads` (CONSTRUCT) returns the constructed graph as N-Triples across the boundary.
#[wasm_bindgen_test]
fn construct_quads() {
    let store = Store::load(DATA, "turtle").unwrap();
    let nt = store
        .query_quads("PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }")
        .unwrap();
    assert_eq!(
        nt.lines().filter(|l| !l.trim().is_empty()).count(),
        2,
        "two triples: {nt}"
    );
    assert!(
        nt.contains("<http://ex/alice> <http://ex/label> \"Alice\" ."),
        "constructed line: {nt}"
    );
    assert!(nt.contains("\"Bob\"@en"), "lang tag preserved: {nt}");
}

/// A SELECT routed to `queryQuads` is rejected as Err across the boundary.
#[wasm_bindgen_test]
fn query_quads_rejects_select() {
    let store = Store::load(DATA, "turtle").unwrap();
    assert!(store.query_quads("SELECT ?s WHERE { ?s ?p ?o }").is_err());
}

// ---- streaming cursors: SolutionCursor / QueryChunks / QuadChunks ----

/// `queryCursor` yields self-contained SPARQL-JSON batches; with batch size 1 over a
/// 2-row result it produces two non-empty batches then exhausts, and exposes vars +
/// rowCount + batchSize getters across the boundary.
#[wasm_bindgen_test]
fn query_cursor_batches() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?n WHERE { ?s ex:name ?n } ORDER BY ?n";
    let mut cur = store.query_cursor(q, 1).unwrap();
    assert_eq!(cur.row_count(), 2);
    assert_eq!(cur.batch_size(), 1);
    assert_eq!(cur.vars(), vec!["s".to_string(), "n".to_string()]);

    let b0 = cur.next().expect("first batch");
    let b1 = cur.next().expect("second batch");
    assert!(cur.next().is_none(), "exhausted after the last batch");
    for b in [&b0, &b1] {
        assert!(
            b.contains("\"vars\":[\"s\",\"n\"]"),
            "each batch is a full SPARQL-JSON doc: {b}"
        );
        assert_eq!(b.matches("\"n\":{").count(), 1, "one row per batch: {b}");
    }
}

/// `queryChunks` streams the JSON bytes in row-boundary chunks; concatenating every
/// chunk reproduces `query`'s one-shot string exactly.
#[wasm_bindgen_test]
fn query_chunks_reassemble() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?n WHERE { ?s ex:name ?n } ORDER BY ?n";
    let whole = store.query(q).unwrap();
    let mut chunks = store.query_chunks(q).unwrap();
    let mut reassembled = String::new();
    while let Some(c) = chunks.next() {
        reassembled.push_str(&c);
    }
    assert_eq!(
        reassembled, whole,
        "concatenated chunks must equal the one-shot JSON"
    );
}

/// `queryQuadsChunks` batches the constructed graph; concatenation == `queryQuads`.
#[wasm_bindgen_test]
fn query_quads_chunks_reassemble() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }";
    let whole = store.query_quads(q).unwrap();
    let mut chunks = store.query_quads_chunks(q, 1).unwrap();
    let mut reassembled = String::new();
    let mut n = 0;
    while let Some(c) = chunks.next() {
        n += 1;
        reassembled.push_str(&c);
    }
    assert_eq!(n, 2, "batch_size 1 over 2 triples => 2 batches");
    assert_eq!(
        reassembled, whole,
        "chunked N-Triples must reassemble to the whole document"
    );
}

// ---- sq-0ptd (gh-54): string functions retained in the real wasm Store ----

/// CONTAINS / STRSTARTS / LCASE are exercisable through the REAL exported `Store::query`
/// in a genuine wasm runtime — none is compiled out for `wasm32`. This proves the trio the
/// gh-54 audit calls out survives the wasm build end to end (the native
/// `regression_string_functions` test asserts the same via the engine path).
#[wasm_bindgen_test]
fn string_functions_retained() {
    let data = r#"@prefix ex: <http://ex/> .
        ex:a ex:name "Alice" . ex:b ex:name "BOB" . ex:c ex:name "carol" ."#;
    let store = Store::load(data, "turtle").unwrap();

    // STRSTARTS: only "Alice" starts with "Al".
    let j = store
        .query(r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(STRSTARTS(?n, "Al")) }"#)
        .unwrap();
    assert!(j.contains("\"value\":\"Alice\""), "STRSTARTS: {j}");
    assert_eq!(j.matches("\"n\":{").count(), 1, "STRSTARTS one row: {j}");

    // CONTAINS over LCASE: "BOB"->"bob" and "carol" both contain "o".
    let j = store
        .query(r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(CONTAINS(LCASE(?n), "o")) }"#)
        .unwrap();
    assert_eq!(
        j.matches("\"n\":{").count(),
        2,
        "CONTAINS(LCASE(..)) matches BOB + carol: {j}"
    );

    // LCASE as a projected (BIND) value.
    let j = store
        .query(
            r#"PREFIX ex: <http://ex/> SELECT ?l WHERE { ex:b ex:name ?n BIND(LCASE(?n) AS ?l) }"#,
        )
        .unwrap();
    assert!(
        j.contains("\"value\":\"bob\""),
        "LCASE projects lowercase: {j}"
    );
}

/// [OPUS-4.8] sq-0ptd (gh-54): the NEGATIVE half of the audit — REGEX/REPLACE are compiled
/// OUT of the lean default browser bundle (the `regex` feature is OFF here) — asserted on the
/// REAL `wasm32` target where it is actually observable. `sparq-wasm` disables
/// `sparq-engine`'s defaults, and the wasm build links only `sparq-wasm`'s own (regex-off)
/// `sparq-engine` — it does NOT unify the rest of the workspace's default-feature
/// `sparq-engine` deps, so regex is genuinely absent. (The mirror native unit test was
/// removed: a native `--workspace` build unifies `sparq-engine/regex` ON via other crates,
/// making the native negative unobservable — see the note in `lib.rs`.) Through the real
/// exported `Store::query`, a REGEX/REPLACE query is REJECTED (the `JsError` Err arm), not
/// silently empty. `--features regex` re-enables them (positive half: `string_functions`
/// always-present trio above + the native `regex_present_when_enabled`).
#[cfg(not(feature = "regex"))]
#[wasm_bindgen_test]
fn regex_compiled_out_by_default() {
    let data = r#"@prefix ex: <http://ex/> . ex:a ex:name "Alice" ."#;
    let store = Store::load(data, "turtle").unwrap();
    assert!(
        store
            .query(
                r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(REGEX(?n, "^Al")) }"#,
            )
            .is_err(),
        "REGEX with the `regex` feature off must be rejected (not silently empty)"
    );
    // REPLACE is gated identically.
    assert!(
        store
            .query(
                r#"PREFIX ex: <http://ex/> SELECT (REPLACE(?n, "A", "X") AS ?r) WHERE { ?s ex:name ?n }"#,
            )
            .is_err(),
        "REPLACE with the `regex` feature off must be rejected"
    );
}

// ---- compressed store: identical results, smaller footprint ----

/// `loadCompressed` returns byte-identical SELECT JSON to the raw store and reports a
/// footprint no larger than the raw store — verified inside the real wasm runtime where
/// the 3-permutation compact index is the one actually selected.
#[wasm_bindgen_test]
fn compressed_matches_raw() {
    let raw = Store::load(DATA, "turtle").unwrap();
    let cmp = Store::load_compressed(DATA, "turtle").unwrap();
    assert_eq!(raw.size(), cmp.size());
    let q =
        "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a";
    assert_eq!(
        raw.query(q).unwrap(),
        cmp.query(q).unwrap(),
        "compressed JSON must match raw"
    );
    assert!(
        cmp.heap_bytes() <= raw.heap_bytes(),
        "compressed footprint must not exceed raw"
    );
}

// ---- mutation paths: update / updateInPlace / applyDelta ----

/// `update` returns a NEW store with the inserted data; the receiver is unchanged
/// (immutable rebuild semantics) — both observable across the boundary.
#[wasm_bindgen_test]
fn update_returns_new_store() {
    let store = Store::load(DATA, "turtle").unwrap();
    let next = store
        .update("PREFIX ex: <http://ex/> INSERT DATA { ex:carol ex:name \"Carol\" }")
        .unwrap();
    assert_eq!(store.size(), 5, "receiver unchanged after update");
    assert_eq!(next.size(), 6, "new store has the inserted triple");
    assert!(next
        .ask("PREFIX ex: <http://ex/> ASK { ex:carol ex:name \"Carol\" }")
        .unwrap());
}

/// `updateInPlace` mutates the receiver through the delta overlay (O(batch), no rebuild).
#[wasm_bindgen_test]
fn update_in_place_mutates_receiver() {
    let mut store = Store::load(DATA, "turtle").unwrap();
    store
        .update_in_place("PREFIX ex: <http://ex/> INSERT DATA { ex:dave ex:age 40 }")
        .unwrap();
    assert_eq!(store.size(), 6);
    assert!(store
        .ask("PREFIX ex: <http://ex/> ASK { ex:dave ex:age 40 }")
        .unwrap());
}

/// `applyDelta` applies an N-Triples insert/delete batch in one shot through the overlay.
#[wasm_bindgen_test]
fn apply_delta_batch() {
    let mut store = Store::load(DATA, "turtle").unwrap();
    let inserts = "<http://ex/eve> <http://ex/name> \"Eve\" .\n";
    let deletes = "<http://ex/alice> <http://ex/knows> <http://ex/bob> .\n";
    store.apply_delta(inserts, deletes).unwrap();
    // -1 (knows removed) +1 (eve name) = net 5.
    assert_eq!(store.size(), 5);
    assert!(store
        .ask("PREFIX ex: <http://ex/> ASK { ex:eve ex:name \"Eve\" }")
        .unwrap());
    assert!(!store
        .ask("PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }")
        .unwrap());
}

// ---- [OPUS-4.8] sq-ty78o (#1114): the empty `new Store()` constructor, in real wasm ----

/// `new Store()` (the wasm `constructor`) builds an empty, mutable store, and a
/// `GRAPH`-targeted `updateInPlace` insert then a `GRAPH ?g` query round-trips THROUGH THE
/// REAL wasm export — the named-graph gap the issue reported, closed by the constructor.
#[wasm_bindgen_test]
fn new_store_named_graph_roundtrip() {
    let mut store = Store::new().expect("new Store() must succeed");
    assert_eq!(store.size(), 0, "a new Store() is empty");
    store
        .update_in_place(
            "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
        )
        .expect("named-graph INSERT DATA must apply to an empty store");
    let json = store
        .query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
        .expect("GRAPH query must run");
    assert!(
        json.contains("\"value\":\"http://ex/g\""),
        "named graph round-trips through new Store(): {json}"
    );
}

// ---- [OPUS-4.8] sq-f66jz (#1115): base-IRI load (`loadWithBase`), in real wasm ----

/// `Store::loadWithBase` resolves relative IRIs against the base through the real wasm
/// export, and a syntactically invalid base surfaces as the `JsError` Err arm (not a trap).
#[wasm_bindgen_test]
fn load_with_base_resolves_relative() {
    let store = Store::load_with_base("<a> <p> <../up/o> .", "turtle", "http://ex/dir/")
        .expect("base-IRI load must succeed");
    assert_eq!(store.size(), 1);
    let json = store.query("SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(
        json.contains("\"value\":\"http://ex/dir/a\""),
        "relative subject resolved against base: {json}"
    );
    assert!(
        json.contains("\"value\":\"http://ex/up/o\""),
        "relative object resolved against base: {json}"
    );
    // An invalid base IRI is the `Err` (JsError) arm across the boundary, not a trap.
    assert!(
        Store::load_with_base("<a> <p> <o> .", "turtle", "not a iri").is_err(),
        "an invalid base IRI must return Err, not panic"
    );
}

// ---- [OPUS-4.8] sq-fe1s: the opt-in `serialize` binding, in real wasm ----
//
// These exercise the REAL exported `Store::serialize(format, pretty, indent, abbreviate)`
// through the wasm/JS boundary — both the document string it returns (the pretty-Turtle /
// pretty-TriG shape) and the `JsError` arm for an unknown format. Compiled only under the
// `serialize-rdf` feature, exactly as the binding is. The native `#[cfg(test)]` tests in
// src/serialize.rs assert byte-parity with the engine writer; this proves the method
// actually exports to and runs in wasm, and that the unknown-format Err arm (which DOES
// touch `JsError::new`, untestable natively) surfaces as an `Err` rather than a trap.
#[cfg(feature = "serialize-rdf")]
mod serialize {
    use super::*;

    /// Pretty Turtle through the wasm `Store::serialize` returns a non-empty document with
    /// a `@prefix` header (abbreviate=true) carrying the store's triples.
    #[wasm_bindgen_test]
    fn serialize_turtle_pretty() {
        let store = Store::load(DATA, "turtle").unwrap();
        let ttl = store
            .serialize("turtle", true, Some("  ".to_string()), true, None)
            .unwrap();
        // The well-known `xsd:` prefix (used by the integer ages) is declared + compacted.
        assert!(ttl.contains("@prefix xsd:"), "prefix header: {ttl}");
        assert!(ttl.contains("xsd:integer"), "datatype compacted: {ttl}");
        // The data is present (ex: is not a well-known prefix, so it stays in full form).
        assert!(ttl.contains("<http://ex/alice>"), "subject present: {ttl}");
        assert!(ttl.contains("\"Bob\"@en"), "lang tag preserved: {ttl}");
    }

    /// `abbreviate=false` keeps every IRI in full `<…>` form with no `@prefix` header.
    #[wasm_bindgen_test]
    fn serialize_turtle_no_abbreviate() {
        let store = Store::load(DATA, "turtle").unwrap();
        let ttl = store.serialize("turtle", true, None, false, None).unwrap();
        assert!(!ttl.contains("@prefix"), "no header when abbreviate=false: {ttl}");
        assert!(ttl.contains("<http://ex/alice>"), "full IRIs: {ttl}");
    }

    /// Pretty TriG over a dataset emits a `GRAPH <g> { … }` block for the named graph.
    #[wasm_bindgen_test]
    fn serialize_trig_named_graph() {
        let nq = "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
        let store = Store::load_dataset(nq, "nquads").unwrap();
        let trig = store.serialize("trig", true, None, true, None).unwrap();
        assert!(trig.contains("GRAPH"), "named graph as a GRAPH block: {trig}");
    }

    /// An unknown format string surfaces as the `JsError` Err arm across the boundary, not
    /// a trap — the error path JS relies on (`try { store.serialize("xml", …) } catch`).
    #[wasm_bindgen_test]
    fn serialize_unknown_format_is_err() {
        let store = Store::load(DATA, "turtle").unwrap();
        assert!(
            store.serialize("rdfxml", true, None, true, None).is_err(),
            "an unrecognised format must return Err, not panic"
        );
    }

    // ---- [OPUS-4.8] sq-ixc3.5: the JSON-LD serialise-out path, in real wasm ----

    /// Pretty JSON-LD through the wasm `Store::serialize`, run in a genuine wasm32 runtime,
    /// is byte-identical to the engine writer it delegates to
    /// (`graph_to_jsonld_pretty_with`) — the load-bearing parity invariant proved across
    /// the JS boundary, not just natively. Exercises all three forms.
    #[wasm_bindgen_test]
    fn serialize_jsonld_pretty_matches_engine() {
        use sparq_engine::serialize::{
            default_prefixes, graph_to_jsonld_pretty_with, JsonLdForm, JsonLdPrettyOptions,
        };
        use sparq_core::Graph;
        let store = Store::load(DATA, "turtle").unwrap();
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let opts = JsonLdPrettyOptions {
            indent: "  ".to_string(),
        };
        for (fmt, form) in [
            ("jsonld", JsonLdForm::Expanded),
            ("jsonld-expanded", JsonLdForm::Expanded),
            ("jsonld-flattened", JsonLdForm::Flattened),
            ("jsonld-compacted", JsonLdForm::Compacted),
        ] {
            let got = store
                .serialize(fmt, true, Some("  ".to_string()), true, None)
                .unwrap();
            let want = graph_to_jsonld_pretty_with(&g, form, &default_prefixes(), &opts);
            assert_eq!(got, want, "wasm pretty JSON-LD ({fmt}) must equal the engine output");
            assert!(got.contains('\n'), "pretty JSON-LD is indented: {got}");
        }
        // The compacted form carries a prefix `@context`; expanded does not — a quick
        // shape check that the form selection actually reached the writer.
        let compacted = store
            .serialize("jsonld-compacted", true, None, true, None)
            .unwrap();
        assert!(compacted.contains("@context"), "compacted has @context: {compacted}");
    }

    // ---- [OPUS-4.8] sq-l5kr: the OPTIONAL caller-supplied prefix map, in real wasm ----

    /// A caller-supplied `prefixes` array (`[[prefix, iri], …]`) drives compaction across
    /// the JS boundary: `http://example.org/x` abbreviates to `ex:x`, and the SITE's
    /// `https://schema.org/` (note HTTPS — `default_prefixes()` uses HTTP) is used for the
    /// `schema` prefix. This is the new capability the native tests can't reach (a
    /// `js_sys::Array` only exists in a real wasm runtime), and the byte-for-byte parity
    /// with the engine writer fed the SAME pair list is the load-bearing invariant.
    #[wasm_bindgen_test]
    fn serialize_with_prefix_map() {
        use sparq_core::Graph;
        use sparq_engine::serialize::{
            graph_to_jsonld_with, graph_to_turtle_pretty_with, prefixes_from_pairs, JsonLdForm,
            PrettyOptions,
        };
        use wasm_bindgen::JsValue;

        let data = "<http://example.org/x> a <https://schema.org/Thing> ;\n\
                       <http://example.org/p> <https://schema.org/y> .\n";
        let store = Store::load(data, "turtle").unwrap();

        // Build the JS `[[prefix, iri], …]` array the binding reads.
        let prefixes = js_sys::Array::new();
        for (p, n) in [
            ("ex", "http://example.org/"),
            ("schema", "https://schema.org/"),
        ] {
            let pair = js_sys::Array::new();
            pair.push(&JsValue::from_str(p));
            pair.push(&JsValue::from_str(n));
            prefixes.push(&pair);
        }

        // Pretty Turtle through the binding with the custom map.
        let got = store
            .serialize("turtle", true, Some("  ".to_string()), true, Some(prefixes.clone()))
            .unwrap();
        // Byte-for-byte equal to the engine writer fed the SAME pair list.
        let g = Graph::load_str(data, "turtle").unwrap();
        let map = prefixes_from_pairs([
            ("ex", "http://example.org/"),
            ("schema", "https://schema.org/"),
        ]);
        let want = graph_to_turtle_pretty_with(&g, &map, &PrettyOptions::default());
        assert_eq!(got, want, "wasm prefix-map Turtle must equal the engine output:\n{got}");
        // The new capability: example.org abbreviates and the HTTPS schema is used.
        assert!(got.contains("ex:x"), "ex:x abbreviated: {got}");
        assert!(got.contains("ex:p"), "ex:p abbreviated: {got}");
        assert!(got.contains("schema:Thing"), "https schema type: {got}");
        assert!(
            got.contains("@prefix schema: <https://schema.org/> ."),
            "HTTPS schema namespace in header: {got}"
        );
        assert!(!got.contains("http://schema.org/"), "no default HTTP schema: {got}");

        // The same custom policy reaches the JSON-LD compacted `@context`.
        let jc = store
            .serialize("jsonld-compacted", false, None, true, Some(prefixes))
            .unwrap();
        let jc_want = graph_to_jsonld_with(&g, JsonLdForm::Compacted, &map);
        assert_eq!(jc, jc_want, "wasm prefix-map JSON-LD must equal the engine output:\n{jc}");
        assert!(jc.contains("\"schema\":\"https://schema.org/\""), "@context https schema: {jc}");

        // `None` (omitted) keeps using the engine defaults — back-compat across the boundary:
        // `http://example.org/` is NOT a default prefix, so those IRIs stay full `<…>`.
        let dflt = store.serialize("turtle", true, None, true, None).unwrap();
        assert!(
            dflt.contains("<http://example.org/x>"),
            "None arm uses defaults (no ex: prefix), so example.org stays full: {dflt}"
        );
    }

    /// A malformed `prefixes` entry (an element that is not a two-string array) is rejected
    /// with an `Err` across the boundary, not a silent wrong document or a trap.
    #[wasm_bindgen_test]
    fn serialize_malformed_prefix_map_is_err() {
        use wasm_bindgen::JsValue;
        let store = Store::load(DATA, "turtle").unwrap();
        // An element that is a bare string, not a `[prefix, iri]` array.
        let bad = js_sys::Array::new();
        bad.push(&JsValue::from_str("not-a-pair"));
        assert!(
            store.serialize("turtle", true, None, true, Some(bad)).is_err(),
            "a malformed prefixes entry must return Err, not panic or silently drop"
        );
    }

    // ---- [OPUS-4.8] sq-oy1f.5: full JSON-LD 1.1 Compaction (caller `@context`), real wasm ----

    /// `serializeCompact(context, …)` in a genuine wasm32 runtime: a `@vocab` `@context`
    /// drives the full Compaction Algorithm (bare `@vocab`-relative predicate keys the
    /// prefix-only form cannot produce), the output is byte-identical to the engine writer,
    /// and — the load-bearing invariant — the compacted document round-trips back through the
    /// JSON-LD parser to the SAME triples. The parity assertion runs unconditionally; the
    /// round-trip is guarded by `jsonld` (the INGEST parser).
    #[wasm_bindgen_test]
    fn serialize_compact_round_trips() {
        use sparq_core::Graph;
        use sparq_engine::serialize::{graph_to_jsonld_compact, parse_context_json};
        let data = r#"<http://schema.org/x> <http://schema.org/name> "Alice" ;
                       <http://schema.org/knows> <http://schema.org/y> ."#;
        let store = Store::load(data, "turtle").unwrap();
        let g = Graph::load_str(data, "turtle").unwrap();
        let ctx_text = r#"{"@vocab":"http://schema.org/"}"#;
        // The pretty form is multi-line (the indenter inserts a space after each colon, so
        // the exact no-space substrings are asserted on the MINIFIED doc below).
        let got = store.serialize_compact(ctx_text, true, Some("  ".to_string())).unwrap();
        assert!(got.contains('\n'), "pretty compaction is multi-line: {got}");
        let ctx = parse_context_json(ctx_text).unwrap();
        // Minified parity with the engine writer across the JS boundary + the @vocab shape.
        let minified = store.serialize_compact(ctx_text, false, None).unwrap();
        assert!(minified.contains("\"@vocab\":\"http://schema.org/\""), "context echoed: {minified}");
        assert!(minified.contains("\"name\":"), "predicate is @vocab-relative bare term: {minified}");
        assert_eq!(minified, graph_to_jsonld_compact(&g, &ctx), "wasm compaction == engine");
        // A non-object context surfaces as the Err arm across the boundary, not a trap.
        assert!(
            store.serialize_compact("[\"not an object\"]", false, None).is_err(),
            "a malformed @context must Err",
        );
        // Round-trip: the compacted doc re-parses to the same triple count (jsonld ingest).
        #[cfg(feature = "jsonld")]
        {
            let reparsed = Graph::load_str(&minified, "jsonld").unwrap();
            assert_eq!(reparsed.len(), store.size(), "compaction round-trips to the same triples");
        }
    }
}

// ---- [OPUS-4.8] sq-yqi1 (#162): the opt-in SHACL `validate` binding, in real wasm ----
//
// These exercise the REAL exported `Store::validate(data, shapes, format)` through the
// wasm/JS boundary (the JSON string it returns and the JsError parse-error arm) — the
// surface PSS's ADR-0014 ShaclValidator seam consumes. Compiled only under the `shacl`
// feature, exactly as the binding is. The native `#[cfg(test)]` tests in src/shacl.rs
// cover the JSON serialiser; this proves it actually exports to and runs in wasm.
#[cfg(feature = "shacl")]
mod shacl {
    use super::*;

    const SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property [
            sh:path ex:age ;
            sh:datatype xsd:integer ;
            sh:minInclusive 0 ;
            sh:message "age must be a non-negative integer" ;
          ] ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] .
    "#;

    /// Conforming data validates to `{"conforms":true,"results":[]}` across the boundary.
    #[wasm_bindgen_test]
    fn validate_conforming() {
        let store = Store::load("", "turtle").unwrap();
        let data = r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#;
        let json = store
            .validate(data, SHAPES, "turtle")
            .expect("validate must return a report");
        assert_eq!(json, r#"{"conforms":true,"results":[]}"#, "{json}");
    }

    /// Violating data surfaces a constraint violation with focusNode / path / message —
    /// the fields PSS consumes — as a parseable JSON report.
    #[wasm_bindgen_test]
    fn validate_violating() {
        let store = Store::load("", "turtle").unwrap();
        let data = r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#;
        let json = store.validate(data, SHAPES, "turtle").unwrap();
        assert!(json.contains(r#""conforms":false"#), "{json}");
        assert!(
            json.contains(r#""focusNode":"<http://example.org/bob>""#),
            "focus node: {json}"
        );
        assert!(
            json.contains(r#""path":"<http://example.org/age>""#),
            "path: {json}"
        );
        assert!(
            json.contains(r#""message":"age must be a non-negative integer""#),
            "declared message: {json}"
        );
        assert!(
            json.contains("MinInclusiveConstraintComponent"),
            "minInclusive: {json}"
        );
        assert!(
            json.contains("MinCountConstraintComponent"),
            "minCount (missing name): {json}"
        );
    }

    /// A malformed shapes/data graph surfaces as the JsError Err arm, not a trap.
    #[wasm_bindgen_test]
    fn validate_parse_error_is_err() {
        let store = Store::load("", "turtle").unwrap();
        let bad = store.validate("@prefix ex: <http://ex/> . ex:a ex:p", SHAPES, "turtle");
        assert!(bad.is_err(), "a truncated data graph must return Err");
    }

    /// [SONNET-4.6] gh-2520: `validateStore(shapes, format)` validates the store's
    /// OWN triples across the wasm/JS boundary — the stateful counterpart. (Report
    /// equivalence with the stateless one-shot, which holds only modulo the
    /// blank-node labels each shapes parse mints afresh, is asserted in the native
    /// `src/shacl.rs` tests; here we pin that the export reads the stored triples.)
    #[wasm_bindgen_test]
    fn validate_store_reads_stored_triples() {
        let store = Store::load(
            r#"
            @prefix ex: <http://example.org/> .
            ex:bob a ex:Person ; ex:age -1 .
        "#,
            "turtle",
        )
        .unwrap();
        let json = store
            .validate_store(SHAPES, "turtle")
            .expect("validateStore must return a report");
        assert!(json.contains(r#""conforms":false"#), "{json}");
        assert!(
            json.contains(r#""focusNode":"<http://example.org/bob>""#),
            "the store's own focus node: {json}"
        );
        assert!(
            json.contains(r#""message":"age must be a non-negative integer""#),
            "declared message: {json}"
        );

        // An empty store conforms vacuously — the receiver, not an argument, is
        // what changed between the two calls.
        let empty = Store::load("", "turtle").unwrap();
        assert_eq!(
            empty.validate_store(SHAPES, "turtle").unwrap(),
            r#"{"conforms":true,"results":[]}"#
        );
    }

    /// A malformed shapes document surfaces as the JsError Err arm (the arm that
    /// cannot run natively), not a trap.
    #[wasm_bindgen_test]
    fn validate_store_shapes_parse_error_is_err() {
        let store = Store::load("", "turtle").unwrap();
        let bad = store.validate_store("@prefix sh: <http://ex/> . sh:a sh:p", "turtle");
        assert!(bad.is_err(), "a truncated shapes graph must return Err");
    }
}

// ---- [OPUS-4.8] sq-quly (#796): the opt-in SCS parse binding, in real wasm ----
//
// These exercise the REAL exported `Store::parseShaclCompact(text, base?)` through the
// wasm/JS boundary — both the shapes-graph Turtle string it returns AND the JsError arm
// for a malformed SCS document (which DOES touch `JsError::new`, untestable natively).
// Compiled only under the `scs` feature, exactly as the binding is. The native
// `#[cfg(test)]` tests in src/scs.rs assert the engine-writer parity + that the parsed
// shapes drive validation; this proves the method actually exports to and runs in wasm.
#[cfg(feature = "scs")]
mod scs {
    use super::*;

    // SCS document exercising a shapeClass + property shapes with a path, datatype and
    // a `[1..1]` cardinality (the playground "Compact → shapes" input shape).
    const SCS: &str = "\
PREFIX ex: <http://example.org/>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>

shapeClass ex:Person {
\tex:name xsd:string [1..1] .
\tex:age xsd:integer .
}
";

    /// `parseShaclCompact` parses an SCS document and returns the shapes graph as a
    /// Turtle string carrying the key shape triples (the node-shape type, the Person
    /// shape IRI, and both property paths) — proved across the JS boundary in a real
    /// wasm32 runtime. The returned document re-parses through `Store.load`.
    #[wasm_bindgen_test]
    fn parse_shacl_compact_round_trips() {
        let store = Store::load("", "turtle").expect("empty store loads");
        let ttl = store
            .parse_shacl_compact(SCS, None)
            .expect("SCS must parse + serialise across the boundary");
        // Key shape triples present.
        assert!(ttl.contains("sh:NodeShape"), "node shape type: {ttl}");
        assert!(
            ttl.contains("<http://example.org/Person>"),
            "Person shape IRI: {ttl}"
        );
        assert!(
            ttl.contains("<http://example.org/name>"),
            "ex:name path: {ttl}"
        );
        assert!(
            ttl.contains("<http://example.org/age>"),
            "ex:age path: {ttl}"
        );
        // The returned Turtle re-parses (a real, loadable shapes graph) — round-trip.
        let reparsed = Store::load(&ttl, "turtle").expect("shapes Turtle re-parses");
        assert!(reparsed.size() > 0, "shapes graph is non-empty");
    }

    /// The explicit `base` argument resolves a relative shape IRI when the SCS document
    /// declares no `BASE` — the optional second arg crosses the boundary correctly.
    #[wasm_bindgen_test]
    fn parse_shacl_compact_uses_base_argument() {
        let store = Store::load("", "turtle").unwrap();
        let ttl = store
            .parse_shacl_compact("shape <#S> {\n}\n", Some("http://b.example/".to_string()))
            .unwrap();
        assert!(
            ttl.contains("<http://b.example/#S>"),
            "base argument resolves the relative IRI: {ttl}"
        );
    }

    /// A malformed SCS document surfaces as the `JsError` Err arm across the boundary,
    /// not a trap — the error path JS relies on (`try { store.parseShaclCompact(...) }`).
    #[wasm_bindgen_test]
    fn scs_parse_error_is_err() {
        let store = Store::load("", "turtle").unwrap();
        // An unterminated shape block is a parse error.
        let bad = store.parse_shacl_compact("shape <#S> {\n", None);
        assert!(bad.is_err(), "a malformed SCS document must return Err, not panic");
    }
}

// ---- [OPUS-4.8] sq-1dd5t (#1047): the real wasm32 `canonicalizeNQuads` export ----
//
// The src/canon.rs `#[cfg(test)] mod tests` run NATIVELY (asserting against the
// `sparq_canon` fn the export delegates to — `JsError::new` panics off-wasm), so they
// never touch the actual `#[wasm_bindgen]` free function. These drive the genuine wasm32
// export through the JS boundary: the `String -> String` marshalling and the `Err`
// (JsError) path the @sparq-org/sparq RDF/JS `Dataset` relies on for toCanonical/equals/contains.
#[cfg(feature = "canon")]
mod canon {
    use super::*;
    use sparq_wasm::canonicalize_nquads;

    /// Two N-Quads documents that are RDF-isomorphic but differ in blank-node labels AND
    /// quad order canonicalize, across the wasm boundary, to byte-identical output with
    /// `_:c14nN` labels.
    #[wasm_bindgen_test]
    fn canonicalize_nquads_isomorphic() {
        let a = "_:b0 <http://ex/p> _:b1 .\n_:b1 <http://ex/q> \"v\" .\n";
        let b = "_:y <http://ex/q> \"v\" .\n_:x <http://ex/p> _:y .\n";
        let ca = canonicalize_nquads(a).expect("canonicalize a");
        let cb = canonicalize_nquads(b).expect("canonicalize b");
        assert_eq!(ca, cb, "isomorphic datasets canonicalize identically");
        assert!(ca.contains("_:c14n"), "relabelled to c14nN: {ca}");
    }

    /// A malformed N-Quads document crosses the boundary as the `Err` (JsError) arm, not a trap.
    #[wasm_bindgen_test]
    fn canonicalize_nquads_malformed_is_err() {
        assert!(canonicalize_nquads("_:b0 <http://ex/p>").is_err());
    }
}

// ---- explain-json (feature-gated): the structured plan tree across the boundary ----
//
// [FABLE-5] sq-ixc3.19. The native `tests/exported_api.rs::explain_json` covers the Ok
// arms for coverage; these drive the genuine wasm32 exports through the JS boundary —
// including the `Err` (JsError) arms `JsError::new` makes wasm32-only — and pin that
// ANALYZE `nanos` are REAL on wasm32 (the `performance.now()` host clock, sq-vx7ez,
// #2428 — before it, no monotonic clock existed and every reading was 0) while
// `actual` row counts stay exact.
#[cfg(feature = "explain-json")]
mod explain_json {
    use super::*;

    /// `explainPlanJson` returns the camelCase planning-only tree (nothing executed).
    #[wasm_bindgen_test]
    fn explain_plan_json_dry_run() {
        let store = Store::load(DATA, "turtle").expect("load");
        let json = store
            .explain_plan_json("PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }")
            .expect("explainPlanJson");
        assert!(json.contains("\"operator\":"), "schema keys: {json}");
        assert!(json.contains("\"actual\":null"), "dry run: {json}");
    }

    /// `explainPlanAnalyzeJson` executes: exact `actual` rows, and `nanos` present as a
    /// NUMBER (never `null` in an ANALYZE tree).
    #[wasm_bindgen_test]
    fn explain_plan_analyze_json_fills_actuals() {
        let store = Store::load(DATA, "turtle").expect("load");
        let json = store
            .explain_plan_analyze_json("PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }")
            .expect("explainPlanAnalyzeJson");
        assert!(json.contains("\"actual\":2"), "exact rows: {json}");
        assert!(!json.contains("\"nanos\":null"), "ANALYZE fills nanos: {json}");
    }

    /// The `performance.now()` host clock gives real (non-zero) per-operator wall
    /// times on wasm32 (sq-vx7ez, #2428). A few-thousand-row join keeps the root
    /// operator's span comfortably above Node's `performance.now()` resolution, so a
    /// zero reading would mean the clock never routed through the hook.
    #[wasm_bindgen_test]
    fn explain_plan_analyze_json_real_nanos_via_host_clock() {
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..200 {
            ttl.push_str(&format!("ex:s{} ex:p ex:o{} .\n", i, i % 5));
        }
        let store = Store::load(&ttl, "turtle").expect("load");
        let json = store
            .explain_plan_analyze_json(
                "SELECT * WHERE { ?a <http://ex/p> ?x . ?b <http://ex/p> ?x }",
            )
            .expect("explainPlanAnalyzeJson");
        // The FIRST `"nanos":` in the pre-order JSON is the root operator's.
        let root_nanos: u64 = json
            .split("\"nanos\":")
            .nth(1)
            .expect("ANALYZE tree carries nanos")
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .expect("root nanos is a number");
        assert!(root_nanos > 0, "host clock supplies real wall time: {json}");
    }

    /// A malformed query and a graph-valued ANALYZE cross the boundary as the `Err`
    /// (JsError) arm, not a trap.
    #[wasm_bindgen_test]
    fn explain_json_err_arms() {
        let store = Store::load(DATA, "turtle").expect("load");
        assert!(store.explain_plan_json("SELECT WHERE {").is_err());
        assert!(store
            .explain_plan_analyze_json("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
            .is_err());
    }
}

// ---- [FABLE-5] sq-3ul2n.3: the opt-in `Store::loadBytes` byte-ingest binding ----
//
// Drives the REAL `#[wasm_bindgen]` byte-ingest exports in a genuine wasm32 runtime: the
// result-equivalence invariant (`loadBytes(utf8_of(s), fmt)` == `load(s, fmt)`, incl. a
// multi-byte UTF-8 fixture) and — the surface the native tests can't reach without
// trapping on `JsError::new` — the fail-closed invalid-UTF-8 `Err` path.
#[cfg(feature = "bytes-ingest")]
mod bytes_ingest {
    use super::*;

    /// `loadBytes(utf8_bytes_of(s), fmt)` yields a store equal to `load(s, fmt)` — equal
    /// triple count AND byte-identical probe-query JSON. The load-bearing equivalence
    /// invariant across the JS/wasm boundary.
    #[wasm_bindgen_test]
    fn load_bytes_equivalent_to_load() {
        let via_str = Store::load(DATA, "turtle").expect("load str");
        let via_bytes = Store::load_bytes(DATA.as_bytes(), "turtle").expect("load bytes");
        assert_eq!(via_bytes.size(), via_str.size(), "equal triple count");
        let q =
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n ; ex:age ?a } ORDER BY ?a";
        assert_eq!(
            via_bytes.query(q).unwrap(),
            via_str.query(q).unwrap(),
            "probe-query JSON must be identical across the two ingest paths"
        );
    }

    /// A multi-byte UTF-8 fixture (accented, CJK, emoji literals) survives the byte path
    /// intact and equals the string path — the raw byte copy is not mangling non-ASCII.
    #[wasm_bindgen_test]
    fn load_bytes_multibyte_utf8_equivalent() {
        let data = "@prefix ex: <http://ex/> .\n\
                    ex:a ex:name \"\u{c1}m\u{e9}lie\" .\n\
                    ex:b ex:name \"\u{65e5}\u{672c}\u{8a9e}\" .\n\
                    ex:c ex:name \"crab \u{1f980}\" .\n";
        let via_str = Store::load(data, "turtle").expect("load str");
        let via_bytes = Store::load_bytes(data.as_bytes(), "turtle").expect("load bytes");
        assert_eq!(via_bytes.size(), via_str.size());
        let q = "SELECT ?n WHERE { ?s <http://ex/name> ?n } ORDER BY ?n";
        let got = via_bytes.query(q).unwrap();
        assert_eq!(got, via_str.query(q).unwrap(), "multi-byte JSON equal");
        // The CJK + emoji literals really crossed the byte boundary intact.
        assert!(got.contains("\u{65e5}\u{672c}\u{8a9e}"), "CJK literal preserved: {got}");
        assert!(got.contains("\u{1f980}"), "emoji literal preserved: {got}");
    }

    /// `loadBytesWithBase` resolves relative IRIs against the base, equal to the string
    /// `loadWithBase`.
    #[wasm_bindgen_test]
    fn load_bytes_with_base_equivalent() {
        let doc = "<a> <p> <o> .";
        let via_str = Store::load_with_base(doc, "turtle", "http://ex/dir/").expect("load str");
        let via_bytes = Store::load_bytes_with_base(doc.as_bytes(), "turtle", "http://ex/dir/")
            .expect("load bytes");
        assert_eq!(via_bytes.size(), via_str.size());
        // `<p>` resolves to `http://ex/dir/p` under the base, so query on the resolved IRI.
        let q = "SELECT ?s ?o WHERE { ?s <http://ex/dir/p> ?o }";
        let got = via_bytes.query(q).unwrap();
        assert_eq!(got, via_str.query(q).unwrap());
        assert!(got.contains("http://ex/dir/a"), "base resolved the relative subject: {got}");
    }

    /// Invalid UTF-8 input fails CLOSED: an `Err` (JsError) across the boundary, never a
    /// panic / abort — the fail-closed surface JS `try { … } catch` relies on. `0xFF` is
    /// never a valid UTF-8 byte, so these inputs cannot be a real document.
    #[wasm_bindgen_test]
    fn load_bytes_invalid_utf8_fails_closed() {
        // A lone 0xFF, and a valid-then-invalid sequence — both are rejected.
        assert!(
            Store::load_bytes(&[0xFF, 0xFE], "turtle").is_err(),
            "lone 0xFF is not valid UTF-8 -> Err, not a trap"
        );
        assert!(
            Store::load_bytes(b"<http://ex/s> <http://ex/p> \xFF .", "ntriples").is_err(),
            "an invalid byte mid-document -> Err, not a trap"
        );
        // The with-base variant fails closed on invalid UTF-8 too.
        assert!(
            Store::load_bytes_with_base(&[0xC0], "turtle", "http://ex/").is_err(),
            "0xC0 (invalid UTF-8 lead) -> Err via loadBytesWithBase"
        );
    }
}

// ---- [SONNET-4.6] sq-q4apb (#2644): the opt-in hosted-web forms bridge, in real wasm ----
//
// Drives the REAL exported `Store::deriveForm(data, shapes, focus, format, optionsJson)`
// in a genuine wasm32 runtime — the surface `gui/app`'s `forms-bridge.ts` feature-detects
// on the hosted `/app` bundle. The load-bearing assertion is the SAME one the native
// `src/forms.rs` tests make, but through the wasm binding: the returned string must be
// byte-identical to serialising `sparq_forms::derive_form`'s `FormDescription` directly,
// so the bridge is provably a pass-through and never a reconstruction. Also covers the
// `JsError` Err arm, which the native tests cannot reach (`JsError::new` panics off-wasm).
#[cfg(feature = "forms")]
mod forms {
    use super::*;

    // The same fixtures the native src/forms.rs tests and the desktop Tauri bridge use,
    // so all three hosts stay pinned to one contract.
    const FORM_DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        ex:alice a ex:Person ; ex:name "Alice" ; ex:audit "reviewed" .
    "#;

    const FORM_SHAPES: &str = r#"
        @prefix ex: <http://example.org/> .
        @prefix sh: <http://www.w3.org/ns/shacl#> .

        ex:PersonShape a sh:NodeShape ;
            sh:targetClass ex:Person ;
            sh:name "Person" ;
            sh:property [ sh:path ex:name ; sh:name "Name" ; sh:minCount 1 ] .

        ex:AuditShape a sh:NodeShape ;
            sh:name "Audit" ;
            sh:property [ sh:path ex:audit ; sh:name "Audit status" ] .
    "#;

    /// The FormDescription JSON serialised straight from the `sparq-forms` API — the
    /// document the wasm binding must return verbatim.
    fn direct_description(mode: sparq_forms::Mode, shape: Option<&str>) -> String {
        let data = sparq_core::Graph::load_dataset(FORM_DATA, "turtle").expect("data fixture");
        let shapes = sparq_core::Graph::load_dataset(FORM_SHAPES, "turtle").expect("shapes fixture");
        let focus = oxrdf::Term::from(oxrdf::NamedNode::new_unchecked("http://example.org/alice"));
        let options = sparq_forms::FormOptions {
            mode,
            shape: shape.map(|iri| oxrdf::Term::from(oxrdf::NamedNode::new_unchecked(iri))),
            ..sparq_forms::FormOptions::default()
        };
        serde_json::to_string(&sparq_forms::derive_form(&data, &shapes, &focus, &options))
            .expect("direct FormDescription serializes")
    }

    /// Applicable-shape derivation in BOTH modes crosses the wasm boundary byte-identical
    /// to the direct `sparq_forms` serialization, with the mode-dependent editability
    /// carried through.
    #[wasm_bindgen_test]
    fn derive_form_edit_and_view_match_direct_serialization() {
        let store = Store::load("", "turtle").unwrap();
        for (mode_text, mode) in [
            ("edit", sparq_forms::Mode::Edit),
            ("view", sparq_forms::Mode::View),
        ] {
            let json = store
                .derive_form(
                    FORM_DATA,
                    FORM_SHAPES,
                    "http://example.org/alice",
                    "turtle",
                    &format!("{{\"mode\":\"{}\"}}", mode_text),
                )
                .expect("applicable shape derives across the boundary");
            assert_eq!(json, direct_description(mode, None), "{}", mode_text);

            let parsed: serde_json::Value = serde_json::from_str(&json).expect("bridge JSON");
            assert_eq!(parsed["mode"], mode_text);
            assert_eq!(parsed["shape"]["value"], "http://example.org/PersonShape");
            assert_eq!(parsed["groups"][0]["fields"][0]["label"], "Name");
            assert_eq!(
                parsed["groups"][0]["fields"][0]["editable"],
                mode_text == "edit"
            );
        }
    }

    /// An explicit `shape` option pins the derivation to that node shape (which need not
    /// target the focus node) — again byte-identical to the direct API call.
    #[wasm_bindgen_test]
    fn derive_form_explicit_shape_matches_direct_serialization() {
        const AUDIT_SHAPE: &str = "http://example.org/AuditShape";
        let store = Store::load("", "turtle").unwrap();
        let json = store
            .derive_form(
                FORM_DATA,
                FORM_SHAPES,
                "http://example.org/alice",
                "turtle",
                &format!("{{\"mode\":\"edit\",\"shape\":\"{}\"}}", AUDIT_SHAPE),
            )
            .expect("explicit shape derives across the boundary");
        assert_eq!(
            json,
            direct_description(sparq_forms::Mode::Edit, Some(AUDIT_SHAPE))
        );

        let parsed: serde_json::Value = serde_json::from_str(&json).expect("bridge JSON");
        assert_eq!(parsed["shape"]["value"], AUDIT_SHAPE);
        assert_eq!(parsed["shapes"][0]["via"], "explicit");
        assert_eq!(parsed["groups"][0]["fields"][0]["label"], "Audit status");
    }

    /// The workbench actually sends N-Quads snapshots, so the same graphs in that syntax
    /// must derive the same description (both snapshots share one `format`; the parse is
    /// dataset-style, so named graphs survive).
    #[wasm_bindgen_test]
    fn derive_form_accepts_the_nquads_snapshots_the_workbench_sends() {
        let store = Store::load("", "turtle").unwrap();
        let data_nq = "<http://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .\n\
                       <http://example.org/alice> <http://example.org/name> \"Alice\" .\n";
        let shapes_nq = "<http://example.org/PersonShape> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/shacl#NodeShape> .\n\
                         <http://example.org/PersonShape> <http://www.w3.org/ns/shacl#targetClass> <http://example.org/Person> .\n\
                         <http://example.org/PersonShape> <http://www.w3.org/ns/shacl#property> _:p1 .\n\
                         _:p1 <http://www.w3.org/ns/shacl#path> <http://example.org/name> .\n\
                         _:p1 <http://www.w3.org/ns/shacl#name> \"Name\" .\n";
        let json = store
            .derive_form(
                data_nq,
                shapes_nq,
                "http://example.org/alice",
                "nquads",
                "{\"mode\":\"edit\"}",
            )
            .expect("nquads snapshots derive");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("bridge JSON");
        assert_eq!(parsed["shape"]["value"], "http://example.org/PersonShape");
        assert_eq!(parsed["groups"][0]["fields"][0]["label"], "Name");
    }

    /// Malformed inputs surface as the `JsError` Err arm across the boundary — the
    /// `try { … } catch` surface `forms-bridge.ts` relies on — never a trap, and never a
    /// silently-defaulted derivation. (This arm constructs `JsError`, so only wasm can
    /// exercise it.)
    #[wasm_bindgen_test]
    fn derive_form_malformed_inputs_are_err_not_trap() {
        let store = Store::load("", "turtle").unwrap();
        let derive = |data: &str, shapes: &str, focus: &str, options: &str| {
            store.derive_form(data, shapes, focus, "turtle", options)
        };
        let alice = "http://example.org/alice";
        assert!(
            derive(FORM_DATA, FORM_SHAPES, alice, "not json").is_err(),
            "unparseable options document"
        );
        assert!(
            derive(FORM_DATA, FORM_SHAPES, alice, "[]").is_err(),
            "options must be a JSON object"
        );
        assert!(
            derive(FORM_DATA, FORM_SHAPES, alice, "{\"mode\":\"delete\"}").is_err(),
            "unsupported mode"
        );
        assert!(
            derive(FORM_DATA, FORM_SHAPES, alice, "{\"shape\":3}").is_err(),
            "non-string shape option"
        );
        assert!(
            derive(FORM_DATA, FORM_SHAPES, "not an iri", "{}").is_err(),
            "unparseable focus node"
        );
        assert!(
            derive("@prefix broken", FORM_SHAPES, alice, "{}").is_err(),
            "unparseable data graph"
        );
        assert!(
            derive(FORM_DATA, "@prefix broken", alice, "{}").is_err(),
            "unparseable shapes graph"
        );
    }
}

// ---- [SONNET-4.6] sq-yz27r (#3251): remote-`@context` JSON-LD ingest, in real wasm ----
//
// These drive the REAL exported `Store::loadJsonLdWithContexts(text, contexts)` across the
// JS boundary — specifically the two halves the native `src/jsonld_context.rs` tests cannot
// reach, because both are wasm-bindgen imports that panic off-wasm: reading the JS
// `[[url, documentText], …]` `js_sys::Array` argument, and the `JsError` Err arm. The
// context-resolution semantics themselves are pinned natively; this proves the binding
// actually exports to, and runs in, wasm. Compiled only under `jsonld-contexts`, as the
// binding is.
#[cfg(feature = "jsonld-contexts")]
mod jsonld_contexts {
    use super::*;
    use wasm_bindgen::JsValue;

    const VC_CONTEXT_URL: &str = "https://www.w3.org/2018/credentials/v1";
    const VC_CONTEXT: &str = r#"{"@context":{
        "id": "@id",
        "type": "@type",
        "VerifiableCredential": "https://www.w3.org/2018/credentials#VerifiableCredential",
        "credentialSubject": {
            "@id": "https://www.w3.org/2018/credentials#credentialSubject",
            "@type": "@id"
        }
    }}"#;
    const VC: &str = r#"{
        "@context": "https://www.w3.org/2018/credentials/v1",
        "id": "http://example.org/creds/1",
        "type": "VerifiableCredential",
        "credentialSubject": {"id": "http://example.org/subject/alice"}
    }"#;

    /// Builds the `[[url, documentText], …]` JS array the binding takes.
    fn pairs(entries: &[(&str, &str)]) -> js_sys::Array {
        let out = js_sys::Array::new();
        for (url, document) in entries {
            let pair = js_sys::Array::new();
            pair.push(&JsValue::from_str(url));
            pair.push(&JsValue::from_str(document));
            out.push(&pair);
        }
        out
    }

    /// The bead's motivating case, end to end in wasm: a credential naming its `@context`
    /// only by URL loads once the host hands the fetched context across the boundary, and
    /// the terms it defines are queryable.
    #[wasm_bindgen_test]
    fn vc_with_url_context_loads_and_queries() {
        let store =
            Store::load_jsonld_with_contexts(VC, pairs(&[(VC_CONTEXT_URL, VC_CONTEXT)])).unwrap();
        assert_eq!(store.size(), 2, "rdf:type + credentialSubject");
        assert!(
            store
                .ask("ASK { <http://example.org/creds/1> a <https://www.w3.org/2018/credentials#VerifiableCredential> }")
                .unwrap(),
            "the `type` term resolved through the host-supplied remote context",
        );
    }

    /// Fail-closed across the boundary: an unsupplied `@context` URL surfaces as the
    /// `JsError` Err arm (a catchable JS throw), not a wasm trap and not a silent
    /// partial parse.
    #[wasm_bindgen_test]
    fn unsupplied_context_is_the_err_arm_not_a_trap() {
        assert!(
            Store::load_jsonld_with_contexts(VC, js_sys::Array::new()).is_err(),
            "an unsupplied remote @context must fail closed",
        );
    }

    /// A malformed `contexts` element (not a 2-string array) is rejected rather than
    /// skipped — the `js_sys` argument-reading path, unreachable from the native tests.
    #[wasm_bindgen_test]
    fn malformed_contexts_argument_is_rejected() {
        let bad = js_sys::Array::new();
        bad.push(&JsValue::from_str("not a pair"));
        assert!(
            Store::load_jsonld_with_contexts(VC, bad).is_err(),
            "a non-array contexts element must throw",
        );

        let short = js_sys::Array::new();
        let pair = js_sys::Array::new();
        pair.push(&JsValue::from_str(VC_CONTEXT_URL));
        short.push(&pair);
        assert!(
            Store::load_jsonld_with_contexts(VC, short).is_err(),
            "a pair missing its document text must throw",
        );
    }

    /// An inline `@context` still loads with an EMPTY contexts array, so the binding is a
    /// superset of `load(_, "jsonld")` rather than a separate, stricter path.
    #[wasm_bindgen_test]
    fn inline_context_loads_with_no_supplied_documents() {
        let doc = r#"{"@context":{"name":"http://schema.org/name"},
                      "@id":"http://example.org/alice","name":"Alice"}"#;
        let store = Store::load_jsonld_with_contexts(doc, js_sys::Array::new()).unwrap();
        assert_eq!(store.size(), 1);
    }
}

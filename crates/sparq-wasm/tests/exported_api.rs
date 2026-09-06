// [OPUS-4.8] sq-8r9a — native behavioural tests of the REAL exported `#[wasm_bindgen]`
// `Store` API (and the `SolutionCursor` / `QueryChunks` / `QuadChunks` cursor types).
//
// WHY THIS FILE EXISTS (coverage floor-raise)
// -------------------------------------------
// The crate's `#[cfg(test)] mod tests` in src/lib.rs deliberately exercises the engine
// functions the wasm wrappers *delegate to* (`sparq_engine::query` / `ask` / `explain` /
// …) rather than the exported `Store::*` methods, because the wrappers map errors with
// `JsError::new`, a wasm-bindgen import that PANICS on a non-wasm target. That left every
// exported `Store::*` method's `Ok` (success) arm at 0% native line coverage — the JS
// boundary surface was only ever exercised by `tests/web.rs`, which is `wasm32`-only and
// so contributes nothing to the native `cargo llvm-cov` measurement the coverage ratchet
// gates on.
//
// The key fact this file leans on: only the **`Err` arm** of each wrapper touches
// `JsError::new`. The **success arm never does** — so calling `Store::load(good).unwrap()`,
// `store.query(valid_select)`, `cursor.next()`, … runs natively, returns the real value
// crossing-the-boundary would carry, and IS counted by llvm-cov. These are therefore REAL
// behaviour-asserting tests (each checks the actual SPARQL-JSON / N-Triples / boolean /
// count result content), not vacuous line-touchers: they assert the exported method
// produces the same observable result as the engine path the existing tests pin, through
// the actual published JS binding. The negative (`Err`) arms stay covered by the wasm32
// `tests/web.rs` (where `JsError::new` is real).

use sparq_wasm::Store;

const DATA: &str = r#"@prefix ex: <http://ex/> .
    ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
    ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

/// Extracts the `"value"` strings of every `?<var>` binding from a SPARQL-JSON batch, in
/// document order. [OPUS-4.8]
///
/// We anchor on the binding KEY (`"<var>":{ … "value":"<v>"`) rather than scanning every
/// `"value":"` in the batch, so the extraction stays correct if a row gains a second
/// projected variable (whose own `"value"` would otherwise be conflated). We deliberately
/// do NOT pull in `serde_json` to parse here: this crate's SPARQL-JSON is produced by its
/// own hand-rolled, dependency-free serializer with a fixed key order (see
/// `crates/sparq-wasm/src/serialize.rs` — "**dependency-free** … no `serde_json`"), and a
/// dev-only JSON dep purely for this assertion would be disproportionate. The key-anchored
/// scan below is robust against the only realistic fragility (a stray `"value":"` from a
/// different binding). (Addresses Copilot review on PR #1239.)
fn select_binding_values(batch: &str, var: &str) -> Vec<String> {
    let key = format!("\"{var}\":{{");
    let needle = "\"value\":\"";
    batch
        .match_indices(&key)
        .filter_map(|(start, _)| {
            // The binding object for `?var` ends at its closing brace; scan only within it.
            let obj = &batch[start + key.len()..];
            let end = obj.find('}').unwrap_or(obj.len());
            let obj = &obj[..end];
            obj.find(needle).map(|vi| {
                let rest = &obj[vi + needle.len()..];
                rest[..rest.find('"').unwrap_or(rest.len())].to_string()
            })
        })
        .collect()
}

/// Normalises an N-Triples document to its SORTED set of non-empty lines. [OPUS-4.8]
///
/// A CONSTRUCT/DESCRIBE graph has SET semantics: the document is the *set* of its triples,
/// and the engine makes no ordering guarantee on triple emission (see
/// `sparq_engine::construct::instantiate` — "first-production order", driven by the
/// `eval_select` row order). Comparing two SEPARATE query executions byte-for-byte would
/// therefore over-specify the contract and could flake if emission order ever changed.
/// Normalising to sorted lines pins the load-bearing invariant — both paths yield the SAME
/// set of triples — while leaving the chunk-count / chunk-size assertions to enforce the
/// partitioning. (Addresses Copilot review on PR #1239.)
fn sorted_nt_lines(nt: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = nt.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.sort_unstable();
    lines
}

// ---- load / size / heap_bytes ----------------------------------------------

/// The exported `Store::load` parses Turtle and the getters report the deduplicated
/// triple count + a non-zero footprint estimate — the same two getters JS reads.
#[test]
fn exported_load_size_heap() {
    let store = Store::load(DATA, "turtle").unwrap();
    // 2 names + 2 ages + 1 knows = 5 triples.
    assert_eq!(store.size(), 5);
    assert!(store.heap_bytes() > 0, "heap estimate must be non-zero");
}

/// N-Triples loads through the same exported entry point.
#[test]
fn exported_load_ntriples() {
    let nt = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
              <http://ex/a> <http://ex/p> <http://ex/c> .\n";
    let store = Store::load(nt, "ntriples").unwrap();
    assert_eq!(store.size(), 2);
}

/// `Store::loadDataset` preserves a NAMED graph (N-Quads), so a `GRAPH ?g` query over
/// the exported store returns the graph IRI — the dataset-preserving load path.
#[test]
fn exported_load_dataset_named_graph() {
    let nq = "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
    let store = Store::load_dataset(nq, "nquads").unwrap();
    let json = store
        .query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
        .unwrap();
    assert!(
        json.contains("\"value\":\"http://ex/g1\""),
        "named graph must be queryable through the exported loadDataset: {json}"
    );
}

/// `Store::loadCompressed` returns byte-identical SELECT JSON to the raw exported store
/// and reports a footprint no larger than the raw store.
#[test]
fn exported_load_compressed_matches_raw() {
    let raw = Store::load(DATA, "turtle").unwrap();
    let cmp = Store::load_compressed(DATA, "turtle").unwrap();
    assert_eq!(raw.size(), cmp.size());
    let q =
        "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a";
    assert_eq!(
        raw.query(q).unwrap(),
        cmp.query(q).unwrap(),
        "compressed exported store must return identical JSON"
    );
    assert!(cmp.heap_bytes() <= raw.heap_bytes());
}

/// [OPUS-4.8] sq-ty78o (#1114): the exported `Store::new()` (`new Store()`) builds an empty,
/// mutable store, and a `GRAPH`-targeted `updateInPlace` then a `GRAPH ?g` query round-trips
/// — the named-graph gap closed by the constructor. Exercises the `new`/`update_in_place`
/// success arms natively (the coverage floor for the exported method).
#[test]
fn exported_new_store_named_graph_roundtrip() {
    let mut store = Store::new().unwrap();
    assert_eq!(store.size(), 0, "a new Store() is empty");
    store
        .update_in_place(
            "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
        )
        .unwrap();
    let json = store
        .query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
        .unwrap();
    assert!(
        json.contains("\"value\":\"http://ex/g\""),
        "named graph round-trips through the exported new Store(): {json}"
    );
}

/// [OPUS-4.8] sq-f66jz (#1115): the exported `Store::loadWithBase` resolves relative IRIs
/// against the base (the `Ok` arm runs natively).
#[test]
fn exported_load_with_base_resolves_relative() {
    let store = Store::load_with_base("<a> <p> <../up/o> .", "turtle", "http://ex/dir/").unwrap();
    assert_eq!(store.size(), 1);
    let json = store.query("SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(
        json.contains("\"value\":\"http://ex/dir/a\""),
        "relative subject resolved against base: {json}"
    );
    assert!(
        json.contains("\"value\":\"http://ex/up/o\""),
        "relative object resolved (..) against base: {json}"
    );
}

// ---- query -> SPARQL 1.1 JSON ----------------------------------------------

/// The exported `Store::query` returns a well-formed SPARQL-1.1-JSON string: head vars in
/// order, a typed integer literal, a language tag, a plain string literal without a
/// datatype, and exactly two rows.
#[test]
fn exported_query_sparql_json_shape() {
    let store = Store::load(DATA, "turtle").unwrap();
    let json = store
        .query(
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a",
        )
        .unwrap();
    assert!(json.contains("\"vars\":[\"n\",\"a\"]"), "head vars: {json}");
    assert!(
        json.contains("\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\""),
        "integer datatype: {json}"
    );
    assert!(json.contains("\"xml:lang\":\"en\""), "lang tag: {json}");
    assert!(
        json.contains("\"value\":\"Alice\"}"),
        "plain literal: {json}"
    );
    assert_eq!(json.matches("\"a\":{").count(), 2, "two rows: {json}");
}

/// A URI-valued cell serialises as a `uri` term with its absolute IRI through the export.
#[test]
fn exported_query_uri_term() {
    let store = Store::load(DATA, "turtle").unwrap();
    let json = store
        .query("PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:knows ?o }")
        .unwrap();
    assert!(
        json.contains("\"type\":\"uri\",\"value\":\"http://ex/bob\""),
        "uri term: {json}"
    );
}

// ---- queryChunks / QueryChunks::next ---------------------------------------

/// `Store::queryChunks` streams the JSON bytes in row-boundary chunks; concatenating
/// every chunk (via the exported `QueryChunks::next`) reproduces `query`'s string exactly.
#[test]
fn exported_query_chunks_reassemble() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?n WHERE { ?s ex:name ?n } ORDER BY ?n";
    let whole = store.query(q).unwrap();

    let mut chunks = store.query_chunks(q).unwrap();
    let mut reassembled = String::new();
    let mut n = 0;
    while let Some(c) = chunks.next() {
        n += 1;
        reassembled.push_str(&c);
    }
    assert!(n >= 1, "at least one chunk");
    assert_eq!(
        reassembled, whole,
        "concatenated chunks must equal the one-shot JSON"
    );
}

// ---- queryCursor / SolutionCursor ------------------------------------------

/// `Store::queryCursor` yields self-contained SPARQL-JSON batches; with batch size 1 over
/// a 2-row result it produces two non-empty batches then exhausts, and the exported
/// `SolutionCursor` getters (`vars`, `rowCount`, `batchSize`) report the right values.
#[test]
fn exported_query_cursor_batches() {
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

/// A zero batch size is clamped to 1 by the exported `queryCursor`, and an oversized batch
/// yields everything in one batch.
#[test]
fn exported_query_cursor_clamp_and_oversize() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }";

    let zero = store.query_cursor(q, 0).unwrap();
    assert_eq!(zero.batch_size(), 1, "batch size clamps to >= 1");

    let mut big = store.query_cursor(q, 1000).unwrap();
    let only = big.next().expect("single batch");
    assert!(big.next().is_none());
    assert_eq!(
        only.matches("\"n\":{").count(),
        2,
        "oversized batch holds all rows: {only}"
    );
}

/// An empty result through the exported cursor yields exactly one empty batch, then
/// terminates — so JS can still read `head.vars` and distinguish "no rows" from "drained".
#[test]
fn exported_query_cursor_empty() {
    let store = Store::load(DATA, "turtle").unwrap();
    let mut cur = store
        .query_cursor(
            "PREFIX ex: <http://ex/> SELECT ?x WHERE { ?s ex:nope ?x }",
            8,
        )
        .unwrap();
    assert_eq!(cur.row_count(), 0);
    let only = cur.next().expect("one empty batch even with zero rows");
    assert!(
        only.contains("\"bindings\":[]"),
        "empty result batch must have empty bindings: {only}"
    );
    assert!(
        cur.next().is_none(),
        "exhausted after the single empty batch"
    );
}

/// [OPUS-4.8] sq-bif: cursor PARTITIONING with a `batch_size > 1` over MULTIPLE batches —
/// the case the existing tests never reach (they use only batch_size 1, where every step
/// advances by exactly one row, or an oversized single batch). A 5-row result with
/// batch_size 2 must split into batches of `[2, 2, 1]` (the trailing PARTIAL batch via
/// `min(pos + batch_size, total)`), the per-batch row counts must be exact, and the rows —
/// in `ORDER BY` order — must reassemble with NO drop, duplication, or reorder. This pins
/// the `end`/`pos` advancement against an off-by-one and drives the `pos == total`
/// exhaustion branch from a step larger than one (the existing batch_size-1 test reaches
/// it one row at a time; this reaches it after a partial trailing step).
#[test]
fn exported_query_cursor_partial_trailing_batch() {
    let data = "@prefix ex: <http://ex/> .\n\
        ex:a ex:n \"1\" . ex:b ex:n \"2\" . ex:c ex:n \"3\" . ex:d ex:n \"4\" . ex:e ex:n \"5\" .";
    let store = Store::load(data, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:n ?n } ORDER BY ?n";
    let mut cur = store.query_cursor(q, 2).unwrap();
    assert_eq!(cur.row_count(), 5);
    assert_eq!(cur.batch_size(), 2);

    let b0 = cur.next().expect("batch 0");
    let b1 = cur.next().expect("batch 1");
    let b2 = cur.next().expect("batch 2 (partial)");
    assert!(
        cur.next().is_none(),
        "exhausted after the trailing partial batch"
    );

    // Per-batch row counts: [2, 2, 1] — the partition is exact (the final batch is the
    // remainder, not a full batch_size, and never an empty extra batch).
    assert_eq!(
        b0.matches("\"n\":{").count(),
        2,
        "first full batch has 2 rows: {b0}"
    );
    assert_eq!(
        b1.matches("\"n\":{").count(),
        2,
        "second full batch has 2 rows: {b1}"
    );
    assert_eq!(
        b2.matches("\"n\":{").count(),
        1,
        "trailing batch has the 1 remainder: {b2}"
    );

    // The rows, in order across the batches, are exactly 1..=5 with no gap/dup/reorder.
    // Extract the ?n binding values (key-anchored, so a future second projected var would
    // not be conflated — see `select_binding_values`). [OPUS-4.8]
    let values: Vec<String> = [&b0, &b1, &b2]
        .iter()
        .flat_map(|b| select_binding_values(b, "n"))
        .collect();
    assert_eq!(
        values,
        ["1", "2", "3", "4", "5"],
        "ordered partition: {values:?}"
    );
}

/// [OPUS-4.8] sq-bif: cursor partitioning when `row_count` is an EXACT MULTIPLE of
/// `batch_size` (4 rows, batch_size 2 → two full batches, NO trailing empty batch). This is
/// the boundary the exhaustion guard `pos == total && total != 0` is written for: after the
/// second batch `pos` lands exactly on `total` from a step of 2, and the next call must
/// return `None` rather than emitting a spurious empty batch (the bug a naive `pos < total`
/// loop would have). Distinct from the empty-result case (which DOES emit one empty batch)
/// and from batch_size 1 (where `pos` reaches `total` one row at a time).
#[test]
fn exported_query_cursor_exact_multiple_no_trailing_empty() {
    let data = "@prefix ex: <http://ex/> .\n\
        ex:a ex:n \"1\" . ex:b ex:n \"2\" . ex:c ex:n \"3\" . ex:d ex:n \"4\" .";
    let store = Store::load(data, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:n ?n } ORDER BY ?n";
    let mut cur = store.query_cursor(q, 2).unwrap();
    assert_eq!(cur.row_count(), 4);

    let b0 = cur.next().expect("batch 0");
    let b1 = cur.next().expect("batch 1");
    // The KEY assertion: no third (empty) batch even though row_count is a multiple of the
    // batch size — the `pos == total` guard exits cleanly.
    assert!(
        cur.next().is_none(),
        "an exact-multiple result must NOT emit a trailing empty batch"
    );
    assert_eq!(b0.matches("\"n\":{").count(), 2, "{b0}");
    assert_eq!(b1.matches("\"n\":{").count(), 2, "{b1}");
    // A non-empty final batch (b1) — distinct from the empty-result single-empty-batch case.
    assert!(
        !b1.contains("\"bindings\":[]"),
        "final batch is non-empty: {b1}"
    );
}

// ---- queryQuads / queryQuadsChunks / QuadChunks ----------------------------

/// `Store::queryQuads` (CONSTRUCT) returns the constructed graph as N-Triples through the
/// export: two constructed triples, expanded predicate IRI, language tag preserved.
#[test]
fn exported_query_quads_construct() {
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

/// `Store::queryQuadsChunks` batches the constructed graph (exported `QuadChunks::next`);
/// concatenation reproduces `queryQuads` exactly.
#[test]
fn exported_query_quads_chunks_reassemble() {
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
    // SET-equal: both paths must yield the same triples (emission order is unspecified for
    // CONSTRUCT, so compare the sorted line sets rather than byte-for-byte). [OPUS-4.8]
    assert_eq!(
        sorted_nt_lines(&reassembled),
        sorted_nt_lines(&whole),
        "chunked N-Triples must reassemble to the whole document (as a set)"
    );
}

/// [OPUS-4.8] sq-bif: DESCRIBE through the exported `Store::queryQuads`. The existing quad
/// tests only exercise CONSTRUCT; DESCRIBE is the OTHER graph-valued form routed through the
/// same export, and it returns the Concise Bounded Description (the resource's OUTGOING
/// triples only). This asserts the CBD reaches the N-Triples document across the binding and
/// — the load-bearing CBD invariant — that an INBOUND subject is NOT pulled in (so the
/// binding can never accidentally return a CONSTRUCT-shaped whole-neighbourhood graph).
#[test]
fn exported_query_quads_describe_cbd() {
    let store = Store::load(DATA, "turtle").unwrap();
    let nt = store.query_quads("DESCRIBE <http://ex/bob>").unwrap();
    // ex:bob's CBD = its outgoing triples (name "Bob"@en + age 25), nothing inbound.
    assert!(
        nt.contains("<http://ex/bob> <http://ex/name> \"Bob\"@en ."),
        "outgoing name triple in CBD: {nt}"
    );
    assert!(
        nt.contains("<http://ex/bob> <http://ex/age>"),
        "outgoing age triple in CBD: {nt}"
    );
    // ex:alice ex:knows ex:bob is INBOUND to ex:bob — a CBD (outgoing triples only) must NOT
    // include it (that would be the bug where DESCRIBE leaked the whole neighbourhood). Pin
    // the SPECIFIC inbound triple's absence rather than any mention of ex:alice, so the test
    // would still pass if ex:alice legitimately appeared as the OBJECT of an outgoing triple
    // (it does not here, but the invariant is "no inbound edge", not "no alice"). [OPUS-4.8]
    assert!(
        !nt.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob>"),
        "CBD must not pull in the inbound knows triple: {nt}"
    );
}

/// [OPUS-4.8] sq-bif: `queryQuadsChunks` PARTITIONING at a NON-multiple boundary. The
/// existing test uses batch_size 1 over 2 triples (an even split); this constructs 5
/// triples and batches by 2, so the partition is `[2, 2, 1]` — a trailing PARTIAL chunk
/// (`triples.chunks(2)` over 5). It pins both the exact batch count (3) and that
/// concatenation still reproduces `queryQuads` byte-for-byte (no triple dropped at the
/// short tail), the regression a wrong `chunks()` size or a lost remainder would cause.
#[test]
fn exported_query_quads_chunks_partial_tail() {
    let data = "@prefix ex: <http://ex/> .\n\
        ex:a ex:n \"1\" . ex:b ex:n \"2\" . ex:c ex:n \"3\" . ex:d ex:n \"4\" . ex:e ex:n \"5\" .";
    let store = Store::load(data, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:n ?n }";
    let whole = store.query_quads(q).unwrap();
    assert_eq!(
        whole.lines().filter(|l| !l.trim().is_empty()).count(),
        5,
        "5 constructed triples: {whole}"
    );

    let mut chunks = store.query_quads_chunks(q, 2).unwrap();
    let mut reassembled = String::new();
    let mut sizes = Vec::new();
    while let Some(c) = chunks.next() {
        sizes.push(c.lines().filter(|l| !l.trim().is_empty()).count());
        reassembled.push_str(&c);
    }
    assert_eq!(
        sizes,
        [2, 2, 1],
        "5 triples, batch_size 2 => chunk sizes [2, 2, 1]"
    );
    // SET-equal: no triple dropped at the short tail (emission order is unspecified, so
    // compare the sorted line sets rather than byte-for-byte). [OPUS-4.8]
    assert_eq!(
        sorted_nt_lines(&reassembled),
        sorted_nt_lines(&whole),
        "the partial-tail chunked N-Triples must reassemble to the whole document (as a set)"
    );
}

// ---- count / ask / askWithMaxRows ------------------------------------------

/// The exported `Store::count` returns the solution count without materialising.
#[test]
fn exported_count() {
    let store = Store::load(DATA, "turtle").unwrap();
    let n = store
        .count("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:name ?n }")
        .unwrap();
    assert_eq!(n, 2);
}

/// The exported `Store::ask` answers a boolean: true when the pattern matches (incl. a
/// FILTER), false when it does not.
#[test]
fn exported_ask() {
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

/// The exported `Store::askWithMaxRows` answers under a generous cap (the portable,
/// wasm-safe working-set budget dimension).
#[test]
fn exported_ask_with_max_rows() {
    let store = Store::load(DATA, "turtle").unwrap();
    let q = "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o . ?s ex:age ?a FILTER(?a > 28) }";
    assert!(store.ask_with_max_rows(q, 1024).unwrap());
}

// ---- update / updateInPlace / applyDelta -----------------------------------

/// The exported `Store::update` returns a NEW store with the inserted data; the receiver
/// is unchanged (immutable rebuild semantics).
#[test]
fn exported_update_returns_new_store() {
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

/// The exported `Store::updateInPlace` mutates the receiver through the delta overlay.
#[test]
fn exported_update_in_place() {
    let mut store = Store::load(DATA, "turtle").unwrap();
    store
        .update_in_place("PREFIX ex: <http://ex/> INSERT DATA { ex:dave ex:age 40 }")
        .unwrap();
    assert_eq!(store.size(), 6);
    assert!(store
        .ask("PREFIX ex: <http://ex/> ASK { ex:dave ex:age 40 }")
        .unwrap());
}

/// The exported `Store::applyDelta` applies an N-Triples insert/delete batch in one shot.
#[test]
fn exported_apply_delta() {
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

// ---- explain / explainAnalyze ----------------------------------------------

/// The exported `Store::explain` returns the planning-only plan text (query-form header +
/// `Plan:` tree) without executing.
#[test]
fn exported_explain() {
    let store = Store::load(DATA, "turtle").unwrap();
    let plan = store
        .explain("PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a }")
        .unwrap();
    assert!(plan.contains("EXPLAIN (SELECT)"), "names the form: {plan}");
    assert!(plan.contains("Plan:"), "plan tree header: {plan}");
}

/// The exported `Store::explainAnalyze` returns the plan plus an execution trace for a
/// SELECT.
#[test]
fn exported_explain_analyze() {
    let store = Store::load(DATA, "turtle").unwrap();
    let r = store
        .explain_analyze("PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }")
        .unwrap();
    assert!(
        r.contains("EXPLAIN ANALYZE (SELECT)"),
        "names analyze + form: {r}"
    );
    assert!(r.contains("Plan:"), "analyze output includes the plan: {r}");
}

// ---- explain-json: the exported structured-plan success arms ----------------
//
// [FABLE-5] sq-ixc3.19. Only the OK arm runs natively (the `Err` arm constructs
// `JsError`); the negative arms stay covered by the wasm32 `tests/web.rs`.
#[cfg(feature = "explain-json")]
mod explain_json {
    use super::*;

    /// The exported `Store::explainPlanJson` returns the camelCase typed plan tree
    /// (the sq-jbqh4 schema contract) as a planning-only dry run.
    #[test]
    fn exported_explain_plan_json() {
        let store = Store::load(DATA, "turtle").unwrap();
        let json = store
            .explain_plan_json("PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }")
            .unwrap();
        assert!(json.contains("\"operator\":"), "schema keys: {json}");
        assert!(json.contains("\"children\":"), "schema keys: {json}");
        // Planning-only: nothing executed.
        assert!(json.contains("\"actual\":null"), "dry run: {json}");
        assert!(json.contains("\"qError\":null"), "dry run: {json}");
    }

    /// The exported `Store::explainPlanAnalyzeJson` executes and fills `actual` rows
    /// (2 `ex:name` matches in DATA) — the field the GUI's est-vs-actual view keys on.
    #[test]
    fn exported_explain_plan_analyze_json() {
        let store = Store::load(DATA, "turtle").unwrap();
        let json = store
            .explain_plan_analyze_json(
                "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }",
            )
            .unwrap();
        assert!(json.contains("\"actual\":2"), "executed actuals: {json}");
        assert!(!json.contains("\"nanos\":null"), "analyze fills nanos: {json}");
    }
}

// ---- shacl: the exported `Store::validate` success arm ----------------------
//
// Only the OK arm runs natively (the parse-error `Err` arm constructs `JsError`); the
// negative arm stays covered by the wasm32 `tests/web.rs::shacl::validate_parse_error_is_err`.
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

    /// The exported `Store::validate` returns the conforming report through the boundary.
    #[test]
    fn exported_validate_conforming() {
        let store = Store::load("", "turtle").unwrap();
        let data = r#"
            @prefix ex: <http://example.org/> .
            ex:alice a ex:Person ; ex:age 30 ; ex:name "Alice" .
        "#;
        let json = store.validate(data, SHAPES, "turtle").unwrap();
        assert_eq!(json, r#"{"conforms":true,"results":[]}"#, "{json}");
    }

    /// Violating data surfaces a parseable report with focusNode / path / message through
    /// the exported `Store::validate`.
    #[test]
    fn exported_validate_violating() {
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
            json.contains(r#""message":"age must be a non-negative integer""#),
            "declared message: {json}"
        );
        assert!(
            json.contains("MinCountConstraintComponent"),
            "minCount (missing name): {json}"
        );
    }
}

// ---- forms: the exported `Store::deriveForm` success arm ---------------------
//
// [SONNET-4.6] sq-q4apb (#2644): only the OK arm runs natively (every error path
// constructs `JsError`); the malformed-input `Err` arms stay covered by the wasm32
// `tests/web.rs::forms::derive_form_malformed_inputs_are_err_not_trap`. The assertion that
// matters is PASS-THROUGH: the exported binding's string must be byte-identical to
// serialising `sparq_forms::derive_form`'s `FormDescription` directly, so the hosted-web
// bridge provably never reconstructs the document.
#[cfg(feature = "forms")]
mod forms {
    use super::*;

    // The same fixtures the src/forms.rs unit tests, the wasm tests, and the desktop Tauri
    // bridge use, so all the hosts stay pinned to one contract.
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
    /// document the exported binding must return verbatim.
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

    /// The exported `Store::deriveForm` returns the applicable-shape derivation, in both
    /// modes, byte-identical to the direct `sparq_forms` serialization.
    #[test]
    fn exported_derive_form_edit_and_view_match_direct_serialization() {
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
                .expect("applicable shape derives");
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
    #[test]
    fn exported_derive_form_explicit_shape_matches_direct_serialization() {
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
            .expect("explicit shape derives");
        assert_eq!(
            json,
            direct_description(sparq_forms::Mode::Edit, Some(AUDIT_SHAPE))
        );

        let parsed: serde_json::Value = serde_json::from_str(&json).expect("bridge JSON");
        assert_eq!(parsed["shape"]["value"], AUDIT_SHAPE);
        assert_eq!(parsed["shapes"][0]["via"], "explicit");
        assert_eq!(parsed["groups"][0]["fields"][0]["label"], "Audit status");
    }

    /// The N-Quads snapshots the workbench actually sends derive the same description
    /// (both snapshots share one `format`; the parse is dataset-style).
    #[test]
    fn exported_derive_form_accepts_nquads_snapshots() {
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
}

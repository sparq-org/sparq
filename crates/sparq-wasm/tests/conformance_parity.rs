// sq-3dyje.11 (#2763) — wasm-parity: pinned SPARQL semantics sample,
// wasm32 ≡ committed native-verified results (research/testing-strategy-assessment-2026-07.md §7.6).
//
// The 7 existing wasm-bindgen test files pin the JS-boundary CONTRACTS (result
// serialisation shape, error arms, cursors); nothing compared query RESULTS wasm-vs-native.
// This file closes that gap with the cheapest honest check that the wasm32 build computes
// the same answers as the native engine:
//
//   1. A committed fixture sample — 46 SELECT + 4 ASK query+data cases, hand-pinned to span
//      the semantics-bearing categories of the W3C SPARQL 1.1 feature space: multi-pattern
//      joins, OPTIONAL (incl. nested + inner FILTER + !BOUND), UNION, ORDER BY over
//      mixed-type values (IRI / plain / lang-tagged / boolean / integer / decimal / double),
//      aggregates (COUNT/SUM/AVG/MIN/MAX/GROUP BY/HAVING), FILTER numerics (type promotion,
//      arithmetic, division-to-decimal, negatives, IN), plus DISTINCT / BIND / VALUES /
//      COALESCE / IF / MINUS / NOT EXISTS / subquery — each with its expected SPARQL-1.1-JSON
//      results document (or ASK boolean) committed verbatim.
//   2. A NATIVE #[test] asserting the engine (`sparq_engine::query_json` / `ask` — exactly
//      what `Store::query` / `Store::ask` delegate to) reproduces every committed
//      expectation. This keeps the fixtures honest against main: an engine change that
//      alters any pinned result turns this test red in ordinary `cargo test -p sparq-wasm`,
//      so the expectations cannot drift stale.
//   3. The IDENTICAL cases driven through the real exported `#[wasm_bindgen]` `Store` API,
//      natively (success arms only — the `tests/exported_api.rs` precedent: `JsError::new`
//      is touched only by the Err arm) AND as a `#[wasm_bindgen_test]` in a genuine wasm32
//      runtime under the existing `wasm-pack test --node` lane. Native-verified committed
//      bytes == wasm-computed bytes ⇒ the wasm32 build (which selects the 3-permutation
//      compact index and builds without rayon/threads) is byte-identical to native on the
//      pinned semantics sample.
//
// Determinism discipline: every multi-row SELECT carries an ORDER BY whose key set totally
// orders its rows (ties broken by a second key), the fixtures contain no blank nodes (labels
// are not stable across runs), GROUP_CONCAT is exercised over singleton groups only (SPARQL
// leaves within-group order unspecified), and no query needs the opt-in `regex` feature —
// so the expected documents are byte-stable across targets, index layouts, and feature
// states, and any mismatch is a real parity break, not fixture flakiness.

use sparq_wasm::Store;

// ---------------------------------------------------------------------------
// Committed fixtures: data + queries + native-verified expected results.
// ---------------------------------------------------------------------------

/// Which fixture document a case runs against.
#[derive(Clone, Copy)]
enum Data {
    /// People/knows/city graph: typed integers, decimals, doubles, a negative decimal, a
    /// lang-tagged and a non-ASCII plain literal, IRI-valued edges.
    People,
    /// One `ex:val` per subject with deliberately mixed term types (integer, plain string,
    /// numeric-looking string, decimal, double, IRI, lang-tagged, boolean) for mixed-type
    /// ORDER BY / DATATYPE / isIRI / isNumeric semantics.
    Mixed,
}

const DATA_PEOPLE: &str = r#"@prefix ex: <http://example.org/> .

ex:alice ex:name "Alice" ;
         ex:age 30 ;
         ex:height 1.68 ;
         ex:score 8.5E0 ;
         ex:knows ex:bob , ex:carol ;
         ex:city ex:london .
ex:bob   ex:name "Bob"@en ;
         ex:age 25 ;
         ex:height 1.8 ;
         ex:knows ex:carol ;
         ex:city ex:paris .
ex:carol ex:name "Carol" ;
         ex:age 41 ;
         ex:score 9.25E0 ;
         ex:knows ex:dave ;
         ex:city ex:london .
ex:dave  ex:name "Dave" ;
         ex:age 30 ;
         ex:balance -12.5 ;
         ex:city ex:paris .
ex:eve   ex:name "Ève" ;
         ex:age 17 .
"#;

const DATA_MIXED: &str = r#"@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:m1 ex:val 10 .
ex:m2 ex:val "10" .
ex:m3 ex:val "apple" .
ex:m4 ex:val 2.5 .
ex:m5 ex:val ex:anIri .
ex:m6 ex:val "zebra"@en .
ex:m7 ex:val true .
ex:m8 ex:val "banana" .
ex:m9 ex:val 9.5E0 .
"#;

/// Shared prologue prepended to every case query.
const PREFIXES: &str =
    "PREFIX ex: <http://example.org/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

struct Case {
    name: &'static str,
    data: Data,
    query: &'static str,
    /// The full SPARQL 1.1 JSON results document, produced and verified by the NATIVE
    /// engine (see `native::fixtures_match_native_engine`), committed verbatim.
    expected: &'static str,
}

struct AskCase {
    name: &'static str,
    query: &'static str,
    expected: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "join-two-patterns",
        data: Data::People,
        query: "SELECT ?n WHERE { ex:alice ex:knows ?x . ?x ex:name ?n } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "join-chain",
        data: Data::People,
        query: "SELECT ?a ?b ?c WHERE { ?a ex:knows ?b . ?b ex:knows ?c } ORDER BY ?a ?b ?c",
        expected: r##"{"head":{"vars":["a","b","c"]},"results":{"bindings":[{"a":{"type":"uri","value":"http://example.org/alice"},"b":{"type":"uri","value":"http://example.org/bob"},"c":{"type":"uri","value":"http://example.org/carol"}},{"a":{"type":"uri","value":"http://example.org/alice"},"b":{"type":"uri","value":"http://example.org/carol"},"c":{"type":"uri","value":"http://example.org/dave"}},{"a":{"type":"uri","value":"http://example.org/bob"},"b":{"type":"uri","value":"http://example.org/carol"},"c":{"type":"uri","value":"http://example.org/dave"}}]}}"##,
    },
    Case {
        name: "join-star",
        data: Data::People,
        query: "SELECT ?n ?a ?city WHERE { ?p ex:name ?n . ?p ex:age ?a . ?p ex:city ?city } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n","a","city"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"},"a":{"type":"literal","value":"30","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"city":{"type":"uri","value":"http://example.org/london"}},{"n":{"type":"literal","value":"Carol"},"a":{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"city":{"type":"uri","value":"http://example.org/london"}},{"n":{"type":"literal","value":"Dave"},"a":{"type":"literal","value":"30","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"city":{"type":"uri","value":"http://example.org/paris"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"},"a":{"type":"literal","value":"25","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"city":{"type":"uri","value":"http://example.org/paris"}}]}}"##,
    },
    Case {
        name: "join-same-city-pairs",
        data: Data::People,
        query: "SELECT ?x ?y WHERE { ?x ex:city ?c . ?y ex:city ?c . FILTER(?x != ?y) } ORDER BY ?x ?y",
        expected: r##"{"head":{"vars":["x","y"]},"results":{"bindings":[{"x":{"type":"uri","value":"http://example.org/alice"},"y":{"type":"uri","value":"http://example.org/carol"}},{"x":{"type":"uri","value":"http://example.org/bob"},"y":{"type":"uri","value":"http://example.org/dave"}},{"x":{"type":"uri","value":"http://example.org/carol"},"y":{"type":"uri","value":"http://example.org/alice"}},{"x":{"type":"uri","value":"http://example.org/dave"},"y":{"type":"uri","value":"http://example.org/bob"}}]}}"##,
    },
    Case {
        name: "join-empty",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . ?p ex:missing ?z }",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[]}}"##,
    },
    Case {
        name: "optional-basic",
        data: Data::People,
        query: "SELECT ?n ?s WHERE { ?p ex:name ?n . OPTIONAL { ?p ex:score ?s } } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n","s"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"},"s":{"type":"literal","value":"8.5E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"n":{"type":"literal","value":"Carol"},"s":{"type":"literal","value":"9.25E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Ève"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "optional-filter-inside",
        data: Data::People,
        query: "SELECT ?n ?s WHERE { ?p ex:name ?n . OPTIONAL { ?p ex:score ?s . FILTER(?s > 9) } } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n","s"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Carol"},"s":{"type":"literal","value":"9.25E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Ève"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "optional-unbound-filter",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . OPTIONAL { ?p ex:score ?s } FILTER(!BOUND(?s)) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Ève"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "optional-nested",
        data: Data::People,
        query: "SELECT ?n ?fn ?fs WHERE { ?p ex:name ?n . OPTIONAL { ?p ex:knows ?f . ?f ex:name ?fn . OPTIONAL { ?f ex:score ?fs } } } ORDER BY ?n ?fn",
        expected: r##"{"head":{"vars":["n","fn","fs"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"},"fn":{"type":"literal","value":"Carol"},"fs":{"type":"literal","value":"9.25E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"n":{"type":"literal","value":"Alice"},"fn":{"type":"literal","value":"Bob","xml:lang":"en"}},{"n":{"type":"literal","value":"Carol"},"fn":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Ève"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"},"fn":{"type":"literal","value":"Carol"},"fs":{"type":"literal","value":"9.25E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}}]}}"##,
    },
    Case {
        name: "union-mixed-value-types",
        data: Data::People,
        query: "SELECT ?v WHERE { { ex:alice ex:name ?v } UNION { ex:alice ex:age ?v } } ORDER BY ?v",
        expected: r##"{"head":{"vars":["v"]},"results":{"bindings":[{"v":{"type":"literal","value":"30","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"v":{"type":"literal","value":"Alice"}}]}}"##,
    },
    Case {
        name: "union-disjoint-vars",
        data: Data::People,
        query: "SELECT ?n ?a WHERE { { ?p ex:name ?n . ?p ex:score ?sc } UNION { ?p ex:age ?a . FILTER(?a > 35) } } ORDER BY ?n ?a",
        expected: r##"{"head":{"vars":["n","a"]},"results":{"bindings":[{"a":{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Carol"}}]}}"##,
    },
    Case {
        name: "union-filter-branches",
        data: Data::People,
        query: "SELECT ?p WHERE { { ?p ex:city ex:london } UNION { ?p ex:city ex:paris } } ORDER BY ?p",
        expected: r##"{"head":{"vars":["p"]},"results":{"bindings":[{"p":{"type":"uri","value":"http://example.org/alice"}},{"p":{"type":"uri","value":"http://example.org/bob"}},{"p":{"type":"uri","value":"http://example.org/carol"}},{"p":{"type":"uri","value":"http://example.org/dave"}}]}}"##,
    },
    Case {
        name: "order-mixed-types",
        data: Data::Mixed,
        query: "SELECT ?s ?val WHERE { ?s ex:val ?val } ORDER BY ?val ?s",
        expected: r##"{"head":{"vars":["s","val"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m5"},"val":{"type":"uri","value":"http://example.org/anIri"}},{"s":{"type":"uri","value":"http://example.org/m4"},"val":{"type":"literal","value":"2.5","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}},{"s":{"type":"uri","value":"http://example.org/m9"},"val":{"type":"literal","value":"9.5E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"s":{"type":"uri","value":"http://example.org/m1"},"val":{"type":"literal","value":"10","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"s":{"type":"uri","value":"http://example.org/m7"},"val":{"type":"literal","value":"true","datatype":"http://www.w3.org/2001/XMLSchema#boolean"}},{"s":{"type":"uri","value":"http://example.org/m2"},"val":{"type":"literal","value":"10"}},{"s":{"type":"uri","value":"http://example.org/m3"},"val":{"type":"literal","value":"apple"}},{"s":{"type":"uri","value":"http://example.org/m8"},"val":{"type":"literal","value":"banana"}},{"s":{"type":"uri","value":"http://example.org/m6"},"val":{"type":"literal","value":"zebra","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "order-mixed-types-desc",
        data: Data::Mixed,
        query: "SELECT ?s ?val WHERE { ?s ex:val ?val } ORDER BY DESC(?val) DESC(?s)",
        expected: r##"{"head":{"vars":["s","val"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m6"},"val":{"type":"literal","value":"zebra","xml:lang":"en"}},{"s":{"type":"uri","value":"http://example.org/m8"},"val":{"type":"literal","value":"banana"}},{"s":{"type":"uri","value":"http://example.org/m3"},"val":{"type":"literal","value":"apple"}},{"s":{"type":"uri","value":"http://example.org/m2"},"val":{"type":"literal","value":"10"}},{"s":{"type":"uri","value":"http://example.org/m7"},"val":{"type":"literal","value":"true","datatype":"http://www.w3.org/2001/XMLSchema#boolean"}},{"s":{"type":"uri","value":"http://example.org/m1"},"val":{"type":"literal","value":"10","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"s":{"type":"uri","value":"http://example.org/m9"},"val":{"type":"literal","value":"9.5E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"s":{"type":"uri","value":"http://example.org/m4"},"val":{"type":"literal","value":"2.5","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}},{"s":{"type":"uri","value":"http://example.org/m5"},"val":{"type":"uri","value":"http://example.org/anIri"}}]}}"##,
    },
    Case {
        name: "order-numeric-promotion",
        data: Data::Mixed,
        query: "SELECT ?s ?val WHERE { ?s ex:val ?val . FILTER(isNumeric(?val)) } ORDER BY ?val ?s",
        expected: r##"{"head":{"vars":["s","val"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m4"},"val":{"type":"literal","value":"2.5","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}},{"s":{"type":"uri","value":"http://example.org/m9"},"val":{"type":"literal","value":"9.5E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}},{"s":{"type":"uri","value":"http://example.org/m1"},"val":{"type":"literal","value":"10","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "order-two-keys-tie",
        data: Data::People,
        query: "SELECT ?n ?a WHERE { ?p ex:name ?n . ?p ex:age ?a } ORDER BY DESC(?a) ?n",
        expected: r##"{"head":{"vars":["n","a"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"},"a":{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Alice"},"a":{"type":"literal","value":"30","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Dave"},"a":{"type":"literal","value":"30","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"},"a":{"type":"literal","value":"25","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Ève"},"a":{"type":"literal","value":"17","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "order-by-expression",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . ?p ex:age ?a } ORDER BY (0 - ?a) ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}},{"n":{"type":"literal","value":"Ève"}}]}}"##,
    },
    Case {
        name: "order-limit-offset",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n } ORDER BY ?n LIMIT 2 OFFSET 1",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
    Case {
        name: "agg-count-star",
        data: Data::People,
        query: "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }",
        expected: r##"{"head":{"vars":["c"]},"results":{"bindings":[{"c":{"type":"literal","value":"23","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-count-var",
        data: Data::People,
        query: "SELECT (COUNT(?s) AS ?c) WHERE { ?p ex:score ?s }",
        expected: r##"{"head":{"vars":["c"]},"results":{"bindings":[{"c":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-count-distinct",
        data: Data::People,
        query: "SELECT (COUNT(DISTINCT ?c) AS ?n) WHERE { ?p ex:city ?c }",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-group-by",
        data: Data::People,
        query: "SELECT ?city (COUNT(?p) AS ?n) WHERE { ?p ex:city ?city } GROUP BY ?city ORDER BY ?city",
        expected: r##"{"head":{"vars":["city","n"]},"results":{"bindings":[{"city":{"type":"uri","value":"http://example.org/london"},"n":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"city":{"type":"uri","value":"http://example.org/paris"},"n":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-sum-avg-int",
        data: Data::People,
        query: "SELECT (SUM(?a) AS ?total) (AVG(?a) AS ?mean) WHERE { ?p ex:age ?a }",
        expected: r##"{"head":{"vars":["total","mean"]},"results":{"bindings":[{"total":{"type":"literal","value":"143","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"mean":{"type":"literal","value":"28.6","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}}]}}"##,
    },
    Case {
        name: "agg-avg-double",
        data: Data::People,
        query: "SELECT (AVG(?sc) AS ?m) WHERE { ?p ex:score ?sc }",
        expected: r##"{"head":{"vars":["m"]},"results":{"bindings":[{"m":{"type":"literal","value":"8.875E0","datatype":"http://www.w3.org/2001/XMLSchema#double"}}]}}"##,
    },
    Case {
        name: "agg-min-max-mixed-numeric",
        data: Data::Mixed,
        query: "SELECT (MIN(?v) AS ?lo) (MAX(?v) AS ?hi) WHERE { ?s ex:val ?v . FILTER(isNumeric(?v)) }",
        expected: r##"{"head":{"vars":["lo","hi"]},"results":{"bindings":[{"lo":{"type":"literal","value":"2.5","datatype":"http://www.w3.org/2001/XMLSchema#decimal"},"hi":{"type":"literal","value":"10","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-having",
        data: Data::People,
        query: "SELECT ?city (COUNT(?p) AS ?n) WHERE { ?p ex:city ?city } GROUP BY ?city HAVING(COUNT(?p) >= 2) ORDER BY ?city",
        expected: r##"{"head":{"vars":["city","n"]},"results":{"bindings":[{"city":{"type":"uri","value":"http://example.org/london"},"n":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"city":{"type":"uri","value":"http://example.org/paris"},"n":{"type":"literal","value":"2","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "agg-group-concat-singleton",
        data: Data::People,
        query: "SELECT ?p (GROUP_CONCAT(?n; separator=\",\") AS ?names) WHERE { ?p ex:name ?n } GROUP BY ?p ORDER BY ?p",
        expected: r##"{"head":{"vars":["p","names"]},"results":{"bindings":[{"p":{"type":"uri","value":"http://example.org/alice"},"names":{"type":"literal","value":"Alice"}},{"p":{"type":"uri","value":"http://example.org/bob"},"names":{"type":"literal","value":"Bob"}},{"p":{"type":"uri","value":"http://example.org/carol"},"names":{"type":"literal","value":"Carol"}},{"p":{"type":"uri","value":"http://example.org/dave"},"names":{"type":"literal","value":"Dave"}},{"p":{"type":"uri","value":"http://example.org/eve"},"names":{"type":"literal","value":"Ève"}}]}}"##,
    },
    Case {
        name: "filter-gt-int",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . ?p ex:age ?a . FILTER(?a > 28) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
    Case {
        name: "filter-decimal-ge",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:height ?h . ?p ex:name ?n . FILTER(?h >= 1.7) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "filter-cross-type-eq",
        data: Data::Mixed,
        query: "SELECT ?s WHERE { ?s ex:val ?v . FILTER(?v = 2.5e0) } ORDER BY ?s",
        expected: r##"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m4"}}]}}"##,
    },
    Case {
        name: "filter-arithmetic",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:age ?a . ?p ex:name ?n . FILTER(?a * 2 - 10 >= 50) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
    Case {
        name: "filter-negative-decimal",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:balance ?b . ?p ex:name ?n . FILTER(?b < 0) }",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
    Case {
        name: "filter-int-division",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:age ?a . ?p ex:name ?n . FILTER(?a / 2 = 15) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
    Case {
        name: "filter-in-list",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:age ?a . ?p ex:name ?n . FILTER(?a IN (25, 41)) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "filter-and-or",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:age ?a . ?p ex:name ?n . FILTER(?a > 20 && (?a < 30 || ?a = 41)) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "filter-str-functions",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . FILTER(STRSTARTS(STR(?n), \"C\") || CONTAINS(STR(?p), \"eve\")) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Ève"}}]}}"##,
    },
    Case {
        name: "filter-lang",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . FILTER(LANG(?n) = \"en\") }",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "filter-datatype",
        data: Data::Mixed,
        query: "SELECT ?s WHERE { ?s ex:val ?v . FILTER(DATATYPE(?v) = xsd:integer) } ORDER BY ?s",
        expected: r##"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m1"}}]}}"##,
    },
    Case {
        name: "filter-isiri",
        data: Data::Mixed,
        query: "SELECT ?s WHERE { ?s ex:val ?v . FILTER(isIRI(?v)) }",
        expected: r##"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://example.org/m5"}}]}}"##,
    },
    Case {
        name: "distinct",
        data: Data::People,
        query: "SELECT DISTINCT ?c WHERE { ?p ex:city ?c } ORDER BY ?c",
        expected: r##"{"head":{"vars":["c"]},"results":{"bindings":[{"c":{"type":"uri","value":"http://example.org/london"}},{"c":{"type":"uri","value":"http://example.org/paris"}}]}}"##,
    },
    Case {
        name: "bind-arith",
        data: Data::People,
        query: "SELECT ?n ?next WHERE { ?p ex:name ?n . ?p ex:age ?a . BIND(?a + 1 AS ?next) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n","next"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"},"next":{"type":"literal","value":"31","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Carol"},"next":{"type":"literal","value":"42","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Dave"},"next":{"type":"literal","value":"31","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Ève"},"next":{"type":"literal","value":"18","datatype":"http://www.w3.org/2001/XMLSchema#integer"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"},"next":{"type":"literal","value":"26","datatype":"http://www.w3.org/2001/XMLSchema#integer"}}]}}"##,
    },
    Case {
        name: "values",
        data: Data::People,
        query: "SELECT ?n WHERE { VALUES ?p { ex:alice ex:eve } ?p ex:name ?n } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Ève"}}]}}"##,
    },
    Case {
        name: "coalesce-if",
        data: Data::People,
        query: "SELECT ?n ?v ?k WHERE { ?p ex:name ?n . ?p ex:age ?a . OPTIONAL { ?p ex:score ?s } BIND(COALESCE(?s, 0) AS ?v) BIND(IF(?a >= 18, \"adult\", \"minor\") AS ?k) } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n","v","k"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"},"v":{"type":"literal","value":"8.5E0","datatype":"http://www.w3.org/2001/XMLSchema#double"},"k":{"type":"literal","value":"adult"}},{"n":{"type":"literal","value":"Carol"},"v":{"type":"literal","value":"9.25E0","datatype":"http://www.w3.org/2001/XMLSchema#double"},"k":{"type":"literal","value":"adult"}},{"n":{"type":"literal","value":"Dave"},"v":{"type":"literal","value":"0","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"k":{"type":"literal","value":"adult"}},{"n":{"type":"literal","value":"Ève"},"v":{"type":"literal","value":"0","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"k":{"type":"literal","value":"minor"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"},"v":{"type":"literal","value":"0","datatype":"http://www.w3.org/2001/XMLSchema#integer"},"k":{"type":"literal","value":"adult"}}]}}"##,
    },
    Case {
        name: "minus",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n MINUS { ?p ex:score ?s } } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Dave"}},{"n":{"type":"literal","value":"Ève"}},{"n":{"type":"literal","value":"Bob","xml:lang":"en"}}]}}"##,
    },
    Case {
        name: "filter-not-exists",
        data: Data::People,
        query: "SELECT ?n WHERE { ?p ex:name ?n . FILTER NOT EXISTS { ?x ex:knows ?p } } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Ève"}}]}}"##,
    },
    Case {
        name: "subquery",
        data: Data::People,
        query: "SELECT ?n WHERE { { SELECT ?p WHERE { ?p ex:age ?a . FILTER(?a > 28) } } ?p ex:name ?n } ORDER BY ?n",
        expected: r##"{"head":{"vars":["n"]},"results":{"bindings":[{"n":{"type":"literal","value":"Alice"}},{"n":{"type":"literal","value":"Carol"}},{"n":{"type":"literal","value":"Dave"}}]}}"##,
    },
];

/// ASK cases (run against the People fixture; boolean-valued, inherently order-free).
const ASK_CASES: &[AskCase] = &[
    AskCase { name: "ask-true-ground", query: "ASK { ex:alice ex:knows ex:bob }", expected: true },
    AskCase { name: "ask-false-ground", query: "ASK { ex:bob ex:knows ex:alice }", expected: false },
    AskCase { name: "ask-filter-true", query: "ASK { ?p ex:age ?a . FILTER(?a > 40) }", expected: true },
    AskCase { name: "ask-filter-false", query: "ASK { ?p ex:age ?a . FILTER(?a > 100) }", expected: false },
];

fn full_query(query: &str) -> String {
    let mut q = String::with_capacity(PREFIXES.len() + query.len());
    q.push_str(PREFIXES);
    q.push_str(query);
    q
}

// ---------------------------------------------------------------------------
// Shared runner: every case through the REAL exported `Store` API.
//
// One cfg-free body invoked by BOTH the native #[test] and the wasm32
// #[wasm_bindgen_test], so the two targets execute the byte-identical call path
// (Store::load → Store::query / Store::ask) and only the compilation target differs —
// which is exactly the parity claim under test.
// ---------------------------------------------------------------------------

fn assert_store_matches_fixtures() {
    let people = Store::load(DATA_PEOPLE, "turtle").expect("People fixture must load");
    let mixed = Store::load(DATA_MIXED, "turtle").expect("Mixed fixture must load");
    for case in CASES {
        let store = match case.data {
            Data::People => &people,
            Data::Mixed => &mixed,
        };
        let got = store
            .query(&full_query(case.query))
            .unwrap_or_else(|_| panic!("case {} must evaluate via Store::query", case.name));
        assert_eq!(
            got, case.expected,
            "wasm-parity case {} diverged from the committed native-verified result",
            case.name
        );
    }
    for ask in ASK_CASES {
        let got = people
            .ask(&full_query(ask.query))
            .unwrap_or_else(|_| panic!("ask case {} must evaluate via Store::ask", ask.name));
        assert_eq!(
            got, ask.expected,
            "wasm-parity ask case {} diverged from the committed native-verified boolean",
            ask.name
        );
    }
}

// ---------------------------------------------------------------------------
// Native lane (`cargo test -p sparq-wasm`): fixture honesty + the Store path.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use sparq_core::Graph;

    /// Fixture-honesty: the NATIVE engine reproduces every committed expected document.
    /// Runs on plain `cargo test`, so an engine behaviour change on main cannot leave the
    /// committed expectations stale — it turns this red in the same PR.
    #[test]
    fn fixtures_match_native_engine() {
        let people = Graph::load_str(DATA_PEOPLE, "turtle").expect("People fixture must load");
        let mixed = Graph::load_str(DATA_MIXED, "turtle").expect("Mixed fixture must load");
        for case in CASES {
            let graph = match case.data {
                Data::People => &people,
                Data::Mixed => &mixed,
            };
            let got = sparq_engine::query_json(graph, &full_query(case.query))
                .unwrap_or_else(|e| panic!("case {} must evaluate natively: {e}", case.name));
            assert_eq!(
                got, case.expected,
                "committed fixture {} is stale against the native engine",
                case.name
            );
        }
        for ask in ASK_CASES {
            let got = sparq_engine::ask(&people, &full_query(ask.query))
                .unwrap_or_else(|e| panic!("ask case {} must evaluate natively: {e}", ask.name));
            assert_eq!(
                got, ask.expected,
                "committed ask fixture {} is stale against the native engine",
                ask.name
            );
        }
    }

    /// The exported `Store` API (success arms run natively — `tests/exported_api.rs`
    /// precedent) reproduces the same committed results on the native target.
    #[test]
    fn store_api_matches_fixtures_native() {
        assert_store_matches_fixtures();
    }
}

// ---------------------------------------------------------------------------
// wasm32 lane (`wasm-pack test --node`): the parity assertion itself.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use wasm_bindgen_test::*;

    /// The identical pinned sample through the identical `Store` call path, executed in a
    /// genuine wasm32 runtime: results must be byte-identical to the committed
    /// native-verified documents.
    #[wasm_bindgen_test]
    fn wasm32_results_equal_native_verified_fixtures() {
        assert_store_matches_fixtures();
    }
}

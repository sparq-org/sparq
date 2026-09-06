//! [SONNET-4.6] sq-gcs5q — the **round-trip validation** half of survey §C3
//! ("expand → compact reconstructs the same RDF").
//!
//! The rest of §C3 — the `expand` and `flatten` conformance lanes plus their
//! `EXPAND_FLOOR` / `FLATTEN_FLOOR` ratchets — landed with the native
//! document-level algorithms (`sq-oy1f.25` / `.26` / `.37` / `.45`, `sq-kk1mq`)
//! and is gated in `sparq-conformance`. What was still missing is the *cross-lane*
//! invariant that ties `expand` to `compact`:
//!
//! > **Compaction is lossless.** For any document `D` and any context `C`,
//! > `expand(compact(D, C)) ≡ expand(D)` under JSON-LD data-model equality.
//!
//! This is the document-level form of "reconstructs the same RDF", and it is
//! strictly *stronger*: the RDF projection is a function of the expanded document,
//! so reproducing the expanded document reproduces the dataset — and it additionally
//! pins the JSON-LD-only structure that has **no** RDF projection at all (`@index`,
//! `@json` payload shape, empty-array properties, `@direction` under
//! `rdfDirection: none`), which an RDF-equivalence oracle is blind to. It is also
//! checkable offline: unlike the W3C lanes it needs no fetched fixtures, so it runs
//! on every `cargo test -p sparq-jsonld`.
//!
//! The companion statement for the flatten lane — `flatten(compact(F, C)) ≡ F` for
//! an already-flattened `F` — is asserted at the end. Flatten idempotence itself is
//! covered by `tests/flatten.rs::flatten_expanded_is_idempotent`.

use sparq_jsonld::compact::compact;
use sparq_jsonld::{expand, flatten, Json, JsonLdOptions, NoopLoader};

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

/// How the array immediately under comparison must be compared — the JSON-LD
/// data model gives arrays three different meanings depending on where they sit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arrays {
    /// The default: a property's value array is an unordered SET of values.
    Unordered,
    /// The array under a `@list` key: its order is significant, but its elements
    /// are ordinary JSON-LD values again (so nested arrays revert to `Unordered`).
    ListOrdered,
    /// Inside a `@json` literal's `@value` payload: plain JSON, so array order is
    /// significant RECURSIVELY (object member order stays insignificant).
    JsonExact,
}

/// Native JSON-LD document-level equality over the crate's `Json` AST: object key
/// order insignificant, array order significant inside a `@list` value and
/// (recursively) inside a `@json` literal payload, everything else exact (scalars
/// compared as their `Json::Raw` / `Json::Str` tokens). Same semantics as
/// `tests/flatten.rs` and the conformance crate's `json_ld_equal`, restated here
/// because Rust integration tests are separate binaries with no shared module.
///
/// The `@json` carve-out is load-bearing for this module's claim to pin `@json`
/// payload SHAPE: a `@json` value is opaque JSON, where `[1,2]` and `[2,1]` are
/// different documents, so comparing the payload with set semantics would let a
/// payload-reordering compaction bug pass `json_literal_round_trips`.
fn json_ld_equal(a: &Json, b: &Json) -> bool {
    json_ld_equal_inner(a, b, Arrays::Unordered)
}

fn json_ld_equal_inner(a: &Json, b: &Json, arrays: Arrays) -> bool {
    match (a, b) {
        (Json::Str(x), Json::Str(y)) => x == y,
        (Json::Raw(x), Json::Raw(y)) => x == y,
        (Json::Arr(xs), Json::Arr(ys)) => {
            if xs.len() != ys.len() {
                return false;
            }
            match arrays {
                Arrays::Unordered => array_equal_unordered(xs, ys),
                // A `@list`'s elements are JSON-LD values: back to set semantics.
                Arrays::ListOrdered => xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| json_ld_equal_inner(x, y, Arrays::Unordered)),
                // Inside a `@json` payload the exactness propagates all the way down.
                Arrays::JsonExact => xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| json_ld_equal_inner(x, y, Arrays::JsonExact)),
            }
        }
        (Json::Obj(xa), Json::Obj(ya)) => {
            if xa.len() != ya.len() {
                return false;
            }
            let json_literal = arrays != Arrays::JsonExact && is_json_literal(xa);
            xa.iter().all(|(k, va)| {
                let child = if arrays == Arrays::JsonExact {
                    // An opaque payload: `@list` / `@type` are ordinary member names here.
                    Arrays::JsonExact
                } else if json_literal && k == "@value" {
                    Arrays::JsonExact
                } else if k == "@list" {
                    Arrays::ListOrdered
                } else {
                    Arrays::Unordered
                };
                ya.iter()
                    .find(|(k2, _)| k2 == k)
                    .is_some_and(|(_, vb)| json_ld_equal_inner(va, vb, child))
            })
        }
        _ => false,
    }
}

/// Is this object an expanded `@json` value object (`{"@value":…,"@type":"@json"}`)?
/// `expand` emits the `@type` as a bare string; a kept single-element array is
/// accepted too so the check does not depend on that normalisation.
fn is_json_literal(members: &[(String, Json)]) -> bool {
    members.iter().any(|(k, v)| {
        k == "@type"
            && match v {
                Json::Str(s) => s == "@json",
                Json::Arr(a) => matches!(a.as_slice(), [Json::Str(s)] if s == "@json"),
                _ => false,
            }
    })
}

fn array_equal_unordered(xs: &[Json], ys: &[Json]) -> bool {
    let mut used = vec![false; ys.len()];
    'outer: for x in xs {
        for (j, y) in ys.iter().enumerate() {
            if !used[j] && json_ld_equal_inner(x, y, Arrays::Unordered) {
                used[j] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

fn render(v: &Json) -> String {
    let mut s = String::new();
    v.write(&mut s);
    s
}

// ---------------------------------------------------------------------------
// The round-trip driver
// ---------------------------------------------------------------------------

/// Assert `expand(compact(input, ctx)) ≡ expand(input)` under `opts`.
///
/// Both legs run through the REAL shipping pipeline (`compact()` expands its input
/// natively, so a break in either algorithm surfaces here), and both expansions use
/// the same options, so base resolution / relativisation must cancel exactly.
fn assert_lossless(case: &str, input: &str, ctx: &str, opts: &JsonLdOptions) {
    let input = Json::parse(input).expect("valid input JSON");
    let ctx = Json::parse(ctx).expect("valid context JSON");

    let expanded = expand(&input, opts, &NoopLoader).expect("expansion succeeds");
    let compacted = compact(&input, &ctx, opts, &NoopLoader).expect("compaction succeeds");
    let reexpanded = expand(&compacted, opts, &NoopLoader).expect("re-expansion succeeds");

    assert!(
        json_ld_equal(&reexpanded, &expanded),
        // [SONNET-4.6] positional format args (CodeQL rust/unused-variable false-positive guard).
        "{}: compaction lost information\n  expanded:   {}\n  compacted:  {}\n  re-expanded:{}",
        case,
        render(&expanded),
        render(&compacted),
        render(&reexpanded)
    );
}

/// The same invariant with the default options.
fn assert_lossless_default(case: &str, input: &str, ctx: &str) {
    assert_lossless(case, input, ctx, &JsonLdOptions::default());
}

// ---------------------------------------------------------------------------
// Term selection, IRI compaction, keyword aliasing
// ---------------------------------------------------------------------------

#[test]
fn terms_and_compact_iris_round_trip() {
    assert_lossless_default(
        "terms + compact IRIs",
        r#"{"@context":{"ex":"http://example.com/"},"@id":"ex:a","ex:name":"Alice","ex:knows":{"@id":"ex:b"}}"#,
        r#"{"ex":"http://example.com/","name":"http://example.com/name"}"#,
    );
}

#[test]
fn keyword_aliases_round_trip() {
    assert_lossless_default(
        "keyword aliases",
        r#"{"@context":{"ex":"http://example.com/"},"@id":"ex:a","@type":"ex:Person","ex:name":"Alice"}"#,
        r#"{"id":"@id","type":"@type","ex":"http://example.com/"}"#,
    );
}

#[test]
fn unmatched_iri_falls_back_and_still_round_trips() {
    // No term or prefix covers the predicate: compaction must keep enough to
    // re-expand (an absolute IRI key), not silently drop it.
    assert_lossless_default(
        "no covering term",
        r#"{"@id":"http://other.example/a","http://other.example/p":"v"}"#,
        r#"{"ex":"http://example.com/"}"#,
    );
}

// ---------------------------------------------------------------------------
// Value compaction: type coercion, language, direction, native scalars, @json
// ---------------------------------------------------------------------------

#[test]
fn id_and_vocab_coercions_round_trip() {
    assert_lossless_default(
        "@id/@vocab coercion",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:homepage":{"@id":"http://example.com/home"},
            "ex:kind":{"@id":"http://example.com/Kind"}
        }"#,
        r#"{
            "@vocab":"http://example.com/",
            "homepage":{"@id":"http://example.com/homepage","@type":"@id"},
            "kind":{"@id":"http://example.com/kind","@type":"@vocab"}
        }"#,
    );
}

#[test]
fn datatype_coercion_round_trips() {
    assert_lossless_default(
        "datatype coercion",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:when":{"@value":"2026-07-26","@type":"http://www.w3.org/2001/XMLSchema#date"},
            "ex:other":{"@value":"7","@type":"http://www.w3.org/2001/XMLSchema#integer"}
        }"#,
        r#"{
            "ex":"http://example.com/",
            "when":{"@id":"http://example.com/when","@type":"http://www.w3.org/2001/XMLSchema#date"}
        }"#,
    );
}

#[test]
fn language_and_direction_round_trip() {
    assert_lossless_default(
        "language + direction",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:name":[
                {"@value":"Alice","@language":"en"},
                {"@value":"أليس","@language":"ar","@direction":"rtl"},
                {"@value":"plain"}
            ]
        }"#,
        r#"{
            "ex":"http://example.com/",
            "@language":"en",
            "name":"http://example.com/name",
            "ar":{"@id":"http://example.com/name","@language":"ar","@direction":"rtl"}
        }"#,
    );
}

#[test]
fn native_scalars_round_trip() {
    assert_lossless_default(
        "native scalars",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:count":42,
            "ex:ratio":1.5,
            "ex:flag":true
        }"#,
        r#"{"ex":"http://example.com/"}"#,
    );
}

#[test]
fn json_literal_round_trips() {
    assert_lossless_default(
        "@json literal",
        r#"{
            "@context":{"ex":"http://example.com/","payload":{"@id":"http://example.com/payload","@type":"@json"}},
            "@id":"ex:a",
            "payload":{"b":[1,2,{"c":null}],"a":true}
        }"#,
        r#"{"ex":"http://example.com/","payload":{"@id":"http://example.com/payload","@type":"@json"}}"#,
    );
}

#[test]
fn json_payload_array_order_is_significant() {
    // Witness that `json_literal_round_trips` above is NOT vacuous for the shape
    // half of its claim. A `@json` payload is opaque JSON, where `[1,2]` and
    // `[2,1]` are different documents — but everything else in the expanded form
    // is compared with SET semantics, so without the `@json` carve-out the oracle
    // would accept a compaction bug that reordered the payload.
    let literal = |payload: &str| {
        Json::parse(&format!(
            r#"{{"@id":"http://example.com/a",
                 "http://example.com/payload":[{{"@value":{},"@type":"@json"}}]}}"#,
            payload
        ))
        .expect("valid expanded JSON")
    };

    assert!(
        !json_ld_equal(&literal("[1,2]"), &literal("[2,1]")),
        "@json payload array order must be significant"
    );
    // …recursively, not just at the payload's top level.
    assert!(
        !json_ld_equal(&literal(r#"{"k":[1,2]}"#), &literal(r#"{"k":[2,1]}"#)),
        "@json payload array order must be significant at any depth"
    );
    // Equal payloads still compare equal, and object member order inside the
    // payload stays insignificant (JSON objects are unordered maps).
    assert!(json_ld_equal(&literal("[1,2]"), &literal("[1,2]")));
    assert!(json_ld_equal(
        &literal(r#"{"a":1,"b":[1,2]}"#),
        &literal(r#"{"b":[1,2],"a":1}"#)
    ));

    // The carve-out must not leak: an ordinary (non-`@json`) property value array
    // is still an unordered set.
    let plain = |values: &str| {
        Json::parse(&format!(
            r#"{{"@id":"http://example.com/a","http://example.com/p":{}}}"#,
            values
        ))
        .expect("valid expanded JSON")
    };
    assert!(json_ld_equal(
        &plain(r#"[{"@value":"x"},{"@value":"y"}]"#),
        &plain(r#"[{"@value":"y"},{"@value":"x"}]"#)
    ));
}

// ---------------------------------------------------------------------------
// Containers — the reshaping whose inverse is the risky half
// ---------------------------------------------------------------------------

#[test]
fn list_container_round_trips_in_order() {
    // `@list` order is load-bearing and the comparator IS order-sensitive inside a
    // list, so a reordering regression fails here.
    assert_lossless_default(
        "@list",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:items":{"@list":["one","two","three"]}
        }"#,
        r#"{"ex":"http://example.com/","items":{"@id":"http://example.com/items","@container":"@list"}}"#,
    );
}

#[test]
fn set_container_and_empty_array_round_trip() {
    // An empty-array property has NO RDF projection — an RDF-equivalence oracle
    // cannot see it dropped. The document-level oracle can.
    assert_lossless_default(
        "@set + empty array",
        r#"{
            "@context":{"ex":"http://example.com/","set1":{"@id":"http://example.com/set1","@container":"@set"}},
            "@id":"ex:a",
            "set1":[],
            "ex:one":["only"]
        }"#,
        r#"{"ex":"http://example.com/","set1":{"@id":"http://example.com/set1","@container":"@set"}}"#,
    );
}

#[test]
fn language_map_round_trips() {
    assert_lossless_default(
        "@language map",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:name":[{"@value":"Alice","@language":"en"},{"@value":"Alizée","@language":"fr"}]
        }"#,
        r#"{"ex":"http://example.com/","name":{"@id":"http://example.com/name","@container":"@language"}}"#,
    );
}

#[test]
fn index_map_round_trips() {
    // `@index` has no RDF projection either: this case is invisible to an
    // RDF-equivalence oracle and is exactly why the document-level statement is
    // the stronger one.
    assert_lossless_default(
        "@index map",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:items":[
                {"@value":"first","@index":"a"},
                {"@value":"second","@index":"b"}
            ]
        }"#,
        r#"{"ex":"http://example.com/","items":{"@id":"http://example.com/items","@container":"@index"}}"#,
    );
}

#[test]
fn id_map_round_trips() {
    assert_lossless_default(
        "@id map",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:refs":[
                {"@id":"http://example.com/b","http://example.com/name":"B"},
                {"@id":"http://example.com/c","http://example.com/name":"C"}
            ]
        }"#,
        r#"{
            "ex":"http://example.com/",
            "name":"http://example.com/name",
            "refs":{"@id":"http://example.com/refs","@container":"@id"}
        }"#,
    );
}

#[test]
fn type_map_round_trips() {
    assert_lossless_default(
        "@type map",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:refs":[
                {"@id":"http://example.com/b","@type":"http://example.com/T1"},
                {"@id":"http://example.com/c","@type":"http://example.com/T2"}
            ]
        }"#,
        r#"{"ex":"http://example.com/","refs":{"@id":"http://example.com/refs","@container":"@type"}}"#,
    );
}

#[test]
fn graph_container_round_trips() {
    assert_lossless_default(
        "@graph container",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:claim":{"@graph":{"@id":"http://example.com/b","http://example.com/name":"B"}}
        }"#,
        r#"{"ex":"http://example.com/","claim":{"@id":"http://example.com/claim","@container":"@graph"}}"#,
    );
}

#[test]
fn named_graphs_round_trip() {
    assert_lossless_default(
        "named graphs",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:g",
            "@graph":[
                {"@id":"ex:a","ex:name":"A"},
                {"@id":"ex:b","ex:name":"B"}
            ]
        }"#,
        r#"{"ex":"http://example.com/","name":"http://example.com/name"}"#,
    );
}

#[test]
fn nest_grouping_round_trips() {
    assert_lossless_default(
        "@nest",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:name":"Alice",
            "ex:age":"42"
        }"#,
        r#"{
            "ex":"http://example.com/",
            "detail":"@nest",
            "name":{"@id":"http://example.com/name","@nest":"detail"},
            "age":{"@id":"http://example.com/age","@nest":"detail"}
        }"#,
    );
}

#[test]
fn reverse_properties_round_trip() {
    assert_lossless_default(
        "@reverse",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "@reverse":{"ex:parent":[{"@id":"ex:child"}]}
        }"#,
        r#"{"ex":"http://example.com/","children":{"@reverse":"http://example.com/parent"}}"#,
    );
}

// ---------------------------------------------------------------------------
// Document shape, blank nodes, options
// ---------------------------------------------------------------------------

#[test]
fn multi_node_document_round_trips_through_the_graph_wrap() {
    // A multi-node expanded array compacts to `{"@context":…,"@graph":[…]}`;
    // re-expanding must lift the `@graph` back to the bare array.
    assert_lossless_default(
        "multi-node → @graph wrap",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@graph":[
                {"@id":"ex:a","ex:name":"A"},
                {"@id":"ex:b","ex:name":"B"}
            ]
        }"#,
        r#"{"ex":"http://example.com/","name":"http://example.com/name"}"#,
    );
}

#[test]
fn blank_node_cross_references_round_trip() {
    assert_lossless_default(
        "blank nodes",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"_:left",
            "ex:link":{"@id":"_:right","ex:name":"right"}
        }"#,
        r#"{"ex":"http://example.com/","link":{"@id":"http://example.com/link","@type":"@id"}}"#,
    );
}

#[test]
fn base_relativisation_cancels_on_re_expansion() {
    // `compactToRelative` rewrites `@id`s relative to the base; re-expanding under
    // the SAME base must restore the absolute IRIs exactly.
    // `JsonLdOptions` is `#[non_exhaustive]`, so build from the default and mutate.
    let mut opts = JsonLdOptions::default();
    opts.base = Some("http://example.com/dir/doc".to_string());
    assert_lossless(
        "base relativisation",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"http://example.com/dir/a",
            "ex:link":{"@id":"http://example.com/dir/sub/b"}
        }"#,
        r#"{"ex":"http://example.com/","link":{"@id":"http://example.com/link","@type":"@id"}}"#,
        &opts,
    );
}

#[test]
fn compact_arrays_disabled_still_round_trips() {
    let mut opts = JsonLdOptions::default();
    opts.compact_arrays = false;
    assert_lossless(
        "compactArrays: false",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:name":"Alice",
            "ex:tags":["x","y"]
        }"#,
        r#"{"ex":"http://example.com/","name":"http://example.com/name"}"#,
        &opts,
    );
}

#[test]
fn ordered_option_does_not_change_the_round_trip() {
    let mut opts = JsonLdOptions::default();
    opts.ordered = true;
    assert_lossless(
        "ordered: true",
        r#"{
            "@context":{"ex":"http://example.com/"},
            "@id":"ex:a",
            "ex:z":"last",
            "ex:a":"first"
        }"#,
        r#"{"ex":"http://example.com/"}"#,
        &opts,
    );
}

#[test]
fn empty_context_round_trips() {
    // The degenerate context: compaction is then close to the identity on the
    // expanded form, so this pins the post-processing (`[] → {}`, `@graph` wrap)
    // rather than term selection.
    assert_lossless_default(
        "empty context",
        r#"{"@context":{"ex":"http://example.com/"},"@id":"ex:a","ex:name":"Alice"}"#,
        r#"{}"#,
    );
}

// ---------------------------------------------------------------------------
// The flatten-lane companion
// ---------------------------------------------------------------------------

/// `flatten` is `expand ∘ node-map ∘ fold`, so the lossless-compaction invariant
/// carries to it: compacting a flattened document and re-flattening must reproduce
/// the flattened document. This is the flatten-lane statement of the same §C3
/// round-trip obligation.
#[test]
fn flatten_survives_a_compaction_round_trip() {
    let opts = JsonLdOptions::default();
    let ctx = Json::parse(r#"{"ex":"http://example.com/","name":"http://example.com/name"}"#)
        .expect("valid context JSON");

    for (case, src) in [
        (
            "flat nodes",
            r#"{
                "@context":{"ex":"http://example.com/"},
                "@id":"ex:a",
                "ex:name":"A",
                "ex:link":{"@id":"ex:b","ex:name":"B"}
            }"#,
        ),
        (
            "named graph",
            r#"{
                "@context":{"ex":"http://example.com/"},
                "@id":"ex:g",
                "@graph":[{"@id":"ex:a","ex:name":"A"},{"@id":"ex:b","ex:name":"B"}]
            }"#,
        ),
        (
            "blank nodes",
            r#"{
                "@context":{"ex":"http://example.com/"},
                "ex:link":{"ex:name":"inner"}
            }"#,
        ),
    ] {
        let src = Json::parse(src).expect("valid input JSON");
        let flat = flatten(&src, &opts, &NoopLoader).expect("flattening succeeds");
        let compacted =
            compact(&flat, &ctx, &opts, &NoopLoader).expect("compaction of the flattened form");
        let reflattened =
            flatten(&compacted, &opts, &NoopLoader).expect("re-flattening the compacted form");

        assert!(
            json_ld_equal(&reflattened, &flat),
            // [SONNET-4.6] positional format args (CodeQL false-positive guard).
            "{}: flatten → compact → flatten lost information\n  flat:       {}\n  compacted:  {}\n  reflattened:{}",
            case,
            render(&flat),
            render(&compacted),
            render(&reflattened)
        );
    }
}

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

// ---------------------------------------------------------------------------
// Generated round trips: nested lists, type maps and scoped contexts
// ---------------------------------------------------------------------------

/// A small deterministic generator (64-bit LCG), so failures reproduce from the case
/// number alone.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const PROPS: [&str; 3] = ["http://ex/p", "http://ex/q", "http://ex/r"];
const TYPES: [&str; 3] = ["http://ex/T", "http://ex/U", "http://other/T"];

/// One value of an expanded document: a literal, a reference, an embedded node or a
/// list (which may hold nested lists).
fn gen_value(g: &mut Gen, depth: usize, ids: &mut usize) -> String {
    match g.below(if depth > 0 { 8 } else { 5 }) {
        0 => format!(r#"{{"@value":"{}"}}"#, g.pick(&["a", "b", "T", "p"])),
        4 => format!(r#"{{"@value":"{}","@index":"{}"}}"#, g.pick(&["a", "b"]), g.pick(&["i", "j"])),
        // A graph object carries no @index: JSON-LD 1.1 compaction drops it in an
        // @graph @id map (W3C compact/0088), and RDF has none to carry.
        5 => format!(r#"{{"@graph":[{}]}}"#, gen_node(g, depth - 1, ids)),
        1 => r#"{"@value":"x","@language":"en"}"#.to_string(),
        2 => format!(r#"{{"@id":"{}"}}"#, g.pick(&["http://ex/T", "http://ex/b", "http://other/c"])),
        3 => r#"{"@value":"1","@type":"http://ex/dt"}"#.to_string(),
        6 => gen_node(g, depth - 1, ids),
        _ => {
            let items: Vec<String> = (0..g.below(3) + 1).map(|_| gen_value(g, depth - 1, ids)).collect();
            format!(r#"{{"@list":[{}]}}"#, items.join(","))
        }
    }
}

fn gen_node(g: &mut Gen, depth: usize, ids: &mut usize) -> String {
    *ids += 1;
    let mut members = vec![format!(r#""@id":"http://ex/n{}""#, ids)];
    if g.chance(20) {
        members.push(format!(r#""@index":"{}""#, g.pick(&["i", "j"])));
    }
    if g.chance(70) {
        let mut types: Vec<&str> = TYPES.iter().copied().filter(|_| g.chance(50)).collect();
        if types.is_empty() {
            types.push(g.pick(&TYPES));
        }
        let types: Vec<String> = types.iter().map(|t| format!(r#""{t}""#)).collect();
        members.push(format!(r#""@type":[{}]"#, types.join(",")));
    }
    for p in PROPS {
        if g.chance(50) {
            let values: Vec<String> = (0..g.below(2) + 1).map(|_| gen_value(g, depth, ids)).collect();
            members.push(format!(r#""{p}":[{}]"#, values.join(",")));
        }
    }
    if g.chance(15) {
        members.push(format!(r#""@reverse":{{"{}":[{{"@id":"http://ex/b"}}]}}"#, g.pick(&PROPS)));
    }
    format!("{{{}}}", members.join(","))
}

/// A small context to scope under a term: it may move @vocab, take over or redefine
/// keyword aliases, and respell types and properties.
fn gen_scoped(g: &mut Gen) -> String {
    let mut entries = Vec::new();
    for entry in [
        r#""@vocab":"http://other/""#,
        r#""@vocab":"http://ex/""#,
        r#""type":"http://ex/data""#,
        r#""t":"@type""#,
        r#""l":"http://ex/data""#,
        r#""ls":"@list""#,
        r#""id":"http://ex/data""#,
        r#""T":"http://other/T""#,
        r#""U":"http://ex/T""#,
        r#""q":"http://ex/r""#,
        r#""type":"@type""#,
        r#""p":{"@id":"http://ex/p","@type":"@id"}"#,
        r#""r":{"@id":"http://ex/r","@container":"@list"}"#,
        r#""@propagate":false"#,
        r#""ex":"http://other/""#,
        r#""ex":"http://ex/""#,
        r#""@language":"en""#,
        r#""p":{"@id":"http://ex/p","@container":"@language"}"#,
        r#""v":"@value""#,
        r#""l":"@value""#,
        r#""none":"@none""#,
        r#""p":{"@id":"http://ex/p","@container":"@id","@context":{"q":"http://ex/q"}}"#,
        r#""q":{"@id":"http://ex/q","@container":"@index","@context":{"@vocab":"http://other/"}}"#,
        r#""r":{"@id":"http://ex/r","@container":"@type","@context":{"p":"http://ex/q"}}"#,
        r#""p":{"@id":"http://ex/p","@container":"@index","@index":"q"}"#,
        r#""q":{"@id":"http://ex/q","@nest":"@nest"}"#,
        r#""r":{"@id":"http://ex/r","@container":["@graph","@id"]}"#,
        r#""p":{"@reverse":"http://ex/q"}"#,
        r#""rp":{"@reverse":"http://ex/p"}"#,
    ] {
        if g.chance(20) {
            entries.push(entry.to_string());
        }
    }
    format!("{{{}}}", entries.join(","))
}

fn gen_context(g: &mut Gen) -> String {
    let mut entries = Vec::new();
    if g.chance(70) {
        entries.push(format!(r#""@vocab":"{}""#, g.pick(&["http://ex/", "http://other/"])));
    }
    if g.chance(50) {
        entries.push(format!(r#""ex":"{}""#, g.pick(&["http://ex/", "http://other/"])));
    }
    if g.chance(20) {
        entries.push(r#""@language":"en""#.to_string());
    }
    for alias in [r#""type":"@type""#, r#""l":"@list""#, r#""id":"@id""#, r#""v":"@value""#, r#""none":"@none""#] {
        if g.chance(40) {
            entries.push(alias.to_string());
        }
    }
    if g.chance(30) {
        entries.push(format!(r#""rp":{{"@reverse":"{}"}}"#, g.pick(&PROPS)));
    }
    for (term, iri) in [("T", "http://ex/T"), ("U", "http://ex/U")] {
        if g.chance(50) {
            let scoped = if g.chance(50) { format!(r#","@context":{}"#, gen_scoped(g)) } else { String::new() };
            entries.push(format!(r#""{term}":{{"@id":"{iri}"{scoped}}}"#));
        }
    }
    for (term, iri) in [("p", PROPS[0]), ("q", PROPS[1]), ("r", PROPS[2])] {
        if g.chance(70) {
            let mut def = vec![format!(r#""@id":"{iri}""#)];
            if g.chance(40) {
                def.push(format!(
                    r#""@container":{}"#,
                    g.pick(&[
                        r#""@type""#,
                        r#""@list""#,
                        r#""@set""#,
                        r#""@id""#,
                        r#""@language""#,
                        r#""@graph""#,
                        r#""@index""#,
                        r#"["@graph","@id"]"#,
                        r#"["@graph","@index"]"#,
                        r#"["@index","@set"]"#,
                    ])
                ));
                if def[1].contains(r#""@index""#) && g.chance(30) {
                    def.push(format!(r#""@index":"{}""#, g.pick(&["q", "r"])));
                }
            } else if g.chance(15) {
                def.push(r#""@nest":"@nest""#.to_string());
            }
            if g.chance(20) {
                def.push(format!(r#""@type":"{}""#, g.pick(&["@id", "@vocab"])));
            }
            if g.chance(50) {
                def.push(format!(r#""@context":{}"#, gen_scoped(g)));
            }
            entries.push(format!(r#""{term}":{{{}}}"#, def.join(",")));
        }
    }
    format!("{{{}}}", entries.join(","))
}

/// `expand(compact_expanded(D, C)) ≡ D` over generated expanded documents and contexts
/// mixing nested lists, type maps, keyword aliases and property- and type-scoped
/// contexts. A context or document the processor rejects is skipped; any document it
/// accepts must read back unchanged.
#[test]
fn generated_documents_round_trip_under_scoped_contexts() {
    let opts = JsonLdOptions::default();
    let (mut checked, mut failures) = (0, Vec::new());
    // Each family must be exercised by accepted cases, so a generator change that stops
    // producing one fails instead of silently shrinking the test.
    let families: [(&str, &str); 13] = [
        ("@reverse term", r#"{"@reverse":"#),
        ("@id map", r#""@container":"@id""#),
        ("@type map", r#""@container":"@type""#),
        ("@index map", r#""@container":"@index""#),
        ("property-valued @index map", r#""@index":"q""#),
        ("@graph @id map", r#"["@graph","@id"]"#),
        ("@graph @index map", r#"["@graph","@index"]"#),
        ("@list container", r#""@container":"@list""#),
        ("@nest", r#""@nest":"@nest""#),
        ("@value alias", r#""v":"@value""#),
        ("@none alias", r#""none":"@none""#),
        ("@propagate false", r#""@propagate":false"#),
        ("scoped map term", r#""@container":"@id","@context""#),
    ];
    let mut seen = [0usize; 13];
    let cases: u64 = std::env::var("ROUND_TRIP_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(20_000);
    let only: Option<u64> = std::env::var("ROUND_TRIP_CASE").ok().and_then(|n| n.parse().ok());
    for case in 0..cases {
        if only.is_some_and(|o| o != case) {
            continue;
        }
        let mut g = Gen(case.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
        let mut ids = 0;
        let mut nodes = vec![gen_node(&mut g, 2, &mut ids)];
        if g.chance(30) {
            let inner = gen_node(&mut g, 1, &mut ids);
            nodes.push(format!(r#"{{"@id":"http://ex/g","@graph":[{inner}]}}"#));
        }
        let doc = format!("[{}]", nodes.join(","));
        let ctx = gen_context(&mut g);
        let doc = Json::parse(&doc).expect("generated document parses");
        let ctx_json = Json::parse(&ctx).expect("generated context parses");
        // The generated document is close to expanded form; expansion normalises it
        // (dropping, for one, a node that is only an @id inside a @graph).
        let Ok(expanded) = expand(&doc, &opts, &NoopLoader) else { continue };
        let Ok(compacted) = sparq_jsonld::compact::compact_expanded(&expanded, &ctx_json, &opts, &NoopLoader) else {
            continue;
        };
        checked += 1;
        for (n, (_, needle)) in seen.iter_mut().zip(&families) {
            *n += usize::from(ctx.contains(needle));
        }
        let back = expand(&compacted, &opts, &NoopLoader);
        if !back.as_ref().is_ok_and(|b| json_ld_equal(b, &expanded)) {
            failures.push(format!(
                "case {case}\n  context:  {ctx}\n  input:    {}\n  output:   {}\n  read back:{}",
                render(&expanded),
                render(&compacted),
                back.map(|b| render(&b)).unwrap_or_else(|e| format!("{e:?}"))
            ));
        }
    }
    assert!(only.is_some() || checked > 1000, "only {checked} cases were accepted");
    if only.is_none() && cases >= 20_000 {
        for ((family, _), n) in families.iter().zip(seen) {
            assert!(n >= 50, "only {n} accepted cases exercise {family}");
        }
    }
    assert!(failures.is_empty(), "{} of {checked} changed the data:\n{}", failures.len(), failures.iter().take(5).cloned().collect::<Vec<_>>().join("\n"));
}

/// Every container kind, defined (with a property-scoped context) inside a type-scoped
/// context, on a chain of nodes nested 30 deep through that property. Compaction must
/// finish within its linear work bound and read back unchanged, so a container kind whose
/// items are compacted twice, or under a context expansion does not use, fails here.
#[test]
fn every_container_kind_reads_back_under_a_type_scoped_definition() {
    const DEPTH: usize = 30;
    let kinds: [&str; 11] = [
        r#""""#,
        r#""@set""#,
        r#""@list""#,
        r#""@index""#,
        r#""@id""#,
        r#""@type""#,
        r#""@language""#,
        r#""@graph""#,
        r#"["@graph","@id"]"#,
        r#"["@graph","@index"]"#,
        r#"["@index","@set"]"#,
    ];
    let opts = JsonLdOptions::default();
    for kind in kinds {
        let container = if kind == r#""""# { String::new() } else { format!(r#""@container":{kind},"#) };
        let ctx = format!(
            r#"{{"@vocab":"http://outer/","none":"@none","T":{{"@id":"http://ex/T","@context":{{"v":"@value","p":{{"@id":"http://ex/p",{container}"@context":{{"q":"http://ex/q"}}}}}}}}}}"#
        );
        // The innermost node, then each level wraps the previous one as its `p` value.
        let mut node = r#"{"@id":"http://ex/n0","@type":["http://ex/T"],"http://ex/q":[{"@value":"v"}]}"#.to_string();
        for level in 1..=DEPTH {
            let value = match kind {
                r#""@list""# => format!(r#"{{"@list":[{node}]}}"#),
                r#""@language""# => r#"{"@value":"v","@language":"en"}"#.to_string(),
                // A bare @graph container keeps no @index (RDF carries none either).
                r#""@graph""# | r#"["@graph","@id"]"# => format!(r#"{{"@graph":[{node}]}}"#),
                r#"["@graph","@index"]"# => format!(r#"{{"@graph":[{node}],"@index":"i{level}"}}"#),
                r#""@index""# | r#"["@index","@set"]"# => {
                    let mut n = node.clone();
                    n.insert_str(1, &format!(r#""@index":"i{level}","#));
                    n
                }
                _ => node.clone(),
            };
            node = format!(
                r#"{{"@id":"http://ex/n{level}","@type":["http://ex/T"],"http://ex/q":[{{"@value":"v"}}],"http://ex/p":[{value}]}}"#
            );
        }
        let doc = Json::parse(&format!("[{node}]")).expect("chain parses");
        let ctx_json = Json::parse(&ctx).expect("context parses");
        let expanded = expand(&doc, &opts, &NoopLoader).expect("chain expands");
        let compacted = sparq_jsonld::compact::compact_expanded(&expanded, &ctx_json, &opts, &NoopLoader)
            .unwrap_or_else(|e| panic!("{kind}: compaction failed: {e}"));
        let back = expand(&compacted, &opts, &NoopLoader).unwrap_or_else(|e| panic!("{kind}: re-expansion failed: {e}"));
        assert!(
            json_ld_equal(&back, &expanded),
            "{kind}: the chain changed\n  output:    {}\n  read back: {}",
            render(&compacted),
            render(&back)
        );
    }
}

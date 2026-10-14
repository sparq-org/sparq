//! sq-oy1f.27 — direct crate-local tests for the document-level
//! **Compaction Algorithm** + **Value Compaction** (`sparq_jsonld::compact`).
//!
//! The authoritative breadth oracle is the W3C `compact` conformance lane in
//! `sparq-conformance` (ratchet floor `floors::compact::FLOOR`, compared against the
//! suite's NORMATIVE expected documents). These tests pin the tricky behaviours with
//! EXACT output shapes (byte-precise serialisation, so array order — including
//! compacted `@list` order, which the lane's set-comparator cannot see — and member
//! order are load-bearing here): term selection ties, container coercions
//! (`@list`/`@language`/`@index`/`@id`/`@type`/`@graph` maps), keyword aliasing,
//! compact-IRI vs term preference, value compaction (type/language/direction
//! matching, `@id`/`@vocab` coercion, `@json`), scoped contexts (property/type
//! -scoped + previous-context reversion), `@nest`, `@reverse` redistribution, and
//! the options (`compactArrays`, `compactToRelative`, `ordered`).

use sparq_jsonld::compact::{compact, compact_expanded};
use sparq_jsonld::{Json, JsonLdError, JsonLdErrorCode, JsonLdOptions, NoopLoader};

/// Parse, compact, and serialise. The input is a (compact or expanded) JSON-LD
/// document; `compact()` expands it natively first — so these tests exercise the
/// REAL pipeline end to end.
fn run(input: &str, ctx: &str, opts: &JsonLdOptions) -> String {
    let input = Json::parse(input).expect("valid input JSON");
    let ctx = Json::parse(ctx).expect("valid context JSON");
    let out = compact(&input, &ctx, opts, &NoopLoader).expect("compaction succeeds");
    let mut s = String::new();
    out.write(&mut s);
    s
}

/// `run` with the spec-default options.
fn run_default(input: &str, ctx: &str) -> String {
    run(input, ctx, &JsonLdOptions::default())
}

/// `run` returning the raised error.
fn run_err(input: &str, ctx: &str) -> JsonLdError {
    let input = Json::parse(input).expect("valid input JSON");
    let ctx = Json::parse(ctx).expect("valid context JSON");
    compact(&input, &ctx, &JsonLdOptions::default(), &NoopLoader)
        .expect_err("compaction must raise")
}

// ---------------------------------------------------------------------------
// Basics: term compaction, @context embedding, array collapse
// ---------------------------------------------------------------------------

#[test]
fn term_compaction_embeds_the_caller_context() {
    let got = run_default(
        r#"[{"http://xmlns.com/foaf/0.1/name":[{"@value":"Mark"}]}]"#,
        r#"{"name":"http://xmlns.com/foaf/0.1/name"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"name":"http://xmlns.com/foaf/0.1/name"},"name":"Mark"}"#
    );
}

#[test]
fn context_document_is_unwrapped_one_layer() {
    // A context document ({"@context": …}) contributes its inner value — and the
    // OUTPUT embeds the inner value, not the wrapper.
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":1}]}]"#,
        r#"{"@context":{"p":"http://ex/p"}}"#,
    );
    assert_eq!(got, r#"{"@context":{"p":"http://ex/p"},"p":1}"#);
}

#[test]
fn empty_context_is_not_embedded() {
    assert_eq!(run_default(r#"[{"http://ex/p":[{"@value":1}]}]"#, r#"{}"#), r#"{"http://ex/p":1}"#);
    assert_eq!(run_default(r#"[{"http://ex/p":[{"@value":1}]}]"#, r#"null"#), r#"{"http://ex/p":1}"#);
}

#[test]
fn array_context_is_embedded_verbatim() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":1}]}]"#,
        r#"[{"p":"http://ex/p"},{"q":"http://ex/q"}]"#,
    );
    assert_eq!(
        got,
        r#"{"@context":[{"p":"http://ex/p"},{"q":"http://ex/q"}],"p":1}"#
    );
}

#[test]
fn singleton_collapses_unless_compact_arrays_is_false() {
    let input = r#"[{"http://ex/p":[{"@value":"a"}]}]"#;
    let ctx = r#"{"p":"http://ex/p"}"#;
    assert_eq!(
        run_default(input, ctx),
        r#"{"@context":{"p":"http://ex/p"},"p":"a"}"#
    );
    let mut opts = JsonLdOptions::default();
    opts.compact_arrays = false;
    // With compactArrays false even the TOP-LEVEL single-node array is kept, so
    // the API post-processing wraps it under @graph.
    assert_eq!(
        run(input, ctx, &opts),
        r#"{"@context":{"p":"http://ex/p"},"@graph":[{"p":["a"]}]}"#
    );
}

#[test]
fn set_container_keeps_the_singleton_array() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"a"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@container":"@set"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@container":"@set"}},"p":["a"]}"#
    );
}

#[test]
fn empty_array_property_is_retained_as_an_empty_array() {
    let got = run_default(
        r#"{"@context":{"p":{"@id":"http://ex/p","@container":"@set"}},"p":[]}"#,
        r#"{"p":{"@id":"http://ex/p","@container":"@set"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@container":"@set"}},"p":[]}"#
    );
}

#[test]
fn multi_node_document_is_wrapped_under_aliased_graph() {
    // (Each node carries a property — expansion drops property-less free-floating
    // node references.)
    let got = run_default(
        r#"[{"@id":"http://ex/a","http://ex/p":[{"@value":1}]},{"@id":"http://ex/b","http://ex/p":[{"@value":2}]}]"#,
        r#"{"g":"@graph","p":"http://ex/p"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"g":"@graph","p":"http://ex/p"},"g":[{"@id":"http://ex/a","p":1},{"@id":"http://ex/b","p":2}]}"#
    );
}

#[test]
fn ordered_option_sorts_entry_keys() {
    let mut opts = JsonLdOptions::default();
    opts.ordered = true;
    let got = run(
        r#"[{"http://ex/b":[{"@value":1}],"http://ex/a":[{"@value":2}]}]"#,
        r#"{"a":"http://ex/a","b":"http://ex/b"}"#,
        &opts,
    );
    assert_eq!(
        got,
        r#"{"@context":{"a":"http://ex/a","b":"http://ex/b"},"a":2,"b":1}"#
    );
}

// ---------------------------------------------------------------------------
// Value Compaction
// ---------------------------------------------------------------------------

#[test]
fn matching_type_mapping_drops_the_value_object() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"5","@type":"http://www.w3.org/2001/XMLSchema#integer"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@type":"http://www.w3.org/2001/XMLSchema#integer"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@type":"http://www.w3.org/2001/XMLSchema#integer"}},"p":"5"}"#
    );
}

#[test]
fn matching_type_with_unexpressed_index_keeps_the_value_object() {
    // REGRESSION (sq-iika9): a matching @type must NOT drop to the bare @value when
    // the object carries an @index that no @index container re-expresses — that
    // would silently LOSE the @index (self-reparse-invisible data loss). The REC's
    // literal step-6 text has no @index condition; jsonld.js guards it
    // (`preserveIndex`) and we adjudicate with jsonld.js: fall through to the
    // general map path, which keeps @value + @type + @index intact.
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"5","@type":"http://ex/T","@index":"i0"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@type":"http://ex/T"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@type":"http://ex/T"}},"p":{"@value":"5","@type":"http://ex/T","@index":"i0"}}"#
    );
}

#[test]
fn matching_type_with_index_container_still_drops_to_the_bare_value() {
    // Companion non-regression: when the term's @index container DOES re-express
    // the @index (as the map key), a matching @type still compacts to the bare
    // @value — the guard must not over-block.
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"5","@type":"http://ex/T","@index":"i0"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@type":"http://ex/T","@container":"@index"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@type":"http://ex/T","@container":"@index"}},"p":{"i0":"5"}}"#
    );
}

#[test]
fn mismatched_type_keeps_the_value_object_with_compacted_type() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"5","@type":"http://ex/Custom"}]}]"#,
        r#"{"p":"http://ex/p","Custom":"http://ex/Custom"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":"http://ex/p","Custom":"http://ex/Custom"},"p":{"@value":"5","@type":"Custom"}}"#
    );
}

#[test]
fn id_typed_term_compacts_a_node_reference_to_a_string() {
    let got = run_default(
        r#"[{"http://ex/knows":[{"@id":"http://ex/bob"}]}]"#,
        r#"{"knows":{"@id":"http://ex/knows","@type":"@id"},"bob":"http://ex/bob"}"#,
    );
    // vocab=false for @id coercion: the compact IRI form is NOT used for a term
    // alias, but prefix-less "bob" is a term only under vocab compaction — the
    // document-relative form keeps the absolute IRI here.
    assert_eq!(
        got,
        r#"{"@context":{"knows":{"@id":"http://ex/knows","@type":"@id"},"bob":"http://ex/bob"},"knows":"http://ex/bob"}"#
    );
}

#[test]
fn vocab_typed_term_compacts_a_node_reference_through_terms() {
    let got = run_default(
        r#"[{"http://ex/knows":[{"@id":"http://ex/bob"}]}]"#,
        r#"{"knows":{"@id":"http://ex/knows","@type":"@vocab"},"bob":"http://ex/bob"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"knows":{"@id":"http://ex/knows","@type":"@vocab"},"bob":"http://ex/bob"},"knows":"bob"}"#
    );
}

#[test]
fn language_match_is_case_insensitive() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"Hallo","@language":"DE"}]}]"#,
        r#"{"@language":"de","p":"http://ex/p"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@language":"de","p":"http://ex/p"},"p":"Hallo"}"#
    );
}

#[test]
fn language_mismatch_keeps_the_value_object() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"hello","@language":"en"}]}]"#,
        r#"{"@language":"de","p":"http://ex/p"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@language":"de","p":"http://ex/p"},"p":{"@value":"hello","@language":"en"}}"#
    );
}

#[test]
fn null_language_term_accepts_plain_strings_under_a_default_language() {
    // term5-style: "@language": null suppresses the context default, so a PLAIN
    // string compacts to a bare value under that term.
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"plain"}]}]"#,
        r#"{"@language":"de","p":{"@id":"http://ex/p","@language":null}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@language":"de","p":{"@id":"http://ex/p","@language":null}},"p":"plain"}"#
    );
}

#[test]
fn direction_must_match_the_term_direction() {
    let ctx = r#"{"p":{"@id":"http://ex/p","@direction":"rtl"}}"#;
    // Matching direction → bare string.
    assert_eq!(
        run_default(r#"[{"http://ex/p":[{"@value":"x","@direction":"rtl"}]}]"#, ctx),
        r#"{"@context":{"p":{"@id":"http://ex/p","@direction":"rtl"}},"p":"x"}"#
    );
    // Missing direction: the rtl term cannot be selected at all, so the property
    // stays under its full IRI — where value compaction (no term constraints)
    // legitimately yields the bare string.
    assert_eq!(
        run_default(r#"[{"http://ex/p":[{"@value":"x"}]}]"#, ctx),
        r#"{"@context":{"p":{"@id":"http://ex/p","@direction":"rtl"}},"http://ex/p":"x"}"#
    );
}

#[test]
fn json_typed_term_returns_the_raw_payload() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":{"a":[1,true]},"@type":"@json"}]}]"#,
        r#"{"@version":1.1,"p":{"@id":"http://ex/p","@type":"@json"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"p":{"@id":"http://ex/p","@type":"@json"}},"p":{"a":[1,true]}}"#
    );
}

#[test]
fn non_string_literals_compact_to_native_scalars() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":42},{"@value":true}]}]"#,
        r#"{"p":"http://ex/p"}"#,
    );
    assert_eq!(got, r#"{"@context":{"p":"http://ex/p"},"p":[42,true]}"#);
}

// ---------------------------------------------------------------------------
// Container maps
// ---------------------------------------------------------------------------

#[test]
fn language_map_reshapes_by_language() {
    let got = run_default(
        r#"[{"http://ex/label":[
            {"@value":"Die Königin","@language":"de"},
            {"@value":"Ihre Majestät","@language":"de"},
            {"@value":"The Queen","@language":"en"}]}]"#,
        r#"{"label":{"@id":"http://ex/label","@container":"@language"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"label":{"@id":"http://ex/label","@container":"@language"}},"label":{"de":["Die Königin","Ihre Majestät"],"en":"The Queen"}}"#
    );
}

#[test]
fn index_map_reshapes_by_index_and_drops_the_index_entry() {
    let got = run_default(
        r#"[{"http://ex/p":[
            {"@value":"a","@index":"one"},
            {"@value":"b","@index":"two"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@container":"@index"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@container":"@index"}},"p":{"one":"a","two":"b"}}"#
    );
}

#[test]
fn missing_index_files_under_none_alias() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":"a","@index":"one"},{"@value":"b"}]}]"#,
        r#"{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@index"},"other":"@none"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@index"},"other":"@none"},"p":{"one":"a","other":"b"}}"#
    );
}

#[test]
fn property_valued_index_map_keys_on_the_index_property() {
    // @index names a property: the map key is that property's first value; it is
    // removed from the item (single value) rather than duplicated.
    let got = run_default(
        r#"[{"http://ex/prop":[
            {"@id":"http://ex/a","http://ex/name":[{"@value":"n1"}]},
            {"@id":"http://ex/b","http://ex/name":[{"@value":"n2"}]}]}]"#,
        r#"{"@version":1.1,"prop":{"@id":"http://ex/prop","@container":"@index","@index":"http://ex/name"},"name":"http://ex/name"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"prop":{"@id":"http://ex/prop","@container":"@index","@index":"http://ex/name"},"name":"http://ex/name"},"prop":{"n1":{"@id":"http://ex/a"},"n2":{"@id":"http://ex/b"}}}"#
    );
}

#[test]
fn id_map_keys_on_the_compacted_id() {
    let got = run_default(
        r#"[{"http://ex/p":[
            {"@id":"http://ex/a","http://ex/v":[{"@value":1}]},
            {"@id":"http://ex/b","http://ex/v":[{"@value":2}]}]}]"#,
        r#"{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@id"},"v":"http://ex/v"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@id"},"v":"http://ex/v"},"p":{"http://ex/a":{"v":1},"http://ex/b":{"v":2}}}"#
    );
}

#[test]
fn type_map_keys_on_the_first_type() {
    let got = run_default(
        r#"[{"http://ex/p":[
            {"@type":["http://ex/T1"],"http://ex/v":[{"@value":1}]},
            {"@type":["http://ex/T2"],"http://ex/v":[{"@value":2}]}]}]"#,
        r#"{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@type"},"T1":"http://ex/T1","T2":"http://ex/T2","v":"http://ex/v"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"p":{"@id":"http://ex/p","@container":"@type"},"T1":"http://ex/T1","T2":"http://ex/T2","v":"http://ex/v"},"p":{"T1":{"v":1},"T2":{"v":2}}}"#
    );
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn list_container_compacts_to_a_bare_array_in_list_order() {
    // EXACT order assertion — compacted @list order is invisible to the lane's
    // set-comparator (see the lane doc), so it is pinned here.
    let got = run_default(
        r#"[{"http://ex/p":[{"@list":[{"@value":3},{"@value":1},{"@value":2}]}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@container":"@list"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":{"@id":"http://ex/p","@container":"@list"}},"p":[3,1,2]}"#
    );
}

#[test]
fn list_without_list_container_is_rewrapped_with_aliased_keywords() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@list":[{"@value":"a"}],"@index":"i0"}]}]"#,
        r#"{"p":"http://ex/p","list":"@list","index":"@index"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"p":"http://ex/p","list":"@list","index":"@index"},"p":{"list":["a"],"index":"i0"}}"#
    );
}

#[test]
fn list_term_selection_uses_the_common_language_of_the_items() {
    // Two terms share the IRI; the all-"en" list must select the @language:"en"
    // @list term, and the mixed list must fall back to the plain @list term.
    let ctx = r#"{"@language":"de","t":{"@id":"http://ex/p","@container":"@list"},"ten":{"@id":"http://ex/p","@container":"@list","@language":"en"}}"#;
    let got = run_default(
        r#"[{"http://ex/p":[{"@list":[{"@value":"a","@language":"en"},{"@value":"b","@language":"en"}]}]}]"#,
        ctx,
    );
    assert_eq!(
        got,
        format!(r#"{{"@context":{},"ten":["a","b"]}}"#, ctx)
    );
    let got = run_default(
        r#"[{"http://ex/p":[{"@list":[{"@value":"a","@language":"en"},{"@value":"b","@language":"fr"}]}]}]"#,
        ctx,
    );
    // Conflicting languages: common language @none — the unconstrained @list term
    // wins, and each item keeps its language tag.
    assert_eq!(
        got,
        format!(
            r#"{{"@context":{},"t":[{{"@value":"a","@language":"en"}},{{"@value":"b","@language":"fr"}}]}}"#,
            ctx
        )
    );
}

// ---------------------------------------------------------------------------
// Graph containers
// ---------------------------------------------------------------------------

#[test]
fn graph_id_container_maps_by_graph_name() {
    let got = run_default(
        r#"[{"http://ex/input":[
            {"@graph":[{"http://ex/value":[{"@value":"x"}]}],"@id":"http://ex/g1"}]}]"#,
        r#"{"@version":1.1,"input":{"@id":"http://ex/input","@container":["@graph","@id"]},"value":"http://ex/value"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"input":{"@id":"http://ex/input","@container":["@graph","@id"]},"value":"http://ex/value"},"input":{"http://ex/g1":{"value":"x"}}}"#
    );
}

#[test]
fn graph_index_container_maps_by_graph_index() {
    let got = run_default(
        r#"[{"http://ex/input":[
            {"@graph":[{"http://ex/value":[{"@value":"x"}]}],"@index":"g1"}]}]"#,
        r#"{"@version":1.1,"input":{"@id":"http://ex/input","@container":["@graph","@index"]},"value":"http://ex/value"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"input":{"@id":"http://ex/input","@container":["@graph","@index"]},"value":"http://ex/value"},"input":{"g1":{"value":"x"}}}"#
    );
}

#[test]
fn graph_index_container_indexless_graph_files_under_aliased_none() {
    // REGRESSION (sq-iika9): 12.8.7.2.2 — a simple graph object WITHOUT @index files
    // under @none, and that fallback key is IRI-COMPACTED ("IRI compacting that
    // value"), so a context alias for @none must be honoured — exactly as in the
    // 12.8.7.1 @graph+@id form and the 12.8.9.9 container maps. Before the fix the
    // literal string "@none" leaked out as the map key instead of the "none" alias.
    let got = run_default(
        r#"[{"http://ex/input":[
            {"@graph":[{"http://ex/value":[{"@value":"x"}]}]}]}]"#,
        r#"{"@version":1.1,"none":"@none","input":{"@id":"http://ex/input","@container":["@graph","@index"]},"value":"http://ex/value"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"none":"@none","input":{"@id":"http://ex/input","@container":["@graph","@index"]},"value":"http://ex/value"},"input":{"none":{"value":"x"}}}"#
    );
}

#[test]
fn simple_graph_with_multiple_nodes_wraps_in_included() {
    let got = run_default(
        r#"[{"http://ex/input":[
            {"@graph":[
                {"@id":"http://ex/a","http://ex/v":[{"@value":1}]},
                {"@id":"http://ex/b","http://ex/v":[{"@value":2}]}]}]}]"#,
        r#"{"@version":1.1,"input":{"@id":"http://ex/input","@container":"@graph"},"v":"http://ex/v"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"input":{"@id":"http://ex/input","@container":"@graph"},"v":"http://ex/v"},"input":{"@included":[{"@id":"http://ex/a","v":1},{"@id":"http://ex/b","v":2}]}}"#
    );
}

#[test]
fn named_graph_without_graph_container_is_rewrapped() {
    let got = run_default(
        r#"[{"http://ex/input":[
            {"@graph":[{"http://ex/v":[{"@value":1}]}],"@id":"http://ex/g"}]}]"#,
        r#"{"input":"http://ex/input","v":"http://ex/v"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"input":"http://ex/input","v":"http://ex/v"},"input":{"@graph":{"v":1},"@id":"http://ex/g"}}"#
    );
}

// ---------------------------------------------------------------------------
// @nest, @reverse, @preserve
// ---------------------------------------------------------------------------

#[test]
fn nest_groups_properties_under_the_nest_term() {
    let got = run_default(
        r#"[{"http://ex/p":[{"@value":1}],"http://ex/q":[{"@value":2}]}]"#,
        r#"{"@version":1.1,"meta":"@nest","p":{"@id":"http://ex/p","@nest":"meta"},"q":"http://ex/q"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"meta":"@nest","p":{"@id":"http://ex/p","@nest":"meta"},"q":"http://ex/q"},"meta":{"p":1},"q":2}"#
    );
}

#[test]
fn invalid_nest_value_is_raised() {
    // A @nest mapping that neither is "@nest" nor expands to it is rejected at
    // context-processing time OR compaction time — either way the pipeline fails
    // closed with the spec error code.
    let err = run_err(
        r#"[{"http://ex/p":[{"@value":1}]}]"#,
        r#"{"@version":1.1,"notnest":"http://ex/other","p":{"@id":"http://ex/p","@nest":"notnest"}}"#,
    );
    assert_eq!(err.code(), JsonLdErrorCode::InvalidNestValue);
}

#[test]
fn reverse_entries_redistribute_onto_reverse_terms() {
    let got = run_default(
        r#"[{"@id":"http://ex/markus","@reverse":{"http://ex/knows":[{"@id":"http://ex/dave"}]}}]"#,
        r#"{"isKnownBy":{"@reverse":"http://ex/knows"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"isKnownBy":{"@reverse":"http://ex/knows"}},"@id":"http://ex/markus","isKnownBy":{"@id":"http://ex/dave"}}"#
    );
}

#[test]
fn reverse_index_container_builds_the_map_under_the_reverse_term() {
    // The t0036 shape: @index containers must be considered for REVERSE properties
    // (the spec appends @index containers before the reverse branch).
    let got = run_default(
        r#"[{"@id":"http://ex/m","@reverse":{"http://ex/knows":[
            {"@id":"http://ex/d","@index":"Dave"}]}}]"#,
        r#"{"isKnownBy":{"@reverse":"http://ex/knows","@container":"@index"}}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"isKnownBy":{"@reverse":"http://ex/knows","@container":"@index"}},"@id":"http://ex/m","isKnownBy":{"Dave":{"@id":"http://ex/d"}}}"#
    );
}

#[test]
fn unmatched_reverse_entries_stay_under_the_reverse_alias() {
    let got = run_default(
        r#"[{"@id":"http://ex/m","@reverse":{"http://ex/knows":[{"@id":"http://ex/d"}]}}]"#,
        r#"{"rev":"@reverse","knows":"http://ex/knows"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"rev":"@reverse","knows":"http://ex/knows"},"@id":"http://ex/m","rev":{"knows":{"@id":"http://ex/d"}}}"#
    );
}

#[test]
fn preserve_payload_is_compacted_in_place() {
    // @preserve is a framing-pipeline construct; compact_expanded must compact its
    // payload and keep the keyword (no expansion round-trip, so drive it directly).
    let expanded = Json::parse(
        r#"[{"@preserve":[{"http://ex/p":[{"@value":"x"}]}]}]"#,
    )
    .expect("expanded");
    let ctx = Json::parse(r#"{"p":"http://ex/p"}"#).expect("ctx");
    let out = compact_expanded(&expanded, &ctx, &JsonLdOptions::default(), &NoopLoader)
        .expect("compaction succeeds");
    let mut s = String::new();
    out.write(&mut s);
    assert_eq!(
        s,
        r#"{"@context":{"p":"http://ex/p"},"@preserve":{"p":"x"}}"#
    );
}

// ---------------------------------------------------------------------------
// Keyword aliasing, term-vs-CURIE preference, scoped contexts
// ---------------------------------------------------------------------------

#[test]
fn keyword_aliases_apply_to_id_and_type() {
    let got = run_default(
        r#"[{"@id":"http://ex/a","@type":["http://ex/T"]}]"#,
        r#"{"id":"@id","type":"@type","T":"http://ex/T"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"id":"@id","type":"@type","T":"http://ex/T"},"id":"http://ex/a","type":"T"}"#
    );
}

#[test]
fn type_alias_with_set_container_keeps_the_array_in_1_1() {
    let got = run_default(
        r#"[{"@id":"http://ex/a","@type":["http://ex/T"]}]"#,
        r#"{"@version":1.1,"type":{"@id":"@type","@container":"@set"},"T":"http://ex/T"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"@version":1.1,"type":{"@id":"@type","@container":"@set"},"T":"http://ex/T"},"@id":"http://ex/a","type":["T"]}"#
    );
}

#[test]
fn term_beats_compact_iri_and_shorter_alias_wins_ties() {
    // "n" and "nm" both map to the IRI; the shortest term must win over both the
    // longer term and the CURIE form ("ex:name").
    let got = run_default(
        r#"[{"http://ex/name":[{"@value":"x"}]}]"#,
        r#"{"ex":"http://ex/","nm":"http://ex/name","n":"http://ex/name"}"#,
    );
    assert_eq!(
        got,
        r#"{"@context":{"ex":"http://ex/","nm":"http://ex/name","n":"http://ex/name"},"n":"x"}"#
    );
}

#[test]
fn unmatched_iri_falls_back_to_a_compact_iri() {
    let got = run_default(
        r#"[{"http://ex/other":[{"@value":"x"}]}]"#,
        r#"{"ex":"http://ex/"}"#,
    );
    assert_eq!(got, r#"{"@context":{"ex":"http://ex/"},"ex:other":"x"}"#);
}

#[test]
fn vocab_relative_suffix_is_used_when_unambiguous() {
    let got = run_default(
        r#"[{"http://vocab/other":[{"@value":"x"}]}]"#,
        r#"{"@vocab":"http://vocab/"}"#,
    );
    assert_eq!(got, r#"{"@context":{"@vocab":"http://vocab/"},"other":"x"}"#);
}

#[test]
fn property_scoped_context_applies_inside_the_property() {
    // The tc013 shape: Foo's TYPE-scoped context defines bar with a PROPERTY-scoped
    // context defining baz with @type @vocab. The child node under bar reverts the
    // type-scoped context (step 4) but must still find bar's property-scoped
    // context (looked up in the incoming context), so the inner node reference
    // compacts to the bare vocab term "buzz".
    let ctx = r#"{"@vocab":"http://example/","Foo":{"@context":{"bar":{"@context":{"baz":{"@type":"@vocab"}}}}}}"#;
    let got = run_default(
        r#"{"@context":{"@vocab":"http://example/","Foo":{"@context":{"bar":{"@context":{"baz":{"@type":"@vocab"}}}}}},"@type":"Foo","bar":{"baz":"buzz"}}"#,
        ctx,
    );
    assert_eq!(
        got,
        format!(r#"{{"@context":{},"@type":"Foo","bar":{{"baz":"buzz"}}}}"#, ctx)
    );
}

#[test]
fn type_scoped_context_reverts_for_child_nodes() {
    // A type-scoped context defines "inner"; the CHILD node object must revert to
    // the outer context (previous-context reversion), so the child's property
    // compacts with the outer term, not the scoped one.
    let ctx = r#"{"@version":1.1,"T":{"@id":"http://ex/T","@context":{"p":"http://ex/scoped"}},"p":"http://ex/outer","q":"http://ex/q"}"#;
    let got = run_default(
        r#"[{"@type":["http://ex/T"],"http://ex/scoped":[{"@value":1}],"http://ex/q":[{"http://ex/outer":[{"@value":2}]}]}]"#,
        ctx,
    );
    // At the typed node: http://ex/scoped compacts to "p" (scoped mapping).
    // Inside the child under q: the scoped context has reverted, so
    // http://ex/outer compacts to "p" via the OUTER mapping.
    assert_eq!(
        got,
        format!(r#"{{"@context":{},"@type":"T","p":1,"q":{{"p":2}}}}"#, ctx)
    );
}

// ---------------------------------------------------------------------------
// Base-relative compaction
// ---------------------------------------------------------------------------

#[test]
fn ids_relativize_against_the_base() {
    let mut opts = JsonLdOptions::default();
    opts.base = Some("http://ex/dir/doc".to_string());
    let got = run(
        r#"[{"@id":"http://ex/dir/doc#frag","http://ex/p":[{"@id":"http://ex/dir/sibling"}]}]"#,
        r#"{"p":{"@id":"http://ex/p","@type":"@id"}}"#,
        &opts,
    );
    assert_eq!(
        got,
        r##"{"@context":{"p":{"@id":"http://ex/p","@type":"@id"}},"@id":"#frag","p":"sibling"}"##
    );
}

#[test]
fn keyword_shaped_relative_reference_is_prefixed_with_dot_slash() {
    let mut opts = JsonLdOptions::default();
    opts.base = Some("http://localhost/".to_string());
    let got = run(
        r#"[{"@id":"http://localhost/@special","http://ex/p":[{"@value":1}]}]"#,
        r#"{"ex":"http://example.org/"}"#,
        &opts,
    );
    assert_eq!(
        got,
        r#"{"@context":{"ex":"http://example.org/"},"@id":"./@special","http://ex/p":1}"#
    );
}

#[test]
fn compact_to_relative_false_keeps_absolute_ids() {
    let mut opts = JsonLdOptions::default();
    opts.base = Some("http://ex/dir/doc".to_string());
    opts.compact_to_relative = false;
    let got = run(
        r#"[{"@id":"http://ex/dir/doc#frag","http://ex/p":[{"@value":1}]}]"#,
        r#"{"ex":"http://example.org/"}"#,
        &opts,
    );
    assert_eq!(
        got,
        r#"{"@context":{"ex":"http://example.org/"},"@id":"http://ex/dir/doc#frag","http://ex/p":1}"#
    );
}

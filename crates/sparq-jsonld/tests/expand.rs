//! [OPUS-4.8] (sq-oy1f.25) End-to-end tests for the JSON-LD 1.1 **Expansion Algorithm**,
//! exercised through the crate's public [`sparq_jsonld::expand`] entry. Fixtures are drawn
//! from the worked examples of the W3C JSON-LD 1.1 API + syntax specs
//! (<https://www.w3.org/TR/json-ld11-api/#expansion-algorithms>). No network: expansion runs
//! against the deny-by-default `NoopLoader`.
//!
//! Comparison follows the suite's normative rule — deep JSON equality where **array order is
//! significant only inside `@list`**; every other array (property values, `@type`, `@graph`,
//! `@set`) is compared order-insensitively via the [`canon`] canonicaliser.

use sparq_jsonld::{expand, Json, JsonLdErrorCode, JsonLdOptions, NoopLoader, ProcessingMode};

/// Parses a JSON fixture.
fn json(s: &str) -> Json {
    Json::parse(s).expect("valid JSON fixture")
}

/// Expands `input` with the default options (no base).
fn exp(input: &str) -> Json {
    expand(&json(input), &JsonLdOptions::default(), &NoopLoader).expect("expansion should succeed")
}

/// Expands `input` with an explicit base IRI.
fn exp_base(input: &str, base: &str) -> Json {
    let mut opts = JsonLdOptions::default();
    opts.base = Some(base.to_string());
    expand(&json(input), &opts, &NoopLoader).expect("expansion should succeed")
}

/// Canonicalises a `Json` for the normative comparison: object keys are sorted, and every
/// array is sorted by its canonical serialisation **except** the array under an `@list` key,
/// whose order is preserved.
fn canon(j: &Json) -> Json {
    match j {
        Json::Obj(members) => {
            let mut out: Vec<(String, Json)> = members
                .iter()
                .map(|(k, v)| {
                    let cv = if k == "@list" {
                        canon_ordered(v)
                    } else {
                        canon(v)
                    };
                    (k.clone(), cv)
                })
                .collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Json::Obj(out)
        }
        Json::Arr(items) => {
            let mut c: Vec<Json> = items.iter().map(canon).collect();
            c.sort_by_key(serialize);
            Json::Arr(c)
        }
        other => other.clone(),
    }
}

/// Canonicalises the elements of an ordered array (an `@list` value) **without** reordering.
fn canon_ordered(j: &Json) -> Json {
    match j {
        Json::Arr(items) => Json::Arr(items.iter().map(canon).collect()),
        other => canon(other),
    }
}

/// Serialises a `Json` to its canonical minified string.
fn serialize(j: &Json) -> String {
    let mut s = String::new();
    j.write(&mut s);
    s
}

/// Asserts the expansion of `input` equals `expected` under the normative comparison.
#[track_caller]
fn assert_expands(input: &str, expected: &str) {
    let got = exp(input);
    let want = json(expected);
    assert_eq!(
        canon(&got),
        canon(&want),
        "\n  input:    {}\n  got:      {}\n  expected: {}",
        input,
        serialize(&got),
        serialize(&want)
    );
}

// ---------------------------------------------------------------------------------------
// Core expansion: term IRIs, @id / @type coercion, value objects.
// ---------------------------------------------------------------------------------------

#[test]
fn simple_terms_and_id_coercion() {
    // JSON-LD 1.1 API — the canonical "homepage @type: @id" example.
    assert_expands(
        r#"{
            "@context": {
                "name": "http://schema.org/name",
                "homepage": {"@id": "http://schema.org/url", "@type": "@id"}
            },
            "@id": "http://me.example.com/",
            "name": "Alice",
            "homepage": "http://alice.example.com/"
        }"#,
        r#"[{
            "@id": "http://me.example.com/",
            "http://schema.org/name": [{"@value": "Alice"}],
            "http://schema.org/url": [{"@id": "http://alice.example.com/"}]
        }]"#,
    );
}

#[test]
fn typed_value_object() {
    assert_expands(
        r#"{
            "@context": {"age": {"@id": "http://ex/age", "@type": "http://www.w3.org/2001/XMLSchema#integer"}},
            "age": "42"
        }"#,
        r#"[{"http://ex/age": [{"@value": "42", "@type": "http://www.w3.org/2001/XMLSchema#integer"}]}]"#,
    );
}

#[test]
fn default_language_applied_to_strings() {
    assert_expands(
        r#"{"@context": {"@language": "ja", "term": "http://ex/term"}, "term": "花"}"#,
        r#"[{"http://ex/term": [{"@value": "花", "@language": "ja"}]}]"#,
    );
}

#[test]
fn number_and_boolean_values_have_no_language() {
    // A default @language must NOT be attached to non-string values.
    assert_expands(
        r#"{"@context": {"@language": "en", "n": "http://ex/n", "b": "http://ex/b"}, "n": 42, "b": true}"#,
        r#"[{"http://ex/n": [{"@value": 42}], "http://ex/b": [{"@value": true}]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// Keyword aliases.
// ---------------------------------------------------------------------------------------

#[test]
fn keyword_aliases_for_id_and_type() {
    assert_expands(
        r#"{
            "@context": {"id": "@id", "type": "@type", "Person": "http://schema.org/Person"},
            "id": "http://ex/me",
            "type": "Person"
        }"#,
        r#"[{"@id": "http://ex/me", "@type": ["http://schema.org/Person"]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// @list / @set.
// ---------------------------------------------------------------------------------------

#[test]
fn list_container_preserves_order() {
    assert_expands(
        r#"{"@context": {"foo": {"@id": "http://ex/foo", "@container": "@list"}}, "foo": ["a", "b", "c"]}"#,
        r#"[{"http://ex/foo": [{"@list": [{"@value": "a"}, {"@value": "b"}, {"@value": "c"}]}]}]"#,
    );
}

#[test]
fn set_container_unwraps() {
    assert_expands(
        r#"{"@context": {"foo": {"@id": "http://ex/foo", "@container": "@set"}}, "foo": ["a"]}"#,
        r#"[{"http://ex/foo": [{"@value": "a"}]}]"#,
    );
}

#[test]
fn explicit_list_object() {
    assert_expands(
        r#"{"http://ex/foo": {"@list": ["a", "b"]}}"#,
        r#"[{"http://ex/foo": [{"@list": [{"@value": "a"}, {"@value": "b"}]}]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// @graph, top-level reduction, scalar drops.
// ---------------------------------------------------------------------------------------

#[test]
fn top_level_graph_reduces() {
    assert_expands(
        r#"{"@context": {"foo": "http://ex/foo"}, "@graph": [{"@id": "http://ex/1", "foo": "bar"}]}"#,
        r#"[{"@id": "http://ex/1", "http://ex/foo": [{"@value": "bar"}]}]"#,
    );
}

#[test]
fn top_level_scalar_expands_to_empty_array() {
    assert_eq!(exp("42"), Json::Arr(vec![]));
    assert_eq!(exp("null"), Json::Arr(vec![]));
    assert_eq!(exp("\"a string\""), Json::Arr(vec![]));
}

#[test]
fn free_floating_id_only_node_is_dropped() {
    // A top-level node consisting only of @id projects to no output.
    assert_eq!(exp(r#"{"@id": "http://ex/1"}"#), Json::Arr(vec![]));
}

// ---------------------------------------------------------------------------------------
// @reverse.
// ---------------------------------------------------------------------------------------

#[test]
fn reverse_property_term() {
    assert_expands(
        r#"{
            "@context": {"parentOf": {"@reverse": "http://ex/childOf"}},
            "@id": "http://ex/parent",
            "parentOf": {"@id": "http://ex/child"}
        }"#,
        r#"[{
            "@id": "http://ex/parent",
            "@reverse": {"http://ex/childOf": [{"@id": "http://ex/child"}]}
        }]"#,
    );
}

#[test]
fn reverse_keyword() {
    assert_expands(
        r#"{
            "@id": "http://ex/a",
            "@reverse": {"http://ex/p": {"@id": "http://ex/b"}}
        }"#,
        r#"[{"@id": "http://ex/a", "@reverse": {"http://ex/p": [{"@id": "http://ex/b"}]}}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// Scoped contexts.
// ---------------------------------------------------------------------------------------

#[test]
fn property_scoped_context() {
    assert_expands(
        r#"{
            "@context": {"foo": {"@id": "http://ex/foo", "@context": {"bar": "http://ex/bar"}}},
            "foo": {"bar": "baz"}
        }"#,
        r#"[{"http://ex/foo": [{"http://ex/bar": [{"@value": "baz"}]}]}]"#,
    );
}

#[test]
fn type_scoped_context() {
    assert_expands(
        r#"{
            "@context": {
                "Foo": {"@id": "http://ex/Foo", "@context": {"bar": "http://ex/bar"}},
                "type": "@type"
            },
            "type": "Foo",
            "bar": "baz"
        }"#,
        r#"[{"@type": ["http://ex/Foo"], "http://ex/bar": [{"@value": "baz"}]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// @nest.
// ---------------------------------------------------------------------------------------

#[test]
fn nest_flattens_into_parent() {
    assert_expands(
        r#"{
            "@context": {"nest": "@nest", "name": "http://ex/name", "age": "http://ex/age"},
            "@id": "http://ex/1",
            "name": "Alice",
            "nest": {"age": "30"}
        }"#,
        r#"[{
            "@id": "http://ex/1",
            "http://ex/name": [{"@value": "Alice"}],
            "http://ex/age": [{"@value": "30"}]
        }]"#,
    );
}

// ---------------------------------------------------------------------------------------
// Container maps: @index / @id / @type / @language.
// ---------------------------------------------------------------------------------------

#[test]
fn index_map() {
    assert_expands(
        r#"{"@context": {"foo": {"@id": "http://ex/foo", "@container": "@index"}}, "foo": {"a": "1", "b": "2"}}"#,
        r#"[{"http://ex/foo": [{"@value": "1", "@index": "a"}, {"@value": "2", "@index": "b"}]}]"#,
    );
}

#[test]
fn id_map() {
    assert_expands(
        r#"{
            "@context": {"foo": {"@id": "http://ex/foo", "@container": "@id"}, "name": "http://ex/name"},
            "foo": {"http://ex/1": {"name": "Alice"}}
        }"#,
        r#"[{"http://ex/foo": [{"@id": "http://ex/1", "http://ex/name": [{"@value": "Alice"}]}]}]"#,
    );
}

#[test]
fn type_map() {
    assert_expands(
        r#"{
            "@context": {"foo": {"@id": "http://ex/foo", "@container": "@type"}, "name": "http://ex/name"},
            "foo": {"http://ex/T": {"name": "Alice"}}
        }"#,
        r#"[{"http://ex/foo": [{"@type": ["http://ex/T"], "http://ex/name": [{"@value": "Alice"}]}]}]"#,
    );
}

#[test]
fn language_map() {
    assert_expands(
        r#"{
            "@context": {"label": {"@id": "http://ex/label", "@container": "@language"}},
            "label": {"en": "Hello", "de": "Hallo"}
        }"#,
        r#"[{"http://ex/label": [{"@value": "Hello", "@language": "en"}, {"@value": "Hallo", "@language": "de"}]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// Property-valued index containers (`@container: @index` + `"@index": <prop>`) and the
// expanded-index @none guards — mirrors of W3C expand suite cases pi05/pi06/pi07/pi10/m012
// (§5.1.2 steps 13.8.3.7–13.8.3.10).
// ---------------------------------------------------------------------------------------

#[test]
fn property_valued_index_becomes_property_value() {
    // W3C expand/pi06: the index of a property-valued index map re-expands (Value
    // Expansion, with the index key as active property) as a value of that property,
    // instead of `@index`.
    assert_expands(
        r#"{
            "@context": {
                "@base": "http://example.com/",
                "@vocab": "http://example.com/",
                "author": {"@type": "@id", "@container": "@index", "@index": "prop"}
            },
            "@id": "article",
            "author": {"regular": "person/1", "guest": ["person/2", "person/3"]}
        }"#,
        r#"[{
            "@id": "http://example.com/article",
            "http://example.com/author": [
                {"@id": "http://example.com/person/1", "http://example.com/prop": [{"@value": "regular"}]},
                {"@id": "http://example.com/person/2", "http://example.com/prop": [{"@value": "guest"}]},
                {"@id": "http://example.com/person/3", "http://example.com/prop": [{"@value": "guest"}]}
            ]
        }]"#,
    );
}

#[test]
fn property_valued_index_precedes_existing_values() {
    // §5.1.2 step 13.8.3.7.3: index property values are "an array consisting of re-expanded
    // index FOLLOWED BY the existing values" — the suite's pi07 expected output shows the
    // same ordering. The shared `canon` comparison is order-insensitive, so this asserts on
    // the serialised property array directly.
    let got = exp(r#"{
        "@context": {
            "@base": "http://example.com/",
            "@vocab": "http://example.com/",
            "author": {"@type": "@id", "@container": "@index", "@index": "prop"}
        },
        "@id": "article",
        "author": {"regular": {"@id": "person/1", "prop": "foo"}}
    }"#);
    let s = serialize(&got);
    assert!(
        s.contains(r#""http://example.com/prop":[{"@value":"regular"},{"@value":"foo"}]"#),
        "re-expanded index must precede the item's existing values, got: {s}",
    );
}

#[test]
fn property_valued_index_none_key_adds_no_property() {
    // W3C expand/pi10: an `@none` index adds NO property (step 13.8.3.7 requires "expanded
    // index is not @none"). The `guest` entry also pins the `@type: @vocab` re-expansion of
    // the index into a vocab-IRI node reference.
    assert_expands(
        r#"{
            "@context": {
                "@base": "http://example.com/",
                "@vocab": "http://example.com/",
                "author": {"@type": "@id", "@container": "@index", "@index": "prop"},
                "prop": {"@type": "@vocab"}
            },
            "@id": "http://example.com/article",
            "author": {"@none": {"@id": "person/1"}, "guest": [{"@id": "person/2"}]}
        }"#,
        r#"[{
            "@id": "http://example.com/article",
            "http://example.com/author": [
                {"@id": "http://example.com/person/1"},
                {"@id": "http://example.com/person/2",
                 "http://example.com/prop": [{"@id": "http://example.com/guest"}]}
            ]
        }]"#,
    );
}

#[test]
fn property_valued_index_on_value_object_errors() {
    // W3C expand/pi05: adding the index property to a value object is an
    // "invalid value object" error (§5.1.2 step 13.8.3.7.5).
    assert_error(
        r#"{
            "@context": {
                "@vocab": "http://example.com/",
                "container": {"@id": "http://example.com/container", "@container": "@index", "@index": "prop"}
            },
            "@id": "http://example.com/annotationsTest",
            "container": {"en": "The Queen"}
        }"#,
        JsonLdErrorCode::InvalidValueObject,
    );
}

#[test]
fn type_map_none_alias_adds_no_type() {
    // W3C expand/m012: the @none guards compare the EXPANDED index, so a term aliased to
    // `@none` used as a type-map key adds no `@type` (step 13.8.3.10).
    assert_expands(
        r#"{
            "@context": {"@vocab": "http://example/", "typemap": {"@container": "@type"}, "none": "@none"},
            "typemap": {"@none": {"label": "a"}, "none": {"label": "b"}}
        }"#,
        r#"[{
            "http://example/typemap": [
                {"http://example/label": [{"@value": "a"}]},
                {"http://example/label": [{"@value": "b"}]}
            ]
        }]"#,
    );
}

#[test]
fn type_map_key_precedes_existing_types() {
    // §5.1.2 step 13.8.3.10: "types … consisting of expanded index FOLLOWED BY any existing
    // values of @type" — the suite's m004 expected output shows the same ordering.
    let got = exp(r#"{
        "@context": {"@vocab": "http://example/", "typemap": {"@container": "@type"}},
        "typemap": {"_:bar": {"@type": "_:foo", "label": "x"}}
    }"#);
    let s = serialize(&got);
    assert!(
        s.contains(r#""@type":["_:bar","_:foo"]"#),
        "the type-map key must precede the item's own @type values, got: {s}",
    );
}

#[test]
fn id_coercion_keyword_shaped_scalar_and_arrays_retain_null_id() {
    // [SONNET-4.6] PR #4132 review: §5.3.2 returns the @id map even when IRI Expansion is null.
    assert_expands(
        r#"{"@context": {"p": {"@id": "http://ex/p", "@type": "@id"}}, "p": "@kw"}"#,
        r#"[{"http://ex/p": [{"@id": null}]}]"#,
    );
    assert_expands(
        r#"{"@context": {"p": {"@id": "http://ex/p", "@type": "@id"}}, "p": ["@kw"]}"#,
        r#"[{"http://ex/p": [{"@id": null}]}]"#,
    );
    assert_expands(
        r#"{"@context": {"p": {"@id": "http://ex/p", "@type": "@id"}}, "p": ["@kw", "http://ex/ok"]}"#,
        r#"[{"http://ex/p": [{"@id": null}, {"@id": "http://ex/ok"}]}]"#,
    );
}

#[test]
fn nulled_term_under_id_and_vocab_coercion_is_pinned() {
    // A null term mapping is consulted only by vocabulary-relative IRI Expansion (§5.2 step 5).
    assert_expands(
        r#"{
            "@context": {
                "nt": null,
                "id": {"@id": "http://ex/id", "@type": "@id"},
                "vocab": {"@id": "http://ex/vocab", "@type": "@vocab"}
            },
            "id": "nt",
            "vocab": "nt"
        }"#,
        r#"[{
            "http://ex/id": [{"@id": "nt"}],
            "http://ex/vocab": [{"@id": null}]
        }]"#,
    );
}

#[test]
fn property_index_null_re_expansion_on_value_object_errors() {
    for index in ["@kw", "real"] {
        let input = format!(
            r#"{{
                "@context": {{
                    "@vocab": "http://ex/",
                    "items": {{"@container": "@index", "@index": "kind"}},
                    "kind": {{"@type": "@vocab"}}
                }},
                "items": {{"{}": "kept"}}
            }}"#,
            index,
        );
        assert_error(&input, JsonLdErrorCode::InvalidValueObject);
    }
}

// ---------------------------------------------------------------------------------------
// @json literals.
// ---------------------------------------------------------------------------------------

#[test]
fn json_literal_keeps_value_verbatim() {
    assert_expands(
        r#"{"@context": {"e": {"@id": "http://ex/e", "@type": "@json"}}, "e": {"foo": ["bar", 1, true]}}"#,
        r#"[{"http://ex/e": [{"@value": {"foo": ["bar", 1, true]}, "@type": "@json"}]}]"#,
    );
}

// ---------------------------------------------------------------------------------------
// Null drops + array normalisation + relative IRIs.
// ---------------------------------------------------------------------------------------

#[test]
fn null_values_are_dropped() {
    assert_expands(
        r#"{"@context": {"foo": "http://ex/foo"}, "foo": null, "http://ex/bar": ["a", null, "b"]}"#,
        r#"[{"http://ex/bar": [{"@value": "a"}, {"@value": "b"}]}]"#,
    );
}

#[test]
fn relative_iris_resolve_against_base() {
    assert_eq!(
        canon(&exp_base(
            r#"{"@id": "foo", "http://ex/p": "v"}"#,
            "http://example.org/dir/"
        )),
        canon(&json(
            r#"[{"@id": "http://example.org/dir/foo", "http://ex/p": [{"@value": "v"}]}]"#
        ))
    );
}

// ---------------------------------------------------------------------------------------
// Negative tests: exact spec error codes.
// ---------------------------------------------------------------------------------------

/// Expands `input`, expecting a specific error code.
#[track_caller]
fn assert_error(input: &str, code: JsonLdErrorCode) {
    let err = expand(&json(input), &JsonLdOptions::default(), &NoopLoader)
        .expect_err("expansion should fail");
    assert_eq!(err.code(), code, "input: {}", input);
}

#[test]
fn invalid_id_value() {
    assert_error(r#"{"@id": 42, "http://ex/p": "v"}"#, JsonLdErrorCode::InvalidIdValue);
}

#[test]
fn colliding_keywords() {
    assert_error(
        r#"{"@context": {"id": "@id"}, "@id": "http://ex/1", "id": "http://ex/2"}"#,
        JsonLdErrorCode::CollidingKeywords,
    );
}

#[test]
fn invalid_value_object_disallowed_key() {
    assert_error(
        r#"{"http://ex/p": {"@value": "x", "@id": "http://ex/y"}}"#,
        JsonLdErrorCode::InvalidValueObject,
    );
}

#[test]
fn invalid_value_object_value() {
    // A @value with a non-scalar (and no @json type) is invalid.
    assert_error(
        r#"{"http://ex/p": {"@value": {"nested": "object"}}}"#,
        JsonLdErrorCode::InvalidValueObjectValue,
    );
}

#[test]
fn invalid_language_tagged_string() {
    assert_error(
        r#"{"http://ex/p": {"@value": "x", "@language": 42}}"#,
        JsonLdErrorCode::InvalidLanguageTaggedString,
    );
}

#[test]
fn invalid_type_value() {
    assert_error(
        r#"{"@type": {"not": "a string"}, "http://ex/p": "v"}"#,
        JsonLdErrorCode::InvalidTypeValue,
    );
}

#[test]
fn invalid_reverse_value() {
    assert_error(
        r#"{"@reverse": "not a map"}"#,
        JsonLdErrorCode::InvalidReverseValue,
    );
}

#[test]
fn invalid_nest_value() {
    assert_error(
        r#"{"@context": {"nest": "@nest"}, "nest": "not a map"}"#,
        JsonLdErrorCode::InvalidNestValue,
    );
}

#[test]
fn colliding_value_and_language_type() {
    // A value object may not carry both @type and @language.
    assert_error(
        r#"{"http://ex/p": {"@value": "x", "@type": "http://ex/T", "@language": "en"}}"#,
        JsonLdErrorCode::InvalidValueObject,
    );
}

// ---------------------------------------------------------------------------------------
// [FABLE-5] sq-oy1f.37 — expand-lane correctness regressions (W3C expand cases the native
// document-level oracle exposed). Each is NON-VACUOUS: it fails on the pre-fix expander.
// ---------------------------------------------------------------------------------------

#[test]
fn value_object_type_collapses_to_scalar_via_context_term() {
    // A value object's `@type` is a SINGLE value in the JSON-LD data model.
    // The general keyword path arrayifies `@type`; expansion must collapse the
    // single-element array back to a scalar (W3C expand/0002). Pre-fix the
    // expander raised `invalid typed value` on the arrayified `@type`.
    assert_expands(
        r#"{"@context": {"t2": "http://example.com/t2", "term2": "http://example.com/term2"},
            "term2": {"@value": "v2", "@type": "t2"}}"#,
        r#"[{"http://example.com/term2": [{"@value": "v2", "@type": "http://example.com/t2"}]}]"#,
    );
}

#[test]
fn value_object_type_scalar_for_already_absolute_iri() {
    // The same collapse when `@type` is an already-absolute IRI (W3C expand/0013).
    assert_expands(
        r#"[{"@id": "http://example.com/id1",
             "http://example.com/term2": [{"@value": "v2", "@type": "http://example.com/t2"}]}]"#,
        r#"[{"@id": "http://example.com/id1",
             "http://example.com/term2": [{"@value": "v2", "@type": "http://example.com/t2"}]}]"#,
    );
}

#[test]
fn set_container_empty_array_property_is_retained() {
    // A `@set`-container term whose value expands to an empty array must RETAIN
    // the property as `[]` — the `addValue` `asArray=true` rule (W3C expand/0004,
    // 0015). Pre-fix the property was dropped by the plain empty-array skip.
    assert_expands(
        r#"{"@context": {"myset": {"@id": "http://example.com/myset", "@container": "@set"}},
            "@id": "http://example.org/id",
            "myset": {"@set": []}}"#,
        r#"[{"@id": "http://example.org/id", "http://example.com/myset": []}]"#,
    );
}

#[test]
fn plain_empty_array_property_is_retained() {
    // A plain empty-array property value (no container mapping) is likewise
    // retained as `[]` on the forward-property path (W3C expand/0004 set3).
    assert_expands(
        r#"{"@id": "http://example.org/id", "http://example.org/set3": []}"#,
        r#"[{"@id": "http://example.org/id", "http://example.org/set3": []}]"#,
    );
}

#[test]
fn free_floating_value_object_is_dropped() {
    // A top-level free-floating value object (active property null) is dropped —
    // the whole document reduces to `[]` (W3C expand/0045).
    assert_expands(r#"{"@value": "free-floating value"}"#, r#"[]"#);
}

#[test]
fn free_floating_value_objects_under_graph_are_dropped() {
    // Free-floating value objects (even language-tagged / typed) directly under
    // `@graph` are dropped; a `@graph` of only such values reduces to `[]`
    // (W3C expand/0046).
    assert_expands(
        r#"{"@graph": [
            {"@value": "plain"},
            {"@value": "tagged", "@language": "en"},
            {"@value": "typed", "@type": "http://example.com/type"}
        ]}"#,
        r#"[]"#,
    );
}

// ---------------------------------------------------------------------------------------
// [OPUS-5] sq-gzsky — the W3C `expand` NegativeEvaluationTest lane is now RUN rather than
// skipped, which required these seven spec obligations to be enforced. Each test names the
// suite case it pins; they live here (not only in the conformance lane) because the W3C
// suite is fetched, gitignored, and absent from a fresh checkout — these run everywhere.
// ---------------------------------------------------------------------------------------

/// Expands `input` in JSON-LD 1.0 processing mode, expecting a specific error code.
#[track_caller]
fn assert_error_1_0(input: &str, code: JsonLdErrorCode) {
    let mut opts = JsonLdOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    let err = expand(&json(input), &opts, &NoopLoader).expect_err("expansion should fail");
    assert_eq!(err.code(), code, "input: {}", input);
}

#[test]
fn included_value_must_be_a_node_object() {
    // §5.1.2 step 13.4.13: the expansion of `@included` is arrayified and EVERY element
    // must be a node object. A string, a value object, and a list object each expand to a
    // DROPPED (null) result under the null active property — arrayifying that yields one
    // non-node element, so all three raise (W3C expand/in07, in08, in09). Collapsing the
    // drop to an empty array instead would vacuously accept them.
    for included in [r#""string""#, r#"{"@value": "value"}"#, r#"{"@list": ["value"]}"#] {
        assert_error(
            &format!(
                r#"{{"@context": {{"@version": 1.1, "@vocab": "http://example.org/"}},
                     "@included": {included}}}"#
            ),
            JsonLdErrorCode::InvalidIncludedValue,
        );
    }
}

#[test]
fn included_node_object_still_expands() {
    // The complement of the above: a real node object under `@included` is NOT an error.
    assert_expands(
        r#"{"@context": {"@version": 1.1, "@vocab": "http://example.org/"},
            "@id": "http://example.org/s",
            "@included": {"@id": "http://example.org/o", "p": "v"}}"#,
        r#"[{"@id": "http://example.org/s",
             "@included": [{"@id": "http://example.org/o",
                            "http://example.org/p": [{"@value": "v"}]}]}]"#,
    );
}

#[test]
fn value_object_type_and_direction_are_exclusive() {
    // §5.1.2 step 15.1: a value object "must not contain an `@type` entry if it contains
    // either `@language` or `@direction`". The `@language` half was already enforced; this
    // pins the `@direction` half (W3C expand/di09).
    assert_error(
        r#"{"ex:p": {"@value": "v", "@type": "ex:t", "@direction": "rtl"}}"#,
        JsonLdErrorCode::InvalidValueObject,
    );
}

#[test]
fn datatype_iri_with_a_space_is_rejected() {
    // §5.1.2 step 15.4: "Processors MUST validate datatype IRIs". A scheme-valid string
    // carrying a SPACE is not an IRI (W3C expand/0123). Scheme validity alone accepted it.
    assert_error(
        r#"{"@id": "http://example.com/foo",
            "http://example.com/bar": {"@value": "bar", "@type": "http://example.com/baz z"}}"#,
        JsonLdErrorCode::InvalidTypedValue,
    );
}

#[test]
fn datatype_iri_with_a_malformed_percent_escape_is_rejected() {
    // [SONNET-4.6] (PR #4610 review) §5.1.2 step 15.4 validates the datatype as an IRI, and
    // `pct-encoded = "%" HEXDIG HEXDIG` — a non-hex or truncated escape is not one. The
    // first cut of the check was a character denylist, which accepted all of these.
    for datatype in [
        "http://example.com/%ZZ",
        "http://example.com/%",
        "http://example.com/%A",
    ] {
        assert_error(
            &format!(
                r#"{{"http://example.com/bar": {{"@value": "bar", "@type": "{datatype}"}}}}"#
            ),
            JsonLdErrorCode::InvalidTypedValue,
        );
    }
}

#[test]
fn datatype_iri_with_a_structurally_invalid_authority_is_rejected() {
    // [SONNET-4.6] (PR #4610 review round 2) §5.1.2 step 15.4 validates the datatype as an
    // IRI, which is the RFC 3987 `IRI` production — not merely a scheme plus admitted
    // characters. Every string below is built ENTIRELY from characters an IRI may carry, so
    // the round-one code-point check accepted all of them; each is structurally malformed in
    // the authority (`ihost` / `port`).
    for datatype in [
        "http://[",                       // unterminated IP-literal
        "http://[::1",                    // ...likewise
        "http://a[b]c/",                  // brackets outside an IP-literal
        "http://example.com:bad-port",    // port = *DIGIT
        "http://example.com:80a/",        // ...likewise
        "http://[gggg::1]/",              // non-HEXDIG in an IPv6 group
        "http://[1:2:3:4:5:6:7]/",        // seven groups, no `::` elision
        "http://[::ffff:999.1.1.1]/",     // dec-octet out of range
    ] {
        assert_error(
            &format!(
                r#"{{"http://example.com/bar": {{"@value": "bar", "@type": "{datatype}"}}}}"#
            ),
            JsonLdErrorCode::InvalidTypedValue,
        );
    }
}

#[test]
fn structurally_valid_authorities_and_authority_less_schemes_are_accepted_datatypes() {
    // [SONNET-4.6] (PR #4610 review round 2) The complement of the negatives above: adding
    // the structural grammar must not over-reject. Covers each `ihost` alternative
    // (IP-literal, IPv4address, ireg-name), userinfo/port, and the authority-less schemes
    // (`ipath-rootless`) that carry no `//` at all.
    for datatype in [
        "http://[::1]/dt",
        "http://[2001:db8::1]:8080/dt",
        "http://[::ffff:192.168.0.1]/dt",
        "http://192.168.0.1:80/dt",
        "http://user:pass@example.com:8080/dt",
        "urn:uuid:6e8bc430-9c3a-11d9-9669-0800200c9a66",
        "did:example:123#key-1",
        "tag:example.com,2026:dt",
    ] {
        let doc = format!(
            r#"{{"http://example.com/bar": {{"@value": "bar", "@type": "{datatype}"}}}}"#
        );
        let expected = format!(
            r#"[{{"http://example.com/bar": [{{"@value": "bar", "@type": "{datatype}"}}]}}]"#
        );
        assert_expands(&doc, &expected);
    }
}

#[test]
fn well_formed_percent_encoded_and_unicode_datatype_iris_are_accepted() {
    // The complement of the two negatives above: datatype validation must not over-reject a
    // well-formed escape or a native RFC 3987 `ucschar` IRI — every `is_absolute_iri` call
    // site in context processing shares this predicate.
    assert_expands(
        r#"{"http://example.com/bar": {"@value": "bar", "@type": "http://example.com/a%20b"}}"#,
        r#"[{"http://example.com/bar":
              [{"@value": "bar", "@type": "http://example.com/a%20b"}]}]"#,
    );
    assert_expands(
        "{\"http://example.com/bar\": {\"@value\": \"bar\",
            \"@type\": \"http://\u{4f8b}\u{3048}.jp/na\u{ef}ve\"}}",
        "[{\"http://example.com/bar\":
             [{\"@value\": \"bar\", \"@type\": \"http://\u{4f8b}\u{3048}.jp/na\u{ef}ve\"}]}]",
    );
}

#[test]
fn blank_node_datatype_is_rejected() {
    // §5.1.2 step 15.4: the value of `@type` must be an IRI, and a blank node identifier
    // is not one (W3C expand/er40). Blank-node datatypes are only meaningful under the
    // `GeneralizedRdf` optional feature, which sparq does not opt into.
    assert_error(
        r#"{"http://example/foo": {"@value": "bar", "@type": "_:dt"}}"#,
        JsonLdErrorCode::InvalidTypedValue,
    );
}

#[test]
fn iri_shaped_term_may_not_be_remapped_onto_a_keyword_in_1_1() {
    // §4.2.2 step 14.3.3: a term containing a colon must IRI-expand back to its own IRI
    // mapping. Mapping `rdf:type` onto the keyword `@type` breaks that round trip, so 1.1
    // rejects it (W3C expand/er43). The check was previously skipped whenever the mapping
    // was a keyword — exactly the case the step exists to catch.
    assert_error(
        r#"{"@context": {"http://www.w3.org/1999/02/22-rdf-syntax-ns#type":
                          {"@id": "@type", "@type": "@id"}},
            "@id": "http://example.com/a",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type": "http://example.com/b"}"#,
        JsonLdErrorCode::InvalidIriMapping,
    );
}

#[test]
fn iri_shaped_term_remapped_onto_a_keyword_is_legal_in_1_0() {
    // The 1.0 twin of the case above is a POSITIVE (W3C expand/0026): the round-trip check
    // is absent from the JSON-LD 1.0 algorithm, so the ONLY gate is the processing mode.
    let mut opts = JsonLdOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    let input = json(
        r#"{"@context": {"http://www.w3.org/1999/02/22-rdf-syntax-ns#type":
                          {"@id": "@type", "@type": "@id"}},
            "@id": "http://example.com/a",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type": "http://example.com/b"}"#,
    );
    let got = expand(&input, &opts, &NoopLoader).expect("1.0 mode accepts the remapping");
    assert_eq!(
        canon(&got),
        canon(&json(
            r#"[{"@id": "http://example.com/a", "@type": ["http://example.com/b"]}]"#
        ))
    );
}

#[test]
fn container_array_is_invalid_in_1_0_mode() {
    // §4.2.2 step 21.2: a container value that "is otherwise not a string" is an
    // `invalid container mapping` under `processingMode: json-ld-1.0` — so `["@set"]` is
    // rejected even though the single member `@set` is itself 1.0-legal (W3C expand/es01,
    // compact/ep12). Normalising to a member list alone loses that distinction.
    assert_error_1_0(
        r#"{"@context": {"term": {"@id": "http://example/term", "@container": ["@set"]}},
            "@id": "http://example/test#example", "term": "foo"}"#,
        JsonLdErrorCode::InvalidContainerMapping,
    );
}

#[test]
fn container_string_is_still_valid_in_1_0_mode() {
    // The complement: the bare-string spelling of the same container is 1.0-legal.
    let mut opts = JsonLdOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    let input = json(
        r#"{"@context": {"term": {"@id": "http://example/term", "@container": "@set"}},
            "@id": "http://example/test#example", "term": "foo"}"#,
    );
    expand(&input, &opts, &NoopLoader).expect("a string @container is legal in 1.0");
}

#[test]
fn relative_vocab_is_invalid_in_1_0_mode() {
    // §4.1.2 step 5.8 tests the RAW value: "if value is neither an IRI nor a blank node
    // identifier, an invalid vocab mapping error". Resolving a relative reference (the
    // empty string included) against `@base` is the JSON-LD 1.1 relaxation; in 1.0 both
    // spellings are an error (W3C expand/0115 `""`, expand/0116 `"/relative"`).
    for vocab in [r#""""#, r#""/relative""#] {
        assert_error_1_0(
            &format!(
                r#"{{"@context": {{"@base": "http://example.com/some/deep/directory/and/file/",
                                   "@vocab": {vocab}}},
                     "@id": "relativePropertyIris", "link": "link"}}"#
            ),
            JsonLdErrorCode::InvalidVocabMapping,
        );
    }
}

#[test]
fn relative_vocab_is_accepted_in_1_1_mode() {
    // The 1.1 twin (W3C expand/0112 lineage): the same relative `@vocab` resolves against
    // `@base` instead of erroring, so the 1.0 rejection above must NOT leak into 1.1.
    assert_expands(
        r#"{"@context": {"@base": "http://example.com/deep/", "@vocab": ""},
            "@id": "relativeIri", "link": "value"}"#,
        r#"[{"@id": "http://example.com/deep/relativeIri",
             "http://example.com/deep/link": [{"@value": "value"}]}]"#,
    );
}

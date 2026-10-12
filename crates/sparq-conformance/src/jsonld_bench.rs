//! [FABLE-5] sq-hmd7l.15 — shared JSON-LD comparison helpers for the W3C
//! conformance lane (`tests/jsonld_suite/`) and the comparative-benchmark
//! harness (`bench/jsonld/`, the `bench_jsonld` example).
//!
//! Behind the OPT-IN `jsonld-suite` feature (OFF by default) because the
//! document-level bridge needs the optional `sparq-jsonld` / `oxjsonld` deps —
//! the default harness build links none of this, the lean opt-in posture.
//!
//! The comparator (`json_ld_equal`) and the AST bridge (`sparq_json_to_serde`)
//! moved here VERBATIM from `tests/jsonld_suite/common.rs` (which now
//! re-exports them) so the conformance ratchet and the bench harness's
//! output-equality gate share ONE comparator — the load-bearing invariant of
//! `bench/jsonld` is "no throughput row without output-equality agreement",
//! and that agreement must mean exactly what a conformance pass means.

use oxrdf::dataset::CanonicalizationAlgorithm;
use oxrdf::{Dataset, Quad};
use serde_json::Value;

// ── JSON-LD document-level equality comparator ──────────────────────────────
//
// [SONNET-4.6] sq-kk1mq — the comparator for the native expand() oracle.
//
// Semantics (PINNED IN CODE per sq-kk1mq):
//   • Object key order: insignificant.  Two objects are equal iff they have the
//     same keys with deeply-equal values.
//   • Array element order: SIGNIFICANT when the array is the **direct value of a
//     `"@list"` key** (its elements revert to the default below), and
//     RECURSIVELY inside the `@value` payload of a `@json` literal
//     (`{"@value": …, "@type": "@json"}`), which is opaque JSON where `[1,2]` and
//     `[2,1]` are different documents (#4264).  Insignificant everywhere else
//     (multiset / set semantics): in expanded JSON-LD `@list` is the only
//     structured-sequence construct; all other arrays are bags.
//   • Numbers: integral (i64/u64-representable) compared exactly so integers
//     ≥ 2^53 remain distinct; non-integral (either side is a JSON float) fall
//     back to f64, so `1` ≡ `1.0`.  [SONNET-4.6] sq-kk1mq numeric-guard.
//   • Strings, booleans, null: exact equality.
//
// Why document-level rather than RDF-level?  The expansion algorithm operates
// purely at the JSON-LD document level.  Two expansions can produce the same
// RDF yet differ in JSON structure (e.g. `@direction` handling, `@json` literals,
// `@list` vs plain-array shapes) — the RDF oracle missed those differences.
//
// PRECISION NOTE: this comparator measures JSON-LD data-model (semantic)
// equivalence — order-insensitive outside `@list` — NOT structural identity
// with the reference output.  See `floors::expand` for the honest ordered
// vs unordered pass-count breakdown on the W3C suite.

/// How the array immediately under comparison must be compared: the JSON-LD
/// data model gives arrays three different meanings depending on where they sit.
/// Same three-way mode as `sparq-jsonld/tests/common/mod.rs` (#4226 / #4264),
/// restated over `serde_json::Value`.
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

/// Returns `true` iff `a` and `b` are equal under the JSON-LD comparison rules
/// described in the module-level comment.
///
/// [SONNET-4.6] sq-kk1mq
pub fn json_ld_equal(a: &Value, b: &Value) -> bool {
    json_ld_equal_inner(a, b, Arrays::Unordered)
}

/// Recursive inner: `arrays` says how an array at THIS position compares.
fn json_ld_equal_inner(a: &Value, b: &Value, arrays: Arrays) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            // JSON-LD numeric equality [SONNET-4.6] sq-kk1mq numeric-guard:
            // • Both i64-representable: compare exactly so integers ≥ 2^53 are
            //   not wrongly collapsed by f64 rounding
            //   (e.g. 9007199254740992 ≢ 9007199254740993 must remain distinct).
            // • Both u64-representable (one side exceeds i64::MAX): compare exactly.
            // • Otherwise: fall back to f64 so integer JSON `1` ≡ float JSON `1.0`.
            if let (Some(xi), Some(yi)) = (x.as_i64(), y.as_i64()) {
                xi == yi
            } else if let (Some(xu), Some(yu)) = (x.as_u64(), y.as_u64()) {
                xu == yu
            } else {
                match (x.as_f64(), y.as_f64()) {
                    (Some(xf), Some(yf)) => xf == yf,
                    // Fallback for numbers outside f64 range (unlikely in JSON-LD).
                    _ => x == y,
                }
            }
        }
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(xs), Value::Array(ys)) => {
            if xs.len() != ys.len() {
                return false;
            }
            match arrays {
                // Set semantics: order INSIGNIFICANT (multiset match).
                Arrays::Unordered => json_ld_array_equal_unordered(xs, ys),
                // @list: element order is SIGNIFICANT; the elements are JSON-LD
                // values again, so their own arrays revert to set semantics.
                Arrays::ListOrdered => xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| json_ld_equal_inner(x, y, Arrays::Unordered)),
                // Inside a @json payload the exactness propagates all the way down.
                Arrays::JsonExact => xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| json_ld_equal_inner(x, y, Arrays::JsonExact)),
            }
        }
        (Value::Object(xa), Value::Object(ya)) => {
            if xa.len() != ya.len() {
                return false;
            }
            let json_literal = arrays != Arrays::JsonExact && is_json_literal(xa);
            for (k, va) in xa {
                let Some(vb) = ya.get(k) else {
                    return false;
                };
                let child = if arrays == Arrays::JsonExact {
                    // An opaque payload: `@list` / `@type` are ordinary member names.
                    Arrays::JsonExact
                } else if json_literal && k == "@value" {
                    Arrays::JsonExact
                } else if k == "@list" {
                    // The array VALUE of a "@list" key is ORDER-SIGNIFICANT.
                    Arrays::ListOrdered
                } else {
                    Arrays::Unordered
                };
                if !json_ld_equal_inner(va, vb, child) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Is this object an expanded `@json` value object (`{"@value":…,"@type":"@json"}`)?
/// Expansion emits the `@type` as a bare string; a single-element array is
/// accepted too so the check does not depend on that normalisation.
fn is_json_literal(members: &serde_json::Map<String, Value>) -> bool {
    match members.get("@type") {
        Some(Value::String(s)) => s == "@json",
        Some(Value::Array(a)) => matches!(a.as_slice(), [Value::String(s)] if s == "@json"),
        _ => false,
    }
}

/// Multiset-equality for arrays under set semantics: every element of `xs` must
/// match exactly one unused element of `ys`, regardless of position.
/// O(n²) but conformance-suite arrays are small (typically ≤ 10 elements).
fn json_ld_array_equal_unordered(xs: &[Value], ys: &[Value]) -> bool {
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

/// Convert a `sparq_jsonld::Json` value to a `serde_json::Value` by
/// round-tripping through a JSON string.  Used to bridge the two ASTs so the
/// `json_ld_equal` comparator can operate on both the expand() output and the
/// expected-document JSON that `serde_json::from_str` produces.
pub fn sparq_json_to_serde(j: &sparq_jsonld::Json) -> Result<Value, String> {
    let mut buf = String::new();
    j.write(&mut buf);
    serde_json::from_str(&buf)
        .map_err(|e| format!("parse serialized JSON as serde_json::Value: {}", e))
}

/// Parse an N-Quads document into a canonicalized oxrdf `Dataset` for
/// isomorphic (blank-node-blind) comparison.
pub fn nquads_to_canonical_dataset(text: &str) -> Result<Dataset, String> {
    let mut ds = Dataset::new();
    for q in oxttl::NQuadsParser::new().for_slice(text.as_bytes()) {
        let q: Quad = q.map_err(|e| e.to_string())?;
        ds.insert(&q);
    }
    ds.canonicalize(CanonicalizationAlgorithm::Unstable);
    Ok(ds)
}

/// Parse a JSON-LD document (through `oxjsonld`, the real ingest parser) into a
/// canonicalized oxrdf `Dataset`.
pub fn jsonld_to_canonical_dataset(doc: &str, base: &str) -> Result<Dataset, String> {
    let mut ds = Dataset::new();
    let parser = oxjsonld::JsonLdParser::new()
        .with_base_iri(base)
        .map_err(|e| format!("invalid base {base:?}: {e}"))?;
    for q in parser.for_slice(doc.as_bytes()) {
        let q = q.map_err(|e| e.to_string())?;
        ds.insert(&q);
    }
    ds.canonicalize(CanonicalizationAlgorithm::Unstable);
    Ok(ds)
}

/// [OPUS-4.8] sq-3uos5 (moved from `tests/jsonld_suite/common.rs`, sq-hmd7l.15) —
/// read a context document and extract the caller `@context` value as the
/// engine writer's `JsonLdValue`. The wrapping file holds `{"@context": …}`;
/// sparq's `graph_to_jsonld_compact` expects the INNER value (the
/// term-definition object), so one `@context` layer is unwrapped. When the
/// inner value is an object it is handed straight to the writer; an
/// array/string form (a remote-context reference or multi-context array) is NOT
/// resolved here (no network, and the writer takes a single inline object) —
/// the caller SKIPS such cases.
pub fn read_context_member(
    path: &std::path::Path,
) -> Result<sparq_engine::serialize::JsonLdValue, String> {
    use sparq_engine::serialize::{parse_context_json, JsonLdValue};
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("read context {}: {e}", path.display()))?;
    // Parse the whole context document with the writer's own tiny JSON reader,
    // then pull out the `@context` member (the value the writer compacts against).
    let doc =
        parse_context_json(&text).ok_or_else(|| "context file is not a JSON object".to_string())?;
    match doc {
        JsonLdValue::Obj(members) => {
            let inner = members
                .into_iter()
                .find(|(k, _)| k == "@context")
                .map(|(_, v)| v)
                .ok_or_else(|| "context file has no @context member".to_string())?;
            // Only a single inline object context is drivable through the writer.
            match inner {
                JsonLdValue::Obj(_) => Ok(inner),
                _ => Err("non-object @context (array/string/remote) — not drivable".to_string()),
            }
        }
        _ => Err("context document is not an object".to_string()),
    }
}

/// Serialise an oxrdf `Dataset` to N-Quads text (one canonical `Quad` Display
/// per line). Loss-free: `oxrdf`'s `Quad` Display is canonical N-Quads.
pub fn dataset_to_nquads(ds: &Dataset) -> String {
    let mut out = String::new();
    for q in ds.iter() {
        let owned: Quad = q.into_owned();
        // [OPUS-4.8] positional format arg (avoids the CodeQL rust/unused-variable
        // false positive on inline-captured identifiers).
        out.push_str(&format!("{} .\n", owned));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── json_ld_equal unit tests (sq-kk1mq; moved with the fn, sq-hmd7l.15) ──

    /// (#4264) A `@json` literal's `@value` payload is opaque JSON, where array
    /// order IS significant — recursively — while object member order is not.
    /// With set semantics the W3C lanes were blind to a payload-reordering bug.
    #[test]
    fn json_payload_array_order_is_significant() {
        let literal = |payload: Value| {
            json!({"@id": "http://example.com/a",
                   "http://example.com/payload": [{"@value": payload, "@type": "@json"}]})
        };
        assert!(
            !json_ld_equal(&literal(json!([1, 2])), &literal(json!([2, 1]))),
            "@json payload array order must be significant"
        );
        assert!(
            !json_ld_equal(
                &literal(json!({"k": [1, 2]})),
                &literal(json!({"k": [2, 1]}))
            ),
            "@json payload array order must be significant at any depth"
        );
        // Keyword-looking member names inside the payload are plain JSON: a nested
        // `{"@value":…,"@type":"@json"}` or `@list` there does not reset the mode.
        assert!(!json_ld_equal(
            &literal(json!({"@value": [1, 2], "@type": "x"})),
            &literal(json!({"@value": [2, 1], "@type": "x"}))
        ));
        // Equal payloads stay equal; member order inside the payload is insignificant.
        assert!(json_ld_equal(
            &literal(json!([1, 2])),
            &literal(json!([1, 2]))
        ));
        assert!(json_ld_equal(
            &literal(json!({"a": 1, "b": [1, 2]})),
            &literal(json!({"b": [1, 2], "a": 1}))
        ));
        // A single-element `@type` array is recognised too.
        let arr_typed = |payload: Value| json!([{"@value": payload, "@type": ["@json"]}]);
        assert!(!json_ld_equal(
            &arr_typed(json!([1, 2])),
            &arr_typed(json!([2, 1]))
        ));
        // The carve-out must not leak: an ordinary property value array is still
        // an unordered set, and a non-`@json` `@value` is compared as before.
        assert!(json_ld_equal(
            &json!({"http://example.com/p": [{"@value": "x"}, {"@value": "y"}]}),
            &json!({"http://example.com/p": [{"@value": "y"}, {"@value": "x"}]})
        ));
    }

    /// Arrays outside @list are unordered (set semantics).
    #[test]
    fn arrays_outside_list_are_unordered() {
        let a = json!([1, 2, 3]);
        let b = json!([3, 1, 2]);
        assert!(
            json_ld_equal(&a, &b),
            "permuted array outside @list must be equal"
        );
    }

    /// Arrays outside @list with different elements are not equal.
    #[test]
    fn arrays_outside_list_different_elements() {
        let a = json!([1, 2, 3]);
        let b = json!([1, 2, 4]);
        assert!(!json_ld_equal(&a, &b));
    }

    /// Arrays that are the VALUE of a "@list" key are ORDER-SIGNIFICANT.
    #[test]
    fn array_inside_list_is_ordered_fail() {
        let a = json!({"@list": [1, 2, 3]});
        let b = json!({"@list": [3, 1, 2]});
        assert!(!json_ld_equal(&a, &b), "permuted @list must NOT be equal");
    }

    /// Arrays that are the VALUE of a "@list" key with the same order pass.
    #[test]
    fn array_inside_list_same_order_passes() {
        let a = json!({"@list": [1, 2, 3]});
        let b = json!({"@list": [1, 2, 3]});
        assert!(json_ld_equal(&a, &b));
    }

    /// Outer array is unordered, but each inner @list is ordered.
    #[test]
    fn nested_outer_unordered_inner_list_ordered() {
        // Two @list objects in different outer-array positions → equal (outer unordered).
        let a = json!([{"@list": [1, 2]}, {"@list": [3, 4]}]);
        let b = json!([{"@list": [3, 4]}, {"@list": [1, 2]}]);
        assert!(json_ld_equal(&a, &b), "outer array unordered");

        // But the @list contents themselves are ordered.
        let c = json!({"@list": [2, 1]});
        let d = json!({"@list": [1, 2]});
        assert!(!json_ld_equal(&c, &d), "@list contents are ordered");
    }

    /// Deeply nested @list: an @list inside an outer @list is still ordered.
    #[test]
    fn nested_list_within_list_is_ordered() {
        // value arrays inside @list elements: those inner arrays are NOT @list
        // values, so they are unordered.
        let a = json!({"@list": [{"foo": [1, 2]}, {"foo": [3, 4]}]});
        let b = json!({"@list": [{"foo": [2, 1]}, {"foo": [4, 3]}]});
        // @list order matters (outer), but "foo" arrays inside are unordered.
        assert!(json_ld_equal(&a, &b));
    }

    /// Object key order is insignificant.
    #[test]
    fn object_key_order_insignificant() {
        let a = json!({"@id": "http://example.org/a", "@type": ["http://example.org/T"]});
        let b = json!({"@type": ["http://example.org/T"], "@id": "http://example.org/a"});
        assert!(json_ld_equal(&a, &b));
    }

    /// Numeric equality: JSON integer `1` equals JSON float `1.0`.
    #[test]
    fn numeric_equality_int_float() {
        let a: Value = serde_json::from_str("1").unwrap();
        let b: Value = serde_json::from_str("1.0").unwrap();
        assert!(
            json_ld_equal(&a, &b),
            "1 and 1.0 must be equal under f64 numeric comparison"
        );
    }

    /// Different numeric values are not equal.
    #[test]
    fn numeric_inequality() {
        let a = json!(1);
        let b = json!(2);
        assert!(!json_ld_equal(&a, &b));
    }

    /// Large integers ≥ 2^53 that are distinct as i64 must not be collapsed by f64
    /// rounding.  Under the old f64-only path 9007199254740992 and 9007199254740993
    /// round to the same f64 and would be wrongly equal.  The numeric-guard fix
    /// [SONNET-4.6] sq-kk1mq compares integral values exactly via i64/u64.
    #[test]
    fn large_integer_numeric_guard() {
        // 2^53 and 2^53+1 are distinct i64 values but collapse to the same f64.
        let a: Value = serde_json::from_str("9007199254740992").unwrap();
        let b: Value = serde_json::from_str("9007199254740993").unwrap();
        assert!(
            !json_ld_equal(&a, &b),
            "2^53 and 2^53+1 must be UNEQUAL under exact i64 comparison"
        );
    }

    /// Integer JSON `1` equals float JSON `1.0` (f64 fallback when one side
    /// is non-integral; already covered by numeric_equality_int_float but
    /// kept as an explicit guard for the mixed-representation case).
    #[test]
    fn integer_equals_float_one() {
        let a: Value = serde_json::from_str("1").unwrap();
        let b: Value = serde_json::from_str("1.0").unwrap();
        assert!(
            json_ld_equal(&a, &b),
            "integer JSON 1 and float JSON 1.0 must be EQUAL"
        );
    }

    /// Duplicate elements in unordered arrays use multiset semantics.
    #[test]
    fn unordered_array_multiset_duplicates() {
        let a = json!([1, 1, 2]);
        let b = json!([1, 2, 1]);
        assert!(json_ld_equal(&a, &b), "multiset: [1,1,2] == [1,2,1]");

        let c = json!([1, 1, 2]);
        let d = json!([1, 2, 2]);
        assert!(!json_ld_equal(&c, &d), "multiset: [1,1,2] != [1,2,2]");
    }

    /// Type mismatch: null != false, string != number.
    #[test]
    fn type_mismatch_not_equal() {
        assert!(!json_ld_equal(&json!(null), &json!(false)));
        assert!(!json_ld_equal(&json!("1"), &json!(1)));
        assert!(!json_ld_equal(&json!([]), &json!({})));
    }

    // ── direct unit tests for the moved helpers (sq-hmd7l.15) ───────────────

    /// The `sparq_jsonld::Json` → `serde_json::Value` bridge round-trips a
    /// small document losslessly (both ASTs are plain JSON).
    #[test]
    fn sparq_json_to_serde_roundtrip() {
        let j = sparq_jsonld::Json::parse(r#"{"@id":"http://example.org/a","n":[1,2.5,"x"]}"#)
            .expect("parse sparq Json");
        let v = sparq_json_to_serde(&j).expect("bridge to serde_json");
        assert!(json_ld_equal(
            &v,
            &json!({"@id": "http://example.org/a", "n": [1, 2.5, "x"]})
        ));
    }

    /// N-Quads → canonical dataset: blank-node labels do not affect equality.
    #[test]
    fn nquads_canonical_dataset_is_bnode_blind() {
        let a = nquads_to_canonical_dataset(
            "_:x <http://example.org/p> \"v\" .\n_:x <http://example.org/q> _:y .\n",
        )
        .expect("parse a");
        let b = nquads_to_canonical_dataset(
            "_:n1 <http://example.org/p> \"v\" .\n_:n1 <http://example.org/q> _:n2 .\n",
        )
        .expect("parse b");
        assert_eq!(a, b, "relabelled blank nodes must compare equal");
    }

    /// `read_context_member` unwraps exactly one `@context` layer and refuses
    /// the non-object (remote/array) forms the writer cannot drive.
    #[test]
    fn read_context_member_unwraps_object_and_refuses_remote() {
        let dir =
            std::env::temp_dir().join(format!("sparq-jsonld-bench-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let ok = dir.join("ctx-ok.jsonld");
        std::fs::write(&ok, r#"{"@context": {"name": "http://schema.org/name"}}"#).expect("write");
        assert!(
            read_context_member(&ok).is_ok(),
            "inline object context must parse"
        );
        let remote = dir.join("ctx-remote.jsonld");
        std::fs::write(&remote, r#"{"@context": "http://example.org/ctx"}"#).expect("write");
        assert!(
            read_context_member(&remote).is_err(),
            "string (remote) context is not drivable and must be refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// JSON-LD → canonical dataset drives the REAL oxjsonld ingest parser and
    /// round-trips through `dataset_to_nquads` to the same canonical dataset.
    #[test]
    fn jsonld_to_dataset_and_nquads_roundtrip() {
        let doc = r#"{"@id": "http://example.org/s",
                      "http://example.org/p": [{"@value": "v"}, {"@id": "http://example.org/o"}]}"#;
        let ds = jsonld_to_canonical_dataset(doc, "http://example.org/base").expect("ingest");
        assert_eq!(ds.len(), 2, "two triples expected");
        let nq = dataset_to_nquads(&ds);
        let back = nquads_to_canonical_dataset(&nq).expect("reparse");
        assert_eq!(ds, back, "dataset → N-Quads → dataset must round-trip");
    }
}

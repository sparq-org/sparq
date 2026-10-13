//! Shared JSON-LD document-level equality oracle for this crate's integration
//! tests (`round_trip.rs`, `flatten.rs`). Rust integration tests are separate
//! binaries; a `tests/common/` sub-directory is not a test target, so each one
//! pulls this in with `mod common;` instead of keeping its own copy (#4264).
//!
//! Same semantics as the conformance crate's `json_ld_equal`
//! (`sparq-conformance/src/jsonld_bench.rs`), which restates them over
//! `serde_json::Value` (this crate keeps zero dependencies).

use sparq_jsonld::Json;

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
/// compared as their `Json::Raw` / `Json::Str` tokens).
///
/// The `@json` carve-out is load-bearing: a `@json` value is opaque JSON, where
/// `[1,2]` and `[2,1]` are different documents, so comparing the payload with set
/// semantics would let a payload-reordering bug pass (`round_trip.rs`'s
/// `json_literal_round_trips`, the flatten lane's W3C cases).
pub fn json_ld_equal(a: &Json, b: &Json) -> bool {
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

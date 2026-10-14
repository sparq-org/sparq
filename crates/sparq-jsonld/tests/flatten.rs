//! sq-oy1f.26 — crate-local integration test for the native
//! **Flattening Algorithm** (JSON-LD 1.1 API §7.1) over the official W3C
//! `w3c/json-ld-api` `flatten` test suite.
//!
//! This is the crate's OWN acceptance harness (the conformance-crate ratchet in
//! `sparq-conformance` is the gated CI lane). It walks the pinned `flatten`
//! manifest, runs [`sparq_jsonld::flatten`] on each positive case, and deep-compares
//! the result against the suite's NORMATIVE expected document using a native,
//! dependency-free JSON-LD comparator (`tests/common/mod.rs`: object key order
//! insignificant; array order significant inside `@list` and, recursively, inside a
//! `@json` literal payload) — the same oracle semantics as the conformance
//! lane, but expressed over the crate's own `Json` AST so the crate keeps ZERO
//! dependencies (no `serde_json`).
//!
//! The suite is fetched by `scripts/fetch-jsonld-tests.sh` into the gitignored
//! `tests/w3c/json-ld-api/`; when it is absent this test SKIPS itself (a fresh
//! offline checkout stays green). The asserted floor here is a crate-local sanity
//! floor; the authoritative RATCHET lives in `sparq-conformance`
//! (`src/floors/flatten.rs`, `FLOOR = 46`).

use sparq_jsonld::{flatten, flatten_expanded, Json, JsonLdOptions, NoopLoader, ProcessingMode};
use std::path::{Path, PathBuf};

mod common;
use common::json_ld_equal;

/// The pinned suite root (mirrors the conformance crate's `suite_root`).
fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/json-ld-api/tests")
}

const SUITE_BASE: &str = "https://w3c.github.io/json-ld-api/tests/";

/// A minimal parsed manifest entry (the fields the flatten lane consumes).
struct Entry {
    id: String,
    is_negative: bool,
    input: String,
    expect: Option<String>,
    base_override: Option<String>,
    requires: bool,
    has_context: bool,
    spec_version: Option<String>,
    processing_mode: Option<String>,
}

/// Read the `flatten-manifest.jsonld` sequence with the crate's own JSON parser.
fn read_flatten_manifest(root: &Path) -> Result<Vec<Entry>, String> {
    let path = root.join("flatten-manifest.jsonld");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read manifest: {}", e))?;
    let doc = Json::parse(&text).map_err(|e| format!("parse manifest: {}", e))?;
    let seq = doc
        .get("sequence")
        .and_then(as_array_ref)
        .ok_or("manifest has no sequence array")?;
    let mut out = Vec::new();
    for e in seq {
        let id = e.get("@id").and_then(Json::as_str).unwrap_or("?").to_string();
        let is_negative = type_contains(e, "NegativeEvaluationTest");
        let input = e.get("input").and_then(Json::as_str).unwrap_or("").to_string();
        let expect = e.get("expect").and_then(Json::as_str).map(str::to_string);
        let opt = e.get("option");
        let base_override = opt
            .and_then(|o| o.get("base"))
            .and_then(Json::as_str)
            .map(str::to_string);
        let requires = e.get("requires").is_some();
        let has_context = e.get("context").is_some();
        let spec_version = opt
            .and_then(|o| o.get("specVersion"))
            .and_then(Json::as_str)
            .map(str::to_string);
        let processing_mode = opt
            .and_then(|o| o.get("processingMode"))
            .and_then(Json::as_str)
            .map(str::to_string);
        out.push(Entry {
            id,
            is_negative,
            input,
            expect,
            base_override,
            requires,
            has_context,
            spec_version,
            processing_mode,
        });
    }
    Ok(out)
}

fn as_array_ref(j: &Json) -> Option<&Vec<Json>> {
    match j {
        Json::Arr(items) => Some(items),
        _ => None,
    }
}

/// True iff the entry's `@type` (string or array) contains `needle`.
fn type_contains(e: &Json, needle: &str) -> bool {
    match e.get("@type") {
        Some(Json::Str(s)) => s.contains(needle),
        Some(Json::Arr(items)) => items
            .iter()
            .any(|t| matches!(t, Json::Str(s) if s.contains(needle))),
        _ => false,
    }
}

/// The document base for a case: `option.base` if given, else the suite base + input path.
fn doc_base(e: &Entry) -> String {
    e.base_override
        .clone()
        // positional format arg (CodeQL rust/unused-variable false-positive guard).
        .unwrap_or_else(|| format!("{}{}", SUITE_BASE, e.input))
}

/// (#4264) The flatten lane's oracle must treat a `@json` literal payload as
/// opaque JSON (array order significant, recursively). With set semantics a
/// flatten bug that reordered a payload array would still count as a pass.
#[test]
fn oracle_json_payload_array_order_is_significant() {
    let flat = |payload: &str| {
        Json::parse(&format!(
            r#"[{{"@id":"http://example.com/a",
                  "http://example.com/payload":[{{"@value":{},"@type":"@json"}}]}}]"#,
            payload
        ))
        .expect("valid flattened JSON")
    };
    assert!(!json_ld_equal(&flat("[1,2]"), &flat("[2,1]")));
    assert!(!json_ld_equal(
        &flat(r#"{"k":[1,2]}"#),
        &flat(r#"{"k":[2,1]}"#)
    ));
    assert!(json_ld_equal(
        &flat(r#"{"a":1,"b":[1,2]}"#),
        &flat(r#"{"b":[1,2],"a":1}"#)
    ));
}

/// `flatten_expanded` is a normaliser: representative expanded inputs reach a
/// fixed point after one pass, while a nested input demonstrates that the first pass does
/// real normalisation work.
#[test]
fn flatten_expanded_is_idempotent() {
    let single_node = Json::parse(
        r#"[{"@id":"http://example.com/a","http://example.com/name":[{"@value":"Alice"}]}]"#,
    )
    .expect("valid single-node fixture");
    let named_graph = Json::parse(
        r#"[
            {"@id":"http://example.com/g","@graph":[
                {"@id":"http://example.com/b","http://example.com/name":[{"@value":"B"}]},
                {"@id":"http://example.com/a","http://example.com/name":[{"@value":"A"}]}
            ]},
            {"@id":"http://example.com/root","http://example.com/graph":[{"@id":"http://example.com/g"}]}
        ]"#,
    )
    .expect("valid named-graph fixture");
    let blank_node_cross_references = Json::parse(
        r#"[
            {"@id":"_:left","http://example.com/link":[
                {"@id":"_:right","http://example.com/name":[{"@value":"right"}]}
            ]},
            {"@id":"_:right","http://example.com/back":[{"@id":"_:left"}]}
        ]"#,
    )
    .expect("valid blank-node fixture");
    let empty = Json::Arr(Vec::new());
    let stress = Json::Arr(
        (0..100)
            .rev()
            .map(|index| {
                Json::Obj(vec![
                    (
                        "http://example.com/name".to_string(),
                        Json::Arr(vec![Json::Obj(vec![(
                            "@value".to_string(),
                            Json::Str(format!("node-{index}")),
                        )])]),
                    ),
                    (
                        "@id".to_string(),
                        Json::Str(format!("http://example.com/node/{index:03}")),
                    ),
                ])
            })
            .collect(),
    );

    let fixtures = [
        ("single node with property", &single_node),
        ("multiple nodes with named graph", &named_graph),
        (
            "blank nodes with cross-references",
            &blank_node_cross_references,
        ),
        ("empty array", &empty),
        ("100-node stress document", &stress),
    ];

    for (name, input) in fixtures {
        let once = flatten_expanded(input);
        let twice = flatten_expanded(&once);
        assert!(
            json_ld_equal(&once, &twice),
            "flattening was not idempotent for {name}: once={once:?}, twice={twice:?}"
        );
    }

    let normalized = flatten_expanded(&blank_node_cross_references);
    assert!(
        !json_ld_equal(&blank_node_cross_references, &normalized),
        "blank-node fixture must require real flattening, not already be a fixed point"
    );
}

/// Run the native flatten over every positive `flatten` case and return `(pass, fail, skip,
/// failures)`.
fn run_flatten_suite(root: &Path) -> (usize, usize, usize, Vec<String>) {
    let entries = read_flatten_manifest(root).expect("flatten manifest");
    let (mut pass, mut fail, mut skip) = (0usize, 0usize, 0usize);
    let mut failures = Vec::new();
    for e in &entries {
        // SKIP buckets — identical to the conformance lane.
        if e.requires
            || e.is_negative
            || e.has_context
            || e.spec_version.as_deref() == Some("json-ld-1.0")
            || e.processing_mode.as_deref() == Some("json-ld-1.0")
            || e.input.starts_with("http://")
            || e.input.starts_with("https://")
        {
            skip += 1;
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            skip += 1;
            continue;
        };
        let in_text = std::fs::read_to_string(root.join(&e.input)).expect("read input");
        let input = match Json::parse(&in_text) {
            Ok(j) => j,
            Err(_) => {
                fail += 1;
                failures.push(format!("{}: parse input", e.id));
                continue;
            }
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(doc_base(e));
        opts.processing_mode = match (e.processing_mode.as_deref(), e.spec_version.as_deref()) {
            (Some("json-ld-1.0"), _) | (_, Some("json-ld-1.0")) => ProcessingMode::JsonLd10,
            _ => ProcessingMode::JsonLd11,
        };
        let got = match flatten(&input, &opts, &NoopLoader) {
            Ok(j) => j,
            Err(why) => {
                fail += 1;
                failures.push(format!("{}: flatten() error: {}", e.id, why));
                continue;
            }
        };
        let exp_text = std::fs::read_to_string(root.join(expect_rel)).expect("read expect");
        let want = Json::parse(&exp_text).expect("parse expect");
        if json_ld_equal(&got, &want) {
            pass += 1;
        } else {
            fail += 1;
            failures.push(format!("{}: JSON mismatch", e.id));
        }
    }
    (pass, fail, skip, failures)
}

/// The native flatten pipeline matches the W3C `flatten` suite's normative expected
/// documents on at least the crate-local floor (the authoritative ratchet lives in
/// `sparq-conformance`). SKIPS when the suite is absent (fresh offline checkout).
#[test]
fn native_flatten_matches_w3c_suite() {
    let root = suite_root();
    if !root.join("flatten-manifest.jsonld").exists() {
        eprintln!(
            "SKIP: W3C JSON-LD suite absent at {} — run scripts/fetch-jsonld-tests.sh",
            root.display()
        );
        return;
    }
    let (pass, fail, skip, failures) = run_flatten_suite(&root);
    println!("native flatten: {} pass / {} fail / {} skip", pass, fail, skip);
    for f in &failures {
        println!("  {}", f);
    }
    // Crate-local sanity floor (the gated ratchet is sparq-conformance FLOOR = 46). The
    // remaining fails are inherited native-expand gaps (owned by sq-oy1f.37), not flatten
    // bugs — see crates/sparq-conformance/src/floors/flatten.rs.
    assert!(
        pass >= 46,
        "native flatten pass count {} below the crate-local floor 46",
        pass
    );
}

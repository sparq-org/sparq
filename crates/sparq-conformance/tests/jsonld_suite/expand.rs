//! [FABLE-5] sq-oy1f.40 — the W3C JSON-LD 1.1 `expand` lane runner (split out of the
//! former monolithic `tests/jsonld_suite.rs`; behaviour byte-identical). The `expand`
//! category uses the NATIVE DOCUMENT-LEVEL oracle (`sparq_jsonld::expand()` +
//! `json_ld_equal`), NOT the RDF-writer path.

use super::common::*;
use serde_json::Value;
// [SONNET-4.6] sq-kk1mq — native expand() for the document-level expand oracle.
// [SONNET-4.6] sq-oy1f.45 — FsLoader maps the W3C suite URL prefix to local fixtures so
// `@context`/`@import` relative-URL references (e.g. t0126, tc034, tso08) are resolved
// from the local checkout rather than hitting the network.
use sparq_jsonld::{expand as jsonld_expand, FsLoader, JsonLdOptions, ProcessingMode};
use std::path::Path;

/// [SONNET-4.6] sq-kk1mq — run the W3C JSON-LD `expand` category with the
/// NATIVE DOCUMENT-LEVEL oracle: call `sparq_jsonld::expand()` directly on the
/// input document and deep-compare the result to the suite's expected expanded
/// document via `json_ld_equal`.
///
/// ## Pipeline (the native expand path)
///
/// 1. Parse the case `input` (`.jsonld`) as a `sparq_jsonld::Json` AST —
///    the expander's native input type.
/// 2. Build `JsonLdOptions` from the manifest entry (`base`, `expandContext`,
///    `processingMode`/`specVersion`).  `expandContext` is a path to a context
///    file; it is read and parsed as a `sparq_jsonld::Json` and forwarded as
///    `options.expand_context`.
/// 3. Call `sparq_jsonld::expand(&input_json, &opts, &NoopLoader)`.
///    Remote `@context` / `@import` references raise `loading document failed`
///    from the `NoopLoader` (deny-by-default).
/// 4. Convert the `Result<Json, JsonLdError>` output to a `serde_json::Value`
///    by writing and re-parsing (no structural loss — both ASTs are JSON).
/// 5. Parse the suite's expected document as a `serde_json::Value`.
/// 6. Compare with `json_ld_equal` (object key order insignificant; array
///    order significant only inside `@list`; integers compared exactly,
///    non-integral numbers compared as f64 so `1` ≡ `1.0`).
///
/// ## NegativeEvaluationTests are RUN, not skipped ([OPUS-5] sq-gzsky)
///
/// The 109-case negative SKIP bucket this lane used to carry WAS the entire expand gap
/// (`research/gap-jsonld-conformance-2026-07.md` §6). It is closed: a negative case passes
/// iff `expand()` errors with EXACTLY the manifest's `expectErrorCode` — a WRONG code is a
/// FAIL, never a pass — which is the oracle shape the `frame` lane has used since
/// sq-oy1f.29 and what makes the closed `sparq_jsonld::JsonLdErrorCode` enum load-bearing.
///
/// ## Honest SKIP buckets (recorded, not passed, not failed)
///
/// * `requires` optional-feature cases (same as all other lanes).
/// * Remote `input` URL — no network.
/// * A POSITIVE case with no `expect` file — nothing to compare. (A negative case needs no
///   `expect` document: its expectation is the error code.)
pub fn run_expand_native(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "expand") {
        Ok(e) => e,
        Err(why) => {
            s.fail("expand-manifest", why);
            return s;
        }
    };
    // [SONNET-4.6] sq-oy1f.45 — FsLoader maps the suite's base URL prefix to the local
    // fixture directory so `@context` / `@import` IRI references in test inputs are
    // resolved against the checked-out files instead of hitting the network.  The mapping
    // mirrors the `SUITE_BASE` constant (`https://w3c.github.io/json-ld-api/tests/`).
    let loader = FsLoader::new().map_prefix(SUITE_BASE, root);
    for e in &entries {
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        let is_negative = e.is_negative || e.expect_error_code.is_some();
        // A positive case with no `expect` document has nothing to compare.
        if !is_negative && e.expect.is_none() {
            s.skip();
            continue;
        }
        // Remote input (no network) — guard.
        if e.input.starts_with("http://") || e.input.starts_with("https://") {
            s.skip();
            continue;
        }

        // 1. Read and parse the input document as sparq_jsonld::Json.
        let input_path = root.join(&e.input);
        let in_text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read input: {}", why));
                continue;
            }
        };
        let input_json = match sparq_jsonld::Json::parse(&in_text) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, format!("parse input JSON: {}", why));
                continue;
            }
        };

        // 2. Build JsonLdOptions from the manifest entry.
        let base = doc_base(e);
        let processing_mode = match (e.processing_mode.as_deref(), e.spec_version.as_deref()) {
            (Some("json-ld-1.0"), _) | (_, Some("json-ld-1.0")) => ProcessingMode::JsonLd10,
            _ => ProcessingMode::JsonLd11,
        };
        // JsonLdOptions is #[non_exhaustive] — build via default() + field mutation.
        let mut opts = JsonLdOptions::default();
        opts.base = Some(base.clone());
        opts.processing_mode = processing_mode;

        // Forward expandContext if present: read the context file and parse as
        // sparq_jsonld::Json.  The expander unwraps one "@context" layer itself
        // (options.expand_context.get("@context").unwrap_or(&ctx)).
        if let Some(ctx_rel) = &e.expand_context_path {
            let ctx_path = root.join(ctx_rel);
            let ctx_text = match std::fs::read_to_string(&ctx_path) {
                Ok(t) => t,
                Err(why) => {
                    s.fail(&e.id, format!("read expandContext file: {}", why));
                    continue;
                }
            };
            match sparq_jsonld::Json::parse(&ctx_text) {
                Ok(ctx_json) => opts.expand_context = Some(ctx_json),
                Err(why) => {
                    s.fail(&e.id, format!("parse expandContext JSON: {}", why));
                    continue;
                }
            }
        }

        // 3. Call the native expand() algorithm. The FsLoader resolves any `@context` /
        // `@import` relative-URL references to local fixture files (sq-oy1f.45).
        let outcome = jsonld_expand(&input_json, &opts, &loader);

        // 3b. NegativeEvaluationTest: pass iff expand() raises EXACTLY the
        //     manifest's `expectErrorCode` (same shape as the frame lane).
        if is_negative {
            let want_code = e.expect_error_code.as_deref().unwrap_or("");
            match &outcome {
                Err(err) if err.code().as_str() == want_code => s.pass(),
                Err(err) => s.fail(
                    &e.id,
                    format!("expected error {want_code:?}, got {:?}", err.code().as_str()),
                ),
                Ok(_) => s.fail(&e.id, format!("expected error {want_code:?}, got success")),
            }
            continue;
        }

        let Some(expect_rel) = &e.expect else {
            unreachable!("positive cases without an `expect` document are skipped above")
        };
        let expanded = match outcome {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, format!("expand() error: {}", why));
                continue;
            }
        };

        // 4. Convert the expand() output to serde_json::Value.
        let got: Value = match sparq_json_to_serde(&expanded) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("convert expand output: {}", why));
                continue;
            }
        };

        // 5. Read and parse the expected document.
        let expect_path = root.join(expect_rel);
        let exp_text = match std::fs::read_to_string(&expect_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read expect: {}", why));
                continue;
            }
        };
        let want: Value = match serde_json::from_str(&exp_text) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("parse expect JSON: {}", why));
                continue;
            }
        };

        // 6. Document-level JSON-LD equality (oracle pinned per sq-kk1mq).
        // Report the JSON kind (with size for array/object) so a mismatch on
        // a non-array structure is immediately diagnosable rather than always
        // showing "0 nodes".
        fn json_kind_desc(v: &Value) -> String {
            match v {
                Value::Array(a) => format!("array({} items)", a.len()),
                Value::Object(o) => format!("object({} keys)", o.len()),
                Value::String(_) => "string".to_owned(),
                Value::Null => "null".to_owned(),
                Value::Bool(b) => format!("bool({})", b),
                Value::Number(n) => format!("number({})", n),
            }
        }
        if json_ld_equal(&got, &want) {
            s.pass();
        } else {
            s.fail(
                &e.id,
                format!(
                    "expand JSON mismatch: got {}, want {}",
                    json_kind_desc(&got),
                    json_kind_desc(&want),
                ),
            );
        }
    }
    s
}

/// [OPUS-5] sq-gzsky — scope pin for the negative lane, the analogue of the compact lane's
/// `t0038_skip_is_narrowly_scoped`. Asserts that every `NegativeEvaluationTest` in the
/// pinned `expand` manifest (a) carries an `expectErrorCode` — without one the runner would
/// compare against `""`, which no spec code equals, so the case could only ever FAIL — and
/// (b) clears every skip gate `run_expand_native` applies, so it RUNS. A suite-pin bump that
/// adds a negative behind `requires` or at a remote URL fails HERE and forces a deliberate
/// decision instead of quietly shrinking the gated set.
#[test]
fn expand_negatives_all_run_with_an_expected_code() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C JSON-LD suite not present at {} — run scripts/fetch-jsonld-tests.sh",
            root.display()
        );
        return;
    }
    let entries = read_manifest(&root, "expand").expect("read expand manifest");
    let negatives: Vec<_> = entries.iter().filter(|e| e.is_negative).collect();
    assert!(
        !negatives.is_empty(),
        "the pinned expand manifest must contain NegativeEvaluationTests — an empty set \
         would make the negative lane vacuous"
    );
    for e in negatives {
        assert!(
            e.expect_error_code.is_some(),
            "{} is a NegativeEvaluationTest with no expectErrorCode — it could never pass",
            e.id
        );
        assert!(
            e.requires.is_none()
                && !e.input.starts_with("http://")
                && !e.input.starts_with("https://"),
            "{} would be SKIPPED rather than run — decide it explicitly",
            e.id
        );
    }
}

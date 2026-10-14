//! sq-oy1f.26 — the W3C JSON-LD 1.1 `flatten` lane runner. Moved to
//! the NATIVE DOCUMENT-LEVEL oracle (`sparq_jsonld::flatten()` + `json_ld_equal`),
//! matching the `expand` lane (sq-kk1mq). The former RDF-equivalence oracle over
//! the shipping writer is retired here: the native Flattening Algorithm
//! (JSON-LD 1.1 API §7.1, bead sq-oy1f.26) produces the flattened document
//! directly, so the oracle compares against the suite's NORMATIVE expected
//! document rather than round-tripping through RDF.

use super::common::*;
// sq-oy1f.26 — native flatten() for the document-level flatten oracle.
use sparq_jsonld::{flatten as jsonld_flatten, JsonLdOptions, NoopLoader, ProcessingMode};
use std::path::Path;

/// sq-oy1f.26 — run the W3C JSON-LD `flatten` category with the NATIVE
/// DOCUMENT-LEVEL oracle: call `sparq_jsonld::flatten()` directly on the input
/// document and deep-compare the result to the suite's expected flattened
/// document via `json_ld_equal`.
///
/// ## Pipeline (the native flatten path)
///
/// 1. Parse the case `input` (`.jsonld`) as a `sparq_jsonld::Json` AST — the
///    flattener's native input type.
/// 2. Build `JsonLdOptions` from the manifest entry (`base`,
///    `processingMode`/`specVersion`).
/// 3. Call `sparq_jsonld::flatten(&input_json, &opts, &NoopLoader)` (which expands
///    then flattens). Remote `@context` / `@import` references raise `loading
///    document failed` from the `NoopLoader` (deny-by-default).
/// 4. Convert the `Result<Json, JsonLdError>` output to a `serde_json::Value` by
///    writing and re-parsing (no structural loss — both ASTs are JSON).
/// 5. Parse the suite's expected document as a `serde_json::Value`.
/// 6. Compare with `json_ld_equal` (object key order insignificant; array order
///    significant only inside `@list` and a `@json` payload; integers compared exactly, non-integral
///    numbers compared as f64 so `1` ≡ `1.0`).
///
/// ## Honest SKIP buckets (recorded, not passed, not failed) — see the `flatten` floor
///
/// * `requires` optional-feature cases (same as all other lanes).
/// * NegativeEvaluationTests — flattener error-code completeness is unverified;
///   deferred to a child bead of sq-oy1f. SKIP (honest), never a counted pass.
/// * JSON-LD-1.0-only positives (processing-mode shape differences a 1.1 processor
///   is not obliged to reproduce). SKIP.
/// * Remote `input` URL — no network.
/// * No `expect` file — nothing to compare.
/// * A `context` member (post-flatten compaction to `{ "@context": …, "@graph": … }`)
///   — document-level Compaction is a SEPARATE bead (sq-oy1f.27); the flatten lane
///   emits the expanded flattened form only, so a compaction case is SKIPPED here
///   (it becomes a pass once compaction composes onto this output).
pub fn run_flatten_native(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "flatten") {
        Ok(e) => e,
        Err(why) => {
            s.fail("flatten-manifest", why);
            return s;
        }
    };
    for e in &entries {
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        // NegativeEvaluationTests: flattener raises some errors but error-code
        // completeness is unverified — SKIP honestly (deferred).
        if e.is_negative {
            s.skip();
            continue;
        }
        // JSON-LD-1.0-only positives — SKIP.
        if e.spec_version.as_deref() == Some("json-ld-1.0")
            || e.processing_mode.as_deref() == Some("json-ld-1.0")
        {
            s.skip();
            continue;
        }
        // A compaction `context` member routes through document-level Compaction
        // (bead sq-oy1f.27), not yet composed here — SKIP.
        if e.context.is_some() {
            s.skip();
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
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

        // 3. Call the native flatten() algorithm (a remote-context input raises from
        //    the NoopLoader — those cases surface as an honest FAIL below, matching
        //    the expand lane's posture).
        let flattened = match jsonld_flatten(&input_json, &opts, &NoopLoader) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, format!("flatten() error: {}", why));
                continue;
            }
        };

        // 4. Convert the flatten() output to serde_json::Value.
        let got = match sparq_json_to_serde(&flattened) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("convert flatten output: {}", why));
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
        let want = match serde_json::from_str::<serde_json::Value>(&exp_text) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("parse expect JSON: {}", why));
                continue;
            }
        };

        // 6. Document-level JSON-LD equality (same comparator as the expand lane).
        if json_ld_equal(&got, &want) {
            s.pass();
        } else {
            s.fail(&e.id, format!("flatten JSON mismatch for {}", e.id));
        }
    }
    s
}

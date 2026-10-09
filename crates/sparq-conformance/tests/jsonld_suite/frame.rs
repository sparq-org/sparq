//! [FABLE-5] sq-oy1f.29 — the W3C JSON-LD 1.1 `frame` lane runner, moved to the
//! NATIVE DOCUMENT-LEVEL oracle (`sparq_jsonld::frame::frame()` + `json_ld_equal`),
//! matching the `expand` / `flatten` / `compact` lanes. The former RDF-answer-
//! equivalence oracle over the RDF-first engine framer is retired here: the native
//! Framing Algorithm (JSON-LD 1.1 Framing §4, bead sq-oy1f.29) produces the framed
//! document directly, so the oracle compares against the suite's NORMATIVE expected
//! document rather than round-tripping through RDF. Framing runs against the
//! SEPARATE `w3c/json-ld-framing` suite (`scripts/fetch-jsonld-framing-tests.sh`).

use super::common::*;
use sparq_jsonld::frame::{frame as jsonld_frame, FrameOptions};
use sparq_jsonld::{JsonLdOptions, NoopLoader, ProcessingMode};
use std::path::Path;

/// [FABLE-5] sq-oy1f.29 — run the SEPARATE W3C `w3c/json-ld-framing` suite against
/// the NATIVE document-level framer.
///
/// ## Pipeline (the native framing path)
///
/// 1. Parse the case `input` and `frame` documents as `sparq_jsonld::Json` ASTs.
/// 2. Build `JsonLdOptions` from the manifest entry (`base`,
///    `processingMode`/`specVersion`, `ordered`) and `FrameOptions` from
///    `option.omitGraph` (mode-dependent spec default when absent).
/// 3. Call `sparq_jsonld::frame::frame(&input, &frame, …, &NoopLoader)` — the full
///    §4.1 composition: expand input, frame-expand the frame, node-map + `@merged`,
///    frame matching, prune, compact against the frame's `@context`.
/// 4. **NegativeEvaluationTests are RUN, not skipped** (the framer raises the spec's
///    frame-validation errors since sq-oy1f.29): the case passes iff `frame()`
///    errors with exactly the manifest's `expectErrorCode` (`invalid frame`,
///    `invalid @embed value`).
/// 5. A positive case compares the framed output to the suite's NORMATIVE expected
///    document with `json_ld_equal` (object key order insignificant; array order
///    significant only inside `@list`).
///
/// ## Honest SKIP buckets (recorded, not passed, not failed) — see the `frame` floor
///
/// * `requires` optional-feature cases — out of the gated surface.
/// * A positive case with no `expect` document — nothing to compare.
/// * Remote `input`/`frame` URLs — none in the pinned suite, but guarded (no network).
pub fn run_frame(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "frame") {
        Ok(e) => e,
        Err(why) => {
            s.fail("frame-manifest", why);
            return s;
        }
    };
    for e in &entries {
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        let Some(frame_rel) = &e.frame else {
            s.skip();
            continue;
        };
        // Remote input/frame (no network) — none in the pinned suite; guard.
        if e.input.starts_with("http://")
            || e.input.starts_with("https://")
            || frame_rel.starts_with("http://")
            || frame_rel.starts_with("https://")
        {
            s.skip();
            continue;
        }

        // 1. Read + parse the input and frame documents as sparq_jsonld::Json.
        let input_json = match read_sparq_json(root, &e.input) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, why);
                continue;
            }
        };
        let frame_json = match read_sparq_json(root, frame_rel) {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, why);
                continue;
            }
        };

        // 2. Options from the manifest entry.
        let processing_mode = match (e.processing_mode.as_deref(), e.spec_version.as_deref()) {
            (Some("json-ld-1.0"), _) | (_, Some("json-ld-1.0")) => ProcessingMode::JsonLd10,
            _ => ProcessingMode::JsonLd11,
        };
        let mut opts = JsonLdOptions::default();
        opts.base = Some(frame_doc_base(e));
        opts.processing_mode = processing_mode;
        opts.ordered = e.ordered.unwrap_or(false);
        let mut fopts = FrameOptions::default();
        fopts.omit_graph = e.omit_graph;

        // 3. The native frame() composition.
        let framed = jsonld_frame(&input_json, &frame_json, &opts, &fopts, &NoopLoader);

        // 4. NegativeEvaluationTest: pass iff the framer raises EXACTLY the expected
        //    spec error code.
        if e.is_negative || e.expect_error_code.is_some() {
            let want_code = e.expect_error_code.as_deref().unwrap_or("");
            match &framed {
                Err(err) if err.code().as_str() == want_code => s.pass(),
                Err(err) => s.fail(
                    &e.id,
                    format!(
                        "expected error {want_code:?}, got {:?}",
                        err.code().as_str()
                    ),
                ),
                Ok(_) => s.fail(&e.id, format!("expected error {want_code:?}, got success")),
            }
            continue;
        }
        let Some(expect_rel) = &e.expect else {
            s.skip();
            continue;
        };
        let framed = match framed {
            Ok(j) => j,
            Err(why) => {
                s.fail(&e.id, format!("frame() error: {why}"));
                continue;
            }
        };

        // 5. Document-level JSON-LD equality against the normative expected document.
        let got = match sparq_json_to_serde(&framed) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("convert frame output: {why}"));
                continue;
            }
        };
        let expect_path = root.join(expect_rel);
        let exp_text = match std::fs::read_to_string(&expect_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read expect: {why}"));
                continue;
            }
        };
        let want = match serde_json::from_str::<serde_json::Value>(&exp_text) {
            Ok(v) => v,
            Err(why) => {
                s.fail(&e.id, format!("parse expect JSON: {why}"));
                continue;
            }
        };
        if json_ld_equal(&got, &want) {
            s.pass();
        } else {
            s.fail(&e.id, format!("framed JSON mismatch for {}", e.id));
        }
    }
    s
}

/// Read a suite document and parse it as a `sparq_jsonld::Json` AST.
fn read_sparq_json(root: &Path, rel: &str) -> Result<sparq_jsonld::Json, String> {
    let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
    sparq_jsonld::Json::parse(&text).map_err(|e| format!("parse {rel}: {e}"))
}

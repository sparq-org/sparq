//! [FABLE-5] sq-oy1f.27 — the W3C JSON-LD 1.1 `compact` lane runner, on the NATIVE
//! DOCUMENT-LEVEL oracle: `sparq_jsonld::compact::compact()` (the spec Compaction
//! Algorithm over the expanded document) deep-compared against the suite's NORMATIVE
//! expected document with `json_ld_equal` — the same oracle shape as the `expand` and
//! `flatten` lanes (sq-kk1mq / sq-oy1f.26). This REPLACES the old oxjsonld self-reparse
//! round-trip oracle (`reparse(compact(D, ctx)) ≡ D`), which measured RDF losslessness
//! of the engine's RDF-first writer, not the Compaction Algorithm — and could pass a
//! document whose JSON shape a strict third-party processor reads differently (the exact
//! honesty gap the oracle correction closes; see the floor doc for the side-by-side
//! re-pin).

use super::common::*;
use serde_json::Value;
use sparq_jsonld::{FsLoader, JsonLdOptions, ProcessingMode};
use std::path::Path;

// [GPT-5] sq-ruktv — keep the order-sensitive measurement beside the gating
// JSON-LD-semantic score. The strict score is diagnostic only: compacted
// `@container: @list` arrays do not carry an explicit `@list` marker, so the
// shared comparator cannot infer that their order is significant.
pub struct CompactScores {
    pub semantic: Score,
    pub strict: Score,
}

/// [FABLE-5] sq-oy1f.27 — run the W3C JSON-LD `compact` category with the NATIVE
/// DOCUMENT-LEVEL oracle.
///
/// ## Pipeline (the native compact path)
///
/// 1. Read the case `input` (`.jsonld`) and parse it as a `sparq_jsonld::Json`.
/// 2. Read the case `context` (the sibling `*-context.jsonld`) and parse it likewise.
/// 3. Build `JsonLdOptions` from the manifest entry: `base` (the document IRI or the
///    `option.base` override), `processingMode`/`specVersion`, `compactArrays`,
///    `compactToRelative`.
/// 4. Call `sparq_jsonld::compact::compact(input, context, opts, loader)` — this expands
///    the input through the native Expansion Algorithm, then runs the spec Compaction
///    Algorithm. The `FsLoader` maps the suite URL prefix to the local fixture tree so
///    remote `@context`/`@import` references resolve offline (deny-by-default otherwise).
/// 5. Deep-compare against the suite's EXPECTED document via `json_ld_equal` (object key
///    order insignificant; array order significant only inside explicit `@list` values;
///    numbers compared as in the expand lane).
///
/// ## Honest oracle caveat
///
/// `json_ld_equal` treats arrays outside an explicit `"@list"` key as SETS. In a
/// *compacted* document a `@container: @list` term's array carries list order without
/// the `@list` marker, so the comparator cannot see it — a wrongly-ordered compacted
/// list would still pass. The runner prints a separate advisory `TOTAL compact-strict`
/// measurement, and the current strict order-sensitive pass count is recorded alongside
/// the floor (see `floors::compact`); list-order fidelity is also covered by the crate-local
/// `tests/compact.rs` unit tests, which assert exact shapes.
///
/// ## NegativeEvaluationTests are RUN, not skipped ([OPUS-5] sq-gzsky)
///
/// The 17-case negative SKIP bucket is closed (bead sq-oy1f.31's compact half). A negative
/// passes iff `compact()` — expansion + context processing + compaction — errors with
/// EXACTLY the manifest's `expectErrorCode`; a WRONG code is a FAIL, never a pass. Both
/// the semantic and the advisory strict-order tallies score negatives identically (the
/// comparator plays no part when the expectation is an error code).
///
/// ## Honest SKIP buckets (recorded, not passed, not failed)
///
/// * `requires` optional-feature cases — out of the gated surface.
/// * `#t0038` — the ONE narrowly-pinned 1.0-only skip; see
///   [`is_pinned_1_0_only_skip`] for the justification and
///   [`t0038_skip_is_narrowly_scoped`] for the test enforcing its scope. NOT a
///   blanket `specVersion: json-ld-1.0` skip: `option.processingMode: json-ld-1.0`
///   cases (`#t0075`/`#t0106`/`#tp001`) RUN, and any FUTURE 1.0-only positive
///   added at a suite-pin bump RUNS (and fails loudly) rather than being silently
///   skipped.
/// * Remote `input` URL — no network.
/// * No `context` member — nothing to compact against; and, for a POSITIVE case only, no
///   `expect` document — nothing to compare. (A negative's expectation is its error code.)
pub fn run_compact(root: &Path) -> CompactScores {
    let mut s = Score::default();
    let mut strict = Score::default();
    let entries = match read_manifest(root, "compact") {
        Ok(e) => e,
        Err(why) => {
            s.fail("compact-manifest", why);
            strict.fail(
                "compact-manifest",
                "manifest unavailable for strict-order diagnostic".to_string(),
            );
            return CompactScores {
                semantic: s,
                strict,
            };
        }
    };
    // Map the suite's base URL prefix to the local fixture directory (same as the
    // expand lane) so remote-context references in inputs resolve offline.
    let loader = FsLoader::new().map_prefix(SUITE_BASE, root);
    for e in &entries {
        if e.requires.is_some() {
            s.skip();
            strict.skip();
            continue;
        }
        let is_negative = e.is_negative || e.expect_error_code.is_some();
        // The ONE narrowly-pinned 1.0-only skip (#t0038) — see
        // `is_pinned_1_0_only_skip`; scope enforced by `t0038_skip_is_narrowly_scoped`.
        if is_pinned_1_0_only_skip(e) {
            s.skip();
            strict.skip();
            continue;
        }
        // A positive case with no `expect` document has nothing to compare.
        if !is_negative && e.expect.is_none() {
            s.skip();
            strict.skip();
            continue;
        }
        let Some(ctx_rel) = &e.context else {
            s.skip();
            strict.skip();
            continue;
        };
        if e.input.starts_with("http://") || e.input.starts_with("https://") {
            s.skip();
            strict.skip();
            continue;
        }

        // 1-2. Read and parse the input document and the caller context.
        let input_path = root.join(&e.input);
        let in_text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(why) => {
                let why = format!("read input: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        let input_json = match sparq_jsonld::Json::parse(&in_text) {
            Ok(j) => j,
            Err(why) => {
                let why = format!("parse input JSON: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        let ctx_path = root.join(ctx_rel);
        let ctx_text = match std::fs::read_to_string(&ctx_path) {
            Ok(t) => t,
            Err(why) => {
                let why = format!("read context: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        let ctx_json = match sparq_jsonld::Json::parse(&ctx_text) {
            Ok(j) => j,
            Err(why) => {
                let why = format!("parse context JSON: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };

        // 3. Options from the manifest entry.
        let processing_mode = match (e.processing_mode.as_deref(), e.spec_version.as_deref()) {
            (Some("json-ld-1.0"), _) | (_, Some("json-ld-1.0")) => ProcessingMode::JsonLd10,
            _ => ProcessingMode::JsonLd11,
        };
        // JsonLdOptions is #[non_exhaustive] — build via default() + field mutation.
        let mut opts = JsonLdOptions::default();
        opts.base = Some(doc_base(e));
        opts.processing_mode = processing_mode;
        if let Some(ca) = e.compact_arrays {
            opts.compact_arrays = ca;
        }
        if let Some(cr) = e.compact_to_relative {
            opts.compact_to_relative = cr;
        }

        // 4. The native document-level Compaction Algorithm.
        let outcome = sparq_jsonld::compact::compact(&input_json, &ctx_json, &opts, &loader);

        // 4b. NegativeEvaluationTest: pass iff compact() (expansion + context
        //     processing + compaction) raises EXACTLY the manifest's
        //     `expectErrorCode` — the same shape as the frame and expand lanes.
        if is_negative {
            let want_code = e.expect_error_code.as_deref().unwrap_or("");
            match &outcome {
                Err(err) if err.code().as_str() == want_code => {
                    s.pass();
                    strict.pass();
                }
                Err(err) => {
                    let why = format!(
                        "expected error {want_code:?}, got {:?}",
                        err.code().as_str()
                    );
                    s.fail(&e.id, why.clone());
                    strict.fail(&e.id, why);
                }
                Ok(_) => {
                    let why = format!("expected error {want_code:?}, got success");
                    s.fail(&e.id, why.clone());
                    strict.fail(&e.id, why);
                }
            }
            continue;
        }

        let Some(expect_rel) = &e.expect else {
            unreachable!("positive cases without an `expect` document are skipped above")
        };
        let compacted = match outcome {
            Ok(j) => j,
            Err(why) => {
                let why = format!("compact() error: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };

        // 5. Compare against the suite's NORMATIVE expected document.
        let got: Value = match sparq_json_to_serde(&compacted) {
            Ok(v) => v,
            Err(why) => {
                let why = format!("convert compact output: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        let expect_path = root.join(expect_rel);
        let exp_text = match std::fs::read_to_string(&expect_path) {
            Ok(t) => t,
            Err(why) => {
                let why = format!("read expect: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        let want: Value = match serde_json::from_str(&exp_text) {
            Ok(v) => v,
            Err(why) => {
                let why = format!("parse expect JSON: {}", why);
                s.fail(&e.id, why.clone());
                strict.fail(&e.id, why);
                continue;
            }
        };
        if json_ld_equal_strict_order(&got, &want) {
            strict.pass();
        } else {
            strict.fail(
                &e.id,
                "compacted JSON differs under strict array ordering".to_string(),
            );
        }
        if json_ld_equal(&got, &want) {
            s.pass();
        } else {
            // Truncated got/want dumps keep a regression diagnosable from the
            // failure log without re-running locally.
            let mut g = got.to_string();
            let mut w = want.to_string();
            g.truncate(240);
            w.truncate(240);
            s.fail(
                &e.id,
                format!("compacted JSON differs\n    got:  {}\n    want: {}", g, w),
            );
        }
    }
    CompactScores {
        semantic: s,
        strict,
    }
}

/// JSON-LD scalar/object equality with every array compared as a sequence.
///
/// Numeric comparison deliberately matches [`json_ld_equal`] (`1` equals
/// `1.0`), isolating array order as the only difference from the gating oracle.
fn json_ld_equal_strict_order(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xi), Some(yi)) = (x.as_i64(), y.as_i64()) {
                xi == yi
            } else if let (Some(xu), Some(yu)) = (x.as_u64(), y.as_u64()) {
                xu == yu
            } else {
                match (x.as_f64(), y.as_f64()) {
                    (Some(xf), Some(yf)) => xf == yf,
                    _ => x == y,
                }
            }
        }
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|(x, y)| json_ld_equal_strict_order(x, y))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs.iter().all(|(key, x)| {
                    ys.get(key)
                        .is_some_and(|y| json_ld_equal_strict_order(x, y))
                })
        }
        _ => false,
    }
}

#[test]
fn strict_order_diagnostic_detects_unmarked_array_reordering() {
    let got: Value = serde_json::from_str(r#"{"items":[1,2]}"#).unwrap();
    let want: Value = serde_json::from_str(r#"{"items":[2,1]}"#).unwrap();

    assert!(json_ld_equal(&got, &want));
    assert!(!json_ld_equal_strict_order(&got, &want));
}

/// The single narrowly-pinned 1.0-only skip: compact `#t0038` and NOTHING else
/// (decision: bead sq-uzdw7; scope enforced by [`t0038_skip_is_narrowly_scoped`]).
///
/// Why this exact case is formally inapplicable to sparq (a JSON-LD 1.1 REC
/// processor), sourceable from the pinned suite itself:
///
/// * The suite's `vocab.jsonld` defines `jld:specVersion` as "the JSON-LD version
///   to which the test applies"; `#t0038` carries `specVersion: json-ld-1.0`, i.e.
///   it targets the 2014 JSON-LD 1.0 REC's algorithms, which sparq does not
///   implement (sparq's `ProcessingMode::JsonLd10` is the 1.1 REC's DEFINITION of
///   1.0 processing mode, not the 1.0-REC algorithms).
/// * Its expected document mints compact IRIs (`title:/value`, `body:/format`)
///   from EXPANDED (map-valued) term definitions — but the 1.1 REC's IRI
///   Compaction algorithm only considers terms whose prefix flag is true, which is
///   never set for a map-valued term definition, in EITHER processing mode:
///   `#tp001` ("Compact IRI will not use an expanded term definition in 1.0", a
///   1.1-suite test run under `processingMode: json-ld-1.0`) pins exactly that, so
///   one implementation cannot pass both. jsonld.js and pyld make the same trade.
///
/// The match is by exact manifest id AND its upstream `specVersion` marker, so a
/// suite-pin bump that changes either makes the case RUN again (fail loudly)
/// instead of extending the exception.
fn is_pinned_1_0_only_skip(e: &Entry) -> bool {
    e.id == "#t0038" && e.spec_version.as_deref() == Some("json-ld-1.0")
}

/// Regression pin for the scope of the `#t0038` skip (bead sq-uzdw7): exactly
/// `#t0038` is skipped by [`is_pinned_1_0_only_skip`], the
/// `processingMode: json-ld-1.0` positives still EXECUTE, and `#t0038` stays the
/// only `specVersion: json-ld-1.0` positive in the manifest — so a suite-pin bump
/// that adds another 1.0-only positive fails HERE and forces a deliberate
/// decision rather than a silent skip.
#[test]
fn t0038_skip_is_narrowly_scoped() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C JSON-LD suite not present at {} — run scripts/fetch-jsonld-tests.sh",
            root.display()
        );
        return;
    }
    let entries = read_manifest(&root, "compact").expect("read compact manifest");

    // (a) The skip predicate matches exactly #t0038.
    let skipped: Vec<&str> = entries
        .iter()
        .filter(|e| is_pinned_1_0_only_skip(e))
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        skipped,
        ["#t0038"],
        "the pinned 1.0-only skip must cover exactly #t0038"
    );

    // (b) The processingMode=json-ld-1.0 positives are NOT skipped: they clear
    // every skip gate `run_compact` applies, so they EXECUTE the native compactor
    // in JSON-LD 1.0 processing mode.
    for id in ["#t0075", "#t0106", "#tp001"] {
        let e = entries
            .iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("{} missing from the pinned compact manifest", id));
        assert!(
            !is_pinned_1_0_only_skip(e)
                && e.requires.is_none()
                && !e.is_negative
                && e.expect.is_some()
                && e.context.is_some()
                && !e.input.starts_with("http://")
                && !e.input.starts_with("https://"),
            "{} (processingMode: json-ld-1.0 lineage) must RUN, not skip",
            id
        );
    }

    // (c) #t0038 is the ONLY specVersion=json-ld-1.0 positive at the pin. If a
    // suite-pin bump adds another, this assertion fires: decide it explicitly
    // (fix, or extend the pin with its own justification) — never silently skip.
    let one_zero_positives: Vec<&str> = entries
        .iter()
        .filter(|e| !e.is_negative && e.spec_version.as_deref() == Some("json-ld-1.0"))
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        one_zero_positives,
        ["#t0038"],
        "a new specVersion=json-ld-1.0 positive appeared — decide it explicitly"
    );
}

/// [OPUS-5] sq-gzsky — scope pin for the compact negative lane, the sibling of the expand
/// lane's `expand_negatives_all_run_with_an_expected_code`. Every `NegativeEvaluationTest`
/// in the pinned `compact` manifest must carry an `expectErrorCode` (without one the runner
/// compares against `""`, which no spec code equals, so the case could only ever FAIL) and
/// must clear every skip gate `run_compact` applies — including the `context` member the
/// compaction call needs. A suite-pin bump that adds a negative behind `requires`, at a
/// remote URL, or without a caller context fails HERE rather than quietly shrinking the
/// gated set.
#[test]
fn compact_negatives_all_run_with_an_expected_code() {
    let root = suite_root();
    if !root.exists() {
        eprintln!(
            "SKIP: W3C JSON-LD suite not present at {} — run scripts/fetch-jsonld-tests.sh",
            root.display()
        );
        return;
    }
    let entries = read_manifest(&root, "compact").expect("read compact manifest");
    let negatives: Vec<_> = entries.iter().filter(|e| e.is_negative).collect();
    assert!(
        !negatives.is_empty(),
        "the pinned compact manifest must contain NegativeEvaluationTests — an empty set \
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
                && e.context.is_some()
                && !is_pinned_1_0_only_skip(e)
                && !e.input.starts_with("http://")
                && !e.input.starts_with("https://"),
            "{} would be SKIPPED rather than run — decide it explicitly",
            e.id
        );
    }
}

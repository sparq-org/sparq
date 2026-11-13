//! sq-oy1f.2 — manifest-driven runner for the official W3C JSON-LD
//! 1.1 API test suite (`w3c/json-ld-api`, `tests/`), wired as a RATCHETED
//! conformance gate that mirrors the SPARQL / SHACL / GeoSPARQL / Solid ratchets
//! in this crate (crate-local `cargo test` + a pinned pass-count FLOOR that may
//! only RISE, registered in the central `scoreboard::SUITES`).
//!
//! sq-oy1f.40 — this file is now a THIN ROOT: the per-lane runners live
//! in `tests/jsonld_suite/{common,to_rdf,from_rdf,expand,compact,flatten,frame}.rs`
//! as SUBMODULES of this one test binary (the CI invocation
//! `cargo test -p sparq-conformance --features jsonld-suite --test jsonld_suite`
//! is unchanged), and the eight ratchet FLOORS live LIB-SIDE
//! in `sparq_conformance::floors::<lane>` so the runner's `assert!(pass >= FLOOR)`
//! and `scoreboard::SUITES`' `ratchet_floor` read the SAME compile-time const (no
//! textual drift — the #1463 floor-drift class is killed structurally). The
//! `ci.yml` `jsonld-conformance` job greps each floor from `src/floors/<lane>.rs`.
//!
//! ## What is gated (v1) — only what sparq runs TODAY
//!
//! * **toRdf** (JSON-LD → RDF) — see `jsonld_suite/to_rdf.rs`.
//! * **fromRdf** (RDF → JSON-LD, round-trip) — see `jsonld_suite/from_rdf.rs`.
//! * **compact** (native document-level `sparq_jsonld::compact::compact()` oracle
//!   against the W3C expected document — sq-oy1f.27) — see
//!   `jsonld_suite/compact.rs`.
//! * **frame** (RDF → framed JSON-LD over the SEPARATE `w3c/json-ld-framing`
//!   suite) — see `jsonld_suite/frame.rs`.
//! * **expand** (native document-level `sparq_jsonld::expand()` oracle) — see
//!   `jsonld_suite/expand.rs`.
//! * **flatten** (native document-level flattened JSON-LD) — see
//!   `jsonld_suite/flatten.rs`.
//!
//! Each submodule's own doc-comment carries its oracle, its honest SKIP buckets,
//! and its measured-count caveat; the ratchet floor each asserts lives on the
//! matching `sparq_conformance::floors::<lane>::FLOOR` const with its own
//! documentation. HTML and remote documents run through native document operations and
//! manifest-driven local responses. EARL includes every entry except those with
//! specVersion or processingMode json-ld-1.0 (not applicable); unexecuted entries fail.
//!
//! ## Feature gating (both states)
//!
//! The whole lane is behind this crate's opt-in `jsonld-suite` feature
//! (forwards to `sparq-core/jsonld`). With the
//! feature OFF this file compiles to a single self-SKIP `#[test]`.
//! With it ON the runner executes and
//! asserts the pinned floors. The toRdf/fromRdf/compact fixtures are fetched by
//! `scripts/fetch-jsonld-tests.sh` into the gitignored `tests/w3c/json-ld-api/`,
//! and the frame fixtures by `scripts/fetch-jsonld-framing-tests.sh` into
//! `tests/w3c/json-ld-framing/`; when either is absent the runner SKIPS that lane
//! so a fresh offline checkout stays green.

#[cfg(feature = "jsonld-suite")]
#[path = "support/earl.rs"]
mod earl;

// When the lane feature is OFF this test binary contains only a self-SKIP test.
// The RDF writers are enabled independently for syntax C14N and Eval.
#[cfg(not(feature = "jsonld-suite"))]
#[test]
fn jsonld_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: W3C JSON-LD conformance lane is OFF — build with \
         `--features jsonld-suite` (and run scripts/fetch-jsonld-tests.sh) to run it."
    );
}

// sq-oy1f.40 — the per-lane submodules, in the flat `tests/jsonld_suite/`
// directory. This IS the test-binary ROOT file, whose module directory is `tests/`
// (not `tests/jsonld_suite/`), so each `#[path]` names the subdirectory explicitly.
// Each is feature-gated so the default + `--workspace` builds compile NONE of the
// JSON-LD runner code (the lean-core posture) — only the self-SKIP test above.
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/common.rs"]
mod common;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/compact.rs"]
mod compact;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/expand.rs"]
mod expand;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/flatten.rs"]
mod flatten;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/frame.rs"]
mod frame;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/from_rdf.rs"]
mod from_rdf;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/to_rdf.rs"]
mod to_rdf;
#[cfg(feature = "jsonld-suite")]
#[path = "jsonld_suite/remote.rs"]
mod remote;

#[cfg(feature = "jsonld-suite")]
mod gated {
    use crate::common::{
        frame_suite_root, suite_root, COMPACT_FLOOR, EXPAND_FLOOR,
        FLATTEN_FLOOR, FRAME_FLOOR, FROMRDF_FLOOR, TORDF_FLOOR,
    };
    use crate::{compact, expand, flatten, frame, from_rdf, to_rdf};

    #[test]
    fn jsonld_conformance_ratchet() {
        let root = suite_root();
        if !root.exists() {
            assert!(
                std::env::var_os("SPARQ_JSONLD_ROOT").is_none()
                    && std::env::var_os("SPARQ_EARL_OUT").is_none(),
                "configured JSON-LD suite missing"
            );
            eprintln!(
                "SKIP: W3C JSON-LD suite not present at {} — run scripts/fetch-jsonld-tests.sh",
                root.display()
            );
            return;
        }

        let tordf = to_rdf::run_tordf(&root);
        let fromrdf = from_rdf::run_fromrdf(&root);
        let compact_scores = compact::run_compact(&root);
        let compact = compact_scores.semantic;
        let compact_strict = compact_scores.strict;
        // sq-kk1mq — expand now uses the NATIVE DOCUMENT-LEVEL oracle
        // (sparq_jsonld::expand() + json_ld_equal comparator) instead of the old
        // RDF-equivalence oracle.  See expand::run_expand_native() and the expand
        // floor doc for the oracle-correction rationale and old-vs-new breakdown.
        let expand = expand::run_expand_native(&root);
        // sq-oy1f.26 — flatten now uses the NATIVE DOCUMENT-LEVEL oracle
        // (sparq_jsonld::flatten() + json_ld_equal comparator) — the native
        // Flattening Algorithm (§7.1) replaces the old RDF-writer round-trip. See
        // flatten::run_flatten_native() and the flatten floor doc for the old-vs-new
        // re-pin.
        let flatten = flatten::run_flatten_native(&root);
        let html = crate::remote::run(&root, "html");
        let remote_doc = crate::remote::run(&root, "remote-doc");
        for (cat, score, floor) in [
            ("html", &html, sparq_conformance::floors::html::FLOOR),
            ("remote-doc", &remote_doc, sparq_conformance::floors::remote_doc::FLOOR),
        ] {
            crate::common::report(&root, cat, score);
            println!("TOTAL {cat} {} {} {} (floor {floor})", score.pass, score.fail, score.skip);
            for (id, why) in &score.failures { println!("{cat} {id}: {why}"); }
        }

        // sq-oy1f.19 — framing lives in the SEPARATE w3c/json-ld-framing
        // suite (scripts/fetch-jsonld-framing-tests.sh), which a checkout may have
        // independently of json-ld-api. Run it only when present; otherwise the
        // `frame` line reports "suite absent" and the frame ratchet is not asserted
        // (a fresh offline checkout stays green). When present, the FRAME_FLOOR
        // ratchet is asserted below.
        let frame_root = frame_suite_root();
        let frame = frame_root.exists().then(|| frame::run_frame(&frame_root));
        // Emit before ratchet assertions, including local skips and unsupported buckets.
        for (cat, score) in [
            ("toRdf", &tordf),
            ("fromRdf", &fromrdf),
            ("compact", &compact),
            ("expand", &expand),
            ("flatten", &flatten),
        ] {
            crate::common::report(&root, cat, score);
        }
        if let Some(score) = &frame {
            crate::common::report(&frame_root, "frame", score);
        } else {
            assert!(
                std::env::var_os("SPARQ_JSONLD_FRAME_ROOT").is_none()
                    && std::env::var_os("SPARQ_EARL_OUT").is_none(),
                "configured JSON-LD framing suite missing"
            );
        }

        if std::env::var_os("SPARQ_EARL_OUT").is_some() {
            println!("EARL complete jsonld");
        }
        println!(
            "\nW3C JSON-LD 1.1 conformance scoreboard (pinned w3c/json-ld-api + json-ld-framing)"
        );
        println!(
            "{:<10} {:>5} {:>5} {:>5}",
            "category", "pass", "fail", "skip"
        );
        // The CI ratchet greps these `TOTAL <cat>` lines.
        println!(
            "TOTAL toRdf {} {} {} (floor {})",
            tordf.pass, tordf.fail, tordf.skip, TORDF_FLOOR
        );
        println!(
            "TOTAL fromRdf {} {} {} (floor {})",
            fromrdf.pass, fromrdf.fail, fromrdf.skip, FROMRDF_FLOOR
        );
        // sq-3uos5 — the compact ratchet. The CI grep depends on this
        // exact `^TOTAL compact ` prefix with the pass count in field $3.
        println!(
            "TOTAL compact {} {} {} (floor {})",
            compact.pass, compact.fail, compact.skip, COMPACT_FLOOR
        );
        // sq-ruktv — advisory only. Unlike `TOTAL compact`, CI does not
        // ratchet or assert this order-sensitive diagnostic. It makes an
        // unmarked compacted-list order regression visible in job output.
        println!(
            "TOTAL compact-strict {} {} {} (advisory)",
            compact_strict.pass, compact_strict.fail, compact_strict.skip
        );
        // sq-oy1f — the expand + flatten ratchets. The CI grep depends on
        // these exact `^TOTAL expand `/`^TOTAL flatten ` prefixes with the pass count
        // in field $3 (same shape as the other lanes).
        println!(
            "TOTAL expand {} {} {} (floor {})",
            expand.pass, expand.fail, expand.skip, EXPAND_FLOOR
        );
        println!(
            "TOTAL flatten {} {} {} (floor {})",
            flatten.pass, flatten.fail, flatten.skip, FLATTEN_FLOOR
        );
        // sq-oy1f.19 — the frame ratchet. The CI grep depends on this
        // exact `^TOTAL frame ` prefix with the pass count in field $3 (same shape
        // as the other lanes). Printed only when the framing suite is present.
        if let Some(frame) = &frame {
            println!(
                "TOTAL frame {} {} {} (floor {})",
                frame.pass, frame.fail, frame.skip, FRAME_FLOOR
            );
        } else {
            println!(
                "frame      (suite absent — run scripts/fetch-jsonld-framing-tests.sh; floor {})",
                FRAME_FLOOR
            );
        }
        if !tordf.failures.is_empty() {
            println!("\ntoRdf failures (first 40):");
            for (id, why) in tordf.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if !fromrdf.failures.is_empty() {
            println!("\nfromRdf failures (first 40):");
            for (id, why) in fromrdf.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if !compact.failures.is_empty() {
            println!("\ncompact failures (first 40):");
            for (id, why) in compact.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if !compact_strict.failures.is_empty() {
            println!("\ncompact strict-order diagnostics (first 40; advisory):");
            for (id, why) in compact_strict.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if !expand.failures.is_empty() {
            println!("\nexpand failures (first 40):");
            for (id, why) in expand.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if !flatten.failures.is_empty() {
            println!("\nflatten failures (first 40):");
            for (id, why) in flatten.failures.iter().take(40) {
                println!("  {}: {}", id, why);
            }
        }
        if let Some(frame) = &frame {
            if !frame.failures.is_empty() {
                println!("\nframe failures (first 40):");
                for (id, why) in frame.failures.iter().take(40) {
                    println!("  {}: {}", id, why);
                }
            }
        }

        assert!(html.pass >= sparq_conformance::floors::html::FLOOR, "HTML pass count regressed");
        assert!(html.fail <= sparq_conformance::floors::html::FAIL_CEILING, "HTML failure ceiling exceeded");
        assert!(remote_doc.pass >= sparq_conformance::floors::remote_doc::FLOOR, "remote-document pass count regressed");
        assert_eq!(remote_doc.fail, sparq_conformance::floors::remote_doc::FAIL_CEILING, "remote-document failure ceiling exceeded");

        // The ratchet: pass counts may only RISE. A regression below the pinned
        // floor fails the build.
        assert!(
            tordf.pass >= TORDF_FLOOR,
            "JSON-LD toRdf pass count regressed: {} < floor {} — see failures above",
            tordf.pass,
            TORDF_FLOOR
        );
        assert!(
            fromrdf.pass >= FROMRDF_FLOOR,
            "JSON-LD fromRdf pass count regressed: {} < floor {} — see failures above",
            fromrdf.pass,
            FROMRDF_FLOOR
        );
        // sq-3uos5 — the compact ratchet (lossless round-trip floor).
        assert!(
            compact.pass >= COMPACT_FLOOR,
            "JSON-LD compact pass count regressed: {} < floor {} — see failures above",
            compact.pass,
            COMPACT_FLOOR
        );
        // sq-oy1f — the expand + flatten ratchets (normative
        // answer-equivalence floors).
        assert!(
            expand.pass >= EXPAND_FLOOR,
            "JSON-LD expand pass count regressed: {} < floor {} — see failures above",
            expand.pass,
            EXPAND_FLOOR
        );
        assert!(
            flatten.pass >= FLATTEN_FLOOR,
            "JSON-LD flatten pass count regressed: {} < floor {} — see failures above",
            flatten.pass,
            FLATTEN_FLOOR
        );
        // sq-oy1f.19 — the frame ratchet (normative answer-equivalence
        // floor), asserted only when the separate framing suite is present.
        if let Some(frame) = &frame {
            assert!(
                frame.pass >= FRAME_FLOOR,
                "JSON-LD frame pass count regressed: {} < floor {} — see failures above",
                frame.pass,
                FRAME_FLOOR
            );
        }
    }
}

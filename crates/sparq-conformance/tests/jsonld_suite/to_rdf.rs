//! sq-oy1f.40 — the W3C JSON-LD 1.1 `toRdf` lane runner (split out of the
//! former monolithic `tests/jsonld_suite.rs`; behaviour byte-identical).
//!
//! **toRdf** (JSON-LD → RDF): each `jld:ToRDFTest` input is driven through the REAL
//! parse path — `sparq_core::Graph::load_*` over `oxjsonld` (the `jsonld` feature).
//! The produced RDF is compared, by blank-node-canonical dataset isomorphism,
//! against the suite's expected N-Quads. Negative tests (`jld:NegativeEvaluationTest`)
//! pass when the parse fails.

use super::common::*;
use std::path::Path;

/// Run the toRdf category. Returns the scoreboard.
pub fn run_tordf(root: &Path) -> Score {
    let mut s = Score::default();
    let entries = match read_manifest(root, "toRdf") {
        Ok(e) => e,
        Err(why) => {
            s.fail("toRdf-manifest", why);
            return s;
        }
    };
    for e in &entries {
        // Skip optional-feature tests sparq does not opt into (see `requires`).
        if e.requires.is_some() {
            s.skip();
            continue;
        }
        let input_path = root.join(&e.input);
        let text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read input: {why}"));
                continue;
            }
        };
        let base = doc_base(e);
        // drive the REAL ingest path: load through sparq-core /
        // oxjsonld, then read the parsed quads back out for comparison.
        let parsed = parse_jsonld_dataset(&text, &base);
        if e.is_negative {
            // A NegativeEvaluationTest passes iff the parse fails (sparq
            // rejects the malformed/illegal document).
            match parsed {
                Err(_) => s.pass(),
                Ok(_) => s.fail(&e.id, "negative test parsed without error".into()),
            }
            continue;
        }
        let Some(expect) = &e.expect else {
            s.skip();
            continue;
        };
        let got = match parsed {
            Ok(ds) => ds,
            Err(why) => {
                s.fail(&e.id, format!("parse: {why}"));
                continue;
            }
        };
        let exp_text = match std::fs::read_to_string(root.join(expect)) {
            Ok(t) => t,
            Err(why) => {
                s.fail(&e.id, format!("read expect: {why}"));
                continue;
            }
        };
        let want = match nquads_to_canonical_dataset(&exp_text) {
            Ok(ds) => ds,
            Err(why) => {
                s.fail(&e.id, format!("parse expect: {why}"));
                continue;
            }
        };
        if got == want {
            s.pass();
        } else {
            s.fail(
                &e.id,
                format!("RDF mismatch ({} vs {} quads)", got.len(), want.len()),
            );
        }
    }
    s
}

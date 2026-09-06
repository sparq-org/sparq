//! [SONNET-4.6] sq-6qpyf (epic sq-pbz04) — the NAMED-GRAPH (`qt:graphData`)
//! census of the W3C `sparql11/entailment` suite: a TRIPWIRE, not a ratchet.
//!
//! ## What it pins, and why it exists
//!
//! `inference::sparql_entail::run_one` holds any entailment test whose action
//! carries a `qt:graphData` named-graph dataset, fail-closed, because the runner
//! materializes a SINGLE default-graph closure — running a named-graph dataset
//! through that path would compute the WRONG closure (per-graph entailment
//! semantics live in `sparq-reason` and are unimplemented). At the pinned
//! rdf-tests revision the suite has ZERO such entries, so the guard is
//! unreachable and nothing is skip-laundered.
//!
//! That "zero at the pin" was a PROSE claim in a code comment. This test makes it
//! a CHECKED one. If a future rdf-tests bump adds a named-graph entailment case,
//! this goes red — deliberately. The correct response is NOT to relax the guard
//! (the case would still be honestly reported OutOfScope by the binary, so no
//! conformance number is faked either way); it is to implement per-graph
//! materialization in `sparq-reason` under the reasoner-program epic sq-pbz04,
//! then graduate the case and re-pin this census. The tripwire's whole job is to
//! stop such a case from arriving unnoticed and sitting silently in the
//! out-of-scope bucket.
//!
//! Scope note: this asserts a property of the SUITE (a census), not a pass count
//! — it registers no `scoreboard::SUITES` row and makes no conformance claim.
//!
//! The rdf-tests fixtures are fetched by `scripts/fetch-inference-suites.sh` into
//! the gitignored `tests/w3c/rdf-tests/`; when absent this SKIPS so a fresh
//! offline checkout stays green (CI fetches them in the `inference-conformance`
//! job). Default feature state — no lane feature required.

use sparq_conformance::manifest::{self, EntryKind, TestEntry};
use std::path::PathBuf;

#[test]
fn entailment_suite_has_no_named_graph_cases_at_the_pinned_revision() {
    // CARGO_MANIFEST_DIR is crates/sparq-conformance; the clone lives at the
    // workspace-root `tests/w3c/rdf-tests` (the same one the SPARQL + inference
    // harnesses use).
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdf-tests");
    let Ok(suites_root) = root.join("sparql").canonicalize() else {
        eprintln!(
            "SKIP: rdf-tests clone absent — run scripts/fetch-inference-suites.sh to \
             populate tests/w3c/rdf-tests/."
        );
        return;
    };
    let manifest_path = suites_root.join("sparql11/entailment/manifest.ttl");
    if !manifest_path.is_file() {
        eprintln!("SKIP: sparql11/entailment manifest absent from the rdf-tests clone.");
        return;
    }

    let mut entries: Vec<TestEntry> = Vec::new();
    manifest::collect(&manifest_path, &suites_root, &mut entries)
        .expect("collect the sparql11/entailment manifest");

    // Non-vacuity: an empty (or unparsed) manifest must not read as "zero
    // named-graph cases" — that would make the census silently meaningless.
    assert!(
        !entries.is_empty(),
        "the sparql11/entailment manifest parsed to ZERO entries — the census below \
         would be vacuous; the fixture or the manifest parser is broken."
    );

    let named: Vec<&str> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::QueryEval && !e.action.graph_data.is_empty())
        .map(|e| e.name.as_str())
        .collect();

    // Positional `format!` args per the CodeQL `rust/unused-variable` guard.
    assert!(
        named.is_empty(),
        "the pinned rdf-tests revision now has {} named-graph (qt:graphData) \
         entailment case(s): {:?}. They are currently HELD fail-closed by \
         inference::sparql_entail::run_one (honest, not skip-laundered), because the \
         runner materializes a single default-graph closure. Do NOT relax that guard \
         to make them run: wiring them needs per-graph materialization semantics in \
         sparq-reason (epic sq-pbz04, bead sq-6qpyf), not just harness wiring. Once \
         that lands, graduate the cases and re-pin this census.",
        named.len(),
        named
    );
}

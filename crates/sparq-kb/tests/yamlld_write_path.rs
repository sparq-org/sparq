//! Write-path YAML-LD authoring form — the round-trip + SHACL-conformance gate
//! (FO-bridge Phase 4, sq-mztg8.2; epic sq-mztg8).
//!
//! 🤖 SPARQ agent. Design record: `research/fo-llm-bridge.md` §3.4 / §6
//! Phase 4 (#1112). Written while Fable unavailable; flag for re-review when Fable
//! returns.
//!
//! The write-path lets the AGENTS.md Findings tier be AUTHORED in a compact, IRI-free
//! `ingest/agents-findings.yaml.ld` and deterministically COMPILED (by
//! `ingest/yamlld_compile.py`) to the schema.org-typed PKG Turtle the SHACL shapes
//! admit. This test pins the two load-bearing invariants of the bead's acceptance
//! criterion, over the REAL parse + SHACL path (not a mock):
//!
//!   1. **SHACL conformance (the acceptance gate / kill-criterion K5):** the COMPILED
//!      Findings tier (`PKG_FINDINGS`) conforms to `pkg.shapes.ttl` with **0
//!      violations** — at parity with the hand-authored tier it replaces.
//!   2. **Round-trip equivalence:** the compiled Findings carry the SAME triple set as
//!      the frozen, hand-authored `agents-findings.ttl` the human wrote before the
//!      write-path existed (the fixture in `tests/fixtures/`). So the deterministic
//!      compiler reproduces the hand-authored tier — it did not quietly drop or invent
//!      a fact.
//!
//! (The compiler's parser / guarded-`V()` resolver / ambiguous-token-is-a-hard-error
//! behaviour is regression-covered by the hermetic Python self-test
//! `scripts/tests/test_yamlld_compile.py`, run in the docs-quality lane.)
//!
//! Run: `cargo test -p sparq-kb --features validate --test yamlld_write_path -- --nocapture`
#![cfg(feature = "validate")]

use std::collections::BTreeSet;

use sparq_kb::validate::{parse_turtle, validate_instances};
use sparq_kb::PKG_FINDINGS;

const FIXTURE_HANDAUTHORED: &str =
    include_str!("fixtures/agents-findings.handauthored.ttl");

const BASE: &str = "https://sparq.dev/ns/pkg/example#";

/// A canonical, order-independent string form of a triple, for set comparison.
fn triple_set(ttl: &str) -> BTreeSet<String> {
    parse_turtle(ttl, BASE)
        .expect("turtle parses")
        .into_iter()
        .map(|t| t.to_string())
        .collect()
}

/// (1) The COMPILED Findings tier conforms to the PKG SHACL shapes with 0 violations —
/// the bead's acceptance gate (K5: "hold SHACL conformance at parity with the
/// hand-authored tier"). Loads the compiled `PKG_FINDINGS` alone against the ontology +
/// shapes (the same path the full ingest uses).
#[test]
fn compiled_findings_conform_zero_violations() {
    let report =
        validate_instances(&[PKG_FINDINGS]).expect("compiled findings load + validate");
    assert!(
        report.conforms && report.results.is_empty(),
        "the write-path-COMPILED Findings tier MUST conform with 0 SHACL violations \
         (K5: parity with the hand-authored tier), but got {} result(s):\n{}",
        report.results.len(),
        report.to_text()
    );
    eprintln!(
        "=== write-path compiled findings: {} violations (conforms={}) ===",
        report.results.len(),
        report.conforms
    );
}

/// (2) Round-trip equivalence: the compiled tier carries EXACTLY the same triples as
/// the frozen hand-authored tier it replaces. This is the "compile reconstructs the
/// hand-authored facts" property — the deterministic compiler neither dropped nor
/// invented a triple. (Comparison is over the canonical triple SET, so any cosmetic
/// re-ordering / re-formatting the compiler does is irrelevant; only the RDF content
/// is compared.)
#[test]
fn compiled_findings_match_handauthored_triple_for_triple() {
    let compiled = triple_set(PKG_FINDINGS);
    let handauthored = triple_set(FIXTURE_HANDAUTHORED);

    let missing: Vec<_> = handauthored.difference(&compiled).cloned().collect();
    let extra: Vec<_> = compiled.difference(&handauthored).cloned().collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "write-path compile is NOT triple-equivalent to the hand-authored tier.\n\
         {} triple(s) the hand-authored tier had but the compile dropped:\n  {}\n\
         {} triple(s) the compile invented that the hand-authored tier lacked:\n  {}",
        missing.len(),
        missing.join("\n  "),
        extra.len(),
        extra.join("\n  "),
    );
    eprintln!(
        "=== round-trip: compiled tier == hand-authored tier ({} triples) ===",
        compiled.len()
    );
}

/// Sanity: the compiled tier actually carries the expected Findings (so the two
/// invariants above are not vacuously true over an empty graph). 11 Findings + the
/// Source + 3 concept Topics were authored in the `.yaml.ld`.
#[test]
fn compiled_findings_are_non_empty() {
    let n = PKG_FINDINGS.matches("a pkg:Finding").count();
    assert_eq!(
        n, 11,
        "expected 11 compiled pkg:Finding nodes, got {} — the write-path source or \
         compiler regressed",
        n
    );
}

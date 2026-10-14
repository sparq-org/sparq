//! (sq-bif.10) Feature-state coverage for the SHACL Advanced Features
//! (`shacl-af`) surface — the part the bead calls out as unmet: `src/rules.rs`
//! had only inline `#[cfg(test)]` tests (which compile and run ONLY with the
//! feature on), so the AGENTS.md "tests in BOTH feature states" standard was not
//! met for the rule + expression surface.
//!
//! This file is intentionally NOT feature-gated, so it compiles and runs in both
//! states. It pins two load-bearing invariants:
//!
//!   * **Default (feature OFF)** — the rule/expression surface is cleanly compiled
//!     out. A shape carrying `sh:rule` validates with the base [`validate`] path
//!     without the rule firing and without a panic (rules are an inference step,
//!     never part of `validate`), and a `sh:expression`-bearing shape that *would*
//!     be a violation with the feature on instead conforms (the constraint
//!     component is not compiled in). The no-rule-firing assertions hold REGARDLESS
//!     of feature, so they run in both states; the "expression compiled out" vs
//!     "expression fires" assertions are split per feature so each state pins the
//!     correct behaviour.
//!   * **Feature ON (`--features shacl-af`)** — a representative `sh:rule`
//!     (`sh:TripleRule` + `sh:SPARQLRule`) is exercised end-to-end through the
//!     PUBLIC `apply_rules` / `expand` API (not the crate-internal inline tests),
//!     and a `sh:expression` constraint is exercised through `validate`.

use sparq_core::Graph;

const PRELUDE: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
    @prefix ex: <http://example.org/> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n";

fn g(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PRELUDE}{ttl}"), "turtle").unwrap()
}

// A shape that carries a `sh:TripleRule` AND a base Core constraint. The rule, if
// it fired during `validate`, would mint `(this, rdf:type, ex:Agent)` triples; the
// Core `sh:minCount` is what `validate` is actually responsible for.
const RULE_SHAPE: &str = r#"
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:name ; sh:minCount 1 ] ;
      sh:rule [
        a sh:TripleRule ;
        sh:subject sh:this ;
        sh:predicate rdf:type ;
        sh:object ex:Agent ;
      ] .
"#;

// ---- BOTH feature states: rules are inert in the base `validate` path ----

/// A shape carrying `sh:rule` validates through the base [`validate`] path with no
/// panic, and the rule does NOT fire — i.e. the Core constraints are evaluated
/// exactly as if the `sh:rule` triple were absent. This is the load-bearing
/// invariant the bead asks for and it must hold in BOTH feature states (rule
/// inference is a separate, opt-in step; it is never part of `validate`).
#[test]
fn sh_rule_shape_validates_without_firing_when_conforming() {
    // ex:alice has a name -> the Core sh:minCount passes -> conforms. (If the rule
    // were wrongly woven into `validate`, the result set or conformance would
    // differ from the rule-free expectation; it must not.)
    let data = g("ex:alice a ex:Person ; ex:name \"Alice\" .");
    let shapes = g(RULE_SHAPE);
    let r = sparq_shacl::validate(&data, &shapes);
    assert!(r.conforms, "rule must not affect base validation: {}", r.to_text());
    assert_eq!(r.results.len(), 0, "{}", r.to_text());
}

/// The same `sh:rule` shape over data that violates only the BASE Core constraint:
/// exactly one `sh:minCount` violation is reported (the rule contributes nothing —
/// no extra results, no inferred-type side effect on validation). Holds in both
/// feature states.
#[test]
fn sh_rule_shape_reports_only_base_constraint_violations() {
    // ex:bob has no ex:name -> exactly one MinCount violation, regardless of the
    // attached sh:rule.
    let data = g("ex:bob a ex:Person .");
    let shapes = g(RULE_SHAPE);
    let r = sparq_shacl::validate(&data, &shapes);
    assert!(!r.conforms);
    assert_eq!(r.results.len(), 1, "exactly the base violation: {}", r.to_text());
    assert!(
        r.results[0]
            .source_component
            .ends_with("MinCountConstraintComponent"),
        "only the Core constraint fires, not the rule: {}",
        r.to_text()
    );
}

/// An `sh:SPARQLRule` (CONSTRUCT) attached to a shape is likewise inert under
/// `validate`: the focus node conforms (no Core constraints) and the constructed
/// triple is NOT injected into the report. Both feature states.
#[test]
fn sh_sparql_rule_does_not_run_under_validate() {
    let data = g("ex:alice a ex:Person ; ex:firstName \"Alice\" .");
    let shapes = g(r#"
        ex:S a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:rule [
            a sh:SPARQLRule ;
            sh:construct """
              CONSTRUCT { $this <http://example.org/label> ?n }
              WHERE { $this <http://example.org/firstName> ?n }
            """ ;
          ] .
    "#);
    let r = sparq_shacl::validate(&data, &shapes);
    // No Core constraints + rules never run during validate => conforms, no results.
    assert!(r.conforms, "SPARQLRule must not run under validate: {}", r.to_text());
    assert_eq!(r.results.len(), 0, "{}", r.to_text());
}

// ---- Feature OFF: the expression-constraint component is compiled out ----

/// With `shacl-af` OFF, `sh:expression false` (which would be an
/// `sh:ExpressionConstraintComponent` violation for every targeted node when the
/// feature is on) is silently ignored — the shape conforms. This is the
/// non-vacuous "cleanly compiled out" assertion for the default state: the same
/// shape gives the OPPOSITE conformance with the feature on (see the gated test
/// below), so this assertion fails if the component were ever wired into the
/// default build.
#[cfg(not(feature = "shacl-af"))]
#[test]
fn expression_constraint_is_inert_when_feature_off() {
    let data = g("ex:i a ex:C .");
    let shapes = g("ex:C a rdfs:Class, sh:NodeShape ; sh:expression false .");
    let r = sparq_shacl::validate(&data, &shapes);
    assert!(
        r.conforms,
        "sh:expression must be compiled out (no violation) with shacl-af off: {}",
        r.to_text()
    );
    assert_eq!(r.results.len(), 0, "{}", r.to_text());
}

// ---- Feature ON: the rule + expression surface works end-to-end ----

/// With `shacl-af` ON, the PUBLIC rule-inference API (`apply_rules`) runs the
/// `sh:TripleRule` end-to-end: every `ex:Person` focus node gains the inferred
/// `(this, rdf:type, ex:Agent)` triple. `expand` then yields a fresh graph of
/// data ∪ inferred — verified behaviourally by validating the expanded graph
/// against a shape that conforms only because the inferred type is present.
#[cfg(feature = "shacl-af")]
#[test]
fn apply_rules_fires_triple_rule_when_feature_on() {
    let data = g("ex:alice a ex:Person ; ex:name \"Alice\" . ex:bob a ex:Person ; ex:name \"Bob\" .");
    let shapes = g(RULE_SHAPE);
    let inf = sparq_shacl::apply_rules(&data, &shapes);
    // One inferred type triple per ex:Person focus node.
    assert_eq!(inf.triples.len(), 2, "{:?}", inf.triples);
    let agent = "http://example.org/Agent";
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    for subj in ["http://example.org/alice", "http://example.org/bob"] {
        assert!(
            inf.triples.iter().any(|t| t.subject.to_string() == format!("<{}>", subj)
                && t.predicate.as_str() == rdf_type
                && t.object.to_string() == format!("<{}>", agent)),
            "expected inferred type for {subj}: {:?}",
            inf.triples
        );
    }
    // `expand` merges the inferred triples into a fresh graph. Confirm the inferred
    // `rdf:type ex:Agent` is now queryable: a shape targeting ex:Agent (which NO
    // node carries in the source data) selects the two focus nodes only because
    // the rule's inference is present in the expanded graph.
    let expanded = sparq_shacl::expand(&data, &shapes);
    let agent_shape = g(
        "ex:AgentCheck a sh:NodeShape ; sh:targetClass ex:Agent ; \
         sh:property [ sh:path ex:name ; sh:minCount 1 ] .",
    );
    let model = sparq_shacl::ShapesModel::parse(&agent_shape);
    // No ex:Agent instances in the un-expanded data -> zero focus nodes.
    assert_eq!(
        sparq_shacl::count_focus_nodes(&data, &model),
        0,
        "source data has no ex:Agent before expansion"
    );
    // After expansion the two inferred ex:Agent instances are selectable.
    assert_eq!(
        sparq_shacl::count_focus_nodes(&expanded, &model),
        2,
        "expand merged the inferred ex:Agent type triples"
    );
}

/// With `shacl-af` ON, a `sh:SPARQLRule` (CONSTRUCT) infers its constructed triple
/// through the public `apply_rules` API.
#[cfg(feature = "shacl-af")]
#[test]
fn apply_rules_fires_sparql_rule_when_feature_on() {
    let data = g("ex:alice a ex:Person ; ex:firstName \"Alice\" .");
    let shapes = g(r#"
        ex:S a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:rule [
            a sh:SPARQLRule ;
            sh:construct """
              CONSTRUCT { $this <http://example.org/label> ?n }
              WHERE { $this <http://example.org/firstName> ?n }
            """ ;
          ] .
    "#);
    let inf = sparq_shacl::apply_rules(&data, &shapes);
    assert_eq!(inf.triples.len(), 1, "{:?}", inf.triples);
    let t = &inf.triples[0];
    assert_eq!(t.predicate.as_str(), "http://example.org/label");
    assert_eq!(t.object.to_string(), "\"Alice\"");
}

/// With `shacl-af` ON, the `sh:expression` CONSTRAINT (the dispatch arm in
/// `eval.rs`) does fire under `validate`: `sh:expression false` makes every
/// targeted node an `sh:ExpressionConstraintComponent` violation — the OPPOSITE
/// of the feature-off behaviour pinned above.
#[cfg(feature = "shacl-af")]
#[test]
fn expression_constraint_fires_when_feature_on() {
    let data = g("ex:i a ex:C .");
    let shapes = g("ex:C a rdfs:Class, sh:NodeShape ; sh:expression false .");
    let r = sparq_shacl::validate(&data, &shapes);
    assert!(!r.conforms, "sh:expression false must violate with shacl-af on: {}", r.to_text());
    assert_eq!(r.results.len(), 1, "{}", r.to_text());
    assert!(
        r.results[0]
            .source_component
            .ends_with("ExpressionConstraintComponent"),
        "{}",
        r.to_text()
    );
}

// [FABLE-5] sq-pbz04.4.4 (epic sq-pbz04.4) — integration tests for the L4 fragment-dispatch
// Direct-Semantics checker + entailment-by-refutation (design record
// research/owl2-direct-semantics-scoping.md §4).
//
// 🤖 SPARQ agent. Every test goes end-to-end through the REAL pipeline: Turtle → Dict →
// fail-closed L1 extraction → L2 profile dispatch → the RL/EL/QL/ALCH branch under test.
// The bead's three named acceptance cases are here: the divergence-guarded RL case
// (`rl_divergence_guard_*`), the EL skipped-axioms case (`el_skipped_axioms_*`), and the
// fresh-class refutation case (`entailment_role_assertion_*`). Verdicts are asserted as
// exact enum variants (mutation-robust), and each public API item has a direct test.

#![cfg(feature = "dispatch")]

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_dl::check::{
    Branch, ConsistencyVerdict, DirectChecker, EntailmentVerdict, UnknownReason,
};
use sparq_reason_dl::tableau::Budget;

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;

/// Parse one Turtle document.
fn parse(body: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(&format!("{}{}", PRE, body), "turtle").expect("parse")
}

/// Parse a premise and a conclusion document into ONE shared dict (the conclusion's ids are
/// re-interned into the premise dict term-by-term). Re-interning shares the blank-node label
/// space across both documents: each distinct label (`_:foo`) maps to one dict id regardless
/// of which document introduced it. Tests that use blank nodes in BOTH the premise and the
/// conclusion must therefore use disjoint blank-node labels to avoid aliasing distinct
/// anonymous individuals. [OPUS-4.8] sq-pbz04.4.6 — stale "blank-node-free" claim removed.
fn parse_two(premise: &str, conclusion: &str) -> (Dict, Vec<[Id; 3]>, Vec<[Id; 3]>) {
    let (mut dict, prem) = parse(premise);
    let (cdict, ctriples) = parse(conclusion);
    let concl = ctriples
        .iter()
        .map(|t| t.map(|id| dict.intern(&cdict.term(id))))
        .collect();
    (dict, prem, concl)
}

fn check(body: &str) -> (ConsistencyVerdict, Branch) {
    let (mut dict, triples) = parse(body);
    let out = DirectChecker::new().consistency(&mut dict, &triples);
    (out.verdict, out.branch)
}

/// Same dispatch, but with a tableau budget so starved that the sq-pbz04.4.8 fall-through
/// can never supply a verdict — so what comes back is exactly the OWNING profile branch's
/// own outcome. This is how the guard tests below keep asserting the guard itself (rule 2
/// of the fall-through: an abstaining tableau returns the owner's outcome UNCHANGED).
fn check_starved(body: &str) -> (ConsistencyVerdict, Branch) {
    let (mut dict, triples) = parse(body);
    let starved = DirectChecker::with_budget(Budget {
        max_nodes: 1,
        max_rule_applications: 0,
    });
    let out = starved.consistency(&mut dict, &triples);
    (out.verdict, out.branch)
}

fn entail(premise: &str, conclusion: &str) -> (EntailmentVerdict, Branch) {
    let (mut dict, prem, concl) = parse_two(premise, conclusion);
    let out = DirectChecker::new().entailment(&mut dict, &prem, &concl);
    (out.verdict, out.branch)
}

// -------------------------------------------------------------------------------------------
// Constructor / helper API (direct coverage of every public item)
// -------------------------------------------------------------------------------------------

#[test]
fn checker_constructors() {
    assert_eq!(DirectChecker::new(), DirectChecker::default());
    let pinned = Budget {
        max_nodes: 7,
        max_rule_applications: 9,
    };
    // with_budget produces a distinct checker (the budget is load-bearing state).
    assert_ne!(DirectChecker::with_budget(pinned), DirectChecker::new());
    assert_eq!(
        DirectChecker::with_budget(Budget::default()),
        DirectChecker::new()
    );
}

#[test]
fn consistency_verdict_helpers() {
    assert!(ConsistencyVerdict::Consistent.is_consistent());
    assert!(!ConsistencyVerdict::Consistent.is_inconsistent());
    assert!(ConsistencyVerdict::Inconsistent.is_inconsistent());
    assert!(!ConsistencyVerdict::Inconsistent.is_unknown());
    let unknown = ConsistencyVerdict::Unknown(UnknownReason::QlConsistencyPending);
    assert!(unknown.is_unknown());
    assert!(!unknown.is_consistent());
}

#[test]
fn entailment_verdict_helpers() {
    assert!(EntailmentVerdict::Entailed.is_entailed());
    assert!(!EntailmentVerdict::Entailed.is_not_entailed());
    assert!(EntailmentVerdict::NotEntailed.is_not_entailed());
    assert!(!EntailmentVerdict::NotEntailed.is_unknown());
    let unknown = EntailmentVerdict::Unknown(UnknownReason::ElTopGuard);
    assert!(unknown.is_unknown());
    assert!(!unknown.is_entailed());
}

// -------------------------------------------------------------------------------------------
// RL branch
// -------------------------------------------------------------------------------------------

#[test]
fn rl_consistent_past_clean_guard() {
    // Plain Horn subclass + assertion: in RL, no divergence-implicated construct, no
    // punning, no clash — the ONE shape where the RL branch may say Consistent.
    let (verdict, branch) = check(":A rdfs:subClassOf :B . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::RlMaterialization);
}

#[test]
fn rl_inconsistent_via_materialized_clash() {
    // x ∈ A ⊑ ⊥: the RL closure types x into owl:Nothing (cax-sco + cls-nothing) — an
    // Inconsistent verdict sound via checked-PR1.
    let (verdict, branch) = check(":A rdfs:subClassOf owl:Nothing . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
    assert_eq!(branch, Branch::RlMaterialization);
}

#[test]
fn rl_divergence_guard_disjointness_blocks_consistent() {
    // BEAD ACCEPTANCE CASE: in-RL, clash-free, but touching owl:disjointWith — a construct
    // implicated in DOCUMENTED_DIVERGENCES — so "no clash" must NOT read as Consistent.
    // Observed under a starved budget, where the sq-pbz04.4.8 fall-through cannot mask the
    // guard: the RL branch still owns the dispatch and still abstains.
    let (verdict, branch) = check_starved(":A owl:disjointWith :B . :x a :A .");
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::RlDivergenceGuard(_))
        ),
        "expected RlDivergenceGuard, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::RlMaterialization);
}

#[test]
fn rl_divergence_guard_complement_and_union() {
    // complementOf (RL super-CE position) and unionOf (RL sub-CE position) each trip the
    // guard on a clash-free in-RL ontology (starved budget — see above).
    let (v1, b1) = check_starved(":A rdfs:subClassOf [ owl:complementOf :B ] . :x a :A .");
    assert!(
        matches!(
            v1,
            ConsistencyVerdict::Unknown(UnknownReason::RlDivergenceGuard(_))
        ),
        "complementOf: expected RlDivergenceGuard, got {:?}",
        v1
    );
    assert_eq!(b1, Branch::RlMaterialization);
    let (v2, _) = check_starved("[ owl:unionOf (:A :B) ] rdfs:subClassOf :C .");
    assert!(
        matches!(
            v2,
            ConsistencyVerdict::Unknown(UnknownReason::RlDivergenceGuard(_))
        ),
        "unionOf: expected RlDivergenceGuard, got {:?}",
        v2
    );
}

#[test]
fn rl_divergence_guard_does_not_mask_inconsistency() {
    // Inconsistent stays sound EVEN with a guarded construct present: the guard narrows
    // Consistent only.
    let (verdict, branch) = check(":A owl:disjointWith :B . :x a :A . :x a :B .");
    assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
    assert_eq!(branch, Branch::RlMaterialization);
}

#[test]
fn rl_pr1_punning_abstains() {
    // :A is used both as a class (:x a :A) and as an individual (:A a :B) — usage-level
    // punning violates the Theorem PR1 preconditions, so BOTH verdicts are withheld.
    let (verdict, branch) = check(":x a :A . :A a :B .");
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::RlPr1Preconditions(_))
        ),
        "expected RlPr1Preconditions, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::RlMaterialization);
}

// -------------------------------------------------------------------------------------------
// EL branch
// -------------------------------------------------------------------------------------------

/// An EL ontology (∃-chain on the super side keeps it OUT of RL) nested deeper than the EL
/// classifier's decode depth, so the classifier reports a skipped axiom.
fn deep_el_body(depth: usize) -> String {
    let mut body = String::from(":A rdfs:subClassOf ");
    for _ in 0..depth {
        body.push_str("[ owl:onProperty :r ; owl:someValuesFrom ");
    }
    body.push_str(":B ");
    for _ in 0..depth {
        body.push_str("] ");
    }
    body.push('.');
    body
}

#[test]
fn el_skipped_axioms_abstains() {
    // BEAD ACCEPTANCE CASE: in-EL (L1 extracts it — nesting is under L1's 512 cap), but
    // the EL classifier skips the axiom (over its 256 decode depth): a skipped axiom could
    // BE the inconsistency, so the branch abstains rather than trusting the lattice.
    // Starved budget so the sq-pbz04.4.8 fall-through cannot mask the guard.
    let (verdict, branch) = check_starved(&deep_el_body(300));
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::ElSkippedAxioms(n)) if n >= 1
        ),
        "expected ElSkippedAxioms(>=1), got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::ElClassification);
}

#[test]
fn el_unapplied_abox_abstains() {
    // In-EL with an ABox: the TBox classifier neither applies nor counts assertions, so a
    // Consistent verdict would be unfounded — abstain with the unapplied-kind reason
    // (starved budget — see above).
    let (verdict, branch) = check_starved(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . :x a :A .",
    );
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::ElUnappliedAxioms(_))
        ),
        "expected ElUnappliedAxioms, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::ElClassification);
}

#[test]
fn el_top_guard_never_calls_thing_bottom_consistent() {
    // SOUNDNESS PIN: `owl:Thing ⊑ owl:Nothing` is INCONSISTENT under the Direct Semantics
    // (domains are non-empty), but the EL classifier does not track ⊤'s satisfiability
    // (its unsatisfiable-class list stays empty here). The ⊤ guard must abstain — a
    // Consistent verdict would be UNSOUND. Starved budget so the sq-pbz04.4.8 fall-through
    // cannot supply the verdict; `el_top_guard_falls_through_to_tableau` covers the
    // graduated case.
    let (verdict, branch) = check_starved("owl:Thing rdfs:subClassOf owl:Nothing .");
    assert_eq!(
        verdict,
        ConsistencyVerdict::Unknown(UnknownReason::ElTopGuard)
    );
    assert_eq!(branch, Branch::ElClassification);
}

#[test]
fn el_consistent_for_top_free_tbox() {
    // Pure ⊤-free EL+⊥ TBox (super-∃ keeps it out of RL): Consistent by the
    // empty-interpretation model construction — the one shape the EL branch certifies.
    let (verdict, branch) = check(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :B rdfs:subClassOf :C .",
    );
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::ElClassification);
}

// -------------------------------------------------------------------------------------------
// QL + ALCH branches, extraction, budget
// -------------------------------------------------------------------------------------------

#[test]
#[cfg(not(feature = "dispatch_ql"))]
fn ql_always_pending() {
    // In QL (super-∃ with a named filler kills RL; the complement kills EL), so the QL
    // branch owns it — and without `dispatch_ql` QL consistency is wholly deferred
    // (engaged opt-in via sparq-reason-ql, sq-fj8lj), never decided here. Starved budget so
    // the sq-pbz04.4.8 fall-through cannot mask the deferral.
    let (verdict, branch) = check_starved(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :C rdfs:subClassOf [ owl:complementOf :D ] .",
    );
    assert_eq!(
        verdict,
        ConsistencyVerdict::Unknown(UnknownReason::QlConsistencyPending)
    );
    assert_eq!(branch, Branch::QlDeferred);
}

// [FABLE-5] sq-fj8lj acceptance: with `dispatch_ql` the QL branch delegates to the
// sparq-reason-ql DL-Lite_R checker (its OWN capture accounting owns the verdict) instead of
// abstaining `QlConsistencyPending`. Every graph below is in-QL but NOT in-RL/EL, so the QL
// branch owns it (the complement kills EL; the super-∃ kills RL).
#[cfg(feature = "dispatch_ql")]
mod dispatch_ql {
    use super::*;

    // The complement + super-∃ shape (the routing witness) is NOT fully captured by the QL
    // crate's DL-Lite_R extraction — the QUALIFIED `someValuesFrom :B` on the RHS is outside
    // DL-Lite_R and lands in `skipped` (the subClassOf-complement half IS captured since #2513,
    // as a negative inclusion) — so with no violation found the branch abstains fail-closed with
    // the QL crate's own gap accounting, never a guessed Consistent.
    // Starved budget so the sq-pbz04.4.8 fall-through cannot mask the QL crate's own gap
    // accounting; `ql_capture_gap_falls_through_to_tableau` covers the graduated case.
    #[test]
    fn ql_capture_gap_abstains_fail_closed() {
        let (verdict, branch) = check_starved(
            ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
             :C rdfs:subClassOf [ owl:complementOf :D ] .",
        );
        assert_eq!(branch, Branch::QlConsistency);
        let ConsistencyVerdict::Unknown(UnknownReason::QlCaptureGap(_)) = verdict else {
            panic!("expected Unknown(QlCaptureGap(_)), got {:?}", verdict);
        };
    }

    // A violated captured disjointness graduates to a DEFINITIVE Inconsistent: sound at any
    // capture level (monotonicity). This was Unknown(QlConsistencyPending) before sq-fj8lj.
    #[test]
    fn ql_disjointness_violation_is_inconsistent() {
        let (verdict, branch) = check(
            ":C rdfs:subClassOf [ owl:complementOf :D ] . \
             :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom owl:Thing ] . \
             :E owl:disjointWith :F . \
             :i rdf:type :E . \
             :i rdf:type :F .",
        );
        assert_eq!(branch, Branch::QlConsistency);
        assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
        // MUTATION WITNESS: drop the second :i typing and the violation disappears. Since #2513
        // the QL crate captures `:C ⊑ ¬:D` (the subClassOf-complement RHS is a DL-Lite_R negative
        // inclusion), so every axiom of this graph is captured and the branch graduates a
        // DEFINITIVE Consistent — the model interpreting only `:E` as `{:i}` satisfies all three
        // axioms. Two graphs one triple apart, decided opposite ways: the Inconsistent above is
        // carried by the violation query, not by the fixture. (Before #2513 the same graph could
        // only reach Unknown(QlCaptureGap), the branch's documented Consistent-side blind spot.)
        let (verdict, branch) = check(
            ":C rdfs:subClassOf [ owl:complementOf :D ] . \
             :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom owl:Thing ] . \
             :E owl:disjointWith :F . \
             :i rdf:type :E .",
        );
        assert_eq!(branch, Branch::QlConsistency);
        assert_eq!(verdict, ConsistencyVerdict::Consistent);
    }
}

// -------------------------------------------------------------------------------------------
// Guard-abstention tableau fall-through ([SONNET-4.6] sq-pbz04.4.8) — the bead's acceptance set.
// Each case: the OWNING profile branch abstains (pinned by its `*_abstains` test above under a
// starved budget), and with a real budget the ALCH tableau — complete for the whole L1
// fragment — supplies a DEFINITIVE verdict attributed to `Branch::AlchTableau`.
// -------------------------------------------------------------------------------------------

#[test]
fn rl_divergence_guard_falls_through_to_tableau() {
    // BEAD ACCEPTANCE CASE. `:A ⊓ :B ⊑ ⊥` with `:x ∈ :A` is plainly CONSISTENT (interpret
    // :B as empty), but the RL branch may not say so — owl:disjointWith is implicated in
    // DOCUMENTED_DIVERGENCES. The tableau decides it. Was Unknown(RlDivergenceGuard).
    let (verdict, branch) = check(":A owl:disjointWith :B . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
    // The complementOf shape likewise: `:A ⊑ ¬:B` with `:x ∈ :A` is consistent.
    let (verdict, branch) = check(":A rdfs:subClassOf [ owl:complementOf :B ] . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
}

#[test]
fn el_top_guard_falls_through_to_tableau() {
    // BEAD ACCEPTANCE CASE, and the one that upgrades an abstention to the verdict the EL
    // branch could NEVER have given: `⊤ ⊑ ⊥` is INCONSISTENT under the Direct Semantics
    // (non-empty domains), the EL branch abstains via the ⊤ guard, and the tableau — which
    // seeds a fresh root exactly for this — says Inconsistent. Was Unknown(ElTopGuard).
    let (verdict, branch) = check("owl:Thing rdfs:subClassOf owl:Nothing .");
    assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
    assert_eq!(branch, Branch::AlchTableau);
}

#[test]
fn el_unapplied_and_skipped_fall_through_to_tableau() {
    // The EL classifier applies no ABox axiom (ElUnappliedAxioms); the tableau does.
    let (verdict, branch) =
        check(":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
    // ...and the ABox case where the answer is INCONSISTENT — the fall-through recovers
    // both directions, not just the easy one. `:A ⊑ ∃r.:B`, `:B ⊑ ⊥`, `:x ∈ :A` has no
    // model: the ∃-rule must build the r-successor and clash it against ⊥.
    let (verdict, branch) = check(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :B rdfs:subClassOf owl:Nothing . :x a :A .",
    );
    assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
    assert_eq!(branch, Branch::AlchTableau);
    // An axiom the EL classifier SKIPS (over its decode depth) is still fully present in
    // the L1 model, so the tableau sees all 300 nesting levels and decides.
    let (verdict, branch) = check(&deep_el_body(300));
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
}

// The QL do-not-duplicate rule (sq-pbz04.3.4), re-examined explicitly: the fall-through
// writes no DL-Lite_R reasoning and makes no QL claim — it re-uses the ALCH tableau already
// argued complete for the fragment. With `dispatch_ql` the QL crate is still asked FIRST and
// still owns every verdict it gives (`ql_disjointness_violation_is_inconsistent` above is
// unchanged); the fall-through fires only where the QL crate itself declined.
#[test]
#[cfg(not(feature = "dispatch_ql"))]
fn ql_pending_falls_through_to_tableau() {
    // `:A ⊑ ∃r.:B` + `:C ⊑ ¬:D` is consistent (everything empty). Was
    // Unknown(QlConsistencyPending) — a deferral of the QL PROCEDURE, never a claim that
    // this crate cannot decide the ontology.
    let (verdict, branch) = check(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :C rdfs:subClassOf [ owl:complementOf :D ] .",
    );
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
}

#[test]
#[cfg(feature = "dispatch_ql")]
fn ql_capture_gap_falls_through_to_tableau() {
    // Same graph, feature ON: the QL crate cannot certify capture of the qualified super-∃
    // and abstains (`ql_capture_gap_abstains_fail_closed`), so the tableau decides.
    let (verdict, branch) = check(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :C rdfs:subClassOf [ owl:complementOf :D ] .",
    );
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
}

#[test]
fn fall_through_never_pre_empts_a_deciding_branch() {
    // Rule 1: a profile branch that DECIDED keeps the verdict AND the attribution — the
    // fall-through must not re-route it to the tableau. Both RL directions, pinned.
    let (verdict, branch) = check(":A rdfs:subClassOf :B . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::RlMaterialization);
    let (verdict, branch) = check(":A owl:disjointWith :B . :x a :A . :x a :B .");
    assert_eq!(verdict, ConsistencyVerdict::Inconsistent);
    assert_eq!(branch, Branch::RlMaterialization);
    // The EL branch's one certifiable shape likewise stays EL-attributed.
    let (verdict, branch) = check(
        ":A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] . \
         :B rdfs:subClassOf :C .",
    );
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::ElClassification);
}

#[test]
fn fall_through_keeps_the_owner_abstention_when_the_tableau_abstains() {
    // Rule 2: the fall-through is strictly abstention-REDUCING. When the tableau also
    // abstains, the owner's outcome comes back verbatim — the guard's diagnosis is never
    // overwritten by a `ResourceBudget` the tableau earned second. (This is what
    // `check_starved` relies on, asserted here directly rather than only implied.)
    let (verdict, branch) = check_starved(":A owl:disjointWith :B . :x a :A .");
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::RlDivergenceGuard(_))
        ),
        "expected the OWNER's RlDivergenceGuard, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::RlMaterialization);
    // Same input, real budget: the SAME graph now gets a verdict. Two runs one budget
    // apart, decided differently — the fall-through is what carries the verdict, not the
    // fixture.
    let (verdict, branch) = check(":A owl:disjointWith :B . :x a :A .");
    assert_eq!(verdict, ConsistencyVerdict::Consistent);
    assert_eq!(branch, Branch::AlchTableau);
}

#[test]
fn pr1_punning_does_not_fall_through() {
    // Rule 3, the SOUNDNESS PIN of this bead: the PR1 punning abstention is a
    // well-formedness refusal, not an incompleteness guard — under punning the L1 shadow is
    // one arbitrary reading of an input that is not an OWL 2 DL ontology, so the tableau's
    // verdict on that shadow is a verdict on a DIFFERENT ontology. It must keep abstaining
    // even with a full budget. A fall-through that forgets this exception turns this into
    // Consistent/AlchTableau; that is the mutation this assertion catches.
    let (verdict, branch) = check(":x a :A . :A a :B .");
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::RlPr1Preconditions(_))
        ),
        "expected RlPr1Preconditions to survive the fall-through, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::RlMaterialization);
}

#[test]
fn alch_branch_consistent_and_inconsistent() {
    // ¬A ⊑ B is in NO profile (complement in sub position) — the tableau owns it and is
    // complete for the fragment, so BOTH definitive verdicts flow through.
    let (v1, b1) = check("[ owl:complementOf :A ] rdfs:subClassOf :B .");
    assert_eq!(v1, ConsistencyVerdict::Consistent);
    assert_eq!(b1, Branch::AlchTableau);
    // A ≡ ¬A has no model (also profile-free: complement as an equivalence operand).
    let (v2, b2) = check(":A owl:equivalentClass [ owl:complementOf :A ] .");
    assert_eq!(v2, ConsistencyVerdict::Inconsistent);
    assert_eq!(b2, Branch::AlchTableau);
}

#[test]
fn alch_budget_exhaustion_abstains() {
    let (mut dict, triples) = parse("[ owl:complementOf :A ] rdfs:subClassOf :B .");
    let starved = DirectChecker::with_budget(Budget {
        max_nodes: 1,
        max_rule_applications: 0,
    });
    let out = starved.consistency(&mut dict, &triples);
    assert!(
        matches!(
            out.verdict,
            ConsistencyVerdict::Unknown(UnknownReason::ResourceBudget(_))
        ),
        "expected ResourceBudget, got {:?}",
        out.verdict
    );
    assert_eq!(out.branch, Branch::AlchTableau);
}

#[test]
fn extraction_failure_fails_closed() {
    // owl:inverseOf is outside the L1 fragment: refused BEFORE any branch runs.
    let (verdict, branch) = check(":p owl:inverseOf :q .");
    assert!(
        matches!(
            verdict,
            ConsistencyVerdict::Unknown(UnknownReason::OutOfFragment(_))
        ),
        "expected OutOfFragment, got {:?}",
        verdict
    );
    assert_eq!(branch, Branch::Extraction);
}

// -------------------------------------------------------------------------------------------
// Entailment by refutation
// -------------------------------------------------------------------------------------------

#[test]
fn entailment_subclass_transitivity() {
    let premise = ":A rdfs:subClassOf :B . :B rdfs:subClassOf :C .";
    let (v, b) = entail(premise, ":A rdfs:subClassOf :C .");
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // The converse is NOT entailed — certified by the complete tableau (a definitive
    // NegativeEntailment-style verdict, sound per the record's complete-branch rule).
    let (v, b) = entail(premise, ":C rdfs:subClassOf :A .");
    assert_eq!(v, EntailmentVerdict::NotEntailed);
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_class_assertion() {
    let premise = ":x a :A . :A rdfs:subClassOf :B .";
    let (v, _) = entail(premise, ":x a :B .");
    assert_eq!(v, EntailmentVerdict::Entailed);
    let (v, _) = entail(premise, ":y a :B .");
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_role_assertion_fresh_class_trick() {
    // BEAD ACCEPTANCE CASE: ObjectPropertyAssertion via the fresh-class encoding
    // {B(b), (∀R.¬B)(a)} — r ⊑ s and r(x,y) entail s(x,y); the ∀ on the SUPER-role must
    // reach the sub-role edge (the L3 hierarchy machinery) to close the refutation.
    let premise = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . \
                   :r rdfs:subPropertyOf :s . :x :r :y .";
    let conclusion = ":s a owl:ObjectProperty . :x :s :y .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // Completeness half: the REVERSED assertion is not entailed, and the fresh-class
    // encoding must certify that (a model interpreting the fresh class as {target} exists).
    let (v, _) = entail(premise, ":s a owl:ObjectProperty . :y :s :x .");
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_equivalence_needs_both_inclusions() {
    let (v, _) = entail(
        ":A rdfs:subClassOf :B . :B rdfs:subClassOf :A .",
        ":A owl:equivalentClass :B .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
    // One inclusion alone does not entail the equivalence: the second refutation
    // component is satisfiable.
    let (v, _) = entail(":A rdfs:subClassOf :B .", ":A owl:equivalentClass :B .");
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_disjointness_domain_range_desugarings() {
    // DisjointClasses via A ⊑ ¬B.
    let (v, _) = entail(
        ":A rdfs:subClassOf [ owl:complementOf :B ] .",
        ":A owl:disjointWith :B .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
    // Domain widens along subclassing (∃r.⊤ ⊑ A ⊑ B).
    let (v, _) = entail(
        ":r a owl:ObjectProperty . :r rdfs:domain :A . :A rdfs:subClassOf :B .",
        ":r a owl:ObjectProperty . :r rdfs:domain :B .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
    // Range widens along subclassing (⊤ ⊑ ∀r.A, A ⊑ B).
    let (v, _) = entail(
        ":r a owl:ObjectProperty . :r rdfs:range :A . :A rdfs:subClassOf :B .",
        ":r a owl:ObjectProperty . :r rdfs:range :B .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
}

// -------------------------------------------------------------------------------------------
// SubObjectPropertyOf conclusions — the fresh-individual-pair encoding ([FABLE-5] sq-pbz04.4.9)
// -------------------------------------------------------------------------------------------

#[test]
fn entailment_subproperty_direct_and_converse() {
    // BEAD ACCEPTANCE CASE (sq-pbz04.4.9): a SubObjectPropertyOf CONCLUSION now gets a
    // definitive verdict via {R(a,b), B(b), (∀S.¬B)(a)} with fresh a/b/B — previously an
    // UnencodedConclusion abstention. r ⊑ s entails r ⊑ s...
    let premise = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . :r rdfs:subPropertyOf :s .";
    let conclusion = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . :r rdfs:subPropertyOf :s .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // ...and the CONVERSE s ⊑ r is definitively NOT entailed (completeness half: the model
    // interpreting B = {b} with an s-edge outside r certifies satisfiability). A mutation
    // that quantifies ∀ over the SUB role instead of the SUPER role makes this refutation
    // unsatisfiable and flips it to a WRONG Entailed — this assertion is the witness.
    let (v, b) = entail(
        premise,
        ":r a owl:ObjectProperty . :s a owl:ObjectProperty . :s rdfs:subPropertyOf :r .",
    );
    assert_eq!(v, EntailmentVerdict::NotEntailed);
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_subproperty_transitivity() {
    // Role-hierarchy transitivity flows through the tableau's reflexive-transitive
    // `is_subrole` closure: r ⊑ s ⊑ t entails r ⊑ t.
    let premise = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . :t a owl:ObjectProperty . \
                   :r rdfs:subPropertyOf :s . :s rdfs:subPropertyOf :t .";
    let (v, b) = entail(
        premise,
        ":r a owl:ObjectProperty . :t a owl:ObjectProperty . :r rdfs:subPropertyOf :t .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // t ⊑ r is not entailed by the chain.
    let (v, _) = entail(
        premise,
        ":r a owl:ObjectProperty . :t a owl:ObjectProperty . :t rdfs:subPropertyOf :r .",
    );
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_subproperty_reflexivity_from_empty_premise() {
    // R ⊑ R holds in EVERY interpretation, so even an (axiom-)empty premise entails it —
    // the refutation {r(a,b), B(b), (∀r.¬B)(a)} clashes on the r-edge itself (the
    // reflexive base of `is_subrole`). A mutation that drops B(b) or the role assertion
    // from the encoding leaves the refutation satisfiable and flips this to NotEntailed.
    let (v, b) = entail(
        ":x a :A .",
        ":r a owl:ObjectProperty . :r rdfs:subPropertyOf :r .",
    );
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // Two UNRELATED roles from the same premise: definitively not entailed (freshness: the
    // fresh B/a/b must not capture anything in the premise).
    let (v, _) = entail(
        ":x a :A .",
        ":r a owl:ObjectProperty . :s a owl:ObjectProperty . :r rdfs:subPropertyOf :s .",
    );
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_subproperty_mixed_conclusion_aggregates() {
    // A conclusion mixing the NEW RBox encoding with an ABox one: r ⊑ s AND s(x,y) both
    // follow from {r ⊑ s, r(x,y)} — the per-axiom conjunction aggregates to Entailed.
    let premise = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . \
                   :r rdfs:subPropertyOf :s . :x :r :y .";
    let conclusion = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . \
                      :r rdfs:subPropertyOf :s . :x :s :y .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
    // Flip ONE conjunct (t ⊑ r not entailed): the whole conclusion set fails definitively.
    let conclusion = ":r a owl:ObjectProperty . :s a owl:ObjectProperty . \
                      :t a owl:ObjectProperty . :t rdfs:subPropertyOf :r . :x :s :y .";
    let (v, _) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_out_of_fragment_inputs_fail_closed() {
    // Premise outside the fragment.
    let (v, b) = entail(":p owl:inverseOf :q .", ":A rdfs:subClassOf :B .");
    assert!(
        matches!(v, EntailmentVerdict::Unknown(UnknownReason::OutOfFragment(_))),
        "premise: expected OutOfFragment, got {:?}",
        v
    );
    assert_eq!(b, Branch::Extraction);
    // Conclusion outside the fragment.
    let (v, b) = entail(":A rdfs:subClassOf :B .", ":p owl:inverseOf :q .");
    assert!(
        matches!(v, EntailmentVerdict::Unknown(UnknownReason::OutOfFragment(_))),
        "conclusion: expected OutOfFragment, got {:?}",
        v
    );
    assert_eq!(b, Branch::Extraction);
}

#[test]
fn entailment_empty_conclusion_is_trivially_entailed() {
    let (v, b) = entail(":A rdfs:subClassOf :B .", "");
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
}

// -------------------------------------------------------------------------------------------
// Opt-in transitive roles ([GPT-5.6] sq-zfwzq, features `dispatch` + `dl_transitive`)
// -------------------------------------------------------------------------------------------

#[cfg(feature = "dl_transitive")]
mod transitive {
    use super::*;

    /// Dispatch: a transitive ontology routes to the ALCH+S tableau — the only branch whose
    /// soundness/completeness argument covers transitivity — even though its axioms are
    /// syntactically in-RL/in-EL, and gets a DEFINITIVE verdict there.
    #[test]
    fn transitive_ontology_routes_to_tableau_with_definitive_verdict() {
        let (mut d, t) = parse(
            ":r a owl:TransitiveProperty .\n\
             :a :r :b . :b :r :c .",
        );
        let out = DirectChecker::new().consistency(&mut d, &t);
        assert_eq!(out.branch, Branch::AlchTableau);
        assert_eq!(out.verdict, ConsistencyVerdict::Consistent);
    }

    /// LOAD-BEARING entailment (hand-derived): `{Trans(r), r(a,b), r(b,c)} ⊨ r(a,c)` — the
    /// classic transitive-chain composition, decided through the fresh-class refutation on
    /// the ∀₊-extended tableau. The control (same premise minus `Trans(r)`) is definitively
    /// NotEntailed: the pair is the mutation witness (knocking out the ∀₊-propagation, or
    /// the transitivity extraction, flips the Entailed verdict).
    #[test]
    fn transitive_chain_role_assertion_entailed_and_control_not_entailed() {
        let premise_trans = ":r a owl:TransitiveProperty .\n:a :r :b . :b :r :c .";
        let premise_plain = ":r a owl:ObjectProperty .\n:a :r :b . :b :r :c .";
        // Deliberately omit the semantically-inert declaration from the conclusion, as the
        // W3C DIRECT transitive-chain cases do. Entailment reuses only the premise-confirmed
        // role kind for fail-closed conclusion extraction.
        let conclusion = ":a :r :c .";

        let (mut d, prem, concl) = parse_two(premise_trans, conclusion);
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl);
        assert_eq!(out.verdict, EntailmentVerdict::Entailed);
        assert_eq!(out.branch, Branch::AlchTableau);

        let (mut d, prem, concl) = parse_two(
            premise_plain,
            ":r a owl:ObjectProperty .\n:a :r :c .",
        );
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl);
        assert_eq!(
            out.verdict,
            EntailmentVerdict::NotEntailed,
            "without Trans(r) the composed edge is not entailed — the load-bearing control"
        );
    }

    /// A TRANSITIVITY conclusion is decided by the two-step-chain refutation encoding
    /// (`O ⊨ Trans(r)` iff `O ∪ {r(a,b), r(b,c), B(c), (∀r.¬B)(a)}` is unsatisfiable —
    /// check.rs module docs, sq-zfwzq): a declared `Trans(r)` premise entails it, a plain
    /// object property does NOT (definitive NotEntailed — the mutation witness for the
    /// encoding: dropping the ∀₊-rule or the encoding itself flips the Entailed side).
    #[test]
    fn transitivity_conclusion_entailed_and_control_not_entailed() {
        let concl = ":r a owl:TransitiveProperty .";
        let (mut d, prem, concl_t) = parse_two(":r a owl:TransitiveProperty .", concl);
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl_t);
        assert_eq!(out.verdict, EntailmentVerdict::Entailed);
        assert_eq!(out.branch, Branch::AlchTableau);

        let (mut d, prem, concl_p) = parse_two(":r a owl:ObjectProperty .", concl);
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl_p);
        assert_eq!(
            out.verdict,
            EntailmentVerdict::NotEntailed,
            "a plain object property is not entailed transitive — the load-bearing control"
        );
    }

    /// The transitivity-conclusion encoding is SEMANTIC, not a syntactic premise match:
    /// `{⊤ ⊑ ∀r.⊥}` forces `r` empty in every model, so `r` is (vacuously) transitive —
    /// Entailed even though the premise never mentions `owl:TransitiveProperty`.
    #[test]
    fn transitivity_conclusion_entailed_semantically_for_empty_role() {
        let (mut d, prem, concl) = parse_two(
            ":r a owl:ObjectProperty .\n\
             owl:Thing rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; \
             owl:allValuesFrom owl:Nothing ] .",
            ":r a owl:TransitiveProperty .",
        );
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl);
        assert_eq!(out.verdict, EntailmentVerdict::Entailed);
    }

    /// Transitive SUBROLE through the hierarchy, end-to-end: `{Trans(s), s ⊑ r, s(a,b),
    /// s(b,c)} ⊨ r(a,c)` (the composed s-edge is also an r-edge via the hierarchy).
    #[test]
    fn transitive_subrole_composition_entailed_through_hierarchy() {
        let (mut d, prem, concl) = parse_two(
            ":s a owl:TransitiveProperty .\n:s rdfs:subPropertyOf :r .\n\
             :r a owl:ObjectProperty .\n:a :s :b . :b :s :c .",
            ":r a owl:ObjectProperty .\n:a :r :c .",
        );
        let out = DirectChecker::new().entailment(&mut d, &prem, &concl);
        assert_eq!(out.verdict, EntailmentVerdict::Entailed);
    }
}

// -------------------------------------------------------------------------------------------
// Conclusion anonymous individuals — existential reading + rolling-up ([OPUS-4.8] sq-pbz04.4.13)
// -------------------------------------------------------------------------------------------

#[test]
fn entailment_conclusion_bnode_somevaluesfrom2bnode() {
    // Faithful reconstruction of the W3C DIRECT PositiveEntailmentTest `somevaluesfrom2bnode`
    // ("Shows that a BNode is an existential variable"): the premise types `a` into ∃p.⊤, the
    // conclusion asserts `a p _:x`. Under the official existential reading this IS entailed;
    // rolling `a p _:x` into `a : ∃p.owl:Thing` lets the complete tableau certify it. Before
    // the fix, `_:x` was read as a skolem CONSTANT and the refutation stayed satisfiable → a
    // WRONG NotEntailed (the pinned M2 divergence).
    let premise = ":p a owl:ObjectProperty . \
                   :a rdf:type [ owl:onProperty :p ; owl:someValuesFrom owl:Thing ] .";
    let conclusion = ":p a owl:ObjectProperty . :a :p _:x .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed, "somevaluesfrom2bnode must be Entailed");
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_conclusion_bnode_webont_somevaluesfrom_003() {
    // Faithful reconstruction of W3C DIRECT PositiveEntailmentTest `WebOnt-someValuesFrom-003`
    // ("A simple infinite loop for implementors to avoid"): premise `person ≡ ∃parent.person`
    // with `fred : person`; conclusion an anonymous parent-CHAIN `fred parent _:b1 parent _:b2`
    // (each an owl:Thing). The chain rolls into `fred : ∃parent.(⊤ ⊓ ∃parent.⊤)`, decided by
    // the tableau (termination via blocking) — genuinely Entailed, not the M2 skolem NotEntailed.
    let premise = ":parent a owl:ObjectProperty . \
                   :person owl:equivalentClass [ owl:onProperty :parent ; owl:someValuesFrom :person ] . \
                   :fred a :person .";
    let conclusion = ":parent a owl:ObjectProperty . \
                      :fred a owl:Thing . \
                      :fred :parent _:b1 . _:b1 a owl:Thing . \
                      _:b1 :parent _:b2 . _:b2 a owl:Thing .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed, "WebOnt-someValuesFrom-003 must be Entailed");
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_conclusion_bnode_rollable_notentailed_is_sound() {
    // A rollable conclusion bnode whose existential is genuinely NOT entailed: premise only
    // says `a : A` (a need not have any p-successor), conclusion asserts `a p _:x`. Rolling to
    // `a : ∃p.⊤` and refuting on the COMPLETE tableau yields a SOUND NotEntailed — rolling is
    // sound in BOTH directions, so a definitive negative verdict here is legitimate (it is NOT
    // the unsound skolem-constant NotEntailed, which the non-rollable shapes below abstain on).
    let premise = ":p a owl:ObjectProperty . :a a :A .";
    let conclusion = ":p a owl:ObjectProperty . :a :p _:x .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::NotEntailed);
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_conclusion_bnode_shared_abstains_never_notentailed() {
    // NEGATIVE PROBE: `_:x` is SHARED between two property assertions (`a p _:x` and `b p _:x`)
    // — a non-tree shape. The rolling-up is not applicable, so the checker must ABSTAIN
    // fail-closed, NEVER emit a skolem-constant NotEntailed.
    let premise = ":p a owl:ObjectProperty . :a a :A . :b a :A .";
    let conclusion = ":p a owl:ObjectProperty . :a :p _:x . :b :p _:x .";
    let (v, b) = entail(premise, conclusion);
    assert!(
        matches!(
            v,
            EntailmentVerdict::Unknown(UnknownReason::ConclusionAnonymousIndividual(_))
        ),
        "shared conclusion bnode must abstain (ConclusionAnonymousIndividual), got {:?}",
        v
    );
    assert!(!v.is_not_entailed(), "must NOT be a skolem NotEntailed");
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_conclusion_bnode_cyclic_abstains_never_notentailed() {
    // NEGATIVE PROBE: a cyclic anonymous shape `_:x p _:y . _:y p _:x .` with no named anchor —
    // not tree-shaped, so abstain fail-closed, never a skolem NotEntailed.
    let premise = ":p a owl:ObjectProperty . :a a :A .";
    let conclusion = ":p a owl:ObjectProperty . _:x :p _:y . _:y :p _:x .";
    let (v, b) = entail(premise, conclusion);
    assert!(
        matches!(
            v,
            EntailmentVerdict::Unknown(UnknownReason::ConclusionAnonymousIndividual(_))
        ),
        "cyclic conclusion bnode must abstain (ConclusionAnonymousIndividual), got {:?}",
        v
    );
    assert!(!v.is_not_entailed());
    assert_eq!(b, Branch::AlchTableau);
}

#[test]
fn entailment_conclusion_bnode_named_successor_abstains() {
    // NEGATIVE PROBE: an anonymous individual with a NAMED successor (`a p _:x . _:x q :c .`)
    // would roll up only through a nominal `{c}`, which is out of the ALCH fragment — abstain.
    let premise = ":p a owl:ObjectProperty . :q a owl:ObjectProperty . :a a :A .";
    let conclusion = ":p a owl:ObjectProperty . :q a owl:ObjectProperty . \
                      :a :p _:x . _:x :q :c .";
    let (v, _) = entail(premise, conclusion);
    assert!(
        matches!(
            v,
            EntailmentVerdict::Unknown(UnknownReason::ConclusionAnonymousIndividual(_))
        ),
        "named-successor conclusion bnode must abstain, got {:?}",
        v
    );
    assert!(!v.is_not_entailed());
}

#[test]
fn entailment_premise_side_bnode_unaffected_skolemisation_stays() {
    // Premise-side blank nodes are UNAFFECTED — skolemisation is entailment-preserving on the
    // premise. The premise `a p _:pb`, `_:pb : C`, `C ⊑ D` (a has an anonymous p-successor that
    // is a C, hence a D) entails the bnode-FREE conclusion `a : ∃p.D` — a definitive verdict,
    // proving the abstention is CONCLUSION-triggered only and premise bnodes still reason.
    let premise = ":p a owl:ObjectProperty . :a :p _:pb . _:pb a :C . :C rdfs:subClassOf :D .";
    let conclusion = ":a rdf:type [ owl:onProperty :p ; owl:someValuesFrom :D ] .";
    let (v, b) = entail(premise, conclusion);
    assert_eq!(v, EntailmentVerdict::Entailed);
    assert_eq!(b, Branch::AlchTableau);
}

// -------------------------------------------------------------------------------------------
// Refutation budget fallback ([OPUS-5] sq-pbz04.4.10)
//
// The tableau owns every refutation, but its budget is a deterministic COUNT — a big in-RL
// premise can exhaust it where the RL materializer decides in one pass. These tests pin BOTH
// halves of the contract: the fallback RECOVERS a verdict the tableau abandoned, and it stays
// fail-closed (keeping the honest `ResourceBudget` reason) whenever no profile branch can own
// the question or a branch guard abstains. `STARVED` is a budget small enough that the tableau
// cannot finish ANY of these refutations — the point is to reach the fallback deterministically,
// not to model a realistic budget.
// -------------------------------------------------------------------------------------------

/// A budget so small every refutation below exhausts it (the fallback's trigger condition).
const STARVED: Budget = Budget {
    max_nodes: 1,
    max_rule_applications: 1,
};

fn entail_starved(premise: &str, conclusion: &str) -> (EntailmentVerdict, Branch) {
    let (mut dict, prem, concl) = parse_two(premise, conclusion);
    let out = DirectChecker::with_budget(STARVED).entailment(&mut dict, &prem, &concl);
    (out.verdict, out.branch)
}

#[test]
fn entailment_budget_fallback_rl_recovers_entailed() {
    // `A ⊑ B, a : A ⊨ a : B`. The refutation is the premise plus `(¬B)(a)`, which is IN RL
    // (ClassAssertion of a superClassExpression `¬B`). Starved, the tableau abstains; the RL
    // materializer derives `a : B` by cax-sco and `inconsistencies()` sees the cls-com clash
    // against `a : ¬B`, so the refutation is UNSATISFIABLE ⇒ the axiom IS entailed. The verdict
    // is attributed to the branch that broke the tie, not to the tableau that gave up.
    let premise = ":A rdfs:subClassOf :B . :a rdf:type :A .";
    let conclusion = ":a rdf:type :B .";
    let (v, b) = entail_starved(premise, conclusion);
    assert_eq!(
        v,
        EntailmentVerdict::Entailed,
        "the RL fallback must recover the verdict the starved tableau abandoned"
    );
    assert_eq!(b, Branch::RlMaterialization, "traceability: the fallback branch owns it");
    // Control: with the DEFAULT budget the tableau decides it itself and nothing is attributed
    // to a profile branch — the fallback is reached only through budget exhaustion.
    let (v_default, b_default) = entail(premise, conclusion);
    assert_eq!(v_default, EntailmentVerdict::Entailed);
    assert_eq!(b_default, Branch::AlchTableau);
}

#[test]
fn entailment_budget_fallback_rl_recovers_not_entailed() {
    // `A ⊑ C ⊭ A disjointWith B`. The DisjointClasses refutation adds `(A ⊓ B)(x)` — the ONLY
    // encoding that introduces no `owl:complementOf`, so the augmented model clears the RL
    // divergence guard and the branch's `Consistent` is sound. Refutation SATISFIABLE ⇒ NOT
    // entailed, recovered from a starved tableau.
    let premise = ":A rdfs:subClassOf :C .";
    let conclusion = ":A owl:disjointWith :B .";
    let (v, b) = entail_starved(premise, conclusion);
    assert_eq!(
        v,
        EntailmentVerdict::NotEntailed,
        "the RL fallback must recover the negative verdict too"
    );
    assert_eq!(b, Branch::RlMaterialization);
    // Same answer as the complete tableau at the default budget — the fallback agrees with the
    // branch it is standing in for, it does not invent a different verdict.
    assert_eq!(entail(premise, conclusion).0, EntailmentVerdict::NotEntailed);
}

#[test]
fn entailment_budget_fallback_keeps_resource_budget_when_out_of_profile() {
    // NEGATIVE PROBE: the premise puts `owl:complementOf` in a SUBCLASS position, which is in
    // neither RL nor EL (nor QL), so no profile branch may own the refutation. The fallback
    // must decline and the checker must keep the tableau's honest `ResourceBudget` abstention
    // — a starved tableau is never rescued by a guess.
    let premise = "[ owl:complementOf :B ] rdfs:subClassOf :C . :a rdf:type :A .";
    let conclusion = ":a rdf:type :B .";
    let (v, b) = entail_starved(premise, conclusion);
    assert!(
        matches!(v, EntailmentVerdict::Unknown(UnknownReason::ResourceBudget(_))),
        "out-of-profile refutation must keep the ResourceBudget abstention, got {:?}",
        v
    );
    assert_eq!(b, Branch::AlchTableau, "the abstention is still the tableau's");
}

#[test]
fn entailment_budget_fallback_respects_the_rl_divergence_guard() {
    // NEGATIVE PROBE: the `ClassAssertion` refutation introduces `¬B`, so when RL finds NO
    // clash the divergence guard (owl:complementOf, DisjointClasses-001/-003 /
    // New-Feature-ObjectQCR-002) refuses to read "no clash" as consistent. The fallback must
    // therefore NOT produce a `NotEntailed` here — it keeps the `ResourceBudget` abstention.
    // This is the guard discipline of the consistency branches applying unchanged.
    let premise = ":A rdfs:subClassOf :C . :a rdf:type :A .";
    let conclusion = ":a rdf:type :B .";
    let (v, b) = entail_starved(premise, conclusion);
    assert!(
        matches!(v, EntailmentVerdict::Unknown(UnknownReason::ResourceBudget(_))),
        "the RL divergence guard must block a `Consistent`-derived NotEntailed, got {:?}",
        v
    );
    assert!(!v.is_not_entailed());
    assert_eq!(b, Branch::AlchTableau);
    // The COMPLETE tableau does decide it (negatively) at the default budget — proof that the
    // abstention above is the guard being conservative, not the answer being unknowable.
    assert_eq!(entail(premise, conclusion).0, EntailmentVerdict::NotEntailed);
}

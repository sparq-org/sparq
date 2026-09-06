//! [FABLE-5] sq-p6yb7 — DL-Lite_R consistency-check ORACLE tests: hand-built DL-Lite_R KBs in
//! readable Turtle, each with a HAND-DERIVED expected verdict (the differential oracle the bead's
//! acceptance test names). The soundness/completeness argument lives in `src/consistency.rs`;
//! these cases pin its behaviour on both sides of every boundary:
//!
//! * a violated negative inclusion (direct, subclass-derived, chain-derived, role, and
//!   anonymous-witness) → `Inconsistent` — and from an inconsistent KB EVERYTHING is entailed,
//!   so the positive UCQ rewriting's answers must NOT be read as the certain answers;
//! * a satisfiable KB with negative inclusions → definitive `Consistent`;
//! * any uncaptured axiom (non-QL construct, or `owl:complementOf`) with no found violation →
//!   fail-closed `Unknown`, never a guessed `Consistent`.
//!
//! MUTATION WITNESS (verified during development, sq-p6yb7): knocking out the violation-query
//! sweep in `check_consistency_with` (returning straight to the capture gates) flips every
//! `Inconsistent` case here and in the unit suite red (7 failures) — these tests are non-vacuous.

#![cfg(feature = "ql-consistency")]

use oxrdf::Triple;
use sparq_reason_ql::{
    check_consistency, Basic, NegativeInclusion, QlConsistency, QlConsistencyGap,
};

const PRE: &str = r#"
@prefix : <http://ex/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
"#;

fn ttl(src: &str) -> Vec<Triple> {
    oxttl::TurtleParser::new()
        .for_slice(format!("{PRE}{src}").as_bytes())
        .map(|r| r.expect("turtle parse"))
        .collect()
}

/// Oracle 1 — hand-derivation: `:Bird ⊑ ¬:Fish`, `:nemo ∈ :Bird ∩ :Fish` violates it directly.
/// Expected: INCONSISTENT.
#[test]
fn oracle_direct_violation() {
    let kb = ttl(":Bird owl:disjointWith :Fish . :nemo a :Bird , :Fish .");
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
}

/// Oracle 2 — hand-derivation: `:Penguin ⊑ :Bird`, `:Bird ⊑ ¬:Fish`, `:pingu ∈ :Penguin ∩
/// :Fish`. cln(T) derives `:Penguin ⊑ ¬:Fish`; the violation query's PerfectRef rewriting must
/// find it (the raw violation query over the ABox alone would NOT: `:pingu` is never asserted a
/// `:Bird`). Expected: INCONSISTENT, witnessing the ASSERTED axiom `:Bird ⊑ ¬:Fish`.
#[test]
fn oracle_violation_derived_through_subclass() {
    let kb = ttl(
        ":Penguin rdfs:subClassOf :Bird . :Bird owl:disjointWith :Fish . \
         :pingu a :Penguin , :Fish .",
    );
    let QlConsistency::Inconsistent(v) = check_consistency(&kb) else {
        panic!("expected Inconsistent");
    };
    assert_eq!(
        v.axiom,
        NegativeInclusion::Concept(
            Basic::Class("http://ex/Bird".into()),
            Basic::Class("http://ex/Fish".into())
        )
    );
}

/// Oracle 3 — hand-derivation over a longer positive chain + a role atom: `:emp1 :worksFor
/// :acme` puts `:emp1` in `∃:worksFor ⊑ :Employee ⊑ :Person`, and `:Person ⊑ ¬:Company` with
/// `:emp1 ∈ :Company` violates. Expected: INCONSISTENT.
#[test]
fn oracle_violation_through_domain_and_subclass_chain() {
    let kb = ttl(
        ":worksFor rdfs:domain :Employee . :Employee rdfs:subClassOf :Person . \
         :Person owl:disjointWith :Company . \
         :emp1 :worksFor :acme . :emp1 a :Company .",
    );
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
}

/// Oracle 4 — hand-derivation, ANONYMOUS witness: `:Employee ⊑ ∃:worksFor` (unqualified
/// restriction) plus TWO range axioms — every `:worksFor`-successor is in BOTH `:Org` and
/// `:Team`, which are disjoint — so ANY `:Employee` forces an impossible successor. The
/// violating individual exists only in the canonical model (no successor is asserted).
/// Expected: INCONSISTENT.
#[test]
fn oracle_violation_on_anonymous_canonical_witness() {
    let kb = ttl(
        ":Employee rdfs:subClassOf [ owl:onProperty :worksFor ; owl:someValuesFrom owl:Thing ] . \
         :worksFor rdfs:range :Org . :worksFor rdfs:range :Team . \
         :Org owl:disjointWith :Team . \
         :ada a :Employee .",
    );
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
}

/// Oracle 5 — hand-derivation, role disjointness through a subproperty: `:mentors ⊑ :knows`,
/// `:knows ⊑ ¬:ignores`, and the SAME pair `(:a, :b)` in `:mentors` and `:ignores`.
/// Expected: INCONSISTENT. Swapping the `:ignores` edge to `(:b, :a)` breaks the violation
/// (role disjointness is over PAIRS) — expected: CONSISTENT.
#[test]
fn oracle_role_disjointness_pairs() {
    let kb = ttl(
        ":mentors rdfs:subPropertyOf :knows . :knows owl:propertyDisjointWith :ignores . \
         :a :mentors :b . :a :ignores :b .",
    );
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
    let kb = ttl(
        ":mentors rdfs:subPropertyOf :knows . :knows owl:propertyDisjointWith :ignores . \
         :a :mentors :b . :b :ignores :a .",
    );
    assert_eq!(check_consistency(&kb), QlConsistency::Consistent);
}

/// Oracle 6 — hand-derivation: same TBox as oracle 2 but a SATISFIABLE ABox (the model placing
/// `:pingu` in `{Penguin, Bird}` and `:dory` in `{Fish}` satisfies every axiom).
/// Expected: definitive CONSISTENT (the negative inclusions are captured and unviolated).
#[test]
fn oracle_satisfiable_kb_is_definitively_consistent() {
    let kb = ttl(
        ":Penguin rdfs:subClassOf :Bird . :Bird owl:disjointWith :Fish . \
         :pingu a :Penguin . :dory a :Fish .",
    );
    assert_eq!(check_consistency(&kb), QlConsistency::Consistent);
}

/// Oracle 6b — hand-derivation, sq-fj8lj follow-up: the SUBCLASS-COMPLEMENT spelling of a
/// negative inclusion, `:Bird rdfs:subClassOf [ owl:complementOf :Fish ]` (QL's
/// `superClassExpression ::= ObjectComplementOf(subClassExpression)`), is the axiom
/// `:Bird ⊑ ¬:Fish` — the same DL-Lite_R NI `owl:disjointWith` spells, so cln(T) derives
/// `:Penguin ⊑ ¬:Fish` exactly as in oracle 2. Expected: INCONSISTENT on the violating ABox,
/// definitive CONSISTENT on the satisfiable one (before the capture broadened, the anonymous
/// `¬:Fish` superclass counted as `skipped`, so BOTH sides could only be Unknown).
#[test]
fn oracle_subclass_complement_negative_inclusion() {
    let tbox = ":Penguin rdfs:subClassOf :Bird . \
                :Bird rdfs:subClassOf [ owl:complementOf :Fish ] . ";
    let kb = ttl(&format!("{tbox} :pingu a :Penguin , :Fish ."));
    let QlConsistency::Inconsistent(v) = check_consistency(&kb) else {
        panic!("expected Inconsistent, got {:?}", check_consistency(&kb));
    };
    assert_eq!(
        v.axiom,
        NegativeInclusion::Concept(
            Basic::Class("http://ex/Bird".into()),
            Basic::Class("http://ex/Fish".into())
        ),
        "the witness is the ASSERTED axiom, not the cln(T)-derived one"
    );
    let kb = ttl(&format!("{tbox} :pingu a :Penguin . :dory a :Fish ."));
    assert_eq!(check_consistency(&kb), QlConsistency::Consistent);
}

/// Oracle 7 — fail-closed: an UNCAPTURED axiom keeps the verdict Unknown. (a) a non-QL
/// construct (`owl:FunctionalProperty`) blocks `fully_captured()`; (b) a NAMED-subject
/// `owl:complementOf` is consistency-relevant but never structurally captured (`A ≡ ¬B` is
/// stronger than `A ⊑ ¬B` — e.g. `¬B ⊑ A` can make a KB inconsistent with no captured violation
/// query to see it). Expected: Unknown in both — NEVER silently Consistent.
#[test]
fn oracle_uncaptured_axioms_stay_unknown() {
    let kb = ttl(":p a owl:FunctionalProperty . :x :p :y . :x :p :z .");
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Unknown(QlConsistencyGap::NotFullyCaptured { .. })
    ));
    let kb = ttl(":A owl:complementOf :B . :i a :A .");
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Unknown(QlConsistencyGap::UncapturedNegativeAxioms { uncaptured: 1 })
    ));
}

/// Oracle 8 — Inconsistent BEATS Unknown (monotonicity): a KB carrying BOTH an uncaptured
/// axiom AND a violated captured negative inclusion is definitively INCONSISTENT (an
/// inconsistency derived from a subset of the axioms stands for the whole KB).
#[test]
fn oracle_inconsistent_verdict_survives_partial_capture() {
    let kb = ttl(":p a owl:FunctionalProperty . \
         :Bird owl:disjointWith :Fish . :nemo a :Bird , :Fish .");
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
}

/// Oracle 9 — entailment by inconsistency (the graduation the bead names): from the
/// inconsistent oracle-2 KB EVERYTHING is entailed — including a membership the positive UCQ
/// rewriting alone would NEVER answer (`:unrelated ∈ :Whale` has no support in any asserted or
/// derived atom). The definitive `Inconsistent` verdict is exactly what licenses a consumer to
/// graduate such an entailment case instead of abstaining; the positive rewriting's answers
/// under-approximate and must not be used. (What this crate ships is the verdict; the
/// entailed-everything reading is the standard consequence pinned here as documentation.)
#[test]
fn oracle_inconsistency_licenses_entailment_by_inconsistency() {
    let kb = ttl(
        ":Penguin rdfs:subClassOf :Bird . :Bird owl:disjointWith :Fish . \
         :pingu a :Penguin , :Fish . :unrelated a :Thing .",
    );
    // The license: a definitive Inconsistent — not Unknown, not Consistent.
    assert!(matches!(
        check_consistency(&kb),
        QlConsistency::Inconsistent(_)
    ));
}

// sq-pbz04.2.1: CR6 (safe nominals) correctness fixtures — `owl:oneOf` singletons
// and object-valued `owl:hasValue`.
//
// ORACLE NOTE: like tests/differential.rs, the oracle is hand verification against the
// Baader–Brandt–Lutz EL++ calculus AND against the direct model-theoretic semantics: every
// POSITIVE expectation carries the entailment argument, and every NEGATIVE expectation (the
// soundness half — the load-bearing half for this rule) carries a COUNTERMODEL in which all
// axioms hold but the non-derived subsumption fails. The exact-closure assertions mean a
// spurious derivation (unsoundness) fails the test just as loudly as a missing one.
//
// The recurring countermodel shape: `C ⊑ {a}` does NOT make C non-empty — C may be ∅. CR6's
// reachability side-condition exists precisely because merging on a shared nominal alone
// would be UNSOUND; `cr6_soundness_no_merge_without_reachability` pins that.

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{classify_graph, Classifier};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

/// Exact-closure oracle (same discipline as tests/differential.rs): every pair in `expected`
/// must be derived and NO other proper named subsumption among `classes` may be — so an
/// unsound CR6 merge fails this just as loudly as a missing entailment.
fn assert_closure(ttl: &str, classes: &[&str], expected: &[(&str, &str)]) {
    let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    let exp: std::collections::HashSet<(&str, &str)> = expected.iter().copied().collect();
    for &sub in classes {
        for &sup in classes {
            if sub == sup {
                continue;
            }
            let got = h.is_subclass_of(iri(&dict, sub), iri(&dict, sup));
            let want = exp.contains(&(sub, sup));
            assert_eq!(
                got, want,
                "subsumption {sub} ⊑ {sup}: got {got}, want {want}"
            );
        }
    }
}

#[test]
fn cr6_merges_coreferent_classes_linked_by_an_existential() {
    // A ⊑ {a},  B ⊑ {a},  A ⊑ ∃r.B.
    //
    // ⊨ A ⊑ B: if A ≠ ∅ then A = {a} and A's element has an r-successor in B, so B ≠ ∅,
    // so B = {a} = A; if A = ∅ the inclusion is vacuous. This is CR6 with Y = B reached
    // from X = A over the R(r) link — the classic safe-nominal derivation.
    //
    // ⊭ B ⊑ A (countermodel: B = {a}, A = ∅ — every axiom holds, a ∉ A). The exact-closure
    // oracle pins that the merge fires in ONE direction only.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] ."
    );
    assert_closure(&ttl, &["A", "B"], &[("A", "B")]);
}

#[test]
fn cr6_soundness_no_merge_without_reachability() {
    // A ⊑ {a},  B ⊑ {a} — and NOTHING else. The UNSOUND naive rule ("shared nominal ⇒
    // merge") would derive A ⊑ B and B ⊑ A; neither holds:
    //   countermodel for A ⊑ B: A = {a}, B = ∅;  symmetric for B ⊑ A.
    // This is THE negative test pinning CR6's reachability side-condition.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] ."
    );
    assert_closure(&ttl, &["A", "B"], &[]);
}

#[test]
fn cr6_soundness_distinct_nominals_never_merge() {
    // A ⊑ {a},  B ⊑ {b},  A ⊑ ∃r.B — reachable, but the nominals DIFFER.
    // ⊭ A ⊑ B (countermodel: a ≠ b, A = {a}, B = {b}, r = {(a, b)}). CR6 must key strictly
    // on a SHARED nominal.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :b ) ] .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] ."
    );
    assert_closure(&ttl, &["A", "B"], &[]);
}

#[test]
fn cr6_nominal_rooted_membership_flows_to_coreferent_class() {
    // B ⊑ {a},  {a} ⊑ P ("a is a P", said of the nominal class).
    //
    // ⊨ B ⊑ P: any x ∈ B equals a, and a ∈ P. This is CR6 with Y = {a} itself: a nominal is
    // ALWAYS non-empty, so it qualifies as a merge source with no R-path needed
    // (the nominal-rooted branch of ⇝_R).
    //
    // ⊭ P ⊑ B (countermodel: P = {a, p'}, B = ∅... in fact B = ∅ alone suffices).
    let ttl = format!(
        "{PRE}
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         [ owl:oneOf ( :a ) ] rdfs:subClassOf :P ."
    );
    assert_closure(&ttl, &["B", "P"], &[("B", "P")]);
}

#[test]
fn cr6_reachability_spans_multiple_links() {
    // A ⊑ {a},  B ⊑ {a},  A ⊑ ∃r.M,  M ⊑ ∃s.B — B is reached from A over TWO links (and
    // across DIFFERENT roles: ⇝_R is role-erased reachability).
    // ⊨ A ⊑ B: A ≠ ∅ ⇒ M ≠ ∅ ⇒ B ≠ ∅ ⇒ B = {a} = A.
    // ⊭ A ⊑ M, ⊭ M ⊑ B, … (M carries no nominal; countermodels with M disjoint from {a}).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :M ] .
         :M rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :B ] ."
    );
    assert_closure(&ttl, &["A", "B", "M"], &[("A", "B")]);
}

#[test]
fn bottom_via_nominals_unsatisfiability_is_asymmetric() {
    // A ⊑ {a},  B ⊑ {a},  A ⊑ ∃r.B,  A disjointWith B.
    //
    // ⊨ A ⊑ ⊥: if A ≠ ∅ then (as in the merge fixture) A = B = {a}, contradicting
    // A ⊓ B ⊑ ⊥ — so A must be empty. The pipeline: CR6 merges B into S(A), CR2 fires the
    // disjointness conjunction, ⊥ lands in S(A).
    //
    // ⊭ B ⊑ ⊥ (countermodel: B = {a}, A = ∅ satisfies every axiom with B non-empty). The
    // asymmetry is the soundness half: a symmetric/unsound merge would wrongly kill B too.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :A owl:disjointWith :B ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    let unsat = h.unsatisfiable_classes();
    assert!(
        unsat.contains(&iri(&dict, "A")),
        "A is unsatisfiable (CR6 merge + disjointness clash)"
    );
    assert!(
        !unsat.contains(&iri(&dict, "B")),
        "B stays satisfiable (B = {{a}} with A = ∅ is a model) — a symmetric merge would be unsound"
    );
    assert_eq!(h.report().unsatisfiable_classes, 1, "exactly A is unsatisfiable");
    assert_eq!(h.report().skipped_axioms, 0, "every axiom is in-fragment");
}

#[test]
fn has_value_and_singleton_one_of_unify_on_the_same_nominal() {
    // ObjectHasValue(r, a) ≡ ∃r.{a}: an `owl:hasValue :a` restriction and an
    // `owl:someValuesFrom [ owl:oneOf (:a) ]` restriction over the same role are the SAME
    // concept, so A ⊑ ∃r.{a} (hasValue form) and ∃r.{a} ⊑ B (oneOf form) chain to A ⊑ B
    // through plain CR3/CR4 — the two RDF spellings must mint ONE nominal.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :a ] .
         [ owl:onProperty :r ; owl:someValuesFrom [ owl:oneOf ( :a ) ] ] rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B"], &[("A", "B")]);
}

#[test]
fn has_value_role_mismatch_derives_nothing() {
    // A ⊑ ∃r.{a},  ∃s.{a} ⊑ B — same nominal, DIFFERENT roles, no role axioms.
    // ⊭ A ⊑ B (countermodel: r = {(x, a)}, s = ∅, A = {x}, B = ∅). Holds in both feature
    // states: without `rbox` roles only ever compare equal; with `rbox` there is no r ⊑ s
    // axiom to close over.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :a ] .
         [ owl:onProperty :s ; owl:hasValue :a ] rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B"], &[]);
}

#[test]
fn out_of_fragment_nominal_shapes_stay_skipped() {
    // The fragment boundary, pinned exactly:
    //   [oneOf (a b)] ⊑ C      — multi-individual enumeration = disjunction, OUTSIDE EL; skip.
    //   [oneOf ("lex")] ⊑ D    — STRING DataOneOf (non-numeric concrete domain); skip.
    //   E ⊑ [hasValue "5"^^xsd] — DataHasValue; skipped WITHOUT `cdomain`, APPLIED with it.
    // None may fabricate a subsumption; the in-fragment F ⊑ G still classifies.
    let ttl = format!(
        "{PRE}
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
         [ owl:oneOf ( :a :b ) ] rdfs:subClassOf :C .
         [ owl:oneOf ( \"lex\" ) ] rdfs:subClassOf :D .
         :E rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue \"5\"^^xsd:integer ] .
         :F rdfs:subClassOf :G ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    // sq-pbz04.2.2: under `cdomain` the exact-numeric DataHasValue
    // ("5"^^xsd:integer) is APPLIED; the multi-oneOf (disjunction) and the STRING
    // DataOneOf ("lex" — a non-numeric datatype, deferred) stay skipped in both states.
    let want = if cfg!(feature = "cdomain") { 2 } else { 3 };
    assert_eq!(
        h.report().skipped_axioms,
        want,
        "out-of-fragment shapes are recorded as skips (feature-state-dependent count)"
    );
    assert_closure(&ttl, &["C", "D", "E", "F", "G"], &[("F", "G")]);
}

#[test]
fn classify_graph_with_nominals_is_idempotent_and_never_leaks_individuals() {
    // The materializing path over a CR6 fixture: (1) the derived A ⊑ B lands as a triple,
    // (2) a second run adds nothing (idempotent — the bead's determinism invariant), and
    // (3) NO emitted triple mentions the individual :a — a nominal is an internal concept,
    // never a named class in the lattice (emitting `:a rdfs:subClassOf …` would conflate
    // the individual with a class).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :B rdfs:subClassOf [ owl:oneOf ( :a ) ] .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let before = triples.len();
    let r1 = classify_graph(&mut dict, &mut triples);
    assert_eq!(r1.emitted_subsumptions, 1, "exactly the CR6-derived A ⊑ B edge is new");
    let a_ind = iri(&dict, "a");
    let sc = dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(
        "http://www.w3.org/2000/01/rdf-schema#subClassOf".to_string(),
    )));
    let (a, b) = (iri(&dict, "A"), iri(&dict, "B"));
    assert!(triples.contains(&[a, sc, b]), "A ⊑ B is materialized");
    for t in &triples[before..] {
        assert!(
            t[0] != a_ind && t[2] != a_ind,
            "the individual :a must never appear in an emitted subsumption"
        );
    }
    let r2 = classify_graph(&mut dict, &mut triples);
    assert_eq!(r2.emitted_subsumptions, 0, "second call is idempotent");
}

/// CR6 × CR10 (role hierarchy): the nominal filler must ride derived role links too — the
/// hasValue link over the SUB-role satisfies the SUPER-role restriction. ⊨ A ⊑ B: every
/// x ∈ A has a ∈ r(x) ⊆ s(x) (r ⊑ s), so x ∈ ∃s.{a} ⊑ B. Without `rbox` this fixture is
/// (by design) NOT derived — role axioms are a gated capability, exercised only here.
#[cfg(feature = "rbox")]
#[test]
fn has_value_link_closes_under_role_hierarchy() {
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :a ] .
         :r rdfs:subPropertyOf :s .
         [ owl:onProperty :s ; owl:hasValue :a ] rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B"], &[("A", "B")]);
}

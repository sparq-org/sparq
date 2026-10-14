// sq-pbz04.2.2: CR7–CR9 (concrete domains) correctness fixtures — faceted
// `owl:onDatatype`/`owl:withRestrictions` satisfiability, `DataHasValue`/singleton
// `DataOneOf` point ranges, and the honest-deferral boundary, all decided on the shared
// `sparq_substrate::numeric` exact tier.
//
// ORACLE NOTE (same discipline as tests/nominals.rs / tests/differential.rs): the oracle is
// hand verification against the XSD value spaces + the Baader–Brandt–Lutz EL++ calculus.
// Every UNSAT expectation carries the arithmetic argument; every SAT / not-derived
// expectation is the SOUNDNESS half — a witness value (or countermodel) shows the verdict
// must NOT flip. The exact-closure and exact-unsat assertions mean a flipped verdict
// (mutated comparison, inverted emptiness, dropped guard) fails loudly in both directions.
#![cfg(feature = "cdomain")]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{classify_graph, Classifier};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

/// Classifies and returns (hierarchy handle helpers): exact unsat set + skip count checks
/// are made by the individual tests; this just parses once.
fn classify(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(ttl, "turtle").expect("parse")
}

/// Exact-closure oracle over named classes (mirrors tests/nominals.rs): every expected
/// pair must be derived and NO other proper named subsumption may be — a spurious
/// concrete-domain derivation fails as loudly as a missing one.
fn assert_closure(ttl: &str, classes: &[&str], expected: &[(&str, &str)]) {
    let (dict, triples) = classify(ttl);
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
fn cr7_empty_integer_range_makes_the_class_unsatisfiable_via_cr5() {
    // The bead's acceptance fixture: Bad ⊑ ∃age.(xsd:integer, [18, 10]). No integer
    // satisfies 18 <= x <= 10, so the filler range is EMPTY (⊑ ⊥, CR7) and the clash
    // propagates over the existential link to Bad by CR5. Ok (a SATISFIABLE range on the
    // same shape) pins the negative direction in the same TBox.
    let ttl = format!(
        "{PRE}
         :Bad rdfs:subClassOf
           [ owl:onProperty :age ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 18 ] [ xsd:maxInclusive 10 ] ) ] ] .
         :Ok rdfs:subClassOf
           [ owl:onProperty :age ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 10 ] [ xsd:maxInclusive 18 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 0, "both faceted ranges are supported");
    let unsat = h.unsatisfiable_classes();
    assert!(
        unsat.contains(&iri(&dict, "Bad")),
        "Bad ⊑ ∃age.∅ must be unsatisfiable (CR7 → CR5)"
    );
    assert!(
        !unsat.contains(&iri(&dict, "Ok")),
        "Ok's range [10, 18] has witness 12 — flipping this verdict is UNSOUND"
    );
    assert_eq!(h.report().unsatisfiable_classes, 1, "exactly Bad is unsatisfiable");
}

#[test]
fn cr7_discrete_tightening_differs_from_dense_decimal() {
    // (5, 6) over xsd:integer holds NO integer → Int unsat; the SAME bounds over
    // xsd:decimal hold e.g. 5.5 → Dec satisfiable. This pins the discrete-vs-dense
    // distinction: an implementation that forgot integer tightening (or applied it to
    // decimals) fails one of the two.
    let ttl = format!(
        "{PRE}
         :IntGap rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minExclusive 5 ] [ xsd:maxExclusive 6 ] ) ] ] .
         :DecGap rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:decimal ;
               owl:withRestrictions ( [ xsd:minExclusive 5 ] [ xsd:maxExclusive 6 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 0);
    let unsat = h.unsatisfiable_classes();
    assert!(unsat.contains(&iri(&dict, "IntGap")), "no integer lies in (5, 6)");
    assert!(!unsat.contains(&iri(&dict, "DecGap")), "5.5 witnesses the decimal (5, 6)");
}

#[test]
fn cr7_point_collapse_and_exclusive_point_is_empty() {
    // [5.0, 5.0] is the single point {5} (satisfiable); (5.0, 5.0] is empty.
    let ttl = format!(
        "{PRE}
         :Point rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:decimal ;
               owl:withRestrictions ( [ xsd:minInclusive 5.0 ] [ xsd:maxInclusive 5.0 ] ) ] ] .
         :NoPoint rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:decimal ;
               owl:withRestrictions ( [ xsd:minExclusive 5.0 ] [ xsd:maxInclusive 5.0 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    let unsat = h.unsatisfiable_classes();
    assert!(!unsat.contains(&iri(&dict, "Point")), "{{5.0}} is non-empty");
    assert!(unsat.contains(&iri(&dict, "NoPoint")), "(5.0, 5.0] is empty");
}

#[test]
fn cr7_derived_type_implicit_bounds_participate() {
    // xsd:byte's value space is [-128, 127]: minInclusive 1000 empties it even though the
    // explicit facets alone are satisfiable — the implicit bounds are load-bearing.
    let ttl = format!(
        "{PRE}
         :BigByte rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:byte ;
               owl:withRestrictions ( [ xsd:minInclusive 1000 ] ) ] ] .
         :NoNat rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:nonNegativeInteger ;
               owl:withRestrictions ( [ xsd:maxExclusive 0 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    let unsat = h.unsatisfiable_classes();
    assert!(unsat.contains(&iri(&dict, "BigByte")), "no byte reaches 1000");
    assert!(unsat.contains(&iri(&dict, "NoNat")), "no nonNegativeInteger is < 0");
    assert_eq!(h.report().unsatisfiable_classes, 2);
}

#[test]
fn cr8_containment_threads_through_the_data_existential() {
    // A ⊑ ∃p.[5, 10]int and ∃p.[0, 20]int ⊑ B, with [5, 10] ⊆ [0, 20] ⊨ A ⊑ B (CR8 as a
    // Sub axiom + the ordinary CR3/CR4 traversal). The REVERSE containment must NOT be
    // derived: X ⊑ ∃p.[0, 20] and ∃p.[5, 10] ⊑ Y — countermodel: p(x) = 0 ∈ [0, 20] but
    // 0 ∉ [5, 10], so X ⋢ Y.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 20 ] ) ] ]
           rdfs:subClassOf :B .
         :X rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 20 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ]
           rdfs:subClassOf :Y ."
    );
    // NOTE: A also reaches Y (A's [5,10] filler IS the LHS of the Y axiom — same range,
    // deduped to one concept), and X reaches B likewise. The full expected closure:
    assert_closure(&ttl, &["A", "B", "X", "Y"], &[("A", "B"), ("A", "Y"), ("X", "B")]);
}

#[test]
fn cr8_role_mismatch_derives_nothing() {
    // Same ranges, DIFFERENT data properties: no derivation (the containment axiom is
    // property-independent but the existential traversal is role-exact).
    // Countermodel: p = {(x, 7)}, q = ∅.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ] .
         [ owl:onProperty :q ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 20 ] ) ] ]
           rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B"], &[]);
}

#[test]
fn cr9_has_value_point_and_singleton_data_one_of_unify() {
    // DataHasValue(p, 7) = ∃p.{7} and DataSomeValuesFrom(p, DataOneOf(7)) mint the SAME
    // point range, and {7} ⊆ [0, 20] threads A ⊑ B and C ⊑ B; a point OUTSIDE the range
    // ({25}) derives nothing (countermodel: p(x) = 25).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 7 ] .
         :C rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom [ owl:oneOf ( 7 ) ] ] .
         :D rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 25 ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 20 ] ) ] ]
           rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B", "C", "D"], &[("A", "B"), ("C", "B")]);
}

#[test]
fn cr8_integer_range_inside_decimal_range_but_never_the_reverse() {
    // [5, 10]int ⊆ [0.0, 100.0]dec (integers ARE decimal values) ⊨ A ⊑ B. The reverse
    // sort direction is the DOCUMENTED sound incompleteness: [5.0, 10.0]dec ⊆ [0, 100]int
    // is FALSE anyway here (5.5 is in the decimal range) — countermodel: p(x) = 5.5 — so
    // X ⋢ Y must not be derived.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:decimal ;
             owl:withRestrictions ( [ xsd:minInclusive 0.0 ] [ xsd:maxInclusive 100.0 ] ) ] ]
           rdfs:subClassOf :B .
         :X rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:decimal ;
               owl:withRestrictions ( [ xsd:minInclusive 5.0 ] [ xsd:maxInclusive 10.0 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 100 ] ) ] ]
           rdfs:subClassOf :Y ."
    );
    // Also entailed and derived: A ⊑ Y ([5,10]int ⊆ [0,100]int) and X ⊑ B
    // ([5.0,10.0]dec ⊆ [0.0,100.0]dec, same sort). NOT derived: X ⊑ Y (the honest gap).
    assert_closure(&ttl, &["A", "B", "X", "Y"], &[("A", "B"), ("A", "Y"), ("X", "B")]);
}

#[test]
fn deferred_shapes_produce_no_verdict_and_stay_skipped() {
    // Every deferred shape must (1) count as a skip, (2) derive NOTHING, and (3) never
    // clash — an implementation that "helpfully" guessed a verdict for any of these would
    // be unsound. Shapes: a pattern facet, a double base, a double-valued bound, a string
    // hasValue, and an ill-formed bound ("300"^^xsd:byte).
    let ttl = format!(
        "{PRE}
         :P1 rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:pattern \"[0-9]+\" ] ) ] ] .
         :P2 rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:double ;
               owl:withRestrictions ( [ xsd:minInclusive 18 ] [ xsd:maxInclusive 10 ] ) ] ] .
         :P3 rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive \"5.5\"^^xsd:double ] ) ] ] .
         :P4 rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue \"lex\" ] .
         :P5 rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive \"300\"^^xsd:byte ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(
        h.report().skipped_axioms,
        5,
        "every deferred concrete-domain shape is an honest skip"
    );
    assert!(
        h.unsatisfiable_classes().is_empty(),
        "a DEFERRED shape must never produce an unsat verdict — even P2, whose bounds \
         LOOK empty, is a double base we do not decide"
    );
    assert_closure(&ttl, &["P1", "P2", "P3", "P4", "P5"], &[]);
}

#[test]
fn value_equal_ranges_written_differently_unify() {
    // [5, 10] written with integer facets and with decimal-lexical facets ("5.0"/"10.0")
    // over the SAME integer base canonicalize to ONE range concept, so the two spellings
    // chain: A ⊑ ∃p.(range₁) and ∃p.(range₂) ⊑ B with range₁ = range₂ ⊨ A ⊑ B.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 5.0 ] [ xsd:maxInclusive 10.0 ] ) ] ]
           rdfs:subClassOf :B ."
    );
    assert_closure(&ttl, &["A", "B"], &[("A", "B")]);
}

#[test]
fn classify_graph_with_cdomain_is_idempotent_and_emits_no_range_concepts() {
    // The materializing path: the CR8-derived A ⊑ B lands as ONE new triple, a second run
    // adds nothing, and no minted range concept can leak (ranges have no dict id — checked
    // implicitly: the only new triple is the A ⊑ B edge).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 20 ] ) ] ]
           rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = classify(&ttl);
    let before = triples.len();
    let r1 = classify_graph(&mut dict, &mut triples);
    assert_eq!(r1.emitted_subsumptions, 1, "exactly the derived A ⊑ B edge is new");
    assert_eq!(triples.len(), before + 1);
    let r2 = classify_graph(&mut dict, &mut triples);
    assert_eq!(r2.emitted_subsumptions, 0, "second run is idempotent");
}

#[test]
fn mixed_structure_nodes_are_refused_not_strengthened() {
    // SOUNDNESS GUARD: a "range" node that ALSO carries an intersection is malformed;
    // decoding just its range half in LHS position would STRENGTHEN the axiom
    // ((R ⊓ …) ⊑ D read as R ⊑ D). It must be refused → skipped → derive nothing.
    let ttl = format!(
        "{PRE}
         [ owl:onDatatype xsd:integer ;
           owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ;
           owl:intersectionOf ( :C ) ] rdfs:subClassOf :D .
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 1, "the mixed node's axiom is refused");
    assert_closure(&ttl, &["A", "C", "D"], &[]);
}

// sq-pbz04.2.2 soundness fix (Opus adversarial verify on PR #1434): the strictness
// guard originally only refused the intersectionOf-family structure (on_prop/svf/hasValue/
// intersectionOf/oneOf), NOT the other NON_EL markers — a range node ALSO carrying
// owl:unionOf / owl:allValuesFrom / owl:datatypeComplementOf / … was rescued as range-ONLY,
// dropping that structure and STRENGTHENING an LHS axiom. The three analogue tests below plus
// the threaded G ⋢ H regression pin the completed guard (cd_foreign_marker).

#[test]
fn union_marked_range_nodes_are_refused_not_strengthened() {
    // owl:unionOf analogue of the intersectionOf case above: the mixed node must be
    // refused (skip), and the LEGITIMATE pure range node in the second axiom must
    // still resolve (so skipped_axioms is exactly 1, not 2).
    let ttl = format!(
        "{PRE}
         [ owl:onDatatype xsd:integer ;
           owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ;
           owl:unionOf ( :C :D ) ] rdfs:subClassOf :E .
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 1, "the union-marked node's axiom is refused");
    assert_closure(&ttl, &["A", "C", "D", "E"], &[]);
}

#[test]
fn all_values_marked_range_nodes_are_refused_not_strengthened() {
    // owl:allValuesFrom analogue. No owl:onProperty on the node, so the original
    // on_prop-based structure check never saw it — only the NON_EL marker does.
    let ttl = format!(
        "{PRE}
         [ owl:onDatatype xsd:integer ;
           owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ;
           owl:allValuesFrom :C ] rdfs:subClassOf :E .
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 1, "the allValuesFrom-marked node's axiom is refused");
    assert_closure(&ttl, &["A", "C", "E"], &[]);
}

#[test]
fn datatype_complement_marked_range_nodes_are_refused_not_strengthened() {
    // owl:datatypeComplementOf analogue: the node asserts a datatype NEGATION alongside
    // the facets; decoding the facet half alone would drop the complement.
    let ttl = format!(
        "{PRE}
         [ owl:onDatatype xsd:integer ;
           owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ;
           owl:datatypeComplementOf xsd:integer ] rdfs:subClassOf :E .
         :A rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ] ] ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(h.report().skipped_axioms, 1, "the complement-marked node's axiom is refused");
    assert_closure(&ttl, &["A", "E"], &[]);
}

#[test]
fn union_marked_range_is_not_strengthened_through_an_existential() {
    // LOAD-BEARING regression (empirically confirmed UNSOUND before the cd_foreign_marker
    // guard): the union+range node was rescued as its range half, so the pure range in G's
    // existential chained through value-space containment (CR8) into E and derived G ⊑ H.
    // COUNTERMODEL showing G ⊑ H must NOT hold: take C = D = ∅ ⟹ the union node denotes ∅
    // ⟹ the first axiom is satisfied with E = ∅; put p(g) = 5 so g ∈ G's existential; then
    // ∃p.E is empty and g ∉ H. Deriving G ⊑ H strengthens the mixed LHS node — UNSOUND.
    let ttl = format!(
        "{PRE}
         [ owl:onDatatype xsd:integer ;
           owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ;
           owl:unionOf ( :C :D ) ] rdfs:subClassOf :E .
         :G rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 5 ] ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom :E ] rdfs:subClassOf :H ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert!(
        !h.is_subclass_of(iri(&dict, "G"), iri(&dict, "H")),
        "G ⊑ H must NOT be derived (countermodel: C = D = ∅, E = ∅, p(g) = 5, g ∉ H)"
    );
    assert_eq!(h.report().skipped_axioms, 1, "exactly the union+range axiom is refused");
    assert_closure(&ttl, &["C", "D", "E", "G", "H"], &[]);
}

// sq-pbz04.2.2 non-vacuous regression tests for the cd_foreign_marker guards on
// the DataHasValue path (extract.rs:451) and the DataOneOf path (extract.rs:412, inside
// `structure()`). The four existing tests above only exercise faceted-range nodes; these two
// confirm that LITERAL-hasValue and singleton-oneOf nodes carrying a foreign marker (unionOf)
// are also refused, killing the clean mutation survivors identified by the Opus adversarial
// re-verify on PR #1434.

#[test]
fn data_has_value_marked_node_is_refused_not_strengthened() {
    // SOUNDNESS GUARD: a DataHasValue restriction node (owl:onProperty + literal owl:hasValue)
    // that ALSO carries owl:unionOf is poisoned by `cd_foreign_marker`.  Without the
    // `!idx.cd_foreign_marker.contains(&n)` guard at extract.rs:451, the node would be
    // decoded as ∃p.{5}, silently dropping the unionOf structure and STRENGTHENING the
    // enclosing axiom (A ⊑ ∃p.{5} instead of A ⊑ ∃p.{5} ⊓ (C ⊔ D)), enabling a spurious
    // CR9/CR8 chain to B. The guard must refuse the mixed node (skip it), so A never
    // acquires the ∃p.{5} link. Control: A2 with a CLEAN DataHasValue axiom must still
    // derive B (oracle: {5} ⊆ [0, 10]).
    //
    // MUTATION-KILL (extract.rs:451): deleting the guard lets the mixed node enter `cd_exists`
    // — the A axiom stops being skipped (skipped_axioms drops to 0) and A wrongly derives B
    // via CR9/CR8 — both `assert_eq!(skipped_axioms, 1)` and `assert!(!A ⊑ B)` go RED.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ; owl:unionOf ( :C :D ) ] .
         :A2 rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 10 ] ) ] ]
           rdfs:subClassOf :B ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(
        h.report().skipped_axioms,
        1,
        "exactly the union-poisoned DataHasValue axiom is refused; the clean A2 axiom is processed"
    );
    assert!(
        !h.is_subclass_of(iri(&dict, "A"), iri(&dict, "B")),
        "A ⊑ B must NOT be derived: the mixed node is refused — decoding it as ∃p.{{5}} \
         would silently drop the unionOf structure and STRENGTHEN the axiom"
    );
    assert!(
        h.is_subclass_of(iri(&dict, "A2"), iri(&dict, "B")),
        "A2 ⊑ B must be derived: clean DataHasValue ∃p.{{5}} chains through {{5}} ⊆ [0, 10]"
    );
    assert_closure(&ttl, &["A", "A2", "B", "C", "D"], &[("A2", "B")]);
}

#[test]
fn data_one_of_marked_node_is_refused_not_strengthened() {
    // SOUNDNESS GUARD: a DataOneOf range node (singleton literal owl:oneOf) that ALSO
    // carries owl:unionOf is poisoned by the `cd_foreign_marker` branch inside `structure()`
    // at extract.rs:412. Without that branch, the node would enter `points` → `cd_range`,
    // and the axiom `<mixed> rdfs:subClassOf :E` would decode as `R_{{5}} ⊑ E`, silently
    // dropping the unionOf structure (LHS-strengthening). The guard refuses it (skip).
    //
    // MUTATION-KILL (extract.rs:412, `|| idx.cd_foreign_marker.contains(&n)` in `structure()`):
    // deleting it lets the mixed node enter `cd_range` — the axiom is decoded instead of
    // skipped, so `assert_eq!(skipped_axioms, 1)` goes RED. Control: F via a PURE DataOneOf
    // someValuesFrom filler must still chain through {{5}} ⊆ [0, 10] into G.
    let ttl = format!(
        "{PRE}
         [ owl:oneOf ( 5 ) ; owl:unionOf ( :C :D ) ] rdfs:subClassOf :E .
         :F rdfs:subClassOf
           [ owl:onProperty :p ; owl:someValuesFrom [ owl:oneOf ( 5 ) ] ] .
         [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 0 ] [ xsd:maxInclusive 10 ] ) ] ]
           rdfs:subClassOf :G ."
    );
    let (dict, triples) = classify(&ttl);
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(
        h.report().skipped_axioms,
        1,
        "exactly the union-poisoned DataOneOf axiom is refused; the F axiom and range axiom proceed"
    );
    assert!(
        h.is_subclass_of(iri(&dict, "F"), iri(&dict, "G")),
        "F ⊑ G must be derived: clean ∃p.{{5}} (pure DataOneOf filler) chains through \
         {{5}} ⊆ [0, 10] into G — the non-poisoned DataOneOf path is unaffected by the guard"
    );
    assert_closure(&ttl, &["C", "D", "E", "F", "G"], &[("F", "G")]);
}

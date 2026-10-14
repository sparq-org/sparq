// [SONNET-4.6] sq-vkq9u (epic sq-pbz04.2): the ABox × concrete-domain seam — the
// `DataPropertyAssertion` POINT-RANGE RESCUE that exists ONLY when BOTH `abox` and `cdomain`
// are on: `a q 5` is internalized as `{a} ⊑ ∃q.{5}`, with `{5}` the very concept the CR9 point
// machinery mints for a `DataHasValue 5` / singleton `DataOneOf 5` / faceted `[5, 5]` range —
// so CR8 containment threads an asserted data VALUE into the TBox's data-range obligations.
//
// Without either feature the assertion stays the pre-sq-vkq9u fail-closed counted skip
// (`Report::skipped_assertions`), which is why this whole file is gated on the CONJUNCTION.
//
// ORACLE NOTE (same discipline as tests/cdomain.rs / tests/nominals.rs): the oracle is hand
// verification against the XSD value spaces + the Baader–Brandt–Lutz EL++ calculus. Every
// POSITIVE expectation carries the entailment argument; every NEGATIVE one carries the witness
// value or countermodel that makes flipping the verdict UNSOUND. Soundness of the new axiom:
// `a q v` asserts `(a^I, v) ∈ q^I` and `v ∈ {v}^D`, so `a^I ∈ (∃q.{v})^I`; `{a}^I = {a^I}`
// gives `{a} ⊑ ∃q.{v}` in EVERY model.
#![cfg(all(feature = "abox", feature = "cdomain"))]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{realize, Classifier};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn parse(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(ttl, "turtle").expect("parse")
}

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

/// `∃age.(xsd:integer, [18, ∞)) ⊑ Adult` — the recurring TBox obligation the rescued point
/// range has to satisfy through CR8.
const ADULT_TBOX: &str = r#"
    [ owl:onProperty :age ; owl:someValuesFrom
      [ owl:onDatatype xsd:integer ;
        owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] ]
      rdfs:subClassOf :Adult .
"#;

#[test]
fn npa_data_only_source_derived_positive_is_inconsistent() {
    // Every individual is in owl:Thing, including a source mentioned only in an NPA.
    // Thing ⊑ ∃p.{5} therefore entails p(a, 5) without any separate assertion about a.
    let (dict, triples) = parse(&format!(
        "{PRE}
         owl:Thing rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ] .
         [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
           owl:assertionProperty :p ; owl:targetValue 5 ] ."
    ));
    let r = realize(&dict, &triples);
    assert!(r.is_inconsistent());
    assert!(r.type_assertions().is_empty());
}

#[test]
fn npa_data_invalid_property_does_not_register_source() {
    // A malformed NPA must not introduce a, even though Thing ⊑ C types every individual.
    for property in [r#""invalid""#, "_:property", "owl:Thing"] {
        let (dict, triples) = parse(&format!(
            "{PRE}
             owl:Thing rdfs:subClassOf :C .
             [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
               owl:assertionProperty {property} ; owl:targetValue 5 ] ."
        ));
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "invalid property: {property}");
        assert!(
            r.type_assertions()
                .iter()
                .all(|&(source, _)| source != iri(&dict, "a")),
            "a malformed NPA must not realize its source: {property}"
        );
        assert_eq!(
            r.report().skipped_assertions,
            1,
            "invalid property: {property}"
        );
    }
}

#[test]
fn npa_data_only_source_without_derived_positive_is_consistent() {
    // A can be empty, so A ⊑ ∃p.{5} does not force p(a, 5) for this NPA-only source.
    let (dict, triples) = parse(&format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ] .
         [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
           owl:assertionProperty :p ; owl:targetValue 5 ] ."
    ));
    assert!(!realize(&dict, &triples).is_inconsistent());
}

#[test]
fn npa_data_derived_point_is_inconsistent() {
    // No positive data triple is asserted: a : A and A ⊑ ∃p.{5} entail p(a, 5).
    for filler in [
        "owl:hasValue 5",
        "owl:someValuesFrom [ owl:oneOf ( 5 ) ]",
        "owl:someValuesFrom :Point",
        "owl:someValuesFrom [ owl:onDatatype xsd:integer ; owl:withRestrictions
          ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 5 ] ) ]",
    ] {
        let (dict, triples) = parse(&format!(
            "{PRE}
             :Point a rdfs:Datatype ; owl:equivalentClass [ owl:oneOf ( 5 ) ] .
             :A rdfs:subClassOf [ owl:onProperty :p ; {filler} ] .
             :a a :A .
             [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
               owl:assertionProperty :p ; owl:targetValue 5 ] ."
        ));
        let r = realize(&dict, &triples);
        assert!(
            r.is_inconsistent(),
            "derived point from {filler} contradicts the NPA"
        );
        assert!(r.type_assertions().is_empty());
    }
}

#[test]
fn npa_data_numeric_value_equality_is_inconsistent() {
    for (positive, negative) in [("5", "5.0"), ("5.0", "5"), ("5.50", "5.5")] {
        for derived in [false, true] {
            let positive_axiom = if derived {
                format!(":a a :A . :A rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue {positive} ] .")
            } else {
                format!(":a :p {positive} .")
            };
            let (dict, triples) = parse(&format!(
                "{PRE}{positive_axiom}
                 [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
                   owl:assertionProperty :p ; owl:targetValue {negative} ] ."
            ));
            assert!(
                realize(&dict, &triples).is_inconsistent(),
                "{positive} and {negative} share a point (derived={derived})"
            );
        }
    }
}

#[test]
fn npa_data_non_numeric_literals_keep_term_identity() {
    for (positive, negative, clashes) in [
        (r#""05""#, r#""5""#, false),
        (r#""5""#, r#""5""#, true),
        (r#""5"@en"#, r#""5"@fr"#, false),
        (r#""5""#, "5", false),
    ] {
        let (dict, triples) = parse(&format!(
            "{PRE} :a :p {positive} .
             [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
               owl:assertionProperty :p ; owl:targetValue {negative} ] ."
        ));
        assert_eq!(
            realize(&dict, &triples).is_inconsistent(),
            clashes,
            "non-numeric literals match only identical terms: {positive}, {negative}"
        );
    }
}

#[test]
fn npa_data_requires_the_positive_for_this_source_property_and_value() {
    // A minted point alone is insufficient, as is a positive for a different tuple.
    for positive in [
        ":A rdfs:subClassOf [ owl:onProperty :p ; owl:hasValue 5 ] .",
        ":b :p 5 .",
        ":a :q 5 .",
        ":a :p 6 . :b :p 5 .",
    ] {
        let (dict, triples) = parse(&format!(
            "{PRE}{positive}
             [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
               owl:assertionProperty :p ; owl:targetValue 5.0 ] ."
        ));
        assert!(
            !realize(&dict, &triples).is_inconsistent(),
            "a model with no p(a, 5) exists: {positive}"
        );
    }
}

#[test]
fn npa_data_wider_range_does_not_entail_a_particular_value() {
    // The existential can be witnessed by 6. The unrelated b p 5 mints {5}, so checking
    // range containment in the wrong direction would fabricate a clash for a.
    let (dict, triples) = parse(&format!(
        "{PRE} :a a :A . :b :p 5 .
         :A rdfs:subClassOf [ owl:onProperty :p ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ; owl:withRestrictions
             ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 6 ] ) ] ] .
         [ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ;
           owl:assertionProperty :p ; owl:targetValue 5 ] ."
    ));
    assert!(!realize(&dict, &triples).is_inconsistent());
}

// --- The bead's acceptance fixture: `a q 5` ⇒ `{a} ⊑ ∃q.{5}`, read off as a typing. ---------
#[test]
fn data_property_assertion_types_through_a_faceted_range() {
    // alice age 42 ⇒ {alice} ⊑ ∃age.{42}; CR8 gives {42} ⊑ [18, ∞) (42 >= 18), so the
    // existential link from {alice} carries the range in its S-set and the TBox axiom
    // ∃age.[18, ∞) ⊑ Adult fires (CR4). bob pins the SOUNDNESS half: 7 ∉ [18, ∞), and the
    // countermodel is immediate (interpret Adult as ∅ ∪ {alice}).
    let ttl = format!(
        "{PRE}{ADULT_TBOX}
         :alice :age 42 .
         :bob :age 7 ."
    );
    let (dict, triples) = parse(&ttl);
    let r = realize(&dict, &triples);
    assert!(!r.is_inconsistent());
    assert_eq!(
        r.report().skipped_assertions,
        0,
        "both data-property assertions are RESCUED as point ranges, not skipped"
    );
    let (alice, bob, adult) = (iri(&dict, "alice"), iri(&dict, "bob"), iri(&dict, "Adult"));
    assert!(
        r.type_assertions().contains(&(alice, adult)),
        "alice age 42 ⊨ alice : Adult ({{42}} ⊑ [18, ∞) threaded through ∃age)"
    );
    assert!(
        !r.type_assertions().contains(&(bob, adult)),
        "bob age 7: 7 < 18, so deriving Adult here would be UNSOUND"
    );
}

// --- The point shares ONE concept with the TBox's own point forms (the Mint dedup). ---------
#[test]
fn asserted_point_shares_the_tbox_point_concept() {
    // Three TBox spellings of the point {5} — a faceted [5, 5], a DataHasValue 5, and a
    // singleton DataOneOf 5 — plus `:f v 5.0` (an xsd:decimal literal whose value IS the
    // integer 5). All four must resolve to the SAME minted concept, so `f` picks up all three
    // TBox superclasses. A per-shape concept would derive none of them.
    let ttl = format!(
        "{PRE}
         [ owl:onProperty :v ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 5 ] ) ] ]
           rdfs:subClassOf :ViaFacets .
         [ owl:onProperty :v ; owl:hasValue 5 ] rdfs:subClassOf :ViaHasValue .
         [ owl:onProperty :v ; owl:someValuesFrom [ owl:oneOf ( 5 ) ] ]
           rdfs:subClassOf :ViaOneOf .
         :f :v 5.0 ."
    );
    let (dict, triples) = parse(&ttl);
    let r = realize(&dict, &triples);
    assert!(!r.is_inconsistent());
    assert_eq!(r.report().skipped_assertions, 0, "`:f :v 5.0` is a rescued point");
    let f = iri(&dict, "f");
    for sup in ["ViaFacets", "ViaHasValue", "ViaOneOf"] {
        assert!(
            r.type_assertions().contains(&(f, iri(&dict, sup))),
            "{{5.0}} = {{5}} must share the minted concept behind {sup}"
        );
    }
}

// --- Distinct values stay distinct (no fabricated containment between two points). ----------
#[test]
fn distinct_asserted_points_do_not_merge() {
    // ∃v.{5} ⊑ Five ; g v 5, h v 6. Deriving Five for h would be unsound (countermodel:
    // v(h) = 6 only, Five = {g}). Two DIFFERENT points are never in a containment relation.
    let ttl = format!(
        "{PRE}
         [ owl:onProperty :v ; owl:hasValue 5 ] rdfs:subClassOf :Five .
         :g :v 5 .
         :h :v 6 ."
    );
    let (dict, triples) = parse(&ttl);
    let r = realize(&dict, &triples);
    assert!(!r.is_inconsistent());
    let five = iri(&dict, "Five");
    assert!(r.type_assertions().contains(&(iri(&dict, "g"), five)));
    assert!(
        !r.type_assertions().contains(&(iri(&dict, "h"), five)),
        "h v 6 ⊭ h : Five — {{6}} ⊄ {{5}}"
    );
    // Nor does a shared data property make the two individuals equal.
    assert!(r.same_as().is_empty(), "no owl:sameAs may be fabricated from data values");
}

// --- An ABox-only clash the TBox classifier cannot see, reached through the rescue. ---------
#[test]
fn asserted_values_in_disjoint_data_ranges_are_inconsistent() {
    // ∃age.[18, ∞) ⊑ Adult ; ∃age.(-∞, 10] ⊑ Child ; Adult ⊓ Child ⊑ ⊥ ; eve age 42, 5.
    // A data property is multi-valued in OWL, so both assertions hold: 42 ⊨ eve : Adult and
    // 5 ⊨ eve : Child, which contradicts the disjointness — the ontology has NO model.
    let ttl = format!(
        "{PRE}{ADULT_TBOX}
         [ owl:onProperty :age ; owl:someValuesFrom
           [ owl:onDatatype xsd:integer ;
             owl:withRestrictions ( [ xsd:maxInclusive 10 ] ) ] ]
           rdfs:subClassOf :Child .
         :Adult owl:disjointWith :Child .
         :eve :age 42 , 5 ."
    );
    let (dict, triples) = parse(&ttl);
    let r = realize(&dict, &triples);
    assert!(
        r.is_inconsistent(),
        "eve ∈ Adult ⊓ Child ⊑ ⊥ via two rescued point ranges ⇒ inconsistent"
    );
    // The load-bearing contrast: the clash is ABox-ONLY. Adult and Child are each satisfiable,
    // so the (assertion-agnostic) TBox classifier flags NO unsatisfiable class.
    let h = Classifier::classify(&dict, &triples);
    assert!(h.unsatisfiable_classes().is_empty());
    assert!(!h.is_inconsistent(), "the TBox path never decides whole-ontology consistency");
}

// --- The fail-closed boundary: an out-of-tier literal keeps its counted skip. ---------------
#[test]
fn unsupported_literals_stay_counted_skips() {
    // A plain string, a language-tagged string, an xsd:double (the representation-boundary
    // deferral the CR7–CR9 tier makes everywhere) and a literal ill-formed for its own
    // datatype are ALL outside the exact numeric tower: no point may be minted for them, so
    // each stays a fail-closed skip. Guessing `{"Carol"}` would be a fabricated value space.
    let ttl = format!(
        "{PRE}
         :carol :name \"Carol\" .
         :carol :nick \"Caz\"@en .
         :carol :score \"4.2e1\"^^xsd:double .
         :carol :small \"300\"^^xsd:byte .
         :carol :age 30 ."
    );
    let (dict, triples) = parse(&ttl);
    let r = realize(&dict, &triples);
    assert!(!r.is_inconsistent());
    assert_eq!(
        r.report().skipped_assertions,
        4,
        "string / lang-tagged / double / out-of-bounds-byte defer; only `:age 30` is rescued"
    );
}

// --- The TBox surface is untouched: `Classifier::classify` never internalizes assertions. ---
#[test]
fn tbox_classification_is_unchanged_by_the_rescue() {
    // The same graph with and without the data-property assertions must classify IDENTICALLY
    // through `Classifier::classify` — the rescue lives strictly on the `realize` path, and the
    // extra minted point concepts must not perturb the named-class lattice or the skip counts.
    let tbox = format!("{PRE}{ADULT_TBOX} :Adult rdfs:subClassOf :Person .");
    let with_abox = format!("{tbox}\n:alice :age 42 .");
    let (d1, t1) = parse(&tbox);
    let (d2, t2) = parse(&with_abox);
    let (h1, h2) = (Classifier::classify(&d1, &t1), Classifier::classify(&d2, &t2));
    assert_eq!(h1.report().skipped_axioms, h2.report().skipped_axioms);
    assert_eq!(h1.report().unsatisfiable_classes, 0);
    assert_eq!(h2.report().unsatisfiable_classes, 0);
    assert!(h2.is_subclass_of(iri(&d2, "Adult"), iri(&d2, "Person")));
    // No individual becomes a class: the TBox surface has no `alice` subsumption to report.
    assert!(!h2.is_subclass_of(iri(&d2, "alice"), iri(&d2, "Person")));
}

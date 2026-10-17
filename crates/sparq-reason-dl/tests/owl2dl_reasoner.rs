// #7031: the OWL 2 DL reasoner (`owl2dl::check`, `owl2dl::Reasoner`).
//
// Hand-written cases pin each rule of the calculus (one consistent and one inconsistent
// ontology per construct). The differential test runs a seeded generator of random SHIQ
// ontologies and compares consistency, unsatisfiable classes and every entailed named
// subsumption with answers recorded from an independent OWL 2 DL reasoner
// (`tests/data/owl2dl_oracle_expected.txt`); a second, focused corpus and a corpus of data
// properties and datatypes have their own answers (`tests/data/owl2dl_oracle_focused_expected.txt`,
// `tests/data/owl2dl_oracle_datatypes_expected.txt`). To regenerate them, dump the corpora with
// `OWL2DL_ORACLE_DUMP=<dir> cargo test -p sparq-reason-dl --test owl2dl_reasoner oracle`,
// run the reference reasoner over `<dir>/<corpus>/*.ttl` in seed order, and write its answers
// in the same line format (see `render`).

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_dl::owl2dl::{
    check, extract, Budget, Consistency, DlReport, Reasoner, UnknownReason, MAX_CARDINALITY,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

fn run(body: &str) -> (Dict, DlReport) {
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").expect("parse");
    let o = extract(&d, &t).unwrap_or_else(|e| panic!("extract: {e}"));
    let r = check(&d, &o, Budget::default());
    (d, r)
}

fn ex(d: &Dict, frag: &str) -> Id {
    d.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

fn consistent(body: &str) -> bool {
    match run(body).1.consistency {
        Consistency::Consistent => true,
        Consistency::Inconsistent => false,
        Consistency::Unknown(r) => panic!("unknown: {r:?}"),
    }
}

fn entails_sub(body: &str, a: &str, b: &str) -> bool {
    let (d, r) = run(body);
    let c = r.classification.expect("consistent");
    c.subsumptions.contains(&(ex(&d, a), ex(&d, b)))
}

fn unsat(body: &str, a: &str) -> bool {
    let (d, r) = run(body);
    let c = r.classification.expect("consistent");
    c.unsatisfiable.contains(&ex(&d, a))
}

fn some(r: &str, c: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom {c} ]")
}

fn all(r: &str, c: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {c} ]")
}

fn max(n: u32, r: &str, c: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {r} ; owl:maxQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {c} ]"
    )
}

fn min(n: u32, r: &str, c: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {r} ; owl:minQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {c} ]"
    )
}

const DECL: &str = ":A a owl:Class . :B a owl:Class . :C a owl:Class . :D a owl:Class .
    :r a owl:ObjectProperty . :s a owl:ObjectProperty . :t a owl:ObjectProperty .
    :a a owl:NamedIndividual . :b a owl:NamedIndividual . :c a owl:NamedIndividual .";

// ---- consistency ------------------------------------------------------------------------

#[test]
fn empty_ontology_is_consistent() {
    assert!(consistent(DECL));
}

#[test]
fn disjoint_classes_clash_on_a_shared_individual() {
    assert!(!consistent(&format!(
        "{DECL} :A owl:disjointWith :B . :a a :A , :B ."
    )));
    assert!(consistent(&format!(
        "{DECL} :A owl:disjointWith :B . :a a :A . :b a :B ."
    )));
}

#[test]
fn union_branches_and_backtracks() {
    // a ∈ (B ⊔ C), both disjoint with A, a ∈ A: every branch closes.
    let body =
        format!("{DECL} :a a [ owl:unionOf ( :B :C ) ] , :A . :A owl:disjointWith :B , :C .");
    assert!(!consistent(&body));
    let body = format!("{DECL} :a a [ owl:unionOf ( :B :C ) ] , :A . :A owl:disjointWith :B .");
    assert!(consistent(&body));
}

#[test]
fn universal_restriction_reaches_a_successor() {
    let body = format!(
        "{DECL} :a a {} . :a :r :b . :b a [ owl:complementOf :B ] .",
        all(":r", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn existential_creates_a_witness() {
    // a ∈ ∃r.B, everything r-reachable is ¬B: inconsistent without any ABox successor.
    let body = format!(
        "{DECL} :a a {} , {} .",
        some(":r", ":B"),
        all(":r", "[ owl:complementOf :B ]")
    );
    assert!(!consistent(&body));
}

#[test]
fn inverse_roles_propagate_back() {
    // a ∈ ∃r.(∀r⁻.B) entails a ∈ B.
    let body = format!(
        "{DECL} :a a {} , [ owl:complementOf :B ] .",
        some(":r", &all("[ owl:inverseOf :r ]", ":B"))
    );
    assert!(!consistent(&body));
}

#[test]
fn functional_property_merges_and_clashes() {
    let base = format!("{DECL} :r a owl:FunctionalProperty . :a :r :b , :c .");
    assert!(consistent(&base));
    assert!(!consistent(&format!("{base} :b owl:differentFrom :c .")));
    assert!(!consistent(&format!(
        "{base} :b a :A . :c a :B . :A owl:disjointWith :B ."
    )));
}

#[test]
fn at_most_restriction_counts_distinct_successors() {
    let body = format!(
        "{DECL} :a a {} , {} .",
        max(1, ":r", ":B"),
        min(2, ":r", ":B")
    );
    assert!(!consistent(&body));
    let body = format!(
        "{DECL} :a a {} , {} .",
        max(2, ":r", ":B"),
        min(2, ":r", ":B")
    );
    assert!(consistent(&body));
}

#[test]
fn transitive_role_carries_universal_restrictions() {
    let body = format!(
        "{DECL} :r a owl:TransitiveProperty . :a a {} . :a :r :b . :b :r :c . :c a [ owl:complementOf :B ] .",
        all(":r", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn role_hierarchy_feeds_super_roles() {
    let body = format!(
        "{DECL} :r rdfs:subPropertyOf :s . :a a {} . :a :r :b . :b a [ owl:complementOf :B ] .",
        all(":s", ":B")
    );
    assert!(!consistent(&body));
}

#[test]
fn same_and_different_individuals() {
    assert!(!consistent(&format!(
        "{DECL} :a owl:sameAs :b . :a owl:differentFrom :b ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :a owl:sameAs :b . :a a :A . :b a :B . :A owl:disjointWith :B ."
    )));
}

#[test]
fn negative_property_assertion() {
    let npa = "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :r ; owl:targetIndividual :b ]";
    assert!(consistent(&format!("{DECL} {npa} .")));
    assert!(!consistent(&format!("{DECL} {npa} . :a :r :b .")));
    assert!(!consistent(&format!(
        "{DECL} {npa} . :r rdfs:subPropertyOf :s . :s rdfs:subPropertyOf :r . :a :s :b ."
    )));
}

#[test]
fn role_characteristics() {
    assert!(!consistent(&format!(
        "{DECL} :r a owl:IrreflexiveProperty . :a :r :a ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:AsymmetricProperty . :a :r :b . :b :r :a ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r owl:propertyDisjointWith :s . :a :r :b . :a :s :b ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:ReflexiveProperty . :r a owl:IrreflexiveProperty . :a a owl:Thing ."
    )));
    assert!(!consistent(&format!(
        "{DECL} :r a owl:SymmetricProperty . :a :r :b . :b a {} .",
        all(":r", "owl:Nothing")
    )));
}

#[test]
fn tbox_only_inconsistency() {
    assert!(!consistent(&format!(
        "{DECL} owl:Thing rdfs:subClassOf owl:Nothing ."
    )));
    assert!(!consistent(&format!(
        "{DECL} owl:Thing rdfs:subClassOf {} , {} .",
        some(":r", ":B"),
        all(":r", "[ owl:complementOf :B ]")
    )));
}

#[test]
fn cyclic_tbox_terminates_through_blocking() {
    // Every A has an r-successor in A: an infinite model, found finitely by blocking.
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . :a a :A . [ owl:inverseOf :r ] a owl:FunctionalProperty .",
        some(":r", ":A")
    );
    assert!(consistent(&body));
}

// ---- classification ---------------------------------------------------------------------

#[test]
fn classification_finds_entailed_subsumptions() {
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . {} rdfs:subClassOf :C . :D owl:equivalentClass :C .",
        some(":r", ":B"),
        some(":r", "owl:Thing")
    );
    assert!(entails_sub(&body, "A", "C"));
    assert!(entails_sub(&body, "A", "D"));
    assert!(!entails_sub(&body, "B", "C"));
    let (d, r) = run(&body);
    let c = r.classification.unwrap();
    // C ⊑ D and D ⊑ C are asserted; A ⊑ C and A ⊑ D are not.
    assert!(c.inferred.contains(&(ex(&d, "A"), ex(&d, "C"))));
    assert!(!c.inferred.contains(&(ex(&d, "D"), ex(&d, "C"))));
}

#[test]
fn classification_through_disjunction_needs_a_proof() {
    // A ⊑ B ⊔ C, B ⊑ D, C ⊑ D: A ⊑ D holds in every branch, not deterministically.
    let body = format!(
        "{DECL} :A rdfs:subClassOf [ owl:unionOf ( :B :C ) ] . :B rdfs:subClassOf :D . :C rdfs:subClassOf :D ."
    );
    assert!(entails_sub(&body, "A", "D"));
    assert!(!entails_sub(&body, "A", "B"));
}

#[test]
fn unsatisfiable_classes() {
    let body = format!(
        "{DECL} :A rdfs:subClassOf :B , [ owl:complementOf :B ] . :C rdfs:subClassOf {} , {} .",
        min(3, ":r", "owl:Thing"),
        max(2, ":r", "owl:Thing")
    );
    assert!(unsat(&body, "A"));
    assert!(unsat(&body, "C"));
    assert!(!unsat(&body, "B"));
}

// ---- fail-closed --------------------------------------------------------------------------

#[test]
fn out_of_fragment_constructs_answer_unknown() {
    let (_, r) = run(&format!(
        "{DECL} owl:topObjectProperty rdfs:subPropertyOf :r ."
    ));
    match r.consistency {
        Consistency::Unknown(UnknownReason::OutOfFragment { construct, source }) => {
            assert_eq!(
                construct,
                "owl:topObjectProperty / owl:bottomObjectProperty"
            );
            assert!(!source.is_empty());
        }
        other => panic!("expected OutOfFragment, got {other:?}"),
    }
    assert!(r.classification.is_none());
}

// ---- nominals ----------------------------------------------------------------------------

fn object_one_of(is: &str) -> String {
    format!("[ a owl:Class ; owl:oneOf ( {is} ) ]")
}

fn has_value(r: &str, i: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:hasValue {i} ]")
}

#[test]
fn a_nominal_is_its_individual() {
    // `a ∈ {b}` makes `a` and `b` one element.
    let c = |body: &str| verdict(&format!("{DECL} {body}"));
    assert_eq!(
        c(&format!(
            ":a a {} . :a owl:differentFrom :b .",
            object_one_of(":b")
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":a a {} . :b a :A .", object_one_of(":b"))),
        Consistency::Consistent
    );
    assert_eq!(
        c(&format!(
            ":a a [ owl:complementOf {} ] .",
            object_one_of(":a")
        )),
        Consistency::Inconsistent
    );
    // Through a class: `A ≡ {b}` and `a ∈ A`.
    assert_eq!(
        c(&format!(
            ":A owl:equivalentClass {} . :a a :A . :b a :B . :a a [ owl:complementOf :B ] .",
            object_one_of(":b")
        )),
        Consistency::Inconsistent
    );
}

#[test]
fn nominals_bound_the_domain() {
    // `⊤ ⊑ {a, b}` leaves at most two elements.
    let closed = format!(
        "{DECL} owl:Thing rdfs:subClassOf {} .",
        object_one_of(":a :b")
    );
    assert_eq!(
        verdict(&format!("{closed} :c owl:differentFrom :a , :b .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!("{closed} :a owl:differentFrom :b .")),
        Consistency::Consistent
    );
    let body = format!(
        "{closed} :A rdfs:subClassOf {} . :B rdfs:subClassOf {} .",
        min(3, ":r", "owl:Thing"),
        min(2, ":r", "owl:Thing")
    );
    assert!(unsat(&body, "A"));
    assert!(!unsat(&body, "B"));
}

#[test]
fn class_tests_with_nominals_see_the_abox() {
    // `A ≡ {a}` and `B(a)`: `A ⊑ B`, which only the ABox says.
    let body = format!(
        "{DECL} :A owl:equivalentClass {} . :a a :B .",
        object_one_of(":a")
    );
    assert!(entails_sub(&body, "A", "B"));
    assert!(!entails_sub(&body, "B", "A"));
    // `A ⊑ ∃r.{a}`, `B(a)`, `B ⊑ ∀r⁻.D`: every `A` has an `r` to `a`, so is in `D`.
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . :a a :B . :B rdfs:subClassOf {} .",
        has_value(":r", ":a"),
        all("[ owl:inverseOf :r ]", ":D")
    );
    assert!(entails_sub(&body, "A", "D"));
    assert!(!entails_sub(&body, "D", "A"));
    // Two classes equivalent to one nominal are equivalent.
    let body = format!(
        "{DECL} :A owl:equivalentClass {n} . :B owl:equivalentClass {n} .",
        n = object_one_of(":a")
    );
    assert!(entails_sub(&body, "A", "B") && entails_sub(&body, "B", "A"));
    let (d, o) = {
        let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
        let o = extract(&d, &t).unwrap();
        (d, o)
    };
    let r = Reasoner::new(&d, &o, Budget::default()).unwrap();
    assert_eq!(r.is_subclass_of("http://ex/A", "http://ex/B"), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/A", "http://ex/C"), Some(false));
}

#[test]
fn facts_through_a_merge_depend_on_it() {
    // `A ⊑ {a, b}`, `a ∈ ∃r.C`, `∃r.C ⊑ B`: `A = {b}` with `b ∉ B` is a model, so `A ⋢ B`.
    // The test root merges into `a` on one branch; `B(a)` is derived without any choice,
    // but its clash with the forbidden `B` depends on that branch.
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . :a a {} . {} rdfs:subClassOf :B .",
        object_one_of(":a :b"),
        some(":r", ":C"),
        some(":r", ":C")
    );
    assert!(!entails_sub(&body, "A", "B"));
    // `k`'s two `r`-values merge on one branch (`≤1 r`, the other is `Y`); `m`'s `{j}`
    // successor then lands on `i` through that merge, and `∀s.¬E` clashes with `E(i)` only
    // on that branch. `k ∈ Y`, `i ≠ j` is a model.
    let body = format!(
        "{DECL} :k :r :a , :b . :k a [ a owl:Class ; owl:unionOf ( {} :D ) ] . :a a :A .
         :c a {} , {} .",
        max(1, ":r", "owl:Thing"),
        has_value(":s", ":b"),
        all(":s", "[ owl:complementOf :A ]")
    );
    assert_eq!(verdict(&body), Consistency::Consistent);
}

#[test]
fn nominals_with_blocking_are_decided() {
    // Every person has a parent who is a person, and every person lives in `uk`.
    let base = format!(
        "{DECL} :hasParent a owl:ObjectProperty , owl:AsymmetricProperty , owl:IrreflexiveProperty .
         :livesIn a owl:ObjectProperty .
         :P rdfs:subClassOf {} , {} . :bob a :P .",
        some(":hasParent", ":P"),
        has_value(":livesIn", ":c")
    );
    // An infinite chain of persons all living in `c`: consistent.
    assert_eq!(verdict(&base), Consistency::Consistent);
    // At most two persons live in `c`, so the chain needs a 2-cycle or a loop, which
    // asymmetry and irreflexivity forbid: inconsistent. The NI-rule names the two.
    let capped = format!(
        "{base} {} rdfs:subClassOf {} .",
        object_one_of(":c"),
        max(2, "[ owl:inverseOf :livesIn ]", "owl:Thing")
    );
    assert_eq!(verdict(&capped), Consistency::Inconsistent);
    // With at most one, merging is forced at once: bob is his own parent, a clash.
    let one = format!(
        "{base} {} rdfs:subClassOf {} .",
        object_one_of(":c"),
        max(1, "[ owl:inverseOf :livesIn ]", "owl:Thing")
    );
    assert_eq!(verdict(&one), Consistency::Inconsistent);
}

// ---- keys -------------------------------------------------------------------------------

#[test]
fn keys_identify_named_individuals() {
    let key = format!("{DECL} :A owl:hasKey ( :r ) . :a a :A ; :r :c . :b a :A ; :r :c .");
    assert_eq!(verdict(&key), Consistency::Consistent);
    assert_eq!(
        verdict(&format!("{key} :a owl:differentFrom :b .")),
        Consistency::Inconsistent
    );
    // A shared value on only one of the key's properties does not identify.
    let partial = format!(
        "{DECL} :A owl:hasKey ( :r :s ) . :a a :A ; :r :c . :b a :A ; :r :c . :a owl:differentFrom :b ."
    );
    assert_eq!(verdict(&partial), Consistency::Consistent);
    // Membership derived through the TBox counts, and so does a complex key class.
    let derived = format!(
        "{DECL} [ a owl:Class ; owl:intersectionOf ( :A :B ) ] owl:hasKey ( [ owl:inverseOf :s ] ) .
         :C rdfs:subClassOf :A , :B . :a a :C . :b a :A , :B . :c :s :a , :b . :a owl:differentFrom :b ."
    );
    assert_eq!(verdict(&derived), Consistency::Inconsistent);
    // An inverse property expression in a key: `c` is an `r⁻`-value of both.
    let inverse = format!(
        "{DECL} owl:Thing owl:hasKey ( [ owl:inverseOf :r ] ) . :c :r :a , :b . :a owl:differentFrom :b ."
    );
    assert_eq!(verdict(&inverse), Consistency::Inconsistent);
}

#[test]
fn keys_on_non_simple_properties_are_out_of_fragment() {
    // `a t c t z` gives `a t z`, which a key on `t` would have to see.
    let sub = ":t a owl:TransitiveProperty . :t rdfs:subPropertyOf :r .";
    let chain = ":t owl:propertyChainAxiom ( :t :t ) .";
    for (roles, key) in [
        (sub, ":t"),
        (sub, ":r"),
        (sub, "[ owl:inverseOf :t ]"),
        (chain, ":t"),
    ] {
        let body = format!(
            "{DECL} {roles} owl:Thing owl:hasKey ( {key} ) .
             :z a owl:NamedIndividual . :a :t [ :t :z ] . :b :t :z . :a owl:differentFrom :b ."
        );
        match verdict(&body) {
            Consistency::Unknown(UnknownReason::OutOfFragment { construct, .. }) => {
                assert!(construct.contains("HasKey"), "{construct}")
            }
            other => panic!("{key}: expected OutOfFragment, got {other:?}"),
        }
    }
}

#[test]
fn keys_ignore_anonymous_individuals() {
    // The anonymous individual shares the key value with `a` but is not named, so it is
    // not identified with `a` (which would put `a` in `B` and `¬B`).
    let body = format!(
        "{DECL} :A owl:hasKey ( :r ) . :a a :A , [ owl:complementOf :B ] ; :r :c .
         :a :s [ a :A , :B ; :r :c ] ."
    );
    assert_eq!(verdict(&body), Consistency::Consistent);
    // A class's members in general are not named either: keys do not change the hierarchy.
    let body = format!(
        "{DECL} :A owl:hasKey ( :r ) . :B rdfs:subClassOf :A , {} .",
        has_value(":r", ":c")
    );
    assert!(!entails_sub(&body, "B", "C"));
}

#[test]
fn data_keys_compare_values() {
    let key = format!(
        "{DDECL} :A owl:hasKey ( :d ) . :a a :A ; :d \"1\"^^xsd:int . :b a :A ; :d \"01\"^^xsd:integer ."
    );
    assert_eq!(
        verdict(&format!("{key} :a owl:differentFrom :b .")),
        Consistency::Inconsistent
    );
    let other = format!(
        "{DDECL} :A owl:hasKey ( :d ) . :a a :A ; :d 1 . :b a :A ; :d 2 . :a owl:differentFrom :b ."
    );
    assert_eq!(verdict(&other), Consistency::Consistent);
    // Values the reasoner has not fixed might coincide: no verdict rather than a guess.
    let open = format!(
        "{DDECL} :A owl:hasKey ( :d ) . :A rdfs:subClassOf {} . :a a :A . :b a :A . :a owl:differentFrom :b .",
        dsome(":d", "xsd:integer")
    );
    assert_eq!(
        verdict(&open),
        Consistency::Unknown(UnknownReason::Unconfirmed)
    );
}

#[test]
fn non_simple_role_in_cardinality_answers_unknown() {
    let (_, r) = run(&format!(
        "{DECL} :r a owl:TransitiveProperty . :A rdfs:subClassOf {} .",
        max(1, ":r", "owl:Thing")
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::NotInDl(_))
    ));
}

#[test]
fn imports_downgrade_consistent_to_unknown() {
    let (_, r) = run(&format!(
        "{DECL} <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> ."
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ImportsNotLoaded(_))
    ));
    // An unloaded import could add subsumptions, so no classification is claimed.
    assert!(r.classification.is_none());
    let (_, r) = run(&format!(
        "{DECL} <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> . owl:Thing rdfs:subClassOf owl:Nothing ."
    ));
    assert_eq!(r.consistency, Consistency::Inconsistent);
}

fn reasoner(body: &str) -> (Dict, Reasoner) {
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").expect("parse");
    let o = extract(&d, &t).unwrap_or_else(|e| panic!("extract: {e}"));
    let r = Reasoner::new(&d, &o, Budget::default()).expect("in fragment");
    (d, r)
}

fn owl(d: &Dict, local: &str) -> Id {
    d.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://www.w3.org/2002/07/owl#{local}"
    ))))
}

#[test]
fn negative_assertion_sees_transitive_paths() {
    let neg = "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :r ; owl:targetIndividual :c ]";
    assert!(!consistent(&format!(
        "{DECL} :r a owl:TransitiveProperty . :a :r :b . :b :r :c . {neg} ."
    )));
    // Through a transitive sub-property.
    assert!(!consistent(&format!(
        "{DECL} :s a owl:TransitiveProperty . :s rdfs:subPropertyOf :r . :a :s :b . :b :s :c . {neg} ."
    )));
    assert!(consistent(&format!("{DECL} :a :r :b . :b :r :c . {neg} .")));
}

#[test]
fn large_cardinalities_answer_unknown() {
    let n = MAX_CARDINALITY + 1;
    let (_, r) = run(&format!(
        "{DECL} :A rdfs:subClassOf {} .",
        max(n, ":r", ":B")
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::OutOfFragment { .. })
    ));
    // 65 535 + 1 wrapped to 0 before; it must not become a different restriction.
    let (_, r) = run(&format!(
        "{DECL} :A rdfs:subClassOf {} .",
        max(65_535, ":r", ":B")
    ));
    assert!(matches!(
        r.consistency,
        Consistency::Unknown(UnknownReason::OutOfFragment { .. })
    ));
}

#[test]
fn at_most_with_many_successors_is_decided() {
    let succ: String = (0..12)
        .map(|i| format!(":i{i} a owl:NamedIndividual . :a :r :i{i} . "))
        .collect();
    let diff: String = (0..12)
        .flat_map(|i| (i + 1..12).map(move |j| format!(":i{i} owl:differentFrom :i{j} . ")))
        .collect();
    let at_most = |n: u32| format!("{DECL} {succ} :a a {} .", max(n, ":r", "owl:Thing"));
    assert!(!consistent(&format!("{} {diff}", at_most(11))));
    assert!(consistent(&format!("{} {diff}", at_most(12))));
    assert!(consistent(&at_most(5)));
}

const THING: &str = "http://www.w3.org/2002/07/owl#Thing";
const NOTHING: &str = "http://www.w3.org/2002/07/owl#Nothing";

#[test]
fn queries_answer_builtin_and_unmentioned_classes() {
    let (_, r) = reasoner(&format!(
        "{DECL} :A rdfs:subClassOf :B . owl:Thing rdfs:subClassOf :C . :D owl:disjointWith owl:Nothing ."
    ));
    assert_eq!(r.is_satisfiable(NOTHING), Some(false));
    assert_eq!(r.is_satisfiable(THING), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/A", THING), Some(true));
    assert_eq!(r.is_subclass_of(NOTHING, "http://ex/A"), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/A", "http://ex/B"), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/B", "http://ex/A"), Some(false));
    // :X occurs in no axiom, but everything is a :C.
    assert_eq!(r.is_subclass_of("http://ex/X", "http://ex/C"), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/X", "http://ex/B"), Some(false));
}

#[test]
fn builtin_classes_are_recognised_without_dictionary_entries() {
    // Neither owl:Thing nor owl:Nothing occurs in the graph.
    let (d, r) = reasoner(":A a owl:Class . :B a owl:Class . :A rdfs:subClassOf :B .");
    assert_eq!(owl(&d, "Nothing"), sparq_core::dict::NO_ID);
    assert_eq!(r.is_satisfiable(NOTHING), Some(false));
    assert_eq!(r.is_satisfiable(THING), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/A", THING), Some(true));
    assert_eq!(r.is_subclass_of(THING, NOTHING), Some(false));
    assert_eq!(r.is_subclass_of(NOTHING, "http://ex/B"), Some(true));
}

#[test]
fn distinct_unmentioned_classes_are_distinct() {
    let (_, r) = reasoner(&format!("{DECL} :A rdfs:subClassOf :B ."));
    assert_eq!(r.is_subclass_of("http://ex/X", "http://ex/Y"), Some(false));
    assert_eq!(r.is_subclass_of("http://ex/X", "http://ex/X"), Some(true));
    assert_eq!(r.is_satisfiable("http://ex/X"), Some(true));
}

#[test]
fn clauses_wider_than_the_variable_cap_are_refused() {
    // 17 disjuncts of `≤ 63 r.B` need 17 × 64 successor variables in one clause.
    let union: String = (0..17)
        .map(|_| max(63, ":r", ":B"))
        .collect::<Vec<_>>()
        .join(" ");
    let (_, r) = run(&format!(
        "{DECL} :A rdfs:subClassOf [ a owl:Class ; owl:unionOf ( {union} ) ] ."
    ));
    assert!(
        matches!(
            &r.consistency,
            Consistency::Unknown(UnknownReason::OutOfFragment { construct, .. })
                if construct.contains("MAX_CLAUSE_VARS")
        ),
        "{:?}",
        r.consistency
    );
}

#[test]
fn inner_work_and_node_budgets_answer_unknown() {
    // The at-most clique search over 12 distinct successors is inner work: a step budget
    // that the rule applications alone fit in still runs out.
    let succ: String = (0..12)
        .map(|i| format!(":i{i} a owl:NamedIndividual . :a :r :i{i} . "))
        .collect();
    let diff: String = (0..12)
        .flat_map(|i| (i + 1..12).map(move |j| format!(":i{i} owl:differentFrom :i{j} . ")))
        .collect();
    let body = format!(
        "{PRE}{DECL} {succ} {diff} :a a {} .",
        max(11, ":r", "owl:Thing")
    );
    let (d, t) = Graph::parse_to_triples(&body, "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let r = check(
        &d,
        &o,
        Budget {
            max_steps: 300,
            max_nodes: 100_000,
        },
    );
    assert_eq!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
    // A chain of existentials beyond the node budget.
    let (d, t) = Graph::parse_to_triples(
        &format!("{PRE}{DECL} :a a :A . :A rdfs:subClassOf {} .", some(":r", ":B"))
            .replace(":B ]", "[ a owl:Class ; owl:intersectionOf ( :B [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom :C ] ) ] ]"),
        "turtle",
    )
    .unwrap();
    let o = extract(&d, &t).unwrap();
    let r = check(
        &d,
        &o,
        Budget {
            max_steps: 1_000_000,
            max_nodes: 2,
        },
    );
    assert_eq!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
}

#[test]
fn queries_respect_inconsistency_and_imports() {
    let (_, r) = reasoner(&format!("{DECL} :A owl:disjointWith :B . :a a :A , :B ."));
    assert_eq!(r.is_satisfiable("http://ex/C"), Some(false));
    assert_eq!(r.is_subclass_of("http://ex/C", "http://ex/D"), Some(true));
    assert!(r.classify().is_none());
    let (_, r) = reasoner(&format!(
        "{DECL} <http://ex/o> a owl:Ontology ; owl:imports <http://ex/other> . :A rdfs:subClassOf :B ."
    ));
    // Entailed now, entailed whatever the import adds; a non-entailment could change.
    assert_eq!(r.is_subclass_of("http://ex/A", "http://ex/B"), Some(true));
    assert_eq!(r.is_subclass_of("http://ex/B", "http://ex/A"), None);
    assert_eq!(r.is_satisfiable("http://ex/A"), None);
}

/// A k-bit binary counter: `Start` needs a chain of 2^k distinct nodes before blocking.
fn counter(k: usize) -> String {
    let all =
        |c: &str| format!("[ a owl:Restriction ; owl:onProperty :r ; owl:allValuesFrom {c} ]");
    let and = |cs: &[String]| format!("[ a owl:Class ; owl:intersectionOf ( {} ) ]", cs.join(" "));
    let mut o = String::from(":r a owl:ObjectProperty . :Ctr a owl:Class . :Start a owl:Class .\n");
    o += ":Ctr rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom :Ctr ] .\n";
    for i in 0..k {
        let _ = writeln!(o, ":B{i} a owl:Class . :N{i} a owl:Class .");
        let low: Vec<String> = (0..i).map(|j| format!(":B{j}")).collect();
        let with = |head: &str| {
            let mut v = vec![head.to_string()];
            v.extend(low.iter().cloned());
            v
        };
        if low.is_empty() {
            let _ = writeln!(o, ":B0 rdfs:subClassOf {} .", all(":N0"));
            let _ = writeln!(o, ":N0 rdfs:subClassOf {} .", all(":B0"));
            continue;
        }
        let _ = writeln!(
            o,
            "{} rdfs:subClassOf {} .",
            and(&with(&format!(":B{i}"))),
            all(&format!(":N{i}"))
        );
        let _ = writeln!(
            o,
            "{} rdfs:subClassOf {} .",
            and(&with(&format!(":N{i}"))),
            all(&format!(":B{i}"))
        );
        for j in 0..i {
            let _ = writeln!(
                o,
                "{} rdfs:subClassOf {} .",
                and(&[format!(":B{i}"), format!(":N{j}")]),
                all(&format!(":B{i}"))
            );
            let _ = writeln!(
                o,
                "{} rdfs:subClassOf {} .",
                and(&[format!(":N{i}"), format!(":N{j}")]),
                all(&format!(":N{i}"))
            );
        }
    }
    let start: Vec<String> = std::iter::once(":Ctr".to_string())
        .chain((0..k).map(|i| format!(":N{i}")))
        .collect();
    let _ = writeln!(o, ":Start owl:equivalentClass {} .", and(&start));
    o
}

#[test]
fn the_budget_bounds_blocking_work() {
    // Signature building for blocking is charged, so a small budget stops a long chain
    // quickly instead of running the per-expansion scans unbounded.
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{}", counter(12)), "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let budget = Budget {
        max_steps: 200_000,
        max_nodes: 100_000,
    };
    let r = Reasoner::new(&d, &o, budget).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(r.is_satisfiable("http://ex/Start"), None);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    // With the same budget a small counter is decided.
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{}", counter(3)), "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let r = Reasoner::new(&d, &o, budget).unwrap();
    assert_eq!(r.is_satisfiable("http://ex/Start"), Some(true));
    // Classification shares one budget, so some budget decides consistency but not the
    // hierarchy, and the report says why the hierarchy is missing.
    let mut saw_missing = false;
    for steps in (50..5_000).step_by(50) {
        let report = check(
            &d,
            &o,
            Budget {
                max_steps: steps,
                max_nodes: 100_000,
            },
        );
        let missing =
            report.consistency == Consistency::Consistent && report.classification.is_none();
        assert_eq!(report.classification_unknown.is_some(), missing, "{steps}");
        saw_missing |= missing;
    }
    assert!(saw_missing);
}

#[test]
fn budget_exhaustion_answers_unknown() {
    let (d, t) = Graph::parse_to_triples(
        &format!(
            "{PRE}{DECL} :A rdfs:subClassOf {} . :a a :A .",
            some(":r", ":A")
        ),
        "turtle",
    )
    .unwrap();
    let o = extract(&d, &t).unwrap();
    let r = check(
        &d,
        &o,
        Budget {
            max_steps: 3,
            max_nodes: 100,
        },
    );
    assert_eq!(
        r.consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
}

// ---- data properties and datatypes ----------------------------------------------------------

const DDECL: &str = ":A a owl:Class . :B a owl:Class . :C a owl:Class . :X a owl:Class .
    :d a owl:DatatypeProperty . :e a owl:DatatypeProperty . :f a owl:DatatypeProperty .
    :a a owl:NamedIndividual . :b a owl:NamedIndividual .";

fn verdict(body: &str) -> Consistency {
    run(body).1.consistency
}

fn dsome(d: &str, r: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {d} ; owl:someValuesFrom {r} ]")
}

fn dall(d: &str, r: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {d} ; owl:allValuesFrom {r} ]")
}

fn dvalue(d: &str, lit: &str) -> String {
    format!("[ a owl:Restriction ; owl:onProperty {d} ; owl:hasValue {lit} ]")
}

fn dmin(n: u32, d: &str, r: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {d} ; owl:minQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onDataRange {r} ]"
    )
}

fn dmax(n: u32, d: &str, r: &str) -> String {
    format!(
        "[ a owl:Restriction ; owl:onProperty {d} ; owl:maxQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onDataRange {r} ]"
    )
}

/// `xsd:integer` between `lo` and `hi` inclusive.
fn int_range(lo: i64, hi: i64) -> String {
    format!(
        "[ a rdfs:Datatype ; owl:onDatatype xsd:integer ; owl:withRestrictions ( [ xsd:minInclusive {lo} ] [ xsd:maxInclusive {hi} ] ) ]"
    )
}

fn one_of(lits: &[&str]) -> String {
    format!("[ a rdfs:Datatype ; owl:oneOf ( {} ) ]", lits.join(" "))
}

fn data_unsat(body: &str, class: &str) -> bool {
    let (d, r) = run(&format!("{DDECL} {body}"));
    let c = r
        .classification
        .unwrap_or_else(|| panic!("no classification: {:?}", r.consistency));
    c.unsatisfiable.contains(&ex(&d, class))
}

fn is_datatype_unknown(c: &Consistency) -> bool {
    matches!(c, Consistency::Unknown(UnknownReason::DatatypeUnknown { source }) if !source.is_empty())
}

#[test]
fn an_undecided_alternative_does_not_force_the_next() {
    // `E` is satisfiable (its four values alternate 1, 2 round a four-cycle of `≉`) but the
    // datatype check does not decide it. `A ⊑ E ⊔ B` must not give `A ⊑ B`: closing the `E`
    // branch as undecided is not a proof that `B` is forced.
    let cycle = format!(
        ":g a owl:DatatypeProperty . :d owl:propertyDisjointWith :e . :e owl:propertyDisjointWith :f .
         :f owl:propertyDisjointWith :g . :g owl:propertyDisjointWith :d .
         :A rdfs:subClassOf [ a owl:Class ; owl:unionOf ( [ a owl:Class ; owl:intersectionOf ( {} {} {} {} ) ] :B ) ] .",
        dsome(":d", &int_range(1, 2)),
        dsome(":e", &int_range(1, 2)),
        dsome(":f", &int_range(1, 2)),
        dsome(":g", &int_range(1, 2))
    );
    let body = format!("{PRE}{DDECL} {cycle}");
    let (d, t) = Graph::parse_to_triples(&body, "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let r = Reasoner::new(&d, &o, Budget::default()).unwrap();
    assert_ne!(r.is_subclass_of("http://ex/A", "http://ex/B"), Some(true));
    let report = check(&d, &o, Budget::default());
    if let Some(c) = report.classification {
        assert!(!c.subsumptions.contains(&(ex(&d, "A"), ex(&d, "B"))));
    }
}

#[test]
fn the_data_domain_is_more_than_the_datatype_map() {
    // The Direct Semantics only asks the data domain to contain the map's value spaces:
    // the complement of their union still has values.
    let map = "[ a rdfs:Datatype ; owl:unionOf ( owl:real xsd:float xsd:double xsd:boolean
        xsd:hexBinary xsd:base64Binary xsd:dateTime rdf:PlainLiteral rdf:langString xsd:anyURI rdf:XMLLiteral ) ]";
    let outside = format!("[ a rdfs:Datatype ; owl:datatypeComplementOf {map} ]");
    assert_eq!(
        verdict(&format!("{DDECL} :a a {} .", dsome(":d", &outside))),
        Consistency::Consistent
    );
    assert!(!data_unsat(
        &format!(":X owl:equivalentClass {} .", dsome(":d", &outside)),
        "X"
    ));
    // Two of them are not forced equal: there are infinitely many.
    assert_eq!(
        verdict(&format!(
            "{DDECL} :a a {} , {} . :d owl:propertyDisjointWith :e .",
            dsome(":d", &outside),
            dsome(":e", &outside)
        )),
        Consistency::Consistent
    );
    // But `rdfs:Literal` is the whole domain.
    assert_eq!(
        verdict(&format!(
            "{DDECL} :a a {} .",
            dsome(
                ":d",
                "[ a rdfs:Datatype ; owl:datatypeComplementOf rdfs:Literal ]"
            )
        )),
        Consistency::Inconsistent
    );
}

#[test]
fn datatype_preprocessing_is_inside_the_budget() {
    // Each definition names the previous one twice: the expansion doubles at each level.
    let mut body = String::from(":t0 a rdfs:Datatype ; owl:equivalentClass xsd:integer .");
    for k in 1..=40 {
        let _ = write!(
            body,
            " :t{k} a rdfs:Datatype ; owl:equivalentClass [ a rdfs:Datatype ; owl:unionOf ( :t{p} [ a rdfs:Datatype ; owl:datatypeComplementOf :t{p} ] ) ] .",
            p = k - 1
        );
    }
    let _ = write!(body, " :a :d \"1\"^^:t40 .");
    assert_eq!(
        verdict(&format!("{DDECL} {body}")),
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
    // Many values of one individual: their pairwise inequalities are charged up front.
    let values: Vec<String> = (0..200).map(|i| i.to_string()).collect();
    let body = format!("{PRE}{DDECL} :a :d {} .", values.join(" , "));
    let (d, t) = Graph::parse_to_triples(&body, "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let small = Budget {
        max_steps: 1_000,
        ..Budget::default()
    };
    assert_eq!(
        check(&d, &o, small).consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
    assert_eq!(
        check(&d, &o, Budget::default()).consistency,
        Consistency::Consistent
    );
}

#[test]
fn annotated_repeated_datatype_definitions_answer_datatype_unknown() {
    // Whether two annotated definitions with one range are one axiom depends on their
    // annotations, which are not compared (the profile check leaves it unchecked too).
    let def = |b: &str| format!("_:{b} a rdfs:Datatype ; owl:datatypeComplementOf xsd:string .");
    let ann = |b: &str, ax: &str| {
        format!(
            "_:{ax} a owl:Axiom ; owl:annotatedSource :dt ; owl:annotatedProperty owl:equivalentClass ; owl:annotatedTarget _:{b} ; rdfs:comment \"c\" ."
        )
    };
    for body in [
        format!(
            ":dt a rdfs:Datatype ; owl:equivalentClass _:b1, _:b2 . {} {} {} {}",
            def("b1"),
            def("b2"),
            ann("b1", "x1"),
            ann("b2", "x2")
        ),
        format!(
            ":dt a rdfs:Datatype ; owl:equivalentClass _:b2, _:b1 . {} {} {}",
            def("b2"),
            def("b1"),
            ann("b1", "x1")
        ),
    ] {
        let c = verdict(&format!("{DDECL} {body}"));
        assert!(is_datatype_unknown(&c), "{c:?}");
    }
    // Unannotated repeats are one definition.
    let one = format!(
        "{DDECL} :dt a rdfs:Datatype ; owl:equivalentClass _:b1, _:b2 . {} {}",
        def("b1"),
        def("b2")
    );
    assert_eq!(verdict(&one), Consistency::Consistent);
}

/// The volunteering fixture of issue #7031: qualified cardinality, an inverse, functional
/// object and data properties, a data range, a union domain and `AllDisjointClasses`.
const VOLUNTEERING: &str = r#"
    @prefix vol: <https://w3id.org/odi/volunteering#> .
    vol:Application a owl:Class . vol:Person a owl:Class . vol:Role a owl:Class .
    vol:TimeCommitment a owl:Class . vol:Agent a owl:Class .
    vol:VolunteeringOpportunity a owl:Class . vol:Session a owl:Class .
    vol:submittedBy a owl:ObjectProperty . vol:hasTimeCommitment a owl:ObjectProperty .
    vol:offeredBy a owl:ObjectProperty . vol:offersOpportunity a owl:ObjectProperty .
    vol:hasLocation a owl:ObjectProperty .
    vol:Application rdfs:subClassOf [ a owl:Restriction ;
        owl:onProperty vol:submittedBy ; owl:onClass vol:Person ;
        owl:qualifiedCardinality "1"^^xsd:nonNegativeInteger ] .
    vol:Role rdfs:subClassOf [ a owl:Restriction ;
        owl:onProperty vol:hasTimeCommitment ; owl:onClass vol:TimeCommitment ;
        owl:maxQualifiedCardinality "1"^^xsd:nonNegativeInteger ] .
    vol:offeredBy owl:inverseOf vol:offersOpportunity .
    vol:forOpportunity a owl:ObjectProperty, owl:FunctionalProperty .
    vol:minimumAge a owl:DatatypeProperty, owl:FunctionalProperty ; rdfs:range xsd:nonNegativeInteger .
    vol:hasLocation rdfs:domain [ owl:unionOf ( vol:VolunteeringOpportunity vol:Session ) ] .
    [ a owl:AllDisjointClasses ; owl:members ( vol:Agent vol:VolunteeringOpportunity vol:Role ) ] .
    vol:opp a owl:NamedIndividual , vol:VolunteeringOpportunity .
    vol:org a owl:NamedIndividual , vol:Agent .
    vol:opp vol:offeredBy vol:org .
"#;

#[test]
fn issue_7031_fixture_with_data() {
    let v = |abox: &str| verdict(&format!("{VOLUNTEERING} {abox}"));
    assert_eq!(v(""), Consistency::Consistent);
    assert_eq!(
        v(r#"vol:opp vol:minimumAge "16"^^xsd:nonNegativeInteger ."#),
        Consistency::Consistent
    );
    // Ill-typed: "sixteen" is not in the lexical space, so it has no value.
    assert_eq!(
        v(r#"vol:opp vol:minimumAge "sixteen"^^xsd:nonNegativeInteger ."#),
        Consistency::Inconsistent
    );
    // Well-typed, but outside the range.
    assert_eq!(
        v(r#"vol:opp vol:minimumAge "-1"^^xsd:integer ."#),
        Consistency::Inconsistent
    );
    // Two different ages of a functional property.
    assert_eq!(
        v(
            r#"vol:opp vol:minimumAge "16"^^xsd:nonNegativeInteger , "18"^^xsd:nonNegativeInteger ."#
        ),
        Consistency::Inconsistent
    );
    // The same value, written in two datatypes.
    assert_eq!(
        v(r#"vol:opp vol:minimumAge "16"^^xsd:int , "016"^^xsd:integer ."#),
        Consistency::Consistent
    );
    // The union domain meets the disjointness: an Agent with a location is a Session.
    assert_eq!(
        v("vol:org vol:hasLocation vol:opp ."),
        Consistency::Consistent
    );
    assert_eq!(
        v("vol:org vol:hasLocation vol:opp . vol:org a [ owl:complementOf vol:Session ] ."),
        Consistency::Inconsistent
    );
    // Exactly one submitter.
    assert_eq!(
        v("vol:app a vol:Application ; vol:submittedBy vol:p1 , vol:p2 . vol:p1 a vol:Person . vol:p2 a vol:Person . vol:p1 owl:differentFrom vol:p2 ."),
        Consistency::Inconsistent
    );
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{VOLUNTEERING}"), "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let r = check(&d, &o, Budget::default());
    assert!(r.classification.is_some(), "{:?}", r.classification_unknown);
}

#[test]
fn data_range_and_value_identity() {
    // `1.0` decimal and `1` integer are one value; `-0` and `0` floats are two.
    let f = ":d a owl:FunctionalProperty .";
    assert_eq!(
        verdict(&format!(
            "{DDECL} {f} :a :d \"1.0\"^^xsd:decimal , \"1\"^^xsd:integer ."
        )),
        Consistency::Consistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} {f} :a :d \"0\"^^xsd:float , \"-0\"^^xsd:float ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} {f} :a :d \"1\"^^xsd:float , \"1\"^^xsd:double ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} {f} :a :d \"abc\" , \"abc\"^^xsd:string ."
        )),
        Consistency::Consistent
    );
    // The range applies through a sub-property and an equivalent property.
    assert_eq!(
        verdict(&format!(
            "{DDECL} :e rdfs:range xsd:integer . :d rdfs:subPropertyOf :f . :f owl:equivalentProperty :e . :a :d \"x\" ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} :e rdfs:range xsd:integer . :a :d \"x\" ."
        )),
        Consistency::Consistent
    );
}

#[test]
fn data_restrictions() {
    // ∃ over an empty restriction.
    assert!(data_unsat(
        &format!(":A rdfs:subClassOf {} .", dsome(":d", &int_range(5, 3))),
        "A"
    ));
    assert!(!data_unsat(
        &format!(":A rdfs:subClassOf {} .", dsome(":d", &int_range(3, 5))),
        "A"
    ));
    // ≥3 over two values.
    let two = one_of(&["1", "2"]);
    assert!(data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(3, ":d", &two)),
        "A"
    ));
    assert!(!data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(2, ":d", &two)),
        "A"
    ));
    assert!(data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(3, ":d", &int_range(1, 2))),
        "A"
    ));
    assert!(!data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(3, ":d", &int_range(1, 3))),
        "A"
    ));
    // ≥n over [1, 2] ∪ {5}.
    let union = format!(
        "[ a rdfs:Datatype ; owl:unionOf ( {} {} ) ]",
        int_range(1, 2),
        one_of(&["5"])
    );
    assert!(!data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(3, ":d", &union)),
        "A"
    ));
    assert!(data_unsat(
        &format!(":A rdfs:subClassOf {} .", dmin(4, ":d", &union)),
        "A"
    ));
    // ∀ meets ∃ and has-value.
    assert!(data_unsat(
        &format!(
            ":A rdfs:subClassOf {} , {} .",
            dvalue(":d", "\"7\"^^xsd:integer"),
            dall(":d", &int_range(0, 3))
        ),
        "A"
    ));
    assert!(data_unsat(
        &format!(
            ":A rdfs:subClassOf {} , {} .",
            dsome(":d", "xsd:string"),
            dall(":d", "xsd:integer")
        ),
        "A"
    ));
    // ≤1 against ≥2 of the same range; ≤1 of a range the successors may leave.
    assert!(data_unsat(
        &format!(
            ":A rdfs:subClassOf {} , {} .",
            dmax(1, ":d", "xsd:integer"),
            dmin(2, ":d", "xsd:integer")
        ),
        "A"
    ));
    assert!(!data_unsat(
        &format!(
            ":A rdfs:subClassOf {} , {} .",
            dmax(1, ":d", "xsd:integer"),
            dmin(2, ":d", "rdfs:Literal")
        ),
        "A"
    ));
    let at_most_one = format!(":a a {} .", dmax(1, ":d", "xsd:nonNegativeInteger"));
    assert_eq!(
        verdict(&format!("{DDECL} {at_most_one} :a :d -1 , 2 .")),
        Consistency::Consistent
    );
    assert_eq!(
        verdict(&format!("{DDECL} {at_most_one} :a :d 1 , 2 .")),
        Consistency::Inconsistent
    );
    // The complement of a datatype.
    assert_eq!(
        verdict(&format!(
            "{DDECL} :a a {} . :a :d 5 .",
            dall(
                ":d",
                "[ a rdfs:Datatype ; owl:datatypeComplementOf xsd:integer ]"
            )
        )),
        Consistency::Inconsistent
    );
    // The domain of a data property.
    assert_eq!(
        verdict(&format!(
            "{DDECL} :d rdfs:domain :A . :A owl:disjointWith :B . :a a :B . :a :d 1 ."
        )),
        Consistency::Inconsistent
    );
}

#[test]
fn disjoint_and_negative_data_properties() {
    let disj = ":d owl:propertyDisjointWith :e .";
    assert_eq!(
        verdict(&format!("{DDECL} {disj} :a :d 1 . :a :e \"1\"^^xsd:int .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!("{DDECL} {disj} :a :d 1 . :a :e 2 .")),
        Consistency::Consistent
    );
    assert!(data_unsat(
        &format!(
            "{disj} :A rdfs:subClassOf {} , {} .",
            dvalue(":d", "1"),
            dvalue(":e", "\"1\"^^xsd:int")
        ),
        "A"
    ));
    // ≥2 d.[1,2] and ≥1 e.[1,2] with d, e disjoint: three values needed.
    assert!(data_unsat(
        &format!(
            "{disj} :A rdfs:subClassOf {} , {} .",
            dmin(2, ":d", &int_range(1, 2)),
            dsome(":e", &int_range(1, 2))
        ),
        "A"
    ));
    let npa = "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty :d ; owl:targetValue 5 ]";
    assert_eq!(
        verdict(&format!("{DDECL} {npa} . :a :d 6 .")),
        Consistency::Consistent
    );
    assert_eq!(
        verdict(&format!("{DDECL} {npa} . :a :d \"5\"^^xsd:int .")),
        Consistency::Inconsistent
    );
    // The excluded value is forced by a range.
    assert_eq!(
        verdict(&format!(
            "{DDECL} {npa} . :a a {} , {} .",
            dsome(":d", "xsd:integer"),
            dall(":d", &one_of(&["5"]))
        )),
        Consistency::Inconsistent
    );
}

#[test]
fn datatype_definitions_expand() {
    let adult = ":Adult a rdfs:Datatype ; owl:equivalentClass [ a rdfs:Datatype ; owl:onDatatype xsd:integer ; owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] .";
    let base = format!("{DDECL} {adult} :d rdfs:range :Adult .");
    assert_eq!(
        verdict(&format!("{base} :a :d 20 .")),
        Consistency::Consistent
    );
    assert_eq!(
        verdict(&format!("{base} :a :d 10 .")),
        Consistency::Inconsistent
    );
}

#[test]
fn classification_through_data_ranges() {
    let body = format!(
        "{DDECL} :A owl:equivalentClass {} . :B owl:equivalentClass {} .",
        dsome(":d", &int_range(10, 20)),
        dsome(":d", &int_range(5, 30))
    );
    assert!(entails_sub(&body, "A", "B"));
    assert!(!entails_sub(&body, "B", "A"));
}

/// The cases the differential corpus leaves out because the reference reasoner departs
/// from the Direct Semantics on them (see `datatypes_corpus`), decided as the specification
/// says.
#[test]
fn reference_deviations_follow_the_specification() {
    // A single-value interval at zero is inside an interval ending at zero.
    let body = format!(
        "{DDECL} :A owl:equivalentClass {} . :B owl:equivalentClass {} .",
        dsome(":d", &int_range(0, 0)),
        dsome(":d", &int_range(0, 5))
    );
    assert!(entails_sub(&body, "A", "B"));
    // An at-most over a union of empty ranges holds of everything.
    let empty = |lo: i64, hi: i64| int_range(lo, hi);
    let body = format!(
        "{DDECL} :A rdfs:subClassOf {} . {} rdfs:subClassOf :B .",
        dmax(
            1,
            ":d",
            &format!(
                "[ a rdfs:Datatype ; owl:unionOf ( {} {} ) ]",
                empty(4, 3),
                empty(9, 8)
            )
        ),
        dmax(2, ":d", "xsd:int")
    );
    assert!(!entails_sub(&body, "A", "B"));
    // Exactly one integer value, but two asserted.
    let union = "[ a rdfs:Datatype ; owl:unionOf ( xsd:nonNegativeInteger xsd:integer ) ]";
    assert_eq!(
        verdict(&format!(
            "{DDECL} :a a [ a owl:Restriction ; owl:onProperty :d ; owl:qualifiedCardinality \"1\"^^xsd:nonNegativeInteger ; owl:onDataRange {union} ] . :a :d -1 , \"4\"^^xsd:nonNegativeInteger ."
        )),
        Consistency::Inconsistent
    );
    // A definition by a single-value intersection constrains nothing.
    let single = format!(
        "[ a rdfs:Datatype ; owl:intersectionOf ( {} xsd:integer ) ]",
        one_of(&["2"])
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} :d rdfs:domain {} . :C owl:equivalentClass [ a owl:Class ; owl:complementOf [ a owl:Restriction ; owl:onProperty :d ; owl:qualifiedCardinality \"1\"^^xsd:nonNegativeInteger ; owl:onDataRange {single} ] ] . :a :d 3 .",
            dsome(
                ":d",
                "[ a rdfs:Datatype ; owl:onDatatype xsd:string ; owl:withRestrictions ( [ xsd:length 2 ] ) ]"
            )
        )),
        Consistency::Consistent
    );
}

#[test]
fn ill_typed_literals_are_inconsistent() {
    assert_eq!(
        verdict(&format!(
            "{DDECL} :a :d \"sixteen\"^^xsd:nonNegativeInteger ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} :A rdfs:subClassOf {} .",
            dvalue(":d", "\"x\"^^xsd:boolean")
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        verdict(&format!(
            "{DDECL} :A rdfs:subClassOf {} .",
            dsome(":d", &one_of(&["\"1.5\"^^xsd:integer"]))
        )),
        Consistency::Inconsistent
    );
    // An ill-typed annotation value has no logical meaning.
    assert_eq!(
        verdict(&format!("{DDECL} :a rdfs:label \"x\"^^xsd:integer .")),
        Consistency::Consistent
    );
}

#[test]
fn undecidable_datatype_questions_answer_unknown() {
    // An XSD pattern with a block escape.
    let pattern = "[ a rdfs:Datatype ; owl:onDatatype xsd:string ; owl:withRestrictions ( [ xsd:pattern \"\\\\p{IsBasicLatin}+\" ] ) ]";
    let c = verdict(&format!("{DDECL} :d rdfs:range {pattern} . :a :d \"x\" ."));
    assert!(is_datatype_unknown(&c), "{c:?}");
    // A value beyond the represented range.
    let c = verdict(&format!(
        "{DDECL} :a :d \"1000000000000000000000000000000000000000000\"^^xsd:integer ."
    ));
    assert!(is_datatype_unknown(&c), "{c:?}");
    // A facet bound outside the restricted datatype's value space.
    let c = verdict(&format!(
        "{DDECL} :d rdfs:range [ a rdfs:Datatype ; owl:onDatatype xsd:nonNegativeInteger ; owl:withRestrictions ( [ xsd:maxExclusive -2 ] ) ] ."
    ));
    assert!(is_datatype_unknown(&c), "{c:?}");
    // An rdf:XMLLiteral literal (its XML content is not checked).
    let c = verdict(&format!("{DDECL} :a :d \"<b>x</b>\"^^rdf:XMLLiteral ."));
    assert!(is_datatype_unknown(&c), "{c:?}");
    // A component of the `≉` graph that is neither peelable nor a clique: y and w each
    // differ from z₁ ≠ z₂ but not from each other, all in [1, 2]. (It is unsatisfiable:
    // z₁, z₂ take both values.) The check must not guess.
    let path = format!(
        ":d owl:propertyDisjointWith :e . :f owl:propertyDisjointWith :e . :X rdfs:subClassOf {} , {} , {} .",
        dmin(2, ":e", &int_range(1, 2)),
        dsome(":d", &int_range(1, 2)),
        dsome(":f", &int_range(1, 2))
    );
    let (_, r) = run(&format!("{DDECL} {path} :a a :X ."));
    assert!(is_datatype_unknown(&r.consistency), "{:?}", r.consistency);
    // Undecided when every other branch clashes; consistent when another branch is a model.
    let c = verdict(&format!(
        "{DDECL} {path} :a a [ owl:unionOf ( :X :B ) ] . :a a [ owl:complementOf :B ] ."
    ));
    assert!(is_datatype_unknown(&c), "{c:?}");
    assert_eq!(
        verdict(&format!("{DDECL} {path} :a a [ owl:unionOf ( :X :B ) ] .")),
        Consistency::Consistent
    );
    // The same structure in a TBox only: consistency is decided, the class is not.
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{DDECL} {path}"), "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let report = check(&d, &o, Budget::default());
    assert_eq!(report.consistency, Consistency::Consistent);
    assert!(report.classification.is_none());
    assert!(matches!(
        report.classification_unknown,
        Some(UnknownReason::DatatypeUnknown { .. })
    ));
    let r = Reasoner::new(&d, &o, Budget::default()).unwrap();
    assert_eq!(r.is_satisfiable("http://ex/X"), None);
    // Concrete nodes and their datatype checks count against the budget.
    let body = format!("{PRE}{DDECL} :a a {} .", dmin(40, ":d", &int_range(1, 40)));
    let (d, t) = Graph::parse_to_triples(&body, "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let small = Budget {
        max_steps: 200,
        max_nodes: 100_000,
    };
    assert_eq!(
        check(&d, &o, small).consistency,
        Consistency::Unknown(UnknownReason::ResourceBudget)
    );
    assert_eq!(
        check(&d, &o, Budget::default()).consistency,
        Consistency::Consistent
    );
}

#[test]
fn out_of_fragment_data_constructs_answer_unknown() {
    let out = |body: &str| {
        let c = verdict(&format!("{DDECL} {body}"));
        assert!(
            matches!(c, Consistency::Unknown(UnknownReason::OutOfFragment { .. })),
            "{body}: {c:?}"
        );
    };
    out(":d rdfs:range :Undefined . :Undefined a rdfs:Datatype .");
    out(":T a rdfs:Datatype ; owl:equivalentClass :U . :U a rdfs:Datatype ; owl:equivalentClass :T . :d rdfs:range :T .");
    // Definitions constrain the models even when the datatype is unused.
    out(":T a rdfs:Datatype ; owl:equivalentClass [ a rdfs:Datatype ; owl:datatypeComplementOf :T ] .");
    out(":T a rdfs:Datatype ; owl:equivalentClass xsd:integer , xsd:string .");
    out(":A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty owl:topDataProperty ; owl:someValuesFrom xsd:integer ] .");
    out(":a :d \"x\"^^<http://ex/unknownDatatype> .");
}

// ---- differential test against recorded reference answers ---------------------------------

/// xorshift64*: a small deterministic generator, so the corpus is code, not data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

struct Gen {
    rng: Rng,
    n_classes: usize,
    n_roles: usize,
    simple: Vec<bool>,
    /// Individuals `:i0 …` that nominals may name (0: no nominals).
    nominals: usize,
}

impl Gen {
    fn class(&mut self) -> String {
        format!(":C{}", self.rng.below(self.n_classes))
    }

    fn role(&mut self, simple_only: bool) -> Option<String> {
        let pool: Vec<usize> = (0..self.n_roles)
            .filter(|&i| !simple_only || self.simple[i])
            .collect();
        if pool.is_empty() {
            return None;
        }
        let r = pool[self.rng.below(pool.len())];
        Some(if self.rng.chance(30) {
            format!("[ owl:inverseOf :r{r} ]")
        } else {
            format!(":r{r}")
        })
    }

    fn nominal(&mut self) -> String {
        let n = self.nominals;
        if self.rng.chance(50) {
            let r = self.role(false).unwrap();
            let i = self.rng.below(n);
            return format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:hasValue :i{i} ]");
        }
        let k = 1 + self.rng.below(2.min(n));
        let mut is: Vec<usize> = (0..k).map(|_| self.rng.below(n)).collect();
        is.sort_unstable();
        is.dedup();
        let list: Vec<String> = is.iter().map(|i| format!(":i{i}")).collect();
        format!("[ a owl:Class ; owl:oneOf ( {} ) ]", list.join(" "))
    }

    fn concept(&mut self, depth: usize) -> String {
        if self.nominals > 0 && self.rng.chance(18) {
            return self.nominal();
        }
        if depth == 0 || self.rng.chance(35) {
            return match self.rng.below(10) {
                0 => format!("[ a owl:Class ; owl:complementOf {} ]", self.class()),
                1 if self.rng.chance(30) => "owl:Thing".into(),
                _ => self.class(),
            };
        }
        let d = depth - 1;
        let card = |n: usize| format!("\"{n}\"^^xsd:nonNegativeInteger");
        match self.rng.below(11) {
            0 => format!(
                "[ a owl:Class ; owl:intersectionOf ( {} {} ) ]",
                self.concept(d),
                self.concept(d)
            ),
            1 => format!(
                "[ a owl:Class ; owl:unionOf ( {} {} ) ]",
                self.concept(d),
                self.concept(d)
            ),
            2 => format!("[ a owl:Class ; owl:complementOf {} ]", self.concept(d)),
            3 | 4 => {
                let r = self.role(false).unwrap();
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom {} ]",
                    self.concept(d)
                )
            }
            5 | 6 => {
                let r = self.role(false).unwrap();
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {} ]",
                    self.concept(d)
                )
            }
            k @ 7..=9 => {
                let Some(r) = self.role(true) else {
                    return self.class();
                };
                let (prop, n) = match k {
                    7 => ("owl:minQualifiedCardinality", 1 + self.rng.below(3)),
                    8 => ("owl:maxQualifiedCardinality", self.rng.below(3)),
                    _ => ("owl:qualifiedCardinality", 1 + self.rng.below(2)),
                };
                format!(
                    "[ a owl:Restriction ; owl:onProperty {r} ; {prop} {} ; owl:onClass {} ]",
                    card(n),
                    self.concept(d)
                )
            }
            _ => match self.role(true) {
                Some(r) => format!("[ a owl:Restriction ; owl:onProperty {r} ; owl:hasSelf true ]"),
                None => self.class(),
            },
        }
    }

    fn ontology(seed: u64) -> String {
        Self::ontology_with(seed, false)
    }

    /// The corpus ontology of `seed`; with `nominals`, class expressions may also be
    /// `ObjectOneOf` and `ObjectHasValue` over a few individuals (the RNG stream is
    /// unchanged without).
    fn ontology_with(seed: u64, nominals: bool) -> String {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n_classes = 3 + rng.below(4);
        let n_roles = 1 + rng.below(3);
        let n_nominals = if nominals { 1 + rng.below(3) } else { 0 };
        let mut out = String::new();
        let _ = writeln!(out, "<http://ex/o{seed}> a owl:Ontology .");
        for i in 0..n_nominals {
            let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
        }
        for i in 0..n_classes {
            let _ = writeln!(out, ":C{i} a owl:Class .");
        }
        for i in 0..n_roles {
            let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
        }
        // Role hierarchy and transitivity first, so simplicity is known.
        let mut incl: Vec<(usize, usize)> = Vec::new();
        let mut transitive = vec![false; n_roles];
        for (i, t) in transitive.iter_mut().enumerate() {
            if rng.chance(25) {
                *t = true;
                let _ = writeln!(out, ":r{i} a owl:TransitiveProperty .");
            }
        }
        if n_roles > 1 {
            for _ in 0..rng.below(3) {
                let (a, b) = (rng.below(n_roles), rng.below(n_roles));
                if a != b {
                    incl.push((a, b));
                    if rng.chance(30) {
                        let _ = writeln!(
                            out,
                            "[ owl:inverseOf :r{a} ] rdfs:subPropertyOf [ owl:inverseOf :r{b} ] ."
                        );
                    } else {
                        let _ = writeln!(out, ":r{a} rdfs:subPropertyOf :r{b} .");
                    }
                }
            }
        }
        // A role is non-simple when a transitive role is below it.
        let mut simple: Vec<bool> = transitive.iter().map(|t| !t).collect();
        for _ in 0..n_roles {
            for &(a, b) in &incl {
                if !simple[a] {
                    simple[b] = false;
                }
            }
        }
        let mut g = Gen {
            rng,
            n_classes,
            n_roles,
            simple,
            nominals: n_nominals,
        };
        for i in 0..n_roles {
            if !g.simple[i] {
                continue;
            }
            for (pct, ty) in [
                (12, "owl:FunctionalProperty"),
                (8, "owl:InverseFunctionalProperty"),
                (6, "owl:IrreflexiveProperty"),
                (6, "owl:AsymmetricProperty"),
            ] {
                if g.rng.chance(pct) {
                    let _ = writeln!(out, ":r{i} a {ty} .");
                }
            }
        }
        for i in 0..n_roles {
            if g.rng.chance(8) {
                let _ = writeln!(out, ":r{i} a owl:SymmetricProperty .");
            }
            if g.rng.chance(5) {
                let _ = writeln!(out, ":r{i} a owl:ReflexiveProperty .");
            }
            if g.rng.chance(15) {
                let c = g.concept(1);
                let _ = writeln!(out, ":r{i} rdfs:domain {c} .");
            }
            if g.rng.chance(15) {
                let c = g.concept(1);
                let _ = writeln!(out, ":r{i} rdfs:range {c} .");
            }
        }
        for _ in 0..(2 + g.rng.below(6)) {
            match g.rng.below(10) {
                0..=5 => {
                    let (a, b) = if g.rng.chance(50) {
                        (g.class(), g.concept(2))
                    } else {
                        (g.concept(2), g.class())
                    };
                    let _ = writeln!(out, "{a} rdfs:subClassOf {b} .");
                }
                6 | 7 => {
                    let (a, b) = (g.class(), g.concept(2));
                    let _ = writeln!(out, "{a} owl:equivalentClass {b} .");
                }
                _ => {
                    let (a, b) = (g.class(), g.class());
                    if a != b {
                        let _ = writeln!(out, "{a} owl:disjointWith {b} .");
                    }
                }
            }
        }
        let n_ind = g.rng.below(4);
        for i in 0..n_ind {
            let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
        }
        if n_ind > 0 {
            for _ in 0..g.rng.below(5) {
                let a = g.rng.below(n_ind);
                match g.rng.below(8) {
                    0..=3 => {
                        let c = g.concept(1);
                        let _ = writeln!(out, ":i{a} a {c} .");
                    }
                    4 | 5 => {
                        let b = g.rng.below(n_ind);
                        let r = g.rng.below(n_roles);
                        let _ = writeln!(out, ":i{a} :r{r} :i{b} .");
                    }
                    6 => {
                        let b = g.rng.below(n_ind);
                        if a != b {
                            let _ = writeln!(out, ":i{a} owl:differentFrom :i{b} .");
                        }
                    }
                    _ => {
                        let b = g.rng.below(n_ind);
                        let r = g.rng.below(n_roles);
                        let _ = writeln!(out, "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :i{a} ; owl:assertionProperty :r{r} ; owl:targetIndividual :i{b} ] .");
                    }
                }
            }
        }
        out
    }
}

const SEEDS: u64 = 400;

/// The answer block in the reference line format: `CONSISTENT true|false`, then sorted
/// `UNSAT <iri>` and `SUB <iri> <iri>` lines.
fn render(d: &Dict, report: &DlReport, names: &BTreeMap<Id, String>) -> String {
    let mut lines = Vec::new();
    match &report.consistency {
        Consistency::Consistent => lines.push("CONSISTENT true".to_string()),
        Consistency::Inconsistent => {
            lines.push("CONSISTENT false".to_string());
            return lines.join("\n");
        }
        Consistency::Unknown(r) => return format!("UNKNOWN {r:?}"),
    }
    let _ = d;
    let Some(c) = report.classification.as_ref() else {
        return format!("UNKNOWN {:?}", report.classification_unknown);
    };
    let mut body = Vec::new();
    for u in &c.unsatisfiable {
        body.push(format!("UNSAT {}", names[u]));
    }
    for (a, b) in &c.subsumptions {
        body.push(format!("SUB {} {}", names[a], names[b]));
    }
    body.sort();
    lines.extend(body);
    lines.join("\n")
}

fn expected(text: &str) -> BTreeMap<u64, String> {
    let mut out = BTreeMap::new();
    let mut cur: Option<(u64, Vec<&str>)> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("FILE ") {
            if let Some((s, ls)) = cur.take() {
                out.insert(s, ls.join("\n"));
            }
            let stem = rest.rsplit('/').next().unwrap().trim_end_matches(".ttl");
            cur = Some((stem.trim_start_matches("seed").parse().unwrap(), Vec::new()));
        } else if let Some((_, ls)) = cur.as_mut() {
            ls.push(line);
        }
    }
    if let Some((s, ls)) = cur.take() {
        out.insert(s, ls.join("\n"));
    }
    out
}

/// Compares `seeds` generated ontologies with the reference answers in `recorded`. With
/// `OWL2DL_ORACLE_DUMP=<dir>` set, writes the corpus to `<dir>/<corpus>/` instead.
///
/// Every verdict must agree exactly. With `max_unknown_percent` set, an `Unknown` answer
/// (no consistency verdict, or no classification) is counted instead of failing, and at
/// most that share of the seeds may be unknown; without it, every seed needs a verdict.
fn differential(
    corpus: &str,
    generate: fn(u64) -> String,
    seeds: u64,
    recorded: &str,
    max_unknown_percent: Option<u64>,
) {
    if let Ok(dir) = std::env::var("OWL2DL_ORACLE_DUMP") {
        let dir = format!("{dir}/{corpus}");
        std::fs::create_dir_all(&dir).unwrap();
        for seed in 0..seeds {
            let ttl = format!("{PRE}{}", generate(seed));
            std::fs::write(format!("{dir}/seed{seed:04}.ttl"), ttl).unwrap();
        }
        return;
    }
    let expected = expected(recorded);
    let mut failures = Vec::new();
    let mut skipped = 0;
    let mut unknown = 0;
    for seed in 0..seeds {
        let body = generate(seed);
        let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
        let o = extract(&d, &t).unwrap_or_else(|e| panic!("seed {seed}: extract: {e}"));
        let report = check(&d, &o, Budget::default());
        let mut names = BTreeMap::new();
        for i in 0..8 {
            let id = ex(&d, &format!("C{i}"));
            if id != sparq_core::dict::NO_ID {
                names.insert(id, format!("http://ex/C{i}"));
            }
        }
        let got = render(&d, &report, &names);
        let want = expected
            .get(&seed)
            .unwrap_or_else(|| panic!("no recorded answer for seed {seed}"));
        // The reference itself failed on this seed (recorded as an `ERROR` line).
        if want.starts_with("ERROR") {
            skipped += 1;
            continue;
        }
        if max_unknown_percent.is_some() && got.starts_with("UNKNOWN") {
            unknown += 1;
            continue;
        }
        if &got != want {
            failures.push(format!(
                "seed {seed}:\n--- want\n{want}\n--- got\n{got}\n{body}"
            ));
            continue;
        }
        // The single-query API must agree with the recorded hierarchy too.
        if want.starts_with("CONSISTENT true") {
            let r = Reasoner::new(&d, &o, Budget::default()).expect("decided above");
            let lines: std::collections::BTreeSet<&str> = want.lines().collect();
            let unsat = |c: &str| lines.contains(format!("UNSAT {c}").as_str());
            for a in names.values() {
                if r.is_satisfiable(a) != Some(!unsat(a)) {
                    failures.push(format!("seed {seed}: is_satisfiable({a})\n{body}"));
                }
                for b in names.values() {
                    let entailed =
                        a == b || unsat(a) || lines.contains(format!("SUB {a} {b}").as_str());
                    if r.is_subclass_of(a, b) != Some(entailed) {
                        failures.push(format!("seed {seed}: is_subclass_of({a}, {b})\n{body}"));
                    }
                }
            }
        }
    }
    assert!(
        skipped * 50 < seeds,
        "the reference failed on {skipped} seeds"
    );
    eprintln!(
        "{corpus}: {} of {seeds} agree, {unknown} unknown, {skipped} skipped",
        seeds - skipped - unknown - failures.len() as u64
    );
    if let Some(pct) = max_unknown_percent {
        assert!(
            unknown * 100 <= seeds * pct,
            "{unknown} of {seeds} seeds answered unknown"
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {seeds} seeds disagree:\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn oracle_differential() {
    differential(
        "shiq",
        Gen::ontology,
        SEEDS,
        include_str!("data/owl2dl_oracle_expected.txt"),
        None,
    );
}

/// The SHIQ corpus with nominals: class expressions may also be `ObjectOneOf` and
/// `ObjectHasValue` over a few individuals, so class tests depend on the ABox. Up to a
/// tenth of the seeds may answer Unknown (an unconfirmed model, or the budget).
#[test]
fn oracle_differential_nominals() {
    differential(
        "nominals",
        |seed| Gen::ontology_with(seed, true),
        NOMINAL_SEEDS,
        include_str!("data/owl2dl_oracle_nominals_expected.txt"),
        Some(10),
    );
}

const NOMINAL_SEEDS: u64 = 400;

/// The SHIQ corpus with keys: one to three `HasKey` axioms over object properties and a
/// data property `:d0`, and an ABox of a few named individuals with role assertions,
/// small-integer data assertions and the occasional `differentFrom`, so keys fire. Up to a
/// tenth of the seeds may answer Unknown (values a key compares that are not fixed). Key
/// object properties are simple: a key on a role with a transitive sub-role is refused.
///
/// Key properties are named: the reference reasoner ignores an `ObjectInverseOf` in a key
/// (it finds `HasKey(⊤ (r⁻))` with `r(c, a)`, `r(c, b)` and `a ≠ b` consistent, but not the
/// same key over a property declared `owl:inverseOf r`); the hand-written tests cover it.
fn keys_corpus(seed: u64) -> String {
    let mut out = Gen::ontology(seed);
    let count = |what: &str| {
        (0..)
            .take_while(|i| out.contains(&format!(":{what}{i} a ")))
            .count()
    };
    let (n_classes, n_roles) = (count("C"), count("r"));
    // Keys are on simple roles only (a key on a role with a transitive sub-role is refused):
    // a role is non-simple when it is transitive or above a non-simple one.
    let mut simple: Vec<bool> = (0..n_roles)
        .map(|i| !out.contains(&format!(":r{i} a owl:TransitiveProperty .")))
        .collect();
    for _ in 0..n_roles {
        for a in 0..n_roles {
            for b in 0..n_roles {
                let below = out.contains(&format!(":r{a} rdfs:subPropertyOf :r{b} ."))
                    || out.contains(&format!(
                        "[ owl:inverseOf :r{a} ] rdfs:subPropertyOf [ owl:inverseOf :r{b} ] ."
                    ));
                if below && !simple[a] {
                    simple[b] = false;
                }
            }
        }
    }
    let simple: Vec<usize> = (0..n_roles).filter(|&i| simple[i]).collect();
    let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
    let _ = writeln!(out, ":d0 a owl:DatatypeProperty .");
    let n_ind = 2 + rng.below(4);
    for i in 0..n_ind {
        let _ = writeln!(out, ":k{i} a owl:NamedIndividual .");
    }
    for _ in 0..(1 + rng.below(3)) {
        let class = if rng.chance(20) {
            "owl:Thing".to_string()
        } else {
            format!(":C{}", rng.below(n_classes))
        };
        let mut props: Vec<String> = Vec::new();
        for _ in 0..(1 + rng.below(2)) {
            let r = rng.below(n_roles);
            if !simple.is_empty() {
                props.push(format!(":r{}", simple[r % simple.len()]));
            }
        }
        if props.is_empty() || rng.chance(35) {
            props.push(":d0".into());
        }
        props.dedup();
        let _ = writeln!(out, "{class} owl:hasKey ( {} ) .", props.join(" "));
    }
    for _ in 0..(3 + rng.below(8)) {
        let a = rng.below(n_ind);
        match rng.below(10) {
            0..=3 => {
                let (r, b) = (rng.below(n_roles), rng.below(n_ind));
                let _ = writeln!(out, ":k{a} :r{r} :k{b} .");
            }
            4..=6 => {
                let c = rng.below(n_classes);
                let _ = writeln!(out, ":k{a} a :C{c} .");
            }
            7 | 8 => {
                let v = 1 + rng.below(3);
                let _ = writeln!(out, ":k{a} :d0 {v} .");
            }
            _ => {
                let b = rng.below(n_ind);
                if a != b {
                    let _ = writeln!(out, ":k{a} owl:differentFrom :k{b} .");
                }
            }
        }
    }
    out
}

#[test]
fn oracle_differential_keys() {
    differential(
        "keys",
        keys_corpus,
        KEY_SEEDS,
        include_str!("data/owl2dl_oracle_keys_expected.txt"),
        Some(10),
    );
}

const KEY_SEEDS: u64 = 400;

/// A second corpus aimed at the constructs earlier reviews found answers wrong for:
/// negative property assertions over transitive roles and role hierarchies, `owl:Thing` and
/// `owl:Nothing` in axioms, and declared classes no axiom mentions. Each ontology has an
/// ABox of chained role assertions so transitive paths actually form.
fn focused(seed: u64) -> String {
    let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
    let n_classes = 3 + rng.below(5);
    let n_roles = 1 + rng.below(3);
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/f{seed}> a owl:Ontology .");
    for i in 0..n_classes {
        let _ = writeln!(out, ":C{i} a owl:Class .");
    }
    for i in 0..n_roles {
        let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
    }
    let mut transitive = vec![false; n_roles];
    for (i, t) in transitive.iter_mut().enumerate() {
        if rng.chance(60) {
            *t = true;
            let _ = writeln!(out, ":r{i} a owl:TransitiveProperty .");
        }
    }
    let mut incl = Vec::new();
    if n_roles > 1 && rng.chance(50) {
        let (a, b) = (rng.below(n_roles), rng.below(n_roles));
        if a != b {
            incl.push((a, b));
            let _ = writeln!(out, ":r{a} rdfs:subPropertyOf :r{b} .");
        }
    }
    let mut simple: Vec<bool> = transitive.iter().map(|t| !t).collect();
    for &(a, b) in &incl {
        if !simple[a] {
            simple[b] = false;
        }
    }
    // Only the first few classes appear in axioms; the rest are declared and unused.
    let used = 2 + rng.below(n_classes - 1);
    let mut g = Gen {
        rng,
        n_classes: used,
        n_roles,
        simple,
        nominals: 0,
    };
    for _ in 0..(1 + g.rng.below(4)) {
        let line = match g.rng.below(9) {
            0 => format!("owl:Thing rdfs:subClassOf {} .", g.concept(1)),
            1 => format!("{} rdfs:subClassOf owl:Nothing .", g.concept(1)),
            2 => format!(
                "{} owl:equivalentClass [ a owl:Class ; owl:unionOf ( {} owl:Nothing ) ] .",
                g.class(),
                g.concept(1)
            ),
            3 => format!("{} owl:disjointWith owl:Thing .", g.class()),
            _ => {
                let (a, b) = (g.class(), g.concept(2));
                format!("{a} rdfs:subClassOf {b} .")
            }
        };
        let _ = writeln!(out, "{line}");
    }
    let n_ind = 2 + g.rng.below(4);
    for i in 0..n_ind {
        let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
    }
    // A chain i0 r i1 r i2 ..., with some shortcuts and class assertions.
    for i in 0..n_ind - 1 {
        let r = g.rng.below(n_roles);
        let _ = writeln!(out, ":i{i} :r{r} :i{} .", i + 1);
    }
    for _ in 0..g.rng.below(3) {
        let a = g.rng.below(n_ind);
        let c = g.concept(1);
        let _ = writeln!(out, ":i{a} a {c} .");
    }
    for _ in 0..(1 + g.rng.below(3)) {
        // Mostly forward along the chain, where a transitive path may entail the pair.
        let a = g.rng.below(n_ind - 1);
        let b = if g.rng.chance(75) {
            a + 1 + g.rng.below(n_ind - 1 - a)
        } else {
            g.rng.below(n_ind)
        };
        let r = g.rng.below(n_roles);
        let _ = writeln!(out, "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :i{a} ; owl:assertionProperty :r{r} ; owl:targetIndividual :i{b} ] .");
    }
    out
}

const FOCUSED_SEEDS: u64 = 200;

#[test]
fn oracle_differential_focused() {
    differential(
        "focused",
        focused,
        FOCUSED_SEEDS,
        include_str!("data/owl2dl_oracle_focused_expected.txt"),
        None,
    );
}

/// A corpus of regular property chains: roles `r0 < r1 < …` with sub-property axioms and
/// chains `S₁ ∘ … ∘ Sₙ ⊑ R`, `R ∘ S ⊑ R`, `S ∘ R ⊑ R` and transitivity whose `Sᵢ` (or their
/// inverses) are below `R` in that order, so the hierarchy is regular. The TBox has
/// universal restrictions over the chained roles, and the ABox a path of role assertions,
/// so chains actually compose, plus negative property assertions along it. Every role is
/// non-simple for the generator: no number restriction or `hasSelf`.
fn chains_corpus(seed: u64) -> String {
    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
    let n_classes = 3 + rng.below(4);
    let n_roles = 2 + rng.below(3);
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/ch{seed}> a owl:Ontology .");
    for i in 0..n_classes {
        let _ = writeln!(out, ":C{i} a owl:Class .");
    }
    for i in 0..n_roles {
        let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
    }
    let below = |rng: &mut Rng, j: usize| {
        let i = rng.below(j);
        if rng.chance(25) {
            format!("[ owl:inverseOf :r{i} ]")
        } else {
            format!(":r{i}")
        }
    };
    for _ in 0..(1 + rng.below(4)) {
        let j = 1 + rng.below(n_roles - 1);
        let line = match rng.below(6) {
            0 => format!(":r{} rdfs:subPropertyOf :r{j} .", rng.below(j)),
            1 => format!(":r{j} a owl:TransitiveProperty ."),
            2 => format!(
                ":r{j} owl:propertyChainAxiom ( :r{j} {} ) .",
                below(&mut rng, j)
            ),
            3 => format!(
                ":r{j} owl:propertyChainAxiom ( {} :r{j} ) .",
                below(&mut rng, j)
            ),
            4 => format!(
                ":r{j} owl:propertyChainAxiom ( {} {} {} ) .",
                below(&mut rng, j),
                below(&mut rng, j),
                below(&mut rng, j)
            ),
            _ => format!(
                ":r{j} owl:propertyChainAxiom ( {} {} ) .",
                below(&mut rng, j),
                below(&mut rng, j)
            ),
        };
        let _ = writeln!(out, "{line}");
    }
    let mut g = Gen {
        rng,
        n_classes,
        n_roles,
        simple: vec![false; n_roles],
        nominals: 0,
    };
    for _ in 0..(1 + g.rng.below(3)) {
        let (a, r, c) = (g.class(), g.role(false).unwrap(), g.class());
        let _ = writeln!(
            out,
            "{a} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {c} ] ."
        );
    }
    for _ in 0..g.rng.below(3) {
        let (a, b) = (g.class(), g.concept(2));
        let _ = writeln!(out, "{a} rdfs:subClassOf {b} .");
    }
    let n_ind = 3 + g.rng.below(4);
    for i in 0..n_ind {
        let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
    }
    for i in 0..n_ind - 1 {
        let r = g.rng.below(n_roles);
        if g.rng.chance(25) {
            let _ = writeln!(out, ":i{} :r{r} :i{i} .", i + 1);
        } else {
            let _ = writeln!(out, ":i{i} :r{r} :i{} .", i + 1);
        }
    }
    for _ in 0..(1 + g.rng.below(3)) {
        let a = g.rng.below(n_ind);
        let c = g.concept(1);
        let _ = writeln!(out, ":i{a} a {c} .");
    }
    for _ in 0..g.rng.below(3) {
        let a = g.rng.below(n_ind - 1);
        let b = a + 1 + g.rng.below(n_ind - 1 - a);
        let r = g.rng.below(n_roles);
        let _ = writeln!(out, "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :i{a} ; owl:assertionProperty :r{r} ; owl:targetIndividual :i{b} ] .");
    }
    out
}

const CHAIN_SEEDS: u64 = 400;

#[test]
fn oracle_differential_chains() {
    differential(
        "chains",
        chains_corpus,
        CHAIN_SEEDS,
        include_str!("data/owl2dl_oracle_chains_expected.txt"),
        None,
    );
}

/// A corpus of the built-in properties: `owl:topObjectProperty` and
/// `owl:bottomObjectProperty` in existential, universal and `hasValue` restrictions (also
/// negated and nested), as super-properties of properties and chains, in domain and range
/// axioms and in positive and negative assertions, mixed with ordinary restrictions over a
/// small role hierarchy and an ABox.
fn builtins_corpus(seed: u64) -> String {
    let mut rng = Rng(seed.wrapping_mul(0x9FB2_1C65_1E98_DF25) | 1);
    let n_classes = 3 + rng.below(3);
    let n_roles = 2 + rng.below(2);
    let n_ind = 1 + rng.below(4);
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/bi{seed}> a owl:Ontology .");
    for i in 0..n_classes {
        let _ = writeln!(out, ":C{i} a owl:Class .");
    }
    for i in 0..n_roles {
        let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
    }
    for i in 0..n_ind {
        let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
    }
    fn concept(
        rng: &mut Rng,
        n_classes: usize,
        n_roles: usize,
        n_ind: usize,
        depth: u32,
    ) -> String {
        let leaf = |rng: &mut Rng| format!(":C{}", rng.below(n_classes));
        if depth == 0 || rng.chance(30) {
            return leaf(rng);
        }
        let sub = |rng: &mut Rng| concept(rng, n_classes, n_roles, n_ind, depth - 1);
        let prop = |rng: &mut Rng| match rng.below(5) {
            0 | 1 => "owl:topObjectProperty".to_string(),
            2 => "owl:bottomObjectProperty".to_string(),
            _ => format!(":r{}", rng.below(n_roles)),
        };
        match rng.below(7) {
            0 => format!("[ owl:complementOf {} ]", sub(rng)),
            1 => format!(
                "[ a owl:Class ; owl:intersectionOf ( {} {} ) ]",
                sub(rng),
                sub(rng)
            ),
            2 => format!(
                "[ a owl:Class ; owl:unionOf ( {} {} ) ]",
                sub(rng),
                sub(rng)
            ),
            3 | 4 => {
                let p = prop(rng);
                format!(
                    "[ a owl:Restriction ; owl:onProperty {p} ; owl:someValuesFrom {} ]",
                    sub(rng)
                )
            }
            5 => {
                let p = prop(rng);
                format!(
                    "[ a owl:Restriction ; owl:onProperty {p} ; owl:allValuesFrom {} ]",
                    sub(rng)
                )
            }
            _ => {
                let p = prop(rng);
                format!(
                    "[ a owl:Restriction ; owl:onProperty {p} ; owl:hasValue :i{} ]",
                    rng.below(n_ind)
                )
            }
        }
    }
    for _ in 0..(1 + rng.below(2)) {
        let line = match rng.below(6) {
            0 => format!(
                ":r{} rdfs:subPropertyOf :r{} .",
                rng.below(n_roles),
                rng.below(n_roles)
            ),
            1 => format!(
                ":r{} rdfs:subPropertyOf owl:topObjectProperty .",
                rng.below(n_roles)
            ),
            2 => format!(
                ":r{} rdfs:subPropertyOf owl:bottomObjectProperty .",
                rng.below(n_roles)
            ),
            3 => format!(
                "owl:bottomObjectProperty owl:propertyChainAxiom ( :r{} :r{} ) .",
                rng.below(n_roles),
                rng.below(n_roles)
            ),
            4 => format!(
                "owl:topObjectProperty rdfs:domain {} .",
                concept(&mut rng, n_classes, n_roles, n_ind, 1)
            ),
            _ => format!(":r{} a owl:TransitiveProperty .", rng.below(n_roles)),
        };
        let _ = writeln!(out, "{line}");
    }
    for _ in 0..(2 + rng.below(4)) {
        let (a, b) = (
            concept(&mut rng, n_classes, n_roles, n_ind, 2),
            concept(&mut rng, n_classes, n_roles, n_ind, 2),
        );
        let _ = writeln!(out, "{a} rdfs:subClassOf {b} .");
    }
    for _ in 0..rng.below(3) {
        let a = rng.below(n_ind);
        let c = concept(&mut rng, n_classes, n_roles, n_ind, 1);
        let _ = writeln!(out, ":i{a} a {c} .");
    }
    for _ in 0..rng.below(3) {
        let (a, b) = (rng.below(n_ind), rng.below(n_ind));
        let p = match rng.below(8) {
            0 => "owl:topObjectProperty".to_string(),
            1 => "owl:bottomObjectProperty".to_string(),
            _ => format!(":r{}", rng.below(n_roles)),
        };
        if rng.chance(25) {
            let _ = writeln!(out, "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :i{a} ; owl:assertionProperty {p} ; owl:targetIndividual :i{b} ] .");
        } else {
            let _ = writeln!(out, ":i{a} {p} :i{b} .");
        }
    }
    out
}

const BUILTIN_SEEDS: u64 = 400;

#[test]
fn oracle_differential_builtins() {
    differential(
        "builtins",
        builtins_corpus,
        BUILTIN_SEEDS,
        include_str!("data/owl2dl_oracle_builtins_expected.txt"),
        Some(5),
    );
}

/// A corpus of data properties and datatypes: data property axioms, data restrictions
/// over integer, decimal, string and boolean ranges (facet restrictions, `DataOneOf`, the
/// data booleans, a `DatatypeDefinition`), and data assertions whose literals denote the
/// same value in different datatypes and lexical forms.
///
/// Left out on purpose, where the reference reasoner departs from the OWL 2 Direct
/// Semantics: ill-typed literals (it refuses the ontology instead of answering
/// inconsistent), facet bounds that make a single-value interval at zero (see `range`), a
/// `DataUnionOf` or `DataIntersectionOf` in the filler of a data cardinality restriction
/// (it answers wrongly when the boolean is empty or a single value: with
/// `SubClassOf(C1 DataMaxCardinality(1 d (E1 ∪ E2)))` for empty facet restrictions `E1`,
/// `E2` and `SubClassOf(DataMaxCardinality(2 d xsd:int) C2)` it entails `C1 ⊑ C2`, though
/// each `Eᵢ` alone does not; it finds `ClassAssertion(DataExactCardinality(1 d
/// (xsd:nonNegativeInteger ∪ xsd:integer)) a)` consistent with `d(a, -1)` and `d(a, 4)`;
/// and with a `DataPropertyDomain` it finds `EquivalentClasses(C
/// ¬DataExactCardinality(1 d ({2} ∩ xsd:integer)))` inconsistent with one assertion),
/// `xsd:float` / `xsd:double` zeros (it treats `"0"` and `"-0"` as one
/// value, while OWL 2 §4.2 makes them distinct), and `NegativeDataPropertyAssertion` (it
/// compares the target literal syntactically, so `"2"^^xsd:nonNegativeInteger` does not
/// exclude the value `2`; the corpus uses the equivalent class assertion
/// `∀D.¬{v}(a)`, which it compares by value). `xsd:pattern` is left out too: its
/// regular-expression dialect differs between implementations. All four are covered by
/// the hand-written tests above.
fn datatypes_corpus(seed: u64) -> String {
    struct D {
        g: Gen,
        n_data: usize,
        defined: bool,
    }

    impl D {
        fn data(&mut self) -> String {
            format!(":d{}", self.g.rng.below(self.n_data))
        }

        fn small(&mut self) -> i64 {
            self.g.rng.below(9) as i64 - 2
        }

        fn literal(&mut self) -> String {
            let n = self.small();
            match self.g.rng.below(11) {
                0 => format!("\"{n}\"^^xsd:integer"),
                1 => format!("\"{n}\"^^xsd:int"),
                2 if n >= 0 => format!("\"0{n}\"^^xsd:integer"),
                3 if n >= 0 => format!("\"{n}\"^^xsd:nonNegativeInteger"),
                4 => format!("\"{n}.0\"^^xsd:decimal"),
                5 => format!("\"{n}.5\"^^xsd:decimal"),
                6 => ["\"a\"", "\"b\"", "\"ab\""][self.g.rng.below(3)].to_string(),
                7 => ["\"a\"^^xsd:string", "\"\"^^xsd:string"][self.g.rng.below(2)].to_string(),
                8 => [
                    "\"true\"^^xsd:boolean",
                    "\"false\"^^xsd:boolean",
                    "\"1\"^^xsd:boolean",
                ][self.g.rng.below(3)]
                .to_string(),
                _ => format!("{n}"),
            }
        }

        /// A data range; `in_card` keeps `DataUnionOf` and `DataIntersectionOf` (and the
        /// defined datatype, which may be one) out of the filler of a data cardinality
        /// restriction.
        fn range(&mut self, depth: usize, in_card: bool) -> String {
            let pick = if depth == 0 {
                self.g.rng.below(9)
            } else {
                self.g.rng.below(13)
            };
            let pick = if in_card && matches!(pick, 7 | 9 | 10) {
                3
            } else {
                pick
            };
            match pick {
                0 => "xsd:integer".into(),
                1 => "xsd:nonNegativeInteger".into(),
                2 => [
                    "xsd:string",
                    "xsd:boolean",
                    "xsd:decimal",
                    "xsd:int",
                    "rdfs:Literal",
                ][self.g.rng.below(5)]
                .into(),
                3 | 4 => {
                    let base = [
                        "xsd:integer",
                        "xsd:int",
                        "xsd:nonNegativeInteger",
                        "xsd:decimal",
                    ][self.g.rng.below(4)];
                    // Bounds stay in the restricted datatype's value space (a bound outside it
                    // is outside the facet space, which the oracle answers Unknown for), and
                    // away from -1, 0 and 1: the reference reasoner gets an interval ending
                    // at zero wrong when it is a single value (it does not find
                    // `integer[0, 0] ⊑ integer[0, 5]`, though it finds `[3, 3] ⊑ [0, 5]`).
                    let bound = |d: &mut D| {
                        let n: i64 = [-3, -2, 2, 3, 4, 5, 6][d.g.rng.below(7)];
                        if base == "xsd:nonNegativeInteger" {
                            n.abs()
                        } else {
                            n
                        }
                    };
                    let mut facets = Vec::new();
                    for f in [
                        "xsd:minInclusive",
                        "xsd:maxInclusive",
                        "xsd:minExclusive",
                        "xsd:maxExclusive",
                    ] {
                        if self.g.rng.chance(35) {
                            let n = bound(self);
                            facets.push(format!("[ {f} \"{n}\"^^xsd:integer ]"));
                        }
                    }
                    if facets.is_empty() {
                        let n = bound(self);
                        facets.push(format!("[ xsd:maxInclusive \"{n}\"^^xsd:integer ]"));
                    }
                    format!(
                        "[ a rdfs:Datatype ; owl:onDatatype {base} ; owl:withRestrictions ( {} ) ]",
                        facets.join(" ")
                    )
                }
                5 => {
                    let n = 1 + self.g.rng.below(3);
                    let lits: Vec<String> = (0..n).map(|_| self.literal()).collect();
                    format!("[ a rdfs:Datatype ; owl:oneOf ( {} ) ]", lits.join(" "))
                }
                6 => {
                    let (lo, hi) = (self.g.rng.below(3), self.g.rng.below(3));
                    format!(
                        "[ a rdfs:Datatype ; owl:onDatatype xsd:string ; owl:withRestrictions ( [ xsd:minLength \"{lo}\"^^xsd:integer ] [ xsd:maxLength \"{hi}\"^^xsd:integer ] ) ]"
                    )
                }
                7 if self.defined => ":DT0".into(),
                7 | 8 => "xsd:integer".into(),
                9 | 10 => {
                    let kw = if pick == 9 {
                        "owl:unionOf"
                    } else {
                        "owl:intersectionOf"
                    };
                    let (a, b) = (
                        self.range(depth - 1, in_card),
                        self.range(depth - 1, in_card),
                    );
                    // A data union or intersection needs two distinct members.
                    if a == b {
                        return a;
                    }
                    format!("[ a rdfs:Datatype ; {kw} ( {a} {b} ) ]")
                }
                _ => format!(
                    "[ a rdfs:Datatype ; owl:datatypeComplementOf {} ]",
                    self.range(depth - 1, in_card)
                ),
            }
        }

        fn restriction(&mut self) -> String {
            let d = self.data();
            let card = |n: usize| format!("\"{n}\"^^xsd:nonNegativeInteger");
            // Cardinalities are weighted up: they exercise the datatype check most.
            match [0, 1, 2, 3, 4, 4, 4, 5, 5, 5, 6, 6, 7][self.g.rng.below(13)] {
                0 | 1 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:someValuesFrom {} ]",
                    self.range(1, false)
                ),
                2 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:allValuesFrom {} ]",
                    self.range(1, false)
                ),
                3 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:hasValue {} ]",
                    self.literal()
                ),
                4 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:minQualifiedCardinality {} ; owl:onDataRange {} ]",
                    card(1 + self.g.rng.below(3)),
                    self.range(1, true)
                ),
                5 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:maxQualifiedCardinality {} ; owl:onDataRange {} ]",
                    card(self.g.rng.below(3)),
                    self.range(1, true)
                ),
                6 => format!(
                    "[ a owl:Restriction ; owl:onProperty {d} ; owl:qualifiedCardinality {} ; owl:onDataRange {} ]",
                    card(1 + self.g.rng.below(2)),
                    self.range(1, true)
                ),
                _ => {
                    let kw = ["owl:minCardinality", "owl:maxCardinality", "owl:cardinality"]
                        [self.g.rng.below(3)];
                    format!(
                        "[ a owl:Restriction ; owl:onProperty {d} ; {kw} {} ]",
                        card(self.g.rng.below(3))
                    )
                }
            }
        }

        /// A class expression mixing classes, booleans, an object role and data
        /// restrictions.
        fn concept(&mut self, depth: usize) -> String {
            if depth == 0 || self.g.rng.chance(25) {
                return if self.g.rng.chance(60) {
                    self.restriction()
                } else if self.g.rng.chance(15) {
                    format!("[ a owl:Class ; owl:complementOf {} ]", self.g.class())
                } else {
                    self.g.class()
                };
            }
            let d = depth - 1;
            match self.g.rng.below(6) {
                0 => format!(
                    "[ a owl:Class ; owl:intersectionOf ( {} {} ) ]",
                    self.concept(d),
                    self.concept(d)
                ),
                1 => format!(
                    "[ a owl:Class ; owl:unionOf ( {} {} ) ]",
                    self.concept(d),
                    self.concept(d)
                ),
                2 => format!("[ a owl:Class ; owl:complementOf {} ]", self.concept(d)),
                3 => format!(
                    "[ a owl:Restriction ; owl:onProperty :r0 ; owl:someValuesFrom {} ]",
                    self.concept(d)
                ),
                4 => format!(
                    "[ a owl:Restriction ; owl:onProperty :r0 ; owl:allValuesFrom {} ]",
                    self.concept(d)
                ),
                _ => self.restriction(),
            }
        }
    }

    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
    let n_classes = 3 + rng.below(4);
    let n_data = 2 + rng.below(3);
    let defined = rng.chance(20);
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/dt{seed}> a owl:Ontology .");
    for i in 0..n_classes {
        let _ = writeln!(out, ":C{i} a owl:Class .");
    }
    let _ = writeln!(out, ":r0 a owl:ObjectProperty .");
    for i in 0..n_data {
        let _ = writeln!(out, ":d{i} a owl:DatatypeProperty .");
    }
    let g = Gen {
        rng,
        n_classes,
        n_roles: 1,
        simple: vec![true],
        nominals: 0,
    };
    let mut d = D {
        g,
        n_data,
        defined: false,
    };
    if defined {
        let r = d.range(1, false);
        let _ = writeln!(out, ":DT0 a rdfs:Datatype ; owl:equivalentClass {r} .");
        d.defined = true;
    }
    for i in 0..n_data {
        if d.g.rng.chance(35) {
            let _ = writeln!(out, ":d{i} a owl:FunctionalProperty .");
        }
        if d.g.rng.chance(15) {
            let c = d.concept(1);
            let _ = writeln!(out, ":d{i} rdfs:domain {c} .");
        }
        if d.g.rng.chance(30) {
            let r = d.range(1, false);
            let _ = writeln!(out, ":d{i} rdfs:range {r} .");
        }
    }
    for _ in 0..d.g.rng.below(5) {
        let (a, b) = (d.g.rng.below(n_data), d.g.rng.below(n_data));
        if a == b {
            continue;
        }
        let p = [
            "rdfs:subPropertyOf",
            "owl:equivalentProperty",
            "owl:propertyDisjointWith",
        ][d.g.rng.below(3)];
        let _ = writeln!(out, ":d{a} {p} :d{b} .");
    }
    for _ in 0..(2 + d.g.rng.below(5)) {
        match d.g.rng.below(10) {
            0..=5 => {
                let (a, b) = if d.g.rng.chance(50) {
                    (d.g.class(), d.concept(2))
                } else {
                    (d.concept(2), d.g.class())
                };
                let _ = writeln!(out, "{a} rdfs:subClassOf {b} .");
            }
            6 | 7 => {
                let (a, b) = (d.g.class(), d.concept(2));
                let _ = writeln!(out, "{a} owl:equivalentClass {b} .");
            }
            _ => {
                let (a, b) = (d.g.class(), d.g.class());
                if a != b {
                    let _ = writeln!(out, "{a} owl:disjointWith {b} .");
                }
            }
        }
    }
    let n_ind = 1 + d.g.rng.below(3);
    for i in 0..n_ind {
        let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
    }
    if n_ind > 0 {
        for _ in 0..d.g.rng.below(9) {
            let a = d.g.rng.below(n_ind);
            match d.g.rng.below(10) {
                0..=2 => {
                    let c = d.concept(1);
                    let _ = writeln!(out, ":i{a} a {c} .");
                }
                3..=6 => {
                    let (p, l) = (d.data(), d.literal());
                    let _ = writeln!(out, ":i{a} {p} {l} .");
                }
                7 => {
                    let b = d.g.rng.below(n_ind);
                    let _ = writeln!(out, ":i{a} :r0 :i{b} .");
                }
                _ => {
                    // `NegativeDataPropertyAssertion(p a l)`, in its class-assertion form.
                    let (p, l) = (d.data(), d.literal());
                    let _ = writeln!(out, ":i{a} a [ a owl:Restriction ; owl:onProperty {p} ; owl:allValuesFrom [ a rdfs:Datatype ; owl:datatypeComplementOf [ a rdfs:Datatype ; owl:oneOf ( {l} ) ] ] ] .");
                }
            }
        }
    }
    out
}

const DATATYPE_SEEDS: u64 = 300;

#[test]
fn oracle_differential_datatypes() {
    differential(
        "datatypes",
        datatypes_corpus,
        DATATYPE_SEEDS,
        include_str!("data/owl2dl_oracle_datatypes_expected.txt"),
        Some(10),
    );
}

// ---- incremental rechecks ------------------------------------------------------------------

/// Rechecking with [`IncrementalChecker`] answers what a fresh [`check`] does: over every
/// corpus, each seed's ontology and its first half are checked in the order half, whole,
/// whole, half, whole, each read into its own dictionary, so the checker reuses an
/// extension, an identical ontology, nothing (axioms removed), and an extension again. A
/// reused answer may be decided where the fresh check ran out of budget (it skips tests),
/// but never differs from a decided fresh answer.
#[test]
fn incremental_rechecks_agree_with_fresh_checks() {
    use sparq_reason_dl::owl2dl::{IncrementalChecker, Reuse};
    type Corpus = (&'static str, fn(u64) -> String);
    let corpora: [Corpus; 6] = [
        ("shiq", Gen::ontology),
        ("nominals", |s| Gen::ontology_with(s, true)),
        ("focused", focused),
        ("keys", keys_corpus),
        ("chains", chains_corpus),
        ("datatypes", datatypes_corpus),
    ];
    let decided = |r: &DlReport| match r.consistency {
        Consistency::Inconsistent => true,
        Consistency::Consistent => r.classification.is_some(),
        Consistency::Unknown(_) => false,
    };
    let mut failures = Vec::new();
    let mut reused = [0usize; 2];
    for (corpus, generate) in corpora {
        for seed in 0..100 {
            let whole = generate(seed);
            let lines: Vec<&str> = whole.lines().collect();
            let half = lines[..lines.len() / 2].join("\n");
            let mut checker = IncrementalChecker::new();
            let mut kept: Option<bool> = None;
            for (step, (body, is_whole)) in [
                (&half, false),
                (&whole, true),
                (&whole, true),
                (&half, false),
                (&whole, true),
            ]
            .into_iter()
            .enumerate()
            {
                let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
                let o = extract(&d, &t).unwrap();
                let fresh = check(&d, &o, Budget::default());
                let (got, reuse) = checker.check(&d, &o, Budget::default());
                // Against the last decided version (an undecided one keeps it).
                let want_reuse = match kept {
                    Some(was_whole) if was_whole == is_whole => Reuse::Identical,
                    Some(false) if is_whole => Reuse::Extension,
                    _ => Reuse::None,
                };
                // A refusal is never reused; otherwise the axioms decide.
                let refused =
                    matches!(fresh.consistency, Consistency::Unknown(_)) && reuse == Reuse::None;
                if reuse != want_reuse && !refused {
                    failures.push(format!(
                        "{corpus} {seed} step {step}: {reuse:?}, want {want_reuse:?}"
                    ));
                }
                match reuse {
                    Reuse::Identical => reused[0] += 1,
                    Reuse::Extension => reused[1] += 1,
                    Reuse::None => {}
                }
                let agrees = if decided(&fresh) {
                    got == fresh
                } else {
                    got.profile == fresh.profile && (got == fresh || decided(&got))
                };
                if !agrees {
                    failures.push(format!(
                        "{corpus} {seed} step {step} ({reuse:?}):\n--- fresh\n{fresh:?}\n--- got\n{got:?}\n{body}"
                    ));
                }
                if decided(&got) {
                    kept = Some(is_whole);
                }
            }
        }
    }
    eprintln!(
        "incremental: {} identical, {} extension reuses",
        reused[0], reused[1]
    );
    assert!(
        reused[0] > 300 && reused[1] > 300,
        "too little reuse: {reused:?}"
    );
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// An automaton that grows past `MAX_AUTOMATON_STATES` by embedding non-simple sub-roles
/// (each `r{i+1}` has two sub-roles that are both above `r{i}`, so its automaton doubles)
/// is refused quickly, not built.
#[test]
fn an_automaton_past_the_state_cap_is_refused() {
    let doubling = |n: usize| {
        let mut body = String::from(":r0 a owl:ObjectProperty , owl:TransitiveProperty .\n");
        for i in 0..n {
            let j = i + 1;
            body.push_str(&format!(
            ":a{i} a owl:ObjectProperty . :b{i} a owl:ObjectProperty . :r{j} a owl:ObjectProperty .
             :r{i} rdfs:subPropertyOf :a{i} , :b{i} . :a{i} rdfs:subPropertyOf :r{j} . :b{i} rdfs:subPropertyOf :r{j} .\n"
        ));
        }
        body.push_str(&format!(
            ":C0 a owl:Class . :C1 a owl:Class . :C0 rdfs:subClassOf {} .",
            all(&format!(":r{n}"), ":C1")
        ));
        body
    };
    let body = doubling(24);
    let start = std::time::Instant::now();
    let (_, r) = run(&body);
    assert!(
        matches!(
            r.consistency,
            Consistency::Unknown(UnknownReason::OutOfFragment { .. })
        ),
        "{:?}",
        r.consistency
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    // A small instance of the same shape is decided.
    assert!(consistent(&doubling(3)));
}

/// A deep acyclic hierarchy of non-simple properties (each a sub-property of the next above
/// a transitive one) and a transitive-heavy one (thousands of transitive sub-properties of
/// one property) are refused as out of the fragment, not built past the automaton bounds.
#[test]
fn deep_and_transitive_heavy_hierarchies_are_refused() {
    let refused = |body: &str| {
        let (_, r) = run(body);
        assert!(
            matches!(
                r.consistency,
                Consistency::Unknown(UnknownReason::OutOfFragment { .. })
            ),
            "{:?}",
            r.consistency
        );
    };
    // Declared top first, so the automata are built from the top down, recursively.
    let mut deep = String::new();
    for i in (1..=400).rev() {
        deep.push_str(&format!(
            ":r{i} a owl:ObjectProperty . :r{} rdfs:subPropertyOf :r{i} .\n",
            i - 1
        ));
    }
    deep.push_str(":r0 a owl:ObjectProperty , owl:TransitiveProperty .\n");
    deep.push_str(&format!(
        ":C0 a owl:Class . :C1 a owl:Class . :C0 rdfs:subClassOf {} .",
        all(":r400", ":C1")
    ));
    refused(&deep);
    let mut heavy = String::from(":t a owl:ObjectProperty .\n");
    for i in 0..3000 {
        heavy.push_str(&format!(
            ":s{i} a owl:ObjectProperty , owl:TransitiveProperty ; rdfs:subPropertyOf :t .\n"
        ));
    }
    heavy.push_str(&format!(
        ":C0 a owl:Class . :C1 a owl:Class . :C0 rdfs:subClassOf {} .",
        all(":t", ":C1")
    ));
    refused(&heavy);
}

/// A blocked node's edge to a nominal, when its blocker has the same edge: every `C0`
/// reaches `:o` and an `S`-successor in `C0`, so the graph blocks with edges to `:o` on
/// both ends. Each case gives HermiT's answer and whether this reasoner must decide it;
/// with an at-most restriction at `:o` on the inverse role (which counts the copies the
/// unravelling would add) the answer may stay `Unconfirmed`, but is never wrong.
#[test]
fn a_blocked_node_may_reach_a_nominal_its_blocker_reaches() {
    let pre = ":C0 a owl:Class . :C1 a owl:Class . :R a owl:ObjectProperty .
        :S a owl:ObjectProperty . :T a owl:ObjectProperty . :P a owl:ObjectProperty .
        :o a owl:NamedIndividual . :a a owl:NamedIndividual , :C0 .
        :C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :S ; owl:someValuesFrom :C0 ] ,
            [ a owl:Restriction ; owl:onProperty :R ; owl:hasValue :o ] .";
    let max = |n: u32| {
        format!(
            "[ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ; owl:maxCardinality \"{n}\"^^xsd:nonNegativeInteger ]"
        )
    };
    let chain = ":P owl:propertyChainAxiom ( :R [ owl:inverseOf :R ] ) .
        :C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :P ; owl:allValuesFrom :C1 ] .";
    // At most one `R`-predecessor of `:o` in `:D`: the copies count only if they are `D`s.
    let qmax = ":D a owl:Class . :o a [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ;
        owl:maxQualifiedCardinality \"1\"^^xsd:nonNegativeInteger ; owl:onClass :D ] .";
    // At most one `Q`-predecessor, `R` below `Q`: the copies count through the super-role.
    let smax = ":Q a owl:ObjectProperty . :R rdfs:subPropertyOf :Q . :o a [ a owl:Restriction ;
        owl:onProperty [ owl:inverseOf :Q ] ; owl:maxCardinality \"1\"^^xsd:nonNegativeInteger ] .";
    // Every `R`-predecessor of `:o` is `:a`: the `C0`s are merged into the nominal.
    let only_a = ":o a [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ;
        owl:allValuesFrom [ owl:oneOf ( :a ) ] ] .";
    let cases: Vec<(String, bool, bool)> = vec![
        (String::new(), true, true),
        (format!(":o a {} . :S a owl:IrreflexiveProperty .", max(1)), false, true),
        (format!(":o a {} .", max(1)), true, true),
        // At most two `R`-predecessors of `:o`: the copies are counted, and the NI-rule
        // makes the `C0`s two individuals, an `S`-cycle that asymmetry forbids.
        (format!(":o a {} . :S a owl:AsymmetricProperty .", max(2)), false, true),
        (format!(":o a {} . :S a owl:IrreflexiveProperty .", max(2)), true, true),
        // The restriction is one disjunct of a union: the NI-rule does not apply, so the
        // answer may be unconfirmed, never consistent.
        (
            format!(
                ":D a owl:Class . :o a [ owl:complementOf :D ] , [ a owl:Class ; owl:unionOf ( {} :D ) ] . :S a owl:AsymmetricProperty .",
                max(2)
            ),
            false,
            false,
        ),
        (
            ":C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :T ; owl:hasSelf true ] ."
                .to_string(),
            true,
            true,
        ),
        // Through a chain via the nominal: every `C0` is `C1`.
        (format!("{chain} :b a :C0 , [ owl:complementOf :C1 ] ."), false, true),
        (format!("{chain} :b a [ owl:complementOf :C1 ] ."), true, true),
        (
            format!(
                "{chain} :o a [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ; owl:allValuesFrom :C0 ] ."
            ),
            true,
            true,
        ),
        (
            ":S rdfs:subPropertyOf [ owl:inverseOf :T ] . :o a [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ; owl:allValuesFrom [ a owl:Restriction ; owl:onProperty :T ; owl:someValuesFrom :C1 ] ] ."
                .to_string(),
            true,
            true,
        ),
        (format!("{qmax} :S a owl:IrreflexiveProperty ."), true, true),
        (
            format!("{qmax} :S a owl:IrreflexiveProperty . :C0 rdfs:subClassOf :D ."),
            false,
            true,
        ),
        (format!("{qmax} :C0 rdfs:subClassOf :D ."), true, true),
        (format!("{smax} :S a owl:IrreflexiveProperty ."), false, true),
        (smax.to_string(), true, true),
        (only_a.to_string(), true, true),
        (format!("{only_a} :S a owl:IrreflexiveProperty ."), false, true),
        (
            format!(
                "{only_a} :a a [ owl:complementOf [ a owl:Restriction ; owl:onProperty :S ; owl:hasSelf true ] ] ."
            ),
            false,
            true,
        ),
    ];
    for (extra, consistent, decided) in cases {
        let body = format!("{pre} {extra}");
        let got = verdict(&body);
        let want = if consistent {
            Consistency::Consistent
        } else {
            Consistency::Inconsistent
        };
        if decided {
            assert_eq!(got, want, "{extra}");
        } else {
            assert!(
                got == want || got == Consistency::Unknown(UnknownReason::Unconfirmed),
                "{extra}: {got:?}"
            );
        }
    }
    // The chain cases classify `C0 ⊑ C1`, as HermiT does.
    assert!(entails_sub(
        &format!("{pre} {chain} :b a [ owl:complementOf :C1 ] ."),
        "C0",
        "C1"
    ));
}

/// The NI-rule corpus: cycles of existentials through classes that reach an individual,
/// with at-most restrictions at the individual on the inverse roles (qualified or not,
/// through super-roles), and role characteristics, universals, disjointness and
/// assertions that decide how many elements the cycles may use.
fn ni_corpus(seed: u64) -> String {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (n_classes, n_roles, n_inds) = (2 + rng.below(3), 2 + rng.below(2), 1 + rng.below(2));
    let mut out = String::new();
    let _ = writeln!(out, "<http://ex/ni{seed}> a owl:Ontology .");
    for i in 0..n_classes {
        let _ = writeln!(out, ":C{i} a owl:Class .");
    }
    for i in 0..n_roles {
        let _ = writeln!(out, ":r{i} a owl:ObjectProperty .");
    }
    for i in 0..n_inds {
        let _ = writeln!(out, ":i{i} a owl:NamedIndividual .");
    }
    let card = |n: usize| format!("\"{n}\"^^xsd:nonNegativeInteger");
    let role = |rng: &mut Rng| {
        let r = rng.below(n_roles);
        if rng.chance(30) {
            format!("[ owl:inverseOf :r{r} ]")
        } else {
            format!(":r{r}")
        }
    };
    let inverse = |r: &str| match r.strip_prefix("[ owl:inverseOf ") {
        Some(rest) => rest.trim_end_matches(" ]").to_string(),
        None => format!("[ owl:inverseOf {r} ]"),
    };
    let filler = |rng: &mut Rng| match rng.below(4) {
        0 => "owl:Thing".to_string(),
        1 => format!("[ owl:complementOf :C{} ]", rng.below(n_classes)),
        _ => format!(":C{}", rng.below(n_classes)),
    };
    // The cycle: each class needs a successor in another (or the same) class.
    for _ in 0..1 + rng.below(3) {
        let (c, d, r) = (rng.below(n_classes), rng.below(n_classes), role(&mut rng));
        let _ = writeln!(
            out,
            ":C{c} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:someValuesFrom :C{d} ] ."
        );
    }
    // The cycle's classes reach the individuals.
    let mut links = Vec::new();
    for _ in 0..1 + rng.below(2) {
        let (c, r, i) = (rng.below(n_classes), role(&mut rng), rng.below(n_inds));
        let _ = writeln!(
            out,
            ":C{c} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:hasValue :i{i} ] ."
        );
        links.push((r, i));
    }
    // At-most restrictions at the individuals on the inverse of a link (or a super-role).
    for _ in 0..1 + rng.below(2) {
        let (r, i) = links[rng.below(links.len())].clone();
        let r = if rng.chance(25) {
            let sup = rng.below(n_roles);
            let _ = writeln!(out, "{r} rdfs:subPropertyOf :r{sup} .");
            format!(":r{sup}")
        } else {
            r
        };
        let (n, f) = (1 + rng.below(3), filler(&mut rng));
        let _ = writeln!(
            out,
            ":i{i} a [ a owl:Restriction ; owl:onProperty {} ; owl:maxQualifiedCardinality {} ; owl:onClass {f} ] .",
            inverse(&r),
            card(n)
        );
    }
    for _ in 0..rng.below(5) {
        match rng.below(9) {
            0 => {
                let r = rng.below(n_roles);
                let ch = [
                    "Asymmetric",
                    "Irreflexive",
                    "Functional",
                    "InverseFunctional",
                ][rng.below(4)];
                let _ = writeln!(out, ":r{r} a owl:{ch}Property .");
            }
            1 => {
                let (c, r, f) = (rng.below(n_classes), role(&mut rng), filler(&mut rng));
                let _ = writeln!(
                    out,
                    ":C{c} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:allValuesFrom {f} ] ."
                );
            }
            2 => {
                let (c, d) = (rng.below(n_classes), rng.below(n_classes));
                if c != d {
                    let _ = writeln!(out, ":C{c} owl:disjointWith :C{d} .");
                }
            }
            3 => {
                let (i, c) = (rng.below(n_inds), rng.below(n_classes));
                let _ = writeln!(out, ":i{i} a :C{c} .");
            }
            4 => {
                let (i, r, f, n) = (
                    rng.below(n_inds),
                    role(&mut rng),
                    filler(&mut rng),
                    rng.below(3),
                );
                let _ = writeln!(
                    out,
                    ":i{i} a [ a owl:Restriction ; owl:onProperty {r} ; owl:maxQualifiedCardinality {} ; owl:onClass {f} ] .",
                    card(n)
                );
            }
            5 => {
                let (c, r, n) = (rng.below(n_classes), role(&mut rng), 1 + rng.below(2));
                let _ = writeln!(
                    out,
                    ":C{c} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty {r} ; owl:maxCardinality {} ] .",
                    card(n)
                );
            }
            6 => {
                let (c, d, e) = (
                    rng.below(n_classes),
                    rng.below(n_classes),
                    rng.below(n_classes),
                );
                let _ = writeln!(
                    out,
                    ":C{c} rdfs:subClassOf [ a owl:Class ; owl:unionOf ( [ owl:complementOf :C{d} ] :C{e} ) ] ."
                );
            }
            7 => {
                let (a, b) = (rng.below(n_roles), rng.below(n_roles));
                if a != b {
                    let _ = writeln!(out, ":r{a} rdfs:subPropertyOf :r{b} .");
                }
            }
            _ => {
                let (c, r) = (rng.below(n_classes), rng.below(n_roles));
                let _ = writeln!(
                    out,
                    ":C{c} rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r{r} ; owl:hasSelf true ] ."
                );
            }
        }
    }
    // Something is in the cycle.
    let _ = writeln!(
        out,
        ":a a owl:NamedIndividual , :C{} .",
        rng.below(n_classes)
    );
    out
}

#[test]
fn oracle_differential_ni() {
    differential(
        "ni",
        ni_corpus,
        NI_SEEDS,
        include_str!("data/owl2dl_oracle_ni_expected.txt"),
        Some(5),
    );
}

const NI_SEEDS: u64 = 400;

// ---- built-in properties ---------------------------------------------------------------

const U: &str = "owl:topObjectProperty";
const NONE: &str = "owl:bottomObjectProperty";

#[test]
fn the_top_property_reaches_every_individual() {
    let c = |body: &str| verdict(&format!("{DECL} {body}"));
    // `a ∈ ∀U.C` makes every individual `C`, whatever relates them.
    assert_eq!(
        c(&format!(
            ":a a {} . :b a [ owl:complementOf :C ] .",
            all(U, ":C")
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":a a {} .", all(U, ":C"))),
        Consistency::Consistent
    );
    // `∃U.C` holds everywhere once some individual is `C`.
    assert_eq!(
        c(&format!(
            "{} rdfs:subClassOf :D . :a a :C . :b a [ owl:complementOf :D ] .",
            some(U, ":C")
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(
            "{} rdfs:subClassOf :D . :b a [ owl:complementOf :D ] .",
            some(U, ":C")
        )),
        Consistency::Consistent
    );
    // In a superclass and under negation.
    assert_eq!(
        c(&format!(
            ":A rdfs:subClassOf {} . :a a :A . :C rdfs:subClassOf owl:Nothing .",
            some(U, ":C")
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(
            ":a a [ owl:complementOf {} ] . :b a :C .",
            all(U, "[ owl:complementOf :C ]")
        )),
        Consistency::Consistent
    );
    assert_eq!(
        c(&format!(
            ":a a [ owl:complementOf {} ] .",
            some(U, "owl:Thing")
        )),
        Consistency::Inconsistent
    );
    // Assertions: every pair is related by the top property, none by the bottom one.
    assert_eq!(c(&format!(":a {U} :b .")), Consistency::Consistent);
    assert_eq!(
        c(&format!(
            "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty {U} ; owl:targetIndividual :b ] ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(c(&format!(":a {NONE} :b .")), Consistency::Inconsistent);
    assert_eq!(
        c(&format!(
            "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty {NONE} ; owl:targetIndividual :b ] ."
        )),
        Consistency::Consistent
    );
    // Domain and range of the top property: every element.
    assert_eq!(
        c(&format!(
            "{U} rdfs:domain :C . :a a [ owl:complementOf :C ] ."
        )),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!("{NONE} rdfs:range owl:Nothing . :a a :C .")),
        Consistency::Consistent
    );
}

#[test]
fn classification_through_the_top_property() {
    let body = format!(
        "{DECL} {} rdfs:subClassOf :B . :D rdfs:subClassOf {} .",
        some(U, ":A"),
        all(U, ":C")
    );
    // `A ⊑ ∃U.A ⊑ B`; but `C ⊑ B` holds only where `A` is non-empty.
    assert!(entails_sub(&body, "A", "B"));
    assert!(!entails_sub(&body, "C", "B"));
    // `D ⊑ ∀U.C` and `D ⊑ U.D`: `D ⊑ C`, and `A` is not below `C`.
    assert!(entails_sub(&body, "D", "C"));
    assert!(!entails_sub(&body, "A", "C"));
    let body = format!(
        "{DECL} :A rdfs:subClassOf {} . :B owl:equivalentClass {} .",
        some(U, ":B"),
        all(U, "owl:Nothing")
    );
    // `B` is empty (its member would make every element, itself included, nothing).
    assert!(unsat(&body, "B"));
    assert!(unsat(&body, "A"));
    assert!(!unsat(&body, "C"));
}

#[test]
fn the_bottom_property_relates_nothing() {
    let c = |body: &str| verdict(&format!("{DECL} {body}"));
    assert_eq!(
        c(&format!(":a a {} .", some(NONE, "owl:Thing"))),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":a a {} .", has_value(NONE, ":b"))),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":a a {} .", all(NONE, "owl:Nothing"))),
        Consistency::Consistent
    );
    // A sub-property of the bottom property is empty, through chains too.
    assert_eq!(
        c(&format!(":r rdfs:subPropertyOf {NONE} . :a :r :b .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":r owl:equivalentProperty {NONE} . :a :r :b .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":r owl:inverseOf {NONE} . :a :r :b .")),
        Consistency::Inconsistent
    );
    let body = format!("{NONE} owl:propertyChainAxiom ( :r :s ) . :t rdfs:subPropertyOf :r .");
    assert_eq!(
        c(&format!("{body} :a :t :b . :b :s :c .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!("{body} :a :t :b . :c :s :b .")),
        Consistency::Consistent
    );
    // A chain through the bottom property relates nothing either: no constraint.
    assert_eq!(
        c(&format!(
            ":t owl:propertyChainAxiom ( :r {NONE} ) . :a :r :b . :a a {} .",
            all(":t", "owl:Nothing")
        )),
        Consistency::Consistent
    );
}

#[test]
fn the_bottom_data_property_has_no_values() {
    let c = |body: &str| verdict(&format!("{DECL} :d a owl:DatatypeProperty . {body}"));
    let none = "owl:bottomDataProperty";
    assert_eq!(c(&format!(":a {none} 1 .")), Consistency::Inconsistent);
    assert_eq!(
        c(&format!(":a a {} .", dsome(none, "rdfs:Literal"))),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":a a {} .", dall(none, "xsd:string"))),
        Consistency::Consistent
    );
    assert_eq!(
        c(&format!(":d rdfs:subPropertyOf {none} . :a :d 1 .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":d owl:equivalentProperty {none} . :a :d 1 .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":d rdfs:subPropertyOf {none} . :a a :C .")),
        Consistency::Consistent
    );
    assert_eq!(
        c(&format!(
            "[ a owl:NegativePropertyAssertion ; owl:sourceIndividual :a ; owl:assertionProperty {none} ; owl:targetValue 1 ] ."
        )),
        Consistency::Consistent
    );
}

/// Reflexivity and keys on the built-in properties (answers checked with HermiT): the
/// top property is reflexive by definition, a reflexive bottom property has no model, and
/// a key on a bottom property identifies nothing (the last case is the control).
#[test]
fn reflexive_and_key_axioms_on_the_built_in_properties() {
    let c = |body: &str| {
        verdict(&format!(
            "{DECL} :d a owl:DatatypeProperty . :a a :C ; :d 1 . :b a :C ; :d 1 . :a owl:differentFrom :b . {body}"
        ))
    };
    assert_eq!(
        c(&format!("{U} a owl:ReflexiveProperty .")),
        Consistency::Consistent
    );
    assert_eq!(
        c(&format!("{NONE} a owl:ReflexiveProperty .")),
        Consistency::Inconsistent
    );
    assert_eq!(
        c(&format!(":C owl:hasKey ( {NONE} ) .")),
        Consistency::Consistent
    );
    assert_eq!(
        c(":C owl:hasKey ( owl:bottomDataProperty :d ) ."),
        Consistency::Consistent
    );
    assert_eq!(c(":C owl:hasKey ( :d ) ."), Consistency::Inconsistent);
}

/// A class named only inside an expression on a bottom property, which the translation
/// drops, is still classified: with `⊤ ⊑ C1` (the top property's domain), `C0 ⊑ C1`.
/// HermiT lists it for the first three; the OWL API does not read the undeclared `:C0` in
/// a domain axiom as a class, so it leaves the last two out of its signature.
#[test]
fn classes_under_a_bottom_property_are_classified() {
    for body in [
        format!(":C1 rdfs:subClassOf {} .", all(NONE, ":C0")),
        format!("{} rdfs:subClassOf :C2 .", some(NONE, ":C0")),
        format!(":C0 owl:hasKey ( {NONE} ) ."),
        format!("{NONE} rdfs:domain :C0 ."),
        "owl:bottomDataProperty rdfs:domain :C0 .".to_string(),
    ] {
        let body = format!(":C1 a owl:Class . :C2 a owl:Class . {U} rdfs:domain :C1 . {body}");
        assert!(entails_sub(&body, "C0", "C1"), "{body}");
    }
}

/// An at-most restriction that is one disjunct of a union whose other disjunct holds at the
/// individual counts no copies: classifying `C0` (a cycle reaching `:i2`) stays within the
/// default budget, as HermiT answers it.
#[test]
fn a_satisfied_union_at_an_individual_counts_no_copies() {
    let body = ":C0 a owl:Class . :C1 a owl:Class . :C2 a owl:Class . :C5 a owl:Class .
        :r0 a owl:ObjectProperty . :r1 a owl:ObjectProperty .
        :i0 a owl:NamedIndividual . :i2 a owl:NamedIndividual .
        :C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r0 ; owl:someValuesFrom :C0 ] ,
            [ a owl:Restriction ; owl:onProperty :r1 ; owl:hasValue :i2 ] .
        :C1 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r0 ; owl:allValuesFrom :C1 ] ,
            [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :r1 ] ;
              owl:maxQualifiedCardinality \"3\"^^xsd:nonNegativeInteger ;
              owl:onClass [ a owl:Class ; owl:unionOf ( [ owl:complementOf :C2 ] [ owl:complementOf :C0 ] ) ] ] .
        :i0 a [ a owl:Class ; owl:unionOf ( [ a owl:Restriction ; owl:onProperty :r0 ;
            owl:maxQualifiedCardinality \"3\"^^xsd:nonNegativeInteger ; owl:onClass [ owl:complementOf :C1 ] ] :C5 ) ] .
        :i2 a [ a owl:Class ; owl:unionOf ( [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :r1 ] ;
            owl:maxQualifiedCardinality \"1\"^^xsd:nonNegativeInteger ; owl:onClass owl:Thing ] :C1 ) ] .";
    let (d, t) = Graph::parse_to_triples(&format!("{PRE}{body}"), "turtle").unwrap();
    let o = extract(&d, &t).unwrap();
    let report = check(&d, &o, Budget::default());
    assert_eq!(report.consistency, Consistency::Consistent);
    assert_eq!(report.classification_unknown, None);
}

/// A blocked node's edge to an individual that its blocker does not share is dropped with
/// the node when it is unravelled; the graph is still a model when the individual keeps its
/// at-least restrictions without it. Each case gives HermiT's answer.
#[test]
fn a_blocked_node_s_unshared_edge_to_an_individual_is_dropped() {
    let decl = ":C0 a owl:Class . :C1 a owl:Class . :C2 a owl:Class . :C3 a owl:Class . :R a owl:ObjectProperty . :S a owl:ObjectProperty . :T a owl:ObjectProperty . :Q a owl:ObjectProperty . :o a owl:NamedIndividual . :p a owl:NamedIndividual . :a a owl:NamedIndividual .";
    // `C0`s in a `Q`-chain, each `R⁻`-related to `:o`, whose `R`-successors reach `:o`.
    let up = ":a a :C0 . :C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :Q ; owl:someValuesFrom :C0 ] , [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ; owl:hasValue :o ] . :o a [ a owl:Restriction ; owl:onProperty :R ; owl:allValuesFrom [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :S ] ; owl:hasValue :o ] ] .";
    // `C0`s in an `S⁻`-chain, each an `R`-predecessor of `:o`.
    let down = ":a a :C0 . :C0 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :S ] ; owl:someValuesFrom :C0 ] , [ a owl:Restriction ; owl:onProperty :R ; owl:hasValue :o ] . :o a [ a owl:Restriction ; owl:onProperty [ owl:inverseOf :R ] ; owl:allValuesFrom [ a owl:Restriction ; owl:onProperty :S ; owl:allValuesFrom :C1 ] ] .";
    let min = |n: u32, p: &str, c: &str| {
        format!(
            "[ a owl:Restriction ; owl:onProperty {p} ; owl:minQualifiedCardinality \"{n}\"^^xsd:nonNegativeInteger ; owl:onClass {c} ]"
        )
    };
    let cases: Vec<(String, Consistency)> = vec![
        (up.to_string(), Consistency::Consistent),
        (down.to_string(), Consistency::Consistent),
        (format!("{up} :o a {} .", min(3, ":R", "owl:Thing")), Consistency::Consistent),
        (format!("{down} :C1 owl:disjointWith :C0 ."), Consistency::Inconsistent),
        (
            format!(
                "{up} :Q a owl:AsymmetricProperty . :o a [ a owl:Restriction ; owl:onProperty :R ; owl:maxCardinality \"2\"^^xsd:nonNegativeInteger ] ."
            ),
            Consistency::Inconsistent,
        ),
        (
            format!("{down} :o a {} .", min(2, "[ owl:inverseOf :R ]", ":C3")),
            Consistency::Consistent,
        ),
        (
            format!(
                "{up} :o a [ a owl:Restriction ; owl:onProperty :S ; owl:someValuesFrom :C2 ] . :C2 owl:disjointWith :C0 ."
            ),
            Consistency::Consistent,
        ),
    ];
    for (extra, want) in cases {
        assert_eq!(verdict(&format!("{decl} {extra}")), want, "{extra}");
    }
}

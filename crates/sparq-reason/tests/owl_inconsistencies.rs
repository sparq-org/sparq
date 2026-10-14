//! OWL 2 RL inconsistency (clash) detection — `sparq_reason::inconsistencies`.
//!
//! 🤖 SPARQ agent — sq-qcnn test-quality slice.
//!
//! `inconsistencies(dict, triples)` is the load-bearing safety check the trust-graph admission
//! gate relies on: it reports every OWL-RL CLASH the materialized graph entails (an ABox that
//! contradicts the TBox). The whole routine was previously dark; these tests pin each clash
//! rule to a SPEC-CORRECT, hand-derived input and assert (a) the contradiction IS reported with
//! the right offending entities and (b) a consistent variant reports NOTHING. We assert on the
//! deterministic message substrings the routine emits (the entity IRI text + the rule phrase).
//!
//! Rule references are the W3C OWL 2 RL/RDF entailment table (Profiles §4.3).

use oxrdf::vocab::rdf;
use sparq_core::dict::{Dict, Id};
use sparq_reason::{inconsistencies, materialize_owl_rl};

const OWL: &str = "http://www.w3.org/2002/07/owl#";

fn ex(d: &mut Dict, local: &str) -> Id {
    d.intern_iri(&format!("http://ex/{}", local))
}
fn owl(d: &mut Dict, frag: &str) -> Id {
    d.intern_iri(&format!("{}{}", OWL, frag))
}
fn ty(d: &mut Dict) -> Id {
    d.intern_iri(rdf::TYPE.as_str())
}

/// Materialize OWL-RL then collect every reported clash string. Materialization first so that
/// derived clashes (e.g. a FunctionalProperty forcing sameAs between distinct literals) surface.
fn clashes(d: &mut Dict, mut triples: Vec<[Id; 3]>) -> Vec<String> {
    materialize_owl_rl(d, &mut triples);
    inconsistencies(d, &triples)
}

/// Did ANY reported clash mention all of these substrings?
fn any_mentions(reports: &[String], needles: &[&str]) -> bool {
    reports
        .iter()
        .any(|r| needles.iter().all(|n| r.contains(n)))
}

#[test]
fn disjoint_with_typed_both_is_a_clash() {
    // cax-dw: (alice a Person), (alice a Robot), (Person owl:disjointWith Robot) ⊢ CLASH.
    let mut d = Dict::new();
    let (person, robot, alice) = (
        ex(&mut d, "Person"),
        ex(&mut d, "Robot"),
        ex(&mut d, "alice"),
    );
    let (t, dw) = (ty(&mut d), owl(&mut d, "disjointWith"));
    let reports = clashes(
        &mut d,
        vec![[person, dw, robot], [alice, t, person], [alice, t, robot]],
    );
    assert!(
        any_mentions(
            &reports,
            &["http://ex/alice", "http://ex/Person", "http://ex/Robot"]
        ),
        "cax-dw clash must name alice + both disjoint classes; got {:?}",
        reports
    );
    // A non-overlapping individual is NOT a clash.
    let mut d2 = Dict::new();
    let (person, robot, alice) = (
        ex(&mut d2, "Person"),
        ex(&mut d2, "Robot"),
        ex(&mut d2, "alice"),
    );
    let (t, dw) = (ty(&mut d2), owl(&mut d2, "disjointWith"));
    let ok = clashes(&mut d2, vec![[person, dw, robot], [alice, t, person]]);
    assert!(
        ok.is_empty(),
        "single class membership is consistent; got {:?}",
        ok
    );
}

#[test]
fn complement_of_typed_both_is_a_clash() {
    // cls-com: (x a Human), (x a Machine), (Human owl:complementOf Machine) ⊢ CLASH.
    let mut d = Dict::new();
    let (human, machine, x) = (ex(&mut d, "Human"), ex(&mut d, "Machine"), ex(&mut d, "x"));
    let (t, comp) = (ty(&mut d), owl(&mut d, "complementOf"));
    let reports = clashes(
        &mut d,
        vec![[human, comp, machine], [x, t, human], [x, t, machine]],
    );
    assert!(
        any_mentions(
            &reports,
            &["http://ex/x", "http://ex/Human", "http://ex/Machine"]
        ),
        "cls-com clash must name x + the complementary classes; got {:?}",
        reports
    );
}

#[test]
fn nothing_typed_individual_is_a_clash() {
    // cls-nothing: anything typed owl:Nothing is inconsistent.
    let mut d = Dict::new();
    let (nada, t) = (owl(&mut d, "Nothing"), ty(&mut d));
    let x = ex(&mut d, "ghost");
    let reports = clashes(&mut d, vec![[x, t, nada]]);
    assert!(
        any_mentions(&reports, &["http://ex/ghost", "Nothing"]),
        "owl:Nothing membership must clash; got {:?}",
        reports
    );
    // Nothing axiom present but unused ⇒ no clash.
    let mut d2 = Dict::new();
    let (thing, t, x) = (owl(&mut d2, "Thing"), ty(&mut d2), ex(&mut d2, "ok"));
    assert!(
        clashes(&mut d2, vec![[x, t, thing]]).is_empty(),
        "owl:Thing is consistent"
    );
}

#[test]
fn same_as_and_different_from_is_a_clash() {
    // eq-diff1: (a sameAs b) AND (a differentFrom b) ⊢ CLASH.
    let mut d = Dict::new();
    let (a, b) = (ex(&mut d, "a"), ex(&mut d, "b"));
    let (sa, df) = (owl(&mut d, "sameAs"), owl(&mut d, "differentFrom"));
    let reports = clashes(&mut d, vec![[a, sa, b], [a, df, b]]);
    assert!(
        any_mentions(
            &reports,
            &["http://ex/a", "http://ex/b", "sameAs", "differentFrom"]
        ),
        "sameAs + differentFrom on the same pair must clash; got {:?}",
        reports
    );
    // sameAs alone is consistent.
    let mut d2 = Dict::new();
    let (a, b, sa) = (ex(&mut d2, "a"), ex(&mut d2, "b"), owl(&mut d2, "sameAs"));
    assert!(
        clashes(&mut d2, vec![[a, sa, b]]).is_empty(),
        "sameAs alone is consistent"
    );
}

#[test]
fn asymmetric_property_both_ways_is_a_clash() {
    // prp-asyp: (knows asymmetric), (alice knows bob), (bob knows alice) ⊢ CLASH.
    let mut d = Dict::new();
    let (knows, alice, bob) = (ex(&mut d, "knows"), ex(&mut d, "alice"), ex(&mut d, "bob"));
    let (t, asym) = (ty(&mut d), owl(&mut d, "AsymmetricProperty"));
    let reports = clashes(
        &mut d,
        vec![[knows, t, asym], [alice, knows, bob], [bob, knows, alice]],
    );
    assert!(
        any_mentions(
            &reports,
            &["asymmetric", "http://ex/alice", "http://ex/bob"]
        ),
        "prp-asyp clash must name the property + both ends; got {:?}",
        reports
    );
    // One-directional is fine for an asymmetric property.
    let mut d2 = Dict::new();
    let (knows, alice, bob) = (
        ex(&mut d2, "knows"),
        ex(&mut d2, "alice"),
        ex(&mut d2, "bob"),
    );
    let (t, asym) = (ty(&mut d2), owl(&mut d2, "AsymmetricProperty"));
    let ok = clashes(&mut d2, vec![[knows, t, asym], [alice, knows, bob]]);
    assert!(
        ok.is_empty(),
        "one-way asymmetric edge is consistent; got {:?}",
        ok
    );
}

#[test]
fn irreflexive_property_self_loop_is_a_clash() {
    // prp-irp: (before irreflexive), (e before e) ⊢ CLASH.
    let mut d = Dict::new();
    let (before, e) = (ex(&mut d, "before"), ex(&mut d, "e1"));
    let (t, irr) = (ty(&mut d), owl(&mut d, "IrreflexiveProperty"));
    let reports = clashes(&mut d, vec![[before, t, irr], [e, before, e]]);
    assert!(
        any_mentions(&reports, &["irreflexive", "http://ex/e1"]),
        "prp-irp self-loop must clash; got {:?}",
        reports
    );
    // A non-self edge of an irreflexive property is fine.
    let mut d2 = Dict::new();
    let (before, e, f) = (ex(&mut d2, "before"), ex(&mut d2, "e1"), ex(&mut d2, "e2"));
    let (t, irr) = (ty(&mut d2), owl(&mut d2, "IrreflexiveProperty"));
    assert!(
        clashes(&mut d2, vec![[before, t, irr], [e, before, f]]).is_empty(),
        "distinct-endpoint irreflexive edge is consistent"
    );
}

#[test]
fn property_disjoint_with_sharing_a_pair_is_a_clash() {
    // prp-pdw: (wife propertyDisjointWith daughter), (alice wife bob), (alice daughter bob) ⊢ CLASH.
    let mut d = Dict::new();
    let (wife, daughter, alice, bob) = (
        ex(&mut d, "wife"),
        ex(&mut d, "daughter"),
        ex(&mut d, "alice"),
        ex(&mut d, "bob"),
    );
    let pdw = owl(&mut d, "propertyDisjointWith");
    let reports = clashes(
        &mut d,
        vec![
            [wife, pdw, daughter],
            [alice, wife, bob],
            [alice, daughter, bob],
        ],
    );
    assert!(
        any_mentions(
            &reports,
            &["disjoint properties", "http://ex/alice", "http://ex/bob"]
        ),
        "prp-pdw clash must name the shared (subject, object) pair; got {:?}",
        reports
    );
}

#[test]
fn all_disjoint_classes_two_members_is_a_clash() {
    // cax-adc: a node typed by two members of an owl:AllDisjointClasses set ⊢ CLASH.
    let mut d = Dict::new();
    let (a_cls, b_cls, c_cls, alice) = (
        ex(&mut d, "A"),
        ex(&mut d, "B"),
        ex(&mut d, "C"),
        ex(&mut d, "alice"),
    );
    let (t, adc, members) = (
        ty(&mut d),
        owl(&mut d, "AllDisjointClasses"),
        owl(&mut d, "members"),
    );
    let (rf, rr, rn) = (
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#first"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
    );
    let (l1, l2, l3) = (ex(&mut d, "L1"), ex(&mut d, "L2"), ex(&mut d, "L3"));
    let z = ex(&mut d, "ADC");
    // ADC a AllDisjointClasses ; members ( A B C ) — encoded as an rdf:List.
    let reports = clashes(
        &mut d,
        vec![
            [z, t, adc],
            [z, members, l1],
            [l1, rf, a_cls],
            [l1, rr, l2],
            [l2, rf, b_cls],
            [l2, rr, l3],
            [l3, rf, c_cls],
            [l3, rr, rn],
            [alice, t, a_cls],
            [alice, t, b_cls],
        ],
    );
    assert!(
        any_mentions(&reports, &["http://ex/alice", "AllDisjointClasses"]),
        "cax-adc clash must name the doubly-typed individual; got {:?}",
        reports
    );
    // Typed by only ONE member ⇒ consistent.
    let mut d2 = Dict::new();
    let (a_cls, b_cls, c_cls, alice) = (
        ex(&mut d2, "A"),
        ex(&mut d2, "B"),
        ex(&mut d2, "C"),
        ex(&mut d2, "alice"),
    );
    let (t, adc, members) = (
        ty(&mut d2),
        owl(&mut d2, "AllDisjointClasses"),
        owl(&mut d2, "members"),
    );
    let (rf, rr, rn) = (
        d2.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#first"),
        d2.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"),
        d2.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
    );
    let (l1, l2, l3) = (ex(&mut d2, "L1"), ex(&mut d2, "L2"), ex(&mut d2, "L3"));
    let z = ex(&mut d2, "ADC");
    let ok = clashes(
        &mut d2,
        vec![
            [z, t, adc],
            [z, members, l1],
            [l1, rf, a_cls],
            [l1, rr, l2],
            [l2, rf, b_cls],
            [l2, rr, l3],
            [l3, rf, c_cls],
            [l3, rr, rn],
            [alice, t, a_cls],
        ],
    );
    assert!(
        ok.is_empty(),
        "one disjoint-set member is consistent; got {:?}",
        ok
    );
}

#[test]
fn all_different_same_members_is_a_clash() {
    // eq-diff2/3: (z a AllDifferent ; members (a b)), (a sameAs b) ⊢ CLASH.
    let mut d = Dict::new();
    let (a, b) = (ex(&mut d, "a"), ex(&mut d, "b"));
    let (t, alldiff, members, sa) = (
        ty(&mut d),
        owl(&mut d, "AllDifferent"),
        owl(&mut d, "members"),
        owl(&mut d, "sameAs"),
    );
    let (rf, rr, rn) = (
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#first"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#rest"),
        d.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
    );
    let (l1, l2) = (ex(&mut d, "L1"), ex(&mut d, "L2"));
    let z = ex(&mut d, "AD");
    let reports = clashes(
        &mut d,
        vec![
            [z, t, alldiff],
            [z, members, l1],
            [l1, rf, a],
            [l1, rr, l2],
            [l2, rf, b],
            [l2, rr, rn],
            [a, sa, b],
        ],
    );
    assert!(
        any_mentions(&reports, &["AllDifferent", "http://ex/a", "http://ex/b"]),
        "eq-diff AllDifferent members forced equal must clash; got {:?}",
        reports
    );
}

#[test]
fn negative_property_assertion_violated_is_a_clash() {
    // prp-npa1: an NPA (source alice, property knows, target bob) yet (alice knows bob) ⊢ CLASH.
    let mut d = Dict::new();
    let (alice, knows, bob, z) = (
        ex(&mut d, "alice"),
        ex(&mut d, "knows"),
        ex(&mut d, "bob"),
        ex(&mut d, "NPA"),
    );
    let (t, npa, si, ap, ti) = (
        ty(&mut d),
        owl(&mut d, "NegativePropertyAssertion"),
        owl(&mut d, "sourceIndividual"),
        owl(&mut d, "assertionProperty"),
        owl(&mut d, "targetIndividual"),
    );
    let reports = clashes(
        &mut d,
        vec![
            [z, t, npa],
            [z, si, alice],
            [z, ap, knows],
            [z, ti, bob],
            [alice, knows, bob],
        ],
    );
    assert!(
        any_mentions(
            &reports,
            &[
                "negative property assertion",
                "http://ex/alice",
                "http://ex/bob"
            ]
        ),
        "prp-npa1 violation must clash; got {:?}",
        reports
    );
    // The same NPA WITHOUT the asserted edge is consistent.
    let mut d2 = Dict::new();
    let (alice, knows, bob, z) = (
        ex(&mut d2, "alice"),
        ex(&mut d2, "knows"),
        ex(&mut d2, "bob"),
        ex(&mut d2, "NPA"),
    );
    let (t, npa, si, ap, ti) = (
        ty(&mut d2),
        owl(&mut d2, "NegativePropertyAssertion"),
        owl(&mut d2, "sourceIndividual"),
        owl(&mut d2, "assertionProperty"),
        owl(&mut d2, "targetIndividual"),
    );
    let ok = clashes(
        &mut d2,
        vec![[z, t, npa], [z, si, alice], [z, ap, knows], [z, ti, bob]],
    );
    assert!(
        ok.is_empty(),
        "an un-violated NPA is consistent; got {:?}",
        ok
    );
}

#[test]
fn functional_property_distinct_literals_is_a_clash() {
    // prp-fp then dt-diff: a FunctionalProperty with two DIFFERENT literal values forces them
    // sameAs, and distinct literal values can never be sameAs ⊢ CLASH. Exercises the derived-
    // clash path (the clash only appears AFTER materialization runs prp-fp).
    let mut d = Dict::new();
    let (ssn, alice) = (ex(&mut d, "ssn"), ex(&mut d, "alice"));
    let (t, fp) = (ty(&mut d), owl(&mut d, "FunctionalProperty"));
    let xsd_str = "http://www.w3.org/2001/XMLSchema#string";
    let l1 = d.intern_lit("123", xsd_str, None);
    let l2 = d.intern_lit("456", xsd_str, None);
    let reports = clashes(
        &mut d,
        vec![[ssn, t, fp], [alice, ssn, l1], [alice, ssn, l2]],
    );
    assert!(
        any_mentions(&reports, &["distinct literal values", "123", "456"]),
        "prp-fp on distinct literals must clash via dt-diff; got {:?}",
        reports
    );
    // The SAME literal twice is consistent (no distinct values forced).
    let mut d2 = Dict::new();
    let (ssn, alice) = (ex(&mut d2, "ssn"), ex(&mut d2, "alice"));
    let (t, fp) = (ty(&mut d2), owl(&mut d2, "FunctionalProperty"));
    let l1 = d2.intern_lit("123", xsd_str, None);
    let l1b = d2.intern_lit("123", xsd_str, None);
    let ok = clashes(
        &mut d2,
        vec![[ssn, t, fp], [alice, ssn, l1], [alice, ssn, l1b]],
    );
    assert!(
        ok.is_empty(),
        "same literal twice on a FunctionalProperty is consistent; got {:?}",
        ok
    );
}

#[test]
fn max_cardinality_zero_with_value_is_a_clash() {
    // cls-maxc1: (x a R), (R onProperty p ; maxCardinality 0), (x p y) ⊢ CLASH.
    let mut d = Dict::new();
    let (x, p, y, r) = (
        ex(&mut d, "x"),
        ex(&mut d, "p"),
        ex(&mut d, "y"),
        ex(&mut d, "R"),
    );
    let (t, on_prop, max_card) = (
        ty(&mut d),
        owl(&mut d, "onProperty"),
        owl(&mut d, "maxCardinality"),
    );
    let zero = d.intern_lit(
        "0",
        "http://www.w3.org/2001/XMLSchema#nonNegativeInteger",
        None,
    );
    let reports = clashes(
        &mut d,
        vec![[r, on_prop, p], [r, max_card, zero], [x, t, r], [x, p, y]],
    );
    assert!(
        any_mentions(&reports, &["http://ex/x", "maxCardinality 0"]),
        "cls-maxc1 (a value under a maxCardinality-0 restriction) must clash; got {:?}",
        reports
    );
}

#[test]
fn consistent_ontology_reports_nothing() {
    // A wholly-consistent OWL-RL graph must report ZERO inconsistencies (fail-open guard: the
    // admission gate must not reject a clean graph).
    let mut d = Dict::new();
    let (person, agent, alice, knows, bob) = (
        ex(&mut d, "Person"),
        ex(&mut d, "Agent"),
        ex(&mut d, "alice"),
        ex(&mut d, "knows"),
        ex(&mut d, "bob"),
    );
    let (t, sc, sym) = (
        ty(&mut d),
        d.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf"),
        owl(&mut d, "SymmetricProperty"),
    );
    let reports = clashes(
        &mut d,
        vec![
            [person, sc, agent],
            [knows, t, sym],
            [alice, t, person],
            [alice, knows, bob],
        ],
    );
    assert!(
        reports.is_empty(),
        "a consistent ontology must report no clash; got {:?}",
        reports
    );
}

#[test]
fn max_qualified_cardinality_nonzero_with_a_value_is_consistent() {
    // A maxQualifiedCardinality of 1 (NOT 0) on a typed value is satisfiable with a single
    // value — it must NOT clash. This pins the value==0 GUARD on the qualified-cardinality
    // collection: only the 0-valued restrictions are clash candidates; a >0 restriction with
    // one conforming value is consistent. (If the `p == maxQualifiedCardinality && value==0`
    // guard degraded to an OR, this exact graph would spuriously clash.)
    let mut d = Dict::new();
    let (p, c, u, y, r) = (
        ex(&mut d, "parent"),
        ex(&mut d, "Mother"),
        ex(&mut d, "u"),
        ex(&mut d, "mom"),
        ex(&mut d, "RQ"),
    );
    let (t, on_prop, mqc, on_class) = (
        ty(&mut d),
        owl(&mut d, "onProperty"),
        owl(&mut d, "maxQualifiedCardinality"),
        owl(&mut d, "onClass"),
    );
    let one = d.intern_lit(
        "1",
        "http://www.w3.org/2001/XMLSchema#nonNegativeInteger",
        None,
    );
    let ok = clashes(
        &mut d,
        vec![
            [r, on_prop, p],
            [r, mqc, one],
            [r, on_class, c],
            [u, t, r],
            [u, p, y],
            [y, t, c],
        ],
    );
    assert!(
        ok.is_empty(),
        "maxQualifiedCardinality 1 with a single conforming value is consistent; got {:?}",
        ok
    );
}

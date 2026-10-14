//! ALWAYS-ON per-regime NON-entailment unit set (no fixture needed).
//!
//! The reason suite is strong on POSITIVE coverage (incremental == from-scratch
//! property tests, the EYE differential, the W3C inference ratchet) but the
//! explain_*/EYE/W3C suites all SKIP without their fixtures, so a plain offline
//! `cargo test` exercises far less, and NEGATIVE coverage was thin. These tests
//! need NO fixture — they harden the default-test floor by asserting the
//! reasoner does NOT OVER-entail: for each regime (RDFS, OWL-RL, N3) a tiny
//! graph is materialized and specific triples that MUST NOT be in the closure
//! are asserted absent (an unstated subClassOf, a fabricated sameAs, a spurious
//! type, …). If a regime ever starts inferring one of these, the floor catches
//! it even when every fixture-gated suite is skipped.

use sparq_core::dict::{Dict, Id};
use sparq_reason::{materialize_owl_rl, materialize_rdfs, reason_n3};

use oxrdf::vocab::{rdf, rdfs};
use oxrdf::{NamedNode, Term};

const OWL: &str = "http://www.w3.org/2002/07/owl#";

/// Intern an IRI to its id.
fn iri(dict: &mut Dict, s: &str) -> Id {
    dict.intern_iri(s)
}
/// Intern an `http://ex/<local>` IRI.
fn ex(dict: &mut Dict, local: &str) -> Id {
    dict.intern_iri(&format!("http://ex/{local}"))
}

/// Look up an ALREADY-interned triple by IRI strings; returns `None` if any term
/// is unknown to the dict (which itself proves the triple cannot be in any
/// id-set — an unknown term has id 0 and is never asserted/derived).
fn lookup3(dict: &Dict, s: &str, p: &str, o: &str) -> Option<[Id; 3]> {
    let g = |i: &str| {
        let id = dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(i.to_string())));
        (id != 0).then_some(id)
    };
    Some([g(s)?, g(p)?, g(o)?])
}

/// Assert `(s,p,o)` is PRESENT in the closure (a sanity anchor so each test
/// proves the regime fired at all before we trust its NON-derivations).
fn assert_present(dict: &Dict, set: &[[Id; 3]], s: &str, p: &str, o: &str) {
    let want = lookup3(dict, s, p, o)
        .unwrap_or_else(|| panic!("expected-present triple has an unknown term: ({s} {p} {o})"));
    assert!(set.contains(&want), "expected {s} {p} {o} to be ENTAILED, but it is absent");
}

/// Assert `(s,p,o)` is ABSENT from the closure. If any term is unknown the triple
/// is trivially absent (and we say so) — but the callers intern every term they
/// reference up front, so this normally checks genuine set non-membership.
fn assert_absent(dict: &Dict, set: &[[Id; 3]], s: &str, p: &str, o: &str) {
    if let Some(t) = lookup3(dict, s, p, o) {
        assert!(!set.contains(&t), "reasoner OVER-ENTAILED: ({s} {p} {o}) must NOT be in the closure");
    }
}

// =============================== RDFS ===================================================

/// RDFS must not invent subclass/type edges that the schema does not state.
#[test]
fn rdfs_does_not_over_entail() {
    let mut dict = Dict::new();
    // Dog ⊑ Mammal ⊑ Animal ; Cat ⊑ Mammal ; rex a Dog ; felix a Cat.
    let (dog, cat, mammal, animal, rex, felix) = (
        ex(&mut dict, "Dog"),
        ex(&mut dict, "Cat"),
        ex(&mut dict, "Mammal"),
        ex(&mut dict, "Animal"),
        ex(&mut dict, "rex"),
        ex(&mut dict, "felix"),
    );
    // Terms referenced only in NEGATIVE checks must exist so the lookup is a real
    // set-membership test (not a trivially-unknown miss).
    let (plant, reptile) = (ex(&mut dict, "Plant"), ex(&mut dict, "Reptile"));
    let (sc, ty) = (iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()), iri(&mut dict, rdf::TYPE.as_str()));

    let mut triples = vec![
        [dog, sc, mammal],
        [mammal, sc, animal],
        [cat, sc, mammal],
        [rex, ty, dog],
        [felix, ty, cat],
    ];
    let _ = (plant, reptile);
    materialize_rdfs(&mut dict, &mut triples);

    // Sanity: the regime fired (transitive type + subclass).
    assert_present(&dict, &triples, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Animal");
    assert_present(&dict, &triples, "http://ex/Dog", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Animal");

    // NON-entailment:
    // (1) subClassOf is NOT symmetric — Animal ⊑ Dog is never inferred.
    assert_absent(&dict, &triples, "http://ex/Animal", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Dog");
    // (2) siblings are unrelated — Dog ⊑ Cat (and vice versa) must NOT appear.
    assert_absent(&dict, &triples, "http://ex/Dog", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Cat");
    assert_absent(&dict, &triples, "http://ex/Cat", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Dog");
    // (3) no spurious cross-type: rex (a Dog) is NOT a Cat.
    assert_absent(&dict, &triples, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Cat");
    // (4) classes never mentioned in the hierarchy gain no relationships.
    assert_absent(&dict, &triples, "http://ex/Dog", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Plant");
    assert_absent(&dict, &triples, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Reptile");
    // (5) RDFS materializes the NON-axiomatic subset: no reflexive subClassOf, no
    //     blanket rdfs:Resource typing (this is the documented design choice).
    assert_absent(&dict, &triples, "http://ex/Dog", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Dog");
    assert_absent(&dict, &triples, "http://ex/rex", rdf::TYPE.as_str(), rdfs::RESOURCE.as_str());
}

/// RDFS domain/range type a subject/object — but ONLY for the asserted property,
/// not the converse position, and not for unrelated properties.
#[test]
fn rdfs_domain_range_do_not_cross() {
    let mut dict = Dict::new();
    // authored domain Author, range Book ; ex:alice authored ex:b1.
    let (authored, author, book, alice, b1) = (
        ex(&mut dict, "authored"),
        ex(&mut dict, "Author"),
        ex(&mut dict, "Book"),
        ex(&mut dict, "alice"),
        ex(&mut dict, "b1"),
    );
    let (dom, rng) =
        (iri(&mut dict, rdfs::DOMAIN.as_str()), iri(&mut dict, rdfs::RANGE.as_str()));
    let mut triples = vec![[authored, dom, author], [authored, rng, book], [alice, authored, b1]];
    materialize_rdfs(&mut dict, &mut triples);

    // Sanity: domain types the subject, range the object.
    assert_present(&dict, &triples, "http://ex/alice", rdf::TYPE.as_str(), "http://ex/Author");
    assert_present(&dict, &triples, "http://ex/b1", rdf::TYPE.as_str(), "http://ex/Book");

    // NON-entailment: domain/range do NOT swap — the subject is not a Book, the
    // object is not an Author.
    assert_absent(&dict, &triples, "http://ex/alice", rdf::TYPE.as_str(), "http://ex/Book");
    assert_absent(&dict, &triples, "http://ex/b1", rdf::TYPE.as_str(), "http://ex/Author");
}

// ============================== OWL-RL ==================================================

/// OWL-RL must not fabricate equality. With no sameAs/functional axioms, no
/// `owl:sameAs` triple may appear — and equality is never invented between
/// distinct individuals.
#[test]
fn owl_rl_does_not_fabricate_same_as() {
    let mut dict = Dict::new();
    // Two individuals, each with an ordinary (non-functional) property value.
    let (knows, alice, bob, carol) = (
        ex(&mut dict, "knows"),
        ex(&mut dict, "alice"),
        ex(&mut dict, "bob"),
        ex(&mut dict, "carol"),
    );
    let same_as = iri(&mut dict, &format!("{OWL}sameAs"));
    let mut triples = vec![[alice, knows, bob], [carol, knows, bob]];
    materialize_owl_rl(&mut dict, &mut triples);

    // No equality anywhere: no individual is sameAs another (or itself — OWL-RL
    // does not emit reflexive sameAs here).
    for (a, b) in [("alice", "bob"), ("alice", "carol"), ("bob", "carol"), ("alice", "alice")] {
        assert_absent(&dict, &triples, &format!("http://ex/{a}"), &format!("{OWL}sameAs"), &format!("http://ex/{b}"));
    }
    let _ = same_as;
}

/// A FunctionalProperty merges the two objects of the SAME subject (prp-fp) — but
/// must NOT merge objects across different subjects, nor merge subjects.
#[test]
fn owl_rl_functional_property_is_scoped() {
    let mut dict = Dict::new();
    // hasFather is functional. alice hasFather jim; alice hasFather james ⟹ jim sameAs james.
    // bob hasFather jim — bob and alice share a father but are NOT thereby equal.
    let (has_father, alice, bob, jim, james) = (
        ex(&mut dict, "hasFather"),
        ex(&mut dict, "alice"),
        ex(&mut dict, "bob"),
        ex(&mut dict, "jim"),
        ex(&mut dict, "james"),
    );
    let ty = iri(&mut dict, rdf::TYPE.as_str());
    let functional = iri(&mut dict, &format!("{OWL}FunctionalProperty"));
    let mut triples = vec![
        [has_father, ty, functional],
        [alice, has_father, jim],
        [alice, has_father, james],
        [bob, has_father, jim],
    ];
    materialize_owl_rl(&mut dict, &mut triples);

    // Sanity (prp-fp): alice's two fathers are merged — sameAs in SOME direction
    // (entity rewriting canonicalizes, so check both orientations).
    let jim_james = lookup3(&dict, "http://ex/jim", &format!("{OWL}sameAs"), "http://ex/james");
    let james_jim = lookup3(&dict, "http://ex/james", &format!("{OWL}sameAs"), "http://ex/jim");
    assert!(
        jim_james.is_some_and(|t| triples.contains(&t))
            || james_jim.is_some_and(|t| triples.contains(&t)),
        "prp-fp should merge alice's two fathers (jim sameAs james)"
    );

    // NON-entailment: sharing a (functional) father does NOT make the children
    // equal — alice sameAs bob must never be derived.
    assert_absent(&dict, &triples, "http://ex/alice", &format!("{OWL}sameAs"), "http://ex/bob");
    assert_absent(&dict, &triples, "http://ex/bob", &format!("{OWL}sameAs"), "http://ex/alice");
}

/// OWL-RL must not type individuals into classes they were never (directly or via
/// the stated TBox) declared to belong to, and must not turn an ordinary property
/// into a symmetric/inverse one without the axiom.
#[test]
fn owl_rl_no_spurious_type_or_symmetry() {
    let mut dict = Dict::new();
    // likes is a plain property (NOT symmetric/inverse). Person ⊑ Agent.
    let (likes, person, agent, vehicle, alice, bob) = (
        ex(&mut dict, "likes"),
        ex(&mut dict, "Person"),
        ex(&mut dict, "Agent"),
        ex(&mut dict, "Vehicle"),
        ex(&mut dict, "alice"),
        ex(&mut dict, "bob"),
    );
    let (sc, ty) = (iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()), iri(&mut dict, rdf::TYPE.as_str()));
    let mut triples = vec![[person, sc, agent], [alice, ty, person], [alice, likes, bob]];
    materialize_owl_rl(&mut dict, &mut triples);

    // Sanity: subclass typing still fires under OWL-RL (it subsumes RDFS).
    assert_present(&dict, &triples, "http://ex/alice", rdf::TYPE.as_str(), "http://ex/Agent");

    // NON-entailment:
    // (1) `likes` is not symmetric — (bob likes alice) is NOT derived.
    assert_absent(&dict, &triples, "http://ex/bob", "http://ex/likes", "http://ex/alice");
    // (2) bob gets no type from being liked (no domain/range on `likes`).
    assert_absent(&dict, &triples, "http://ex/bob", rdf::TYPE.as_str(), "http://ex/Person");
    assert_absent(&dict, &triples, "http://ex/bob", rdf::TYPE.as_str(), "http://ex/Agent");
    // (3) no membership in an unrelated class.
    assert_absent(&dict, &triples, "http://ex/alice", rdf::TYPE.as_str(), "http://ex/Vehicle");
    let _ = vehicle;
}

// ================================= N3 ===================================================

/// The closure of an N3 document as canonical (s, p, o) STRING triples.
fn n3_closure_strings(src: &str) -> std::collections::HashSet<(String, String, String)> {
    let mut d = Dict::new();
    let triples = reason_n3(&mut d, src).expect("N3 reasoning failed");
    triples
        .iter()
        .map(|[s, p, o]| (d.term(*s).to_string(), d.term(*p).to_string(), d.term(*o).to_string()))
        .collect()
}

/// N3 forward rules fire only on the stated premises — they must not fire in
/// reverse, nor produce a conclusion whose premise is unmet.
#[test]
fn n3_rule_does_not_fire_in_reverse() {
    // Rule: { ?x :parent ?y } => { ?x :ancestor ?y }. Data: a parent b.
    let src = r#"
        @prefix : <http://ex/> .
        { ?x :parent ?y } => { ?x :ancestor ?y } .
        :a :parent :b .
    "#;
    let c = n3_closure_strings(src);
    let t = |s: &str, p: &str, o: &str| (s.to_string(), p.to_string(), o.to_string());
    let ex = |l: &str| format!("<http://ex/{l}>");

    // Sanity: the forward rule fires.
    assert!(
        c.contains(&t(&ex("a"), &ex("ancestor"), &ex("b"))),
        "the forward rule {{?x :parent ?y}} => {{?x :ancestor ?y}} should fire"
    );

    // NON-entailment:
    // (1) the rule is NOT bidirectional — having an ancestor does not derive a
    //     parent edge in reverse, and no ancestor edge appears for the reverse
    //     pair.
    assert!(!c.contains(&t(&ex("b"), &ex("parent"), &ex("a"))), "rule must not run in reverse");
    assert!(!c.contains(&t(&ex("b"), &ex("ancestor"), &ex("a"))), "no reverse ancestor edge");
    // (2) :ancestor is NOT (silently) transitive — no rule states it, so a single
    //     parent edge yields no chained ancestor.
    assert!(
        !c.contains(&t(&ex("a"), &ex("ancestor"), &ex("a"))),
        ":ancestor must not be reflexive without a rule"
    );
}

/// An N3 rule whose premise is UNSATISFIED must produce nothing — no conclusion
/// is fabricated, and a `math:` guard that fails blocks the firing.
#[test]
fn n3_unmet_premise_yields_no_conclusion() {
    // Rule fires only when ?n math:greaterThan 10. Data has n=3 (fails) and n=42
    // (passes). Only the passing one may produce :big.
    let src = r#"
        @prefix : <http://ex/> .
        @prefix math: <http://www.w3.org/2000/10/swap/math#> .
        { :small :n 3 . :small :n ?v . ?v math:greaterThan 10 } => { :small :flag true } .
        { :large :n ?v . ?v math:greaterThan 10 } => { :large :flag true } .
        :small :n 3 .
        :large :n 42 .
    "#;
    let c = n3_closure_strings(src);
    let big = |subj: &str| {
        c.iter()
            .any(|(s, p, _)| s == &format!("<http://ex/{subj}>") && p == "<http://ex/flag>")
    };
    // Sanity: the satisfied guard fires for :large.
    assert!(big("large"), "math:greaterThan 10 holds for 42 — :large :flag should fire");
    // NON-entailment: the FAILED guard (3 > 10 is false) fires nothing for :small.
    assert!(!big("small"), "math:greaterThan 10 is false for 3 — :small must get no :flag");
}

/// N3 over-firing guard: a rule with two premise atoms must require BOTH; a
/// partial match (only one atom present) derives nothing.
#[test]
fn n3_conjunctive_premise_requires_all_atoms() {
    // Rule needs both :p and :q on ?x. node1 has only :p; node2 has both.
    let src = r#"
        @prefix : <http://ex/> .
        { ?x :p :v . ?x :q :w } => { ?x :ok true } .
        :node1 :p :v .
        :node2 :p :v .
        :node2 :q :w .
    "#;
    let c = n3_closure_strings(src);
    let ok = |subj: &str| {
        c.iter()
            .any(|(s, p, _)| s == &format!("<http://ex/{subj}>") && p == "<http://ex/ok>")
    };
    assert!(ok("node2"), "node2 satisfies both premise atoms — should derive :ok");
    assert!(!ok("node1"), "node1 satisfies only one atom — must NOT derive :ok");
}

//! Public API entry points of sparq-reason, called directly.
//!
//! 🤖 SPARQ agent — sq-qcnn test-quality slice.
//!
//! `Profile::parse`, `materialize(profile, …)`, and the N3 entry points (`reason_n3`,
//! `reason_n3_proof`, `reason_n3_terms`) were dark when invoked through the crate's PUBLIC
//! surface (the inline tests reach the rule layer directly). These pin the published API
//! contract: each call returns the EXACT entailed closure / parsed profile, hand-derived
//! from RDFS / OWL-RL / N3 semantics.

use sparq_core::dict::{Dict, Id};
use sparq_reason::{materialize, reason_n3, reason_n3_proof, reason_n3_terms, Profile};

const RDFS_SC: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const OWL_SYM: &str = "http://www.w3.org/2002/07/owl#SymmetricProperty";

fn iri(d: &mut Dict, s: &str) -> Id {
    d.intern_iri(s)
}

#[test]
fn profile_parse_accepts_documented_aliases_and_rejects_others() {
    // The exact alias set the CLI exposes; everything else is None.
    assert_eq!(Profile::parse("rdfs"), Some(Profile::Rdfs));
    assert_eq!(
        Profile::parse("RDFS"),
        Some(Profile::Rdfs),
        "case-insensitive"
    );
    assert_eq!(Profile::parse("owl"), Some(Profile::OwlRl));
    assert_eq!(Profile::parse("owl-rl"), Some(Profile::OwlRl));
    assert_eq!(Profile::parse("owlrl"), Some(Profile::OwlRl));
    assert_eq!(
        Profile::parse("OWL-RL"),
        Some(Profile::OwlRl),
        "case-insensitive alias"
    );
    assert_eq!(
        Profile::parse("owl2"),
        None,
        "unknown profile name is rejected"
    );
    assert_eq!(Profile::parse(""), None);
    assert_eq!(
        Profile::parse("n3"),
        None,
        "n3 is not a materialization profile"
    );
}

#[test]
fn materialize_rdfs_via_public_api_derives_the_subclass_closure() {
    // Profile::Rdfs through the public `materialize` entry: Dog ⊑ Mammal ⊑ Animal, rex a Dog
    // ⊢ rex a Mammal, rex a Animal, Dog ⊑ Animal (rdfs9 + rdfs11). Assert EXACT new count.
    let mut d = Dict::new();
    let (dog, mammal, animal, rex) = (
        iri(&mut d, "http://ex/Dog"),
        iri(&mut d, "http://ex/Mammal"),
        iri(&mut d, "http://ex/Animal"),
        iri(&mut d, "http://ex/rex"),
    );
    let (sc, ty) = (iri(&mut d, RDFS_SC), iri(&mut d, RDF_TYPE));
    let mut triples = vec![[dog, sc, mammal], [mammal, sc, animal], [rex, ty, dog]];
    let added = materialize(Profile::Rdfs, &mut d, &mut triples);
    let set: std::collections::HashSet<[Id; 3]> = triples.iter().copied().collect();
    assert!(set.contains(&[rex, ty, mammal]), "rdfs9 one hop");
    assert!(set.contains(&[rex, ty, animal]), "rdfs9 transitive");
    assert!(
        set.contains(&[dog, sc, animal]),
        "rdfs11 transitive subclass"
    );
    // New = {rex a Mammal, rex a Animal, Dog ⊑ Animal} and NOTHING else.
    assert_eq!(
        added, 3,
        "exactly three RDFS entailments; got {} -> {:?}",
        added, set
    );
    // Idempotent through the public API.
    assert_eq!(
        materialize(Profile::Rdfs, &mut d, &mut triples),
        0,
        "second call adds nothing"
    );
}

#[test]
fn materialize_owl_rl_via_public_api_runs_symmetry() {
    // Profile::OwlRl through `materialize`: a SymmetricProperty swaps the edge (prp-symp).
    let mut d = Dict::new();
    let (knows, a, b) = (
        iri(&mut d, "http://ex/knows"),
        iri(&mut d, "http://ex/a"),
        iri(&mut d, "http://ex/b"),
    );
    let (ty, sym) = (iri(&mut d, RDF_TYPE), iri(&mut d, OWL_SYM));
    let mut triples = vec![[knows, ty, sym], [a, knows, b]];
    materialize(Profile::OwlRl, &mut d, &mut triples);
    let set: std::collections::HashSet<[Id; 3]> = triples.iter().copied().collect();
    assert!(
        set.contains(&[b, knows, a]),
        "OWL-RL prp-symp via public materialize"
    );
}

#[test]
fn reason_n3_runs_a_forward_rule_to_a_ground_triple() {
    // The headline N3 entry: a forward subClassOf-style rule produces the entailed ground fact,
    // and the rule/variable machinery is consumed (only ground triples survive).
    let mut d = Dict::new();
    let src = "@prefix ex: <http://ex/> .\n\
               ex:socrates a ex:Man .\n\
               { ?x a ex:Man } => { ?x a ex:Mortal } .";
    let triples = reason_n3(&mut d, src).expect("valid n3");
    let want_s = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/socrates",
    )));
    let want_p = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        RDF_TYPE,
    )));
    let want_o = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/Mortal",
    )));
    assert!(
        triples.contains(&[want_s, want_p, want_o]),
        "reason_n3 must derive (socrates a Mortal); closure = {:?}",
        triples
    );
}

#[test]
fn reason_n3_proof_returns_a_step_per_new_triple() {
    // reason_n3_proof returns the same closure PLUS a ProofStep whose conclusion is the
    // derived triple and whose premise is the matched antecedent fact.
    let mut d = Dict::new();
    let src = "@prefix ex: <http://ex/> .\n\
               ex:socrates a ex:Man .\n\
               { ?x a ex:Man } => { ?x a ex:Mortal } .";
    let (triples, steps) = reason_n3_proof(&mut d, src).expect("valid n3");
    let mortal = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/Mortal",
    )));
    let man = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/Man",
    )));
    let socrates = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        "http://ex/socrates",
    )));
    let ty = d.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        RDF_TYPE,
    )));
    assert!(
        triples.contains(&[socrates, ty, mortal]),
        "closure includes the derived fact"
    );
    let step = steps
        .iter()
        .find(|s| s.conclusion == [socrates, ty, mortal])
        .expect("a proof step for (socrates a Mortal)");
    assert_eq!(step.rule, 0, "the only rule (index 0) justified it");
    assert!(
        step.premises.contains(&[socrates, ty, man]),
        "the supporting premise is (socrates a Man); got {:?}",
        step.premises
    );
}

#[test]
fn reason_n3_terms_reports_rule_counts_and_term_level_closure() {
    // reason_n3_terms keeps the closure at the TERM level (no Dict) and reports the document's
    // forward/backward rule counts — the conformance harness's entry shape.
    let src = "@prefix ex: <http://ex/> .\n\
               ex:a ex:p ex:b .\n\
               { ?x ex:p ?y } => { ?y ex:q ?x } .\n\
               { ex:goal ex:reached ex:yes } <= { ex:a ex:p ex:b } .";
    let closure = reason_n3_terms(src, None).expect("valid n3");
    assert_eq!(closure.n_rules, 1, "one forward (=>) rule");
    assert_eq!(closure.n_backward_rules, 1, "one backward (<=) rule");
    // The forward rule derives (b q a).
    use sparq_reason::n3::Term;
    let derived = [
        Term::Iri("http://ex/b".into()),
        Term::Iri("http://ex/q".into()),
        Term::Iri("http://ex/a".into()),
    ];
    assert!(
        closure.facts.contains(&derived),
        "term-level closure contains the forward derivation (b q a); facts = {:?}",
        closure.facts
    );
    // `derived` lists ONLY new conclusions, never the original asserted facts.
    let original = [
        Term::Iri("http://ex/a".into()),
        Term::Iri("http://ex/p".into()),
        Term::Iri("http://ex/b".into()),
    ];
    assert!(
        !closure.derived.contains(&original),
        "derived excludes the asserted base fact"
    );
}

#[test]
fn reason_n3_propagates_a_parse_error() {
    // A malformed document surfaces as an Err from the public entry, not a panic.
    let mut d = Dict::new();
    assert!(
        reason_n3(&mut d, "this is not <valid n3").is_err(),
        "a malformed N3 document is reported as Err"
    );
}

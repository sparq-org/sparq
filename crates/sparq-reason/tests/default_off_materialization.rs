//! Default-feature (explain OFF) forward-chaining correctness + compile-out guard
//! (sq-bif.11).
//!
//! The crate's `explain` feature is NON-DEFAULT by design: when it is off, every
//! explanation structure and hook is `cfg`'d out entirely and the reasoning path must be
//! byte-identical to a build that never knew about explanations. The big differential
//! sweeps in `explain_rdfs.rs` / `explain_owl.rs` / `explain_n3.rs` are all
//! `#![cfg(feature = "explain")]`, so before this file the DEFAULT-OFF reasoning path had
//! no test of its own.
//!
//! This file is deliberately NOT feature-gated: it compiles and runs in BOTH states, and
//! it pins two things the bead calls out:
//!   1. forward-chaining materialization (RDFS + a small OWL-RL fragment) is correct with
//!      `explain` off, and
//!   2. the `explain` surface is gated by the feature flag from BOTH cfg arms (the
//!      `explain_off` / `explain_on` modules below each compile under exactly one arm).
//!
//! It exercises the REAL `materialize` / `materialize_rdfs` / `materialize_owl_rl` /
//! `MaterializedGraph` path, not a mock.

use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};
use sparq_reason::{
    materialize, materialize_owl_rl, materialize_rdfs, MaterializedGraph, Profile,
};

fn ex(dict: &mut Dict, l: &str) -> Id {
    dict.intern_iri(&format!("http://ex/{l}"))
}

fn rdfs_ids(dict: &mut Dict) -> (Id, Id, Id, Id, Id) {
    use oxrdf::vocab::{rdf, rdfs};
    (
        dict.intern_iri(rdf::TYPE.as_str()),
        dict.intern_iri(rdfs::SUB_CLASS_OF.as_str()),
        dict.intern_iri(rdfs::SUB_PROPERTY_OF.as_str()),
        dict.intern_iri(rdfs::DOMAIN.as_str()),
        dict.intern_iri(rdfs::RANGE.as_str()),
    )
}

/// Does the closure contain this triple?
fn has(set: &FxHashSet<[Id; 3]>, s: Id, p: Id, o: Id) -> bool {
    set.contains(&[s, p, o])
}

#[test]
fn rdfs_forward_chaining_correct_default_off() {
    // Dog sc Mammal sc Animal ; rex a Dog. rdfs11 closes the chain; rdfs9 types rex up.
    let mut dict = Dict::new();
    let (ty, sc, _sp, _dom, _rng) = rdfs_ids(&mut dict);
    let (dog, mammal, animal, rex) = (
        ex(&mut dict, "Dog"),
        ex(&mut dict, "Mammal"),
        ex(&mut dict, "Animal"),
        ex(&mut dict, "rex"),
    );
    let mut triples = vec![[dog, sc, mammal], [mammal, sc, animal], [rex, ty, dog]];
    let before = triples.len();
    let added = materialize_rdfs(&mut dict, &mut triples);
    assert!(added > 0, "RDFS closure must add entailed triples");
    assert_eq!(triples.len(), before + added, "added count must match growth");

    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    // rdfs11: (Dog sc Animal) is the transitive-closure schema fact.
    assert!(has(&set, dog, sc, animal), "rdfs11: Dog sc Animal");
    // rdfs9: (rex a Mammal) and (rex a Animal) follow from the chain.
    assert!(has(&set, rex, ty, mammal), "rdfs9: rex a Mammal");
    assert!(has(&set, rex, ty, animal), "rdfs9: rex a Animal");
    // A triple that is NOT entailed must be absent (non-vacuity guard).
    assert!(!has(&set, animal, ty, rex), "non-entailed triple leaked");

    // Idempotence: a second pass adds nothing (the materialization is a fixpoint).
    let again = materialize_rdfs(&mut dict, &mut triples);
    assert_eq!(again, 0, "materialization must be idempotent");
}

#[test]
fn rdfs_domain_range_subproperty_default_off() {
    // p sp q ; q domain C ; q range D ; a p b — rdfs7 lifts p→q, rdfs2/rdfs3 type the
    // endpoints. Exercises the dom/range/subproperty rules, not just subclass.
    let mut dict = Dict::new();
    let (ty, _sc, sp, dom, rng) = rdfs_ids(&mut dict);
    let (p, q, c, d, a, b) = (
        ex(&mut dict, "p"),
        ex(&mut dict, "q"),
        ex(&mut dict, "C"),
        ex(&mut dict, "D"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
    );
    let mut triples = vec![[p, sp, q], [q, dom, c], [q, rng, d], [a, p, b]];
    let added = materialize_rdfs(&mut dict, &mut triples);
    assert!(added > 0);
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    assert!(has(&set, a, q, b), "rdfs7: a q b (p sp q)");
    assert!(has(&set, a, ty, c), "rdfs2: a type C (domain of q)");
    assert!(has(&set, b, ty, d), "rdfs3: b type D (range of q)");
}

#[test]
fn owl_rl_fragment_correct_default_off() {
    // Small OWL-RL fragment: inverseOf, TransitiveProperty, SymmetricProperty. Mirrors the
    // crate's own owl.rs unit test, but asserted through the DEFAULT-OFF public entry point.
    let mut dict = Dict::new();
    let ty = dict.intern_iri(oxrdf::vocab::rdf::TYPE.as_str());
    let owl = |dict: &mut Dict, frag: &str| {
        dict.intern_iri(&format!("http://www.w3.org/2002/07/owl#{frag}"))
    };
    let (parent, child, anc, knows, a, b, c) = (
        ex(&mut dict, "parentOf"),
        ex(&mut dict, "childOf"),
        ex(&mut dict, "ancestorOf"),
        ex(&mut dict, "knows"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
        ex(&mut dict, "c"),
    );
    let inv = owl(&mut dict, "inverseOf");
    let trans = owl(&mut dict, "TransitiveProperty");
    let sym = owl(&mut dict, "SymmetricProperty");
    let mut triples = vec![
        [parent, inv, child], // parentOf inverseOf childOf
        [anc, ty, trans],     // ancestorOf a TransitiveProperty
        [knows, ty, sym],     // knows a SymmetricProperty
        [a, parent, b],
        [a, anc, b],
        [b, anc, c],
        [a, knows, b],
    ];
    // Cross-check the dedicated entry point against the generic dispatcher: identical input
    // must close to the identical triple set (both routes are the supported default-OFF API).
    let mut direct = triples.clone();
    materialize_owl_rl(&mut dict, &mut direct);
    let added = materialize(Profile::OwlRl, &mut dict, &mut triples);
    assert!(added > 0, "OWL-RL closure must add entailed triples");
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let direct_set: FxHashSet<[Id; 3]> = direct.iter().copied().collect();
    assert_eq!(set, direct_set, "materialize(OwlRl) must match materialize_owl_rl");

    assert!(has(&set, b, child, a), "prp-inv: b childOf a");
    assert!(has(&set, a, anc, c), "prp-trp: a ancestorOf c");
    assert!(has(&set, b, knows, a), "prp-symp: b knows a");
    // A transitive property must NOT run backwards (non-vacuity guard).
    assert!(!has(&set, c, anc, a), "transitive must not run backwards");

    // Idempotence via the generic entry point.
    assert_eq!(materialize(Profile::OwlRl, &mut dict, &mut triples), 0);
}

#[test]
fn materialized_graph_incremental_correct_default_off() {
    // The incremental MaterializedGraph handle is available in BOTH feature states (only
    // its `why()` explanation method is explain-gated). Confirm insert/delete maintenance
    // is correct with explain OFF.
    let mut dict = Dict::new();
    let (ty, sc, _sp, _dom, _rng) = rdfs_ids(&mut dict);
    let (dog, mammal, animal, rex, fido) = (
        ex(&mut dict, "Dog"),
        ex(&mut dict, "Mammal"),
        ex(&mut dict, "Animal"),
        ex(&mut dict, "rex"),
        ex(&mut dict, "fido"),
    );
    let mut g = MaterializedGraph::new(
        &mut dict,
        &[[dog, sc, mammal], [mammal, sc, animal], [rex, ty, dog]],
    );
    assert!(g.contains(&[rex, ty, animal]), "initial closure types rex up");

    // Insert a second individual: its typing must close up the same chain.
    g.insert(&[[fido, ty, dog]]);
    assert!(g.contains(&[fido, ty, animal]), "insert maintains the closure");

    // Delete the only support of (rex a animal): it must leave the closure.
    g.delete(&[[rex, ty, dog]]);
    assert!(!g.contains(&[rex, ty, animal]), "delete retracts the entailment");
    // fido's typing is independent and must survive.
    assert!(g.contains(&[fido, ty, animal]), "unrelated entailment survives the delete");
    // The schema chain is untouched by an ABox delete.
    assert!(g.contains(&[dog, sc, animal]), "schema closure intact");
}

/// Compile-out guard (default arm): with `explain` OFF this module compiles and uses ONLY
/// the materialization surface — it names NO explain symbol. It is the supported
/// default-OFF API. Compiling only under `not(feature = "explain")` pins the default arm.
#[cfg(not(feature = "explain"))]
mod explain_off {
    use sparq_core::dict::Dict;
    use sparq_reason::MaterializedGraph;

    #[test]
    fn graph_usable_without_explain_symbols() {
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/a");
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let (cc, dd) = (dict.intern_iri("http://ex/C"), dict.intern_iri("http://ex/D"));
        let g = MaterializedGraph::new(&mut dict, &[[cc, sc, dd], [a, ty, cc]]);
        assert!(g.contains(&[a, ty, dd]));
        // Sanity: the closure is non-trivial (base 2 + the derived typing).
        assert!(g.len() >= 3);
    }
}

/// Symmetric guard (feature arm): with `explain` ON the `why()` method IS present and
/// returns a proof for a derived triple. Compiling this only under the feature pins that
/// the gate flips the surface on, complementing `explain_off` above (so the two together
/// cover BOTH cfg arms).
#[cfg(feature = "explain")]
mod explain_on {
    use sparq_core::dict::Dict;
    use sparq_reason::MaterializedGraph;

    #[test]
    fn why_present_when_feature_on() {
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/a");
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let (cc, dd) = (dict.intern_iri("http://ex/C"), dict.intern_iri("http://ex/D"));
        let g = MaterializedGraph::new(&mut dict, &[[cc, sc, dd], [a, ty, cc]]);
        let tree = g.why(&dict, [a, ty, dd]).expect("derived triple must explain");
        assert!(!tree.nodes().is_empty());
        assert_eq!(tree.conclusion()[1], dict.term(ty).to_string());
    }
}

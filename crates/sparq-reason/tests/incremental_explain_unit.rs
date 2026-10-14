//! Focused, ISOLATED unit tests for `src/incremental_explain.rs` — the
//! delta-with-explanation path (sq-bif.11).
//!
//! Before this file `incremental_explain.rs` (~1188 LOC) had no focused coverage: it was
//! exercised only transitively by the big randomized differential sweeps in
//! `explain_rdfs.rs` / `explain_owl.rs` / `explain_n3.rs`. Those sweeps prove the prover is
//! sound over large random closures, but they do NOT isolate the incremental
//! delta-WITH-explanation behaviour: `insert` a fact and explain its newly-derived
//! consequence; `delete` a support and watch the explanation update (drop the retracted
//! leaf, find an alternative support, or report the triple gone).
//!
//! These are small, hand-checked fixtures over the REAL `why()` / `why_with()` provers on
//! all three maintenance handles (RDFS, OWL, N3), plus the `ExplainOpts` cap surface. They
//! assert structural proof invariants directly (leaves currently asserted, conclusion
//! correct, premises-before-conclusion, retracted leaf absent) rather than reusing the
//! heavyweight naive oracle, keeping each test independent and fast.

#![cfg(feature = "explain")]

use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};
use sparq_reason::n3::Term;
use sparq_reason::{
    ExplainOpts, MaterializedGraph, MaterializedN3Graph, MaterializedOwlGraph, N3Mode, OwlMode,
    ProofTree,
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

fn render(dict: &Dict, t: [Id; 3]) -> [String; 3] {
    [dict.term(t[0]).to_string(), dict.term(t[1]).to_string(), dict.term(t[2]).to_string()]
}

/// Lightweight structural invariants every well-formed [`ProofTree`] must satisfy,
/// independent of the rule table: non-empty, root last, premises strictly before their
/// node, exactly one root reachable, and every conclusion well-formed (3 non-empty terms).
fn assert_structurally_sound(tree: &ProofTree, expect_conclusion: &[String; 3]) {
    let nodes = tree.nodes();
    assert!(!nodes.is_empty(), "empty proof");
    assert_eq!(tree.root() as usize, nodes.len() - 1, "root must be the last node");
    assert_eq!(tree.conclusion(), expect_conclusion, "wrong conclusion");
    for (i, n) in nodes.iter().enumerate() {
        for &p in &n.premises {
            assert!((p as usize) < i, "node {i}: premise {p} not strictly before it");
        }
        assert!(!n.rule.is_empty(), "node {i}: empty rule label");
        for term in &n.conclusion {
            assert!(!term.is_empty(), "node {i}: empty term in conclusion");
        }
        if n.rule == "asserted" {
            assert!(n.premises.is_empty(), "node {i}: asserted leaf with premises");
        }
    }
}

/// The set of `asserted`-leaf conclusions of a proof (the facts the proof actually leans on
/// from the base; axioms are not base facts).
fn asserted_leaves(tree: &ProofTree) -> Vec<[String; 3]> {
    tree.nodes()
        .iter()
        .filter(|n| n.rule == "asserted")
        .map(|n| n.conclusion.clone())
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════════════════
// RDFS — MaterializedGraph: delta-with-explanation
// ════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn rdfs_insert_then_explain_new_derivation() {
    // Base schema only (Dog sc Mammal sc Animal). Insert the ABox fact (rex a Dog) and
    // explain the NEWLY-derived (rex a Animal): the proof must be sound, conclude the right
    // triple, and have a top rule of rdfs9 (typing up the subclass chain).
    let mut dict = Dict::new();
    let (ty, sc, _sp, _dom, _rng) = rdfs_ids(&mut dict);
    let (dog, mammal, animal, rex) = (
        ex(&mut dict, "Dog"),
        ex(&mut dict, "Mammal"),
        ex(&mut dict, "Animal"),
        ex(&mut dict, "rex"),
    );
    let mut g = MaterializedGraph::new(&mut dict, &[[dog, sc, mammal], [mammal, sc, animal]]);
    // Before the insert the typing is not in the closure and has no proof.
    assert!(!g.contains(&[rex, ty, animal]));
    assert!(g.why(&dict, [rex, ty, animal]).is_none(), "no proof before the fact exists");

    let added = g.insert(&[[rex, ty, dog]]);
    assert!(added > 0, "insert must add the asserted fact + its consequences");
    assert!(g.contains(&[rex, ty, animal]), "incremental closure types rex up");

    let t = [rex, ty, animal];
    let tree = g.why(&dict, t).expect("newly-derived triple must explain");
    assert_structurally_sound(&tree, &render(&dict, t));
    assert_eq!(
        tree.nodes()[tree.root() as usize].rule,
        "rdfs9",
        "the typing-up step is rdfs9"
    );
    // Every asserted leaf is CURRENTLY in the base (the consistency model).
    let base: FxHashSet<[String; 3]> = g.base_triples().map(|b| render(&dict, b)).collect();
    for leaf in asserted_leaves(&tree) {
        assert!(base.contains(&leaf), "proof leaf {leaf:?} not in the current base");
    }
    // The just-inserted fact is one of those leaves.
    assert!(asserted_leaves(&tree).contains(&render(&dict, [rex, ty, dog])));
}

#[test]
fn rdfs_retract_updates_explanation_to_alternative_support() {
    // Two independent supports for (a type C): p sp q, q dom C, a p b, a q b. Retracting one
    // support must keep (a type C) derivable and the explanation must NOT cite the retracted
    // leaf; retracting BOTH must drop the triple and its proof.
    let mut dict = Dict::new();
    let (ty, _sc, sp, dom, _rng) = rdfs_ids(&mut dict);
    let (p, q, c, a, b) = (
        ex(&mut dict, "p"),
        ex(&mut dict, "q"),
        ex(&mut dict, "C"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
    );
    let mut g = MaterializedGraph::new(
        &mut dict,
        &[[p, sp, q], [q, dom, c], [a, p, b], [a, q, b]],
    );
    let t = [a, ty, c];
    assert!(g.contains(&t));
    let tree0 = g.why(&dict, t).expect("initial proof");
    assert_structurally_sound(&tree0, &render(&dict, t));

    // Retract (a p b): the triple survives via (a q b); the proof must not lean on (a p b).
    g.delete(&[[a, p, b]]);
    assert!(g.contains(&t), "alternative support keeps the triple");
    let tree1 = g.why(&dict, t).expect("surviving support yields a proof");
    assert_structurally_sound(&tree1, &render(&dict, t));
    let retracted = render(&dict, [a, p, b]);
    assert!(
        tree1.nodes().iter().all(|n| n.conclusion != retracted),
        "explanation still cites the retracted (a p b)"
    );
    let base: FxHashSet<[String; 3]> = g.base_triples().map(|x| render(&dict, x)).collect();
    for leaf in asserted_leaves(&tree1) {
        assert!(base.contains(&leaf), "stale leaf {leaf:?} after retraction");
    }

    // Retract the second support: triple leaves the closure, no proof.
    g.delete(&[[a, q, b]]);
    assert!(!g.contains(&t));
    assert!(g.why(&dict, t).is_none(), "no proof for a triple that left the closure");

    // Re-insert: support and proof return.
    g.insert(&[[a, q, b]]);
    assert!(g.contains(&t));
    let tree2 = g.why(&dict, t).expect("re-inserted support restores the proof");
    assert_structurally_sound(&tree2, &render(&dict, t));
}

#[test]
fn rdfs_why_respects_node_cap() {
    // A long subclass chain forces a multi-node proof; a tight max_nodes cap must make
    // why_with abort (return None) rather than exceed it, while the default caps succeed.
    let mut dict = Dict::new();
    let (ty, sc, _sp, _dom, _rng) = rdfs_ids(&mut dict);
    let chain: Vec<Id> = (0..8).map(|i| ex(&mut dict, &format!("C{i}"))).collect();
    let rex = ex(&mut dict, "rex");
    let mut base: Vec<[Id; 3]> = chain.windows(2).map(|w| [w[0], sc, w[1]]).collect();
    base.push([rex, ty, chain[0]]);
    let g = MaterializedGraph::new(&mut dict, &base);
    let top = [rex, ty, chain[chain.len() - 1]];
    assert!(g.contains(&top));

    // Default caps: a proof exists.
    let full = g.why(&dict, top).expect("default caps prove the deep chain");
    assert!(full.nodes().len() > 2, "deep chain should need several nodes");
    // Tight node cap: aborts to None.
    let capped = g.why_with(&dict, top, ExplainOpts { max_depth: 128, max_nodes: 2 });
    assert!(capped.is_none(), "max_nodes=2 must abort the deep proof");
    // Tight depth cap: also aborts.
    let depth_capped = g.why_with(&dict, top, ExplainOpts { max_depth: 1, max_nodes: 65_536 });
    assert!(depth_capped.is_none(), "max_depth=1 must abort the deep proof");
}

// ════════════════════════════════════════════════════════════════════════════════════════
// OWL — MaterializedOwlGraph: delta-with-explanation in both counting modes
// ════════════════════════════════════════════════════════════════════════════════════════

fn owl_iri(dict: &mut Dict, frag: &str) -> Id {
    dict.intern_iri(&format!("http://www.w3.org/2002/07/owl#{frag}"))
}

#[test]
fn owl_mono_insert_then_explain_inverse() {
    // CountingMono fixture (inverseOf, no transitive property). Insert (a parentOf b) and
    // explain the prp-inv consequence (b childOf a).
    let mut dict = Dict::new();
    let (parent, child, a, b) = (
        ex(&mut dict, "parentOf"),
        ex(&mut dict, "childOf"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
    );
    let inv = owl_iri(&mut dict, "inverseOf");
    let mut g = MaterializedOwlGraph::new(&mut dict, &[[parent, inv, child]]);
    assert_eq!(g.mode(), OwlMode::CountingMono, "no transitive prop => mono mode");
    assert!(!g.contains(&[b, child, a]), "nothing derived before the edge exists");

    g.insert(&mut dict, &[[a, parent, b]]);
    let t = [b, child, a];
    assert!(g.contains(&t), "prp-inv fires incrementally");
    let tree = g.why(&dict, t).expect("inverse consequence must explain");
    assert_structurally_sound(&tree, &render(&dict, t));
    // The proof's asserted leaves are all currently in the base.
    let base: FxHashSet<[String; 3]> = g.base_triples().map(|x| render(&dict, x)).collect();
    for leaf in asserted_leaves(&tree) {
        assert!(base.contains(&leaf), "stale OWL leaf {leaf:?}");
    }
    assert!(asserted_leaves(&tree).contains(&render(&dict, [a, parent, b])));
}

#[test]
fn owl_fixpoint_transitive_insert_retract_explanation() {
    // CountingFixpoint fixture (ancestorOf a TransitiveProperty). Inserting the bridging
    // edge derives (a ancestorOf c) via prp-trp; retracting it removes the derivation.
    let mut dict = Dict::new();
    let ty = dict.intern_iri(oxrdf::vocab::rdf::TYPE.as_str());
    let (anc, a, b, c) = (
        ex(&mut dict, "ancestorOf"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
        ex(&mut dict, "c"),
    );
    let trans = owl_iri(&mut dict, "TransitiveProperty");
    let mut g = MaterializedOwlGraph::new(
        &mut dict,
        &[[anc, ty, trans], [a, anc, b]],
    );
    assert_eq!(g.mode(), OwlMode::CountingFixpoint, "transitive prop => fixpoint mode");
    let derived = [a, anc, c];
    assert!(!g.contains(&derived), "no bridge yet");

    // Insert the bridge (b ancestorOf c): prp-trp now derives (a ancestorOf c).
    g.insert(&mut dict, &[[b, anc, c]]);
    assert!(g.contains(&derived), "prp-trp derives the transitive edge");
    let tree = g.why(&dict, derived).expect("transitive consequence must explain");
    assert_structurally_sound(&tree, &render(&dict, derived));
    let base: FxHashSet<[String; 3]> = g.base_triples().map(|x| render(&dict, x)).collect();
    for leaf in asserted_leaves(&tree) {
        assert!(base.contains(&leaf), "stale transitive leaf {leaf:?}");
    }

    // Retract the bridge: the transitive edge and its explanation disappear.
    g.delete(&mut dict, &[[b, anc, c]]);
    assert!(!g.contains(&derived), "retracting the bridge removes the transitive edge");
    assert!(g.why(&dict, derived).is_none(), "no proof once the bridge is gone");
}

// ════════════════════════════════════════════════════════════════════════════════════════
// N3 — MaterializedN3Graph: delta-with-explanation (re-derive-per-call prover)
// ════════════════════════════════════════════════════════════════════════════════════════

const N3_RULES: &str = r#"
@prefix : <http://ex/> .
{ ?x :parent ?y } => { ?x :ancestor ?y } .
{ ?x :ancestor ?y . ?y :ancestor ?z } => { ?x :ancestor ?z } .
"#;

fn n3_ex(local: &str) -> Term {
    Term::Iri(format!("http://ex/{local}"))
}

fn n3_render(f: &[Term; 3]) -> [String; 3] {
    f.clone().map(|t| match t {
        Term::Iri(i) => format!("<{i}>"),
        other => panic!("unexpected term in fixture: {other:?}"),
    })
}

#[test]
fn n3_insert_then_explain_transitive_chain() {
    // a parent b ; b parent c. Insert and explain the transitively-derived (a ancestor c):
    // its asserted leaves must all be currently-asserted base facts, and the conclusion the
    // ancestor edge.
    let base = vec![[n3_ex("a"), n3_ex("parent"), n3_ex("b")]];
    let mut g = MaterializedN3Graph::new(N3_RULES, &base).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Counting);
    let target = [n3_ex("a"), n3_ex("ancestor"), n3_ex("c")];
    assert!(!g.contains(&target), "no chain to c yet");

    g.insert(&[[n3_ex("b"), n3_ex("parent"), n3_ex("c")]]);
    assert!(g.contains(&target), "the transitive ancestor edge is derived after the insert");
    let tree = g.why(&target).expect("derived N3 fact must explain");
    assert_structurally_sound(&tree, &n3_render(&target));
    // `MaterializedN3Graph` exposes no base iterator, but every asserted leaf is necessarily
    // also in the closure; check that, and that the just-inserted bridge is one of them.
    let closure_set: FxHashSet<[String; 3]> =
        g.closure().iter().map(n3_render).collect();
    for leaf in asserted_leaves(&tree) {
        assert!(closure_set.contains(&leaf), "stale N3 leaf {leaf:?}");
    }
    assert!(
        asserted_leaves(&tree).contains(&n3_render(&[n3_ex("b"), n3_ex("parent"), n3_ex("c")])),
        "the inserted bridge fact must be an asserted leaf of the proof"
    );
    // The root rule is one of the document's forward rules.
    assert!(
        tree.nodes()[tree.root() as usize].rule.starts_with("n3-rule-"),
        "derived N3 fact's top step names a rule"
    );
}

#[test]
fn n3_retract_drops_explanation() {
    // a parent b ; b parent c derives a ancestor c. Retract (b parent c): the chain (and its
    // explanation) to c is gone; the direct (a ancestor b) survives.
    let base = vec![
        [n3_ex("a"), n3_ex("parent"), n3_ex("b")],
        [n3_ex("b"), n3_ex("parent"), n3_ex("c")],
    ];
    let mut g = MaterializedN3Graph::new(N3_RULES, &base).expect("rules parse");
    let to_c = [n3_ex("a"), n3_ex("ancestor"), n3_ex("c")];
    let to_b = [n3_ex("a"), n3_ex("ancestor"), n3_ex("b")];
    assert!(g.contains(&to_c));
    assert!(g.why(&to_c).is_some(), "chain to c explains before the retraction");

    g.delete(&[[n3_ex("b"), n3_ex("parent"), n3_ex("c")]]);
    assert!(!g.contains(&to_c), "no path to c after retracting the bridge");
    assert!(g.why(&to_c).is_none(), "no explanation for a fact that left the closure");
    // The independent (a ancestor b) survives and still explains.
    assert!(g.contains(&to_b));
    let tree = g.why(&to_b).expect("surviving fact still explains");
    assert_structurally_sound(&tree, &n3_render(&to_b));
}

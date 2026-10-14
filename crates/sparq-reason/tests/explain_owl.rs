//! `why()` on [`MaterializedOwlGraph`] (OWL 2 RL counting modes, `explain` feature):
//! hand-checked proof trees for the OWL-specific rules (prp-trp, prp-inv, prp-symp,
//! equivalences, closed domain/range), a differential sweep over randomized
//! materializations in BOTH counting modes, and incremental-retraction consistency.
#![cfg(feature = "explain")]

mod explain_common;

use explain_common::{check_proof, proof_leaves};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use sparq_reason::{materialize_owl_rl, MaterializedOwlGraph, OwlMode, ProofTree};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

struct V {
    ty: Id,
    sc: Id,
    sp: Id,
    dom: Id,
    rng: Id,
    inv: Id,
    symmetric: Id,
    transitive: Id,
    eqc: Id,
    eqp: Id,
    diff: Id,
}

fn vocab(dict: &mut Dict) -> V {
    use oxrdf::vocab::{rdf, rdfs};
    let owl = "http://www.w3.org/2002/07/owl#";
    V {
        ty: dict.intern_iri(rdf::TYPE.as_str()),
        sc: dict.intern_iri(rdfs::SUB_CLASS_OF.as_str()),
        sp: dict.intern_iri(rdfs::SUB_PROPERTY_OF.as_str()),
        dom: dict.intern_iri(rdfs::DOMAIN.as_str()),
        rng: dict.intern_iri(rdfs::RANGE.as_str()),
        inv: dict.intern_iri(&format!("{owl}inverseOf")),
        symmetric: dict.intern_iri(&format!("{owl}SymmetricProperty")),
        transitive: dict.intern_iri(&format!("{owl}TransitiveProperty")),
        eqc: dict.intern_iri(&format!("{owl}equivalentClass")),
        eqp: dict.intern_iri(&format!("{owl}equivalentProperty")),
        diff: dict.intern_iri(&format!("{owl}differentFrom")),
    }
}

fn ex(dict: &mut Dict, l: &str) -> Id {
    dict.intern_iri(&format!("http://ex/{l}"))
}

fn render(dict: &Dict, t: [Id; 3]) -> [String; 3] {
    [dict.term(t[0]).to_string(), dict.term(t[1]).to_string(), dict.term(t[2]).to_string()]
}

fn rendered_base(dict: &Dict, g: &MaterializedOwlGraph) -> FxHashSet<[String; 3]> {
    g.base_triples().map(|t| render(dict, t)).collect()
}

/// Strong validity oracle: the proof's asserted leaves (plus any axiom-leaf conclusions —
/// they are tautologies) must alone re-entail the conclusion under `materialize_owl_rl`.
fn leaves_entail(dict: &mut Dict, g: &MaterializedOwlGraph, tree: &ProofTree, t: [Id; 3]) {
    let rendered: FxHashMap<[String; 3], [Id; 3]> =
        g.closure().into_iter().map(|b| (render(dict, b), b)).collect();
    let mut input: Vec<[Id; 3]> = proof_leaves(tree)
        .iter()
        .map(|l| *rendered.get(l).expect("leaf not renderable from the closure"))
        .collect();
    for n in tree.nodes() {
        if n.rule.starts_with("axiom-") {
            input.push(*rendered.get(&n.conclusion).expect("axiom conclusion not in closure"));
        }
    }
    if g.mode() == OwlMode::CountingFixpoint {
        // Keep the re-run on the engine's FIXPOINT route (the graph's own route): the
        // domain/range inverse-transposition (inv-dom/inv-rng) only runs there, so a leaf
        // subset without a transitive property would otherwise under-derive. The fresh
        // property adds no other entailments.
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let trans = dict.intern_iri("http://www.w3.org/2002/07/owl#TransitiveProperty");
        let dummy = dict.intern_iri("http://ex/__route_pin__");
        input.push([dummy, ty, trans]);
    }
    materialize_owl_rl(dict, &mut input);
    assert!(
        input.contains(&t),
        "proof leaves do not re-entail the conclusion {:?}\nproof:\n{}",
        render(dict, t),
        tree.to_text()
    );
}

/// Sweep every closure triple: a proof must exist, satisfy the naive checker, and (for
/// derived triples) re-entail from its leaves alone. Returns the number of derived triples.
fn sweep(dict: &mut Dict, g: &MaterializedOwlGraph, ctx: &str) -> usize {
    let rendered = rendered_base(dict, g);
    let base: FxHashSet<[Id; 3]> = g.base_triples().collect();
    let mut derived = 0usize;
    for t in g.closure() {
        let tree = g
            .why(dict, t)
            .unwrap_or_else(|| panic!("{ctx}: no proof for closure triple {:?}", render(dict, t)));
        check_proof(&tree, &rendered, Some(&render(dict, t)));
        if !base.contains(&t) {
            leaves_entail(dict, g, &tree, t);
            derived += 1;
        }
    }
    derived
}

#[test]
fn transitive_property_hand_checked() {
    // parent ⊑ ancestor (transitive): a parent b, b parent c, c parent d ⊢ a ancestor d via
    // prp-trp over prp-spo1-lifted edges.
    let mut dict = Dict::new();
    let v = vocab(&mut dict);
    let (anc, par, a, b, c, d) = (
        ex(&mut dict, "ancestorOf"),
        ex(&mut dict, "parentOf"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
        ex(&mut dict, "c"),
        ex(&mut dict, "d"),
    );
    let g = MaterializedOwlGraph::new(
        &mut dict,
        &[
            [anc, v.ty, v.transitive],
            [par, v.sp, anc],
            [a, par, b],
            [b, par, c],
            [c, par, d],
        ],
    );
    assert_eq!(g.mode(), OwlMode::CountingFixpoint);
    let t = [a, anc, d];
    assert!(g.contains(&t));
    let tree = g.why(&dict, t).expect("transitive closure fact must have a proof");
    check_proof(&tree, &rendered_base(&dict, &g), Some(&render(&dict, t)));
    leaves_entail(&mut dict, &g, &tree, t);
    let root = &tree.nodes()[tree.root() as usize];
    assert_eq!(root.rule, "prp-trp");
    // Premise 0 is the TransitiveProperty typing; the data premises chain through prp-trp /
    // prp-spo1 down to asserted parent edges.
    assert_eq!(
        tree.nodes()[root.premises[0] as usize].conclusion,
        render(&dict, [anc, v.ty, v.transitive])
    );
    assert!(tree
        .nodes()
        .iter()
        .any(|n| n.rule == "prp-spo1"), "lifted edges must appear as prp-spo1 steps");

    // Each one-hop lift is also explained.
    let t1 = [a, anc, b];
    let tree1 = g.why(&dict, t1).unwrap();
    check_proof(&tree1, &rendered_base(&dict, &g), Some(&render(&dict, t1)));
    assert_eq!(tree1.nodes()[tree1.root() as usize].rule, "prp-spo1");
}

#[test]
fn inverse_symmetric_equivalence_hand_checked() {
    let mut dict = Dict::new();
    let v = vocab(&mut dict);
    let (hp, po, touches, ca, cb, x, y) = (
        ex(&mut dict, "hasPart"),
        ex(&mut dict, "partOf"),
        ex(&mut dict, "touches"),
        ex(&mut dict, "A"),
        ex(&mut dict, "B"),
        ex(&mut dict, "x"),
        ex(&mut dict, "y"),
    );
    let g = MaterializedOwlGraph::new(
        &mut dict,
        &[
            [hp, v.inv, po],
            [po, v.rng, ca],
            [touches, v.ty, v.symmetric],
            [ca, v.eqc, cb],
            [x, hp, y],
            [x, touches, y],
        ],
    );
    assert_eq!(g.mode(), OwlMode::CountingMono);
    let base = rendered_base(&dict, &g);

    // prp-inv1: x hasPart y ⊢ y partOf x.
    let t = [y, po, x];
    let tree = g.why(&dict, t).expect("inverse edge");
    check_proof(&tree, &base, Some(&render(&dict, t)));
    assert_eq!(tree.nodes()[tree.root() as usize].rule, "prp-inv1");

    // prp-symp: x touches y ⊢ y touches x.
    let t = [y, touches, x];
    let tree = g.why(&dict, t).expect("symmetric edge");
    check_proof(&tree, &base, Some(&render(&dict, t)));
    assert_eq!(tree.nodes()[tree.root() as usize].rule, "prp-symp");

    // Typing through the inverse-transposed domain: x hasPart y + (partOf range A) gives
    // x type A only via the engine's inv-dom transposition... (hasPart domain A).
    let t = [x, v.ty, ca];
    assert!(g.contains(&t), "inverse-transposed typing expected");
    let tree = g.why(&dict, t).expect("inverse-transposed typing");
    check_proof(&tree, &base, Some(&render(&dict, t)));
    leaves_entail(&mut dict, &g, &tree, t);

    // equivalentClass: A ≡ B lifts the typing to B (cax-sco over the scm-eqc1 edge), and
    // the subclass edges themselves explain via scm-eqc1.
    let t = [x, v.ty, cb];
    assert!(g.contains(&t));
    let tree = g.why(&dict, t).expect("typing through equivalence");
    check_proof(&tree, &base, Some(&render(&dict, t)));
    assert!(tree.nodes().iter().any(|n| n.rule == "scm-eqc1"));
    let t = [cb, v.sc, ca];
    let tree = g.why(&dict, t).expect("folded subclass edge");
    check_proof(&tree, &base, Some(&render(&dict, t)));

    // scm-eqc2: the mutual-subsumption-derived equivalence (B ≡ A, the mirror).
    let t = [cb, v.eqc, ca];
    assert!(g.contains(&t));
    let tree = g.why(&dict, t).expect("post-equivalence");
    check_proof(&tree, &base, Some(&render(&dict, t)));
    assert_eq!(tree.nodes()[tree.root() as usize].rule, "scm-eqc2");

    // sym-dif sanity (engine rule).
    let mut dict2 = Dict::new();
    let v2 = vocab(&mut dict2);
    let (m, n) = (ex(&mut dict2, "m"), ex(&mut dict2, "n"));
    let g2 = MaterializedOwlGraph::new(&mut dict2, &[[m, v2.diff, n]]);
    let t = [n, v2.diff, m];
    assert!(g2.contains(&t));
    let tree = g2.why(&dict2, t).expect("differentFrom mirror");
    check_proof(&tree, &rendered_base(&dict2, &g2), Some(&render(&dict2, t)));
    assert_eq!(tree.nodes()[tree.root() as usize].rule, "sym-dif");
}

/// Randomized ontology exercising every counting-mode feature (mirrors
/// tests/incremental_owl_prop.rs).
// test fixture builder: the 5-tuple is an ad-hoc bundle of the generated
// vocab + base triples + the three id pools the assertions index into; a named struct
// would add ceremony for a test-only helper, so the lint is allowed locally.
#[allow(clippy::type_complexity)]
fn build(dict: &mut Dict, rng: &mut Rng, with_transitive: bool) -> (V, Vec<[Id; 3]>, Vec<Id>, Vec<Id>, Vec<Id>) {
    let v = vocab(dict);
    let mut base: Vec<[Id; 3]> = Vec::new();
    let mut classes = Vec::new();
    for i in 0..4 {
        let chain: Vec<Id> =
            (0..5).map(|j| dict.intern_iri(&format!("http://ex/C{i}_{j}"))).collect();
        for w in chain.windows(2) {
            base.push([w[0], v.sc, w[1]]);
        }
        classes.extend(chain);
    }
    base.push([classes[2], v.eqc, classes[7]]);
    let mut props = Vec::new();
    for i in 0..3 {
        let chain: Vec<Id> =
            (0..3).map(|j| dict.intern_iri(&format!("http://ex/p{i}_{j}"))).collect();
        for w in chain.windows(2) {
            base.push([w[0], v.sp, w[1]]);
        }
        for &p in &chain {
            base.push([p, v.dom, classes[rng.below(classes.len())]]);
            base.push([p, v.rng, classes[rng.below(classes.len())]]);
        }
        props.extend(chain);
    }
    base.push([props[1], v.eqp, props[4]]);
    let has_part = dict.intern_iri("http://ex/hasPart");
    let part_of = dict.intern_iri("http://ex/partOf");
    base.push([has_part, v.inv, part_of]);
    base.push([has_part, v.dom, classes[0]]);
    base.push([part_of, v.rng, classes[5]]);
    props.push(has_part);
    props.push(part_of);
    let touches = dict.intern_iri("http://ex/touches");
    base.push([touches, v.ty, v.symmetric]);
    props.push(touches);
    if with_transitive {
        let ancestor = dict.intern_iri("http://ex/ancestorOf");
        let parent = dict.intern_iri("http://ex/parentOf");
        base.push([ancestor, v.ty, v.transitive]);
        base.push([parent, v.sp, ancestor]);
        base.push([ancestor, v.dom, classes[10]]);
        let connected = dict.intern_iri("http://ex/connectedTo");
        base.push([connected, v.ty, v.transitive]);
        base.push([connected, v.ty, v.symmetric]);
        props.push(ancestor);
        props.push(parent);
        props.push(connected);
    }
    let individuals: Vec<Id> =
        (0..60).map(|i| dict.intern_iri(&format!("http://ex/ind{i}"))).collect();
    for &s in &individuals {
        base.push([s, v.ty, classes[rng.below(classes.len())]]);
        for _ in 0..2 {
            let p = props[rng.below(props.len())];
            base.push([s, p, individuals[rng.below(individuals.len())]]);
        }
    }
    for _ in 0..6 {
        let a = individuals[rng.below(individuals.len())];
        let b = individuals[rng.below(individuals.len())];
        base.push([a, v.diff, b]);
    }
    (v, base, classes, props, individuals)
}

#[test]
fn differential_counting_mono() {
    let mut rng = Rng(0x0D1F_F0A1);
    let mut dict = Dict::new();
    let (_, base, _, _, _) = build(&mut dict, &mut rng, false);
    let g = MaterializedOwlGraph::new(&mut dict, &base);
    assert_eq!(g.mode(), OwlMode::CountingMono);
    let derived = sweep(&mut dict, &g, "mono");
    assert!(derived > 300, "mono sweep too small: {derived}");
}

#[test]
fn differential_counting_fixpoint() {
    let mut rng = Rng(0xF1A9_0001);
    let mut dict = Dict::new();
    let (_, base, _, _, _) = build(&mut dict, &mut rng, true);
    let g = MaterializedOwlGraph::new(&mut dict, &base);
    assert_eq!(g.mode(), OwlMode::CountingFixpoint);
    let derived = sweep(&mut dict, &g, "fixpoint");
    assert!(derived > 300, "fixpoint sweep too small: {derived}");
    // Determinism spot-check.
    for t in g.closure().into_iter().take(40) {
        assert_eq!(g.why(&dict, t).unwrap().to_json(), g.why(&dict, t).unwrap().to_json());
    }
}

#[test]
fn retraction_transitive_chain() {
    // a-b-c-d ancestor chain; retract the middle edge: long-range closure facts disappear
    // (why → None), surviving ones still explain without citing the retracted edge.
    let mut dict = Dict::new();
    let v = vocab(&mut dict);
    let (anc, a, b, c, d) = (
        ex(&mut dict, "ancestorOf"),
        ex(&mut dict, "a"),
        ex(&mut dict, "b"),
        ex(&mut dict, "c"),
        ex(&mut dict, "d"),
    );
    let mut g = MaterializedOwlGraph::new(
        &mut dict,
        &[[anc, v.ty, v.transitive], [a, anc, b], [b, anc, c], [c, anc, d]],
    );
    let t = [a, anc, d];
    assert!(g.contains(&t));
    check_proof(&g.why(&dict, t).unwrap(), &rendered_base(&dict, &g), Some(&render(&dict, t)));

    g.delete(&mut dict, &[[b, anc, c]]);
    assert!(!g.contains(&t), "chain broken");
    assert!(g.why(&dict, t).is_none(), "no proof after retraction");
    assert!(g.why(&dict, [a, anc, c]).is_none());
    // (c ancestor d) is still asserted and explains as a leaf.
    let tree = g.why(&dict, [c, anc, d]).unwrap();
    assert_eq!(tree.nodes()[0].rule, "asserted");

    // Restore + add a parallel path; retract one of two supports: proof survives on the other.
    g.insert(&mut dict, &[[b, anc, c]]);
    let e = ex(&mut dict, "e");
    g.insert(&mut dict, &[[a, anc, e], [e, anc, d]]);
    assert!(g.contains(&t));
    g.delete(&mut dict, &[[b, anc, c]]);
    assert!(g.contains(&t), "second path keeps a→d");
    let tree = g.why(&dict, t).expect("alternative support");
    check_proof(&tree, &rendered_base(&dict, &g), Some(&render(&dict, t)));
    let dead = render(&dict, [b, anc, c]);
    assert!(tree.nodes().iter().all(|n| n.conclusion != dead), "proof cites retracted edge");
}

#[test]
fn retraction_differential_random_edits() {
    let mut rng = Rng(0xD3AD_2026);
    let mut dict = Dict::new();
    let (v, base, classes, props, individuals) = build(&mut dict, &mut rng, true);
    let mut g = MaterializedOwlGraph::new(&mut dict, &base);
    assert_eq!(g.mode(), OwlMode::CountingFixpoint);
    let abox: Vec<[Id; 3]> = base
        .iter()
        .filter(|t| {
            !(t[1] == v.sc
                || t[1] == v.sp
                || t[1] == v.dom
                || t[1] == v.rng
                || t[1] == v.inv
                || t[1] == v.eqc
                || t[1] == v.eqp
                || (t[1] == v.ty && (t[2] == v.symmetric || t[2] == v.transitive)))
        })
        .copied()
        .collect();
    let _ = (classes, props, individuals);
    for round in 0..4 {
        let dels: Vec<[Id; 3]> = (0..6).map(|_| abox[rng.below(abox.len())]).collect();
        g.delete(&mut dict, &dels);
        let rendered = rendered_base(&dict, &g);
        for t in g.closure() {
            let tree = g
                .why(&dict, t)
                .unwrap_or_else(|| panic!("round {round}: no proof for closure triple"));
            check_proof(&tree, &rendered, Some(&render(&dict, t)));
        }
        for dtr in &dels {
            if !g.contains(dtr) {
                assert!(g.why(&dict, *dtr).is_none(), "round {round}: proof for absent triple");
            }
        }
        g.insert(&mut dict, &dels);
    }
}

#[test]
fn fallback_mode_returns_none_for_derived() {
    // A someValuesFrom restriction forces OwlMode::Fallback: derived triples have no
    // counting state to explain from (documented); asserted triples still explain as leaves.
    let mut dict = Dict::new();
    let v = vocab(&mut dict);
    let owl = "http://www.w3.org/2002/07/owl#";
    let some = dict.intern_iri(&format!("{owl}someValuesFrom"));
    let onprop = dict.intern_iri(&format!("{owl}onProperty"));
    let (r, p, cc, x, y) = (
        ex(&mut dict, "R"),
        ex(&mut dict, "p"),
        ex(&mut dict, "C"),
        ex(&mut dict, "x"),
        ex(&mut dict, "y"),
    );
    let g = MaterializedOwlGraph::new(
        &mut dict,
        &[[r, some, cc], [r, onprop, p], [x, p, y], [y, v.ty, cc]],
    );
    assert_eq!(g.mode(), OwlMode::Fallback);
    assert!(g.contains(&[x, v.ty, r]), "cls-svf1 fires in fallback mode");
    assert!(g.why(&dict, [x, v.ty, r]).is_none(), "fallback derivations are unexplained");
    let tree = g.why(&dict, [x, p, y]).expect("asserted triples still explain");
    assert_eq!(tree.nodes()[0].rule, "asserted");
}

//! `why()` on [`MaterializedN3Graph`] + the id-level [`explain::n3_proof_tree`] bridge
//! (`explain` feature): hand-checked rule-firing chains, a differential sweep (every
//! derived fact explains; the proof's asserted leaves alone re-entail the conclusion under
//! the same rules), and retraction consistency.
#![cfg(feature = "explain")]

mod explain_common;

use explain_common::{check_proof, proof_leaves};
use rustc_hash::FxHashSet;
use sparq_reason::n3::Term;
use sparq_reason::{
    explain::{n3_proof_tree, n3_proof_tree_for_key}, reason_n3_proof_run, reason_n3_terms, ExplainOpts, MaterializedN3Graph,
    N3Mode, ProofTree,
};

fn iri(s: &str) -> Term {
    Term::Iri(s.into())
}
fn ex(local: &str) -> Term {
    iri(&format!("http://ex/{local}"))
}

fn render(f: &[Term; 3]) -> [String; 3] {
    f.clone().map(|t| match t {
        Term::Iri(i) => format!("<{i}>"),
        Term::Lit(v, dt, None) if dt == "http://www.w3.org/2001/XMLSchema#string" => {
            format!("\"{v}\"")
        }
        Term::Lit(v, dt, None) => format!("\"{v}\"^^<{dt}>"),
        Term::Lit(v, _, Some(l)) => format!("\"{v}\"@{l}"),
        other => panic!("unexpected term shape in test: {other:?}"),
    })
}

fn rendered(base: &[[Term; 3]]) -> FxHashSet<[String; 3]> {
    base.iter().map(render).collect()
}

/// Re-entailment oracle: rules + the proof's asserted leaves must re-derive the conclusion.
/// (Leaf conclusions are already serialized N3 terms — feed them straight back.)
fn leaves_entail(rules: &str, tree: &ProofTree, conclusion: &[String; 3]) {
    let mut src = String::from(rules);
    src.push('\n');
    for l in proof_leaves(tree) {
        src.push_str(&format!("{} {} {} .\n", l[0], l[1], l[2]));
    }
    let closure = reason_n3_terms(&src, None).expect("leaf subset must re-parse");
    let ok = closure.facts.iter().any(|f| &render(f) == conclusion);
    assert!(ok, "proof leaves do not re-entail {conclusion:?}\nproof:\n{}", tree.to_text());
}

const RULES: &str = r#"
@prefix : <http://ex/> .
{ ?x :parent ?y } => { ?x :ancestor ?y } .
{ ?x :ancestor ?y . ?y :ancestor ?z } => { ?x :ancestor ?z } .
{ ?x a :Human } => { ?x a :Mortal } .
"#;

#[test]
fn rule_chain_hand_checked() {
    let ty = iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let base = vec![
        [ex("a"), ex("parent"), ex("b")],
        [ex("b"), ex("parent"), ex("c")],
        [ex("socrates"), ty.clone(), ex("Human")],
    ];
    let g = MaterializedN3Graph::new(RULES, &base).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Counting);

    // socrates a :Mortal — one firing of rule 2.
    let f = [ex("socrates"), ty.clone(), ex("Mortal")];
    assert!(g.contains(&f));
    let tree = g.why(&f).expect("derived fact explains");
    check_proof(&tree, &rendered(&base), Some(&render(&f)));
    leaves_entail(RULES, &tree, &render(&f));
    let root = &tree.nodes()[tree.root() as usize];
    assert_eq!(root.rule, "n3-rule-2");
    assert_eq!(root.premises.len(), 1);
    assert_eq!(tree.nodes()[root.premises[0] as usize].rule, "asserted");

    // a :ancestor c — rule 1 over two rule-0 lifts.
    let f = [ex("a"), ex("ancestor"), ex("c")];
    assert!(g.contains(&f));
    let tree = g.why(&f).expect("recursive fact explains");
    check_proof(&tree, &rendered(&base), Some(&render(&f)));
    leaves_entail(RULES, &tree, &render(&f));
    let root = &tree.nodes()[tree.root() as usize];
    assert_eq!(root.rule, "n3-rule-1");
    assert!(tree.nodes().iter().filter(|n| n.rule == "n3-rule-0").count() >= 2);
    assert!(tree.nodes().iter().filter(|n| n.rule == "asserted").count() == 2);

    // Asserted facts explain as single leaves; absent facts do not explain.
    let tree = g.why(&base[0]).unwrap();
    assert_eq!(tree.nodes().len(), 1);
    assert_eq!(tree.nodes()[0].rule, "asserted");
    assert!(g.why(&[ex("c"), ex("ancestor"), ex("a")]).is_none());

    // Determinism.
    let f = [ex("a"), ex("ancestor"), ex("c")];
    assert_eq!(g.why(&f).unwrap().to_json(), g.why(&f).unwrap().to_json());
}

#[test]
fn differential_every_derived_fact_explains() {
    // A chain of parents → quadratic ancestor closure; every derived fact must explain and
    // re-entail from its leaves.
    let mut base: Vec<[Term; 3]> = Vec::new();
    for i in 0..12 {
        base.push([ex(&format!("p{i}")), ex("parent"), ex(&format!("p{}", i + 1))]);
    }
    let g = MaterializedN3Graph::new(RULES, &base).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Counting);
    let base_set: FxHashSet<[Term; 3]> = base.iter().cloned().collect();
    let rendered_set = rendered(&base);
    let mut derived = 0usize;
    for f in g.closure() {
        let tree = g.why(&f).unwrap_or_else(|| panic!("no proof for closure fact"));
        check_proof(&tree, &rendered_set, Some(&render(&f)));
        if !base_set.contains(&f) {
            leaves_entail(RULES, &tree, &render(&f));
            derived += 1;
        }
    }
    assert!(derived >= 78, "expected the full ancestor closure, got {derived}");
}

#[test]
fn retraction_consistency() {
    let mut base: Vec<[Term; 3]> = vec![
        [ex("a"), ex("parent"), ex("b")],
        [ex("b"), ex("parent"), ex("c")],
        [ex("a"), ex("ancestor"), ex("c")], // ALSO asserted: second support
    ];
    let mut g = MaterializedN3Graph::new(RULES, &base).expect("rules parse");
    let f = [ex("a"), ex("ancestor"), ex("c")];
    assert!(g.contains(&f));
    // Asserted + derived: explains as the asserted leaf (a witness, not all witnesses).
    assert_eq!(g.why(&f).unwrap().nodes().len(), 1);

    // Retract the assertion: the derived support takes over.
    g.delete(&[base.pop().unwrap()]);
    assert!(g.contains(&f));
    let tree = g.why(&f).expect("derived support survives");
    check_proof(&tree, &rendered(&base), Some(&render(&f)));
    assert_eq!(tree.nodes()[tree.root() as usize].rule, "n3-rule-1");
    leaves_entail(RULES, &tree, &render(&f));

    // Retract a premise of the derivation: the fact must vanish and stop explaining.
    g.delete(&[[ex("b"), ex("parent"), ex("c")]]);
    assert!(!g.contains(&f));
    assert!(g.why(&f).is_none());
}

#[test]
fn fallback_mode_still_explains() {
    // A backward rule disqualifies the rule set from counting — why() re-runs the batch
    // engine, so explanations still work in fallback mode.
    let rules = r#"
@prefix : <http://ex/> .
{ ?x :ancestor ?y } <= { ?x :parent ?y } .
{ ?x :parent ?y . ?y :parent ?z } => { ?x :grandparent ?z } .
"#;
    let base = vec![[ex("a"), ex("parent"), ex("b")], [ex("b"), ex("parent"), ex("c")]];
    let g = MaterializedN3Graph::new(rules, &base).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Fallback);
    let f = [ex("a"), ex("grandparent"), ex("c")];
    assert!(g.contains(&f));
    let tree = g.why(&f).expect("fallback-mode derivation explains");
    check_proof(&tree, &rendered(&base), Some(&render(&f)));
    assert_eq!(tree.nodes()[tree.root() as usize].premises.len(), 2);
}

#[test]
fn id_level_bridge_from_reason_n3_proof() {
    use sparq_core::dict::Dict;
    let src = r#"
@prefix : <http://ex/> .
:a :parent :b .
:b :parent :c .
{ ?x :parent ?y } => { ?x :ancestor ?y } .
{ ?x :ancestor ?y . ?y :ancestor ?z } => { ?x :ancestor ?z } .
"#;
    let mut dict = Dict::new();
    let run = reason_n3_proof_run(&mut dict, src).expect("reasoning succeeds");
    let (facts, steps) = (&run.closure, &run.steps);
    let (a, anc, c) = (
        dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/a"))),
        dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/ancestor"))),
        dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/c"))),
    );
    let target = [a, anc, c];
    assert!(facts.contains(&target));
    let tree = n3_proof_tree(&dict, &run, target, ExplainOpts::default())
        .expect("one N3 fact interns to the target")
        .expect("derived triple bridges to a proof tree");
    let asserted: FxHashSet<[String; 3]> = facts
        .iter()
        .filter(|t| !steps.iter().any(|s| s.conclusion == **t))
        .map(|&t| {
            [dict.term(t[0]).to_string(), dict.term(t[1]).to_string(), dict.term(t[2]).to_string()]
        })
        .collect();
    check_proof(&tree, &asserted, None);
    assert_eq!(tree.conclusion()[1], "<http://ex/ancestor>");
    // Inputs have no step: the bridge returns None for them (callers explain those as asserted).
    let b = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/b")));
    let par = dict
        .lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://ex/parent")));
    assert!(n3_proof_tree(&dict, &run, [a, par, b], ExplainOpts::default()).expect("unambiguous").is_none());
}

/// GH #6701 review rounds 3–4: a proof never renders an `@forAll` universal by its bare
/// local name — that wrote `{ :x :q ?x }` as `{ ?x :q ?x }`, a formula requiring the two to
/// be equal. The display has no document line to declare it under, so it spells the
/// universal as a plain variable that differs from every variable of the fact (`?x_2`);
/// the KEY carries the universal itself. Declared at document level or in the formula, it
/// is the same one variable (GH #6754), so both read the same.
#[test]
fn why_keeps_a_for_all_universal_distinct_from_a_source_variable() {
    // The variable predicate keeps the graph on the fallback path, whose `why` re-derives
    // through the batch engine (the counting path does not currently derive through a
    // formula that carries a variable — a separate issue).
    for src in [
        "@prefix : <http://ex/>. :a :p { @forAll :x. :x :q ?x }.\n{ :a ?p ?f } => { :b :r ?f }.\n",
        "@prefix : <http://ex/>. @forAll :x. :a :p { :x :q ?x }.\n{ :a ?p ?f } => { :b :r ?f }.\n",
    ] {
        let formula = "{ ?x_2 <http://ex/q> ?x . }";
        let closure = reason_n3_terms(src, None).expect("oracle").facts;
        let asserted = closure.iter().find(|f| f[0] == ex("a")).expect("the asserted formula fact");
        let derived = closure.iter().find(|f| f[0] == ex("b")).expect("the derived fact");
        let g = MaterializedN3Graph::new(src, &[]).expect("rules parse");
        assert_eq!(g.mode(), N3Mode::Fallback);

        let proof = g.why(asserted).expect("asserted fact explains");
        assert_eq!(proof.conclusion()[2], formula, "{}", proof.to_text());
        assert!(proof.nodes()[0].key[2].contains("__ua.http://ex/x"), "{:?}", proof.nodes()[0].key);

        let proof = g.why(derived).expect("derived fact explains");
        let nodes = proof.nodes();
        assert_eq!(nodes.len(), 2, "{}", proof.to_text());
        // The premise and the conclusion carry the same formula, named the same way.
        for n in nodes {
            assert_eq!(n.conclusion[2], formula, "{}", proof.to_text());
        }
        assert_eq!(nodes[1].premises, vec![0]);
    }
}

/// GH #6701 review round 4 (3): a fact renders the SAME in every proof it appears in.
/// `sparq-prov` hashes these strings into the fact's identity, so per-proof naming (`?x` in
/// one proof, `?x_2` in another) split one fact into two entities.
#[test]
fn a_fact_renders_the_same_in_every_proof() {
    let src = "@prefix : <http://ex/>. @forAll :x.
:a :p { :x :q :z }.
:b :p { ?x :q :z }.
{ :a ?p ?f. :b :p ?g } => { :c :r ?f }.
";
    let closure = reason_n3_terms(src, None).expect("oracle").facts;
    let a = closure.iter().find(|f| f[0] == ex("a")).expect(":a fact");
    let c = closure.iter().find(|f| f[0] == ex("c")).expect(":c fact");
    let g = MaterializedN3Graph::new(src, &[]).expect("rules parse");
    let alone = g.why(a).expect(":a explains").conclusion().clone();
    let proof = g.why(c).expect(":c explains");
    let inside = proof
        .nodes()
        .iter()
        .find(|n| n.conclusion[0] == "<http://ex/a>")
        .expect("the :a premise")
        .conclusion
        .clone();
    assert_eq!(alone, inside, "{}", proof.to_text());
}

/// #6735 review: two structurally distinct derived facts — `:a :value ()` and
/// `:a :value <rdf:nil>` (an IRI from `log:uri`) — intern to ONE id triple. The id-level
/// bridge must not pick one: it reports the ambiguity, and the structural selector
/// explains each fact with its own key and its own derivation.
#[test]
fn colliding_id_triples_are_ambiguous_not_first_match() {
    let src = r#"@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .
:go :go :go .
{ :go :go :go } => { :a :value () } .
{ ?i log:uri "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil" } => { :a :value ?i } .
"#;
    let mut dict = sparq_core::dict::Dict::new();
    let run = reason_n3_proof_run(&mut dict, src).expect("reasoning succeeds");
    let steps = &run.steps;
    let ex = |l: &str| Term::Iri(format!("http://ex/{l}"));
    let list = [ex("a"), ex("value"), Term::List(vec![])];
    let nil = [ex("a"), ex("value"), Term::Iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil".into())];
    let (kl, kn) = (sparq_reason::n3::serialize::statement_keys(&list), sparq_reason::n3::serialize::statement_keys(&nil));
    assert_ne!(kl, kn);
    let target = steps.iter().find(|s| s.conclusion_key == kl).expect("the () step").conclusion;
    assert_eq!(steps.iter().find(|s| s.conclusion_key == kn).expect("the rdf:nil step").conclusion, target, "both intern alike");
    let err = n3_proof_tree(&dict, &run, target, ExplainOpts::default()).expect_err("ambiguous");
    let mut want = vec![kl.clone(), kn.clone()];
    want.sort();
    assert_eq!(err.keys, want);
    let tl = n3_proof_tree_for_key(&dict, steps, &kl, ExplainOpts::default()).expect("() explains");
    let tn = n3_proof_tree_for_key(&dict, steps, &kn, ExplainOpts::default()).expect("rdf:nil explains");
    assert_eq!(tl.nodes().last().unwrap().key, kl);
    assert_eq!(tn.nodes().last().unwrap().key, kn);
    assert_eq!(tl.nodes().last().unwrap().rule, "n3-rule-0");
    assert_eq!(tn.nodes().last().unwrap().rule, "n3-rule-1");
}

/// #6735 review round 2: ambiguity is decided over the WHOLE closure, asserted facts
/// included. Two structurally distinct facts that intern to one id triple — one ASSERTED,
/// one DERIVED — are ambiguous for the id-level bridge whichever of them the caller means,
/// whether the caller asks by the ids (ambiguous) or by either key (resolved, the derived
/// one to its proof and the asserted one to none). The asserted side is `()`: the reverse
/// pairing, an ASSERTED rdf:nil IRI, cannot be written in N3 text (the parser reads a
/// written rdf:nil as `()`, and rejects a written directional tag such as `en--ltr`, the
/// other lossy field), so a derived-vs-derived collision is covered by
/// `colliding_id_triples_are_ambiguous_not_first_match` instead.
#[test]
fn an_asserted_fact_and_a_derived_one_that_intern_alike_are_ambiguous() {
    const PRE: &str = "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n";
    let nil_iri = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
    let ex = |l: &str| Term::Iri(format!("http://ex/{l}"));
    let key = |o: Term| sparq_reason::n3::serialize::statement_keys(&[ex("a"), ex("value"), o]);
    let cases = [
        (
            format!("{PRE}:a :value () .\n{{ ?i log:uri \"{nil_iri}\" }} => {{ :a :value ?i }} .\n"),
            key(Term::List(vec![])),
            key(Term::Iri(nil_iri.into())),
        ),
        // The same with the asserted fact written AFTER the rule (document order is not
        // what decides).
        (
            format!("{PRE}{{ ?i log:uri \"{nil_iri}\" }} => {{ :a :value ?i }} .\n:a :value () .\n"),
            key(Term::List(vec![])),
            key(Term::Iri(nil_iri.into())),
        ),
    ];
    for (src, asserted, derived) in &cases {
        let mut dict = sparq_core::dict::Dict::new();
        let run = reason_n3_proof_run(&mut dict, src).expect("reasoning succeeds");
        assert!(run.closure_keys.contains(asserted), "asserted fact keyed:\n{src}\n{:#?}", run.closure_keys);
        assert!(run.closure_keys.contains(derived), "derived fact keyed:\n{src}\n{:#?}", run.closure_keys);
        assert!(!run.steps.iter().any(|s| &s.conclusion_key == asserted), "asserted, not derived: {src}");
        let step = run.steps.iter().find(|s| &s.conclusion_key == derived).expect("the derived fact has a step");
        let at = run.closure_keys.iter().position(|k| k == asserted).unwrap();
        assert_eq!(run.closure[at], step.conclusion, "they intern alike: {src}");
        let err = n3_proof_tree(&dict, &run, step.conclusion, ExplainOpts::default()).expect_err("ambiguous");
        let mut want = vec![asserted.clone(), derived.clone()];
        want.sort();
        assert_eq!(err.keys, want, "{src}");
        // By key, each resolves to what it is: the derived one has a proof, the asserted none.
        let t = n3_proof_tree_for_key(&dict, &run.steps, derived, ExplainOpts::default()).expect("derived explains");
        assert_eq!(&t.nodes().last().unwrap().key, derived);
        assert!(n3_proof_tree_for_key(&dict, &run.steps, asserted, ExplainOpts::default()).is_none(), "{src}");
    }
}

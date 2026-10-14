// sq-clsv6 (Phase E5, `incremental`): the acceptance oracle for incremental
// classification under TBox edits — "the incremental result matches a full re-classification
// (ELK-differential) on an edited-TBox fixture".
//
// ORACLE NOTE (inherited verbatim from tests/differential.rs, sq-evb1): the bead asks for a
// differential check vs ELK (Java, Apache-2.0). Making ELK a CI/build dependency (a JVM + the ELK
// jar, fetched over the network) is fragile and pushes a heavy non-Rust toolchain onto the gate, so
// the oracle here is EL ontologies whose complete subsumption closure is HAND-DERIVED from the
// Baader–Brandt–Lutz / ELK CR1–CR5 calculus (the same rules ELK implements) and asserted
// exhaustively: every expected subsumption present AND no spurious one. Each fixture is small
// enough to verify by hand.
//
// Two obligations are checked, and the SECOND is the load-bearing one:
//
//  1. ELK-differential: after each edit the hierarchy equals the hand-derived closure of the
//     edited TBox.
//  2. FULL-RECLASSIFICATION EQUALITY: after each edit the hierarchy equals
//     `Classifier::classify` over the SAME post-edit triple set — the property that makes the
//     incremental path safe to use at all. It is asserted after EVERY edit of every fixture here,
//     including the randomized edit sequences, and on BOTH dispositions (an incremental fold and a
//     full-rebuild fallback), so a bug in the fold cannot hide behind a fallback.
//
// GATING: the whole file is `#![cfg(feature = "incremental")]` — `IncrementalClassifier` only
// EXISTS under the feature, so the default `cargo test --all-targets` build (which passes NO
// features) compiles this file to nothing, exactly like tests/par_differential.rs under `par`.
// The feature-matrix `sparq-reason-el (incremental …)` legs are what actually RUN it.

#![cfg(feature = "incremental")]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{Classifier, EditDisposition, FullReason, IncrementalClassifier};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

fn vocab(dict: &Dict, iri_str: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(
        iri_str.to_string(),
    )))
}

fn sub_class_of(dict: &Dict) -> Id {
    vocab(dict, "http://www.w3.org/2000/01/rdf-schema#subClassOf")
}

fn parse(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
    Graph::parse_to_triples(ttl, "turtle").expect("parse")
}

/// The hand-derived (ELK-calculus) closure oracle from `tests/differential.rs`: asserts the
/// named-class subsumption relation of `h` EXACTLY equals `expected` over `classes`.
fn assert_closure(
    dict: &Dict,
    h: &sparq_reason_el::ClassHierarchy,
    classes: &[&str],
    expected: &[(&str, &str)],
    ctx: &str,
) {
    let exp: std::collections::HashSet<(&str, &str)> = expected.iter().copied().collect();
    for &sub in classes {
        for &sup in classes {
            if sub == sup {
                continue;
            }
            let got = h.is_subclass_of(iri(dict, sub), iri(dict, sup));
            let want = exp.contains(&(sub, sup));
            assert_eq!(got, want, "[{ctx}] {sub} ⊑ {sup}: got {got}, want {want}");
        }
    }
}

/// Obligation 2: the incremental classifier's hierarchy equals a FULL re-classification of its own
/// current triple set — the whole safety property, asserted after every edit.
fn assert_matches_full_reclassification(dict: &Dict, inc: &IncrementalClassifier, ctx: &str) {
    let got = inc.hierarchy();
    let want = Classifier::classify(dict, inc.triples());

    // Compare the WHOLE named-class relation over every class the dict knows, not a chosen subset:
    // a spurious subsumption between two classes a fixture forgot to list would otherwise slip
    // through. Concept ids come from the dict, so iterating dict ids covers both hierarchies.
    for id in 0..dict.len() as u32 {
        let g = got.super_classes(id);
        let w = want.super_classes(id);
        let (mut g, mut w) = (g.to_vec(), w.to_vec());
        g.sort_unstable();
        w.sort_unstable();
        assert_eq!(g, w, "[{ctx}] super-classes of dict id {id} diverged");
    }
    let (mut gu, mut wu) = (
        got.unsatisfiable_classes().to_vec(),
        want.unsatisfiable_classes().to_vec(),
    );
    gu.sort_unstable();
    wu.sort_unstable();
    assert_eq!(gu, wu, "[{ctx}] unsatisfiable classes diverged");
    assert_eq!(
        got.report().thing_unsatisfiable,
        want.report().thing_unsatisfiable,
        "[{ctx}] the global ⊤ ⊑ ⊥ verdict diverged"
    );
    // `skipped_axioms` is an ENTAILMENT-honesty counter (how many axioms were not applied), so it
    // must agree too — the incremental path accumulates each class-axiom triple's verdict exactly
    // once over the graph's lifetime. (`named_classes` is the live index size and is documented NOT
    // to agree; see `IncrementalReport::report`.)
    assert_eq!(
        got.report().skipped_axioms,
        want.report().skipped_axioms,
        "[{ctx}] skipped_axioms diverged"
    );
    #[cfg(feature = "rbox")]
    assert_eq!(
        got.report().rbox_non_regular,
        want.report().rbox_non_regular,
        "[{ctx}] rbox_non_regular diverged"
    );
}

// ---------------------------------------------------------------------------------------------
// 1. The acceptance fixture: an edited TBox, classified incrementally, vs the ELK-derived closure.
// ---------------------------------------------------------------------------------------------

#[test]
fn elk_differential_across_an_edited_tbox() {
    // The spike's CR4 chain, GROWN one axiom at a time. Final TBox:
    //   A ⊑ ∃r.B,  B ⊑ C,  ∃r.C ⊑ D,  D ⊑ E
    // Closure (CR1–CR4): B⊑C; A⊑D (CR4 through the r-successor); A⊑E, D⊑E (CR1).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :C .
         [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D .
         :D rdfs:subClassOf :E ."
    );
    let (dict, all) = parse(&ttl);
    let classes = ["A", "B", "C", "D", "E"];
    let sub = sub_class_of(&dict);
    let b_sub_c = [iri(&dict, "B"), sub, iri(&dict, "C")];
    let d_sub_e = [iri(&dict, "D"), sub, iri(&dict, "E")];

    // Start from the TBox WITHOUT `B ⊑ C` and WITHOUT `D ⊑ E`. Then CR4 cannot fire (nothing puts
    // C into S(B)), so the only closure edge is... none at all.
    let start: Vec<[Id; 3]> = all
        .iter()
        .copied()
        .filter(|t| *t != b_sub_c && *t != d_sub_e)
        .collect();
    let mut inc = IncrementalClassifier::new(&dict, &start);
    assert_closure(&dict, &inc.hierarchy(), &classes, &[], "step 0");
    assert_matches_full_reclassification(&dict, &inc, "step 0");

    // Edit 1: add `B ⊑ C`. This UNLOCKS the CR4 traversal — A ⊑ D appears without D ⊑ E existing.
    let r1 = inc.apply_edits(&dict, &[b_sub_c], &[]);
    assert_eq!(
        r1.disposition,
        EditDisposition::Incremental,
        "adding a class axiom is a monotone extension"
    );
    assert_eq!(r1.added_triples, 1);
    assert!(
        r1.reseeded_memberships > 0,
        "the delta must re-queue the retained memberships its trigger keys occur in"
    );
    assert_closure(
        &dict,
        &inc.hierarchy(),
        &classes,
        &[("B", "C"), ("A", "D")],
        "step 1",
    );
    assert_matches_full_reclassification(&dict, &inc, "step 1");

    // Edit 2: add `D ⊑ E`. CR1 must now thread it onto the DERIVED A ⊑ D from the previous edit —
    // i.e. the fold has to reason over a subsumption that was itself incrementally derived.
    let r2 = inc.apply_edits(&dict, &[d_sub_e], &[]);
    assert_eq!(r2.disposition, EditDisposition::Incremental);
    assert_closure(
        &dict,
        &inc.hierarchy(),
        &classes,
        &[("B", "C"), ("A", "D"), ("A", "E"), ("D", "E")],
        "step 2",
    );
    assert_matches_full_reclassification(&dict, &inc, "step 2");

    // Edit 3: RETRACT `B ⊑ C`. A ⊑ D must DISAPPEAR — the honest full-reclassification fallback.
    let r3 = inc.apply_edits(&dict, &[], &[b_sub_c]);
    assert_eq!(
        r3.disposition,
        EditDisposition::Full(FullReason::Retraction),
        "a retraction is not incremental here, and must say so"
    );
    assert_eq!(r3.removed_triples, 1);
    assert_closure(
        &dict,
        &inc.hierarchy(),
        &classes,
        &[("D", "E")],
        "step 3 (retracted)",
    );
    assert_matches_full_reclassification(&dict, &inc, "step 3 (retracted)");

    // Edit 4: put it back. The rebuilt state must fold a further addition just like a fresh one.
    let r4 = inc.apply_edits(&dict, &[b_sub_c], &[]);
    assert_eq!(r4.disposition, EditDisposition::Incremental);
    assert_closure(
        &dict,
        &inc.hierarchy(),
        &classes,
        &[("B", "C"), ("A", "D"), ("A", "E"), ("D", "E")],
        "step 4 (restored)",
    );
    assert_matches_full_reclassification(&dict, &inc, "step 4 (restored)");
}

#[test]
fn elk_differential_for_a_brand_new_restriction_node() {
    // An edit that adds a WHOLE class expression, not just an axiom between named classes:
    //   pre:   X ⊑ P,  ∃r.P ⊑ Q
    //   edit:  X ⊑ ∃r.X            (a fresh restriction bnode + its two structural triples)
    //   closure after: X ⊑ Q (CR3 makes the link (X,X) ∈ R(r), CR4 fires on P ∈ S(X)).
    let ttl = format!(
        "{PRE}
         :X rdfs:subClassOf :P .
         [ owl:onProperty :r ; owl:someValuesFrom :P ] rdfs:subClassOf :Q .
         :X rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :X ] ."
    );
    let (dict, all) = parse(&ttl);
    let classes = ["X", "P", "Q"];

    // The edit is "every triple mentioning the SECOND restriction bnode", i.e. the one whose
    // someValuesFrom filler is :X. Identify it structurally rather than by bnode label.
    let svf = vocab(&dict, "http://www.w3.org/2002/07/owl#someValuesFrom");
    let x = iri(&dict, "X");
    let node = all
        .iter()
        .find(|t| t[1] == svf && t[2] == x)
        .expect("the X-filled restriction node")[0];
    let edit: Vec<[Id; 3]> = all
        .iter()
        .copied()
        .filter(|t| t[0] == node || t[2] == node)
        .collect();
    assert_eq!(
        edit.len(),
        3,
        "onProperty + someValuesFrom + the X ⊑ node axiom"
    );
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| !edit.contains(t)).collect();

    let mut inc = IncrementalClassifier::new(&dict, &start);
    assert_closure(&dict, &inc.hierarchy(), &classes, &[("X", "P")], "pre");
    assert_matches_full_reclassification(&dict, &inc, "pre");
    let pre_named = inc.hierarchy().report().named_classes;

    let r = inc.apply_edits(&dict, &edit, &[]);
    assert_eq!(
        r.disposition,
        EditDisposition::Incremental,
        "a fresh class-expression node is delta-local"
    );
    // `IncrementalReport::report`'s documented contract for `named_classes`: the LIVE concept-index
    // size, not the pre-edit snapshot. The edit mints a fresh normalization name for the new
    // restriction node, so the count must have grown — and it must be the live index size, which
    // `hierarchy()` reports too.
    assert!(
        r.report.named_classes > pre_named,
        "the fresh restriction node mints a concept, so the live index must grow \
         (pre {}, post {})",
        pre_named,
        r.report.named_classes
    );
    assert_eq!(
        inc.hierarchy().report().named_classes,
        r.report.named_classes,
        "the hierarchy must carry the same live count the edit reported"
    );
    assert_closure(
        &dict,
        &inc.hierarchy(),
        &classes,
        &[("X", "P"), ("X", "Q")],
        "post",
    );
    assert_matches_full_reclassification(&dict, &inc, "post");
}

#[test]
fn elk_differential_for_an_incrementally_derived_clash() {
    // Unsatisfiability must appear incrementally too: A ⊓ B ⊑ ⊥ via owl:disjointWith, reached only
    // after the second membership axiom lands.
    let ttl = format!(
        "{PRE}
         :A owl:disjointWith :B .
         :Z rdfs:subClassOf :A .
         :Z rdfs:subClassOf :B ."
    );
    let (dict, all) = parse(&ttl);
    let sub = sub_class_of(&dict);
    let z_sub_b = [iri(&dict, "Z"), sub, iri(&dict, "B")];
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| *t != z_sub_b).collect();

    let mut inc = IncrementalClassifier::new(&dict, &start);
    assert!(
        inc.hierarchy().unsatisfiable_classes().is_empty(),
        "Z ⊑ A alone is satisfiable"
    );

    let r = inc.apply_edits(&dict, &[z_sub_b], &[]);
    assert_eq!(r.disposition, EditDisposition::Incremental);
    assert!(
        inc.hierarchy()
            .unsatisfiable_classes()
            .contains(&iri(&dict, "Z")),
        "Z ⊑ A ⊓ B ⊑ ⊥ must be derived by the incremental fold"
    );
    assert_matches_full_reclassification(&dict, &inc, "clash");
}

// ---------------------------------------------------------------------------------------------
// 2. Disposition discipline: every non-monotone edit shape falls back, and SAYS it fell back.
// ---------------------------------------------------------------------------------------------

#[test]
fn re_adding_a_present_triple_is_a_no_op() {
    let ttl = format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C .");
    let (dict, all) = parse(&ttl);
    let mut inc = IncrementalClassifier::new(&dict, &all);
    let before = inc.triples().len();

    let r = inc.apply_edits(&dict, &[all[0]], &[]);
    assert_eq!(r.disposition, EditDisposition::NoOp);
    assert_eq!((r.added_triples, r.removed_triples), (0, 0));
    assert_eq!(r.added_axioms, 0, "a no-op must not duplicate the axiom");
    assert_eq!(inc.triples().len(), before, "the graph is unchanged");
    assert_matches_full_reclassification(&dict, &inc, "no-op");
}

#[test]
fn removing_an_absent_triple_is_a_no_op() {
    let ttl = format!("{PRE} :A rdfs:subClassOf :B . :C rdfs:subClassOf :D .");
    let (dict, all) = parse(&ttl);
    let absent = [iri(&dict, "A"), sub_class_of(&dict), iri(&dict, "D")];
    let start: Vec<[Id; 3]> = all.to_vec();
    let mut inc = IncrementalClassifier::new(&dict, &start);

    let r = inc.apply_edits(&dict, &[], &[absent]);
    assert_eq!(r.disposition, EditDisposition::NoOp);
    assert_eq!(r.removed_triples, 0);
    assert_matches_full_reclassification(&dict, &inc, "absent removal");
}

#[test]
fn removing_and_re_adding_the_same_triple_nets_to_a_no_op() {
    // "remove then add" leaves the triple present, so the NET effect is nothing — and in
    // particular it must not be reported as a retraction (which would force a needless rebuild).
    let ttl = format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C .");
    let (dict, all) = parse(&ttl);
    let mut inc = IncrementalClassifier::new(&dict, &all);

    let r = inc.apply_edits(&dict, &[all[0]], &[all[0]]);
    assert_eq!(r.disposition, EditDisposition::NoOp);
    assert_eq!(inc.triples().len(), all.len());
    assert_matches_full_reclassification(&dict, &inc, "net no-op");
}

#[test]
fn mutating_a_live_restriction_node_falls_back() {
    // A second `owl:someValuesFrom` on a restriction node that is ALREADY in the graph changes what
    // the enclosing axiom means. That is not monotone, so it must rebuild rather than fold.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :C ."
    );
    let (dict, all) = parse(&ttl);
    let svf = vocab(&dict, "http://www.w3.org/2002/07/owl#someValuesFrom");
    let node = all.iter().find(|t| t[1] == svf).expect("restriction")[0];
    let mut inc = IncrementalClassifier::new(&dict, &all);

    let r = inc.apply_edits(&dict, &[[node, svf, iri(&dict, "C")]], &[]);
    assert_eq!(
        r.disposition,
        EditDisposition::Full(FullReason::ExistingNode),
        "structure on a live node is not delta-local"
    );
    assert_matches_full_reclassification(&dict, &inc, "mutated node");
}

#[test]
fn giving_structure_to_an_object_only_node_falls_back() {
    // The subtle one: `_:b` occurs ONLY as the object of `:A rdfs:subClassOf _:b`, so it currently
    // decodes as an opaque class atom. Adding `_:b owl:onProperty :r` turns that existing axiom
    // into a restriction — it CHANGES the axiom, so "not a subject yet" is not good enough and the
    // check must reject it on "not mentioned anywhere yet".
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf _:b .
         :Q rdfs:subClassOf :R ."
    );
    let (dict, all) = parse(&ttl);
    let sub = sub_class_of(&dict);
    let node = all
        .iter()
        .find(|t| t[0] == iri(&dict, "A") && t[1] == sub)
        .expect("A ⊑ _:b")[2];
    let on_prop = vocab(&dict, "http://www.w3.org/2002/07/owl#onProperty");
    let mut inc = IncrementalClassifier::new(&dict, &all);

    let r = inc.apply_edits(&dict, &[[node, on_prop, iri(&dict, "r")]], &[]);
    assert_eq!(
        r.disposition,
        EditDisposition::Full(FullReason::ExistingNode),
        "an object-only node is still a live node"
    );
    assert_matches_full_reclassification(&dict, &inc, "object-only node");
}

#[test]
fn an_rbox_axiom_falls_back() {
    // `rdfs:subPropertyOf` changes the role automaton EVERY existing link is closed under (under
    // `rbox`), so it is rejected — uniformly, in every feature state, so the disposition does not
    // depend on which features are on.
    // The RBox triple is parsed WITH the fixture (so its vocabulary is interned in the dict, as the
    // API requires of any edit) and then held back out of the starting graph. Its subject is a
    // BRAND-NEW property, so `ExistingNode` cannot be what rejects it — the verdict has to come from
    // the vocabulary whitelist.
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :brandNewProp rdfs:subPropertyOf :alsoBrandNew ."
    );
    let (dict, all) = parse(&ttl);
    let sub_prop = vocab(&dict, "http://www.w3.org/2000/01/rdf-schema#subPropertyOf");
    let rbox_triple = [
        iri(&dict, "brandNewProp"),
        sub_prop,
        iri(&dict, "alsoBrandNew"),
    ];
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| *t != rbox_triple).collect();
    let mut inc = IncrementalClassifier::new(&dict, &start);

    let r = inc.apply_edits(&dict, &[rbox_triple], &[]);
    assert_eq!(
        r.disposition,
        EditDisposition::Full(FullReason::Vocabulary),
        "an RBox edit is not delta-local"
    );
    assert_matches_full_reclassification(&dict, &inc, "rbox edit");
}

#[test]
fn a_non_el_axiom_added_incrementally_is_counted_as_a_skip() {
    // Honest incompleteness must survive the fold: an `owl:unionOf` axiom added by an edit lands in
    // `skipped_axioms` exactly as a from-scratch extraction would count it, and derives nothing.
    let ttl = format!(
        "{PRE}
         :E rdfs:subClassOf :F .
         [ owl:unionOf ( :A :B ) ] rdfs:subClassOf :C ."
    );
    let (dict, all) = parse(&ttl);
    let union_of = vocab(&dict, "http://www.w3.org/2002/07/owl#unionOf");
    let node = all.iter().find(|t| t[1] == union_of).expect("unionOf")[0];
    let edit: Vec<[Id; 3]> = all
        .iter()
        .copied()
        .filter(|t| t[0] == node || t[2] == node)
        .collect();
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| !edit.contains(t)).collect();

    let mut inc = IncrementalClassifier::new(&dict, &start);
    assert_eq!(inc.hierarchy().report().skipped_axioms, 0);

    let r = inc.apply_edits(&dict, &edit, &[]);
    assert_eq!(r.disposition, EditDisposition::Incremental);
    assert_eq!(
        r.report.skipped_axioms, 1,
        "the unionOf axiom must be counted as skipped, not misapplied"
    );
    assert!(
        !inc.hierarchy()
            .is_subclass_of(iri(&dict, "A"), iri(&dict, "C")),
        "a skipped axiom must fabricate no subsumption"
    );
    assert_matches_full_reclassification(&dict, &inc, "non-EL addition");
}

#[test]
fn edit_disposition_is_incremental_reports_only_the_fold() {
    assert!(EditDisposition::Incremental.is_incremental());
    assert!(!EditDisposition::NoOp.is_incremental());
    for reason in [
        FullReason::Retraction,
        FullReason::ExistingNode,
        FullReason::Vocabulary,
    ] {
        assert!(
            !EditDisposition::Full(reason).is_incremental(),
            "{reason:?} is a full re-classification"
        );
    }
}

#[test]
fn triples_returns_the_live_graph() {
    let ttl = format!("{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C . :C rdfs:subClassOf :D .");
    let (dict, all) = parse(&ttl);
    let sub = sub_class_of(&dict);
    let c_sub_d = [iri(&dict, "C"), sub, iri(&dict, "D")];
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| *t != c_sub_d).collect();

    let mut inc = IncrementalClassifier::new(&dict, &start);
    assert_eq!(inc.triples().len(), start.len());
    assert!(!inc.triples().contains(&c_sub_d));

    inc.apply_edits(&dict, &[c_sub_d], &[]);
    assert!(inc.triples().contains(&c_sub_d), "the addition is live");
    assert_eq!(inc.triples().len(), start.len() + 1);

    inc.apply_edits(&dict, &[], &[c_sub_d]);
    assert!(!inc.triples().contains(&c_sub_d), "the retraction is live");
    assert_eq!(inc.triples().len(), start.len());
}

#[cfg(feature = "cdomain")]
#[test]
fn a_concrete_domain_graph_always_takes_the_full_path() {
    // The `cdomain` datatype concepts are minted by a whole-graph pre-pass whose node → concept map
    // the delta path cannot reconstruct, so such a graph is excluded from the fold OUTRIGHT — and
    // the fallback still has to produce the right answer.
    let ttl = format!(
        "{PRE}
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
         :Adult rdfs:subClassOf
           [ owl:onProperty :age ;
             owl:someValuesFrom
               [ owl:onDatatype xsd:integer ;
                 owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] ] .
         :G rdfs:subClassOf :H .
         :H rdfs:subClassOf :I ."
    );
    let (dict, all) = parse(&ttl);
    let sub = sub_class_of(&dict);
    let h_sub_i = [iri(&dict, "H"), sub, iri(&dict, "I")];
    let start: Vec<[Id; 3]> = all.iter().copied().filter(|t| *t != h_sub_i).collect();

    let mut inc = IncrementalClassifier::new(&dict, &start);
    let r = inc.apply_edits(&dict, &[h_sub_i], &[]);
    assert_eq!(
        r.disposition,
        EditDisposition::Full(FullReason::ConcreteDomain)
    );
    assert!(inc
        .hierarchy()
        .is_subclass_of(iri(&dict, "G"), iri(&dict, "I")));
    assert_matches_full_reclassification(&dict, &inc, "cdomain graph");
}

// ---------------------------------------------------------------------------------------------
// 3. Randomized edit sequences: the fold has to survive orders no fixture author thought of.
// ---------------------------------------------------------------------------------------------

/// A deterministic 64-bit LCG — the crate has no `rand` dependency and a fixed seed keeps a failure
/// reproducible (the whole point of a differential stress test).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A TBox mixing every default-fragment shape (named inclusions, conjunctions, existentials on both
/// sides of ⊑, a nominal `hasValue`, a `hasSelf`, a disjointness clash, an equivalence, and a
/// non-EL `unionOf` skip) so a randomized edit stream exercises all of CR1–CR6 + CR-Self.
fn stress_tbox() -> String {
    format!(
        "{PRE}
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
         :A1 rdfs:subClassOf :A2 . :A2 rdfs:subClassOf :A3 . :A3 rdfs:subClassOf :A4 .
         :A4 rdfs:subClassOf :A5 . :A5 rdfs:subClassOf :A6 .
         :B1 rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :A2 ] .
         [ owl:onProperty :r ; owl:someValuesFrom :A4 ] rdfs:subClassOf :B2 .
         :B2 rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :B3 ] .
         [ owl:onProperty :s ; owl:someValuesFrom :B3 ] rdfs:subClassOf :B4 .
         :C1 rdfs:subClassOf [ owl:intersectionOf ( :A1 :B1 ) ] .
         [ owl:intersectionOf ( :A2 :B2 ) ] rdfs:subClassOf :C2 .
         :C3 owl:equivalentClass [ owl:intersectionOf ( :A3 :B3 ) ] .
         :D1 rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :ind ] .
         [ owl:onProperty :r ; owl:hasValue :ind ] rdfs:subClassOf :D2 .
         :E1 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :t ; owl:hasSelf \"true\"^^xsd:boolean ] .
         [ a owl:Restriction ; owl:onProperty :t ; owl:hasSelf \"true\"^^xsd:boolean ] rdfs:subClassOf :E2 .
         :F1 owl:disjointWith :F2 .
         :F3 rdfs:subClassOf :F1 . :F3 rdfs:subClassOf :F2 .
         [ owl:unionOf ( :A1 :B1 ) ] rdfs:subClassOf :G1 ."
    )
}

/// Groups the graph into EDIT UNITS: a top-level class axiom plus, transitively, every triple whose
/// subject is a bnode only that axiom reaches. Editing whole units is what a real ontology editor
/// does (you add or delete an axiom, not half a restriction) and it is the shape the fast path is
/// specified for; splitting a unit across edits is exercised by the `ExistingNode` fallback tests
/// above, and those edits still have to produce the right answer, which is asserted either way.
fn edit_units(dict: &Dict, triples: &[[Id; 3]]) -> Vec<Vec<[Id; 3]>> {
    let sub = sub_class_of(dict);
    let equiv = vocab(dict, "http://www.w3.org/2002/07/owl#equivalentClass");
    let disjoint = vocab(dict, "http://www.w3.org/2002/07/owl#disjointWith");
    let is_axiom = |p: Id| p == sub || p == equiv || p == disjoint;

    // Anonymous (blank-node) structure is owned by the axiom that references it.
    let anon = |id: Id| {
        !sparq_core::dict::is_inline(id)
            && matches!(
                dict.term_parts(id),
                sparq_core::dict::TermParts::Blank(_)
            )
    };
    let mut units = Vec::new();
    for &t in triples {
        if !is_axiom(t[1]) {
            continue;
        }
        let mut unit = vec![t];
        let mut frontier: Vec<Id> = [t[0], t[2]].into_iter().filter(|&i| anon(i)).collect();
        let mut seen: std::collections::HashSet<Id> = frontier.iter().copied().collect();
        while let Some(node) = frontier.pop() {
            for &u in triples {
                if u[0] != node || is_axiom(u[1]) {
                    continue;
                }
                unit.push(u);
                if anon(u[2]) && seen.insert(u[2]) {
                    frontier.push(u[2]);
                }
            }
        }
        units.push(unit);
    }
    units
}

#[test]
fn randomized_addition_streams_match_full_reclassification() {
    let (dict, all) = parse(&stress_tbox());
    let units = edit_units(&dict, &all);
    assert!(units.len() >= 15, "the stress TBox must have real breadth");

    for seed in 0..12u64 {
        let mut order: Vec<usize> = (0..units.len()).collect();
        let mut rng = Lcg(0x5eed_0000 + seed);
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }

        let mut inc = IncrementalClassifier::new(&dict, &[]);
        let mut folds = 0usize;
        for (step, &u) in order.iter().enumerate() {
            let r = inc.apply_edits(&dict, &units[u], &[]);
            if r.disposition.is_incremental() {
                folds += 1;
            }
            assert_whole_unit_addition_folds(&r);
            assert_matches_full_reclassification(&dict, &inc, &format!("seed {seed} step {step}"));
        }
        // The whole graph must have been rebuilt by the stream, and the stream must actually have
        // EXERCISED the fold (otherwise this test would silently only cover the fallback).
        assert_eq!(inc.triples().len(), all.len(), "seed {seed}: every unit applied");
        assert!(folds > 0, "seed {seed}: the incremental path was never taken");
    }
}

/// A whole-unit addition is either a genuine no-op (a unit whose triples another unit already
/// carried) or a fold — never a fallback: whole units never mutate a live node.
fn assert_whole_unit_addition_folds(r: &sparq_reason_el::IncrementalReport) {
    assert!(
        matches!(
            r.disposition,
            EditDisposition::Incremental | EditDisposition::NoOp
        ),
        "a whole-unit addition must fold or be a no-op, got {:?}",
        r.disposition
    );
}

#[test]
fn randomized_add_and_retract_streams_match_full_reclassification() {
    // Mixed streams: each step either adds a missing unit or retracts a present one. Retraction
    // takes the full path, but the ALTERNATION is what stresses the state hygiene — a rebuild must
    // leave the classifier able to fold again, with `mentioned` / `present` exactly rebuilt.
    let (dict, all) = parse(&stress_tbox());
    let units = edit_units(&dict, &all);

    for seed in 0..8u64 {
        let mut rng = Lcg(0xdead_0000 + seed);
        let mut inc = IncrementalClassifier::new(&dict, &all);
        let mut applied: Vec<bool> = vec![true; units.len()];
        let (mut folds, mut rebuilds) = (0usize, 0usize);

        for step in 0..24 {
            let u = rng.below(units.len());
            let r = if applied[u] {
                applied[u] = false;
                inc.apply_edits(&dict, &[], &units[u])
            } else {
                applied[u] = true;
                inc.apply_edits(&dict, &units[u], &[])
            };
            match r.disposition {
                EditDisposition::Incremental => folds += 1,
                EditDisposition::Full(_) => rebuilds += 1,
                EditDisposition::NoOp => {}
            }
            assert_matches_full_reclassification(&dict, &inc, &format!("seed {seed} step {step}"));
        }
        assert!(
            folds > 0 && rebuilds > 0,
            "seed {seed}: the stream must exercise BOTH paths (folds {folds}, rebuilds {rebuilds})"
        );
    }
}

#[test]
fn randomized_triple_at_a_time_streams_match_full_reclassification() {
    // The STRONGEST form of obligation 2, and the one that does not depend on the author guessing
    // the right edit granularity: add ONE triple at a time in random order, which routinely splits a
    // class expression across edits and so exercises exactly the "an existing axiom's meaning
    // changes" hazard the whitelist exists to reject. Whatever disposition each step takes, the
    // hierarchy must equal a full re-classification of the graph built so far.
    let (dict, all) = parse(&stress_tbox());

    for seed in 0..10u64 {
        let mut order: Vec<usize> = (0..all.len()).collect();
        let mut rng = Lcg(0xfeed_0000 + seed);
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }

        let mut inc = IncrementalClassifier::new(&dict, &[]);
        for (step, &t) in order.iter().enumerate() {
            inc.apply_edits(&dict, &[all[t]], &[]);
            assert_matches_full_reclassification(
                &dict,
                &inc,
                &format!("triple stream seed {seed} step {step}"),
            );
        }
        assert_eq!(inc.triples().len(), all.len(), "seed {seed}: whole graph rebuilt");
    }
}

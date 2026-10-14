// sq-xetf7: differential-correctness fixtures for the EL+ RBox classifier (Phase E2:
// CR10 role inclusion + CR11 role composition / transitive roles / property chains).
//
// `#![cfg(feature = "rbox")]` — these run ONLY with the `rbox` feature on. A feature-matrix.yml
// leg (`sparq-reason-el (rbox)`) builds + tests + clippies this file with the feature enabled, so
// it is NOT a compiled-empty-and-never-run suite (the gap #1171 flagged); the default build (and
// ci.yml's `--workspace --all-targets` archive, which passes no `--features`) compiles it empty.
//
// ORACLE NOTE (same discipline as tests/differential.rs, flagged in the PR + issue #1163): the
// expected closures are hand-derived from the Baader–Brandt–Lutz / ELK CR1–CR5 + CR10/CR11
// calculus (the SAME role rules ELK implements) and asserted EXHAUSTIVELY — every expected
// subsumption holds AND no spurious one does. Each fixture is small enough to verify the role
// automaton by hand. Making ELK (a JVM + jar fetched over the network) a CI dependency is against
// the lean-default discipline; where ELK is available locally it can cross-check the same Turtle
// offline (a developer step, not a gate). E3 (sq-s2nob) revisits ELK-on-CI at biomedical scale.

#![cfg(feature = "rbox")]

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{classify_graph, Classifier};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;

fn iri(dict: &Dict, frag: &str) -> Id {
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
        "http://ex/{frag}"
    ))))
}

/// Asserts the classifier's named-class subsumption relation EXACTLY equals `expected` over the
/// class set `classes` (every pair in `expected` holds; no other proper named subsumption does).
fn assert_closure(ttl: &str, classes: &[&str], expected: &[(&str, &str)]) {
    let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    let exp: std::collections::HashSet<(&str, &str)> = expected.iter().copied().collect();
    for &sub in classes {
        for &sup in classes {
            if sub == sup {
                continue;
            }
            let got = h.is_subclass_of(iri(&dict, sub), iri(&dict, sup));
            let want = exp.contains(&(sub, sup));
            assert_eq!(
                got, want,
                "subsumption {sub} ⊑ {sup}: got {got}, want {want}"
            );
        }
    }
}

#[test]
fn cr10_role_inclusion_propagates_existential() {
    // r ⊑ s,  A ⊑ ∃r.B,  ∃s.B ⊑ C.  CR10 lifts the (A,B) ∈ R(r) link to R(s), so CR4 over
    // `∃s.B ⊑ C` derives A ⊑ C — a subsumption the E1 classifier (no role hierarchy) misses.
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         [ owl:onProperty :s ; owl:someValuesFrom :B ] rdfs:subClassOf :C ."
    );
    assert_closure(&ttl, &["A", "B", "C"], &[("A", "C")]);
}

#[test]
fn cr10_does_not_lift_downward() {
    // r ⊑ s,  A ⊑ ∃s.B,  ∃r.B ⊑ C.  The link is on s (a SUPER-role of r); CR10 only lifts
    // UPWARD, so it must NOT reach `∃r.B ⊑ C`. A ⊑ C must NOT be derived (soundness witness).
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :A rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :B ] .
         [ owl:onProperty :r ; owl:someValuesFrom :B ] rdfs:subClassOf :C ."
    );
    assert_closure(&ttl, &["A", "B", "C"], &[]);
}

#[test]
fn cr10_lifts_self_restriction_to_super_role() {
    // sq-l2o9e: r ⊑ s, A ⊑ ∃r.Self, ∃s.Self ⊑ D. The r-self loop is also
    // locally reflexive for its super-role s, so CR-Self-2 must derive A ⊑ D. A generic
    // existential `(A,A)` link remains insufficient; this fixture specifically carries
    // `ObjectHasSelf` provenance.
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :A rdfs:subClassOf [
             a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>
         ] .
         [
             a owl:Restriction ; owl:onProperty :s ; owl:hasSelf \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>
         ] rdfs:subClassOf :D ."
    );
    assert_closure(&ttl, &["A", "D"], &[("A", "D")]);
}

#[test]
fn cr11_transitive_role() {
    // r transitive (r ∘ r ⊑ r),  A ⊑ ∃r.B,  B ⊑ ∃r.D,  ∃r.D ⊑ E.
    // (A,B) ∈ R(r) and (B,D) ∈ R(r) compose to (A,D) ∈ R(r) (CR11), so CR4 over `∃r.D ⊑ E`
    // derives A ⊑ E. Also B ⊑ E directly via (B,D). Transitive-role reasoning is SNOMED-critical.
    let ttl = format!(
        "{PRE}
         :r rdf:type owl:TransitiveProperty .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :D ] .
         [ owl:onProperty :r ; owl:someValuesFrom :D ] rdfs:subClassOf :E ."
    );
    assert_closure(&ttl, &["A", "B", "D", "E"], &[("A", "E"), ("B", "E")]);
}

#[test]
fn cr11_transitive_three_hops_recomposes_derived_link() {
    // Stress the worklist: r transitive, A→B→C→D all via r. (A,B)∘(B,C)⇒(A,C), then the
    // DERIVED (A,C) must compose with (C,D) ⇒ (A,D) — a composed link re-feeding CR11. With
    // `∃r.D ⊑ E`, A ⊑ E only holds if the derived link recomposes. B ⊑ E and C ⊑ E too.
    let ttl = format!(
        "{PRE}
         :r rdf:type owl:TransitiveProperty .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :C ] .
         :C rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :D ] .
         [ owl:onProperty :r ; owl:someValuesFrom :D ] rdfs:subClassOf :E ."
    );
    assert_closure(
        &ttl,
        &["A", "B", "C", "D", "E"],
        &[("A", "E"), ("B", "E"), ("C", "E")],
    );
}

#[test]
fn cr11_property_chain_right_identity() {
    // The SNOMED right-identity shape: partOf ∘ isa ⊑ partOf — here r ∘ s ⊑ r.
    // A ⊑ ∃r.B,  B ⊑ ∃s.D,  ∃r.D ⊑ E.  (A,B)∈R(r) ∘ (B,D)∈R(s) ⇒ (A,D)∈R(r) (CR11),
    // so CR4 over `∃r.D ⊑ E` derives A ⊑ E.
    let ttl = format!(
        "{PRE}
         :r owl:propertyChainAxiom ( :r :s ) .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :D ] .
         [ owl:onProperty :r ; owl:someValuesFrom :D ] rdfs:subClassOf :E ."
    );
    assert_closure(&ttl, &["A", "B", "D", "E"], &[("A", "E")]);
}

#[test]
fn cr11_three_role_chain() {
    // Length-3 chain p ∘ q ∘ r ⊑ t (left-folded over a fresh role).
    // A ⊑ ∃p.B,  B ⊑ ∃q.C,  C ⊑ ∃r.D,  ∃t.D ⊑ G.  The three links compose to (A,D)∈R(t),
    // so CR4 over `∃t.D ⊑ G` derives A ⊑ G. No shorter prefix derives anything (t is the only
    // role with a `∃t.· ⊑ ·` axiom).
    let ttl = format!(
        "{PRE}
         :t owl:propertyChainAxiom ( :p :q :r ) .
         :A rdfs:subClassOf [ owl:onProperty :p ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :q ; owl:someValuesFrom :C ] .
         :C rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :D ] .
         [ owl:onProperty :t ; owl:someValuesFrom :D ] rdfs:subClassOf :G ."
    );
    assert_closure(&ttl, &["A", "B", "C", "D", "G"], &[("A", "G")]);
}

#[test]
fn cr10_transitive_inclusion_chain() {
    // r ⊑ s ⊑ t (a 2-level role hierarchy),  A ⊑ ∃r.B,  ∃t.B ⊑ C.
    // The reflexive-transitive closure lifts (A,B)∈R(r) all the way to R(t), so A ⊑ C.
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .  :s rdfs:subPropertyOf :t .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         [ owl:onProperty :t ; owl:someValuesFrom :B ] rdfs:subClassOf :C ."
    );
    assert_closure(&ttl, &["A", "B", "C"], &[("A", "C")]);
}

#[test]
fn cr11_bottom_propagates_through_composed_link() {
    // Composition feeds CR5 too: r transitive,  A ⊑ ∃r.B,  B ⊑ ∃r.D,  D ⊑ ⊥ (via disjointness).
    // (A,D)∈R(r) by composition, and ⊥∈S(D) ⇒ ⊥∈S(A) (CR5) — A and B are both unsatisfiable.
    let ttl = format!(
        "{PRE}
         :r rdf:type owl:TransitiveProperty .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :D ] .
         :D rdfs:subClassOf :P .  :D rdfs:subClassOf :Q .  :P owl:disjointWith :Q ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    let unsat = h.unsatisfiable_classes();
    assert!(unsat.contains(&iri(&dict, "D")), "D ⊑ ⊥ (P ⊓ Q ⊑ ⊥)");
    assert!(
        unsat.contains(&iri(&dict, "B")),
        "CR5: B ⊑ ∃r.D with D ⊑ ⊥ makes B ⊑ ⊥"
    );
    assert!(
        unsat.contains(&iri(&dict, "A")),
        "CR5 over the composed (A,D) link makes A ⊑ ⊥"
    );
}

#[test]
fn no_chain_no_spurious_composition() {
    // Sanity: WITHOUT any transitive/chain axiom, a two-hop existential structure must NOT
    // self-compose. r is a plain role; A ⊑ ∃r.B, B ⊑ ∃r.D, ∃r.D ⊑ E ⇒ only B ⊑ E (not A ⊑ E).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :D ] .
         [ owl:onProperty :r ; owl:someValuesFrom :D ] rdfs:subClassOf :E ."
    );
    assert_closure(&ttl, &["A", "B", "D", "E"], &[("B", "E")]);
}

// ---------------------------------------------------------------------------------------------
// sq-bif: RBox correctness-coverage additions. The fixtures above all use the LEFT
// (`r ∘ s ⊑ r`) right-identity and the transitive special case; the additions below close the
// distinct RIGHT-identity `r ∘ s ⊑ s` direction (the super-role is the SECOND chain component —
// indexed in `comp_by_second` not `comp_by_first`), a chain that fires through a SUPER-role of a
// composed link (CR10 over a CR11 result), and the honest-report invariant that an RBox axiom is
// NOT double-counted as a skipped non-EL class axiom under the active fragment.

#[test]
fn cr11_right_identity_super_is_second_component() {
    // r ∘ s ⊑ s — the OTHER right-identity (the existing `cr11_property_chain_right_identity` uses
    // r ∘ s ⊑ r, where the super-role is the FIRST component). Here the super-role `s` is the
    // SECOND chain component, so the link must compose onto R(s), not R(r). A ⊑ ∃r.B, B ⊑ ∃s.D,
    // ∃s.D ⊑ E: (A,B)∈R(r) ∘ (B,D)∈R(s) ⇒ (A,D)∈R(s), and CR4 over `∃s.D ⊑ E` derives A ⊑ E. B ⊑ E
    // holds directly via (B,D)∈R(s); no other named subsumption.
    let ttl = format!(
        "{PRE}
         :s owl:propertyChainAxiom ( :r :s ) .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :D ] .
         [ owl:onProperty :s ; owl:someValuesFrom :D ] rdfs:subClassOf :E ."
    );
    assert_closure(&ttl, &["A", "B", "D", "E"], &[("A", "E"), ("B", "E")]);
}

#[test]
fn cr10_lifts_a_cr11_composed_link_to_a_super_role() {
    // Compose THEN lift: t is transitive (t ∘ t ⊑ t) and t ⊑ u (a super-role). A ⊑ ∃t.B, B ⊑ ∃t.D
    // compose to (A,D) ∈ R(t) (CR11), which CR10 then lifts to (A,D) ∈ R(u). With `∃u.D ⊑ G` only
    // the lifted link reaches the axiom, so A ⊑ G holds ONLY if CR10 fires over the CR11 result. A
    // soundness+completeness witness for the interaction of the two role rules on a derived link.
    let ttl = format!(
        "{PRE}
         :t rdf:type owl:TransitiveProperty .  :t rdfs:subPropertyOf :u .
         :A rdfs:subClassOf [ owl:onProperty :t ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf [ owl:onProperty :t ; owl:someValuesFrom :D ] .
         [ owl:onProperty :u ; owl:someValuesFrom :D ] rdfs:subClassOf :G ."
    );
    assert_closure(&ttl, &["A", "B", "D", "G"], &[("A", "G"), ("B", "G")]);
}

#[test]
fn rbox_axioms_are_not_counted_as_skipped_class_axioms() {
    // With `rbox` ON, a role inclusion + a property chain are APPLIED, not skipped — so they must
    // NOT inflate `Report::skipped_axioms` (which counts only out-of-fragment CLASS axioms). The
    // sole class axiom here (A ⊑ B) is in-fragment, so the count must be exactly 0. Guards against a
    // regression that routed RBox triples through the class-axiom skip path.
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :t owl:propertyChainAxiom ( :r :s ) .
         :A rdfs:subClassOf :B ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    assert_eq!(
        h.report().skipped_axioms,
        0,
        "RBox axioms are applied under `rbox`, never counted as skipped class axioms"
    );
    assert!(
        h.is_subclass_of(iri(&dict, "A"), iri(&dict, "B")),
        "the in-fragment class axiom still classifies"
    );
}

// ---------------------------------------------------------------------------------------------
// sq-pbz04.2.7: role-lattice readoff — `classify_graph` under `rbox` emits the
// non-reflexive told-inclusion closure as `rdfs:subPropertyOf` triples.
//
// ORACLE: every emitted pair (r, s) holds in every model of the told inclusions (soundness),
// and every non-reflexive closure pair that can be derived by the transitive closure of the
// told `rdfs:subPropertyOf` axioms is present in the output (completeness of the readoff).
// Self-pairs (r ⊑ r) are NEVER emitted — they are semantically trivial and excluded by spec.
// Told pairs already in the input are NOT re-emitted (idempotency / deduplication).

fn full_iri(s: &str) -> OTerm {
    OTerm::NamedNode(NamedNode::new_unchecked(s.to_string()))
}

fn sub_property_of_id(dict: &Dict) -> Id {
    dict.lookup(&full_iri(
        "http://www.w3.org/2000/01/rdf-schema#subPropertyOf",
    ))
}

#[test]
fn role_readoff_chain_closure_emits_all_three_pairs() {
    // sq-pbz04.2.7: r ⊑ s ⊑ t (two told inclusions). The told-inclusion
    // closure contains: (r,s), (r,t), (s,t). All 3 non-reflexive pairs must be present in
    // `triples` after `classify_graph`. Exactly 1 triple is NEW (r⊑t is the derived one;
    // r⊑s and s⊑t were already in the input).
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :s rdfs:subPropertyOf :t .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);

    let sub_prop = sub_property_of_id(&dict);
    let r = iri(&dict, "r");
    let s = iri(&dict, "s");
    let t = iri(&dict, "t");

    // All 3 non-reflexive closure pairs are present.
    assert!(
        triples.contains(&[r, sub_prop, s]),
        "told r ⊑ s must be present"
    );
    assert!(
        triples.contains(&[r, sub_prop, t]),
        "derived transitive r ⊑ t must be emitted"
    );
    assert!(
        triples.contains(&[s, sub_prop, t]),
        "told s ⊑ t must be present"
    );

    // Self-pairs are NOT emitted.
    assert!(
        !triples.contains(&[r, sub_prop, r]),
        "self-pair r ⊑ r must NOT be emitted"
    );
    assert!(
        !triples.contains(&[s, sub_prop, s]),
        "self-pair s ⊑ s must NOT be emitted"
    );
    assert!(
        !triples.contains(&[t, sub_prop, t]),
        "self-pair t ⊑ t must NOT be emitted"
    );

    // Exactly 1 new triple (r⊑t is the only derived one; r⊑s and s⊑t were told).
    assert_eq!(
        report.emitted_role_subsumptions,
        1,
        "only the transitive r ⊑ t is a NEW triple (told pairs are already in the graph)"
    );
}

#[test]
fn role_readoff_mutual_inclusion_emits_both_directions() {
    // sq-pbz04.2.7: r ⊑ s and s ⊑ r (equivalent roles). Both directions must
    // be present in the output. Neither is a self-pair. Exactly 0 NEW triples (both directions
    // were told — the readoff covers them but adds nothing new).
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :s rdfs:subPropertyOf :r .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);

    let sub_prop = sub_property_of_id(&dict);
    let r = iri(&dict, "r");
    let s = iri(&dict, "s");

    assert!(
        triples.contains(&[r, sub_prop, s]),
        "told r ⊑ s must be present"
    );
    assert!(
        triples.contains(&[s, sub_prop, r]),
        "told s ⊑ r must be present"
    );
    // No self-pairs emitted.
    assert!(
        !triples.contains(&[r, sub_prop, r]),
        "self-pair r ⊑ r must NOT be emitted"
    );
    assert!(
        !triples.contains(&[s, sub_prop, s]),
        "self-pair s ⊑ s must NOT be emitted"
    );
    // Both were told — no new triples.
    assert_eq!(
        report.emitted_role_subsumptions,
        0,
        "both directions were told — readoff adds nothing new"
    );
}

#[test]
fn role_readoff_no_self_pairs_ever() {
    // sq-pbz04.2.7: a transitive role (r ∘ r ⊑ r) and a plain inclusion (r ⊑ s)
    // — the closure gives super_of[r] = {r, s}. The self-pair (r, r) must never appear in the
    // emitted triples, even though `r` is in its own `super_of` row (reflexivity). Likewise
    // for `s`.
    let ttl = format!(
        "{PRE}
         :r rdf:type owl:TransitiveProperty .
         :r rdfs:subPropertyOf :s .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    classify_graph(&mut dict, &mut triples);

    let sub_prop = sub_property_of_id(&dict);
    let r = iri(&dict, "r");
    let s = iri(&dict, "s");

    // The non-reflexive (r, s) pair is present.
    assert!(
        triples.contains(&[r, sub_prop, s]),
        "told r ⊑ s must be present"
    );
    // Self-pairs are NOT emitted, even for a transitive role.
    assert!(
        !triples.contains(&[r, sub_prop, r]),
        "self-pair r ⊑ r must NOT be emitted (transitivity is not self-inclusion)"
    );
    assert!(
        !triples.contains(&[s, sub_prop, s]),
        "self-pair s ⊑ s must NOT be emitted"
    );
}

#[test]
fn role_readoff_is_idempotent() {
    // sq-pbz04.2.7: a second `classify_graph` call adds no new role triples
    // (the `present`-set deduplication covers the derived pairs too).
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :s rdfs:subPropertyOf :t .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let r1 = classify_graph(&mut dict, &mut triples);
    assert_eq!(r1.emitted_role_subsumptions, 1, "first call derives r ⊑ t");
    let r2 = classify_graph(&mut dict, &mut triples);
    assert_eq!(
        r2.emitted_role_subsumptions,
        0,
        "second call adds nothing (idempotent)"
    );
}

// sq-oj06v: told-RBox regularity — the honest non-regular report (E2 follow-up).
// The saturation is sound + terminating on ANY input, but EL+ completeness assumes a REGULAR
// RBox (the OWL 2 global restrictions forbid non-regular ones); `Report::rbox_non_regular` is
// the flag that keeps a spec-illegal input from being silently classified as if complete.

#[test]
fn regular_rbox_is_not_flagged() {
    // Hierarchy + transitivity + a plain chain + SNOMED right identity: all regular.
    let ttl = format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :t rdf:type owl:TransitiveProperty .
         :hasUncle owl:propertyChainAxiom ( :hasFather :hasBrother ) .
         :partOf owl:propertyChainAxiom ( :properPartOf :partOf ) .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let (d2, t2) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);
    assert!(!report.rbox_non_regular, "a regular RBox must not be flagged");
    // The typed (non-mutating) entry reports identically.
    let h = Classifier::classify(&d2, &t2);
    assert!(!h.report().rbox_non_regular);
}

#[test]
fn non_regular_chain_cycle_is_flagged() {
    // The spec's classic non-regular pair: hasUncle depends on hasBrother AND hasBrother
    // depends on hasUncle through chain constraints — no strict role order admits both.
    let ttl = format!(
        "{PRE}
         :hasUncle owl:propertyChainAxiom ( :hasFather :hasBrother ) .
         :hasBrother owl:propertyChainAxiom ( :hasUncle :hasFriend ) .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let (d2, t2) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);
    assert!(
        report.rbox_non_regular,
        "a chain-constraint cycle must set rbox_non_regular"
    );
    let h = Classifier::classify(&d2, &t2);
    assert!(h.report().rbox_non_regular, "typed entry flags it too");
}

#[test]
fn non_regular_chain_fed_back_through_inclusion_is_flagged() {
    // r ∘ q ⊑ s with s ⊑ r: the chain constraint r < s cycles back through the told
    // inclusion — flagged by the (conservative) hierarchy-aware detection.
    let ttl = format!(
        "{PRE}
         :s owl:propertyChainAxiom ( :r :q ) .
         :s rdfs:subPropertyOf :r .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);
    assert!(report.rbox_non_regular);
}

#[test]
fn told_left_identity_long_chain_is_regular() {
    // s ∘ p ∘ q ⊑ s — a TOLD left identity (admissible: p < s, q < s). Its left-folded
    // BINARIZED form (s ∘ p ⊑ fresh, fresh ∘ q ⊑ s) looks like an s → fresh → s cycle; this
    // pins that the regularity check runs on the told chain, not the binarization.
    let ttl = format!(
        "{PRE}
         :s owl:propertyChainAxiom ( :s :p :q ) .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);
    assert!(
        !report.rbox_non_regular,
        "a told left-identity chain is regular — binarization must not leak into the check"
    );
}

#[test]
fn top_superproperty_chain_is_regular() {
    // The OWL 2 property-hierarchy restriction's FIRST case: every chain axiom whose
    // superproperty is owl:topObjectProperty is admissible. The ternary top self-chain would
    // false-positive via the `s < s` self-constraint without the exemption (binary transitivity
    // does not cover n = 3).
    let ttl = format!(
        "{PRE}
         owl:topObjectProperty owl:propertyChainAxiom
             ( owl:topObjectProperty owl:topObjectProperty owl:topObjectProperty ) .
         :A rdfs:subClassOf :B ."
    );
    let (mut dict, mut triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let (d2, t2) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let report = classify_graph(&mut dict, &mut triples);
    assert!(
        !report.rbox_non_regular,
        "a top-superproperty chain is normatively admissible, never non-regular"
    );
    let h = Classifier::classify(&d2, &t2);
    assert!(!h.report().rbox_non_regular, "typed entry agrees");
}

#[test]
fn non_regular_saturation_still_terminates_and_stays_sound() {
    // On a (spec-illegal) non-regular RBox the classifier still terminates and every emitted
    // subsumption is sound; the flag is the ONLY behavioural difference. The chain cycle here
    // still derives A ⊑ FoundUncle soundly via hasFather ∘ hasBrother ⊑ hasUncle.
    let ttl = format!(
        "{PRE}
         :hasUncle owl:propertyChainAxiom ( :hasFather :hasBrother ) .
         :hasBrother owl:propertyChainAxiom ( :hasUncle :hasFriend ) .
         :A rdfs:subClassOf [ owl:onProperty :hasFather ; owl:someValuesFrom :M ] .
         :M rdfs:subClassOf [ owl:onProperty :hasBrother ; owl:someValuesFrom :U ] .
         [ owl:onProperty :hasUncle ; owl:someValuesFrom :U ] rdfs:subClassOf :FoundUncle ."
    );
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let h = Classifier::classify(&dict, &triples);
    assert!(h.report().rbox_non_regular, "non-regular input is flagged");
    assert!(
        h.is_subclass_of(iri(&dict, "A"), iri(&dict, "FoundUncle")),
        "CR11 stays sound: A ⊑ FoundUncle via the composed chain"
    );
}

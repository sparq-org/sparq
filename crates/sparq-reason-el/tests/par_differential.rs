// sq-wy3i6 (Phase E4): the parallel-saturation differential + determinism oracles.
//
// INVARIANT UNDER TEST (the bead's acceptance oracle): for every input ontology and EVERY
// thread count, the feature-ON parallel engine (`Classifier::classify_par` /
// `classify_graph_par`) derives EXACTLY the closure the single-threaded engine derives —
// no derivation missed (completeness), none spurious (soundness) — and repeated runs under
// the same thread count are bit-identical (determinism; a nondeterministic race that drops
// or reorders a rule application fails loudly here).
//
// The comparison is BIT-LEVEL, not merely set-level: both engines run over clones of the
// SAME parsed `(Dict, Vec<[Id;3]>)`, so `classify_graph[_par]` must produce identical triple
// VECTORS (content and order) and identical `Report`s, and `classify[_par]` must agree on
// `super_classes` / `unsatisfiable_classes` for every named class in the input.
//
// Fixture corpus: every CR surface the crate reasons over — CR1 (told inclusion), CR2
// (conjunction), CR3/CR4 (existential traversal, incl. the vendored `el_smoke.ttl` bench
// ontology's RL-unreachable case), CR5/⊥ (disjointness clash + propagation through links),
// CR6 (safe nominals + reachability side-condition), CR-Self, plus deep/wide synthetic
// ontologies sized to force MANY multi-chunk parallel rounds. The `rbox` (CR10/CR11) and
// `cdomain` (CR7–CR9) interplays are covered by additional fixtures under
// `cfg(all(feature = "par", feature = "..."))` — run by the combined feature-matrix leg.
#![cfg(feature = "par")]

use std::num::NonZeroUsize;

use oxrdf::{NamedNode, Term as OTerm};
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason_el::{
    classify_graph, classify_graph_par, classify_graph_par_stats, Classifier, ParPhaseStats,
};

const PRE: &str = r#"
    @prefix : <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

/// Thread counts every fixture is exercised at: sequential-through-the-round-loop (1), the
/// smallest genuinely concurrent pool (2), and an oversubscribed pool (8) so chunk
/// boundaries land in many different places.
const THREAD_COUNTS: [usize; 3] = [1, 2, 8];

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("thread count is non-zero")
}

/// The differential oracle: parse once, then assert `classify_graph_par` at every thread
/// count reproduces `classify_graph`'s output triples (bit-identical vector — same dict, so
/// id-level equality IS term-level equality), `Report`, and per-class API answers.
fn assert_par_equals_seq(ttl: &str) {
    let (dict0, triples0) = Graph::parse_to_triples(ttl, "turtle").expect("parse");

    // Sequential reference run.
    let (mut dict_seq, mut triples_seq) = (dict0.clone(), triples0.clone());
    let report_seq = classify_graph(&mut dict_seq, &mut triples_seq);
    let h_seq = Classifier::classify(&dict0, &triples0);

    // Named classes to compare the typed API over: every distinct id in the input.
    let ids: std::collections::BTreeSet<Id> = triples0.iter().flatten().copied().collect();

    for t in THREAD_COUNTS {
        let (mut dict_par, mut triples_par) = (dict0.clone(), triples0.clone());
        let report_par = classify_graph_par(&mut dict_par, &mut triples_par, nz(t));
        assert_eq!(
            report_par, report_seq,
            "classify_graph_par(threads={t}) Report differs from sequential"
        );
        assert_eq!(
            triples_par, triples_seq,
            "classify_graph_par(threads={t}) emitted triples differ from sequential"
        );

        let h_par = Classifier::classify_par(&dict0, &triples0, nz(t));
        assert_eq!(
            h_par.unsatisfiable_classes(),
            h_seq.unsatisfiable_classes(),
            "classify_par(threads={t}) unsatisfiable set differs"
        );
        for &id in &ids {
            assert_eq!(
                h_par.super_classes(id),
                h_seq.super_classes(id),
                "classify_par(threads={t}) super_classes({id:?}) differs"
            );
        }
    }
}

// --- The vendored EL smoke ontology (the bead-named CR4 existential-subsumption case) -----

#[test]
fn el_smoke_ontology_par_matches_seq() {
    assert_par_equals_seq(include_str!("../examples/data/el_smoke.ttl"));
}

// --- Per-rule fixtures (mirroring the sequential differential suite's CR coverage) --------

#[test]
fn cr1_told_chain_par_matches_seq() {
    assert_par_equals_seq(&format!(
        "{PRE} :A rdfs:subClassOf :B . :B rdfs:subClassOf :C . :C rdfs:subClassOf :D ."
    ));
}

#[test]
fn cr2_conjunction_par_matches_seq() {
    assert_par_equals_seq(&format!(
        "{PRE}
         [ owl:intersectionOf ( :A :B ) ] rdfs:subClassOf :C .
         [ owl:intersectionOf ( :C :D ) ] rdfs:subClassOf :E .
         :X rdfs:subClassOf :A . :X rdfs:subClassOf :B . :X rdfs:subClassOf :D ."
    ));
}

#[test]
fn cr4_existential_traversal_par_matches_seq() {
    // The spike §1.3 shape, plus a second hop so the traversal chains: the CR4 conclusion of
    // one link is the CR3 trigger of the next.
    assert_par_equals_seq(&format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :C .
         [ owl:onProperty :r ; owl:someValuesFrom :C ] rdfs:subClassOf :D .
         :D rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :E ] .
         [ owl:onProperty :s ; owl:someValuesFrom :E ] rdfs:subClassOf :F ."
    ));
}

#[test]
fn cr4_membership_triggered_arm_with_delayed_filler() {
    // MUTATION-WITNESS-CRITICAL fixture: the filler membership arrives many rounds AFTER the
    // last link to it was inserted, so the link-insertion rescan inside `add_link` can NEVER
    // see it — ONLY the membership-triggered CR4 arm of the parallel kernel can fire the
    // conclusion. A ⊑ ∃r.B, B ⊑ M1 ⊑ … ⊑ M12, ∃r.M12 ⊑ E ⊨ A ⊑ E: the (r, A→B) link is
    // created in round 1 (and never re-created — no other existential axiom touches it),
    // while M12 ∈ S(B) lands ~12 rounds later. Knocking out the membership-triggered CR4
    // arm in `derive_chunk` loses A ⊑ E (verified red under that mutation).
    use std::fmt::Write as _;
    let mut ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         :B rdfs:subClassOf :M1 .
         [ owl:onProperty :r ; owl:someValuesFrom :M12 ] rdfs:subClassOf :E ."
    );
    for i in 1..12 {
        let _ = writeln!(ttl, ":M{i} rdfs:subClassOf :M{j} .", j = i + 1);
    }
    assert_par_equals_seq(&ttl);
    // Positive (non-vacuous) assert too, so a bug SHARED by both engines cannot hide in the
    // differential: the delayed-filler CR4 conclusion must actually be derived.
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let lk = |s: &str| {
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
            "http://ex/{s}"
        ))))
    };
    for t in THREAD_COUNTS {
        let h = Classifier::classify_par(&dict, &triples, nz(t));
        assert!(
            h.is_subclass_of(lk("A"), lk("E")),
            "threads={t}: delayed-filler CR4 must derive A ⊑ E"
        );
    }
}

#[test]
fn cr5_bottom_propagates_through_links_par_matches_seq() {
    // X clashes (disjoint conjunction); ⊥ must propagate to every r-predecessor of X in both
    // engines (CR5's membership- AND link-triggered halves).
    assert_par_equals_seq(&format!(
        "{PRE}
         :A owl:disjointWith :B .
         :X rdfs:subClassOf :A . :X rdfs:subClassOf :B .
         :P rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :X ] .
         :Q rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :P ] ."
    ));
}

#[test]
fn cr6_safe_nominals_par_matches_seq() {
    // Shared nominal filler on both sides of ⊑ (the CR6 merge) + a nominal-rooted concept,
    // exercising the reachability side-condition in both engines.
    assert_par_equals_seq(&format!(
        "{PRE}
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:hasValue :a ] .
         [ owl:onProperty :r ; owl:hasValue :a ] rdfs:subClassOf :B .
         :C rdfs:subClassOf [ owl:oneOf ( :b ) ] .
         :D rdfs:subClassOf [ owl:oneOf ( :b ) ] .
         :C rdfs:subClassOf [ owl:onProperty :s ; owl:someValuesFrom :D ] ."
    ));
}

#[test]
fn cr_self_restriction_par_matches_seq() {
    // CRs1/CRs2 (owl:hasSelf) + the general-self-link negative case in one fixture. The
    // `:A ⊑ :B` + `∃r.B ⊑ :F` pair makes the CRs1 REFLEXIVE LINK itself load-bearing
    // (A ⊑ F needs (A,A) ∈ R(r) feeding CR4, not just the CRs2-via-CR1 readoff) — knocking
    // out the CRs1 arm in the par kernel fails this test (mutation-witnessed).
    let ttl = format!(
        "{PRE}
         :A rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^xsd:boolean ] .
         [ a owl:Restriction ; owl:onProperty :r ; owl:hasSelf \"true\"^^xsd:boolean ] rdfs:subClassOf :D .
         :A rdfs:subClassOf :B .
         [ a owl:Restriction ; owl:onProperty :r ; owl:someValuesFrom :B ] rdfs:subClassOf :F .
         :B2 rdfs:subClassOf [ a owl:Restriction ; owl:onProperty :s ; owl:someValuesFrom :B2 ] .
         [ a owl:Restriction ; owl:onProperty :s ; owl:hasSelf \"true\"^^xsd:boolean ] rdfs:subClassOf :E ."
    );
    assert_par_equals_seq(&ttl);
    // Positive assert so a shared miss cannot hide in the differential.
    let (dict, triples) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");
    let lk = |s: &str| {
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
            "http://ex/{s}"
        ))))
    };
    for t in THREAD_COUNTS {
        let h = Classifier::classify_par(&dict, &triples, nz(t));
        assert!(
            h.is_subclass_of(lk("A"), lk("F")),
            "threads={t}: the CRs1 reflexive link must feed CR4 (A ⊑ F)"
        );
    }
}

#[test]
fn skipped_axiom_counting_par_matches_seq() {
    // Out-of-fragment constructs: the Report::skipped_axioms tally (extraction-level, shared
    // by both engines) must survive the par path unchanged.
    assert_par_equals_seq(&format!(
        "{PRE}
         [ owl:unionOf ( :A :B ) ] rdfs:subClassOf :C .
         [ owl:oneOf ( :a :b ) ] rdfs:subClassOf :D .
         :E rdfs:subClassOf :F ."
    ));
}

// --- rbox (CR10/CR11) interplay — run by the combined `par,rbox,cdomain` matrix leg -------

#[cfg(feature = "rbox")]
#[test]
fn rbox_chain_and_transitivity_par_matches_seq() {
    // Role hierarchy + a property chain + a transitive role: the derived links from
    // `add_link_rbox` (sequential apply phase) must compose identically with the parallel
    // frontier rounds. locatedIn ∘ partOf ⊑ locatedIn is the SNOMED right-identity shape.
    assert_par_equals_seq(&format!(
        "{PRE}
         :r rdfs:subPropertyOf :s .
         :locatedIn owl:propertyChainAxiom ( :locatedIn :partOf ) .
         :t a owl:TransitiveProperty .
         :A rdfs:subClassOf [ owl:onProperty :r ; owl:someValuesFrom :B ] .
         [ owl:onProperty :s ; owl:someValuesFrom :B ] rdfs:subClassOf :C .
         :X rdfs:subClassOf [ owl:onProperty :locatedIn ; owl:someValuesFrom :Y ] .
         :Y rdfs:subClassOf [ owl:onProperty :partOf ; owl:someValuesFrom :Z ] .
         [ owl:onProperty :locatedIn ; owl:someValuesFrom :Z ] rdfs:subClassOf :W .
         :P rdfs:subClassOf [ owl:onProperty :t ; owl:someValuesFrom :Q ] .
         :Q rdfs:subClassOf [ owl:onProperty :t ; owl:someValuesFrom :R2 ] .
         [ owl:onProperty :t ; owl:someValuesFrom :R2 ] rdfs:subClassOf :V ."
    ));
}

// --- cdomain (CR7–CR9) interplay — extraction-level, must survive the par path ------------

#[cfg(feature = "cdomain")]
#[test]
fn cdomain_facets_par_matches_seq() {
    // An UNSATISFIABLE faceted range (min > max ⇒ clash via CR5) and a SATISFIABLE
    // containment-threaded one: extraction emits the same normal axioms for both engines;
    // the differential pins that the par fixpoint reasons over them identically.
    assert_par_equals_seq(&format!(
        "{PRE}
         :Bad rdfs:subClassOf
           [ owl:onProperty :age ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 10 ] [ xsd:maxInclusive 5 ] ) ] ] .
         :Adult rdfs:subClassOf
           [ owl:onProperty :age ; owl:someValuesFrom
             [ owl:onDatatype xsd:integer ;
               owl:withRestrictions ( [ xsd:minInclusive 18 ] ) ] ] .
         :G rdfs:subClassOf :H ."
    ));
}

// --- Synthetic scale: force MANY multi-chunk rounds --------------------------------------

/// Deterministically generates a non-trivial EL ontology (no randomness — wall-clock and
/// RNG are banned oracles here): `width` parallel told-chains of `depth` classes laced with
/// cross-chain existentials (every chain step also points into the next chain via `∃r`),
/// conjunction axioms joining adjacent chains, one disjointness clash seeded at the deepest
/// level of chain 0, and a shared nominal filler between the last two chains. The closure
/// is large and derivation-order-sensitive enough that a dropped or duplicated rule firing
/// (a race) is overwhelmingly likely to change the derived set.
fn synthetic_ontology(width: usize, depth: usize) -> String {
    use std::fmt::Write as _;
    let mut ttl = String::from(PRE);
    for w in 0..width {
        for d in 0..depth {
            // Told chain: C{w}_{d} ⊑ C{w}_{d+1}.
            if d + 1 < depth {
                let _ = writeln!(ttl, ":C{w}_{d} rdfs:subClassOf :C{w}_{e} .", e = d + 1);
            }
            // Cross-chain existential: C{w}_{d} ⊑ ∃r{w}.C{w2}_{d}, and the matching
            // ∃r{w}.TOP-of-chain ⊑ B{w}_{d} so CR4 has something to conclude.
            let w2 = (w + 1) % width;
            let _ = writeln!(
                ttl,
                ":C{w}_{d} rdfs:subClassOf [ owl:onProperty :r{w} ; owl:someValuesFrom :C{w2}_{d} ] ."
            );
            let _ = writeln!(
                ttl,
                "[ owl:onProperty :r{w} ; owl:someValuesFrom :C{w2}_{last} ] rdfs:subClassOf :B{w}_{d} .",
                last = depth - 1
            );
        }
        // Conjunction joining adjacent chains' roots.
        let w2 = (w + 1) % width;
        let _ = writeln!(
            ttl,
            "[ owl:intersectionOf ( :B{w}_0 :B{w2}_0 ) ] rdfs:subClassOf :J{w} ."
        );
    }
    // A clash at the deepest level of chain 0: everything told-below it goes to ⊥.
    let _ = writeln!(ttl, ":C0_{last} owl:disjointWith :D0 .", last = depth - 1);
    let _ = writeln!(ttl, ":C0_0 rdfs:subClassOf :D0 .");
    // Safe nominal shared between the last two chains (CR6 under load).
    let _ = writeln!(
        ttl,
        ":C{a}_0 rdfs:subClassOf [ owl:onProperty :n ; owl:hasValue :nom ] .",
        a = width - 1
    );
    let _ = writeln!(
        ttl,
        "[ owl:onProperty :n ; owl:hasValue :nom ] rdfs:subClassOf :C{b}_0 .",
        b = width.saturating_sub(2)
    );
    ttl
}

#[test]
fn synthetic_scale_par_matches_seq() {
    // 6 chains × 40 deep = 240 chain classes + ~480 existential axioms: frontiers of several
    // hundred memberships per round, well past the inline threshold, so the scoped worker
    // pool genuinely splits chunks at threads = 2 and 8.
    assert_par_equals_seq(&synthetic_ontology(6, 40));
}

#[test]
fn determinism_stress_repeated_runs_are_bit_identical() {
    // The bead's determinism oracle: repeat a non-trivial ontology under a concurrent pool
    // and assert bit-identical output EVERY run (catches nondeterministic races that a
    // single differential pass could miss), and identical to the sequential engine.
    let ttl = synthetic_ontology(5, 30);
    let (dict0, triples0) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");

    let (mut dict_seq, mut triples_seq) = (dict0.clone(), triples0.clone());
    let report_seq = classify_graph(&mut dict_seq, &mut triples_seq);

    for run in 0..10 {
        let (mut dict_par, mut triples_par) = (dict0.clone(), triples0.clone());
        let report_par = classify_graph_par(&mut dict_par, &mut triples_par, nz(4));
        assert_eq!(
            report_par, report_seq,
            "run {run}: parallel Report diverged from sequential"
        );
        assert_eq!(
            triples_par, triples_seq,
            "run {run}: parallel emitted triples diverged from sequential"
        );
    }
}

#[test]
fn par_thread_count_does_not_change_the_answer_api() {
    // Spot-check the typed API on the smoke ontology: the RL-unreachable CR4 subsumption
    // (Neuron ⊑ NucleatedCell) must hold at every thread count — a non-vacuous positive
    // (knocking out CR4 derivation in the par kernel fails this, mutation-witnessed).
    let ttl = include_str!("../examples/data/el_smoke.ttl");
    let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
    let lk = |s: &str| {
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(format!(
            "http://sparq.dev/bench/reason-el/smoke#{s}"
        ))))
    };
    let (neuron, nucleated) = (lk("Neuron"), lk("NucleatedCell"));
    for t in THREAD_COUNTS {
        let h = Classifier::classify_par(&dict, &triples, nz(t));
        assert!(
            h.is_subclass_of(neuron, nucleated),
            "threads={t}: CR4 must derive Neuron ⊑ NucleatedCell"
        );
    }
}

// --- sq-q0o82 (E4 follow-up): the phase-attribution surface ------------------------------
//
// `classify_graph_par_stats` is `classify_graph_par` plus the compute/apply split. Two
// obligations: the instrumentation must not perturb the classification (same Report, same
// triples, byte for byte), and the DERIVATION-WORK counters must be a function of the input
// ALONE — chunking decides which worker derives a conclusion, never which conclusions are
// derived. Both are asserted here so a phase number can never be reported for a run that
// silently derived something else, and so a partitioning bug cannot hide behind "scheduling".

#[test]
fn par_phase_stats_does_not_perturb_the_classification() {
    let ttl = synthetic_ontology(5, 30);
    let (dict0, triples0) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");

    let (mut dict_seq, mut triples_seq) = (dict0.clone(), triples0.clone());
    let report_seq = classify_graph(&mut dict_seq, &mut triples_seq);

    for t in THREAD_COUNTS {
        let (mut dict_par, mut triples_par) = (dict0.clone(), triples0.clone());
        let (report, stats) = classify_graph_par_stats(&mut dict_par, &mut triples_par, nz(t));
        assert_eq!(
            report, report_seq,
            "threads={t}: classify_graph_par_stats Report differs from sequential"
        );
        assert_eq!(
            triples_par, triples_seq,
            "threads={t}: classify_graph_par_stats emitted triples differ from sequential"
        );
        // Non-vacuous: this ontology genuinely drives the round loop, so a stats surface that
        // silently reported zeros (or was never wired into the loop) fails here.
        assert!(
            stats.rounds > 0 && stats.frontier_items > 0 && stats.derived_members > 0,
            "threads={t}: expected non-zero phase work, got {stats:?}"
        );
        // The fixture's cross-chain existentials mean CR3 must have concluded links too.
        assert!(
            stats.derived_links > 0,
            "threads={t}: expected CR3 link derivations, got {stats:?}"
        );
    }
}

#[test]
fn par_phase_stats_work_counts_are_thread_count_invariant() {
    let ttl = synthetic_ontology(6, 40);
    let (dict0, triples0) = Graph::parse_to_triples(&ttl, "turtle").expect("parse");

    let mut reference: Option<(u64, u64, u64, u64)> = None;
    for t in THREAD_COUNTS {
        let (mut dict_par, mut triples_par) = (dict0.clone(), triples0.clone());
        let (_report, s) = classify_graph_par_stats(&mut dict_par, &mut triples_par, nz(t));
        let counts = (s.rounds, s.frontier_items, s.derived_members, s.derived_links);
        match reference {
            None => reference = Some(counts),
            Some(r) => assert_eq!(
                counts, r,
                "threads={t}: derivation work counts drifted from the threads={} run",
                THREAD_COUNTS[0]
            ),
        }
    }
}

#[test]
fn apply_fraction_is_the_sequential_share_of_measured_time() {
    // Direct unit test of the decision metric — constructed values, so it is exact and
    // independent of any clock.
    let s = ParPhaseStats {
        compute_nanos: 1,
        apply_nanos: 3,
        ..ParPhaseStats::default()
    };
    assert_eq!(s.apply_fraction(), 0.75);

    let all_compute = ParPhaseStats {
        compute_nanos: 8,
        ..ParPhaseStats::default()
    };
    assert_eq!(all_compute.apply_fraction(), 0.0);

    let all_apply = ParPhaseStats {
        apply_nanos: 8,
        ..ParPhaseStats::default()
    };
    assert_eq!(all_apply.apply_fraction(), 1.0);

    // Nothing measured (an empty ontology derives nothing): defined as 0.0, never a NaN that
    // would silently poison a comparison downstream.
    assert_eq!(ParPhaseStats::default().apply_fraction(), 0.0);
}

#[test]
fn apply_fraction_stays_in_range_when_the_timers_saturate_u64() {
    // The timer fields are public, so `compute_nanos + apply_nanos` can exceed `u64::MAX`.
    // The denominator must not overflow: that would panic in debug and wrap to a
    // divide-by-zero (infinity/NaN) in release, making the documented `0.0..=1.0` contract
    // build-profile-dependent.
    let compute_heavy = ParPhaseStats {
        compute_nanos: u64::MAX,
        apply_nanos: 1,
        ..ParPhaseStats::default()
    };
    let f = compute_heavy.apply_fraction();
    assert!(f.is_finite(), "{:?} -> {}", compute_heavy, f);
    assert!((0.0..=1.0).contains(&f), "{:?} -> {}", compute_heavy, f);
    // 1 / 2^64, exactly representable in f64.
    assert_eq!(f, 1.0 / 18_446_744_073_709_551_616.0);

    let both_saturated = ParPhaseStats {
        compute_nanos: u64::MAX,
        apply_nanos: u64::MAX,
        ..ParPhaseStats::default()
    };
    assert_eq!(both_saturated.apply_fraction(), 0.5);

    let apply_heavy = ParPhaseStats {
        compute_nanos: 1,
        apply_nanos: u64::MAX,
        ..ParPhaseStats::default()
    };
    let f = apply_heavy.apply_fraction();
    assert!(f.is_finite(), "{:?} -> {}", apply_heavy, f);
    assert!((0.0..=1.0).contains(&f), "{:?} -> {}", apply_heavy, f);
}

#[test]
fn empty_graph_par_phase_stats_derives_nothing() {
    let (mut dict, mut triples) = (Dict::new(), Vec::<[Id; 3]>::new());
    let (report, stats) = classify_graph_par_stats(&mut dict, &mut triples, nz(4));
    assert_eq!(report.emitted_subsumptions, 0);
    // The seeding pass still queues the reflexive/⊤ memberships of the always-present ⊤/⊥
    // concepts, so `frontier_items` is NOT zero — but with no axioms to fire, the compute
    // phase must conclude nothing at all.
    assert_eq!(stats.derived_members, 0, "{stats:?}");
    assert_eq!(stats.derived_links, 0, "{stats:?}");
}

#[test]
fn empty_graph_par_is_safe() {
    let (dict, triples) = (Dict::new(), Vec::<[Id; 3]>::new());
    for t in THREAD_COUNTS {
        let h = Classifier::classify_par(&dict, &triples, nz(t));
        assert_eq!(h.report().emitted_subsumptions, 0);
        assert!(h.unsatisfiable_classes().is_empty());
    }
}

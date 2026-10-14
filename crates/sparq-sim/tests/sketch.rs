#![cfg(feature = "sketch")]
// sq-lgw: the MinHash/LSH sketch index (dense-graph candidate generation).
//
// The load-bearing obligations, in order:
//   - RESULT EXACTNESS: every (entity, score) `SketchIndex::most_similar` returns must
//     equal `Sim::similarity(a, entity)` EXACTLY (a randomized differential over
//     pseudo-random graphs, plus a hand-computed pin whose corruption goes red);
//   - GUARANTEED RECALL where the math guarantees it: identical-signature twins share
//     an identical sketch, so they collide in EVERY band and are always found;
//   - DECLINE: absent terms, literals, and empty-signature entities are never indexed
//     and yield empty results / 0.0 estimates — same contract shape as `Sim`.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_sim::{Sim, SimConfig, SketchConfig};

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://ex.org/{s}")).unwrap())
}

fn graph(ttl: &str) -> Graph {
    let prefix = "@prefix : <http://ex.org/> .\n";
    Graph::load_str(&format!("{prefix}{ttl}"), "turtle").unwrap()
}

fn unweighted(g: &Graph) -> Sim<'_> {
    Sim::with_config(g, SimConfig { idf: false, ..SimConfig::default() })
}

/// Direct `SketchConfig::default` witness: the documented defaults, and two builds
/// from the same config produce identical results (determinism — fixed seed).
#[test]
fn default_config_is_documented_and_deterministic() {
    let cfg = SketchConfig::default();
    assert_eq!(cfg.num_hashes, 128);
    assert_eq!(cfg.bands, 32);

    let g = graph(":a :p :x, :y . :b :p :x, :y . :c :p :x ; :q :z .");
    let sim = unweighted(&g);
    let run = || {
        sim.sketch_index(SketchConfig::default())
            .most_similar(&iri("a"), 5)
            .into_iter()
            .map(|(t, s)| (t.to_string(), s))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

/// Direct `Sim::sketch_index` + `len`/`is_empty` witnesses: the index holds exactly
/// the entities with a non-empty signature — literals never, and excluding an
/// entity's only predicate removes it.
#[test]
fn index_len_counts_nonempty_entity_signatures() {
    let g = graph(":a :p :x . :b :p \"literal\" .");
    let sim = unweighted(&g);
    let idx = sim.sketch_index(SketchConfig::default());
    // a, b, x are entities; the literal is not (it is a NEIGHBOR in b's signature).
    assert_eq!(idx.len(), 3);
    assert!(!idx.is_empty());

    let no_p = Sim::with_config(
        &g,
        SimConfig {
            idf: false,
            exclude_predicates: vec![NamedNode::new("http://ex.org/p").unwrap()],
            ..SimConfig::default()
        },
    );
    let idx = no_p.sketch_index(SketchConfig::default());
    assert!(idx.is_empty(), "every signature is empty once :p is excluded");
    assert_eq!(idx.len(), 0);
    assert!(idx.most_similar(&iri("a"), 5).is_empty());
}

/// Identical-signature twins have identical sketches, so they collide in every band:
/// found with probability 1, scored exactly 1.0 — and the estimate is exactly 1.0.
#[test]
fn twins_are_always_found_with_exact_scores() {
    let g = graph(
        ":a :knows :bob, :eve ; :worksAt :acme .
         :twin :knows :bob, :eve ; :worksAt :acme .
         :half :knows :bob ; :worksAt :other .
         :far :plays :chess .",
    );
    let sim = unweighted(&g);
    let idx = sim.sketch_index(SketchConfig::default());

    let top = idx.most_similar(&iri("a"), 10);
    let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
    assert_eq!(names[0], "<http://ex.org/twin>");
    assert_eq!(top[0].1, 1.0);
    assert!(!names.iter().any(|n| n == "<http://ex.org/a>"), "self must be excluded");
    assert!(
        !names.iter().any(|n| n == "<http://ex.org/far>"),
        "a zero-overlap entity must never be returned: {names:?}"
    );
    // Every returned score must equal the exact pairwise similarity.
    for (t, s) in &top {
        assert_eq!(*s, sim.similarity(&iri("a"), t), "inexact score for {t}");
    }
    assert_eq!(idx.estimate(&iri("a"), &iri("twin")), 1.0);
    assert_eq!(idx.estimate(&iri("a"), &iri("a")), 1.0);
}

/// Direct `estimate` witnesses: bounds, symmetry, disjoint-set zero (64-bit hashes —
/// no component collision at this scale under the fixed default seed), and the 0.0
/// decline for terms outside the index.
#[test]
fn estimate_bounds_symmetry_and_decline() {
    let g = graph(":a :p :x . :b :p :x . :c :q :z . :s :r \"lit\" .");
    let sim = unweighted(&g);
    let idx = sim.sketch_index(SketchConfig::default());

    assert_eq!(idx.estimate(&iri("a"), &iri("b")), 1.0, "identical signatures");
    assert_eq!(idx.estimate(&iri("a"), &iri("c")), 0.0, "disjoint signatures");
    let e = idx.estimate(&iri("a"), &iri("x"));
    assert_eq!(e, idx.estimate(&iri("x"), &iri("a")), "estimate must be symmetric");
    assert!((0.0..=1.0).contains(&e));
    // Decline: absent term, and a literal (never indexed).
    assert_eq!(idx.estimate(&iri("a"), &iri("missing")), 0.0);
    let lit = Term::Literal(oxrdf::Literal::new_simple_literal("lit"));
    assert_eq!(idx.estimate(&iri("a"), &lit), 0.0);
    assert!(idx.most_similar(&iri("missing"), 5).is_empty());
    assert!(idx.most_similar(&lit, 5).is_empty());
    assert!(idx.most_similar(&iri("a"), 0).is_empty());
}

/// Hand-computed exactness pin (the mutation check): `:alice` vs `:dave` share exactly
/// one of four union elements → the returned score must be exactly 0.25. Any
/// corruption of the exact re-scoring (e.g. returning the sketch ESTIMATE, or a
/// wrong-signature jaccard) goes red here, because with 128 components the estimate of
/// a 1/4-overlap set almost surely differs from 0.25 exactly.
#[test]
fn hand_computed_score_pins_exact_rescoring() {
    let g = graph(":alice :knows :bob, :eve ; :worksAt :acme . :dave :knows :eve ; :plays :chess .");
    let sim = unweighted(&g);
    // rows = 1 → any shared component makes a candidate (maximal-recall banding).
    let idx = sim.sketch_index(SketchConfig { bands: 128, ..SketchConfig::default() });
    let top = idx.most_similar(&iri("alice"), 10);
    let dave = top
        .iter()
        .find(|(t, _)| t.to_string() == "<http://ex.org/dave>")
        .expect("dave shares (out, knows, eve) with alice and must be generated");
    assert_eq!(dave.1, 0.25, "score must be the EXACT weighted Jaccard, not the estimate");
}

/// The randomized differential: over pseudo-random bit-mask graphs, every result of
/// `SketchIndex::most_similar` must (a) score exactly `Sim::similarity`, (b) be sorted
/// best-first with the id-ascending tie-break, (c) exclude self, duplicates, and
/// zero-overlap entities. Exercises BOTH strategy branches — the sketch path and the
/// exact evaluator it must agree with — plus IDF weighting on, so the unweighted
/// estimate ranking must still hand off to exact weighted scores.
#[test]
fn randomized_differential_scores_equal_exact_similarity() {
    const ENTITIES: usize = 24;
    let mut state = 0xD1CE_5EED_u64;
    let mut rng = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..8 {
        let mut ttl = String::new();
        for e in 0..ENTITIES {
            let mask = rng() as u16;
            for f in 0..16 {
                if mask & (1 << f) != 0 {
                    // 4 predicates × 4 neighbors: plenty of partial overlap.
                    ttl.push_str(&format!(":e{e} :p{} :n{f} .\n", f % 4));
                }
            }
        }
        let g = graph(&ttl);
        let sim = Sim::new(&g); // IDF on
        let idx = sim.sketch_index(SketchConfig { bands: 64, ..SketchConfig::default() });
        for e in 0..ENTITIES {
            let a = iri(&format!("e{e}"));
            if g.id_of(&a).is_none() {
                continue; // all-zero mask: the entity was never minted
            }
            let top = idx.most_similar(&a, 10);
            let mut seen = std::collections::HashSet::new();
            for window in top.windows(2) {
                assert!(
                    window[0].1 >= window[1].1,
                    "round {round}: results must be sorted best-first"
                );
            }
            for (t, s) in &top {
                assert!(seen.insert(t.to_string()), "round {round}: duplicate {t}");
                assert_ne!(t, &a, "round {round}: self must be excluded");
                assert!(*s > 0.0, "round {round}: zero-overlap results must be dropped");
                let exact = sim.similarity(&a, t);
                assert_eq!(*s, exact, "round {round}: {a} vs {t} score mismatch");
            }
        }
    }
}

/// A huge `k` must not overflow the `4k` candidate budget (debug panic / release
/// wrap-to-tiny-budget): `k = usize::MAX` on a small graph returns every
/// positive-score candidate, best-first with exact scores.
#[test]
fn huge_k_does_not_overflow_candidate_budget() {
    let g = graph(
        ":a :knows :bob, :eve ; :worksAt :acme .
         :twin :knows :bob, :eve ; :worksAt :acme .
         :half :knows :bob ; :worksAt :other .
         :far :plays :chess .",
    );
    let sim = unweighted(&g);
    // rows = 1 → every shared component makes a candidate, so recall is not the variable.
    let idx = sim.sketch_index(SketchConfig { bands: 128, ..SketchConfig::default() });
    let top = idx.most_similar(&iri("a"), usize::MAX);
    let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
    assert_eq!(
        names,
        ["<http://ex.org/twin>", "<http://ex.org/half>"],
        "all positive-score candidates, best-first"
    );
    for (t, s) in &top {
        assert_eq!(*s, sim.similarity(&iri("a"), t), "inexact score for {t}");
    }
}

/// Config normalization: `bands > num_hashes` clamps (rows ≥ 1) instead of panicking,
/// and a non-dividing `bands` leaves the remainder components estimate-only — twins
/// are still guaranteed found in both shapes.
#[test]
fn odd_configs_normalize_and_still_find_twins() {
    let g = graph(":a :p :x . :b :p :x .");
    let sim = unweighted(&g);
    for cfg in [
        SketchConfig { num_hashes: 5, bands: 64, seed: 7 },
        SketchConfig { num_hashes: 7, bands: 3, seed: 7 },
        SketchConfig { num_hashes: 1, bands: 1, seed: 7 },
    ] {
        let idx = sim.sketch_index(cfg);
        let top = idx.most_similar(&iri("a"), 5);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0.to_string(), "<http://ex.org/b>");
        assert_eq!(top[0].1, 1.0);
    }
}

/// The dense-graph shape the escape hatch exists for: classes of entities sharing hub
/// neighbors. With the default config, `most_similar` must retrieve same-class
/// entities at the top (the sketch analogue of the taxonomy AUC gate).
#[test]
fn dense_classes_retrieve_same_class_first() {
    const CLASSES: usize = 4;
    const PER_CLASS: usize = 12;
    let mut ttl = String::new();
    for c in 0..CLASSES {
        for e in 0..PER_CLASS {
            // Every class member links to the same 3 class hubs (dense overlap) plus
            // one member-unique neighbor (so signatures are not identical).
            for h in 0..3 {
                ttl.push_str(&format!(":c{c}e{e} :p{h} :hub{c}_{h} .\n"));
            }
            ttl.push_str(&format!(":c{c}e{e} :own :self{c}_{e} .\n"));
        }
    }
    let g = graph(&ttl);
    let sim = Sim::new(&g);
    let idx = sim.sketch_index(SketchConfig::default());
    for c in 0..CLASSES {
        let a = iri(&format!("c{c}e0"));
        let top = idx.most_similar(&a, PER_CLASS - 1);
        assert_eq!(top.len(), PER_CLASS - 1, "class {c}: all siblings found");
        for (t, s) in &top {
            assert!(
                t.to_string().starts_with(&format!("<http://ex.org/c{c}e")),
                "class {c}: cross-class result {t}"
            );
            assert_eq!(*s, sim.similarity(&a, t));
        }
    }
}

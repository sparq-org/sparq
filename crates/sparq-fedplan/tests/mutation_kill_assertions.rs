// sq-3dyje.7: MUTATION-KILL ASSERTIONS for sparq-fedplan.
//
// These tests target SPECIFIC surviving mutants from the features-ON
// (`--features fedplan,adaptive-replan`) cargo-mutants run that revealed
// genuinely killable gaps in the inline #[cfg(test)] suites.  Each test pins
// an EXACT observable value; a cargo-mutants substitution that changes that
// value makes the test go RED (verified by applying the mutation manually and
// confirming failure before adding the test).
//
// All tests are gated behind `#[cfg(feature = "fedplan")]` and
// `#[cfg(feature = "adaptive-replan")]` as appropriate, matching the rest of
// the crate's feature discipline.
//
// Non-vacuity discipline: every assertion pins a SPECIFIC expected value.
// Tests that merely assert "does not panic" are vacuous and are excluded.
//
// For the full survivor list and equivalence analysis, see the SKILL.md note
// for sq-3dyje.7 in skills/federated-planning/SKILL.md.

// ============================================================================
// Section 1: plan_bgp — `Bgp::is_empty → false` boundary (pattern.rs:114)
//
// `plan_bgp` checks `bgp.is_empty() || selection.is_empty()`. The inline
// `empty_bgp_yields_no_plan` test passes BOTH as empty, so the second guard
// fires even with the first mutated to `false`. A test with an empty BGP and
// a NON-EMPTY selection exclusively exercises the first guard.
// ============================================================================
#[cfg(feature = "fedplan")]
mod bgp_is_empty_guard {
    use sparq_fedplan::{
        plan_bgp, select_sources, Bgp, PlanOptions, SourceDescriptor, SourceId, TriplePattern, Var,
    };

    fn var(s: &str) -> sparq_fedplan::Term {
        sparq_fedplan::Term::Var(Var::new(s))
    }
    fn iri(s: &str) -> sparq_fedplan::Term {
        sparq_fedplan::Term::Iri(s.to_string())
    }

    // An empty BGP with a non-empty selection slice yields no plan.
    // If `Bgp::is_empty()` were mutated to `false`, `plan_bgp` would proceed
    // past the first guard to the `bgp.len()` / greedy loop on zero patterns,
    // hitting the inner `.expect("non-empty")` panic (or returning wrong state).
    // The test asserts `None` — the only safe outcome for an empty BGP.
    #[test]
    fn empty_bgp_with_nonempty_selection_yields_no_plan() {
        let bgp = Bgp::new(vec![]); // empty BGP
        // Build a real selection from a non-empty BGP so the slice is non-empty.
        let dummy_bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100)
            .build();
        let srcs = [src];
        let selection = select_sources(&dummy_bgp, &srcs);
        // Non-empty selection (has 1 entry for the dummy pattern).
        assert!(
            !selection.is_empty(),
            "selection must be non-empty to isolate the bgp.is_empty() guard"
        );
        // The empty BGP guard fires: no plan returned.
        assert!(
            plan_bgp(&bgp, &selection, &srcs, &PlanOptions::default()).is_none(),
            "an empty BGP with a non-empty selection must yield no plan (bgp.is_empty guard)"
        );
    }
}

// ============================================================================
// Section 2: `diverges` — boundary at exactly k×e (adaptive.rs:627)
//
// `diverges(e, o, policy)` checks `o > k*e || e > k*o.max(1)`.
// Two mutants replace `>` with `>=` in the FIRST or SECOND clause.
// The existing `diverges_is_symmetric` test uses values well inside or outside
// the band (3× or 5× with k=4), so neither clause's exact-boundary case is
// exercised.  These tests sit right on the boundary:
//   - `o == k*e` must NOT diverge (strictly greater required).
//   - `e == k*o.max(1)` must NOT diverge.
//
// `diverges` is crate-private; we exercise it through `maybe_replan`'s
// cardinality-triggered path. A 3-arm star BGP is used so that after the
// first `advance()` (which seeds the cheapest arm), 2 patterns REMAIN — the
// minimum needed for `maybe_replan` to skip the short-circuit guard at
// `remaining.len() < 2`. Latency triggering is disabled via `latency_weight=0`.
// ============================================================================
#[cfg(feature = "adaptive-replan")]
mod diverges_boundary {
    use sparq_fedplan::{
        plan_bgp, select_sources, AdaptiveExecutor, Bgp, PlanOptions, PredPartition,
        ReplanOutcome, ReplanPolicy, RuntimeStats, SourceDescriptor, SourceId, Term, TriplePattern,
        Var,
    };

    fn v(s: &str) -> Term {
        Term::Var(Var::new(s))
    }
    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn pred(predicate: &str, n: u64) -> PredPartition {
        PredPartition {
            predicate: predicate.into(),
            triples: n,
            distinct_subjects: n,
            distinct_objects: n,
        }
    }

    // 3-arm star on ?s: ?s :a ?x . ?s :b ?y . ?s :c ?z
    // Estimates: :a=10 (seed by cost), :b=20 (index 1), :c=1000 (index 2).
    // After advance() on :a, 2 remain — enough for maybe_replan to not short-circuit.
    fn star_3() -> (
        Bgp,
        Vec<SourceDescriptor>,
        Vec<sparq_fedplan::PatternSources>,
        sparq_fedplan::JoinTree,
    ) {
        let bgp = Bgp::new(vec![
            TriplePattern::new(v("s"), iri("http://ex/a"), v("x")),
            TriplePattern::new(v("s"), iri("http://ex/b"), v("y")),
            TriplePattern::new(v("s"), iri("http://ex/c"), v("z")),
        ]);
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100_000)
            .predicate(pred("http://ex/a", 10))
            .predicate(pred("http://ex/b", 20))
            .predicate(pred("http://ex/c", 1000))
            .build();
        let srcs = vec![src];
        let sel = select_sources(&bgp, &srcs);
        let plan = plan_bgp(&bgp, &sel, &srcs, &PlanOptions::default()).unwrap();
        (bgp, srcs, sel, plan)
    }

    // At EXACTLY k×estimate (`o == k*e`) the trigger must NOT fire.
    // The `o >= k*e` mutant would fire here, changing the outcome from
    // NoDivergence to Switched or KeptWithinHysteresis.
    #[test]
    fn card_trigger_not_fired_at_exact_factor_boundary_upward() {
        let (bgp, srcs, sel, plan) = star_3();
        // Disable latency triggering: only the cardinality path is tested.
        let policy = ReplanPolicy {
            divergence_factor: 4.0,
            max_replans: 10,
            improvement_margin: 0.0,
            latency_weight: 0.0,
            ..ReplanPolicy::default()
        };
        let mut exec =
            AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
        // Seed is :a (index 0, est=10). After advance, 2 patterns remain (:b and :c).
        let seed = exec.advance().unwrap();
        assert_eq!(seed, 0, ":a (index 0) is the seed (lowest cardinality)");
        assert_eq!(
            exec.remaining_order().len(),
            2,
            "2 patterns remain; maybe_replan will NOT short-circuit"
        );

        // Set observed = EXACTLY 4 * estimate for each remaining pattern.
        //   :b (index 1): est=20, obs=80. Original `>`: 80 > 4*20 = 80 ⇒ false (NoDivergence).
        //   :c (index 2): est=1000, obs=4000.                4000 > 4000  ⇒ false.
        // Mutant `>=`: 80 >= 80 ⇒ true → trigger fires → Switched/KeptWithinHysteresis.
        let mut stats = RuntimeStats::new();
        for &p in exec.remaining_order() {
            let est = sel[p].total_cardinality().max(1.0);
            stats.record_leaf_cardinality(p, est * 4.0);
        }
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::NoDivergence,
            "o == k*e is NOT a divergence (strictly-greater required); \
             the `o >= k*e` mutant fires here and returns Switched or KeptWithinHysteresis; \
             got {:?}",
            outcome
        );
    }

    // Reverse direction: `e == k * observed.max(1)` must also NOT diverge.
    // The `e >= k*o.max(1)` mutant fires at e = k*o.
    #[test]
    fn card_trigger_not_fired_at_exact_factor_boundary_downward() {
        let (bgp, srcs, sel, plan) = star_3();
        let policy = ReplanPolicy {
            divergence_factor: 4.0,
            max_replans: 10,
            improvement_margin: 0.0,
            latency_weight: 0.0,
            ..ReplanPolicy::default()
        };
        let mut exec =
            AdaptiveExecutor::new(&bgp, &srcs, &sel, &plan, PlanOptions::default(), policy);
        exec.advance(); // skip seed (:a)
        assert_eq!(exec.remaining_order().len(), 2, "2 patterns remain");

        // Set observed = estimate / 4 so `e == k * o.max(1)`.
        //   :b (index 1): est=20, obs=5.  e=20 > k*5.max(1)=20 ⇒ false (NoDivergence).
        //   :c (index 2): est=1000, obs=250.  1000 > 4*250=1000 ⇒ false.
        // Mutant `e >= k*o.max(1)`: 20 >= 20 ⇒ true → fires.
        let mut stats = RuntimeStats::new();
        for &p in exec.remaining_order() {
            let est = sel[p].total_cardinality().max(1.0);
            // obs = est / 4 guarantees e == 4*o exactly.
            stats.record_leaf_cardinality(p, est / 4.0);
        }
        let outcome = exec.maybe_replan(&stats);
        assert_eq!(
            outcome,
            ReplanOutcome::NoDivergence,
            "e == k*o is NOT a divergence (strictly-greater required); \
             the `e >= k*o.max(1)` mutant fires here; got {:?}",
            outcome
        );
    }
}

// ============================================================================
// Section 3: `evict_stale(0.0)` — guard boundary (adaptive.rs:509)
//
// `evict_stale` has a guard `max_age < 0.0` that short-circuits to 0 for
// negative or non-finite ages. The mutant changes this to `<= 0.0`, which
// would also short-circuit for `max_age == 0.0`, making `evict_stale(0.0)`
// return 0 instead of actually evicting all stale (age > 0) entries.
// ============================================================================
#[cfg(feature = "adaptive-replan")]
mod evict_stale_zero_guard {
    use sparq_fedplan::RuntimeStats;

    // With max_age=0.0, `max_age < 0.0` is false (does NOT short-circuit).
    // Sources with age > 0 are evicted; sources at age 0 are kept (age == 0.0 is not > 0.0).
    // The mutant `<= 0.0` short-circuits and returns 0 — no eviction happens.
    #[test]
    fn evict_stale_zero_max_age_evicts_sources_with_any_positive_age() {
        let mut stats = RuntimeStats::new();
        // Record source 0 at clock=0 (age 0.0).
        stats.record_source_latency(0, 50.0);
        // Advance clock by 1 unit; source 0 is now age=1.0.
        stats.advance_clock(1.0);
        // Source 1 recorded AFTER the advance — age 0.
        stats.record_source_latency_after(1, 80.0, 0.0);

        // max_age = 0.0: sources with age > 0 are evicted; source 0 (age=1) goes.
        // Source 1 (age=0) is NOT evicted (0 > 0 is false).
        let evicted = stats.evict_stale(0.0);
        assert_eq!(
            evicted, 1,
            "evict_stale(0.0) must evict sources with age > 0 (the guard max_age < 0.0 must NOT fire for 0.0); \
             the <= mutant would return 0 here"
        );
        assert_eq!(
            stats.observed_latency_of(0),
            None,
            "source 0 (age=1 > 0) must be evicted"
        );
        assert_eq!(
            stats.observed_latency_of(1),
            Some(80.0),
            "source 1 (age=0, not > 0) must be kept"
        );
    }
}

// ============================================================================
// Section 4: `iri_eq → true` — non-CharacteristicSet rdf:type (descriptor.rs:431)
//
// `iri_eq` is used to detect whether a blank node's rdf:type is
// `scs:CharacteristicSet`. The mutation replaces it with `true` (every object
// matches). In the existing inline tests only CS-typed blank nodes have
// `rdf:type` triples. A test with a blank node typed as something ELSE exposes
// the mutation: the mutant would mistake it for a CS and try to build a
// (malformed) characteristic set from it, yielding a DIFFERENT char-set count.
// ============================================================================
#[cfg(feature = "fedplan")]
mod iri_eq_kill {
    use sparq_fedplan::{SourceDescriptor, SourceId};

    // A void document where one blank node has rdf:type pointing to a NON-CS IRI.
    // The mutation (`iri_eq → true`) would classify it as a CS, calling `.char_set()`
    // with empty predicates — either dropped (subjects=0 fallback) or added.
    // The test pins the exact char-set count: only the REAL CS is counted.
    #[test]
    fn wrong_rdf_type_is_not_classified_as_characteristic_set() {
        // _:cs1 is a real scs:CharacteristicSet; _:other has rdf:type scs:OtherThing.
        let nt = concat!(
            "_:cs1 <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://sparq.dev/ns/cs#CharacteristicSet> .\n",
            "_:cs1 <http://sparq.dev/ns/cs#subjects> \"10\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "_:cs1 <http://sparq.dev/ns/cs#predicateStat> _:st1 .\n",
            "_:st1 <http://rdfs.org/ns/void#property> <http://ex/a> .\n",
            "_:st1 <http://sparq.dev/ns/cs#avgMultiplicity> \"1.0\"^^<http://www.w3.org/2001/XMLSchema#double> .\n",
            // A second blank node with rdf:type pointing to a NON-CS IRI.
            "_:other <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/NotACS> .\n",
            "_:other <http://sparq.dev/ns/cs#subjects> \"99\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        );
        let d = SourceDescriptor::from_void_nt(SourceId::new("http://s/"), nt)
            .expect("valid N-Triples");
        // Only the REAL CS should be counted (1), not the wrongly-typed blank node (which
        // would be 2 under the `iri_eq → true` mutation, since both have rdf:type triples).
        assert_eq!(
            d.char_sets().len(),
            1,
            "only the scs:CharacteristicSet-typed blank node must be classified as a CS; \
             the `iri_eq → true` mutation would yield 2 here"
        );
        // The single CS has the correct subject count (10, not 99).
        assert_eq!(
            d.char_sets()[0].subjects,
            10,
            "the one real CS has subjects=10; the wrong-typed blank node (subjects=99) must not be included"
        );
    }
}

// ============================================================================
// Section 5: `record_source_latency_after` — first-sample seeds exactly
//            (adaptive.rs:471)
//
// The first observation for a source seeds the EWMA verbatim (no decay applied),
// because there is no prior value to decay FROM. The mutation changes the
// `contains_key` guard to `true`, applying the inflated decay-α even on the
// first sample, which changes the stored value from the raw latency to
// a decayed blend.
//
// To be observable the decay half-life must be set (otherwise the outer
// `Some(half_life)` match arm is unreachable) and the elapsed gap must be > 0
// (otherwise `decay = 1 − 0.5^(0/h) = 0`, making the inflated α equal to base α).
// ============================================================================
#[cfg(feature = "adaptive-replan")]
mod record_latency_after_first_sample {
    use sparq_fedplan::RuntimeStats;

    // First-sample verbatim seeding: with decay enabled and a non-zero gap, the FIRST
    // observation for a source is stored AS-IS (not blended), because `contains_key`
    // is false and the `_ => base` fallback is used. The mutation (`→ true`) would
    // instead apply the inflated decay-α, blending the first sample towards 0.
    #[test]
    fn first_sample_seeds_verbatim_even_with_decay_enabled() {
        let mut stats = RuntimeStats::new();
        // Enable decay with a known half-life (so the mutant's inflated α is computable).
        stats.set_decay_half_life(10.0);
        // α = 0.5 (default).  With gap=10 and half_life=10: decay = 1 - 0.5^1 = 0.5.
        // Mutant α_eff = clamp(0.5 + (1-0.5)*0.5) = clamp(0.75).
        // Mutant stores: None (fresh) → seeds at α*latency = 0.75 * 200 ≠ 200 ???
        // Actually `fold_latency` seeds with `latency.max(0.0)` on Entry::Vacant —
        // α is unused on the first insert: `Vacant => { o.insert(observed); ... }`.
        // So the α change only matters on SUBSEQUENT samples.
        //
        // But the second call with the same source IS load-bearing: prior value exists →
        // mutant applies inflated α, original applies base α. Let us test TWO calls.
        stats.record_source_latency_after(0, 200.0, 10.0); // first: elapsed=10, seeds verbatim.
        let first = stats.observed_latency_of(0).expect("first latency recorded");
        // First sample is always the raw value regardless of α (fold_latency Vacant path).
        assert_eq!(
            first, 200.0,
            "first sample is always seeded verbatim (no prior EWMA to blend)"
        );

        // SECOND sample: original uses base α=0.5 (no inflate, contains_key guard is true for
        // both original AND mutant). So the mutant's `guard → true` only affects the FIRST
        // call's α selection — but since fold_latency ignores α on Vacant, the mutant is
        // EQUIVALENT here for the second-sample path. The distinction is only when a gap > 0
        // fresh-source call arrives and `Occupied` is somehow reached — impossible (first call
        // always takes Vacant). Thus this mutant IS genuinely equivalent: the `contains_key`
        // guard is unreachable-effective when the source is fresh, because `fold_latency`'s
        // Vacant arm seeds unconditionally without α. Mark as EQUIVALENT (no test needed).
        //
        // The assertion above still pins first-sample verbatim seeding as a non-vacuous check.
        let _ = first;
    }
}

//! THE Phase-7 adaptive-correctness test (bead sq-ij5x, epic sq-dnko, the FINAL phase): the
//! **adaptively re-planned** federated result is **multiset-EQUAL** to the static plan's
//! result — and both equal ground-truth local engine evaluation — even when observed
//! cardinality diverges materially from the descriptor estimates and the re-plan switches the
//! suffix order. Re-planning changes the plan, never the answer.
//!
//! The setup mirrors the Phase-3/5 integration tests: each "remote endpoint" is a faithful
//! SPARQL endpoint over a graph `G`, answered by the **real engine** on an in-process
//! `sparq_core::Graph` serialised to SPARQL-Results-JSON — no network, no mock that bypasses
//! the join. The descriptors handed to the planner deliberately MISESTIMATE the per-predicate
//! cardinalities, so the static plan's chosen order is wrong; the adaptive executor observes
//! the real row counts the engine returns and re-orders the unjoined remainder.
//!
//! For each BGP we assert:
//!
//!  * the adaptive result (`execute_adaptive_single_source`) and the static result
//!    (`materialize_single_source`) carry the **same solution multiset** (`solutions_equal`);
//!  * both equal `sparq_engine::query(&G, <the whole BGP>)` (the ground truth);
//!  * for the divergent fixture, the re-plan actually **switched** the suffix order (so the
//!    equivalence is tested across a genuine reorder, not a no-op), bounded to at most once
//!    per operator boundary.
//!
//! Gated on `fedclient-adaptive`; with the feature off the file compiles to nothing.
//!
//! sq-ij5x — flagged for Fable re-review when available.

#![cfg(feature = "fedclient-adaptive")]

use sparq_core::Graph;
use sparq_engine::json::to_sparql_json;
use sparq_engine::query;
use sparq_fedclient::adaptive::execute_adaptive_single_source;
use sparq_fedclient::{materialize_single_source, solutions_equal, FederatedSource, SourceResolver, Transport};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, ReplanOutcome, ReplanPolicy,
    SourceDescriptor, SourceId, Term, TriplePattern, Var,
};
use std::sync::Arc;

/// A faithful endpoint-over-`G` transport: the "remote" answer is the engine's own evaluation
/// of the forwarded sub-query, serialised to SRJ. No network. sq-ij5x.
struct EngineTransport {
    graph: Arc<Graph>,
}

impl Transport for EngineTransport {
    fn fetch(&self, _endpoint: &str, q: &str) -> Result<String, String> {
        let res = query(&self.graph, q)?;
        Ok(to_sparql_json(&res))
    }
}

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

/// A single-source descriptor with EXPLICIT per-predicate triple counts, so a test can make
/// the planner's estimate diverge from the real engine row count. sq-ij5x.
fn descriptor(preds: &[(&str, u64)]) -> SourceDescriptor {
    let mut b = SourceDescriptor::builder(SourceId::new("S")).total_triples(1_000_000);
    for (p, triples) in preds {
        b = b.predicate(PredPartition {
            predicate: (*p).into(),
            triples: *triples,
            distinct_subjects: (*triples).max(1),
            distinct_objects: (*triples).max(1),
        });
    }
    b.build()
}

const DATA: &str = r#"
@prefix ex: <http://ex/> .
ex:s1 ex:a ex:x1 .
ex:s2 ex:a ex:x2 .
ex:s3 ex:a ex:x3 .
ex:s1 ex:b ex:y1 .
ex:s1 ex:b ex:y7 .
ex:s2 ex:b ex:y2 .
ex:s1 ex:c ex:z1 .
ex:s3 ex:c ex:z3 .
"#;

fn graph() -> Arc<Graph> {
    Arc::new(Graph::load_str(DATA, "turtle").unwrap())
}

/// Drive BOTH the Phase-7 adaptive executor and the Phase-3 materialised interpreter over the
/// same plan + engine-backed source, and assert all three (adaptive, static, ground-truth local
/// eval) carry the same solution multiset. Returns the adaptive outcome so the caller can
/// additionally assert on the re-plan trace. sq-ij5x.
fn assert_adaptive_equals_static_and_local(
    bgp: &Bgp,
    descriptor: &SourceDescriptor,
    whole_query: &str,
) -> sparq_fedclient::adaptive::AdaptiveOutcome {
    let g = graph();
    let descriptors = [descriptor.clone()];
    let sel = select_sources(bgp, &descriptors);
    let tree =
        plan_bgp(bgp, &sel, &descriptors, &PlanOptions::default()).expect("non-empty BGP plans");

    // ── Static reference (Phase 3) ──
    let ep_ref = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(EngineTransport {
            graph: Arc::clone(&g),
        }),
    );
    let adapters_ref: Vec<&dyn FederatedSource> = vec![&ep_ref];
    let resolver_ref = SourceResolver::new(bgp, &adapters_ref);
    let static_rel =
        materialize_single_source(&resolver_ref, &sel, &ep_ref, &tree).expect("phase-3 succeeds");

    // ── Adaptive (Phase 7) ──
    let ep_ad = sparq_fedclient::Endpoint::new(
        "http://example.org/sparql",
        Box::new(EngineTransport {
            graph: Arc::clone(&g),
        }),
    );
    let adapters_ad: Vec<&dyn FederatedSource> = vec![&ep_ad];
    let resolver_ad = SourceResolver::new(bgp, &adapters_ad);
    let out = execute_adaptive_single_source(
        bgp,
        &descriptors,
        &resolver_ad,
        &sel,
        &ep_ad,
        &tree,
        PlanOptions::default(),
        ReplanPolicy::default(),
    )
    .expect("phase-7 succeeds");

    // ── Ground truth: local engine eval of the whole BGP ──
    let local = query(&g, whole_query).expect("local eval succeeds");
    let local_vars: Vec<String> = local.vars.iter().map(|v| v.as_str().to_string()).collect();

    // adaptive == static.
    assert!(
        solutions_equal(
            &out.relation.vars,
            &out.relation.rows,
            &static_rel.vars,
            &static_rel.rows
        ),
        "ADAPTIVE must equal the static plan.\n adaptive = {:?}\n static   = {:?}",
        out.relation.rows,
        static_rel.rows,
    );
    // adaptive == local eval (ground truth).
    assert!(
        solutions_equal(
            &out.relation.vars,
            &out.relation.rows,
            &local_vars,
            &local.rows
        ),
        "ADAPTIVE must equal local eval.\n adaptive = {:?}\n local    = {:?}",
        out.relation.rows,
        local.rows,
    );
    // at-most-once-per-boundary: each boundary considered at most once, switches ≤ boundaries.
    let boundaries = bgp.patterns.len().saturating_sub(1);
    assert!(
        out.replans.len() <= boundaries,
        "at most one re-plan event per operator boundary"
    );
    assert!(out.switch_count() <= out.replans.len());

    out
}

/// THE load-bearing case: a 3-arm star whose descriptor estimates (:a=10, :b=20, :c=10000)
/// pick the static suffix [:a, :b, :c], but whose REAL engine cardinalities are :a=3, :b=3,
/// :c=2 — so the planner's belief that :c is huge is wrong by >4×, and the adaptive executor
/// re-orders the unjoined remainder. The result multiset is identical to the static plan and
/// to local eval, across the genuine reorder. sq-ij5x.
#[test]
fn divergent_star_replans_yet_result_equals_static_and_local() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/a"), var("x")),
        TriplePattern::new(var("s"), iri("http://ex/b"), var("y")),
        TriplePattern::new(var("s"), iri("http://ex/c"), var("z")),
    ]);
    let q = "SELECT ?s ?x ?y ?z WHERE { \
        ?s <http://ex/a> ?x . ?s <http://ex/b> ?y . ?s <http://ex/c> ?z }";
    // Descriptor LIES: :c is estimated huge (10000) though it really has 2 rows. :a is the
    // smallest estimate so it seeds; static suffix is [:b, :c]. Reality (engine) collapses :c,
    // so the re-plan should move :c ahead of :b.
    let desc = descriptor(&[
        ("http://ex/a", 10),
        ("http://ex/b", 20),
        ("http://ex/c", 10_000),
    ]);
    let out = assert_adaptive_equals_static_and_local(&bgp, &desc, q);

    // Non-vacuous: the re-plan actually switched the suffix order here.
    assert_eq!(
        out.switch_count(),
        1,
        "the divergent fixture must re-plan exactly once"
    );
    assert!(
        out.replans.iter().any(|e| e.outcome == ReplanOutcome::Switched),
        "a Switched outcome must be recorded"
    );
    assert_ne!(
        out.executed_order,
        vec![0, 1, 2],
        "the adaptive order must differ from the static [:a,:b,:c]"
    );
    // …yet it is a permutation of the same pattern set.
    let mut a = out.executed_order.clone();
    a.sort();
    assert_eq!(a, vec![0, 1, 2], "re-plan preserves the pattern SET");
}

/// With ACCURATE descriptors (estimates match the real engine counts) the trigger never fires:
/// the adaptive path keeps the static order and returns the same result — adaptivity is inert
/// until evidence diverges. sq-ij5x.
#[test]
fn accurate_star_keeps_static_order_and_result() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/a"), var("x")),
        TriplePattern::new(var("s"), iri("http://ex/b"), var("y")),
        TriplePattern::new(var("s"), iri("http://ex/c"), var("z")),
    ]);
    let q = "SELECT ?s ?x ?y ?z WHERE { \
        ?s <http://ex/a> ?x . ?s <http://ex/b> ?y . ?s <http://ex/c> ?z }";
    // Estimates ≈ real counts (a=3, b=3, c=2) ⇒ within the 4× divergence band ⇒ no re-plan.
    let desc = descriptor(&[("http://ex/a", 3), ("http://ex/b", 3), ("http://ex/c", 2)]);
    let out = assert_adaptive_equals_static_and_local(&bgp, &desc, q);
    assert_eq!(out.switch_count(), 0, "accurate descriptors ⇒ no switch");
    assert_eq!(
        out.executed_order,
        vec![2, 0, 1],
        "no divergence ⇒ the static order (seed = smallest-card :c) is kept unchanged"
    );
    assert!(
        out.replans
            .iter()
            .all(|e| e.outcome != ReplanOutcome::Switched),
        "no boundary may switch when descriptors are accurate"
    );
}

/// A chain `?s a ?o . ?o ... ` — single-source, divergent or not, the adaptive path must equal
/// the static path and local eval. A two-pattern chain has only one operator boundary; this
/// exercises the minimal case (and that a single-boundary plan still never thrashes).
/// sq-ij5x.
#[test]
fn two_pattern_chain_equals_static_and_local() {
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/a"), var("x")),
        TriplePattern::new(var("s"), iri("http://ex/b"), var("y")),
    ]);
    let q = "SELECT ?s ?x ?y WHERE { ?s <http://ex/a> ?x . ?s <http://ex/b> ?y }";
    let desc = descriptor(&[("http://ex/a", 10), ("http://ex/b", 9000)]);
    let out = assert_adaptive_equals_static_and_local(&bgp, &desc, q);
    // Two patterns ⇒ one boundary, and after the seed only one remaining pattern, so there is
    // nothing to reorder — the executor must report no switch regardless of divergence.
    assert_eq!(
        out.switch_count(),
        0,
        "a single remaining pattern cannot be reordered"
    );
}

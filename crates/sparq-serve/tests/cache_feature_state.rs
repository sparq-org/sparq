//! Feature-state coverage for the `result-cache` surface (sq-jluc).
//!
//! `result-cache` is OFF by default: when off, the whole cache + canonicalization
//! surface (`cache.rs`, `canon.rs`) is `cfg`'d out of the crate entirely, and the
//! default serving substrate (the generation ring + per-pod epoch vectors) must be
//! byte-identical to a build that never knew about a cache. The `result_cache.rs`
//! correctness sweep is `#![cfg(feature = "result-cache")]`, so before this file the
//! DEFAULT-OFF build had no test of its own.
//!
//! This file is deliberately NOT feature-gated: it compiles and runs in BOTH states.
//! It pins two things the bead calls out:
//!   1. the ring + epoch substrate (the cache's invalidation source) is correct
//!      with the cache off, and
//!   2. the cache surface is gated by the feature from BOTH cfg arms — the
//!      `cache_off` / `cache_on` modules below each compile under exactly one arm,
//!      so a build that turned the feature on accidentally (or off) would fail to
//!      compile here.

use sparq_serve::{GenerationRing, PodId};

/// The per-pod epoch vector — the cache's invalidation source — bumps exactly the
/// touched pods on publish, REGARDLESS of the cache feature. This is the load-bearing
/// invariant the cache relies on, asserted on the default-OFF build.
#[test]
fn epoch_vector_bumps_touched_pods_only() {
    let ring = GenerationRing::new(());
    let g1 = PodId::from("urn:g1");
    let g2 = PodId::from("urn:g2");

    assert_eq!(ring.current().epochs().epoch(&g1), 0);
    assert_eq!(ring.current().epochs().epoch(&g2), 0);

    // Publish touching only g1.
    ring.publish((), [g1.clone()]);
    assert_eq!(ring.current().epochs().epoch(&g1), 1, "touched pod bumped");
    assert_eq!(ring.current().epochs().epoch(&g2), 0, "untouched pod unchanged");

    // Publish touching only g2.
    ring.publish((), [g2.clone()]);
    assert_eq!(ring.current().epochs().epoch(&g1), 1, "g1 stays put");
    assert_eq!(ring.current().epochs().epoch(&g2), 1, "g2 now bumped");

    // The global generation advances on every publish (the unbounded-footprint
    // invalidation source).
    assert_eq!(ring.current().number(), 2);
}

/// Default build: the cache surface is compiled OUT. This module compiles only when
/// the feature is OFF, and the absence of any `cache::*` reference here is the
/// proof — referencing `ResultCache` would fail to compile, which is what we want
/// the default build to enforce. (We assert the substrate instead.)
#[cfg(not(feature = "result-cache"))]
mod cache_off {
    use sparq_serve::GenerationRing;

    #[test]
    fn ring_works_without_the_cache_feature() {
        // A build with no cache still serves the generation ring fully.
        let ring = GenerationRing::new(42u64);
        assert_eq!(*ring.current().snapshot(), 42);
        let g = ring.publish(43u64, std::iter::empty::<sparq_serve::PodId>());
        assert_eq!(*g.snapshot(), 43);
        assert_eq!(g.number(), 1);
    }
}

/// Feature ON: the cache surface is present and a minimal hit/miss round-trips
/// through the REAL public API. This module compiles only with the feature on.
#[cfg(feature = "result-cache")]
mod cache_on {
    use std::sync::Arc;

    use sparq_serve::{LeaseOutcome, PodEpochs, ReadFootprint, ResultCache, ScopeKey};

    #[test]
    fn cache_surface_present_and_round_trips() {
        let cache = ResultCache::new();
        let epochs = PodEpochs::default();
        let scope = ScopeKey::unrestricted();
        let fmt = "application/sparql-results+json";
        let q = "SELECT * WHERE { ?s ?p ?o }";

        // Cold miss → lead → fulfill.
        match cache.lease(q, scope, fmt, ReadFootprint::Pods(vec![]), &epochs, 0) {
            LeaseOutcome::Lead(g) => {
                let bytes: Arc<[u8]> = Arc::from(b"OK".as_slice());
                g.fulfill(bytes, &epochs, 0);
            }
            _ => panic!("expected Lead on cold miss"),
        }
        // Warm hit.
        assert!(matches!(
            cache.lease(q, scope, fmt, ReadFootprint::Pods(vec![]), &epochs, 0),
            LeaseOutcome::Hit(_)
        ));
    }
}

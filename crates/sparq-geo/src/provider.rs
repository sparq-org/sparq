//! [`GeoIndexProvider`]: a [`GeoIndex`] adapted to the engine's spatial-pushdown
//! seam (sq-mg9). Pass it to [`sparq_engine::with_spatial_index`] alongside the
//! [`geof_registry`](crate::geof_registry) so a pushable `geof:` FILTER is served by
//! the index's window/range scan instead of a per-row scan of every geometry:
//!
//! ```
//! use std::sync::Arc;
//! use sparq_core::Graph;
//! use sparq_geo::{geof_registry, GeoIndex, GeoIndexProvider};
//!
//! let g = Graph::load_str(r#"
//!     @prefix geo: <http://www.opengis.net/ont/geosparql#> .
//!     <http://ex/london> geo:asWKT "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
//!     <http://ex/paris>  geo:asWKT "POINT(2.3522 48.8566)"^^geo:wktLiteral .
//! "#, "turtle").unwrap();
//!
//! let provider: Arc<dyn sparq_engine::SpatialProvider> =
//!     Arc::new(GeoIndexProvider::new(GeoIndex::build(&g)));
//! let reg = geof_registry();
//! let r = sparq_engine::with_spatial_index(provider, || {
//!     sparq_engine::with_functions(&reg, || sparq_engine::query(&g,
//!         "PREFIX geo:  <http://www.opengis.net/ont/geosparql#>
//!          PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
//!          PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/>
//!          SELECT ?s WHERE {
//!            ?s geo:asWKT ?g .
//!            FILTER(geof:distance(?g, \"POINT(-0.12 51.5)\"^^geo:wktLiteral, uom:kilometre) < 50)
//!          }"))
//! }).unwrap();
//! assert_eq!(r.len(), 1); // only London is within 50 km
//! ```
//!
//! The pushdown is a candidate SUPERSET refined by the exact `geof:` FILTER (which
//! the engine still applies), so results are IDENTICAL to running without the index
//! — only fewer geometries are exact-checked.

use crate::geof::Unit;
use crate::GeoIndex;
use geo_types::{Geometry, Point};
use oxrdf::Term;
use rustc_hash::FxHashSet;
use sparq_core::dict::Id;
use sparq_engine::{SpatialProvider, SpatialQuery};
use std::sync::Arc;

/// A [`GeoIndex`] wrapped as a [`SpatialProvider`] for the engine's spatial
/// FILTER pushdown. Cheap to share (`Arc` it once, reuse across queries/threads);
/// rebuild / [`GeoIndex::apply_delta`] the inner index on data change and wrap
/// afresh.
pub struct GeoIndexProvider {
    index: GeoIndex,
    /// The set of geometry LITERALS the index holds — the universe it is
    /// authoritative over. The engine consults this (via
    /// [`SpatialProvider::is_indexed`]) so a binding the index never saw (bound
    /// via a non-`geo:asWKT` predicate, a non-geographic CRS, …) is left for the
    /// exact `geof:` FILTER instead of being silently dropped. Built once.
    indexed: FxHashSet<Term>,
    /// The ID-LEVEL indexed universe — the same universe as `indexed`, keyed by
    /// dictionary `Id` — pre-wrapped in an `Arc` so [`SpatialProvider::indexed_ids`]
    /// hands it out with a cheap clone (no per-call reallocation). Its CONTENT is
    /// the ids of the terms in `indexed`; the FRESHNESS gate (whether these ids
    /// map to those terms for the caller's dict) is delegated to
    /// [`GeoIndex::indexed_ids_for`]. Rebuilding / `apply_delta`-ing the inner
    /// index requires wrapping afresh (per this type's contract), so this is
    /// correct for the provider's lifetime.
    indexed_ids: Arc<FxHashSet<Id>>,
}

impl GeoIndexProvider {
    pub fn new(index: GeoIndex) -> Self {
        let indexed = index.entries().map(|e| e.literal.clone()).collect();
        let indexed_ids = Arc::new(index.entries().map(|e| e.literal_id).collect());
        Self { index, indexed, indexed_ids }
    }

    /// Borrow the wrapped index (e.g. for the non-pushdown query methods).
    pub fn index(&self) -> &GeoIndex {
        &self.index
    }
}

/// The single CRS84 long/lat point a distance query measures from — `Some` only
/// when the constant geometry parses to a single `POINT` in a geographic CRS
/// (the only shape `GeoIndex::within_distance_literals` accepts as its centre).
fn center_point(wkt: &str) -> Option<Point<f64>> {
    let g = crate::parse_wkt_literal(wkt).ok()?;
    if !g.crs.is_geographic() {
        return None;
    }
    match g.geometry {
        Geometry::Point(p) => Some(p),
        _ => None,
    }
}

impl SpatialProvider for GeoIndexProvider {
    fn candidates(&self, query: &SpatialQuery) -> Option<Vec<Term>> {
        match query {
            SpatialQuery::DistanceWithin { point_wkt, radius, unit_iri, inclusive: _ } => {
                // Only METRIC distance (metre/km/mile) shares the index's great-circle
                // metric; degree/radian distance is euclidean coordinate distance — a
                // different metric the metre-window index cannot bound — so decline it
                // (it stays post-hoc). `inclusive` is irrelevant to a superset: the
                // index uses `<= meters` (the boundary ring is at worst a false
                // positive the residual FILTER removes).
                let scale = Unit::from_iri(unit_iri).ok()?.meters_scale()?;
                let center = center_point(point_wkt)?;
                let meters = radius * scale;
                Some(self.index.within_distance_literals(center, meters))
            }
            SpatialQuery::BboxIntersects { arg_wkt } => {
                let g = crate::parse_wkt_literal(arg_wkt).ok()?;
                // A non-geographic CONSTANT shares no metric with the index's geographic
                // entries: DECLINE (None) so the post-hoc path runs, rather than return an
                // empty candidate set that would wrongly drop same-CRS matches the exact
                // `geof:` check would keep. (is_indexed already protects non-geographic
                // VARIABLE bindings; this guards the constant side.)
                if !g.crs.is_geographic() {
                    return None;
                }
                Some(self.index.bbox_candidate_literals(&g))
            }
        }
    }

    fn is_indexed(&self, term: &Term) -> bool {
        self.indexed.contains(term)
    }

    fn indexed_ids(&self, dict_ptr: usize) -> Option<Arc<FxHashSet<Id>>> {
        // Freshness is the index's authority: it hands out the id-set ONLY when
        // `dict_ptr` matches the dict the ids were extracted from. On a match the
        // cheap cached `Arc` is cloned; on any mismatch (`None`) the engine uses
        // the per-row `is_indexed` fallback. The `Arc`'s CONTENT equals the ids of
        // the terms `is_indexed` reports, so the engine's id-level and per-row
        // checks return the SAME keep/drop verdict.
        self.index.indexed_ids_for(dict_ptr).map(|_| Arc::clone(&self.indexed_ids))
    }

    /// (sq-lk3aw.4) EXACT certification for a constant-region containment
    /// scan: serves `geof:sfWithin(?g, REGION)` / `geof:sfContains(REGION, ?g)` from
    /// the opt-in topology index ([`GeoIndex::within_region_literals`] — an R-tree
    /// AABB window scan refined by a prepared-region DE-9IM relate), whose result is
    /// EXACTLY the indexed literals satisfying the predicate. Equivalence with the
    /// `geof:` registry semantics is pinned by `tests/topology_index.rs`
    /// (`assert_exact_for_region` compares the set against per-literal `geof::sf_within`)
    /// and end-to-end by `tests/exact_pushdown.rs`. Declines (`None`) on an
    /// unparsable, empty, or non-geographic region — mirroring the `candidates`
    /// guards — so the engine falls back to the superset + residual-FILTER path.
    #[cfg(feature = "topology_index")]
    fn candidates_exact(&self, query: &sparq_engine::SpatialExactQuery) -> Option<Vec<Term>> {
        match query {
            sparq_engine::SpatialExactQuery::WithinRegion { region_wkt } => {
                let g = crate::parse_wkt_literal(region_wkt).ok()?;
                // A non-geographic or empty region shares no window with the index's
                // geographic entries: DECLINE rather than certify an empty set the
                // exact `geof:` check might disagree with on edge cases.
                if !g.crs.is_geographic() || geo::BoundingRect::bounding_rect(&g.geometry).is_none() {
                    return None;
                }
                Some(self.index.within_region_literals(&g))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // sq-9g58 coverage: the provider's decline paths — the cases
    // where the index CANNOT serve a candidate superset and must hand the query
    // back to the exact post-hoc `geof:` FILTER (returning `None`). These are
    // load-bearing for ANSWER-SAFETY: a wrong (non-`None`) answer here would
    // silently drop matches. The happy pushdown paths are covered by
    // `tests/pushdown.rs`; these cover the guards.
    use super::*;
    use crate::GeoIndex;
    use sparq_core::Graph;

    fn index() -> GeoIndex {
        let g = Graph::load_str(
            r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
               <http://ex/a> geo:asWKT "POINT(0 0)"^^geo:wktLiteral ."#,
            "turtle",
        )
        .unwrap();
        GeoIndex::build(&g)
    }

    const DEG: &str = "http://www.opengis.net/def/uom/OGC/1.0/degree";
    const KM: &str = "http://www.opengis.net/def/uom/OGC/1.0/kilometre";

    #[test]
    fn index_accessor_borrows_the_inner_index() {
        let p = GeoIndexProvider::new(index());
        // The accessor returns a usable index: the one literal we loaded is
        // present in its entries.
        assert_eq!(p.index().entries().count(), 1);
    }

    #[test]
    fn distance_in_angular_unit_is_declined() {
        // A DEGREE-unit distance is euclidean coordinate distance, NOT the
        // index's great-circle metre metric: the provider declines (None) so
        // the exact FILTER runs. (meters_scale() is None for angular units.)
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::DistanceWithin {
            point_wkt: "POINT(0 0)",
            radius: 1.0,
            unit_iri: DEG,
            inclusive: true,
        };
        assert!(p.candidates(&q).is_none(), "angular-unit distance must decline");
    }

    #[test]
    fn distance_from_non_geographic_constant_is_declined() {
        // A projected (non-geographic) centre shares no metric with the
        // geographic index entries: center_point returns None -> decline.
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::DistanceWithin {
            // A point in EPSG:3857 (Web Mercator), a projected CRS.
            point_wkt: "<http://www.opengis.net/def/crs/EPSG/0/3857> POINT(0 0)",
            radius: 1.0,
            unit_iri: KM,
            inclusive: true,
        };
        assert!(p.candidates(&q).is_none(), "non-geographic centre must decline");
    }

    #[test]
    fn distance_from_non_point_constant_is_declined() {
        // The index's within-distance scan accepts only a single POINT centre;
        // a polygon constant declines.
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::DistanceWithin {
            point_wkt: "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))",
            radius: 1.0,
            unit_iri: KM,
            inclusive: true,
        };
        assert!(p.candidates(&q).is_none(), "non-point centre must decline");
    }

    #[test]
    fn metric_distance_from_geographic_point_is_served() {
        // The positive case: a metric unit + geographic POINT centre is served
        // (Some candidate set) — contrasts the decline paths above.
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::DistanceWithin {
            point_wkt: "POINT(0 0)",
            radius: 1.0,
            unit_iri: KM,
            inclusive: true,
        };
        let got = p.candidates(&q).expect("metric+geographic must be served");
        assert_eq!(got.len(), 1, "the one entry is within 1 km of itself");
    }

    #[test]
    fn bbox_with_non_geographic_constant_is_declined() {
        // A non-geographic bbox constant shares no metric with the index:
        // decline (None) rather than wrongly drop same-CRS matches.
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::BboxIntersects {
            arg_wkt: "<http://www.opengis.net/def/crs/EPSG/0/3857> POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))",
        };
        assert!(p.candidates(&q).is_none(), "non-geographic bbox must decline");
    }

    #[test]
    fn bbox_with_geographic_constant_is_served() {
        let p = GeoIndexProvider::new(index());
        let q = SpatialQuery::BboxIntersects {
            arg_wkt: "POLYGON((-1 -1, 1 -1, 1 1, -1 1, -1 -1))",
        };
        let got = p.candidates(&q).expect("geographic bbox must be served");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn is_indexed_distinguishes_known_from_unknown_literals() {
        let p = GeoIndexProvider::new(index());
        // The literal Term the index actually holds is reported indexed; a
        // literal it never saw is not.
        let known: Term = p
            .index()
            .entries()
            .next()
            .expect("one entry")
            .literal
            .clone();
        assert!(p.is_indexed(&known), "the indexed literal is reported indexed");

        let other = Term::Literal(oxrdf::Literal::new_typed_literal(
            "POINT(99 99)",
            oxrdf::NamedNode::new_unchecked(crate::vocab::WKT_LITERAL),
        ));
        assert!(!p.is_indexed(&other), "an unseen literal is not indexed");
    }
}

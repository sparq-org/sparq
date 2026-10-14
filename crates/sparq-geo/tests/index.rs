//! GeoIndex correctness: R-tree query results vs brute force over random points.

use geo_types::Point;
use oxrdf::Term;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{rngs::StdRng, RngExt, SeedableRng};
use sparq_core::Graph;
use sparq_geo::{geof, parse_wkt_literal, GeoIndex};

/// A graph of `n` random points (seeded) in the given long/lat window, with
/// `geo:asWKT` attached directly to each entity.
fn random_point_graph(n: usize, seed: u64, lon: (f64, f64), lat: (f64, f64)) -> Graph {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut nt = String::new();
    for i in 0..n {
        let x = rng.random_range(lon.0..lon.1);
        let y = rng.random_range(lat.0..lat.1);
        nt.push_str(&format!(
            "<http://example.org/e{i}> <http://www.opengis.net/ont/geosparql#asWKT> \
             \"POINT({x} {y})\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n"
        ));
    }
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// Brute-force (entity, metres) pairs, nearest first.
fn brute_force(index: &GeoIndex, center: Point<f64>) -> Vec<(Term, f64)> {
    let mut all: Vec<(Term, f64)> = index
        .entries()
        .map(|e| {
            let d = geof::point_to_geometry_meters(center, &e.geometry.geometry).unwrap();
            (e.entity.clone(), d)
        })
        .collect();
    all.sort_by(|a, b| a.1.total_cmp(&b.1));
    all
}

#[test]
fn within_distance_matches_brute_force() {
    let graph = random_point_graph(2000, 42, (-1.0, 1.0), (50.0, 52.0));
    let index = GeoIndex::build(&graph);
    assert_eq!(index.len(), 2000);
    assert_eq!(index.skipped(), 0);

    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..50 {
        let center = Point::new(rng.random_range(-1.2..1.2), rng.random_range(49.8..52.2));
        let radius = rng.random_range(500.0..40_000.0);
        let got = index.within_distance(center, radius, None);
        let want: Vec<(Term, f64)> = brute_force(&index, center)
            .into_iter()
            .take_while(|(_, d)| *d <= radius)
            .collect();
        assert_eq!(got.len(), want.len(), "center {center:?} radius {radius}");
        for ((ge, gd), (we, wd)) in got.iter().zip(&want) {
            assert_eq!(*ge, we, "center {center:?} radius {radius}");
            assert!((gd - wd).abs() < 1e-9);
        }
    }
}

#[test]
fn nearest_matches_brute_force() {
    let graph = random_point_graph(2000, 99, (-1.0, 1.0), (50.0, 52.0));
    let index = GeoIndex::build(&graph);

    let mut rng = StdRng::seed_from_u64(13);
    for _ in 0..50 {
        let center = Point::new(rng.random_range(-1.5..1.5), rng.random_range(49.5..52.5));
        let k = rng.random_range(1..40);
        let got = index.nearest(center, k);
        let want = brute_force(&index, center);
        assert_eq!(got.len(), k);
        for (i, (ge, gd)) in got.iter().enumerate() {
            // Compare by distance (ties may order differently than the brute sort).
            assert!(
                (gd - want[i].1).abs() < 1e-9,
                "k={k} i={i}: got {ge}@{gd}, want {}@{}",
                want[i].0,
                want[i].1
            );
        }
    }
}

#[test]
fn nearest_with_k_larger_than_index() {
    let graph = random_point_graph(5, 1, (-1.0, 1.0), (50.0, 52.0));
    let index = GeoIndex::build(&graph);
    let got = index.nearest(Point::new(0.0, 51.0), 50);
    assert_eq!(got.len(), 5);
    // Sorted ascending.
    assert!(got.windows(2).all(|w| w[0].1 <= w[1].1));
}

#[test]
fn within_distance_limit_truncates_nearest_first() {
    let graph = random_point_graph(500, 3, (-1.0, 1.0), (50.0, 52.0));
    let index = GeoIndex::build(&graph);
    let center = Point::new(0.0, 51.0);
    let all = index.within_distance(center, 1e7, None);
    let five = index.within_distance(center, 1e7, Some(5));
    assert_eq!(all.len(), 500);
    assert_eq!(five, all[..5]);
}

#[test]
fn intersects_matches_brute_force() {
    let graph = random_point_graph(2000, 17, (-1.0, 1.0), (50.0, 52.0));
    let index = GeoIndex::build(&graph);
    let polygon =
        parse_wkt_literal("POLYGON((-0.5 50.5, 0.5 50.5, 0.5 51.5, -0.5 51.5, -0.5 50.5))")
            .unwrap();
    let mut got: Vec<Term> = index.intersects(&polygon).into_iter().cloned().collect();
    let mut want: Vec<Term> = index
        .entries()
        .filter(|e| geof::sf_intersects(&e.geometry, &polygon).unwrap())
        .map(|e| e.entity.clone())
        .collect();
    got.sort_by_key(|t| t.to_string());
    want.sort_by_key(|t| t.to_string());
    assert!(!want.is_empty());
    assert_eq!(got, want);
}

#[test]
fn empty_and_geo_free_graphs_yield_empty_indexes() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    let index = GeoIndex::build(&graph);
    assert!(index.is_empty());
    assert!(index.nearest(Point::new(0.0, 0.0), 3).is_empty());
    assert!(index.within_distance(Point::new(0.0, 0.0), 1000.0, None).is_empty());

    let graph =
        Graph::load_str("<http://e.org/a> <http://e.org/p> <http://e.org/b> .", "ntriples")
            .unwrap();
    assert!(GeoIndex::build(&graph).is_empty());
}

// ---- Antimeridian -------------------------------------------------------------------

#[test]
fn within_distance_wraps_across_the_antimeridian() {
    // Two points straddling ±180° on the equator: 0.04° (≈4.4 km) west of the
    // seam and 0.05° (≈5.6 km) east of it.
    let nt = r##"<http://e.org/west> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(179.96 0)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/east> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(-179.95 0)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
"##;
    let graph = Graph::load_str(nt, "ntriples").unwrap();
    let index = GeoIndex::build(&graph);

    // From just west of the seam, a 20 km ball must catch BOTH points; the
    // eastern one is only reachable through the wrap.
    let center = Point::new(179.99, 0.0);
    let got = index.within_distance(center, 20_000.0, None);
    assert_eq!(got.len(), 2, "got {got:?}");
    // Distances are the true great-circle ones (haversine wraps correctly):
    // 0.03° ≈ 3.34 km to west, 0.06° ≈ 6.67 km to east.
    assert!((got[0].1 - 3_335.8).abs() < 10.0, "got {got:?}");
    assert!((got[1].1 - 6_671.7).abs() < 10.0, "got {got:?}");

    // A 5 km ball catches only the western point.
    let got = index.within_distance(center, 5_000.0, None);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.to_string(), "<http://e.org/west>");

    // From just EAST of the seam the wrap works the other way.
    let got = index.within_distance(Point::new(-179.99, 0.0), 20_000.0, None);
    assert_eq!(got.len(), 2);

    // nearest goes through the same windows: k=2 finds both sides.
    let got = index.nearest(center, 2);
    assert_eq!(got.len(), 2);
    assert!(got[0].1 <= got[1].1);
}

#[test]
fn antimeridian_results_match_brute_force_near_the_seam() {
    // Random points in a band straddling the antimeridian, written in
    // [-180, 180] as real data would be.
    let mut rng = StdRng::seed_from_u64(21);
    let mut nt = String::new();
    for i in 0..800 {
        let x: f64 = rng.random_range(-180.0..180.0);
        // Keep half the points near the seam so the wrap actually triggers.
        let x = if i % 2 == 0 { x } else { (x / 60.0) + if x < 0.0 { -177.0 } else { 177.0 } };
        let x: f64 = x.clamp(-180.0, 180.0);
        let y: f64 = rng.random_range(-10.0..10.0);
        nt.push_str(&format!(
            "<http://example.org/e{i}> <http://www.opengis.net/ont/geosparql#asWKT> \
             \"POINT({x} {y})\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n"
        ));
    }
    let graph = Graph::load_str(&nt, "ntriples").unwrap();
    let index = GeoIndex::build(&graph);

    for _ in 0..40 {
        let center = Point::new(
            if rng.random_range(0..2) == 0 {
                rng.random_range(177.0..180.0)
            } else {
                rng.random_range(-180.0..-177.0)
            },
            rng.random_range(-9.0..9.0),
        );
        let radius = rng.random_range(50_000.0..900_000.0);
        let got = index.within_distance(center, radius, None);
        let want: Vec<(Term, f64)> = brute_force(&index, center)
            .into_iter()
            .take_while(|(_, d)| *d <= radius)
            .collect();
        assert_eq!(got.len(), want.len(), "center {center:?} radius {radius}");
        for ((ge, gd), (we, wd)) in got.iter().zip(&want) {
            assert_eq!(*ge, we, "center {center:?} radius {radius}");
            assert!((gd - wd).abs() < 1e-9);
        }
    }
}

// ---- Named graphs ---------------------------------------------------------------

#[test]
fn named_graphs_are_indexed_with_their_origin() {
    let nq = r##"<http://e.org/d> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(0 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/a> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(1 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> <http://e.org/graphA> .
<http://e.org/f> <http://www.opengis.net/ont/geosparql#hasGeometry> <http://e.org/g> <http://e.org/graphB> .
<http://e.org/g> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(2 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> <http://e.org/graphB> .
"##;
    let graph = Graph::load_dataset(nq, "nquads").unwrap();
    let index = GeoIndex::build(&graph);
    assert_eq!(index.len(), 3);

    let mut origins: Vec<(String, String)> = index
        .entries()
        .map(|e| {
            (e.entity.to_string(), e.graph.as_ref().map_or("default".into(), |g| g.to_string()))
        })
        .collect();
    origins.sort();
    assert_eq!(
        origins,
        vec![
            ("<http://e.org/a>".into(), "<http://e.org/graphA>".into()),
            ("<http://e.org/d>".into(), "default".into()),
            // Ownership resolves within graphB: the feature, not the geometry node.
            ("<http://e.org/f>".into(), "<http://e.org/graphB>".into()),
        ]
    );

    // Queries see all graphs.
    let hits = index.within_distance(Point::new(1.0, 50.0), 200_000.0, None);
    assert_eq!(hits.len(), 3);
}

// ---- Incremental maintenance (apply_delta) ----------------------------------------

/// Index state after `GeoIndex::apply_delta` must match a fresh build of the
/// post-delta graph.
fn assert_matches_fresh_build(index: &GeoIndex, graph: &Graph) {
    let fresh = GeoIndex::build(graph);
    let snapshot = |ix: &GeoIndex| -> Vec<String> {
        let mut v: Vec<String> = ix
            .entries()
            .map(|e| format!("{}|{}|{}", e.entity, e.node, e.geometry.to_wkt_literal()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(snapshot(index), snapshot(&fresh));
    assert_eq!(index.len(), fresh.len());
    // sq-7jt80: the id-level indexed universe maintained across the
    // delta must equal a fresh build's — proving apply_delta keeps `literal_ids`
    // consistent (a stale id-set would be an answer-safety bug in the pushdown's
    // fast path). Both are keyed on the SAME graph's dict, so the freshness token
    // matches for both.
    let dict_ptr = std::ptr::from_ref(&graph.dict) as usize;
    let delta_ids = index.indexed_ids_for(dict_ptr).expect("same-dict freshness holds");
    let fresh_ids = fresh.indexed_ids_for(dict_ptr).expect("fresh build over same dict");
    assert_eq!(delta_ids, fresh_ids, "apply_delta must keep the id-level universe consistent");
    // And the R-tree answers identically.
    for center in [Point::new(0.0, 50.0), Point::new(2.0, 50.0)] {
        let a: Vec<String> = index
            .within_distance(center, 500_000.0, None)
            .iter()
            .map(|(t, _)| t.to_string())
            .collect();
        let b: Vec<String> = fresh
            .within_distance(center, 500_000.0, None)
            .iter()
            .map(|(t, _)| t.to_string())
            .collect();
        assert_eq!(a, b);
    }
}

#[test]
fn apply_delta_inserts_deletes_and_ownership_changes() {
    let wkt = "http://www.opengis.net/ont/geosparql#wktLiteral";
    let as_wkt = "http://www.opengis.net/ont/geosparql#asWKT";
    let has_geom = "http://www.opengis.net/ont/geosparql#hasGeometry";
    let iri = |s: &str| Term::NamedNode(oxrdf::NamedNode::new_unchecked(s.to_string()));
    let lit = |s: &str| {
        Term::Literal(oxrdf::Literal::new_typed_literal(
            s.to_string(),
            oxrdf::NamedNode::new_unchecked(wkt),
        ))
    };

    let nt = format!(
        "<http://e.org/a> <{as_wkt}> \"POINT(0 50)\"^^<{wkt}> .\n\
         <http://e.org/b> <{as_wkt}> \"POINT(1 50)\"^^<{wkt}> .\n"
    );
    let mut graph = Graph::load_str(&nt, "ntriples").unwrap();
    let mut index = GeoIndex::build(&graph);
    assert_eq!(index.len(), 2);

    // 1. Plain insert of a new asWKT entity.
    let ins = [[iri("http://e.org/c"), iri(as_wkt), lit("POINT(2 50)")]];
    graph.apply_delta(&ins, &[]).unwrap();
    index.apply_delta(&graph, &ins, &[]);
    assert_eq!(index.len(), 3);
    assert_matches_fresh_build(&index, &graph);

    // 2. Delete one entity's geometry.
    let del = [[iri("http://e.org/b"), iri(as_wkt), lit("POINT(1 50)")]];
    graph.apply_delta(&[], &del).unwrap();
    index.apply_delta(&graph, &[], &del);
    assert_eq!(index.len(), 2);
    assert_matches_fresh_build(&index, &graph);

    // 3. Geometry UPDATE: delete + insert in one batch.
    let del = [[iri("http://e.org/a"), iri(as_wkt), lit("POINT(0 50)")]];
    let ins = [[iri("http://e.org/a"), iri(as_wkt), lit("POINT(0.5 50.5)")]];
    graph.apply_delta(&ins, &del).unwrap();
    index.apply_delta(&graph, &ins, &del);
    assert_eq!(index.len(), 2);
    assert_matches_fresh_build(&index, &graph);
    let hits = index.within_distance(Point::new(0.5, 50.5), 1_000.0, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.to_string(), "<http://e.org/a>");

    // 4. Ownership change: a feature claims c's geometry node via hasGeometry —
    //    the entry must be re-keyed from the node to the owning feature.
    let ins = [[iri("http://e.org/feature"), iri(has_geom), iri("http://e.org/c")]];
    graph.apply_delta(&ins, &[]).unwrap();
    index.apply_delta(&graph, &ins, &[]);
    assert_matches_fresh_build(&index, &graph);
    let hits = index.within_distance(Point::new(2.0, 50.0), 1_000.0, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.to_string(), "<http://e.org/feature>");

    // 5. …and dropping the ownership re-keys it back to the node.
    let del = [[iri("http://e.org/feature"), iri(has_geom), iri("http://e.org/c")]];
    graph.apply_delta(&[], &del).unwrap();
    index.apply_delta(&graph, &[], &del);
    assert_matches_fresh_build(&index, &graph);
    let hits = index.within_distance(Point::new(2.0, 50.0), 1_000.0, None);
    assert_eq!(hits[0].0.to_string(), "<http://e.org/c>");

    // 6. Non-geo triples are a no-op; a skipped (unparseable) insert counts.
    let before_skipped = index.skipped();
    let ins = [
        [iri("http://e.org/x"), iri("http://e.org/p"), iri("http://e.org/y")],
        [iri("http://e.org/bad"), iri(as_wkt), lit("POINT(broken")],
    ];
    graph.apply_delta(&ins, &[]).unwrap();
    index.apply_delta(&graph, &ins, &[]);
    assert_eq!(index.skipped(), before_skipped + 1);
    assert_matches_fresh_build(&index, &graph);
}

#[test]
fn apply_delta_random_churn_matches_fresh_build() {
    let wkt = "http://www.opengis.net/ont/geosparql#wktLiteral";
    let as_wkt_iri = "http://www.opengis.net/ont/geosparql#asWKT";
    let iri = |s: String| Term::NamedNode(oxrdf::NamedNode::new_unchecked(s));
    let lit = |s: String| {
        Term::Literal(oxrdf::Literal::new_typed_literal(s, oxrdf::NamedNode::new_unchecked(wkt)))
    };

    let mut graph = random_point_graph(300, 5, (-1.0, 1.0), (50.0, 52.0));
    let mut index = GeoIndex::build(&graph);
    let mut rng = StdRng::seed_from_u64(11);

    // (entity, wkt) pairs added by churn that are still live (deletable).
    let mut live: Vec<(String, String)> = Vec::new();
    for round in 0..20 {
        let mut inserts: Vec<[Term; 3]> = Vec::new();
        let mut deletes: Vec<[Term; 3]> = Vec::new();
        for j in 0..rng.random_range(1..6) {
            let x: f64 = rng.random_range(-1.0..1.0);
            let y: f64 = rng.random_range(50.0..52.0);
            let entity = format!("http://example.org/new{round}_{j}");
            let geom = format!("POINT({x} {y})");
            inserts.push([iri(entity.clone()), iri(as_wkt_iri.to_string()), lit(geom.clone())]);
            live.push((entity, geom));
        }
        for _ in 0..rng.random_range(0..3) {
            if live.is_empty() {
                break;
            }
            let (entity, geom) = live.swap_remove(rng.random_range(0..live.len()));
            deletes.push([iri(entity), iri(as_wkt_iri.to_string()), lit(geom)]);
        }
        graph.apply_delta(&inserts, &deletes).unwrap();
        index.apply_delta(&graph, &inserts, &deletes);
    }

    let fresh = GeoIndex::build(&graph);
    assert_eq!(index.len(), fresh.len());
    let center = Point::new(0.0, 51.0);
    let a: Vec<String> = index
        .within_distance(center, 100_000.0, None)
        .iter()
        .map(|(t, d)| format!("{t}@{d:.3}"))
        .collect();
    let b: Vec<String> = fresh
        .within_distance(center, 100_000.0, None)
        .iter()
        .map(|(t, d)| format!("{t}@{d:.3}"))
        .collect();
    assert_eq!(a, b);
}

#[test]
fn unparseable_and_non_geographic_literals_are_skipped() {
    let nt = r##"<http://e.org/good> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(1 2)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/bad> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(broken"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/plain> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(3 4)" .
<http://e.org/bng> <http://www.opengis.net/ont/geosparql#asWKT> "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/empty> <http://www.opengis.net/ont/geosparql#asWKT> "POINT EMPTY"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
"##;
    let graph = Graph::load_str(nt, "ntriples").unwrap();
    let index = GeoIndex::build(&graph);
    assert_eq!(index.len(), 1);
    assert_eq!(index.skipped(), 4);
}

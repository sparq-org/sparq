//! sq-9h1r — DE-9IM differential vs the upstream `geo` crate.
//!
//! sparq-geo computes the eight simple-features relations via `geo`'s `Relate`
//! (the DE-9IM IntersectionMatrix) and the Egenhofer / RCC8 families via
//! hand-written DE-9IM *pattern strings* matched against that same matrix. `geo`
//! ALSO ships standalone predicate traits — `Contains` and `Intersects` — whose
//! implementations are SEPARATE from `Relate` (different algorithms, not a
//! re-derivation of the matrix). This test is a **differential**: over a corpus
//! of randomly-generated geometry pairs it cross-checks
//!
//!   * `geof::sf_intersects` against `geo::Intersects`,
//!   * `geof::sf_contains`   against `geo::Contains`,
//!   * `geof::sf_within(a,b)` against `geo::Contains` with operands swapped
//!     (`geo` has no `Within` trait; `within(a,b) ≡ contains(b,a)`),
//!
//! and asserts agreement in the load-bearing direction. A disagreement there is
//! a real signal: either a bug in sparq-geo's wrapper or a genuine semantic
//! divergence between `Relate`-DE-9IM and the standalone predicate. The one
//! known, deliberate divergence is on `sfContains`: OGC `contains` (via `Relate`,
//! pattern `T*****FF*`) requires the operands' INTERIORS to intersect, so it is
//! `false` for a sub-geometry lying entirely on the boundary, whereas
//! `geo::Contains` can answer `true` for such boundary-only containment. The
//! `sfContains` check below therefore only asserts the direction that catches an
//! over-permissive bug (sparq-true must imply geo-true); the OGC
//! interior-vs-boundary asymmetric case is not asserted on.
//!
//! It additionally checks the **DE-9IM matrix laws** that any correct relation
//! set must obey (`sfDisjoint ≡ ¬sfIntersects`, `sfWithin(a,b) ≡
//! sfContains(b,a)`, the eh/rcc8 families partition region space, eh/rcc8 vs the
//! generic `geof::relate` pattern), exercising the hand-written eh/rcc8 patterns
//! against the matrix `geo` computes — the part of sparq-geo that is NOT a thin
//! pass-through and where a wrong pattern string would hide.
//!
//! Corpus: deterministically seeded (reproducible). Size is reported in the test
//! output and asserted to exceed [`MIN_CORPUS_PAIRS`].

use geo::{Contains, Intersects};
use geo_types::{Coord, Geometry, LineString, Point, Polygon};
use rand::rngs::StdRng;
// sq-8xug: `random_range` moved from `Rng` to `RngExt` in rand 0.10.
use rand::{RngExt, SeedableRng};
use sparq_geo::geof;
use sparq_geo::{GeoGeometry, Crs};

/// The differential must run over at least this many generated pairs, so the
/// corpus can't silently shrink to nothing.
const MIN_CORPUS_PAIRS: usize = 400;

/// Build a CRS84 [`GeoGeometry`] around an existing geo-types geometry.
fn g(geometry: Geometry<f64>) -> GeoGeometry {
    GeoGeometry { crs: Crs::Crs84, geometry }
}

/// A small axis-aligned square of side `s` with lower-left corner `(x, y)`.
fn square(x: f64, y: f64, s: f64) -> Geometry<f64> {
    Geometry::Polygon(Polygon::new(
        LineString(vec![
            Coord { x, y },
            Coord { x: x + s, y },
            Coord { x: x + s, y: y + s },
            Coord { x, y: y + s },
            Coord { x, y },
        ]),
        vec![],
    ))
}

/// A random geometry drawn from points, segments, and squares on an integer-ish
/// lattice (so pairs frequently share vertices / edges — the topologically
/// interesting cases, not just disjoint blobs).
fn random_geometry(rng: &mut StdRng) -> Geometry<f64> {
    // Snap to a coarse lattice so touches / equals / collinear-overlap occur
    // with meaningful frequency.
    let coord = |rng: &mut StdRng| -> f64 { rng.random_range(0..6) as f64 };
    match rng.random_range(0..4) {
        0 => Geometry::Point(Point::new(coord(rng), coord(rng))),
        1 => {
            let (x0, y0, x1, y1) = (coord(rng), coord(rng), coord(rng), coord(rng));
            Geometry::LineString(LineString(vec![
                Coord { x: x0, y: y0 },
                Coord { x: x1, y: y1 },
            ]))
        }
        2 => {
            // A two-segment polyline.
            Geometry::LineString(LineString(vec![
                Coord { x: coord(rng), y: coord(rng) },
                Coord { x: coord(rng), y: coord(rng) },
                Coord { x: coord(rng), y: coord(rng) },
            ]))
        }
        _ => {
            let s = rng.random_range(1..3) as f64;
            square(coord(rng), coord(rng), s)
        }
    }
}

/// Skip degenerate inputs the relation algebra treats as empty (a zero-length
/// segment, a zero-area "square"). They are not interesting for the differential
/// and `geo`'s predicates and `Relate` can legitimately differ on empties.
fn is_degenerate(geom: &Geometry<f64>) -> bool {
    match geom {
        Geometry::LineString(ls) => {
            ls.0.windows(2).all(|w| w[0] == w[1])
        }
        Geometry::Polygon(p) => {
            use geo::Area;
            p.unsigned_area() == 0.0
        }
        _ => false,
    }
}

struct Stats {
    pairs: usize,
    intersects_checked: usize,
    contains_checked: usize,
    within_checked: usize,
    disagreements: Vec<String>,
}

#[test]
fn de9im_differential_vs_geo_predicates() {
    let mut rng = StdRng::seed_from_u64(0x9417_2026_u64);
    let mut stats = Stats {
        pairs: 0,
        intersects_checked: 0,
        contains_checked: 0,
        within_checked: 0,
        disagreements: Vec::new(),
    };

    // Generate pairs; keep only non-degenerate ones.
    let mut generated = 0usize;
    while stats.pairs < 500 && generated < 20_000 {
        generated += 1;
        let ga = random_geometry(&mut rng);
        let gb = random_geometry(&mut rng);
        if is_degenerate(&ga) || is_degenerate(&gb) {
            continue;
        }
        stats.pairs += 1;
        let a = g(ga.clone());
        let b = g(gb.clone());

        // ---- sfIntersects vs geo::Intersects (separate impl) --------------
        let sparq_inter = geof::sf_intersects(&a, &b).unwrap();
        let geo_inter = ga.intersects(&gb);
        stats.intersects_checked += 1;
        if sparq_inter != geo_inter {
            stats.disagreements.push(format!(
                "INTERSECTS sparq={sparq_inter} geo={geo_inter} | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // ---- sfContains vs geo::Contains ----------------------------------
        // DOCUMENTED DIVERGENCE: the OGC simple-features `contains`
        // (which sparq-geo implements via Relate, pattern `T*****FF*`) requires
        // the operands' INTERIORS to intersect, so a geometry does NOT contain a
        // sub-geometry that lies entirely on its boundary. `geo::Contains` uses a
        // point-set ⊇ test on some type pairs and can answer `true` for a
        // boundary-only containment. So we assert ONLY the load-bearing
        // direction: sparq-true must imply geo-true (an over-permissive
        // sfContains is a bug). The reverse asymmetric case — geo-true while
        // sparq-false (OGC interior-vs-boundary) — is the known divergence and is
        // simply NOT asserted on here.
        let sparq_contains = geof::sf_contains(&a, &b).unwrap();
        let geo_contains = ga.contains(&gb);
        stats.contains_checked += 1;
        if sparq_contains && !geo_contains {
            stats.disagreements.push(format!(
                "CONTAINS sparq=true geo=false (sparq stronger — bug) | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // ---- sfWithin(a,b) vs geo::Contains(b,a) --------------------------
        let sparq_within = geof::sf_within(&a, &b).unwrap();
        let geo_b_contains_a = gb.contains(&ga);
        stats.within_checked += 1;
        if sparq_within && !geo_b_contains_a {
            stats.disagreements.push(format!(
                "WITHIN sparq=true (b.contains(a))=false (sparq stronger — bug) | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }
    }

    println!("\nDE-9IM differential vs geo crate predicates");
    println!(
        "corpus pairs {} (generated {generated}); checks: intersects {}, contains {}, within {}",
        stats.pairs, stats.intersects_checked, stats.contains_checked, stats.within_checked
    );
    for d in &stats.disagreements {
        println!("  DISAGREE {d}");
    }

    assert!(
        stats.pairs >= MIN_CORPUS_PAIRS,
        "corpus too small: {} < {MIN_CORPUS_PAIRS}",
        stats.pairs
    );
    assert!(
        stats.disagreements.is_empty(),
        "{} differential disagreement(s) vs the geo crate:\n{}",
        stats.disagreements.len(),
        stats.disagreements.join("\n")
    );
}

/// The DE-9IM relation algebra laws, checked over the same generated corpus.
/// These exercise the hand-written eh/rcc8 DE-9IM patterns against the matrix
/// `geo` computes (the non-pass-through part of sparq-geo), plus the structural
/// invariants every correct relation set obeys.
#[test]
fn de9im_relation_laws_hold_over_corpus() {
    let mut rng = StdRng::seed_from_u64(0xC0FFEE_u64);
    let mut pairs = 0usize;
    let mut region_pairs = 0usize;
    let mut violations: Vec<String> = Vec::new();

    let mut generated = 0usize;
    while pairs < 500 && generated < 20_000 {
        generated += 1;
        let ga = random_geometry(&mut rng);
        let gb = random_geometry(&mut rng);
        if is_degenerate(&ga) || is_degenerate(&gb) {
            continue;
        }
        pairs += 1;
        let a = g(ga.clone());
        let b = g(gb.clone());

        // Law 1: disjoint == not intersects (a DE-9IM tautology).
        let disjoint = geof::sf_disjoint(&a, &b).unwrap();
        let intersects = geof::sf_intersects(&a, &b).unwrap();
        if disjoint == intersects {
            violations.push(format!(
                "disjoint({disjoint}) should be ¬intersects({intersects}) | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // Law 2: within(a,b) == contains(b,a).
        let within = geof::sf_within(&a, &b).unwrap();
        let contains_ba = geof::sf_contains(&b, &a).unwrap();
        if within != contains_ba {
            violations.push(format!(
                "within(a,b)={within} != contains(b,a)={contains_ba} | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // Law 3: equals is symmetric.
        if geof::sf_equals(&a, &b).unwrap() != geof::sf_equals(&b, &a).unwrap() {
            violations.push(format!(
                "equals not symmetric | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // Law 4: touches is symmetric.
        if geof::sf_touches(&a, &b).unwrap() != geof::sf_touches(&b, &a).unwrap() {
            violations.push(format!(
                "touches not symmetric | a={} b={}",
                a.to_wkt_literal(),
                b.to_wkt_literal()
            ));
        }

        // Region-only laws: the eh/rcc8 families PARTITION region-region space —
        // exactly one relation per family is true for any pair of regions
        // (polygons with non-empty interior).
        if matches!(ga, Geometry::Polygon(_)) && matches!(gb, Geometry::Polygon(_)) {
            region_pairs += 1;
            // Egenhofer family (8 relations, mutually exclusive + exhaustive).
            let eh = [
                geof::eh_equals(&a, &b).unwrap(),
                geof::eh_disjoint(&a, &b).unwrap(),
                geof::eh_meet(&a, &b).unwrap(),
                geof::eh_overlap(&a, &b).unwrap(),
                geof::eh_covers(&a, &b).unwrap(),
                geof::eh_covered_by(&a, &b).unwrap(),
                geof::eh_inside(&a, &b).unwrap(),
                geof::eh_contains(&a, &b).unwrap(),
            ];
            let eh_true = eh.iter().filter(|x| **x).count();
            if eh_true != 1 {
                violations.push(format!(
                    "Egenhofer family non-partition: {eh_true} true (expected 1) | a={} b={}",
                    a.to_wkt_literal(),
                    b.to_wkt_literal()
                ));
            }
            // RCC8 family (8 relations, mutually exclusive + exhaustive).
            let rcc8 = [
                geof::rcc8_eq(&a, &b).unwrap(),
                geof::rcc8_dc(&a, &b).unwrap(),
                geof::rcc8_ec(&a, &b).unwrap(),
                geof::rcc8_po(&a, &b).unwrap(),
                geof::rcc8_tpp(&a, &b).unwrap(),
                geof::rcc8_tppi(&a, &b).unwrap(),
                geof::rcc8_ntpp(&a, &b).unwrap(),
                geof::rcc8_ntppi(&a, &b).unwrap(),
            ];
            let rcc8_true = rcc8.iter().filter(|x| **x).count();
            if rcc8_true != 1 {
                violations.push(format!(
                    "RCC8 family non-partition: {rcc8_true} true (expected 1) | a={} b={}",
                    a.to_wkt_literal(),
                    b.to_wkt_literal()
                ));
            }
        }
    }

    println!("\nDE-9IM relation laws over corpus");
    println!("pairs {pairs} ({region_pairs} region-region pairs)");
    for v in &violations {
        println!("  VIOLATION {v}");
    }
    assert!(pairs >= MIN_CORPUS_PAIRS, "corpus too small: {pairs} < {MIN_CORPUS_PAIRS}");
    assert!(region_pairs > 0, "no region-region pairs generated — partition laws untested");
    assert!(
        violations.is_empty(),
        "{} DE-9IM relation-law violation(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// The eh/rcc8 families are derived from hand-written DE-9IM pattern strings;
/// cross-check each against the GENERIC `geof::relate(a, b, pattern)` (which
/// matches the SAME pattern against `geo`'s matrix) for a representative region
/// pair set — guarding the literal pattern strings in the macros against a typo.
#[test]
fn eh_rcc8_patterns_match_generic_relate() {
    // (relation fn, its single canonical DE-9IM pattern). The multi-pattern
    // relations (ehMeet) are excluded — they are a disjunction, not one matrix.
    const SMALL: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))";
    const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    const TANGENT: &str = "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))";
    const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
    const ADJ: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))";
    const OVL: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))";
    let regions = [SMALL, BIG, TANGENT, FAR, ADJ, OVL];

    type Rel = fn(&str, &str) -> Result<bool, GeoError>;
    use sparq_geo::geof::lex;
    use sparq_geo::GeoError;
    // Single-matrix relations and the DE-9IM pattern the spec assigns them.
    let single_pattern: &[(Rel, &str)] = &[
        (lex::eh_equals, "TFFFTFFFT"),
        (lex::eh_overlap, "T*T***T**"),
        (lex::eh_covers, "T*TFT*FF*"),
        (lex::eh_covered_by, "TFF*TFT**"),
        (lex::eh_inside, "TFF*FFT**"),
        (lex::eh_contains, "T*TFF*FF*"),
        (lex::rcc8_eq, "TFFFTFFFT"),
        (lex::rcc8_dc, "FFTFFTTTT"),
        (lex::rcc8_ec, "FFTFTTTTT"),
        (lex::rcc8_po, "TTTTTTTTT"),
        (lex::rcc8_tppi, "TTTFTTFFT"),
        (lex::rcc8_tpp, "TFFTTFTTT"),
        (lex::rcc8_ntpp, "TFFTFFTTT"),
        (lex::rcc8_ntppi, "TTTFFTFFT"),
    ];

    let mut checks = 0usize;
    for a in &regions {
        for b in &regions {
            for (rel, pattern) in single_pattern {
                let via_named = rel(a, b).unwrap();
                let via_generic = lex::relate(a, b, pattern).unwrap();
                assert_eq!(
                    via_named, via_generic,
                    "pattern {pattern:?} mismatch on ({a}, {b}): named={via_named} generic={via_generic}"
                );
                checks += 1;
            }
        }
    }
    println!("\neh/rcc8 pattern cross-check: {checks} assertions");
    assert!(checks > 0);
}

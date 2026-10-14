//! sq-iwulr — GeoSPARQL geometry-function DIFFERENTIAL vs a captured
//! JTS/GEOS golden truth-table.
//!
//! sparq-geo computes the GeoSPARQL topology + measurement functions by wrapping
//! the georust stack (`geo`'s `Relate` DE-9IM, planar distance, planar buffer).
//! Its existing differentials cross-check georust against ITSELF (`Relate` vs the
//! standalone `Intersects`/`Contains` traits) and against haversine/geodesic
//! oracles. This lane adds the missing EXTERNAL oracle: the industry-reference
//! GEOS library (the C++ port of JTS — what PostGIS, Shapely, GDAL, and most
//! GeoSPARQL stores execute), whose results over a small committed WKT corpus
//! were captured OFFLINE into `bench/geo/function-golden/*.tsv` by
//! `bench/geo/function-golden/capture.py` (tool versions in each fixture header;
//! GEOS is NEVER run in CI — CI only reads the committed truth-tables).
//!
//! What is asserted — result-equivalence with GEOS on EVERY committed row:
//!   * the 8 OGC simple-features predicates (`geof:sfEquals` … `geof:sfOverlaps`)
//!     over the full ordered cross-product of the corpus (`relations.tsv`);
//!   * the generic `geof:relate` against the exact 9-char DE-9IM intersection
//!     matrix GEOS computed for each pair (`de9im.tsv`) — a matrix-level check,
//!     strictly stronger than the boolean predicates;
//!   * `geof:distance` in the DEGREE unit (planar coordinate-space distance —
//!     the same metric GEOS's Cartesian `distance` measures) within a tight
//!     relative tolerance (`distance.tsv`);
//!   * buffer-membership: `geof:sfWithin(probe, geof:buffer(g, r, degree))`
//!     against GEOS's `probe.within(g.buffer(r))` (`buffer_relation.tsv`) — the
//!     capture script only commits rows whose probe sits ≥ 20 % of the radius
//!     away from the buffer boundary, so the engines' different arc
//!     discretisations cannot flip a row.
//!
//! Everything is driven through sparq-geo's PUBLIC lexical API (`geof::lex` —
//! the exact string-in/value-out shape a SPARQL engine builtin sees), so the WKT
//! literal parser is on the tested path too. The crate is pulled with
//! `default-features = false`: pure geometry, no engine.
//!
//! HONESTY SCOPE: this is ORACLE-AGREEMENT ON THE COMMITTED CORPUS, not a proof
//! of OGC completeness (the OGC compliance ratchets live in sparq-geo's own
//! tests). A disagreement is a real signal — a sparq-geo wrapper bug, a georust
//! bug, or a genuine GEOS divergence — and must be investigated, never
//! tolerance-laundered. NON-VACUITY is self-asserted by
//! `corrupted_golden_row_is_detected`: flipping a real golden row's expected
//! value makes the row checker report a mismatch.

// With the lane feature OFF this runner is a single self-SKIP test, so
// the bare `cargo test -p sparq-conformance` and the default `--workspace` shards
// neither link the georust geometry stack nor go red. (cfg gate, not a runtime
// branch — zero geometry code compiles in the default state.)
#[cfg(not(feature = "geo"))]
#[test]
fn geo_function_differential_skipped_without_feature() {
    eprintln!(
        "SKIP: GeoSPARQL geometry-function differential lane is OFF — build with \
         `--features geo` to run it against bench/geo/function-golden/."
    );
}

#[cfg(feature = "geo")]
mod gated {
    use sparq_geo::geof::lex;
    use std::path::PathBuf;

    /// The OGC `uom:degree` IRI — selects planar coordinate-space distance /
    /// buffer radius, the same Cartesian metric the GEOS oracle measures.
    const DEGREE_IRI: &str = "http://www.opengis.net/def/uom/OGC/1.0/degree";

    /// Distance rows must agree within this RELATIVE tolerance (absolute below
    /// 1.0 coordinate units). Both engines compute exact planar segment
    /// arithmetic; observed agreement is ~1e-15 — 1e-9 only absorbs benign
    /// last-bit differences, it can never launder a wrong nearest-feature pair.
    const DISTANCE_RTOL: f64 = 1e-9;

    // Row-count FLOORS: the committed fixture cannot silently shrink (a
    // truncated or mis-parsed golden file fails loudly instead of passing
    // vacuously over zero rows). Raise when the corpus grows; never lower.
    const MIN_RELATION_ROWS: usize = 3200; // 8 predicates × 20×20 pairs
    const MIN_MATRIX_ROWS: usize = 400;
    const MIN_DISTANCE_ROWS: usize = 400;
    const MIN_BUFFER_ROWS: usize = 10;

    /// `bench/geo/function-golden/` at the workspace root. The fixture is
    /// COMMITTED (unlike the fetched W3C suites) — a missing file is a broken
    /// checkout, so the reader panics rather than skips.
    fn golden_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/geo/function-golden")
    }

    /// Reads a golden TSV: `#`-prefixed header lines and blanks are skipped,
    /// every remaining line is split on tabs.
    fn read_rows(name: &str) -> Vec<Vec<String>> {
        let path = golden_dir().join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "committed golden fixture {} missing/unreadable ({e}) — \
                 bench/geo/function-golden/ ships with the repo",
                path.display()
            )
        });
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split('\t').map(str::to_owned).collect())
            .collect()
    }

    /// gid → WKT lexical form (plain WKT ⇒ default CRS84), from `geometries.tsv`.
    fn corpus() -> std::collections::BTreeMap<String, String> {
        read_rows("geometries.tsv")
            .into_iter()
            .map(|r| {
                assert_eq!(r.len(), 2, "malformed geometries.tsv row {r:?}");
                (r[0].clone(), r[1].clone())
            })
            .collect()
    }

    fn wkt<'a>(corpus: &'a std::collections::BTreeMap<String, String>, gid: &str) -> &'a str {
        corpus
            .get(gid)
            .unwrap_or_else(|| panic!("golden row references unknown geometry id {gid:?}"))
    }

    fn parse_bool(s: &str) -> bool {
        match s {
            "true" => true,
            "false" => false,
            other => panic!("malformed golden boolean {other:?}"),
        }
    }

    // ---- Per-row checkers. Each returns Err(description) on a mismatch so the
    // ---- suite can report EVERY disagreement (and the non-vacuity test can
    // ---- assert a corrupted row is caught) instead of stopping at the first.

    /// One `relations.tsv` row: dispatch the named simple-features predicate
    /// through the PUBLIC `geof::lex` API and compare with the GEOS boolean.
    fn check_relation_row(
        corpus: &std::collections::BTreeMap<String, String>,
        row: &[String],
    ) -> Result<(), String> {
        let (func, a, b, expected) = (&row[0], &row[1], &row[2], parse_bool(&row[3]));
        let (aw, bw) = (wkt(corpus, a), wkt(corpus, b));
        let got = match func.as_str() {
            "sfEquals" => lex::sf_equals(aw, bw),
            "sfDisjoint" => lex::sf_disjoint(aw, bw),
            "sfIntersects" => lex::sf_intersects(aw, bw),
            "sfTouches" => lex::sf_touches(aw, bw),
            "sfCrosses" => lex::sf_crosses(aw, bw),
            "sfWithin" => lex::sf_within(aw, bw),
            "sfContains" => lex::sf_contains(aw, bw),
            "sfOverlaps" => lex::sf_overlaps(aw, bw),
            other => panic!("golden names unknown predicate {other:?}"),
        }
        .map_err(|e| format!("{func}({a}, {b}) errored: {e:?}"))?;
        if got == expected {
            Ok(())
        } else {
            Err(format!(
                "{func}({a}, {b}): sparq-geo={got}, GEOS golden={expected}"
            ))
        }
    }

    /// One `de9im.tsv` row: sparq-geo's intersection matrix must MATCH the exact
    /// matrix GEOS computed (digits compare intersection dimensions exactly, so
    /// this is stronger than any single boolean predicate).
    fn check_matrix_row(
        corpus: &std::collections::BTreeMap<String, String>,
        row: &[String],
    ) -> Result<(), String> {
        let (a, b, matrix) = (&row[0], &row[1], &row[2]);
        let got = lex::relate(wkt(corpus, a), wkt(corpus, b), matrix)
            .map_err(|e| format!("relate({a}, {b}, {matrix:?}) errored: {e:?}"))?;
        if got {
            Ok(())
        } else {
            Err(format!(
                "relate({a}, {b}): sparq-geo matrix does not match GEOS golden {matrix:?}"
            ))
        }
    }

    // Keep the cfg-gated constant in code spans so rustdoc does not resolve it
    // when this optional lane is disabled (the #926 intra-doc-link trap).
    /// One `distance.tsv` row: degree-unit (planar coordinate-space) distance
    /// within `DISTANCE_RTOL`.
    fn check_distance_row(
        corpus: &std::collections::BTreeMap<String, String>,
        row: &[String],
    ) -> Result<(), String> {
        let (a, b) = (&row[0], &row[1]);
        let expected: f64 = row[2].parse().expect("malformed golden distance");
        let got = lex::distance(wkt(corpus, a), wkt(corpus, b), DEGREE_IRI)
            .map_err(|e| format!("distance({a}, {b}) errored: {e:?}"))?;
        if (got - expected).abs() <= DISTANCE_RTOL * expected.abs().max(1.0) {
            Ok(())
        } else {
            Err(format!(
                "distance({a}, {b}): sparq-geo={got}, GEOS golden={expected} \
                 (|Δ| > {DISTANCE_RTOL} relative)"
            ))
        }
    }

    /// One `buffer_relation.tsv` row: buffer through the public lexical API,
    /// then test probe membership — composition of two public functions.
    fn check_buffer_row(
        corpus: &std::collections::BTreeMap<String, String>,
        row: &[String],
    ) -> Result<(), String> {
        let (gid, probe) = (&row[0], &row[2]);
        let radius: f64 = row[1].parse().expect("malformed golden radius");
        let expected = parse_bool(&row[3]);
        let buffered = lex::buffer(wkt(corpus, gid), radius, DEGREE_IRI)
            .map_err(|e| format!("buffer({gid}, {radius}) errored: {e:?}"))?;
        let got = lex::sf_within(probe, &buffered)
            .map_err(|e| format!("sfWithin({probe}, buffer({gid}, {radius})) errored: {e:?}"))?;
        if got == expected {
            Ok(())
        } else {
            Err(format!(
                "sfWithin({probe}, buffer({gid}, {radius})): sparq-geo={got}, \
                 GEOS golden={expected}"
            ))
        }
    }

    /// Runs `check` over every row of `file`, asserting the row-count floor and
    /// zero mismatches (reporting ALL of them, capped for readability).
    fn run_table(
        file: &str,
        columns: usize,
        floor: usize,
        check: impl Fn(&std::collections::BTreeMap<String, String>, &[String]) -> Result<(), String>,
    ) {
        let corpus = corpus();
        let rows = read_rows(file);
        assert!(
            rows.len() >= floor,
            "{file}: only {} golden rows (< floor {floor}) — truncated fixture?",
            rows.len()
        );
        let mut mismatches: Vec<String> = Vec::new();
        for row in &rows {
            assert_eq!(row.len(), columns, "malformed {file} row {row:?}");
            if let Err(m) = check(&corpus, row) {
                mismatches.push(m);
            }
        }
        assert!(
            mismatches.is_empty(),
            "{file}: {} of {} rows disagree with the GEOS golden truth-table \
             (a sparq-geo/georust bug or a genuine GEOS divergence — investigate, \
             do not widen tolerances):\n  {}",
            mismatches.len(),
            rows.len(),
            mismatches[..mismatches.len().min(25)].join("\n  ")
        );
        eprintln!(
            "{file}: all {} rows agree with the GEOS golden truth-table",
            rows.len()
        );
    }

    #[test]
    fn sf_relations_match_geos_golden() {
        run_table("relations.tsv", 4, MIN_RELATION_ROWS, check_relation_row);
    }

    #[test]
    fn de9im_matrices_match_geos_golden() {
        run_table("de9im.tsv", 3, MIN_MATRIX_ROWS, check_matrix_row);
    }

    #[test]
    fn distances_match_geos_golden() {
        run_table("distance.tsv", 3, MIN_DISTANCE_ROWS, check_distance_row);
    }

    #[test]
    fn buffer_relations_match_geos_golden() {
        run_table("buffer_relation.tsv", 4, MIN_BUFFER_ROWS, check_buffer_row);
    }

    /// NON-VACUITY (the bead's mutation-witness, kept as a permanent in-tree
    /// assertion): corrupting a REAL golden row — flipping a boolean, breaking a
    /// matrix, shifting a distance — must make the corresponding row checker
    /// report a mismatch. If the checkers ever degenerate into yes-machines,
    /// this test goes red.
    #[test]
    fn corrupted_golden_row_is_detected() {
        let corpus = corpus();
        let flip = |s: &str| {
            if s == "true" {
                "false".into()
            } else {
                "true".to_string()
            }
        };

        // A real relations.tsv row with its expected boolean negated.
        let row = read_rows("relations.tsv")
            .into_iter()
            .next()
            .expect("relations.tsv has rows");
        let mut bad = row.clone();
        bad[3] = flip(&row[3]);
        assert!(check_relation_row(&corpus, &row).is_ok());
        assert!(
            check_relation_row(&corpus, &bad).is_err(),
            "negated golden boolean was NOT detected — the relation checker is vacuous"
        );

        // A real de9im.tsv row with an impossible matrix (a disjoint-style
        // matrix cannot match any pair whose golden matrix differs — and for a
        // self-pair we corrupt toward disjoint, which can never hold).
        let row = read_rows("de9im.tsv")
            .into_iter()
            .next()
            .expect("de9im.tsv has rows");
        let mut bad = row.clone();
        bad[2] = if row[2] == "FF0FFF0F2" {
            "0FFFFFFF2".into()
        } else {
            "FF0FFF0F2".to_string()
        };
        assert!(check_matrix_row(&corpus, &row).is_ok());
        assert!(
            check_matrix_row(&corpus, &bad).is_err(),
            "corrupted golden DE-9IM matrix was NOT detected — the matrix checker is vacuous"
        );

        // A real distance.tsv row shifted far outside the tolerance.
        let row = read_rows("distance.tsv")
            .into_iter()
            .next()
            .expect("distance.tsv has rows");
        let mut bad = row.clone();
        bad[2] = format!("{}", row[2].parse::<f64>().unwrap() + 1.0);
        assert!(check_distance_row(&corpus, &row).is_ok());
        assert!(
            check_distance_row(&corpus, &bad).is_err(),
            "shifted golden distance was NOT detected — the distance checker is vacuous"
        );

        // A real buffer row with its membership bit flipped.
        let row = read_rows("buffer_relation.tsv")
            .into_iter()
            .next()
            .expect("buffer_relation.tsv has rows");
        let mut bad = row.clone();
        bad[3] = flip(&row[3]);
        assert!(check_buffer_row(&corpus, &row).is_ok());
        assert!(
            check_buffer_row(&corpus, &bad).is_err(),
            "flipped golden buffer membership was NOT detected — the buffer checker is vacuous"
        );
    }
}

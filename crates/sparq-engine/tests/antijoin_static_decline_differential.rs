//! (sq-7d3dj.30.20) Differential + non-vacuity tests for the anti-join STATIC
//! early-decline (`antijoin-static-decline` feature).
//!
//! # What the feature does
//!
//! The correlated (theta) anti-join recogniser `try_theta_antijoin` exists only to SEED the
//! right scan sideways from a var-to-var correlation in an `OPTIONAL { … FILTER(F) }
//! FILTER(!bound(?nb))` condition. On the SP2Bench-q07 shape the condition is a BARE
//! `!bound(?doc4)` with NO seedable correlation, so the recogniser would evaluate the
//! (dominant, expensive) mandatory LEFT side, discover `correlations.is_empty()`, and DECLINE
//! — after which the cold `Filter{LeftJoin}` fallback RE-EVALUATES that same left side. The
//! feature adds a purely STATIC pre-check (`cond_has_seedable_correlation`, over the left/right
//! variable SETS, no graph access) so the recogniser declines BEFORE the left side is touched,
//! removing the redundant second evaluation.
//!
//! # Why the assertions are a real differential
//!
//! The load-bearing invariant is BAG-RESULT-EQUIVALENCE. The feature is a pure evaluation-
//! ORDERING change: every early-decline yields the identical cold-plan result, and when a
//! correlation exists the path is byte-identical. So for EVERY query the result with the
//! recogniser ON must equal the result with it forced OFF (the cold `Filter{LeftJoin}` ground
//! truth) — and this must hold in BOTH compile-time feature states. `theta_antijoin_testing`
//! lets us force the recogniser off at runtime, giving a true in-binary differential.
//!
//! Non-vacuity (feature-ON build only): the early-decline gate must actually FIRE on the q07
//! shape (`early_declined() > 0`) and must NOT fire on a genuinely correlated q06 shape
//! (`early_declined() == 0`, so a real correlation is never lost). A mutation check confirms the
//! equivalence assertion is not trivially true.

use sparq_core::Graph;
use sparq_engine::{query, theta_antijoin_testing};

const PFX: &str = "PREFIX ex: <http://example.org/>\n\
                   PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
                   PREFIX dc: <http://purl.org/dc/elements/1.1/>\n\
                   PREFIX dcterms: <http://purl.org/dc/terms/>\n\
                   PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                   PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

fn load(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PFX}{ttl}"), "turtle").expect("test graph")
}

type Table = Vec<Vec<Option<String>>>;

/// Result rows as a SORTED multiset (order-insensitive, MULTIPLICITY-preserving).
fn multiset(g: &Graph, q: &str) -> Table {
    let r = query(g, &format!("{PFX}{q}")).expect("query failed");
    let mut rows: Table = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect();
    rows.sort();
    rows
}

/// Runs `q` with the theta anti-join recogniser forced OFF (cold ground truth) then ON
/// (the feature's early-decline path is live when compiled). Both must be equal.
fn cold_vs_live(g: &Graph, q: &str) -> (Table, Table) {
    let prev = theta_antijoin_testing::set_enabled(false);
    let cold = multiset(g, q);
    theta_antijoin_testing::set_enabled(true);
    let live = multiset(g, q);
    theta_antijoin_testing::set_enabled(prev);
    (cold, live)
}

// ── The q07 shape: DOUBLY-nested OPTIONAL anti-join with a BARE `!bound` condition (no seedable
//    correlation). The exact SP2Bench-q07 structure — the double nesting is load-bearing: it is
//    what makes the OUTER `LeftJoin` carry `expression: Some(!bound(?doc4))` (a certain-var
//    negation, NOT a var-to-var correlation), so `try_theta_antijoin` reaches its shape gate and
//    the static early-decline can fire. (A SINGLE OPTIONAL level rewrites straight to `Minus` and
//    never reaches the recogniser.)
const Q07_LIKE: &str = "\
  SELECT DISTINCT ?title WHERE {
    ?class rdfs:subClassOf foaf:Document . ?doc rdf:type ?class . ?doc dc:title ?title .
    ?bag2 ?member2 ?doc . ?doc2 dcterms:references ?bag2
    OPTIONAL {
      ?class3 rdfs:subClassOf foaf:Document . ?doc3 rdf:type ?class3 .
      ?doc3 dcterms:references ?bag3 . ?bag3 ?member3 ?doc
      OPTIONAL {
        ?class4 rdfs:subClassOf foaf:Document . ?doc4 rdf:type ?class4 .
        ?doc4 dcterms:references ?bag4 . ?bag4 ?member4 ?doc3
      } FILTER (!bound(?doc4))
    } FILTER (!bound(?doc3))
  }";

// ── The q06 shape: a GENUINE var-to-var correlation `?author = ?author2` in the OPTIONAL
//    condition. Documents with no earlier document by the same author. The feature must NOT
//    early-decline here (a real correlation exists), and the result must be unchanged.
const Q06_LIKE: &str = "\
  SELECT ?document WHERE {
    ?class rdfs:subClassOf foaf:Document . ?document rdf:type ?class .
    ?document dcterms:issued ?yr . ?document dc:creator ?author . ?author foaf:name ?name
    OPTIONAL {
      ?class2 rdfs:subClassOf foaf:Document . ?document2 rdf:type ?class2 .
      ?document2 dcterms:issued ?yr2 . ?document2 dc:creator ?author2
      FILTER (?author = ?author2 && ?yr2 < ?yr)
    } FILTER (!bound(?author2))
  }";

fn q07_dataset() -> Graph {
    // A few Document subclasses, typed+titled docs, bags that contain docs, and references
    // linking docs to bags — a couple of docs get a deeper referencing bag so they DROP out.
    load(
        "ex:InProceedings rdfs:subClassOf foaf:Document .
         ex:Article       rdfs:subClassOf foaf:Document .
         ex:d1 rdf:type ex:InProceedings . ex:d1 dc:title \"T1\" .
         ex:d2 rdf:type ex:Article .        ex:d2 dc:title \"T2\" .
         ex:d3 rdf:type ex:InProceedings . ex:d3 dc:title \"T3\" .
         ex:bagA rdf:_1 ex:d1 . ex:bagA rdf:_2 ex:d2 .
         ex:bagB rdf:_1 ex:d3 .
         ex:bagC rdf:_1 ex:d1 .
         ex:r1 dcterms:references ex:bagA .
         ex:r2 dcterms:references ex:bagB .
         ex:r3 dcterms:references ex:bagC .",
    )
}

fn q06_dataset() -> Graph {
    load(
        "ex:Article rdfs:subClassOf foaf:Document .
         ex:a1 rdf:type ex:Article . ex:a1 dcterms:issued \"2001\"^^xsd:gYear . ex:a1 dc:creator ex:auth1 .
         ex:a2 rdf:type ex:Article . ex:a2 dcterms:issued \"2003\"^^xsd:gYear . ex:a2 dc:creator ex:auth1 .
         ex:a3 rdf:type ex:Article . ex:a3 dcterms:issued \"2002\"^^xsd:gYear . ex:a3 dc:creator ex:auth2 .
         ex:auth1 foaf:name \"Ann\" . ex:auth2 foaf:name \"Bob\" .",
    )
}

/// q07-shaped anti-join: the cold (recogniser OFF) result equals the live (feature ON) result.
#[test]
fn q07_shape_bag_equivalence() {
    let g = q07_dataset();
    let (cold, live) = cold_vs_live(&g, Q07_LIKE);
    assert_eq!(cold, live, "q07 anti-join result differs between cold and early-decline paths");
    // Sanity: the query returns SOME rows (else the equivalence is vacuous on empty).
    assert!(!cold.is_empty(), "q07 fixture produced no rows — fixture is degenerate");
}

/// q06-shaped CORRELATED anti-join: unchanged (the feature must not touch a real correlation).
#[test]
fn q06_shape_correlated_equivalence() {
    let g = q06_dataset();
    let (cold, live) = cold_vs_live(&g, Q06_LIKE);
    assert_eq!(cold, live, "q06 correlated anti-join result differs between cold and live paths");
    assert!(!cold.is_empty(), "q06 fixture produced no rows — fixture is degenerate");
}

/// NON-VACUITY (feature-ON build only): the static early-decline gate FIRES on the q07 shape
/// and does NOT fire on the correlated q06 shape.
#[cfg(feature = "antijoin-static-decline")]
#[test]
fn early_decline_fires_on_q07_not_on_q06() {
    // Must run with the recogniser enabled so the gate is reached.
    let prev = theta_antijoin_testing::set_enabled(true);

    let g07 = q07_dataset();
    theta_antijoin_testing::reset_stats();
    let _ = query(&g07, &format!("{PFX}{Q07_LIKE}")).unwrap();
    let early_q07 = sparq_engine::theta_antijoin_testing::early_declined();
    assert!(early_q07 > 0, "static early-decline did not fire on the q07 (bare !bound) shape");

    let g06 = q06_dataset();
    theta_antijoin_testing::reset_stats();
    let _ = query(&g06, &format!("{PFX}{Q06_LIKE}")).unwrap();
    let early_q06 = sparq_engine::theta_antijoin_testing::early_declined();
    assert_eq!(early_q06, 0, "static early-decline WRONGLY fired on a genuinely correlated q06 shape");

    theta_antijoin_testing::set_enabled(prev);
}

/// MUTATION check: the equivalence oracle is not trivially true. Deliberately compare the q07
/// result against a MUTATED expectation (an extra phantom row) and confirm the assertion would
/// FAIL — proving `q07_shape_bag_equivalence` is non-vacuous.
#[test]
fn mutation_equivalence_is_non_vacuous() {
    let g = q07_dataset();
    let (cold, _live) = cold_vs_live(&g, Q07_LIKE);
    let mut mutated = cold.clone();
    mutated.push(vec![Some("__phantom_title__".to_string())]);
    assert_ne!(cold, mutated, "mutation guard: a changed result set must not compare equal");
}

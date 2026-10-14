//! Acceptance tests for sideways information passing (SIP) — correlated
//! graph-pattern join evaluation (bead sq-7d3dj.30.3).
//!
//! When `Join(A, B)` is evaluated and the already-evaluated side `A` is SMALL,
//! the engine evaluates the big child `B` CORRELATED on A's bindings: for each
//! distinct binding of A's variables that are CERTAINLY bound in B (and bound to
//! an IRI in A), the IRI is substituted as a constant into B's patterns (pushed
//! into UNION branches / BGP scans / filters), so a scan seeds from the constant
//! instead of running blind. The load-bearing invariant is BAG-SEMANTICS
//! RESULT-EQUIVALENCE with correlation on vs off (multiplicities must match).
//!
//! Coverage:
//!   * `q08_shape_equivalence`        — SP2Bench q08-shaped person + Union of
//!     creator self-joins: correlated-on == correlated-off (SELECT DISTINCT and
//!     the raw multiset without DISTINCT).
//!   * `q08_shape_anti_vacuity`       — the correlated Union child actually FIRED
//!     and its produced row count COLLAPSED vs the blind (cold) evaluation.
//!   * `q12b_shape_ask`               — the ASK form (q12b) agrees on vs off.
//!   * `multiplicity_duplicate_*`     — duplicate A-rows and duplicate B-matches.
//!   * `optional_inside_union_fallback` / `minus_fallback` — a correlation var
//!     that is NOT certain in B (inside an OPTIONAL right side / MINUS) still
//!     produces the correct answer (conservative fallback to the cold path).
//!   * `public_toggle_and_stats`      — the `sip_testing` public surface.

use sparq_core::Graph;
use sparq_engine::{ask, query, sip_testing};

const PFX: &str = "PREFIX ex: <http://example.org/>\n\
                   PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
                   PREFIX dc: <http://purl.org/dc/elements/1.1/>\n\
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                   PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

fn load(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PFX}{ttl}"), "turtle").expect("test graph")
}

/// A stringified result table: one row per solution, one `Option<String>` per column.
type Table = Vec<Vec<Option<String>>>;

/// Result rows as a SORTED multiset of stringified cells (order-insensitive but
/// MULTIPLICITY-preserving — the bag-semantics oracle).
fn multiset(g: &Graph, q: &str) -> Table {
    let r = query(g, &format!("{PFX}{q}")).expect("query failed");
    let mut rows: Vec<Vec<Option<String>>> = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect();
    rows.sort();
    rows
}

/// Runs `q` with SIP forced OFF then forced ON, returning both multisets.
fn on_off(g: &Graph, q: &str) -> (Table, Table) {
    let prev = sip_testing::set_enabled(false);
    let off = multiset(g, q);
    sip_testing::set_enabled(true);
    let on = multiset(g, q);
    sip_testing::set_enabled(prev);
    (off, on)
}

// ── q08 dataset ──────────────────────────────────────────────────────────────
//
// A moderate SP2Bench-q08-shaped corpus: 10 authors (a0 = "Paul Erdoes"), 40
// documents each with 2 deterministic creators; a share of documents include
// erdoes as a creator. Blind (cold) evaluation of the Union computes a
// creator×creator self-join over ALL documents; correlated evaluation seeds it
// from `?document dc:creator <a0>` (only erdoes's documents).
fn q08_dataset() -> Graph {
    let mut ttl = String::new();
    for a in 0..10usize {
        ttl.push_str(&format!("ex:a{a} rdf:type foaf:Person .\n"));
        if a == 0 {
            ttl.push_str("ex:a0 foaf:name \"Paul Erdoes\"^^xsd:string .\n");
        } else {
            ttl.push_str(&format!("ex:a{a} foaf:name \"Author {a}\"^^xsd:string .\n"));
        }
    }
    for d in 0..40usize {
        // Two distinct creators; roughly a third of documents include erdoes (a0).
        let c1 = if d % 3 == 0 { 0 } else { 1 + (d % 9) };
        let mut c2 = 1 + ((d + 4) % 9);
        if c2 == c1 {
            c2 = 1 + (c2 % 9);
        }
        ttl.push_str(&format!("ex:d{d} dc:creator ex:a{c1} .\n"));
        ttl.push_str(&format!("ex:d{d} dc:creator ex:a{c2} .\n"));
    }
    load(&ttl)
}

const Q08_BODY: &str = "\
  ?erdoes rdf:type foaf:Person .
  ?erdoes foaf:name \"Paul Erdoes\"^^xsd:string .
  {
    ?document dc:creator ?erdoes .
    ?document dc:creator ?author .
    ?document2 dc:creator ?author .
    ?document2 dc:creator ?author2 .
    ?author2 foaf:name ?name
    FILTER (?author != ?erdoes &&
            ?document2 != ?document &&
            ?author2 != ?erdoes &&
            ?author2 != ?author)
  } UNION {
    ?document dc:creator ?erdoes .
    ?document dc:creator ?author .
    ?author foaf:name ?name
    FILTER (?author != ?erdoes)
  }
";

#[test]
fn q08_shape_equivalence() {
    let g = q08_dataset();
    // The real q08 (SELECT DISTINCT ?name).
    let (off, on) = on_off(&g, &format!("SELECT DISTINCT ?name WHERE {{\n{Q08_BODY}\n}}"));
    assert_eq!(off, on, "q08 DISTINCT result differs with SIP on vs off");
    assert!(!on.is_empty(), "q08 fixture should return co-authors of Erdoes");

    // Same pattern WITHOUT distinct — the strict bag-semantics multiplicity check
    // (every ?name/?author/?document combination, duplicates and all).
    let sel = format!("SELECT ?name ?author ?document ?document2 ?author2 WHERE {{\n{Q08_BODY}\n}}");
    let (off_bag, on_bag) = on_off(&g, &sel);
    assert_eq!(off_bag, on_bag, "q08 bag multiplicity differs with SIP on vs off");
}

#[test]
fn q08_shape_anti_vacuity() {
    let g = q08_dataset();
    let sel = format!("SELECT ?name ?author ?document ?document2 ?author2 WHERE {{\n{Q08_BODY}\n}}");

    // Cold (SIP off): read the largest Union node row count from the ANALYZE trace.
    sip_testing::set_enabled(false);
    let cold_trace =
        sparq_engine::explain_analyze(&g, &format!("{PFX}{sel}")).expect("explain_analyze");
    let cold_union = max_union_rows(&cold_trace);

    // Correlated (SIP on): assert it FIRED and produced far fewer child rows.
    sip_testing::set_enabled(true);
    sip_testing::reset_stats();
    let _ = query(&g, &format!("{PFX}{sel}")).expect("query");
    let (fired, correlated_rows, bindings) = sip_testing::stats();
    sip_testing::set_enabled(true);

    assert!(fired, "SIP did not fire on the q08-shaped join");
    assert!(bindings >= 1, "SIP recorded no distinct correlated bindings");
    assert!(cold_union > 0, "could not read cold Union row count from the trace");
    // Anti-vacuity: the correlated child collapsed to a small fraction of the blind
    // self-join. (Cold is the whole-corpus creator self-join; correlated seeds from
    // Erdoes's documents only.)
    assert!(
        correlated_rows * 2 < cold_union,
        "SIP did not collapse the child: correlated={} cold_union={}",
        correlated_rows,
        cold_union
    );
}

/// Largest `rows=` value on any `Union` line of an EXPLAIN ANALYZE trace.
fn max_union_rows(trace: &str) -> usize {
    trace
        .lines()
        .filter(|l| l.trim_start().starts_with("Union"))
        .filter_map(|l| {
            l.split("rows=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|n| n.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn q12b_shape_ask() {
    let g = q08_dataset();
    let q = format!("{PFX}ASK {{\n{Q08_BODY}\n}}");
    let prev = sip_testing::set_enabled(false);
    let off = ask(&g, &q).expect("ask off");
    sip_testing::set_enabled(true);
    let on = ask(&g, &q).expect("ask on");
    sip_testing::set_enabled(prev);
    assert_eq!(off, on, "q12b ASK differs with SIP on vs off");
    assert!(on, "q12b fixture should be satisfiable");
}

#[test]
fn multiplicity_duplicate_b_matches() {
    // One small A (?p bound once) joined with a B that has MANY matches for the
    // pushed binding — the correlated recombine must reproduce each match.
    let g = load(
        "ex:p rdf:type ex:Seed . ex:p ex:label \"seed\" .
         ex:p ex:knows ex:x . ex:p ex:knows ex:y . ex:p ex:knows ex:z .
         ex:x ex:v 1 . ex:y ex:v 1 . ex:z ex:v 1 .",
    );
    // Join( { ?p a ex:Seed . ?p ex:label ?l } , { ?p ex:knows ?f . ?f ex:v ?val } )
    let q = "SELECT ?f ?val ?l WHERE { \
             ?p rdf:type ex:Seed . ?p ex:label ?l . \
             ?p ex:knows ?f . ?f ex:v ?val }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on);
    assert_eq!(on.len(), 3, "expected three ex:knows matches");
}

#[test]
fn multiplicity_duplicate_a_rows() {
    // A (the small side) has DUPLICATE rows for the pushed variable (produced by a
    // UNION of two identical branches). Each duplicate A-row must join with the full
    // B relation for its binding — the bag join, not a dedup.
    let g = load(
        "ex:p rdf:type ex:Seed . ex:p ex:tag ex:t .
         ex:p ex:knows ex:x . ex:p ex:knows ex:y .
         ex:x ex:v 1 . ex:y ex:v 1 .",
    );
    let q = "SELECT ?f WHERE { \
             { { ?p rdf:type ex:Seed } UNION { ?p ex:tag ex:t } } \
             ?p ex:knows ?f }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "duplicate A-row multiplicity mismatch");
    // 2 A-rows (Seed + tag) × 2 B-matches = 4 rows.
    assert_eq!(on.len(), 4);
}

#[test]
fn optional_inside_union_fallback() {
    // ?x is bound by A, but in B it appears ONLY inside an OPTIONAL right side in one
    // union branch — it is NOT a certain variable of B, so SIP must NOT substitute it
    // (which would drop the unbound-in-B solutions). The answer must be correct either
    // way; this asserts the conservative-fallback path is equivalence-preserving.
    let g = load(
        "ex:a ex:p ex:x . ex:a ex:q ex:m .
         ex:x ex:r ex:n .
         ex:b ex:p ex:x .",
    );
    let q = "SELECT ?a ?m ?opt WHERE { \
             ex:a ex:q ?m . \
             { { ?a ex:p ?x } UNION { ?a ex:p ?x . OPTIONAL { ?x ex:r ?opt } } } }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "OPTIONAL-inside-UNION fallback not equivalence-preserving");
}

#[test]
fn minus_fallback() {
    // A correlation variable used inside MINUS — the cold and correlated paths must
    // agree (MINUS bound-domain semantics must survive substitution or fall back).
    let g = load(
        "ex:a ex:tag ex:t . ex:a ex:knows ex:x . ex:a ex:knows ex:y .
         ex:x ex:banned ex:t .",
    );
    let q = "SELECT ?f WHERE { \
             ex:a ex:tag ?t . \
             ?a ex:knows ?f \
             MINUS { ?f ex:banned ?t } }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on, "MINUS correlated/cold mismatch");
}

#[test]
fn cross_product_no_shared_var_unchanged() {
    // No shared/pushable variable between the two join sides — SIP must not fire and
    // the cross product must be identical.
    let g = load("ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:q 3 . ex:d ex:q 4 .");
    let q = "SELECT ?x ?y WHERE { ?x ex:p ?v . ?y ex:q ?w }";
    let (off, on) = on_off(&g, q);
    assert_eq!(off, on);
    assert_eq!(on.len(), 4);
}

#[test]
fn public_toggle_and_stats() {
    // Direct unit coverage of the public `sip_testing` surface.
    let prev = sip_testing::set_enabled(false);
    assert!(!sip_testing::set_enabled(true), "set_enabled should return the prior value");
    assert!(sip_testing::set_enabled(prev));

    sip_testing::reset_stats();
    let (fired, rows, bindings) = sip_testing::stats();
    assert!(!fired && rows == 0 && bindings == 0, "stats not cleared by reset");
}

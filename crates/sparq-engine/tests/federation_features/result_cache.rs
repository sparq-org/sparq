//! (sq-a9cn) Materialised-view / query-result cache tests.
//!
//! Only built under the opt-in `result-cache` feature; the whole file is empty
//! otherwise so the default `cargo test` is unaffected.
#![cfg(feature = "result-cache")]

use sparq_core::Graph;
use sparq_engine::PreparedQuery;
use sparq_engine::{is_cacheable, query, QueryBudget, ResultCache};

const DATA: &str = r#"@prefix ex: <http://ex/> .
    ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 .
"#;

fn g() -> Graph {
    Graph::load_str(DATA, "turtle").unwrap()
}

fn parse(q: &str) -> spargebra::Query {
    PreparedQuery::parse(q).unwrap().into_query()
}

/// A hit returns the SAME result a fresh evaluation would, and is counted as a hit.
#[test]
fn hit_matches_fresh_eval_and_counts() {
    let graph = g();
    let cache = ResultCache::new(16);
    let q = parse("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o } ORDER BY ?o");
    let budget = QueryBudget::unlimited();

    let fresh = query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o } ORDER BY ?o",
    )
    .unwrap();

    let first = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    let second = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();

    // Same row count + same bound terms as the uncached path.
    assert_eq!(first.rows.len(), fresh.rows.len());
    assert_eq!(first.vars, fresh.vars);
    for (a, b) in first.rows.iter().zip(fresh.rows.iter()) {
        assert_eq!(a, b);
    }
    // The two cached reads are the SAME Arc (a genuine hit, not a re-eval).
    assert!(std::sync::Arc::ptr_eq(&first, &second));

    let stats = cache.stats();
    assert_eq!(stats.misses, 1, "first call is a miss");
    assert_eq!(stats.hits, 1, "second call is a hit");
    assert_eq!(stats.entries, 1);
}

/// Whitespace / comment / prefix-spelling differences collapse to one entry
/// (keyed on the parsed algebra, not the source string).
#[test]
fn textual_variants_share_one_entry() {
    let graph = g();
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let a = parse("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o }");
    let b = parse("PREFIX foo: <http://ex/>\n# a comment\nSELECT   ?s WHERE {?s foo:p ?o}");

    cache.get_or_eval(&graph, &a, 0, &budget).unwrap();
    cache.get_or_eval(&graph, &b, 0, &budget).unwrap();

    let stats = cache.stats();
    assert_eq!(stats.hits, 1, "the second spelling hits the first's entry");
    assert_eq!(stats.entries, 1);
}

/// A version bump invalidates: the new version misses, and the stale generation
/// is reclaimed (no unbounded growth across epochs).
#[test]
fn version_bump_invalidates() {
    let graph = g();
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let q = parse("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o }");

    cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(cache.stats().entries, 1);
    // Advance the epoch (as a writer would after a mutation).
    cache.get_or_eval(&graph, &q, 1, &budget).unwrap();
    let stats = cache.stats();
    assert_eq!(stats.misses, 2, "the new version is a miss");
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.entries, 1, "the stale v0 entry was reclaimed");
}

/// A version bump after a real mutation serves the UPDATED result, never the stale one.
#[test]
fn updated_graph_after_version_bump_is_fresh() {
    let mut graph = g();
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let q = parse("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }");

    let before = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    let n_before = before.rows[0][0].as_ref().unwrap().to_string();

    graph
        .apply_delta_nquads(
            "<http://ex/d> <http://ex/p> \"4\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "",
        )
        .unwrap();

    // Same version => STALE hit (caller bug we deliberately demonstrate is possible).
    let stale = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(stale.rows[0][0].as_ref().unwrap().to_string(), n_before);

    // Bumped version => fresh count.
    let fresh = cache.get_or_eval(&graph, &q, 1, &budget).unwrap();
    let n_fresh = fresh.rows[0][0].as_ref().unwrap().to_string();
    assert_ne!(
        n_fresh, n_before,
        "after a mutation + version bump the count changed"
    );
}

/// Non-deterministic queries are never cached: each call re-evaluates and the store
/// stays empty.
#[test]
fn non_deterministic_not_cached() {
    let graph = g();
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();

    for src in [
        "SELECT (NOW() AS ?x) WHERE {}",
        "SELECT (RAND() AS ?x) WHERE {}",
        "SELECT (UUID() AS ?x) WHERE {}",
        "SELECT (STRUUID() AS ?x) WHERE {}",
        "SELECT (BNODE() AS ?x) WHERE {}",
    ] {
        let q = parse(src);
        assert!(!is_cacheable(&q), "{src} must be classified non-cacheable");
        cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    }
    assert_eq!(
        cache.stats().entries,
        0,
        "no non-deterministic query was stored"
    );
    assert_eq!(cache.stats().hits, 0);
}

/// A deterministic query with a non-deterministic sub-expression (e.g. inside a
/// FILTER) is also refused.
#[test]
fn non_determinism_inside_filter_refused() {
    let q = parse("SELECT ?s WHERE { ?s ?p ?o FILTER(?o > RAND()) }");
    assert!(!is_cacheable(&q));
    // A plain deterministic FILTER is fine.
    let ok = parse("SELECT ?s WHERE { ?s ?p ?o FILTER(?o > 1) }");
    assert!(is_cacheable(&ok));
}

/// Standard aggregates, ORDER BY, DISTINCT, UNION, OPTIONAL, VALUES, ASK are cacheable.
#[test]
fn standard_forms_are_cacheable() {
    for src in [
        "SELECT DISTINCT ?o WHERE { ?s ?p ?o }",
        "SELECT (SUM(?o) AS ?t) WHERE { ?s ?p ?o }",
        "SELECT ?s WHERE { { ?s ?p 1 } UNION { ?s ?p 2 } }",
        "SELECT ?s ?o2 WHERE { ?s ?p ?o OPTIONAL { ?s ?p2 ?o2 } }",
        "SELECT ?x WHERE { VALUES ?x { 1 2 3 } }",
        "ASK { ?s ?p ?o }",
    ] {
        assert!(is_cacheable(&parse(src)), "{src} should be cacheable");
    }
}

/// SERVICE (remote, time-varying) and CONSTRUCT/DESCRIBE are not cacheable.
#[test]
fn service_and_graph_forms_refused() {
    assert!(!is_cacheable(&parse(
        "SELECT * WHERE { SERVICE <http://example.org/sparql> { ?s ?p ?o } }"
    )));
    assert!(!is_cacheable(&parse(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"
    )));
    assert!(!is_cacheable(&parse("DESCRIBE ?s WHERE { ?s ?p ?o }")));
}

/// Capacity 0 disables storage; an over-capacity cache evicts the LRU entry.
#[test]
fn eviction_and_disabled() {
    let graph = g();
    let budget = QueryBudget::unlimited();

    let off = ResultCache::new(0);
    let q = parse("SELECT ?s WHERE { ?s ?p ?o }");
    off.get_or_eval(&graph, &q, 0, &budget).unwrap();
    off.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(off.stats().entries, 0);
    assert_eq!(off.stats().hits, 0, "capacity-0 never hits");

    let cache = ResultCache::new(2);
    let q1 = parse("SELECT ?s WHERE { ?s ?p 1 }");
    let q2 = parse("SELECT ?s WHERE { ?s ?p 2 }");
    let q3 = parse("SELECT ?s WHERE { ?s ?p 3 }");
    cache.get_or_eval(&graph, &q1, 0, &budget).unwrap();
    cache.get_or_eval(&graph, &q2, 0, &budget).unwrap();
    // Touch q1 so q2 becomes the LRU.
    cache.get_or_eval(&graph, &q1, 0, &budget).unwrap();
    cache.get_or_eval(&graph, &q3, 0, &budget).unwrap(); // evicts q2
    assert_eq!(cache.stats().entries, 2);
    // q1 still resident (hit), q2 evicted (miss again).
    let h0 = cache.stats().hits;
    cache.get_or_eval(&graph, &q1, 0, &budget).unwrap();
    assert_eq!(cache.stats().hits, h0 + 1, "q1 survived eviction");
    let m0 = cache.stats().misses;
    cache.get_or_eval(&graph, &q2, 0, &budget).unwrap();
    assert_eq!(cache.stats().misses, m0 + 1, "q2 was evicted, so it misses");
}

/// `clear` empties the store but keeps capacity.
#[test]
fn clear_empties() {
    let graph = g();
    let budget = QueryBudget::unlimited();
    let cache = ResultCache::new(4);
    cache
        .get_or_eval(&graph, &parse("SELECT ?s WHERE { ?s ?p ?o }"), 0, &budget)
        .unwrap();
    assert_eq!(cache.stats().entries, 1);
    cache.clear();
    assert_eq!(cache.stats().entries, 0);
    assert_eq!(cache.capacity(), 4);
}

// ---------------------------------------------------------------------------
// (sq-bif.12) WRITE-INVALIDATION coverage.
//
// The existing tests above prove the *mechanism* (a version bump misses + reclaims
// the stale generation). What was missing is the END-TO-END contract over the FOUR
// real mutation shapes — INSERT / DELETE / UPDATE / CLEAR — driven through the actual
// `Graph` mutation path: after a writer mutates and bumps the version (the documented
// soundness contract), the next `get_or_eval` at the new version must be a MISS that
// serves the POST-mutation result, and a stale generation must NEVER be served.
//
// The load-bearing assertion in each is the *value*: we don't merely count misses, we
// assert the served result equals a FRESH evaluation of the mutated graph (so the test
// fails if invalidation regressed to a stale read), and DIFFERS from the pre-mutation
// snapshot (so it fails if the mutation never reached the served result).
// ---------------------------------------------------------------------------

/// Count `?s` (the number of bound rows) for an all-triples scan — a query whose
/// result changes under any insert/delete, so it is a sharp staleness witness.
const COUNT_Q: &str = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";

/// Extracts the integer COUNT value from a `QueryResult` (a `"<n>"^^xsd:integer`
/// term) as a `usize` — the leading run of digits in the term's lexical form.
fn cached_count(result: &sparq_engine::QueryResult) -> usize {
    let term = result.rows[0][0].as_ref().unwrap().to_string();
    term.chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap()
}

/// The COUNT of a graph via a direct, uncached evaluation — the ground truth the
/// cache must agree with after a mutation.
fn count_of(graph: &Graph) -> usize {
    cached_count(&query(graph, COUNT_Q).unwrap())
}

/// INSERT: after `apply_delta_nquads` adds a triple and the writer bumps the version,
/// the cached COUNT at the new version is a MISS that serves the larger, fresh count —
/// never the stale pre-insert count.
#[test]
fn insert_invalidates_via_version_bump() {
    let mut graph = g(); // 3 triples
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let q = parse(COUNT_Q);

    let before = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(cached_count(&before), 3);
    assert_eq!(cache.stats().misses, 1);

    // Writer mutates the graph IN PLACE and bumps its epoch.
    graph
        .apply_delta_nquads(
            "<http://ex/d> <http://ex/p> \"4\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "",
        )
        .unwrap();
    let version = 1u64;

    let after = cache.get_or_eval(&graph, &q, version, &budget).unwrap();
    assert_eq!(cache.stats().misses, 2, "the new version misses (invalidated)");
    assert_eq!(cache.stats().hits, 0, "no stale hit was served");
    // The served result is the FRESH post-insert count, and it grew.
    assert_eq!(cached_count(&after), count_of(&graph));
    assert_eq!(cached_count(&after), 4);
    assert!(
        cached_count(&after) > cached_count(&before),
        "insert must increase the served count"
    );
    // The bumped read is itself now cached (a re-read at v1 is a hit on the fresh entry).
    let again = cache.get_or_eval(&graph, &q, version, &budget).unwrap();
    assert!(std::sync::Arc::ptr_eq(&after, &again));
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().entries, 1, "stale v0 entry was reclaimed");
}

/// DELETE: after `apply_delta_nquads` removes a triple and the writer bumps the
/// version, the cached COUNT at the new version serves the smaller, fresh count.
#[test]
fn delete_invalidates_via_version_bump() {
    let mut graph = g(); // 3 triples
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let q = parse(COUNT_Q);

    let before = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(cached_count(&before), 3);

    // Retract `ex:a ex:p 1`.
    graph
        .apply_delta_nquads(
            "",
            "<http://ex/a> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        )
        .unwrap();

    let after = cache.get_or_eval(&graph, &q, 1, &budget).unwrap();
    assert_eq!(cache.stats().hits, 0, "no stale hit served after delete");
    assert_eq!(cached_count(&after), count_of(&graph));
    assert_eq!(cached_count(&after), 2);
    assert!(
        cached_count(&after) < cached_count(&before),
        "delete must decrease the served count"
    );
}

/// UPDATE (SPARQL `INSERT DATA`/`DELETE DATA` via `sparq_engine::update`, which returns
/// a NEW graph): the writer queries the cache against the NEW graph at a bumped
/// version, so the prior entry (keyed on the old version) is invalidated and the served
/// result reflects the update. Also exercises a non-count query whose BOUND VALUE
/// changes — the strongest staleness witness.
#[test]
fn sparql_update_invalidates_and_serves_new_binding() {
    let graph0 = g(); // ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 .
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    // The object bound to ex:a — changes when the update rewrites it.
    let q = parse("PREFIX ex: <http://ex/> SELECT ?o WHERE { ex:a ex:p ?o }");

    let before = cache.get_or_eval(&graph0, &q, 0, &budget).unwrap();
    let o_before = before.rows[0][0].as_ref().unwrap().to_string();
    assert!(o_before.contains('1'), "ex:a starts bound to 1, got {o_before}");

    // SPARQL UPDATE: rebind ex:a from 1 to 99. `update` rebuilds a fresh graph.
    let graph1 = sparq_engine::update(
        &graph0,
        "PREFIX ex: <http://ex/> DELETE DATA { ex:a ex:p 1 } ; INSERT DATA { ex:a ex:p 99 }",
    )
    .unwrap();

    // Writer routes the read at the bumped version against the UPDATED graph.
    let after = cache.get_or_eval(&graph1, &q, 1, &budget).unwrap();
    assert_eq!(cache.stats().hits, 0, "no stale binding served after UPDATE");
    let o_after = after.rows[0][0].as_ref().unwrap().to_string();
    assert!(o_after.contains("99"), "ex:a must now bind to 99, got {o_after}");
    assert_ne!(o_before, o_after, "the served binding changed after the UPDATE");
    // It matches a fresh, uncached evaluation of the updated graph.
    let fresh = query(&graph1, "PREFIX ex: <http://ex/> SELECT ?o WHERE { ex:a ex:p ?o }").unwrap();
    assert_eq!(o_after, fresh.rows[0][0].as_ref().unwrap().to_string());
}

/// CLEAR (delete every triple + bump): the cached non-empty result is invalidated and
/// the new version serves the EMPTY result of the cleared graph — the prior, non-empty
/// generation is never served and is reclaimed.
#[test]
fn clear_invalidates_to_empty_result() {
    let mut graph = g(); // 3 triples
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let rows_q = parse("SELECT ?s WHERE { ?s ?p ?o }");

    let before = cache.get_or_eval(&graph, &rows_q, 0, &budget).unwrap();
    assert_eq!(before.rows.len(), 3, "three rows before CLEAR");
    assert_eq!(cache.stats().entries, 1);

    // CLEAR the default graph: retract all three triples.
    graph
        .apply_delta_nquads(
            "",
            "<http://ex/a> <http://ex/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://ex/b> <http://ex/p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
             <http://ex/c> <http://ex/p> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        )
        .unwrap();

    let after = cache.get_or_eval(&graph, &rows_q, 1, &budget).unwrap();
    assert_eq!(cache.stats().hits, 0, "the non-empty result was not re-served");
    assert!(after.rows.is_empty(), "CLEARed graph yields an empty result");
    assert_eq!(after.rows.len(), query(&graph, "SELECT ?s WHERE { ?s ?p ?o }").unwrap().rows.len());
    assert_eq!(cache.stats().entries, 1, "the stale v0 entry was reclaimed, not kept");
}

/// Reusing the OLD version after a mutation is the documented caller bug — and the
/// cache makes it observable (a stale HIT), which is exactly why the contract requires
/// the writer to bump. This pins that boundary so a future change that silently started
/// keying on something else would be caught by the contract test, not in production.
#[test]
fn stale_version_after_mutation_is_a_hit_contract_boundary() {
    let mut graph = g();
    let cache = ResultCache::new(16);
    let budget = QueryBudget::unlimited();
    let q = parse(COUNT_Q);

    let before = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(cached_count(&before), 3);

    graph
        .apply_delta_nquads(
            "<http://ex/d> <http://ex/p> \"4\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "",
        )
        .unwrap();

    // Caller forgot to bump: SAME version => stale hit (the documented hazard).
    let stale = cache.get_or_eval(&graph, &q, 0, &budget).unwrap();
    assert_eq!(cache.stats().hits, 1, "same-version read is a hit (no auto-detection)");
    assert_eq!(cached_count(&stale), 3, "the stale (pre-insert) count is served");
    // Bumping recovers correctness immediately.
    let fresh = cache.get_or_eval(&graph, &q, 1, &budget).unwrap();
    assert_eq!(cached_count(&fresh), 4);
}

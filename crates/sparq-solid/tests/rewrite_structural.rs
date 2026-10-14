//! sq-2xdr — behavioural tests for the query-rewrite path
//! ([`sparq_solid::rewrite_for`] / [`sparq_solid::wrap_for_view`]).
//!
//! These exercise the recursive `wrap_in_graph` / `wrap_expr` walk over the SPARQL
//! algebra (the lowest-covered module of the crate): every structural operator must
//! (a) push each default-graph triple/path pattern under its own `GRAPH ?fresh { … }`
//! scope, (b) leave patterns already inside a `GRAPH`/`SERVICE` scope alone, and
//! (c) still produce a query that round-trips through the SPARQL parser. The asserts
//! are behavioural — that the rewrite preserves authorized-graph scoping semantics —
//! not vacuous line-touchers.

use oxrdf::NamedNode;
use sparq_solid::{rewrite_for, wrap_for_view};

const G: &str = "https://pod.ex/notes/n1.ttl";

fn allowed() -> [NamedNode; 1] {
    [NamedNode::new(G).unwrap()]
}

/// Every wrapped query must remain a parseable SPARQL query.
fn assert_reparses(q: &str) {
    assert!(
        spargebra::SparqlParser::new().parse_query(q).is_ok(),
        "rewritten query must reparse:\n{q}"
    );
}

/// Count the `GRAPH ?__sgN` wraps the rewrite introduced.
fn count_wraps(q: &str) -> usize {
    q.matches("GRAPH ?__sg").count()
}

/// A UNION's two branches each get their default-graph patterns wrapped.
#[test]
fn union_wraps_both_branches() {
    let q = "SELECT * WHERE { { ?a <urn:p> ?b } UNION { ?c <urn:q> ?d } }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 2, "each UNION branch wrapped: {out}");
    assert_reparses(&out);
}

/// OPTIONAL (LeftJoin) wraps both the required and the optional side.
#[test]
fn optional_wraps_both_sides() {
    let q = "SELECT * WHERE { ?s <urn:p> ?o OPTIONAL { ?s <urn:q> ?x } }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 2, "required + optional wrapped: {out}");
    assert_reparses(&out);
}

/// OPTIONAL with a FILTER condition: the LeftJoin's `expression` branch is walked too.
#[test]
fn optional_with_filter_condition_wraps_and_reparses() {
    let q = "SELECT * WHERE { ?s <urn:p> ?o OPTIONAL { ?s <urn:q> ?x . FILTER(?x > 1) } }";
    let out = wrap_for_view(q).unwrap();
    // both triple patterns get wrapped; the LeftJoin expression has no nested EXISTS
    assert_eq!(count_wraps(&out), 2, "{out}");
    assert_reparses(&out);
}

/// MINUS wraps both operands.
#[test]
fn minus_wraps_both_operands() {
    let q = "SELECT * WHERE { ?s <urn:p> ?o MINUS { ?s <urn:bad> ?x } }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 2, "{out}");
    assert_reparses(&out);
}

/// A property-path pattern (one that stays a `Path` algebra node, e.g. a `+` closure)
/// in the default graph is wrapped as a whole `GRAPH` scope.
#[test]
fn property_path_is_wrapped() {
    let q = "SELECT * WHERE { ?s <urn:p>+ ?o }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 1, "path wrapped once: {out}");
    assert_reparses(&out);
}

/// A path pattern ALREADY inside a GRAPH scope is left untouched (in_graph short-circuit).
#[test]
fn property_path_inside_graph_is_left_alone() {
    let q = "SELECT * WHERE { GRAPH ?g { ?s <urn:p>+ ?o } }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 0, "no extra wrap inside GRAPH: {out}");
    // the user's own GRAPH var survives
    assert!(out.contains("GRAPH ?g"), "{out}");
    assert_reparses(&out);
}

/// Patterns inside an explicit GRAPH block are not re-wrapped, but sibling default-graph
/// patterns still are.
#[test]
fn explicit_graph_block_not_rewrapped_siblings_are() {
    let q = "SELECT * WHERE { ?s <urn:p> ?o . GRAPH ?g { ?s <urn:q> ?x } }";
    let out = wrap_for_view(q).unwrap();
    // exactly the one default-graph triple gets an __sg wrap; the GRAPH ?g block is intact
    assert_eq!(count_wraps(&out), 1, "{out}");
    assert!(out.contains("GRAPH ?g"), "{out}");
    assert_reparses(&out);
}

/// SERVICE blocks are treated like GRAPH scopes: their inner patterns are NOT wrapped
/// (the remote endpoint owns that scope), but the rewrite still descends without panic.
#[test]
fn service_inner_patterns_not_wrapped() {
    let q =
        "SELECT * WHERE { ?s <urn:p> ?o . SERVICE <https://remote.ex/sparql> { ?s <urn:q> ?x } }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 1, "only the local triple wrapped: {out}");
    assert!(out.contains("SERVICE"), "{out}");
    assert_reparses(&out);
}

/// FILTER NOT EXISTS { … } carries a nested pattern; the EXISTS sub-pattern is walked
/// and its default-graph triple is wrapped.
#[test]
fn filter_not_exists_nested_pattern_is_wrapped() {
    let q = "SELECT * WHERE { ?s <urn:p> ?o FILTER NOT EXISTS { ?s <urn:secret> ?z } }";
    let out = wrap_for_view(q).unwrap();
    // the main triple + the EXISTS-nested triple are both wrapped
    assert_eq!(count_wraps(&out), 2, "outer + EXISTS-nested wrapped: {out}");
    assert_reparses(&out);
}

/// A FILTER with a boolean/arithmetic expression tree (And/Or/comparisons/arith/IN/
/// COALESCE/IF/unary/function-call) must be walked without panic and reparse — even
/// though those leaves carry no nested pattern to wrap.
#[test]
fn filter_rich_expression_tree_reparses() {
    let q = "SELECT * WHERE { \
        ?s <urn:p> ?o . \
        FILTER( (?o > 1 && ?o <= 10) || (?o IN (2, 3) && !BOUND(?x)) ) \
        FILTER( IF(?o = 5, COALESCE(?x, ?o), -?o + 1) = ?o ) \
        FILTER( STRLEN(STR(?o)) > 0 ) \
    }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(
        count_wraps(&out),
        1,
        "only the BGP triple is wrapped: {out}"
    );
    assert_reparses(&out);
}

/// A sub-SELECT (Project) with DISTINCT, ORDER BY, GROUP BY, and LIMIT/OFFSET (Slice)
/// reaches the Extend/OrderBy/Project/Distinct/Group/Slice arms of the walk.
#[test]
fn projection_modifiers_chain_is_walked() {
    let q = "SELECT ?s (COUNT(?o) AS ?n) WHERE { \
        { SELECT DISTINCT ?s ?o WHERE { ?s <urn:p> ?o } ORDER BY ?o LIMIT 5 OFFSET 1 } \
    } GROUP BY ?s";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(
        count_wraps(&out),
        1,
        "the inner BGP triple is wrapped once: {out}"
    );
    assert_reparses(&out);
}

/// BIND (Extend) and VALUES (Values) arms are reached; VALUES injects no pattern to wrap.
#[test]
fn bind_and_values_arms_are_walked() {
    let q = "SELECT * WHERE { \
        VALUES ?v { 1 2 3 } \
        ?s <urn:p> ?o . \
        BIND(?o + 1 AS ?o2) \
    }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(
        count_wraps(&out),
        1,
        "only the triple wrapped, not VALUES: {out}"
    );
    assert_reparses(&out);
}

/// An empty WHERE (empty BGP) is a no-op for the wrap (the `patterns.is_empty()` guard)
/// and must still reparse.
#[test]
fn empty_bgp_is_left_alone() {
    let q = "ASK WHERE { }";
    let out = wrap_for_view(q).unwrap();
    assert_eq!(count_wraps(&out), 0, "{out}");
    assert_reparses(&out);
}

/// CONSTRUCT and DESCRIBE go through the same `wrap_query` arms as SELECT/ASK.
#[test]
fn construct_and_describe_are_wrapped() {
    let c = wrap_for_view("CONSTRUCT { ?s <urn:p> ?o } WHERE { ?s <urn:p> ?o }").unwrap();
    assert_eq!(count_wraps(&c), 1, "{c}");
    assert_reparses(&c);

    let d = wrap_for_view("DESCRIBE ?s WHERE { ?s <urn:p> ?o }").unwrap();
    assert_eq!(count_wraps(&d), 1, "{d}");
    assert_reparses(&d);
}

/// `rewrite_for` intersects a pre-existing FROM NAMED with the allowed set: a graph the
/// query names but is NOT authorized is dropped; an authorized one is kept.
#[test]
fn rewrite_for_intersects_existing_from_named() {
    let q = format!(
        "SELECT * FROM NAMED <{G}> FROM NAMED <https://pod.ex/other.ttl> WHERE {{ ?s <urn:p> ?o }}"
    );
    let out = rewrite_for(&q, &allowed()).unwrap();
    assert!(
        out.contains(&format!("FROM NAMED <{G}>")),
        "authorized graph kept: {out}"
    );
    assert!(
        !out.contains("https://pod.ex/other.ttl"),
        "unauthorized named graph dropped: {out}"
    );
    assert_reparses(&out);
}

/// `rewrite_for` with a pre-existing FROM NAMED that has NO overlap with the allowed set
/// collapses to the absent sentinel graph (fail-closed).
#[test]
fn rewrite_for_disjoint_from_named_falls_to_sentinel() {
    let q = "SELECT * FROM NAMED <https://pod.ex/other.ttl> WHERE { ?s <urn:p> ?o }";
    let out = rewrite_for(q, &allowed()).unwrap();
    assert!(out.contains("FROM NAMED <urn:sparq:nothing>"), "{out}");
    assert!(!out.contains("https://pod.ex/other.ttl"), "{out}");
    assert_reparses(&out);
}

/// A bare FROM (default-graph) clause is dropped — pod data never lives in the store
/// default graph. spargebra parses `FROM <g>` as a dataset with an EMPTY named set, so
/// after intersection there are no authorized named graphs and the rewrite fails closed
/// to the sentinel (it does NOT silently promote the default-graph URI into FROM NAMED).
#[test]
fn rewrite_for_drops_default_graph_clause() {
    let q = format!("SELECT * FROM <{G}> WHERE {{ ?s <urn:p> ?o }}");
    let out = rewrite_for(&q, &allowed()).unwrap();
    // the default-graph clause does not survive, and nothing was named, so: sentinel
    assert!(
        !out.contains(&format!("FROM <{G}>")),
        "default-graph FROM dropped: {out}"
    );
    assert!(
        out.contains("FROM NAMED <urn:sparq:nothing>"),
        "fails closed: {out}"
    );
    assert_reparses(&out);
}

/// The graph-variable prefix lengthens until it cannot collide with ANY user variable —
/// including the lengthened `?__sgx…` forms.
#[test]
fn prefix_lengthens_past_nested_collisions() {
    // user already uses ?__sg AND ?__sgx — the prefix must grow to ?__sgxx
    let q = "SELECT ?__sg ?__sgx WHERE { ?__sg <urn:p> ?__sgx . ?a <urn:q> ?b }";
    let out = wrap_for_view(q).unwrap();
    assert!(
        out.contains("GRAPH ?__sgxx"),
        "prefix grew past collisions: {out}"
    );
    assert!(!out.contains("GRAPH ?__sg0"), "{out}");
    assert!(!out.contains("GRAPH ?__sgx0"), "{out}");
    assert_reparses(&out);
}

/// An invalid query is a parse error, surfaced as `Err` (the only failure mode).
#[test]
fn invalid_query_is_an_error() {
    assert!(rewrite_for("NOT SPARQL", &allowed()).is_err());
    assert!(wrap_for_view("SELECT WHERE oops").is_err());
}

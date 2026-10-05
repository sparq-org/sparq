use super::*;
use sparq_engine_service::service::Transport;

/// Parse a query to its top-level `GraphPattern` (Select), ready for `eval_select`.
fn pattern(sparql: &str) -> spargebra::algebra::GraphPattern {
    use spargebra::SparqlParser;
    let q = SparqlParser::new().parse_query(sparql).unwrap();
    let spargebra::Query::Select { pattern, .. } = q else { panic!("not a SELECT") };
    pattern
}

struct Canned(&'static str);
impl Transport for Canned {
    fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
        Ok(self.0.to_string())
    }
}
/// Returns an owned body, so a test can build a valid-rows-then-malformed document
/// at runtime to drive the SILENT-error-mid-stream path. (sq-my8wd.4)
struct CannedOwned(String);
impl Transport for CannedOwned {
    fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
        Ok(self.0.clone())
    }
}
/// An SRJ document that streams ONE valid binding for `vars` (a long literal on the
/// last var, so interning it charges the byte budget) then a MALFORMED second
/// binding, so the streaming parser delivers row 0 to the sink and then errors —
/// the exact adversarial shape the SILENT-rollback fix must neutralise.
fn valid_then_malformed_srj(head_and_first_row: &str, big_literal: &str) -> String {
    let mut body = String::from(head_and_first_row);
    body.push_str(big_literal);
    // Close the first (valid) binding, comma, then a malformed second binding whose
    // value is an invalid JSON token (`%`), then close results.
    body.push_str(r#""}},{"o":{"type":"literal","value":%}}]}}"#);
    body
}
struct Boom;
impl Transport for Boom {
    fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
        Err("transport boom".into())
    }
}

#[test]
fn service_joins_remote_into_local_via_mock() {
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:alice a ex:Person . ex:bob a ex:Person .",
        "turtle",
    )
    .unwrap();
    let body = r#"{"head":{"vars":["s","name"]},"results":{"bindings":[
            {"s":{"type":"uri","value":"http://ex/alice"},"name":{"type":"literal","value":"Alice"}}
        ]}}"#;
    let _g = service_transport::install(Box::new(Canned(body)));
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
             { ?s a ex:Person . SERVICE <http://remote/> { ?s ex:name ?name } }",
    );
    let res = eval_select(&g, &p).unwrap();
    // Only alice has a remote name -> the join keeps one row.
    assert_eq!(res.rows.len(), 1);
}

#[test]
fn service_silent_swallows_mock_error() {
    let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a a ex:T .", "turtle").unwrap();
    let _g = service_transport::install(Box::new(Boom));
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE \
             { ?s a ex:T . SERVICE SILENT <http://remote/> { ?s ex:p ?o } }",
    );
    let res = eval_select(&g, &p).unwrap();
    assert_eq!(res.rows.len(), 1, "SILENT -> identity -> local row survives");
}

// -------------------------------------------------------------------------
// (sq-my8wd.4) SILENT-error-MID-STREAM behaviour-neutrality (Copilot
// review of PR #1424, threads on eval_service + bound_join_to_endpoint).
//
// The streaming path interns each remote row AS IT PARSES, which charges the
// `LocalVocab` byte budget (`intern` -> `budget::add_bytes`). The pre-streaming
// collect-then-intern-on-success path interned NOTHING on a swallowed (SILENT)
// error, so it charged 0 bytes. Without the rollback these tests pin, a SILENT
// SERVICE whose response streams a row then errors would RETAIN that row's intern
// and its byte charge — tripping a `max_bytes` budget the SILENT fallback (join
// identity) otherwise fits, turning a query the old path ANSWERED into a
// "query budget exceeded" error. That is an observable, non-neutral result
// difference — hence a fix, not just documentation. The budget is deliberately
// tight so the single discarded remote row's charge alone would trip it.
// -------------------------------------------------------------------------

#[test]
fn service_silent_midstream_rollback_verbatim_neutral_under_byte_budget() {
    let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a a ex:T .", "turtle").unwrap();
    let big = "A".repeat(120);
    let body = valid_then_malformed_srj(
        r#"{"head":{"vars":["o"]},"results":{"bindings":[{"o":{"type":"literal","value":""#,
        &big,
    );
    let _t = service_transport::install(Box::new(CannedOwned(body)));
    // Standalone SERVICE (no left bindings to push) -> verbatim `eval_service`.
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?o WHERE \
             { SERVICE SILENT <http://remote/> { ?o ex:p ?x } }",
    );
    // A cap the ONE discarded remote literal (interned twice) blows, but the SILENT
    // fallback's own working set (one identity row) fits well under.
    let b = crate::QueryBudget { max_bytes: Some(64), ..crate::QueryBudget::unlimited() };
    budget::with_budget(&b, || {
        let res = eval_select(&g, &p)
            .expect("SILENT mid-stream error must roll back the discarded row's byte charge");
        assert_eq!(res.rows.len(), 1, "SILENT verbatim -> identity -> one (unbound ?o) row");
    })
}

#[test]
fn service_silent_midstream_rollback_boundjoin_neutral_under_byte_budget() {
    let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a a ex:T .", "turtle").unwrap();
    let big = "A".repeat(120);
    let body = valid_then_malformed_srj(
        r#"{"head":{"vars":["s","o"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://ex/a"},"o":{"type":"literal","value":""#,
        &big,
    );
    let _t = service_transport::install(Box::new(CannedOwned(body)));
    // `?s` is bound by the left BGP and shared with the SERVICE -> bind-join pushdown,
    // so this exercises the `bound_join_to_endpoint` SILENT arm.
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE \
             { ?s a ex:T . SERVICE SILENT <http://remote/> { ?s ex:p ?o } }",
    );
    let b = crate::QueryBudget { max_bytes: Some(64), ..crate::QueryBudget::unlimited() };
    budget::with_budget(&b, || {
        let res = eval_select(&g, &p)
            .expect("bind-join SILENT mid-stream error must roll back the discarded row's byte charge");
        assert_eq!(res.rows.len(), 1, "SILENT block failure -> identity -> local ?s=ex:a row survives");
    })
}

#[test]
fn service_nonsilent_propagates_mock_error() {
    let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a a ex:T .", "turtle").unwrap();
    let _g = service_transport::install(Box::new(Boom));
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE \
             { ?s a ex:T . SERVICE <http://remote/> { ?s ex:p ?o } }",
    );
    assert!(eval_select(&g, &p).is_err());
}

// ---------------------------------------------------------------------
// Bind-join (VALUES pushdown) differential: bound == verbatim.
// (bead sq-sjkj). A mock transport that evaluates the RECEIVED query (VALUES
// and all) against a real remote `Graph`, so the injected pushdown is
// genuinely executed and we can assert the bound path's results are IDENTICAL
// to the verbatim path's — plus count remote requests.
// ---------------------------------------------------------------------

use std::cell::RefCell;
use std::rc::Rc;

/// A transport backed by a remote `Graph`: it runs each received query through
/// the engine (`query_json`) and records the query strings it served.
struct RemoteGraph {
    remote: Graph,
    seen: Rc<RefCell<Vec<String>>>,
}
impl Transport for RemoteGraph {
    fn fetch(&self, _e: &str, q: &str) -> Result<String, String> {
        self.seen.borrow_mut().push(q.to_string());
        crate::query_json(&self.remote, q)
    }
}

/// Sort a result's rows into a canonical multiset for order-independent equality.
fn canon(res: &QueryResult) -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = res
        .rows
        .iter()
        .map(|r| r.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect();
    rows.sort();
    rows
}

fn three_persons() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> . \
             ex:alice a ex:Person . ex:bob a ex:Person . ex:carol a ex:Person .",
        "turtle",
    )
    .unwrap()
}

fn remote_names() -> Graph {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    ttl.push_str("ex:alice ex:name \"Alice\" . ex:bob ex:name \"Bob\" .\n");
    for i in 0..60 {
        ttl.push_str(&format!("ex:noise{i} ex:name \"N{i}\" .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

/// Run a federated query with a remote-graph mock, returning (result, n_requests).
fn run_fed(local: &Graph, q: &str, remote: Graph) -> (QueryResult, usize) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteGraph { remote, seen: Rc::clone(&seen) }));
    let p = pattern(q);
    let res = eval_select(local, &p).unwrap();
    let n = seen.borrow().len();
    (res, n)
}

#[test]
fn bound_join_equals_verbatim_basic() {
    // Same query, same mock: the bound-join path (default, ON) must match the
    // verbatim path (forced by a single-tuple block can't disable it, so we
    // compare against an explicit local-join reconstruction via VALUES-free mock).
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s a ex:Person . SERVICE <http://r/> { ?s ex:name ?name } }";
    let (bound, n) = run_fed(&three_persons(), q, remote_names());
    // alice + bob match; carol + 60 noise do not contribute to the join.
    assert_eq!(bound.rows.len(), 2, "only alice/bob have remote names");
    assert_eq!(n, 1, "3 distinct subjects < block 50 => one request");
    let got = canon(&bound);
    // The verbatim equivalent: no SERVICE, just the remote relation joined.
    // Reconstruct it locally to pin the expected multiset.
    let expect: Vec<Vec<Option<String>>> = {
        let mut v = vec![
            vec![Some("<http://ex/alice>".to_string()), Some("\"Alice\"".to_string())],
            vec![Some("<http://ex/bob>".to_string()), Some("\"Bob\"".to_string())],
        ];
        v.sort();
        v
    };
    assert_eq!(got, expect, "bound-join result multiset");
}

#[test]
fn bound_join_block_boundary_unions_all_blocks() {
    // Block size 1 => one request per distinct subject; the union must still be
    // the SAME result as a single block. Proves block-boundary correctness.
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteGraph {
        remote: remote_names(),
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s a ex:Person . SERVICE <http://r/> { ?s ex:name ?name } }";
    let res = sparq_engine_service::service::with_service_bound_join_block_size(1, || {
        eval_select(&three_persons(), &pattern(q)).unwrap()
    });
    assert_eq!(res.rows.len(), 2);
    assert_eq!(seen.borrow().len(), 3, "block size 1 over 3 subjects => 3 requests");
}

#[test]
fn bound_join_optional_keeps_unmatched() {
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s a ex:Person . OPTIONAL { SERVICE <http://r/> { ?s ex:name ?name } } }";
    let (res, _n) = run_fed(&three_persons(), q, remote_names());
    assert_eq!(res.rows.len(), 3, "left-join: carol survives with an unbound name");
    let name_i = res.vars.iter().position(|v| v.as_str() == "name").unwrap();
    let bound_names = res.rows.iter().filter(|r| r[name_i].is_some()).count();
    assert_eq!(bound_names, 2, "only alice/bob got a name");
}

#[test]
fn bound_join_silent_block_failure_keeps_left() {
    // A transport that fails => under SILENT, the bound-join degrades to the join
    // identity (empty remote relation), so the local rows survive unchanged —
    // identical to the verbatim SILENT path.
    let _g = service_transport::install(Box::new(Boom));
    let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE \
                 { ?s a ex:Person . SERVICE SILENT <http://r/> { ?s ex:name ?name } }";
    let res = eval_select(&three_persons(), &pattern(q)).unwrap();
    assert_eq!(res.rows.len(), 3, "SILENT remote failure keeps all local persons");
}

#[test]
fn bound_join_unbound_join_var_falls_back_to_verbatim() {
    // Here ?s is NOT bound on the left of the SERVICE (the left binds only ?p),
    // so there is no bound join var to push — the verbatim path runs. The remote
    // returns its whole relation, joined locally. Still correct.
    let local = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:alice ex:knows ex:bob .",
        "turtle",
    )
    .unwrap();
    let remote = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:bob ex:name \"Bob\" . ex:zzz ex:name \"Z\" .",
        "turtle",
    )
    .unwrap();
    // The SERVICE binds ?o/?name; the left binds ?s/?o (via ex:knows). ?o is shared
    // and bound => this actually DOES bind-join on ?o. Use a genuinely unshared var.
    let q = "PREFIX ex: <http://ex/> SELECT ?name WHERE \
                 { ex:alice ex:knows ?x . SERVICE <http://r/> { ?y ex:name ?name } }";
    let (res, _n) = run_fed(&local, q, remote);
    // No shared var => cross product of {?x=bob} with remote {bob,zzz} => 2 rows.
    assert_eq!(res.rows.len(), 2, "no bound join var => verbatim cross-join");
}

#[test]
fn variable_endpoint_unbound_top_level_errors() {
    // A variable endpoint with NOTHING to bind it (the surrounding pattern produces
    // no row binding `?e`) has no endpoint to call: the verbatim path errors
    // (non-SILENT) / yields the identity (SILENT). (sq-d4p)
    let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a a ex:T .", "turtle").unwrap();
    // No `ex:ep` data => `?s ex:ep ?e` yields 0 rows => empty left => verbatim path.
    let p = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE \
             { ?s a ex:T . ?s ex:ep ?e . SERVICE ?e { ?s ex:p ?o } }",
    );
    let err = eval_select(&g, &p).unwrap_err();
    assert!(err.contains("variable endpoint"), "got: {err}");

    // SILENT variable endpoint -> identity, no error.
    let p2 = pattern(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE \
             { ?s a ex:T . ?s ex:ep ?e . SERVICE SILENT ?e { ?s ex:p ?o } }",
    );
    assert!(eval_select(&g, &p2).is_ok());
}

// ---------------------------------------------------------------------
// SERVICE ?var — variable endpoint, per-endpoint dispatch. (sq-d4p)
// ---------------------------------------------------------------------

/// A transport that routes each query to a distinct remote `Graph` keyed by the
/// endpoint IRI, recording the (endpoint, query) pairs it served. Lets a test prove
/// that `SERVICE ?ep` dials the RIGHT endpoint per left binding of `?ep`.
struct RemoteByEndpoint {
    graphs: std::collections::HashMap<String, Graph>,
    seen: Rc<RefCell<Vec<(String, String)>>>,
}
impl Transport for RemoteByEndpoint {
    fn fetch(&self, e: &str, q: &str) -> Result<String, String> {
        self.seen.borrow_mut().push((e.to_string(), q.to_string()));
        match self.graphs.get(e) {
            Some(g) => crate::query_json(g, q),
            None => Err(format!("no such endpoint {e}")),
        }
    }
}

/// `local` data binds each person to the endpoint that knows their name.
fn persons_with_endpoints() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> . \
             ex:alice ex:ep <http://r1/> . \
             ex:bob   ex:ep <http://r2/> . \
             ex:carol ex:ep <http://r1/> .",
        "turtle",
    )
    .unwrap()
}

#[test]
fn variable_endpoint_dispatches_per_endpoint() {
    // alice & carol -> r1 ; bob -> r2. Each remote knows only its own names.
    let r1 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:alice ex:name \"Alice\" . ex:carol ex:name \"Carol\" .",
        "turtle",
    )
    .unwrap();
    let r2 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:bob ex:name \"Bob\" .",
        "turtle",
    )
    .unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut graphs = std::collections::HashMap::new();
    graphs.insert("http://r1/".to_string(), r1);
    graphs.insert("http://r2/".to_string(), r2);
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs,
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE ?e { ?s ex:name ?name } }";
    let res = eval_select(&persons_with_endpoints(), &pattern(q)).unwrap();
    let mut got = canon(&res);
    got.sort();
    let mut expect = vec![
        vec![Some("<http://ex/alice>".to_string()), Some("\"Alice\"".to_string())],
        vec![Some("<http://ex/bob>".to_string()), Some("\"Bob\"".to_string())],
        vec![Some("<http://ex/carol>".to_string()), Some("\"Carol\"".to_string())],
    ];
    expect.sort();
    assert_eq!(got, expect, "each person resolved against its own endpoint");
    // Two distinct endpoints => exactly two remote requests (one bind-join each).
    let endpoints: std::collections::HashSet<String> =
        seen.borrow().iter().map(|(e, _)| e.clone()).collect();
    assert_eq!(endpoints.len(), 2, "dialled both r1 and r2, and only those");
}

#[test]
fn variable_endpoint_optional_keeps_unmatched() {
    // carol's endpoint r1 does NOT know her name -> under OPTIONAL she survives
    // unbound; alice (r1) and bob (r2) get names.
    let r1 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:alice ex:name \"Alice\" .",
        "turtle",
    )
    .unwrap();
    let r2 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:bob ex:name \"Bob\" .",
        "turtle",
    )
    .unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut graphs = std::collections::HashMap::new();
    graphs.insert("http://r1/".to_string(), r1);
    graphs.insert("http://r2/".to_string(), r2);
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs,
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . OPTIONAL { SERVICE ?e { ?s ex:name ?name } } }";
    let res = eval_select(&persons_with_endpoints(), &pattern(q)).unwrap();
    assert_eq!(res.rows.len(), 3, "all three persons survive the left join");
    let name_i = res.vars.iter().position(|v| v.as_str() == "name").unwrap();
    let named = res.rows.iter().filter(|r| r[name_i].is_some()).count();
    assert_eq!(named, 2, "only alice/bob got a name; carol's endpoint lacked it");
}

#[test]
fn variable_endpoint_silent_failure_keeps_left() {
    // A failing transport under SILENT must keep the left rows unchanged (identity),
    // exactly like the concrete-endpoint SILENT path.
    let _g = service_transport::install(Box::new(Boom));
    let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE \
                 { ?s ex:ep ?e . SERVICE SILENT ?e { ?s ex:name ?name } }";
    let res = eval_select(&persons_with_endpoints(), &pattern(q)).unwrap();
    assert_eq!(res.rows.len(), 3, "SILENT remote failure keeps all three persons");
}

#[test]
fn variable_endpoint_non_iri_binding_yields_no_remote_row() {
    // A literal-valued `?e` is not a valid endpoint -> that solution contributes no
    // remote row (inner join drops it). alice (valid r1) survives; dave (literal) does not.
    let local = Graph::load_str(
        "@prefix ex: <http://ex/> . \
             ex:alice ex:ep <http://r1/> . \
             ex:dave  ex:ep \"not-an-iri\" .",
        "turtle",
    )
    .unwrap();
    let r1 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:alice ex:name \"Alice\" .",
        "turtle",
    )
    .unwrap();
    let mut graphs = std::collections::HashMap::new();
    graphs.insert("http://r1/".to_string(), r1);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs,
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE ?e { ?s ex:name ?name } }";
    let res = eval_select(&local, &pattern(q)).unwrap();
    assert_eq!(res.rows.len(), 1, "only the valid-IRI endpoint produced a row");
    // The literal endpoint was never dialled.
    assert!(
        seen.borrow().iter().all(|(e, _)| e == "http://r1/"),
        "only the valid IRI endpoint was contacted"
    );
}

// ---------------------------------------------------------------------
// Per-query remote-request cap for high-cardinality SERVICE ?ep.
// (sq-b93pv). The cap bounds the number of DISTINCT endpoints a single
// `SERVICE ?ep` may dial, enforced PRE-HTTP: when the cap fires, the mock
// transport must record ZERO requests (proof the bound is before dispatch, not
// a post-HTTP cancellation).
// ---------------------------------------------------------------------

/// `local` data binding `n` distinct persons each to a DISTINCT endpoint IRI
/// (`http://r0/`, `http://r1/`, …) — i.e. a high-cardinality `?ep`.
fn persons_with_distinct_endpoints(n: usize) -> Graph {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..n {
        ttl.push_str(&format!("ex:p{i} ex:ep <http://r{i}/> .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

#[test]
fn remote_request_cap_fires_before_dispatch() {
    // 5 distinct endpoints, cap 3 => the query is REFUSED, and the mock records
    // NO requests (the bound is enforced pre-HTTP, before any socket is opened).
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs: std::collections::HashMap::new(),
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE ?e { ?s ex:name ?name } }";
    let p = pattern(q);
    let err = sparq_engine_service::service::with_service_remote_request_cap(3, || {
        eval_select(&persons_with_distinct_endpoints(5), &p)
    })
    .unwrap_err();
    // Typed refusal: carries the stable marker the server classifies on.
    assert!(
        err.contains(sparq_engine_service::service::SERVICE_REMOTE_CAP_MARKER),
        "cap error must carry the marker, got: {err}"
    );
    // PRE-DISPATCH: not one remote request was made.
    assert!(
        seen.borrow().is_empty(),
        "cap must fire BEFORE any HTTP dispatch; mock saw: {:?}",
        seen.borrow()
    );
}

#[test]
fn remote_request_cap_not_swallowed_by_silent() {
    // The cap is a resource-policy refusal, not a remote failure: SERVICE SILENT
    // must NOT mask it (SILENT only masks an endpoint being unreachable).
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs: std::collections::HashMap::new(),
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE SILENT ?e { ?s ex:name ?name } }";
    let p = pattern(q);
    let err = sparq_engine_service::service::with_service_remote_request_cap(2, || {
        eval_select(&persons_with_distinct_endpoints(5), &p)
    })
    .unwrap_err();
    assert!(
        err.contains(sparq_engine_service::service::SERVICE_REMOTE_CAP_MARKER),
        "SILENT must not swallow the cap refusal, got: {err}"
    );
    assert!(seen.borrow().is_empty(), "cap fires before dispatch even under SILENT");
}

#[test]
fn remote_request_cap_at_or_under_limit_passes() {
    // 2 distinct endpoints, cap 2 (the boundary: > cap fires, == cap is allowed).
    // Each endpoint knows its own person's name; the query must succeed normally.
    let r0 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:p0 ex:name \"P0\" .",
        "turtle",
    )
    .unwrap();
    let r1 = Graph::load_str(
        "@prefix ex: <http://ex/> . ex:p1 ex:name \"P1\" .",
        "turtle",
    )
    .unwrap();
    let mut graphs = std::collections::HashMap::new();
    graphs.insert("http://r0/".to_string(), r0);
    graphs.insert("http://r1/".to_string(), r1);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs,
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE ?e { ?s ex:name ?name } }";
    let p = pattern(q);
    let res = sparq_engine_service::service::with_service_remote_request_cap(2, || {
        eval_select(&persons_with_distinct_endpoints(2), &p)
    })
    .expect("at-cap query is allowed");
    assert_eq!(res.rows.len(), 2, "both persons resolved against their own endpoint");
    // Exactly the two distinct endpoints were dialled.
    let dialled: std::collections::HashSet<String> =
        seen.borrow().iter().map(|(e, _)| e.clone()).collect();
    assert_eq!(dialled.len(), 2, "dialled exactly the two permitted endpoints");
}

#[test]
fn remote_request_cap_default_off_is_unchanged() {
    // DEFAULT (no cap installed): a high-cardinality SERVICE ?ep dispatches to ALL
    // distinct endpoints exactly as before — the cap adds nothing to normal queries.
    let mut graphs = std::collections::HashMap::new();
    for i in 0..5 {
        graphs.insert(
            format!("http://r{i}/"),
            Graph::load_str(
                &format!("@prefix ex: <http://ex/> . ex:p{i} ex:name \"P{i}\" ."),
                "turtle",
            )
            .unwrap(),
        );
    }
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteByEndpoint {
        graphs,
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s ex:ep ?e . SERVICE ?e { ?s ex:name ?name } }";
    // No `with_service_remote_request_cap` scope => uncapped (the default).
    let res = eval_select(&persons_with_distinct_endpoints(5), &pattern(q)).unwrap();
    assert_eq!(res.rows.len(), 5, "all five persons resolved (no cap by default)");
    let dialled: std::collections::HashSet<String> =
        seen.borrow().iter().map(|(e, _)| e.clone()).collect();
    assert_eq!(dialled.len(), 5, "all five distinct endpoints dialled by default");
}

#[test]
fn remote_request_cap_does_not_bound_concrete_endpoint() {
    // A concrete-IRI SERVICE is a SINGLE endpoint; even cap 0 (refuse any variable
    // dispatch) must not touch it — the cap is scoped to the variable-endpoint
    // fan-out, not the per-block request count of a single endpoint.
    let g = three_persons();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _g = service_transport::install(Box::new(RemoteGraph {
        remote: remote_names(),
        seen: Rc::clone(&seen),
    }));
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE \
                 { ?s a ex:Person . SERVICE <http://r/> { ?s ex:name ?name } }";
    let res = sparq_engine_service::service::with_service_remote_request_cap(0, || {
        eval_select(&g, &pattern(q))
    })
    .expect("concrete-IRI SERVICE is unaffected by the variable-endpoint cap");
    assert_eq!(res.rows.len(), 2, "alice + bob matched, cap did not interfere");
}

// ---------------------------------------------------------------------
// Per-query timeout wired to the QueryBudget deadline. (sq-d4p)
// (The HttpTransport timeout-math test `http_transport_timeout_tracks_budget`
// MOVED with the transport to `sparq-engine-service`, seam A2 / sq-6vshe.4 — its
// `timeout_for_test` accessor is `#[cfg(test)]` there. The caller side —
// `budget::remaining_timeout` feeding `with_budget` — stays tested below.)
// ---------------------------------------------------------------------

#[test]
fn budget_remaining_timeout_reflects_deadline() {
    use std::time::{Duration, Instant};
    // No budget installed -> None.
    assert!(budget::remaining_timeout().is_none());
    // A future deadline -> Some(positive), bounded by the deadline.
    let b = crate::QueryBudget {
        deadline: Some(Instant::now() + Duration::from_secs(10)),
        ..crate::QueryBudget::unlimited()
    };
    budget::with_budget(&b, || {
        let r = budget::remaining_timeout().expect("deadline installed");
        assert!(
            r <= Duration::from_secs(10) && r > Duration::from_secs(8),
            "got {r:?}"
        );
        // An expired deadline saturates to ZERO (never panics / underflows).
        let b2 = crate::QueryBudget {
            deadline: Some(Instant::now() - Duration::from_millis(1)),
            ..crate::QueryBudget::unlimited()
        };
        budget::with_budget(&b2, || {
            assert_eq!(budget::remaining_timeout(), Some(Duration::ZERO));
        })
    })
}

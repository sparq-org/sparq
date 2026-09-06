//! LOCAL rows-returning `SERVICE` handlers — the table-function seam.
//! [OPUS-5] (sq-lsp7k.2.2)
//!
//! A toy in-process handler stands in for the "extension that returns a RELATION"
//! case (`sparq-shacl`'s conforming-focus-nodes lookup is the motivating consumer;
//! this crate deliberately has NO dependency on it). The tests pin: the returned rows
//! JOIN with the surrounding query, the request carries the group's variables and
//! pre-bound terms, `SILENT` degrades exactly as the remote path does, a bad relation
//! is reported rather than silently mangled, the registry is scoped to the install,
//! and it survives re-entry on a rayon worker (`FILTER EXISTS`).
//!
//! Built only when the `service-local` feature is on:
//!   cargo test -p sparq-engine --features service-local --test federation_features
//!   cargo test -p sparq-engine --features service,service-local --test federation_features

#![cfg(feature = "service-local")]

use oxrdf::{Literal, NamedNode, Term, Variable};
use sparq_core::Graph;
use sparq_engine::{
    query, with_local_services, LocalServicePattern, LocalServiceRegistry, LocalServiceRequest,
    LocalServiceRows, LocalServiceSlot,
};

/// The handler IRI used throughout. A `urn:` so it could never be dialled anyway.
const HANDLER: &str = "urn:sparq:test:local-service";

fn var(n: &str) -> Variable {
    Variable::new(n).expect("valid variable name")
}

fn iri(s: &str) -> Term {
    Term::from(NamedNode::new(s).expect("valid IRI"))
}

fn lit(s: &str) -> Term {
    Term::from(Literal::new_simple_literal(s))
}

/// Two people locally; the handler supplies their names.
fn local_graph() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
         ex:alice a ex:Person .\n\
         ex:bob   a ex:Person .\n",
        "turtle",
    )
    .unwrap()
}

/// A handler that returns a fixed `?s ?name` relation, ignoring the request.
fn names_registry() -> LocalServiceRegistry {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(
            vec![var("s"), var("name")],
            vec![
                vec![Some(iri("http://ex/alice")), Some(lit("Alice"))],
                vec![Some(iri("http://ex/bob")), Some(lit("Bob"))],
                // A row for someone the local graph does not know — the join drops it.
                vec![Some(iri("http://ex/carol")), Some(lit("Carol"))],
            ],
        ))
    });
    reg
}

fn column(res: &sparq_engine::QueryResult, name: &str) -> Vec<String> {
    let i = res.vars.iter().position(|v| v.as_str() == name).expect("projected variable");
    res.rows.iter().filter_map(|r| r[i].as_ref().map(|t| t.to_string())).collect()
}

#[test]
fn handler_rows_join_with_the_surrounding_query() {
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let res = with_local_services(&names_registry(), || query(&g, &q)).expect("local SERVICE");
    let names = column(&res, "name");
    assert_eq!(names.len(), 2, "carol has no local ex:Person triple, so she is joined away: {:?}", names);
    assert!(names.iter().any(|n| n.contains("Alice")), "{:?}", names);
    assert!(names.iter().any(|n| n.contains("Bob")), "{:?}", names);
}

#[test]
fn unbound_cell_is_a_genuinely_unbound_solution() {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(
            vec![var("s"), var("nick")],
            vec![
                vec![Some(iri("http://ex/alice")), Some(lit("Ali"))],
                vec![Some(iri("http://ex/bob")), None],
            ],
        ))
    });
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?nick WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:nick ?nick }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &q)).expect("local SERVICE");
    assert_eq!(res.rows.len(), 2, "both people match; bob's ?nick is simply unbound");
    assert_eq!(column(&res, "nick"), vec!["\"Ali\""], "only alice has a bound nick");
}

#[test]
fn request_exposes_the_groups_vars_query_and_prebound_terms() {
    // The handler ASSERTS on its request and answers from the pre-bound constant — the
    // table-function argument channel.
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |req: &LocalServiceRequest<'_>| {
        assert_eq!(req.service(), HANDLER);
        assert!(req.query().starts_with("SELECT * WHERE {"), "{}", req.query());
        assert!(req.query().contains("http://ex/name"), "{}", req.query());
        let vars: Vec<&str> = req.vars().iter().map(Variable::as_str).collect();
        assert_eq!(vars, vec!["s", "name"], "in-scope vars in first-occurrence order");
        // Flat BGP => a full decomposition, with the predicate as a pre-bound argument.
        let pats: &[LocalServicePattern] = req.patterns();
        assert_eq!(pats.len(), 1);
        assert_eq!(pats[0].subject, LocalServiceSlot::Var(var("s")));
        assert_eq!(
            pats[0].predicate,
            LocalServiceSlot::Term(iri("http://ex/name")),
            "the constant slots ARE the pre-bound terms"
        );
        assert_eq!(pats[0].object, LocalServiceSlot::Var(var("name")));
        Ok(LocalServiceRows::new(
            vec![var("s"), var("name")],
            vec![vec![Some(iri("http://ex/alice")), Some(lit("Alice"))]],
        ))
    });
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &q)).expect("local SERVICE");
    assert_eq!(res.rows.len(), 1);
}

#[test]
fn non_bgp_group_decomposes_to_no_patterns() {
    // All-or-nothing: a group the engine will not flatten hands the handler an EMPTY
    // pattern list, never a partial view. `query()` still carries the whole group.
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |req: &LocalServiceRequest<'_>| {
        assert!(req.patterns().is_empty(), "a UNION group must not decompose");
        assert!(req.query().contains("UNION"), "{}", req.query());
        Ok(LocalServiceRows::empty(vec![var("s")]))
    });
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ SERVICE <{}> {{ {{ ?s ex:a ?o }} UNION {{ ?s ex:b ?o }} }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &q)).expect("local SERVICE");
    assert!(res.rows.is_empty(), "the handler returned the empty relation");
}

#[test]
fn empty_relation_makes_the_surrounding_join_empty() {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| Ok(LocalServiceRows::empty(vec![var("name")])));
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &q)).expect("local SERVICE");
    assert!(res.rows.is_empty(), "an empty relation is NOT the join identity");
}

#[test]
fn handler_error_fails_the_query_and_silent_keeps_the_bindings() {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| Err("handler exploded".to_string()));
    let g = local_graph();

    let hard = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let err = with_local_services(&reg, || query(&g, &hard)).expect_err("must propagate");
    assert!(err.contains("handler exploded"), "{}", err);

    let silent = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE SILENT <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &silent)).expect("SILENT must not fail");
    assert_eq!(res.rows.len(), 2, "SILENT yields the join identity, keeping both people");
}

#[test]
fn a_ragged_relation_is_reported_not_silently_mangled() {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(vec![var("s"), var("name")], vec![vec![Some(lit("only one cell"))]]))
    });
    let g = local_graph();
    let q = format!("SELECT ?s WHERE {{ SERVICE <{}> {{ ?s ?p ?name }} }}", HANDLER);
    let err = with_local_services(&reg, || query(&g, &q)).expect_err("arity violation");
    assert!(err.contains("invalid relation"), "{}", err);
    assert!(err.contains(HANDLER), "the message names the offending handler: {}", err);
}

#[test]
fn a_duplicate_header_variable_is_reported() {
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(vec![var("s"), var("s")], Vec::new()))
    });
    let g = local_graph();
    let q = format!("SELECT ?s WHERE {{ SERVICE <{}> {{ ?s ?p ?o }} }}", HANDLER);
    let err = with_local_services(&reg, || query(&g, &q)).expect_err("duplicate header var");
    assert!(err.contains("duplicate variable ?s"), "{}", err);
}

#[test]
fn a_header_variable_outside_the_group_is_reported() {
    // The relation must be one the SERVICE group itself could have produced. A column
    // the group does not put in scope would join against the identically-named variable
    // of the SURROUNDING query — here it would silently drop the outer `?x = 1` row.
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(
            vec![var("o"), var("x")],
            vec![vec![Some(lit("anything")), Some(Term::from(Literal::from(2)))]],
        ))
    });
    let g = local_graph();
    let q = format!(
        "SELECT ?x ?o WHERE {{ VALUES ?x {{ 1 }} SERVICE <{}> {{ ?s ?p ?o }} }}",
        HANDLER
    );
    let err = with_local_services(&reg, || query(&g, &q)).expect_err("?x is outside the group");
    assert!(err.contains("invalid relation"), "{}", err);
    assert!(err.contains("?x is not in scope"), "{}", err);

    // ...and the outer solution is untouched under SILENT, where the failure degrades to
    // the join identity exactly as a remote failure does.
    let silent = format!(
        "SELECT ?x WHERE {{ VALUES ?x {{ 1 }} SERVICE SILENT <{}> {{ ?s ?p ?o }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &silent)).expect("SILENT must not fail");
    let xs = column(&res, "x");
    assert_eq!(xs.len(), 1, "the outer ?x = 1 row survives: {:?}", xs);
    assert!(xs[0].starts_with("\"1\""), "and still binds 1, not the handler's 2: {}", xs[0]);
}

#[test]
fn a_subset_of_the_groups_vars_is_accepted() {
    // The other side of the scope check: returning FEWER columns than the group has in
    // scope is fine — those variables are simply unbound in the handler's solutions.
    let mut reg = LocalServiceRegistry::new();
    reg.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(vec![var("s")], vec![vec![Some(iri("http://ex/alice"))]]))
    });
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let res = with_local_services(&reg, || query(&g, &q)).expect("a subset header is valid");
    assert_eq!(column(&res, "s"), vec!["<http://ex/alice>"]);
}

#[test]
fn a_nested_install_restores_the_outer_registry() {
    // `with_local_services` is scoped RAII: an inner install shadows the outer registry
    // only for the inner closure. Clearing instead of restoring would leave the outer
    // scope's IRI unregistered — silently unsupported, or dialled over HTTP.
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    // The inner registry answers the SAME IRI with a different relation, so "which
    // registry served this?" is observable from the answer, not just from success.
    let mut inner = LocalServiceRegistry::new();
    inner.register(HANDLER, |_req| {
        Ok(LocalServiceRows::new(
            vec![var("s"), var("name")],
            vec![vec![Some(iri("http://ex/alice")), Some(lit("INNER"))]],
        ))
    });

    with_local_services(&names_registry(), || {
        assert_eq!(column(&query(&g, &q).expect("outer"), "name").len(), 2, "before");
        let mid = with_local_services(&inner, || query(&g, &q).expect("inner"));
        assert_eq!(column(&mid, "name"), vec!["\"INNER\""], "the inner registry shadows");
        let after = query(&g, &q).expect("the outer registry must be restored");
        assert_eq!(column(&after, "name").len(), 2, "after");
    });

    // ...and the OUTER install still uninstalls at ITS end.
    query(&g, &q).expect_err("no registry is installed here");
}

#[test]
fn a_panicking_inner_install_restores_the_outer_registry() {
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    with_local_services(&names_registry(), || {
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_local_services(&LocalServiceRegistry::new(), || panic!("inner scope blew up"));
        }));
        assert!(unwound.is_err(), "the inner closure must have panicked");
        assert!(query(&g, &q).is_ok(), "unwinding the inner scope must not unregister the outer");
    });
}

#[test]
fn the_registry_is_scoped_to_the_install() {
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{}> {{ ?s ex:name ?name }} }}",
        HANDLER
    );
    let reg = names_registry();
    assert!(with_local_services(&reg, || query(&g, &q)).is_ok(), "installed");
    // Outside the install the SERVICE is NOT locally handled. With the `service`
    // feature also on it falls through to the HTTP path (which refuses a `urn:` /
    // non-allowlisted endpoint); with `service` off it is an unsupported pattern.
    // Either way: an error, never a silently-served answer.
    query(&g, &q).expect_err("the handler must not outlive its install");
}

#[test]
fn handler_is_visible_under_filter_exists() {
    // FILTER EXISTS re-enters pattern evaluation, potentially on a rayon worker. The
    // registry must be propagated there, or the SERVICE would miss the handler and try
    // to dial the IRI instead.
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . \
          FILTER EXISTS {{ SERVICE <{}> {{ ?s ex:name ?name }} }} }}",
        HANDLER
    );
    let res = with_local_services(&names_registry(), || query(&g, &q)).expect("local SERVICE");
    assert_eq!(res.rows.len(), 2, "both people have a handler-supplied name");
}

#[test]
fn a_variable_endpoint_is_never_dispatched_locally() {
    // Documented v1 scope-out: only a CONCRETE `SERVICE <iri>` is intercepted.
    let g = local_graph();
    let q = format!(
        "SELECT ?ep WHERE {{ VALUES ?ep {{ <{}> }} SERVICE ?ep {{ ?s ?p ?o }} }}",
        HANDLER
    );
    with_local_services(&names_registry(), || query(&g, &q))
        .expect_err("SERVICE ?ep keeps its existing (unsupported / egress-refused) behaviour");
}

/// With BOTH features on, an IRI that misses the registry keeps the unchanged HTTP
/// path — including its default-deny SSRF egress refusal. The local registry can only
/// ever REMOVE outbound reach, never grant it.
#[cfg(feature = "service")]
#[test]
fn an_unregistered_iri_still_faces_the_egress_filter() {
    let g = local_graph();
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT ?s WHERE { ?s a ex:Person . \
             SERVICE <http://127.0.0.1:1/> { ?s ex:name ?name } }";
    let err = with_local_services(&names_registry(), || query(&g, q))
        .expect_err("loopback is not allowlisted");
    assert!(
        err.contains(sparq_engine::SERVICE_EGRESS_REFUSED_MARKER),
        "a registry MISS must still be judged by the egress policy: {}",
        err
    );
}

/// With `service-local` alone, an unregistered SERVICE IRI is the same "unsupported"
/// outcome the fully-feature-off build gives it — no HTTP stack is compiled at all.
#[cfg(not(feature = "service"))]
#[test]
fn without_the_service_feature_an_unregistered_iri_is_unsupported() {
    let g = local_graph();
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT ?s WHERE { ?s a ex:Person . \
             SERVICE <http://example.invalid/sparql> { ?s ex:name ?name } }";
    let err = with_local_services(&names_registry(), || query(&g, q)).expect_err("no handler");
    assert!(err.contains("unsupported graph pattern"), "{}", err);

    let silent = "PREFIX ex: <http://ex/>\n\
                  SELECT ?s WHERE { ?s a ex:Person . \
                  SERVICE SILENT <http://example.invalid/sparql> { ?s ex:name ?name } }";
    let res = with_local_services(&names_registry(), || query(&g, silent))
        .expect("SILENT degrades to the join identity");
    assert_eq!(res.rows.len(), 2);
}

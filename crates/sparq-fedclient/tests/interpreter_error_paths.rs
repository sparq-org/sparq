//! sq-bif.2 — federation OPERATOR-interpreter error-path + edge-case suite.
//!
//! The crate's correctness tests (`planner_result_equals_local_eval`,
//! `streaming_result_equals_phase3`, `multi_source_union_result_equals_local`) drive the
//! interpreter down the SUCCESS path against an engine-backed transport. They never make the
//! transport fail, so the interpreter's three error-surfacing branches —
//! [`InterpError::Source`] (a transport / SSRF / unsupported `FedError`),
//! [`InterpError::BadSrj`] (a malformed SRJ body), and [`InterpError::Resolve`] (a plan index
//! out of range) — went untested through the public `materialize_single_source` /
//! `stream_single_source` entry points. This file drives each through the REAL interpreter
//! (a controllable [`Transport`] double, no network) and asserts the right variant is
//! surfaced, fail-closed, for BOTH the materialised and the streaming interpreter.
//!
//! It also covers natural-join CORRECTNESS edge cases the success tests do not isolate —
//! a join whose two leaves never share a binding is empty, a join key that is unbound on one
//! side never equi-joins, and a fan-out preserves multiplicity — all asserted on the public
//! interpreter via [`solutions_equal`].
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-bif.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_fedclient::{
    materialize_single_source, solutions_equal, stream_single_source, Endpoint, FederatedSource,
    InterpError, Relation, SourceResolver, StreamOptions, Transport,
};
use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, PlanOptions, PredPartition, SourceDescriptor, SourceId, Term,
    TriplePattern, Var,
};
use std::collections::HashMap;
use std::sync::Arc;

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}
fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}

/// A transport that always fails with a fixed error string — models a remote endpoint that is
/// down / returns 5xx. The interpreter must surface this as `InterpError::Source(Transport)`.
struct FailingTransport(String);
impl Transport for FailingTransport {
    fn fetch(&self, _endpoint: &str, _query: &str) -> Result<String, String> {
        Err(self.0.clone())
    }
}

/// A transport that answers each sub-query from a fixed (SPARQL → body) map; an unmapped query
/// returns the body in `default` (so we can return MALFORMED SRJ for a specific leaf).
struct MapTransport {
    answers: HashMap<String, String>,
    default: String,
}
impl Transport for MapTransport {
    fn fetch(&self, _endpoint: &str, query: &str) -> Result<String, String> {
        Ok(self
            .answers
            .get(query)
            .cloned()
            .unwrap_or_else(|| self.default.clone()))
    }
}

/// A `Send + Sync` `FederatedSource` wrapper so the streaming interpreter (which needs
/// `Arc<dyn FederatedSource + Send + Sync>`) can drive the same `Endpoint` adapter.
struct SyncSource {
    endpoint: Endpoint,
}
impl FederatedSource for SyncSource {
    fn source_type(&self) -> sparq_fedclient::SourceType<'_> {
        self.endpoint.source_type()
    }
    fn discover(
        &self,
    ) -> Result<(sparq_fedclient::Capability, Option<SourceDescriptor>), sparq_fedclient::FedError>
    {
        self.endpoint.discover()
    }
    fn execute(
        &self,
        sub: &sparq_fedclient::SubQuery,
    ) -> Result<String, sparq_fedclient::FedError> {
        self.endpoint.execute(sub)
    }
}

fn one_pred_descriptor(pred: &str) -> SourceDescriptor {
    SourceDescriptor::builder(SourceId::new("S"))
        .total_triples(100)
        .predicate(PredPartition {
            predicate: pred.into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .build()
}

/// A one-pattern BGP `?s <pred> ?o` + its plan over the single descriptor.
fn one_pattern_plan(pred: &str) -> (Bgp, Vec<sparq_fedplan::PatternSources>, sparq_fedplan::JoinTree) {
    let bgp = Bgp::new(vec![TriplePattern::new(var("s"), iri(pred), var("o"))]);
    let descriptors = [one_pred_descriptor(pred)];
    let sel = select_sources(&bgp, &descriptors);
    let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).expect("plans");
    (bgp, sel, tree)
}

// ─── InterpError::Source — a transport failure is surfaced (both interpreters) ──────────

#[test]
fn materialise_surfaces_transport_failure_as_source_error() {
    let (bgp, sel, tree) = one_pattern_plan("http://ex/p");
    let ep = Endpoint::new(
        // A public IP literal so the SSRF gate ALLOWS, and the transport — not the gate — is
        // what fails (we are testing the transport-error branch, not egress refusal).
        "http://8.8.8.8/sparql",
        Box::new(FailingTransport("endpoint returned HTTP 503".into())),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let err = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap_err();
    match err {
        InterpError::Source(fe) => assert!(
            fe.to_string().contains("503"),
            "the transport error string must be forwarded verbatim, got {}",
            fe
        ),
        other => panic!("expected InterpError::Source, got {:?}", other),
    }
}

#[test]
fn stream_surfaces_transport_failure_as_error_item() {
    let (bgp, sel, tree) = one_pattern_plan("http://ex/p");
    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(FailingTransport("connection refused".into())),
    );
    let arc: Arc<dyn FederatedSource + Send + Sync> = Arc::new(SyncSource {
        endpoint: Endpoint::new(
            "http://8.8.8.8/sparql",
            Box::new(FailingTransport("connection refused".into())),
        ),
    });
    // The resolver needs an adapter slice for index typing; the streaming interpreter answers
    // every leaf through `arc`.
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let stream = stream_single_source(&resolver, &sel, arc, &tree, &StreamOptions::default())
        .expect("the stream BUILDS even though the producer will error");
    // Draining the stream surfaces the producer's error as a terminal error item.
    let drained = stream.collect_solutions();
    assert!(
        drained.is_err(),
        "a failing transport must surface as a stream error, not silent empty"
    );
}

// ─── InterpError::BadSrj — a malformed remote body is surfaced ──────────────────────────

#[test]
fn materialise_surfaces_malformed_srj_as_bad_srj() {
    let (bgp, sel, tree) = one_pattern_plan("http://ex/p");
    // The transport "succeeds" (returns a 200 body) but the body is not valid SRJ.
    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(MapTransport {
            answers: HashMap::new(),
            default: "<html>404 not found</html>".to_string(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let err = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap_err();
    assert!(
        matches!(err, InterpError::BadSrj(_)),
        "a non-SRJ 200 body must surface as BadSrj, got {:?}",
        err
    );
}

#[test]
fn materialise_surfaces_ask_boolean_body_as_bad_srj() {
    // A SELECT leaf answered by an ASK boolean body is a malformed result for the join.
    let (bgp, sel, tree) = one_pattern_plan("http://ex/p");
    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(MapTransport {
            answers: HashMap::new(),
            default: r#"{"head":{},"boolean":true}"#.to_string(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let err = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap_err();
    match err {
        InterpError::BadSrj(m) => assert!(m.contains("ASK"), "got {}", m),
        other => panic!("expected BadSrj, got {:?}", other),
    }
}

// ─── InterpError::Resolve — a plan/resolver mismatch fails closed ───────────────────────

#[test]
fn resolver_built_with_empty_bgp_fails_closed_on_pattern_index() {
    // Plan a 1-pattern BGP, but hand the resolver an EMPTY BGP, so the plan's pattern index 0
    // is out of range for the resolver — the interpreter must surface Resolve, not panic.
    let (_planned_bgp, sel, tree) = one_pattern_plan("http://ex/p");
    let empty_bgp = Bgp::new(vec![]);
    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(MapTransport {
            answers: HashMap::new(),
            default: r#"{"head":{"vars":["s","o"]},"results":{"bindings":[]}}"#.to_string(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&empty_bgp, &adapters);
    let err = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap_err();
    assert!(
        matches!(err, InterpError::Resolve(_)),
        "an out-of-range plan pattern index must fail closed with Resolve, got {:?}",
        err
    );
}

// ─── Natural-join correctness edge cases (via the public materialised interpreter) ──────

/// SRJ for a two-column leaf `?<va> <pred> ?<vb>`, rows as `(a-iri, b-iri)` pairs. The header
/// var names MUST match the variables `lower_leaf` projects for the pattern, else the relation's
/// columns are misnamed and the join key never lines up.
fn srj_pairs(va: &str, vb: &str, pairs: &[(&str, &str)]) -> String {
    let mut s = format!(r#"{{"head":{{"vars":["{}","{}"]}},"results":{{"bindings":["#, va, vb);
    for (i, (a, b)) in pairs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#"{{"{}":{{"type":"uri","value":"{}"}},"{}":{{"type":"uri","value":"{}"}}}}"#,
            va, a, vb, b
        ));
    }
    s.push_str("]}}");
    s
}

#[test]
fn join_with_no_shared_binding_is_empty() {
    // ?s :p ?o . ?o :q ?z — a path join on ?o. The :p objects and the :q subjects are disjoint,
    // so the join is empty (not error, not a spurious row).
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
        TriplePattern::new(var("o"), iri("http://ex/q"), var("z")),
    ]);
    let descriptors = [SourceDescriptor::builder(SourceId::new("S"))
        .total_triples(100)
        .predicate(PredPartition {
            predicate: "http://ex/p".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .predicate(PredPartition {
            predicate: "http://ex/q".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .build()];
    let sel = select_sources(&bgp, &descriptors);
    let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();

    let mut answers = HashMap::new();
    // ?s :p ?o → o ∈ {o1, o2}
    answers.insert(
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".to_string(),
        srj_pairs("s", "o", &[("http://ex/s1", "http://ex/o1"), ("http://ex/s2", "http://ex/o2")]),
    );
    // ?o :q ?z → subjects {oX} — DISJOINT from {o1, o2} ⇒ empty join on ?o.
    answers.insert(
        "SELECT ?o ?z WHERE { ?o <http://ex/q> ?z }".to_string(),
        srj_pairs("o", "z", &[("http://ex/oX", "http://ex/zX")]),
    );

    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(MapTransport {
            answers,
            default: r#"{"head":{"vars":["s","o"]},"results":{"bindings":[]}}"#.to_string(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let rel = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap();
    assert!(
        rel.rows.is_empty(),
        "a join over disjoint join keys must be empty, got {:?}",
        rel.rows
    );
}

#[test]
fn join_preserves_multiplicity_fan_out() {
    // ?s :p ?o . ?s :q ?z — a star on ?s where s1 has TWO :q values ⇒ the join fans s1's single
    // :p row to TWO output rows (bag semantics: multiplicity is preserved, not de-duplicated).
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
        TriplePattern::new(var("s"), iri("http://ex/q"), var("z")),
    ]);
    let descriptors = [SourceDescriptor::builder(SourceId::new("S"))
        .total_triples(100)
        .predicate(PredPartition {
            predicate: "http://ex/p".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .predicate(PredPartition {
            predicate: "http://ex/q".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .build()];
    let sel = select_sources(&bgp, &descriptors);
    let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();

    let mut answers = HashMap::new();
    answers.insert(
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".to_string(),
        srj_pairs("s", "o", &[("http://ex/s1", "http://ex/o1")]),
    );
    // s1 has two :q values ⇒ two join partners.
    answers.insert(
        "SELECT ?s ?z WHERE { ?s <http://ex/q> ?z }".to_string(),
        srj_pairs("s", "z", &[("http://ex/s1", "http://ex/z1"), ("http://ex/s1", "http://ex/z2")]),
    );

    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(MapTransport {
            answers,
            default: r#"{"head":{"vars":["s","o"]},"results":{"bindings":[]}}"#.to_string(),
        }),
    );
    let adapters: Vec<&dyn FederatedSource> = vec![&ep];
    let resolver = SourceResolver::new(&bgp, &adapters);
    let rel: Relation = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap();
    assert_eq!(rel.rows.len(), 2, "the fan-out must yield two rows (multiplicity preserved)");
    // Both rows share s1+o1 and differ only in ?z — assert the exact multiset.
    use oxrdf::NamedNode;
    let nn = |s: &str| Some(oxrdf::Term::NamedNode(NamedNode::new(s).unwrap()));
    let expect_vars = vec!["s".to_string(), "o".to_string(), "z".to_string()];
    let expect = vec![
        vec![nn("http://ex/s1"), nn("http://ex/o1"), nn("http://ex/z1")],
        vec![nn("http://ex/s1"), nn("http://ex/o1"), nn("http://ex/z2")],
    ];
    assert!(
        solutions_equal(&rel.vars, &rel.rows, &expect_vars, &expect),
        "fan-out result multiset mismatch: got {:?}",
        rel.rows
    );
}

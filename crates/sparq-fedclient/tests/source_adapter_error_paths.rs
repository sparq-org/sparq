//! sq-bif.2 — source-adapter (transport / wire / fragment) error-path suite.
//!
//! The inline `source` tests cover the SSRF guard (deny / allow), the happy Endpoint fetch, and
//! the fragment completeness invariant against an always-succeeding fixture server. This file
//! targets the ADAPTER-level error and edge branches the inline suite does not:
//!
//!  * the [`Endpoint`] adapter forwarding a **transport error** as [`FedError::Transport`]
//!    (after the SSRF gate ALLOWS — a public IP literal, so the transport, not the gate, fails);
//!  * the SSRF guard refusing a **DNS name that resolves only to private/internal** addresses
//!    (`localhost`) and an endpoint **with no host authority** ([`FedError::BadEndpoint`]);
//!  * a [`FragmentTransport`] that returns an **error** — surfaced as [`FedError::Transport`]
//!    through both [`TpfSource::solutions`] and [`BrTpfSource::solutions`];
//!  * a fragment server whose pages **never terminate** (no `hydra:next` end) is fail-stopped by
//!    the page-cap rather than looping forever;
//!  * the [`Capability`] per-interface defaults + the brTPF `maxMpR` clamp + `FragPattern::vars`
//!    repeated-variable de-duplication, units the success path exercises only incidentally.
//!
//! All hermetic — a controllable in-memory transport double, no network.
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-bif.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_fedclient::{
    BindJoin, BrTpfSource, Capability, EgressGuard, Endpoint, FedError, FederatedSource,
    FragPattern, FragTerm, FragTriple, FragmentPage, FragmentTransport, Interface, PatternTerm,
    SubQuery, TpfSource, Transport,
};

// ─── Endpoint adapter: transport error → FedError::Transport (after egress ALLOWS) ──────

/// A transport that always fails with a fixed error string.
struct FailingTransport(&'static str);
impl Transport for FailingTransport {
    fn fetch(&self, _endpoint: &str, _query: &str) -> Result<String, String> {
        Err(self.0.to_string())
    }
}

#[test]
fn endpoint_forwards_transport_error_verbatim() {
    // A PUBLIC IP literal so the default-deny guard ALLOWS; the transport then fails, and the
    // adapter surfaces the transport's error string as FedError::Transport (verbatim).
    let ep = Endpoint::new(
        "http://8.8.8.8/sparql",
        Box::new(FailingTransport("upstream timed out")),
    );
    let err = ep.execute(&SubQuery::new("SELECT * WHERE { ?s ?p ?o }")).unwrap_err();
    match err {
        FedError::Transport(m) => assert_eq!(m, "upstream timed out"),
        other => panic!("expected FedError::Transport, got {:?}", other),
    }
}

// ─── EgressGuard: DNS-name resolving only to private, and no-host authority ─────────────

#[test]
fn guard_refuses_localhost_dns_name() {
    // `localhost` resolves only to loopback (127.0.0.1 / ::1); the default-deny guard must
    // refuse it on the resolved IP (DNS-rebinding-safe), with EgressRefused.
    let g = EgressGuard::deny_private();
    let err = g.check_endpoint("http://localhost:7000/sparql").unwrap_err();
    assert!(
        matches!(err, FedError::EgressRefused(_)),
        "localhost resolves only to loopback ⇒ refused, got {:?}",
        err
    );
}

#[test]
fn guard_rejects_endpoint_with_no_host() {
    // An endpoint IRI with no host authority cannot be vetted ⇒ BadEndpoint.
    let g = EgressGuard::deny_private();
    let err = g.check_endpoint("http:///no-host/sparql").unwrap_err();
    assert!(
        matches!(err, FedError::BadEndpoint(_)),
        "no host authority ⇒ BadEndpoint, got {:?}",
        err
    );
}

#[test]
fn endpoint_execute_refuses_private_before_transport() {
    // A private endpoint must be refused at the SSRF gate BEFORE the transport is reached — the
    // PanicTransport proves no fetch happens.
    struct PanicTransport;
    impl Transport for PanicTransport {
        fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
            panic!("the transport must not be reached when the egress gate denies");
        }
    }
    let ep = Endpoint::new("http://10.1.2.3/sparql", Box::new(PanicTransport));
    let err = ep.execute(&SubQuery::new("ASK {}")).unwrap_err();
    assert!(matches!(err, FedError::EgressRefused(_)), "got {:?}", err);
}

// ─── Fragment adapters: a transport error → FedError::Transport ─────────────────────────

/// A fragment transport that always fails.
struct FailingFragments(&'static str);
impl FragmentTransport for FailingFragments {
    fn fetch_fragment(
        &self,
        _url: &str,
        _pattern: &FragPattern,
        _bindings: &[Vec<(String, FragTerm)>],
        _page: Option<&str>,
    ) -> Result<FragmentPage, String> {
        Err(self.0.to_string())
    }
}

fn knows_pattern() -> FragPattern {
    FragPattern::new(
        PatternTerm::Var("s".into()),
        PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows")),
        PatternTerm::Var("o".into()),
    )
}

#[test]
fn tpf_solutions_surfaces_fragment_transport_error() {
    let tpf = TpfSource::new("http://frag/tpf", Box::new(FailingFragments("502 bad gateway")));
    let err = tpf.solutions(&knows_pattern()).unwrap_err();
    match err {
        FedError::Transport(m) => assert!(m.contains("502"), "got {}", m),
        other => panic!("expected FedError::Transport, got {:?}", other),
    }
}

#[test]
fn brtpf_solutions_surfaces_fragment_transport_error() {
    let brtpf = BrTpfSource::new("http://frag/brtpf", 10, Box::new(FailingFragments("dns failure")));
    // Even with an upstream binding block, the first fetch fails and is surfaced.
    let bindings = vec![vec![("s".to_string(), FragTerm::iri("http://ex/alice"))]];
    let err = brtpf.solutions(&knows_pattern(), &bindings).unwrap_err();
    assert!(matches!(err, FedError::Transport(_)), "got {:?}", err);
}

#[test]
fn fragment_execute_routes_to_solutions_not_srj() {
    // A fragment source's SRJ `execute` is intentionally Unsupported (it speaks triples), and
    // points the caller at the typed `solutions` method.
    let tpf = TpfSource::new("http://frag/tpf", Box::new(FailingFragments("x")));
    let err = tpf.execute(&SubQuery::new("SELECT * WHERE { ?s ?p ?o }")).unwrap_err();
    assert!(matches!(err, FedError::Unsupported(_)), "got {:?}", err);
}

// ─── Fragment pagination: a never-terminating server is fail-stopped by the page cap ────

/// A misbehaving fragment server that ALWAYS returns a `next` token (never `None`), so a naive
/// drainer would loop forever. The adapter's page cap must fail-stop it with a transport error.
struct InfinitePager;
impl FragmentTransport for InfinitePager {
    fn fetch_fragment(
        &self,
        _url: &str,
        pattern: &FragPattern,
        _bindings: &[Vec<(String, FragTerm)>],
        page: Option<&str>,
    ) -> Result<FragmentPage, String> {
        // Each page returns one matching triple and ALWAYS a next token (monotonic counter).
        let n: u64 = page
            .and_then(|t| t.strip_prefix("p:"))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let triple = FragTriple::new(
            FragTerm::iri(format!("http://ex/s{}", n)),
            FragTerm::iri("http://xmlns.com/foaf/0.1/knows"),
            FragTerm::iri(format!("http://ex/o{}", n)),
        );
        // Only emit a data triple that actually matches the requested pattern.
        let triples = if pattern.predicate
            == PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows"))
        {
            vec![triple]
        } else {
            vec![]
        };
        Ok(FragmentPage {
            triples,
            total_items: u64::MAX,
            next: Some(format!("p:{}", n + 1)), // NEVER None — runs forever without the cap.
        })
    }
}

#[test]
fn fragment_pagination_is_fail_stopped_by_page_cap() {
    // A server that never sets next = None is bounded by the defensive page cap, surfacing a
    // clean transport error rather than hanging the client. (The cap is ~1e6 pages; this test
    // proves the cap EXISTS and fail-stops — a real server terminates well within it.)
    //
    // NOTE: this drains up to the cap, so it is intentionally a heavier test; it still completes
    // in well under a second because each fixture page is trivial.
    let tpf = TpfSource::new("http://frag/runaway", Box::new(InfinitePager));
    let err = tpf.solutions(&knows_pattern()).unwrap_err();
    match err {
        FedError::Transport(m) => assert!(
            m.contains("pagination") && m.contains("cap"),
            "the runaway pager must fail-stop with a page-cap transport error, got {}",
            m
        ),
        other => panic!("expected a page-cap Transport error, got {:?}", other),
    }
}

// ─── Capability defaults + maxMpR clamp + FragPattern::vars dedup ───────────────────────

#[test]
fn capability_defaults_match_interface() {
    let e = Capability::endpoint();
    assert_eq!(e.interface, Interface::Endpoint);
    assert_eq!(e.bind_join, BindJoin::Values);
    assert!(e.aggregates && e.property_paths && e.order_limit);

    let l = Capability::local();
    assert_eq!(l.interface, Interface::LocalEngine);
    assert_eq!(l.bind_join, BindJoin::Values, "local engine inherits endpoint caps");

    let b = Capability::brtpf(42);
    assert_eq!(b.interface, Interface::BrTpf);
    assert_eq!(b.bind_join, BindJoin::MaxMpR(42));
    assert!(!b.aggregates && !b.property_paths && !b.order_limit);

    let t = Capability::tpf();
    assert_eq!(t.interface, Interface::Tpf);
    assert_eq!(t.bind_join, BindJoin::None);
}

#[test]
fn frag_pattern_vars_dedups_repeated_variable() {
    // ?x knows ?x — the same variable in subject + object position is projected once.
    let pat = FragPattern::new(
        PatternTerm::Var("x".into()),
        PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows")),
        PatternTerm::Var("x".into()),
    );
    assert_eq!(pat.vars(), vec!["x".to_string()]);
    // A fully-variable pattern projects all three distinct names in subject→predicate→object order.
    let triple = FragPattern::new(
        PatternTerm::Var("s".into()),
        PatternTerm::Var("p".into()),
        PatternTerm::Var("o".into()),
    );
    assert_eq!(
        triple.vars(),
        vec!["s".to_string(), "p".to_string(), "o".to_string()]
    );
    // A fully-bound pattern projects nothing.
    let bound = FragPattern::new(
        PatternTerm::Bound(FragTerm::iri("http://ex/a")),
        PatternTerm::Bound(FragTerm::iri("http://ex/p")),
        PatternTerm::Bound(FragTerm::iri("http://ex/b")),
    );
    assert!(bound.vars().is_empty());
}

#[test]
fn pattern_term_as_var_distinguishes_var_from_bound() {
    assert_eq!(PatternTerm::Var("s".into()).as_var(), Some("s"));
    assert_eq!(PatternTerm::Bound(FragTerm::iri("http://ex/a")).as_var(), None);
}

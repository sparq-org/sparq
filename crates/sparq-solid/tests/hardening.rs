//! Regression tests for the roborev-1723 hardening findings: reserved-namespace
//! graph stripping, pair-principal delimiter injection, rewrite variable capture.

use sparq_core::Graph;
use sparq_solid::fixture::{wac_fixture, ALICE};
use sparq_solid::{rewrite_for, Mode, PodStore, Session};

/// A dataset smuggling in the rewrite sentinel graph must not make it visible to a
/// session with NO grants (the empty-FROM-NAMED fail-closed path).
#[test]
fn sentinel_graph_cannot_be_smuggled() {
    let mut nq = wac_fixture();
    nq.push_str("<urn:sparq:nothing#s> <urn:p> \"leak\" <urn:sparq:nothing> .\n");
    nq.push_str("<urn:sparq:auth#s> <https://sparq.dev/ns/auth#read> <https://pod.ex/priv0/> <urn:sparq:auth> .\n");
    let mut s = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
    // BOTH reserved graphs are dropped at construction (incl. the forged auth view —
    // only install_auth_view may create <urn:sparq:auth>)
    assert!(!s.graph().named().iter().any(|(n, _)| match n {
        oxrdf::Term::NamedNode(n) => n.as_str().starts_with("urn:sparq:"),
        _ => false,
    }));
    assert!(s.accessible(&Session::default(), Mode::Read).is_empty());
    s.materialize_wac().unwrap();
    let q = "SELECT ?o WHERE { GRAPH ?g { ?s ?p ?o } }";
    // a session with zero grants must see nothing — even the sentinel-named graph
    let stranger = Session { agent: Some("https://stranger.ex/card#me"), client: None, issuer: None, now: None };
    let denied_modes = s.accessible(&stranger, Mode::Write);
    assert!(denied_modes.is_empty());
    let res = s.query_as(&stranger, Mode::Write, q).unwrap();
    assert_eq!(res.rows.len(), 0, "sentinel graph must not leak");
}

/// An ACL whose agent WebID embeds the pair delimiter could collide with a minted
/// (agent, client) pair principal: the loader rejects the whole document, so it grants
/// nothing, and the pair it imitates gets nothing from it.
#[test]
fn pair_delimiter_injection_is_rejected() {
    let mut nq = wac_fixture();
    // crafted agent: pretends to be the pair (alice, https://app.ex)
    let evil = format!("urn:x{}{}", "?agent=", "https://alice.ex/card%23me&client=https://app.ex");
    nq.push_str(&format!(
        "<https://pod.ex/.acl#evil> <http://www.w3.org/ns/auth/acl#agent> <{evil}> <https://pod.ex/.acl> .\n"
    ));
    let mut s = PodStore::new(Graph::load_dataset(&nq, "nquads").unwrap());
    s.materialize_wac().expect("the document is rejected, not fatal");
    let mut clean = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
    clean.materialize_wac().unwrap();
    let alone = Session { agent: Some(&evil), client: None, issuer: None, now: None };
    assert!(s.accessible(&alone, Mode::Read).is_empty(), "the crafted name is no principal");
    let pair = Session { agent: Some("https://alice.ex/card#me"), client: Some("https://app.ex"), issuer: None, now: None };
    let before = clean.accessible(&pair, Mode::Read);
    let after = s.accessible(&pair, Mode::Read);
    assert!(after.iter().all(|g| before.contains(g)), "the imitated pair gains nothing: {after:?}");
}

/// User variables shaped like the internal graph variables must not be captured by the
/// per-pattern GRAPH wrap.
#[test]
fn rewrite_avoids_user_variable_capture() {
    let allowed = [oxrdf::NamedNode::new("https://pod.ex/pub1/c0/g0/d0.ttl").unwrap()];
    let q = "SELECT ?__sg0 WHERE { ?__sg0 <https://ex.dev/ns#title> ?t . ?x <https://ex.dev/ns#rel> ?y }";
    let out = rewrite_for(q, &allowed).unwrap();
    assert!(!out.contains("GRAPH ?__sg0"), "internal vars must avoid ?__sg0: {out}");
    // …and the rewritten query still parses
    assert!(spargebra::SparqlParser::new().parse_query(&out).is_ok(), "{out}");
}

/// A caller-supplied "WebID"/client inside the reserved principal encoding must not
/// impersonate a minted pair principal — sessions fail closed.
#[test]
fn reserved_session_values_fail_closed() {
    let mut s = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
    s.materialize_wac().unwrap();
    let spoof = "urn:sparq:pair?agent=https://bob.ex/card#me&client=https://app.ex";
    assert!(s.accessible(&Session { agent: Some(spoof), client: None, issuer: None, now: None }, Mode::Read).is_empty());
    assert!(s
        .accessible(
            &Session { agent: Some("https://bob.ex/card#me"), client: Some("urn:sparq:x"), issuer: None, now: None },
            Mode::Read
        )
        .is_empty());
    // delimiter-only values (NOT in the reserved space) must fail closed too: a WebID
    // embedding `&client=` could otherwise complete someone else's pair encoding
    let delim_agent = format!("https://x.ex/a{}https://app.ex", "&client=");
    assert!(s.accessible(&Session { agent: Some(&delim_agent), client: None, issuer: None, now: None }, Mode::Read).is_empty());
    assert!(s
        .accessible(
            &Session { agent: Some("https://bob.ex/card#me"), client: Some(&delim_agent), issuer: None, now: None },
            Mode::Read
        )
        .is_empty());
    // …while the REAL pair still works
    let real = Session { agent: Some("https://bob.ex/card#me"), client: Some("https://app.ex"), issuer: None, now: None };
    assert!(s
        .accessible(&real, Mode::Read)
        .iter()
        .any(|g| g.as_str() == "https://pod.ex/origin5/c2/g0/d0.ttl"));
}

/// End-to-end sanity after hardening: the normal path still works.
// sq-gq28y (issue #1546): probes via an explicit `GRAPH ?g` pattern (feature-
// agnostic under the empty-default flip — a single-triple bare pattern under the old
// union-always default was exactly this) so alice's authorized-union count (599) is asserted
// on the default (spec-conformant) build.
#[test]
fn normal_path_still_green_after_hardening() {
    let mut s = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
    s.materialize_wac().unwrap();
    let res = s
        .query_as(
            &Session { agent: Some(ALICE), client: None, issuer: None, now: None },
            Mode::Read,
            "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }",
        )
        .unwrap();
    assert_eq!(res.rows.len(), 599);
}

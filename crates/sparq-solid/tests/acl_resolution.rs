//! ACL discovery parity with the original suffix-based ancestor walk.
mod common;

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_solid::{AclScope, AclStatus, EffectiveAcl, Mode, PodStore, Session, WacDecision};
use std::collections::HashSet;

// Keep both the old suffix lookup and the old parent walk independent of production.
fn old_resolve(graph: &Graph, resource: &str) -> Option<EffectiveAcl> {
    let control: HashSet<_> = graph
        .named()
        .iter()
        .filter_map(|(name, _)| match name {
            Term::NamedNode(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let mut cur = resource;
    loop {
        for suffix in [".acl", ".acr"] {
            let candidate = format!("{cur}{suffix}");
            if control.contains(candidate.as_str()) {
                return Some(EffectiveAcl {
                    acl: NamedNode::new_unchecked(candidate),
                    scope: if cur == resource {
                        AclScope::AccessTo
                    } else {
                        AclScope::Default
                    },
                });
            }
        }
        let scheme_end = cur.find("://").map(|i| i + 3)?;
        let host_end = scheme_end + cur[scheme_end..].find('/')?;
        let trimmed = cur.strip_suffix('/').unwrap_or(cur);
        if trimmed.len() <= host_end {
            return None;
        }
        let cut = trimmed.rfind('/')?;
        if cut < host_end {
            return None;
        }
        cur = &cur[..cut + 1];
    }
}

// Main's decision body, using the independent original ACL resolver above and
// the public authorization oracle rather than production structural discovery.
fn original_decision(
    store: &PodStore,
    session: &Session,
    resource: &str,
    mode: Mode,
) -> WacDecision {
    let mut result = WacDecision {
        allow: false,
        granted_modes: Vec::new(),
        governing_acl: None,
        scope: None,
        status: AclStatus::Transient,
    };
    let Ok(node) = NamedNode::new(resource) else {
        return result;
    };
    let Some(effective) = old_resolve(store.graph(), resource) else {
        result.status = AclStatus::NoAcl;
        return result;
    };
    result.governing_acl = Some(effective.acl);
    result.scope = Some(effective.scope);
    if !store
        .graph()
        .named()
        .iter()
        .any(|(n, _)| matches!(n, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH))
    {
        result.status = AclStatus::Unloaded;
        return result;
    }
    result.status = AclStatus::Resolved;
    result.granted_modes = [Mode::Read, Mode::Write, Mode::Append, Mode::Control]
        .into_iter()
        .filter(|m| store.accessible(session, *m).contains(&node))
        .collect();
    result.allow = result.granted_modes.contains(&mode);
    result
}

fn assert_decision_parity(store: &PodStore, resource: &str, session: &Session) {
    for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
        assert_eq!(
            store.decide(session, resource, mode),
            original_decision(store, session, resource, mode),
            "{resource}: {mode:?}"
        );
    }
}

#[test]
fn decisions_match_main_over_wac_fixtures_and_name_boundaries() {
    for scenario in common::wac_corpus() {
        let mut store =
            PodStore::new(Graph::load_dataset(scenario.nquads_str(), "nquads").unwrap());
        for materialized in [false, true] {
            if materialized {
                store.materialize_wac().unwrap();
            }
            for expected in scenario.expects() {
                let session = Session {
                    agent: expected.req_agent(),
                    client: expected.req_client(),
                    ..Session::default()
                };
                assert_decision_parity(&store, expected.req_resource(), &session);
            }
        }
    }
    let mut base =
        PodStore::new(Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads").unwrap());
    base.materialize_wac().unwrap();
    let cap = base.auth_budget().max_resource_iri_bytes;
    let mut graph = base.graph().fork();
    let at_cap = format!(
        "https://pod.ex/{}",
        "x".repeat(cap - "https://pod.ex/".len())
    );
    for target in [
        &at_cap,
        &format!("https://other.ex/{}", "x".repeat(1 << 20)),
    ] {
        graph
            .add_named(
                Term::NamedNode(NamedNode::new_unchecked(format!("{target}.acl"))),
                Graph::default(),
            )
            .unwrap();
    }
    let store = PodStore::new(graph);
    for resource in [
        at_cap,
        "https://pod.ex/".into(),
        "https://pod.ex//a///leaf?x#part".into(),
    ] {
        for agent in [None, Some("https://alice.ex/card#me")] {
            assert_decision_parity(
                &store,
                &resource,
                &Session {
                    agent,
                    ..Session::default()
                },
            );
        }
    }
    for resource in [
        format!("https://pod.ex/{}leaf", "a/".repeat(4096)),
        format!(
            "https://pod.ex/{}leaf",
            format!("{}/", "a".repeat(4095)).repeat(63)
        ),
    ] {
        for agent in [None, Some("https://alice.ex/card#me")] {
            for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
                assert_eq!(
                    store.decide(
                        &Session {
                            agent,
                            ..Session::default()
                        },
                        &resource,
                        mode
                    ),
                    WacDecision {
                        allow: false,
                        granted_modes: Vec::new(),
                        governing_acl: None,
                        scope: None,
                        status: AclStatus::NoAcl,
                    }
                );
            }
        }
    }
}

#[test]
fn acl_resolution_matches_original_over_wac_fixtures() {
    for scenario in common::wac_corpus() {
        let store = PodStore::new(Graph::load_dataset(scenario.nquads_str(), "nquads").unwrap());
        for expected in scenario.expects() {
            let resource = expected.req_resource();
            assert_eq!(
                store.resolve_acl(resource),
                old_resolve(store.graph(), resource),
                "{}: {resource}",
                scenario.name()
            );
        }
    }
    for fixture in [sparq_solid::wac_fixture(), sparq_solid::acp_fixture()] {
        let store = PodStore::new(Graph::load_dataset(&fixture, "nquads").unwrap());
        for (name, _) in store.graph().named() {
            if let Term::NamedNode(n) = name {
                for resource in [
                    n.as_str(),
                    n.as_str()
                        .strip_suffix(".acl")
                        .or_else(|| n.as_str().strip_suffix(".acr"))
                        .unwrap_or(n.as_str()),
                ] {
                    assert_eq!(
                        store.resolve_acl(resource),
                        old_resolve(store.graph(), resource)
                    );
                }
            }
        }
    }
}

#[test]
fn acl_resolution_matches_original_over_random_ancestors() {
    // Deterministic generation, including empty segments, Unicode, both control
    // suffixes at one target, absent ACLs, and resources beyond every stored name.
    let mut seed = 0x81f6_2039_475a_ebcd_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..256 {
        let mut dataset = String::new();
        let mut resource = "https://pod.ex/".to_owned();
        let mut resources = vec![resource.clone(), "urn:example:resource".into()];
        let depth = next() % 80;
        for _ in 0..depth {
            let r = next();
            resource.push_str(match r % 4 {
                0 => "/",
                1 => "é/",
                2 => "a/",
                _ => "long-segment/",
            });
            resources.push(resource.clone());
            if r % 3 == 0 {
                dataset.push_str(&format!("<urn:s> <urn:p> <urn:o> <{resource}.acr> .\n"));
            }
            if r % 5 == 0 {
                dataset.push_str(&format!("<urn:s> <urn:p> <urn:o> <{resource}.acl> .\n"));
            }
        }
        let outside_caps = format!("{resource}{}", "x/".repeat(2048));
        resources.push(format!("{resource}leaf"));
        let mut store = PodStore::new(Graph::load_dataset(&dataset, "nquads").unwrap());
        // This generated corpus admits targets beyond the default container cap.
        let mut budget = store.auth_budget();
        budget.max_resource_path_depth = 128;
        store.set_auth_budget(budget);
        for r in resources {
            assert_eq!(
                store.resolve_acl(&r),
                old_resolve(store.graph(), &r),
                "resource {r}"
            );
        }
        assert!(store.resolve_acl(&outside_caps).is_none());
    }
}

#[test]
fn name_caps_are_captured_when_the_view_is_materialized() {
    let resource = format!("https://pod.ex/{}", "a".repeat(80));
    let mut graph = Graph::default();
    graph
        .add_named(
            Term::NamedNode(NamedNode::new_unchecked(format!("{resource}.acl"))),
            Graph::default(),
        )
        .unwrap();
    let mut store = PodStore::new(graph);
    store.materialize_wac().unwrap();
    let mut budget = store.auth_budget();
    budget.max_resource_iri_bytes = 32;
    store.set_auth_budget(budget);
    assert!(
        store.resolve_acl(&resource).is_some(),
        "the installed view retains its name budget"
    );
    store.materialize_wac().unwrap();
    assert!(
        store.resolve_acl(&resource).is_none(),
        "a rebuilt view uses the lowered cap"
    );
}

#[test]
fn acl_precedes_acr_in_both_document_orders() {
    for suffixes in [[".acl", ".acr"], [".acr", ".acl"]] {
        let mut graph = Graph::default();
        for suffix in suffixes {
            graph
                .add_named(
                    Term::NamedNode(NamedNode::new_unchecked(format!(
                        "https://pod.ex/r{suffix}"
                    ))),
                    Graph::default(),
                )
                .unwrap();
        }
        let store = PodStore::new(graph);
        let effective = store.resolve_acl("https://pod.ex/r").unwrap();
        assert_eq!(effective.acl.as_str(), "https://pod.ex/r.acl");
        assert_eq!(effective.scope, AclScope::AccessTo);
    }
}

#[test]
fn request_names_outside_view_caps_are_refused_before_discovery() {
    let mut budget = sparq_solid::default_auth_budget();
    budget.max_resource_iri_bytes = 64;
    budget.max_resource_path_depth = 2;
    let long = format!("https://pod.ex/{}", "x".repeat(51));
    for resource in [long.as_str(), "https://pod.ex/a/b/c"] {
        let refusal = budget.validate_resource_iri(resource).unwrap_err();
        assert_eq!(
            sparq_solid::auth_allowance_refusal(&refusal),
            Some(sparq_solid::AuthAllowance::ResourceIri)
        );
        for ancestor in [false, true] {
            let mut dataset = format!(
                r#"
<{resource}.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <{resource}.acl> .
<{resource}.acl#r> <http://www.w3.org/ns/auth/acl#accessTo> <{resource}> <{resource}.acl> .
<{resource}.acl#r> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <{resource}.acl> .
<{resource}.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <{resource}.acl> .
"#
            );
            if ancestor {
                dataset.push_str(r#"
<https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/.acl> .
<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#);
            }
            let mut store = PodStore::new(Graph::load_dataset(&dataset, "nquads").unwrap());
            store.set_auth_budget(budget);
            store.materialize_wac().unwrap();
            let session = Session::default();
            let decision = store.decide(&session, resource, Mode::Read);
            assert!(!decision.allow);
            assert_eq!(decision.status, AclStatus::NoAcl);
            assert!(!decision.status.is_retryable());
            assert!(decision.governing_acl.is_none());
            assert!(decision.scope.is_none());
            assert!(decision.granted_modes.is_empty());
            assert_eq!(
                store.decide_batch(&session, &[(resource, Mode::Read)]),
                vec![decision]
            );
            assert!(store.resolve_acl(resource).is_none());
            assert_eq!(
                store.wac_allow(&session, &NamedNode::new(resource).unwrap()),
                r#"user="",public="""#
            );
        }
    }
}

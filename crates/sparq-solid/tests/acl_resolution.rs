//! ACL discovery parity with the original suffix-based ancestor walk.
mod common;

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_solid::{AclScope, EffectiveAcl, PodStore};
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
                0 => "",
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
        resources.push(format!("{resource}{}", "x/".repeat(2048)));
        resources.push(format!("{resource}leaf"));
        let store = PodStore::new(Graph::load_dataset(&dataset, "nquads").unwrap());
        for r in resources {
            assert_eq!(
                store.resolve_acl(&r),
                old_resolve(store.graph(), &r),
                "resource {r}"
            );
        }
    }
}

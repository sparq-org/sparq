//! Baseline parity for installed views, fixture requests, and closure charges.
//!
//! Regenerate on the baseline revision: copy this harness and its charge observer into
//! an archived source tree, then run the library test with AUTH_LEDGER_GOLDEN_DIR set
//! to a scratch directory. Copy the resulting JSON fixtures here only after reviewing
//! the source revision and the view/decision differences. The recorded revision is
//! `4f9c392005490862aa4e2d0d8c632b933a5eb6a7`. Run
//! `cargo test -p sparq-solid --all-features --lib baseline` with the scratch directory
//! in `AUTH_LEDGER_GOLDEN_DIR`. Never regenerate on the implementation being checked.

mod common;

use oxrdf::Term;
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_solid::{AccessProvenance, Mode, PodStore, Session, VerifiedCredentials};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Case {
    pub name: String,
    pub source: String,
    pub acp: bool,
    pub provenance: AccessProvenance,
    pub credentials: VerifiedCredentials,
    pub requests: Vec<(Option<String>, Option<String>, String, Mode)>,
}

impl Case {
    pub fn run(&self) -> PodStore {
        let mut store = PodStore::new(Graph::load_dataset(&self.source, "nquads").unwrap());
        if self.acp {
            store
                .materialize_acp_with_credentials(&self.provenance, &self.credentials)
                .unwrap();
        } else {
            store.materialize_wac().unwrap();
        }
        store
    }
}

pub(crate) fn cases() -> Vec<Case> {
    let empty = |name: String, source: String, acp, requests| Case {
        name,
        source,
        acp,
        requests,
        provenance: AccessProvenance::new(),
        credentials: VerifiedCredentials::new(),
    };
    let mut cases = Vec::new();
    for scenario in common::wac_corpus() {
        cases.push(empty(
            format!("wac/{}", scenario.name()),
            scenario.nquads_str().into(),
            false,
            scenario
                .expects()
                .iter()
                .map(|e| {
                    (
                        e.req_agent().map(str::to_owned),
                        e.req_client().map(str::to_owned),
                        e.req_resource().into(),
                        e.req_mode(),
                    )
                })
                .collect(),
        ));
    }
    for scenario in common::acp_corpus() {
        cases.push(empty(
            format!("acp/{}", scenario.name()),
            scenario.nquads_str().into(),
            true,
            scenario
                .expects()
                .iter()
                .map(|e| {
                    (
                        e.req_agent().map(str::to_owned),
                        e.req_client().map(str::to_owned),
                        e.req_resource().into(),
                        e.req_mode(),
                    )
                })
                .collect(),
        ));
    }
    for (name, source, acp) in [
        ("fixture/wac", sparq_solid::wac_fixture(), false),
        (
            "fixture/wac-content",
            sparq_solid::fixture::wac_fixture_sized(50),
            false,
        ),
        ("fixture/acp", sparq_solid::acp_fixture(), true),
        ("fixture/acp-context", sparq_solid::acp_fixture(), true),
    ] {
        let graph = Graph::load_dataset(&source, "nquads").unwrap();
        let mut resources = BTreeSet::new();
        for (name, _) in graph.named() {
            if let Term::NamedNode(n) = name {
                resources.insert(n.as_str().to_owned());
                let mut cur = n.as_str();
                while let Some(parent) = sparq_solid::parent_container(cur) {
                    resources.insert(parent.to_owned());
                    cur = parent;
                }
            }
        }
        let mut requests = Vec::new();
        for agent in [
            None,
            Some(sparq_solid::fixture::ALICE),
            Some(sparq_solid::fixture::BOB),
            Some(sparq_solid::fixture::CAROL),
            Some(sparq_solid::fixture::DAVE),
        ] {
            for client in [
                None,
                Some(sparq_solid::fixture::APP),
                Some("https://other.ex"),
            ] {
                for resource in &resources {
                    for mode in [Mode::Read, Mode::Append, Mode::Write, Mode::Control] {
                        requests.push((
                            agent.map(str::to_owned),
                            client.map(str::to_owned),
                            resource.clone(),
                            mode,
                        ));
                    }
                }
            }
        }
        let mut case = empty(name.into(), source, acp, requests);
        if name.ends_with("context") {
            for resource in &resources {
                case.provenance
                    .set_creator(resource, sparq_solid::fixture::CAROL);
                case.provenance
                    .set_owner(resource, sparq_solid::fixture::ALICE);
            }
            case.credentials
                .hold(sparq_solid::fixture::BOB, "https://credentials.ex/member");
        }
        cases.push(case);
    }
    cases
}

fn views(graph: &Graph) -> BTreeMap<String, Value> {
    let mut views = BTreeMap::new();
    for (name, graph) in graph.named() {
        let Term::NamedNode(name) = name else {
            continue;
        };
        if !name.as_str().starts_with("urn:sparq:") {
            continue;
        }
        let scan = graph.store().scan(&[None, None, None]);
        let mut triples: Vec<_> = scan
            .rows
            .iter()
            .map(|row| {
                let [s, p, o] = scan.to_spo(row);
                format!(
                    "{} {} {}",
                    graph.dict().term(s),
                    graph.dict().term(p),
                    graph.dict().term(o)
                )
            })
            .collect();
        triples.sort();
        views.insert(name.as_str().into(), compact_view(&triples));
    }
    views
}

fn intern_snapshot(pool: &mut Vec<Value>, value: Value) -> usize {
    pool.iter()
        .position(|entry| *entry == value)
        .unwrap_or_else(|| {
            pool.push(value);
            pool.len() - 1
        })
}

fn compact_view(triples: &[String]) -> Value {
    if triples.iter().map(String::len).sum::<usize>() <= 1024 {
        return json!({"triples": triples});
    }
    let mut digest = 0x6c62272e07bb014262b821756295c58du128;
    for triple in triples {
        for byte in (triple.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(triple.as_bytes())
        {
            digest ^= u128::from(*byte);
            digest = digest.wrapping_mul(0x0000000001000000000000000000013bu128);
        }
    }
    json!({"count": triples.len(), "fnv1a128": format!("{digest:032x}")})
}

fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    if expected == actual {
        return None;
    }
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            for key in expected
                .keys()
                .chain(actual.keys())
                .collect::<BTreeSet<_>>()
            {
                let child = format!("{path}/{key}");
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(diff) = first_difference(expected, actual, &child) {
                            return Some(diff);
                        }
                    }
                    (expected, actual) => {
                        return Some(format!("{child}: expected {expected:?}, actual {actual:?}"))
                    }
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.iter().chain(actual).all(Value::is_string) {
                let expected: BTreeSet<_> = expected.iter().map(|v| v.as_str().unwrap()).collect();
                let actual: BTreeSet<_> = actual.iter().map(|v| v.as_str().unwrap()).collect();
                let missing: Vec<_> = expected.difference(&actual).take(8).copied().collect();
                let unexpected: Vec<_> = actual.difference(&expected).take(8).copied().collect();
                return Some(format!(
                    "{path}: missing triples {missing:#?}; unexpected triples {unexpected:#?}"
                ));
            }
            for i in 0..expected.len().max(actual.len()) {
                let child = format!("{path}/{i}");
                match (expected.get(i), actual.get(i)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(diff) = first_difference(expected, actual, &child) {
                            return Some(diff);
                        }
                    }
                    (expected, actual) => {
                        return Some(format!("{child}: expected {expected:?}, actual {actual:?}"))
                    }
                }
            }
        }
        _ => return Some(format!("{path}: expected {expected}, actual {actual}")),
    }
    None
}

#[test]
fn small_view_mismatches_show_the_missing_and_unexpected_triples() {
    let missing = "<urn:a> <urn:p> <urn:b>";
    let unexpected = "<urn:a> <urn:p> <urn:c>";
    let expected = compact_view(&[missing.into()]);
    let actual = compact_view(&[unexpected.into()]);
    let diff = first_difference(&expected, &actual, "view").unwrap();
    assert!(diff.contains(missing), "{diff}");
    assert!(diff.contains(unexpected), "{diff}");
    assert!(first_difference(&expected, &expected, "view").is_none());
}

pub(crate) fn assert_golden(name: &str, actual: &Value, expected: &str) {
    if let Some(dir) = std::env::var_os("AUTH_LEDGER_GOLDEN_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(name),
            serde_json::to_string_pretty(actual).unwrap(),
        )
        .unwrap();
    } else {
        let expected: Value = serde_json::from_str(expected).unwrap();
        if name == "auth-ledger-views.json" {
            if let Some(diff) = first_difference(&expected["views"], &actual["views"], "views") {
                panic!("baseline parity: {name}: {diff}");
            }
        }
        if let Some(diff) = first_difference(&expected, actual, name) {
            panic!("baseline parity: {diff}");
        }
    }
}

#[test]
fn installed_views_and_named_requests_match_the_baseline() {
    let mut snapshots = BTreeMap::new();
    // Store each sorted view once: filler content and context often share it.
    let mut unique_views: Vec<BTreeMap<String, Value>> = Vec::new();
    for case in cases() {
        let store = case.run();
        let view = views(store.graph());
        let view_id = unique_views
            .iter()
            .position(|v| *v == view)
            .unwrap_or_else(|| {
                unique_views.push(view);
                unique_views.len() - 1
            });
        let mut decisions: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
        for (agent, client, resource, mode) in &case.requests {
            let session = Session {
                agent: agent.as_deref(),
                client: client.as_deref(),
                ..Session::default()
            };
            let allowed = store
                .accessible(&session, *mode)
                .iter()
                .any(|r| r.as_str() == resource);
            let key = format!("{agent:?}/{client:?}/{mode:?}");
            decisions
                .entry(key)
                .or_default()
                .insert(resource.clone(), allowed);
        }
        let resources: Vec<_> = case
            .requests
            .iter()
            .map(|r| r.2.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let decisions: BTreeMap<_, String> = decisions
            .into_iter()
            .map(|(session, by_resource)| {
                let bits = resources
                    .iter()
                    .map(|resource| match by_resource.get(resource) {
                        Some(true) => '1',
                        Some(false) => '0',
                        None => '-',
                    })
                    .collect();
                (session, bits)
            })
            .collect();
        snapshots.insert(
            case.name,
            json!({"view": view_id, "resources": resources, "decisions": decisions}),
        );
    }
    // Requests remain exact: share repeated resource lists and decision bitmaps.
    let mut resource_lists = Vec::new();
    let mut bitmaps = Vec::new();
    for case in snapshots.values_mut() {
        let resources = case["resources"].take();
        case["resources"] = json!(intern_snapshot(&mut resource_lists, resources));
        for bitmap in case["decisions"].as_object_mut().unwrap().values_mut() {
            let bits = bitmap.take();
            *bitmap = json!(intern_snapshot(&mut bitmaps, bits));
        }
    }
    assert_golden(
        "auth-ledger-views.json",
        &json!({"views":unique_views, "cases":snapshots, "resources":resource_lists, "bitmaps":bitmaps}),
        include_str!("fixtures/auth-ledger-views.json"),
    );
}

#[cfg(feature = "odrl-bridge")]
pub(crate) fn odrl_cases() -> Vec<(String, String, sparq_policy::Request)> {
    let mut out = Vec::new();
    // Reuse every named policy fixture from the existing differential suite.
    let source = include_str!("odrl_n3_differential.rs");
    for part in source.split("const POL_").skip(1) {
        let Some((name, rest)) = part.split_once(':') else {
            continue;
        };
        let Some((_, rest)) = rest.split_once("r#\"") else {
            continue;
        };
        let Some((policy, _)) = rest.split_once("\"#;") else {
            continue;
        };
        for party in ["urn:alice", "urn:bob"] {
            for time in ["2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z"] {
                let request = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
                    .on("urn:t/1")
                    .by(party)
                    .at(time);
                out.push((format!("{name}/{party}/{time}"), policy.into(), request));
            }
        }
    }
    out
}

#[cfg(feature = "odrl-bridge")]
#[test]
fn odrl_views_and_decisions_match_the_baseline() {
    let mut snapshots = BTreeMap::new();
    for (name, policy, request) in odrl_cases() {
        let mut graph = Graph::new();
        let result = sparq_solid::odrl_bridge::materialize_odrl_n3(&mut graph, &policy, &request);
        let outcome = match result {
            Ok(out) => json!({"grant":out.grant_triple,"deny":out.deny_triple}),
            Err(_) => json!({"refused":true}),
        };
        snapshots.insert(name, json!({"outcome":outcome,"views":views(&graph)}));
    }
    assert_golden(
        "auth-ledger-odrl.json",
        &json!(snapshots),
        include_str!("fixtures/auth-ledger-odrl.json"),
    );
}

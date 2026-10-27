//! Baseline parity for installed views, named fixture requests, and closure charges.
//!
//! Source: `81df9457597ef8b81c384360349c94ef956b5220` (origin/main).
//! Regenerate with `python3 crates/sparq-solid/tests/regenerate_auth_ledger.py OUTPUT_DIR`.
//! The script archives that revision, installs this harness and a test-only charge
//! observer, and runs the baseline library tests with AUTH_LEDGER_GOLDEN_DIR set.
//! Review the resulting JSON before replacing the committed fixtures. Never capture
//! expected data from the implementation being checked.
//! Small views retain sorted canonical triples; larger views retain counts and an
//! FNV-1a-128 digest, framing each triple's UTF-8 bytes with its little-endian u64 length.

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
            Err(error) => json!({
                "refused":true,
                "refusal_kind":sparq_solid::auth_allowance_refusal(&error).map(|kind| format!("{kind:?}")),
            }),
        };
        snapshots.insert(name, json!({"outcome":outcome,"views":views(&graph)}));
    }
    let (name, policy, request) = odrl_cases().into_iter().next().unwrap();
    let mut graph = Graph::new();
    let mut budget = sparq_solid::default_auth_budget();
    budget.max_decision_join_rows = 0;
    let error = sparq_solid::odrl_bridge::materialize_odrl_n3_with_budget(
        &mut graph, &policy, &request, budget,
    )
    .unwrap_err();
    snapshots.insert(format!("decision-cap/{name}"), json!({
        "outcome":{
            "refused":true,
            "refusal_kind":sparq_solid::auth_allowance_refusal(&error).map(|kind| format!("{kind:?}")),
        },
        "views":views(&graph),
    }));
    assert_golden(
        "auth-ledger-odrl.json",
        &json!(snapshots),
        include_str!("fixtures/auth-ledger-odrl.json"),
    );
}

fn resource_pod(names: impl IntoIterator<Item = String>) -> String {
    use std::fmt::Write;
    let mut source = String::new();
    for name in names {
        writeln!(source, "<{name}#it> <urn:p> \"v\" <{name}> .").unwrap();
    }
    source
}

fn cap_snapshot(source: &str) -> Value {
    let mut store = PodStore::new(Graph::load_dataset(source, "nquads").unwrap());
    match store.materialize_wac() {
        Ok(_) => json!({"views": views(store.graph())}),
        Err(error) => {
            let kind = sparq_solid::auth_allowance_refusal(&error)
                .unwrap_or_else(|| panic!("unexpected non-allowance refusal: {error}"));
            json!({"refusal_kind":format!("{kind:?}"),"views":views(store.graph())})
        }
    }
}

#[test]
fn near_fact_cap_matches_the_baseline() {
    let cap = sparq_solid::default_auth_budget().max_input_facts;
    let mut snapshots = BTreeMap::new();
    // A flat content resource adds isResource and parent; the root adds one fact.
    for (label, count) in [("below", cap / 2 - 1), ("above", cap / 2)] {
        let source = resource_pod((0..count).map(|i| format!("https://pod.ex/d{i}")));
        snapshots.insert(label, cap_snapshot(&source));
    }
    // An own-ACL row contributes ownership, content and inDoc: three more facts.
    let mut source = resource_pod((0..cap / 2 - 2).map(|i| format!("https://pod.ex/d{i}")));
    source.push_str("<https://pod.ex/.acl#a> <urn:p> \"v\" <https://pod.ex/.acl> .\n");
    snapshots.insert("at", cap_snapshot(&source));
    assert_golden(
        "auth-ledger-fact-cap.json",
        &json!(snapshots),
        include_str!("fixtures/auth-ledger-fact-cap.json"),
    );
}

#[test]
fn near_byte_cap_matches_the_baseline() {
    let cap = sparq_solid::default_auth_budget().max_input_bytes;
    let root = "https://pod.ex/";
    let mut snapshots = BTreeMap::new();
    for (label, target) in [("below", cap - 1), ("at", cap), ("above", cap + 1)] {
        let mut names = Vec::new();
        let mut remaining = target - root.len();
        while remaining > 0 {
            let prefix = format!("{root}{:04}", names.len());
            let bytes = remaining.min(1024);
            names.push(format!("{prefix}{}", "x".repeat(bytes - prefix.len())));
            remaining -= bytes;
        }
        snapshots.insert(label, cap_snapshot(&resource_pod(names)));
    }
    assert_golden(
        "auth-ledger-byte-cap.json",
        &json!(snapshots),
        include_str!("fixtures/auth-ledger-byte-cap.json"),
    );
}

#[test]
fn near_term_cap_matches_the_baseline() {
    let cap = sparq_solid::default_auth_budget().max_input_term_bytes;
    // Literal source sizing includes its fixed node cost and xsd:string datatype.
    let overhead = std::mem::size_of::<Term>() + "http://www.w3.org/2001/XMLSchema#string".len();
    let mut snapshots = BTreeMap::new();
    for (label, size) in [("below", cap - 1), ("at", cap), ("above", cap + 1)] {
        let source = format!(
            "<https://pod.ex/.acl#a> <urn:p> \"{}\" <https://pod.ex/.acl> .",
            "x".repeat(size - overhead),
        );
        snapshots.insert(label, cap_snapshot(&source));
    }
    assert_golden(
        "auth-ledger-term-cap.json",
        &json!(snapshots),
        include_str!("fixtures/auth-ledger-term-cap.json"),
    );
}

#[test]
fn nested_resource_counterexample_view_matches_the_baseline() {
    let mut source = resource_pod((0..6_000).map(|i| format!("https://pod.ex/a{}/d{i}", i / 16)));
    source.push_str(concat!(
        "<https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
    ));
    assert_golden(
        "auth-ledger-nested.json",
        &cap_snapshot(&source),
        include_str!("fixtures/auth-ledger-nested.json"),
    );
}

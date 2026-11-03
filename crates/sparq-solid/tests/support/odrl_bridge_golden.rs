//! Full outcomes and graph snapshots for every N3 call in the existing ODRL corpus.
//! Generated only from pinned main by `regenerate_odrl_bridge.py`.
#![cfg(feature = "odrl-bridge")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_policy::Request;
use sparq_solid::odrl_bridge::BridgeOutcome;
use std::collections::BTreeMap;

pub(crate) fn graph_snapshot(graph: &Graph) -> Value {
    fn triples(graph: &Graph) -> Vec<String> {
        let scan = graph.store().scan(&[None, None, None]);
        let mut out: Vec<_> = scan
            .rows
            .iter()
            .map(|row| {
                let ids = scan.to_spo(row);
                format!(
                    "{} {} {}",
                    graph.dict().term(ids[0]),
                    graph.dict().term(ids[1]),
                    graph.dict().term(ids[2])
                )
            })
            .collect();
        out.sort();
        out
    }
    let mut named = BTreeMap::new();
    for (name, sub) in graph.named() {
        named.insert(name.to_string(), triples(sub));
    }
    json!({"default": triples(graph), "named": named})
}

// oxttl gives anonymous nodes a fresh hexadecimal label on each source load.
// Preserve authored labels and all other wording; rename only generated labels,
// consistently within an outcome. These labels also differ on main versus itself.
fn normalize_generated_nodes(
    value: &mut Value,
    source: &str,
    names: &mut BTreeMap<String, String>,
) {
    match value {
        Value::String(text) => {
            let mut out = String::new();
            let mut rest = text.as_str();
            while let Some(at) = rest.find("_:") {
                out.push_str(&rest[..at]);
                rest = &rest[at..];
                let len = rest[2..].bytes().take_while(u8::is_ascii_hexdigit).count();
                let end = 2 + len;
                if (16..=32).contains(&len) && !source.contains(&rest[..end]) {
                    let next = names.len();
                    let name = names
                        .entry(rest[..end].to_owned())
                        .or_insert_with(|| format!("_:generated{next}"));
                    out.push_str(name);
                    rest = &rest[end..];
                } else {
                    out.push_str("_:");
                    rest = &rest[2..];
                }
            }
            out.push_str(rest);
            *text = out;
        }
        Value::Array(values) => {
            for value in values {
                normalize_generated_nodes(value, source, names);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                normalize_generated_nodes(value, source, names);
            }
        }
        _ => {}
    }
}

fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    if expected == actual {
        return None;
    }
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) if a.keys().eq(b.keys()) => {
            for (key, value) in a {
                if let Some(diff) = first_difference(value, &b[key], &format!("{path}/{key}")) {
                    return Some(diff);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(diff) = first_difference(a, b, &format!("{path}/{index}")) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: expected {expected}, actual {actual}"))
}

pub(crate) fn check(
    policy: &str,
    request: &Request,
    before: Value,
    graph: &Graph,
    result: &Result<BridgeOutcome, String>,
) {
    if !std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with("odrl_bridge::bridge_golden::"))
    {
        return;
    }
    let mut outcome = match result {
        Ok(out) => json!({
            "granted": out.granted, "prohibited": out.prohibited,
            "mode": format!("{:?}", out.mode), "grant": out.grant_triple,
            "deny": out.deny_triple, "refused": out.refused,
            "reasons": out.reasons, "consumed": out.consumed,
            "emitted": out.emitted.iter().map(|t| t.each_ref().map(ToString::to_string)).collect::<Vec<_>>(),
        }),
        Err(err) => {
            json!({"error": err, "kind": format!("{:?}", sparq_solid::auth_allowance_refusal(err))})
        }
    };
    normalize_generated_nodes(&mut outcome, policy, &mut BTreeMap::new());
    let input = json!({"policy": policy, "request": format!("{request:?}"), "before": before});
    let key = serde_json::to_string(&input).unwrap();
    let actual = json!({"input": input, "outcome": outcome, "graph": graph_snapshot(graph)});
    if let Ok(dir) = std::env::var("ODRL_BRIDGE_GOLDEN_DIR") {
        let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{hash:016x}.json")),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    } else {
        static EXPECTED: std::sync::OnceLock<BTreeMap<String, Value>> = std::sync::OnceLock::new();
        let expected = EXPECTED.get_or_init(|| {
            let entries: Vec<Value> =
                serde_json::from_str(include_str!("../fixtures/odrl-bridge-main.json")).unwrap();
            entries
                .into_iter()
                .map(|entry| (serde_json::to_string(&entry["input"]).unwrap(), entry))
                .collect()
        });
        let expected = expected
            .get(&key)
            .unwrap_or_else(|| panic!("uncaptured input: {key}"));
        assert!(
            first_difference(expected, &actual, "").is_none(),
            "bridge differs from pinned main: {}\ninput: {key}",
            first_difference(expected, &actual, "").unwrap()
        );
    }
}

#[path = "../odrl_bridge.rs"]
mod bridge;
#[path = "../odrl_count_bridge.rs"]
mod count_bridge;
#[path = "../odrl_decision_entry_points.rs"]
mod decision_entry_points;
#[path = "../odrl_deny_superset.rs"]
mod deny_superset;
#[path = "../odrl_n3_differential.rs"]
mod differential;
#[path = "../odrl_n3_failopen_regression.rs"]
mod failopen_regression;
#[path = "../odrl_request_decide.rs"]
mod request_decide;

#[test]
fn legacy_reader_term_refusals() {
    let policy = "@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:action o:read; o:target <urn:t>; o:assignee <urn:a> .";
    let request = Request::new("http://www.w3.org/ns/odrl/2/read")
        .on("urn:t")
        .by("urn:a");
    for object in [
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#nil>",
        "\"direction\"@en--ltr",
        "<<( <urn:s> <urn:p> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> )>>",
    ] {
        let source = format!("{policy} <urn:unused> <urn:metadata> {object} .");
        assert!(sparq_solid::odrl_bridge::materialize_odrl_n3(
            &mut Graph::new(),
            &source,
            &request
        )
        .is_err());
    }
    for request in [
        request
            .clone()
            .on("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
        request
            .clone()
            .by("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
        Request::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#nil"),
    ] {
        assert!(
            sparq_solid::odrl_bridge::materialize_odrl_n3(&mut Graph::new(), policy, &request)
                .is_err()
        );
    }
}

pub(crate) fn boundary_policy_source() -> String {
    let cap = sparq_solid::default_auth_budget().max_input_bytes;
    let mut source = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:action o:read; o:target <urn:t>; o:assignee <urn:alice> . <urn:unused> <urn:metadata> \"");
    source.extend(std::iter::repeat_n('x', cap - 16 * 1024));
    source.push_str("\" .\n#");
    source.extend(std::iter::repeat_n(' ', cap - 1 - source.len()));
    source
}

#[test]
fn large_policy_inputs_match_main() {
    let request = Request::new("http://www.w3.org/ns/odrl/2/read")
        .on("urn:t")
        .by("urn:alice");
    let base = "@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:action o:read; o:target <urn:t>; o:assignee <urn:alice> .\n";
    for count in [400, 31_000] {
        let mut source = base.to_owned();
        for i in 0..count {
            source.push_str(&format!("<urn:s{i}> <urn:metadata> <urn:value> .\n"));
        }
        // Use the short predicate/object from the independent size probe.
        source = source
            .replace("<urn:metadata>", "<urn:p>")
            .replace("<urn:value>", "<urn:o>");
        assert!(source.len() < sparq_solid::default_auth_budget().max_input_bytes);
        let out =
            sparq_solid::odrl_bridge::materialize_odrl_n3(&mut Graph::new(), &source, &request)
                .unwrap();
        assert!(out.granted);
    }
    let out = sparq_solid::odrl_bridge::materialize_odrl_n3(
        &mut Graph::new(),
        &boundary_policy_source(),
        &request,
    )
    .unwrap();
    assert!(out.granted);
    let long_party = format!("urn:{}", "a".repeat(5000));
    let source = base.replace("urn:alice", &long_party);
    let out = sparq_solid::odrl_bridge::materialize_odrl_n3(
        &mut Graph::new(),
        &source,
        &request.by(long_party),
    )
    .unwrap();
    assert!(out.granted);
}

pub(crate) fn source_only_policy_cases() -> Vec<String> {
    let base = "@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:action o:read; o:target <urn:t>; o:assignee <urn:alice> .\n";
    let mut cases = Vec::new();
    let mut source = format!("{base}<urn:unused> <urn:metadata> ");
    for i in 0..10_000 {
        if i > 0 {
            source.push_str(", ");
        }
        source.push_str(&format!("\"v{i:05}{}\"", "x".repeat(90)));
    }
    source.push_str(" .\n#");
    let cap = sparq_solid::default_auth_budget().max_input_bytes;
    source.extend(std::iter::repeat_n(' ', cap - 1 - source.len()));
    cases.push(source);
    for language in [false, true] {
        let mut source = format!("{base}<urn:unused> <urn:metadata> ");
        for i in 0..20_000 {
            if i > 0 {
                source.push_str(", ");
            }
            if language {
                source.push_str(&format!("\"word{i:05}{}\"@en", "x".repeat(25)));
            } else {
                source.push_str(&format!("10000000000000000000000000000000{i:08}"));
            }
        }
        source.push_str(" .");
        cases.push(source);
    }
    let mut source = format!(
        "{base}@prefix l: <urn:long:{}:> . l:s l:p ",
        "x".repeat(120)
    );
    for i in 0..17_000 {
        if i > 0 {
            source.push_str(", ");
        }
        source.push_str(&format!("l:o{i}"));
    }
    source.push_str(" .");
    cases.push(source);
    for distinct in [true, false] {
        let mut source = format!("{base}<urn:unused> <urn:metadata> ");
        for i in 0..40_000 {
            if i > 0 {
                source.push_str(", ");
            }
            source.push_str(&format!("<urn:o{}>", if distinct { i } else { 0 }));
        }
        source.push_str(" .");
        cases.push(source);
    }
    assert!(cases.iter().all(|source| source.len() <= cap));
    cases
}

#[test]
fn source_only_policy_inputs_match_main() {
    let request = Request::new("http://www.w3.org/ns/odrl/2/read")
        .on("urn:t")
        .by("urn:alice");
    for source in source_only_policy_cases() {
        let result =
            sparq_solid::odrl_bridge::materialize_odrl_n3(&mut Graph::new(), &source, &request);
        assert!(result.as_ref().is_ok_and(|out| out.granted), "{result:?}");
    }
}

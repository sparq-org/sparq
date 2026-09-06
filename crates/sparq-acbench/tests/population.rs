//! [GPT-6] Determinism, bounded writing, empirical inputs, and independent policy parity.

use oxttl::NQuadsParser;
use sparq_acbench::population::{
    PolicyModel, PopulationConfig, Service, VOCAB, can_read, expected_record_count, owner_webid,
    pod_root, recipient_webid, write_pod, write_readable_content,
};
use std::collections::BTreeSet;
use std::io::{self, Write};

// Reuse the independently implemented, procedural policy-language references. These
// source modules depend only on std; no authorization/query-engine code is linked.
#[path = "../../sparq-solid/tests/reference/mod.rs"]
mod reference;

fn render(config: &PopulationConfig, pod: u64, model: PolicyModel) -> String {
    let mut data = Vec::new();
    let summary = write_pod(config, pod, model, &mut data).unwrap();
    assert_eq!(summary.bytes, data.len() as u64);
    let text = String::from_utf8(data).unwrap();
    let parsed: Vec<_> = NQuadsParser::new()
        .for_slice(text.as_bytes())
        .map(Result::unwrap)
        .collect();
    assert_eq!(summary.quads, parsed.len() as u64);
    let records = parsed
        .iter()
        .filter(|q| q.object.to_string() == format!("<{VOCAB}Record>"))
        .count();
    assert_eq!(summary.records, records as u64);
    text
}

fn iri_projection(source: &str) -> String {
    // The procedural reference parser intentionally accepts only plain literals.
    // Policy semantics use IRI objects exclusively in this profile. Retain all
    // IRI-object quads, including every content graph's Record type marker; typed
    // record values are already checked by the independent oxttl parser above.
    source
        .lines()
        .filter(|line| !line.contains(" \""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn population_is_order_independent_and_models_share_identical_content() {
    let config = PopulationConfig::smoke();
    let a = render(&config, 7, PolicyModel::Wac);
    render(&config, 1_999_999, PolicyModel::Wac);
    assert_eq!(a, render(&config, 7, PolicyModel::Wac));
    let b = render(&config, 7, PolicyModel::Acp);
    let content = |source: &str| {
        source
            .lines()
            .filter(|line| !line.contains(".acl") && !line.contains(".acr"))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(content(&a), content(&b));
    let owner = owner_webid(&config, 7);
    assert_eq!(expected_record_count(&config, 7, Some(&owner)).unwrap(), 48);
    assert_eq!(expected_record_count(&config, 7, None).unwrap(), 8);
}

#[test]
fn both_policy_languages_match_independent_intent_for_all_principals_and_modes() {
    let config = PopulationConfig::smoke();
    for pod in [0, 17] {
        let wac = reference::wac::WacModel::from_nquads(&iri_projection(&render(
            &config,
            pod,
            PolicyModel::Wac,
        )))
        .unwrap();
        let acp = reference::acp::AcpModel::from_nquads(&iri_projection(&render(
            &config,
            pod,
            PolicyModel::Acp,
        )))
        .unwrap();
        let owner = owner_webid(&config, pod);
        let mut agents = vec![
            None,
            Some(owner.clone()),
            Some(owner_webid(&config, pod + 1)),
        ];
        agents.extend((0..=config.group_members).map(|i| Some(recipient_webid(&config, pod, i))));
        for service in Service::ALL {
            for month in 0..if service == Service::Contacts {
                1
            } else {
                config.history_months
            } {
                let resource = format!(
                    "{}{}/m{month:04}.ttl",
                    pod_root(&config, pod),
                    service.name()
                );
                for agent in &agents {
                    for mode in [
                        reference::RefMode::Read,
                        reference::RefMode::Write,
                        reference::RefMode::Append,
                        reference::RefMode::Control,
                    ] {
                        let expected = if mode == reference::RefMode::Read {
                            can_read(&config, pod, service, month, agent.as_deref())
                        } else {
                            agent.as_deref() == Some(owner.as_str())
                        };
                        let expected = if expected {
                            reference::RefDecision::Allow
                        } else {
                            reference::RefDecision::Deny
                        };
                        assert_eq!(
                            wac.decide(&reference::wac::Request {
                                agent: agent.as_deref(),
                                client: None,
                                mode,
                                resource: &resource
                            }),
                            expected,
                            "WAC {pod} {resource} {agent:?} {mode:?}"
                        );
                        assert_eq!(
                            acp.decide(&reference::acp::Request {
                                agent: agent.as_deref(),
                                client: None,
                                mode,
                                resource: &resource
                            }),
                            expected,
                            "ACP {pod} {resource} {agent:?} {mode:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn physically_filtered_records_match_count_oracle() {
    let config = PopulationConfig::smoke();
    for agent in [
        None,
        Some(owner_webid(&config, 0)),
        Some(recipient_webid(&config, 0, 0)),
        Some(recipient_webid(&config, 0, 2)),
    ] {
        let mut output = Vec::new();
        let count = write_readable_content(&config, 0, agent.as_deref(), &mut output).unwrap();
        assert_eq!(
            count,
            expected_record_count(&config, 0, agent.as_deref()).unwrap()
        );
        let parsed = NQuadsParser::new()
            .for_slice(&output)
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let actual = parsed
            .iter()
            .filter(|q| q.object.to_string() == format!("<{VOCAB}Record>"))
            .count();
        assert_eq!(actual as u64, count);
    }
}

#[test]
fn empirical_aggregate_and_config_round_trip_are_stable() {
    let config = PopulationConfig::service_history();
    config.validate().unwrap();
    assert_eq!(config.rating_count_cdf.last(), Some(&(2314, 6040)));
    let mut previous = 0;
    let mut ratings = 0_u64;
    for &(count, cumulative) in &config.rating_count_cdf {
        ratings += u64::from(count) * u64::from(cumulative - previous);
        previous = cumulative;
    }
    assert_eq!(ratings, 1_000_209);
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(config, serde_json::from_str(&json).unwrap());
    assert!(
        serde_json::from_str::<PopulationConfig>(&json.replace("\"seed\"", "\"sead\"")).is_err()
    );
}

struct RejectLargeWrites {
    calls: usize,
}
impl Write for RejectLargeWrites {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(
            bytes.len() < 2048,
            "whole-Pod buffering must not reach writer"
        );
        self.calls += 1;
        if self.calls == 20 {
            return Err(io::Error::other("simulated full disk"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn streaming_propagates_storage_failure_and_rejects_bad_config_before_writing() {
    let mut writer = RejectLargeWrites { calls: 0 };
    assert!(
        write_pod(
            &PopulationConfig::service_history(),
            0,
            PolicyModel::Wac,
            &mut writer
        )
        .is_err()
    );
    assert_eq!(writer.calls, 20);
    let mut config = PopulationConfig::smoke();
    config.history_months = 0;
    assert!(write_pod(&config, 0, PolicyModel::Wac, &mut writer).is_err());
    assert_eq!(writer.calls, 20);
}

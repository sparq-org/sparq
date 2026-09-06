//! [GPT-6] Cross-check mutation addresses against independently parsed corpus records.

use oxrdf::{GraphName, Term};
use oxttl::NQuadsParser;
use sparq_acbench::population::mutation::{
    MAX_BATCH_RECORDS, PopulationMutationRequest as Request, emit_mutation_batch,
    existing_record_refs,
};
use sparq_acbench::population::{
    LiteralProfile, PolicyModel, PopulationConfig, Service, VOCAB, planned_record_counts, write_pod,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Observed {
    graph: String,
    service: String,
    created: String,
    sequence: u64,
    value: u64,
    triples: u64,
}

fn records(source: &[u8]) -> BTreeMap<String, Observed> {
    let mut records = BTreeMap::<String, Observed>::new();
    for quad in NQuadsParser::new().for_slice(source).map(Result::unwrap) {
        let subject = quad.subject.to_string();
        if !subject.contains(".ttl#r") {
            continue;
        }
        let record = records
            .entry(subject.trim_matches(['<', '>']).into())
            .or_default();
        let GraphName::NamedNode(graph) = quad.graph_name else {
            panic!("record outside named graph")
        };
        record.graph = graph.as_str().into();
        record.triples += 1;
        let predicate = quad.predicate.as_str();
        match quad.object {
            Term::NamedNode(value) if predicate == format!("{VOCAB}service") => {
                record.service = value.as_str().into();
            }
            Term::Literal(value) if predicate == format!("{VOCAB}created") => {
                record.created = value.value().into();
            }
            Term::Literal(value) if predicate == format!("{VOCAB}sequence") => {
                record.sequence = value.value().parse().unwrap();
            }
            Term::Literal(value) if predicate == format!("{VOCAB}value") => {
                record.value = value.value().parse().unwrap();
            }
            _ => {}
        }
    }
    records
}

#[test]
fn chronological_pages_match_parsed_dates_and_cover_each_record_once() {
    let mut config = PopulationConfig::smoke();
    config.history_months = 3;
    config.monthly_records.communication = 61;
    let mut source = Vec::new();
    write_pod(&config, 29, PolicyModel::Wac, &mut source).unwrap();
    let observed = records(&source);
    for service in Service::ALL {
        let mut expected: Vec<_> = observed
            .iter()
            .filter(|(_, r)| r.service == format!("{VOCAB}{}", service.name()))
            .collect();
        expected.sort_by_key(|(_, r)| (&r.created, r.sequence));
        let mut actual = Vec::new();
        for offset in (0..expected.len()).step_by(7) {
            actual.extend(existing_record_refs(&config, 29, service, offset as u64, 7).unwrap());
        }
        assert_eq!(actual.len(), expected.len());
        for (address, (subject, parsed)) in actual.iter().zip(expected) {
            assert_eq!(&address.subject, subject);
            assert_eq!(address.graph, parsed.graph);
            assert_eq!(address.created, parsed.created);
            assert_eq!(address.record_number, parsed.sequence);
            assert_eq!(address.value, parsed.value);
            assert_eq!(address.triples, parsed.triples);
        }
    }
    let first = existing_record_refs(&config, 29, Service::Communication, 0, 4).unwrap();
    assert_eq!(
        first.iter().map(|r| r.record_number).collect::<Vec<_>>(),
        [0, 28, 56, 1]
    );
}

#[test]
fn insertion_reuses_full_schema_and_literal_profile_without_baseline_collisions() {
    let mut config = PopulationConfig::smoke();
    config.literal_profile = LiteralProfile::Seeded {
        message_text_bytes: 1024,
        short_text_bytes: 64,
    };
    let counts = planned_record_counts(&config, 7).unwrap();
    for service in Service::ALL {
        let request = Request::Insert {
            batch_id: 2,
            count: 8,
        };
        let batch = emit_mutation_batch(&config, 7, service, request).unwrap();
        assert_eq!(
            batch,
            emit_mutation_batch(&config, 7, service, request).unwrap()
        );
        let observed = records(batch.inserted_nquads.as_bytes());
        assert_eq!(observed.len(), 8);
        assert_eq!(
            batch.expected_inserted_triples,
            observed.values().map(|r| r.triples).sum::<u64>()
        );
        assert_eq!(batch.expected_deleted_triples, 0);
        for address in &batch.record_refs {
            let record = &observed[&address.subject];
            assert_eq!(record.service, format!("{VOCAB}{}", service.name()));
            assert_eq!(record.created, address.created);
            assert!(record.created.starts_with("2026-01-"));
            assert_eq!(record.triples, address.triples);
            assert_eq!(record.value, address.value);
            assert!(record.sequence >= counts[&service]);
        }
        if service == Service::Communication {
            let text_lengths: Vec<_> = NQuadsParser::new()
                .for_slice(batch.inserted_nquads.as_bytes())
                .map(Result::unwrap)
                .filter(|q| q.predicate.as_str() == format!("{VOCAB}text"))
                .map(|q| match q.object {
                    Term::Literal(t) => t.value().len(),
                    _ => panic!("nonliteral text"),
                })
                .collect();
            assert_eq!(text_lengths, [1024; 8]);
        }
        let next = emit_mutation_batch(
            &config,
            7,
            service,
            Request::Insert {
                batch_id: 3,
                count: 1,
            },
        )
        .unwrap();
        let subjects: BTreeSet<_> = batch.record_refs.iter().map(|r| &r.subject).collect();
        assert!(!subjects.contains(&next.record_refs[0].subject));
    }
}

#[test]
fn sparse_service_ingests_into_its_latest_populated_graph() {
    let mut config = PopulationConfig::smoke();
    config.history_months = 60;
    let batch = emit_mutation_batch(
        &config,
        0,
        Service::Ratings,
        Request::Insert {
            batch_id: 0,
            count: 1,
        },
    )
    .unwrap();
    assert!(batch.record_refs[0].graph.ends_with("ratings/m0007.ttl"));
    config.monthly_records.communication = 0;
    assert!(
        emit_mutation_batch(
            &config,
            0,
            Service::Communication,
            Request::Insert {
                batch_id: 0,
                count: 1
            }
        )
        .is_err()
    );
}

#[test]
fn expiry_and_edit_address_current_state_and_report_finite_exhaustion() {
    let config = PopulationConfig::smoke();
    let delete = emit_mutation_batch(
        &config,
        0,
        Service::Media,
        Request::Delete {
            offset: 4,
            count: 8,
        },
    )
    .unwrap();
    assert_eq!(delete.record_refs.len(), 2);
    assert_eq!(
        delete.expected_deleted_triples,
        delete.record_refs.iter().map(|r| r.triples).sum::<u64>()
    );
    assert_eq!(delete.sparql.matches("?p ?o").count(), 2);
    assert!(!delete.sparql.contains("DELETE DATA"));
    let edit = emit_mutation_batch(
        &config,
        0,
        Service::Media,
        Request::Modify {
            offset: 4,
            count: 8,
            revision: 1,
        },
    )
    .unwrap();
    assert_eq!(edit.record_refs, delete.record_refs);
    assert_eq!(edit.expected_inserted_triples, 2);
    assert_eq!(edit.expected_deleted_triples, 2);
    assert!(edit.sparql.contains("\"10001\"^^"));
    assert!(edit.sparql.contains("?old"));
    let exhausted = emit_mutation_batch(
        &config,
        0,
        Service::Media,
        Request::Delete {
            offset: 6,
            count: 1,
        },
    )
    .unwrap();
    assert!(exhausted.sparql.is_empty());
    assert!(exhausted.record_refs.is_empty());
    assert_eq!(exhausted.expected_deleted_triples, 0);
}

#[test]
fn batch_and_payload_bounds_reject_oversized_or_overflowing_requests() {
    let mut config = PopulationConfig::smoke();
    for request in [
        Request::Insert {
            batch_id: u64::MAX,
            count: 1,
        },
        Request::Insert {
            batch_id: 0,
            count: MAX_BATCH_RECORDS + 1,
        },
        Request::Delete {
            offset: 0,
            count: MAX_BATCH_RECORDS + 1,
        },
        Request::Modify {
            offset: 0,
            count: 1,
            revision: 0,
        },
        Request::Modify {
            offset: 0,
            count: 1,
            revision: u64::MAX,
        },
    ] {
        assert!(emit_mutation_batch(&config, 0, Service::Communication, request).is_err());
    }
    assert!(
        existing_record_refs(&config, 0, Service::Media, u64::MAX, 8)
            .unwrap()
            .is_empty()
    );
    config.literal_profile = LiteralProfile::Seeded {
        message_text_bytes: 65_536,
        short_text_bytes: 64,
    };
    assert!(
        emit_mutation_batch(
            &config,
            0,
            Service::Communication,
            Request::Insert {
                batch_id: 0,
                count: 257
            }
        )
        .is_err()
    );
}

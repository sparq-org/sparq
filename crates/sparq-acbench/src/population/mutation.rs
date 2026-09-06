//! Build bounded, deterministic content mutations from the population record schema.
//!
//! [GPT-6] This module plans requests, not observed state transitions. Expected deltas
//! assume the selected records exist and must be checked against acknowledged server
//! changes. It neither changes policies nor infers empirical mutation frequencies.

use super::{
    Output, PopulationConfig, Service, VOCAB, months, owner_webid, pod_root, record_counts,
    record_date, record_value, records_in_month, write_record,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

/// Maximum records requested by one helper call; HTTP limits may require smaller batches.
pub const MAX_BATCH_RECORDS: u32 = 1024;
/// Maximum generated N-Quads or SPARQL bytes in one returned field.
pub const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024;

/// A stable baseline record address and its original scalar observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationRecordRef {
    /// Named content graph containing the record.
    pub graph: String,
    /// Record subject, unique within the Pod.
    pub subject: String,
    /// Global sequence within this Pod's service; distinct from chronological position.
    pub record_number: u64,
    /// Frozen scenario creation timestamp, in UTC.
    pub created: String,
    /// Original `p:value` integer; subsequent modifications can change it.
    pub value: u64,
    /// Outgoing record triples, including its type marker and relationship links.
    pub triples: u64,
}

/// A bounded content mutation; offsets refer to chronological baseline records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PopulationMutationRequest {
    /// Add full records in the latest populated service graph.
    Insert {
        /// Must be unique per Pod and service across the run and any resumed run.
        batch_id: u64,
        /// Number of new records, at most `MAX_BATCH_RECORDS`.
        count: u32,
    },
    /// Remove all current outgoing triples of selected baseline records.
    Delete {
        /// Number of baseline records to skip in chronological order.
        offset: u64,
        /// Maximum records to remove; the finite baseline can supply fewer.
        count: u32,
    },
    /// Replace each selected record's value while preserving its other fields.
    Modify {
        /// Number of baseline records to skip in chronological order.
        offset: u64,
        /// Maximum records to modify; the finite baseline can supply fewer.
        count: u32,
        /// Positive replacement revision; fixed replay is intentionally idempotent.
        revision: u64,
    },
}

/// A generated request and reference expectations, not measured mutation counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationMutation {
    /// Service whose records are addressed.
    pub service: Service,
    /// Reproducible request specification.
    pub request: PopulationMutationRequest,
    /// SPARQL update; empty when the requested finite baseline is exhausted.
    pub sparql: String,
    /// Addressed records; their values describe the baseline or inserted records.
    pub record_refs: Vec<PopulationRecordRef>,
    /// Full newly ingested records for an independent stateful content oracle.
    /// Empty for deletion and modification; those operate on current oracle state.
    pub inserted_nquads: String,
    /// Intended inserted triples; actual changes require server-side observation.
    pub expected_inserted_triples: u64,
    /// Intended deleted triples; absent records and repeated requests can change this.
    pub expected_deleted_triples: u64,
}

/// Select existing records by creation time without generating or sorting the corpus.
///
/// Dates in the fixture cycle over 28 days within each month. Selection orders by
/// `(created, numeric record sequence)`, not by the lexical order of the subject IRI.
/// Work is bounded by history months, 28 day buckets per visited month, and `count`.
/// Offsets count baseline records even if a previous request deleted them. Callers
/// must maintain distinct expiry cursors and record finite-run exhaustion explicitly.
/// Contacts are a snapshot; their expiry rate is a workload decision, not this API's.
///
/// # Errors
/// Rejects invalid configuration and requests above the fixed batch bound.
pub fn existing_record_refs(
    config: &PopulationConfig,
    pod_id: u64,
    service: Service,
    mut offset: u64,
    count: u32,
) -> io::Result<Vec<PopulationRecordRef>> {
    config.validate()?;
    validate_count(count)?;
    let counts = record_counts(config, pod_id);
    let total = counts[&service];
    let root = pod_root(config, pod_id);
    let owner = owner_webid(config, pod_id);
    let mut refs = Vec::new();
    let mut first_record = 0;
    for month in 0..months(config, service) {
        let n = records_in_month(config, service, total, month);
        if offset >= n {
            offset -= n;
            first_record += n;
            continue;
        }
        let graph = format!("{root}{}/m{month:04}.ttl", service.name());
        for day in 0..28_u64 {
            let on_day = if day < n { (n - 1 - day) / 28 + 1 } else { 0 };
            if offset >= on_day {
                offset -= on_day;
                continue;
            }
            for within_day in offset..on_day {
                if refs.len() == count as usize {
                    return Ok(refs);
                }
                let local = day + 28 * within_day;
                refs.push(record_ref(
                    config,
                    &root,
                    &owner,
                    &graph,
                    service,
                    first_record + local,
                    month,
                    local,
                    counts[&Service::Contacts],
                )?);
            }
            offset = 0;
        }
        first_record += n;
    }
    Ok(refs)
}

/// Emit an ingestion, expiry, or value-edit request using the shared record emitter.
///
/// Ingestion uses the latest populated graph, preserving its existing permissions,
/// with January 2026 virtual creation dates. No container or policy changes are hidden
/// in the request. Subject sequence is `baseline_total + batch_id * 1024 + local`,
/// reserving disjoint ranges regardless of batch size. A missing service collection
/// is rejected rather than inventing a new ungoverned resource.
///
/// Deletion removes current outgoing triples, including previously modified values.
/// Modification replaces `p:value` with `10000 + revision`, so it differs from every
/// baseline value. Both use explicit graph/subject patterns and produce no orphan
/// value when the selected record is already absent. Referential cleanup of links
/// from other records is outside this operation's declared semantics.
///
/// # Errors
/// Rejects invalid configurations, oversized batches or payloads, numeric overflow,
/// revision zero, and ingestion into services with no populated graph.
pub fn emit_mutation_batch(
    config: &PopulationConfig,
    pod_id: u64,
    service: Service,
    request: PopulationMutationRequest,
) -> io::Result<PopulationMutation> {
    config.validate()?;
    let mut result = PopulationMutation {
        service,
        request,
        sparql: String::new(),
        record_refs: Vec::new(),
        inserted_nquads: String::new(),
        expected_inserted_triples: 0,
        expected_deleted_triples: 0,
    };
    match request {
        PopulationMutationRequest::Insert { batch_id, count } => {
            insert_batch(config, pod_id, service, batch_id, count, &mut result)?;
        }
        PopulationMutationRequest::Delete { offset, count } => {
            result.record_refs = existing_record_refs(config, pod_id, service, offset, count)?;
            for record in &result.record_refs {
                append_update(
                    &mut result.sparql,
                    &format!(
                        "DELETE WHERE {{ GRAPH <{}> {{ <{}> ?p ?o }} }}",
                        record.graph, record.subject
                    ),
                )?;
                result.expected_deleted_triples += record.triples;
            }
        }
        PopulationMutationRequest::Modify {
            offset,
            count,
            revision,
        } => {
            let value = revision
                .checked_add(10_000)
                .filter(|_| revision > 0)
                .ok_or_else(|| invalid("revision must be positive and fit the integer domain"))?;
            result.record_refs = existing_record_refs(config, pod_id, service, offset, count)?;
            for record in &result.record_refs {
                let g = &record.graph;
                let s = &record.subject;
                append_update(
                    &mut result.sparql,
                    &format!(
                        "DELETE {{ GRAPH <{g}> {{ <{s}> <{VOCAB}value> ?old }} }} INSERT {{ GRAPH <{g}> {{ <{s}> <{VOCAB}value> \"{value}\"^^<http://www.w3.org/2001/XMLSchema#integer> }} }} WHERE {{ GRAPH <{g}> {{ <{s}> a <{VOCAB}Record>; <{VOCAB}value> ?old }} }}"
                    ),
                )?;
            }
            result.expected_inserted_triples = result.record_refs.len() as u64;
            result.expected_deleted_triples = result.record_refs.len() as u64;
        }
    }
    Ok(result)
}

fn validate_count(count: u32) -> io::Result<()> {
    if count > MAX_BATCH_RECORDS {
        return Err(invalid("mutation count exceeds MAX_BATCH_RECORDS"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn append_update(output: &mut String, next: &str) -> io::Result<()> {
    if output.len().saturating_add(next.len()).saturating_add(2) > MAX_BATCH_BYTES {
        return Err(invalid("mutation SPARQL exceeds MAX_BATCH_BYTES"));
    }
    if !output.is_empty() {
        output.push_str(";\n");
    }
    output.push_str(next);
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "bounded scalar address into the shared record emitter"
)]
fn record_ref(
    config: &PopulationConfig,
    root: &str,
    owner: &str,
    graph: &str,
    service: Service,
    record: u64,
    month: u32,
    local: u64,
    contacts: u64,
) -> io::Result<PopulationRecordRef> {
    let mut out = Output {
        writer: io::sink(),
        bytes: 0,
        quads: 0,
    };
    write_record(
        &mut out, config, root, owner, graph, service, record, month, local, contacts,
    )?;
    Ok(PopulationRecordRef {
        graph: graph.into(),
        subject: format!("{graph}#r{record}"),
        record_number: record,
        created: record_date(config, month, local),
        value: record_value(config, root, service, record, month) % 10_000,
        triples: out.quads,
    })
}

fn insert_batch(
    config: &PopulationConfig,
    pod_id: u64,
    service: Service,
    batch_id: u64,
    count: u32,
    result: &mut PopulationMutation,
) -> io::Result<()> {
    validate_count(count)?;
    if count == 0 {
        return Ok(());
    }
    let counts = record_counts(config, pod_id);
    let total = counts[&service];
    if total == 0 {
        return Err(invalid(
            "ingestion requires an already populated service graph",
        ));
    }
    let first = batch_id
        .checked_mul(u64::from(MAX_BATCH_RECORDS))
        .and_then(|n| n.checked_add(total))
        .filter(|n| n.checked_add(u64::from(count) - 1).is_some())
        .ok_or_else(|| invalid("ingestion record sequence overflow"))?;
    let latest = u64::from(months(config, service)).min(total) - 1;
    let root = pod_root(config, pod_id);
    let owner = owner_webid(config, pod_id);
    let graph = format!("{root}{}/m{latest:04}.ttl", service.name());
    let mut out = Output {
        writer: BoundedBuffer(Vec::new()),
        bytes: 0,
        quads: 0,
    };
    for local in 0..u64::from(count) {
        let record = first + local;
        write_record(
            &mut out,
            config,
            &root,
            &owner,
            &graph,
            service,
            record,
            config.history_months,
            local,
            counts[&Service::Contacts],
        )?;
        result.record_refs.push(record_ref(
            config,
            &root,
            &owner,
            &graph,
            service,
            record,
            config.history_months,
            local,
            counts[&Service::Contacts],
        )?);
    }
    result.expected_inserted_triples = out.quads;
    result.inserted_nquads =
        String::from_utf8(out.writer.0).map_err(|_| invalid("generated N-Quads were not UTF-8"))?;
    let suffix = format!(" <{graph}> .");
    let mut update = format!("INSERT DATA {{ GRAPH <{graph}> {{\n");
    for line in result.inserted_nquads.lines() {
        let triple = line
            .strip_suffix(&suffix)
            .ok_or_else(|| invalid("generated N-Quads graph differs from ingestion graph"))?;
        if update.len().saturating_add(triple.len()).saturating_add(8) > MAX_BATCH_BYTES {
            return Err(invalid("mutation SPARQL exceeds MAX_BATCH_BYTES"));
        }
        update.push_str(triple);
        update.push_str(" .\n");
    }
    update.push_str("} }");
    result.sparql = update;
    Ok(())
}

struct BoundedBuffer(Vec<u8>);

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BATCH_BYTES {
            return Err(invalid("mutation N-Quads exceeds MAX_BATCH_BYTES"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

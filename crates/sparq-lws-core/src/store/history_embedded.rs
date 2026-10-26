//! History transactions executed as one embedded actor job and one journal commit.
use super::*;
use crate::store::history::{self, TrashEntry, TrashFields, VersionEntry, VersionFields};
use std::time::SystemTime;

fn field_update(member: &str, fields: &[(&str, String)]) -> Result<String, SparqError> {
    let triples = fields
        .iter()
        .map(|(p, v)| {
            format!(
                "<{}> <{}{p}> {} .",
                sparql::s_record(),
                sparql::PSS_NS,
                sparql::literal(v)
            )
        })
        .collect::<String>();
    Ok(format!(
        "INSERT DATA {{ GRAPH {} {{ {triples} }} }}",
        sparql::iri(member)?
    ))
}
fn meta_update(member: &str, meta: &ResourceMeta) -> Result<String, SparqError> {
    sparql::update_put_meta(
        member,
        &meta.content_type,
        &meta.blob_key,
        &meta.etag,
        meta.last_modified
            .and_then(timestamp::to_xsd_datetime)
            .as_deref(),
    )
    .map_err(Into::into)
}
fn copy_body(from: &str, to: &str) -> Result<String, SparqError> {
    Ok(format!("INSERT {{ GRAPH {} {{ ?s ?p ?o }} }} WHERE {{ GRAPH {} {{ ?s ?p ?o }} FILTER (?s != <{}> && !STRSTARTS(STR(?p), {})) }}",sparql::iri(to)?,sparql::iri(from)?,sparql::s_record(),sparql::literal(sparql::PSS_NS)))
}
fn records(
    graph: &sparq_core::Graph,
    trash: bool,
    original: Option<&str>,
    before: Option<SystemTime>,
    limit: Option<usize>,
    member: Option<&str>,
) -> Result<sparq_engine::QueryResult, SparqError> {
    let prefix = if trash {
        history::TRASH
    } else {
        history::VERSIONS
    };
    let filter = original
        .map(|o| {
            format!(
                "FILTER STRSTARTS(STR(?member), {})",
                sparql::literal(&history::version_dir(o))
            )
        })
        .unwrap_or_default();
    let extra = if trash {
        "?s <urn:pss:index#trashedAt> ?at ; <urn:pss:index#deleterCouldRead> ?read . OPTIONAL { ?s <urn:pss:index#deletedBy> ?by }"
    } else {
        "?s <urn:pss:index#commitOrder> ?order ."
    };
    let named_graph = member
        .map(sparql::iri)
        .transpose()?
        .unwrap_or_else(|| "?member".into());
    let bind = member
        .map(|m| sparql::iri(m).map(|iri| format!("BIND({iri} AS ?member)")))
        .transpose()?
        .unwrap_or_default();
    let q = format!("SELECT ?member ?original ?ct ?bk ?etag ?mod ?order ?at ?read ?by WHERE {{ {bind} GRAPH {named_graph} {{ ?s <urn:pss:index#original> ?original ; <urn:pss:index#contentType> ?ct ; <urn:pss:index#blobKey> ?bk ; <urn:pss:index#etag> ?etag . OPTIONAL {{ ?s <urn:pss:index#modified> ?mod }} {extra} }} FILTER STRSTARTS(STR(?member), {}) {filter} }}", sparql::literal(prefix));
    let mut q = q;
    if let Some(before) = before {
        let date = timestamp::to_xsd_datetime(before)
            .ok_or_else(|| SparqError::Backend("invalid expiry cutoff".into()))?;
        let field = if trash { "at" } else { "mod" };
        let value = if trash {
            sparql::literal(&date)
        } else {
            format!("{}^^<{}>", sparql::literal(&date), sparql::XSD_DATETIME)
        };
        // The outer query bounds the rows transferred out of the index.
        q = format!("SELECT * WHERE {{ {{ {q} }} FILTER (?{field} <= {value}) }} ORDER BY ?{field} ?order ?member");
    }
    if let Some(limit) = limit {
        q.push_str(&format!(" LIMIT {limit}"));
    }
    sparq_engine::query(graph, &q).map_err(|e| engine_err("history/read", e))
}
fn cell(r: &sparq_engine::QueryResult, row: &[Option<oxrdf::Term>], name: &str) -> Option<String> {
    var_col(r, name).and_then(|c| term_value(row.get(c).and_then(Option::as_ref)))
}
fn row_meta(
    r: &sparq_engine::QueryResult,
    row: &[Option<oxrdf::Term>],
) -> Result<ResourceMeta, SparqError> {
    let required = |name| {
        cell(r, row, name).ok_or_else(|| SparqError::Backend("incomplete history record".into()))
    };
    Ok(ResourceMeta {
        content_type: required("ct")?,
        blob_key: required("bk")?,
        etag: required("etag")?,
        last_modified: cell(r, row, "mod").and_then(|s| timestamp::from_xsd_datetime(&s)),
    })
}
pub(super) fn versions(
    graph: &sparq_core::Graph,
    original: Option<&str>,
) -> Result<Vec<VersionEntry>, SparqError> {
    versions_selected(graph, original, None)
}
pub(super) fn version_entry(
    graph: &sparq_core::Graph,
    member: &str,
) -> Result<Option<VersionEntry>, SparqError> {
    Ok(versions_selected(graph, None, Some(member))?
        .into_iter()
        .next())
}
fn versions_selected(
    graph: &sparq_core::Graph,
    original: Option<&str>,
    member: Option<&str>,
) -> Result<Vec<VersionEntry>, SparqError> {
    let r = records(graph, false, original, None, None, member)?;
    let mut entries = Vec::new();
    for row in &r.rows {
        entries.push(VersionEntry {
            member: cell(&r, row, "member").ok_or(SparqError::NotFound)?,
            original: cell(&r, row, "original").ok_or(SparqError::NotFound)?,
            meta: row_meta(&r, row)?,
            order: cell(&r, row, "order")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| SparqError::Backend("invalid history order".into()))?,
        });
    }
    history::sort_versions(&mut entries);
    Ok(entries)
}
pub(super) fn trash(graph: &sparq_core::Graph) -> Result<Vec<TrashEntry>, SparqError> {
    trash_selected(graph, None)
}
pub(super) fn trash_entry(
    graph: &sparq_core::Graph,
    member: &str,
) -> Result<Option<TrashEntry>, SparqError> {
    Ok(trash_selected(graph, Some(member))?.into_iter().next())
}
fn trash_selected(
    graph: &sparq_core::Graph,
    member: Option<&str>,
) -> Result<Vec<TrashEntry>, SparqError> {
    let r = records(graph, true, None, None, None, member)?;
    let mut entries = Vec::new();
    for row in &r.rows {
        entries.push(TrashEntry {
            member: cell(&r, row, "member").ok_or(SparqError::NotFound)?,
            meta: row_meta(&r, row)?,
            fields: TrashFields {
                original: cell(&r, row, "original").ok_or(SparqError::NotFound)?,
                trashed_at: cell(&r, row, "at")
                    .and_then(|s| timestamp::from_xsd_datetime(&s))
                    .ok_or_else(|| SparqError::Backend("invalid trash date".into()))?,
                deleted_by: cell(&r, row, "by").filter(|s| !s.is_empty()),
                deleter_could_read: cell(&r, row, "read").as_deref() == Some("true"),
            },
        });
    }
    entries.sort_by_key(|e| e.fields.trashed_at);
    Ok(entries)
}
pub(super) fn replace(
    graph: &mut ActorGraph,
    iri: &str,
    meta: ResourceMeta,
    member: &str,
    fields: VersionFields,
) -> Result<Vec<String>, SparqError> {
    let r = sparq_engine::query(graph, &sparql::select_meta(iri)?)
        .map_err(|e| engine_err("history/current", e))?;
    let old = match meta_from_result(&r) {
        Ok(m) => Some(m),
        Err(SparqError::NotFound) => None,
        Err(e) => return Err(e),
    };
    let mut updates = vec![];
    let mut pruned_keys = Vec::new();
    if let Some(old) = old {
        let entries = versions(graph, Some(iri))?;
        let order = entries
            .iter()
            .map(|v| v.order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for entry in entries.iter().take(
            entries
                .len()
                .saturating_add(1)
                .saturating_sub(fields.max_versions),
        ) {
            updates.push(sparql::update_delete_resource(&entry.member)?);
            pruned_keys.push(entry.meta.blob_key.clone());
        }
        updates.push(meta_update(member, &old)?);
        updates.push(field_update(
            member,
            &[
                ("original", fields.original.clone()),
                ("commitOrder", order.to_string()),
            ],
        )?);
    }
    updates.push(meta_update(iri, &meta)?);
    commit(graph, &updates.join(";\n"), "replace_meta_retaining")?;
    Ok(pruned_keys)
}
fn rekey_versions(
    graph: &sparq_core::Graph,
    from: &str,
    to: &str,
) -> Result<Vec<String>, SparqError> {
    let mut updates = Vec::new();
    for entry in versions(graph, Some(from))? {
        let member = history::rekey_member(&entry.member, to);
        updates.push(meta_update(&member, &entry.meta)?);
        updates.push(field_update(
            &member,
            &[
                ("original", to.to_string()),
                ("commitOrder", entry.order.to_string()),
            ],
        )?);
        updates.push(sparql::update_delete_resource(&entry.member)?);
    }
    Ok(updates)
}
pub(super) fn move_to_trash(
    graph: &mut ActorGraph,
    iri: &str,
    parent: Option<&str>,
    member: &str,
    fields: TrashFields,
) -> Result<(), SparqError> {
    let r = sparq_engine::query(graph, &sparql::select_meta(iri)?)
        .map_err(|e| engine_err("trash/current", e))?;
    let meta = meta_from_result(&r)?;
    let at = timestamp::to_xsd_datetime(fields.trashed_at)
        .ok_or_else(|| SparqError::Backend("invalid trash date".into()))?;
    let mut updates = vec![
        meta_update(member, &meta)?,
        field_update(
            member,
            &[
                ("original", fields.original),
                ("trashedAt", at),
                ("deletedBy", fields.deleted_by.unwrap_or_default()),
                ("deleterCouldRead", fields.deleter_could_read.to_string()),
            ],
        )?,
        copy_body(iri, member)?,
        sparql::update_delete_resource_with_parent(iri, parent)?,
    ];
    updates.extend(rekey_versions(graph, iri, member)?);
    commit(graph, &updates.join(";\n"), "move_to_trash")
}
pub(super) fn restore(
    graph: &mut ActorGraph,
    member: &str,
    target: &str,
    parent: &str,
) -> Result<(), SparqError> {
    let exists = |iri| {
        sparq_engine::ask(graph, &sparql::ask_exists(iri)?)
            .map_err(|e| engine_err("restore/exists", e))
    };
    if exists(target)? || !exists(parent)? {
        return Err(SparqError::Backend(
            "Conflict: restore target or parent".into(),
        ));
    }
    let t = trash_entry(graph, member)?.ok_or(SparqError::NotFound)?;
    let create = sparql::update_create_child(
        parent,
        target,
        &t.meta.content_type,
        &t.meta.blob_key,
        &t.meta.etag,
        t.meta
            .last_modified
            .and_then(timestamp::to_xsd_datetime)
            .as_deref(),
        &next_nonce(),
    )?;
    let mut updates = vec![
        create,
        copy_body(member, target)?,
        sparql::update_delete_resource(member)?,
    ];
    updates.extend(rekey_versions(graph, member, target)?);
    commit(graph, &updates.join(";\n"), "restore_from_trash")
}

pub(super) fn expired(
    graph: &sparq_core::Graph,
    versions_before: SystemTime,
    trash_before: SystemTime,
    limit: usize,
) -> Result<Vec<history::ExpiredEntry>, SparqError> {
    let mut entries = Vec::new();
    for (trash, before) in [(false, versions_before), (true, trash_before)] {
        let r = records(graph, trash, None, Some(before), Some(limit), None)?;
        for row in &r.rows {
            let at = cell(&r, row, if trash { "at" } else { "mod" })
                .and_then(|s| timestamp::from_xsd_datetime(&s))
                .ok_or_else(|| SparqError::Backend("invalid expiry date".into()))?;
            entries.push(history::ExpiredEntry {
                member: cell(&r, row, "member").ok_or(SparqError::NotFound)?,
                blob_key: row_meta(&r, row)?.blob_key,
                at,
                trash,
            });
        }
    }
    entries.sort_by_key(|e| e.at);
    entries.truncate(limit);
    Ok(entries)
}

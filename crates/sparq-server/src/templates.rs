//! (sq-lsp7k.10) OPT-IN **named parameterized SPARQL template** REST surface —
//! the GraphDB "SPARQL templates" (smart updates) / Stardog "stored queries" parity
//! feature: server-stored, IRI-identified query/UPDATE templates with typed parameter
//! binding, managed over REST and invoked with a JSON argument object instead of
//! free-form SPARQL (exactly what an app backend or an LLM agent wants in front of a
//! gated write path).
//!
//! Compiled ONLY behind the `templates` feature (which enables
//! `sparq-engine/templates`, itself on top of the #901 `params` injection-safe binding);
//! served only when [`ServerConfig::templates`](crate::http::ServerConfig) is also set
//! (`--templates` / `SPARQ_TEMPLATES=1`) — the same double-opt-in as `tpf` / `shacl` /
//! `terse` / `solid-authz`. The handlers live here to keep the touch surface on the
//! conflict-hot `http.rs` minimal (the `solid_authz` pattern).
//!
//! # Routes
//!
//! - `GET /templates` — list every stored definition (read-gated).
//! - `GET /templates/{name}` — one definition (read-gated). `{name}` is a wildcard
//!   capture, so an IRI-identified template's name can carry `/`.
//! - `PUT /templates/{name}` — store/replace a definition (WRITE-gated). The body is the
//!   JSON definition (`text`/`sparql`, `parameters`, `description`); a `name` in the body
//!   must match the path. Fail-closed: an unparseable text or an undeclarable parameter
//!   is a `400` at registration — a stored template is always invocable.
//! - `DELETE /templates/{name}` — remove a definition (WRITE-gated).
//! - `POST /templates/{name}` — INVOKE with a JSON argument object. A query template is
//!   read-gated and answers SPARQL-JSON (SELECT/ASK) or N-Triples (CONSTRUCT/DESCRIBE);
//!   an UPDATE template is **write-gated and executed through the SAME sequenced-writer
//!   path (`run_update`) as a `/sparql` update** — budgets, atomicity, durability and the
//!   auth posture are identical (the template layer never widens the gated-update
//!   posture).
//!
//! # Fail-closed invocation
//!
//! Unknown argument names, missing declared parameters, and JSON shapes that do not
//! match a declared type are all `400`s from
//! [`Template::bind_json`](sparq_engine::templates::Template::bind_json) — never a
//! silent no-op. Binding is the #901 algebra rewrite, so a hostile bound value is
//! carried as opaque data and cannot change the query structure; the bound algebra is
//! then rendered canonically (spargebra's serializer escapes terms) for the shared
//! execution paths.
//!
//! # Persistence
//!
//! With `--templates-file <PATH>` the store is durable: loaded (fail-closed) at startup,
//! rewritten atomically (write-temp + rename) after every successful `PUT` / `DELETE`.
//! The file write happens BEFORE the in-memory store is updated, so a full disk never
//! leaves memory and disk disagreeing in the durable direction.

use std::collections::BTreeMap;
use std::path::Path;

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use sparq_engine::templates::Template;

use crate::http::{
    auth_gate, await_worker, bad_request, engine_error_response, json_error, make_budget,
    run_update, text_response, AppState, Operation,
};

/// The store type held by `AppState` (name → validated template).
pub(crate) type Store = BTreeMap<String, Template>;

/// Loads the template store from the optional persistence file at startup. `None` (or a
/// file that does not exist yet) is an empty store; an unreadable or invalid file is a
/// fail-closed startup ERROR — the operator asked for durable templates, so a corrupt
/// definition file must not become a silently-empty store.
pub(crate) fn load_store(file: Option<&Path>) -> Result<Store, String> {
    let Some(path) = file else {
        return Ok(Store::new());
    };
    if !path.exists() {
        return Ok(Store::new());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("templates file {} unreadable: {}", path.display(), e))?;
    let defs: Value = serde_json::from_str(&text)
        .map_err(|e| format!("templates file {} is not valid JSON: {}", path.display(), e))?;
    let Some(arr) = defs.as_array() else {
        return Err(format!(
            "templates file {} must hold a JSON array of template definitions",
            path.display()
        ));
    };
    let mut store = Store::new();
    for def in arr {
        let t = Template::from_json(def)
            .map_err(|e| format!("templates file {}: {}", path.display(), e))?;
        if store.insert(t.name().to_string(), t.clone()).is_some() {
            return Err(format!(
                "templates file {}: duplicate template name `{}`",
                path.display(),
                t.name()
            ));
        }
    }
    Ok(store)
}

/// Persists `store` to the configured file (no-op when persistence is off). Atomic in
/// the crash sense: written to a `.tmp` sibling then renamed over the target, so a
/// crash mid-write never truncates the existing file.
fn persist_store(state: &AppState, store: &Store) -> Result<(), String> {
    let Some(path) = state.config().templates_file.as_deref() else {
        return Ok(());
    };
    let defs: Vec<Value> = store.values().map(Template::to_json).collect();
    let text = serde_json::to_string_pretty(&Value::Array(defs))
        .map_err(|e| format!("serialize: {}", e))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {}", path.display(), e))?;
    Ok(())
}

/// A JSON `200` (the list/get/invoke success shape).
fn json_ok(body: String) -> Response {
    text_response(StatusCode::OK, "application/json; charset=utf-8", body, false)
}

/// The shared "surface off" refusal: the same structured 404 the other double-opt-in
/// endpoints mint, leak-free (it does not reveal the feature exists).
fn surface_off() -> Response {
    json_error(StatusCode::NOT_FOUND, "not found")
}

/// `GET /templates` — every stored definition, read-gated.
pub(crate) async fn list_endpoint(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.config().templates {
        return surface_off();
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let defs: Vec<Value> = state
        .templates()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .map(Template::to_json)
        .collect();
    json_ok(Value::Array(defs).to_string())
}

/// `GET /templates/{name}` — one definition, read-gated; `404` when absent.
pub(crate) async fn get_endpoint(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !state.config().templates {
        return surface_off();
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let found = state
        .templates()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(&name)
        .map(Template::to_json);
    match found {
        Some(def) => json_ok(def.to_string()),
        None => json_error(StatusCode::NOT_FOUND, "no such template"),
    }
}

/// `PUT /templates/{name}` — store/replace a definition. A WRITE (it changes what a
/// later invocation executes), so it is gated exactly like an update. `201` on create,
/// `204` on replace, `400` on an invalid definition (fail-closed registration).
pub(crate) async fn put_endpoint(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().templates {
        return surface_off();
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    let Ok(mut def) = serde_json::from_slice::<Value>(&body) else {
        return bad_request("template definition body is not valid JSON");
    };
    // The path segment is the identity; a body `name` must agree (fail-closed), and an
    // absent one is filled in so the body can stay minimal.
    match def.get("name").and_then(Value::as_str) {
        None => {
            if let Some(obj) = def.as_object_mut() {
                obj.insert("name".to_string(), Value::String(name.clone()));
            }
        }
        Some(n) if n == name => {}
        Some(_) => return bad_request("template `name` in the body must match the path"),
    }
    let template = match Template::from_json(&def) {
        Ok(t) => t,
        Err(e) => return bad_request(&format!("invalid template definition: {}", e)),
    };
    // Build the updated map, persist it FIRST (durability is fail-closed), then commit
    // it to memory.
    let lock = state.templates();
    let mut guard = lock.write().unwrap_or_else(|p| p.into_inner());
    let mut next = guard.clone();
    let existed = next.insert(name.clone(), template).is_some();
    if let Err(e) = persist_store(&state, &next) {
        tracing::warn!(target: "sparq_server::templates", error = %e, "template persist failed");
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "template store persistence failed; nothing was changed",
        );
    }
    *guard = next;
    if existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

/// `DELETE /templates/{name}` — remove a definition (WRITE-gated). `204` / `404`.
pub(crate) async fn delete_endpoint(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !state.config().templates {
        return surface_off();
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    let lock = state.templates();
    let mut guard = lock.write().unwrap_or_else(|p| p.into_inner());
    if !guard.contains_key(&name) {
        return json_error(StatusCode::NOT_FOUND, "no such template");
    }
    let mut next = guard.clone();
    next.remove(&name);
    if let Err(e) = persist_store(&state, &next) {
        tracing::warn!(target: "sparq_server::templates", error = %e, "template persist failed");
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "template store persistence failed; nothing was changed",
        );
    }
    *guard = next;
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /templates/{name}` — invoke a stored template with a JSON argument object.
///
/// Gate order (fail-closed, before any engine work): the read gate FIRST (invocation is
/// at minimum a read — an unauthenticated caller learns nothing, not even whether the
/// template exists), then the lookup, then the WRITE gate when the template is an
/// UPDATE. The bound update is rendered from the value-substituted algebra and executed
/// through [`run_update`] — the exact writer path, budget and status mapping a
/// `/sparql` `application/sparql-update` request gets.
pub(crate) async fn invoke_endpoint(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().templates {
        return surface_off();
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let template = {
        let guard = state.templates().read().unwrap_or_else(|p| p.into_inner());
        match guard.get(&name) {
            Some(t) => t.clone(),
            None => return json_error(StatusCode::NOT_FOUND, "no such template"),
        }
    };
    // The gated-update posture: an UPDATE template invocation is a write, gated BEFORE
    // binding (no engine work for an unauthorized caller).
    if template.is_update() {
        if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
            return resp;
        }
    }
    // An empty body is an empty argument object (a zero-parameter template invokes bare).
    let args: Value = if body.is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return bad_request("template arguments body is not valid JSON"),
        }
    };
    let bound = match template.bind_json(&args) {
        Ok(b) => b,
        Err(e) => return bad_request(&format!("template invocation failed: {}", e)),
    };
    if bound.is_update() {
        // Render the bound algebra canonically (terms are escaped by spargebra's
        // serializer — the value substitution already happened structurally) and run
        // it through the SAME sequenced-writer path as a /sparql update.
        return run_update(&state, bound.render()).await;
    }
    let is_graph_form = bound.is_graph_form();
    let rendered = bound.render();
    let pin = state.current();
    let config = state.config().clone();
    let budget = make_budget(&config, true);
    let task = tokio::task::spawn_blocking(move || {
        let result = if is_graph_form {
            sparq_engine::construct_ntriples_with_budget(pin.snapshot(), &rendered, &budget)
                .map(|nt| (nt, "application/n-triples; charset=utf-8"))
        } else {
            sparq_engine::query_json_with_budget(pin.snapshot(), &rendered, &budget)
                .map(|json| (json, "application/sparql-results+json; charset=utf-8"))
        };
        match result {
            Ok((body, ct)) => text_response(StatusCode::OK, ct, body, false),
            // make_budget(_, true) applied max_results, so the shared mapper
            // classifies budget trips (413/503) vs genuine 400s.
            Err(e) => engine_error_response(&e, &config, true),
        }
    });
    await_worker(task, state.config()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // `load_store` — the pure startup seeding path (absent file, valid file, fail-closed
    // invalid shapes). The HTTP behaviour is covered by tests/templates.rs.
    #[test]
    fn load_store_paths() {
        // No persistence configured / file absent ⇒ empty store.
        assert!(load_store(None).unwrap().is_empty());
        let dir = std::env::temp_dir().join(format!("sparq-tpl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("templates.json");
        assert!(load_store(Some(&path)).unwrap().is_empty());
        // A valid definition file loads (and keys by name).
        std::fs::write(
            &path,
            r#"[{"name":"t1","text":"ASK { ?s ?p ?o }","parameters":{}}]"#,
        )
        .unwrap();
        let store = load_store(Some(&path)).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store["t1"].kind(), "query");
        // Fail-closed: not-JSON, not-an-array, an invalid definition, duplicate names.
        std::fs::write(&path, "not json").unwrap();
        assert!(load_store(Some(&path)).unwrap_err().contains("not valid JSON"));
        std::fs::write(&path, r#"{"name":"t1"}"#).unwrap();
        assert!(load_store(Some(&path)).unwrap_err().contains("array"));
        std::fs::write(&path, r#"[{"name":"t1","text":"NOT SPARQL"}]"#).unwrap();
        assert!(load_store(Some(&path)).is_err());
        std::fs::write(
            &path,
            r#"[{"name":"t1","text":"ASK { ?s ?p ?o }"},{"name":"t1","text":"ASK { ?s ?p ?o }"}]"#,
        )
        .unwrap();
        assert!(load_store(Some(&path)).unwrap_err().contains("duplicate"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

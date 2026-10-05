//! Authorization, access grants and access requests (LWS 1.0 core section 11, the Access Profile).
//!
//! The storage owner may do anything; the agent that created a resource may do anything with it;
//! anyone else what an access grant gives them. A grant's policies name an assignee (an agent, or
//! `foaf:Agent` for everyone), actions (`read`, `modify`, `create`, `delete`), the resources they
//! cover (not recursively: `create` on a container lets the assignee add members to it) and ODRL
//! constraints that must all hold.
//!
//! The access grant service and the access request service are LWS containers at
//! [`GRANTS_PATH`](super::GRANTS_PATH) and [`REQUESTS_PATH`](super::REQUESTS_PATH). Only the owner
//! may grant access or list grants; any authenticated agent may ask for access, and may read and
//! withdraw what it asked. Both are stored through the [`Store`], so they survive a restart on a
//! durable backend, and kept in memory for authorization.

use std::collections::BTreeMap;
use std::sync::RwLock;

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use serde_json::{json, Value};

use super::{
    add_link, has_type, is_uri, jose, json_is_uri, method_not_allowed, problem, set, Agent, LwsConfig,
    LwsRequest, LwsState, FOAF_AGENT, GRANTS_PATH, LWS_CONTEXT, LWS_JSON, LWS_NS, REQUESTS_PATH,
};
use crate::store::Store;

/// What a request does to a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Modify,
    Create,
    Delete,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Modify => "modify",
            Action::Create => "create",
            Action::Delete => "delete",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "read" => Action::Read,
            "modify" => Action::Modify,
            "create" => Action::Create,
            "delete" => Action::Delete,
            _ => return None,
        })
    }
}

const LEFT_OPERANDS: &[&str] = &["client", "format", "type", "purpose", "dateTime"];
const OPERATORS: &[&str] = &["eq", "isAnyOf", "gt", "gteq", "lt", "lteq"];

/// One ODRL constraint of the Access Profile.
#[derive(Debug, Clone)]
pub struct Constraint {
    left: String,
    operator: String,
    right: Value,
}

/// What a constraint is checked against.
pub struct ConstraintContext<'a> {
    pub client: Option<&'a str>,
    /// The resource's media type (`application/lws+json` for a container).
    pub format: Option<&'a str>,
    /// The resource's types: its LWS type and any it declared.
    pub types: &'a [String],
}

impl Constraint {
    /// `dateTime` compares the current time; `client` the access token's `client_id`; `format`
    /// and `type` the resource. The draft does not say how a request states its `purpose`, so a
    /// purpose constraint never holds (fail closed), and neither does an operator that makes no
    /// sense for its operand.
    pub fn satisfied(&self, ctx: &ConstraintContext<'_>) -> bool {
        match self.left.as_str() {
            "dateTime" => self.compare_now(),
            "client" => ctx.client.is_some_and(|c| self.matches(c)),
            "format" => ctx.format.is_some_and(|f| self.matches(f)),
            "type" => ctx.types.iter().any(|t| {
                self.matches(t) || t.strip_prefix(LWS_NS).is_some_and(|short| self.matches(short))
            }),
            _ => false,
        }
    }

    fn matches(&self, actual: &str) -> bool {
        match self.operator.as_str() {
            "eq" => self.right.as_str() == Some(actual),
            "isAnyOf" => self.right.as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(actual))),
            _ => false,
        }
    }

    fn compare_now(&self) -> bool {
        let Some(bound) = self.right.as_str().and_then(parse_rfc3339) else {
            return false;
        };
        let now = jose::now_secs();
        match self.operator.as_str() {
            "eq" => now == bound,
            "gt" => now > bound,
            "gteq" => now >= bound,
            "lt" => now < bound,
            "lteq" => now <= bound,
            _ => false,
        }
    }
}

/// An RFC 3339 / xsd:dateTime instant, as seconds since the epoch.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    // YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        rest = &frac[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            sign * (o[1..3].parse::<i64>().ok()? * 3600 + o[4..6].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// An RFC 3339 UTC timestamp for `secs` since the epoch.
pub fn format_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// One AccessPolicy of a grant.
#[derive(Debug, Clone)]
pub struct Policy {
    pub actions: Vec<Action>,
    pub assignee: String,
    pub targets: Vec<String>,
    pub constraints: Vec<Constraint>,
}

impl Policy {
    fn applies(&self, subject: Option<&str>, action: Action, uri: &str) -> bool {
        (self.assignee == FOAF_AGENT || Some(self.assignee.as_str()) == subject)
            && self.actions.contains(&action)
            && self.targets.iter().any(|t| t == uri)
    }
}

/// A stored access grant or access request.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub document: Value,
    pub policies: Vec<Policy>,
    pub author: Option<String>,
    pub etag: String,
}

/// The grants and requests of the storage, in memory and in the store.
pub struct AccessStore {
    grants: RwLock<BTreeMap<String, Record>>,
    requests: RwLock<BTreeMap<String, Record>>,
    grants_etag: RwLock<String>,
    requests_etag: RwLock<String>,
}

fn new_etag() -> String {
    format!("\"{}\"", jose::random_id())
}

impl AccessStore {
    /// Ensure the two service containers exist and load what they hold.
    pub async fn load<S: Store>(store: &S, cfg: &LwsConfig) -> Result<Self, String> {
        let me = Self {
            grants: RwLock::new(BTreeMap::new()),
            requests: RwLock::new(BTreeMap::new()),
            grants_etag: RwLock::new(new_etag()),
            requests_etag: RwLock::new(new_etag()),
        };
        for grants in [true, false] {
            let container = cfg.absolute(if grants { GRANTS_PATH } else { REQUESTS_PATH });
            ensure_container(store, &container).await?;
            let children = store.list_children(&container).await.map_err(|e| format!("store: {e}"))?;
            for child in children {
                let Ok(r) = store.read(child.as_str()).await else { continue };
                let Ok(stored) = serde_json::from_slice::<Value>(&r.body) else { continue };
                let id = child.as_str().rsplit('/').next().unwrap_or_default().to_string();
                let document = stored.get("document").cloned().unwrap_or(Value::Null);
                let author = stored.get("author").and_then(Value::as_str).map(str::to_string);
                let policies = if grants { policies(document.get("access").unwrap_or(&Value::Null)).unwrap_or_default() } else { Vec::new() };
                let record = Record { id: id.clone(), document, policies, author, etag: new_etag() };
                me.map(grants).write().expect("lock").insert(id, record);
            }
        }
        Ok(me)
    }

    fn map(&self, grants: bool) -> &RwLock<BTreeMap<String, Record>> {
        if grants { &self.grants } else { &self.requests }
    }

    fn bump(&self, grants: bool) {
        *(if grants { &self.grants_etag } else { &self.requests_etag }).write().expect("lock") = new_etag();
    }

    /// Every policy of every grant, for an authorization decision.
    pub fn grant_policies(&self) -> Vec<Policy> {
        self.grants.read().expect("lock").values().flat_map(|r| r.policies.clone()).collect()
    }
}

async fn ensure_container<S: Store>(store: &S, iri: &str) -> Result<(), String> {
    if !store.exists(iri).await.map_err(|e| format!("store: {e}"))? {
        store.write(iri, Bytes::new(), LWS_JSON).await.map_err(|e| format!("store: {e}"))?;
    }
    Ok(())
}

/// Whether `agent` may perform `action` on the resource at `uri`.
pub async fn allowed<S: Store + 'static>(state: &LwsState<S>, action: Action, uri: &str, agent: &Agent) -> bool {
    if state.cfg.open {
        return true;
    }
    let subject = agent.subject.as_deref();
    if subject.is_some() && subject == state.cfg.owner.as_deref() {
        return true;
    }
    let meta = state.resource_meta(uri).await;
    if subject.is_some() && subject == meta.creator.as_deref() {
        return true;
    }
    let candidates: Vec<Policy> =
        state.access.grant_policies().into_iter().filter(|p| p.applies(subject, action, uri)).collect();
    if candidates.is_empty() {
        return false;
    }
    let is_container = uri.ends_with('/');
    let format = if is_container {
        Some(LWS_JSON.to_string())
    } else {
        state.store.meta(uri).await.ok().flatten().map(|m| m.content_type)
    };
    let mut types = vec![format!("{LWS_NS}{}", if is_container { "Container" } else { "DataResource" })];
    types.extend(meta.types.iter().cloned());
    let ctx = ConstraintContext { client: agent.client.as_deref(), format: format.as_deref(), types: &types };
    candidates.iter().any(|p| p.constraints.iter().all(|c| c.satisfied(&ctx)))
}

/// The AccessPolicy entries of `access`, or `None` when they are malformed.
pub fn policies(access: &Value) -> Option<Vec<Policy>> {
    let list = access.as_array().filter(|a| !a.is_empty())?;
    let mut out = Vec::new();
    for p in list {
        // type is REQUIRED and MUST include AccessPolicy; target, when given, MUST be an object.
        if !has_type(p.get("type").unwrap_or(&Value::Null), "AccessPolicy") {
            return None;
        }
        if p.get("target").is_some_and(|t| !t.is_object()) {
            return None;
        }
        let constraints = match p.get("constraint") {
            None => Vec::new(),
            Some(Value::Array(cs)) => {
                let mut out = Vec::new();
                for c in cs {
                    let left = c.get("leftOperand").and_then(Value::as_str).unwrap_or_default();
                    let op = c.get("operator").and_then(Value::as_str).unwrap_or_default();
                    if !LEFT_OPERANDS.contains(&left) || !OPERATORS.contains(&op) {
                        return None;
                    }
                    let right = c.get("rightOperand")?.clone();
                    out.push(Constraint { left: left.into(), operator: op.into(), right });
                }
                out
            }
            Some(_) => return None,
        };
        let action_values: Vec<&Value> = match p.get("action") {
            Some(Value::Array(a)) => a.iter().collect(),
            Some(v) => vec![v],
            None => Vec::new(),
        };
        let mut actions = Vec::new();
        for a in action_values {
            actions.push(Action::parse(a.as_str()?)?);
        }
        let assignee = p.get("assignee").and_then(Value::as_str).unwrap_or_default().to_string();
        let targets: Vec<String> = match p.get("target").and_then(|t| t.get("value")) {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        if actions.is_empty() || assignee.is_empty() || targets.is_empty() {
            return None;
        }
        out.push(Policy { actions, assignee, targets, constraints });
    }
    Some(out)
}

/// The access grant service and the access request service.
pub async fn handle<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, agent: &Agent) -> Response {
    let grants = req.path.starts_with(GRANTS_PATH);
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let id = &req.path[base.len()..];
    let owner = state.cfg.open || (agent.subject.is_some() && agent.subject == state.cfg.owner);
    if id.is_empty() {
        return match req.method {
            Method::GET | Method::HEAD => {
                if !owner {
                    return state.deny(agent);
                }
                listing(state, grants)
            }
            Method::POST => {
                if !state.cfg.open && (agent.subject.is_none() || (grants && !owner)) {
                    return state.deny(agent);
                }
                create(state, req, agent, grants).await
            }
            _ => method_not_allowed("GET, HEAD, POST"),
        };
    }
    let record = state.access.map(grants).read().expect("lock").get(id).cloned();
    let Some(record) = record else {
        return if state.needs_auth(agent) { state.challenge(None) } else { problem(StatusCode::NOT_FOUND, None) };
    };
    let mine = owner || (agent.subject.is_some() && agent.subject == record.author);
    if !mine {
        return state.deny(agent);
    }
    match req.method {
        Method::GET | Method::HEAD => {
            let mut resp = super::json_response(StatusCode::OK, LWS_JSON, &record.document);
            set(resp.headers_mut(), header::ETAG, &record.etag);
            add_link(resp.headers_mut(), &state.cfg.absolute(base), "up", None);
            add_link(resp.headers_mut(), &state.cfg.storage(), &format!("{LWS_NS}storage"), None);
            resp
        }
        Method::DELETE => {
            let iri = state.cfg.absolute(&format!("{base}{id}"));
            let _ = state.store.delete(&iri, Some(&state.cfg.absolute(base))).await;
            state.access.map(grants).write().expect("lock").remove(id);
            state.access.bump(grants);
            problem(StatusCode::NO_CONTENT, None)
        }
        _ => method_not_allowed("GET, HEAD, DELETE"),
    }
}

fn listing<S: Store + 'static>(state: &LwsState<S>, grants: bool) -> Response {
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let items: Vec<Value> = state
        .access
        .map(grants)
        .read()
        .expect("lock")
        .keys()
        .map(|id| json!({"id": state.cfg.absolute(&format!("{base}{id}")), "type": "DataResource", "format": LWS_JSON}))
        .collect();
    let body = json!({
        "@context": LWS_CONTEXT,
        "id": state.cfg.absolute(base),
        "type": "Container",
        "totalItems": items.len(),
        "items": items,
    });
    let mut resp = super::json_response(StatusCode::OK, LWS_JSON, &body);
    let etag = (if grants { &state.access.grants_etag } else { &state.access.requests_etag }).read().expect("lock").clone();
    set(resp.headers_mut(), header::ETAG, &etag);
    add_link(resp.headers_mut(), &format!("{LWS_NS}Container"), "type", None);
    add_link(resp.headers_mut(), &state.cfg.storage(), &format!("{LWS_NS}storage"), None);
    resp
}

async fn create<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, agent: &Agent, grants: bool) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&req.body) else {
        return problem(StatusCode::BAD_REQUEST, Some("the body is not JSON"));
    };
    let wanted = if grants { "AccessGrant" } else { "AccessRequest" };
    // The access data model: type, storage and access are REQUIRED, and an inbox, when given, MUST
    // be a URI. A document that breaks them is refused rather than stored.
    let valid = body.is_object()
        && has_type(body.get("type").unwrap_or(&Value::Null), wanted)
        && body.get("storage").is_some_and(Value::is_string)
        && body.get("inbox").is_none_or(json_is_uri);
    let Some(parsed) = valid.then(|| policies(body.get("access").unwrap_or(&Value::Null))).flatten() else {
        return problem(StatusCode::BAD_REQUEST, Some("not a valid access document"));
    };
    let id = jose::random_id();
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let iri = state.cfg.absolute(&format!("{base}{id}"));
    let mut document = body.clone();
    document["id"] = Value::String(iri.clone());
    let stored = json!({"document": document, "author": agent.subject});
    if let Err(e) = state
        .store
        .create_in_container(&state.cfg.absolute(base), &iri, Bytes::from(stored.to_string()), LWS_JSON)
        .await
    {
        return problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    }
    let record = Record {
        id: id.clone(),
        document,
        policies: if grants { parsed } else { Vec::new() },
        author: agent.subject.clone(),
        etag: new_etag(),
    };
    state.access.map(grants).write().expect("lock").insert(id, record);
    state.access.bump(grants);
    // "When an inbox property is present on an access request or access grant, the server SHOULD
    // deliver notifications to that endpoint" (section 11.6).
    if let Some(inbox) = body.get("inbox").and_then(Value::as_str).filter(|i| is_uri(i)) {
        let activity = json!({"type": ["Create"], "object": {"id": iri, "type": [wanted]}});
        state.notify.deliver(state, inbox, activity, None);
    }
    let mut resp = problem(StatusCode::CREATED, None);
    set(resp.headers_mut(), header::LOCATION, &iri);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-05T16:20:08Z").map(format_rfc3339).as_deref(), Some("2026-10-05T16:20:08Z"));
        assert_eq!(parse_rfc3339("2026-10-05T18:20:08.123+02:00"), parse_rfc3339("2026-10-05T16:20:08Z"));
        assert_eq!(parse_rfc3339("yesterday"), None);
    }

    #[test]
    fn policies_validate() {
        let ok = json!([{"type": "AccessPolicy", "action": ["read"], "assignee": FOAF_AGENT,
                         "target": {"value": ["https://s/x"]}}]);
        assert_eq!(policies(&ok).unwrap().len(), 1);
        let bad_action = json!([{"type": "AccessPolicy", "action": ["fly"], "assignee": "a", "target": {"value": "x"}}]);
        assert!(policies(&bad_action).is_none());
        let bad_constraint = json!([{"type": "AccessPolicy", "action": "read", "assignee": "a", "target": {"value": "x"},
                                     "constraint": [{"leftOperand": "colour", "operator": "eq", "rightOperand": "red"}]}]);
        assert!(policies(&bad_constraint).is_none());
        assert!(policies(&json!([])).is_none());
    }

    #[test]
    fn constraints() {
        let c = Constraint { left: "client".into(), operator: "eq".into(), right: json!("app") };
        let types = vec![format!("{LWS_NS}DataResource")];
        assert!(c.satisfied(&ConstraintContext { client: Some("app"), format: None, types: &types }));
        assert!(!c.satisfied(&ConstraintContext { client: Some("other"), format: None, types: &types }));
        let t = Constraint { left: "type".into(), operator: "eq".into(), right: json!("DataResource") };
        assert!(t.satisfied(&ConstraintContext { client: None, format: None, types: &types }));
        let past = Constraint { left: "dateTime".into(), operator: "lt".into(), right: json!("2000-01-01T00:00:00Z") };
        assert!(!past.satisfied(&ConstraintContext { client: None, format: None, types: &types }));
    }
}

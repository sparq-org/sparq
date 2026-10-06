//! Authorization, access grants and access requests (LWS 1.0 core section 11, the Access Profile).
//!
//! The storage owner may do anything; the agent that created a resource may do anything with it;
//! anyone else what an access grant gives them. A grant's policies name an assignee (an agent, or
//! `foaf:Agent` for everyone), actions (`read`, `modify`, `create`, `delete`), the resources they
//! cover and ODRL constraints that must all hold. A target names resources through a matcher type
//! (`DataResource`, `Container` or `StorageResource`) and values, not recursively (the draft gives
//! target values no containment semantics; `create` on a container lets the assignee add members
//! to it); a policy without a target (it is OPTIONAL) covers every resource of the grant's
//! `storage`.
//!
//! The access grant service and the access request service are LWS containers at
//! [`GRANTS_PATH`](super::GRANTS_PATH) and [`REQUESTS_PATH`](super::REQUESTS_PATH). Only the owner
//! may grant access or list grants; any authenticated agent may ask for access, and may read and
//! withdraw what it asked. Both are stored through the [`Store`], so they survive a restart on a
//! durable backend, and kept in memory for authorization.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use super::{
    has_lws_context, has_type, is_uri, jose, json_is_uri, method_not_allowed, none_match, problem,
    service_links, service_linkset, service_listing, set, subject_tokens, Agent, LwsConfig,
    LwsRequest, LwsState, ResourceMeta, FOAF_AGENT, GRANTS_PATH, LWS_JSON, LWS_NS, META_SUFFIX,
    REQUESTS_PATH,
};
use crate::error::ServerError;
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
                self.matches(t)
                    || t.strip_prefix(LWS_NS)
                        .is_some_and(|short| self.matches(short))
            }),
            _ => false,
        }
    }

    fn matches(&self, actual: &str) -> bool {
        match self.operator.as_str() {
            "eq" => self.right.as_str() == Some(actual),
            "isAnyOf" => self
                .right
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(actual))),
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
///
/// Parsed over bytes with every field checked to be ASCII digits before it is read, so malformed
/// (including non-ASCII) input is `None`, never a slice-on-a-char-boundary panic.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    // YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)
    let b = s.trim().as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (
        ascii_num(b, 0, 4)?,
        ascii_num(b, 5, 2)?,
        ascii_num(b, 8, 2)?,
        ascii_num(b, 11, 2)?,
        ascii_num(b, 14, 2)?,
        ascii_num(b, 17, 2)?,
    );
    let mut rest = &b[19..];
    if let Some(frac) = rest.strip_prefix(b".") {
        let digits = frac.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &frac[digits..];
    }
    let offset = match rest {
        b"Z" | b"z" => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (oh, om) = (ascii_num(rest, 1, 2)?, ascii_num(rest, 4, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            (if *sign == b'-' { -1 } else { 1 }) * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let (y2, m2) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// The `len` bytes at `b[at..]` as a number, if they exist and are all ASCII digits.
pub(crate) fn ascii_num(b: &[u8], at: usize, len: usize) -> Option<i64> {
    let digits = b.get(at..at.checked_add(len)?)?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    digits.iter().try_fold(0i64, |n, d| {
        n.checked_mul(10)?.checked_add(i64::from(d - b'0'))
    })
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
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// What kind of resource a target matcher matches (Access Profile, Targets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// `lws:DataResource`: data resources only.
    DataResource,
    /// `lws:Container`: containers only.
    Container,
    /// `lws:StorageResource`: any storage resource.
    StorageResource,
}

impl TargetKind {
    /// The matcher a target `type` names, as a term or in the LWS namespace; `None` for one this
    /// server does not support (and so cannot enforce).
    fn parse(v: &Value) -> Option<Self> {
        let s = v.as_str()?;
        Some(match s.strip_prefix(LWS_NS).unwrap_or(s) {
            "DataResource" => TargetKind::DataResource,
            "Container" => TargetKind::Container,
            "StorageResource" => TargetKind::StorageResource,
            _ => return None,
        })
    }

    fn matches(self, uri: &str) -> bool {
        match self {
            TargetKind::DataResource => !uri.ends_with('/'),
            TargetKind::Container => uri.ends_with('/'),
            TargetKind::StorageResource => true,
        }
    }
}

/// A policy's target: a matcher and the resources it names. The draft defines no containment
/// semantics for target values, so a value names that resource alone, not its members (`create` on
/// a container is what lets an assignee add members to it).
#[derive(Debug, Clone)]
pub struct Target {
    pub kind: TargetKind,
    pub values: Vec<String>,
}

/// One AccessPolicy of a grant.
#[derive(Debug, Clone)]
pub struct Policy {
    pub actions: Vec<Action>,
    pub assignee: String,
    /// `None` when the policy names no target (the property is OPTIONAL): it then covers every
    /// resource of the storage the grant is scoped to.
    pub target: Option<Target>,
    /// The grant's `storage`: the scope of an untargeted policy.
    pub storage: String,
    pub constraints: Vec<Constraint>,
}

impl Policy {
    fn applies(&self, subject: Option<&str>, action: Action, uri: &str) -> bool {
        (self.assignee == FOAF_AGENT || Some(self.assignee.as_str()) == subject)
            && self.actions.contains(&action)
            && self.covers(uri)
    }

    /// Whether the policy's target covers `uri`.
    pub fn covers(&self, uri: &str) -> bool {
        match &self.target {
            Some(t) => t.kind.matches(uri) && t.values.iter().any(|v| v == uri),
            None => {
                let scope = self.storage.trim_end_matches('/');
                !scope.is_empty()
                    && uri
                        .strip_prefix(scope)
                        .is_some_and(|rest| rest.starts_with('/'))
            }
        }
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
    /// Put a grant of `policies` in force under `id`, in memory only.
    #[cfg(test)]
    pub fn put_grant_for_test(&self, id: &str, policies: Vec<Policy>) {
        let record = Record {
            id: id.into(),
            document: Value::Null,
            policies,
            author: None,
            etag: new_etag(),
        };
        self.grants.write().expect("lock").insert(id.into(), record);
    }

    /// Revoke the grant `id`, in memory only.
    #[cfg(test)]
    pub fn remove_grant_for_test(&self, id: &str) {
        self.grants.write().expect("lock").remove(id);
    }

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
            let children = store
                .list_children(&container)
                .await
                .map_err(|e| format!("store: {e}"))?;
            for child in children {
                let Ok(r) = store.read(child.as_str()).await else {
                    continue;
                };
                let Ok(stored) = serde_json::from_slice::<Value>(&r.body) else {
                    continue;
                };
                let id = child
                    .as_str()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let document = stored.get("document").cloned().unwrap_or(Value::Null);
                let author = stored
                    .get("author")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let storage = document
                    .get("storage")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let policies = if grants {
                    policies(document.get("access").unwrap_or(&Value::Null), storage)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let record = Record {
                    id: id.clone(),
                    document,
                    policies,
                    author,
                    etag: new_etag(),
                };
                me.map(grants).write().expect("lock").insert(id, record);
            }
        }
        Ok(me)
    }

    fn map(&self, grants: bool) -> &RwLock<BTreeMap<String, Record>> {
        if grants {
            &self.grants
        } else {
            &self.requests
        }
    }

    fn bump(&self, grants: bool) {
        *(if grants {
            &self.grants_etag
        } else {
            &self.requests_etag
        })
        .write()
        .expect("lock") = new_etag();
    }

    /// Every policy of every grant, for an authorization decision.
    pub fn grant_policies(&self) -> Vec<Policy> {
        self.grants
            .read()
            .expect("lock")
            .values()
            .flat_map(|r| r.policies.clone())
            .collect()
    }
}

async fn ensure_container<S: Store>(store: &S, iri: &str) -> Result<(), String> {
    if !store.exists(iri).await.map_err(|e| format!("store: {e}"))? {
        store
            .write(iri, Bytes::new(), LWS_JSON)
            .await
            .map_err(|e| format!("store: {e}"))?;
    }
    Ok(())
}

/// Whether `agent` may perform `action` on the resource at `uri`; an error when the metadata the
/// decision rests on cannot be read.
pub async fn allowed<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
) -> Result<bool, ServerError> {
    if state.cfg.open || is_owner(state, agent) {
        return Ok(true);
    }
    let meta = state.resource_meta(uri).await?;
    decide(state, action, uri, agent, &meta, None).await
}

/// What a decision about a resource rests on, taken while the resource is there: its metadata
/// and its format. A Delete is announced once the resource is gone, so each delivery of it is
/// authorized against this snapshot (and the grants as they stand then).
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    meta: ResourceMeta,
    format: Option<String>,
}

impl Snapshot {
    /// The state of the resource at `uri` now.
    pub async fn take<S: Store + 'static>(
        state: &LwsState<S>,
        uri: &str,
    ) -> Result<Self, ServerError> {
        Ok(Self {
            meta: state.resource_meta(uri).await?,
            format: format_of(state, uri).await?,
        })
    }
}

/// As [`allowed`], against `snapshot` rather than the resource as it is now; the grants are those
/// in force now.
pub async fn allowed_as<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
    snapshot: &Snapshot,
) -> bool {
    if state.cfg.open || is_owner(state, agent) {
        return true;
    }
    decide(
        state,
        action,
        uri,
        agent,
        &snapshot.meta,
        Some(snapshot.format.clone()),
    )
    .await
    .unwrap_or(false)
}

fn is_owner<S: Store + 'static>(state: &LwsState<S>, agent: &Agent) -> bool {
    let subject = agent.subject.as_deref();
    subject.is_some() && subject == state.cfg.owner.as_deref()
}

/// The format a format constraint is checked against: a container's is its listing's.
async fn format_of<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Option<String>, ServerError> {
    if uri.ends_with('/') {
        return Ok(Some(LWS_JSON.to_string()));
    }
    Ok(state.store.meta(uri).await?.map(|m| m.content_type))
}

/// The decision past the owner: the creator, then the grants. `format` is looked up only when a
/// grant needs it, unless it is given.
async fn decide<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
    meta: &ResourceMeta,
    format: Option<Option<String>>,
) -> Result<bool, ServerError> {
    let subject = agent.subject.as_deref();
    if subject.is_some() && subject == meta.creator.as_deref() {
        return Ok(true);
    }
    // Content and metadata may not match (a write failed part way): fail closed for every
    // grant, whose constraints rest on the types and format.
    if meta.pending {
        return Ok(false);
    }
    let candidates: Vec<Policy> = state
        .access
        .grant_policies()
        .into_iter()
        .filter(|p| p.applies(subject, action, uri))
        .collect();
    if candidates.is_empty() {
        return Ok(false);
    }
    let is_container = uri.ends_with('/');
    let format = match format {
        Some(f) => f,
        None => format_of(state, uri).await?,
    };
    let mut types = vec![format!(
        "{LWS_NS}{}",
        if is_container {
            "Container"
        } else {
            "DataResource"
        }
    )];
    types.extend(meta.types.iter().cloned());
    let ctx = ConstraintContext {
        client: agent.client.as_deref(),
        format: format.as_deref(),
        types: &types,
    };
    Ok(candidates
        .iter()
        .any(|p| p.constraints.iter().all(|c| c.satisfied(&ctx))))
}

/// The AccessPolicy entries of `access` in a document scoped to `storage`, or `None` when they are
/// malformed.
pub fn policies(access: &Value, storage: &str) -> Option<Vec<Policy>> {
    let list = access.as_array().filter(|a| !a.is_empty())?;
    let mut out = Vec::new();
    for p in list {
        // type is REQUIRED and MUST include AccessPolicy.
        if !has_type(p.get("type").unwrap_or(&Value::Null), "AccessPolicy") {
            return None;
        }
        // target is OPTIONAL; when given it is an object with a supported matcher type and one or
        // more value strings.
        let target = match p.get("target") {
            None => None,
            Some(t) => {
                let kind = TargetKind::parse(t.get("type")?)?;
                let values: Vec<String> = match t.get("value")? {
                    Value::Array(a) => a
                        .iter()
                        .map(|v| v.as_str().map(str::to_string))
                        .collect::<Option<_>>()?,
                    Value::String(s) => vec![s.clone()],
                    _ => return None,
                };
                if values.is_empty() {
                    return None;
                }
                Some(Target { kind, values })
            }
        };
        let constraints = match p.get("constraint") {
            None => Vec::new(),
            Some(Value::Array(cs)) => {
                let mut out = Vec::new();
                for c in cs {
                    let left = c
                        .get("leftOperand")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let op = c
                        .get("operator")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !LEFT_OPERANDS.contains(&left) || !OPERATORS.contains(&op) {
                        return None;
                    }
                    let right = c.get("rightOperand")?.clone();
                    out.push(Constraint {
                        left: left.into(),
                        operator: op.into(),
                        right,
                    });
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
        let assignee = p
            .get("assignee")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if actions.is_empty() || assignee.is_empty() {
            return None;
        }
        out.push(Policy {
            actions,
            assignee,
            target,
            storage: storage.to_string(),
            constraints,
        });
    }
    Some(out)
}

/// The assignees an access document's policies name.
fn assignees(document: &Value) -> Vec<String> {
    document
        .get("access")
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p.get("assignee").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The access grant service and the access request service.
pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    let grants = req.path.starts_with(GRANTS_PATH);
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let container = state.cfg.absolute(base);
    let mut id = &req.path[base.len()..];
    // `{container}.meta` and `{member}.meta` are the (read-only) linksets the container and its
    // members link to.
    let linkset = id.ends_with(META_SUFFIX);
    if linkset {
        id = &id[..id.len() - META_SUFFIX.len()];
    }
    let owner = state.cfg.open || (agent.subject.is_some() && agent.subject == state.cfg.owner);
    if id.is_empty() {
        if linkset {
            return if owner {
                service_linkset(&state.cfg, req, &container)
            } else {
                state.deny(agent)
            };
        }
        return match req.method {
            Method::GET | Method::HEAD => {
                if !owner {
                    return state.deny(agent);
                }
                listing(state, req, grants)
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
    let record = state
        .access
        .map(grants)
        .read()
        .expect("lock")
        .get(id)
        .cloned();
    let Some(record) = record else {
        return if state.needs_auth(agent) {
            state.challenge(None)
        } else {
            problem(StatusCode::NOT_FOUND, None)
        };
    };
    let mine = owner || (agent.subject.is_some() && agent.subject == record.author);
    if !mine {
        return state.deny(agent);
    }
    let iri = format!("{container}{id}");
    if linkset {
        return service_linkset(&state.cfg, req, &iri);
    }
    match req.method {
        Method::GET | Method::HEAD => {
            if none_match(req, &record.etag) {
                let mut resp = StatusCode::NOT_MODIFIED.into_response();
                set(resp.headers_mut(), header::ETAG, &record.etag);
                service_links(&state.cfg, resp.headers_mut(), &iri, &container);
                return resp;
            }
            let mut resp = super::json_response(StatusCode::OK, LWS_JSON, &record.document);
            set(resp.headers_mut(), header::ETAG, &record.etag);
            service_links(&state.cfg, resp.headers_mut(), &iri, &container);
            resp
        }
        Method::DELETE => {
            // A revocation is durable before it is reported: a grant whose stored copy survives
            // would be reloaded, and so reinstated, at the next boot.
            match state.store.delete(&iri, Some(&container)).await {
                Ok(_) | Err(ServerError::NotFound) => {}
                Err(e) => {
                    return problem(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Some(&format!("cannot delete {iri}: {e}")),
                    )
                }
            }
            state.access.map(grants).write().expect("lock").remove(id);
            state.access.bump(grants);
            problem(StatusCode::NO_CONTENT, None)
        }
        _ => method_not_allowed("GET, HEAD, DELETE"),
    }
}

fn listing<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, grants: bool) -> Response {
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let items: Vec<Value> = state
        .access
        .map(grants)
        .read()
        .expect("lock")
        .keys()
        .map(|id| json!({"id": state.cfg.absolute(&format!("{base}{id}")), "type": "DataResource", "format": LWS_JSON}))
        .collect();
    let version = (if grants {
        &state.access.grants_etag
    } else {
        &state.access.requests_etag
    })
    .read()
    .expect("lock")
    .clone();
    service_listing(&state.cfg, req, &state.cfg.absolute(base), items, &version)
}

async fn create<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    grants: bool,
) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&req.body) else {
        return problem(StatusCode::BAD_REQUEST, Some("the body is not JSON"));
    };
    let wanted = if grants {
        "AccessGrant"
    } else {
        "AccessRequest"
    };
    // The JSON-LD serialization MUST carry an @context that includes the LWS context.
    if !has_lws_context(body.get("@context")) {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("an access document's @context must include https://www.w3.org/ns/lws/v1"),
        );
    }
    // The access data model: type, storage and access are REQUIRED, and storage and an inbox, when
    // given, MUST be URIs. A document that breaks them is refused rather than stored.
    let valid = body.is_object()
        && has_type(body.get("type").unwrap_or(&Value::Null), wanted)
        && body.get("storage").is_some_and(json_is_uri)
        && body.get("inbox").is_none_or(json_is_uri);
    let storage = body
        .get("storage")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(parsed) = valid
        .then(|| policies(body.get("access").unwrap_or(&Value::Null), storage))
        .flatten()
    else {
        return problem(StatusCode::BAD_REQUEST, Some("not a valid access document"));
    };
    let id = jose::random_id();
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let container = state.cfg.absolute(base);
    let iri = format!("{container}{id}");
    let mut document = body.clone();
    document["id"] = Value::String(iri.clone());
    let stored = json!({"document": document, "author": agent.subject});
    if let Err(e) = state
        .store
        .create_in_container(&container, &iri, Bytes::from(stored.to_string()), LWS_JSON)
        .await
    {
        return problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    }
    let record = Record {
        id: id.clone(),
        document: document.clone(),
        policies: if grants { parsed } else { Vec::new() },
        author: agent.subject.clone(),
        etag: new_etag(),
    };
    state
        .access
        .map(grants)
        .write()
        .expect("lock")
        .insert(id, record);
    state.access.bump(grants);
    let activity = json!({"type": ["Create"], "object": {"id": iri, "type": [wanted]}});
    // "When an inbox property is present on an access request or access grant, the server SHOULD
    // deliver notifications to that endpoint" (section 11.6).
    let mut inboxes: BTreeSet<String> = body
        .get("inbox")
        .and_then(Value::as_str)
        .filter(|i| is_uri(i))
        .map(str::to_string)
        .into_iter()
        .collect();
    if grants {
        // "When a new access grant is created, the requesting agent SHOULD be notified at the inbox
        // specified in the associated access request." The draft gives a grant no link to its
        // request, so the associated requests are those by or for an agent the grant names.
        inboxes.extend(requester_inboxes(state, &document));
    } else {
        // "When a new access request is submitted, the storage controller SHOULD be notified": at
        // the inbox the owner's identity document names, looked up in the background.
        let state = state.clone();
        let activity = activity.clone();
        tokio::spawn(async move {
            if let Some(inbox) = owner_inbox(&state).await {
                state.notify.deliver(&state, &inbox, activity, None);
            }
        });
    }
    for inbox in inboxes {
        state.notify.deliver(state, &inbox, activity.clone(), None);
    }
    let mut resp = problem(StatusCode::CREATED, None);
    set(resp.headers_mut(), header::LOCATION, &iri);
    service_links(&state.cfg, resp.headers_mut(), &iri, &container);
    resp
}

/// The inboxes of the access requests associated with `grant`: those made by, or asking for, an
/// agent one of its policies is assigned to (public grants name no requester).
fn requester_inboxes<S: Store + 'static>(state: &LwsState<S>, grant: &Value) -> BTreeSet<String> {
    let grantees: BTreeSet<String> = assignees(grant)
        .into_iter()
        .filter(|a| a != FOAF_AGENT)
        .collect();
    state
        .access
        .requests
        .read()
        .expect("lock")
        .values()
        .filter(|r| {
            r.author.as_ref().is_some_and(|a| grantees.contains(a))
                || assignees(&r.document).iter().any(|a| grantees.contains(a))
        })
        .filter_map(|r| r.document.get("inbox").and_then(Value::as_str))
        .filter(|i| is_uri(i))
        .map(str::to_string)
        .collect()
}

/// The storage controller's inbox: the `inbox` (or `ldp:inbox`) its identity document names.
/// Only an http(s) owner with a JSON(-LD) document has a discoverable one; a did:key owner, a
/// Turtle-only profile or an unreachable document has none, and the notification is skipped.
async fn owner_inbox<S: Store + 'static>(state: &LwsState<S>) -> Option<String> {
    let owner = state.cfg.owner.as_deref()?;
    if !(owner.starts_with("https://") || owner.starts_with("http://")) {
        return None;
    }
    let (_, body) = subject_tokens::fetch(
        &state.cfg,
        &state.http,
        owner,
        "application/ld+json, application/json;q=0.9",
    )
    .await
    .ok()?;
    let doc: Value = serde_json::from_slice(&body).ok()?;
    inbox_of(&doc, owner)
}

/// The inbox a JSON(-LD) identity document names for `subject`: on the top-level node, or on the
/// `@graph` node whose id is the subject.
pub fn inbox_of(doc: &Value, subject: &str) -> Option<String> {
    let node_inbox = |n: &Value| {
        ["inbox", "ldp:inbox", "http://www.w3.org/ns/ldp#inbox"]
            .iter()
            .find_map(|k| match n.get(*k)? {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o
                    .get("id")
                    .or_else(|| o.get("@id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .filter(|i| is_uri(i))
    };
    node_inbox(doc).or_else(|| {
        doc.get("@graph")?
            .as_array()?
            .iter()
            .filter(|n| {
                n.get("id").or_else(|| n.get("@id")).and_then(Value::as_str) == Some(subject)
            })
            .find_map(node_inbox)
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_store;
    use super::super::LD_JSON;
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2026-10-05T16:20:08Z")
                .map(format_rfc3339)
                .as_deref(),
            Some("2026-10-05T16:20:08Z")
        );
        assert_eq!(
            parse_rfc3339("2026-10-05T18:20:08.123+02:00"),
            parse_rfc3339("2026-10-05T16:20:08Z")
        );
        assert_eq!(parse_rfc3339("yesterday"), None);
    }

    /// Review finding: a non-ASCII byte in the timezone used to reach `o[1..3]` and panic on a
    /// UTF-8 char boundary. Every malformed shape is now `None`.
    #[test]
    fn rfc3339_rejects_malformed_without_panicking() {
        for bad in [
            "2026-10-05T00:00:00+0\u{e9}00",
            "2026-10-05T00:00:00+\u{e9}:00",
            "2026-10-05T00:00:00\u{e9}0:00",
            "2026-10-05T00:00:00.\u{e9}Z",
            "2026-10-05T00:00:00.Z",
            "2026-10-05T00:00:00+0a:00",
            "2026-10-05T00:00:00+05-00",
            "2026-10-05T00:00:00+24:00",
            "+026-10-05T00:00:00Z",
            "2026-1\u{e9}5T00:00:00Z",
            "2026-10-05T00:00:\u{e9}Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?}");
        }
        assert_eq!(
            parse_rfc3339("2026-10-05T05:30:00+05:30"),
            parse_rfc3339("2026-10-05T00:00:00Z")
        );
    }

    const S: &str = "https://s/";

    #[test]
    fn policies_validate() {
        let ok = json!([{"type": "AccessPolicy", "action": ["read"], "assignee": FOAF_AGENT,
                         "target": {"type": "StorageResource", "value": ["https://s/x"]}}]);
        assert_eq!(policies(&ok, S).unwrap().len(), 1);
        let bad_action = json!([{"type": "AccessPolicy", "action": ["fly"], "assignee": "a",
                                 "target": {"type": "StorageResource", "value": "x"}}]);
        assert!(policies(&bad_action, S).is_none());
        let bad_constraint = json!([{"type": "AccessPolicy", "action": "read", "assignee": "a",
                                     "target": {"type": "StorageResource", "value": "x"},
                                     "constraint": [{"leftOperand": "colour", "operator": "eq", "rightOperand": "red"}]}]);
        assert!(policies(&bad_constraint, S).is_none());
        assert!(policies(&json!([]), S).is_none());
        // A target needs a supported matcher type and one or more value strings.
        for target in [
            json!({"value": ["https://s/x"]}),
            json!({"type": "Folder", "value": ["https://s/x"]}),
            json!({"type": "Container", "value": []}),
            json!({"type": "Container", "value": [1]}),
            json!({"type": "Container"}),
            json!("https://s/x"),
        ] {
            let p = json!([{"type": "AccessPolicy", "action": "read", "assignee": "a", "target": target}]);
            assert!(policies(&p, S).is_none(), "{p}");
        }
    }

    fn policy(target: Option<Value>) -> Policy {
        let mut p = json!({"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a/"});
        if let Some(t) = target {
            p["target"] = t;
        }
        policies(&json!([p]), S).unwrap().remove(0)
    }

    #[test]
    fn an_untargeted_policy_covers_the_whole_storage() {
        let p = policy(None);
        assert!(p.target.is_none());
        for uri in [
            "https://s/",
            "https://s/a",
            "https://s/a/b/",
            "https://s/a/b/c.txt",
        ] {
            assert!(p.covers(uri), "{uri}");
        }
        // ...and nothing outside the storage it is scoped to.
        assert!(!p.covers("https://other/a"));
        assert!(!p.covers("https://s.evil/a"));
        assert!(p.applies(Some("https://a/"), Action::Read, "https://s/a"));
        assert!(!p.applies(Some("https://a/"), Action::Modify, "https://s/a"));
        // A grant scoped to another storage grants nothing here.
        let elsewhere = policies(
            &json!([{"type": "AccessPolicy", "action": "read", "assignee": "https://a/"}]),
            "https://other/",
        )
        .unwrap()
        .remove(0);
        assert!(!elsewhere.covers("https://s/a"));
    }

    #[test]
    fn target_type_matchers() {
        let values = json!(["https://s/c/", "https://s/c/d"]);
        let data = policy(Some(json!({"type": "DataResource", "value": values})));
        assert!(data.covers("https://s/c/d"));
        assert!(!data.covers("https://s/c/"));
        let container = policy(Some(
            json!({"type": format!("{LWS_NS}Container"), "value": values}),
        ));
        assert!(container.covers("https://s/c/"));
        assert!(!container.covers("https://s/c/d"));
        let any = policy(Some(json!({"type": "StorageResource", "value": values})));
        assert!(any.covers("https://s/c/") && any.covers("https://s/c/d"));
        // Values name resources, not their members: no recursion.
        assert!(!any.covers("https://s/c/e"));
        assert!(!any.covers("https://s/other"));
    }

    #[test]
    fn identity_document_inboxes() {
        let me = "https://alice.example/profile#me";
        assert_eq!(
            inbox_of(
                &json!({"id": me, "inbox": "https://alice.example/inbox/"}),
                me
            )
            .as_deref(),
            Some("https://alice.example/inbox/")
        );
        assert_eq!(
            inbox_of(&json!({"ldp:inbox": {"@id": "https://a/i/"}}), me).as_deref(),
            Some("https://a/i/")
        );
        let graph = json!({"@graph": [
            {"@id": "https://alice.example/profile", "inbox": "https://wrong/"},
            {"@id": me, "http://www.w3.org/ns/ldp#inbox": {"@id": "https://a/i/"}}
        ]});
        assert_eq!(inbox_of(&graph, me).as_deref(), Some("https://a/i/"));
        assert_eq!(inbox_of(&json!({"inbox": "relative/"}), me), None);
        assert_eq!(inbox_of(&json!({"name": "Alice"}), me), None);
    }

    fn access_doc(kind: &str, assignee: &str, inbox: Option<&str>) -> String {
        let mut d = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": [kind],
            "storage": "http://localhost:3000/",
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": assignee}],
        });
        if let Some(i) = inbox {
            d["inbox"] = json!(i);
        }
        d.to_string()
    }

    #[tokio::test]
    async fn access_documents_need_the_lws_context() {
        let (state, _) = test_store::state(100).await;
        let post = |body: String| {
            test_store::request(
                Method::POST,
                GRANTS_PATH,
                &[("content-type", LWS_JSON)],
                &body,
            )
        };
        let agent = Agent::anonymous();
        let ok = access_doc("AccessGrant", "https://a/", None);
        let resp = handle(&state, &post(ok.clone()), &agent).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let mut no_ctx: Value = serde_json::from_str(&ok).unwrap();
        no_ctx.as_object_mut().unwrap().remove("@context");
        let resp = handle(&state, &post(no_ctx.to_string()), &agent).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        no_ctx["@context"] = json!(["https://www.w3.org/ns/odrl.jsonld"]);
        let resp = handle(&state, &post(no_ctx.to_string()), &agent).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn created_grants_link_up_type_and_linkset() {
        let (state, _) = test_store::state(100).await;
        let req = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let resp = handle(&state, &req, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let location = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let container = state.cfg.absolute(GRANTS_PATH);
        assert_eq!(test_store::links(&resp, "up"), vec![container.clone()]);
        assert_eq!(
            test_store::links(&resp, "type"),
            vec![format!("{LWS_NS}DataResource")]
        );
        let linkset = test_store::links(&resp, "linkset");
        assert_eq!(linkset, vec![format!("{location}{META_SUFFIX}")]);
        // The linkset and the container's both resolve.
        let path = linkset[0].strip_prefix(&state.cfg.base_url).unwrap();
        let ls = handle(
            &state,
            &test_store::request(Method::GET, path, &[], ""),
            &Agent::anonymous(),
        )
        .await;
        assert_eq!(ls.status(), StatusCode::OK);
        assert_eq!(
            test_store::body_json(ls).await["linkset"][0]["anchor"],
            location
        );
        let cls = handle(
            &state,
            &test_store::request(Method::GET, &format!("{GRANTS_PATH}{META_SUFFIX}"), &[], ""),
            &Agent::anonymous(),
        )
        .await;
        assert_eq!(cls.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn grant_listing_is_a_paged_container() {
        let (state, _) = test_store::state(2).await;
        for i in 0..3 {
            let req = test_store::request(
                Method::POST,
                GRANTS_PATH,
                &[],
                &access_doc("AccessGrant", &format!("https://a/{i}"), None),
            );
            assert_eq!(
                handle(&state, &req, &Agent::anonymous()).await.status(),
                StatusCode::CREATED
            );
        }
        let get = |q: &str, accept: &str| {
            test_store::request(
                Method::GET,
                &format!("{GRANTS_PATH}{q}"),
                &[("accept", accept)],
                "",
            )
        };
        let first = handle(&state, &get("", "application/ld+json"), &Agent::anonymous()).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], LD_JSON);
        assert!(first.headers().contains_key(header::ETAG));
        assert_eq!(test_store::links(&first, "next").len(), 1);
        let doc = test_store::body_json(first).await;
        assert_eq!(doc["totalItems"], 3);
        assert_eq!(doc["items"].as_array().unwrap().len(), 2);
        let second = handle(&state, &get("?page=2", "*/*"), &Agent::anonymous()).await;
        assert_eq!(test_store::links(&second, "prev").len(), 1);
        let bad = handle(&state, &get("", "text/html"), &Agent::anonymous()).await;
        assert_eq!(bad.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn a_failed_revocation_keeps_the_grant() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let req = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let resp = handle(&state, &req, &Agent::anonymous()).await;
        let location = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let path = location
            .strip_prefix(&state.cfg.base_url)
            .unwrap()
            .to_string();
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        store.fail_delete.store(true, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // Still enforced, still stored.
        assert_eq!(state.access.grant_policies().len(), 1);
        assert!(state.store.exists(&location).await.unwrap());
        store.fail_delete.store(false, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.access.grant_policies().is_empty());
        assert!(!state.store.exists(&location).await.unwrap());
    }

    #[tokio::test]
    async fn grants_reach_the_inboxes_of_their_requests() {
        let (state, _) = test_store::state(100).await;
        let alice = Agent {
            subject: Some("https://alice.example/#me".into()),
            client: None,
        };
        for (assignee, inbox) in [
            ("https://alice.example/#me", "https://alice.example/inbox/"),
            ("https://bob.example/#me", "https://bob.example/inbox/"),
        ] {
            let req = test_store::request(
                Method::POST,
                REQUESTS_PATH,
                &[],
                &access_doc("AccessRequest", assignee, Some(inbox)),
            );
            assert_eq!(
                handle(&state, &req, &alice).await.status(),
                StatusCode::CREATED
            );
        }
        let grant: Value =
            serde_json::from_str(&access_doc("AccessGrant", "https://bob.example/#me", None))
                .unwrap();
        // Bob's request asked for Bob; Alice's request was made by Alice, for Alice.
        assert_eq!(
            requester_inboxes(&state, &grant)
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["https://bob.example/inbox/".to_string()]
        );
        let alice_grant: Value = serde_json::from_str(&access_doc(
            "AccessGrant",
            "https://alice.example/#me",
            None,
        ))
        .unwrap();
        // Alice authored both requests, so both are hers to hear about.
        assert_eq!(requester_inboxes(&state, &alice_grant).len(), 2);
        // A public grant names no requester.
        let public: Value =
            serde_json::from_str(&access_doc("AccessGrant", FOAF_AGENT, None)).unwrap();
        assert!(requester_inboxes(&state, &public).is_empty());
    }

    #[tokio::test]
    async fn an_owner_without_an_identity_document_has_no_inbox() {
        let (state, _) = test_store::state(100).await;
        assert_eq!(owner_inbox(&state).await, None);
    }

    #[test]
    fn constraints() {
        let c = Constraint {
            left: "client".into(),
            operator: "eq".into(),
            right: json!("app"),
        };
        let types = vec![format!("{LWS_NS}DataResource")];
        assert!(c.satisfied(&ConstraintContext {
            client: Some("app"),
            format: None,
            types: &types
        }));
        assert!(!c.satisfied(&ConstraintContext {
            client: Some("other"),
            format: None,
            types: &types
        }));
        let t = Constraint {
            left: "type".into(),
            operator: "eq".into(),
            right: json!("DataResource"),
        };
        assert!(t.satisfied(&ConstraintContext {
            client: None,
            format: None,
            types: &types
        }));
        let past = Constraint {
            left: "dateTime".into(),
            operator: "lt".into(),
            right: json!("2000-01-01T00:00:00Z"),
        };
        assert!(!past.satisfied(&ConstraintContext {
            client: None,
            format: None,
            types: &types
        }));
    }
}

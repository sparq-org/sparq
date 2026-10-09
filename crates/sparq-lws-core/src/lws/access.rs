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

use std::collections::BTreeMap;
use std::sync::RwLock;

use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use super::{
    has_lws_context, has_type, jose, json_is_uri, method_not_allowed, problem, service_links,
    service_linkset, service_listing, set, Agent, LwsConfig, LwsRequest, LwsState, ResourceMeta,
    FOAF_AGENT, GRANTS_PATH, LWS_JSON, LWS_NS, META_SUFFIX, REQUESTS_PATH,
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

/// Most access requests held at once, from all authors together.
pub const MAX_REQUESTS: usize = 512;

/// Most access requests one author may hold open at once.
pub const MAX_REQUESTS_PER_AUTHOR: usize = 16;

/// Largest access request document accepted.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// Largest access grant body, in bytes: grants come from the owner, and are held to this while
/// they are read so a request that turns out not to be the owner's buffers no more.
pub const MAX_GRANT_BYTES: usize = 256 * 1024;

const LEFT_OPERANDS: &[&str] = &["client", "format", "type", "purpose", "dateTime"];
const OPERATORS: &[&str] = &["eq", "isAnyOf", "gt", "gteq", "lt", "lteq"];

/// One ODRL constraint of the Access Profile.
#[derive(Debug, Clone, Default)]
pub struct Constraint {
    left: String,
    operator: String,
    right: Value,
    /// An `isAnyOf` operand's strings, as a set: a resource's types are each looked up in it, so
    /// a check costs what the types do, not their product with the operand.
    any_of: std::sync::Arc<std::collections::HashSet<String>>,
}

/// A media type as compared (RFC 9110 section 8.3.1): type, subtype and parameter names are
/// case-insensitive, as is a `charset` value; a value is the same quoted or not; parameters are
/// unordered. `None` when it is not a media type, which matches nothing.
pub(crate) fn media_key(s: &str) -> Option<String> {
    fn token(t: &str) -> bool {
        !t.is_empty()
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    }
    let mut rest = s.trim();
    let end = rest.find(';').unwrap_or(rest.len());
    let (ty, sub) = rest[..end].trim().split_once('/')?;
    if !token(ty) || !token(sub) {
        return None;
    }
    let mut key = format!("{}/{}", ty.to_ascii_lowercase(), sub.to_ascii_lowercase());
    rest = &rest[end..];
    let mut params = Vec::new();
    while let Some(after) = rest.strip_prefix(';') {
        let after = after.trim_start();
        // An empty segment (`;;` or a trailing `;`) says nothing.
        if after.is_empty() || after.starts_with(';') {
            rest = after;
            continue;
        }
        let (name, tail) = after.split_once('=')?;
        let name = name.trim_end();
        if !token(name) {
            return None;
        }
        let (mut value, tail) = if let Some(quoted) = tail.strip_prefix('"') {
            let mut value = String::new();
            let mut chars = quoted.char_indices();
            let close = loop {
                match chars.next()? {
                    (i, '"') => break i,
                    (_, '\\') => value.push(chars.next()?.1),
                    (_, c) => value.push(c),
                }
            };
            (value, &quoted[close + 1..])
        } else {
            let end = tail.find(';').unwrap_or(tail.len());
            let value = tail[..end].trim_end();
            if !token(value) {
                return None;
            }
            (value.to_string(), &tail[end..])
        };
        let tail = tail.trim_start();
        if !(tail.is_empty() || tail.starts_with(';')) {
            return None;
        }
        let name = name.to_ascii_lowercase();
        if name == "charset" {
            value.make_ascii_lowercase();
        }
        params.push((name, value));
        rest = tail;
    }
    params.sort();
    for (name, value) in params {
        let value = value.replace('\\', "\\\\").replace('"', "\\\"");
        key.push_str(&format!(";{name}=\"{value}\""));
    }
    Some(key)
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
            "format" => ctx
                .format
                .and_then(media_key)
                .is_some_and(|f| self.matches_media(&f)),
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
            "isAnyOf" => self.any_of.contains(actual),
            _ => false,
        }
    }

    /// [`Self::matches`] for media types, compared as [`media_key`]s: an `isAnyOf` operand's
    /// were keyed when the policy was read.
    fn matches_media(&self, actual: &str) -> bool {
        match self.operator.as_str() {
            "eq" => self.right.as_str().and_then(media_key).as_deref() == Some(actual),
            "isAnyOf" => self.any_of.contains(actual),
            _ => false,
        }
    }

    fn compare_now(&self) -> bool {
        let Some(bound) = self.right.as_str().and_then(parse_rfc3339_nanos) else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i128);
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

/// An RFC 3339 / xsd:dateTime instant, as whole seconds since the epoch (any fraction dropped).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    parse_rfc3339_nanos(s).map(|n| n.div_euclid(1_000_000_000) as i64)
}

/// An RFC 3339 / xsd:dateTime instant, as nanoseconds since the epoch: exact, so a temporal
/// constraint compares the instant it states, not one rounded to its second. A date the calendar
/// does not have (February 31, a leap second) and a fraction finer than a nanosecond are refused.
///
/// Parsed over bytes with every field checked to be ASCII digits before it is read, so malformed
/// (including non-ASCII) input is `None`, never a slice-on-a-char-boundary panic.
pub fn parse_rfc3339_nanos(s: &str) -> Option<i128> {
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
    let mut nanos: i128 = 0;
    if let Some(frac) = rest.strip_prefix(b".") {
        let digits = frac.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        nanos = i128::from(ascii_num(frac, 0, digits)?) * 10i128.pow(9 - digits as u32);
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
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = match mo {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&mo) || !(1..=month_days).contains(&d) || h > 23 || mi > 59 || sec > 59 {
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
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset;
    Some(i128::from(secs) * 1_000_000_000 + nanos)
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
    pub storage: std::sync::Arc<str>,
    pub constraints: Vec<Constraint>,
}

impl Policy {
    fn applies(&self, subject: Option<&str>, action: Action, uri: &str) -> bool {
        (self.assignee == FOAF_AGENT || Some(self.assignee.as_str()) == subject)
            && self.actions.contains(&action)
            && self.covers(uri)
    }

    /// Whether the policy covers `uri`: always within the grant's storage, and then the resources
    /// its target names, or the whole storage when it names none.
    pub fn covers(&self, uri: &str) -> bool {
        let scope = self.storage.trim_end_matches('/');
        let in_storage = !scope.is_empty()
            && uri
                .strip_prefix(scope)
                .is_some_and(|rest| rest.starts_with('/'));
        in_storage
            && match &self.target {
                Some(t) => t.kind.matches(uri) && t.values.iter().any(|v| v == uri),
                None => true,
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
    /// The share of the store access requests may take (see [`super::Quota`]).
    request_quota: super::Quota,
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
            request_quota: super::Quota::new(MAX_REQUESTS, MAX_REQUESTS_PER_AUTHOR),
        };
        for grants in [true, false] {
            let container = cfg.absolute(if grants { GRANTS_PATH } else { REQUESTS_PATH });
            ensure_container(store, &container).await?;
            let children = store
                .list_children(&container)
                .await
                .map_err(|e| format!("store: {e}"))?;
            for child in children {
                // Every stored request counts against the request quota, so one that cannot be read
                // stops the server rather than leaving a place uncounted; a grant that cannot be
                // read grants nothing.
                let stored = match store.read(child.as_str()).await {
                    Ok(r) => serde_json::from_slice::<Value>(&r.body).ok(),
                    Err(e) if !grants => {
                        return Err(format!("store: access request {}: {e}", child.as_str()))
                    }
                    Err(_) => None,
                };
                let stored = match stored {
                    Some(v) => v,
                    None if !grants => {
                        return Err(format!(
                            "access request {} is not a stored record",
                            child.as_str()
                        ))
                    }
                    None => continue,
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

/// Whether a constraint's right operand is one its operator can be evaluated against: `isAnyOf` a
/// non-empty list of strings and every other operator one string; a `dateTime` an exact instant
/// compared by order or equality; `client`, `format` and `type` compared by equality or
/// membership. A constraint that fails this is malformed, and its grant with it, so none is ever
/// enforced in part.
fn operand_valid(left: &str, op: &str, right: &Value) -> bool {
    let strings = |v: &Value| v.as_str().is_some_and(|s| !s.is_empty());
    let shape = match op {
        "isAnyOf" => right
            .as_array()
            .is_some_and(|a| !a.is_empty() && a.iter().all(strings)),
        _ => strings(right),
    };
    shape
        && match left {
            "dateTime" => op != "isAnyOf" && right.as_str().and_then(parse_rfc3339_nanos).is_some(),
            "client" | "format" | "type" => matches!(op, "eq" | "isAnyOf"),
            _ => true,
        }
}

/// The AccessPolicy entries of `access` in a document scoped to `storage`, or `None` when they are
/// malformed.
pub fn policies(access: &Value, storage: &str) -> Option<Vec<Policy>> {
    let list = access.as_array().filter(|a| !a.is_empty())?;
    // Every policy shares the one copy of the grant's storage.
    let storage: std::sync::Arc<str> = storage.into();
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
                    if !operand_valid(left, op, &right) {
                        return None;
                    }
                    let any_of = right
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter_map(|v| match left {
                            "format" => media_key(v),
                            _ => Some(v.to_string()),
                        })
                        .collect();
                    out.push(Constraint {
                        left: left.into(),
                        operator: op.into(),
                        right,
                        any_of: std::sync::Arc::new(any_of),
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
            storage: storage.clone(),
            constraints,
        });
    }
    Some(out)
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
                // A conditional create is evaluated against the listing.
                let held = match super::service_preconditions(state, req, &container, |get| {
                    listing(state, get, grants)
                })
                .await
                {
                    Ok(held) => held,
                    Err(refused) => return refused,
                };
                create(state, req, agent, grants, held).await
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
            let refusal = super::resources::read_refusal(req, &record.etag);
            if refusal == Some(StatusCode::PRECONDITION_FAILED) {
                return problem(StatusCode::PRECONDITION_FAILED, None);
            }
            if refusal.is_some() {
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
            if let Some(refused) =
                super::resources::unless_preconditions(req, Some(&record.etag), None)
            {
                return refused;
            }
            // A revocation is durable before it is reported: a grant whose stored copy survives
            // would be reloaded, and so reinstated, at the next boot. Once the stored record is
            // gone the grant is revoked in memory too, whatever happens after (a cleanup that
            // fails, a client that goes away): the removal and the revocation run in a task of
            // their own.
            let revoke = {
                let (state, iri, container, id, admission) = (
                    state.clone(),
                    iri.clone(),
                    container.clone(),
                    id.to_string(),
                    req.admission.clone(),
                );
                async move {
                    let _admission = admission;
                    // Shared with other members' changes; a conditional create holds it alone.
                    let _listing = state.locks.read(&container).await;
                    super::delete_record(&state.store, &iri, &container).await?;
                    state.access.map(grants).write().expect("lock").remove(&id);
                    state.access.bump(grants);
                    Ok::<_, ServerError>(())
                }
            };
            match tokio::spawn(revoke).await {
                Ok(Ok(())) => problem(StatusCode::NO_CONTENT, None),
                Ok(Err(e)) => problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(&format!("cannot delete {iri}: {e}")),
                ),
                Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string())),
            }
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
    held: Option<super::resources::IriGuard>,
) -> Response {
    // Anyone authenticated may ask for access, so a request is held to its size before it is
    // parsed; grants come from the owner and are held to the request body ceiling.
    let body = if grants {
        serde_json::from_slice::<Value>(&req.body)
            .map_err(|_| problem(StatusCode::BAD_REQUEST, Some("the body is not JSON")))
    } else {
        super::bounded_json(&req.body, MAX_REQUEST_BYTES, "an access request")
    };
    let body = match body {
        Ok(b) => b,
        Err(r) => return r,
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
    // A grant or request is about this storage only: one scoped to another is refused, not
    // stored inert.
    if valid && storage.trim_end_matches('/') != state.cfg.storage().trim_end_matches('/') {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("an access document's storage must be this storage"),
        );
    }
    let Some(parsed) = valid
        .then(|| policies(body.get("access").unwrap_or(&Value::Null), storage))
        .flatten()
    else {
        return problem(StatusCode::BAD_REQUEST, Some("not a valid access document"));
    };
    // Each request is stored as a resource is: requests are held to a share of their own, overall
    // and per author, so asking can never fill the storage that resources and grants need. The
    // place is reserved before anything is stored and held until the request is registered.
    let slot = if grants {
        None
    } else {
        let reserved = state
            .access
            .request_quota
            .reserve(agent.subject.as_deref(), || {
                let requests = state.access.requests.read().expect("lock");
                let mine = requests
                    .values()
                    .filter(|r| r.author == agent.subject)
                    .count();
                (requests.len(), mine)
            });
        match reserved {
            Ok(slot) => Some(slot),
            Err(full) => return full.response("open access requests"),
        }
    };
    let id = jose::random_id();
    let base = if grants { GRANTS_PATH } else { REQUESTS_PATH };
    let container = state.cfg.absolute(base);
    let iri = format!("{container}{id}");
    let mut document = body.clone();
    document["id"] = Value::String(iri.clone());
    let stored = json!({"document": document, "author": agent.subject});
    let record = Record {
        id: id.clone(),
        document: document.clone(),
        policies: if grants { parsed } else { Vec::new() },
        author: agent.subject.clone(),
        etag: new_etag(),
    };
    let register = {
        let state = state.clone();
        move || {
            state
                .access
                .map(grants)
                .write()
                .expect("lock")
                .insert(id, record);
            state.access.bump(grants);
            // Counted as registered from here on.
            drop(slot);
        }
    };
    let created = super::create_record(
        state,
        &container,
        &iri,
        Bytes::from(stored.to_string()),
        req.admission.clone(),
        held,
        register,
    );
    if let Err(e) = created.await {
        return problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    }
    let mut resp = problem(StatusCode::CREATED, None);
    set(resp.headers_mut(), header::LOCATION, &iri);
    service_links(&state.cfg, resp.headers_mut(), &iri, &container);
    resp
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

    /// Review finding: a targeted policy was never held to its grant's storage, so a grant
    /// scoped to another storage still authorized the resources its target named here. Every
    /// policy covers its storage only, and a grant scoped to another is refused when posted.
    #[tokio::test]
    async fn grants_hold_to_their_storage() {
        let target = json!({"type": "StorageResource", "value": ["https://s/x"]});
        let elsewhere = policies(
            &json!([{"type": "AccessPolicy", "action": "read", "assignee": "https://a/",
                     "target": target}]),
            "https://other.example/",
        )
        .unwrap()
        .remove(0);
        assert!(!elsewhere.covers("https://s/x"));
        assert!(policy(Some(target)).covers("https://s/x"));
        let (state, _) = test_store::state(100).await;
        let mut doc: Value =
            serde_json::from_str(&access_doc("AccessGrant", "https://a/", None)).unwrap();
        doc["storage"] = json!("https://other.example/");
        let req = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[("content-type", LWS_JSON)],
            &doc.to_string(),
        );
        let resp = handle(&state, &req, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(state.access.grant_policies().is_empty());
    }

    /// Review finding: a temporal bound and the current time were both cut to the second, so a
    /// bound's fraction was ignored either way, and dates the calendar does not have were read
    /// as later ones. Instants are exact to the nanosecond, and impossible dates are refused.
    #[test]
    fn temporal_constraints_compare_exact_instants() {
        let ns = |s: &str| parse_rfc3339_nanos(s);
        assert_eq!(
            ns("2026-10-09T12:00:00.9Z").unwrap() - ns("2026-10-09T12:00:00.1Z").unwrap(),
            800_000_000
        );
        assert_eq!(ns("1970-01-01T00:00:00.000000001Z"), Some(1));
        for bad in [
            "2026-02-31T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2026-10-09T23:59:60Z",
            "2026-10-09T12:00:00.1234567891Z",
        ] {
            assert_eq!(ns(bad), None, "{bad}");
        }
        assert!(ns("2024-02-29T00:00:00Z").is_some());
        // A bound a few milliseconds past is past, whatever second it falls in.
        let past = std::time::SystemTime::now() - std::time::Duration::from_millis(5);
        let d = past.duration_since(std::time::UNIX_EPOCH).unwrap();
        let bound = format!(
            "{}.{:09}Z",
            format_rfc3339(d.as_secs() as i64).trim_end_matches('Z'),
            d.subsec_nanos()
        );
        let constraint = |op: &str| Constraint {
            left: "dateTime".into(),
            operator: op.into(),
            right: json!(bound),
            any_of: Default::default(),
        };
        let ctx = ConstraintContext {
            client: None,
            format: None,
            types: &[],
        };
        assert!(!constraint("lteq").satisfied(&ctx));
        assert!(!constraint("eq").satisfied(&ctx));
        assert!(constraint("gt").satisfied(&ctx));
    }

    /// Review finding: an `isAnyOf` operand's members that were not strings were dropped, so a
    /// malformed constraint was enforced in part. A constraint whose operand its operator cannot
    /// evaluate makes the whole grant malformed.
    #[test]
    fn malformed_operands_refuse_the_grant() {
        let with = |left: &str, op: &str, right: Value| {
            policies(
                &json!([{"type": "AccessPolicy", "action": "read", "assignee": "https://a/",
                         "constraint": [{"leftOperand": left, "operator": op, "rightOperand": right}]}]),
                S,
            )
        };
        assert!(with("format", "isAnyOf", json!(["text/plain"])).is_some());
        assert!(with("dateTime", "lt", json!("2999-01-01T00:00:00Z")).is_some());
        for (left, op, right) in [
            ("format", "isAnyOf", json!(["text/plain", 42])),
            ("format", "isAnyOf", json!([])),
            ("format", "isAnyOf", json!("text/plain")),
            ("format", "eq", json!(["text/plain"])),
            ("format", "gt", json!("text/plain")),
            ("client", "lteq", json!("https://app/")),
            ("dateTime", "isAnyOf", json!(["2999-01-01T00:00:00Z"])),
            ("dateTime", "lt", json!("2026-02-31T00:00:00Z")),
            ("dateTime", "lt", json!("soon")),
            ("type", "eq", json!("")),
        ] {
            assert!(
                with(left, op, right.clone()).is_none(),
                "{left} {op} {right}"
            );
        }
    }

    /// Review finding: a stored access request that could not be read at startup was skipped,
    /// and so left out of the request quota while it stayed stored. A request that cannot be
    /// accounted for stops the load.
    #[tokio::test]
    async fn requests_that_cannot_be_read_stop_the_load() {
        let (state, store) = test_store::state(100).await;
        let req = test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[("content-type", LWS_JSON)],
            &access_doc("AccessRequest", "https://a/", None),
        );
        let resp = handle(&state, &req, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let iri = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        assert!(AccessStore::load(&store, &state.cfg).await.is_ok());
        *store.fail_read_of.lock().unwrap() = Some(iri);
        assert!(AccessStore::load(&store, &state.cfg).await.is_err());
    }

    /// Review finding (atomicity): a request's store record, its registration and its place in
    /// the request quota must change together. Failing each store step of a create and of a
    /// revoke in turn leaves the store as it was, and what is in force in memory (and counted)
    /// always matches what a restart would load.
    #[tokio::test]
    async fn requests_are_stored_and_counted_together() {
        use test_store::each_failure_changes_nothing;
        let setup = |with_one: bool| async move {
            let (state, store) = test_store::state(100).await;
            let container = state.cfg.absolute(REQUESTS_PATH);
            let mut iris = Vec::new();
            if with_one {
                let req = test_store::request(
                    Method::POST,
                    REQUESTS_PATH,
                    &[("content-type", LWS_JSON)],
                    &access_doc("AccessRequest", "https://a/", None),
                );
                let resp = handle(&state, &req, &Agent::anonymous()).await;
                assert_eq!(resp.status(), StatusCode::CREATED);
                iris.push(
                    resp.headers()[header::LOCATION]
                        .to_str()
                        .unwrap()
                        .to_string(),
                );
            }
            (state, store, iris, vec![container])
        };
        // In force and counted exactly when stored.
        let agrees = |state: LwsState<test_store::FlakyStore>| async move {
            let loaded = AccessStore::load(&state.store, &state.cfg).await.unwrap();
            let stored = loaded.requests.read().unwrap().len();
            assert_eq!(state.access.requests.read().unwrap().len(), stored);
        };
        let steps = each_failure_changes_nothing(
            || setup(false),
            |state| async move {
                let req = test_store::request(
                    Method::POST,
                    REQUESTS_PATH,
                    &[("content-type", LWS_JSON)],
                    &access_doc("AccessRequest", "https://a/", None),
                );
                let status = handle(&state, &req, &Agent::anonymous()).await.status();
                agrees(state).await;
                status
            },
        )
        .await;
        assert!(steps >= 1, "a request took {steps} steps");
        each_failure_changes_nothing(
            || setup(true),
            |state| async move {
                let id = state
                    .access
                    .requests
                    .read()
                    .unwrap()
                    .keys()
                    .next()
                    .cloned()
                    .unwrap();
                let path = format!("{REQUESTS_PATH}{id}");
                let req = test_store::request(Method::DELETE, &path, &[], "");
                let status = handle(&state, &req, &Agent::anonymous()).await.status();
                agrees(state).await;
                status
            },
        )
        .await;
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

    /// Review findings: a service linkset answered 200 whatever the request's preconditions, and
    /// access documents were parsed whatever their content coding.
    #[tokio::test]
    async fn service_linksets_evaluate_preconditions() {
        let (state, _) = test_store::state(100).await;
        let path = format!("{GRANTS_PATH}{META_SUFFIX}");
        let get = |h: &[(&str, &str)]| test_store::request(Method::GET, &path, h, "");
        let r = handle(&state, &get(&[]), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::OK);
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let status = |h: &[(&str, &str)]| {
            let req = get(h);
            let state = state.clone();
            async move { handle(&state, &req, &Agent::anonymous()).await.status() }
        };
        assert_eq!(
            status(&[("if-none-match", &tag)]).await,
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            status(&[("if-match", "\"never-issued\"")]).await,
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(status(&[("if-match", &tag)]).await, StatusCode::OK);
        // A coded access document is refused before it is parsed, as any coded body is.
        let coded = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[("content-type", LWS_JSON), ("content-encoding", "gzip")],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let r = super::super::route(&state, coded).await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(state.access.grant_policies().is_empty());
    }

    /// Review finding: a create that failed before it committed, whose removal and lookup failed
    /// too (a backend outage), was registered anyway: a grant in force with nothing stored. It
    /// is never put in force: after a few tries its removal is set aside, with the container,
    /// until the store answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_uncertain_create_is_in_force_only_once_seen_stored() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;
        let (state, store) = test_store::state(100).await;
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        // The create fails before it lands; removing it and looking it up fail too, so the
        // container is set aside until the store answers.
        *store.fail_step.lock().unwrap() = Some(0);
        store.fail_delete.store(true, Ordering::SeqCst);
        store.fail_exists.store(true, Ordering::SeqCst);
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let container = state.cfg.absolute(GRANTS_PATH);
        assert!(state.is_set_aside(&container));
        let listing = test_store::request(Method::GET, GRANTS_PATH, &[], "");
        let r = super::super::route(&state, listing).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(state.access.grant_policies().is_empty(), "in force unseen");
        // The store recovers: the record is not there, and never comes into force.
        store.fail_delete.store(false, Ordering::SeqCst);
        store.fail_exists.store(false, Ordering::SeqCst);
        for _ in 0..200 {
            if !state.is_set_aside(&container) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!state.is_set_aside(&container));
        assert!(state.access.grant_policies().is_empty());
        assert!(state
            .store
            .list_children(&state.cfg.absolute(GRANTS_PATH))
            .await
            .unwrap()
            .is_empty());
    }

    /// Review finding: a grant whose create committed but reported a failure (a remote store's
    /// lost reply), or whose client went away while it was pending, was stored but never put in
    /// force in memory: it could be neither listed nor revoked, and came into force at the next
    /// boot. Such a record is now removed (set aside until it is), and a create that was sent is
    /// registered whether or not its client is still there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_created_grant_is_never_stored_unregistered() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        use std::time::Duration;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(GRANTS_PATH);
        let post = || {
            test_store::request(
                Method::POST,
                GRANTS_PATH,
                &[],
                &access_doc("AccessGrant", "https://a/", None),
            )
        };
        let stored = || async { state.store.list_children(&container).await.unwrap().len() };
        // The create lands, then reports a failure: the record is removed.
        store.fail_after_create.store(true, Ordering::SeqCst);
        let resp = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(stored().await, 0);
        assert!(state.access.grant_policies().is_empty());
        // And when it cannot be removed yet, the create ends (500) and the record and its
        // container are set aside, the container held, until it can be: the record is never
        // stored out of force, nor in force once refused.
        store.fail_delete.store(true, Ordering::SeqCst);
        let resp = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(state.is_set_aside(&container));
        assert!(state.access.grant_policies().is_empty());
        store.fail_after_create.store(false, Ordering::SeqCst);
        store.fail_delete.store(false, Ordering::SeqCst);
        for _ in 0..200 {
            if !state.is_set_aside(&container) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!state.is_set_aside(&container));
        assert_eq!(stored().await, 0);
        assert!(state.access.grant_policies().is_empty());
        // The client goes away while the create is pending: once it lands, the grant is in force.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_create.lock().unwrap() = Some(gate.clone());
        let cancelled = tokio::time::timeout(
            Duration::from_millis(50),
            handle(&state, &post(), &Agent::anonymous()),
        )
        .await;
        assert!(cancelled.is_err());
        gate.add_permits(1);
        for _ in 0..200 {
            if !state.access.grant_policies().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(stored().await, 1);
        assert_eq!(state.access.grant_policies().len(), 1);
    }

    /// Review finding: a revocation removed the stored grant, then waited on the cleanup of its
    /// bytes; when that failed, or the client went away, the grant stayed in force in memory
    /// though its record was gone. The index commit is now the point of revocation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_revocation_takes_effect_once_the_record_is_gone() {
        use std::sync::Arc;
        use std::time::Duration;
        let (state, store) = test_store::state(100).await;
        let grant = || async {
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
            (
                location,
                test_store::request(Method::DELETE, &path, &[], ""),
            )
        };
        // The cleanup after the index commit fails: the grant is revoked all the same.
        let (location, delete) = grant().await;
        *store.fail_after_delete_of.lock().unwrap() = Some(location.clone());
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!state.store.exists(&location).await.unwrap());
        assert!(state.access.grant_policies().is_empty());
        // The client goes away while the removal is pending: once it lands, the grant is gone.
        let (location, delete) = grant().await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((location.clone(), gate.clone()));
        let cancelled = tokio::time::timeout(
            Duration::from_millis(50),
            handle(&state, &delete, &Agent::anonymous()),
        )
        .await;
        assert!(cancelled.is_err());
        assert_eq!(state.access.grant_policies().len(), 1);
        gate.add_permits(1);
        for _ in 0..200 {
            if state.access.grant_policies().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(state.access.grant_policies().is_empty());
    }

    /// Sweep finding: an `isAnyOf` type constraint compared every type of the resource with
    /// every operand. The operand is a set, built once with the grant.
    #[test]
    fn any_of_constraints_are_sets() {
        let operands: Vec<String> = (0..20_000).map(|i| format!("https://t/{i}")).collect();
        let p = policies(
            &json!([{"type": "AccessPolicy", "action": "read", "assignee": "https://a/",
                "constraint": [{"leftOperand": "type", "operator": "isAnyOf", "rightOperand": operands}]}]),
            "https://s/",
        )
        .unwrap()
        .remove(0);
        let c = &p.constraints[0];
        let mut types: Vec<String> = (0..20_000).map(|i| format!("https://u/{i}")).collect();
        let started = std::time::Instant::now();
        let held = |t: &[String]| {
            c.satisfied(&ConstraintContext {
                client: None,
                format: None,
                types: t,
            })
        };
        assert!(!held(&types));
        types.push("https://t/19999".into());
        assert!(held(&types));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    /// Review finding: anyone authenticated could store access requests without limit, each
    /// costing the storage quota resources need. Requests are held to a share of their own.
    #[tokio::test]
    async fn access_requests_are_held_to_their_share() {
        let (state, _store) = super::super::test_store::state(100).await;
        let body = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": ["AccessRequest"],
            "storage": state.cfg.storage(),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a/"}],
        })
        .to_string();
        let post = |body: &str| {
            super::super::test_store::request(
                Method::POST,
                REQUESTS_PATH,
                &[("content-type", LWS_JSON)],
                body,
            )
        };
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        for _ in 0..MAX_REQUESTS_PER_AUTHOR {
            let r = handle(&state, &post(&body), &asker).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let r = handle(&state, &post(&body), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        let other = Agent {
            subject: Some("https://other.example/#me".into()),
            ..Agent::anonymous()
        };
        let r = handle(&state, &post(&body), &other).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let big = format!(
            "{},\"x\":\"{}\"}}",
            &body[..body.len() - 1],
            "y".repeat(MAX_REQUEST_BYTES)
        );
        let r = handle(&state, &post(&big), &other).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        // The size is checked before the body is parsed, so an oversized body is refused for its
        // size whatever it holds.
        let r = handle(&state, &post(&"[".repeat(MAX_REQUEST_BYTES + 1)), &other).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// Review finding: the request caps were checked, then the request was stored and registered
    /// later, so concurrent requests all passed the check. A place is reserved before anything
    /// is stored.
    #[tokio::test]
    async fn concurrent_access_requests_are_held_to_their_share() {
        let (state, _store) = super::super::test_store::state(100).await;
        let body = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": ["AccessRequest"],
            "storage": state.cfg.storage(),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a/"}],
        })
        .to_string();
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = super::super::test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let all = (0..2 * MAX_REQUESTS_PER_AUTHOR).map(|_| handle(&state, &post, &asker));
        let created = futures_util::future::join_all(all)
            .await
            .iter()
            .filter(|r| r.status() == StatusCode::CREATED)
            .count();
        assert_eq!(created, MAX_REQUESTS_PER_AUTHOR);
    }

    #[test]
    fn constraints() {
        let c = Constraint {
            left: "client".into(),
            operator: "eq".into(),
            right: json!("app"),
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };
        assert!(!past.satisfied(&ConstraintContext {
            client: None,
            format: None,
            types: &types
        }));
    }

    /// Review finding: a format constraint compared media types by spelling, so
    /// `Text/Plain; Charset=UTF-8` did not meet `text/plain;charset=utf-8`. They are compared as
    /// RFC 9110 media types.
    #[test]
    fn format_constraints_compare_media_types() {
        let key = |s: &str| media_key(s);
        assert_eq!(
            key("Text/Plain; Charset=UTF-8"),
            key("text/plain;charset=\"utf-8\"")
        );
        assert_eq!(key("a/b; y=1; x=2"), key("a/b;x=2;y=1;"));
        assert_ne!(key("a/b; x=A"), key("a/b; x=a"));
        assert_ne!(key("text/plain"), key("text/html"));
        assert_eq!(key("a/b; x=\"q;r\""), Some("a/b;x=\"q;r\"".into()));
        for bad in [
            "text",
            "/plain",
            "text/plain; x",
            "a/b; x=\"open",
            "a b/c",
            "a/b; x=1 2",
        ] {
            assert_eq!(key(bad), None, "{bad}");
        }
        let format = |op: &str, right: Value| {
            let any_of = right
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(media_key)
                .collect();
            Constraint {
                left: "format".into(),
                operator: op.into(),
                right,
                any_of: std::sync::Arc::new(any_of),
            }
        };
        let ctx = ConstraintContext {
            client: None,
            format: Some("Text/Plain; Charset=UTF-8"),
            types: &[],
        };
        assert!(format("eq", json!("text/plain;charset=utf-8")).satisfied(&ctx));
        assert!(format(
            "isAnyOf",
            json!(["text/html", "TEXT/plain; charset=\"UTF-8\""])
        )
        .satisfied(&ctx));
        assert!(!format("eq", json!("text/plain")).satisfied(&ctx));
    }
}

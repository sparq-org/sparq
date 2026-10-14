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
//! [`GRANTS_PATH`] and [`REQUESTS_PATH`]. Only the owner
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
    has_lws_context, has_type, is_uri, jose, json_is_uri, method_not_allowed, problem,
    service_links, service_linkset, service_listing, set, Agent, LwsConfig, LwsRequest, LwsState,
    ResourceMeta, FOAF_AGENT, GRANTS_PATH, LWS_JSON, LWS_NS, META_SUFFIX, REQUESTS_PATH,
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
    /// Stored, but not readable as a record at the start: it grants nothing and is served to
    /// nobody, but is listed, counted against the request quota, and removed (its place freed)
    /// by the owner's DELETE of its IRI, as any record is.
    pub quarantined: bool,
}

/// The grants and requests of the storage, in memory and in the store.
pub struct AccessStore {
    grants: RwLock<BTreeMap<String, Record>>,
    requests: RwLock<BTreeMap<String, Record>>,
    /// The owner's inbox as last looked up, and when (see [`owner_inbox_cached`]).
    owner_inbox: tokio::sync::Mutex<Option<(std::time::Instant, Option<String>)>>,
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
            quarantined: false,
        };
        self.grants.write().expect("lock").insert(id.into(), record);
    }

    /// Revoke the grant `id`, in memory only.
    #[cfg(test)]
    pub fn remove_grant_for_test(&self, id: &str) {
        self.grants.write().expect("lock").remove(id);
    }

    /// Holding no grant or request.
    pub(crate) fn empty() -> Self {
        Self {
            grants: RwLock::new(BTreeMap::new()),
            requests: RwLock::new(BTreeMap::new()),
            owner_inbox: Default::default(),
            grants_etag: RwLock::new(new_etag()),
            requests_etag: RwLock::new(new_etag()),
            request_quota: super::Quota::new(MAX_REQUESTS, MAX_REQUESTS_PER_AUTHOR),
        }
    }

    /// Ensure the two service containers exist and load what they hold.
    pub async fn load<S: Store>(store: &S, cfg: &LwsConfig) -> Result<Self, String> {
        Self::load_with(store, cfg, |_| true).await
    }

    /// As [`AccessStore::load`], skipping the grants not `visible` (a change to them, put back
    /// at boot, is still being settled: see the `intents` module). What the listing holds is
    /// what is in force: a create or a revocation is answered only once the store confirms it.
    pub(crate) async fn load_with<S: Store>(
        store: &S,
        cfg: &LwsConfig,
        visible: impl Fn(&str) -> bool,
    ) -> Result<Self, String> {
        let me = Self::empty();
        for grants in [true, false] {
            let container = cfg.absolute(if grants { GRANTS_PATH } else { REQUESTS_PATH });
            ensure_container(store, &container).await?;
            let children = store
                .list_children(&container)
                .await
                .map_err(|e| format!("store: {e}"))?;
            for child in children {
                // A record a change put back at start is still settling is not read at all: what
                // it holds may be from before that change, and a withdrawn request must not come
                // back while its removal lands.
                if !visible(child.as_str()) {
                    continue;
                }
                let id = child
                    .as_str()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let stored = match store.read(child.as_str()).await {
                    Ok(r) => serde_json::from_slice::<Value>(&r.body)
                        .ok()
                        .filter(|v| v.get("document").is_some_and(Value::is_object)),
                    // Listed but gone: nothing of it is stored.
                    Err(crate::error::ServerError::NotFound) => continue,
                    // A read that fails says nothing of what the record holds: the start stops
                    // rather than decide without it (a later start reads it again).
                    Err(e) => return Err(format!("store: {}: {e}", child.as_str())),
                };
                // A record whose stored body is not a record never stops the start: it is kept
                // by its IRI, quarantined (see [`Record::quarantined`]). What makes it so is what
                // is stored, so every later start quarantines it too, until the owner deletes it.
                let Some(stored) = stored else {
                    eprintln!(
                        "lws: access {} {} is not a record; it is quarantined until deleted",
                        if grants { "grant" } else { "request" },
                        child.as_str()
                    );
                    let record = Record {
                        id: id.clone(),
                        document: Value::Null,
                        policies: Vec::new(),
                        author: None,
                        etag: new_etag(),
                        quarantined: true,
                    };
                    me.map(grants).write().expect("lock").insert(id, record);
                    continue;
                };
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
                    quarantined: false,
                };
                me.map(grants).write().expect("lock").insert(id, record);
            }
        }
        // Grants have no quota on their number: only the owner makes them.
        if let Some(past) = me.requests_past_quota() {
            super::report_past_quota("access requests", &cfg.absolute(REQUESTS_PATH), &past);
        }
        Ok(me)
    }

    /// What of the access requests in force is past their quota, counted as a new request is
    /// admitted; `None` when nothing is.
    pub(crate) fn requests_past_quota(&self) -> Option<String> {
        let requests = self.requests.read().expect("lock");
        self.request_quota.past(
            requests.len(),
            requests.values().map(|r| r.author.as_deref()),
        )
    }

    /// Take what `loaded` holds in place of what this holds.
    pub(crate) fn replace(&self, loaded: AccessStore) {
        *self.grants.write().expect("lock") = loaded.grants.into_inner().expect("lock");
        *self.requests.write().expect("lock") = loaded.requests.into_inner().expect("lock");
        self.bump(true);
        self.bump(false);
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

    /// The policies of every grant that cover `uri` for `action`, whoever they are assigned to.
    pub(crate) fn policies_covering(&self, action: Action, uri: &str) -> Vec<Policy> {
        self.grants
            .read()
            .expect("lock")
            .values()
            .flat_map(|r| r.policies.iter())
            .filter(|p| p.actions.contains(&action) && p.covers(uri))
            .cloned()
            .collect()
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

/// Whether `agent` may learn that the set-aside resource at `uri` is set aside: never whether it
/// may be served or changed, which waits until it is put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aside {
    /// It may act on it.
    Allowed,
    /// It may not.
    Denied,
    /// The resource was not there before the stuck change (to anyone but the owner).
    Missing,
}

/// [`Aside`] for `agent` and `action` on the set-aside resource at `uri`, decided as
/// [`allowed`] decided before the stuck change to it began: on what it was then (whether it was
/// there, its metadata and its format, see `Prior`), with the grants in force
/// now. What the change may have written meanwhile (new content, new types, a `pending` mark)
/// is never read for it: a grant the change would newly satisfy is not met, and one it would
/// stop meeting still is. What the change left untouched is read as stored. An error when that
/// cannot be read.
pub async fn aside<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
) -> Result<Aside, ServerError> {
    if state.cfg.open || is_owner(state, agent) {
        return Ok(Aside::Allowed);
    }
    let prior = state.locks.prior(uri);
    let existed = match prior.existed {
        Some(existed) => existed,
        None => state.store.exists(uri).await?,
    };
    if !existed {
        return Ok(Aside::Missing);
    }
    let meta = match prior.meta {
        Some(meta) => meta,
        None => state.resource_meta(uri).await?,
    };
    Ok(
        if decide(state, action, uri, agent, &meta, prior.format).await? {
            Aside::Allowed
        } else {
            Aside::Denied
        },
    )
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

impl Snapshot {
    /// The snapshot of a resource whose state could not be read: its metadata marked pending, so
    /// that, past the owner (and open mode), every grant fails closed against it, as a read of
    /// the resource then would.
    pub fn unreadable() -> Self {
        Self {
            meta: ResourceMeta {
                pending: true,
                ..ResourceMeta::default()
            },
            format: None,
        }
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

pub(crate) fn is_owner<S: Store + 'static>(state: &LwsState<S>, agent: &Agent) -> bool {
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
    let format = match format {
        Some(f) => f,
        None => format_of(state, uri).await?,
    };
    Ok(grants_allow(
        candidates.iter(),
        agent,
        uri,
        format.as_deref(),
        meta,
    ))
}

/// Whether one of `candidates` (grant policies that apply to `agent` for the action on `uri`)
/// has every constraint satisfied, against the resource's `format` and `meta`.
fn grants_allow<'p>(
    mut candidates: impl Iterator<Item = &'p Policy>,
    agent: &Agent,
    uri: &str,
    format: Option<&str>,
    meta: &ResourceMeta,
) -> bool {
    let is_container = uri.ends_with('/');
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
        format,
        types: &types,
    };
    candidates.any(|p| p.constraints.iter().all(|c| c.satisfied(&ctx)))
}

/// One resource's access for one action, read once and decided for many agents as [`allowed`]
/// decides for one: a change's deliveries are each authorized against it, so the resource's
/// metadata is read, and the grants scanned, once per change rather than once per subscriber.
/// The grants are those in force when it is taken.
pub(crate) struct ResourceAccess {
    uri: String,
    /// `None` when the metadata could not be read: past the owner, everyone is refused.
    meta: Option<ResourceMeta>,
    /// The grant policies covering `uri` for the action, whoever they are assigned to.
    policies: Vec<Policy>,
    /// The resource's format, looked up only when a policy may apply; `Err` when it could not be.
    format: Result<Option<String>, ()>,
}

impl ResourceAccess {
    /// `uri`'s access for `action` as it stands now.
    pub(crate) async fn take<S: Store + 'static>(
        state: &LwsState<S>,
        action: Action,
        uri: &str,
    ) -> Self {
        let meta = state.resource_meta(uri).await.ok();
        let policies = match &meta {
            Some(m) if !m.pending => state.access.policies_covering(action, uri),
            _ => Vec::new(),
        };
        let format = if policies.is_empty() {
            Ok(None)
        } else {
            format_of(state, uri).await.map_err(|_| ())
        };
        Self {
            uri: uri.to_string(),
            meta,
            policies,
            format,
        }
    }

    /// Whether `agent` may take the action on the resource, as [`allowed`] decides.
    pub(crate) fn allows<S: Store + 'static>(&self, state: &LwsState<S>, agent: &Agent) -> bool {
        if state.cfg.open || is_owner(state, agent) {
            return true;
        }
        let Some(meta) = &self.meta else {
            return false;
        };
        let subject = agent.subject.as_deref();
        if subject.is_some() && subject == meta.creator.as_deref() {
            return true;
        }
        if meta.pending {
            return false;
        }
        let mut candidates = self
            .policies
            .iter()
            .filter(|p| p.assignee == FOAF_AGENT || Some(p.assignee.as_str()) == subject)
            .peekable();
        if candidates.peek().is_none() {
            return false;
        }
        let Ok(format) = &self.format else {
            return false;
        };
        grants_allow(candidates, agent, &self.uri, format.as_deref(), meta)
    }
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
    // Whether what the request is to is set aside is said only to a caller that may act on it:
    // the owner, who may act on every record, at once; anyone else once authorized below.
    let unavailable = || super::resources::unavailable(state, req);
    if owner {
        if let Some(refused) = unavailable() {
            return refused;
        }
    }
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
                if let Some(refused) = unavailable() {
                    return refused;
                }
                // A conditional create is evaluated against the listing, which only the owner
                // may read: anyone else is refused rather than told how it compares.
                if !owner && super::resources::compares_state(&req.headers, true) {
                    return state.deny(agent);
                }
                let held =
                    match super::records::service_preconditions(state, req, &container, |get| {
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
    if let Some(refused) = unavailable() {
        return refused;
    }
    // A quarantined record (unreadable at the start) is never served; the owner deletes it.
    if record.quarantined && matches!(req.method, Method::GET | Method::HEAD) {
        return problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("this record could not be read; deleting it frees its place"),
        );
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
            // Removed as every service record is ([`super::records::remove_record`]): out of
            // force while its removal runs (its policies are set aside), but still counted, as it
            // is still stored; it leaves only once it is confirmed gone, and is put back in force
            // otherwise.
            let set_aside = {
                let (state, id) = (state.clone(), id.to_string());
                move || {
                    let held = {
                        let mut map = state.access.map(grants).write().expect("lock");
                        let record = map.get_mut(&id)?;
                        std::mem::take(&mut record.policies)
                    };
                    state.access.bump(grants);
                    Some(held)
                }
            };
            let settle = {
                let id = id.to_string();
                let author = record.author.clone();
                move |state: &LwsState<S>, gone: bool, held| {
                    {
                        let mut map = state.access.map(grants).write().expect("lock");
                        if gone {
                            map.remove(&id);
                        } else if let Some(record) = map.get_mut(&id) {
                            record.policies = held;
                        }
                    }
                    state.access.bump(grants);
                    author
                }
            };
            let what = if grants { "grant" } else { "request" };
            let removed = super::records::remove_record(
                state,
                &container,
                &iri,
                what,
                req.admission.clone(),
                set_aside,
                settle,
            );
            match removed.await {
                Ok(super::records::Removed::Gone) => problem(StatusCode::NO_CONTENT, None),
                // Revoked by another request while this one waited.
                Ok(super::records::Removed::Absent) => problem(StatusCode::NOT_FOUND, None),
                Err(refused) => refused,
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
    // The record wraps the document one level deeper than the body was parsed, so a document
    // nested to the parser's limit would be stored and then not read back, which stops every later
    // start. What is stored must parse back.
    let wrapped = json!({"document": &body, "author": agent.subject});
    if serde_json::from_str::<Value>(&wrapped.to_string()).is_err() {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("the access document nests too deeply to store"),
        );
    }
    drop(wrapped);
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
        quarantined: false,
    };
    // "When an inbox property is present on an access request or access grant, the server SHOULD
    // deliver notifications to that endpoint" (section 11.6).
    let own_inbox = body
        .get("inbox")
        .and_then(Value::as_str)
        .filter(|i| is_uri(i))
        .map(str::to_string);
    let activity = json!({"type": ["Create"], "object": {"id": iri, "type": [wanted]}});
    // The record is put in force, counted and announced in one step, once it is stored: a client
    // that goes away after the store committed cannot keep it from being announced.
    let register = {
        move |state: &LwsState<S>| {
            state
                .access
                .map(grants)
                .write()
                .expect("lock")
                .insert(id, record);
            state.access.bump(grants);
            // Counted as registered from here on.
            drop(slot);
            announce(state, grants, &document, own_inbox, activity);
        }
    };
    let created = super::records::create_record(
        state,
        &container,
        &iri,
        Bytes::from(stored.to_string()),
        req.admission.clone(),
        held,
        agent.subject.clone(),
        register,
    );
    if let Err(refused) = created.await {
        return refused;
    }
    let mut resp = problem(StatusCode::CREATED, None);
    set(resp.headers_mut(), header::LOCATION, &iri);
    service_links(&state.cfg, resp.headers_mut(), &iri, &container);
    resp
}

/// Announce a new grant or request (`document`, by `activity`): at its own inbox, and
/// - for a grant, "the requesting agent SHOULD be notified at the inbox specified in the
///   associated access request." The draft gives a grant no link to its request, so the
///   associated requests are those by or for an agent the grant names.
/// - for a request, "the storage controller SHOULD be notified": at the inbox the owner's
///   identity document names, looked up in the background. The lookup holds a place in the
///   delivery queue (so lookups are as bounded as deliveries), and lookups share one fetch and
///   its answer (see [`owner_inbox_cached`]).
fn announce<S: Store + 'static>(
    state: &LwsState<S>,
    grants: bool,
    document: &Value,
    own_inbox: Option<String>,
    activity: Value,
) {
    let mut inboxes: BTreeSet<String> = own_inbox.into_iter().collect();
    if grants {
        inboxes.extend(requester_inboxes(state, document));
    } else {
        let find = {
            let state = state.clone();
            async move { owner_inbox_cached(&state).await }
        };
        state.notify.deliver_found(state, activity.clone(), find);
    }
    for inbox in inboxes {
        state.notify.deliver(state, &inbox, activity.clone(), None);
    }
}

/// The inboxes of the access requests associated with `grant`: those an agent one of its policies
/// is assigned to made itself. A request that only names an assignee is not that agent's word
/// (anyone may make one, with any inbox), so its inbox is not told; public grants name no
/// requester.
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
        .filter(|r| r.author.as_ref().is_some_and(|a| grantees.contains(a)))
        .filter_map(|r| r.document.get("inbox").and_then(Value::as_str))
        .filter(|i| is_uri(i))
        .map(str::to_string)
        .collect()
}

/// How long a looked-up owner inbox (or its absence) is reused.
const OWNER_INBOX_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// [`owner_inbox`], looked up once at a time and reused for [`OWNER_INBOX_TTL`]: a burst of
/// access requests makes one fetch of the owner's identity document, not one each.
async fn owner_inbox_cached<S: Store + 'static>(state: &LwsState<S>) -> Option<String> {
    let mut cached = state.access.owner_inbox.lock().await;
    if let Some((at, inbox)) = cached.as_ref() {
        if at.elapsed() < OWNER_INBOX_TTL {
            return inbox.clone();
        }
    }
    let inbox = owner_inbox(state).await;
    *cached = Some((std::time::Instant::now(), inbox.clone()));
    inbox
}

/// The storage controller's inbox: the `inbox` (or `ldp:inbox`) its identity document names.
/// Only an http(s) owner with a JSON(-LD) document has a discoverable one; a did:key owner, a
/// Turtle-only profile or an unreachable document has none, and the notification is skipped.
async fn owner_inbox<S: Store + 'static>(state: &LwsState<S>) -> Option<String> {
    let owner = state.cfg.owner.as_deref()?;
    if !(owner.starts_with("https://") || owner.starts_with("http://")) {
        return None;
    }
    let (_, body) = sparq_lws_authn::fetch::fetch(
        &state.cfg.fetch_policy(),
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

    /// Review findings: a stored access request that could not be read at startup was skipped,
    /// and so left out of the request quota while it stayed stored; then one that could not be
    /// read stopped every start; then one held back kept its place for good, as nothing could
    /// remove it; then a grant quarantined because a read failed came back in force at the next
    /// healthy start. A read that fails stops the start (nothing is decided without the record);
    /// a record whose stored body is not one is quarantined under its IRI, at every start: never
    /// served or in force, listed and counted, until the owner's DELETE removes it and frees its
    /// place.
    #[tokio::test]
    async fn records_that_cannot_be_read_are_quarantined_until_deleted() {
        let (state, store) = test_store::state(100).await;
        let post = |state: &LwsState<test_store::FlakyStore>,
                    path: &'static str,
                    kind: &'static str,
                    who: String| {
            let state = state.clone();
            async move {
                let req = test_store::request(
                    Method::POST,
                    path,
                    &[("content-type", LWS_JSON)],
                    &access_doc(kind, "https://a/", None),
                );
                let who = Agent {
                    subject: Some(who),
                    client: None,
                };
                handle(&state, &req, &who).await
            }
        };
        let resp = post(
            &state,
            REQUESTS_PATH,
            "AccessRequest",
            "https://r.example/#me".into(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let request = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let resp = post(
            &state,
            GRANTS_PATH,
            "AccessGrant",
            "https://g.example/#me".into(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let grant = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        // A read that fails stops the start, whichever record it is.
        for iri in [&request, &grant] {
            *store.fail_read_of_any.lock().unwrap() = vec![iri.clone()];
            let mut cfg = LwsConfig::new("http://localhost:3000");
            cfg.open = true;
            assert!(LwsState::new(store.clone(), cfg).await.is_err(), "{iri}");
        }
        store.fail_read_of_any.lock().unwrap().clear();
        // Two requests and a grant whose stored bodies are not records.
        let requests = state.cfg.absolute(REQUESTS_PATH);
        let garbled = format!("{requests}garbled");
        state
            .store
            .create_in_container(
                &requests,
                &garbled,
                Bytes::from_static(b"{not json"),
                LWS_JSON,
            )
            .await
            .unwrap();
        for iri in [&request, &grant] {
            state
                .store
                .write(iri, Bytes::from_static(b"[]"), LWS_JSON)
                .await
                .unwrap();
        }
        let state = restart(&store).await;
        // A healthy start after it quarantines them again.
        let state = {
            drop(state);
            restart(&store).await
        };
        let quarantined = |grants: bool| -> Vec<String> {
            let map = state.access.map(grants).read().unwrap();
            map.values()
                .filter(|r| r.quarantined)
                .map(|r| r.id.clone())
                .collect()
        };
        assert_eq!(quarantined(false).len(), 2);
        assert_eq!(quarantined(true).len(), 1);
        assert!(
            state.access.grant_policies().is_empty(),
            "a quarantined grant grants"
        );
        let path = |iri: &str| iri.strip_prefix(&state.cfg.base_url).unwrap().to_string();
        let anyone = Agent::anonymous();
        let get = test_store::request(Method::GET, &path(&request), &[], "");
        assert_eq!(
            handle(&state, &get, &anyone).await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        // Both requests still count: the quota is full two places early.
        let mut created = 0;
        while post(
            &state,
            REQUESTS_PATH,
            "AccessRequest",
            format!("https://a{created}.example/#me"),
        )
        .await
        .status()
            == StatusCode::CREATED
        {
            created += 1;
        }
        assert_eq!(created, MAX_REQUESTS - 2);
        // The owner deletes them by IRI, which frees their places, and revokes the grant.
        for iri in [&request, &garbled, &grant] {
            let delete = test_store::request(Method::DELETE, &path(iri), &[], "");
            assert_eq!(
                handle(&state, &delete, &anyone).await.status(),
                StatusCode::NO_CONTENT,
                "{iri}"
            );
            assert!(!state.store.exists(iri).await.unwrap());
        }
        assert!(quarantined(false).is_empty() && quarantined(true).is_empty());
        let resp = post(
            &state,
            REQUESTS_PATH,
            "AccessRequest",
            "https://late.example/#me".into(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        // And none comes back.
        let state = restart(&store).await;
        assert_eq!(
            state.access.requests.read().unwrap().len(),
            MAX_REQUESTS - 1
        );
        assert!(state.access.grants.read().unwrap().is_empty());
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

    /// Only the owner may read the requests listing, so only the owner is told how a tag
    /// compares with it: anyone else's conditional request naming one is refused, matched or not.
    #[tokio::test]
    async fn a_requester_is_not_told_how_the_listing_compares() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let as_ = |who: &str| Agent {
            subject: Some(who.into()),
            client: None,
        };
        let (owner, someone) = (
            as_("https://owner.example/#me"),
            as_("https://r.example/#me"),
        );
        let tag = handle(
            &state,
            &test_store::request(Method::GET, REQUESTS_PATH, &[], ""),
            &owner,
        )
        .await
        .headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_string();
        let post = |who: Agent, h: (&'static str, String)| {
            let state = state.clone();
            async move {
                let req = test_store::request(
                    Method::POST,
                    REQUESTS_PATH,
                    &[("content-type", LWS_JSON), (h.0, &h.1)],
                    &access_doc("AccessRequest", "https://a/", None),
                );
                handle(&state, &req, &who).await.status()
            }
        };
        for h in [
            ("if-match", tag.clone()),
            ("if-match", "\"other\"".to_string()),
            ("if-none-match", tag.clone()),
        ] {
            assert_eq!(
                post(someone.clone(), h.clone()).await,
                StatusCode::FORBIDDEN,
                "{h:?}"
            );
        }
        assert_eq!(
            post(owner.clone(), ("if-match", "\"other\"".into())).await,
            StatusCode::PRECONDITION_FAILED
        );
        // `*` asks only whether the container exists.
        assert_eq!(
            post(someone, ("if-match", "*".into())).await,
            StatusCode::CREATED
        );
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

    /// A revocation takes the grant out of force once its removal is confirmed, and answers only
    /// what is confirmed: review findings, the grant stayed in force while its stored record was
    /// gone, and then left force while a restart would put it back. A full store still revokes.
    #[tokio::test]
    async fn a_revocation_that_does_not_land_takes_the_grant_out_of_force() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let grant = || {
            let (state, post) = (state.clone(), post.clone());
            async move {
                let resp = handle(&state, &post, &Agent::anonymous()).await;
                assert_eq!(resp.status(), StatusCode::CREATED);
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
            }
        };
        let (location, delete) = grant().await;
        *store.refuse_write_of.lock().unwrap() = Some(location.clone());
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.access.grant_policies().is_empty());
        assert!(!state.store.exists(&location).await.unwrap());
        *store.refuse_write_of.lock().unwrap() = None;
        let (location, delete) = grant().await;
        // The removal fails and, read again, the record is there as it was: still in force, here
        // as at the next start, and the answer says so.
        store.fail_delete.store(true, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("still stored"));
        assert!(!state.access.grant_policies().is_empty());
        assert!(state.store.exists(&location).await.unwrap());
        let loaded = AccessStore::load(&state.store, &state.cfg).await.unwrap();
        assert!(!loaded.grant_policies().is_empty());
        // The store recovers: the request, retried, revokes it.
        store.fail_delete.store(false, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!state.store.exists(&location).await.unwrap());
        assert!(state.access.grant_policies().is_empty());
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
        // Out of force from the moment its removal starts.
        assert!(state.access.grant_policies().is_empty());
        gate.add_permits(1);
        for _ in 0..200 {
            if state.access.grant_policies().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(state.access.grant_policies().is_empty());
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
        // Review finding: a request Alice made naming Bob, with an inbox of her choosing, was
        // told of Bob's grant. Only the requests Bob made himself are.
        assert!(requester_inboxes(&state, &grant).is_empty());
        let bob = Agent {
            subject: Some("https://bob.example/#me".into()),
            client: None,
        };
        let req = test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[],
            &access_doc(
                "AccessRequest",
                "https://bob.example/#me",
                Some("https://bob.example/own-inbox/"),
            ),
        );
        assert_eq!(
            handle(&state, &req, &bob).await.status(),
            StatusCode::CREATED
        );
        assert_eq!(
            requester_inboxes(&state, &grant)
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["https://bob.example/own-inbox/".to_string()]
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

    /// Review finding: a grant or request was announced by the handler after the store
    /// committed it, so a client that went away (or a request timeout) in between left a stored,
    /// in-force record nobody was told about. It is announced in the step that registers it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stored_records_are_announced_when_the_client_is_gone() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;
        let delivered = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = {
            let delivered = delivered.clone();
            axum::Router::new().route(
                "/inbox",
                axum::routing::post(move || {
                    let delivered = delivered.clone();
                    async move {
                        delivered.fetch_add(1, Ordering::SeqCst);
                        StatusCode::NO_CONTENT
                    }
                }),
            )
        };
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let (state, store) = test_store::state(100).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_create.lock().unwrap() = Some(gate.clone());
        let inbox = format!("http://{addr}/inbox");
        let req = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[("content-type", LWS_JSON)],
            &access_doc("AccessGrant", "https://a/", Some(&inbox)),
        );
        let client = {
            let state = state.clone();
            tokio::spawn(async move { handle(&state, &req, &Agent::anonymous()).await.status() })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        client.abort();
        assert!(client.await.is_err(), "the client went away mid-create");
        gate.add_permits(1);
        for _ in 0..200 {
            if delivered.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state.access.grant_policies().len(), 1);
        assert_eq!(delivered.load(Ordering::SeqCst), 1);
    }

    /// Review finding: every access request spawned its own fetch of the owner's identity
    /// document, outside the bounded delivery queue. The lookup now takes a place in the queue
    /// first, and lookups share one fetch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_lookups_are_bounded_and_shared() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;
        let (fetched, delivered) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = {
            let (fetched, delivered) = (fetched.clone(), delivered.clone());
            axum::Router::new()
                .route(
                    "/profile",
                    axum::routing::get(move || {
                        let fetched = fetched.clone();
                        async move {
                            fetched.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            (
                                [(header::CONTENT_TYPE, "application/json")],
                                json!({"inbox": format!("http://{addr}/inbox")}).to_string(),
                            )
                        }
                    }),
                )
                .route(
                    "/inbox",
                    axum::routing::post(move || {
                        let delivered = delivered.clone();
                        async move {
                            delivered.fetch_add(1, Ordering::SeqCst);
                            StatusCode::NO_CONTENT
                        }
                    }),
                )
        };
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.owner = Some(format!("http://{addr}/profile#me"));
        cfg.delivery = super::super::notify::DeliveryLimits {
            queue: 3,
            workers: 4,
            per_inbox: 4,
        };
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        for _ in 0..10 {
            let req = test_store::request(
                Method::POST,
                REQUESTS_PATH,
                &[],
                &access_doc("AccessRequest", "https://bob.example/#me", None),
            );
            let resp = handle(&state, &req, &Agent::anonymous()).await;
            assert_eq!(resp.status(), StatusCode::CREATED);
        }
        // Three admitted (the queue's limit), the rest dropped and counted.
        assert_eq!(state.notify.dropped(), 7);
        for _ in 0..200 {
            if delivered.load(Ordering::SeqCst) >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(delivered.load(Ordering::SeqCst), 3);
        // One fetch served them all.
        assert_eq!(fetched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_owner_without_an_identity_document_has_no_inbox() {
        let (state, _) = test_store::state(100).await;
        assert_eq!(owner_inbox(&state).await, None);
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

    /// Review finding (#6810): stored access requests can exceed their quota (stored before it
    /// was lowered, or written to the store directly). The start loads them all, removes none,
    /// reports what is past the quota, and admits no new request from an author past its share.
    #[tokio::test]
    async fn requests_stored_past_their_quota_are_kept_reported_and_admit_no_more() {
        let (state, store) = super::super::test_store::state(100).await;
        let container = state.cfg.absolute(REQUESTS_PATH);
        let asker = "https://asker.example/#me";
        let document = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": ["AccessRequest"],
            "storage": state.cfg.storage(),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a/"}],
        });
        for i in 0..=MAX_REQUESTS_PER_AUTHOR {
            let stored = json!({"document": document, "author": asker});
            store
                .create_in_container(
                    &container,
                    &format!("{container}r{i:04}"),
                    Bytes::from(stored.to_string()),
                    LWS_JSON,
                )
                .await
                .unwrap();
        }
        let state = LwsState::new(store.clone(), state.cfg.clone())
            .await
            .unwrap();
        assert_eq!(
            state.access.requests.read().unwrap().len(),
            MAX_REQUESTS_PER_AUTHOR + 1,
            "all loaded"
        );
        let past = state.access.requests_past_quota().unwrap();
        assert!(
            past.contains(&format!(
                "1 author(s) holding more than {MAX_REQUESTS_PER_AUTHOR} each (at most {})",
                MAX_REQUESTS_PER_AUTHOR + 1
            )),
            "{past}"
        );
        let post = super::super::test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[("content-type", LWS_JSON)],
            &document.to_string(),
        );
        let agent = |who: &str| Agent {
            subject: Some(who.into()),
            ..Agent::anonymous()
        };
        let r = handle(&state, &post, &agent(asker)).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        let r = handle(&state, &post, &agent("https://other.example/#me")).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        // Withdrawing two brings the asker under its share.
        for i in 0..2 {
            let delete = super::super::test_store::request(
                Method::DELETE,
                &format!("{REQUESTS_PATH}r{i:04}"),
                &[],
                "",
            );
            let r = handle(&state, &delete, &agent(asker)).await;
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
        }
        assert!(state.access.requests_past_quota().is_none());
        let r = handle(&state, &post, &agent(asker)).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert!(store.exists(&format!("{container}r0002")).await.unwrap());
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

    /// The server started again over `store`, as after a process stop.
    async fn restart(store: &test_store::FlakyStore) -> LwsState<test_store::FlakyStore> {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.page_size = 100;
        LwsState::new(store.clone(), cfg).await.expect("starts")
    }

    /// Wait until `done`, or fail.
    async fn eventually(what: &str, done: impl Fn() -> bool) {
        for _ in 0..250 {
            if done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("{what}");
    }

    /// Grant `https://a/` access: the grant's IRI.
    async fn granted(state: &LwsState<test_store::FlakyStore>) -> String {
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let resp = handle(state, &post, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    /// Review findings: a revocation that could not tell whether it removed the record took the
    /// grant out of force though a restart would put it back, or answered "still in force" though
    /// the record was gone. The answer says only what is confirmed: a removal that reports a
    /// failure but is confirmed by reading the store again revokes; one that cannot be checked is
    /// answered as not known, with the grant left in force as the store last had it; and a retry
    /// once the store answers settles it.
    #[tokio::test]
    async fn a_revocation_answers_only_what_is_confirmed() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let answer = |state: LwsState<test_store::FlakyStore>, path: String| async move {
            let delete = test_store::request(Method::DELETE, &path, &[], "");
            let resp = handle(&state, &delete, &Agent::anonymous()).await;
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, String::from_utf8_lossy(&body).to_string())
        };
        // The removal lands but reports a failure; read again, the record is gone: revoked.
        let location = granted(&state).await;
        let path = location
            .strip_prefix(&state.cfg.base_url)
            .unwrap()
            .to_string();
        *store.fail_after_delete_of.lock().unwrap() = Some(location.clone());
        store.fail_exists.store(true, Ordering::SeqCst);
        let (status, _) = answer(state.clone(), path).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(state.access.grant_policies().is_empty());
        store.fail_exists.store(false, Ordering::SeqCst);
        *store.fail_after_delete_of.lock().unwrap() = None;
        // The removal lands but reports a failure, and the store cannot be read again: not known.
        let location = granted(&state).await;
        let path = location
            .strip_prefix(&state.cfg.base_url)
            .unwrap()
            .to_string();
        *store.fail_after_delete_of.lock().unwrap() = Some(location.clone());
        store.fail_exists.store(true, Ordering::SeqCst);
        store.fail_list.store(true, Ordering::SeqCst);
        let (status, body) = answer(state.clone(), path.clone()).await;
        assert!(status.is_server_error());
        assert!(body.contains("not known"), "{body}");
        assert!(!body.contains("still stored"), "{body}");
        store.fail_exists.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        *store.fail_after_delete_of.lock().unwrap() = None;
        // A retry, the store answering, settles it: the record is gone, so the grant goes too,
        // and a restart agrees.
        let (status, _) = answer(state.clone(), path).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(state.access.grant_policies().is_empty());
        let state = restart(&store).await;
        assert!(state.access.grant_policies().is_empty());
        // The removal does not land and cannot be checked: not known, in force as before, here
        // as at the next start.
        let location = granted(&state).await;
        let path = location
            .strip_prefix(&state.cfg.base_url)
            .unwrap()
            .to_string();
        store.fail_delete.store(true, Ordering::SeqCst);
        store.fail_list.store(true, Ordering::SeqCst);
        let (status, body) = answer(state.clone(), path).await;
        assert!(status.is_server_error());
        assert!(body.contains("not known"), "{body}");
        assert!(!state.access.grant_policies().is_empty());
        store.fail_delete.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        let state = restart(&store).await;
        assert!(!state.access.grant_policies().is_empty());
    }

    /// Review finding: a document nested to the parser's limit was stored one level deeper inside
    /// its record, which then did not parse, and every later start failed. It is refused.
    #[tokio::test]
    async fn an_access_document_too_deep_to_store_is_refused() {
        let (state, store) = test_store::state(100).await;
        for (path, kind) in [
            (GRANTS_PATH, "AccessGrant"),
            (REQUESTS_PATH, "AccessRequest"),
        ] {
            let base: Value = serde_json::from_str(&access_doc(kind, "https://a/", None)).unwrap();
            let nested = |d: usize| format!("{}1{}", "[".repeat(d), "]".repeat(d));
            let with = |d: usize| {
                let mut doc = base.clone();
                doc["x"] = serde_json::from_str(&nested(d)).unwrap();
                doc
            };
            // The deepest document the parser accepts.
            let deepest = (1..200)
                .take_while(|&d| {
                    serde_json::from_str::<Value>(&nested(d)).is_ok()
                        && serde_json::from_str::<Value>(&with(d).to_string()).is_ok()
                })
                .last()
                .unwrap();
            let doc = with(deepest);
            let post = test_store::request(Method::POST, path, &[], &doc.to_string());
            let resp = handle(&state, &post, &Agent::anonymous()).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{kind}");
        }
        // Nothing was stored, and the server starts again.
        restart(&store).await;
    }

    /// Review findings: a withdrawal that could not remove the request left it hidden, and the
    /// next start loaded it again; then it was answered as withdrawn while a restart would bring
    /// it back. A withdrawal whose removal fails leaves the request as the store has it, here as
    /// at the next start, until a withdrawal lands.
    #[tokio::test]
    async fn a_withdrawal_that_fails_leaves_the_request_as_stored() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let post = test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[],
            &access_doc("AccessRequest", "https://a/", None),
        );
        let resp = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let location = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let path = location
            .strip_prefix(&state.cfg.base_url)
            .unwrap()
            .to_string();
        store.fail_delete.store(true, Ordering::SeqCst);
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(state.store.exists(&location).await.unwrap());
        // Still stored as it was: still there, here as at the next start, until a withdrawal
        // lands.
        assert_eq!(state.access.requests.read().unwrap().len(), 1);
        let state = restart(&store).await;
        assert_eq!(state.access.requests.read().unwrap().len(), 1);
        store.fail_delete.store(false, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!state.store.exists(&location).await.unwrap());
        let state = restart(&store).await;
        assert!(state.access.requests.read().unwrap().is_empty());
    }

    /// Review findings: a create was a sequence of writes (an intent, the record, then its type),
    /// and each gap between them could leave a grant stored but refused, or in force but not
    /// stored. A create is now one write, and memory follows the store: a write that reports a
    /// failure is checked against the listing, and the answer says only what is confirmed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_create_answers_only_what_is_confirmed() {
        use std::sync::atomic::Ordering;
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
        // The write fails before it lands: refused, nothing stored, nothing in force.
        *store.fail_step.lock().unwrap() = Some(0);
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(stored().await, 0);
        assert!(state.access.grant_policies().is_empty());
        // The write lands and its reply is lost: the listing confirms it, so it is created.
        store.fail_after_create.store(true, Ordering::SeqCst);
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(stored().await, 1);
        assert_eq!(state.access.grant_policies().len(), 1);
        // The listing cannot be read either: not known, and not in force until the listing
        // shows it, then in force without another request.
        store.fail_list.store(true, Ordering::SeqCst);
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = String::from_utf8(
            axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("not known"), "{body}");
        assert_eq!(state.access.grant_policies().len(), 1);
        store.fail_after_create.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        eventually("the stored grant is put in force", || {
            state.access.grant_policies().len() == 2
        })
        .await;
        assert_eq!(stored().await, 2);
        // A start loads what the listing holds, and nothing else.
        let state = restart(&store).await;
        assert_eq!(state.access.grant_policies().len(), 2);
    }

    /// Review finding: a create whose outcome was settled later registered it without the
    /// container's lock, so the listing could change under a conditional create holding the
    /// container. It is registered only once it holds the container's lock, shared.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_settled_create_registers_under_the_container_lock() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(GRANTS_PATH);
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        store.fail_after_create.store(true, Ordering::SeqCst);
        store.fail_list.store(true, Ordering::SeqCst);
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let held = state.locks.lock(&container).await.expect("held");
        store.fail_after_create.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(state.access.grant_policies().is_empty());
        drop(held);
        eventually("the stored grant is put in force", || {
            state.access.grant_policies().len() == 1
        })
        .await;
    }

    /// Review finding: a request whose create was not known to be stored gave its place back,
    /// and one being withdrawn left the count while it was still stored, so concurrent creates
    /// could hold more than their share. A place is held until the store confirms it is free.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_place_is_freed_only_once_the_store_confirms_it() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = || {
            test_store::request(
                Method::POST,
                REQUESTS_PATH,
                &[],
                &access_doc("AccessRequest", "https://a/", None),
            )
        };
        // Every create's outcome is unknown: each keeps its place, so the share still holds.
        store.fail_after_create.store(true, Ordering::SeqCst);
        store.fail_list.store(true, Ordering::SeqCst);
        for _ in 0..MAX_REQUESTS_PER_AUTHOR {
            let r = handle(&state, &post(), &asker).await;
            assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        store.fail_after_create.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        eventually("the stored requests are registered", || {
            state.access.requests.read().unwrap().len() == MAX_REQUESTS_PER_AUTHOR
        })
        .await;
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // A withdrawal still pending keeps its place.
        let id = state
            .access
            .requests
            .read()
            .unwrap()
            .keys()
            .next()
            .cloned()
            .unwrap();
        let iri = format!("{}{id}", state.cfg.absolute(REQUESTS_PATH));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        let withdrawal = {
            let state = state.clone();
            let asker = asker.clone();
            let delete = test_store::request(
                Method::DELETE,
                iri.strip_prefix(&state.cfg.base_url).unwrap(),
                &[],
                "",
            );
            tokio::spawn(async move { handle(&state, &delete, &asker).await.status() })
        };
        eventually("the withdrawal is pending", || {
            store.hold_next_delete_of.lock().unwrap().is_none()
        })
        .await;
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        gate.add_permits(1);
        assert_eq!(withdrawal.await.unwrap(), StatusCode::NO_CONTENT);
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// Issue #6836: a create whose write failed unsettled (an HTTP update that timed out, still
    /// running on the store) was answered `500` with its place freed when the listing did not
    /// show it yet; the write then committed, leaving a record stored, not in force, and outside
    /// its author's share until the next start. It keeps its place, unregistered, until its fence
    /// lands; then it is put in force, as the write landed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_create_that_may_still_commit_keeps_its_place_until_it_is_settled() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(REQUESTS_PATH);
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = || {
            test_store::request(
                Method::POST,
                REQUESTS_PATH,
                &[],
                &access_doc("AccessRequest", "https://a/", None),
            )
        };
        let stored = || async { state.store.list_children(&container).await.unwrap().len() };
        let registered = || state.access.requests.read().unwrap().len();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
        store.fail_fence.store(true, Ordering::SeqCst);
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("not known"), "{body}");
        // Not listed, and not in force, but not taken as absent: its place is held.
        assert_eq!(stored().await, 0);
        assert_eq!(registered(), 0);
        for _ in 1..MAX_REQUESTS_PER_AUTHOR {
            let r = handle(&state, &post(), &asker).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // The write lands while the store cannot be reached to fence it: still not in force,
        // and still holding its place.
        gate.add_permits(1);
        for _ in 0..250 {
            if stored().await == MAX_REQUESTS_PER_AUTHOR {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(stored().await, MAX_REQUESTS_PER_AUTHOR);
        assert_eq!(registered(), MAX_REQUESTS_PER_AUTHOR - 1);
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // Once the store answers, the fence settles it: stored, so in force.
        store.fail_fence.store(false, Ordering::SeqCst);
        eventually("the stored request is put in force", || {
            registered() == MAX_REQUESTS_PER_AUTHOR
        })
        .await;
        assert_eq!(
            state.access.request_quota.in_flight.lock().unwrap().total,
            0
        );
        let r = handle(&state, &post(), &asker).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // A start loads the same.
        let state = restart(&store).await;
        assert_eq!(
            state.access.requests.read().unwrap().len(),
            MAX_REQUESTS_PER_AUTHOR
        );
    }

    /// Issue #6836: a create whose fence lands before its write does can no longer commit: it is
    /// settled as not stored, its place is freed, and the write, when it arrives, changes nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_create_fenced_before_it_commits_is_settled_as_not_stored() {
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(REQUESTS_PATH);
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[],
            &access_doc("AccessRequest", "https://a/", None),
        );
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
        let r = handle(&state, &post, &asker).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        eventually("the place is freed", || {
            state.access.request_quota.in_flight.lock().unwrap().total == 0
        })
        .await;
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(state
            .store
            .list_children(&container)
            .await
            .unwrap()
            .is_empty());
        assert!(state.access.requests.read().unwrap().is_empty());
        let state = restart(&store).await;
        assert!(state.access.requests.read().unwrap().is_empty());
    }

    /// Issue #6836, the mirror: a revocation whose delete failed unsettled could be answered
    /// "still stored", the grant put back in force, and the delete then commit, leaving a grant
    /// in force that is not stored. It stays out of force until the delete is fenced, and is then
    /// removed as the delete landed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_revocation_that_may_still_commit_is_settled_once_fenced() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(GRANTS_PATH);
        let iri = granted(&state).await;
        assert_eq!(state.access.grant_policies().len(), 1);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let delete = test_store::request(
            Method::DELETE,
            iri.strip_prefix(&state.cfg.base_url).unwrap(),
            &[],
            "",
        );
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("not known"), "{body}");
        // Still listed, but not taken as kept: out of force until settled.
        assert_eq!(
            state.store.list_children(&container).await.unwrap().len(),
            1
        );
        assert!(state.access.grant_policies().is_empty());
        gate.add_permits(1);
        for _ in 0..250 {
            if state
                .store
                .list_children(&container)
                .await
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(state
            .store
            .list_children(&container)
            .await
            .unwrap()
            .is_empty());
        store.fail_fence.store(false, Ordering::SeqCst);
        eventually("the revoked grant is removed", || {
            state.access.grants.read().unwrap().is_empty()
        })
        .await;
        let state = restart(&store).await;
        assert!(state.access.grants.read().unwrap().is_empty());
    }

    /// Issue #6836, the mirror: a revocation fenced before its delete lands is settled as not
    /// removed: the grant is put back in force, and the delete, when it arrives, changes nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_revocation_fenced_before_it_commits_puts_the_grant_back() {
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(GRANTS_PATH);
        let iri = granted(&state).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        let delete = test_store::request(
            Method::DELETE,
            iri.strip_prefix(&state.cfg.base_url).unwrap(),
            &[],
            "",
        );
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        eventually("the grant is put back in force", || {
            state.access.grant_policies().len() == 1
        })
        .await;
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            state.store.list_children(&container).await.unwrap().len(),
            1
        );
        assert_eq!(state.access.grant_policies().len(), 1);
        // And it can be revoked.
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
    }

    /// Codex review of #6836: settlement tasks had no bound, so creates (grants have no quota)
    /// against a store that keeps failing piled up tasks, records and locks. A change holds a
    /// place among those being settled from its request until it is settled; past the bound, a
    /// new one is refused with `503`, and once the store answers again, the places come back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn record_changes_being_settled_are_bounded() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let post = || {
            test_store::request(
                Method::POST,
                GRANTS_PATH,
                &[],
                &access_doc("AccessGrant", "https://a/", None),
            )
        };
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
        store.fail_fence.store(true, Ordering::SeqCst);
        for _ in 0..super::super::records::MAX_SETTLING {
            let r = handle(&state, &post(), &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        assert_eq!(
            state.settlements.pending(),
            super::super::records::MAX_SETTLING
        );
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        // The store answers again: every one is fenced (before its write, still held), so none
        // is stored, and the places come back.
        *store.unsettle_creates.lock().unwrap() = None;
        store.fail_fence.store(false, Ordering::SeqCst);
        eventually("every change is settled", || {
            state.settlements.pending() == 0
        })
        .await;
        gate.add_permits(super::super::records::MAX_SETTLING);
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(state.access.grant_policies().len(), 1);
    }

    /// Codex review of #6836: a settlement task held the state, so a server dropped on a
    /// runtime that goes on kept it, and the task, running. The tasks hold the state only
    /// weakly: dropping it ends them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_state_ends_its_settlements() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        *store.unsettle_next_create.lock().unwrap() =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
        store.fail_fence.store(true, Ordering::SeqCst);
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        eventually("the settlement is retrying", || {
            store.fences_tried.load(Ordering::SeqCst) >= 2
        })
        .await;
        let weak = state.downgrade();
        drop(state);
        eventually("the state is dropped", || weak.upgrade().is_none()).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let tried = store.fences_tried.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), tried);
    }

    /// Codex review of #6836: a settlement step held the state across a store call, so one that
    /// stalled kept the state, its tasks, the record's lock and its quota place alive after every
    /// other handle was dropped. A settlement waits on the store and locks through their own
    /// handles: dropping the state ends it even while its fence never answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_state_ends_a_settlement_whose_store_call_stalls() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let asker = Agent {
            subject: Some("https://asker.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = test_store::request(
            Method::POST,
            REQUESTS_PATH,
            &[],
            &access_doc("AccessRequest", "https://a/", None),
        );
        *store.unsettle_next_create.lock().unwrap() =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
        store.fail_fence.store(true, Ordering::SeqCst);
        let r = handle(&state, &post, &asker).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let child = store.last_unsettled.lock().unwrap()[1].clone();
        // The settlement's next fence stalls for good.
        store.hang_fence.store(true, Ordering::SeqCst);
        let tried = store.fences_tried.load(Ordering::SeqCst);
        eventually("the settlement's fence is stalled", || {
            store.fences_tried.load(Ordering::SeqCst) > tried
        })
        .await;
        let (locks, quota) = (
            state.locks.clone(),
            state.access.request_quota.in_flight.clone(),
        );
        assert_eq!(quota.lock().unwrap().total, 1);
        assert!(locks.try_lock(&child).is_none(), "the settlement holds it");
        let weak = state.downgrade();
        drop(state);
        eventually("the state is dropped", || weak.upgrade().is_none()).await;
        eventually("the record's lock is released", || {
            locks.try_lock(&child).is_some()
        })
        .await;
        eventually("the quota place is released", || {
            quota.lock().unwrap().total == 0
        })
        .await;
    }

    /// Review finding: a create waited for its container inside a task of its own, so requests
    /// queued behind a container held while a change was put back. A create waits a bounded time
    /// in the request and then answers 503.
    #[tokio::test]
    async fn a_create_into_a_busy_container_answers_promptly() {
        let (state, _store) = test_store::state(100).await;
        let container = state.cfg.absolute(GRANTS_PATH);
        let held = state.locks.lock(&container).await.expect("held");
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let started = std::time::Instant::now();
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(state.access.grant_policies().is_empty());
        drop(held);
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// Review finding: a revocation or a conditional create waited for its locks inside a task
    /// of its own, so while one removal stalled in the store, repeated requests queued detached
    /// tasks that kept their admission past the client's timeout. The locks are taken in the
    /// request, waiting a bounded time, before any task starts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stalled_removal_answers_later_requests_promptly() {
        use std::sync::Arc;
        let (state, store) = test_store::state(100).await;
        let iri = granted(&state).await;
        let path = iri.strip_prefix(&state.cfg.base_url).unwrap().to_string();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        let first = {
            let state = state.clone();
            let delete = test_store::request(Method::DELETE, &path, &[], "");
            tokio::spawn(async move { handle(&state, &delete, &Agent::anonymous()).await.status() })
        };
        eventually("the removal is pending", || {
            store.hold_next_delete_of.lock().unwrap().is_none()
        })
        .await;
        let started = std::time::Instant::now();
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        // A conditional create needs the container exclusively, held shared by the removal.
        let post = test_store::request(
            Method::POST,
            GRANTS_PATH,
            &[("if-match", "*")],
            &access_doc("AccessGrant", "https://a/", None),
        );
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        gate.add_permits(1);
        assert_eq!(first.await.unwrap(), StatusCode::NO_CONTENT);
        assert!(state.access.grant_policies().is_empty());
    }

    /// What a request answered, whether the grant was in force in the process that answered it,
    /// and whether it is after that process stopped (at the store step the request's store died
    /// at) and the server started again.
    struct Crashed {
        status: StatusCode,
        body: String,
        in_force_before: bool,
        in_force_after: bool,
        grants_stored_after: usize,
    }

    /// Create a grant (or, with `delete`, revoke one) on a fresh server whose store dies from its
    /// `n`th step on (a process stop there: the runtime it ran on is shut down, with every task
    /// it started), then start the server again over the store, well again. `None` once the
    /// request takes fewer steps than `n`.
    fn crash_at(n: usize, delete: bool) -> Option<Crashed> {
        use std::sync::atomic::Ordering;
        let runtime = || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
        };
        let first = runtime();
        let (store, fired, status, body, in_force_before) = first.block_on(async {
            let (state, store) = test_store::state(100).await;
            let req = match delete {
                true => {
                    let location = granted(&state).await;
                    let path = location.strip_prefix(&state.cfg.base_url).unwrap();
                    test_store::request(Method::DELETE, path, &[], "")
                }
                false => test_store::request(
                    Method::POST,
                    GRANTS_PATH,
                    &[],
                    &access_doc("AccessGrant", "https://a/", None),
                ),
            };
            *store.fail_step.lock().unwrap() = Some(n);
            store.dead_after.store(true, Ordering::SeqCst);
            let resp = handle(&state, &req, &Agent::anonymous()).await;
            let fired = store.fail_step.lock().unwrap().take().is_none();
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&body).to_string();
            let in_force = !state.access.grant_policies().is_empty();
            (store, fired, status, body, in_force)
        });
        first.shutdown_timeout(std::time::Duration::from_secs(5));
        if !fired {
            return None;
        }
        store.dead_after.store(false, Ordering::SeqCst);
        let second = runtime();
        let (in_force_after, grants_stored_after) = second.block_on(async {
            let state = restart(&store).await;
            let container = state.cfg.absolute(GRANTS_PATH);
            let stored = state.store.list_children(&container).await.unwrap().len();
            (!state.access.grant_policies().is_empty(), stored)
        });
        Some(Crashed {
            status,
            body,
            in_force_before,
            in_force_after,
            grants_stored_after,
        })
    }

    /// Review finding (twice): a grant's create or revocation cut short could leave a grant in
    /// force after a restart that the request had refused or revoked. For every store step a
    /// create or a revocation takes, a process stop there leaves the grant in force after the
    /// restart exactly when it was in force when the process stopped and the client was told
    /// so: a create answered `201`, a revocation answered that the grant is still in force.
    #[test]
    fn a_stop_at_any_step_never_leaves_a_refused_or_revoked_grant_in_force() {
        for delete in [false, true] {
            let what = if delete { "revocation" } else { "create" };
            let mut n = 0;
            while let Some(c) = crash_at(n, delete) {
                // A revocation not known to have happened leaves the grant as it was.
                let kept = if delete {
                    c.status.is_server_error()
                        && (c.body.contains("still stored") || c.body.contains("not known"))
                } else {
                    c.status == StatusCode::CREATED
                };
                assert_eq!(
                    c.in_force_before, kept,
                    "{what} stopped at step {n}: {} {}",
                    c.status, c.body
                );
                assert_eq!(
                    c.in_force_after, kept,
                    "{what} stopped at step {n}, after the restart: {} {}",
                    c.status, c.body
                );
                if !kept {
                    assert_eq!(c.grants_stored_after, 0, "{what} at step {n} left a record");
                }
                n += 1;
            }
            // Each is one store step.
            assert_eq!(n, 1, "{what} took {n} steps");
        }
    }

    /// Review finding: a start that recovered a change it could not yet put back, then failed
    /// loading the access requests, left the task putting it back running; a later start's
    /// writes could then be overwritten by it. Nothing is put back in the background until
    /// every step of the start that can fail has succeeded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_start_that_fails_loading_requests_leaves_nothing_running() {
        let (state, store) = test_store::state(100).await;
        let turtle = ("content-type", "text/turtle");
        let post = test_store::request(
            Method::POST,
            "/",
            &[turtle, ("slug", "d")],
            "<> a <urn:A> .",
        );
        let resp = super::super::route(&state, post).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let d = state.cfg.absolute("/d");
        // A change cut short (its process stopped) that cannot be put back yet.
        let mut journal = state.journal();
        journal.stage(&super::super::meta_key(&d)).await.unwrap();
        journal.stage(&d).await.unwrap();
        journal
            .write(&d, Bytes::from_static(b"<> a <urn:B> ."), "text/turtle")
            .await
            .unwrap();
        std::mem::forget(journal);
        *store.fail_restore_of.lock().unwrap() = Some(d.clone());
        // And the access requests container cannot be looked up.
        let requests = state.cfg.absolute(REQUESTS_PATH);
        *store.fail_exists_of.lock().unwrap() = Some(requests.clone());
        drop(state);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        assert!(LwsState::new(store.clone(), cfg).await.is_err());
        // Nothing of the failed start keeps trying to put `/d` back.
        let tried = store.restores.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(
            store.restores.load(std::sync::atomic::Ordering::SeqCst),
            tried,
            "still putting it back"
        );
        // Repaired; the next start puts `/d` back, and a write after it stands, however long a
        // task of the failed start might wait.
        *store.fail_exists_of.lock().unwrap() = None;
        *store.fail_restore_of.lock().unwrap() = None;
        let state = restart(&store).await;
        let put = test_store::request(Method::PUT, "/d", &[turtle], "<> a <urn:C> .");
        let resp = super::super::route(&state, put).await;
        assert!(resp.status().is_success(), "{}", resp.status());
        let after = state.store.read(&d).await.unwrap().body;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(state.store.read(&d).await.unwrap().body, after);
    }

    /// Review finding: a start read a grant, then a change put back in the background removed it
    /// and showed it again, and the start put the grant it had read in force. A grant still being
    /// settled is not read, and nothing is put back until the grants are loaded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_start_never_loads_a_grant_still_being_removed() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let location = granted(&state).await;
        // A revocation cut short: its intent stored, the record not yet removed.
        let remove = super::super::Undo::Remove {
            iri: location.clone(),
            parent: state.cfg.absolute(GRANTS_PATH),
        };
        let record = super::super::intents::mint(&state.cfg.storage());
        super::super::intents::store(&state, &record, false, &[&remove], &[])
            .await
            .unwrap();
        drop(state);
        // The start cannot remove it at first; reading it (were it read) answers only once the
        // removal could have landed in the background, and shown it again.
        store.fail_delete.store(true, Ordering::SeqCst);
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_read_reply_of.lock().unwrap() = Some((location.clone(), gate.clone()));
        let starting = {
            let store = store.clone();
            tokio::spawn(async move { restart(&store).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        store.fail_delete.store(false, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        gate.add_permits(1);
        let state = starting.await.unwrap();
        assert!(state.access.grant_policies().is_empty(), "in force");
        eventually("removed", || state.visible(&location)).await;
        assert!(!state.store.exists(&location).await.unwrap());
        assert!(state.access.grant_policies().is_empty());
    }
}

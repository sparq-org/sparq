//! Notifications (LWS 1.0 core section 10) and the webhook subscription type
//! (lws10-notifications-webhook).
//!
//! The NotificationService lives at [`SUBSCRIPTIONS_PATH`](super::SUBSCRIPTIONS_PATH) and offers
//! one subscription type, `WebhookSubscription`. An authenticated agent subscribes with an
//! `application/lws+json` request naming a `topic` (an array of resource URIs, each of which it
//! must be able to read now) and an `inbox`; a topic covers itself and, for a container,
//! everything beneath it. Each Create, Update and Delete inside a topic is then POSTed to the
//! inbox as an `application/lws+json` Notification, provided the subscriber may read the
//! resource when the event occurs (section 10.3.3).
//!
//! Deliveries leave the request path (`tokio::spawn`), are signed per RFC 9421 with the key the
//! storage description publishes (covering `@method`, `@scheme`, `@authority`, `@path`,
//! `content-type` and an RFC 9530 `content-digest`), are tried again on a 5xx or an unreachable
//! inbox, and a subscription whose inbox answers 410 Gone, or fails repeatedly, is deactivated
//! (removed). Unless `allow_insecure_fetch` is set, inboxes must be `https:` and resolve to public
//! addresses only (checked when subscribing and again by the delivery client's resolver), and
//! deliveries never follow redirects or go through a proxy.
//!
//! Subscriptions are stored through the [`Store`] as members of the service container, so they
//! survive a restart on a durable backend, and kept in memory for event matching. Their
//! subscriber and the storage owner may read and cancel them; to anyone else they do not exist.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::access::{format_rfc3339, parse_rfc3339, Action};
use super::{
    add_link, jose, json_is_uri, method_not_allowed, problem, set, Agent, LwsConfig, LwsRequest,
    LwsState, AS_CONTEXT, LWS_CONTEXT, LWS_JSON, LWS_NS, SUBSCRIPTIONS_PATH,
};
use crate::store::Store;

/// The one subscription type this NotificationService offers.
pub const WEBHOOK: &str = "WebhookSubscription";

/// Attempts at one delivery: a 5xx or an unreachable inbox is tried again.
const DELIVERY_ATTEMPTS: u32 = 3;
/// Pause between attempts at one delivery.
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// Consecutive failed deliveries before a subscription is deactivated.
const MAX_DELIVERY_FAILURES: u32 = 5;

/// The components every delivery signature covers (lws10-notifications-webhook, "Signature
/// Requirements").
const SIGNATURE_COMPONENTS: [&str; 6] = [
    "@method",
    "@scheme",
    "@authority",
    "@path",
    "content-type",
    "content-digest",
];

/// A change to a storage resource, announced to the subscriptions whose topics cover it.
#[derive(Debug, Clone)]
pub struct Event {
    /// `Create`, `Update` or `Delete`.
    pub kind: &'static str,
    /// The resource that changed.
    pub uri: String,
    pub is_container: bool,
    /// `("target", container)` for a Create, `("origin", container)` for a Delete.
    pub relation: Option<(&'static str, String)>,
}

/// One webhook subscription, as stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subscription {
    #[serde(skip)]
    pub id: String,
    /// The subscribing agent (none in open mode without a token).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriber: Option<String>,
    /// The client the subscriber subscribed through, for delivery-time constraint checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub topics: Vec<String>,
    pub inbox: String,
    /// The expiry the subscriber asked for, as given, and as seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl Subscription {
    fn expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }

    fn covers(&self, uri: &str) -> bool {
        self.topics.iter().any(|t| covers(t, uri))
    }

    fn agent(&self) -> Agent {
        Agent {
            subject: self.subscriber.clone(),
            client: self.client.clone(),
        }
    }
}

/// Whether a subscription to `topic` hears about `uri`: a topic covers itself and, for a
/// container, every resource transitively contained in it (section 10.3.2).
pub fn covers(topic: &str, uri: &str) -> bool {
    topic == uri || (topic.ends_with('/') && uri.starts_with(topic))
}

/// The subscriptions of the storage and the notification sender.
pub struct Notifier {
    subs: RwLock<BTreeMap<String, Subscription>>,
    /// Consecutive failed deliveries per subscription.
    failures: Mutex<HashMap<String, u32>>,
    /// Entity tag of the subscription listing; changes with its membership.
    etag: RwLock<String>,
    /// The delivery client: no redirects, no proxy, and (unless insecure fetches are allowed) a
    /// resolver that refuses non-public addresses.
    client: reqwest::Client,
}

fn new_etag() -> String {
    format!("\"{}\"", jose::random_id())
}

impl Notifier {
    /// Ensure the service container exists and load the subscriptions it holds.
    pub async fn load<S: Store>(store: &S, cfg: &LwsConfig) -> Result<Self, String> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !cfg.allow_insecure_fetch {
            builder = builder
                .dns_resolver(Arc::new(PublicOnlyResolver))
                .https_only(true);
        }
        let client = builder
            .build()
            .map_err(|e| format!("notification client: {e}"))?;
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        if !store
            .exists(&container)
            .await
            .map_err(|e| format!("store: {e}"))?
        {
            store
                .write(&container, Bytes::new(), LWS_JSON)
                .await
                .map_err(|e| format!("store: {e}"))?;
        }
        let mut subs = BTreeMap::new();
        let now = jose::now_secs();
        for child in store
            .list_children(&container)
            .await
            .map_err(|e| format!("store: {e}"))?
        {
            let Ok(r) = store.read(child.as_str()).await else {
                continue;
            };
            let Ok(mut sub) = serde_json::from_slice::<Subscription>(&r.body) else {
                continue;
            };
            sub.id = child
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            if sub.id.is_empty() {
                continue;
            }
            if sub.expired(now) {
                let _ = store.delete(child.as_str(), Some(&container)).await;
                continue;
            }
            subs.insert(sub.id.clone(), sub);
        }
        Ok(Self {
            subs: RwLock::new(subs),
            failures: Mutex::new(HashMap::new()),
            etag: RwLock::new(new_etag()),
            client,
        })
    }

    fn get(&self, id: &str) -> Option<Subscription> {
        self.subs
            .read()
            .expect("lock")
            .get(id)
            .filter(|s| !s.expired(jose::now_secs()))
            .cloned()
    }

    /// The live subscriptions, dropping (from memory) any that have expired; the store copy of
    /// an expired one is removed the next time it is touched or at boot.
    fn live(&self) -> Vec<Subscription> {
        let now = jose::now_secs();
        self.subs
            .read()
            .expect("lock")
            .values()
            .filter(|s| !s.expired(now))
            .cloned()
            .collect()
    }

    /// Remove a subscription from memory and the store.
    async fn remove<S: Store + 'static>(&self, state: &LwsState<S>, id: &str) -> bool {
        let removed = self.subs.write().expect("lock").remove(id).is_some();
        self.failures.lock().expect("lock").remove(id);
        if removed {
            *self.etag.write().expect("lock") = new_etag();
            let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
            let _ = state
                .store
                .delete(&format!("{container}{id}"), Some(&container))
                .await;
        }
        removed
    }

    /// Tell every subscriber whose topic covers `event.uri`, and who may read it now, that it
    /// changed. Awaited before a delete, while who may read the resource can still be decided;
    /// delivery itself happens in the background.
    pub async fn announce<S: Store + 'static>(&self, state: &LwsState<S>, event: Event) {
        let candidates: Vec<Subscription> = self
            .live()
            .into_iter()
            .filter(|s| s.covers(&event.uri))
            .collect();
        for sub in candidates {
            // Delivery-time authorization (section 10.3.3): a subscriber that may not read the
            // resource now hears nothing about it.
            if !state.allowed(Action::Read, &event.uri, &sub.agent()).await {
                continue;
            }
            let mut activity = json!({
                "type": [event.kind],
                "object": {"id": event.uri, "type": [if event.is_container { "Container" } else { "DataResource" }]},
            });
            if let Some((rel, container)) = &event.relation {
                activity[*rel] = Value::String(container.clone());
            }
            self.deliver(state, &sub.inbox, activity, Some(&sub.id));
        }
    }

    /// Deliver one notification wrapping `activity` to `inbox` in the background. `subscription`
    /// names the subscription it is for, whose failure count it feeds; `None` for a notification
    /// that belongs to no subscription (an access grant's or request's inbox).
    pub fn deliver<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        inbox: &str,
        activity: Value,
        subscription: Option<&str>,
    ) {
        let Some(target) = inbox_url(inbox, state.cfg.allow_insecure_fetch) else {
            return;
        };
        let body = Bytes::from(envelope(&state.cfg.storage(), activity).to_string());
        let keyid = format!("{}#{}", state.cfg.storage(), state.cfg.notify_key.kid());
        let state = state.clone();
        let subscription = subscription.map(str::to_string);
        tokio::spawn(async move {
            let status = attempt_delivery(&state, &target, &body, &keyid).await;
            let Some(id) = subscription else { return };
            let notifier = &state.notify;
            if status.is_some_and(|s| s.is_success()) {
                notifier.failures.lock().expect("lock").remove(&id);
                return;
            }
            let failures = {
                let mut f = notifier.failures.lock().expect("lock");
                let n = f.entry(id.clone()).or_insert(0);
                *n += 1;
                *n
            };
            if status == Some(StatusCode::GONE) || failures >= MAX_DELIVERY_FAILURES {
                notifier.remove(&state, &id).await;
            }
        });
    }
}

/// POST `body` to `target` up to [`DELIVERY_ATTEMPTS`] times, re-signing each time; the last
/// status, or `None` when the inbox could not be reached at all.
async fn attempt_delivery<S: Store + 'static>(
    state: &LwsState<S>,
    target: &url::Url,
    body: &Bytes,
    keyid: &str,
) -> Option<StatusCode> {
    let mut last = None;
    for attempt in 1..=DELIVERY_ATTEMPTS {
        let signed = sign(&state.cfg.notify_key, target, body, keyid, jose::now_secs());
        let result = state
            .notify
            .client
            .post(target.clone())
            .header(header::CONTENT_TYPE, LWS_JSON)
            .header("content-digest", &signed.content_digest)
            .header("signature-input", &signed.signature_input)
            .header("signature", &signed.signature)
            .body(body.clone())
            .send()
            .await;
        last = result.ok().map(|r| r.status());
        let retryable = last.is_none_or(|s| s.is_server_error());
        if !retryable || attempt == DELIVERY_ATTEMPTS {
            break;
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
    last
}

/// The Notification envelope around one activity (section 10.2): the LWS and Activity Streams
/// contexts, the storage, and the activity with a fresh `urn:uuid` id and a published time. No
/// actor: "The actor property SHOULD be omitted by default."
pub fn envelope(storage: &str, mut activity: Value) -> Value {
    activity["id"] = Value::String(format!("urn:uuid:{}", uuid_v4()));
    activity["published"] = Value::String(format_rfc3339(jose::now_secs()));
    json!({
        "@context": [LWS_CONTEXT, AS_CONTEXT],
        "type": "Notification",
        "storage": storage,
        "activity": activity,
    })
}

/// A random (version 4) UUID.
fn uuid_v4() -> String {
    let mut b = jose::random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

// ---- signing (RFC 9421, RFC 9530) ----

/// The headers that sign one delivery.
#[derive(Debug, Clone)]
pub struct Signed {
    pub content_digest: String,
    pub signature_input: String,
    pub signature: String,
}

/// RFC 9530 `Content-Digest` with SHA-256.
pub fn content_digest(body: &[u8]) -> String {
    format!("sha-256=:{}:", jose::b64(&Sha256::digest(body)))
}

/// The `@signature-params` value: the covered components, `created`, `keyid` and `alg`.
pub fn signature_params(created: i64, keyid: &str) -> String {
    let components: Vec<String> = SIGNATURE_COMPONENTS
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect();
    format!(
        "({});created={created};keyid=\"{keyid}\";alg=\"ecdsa-p256-sha256\"",
        components.join(" ")
    )
}

/// `@authority` (RFC 9421 section 2.2.3): the host, lower-cased, with the port only when it is not
/// the scheme's default.
fn authority(target: &url::Url) -> String {
    let host = target.host_str().unwrap_or_default().to_ascii_lowercase();
    match target.port() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    }
}

/// The RFC 9421 signature base of a POST of `content_type` with `digest` to `target`.
pub fn signature_base(target: &url::Url, content_type: &str, digest: &str, params: &str) -> String {
    let path = if target.path().is_empty() {
        "/"
    } else {
        target.path()
    };
    let values = [
        "POST".to_string(),
        target.scheme().to_string(),
        authority(target),
        path.to_string(),
        content_type.to_string(),
        digest.to_string(),
    ];
    let mut lines: Vec<String> = SIGNATURE_COMPONENTS
        .iter()
        .zip(values.iter())
        .map(|(c, v)| format!("\"{c}\": {v}"))
        .collect();
    lines.push(format!("\"@signature-params\": {params}"));
    lines.join("\n")
}

/// Sign a delivery of `body` to `target` with `key`, naming `keyid`, at `created`.
pub fn sign(
    key: &jose::EcKey,
    target: &url::Url,
    body: &[u8],
    keyid: &str,
    created: i64,
) -> Signed {
    let content_digest = content_digest(body);
    let params = signature_params(created, keyid);
    let base = signature_base(target, LWS_JSON, &content_digest, &params);
    let signature = format!("sig1=:{}:", jose::b64(&key.sign(base.as_bytes())));
    Signed {
        content_digest,
        signature_input: format!("sig1={params}"),
        signature,
    }
}

// ---- inbox safety ----

/// The inbox as a URL deliveries may go to: `http(s):` with a host, and, unless insecure fetches
/// are allowed, `https:` to a host that is neither `localhost` nor a non-public IP literal (names
/// are checked again when the delivery client resolves them).
pub fn inbox_url(inbox: &str, allow_insecure: bool) -> Option<url::Url> {
    let url = url::Url::parse(inbox).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
    {
        return None;
    }
    if allow_insecure {
        return Some(url);
    }
    if url.scheme() != "https" {
        return None;
    }
    let public = match url.host()? {
        url::Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d != "localhost" && !d.ends_with(".localhost")
        }
        url::Host::Ipv4(ip) => is_public(IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => is_public(IpAddr::V6(ip)),
    };
    public.then_some(url)
}

/// Whether `ip` is a globally routable unicast address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// Resolves names to their public addresses only, so a delivery cannot be pointed at the
/// server's own network by a name that resolves there.
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// ---- the NotificationService ----

/// Everything under `/.lws/subscriptions/`: the subscription listing and creation, and each
/// subscription's GET and DELETE.
pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    let id = &req.path[SUBSCRIPTIONS_PATH.len()..];
    let owner = state.cfg.open || (agent.subject.is_some() && agent.subject == state.cfg.owner);
    if id.is_empty() {
        return match req.method {
            Method::GET | Method::HEAD => {
                if state.needs_auth(agent) {
                    return state.challenge(None);
                }
                listing(state, agent, owner)
            }
            Method::POST => {
                if state.needs_auth(agent) {
                    return state.challenge(None);
                }
                subscribe(state, req, agent).await
            }
            _ => method_not_allowed("GET, HEAD, POST"),
        };
    }
    let sub = state.notify.get(id);
    let mine = sub
        .as_ref()
        .is_some_and(|s| owner || (agent.subject.is_some() && agent.subject == s.subscriber));
    let Some(sub) = sub.filter(|_| mine) else {
        // A subscription discloses its topics and inbox; to anyone else it does not exist.
        return if state.needs_auth(agent) {
            state.challenge(None)
        } else {
            problem(StatusCode::NOT_FOUND, None)
        };
    };
    match req.method {
        Method::GET | Method::HEAD => {
            let mut resp = super::json_response(StatusCode::OK, LWS_JSON, &document(state, &sub));
            add_link(
                resp.headers_mut(),
                &state.cfg.absolute(SUBSCRIPTIONS_PATH),
                "up",
                None,
            );
            add_link(
                resp.headers_mut(),
                &state.cfg.storage(),
                &format!("{LWS_NS}storage"),
                None,
            );
            resp
        }
        Method::DELETE => {
            state.notify.remove(state, id).await;
            problem(StatusCode::NO_CONTENT, None)
        }
        _ => method_not_allowed("GET, HEAD, DELETE"),
    }
}

/// The subscription document: type and subscription are REQUIRED; topic, inbox and expires echo
/// the request.
fn document<S: Store>(state: &LwsState<S>, sub: &Subscription) -> Value {
    let url = state
        .cfg
        .absolute(&format!("{SUBSCRIPTIONS_PATH}{}", sub.id));
    let mut doc = json!({
        "@context": [LWS_CONTEXT],
        "id": url,
        "type": WEBHOOK,
        "subscription": url,
        "topic": sub.topics,
        "inbox": sub.inbox,
    });
    if let Some(e) = &sub.expires {
        doc["expires"] = Value::String(e.clone());
    }
    doc
}

/// The subscriber's live subscriptions (every one for the owner) as an LWS container.
fn listing<S: Store + 'static>(state: &LwsState<S>, agent: &Agent, owner: bool) -> Response {
    let base = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let items: Vec<Value> = state
        .notify
        .live()
        .into_iter()
        .filter(|s| owner || (agent.subject.is_some() && agent.subject == s.subscriber))
        .map(|s| json!({"id": format!("{base}{}", s.id), "type": "DataResource", "format": LWS_JSON}))
        .collect();
    let body = json!({
        "@context": LWS_CONTEXT,
        "id": base,
        "type": "Container",
        "totalItems": items.len(),
        "items": items,
    });
    let mut resp = super::json_response(StatusCode::OK, LWS_JSON, &body);
    set(
        resp.headers_mut(),
        header::ETAG,
        &state.notify.etag.read().expect("lock"),
    );
    add_link(
        resp.headers_mut(),
        &format!("{LWS_NS}Container"),
        "type",
        None,
    );
    add_link(
        resp.headers_mut(),
        &state.cfg.storage(),
        &format!("{LWS_NS}storage"),
        None,
    );
    resp
}

async fn subscribe<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    // "The request body MUST conform to the application/lws+json media type."
    if req.content_type().as_deref() != Some(LWS_JSON) {
        return problem(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Some("a subscription request is application/lws+json"),
        );
    }
    let Ok(body) = serde_json::from_slice::<Value>(&req.body) else {
        return problem(StatusCode::BAD_REQUEST, Some("the body is not JSON"));
    };
    if !body.is_object() {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("the body is not a JSON object"),
        );
    }
    // type is REQUIRED and one of the advertised subscription types.
    let ty = body.get("type").and_then(Value::as_str);
    if ty != Some(WEBHOOK) && ty.and_then(|t| t.strip_prefix(LWS_NS)) != Some(WEBHOOK) {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("type must be WebhookSubscription"),
        );
    }
    // topic is REQUIRED: a non-empty array of URIs.
    let topics: Vec<String> = match body.get("topic") {
        Some(Value::Array(a)) if !a.is_empty() && a.iter().all(json_is_uri) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => {
            return problem(
                StatusCode::BAD_REQUEST,
                Some("topic must be a non-empty array of URIs"),
            )
        }
    };
    // A WebhookSubscription's inbox is REQUIRED.
    let Some(inbox) = body
        .get("inbox")
        .filter(|v| json_is_uri(v))
        .and_then(Value::as_str)
    else {
        return problem(StatusCode::BAD_REQUEST, Some("inbox must be a URI"));
    };
    if inbox_url(inbox, state.cfg.allow_insecure_fetch).is_none() {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("inbox must be a public https URL"),
        );
    }
    let (expires, expires_at) = match body.get("expires") {
        None | Some(Value::Null) => (None, None),
        Some(Value::String(e)) => match parse_rfc3339(e) {
            Some(t) if t > jose::now_secs() => (Some(e.clone()), Some(t)),
            Some(_) => return problem(StatusCode::BAD_REQUEST, Some("expires is in the past")),
            None => {
                return problem(
                    StatusCode::BAD_REQUEST,
                    Some("expires must be an RFC 3339 datetime"),
                )
            }
        },
        Some(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                Some("expires must be an RFC 3339 datetime"),
            )
        }
    };
    // "If a subscriber does not have the equivalent of read access to all resources listed in the
    // topic array, the server MUST reject the subscription request." A topic outside this storage,
    // or naming nothing, is refused the same way.
    let storage = state.cfg.storage();
    for topic in &topics {
        let exists =
            topic.starts_with(&storage) && state.store.exists(topic).await.unwrap_or(false);
        if !exists || !state.allowed(Action::Read, topic, agent).await {
            return if agent.is_authenticated() || state.cfg.open {
                problem(
                    StatusCode::FORBIDDEN,
                    Some("the subscriber cannot read every topic"),
                )
            } else {
                state.challenge(None)
            };
        }
    }
    let id = jose::random_id();
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let iri = format!("{container}{id}");
    let sub = Subscription {
        id: id.clone(),
        subscriber: agent.subject.clone(),
        client: agent.client.clone(),
        topics,
        inbox: inbox.to_string(),
        expires,
        expires_at,
    };
    let stored = serde_json::to_vec(&sub).unwrap_or_default();
    if let Err(e) = state
        .store
        .create_in_container(&container, &iri, Bytes::from(stored), LWS_JSON)
        .await
    {
        return problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    }
    let doc = document(state, &sub);
    state.notify.subs.write().expect("lock").insert(id, sub);
    *state.notify.etag.write().expect("lock") = new_etag();
    let mut resp = super::json_response(StatusCode::CREATED, LWS_JSON, &doc);
    set(resp.headers_mut(), header::LOCATION, &iri);
    add_link(
        resp.headers_mut(),
        &state.cfg.storage(),
        &format!("{LWS_NS}storage"),
        None,
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::{Signature, VerifyingKey};

    #[test]
    fn topic_coverage() {
        assert!(covers("https://s/a/", "https://s/a/"));
        assert!(covers("https://s/a/", "https://s/a/b"));
        assert!(covers("https://s/a/", "https://s/a/b/c/d"));
        assert!(covers("https://s/a/x", "https://s/a/x"));
        assert!(!covers("https://s/a/x", "https://s/a/xy"));
        assert!(!covers("https://s/a/x", "https://s/a/x/y"));
        assert!(!covers("https://s/a/", "https://s/ab"));
        assert!(!covers("https://s/a/", "https://s/"));
    }

    #[test]
    fn signature_base_layout() {
        let target = url::Url::parse("https://Receiver.Example:443/hooks/lws?x=1").unwrap();
        let params = signature_params(1_700_000_000, "https://s/#notify-key");
        assert_eq!(
            params,
            "(\"@method\" \"@scheme\" \"@authority\" \"@path\" \"content-type\" \"content-digest\");created=1700000000;keyid=\"https://s/#notify-key\";alg=\"ecdsa-p256-sha256\""
        );
        let base = signature_base(&target, LWS_JSON, "sha-256=:abc=:", &params);
        let expected = format!(
            "\"@method\": POST\n\"@scheme\": https\n\"@authority\": receiver.example\n\"@path\": /hooks/lws\n\
             \"content-type\": application/lws+json\n\"content-digest\": sha-256=:abc=:\n\"@signature-params\": {params}"
        );
        assert_eq!(base, expected);
        let local = url::Url::parse("http://localhost:3938").unwrap();
        assert!(signature_base(&local, LWS_JSON, "d", "p")
            .contains("\"@authority\": localhost:3938\n\"@path\": /\n"));
    }

    #[test]
    fn content_digest_is_rfc9530() {
        // SHA-256 of the empty string.
        assert_eq!(
            content_digest(b""),
            "sha-256=:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=:"
        );
    }

    #[test]
    fn signature_verifies() {
        let key = jose::EcKey::generate("notify-key");
        let target = url::Url::parse("https://receiver.example/in").unwrap();
        let signed = sign(&key, &target, b"{}", "https://s/#notify-key", 42);
        let params = signed.signature_input.strip_prefix("sig1=").unwrap();
        let base = signature_base(&target, LWS_JSON, &signed.content_digest, params);
        let raw = signed
            .signature
            .strip_prefix("sig1=:")
            .and_then(|s| s.strip_suffix(':'))
            .unwrap();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .unwrap();
        let sig = Signature::from_slice(&bytes).unwrap();
        VerifyingKey::from(&key.public_key())
            .verify(base.as_bytes(), &sig)
            .unwrap();
    }

    #[test]
    fn inbox_safety() {
        assert!(inbox_url("https://receiver.example/in", false).is_some());
        assert!(inbox_url("http://receiver.example/in", false).is_none());
        assert!(inbox_url("https://localhost/in", false).is_none());
        assert!(inbox_url("https://127.0.0.1/in", false).is_none());
        assert!(inbox_url("https://10.1.2.3/in", false).is_none());
        assert!(inbox_url("https://[::1]/in", false).is_none());
        assert!(inbox_url("https://[::ffff:192.168.0.1]/in", false).is_none());
        assert!(inbox_url("mailto:a@b", true).is_none());
        assert!(inbox_url("http://localhost:3938/in", true).is_some());
        assert!(is_public("8.8.8.8".parse().unwrap()));
        assert!(!is_public("169.254.169.254".parse().unwrap()));
        assert!(!is_public("100.64.0.1".parse().unwrap()));
    }

    #[test]
    fn envelope_shape() {
        let e = envelope(
            "https://s/",
            json!({"type": ["Update"], "object": {"id": "https://s/x", "type": ["DataResource"]}}),
        );
        assert_eq!(e["type"], "Notification");
        assert_eq!(e["storage"], "https://s/");
        assert!(e["activity"]["id"]
            .as_str()
            .unwrap()
            .starts_with("urn:uuid:"));
        assert!(parse_rfc3339(e["activity"]["published"].as_str().unwrap()).is_some());
        assert!(e["activity"].get("actor").is_none());
    }
}

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
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::access::{format_rfc3339, parse_rfc3339, Action};
use super::{
    etag_of, jose, json_is_uri, method_not_allowed, none_match, problem, service_links,
    service_linkset, service_listing, set, Agent, LwsConfig, LwsRequest, LwsState, AS_CONTEXT,
    LWS_CONTEXT, LWS_JSON, LWS_NS, META_SUFFIX, SUBSCRIPTIONS_PATH,
};
use crate::error::ServerError;
use crate::store::Store;

/// The one subscription type this NotificationService offers.
pub const WEBHOOK: &str = "WebhookSubscription";

/// Attempts at one delivery: a 5xx or an unreachable inbox is tried again.
const DELIVERY_ATTEMPTS: u32 = 3;
/// Pause between attempts at one delivery.
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// Consecutive failed deliveries before a subscription is deactivated.
const MAX_DELIVERY_FAILURES: u32 = 5;

/// Bounds on outgoing webhook deliveries, so many subscriptions times many changes cannot exhaust
/// sockets or memory.
///
/// A delivery is admitted while fewer than `queue` are waiting or in flight; past that it is
/// dropped (and logged), which counts toward no subscription's failures. At most `workers` are in
/// flight at once, and at most `per_inbox` to any one inbox origin (scheme, host and port, so
/// many paths on one host share a limit). A delivery waits for its inbox's turn before it takes a
/// worker, so a slow inbox holds back only its own deliveries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryLimits {
    /// Deliveries waiting or in flight (`SOLID_SERVER_LWS_DELIVERY_QUEUE`, default 1024).
    pub queue: usize,
    /// Deliveries in flight (`SOLID_SERVER_LWS_DELIVERY_WORKERS`, default 16).
    pub workers: usize,
    /// Deliveries in flight to one inbox origin (`SOLID_SERVER_LWS_DELIVERY_PER_INBOX`, default 2).
    pub per_inbox: usize,
}

impl Default for DeliveryLimits {
    fn default() -> Self {
        Self {
            queue: 1024,
            workers: 16,
            per_inbox: 2,
        }
    }
}

/// One admitted delivery: its place in the queue, given back when it is dropped.
struct Admitted(Arc<AtomicUsize>);

impl Drop for Admitted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

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

/// A notification addressed to one subscriber, not yet sent (see [`Notifier::prepare`]).
#[derive(Debug, Clone)]
pub struct Pending {
    inbox: String,
    activity: Value,
    subscription: String,
}

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
    limits: DeliveryLimits,
    /// Deliveries admitted and not yet finished.
    admitted: Arc<AtomicUsize>,
    /// Deliveries dropped because the queue was full.
    dropped: AtomicU64,
    /// The worker pool: one permit per delivery in flight.
    workers: Arc<tokio::sync::Semaphore>,
    /// Per inbox origin, its own limit (dropped once no delivery holds it).
    inboxes: Mutex<HashMap<String, Weak<tokio::sync::Semaphore>>>,
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
                .dns_resolver(Arc::new(super::PublicOnlyResolver))
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
            limits: cfg.delivery,
            admitted: Arc::new(AtomicUsize::new(0)),
            dropped: AtomicU64::new(0),
            workers: Arc::new(tokio::sync::Semaphore::new(cfg.delivery.workers.max(1))),
            inboxes: Mutex::new(HashMap::new()),
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

    /// Remove a subscription from the store, then from memory. A store failure leaves it in place
    /// (a cancelled subscription whose stored copy survived would come back at the next boot) and
    /// is returned.
    async fn remove<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        id: &str,
    ) -> Result<(), String> {
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        match state
            .store
            .delete(&format!("{container}{id}"), Some(&container))
            .await
        {
            Ok(_) | Err(ServerError::NotFound) => {}
            Err(e) => return Err(e.to_string()),
        }
        if self.subs.write().expect("lock").remove(id).is_some() {
            *self.etag.write().expect("lock") = new_etag();
        }
        self.failures.lock().expect("lock").remove(id);
        Ok(())
    }

    /// Tell every subscriber whose topic covers `event.uri`, and who may read it now, that it
    /// changed. Delivery itself happens in the background.
    pub async fn announce<S: Store + 'static>(&self, state: &LwsState<S>, event: Event) {
        let pending = self.prepare(state, &event).await;
        self.send(state, pending);
    }

    /// The notifications `event` makes, addressed but not yet sent: one per subscriber whose topic
    /// covers `event.uri` and who may read it now. A delete prepares its notifications before the
    /// resource goes, while who may read it can still be decided, and [`Notifier::send`]s them only
    /// once the removal is confirmed.
    pub async fn prepare<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: &Event,
    ) -> Vec<Pending> {
        let candidates: Vec<Subscription> = self
            .live()
            .into_iter()
            .filter(|s| s.covers(&event.uri))
            .collect();
        let mut out = Vec::new();
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
            out.push(Pending {
                inbox: sub.inbox,
                activity,
                subscription: sub.id,
            });
        }
        out
    }

    /// Deliver notifications [`Notifier::prepare`]d earlier.
    pub fn send<S: Store + 'static>(&self, state: &LwsState<S>, pending: Vec<Pending>) {
        for p in pending {
            self.deliver(state, &p.inbox, p.activity, Some(&p.subscription));
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
        let Some(admitted) = self.admit() else {
            let dropped = self.dropped.fetch_add(1, Ordering::SeqCst) + 1;
            // Logged at the first drop and then at each power of two, so a flood cannot flood the
            // log too.
            if dropped.is_power_of_two() {
                eprintln!(
                    "lws: webhook delivery queue full ({} waiting or in flight); dropped a \
                     notification to {target} ({dropped} dropped so far)",
                    self.limits.queue
                );
            }
            return;
        };
        let inbox_slot = self.inbox_slot(&target);
        let workers = self.workers.clone();
        let body = Bytes::from(envelope(&state.cfg.storage(), activity).to_string());
        let keyid = format!("{}#{}", state.cfg.storage(), state.cfg.notify_key.kid());
        let state = state.clone();
        let subscription = subscription.map(str::to_string);
        tokio::spawn(async move {
            let _admitted = admitted;
            // The inbox's turn first, then a worker: a delivery waiting on a busy inbox holds no
            // worker.
            let Ok(_inbox) = inbox_slot.acquire_owned().await else {
                return;
            };
            let Ok(_worker) = workers.acquire_owned().await else {
                return;
            };
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
                // A store failure keeps the subscription (and its failure count), so the next
                // failed delivery tries to deactivate it again.
                let _ = notifier.remove(&state, &id).await;
            }
        });
    }
}

impl Notifier {
    /// A place in the delivery queue, or `None` when it is full.
    fn admit(&self) -> Option<Admitted> {
        let limit = self.limits.queue;
        self.admitted
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < limit).then_some(n + 1)
            })
            .ok()
            .map(|_| Admitted(self.admitted.clone()))
    }

    /// The limit shared by every delivery to `target`'s origin.
    fn inbox_slot(&self, target: &url::Url) -> Arc<tokio::sync::Semaphore> {
        let origin = target.origin().ascii_serialization();
        let mut map = self.inboxes.lock().expect("lock");
        if map.len() > 1024 {
            map.retain(|_, w| w.strong_count() > 0);
        }
        if let Some(s) = map.get(&origin).and_then(Weak::upgrade) {
            return s;
        }
        let s = Arc::new(tokio::sync::Semaphore::new(self.limits.per_inbox.max(1)));
        map.insert(origin, Arc::downgrade(&s));
        s
    }

    /// Deliveries dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
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

/// Whether `ip` is a globally routable unicast address: what the authorization server's fetches
/// may reach too (see [`super::is_forbidden_ip`]).
pub fn is_public(ip: IpAddr) -> bool {
    !super::is_forbidden_ip(ip)
}

// ---- the NotificationService ----

/// Everything under `/.lws/subscriptions/`: the subscription listing and creation, and each
/// subscription's GET and DELETE.
pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    let mut id = &req.path[SUBSCRIPTIONS_PATH.len()..];
    // `{container}.meta` and `{member}.meta` are the (read-only) linksets the container and its
    // members link to.
    let linkset = id.ends_with(META_SUFFIX);
    if linkset {
        id = &id[..id.len() - META_SUFFIX.len()];
    }
    let owner = state.cfg.open || (agent.subject.is_some() && agent.subject == state.cfg.owner);
    if id.is_empty() {
        if linkset {
            return if state.needs_auth(agent) {
                state.challenge(None)
            } else {
                service_linkset(&state.cfg, req, &state.cfg.absolute(SUBSCRIPTIONS_PATH))
            };
        }
        return match req.method {
            Method::GET | Method::HEAD => {
                if state.needs_auth(agent) {
                    return state.challenge(None);
                }
                listing(state, req, agent, owner)
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
    if linkset {
        let iri = state.cfg.absolute(&format!("{SUBSCRIPTIONS_PATH}{id}"));
        return service_linkset(&state.cfg, req, &iri);
    }
    match req.method {
        Method::GET | Method::HEAD => {
            let doc = document(state, &sub);
            let iri = state.cfg.absolute(&format!("{SUBSCRIPTIONS_PATH}{id}"));
            // A subscription never changes once made, so its document is its entity tag.
            let etag = etag_of([doc.to_string().as_str()]);
            let mut resp = if none_match(req, &etag) {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                super::json_response(StatusCode::OK, LWS_JSON, &doc)
            };
            set(resp.headers_mut(), header::ETAG, &etag);
            service_links(
                &state.cfg,
                resp.headers_mut(),
                &iri,
                &state.cfg.absolute(SUBSCRIPTIONS_PATH),
            );
            resp
        }
        Method::DELETE => match state.notify.remove(state, id).await {
            Ok(()) => problem(StatusCode::NO_CONTENT, None),
            Err(e) => problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(&format!("cannot cancel the subscription: {e}")),
            ),
        },
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

/// The subscriber's live subscriptions (every one for the owner) as an LWS container: negotiated,
/// paged and tagged like any other container (webhook "Subscription Management").
fn listing<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    owner: bool,
) -> Response {
    let base = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let items: Vec<Value> = state
        .notify
        .live()
        .into_iter()
        .filter(|s| owner || (agent.subject.is_some() && agent.subject == s.subscriber))
        .map(|s| json!({"id": format!("{base}{}", s.id), "type": "DataResource", "format": LWS_JSON}))
        .collect();
    let version = state.notify.etag.read().expect("lock").clone();
    let mut resp = service_listing(&state.cfg, req, &base, items, &version);
    // Each subscriber sees only its own subscriptions.
    set(resp.headers_mut(), header::VARY, "Accept, Authorization");
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
    service_links(&state.cfg, resp.headers_mut(), &iri, &container);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::{Signature, VerifyingKey};

    use super::super::test_store;

    /// A subscription to the storage root, put straight into the state and the store.
    async fn subscribe_root(state: &LwsState<test_store::FlakyStore>, id: &str) -> String {
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let sub = Subscription {
            id: id.into(),
            subscriber: None,
            client: None,
            topics: vec![state.cfg.storage()],
            inbox: "https://inbox.example/".into(),
            expires: None,
            expires_at: None,
        };
        let iri = format!("{container}{id}");
        state
            .store
            .create_in_container(
                &container,
                &iri,
                Bytes::from(serde_json::to_vec(&sub).unwrap()),
                LWS_JSON,
            )
            .await
            .unwrap();
        state.notify.subs.write().unwrap().insert(id.into(), sub);
        *state.notify.etag.write().unwrap() = new_etag();
        format!("{SUBSCRIPTIONS_PATH}{id}")
    }

    #[tokio::test]
    async fn subscription_listing_and_get() {
        let (state, _) = test_store::state(2).await;
        for id in ["a", "b", "c"] {
            subscribe_root(&state, id).await;
        }
        let anon = Agent::anonymous();
        let get = |path: &str, headers: &[(&str, &str)]| {
            test_store::request(Method::GET, path, headers, "")
        };
        let list = handle(&state, &get(SUBSCRIPTIONS_PATH, &[]), &anon).await;
        assert_eq!(list.status(), StatusCode::OK);
        assert!(list.headers().contains_key(header::ETAG));
        assert!(list.headers()[header::VARY]
            .to_str()
            .unwrap()
            .contains("Accept"));
        assert_eq!(test_store::links(&list, "next").len(), 1);
        assert_eq!(
            test_store::links(&list, "type"),
            vec![format!("{LWS_NS}Container")]
        );
        assert_eq!(test_store::body_json(list).await["totalItems"], 3);
        let page2 = handle(
            &state,
            &get(&format!("{SUBSCRIPTIONS_PATH}?page=2"), &[]),
            &anon,
        )
        .await;
        assert_eq!(
            test_store::body_json(page2).await["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let refused = handle(
            &state,
            &get(SUBSCRIPTIONS_PATH, &[("accept", "text/turtle")]),
            &anon,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::NOT_ACCEPTABLE);
        // One subscription: an entity tag that revalidates.
        let one = handle(&state, &get(&format!("{SUBSCRIPTIONS_PATH}a"), &[]), &anon).await;
        assert_eq!(one.status(), StatusCode::OK);
        let etag = one.headers()[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(
            test_store::links(&one, "up"),
            vec![state.cfg.absolute(SUBSCRIPTIONS_PATH)]
        );
        let again = handle(
            &state,
            &get(
                &format!("{SUBSCRIPTIONS_PATH}a"),
                &[("if-none-match", &etag)],
            ),
            &anon,
        )
        .await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
    }

    /// Review finding: a non-ASCII byte in `expires`' timezone panicked the parser (and, with
    /// panic=abort, the server); it is a 400 like any other bad datetime.
    #[tokio::test]
    async fn a_malformed_expires_is_a_400_not_a_panic() {
        let (state, _) = test_store::state(100).await;
        for expires in ["2026-10-05T00:00:00+0\u{e9}00", "2099-01-01T00:00:00+0a:00"] {
            let body = json!({
                "type": WEBHOOK,
                "topic": [state.cfg.storage()],
                "inbox": "https://inbox.example/in",
                "expires": expires,
            });
            let req = test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON)],
                &body.to_string(),
            );
            let r = handle(&state, &req, &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{expires:?}");
        }
    }

    /// Review finding: every delivery was its own unbounded task. Deliveries are now admitted to
    /// a bounded queue (the rest dropped and counted), run on a fixed worker pool, and limited
    /// per inbox origin.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deliveries_are_bounded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (now, peak, total) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        let app = {
            let (now, peak, total) = (now.clone(), peak.clone(), total.clone());
            axum::Router::new().route(
                "/inbox",
                axum::routing::post(move || {
                    let (now, peak, total) = (now.clone(), peak.clone(), total.clone());
                    async move {
                        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        now.fetch_sub(1, Ordering::SeqCst);
                        total.fetch_add(1, Ordering::SeqCst);
                        StatusCode::NO_CONTENT
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.delivery = DeliveryLimits {
            queue: 4,
            workers: 8,
            per_inbox: 2,
        };
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let inbox = format!("http://{addr}/inbox");
        for _ in 0..20 {
            state
                .notify
                .deliver(&state, &inbox, json!({"type": ["Update"]}), None);
        }
        assert_eq!(state.notify.dropped(), 16);
        for _ in 0..100 {
            if total.load(Ordering::SeqCst) >= 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(total.load(Ordering::SeqCst), 4);
        assert!(peak.load(Ordering::SeqCst) <= 2, "{peak:?}");
        // Places are given back: once the queue drains, deliveries are admitted again.
        for _ in 0..100 {
            if state.notify.admitted.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        state
            .notify
            .deliver(&state, &inbox, json!({"type": ["Update"]}), None);
        assert_eq!(state.notify.dropped(), 16);
    }

    #[tokio::test]
    async fn a_failed_cancellation_keeps_the_subscription() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let path = subscribe_root(&state, "s1").await;
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        store.fail_delete.store(true, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(state.notify.get("s1").is_some());
        let iri = state.cfg.absolute(&path);
        assert!(state.store.exists(&iri).await.unwrap());
        store.fail_delete.store(false, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.notify.get("s1").is_none());
        assert!(!state.store.exists(&iri).await.unwrap());
    }

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
        // Review finding: the webhook predicate admitted what the fetch predicate refuses.
        for ip in [
            "fec0::1",
            "64:ff9b::a00:1",
            "::a00:1",
            "192.0.0.8",
            "198.18.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
            assert!(
                inbox_url(
                    &format!("https://[{ip}]/in")
                        .replace("[192.0.0.8]", "192.0.0.8")
                        .replace("[198.18.0.1]", "198.18.0.1"),
                    false
                )
                .is_none(),
                "{ip}"
            );
        }
        assert!(is_public("2606:4700::1".parse().unwrap()));
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

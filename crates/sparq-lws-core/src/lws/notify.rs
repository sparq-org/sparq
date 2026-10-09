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

use super::access::{self, format_rfc3339, parse_rfc3339, Action};
use super::{
    etag_of, jose, json_is_uri, method_not_allowed, problem, service_links, service_linkset,
    service_listing, set, Agent, LwsConfig, LwsRequest, LwsState, AS_CONTEXT, LWS_CONTEXT,
    LWS_JSON, LWS_NS, META_SUFFIX, SUBSCRIPTIONS_PATH,
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
    watch: Watch,
}

/// What a delivery for a subscription is checked against before every attempt: the subscription
/// must still stand, and its subscriber still be allowed to read the resource the notification is
/// about.
#[derive(Debug, Clone)]
pub struct Watch {
    subscription: String,
    uri: String,
    agent: Agent,
    /// For a Delete, the resource as it was before it went (see [`access::Snapshot`]); otherwise
    /// the resource is read as it is at each attempt.
    snapshot: Option<Arc<access::Snapshot>>,
}

impl Watch {
    /// Whether the delivery may still be made: the subscription stands (not cancelled, not
    /// expired) and its subscriber may read the resource (grants revoked or expired since count).
    /// The resource is read under its shared lock, as a GET reads it, so the decision rests on a
    /// state some write left whole, never on one a write is part way through (or rolling back).
    /// The lock is not held through the send: a slow inbox must not hold up writes.
    async fn stands<S: Store + 'static>(&self, state: &LwsState<S>) -> bool {
        if !state.notify.is_live(&self.subscription) {
            return false;
        }
        match &self.snapshot {
            Some(s) => access::allowed_as(state, Action::Read, &self.uri, &self.agent, s).await,
            None => {
                // A resource that is not visible is not read (its lock is held until it is put
                // back): the delivery fails its check rather than wait, including when the
                // resource is set aside while the delivery waits for its lock.
                let Some(_guard) = state.read_visible(&self.uri).await else {
                    return false;
                };
                // The subscription may have been cancelled, or expired, meanwhile.
                state.notify.is_live(&self.subscription)
                    && state.allowed(Action::Read, &self.uri, &self.agent).await
            }
        }
    }
}

/// Most distinct topics one subscription may name.
pub const MAX_TOPICS: usize = 64;

/// Most subscriptions held at once by subscribers other than the storage owner, expired ones
/// included until they are removed: anyone who can read a resource may subscribe to it, and each
/// subscription is stored as a resource is.
pub const MAX_SUBSCRIPTIONS: usize = 512;

/// Most subscriptions one subscriber other than the storage owner may hold. The owner has a
/// share of [`MAX_SUBSCRIPTIONS`] of its own.
pub const MAX_SUBSCRIPTIONS_PER_SUBSCRIBER: usize = 16;

/// Largest subscription request body, in bytes.
pub const MAX_SUBSCRIPTION_BYTES: usize = 16 * 1024;

/// Most expired subscriptions a new subscription removes before it is counted.
const EXPIRED_REMOVED_PER_SUBSCRIBE: usize = 16;

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
    /// Consecutive failed deliveries, in memory only. Kept in the subscription itself, so it goes
    /// when the subscription goes, and a delivery that finishes later finds nothing to count.
    #[serde(skip)]
    pub failures: u32,
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
    /// The share of the store subscriptions may take (see [`super::Quota`]).
    quota: super::Quota,
    /// The storage owner's own share, apart from everyone else's.
    owner_quota: super::Quota,
    /// The last expired subscription a purge tried: the next starts after it, so ones that cannot
    /// be removed do not keep the rest from being tried.
    purged_to: Mutex<String>,
}

fn new_etag() -> String {
    format!("\"{}\"", jose::random_id())
}

impl Notifier {
    /// Put a subscription of `inbox` to the storage root in force, in memory only.
    #[cfg(test)]
    pub fn subscribe_root_for_test(&self, storage: &str, inbox: &str) {
        let sub = Subscription {
            id: "test".into(),
            subscriber: None,
            client: None,
            topics: vec![storage.to_string()],
            inbox: inbox.to_string(),
            expires: None,
            expires_at: None,
            failures: 0,
        };
        self.subs.write().expect("lock").insert("test".into(), sub);
    }

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
            // Every stored subscription counts against the quota, so one that cannot be read
            // stops the load rather than being left stored and uncounted.
            let r = store
                .read(child.as_str())
                .await
                .map_err(|e| format!("subscription {}: {e}", child.as_str()))?;
            let mut sub = serde_json::from_slice::<Subscription>(&r.body)
                .map_err(|e| format!("subscription {}: {e}", child.as_str()))?;
            sub.id = child
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            if sub.id.is_empty() {
                continue;
            }
            // An expired subscription is purged; one that cannot be removed stays loaded and
            // counted (no delivery goes to it: every attempt checks expiry) until it can be.
            if sub.expired(now)
                && matches!(
                    super::remove_member(store, child.as_str(), Some(&container)).await,
                    Ok(crate::store::DeleteOutcome::Deleted
                        | crate::store::DeleteOutcome::NotFound)
                )
            {
                continue;
            }
            subs.insert(sub.id.clone(), sub);
        }
        Ok(Self {
            subs: RwLock::new(subs),
            etag: RwLock::new(new_etag()),
            client,
            limits: cfg.delivery,
            admitted: Arc::new(AtomicUsize::new(0)),
            dropped: AtomicU64::new(0),
            workers: Arc::new(tokio::sync::Semaphore::new(cfg.delivery.workers.max(1))),
            inboxes: Mutex::new(HashMap::new()),
            quota: super::Quota::new(MAX_SUBSCRIPTIONS, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER),
            owner_quota: super::Quota::new(MAX_SUBSCRIPTIONS, MAX_SUBSCRIPTIONS),
            purged_to: Mutex::new(String::new()),
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

    /// Whether the subscription `id` still stands: not cancelled (deleted, or deactivated after
    /// failed deliveries) and not expired.
    fn is_live(&self, id: &str) -> bool {
        let now = jose::now_secs();
        self.subs
            .read()
            .expect("lock")
            .get(id)
            .is_some_and(|s| !s.expired(now))
    }

    /// Remove a subscription from the store, then from memory. A store failure leaves it in place
    /// (a cancelled subscription whose stored copy survived would come back at the next boot) and
    /// is returned.
    ///
    /// Once the stored record is gone the subscription is cancelled in memory too, whatever
    /// happens after (a cleanup that fails, a caller that goes away): the removal and the
    /// cancellation run in a task of their own.
    ///
    /// `admission` is the request's share of its admission permit, when a request asked for the
    /// removal: the task holds it until it ends.
    async fn remove<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        id: &str,
        admission: Option<crate::overload::AdmissionSlot>,
    ) -> Result<(), String> {
        let (state, id) = (state.clone(), id.to_string());
        let cancel = async move {
            let _admission = admission;
            let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
            // Shared with other members' changes; a conditional create holds it alone.
            let _listing = state.locks.read(&container).await;
            super::delete_record(&state.store, &format!("{container}{id}"), &container)
                .await
                .map_err(|e| e.to_string())?;
            let me = &state.notify;
            // An expired subscription is listed nowhere, so its removal changes no listing.
            let removed = me.subs.write().expect("lock").remove(&id);
            if removed.is_some_and(|s| !s.expired(jose::now_secs())) {
                *me.etag.write().expect("lock") = new_etag();
            }
            Ok(())
        };
        tokio::spawn(cancel).await.map_err(|e| e.to_string())?
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
    ///
    /// At most as many as the delivery queue holds are prepared ([`Notifier::prepare_at_most`]):
    /// past that they would be dropped when sent.
    pub async fn prepare<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: &Event,
    ) -> Vec<Pending> {
        self.prepare_at_most(state, event, self.limits.queue).await
    }

    /// [`Notifier::prepare`], for at most `limit` subscriptions: a change that prepares many
    /// (a recursive delete prepares one per resource it removes) shares one bound between them.
    pub async fn prepare_at_most<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: &Event,
        limit: usize,
    ) -> Vec<Pending> {
        // Only the subscriptions within the bound are copied out; those past it are dropped as a
        // full queue drops them, and counted so.
        let now = jose::now_secs();
        // A resource's linkset changes with it, and is read under its authorization: a
        // subscription to the linkset itself hears of it as of the resource.
        let linkset = format!("{}{META_SUFFIX}", event.uri);
        let names_linkset = |s: &Subscription| s.topics.contains(&linkset);
        let (candidates, over) = {
            let subs = self.subs.read().expect("lock");
            let mut matching = subs
                .values()
                .filter(|s| !s.expired(now) && (s.covers(&event.uri) || names_linkset(s)));
            let candidates: Vec<Subscription> = matching.by_ref().take(limit).cloned().collect();
            (candidates, matching.count())
        };
        if over > 0 {
            self.note_dropped(over, &event.uri);
        }
        let mut out = Vec::new();
        // A Delete is prepared while the resource is still there; its deliveries, made once it
        // is gone, are authorized against what it was.
        let snapshot = if event.kind == "Delete" && !candidates.is_empty() {
            match access::Snapshot::take(state, &event.uri).await {
                Ok(s) => Some(Arc::new(s)),
                Err(_) => return out,
            }
        } else {
            None
        };
        for sub in candidates {
            // Delivery-time authorization (section 10.3.3): a subscriber that may not read the
            // resource now hears nothing about it.
            if !state.allowed(Action::Read, &event.uri, &sub.agent()).await {
                continue;
            }
            let mut activities = Vec::new();
            if sub.covers(&event.uri) {
                let mut activity = json!({
                    "type": [event.kind],
                    "object": {"id": event.uri, "type": [if event.is_container { "Container" } else { "DataResource" }]},
                });
                if let Some((rel, container)) = &event.relation {
                    activity[*rel] = Value::String(container.clone());
                }
                activities.push(activity);
            }
            if names_linkset(&sub) {
                activities.push(json!({
                    "type": [event.kind],
                    "object": {"id": linkset, "type": ["DataResource"]},
                }));
            }
            // The bound is on what is prepared: a subscription naming both the resource and its
            // linkset brings two, and what is past the bound is dropped and counted so.
            for activity in activities {
                if out.len() == limit {
                    self.note_dropped(1, &event.uri);
                    continue;
                }
                out.push(Pending {
                    inbox: sub.inbox.clone(),
                    activity,
                    // Checked as the resource the linkset describes: its lock and who may
                    // read it.
                    watch: Watch {
                        agent: sub.agent(),
                        subscription: sub.id.clone(),
                        uri: event.uri.clone(),
                        snapshot: snapshot.clone(),
                    },
                });
            }
        }
        out
    }

    /// Deliver notifications [`Notifier::prepare`]d earlier.
    pub fn send<S: Store + 'static>(&self, state: &LwsState<S>, pending: Vec<Pending>) {
        for p in pending {
            self.deliver(state, &p.inbox, p.activity, Some(p.watch));
        }
    }

    /// Deliver one notification wrapping `activity` to `inbox` in the background. `watch` names
    /// the subscription it is for, whose failure count it feeds, and what each attempt is checked
    /// against; `None` for a notification that belongs to no subscription (an access grant's or
    /// request's inbox).
    pub fn deliver<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        inbox: &str,
        activity: Value,
        watch: Option<Watch>,
    ) {
        let Some(target) = inbox_url(inbox, state.cfg.allow_insecure_fetch) else {
            return;
        };
        let Some(admitted) = self.admit_or_drop(&target) else {
            return;
        };
        self.run(state, admitted, target, activity, watch);
    }

    /// Deliver a notification wrapping `activity` to the inbox `find` looks up, in the background.
    /// The delivery takes its place in the queue before the lookup starts, so lookups are bounded
    /// as deliveries are: past the queue's limit the notification is dropped, lookup and all.
    pub fn deliver_found<S, F>(&self, state: &LwsState<S>, activity: Value, find: F)
    where
        S: Store + 'static,
        F: std::future::Future<Output = Option<String>> + Send + 'static,
    {
        let Some(admitted) = self.admit_or_drop(&"an inbox still to be looked up") else {
            return;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let Some(inbox) = find.await else { return };
            let Some(target) = inbox_url(&inbox, state.cfg.allow_insecure_fetch) else {
                return;
            };
            state.notify.run(&state, admitted, target, activity, None);
        });
    }

    /// A place in the delivery queue, or `None` (the drop counted and logged) when it is full.
    fn admit_or_drop(&self, target: &dyn std::fmt::Display) -> Option<Admitted> {
        let admitted = self.admit();
        if admitted.is_none() {
            self.note_dropped(1, target);
        }
        admitted
    }

    /// Count `n` notifications about or to `target` as dropped for want of queue room. Logged at
    /// the first drop and then each time the count passes a power of two, so a flood cannot flood
    /// the log too.
    fn note_dropped(&self, n: usize, target: &dyn std::fmt::Display) {
        let before = self.dropped.fetch_add(n as u64, Ordering::SeqCst);
        let after = before + n as u64;
        if before == 0 || before.ilog2() != after.ilog2() {
            eprintln!(
                "lws: webhook delivery queue full ({} waiting or in flight); dropped {n} \
                 notification(s) for {target} ({after} dropped so far)",
                self.limits.queue
            );
        }
    }

    /// Run one admitted delivery in the background.
    fn run<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        admitted: Admitted,
        target: url::Url,
        activity: Value,
        watch: Option<Watch>,
    ) {
        let inbox_slot = self.inbox_slot(&target);
        let workers = self.workers.clone();
        let body = Bytes::from(envelope(&state.cfg.storage(), activity).to_string());
        let keyid = format!("{}#{}", state.cfg.storage(), state.cfg.notify_key.kid());
        let state = state.clone();
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
            // Waiting for the permits (and between retries) can take a while; the subscription and
            // the subscriber's access are checked again before every attempt, and work that no
            // longer stands is discarded.
            let Some(status) =
                attempt_delivery(&state, &target, &body, &keyid, watch.as_ref()).await
            else {
                return;
            };
            let Some(Watch {
                subscription: id, ..
            }) = watch
            else {
                return;
            };
            let notifier = &state.notify;
            // The count lives in the subscription: one cancelled while this delivery was out is
            // gone, and nothing is counted for it.
            let failures = {
                let mut subs = notifier.subs.write().expect("lock");
                let Some(sub) = subs.get_mut(&id) else {
                    return;
                };
                sub.failures = if status.is_some_and(|s| s.is_success()) {
                    0
                } else {
                    sub.failures + 1
                };
                sub.failures
            };
            if failures == 0 {
                return;
            }
            if status == Some(StatusCode::GONE) || failures >= MAX_DELIVERY_FAILURES {
                // A store failure keeps the subscription (and its failure count), so the next
                // failed delivery tries to deactivate it again.
                let _ = notifier.remove(&state, &id, None).await;
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
/// status, or `Some(None)` when the inbox could not be reached at all. `None` when, before an
/// attempt, the delivery no longer stands ([`Watch::stands`]: its subscription was cancelled or
/// expired, or its subscriber may no longer read the resource): nothing more is sent for it.
async fn attempt_delivery<S: Store + 'static>(
    state: &LwsState<S>,
    target: &url::Url,
    body: &Bytes,
    keyid: &str,
    watch: Option<&Watch>,
) -> Option<Option<StatusCode>> {
    let mut last = None;
    for attempt in 1..=DELIVERY_ATTEMPTS {
        if let Some(w) = watch {
            if !w.stands(state).await {
                return None;
            }
        }
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
    Some(last)
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
                // Expired subscriptions still hold their place in the store until they are
                // removed: a few go now, before the listing a conditional create is evaluated
                // against is held, and before this one is counted.
                purge_expired(state, req.admission.clone()).await;
                // A conditional create is evaluated against the listing.
                let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
                let held = match super::service_preconditions(state, req, &container, |get| {
                    listing(state, get, agent, owner)
                })
                .await
                {
                    Ok(held) => held,
                    Err(refused) => return refused,
                };
                subscribe(state, req, agent, held).await
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
            let refusal = super::resources::read_refusal(req, &etag);
            if refusal == Some(StatusCode::PRECONDITION_FAILED) {
                return problem(StatusCode::PRECONDITION_FAILED, None);
            }
            let mut resp = if let Some(status) = refusal {
                status.into_response()
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
        Method::DELETE => {
            let etag = etag_of([document(state, &sub).to_string().as_str()]);
            if let Some(refused) = super::resources::unless_preconditions(req, Some(&etag), None) {
                return refused;
            }
            match state.notify.remove(state, id, req.admission.clone()).await {
                Ok(()) => problem(StatusCode::NO_CONTENT, None),
                Err(e) => problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(&format!("cannot cancel the subscription: {e}")),
                ),
            }
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

/// Remove a few expired subscriptions ([`EXPIRED_REMOVED_PER_SUBSCRIBE`]), as a new one is
/// requested: never while the request holds the listing alone, since a removal waits for it.
async fn purge_expired<S: Store + 'static>(
    state: &LwsState<S>,
    admission: Option<crate::overload::AdmissionSlot>,
) {
    let now = jose::now_secs();
    // Taken in turn, from after the last one tried round to it: one that cannot be removed is
    // tried again only after every other expired subscription was.
    let expired: Vec<String> = {
        use std::ops::Bound::{Excluded, Unbounded};
        let subs = state.notify.subs.read().expect("lock");
        let cursor = state.notify.purged_to.lock().expect("lock");
        let expired: Vec<String> = subs
            .range::<String, _>((Excluded(&*cursor), Unbounded))
            .chain(subs.range::<String, _>(..=&*cursor))
            .map(|(_, s)| s)
            .filter(|s| s.expired(now))
            .take(EXPIRED_REMOVED_PER_SUBSCRIBE)
            .map(|s| s.id.clone())
            .collect();
        drop(cursor);
        expired
    };
    // The cursor moves to each one as its removal is tried, not before: a request that goes away
    // part way leaves the ones it never tried first in line.
    for id in expired {
        state.notify.purged_to.lock().expect("lock").clone_from(&id);
        let _ = state.notify.remove(state, &id, admission.clone()).await;
    }
}

async fn subscribe<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    held: Option<super::resources::IriGuard>,
) -> Response {
    // "The request body MUST conform to the application/lws+json media type."
    if req.content_type().as_deref() != Some(LWS_JSON) {
        return problem(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Some("a subscription request is application/lws+json"),
        );
    }
    let body =
        match super::bounded_json(&req.body, MAX_SUBSCRIPTION_BYTES, "a subscription request") {
            Ok(b) => b,
            Err(r) => return r,
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
    // topic is REQUIRED: a non-empty array of URIs. Repeats count once; each topic is checked
    // against the store and the subscriber's access, and weighed at every change announced, so
    // a subscription names at most MAX_TOPICS.
    let mut topics: Vec<String> = match body.get("topic") {
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
    let mut seen = std::collections::HashSet::new();
    topics.retain(|t| seen.insert(t.clone()));
    if topics.len() > MAX_TOPICS {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            Some(&format!("a subscription names at most {MAX_TOPICS} topics")),
        );
    }
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
    // or naming nothing, is refused the same way. A resource's linkset is read under the
    // resource's authorization (as a GET of it is), so a topic naming one is checked as the
    // resource, under the resource's shared lock; the topic itself is kept as named.
    let storage = state.cfg.storage();
    for topic in &topics {
        let resource = topic.strip_suffix(META_SUFFIX).unwrap_or(topic);
        if let Some(unavailable) = super::resources::set_aside(state, resource, &Method::GET) {
            return unavailable;
        }
        let _guard = state.locks.read(resource).await;
        let exists =
            resource.starts_with(&storage) && state.store.exists(resource).await.unwrap_or(false);
        if !exists || !state.allowed(Action::Read, resource, agent).await {
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
    // The place is reserved before anything is stored and held until the subscription is
    // registered. The storage owner's subscriptions have a share of their own: they neither
    // count against everyone else's nor are held to one subscriber's. Expired subscriptions take
    // room in the store until they are removed, but none of their subscriber's share: it can
    // neither see nor cancel them.
    let now = jose::now_secs();
    let owner = access::is_owner(state, agent);
    let (quota, what) = if owner {
        (&state.notify.owner_quota, "owner subscriptions")
    } else {
        (&state.notify.quota, "subscriptions")
    };
    let reserved = quota.reserve(agent.subject.as_deref(), || {
        let subs = state.notify.subs.read().expect("lock");
        let owners = state.cfg.owner.as_deref();
        let in_share = subs
            .values()
            .filter(|s| (owners.is_some() && s.subscriber.as_deref() == owners) == owner)
            .count();
        let mine = subs
            .values()
            .filter(|s| s.subscriber == agent.subject && !s.expired(now))
            .count();
        (in_share, mine)
    });
    let slot = match reserved {
        Ok(slot) => slot,
        Err(full) => return full.response(what),
    };
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
        failures: 0,
    };
    let stored = serde_json::to_vec(&sub).unwrap_or_default();
    let doc = document(state, &sub);
    let register = {
        let state = state.clone();
        move || {
            state.notify.subs.write().expect("lock").insert(id, sub);
            *state.notify.etag.write().expect("lock") = new_etag();
            // Counted as registered from here on.
            drop(slot);
        }
    };
    let created = super::create_record(
        state,
        &container,
        &iri,
        Bytes::from(stored),
        req.admission.clone(),
        held,
        false,
        register,
    );
    if let Err(e) = created.await {
        return problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    }
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

    /// Review finding: at startup an expired subscription whose removal failed was dropped from
    /// memory though it stayed stored, and one that could not be read was skipped: both escaped
    /// the subscription quota. The first stays loaded (and counted) until it can be removed; the
    /// second stops the load.
    #[tokio::test]
    async fn stored_subscriptions_are_all_accounted_for_at_startup() {
        let store = test_store::FlakyStore::new();
        let cfg = LwsConfig::new("http://localhost:3000");
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        Notifier::load(&store, &cfg).await.unwrap();
        let sub = Subscription {
            id: String::new(),
            subscriber: None,
            client: None,
            topics: vec![cfg.storage()],
            inbox: "https://a/inbox".into(),
            expires: None,
            expires_at: Some(1),
            failures: 0,
        };
        let iri = format!("{container}old");
        let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
        store
            .create_in_container(&container, &iri, body, LWS_JSON)
            .await
            .unwrap();
        *store.fail_delete_of.lock().unwrap() = Some(iri.clone());
        let loaded = Notifier::load(&store, &cfg).await.unwrap();
        assert_eq!(loaded.subs.read().unwrap().len(), 1, "kept while stored");
        *store.fail_delete_of.lock().unwrap() = None;
        let loaded = Notifier::load(&store, &cfg).await.unwrap();
        assert!(loaded.subs.read().unwrap().is_empty(), "purged");
        assert!(!store.exists(&iri).await.unwrap());
        let unreadable = format!("{container}unreadable");
        store
            .create_in_container(&container, &unreadable, Bytes::from("{}"), LWS_JSON)
            .await
            .unwrap();
        assert!(Notifier::load(&store, &cfg).await.is_err());
    }

    /// Review finding: anyone who could read a resource could store subscriptions without limit,
    /// filling the store resources need, and a large one was parsed before any limit applied.
    /// Subscriptions are held to a share of their own, reserved before anything is stored, and
    /// a request body is held to its size before it is parsed.
    #[tokio::test]
    async fn subscriptions_are_held_to_their_share() {
        let (state, _store) = test_store::state(100).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |body: &str| {
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON)],
                body,
            )
        };
        let reader = Agent {
            subject: Some("https://reader.example/#me".into()),
            ..Agent::anonymous()
        };
        let req = post(&body);
        let all = (0..2 * MAX_SUBSCRIPTIONS_PER_SUBSCRIBER).map(|_| handle(&state, &req, &reader));
        let created = futures_util::future::join_all(all)
            .await
            .iter()
            .filter(|r| r.status() == StatusCode::CREATED)
            .count();
        assert_eq!(created, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER);
        let r = handle(&state, &post(&body), &reader).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        let r = handle(
            &state,
            &post(&"[".repeat(MAX_SUBSCRIPTION_BYTES + 1)),
            &reader,
        )
        .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        // An expired subscription gives its place back once a new one is asked for.
        let mine = state
            .notify
            .subs
            .read()
            .unwrap()
            .keys()
            .next()
            .cloned()
            .unwrap();
        state
            .notify
            .subs
            .write()
            .unwrap()
            .get_mut(&mine)
            .unwrap()
            .expires_at = Some(1);
        let r = handle(&state, &post(&body), &reader).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert!(!state.notify.subs.read().unwrap().contains_key(&mine));
    }

    /// Review finding: expired subscriptions kept their subscriber's places, and a conditional
    /// create, holding the listing, removed none, so a subscriber whose subscriptions had all
    /// expired could never renew one conditionally; and an expiry changed the listing without
    /// changing its entity tag. Expired subscriptions take none of their subscriber's share and
    /// are removed before the listing is held, and the listing's tag covers what it shows.
    #[tokio::test]
    async fn expired_subscriptions_neither_block_renewal_nor_keep_a_stale_tag() {
        let (state, _store) = test_store::state(4).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |headers: &[(&str, &str)]| {
            let mut h = vec![("content-type", LWS_JSON)];
            h.extend_from_slice(headers);
            test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body)
        };
        let get = |headers: &[(&str, &str)]| {
            test_store::request(Method::GET, SUBSCRIPTIONS_PATH, headers, "")
        };
        let reader = Agent {
            subject: Some("https://reader.example/#me".into()),
            ..Agent::anonymous()
        };
        let expire_all = || {
            for s in state.notify.subs.write().unwrap().values_mut() {
                s.expires_at = Some(1);
            }
        };
        for _ in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            let r = handle(&state, &post(&[]), &reader).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        expire_all();
        let listed = handle(&state, &get(&[]), &reader).await;
        let tag = listed.headers()["etag"].to_str().unwrap().to_string();
        let r = handle(&state, &post(&[("if-match", &tag)]), &reader).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        // Five live subscriptions, four to a page; the fifth expires, and the first page's tag
        // changes with the count it shows.
        for _ in 0..4 {
            let r = handle(&state, &post(&[]), &reader).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let listed = handle(&state, &get(&[]), &reader).await;
        let tag = listed.headers()["etag"].to_str().unwrap().to_string();
        let r = handle(&state, &get(&[("if-none-match", &tag)]), &reader).await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        {
            let mut subs = state.notify.subs.write().unwrap();
            // The last listed, on the second page: the first page lists the same members.
            let live = subs
                .values_mut()
                .filter(|s| s.expires_at.is_none())
                .last()
                .unwrap();
            live.expires_at = Some(1);
        }
        let r = handle(&state, &get(&[("if-none-match", &tag)]), &reader).await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    /// Review finding: a purge always tried the first expired subscriptions, so sixteen that could
    /// not be removed kept every later one from being tried. Purges take them in turn.
    #[tokio::test]
    async fn every_expired_subscription_gets_its_turn() {
        let (state, store) = test_store::state(4).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let n = EXPIRED_REMOVED_PER_SUBSCRIBE + 4;
        for i in 0..n {
            let who = Agent {
                subject: Some(format!("https://r{i}.example/#me")),
                ..Agent::anonymous()
            };
            let h = [("content-type", LWS_JSON)];
            let r = handle(
                &state,
                &test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body),
                &who,
            )
            .await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let ids: Vec<String> = {
            let mut subs = state.notify.subs.write().unwrap();
            for s in subs.values_mut() {
                s.expires_at = Some(1);
            }
            subs.keys().cloned().collect()
        };
        // The first sixteen cannot be removed now.
        store
            .fail_delete
            .store(true, std::sync::atomic::Ordering::SeqCst);
        purge_expired(&state, None).await;
        store
            .fail_delete
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(state.notify.subs.read().unwrap().len(), n);
        // The next purge starts with the ones after them.
        purge_expired(&state, None).await;
        let left = state.notify.subs.read().unwrap();
        for id in &ids[EXPIRED_REMOVED_PER_SUBSCRIBE..] {
            assert!(!left.contains_key(id), "{id} never got its turn");
        }
    }

    /// Review finding: the owner's subscriptions were not held to a share, but counted against
    /// everyone else's, so an owner with many left no room for anyone. Each has a share of its
    /// own.
    #[tokio::test]
    async fn owner_subscriptions_have_their_own_share() {
        let owner = "https://owner.example/#me";
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.owner = Some(owner.into());
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        {
            let mut subs = state.notify.subs.write().unwrap();
            for i in 0..MAX_SUBSCRIPTIONS {
                let id = format!("o{i}");
                subs.insert(
                    id.clone(),
                    Subscription {
                        id,
                        subscriber: Some(owner.into()),
                        client: None,
                        topics: vec![state.cfg.storage()],
                        inbox: "https://inbox.example/".into(),
                        expires: None,
                        expires_at: None,
                        failures: 0,
                    },
                );
            }
        }
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = test_store::request(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let reader = Agent {
            subject: Some("https://reader.example/#me".into()),
            ..Agent::anonymous()
        };
        assert_eq!(
            handle(&state, &post, &reader).await.status(),
            StatusCode::CREATED
        );
        let owner = Agent {
            subject: Some(owner.into()),
            ..Agent::anonymous()
        };
        // The owner's own share is full.
        assert_ne!(
            handle(&state, &post, &owner).await.status(),
            StatusCode::CREATED
        );
    }

    /// Review finding: a subscription was cancelled whatever the request's `If-Match` named.
    #[tokio::test]
    async fn a_cancellation_evaluates_its_preconditions() {
        let (state, _store) = test_store::state(100).await;
        let path = subscribe_root(&state, "s1").await;
        let get = test_store::request(Method::GET, &path, &[], "");
        let r = handle(&state, &get, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::OK);
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let delete =
            |tag: &str| test_store::request(Method::DELETE, &path, &[("if-match", tag)], "");
        let r = handle(&state, &delete("\"other\""), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        assert!(state.notify.get("s1").is_some());
        // A read is held to If-Match as well.
        let other = test_store::request(Method::GET, &path, &[("if-match", "\"other\"")], "");
        let r = handle(&state, &other, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        let r = handle(&state, &delete(&tag), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(state.notify.get("s1").is_none());
    }

    /// Review finding: a recursive delete prepared every notification for every descendant
    /// before removing anything, past what the delivery queue would ever take. Preparation shares
    /// the queue's bound.
    #[tokio::test]
    async fn preparation_is_bounded_by_the_delivery_queue() {
        let (state, _store) = test_store::state(100).await;
        for i in 0..10 {
            let sub = Subscription {
                id: format!("s{i}"),
                subscriber: None,
                client: None,
                topics: vec![state.cfg.storage()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state
                .notify
                .subs
                .write()
                .unwrap()
                .insert(sub.id.clone(), sub);
        }
        let event = Event {
            kind: "Update",
            is_container: true,
            uri: state.cfg.storage(),
            relation: None,
        };
        assert_eq!(
            state.notify.prepare_at_most(&state, &event, 3).await.len(),
            3
        );
        assert_eq!(state.notify.prepare(&state, &event).await.len(), 10);
    }

    /// Sweep finding: a subscription could name any number of topics, each checked against the
    /// store and the subscriber's access, and weighed at every change announced. Repeats count
    /// once, and more than [`MAX_TOPICS`] distinct ones are refused.
    #[tokio::test]
    async fn subscriptions_name_a_bounded_number_of_topics() {
        let (state, _store) = test_store::state(100).await;
        let body = |topics: Vec<String>| {
            json!({
                "@context": ["https://www.w3.org/ns/lws/v1"],
                "type": WEBHOOK,
                "topic": topics,
                "inbox": "https://inbox.example/",
            })
            .to_string()
        };
        let post = |b: String| {
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON)],
                &b,
            )
        };
        let many = (0..=MAX_TOPICS)
            .map(|i| state.cfg.absolute(&format!("/t{i}")))
            .collect();
        let r = handle(&state, &post(body(many)), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let repeated = vec![state.cfg.storage(); 10 * MAX_TOPICS];
        let r = handle(&state, &post(body(repeated)), &Agent::anonymous()).await;
        assert_ne!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Review finding: a subscription's cancellation ran in a task of its own that held no
    /// admission permit, so once its request timed out a stalled removal no longer counted
    /// against the concurrency ceiling, and repeated cancellations could pile up past it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancellation_keeps_its_admission_slot() {
        use crate::app::{with_overload_layers, OverloadConfig};
        use axum::body::Body;
        use std::sync::Arc;
        use std::time::Duration;
        use tower::ServiceExt;
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let store = test_store::FlakyStore::new();
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}s1");
        let sub = Subscription {
            id: "s1".into(),
            subscriber: None,
            client: None,
            topics: vec![cfg.storage()],
            inbox: "https://inbox.example/".into(),
            expires: None,
            expires_at: None,
            failures: 0,
        };
        // A first boot makes the containers; the subscription is stored after it and before
        // the boot under test, which loads it.
        let boot = |cfg: LwsConfig| super::super::router(store.clone(), cfg);
        let _ = boot(cfg.clone()).await.expect("router");
        store
            .create_in_container(
                &container,
                &iri,
                Bytes::from(serde_json::to_vec(&sub).unwrap()),
                LWS_JSON,
            )
            .await
            .unwrap();
        let app = boot(cfg).await.expect("router");
        let app = with_overload_layers(
            app,
            OverloadConfig::new(1, Some(Duration::from_millis(100))),
        );
        let send = |method: &str, path: &str| {
            let req = axum::http::Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap();
            app.clone().oneshot(req)
        };
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        let path = format!("{SUBSCRIPTIONS_PATH}s1");
        let r = send("DELETE", &path).await.unwrap();
        assert_eq!(r.status(), StatusCode::GATEWAY_TIMEOUT);
        // The removal still holds the only slot: the next request is shed.
        let r = send("GET", "/").await.unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        gate.add_permits(1);
        let mut status = StatusCode::SERVICE_UNAVAILABLE;
        for _ in 0..200 {
            status = send("GET", "/").await.unwrap().status();
            if status != StatusCode::SERVICE_UNAVAILABLE {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(status, StatusCode::OK);
    }

    /// A subscription to the storage root, put straight into the state and the store.
    /// Review finding: a subscription to a resource's linkset was accepted and never heard
    /// anything, since changes were announced only for the resource. The linkset's subscribers
    /// hear of them as of the linkset, checked as the resource it describes.
    #[tokio::test]
    async fn a_linkset_subscription_hears_of_its_resource() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let root = state.cfg.storage();
        let file = format!("{root}file");
        for (id, topic) in [
            ("linkset", format!("{file}{META_SUFFIX}")),
            ("other", format!("{root}other{META_SUFFIX}")),
            ("root", root.clone()),
        ] {
            let sub = Subscription {
                id: id.into(),
                subscriber: None,
                client: None,
                topics: vec![topic],
                inbox: format!("https://inbox.example/{id}"),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        for kind in ["Create", "Update", "Delete"] {
            let event = Event {
                kind,
                uri: file.clone(),
                is_container: false,
                relation: None,
            };
            let mut heard: Vec<(String, String, String)> = state
                .notify
                .prepare(&state, &event)
                .await
                .into_iter()
                .map(|p| {
                    (
                        p.inbox,
                        p.activity["object"]["id"].as_str().unwrap().to_string(),
                        p.watch.uri,
                    )
                })
                .collect();
            heard.sort();
            assert_eq!(
                heard,
                vec![
                    (
                        "https://inbox.example/linkset".into(),
                        format!("{file}{META_SUFFIX}"),
                        file.clone()
                    ),
                    (
                        "https://inbox.example/root".into(),
                        file.clone(),
                        file.clone()
                    ),
                ],
                "{kind}"
            );
        }
    }

    /// Review findings: the delivery bound counted subscriptions, not what they brought (one
    /// naming a resource and its linkset brings two), so a recursive delete's shared bound
    /// could underflow; and a delivery waiting for a resource's lock while the resource was set
    /// aside waited until it was put back, holding a worker.
    #[tokio::test]
    async fn deliveries_are_bounded_and_never_wait_on_a_set_aside_resource() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let file = format!("{}file", state.cfg.storage());
        let sub = Subscription {
            id: "both".into(),
            subscriber: None,
            client: None,
            topics: vec![file.clone(), format!("{file}{META_SUFFIX}")],
            inbox: "https://inbox.example/both".into(),
            expires: None,
            expires_at: None,
            failures: 0,
        };
        state
            .notify
            .subs
            .write()
            .unwrap()
            .insert("both".into(), sub);
        let event = Event {
            kind: "Update",
            uri: file.clone(),
            is_container: false,
            relation: None,
        };
        let prepared = state.notify.prepare_at_most(&state, &event, 1).await;
        assert_eq!(prepared.len(), 1);
        let watch = prepared.into_iter().next().unwrap().watch;
        // A write holds the resource; the delivery waits for it, and the write's rollback is
        // then set aside with the lock.
        let held = state.locks.lock(&file).await;
        let check = tokio::spawn({
            let state = state.clone();
            async move { watch.stands(&state).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let left = super::super::Unsettled(vec![super::super::Undo::Remove {
            iri: file.clone(),
            parent: state.cfg.storage(),
        }]);
        // Putting it back keeps failing.
        let failing = |on: bool| {
            use std::sync::atomic::Ordering::SeqCst;
            state.store.fail_delete.store(on, SeqCst);
            state.store.fail_exists.store(on, SeqCst);
        };
        failing(true);
        state.set_aside(left, held);
        let stands = tokio::time::timeout(std::time::Duration::from_secs(1), check)
            .await
            .expect("the delivery gave up rather than wait")
            .unwrap();
        assert!(!stands);
        failing(false);
    }

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
            failures: 0,
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

    /// Review finding: a delivery that failed without a retry after its subscription was
    /// cancelled recreated the subscription's failure count, which nothing removed again. The
    /// count lives in the subscription, so a late delivery finds nothing to count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_late_failure_leaves_nothing_of_a_cancelled_subscription() {
        use std::sync::atomic::Ordering;
        let app = axum::Router::new().route(
            "/inbox",
            axum::routing::post(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                StatusCode::BAD_REQUEST
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        subscribe_root(&state, "gone").await;
        subscribe_root(&state, "kept").await;
        for id in ["gone", "kept"] {
            let watch = Watch {
                subscription: id.into(),
                uri: state.cfg.storage(),
                agent: Agent::anonymous(),
                snapshot: None,
            };
            let inbox = format!("http://{addr}/inbox");
            state
                .notify
                .deliver(&state, &inbox, json!({"type": ["Update"]}), Some(watch));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let path = format!("{SUBSCRIPTIONS_PATH}gone");
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        for _ in 0..100 {
            if state.notify.admitted.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(state.notify.admitted.load(Ordering::SeqCst), 0);
        let subs = state.notify.subs.read().unwrap();
        assert!(!subs.contains_key("gone"));
        // The one still standing counted its failure.
        assert_eq!(subs["kept"].failures, 1);
    }

    /// Review finding: a delivery waiting for its inbox's turn or a worker, and a retry, never
    /// looked at the subscription again, so one cancelled (or expired) meanwhile still heard of
    /// the change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_subscriptions_hear_nothing_more() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let total = Arc::new(AtomicUsize::new(0));
        let app = {
            let total = total.clone();
            axum::Router::new().route(
                "/inbox",
                axum::routing::post(move || {
                    let total = total.clone();
                    async move {
                        total.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        StatusCode::SERVICE_UNAVAILABLE
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
            queue: 8,
            workers: 4,
            per_inbox: 1,
        };
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let inbox = format!("http://{addr}/inbox");
        for id in ["gone", "expired"] {
            subscribe_root(&state, id).await;
            // The first is in flight (and will fail, so it would be retried); the second waits
            // for the inbox's turn.
            for _ in 0..2 {
                let activity = json!({"type": ["Update"]});
                let watch = Watch {
                    subscription: id.into(),
                    uri: state.cfg.storage(),
                    agent: Agent::anonymous(),
                    snapshot: None,
                };
                state.notify.deliver(&state, &inbox, activity, Some(watch));
            }
            for _ in 0..100 {
                if total.load(Ordering::SeqCst) == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(total.load(Ordering::SeqCst), 1, "{id}");
            if id == "gone" {
                let path = format!("{SUBSCRIPTIONS_PATH}{id}");
                let delete = test_store::request(Method::DELETE, &path, &[], "");
                let resp = handle(&state, &delete, &Agent::anonymous()).await;
                assert_eq!(resp.status(), StatusCode::NO_CONTENT);
            } else {
                let mut subs = state.notify.subs.write().unwrap();
                subs.get_mut(id).unwrap().expires_at = Some(jose::now_secs() - 1);
            }
            // Past the retry delay: neither the retry nor the queued delivery was sent.
            tokio::time::sleep(RETRY_DELAY + Duration::from_millis(700)).await;
            assert_eq!(total.load(Ordering::SeqCst), 1, "{id}");
            for _ in 0..100 {
                if state.notify.admitted.load(Ordering::SeqCst) == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert_eq!(state.notify.admitted.load(Ordering::SeqCst), 0, "{id}");
            total.store(0, Ordering::SeqCst);
        }
    }

    /// Review finding: read access was checked when a notification was prepared, never again;
    /// a grant revoked (or expired) meanwhile still let the queued delivery and its retries go
    /// out. A Delete, delivered once the resource is gone, is checked against the resource as it
    /// was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deliveries_stop_once_read_access_is_gone() {
        use super::super::access::Policy;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let total = Arc::new(AtomicUsize::new(0));
        let accept = Arc::new(AtomicBool::new(false));
        let app = {
            let (total, accept) = (total.clone(), accept.clone());
            axum::Router::new().route(
                "/inbox",
                axum::routing::post(move || {
                    let (total, accept) = (total.clone(), accept.clone());
                    async move {
                        total.fetch_add(1, Ordering::SeqCst);
                        if accept.load(Ordering::SeqCst) {
                            return StatusCode::NO_CONTENT;
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        cfg.allow_insecure_fetch = true;
        cfg.delivery = DeliveryLimits {
            queue: 8,
            workers: 4,
            per_inbox: 1,
        };
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let bob = "https://bob.example/#me";
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let sub = Subscription {
            id: "s".into(),
            subscriber: Some(bob.into()),
            client: None,
            topics: vec![state.cfg.storage()],
            inbox: format!("http://{addr}/inbox"),
            expires: None,
            expires_at: None,
            failures: 0,
        };
        state
            .store
            .create_in_container(
                &container,
                &format!("{container}s"),
                Bytes::from(serde_json::to_vec(&sub).unwrap()),
                LWS_JSON,
            )
            .await
            .unwrap();
        state.notify.subs.write().unwrap().insert("s".into(), sub);
        let r = state.cfg.absolute("/r");
        state
            .store
            .write(&r, Bytes::from("r"), "text/plain")
            .await
            .unwrap();
        state.access.put_grant_for_test(
            "g",
            vec![Policy {
                actions: vec![Action::Read],
                assignee: bob.into(),
                target: None,
                storage: state.cfg.storage().into(),
                constraints: Vec::new(),
            }],
        );
        let update = Event {
            kind: "Update",
            uri: r.clone(),
            is_container: false,
            relation: None,
        };
        // Two notifications: the first in flight (failing, so it would be retried), the second
        // waiting for the inbox's turn.
        for _ in 0..2 {
            let pending = state.notify.prepare(&state, &update).await;
            assert_eq!(pending.len(), 1);
            state.notify.send(&state, pending);
        }
        for _ in 0..100 {
            if total.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(total.load(Ordering::SeqCst), 1);
        // The grant is revoked: neither the retry nor the queued delivery goes out.
        state.access.remove_grant_for_test("g");
        tokio::time::sleep(RETRY_DELAY + Duration::from_millis(700)).await;
        assert_eq!(total.load(Ordering::SeqCst), 1);
        for _ in 0..100 {
            if state.notify.admitted.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // A Delete of Bob's own resource reaches him once it is gone: he was its creator.
        total.store(0, Ordering::SeqCst);
        accept.store(true, Ordering::SeqCst);
        let d = state.cfg.absolute("/d");
        state
            .store
            .write(&d, Bytes::from("d"), "text/plain")
            .await
            .unwrap();
        let meta = super::super::ResourceMeta {
            creator: Some(bob.into()),
            ..Default::default()
        };
        state.put_resource_meta(&d, &meta).await.unwrap();
        let delete = Event {
            kind: "Delete",
            uri: d.clone(),
            is_container: false,
            relation: None,
        };
        let pending = state.notify.prepare(&state, &delete).await;
        assert_eq!(pending.len(), 1);
        state.store.delete(&d, None).await.unwrap();
        state
            .store
            .delete(&super::super::meta_key(&d), None)
            .await
            .unwrap();
        state.notify.send(&state, pending);
        for _ in 0..200 {
            if total.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(total.load(Ordering::SeqCst), 1);
    }

    /// Review finding: a cancellation removed the stored subscription, then waited on the
    /// cleanup of its bytes; when that failed the subscription stayed live in memory. The index
    /// commit is now the point of cancellation.
    #[tokio::test]
    async fn a_cancellation_takes_effect_once_the_record_is_gone() {
        let (state, store) = test_store::state(100).await;
        let path = subscribe_root(&state, "s1").await;
        let iri = state.cfg.absolute(&path);
        *store.fail_after_delete_of.lock().unwrap() = Some(iri.clone());
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!state.store.exists(&iri).await.unwrap());
        assert!(state.notify.get("s1").is_none());
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

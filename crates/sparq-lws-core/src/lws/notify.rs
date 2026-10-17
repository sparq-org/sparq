//! Notifications (LWS 1.0 core section 10) and the webhook subscription type
//! (lws10-notifications-webhook).
//!
//! The NotificationService lives at [`SUBSCRIPTIONS_PATH`] and offers
//! one subscription type, `WebhookSubscription`. An authenticated agent subscribes with an
//! `application/lws+json` request naming a `topic` (an array of resource URIs, each of which it
//! must be able to read now) and an `inbox`; a topic covers itself and, for a container,
//! everything beneath it. Each Create, Update and Delete inside a topic is then POSTed to the
//! inbox as an `application/lws+json` Notification, provided the subscriber may read the
//! resource when the event occurs (section 10.3.3).
//!
//! Deliveries leave the request's task (`tokio::spawn`) before any of them takes a place in the
//! delivery queue, so a change waits neither on how slow an inbox is nor on the per-delivery
//! work of however many subscriptions it has; who hears of the change is still decided on the
//! request's task, while the change's locks are held. They are signed per RFC 9421 with the key the
//! storage description publishes (covering `@method`, `@scheme`, `@authority`, `@path`,
//! `content-type` and an RFC 9530 `content-digest`), are tried again on a 5xx or an unreachable
//! inbox, and a subscription whose inbox answers 410 Gone, or fails repeatedly, is deactivated
//! (removed). Unless `allow_insecure_fetch` is set, inboxes must be `https:` and resolve to public
//! addresses only (checked when subscribing and again by the delivery client's resolver), and
//! deliveries never follow redirects or go through a proxy.
//!
//! Subscriptions are stored through the [`Store`] as members of the service container, so they
//! survive a restart on a durable backend, and kept in memory for event matching, by id and by
//! subscriber: a subscriber's listing, and the count of its own capacity, read its own records
//! and no one else's. That index is built as the records are loaded, and what is stored is the
//! same either way. The store they share is finite, so how many subscriptions stand in all is
//! bounded too ([`MAX_SUBSCRIPTIONS`]); a create at that bound is refused `507`, whoever filled
//! it, as an access request at its own total is. Their
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

/// The components every delivery signature covers: those lws10-notifications-webhook
/// ("Signature Requirements") requires, and `@query`, so a notification signed for an inbox with
/// a query (`/inbox?account=alice`) is not valid for the same path with another (RFC 9421
/// section 7.2.1).
const SIGNATURE_COMPONENTS: [&str; 7] = [
    "@method",
    "@scheme",
    "@authority",
    "@path",
    "@query",
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

/// The notifications one change makes, addressed but not yet sent ([`Notifier::prepare`]), and
/// what preparing them left out. What was left out is counted ([`Notifier::dropped`],
/// [`Notifier::unprepared`]) only when they are [`Notifier::send`]t: a change that does not
/// happen (a delete that fails) sends nothing and counts nothing.
#[derive(Debug, Default)]
#[must_use = "prepared notifications go out, and are counted, only through Notifier::send"]
pub struct Prepared {
    pending: Vec<Pending>,
    /// The resource the change is to.
    uri: String,
    /// The bound the deliveries were prepared to.
    limit: usize,
    /// Authorized deliveries past the bound: owed, and dropped.
    dropped: usize,
    /// Subscriptions past the bound: neither authorized nor notified.
    unprepared: usize,
}

impl Prepared {
    /// The notifications prepared.
    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    /// How many notifications were prepared.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether none was.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
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
    /// For a service record (an access grant or request, a subscription), its author: such a
    /// record is read by the storage owner and its author only, never through the resource
    /// rules.
    service: Option<Option<String>>,
}

/// Whether `agent` may read a service record (an access grant or request, a subscription) whose
/// author is `author`: the storage owner and the author only, as its own GET decides.
fn reads_service_record<S: Store + 'static>(
    state: &LwsState<S>,
    agent: &Agent,
    author: &Option<String>,
) -> bool {
    state.cfg.open
        || access::is_owner(state, agent)
        || (agent.subject.is_some() && agent.subject == *author)
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
        if let Some(author) = &self.service {
            return reads_service_record(state, &self.agent, author);
        }
        match &self.snapshot {
            Some(s) => access::allowed_as(state, Action::Read, &self.uri, &self.agent, s).await,
            None => {
                // A resource that is not visible is not read (its lock is held until it is put
                // back): the delivery fails its check rather than wait, including when the
                // resource is set aside while the delivery waits for its lock.
                let Some(_guard) = state.locks.read(&self.uri).await else {
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

/// Most subscriptions the storage owner may hold, expired ones included until they are removed.
/// The owner is one principal, so this is its own capacity and no one else's: what fills it is
/// the owner's own requests, and the subscriptions held back or quarantined at the start, which
/// may be anyone's and are counted here because nothing says whose they are.
pub const MAX_OWNER_SUBSCRIPTIONS: usize = 512;

/// Most subscriptions held at once by subscribers other than the storage owner, expired ones
/// included until they are removed: a bound on what the store holds, not a capacity any one
/// subscriber is given. Each subscription is stored as a resource is, in the same store as the
/// rest of the storage, and that store is finite; without this bound the first refusal would
/// come from the store itself, which the rest of the storage shares.
/// So a create is refused `507` once this many stand, whoever made them — a resource bound, as
/// the total on access requests is. One subscriber cannot reach it on its own: each is held to
/// [`MAX_SUBSCRIPTIONS_PER_SUBSCRIBER`], so it takes many subscribers, and `507` here says the
/// storage is full of subscriptions, not that the requester asked for too many (`429` says
/// that). The owner's subscriptions are counted apart, against
/// [`MAX_OWNER_SUBSCRIPTIONS`].
pub const MAX_SUBSCRIPTIONS: usize = 512;

/// Most subscriptions one subscriber other than the storage owner may hold, expired ones
/// excluded (an expired one can neither be seen nor cancelled by its subscriber, so it takes
/// none of this capacity). Every subscriber has this capacity in full: how many subscriptions
/// anyone else holds never makes a subscriber's own share smaller. What all of them hold
/// together is bounded apart, by [`MAX_SUBSCRIPTIONS`].
pub const MAX_SUBSCRIPTIONS_PER_SUBSCRIBER: usize = 16;

/// Largest subscription request body, in bytes.
pub const MAX_SUBSCRIPTION_BYTES: usize = 16 * 1024;

/// The store holds these records in a pool of its own, apart from stored content, so that pool
/// must have room for every subscription this service admits, each as large as a request may
/// leave it. The store spells the service's path itself, its module being built where this one
/// is not; the test `the_store_pools_what_the_notification_service_bounds` holds the two
/// spellings to each other.
const _: () = {
    assert!(
        crate::store::DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT
            > MAX_SUBSCRIPTIONS + MAX_OWNER_SUBSCRIPTIONS
    );
    assert!(
        crate::store::DEFAULT_IN_MEMORY_MAX_POOL_BYTES
            >= crate::store::DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT * MAX_SUBSCRIPTION_BYTES
    );
};

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

/// The subscriptions in force, held by id and by subscriber.
///
/// Both indexes live under one lock, so the ids recorded for a subscriber and the subscriptions
/// themselves can never disagree. The subscriber index is what a subscriber's listing and the
/// count of its own capacity read ([`Subs::live_of`], [`Subs::live_count_of`]): each reads that
/// subscriber's records and no one else's, so neither its answer nor the work it takes grows
/// with how many subscriptions other subscribers hold.
///
/// The subscriber a record is indexed under is set when it is inserted and never changed:
/// [`Subs::get_mut`] is for the delivery failure count, which does change in place.
#[derive(Default)]
struct Subs {
    by_id: BTreeMap<String, Subscription>,
    by_subscriber: BTreeMap<Option<String>, std::collections::BTreeSet<String>>,
}

impl Subs {
    /// Hold `sub` under `id`, giving back what `id` held before.
    fn insert(&mut self, id: String, sub: Subscription) -> Option<Subscription> {
        let subscriber = sub.subscriber.clone();
        let replaced = self.by_id.insert(id.clone(), sub);
        if let Some(old) = &replaced {
            self.unindex(&id, &old.subscriber);
        }
        self.by_subscriber.entry(subscriber).or_default().insert(id);
        replaced
    }

    fn remove(&mut self, id: &str) -> Option<Subscription> {
        let gone = self.by_id.remove(id)?;
        self.unindex(id, &gone.subscriber);
        Some(gone)
    }

    /// Drop `id` from `subscriber`'s ids, and the subscriber itself once it holds none. Every
    /// change to `by_id` goes through [`Subs::insert`] or [`Subs::remove`], which keep the index
    /// in step, so a subscriber's entry holds exactly its subscriptions ([`Subs::count_of`] is
    /// its size).
    fn unindex(&mut self, id: &str, subscriber: &Option<String>) {
        if let Some(ids) = self.by_subscriber.get_mut(subscriber) {
            ids.remove(id);
            if ids.is_empty() {
                self.by_subscriber.remove(subscriber);
            }
        }
    }

    fn get(&self, id: &str) -> Option<&Subscription> {
        self.by_id.get(id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Subscription> {
        self.by_id.get_mut(id)
    }

    fn contains_key(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }

    /// How many subscriptions are held in all, whoever holds them.
    fn len(&self) -> usize {
        self.by_id.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    #[cfg(test)]
    fn keys(&self) -> std::collections::btree_map::Keys<'_, String, Subscription> {
        self.by_id.keys()
    }

    fn values(&self) -> std::collections::btree_map::Values<'_, String, Subscription> {
        self.by_id.values()
    }

    #[cfg(test)]
    fn values_mut(&mut self) -> std::collections::btree_map::ValuesMut<'_, String, Subscription> {
        self.by_id.values_mut()
    }

    fn range<T, R>(&self, range: R) -> std::collections::btree_map::Range<'_, String, Subscription>
    where
        T: Ord + ?Sized,
        String: std::borrow::Borrow<T> + Ord,
        R: std::ops::RangeBounds<T>,
    {
        self.by_id.range(range)
    }

    /// `subscriber`'s own unexpired subscriptions, in id order, read through the subscriber
    /// index: no other subscriber's record is touched.
    fn live_of(&self, subscriber: Option<&str>, now: i64) -> Vec<Subscription> {
        self.mine(subscriber)
            .filter(|s| !s.expired(now))
            .cloned()
            .collect()
    }

    /// How many unexpired subscriptions `subscriber` holds, read as [`Subs::live_of`] reads.
    fn live_count_of(&self, subscriber: Option<&str>, now: i64) -> usize {
        self.mine(subscriber).filter(|s| !s.expired(now)).count()
    }

    /// How many subscriptions `subscriber` holds, expired ones included: the size of its index
    /// entry, so no record (its own or anyone else's) is read to count them.
    fn count_of(&self, subscriber: Option<&str>) -> usize {
        self.by_subscriber
            .get(&subscriber.map(str::to_string))
            .map_or(0, |ids| ids.len())
    }

    /// The ids of `subscriber`'s own expired subscriptions, at most `most` of them, read
    /// through the subscriber index: no other subscriber's record is touched.
    fn expired_of(&self, subscriber: Option<&str>, now: i64, most: usize) -> Vec<String> {
        self.mine(subscriber)
            .filter(|s| s.expired(now))
            .take(most)
            .map(|s| s.id.clone())
            .collect()
    }

    fn mine(&self, subscriber: Option<&str>) -> impl Iterator<Item = &Subscription> {
        self.by_subscriber
            .get(&subscriber.map(str::to_string))
            .into_iter()
            .flatten()
            .filter_map(|id| self.by_id.get(id))
    }
}

/// The subscriptions of the storage and the notification sender.
pub struct Notifier {
    subs: RwLock<Subs>,
    /// Entity tag of the subscription listing; changes with its membership.
    etag: RwLock<String>,
    /// The delivery client: no redirects, no proxy, and (unless insecure fetches are allowed) a
    /// resolver that refuses non-public addresses.
    client: reqwest::Client,
    limits: DeliveryLimits,
    /// Deliveries admitted and not yet finished.
    admitted: Arc<AtomicUsize>,
    /// Notifications dropped for want of queue room, each one known to be owed: authorized for
    /// its subscriber, or belonging to no subscription.
    dropped: AtomicU64,
    /// Subscriptions that heard of a change once its deliveries were already at their bound:
    /// neither authorized nor notified, so whether each was owed a notification is not known.
    unprepared: AtomicU64,
    /// Preparations that had a subscription to weigh, for tests to see whether one ran.
    #[cfg(test)]
    pub(crate) prepared_for: AtomicUsize,
    /// The worker pool: one permit per delivery in flight.
    workers: Arc<tokio::sync::Semaphore>,
    /// Per inbox origin, its own limit (dropped once no delivery holds it).
    inboxes: Mutex<HashMap<String, Weak<tokio::sync::Semaphore>>>,
    /// What subscribers other than the storage owner may hold (see [`super::Quota`]): each its
    /// own capacity of [`MAX_SUBSCRIPTIONS_PER_SUBSCRIBER`], and all of them together the
    /// store's bound of [`MAX_SUBSCRIPTIONS`].
    quota: super::Quota,
    /// The storage owner's own capacity, apart from everyone else's, as a total as well: the
    /// owner is one principal, so its own requests are what fill it, together with the records
    /// held back or quarantined at the start.
    owner_quota: super::Quota,
    /// The last subscription a sweep looked at: the next starts after it, so ones that cannot be
    /// removed do not keep the rest from being tried.
    purged_to: Mutex<String>,
    /// Wakes the expiry sweeper ([`wake_expiry_sweeper`]).
    sweep_now: Arc<tokio::sync::Notify>,
    /// Keeps the expiry sweeper from sweeping, for a test that sweeps itself.
    #[cfg(test)]
    sweeper_off: Arc<std::sync::atomic::AtomicBool>,
    /// Stored subscriptions the start did not load because a change to them was still being
    /// settled, or their removal failed. Each is counted against both shares until the next
    /// start, as it may still be stored.
    held_back: AtomicUsize,
    /// Stored subscriptions whose stored body is not a subscription, by id: quarantined at every
    /// start (what makes them so is what is stored). Each is listed to the owner, counted
    /// against both shares, answers `500` to a read, and goes when the owner deletes it.
    quarantined: RwLock<std::collections::BTreeSet<String>>,
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

    /// Ensure the service container exists and load the subscriptions it holds, all visible.
    #[cfg(test)]
    pub async fn load<S: Store>(store: &S, cfg: &LwsConfig) -> Result<Self, String> {
        let me = Self::new(cfg)?;
        me.load_with(store, cfg, |_| true).await?;
        Ok(me)
    }

    /// Holding no subscription.
    pub fn new(cfg: &LwsConfig) -> Result<Self, String> {
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
        Ok(Self {
            subs: RwLock::new(Subs::default()),
            etag: RwLock::new(new_etag()),
            client,
            limits: cfg.delivery,
            admitted: Arc::new(AtomicUsize::new(0)),
            dropped: AtomicU64::new(0),
            unprepared: AtomicU64::new(0),
            #[cfg(test)]
            prepared_for: AtomicUsize::new(0),
            workers: Arc::new(tokio::sync::Semaphore::new(cfg.delivery.workers.max(1))),
            inboxes: Mutex::new(HashMap::new()),
            quota: super::Quota::new(MAX_SUBSCRIPTIONS, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER),
            owner_quota: super::Quota::new(MAX_OWNER_SUBSCRIPTIONS, MAX_OWNER_SUBSCRIPTIONS),
            purged_to: Mutex::new(String::new()),
            sweep_now: Default::default(),
            #[cfg(test)]
            sweeper_off: Default::default(),
            held_back: AtomicUsize::new(0),
            quarantined: Default::default(),
        })
    }

    /// Ensure the service container exists and load the subscriptions it holds, skipping those
    /// not `visible` (a change to them, put back at boot, is still being settled: see the
    /// `intents` module), as access grants and requests are loaded: what the listing holds is in
    /// force. One that cannot be read
    /// stops the start (a later start reads it again); one whose stored body is not a
    /// subscription is quarantined ([`Notifier::quarantined`]).
    pub(crate) async fn load_with<S: Store>(
        &self,
        store: &S,
        cfg: &LwsConfig,
        visible: impl Fn(&str) -> bool,
    ) -> Result<(), String> {
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
        let mut subs = Subs::default();
        let mut held_back = 0;
        let mut quarantined = std::collections::BTreeSet::new();
        let now = jose::now_secs();
        for child in store
            .list_children(&container)
            .await
            .map_err(|e| format!("store: {e}"))?
        {
            if !visible(child.as_str()) {
                held_back += 1;
                continue;
            }
            let read = match store.read(child.as_str()).await {
                // Listed but gone: nothing of it is stored.
                Err(crate::error::ServerError::NotFound) => continue,
                // A read that fails says nothing of what is stored: the start stops rather than
                // decide without it.
                Err(e) => return Err(format!("store: {}: {e}", child.as_str())),
                Ok(r) => r,
            };
            let id = child
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                continue;
            }
            let Ok(mut sub) = serde_json::from_slice::<Subscription>(&read.body) else {
                eprintln!(
                    "lws: subscription {} is not a subscription; it is quarantined until deleted",
                    child.as_str()
                );
                quarantined.insert(id);
                continue;
            };
            sub.id = id;
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
        *self.subs.write().expect("lock") = subs;
        self.held_back.store(held_back, Ordering::SeqCst);
        *self.quarantined.write().expect("lock") = quarantined;
        *self.etag.write().expect("lock") = new_etag();
        if let Some(past) = self.past_quota(cfg.owner.as_deref()) {
            super::report_past_quota("subscriptions", &container, &past);
        }
        Ok(())
    }

    /// What of the subscriptions in force is past the bounds they are held to, counted as a new
    /// subscription is admitted: each total holds its own subscriptions and those held back or
    /// quarantined at the start (which may be anyone's), and each subscriber's capacity its own
    /// unexpired ones. `None` when nothing is past one.
    pub(crate) fn past_quota(&self, owner: Option<&str>) -> Option<String> {
        let now = jose::now_secs();
        let subs = self.subs.read().expect("lock");
        let unloaded =
            self.held_back.load(Ordering::SeqCst) + self.quarantined.read().expect("lock").len();
        let is_owner = |s: &&Subscription| owner.is_some() && s.subscriber.as_deref() == owner;
        let share = |owners: bool| {
            let mine = || subs.values().filter(move |s| is_owner(s) == owners);
            let held = mine().count() + unloaded;
            let authors = mine()
                .filter(|s| !s.expired(now))
                .map(|s| s.subscriber.as_deref());
            let quota = if owners {
                &self.owner_quota
            } else {
                &self.quota
            };
            quota.past(held, authors)
        };
        let past: Vec<String> = [(false, "others'"), (true, "the owner's")]
            .into_iter()
            .filter_map(|(owners, whose)| share(owners).map(|p| format!("{whose} share: {p}")))
            .collect();
        (!past.is_empty()).then(|| past.join("; "))
    }

    fn get(&self, id: &str) -> Option<Subscription> {
        self.subs
            .read()
            .expect("lock")
            .get(id)
            .filter(|s| !s.expired(jose::now_secs()))
            .cloned()
    }

    /// The unexpired subscriptions a listing shows: `subscriber`'s own, or, for the storage
    /// `owner`, every one. A subscriber's are read through the subscriber index, so the listing
    /// reads its records and no one else's; an expired one is shown to nobody (the store copy of
    /// it is removed the next time it is touched, or at boot).
    fn live_for(&self, subscriber: Option<&str>, owner: bool) -> Vec<Subscription> {
        let now = jose::now_secs();
        let subs = self.subs.read().expect("lock");
        if owner {
            return subs.values().filter(|s| !s.expired(now)).cloned().collect();
        }
        // An unauthenticated subscriber holds nothing of its own to show: subscriptions made
        // without a subject belong to no one who could come back for them.
        match subscriber {
            Some(_) => subs.live_of(subscriber, now),
            None => Vec::new(),
        }
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

    /// Remove a subscription as every service record is removed
    /// ([`super::records::remove_record`]): memory follows the store, so it leaves only once its
    /// removal is confirmed, and is announced then. One no longer in force is already gone.
    async fn remove<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        id: &str,
        admission: Option<crate::overload::AdmissionSlot>,
        by: super::records::Remover,
    ) -> Result<(), Response> {
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}{id}");
        // An expired subscription is listed nowhere, so its removal holds no listing's lock and
        // keeps no conditional create waiting; a live one is in its subscriber's listing too.
        // (Expiry only ever comes: one expired now stays expired.)
        let own_listing = {
            let subs = self.subs.read().expect("lock");
            match subs.get(id) {
                Some(s) if !s.expired(jose::now_secs()) => {
                    Some(listing_lock(&container, s.subscriber.as_deref()))
                }
                _ => None,
            }
        };
        let listed = match &own_listing {
            Some(name) => super::records::Listed::AlsoIn(name),
            None => super::records::Listed::Nowhere,
        };
        let set_aside = {
            let (state, id) = (state.clone(), id.to_string());
            move || {
                state
                    .notify
                    .subs
                    .read()
                    .expect("lock")
                    .contains_key(&id)
                    .then_some(())
            }
        };
        let settle = {
            let id = id.to_string();
            move |state: &LwsState<S>, gone: bool, ()| {
                if !gone {
                    return None;
                }
                let me = &state.notify;
                // An expired subscription is listed nowhere, so its removal changes no listing.
                let removed = me.subs.write().expect("lock").remove(&id);
                let author = removed.as_ref().and_then(|s| s.subscriber.clone());
                if removed.is_some_and(|s| !s.expired(jose::now_secs())) {
                    *me.etag.write().expect("lock") = new_etag();
                }
                author
            }
        };
        super::records::remove_record(
            state,
            &container,
            &iri,
            "subscription",
            listed,
            by,
            admission,
            set_aside,
            settle,
        )
        .await
        .map(|_| ())
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
    ) -> Prepared {
        self.prepare_at_most(state, event, self.limits.queue).await
    }

    /// [`Notifier::prepare`], for at most `limit` subscriptions: a change that prepares many
    /// (a recursive delete prepares one per resource it removes) shares one bound between them.
    pub async fn prepare_at_most<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: &Event,
        limit: usize,
    ) -> Prepared {
        self.prepare_for(state, event, limit, None).await
    }

    /// Tell every subscriber whose topic covers the service record `event.uri` (an access grant
    /// or request, a subscription), and who may read it, that it was created or removed:
    /// `author` is the record's author, who may read it, as the storage owner may. Called once
    /// the change is made and its locks released.
    pub async fn announce_service<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: Event,
        author: Option<String>,
    ) {
        let pending = self
            .prepare_for(state, &event, self.limits.queue, Some(author))
            .await;
        self.send(state, pending);
    }

    /// [`Notifier::prepare_at_most`], for a service record when `service` holds its author.
    async fn prepare_for<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        event: &Event,
        limit: usize,
        service: Option<Option<String>>,
    ) -> Prepared {
        let now = jose::now_secs();
        // A resource's linkset changes with it, and is read under its authorization: a
        // subscription to the linkset itself hears of it as of the resource.
        let linkset = format!("{}{META_SUFFIX}", event.uri);
        let names_linkset = |s: &Subscription| s.topics.contains(&linkset);
        // A service record is not a member of the storage's containers, so only a subscription
        // to its own service container (or to the record) hears of it, not one to an ancestor.
        let service_container = event.relation.as_ref().map(|(_, c)| c.as_str());
        let hears = |s: &Subscription| match service {
            Some(_) => s
                .topics
                .iter()
                .any(|t| *t == event.uri || Some(t.as_str()) == service_container),
            None => s.covers(&event.uri) || names_linkset(s),
        };
        // Every matching subscription is a candidate: the bound is on the deliveries prepared,
        // which only an authorized one makes, so one that may not read the resource never takes
        // the place of one that may. For the same reason the authorization done here has no
        // budget of its own: a budget spent on the candidates in turn could be used up by one
        // party's subscriptions that may not read the resource before an authorized one is
        // reached. The work is bounded instead by the subscriptions in force, which the totals
        // bound: at most MAX_SUBSCRIPTIONS held by subscribers other than the owner and
        // MAX_OWNER_SUBSCRIPTIONS by the owner, so one authorization each for at most
        // MAX_SUBSCRIPTIONS + MAX_OWNER_SUBSCRIPTIONS candidates, and none past the `limit`-th
        // delivery prepared. That is a ceiling on the number of checks, not on their work: each
        // takes the resource's access from one snapshot taken for the change
        // ([`access::ResourceAccess`]), so the resource's metadata is read and the grants (which
        // have no quota) are scanned once per change, not once per check. What the checks admit
        // is bounded by `limit`, and the deliveries they make leave the request's task before
        // any of them is queued ([`Notifier::send`]). Stored subscriptions loaded past a bound are reported at the
        // start, and admit no new one to it until it is under it.
        let candidates: Vec<Subscription> = {
            let subs = self.subs.read().expect("lock");
            subs.values()
                .filter(|s| !s.expired(now) && hears(s))
                .cloned()
                .collect()
        };
        #[cfg(test)]
        if !candidates.is_empty() {
            self.prepared_for.fetch_add(1, Ordering::SeqCst);
        }
        let mut out = Prepared {
            uri: event.uri.clone(),
            limit,
            ..Prepared::default()
        };
        // A Delete is prepared while the resource is still there; its deliveries, made once it
        // is gone, are authorized against what it was. When that cannot be read, only the owner
        // (and everyone in open mode) hears of it: no one else's access can be decided.
        let mut unreadable = false;
        let snapshot = if event.kind == "Delete" && !candidates.is_empty() && service.is_none() {
            match access::Snapshot::take(state, &event.uri).await {
                Ok(s) => Some(Arc::new(s)),
                Err(_) => {
                    unreadable = true;
                    Some(Arc::new(access::Snapshot::unreadable()))
                }
            }
        } else {
            None
        };
        let total = candidates.len();
        // The resource's access, taken at the first check that needs it.
        let mut resource: Option<access::ResourceAccess> = None;
        for (i, sub) in candidates.into_iter().enumerate() {
            // Once the bound is reached the rest are not authorized (that work is what the bound
            // saves), so whether each was owed a notification is not known: they are counted
            // apart from the dropped ones, each of which was.
            if out.len() == limit {
                out.unprepared = total - i;
                break;
            }
            // Delivery-time authorization (section 10.3.3): a subscriber that may not read the
            // resource now hears nothing about it.
            let agent = sub.agent();
            let allowed = match &service {
                Some(author) => reads_service_record(state, &agent, author),
                None if state.cfg.open || access::is_owner(state, &agent) => true,
                None if unreadable => false,
                None => {
                    if resource.is_none() {
                        resource = Some(
                            access::ResourceAccess::take(state, Action::Read, &event.uri).await,
                        );
                    }
                    resource.as_ref().is_some_and(|r| r.allows(state, &agent))
                }
            };
            if !allowed {
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
                    out.dropped += 1;
                    continue;
                }
                out.pending.push(Pending {
                    inbox: sub.inbox.clone(),
                    activity,
                    // Checked as the resource the linkset describes: its lock and who may
                    // read it.
                    watch: Watch {
                        agent: sub.agent(),
                        subscription: sub.id.clone(),
                        uri: event.uri.clone(),
                        snapshot: snapshot.clone(),
                        service: service.clone(),
                    },
                });
            }
        }
        out
    }

    /// Deliver notifications [`Notifier::prepare`]d earlier, and count what preparing them left
    /// out.
    pub fn send<S: Store + 'static>(&self, state: &LwsState<S>, prepared: Prepared) {
        if prepared.dropped > 0 {
            self.note_dropped(prepared.dropped, &prepared.uri);
        }
        if prepared.unprepared > 0 {
            self.note_unprepared(prepared.unprepared, prepared.limit, &prepared.uri);
        }
        if prepared.pending.is_empty() {
            return;
        }
        // The deliveries themselves leave this task: taking a place in the delivery queue,
        // parsing each inbox, building and signing each body and running each delivery is work
        // per subscription, and how many subscriptions one change has is no business of the
        // request that made it. One task takes the queue for all of them, in the order they
        // were prepared, so a subscription's notifications still go out in that order. When the
        // runtime runs that task is the runtime's business: this says the request does not do
        // the work, not that the work happens after the response reaches the client.
        let state = state.clone();
        tokio::spawn(async move {
            for p in prepared.pending {
                state
                    .notify
                    .deliver(&state, &p.inbox, p.activity, Some(p.watch));
            }
        });
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

    /// Count `n` subscriptions that heard of a change to `uri` past the bound of `limit`
    /// deliveries, and so were neither authorized nor notified. Logged as drops are.
    fn note_unprepared(&self, n: usize, limit: usize, uri: &str) {
        let before = self.unprepared.fetch_add(n as u64, Ordering::SeqCst);
        let after = before + n as u64;
        if before == 0 || before.ilog2() != after.ilog2() {
            eprintln!(
                "lws: {limit} webhook notification(s) prepared for {uri}, the most one change \
                 may have; {n} more subscription(s) to it were neither checked nor notified \
                 ({after} so far)"
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
                let by = super::records::Remover::Request;
                let _ = notifier.remove(&state, &id, None, by).await;
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

    /// Notifications dropped because the queue was full (or a change's deliveries were at their
    /// bound), each one known to be owed: authorized for its subscriber when it was prepared, or
    /// belonging to no subscription. Counted in memory since this notifier was made (each start
    /// begins at zero), and for a change only once it happened.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
    }

    /// Subscriptions that heard of a change once its deliveries were at their bound: neither
    /// authorized nor notified, so some may not have been owed a notification at all. Counted as
    /// [`Notifier::dropped`] is: since this notifier was made, for changes that happened.
    pub fn unprepared(&self) -> u64 {
        self.unprepared.load(Ordering::SeqCst)
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
        // `@query` (RFC 9421 section 2.2.7): the query as sent, with its `?`; `?` alone when
        // there is none.
        format!("?{}", target.query().unwrap_or_default()),
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
    // Whether what the request is to is set aside is said only to a caller that may act on it:
    // the owner, who may act on every subscription, at once; anyone else once authorized below.
    let unavailable = || super::resources::unavailable(state, req);
    if owner {
        if let Some(refused) = unavailable() {
            return refused;
        }
    }
    if id.is_empty() {
        if !linkset && !matches!(req.method, Method::GET | Method::HEAD | Method::POST) {
            return method_not_allowed("GET, HEAD, POST");
        }
        if state.needs_auth(agent) {
            return state.challenge(None);
        }
        // A new subscription is authorized by its topics: whether the container is set aside
        // is said once they are checked (see [`subscribe`]).
        if req.method != Method::POST || linkset {
            if let Some(refused) = unavailable() {
                return refused;
            }
        }
        if linkset {
            return service_linkset(&state.cfg, req, &state.cfg.absolute(SUBSCRIPTIONS_PATH));
        }
        return match req.method {
            Method::GET | Method::HEAD => listing(state, req, agent, owner),
            // Expired subscriptions still hold their place in the store until they are removed:
            // a few go once the request is authorized (see [`subscribe`]).
            Method::POST => subscribe(state, req, agent, owner).await,
            _ => method_not_allowed("GET, HEAD, POST"),
        };
    }
    if owner && state.notify.quarantined.read().expect("lock").contains(id) {
        return quarantined(state, req, id).await;
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
    if let Some(refused) = unavailable() {
        return refused;
    }
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
            let by = super::records::Remover::Request;
            match state
                .notify
                .remove(state, id, req.admission.clone(), by)
                .await
            {
                Ok(()) => problem(StatusCode::NO_CONTENT, None),
                Err(refused) => refused,
            }
        }
        _ => method_not_allowed("GET, HEAD, DELETE"),
    }
}

/// The owner's request for a quarantined subscription ([`Notifier::quarantined`]): a read is
/// refused, as its stored body is not a subscription; a DELETE removes it and frees its place.
async fn quarantined<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    id: &str,
) -> Response {
    match req.method {
        Method::GET | Method::HEAD => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("this subscription could not be read; deleting it frees its place"),
        ),
        Method::DELETE => {
            let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
            let iri = format!("{container}{id}");
            // Removed as every service record is ([`super::records::remove_record`]): its place
            // is freed only once its removal is confirmed.
            let set_aside = {
                let (state, id) = (state.clone(), id.to_string());
                move || {
                    let quarantined = state.notify.quarantined.read().expect("lock");
                    quarantined.contains(&id).then_some(())
                }
            };
            let settle = {
                let id = id.to_string();
                move |state: &LwsState<S>, gone: bool, ()| {
                    if gone {
                        state.notify.quarantined.write().expect("lock").remove(&id);
                        *state.notify.etag.write().expect("lock") = new_etag();
                    }
                    None
                }
            };
            let removed = super::records::remove_record(
                state,
                &container,
                &iri,
                "subscription",
                // Listed to the owner alone, in the container's listing.
                super::records::Listed::InContainer,
                super::records::Remover::Request,
                req.admission.clone(),
                set_aside,
                settle,
            );
            match removed.await {
                Ok(super::records::Removed::Gone) => problem(StatusCode::NO_CONTENT, None),
                Ok(super::records::Removed::Absent) => problem(StatusCode::NOT_FOUND, None),
                Err(refused) => refused,
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
        .live_for(agent.subject.as_deref(), owner)
        .into_iter()
        .map(|s| s.id)
        .chain(
            owner
                .then(|| state.notify.quarantined.read().expect("lock").clone())
                .into_iter()
                .flatten(),
        )
        .map(|id| json!({"id": format!("{base}{id}"), "type": "DataResource", "format": LWS_JSON}))
        .collect();
    // The owner's listing changes with every subscription; a subscriber's only with its own, so
    // its tag follows those alone and says nothing of anyone else's.
    let version = if owner {
        state.notify.etag.read().expect("lock").clone()
    } else {
        let listed: Vec<String> = items.iter().map(Value::to_string).collect();
        etag_of(listed.iter().map(String::as_str))
    };
    let mut resp = service_listing(&state.cfg, req, &base, items, &version);
    // Each subscriber sees only its own subscriptions.
    set(resp.headers_mut(), header::VARY, "Accept, Authorization");
    resp
}

/// The name of the lock of `subscriber`'s own listing of the subscriptions `container` holds
/// (see `records::Listed`): no IRI, so it is never an IRI's lock, and one per subscriber
/// (anonymous subscriptions share theirs, as they share their listing).
fn listing_lock(container: &str, subscriber: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(container.as_bytes());
    digest.update([0xff]);
    if let Some(s) = subscriber {
        digest.update([1]);
        digest.update(s.as_bytes());
    }
    let hex: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("subscriber-listing:{hex}")
}

/// Remove the requester's own expired subscriptions, as a new one is requested: at most
/// [`EXPIRED_REMOVED_PER_SUBSCRIBE`], all of them `subscriber`'s own, read through the
/// subscriber index. This is the only purging the request waits for, so the work it does here
/// is bounded by its own records; everyone else's are swept by the expiry sweeper
/// ([`spawn_expiry_sweeper`]). Never while the request holds the listing alone.
async fn purge_own_expired<S: Store + 'static>(
    state: &LwsState<S>,
    subscriber: Option<&str>,
    admission: Option<crate::overload::AdmissionSlot>,
) {
    let now = jose::now_secs();
    let expired = {
        let subs = state.notify.subs.read().expect("lock");
        subs.expired_of(subscriber, now, EXPIRED_REMOVED_PER_SUBSCRIBE)
    };
    for id in expired {
        let _ = state
            .notify
            .remove(
                state,
                &id,
                admission.clone(),
                super::records::Remover::Request,
            )
            .await;
    }
}

/// Ask the expiry sweeper ([`spawn_expiry_sweeper`]) for a sweep now.
///
/// Asked by a create that goes through, once its locks are released, so the sweep neither waits
/// on the listing that request held nor makes the request wait on records that are not its own.
fn wake_expiry_sweeper<S: Store + 'static>(state: &LwsState<S>) {
    state.notify.sweep_now.notify_one();
}

/// How long the expiry sweeper ([`spawn_expiry_sweeper`]) waits between sweeps, at a fixed rate
/// (a sweep only starts removals, so a slow one puts none of the next back).
const EXPIRY_SWEEP_EVERY: Duration = Duration::from_secs(2);

/// How many subscriptions one sweep looks at, at most, from where the last one stopped.
const SCANNED_PER_SWEEP: usize = 256;

/// How long one of the sweeper's removals may take before it is given up, to be tried again.
const SWEEP_REMOVAL_DEADLINE: Duration = Duration::from_secs(5);

/// How long a subscription whose removal failed, was given up or panicked waits before it is
/// tried again, the first time: doubled each time after, up to [`SWEEP_BACKOFF_MOST`].
const SWEEP_BACKOFF_FIRST: Duration = Duration::from_secs(10);

/// The longest a subscription whose removal keeps failing waits between tries.
const SWEEP_BACKOFF_MOST: Duration = Duration::from_secs(600);

/// Sweep expired subscriptions every [`EXPIRY_SWEEP_EVERY`] (and when a create asks,
/// [`wake_expiry_sweeper`]), on a task of its own for as long as the state lives: whatever
/// requests come, and whether any is admitted or refused, an expired subscription is removed in
/// time, as far as the store makes progress (reclamation is eventual, subject to it).
///
/// The task holds the state weakly, and strongly only while a sweep starts its removals (a
/// synchronous step): each removal runs in a task the sweeper owns ([`Sweeper`]), holding the
/// state for at most [`SWEEP_REMOVAL_DEADLINE`], so once every other handle is gone the state
/// goes within that deadline, and the sweeper, with every removal it still owns, at its next
/// tick.
pub(crate) fn spawn_expiry_sweeper<S: Store + 'static>(state: &LwsState<S>) {
    let (weak, wake) = (state.downgrade(), state.notify.sweep_now.clone());
    tokio::spawn(async move {
        let mut sweeper = Sweeper::default();
        let start = tokio::time::Instant::now() + EXPIRY_SWEEP_EVERY;
        let mut ticks = tokio::time::interval_at(start, EXPIRY_SWEEP_EVERY);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = wake.notified() => {}
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            #[cfg(test)]
            if state.notify.sweeper_off.load(Ordering::SeqCst) {
                continue;
            }
            sweeper.sweep(&state);
        }
    });
}

/// The expiry sweeper's removals and what it remembers between sweeps.
#[derive(Default)]
struct Sweeper {
    /// Each removal under way, owned here: one panicking ends only itself, and all go (aborted)
    /// with the sweeper.
    removals: tokio::task::JoinSet<bool>,
    /// Which subscription each removal under way is of.
    by_task: HashMap<tokio::task::Id, String>,
    /// The subscriptions whose removal did not go through: when each may be tried again, and how
    /// many times it has not.
    backoff: HashMap<String, (tokio::time::Instant, u32)>,
}

impl Sweeper {
    /// Take in the removals that ended, then start removing expired subscriptions: those not
    /// already being removed or waiting to be tried again, found among at most
    /// [`SCANNED_PER_SWEEP`] subscriptions from where the last sweep stopped, as many as there
    /// are places reserved for the sweeper ([`super::records::Remover::Sweeper`]). Each runs
    /// under [`SWEEP_REMOVAL_DEADLINE`], so one that never completes holds up no other. An
    /// expired subscription is listed nowhere, so its removal holds no listing's lock (see
    /// [`Notifier::remove`]).
    fn sweep<S: Store + 'static>(&mut self, state: &LwsState<S>) {
        let clock = tokio::time::Instant::now();
        while let Some(ended) = self.removals.try_join_next_with_id() {
            let (task, removed) = match ended {
                Ok((task, removed)) => (task, removed),
                Err(e) => (e.id(), false),
            };
            let Some(id) = self.by_task.remove(&task) else {
                continue;
            };
            if removed {
                self.backoff.remove(&id);
            } else {
                let entry = self.backoff.entry(id).or_insert((clock, 0));
                entry.1 = entry.1.saturating_add(1);
                let wait = SWEEP_BACKOFF_FIRST
                    .saturating_mul(1 << entry.1.min(16).saturating_sub(1))
                    .min(SWEEP_BACKOFF_MOST);
                entry.0 = clock + wait;
            }
        }
        let room = state.settlements.maintenance_free();
        let now = jose::now_secs();
        let candidates: Vec<String> = {
            use std::ops::Bound::{Excluded, Unbounded};
            let subs = state.notify.subs.read().expect("lock");
            // Waits are kept only for subscriptions still there.
            self.backoff.retain(|id, _| subs.contains_key(id));
            let mut cursor = state.notify.purged_to.lock().expect("lock");
            let busy: std::collections::HashSet<&String> = self.by_task.values().collect();
            let mut found = Vec::new();
            let mut last = None;
            for (id, sub) in subs
                .range::<String, _>((Excluded(&*cursor), Unbounded))
                .chain(subs.range::<String, _>(..=&*cursor))
                .take(SCANNED_PER_SWEEP)
            {
                if found.len() == room {
                    break;
                }
                last = Some(id.clone());
                let waiting = self.backoff.get(id).is_some_and(|(at, _)| *at > clock);
                if sub.expired(now) && !busy.contains(id) && !waiting {
                    found.push(id.clone());
                }
            }
            // The next sweep starts after the last one looked at.
            if let Some(last) = last {
                *cursor = last;
            }
            found
        };
        for id in candidates {
            let (state, sub) = (state.clone(), id.clone());
            let removal = self.removals.spawn(async move {
                let removal =
                    state
                        .notify
                        .remove(&state, &sub, None, super::records::Remover::Sweeper);
                matches!(
                    tokio::time::timeout(SWEEP_REMOVAL_DEADLINE, removal).await,
                    Ok(Ok(()))
                )
            });
            self.by_task.insert(removal.id(), id);
        }
    }

    /// Wait for every removal under way to end (tests).
    #[cfg(test)]
    async fn settled(&mut self) {
        while self.removals.join_next().await.is_some() {}
    }
}

async fn subscribe<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    lister: bool,
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
    //
    // The subscriber is checked before any wait for the lock, and only a topic it may read is
    // waited on, or said to be set aside: one it may not read is refused the same whatever its
    // state, so neither a stuck change to it nor a write holding its lock shows through. A
    // set-aside topic is decided as a request to it is (see [`access::aside`]): on what it was
    // before the stuck change began, failing closed when that cannot be read. The check that
    // counts is made again under the lock.
    let storage = state.cfg.storage();
    let cannot_read = || {
        if agent.is_authenticated() || state.cfg.open {
            problem(
                StatusCode::FORBIDDEN,
                Some("the subscriber cannot read every topic"),
            )
        } else {
            state.challenge(None)
        }
    };
    let storage = storage.as_str();
    let readable = |resource: &str, aside: bool| {
        let resource = resource.to_string();
        async move {
            if !resource.starts_with(storage) {
                return false;
            }
            if aside {
                return matches!(
                    access::aside(state, Action::Read, &resource, agent).await,
                    Ok(access::Aside::Allowed)
                );
            }
            state.store.exists(&resource).await.unwrap_or(false)
                && state.allowed(Action::Read, &resource, agent).await
        }
    };
    for topic in &topics {
        let resource = topic.strip_suffix(META_SUFFIX).unwrap_or(topic);
        // A linkset's record is no resource, and has no linkset: whatever is stored, it is not
        // there to read.
        if super::names_record(resource) {
            return cannot_read();
        }
        // The lock of a visible topic that nobody is changing is taken at once, without a wait:
        // then the one check under it says nothing of its state. Otherwise the topic is checked
        // first, without it.
        let _guard = match state.locks.try_read(resource) {
            Some(guard) => guard,
            None => {
                // A topic being changed is decided as a set-aside one is, on what it was before
                // the change began.
                // Made again while changes to it begin or end during it (what was read may be
                // one part way, see `resources::stably`); refused when that never settles.
                let decided = super::resources::stably(state, resource, || async {
                    let hidden = !state.visible(resource);
                    let aside = hidden || state.locks.changing(resource);
                    (readable(resource, aside).await, hidden)
                })
                .await;
                let Some((true, hidden)) = decided else {
                    return cannot_read();
                };
                if hidden {
                    return super::resources::set_aside_meanwhile();
                }
                // Waited for a bounded time, as every lock a request to a service container
                // takes.
                match super::records::within_wait(state.locks.read(resource)).await {
                    Ok(guard) => guard,
                    Err(refused) => return refused,
                }
            }
        };
        if !readable(resource, false).await {
            return cannot_read();
        }
    }
    // Only now, with every topic authorized, does the subscriber learn whether the
    // subscriptions container is set aside (or new changes wait on set-aside ones), and do
    // expired subscriptions, which may be anyone's, have a few removed (waiting on their
    // locks): before the listing a conditional create is evaluated against is held, and before
    // this one is counted.
    //
    // Only the requester's own expired subscriptions are removed here, at most
    // EXPIRED_REMOVED_PER_SUBSCRIBE of them: the work this request waits for is its own
    // records'. Everyone else's are swept by the expiry sweeper ([`spawn_expiry_sweeper`]),
    // which a create that goes through wakes at the end, and which runs whatever is admitted, so
    // no request waits on a sweep of records that are not its own, and no sweep takes a
    // listing's lock.
    if let Some(refused) = super::resources::unavailable(state, req) {
        return refused;
    }
    purge_own_expired(state, agent.subject.as_deref(), req.admission.clone()).await;
    // A conditional create is evaluated against the listing, held exclusively from here until
    // the subscription is registered. Taken only once every topic has been checked (and its
    // lock released): a topic naming the subscriptions container itself, or its linkset, would
    // otherwise wait for the lock this holds.
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    // The subscriber's own listing: what a subscriber that does not list them all compares
    // against, and what every change to its subscriptions holds (see `records::Listed`).
    let own_listing = listing_lock(&container, agent.subject.as_deref());
    let held = match super::records::service_preconditions(
        state,
        req,
        &container,
        (!lister).then_some(own_listing.as_str()),
        |get| listing(state, get, agent, lister),
    )
    .await
    {
        Ok(held) => held,
        Err(refused) => return refused,
    };
    // The place is reserved before anything is stored and held until the subscription is
    // registered. Two bounds are counted, and they say different things. The requester's own
    // capacity ([`MAX_SUBSCRIPTIONS_PER_SUBSCRIBER`]) is counted from its own records alone, so
    // `429` rests on nothing but what it holds itself. The total
    // ([`MAX_SUBSCRIPTIONS`], or [`MAX_OWNER_SUBSCRIPTIONS`] for the storage owner, whose
    // subscriptions are counted apart) is a bound on what the store holds: `507` says the
    // storage is full of subscriptions, whoever filled it, as it does for access requests, and
    // one subscriber cannot reach it alone. Expired subscriptions take room in the store until
    // they are removed, and so count against the total, but take none of their subscriber's
    // capacity: it can neither see nor cancel them.
    let now = jose::now_secs();
    let owner = access::is_owner(state, agent);
    let (quota, what) = if owner {
        (&state.notify.owner_quota, "owner subscriptions")
    } else {
        (&state.notify.quota, "subscriptions")
    };
    let reserved = quota.reserve(agent.subject.as_deref(), || {
        let subs = state.notify.subs.read().expect("lock");
        // Read through the subscriber index: only this subscriber's own records are counted,
        // and only they are touched.
        let mine = subs.live_count_of(agent.subject.as_deref(), now);
        // Each total counts the records in it. The owner's are the ones the subscriber index
        // holds for it (none when no owner is configured: nothing is then the owner's), and
        // everyone else's are the rest, which is the count of all of them less the owner's --
        // a length and one indexed lookup, so no one else's records are read to reach it.
        let owners = state
            .cfg
            .owner
            .as_deref()
            .map_or(0, |o| subs.count_of(Some(o)));
        let in_share = if owner { owners } else { subs.len() - owners };
        // Those held back or quarantined at the start may be anyone's: they count against
        // either total.
        (
            in_share
                + state.notify.held_back.load(Ordering::SeqCst)
                + state.notify.quarantined.read().expect("lock").len(),
            mine,
        )
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
    let author = sub.subscriber.clone();
    let register = {
        move |state: &LwsState<S>| {
            state.notify.subs.write().expect("lock").insert(id, sub);
            *state.notify.etag.write().expect("lock") = new_etag();
            // Counted as registered from here on.
            drop(slot);
        }
    };
    let created = super::records::create_record(
        state,
        &container,
        &iri,
        Bytes::from(stored),
        req.admission.clone(),
        super::records::Listed::AlsoIn(&own_listing),
        held,
        author,
        register,
    );
    if let Err(refused) = created.await {
        return refused;
    }
    // The listing and the new record's locks are gone with the create, so the sweep of
    // everyone's expired subscriptions can run now without waiting on this request, and this
    // request answers without waiting on it.
    wake_expiry_sweeper(state);
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
    /// second is held back (counted, not loaded).
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
        // Review findings: one that could not be parsed stopped every start; then, held back,
        // it kept its place for good. It is quarantined at every start, until deleted; a read
        // that fails stops the start instead.
        for _ in 0..2 {
            let loaded = Notifier::load(&store, &cfg).await.unwrap();
            assert!(loaded.subs.read().unwrap().is_empty());
            assert!(loaded.quarantined.read().unwrap().contains("unreadable"));
            assert_eq!(loaded.held_back.load(Ordering::SeqCst), 0);
        }
        *store.fail_read_of.lock().unwrap() = Some(unreadable.clone());
        assert!(Notifier::load(&store, &cfg).await.is_err());
        *store.fail_read_of.lock().unwrap() = None;
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

    /// A subscriber's listing changes only with its own subscriptions, and so does its tag.
    #[tokio::test]
    async fn a_subscribers_tag_does_not_follow_anyone_elses_subscriptions() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, _store) = closed_state(4).await;
        let b = data(&state, "b").await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [b.clone()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let (alice, bob) = ("https://alice.example/#me", "https://bob.example/#me");
        grant_read(&state, alice, &[&b]).await;
        grant_read(&state, bob, &[&b]).await;
        let (alice, bob) = (agent(alice), agent(bob));
        let subscribe = |agent: Agent, headers: Vec<(&'static str, String)>| {
            let (state, body) = (state.clone(), body.clone());
            async move {
                let mut h = vec![("content-type", LWS_JSON)];
                h.extend(headers.iter().map(|(k, v)| (*k, v.as_str())));
                let req = test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body);
                test_store::route_as(&state, req, &agent).await.status()
            }
        };
        let tag = || {
            let (state, alice) = (state.clone(), alice.clone());
            async move {
                let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
                let r = test_store::route_as(&state, req, &alice).await;
                r.headers()["etag"].to_str().unwrap().to_string()
            }
        };
        assert_eq!(subscribe(alice.clone(), vec![]).await, StatusCode::CREATED);
        let before = tag().await;
        assert_eq!(subscribe(bob, vec![]).await, StatusCode::CREATED);
        assert_eq!(tag().await, before);
        let status = subscribe(alice.clone(), vec![("if-match", before.clone())]).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_ne!(tag().await, before);
    }

    /// Review finding: a purge always tried the first expired subscriptions, so sixteen that could
    /// not be removed kept every later one from being tried. Sweeps take them in turn, and one
    /// whose removal failed waits before it is tried again.
    #[tokio::test]
    async fn every_expired_subscription_gets_its_turn() {
        let (state, store) = test_store::state(4).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        // Swept by this test alone.
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let n = super::super::records::MAINTENANCE_PLACES + 4;
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
        // The first sweep's cannot be removed now.
        let mut sweeper = Sweeper::default();
        store
            .fail_delete
            .store(true, std::sync::atomic::Ordering::SeqCst);
        sweeper.sweep(&state);
        sweeper.settled().await;
        store
            .fail_delete
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(state.notify.subs.read().unwrap().len(), n);
        // The next sweep starts with the ones after them; those that failed wait.
        sweeper.sweep(&state);
        sweeper.settled().await;
        let left = state.notify.subs.read().unwrap();
        for id in &ids[super::super::records::MAINTENANCE_PLACES..] {
            assert!(!left.contains_key(id), "{id} never got its turn");
        }
        assert_eq!(left.len(), super::super::records::MAINTENANCE_PLACES);
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
            for i in 0..MAX_OWNER_SUBSCRIPTIONS {
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

    /// Review finding: a conditional subscription held the subscriptions container exclusively
    /// while it checked its topics, and checking a topic naming that container (or its linkset)
    /// waited for the container's shared lock: the create deadlocked, and every subscription
    /// create and cancellation behind it. The topics are checked before the container is held.
    #[tokio::test]
    async fn a_conditional_subscription_to_its_own_container_does_not_deadlock() {
        let (state, _store) = test_store::state(100).await;
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        for topic in [container.clone(), format!("{container}{META_SUFFIX}")] {
            let body = json!({
                "@context": ["https://www.w3.org/ns/lws/v1"],
                "type": WEBHOOK,
                "topic": [topic],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let post = test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON), ("if-match", "*")],
                &body,
            );
            let r = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                handle(&state, &post, &Agent::anonymous()),
            )
            .await
            .expect("answered");
            assert_ne!(r.status(), StatusCode::INTERNAL_SERVER_ERROR, "{topic}");
        }
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
                .pending
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
        let watch = prepared.pending.into_iter().next().unwrap().watch;
        // A write holds the resource; the delivery waits for it, and the write's rollback is
        // then set aside with the lock.
        let held = state.locks.lock(&file).await.unwrap();
        let check = tokio::spawn({
            let state = state.clone();
            async move { watch.stands(&state).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let left = super::super::Unsettled::new(vec![super::super::Undo::Remove {
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

    /// A quarantined subscription is listed to the owner, refuses a read, and goes, freeing its
    /// place, when the owner deletes it.
    #[tokio::test]
    async fn a_quarantined_subscription_is_listed_until_the_owner_deletes_it() {
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}bad");
        store
            .create_in_container(&container, &iri, Bytes::from("[]"), LWS_JSON)
            .await
            .unwrap();
        state
            .notify
            .load_with(&store, &state.cfg, |_| true)
            .await
            .unwrap();
        let anon = Agent::anonymous();
        let path = format!("{SUBSCRIPTIONS_PATH}bad");
        let req = |m: Method, p: &str| test_store::request(m, p, &[], "");
        let list = handle(&state, &req(Method::GET, SUBSCRIPTIONS_PATH), &anon).await;
        assert_eq!(test_store::body_json(list).await["totalItems"], 1);
        let r = handle(&state, &req(Method::GET, &path), &anon).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let r = handle(&state, &req(Method::DELETE, &path), &anon).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(!store.exists(&iri).await.unwrap());
        assert!(state.notify.quarantined.read().unwrap().is_empty());
        let list = handle(&state, &req(Method::GET, SUBSCRIPTIONS_PATH), &anon).await;
        assert_eq!(test_store::body_json(list).await["totalItems"], 0);
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
                service: None,
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
        assert_eq!(subs.get("kept").unwrap().failures, 1);
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
                    service: None,
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

    /// Review finding: a subscription whose create was confirmed only after the request had
    /// answered (the store's reply was lost, and the listing could not be read) was put in force
    /// but never announced. It is announced as it is put in force, wherever that happens.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_subscription_confirmed_after_its_create_answered_is_announced() {
        use std::sync::atomic::Ordering;
        let heard = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let app = {
            let heard = heard.clone();
            axum::Router::new().route(
                "/inbox",
                axum::routing::post(move |body: String| {
                    let heard = heard.clone();
                    async move {
                        heard.lock().unwrap().push(body);
                        StatusCode::NO_CONTENT
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        // A subscription to the subscriptions container hears of each one created.
        let watcher = Subscription {
            id: "w".into(),
            subscriber: None,
            client: None,
            topics: vec![container.clone()],
            inbox: format!("http://{addr}/inbox"),
            expires: None,
            expires_at: None,
            failures: 0,
        };
        state
            .notify
            .subs
            .write()
            .unwrap()
            .insert("w".into(), watcher);
        let body = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
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
        store.fail_after_create.store(true, Ordering::SeqCst);
        store.fail_list.store(true, Ordering::SeqCst);
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        store.fail_after_create.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        for _ in 0..100 {
            if heard.lock().unwrap().iter().any(|b| b.contains("Create")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(state.notify.subs.read().unwrap().len(), 2);
        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert!(heard[0].contains("Create") && heard[0].contains(&container));
    }

    /// Review finding: the deliveries prepared for a change were bounded before they were
    /// authorized, so a subscription whose subscriber may not read the resource took the place
    /// of one whose subscriber may. Only authorized deliveries count against the bound.
    #[tokio::test]
    async fn an_unauthorized_subscription_takes_no_place_from_an_authorized_one() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        for (id, who) in [
            ("a", "https://bob.example/#me"),
            ("b", "https://owner.example/#me"),
        ] {
            let sub = Subscription {
                id: id.into(),
                subscriber: Some(who.into()),
                client: None,
                topics: vec![state.cfg.storage()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let event = Event {
            kind: "Update",
            is_container: true,
            uri: state.cfg.storage(),
            relation: None,
        };
        let prepared = state.notify.prepare_at_most(&state, &event, 1).await;
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared.pending[0].watch.subscription, "b");
    }

    /// Review finding (#6810): once a change's deliveries reached their bound, the subscriptions
    /// left were counted as dropped deliveries without being authorized, so the count took in
    /// ones that were owed nothing. Only deliveries known to be owed are counted dropped; the
    /// subscriptions past the bound are counted apart, as neither checked nor notified.
    #[tokio::test]
    async fn only_owed_deliveries_are_counted_dropped() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let root = state.cfg.storage();
        let linkset = format!("{root}{META_SUFFIX}");
        for (id, who, topics) in [
            ("a", "https://bob.example/#me", vec![root.clone()]),
            ("b", "https://owner.example/#me", vec![root.clone()]),
            ("c", "https://bob.example/#me", vec![root.clone()]),
            ("d", "https://owner.example/#me", vec![root.clone()]),
            (
                "e",
                "https://owner.example/#me",
                vec![root.clone(), linkset],
            ),
        ] {
            let sub = Subscription {
                id: id.into(),
                subscriber: Some(who.into()),
                client: None,
                topics,
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let event = Event {
            kind: "Update",
            is_container: true,
            uri: root.clone(),
            relation: None,
        };
        // What preparing left out is counted once the notifications are sent (here, none).
        let send = |mut prepared: Prepared| {
            prepared.pending.clear();
            state.notify.send(&state, prepared);
        };
        // Bob may not read the root, the owner may: `a` is passed over, `b` fills the bound, and
        // `c`, `d` and `e` are neither checked nor notified, whoever they belong to.
        let prepared = state.notify.prepare_at_most(&state, &event, 1).await;
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared.pending[0].watch.subscription, "b");
        assert_eq!((prepared.dropped, prepared.unprepared), (0, 3));
        assert_eq!(state.notify.unprepared(), 0, "counted only once sent");
        send(prepared);
        assert_eq!(state.notify.dropped(), 0);
        assert_eq!(state.notify.unprepared(), 3);
        // With room for all three of the owner's, `e` brings two (the root and its linkset),
        // the second past the bound: authorized, so owed, and counted dropped.
        let prepared = state.notify.prepare_at_most(&state, &event, 3).await;
        assert_eq!(prepared.len(), 3);
        send(prepared);
        assert_eq!(state.notify.dropped(), 1);
        assert_eq!(state.notify.unprepared(), 3);
        // A shared bound already spent (a recursive delete's) checks nothing at all.
        let prepared = state.notify.prepare_at_most(&state, &event, 0).await;
        assert!(prepared.is_empty());
        send(prepared);
        assert_eq!(state.notify.unprepared(), 8);
        assert_eq!(state.notify.dropped(), 1);
    }

    /// Codex review of #6841: each check scanned every grant (and read the resource's metadata)
    /// again. One snapshot of the resource's access serves a change's checks; it decides as a
    /// read does, grant by grant and constraint by constraint.
    #[tokio::test]
    async fn preparation_decides_as_a_read_does() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let root = state.cfg.storage();
        let r = state.cfg.absolute("/r");
        state
            .store
            .write(&r, Bytes::from("r"), "text/plain")
            .await
            .unwrap();
        let grant = |access: Value| access::policies(&access, &root).unwrap();
        state.access.put_grant_for_test(
            "read",
            grant(json!([{"type": ["AccessPolicy"], "action": ["read"],
                "assignee": "https://bob.example/#me"}])),
        );
        state.access.put_grant_for_test(
            "modify",
            grant(json!([{"type": ["AccessPolicy"], "action": ["modify"],
                "assignee": "https://dave.example/#me"}])),
        );
        state.access.put_grant_for_test(
            "client",
            grant(json!([{"type": ["AccessPolicy"], "action": ["read"],
                "assignee": "https://erin.example/#me",
                "constraint": [{"leftOperand": "client", "operator": "eq", "rightOperand": "https://app.example/"}]}])),
        );
        state.access.put_grant_for_test(
            "elsewhere",
            grant(json!([{"type": ["AccessPolicy"], "action": ["read"],
                "assignee": "https://frank.example/#me",
                "target": {"type": "DataResource", "value": [format!("{root}other")]}}])),
        );
        let agents = [
            ("bob", Some("https://bob.example/#me"), None),
            ("carol", Some("https://carol.example/#me"), None),
            ("dave", Some("https://dave.example/#me"), None),
            (
                "erin-app",
                Some("https://erin.example/#me"),
                Some("https://app.example/"),
            ),
            (
                "erin-other",
                Some("https://erin.example/#me"),
                Some("https://other.example/"),
            ),
            ("frank", Some("https://frank.example/#me"), None),
            ("owner", Some("https://owner.example/#me"), None),
            ("anonymous", None, None),
        ];
        for (id, subject, client) in agents {
            let sub = Subscription {
                id: id.into(),
                subscriber: subject.map(str::to_string),
                client: client.map(str::to_string),
                topics: vec![root.clone()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let event = Event {
            kind: "Update",
            uri: r.clone(),
            is_container: false,
            relation: None,
        };
        let mut prepared: Vec<String> = state
            .notify
            .prepare(&state, &event)
            .await
            .pending
            .iter()
            .map(|p| p.watch.subscription.clone())
            .collect();
        prepared.sort();
        let mut reads = Vec::new();
        for (id, subject, client) in agents {
            let agent = Agent {
                subject: subject.map(str::to_string),
                client: client.map(str::to_string),
            };
            if state.allowed(Action::Read, &r, &agent).await {
                reads.push(id.to_string());
            }
        }
        reads.sort();
        assert_eq!(prepared, reads);
        assert_eq!(prepared, ["bob", "erin-app", "owner"]);
    }

    /// The subscriptions a change to `/r` (a `text/plain` resource) prepares deliveries for, and
    /// the agents a read decides may read it, one subscription per agent, after `setup` has
    /// written the grants, the metadata and the failures of the case. For a Delete, a read is
    /// decided as its deliveries are checked: against the snapshot of the resource taken while
    /// it was there, or [`access::Snapshot::unreadable`] when that could not be taken.
    async fn prepared_and_reads(
        kind: &'static str,
        grants: &[(&str, Value)],
        setup: impl FnOnce(&mut super::super::ResourceMeta, &test_store::FlakyStore),
    ) -> (Vec<String>, Vec<String>) {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = test_store::FlakyStore::new();
        let state = LwsState::new(store.clone(), cfg).await.unwrap();
        let root = state.cfg.storage();
        let r = state.cfg.absolute("/r");
        state
            .store
            .write(&r, Bytes::from("r"), "text/plain")
            .await
            .unwrap();
        for (id, access) in grants {
            state
                .access
                .put_grant_for_test(id, access::policies(access, &root).unwrap());
        }
        let mut meta = state.resource_meta(&r).await.unwrap();
        setup(&mut meta, &store);
        state.put_resource_meta(&r, &meta).await.ok();
        let agents = [
            ("bob", Some("https://bob.example/#me"), None),
            ("carol", Some("https://carol.example/#me"), None),
            ("dave", Some("https://dave.example/#me"), None),
            (
                "erin-app",
                Some("https://erin.example/#me"),
                Some("https://app.example/"),
            ),
            (
                "erin-other",
                Some("https://erin.example/#me"),
                Some("https://other.example/"),
            ),
            ("frank", Some("https://frank.example/#me"), None),
            ("owner", Some("https://owner.example/#me"), None),
            ("anonymous", None, None),
        ];
        for (id, subject, client) in agents {
            let sub = Subscription {
                id: id.into(),
                subscriber: subject.map(str::to_string),
                client: client.map(str::to_string),
                topics: vec![root.clone()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let event = Event {
            kind,
            uri: r.clone(),
            is_container: false,
            relation: None,
        };
        let snapshot = if kind == "Delete" {
            Some(
                access::Snapshot::take(&state, &r)
                    .await
                    .unwrap_or_else(|_| access::Snapshot::unreadable()),
            )
        } else {
            None
        };
        let mut prepared: Vec<String> = state
            .notify
            .prepare(&state, &event)
            .await
            .pending
            .iter()
            .map(|p| p.watch.subscription.clone())
            .collect();
        prepared.sort();
        let mut reads = Vec::new();
        for (id, subject, client) in agents {
            let agent = Agent {
                subject: subject.map(str::to_string),
                client: client.map(str::to_string),
            };
            let reads_it = match &snapshot {
                Some(s) => access::allowed_as(&state, Action::Read, &r, &agent, s).await,
                None => state.allowed(Action::Read, &r, &agent).await,
            };
            if reads_it {
                reads.push(id.to_string());
            }
        }
        reads.sort();
        (prepared, reads)
    }

    /// Issue #6874: preparation decides from one snapshot of the resource's access
    /// ([`access::ResourceAccess`]); it agrees with a read for every kind of grant and
    /// constraint, the creator, pending metadata and a backend that fails to read. A Delete's
    /// deliveries are checked against a second snapshot ([`access::Snapshot`]), taken in the
    /// same preparation: the two agree too, and where that one cannot be taken only the owner
    /// is prepared for and allowed (#6903), so a subscriber never hears of a change one of them
    /// would refuse it.
    #[tokio::test]
    async fn preparation_decides_as_a_read_does_for_every_kind_of_access() {
        let read = |who: &str, constraint: Value| {
            json!([{"type": ["AccessPolicy"], "action": ["read"], "assignee": who,
                "constraint": constraint}])
        };
        let c = |left: &str, op: &str, right: Value| json!([{"leftOperand": left, "operator": op, "rightOperand": right}]);
        let (bob, carol, dave) = (
            "https://bob.example/#me",
            "https://carol.example/#me",
            "https://dave.example/#me",
        );
        let types = [
            (
                "bob",
                read(bob, c("type", "eq", json!("https://e.example/T"))),
            ),
            (
                "carol",
                read(carol, c("type", "isAnyOf", json!(["DataResource"]))),
            ),
            (
                "dave",
                read(dave, c("type", "eq", json!("https://e.example/Other"))),
            ),
        ];
        let formats = [
            ("bob", read(bob, c("format", "eq", json!("text/plain")))),
            (
                "carol",
                read(carol, c("format", "isAnyOf", json!(["text/turtle"]))),
            ),
        ];
        let times = [
            (
                "bob",
                read(bob, c("dateTime", "lt", json!("2999-01-01T00:00:00Z"))),
            ),
            (
                "carol",
                read(carol, c("dateTime", "lt", json!("2000-01-01T00:00:00Z"))),
            ),
            (
                "dave",
                read(dave, c("dateTime", "gt", json!("2000-01-01T00:00:00Z"))),
            ),
        ];
        let everyone = [("anyone", read(super::super::FOAF_AGENT, json!([])))];
        let any_app = [(
            "app",
            read(
                super::super::FOAF_AGENT,
                c("client", "eq", json!("https://app.example/")),
            ),
        )];
        let bob_reads = [("bob", read(bob, json!([])))];
        let dave_modifies = [(
            "dave",
            json!([{"type": ["AccessPolicy"], "action": ["modify"], "assignee": dave}]),
        )];
        let typed = |m: &mut super::super::ResourceMeta, _: &test_store::FlakyStore| {
            m.types = vec!["https://e.example/T".into()];
        };
        let created_by_carol = |m: &mut super::super::ResourceMeta, _: &test_store::FlakyStore| {
            m.creator = Some("https://carol.example/#me".into());
        };
        let pending = |m: &mut super::super::ResourceMeta, _: &test_store::FlakyStore| {
            m.creator = Some("https://carol.example/#me".into());
            m.pending = true;
        };
        let meta_unreadable = |_: &mut super::super::ResourceMeta, s: &test_store::FlakyStore| {
            *s.fail_read_of.lock().unwrap() =
                Some(super::super::meta_key("http://localhost:3000/r"));
        };
        let format_unreadable = |_: &mut super::super::ResourceMeta, s: &test_store::FlakyStore| {
            s.fail_meta.store(true, Ordering::SeqCst);
        };
        type Setup = fn(&mut super::super::ResourceMeta, &test_store::FlakyStore);
        type Case<'a> = (&'a str, &'a [(&'a str, Value)], Setup, &'a [&'a str]);
        let cases: Vec<Case<'_>> = vec![
            ("type", &types, typed, &["bob", "carol", "owner"]),
            ("format", &formats, |_, _| {}, &["bob", "owner"]),
            ("dateTime", &times, |_, _| {}, &["bob", "dave", "owner"]),
            (
                "foaf:Agent",
                &everyone,
                |_, _| {},
                &[
                    "anonymous",
                    "bob",
                    "carol",
                    "dave",
                    "erin-app",
                    "erin-other",
                    "frank",
                    "owner",
                ],
            ),
            (
                "foaf:Agent with a client",
                &any_app,
                |_, _| {},
                &["erin-app", "owner"],
            ),
            (
                "creator",
                &dave_modifies,
                created_by_carol,
                &["carol", "owner"],
            ),
            ("pending", &bob_reads, pending, &["carol", "owner"]),
            (
                "metadata unreadable",
                &bob_reads,
                meta_unreadable,
                &["owner"],
            ),
            (
                "format unreadable",
                &bob_reads,
                format_unreadable,
                &["owner"],
            ),
        ];
        for (case, grants, setup, want) in cases {
            let (prepared, reads) = prepared_and_reads("Update", grants, setup).await;
            assert_eq!(prepared, reads, "{case}");
            assert_eq!(prepared, want, "{case}");
            let (prepared, reads) = prepared_and_reads("Delete", grants, setup).await;
            // Where the resource's state could not be read, only the owner hears of it (#6903).
            assert_eq!(prepared, reads, "{case}: delete");
            assert_eq!(prepared, want, "{case}: delete");
        }
    }

    /// Codex review of #6841: a delete prepares its notifications before the removal is tried,
    /// and preparing counted what it left out at once, so a delete that failed (nothing removed,
    /// nothing announced) still moved both counters. They move only for a delete that happened.
    #[tokio::test]
    async fn a_failed_delete_counts_nothing() {
        use std::sync::atomic::Ordering;
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.delivery = DeliveryLimits {
            queue: 1,
            workers: 1,
            per_inbox: 1,
        };
        let store = test_store::FlakyStore::new();
        let state = LwsState::new(store.clone(), cfg).await.unwrap();
        let root = state.cfg.storage();
        let created = super::super::route(
            &state,
            test_store::request(
                Method::POST,
                "/",
                &[("slug", "f"), ("content-type", "text/plain")],
                "x",
            ),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let f = format!("{root}f");
        // `a` hears of `f` and of its linkset, `b` of everything: with room for one delivery,
        // a Delete of `f` prepares `a`'s first, drops `a`'s second (owed) and leaves `b` out.
        for (id, topics) in [
            ("a", vec![f.clone(), format!("{f}{META_SUFFIX}")]),
            ("b", vec![root.clone()]),
        ] {
            let sub = Subscription {
                id: id.into(),
                subscriber: None,
                client: None,
                topics,
                inbox: "http://127.0.0.1:9/inbox".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let delete =
            || super::super::route(&state, test_store::request(Method::DELETE, "/f", &[], ""));
        store.fail_delete.store(true, Ordering::SeqCst);
        let r = delete().await;
        assert!(r.status().is_server_error(), "{}", r.status());
        store.fail_delete.store(false, Ordering::SeqCst);
        for _ in 0..250 {
            if state.visible(&f) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(state.store.exists(&f).await.unwrap());
        assert_eq!(
            state.notify.prepared_for.load(Ordering::SeqCst),
            1,
            "prepared"
        );
        assert_eq!(state.notify.dropped(), 0);
        assert_eq!(state.notify.unprepared(), 0);
        let r = delete().await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.notify.dropped(), 1);
        assert_eq!(state.notify.unprepared(), 1);
    }

    /// Review finding (#6810): stored subscriptions can exceed the bounds they are held to
    /// (stored before one was lowered, or written to the store directly). The start loads them
    /// all, removes none, reports what is past which bound, and admits no new subscription to a
    /// bound until it is under it again. The two bounds answer differently and say different
    /// things: a subscriber at its own capacity is told `429`, it asked for too many; a
    /// subscriber at the store's total is told `507`, the storage is full of subscriptions —
    /// which it is whoever filled it, so a newcomer holding none is refused too.
    #[tokio::test]
    async fn subscriptions_stored_past_their_quota_are_kept_reported_and_admit_no_more() {
        let (state, store) = test_store::state(100).await;
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let reader = "https://reader.example/#me";
        // The reader holds one more than its own capacity, and with one subscription each of
        // others, everyone but the owner holds one more together than the store's total.
        let others = MAX_SUBSCRIPTIONS + 1 - (MAX_SUBSCRIPTIONS_PER_SUBSCRIBER + 1);
        let subscribers = (0..=MAX_SUBSCRIPTIONS_PER_SUBSCRIBER)
            .map(|_| reader.to_string())
            .chain((0..others).map(|i| format!("https://p{i}.example/#me")));
        for (i, who) in subscribers.enumerate() {
            let sub = Subscription {
                id: String::new(),
                subscriber: Some(who),
                client: None,
                topics: vec![state.cfg.storage()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
            store
                .create_in_container(&container, &format!("{container}s{i:04}"), body, LWS_JSON)
                .await
                .unwrap();
        }
        let state = LwsState::new(store.clone(), state.cfg.clone())
            .await
            .unwrap();
        assert_eq!(
            state.notify.subs.read().unwrap().len(),
            MAX_SUBSCRIPTIONS + 1,
            "all loaded"
        );
        let past = state.notify.past_quota(state.cfg.owner.as_deref()).unwrap();
        assert!(
            past.contains(&format!("{} held", MAX_SUBSCRIPTIONS + 1)),
            "{past}"
        );
        assert!(past.contains("1 author(s)"), "{past}");
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
        let agent = |who: &str| Agent {
            subject: Some(who.into()),
            ..Agent::anonymous()
        };
        let newcomer = agent("https://new.example/#me");
        let r = handle(&state, &post, &agent(reader)).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        let r = handle(&state, &post, &newcomer).await;
        assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);
        // The reader cancels two of its own: it is under its own capacity, and everyone
        // together is under the total.
        for i in 0..2 {
            let delete = test_store::request(
                Method::DELETE,
                &format!("{SUBSCRIPTIONS_PATH}s{i:04}"),
                &[],
                "",
            );
            let r = handle(&state, &delete, &agent(reader)).await;
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
            if i == 0 {
                // At its capacity, not under it: still refused, and the total is still full.
                let r = handle(&state, &post, &agent(reader)).await;
                assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
                let r = handle(&state, &post, &newcomer).await;
                assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);
            }
        }
        assert!(state
            .notify
            .past_quota(state.cfg.owner.as_deref())
            .is_none());
        let r = handle(&state, &post, &newcomer).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        // The newcomer's create filled the total again, so the reader -- under its own
        // capacity, and holding none of the blame -- is told the storage is full, not that it
        // asked for too many.
        let r = handle(&state, &post, &agent(reader)).await;
        assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);
    }

    /// A closed state holding `mine` subscriptions of `alice` to one data resource, and one
    /// subscription each of `others` other subscribers, all stored and then loaded as a start
    /// loads them. Gives back the state, the topic, and alice's subscription IRIs in the order
    /// they were stored.
    async fn state_with_other_subscribers(
        alice: &str,
        mine: usize,
        others: usize,
    ) -> (LwsState<test_store::FlakyStore>, String, Vec<String>) {
        use test_store::{closed_state, data, grant_read};
        let (state, store) = closed_state(100).await;
        let topic = data(&state, "b").await;
        grant_read(&state, alice, &[&topic]).await;
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let mut ours = Vec::new();
        let subscribers = (0..mine)
            .map(|_| alice.to_string())
            .chain((0..others).map(|i| format!("https://p{}.example/#me", i)));
        for (i, subscriber) in subscribers.enumerate() {
            let sub = Subscription {
                id: String::new(),
                subscriber: Some(subscriber.clone()),
                client: None,
                topics: vec![topic.clone()],
                inbox: "https://inbox.example/".into(),
                expires: None,
                expires_at: None,
                failures: 0,
            };
            let iri = format!("{}s{:04}", container, i);
            if subscriber == alice {
                ours.push(iri.clone());
            }
            let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
            store
                .create_in_container(&container, &iri, body, LWS_JSON)
                .await
                .unwrap();
        }
        let cfg = state.cfg.clone();
        let state = LwsState::new(store, cfg).await.unwrap();
        (state, topic, ours)
    }

    /// The members a subscription listing holds, in the order it lists them.
    async fn listed_for(
        state: &LwsState<test_store::FlakyStore>,
        who: &Agent,
    ) -> (StatusCode, Vec<String>) {
        let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
        let resp = handle(state, &req, who).await;
        let status = resp.status();
        let body = test_store::body_json(resp).await;
        let items = body["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        (status, items)
    }

    /// A subscriber's listing and its subscription request are answered from its own records:
    /// how many subscriptions other subscribers hold moves neither what it is shown nor what it
    /// is answered, and its own cancellation is reflected at once. (What everyone holds
    /// together is bounded apart, by [`MAX_SUBSCRIPTIONS`]; here it is well under it.)
    #[tokio::test]
    async fn another_subscribers_records_move_neither_my_listing_nor_my_create() {
        let alice = "https://alice.example/#me";
        let others = MAX_SUBSCRIPTIONS / 8;
        let (state, topic, mine) = state_with_other_subscribers(alice, 2, others).await;
        let me = test_store::agent(alice);
        let (status, listed) = listed_for(&state, &me).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed, mine, "only this subscriber's own are listed");
        // A create is answered the same however many subscriptions everyone else holds.
        let body = json!({
            "type": WEBHOOK,
            "topic": [topic],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = test_store::request(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let created = handle(&state, &post, &me).await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let new = created.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let (_, listed) = listed_for(&state, &me).await;
        assert_eq!(listed.len(), 3);
        assert!(listed.contains(&new), "{:?}", listed);
        // Cancelling one of its own takes it out of what this subscriber is shown, and leaves
        // the rest of its own standing.
        let path = format!(
            "{}{}",
            SUBSCRIPTIONS_PATH,
            new.rsplit('/').next().unwrap_or_default()
        );
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        assert_eq!(
            handle(&state, &delete, &me).await.status(),
            StatusCode::NO_CONTENT
        );
        let (_, listed) = listed_for(&state, &me).await;
        assert_eq!(listed, mine);
    }

    /// A subscriber at its own capacity is refused, and one under it is admitted, whatever
    /// anyone else holds: the capacity is each subscriber's own. Everyone here holds well under
    /// [`MAX_SUBSCRIPTIONS`] together, so the store's own bound decides nothing.
    #[tokio::test]
    async fn a_create_is_refused_at_the_subscribers_own_capacity_whatever_others_hold() {
        let alice = "https://alice.example/#me";
        let others = MAX_SUBSCRIPTIONS / 8;
        let (state, topic, mine) =
            state_with_other_subscribers(alice, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER, others).await;
        assert_eq!(mine.len(), MAX_SUBSCRIPTIONS_PER_SUBSCRIBER);
        let body = json!({
            "type": WEBHOOK,
            "topic": [topic.clone()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = || {
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON)],
                &body,
            )
        };
        let me = test_store::agent(alice);
        let r = handle(&state, &post(), &me).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        // Another subscriber with the same read access, holding one, is admitted: what alice
        // holds is no part of its answer either.
        let bob = "https://bob.example/#me";
        test_store::grant_read(&state, bob, &[&topic]).await;
        let them = test_store::agent(bob);
        for _ in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            assert_eq!(
                handle(&state, &post(), &them).await.status(),
                StatusCode::CREATED
            );
        }
        assert_eq!(
            handle(&state, &post(), &them).await.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "and is refused at its own capacity in turn"
        );
        // Alice cancelling one of its own puts it back under its capacity.
        let path = format!(
            "{}{}",
            SUBSCRIPTIONS_PATH,
            mine[0].rsplit('/').next().unwrap_or_default()
        );
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        assert_eq!(
            handle(&state, &delete, &me).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            handle(&state, &post(), &me).await.status(),
            StatusCode::CREATED
        );
    }

    /// A change's deliveries leave the request's task before any of them is queued: when the
    /// task that made the change goes on, no place in the delivery queue has been taken for it,
    /// and a delivery target that never answers holds up neither that change nor the next. (The
    /// runtime decides when the delivery task runs, so this says the request does not do that
    /// work, not that the deliveries happen after the response reaches the client.)
    #[tokio::test]
    async fn a_changes_deliveries_leave_the_request_task() {
        // An inbox that accepts a delivery and never answers it.
        let app = axum::Router::new().route(
            "/inbox",
            axum::routing::post(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                StatusCode::NO_CONTENT
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.delivery = DeliveryLimits {
            queue: 1,
            workers: 1,
            per_inbox: 1,
        };
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        for i in 0..3 {
            let sub = Subscription {
                id: format!("s{}", i),
                subscriber: Some(format!("https://p{}.example/#me", i)),
                client: None,
                topics: vec![state.cfg.storage()],
                inbox: format!("http://{}/inbox", addr),
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
        let drained = |want: u64| {
            let state = state.clone();
            async move {
                for _ in 0..200 {
                    if state.notify.dropped() >= want {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert_eq!(state.notify.dropped(), want);
            }
        };
        let prepared = state.notify.prepare_at_most(&state, &event, 3).await;
        assert_eq!(prepared.len(), 3);
        state.notify.send(&state, prepared);
        // Nothing was queued on the way out: this task has not yielded since `send` returned,
        // so no delivery can have taken a place or been refused one.
        assert_eq!(state.notify.admitted.load(Ordering::SeqCst), 0);
        assert_eq!(state.notify.dropped(), 0);
        // They follow in the background: one takes the single place, two find none.
        drained(2).await;
        // That one delivery is now out to an inbox that never answers. The next change gets
        // through all the same, and its deliveries leave its task too.
        let prepared = state.notify.prepare_at_most(&state, &event, 3).await;
        assert_eq!(prepared.len(), 3);
        state.notify.send(&state, prepared);
        assert_eq!(state.notify.dropped(), 2, "none queued on the way out");
        drained(5).await;
    }

    /// The store's service pool is the one these bounds are for: it names the same container,
    /// and takes this service's records and nothing else. (That it is large enough is held to
    /// the bounds here at compile time; see the `const _` beside them.) The store spells the
    /// path itself, its module being built where `lws` is not, so the two are held to each
    /// other here.
    #[test]
    fn the_store_pools_what_the_notification_service_bounds() {
        use crate::store::{in_service_pool, SERVICE_POOL_PATH};
        assert_eq!(SERVICE_POOL_PATH, SUBSCRIPTIONS_PATH);
        let cfg = LwsConfig::new("http://localhost:3000");
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        assert!(in_service_pool(&container));
        assert!(in_service_pool(&format!("{}abc", container)));
        assert!(!in_service_pool(&format!("{}doc.txt", cfg.storage())));
        assert!(!in_service_pool(&cfg.absolute(super::super::GRANTS_PATH)));
    }

    /// Subscription records are admitted against a pool of their own, so however many stand, a
    /// write of content is admitted exactly as it would be with none: here more subscriptions
    /// are held than the whole content ceiling, and content writes go on being admitted.
    #[tokio::test]
    async fn subscriptions_past_the_content_ceiling_leave_content_writes_admitted() {
        let content_slots = 32;
        let limits = crate::store::InMemoryStoreLimits::new(16 << 20, content_slots)
            .with_pool(512, 16 << 20);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        let state = LwsState::new(test_store::FlakyStore::with_limits(limits), cfg)
            .await
            .unwrap();
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        // More subscriptions than the whole content ceiling, from as many subscribers as it
        // takes (each is held to its own capacity).
        let wanted = 2 * content_slots;
        let mut made = 0;
        for i in 0..wanted.div_ceil(MAX_SUBSCRIPTIONS_PER_SUBSCRIBER) {
            let who = test_store::agent(&format!("https://p{}.example/#me", i));
            for _ in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                assert_eq!(
                    handle(&state, &req, &who).await.status(),
                    StatusCode::CREATED
                );
                made += 1;
            }
        }
        assert!(made > content_slots, "{made} subscriptions stand");
        assert_eq!(state.notify.subs.read().unwrap().len(), made);
        // Content is written as it would be with none of them.
        for i in 0..4 {
            let iri = format!("{}doc{}", state.cfg.storage(), i);
            state
                .store
                .create_in_container(&state.cfg.storage(), &iri, Bytes::from("x"), "text/plain")
                .await
                .unwrap_or_else(|e| panic!("content write refused with {made} subscriptions: {e}"));
        }
    }

    /// Content whose name only holds the pool's segments is content, held to its ceilings.
    #[tokio::test]
    async fn content_named_like_the_pool_is_held_to_the_content_ceilings() {
        // Few content entries, a roomy pool: content whose name merely holds the pool's segments
        // fills the content ceiling, not the pool.
        let limits = crate::store::InMemoryStoreLimits::new(16 << 20, 8).with_pool(512, 16 << 20);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let state = LwsState::new(test_store::FlakyStore::with_limits(limits), cfg)
            .await
            .unwrap();
        let root = state.cfg.storage();
        let mut refused = false;
        for i in 0..16 {
            let iri = format!("{root}alice/.lws/subscriptions/x{i}");
            if state
                .store
                .write(&iri, Bytes::from_static(b"x"), "text/plain")
                .await
                .is_err()
            {
                refused = true;
                break;
            }
        }
        assert!(refused, "lookalike content escaped the content ceiling");
    }

    /// The subscription pool is bounded in its own right: once it is full a subscription create
    /// is refused `507`, and content writes are admitted all the same.
    #[tokio::test]
    async fn a_full_subscription_pool_refuses_only_subscriptions() {
        // Room for the service container and two subscription records.
        let limits = crate::store::InMemoryStoreLimits::new(16 << 20, 64).with_pool(3, 16 << 20);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        let state = LwsState::new(test_store::FlakyStore::with_limits(limits), cfg)
            .await
            .unwrap();
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |who: Agent| {
            let (state, body) = (state.clone(), body.clone());
            async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                handle(&state, &req, &who).await.status()
            }
        };
        let me = test_store::agent("https://alice.example/#me");
        let mut created = 0;
        let mut full = None;
        for _ in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            match post(me.clone()).await {
                StatusCode::CREATED => created += 1,
                other => {
                    full = Some(other);
                    break;
                }
            }
        }
        assert!(created > 0, "the pool held nothing at all");
        assert_eq!(
            full,
            Some(StatusCode::INSUFFICIENT_STORAGE),
            "a full pool refuses a subscription create"
        );
        // The pool is full and this subscriber is nowhere near its own capacity; content is
        // written all the same.
        assert!(created < MAX_SUBSCRIPTIONS_PER_SUBSCRIBER);
        let iri = format!("{}doc", state.cfg.storage());
        state
            .store
            .create_in_container(&state.cfg.storage(), &iri, Bytes::from("x"), "text/plain")
            .await
            .expect("content write refused while the subscription pool is full");
    }

    /// A subscription create waits only on its own expired subscriptions: at most
    /// [`EXPIRED_REMOVED_PER_SUBSCRIBE`] of them, all its own, go before it is answered.
    /// Everyone else's are swept on a task of their own, which this request does not wait for.
    #[tokio::test]
    async fn a_create_waits_only_on_its_own_expired_subscriptions() {
        let (state, _store) = test_store::state(100).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |who: &Agent| {
            let (state, body, who) = (state.clone(), body.clone(), who.clone());
            async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                handle(&state, &req, &who).await.status()
            }
        };
        let (me, them) = (
            test_store::agent("https://alice.example/#me"),
            test_store::agent("https://bob.example/#me"),
        );
        assert_eq!(post(&me).await, StatusCode::CREATED);
        for _ in 0..3 {
            assert_eq!(post(&them).await, StatusCode::CREATED);
        }
        // Let the sweeps those creates left behind finish before anything expires, so what is
        // removed from here on is this test's doing and not theirs.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Everyone's subscriptions expire.
        let (mine, theirs): (Vec<String>, Vec<String>) = {
            let mut subs = state.notify.subs.write().unwrap();
            for s in subs.values_mut() {
                s.expires_at = Some(1);
            }
            let ids: Vec<(String, Option<String>)> = subs
                .values()
                .map(|s| (s.id.clone(), s.subscriber.clone()))
                .collect();
            let mine = ids
                .iter()
                .filter(|(_, who)| who.as_deref() == me.subject.as_deref())
                .map(|(id, _)| id.clone())
                .collect();
            let theirs = ids
                .iter()
                .filter(|(_, who)| who.as_deref() == them.subject.as_deref())
                .map(|(id, _)| id.clone())
                .collect();
            (mine, theirs)
        };
        assert_eq!((mine.len(), theirs.len()), (1, 3));
        // A create by this subscriber takes its own expired subscription with it. Nothing has
        // yielded between the create being answered and this check, so nothing else can have
        // run: the other subscriber's are all still standing, none of them removed on this
        // request's way through.
        assert_eq!(post(&me).await, StatusCode::CREATED);
        let standing = |ids: &[String]| {
            let subs = state.notify.subs.read().unwrap();
            ids.iter().filter(|id| subs.contains_key(id)).count()
        };
        assert_eq!(standing(&mine), 0, "its own went with its create");
        assert_eq!(
            standing(&theirs),
            theirs.len(),
            "another subscriber's were removed on this request"
        );
        // They are swept on a task of their own, which the request did not wait for.
        for _ in 0..200 {
            if standing(&theirs) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(standing(&theirs), 0, "the sweep never ran");
    }

    /// A subscriber's conditional create holds its own listing alone, not the container's: a
    /// removal of another subscriber's subscription stalled in the store, or a sweep of expired
    /// subscriptions stalled the same way, neither delays it nor makes it `503`. A removal of
    /// one of its own subscriptions, which changes its own listing, still keeps it waiting.
    #[tokio::test(start_paused = true)]
    async fn a_conditional_create_waits_only_on_its_own_listing() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, store) = closed_state(100).await;
        // Swept by this test alone.
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let topic = data(&state, "doc").await;
        let (alice, bob) = (
            agent("https://alice.example/#me"),
            agent("https://bob.example/#me"),
        );
        for who in [&alice, &bob] {
            grant_read(&state, who.subject.as_deref().unwrap(), &[&topic]).await;
        }
        let body = json!({
            "type": WEBHOOK,
            "topic": [topic],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |who: &Agent, headers: Vec<(&'static str, String)>| {
            let (state, body, who) = (state.clone(), body.clone(), who.clone());
            async move {
                let mut h = vec![("content-type", LWS_JSON.to_string())];
                h.extend(headers);
                let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let req = test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body);
                let r = handle(&state, &req, &who).await;
                (
                    r.status(),
                    r.headers()
                        .get(header::LOCATION)
                        .map(|l| l.to_str().unwrap().to_string()),
                )
            }
        };
        let tag = |who: &Agent| {
            let (state, who) = (state.clone(), who.clone());
            async move {
                let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
                let r = test_store::route_as(&state, req, &who).await;
                r.headers()["etag"].to_str().unwrap().to_string()
            }
        };
        let gated = |iri: &str| {
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *store.hold_next_delete_of.lock().unwrap() = Some((iri.to_string(), gate.clone()));
            gate
        };
        let reached = || async {
            for _ in 0..1000 {
                if store.hold_next_delete_of.lock().unwrap().is_none() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            panic!("the removal never reached the store");
        };
        let (status, mine) = post(&alice, vec![]).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, theirs) = post(&bob, vec![]).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, expired) = post(&bob, vec![]).await;
        assert_eq!(status, StatusCode::CREATED);
        let (mine, theirs, expired) = (mine.unwrap(), theirs.unwrap(), expired.unwrap());

        // Bob's removal of his own subscription stalls in the store, holding his listing.
        let gate = gated(&theirs);
        let removal = {
            let (state, bob) = (state.clone(), bob.clone());
            let path = theirs
                .strip_prefix(&state.cfg.base_url)
                .unwrap()
                .to_string();
            tokio::spawn(async move {
                let req = test_store::request(Method::DELETE, &path, &[], "");
                handle(&state, &req, &bob).await.status()
            })
        };
        reached().await;
        let started = tokio::time::Instant::now();
        let (status, _) = post(&alice, vec![("if-match", tag(&alice).await)]).await;
        assert_eq!(status, StatusCode::CREATED, "held up by another's removal");
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "waited on it"
        );
        gate.add_permits(1);
        assert_eq!(removal.await.unwrap(), StatusCode::NO_CONTENT);

        // A sweep of Bob's expired subscription stalls in the store.
        let id = expired.rsplit('/').next().unwrap().to_string();
        state
            .notify
            .subs
            .write()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .expires_at = Some(1);
        let gate = gated(&expired);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        reached().await;
        let started = tokio::time::Instant::now();
        let (status, _) = post(&alice, vec![("if-match", tag(&alice).await)]).await;
        assert_eq!(status, StatusCode::CREATED, "held up by a sweep");
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "waited on it"
        );
        gate.add_permits(1);
        sweeper.settled().await;
        assert!(!state.notify.subs.read().unwrap().contains_key(&id));

        // Alice's own removal stalled the same way keeps her conditional create waiting: it
        // changes the listing she compares against.
        let gate = gated(&mine);
        let removal = {
            let (state, alice) = (state.clone(), alice.clone());
            let path = mine.strip_prefix(&state.cfg.base_url).unwrap().to_string();
            tokio::spawn(async move {
                let req = test_store::request(Method::DELETE, &path, &[], "");
                handle(&state, &req, &alice).await.status()
            })
        };
        reached().await;
        let etag = tag(&alice).await;
        let (status, _) = post(&alice, vec![("if-match", etag)]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        gate.add_permits(1);
        assert_eq!(removal.await.unwrap(), StatusCode::NO_CONTENT);
    }

    /// `n` expired subscriptions, of `n` subscribers, put in force directly; their ids, in order.
    fn expired_subscriptions<S: Store>(state: &LwsState<S>, n: usize) -> Vec<String> {
        let mut subs = state.notify.subs.write().unwrap();
        (0..n)
            .map(|i| {
                let id = format!("x{i:04}");
                subs.insert(
                    id.clone(),
                    Subscription {
                        id: id.clone(),
                        subscriber: Some(format!("https://r{i}.example/#me")),
                        client: None,
                        topics: vec![state.cfg.storage()],
                        inbox: "https://inbox.example/".into(),
                        expires: None,
                        expires_at: Some(1),
                        failures: 0,
                    },
                );
                id
            })
            .collect()
    }

    /// Let every task that can run, run (on a paused clock).
    async fn run_ready() {
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
    }

    /// Expired subscriptions go whatever requests come: with the totals full of them and every
    /// create refused, the expiry sweeper still removes them, a bounded number per sweep, and
    /// their places come back.
    #[tokio::test(start_paused = true)]
    async fn expired_subscriptions_are_swept_without_any_create() {
        use super::super::records::MAINTENANCE_PLACES;
        let (state, _store) = test_store::state(100).await;
        expired_subscriptions(&state, MAX_SUBSCRIPTIONS);
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = || async {
            let req = test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &[("content-type", LWS_JSON)],
                &body,
            );
            handle(&state, &req, &test_store::agent("https://new.example/#me"))
                .await
                .status()
        };
        // The totals are full of expired subscriptions, and a create is refused.
        assert_eq!(post().await, StatusCode::INSUFFICIENT_STORAGE);
        let sweep = || async {
            tokio::time::sleep(EXPIRY_SWEEP_EVERY).await;
            run_ready().await;
        };
        // No create goes through, yet a sweep comes, and removes a bounded number.
        sweep().await;
        assert_eq!(
            state.notify.subs.read().unwrap().len(),
            MAX_SUBSCRIPTIONS - MAINTENANCE_PLACES,
            "one sweep is bounded"
        );
        // Further sweeps take the rest, and the places come back.
        for _ in 0..MAX_SUBSCRIPTIONS.div_ceil(MAINTENANCE_PLACES) {
            sweep().await;
        }
        assert!(state.notify.subs.read().unwrap().is_empty(), "never swept");
        assert_eq!(post().await, StatusCode::CREATED);
    }

    /// One of the sweeper's removals that never completes in the store holds up no other: the
    /// rest go meanwhile, it is given up past its deadline, and tried again later.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_removal_holds_up_no_other() {
        let (state, store) = test_store::state(100).await;
        let ids = expired_subscriptions(&state, 3);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let stuck = format!("{container}{}", ids[0]);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((stuck, gate.clone()));
        tokio::time::sleep(EXPIRY_SWEEP_EVERY).await;
        run_ready().await;
        {
            let subs = state.notify.subs.read().unwrap();
            assert!(subs.contains_key(&ids[0]), "the stuck one went");
            assert_eq!(subs.len(), 1, "the others waited on it");
        }
        // Given up past its deadline (its store call still pending), it is tried again once its
        // wait is over, and goes.
        tokio::time::sleep(SWEEP_REMOVAL_DEADLINE + SWEEP_BACKOFF_FIRST + EXPIRY_SWEEP_EVERY).await;
        run_ready().await;
        tokio::time::sleep(EXPIRY_SWEEP_EVERY).await;
        run_ready().await;
        assert!(
            state.notify.subs.read().unwrap().is_empty(),
            "never tried again"
        );
        drop(gate);
    }

    /// The sweeper keeps no state alive: once every other handle is gone, the state goes, even
    /// while one of its removals never completes in the store.
    #[tokio::test(start_paused = true)]
    async fn the_state_goes_with_its_handles_while_a_removal_stalls() {
        let (state, store) = test_store::state(100).await;
        let ids = expired_subscriptions(&state, 1);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let stuck = format!("{container}{}", ids[0]);
        *store.hold_next_delete_of.lock().unwrap() = Some((stuck, gate.clone()));
        tokio::time::sleep(EXPIRY_SWEEP_EVERY).await;
        run_ready().await;
        assert!(
            store.hold_next_delete_of.lock().unwrap().is_none(),
            "the removal never started"
        );
        let weak = state.downgrade();
        drop(state);
        tokio::time::sleep(SWEEP_REMOVAL_DEADLINE + EXPIRY_SWEEP_EVERY).await;
        run_ready().await;
        assert!(weak.upgrade().is_none(), "the state outlived its handles");
        drop(gate);
    }

    /// Review findings: a topic's lock was waited for without bound, so a write stalled on a
    /// topic held a subscription request (and its admission) for as long as it stalled; and a
    /// quarantined subscription was deleted in the request, waiting on its record's lock without
    /// bound. Both wait a bounded time, then answer `503`.
    #[tokio::test]
    async fn subscription_requests_wait_a_bounded_time_for_locks() {
        let (state, store) = test_store::state(100).await;
        let topic = state.cfg.storage();
        let body = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": WEBHOOK,
            "topic": [topic.clone()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = test_store::request(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let held = state.locks.lock(&topic).await.expect("held");
        let r = handle(&state, &post, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(held);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}bad");
        store
            .create_in_container(&container, &iri, Bytes::from("[]"), LWS_JSON)
            .await
            .unwrap();
        state
            .notify
            .load_with(&store, &state.cfg, |_| true)
            .await
            .unwrap();
        let delete =
            test_store::request(Method::DELETE, &format!("{SUBSCRIPTIONS_PATH}bad"), &[], "");
        let held = state.locks.lock(&iri).await.expect("held");
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(store.exists(&iri).await.unwrap());
        drop(held);
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
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
            "(\"@method\" \"@scheme\" \"@authority\" \"@path\" \"@query\" \"content-type\" \"content-digest\");created=1700000000;keyid=\"https://s/#notify-key\";alg=\"ecdsa-p256-sha256\""
        );
        let base = signature_base(&target, LWS_JSON, "sha-256=:abc=:", &params);
        let expected = format!(
            "\"@method\": POST\n\"@scheme\": https\n\"@authority\": receiver.example\n\"@path\": /hooks/lws\n\"@query\": ?x=1\n\
             \"content-type\": application/lws+json\n\"content-digest\": sha-256=:abc=:\n\"@signature-params\": {params}"
        );
        assert_eq!(base, expected);
        let local = url::Url::parse("http://localhost:3938").unwrap();
        assert!(signature_base(&local, LWS_JSON, "d", "p")
            .contains("\"@authority\": localhost:3938\n\"@path\": /\n\"@query\": ?\n"));
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

    /// Issue #6814: a webhook subscription took each topic's lock (waiting while a write held
    /// it) and answered `503` on a set-aside one, before it checked the subscriber may read the
    /// topic: a subscriber that may not read it could tell a stuck change to it, or a write in
    /// progress, from a resource it may not read. Whoever may not read a topic is refused at
    /// once, the same whatever its state; only whoever may is told to retry, or waits.
    #[tokio::test]
    async fn a_topic_is_authorized_before_its_lock_or_state_shows() {
        use test_store::{agent, closed_state, data, grant_read, stick, OWNER};
        let (state, store) = closed_state(100).await;
        let (a, b) = (data(&state, "a").await, data(&state, "b").await);
        let reader = agent("https://reader.example/#me");
        let stranger = agent("https://stranger.example/#me");
        grant_read(&state, "https://reader.example/#me", &[&a, &b]).await;
        let subscribe = |topic: &str, who: &Agent| {
            let body = json!({
                "type": WEBHOOK,
                "topic": [topic],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let (state, who) = (state.clone(), who.clone());
            async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                let r = tokio::time::timeout(
                    Duration::from_millis(500),
                    test_store::route_as(&state, req, &who),
                )
                .await
                .ok()?;
                let retry = r.headers().contains_key(header::RETRY_AFTER);
                let status = r.status();
                let body = axum::body::to_bytes(r.into_body(), usize::MAX)
                    .await
                    .unwrap();
                Some((status, retry, body))
            }
        };
        let refused = subscribe(&b, &stranger).await.expect("answered at once");
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        // A write holds b's lock: the stranger is refused at once, as before; the reader waits.
        let held = state.locks.lock(&b).await.unwrap();
        assert_eq!(subscribe(&b, &stranger).await.as_ref(), Some(&refused));
        assert_eq!(
            subscribe(&b, &reader).await,
            None,
            "the reader waits on the write"
        );
        drop(held);
        // a is set aside: the stranger is refused as for b; the reader is told to retry.
        stick(&state, &store, &a).await;
        assert_eq!(subscribe(&a, &stranger).await.as_ref(), Some(&refused));
        let got = subscribe(&a, &reader).await.expect("answered at once");
        assert_eq!(got.0, StatusCode::SERVICE_UNAVAILABLE);
        assert!(got.1);
        assert_eq!(
            subscribe(&a, &agent(OWNER)).await.expect("answered").0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        // Anonymous, the challenge, whatever the state.
        assert_eq!(
            subscribe(&a, &Agent::anonymous())
                .await
                .expect("answered")
                .0,
            StatusCode::UNAUTHORIZED
        );
        let ok = subscribe(&b, &reader).await.expect("answered");
        assert_eq!(ok.0, StatusCode::CREATED);
    }

    /// Issue #6883: a topic a journaled change is in flight to (its lock held) is decided on
    /// what it was before the change began, as a request to it is: what is stored may be the
    /// change part way, or being put back. A subscriber that only the stored state lets read it
    /// is refused at once rather than left to wait on the lock.
    #[tokio::test]
    async fn a_topic_being_changed_is_decided_by_what_it_was_before() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, _store) = closed_state(100).await;
        let b = data(&state, "b").await;
        let reader = agent("https://reader.example/#me");
        grant_read(&state, "https://reader.example/#me", &[&b]).await;
        let subscribe = || {
            let body = json!({
                "type": WEBHOOK,
                "topic": [b.clone()],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let (state, who) = (state.clone(), reader.clone());
            async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                tokio::time::timeout(
                    Duration::from_millis(500),
                    test_store::route_as(&state, req, &who),
                )
                .await
                .ok()
                .map(|r| r.status())
            }
        };
        // A change holds b's lock, and before it began b's metadata could not be read (so every
        // grant fails closed on it): the reader is refused at once.
        let held = state.locks.lock(&b).await.unwrap();
        let before = super::super::Prior {
            meta: Some(super::super::ResourceMeta {
                pending: true,
                ..super::super::ResourceMeta::default()
            }),
            ..Default::default()
        };
        state
            .locks
            .changing_from([(b.clone(), before)].into_iter().collect());
        assert_eq!(subscribe().await, Some(StatusCode::FORBIDDEN));
        state.locks.landed(std::slice::from_ref(&b));
        assert_eq!(subscribe().await, None, "the reader waits on the change");
        drop(held);
        assert_eq!(subscribe().await, Some(StatusCode::CREATED));
    }

    /// Review of #6925: a change can begin while the topic check reads what is stored without
    /// the lock. Whether one is in flight is checked again once the reads are over, and the
    /// topic is then decided on what it was before the change.
    #[tokio::test]
    async fn a_change_that_begins_during_a_topic_check_decides_it() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, store) = closed_state(100).await;
        let b = data(&state, "b").await;
        grant_read(&state, "https://reader.example/#me", &[&b]).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_read_reply_of.lock().unwrap() =
            Some((super::super::meta_key(&b), gate.clone()));
        let subscribing = {
            let body = json!({
                "type": WEBHOOK,
                "topic": [b.clone()],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let (state, who) = (state.clone(), agent("https://reader.example/#me"));
            tokio::spawn(async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                tokio::time::timeout(
                    Duration::from_millis(500),
                    test_store::route_as(&state, req, &who),
                )
                .await
                .ok()
                .map(|r| r.status())
            })
        };
        // The topic's lock is held, so the check reads without it; it has read the metadata.
        let held = state.locks.lock(&b).await.unwrap();
        while store.hold_next_read_reply_of.lock().unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // The change begins, and before it b's metadata could not be read (so every grant fails
        // closed on it).
        let before = super::super::Prior {
            meta: Some(super::super::ResourceMeta {
                pending: true,
                ..super::super::ResourceMeta::default()
            }),
            ..Default::default()
        };
        state
            .locks
            .changing_from([(b.clone(), before)].into_iter().collect());
        gate.add_permits(1);
        assert_eq!(subscribing.await.unwrap(), Some(StatusCode::FORBIDDEN));
        state.locks.landed(std::slice::from_ref(&b));
        drop(held);
    }

    /// Review of #6925: the topic check can read a change's state part way and have the reply
    /// come back once the change is over; it is made again, and a subscriber the topic allows
    /// before and after subscribes.
    #[tokio::test]
    async fn a_change_over_by_the_end_of_a_topic_check_has_it_made_again() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, store) = closed_state(100).await;
        let b = data(&state, "b").await;
        grant_read(&state, "https://reader.example/#me", &[&b]).await;
        let before = state.resource_meta(&b).await.unwrap();
        let mut part_way = before.clone();
        part_way.pending = true;
        state.put_resource_meta(&b, &part_way).await.unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_read_reply_of.lock().unwrap() =
            Some((super::super::meta_key(&b), gate.clone()));
        let held = state.locks.lock(&b).await.unwrap();
        let subscribing = {
            let body = json!({
                "type": WEBHOOK,
                "topic": [b.clone()],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let (state, who) = (state.clone(), agent("https://reader.example/#me"));
            tokio::spawn(async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                tokio::time::timeout(
                    Duration::from_millis(500),
                    test_store::route_as(&state, req, &who),
                )
                .await
                .ok()
                .map(|r| r.status())
            })
        };
        while store.hold_next_read_reply_of.lock().unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let prior = super::super::Prior {
            meta: Some(before.clone()),
            ..Default::default()
        };
        state
            .locks
            .changing_from([(b.clone(), prior)].into_iter().collect());
        state.put_resource_meta(&b, &before).await.unwrap();
        state.locks.landed(std::slice::from_ref(&b));
        drop(held);
        gate.add_permits(1);
        assert_eq!(subscribing.await.unwrap(), Some(StatusCode::CREATED));
    }

    /// Issue #6814: the service routes answered `503` for a set-aside record before they
    /// checked the caller may see it. A subscription (or a grant) is said to be set aside only
    /// to its subscriber (its author) and the owner; to anyone else it is as any record they
    /// may not see.
    #[tokio::test]
    async fn a_set_aside_record_shows_only_to_who_may_see_it() {
        use test_store::{agent, closed_state, stick, OWNER};
        let (state, store) = closed_state(100).await;
        let reader = agent("https://reader.example/#me");
        let stranger = agent("https://stranger.example/#me");
        let owner = agent(OWNER);
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
        let r = handle(&state, &post, &owner).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let iri = r.headers()[header::LOCATION].to_str().unwrap().to_string();
        let path = iri
            .strip_prefix(state.cfg.storage().trim_end_matches('/'))
            .unwrap()
            .to_string();
        let ask = |path: &str, m: Method, who: &Agent| {
            let (state, who) = (state.clone(), who.clone());
            let req = test_store::request(m, path, &[], "");
            async move { test_store::route_as(&state, req, &who).await.status() }
        };
        stick(&state, &store, &iri).await;
        let unknown = format!("{SUBSCRIPTIONS_PATH}unknown");
        for m in [Method::GET, Method::DELETE] {
            for who in [&reader, &stranger] {
                assert_eq!(
                    ask(&path, m.clone(), who).await,
                    StatusCode::NOT_FOUND,
                    "{m}"
                );
                assert_eq!(ask(&unknown, m.clone(), who).await, StatusCode::NOT_FOUND);
            }
            assert_eq!(
                ask(&path, m.clone(), &Agent::anonymous()).await,
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                ask(&path, m.clone(), &owner).await,
                StatusCode::SERVICE_UNAVAILABLE,
                "{m}"
            );
        }
        // The container lists it, so it is set aside too: any subscriber may act on it.
        let container = SUBSCRIPTIONS_PATH;
        assert_eq!(
            ask(container, Method::GET, &Agent::anonymous()).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ask(container, Method::GET, &reader).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        // A grant record, likewise: only its author and the owner hear it is set aside.
        test_store::unstick(&store);
        test_store::put_back(&state, &iri).await;
        let grant = test_store::grant_read(
            &state,
            "https://reader.example/#me",
            &[&state.cfg.storage()],
        )
        .await;
        let grant_path = grant
            .strip_prefix(state.cfg.storage().trim_end_matches('/'))
            .unwrap()
            .to_string();
        stick(&state, &store, &grant).await;
        let ask_grant = |who: &Agent| {
            let (state, who) = (state.clone(), who.clone());
            let req = test_store::request(Method::GET, &grant_path, &[], "");
            async move { test_store::route_as(&state, req, &who).await.status() }
        };
        assert_eq!(ask_grant(&reader).await, StatusCode::FORBIDDEN);
        assert_eq!(ask_grant(&stranger).await, StatusCode::FORBIDDEN);
        assert_eq!(ask_grant(&owner).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Issue #6814 (review): a subscription POST said the subscriptions container was set
    /// aside, or that new changes waited on set-aside ones, and removed expired subscriptions
    /// (waiting on their locks), before it checked the subscriber may read its topics. A topic
    /// it may not read is refused first, at once, whatever the container's state.
    #[tokio::test]
    async fn a_subscription_is_authorized_by_its_topics_before_anything_else() {
        use test_store::{agent, closed_state, data, grant_read, route_as, stick, OWNER};
        let (state, store) = closed_state(100).await;
        let (a, b) = (data(&state, "a").await, data(&state, "b").await);
        let reader = agent("https://reader.example/#me");
        grant_read(&state, "https://reader.example/#me", &[&a]).await;
        let post = |topic: &str, who: &Agent| {
            let body = json!({
                "type": WEBHOOK,
                "topic": [topic],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let (state, who) = (state.clone(), who.clone());
            async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                let r =
                    tokio::time::timeout(Duration::from_millis(500), route_as(&state, req, &who))
                        .await
                        .expect("answered at once");
                let retry = r.headers().contains_key(header::RETRY_AFTER);
                (r.status(), retry)
            }
        };
        let refused = (StatusCode::FORBIDDEN, false);
        assert_eq!(post(&b, &reader).await, refused);
        // An expired subscription, its record held by a removal in progress: a request that
        // removed it first would wait on it.
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let old = format!("{container}old");
        let sub = Subscription {
            id: "old".into(),
            subscriber: Some(OWNER.into()),
            client: None,
            topics: vec![state.cfg.storage()],
            inbox: "https://inbox.example/".into(),
            expires: None,
            expires_at: Some(1),
            failures: 0,
        };
        state
            .store
            .create_in_container(
                &container,
                &old,
                Bytes::from(serde_json::to_vec(&sub).unwrap()),
                LWS_JSON,
            )
            .await
            .unwrap();
        state.notify.subs.write().unwrap().insert("old".into(), sub);
        let held = state.locks.lock(&old).await.unwrap();
        assert_eq!(post(&b, &reader).await, refused);
        drop(held);
        // The container is set aside (a stuck change to a subscription in it).
        stick(&state, &store, &format!("{container}stuck")).await;
        assert!(!state.visible(&container));
        assert_eq!(post(&b, &reader).await, refused);
        assert_eq!(
            post(&a, &reader).await,
            (StatusCode::SERVICE_UNAVAILABLE, true)
        );
        test_store::unstick(&store);
        test_store::put_back(&state, &container).await;
        // New changes wait while set-aside ones hold as much as they may.
        state.set_aside_bytes.store(
            super::super::MAX_SET_ASIDE_BYTES,
            std::sync::atomic::Ordering::Release,
        );
        assert_eq!(post(&b, &reader).await, refused);
        assert_eq!(
            post(&a, &reader).await,
            (StatusCode::SERVICE_UNAVAILABLE, true)
        );
        state
            .set_aside_bytes
            .store(0, std::sync::atomic::Ordering::Release);
        assert_eq!(post(&a, &reader).await.0, StatusCode::CREATED);
    }
}

//! Notifications (LWS 1.0 core section 10) and the webhook subscription type
//! (lws10-notifications-webhook).
//!
//! The NotificationService lives at [`SUBSCRIPTIONS_PATH`] and offers
//! `WebhookSubscription`, `SseSubscription` and `WebSocketSubscription`. An authenticated agent
//! subscribes with an `application/lws+json` request naming a `topic` (an array of resource URIs,
//! each of which it must be able to read now). A topic covers itself and, for a container,
//! everything beneath it. Each Create, Update and Delete inside a topic is delivered as an
//! `application/lws+json` Notification, provided the subscriber may read the resource when the
//! event occurs (section 10.3.3). Webhooks name an inbox; streams use capability URLs.
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
//!
//! Expiry is decided by the clock and a stored wall-clock high-water mark together, and nothing
//! outside sees a subscription expire until a time at or past its expiry is stored (see
//! `HighWater`). When that time cannot be stored, the service halts: no delivery, no request
//! answered but `503`, no subscription removed, until a raise to the time owed goes through:
//! the expiry sweeper alone retries it, waiting longer after each failure. A mark far ahead of
//! the clock stands; only a start in the recovery mode lowers it
//! ([`LwsConfig::reset_subscription_clock`]). A halt shows no expiry, so a subscription that was
//! never durably expired may be delivered to again after a restart whose clock was stepped back.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

mod streams;
pub(super) use streams::{connect, stream_path};

/// The webhook subscription type this NotificationService offers.
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
    kind: SubscriptionKind,
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
    #[cfg(feature = "crdt-sync")]
    crdt_create: Option<(String, String)>,
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
        if !state
            .notify
            .stands_now(&state.store, &self.subscription)
            .await
        {
            return false;
        }
        let allowed = if let Some(author) = &self.service {
            reads_service_record(state, &self.agent, author)
        } else {
            match &self.snapshot {
                Some(s) => access::allowed_as(state, Action::Read, &self.uri, &self.agent, s).await,
                None => {
                    // A resource that is not visible is not read (its lock is held until it is
                    // put back): the delivery fails its check rather than wait, including when
                    // the resource is set aside while the delivery waits for its lock.
                    let Some(_guard) = state.locks.read(&self.uri).await else {
                        return false;
                    };
                    // The subscription may have been cancelled, or expired, meanwhile: that is
                    // decided once, after the access decision, below.
                    state.allowed(Action::Read, &self.uri, &self.agent).await
                }
            }
        };
        // Deciding may have waited (on a lock, or a read of metadata): the subscription may have
        // been cancelled, or expired, meanwhile, which is decided durably here. Nothing waits
        // between this and the send.
        allowed
            && state
                .notify
                .stands_now(&state.store, &self.subscription)
                .await
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

/// The transport of a notification subscription.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionKind {
    /// An outgoing HTTP delivery; its record retains the original inbox field.
    #[default]
    Webhook,
    /// An incoming EventSource connection.
    Sse,
    /// An incoming WebSocket connection.
    WebSocket,
}

impl SubscriptionKind {
    fn name(self) -> &'static str {
        match self {
            Self::Webhook => WEBHOOK,
            Self::Sse => streams::SSE,
            Self::WebSocket => streams::WEBSOCKET,
        }
    }
}

/// One notification subscription, as stored.
///
/// Legacy webhook records omit kind and capability_hash and round-trip unchanged.
#[derive(Debug, Clone, Deserialize, PartialEq)]
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
    #[serde(default)]
    pub inbox: String,
    /// Delivery transport; omitted from legacy webhook records.
    #[serde(default)]
    pub kind: SubscriptionKind,
    /// SHA-256 of the capability token; the token itself is never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_hash: Option<[u8; 32]>,
    /// The expiry the subscriber asked for, as given, and as seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Consecutive failed deliveries, in memory only. Kept in the subscription itself, so it goes
    /// when the subscription goes, and a delivery that finishes later finds nothing to count.
    #[serde(skip)]
    pub failures: u32,
    /// Whether it has been seen expired, in memory only ([`Lapsed`]).
    #[serde(skip)]
    pub lapsed: Lapsed,
}

// Keep the old webhook field order and omission rules, including an empty inbox, exactly.
// Stream records have their own shape and do not serialize an inbox.
impl Serialize for Subscription {
    fn serialize<T: serde::Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        if let Some(subscriber) = &self.subscriber {
            map.serialize_entry("subscriber", subscriber)?;
        }
        if let Some(client) = &self.client {
            map.serialize_entry("client", client)?;
        }
        map.serialize_entry("topics", &self.topics)?;
        if self.kind == SubscriptionKind::Webhook {
            map.serialize_entry("inbox", &self.inbox)?;
        } else {
            map.serialize_entry("kind", &self.kind)?;
        }
        if let Some(hash) = &self.capability_hash {
            map.serialize_entry("capability_hash", hash)?;
        }
        if let Some(expires) = &self.expires {
            map.serialize_entry("expires", expires)?;
        }
        if let Some(at) = &self.expires_at {
            map.serialize_entry("expires_at", at)?;
        }
        map.end()
    }
}

/// Set once a [`Subscription`] is seen expired, shared by its copies: from then on it stays
/// expired, whatever the clock says later.
#[derive(Debug, Clone, Default)]
pub struct Lapsed(Arc<AtomicBool>);

impl PartialEq for Lapsed {
    /// What a subscription is, not what has been seen of it.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Subscription {
    /// Whether it has expired by `high_water`, the stored high water ([`HighWater`]), or was
    /// seen expired before: expiry is never undone, so a clock stepped back cannot list again
    /// one whose removal began while it was expired (that removal holds no listing's lock, and
    /// would then change a listing a conditional create holds).
    fn expired(&self, high_water: i64) -> bool {
        if self.lapsed.0.load(Ordering::Acquire) {
            return true;
        }
        let expired = self.expires_at.is_some_and(|t| t <= high_water);
        if expired {
            self.lapsed.0.store(true, Ordering::Release);
        }
        expired
    }

    /// Whether it has expired by `now` or by `high_water`, whichever is later: [`expired`], or
    /// past its expiry by the clock and not yet by the stored high water, in which case its
    /// expiry is shown only once `now` is stored ([`Notifier::decide`]).
    ///
    /// [`expired`]: Subscription::expired
    fn lapsing(&self, high_water: i64, now: i64) -> bool {
        self.expired(high_water) || self.expires_at.is_some_and(|t| t <= now)
    }

    /// Whether it is past its expiry by the clock `now` but not by `high_water`: its expiry has
    /// yet to be stored before anything outside may see it.
    fn due(&self, high_water: i64, now: i64) -> bool {
        !self.expired(high_water) && self.expires_at.is_some_and(|t| t <= now)
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
/// count of its own capacity read ([`Subs::mine`], [`Subs::live_count_of`]): each reads that
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

    /// How many unexpired subscriptions `subscriber` holds, read through the subscriber index as
    /// its listing reads ([`Subs::mine`]).
    fn live_count_of(&self, subscriber: Option<&str>, high_water: i64) -> usize {
        self.mine(subscriber)
            .filter(|s| !s.expired(high_water))
            .count()
    }

    /// How many subscriptions `subscriber` holds, expired ones included: the size of its index
    /// entry, so no record (its own or anyone else's) is read to count them.
    fn count_of(&self, subscriber: Option<&str>) -> usize {
        self.by_subscriber
            .get(&subscriber.map(str::to_string))
            .map_or(0, |ids| ids.len())
    }

    /// The ids of `subscriber`'s own subscriptions expired by `high_water` or by `now`
    /// ([`Subscription::lapsing`]), at most `most` of them, read through the subscriber index: no
    /// other subscriber's record is touched.
    fn expired_of(
        &self,
        subscriber: Option<&str>,
        high_water: i64,
        now: i64,
        most: usize,
    ) -> Vec<String> {
        self.mine(subscriber)
            .filter(|s| s.lapsing(high_water, now))
            .take(most)
            .map(|s| s.id.clone())
            .collect()
    }

    /// `subscriber`'s own subscriptions, in id order, read through the subscriber index: no other
    /// subscriber's record is touched.
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
    #[cfg(feature = "crdt-sync")]
    pub(crate) crdt_changes: sparq_lws_ysync::Changes,
    subs: RwLock<Subs>,
    streams: streams::Streams,
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
    sweeper_off: Arc<AtomicBool>,
    /// Stored subscriptions the start did not load because a change to them was still being
    /// settled, or their removal failed. Each is counted against both shares until the next
    /// start, as it may still be stored.
    held_back: AtomicUsize,
    /// Stored subscriptions whose stored body is not a subscription, by id: quarantined at every
    /// start (what makes them so is what is stored). Each is listed to the owner, counted
    /// against both shares, answers `500` to a read, and goes when the owner deletes it.
    quarantined: RwLock<std::collections::BTreeSet<String>>,
    /// Live subscriptions out of force while their removal runs ([`Notifier::remove`]): no
    /// delivery is prepared or made for one, and it is put back in force should its removal
    /// leave it stored.
    withdrawn: RwLock<std::collections::BTreeSet<String>>,
    /// The time by which subscriptions are expired, as stored.
    high_water: HighWater,
    /// Added to the clock expiry is decided by ([`Notifier::clock`]).
    #[cfg(test)]
    clock_offset: std::sync::atomic::AtomicI64,
    /// The backend's lease the state holds for as long as it runs (see
    /// `crate::store::Lifecycle`): the authority to store a first mark over a missing or
    /// unusable one ([`Store::reset_marker`]). Without it, no mark is ever lowered.
    lease: std::sync::OnceLock<crate::store::LifecycleGuard>,
}

/// Refuse a store whose marker raise is not atomic ([`Store::raises_markers`]): over one, a
/// raise that lands late could lower the stored high water ([`HighWater`]), so notifications do
/// not start there.
pub(crate) fn refuse_without_atomic_markers<S: Store>(store: &S) -> Result<(), String> {
    if store.raises_markers() {
        return Ok(());
    }
    Err(
        "store: notifications need a store that raises a stored time atomically, and this one \
         does not"
            .into(),
    )
}

/// The notification service's wall-clock high-water mark: the latest time, in seconds since the
/// epoch, known to be stored. A subscription is expired once its expiry is at or before the later
/// of the clock and this mark, and nothing outside (a read refused, a listing that leaves it out,
/// its removal, a delivery stopped) sees it expire until a time at or past its expiry is stored
/// ([`Notifier::decide`]): a start whose clock is behind still finds it expired.
///
/// The mark is one small record in the store's index alone, raised in one indivisible step at
/// the store ([`Store::raise_marker`]), so a raise that lands late never lowers it, and storing
/// it needs no blob capacity; its index entry has a slot of its own, admitted however full the
/// subscription pool is. It is written at most once for each second in which a subscription is
/// seen to expire.
///
/// When a raise the service needs fails, the service halts ([`HighWater::halted`]): no
/// delivery, no subscription request answered, no subscription removed, until a raise to the
/// latest time any decision needed ([`HighWater::pending`]) goes through. A halt shows no
/// expiry, so a subscription never shown expired may be delivered to again after a restart
/// whose clock is behind.
#[derive(Default)]
struct HighWater {
    /// The record's IRI: in the service container's pool, under a fragment no request can name.
    iri: String,
    /// The latest time known to be stored (0 while none is).
    stored: std::sync::atomic::AtomicI64,
    /// The latest time a decision needed stored: what a raise after a failure stores, never an
    /// earlier clock.
    pending: std::sync::atomic::AtomicI64,
    /// Whether a mark is stored. Until one is, no subscription is created.
    present: AtomicBool,
    /// Whether a raise the service needed failed, and none at or past `pending` has gone
    /// through since.
    halted: AtomicBool,
    /// How many raises have failed since the process started.
    failures: AtomicU64,
    /// Raises one at a time within the process (the store keeps them in order across it); a
    /// subscription create holds it, too, from its last check of the halt and its expiry until
    /// it is registered (or known not stored), so no raise fails between them.
    writing: Arc<tokio::sync::Mutex<()>>,
    /// The subscriptions expired at the start because no mark was stored (or it could not be
    /// read as one) while subscriptions with an expiry were: until each is removed, none is
    /// stored, so a start after a stop finds them expired again.
    unrecorded: Mutex<std::collections::BTreeSet<String>>,
}

/// How far ahead of the clock a stored high water may be before a start warns of it, naming the
/// recovery ([`reset_subscription_clock`]); the mark stands either way ([`Notifier::load_with`]).
const MAX_HIGH_WATER_AHEAD: i64 = 24 * 60 * 60;

/// The environment variable that starts the LWS server in the notification service's recovery
/// mode ([`super::LwsConfig::reset_subscription_clock`]): `1` or `true`.
pub const RESET_SUBSCRIPTION_CLOCK_VAR: &str = "SOLID_SERVER_LWS_RESET_SUBSCRIPTION_CLOCK";

/// Lower the notification service's stored high water to the clock, after removing every stored
/// subscription with an expiry: the operator's recovery from a mark stored while the clock had
/// jumped forward (a start warns of a mark more than a day ahead), run only while the state is
/// built, before any subscription is loaded or any request served
/// (`LwsConfig::reset_subscription_clock`). Subscriptions without an expiry are kept; the
/// subscribers of the others subscribe again.
///
/// Returns how many subscriptions were removed. Refuses (an error, the mark unchanged, whatever
/// it could remove gone; it may be run again) while any subscription with an expiry is still
/// stored: one whose removal failed, one a change cut short holds hidden (not `visible`), or one
/// a listing after the removals still shows. Stored records that are not subscriptions are left
/// as they are (a start quarantines them, and nothing is delivered for them).
pub(crate) async fn reset_subscription_clock<S: Store>(
    store: &S,
    cfg: &LwsConfig,
    visible: impl Fn(&str) -> bool,
    lease: &crate::store::LifecycleGuard,
) -> Result<usize, String> {
    refuse_without_atomic_markers(store)?;
    let container = cfg.absolute(SUBSCRIPTIONS_PATH);
    let marker = format!("{container}{HIGH_WATER_FRAGMENT}");
    let store_err = |e: crate::error::ServerError| format!("store: {e}");
    // The stored subscriptions with an expiry.
    let limited = || async {
        let mut found = Vec::new();
        if !store.exists(&container).await.map_err(store_err)? {
            return Ok::<_, String>(found);
        }
        for child in store.list_children(&container).await.map_err(store_err)? {
            let body = match store.read(child.as_str()).await {
                Err(crate::error::ServerError::NotFound) => continue,
                Err(e) => return Err(format!("store: {}: {e}", child.as_str())),
                Ok(r) => r.body,
            };
            if serde_json::from_slice::<Subscription>(&body).is_ok_and(|s| s.expires_at.is_some()) {
                found.push(child.as_str().to_string());
            }
        }
        Ok(found)
    };
    let mut removed = 0;
    for iri in limited().await?.into_iter().filter(|iri| visible(iri)) {
        if let Ok(crate::store::DeleteOutcome::Deleted) =
            super::remove_member(store, &iri, Some(&container)).await
        {
            removed += 1;
        }
    }
    let left = limited().await?.len();
    if left > 0 {
        return Err(format!(
            "{left} subscriptions with an expiry are still stored in {container}; the \
             notification high water is left as it is"
        ));
    }
    store
        .reset_marker(&marker, jose::now_secs(), lease)
        .await
        .map_err(|e| format!("store: {marker}: {e}"))?;
    Ok(removed)
}

/// The refusal of a write that may lower the mark, made without the backend's lease
/// ([`Notifier::lease`]).
fn no_lease() -> crate::error::ServerError {
    crate::error::ServerError::Storage("no lease is held".into())
}

/// The fragment the high water is stored under, in the subscriptions container ([`HighWater`]).
const HIGH_WATER_FRAGMENT: &str = crate::store::SERVICE_MARKER_FRAGMENT;

/// The answer to a notification request while the service is halted ([`HighWater::halted`]),
/// or not yet ready to take subscriptions.
fn halted() -> Response {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        Some("notifications are paused until the time can be recorded; the request may be retried"),
    )
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
        };
        self.subs.write().expect("lock").insert("test".into(), sub);
    }

    /// Ensure the service container exists and load the subscriptions it holds, all visible.
    #[cfg(test)]
    pub async fn load<S: Store>(store: &S, cfg: &LwsConfig) -> Result<Self, String> {
        let me = Self::new(cfg)?;
        let lease = store.lifecycle().unwrap_or_default();
        me.hold(lease.acquire().ok_or("the store's lease is held")?);
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
            #[cfg(feature = "crdt-sync")]
            crdt_changes: Default::default(),
            subs: RwLock::new(Subs::default()),
            streams: streams::Streams::new(),
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
            withdrawn: Default::default(),
            high_water: HighWater {
                iri: format!("{}{HIGH_WATER_FRAGMENT}", cfg.absolute(SUBSCRIPTIONS_PATH)),
                ..HighWater::default()
            },
            #[cfg(test)]
            clock_offset: Default::default(),
            lease: Default::default(),
        })
    }

    /// Hold the backend's `lease` for as long as this lives (see [`Notifier::lease`]).
    pub(crate) fn hold(&self, lease: crate::store::LifecycleGuard) {
        let _ = self.lease.set(lease);
    }

    /// The backend's lease this holds, when it holds it: what a write that may lower the mark
    /// needs.
    pub(crate) fn lease(&self) -> Option<&crate::store::LifecycleGuard> {
        self.lease.get()
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
        refuse_without_atomic_markers(store)?;
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
        // The high water first: what it says decides which subscriptions are expired. A read
        // that fails says nothing of it, and the start stops; one that is not a time is as none.
        let recorded = match store.read_marker(&self.high_water.iri).await {
            Ok(v) => v,
            Err(e) => return Err(format!("store: {}: {e}", self.high_water.iri)),
        };
        let now = self.clock();
        let stored = recorded.as_deref().and_then(|v| v.parse::<i64>().ok());
        // A mark further ahead of the clock than a clock is ever off was most likely stored
        // while the clock had jumped forward. It still stands (whatever expired by it stays
        // expired); only the operator may lower it, by removing every subscription with an
        // expiry first ([`reset_subscription_clock`]).
        if let Some(ahead) = stored.filter(|t| *t > now + MAX_HIGH_WATER_AHEAD) {
            eprintln!(
                "lws: WARNING: the notification high water at {} is {} seconds ahead of the \
                 clock; subscriptions expiring before it are expired, and a new subscription \
                 must expire after it. If the clock had jumped forward, correct it and start \
                 once with {RESET_SUBSCRIPTION_CLOCK_VAR}=1, which removes every subscription \
                 with an expiry and then stores the clock",
                self.high_water.iri,
                ahead - now
            );
        }
        let mut loaded = Vec::new();
        let mut held_back = 0;
        let mut quarantined = std::collections::BTreeSet::new();
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
            // Only the new stream shape may omit inbox. Keep the old webhook loader's
            // required-field check even though the shared record now has a default for it.
            let missing_webhook_inbox = sub.kind == SubscriptionKind::Webhook
                && serde_json::from_slice::<Value>(&read.body)
                    .map_or(true, |raw| raw.get("inbox").is_none());
            if missing_webhook_inbox
                || (sub.kind != SubscriptionKind::Webhook
                    && (sub.capability_hash.is_none()
                        || sub.expires_at.is_none()
                        || !sub.inbox.is_empty()))
            {
                quarantined.insert(id);
                continue;
            }
            sub.id = id;
            loaded.push((child, sub));
        }
        // No high water stored (or none that reads as a time) while subscriptions with an expiry
        // are: when they would expire cannot be told, so every one of them is expired (its
        // subscriber may subscribe again), and none is stored until they are all removed.
        let mut unrecorded = std::collections::BTreeSet::new();
        if stored.is_none() {
            for (_, sub) in loaded.iter().filter(|(_, s)| s.expires_at.is_some()) {
                sub.lapsed.0.store(true, Ordering::Release);
                unrecorded.insert(sub.id.clone());
            }
            if !unrecorded.is_empty() {
                eprintln!(
                    "lws: no notification high water could be read at {}; {} subscriptions with \
                     an expiry are expired",
                    self.high_water.iri,
                    unrecorded.len()
                );
            }
        }
        // A subscription past its expiry by the clock has the clock stored before it is purged.
        // A raise that fails halts the service, which then removes nothing.
        let mut high_water = stored.unwrap_or(0);
        let mut pending = high_water;
        let mut halted = false;
        let iri = &self.high_water.iri;
        if stored.is_some() && loaded.iter().any(|(_, s)| s.due(high_water, now)) {
            pending = now;
            match store.raise_marker(iri, now).await {
                Ok(raised) => high_water = raised,
                Err(_) => halted = true,
            }
        }
        // An expired subscription is purged; one that cannot be removed stays loaded and counted
        // (no delivery goes to it: every attempt checks expiry) until it can be.
        let mut subs = Subs::default();
        for (child, sub) in loaded {
            if !halted
                && sub.expired(high_water)
                && matches!(
                    super::remove_member(store, child.as_str(), Some(&container)).await,
                    Ok(crate::store::DeleteOutcome::Deleted
                        | crate::store::DeleteOutcome::NotFound)
                )
            {
                unrecorded.remove(&sub.id);
                continue;
            }
            if sub.kind != SubscriptionKind::Webhook {
                self.streams.register(&sub.id);
            }
            subs.insert(sub.id.clone(), sub);
        }
        // A fresh storage (or one whose mark could not be read) has one stored now, over
        // whatever was there, once nothing expired for want of it is left; until then no
        // subscription is created.
        let mut present = stored.is_some();
        if !present && unrecorded.is_empty() {
            pending = now;
            let reset = match self.lease() {
                Some(lease) => store.reset_marker(iri, now, lease).await,
                None => Err(no_lease()),
            };
            match reset {
                Ok(()) => (high_water, present) = (now, true),
                Err(_) => halted = true,
            }
        }
        if halted {
            self.high_water.failures.fetch_add(1, Ordering::AcqRel);
            eprintln!(
                "lws: the notification high water could not be stored at {iri}; notifications \
                 are paused until it is"
            );
        }
        let hw = &self.high_water;
        hw.stored.store(high_water, Ordering::Release);
        hw.pending.store(pending.max(high_water), Ordering::Release);
        hw.present.store(present, Ordering::Release);
        hw.halted.store(halted, Ordering::Release);
        *hw.unrecorded.lock().expect("lock") = unrecorded;
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
        let high_water = self.high_water();
        let subs = self.subs.read().expect("lock");
        let unloaded =
            self.held_back.load(Ordering::SeqCst) + self.quarantined.read().expect("lock").len();
        let is_owner = |s: &&Subscription| owner.is_some() && s.subscriber.as_deref() == owner;
        let share = |owners: bool| {
            let mine = || subs.values().filter(move |s| is_owner(s) == owners);
            let held = mine().count() + unloaded;
            let authors = mine()
                .filter(|s| !s.expired(high_water))
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

    /// The latest time known to be stored as the high water ([`HighWater`]).
    fn high_water(&self) -> i64 {
        self.high_water.stored.load(Ordering::Acquire)
    }

    /// Whether the service is halted ([`HighWater::halted`]).
    fn halted(&self) -> bool {
        self.high_water.halted.load(Ordering::Acquire)
    }

    /// The wall clock expiry is decided by, in seconds since the epoch (moved by a test to stand
    /// for a clock stepped back or on).
    fn clock(&self) -> i64 {
        #[cfg(test)]
        return jose::now_secs() + self.clock_offset.load(Ordering::SeqCst);
        #[cfg(not(test))]
        jose::now_secs()
    }

    /// The one decision of expiry: every subscription shown expired, left out, removed or not
    /// delivered to because it expired is decided here. The clock is read once; when one of the
    /// subscriptions `weighed` names has expired by it and not yet by the stored high water, the
    /// high water is raised to it first ([`Notifier::raise`]). The time returned then decides
    /// each of them durably: [`Subscription::lapsing`] by it and the stored high water says
    /// whether it is expired. `None` while the service is halted, or when the raise fails
    /// (which halts it): nothing may then be decided.
    async fn decide<S: Store>(
        &self,
        store: &S,
        weighed: impl FnOnce(&Subs) -> Vec<String>,
    ) -> Option<i64> {
        if self.halted() {
            return None;
        }
        let now = self.clock();
        let due = {
            let high_water = self.high_water();
            let held = self.subs.read().expect("lock");
            weighed(&held)
                .iter()
                .any(|id| held.get(id).is_some_and(|s| s.due(high_water, now)))
        };
        (!due || self.raise(store, now).await).then_some(now)
    }

    /// Raise the stored high water to `to`, unless it is there already; `false` when that could
    /// not be done. A raise that fails halts the service, and `to` is kept as owed
    /// ([`HighWater::pending`]): only a raise at or past it ends the halt
    /// ([`Notifier::resume`]). At most one write goes out for each second in which a
    /// subscription is seen to expire.
    async fn raise<S: Store>(&self, store: &S, to: i64) -> bool {
        let hw = &self.high_water;
        hw.pending.fetch_max(to, Ordering::AcqRel);
        if self.halted() {
            return false;
        }
        if self.high_water() >= to {
            return true;
        }
        let _writing = hw.writing.lock().await;
        if self.halted() {
            return false;
        }
        self.high_water() >= to || self.raise_pending(store).await
    }

    /// While halted, or while no high water is stored and nothing expired for want of one is
    /// left, raise it to what is owed ([`HighWater::pending`]), never to the clock: what the
    /// expiry sweeper tries, unless a raise is under way already.
    async fn resume<S: Store>(&self, store: &S) {
        let hw = &self.high_water;
        let unready = !hw.present.load(Ordering::Acquire);
        if !self.halted() && !unready {
            return;
        }
        let Ok(_writing) = hw.writing.try_lock() else {
            return;
        };
        if unready {
            // A first mark: what nothing has yet been decided by may start at the clock.
            hw.pending.fetch_max(self.clock(), Ordering::AcqRel);
        }
        self.raise_pending(store).await;
    }

    /// Raise the stored high water to what is owed, its raises held one at a time; never while
    /// subscriptions expired for want of one remain ([`HighWater::unrecorded`]). `true` once
    /// what is owed is stored, which ends a halt.
    async fn raise_pending<S: Store>(&self, store: &S) -> bool {
        let hw = &self.high_water;
        if !hw.unrecorded.lock().expect("lock").is_empty() {
            return false;
        }
        let owed = hw.pending.load(Ordering::Acquire);
        // With none stored (or none usable), the first is written over whatever is there.
        let raised = match hw.present.load(Ordering::Acquire) {
            true => store.raise_marker(&hw.iri, owed).await,
            false => match self.lease() {
                Some(lease) => store
                    .reset_marker(&hw.iri, owed, lease)
                    .await
                    .map(|()| owed),
                None => Err(no_lease()),
            },
        };
        match raised {
            Ok(raised) if raised >= owed => {
                hw.stored.fetch_max(raised, Ordering::AcqRel);
                hw.present.store(true, Ordering::Release);
                if hw.halted.swap(false, Ordering::AcqRel) {
                    eprintln!("lws: the notification high water is stored; notifications resume");
                }
                true
            }
            outcome => {
                hw.halted.store(true, Ordering::Release);
                // Said at the first failure and at each doubling of their count after it.
                let failures = hw.failures.fetch_add(1, Ordering::AcqRel) + 1;
                if failures.is_power_of_two() {
                    let why = outcome
                        .err()
                        .map_or("it was lowered".into(), |e| e.to_string());
                    eprintln!(
                        "lws: the notification high water could not be stored ({why}; {failures} \
                         failed attempts); notifications are paused until it is"
                    );
                }
                false
            }
        }
    }

    /// Subscription `id`, unless it is expired by `now` ([`Notifier::decide`]).
    fn get(&self, id: &str, now: i64) -> Option<Subscription> {
        let high_water = self.high_water();
        self.subs
            .read()
            .expect("lock")
            .get(id)
            .filter(|s| !s.lapsing(high_water, now))
            .cloned()
    }

    /// What a listing by `subscriber` (every one, for the storage `owner`) weighs, unexpired by
    /// the stored high water: `subscriber`'s own, read through the subscriber index, so the
    /// listing reads its records and no one else's. An unauthenticated subscriber holds nothing
    /// of its own to show: subscriptions made without a subject belong to no one who could come
    /// back for them.
    fn listed(&self, subscriber: Option<&str>, owner: bool) -> Vec<Subscription> {
        let high_water = self.high_water();
        let subs = self.subs.read().expect("lock");
        let listed: Box<dyn Iterator<Item = &Subscription>> = match (owner, subscriber) {
            (true, _) => Box::new(subs.values()),
            (false, Some(_)) => Box::new(subs.mine(subscriber)),
            (false, None) => Box::new(std::iter::empty()),
        };
        listed.filter(|s| !s.expired(high_water)).cloned().collect()
    }

    /// The subscriptions a listing by `subscriber` (every one, for the storage `owner`) shows,
    /// decided durably from one snapshot: they are read, then the clock is read once, and when
    /// one of them has expired by it and not yet by the stored high water, the high water is
    /// raised to it ([`Notifier::raise`]) before any is left out. One registered after the
    /// snapshot is not in the listing at all, rather than left out as expired by a time not
    /// stored. `None` while the service is halted, or when the raise fails.
    async fn shown<S: Store>(
        &self,
        store: &S,
        subscriber: Option<&str>,
        owner: bool,
    ) -> Option<Vec<Subscription>> {
        if self.halted() {
            return None;
        }
        let listed = self.listed(subscriber, owner);
        let now = self.clock();
        let high_water = self.high_water();
        if listed.iter().any(|s| s.due(high_water, now)) && !self.raise(store, now).await {
            return None;
        }
        let high_water = self.high_water();
        Some(
            listed
                .into_iter()
                .filter(|s| !s.lapsing(high_water, now))
                .collect(),
        )
    }

    /// The subscriptions a listing by `subscriber` (every one, for the `owner`) would show by
    /// `now`, deciding nothing.
    #[cfg(test)]
    fn live_for(&self, subscriber: Option<&str>, owner: bool, now: i64) -> Vec<Subscription> {
        let high_water = self.high_water();
        self.listed(subscriber, owner)
            .into_iter()
            .filter(|s| !s.lapsing(high_water, now))
            .collect()
    }

    /// Whether the subscription `id` still stands by the clock, deciding nothing: not halted,
    /// not cancelled, not expired by the clock or by the stored high water.
    #[cfg(test)]
    fn is_live(&self, id: &str) -> bool {
        let (high_water, now) = (self.high_water(), self.clock());
        !self.halted()
            && self
                .subs
                .read()
                .expect("lock")
                .get(id)
                .is_some_and(|s| !s.lapsing(high_water, now))
    }

    /// Whether a delivery to the subscription `id` may be made now: the service is not halted,
    /// and the subscription is neither cancelled (deleted, or deactivated after failed
    /// deliveries) nor expired, decided durably ([`Notifier::decide`]). A delivery that goes
    /// ahead waits on nothing here.
    async fn stands_now<S: Store>(&self, store: &S, id: &str) -> bool {
        let Some(now) = self.decide(store, |_| vec![id.to_string()]).await else {
            return false;
        };
        let high_water = self.high_water();
        let subs = self.subs.read().expect("lock");
        subs.get(id).is_some_and(|s| !s.lapsing(high_water, now)) && !self.is_withdrawn(id)
    }

    /// Whether the subscription `id` is out of force while its removal runs.
    fn is_withdrawn(&self, id: &str) -> bool {
        self.withdrawn.read().expect("lock").contains(id)
    }

    /// Remove a subscription as every service record is removed
    /// ([`super::records::remove_record`]): memory follows the store, so it leaves only once its
    /// removal is confirmed, and is announced then. One no longer in force is already gone
    /// ([`super::records::Removed::Absent`]), as is one that expired while its removal by its
    /// subscriber waited: an expired subscription is gone to its subscriber, and is left for the
    /// expiry sweeper.
    ///
    /// Expiry is decided durably ([`Notifier::decide`]), before the locks and again under
    /// them; while the service is halted nothing is removed (`503`). A `reclaim` (the expiry
    /// sweeper's, or a create's purge of its subscriber's own) removes only a subscription
    /// expired by the stored high water once that is decided, never one that stands, whatever
    /// the clock did since it was picked.
    async fn remove<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        id: &str,
        admission: Option<crate::overload::AdmissionSlot>,
        by: super::records::Remover,
        reclaim: bool,
    ) -> Result<super::records::Removed, Response> {
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}{id}");
        let Some(now) = self.decide(&state.store, |_| vec![id.to_string()]).await else {
            return Err(halted());
        };
        if reclaim {
            let high_water = self.high_water();
            let subs = self.subs.read().expect("lock");
            if !subs.get(id).is_some_and(|s| s.lapsing(high_water, now)) {
                return Ok(super::records::Removed::Absent);
            }
        }
        // An expired subscription is listed nowhere, so its removal holds no listing's lock and
        // keeps no conditional create waiting; a live one is in its subscriber's listing too.
        // Expiry is decided again under the record's lock, before anything is removed
        // (`set_aside`, below): a removal taken as of a live subscription (holding its
        // listing's lock) goes on only while it is still live, and one taken as of an expired
        // subscription (holding none) only while it is still expired. Otherwise nothing is
        // removed and the removal finds it `Absent`: a subscriber's DELETE of one that expired
        // meanwhile is answered `404`, and the sweeper removes it later as an expired one.
        let own_listing = {
            let high_water = self.high_water();
            let subs = self.subs.read().expect("lock");
            match subs.get(id) {
                Some(s) if !s.lapsing(high_water, now) => {
                    Some(listing_lock(&container, s.subscriber.as_deref()))
                }
                _ => None,
            }
        };
        let listed = match &own_listing {
            Some(name) => super::records::Listed::AlsoIn(name),
            None => super::records::Listed::Nowhere,
        };
        let undecided = Arc::new(AtomicBool::new(false));
        let set_aside = {
            let (state, id, undecided) = (state.clone(), id.to_string(), undecided.clone());
            let unlisted = own_listing.is_none();
            move || async move {
                let me = &state.notify;
                let weighed = |_: &Subs| vec![id.clone()];
                let Some(now) = me.decide(&state.store, weighed).await else {
                    undecided.store(true, Ordering::Release);
                    return None;
                };
                let high_water = me.high_water();
                let subs = me.subs.read().expect("lock");
                subs.get(&id)
                    .filter(|s| s.lapsing(high_water, now) == unlisted)?;
                // A live one is taken out of force while its removal runs: nothing is delivered
                // to it from here on, and it is put back should its removal leave it stored.
                // (An expired one is out of force already.)
                if !unlisted {
                    me.withdrawn.write().expect("lock").insert(id.clone());
                }
                Some(!unlisted)
            }
        };
        let settle = {
            let id = id.to_string();
            move |state: &LwsState<S>, gone: bool, withdrawn: bool| {
                let me = &state.notify;
                // Back in force once its removal left it stored; gone from memory first
                // otherwise, so no preparation sees it in force between the two.
                let author = gone.then(|| {
                    // An expired subscription is listed nowhere, so its removal changes no
                    // listing.
                    let removed = me.subs.write().expect("lock").remove(&id);
                    me.streams.remove(&id);
                    me.high_water.unrecorded.lock().expect("lock").remove(&id);
                    let author = removed.as_ref().and_then(|s| s.subscriber.clone());
                    if removed.is_some_and(|s| !s.expired(me.high_water())) {
                        *me.etag.write().expect("lock") = new_etag();
                    }
                    author
                });
                if withdrawn {
                    me.withdrawn.write().expect("lock").remove(&id);
                }
                author.flatten()
            }
        };
        let removed = super::records::remove_record(
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
        .await;
        if undecided.load(Ordering::Acquire) {
            return Err(halted());
        }
        removed
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
        #[allow(unused_mut)]
        let mut prepared = self.prepare_for(state, event, limit, None).await;
        #[cfg(feature = "crdt-sync")]
        if event.kind == "Create" {
            if let Some(("target", parent)) = &event.relation {
                prepared.crdt_create = Some((parent.clone(), event.uri.clone()));
            }
        }
        prepared
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
        // Preparing runs under the change's locks, so it writes nothing and reads no clock: only
        // what the stored high water has expired is left out. One expired by the clock alone is
        // passed on, and its delivery decides it ([`Watch::stands`], which stores the time
        // before it stops one). While the service is halted nothing is prepared.
        let candidates: Vec<Subscription> = if self.halted() {
            Vec::new()
        } else {
            let high_water = self.high_water();
            let subs = self.subs.read().expect("lock");
            let withdrawn = self.withdrawn.read().expect("lock");
            subs.values()
                .filter(|s| hears(s) && !s.expired(high_water) && !withdrawn.contains(&s.id))
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
                    kind: sub.kind,
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
        #[cfg(feature = "crdt-sync")]
        if let Some((room, file)) = &prepared.crdt_create {
            self.crdt_changes.created(room, file);
        }
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
        // request that made it. One task admits webhooks first, then publishes stream pendings
        // in preparation order. Each subscription uses one transport, so its pending order is
        // preserved. When the
        // runtime runs that task is the runtime's business: this says the request does not do
        // the work, not that the work happens after the response reaches the client.
        let state = state.detached();
        tokio::spawn(async move {
            let mut streams = Vec::new();
            for p in prepared.pending {
                if p.kind == SubscriptionKind::Webhook {
                    state
                        .notify
                        .deliver(&state, &p.inbox, p.activity, Some(p.watch));
                } else {
                    streams.push(p);
                }
            }
            // Admit every webhook before any stream's asynchronous access recheck or gate.
            // The stream pendings retain their preparation order.
            for p in streams {
                streams::publish(&state, p).await;
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
        let state = state.detached();
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
        let state = state.detached();
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
                let _ = notifier.remove(&state, &id, None, by, false).await;
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

    /// Raises of the stored high water that failed since this notifier was made: while the last
    /// one failed, notifications are paused (see the module docs).
    pub fn high_water_failures(&self) -> u64 {
        self.high_water.failures.load(Ordering::SeqCst)
    }

    /// Whether notifications are paused for want of a stored high water (see the module docs).
    pub fn paused(&self) -> bool {
        self.halted()
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
    // While the service is halted, every request to it is refused alike, whatever it names.
    if state.notify.halted() {
        return halted();
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
            Method::GET | Method::HEAD => {
                // What is left out as expired is decided durably.
                let subscriber = agent.subject.as_deref();
                let Some(shown) = state.notify.shown(&state.store, subscriber, owner).await else {
                    return halted();
                };
                listing(state, req, owner, shown)
            }
            // Expired subscriptions still hold their place in the store until they are removed:
            // a few go once the request is authorized (see [`subscribe`]).
            Method::POST => subscribe(state, req, agent, owner).await,
            _ => method_not_allowed("GET, HEAD, POST"),
        };
    }
    if owner && state.notify.quarantined.read().expect("lock").contains(id) {
        return quarantined(state, req, id).await;
    }
    // Who may see it is asked of what is stored; whether it expired is decided below.
    let sub = state.notify.get(id, state.notify.high_water());
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
    // Gone to its subscriber once expired, decided durably.
    let Some(now) = state
        .notify
        .decide(&state.store, |_| vec![id.to_string()])
        .await
    else {
        return halted();
    };
    if state.notify.get(id, now).is_none() {
        return problem(StatusCode::NOT_FOUND, None);
    }
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
                .remove(state, id, req.admission.clone(), by, false)
                .await
            {
                Ok(super::records::Removed::Gone) => problem(StatusCode::NO_CONTENT, None),
                // Removed by another request while this one waited, or expired meanwhile: gone
                // to its subscriber either way, as an expired one is to every request.
                Ok(super::records::Removed::Absent) => problem(StatusCode::NOT_FOUND, None),
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
                super::records::at_once(set_aside),
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

/// The inspection document, without capability URLs; webhooks also echo their inbox.
fn document<S: Store>(state: &LwsState<S>, sub: &Subscription) -> Value {
    let url = state
        .cfg
        .absolute(&format!("{SUBSCRIPTIONS_PATH}{}", sub.id));
    let mut doc = json!({
        "@context": [LWS_CONTEXT],
        "id": url,
        "type": sub.kind.name(),
        "subscription": url,
        "topic": sub.topics,
    });
    if sub.kind == SubscriptionKind::Webhook {
        doc["inbox"] = json!(sub.inbox);
    } else {
        doc["@context"] = json!([LWS_CONTEXT, {
            "@vocab": "https://w3id.org/jeswr/lws#",
            "id": "@id", "type": "@type",
            "topic": {"@id": "https://w3id.org/jeswr/lws#topic", "@type": "@id"},
            "source": {"@id": "https://w3id.org/jeswr/lws#source", "@type": "@id"},
            "receiveFrom": {"@id": "https://w3id.org/jeswr/lws#receiveFrom", "@type": "@id"},
            "expires": {"@id": "https://w3id.org/jeswr/lws#expires", "@type": "http://www.w3.org/2001/XMLSchema#dateTime"}
        }]);
        doc["authMode"] = json!("capability");
        if sub.kind == SubscriptionKind::Sse {
            doc["retention"] = json!("PT10M");
        }
    }
    if let Some(e) = &sub.expires {
        doc["expires"] = Value::String(e.clone());
    }
    doc
}

/// The subscriber's live subscriptions (every one for the owner), `shown` as
/// [`Notifier::shown`] decided them, as an LWS container: negotiated, paged and tagged like any
/// other container (webhook "Subscription Management").
fn listing<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    owner: bool,
    shown: Vec<Subscription>,
) -> Response {
    let base = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let items: Vec<Value> = shown
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
/// (see `records::Listed`): outside the storage, so it is never a resource's lock (a topic is
/// checked against the storage before any lock is taken), and one per subscriber (anonymous
/// subscriptions share theirs, as they share their listing).
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
/// subscriber index, once every one of them past its expiry by the clock is decided expired
/// ([`Notifier::decide`]), so none of those counts against its capacity, whichever are removed. This is the only purging the request waits for, so the work it does here
/// is bounded by its own records; everyone else's are swept by the expiry sweeper
/// ([`spawn_expiry_sweeper`]). Never while the request holds the listing alone.
///
/// `false` when the service is halted once it is done (a removal's decision may have halted it,
/// [`Notifier::decide`]): the request then answers `503` and creates nothing.
async fn purge_own_expired<S: Store + 'static>(
    state: &LwsState<S>,
    subscriber: Option<&str>,
    admission: Option<crate::overload::AdmissionSlot>,
) -> bool {
    // Every one of the subscriber's own subscriptions past its expiry by the clock is decided
    // first, whatever the removals below reach: decided, none is counted against the
    // subscriber's capacity, even when the removals are spent on expired ones that cannot be
    // removed (whose ids may come first).
    let subscriber_owned = subscriber.map(str::to_string);
    let decided = state
        .notify
        .decide(&state.store, |subs| {
            subs.mine(subscriber_owned.as_deref())
                .map(|s| s.id.clone())
                .collect()
        })
        .await;
    let Some(now) = decided else {
        return false;
    };
    let high_water = state.notify.high_water();
    let expired = {
        let subs = state.notify.subs.read().expect("lock");
        subs.expired_of(subscriber, high_water, now, EXPIRED_REMOVED_PER_SUBSCRIBE)
    };
    for id in expired {
        let _ = state
            .notify
            .remove(
                state,
                &id,
                admission.clone(),
                super::records::Remover::Request,
                true,
            )
            .await;
    }
    !state.notify.halted()
}

/// The gate a subscription create passes once its listing locks are held, before anything is
/// stored ([`super::records::create_record`]): the high water's raise lock
/// ([`HighWater::writing`], so the lock order is listing, then raise), held until the create is
/// settled. Under it the create is refused, `503`, while the service is halted, and `400` when
/// its expiry (`expires_at`) is no longer after the clock and the stored high water: no raise
/// can fail, nor the high water pass the expiry, between that check and the registration.
async fn commit_gate<S: Store + 'static>(
    state: &LwsState<S>,
    expires_at: Option<i64>,
) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, Response> {
    let writing = state.notify.high_water.writing.clone();
    let guard = super::records::within_wait(async { Some(writing.lock_owned().await) }).await?;
    if state.notify.halted() {
        return Err(halted());
    }
    let after = state.notify.clock().max(state.notify.high_water());
    if expires_at.is_some_and(|t| t <= after) {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            Some("expires is in the past"),
        ));
    }
    Ok(Some(guard))
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

/// How long one of the sweeper's removals may take before it is given up, to be tried again:
/// longer than the store calls a removal makes take while the store is healthy (an HTTP index
/// allows each call 30 seconds). A degraded store may still cut one short, above all one whose
/// delete's outcome is unknown and must be settled; it is tried again after its wait.
const SWEEP_REMOVAL_DEADLINE: Duration = Duration::from_secs(120);

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
/// The sweeper is tied to the state's owners, not to whatever holds the state: it ends as soon
/// as the last handle from [`LwsState::new`] (and its clones) is dropped (`closed`), and with
/// it every removal it owns ([`Sweeper`]), aborted. Between sweeps it holds the state weakly;
/// its removals hold a detached handle ([`LwsState::detached`]), which keeps the state alive
/// until they are aborted but never keeps the sweeper going.
pub(crate) fn spawn_expiry_sweeper<S: Store + 'static>(
    state: &LwsState<S>,
    mut closed: tokio::sync::watch::Receiver<()>,
) {
    let (weak, wake) = (state.downgrade(), state.notify.sweep_now.clone());
    tokio::spawn(async move {
        let mut sweeper = Sweeper::default();
        let start = tokio::time::Instant::now() + EXPIRY_SWEEP_EVERY;
        let mut ticks = tokio::time::interval_at(start, EXPIRY_SWEEP_EVERY);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                // Every owner's handle is gone (see `LwsState`): the sweeper goes, and its
                // removals under way with it (aborted, see `Sweeper::removals`).
                _ = closed.changed() => return,
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
    /// A raise of the high water under way, while the service is halted ([`HighWater`]):
    /// `true` if it ended with the high water stored.
    records: tokio::task::JoinSet<bool>,
    /// After a raise that did not store the high water: when the next may start, and how many
    /// in a row have not.
    record_wait: Option<(tokio::time::Instant, u32)>,
    /// Holds whose release failed, being released again (one task at a time; see
    /// [`LwsState::release_unreleased`]), owned here so it goes with the sweeper.
    releases: tokio::task::JoinSet<()>,
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
            self.ended(ended, clock);
        }
        while self.releases.try_join_next().is_some() {}
        if self.releases.is_empty() && state.has_unreleased() {
            let state = state.clone();
            self.releases
                .spawn(async move { state.release_unreleased().await });
        }
        // Counted from the removals it owns, not from the places free now: a removal spawned
        // but not yet run has not taken its place, and one more started for it would find
        // none, and be backed off without ever being tried.
        let room = super::records::MAINTENANCE_PLACES.saturating_sub(self.removals.len());
        // While the service is halted, or has no high water stored yet, the high water is raised
        // again, one raise at a time, to what is owed ([`Notifier::resume`]), waiting longer
        // after each that fails (as a removal does); nothing is removed while it is halted. A
        // first mark waits for the removal of what expired for want of one, and that wait is
        // no failure.
        while let Some(ended) = self.records.try_join_next() {
            self.raise_ended(ended, clock);
        }
        let hw = &state.notify.high_water;
        let unready =
            !hw.present.load(Ordering::Acquire) && hw.unrecorded.lock().expect("lock").is_empty();
        let due = self.record_wait.is_none_or(|(at, _)| at <= clock);
        if self.records.is_empty() && due && (state.notify.halted() || unready) {
            let state = state.detached();
            self.records.spawn(async move {
                let again = state.notify.resume(&state.store);
                let _ = tokio::time::timeout(SWEEP_REMOVAL_DEADLINE, again).await;
                let hw = &state.notify.high_water;
                !state.notify.halted() && hw.present.load(Ordering::Acquire)
            });
        }
        if state.notify.halted() {
            return;
        }
        let (high_water, now) = (state.notify.high_water(), state.notify.clock());
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
                if sub.lapsing(high_water, now) && !busy.contains(id) && !waiting {
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
            let (state, sub) = (state.detached(), id.clone());
            let removal = self.removals.spawn(async move {
                let removal =
                    state
                        .notify
                        .remove(&state, &sub, None, super::records::Remover::Sweeper, true);
                matches!(
                    tokio::time::timeout(SWEEP_REMOVAL_DEADLINE, removal).await,
                    Ok(Ok(_))
                )
            });
            self.by_task.insert(removal.id(), id);
        }
    }

    /// Take in one raise of the high water that ended: one that did not store it makes the next
    /// wait, longer each time.
    fn raise_ended(
        &mut self,
        ended: Result<bool, tokio::task::JoinError>,
        clock: tokio::time::Instant,
    ) {
        if matches!(ended, Ok(true)) {
            self.record_wait = None;
            return;
        }
        let failed = self.record_wait.map_or(0, |(_, n)| n).saturating_add(1);
        let wait = SWEEP_BACKOFF_FIRST
            .saturating_mul(1 << (failed - 1).min(16))
            .min(SWEEP_BACKOFF_MOST);
        self.record_wait = Some((clock + wait, failed));
    }

    /// Wait for the raise of the high water under way, if any, and take it in.
    #[cfg(test)]
    async fn raised(&mut self) {
        while let Some(ended) = self.records.join_next().await {
            self.raise_ended(ended, tokio::time::Instant::now());
        }
    }

    /// Take in one removal that ended: one that did not remove its subscription waits before
    /// it is tried again, longer each time.
    fn ended(
        &mut self,
        ended: Result<(tokio::task::Id, bool), tokio::task::JoinError>,
        clock: tokio::time::Instant,
    ) {
        let (task, removed) = match ended {
            Ok((task, removed)) => (task, removed),
            Err(e) => (e.id(), false),
        };
        let Some(id) = self.by_task.remove(&task) else {
            return;
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

    /// Wait for every removal under way to end, and take each in (tests).
    #[cfg(test)]
    async fn settled(&mut self) {
        while let Some(ended) = self.removals.join_next_with_id().await {
            self.ended(ended, tokio::time::Instant::now());
        }
    }
}

async fn subscribe<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    lister: bool,
) -> Response {
    // No subscription is created until a high water is stored: without one, none could be
    // removed once expired ([`HighWater`]).
    if !state.notify.high_water.present.load(Ordering::Acquire) {
        return halted();
    }
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
    let ty = body.get("type").and_then(Value::as_str).unwrap_or_default();
    let ty = ty
        .strip_prefix(LWS_NS)
        .or_else(|| ty.strip_prefix("https://jeswr.org/ns/solid-sd#"))
        .or_else(|| ty.strip_prefix("https://w3id.org/jeswr/lws#"))
        .unwrap_or(ty);
    let kind = match ty {
        WEBHOOK => SubscriptionKind::Webhook,
        streams::SSE => SubscriptionKind::Sse,
        streams::WEBSOCKET => SubscriptionKind::WebSocket,
        _ => {
            return problem(
                StatusCode::BAD_REQUEST,
                Some("type is not an advertised subscription type"),
            )
        }
    };
    if kind != SubscriptionKind::Webhook
        && !url::Url::parse(&state.cfg.base_url)
            .is_ok_and(|u| u.scheme() == "https" && u.has_host())
        && !state.cfg.allow_insecure_fetch
    {
        return problem(
            StatusCode::BAD_REQUEST,
            Some("capability URLs require an https base"),
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
    let inbox = if kind == SubscriptionKind::Webhook {
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
        inbox
    } else {
        if body.get("inbox").is_some() {
            return problem(
                StatusCode::BAD_REQUEST,
                Some("stream subscriptions have no inbox"),
            );
        }
        ""
    };
    let (mut expires, mut expires_at) = match body.get("expires") {
        None | Some(Value::Null) => (None, None),
        Some(Value::String(e)) => match parse_rfc3339(e) {
            // Past by the clock or by the stored high water, whichever is later.
            Some(t) if t > state.notify.clock().max(state.notify.high_water()) => {
                (Some(e.clone()), Some(t))
            }
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
    if kind != SubscriptionKind::Webhook && expires_at.is_none() {
        let until = state.notify.clock().max(state.notify.high_water()) + 24 * 60 * 60;
        expires = Some(format_rfc3339(until));
        expires_at = Some(until);
    }
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
            // Read is decided before anything is asked of whether the topic is there, so a
            // subscriber that may not read it causes the same store work, and gets the same
            // answer, whether it exists or not.
            if aside {
                return matches!(
                    access::aside_read_first(state, Action::Read, &resource, agent).await,
                    Ok(access::Aside::Allowed)
                );
            }
            state.allowed(Action::Read, &resource, agent).await
                && state.store.exists(&resource).await.unwrap_or(false)
        }
    };
    for topic in &topics {
        let resource = topic.strip_suffix(META_SUFFIX).unwrap_or(topic);
        // A linkset's record is no resource, and has no linkset: whatever is stored, it is not
        // there to read.
        if super::names_record(resource) || !resource.starts_with(storage) {
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
    if !purge_own_expired(state, agent.subject.as_deref(), req.admission.clone()).await {
        return halted();
    }
    // A conditional create is evaluated against the listing it compares, held alone from here
    // until the subscription is registered (or, its outcome unknown, shared until it is
    // settled). Taken only once every topic has been checked (and its
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
        // What it compares leaves out as expired is decided durably, once the listing is held
        // (after every change to it that was under way).
        |get| async move {
            match state
                .notify
                .shown(&state.store, agent.subject.as_deref(), lister)
                .await
            {
                Some(shown) => listing(state, &get, lister, shown),
                None => halted(),
            }
        },
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
    let high_water = state.notify.high_water();
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
        let mine = subs.live_count_of(agent.subject.as_deref(), high_water);
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
    let capability = (kind != SubscriptionKind::Webhook).then(streams::capability);
    let sub = Subscription {
        id: id.clone(),
        subscriber: agent.subject.clone(),
        client: agent.client.clone(),
        topics,
        inbox: inbox.to_string(),
        expires,
        expires_at,
        failures: 0,
        kind,
        capability_hash: capability.as_ref().map(|(_, hash)| *hash),
        lapsed: Lapsed::default(),
    };
    let stored = serde_json::to_vec(&sub).unwrap_or_default();
    let mut doc = document(state, &sub);
    if let Some((token, _)) = &capability {
        streams::connection_details(state, kind, token, &mut doc);
    }
    let author = sub.subscriber.clone();
    let register = {
        move |state: &LwsState<S>| {
            if sub.kind != SubscriptionKind::Webhook {
                state.notify.streams.register(&id);
            }
            state.notify.subs.write().expect("lock").insert(id, sub);
            *state.notify.etag.write().expect("lock") = new_etag();
            // Counted as registered from here on.
            drop(slot);
        }
    };
    // Nothing is created while the service is halted, whatever halted it since the request
    // began (its own purge, or anything else).
    if state.notify.halted() {
        return halted();
    }
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
        commit_gate(state, expires_at),
    );
    if let Err(refused) = created.await {
        return refused;
    }
    // The listing and the new record's locks are gone with the create, so the sweep of
    // everyone's expired subscriptions can run now without waiting on this request, and this
    // request answers without waiting on it.
    wake_expiry_sweeper(state);
    let mut resp = super::json_response(StatusCode::CREATED, LWS_JSON, &doc);
    if kind != SubscriptionKind::Webhook {
        set(resp.headers_mut(), header::CACHE_CONTROL, "no-store");
        set(resp.headers_mut(), header::REFERRER_POLICY, "no-referrer");
    }
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

    fn sub_until(expires_at: i64) -> Subscription {
        Subscription {
            id: String::new(),
            subscriber: None,
            client: None,
            topics: vec!["http://localhost:3000/".into()],
            inbox: "https://a/inbox".into(),
            expires: None,
            expires_at: Some(expires_at),
            failures: 0,
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
        }
    }

    /// Store `at` as the high water, whatever is stored now (lower, too): as an earlier time
    /// stored would stand.
    async fn set_high_water(state: &LwsState<test_store::FlakyStore>, at: i64) {
        let iri = &state.notify.high_water.iri;
        state.store.plant_marker(iri, &at.to_string()).await;
        state.notify.high_water.stored.store(at, Ordering::SeqCst);
    }

    /// The high water as stored.
    async fn stored_high_water<S: Store>(store: &S, cfg: &LwsConfig) -> Option<String> {
        let iri = format!("{}{HIGH_WATER_FRAGMENT}", cfg.absolute(SUBSCRIPTIONS_PATH));
        store.read_marker(&iri).await.unwrap()
    }

    /// Store `sub` as `id` and put it in force.
    async fn stored_sub<S: Store>(state: &LwsState<S>, id: &str, mut sub: Subscription) -> String {
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let iri = format!("{container}{id}");
        let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
        state
            .store
            .create_in_container(&container, &iri, body, LWS_JSON)
            .await
            .unwrap();
        sub.id = id.into();
        state.notify.subs.write().unwrap().insert(id.into(), sub);
        iri
    }

    /// Make the subscription `id` past its expiry by the clock, not yet by the stored high water,
    /// in memory only: as the clock moving on past it would.
    async fn due_by_the_clock(state: &LwsState<test_store::FlakyStore>, id: &str) {
        let now = jose::now_secs();
        set_high_water(state, now - 60).await;
        let mut subs = state.notify.subs.write().unwrap();
        subs.get_mut(id).unwrap().expires_at = Some(now - 30);
    }

    /// An inbox that counts the notifications POSTed to each path, answering `204`.
    async fn counting_inbox() -> (std::net::SocketAddr, Arc<Mutex<HashMap<String, usize>>>) {
        let posts: Arc<Mutex<HashMap<String, usize>>> = Default::default();
        let app = {
            let posts = posts.clone();
            axum::Router::new().fallback(axum::routing::post(move |uri: axum::http::Uri| {
                let posts = posts.clone();
                async move {
                    *posts
                        .lock()
                        .unwrap()
                        .entry(uri.path().to_string())
                        .or_default() += 1;
                    StatusCode::NO_CONTENT
                }
            }))
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (addr, posts)
    }

    /// Review finding: a delivery checked its subscription before deciding whether its
    /// subscriber may read the resource, and that decision can wait on a read of metadata; a
    /// subscription that expired meanwhile still got the notification. It is checked again
    /// once the decision is made, and nothing is POSTed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_delivery_whose_subscription_expires_while_it_is_decided_is_not_sent() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.allow_insecure_fetch = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let bob = "https://bob.example/#me";
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
        let (addr, posts) = counting_inbox().await;
        for id in ["s", "t", "u"] {
            let mut sub = sub_until(jose::now_secs() + 3600);
            sub.id = id.into();
            sub.subscriber = Some(bob.into());
            sub.inbox = format!("http://{addr}/{id}");
            state.notify.subs.write().unwrap().insert(id.into(), sub);
        }
        let watch = |id: &str| Watch {
            subscription: id.into(),
            uri: d.clone(),
            agent: Agent {
                subject: Some(bob.into()),
                client: None,
            },
            snapshot: None,
            service: None,
        };
        let latch = |id: &str| {
            let subs = state.notify.subs.read().unwrap();
            subs.get(id).unwrap().lapsed.0.store(true, Ordering::SeqCst);
        };
        let held = || {
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *state.store.hold_next_read_reply_of.lock().unwrap() =
                Some((super::super::meta_key(&d), gate.clone()));
            gate
        };
        let decided = |expire: bool| {
            let (state, watch) = (state.clone(), watch("s"));
            let gate = held();
            async move {
                let check = tokio::spawn({
                    let state = state.clone();
                    async move { watch.stands(&state).await }
                });
                tokio::time::sleep(Duration::from_millis(100)).await;
                assert!(!check.is_finished(), "the decision did not wait");
                if expire {
                    let subs = state.notify.subs.read().unwrap();
                    subs.get("s")
                        .unwrap()
                        .lapsed
                        .0
                        .store(true, Ordering::SeqCst);
                }
                gate.add_permits(1);
                check.await.unwrap()
            }
        };
        assert!(decided(false).await, "the subscriber may read it");
        assert!(!decided(true).await, "sent after it expired");
        // A delivery itself: one that stands is POSTed; one whose subscription expires while its
        // decision waits is not.
        let sent = |id: &'static str| {
            let posts = posts.clone();
            move || posts.lock().unwrap().get(&format!("/{id}")).copied()
        };
        let inbox = |id: &str| format!("http://{addr}/{id}");
        state.notify.deliver(
            &state,
            &inbox("t"),
            json!({"type": ["Update"]}),
            Some(watch("t")),
        );
        for _ in 0..200 {
            if sent("t")().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(sent("t")(), Some(1), "a delivery that stands was not sent");
        let gate = held();
        state.notify.deliver(
            &state,
            &inbox("u"),
            json!({"type": ["Update"]}),
            Some(watch("u")),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        latch("u");
        gate.add_permits(1);
        tokio::time::sleep(RETRY_DELAY + Duration::from_millis(500)).await;
        assert_eq!(sent("u")(), None, "POSTed after it expired");
        assert_eq!(state.notify.admitted.load(Ordering::SeqCst), 0);
    }

    /// Review finding: with the store's subscription pool full, an expired subscription could not
    /// be reclaimed, as its removal began with a write over it, and a write takes room before it
    /// gives any back. Its removal writes nothing to the pool: the time it expired by is kept in
    /// the index alone.
    #[tokio::test]
    async fn expired_subscriptions_are_reclaimed_with_the_pool_full() {
        let far = jose::now_secs() + 3600;
        let size = serde_json::to_vec(&sub_until(far)).unwrap().len();
        let limits = crate::store::InMemoryStoreLimits::default().with_pool(
            crate::store::DEFAULT_IN_MEMORY_MAX_POOL_RESOURCE_COUNT,
            2 * size,
        );
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let state = LwsState::new(test_store::FlakyStore::with_limits(limits), cfg)
            .await
            .unwrap();
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let a = stored_sub(&state, "a", sub_until(far)).await;
        let b = stored_sub(&state, "b", sub_until(far)).await;
        // The pool is full: not even one of them can be written over.
        let body = Bytes::from(serde_json::to_vec(&sub_until(far)).unwrap());
        assert!(matches!(
            state.store.write(&a, body, LWS_JSON).await,
            Err(crate::error::ServerError::InsufficientStorage)
        ));
        due_by_the_clock(&state, "a").await;
        due_by_the_clock(&state, "b").await;
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(!state.store.exists(&a).await.unwrap(), "not reclaimed");
        assert!(!state.store.exists(&b).await.unwrap(), "not reclaimed");
        assert!(state.notify.subs.read().unwrap().is_empty());
        let stored = stored_high_water(&state.store, &state.cfg).await.unwrap();
        assert!(stored.parse::<i64>().unwrap() >= jose::now_secs() - 1);
    }

    /// Review finding: an expiry was seen in memory alone, so a start after a stop, its clock
    /// stepped back, served again a subscription already shown expired. Until the time it
    /// expired by is stored, nothing shows it expired.
    ///
    /// The last step pins the documented residual (see the `solid-lws-server` skill), not a
    /// behaviour wanted for its own sake: the halt and the time owed are held in the process
    /// alone, so a stop after a failed raise followed by a start with the clock stepped back
    /// serves the subscription again. Nothing outside ever saw it expire.
    #[tokio::test]
    async fn nothing_shows_an_expiry_until_stored_and_a_restart_may_revive_it_as_documented() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let iri = stored_sub(&state, "c", sub_until(jose::now_secs() + 3600)).await;
        due_by_the_clock(&state, "c").await;
        store.fail_marker_write.store(true, Ordering::SeqCst);
        let path = format!("{SUBSCRIPTIONS_PATH}c");
        for (method, path) in [
            (Method::GET, path.as_str()),
            (Method::DELETE, path.as_str()),
            (Method::GET, SUBSCRIPTIONS_PATH),
        ] {
            let req = test_store::request(method.clone(), path, &[], "");
            let r = handle(&state, &req, &Agent::anonymous()).await;
            assert_eq!(
                r.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {path}"
            );
        }
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(store.exists(&iri).await.unwrap(), "removed");
        assert!(!state.notify.is_live("c"), "a delivery went ahead");
        // The documented residual: the process stops here (the halt and the time owed go with
        // it) and starts with its clock back before the expiry; the subscription stands again,
        // and nothing ever showed otherwise.
        store.fail_marker_write.store(false, Ordering::SeqCst);
        let restarted = Notifier::load(&store, &state.cfg).await.unwrap();
        assert!(restarted.is_live("c"));
    }

    /// Review finding, the other half: once an expiry has been shown, a start with the clock
    /// stepped back before it finds the subscription expired, as the time it was shown by is
    /// stored first.
    #[tokio::test]
    async fn an_expiry_shown_stays_shown_with_the_clock_stepped_back() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        stored_sub(&state, "r", sub_until(jose::now_secs() + 3600)).await;
        due_by_the_clock(&state, "r").await;
        let get = test_store::request(Method::GET, &format!("{SUBSCRIPTIONS_PATH}r"), &[], "");
        let r = handle(&state, &get, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let stored = stored_high_water(&store, &state.cfg).await.unwrap();
        assert!(
            stored.parse::<i64>().unwrap() >= jose::now_secs() - 1,
            "not stored"
        );
        // A start whose clock is an hour behind what was stored: one expiring before what is
        // stored is expired, one after it stands. One that cannot be removed is kept, expired.
        let now = jose::now_secs();
        set_high_water(&state, now + 3600).await;
        let gone = stored_sub(&state, "gone", sub_until(now + 1800)).await;
        let kept = stored_sub(&state, "kept", sub_until(now + 1800)).await;
        stored_sub(&state, "later", sub_until(now + 7200)).await;
        *store.fail_delete_of.lock().unwrap() = Some(kept.clone());
        let restarted = Notifier::load(&store, &state.cfg).await.unwrap();
        assert!(!store.exists(&gone).await.unwrap(), "not purged");
        assert!(store.exists(&kept).await.unwrap());
        assert!(!restarted.is_live("kept"), "served again");
        assert!(!restarted.is_live("gone"));
        assert!(restarted.is_live("later"));
        let listed: Vec<String> = restarted
            .live_for(None, true, restarted.clock())
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(listed, ["later"]);
    }

    /// A removal whose delete's outcome is unknown leaves the subscription expired: a start
    /// meanwhile, its clock stepped back, still finds it so and removes it, and the removal is
    /// settled later with nothing put back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_expired_subscription_whose_removal_is_unknown_stays_expired() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let now = jose::now_secs();
        set_high_water(&state, now + 3600).await;
        let iri = stored_sub(&state, "u", sub_until(now + 1800)).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let removed = state
            .notify
            .remove(
                &state,
                "u",
                None,
                super::super::records::Remover::Request,
                true,
            )
            .await;
        let Err(r) = removed else {
            panic!("the removal was taken as known");
        };
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(store.exists(&iri).await.unwrap());
        assert!(!state.notify.is_live("u"));
        let get = test_store::request(Method::GET, &format!("{SUBSCRIPTIONS_PATH}u"), &[], "");
        let r = handle(&state, &get, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        // A start now, an hour behind what is stored, finds it expired and removes it.
        let restarted = Notifier::load(&store, &state.cfg).await.unwrap();
        assert!(!restarted.is_live("u"), "served again");
        assert!(!store.exists(&iri).await.unwrap());
        // The first removal is settled once the store lets it: gone.
        store.fail_fence.store(false, Ordering::SeqCst);
        for _ in 0..250 {
            if state.notify.subs.read().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            state.notify.subs.read().unwrap().is_empty(),
            "never settled"
        );
        drop(gate);
    }

    /// A start that finds no high water stored, or one that is not a time, while subscriptions
    /// with an expiry are stored cannot tell whether they expired: every one of them is expired,
    /// and one is stored only once they are gone. Those without an expiry stand, and a fresh
    /// storage simply has one stored.
    #[tokio::test]
    async fn a_missing_or_unreadable_high_water_expires_what_could_have_expired() {
        let cfg = LwsConfig::new("http://localhost:3000");
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        let fresh = test_store::FlakyStore::new();
        Notifier::load(&fresh, &cfg).await.unwrap();
        let first = stored_high_water(&fresh, &cfg).await.expect("not stored");
        assert!(first.parse::<i64>().is_ok());
        let marker = format!("{container}{HIGH_WATER_FRAGMENT}");
        for corrupt in [None, Some("soon")] {
            let store = test_store::FlakyStore::new();
            store
                .write(&container, Bytes::new(), LWS_JSON)
                .await
                .unwrap();
            if let Some(v) = corrupt {
                store.plant_marker(&marker, v).await;
            }
            let mut stored = Vec::new();
            for (id, sub) in [
                ("limited", sub_until(jose::now_secs() + 3600)),
                ("stuck", sub_until(jose::now_secs() + 3600)),
                (
                    "open",
                    Subscription {
                        expires_at: None,
                        ..sub_until(0)
                    },
                ),
            ] {
                let iri = format!("{container}{id}");
                let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
                store
                    .create_in_container(&container, &iri, body, LWS_JSON)
                    .await
                    .unwrap();
                stored.push(iri);
            }
            // One of them cannot be removed: kept, expired, and nothing stored while it is.
            *store.fail_delete_of.lock().unwrap() = Some(stored[1].clone());
            let loaded = Notifier::load(&store, &cfg).await.unwrap();
            assert!(!store.exists(&stored[0]).await.unwrap(), "{corrupt:?}");
            assert!(!loaded.is_live("limited"), "{corrupt:?}");
            assert!(!loaded.is_live("stuck"), "{corrupt:?}");
            assert!(loaded.is_live("open"), "{corrupt:?}");
            let left = stored_high_water(&store, &cfg).await;
            assert!(
                left.as_deref() == corrupt,
                "stored while one is left: {left:?}"
            );
            // Once it is removed, the next start stores one.
            *store.fail_delete_of.lock().unwrap() = None;
            let loaded = Notifier::load(&store, &cfg).await.unwrap();
            assert!(!store.exists(&stored[1]).await.unwrap(), "{corrupt:?}");
            assert!(loaded.is_live("open"));
            let now = stored_high_water(&store, &cfg).await.unwrap();
            assert!(now.parse::<i64>().is_ok(), "{corrupt:?}: {now}");
        }
    }

    /// Review findings: a raise of the high water that failed stopped only some deliveries and
    /// kept nothing of the time it was for, so a later raise at an earlier clock could let them
    /// resume. A failed raise halts the service: nothing is delivered (to any subscription),
    /// subscribed, cancelled, listed or removed, and a clock stepped back resumes nothing. Only
    /// a raise to the time that was owed ends the halt, and what expired by it stays expired.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_raise_halts_notifications_until_what_was_owed_is_stored() {
        let (addr, posts) = counting_inbox().await;
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        let store = test_store::FlakyStore::new();
        let state = LwsState::new(store.clone(), cfg).await.unwrap();
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        for (id, expires_at) in [
            ("limited", Some(jose::now_secs() + 3600)),
            ("due", Some(jose::now_secs() + 3600)),
            ("open", None),
        ] {
            let sub = Subscription {
                topics: vec![state.cfg.storage()],
                inbox: format!("http://{addr}/{id}"),
                expires_at,
                ..sub_until(0)
            };
            stored_sub(&state, id, sub).await;
        }
        due_by_the_clock(&state, "due").await;
        let due = state.cfg.absolute(&format!("{SUBSCRIPTIONS_PATH}due"));
        store.fail_marker_write.store(true, Ordering::SeqCst);
        let event = Event {
            kind: "Update",
            uri: state.cfg.absolute("/x"),
            is_container: false,
            relation: None,
        };
        let count = |id: &str| {
            let n = posts.lock().unwrap().get(&format!("/{id}")).copied();
            n.unwrap_or(0)
        };
        let settle = || async {
            for _ in 0..200 {
                if state.notify.admitted.load(Ordering::SeqCst) == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let owed = jose::now_secs();
        // The delivery to the expired subscription halts the service. Those to the others run
        // alongside it, and one decided before the halt may go out; none decided after it does.
        state.notify.announce(&state, event.clone()).await;
        settle().await;
        assert!(state.notify.halted(), "not halted");
        assert_eq!(count("due"), 0, "delivered after it expired");
        let before = (count("open"), count("limited"));
        state.notify.announce(&state, event.clone()).await;
        settle().await;
        assert_eq!(
            (count("open"), count("limited")),
            before,
            "delivered while halted"
        );
        let body = json!({"type": WEBHOOK, "topic": [state.cfg.storage()], "inbox": "https://i/"});
        let requests = [
            (Method::GET, SUBSCRIPTIONS_PATH.to_string(), String::new()),
            (
                Method::POST,
                SUBSCRIPTIONS_PATH.to_string(),
                body.to_string(),
            ),
            (
                Method::GET,
                format!("{SUBSCRIPTIONS_PATH}open"),
                String::new(),
            ),
            (
                Method::DELETE,
                format!("{SUBSCRIPTIONS_PATH}open"),
                String::new(),
            ),
            (
                Method::GET,
                format!("{SUBSCRIPTIONS_PATH}nothing"),
                String::new(),
            ),
        ];
        let answers = || async {
            let mut answers = Vec::new();
            for (method, path, body) in &requests {
                let headers = [("content-type", LWS_JSON)];
                let req = test_store::request(method.clone(), path, &headers, body);
                answers.push(handle(&state, &req, &Agent::anonymous()).await.status());
            }
            answers
        };
        assert!(
            answers()
                .await
                .iter()
                .all(|s| *s == StatusCode::SERVICE_UNAVAILABLE),
            "answered while halted"
        );
        // The store still refuses, and the clock steps back an hour: nothing resumes, nothing is
        // removed.
        state.notify.clock_offset.store(-3600, Ordering::SeqCst);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.raised().await;
        sweeper.settled().await;
        assert!(state.notify.halted());
        assert!(store.exists(&due).await.unwrap(), "removed while halted");
        state.notify.announce(&state, event.clone()).await;
        settle().await;
        assert_eq!(count("open"), before.0, "resumed in the process");
        // The store takes it again, and the sweeper, having failed once, waits before it raises
        // again; a request makes no attempt of its own.
        store.fail_marker_write.store(false, Ordering::SeqCst);
        let writes = store.marker_writes.load(Ordering::SeqCst);
        assert!(answers()
            .await
            .iter()
            .all(|s| *s == StatusCode::SERVICE_UNAVAILABLE));
        sweeper.sweep(&state);
        assert!(sweeper.records.is_empty(), "raised again before its wait");
        assert_eq!(store.marker_writes.load(Ordering::SeqCst), writes);
        assert!(state.notify.halted());
        // Once it has waited, what was owed is stored, not the earlier clock, and the expired
        // subscription stays expired.
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.raised().await;
        assert!(!state.notify.halted());
        let stored = stored_high_water(&store, &state.cfg).await.unwrap();
        assert!(
            stored.parse::<i64>().unwrap() >= owed,
            "stored {stored}, owed {owed}"
        );
        state.notify.announce(&state, event).await;
        settle().await;
        assert_eq!(count("open"), before.0 + 1);
        assert_eq!(count("limited"), before.1 + 1);
        assert_eq!(count("due"), 0, "delivered after it expired");
    }

    /// Review finding: the store raised the high water as a read and a write, so a raise that
    /// failed and landed late could store an earlier time over a later one. A raise is one step
    /// at the store: whatever order raises land in, the later time stands.
    #[tokio::test]
    async fn a_late_lower_raise_lowers_nothing() {
        let store = test_store::FlakyStore::new();
        let iri = "http://localhost:3000/.lws/subscriptions/#high-water";
        assert_eq!(store.raise_marker(iri, 300).await.unwrap(), 300);
        assert_eq!(store.raise_marker(iri, 200).await.unwrap(), 300);
        assert_eq!(
            store.read_marker(iri).await.unwrap().as_deref(),
            Some("300")
        );
        // Over what does not read as a time, a raise stores its own.
        store.plant_marker(iri, "soon").await;
        assert_eq!(store.raise_marker(iri, 200).await.unwrap(), 200);
    }

    /// Notifications start only over a store whose raise is atomic, and refuse any other before
    /// anything is read or written.
    #[tokio::test]
    async fn notifications_refuse_a_store_without_an_atomic_raise() {
        let store = test_store::FlakyStore::new();
        store.no_atomic_markers.store(true, Ordering::SeqCst);
        let cfg = LwsConfig::new("http://localhost:3000");
        let root = cfg.storage();
        let refused = LwsState::new(store.clone(), cfg.clone()).await;
        assert!(refused.is_err());
        assert!(
            !store.exists(&root).await.unwrap(),
            "written before refusing"
        );
        assert!(Notifier::load(&store, &cfg).await.is_err());
    }

    /// Review finding: the marker's first write needed room in the subscription pool, and a
    /// start whose first write failed went on taking subscriptions, so a pool full of them left
    /// no room for the marker every removal needs. The marker has a slot of its own, admitted
    /// however full the pool is, and no subscription is created until it is stored.
    #[tokio::test]
    async fn the_first_mark_is_stored_with_a_full_pool_and_comes_before_any_subscription() {
        let limits = crate::store::InMemoryStoreLimits::default()
            .with_pool(3, crate::store::DEFAULT_IN_MEMORY_MAX_POOL_BYTES);
        let store = test_store::FlakyStore::with_limits(limits);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        store
            .write(&container, Bytes::new(), LWS_JSON)
            .await
            .unwrap();
        // A pool filled before any mark was kept: subscriptions with no expiry.
        let open = Subscription {
            expires_at: None,
            ..sub_until(0)
        };
        let body = Bytes::from(serde_json::to_vec(&open).unwrap());
        for id in ["a", "b"] {
            let iri = format!("{container}{id}");
            store
                .create_in_container(&container, &iri, body.clone(), LWS_JSON)
                .await
                .unwrap();
        }
        let third = format!("{container}c");
        assert!(store
            .create_in_container(&container, &third, body.clone(), LWS_JSON)
            .await
            .is_err());
        let state = LwsState::new(store.clone(), cfg.clone()).await.unwrap();
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        assert!(stored_high_water(&store, &cfg).await.is_some(), "no mark");
        assert!(!state.notify.halted());
        // With the mark, one that expires is reclaimed.
        due_by_the_clock(&state, "a").await;
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(!store.exists(&format!("{container}a")).await.unwrap());
        // A start whose first mark cannot be stored takes no subscription until one is.
        let fresh = test_store::FlakyStore::new();
        fresh.fail_marker_write.store(true, Ordering::SeqCst);
        let state = LwsState::new(fresh.clone(), cfg).await.unwrap();
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let post = || {
            let body =
                json!({"type": WEBHOOK, "topic": [state.cfg.storage()], "inbox": "https://i/"});
            let headers = [("content-type", LWS_JSON)];
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &headers,
                &body.to_string(),
            )
        };
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(state.notify.subs.read().unwrap().is_empty());
        fresh.fail_marker_write.store(false, Ordering::SeqCst);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.raised().await;
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// A subscriber's DELETE of its subscription that expires while the removal waits for its
    /// lock is decided once the lock is held: the subscription is gone to its subscriber, `404`,
    /// as a DELETE of one already expired is, and it is left for the expiry sweeper.
    #[tokio::test]
    async fn a_cancellation_of_a_subscription_that_expires_meanwhile_is_a_404() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let iri = stored_sub(&state, "w", sub_until(jose::now_secs() + 3600)).await;
        let path = format!("{SUBSCRIPTIONS_PATH}w");
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let held = state.locks.lock(&iri).await.expect("held");
        let cancel = tokio::spawn({
            let state = state.clone();
            async move { handle(&state, &delete, &Agent::anonymous()).await.status() }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!cancel.is_finished());
        due_by_the_clock(&state, "w").await;
        drop(held);
        assert_eq!(cancel.await.unwrap(), StatusCode::NOT_FOUND);
        assert!(store.exists(&iri).await.unwrap(), "removed as live");
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let r = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(!store.exists(&iri).await.unwrap());
    }

    /// A subscriber's DELETE of its subscription that expires while the removal waits for its
    /// lock, when that expiry cannot be stored, is a `503`: the subscription is neither removed
    /// nor shown expired, and the service halts.
    #[tokio::test]
    async fn a_cancellation_whose_expiry_cannot_be_stored_meanwhile_is_a_503() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let iri = stored_sub(&state, "w", sub_until(jose::now_secs() + 3600)).await;
        let path = format!("{SUBSCRIPTIONS_PATH}w");
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let held = state.locks.lock(&iri).await.expect("held");
        let cancel = tokio::spawn({
            let state = state.clone();
            async move { handle(&state, &delete, &Agent::anonymous()).await.status() }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!cancel.is_finished());
        due_by_the_clock(&state, "w").await;
        let high_water = state.notify.high_water();
        store.fail_marker_write.store(true, Ordering::SeqCst);
        drop(held);
        assert_eq!(cancel.await.unwrap(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(store.exists(&iri).await.unwrap(), "removed");
        assert!(state.notify.halted());
        assert_eq!(state.notify.high_water(), high_water);
        let subs = state.notify.subs.read().unwrap();
        assert!(!subs.get("w").unwrap().expired(high_water), "shown expired");
    }

    /// A start that finds a subscription past its expiry by the clock and cannot store the
    /// clock as the high water keeps it, stored and in memory, expired by nothing stored, and
    /// halts: the high water it holds is the one it read.
    #[tokio::test]
    async fn a_start_whose_raise_fails_removes_nothing_and_keeps_the_mark_it_read() {
        let cfg = LwsConfig::new("http://localhost:3000");
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        let store = test_store::FlakyStore::new();
        store
            .write(&container, Bytes::new(), LWS_JSON)
            .await
            .unwrap();
        let now = jose::now_secs();
        let marker = format!("{container}{HIGH_WATER_FRAGMENT}");
        store.plant_marker(&marker, &(now - 60).to_string()).await;
        let iri = format!("{container}due");
        let body = Bytes::from(serde_json::to_vec(&sub_until(now - 30)).unwrap());
        store
            .create_in_container(&container, &iri, body, LWS_JSON)
            .await
            .unwrap();
        store.fail_marker_write.store(true, Ordering::SeqCst);
        let loaded = Notifier::load(&store, &cfg).await.unwrap();
        assert!(store.exists(&iri).await.unwrap(), "removed");
        assert!(loaded.halted());
        assert!(!loaded.is_live("due"));
        assert_eq!(loaded.high_water(), now - 60);
        let lapsed = loaded
            .subs
            .read()
            .unwrap()
            .get("due")
            .map(|s| s.lapsed.0.load(Ordering::SeqCst));
        assert_eq!(lapsed, Some(false), "dropped from memory, or shown expired");
        assert_eq!(
            stored_high_water(&store, &cfg).await,
            Some((now - 60).to_string())
        );
    }

    /// A start with no high water stored that could not remove every subscription it expired
    /// for want of one finishes at runtime: the expiry sweeper removes what is left, and only
    /// then stores the first mark; until then no subscription is created.
    #[tokio::test]
    async fn what_a_start_could_not_remove_is_removed_before_the_first_mark() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        let store = test_store::FlakyStore::new();
        store
            .write(&container, Bytes::new(), LWS_JSON)
            .await
            .unwrap();
        let mut stored = Vec::new();
        for id in ["limited", "stuck"] {
            let iri = format!("{container}{id}");
            let body =
                Bytes::from(serde_json::to_vec(&sub_until(jose::now_secs() + 3600)).unwrap());
            store
                .create_in_container(&container, &iri, body, LWS_JSON)
                .await
                .unwrap();
            stored.push(iri);
        }
        *store.fail_delete_of.lock().unwrap() = Some(stored[1].clone());
        let state = LwsState::new(store.clone(), cfg).await.unwrap();
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        assert!(!store.exists(&stored[0]).await.unwrap());
        assert!(store.exists(&stored[1]).await.unwrap());
        let post = || {
            let body =
                json!({"type": WEBHOOK, "topic": [state.cfg.storage()], "inbox": "https://i/"});
            let headers = [("content-type", LWS_JSON)];
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &headers,
                &body.to_string(),
            )
        };
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        // While it still cannot be removed, nothing is stored.
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        sweeper.sweep(&state);
        sweeper.raised().await;
        assert!(store.exists(&stored[1]).await.unwrap());
        assert_eq!(stored_high_water(&store, &state.cfg).await, None);
        assert!(
            !state.notify.halted(),
            "a wait for a removal halted the service"
        );
        // Once it can be, it is removed, and then the first mark is stored.
        *store.fail_delete_of.lock().unwrap() = None;
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(!store.exists(&stored[1]).await.unwrap());
        sweeper.sweep(&state);
        sweeper.raised().await;
        let mark = stored_high_water(&store, &state.cfg)
            .await
            .expect("no mark");
        assert!(mark.parse::<i64>().is_ok());
        let r = handle(&state, &post(), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// Review finding: a create whose purge of its subscriber's own expired subscriptions
    /// halted the service (the raise of the high water failed) went on, and an unconditional or
    /// existence-only create stored its subscription and answered `201` while halted. It answers
    /// `503` and creates nothing.
    #[tokio::test]
    async fn a_create_whose_purge_halts_the_service_creates_nothing() {
        for conditions in [vec![], vec![("if-match", "*")]] {
            let (state, store) = test_store::state(100).await;
            state.notify.sweeper_off.store(true, Ordering::SeqCst);
            let due = stored_sub(&state, "due", sub_until(jose::now_secs() + 3600)).await;
            due_by_the_clock(&state, "due").await;
            store.fail_marker_write.store(true, Ordering::SeqCst);
            let body =
                json!({"type": WEBHOOK, "topic": [state.cfg.storage()], "inbox": "https://i/"});
            let mut headers = vec![("content-type", LWS_JSON)];
            headers.extend(conditions.iter().copied());
            let req = test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &headers,
                &body.to_string(),
            );
            let r = handle(&state, &req, &Agent::anonymous()).await;
            assert_eq!(
                r.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{conditions:?}"
            );
            assert!(state.notify.halted());
            assert!(store.exists(&due).await.unwrap(), "{conditions:?}");
            let ids: Vec<String> = state.notify.subs.read().unwrap().keys().cloned().collect();
            assert_eq!(ids, ["due"], "{conditions:?}");
            let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
            assert_eq!(
                store.list_children(&container).await.unwrap().len(),
                1,
                "{conditions:?}"
            );
        }
    }

    /// Review finding: a create checked the halt and its expiry before waiting for its listing
    /// locks and storing, and shared no lock with a raise, so a raise that failed meanwhile
    /// (halting the service, the stored high water unchanged) let it store and register its
    /// subscription and answer `201`. The checks are made again under the raise lock, once the
    /// listing locks are held, and that lock is held until the create is settled.
    #[tokio::test]
    async fn a_create_waiting_while_a_raise_fails_creates_nothing() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let held = state.locks.lock(&container).await.expect("held");
        let create = tokio::spawn({
            let state = state.clone();
            async move {
                let body = json!({
                    "type": WEBHOOK,
                    "topic": [state.cfg.storage()],
                    "inbox": "https://i/",
                    "expires": format_rfc3339(jose::now_secs() + 3600),
                });
                let headers = [("content-type", LWS_JSON)];
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &headers,
                    &body.to_string(),
                );
                handle(&state, &req, &Agent::anonymous()).await.status()
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!create.is_finished());
        // Meanwhile a raise fails: a read of a subscription past its expiry by the clock.
        let due = stored_sub(&state, "due", sub_until(jose::now_secs() + 3600)).await;
        due_by_the_clock(&state, "due").await;
        store.fail_marker_write.store(true, Ordering::SeqCst);
        let get = test_store::request(Method::GET, &format!("{SUBSCRIPTIONS_PATH}due"), &[], "");
        let r = handle(&state, &get, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(state.notify.halted());
        drop(held);
        assert_eq!(create.await.unwrap(), StatusCode::SERVICE_UNAVAILABLE);
        let ids: Vec<String> = state.notify.subs.read().unwrap().keys().cloned().collect();
        assert_eq!(ids, ["due"]);
        let stored = store.list_children(&container).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].as_str(), due);
    }

    /// Review finding: a second state could be built over a running one's store (a clone of
    /// it), with a registry of its own, and in the recovery mode lower the high water under the
    /// running service. The backend's lease is held for as long as a state lives: a second is
    /// refused, and one built once the first is dropped is not.
    #[tokio::test]
    async fn a_second_state_over_a_running_ones_store_is_refused() {
        let (state, _store) = test_store::state(100).await;
        let mut cfg = state.cfg.clone();
        cfg.reset_subscription_clock = true;
        let backend = state.store.clone();
        let refused = LwsState::new(backend.clone(), cfg.clone()).await;
        let Err(why) = refused else {
            panic!("a second state was built");
        };
        assert!(why.contains("another LWS service"), "{why}");
        drop(state);
        let state = LwsState::new(backend, cfg).await.unwrap();
        assert!(state.notify.lease().is_some());
    }

    /// The store a running state hands out reaches the high water only through its raise:
    /// putting a held record back, or any other generic write, under the marker's IRI is
    /// refused, and the high water stays as stored.
    #[tokio::test]
    async fn the_states_store_cannot_lower_the_high_water() {
        let (state, _store) = test_store::state(100).await;
        let marker = format!(
            "{}{HIGH_WATER_FRAGMENT}",
            state.cfg.absolute(SUBSCRIPTIONS_PATH)
        );
        let before = stored_high_water(&state.store, &state.cfg).await;
        assert!(before.is_some());
        let lower = crate::store::sparq::marker_meta("1");
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        for parent in [None, Some(container.as_str())] {
            let r = state.store.reinstate(&marker, parent, &lower).await;
            assert!(
                matches!(r, Err(crate::error::ServerError::Forbidden)),
                "{r:?}"
            );
            let r = state
                .store
                .restore(&marker, parent, Bytes::from_static(b"1"), &lower)
                .await;
            assert!(
                matches!(r, Err(crate::error::ServerError::Forbidden)),
                "{r:?}"
            );
            let r = state.store.delete(&marker, parent).await;
            assert!(
                matches!(r, Err(crate::error::ServerError::Forbidden)),
                "{r:?}"
            );
        }
        let r = state
            .store
            .write(&marker, Bytes::from_static(b"1"), LWS_JSON)
            .await;
        assert!(
            matches!(r, Err(crate::error::ServerError::Forbidden)),
            "{r:?}"
        );
        assert_eq!(stored_high_water(&state.store, &state.cfg).await, before);
    }

    /// Review of #7138: a create purged the first of its subscriber's lapsing subscriptions in
    /// id order, so expired ones it could not remove used up the purge, and those only past
    /// their expiry by the clock were left undecided and counted against the subscriber's
    /// capacity (a `429` with none of them live). Every one of them is decided before counting.
    #[tokio::test]
    async fn a_create_counts_none_of_its_subscribers_due_subscriptions() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let me = "https://alice.example/#me";
        let now = jose::now_secs();
        set_high_water(&state, now - 60).await;
        let mine = |expires_at: i64| Subscription {
            subscriber: Some(me.into()),
            ..sub_until(expires_at)
        };
        // Expired by the stored high water, and first in id order: their removal fails.
        for i in 0..EXPIRED_REMOVED_PER_SUBSCRIBE {
            stored_sub(&state, &format!("a{i:02}"), mine(now - 120)).await;
        }
        // Past their expiry by the clock alone, as many as the subscriber may hold.
        for i in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            stored_sub(&state, &format!("z{i:02}"), mine(now - 30)).await;
        }
        store.fail_delete.store(true, Ordering::SeqCst);
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let req = test_store::request(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let status = handle(&state, &req, &test_store::agent(me)).await.status();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "expired subscriptions were counted"
        );
        assert!(
            state.notify.high_water() >= now - 30,
            "decided before counting"
        );
    }

    /// Review of #7138: the check that a reclaim removes only a subscription still expired
    /// once its removal decides (`Notifier::remove`) had no test. The sweeper picks one past its
    /// expiry by the clock alone; the clock steps back before the removal runs: it stands, and
    /// is neither removed nor taken out of force.
    #[tokio::test]
    async fn a_reclaim_leaves_a_subscription_the_clock_stepped_back_before() {
        let (state, _store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let now = jose::now_secs();
        set_high_water(&state, now - 60).await;
        let iri = stored_sub(&state, "s", sub_until(now - 30)).await;
        // Chosen while it is past its expiry by the clock...
        assert!(state
            .notify
            .subs
            .read()
            .unwrap()
            .get("s")
            .unwrap()
            .lapsing(state.notify.high_water(), state.notify.clock()));
        // ...and the clock steps back before it is removed.
        state.notify.clock_offset.store(-120, Ordering::SeqCst);
        let by = super::super::records::Remover::Sweeper;
        let removed = state.notify.remove(&state, "s", None, by, true).await;
        assert!(matches!(
            removed,
            Ok(super::super::records::Removed::Absent)
        ));
        assert!(
            state.notify.is_live("s"),
            "a subscription that stands was removed"
        );
        assert!(state.store.exists(&iri).await.unwrap());
    }

    /// The create body for a subscription by the storage's anonymous subscriber, expiring at
    /// `expires_at`.
    fn expiring_body(state: &LwsState<test_store::FlakyStore>, expires_at: i64) -> String {
        json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
            "expires": super::super::access::format_rfc3339(expires_at),
        })
        .to_string()
    }

    /// Review of #7138: the create's recheck of its expiry under the raise lock had no test.
    /// The high water passes the expiry while the create waits for that lock: it is refused,
    /// `400`, and nothing is stored.
    #[tokio::test]
    async fn a_create_whose_expiry_passes_while_it_waits_is_refused() {
        let (state, _store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let expires_at = jose::now_secs() + 30;
        let writing = state.notify.high_water.writing.clone();
        let held = writing.lock_owned().await;
        let creating = {
            let state = state.clone();
            let body = expiring_body(&state, expires_at);
            tokio::spawn(async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                handle(&state, &req, &Agent::anonymous()).await.status()
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !creating.is_finished(),
            "the create waits on the raise lock"
        );
        set_high_water(&state, expires_at).await;
        drop(held);
        assert_eq!(creating.await.unwrap(), StatusCode::BAD_REQUEST);
        assert!(state.notify.subs.read().unwrap().is_empty());
    }

    /// Review of #7138: that a create holds the raise lock through its write had no test. A
    /// raise that fails while the create's write is under way waits for it: the create is
    /// answered with the service not halted, and only then does the raise fail and halt it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_raise_waits_for_a_create_under_way() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let expires_at = jose::now_secs() + 3600;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_create.lock().unwrap() = Some(gate.clone());
        let creating = {
            let state = state.clone();
            let body = expiring_body(&state, expires_at);
            tokio::spawn(async move {
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                handle(&state, &req, &Agent::anonymous()).await.status()
            })
        };
        // The write is under way (its gate taken).
        while store.hold_next_create.lock().unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        store.fail_marker_write.store(true, Ordering::SeqCst);
        let raising = {
            let state = state.clone();
            let to = state.notify.high_water() + 10;
            tokio::spawn(async move { state.notify.raise(&state.store, to).await })
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !raising.is_finished(),
            "the raise did not wait for the write"
        );
        assert!(!state.notify.halted());
        gate.add_permits(1);
        // A `201` was decided with the service not halted (a halted one answers `503`); whether
        // the raise has failed by the time the create's task ends is not this test's to say.
        assert_eq!(creating.await.unwrap(), StatusCode::CREATED);
        assert!(!raising.await.unwrap());
        assert!(state.notify.halted());
    }

    /// A wrapper of a running state's store keeps that state's lease: a state built over it,
    /// wrapped once or twice more, is refused, so it cannot run the recovery (and lower the high
    /// water) under the running service.
    #[tokio::test]
    async fn a_state_over_a_wrapper_of_a_running_ones_store_is_refused() {
        use super::super::tracked::Tracked;
        let (state, _store) = test_store::state(100).await;
        let mut cfg = state.cfg.clone();
        cfg.reset_subscription_clock = true;
        let root = cfg.storage();
        let marks_before = _store.marker_writes.load(Ordering::SeqCst);
        let once = Tracked::new(state.store.clone(), root.clone());
        let Err(why) = LwsState::new(once, cfg.clone()).await else {
            panic!("a state was built over a wrapper of a running one's store");
        };
        assert!(why.contains("another LWS service"), "{why}");
        let twice = Tracked::new(
            Tracked::new(state.store.clone(), root.clone()),
            root.clone(),
        );
        let Err(why) = LwsState::new(twice, cfg.clone()).await else {
            panic!("a state was built over a nested wrapper of a running one's store");
        };
        assert!(why.contains("another LWS service"), "{why}");
        assert_eq!(
            _store.marker_writes.load(Ordering::SeqCst),
            marks_before,
            "a refused state wrote no marker"
        );
        // Once the running state is gone, a wrapper of its store may be served.
        let backend = state.store.clone();
        drop(state);
        let state = LwsState::new(Tracked::new(backend, root), cfg)
            .await
            .unwrap();
        assert!(state.notify.lease().is_some());
    }

    /// A conditional create that waits for the listing it compares, while a subscription
    /// registered meanwhile passes its expiry by the clock, decides what that listing leaves out
    /// once it holds it: the subscription is left out only with its expiry stored.
    #[tokio::test]
    async fn a_conditional_create_decides_its_listing_once_it_holds_it() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        let held = state.locks.lock(&container).await.expect("held");
        let create = tokio::spawn({
            let state = state.clone();
            async move {
                let body =
                    json!({"type": WEBHOOK, "topic": [state.cfg.storage()], "inbox": "https://i/"});
                let headers = [("content-type", LWS_JSON), ("if-match", "\"other\"")];
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &headers,
                    &body.to_string(),
                );
                handle(&state, &req, &Agent::anonymous()).await.status()
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!create.is_finished());
        // Registered while the create waits, and past its expiry by the clock before it is
        // compared.
        stored_sub(&state, "a", sub_until(jose::now_secs() + 3600)).await;
        due_by_the_clock(&state, "a").await;
        let expires_at = state
            .notify
            .subs
            .read()
            .unwrap()
            .get("a")
            .unwrap()
            .expires_at;
        let expires_at = expires_at.unwrap();
        drop(held);
        assert_eq!(create.await.unwrap(), StatusCode::PRECONDITION_FAILED);
        let mark: i64 = stored_high_water(&store, &state.cfg)
            .await
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            mark >= expires_at,
            "left out with {mark} stored, expiry {expires_at}"
        );
    }

    /// A new subscription must expire after the stored high water as well as after the clock:
    /// one the high water has already passed would be expired as it is made.
    #[tokio::test]
    async fn a_subscription_expiring_before_the_stored_high_water_is_refused() {
        let (state, _store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let now = jose::now_secs();
        set_high_water(&state, now + 3600).await;
        let post = |at: i64| {
            let body = json!({
                "type": WEBHOOK,
                "topic": [state.cfg.storage()],
                "inbox": "https://i/",
                "expires": format_rfc3339(at),
            });
            let headers = [("content-type", LWS_JSON)];
            test_store::request(
                Method::POST,
                SUBSCRIPTIONS_PATH,
                &headers,
                &body.to_string(),
            )
        };
        let r = handle(&state, &post(now + 1800), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert!(state.notify.subs.read().unwrap().is_empty());
        let r = handle(&state, &post(now + 7200), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// Review finding: a stored high water far ahead of the clock was taken as unreadable, which
    /// expired every subscription with an expiry in memory only, so one whose removal failed came
    /// back at a start with the clock corrected. Such a mark stands: what expired by it stays
    /// expired, across a restart with the clock moved up to it, until a start in the operator's
    /// recovery mode (`LwsConfig::reset_subscription_clock`) removes every subscription with an
    /// expiry, failing while one is left, and only then stores the clock.
    #[tokio::test]
    async fn a_high_water_far_ahead_of_the_clock_stands_until_the_operator_resets_it() {
        let cfg = LwsConfig::new("http://localhost:3000");
        let container = cfg.absolute(SUBSCRIPTIONS_PATH);
        let marker = format!("{container}{HIGH_WATER_FRAGMENT}");
        let now = jose::now_secs();
        let ahead = 2 * MAX_HIGH_WATER_AHEAD;
        let store = test_store::FlakyStore::new();
        store
            .write(&container, Bytes::new(), LWS_JSON)
            .await
            .unwrap();
        store
            .plant_marker(&marker, &(now + ahead).to_string())
            .await;
        let open = Subscription {
            expires_at: None,
            ..sub_until(0)
        };
        let mut stored = Vec::new();
        for (id, sub) in [
            ("before", sub_until(now + MAX_HIGH_WATER_AHEAD)),
            ("after", sub_until(now + 3 * MAX_HIGH_WATER_AHEAD)),
            ("open", open),
        ] {
            let iri = format!("{container}{id}");
            let body = Bytes::from(serde_json::to_vec(&sub).unwrap());
            store
                .create_in_container(&container, &iri, body, LWS_JSON)
                .await
                .unwrap();
            stored.push(iri);
        }
        *store.fail_delete_of.lock().unwrap() = Some(stored[0].clone());
        let mark = (now + ahead).to_string();
        let loaded = Notifier::load(&store, &cfg).await.unwrap();
        assert!(store.exists(&stored[0]).await.unwrap());
        assert!(!loaded.is_live("before"));
        assert!(loaded.is_live("after"));
        assert!(loaded.is_live("open"));
        assert!(!loaded.halted());
        assert_eq!(loaded.high_water(), now + ahead);
        assert_eq!(stored_high_water(&store, &cfg).await, Some(mark.clone()));
        // The clock moved up to the mark: still expired.
        let restarted = Notifier::new(&cfg).unwrap();
        restarted.clock_offset.store(ahead, Ordering::SeqCst);
        restarted.load_with(&store, &cfg, |_| true).await.unwrap();
        assert!(store.exists(&stored[0]).await.unwrap());
        assert!(!restarted.is_live("before"));
        assert!(restarted.is_live("after"));
        // A start in the recovery mode fails while one with an expiry is left, and lowers
        // nothing (what it could remove is gone).
        let mut recovery = cfg.clone();
        recovery.reset_subscription_clock = true;
        assert!(LwsState::new(store.clone(), recovery.clone())
            .await
            .is_err());
        assert_eq!(stored_high_water(&store, &cfg).await, Some(mark));
        assert!(!store.exists(&stored[1]).await.unwrap());
        // Once it can be removed, every one with an expiry goes, and then the clock is stored.
        *store.fail_delete_of.lock().unwrap() = None;
        let recovered = LwsState::new(store.clone(), recovery).await.unwrap();
        recovered.notify.sweeper_off.store(true, Ordering::SeqCst);
        assert!(recovered.notify.is_live("open"));
        assert!(!store.exists(&stored[0]).await.unwrap());
        assert!(store.exists(&stored[2]).await.unwrap());
        let reset: i64 = stored_high_water(&store, &cfg)
            .await
            .unwrap()
            .parse()
            .unwrap();
        assert!((now..now + 60).contains(&reset), "{reset}");
        let loaded = Notifier::load(&store, &cfg).await.unwrap();
        assert!(loaded.is_live("open"));
        assert!(!loaded.halted());
        assert_eq!(loaded.high_water(), reset);
    }

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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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

    /// Review finding: a removal of an expired subscription holds no listing's lock; with the
    /// clock stepped back while it waited on the store, the subscription was listed again, and
    /// its removal then changed a listing a conditional create held. Once seen expired, it
    /// stays expired.
    #[tokio::test]
    async fn an_expiry_is_never_undone() {
        let (state, _store) = test_store::state(4).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let reader = Agent {
            subject: Some("https://reader.example/#me".into()),
            ..Agent::anonymous()
        };
        let post = test_store::request(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body,
        );
        let r = handle(&state, &post, &reader).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let me = &state.notify;
        let id = me.subs.read().unwrap().keys().next().cloned().unwrap();
        let set_expiry = |at: i64| {
            me.subs.write().unwrap().get_mut(&id).unwrap().expires_at = Some(at);
        };
        set_expiry(1);
        assert!(!me.is_live(&id));
        // The clock steps back past its expiry: as if it had not come yet.
        set_expiry(jose::now_secs() + 3600);
        assert!(!me.is_live(&id), "listed again");
        assert!(me
            .live_for(reader.subject.as_deref(), false, me.clock())
            .is_empty());
        assert!(me.live_for(None, true, me.clock()).is_empty());
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
                        kind: SubscriptionKind::Webhook,
                        capability_hash: None,
                        lapsed: Lapsed::default(),
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
        assert!(state.notify.get("s1", state.notify.clock()).is_some());
        // A read is held to If-Match as well.
        let other = test_store::request(Method::GET, &path, &[("if-match", "\"other\"")], "");
        let r = handle(&state, &other, &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        let r = handle(&state, &delete(&tag), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(state.notify.get("s1", state.notify.clock()).is_none());
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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

    /// Review of #7135: a live subscription whose removal was under way still heard of
    /// changes, and still had deliveries made to it, until the store confirmed it gone. Its
    /// removal takes it out of force at once; a removal that leaves it stored puts it back.
    #[tokio::test]
    async fn a_subscription_being_removed_hears_nothing_until_it_stays() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let state = LwsState::new(test_store::FlakyStore::new(), cfg)
            .await
            .unwrap();
        let path = subscribe_root(&state, "s").await;
        let iri = state.cfg.absolute(&path);
        let event = Event {
            kind: "Update",
            uri: state.cfg.storage(),
            is_container: true,
            relation: None,
        };
        let heard = || {
            let (state, event) = (state.clone(), event.clone());
            async move {
                let prepared = state.notify.prepare(&state, &event).await;
                let stands = state.notify.stands_now(&state.store, "s").await;
                (prepared.len(), stands)
            }
        };
        assert_eq!(heard().await, (1, true));
        // The removal's store step is held: the subscription is out of force meanwhile.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.store.hold_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        let deleting = {
            let state = state.clone();
            let delete = test_store::request(Method::DELETE, &path, &[], "");
            tokio::spawn(async move { handle(&state, &delete, &Agent::anonymous()).await.status() })
        };
        while state.store.hold_next_delete_of.lock().unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(heard().await, (0, false), "heard while its removal ran");
        gate.add_permits(1);
        assert_eq!(deleting.await.unwrap(), StatusCode::NO_CONTENT);
        assert_eq!(heard().await, (0, false));
        // A removal that fails, the subscription still stored, puts it back in force.
        let path = subscribe_root(&state, "s").await;
        *state.store.fail_delete_of.lock().unwrap() = Some(iri.clone());
        let delete = test_store::request(Method::DELETE, &path, &[], "");
        let status = handle(&state, &delete, &Agent::anonymous()).await.status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(heard().await, (1, true), "still stored, so back in force");
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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
        assert!(state.notify.get("s1", state.notify.clock()).is_none());
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
        assert!(state.notify.get("s1", state.notify.clock()).is_some());
        let iri = state.cfg.absolute(&path);
        assert!(state.store.exists(&iri).await.unwrap());
        store.fail_delete.store(false, Ordering::SeqCst);
        let resp = handle(&state, &delete, &Agent::anonymous()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.notify.get("s1", state.notify.clock()).is_none());
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
                kind: SubscriptionKind::Webhook,
                capability_hash: None,
                lapsed: Lapsed::default(),
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
        // Nor the owner's, which holds the whole listing alone.
        let owner = agent(test_store::OWNER);
        let started = tokio::time::Instant::now();
        let (status, _) = post(&owner, vec![("if-match", tag(&owner).await)]).await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the owner's held up by a sweep"
        );
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "the owner's waited on it"
        );
        // Nor Bob's own, though the subscription swept is his.
        let (status, _) = post(&bob, vec![("if-match", tag(&bob).await)]).await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "Bob's held up by a sweep of his own"
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

    /// A conditional create whose outcome the store leaves unknown keeps its subscriber's
    /// listing only shared while it is settled: the subscriber's other creates and removals go on
    /// meanwhile, and only another conditional create of its own waits for the outcome.
    #[tokio::test(start_paused = true)]
    async fn an_unsettled_conditional_create_keeps_its_listing_shared() {
        use test_store::{agent, closed_state, data, grant_read};
        let (state, store) = closed_state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let topic = data(&state, "doc").await;
        let alice = agent("https://alice.example/#me");
        grant_read(&state, alice.subject.as_deref().unwrap(), &[&topic]).await;
        let body = json!({
            "type": WEBHOOK,
            "topic": [topic],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |headers: Vec<(&'static str, String)>| {
            let (state, body, alice) = (state.clone(), body.clone(), alice.clone());
            async move {
                let mut h = vec![("content-type", LWS_JSON.to_string())];
                h.extend(headers);
                let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let req = test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body);
                let r = handle(&state, &req, &alice).await;
                (
                    r.status(),
                    r.headers()
                        .get(header::LOCATION)
                        .map(|l| l.to_str().unwrap().to_string()),
                )
            }
        };
        let (status, first) = post(vec![]).await;
        assert_eq!(status, StatusCode::CREATED);
        let first = first.unwrap();
        let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
        let etag = test_store::route_as(&state, req, &alice).await.headers()["etag"]
            .to_str()
            .unwrap()
            .to_string();
        // Its write never settles: the fence that would settle it never answers.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
        store.hang_fence.store(true, Ordering::SeqCst);
        let (status, _) = post(vec![("if-match", etag.clone())]).await;
        assert!(status.is_server_error(), "{status}");
        assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
        // Its settlement is under way, holding Alice's listing shared.
        let (status, _) = post(vec![]).await;
        assert_eq!(status, StatusCode::CREATED, "a plain create waited on it");
        let path = first.strip_prefix(&state.cfg.base_url).unwrap().to_string();
        let req = test_store::request(Method::DELETE, &path, &[], "");
        assert_eq!(
            handle(&state, &req, &alice).await.status(),
            StatusCode::NO_CONTENT,
            "a removal waited on it"
        );
        // Another conditional create of hers still waits for the outcome.
        let (status, _) = post(vec![("if-match", etag)]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        drop(gate);
    }

    /// Review finding: the owner's conditional create, evaluated on the whole listing, dropped
    /// the container's lock once the store left its outcome unknown, so a second one with the
    /// same entity tag passed and both could register. The settlement keeps that lock shared:
    /// the owner's plain creates go on, and another conditional create waits for the outcome.
    #[tokio::test(start_paused = true)]
    async fn an_unsettled_owner_create_keeps_the_whole_listing_shared() {
        use test_store::{agent, closed_state, OWNER};
        let (state, store) = closed_state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let owner = agent(OWNER);
        let body = json!({
            "type": WEBHOOK,
            "topic": [state.cfg.storage()],
            "inbox": "https://inbox.example/",
        })
        .to_string();
        let post = |headers: Vec<(&'static str, String)>| {
            let (state, body, owner) = (state.clone(), body.clone(), owner.clone());
            async move {
                let mut h = vec![("content-type", LWS_JSON.to_string())];
                h.extend(headers);
                let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let req = test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body);
                handle(&state, &req, &owner).await.status()
            }
        };
        assert_eq!(post(vec![]).await, StatusCode::CREATED);
        let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
        let etag = test_store::route_as(&state, req, &owner).await.headers()["etag"]
            .to_str()
            .unwrap()
            .to_string();
        // Its write never settles: the fence that would settle it never answers.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
        store.hang_fence.store(true, Ordering::SeqCst);
        let status = post(vec![("if-match", etag.clone())]).await;
        assert!(status.is_server_error(), "{status}");
        assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            post(vec![]).await,
            StatusCode::CREATED,
            "a plain create waited"
        );
        // Another conditional create on the same listing waits for the outcome.
        assert_eq!(
            post(vec![("if-match", etag)]).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        drop(gate);
    }

    /// Once the owner's unsettled conditional create is settled, stored or not, the listing's
    /// lock is released: a later conditional create is decided on the listing as it then is,
    /// no longer told to wait.
    #[tokio::test(start_paused = true)]
    async fn a_settled_owner_create_releases_the_whole_listing() {
        use test_store::{agent, closed_state, OWNER};
        for stored in [false, true] {
            let (state, store) = closed_state(100).await;
            state.notify.sweeper_off.store(true, Ordering::SeqCst);
            let owner = agent(OWNER);
            let body = json!({
                "type": WEBHOOK,
                "topic": [state.cfg.storage()],
                "inbox": "https://inbox.example/",
            })
            .to_string();
            let post = |headers: Vec<(&'static str, String)>| {
                let (state, body, owner) = (state.clone(), body.clone(), owner.clone());
                async move {
                    let mut h = vec![("content-type", LWS_JSON.to_string())];
                    h.extend(headers);
                    let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    let req = test_store::request(Method::POST, SUBSCRIPTIONS_PATH, &h, &body);
                    handle(&state, &req, &owner).await.status()
                }
            };
            let req = test_store::request(Method::GET, SUBSCRIPTIONS_PATH, &[], "");
            let etag = test_store::route_as(&state, req, &owner).await.headers()["etag"]
                .to_str()
                .unwrap()
                .to_string();
            // Its write is left unsettled, and the fence that would settle it fails for now.
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *store.unsettle_next_create.lock().unwrap() = Some(gate.clone());
            store.fail_fence.store(true, Ordering::SeqCst);
            let status = post(vec![("if-match", etag.clone())]).await;
            assert!(status.is_server_error(), "{status}");
            assert_eq!(
                post(vec![("if-match", etag.clone())]).await,
                StatusCode::SERVICE_UNAVAILABLE
            );
            if stored {
                gate.add_permits(1);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            store.fail_fence.store(false, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(60)).await;
            let expected = match stored {
                true => StatusCode::PRECONDITION_FAILED,
                false => StatusCode::CREATED,
            };
            assert_eq!(post(vec![("if-match", etag)]).await, expected, "stored: {stored}");
        }
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
                        kind: SubscriptionKind::Webhook,
                        capability_hash: None,
                        lapsed: Lapsed::default(),
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

    /// Two sweeps in a row, the second before any removal of the first has run, start no more
    /// removals than there are places: none is started only to find no place, and so none is
    /// held back as if it had failed.
    #[tokio::test(start_paused = true)]
    async fn back_to_back_sweeps_start_no_more_removals_than_places() {
        use super::super::records::MAINTENANCE_PLACES;
        let (state, _store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        expired_subscriptions(&state, 3 * MAINTENANCE_PLACES);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        sweeper.sweep(&state);
        assert_eq!(sweeper.removals.len(), MAINTENANCE_PLACES);
        sweeper.settled().await;
        assert!(sweeper.backoff.is_empty(), "held back untried");
        assert_eq!(
            state.notify.subs.read().unwrap().len(),
            2 * MAINTENANCE_PLACES
        );
    }

    /// A hold whose release failed stays only until the sweeper releases it again.
    #[tokio::test]
    async fn a_hold_whose_release_failed_is_released_by_the_sweeper() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let d = format!("{}doc", state.cfg.storage());
        store
            .write(&d, Bytes::from("x"), "text/plain")
            .await
            .unwrap();
        store.fail_release.store(true, Ordering::SeqCst);
        let mut journal = state.journal();
        journal.stage(&d).await.unwrap();
        assert!(journal.commit(None).await.is_ok());
        assert_eq!(state.store.holds().await.unwrap().len(), 1);
        let mut sweeper = Sweeper::default();
        // Failing still: kept for the next sweep.
        sweeper.sweep(&state);
        while sweeper.releases.join_next().await.is_some() {}
        assert_eq!(state.store.holds().await.unwrap().len(), 1);
        store.fail_release.store(false, Ordering::SeqCst);
        sweeper.sweep(&state);
        while sweeper.releases.join_next().await.is_some() {}
        assert!(state.store.holds().await.unwrap().is_empty());
    }

    /// A removal that fails is tried again only once its wait is over, and waits twice as long
    /// each time it fails again, up to [`SWEEP_BACKOFF_MOST`].
    #[tokio::test(start_paused = true)]
    async fn a_failing_removal_waits_longer_each_time() {
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let ids = expired_subscriptions(&state, 1);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        store
            .create_in_container(
                &container,
                &format!("{container}{}", ids[0]),
                Bytes::from("{}"),
                LWS_JSON,
            )
            .await
            .unwrap();
        store.fail_delete.store(true, Ordering::SeqCst);
        let mut sweeper = Sweeper::default();
        let mut tries = 0;
        for wait in [
            SWEEP_BACKOFF_FIRST,
            SWEEP_BACKOFF_FIRST * 2,
            SWEEP_BACKOFF_FIRST * 4,
        ] {
            sweeper.sweep(&state);
            assert_eq!(
                sweeper.removals.len(),
                1,
                "not tried once its wait was over"
            );
            sweeper.settled().await;
            tries += 1;
            let (at, failed) = sweeper.backoff[&ids[0]];
            assert_eq!(failed, tries);
            assert_eq!(at - tokio::time::Instant::now(), wait);
            // Not tried again while it waits.
            tokio::time::sleep(wait - Duration::from_millis(1)).await;
            sweeper.sweep(&state);
            assert!(sweeper.removals.is_empty(), "tried while it waits");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // The wait stops growing at its cap.
        sweeper.backoff.get_mut(&ids[0]).unwrap().1 = 40;
        sweeper.sweep(&state);
        sweeper.settled().await;
        let (at, _) = sweeper.backoff[&ids[0]];
        assert_eq!(at - tokio::time::Instant::now(), SWEEP_BACKOFF_MOST);
        // Once the store lets it, it goes, and its wait with it.
        store.fail_delete.store(false, Ordering::SeqCst);
        tokio::time::sleep(SWEEP_BACKOFF_MOST).await;
        sweeper.sweep(&state);
        sweeper.settled().await;
        assert!(state.notify.subs.read().unwrap().is_empty());
        assert!(sweeper.backoff.is_empty());
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
        // At once: not when the removal's deadline is past, nor at the sweeper's next tick.
        run_ready().await;
        assert!(weak.upgrade().is_none(), "the state outlived its handles");
        drop(gate);
    }

    /// One of the sweeper's removals whose delete the store leaves unknown, and whose settling
    /// then never completes, is settled within the removal's own deadline: given up, it gives
    /// back its place, and its subscription is tried again later.
    #[tokio::test(start_paused = true)]
    async fn an_unsettled_sweep_gives_back_its_place_at_its_deadline() {
        use super::super::records::MAINTENANCE_PLACES;
        let (state, store) = test_store::state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let ids = expired_subscriptions(&state, 1);
        let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
        // Stored too, so the delete's outcome is unknown while it stays listed.
        store
            .create_in_container(
                &container,
                &format!("{container}{}", ids[0]),
                Bytes::from("{}"),
                LWS_JSON,
            )
            .await
            .unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.unsettle_next_delete_of.lock().unwrap() =
            Some((format!("{container}{}", ids[0]), gate.clone()));
        store.hang_fence.store(true, Ordering::SeqCst);
        let mut sweeper = Sweeper::default();
        sweeper.sweep(&state);
        run_ready().await;
        assert!(
            store.unsettle_next_delete_of.lock().unwrap().is_none(),
            "the removal never reached the store"
        );
        assert_eq!(state.settlements.maintenance_free(), MAINTENANCE_PLACES - 1);
        tokio::time::sleep(SWEEP_REMOVAL_DEADLINE).await;
        run_ready().await;
        sweeper.sweep(&state);
        assert_eq!(
            state.settlements.maintenance_free(),
            MAINTENANCE_PLACES,
            "its place was kept past its deadline"
        );
        assert!(state.notify.subs.read().unwrap().contains_key(&ids[0]));
        assert!(
            sweeper.backoff.contains_key(&ids[0]),
            "not to be tried again"
        );
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

    /// A subscriber that may not read a topic causes the same store work, and gets the same
    /// answer, whether the topic exists or not: its access is decided before anything is asked
    /// of whether the topic is there, both when the topic's lock is free and when a change to it
    /// holds it (one that kept nothing of what it was, or, as a change does, an update of a topic
    /// that is there, kept as it was, against a create of one that is not).
    #[tokio::test]
    async fn a_topic_the_subscriber_may_not_read_is_decided_the_same_whether_it_exists() {
        use test_store::{agent, closed_state, data};
        let (state, store) = closed_state(100).await;
        state.notify.sweeper_off.store(true, Ordering::SeqCst);
        let there = data(&state, "there").await;
        let gone = format!("{}gone", state.cfg.storage());
        assert!(!state.store.exists(&gone).await.unwrap());
        let stranger = agent("https://stranger.example/#me");
        let as_stored = super::super::Prior {
            existed: Some(true),
            meta: Some(state.resource_meta(&there).await.unwrap()),
            format: Some(Some("text/plain".to_string())),
        };
        let creating = super::super::Prior {
            existed: Some(false),
            ..super::super::Prior::default()
        };
        let attempt = |topic: String, changing: Option<super::super::Prior>| {
            let (state, store, who) = (state.clone(), store.clone(), stranger.clone());
            async move {
                let body = json!({
                    "type": WEBHOOK,
                    "topic": [topic.clone()],
                    "inbox": "https://inbox.example/",
                })
                .to_string();
                let req = test_store::request(
                    Method::POST,
                    SUBSCRIPTIONS_PATH,
                    &[("content-type", LWS_JSON)],
                    &body,
                );
                // A change in flight to it: its lock held, and what it was before not kept, so
                // that is read as stored.
                let held = match changing.clone() {
                    Some(before) => {
                        let held = state.locks.lock(&topic).await.unwrap();
                        state
                            .locks
                            .changing_from([(topic.clone(), before)].into_iter().collect());
                        Some(held)
                    }
                    None => None,
                };
                *store.trace.lock().unwrap() = Some(Vec::new());
                let r = test_store::route_as(&state, req, &who).await;
                let trace = store.trace.lock().unwrap().take().unwrap();
                if changing.is_some() {
                    state.locks.landed(std::slice::from_ref(&topic));
                }
                drop(held);
                let status = r.status();
                let body = axum::body::to_bytes(r.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let trace: Vec<String> = trace
                    .into_iter()
                    .map(|t| t.replace(&topic, "{topic}"))
                    .collect();
                (status, body, trace)
            }
        };
        for changing in [None, Some(super::super::Prior::default())] {
            let a = attempt(there.clone(), changing.clone()).await;
            let b = attempt(gone.clone(), changing.clone()).await;
            assert_eq!(a.0, StatusCode::FORBIDDEN);
            assert_eq!(a, b, "changing: {changing:?}");
        }
        // An update in flight on the topic that is there, against a create in flight of the
        // one that is not.
        let updating = attempt(there.clone(), Some(as_stored.clone())).await;
        let created = attempt(gone.clone(), Some(creating.clone())).await;
        assert_eq!(updating.0, StatusCode::FORBIDDEN);
        assert_eq!(updating, created);
        // Review of #7174: a create that failed and was set aside keeps the default metadata
        // it wrote over nothing (and no format). It is still a URI where nothing was: with a
        // grant to the subscriber whose format constraint the topic does not meet, nothing is
        // read of it either (its format is not looked up), as for the update in flight.
        state.access.put_grant_for_test(
            "format",
            access::policies(
                &json!([{"type": ["AccessPolicy"], "action": ["read"],
                    "assignee": "https://stranger.example/#me",
                    "constraint": [{"leftOperand": "format", "operator": "isAnyOf",
                        "rightOperand": ["text/turtle"]}]}]),
                &state.cfg.storage(),
            )
            .unwrap(),
        );
        let failed_create = super::super::Prior {
            existed: Some(false),
            meta: Some(super::super::ResourceMeta::default()),
            format: None,
        };
        let updating = attempt(there.clone(), Some(as_stored)).await;
        let created = attempt(gone.clone(), Some(creating)).await;
        let failed = attempt(gone.clone(), Some(failed_create)).await;
        assert_eq!(updating.0, StatusCode::FORBIDDEN);
        assert_eq!(updating, created);
        assert_eq!(created, failed);
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
            kind: SubscriptionKind::Webhook,
            capability_hash: None,
            lapsed: Lapsed::default(),
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

// AUTHORED-BY Claude Opus 4.8
//! Solid Notifications (WebSocketChannel2023) — an in-process subscription registry + AS2.0 emit.
//!
//! This module is **net-new and isolated**: it does NOT modify the storage layer. The LDP write path
//! makes a SINGLE [`NotificationHub::notify`] call after a successful mutation (an emit hook), and the
//! discovery + subscription endpoints ([`ws`]) advertise + serve the WebSocketChannel2023 channel.
//!
//! ## What this implements (WebSocketChannel2023)
//! - **Discovery** — a storage-description document + `Link` headers advertise the subscription
//!   service URL ([`ws::storage_description_handler`], [`ws::link_headers`]).
//! - **Subscribe** — a client `POST`s a JSON-LD channel request naming a `topic`; the server returns
//!   a channel description whose `receiveFrom` is a `ws(s)://` URL ([`ws::subscribe_handler`]).
//! - **Receive** — connecting to `receiveFrom` upgrades to a WebSocket and registers the connection
//!   under the topic IRI ([`ws::receive_handler`]); the server pushes an AS2.0 notification whenever
//!   the topic (or its container membership) changes.
//!
//! ## Concurrency model (the make-the-call decision, documented)
//! The registry is a `Mutex<HashMap<topic_iri, broadcast::Sender<Arc<str>>>>`:
//! - A `tokio::sync::broadcast` channel per watched topic IRI gives clean 1-N fan-out: an emit is a
//!   single `send` and every live receiver gets it. This is the idiomatic axum chat-broadcast pattern.
//! - The notification body is built + serialised ONCE per emit (an `Arc<str>`) and the SAME `Arc` is
//!   broadcast to all subscribers — no per-subscriber re-serialisation.
//! - A `broadcast::Sender` with NO receivers is pruned lazily on the next emit (a `send` returns
//!   `Err(SendError)` when there are zero receivers) so a torn-down subscription cannot leak the map
//!   entry. A receiver that lags past the buffer drops the oldest frame (`RecvError::Lagged`) — the
//!   client must reconcile on reconnect (the documented missed-update safety contract), which is the
//!   correct trade-off for a notification stream vs. unbounded memory growth.
//! - The `Mutex` is held only for the O(1) map get/insert/prune; the actual fan-out (`send`) is a
//!   lock-free broadcast push, so emit does not serialise behind subscriber I/O.
//!
//! ## Auth (fail-closed; authenticated + WAC-gated subscribe, token + WAC-gated receive)
//! Subscription is gated on an AUTHENTICATED WebID (no anonymous subscriptions) — the subscribe POST
//! runs behind the same DPoP auth middleware as the LDP routes — AND on a per-resource Web Access
//! Control decision: the subscriber must hold `acl:Read` on the topic (`acl:Control` when the topic
//! is an `.acl`), evaluated by the same [`WacAuthorizer`](crate::authz::WacAuthorizer) the LDP read
//! path uses. A change notification discloses that (and when) the resource changed, so subscribing
//! IS a read of it. Only after that decision does the handler MINT an unguessable, short-lived
//! **receive token** bound to `(authenticated WebID, topic, expiry)` and embed it in the
//! `receiveFrom` URL.
//!
//! The WS receive endpoint REQUIRES a valid token (unexpired + its bound topic must match the
//! requested topic) — closing the open-receive bypass even though a browser `WebSocket` cannot carry
//! the DPoP `Authorization` header (the token in the URL is the spec's mechanism for exactly this) —
//! and then RE-RUNS the same WAC check for the WebID the token is bound to
//! ([`NotificationHub::authorize_receive_token`]). Re-checking at connect time keeps the two gates in
//! lock-step and means a grant REVOKED after subscribe cannot be replayed for the remainder of the
//! token's TTL. The token gates connecting only, with at most one socket open per token at a time:
//! an open socket lives until the client closes it, its subscriber loses read access (1008), or it
//! is closed 1011 (its backlog overflows, its access cannot be checked on the timer, or a frame was
//! dropped and the client is to reconnect and reconcile). The same check runs again before every
//! frame on an open socket (a frame the subscriber may not read is dropped), and on a timer of the
//! socket's own ([`ws::ACCESS_RECHECK`]), which closes it (1008) once the subscriber has lost read
//! access. Within two timer periods of losing Read it closes: 1008 for a confirmed denial, or
//! 1011 if the timer check is undecided. Each check has at most one period; a slower check
//! counts as undecided and the client must resubscribe. A due tick pre-empts an Event or Lag
//! check, noting it as missed: a reader then gets a reconcile close (also for a dropped Lag
//! check, rather than an overflow close). Inbound frames cannot starve the timer or its decision.
//!
//! **Existence non-disclosure** (`research/lws-design-records.md` §6): neither gate probes the topic
//! resource itself — the decision comes ONLY from the ACL-resolution chain — so a denial is the same
//! `403` whether the topic exists or not, and an authorized subscription to a not-yet-created
//! resource still succeeds (that is how a client watches for its `Create`).

pub mod activity;
pub mod ws;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, Mutex};

pub use activity::{ActivityType, AS2_CONTEXT, NOTIFICATIONS_CONTEXT};

/// How long a minted receive token stays valid after the authenticated subscribe (make-the-call
/// decision, documented). 5 minutes: long enough to bridge subscribe-POST → WS-connect (which a
/// client does immediately) AND to absorb a transient reconnect, short enough that a
/// `receiveFrom` URL in someone else's hands stops connecting quickly. The token is REUSABLE until
/// expiry (NOT single-use at connect) so a client that drops the socket can reconnect with the
/// same `receiveFrom` URL within the window — the standard WebSocketChannel2023 reconnect
/// behaviour — but it has at most ONE socket open at a time ([`NotificationHub::claim_receive_socket`]):
/// a connect while a socket opened with it is still open is refused as an invalid token is (401),
/// and a reconnect is accepted once that socket has closed.
/// It is a window for CONNECTING: a socket opened within it is not closed when it ends (an open
/// socket follows its subscriber's access instead, see [`ws::ACCESS_RECHECK`]), so expiry bounds
/// when a socket can be opened, not how long one stays open: whoever holds the URL within its
/// window can hold one socket open for as long as the subscriber may read the topic.
const RECEIVE_TOKEN_TTL: Duration = Duration::from_secs(300);

/// The number of random bytes in a receive token before base64url encoding. 32 bytes = 256 bits of
/// entropy from a cryptographically-secure RNG ⇒ unguessable.
const RECEIVE_TOKEN_BYTES: usize = 32;

/// A receive token's binding: which topic it authorizes RECEIVE on, which authenticated WebID minted
/// it (subscribed), and when it expires. The token STRING itself is the map key (never stored in the
/// value, never logged).
#[derive(Clone, Debug)]
struct ReceiveTokenBinding {
    /// The topic IRI this token authorizes RECEIVE on. The requested `?topic=` MUST equal this.
    topic: String,
    /// The authenticated WebID that subscribed (minted the token). Read back by
    /// [`NotificationHub::authorize_receive_token`] so the receive endpoint can re-run the
    /// per-resource WAC check against the SUBSCRIBER's identity, not merely against the token.
    web_id: String,
    /// When the token stops connecting.
    expiry: TokenExpiry,
    /// Whether a socket opened with this token is open: at most one is at a time (see
    /// [`NotificationHub::claim_receive_socket`]).
    open: Arc<std::sync::atomic::AtomicBool>,
}

/// The one socket a receive token may have open, held by that socket's task: released (so the
/// token can connect again, within its window) when the task ends, however it ends.
pub(crate) struct SocketClaim(Arc<std::sync::atomic::AtomicBool>);

impl Drop for SocketClaim {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// When a receive token stops connecting: its expiry instant, and a latch set once it is seen
/// expired. Once set it stays set, so a token that has expired never connects again, whatever
/// its binding says later (a binding whose instant is moved later, say). Held in memory, on the
/// process's monotonic clock: no token outlives the process. It gates connecting only; an open
/// socket is not closed by it.
#[derive(Clone, Debug)]
struct TokenExpiry {
    at: Instant,
    lapsed: Arc<std::sync::atomic::AtomicBool>,
}

impl TokenExpiry {
    fn new(at: Instant) -> Self {
        Self {
            at,
            lapsed: Default::default(),
        }
    }

    /// Whether the token has expired, by its instant or by having been seen expired before;
    /// seen expired now, it is latched so.
    fn expired(&self) -> bool {
        use std::sync::atomic::Ordering;
        if self.lapsed.load(Ordering::Acquire) {
            return true;
        }
        let expired = self.at <= Instant::now();
        if expired {
            self.lapsed.store(true, Ordering::Release);
        }
        expired
    }
}

/// How many notifications a per-topic broadcast channel buffers before a slow receiver starts
/// dropping the oldest (`RecvError::Lagged`). Small on purpose: notifications are change SIGNALS, not
/// a durable log — a client that falls this far behind should reconcile by re-reading, not replay a
/// long backlog (the missed-update-safety contract in the skill). 64 absorbs a normal burst without
/// holding unbounded memory per topic.
const TOPIC_BUFFER: usize = 64;

/// The in-process subscription registry + AS2.0 notification emitter.
///
/// Cloning a [`NotificationHub`] shares the SAME underlying registry (it is an `Arc` inside), so the
/// LDP state and the notification routes both hold a handle to one hub.
#[derive(Clone, Default)]
pub struct NotificationHub {
    /// topic IRI -> the broadcast sender that fans a notification to that topic's subscribers.
    topics: Arc<Mutex<HashMap<String, broadcast::Sender<Arc<str>>>>>,
    /// receive token -> its `(topic, web_id, expiry)` binding. Minted by an authenticated subscribe,
    /// required (valid + topic-matching + unexpired) by the WS receive endpoint. The token string is
    /// the key; it is never placed in a value field and never logged.
    receive_tokens: Arc<Mutex<HashMap<String, ReceiveTokenBinding>>>,
}

impl NotificationHub {
    /// A fresh, empty hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to a topic IRI: returns a [`broadcast::Receiver`] that yields every notification
    /// emitted for `topic` from this point on. Creates the topic's channel on first subscription.
    pub async fn subscribe(&self, topic: &str) -> broadcast::Receiver<Arc<str>> {
        let mut topics = self.topics.lock().await;
        match topics.get(topic) {
            Some(tx) => tx.subscribe(),
            None => {
                let (tx, rx) = broadcast::channel(TOPIC_BUFFER);
                topics.insert(topic.to_string(), tx);
                rx
            }
        }
    }

    /// Mint a fresh receive token for `(web_id, topic)`, valid for `RECEIVE_TOKEN_TTL`. Called by
    /// the authenticated subscribe handler; the token is embedded in the returned `receiveFrom` URL.
    ///
    /// The token is 256 bits from a cryptographically-secure RNG (the in-tree rustls/aws-lc-rs
    /// `SecureRandom`, NO new crate), base64url-encoded so it is URL-safe ⇒ unguessable. We
    /// opportunistically prune expired tokens while we hold the lock so the map cannot grow without
    /// bound from never-connected subscriptions.
    pub async fn mint_receive_token(&self, web_id: &str, topic: &str) -> String {
        let token = generate_receive_token();
        let binding = ReceiveTokenBinding {
            topic: topic.to_string(),
            web_id: web_id.to_string(),
            expiry: TokenExpiry::new(Instant::now() + RECEIVE_TOKEN_TTL),
            open: Default::default(),
        };
        let mut tokens = self.receive_tokens.lock().await;
        // Opportunistic sweep: drop any expired token so a stream of subscribes-without-connect
        // cannot leak the map unboundedly.
        tokens.retain(|_, b| !b.expiry.expired());
        tokens.insert(token.clone(), binding);
        token
    }

    /// [`mint_receive_token`](Self::mint_receive_token), for a token that expires `ttl` from
    /// now.
    #[cfg(test)]
    pub(crate) async fn mint_receive_token_for(
        &self,
        web_id: &str,
        topic: &str,
        ttl: Duration,
    ) -> String {
        let token = generate_receive_token();
        let binding = ReceiveTokenBinding {
            topic: topic.to_string(),
            web_id: web_id.to_string(),
            expiry: TokenExpiry::new(Instant::now() + ttl),
            open: Default::default(),
        };
        self.receive_tokens
            .lock()
            .await
            .insert(token.clone(), binding);
        token
    }

    /// Validate a receive token presented on the WS upgrade against the requested `topic`. Returns
    /// `true` ONLY when the token exists, is unexpired, AND its bound topic equals `topic`. An
    /// absent / unknown / expired / topic-mismatched token returns `false` (fail-closed). An
    /// expired entry is dropped by the next mint's sweep.
    ///
    /// The token is REUSABLE until expiry (it is NOT consumed here) so a client can reconnect within
    /// the TTL window — see `RECEIVE_TOKEN_TTL`. Never logs the token.
    pub async fn validate_receive_token(&self, token: &str, topic: &str) -> bool {
        self.authorize_receive_token(token, topic).await.is_some()
    }

    /// The identity half of the token gate: validate exactly as
    /// [`validate_receive_token`](Self::validate_receive_token) does AND return the authenticated
    /// WebID the token was minted for. `None` on any failure (absent / unknown / expired /
    /// topic-mismatched) — fail-closed; an expired entry stays refused (its latch is set) until
    /// the next mint's sweep drops it.
    ///
    /// The receive endpoint needs the SUBSCRIBER's WebID, not just a yes/no, so it can re-run the
    /// per-resource WAC check at connect time (see [`ws::receive_handler`]): the token proves *who*
    /// subscribed, and WAC then decides whether that WebID may STILL read the topic. Never logs the
    /// token.
    pub async fn authorize_receive_token(&self, token: &str, topic: &str) -> Option<String> {
        let tokens = self.receive_tokens.lock().await;
        match tokens.get(token) {
            // Expired (latched, so it is refused from now on): not pruned here, so the binding
            // keeps its latch until the next mint's sweep drops it.
            Some(b) if b.expiry.expired() => None,
            Some(b) if b.topic == topic => Some(b.web_id.clone()),
            // Present but bound to a DIFFERENT topic, or absent entirely.
            _ => None,
        }
    }

    /// Claim the one socket `token` may have open for `topic`: `None` when the token does not
    /// validate (as for [`authorize_receive_token`](Self::authorize_receive_token)) or a socket
    /// opened with it is still open. The claim is released when it is dropped (the socket's task
    /// has ended), and the token may then connect again within its window.
    pub(crate) async fn claim_receive_socket(
        &self,
        token: &str,
        topic: &str,
    ) -> Option<SocketClaim> {
        use std::sync::atomic::Ordering;
        let tokens = self.receive_tokens.lock().await;
        let b = tokens.get(token)?;
        if b.expiry.expired() || b.topic != topic {
            return None;
        }
        b.open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(SocketClaim(b.open.clone()))
    }

    /// The number of LIVE subscribers for a topic (0 if the topic has no channel). Test/observability
    /// aid — also the basis of the lazy prune (a topic with 0 receivers is dropped on the next emit).
    pub async fn subscriber_count(&self, topic: &str) -> usize {
        let topics = self.topics.lock().await;
        topics.get(topic).map(|tx| tx.receiver_count()).unwrap_or(0)
    }

    /// Emit an AS2.0 notification for a change to `resource`.
    ///
    /// This is THE emit hook the LDP write path calls after a successful mutation. It fans the
    /// notification to:
    /// 1. subscribers of `resource` itself (the changed resource), and
    /// 2. subscribers of `parent` — the container — as a MEMBERSHIP activity (`Add` on create,
    ///    `Remove` on delete), so a client watching a container learns its membership changed.
    ///
    /// `activity` is the resource-level activity (Create/Update/Delete). The container membership
    /// activity is DERIVED: a `Create` ⇒ `Add` on the parent, a `Delete` ⇒ `Remove`; an `Update`
    /// does not change membership, so no parent notification is emitted for it. (Make-the-call
    /// mapping, documented in [`activity`].)
    pub async fn notify(&self, resource: &str, activity: ActivityType, parent: Option<&str>) {
        // 1. The resource's own subscribers: the resource-level activity, `object` = the resource.
        let resource_body: Arc<str> = Arc::from(activity::build_notification_string(
            activity, resource, None,
        ));
        self.fan_out(resource, resource_body).await;

        // 2. The parent container's subscribers: a membership activity (Add/Remove) iff membership
        //    actually changed. An Update edits content without changing the container's `ldp:contains`
        //    set, so it does NOT notify the parent (avoids spurious container churn).
        if let Some(container) = parent {
            let membership = match activity {
                ActivityType::Create => Some(ActivityType::Add),
                ActivityType::Delete => Some(ActivityType::Remove),
                // Update / (already-membership) Add / Remove: no derived parent membership change.
                ActivityType::Update | ActivityType::Add | ActivityType::Remove => None,
            };
            if let Some(member_activity) = membership {
                // AS2 Add/Remove: the container is the `target` collection, the child is the `object`.
                let body: Arc<str> = Arc::from(activity::build_notification_string(
                    member_activity,
                    resource,
                    Some(container),
                ));
                self.fan_out(container, body).await;
            }
        }
    }

    /// Send a pre-built notification frame to a topic's subscribers, pruning a dead (0-receiver)
    /// channel. Holds the lock only for the map lookup + the (lock-free) broadcast `send`.
    async fn fan_out(&self, topic: &str, body: Arc<str>) {
        let mut topics = self.topics.lock().await;
        if let Some(tx) = topics.get(topic) {
            // `send` returns Err only when there are zero receivers — the subscription was torn down
            // (all sockets closed) without the entry being removed. Prune it so the map cannot leak.
            if tx.send(body).is_err() {
                topics.remove(topic);
            }
        }
        // No entry ⇒ nobody is watching this topic; nothing to do (and nothing to allocate).
    }
}

/// Generate a cryptographically-secure receive token: [`RECEIVE_TOKEN_BYTES`] (256-bit) of randomness
/// from the in-tree rustls/aws-lc-rs `SecureRandom`, base64url-encoded (URL-safe, no padding) so it
/// drops straight into the `receiveFrom` query string. Uses NO new crate — `rustls` is already a
/// direct dependency and aws-lc-rs is its backend.
///
/// A `SecureRandom::fill` failure is treated as fatal for the token (we panic rather than return a
/// guessable/empty token) — a CSPRNG that cannot produce randomness is a broken-environment condition,
/// and returning a weak token would silently defeat the security boundary this token exists to create.
fn generate_receive_token() -> String {
    let mut buf = [0u8; RECEIVE_TOKEN_BYTES];
    rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut buf)
        .expect("CSPRNG (aws-lc-rs SecureRandom) must provide randomness for a receive token");
    base64url_nopad(&buf)
}

/// Encode bytes as base64url (RFC 4648 §5, URL-safe alphabet) with NO padding — URL-safe for the
/// query string. Implemented inline (a few lines) rather than adding/declaring a base64 crate to the
/// non-dev build (`base64` is only a dev-dependency); the alphabet is fixed and the input length is
/// fixed at mint time.
fn base64url_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_then_notify_delivers_to_that_topic() {
        let hub = NotificationHub::new();
        let mut rx = hub.subscribe("https://pod.example/a").await;
        assert_eq!(hub.subscriber_count("https://pod.example/a").await, 1);

        hub.notify("https://pod.example/a", ActivityType::Update, None)
            .await;

        let frame = rx.try_recv().expect("a notification was delivered");
        assert!(frame.contains("\"type\":\"Update\""), "{frame}");
        assert!(
            frame.contains("\"object\":\"https://pod.example/a\""),
            "{frame}"
        );
    }

    #[tokio::test]
    async fn notify_does_not_reach_unrelated_subscribers() {
        let hub = NotificationHub::new();
        let mut other = hub.subscribe("https://pod.example/other").await;

        hub.notify("https://pod.example/a", ActivityType::Update, None)
            .await;

        // The unrelated topic got NOTHING.
        assert!(
            other.try_recv().is_err(),
            "an unrelated subscriber must not receive the notification"
        );
    }

    #[tokio::test]
    async fn create_fans_to_resource_and_parent_membership() {
        let hub = NotificationHub::new();
        let mut resource_rx = hub.subscribe("https://pod.example/c/child").await;
        let mut container_rx = hub.subscribe("https://pod.example/c/").await;

        hub.notify(
            "https://pod.example/c/child",
            ActivityType::Create,
            Some("https://pod.example/c/"),
        )
        .await;

        // The resource subscriber sees the Create.
        let r = resource_rx.try_recv().expect("resource notified");
        assert!(r.contains("\"type\":\"Create\""), "{r}");

        // The container subscriber sees a membership Add naming the container as `target`.
        let c = container_rx.try_recv().expect("container notified");
        assert!(c.contains("\"type\":\"Add\""), "{c}");
        assert!(c.contains("\"target\":\"https://pod.example/c/\""), "{c}");
        assert!(
            c.contains("\"object\":\"https://pod.example/c/child\""),
            "{c}"
        );
    }

    #[tokio::test]
    async fn update_does_not_notify_parent_container() {
        let hub = NotificationHub::new();
        let mut container_rx = hub.subscribe("https://pod.example/c/").await;

        hub.notify(
            "https://pod.example/c/child",
            ActivityType::Update,
            Some("https://pod.example/c/"),
        )
        .await;

        // An UPDATE does not change membership ⇒ the container is NOT notified.
        assert!(
            container_rx.try_recv().is_err(),
            "an Update must not emit a parent membership notification"
        );
    }

    #[tokio::test]
    async fn delete_fans_membership_remove_to_parent() {
        let hub = NotificationHub::new();
        let mut container_rx = hub.subscribe("https://pod.example/c/").await;

        hub.notify(
            "https://pod.example/c/child",
            ActivityType::Delete,
            Some("https://pod.example/c/"),
        )
        .await;

        let c = container_rx.try_recv().expect("container notified");
        assert!(c.contains("\"type\":\"Remove\""), "{c}");
    }

    #[tokio::test]
    async fn dropped_subscriber_is_pruned_on_next_emit_no_leak() {
        let hub = NotificationHub::new();
        let rx = hub.subscribe("https://pod.example/a").await;
        assert_eq!(hub.subscriber_count("https://pod.example/a").await, 1);

        // Tear down the only subscriber (simulates a closed WebSocket dropping its Receiver).
        drop(rx);
        assert_eq!(hub.subscriber_count("https://pod.example/a").await, 0);

        // The next emit prunes the now-deadtopic entry — no map leak.
        hub.notify("https://pod.example/a", ActivityType::Update, None)
            .await;

        let topics = hub.topics.lock().await;
        assert!(
            !topics.contains_key("https://pod.example/a"),
            "a 0-receiver topic must be pruned on emit, not leaked"
        );
    }

    #[tokio::test]
    async fn notify_with_no_subscribers_is_a_noop() {
        let hub = NotificationHub::new();
        // No panic, no entry created for a topic nobody watches.
        hub.notify("https://pod.example/nobody", ActivityType::Update, None)
            .await;
        assert_eq!(hub.subscriber_count("https://pod.example/nobody").await, 0);
    }

    // --- receive-token gating (the HIGH-finding fix) --------------------------------------------

    #[tokio::test]
    async fn minted_receive_token_validates_for_its_bound_topic() {
        let hub = NotificationHub::new();
        let tok = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        // A token minted for topic `a` validates for `a`.
        assert!(
            hub.validate_receive_token(&tok, "https://pod.example/a")
                .await
        );
    }

    #[tokio::test]
    async fn receive_token_rejects_wrong_topic() {
        let hub = NotificationHub::new();
        let tok = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        // The SAME token must NOT validate for a DIFFERENT topic (topic-binding enforced).
        assert!(
            !hub.validate_receive_token(&tok, "https://pod.example/b")
                .await,
            "a token bound to topic a must not authorize topic b"
        );
    }

    #[tokio::test]
    async fn receive_token_rejects_unknown_token() {
        let hub = NotificationHub::new();
        // Mint one (so the map is non-empty) then present a different, never-minted token.
        let _ = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        assert!(
            !hub.validate_receive_token("not-a-real-token", "https://pod.example/a")
                .await,
            "an unknown token must be rejected"
        );
    }

    #[tokio::test]
    async fn receive_token_rejects_expired() {
        let hub = NotificationHub::new();
        let tok = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        // Force the binding to be already-expired (set its expiry into the past).
        {
            let mut tokens = hub.receive_tokens.lock().await;
            let b = tokens.get_mut(&tok).expect("token present");
            b.expiry.at = Instant::now() - Duration::from_secs(1);
        }
        assert!(
            !hub.validate_receive_token(&tok, "https://pod.example/a")
                .await,
            "an expired token must be rejected"
        );
        // Its instant moved later, it stays refused: it was seen expired.
        hub.receive_tokens
            .lock()
            .await
            .get_mut(&tok)
            .expect("token present")
            .expiry
            .at = Instant::now() + Duration::from_secs(60);
        assert!(
            !hub.validate_receive_token(&tok, "https://pod.example/a")
                .await,
            "a token seen expired stays expired"
        );
        // And the next mint's sweep drops it.
        let _ = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/b")
            .await;
        let tokens = hub.receive_tokens.lock().await;
        assert!(
            !tokens.contains_key(&tok),
            "an expired token must be pruned by the next mint"
        );
    }

    #[tokio::test]
    async fn receive_tokens_are_unguessable_and_distinct() {
        let hub = NotificationHub::new();
        let a = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        let b = hub
            .mint_receive_token("https://alice.example/#me", "https://pod.example/a")
            .await;
        // Two mints for the SAME (webid, topic) yield DIFFERENT tokens (randomness, not derivation).
        assert_ne!(a, b, "tokens must be random, not deterministic from inputs");
        // 256 bits base64url-nopad ⇒ ceil(32/3)*4 = 44 chars (43 significant + the trailing group).
        // Assert sufficient length as a coarse entropy guard (a 256-bit secret is >= 43 b64url chars).
        assert!(
            a.len() >= 43,
            "a 256-bit base64url token should be >= 43 chars, got {} ({a})",
            a.len()
        );
        // The token is URL-safe (no '+', '/', '=' that would need re-encoding in a query).
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token must be base64url-safe: {a}"
        );
    }

    #[test]
    fn base64url_nopad_matches_known_vectors() {
        // RFC 4648 base64url, no padding.
        assert_eq!(base64url_nopad(b""), "");
        assert_eq!(base64url_nopad(b"f"), "Zg");
        assert_eq!(base64url_nopad(b"fo"), "Zm8");
        assert_eq!(base64url_nopad(b"foo"), "Zm9v");
        assert_eq!(base64url_nopad(b"foob"), "Zm9vYg");
        assert_eq!(base64url_nopad(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_nopad(b"foobar"), "Zm9vYmFy");
        // URL-safe alphabet: bytes that map to index 62/63 use '-'/'_' not '+'/'/'.
        assert_eq!(base64url_nopad(&[0xfb, 0xff]), "-_8");
    }
}

#[cfg(test)]
mod receive_window_tests {
    use super::*;

    #[tokio::test]
    async fn receive_socket_claims_use_the_current_token_window() {
        let hub = NotificationHub::new();
        let topic = "https://pod.example/a";
        let token = hub
            .mint_receive_token("https://alice.example/#me", topic)
            .await;
        drop(
            hub.claim_receive_socket(&token, topic)
                .await
                .expect("initial claim"),
        );
        assert!(hub.claim_receive_socket(&token, topic).await.is_some());
        hub.receive_tokens
            .lock()
            .await
            .get_mut(&token)
            .expect("token present")
            .expiry
            .at = Instant::now() - Duration::from_secs(1);
        assert!(hub.claim_receive_socket(&token, topic).await.is_none());
        hub.receive_tokens
            .lock()
            .await
            .get_mut(&token)
            .expect("token present")
            .expiry
            .at = Instant::now() + Duration::from_secs(60);
        assert!(hub.claim_receive_socket(&token, topic).await.is_none());
    }
}

//! [SONNET-4.6] sq-cmjmr — the MCP **resources subscription + notification** machinery
//! behind the pod-backed server (feature `solid`), bound to Solid Notifications
//! Protocol semantics (MCP-Solid proposal draft §8 / §10, Class N).
//!
//! # What lives here
//!
//! - [`Subscriptions`] — the per-server registry of subscribed topic IRIs plus the
//!   queue of outbound JSON-RPC notification messages the embedder drains with
//!   [`crate::SolidMcpServer::take_notifications`].
//! - [`TopicDigest`] — an order-independent content digest of ONE pod document,
//!   snapshotted before and after every mutating tool call so a state change can be
//!   detected without threading change-reporting through each tool.
//! - [`ActivityType`] + [`classify`] — the ActivityStreams 2.0 verb a change maps to,
//!   mirroring the mapping `sparq-lws-core`'s Solid Notifications implementation uses
//!   (`Create` / `Update` / `Delete` for the resource itself, `Add` / `Remove` for a
//!   container whose stored `ldp:contains` membership grew / shrank).
//!
//! # Content-free payloads (draft §10)
//!
//! A delivered notification carries the **topic IRI and the activity type only** — never
//! the changed triples. A subscriber learns *that* a resource it may read changed and
//! re-reads it through the authorized read path; the notification channel itself is not
//! a data channel, so it cannot leak content that a later authorization change would
//! have withheld.
//!
//! # Digest honesty
//!
//! [`TopicDigest`] is a 64-bit order-independent digest, not a cryptographic hash: two
//! genuinely different document states could in principle collide and suppress one
//! notification. That is a *missed signal*, never a leak, and it is the same
//! missed-update class the Solid Notifications reconnect contract already tells clients
//! to reconcile by re-reading. It is NOT a security boundary — authorization is enforced
//! separately at subscribe time and again at every delivery.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use oxrdf::{NamedNode, Term};
use serde_json::{json, Value};
use sparq_core::Graph;

/// The MCP method name of a resource-changed notification.
pub(crate) const RESOURCE_UPDATED_METHOD: &str = "notifications/resources/updated";

/// `ldp:contains` — the containment predicate whose count distinguishes a membership
/// change (`Add`/`Remove`) from a plain content change (`Update`).
const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";

/// The ActivityStreams 2.0 verb a detected state change maps to — the same mapping the
/// in-tree Solid Notifications emitter uses, so an MCP subscriber and a
/// WebSocketChannel2023 subscriber describe the same change with the same word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivityType {
    /// The document did not exist and now does.
    Create,
    /// The document existed before and after, and its content changed.
    Update,
    /// The document existed and no longer does.
    Delete,
    /// A container's stored `ldp:contains` membership grew.
    Add,
    /// A container's stored `ldp:contains` membership shrank.
    Remove,
}

impl ActivityType {
    /// The AS2 type token carried in the notification payload.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ActivityType::Create => "Create",
            ActivityType::Update => "Update",
            ActivityType::Delete => "Delete",
            ActivityType::Add => "Add",
            ActivityType::Remove => "Remove",
        }
    }
}

/// An order-independent content digest of one pod document, plus its `ldp:contains`
/// membership count. Equality means "no detected change" (see the [module
/// docs](self#digest-honesty) for the honest limit of that claim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TopicDigest {
    /// Order-independent 64-bit digest of the document's triples, with the triple count
    /// folded in.
    digest: u64,
    /// How many `ldp:contains` triples the document stores.
    contains: usize,
}

impl TopicDigest {
    /// Digest one document's stored graph. `O(triples)` and computed ONLY for topics a
    /// client actually subscribed to, so an unsubscribed server pays nothing.
    pub(crate) fn of(graph: &Graph) -> TopicDigest {
        let contains_id = NamedNode::new(LDP_CONTAINS)
            .ok()
            .and_then(|iri| graph.id_of(&Term::NamedNode(iri)));
        let mut digest: u64 = 0;
        let mut contains = 0usize;
        let mut triples = 0u64;
        for triple in graph.iter_ids() {
            let mut hasher = DefaultHasher::new();
            for id in triple {
                graph.dict.term(id).hash(&mut hasher);
            }
            // Summing per-triple hashes is order-independent: a graph is a SET, and the
            // scan order is an implementation detail we must not treat as content.
            digest = digest.wrapping_add(hasher.finish());
            triples += 1;
            if contains_id == Some(triple[1]) {
                contains += 1;
            }
        }
        TopicDigest {
            digest: digest.wrapping_mul(31).wrapping_add(triples),
            contains,
        }
    }
}

/// Classify the change between a topic's before/after digests, or `None` when nothing
/// changed (⇒ no notification is emitted).
///
/// A membership delta wins over a plain content delta: when a container's
/// `ldp:contains` count moved, the change IS the membership change as far as a
/// subscriber is concerned, so it is reported as `Add`/`Remove` rather than `Update`.
pub(crate) fn classify(
    before: Option<&TopicDigest>,
    after: Option<&TopicDigest>,
) -> Option<ActivityType> {
    match (before, after) {
        (None, None) => None,
        (None, Some(_)) => Some(ActivityType::Create),
        (Some(_), None) => Some(ActivityType::Delete),
        (Some(b), Some(a)) if b == a => None,
        (Some(b), Some(a)) => Some(match a.contains.cmp(&b.contains) {
            std::cmp::Ordering::Greater => ActivityType::Add,
            std::cmp::Ordering::Less => ActivityType::Remove,
            std::cmp::Ordering::Equal => ActivityType::Update,
        }),
    }
}

/// Build the outbound `notifications/resources/updated` JSON-RPC message for one change.
/// CONTENT-FREE by construction: the params carry the topic IRI and the activity type,
/// and nothing else.
pub(crate) fn updated_notification(uri: &str, activity: ActivityType) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": RESOURCE_UPDATED_METHOD,
        "params": { "uri": uri, "activity": activity.as_str() },
    })
}

/// The subscribed-topic registry plus the outbound notification queue for ONE pod
/// server (i.e. one authenticated session — subscriptions are never shared across
/// sessions, so a subscription can never outlive the session's authorization context).
#[derive(Debug, Clone, Default)]
pub(crate) struct Subscriptions {
    /// Subscribed topic IRIs, ordered so snapshots and emissions are deterministic.
    topics: BTreeSet<String>,
    /// Serialized JSON-RPC notification messages awaiting the embedder's drain.
    pending: Vec<String>,
}

impl Subscriptions {
    /// Register `topic`; returns `true` when it was not already subscribed.
    pub(crate) fn subscribe(&mut self, topic: &str) -> bool {
        self.topics.insert(topic.to_string())
    }

    /// Drop `topic`; returns `true` when it had been subscribed. Idempotent — the MCP
    /// `resources/unsubscribe` reply is the same either way, so an unsubscribe never
    /// discloses whether a topic was being watched.
    pub(crate) fn unsubscribe(&mut self, topic: &str) -> bool {
        self.topics.remove(topic)
    }

    /// True when nothing is subscribed — the fast path that skips digesting entirely.
    pub(crate) fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }

    /// The subscribed topics, sorted (an owned snapshot: the caller re-borrows the
    /// server to authorize each delivery).
    pub(crate) fn topics(&self) -> Vec<String> {
        self.topics.iter().cloned().collect()
    }

    /// Queue one delivery. Called only AFTER the delivery-time read check passed.
    pub(crate) fn enqueue(&mut self, topic: &str, activity: ActivityType) {
        self.pending
            .push(updated_notification(topic, activity).to_string());
    }

    /// Drain every queued notification message.
    pub(crate) fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    fn graph(nt: &str) -> Graph {
        Graph::load_str(nt, "ntriples").expect("fixture parses")
    }

    #[test]
    fn digest_is_order_independent_but_content_sensitive() {
        let a = graph("<https://p.ex/a> <https://p.ex/p> <https://p.ex/b> .\n<https://p.ex/c> <https://p.ex/p> <https://p.ex/d> .\n");
        let b = graph("<https://p.ex/c> <https://p.ex/p> <https://p.ex/d> .\n<https://p.ex/a> <https://p.ex/p> <https://p.ex/b> .\n");
        let c = graph("<https://p.ex/a> <https://p.ex/p> <https://p.ex/z> .\n<https://p.ex/c> <https://p.ex/p> <https://p.ex/d> .\n");
        assert_eq!(
            TopicDigest::of(&a),
            TopicDigest::of(&b),
            "a graph is a set, not a list"
        );
        assert_ne!(
            TopicDigest::of(&a),
            TopicDigest::of(&c),
            "an object swap must be detected"
        );
    }

    #[test]
    fn digest_counts_containment_membership() {
        let empty = graph("<https://p.ex/c/> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Container> .\n");
        let one = graph("<https://p.ex/c/> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Container> .\n<https://p.ex/c/> <http://www.w3.org/ns/ldp#contains> <https://p.ex/c/x> .\n");
        assert_eq!(TopicDigest::of(&empty).contains, 0);
        assert_eq!(TopicDigest::of(&one).contains, 1);
    }

    #[test]
    fn classify_maps_every_transition_to_its_as2_verb() {
        let empty = TopicDigest::of(&graph(""));
        let one = TopicDigest::of(&graph(
            "<https://p.ex/c/> <http://www.w3.org/ns/ldp#contains> <https://p.ex/c/x> .\n",
        ));
        let other = TopicDigest::of(&graph("<https://p.ex/c/> <https://p.ex/p> \"v\" .\n"));

        assert_eq!(classify(None, None), None);
        assert_eq!(classify(None, Some(&one)), Some(ActivityType::Create));
        assert_eq!(classify(Some(&one), None), Some(ActivityType::Delete));
        assert_eq!(classify(Some(&one), Some(&one)), None);
        assert_eq!(classify(Some(&empty), Some(&one)), Some(ActivityType::Add));
        assert_eq!(
            classify(Some(&one), Some(&empty)),
            Some(ActivityType::Remove)
        );
        assert_eq!(
            classify(Some(&empty), Some(&other)),
            Some(ActivityType::Update)
        );
    }

    #[test]
    fn activity_type_tokens_are_the_as2_verbs() {
        assert_eq!(ActivityType::Create.as_str(), "Create");
        assert_eq!(ActivityType::Update.as_str(), "Update");
        assert_eq!(ActivityType::Delete.as_str(), "Delete");
        assert_eq!(ActivityType::Add.as_str(), "Add");
        assert_eq!(ActivityType::Remove.as_str(), "Remove");
    }

    #[test]
    fn updated_notification_is_content_free() {
        let msg = updated_notification("https://p.ex/notes/n1", ActivityType::Update);
        assert_eq!(msg["jsonrpc"], "2.0");
        assert_eq!(msg["method"], RESOURCE_UPDATED_METHOD);
        assert_eq!(msg["params"]["uri"], "https://p.ex/notes/n1");
        assert_eq!(msg["params"]["activity"], "Update");
        // Load-bearing for §10: the payload carries the topic + verb and NOTHING else.
        let params = msg["params"].as_object().expect("params object");
        assert_eq!(
            params.len(),
            2,
            "notification payloads must stay content-free"
        );
        assert!(msg.get("result").is_none() && msg.get("id").is_none());
    }

    #[test]
    fn subscriptions_register_deregister_and_drain() {
        let mut subs = Subscriptions::default();
        assert!(subs.is_empty());
        assert!(subs.subscribe("https://p.ex/a"));
        assert!(
            !subs.subscribe("https://p.ex/a"),
            "re-subscribe is idempotent"
        );
        assert!(!subs.is_empty());
        assert_eq!(subs.topics(), vec!["https://p.ex/a".to_string()]);

        subs.enqueue("https://p.ex/a", ActivityType::Create);
        let drained = subs.take();
        assert_eq!(drained.len(), 1);
        assert!(drained[0].contains(RESOURCE_UPDATED_METHOD));
        assert!(subs.take().is_empty(), "draining twice yields nothing");

        assert!(subs.unsubscribe("https://p.ex/a"));
        assert!(
            !subs.unsubscribe("https://p.ex/a"),
            "unsubscribe is idempotent"
        );
        assert!(subs.is_empty());
    }
}

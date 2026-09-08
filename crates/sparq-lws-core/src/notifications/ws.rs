// AUTHORED-BY Claude Opus 4.8
//! The WebSocketChannel2023 HTTP surface: discovery, subscribe, and the WS receive endpoint.
//!
//! Route layout (mounted by [`crate::app::build_router`]):
//! - `POST /.notifications/WebSocketChannel2023/`  — subscribe; returns a channel description with a
//!   `receiveFrom` `ws(s)://` URL that carries a minted receive token. **Auth-gated** (behind the
//!   DPoP middleware — fail-closed) **and WAC-gated** (the subscriber must hold `acl:Read` on the
//!   topic, `acl:Control` for an `.acl`); the token binds receive to the authenticated
//!   subscriber+topic.
//! - `GET  /.notifications/WebSocketChannel2023/receive?topic=<iri>&token=<tok>` — upgrade to a
//!   WebSocket and register the connection under `<iri>`. **Token-gated:** the `token` must be a
//!   valid, unexpired receive token whose bound topic matches `<iri>`, else the upgrade is rejected
//!   (401, no socket). **Then WAC-gated again** for the WebID that token names, so a revoked grant
//!   cannot be replayed within the token's TTL (403, no socket). The server then pushes AS2.0
//!   notifications on change.
//! - `GET  /.well-known/solid`                     — a storage-description document advertising the
//!   subscription service (discovery; unauthenticated, like a storage description).
//!
//! ## Discovery (per the Solid Notifications Protocol)
//! A client finds the channel two ways, both implemented here:
//! 1. the `/.well-known/solid` storage description lists the `notificationChannel` subscription
//!    service + its supported `channelType`, and
//! 2. [`link_headers`] returns the `Link` rels (`describedby` + `solid:storageDescription`) the LDP
//!    GET/HEAD handler can attach so a client can `HEAD` a resource and discover the same service.
//!    (Attaching them to the LDP responses is a one-line wire in the handler; this module owns the
//!    values so the discovery contract lives in one place.)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::auth::VerifiedToken;
use crate::authz::{is_acl_resource, AccessMode, Decision, WacAuthorizer};
use crate::ldp::handler::{request_origin, LdpState};
use crate::notifications::activity::{AS2_CONTEXT, NOTIFICATIONS_CONTEXT};
use crate::notifications::NotificationHub;
use crate::store::Store;

/// The WebSocketChannel2023 channel-type IRI (the spec's `type` value).
pub const WEBSOCKET_CHANNEL_2023_TYPE: &str =
    "http://www.w3.org/ns/solid/notifications#WebSocketChannel2023";
/// The path of the subscription service (the POST target).
pub const SUBSCRIPTION_PATH: &str = "/.notifications/WebSocketChannel2023/";
/// The path of the WS receive endpoint (the GET-upgrade target; topic in `?topic=`).
pub const RECEIVE_PATH: &str = "/.notifications/WebSocketChannel2023/receive";
/// The storage-description / well-known discovery document path.
pub const WELL_KNOWN_SOLID_PATH: &str = "/.well-known/solid";

// [GPT-5.6] Keep the hot-path counters private so callers can observe but cannot mutate them.
struct NotificationMetricCounters {
    dropped_subscribers_total: AtomicU64,
    lagged_events_total: AtomicU64,
}

static NOTIFICATION_METRICS: LazyLock<NotificationMetricCounters> =
    LazyLock::new(|| NotificationMetricCounters {
        dropped_subscribers_total: AtomicU64::new(0),
        lagged_events_total: AtomicU64::new(0),
    });

/// Process-wide WebSocket notification overflow counters.
///
/// Obtain the current values with [`NotificationMetrics::snapshot`]. Counts are monotonic for the
/// lifetime of the process and use relaxed atomic ordering because they are observability-only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationMetrics {
    /// Subscribers closed after falling behind the per-topic broadcast buffer.
    pub dropped_subscribers_total: u64,
    /// Notification events skipped by lagged subscribers before they were closed.
    pub lagged_events_total: u64,
}

impl NotificationMetrics {
    /// Returns a point-in-time snapshot of the process-wide notification overflow counters.
    #[must_use]
    pub fn snapshot() -> Self {
        Self {
            dropped_subscribers_total: NOTIFICATION_METRICS
                .dropped_subscribers_total
                .load(Ordering::Relaxed),
            lagged_events_total: NOTIFICATION_METRICS
                .lagged_events_total
                .load(Ordering::Relaxed),
        }
    }
}

/// The single WAC-denial response for a notification topic, shared by the subscribe and receive
/// seams so a non-reader sees the SAME `403` on both — and the same one whether or not the topic
/// exists (the decision never probes the resource). Carries no topic-specific detail.
fn topic_denied() -> Response {
    (
        StatusCode::FORBIDDEN,
        "not authorized to read notifications for this topic",
    )
        .into_response()
}

/// The response when the ACL chain could NOT be resolved (a store fault). A backend failure must
/// never read as a grant, so it is a `500` — never an accept.
fn authorization_unavailable() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "authorization could not be evaluated",
    )
        .into_response()
}

/// State for the notification routes: the hub + the server's public base URL (for building the
/// absolute `receiveFrom` / subscription-service URLs in discovery + subscribe responses) + the LDP
/// state the per-resource WAC check runs against.
///
/// It holds the SAME [`LdpState`] the LDP routes carry, so the notification surface authorizes a
/// topic through the SAME store, base URL and parsed-ACL cache as a GET of that resource would —
/// the two gates cannot drift onto different views of the ACL graph.
pub struct NotifyState<S: Store> {
    pub hub: NotificationHub,
    pub base_url: String,
    /// The LDP state — the store + ACL cache the [`WacAuthorizer`] reads `.acl` resources through.
    ldp: Arc<LdpState<S>>,
}

impl<S: Store> NotifyState<S> {
    /// Derive the notification state from the LDP state: the hub and base URL are TAKEN from it
    /// (single-sourced), so a subscriber registered via `…/receive` is on the same registry the LDP
    /// emit path fans to and the advertised URLs match the LDP layer's own base.
    pub fn new(ldp: Arc<LdpState<S>>) -> Self {
        Self {
            hub: ldp.notifications.clone(),
            base_url: ldp.base_url().to_string(),
            ldp,
        }
    }

    /// The per-resource WAC gate for a notification topic, shared by BOTH the subscribe seam and the
    /// receive seam so the two cannot drift (the design's lock-step requirement).
    ///
    /// A change notification discloses that — and when — the topic changed, so watching a resource is
    /// a READ of it: the required mode is [`AccessMode::Read`], overridden to
    /// [`AccessMode::Control`] for an `.acl` topic exactly as the LDP read path overrides it
    /// (managing access rules is always Control) — i.e. the same mode
    /// [`mode_for_operation`](crate::authz::mode_for_operation) yields for a `GET` of the topic.
    /// `origin` is threaded so an `acl:origin`-restricted grant is honoured, identically to a GET.
    ///
    /// It uses the SEQUENTIAL [`WacAuthorizer::authorize`] rather than the read path's planned
    /// variant: subscribe/connect are not the hot path, and the planned walk exists only to collapse
    /// the per-candidate probes into one round-trip — the two are differentially tested to produce
    /// identical decisions, so the gate is the same, just without the batching machinery.
    ///
    /// Returns the response to send on a DENIAL, or `None` when the WebID is authorized:
    /// - denied → a uniform `403` (the caller is always an authenticated WebID here, so WAC's
    ///   anonymous-`401` branch is unreachable; it is mapped to the same `403` rather than left to
    ///   fall through, so no code path can turn a denial into an accept);
    /// - a store fault while resolving the ACL chain → `500`, never an accept (fail-closed).
    ///
    /// **Existence non-disclosure** (`research/lws-design-records.md` §6): this resolves the ACL
    /// chain ONLY — it never probes the topic resource — so the `403` is byte-identical whether the
    /// topic exists or not, and it adds no new existence oracle on the topic IRI.
    async fn deny_unless_may_read(
        &self,
        topic: &str,
        web_id: &str,
        origin: Option<&str>,
    ) -> Option<Response> {
        let required = if is_acl_resource(topic) {
            AccessMode::Control
        } else {
            AccessMode::Read
        };
        let wac = WacAuthorizer::with_cache(&self.ldp.store, &self.base_url, &self.ldp.acl_cache);
        match wac.authorize(topic, required, Some(web_id), origin).await {
            Ok(Decision::Allow(_)) => None,
            Ok(Decision::Forbidden | Decision::Unauthenticated) => Some(topic_denied()),
            // A backend fault resolving the ACL chain must never read as a grant.
            Err(_) => Some(authorization_unavailable()),
        }
    }

    /// The absolute subscription-service URL (the POST target).
    fn subscription_service_url(&self) -> String {
        format!("{}{SUBSCRIPTION_PATH}", self.base_url.trim_end_matches('/'))
    }

    /// The `receiveFrom` WebSocket URL for a topic, carrying the minted receive `token`. The base
    /// URL's scheme is mapped http→ws / https→wss (WebSocketChannel2023 §receiveFrom — the receive
    /// endpoint is a WebSocket URL). The token authorizes the WS upgrade for this topic (a browser
    /// `WebSocket` cannot send the DPoP `Authorization` header, so the spec carries authz in the URL).
    fn receive_from_url(&self, topic: &str, token: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let ws_base = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            base.to_string()
        };
        // URL-encode the topic + token into the query string (minimal: encode the few reserved chars
        // that matter for a query value; the topic is a server-issued absolute IRI and the token is a
        // server-issued base64url string — neither is user free-text).
        format!(
            "{ws_base}{RECEIVE_PATH}?topic={}&token={}",
            encode_query_value(topic),
            encode_query_value(token),
        )
    }
}

/// The JSON-LD subscription request body a client POSTs. Per WebSocketChannel2023 the client sends a
/// `type` (the channel-type IRI) and a `topic` (the resource/container to watch). We accept the flat
/// shape from the skill; extra JSON-LD framing fields are ignored.
#[derive(Debug, Deserialize)]
pub struct SubscriptionRequest {
    /// The channel type IRI; must be the WebSocketChannel2023 type. (Optional in the parse — a
    /// missing/other type is rejected in the handler with a clear 400, not a silent accept.)
    #[serde(rename = "type")]
    pub channel_type: Option<String>,
    /// The resource OR container IRI to watch.
    pub topic: Option<String>,
}

/// `POST /.notifications/WebSocketChannel2023/` — subscribe to a topic.
///
/// **Auth (fail-closed):** the caller MUST be authenticated (a WebID). An anonymous/public caller is
/// rejected with 401 — there are NO anonymous subscriptions. (This handler runs behind the DPoP auth
/// middleware, which injects the [`VerifiedToken`]; `is_public()` ⇒ unauthenticated.)
///
/// **Authorization (per-resource WAC):** the authenticated WebID must additionally hold `acl:Read`
/// on `topic` (`acl:Control` when the topic is an `.acl`) — a change notification discloses that and
/// when the resource changed, so watching it is a read of it. The decision comes from the SAME
/// [`WacAuthorizer`] the LDP read path uses, over the same store + ACL cache — the shared private
/// `NotifyState::deny_unless_may_read` gate. A non-reader gets a uniform `403` and NO receive token
/// is minted. This closes the gap where any authenticated WebID could subscribe to any topic IRI.
///
/// On success the handler MINTS an unguessable, short-lived **receive token** bound to
/// `(authenticated WebID, topic, expiry)` and embeds it in the `receiveFrom` URL — this is what gates
/// the otherwise-headerless WS receive endpoint (see [`receive_handler`], which re-runs the SAME WAC
/// check at connect time so a revoked grant cannot be replayed within the token's TTL).
///
/// Ordering: the request-shape rejections (unknown channel type, missing topic — `400`) precede the
/// WAC call because it needs the topic, and they depend ONLY on the request body, never on the
/// resource, so they disclose nothing about it.
pub async fn subscribe_handler<S: Store>(
    State(state): State<Arc<NotifyState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    headers: HeaderMap,
    Json(req): Json<SubscriptionRequest>,
) -> Response {
    // Fail-closed: no anonymous subscriptions. After this check `web_id` is `Some`.
    let web_id = match &token.web_id {
        Some(w) => w.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                "authentication required to subscribe",
            )
                .into_response();
        }
    };

    // Validate the channel type if the client sent one (reject a wrong type rather than silently
    // treating it as WebSocketChannel2023).
    if let Some(ty) = req.channel_type.as_deref() {
        if ty != WEBSOCKET_CHANNEL_2023_TYPE {
            return (
                StatusCode::BAD_REQUEST,
                "unsupported channel type (only WebSocketChannel2023)",
            )
                .into_response();
        }
    }

    let topic = match req.topic.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return (StatusCode::BAD_REQUEST, "missing topic").into_response(),
    };

    // Per-resource WAC: this authenticated WebID must be able to READ the topic. A denial returns
    // before anything is minted or registered.
    if let Some(denial) = state
        .deny_unless_may_read(topic, &web_id, request_origin(&headers))
        .await
    {
        return denial;
    }

    // Mint the receive token: unguessable, short-lived, bound to (this authenticated WebID, topic).
    // Without it the receive endpoint refuses the upgrade — so only this authenticated subscriber of
    // this topic can connect. The token (never logged) is embedded in `receiveFrom`.
    let receive_token = state.hub.mint_receive_token(&web_id, topic).await;
    let receive_from = state.receive_from_url(topic, &receive_token);

    // The channel description: per WebSocketChannel2023, `receiveFrom` is the ws(s):// URL the client
    // opens. We do NOT pre-register the topic here — registration happens when the WebSocket connects
    // (so a subscribe POST that is never followed by a connect leaks nothing).
    let body = json!({
        "@context": [NOTIFICATIONS_CONTEXT, AS2_CONTEXT],
        "id": receive_from,
        "type": WEBSOCKET_CHANNEL_2023_TYPE,
        "topic": topic,
        "receiveFrom": receive_from,
    });
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
        body.to_string(),
    )
        .into_response()
}

/// Query params for the WS receive endpoint.
#[derive(Debug, Deserialize)]
pub struct ReceiveQuery {
    pub topic: Option<String>,
    /// The receive token minted by the authenticated subscribe (see [`subscribe_handler`]). Required.
    pub token: Option<String>,
}

/// `GET /.notifications/WebSocketChannel2023/receive?topic=<iri>&token=<tok>` — upgrade to a
/// WebSocket and stream notifications for `<iri>`.
///
/// ## Auth on the WS upgrade — token-gated (the spec reality, implemented)
/// A browser `WebSocket` cannot carry the DPoP-bound `Authorization` header, so per the spec the
/// `receiveFrom` URL carries its own short-lived authorization. We REQUIRE a valid **receive token**
/// here: it must exist, be unexpired, and its bound topic must equal the requested `topic`. The token
/// is minted ONLY by the authenticated subscribe (bound to that WebID + topic), so a connection
/// without a token — or with an invalid / expired / wrong-topic token — is rejected (401, NO socket,
/// NO subscriber registered). This closes the previously-open receive bypass (anyone who guessed a
/// resource IRI could receive its change notifications without subscribing).
///
/// ## Authorization — the same per-resource WAC check as subscribe, re-run here
/// The token identifies WHICH authenticated WebID subscribed
/// ([`NotificationHub::authorize_receive_token`]); this handler then re-runs
/// `NotifyState::deny_unless_may_read` — the SAME private gate [`subscribe_handler`] used — for that
/// WebID. Re-checking rather than trusting the mint keeps the two seams in lock-step and means a
/// grant REVOKED between subscribe and connect cannot be replayed for the remainder of the token's
/// TTL. A WebID that no longer holds `acl:Read` gets a `403` and NO socket, NO subscriber registered.
///
/// `ws` is taken as a `Result` (not a bare `WebSocketUpgrade`) ON PURPOSE: the token-gate must run
/// FIRST and UNCONDITIONALLY. If `WebSocketUpgrade` were a plain extractor, its rejection would
/// short-circuit BEFORE the token check — so a request with bad/missing upgrade headers would 426
/// without ever validating authorization, and (more importantly) the security gate would be coupled
/// to the WS extractor's success. By deferring the `Result`, we reject an absent/invalid/expired/
/// wrong-topic token with 401 regardless of the upgrade headers, and only surface the WS rejection
/// after the token has validated.
pub async fn receive_handler<S: Store>(
    State(state): State<Arc<NotifyState<S>>>,
    Query(q): Query<ReceiveQuery>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let topic = match q.topic {
        Some(t) if !t.is_empty() => t,
        _ => return (StatusCode::BAD_REQUEST, "missing topic").into_response(),
    };
    // Token-gate (runs FIRST, unconditionally): require a valid, unexpired, topic-matching receive
    // token. Reject (401, no socket) otherwise. We deliberately do NOT echo the token or distinguish
    // absent/invalid/expired in the response body — a uniform 401 avoids leaking which condition
    // failed.
    let token = match q.token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                "a valid receive token is required",
            )
                .into_response()
        }
    };
    let Some(web_id) = state.hub.authorize_receive_token(&token, &topic).await else {
        return (
            StatusCode::UNAUTHORIZED,
            "a valid receive token is required",
        )
            .into_response();
    };
    // The token validated and named its subscriber. Re-run the SAME per-resource WAC gate subscribe
    // ran, so a grant revoked since the mint cannot be replayed for the rest of the token's TTL.
    if let Some(denial) = state
        .deny_unless_may_read(&topic, &web_id, request_origin(&headers))
        .await
    {
        return denial;
    }
    // The token validated. Now the request MUST be a genuine WS upgrade; surface the extractor's own
    // rejection (e.g. 426 Upgrade Required) if not.
    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };
    // Only AFTER the token validates do we upgrade + register a subscriber.
    let hub = state.hub.clone();
    ws.on_upgrade(move |socket| stream_notifications(socket, hub, topic))
}

/// The per-connection task: register a subscriber, forward every notification to the socket, and
/// clean up (drop the receiver ⇒ the hub prunes the topic on its next emit) when the socket closes.
///
/// Concurrency: a `tokio::select!` over (a) the next broadcast notification and (b) the next inbound
/// socket message. Inbound frames from the client are drained (a WebSocketChannel2023 receive socket
/// is server→client only; we read solely to observe a Close / a transport error so we can tear down
/// promptly and not leak the subscription).
async fn stream_notifications(mut socket: WebSocket, hub: NotificationHub, topic: String) {
    let mut rx = hub.subscribe(&topic).await;

    loop {
        tokio::select! {
            // (a) A notification for this topic — forward it as a text frame.
            received = rx.recv() => {
                match received {
                    Ok(body) => {
                        if socket.send(Message::text(body.to_string())).await.is_err() {
                            break; // the client went away mid-send
                        }
                    }
                    // The buffer overran for this slow client: a frame was dropped. Tell the client to
                    // reconcile by closing — it should re-subscribe + re-read (missed-update safety).
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        NOTIFICATION_METRICS
                            .dropped_subscribers_total
                            .fetch_add(1, Ordering::Relaxed);
                        NOTIFICATION_METRICS
                            .lagged_events_total
                            .fetch_add(skipped, Ordering::Relaxed);
                        let _ = socket
                            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                                code: 1011, // "internal error" / server overload — client reconnects
                                reason: "notification backlog overflow; reconnect and reconcile".into(),
                            })))
                            .await;
                        break;
                    }
                    // The sender was dropped (the topic channel went away) — nothing more will arrive.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // (b) An inbound socket message — only meaningful as a Close / error signal.
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break, // clean close or stream end
                    Some(Ok(_)) => { /* ignore any client frame; receive socket is server→client */ }
                    Some(Err(_)) => break, // transport error — tear down
                }
            }
        }
    }
    // `rx` drops here ⇒ the broadcast receiver count for `topic` decrements; the hub prunes a
    // now-0-receiver topic on its next emit. No explicit deregister call is needed — the registry is
    // self-cleaning, which is leak-free even if this task is cancelled.
}

/// `GET /.well-known/solid` — the storage-description / discovery document.
///
/// Advertises the notification subscription service + the supported channel type so a client can find
/// where to subscribe WITHOUT hardcoding the path. Unauthenticated (discovery is public, like a
/// storage description).
pub async fn storage_description_handler<S: Store>(
    State(state): State<Arc<NotifyState<S>>>,
) -> Response {
    let body = json!({
        "@context": [NOTIFICATIONS_CONTEXT, AS2_CONTEXT],
        "notificationChannel": [
            {
                "id": state.subscription_service_url(),
                "channelType": WEBSOCKET_CHANNEL_2023_TYPE,
                // The subscription service: POST a channel request here to obtain a `receiveFrom` URL.
                "subscriptionService": state.subscription_service_url(),
            }
        ],
    });
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/ld+json")],
        body.to_string(),
    )
        .into_response()
}

/// The discovery `Link` header VALUES the LDP GET/HEAD handler can attach to a resource response so a
/// client can `HEAD` the resource and find the storage description (which lists the subscription
/// service). Returns `(rel, target)` pairs; the caller formats `<target>; rel="rel"`.
///
/// This is the single home for the discovery contract — both the well-known document and the LDP
/// `Link` headers point at the same storage description, so the two never drift.
pub fn link_headers(base_url: &str) -> Vec<(&'static str, String)> {
    let base = base_url.trim_end_matches('/');
    let storage_desc = format!("{base}{WELL_KNOWN_SOLID_PATH}");
    vec![
        // The resource is described by the storage description (which lists notification channels).
        ("describedby", storage_desc.clone()),
        // The Solid storage-description rel (the protocol's discovery anchor).
        (
            "http://www.w3.org/ns/solid/terms#storageDescription",
            storage_desc,
        ),
    ]
}

/// Minimal percent-encoding for a URL query VALUE. Encodes the characters that would otherwise break
/// the query (`&`, `=`, `#`, `?`, space, `%`) and the IRI scheme separators are left as-is since the
/// topic is a server-issued absolute IRI. (Deliberately not a general URL-encoder — see the note in
/// [`NotifyState::receive_from_url`].)
fn encode_query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            // unreserved per RFC 3986 + the IRI chars common in an http(s) IRI we keep readable.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' | b'/' => {
                out.push(b as char)
            }
            other => {
                out.push('%');
                out.push(hex_digit(other >> 4));
                out.push(hex_digit(other & 0x0f));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
    use axum::body::Bytes;

    const BASE: &str = "https://pod.example";
    const ALICE: &str = "https://alice.example/profile#me";
    const BOB: &str = "https://bob.example/profile#me";

    type TestStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

    /// Notification state over an EMPTY store — no ACL exists anywhere, so WAC denies every topic
    /// (fail-closed). Used by the URL-shaping tests and by the request-shape rejections, which are
    /// all decided before the WAC gate.
    fn state() -> Arc<NotifyState<TestStore>> {
        state_over(CompositeStore::new(
            InMemorySparqClient::new(),
            InMemoryBlobStore::new(),
        ))
    }

    fn state_over(store: TestStore) -> Arc<NotifyState<TestStore>> {
        Arc::new(NotifyState::new(Arc::new(LdpState::new(store, BASE))))
    }

    /// State whose store carries a root `.acl` granting exactly `owner` Read+Write+Control over the
    /// whole pod (`acl:default`), so `owner` inherits Read on any topic and every other WebID
    /// inherits nothing.
    async fn state_owned_by(owner: &str) -> Arc<NotifyState<TestStore>> {
        use crate::store::Store;
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let body = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{owner}>;
         acl:accessTo <{BASE}/>;
         acl:default <{BASE}/>;
         acl:mode acl:Read, acl:Write, acl:Control."#
        );
        store
            .write(&format!("{BASE}/.acl"), Bytes::from(body), "text/turtle")
            .await
            .expect("seed root acl");
        state_over(store)
    }

    fn web_id_token(web_id: &str) -> VerifiedToken {
        VerifiedToken {
            web_id: Some(web_id.to_string()),
            ..VerifiedToken::default()
        }
    }

    fn subscribe_to(topic: &str) -> SubscriptionRequest {
        SubscriptionRequest {
            channel_type: Some(WEBSOCKET_CHANNEL_2023_TYPE.to_string()),
            topic: Some(topic.to_string()),
        }
    }

    #[test]
    fn receive_from_maps_https_to_wss() {
        let s = state();
        let url = s.receive_from_url("https://pod.example/a", "tok123");
        assert!(
            url.starts_with("wss://pod.example/.notifications/WebSocketChannel2023/receive?topic="),
            "{url}"
        );
        // The topic IRI round-trips (its reserved query chars are encoded).
        assert!(
            url.contains("https%3A%2F%2Fpod.example%2Fa") || url.contains("https://pod.example/a"),
            "{url}"
        );
        // The receive token is carried in the URL.
        assert!(url.contains("&token=tok123"), "{url}");
    }

    #[test]
    fn receive_from_maps_http_to_ws() {
        let s = Arc::new(NotifyState::new(Arc::new(LdpState::new(
            CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new()),
            "http://localhost:3000",
        ))));
        let url = s.receive_from_url("http://localhost:3000/a", "tok123");
        assert!(url.starts_with("ws://localhost:3000/"), "{url}");
    }

    #[test]
    fn subscription_service_url_is_absolute() {
        assert_eq!(
            state().subscription_service_url(),
            "https://pod.example/.notifications/WebSocketChannel2023/"
        );
    }

    #[test]
    fn link_headers_point_at_well_known() {
        let links = link_headers("https://pod.example");
        assert!(links
            .iter()
            .any(|(rel, t)| *rel == "describedby" && t == "https://pod.example/.well-known/solid"));
        assert!(links
            .iter()
            .any(|(rel, _)| rel.contains("storageDescription")));
    }

    #[test]
    fn encode_query_value_escapes_reserved() {
        // `&` and `=` and space and `#` must be encoded so they cannot break out of the query value.
        let e = encode_query_value("a&b=c d#e");
        assert!(!e.contains('&'));
        assert!(!e.contains(' '));
        assert!(!e.contains('#'));
        assert!(e.contains("%26") && e.contains("%3D") && e.contains("%20") && e.contains("%23"));
    }

    #[tokio::test]
    async fn subscribe_handler_rejects_anonymous() {
        let resp = subscribe_handler(
            State(state()),
            Extension(VerifiedToken::public()),
            HeaderMap::new(),
            Json(subscribe_to("https://pod.example/a")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn subscribe_handler_accepts_authenticated_and_returns_receive_from() {
        let resp = subscribe_handler(
            State(state_owned_by(ALICE).await),
            Extension(web_id_token(ALICE)),
            HeaderMap::new(),
            Json(subscribe_to("https://pod.example/a")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp).await;
        assert!(body.contains("\"receiveFrom\""), "{body}");
        assert!(
            body.contains("wss://pod.example/.notifications/WebSocketChannel2023/receive"),
            "{body}"
        );
        assert!(body.contains(WEBSOCKET_CHANNEL_2023_TYPE), "{body}");
    }

    // --- per-resource WAC on subscribe (the authorization gap this closes) ----------------------

    /// THE headline guard: an AUTHENTICATED WebID who is not granted `acl:Read` on another agent's
    /// resource cannot subscribe to it — so it never receives that resource's change notifications.
    #[tokio::test]
    async fn subscribe_handler_denies_authenticated_non_reader() {
        // Alice owns the pod; Bob is authenticated but holds nothing.
        let s = state_owned_by(ALICE).await;
        let resp = subscribe_handler(
            State(s.clone()),
            Extension(web_id_token(BOB)),
            HeaderMap::new(),
            Json(subscribe_to("https://pod.example/alice/secret")),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "an authenticated non-reader must not be able to subscribe to another agent's resource"
        );
        // And nothing was handed back that would let Bob connect anyway.
        let body = body_to_string(resp).await;
        assert!(
            !body.contains("receiveFrom"),
            "a denied subscribe must not hand back a receiveFrom URL: {body}"
        );
        // The topic has no channel either — a denied subscribe registers nothing.
        assert_eq!(
            s.hub.subscriber_count("https://pod.example/alice/secret").await,
            0
        );
    }

    /// Fail-closed: with NO ACL anywhere, even an authenticated caller is denied (WAC grants nothing
    /// by default) — the gate is not "allow unless explicitly denied".
    #[tokio::test]
    async fn subscribe_handler_denies_when_no_acl_exists() {
        let resp = subscribe_handler(
            State(state()),
            Extension(web_id_token(ALICE)),
            HeaderMap::new(),
            Json(subscribe_to("https://pod.example/a")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// Existence non-disclosure (`research/lws-design-records.md` §6): the denial is decided from the
    /// ACL chain alone, so a non-reader gets the SAME response for a topic that EXISTS and one that
    /// does not — subscribe adds no new existence oracle on the topic IRI.
    #[tokio::test]
    async fn subscribe_denial_does_not_disclose_topic_existence() {
        use crate::store::Store;
        let s = state_owned_by(ALICE).await;
        // Create one of the two topics; leave the other absent.
        s.ldp
            .store
            .write(
                "https://pod.example/alice/exists",
                Bytes::from("<a> <b> <c> ."),
                "text/turtle",
            )
            .await
            .expect("seed topic");

        let mut seen = Vec::new();
        for topic in [
            "https://pod.example/alice/exists",
            "https://pod.example/alice/absent",
        ] {
            let resp = subscribe_handler(
                State(s.clone()),
                Extension(web_id_token(BOB)),
                HeaderMap::new(),
                Json(subscribe_to(topic)),
            )
            .await;
            seen.push((resp.status(), body_to_string(resp).await));
        }
        assert_eq!(
            seen[0], seen[1],
            "the denial for an existing topic must be byte-identical to the denial for a missing one"
        );
    }

    /// The receive seam stays in lock-step: a token minted while the subscriber HELD read is refused
    /// once the grant is revoked, rather than remaining a bearer for the rest of its TTL.
    #[tokio::test]
    async fn receive_handler_rejects_token_whose_grant_was_revoked() {
        use crate::store::Store;
        let s = state_owned_by(BOB).await;
        let topic = "https://pod.example/shared";
        // Bob holds Read via the root ACL, so subscribe succeeds and mints a token.
        let token = s.hub.mint_receive_token(BOB, topic).await;
        assert!(s.deny_unless_may_read(topic, BOB, None).await.is_none());

        // The owner rewrites the root ACL, revoking Bob.
        s.ldp
            .store
            .write(
                &format!("{BASE}/.acl"),
                Bytes::from(format!(
                    r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{ALICE}>;
         acl:accessTo <{BASE}/>;
         acl:default <{BASE}/>;
         acl:mode acl:Read, acl:Write, acl:Control."#
                )),
                "text/turtle",
            )
            .await
            .expect("rewrite root acl");

        // The token itself is still structurally valid...
        assert!(s.hub.validate_receive_token(&token, topic).await);
        // ...but the WAC re-check the receive handler runs now denies Bob.
        let denial = s.deny_unless_may_read(topic, BOB, None).await;
        assert_eq!(
            denial.map(|r| r.status()),
            Some(StatusCode::FORBIDDEN),
            "a revoked grant must not be replayable through a still-unexpired receive token"
        );
    }

    /// The token's WebID binding is what the receive-side WAC check reads — it must come back
    /// exactly as minted, and only for the bound topic.
    #[tokio::test]
    async fn authorize_receive_token_returns_the_bound_web_id() {
        let hub = NotificationHub::new();
        let topic = "https://pod.example/a";
        let token = hub.mint_receive_token(BOB, topic).await;
        assert_eq!(
            hub.authorize_receive_token(&token, topic).await.as_deref(),
            Some(BOB)
        );
        assert_eq!(
            hub.authorize_receive_token(&token, "https://pod.example/b")
                .await,
            None,
            "a token bound to another topic must yield no identity"
        );
    }

    #[tokio::test]
    async fn subscribe_handler_rejects_wrong_channel_type() {
        let resp = subscribe_handler(
            State(state()),
            Extension(web_id_token(ALICE)),
            HeaderMap::new(),
            Json(SubscriptionRequest {
                channel_type: Some("http://example/OtherChannel".to_string()),
                topic: Some("https://pod.example/a".to_string()),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn subscribe_handler_rejects_missing_topic() {
        let resp = subscribe_handler(
            State(state()),
            Extension(web_id_token(ALICE)),
            HeaderMap::new(),
            Json(SubscriptionRequest {
                channel_type: None,
                topic: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn storage_description_advertises_subscription_service() {
        let resp = storage_description_handler(State(state())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_to_string(resp).await;
        assert!(body.contains("notificationChannel"), "{body}");
        assert!(body.contains(WEBSOCKET_CHANNEL_2023_TYPE), "{body}");
        assert!(
            body.contains("https://pod.example/.notifications/WebSocketChannel2023/"),
            "{body}"
        );
    }

    /// Drain a Response body to a String (test helper).
    async fn body_to_string(resp: Response) -> String {
        use http_body_util::BodyExt;
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }
}

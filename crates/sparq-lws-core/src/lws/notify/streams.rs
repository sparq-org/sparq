//! Capability-authenticated LWS SSE and WebSocket bindings.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, MissedTickBehavior};

use crate::notifications::ws::{authorization_unavailable, ReadAccess};

use super::*;

pub(super) const SSE: &str = "SseSubscription";
pub(super) const WEBSOCKET: &str = "WebSocketSubscription";
const STREAMS: &str = "/.lws/streams/";
const SOCKETS: &str = "/.lws/sockets/";
/// Shared limits for the two native incoming transports.
const MAX_CONNECTIONS: usize = 1024;
const QUEUE: usize = 64;
const RETAINED: usize = 256;
/// Bound serialized envelope storage independently of event count and age.
const RETAINED_BYTES: usize = 128 * 1024;
const RETENTION: Duration = Duration::from_secs(10 * 60);
const RECHECK: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(30);
/// Socket writes, including terminal frames, must not defeat revocation's deadline.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub(in crate::lws) fn stream_path(path: &str) -> bool {
    path.starts_with(STREAMS) || path.starts_with(SOCKETS)
}

pub(super) fn capability() -> (String, [u8; 32]) {
    let token = URL_SAFE_NO_PAD.encode(jose::random_bytes(32));
    let hash = Sha256::digest(token.as_bytes()).into();
    (token, hash)
}

pub(super) fn connection_details<S: Store>(
    state: &LwsState<S>,
    kind: SubscriptionKind,
    token: &str,
    doc: &mut Value,
) {
    if kind == SubscriptionKind::Sse {
        let source = state.cfg.absolute(&format!("{STREAMS}{token}"));
        doc["source"] = json!(source);
        // ADR-008 A4 also names receiveFrom for this binding.
        doc["receiveFrom"] = json!(source);
    } else {
        let url = state.cfg.absolute(&format!("{SOCKETS}{token}"));
        let url = url
            .replacen("https:", "wss:", 1)
            .replacen("http:", "ws:", 1);
        doc["receiveFrom"] = json!(url);
    }
}

#[derive(Clone)]
struct Retained {
    id: String,
    at: Instant,
    body: Arc<str>,
    watch: Watch,
}

#[derive(Clone)]
enum Frame {
    Notification { id: String, body: Arc<str> },
    Reset,
    Heartbeat,
}

impl Frame {
    fn sse(self) -> Bytes {
        Bytes::from(match self {
            Self::Notification { id, body } => {
                format!("id: {id}\nevent: notification\ndata: {body}\n\n")
            }
            Self::Reset => "event: reset\ndata:\n\n".into(),
            Self::Heartbeat => ": heartbeat\n\n".into(),
        })
    }
}

#[derive(Clone, Copy)]
enum End {
    Gone,
    Replaced,
    Slow,
    Revoked,
    Undecided,
}

struct Sender {
    data: mpsc::Sender<Frame>,
    end: mpsc::Sender<End>,
    /// Released on replacement/revocation even if the peer stops polling its body.
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}

impl Sender {
    fn close(&self, end: End) {
        let _ = self.end.try_send(end);
        self.permit.lock().expect("lock").take();
    }

    fn active(&self) -> bool {
        self.permit.lock().expect("lock").is_some() && !self.data.is_closed()
    }

    fn push(&self, frame: Frame) {
        if self.active() && self.data.try_send(frame).is_err() {
            self.close(End::Slow);
        }
    }
}

/// A subscription's retained events and its one current connection.
#[derive(Default)]
struct Channel {
    seq: u64,
    ring: VecDeque<Retained>,
    ring_bytes: usize,
    sender: Option<Arc<Sender>>,
}

impl Channel {
    fn prune(&mut self) {
        let now = Instant::now();
        while self
            .ring
            .front()
            .is_some_and(|e| now.duration_since(e.at) >= RETENTION)
            || self.ring.len() > RETAINED
            || self.ring_bytes > RETAINED_BYTES
        {
            if let Some(event) = self.ring.pop_front() {
                self.ring_bytes -= event.body.len();
            }
        }
    }
}

/// Serializes replay/registration and delivery for one subscription, without holding a resource lock.
#[derive(Default)]
struct Slot {
    gate: Arc<tokio::sync::Mutex<()>>,
    channel: Mutex<Channel>,
}

pub(super) struct Streams {
    epoch: String,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    capacity: Arc<Semaphore>,
}

impl Streams {
    pub(super) fn new() -> Self {
        Self {
            epoch: jose::random_id(),
            slots: Mutex::new(HashMap::new()),
            capacity: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        }
    }

    fn slot(&self, id: &str) -> Arc<Slot> {
        self.slots
            .lock()
            .expect("lock")
            .entry(id.into())
            .or_default()
            .clone()
    }

    pub(super) fn register(&self, id: &str) {
        self.slot(id);
    }

    pub(super) fn remove(&self, id: &str) {
        if let Some(slot) = self.slots.lock().expect("lock").remove(id) {
            if let Some(sender) = slot.channel.lock().expect("lock").sender.take() {
                sender.close(End::Gone);
            }
        }
    }
}

/// Recheck the original Watch immediately before retaining/pushing an event. Preparing it earlier
/// under the change's locks is what prevents a later grant from exposing an earlier change.
pub(super) async fn publish<S: Store + 'static>(state: &LwsState<S>, pending: Pending) {
    let id = &pending.watch.subscription;
    let Some(slot) = state
        .notify
        .streams
        .slots
        .lock()
        .expect("lock")
        .get(id)
        .cloned()
    else {
        return;
    };
    let _gate = slot.gate.lock().await;
    if !pending.watch.stands(state).await {
        return;
    }
    let mut channel = slot.channel.lock().expect("lock");
    channel.seq += 1;
    let event = Retained {
        id: format!("{}-{}", state.notify.streams.epoch, channel.seq),
        at: Instant::now(),
        body: Arc::from(envelope(&state.cfg.storage(), pending.activity).to_string()),
        watch: pending.watch,
    };
    if pending.kind == SubscriptionKind::Sse {
        channel.ring_bytes += event.body.len();
        channel.ring.push_back(event.clone());
        channel.prune();
    }
    if let Some(sender) = &channel.sender {
        sender.push(Frame::Notification {
            id: event.id,
            body: event.body,
        });
    }
}

/// Replay completes before live frames are polled, even if its Watch checks yield.
struct Frames {
    replay: Option<mpsc::Receiver<Frame>>,
    live: mpsc::Receiver<Frame>,
}

impl Frames {
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        if let Some(replay) = &mut self.replay {
            match replay.poll_recv(cx) {
                Poll::Ready(None) => self.replay = None,
                frame => return frame,
            }
        }
        self.live.poll_recv(cx)
    }

    async fn recv(&mut self) -> Option<Frame> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }
}

/// The receiver owns the connection's lifetime; dropping an HTTP body frees its slot immediately.
struct Connection {
    data: Frames,
    end: mpsc::Receiver<End>,
    sender: Arc<Sender>,
    done: bool,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.sender.close(End::Gone);
    }
}

impl axum::body::HttpBody for Connection {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        if let Poll::Ready(Some(end)) = self.end.poll_recv(cx) {
            self.done = true;
            return Poll::Ready(match end {
                End::Revoked => Some(Ok(hyper::body::Frame::data(Bytes::from_static(
                    b"event: revoked\ndata:\n\n",
                )))),
                _ => None,
            });
        }
        match self.data.poll_recv(cx) {
            Poll::Ready(Some(frame)) => {
                Poll::Ready(Some(Ok(hyper::body::Frame::data(frame.sse()))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Authenticate the capability before extracting an upgrade, with no request body buffering.
pub(in crate::lws) async fn connect<S: Store + 'static>(
    state: &LwsState<S>,
    req: Request,
) -> Response {
    if req.method() != Method::GET {
        return method_not_allowed("GET");
    }
    let path = req.uri().path();
    let (kind, token) = if let Some(token) = path.strip_prefix(STREAMS) {
        (SubscriptionKind::Sse, token)
    } else {
        (
            SubscriptionKind::WebSocket,
            path.strip_prefix(SOCKETS).unwrap_or_default(),
        )
    };
    if token.len() != 43 || URL_SAFE_NO_PAD.decode(token).is_err() {
        return problem(StatusCode::NOT_FOUND, None);
    }
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    // Index only SHA-256 digests of random 256-bit capabilities, never their tokens. A digest
    // identifies the record without scanning other subscriptions; confirm it in constant time.
    let sub = {
        let subs = state.notify.subs.read().expect("lock");
        subs.by_capability(&hash)
            .filter(|s| {
                s.kind == kind
                    && s.capability_hash
                        .is_some_and(|h| bool::from(h.ct_eq(&hash)))
            })
            .cloned()
    };
    let Some(sub) = sub else {
        return problem(StatusCode::NOT_FOUND, None);
    };
    if !state.notify.stands_now(&state.store, &sub.id).await {
        return problem(StatusCode::NOT_FOUND, None);
    }
    if kind == SubscriptionKind::Sse
        && !req
            .headers()
            .get(header::ACCEPT)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                h.split(',')
                    .any(|v| v.trim().split(';').next() == Some("text/event-stream"))
            })
    {
        return problem(
            StatusCode::NOT_ACCEPTABLE,
            Some("Accept must include text/event-stream"),
        );
    }
    let last_id = req
        .headers()
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let (mut parts, _body) = req.into_parts();
    // An upgrade task must never keep an admission permit through the socket's lifetime.
    parts.extensions.remove::<crate::overload::AdmissionSlot>();
    let upgrade = if kind == SubscriptionKind::WebSocket {
        match WebSocketUpgrade::from_request_parts(&mut parts, state).await {
            Ok(upgrade) => Some(upgrade),
            Err(refused) => return refused.into_response(),
        }
    } else {
        None
    };
    let Some(slot) = state
        .notify
        .streams
        .slots
        .lock()
        .expect("lock")
        .get(&sub.id)
        .cloned()
    else {
        return problem(StatusCode::NOT_FOUND, None);
    };
    let gate = slot.gate.clone().lock_owned().await;
    if !state.notify.stands_now(&state.store, &sub.id).await {
        return problem(StatusCode::NOT_FOUND, None);
    }
    match tokio::time::timeout(RECHECK, readable(state, &sub))
        .await
        .unwrap_or(ReadAccess::Undecided)
    {
        ReadAccess::Allowed => {}
        ReadAccess::Denied => {
            deactivate(state, &sub.id).await;
            return problem(StatusCode::FORBIDDEN, None);
        }
        ReadAccess::Undecided => {
            let mut response = authorization_unavailable();
            *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            set(response.headers_mut(), header::RETRY_AFTER, "1");
            return response;
        }
    }
    let (data_tx, data) = mpsc::channel(QUEUE);
    // A full replay snapshot plus reset fits without depending on client consumption.
    let (replay_tx, replay_rx) = mpsc::channel(RETAINED + 1);
    let (end_tx, end) = mpsc::channel(1);
    let (sender, replay, reset) = {
        // Serialize counts/registration across subscriptions as well as within this one.
        let slots = state.notify.streams.slots.lock().expect("lock");
        let quota = if access::is_owner(state, &sub.agent()) {
            MAX_OWNER_SUBSCRIPTIONS
        } else {
            MAX_SUBSCRIPTIONS_PER_SUBSCRIBER
        };
        let subs = state.notify.subs.read().expect("lock");
        let count = subs
            .mine(sub.subscriber.as_deref())
            .filter(|s| s.id != sub.id)
            .filter(|s| {
                slots.get(&s.id).is_some_and(|slot| {
                    slot.channel
                        .lock()
                        .expect("lock")
                        .sender
                        .as_ref()
                        .is_some_and(|s| s.active())
                })
            })
            .count();
        if count >= quota {
            return problem(StatusCode::TOO_MANY_REQUESTS, None);
        }
        let mut channel = slot.channel.lock().expect("lock");
        if let Some(old) = channel.sender.take() {
            old.close(End::Replaced);
        }
        let Ok(permit) = state.notify.streams.capacity.clone().try_acquire_owned() else {
            return problem(StatusCode::TOO_MANY_REQUESTS, None);
        };
        let sender = Arc::new(Sender {
            data: data_tx,
            end: end_tx,
            permit: Mutex::new(Some(permit)),
        });
        channel.prune();
        let index = last_id
            .as_ref()
            .and_then(|id| channel.ring.iter().position(|e| &e.id == id));
        let replay = index.map_or_else(Vec::new, |i| {
            channel.ring.iter().skip(i + 1).cloned().collect()
        });
        let reset = last_id.is_some() && index.is_none();
        channel.sender = Some(sender.clone());
        (sender, replay, reset)
    };
    // Registration and the snapshot are atomic with publication; client pacing is not.
    drop(gate);
    let connection = Connection {
        data: Frames {
            replay: Some(replay_rx),
            live: data,
        },
        end,
        sender: sender.clone(),
        done: false,
    };
    monitor(state, &sub, sender.clone());
    let weak = state.downgrade();
    // Recheck each Watch into a separate queue. Frames drains it before the live queue, so
    // publication can proceed while replay waits on authorization or the client.
    tokio::spawn(async move {
        if reset && replay_tx.try_send(Frame::Reset).is_err() {
            sender.close(End::Slow);
            return;
        }
        for event in replay {
            let Some(state) = weak.upgrade() else { return };
            if !sender.active() {
                return;
            }
            if event.watch.stands(&state).await {
                let frame = Frame::Notification {
                    id: event.id,
                    body: event.body,
                };
                if !matches!(
                    tokio::time::timeout(RECHECK, replay_tx.send(frame)).await,
                    Ok(Ok(()))
                ) {
                    sender.close(End::Slow);
                    return;
                }
            }
        }
    });
    if let Some(upgrade) = upgrade {
        let identity = connection.sender.clone();
        let weak = state.downgrade();
        upgrade
            .on_upgrade(move |socket| async move {
                let retry = socket_loop(socket, connection).await;
                // Retry closes keep the capability, although WebSocket has no event replay.
                if let Some(state) = weak.upgrade() {
                    let current = slot
                        .channel
                        .lock()
                        .expect("lock")
                        .sender
                        .as_ref()
                        .is_some_and(|s| Arc::ptr_eq(s, &identity));
                    if current && !retry {
                        deactivate(&state, &sub.id).await;
                    }
                }
            })
            .into_response()
    } else {
        let mut response = Response::new(Body::new(connection));
        set(
            response.headers_mut(),
            header::CONTENT_TYPE,
            "text/event-stream",
        );
        set(response.headers_mut(), header::CACHE_CONTROL, "no-store");
        set(
            response.headers_mut(),
            header::REFERRER_POLICY,
            "no-referrer",
        );
        response
    }
}

/// Any readable topic keeps the subscription usable. Only set-aside topics, store errors or
/// a caller's timeout make access undecided. An unreadable absent topic is denied by `check`,
/// just like an unreadable stored topic; absence alone never selects a different close.
async fn readable<S: Store + 'static>(state: &LwsState<S>, sub: &Subscription) -> ReadAccess {
    let mut access = ReadAccess::Denied;
    for topic in &sub.topics {
        let uri = topic.strip_suffix(META_SUFFIX).unwrap_or(topic);
        let Some(_guard) = state.locks.read(uri).await else {
            access = ReadAccess::Undecided;
            continue;
        };
        match state.check(Action::Read, uri, &sub.agent()).await {
            Ok(true) => return ReadAccess::Allowed,
            Ok(false) => {}
            Err(_) => access = ReadAccess::Undecided,
        }
    }
    access
}

async fn deactivate<S: Store + 'static>(state: &LwsState<S>, id: &str) {
    // Close first: a slow durable removal may not extend the revocation deadline. Withdrawing also
    // revokes the capability when removal has to be retried on a later request/restart.
    state
        .notify
        .withdrawn
        .write()
        .expect("lock")
        .insert(id.into());
    state.notify.streams.remove(id);
    let _ = state
        .notify
        .remove(
            state,
            id,
            None,
            super::super::records::Remover::Sweeper,
            false,
        )
        .await;
}

fn monitor<S: Store + 'static>(state: &LwsState<S>, sub: &Subscription, sender: Arc<Sender>) {
    let (weak, sub) = (state.downgrade(), sub.clone());
    tokio::spawn(async move {
        let start = Instant::now();
        let mut checks = tokio::time::interval_at(start + RECHECK, RECHECK);
        let mut heartbeats = tokio::time::interval_at(start + HEARTBEAT, HEARTBEAT);
        checks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        heartbeats.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = sender.data.closed() => return,
                _ = checks.tick() => {
                    if !sender.active() { return }
                    let Some(state) = weak.upgrade() else { sender.close(End::Gone); return };
                    let decision = async {
                        if state.notify.stands_now(&state.store, &sub.id).await {
                            readable(&state, &sub).await
                        } else {
                            ReadAccess::Denied
                        }
                    };
                    match tokio::time::timeout(RECHECK, decision).await.unwrap_or(ReadAccess::Undecided) {
                        ReadAccess::Allowed => {}
                        ReadAccess::Denied => {
                            sender.close(End::Revoked);
                            deactivate(&state, &sub.id).await;
                            return;
                        }
                        ReadAccess::Undecided => {
                            sender.close(End::Undecided);
                            return;
                        }
                    }
                }
                _ = heartbeats.tick() => {
                    if !sender.active() { return }
                    sender.push(Frame::Heartbeat);
                }
            }
        }
    });
}

trait StreamSocket: Send {
    type Error: Send;

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<Result<Message, Self::Error>>> + Send;
    fn transmit(
        &mut self,
        message: Message,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

impl StreamSocket for WebSocket {
    type Error = axum::Error;

    async fn receive(&mut self) -> Option<Result<Message, Self::Error>> {
        self.recv().await
    }

    async fn transmit(&mut self, message: Message) -> Result<(), Self::Error> {
        self.send(message).await
    }
}

/// A 1013 close asks the client to reconnect with the same capability. A 4403 close withdraws
/// the subscription, so the client must subscribe again; SSE marks that case as `revoked`.
async fn socket_loop(mut socket: impl StreamSocket, mut connection: Connection) -> bool {
    loop {
        let message = tokio::select! {
            biased;
            end = connection.end.recv() => {
                let retry = matches!(end, Some(End::Slow | End::Undecided));
                let code = match end { Some(End::Revoked) => 4403, Some(End::Slow | End::Undecided) => 1013, _ => 1000 };
                let _ = tokio::time::timeout(WRITE_TIMEOUT, socket.transmit(Message::Close(Some(CloseFrame { code, reason: "".into() })))).await;
                return retry;
            }
            frame = connection.data.recv() => match frame {
                Some(Frame::Notification { body, .. }) => Message::Text(body.to_string().into()),
                Some(Frame::Heartbeat) => Message::Ping(Bytes::new()),
                Some(Frame::Reset) => continue,
                None => return false,
            },
            incoming = socket.receive() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return false,
                _ => continue,
            },
        };
        if !matches!(
            tokio::time::timeout(WRITE_TIMEOUT, socket.transmit(message)).await,
            Ok(Ok(()))
        ) {
            // A retry end can arrive while a write is pending. Preserve it even when that
            // write fails or reaches its deadline before the next receive of the end queue.
            return matches!(connection.end.try_recv(), Ok(End::Slow | End::Undecided));
        }
    }
}

#[cfg(test)]
mod tests;

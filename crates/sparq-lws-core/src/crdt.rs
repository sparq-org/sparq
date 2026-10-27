//! Optional Yjs collaboration routes for the native Solid and LWS routers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::json;
use sparq_lws_ysync::{Access, Agent, Authorizer, Fault, Limits, LogFile, Relay, RoomStore};

use crate::auth::VerifiedToken;
use crate::authz::wac::{Decision, WacAuthorizer};
use crate::authz::AccessMode;
use crate::ldp::handler::{delete_handler, post_handler, LdpState};
use crate::lws::{self, access::Action, LwsRequest, LwsState};
use crate::store::Store;

pub(crate) const SOLID_TICKETS: &str = "/.crdt/tickets";
pub(crate) const SOLID_SOCKET: &str = "/.crdt/socket";
const LWS_TICKETS: &str = "/.lws/crdt/tickets";
const LWS_SOCKET: &str = "/.lws/crdt/socket";

/// Native relay bounds from environment. Values must be positive integers.
/// See the collaboration reference in the Solid/LWS skill for names and defaults.
pub fn limits_from_env() -> Result<Limits, String> {
    let mut limits = Limits::default();
    for (name, field) in [
        ("MAX_ROOMS", &mut limits.max_rooms),
        ("PEERS_PER_ROOM", &mut limits.peers_per_room),
        ("MESSAGE_BYTES", &mut limits.message_bytes),
        ("FRAME_MESSAGES", &mut limits.frame_messages),
        ("DOCUMENT_BYTES", &mut limits.document_bytes),
        ("TICKETS_PER_AGENT", &mut limits.tickets_per_agent),
        ("MAX_TICKETS", &mut limits.max_tickets),
        ("MAX_LOG_FILES", &mut limits.max_log_files),
        ("AWARENESS_CLIENTS", &mut limits.awareness_clients),
        ("COMPACT_AFTER", &mut limits.compact_after),
    ] {
        let key = format!("SOLID_SERVER_CRDT_{name}");
        if let Ok(value) = std::env::var(&key) {
            *field = value
                .parse::<usize>()
                .ok()
                .filter(|n| *n > 0 && *n <= 1024 * 1024 * 1024)
                .ok_or_else(|| format!("{key} must be a positive bounded integer"))?;
        }
    }
    Ok(limits)
}

pub(crate) enum Backend<S: Store> {
    Solid(Arc<LdpState<S>>),
    Lws(lws::WeakState<S>),
}

fn room_path(base: &str, room: &str) -> Result<String, Fault> {
    let url = url::Url::parse(room).map_err(|_| Fault::Forbidden)?;
    if !room.starts_with(&format!("{}/", base.trim_end_matches('/')))
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().ends_with('/')
    {
        return Err(Fault::Forbidden);
    }
    let path = url.path();
    let target = crate::ldp::target::parse_target(base, path).map_err(|_| Fault::Forbidden)?;
    if target.iri != room || path.contains("/.crdt/") {
        return Err(Fault::Forbidden);
    }
    Ok(path.to_string())
}

fn token(agent: &Agent) -> VerifiedToken {
    VerifiedToken {
        web_id: Some(agent.subject.clone()),
        client_id: agent.client.clone(),
        ..Default::default()
    }
}

fn headers(agent: &Agent) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    if let Some(origin) = &agent.origin {
        if let Ok(value) = origin.parse() {
            headers.insert(header::ORIGIN, value);
        }
    }
    headers
}

fn lws_agent(agent: &Agent) -> lws::Agent {
    lws::Agent {
        subject: Some(agent.subject.clone()),
        client: agent.client.clone(),
    }
}

fn request(method: Method, path: String, body: Bytes) -> LwsRequest {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    LwsRequest {
        method,
        path,
        query: None,
        headers,
        body,
        admission: None,
    }
}

#[async_trait]
impl<S: Store + 'static> Authorizer for Backend<S> {
    async fn access(&self, room: &str, agent: &Agent) -> Result<Access, Fault> {
        match self {
            Self::Solid(state) => {
                room_path(state.base_url(), room)?;
                let auth =
                    WacAuthorizer::with_cache(&state.store, state.base_url(), &state.acl_cache);
                let read = matches!(
                    auth.authorize(
                        room,
                        AccessMode::Read,
                        Some(&agent.subject),
                        agent.origin.as_deref()
                    )
                    .await
                    .map_err(|_| Fault::Store)?,
                    Decision::Allow(_)
                );
                if !read {
                    return Ok(Access {
                        read: false,
                        write: false,
                    });
                }
                let write = matches!(
                    auth.authorize(
                        room,
                        AccessMode::Append,
                        Some(&agent.subject),
                        agent.origin.as_deref()
                    )
                    .await
                    .map_err(|_| Fault::Store)?,
                    Decision::Allow(_)
                );
                Ok(Access {
                    read: state.store.exists(room).await.map_err(|_| Fault::Store)?,
                    write,
                })
            }
            Self::Lws(weak) => {
                let state = weak.upgrade().ok_or(Fault::Store)?;
                room_path(&state.cfg.base_url, room)?;
                let agent = lws_agent(agent);
                let read = state
                    .check(Action::Read, room, &agent)
                    .await
                    .map_err(|_| Fault::Store)?;
                if !read {
                    return Ok(Access {
                        read: false,
                        write: false,
                    });
                }
                let write = state
                    .check(Action::Create, room, &agent)
                    .await
                    .map_err(|_| Fault::Store)?;
                Ok(Access {
                    read: state.store.exists(room).await.map_err(|_| Fault::Store)?,
                    write,
                })
            }
        }
    }
}

#[async_trait]
impl<S: Store + 'static> RoomStore for Backend<S> {
    async fn list(&self, room: &str) -> Result<Vec<String>, Fault> {
        let children = match self {
            Self::Solid(s) => {
                s.store
                    .list_children_within(room, s.crdt_limits.max_log_files.saturating_mul(8192))
                    .await
            }
            Self::Lws(w) => {
                let state = w.upgrade().ok_or(Fault::Store)?;
                state
                    .store
                    .list_children_within(
                        room,
                        state.cfg.crdt_limits.max_log_files.saturating_mul(8192),
                    )
                    .await
            }
        }
        .map_err(|_| Fault::Store)?
        .ok_or(Fault::Capacity)?;
        Ok(children
            .into_iter()
            .filter(|c| !c.as_str().ends_with('/'))
            .map(|c| c.as_str().to_string())
            .collect())
    }

    async fn read(&self, file: &str) -> Result<Vec<u8>, Fault> {
        let resource = match self {
            Self::Solid(s) => s.store.read(file).await,
            Self::Lws(w) => w.upgrade().ok_or(Fault::Store)?.store.read(file).await,
        }
        .map_err(|_| Fault::Store)?;
        let limit = match self {
            Self::Solid(s) => s.crdt_limits.document_bytes,
            Self::Lws(w) => {
                w.upgrade()
                    .ok_or(Fault::Store)?
                    .cfg
                    .crdt_limits
                    .document_bytes
            }
        };
        if resource.body.len() > limit.saturating_add(16) {
            return Err(Fault::TooLarge);
        }
        Ok(resource.body.to_vec())
    }

    async fn create(&self, room: &str, agent: &Agent, file: &LogFile) -> Result<String, Fault> {
        // Re-evaluate at the write boundary, including retries after grant changes.
        let access = self.access(room, agent).await?;
        if !access.read || !access.write {
            return Err(Fault::Forbidden);
        }
        let body = Bytes::from(file.encode());
        let response = match self {
            Self::Solid(s) => {
                let path = room_path(s.base_url(), room)?;
                post_handler(
                    State(s.clone()),
                    Extension(token(agent)),
                    path.parse::<Uri>().map_err(|_| Fault::Store)?,
                    headers(agent),
                    body,
                )
                .await
                .map_err(|_| Fault::Store)?
            }
            Self::Lws(w) => {
                let state = w.upgrade().ok_or(Fault::Store)?;
                let path = room_path(&state.cfg.base_url, room)?;
                lws::resources::handle(
                    &state,
                    &request(Method::POST, path, body),
                    &lws_agent(agent),
                )
                .await
            }
        };
        if response.status() != StatusCode::CREATED {
            return Err(Fault::Store);
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(Fault::Store)?
            .to_string();
        if let Self::Solid(s) = self {
            // Solid's binary store has no creator field. Use the existing server metadata
            // representation so each flush retains its authenticated author.
            let meta = lws::ResourceMeta {
                creator: Some(agent.subject.clone()),
                ..Default::default()
            };
            s.store
                .write(
                    &lws::meta_key(&location),
                    Bytes::from(lws::encode_meta(&meta).map_err(|_| Fault::Store)?),
                    lws::LWS_JSON,
                )
                .await
                .map_err(|_| Fault::Store)?;
        }
        Ok(location)
    }

    async fn delete(&self, room: &str, agent: &Agent, file: &str) -> Result<(), Fault> {
        let access = self.access(room, agent).await?;
        if !access.read || !access.write {
            return Err(Fault::Forbidden);
        }
        let path = url::Url::parse(file)
            .map_err(|_| Fault::Store)?
            .path()
            .to_string();
        let response = match self {
            Self::Solid(s) => delete_handler(
                State(s.clone()),
                Extension(token(agent)),
                path.parse::<Uri>().map_err(|_| Fault::Store)?,
                headers(agent),
            )
            .await
            .map_err(|_| Fault::Store)?,
            Self::Lws(w) => {
                let state = w.upgrade().ok_or(Fault::Store)?;
                lws::resources::handle(
                    &state,
                    &request(Method::DELETE, path, Bytes::new()),
                    &lws_agent(agent),
                )
                .await
            }
        };
        if !response.status().is_success() {
            return Err(Fault::Store);
        }
        if let Self::Solid(s) = self {
            let _ = s.store.delete(&lws::meta_key(file), None).await;
        }
        Ok(())
    }
}

pub(crate) struct CrdtState<S: Store> {
    relay: Relay<Backend<S>>,
    socket_url: String,
    connections: Arc<tokio::sync::Semaphore>,
}

impl<S: Store + 'static> CrdtState<S> {
    pub(crate) fn solid(state: Arc<LdpState<S>>) -> Arc<Self> {
        let socket_url = socket_url(state.base_url(), SOLID_SOCKET);
        let changes = state.notifications.crdt_changes.clone();
        let limits = state.crdt_limits.clone();
        let connections = Arc::new(tokio::sync::Semaphore::new(
            limits.max_rooms.saturating_mul(limits.peers_per_room),
        ));
        Arc::new(Self {
            relay: Relay::new(Backend::Solid(state), limits, changes),
            socket_url,
            connections,
        })
    }

    pub(crate) fn lws(state: &LwsState<S>) -> Arc<Self> {
        Arc::new(Self {
            relay: Relay::new(
                Backend::Lws(state.downgrade()),
                state.cfg.crdt_limits.clone(),
                state.notify.crdt_changes.clone(),
            ),
            connections: Arc::new(tokio::sync::Semaphore::new(
                state
                    .cfg
                    .crdt_limits
                    .max_rooms
                    .saturating_mul(state.cfg.crdt_limits.peers_per_room),
            )),
            socket_url: socket_url(&state.cfg.base_url, LWS_SOCKET),
        })
    }
}

fn socket_url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
        .replacen("https:", "wss:", 1)
        .replacen("http:", "ws:", 1)
}

pub(crate) fn register(catalog: &mut crate::describe::ServiceCatalog, base: &str, solid: bool) {
    let path = if solid { SOLID_TICKETS } else { LWS_TICKETS };
    let socket = if solid { SOLID_SOCKET } else { LWS_SOCKET };
    let mut entry = crate::describe::entry(
        "crdt-sync",
        &format!("{}CrdtSyncService", crate::describe::SSD),
        format!("{}{path}", base.trim_end_matches('/')),
    );
    entry.extras.insert(
        "profile".into(),
        json!(format!("{}y-protocol-v1", crate::describe::SSD)),
    );
    entry.extras.insert(
        "roomTemplate".into(),
        json!(format!("{}?room={{room}}", socket_url(base, socket))),
    );
    catalog.service(entry);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketBody {
    room: String,
}

fn refused(fault: Fault) -> Response {
    let status = match fault {
        Fault::Capacity => StatusCode::TOO_MANY_REQUESTS,
        Fault::Forbidden => StatusCode::FORBIDDEN,
        Fault::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        Fault::Malformed => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    lws::problem(status, None)
}

async fn issue<S: Store + 'static>(state: &CrdtState<S>, agent: Agent, body: &[u8]) -> Response {
    if body.len() > 8192 {
        return refused(Fault::TooLarge);
    }
    let Ok(body) = serde_json::from_slice::<TicketBody>(body) else {
        return refused(Fault::Malformed);
    };
    let mut secret = [0u8; 32];
    if rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut secret)
        .is_err()
    {
        return refused(Fault::Store);
    }
    if let Err(error) = state.relay.ticket(secret, &body.room, agent).await {
        return refused(error);
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("room", &body.room)
        .append_pair("ticket", &URL_SAFE_NO_PAD.encode(secret))
        .finish();
    let expires = lws::access::format_rfc3339(
        crate::clock::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            + 60,
    );
    let mut response = (
        StatusCode::CREATED,
        axum::Json(json!({"url": format!("{}?{query}", state.socket_url), "expires": expires})),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

pub(crate) async fn solid_ticket<S: Store + 'static>(
    State(state): State<Arc<CrdtState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(subject) = token.web_id else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    issue(
        &state,
        Agent {
            subject,
            client: token.client_id,
            origin: headers
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
        },
        &body,
    )
    .await
}

pub(crate) async fn lws_ticket<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &lws::Agent,
) -> Response {
    if req.method != Method::POST {
        return lws::method_not_allowed("POST");
    }
    let Some(subject) = agent.subject.clone() else {
        return state.challenge(None);
    };
    issue(
        state.crdt.get().expect("mounted CRDT"),
        Agent {
            subject,
            client: agent.client.clone(),
            origin: None,
        },
        &req.body,
    )
    .await
}

#[derive(Deserialize)]
pub(crate) struct Join {
    room: String,
    ticket: String,
}

pub(crate) async fn solid_socket<S: Store + 'static>(
    State(state): State<Arc<CrdtState<S>>>,
    Query(query): Query<Join>,
    ws: WebSocketUpgrade,
) -> Response {
    upgrade(state, query, ws)
}

fn upgrade<S: Store + 'static>(
    state: Arc<CrdtState<S>>,
    query: Join,
    ws: WebSocketUpgrade,
) -> Response {
    let max = state.relay.limits().message_bytes.saturating_mul(2);
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| serve(state, query, socket))
}

pub(crate) async fn lws_socket<S: Store + 'static>(state: &LwsState<S>, req: Request) -> Response {
    let (mut parts, _) = req.into_parts();
    let query = match Query::<Join>::from_request_parts(&mut parts, &()).await {
        Ok(Query(q)) => q,
        Err(_) => return refused(Fault::Malformed),
    };
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    upgrade(state.crdt.get().expect("mounted CRDT").clone(), query, ws)
}

pub(crate) fn is_socket(path: &str) -> bool {
    path == LWS_SOCKET
}
pub(crate) fn is_ticket(path: &str) -> bool {
    path == LWS_TICKETS
}

async fn send(socket: &mut WebSocket, frame: Message) -> Result<(), Fault> {
    tokio::time::timeout(Duration::from_secs(2), socket.send(frame))
        .await
        .map_err(|_| Fault::Capacity)?
        .map_err(|_| Fault::Capacity)
}

async fn close(socket: &mut WebSocket, fault: Fault) {
    let _ = send(
        socket,
        Message::Close(Some(CloseFrame {
            code: fault.code(),
            reason: fault.to_string().into(),
        })),
    )
    .await;
}

async fn serve<S: Store + 'static>(state: Arc<CrdtState<S>>, query: Join, mut socket: WebSocket) {
    let Ok(_permit) = state.connections.clone().try_acquire_owned() else {
        close(&mut socket, Fault::Capacity).await;
        return;
    };
    let secret = URL_SAFE_NO_PAD
        .decode(query.ticket)
        .ok()
        .and_then(|s| s.try_into().ok());
    let Some(secret) = secret else {
        close(&mut socket, Fault::Ticket).await;
        return;
    };
    let mut session = match tokio::time::timeout(
        Duration::from_secs(10),
        state.relay.join(secret, &query.room),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            close(&mut socket, error).await;
            return;
        }
        Err(_) => {
            close(&mut socket, Fault::Store).await;
            return;
        }
    };
    let mut recheck = tokio::time::interval(Duration::from_secs(10));
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    let fault = loop {
        tokio::select! {
            biased;
            _ = session.ended.changed() => break Some(session.ended.borrow().unwrap_or(Fault::Store)),
            _ = recheck.tick() => {
                match tokio::time::timeout(Duration::from_secs(2), state.relay.recheck(&session)).await {
                    Ok(Ok(())) => {}, Ok(Err(error)) => break Some(error), Err(_) => break Some(Fault::Store),
                }
            }
            _ = ping.tick() => { if let Err(error) = send(&mut socket, Message::Ping(Bytes::new())).await { break Some(error); } }
            Some(frame) = session.incoming.recv() => {
                match frame {
                    Ok(frame) => if let Err(error) = send(&mut socket, Message::Binary(Bytes::copy_from_slice(&frame))).await { break Some(error); },
                    Err(error) => break Some(error),
                }
            }
            frame = socket.recv() => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => if let Err(error) = state.relay.handle(&session, &bytes).await { break Some(error); },
                    Some(Ok(Message::Ping(bytes))) => { if let Err(error) = send(&mut socket, Message::Pong(bytes)).await { break Some(error); } },
                    Some(Ok(Message::Pong(_))) => {},
                    Some(Ok(Message::Close(_))) | None => break None,
                    Some(Err(_)) => break Some(Fault::TooLarge),
                    _ => break Some(Fault::Malformed),
                }
            }
        }
    };
    // Remove presence before attempting the terminal network write.
    state.relay.leave(&session).await;
    if let Some(fault) = fault {
        close(&mut socket, fault).await;
    } else {
        // Drive the queued close reply to completion before releasing the transport.
        let _ = tokio::time::timeout(Duration::from_secs(2), socket.recv()).await;
    }
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::Instant;
use yrs::merge_updates_v1;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::encoder::Encode;

use crate::room::{messages, Document};
use crate::{load, Fault, LogFile, LogKind};

/// Authenticated request identity, retained without credentials.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Agent {
    pub subject: String,
    pub client: Option<String>,
    pub origin: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct Access {
    pub read: bool,
    pub write: bool,
}

#[async_trait]
pub trait Authorizer: Send + Sync + 'static {
    /// Include room validation and existence in the read decision.
    async fn access(&self, room: &str, agent: &Agent) -> Result<Access, Fault>;
}

#[async_trait]
pub trait RoomStore: Send + Sync + 'static {
    async fn list(&self, room: &str) -> Result<Vec<String>, Fault>;
    async fn read(&self, file: &str) -> Result<Vec<u8>, Fault>;
    /// Ordinary member creation attributed to `agent`; return only after confirmed creation.
    async fn create(&self, room: &str, agent: &Agent, file: &LogFile) -> Result<String, Fault>;
    async fn delete(&self, room: &str, agent: &Agent, file: &str) -> Result<(), Fault>;
}

/// Per-service finite bounds. Durations are fixed protocol deadlines.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_rooms: usize,
    pub peers_per_room: usize,
    pub message_bytes: usize,
    pub frame_messages: usize,
    pub document_bytes: usize,
    pub tickets_per_agent: usize,
    pub max_tickets: usize,
    pub max_log_files: usize,
    pub awareness_clients: usize,
    pub compact_after: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rooms: 128,
            peers_per_room: 32,
            message_bytes: 1024 * 1024,
            frame_messages: 32,
            document_bytes: 16 * 1024 * 1024,
            tickets_per_agent: 8,
            max_tickets: 1024,
            max_log_files: 4096,
            awareness_clients: 256,
            compact_after: 64,
        }
    }
}

/// Internal container-create feed. No network subscription or polling is involved.
#[derive(Clone, Default)]
pub struct Changes(Arc<StdMutex<HashMap<String, broadcast::Sender<String>>>>);

impl Changes {
    fn subscribe(&self, room: &str) -> broadcast::Receiver<String> {
        let mut topics = self.0.lock().expect("change lock");
        topics.retain(|_, tx| tx.receiver_count() > 0);
        topics
            .entry(room.to_string())
            .or_insert_with(|| broadcast::channel(128).0)
            .subscribe()
    }

    pub fn created(&self, room: &str, file: &str) {
        if let Some(tx) = self.0.lock().expect("change lock").get(room) {
            let _ = tx.send(file.to_string());
        }
    }
}

struct Ticket {
    room: String,
    agent: Agent,
    write: bool,
    expires: Instant,
}

struct Peer {
    write: bool,
    tx: mpsc::Sender<Result<Arc<[u8]>, Fault>>,
    end: tokio::sync::watch::Sender<Option<Fault>>,
}

#[derive(Clone)]
struct Pending {
    update: Vec<u8>,
    revision: u64,
    file: LogFile,
    retry: Instant,
    delay: Duration,
}

struct Room {
    doc: Document,
    peers: HashMap<u64, Peer>,
    pending: HashMap<Agent, Pending>,
    flushing: Option<(Agent, u64)>,
    files: Vec<String>,
    update_files: usize,
    last_writer: Option<Agent>,
    last_left: Instant,
    sequence: u64,
    newest_ms: u64,
    needs_reload: bool,
    unloading: bool,
}

impl Room {
    fn broadcast(&mut self, except: u64, bytes: Vec<u8>) {
        let bytes: Arc<[u8]> = bytes.into();
        for (id, peer) in &self.peers {
            if *id != except && peer.tx.try_send(Ok(bytes.clone())).is_err() {
                let _ = peer.end.send(Some(Fault::Capacity));
            }
        }
    }
}

struct Inner<B> {
    backend: B,
    limits: Limits,
    changes: Changes,
    tickets: Mutex<HashMap<[u8; 32], Ticket>>,
    rooms: Mutex<HashMap<String, RoomReady>>,
}

type RoomReady = tokio::sync::watch::Receiver<Option<Result<Arc<Mutex<Room>>, Fault>>>;

/// One service instance. Each mode must have its own instance and ticket table.
pub struct Relay<B>(Arc<Inner<B>>);

impl<B> Clone for Relay<B> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// An admitted peer. Call `leave` after transport closure, including write errors.
pub struct Session {
    pub incoming: mpsc::Receiver<Result<Arc<[u8]>, Fault>>,
    pub ended: tokio::sync::watch::Receiver<Option<Fault>>,
    room: Arc<Mutex<Room>>,
    room_url: String,
    agent: Agent,
    id: u64,
}

impl<B: RoomStore + Authorizer> Relay<B> {
    pub fn new(backend: B, limits: Limits, changes: Changes) -> Self {
        Self(Arc::new(Inner {
            backend,
            limits,
            changes,
            tickets: Mutex::new(HashMap::new()),
            rooms: Mutex::new(HashMap::new()),
        }))
    }

    pub fn limits(&self) -> &Limits {
        &self.0.limits
    }

    /// The adapter supplies exactly 32 bytes from its cryptographic random source.
    pub async fn ticket(&self, secret: [u8; 32], room: &str, agent: Agent) -> Result<(), Fault> {
        let access = self.0.backend.access(room, &agent).await?;
        if !access.read {
            return Err(Fault::Forbidden);
        }
        let mut tickets = self.0.tickets.lock().await;
        tickets.retain(|_, t| t.expires > Instant::now());
        if tickets.len() >= self.0.limits.max_tickets
            || tickets
                .values()
                .filter(|t| t.agent.subject == agent.subject)
                .count()
                >= self.0.limits.tickets_per_agent
        {
            return Err(Fault::Capacity);
        }
        tickets.insert(
            secret,
            Ticket {
                room: room.into(),
                agent,
                write: access.write,
                expires: Instant::now() + Duration::from_secs(60),
            },
        );
        Ok(())
    }

    pub async fn join(&self, secret: [u8; 32], room_url: &str) -> Result<Session, Fault> {
        // Removal is atomic even on mismatched or expired use.
        let ticket = self
            .0
            .tickets
            .lock()
            .await
            .remove(&secret)
            .ok_or(Fault::Ticket)?;
        if ticket.room != room_url || ticket.expires <= Instant::now() {
            return Err(Fault::Ticket);
        }
        let access = self.0.backend.access(room_url, &ticket.agent).await?;
        if !access.read {
            return Err(Fault::Forbidden);
        }
        let mut ready = {
            let mut rooms = self.0.rooms.lock().await;
            if let Some(ready) = rooms.get(room_url) {
                ready.clone()
            } else {
                if rooms.len() >= self.0.limits.max_rooms {
                    return Err(Fault::Capacity);
                }
                let (tx, ready) = tokio::sync::watch::channel(None);
                rooms.insert(room_url.into(), ready.clone());
                let relay = self.clone();
                let name = room_url.to_string();
                // Loading owns its reservation independently of any joining socket's deadline.
                tokio::spawn(async move {
                    let result =
                        tokio::time::timeout(Duration::from_secs(10), relay.open_room(&name))
                            .await
                            .unwrap_or(Err(Fault::Store));
                    let failed = result.is_err();
                    let _ = tx.send(Some(result));
                    if failed {
                        relay.0.rooms.lock().await.remove(&name);
                    }
                });
                ready
            }
        };
        let room = ready
            .wait_for(|r| r.is_some())
            .await
            .map_err(|_| Fault::Store)?
            .as_ref()
            .unwrap()
            .clone()?;
        let mut guard = room.lock().await;
        if guard.unloading {
            return Err(Fault::Capacity);
        }
        if guard.needs_reload {
            return Err(Fault::Store);
        }
        if guard.peers.len() >= self.0.limits.peers_per_room {
            return Err(Fault::Capacity);
        }
        guard.sequence += 1;
        let id = guard.sequence;
        let (tx, incoming) = mpsc::channel(16);
        let (end, ended) = tokio::sync::watch::channel(None);
        for frame in guard.doc.start() {
            let _ = tx.try_send(Ok(frame.into()));
        }
        if ticket.write && access.write {
            guard.last_writer = Some(ticket.agent.clone());
        }
        guard.peers.insert(
            id,
            Peer {
                write: ticket.write && access.write,
                tx,
                end,
            },
        );
        drop(guard);
        Ok(Session {
            room,
            room_url: room_url.into(),
            agent: ticket.agent,
            id,
            incoming,
            ended,
        })
    }

    async fn open_room(&self, name: &str) -> Result<Arc<Mutex<Room>>, Fault> {
        // Subscribe before listing so an overlapping committed append cannot be missed.
        let changes = self.0.changes.subscribe(name);
        let (files, state, newest_ms, update_files) = self.read_room(name).await?;
        let room = Arc::new(Mutex::new(Room {
            doc: Document::new(state)?,
            peers: HashMap::new(),
            pending: HashMap::new(),
            flushing: None,
            files,
            update_files,
            last_writer: None,
            last_left: Instant::now(),
            sequence: 0,
            newest_ms,
            needs_reload: false,
            unloading: false,
        }));
        self.spawn(name.to_string(), room.clone(), changes);
        Ok(room)
    }

    async fn read_room(&self, room: &str) -> Result<(Vec<String>, Vec<u8>, u64, usize), Fault> {
        let files = self.0.backend.list(room).await?;
        if files.len() > self.0.limits.max_log_files {
            return Err(Fault::Capacity);
        }
        let mut decoded = Vec::new();
        let mut logs = Vec::new();
        let mut bytes = 0usize;
        let mut newest = 0;
        let mut update_files = 0;
        for name in files {
            let body = self.0.backend.read(&name).await?;
            if !body.starts_with(b"JYU1") && !body.starts_with(b"JYS1") {
                continue;
            }
            bytes = bytes.saturating_add(body.len());
            if body.len() > self.0.limits.document_bytes.saturating_add(16)
                || bytes > self.0.limits.document_bytes.saturating_mul(4)
            {
                return Err(Fault::TooLarge);
            }
            let file = LogFile::decode(&body)?;
            update_files += usize::from(file.kind == LogKind::Update);
            newest = newest.max(file.millis);
            logs.push(name);
            decoded.push(file);
        }
        Ok((
            logs,
            load(decoded, self.0.limits.document_bytes)?,
            newest,
            update_files,
        ))
    }

    /// Handle all messages in one binary frame under the room's serialization lock.
    pub async fn handle(&self, session: &Session, bytes: &[u8]) -> Result<(), Fault> {
        if bytes.len() > self.0.limits.message_bytes {
            return Err(Fault::TooLarge);
        }
        let messages = messages(bytes, self.0.limits.frame_messages)?;
        let mut room = session.room.lock().await;
        let mut response_bytes = 0usize;
        for message in messages {
            let peer = room.peers.get(&session.id).ok_or(Fault::Forbidden)?;
            let writable = peer.write;
            match message {
                Message::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) => {
                    let Some((state, doc)) =
                        room.doc
                            .candidate(&update, writable, self.0.limits.document_bytes)?
                    else {
                        continue;
                    };
                    let existing = room.pending.get(&session.agent);
                    let split = room.flushing.as_ref().is_some_and(|(agent, revision)| {
                        agent == &session.agent && existing.is_some_and(|p| p.revision == *revision)
                    });
                    let reserved = room.flushing.as_ref().is_some_and(|(agent, revision)| {
                        room.pending
                            .get(agent)
                            .is_some_and(|p| p.revision != *revision)
                    });
                    if (existing.is_none() || split)
                        && room
                            .files
                            .len()
                            .saturating_add(room.pending.len())
                            .saturating_add(usize::from(reserved))
                            >= self.0.limits.max_log_files
                    {
                        return Err(Fault::Capacity);
                    }
                    if existing.is_none() && room.pending.len() >= self.0.limits.peers_per_room {
                        return Err(Fault::Capacity);
                    }
                    let pending = merge_updates_v1([
                        existing.map(|p| p.update.as_slice()).unwrap_or(&[0, 0]),
                        &update,
                    ])
                    .map_err(|_| Fault::Malformed)?;
                    let total: usize = room.pending.values().map(|p| p.update.len()).sum();
                    if total
                        .saturating_sub(existing.map_or(0, |p| p.update.len()))
                        .saturating_add(pending.len())
                        > self.0.limits.document_bytes
                    {
                        return Err(Fault::Capacity);
                    }
                    {
                        let (retry, delay) = existing
                            .map(|p| (p.retry, p.delay))
                            .unwrap_or((Instant::now(), Duration::from_millis(500)));
                        let random = nonce()?;
                        room.doc.replace(state, doc);
                        room.sequence += 1;
                        let revision = room.sequence;
                        let millis = millis().max(room.newest_ms);
                        let file = LogFile {
                            kind: LogKind::Update,
                            millis,
                            nonce: random,
                            update: pending.clone(),
                        };
                        room.pending.insert(
                            session.agent.clone(),
                            Pending {
                                update: pending,
                                revision,
                                file,
                                retry,
                                delay,
                            },
                        );
                        room.last_writer = Some(session.agent.clone());
                        room.broadcast(
                            session.id,
                            Message::Sync(SyncMessage::Update(update)).encode_v1(),
                        );
                    }
                }
                Message::Awareness(update) => {
                    let frame = room.doc.presence(
                        session.id,
                        update,
                        self.0.limits.awareness_clients,
                        self.0.limits.message_bytes,
                    )?;
                    room.broadcast(session.id, frame);
                }
                other => {
                    let reply = room.doc.reply(other)?;
                    response_bytes = response_bytes.saturating_add(reply.len());
                    if response_bytes
                        > self
                            .0
                            .limits
                            .document_bytes
                            .saturating_add(self.0.limits.message_bytes)
                    {
                        return Err(Fault::Capacity);
                    }
                    if !reply.is_empty()
                        && room.peers[&session.id]
                            .tx
                            .try_send(Ok(reply.into()))
                            .is_err()
                    {
                        return Err(Fault::Capacity);
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn recheck(&self, session: &Session) -> Result<(), Fault> {
        let access = self
            .0
            .backend
            .access(&session.room_url, &session.agent)
            .await?;
        if !access.read {
            return Err(Fault::Forbidden);
        }
        if let Some(peer) = session.room.lock().await.peers.get_mut(&session.id) {
            // A ticket never upgrades; a revoked writer stays an observer for this connection.
            peer.write &= access.write;
        }
        Ok(())
    }

    pub async fn leave(&self, session: &Session) {
        let mut room = session.room.lock().await;
        room.peers.remove(&session.id);
        let removal = room.doc.leave(session.id);
        room.broadcast(session.id, removal);
        if room.peers.is_empty() {
            room.last_left = Instant::now();
        }
    }

    fn spawn(
        &self,
        name: String,
        room: Arc<Mutex<Room>>,
        mut changes: broadcast::Receiver<String>,
    ) {
        let weak = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut retry = Instant::now();
            let mut backoff = Duration::from_millis(500);
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    change = changes.recv() => {
                        let Some(inner) = weak.upgrade() else { break; };
                        let relay = Relay(inner);
                        let result = match change {
                            Ok(file) => relay.ingest(&name, &room, &file).await,
                            Err(broadcast::error::RecvError::Lagged(_)) => relay.reload(&name, &room).await,
                            Err(broadcast::error::RecvError::Closed) => break,
                        };
                        if let Err(error) = result {
                            let mut guard = room.lock().await;
                            guard.needs_reload = true;
                            for peer in guard.peers.values() { let _ = peer.end.send(Some(error)); }
                        }
                        continue;
                    }
                }
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let relay = Relay(inner);
                if Instant::now() >= retry {
                    let needs_reload = room.lock().await.needs_reload;
                    let result = if needs_reload {
                        relay.reload(&name, &room).await
                    } else {
                        relay.flush(&name, &room).await;
                        Ok(())
                    };
                    match result {
                        Ok(()) => {
                            backoff = Duration::from_millis(500);
                            retry = Instant::now();
                        }
                        Err(_) => {
                            retry = Instant::now() + backoff;
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                        }
                    }
                }
                let compact = {
                    let guard = room.lock().await;
                    guard.peers.is_empty()
                        && guard.pending.is_empty()
                        && guard.last_left.elapsed() >= Duration::from_secs(60)
                        && guard.update_files >= relay.0.limits.compact_after
                };
                // Drain committed log appends before selecting the snapshot's folded set.
                if compact && relay.reload(&name, &room).await.is_err() {
                    continue;
                }
                // Retire this room without holding the service-wide admission lock over I/O.
                let mut guard = room.lock().await;
                if guard.peers.is_empty()
                    && guard.pending.is_empty()
                    && !guard.needs_reload
                    && guard.last_left.elapsed() >= Duration::from_secs(60)
                {
                    guard.unloading = true;
                    let snapshot = if guard.update_files >= relay.0.limits.compact_after {
                        guard
                            .last_writer
                            .clone()
                            .zip(nonce().ok())
                            .map(|(agent, random)| {
                                (
                                    agent,
                                    LogFile {
                                        kind: LogKind::Snapshot,
                                        millis: guard.newest_ms,
                                        nonce: random,
                                        update: guard.doc.state.clone(),
                                    },
                                    guard.files.clone(),
                                )
                            })
                    } else {
                        None
                    };
                    drop(guard);
                    if let Some((agent, snapshot, folded)) = snapshot {
                        // Confirm the snapshot before any deletion; ordinary DELETE checks apply.
                        if relay
                            .0
                            .backend
                            .create(&name, &agent, &snapshot)
                            .await
                            .is_ok()
                        {
                            for file in folded {
                                let _ = relay.0.backend.delete(&name, &agent, &file).await;
                            }
                        }
                    }
                    relay.0.rooms.lock().await.remove(&name);
                    break;
                }
            }
        });
    }

    async fn flush(&self, name: &str, room: &Arc<Mutex<Room>>) {
        let authors: Vec<_> = room.lock().await.pending.keys().cloned().collect();
        for agent in authors {
            let batch = {
                let mut guard = room.lock().await;
                let Some(batch) = guard.pending.get(&agent) else {
                    continue;
                };
                if batch.retry > Instant::now() {
                    continue;
                }
                let mut batch = batch.clone();
                // Preserve replay ordering after a snapshot arrives while an update is queued.
                batch.file.millis = millis().max(batch.file.millis).max(guard.newest_ms);
                guard.flushing = Some((agent.clone(), batch.revision));
                batch
            };
            let created = self.0.backend.create(name, &agent, &batch.file).await;
            let mut guard = room.lock().await;
            guard.flushing = None;
            let file = match created {
                Ok(file) => file,
                Err(_) => {
                    if let Some(held) = guard.pending.get_mut(&agent) {
                        held.retry = Instant::now() + held.delay;
                        held.delay = (held.delay * 2).min(Duration::from_secs(30));
                    }
                    continue;
                }
            };
            guard.newest_ms = guard.newest_ms.max(batch.file.millis);
            if !guard.files.contains(&file) {
                guard.files.push(file);
                guard.update_files += 1;
            }
            if guard
                .pending
                .get(&agent)
                .is_some_and(|p| p.revision == batch.revision)
            {
                guard.pending.remove(&agent);
            }
        }
    }

    async fn ingest(&self, name: &str, room: &Arc<Mutex<Room>>, file: &str) -> Result<(), Fault> {
        if room.lock().await.files.iter().any(|f| f == file) {
            return Ok(());
        }
        let bytes = self.0.backend.read(file).await?;
        if !bytes.starts_with(b"JYU1") && !bytes.starts_with(b"JYS1") {
            return Ok(());
        }
        if bytes.len() > self.0.limits.document_bytes.saturating_add(16) {
            return Err(Fault::TooLarge);
        }
        let log = LogFile::decode(&bytes)?;
        let mut guard = room.lock().await;
        if guard.files.len() >= self.0.limits.max_log_files {
            return Err(Fault::Capacity);
        }
        if guard
            .doc
            .merge(&log.update, true, self.0.limits.document_bytes)?
        {
            guard.broadcast(
                0,
                Message::Sync(SyncMessage::Update(log.update)).encode_v1(),
            );
        }
        guard.newest_ms = guard.newest_ms.max(log.millis);
        guard.update_files += usize::from(log.kind == LogKind::Update);
        guard.files.push(file.into());
        let _ = name;
        Ok(())
    }

    async fn reload(&self, name: &str, room: &Arc<Mutex<Room>>) -> Result<(), Fault> {
        let (files, state, newest, update_files) = self.read_room(name).await?;
        let mut guard = room.lock().await;
        if guard
            .doc
            .merge(&state, true, self.0.limits.document_bytes)?
        {
            guard.broadcast(0, Message::Sync(SyncMessage::Update(state)).encode_v1());
        }
        guard.files = files;
        guard.update_files = update_files;
        guard.needs_reload = false;
        guard.newest_ms = guard.newest_ms.max(newest);
        Ok(())
    }
}

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn nonce() -> Result<u32, Fault> {
    let mut bytes = [0; 4];
    getrandom::fill(&mut bytes).map_err(|_| Fault::Store)?;
    Ok(u32::from_be_bytes(bytes))
}

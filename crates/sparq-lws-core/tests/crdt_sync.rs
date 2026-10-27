#![cfg(all(feature = "crdt-sync", not(target_arch = "wasm32")))]

mod common;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use solid_oidc_verifier::{
    config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
};
use sparq_lws_core::{
    app::{build_router, AppState},
    auth::AuthContext,
    error::ServerResult,
    ldp::handler::LdpState,
    lws::{self, LwsConfig, LWS_CONTEXT, LWS_JSON},
    store::{
        CompositeStore, DeleteOutcome, InMemoryBlobStore, InMemorySparqClient, Resource,
        ResourceMeta, Store, ValidatedChildIri,
    },
};
use sparq_lws_ysync::{yrs, Limits};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message as Ws, MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;
use yrs::sync::{Awareness, Message, SyncMessage};
use yrs::updates::{decoder::Decode, encoder::Encode};
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

const OWNER: &str = common::WEBID;
const WRITER: &str = "https://bob.example/#me";
const READER: &str = "https://reader.example/#me";
const ROOM: &str = "https://pod.example/room/";
type Storage = lws::tracked::Tracked<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Default)]
struct StorageGate {
    listing: AtomicBool,
    snapshot: AtomicBool,
    flush: AtomicBool,
    pause_flush: AtomicBool,
    entered: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

struct GatedStore {
    inner: Storage,
    gate: Arc<StorageGate>,
}

#[async_trait::async_trait]
impl Store for GatedStore {
    fn exclusive(&self) -> ServerResult<()> {
        self.inner.exclusive()
    }
    fn raises_markers(&self) -> bool {
        self.inner.raises_markers()
    }
    async fn read_marker(&self, iri: &str) -> ServerResult<Option<String>> {
        self.inner.read_marker(iri).await
    }
    fn lifecycle(&self) -> Option<Arc<sparq_lws_core::store::Lifecycle>> {
        self.inner.lifecycle()
    }
    async fn raise_marker(&self, iri: &str, value: i64) -> ServerResult<i64> {
        self.inner.raise_marker(iri, value).await
    }
    async fn reset_marker(
        &self,
        iri: &str,
        value: i64,
        lease: &sparq_lws_core::store::LifecycleGuard,
    ) -> ServerResult<()> {
        self.inner.reset_marker(iri, value, lease).await
    }
    async fn read(&self, iri: &str) -> ServerResult<Resource> {
        self.inner.read(iri).await
    }
    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
        self.inner.meta(iri).await
    }
    async fn exists(&self, iri: &str) -> ServerResult<bool> {
        self.inner.exists(iri).await
    }
    async fn write(&self, iri: &str, body: Bytes, media: &str) -> ServerResult<ResourceMeta> {
        self.inner.write(iri, body, media).await
    }
    async fn create_in_container(
        &self,
        room: &str,
        child: &str,
        body: Bytes,
        media: &str,
    ) -> ServerResult<ResourceMeta> {
        if body.starts_with(b"JYU1") && self.gate.flush.load(Ordering::SeqCst) {
            return Err(sparq_lws_core::error::ServerError::Storage(
                "fixture failure".into(),
            ));
        }
        if body.starts_with(b"JYU1") && self.gate.pause_flush.load(Ordering::SeqCst) {
            self.gate.entered.notify_one();
            self.gate.released.notified().await;
        }
        if body.starts_with(b"JYS1") && self.gate.snapshot.load(Ordering::SeqCst) {
            self.gate.entered.notify_one();
            self.gate.released.notified().await;
        }
        self.inner
            .create_in_container(room, child, body, media)
            .await
    }
    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
        self.inner.delete(iri, parent).await
    }
    async fn delete_container_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> ServerResult<DeleteOutcome> {
        self.inner.delete_container_if_empty(iri, parent).await
    }
    async fn list_children(&self, iri: &str) -> ServerResult<Vec<ValidatedChildIri>> {
        self.inner.list_children(iri).await
    }
    async fn list_children_within(
        &self,
        iri: &str,
        bytes: usize,
    ) -> ServerResult<Option<Vec<ValidatedChildIri>>> {
        if iri == ROOM && self.gate.listing.load(Ordering::SeqCst) {
            self.gate.entered.notify_one();
            self.gate.released.notified().await;
        }
        self.inner.list_children_within(iri, bytes).await
    }
    async fn holds(&self) -> ServerResult<Vec<(String, ResourceMeta)>> {
        self.inner.holds().await
    }
    async fn hold(&self, iri: &str, holder: &str) -> ServerResult<Option<ResourceMeta>> {
        self.inner.hold(iri, holder).await
    }
    async fn reinstate(
        &self,
        iri: &str,
        container: Option<&str>,
        held: &ResourceMeta,
    ) -> ServerResult<ResourceMeta> {
        self.inner.reinstate(iri, container, held).await
    }
    async fn release(&self, held: &ResourceMeta, holder: &str) -> ServerResult<()> {
        self.inner.release(held, holder).await
    }
    async fn finish_reclaims(&self) -> ServerResult<()> {
        self.inner.finish_reclaims().await
    }
}

struct Harness {
    app: axum::Router,
    store: Storage,
    lws: Option<LwsConfig>,
    issuer: common::KeyKit,
    client: common::KeyKit,
    addr: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Harness {
    async fn gated(mode: bool, limits: Limits) -> (Self, Arc<StorageGate>) {
        let mut h = Self::new(mode, limits.clone()).await;
        h.server.abort();
        let _ = (&mut h.server).await;
        h.app = axum::Router::new();
        let gate = Arc::new(StorageGate::default());
        let store = GatedStore {
            inner: h.store.clone(),
            gate: gate.clone(),
        };
        let app = if let Some(cfg) = &h.lws {
            lws::router(store, cfg.clone()).await.unwrap()
        } else {
            let config = VerifierConfig::new(vec![common::ISSUER.into()], common::BASE_URL);
            let replay = InMemoryReplayStore::with_window(config.replay_ttl());
            let verifier = Verifier::new(config, common::jwks_provider(&h.issuer), replay).unwrap();
            let mut ldp = LdpState::new(store, common::BASE_URL);
            ldp.crdt_limits = limits;
            build_router(AppState::new(
                AuthContext::new(verifier, common::BASE_URL),
                ldp,
            ))
        };
        h.server.abort();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        h.addr = listener.local_addr().unwrap();
        h.app = app.clone();
        h.server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (h, gate)
    }

    async fn new(lws_mode: bool, limits: Limits) -> Self {
        Self::with_store(
            lws_mode,
            limits,
            lws::tracked::Tracked::new(
                CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new()),
                "https://pod.example/".into(),
            ),
        )
        .await
    }

    async fn with_store(lws_mode: bool, limits: Limits, store: Storage) -> Self {
        store
            .write(ROOM, Bytes::new(), "text/turtle")
            .await
            .unwrap();
        let issuer = common::KeyKit::generate();
        let client = common::KeyKit::generate();
        let (app, cfg) = if lws_mode {
            let mut cfg = LwsConfig::new(common::BASE_URL);
            cfg.owner = Some(OWNER.into());
            cfg.crdt_limits = limits;
            (
                lws::router(store.clone(), cfg.clone()).await.unwrap(),
                Some(cfg),
            )
        } else {
            let acl = format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#writers> a acl:Authorization; acl:agent <{OWNER}>, <{WRITER}>;
 acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
 acl:mode acl:Read, acl:Write, acl:Control.
<#reader> a acl:Authorization; acl:agent <{READER}>;
 acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>; acl:mode acl:Read."#
            );
            store
                .write("https://pod.example/.acl", Bytes::from(acl), "text/turtle")
                .await
                .unwrap();
            let config = VerifierConfig::new(vec![common::ISSUER.into()], common::BASE_URL);
            let replay = InMemoryReplayStore::with_window(config.replay_ttl());
            let verifier = Verifier::new(config, common::jwks_provider(&issuer), replay).unwrap();
            let auth = AuthContext::new(verifier, common::BASE_URL);
            let mut ldp = LdpState::new(store.clone(), common::BASE_URL);
            ldp.crdt_limits = limits;
            (build_router(AppState::new(auth, ldp)), None)
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let service = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, service).await.unwrap();
        });
        let h = Self {
            app,
            store,
            lws: cfg,
            issuer,
            client,
            addr,
            server,
        };
        if lws_mode {
            h.grant(WRITER, &["read", "create", "modify", "delete"])
                .await;
            h.grant(READER, &["read"]).await;
        }
        h
    }

    fn tickets(&self) -> &'static str {
        if self.lws.is_some() {
            "/.lws/crdt/tickets"
        } else {
            "/.crdt/tickets"
        }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        subject: &str,
        media: &str,
        body: impl Into<Body>,
    ) -> axum::response::Response {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, media);
        if let Some(cfg) = &self.lws {
            let token = lws::tokens::mint(cfg, subject, "https://client.example/id", &cfg.realm());
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        } else {
            let access =
                common::mint_access_token_for(&self.issuer, &self.client.thumbprint, subject);
            let proof = common::mint_dpop_proof(
                &self.client,
                method,
                &format!("{}{path}", common::BASE_URL),
                &access,
            );
            req = req
                .header(header::AUTHORIZATION, format!("DPoP {access}"))
                .header("dpop", proof);
        }
        self.app
            .clone()
            .oneshot(req.body(body.into()).unwrap())
            .await
            .unwrap()
    }

    async fn grant(&self, subject: &str, actions: &[&str]) -> String {
        let body = json!({"@context": [LWS_CONTEXT], "type": "AccessGrant", "storage": "https://pod.example/",
            "access": [{"type": ["AccessPolicy"], "assignee": subject, "action": actions}]});
        let r = self
            .call("POST", lws::GRANTS_PATH, OWNER, LWS_JSON, body.to_string())
            .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        r.headers()[header::LOCATION].to_str().unwrap().to_string()
    }

    async fn ticket(&self, subject: &str) -> String {
        let r = self
            .call(
                "POST",
                self.tickets(),
                subject,
                "application/json",
                json!({"room": ROOM}).to_string(),
            )
            .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        let body: Value =
            serde_json::from_slice(&to_bytes(r.into_body(), 8192).await.unwrap()).unwrap();
        assert!(body["expires"].as_str().unwrap().ends_with('Z'));
        body["url"].as_str().unwrap().to_string()
    }

    async fn socket(&self, url: &str) -> Socket {
        let url = url::Url::parse(url).unwrap();
        tokio_tungstenite::connect_async(format!(
            "ws://{}{}?{}",
            self.addr,
            url.path(),
            url.query().unwrap()
        ))
        .await
        .unwrap()
        .0
    }

    async fn client(&self, subject: &str, id: u64) -> Client {
        let mut c = Client {
            ws: self.socket(&self.ticket(subject).await).await,
            doc: Doc::with_client_id(id),
            awareness: Awareness::new(Doc::new()),
        };
        c.send(Message::Sync(
            SyncMessage::SyncStep1(StateVector::default()),
        ))
        .await;
        loop {
            if matches!(c.receive().await, Message::Sync(SyncMessage::SyncStep2(_))) {
                break;
            }
        }
        c
    }

    async fn file_state(&self) -> Doc {
        let names = self.store.list_children(ROOM).await.unwrap();
        let mut records = Vec::new();
        for name in names {
            if name.as_str().ends_with(".meta") {
                continue;
            }
            let path = url::Url::parse(name.as_str()).unwrap().path().to_string();
            let response = self
                .call(
                    "GET",
                    &path,
                    OWNER,
                    "application/octet-stream",
                    Body::empty(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            // Independent envelope decoder; neither the relay loader nor LogFile is used.
            assert!(bytes.len() >= 16);
            let snapshot = match &bytes[..4] {
                b"JYU1" => false,
                b"JYS1" => true,
                _ => panic!("unexpected log marker"),
            };
            let ms = u64::from_be_bytes(bytes[4..12].try_into().unwrap());
            let nonce = u32::from_be_bytes(bytes[12..16].try_into().unwrap());
            records.push((ms, nonce, snapshot, bytes[16..].to_vec()));
        }
        records.sort_by_key(|f| (f.0, f.1));
        let latest = records.iter().rposition(|f| f.2);
        let doc = Doc::new();
        let mut txn = doc.transact_mut();
        if let Some(i) = latest {
            txn.apply_update(Update::decode_v1(&records[i].3).unwrap())
                .unwrap();
        }
        for record in records
            .iter()
            .filter(|f| !f.2 && f.0 >= latest.map(|i| records[i].0).unwrap_or(0))
        {
            txn.apply_update(Update::decode_v1(&record.3).unwrap())
                .unwrap();
        }
        drop(txn);
        doc
    }
}

struct Client {
    ws: Socket,
    doc: Doc,
    awareness: Awareness,
}
impl Client {
    async fn send(&mut self, message: Message) {
        self.ws
            .send(Ws::Binary(message.encode_v1().into()))
            .await
            .unwrap();
    }
    async fn receive(&mut self) -> Message {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(4), self.ws.next())
                .await
                .expect("socket deadline")
                .unwrap()
                .unwrap();
            match frame {
                Ws::Binary(bytes) => {
                    let m = Message::decode_v1(&bytes).unwrap();
                    match &m {
                        Message::Sync(SyncMessage::SyncStep1(sv)) => {
                            let update = self.doc.transact().encode_state_as_update_v1(sv);
                            self.send(Message::Sync(SyncMessage::SyncStep2(update)))
                                .await;
                        }
                        Message::Sync(SyncMessage::SyncStep2(u) | SyncMessage::Update(u)) => {
                            self.doc
                                .transact_mut()
                                .apply_update(Update::decode_v1(u).unwrap())
                                .unwrap();
                        }
                        Message::Awareness(update) => {
                            self.awareness.apply_update(update.clone()).unwrap();
                        }
                        _ => {}
                    }
                    return m;
                }
                Ws::Ping(p) => self.ws.send(Ws::Pong(p)).await.unwrap(),
                other => panic!("unexpected socket frame {other:?}"),
            }
        }
    }
    fn text(&self) -> String {
        self.doc
            .get_or_insert_text("text")
            .get_string(&self.doc.transact())
    }
    fn vector(&self) -> StateVector {
        self.doc.transact().state_vector()
    }
    async fn edit(&mut self, text: &str) {
        let txt = self.doc.get_or_insert_text("text");
        txt.push(&mut self.doc.transact_mut(), text);
        let update = self
            .doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        self.send(Message::Sync(SyncMessage::Update(update))).await;
    }
    async fn until_text(&mut self, expected: &str) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while self.text() != expected {
                self.receive().await;
            }
        })
        .await
        .expect("content must converge");
    }
    async fn wait_presence(&mut self, id: u64, present: bool) {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if self
                    .awareness
                    .state::<Value>(yrs::ClientID::new(id))
                    .is_some()
                    == present
                {
                    break;
                }
                self.receive().await;
            }
        })
        .await
        .expect("awareness client must observe the announced presence state");
    }

    async fn finish_close(&mut self) {
        self.ws.close(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Ws::Close(frame) = self.ws.next().await.unwrap().unwrap() {
                    assert!(frame.is_none_or(|c| u16::from(c.code) == 1000));
                    break;
                }
            }
        })
        .await
        .expect("normal closure must complete normally");
    }

    async fn close_code(&mut self) -> u16 {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(4), self.ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match frame {
                Ws::Close(Some(close)) => return close.code.into(),
                Ws::Ping(p) => self.ws.send(Ws::Pong(p)).await.unwrap(),
                _ => {}
            }
        }
    }
}

async fn sq5(lws: bool) {
    let h = Harness::new(lws, Limits::default()).await;
    let mut a = h.client(OWNER, 1).await;
    let mut b = h.client(WRITER, 2).await;
    a.edit("a").await;
    b.edit("b").await;
    tokio::time::timeout(Duration::from_secs(4), async {
        while a.vector() != b.vector() || a.text() != b.text() {
            tokio::join!(a.receive(), b.receive());
        }
    })
    .await
    .expect("concurrent edits converge");
    assert_eq!(a.text().len(), 2);
    let expected = a.text();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == a.vector() {
                assert_eq!(
                    doc.get_or_insert_text("text").get_string(&doc.transact()),
                    expected
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("file-only replica converges");
    // Each writer's flush must have its own creator, including the Solid metadata adapter.
    let mut authors = Vec::new();
    for child in h.store.list_children(ROOM).await.unwrap() {
        if child.as_str().ends_with(".meta") {
            continue;
        }
        let meta = h.store.read(&lws::meta_key(child.as_str())).await.unwrap();
        let meta: Value = serde_json::from_slice(&meta.body).unwrap();
        authors.push(meta["creator"].as_str().unwrap().to_string());
    }
    assert!(authors.contains(&OWNER.to_string()));
    assert!(authors.contains(&WRITER.to_string()));
    let mut viewer = h.client(READER, 3).await;
    viewer.until_text(&expected).await;
    let unchanged = viewer
        .doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    viewer
        .send(Message::Sync(SyncMessage::SyncStep2(unchanged)))
        .await;
    let mut presence = Awareness::new(Doc::with_client_id(33));
    presence.set_local_state(json!({"name": "viewer"})).unwrap();
    viewer
        .send(Message::Awareness(presence.update().unwrap()))
        .await;
    a.wait_presence(33, true).await;
    viewer.ws.close(None).await.unwrap();
    a.wait_presence(33, false).await;
    let mut viewer = h.client(READER, 4).await;
    viewer.edit("forbidden").await;
    assert_eq!(viewer.close_code().await, 4403);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let files = h.file_state().await;
    assert_eq!(files.transact().state_vector(), a.vector());
    assert_eq!(
        files
            .get_or_insert_text("text")
            .get_string(&files.transact()),
        expected
    );
    let mut step2 = h.client(READER, 6).await;
    step2
        .doc
        .get_or_insert_text("text")
        .push(&mut step2.doc.transact_mut(), "also refused");
    let update = step2
        .doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    step2
        .send(Message::Sync(SyncMessage::SyncStep2(update)))
        .await;
    assert_eq!(step2.close_code().await, 4403);
    let mut reconnect = h.client(OWNER, 5).await;
    reconnect.until_text(&expected).await;
}

#[tokio::test]
async fn sq5_solid_live_and_file_replicas() {
    sq5(false).await;
}
#[tokio::test]
async fn sq5_lws_live_and_file_replicas() {
    sq5(true).await;
}

async fn ticket_guards(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            tickets_per_agent: 1,
            ..Default::default()
        },
    )
    .await;
    let ticket = h.ticket(OWNER).await;
    let over = h
        .call(
            "POST",
            h.tickets(),
            OWNER,
            "application/json",
            json!({"room": ROOM}).to_string(),
        )
        .await;
    assert_eq!(over.status(), StatusCode::TOO_MANY_REQUESTS);
    let mut c = Client {
        ws: h.socket(&ticket).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    c.receive().await;
    let mut reused = Client {
        ws: h.socket(&ticket).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    assert_eq!(reused.close_code().await, 4401);
    let wrong = h.ticket(OWNER).await.replace("room%2F", "other%2F");
    assert_ne!(wrong, ticket);
    let mut wrong = Client {
        ws: h.socket(&wrong).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    assert_eq!(wrong.close_code().await, 4401);
    let expired = h.ticket(OWNER).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    let mut expired = Client {
        ws: h.socket(&expired).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    assert_eq!(expired.close_code().await, 4401);
    let denied = h
        .call(
            "POST",
            h.tickets(),
            "https://stranger.example/#me",
            "application/json",
            json!({"room": ROOM}).to_string(),
        )
        .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    for room in [
        "https://elsewhere.example/room/",
        "https://pod.example/missing/",
        "https://pod.example/plain",
    ] {
        let r = h
            .call(
                "POST",
                h.tickets(),
                OWNER,
                "application/json",
                json!({"room": room}).to_string(),
            )
            .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn tickets_are_bounded_single_use_and_room_bound_solid() {
    ticket_guards(false).await;
}
#[tokio::test]
async fn tickets_are_bounded_single_use_and_room_bound_lws() {
    ticket_guards(true).await;
}

fn envelope(marker: &[u8; 4], ms: u64, nonce: u32, update: &[u8]) -> Vec<u8> {
    let mut out = marker.to_vec();
    out.extend_from_slice(&ms.to_be_bytes());
    out.extend_from_slice(&nonce.to_be_bytes());
    out.extend_from_slice(update);
    out
}

async fn logs_and_restart(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            compact_after: 1,
            ..Default::default()
        },
    )
    .await;
    let base = Doc::with_client_id(100);
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), "first");
    let old = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), "-latest");
    let latest = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for (name, ms, nonce, update) in [
        ("000-new.ysnap", 200, 2, &latest),
        ("mmm-earlier.ysnap", 100, 3, &old),
        ("zzz-old.ysnap", 200, 1, &old),
    ] {
        h.store
            .create_in_container(
                ROOM,
                &format!("{ROOM}{name}"),
                Bytes::from(envelope(b"JYS1", ms, nonce, update)),
                "application/octet-stream",
            )
            .await
            .unwrap();
    }
    let mut client = h.client(OWNER, 101).await;
    client.until_text("first-latest").await;
    base.get_or_insert_text("text")
        .push(&mut base.transact_mut(), "-log");
    let update = base
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let response = h
        .call(
            "POST",
            "/room/",
            OWNER,
            "application/octet-stream",
            envelope(b"JYU1", 300, 2, &update),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    client.until_text("first-latest-log").await;
    client.edit("-live").await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let expected = client.text();
    let vector = client.vector();
    client.ws.close(None).await.unwrap();
    drop(client);
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Advance the idle deadline without waiting for a wall-clock minute.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let children = h.store.list_children(ROOM).await.unwrap();
    let logs: Vec<_> = children
        .iter()
        .filter(|c| !c.as_str().ends_with(".meta"))
        .collect();
    assert_eq!(logs.len(), 1, "unload compacts folded log resources");
    assert!(h
        .store
        .read(logs[0].as_str())
        .await
        .unwrap()
        .body
        .starts_with(b"JYS1"));
    let file = h.file_state().await;
    assert_eq!(file.transact().state_vector(), vector);
    assert_eq!(
        file.get_or_insert_text("text").get_string(&file.transact()),
        expected
    );
    let store = h.store.clone();
    drop(h);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let restart = Harness::with_store(mode, Limits::default(), store).await;
    let mut rejoined = restart.client(OWNER, 102).await;
    rejoined.until_text(&expected).await;
    assert_eq!(rejoined.vector(), vector);
}

#[tokio::test]
async fn external_logs_header_order_compaction_restart_solid() {
    logs_and_restart(false).await;
}
#[tokio::test]
async fn external_logs_header_order_compaction_restart_lws() {
    logs_and_restart(true).await;
}

async fn bounds(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            peers_per_room: 1,
            max_rooms: 1,
            message_bytes: 256,
            document_bytes: 1024,
            ..Default::default()
        },
    )
    .await;
    let mut a = h.client(OWNER, 10).await;
    let mut excess = Client {
        ws: h.socket(&h.ticket(WRITER).await).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    assert_eq!(excess.close_code().await, 4429);
    h.store
        .write("https://pod.example/other/", Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let response = h
        .call(
            "POST",
            h.tickets(),
            OWNER,
            "application/json",
            json!({"room": "https://pod.example/other/"}).to_string(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let ticket: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let mut room_bound = Client {
        ws: h.socket(ticket["url"].as_str().unwrap()).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    assert_eq!(room_bound.close_code().await, 4429);
    a.ws.send(Ws::Binary(vec![0u8; 300].into())).await.unwrap();
    assert_eq!(a.close_code().await, 4413);
}

#[tokio::test]
async fn socket_bounds_solid() {
    bounds(false).await;
}
#[tokio::test]
async fn socket_bounds_lws() {
    bounds(true).await;
}

async fn revocation(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    let mut reader = h.client(READER, 30).await;
    restrict_writer(&h, true, false).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    assert_eq!(reader.close_code().await, 4403);
}

#[tokio::test]
async fn revoked_reader_closes_solid() {
    revocation(false).await;
}
#[tokio::test]
async fn revoked_reader_closes_lws() {
    revocation(true).await;
}

async fn revoke_grants(h: &Harness, keep: &str) {
    let response = h
        .call("GET", lws::GRANTS_PATH, OWNER, LWS_JSON, Body::empty())
        .await;
    let listing: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    for item in listing["items"].as_array().unwrap() {
        let uri = item.as_str().or_else(|| item["id"].as_str()).unwrap();
        if uri == keep {
            continue;
        }
        let path = url::Url::parse(uri).unwrap().path().to_string();
        assert!(h
            .call("DELETE", &path, OWNER, LWS_JSON, Body::empty())
            .await
            .status()
            .is_success());
    }
}

async fn restrict_writer(h: &Harness, read: bool, write: bool) {
    if h.lws.is_some() {
        let actions: Vec<_> = [(read, "read"), (write, "create")]
            .into_iter()
            .filter_map(|(yes, a)| yes.then_some(a))
            .collect();
        let keep = h.grant(WRITER, &actions).await;
        revoke_grants(h, &keep).await;
    } else {
        let modes: Vec<_> = [(read, "acl:Read"), (write, "acl:Append")]
            .into_iter()
            .filter_map(|(yes, m)| yes.then_some(m))
            .collect();
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{OWNER}>;
 acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
 acl:mode acl:Read, acl:Write, acl:Control.
<#writer> a acl:Authorization; acl:agent <{WRITER}>;
 acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>; acl:mode {}."#,
            modes.join(", ")
        );
        assert!(h
            .call("PUT", "/.acl", OWNER, "text/turtle", acl)
            .await
            .status()
            .is_success());
    }
}

async fn persist_authorization(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    let mut owner = h.client(OWNER, 200).await;
    let mut writer = h.client(WRITER, 201).await;
    writer.edit("queued").await;
    owner.until_text("queued").await;
    // Withdraw read while retaining member creation before the scheduled flush.
    tokio::time::pause();
    restrict_writer(&h, false, true).await;
    tokio::time::advance(Duration::from_millis(600)).await;
    tokio::time::resume();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        h.store.list_children(ROOM).await.unwrap().is_empty(),
        "a flush rechecks access before creating a file"
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    assert_eq!(writer.close_code().await, 4403);
}

#[tokio::test]
async fn flush_checks_current_access_solid() {
    persist_authorization(false).await;
}
#[tokio::test]
async fn flush_checks_current_access_lws() {
    persist_authorization(true).await;
}

async fn downgrade(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    let mut writer = h.client(WRITER, 301).await;
    restrict_writer(&h, true, false).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    // A sync request still works after write is withdrawn.
    writer
        .send(Message::Sync(
            SyncMessage::SyncStep1(StateVector::default()),
        ))
        .await;
    writer.receive().await;
    writer.edit("refused after downgrade").await;
    assert_eq!(writer.close_code().await, 4403);
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
}

#[tokio::test]
async fn lost_write_downgrades_solid() {
    downgrade(false).await;
}
#[tokio::test]
async fn lost_write_downgrades_lws() {
    downgrade(true).await;
}

async fn document_bound(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            document_bytes: 128,
            message_bytes: 4096,
            ..Default::default()
        },
    )
    .await;
    let mut client = h.client(OWNER, 700).await;
    client.edit(&"x".repeat(256)).await;
    assert_eq!(client.close_code().await, 4413);
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
}

#[tokio::test]
async fn document_bound_solid() {
    document_bound(false).await;
}
#[tokio::test]
async fn document_bound_lws() {
    document_bound(true).await;
}

async fn awareness_bound(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            awareness_clients: 1,
            ..Default::default()
        },
    )
    .await;
    let mut client = h.client(OWNER, 800).await;
    let mut first = Awareness::new(Doc::with_client_id(801));
    first.set_local_state(json!({"name":"first"})).unwrap();
    let mut second = Awareness::new(Doc::with_client_id(802));
    second.set_local_state(json!({"name":"second"})).unwrap();
    let mut update = first.update().unwrap();
    update.clients.extend(second.update().unwrap().clients);
    client.send(Message::Awareness(update)).await;
    assert_eq!(client.close_code().await, 4429);
}

#[tokio::test]
async fn awareness_bound_solid() {
    awareness_bound(false).await;
}
#[tokio::test]
async fn awareness_bound_lws() {
    awareness_bound(true).await;
}

async fn awareness_retained_bytes(mode: bool) {
    use yrs::sync::{awareness::AwarenessUpdateEntry, AwarenessUpdate};

    let h = Harness::new(
        mode,
        Limits {
            message_bytes: 256,
            document_bytes: 1024,
            ..Default::default()
        },
    )
    .await;
    let mut observer = h.client(OWNER, 1100).await;
    let mut viewer = h.client(READER, 1101).await;
    let entry = |clock, text: &str| AwarenessUpdateEntry {
        clock,
        json: Arc::from(text),
    };
    let a = json!({"v": "a".repeat(110)}).to_string();
    let b = json!({"v": "b".repeat(20)}).to_string();
    let larger = json!({"v": "b".repeat(150)}).to_string();
    viewer
        .send(Message::Awareness(AwarenessUpdate {
            clients: [
                (yrs::ClientID::new(1101), entry(2, &a)),
                (yrs::ClientID::new(1102), entry(2, &b)),
            ]
            .into_iter()
            .collect(),
        }))
        .await;
    observer.wait_presence(1101, true).await;
    observer.wait_presence(1102, true).await;
    viewer
        .send(Message::Awareness(AwarenessUpdate {
            clients: [
                (yrs::ClientID::new(1101), entry(1, "null")),
                (yrs::ClientID::new(1102), entry(3, &larger)),
            ]
            .into_iter()
            .collect(),
        }))
        .await;
    assert_eq!(viewer.close_code().await, 4413);
    while observer
        .awareness
        .state::<Value>(yrs::ClientID::new(1102))
        .is_some()
    {
        if let Message::Awareness(update) = observer.receive().await {
            assert!(update
                .clients
                .values()
                .all(|entry| entry.json.as_ref() != larger));
        }
    }
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
}

#[tokio::test]
async fn awareness_retained_bytes_solid() {
    awareness_retained_bytes(false).await;
}
#[tokio::test]
async fn awareness_retained_bytes_lws() {
    awareness_retained_bytes(true).await;
}

async fn awareness_forwarding(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            awareness_clients: 1,
            ..Default::default()
        },
    )
    .await;
    let mut observer = h.client(OWNER, 1300).await;
    let mut origin = h.client(WRITER, 1301).await;
    let mut forwarder = h.client(READER, 1302).await;
    let id = yrs::ClientID::new(1301);
    let mut awareness = Awareness::new(Doc::with_client_id(1301));
    awareness
        .set_local_state(json!({"name": "origin"}))
        .unwrap();
    let update = awareness.update().unwrap();
    origin.send(Message::Awareness(update.clone())).await;
    observer.wait_presence(1301, true).await;
    forwarder.wait_presence(1301, true).await;
    forwarder.send(Message::Awareness(update.clone())).await;
    let mut forwarded = update;
    let entry = forwarded.clients.get_mut(&id).unwrap();
    entry.clock += 1;
    entry.json = Arc::from(json!({"name": "forwarded"}).to_string());
    forwarder.send(Message::Awareness(forwarded.clone())).await;
    loop {
        if let Message::Awareness(update) = observer.receive().await {
            if update
                .clients
                .get(&id)
                .is_some_and(|entry| entry.clock == 2)
            {
                break;
            }
        }
    }
    forwarder.finish_close().await;
    observer.send(Message::AwarenessQuery).await;
    observer
        .send(Message::Sync(
            SyncMessage::SyncStep1(StateVector::default()),
        ))
        .await;
    let mut snapshot = None;
    loop {
        match observer.receive().await {
            Message::Awareness(update) => {
                assert!(update
                    .clients
                    .get(&id)
                    .is_none_or(|entry| entry.json.as_ref() != "null"));
                snapshot = Some(update);
            }
            Message::Sync(SyncMessage::SyncStep2(_)) => break,
            _ => {}
        }
    }
    assert_eq!(
        snapshot.unwrap().clients.get(&id).unwrap().json,
        forwarded.clients[&id].json
    );
    origin.finish_close().await;
    observer.wait_presence(1301, false).await;
    forwarded.clients.get_mut(&id).unwrap().json = Arc::from("null");
    observer.send(Message::Awareness(forwarded)).await;
    let mut next = h.client(READER, 1303).await;
    let mut awareness = Awareness::new(Doc::with_client_id(1303));
    awareness.set_local_state(json!({"name": "next"})).unwrap();
    next.send(Message::Awareness(awareness.update().unwrap()))
        .await;
    observer.wait_presence(1303, true).await;
    next.finish_close().await;
    observer.wait_presence(1303, false).await;
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
}

#[tokio::test]
async fn awareness_forwarding_solid() {
    awareness_forwarding(false).await;
}
#[tokio::test]
async fn awareness_forwarding_lws() {
    awareness_forwarding(true).await;
}

#[tokio::test]
#[ignore = "requires npm ci in tests/fixtures/crdt-interop; run that package's npm test"]
async fn real_javascript_peers() {
    for mode in [false, true] {
        let h = Harness::new(mode, Limits::default()).await;
        let mut urls = Vec::new();
        for subject in [OWNER, WRITER, READER] {
            let url = url::Url::parse(&h.ticket(subject).await).unwrap();
            urls.push(format!(
                "ws://{}{}?{}",
                h.addr,
                url.path(),
                url.query().unwrap()
            ));
        }
        let input = serde_json::to_string(&urls).unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(25),
            tokio::task::spawn_blocking(move || {
                std::process::Command::new("node")
                    .arg(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/crdt-interop/client.cjs"
                    ))
                    .arg(input)
                    .output()
            }),
        )
        .await
        .expect("JavaScript peer deadline")
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "mode {mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result,
            json!({"converged": true, "presence": true, "continuedEditing": true})
        );
    }
}

async fn presence_churn(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            awareness_clients: 1,
            ..Default::default()
        },
    )
    .await;
    let mut observer = h.client(OWNER, 900).await;
    for (id, clock) in [(901, 1), (901, 2), (902, 1)] {
        let mut viewer = h.client(READER, id).await;
        let mut awareness = Awareness::new(Doc::with_client_id(id));
        for revision in 0..clock {
            awareness
                .set_local_state(json!({"name":"present", "revision":revision}))
                .unwrap();
        }
        viewer
            .send(Message::Awareness(awareness.update().unwrap()))
            .await;
        observer.wait_presence(id, true).await;
        viewer.finish_close().await;
        observer.wait_presence(id, false).await;
    }
}

#[tokio::test]
async fn presence_ids_released_and_clean_close_solid() {
    presence_churn(false).await;
}
#[tokio::test]
async fn presence_ids_released_and_clean_close_lws() {
    presence_churn(true).await;
}

async fn ticket_body_limit(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    let chunks =
        futures_util::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from(vec![
            b' ';
            8193
        ]))])
        .chain(futures_util::stream::pending());
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        h.call(
            "POST",
            h.tickets(),
            OWNER,
            "application/json",
            Body::from_stream(chunks),
        ),
    )
    .await
    .expect("oversized ticket body must be refused before reading more chunks");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn ticket_body_is_bounded_while_reading_solid() {
    ticket_body_limit(false).await;
}
#[tokio::test]
async fn ticket_body_is_bounded_while_reading_lws() {
    ticket_body_limit(true).await;
}

async fn other_room_client(h: &Harness) -> Client {
    const OTHER: &str = "https://pod.example/other/";
    h.store
        .write(OTHER, Bytes::new(), "text/turtle")
        .await
        .unwrap();
    let response = h
        .call(
            "POST",
            h.tickets(),
            OWNER,
            "application/json",
            json!({"room":OTHER}).to_string(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let mut client = Client {
        ws: h.socket(body["url"].as_str().unwrap()).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    tokio::time::timeout(Duration::from_secs(3), client.receive())
        .await
        .expect("storage work in another room must not block admission");
    client
}

async fn load_isolation(mode: bool) {
    let (h, gate) = Harness::gated(mode, Limits::default()).await;
    gate.listing.store(true, Ordering::SeqCst);
    let mut slow = Client {
        ws: h.socket(&h.ticket(OWNER).await).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    let _other = other_room_client(&h).await;
    gate.listing.store(false, Ordering::SeqCst);
    gate.released.notify_one();
    assert!(matches!(
        slow.receive().await,
        Message::Sync(SyncMessage::SyncStep1(_))
    ));
}

#[tokio::test]
async fn loading_does_not_block_other_rooms_solid() {
    load_isolation(false).await;
}
#[tokio::test]
async fn loading_does_not_block_other_rooms_lws() {
    load_isolation(true).await;
}

async fn compaction_isolation(mode: bool) {
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            compact_after: 1,
            ..Default::default()
        },
    )
    .await;
    gate.snapshot.store(true, Ordering::SeqCst);
    let mut writer = h.client(OWNER, 1000).await;
    writer.edit("saved").await;
    tokio::time::timeout(Duration::from_secs(4), async {
        while h.store.list_children(ROOM).await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    writer.finish_close().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    let _other = other_room_client(&h).await;
    gate.snapshot.store(false, Ordering::SeqCst);
    gate.released.notify_one();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let files = h.store.list_children(ROOM).await.unwrap();
            if files.len() == 1
                && h.store
                    .read(files[0].as_str())
                    .await
                    .unwrap()
                    .body
                    .starts_with(b"JYS1")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut rejoined = h.client(OWNER, 1001).await;
    rejoined.until_text("saved").await;
}

#[tokio::test]
async fn compaction_does_not_block_other_rooms_solid() {
    compaction_isolation(false).await;
}
#[tokio::test]
async fn compaction_does_not_block_other_rooms_lws() {
    compaction_isolation(true).await;
}

async fn pending_capacity(mode: bool) {
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            max_log_files: 1,
            ..Default::default()
        },
    )
    .await;
    gate.flush.store(true, Ordering::SeqCst);
    let mut owner = h.client(OWNER, 1100).await;
    let mut writer = h.client(WRITER, 1101).await;
    owner.edit("accepted").await;
    writer.until_text("accepted").await;
    writer.edit("refused").await;
    assert_eq!(writer.close_code().await, 4429);
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
    let mut reader = h.client(READER, 1102).await;
    reader.until_text("accepted").await;
    assert_eq!(reader.vector(), owner.vector());
    gate.flush.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(4), async {
        while h.file_state().await.transact().state_vector() != owner.vector() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.store.list_children(ROOM).await.unwrap().len(), 1);
}

#[tokio::test]
async fn pending_authors_reserve_log_capacity_solid() {
    pending_capacity(false).await;
}
#[tokio::test]
async fn pending_authors_reserve_log_capacity_lws() {
    pending_capacity(true).await;
}

async fn pending_snapshot(mode: bool) {
    let (h, gate) = Harness::gated(mode, Limits::default()).await;
    gate.flush.store(true, Ordering::SeqCst);
    let mut owner = h.client(OWNER, 1200).await;
    let mut writer = h.client(WRITER, 1201).await;
    writer.edit("queued").await;
    owner.until_text("queued").await;
    let snapshot = Doc::with_client_id(1202);
    snapshot
        .get_or_insert_text("text")
        .push(&mut snapshot.transact_mut(), "snapshot");
    let update = snapshot
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let cutoff = u64::MAX - 1;
    let response = h
        .call(
            "POST",
            "/room/",
            OWNER,
            "application/octet-stream",
            envelope(b"JYS1", cutoff, 1, &update),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    while !owner.text().contains("snapshot") {
        owner.receive().await;
    }
    gate.flush.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == owner.vector() {
                assert_eq!(
                    doc.get_or_insert_text("text").get_string(&doc.transact()),
                    owner.text()
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued update must remain replayable after the snapshot");
    for file in h.store.list_children(ROOM).await.unwrap() {
        let bytes = h.store.read(file.as_str()).await.unwrap().body;
        if bytes.starts_with(b"JYU1") {
            assert!(u64::from_be_bytes(bytes[4..12].try_into().unwrap()) >= cutoff);
        }
    }
}

#[tokio::test]
async fn pending_updates_survive_new_snapshot_solid() {
    pending_snapshot(false).await;
}
#[tokio::test]
async fn pending_updates_survive_new_snapshot_lws() {
    pending_snapshot(true).await;
}

async fn external_document_bound(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            document_bytes: 128,
            message_bytes: 4096,
            ..Default::default()
        },
    )
    .await;
    let mut peer = h.client(OWNER, 1300).await;
    let doc = Doc::with_client_id(1301);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), &"x".repeat(256));
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let response = h
        .call(
            "POST",
            "/room/",
            OWNER,
            "application/octet-stream",
            envelope(b"JYU1", 1, 1, &update),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(peer.close_code().await, 4413);
}

#[tokio::test]
async fn external_log_bound_uses_size_close_code_solid() {
    external_document_bound(false).await;
}
#[tokio::test]
async fn external_log_bound_uses_size_close_code_lws() {
    external_document_bound(true).await;
}

async fn overlapping_flush_capacity(mode: bool, capacity: usize) {
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            max_log_files: capacity,
            ..Default::default()
        },
    )
    .await;
    gate.pause_flush.store(true, Ordering::SeqCst);
    let mut writer = h.client(OWNER, 1400).await;
    let mut reader = h.client(READER, 1401).await;
    writer.edit("first").await;
    reader.until_text("first").await;
    tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
        .await
        .unwrap();
    writer.edit("-second").await;
    if capacity == 1 {
        assert_eq!(writer.close_code().await, 4429);
    } else {
        reader.until_text("first-second").await;
        assert!(
            h.store.list_children(ROOM).await.unwrap().is_empty(),
            "the second edit broadcasts while persistence remains blocked"
        );
    }
    gate.pause_flush.store(false, Ordering::SeqCst);
    gate.released.notify_one();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == reader.vector() {
                assert_eq!(
                    doc.get_or_insert_text("text").get_string(&doc.transact()),
                    if capacity == 1 {
                        "first"
                    } else {
                        "first-second"
                    }
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.store.list_children(ROOM).await.unwrap().len(), capacity);
    let mut rejoined = h.client(OWNER, 1402).await;
    rejoined
        .until_text(if capacity == 1 {
            "first"
        } else {
            "first-second"
        })
        .await;
    assert_eq!(rejoined.vector(), reader.vector());
}

#[tokio::test]
async fn in_progress_flush_reserves_capacity_solid() {
    overlapping_flush_capacity(false, 1).await;
}
#[tokio::test]
async fn in_progress_flush_reserves_capacity_lws() {
    overlapping_flush_capacity(true, 1).await;
}

async fn minimal_writer_grants(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    restrict_writer(&h, true, true).await;
    let mut observer = h.client(OWNER, 1500).await;
    let mut writer = h.client(WRITER, 1501).await;
    writer.edit("member creation is sufficient").await;
    observer.until_text("member creation is sufficient").await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == observer.vector() {
                assert_eq!(
                    doc.get_or_insert_text("text").get_string(&doc.transact()),
                    observer.text()
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

async fn compaction_requires_delete_authority(mode: bool) {
    let document_bytes = 4096;
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            compact_after: 1,
            document_bytes,
            ..Default::default()
        },
    )
    .await;
    restrict_writer(&h, true, true).await;
    let mut expected = String::new();
    let mut expected_vector = StateVector::default();
    for round in 0..5 {
        // Requiring a fresh listing proves the previous room actually retired.
        gate.listing.store(true, Ordering::SeqCst);
        let release = async {
            tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
                .await
                .expect("rejoining must load the persisted room");
            gate.listing.store(false, Ordering::SeqCst);
            gate.released.notify_one();
        };
        let (mut writer, ()) = tokio::join!(h.client(WRITER, 1600 + round), release);
        assert_eq!(writer.text(), expected);
        assert_eq!(writer.vector(), expected_vector);
        let before = writer.vector();
        let addition = if round == 0 {
            "x".repeat(3500)
        } else {
            format!("-{round}")
        };
        expected.push_str(&addition);
        writer
            .doc
            .get_or_insert_text("text")
            .push(&mut writer.doc.transact_mut(), &addition);
        let update = writer.doc.transact().encode_state_as_update_v1(&before);
        writer
            .send(Message::Sync(SyncMessage::Update(update)))
            .await;
        expected_vector = writer.vector();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if h.file_state().await.transact().state_vector() == expected_vector {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("each accepted edit must reach the room files");
        let encoded = writer
            .doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(encoded.len() < document_bytes);
        assert!(encoded.len() * 5 > document_bytes * 4);
        writer.finish_close().await;
        drop(writer);
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::time::resume();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut updates = 0;
        for child in h.store.list_children(ROOM).await.unwrap() {
            if child.as_str().ends_with(".meta") {
                continue;
            }
            let body = h.store.read(child.as_str()).await.unwrap().body;
            assert!(
                body.starts_with(b"JYU1"),
                "compaction requires delete authority"
            );
            updates += 1;
        }
        assert_eq!(updates, round as usize + 1);
    }
    let mut owner = h.client(OWNER, 1700).await;
    assert_eq!(owner.text(), expected);
    assert_eq!(owner.vector(), expected_vector);
    // The last departing writer need not be the eligible compactor.
    let mut writer = h.client(WRITER, 1701).await;
    owner.finish_close().await;
    writer.finish_close().await;
    drop((owner, writer));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let mut logs = Vec::new();
            for child in h.store.list_children(ROOM).await.unwrap() {
                if !child.as_str().ends_with(".meta") {
                    logs.push(h.store.read(child.as_str()).await.unwrap().body);
                }
            }
            if logs.len() == 1 && logs[0].starts_with(b"JYS1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("eligible writer must compact to one confirmed snapshot");
    let mut reloaded = h.client(OWNER, 1702).await;
    reloaded.until_text(&expected).await;
    assert_eq!(reloaded.vector(), expected_vector);
}

#[tokio::test]
async fn compaction_requires_delete_authority_solid() {
    compaction_requires_delete_authority(false).await;
}
#[tokio::test]
async fn compaction_requires_delete_authority_lws() {
    compaction_requires_delete_authority(true).await;
}

async fn socket_decoder_capacity(mode: bool) {
    use tokio_tungstenite::tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    for fragmented in [false, true] {
        let h = Harness::new(
            mode,
            Limits {
                message_bytes: 256,
                document_bytes: 1024,
                ..Default::default()
            },
        )
        .await;
        let mut client = h.client(OWNER, 1750).await;
        if fragmented {
            for (opcode, final_frame) in [(Data::Binary, false), (Data::Continue, true)] {
                client
                    .ws
                    .send(Ws::Frame(Frame::message(
                        vec![0; 300],
                        OpCode::Data(opcode),
                        final_frame,
                    )))
                    .await
                    .unwrap();
            }
        } else {
            client
                .ws
                .send(Ws::Binary(vec![0; 600].into()))
                .await
                .unwrap();
        }
        assert_eq!(client.close_code().await, 4413);
    }
}

#[tokio::test]
async fn socket_decoder_capacity_solid() {
    socket_decoder_capacity(false).await;
}
#[tokio::test]
async fn socket_decoder_capacity_lws() {
    socket_decoder_capacity(true).await;
}

async fn malformed_transport_drops_connection(mode: bool) {
    use tokio::io::AsyncWriteExt;
    let h = Harness::new(mode, Limits::default()).await;
    let mut client = h.client(OWNER, 1800).await;
    let MaybeTlsStream::Plain(stream) = client.ws.get_mut() else {
        panic!("fixture uses a plain local connection");
    };
    // An unmasked client frame is a protocol receive error, not an oversized message.
    stream.write_all(&[0x82, 0x01, 0x00]).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match client.ws.next().await {
                Some(Ok(Ws::Close(frame))) => panic!("unexpected error close frame: {frame:?}"),
                Some(Err(_)) | None => break,
                _ => {}
            }
        }
    })
    .await
    .expect("malformed transport must be dropped");
}

#[tokio::test]
async fn malformed_transport_drops_connection_solid() {
    malformed_transport_drops_connection(false).await;
}
#[tokio::test]
async fn malformed_transport_drops_connection_lws() {
    malformed_transport_drops_connection(true).await;
}

#[tokio::test]
async fn read_and_append_grants_allow_edits_solid() {
    minimal_writer_grants(false).await;
}
#[tokio::test]
async fn read_and_create_grants_allow_edits_lws() {
    minimal_writer_grants(true).await;
}

async fn failed_load_releases_room_slot(mode: bool) {
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            max_rooms: 1,
            ..Default::default()
        },
    )
    .await;
    gate.listing.store(true, Ordering::SeqCst);
    let mut slow = Client {
        ws: h.socket(&h.ticket(OWNER).await).await,
        doc: Doc::new(),
        awareness: Awareness::new(Doc::new()),
    };
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    assert_eq!(slow.close_code().await, 1011);
    let _other = other_room_client(&h).await;
}

#[tokio::test]
async fn failed_load_releases_reserved_slot_solid() {
    failed_load_releases_room_slot(false).await;
}
#[tokio::test]
async fn failed_load_releases_reserved_slot_lws() {
    failed_load_releases_room_slot(true).await;
}

#[tokio::test]
async fn in_progress_flush_allows_reserved_edits_solid() {
    overlapping_flush_capacity(false, 2).await;
}
#[tokio::test]
async fn in_progress_flush_allows_reserved_edits_lws() {
    overlapping_flush_capacity(true, 2).await;
}

async fn unicode_roundtrip(mode: bool) {
    let h = Harness::new(mode, Limits::default()).await;
    let mut a = h.client(OWNER, 1600).await;
    let mut b = h.client(WRITER, 1601).await;
    a.edit("🙂-").await;
    b.until_text("🙂-").await;
    let suffix = "café".repeat(16);
    b.edit(&suffix).await;
    a.until_text(&format!("🙂-{suffix}")).await;
    // Native text offsets are UTF-8 bytes; the encoded update retains Yjs UTF-16 clocks.
    a.doc
        .get_or_insert_text("text")
        .remove_range(&mut a.doc.transact_mut(), 0, 4);
    let update = a
        .doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    a.send(Message::Sync(SyncMessage::Update(update))).await;
    let expected = format!("-{suffix}");
    b.until_text(&expected).await;
    let vector = b.vector();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == vector
                && doc.get_or_insert_text("text").get_string(&doc.transact()) == expected
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    a.finish_close().await;
    b.finish_close().await;
    let store = h.store.clone();
    drop((a, b, h));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let restart = Harness::with_store(mode, Limits::default(), store).await;
    let mut rejoined = restart.client(OWNER, 1602).await;
    rejoined.until_text(&expected).await;
    assert_eq!(rejoined.vector(), vector);
}

#[tokio::test]
async fn unicode_edits_delete_and_reload_solid() {
    unicode_roundtrip(false).await;
}
#[tokio::test]
async fn unicode_edits_delete_and_reload_lws() {
    unicode_roundtrip(true).await;
}

async fn split_flush_reservation(mode: bool) {
    let (h, gate) = Harness::gated(
        mode,
        Limits {
            max_log_files: 2,
            ..Default::default()
        },
    )
    .await;
    gate.pause_flush.store(true, Ordering::SeqCst);
    let mut first = h.client(OWNER, 1700).await;
    let mut second = h.client(WRITER, 1701).await;
    first.edit("first").await;
    second.until_text("first").await;
    tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
        .await
        .unwrap();
    first.edit("-second").await;
    second.until_text("first-second").await;
    second.edit("-third").await;
    assert_eq!(second.close_code().await, 4429);
    gate.pause_flush.store(false, Ordering::SeqCst);
    gate.released.notify_one();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let doc = h.file_state().await;
            if doc.transact().state_vector() == first.vector() {
                assert_eq!(
                    doc.get_or_insert_text("text").get_string(&doc.transact()),
                    "first-second"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.store.list_children(ROOM).await.unwrap().len(), 2);
    let mut rejoined = h.client(READER, 1702).await;
    rejoined.until_text("first-second").await;
    assert_eq!(rejoined.vector(), first.vector());
}

#[tokio::test]
async fn split_flush_slot_is_reserved_from_other_authors_solid() {
    split_flush_reservation(false).await;
}
#[tokio::test]
async fn split_flush_slot_is_reserved_from_other_authors_lws() {
    split_flush_reservation(true).await;
}

async fn frame_record_bound(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            frame_messages: 2,
            ..Default::default()
        },
    )
    .await;
    let mut peer = h.client(OWNER, 1800).await;
    let record = Message::Sync(SyncMessage::Update(vec![0, 0])).encode_v1();
    // A frame at the configured count remains usable.
    peer.ws
        .send(Ws::Binary(record.repeat(2).into()))
        .await
        .unwrap();
    peer.send(Message::Sync(
        SyncMessage::SyncStep1(StateVector::default()),
    ))
    .await;
    loop {
        if matches!(
            peer.receive().await,
            Message::Sync(SyncMessage::SyncStep2(_))
        ) {
            break;
        }
    }
    peer.ws
        .send(Ws::Binary(record.repeat(3).into()))
        .await
        .unwrap();
    assert_eq!(peer.close_code().await, 4429);
    assert!(h.store.list_children(ROOM).await.unwrap().is_empty());
}

#[tokio::test]
async fn protocol_record_count_is_bounded_solid() {
    frame_record_bound(false).await;
}
#[tokio::test]
async fn protocol_record_count_is_bounded_lws() {
    frame_record_bound(true).await;
}

async fn frame_response_bound(mode: bool) {
    let h = Harness::new(
        mode,
        Limits {
            message_bytes: 512,
            document_bytes: 1024,
            ..Default::default()
        },
    )
    .await;
    let doc = Doc::with_client_id(1900);
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), &"x".repeat(800));
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let response = h
        .call(
            "POST",
            "/room/",
            OWNER,
            "application/octet-stream",
            envelope(b"JYU1", 1, 1, &update),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let mut peer = h.client(OWNER, 1901).await;
    assert_eq!(peer.text(), "x".repeat(800));
    let request = Message::Sync(SyncMessage::SyncStep1(StateVector::default())).encode_v1();
    peer.ws
        .send(Ws::Binary(request.repeat(2).into()))
        .await
        .unwrap();
    assert_eq!(peer.close_code().await, 4429);
    let persisted = h.file_state().await;
    assert_eq!(
        persisted.transact().state_vector(),
        doc.transact().state_vector()
    );
}

#[tokio::test]
async fn protocol_response_bytes_are_bounded_solid() {
    frame_response_bound(false).await;
}
#[tokio::test]
async fn protocol_response_bytes_are_bounded_lws() {
    frame_response_bound(true).await;
}

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
    ldp::handler::LdpState,
    lws::{self, LwsConfig, LWS_CONTEXT, LWS_JSON},
    store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store},
};
use sparq_lws_ysync::{yrs, Limits};
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
    loop {
        if let Message::Awareness(update) = a.receive().await {
            if update.clients.values().any(|p| p.json.contains("viewer")) {
                break;
            }
        }
    }
    viewer.ws.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Awareness(update) = a.receive().await {
                if update.clients.values().any(|p| p.json.as_ref() == "null") {
                    break;
                }
            }
        }
    })
    .await
    .expect("closed viewer presence removed");
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
    };
    c.receive().await;
    let mut reused = Client {
        ws: h.socket(&ticket).await,
        doc: Doc::new(),
    };
    assert_eq!(reused.close_code().await, 4401);
    let wrong = h.ticket(OWNER).await.replace("room%2F", "other%2F");
    assert_ne!(wrong, ticket);
    let mut wrong = Client {
        ws: h.socket(&wrong).await,
        doc: Doc::new(),
    };
    assert_eq!(wrong.close_code().await, 4401);
    let expired = h.ticket(OWNER).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    let mut expired = Client {
        ws: h.socket(&expired).await,
        doc: Doc::new(),
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
    for (name, ms, update) in [
        ("000-new.ysnap", 200, &latest),
        ("zzz-old.ysnap", 100, &old),
    ] {
        h.store
            .create_in_container(
                ROOM,
                &format!("{ROOM}{name}"),
                Bytes::from(envelope(b"JYS1", ms, 1, update)),
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

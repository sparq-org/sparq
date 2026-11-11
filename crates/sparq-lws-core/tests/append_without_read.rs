//! Request-only insert storage through the real authenticated LDP router.
mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use common::{jwks_provider, mint_access_token_for, mint_dpop_proof, KeyKit, BASE_URL};
use solid_oidc_verifier::{
    config::VerifierConfig, replay::InMemoryReplayStore, verifier::Verifier,
};
use sparq_lws_core::store::{
    BackendCounters, BodyCache, CompositeStore, CounterSnapshot, CountingBlobStore,
    CountingSparqClient, InMemoryBlobStore, InMemorySparqClient, InMemoryStoreLimits, Store,
};
use sparq_lws_core::{
    app::{build_router, AppState},
    auth::AuthContext,
    ldp::handler::LdpState,
};
use tower::ServiceExt;

const OWNER: &str = "https://owner.example/#me";
const WRITER: &str = "https://writer.example/#me";
const TARGET: &str = "https://pod.example/doc";
const INSERT: &str = "<https://e.example/s> <https://e.example/p> \"new\" .\n";
const PATCH: &str = "@prefix solid: <http://www.w3.org/ns/solid/terms#>.\n_:p solid:inserts { <https://e.example/s> <https://e.example/p> \"new\" . }.";

type CountedStore =
    CompositeStore<CountingSparqClient<InMemorySparqClient>, CountingBlobStore<InMemoryBlobStore>>;
struct Harness {
    app: axum::Router,
    store: CountedStore,
    counters: Arc<BackendCounters>,
    issuer: KeyKit,
    client: KeyKit,
}

impl Harness {
    async fn new(
        body: Option<Bytes>,
        ct: &str,
        container_append: bool,
        spare: Option<usize>,
    ) -> Self {
        Self::with_writer_mode(body, ct, container_append, spare, "Append").await
    }

    async fn with_writer_mode(
        body: Option<Bytes>,
        ct: &str,
        container_append: bool,
        spare: Option<usize>,
        mode: &str,
    ) -> Self {
        let root_acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>.\n<#o> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <{BASE_URL}/>; acl:default <{BASE_URL}/>; acl:mode acl:Read, acl:Write, acl:Control.\n{}",
            if container_append { format!("<#w> a acl:Authorization; acl:agent <{WRITER}>; acl:accessTo <{BASE_URL}/>; acl:default <{BASE_URL}/>; acl:mode acl:{mode}.") } else { String::new() });
        let target_acl = format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>.\n<#o> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <{TARGET}>; acl:mode acl:Read, acl:Write, acl:Control.\n<#w> a acl:Authorization; acl:agent <{WRITER}>; acl:accessTo <{TARGET}>; acl:mode acl:{mode}.");
        let used = root_acl.len() + target_acl.len() + body.as_ref().map_or(0, Bytes::len);
        let limits = InMemoryStoreLimits::new(spare.map_or(usize::MAX, |n| used + n), 1024);
        let counters = BackendCounters::new();
        let store = CompositeStore::with_body_cache(
            CountingSparqClient::new(InMemorySparqClient::with_limits(limits), counters.clone()),
            CountingBlobStore::new(InMemoryBlobStore::with_limits(limits), counters.clone()),
            BodyCache::disabled(),
        );
        store
            .write(&format!("{BASE_URL}/"), Bytes::new(), "text/turtle")
            .await
            .unwrap();
        store
            .write(
                &format!("{BASE_URL}/.acl"),
                Bytes::from(root_acl),
                "text/turtle",
            )
            .await
            .unwrap();
        store
            .write(
                &format!("{TARGET}.acl"),
                Bytes::from(target_acl),
                "text/turtle",
            )
            .await
            .unwrap();
        if let Some(body) = body {
            store.write(TARGET, body, ct).await.unwrap();
        }
        let issuer = KeyKit::generate();
        let client = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer), replay).unwrap();
        let app = build_router(AppState::new(
            AuthContext::new(verifier, BASE_URL),
            LdpState::new(store.clone(), BASE_URL),
        ));
        Self {
            app,
            store,
            counters,
            issuer,
            client,
        }
    }

    async fn request(
        &self,
        who: Option<&str>,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> axum::response::Response {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(who) = who {
            let access = mint_access_token_for(&self.issuer, &self.client.thumbprint, who);
            let proof =
                mint_dpop_proof(&self.client, method, &format!("{BASE_URL}{path}"), &access);
            req = req
                .header("authorization", format!("DPoP {access}"))
                .header("dpop", proof);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let body = if method == "PATCH" {
            req = req.header("content-type", "text/n3");
            Body::from(PATCH)
        } else {
            Body::empty()
        };
        self.app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn insert(&self) -> (StatusCode, HeaderMap, Bytes, CounterSnapshot) {
        let scope = self.counters.measure();
        let response = self.request(Some(WRITER), "PATCH", "/doc", &[]).await;
        let (mut parts, body) = response.into_parts();
        parts.headers.remove("date");
        (
            parts.status,
            parts.headers,
            to_bytes(body, usize::MAX).await.unwrap(),
            scope.delta(),
        )
    }
}

#[tokio::test]
async fn an_insert_without_read_reads_nothing_of_the_target() {
    let mut supported = Vec::new();
    for (ct, body) in [
        ("text/turtle", Bytes::from_static(b"0123456789")),
        ("text/turtle", Bytes::from(vec![b'x'; 4 * 1024 * 1024])),
        ("application/n-triples", Bytes::from_static(b"broken")),
        ("application/n-quads", Bytes::from_static(b"broken")),
        ("application/ld+json", Bytes::from_static(b"{}")),
        ("image/png", Bytes::from_static(b"PNG")),
    ] {
        let h = Harness::new(Some(body), ct, true, None).await;
        let (status, _, _, counts) = h.insert().await;
        // The one blob read resolves the ACL, never the target, even when its
        // representation is refused. Replacing this path with store.read adds a get.
        assert_eq!(counts.blob_gets, 1, "{ct}: only the target ACL's bytes");
        if matches!(ct, "application/ld+json" | "image/png") {
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "unsupported stored format residual"
            );
            assert_eq!(counts.blob_puts, 0);
        } else {
            assert_eq!(status, StatusCode::NO_CONTENT);
            supported.push(counts);
        }
    }
    let prior = Harness::new(Some(Bytes::from_static(b"old")), "text/turtle", true, None).await;
    prior
        .store
        .append(TARGET, Bytes::from_static(INSERT.as_bytes()))
        .await
        .unwrap();
    let (status, _, _, counts) = prior.insert().await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        counts, supported[0],
        "a target with an earlier stored insert"
    );
    for counts in &supported[1..] {
        assert_eq!(*counts, supported[0]);
    }
}

#[tokio::test]
async fn an_insert_without_read_is_answered_alike_whatever_is_stored() {
    let mut answers = Vec::new();
    for (ct, body) in [
        ("text/turtle", Some(Bytes::from_static(b"0123456789"))),
        (
            "text/turtle",
            Some(Bytes::from(vec![b'x'; 4 * 1024 * 1024])),
        ),
        ("application/n-triples", Some(Bytes::from_static(b"broken"))),
        ("application/n-quads", Some(Bytes::from_static(b"broken"))),
        ("text/turtle", None),
    ] {
        let h = Harness::new(body, ct, true, None).await;
        let (status, headers, body, _) = h.insert().await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!headers.contains_key("location"));
        assert!(!headers.contains_key("etag"));
        assert!(body.is_empty());
        answers.push((status, headers, body));
        assert!(h
            .store
            .read(TARGET)
            .await
            .unwrap()
            .body
            .ends_with(INSERT.as_bytes()));
    }
    for answer in &answers[1..] {
        assert_eq!(*answer, answers[0]);
    }
}

#[tokio::test]
async fn an_insert_without_read_fits_the_quota_of_its_own_size() {
    let original = Bytes::from(vec![b'x'; 4096]);
    let h = Harness::new(
        Some(original.clone()),
        "text/turtle",
        true,
        Some(INSERT.len() + 3),
    )
    .await;
    assert_eq!(h.insert().await.0, StatusCode::NO_CONTENT);
    let mut expected = original.to_vec();
    expected.push(b'\n');
    expected.extend_from_slice(INSERT.as_bytes());
    assert_eq!(h.store.read(TARGET).await.unwrap().body, expected);
    assert_eq!(h.insert().await.0, StatusCode::INSUFFICIENT_STORAGE);
}

#[tokio::test]
async fn an_insert_is_shown_to_readers() {
    let original = b"<https://e.example/a> <https://e.example/b> \"old\" .";
    for ct in [
        "text/turtle",
        "application/n-triples",
        "application/n-quads",
    ] {
        let h = Harness::new(Some(Bytes::from_static(original)), ct, true, None).await;
        let old = h
            .request(Some(OWNER), "GET", "/doc", &[("accept", ct)])
            .await;
        let tag = old.headers()["etag"].to_str().unwrap().to_owned();
        assert_eq!(h.insert().await.0, StatusCode::NO_CONTENT);
        let get = h
            .request(
                Some(OWNER),
                "GET",
                "/doc",
                &[("if-none-match", &tag), ("accept", ct)],
            )
            .await;
        assert_eq!(get.status(), StatusCode::OK);
        assert_ne!(get.headers()["etag"], tag);
        let mut expected = original.to_vec();
        expected.push(b'\n');
        expected.extend_from_slice(INSERT.as_bytes());
        assert_eq!(
            to_bytes(get.into_body(), usize::MAX).await.unwrap(),
            expected
        );
        let range = format!("bytes={}-", original.len() + 1);
        let get = h
            .request(
                Some(OWNER),
                "GET",
                "/doc",
                &[("range", &range), ("accept", ct)],
            )
            .await;
        assert_eq!(get.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(to_bytes(get.into_body(), usize::MAX).await.unwrap(), INSERT);
    }
}

#[tokio::test]
async fn inserts_into_unsupported_stored_formats_are_refused_without_changes() {
    for ct in ["application/ld+json", "image/png", "text/plain"] {
        let original = Bytes::from_static(b"opaque");
        let h = Harness::new(Some(original.clone()), ct, true, None).await;
        let before = h.store.meta(TARGET).await.unwrap().unwrap();
        assert_eq!(h.insert().await.0, StatusCode::FORBIDDEN);
        let after = h.store.read(TARGET).await.unwrap();
        assert_eq!(after.body, original);
        assert_eq!(after.meta, before);
    }
}

#[tokio::test]
async fn a_container_insert_without_read_is_refused_alike() {
    for who in [Some(WRITER), None] {
        let h = Harness::new(None, "text/turtle", true, None).await;
        // Give anonymous the same Append-only grant for this case.
        if who.is_none() {
            h.store.write(&format!("{BASE_URL}/.acl"), Bytes::from(format!("@prefix acl: <http://www.w3.org/ns/auth/acl#>. <#a> a acl:Authorization; acl:agentClass <http://xmlns.com/foaf/0.1/Agent>; acl:accessTo <{BASE_URL}/>; acl:default <{BASE_URL}/>; acl:mode acl:Append.")), "text/turtle").await.unwrap();
        }
        let expected = if who.is_some() {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::UNAUTHORIZED
        };
        let missing = h.request(who, "PATCH", "/container/", &[]).await;
        assert_eq!(missing.status(), expected);
        h.store
            .write(
                &format!("{BASE_URL}/container/"),
                Bytes::from_static(b"whatever"),
                "image/png",
            )
            .await
            .unwrap();
        let present = h.request(who, "PATCH", "/container/", &[]).await;
        assert_eq!(present.status(), expected);
        let mut missing_headers = missing.headers().clone();
        let mut present_headers = present.headers().clone();
        missing_headers.remove("date");
        present_headers.remove("date");
        assert_eq!(missing_headers, present_headers);
        assert_eq!(
            to_bytes(missing.into_body(), usize::MAX).await.unwrap(),
            to_bytes(present.into_body(), usize::MAX).await.unwrap()
        );
    }
}

#[tokio::test]
async fn residual_creating_by_insert_needs_append_on_the_container() {
    // CTH write-access-bob.feature: "Bob can PATCH rdf no A". A target
    // Append grant suffices for an existing document even with no container mode.
    let present = Harness::new(Some(Bytes::from_static(b"old")), "text/turtle", false, None).await;
    assert_eq!(present.insert().await.0, StatusCode::NO_CONTENT);
    let absent = Harness::new(None, "text/turtle", false, None).await;
    assert_eq!(absent.insert().await.0, StatusCode::FORBIDDEN);
    assert!(!absent.store.exists(TARGET).await.unwrap());
}

#[tokio::test]
async fn conditional_inserts_without_read_are_refused_without_changes() {
    let mut answers = Vec::new();
    for mode in ["Append", "Write"] {
        for present in [false, true] {
            for header in ["if-match", "if-none-match"] {
                for value in ["*", "\"held\"", ""] {
                    let original = present.then(|| Bytes::from_static(b"old"));
                    let h = Harness::with_writer_mode(
                        original.clone(),
                        "text/turtle",
                        true,
                        None,
                        mode,
                    )
                    .await;
                    let before = h.store.meta(TARGET).await.unwrap();
                    let scope = h.counters.measure();
                    let response = h
                        .request(Some(WRITER), "PATCH", "/doc", &[(header, value)])
                        .await;
                    let counts = scope.delta();
                    let (mut parts, body) = response.into_parts();
                    parts.headers.remove("date");
                    assert_eq!(
                        parts.status,
                        StatusCode::FORBIDDEN,
                        "{header}: {value}, present={present}"
                    );
                    assert_eq!(counts.blob_puts, 0, "a denied insert stores no bytes");
                    assert_eq!(
                        counts.sparql_updates, 0,
                        "a denied insert changes no index rows"
                    );
                    assert_eq!(h.store.meta(TARGET).await.unwrap(), before);
                    if let Some(original) = original {
                        assert_eq!(h.store.read(TARGET).await.unwrap().body, original);
                    }
                    answers.push((
                        parts.status,
                        parts.headers,
                        to_bytes(body, usize::MAX).await.unwrap(),
                    ));
                }
            }
        }
    }
    for answer in &answers[1..] {
        assert_eq!(
            *answer, answers[0],
            "denial depends only on the requester's modes"
        );
    }
}

#[tokio::test]
async fn an_owner_get_folds_long_insert_chains_and_bounds_repeat_reads() {
    // Fixed inputs pin when maintenance must have happened, independently of
    // the implementation threshold.
    for count in [33, 96] {
        let h = Harness::new(
            Some(Bytes::from_static(b"old\n")),
            "text/turtle",
            true,
            None,
        )
        .await;
        let scope = h.counters.measure();
        let baseline = h.request(Some(OWNER), "GET", "/doc", &[]).await;
        assert_eq!(baseline.status(), StatusCode::OK);
        let budget = scope.delta().total_backend_ops();
        let mut expected = b"old\n".to_vec();
        for _ in 0..count {
            h.store
                .append(TARGET, Bytes::from_static(INSERT.as_bytes()))
                .await
                .unwrap();
            expected.extend_from_slice(INSERT.as_bytes());
        }
        let before = h.store.meta(TARGET).await.unwrap().unwrap();
        let folded = h.request(Some(OWNER), "GET", "/doc", &[]).await;
        assert_eq!(folded.status(), StatusCode::OK);
        assert_eq!(folded.headers()["etag"], before.etag);
        assert_eq!(
            to_bytes(folded.into_body(), usize::MAX).await.unwrap(),
            expected
        );
        let after = h.store.meta(TARGET).await.unwrap().unwrap();
        assert_eq!(after.append_seq, 0, "a reader folds the long chain");
        assert_ne!(after.blob_key, before.blob_key);
        assert_eq!(after.etag, before.etag);
        assert_eq!(after.last_modified, before.last_modified);
        let scope = h.counters.measure();
        let repeated = h.request(Some(OWNER), "GET", "/doc", &[]).await;
        assert_eq!(repeated.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(repeated.into_body(), usize::MAX).await.unwrap(),
            expected
        );
        assert!(
            scope.delta().total_backend_ops() <= budget,
            "a folded GET fits the unappended GET's operation budget: {:?}",
            scope.delta()
        );
        h.store
            .append(TARGET, Bytes::from_static(INSERT.as_bytes()))
            .await
            .unwrap();
        expected.extend_from_slice(INSERT.as_bytes());
        assert_eq!(h.store.read(TARGET).await.unwrap().body, expected);
    }
}

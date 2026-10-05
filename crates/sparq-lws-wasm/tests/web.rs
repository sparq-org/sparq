// [GPT-5.6] sq-6xasp.3: execute the real LWS router through the exported wasm entry.
// [SONNET-4.6] sq-wubkf: and the linear-memory admission refusal.
// [GPT-5.6] sq-6xasp.7: and the opt-in snapshot that survives a listener restart.
#![cfg(target_arch = "wasm32")]

use sparq_lws_wasm::memory;
use sparq_lws_wasm::SolidServer;
use wasm_bindgen_test::wasm_bindgen_test;

const BASE_URL: &str = "https://pod.example";
const OWNER: &str = "https://id.example/alice#me";
const INTRUDER: &str = "https://id.example/mallory#me";
const TURTLE: &[u8] = b"<https://pod.example/card> <http://xmlns.com/foaf/0.1/name> \"Ada\" .\n";
const TURTLE_V2: &[u8] = b"<https://pod.example/card> <http://xmlns.com/foaf/0.1/name> \"Grace\" .\n";

// [SONNET-4.6] sq-wubkf: the bounded allocator is only installed on wasm32, so this is the only
// place the live-byte accounting and the 507 refusal can be exercised end to end. A ceiling below
// the module's own live total must refuse the very next request with a clean HTTP 507 — an HTTP
// answer, not the `unreachable` trap an allocation failure at the real ceiling would produce.
#[wasm_bindgen_test]
async fn a_ceiling_below_the_live_total_refuses_the_next_request_with_507() {
    let server = SolidServer::new(BASE_URL.to_owned(), OWNER.to_owned())
        .expect("the in-memory pod must provision its owner ACL");
    let restore = memory::ceiling_bytes();

    assert!(
        memory::live_bytes() > 0,
        "the bounded allocator must be installed on wasm32 and counting"
    );
    assert!(
        memory::peak_bytes() >= memory::live_bytes(),
        "the high-water mark never trails the live total"
    );

    // A ceiling of one byte cannot admit anything: the projection always adds the fixed headroom.
    memory::set_ceiling_bytes(1);
    let refused = server
        .handle_request(
            "PUT".to_owned(),
            "/card".to_owned(),
            vec!["content-type".to_owned(), "text/turtle".to_owned()],
            TURTLE.to_vec(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("an exhausted linear memory is an HTTP response, not a host error");
    assert_eq!(refused.status(), 507, "insufficient storage, not a trap");
    assert_eq!(
        String::from_utf8(refused.body()).expect("the 507 body is UTF-8"),
        "insufficient storage",
        "byte-identical to the store-quota 507 the host already handles"
    );

    memory::set_ceiling_bytes(restore as usize);
    let admitted = server
        .handle_request(
            "PUT".to_owned(),
            "/card".to_owned(),
            vec!["content-type".to_owned(), "text/turtle".to_owned()],
            TURTLE.to_vec(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("owner PUT must complete once the ceiling is restored");
    assert_eq!(
        admitted.status(),
        201,
        "the refusal is admission control, not a poisoned instance"
    );
}

#[wasm_bindgen_test]
async fn put_get_round_trip_and_wac_denial_drive_the_real_router() {
    let server = SolidServer::new(BASE_URL.to_owned(), OWNER.to_owned())
        .expect("the in-memory pod must provision its owner ACL");

    let put = server
        .handle_request(
            "PUT".to_owned(),
            "/card".to_owned(),
            vec!["content-type".to_owned(), "text/turtle".to_owned()],
            TURTLE.to_vec(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("owner PUT must complete");
    assert_eq!(put.status(), 201, "a new LDP resource is created");

    let get = server
        .handle_request(
            "GET".to_owned(),
            "/card".to_owned(),
            vec!["accept".to_owned(), "text/turtle".to_owned()],
            Vec::new(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("owner GET must complete");
    assert_eq!(get.status(), 200);
    assert_eq!(get.body(), TURTLE, "PUT bytes must round-trip through GET");

    let denied = server
        .handle_request(
            "GET".to_owned(),
            "/card".to_owned(),
            Vec::new(),
            Vec::new(),
            Some(INTRUDER.to_owned()),
        )
        .await
        .expect("WAC denial is an HTTP response, not a host error");
    assert_eq!(denied.status(), 403, "an authenticated non-owner is denied");

    #[cfg(not(feature = "sparql-endpoint"))]
    {
        let absent = server
            .handle_request(
                "GET".to_owned(),
                "/sparql".to_owned(),
                Vec::new(),
                Vec::new(),
                Some(OWNER.to_owned()),
            )
            .await
            .expect("core-tier route absence is an HTTP response");
        assert_eq!(absent.status(), 404, "core wasm compiles /sparql out");
    }

    #[cfg(feature = "sparql-endpoint")]
    {
        // [GPT-5.6] sq-r1ei8: the full wasm artifact drives the same WAC-scoped
        // handler. The owner can query the card just PUT through LDP.
        let query = server
            .handle_request(
                "POST".to_owned(),
                "/sparql".to_owned(),
                vec![
                    "content-type".to_owned(),
                    "application/sparql-query".to_owned(),
                ],
                b"ASK { GRAPH <https://pod.example/card> { <https://pod.example/card> <http://xmlns.com/foaf/0.1/name> \"Ada\" } }".to_vec(),
                Some(OWNER.to_owned()),
            )
            .await
            .expect("full-tier SPARQL request completes");
        assert_eq!(query.status(), 200);
        assert!(
            String::from_utf8(query.body())
                .expect("SPARQL JSON is UTF-8")
                .contains("\"boolean\":true"),
            "the LDP-created resource is queryable in its authorized named graph"
        );
    }
}

// [GPT-5.6] sq-6xasp.7: the acceptance property for opt-in persistence — a pod rebuilt from the
// bytes a host kept serves exactly what the previous instance held. Only reachable on wasm32,
// where the store, the router, and the journal decorator are all compiled in.
async fn put(server: &SolidServer, path: &str, body: &[u8]) -> u16 {
    server
        .handle_request(
            "PUT".to_owned(),
            path.to_owned(),
            vec!["content-type".to_owned(), "text/turtle".to_owned()],
            body.to_vec(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("owner PUT must complete")
        .status()
}

async fn get_as(server: &SolidServer, path: &str, webid: &str) -> (u16, Vec<u8>) {
    let (status, body, _) = get_with_etag(server, path, webid).await;
    (status, body)
}

async fn get_with_etag(server: &SolidServer, path: &str, webid: &str) -> (u16, Vec<u8>, String) {
    let response = server
        .handle_request(
            "GET".to_owned(),
            path.to_owned(),
            vec!["accept".to_owned(), "text/turtle".to_owned()],
            Vec::new(),
            Some(webid.to_owned()),
        )
        .await
        .expect("a GET is an HTTP response, not a host error");
    let headers = response.headers();
    let etag = headers
        .chunks_exact(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("etag"))
        .map(|pair| pair[1].clone())
        .unwrap_or_default();
    (response.status(), response.body(), etag)
}

#[wasm_bindgen_test]
async fn a_pod_restored_from_a_snapshot_serves_what_was_written_before_the_restart() {
    let server = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), Vec::new())
        .expect("a fresh persistent pod must provision its owner ACL");

    assert_eq!(put(&server, "/card", TURTLE).await, 201);
    assert_eq!(put(&server, "/card", TURTLE_V2).await, 204, "overwrite");
    assert_eq!(put(&server, "/notes", TURTLE).await, 201);
    let deleted = server
        .handle_request(
            "DELETE".to_owned(),
            "/notes".to_owned(),
            Vec::new(),
            Vec::new(),
            Some(OWNER.to_owned()),
        )
        .await
        .expect("owner DELETE must complete");
    assert_eq!(deleted.status(), 204);

    let (_, _, etag_before) = get_with_etag(&server, "/card", OWNER).await;
    assert!(!etag_before.is_empty(), "the LDP GET carries a validator");

    let snapshot = server
        .snapshot()
        .expect("a pod built with withSnapshot exposes its bytes");
    // The listener restart: the previous instance and all of its linear memory are gone.
    drop(server);

    let restored = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), snapshot)
        .expect("the snapshot this pod just produced must restore");

    let (status, body, etag_after) = get_with_etag(&restored, "/card", OWNER).await;
    assert_eq!(status, 200, "the resource survived the restart");
    assert_eq!(body, TURTLE_V2, "and carries the LATEST body, not the first");
    assert_eq!(
        etag_after, etag_before,
        "the ETag is derived from the body, so a restart does not invalidate a client's cache"
    );

    let (gone, _) = get_as(&restored, "/notes", OWNER).await;
    assert_eq!(gone, 404, "a deletion survives the restart too");

    let (denied, _) = get_as(&restored, "/card", INTRUDER).await;
    assert_eq!(denied, 403, "the restored pod still enforces the owner ACL");

    // And it is immediately snapshottable again, so a restart does not end persistence.
    let second = restored
        .snapshot()
        .expect("a restored pod journals as the original did");
    let twice = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), second)
        .expect("a second restart restores from the restored pod's own snapshot");
    assert_eq!(get_as(&twice, "/card", OWNER).await, (200, TURTLE_V2.to_vec()));
}

// The owner ACL is provisioned only when the restored pod has none, so a restart must not revert
// an ACL the owner edited. Written with absolute IRIs so the assertion does not depend on how the
// store resolves a relative Turtle base.
const WIDENED_ACL: &[u8] = br#"@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<https://pod.example/.acl#owner> a acl:Authorization ;
  acl:agent <https://id.example/alice#me> ;
  acl:accessTo <https://pod.example/> ;
  acl:default <https://pod.example/> ;
  acl:mode acl:Read, acl:Write, acl:Control .
<https://pod.example/.acl#reader> a acl:Authorization ;
  acl:agent <https://id.example/mallory#me> ;
  acl:accessTo <https://pod.example/> ;
  acl:default <https://pod.example/> ;
  acl:mode acl:Read .
"#;

#[wasm_bindgen_test]
async fn a_restart_keeps_an_edited_owner_acl_instead_of_reprovisioning_over_it() {
    let server = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), Vec::new())
        .expect("a fresh persistent pod must provision its owner ACL");
    assert_eq!(put(&server, "/card", TURTLE).await, 201);
    assert_eq!(get_as(&server, "/card", INTRUDER).await.0, 403, "before");
    assert_eq!(put(&server, "/.acl", WIDENED_ACL).await, 204);
    assert_eq!(
        get_as(&server, "/card", INTRUDER).await.0,
        200,
        "the owner widened the root ACL"
    );

    let snapshot = server.snapshot().expect("a persistent pod has bytes");
    drop(server);
    let restored = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), snapshot)
        .expect("the snapshot must restore");
    assert_eq!(
        get_as(&restored, "/card", INTRUDER).await.0,
        200,
        "re-seeding the owner ACL on boot would silently revoke this grant"
    );
}

#[wasm_bindgen_test]
async fn an_in_memory_pod_exposes_no_snapshot_so_persistence_stays_opt_in() {
    let ephemeral = SolidServer::new(BASE_URL.to_owned(), OWNER.to_owned())
        .expect("the in-memory pod must provision its owner ACL");
    assert_eq!(put(&ephemeral, "/card", TURTLE).await, 201);
    assert!(
        ephemeral.snapshot().is_none(),
        "the default constructor journals nothing"
    );
    assert_eq!(ephemeral.snapshot_revision(), 0.0);

    let persistent = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), Vec::new())
        .expect("a fresh persistent pod must provision its owner ACL");
    let before = persistent.snapshot_revision();
    assert_eq!(put(&persistent, "/card", TURTLE).await, 201);
    assert!(
        persistent.snapshot_revision() > before,
        "a write moves the revision a host watches to decide when to flush"
    );
}

#[wasm_bindgen_test]
async fn a_snapshot_that_cannot_be_decoded_is_refused_rather_than_partly_applied() {
    assert!(
        SolidServer::with_snapshot(
            BASE_URL.to_owned(),
            OWNER.to_owned(),
            b"not a journal".to_vec(),
        )
        .is_err(),
        "booting a pod from unreadable bytes would silently lose the host's data"
    );

    let good = SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), Vec::new())
        .expect("a fresh persistent pod must provision its owner ACL");
    assert_eq!(put(&good, "/card", TURTLE).await, 201);
    let mut truncated = good.snapshot().expect("a persistent pod has bytes");
    truncated.truncate(truncated.len() - 1);
    assert!(
        SolidServer::with_snapshot(BASE_URL.to_owned(), OWNER.to_owned(), truncated).is_err(),
        "a half-written snapshot file must not boot a half-populated pod"
    );
}

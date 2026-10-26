use super::*;
use test_store::{call, response_bytes};

#[tokio::test]
async fn quota_refusals_are_indistinguishable() {
    let (state, _) = test_store::state(1000).await;
    let x = "https://x.example/";
    {
        let mut requests = state.access.requests.write().unwrap();
        for i in 0..MAX_REQUESTS {
            let id = format!("r{i:04}");
            requests.insert(
                id.clone(),
                Record {
                    id,
                    document: Value::Null,
                    policies: Vec::new(),
                    author: Some(if i < 16 {
                        x.to_string()
                    } else {
                        format!("https://p{i}.example/")
                    }),
                    etag: new_etag(),
                    quarantined: false,
                },
            );
        }
    }
    let body = access_doc("AccessRequest", "https://a/", None);
    let own = response_bytes(call(&state, "POST", REQUESTS_PATH, Some(x), &body).await).await;
    let full = response_bytes(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://new.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(own.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(own, full);
    assert!(String::from_utf8(own.2.to_vec())
        .unwrap()
        .contains("this agent may not hold more open access requests now"));
}

#[tokio::test]
async fn quarantined_requests_take_no_place_from_others() {
    let (state, store) = test_store::state(1000).await;
    let container = state.cfg.absolute(REQUESTS_PATH);
    for i in 0..3 {
        store
            .create_in_container(
                &container,
                &format!("{container}bad{i}"),
                Bytes::from_static(b"bad"),
                LWS_JSON,
            )
            .await
            .unwrap();
    }
    state
        .access
        .replace(AccessStore::load(&store, &state.cfg).await.unwrap());
    let body = access_doc("AccessRequest", "https://a/", None);
    for i in 0..MAX_REQUESTS {
        let who = format!("https://p{i}.example/");
        assert_eq!(
            call(&state, "POST", REQUESTS_PATH, Some(&who), &body)
                .await
                .status(),
            StatusCode::CREATED,
            "request {i}"
        );
    }
    assert_eq!(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://last.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn quarantined_requests_take_no_place_from_the_anonymous_principal() {
    let (state, _) = test_store::state(100).await;
    for i in 0..MAX_REQUESTS_PER_AUTHOR {
        let id = format!("bad{i}");
        state.access.requests.write().unwrap().insert(
            id.clone(),
            Record {
                id,
                document: Value::Null,
                policies: Vec::new(),
                author: None,
                etag: new_etag(),
                quarantined: true,
            },
        );
    }
    let body = access_doc("AccessRequest", "https://a/", None);
    for _ in 0..MAX_REQUESTS_PER_AUTHOR {
        assert_eq!(
            call(&state, "POST", REQUESTS_PATH, None, &body)
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    assert_eq!(
        call(&state, "POST", REQUESTS_PATH, None, &body)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn one_principal_cannot_take_every_settlement_place() {
    use crate::lws::records::MAX_SETTLING_PER_PRINCIPAL;
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::state(100).await;
    let body = access_doc("AccessGrant", "https://a/", None);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let x = "https://x.example/";
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            call(&state, "POST", GRANTS_PATH, Some(x), &body)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    let principal_full =
        response_bytes(call(&state, "POST", GRANTS_PATH, Some(x), &body).await).await;
    assert_eq!(principal_full.0, StatusCode::SERVICE_UNAVAILABLE);
    *store.unsettle_creates.lock().unwrap() = None;
    assert_eq!(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            Some("https://y.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    // Exercise the shared limit using the same real route and compare the full refusal.
    let mut places = Vec::new();
    for i in 0..(crate::lws::records::MAX_SETTLING - MAX_SETTLING_PER_PRINCIPAL) {
        let principal = format!("https://other{i}.example/");
        places.push(state.settlements.reserve(Some(&principal)).unwrap());
    }
    let shared_full = response_bytes(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            Some("https://z.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(principal_full, shared_full);
    drop(places);
    store.fail_fence.store(false, Ordering::SeqCst);
    eventually("all principal places returned", || {
        state.settlements.pending() == 0
    })
    .await;
    assert_eq!(
        call(&state, "POST", GRANTS_PATH, Some(x), &body)
            .await
            .status(),
        StatusCode::CREATED
    );
    gate.add_permits(MAX_SETTLING_PER_PRINCIPAL);
}

#[tokio::test]
async fn one_principal_cannot_take_every_settlement_place_with_removals() {
    use crate::lws::records::MAX_SETTLING_PER_PRINCIPAL;
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::state(100).await;
    let body = access_doc("AccessGrant", "https://a/", None);
    let x = "https://x.example/";
    let mut iris = Vec::new();
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        let response = call(&state, "POST", GRANTS_PATH, Some(x), &body).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        iris.push(
            response.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .to_string(),
        );
    }
    store.fail_fence.store(true, Ordering::SeqCst);
    for iri in iris {
        *store.unsettle_next_delete_of.lock().unwrap() =
            Some((iri.clone(), Arc::new(tokio::sync::Semaphore::new(0))));
        let path = iri.strip_prefix(&state.cfg.base_url).unwrap();
        assert_eq!(
            call(&state, "DELETE", path, Some(x), "").await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    assert_eq!(
        call(&state, "POST", GRANTS_PATH, Some(x), &body)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            Some("https://y.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    store.fail_fence.store(false, Ordering::SeqCst);
}

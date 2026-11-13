//! Disposable loopback fixture server for conformance/ldp.sh.
//! Uses the library router, in-memory storage and explicit fixture ACLs.

use std::error::Error;

use axum::body::Bytes;
use solid_oidc_verifier::config::{StaticJwksProvider, VerifierConfig};
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};

const BASE: &str = "http://127.0.0.1:3001";
type FixtureStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

async fn fixture() -> Result<axum::Router, Box<dyn Error>> {
    let store = FixtureStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    store
        .write(&format!("{BASE}/"), Bytes::new(), "text/turtle")
        .await?;
    for level in ["basic", "direct"] {
        let container = format!("{BASE}/{level}/");
        // The Direct fixture declares the requested membership configuration. The
        // current router advertises BasicContainer; the Direct suite probes that
        // gap without adding membership semantics to the server under test.
        let body = if level == "direct" {
            format!(
                "@prefix ldp: <http://www.w3.org/ns/ldp#>. \
                     <{container}> a ldp:DirectContainer; ldp:membershipResource <{container}>; \
                     ldp:hasMemberRelation <urn:ldp-conformance:member>."
            )
        } else {
            format!("<{container}> a <http://www.w3.org/ns/ldp#BasicContainer>.")
        };
        store
            .create_in_container(&format!("{BASE}/"), &container, body.into(), "text/turtle")
            .await?;
        let acl = format!(
            "@prefix acl: <http://www.w3.org/ns/auth/acl#>. \
            <{container}.acl#fixture> a acl:Authorization; \
            acl:agentClass <http://xmlns.com/foaf/0.1/Agent>; \
            acl:accessTo <{container}>; acl:default <{container}>; \
            acl:mode acl:Read, acl:Write, acl:Append."
        );
        store
            .write(&format!("{container}.acl"), acl.into(), "text/turtle")
            .await?;
    }
    let verifier = Verifier::new(
        VerifierConfig::new(vec!["https://issuer.example".into()], BASE),
        StaticJwksProvider::new(),
        InMemoryReplayStore::with_window(std::time::Duration::from_secs(300)),
    )?;
    Ok(build_router(AppState::new(
        AuthContext::new(verifier, BASE),
        LdpState::new(store, BASE),
    )))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args_os().len() != 1 {
        return Err("Usage: ldp_conformance".into());
    }
    let app = fixture().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3001").await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn fixture_serves_both_configurations_through_real_router() {
        let app = fixture().await.unwrap();
        for level in ["basic", "direct"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/{level}/"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            assert!(response
                .headers()
                .get_all("link")
                .iter()
                .any(|h| h.to_str().unwrap().contains("ldp#BasicContainer")));
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let text = String::from_utf8(body.to_vec()).unwrap();
            if level == "direct" {
                assert!(text.contains("urn:ldp-conformance:member"));
            }
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/{level}/"))
                        .header("content-type", "text/turtle")
                        .body(Body::from("<> <urn:predicate> <urn:object>."))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 201);
        }
    }
}

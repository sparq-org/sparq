//! Disposable loopback fixture and native EARL merger for conformance/ldp.sh.
//! Uses the library router, in-memory storage and explicit fixture ACLs.

use std::error::Error;
use std::path::Path;

use axum::body::Bytes;
use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple};
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

fn scope_node(node: NamedOrBlankNode, scope: usize) -> NamedOrBlankNode {
    match node {
        NamedOrBlankNode::BlankNode(b) => BlankNode::new(format!("report{scope}_{}", b.as_str()))
            .unwrap()
            .into(),
        other => other,
    }
}

fn scope_term(term: Term, scope: usize) -> Term {
    match term {
        Term::BlankNode(b) => BlankNode::new(format!("report{scope}_{}", b.as_str()))
            .unwrap()
            .into(),
        Term::Triple(t) => Term::Triple(Box::new(scope_triple(*t, scope))),
        other => other,
    }
}

fn scope_triple(triple: Triple, scope: usize) -> Triple {
    Triple::new(
        scope_node(triple.subject, scope),
        triple.predicate,
        scope_term(triple.object, scope),
    )
}

fn merge_reports(paths: &[String]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut writer = oxttl::TurtleSerializer::new().for_writer(Vec::new());
    for (scope, path) in paths.iter().enumerate() {
        let mut count = 0;
        for triple in oxttl::TurtleParser::new().for_reader(std::fs::File::open(path)?) {
            let triple = scope_triple(triple?, scope);
            if triple.predicate.as_str() == "http://www.w3.org/ns/earl#outcome" {
                count += 1;
            }
            writer.serialize_triple(&triple)?;
        }
        if count == 0 {
            return Err(format!("no EARL outcomes in {path}").into());
        }
        println!("{path}: {count} EARL outcomes");
    }
    Ok(writer.finish()?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("merge") && args.len() == 4 {
        let bytes = merge_reports(&args[2..])?;
        std::fs::write(Path::new(&args[1]), bytes)?;
        return Ok(());
    }
    if !args.is_empty() {
        return Err("Usage: ldp_conformance [merge OUTPUT BASIC DIRECT]".into());
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

    #[test]
    fn report_scopes_keep_equal_blank_labels_distinct() {
        let triple = Triple::new(
            BlankNode::new("same").unwrap(),
            oxrdf::NamedNode::new("urn:p").unwrap(),
            BlankNode::new("same").unwrap(),
        );
        let first = scope_triple(triple.clone(), 0);
        let second = scope_triple(triple, 1);
        assert_ne!(first.subject, second.subject);
        assert_eq!(Term::from(first.subject), first.object);
        assert_eq!(Term::from(second.subject), second.object);
    }

    #[test]
    fn native_reports_merge_without_losing_failures_or_aliasing_blank_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for (level, outcome) in [("basic", "passed"), ("direct", "failed")] {
            let path = dir.path().join(format!("{level}.ttl"));
            std::fs::write(
                &path,
                format!(
                    "@prefix earl: <http://www.w3.org/ns/earl#>. \
                 _:a a earl:Assertion; earl:test <urn:{level}>; earl:result _:r. \
                 _:r earl:outcome earl:{outcome}."
                ),
            )
            .unwrap();
            paths.push(path.to_str().unwrap().to_owned());
        }
        let bytes = merge_reports(&paths).unwrap();
        let triples: Vec<_> = oxttl::TurtleParser::new()
            .for_slice(&bytes)
            .map(Result::unwrap)
            .collect();
        let results: Vec<_> = triples
            .iter()
            .filter(|t| t.predicate.as_str() == "http://www.w3.org/ns/earl#result")
            .collect();
        assert_eq!(results.len(), 2);
        assert_ne!(results[0].object, results[1].object);
        for outcome in ["passed", "failed"] {
            assert!(triples.iter().any(|t| t.object
                == Term::NamedNode(
                    oxrdf::NamedNode::new(format!("http://www.w3.org/ns/earl#{outcome}")).unwrap()
                )));
        }
        std::fs::write(&paths[0], "<urn:x> <urn:y> <urn:z>.").unwrap();
        assert!(merge_reports(&paths).is_err(), "empty report must fail");
        std::fs::write(&paths[0], "broken Turtle").unwrap();
        assert!(merge_reports(&paths).is_err(), "invalid report must fail");
    }
}

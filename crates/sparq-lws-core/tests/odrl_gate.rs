// AUTHORED-BY Claude Sonnet 4.6
//! sq-elg47 — the ODRL gate seam (`authz::odrl`, the `odrl-authz` feature), end-to-end through
//! the assembled router: the follow-up `tests/odrl_query_enforcement.rs` filed ("`sparq-lws-core`
//! today has NO reach to the ODRL bridge … a src seam" — now it does). An ODRL policy attached at
//! router assembly ([`LdpState::set_odrl_gate`]) gates the read path natively, superseding the
//! WAC-only graph-granularity contract:
//!
//! 1. **Deny-overrides**: on a WAC-public doc the policy targets, the prohibited (and every
//!    non-granted) party is DENIED — the static WAC grant does not survive the policy; anonymous
//!    keeps the 401 challenge; no row leaks into a denial.
//! 2. **Permit-extends**: on a WAC-private doc, the policy's permitted party reads — the ODRL
//!    permission is a native grant, not a WAC translation.
//! 3. **Graph granularity**: a doc the policy does NOT target is untouched (WAC alone decides).
//! 4. **Unattached gate ⇒ unchanged**: the feature compiled in but no gate attached is
//!    behaviour-identical to the default build.
//!
//! Runs on the in-memory store doubles (no `embedded-sparq` needed — the seam composes at the
//! decision layer, not the backend).

#![cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, Request, StatusCode};
use common::{jwks_provider, mint_dpop_proof, KeyKit, BASE_URL, CLIENT_ID, ISSUER, WEBID};
use serde_json::json;
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::authz::odrl::PolicyOdrlGate;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use tower::ServiceExt;

/// The pod owner (`common::WEBID`) — holds the root-default grants.
const OWNER: &str = WEBID;
/// Requester A — the party the ODRL policy PERMITS.
const REQ_A: &str = "https://agents.example/req-a#me";
/// Requester B — a distinct authenticated party the policy PROHIBITS.
const REQ_B: &str = "https://agents.example/req-b#me";

/// A WAC-PUBLIC doc the policy targets (deny-overrides is observed here).
const PUB_DOC: &str = "https://pod.example/alice/pub/doc";
/// A WAC-PUBLIC doc the policy does NOT target (graph granularity: WAC alone decides).
const PUB_OTHER: &str = "https://pod.example/alice/pub/other";
/// A WAC-PRIVATE (owner-only) doc the policy targets (permit-extends is observed here).
const PRIV_DOC: &str = "https://pod.example/alice/priv/doc";

/// Distinctive row so a leak into a denial body is caught by substring assertion.
const SECRET_MARKER: &str = "odrl-gate-row-7c2e";

/// The ODRL policy: permit A (read) on both targeted docs; prohibit B (read) on the public one.
fn policy_turtle() -> String {
    format!(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:sparq:policy/odrl-gate-test> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <{PUB_DOC}> ; odrl:assignee <{REQ_A}> ] ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <{PRIV_DOC}> ; odrl:assignee <{REQ_A}> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <{PUB_DOC}> ; odrl:assignee <{REQ_B}> ] .
"#
    )
}

/// Mint a well-formed RFC-9068 access token for an ARBITRARY WebID (the `common` helper
/// hard-codes the owner's) — the same shape as `tests/odrl_query_enforcement.rs`.
fn mint_access_token_for(issuer_key: &KeyKit, cnf_jkt: &str, webid: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = json!({
        "iss": ISSUER,
        "sub": webid,
        "jti": format!("at-odrl-gate-{}", TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": webid,
        "cnf": { "jkt": cnf_jkt },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

struct Requester {
    key: KeyKit,
    webid: &'static str,
}

impl Requester {
    fn new(webid: &'static str) -> Self {
        Self {
            key: KeyKit::generate(),
            webid,
        }
    }
}

struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
}

impl Harness {
    /// Assemble the full router (auth → WAC → ODRL gate → LDP) over the in-memory store doubles.
    /// `gate` is the seam under test: `None` builds the feature-on-but-unattached configuration.
    async fn new(gate: Option<Arc<PolicyOdrlGate>>) -> Self {
        let issuer_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);

        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        seed_fixtures(&store).await;

        let mut ldp = LdpState::new(store, BASE_URL);
        if let Some(gate) = gate {
            ldp.set_odrl_gate(gate);
        }
        Self {
            app: build_router(AppState::new(ctx, ldp)),
            issuer_key,
        }
    }

    async fn get_as(&self, who: &Requester, path: &str) -> axum::http::Response<Body> {
        let access = mint_access_token_for(&self.issuer_key, &who.key.thumbprint, who.webid);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&who.key, "GET", &htu, &access);
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", format!("DPoP {access}"))
            .header("dpop", proof)
            .body(Body::empty())
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }

    async fn get_anon(&self, path: &str) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }
}

/// Seed the docs + their WAC state at the store seam (pre-router, like the sibling test files):
/// the root grants the owner everything; `pub/*` docs are WAC-public-read; `priv/doc` is
/// owner-only. Every doc carries the distinctive marker row.
async fn seed_fixtures(store: &CompositeStore<InMemorySparqClient, InMemoryBlobStore>) {
    let root = format!("{BASE_URL}/");
    let write_ttl = |iri: &'static str, body: String| async move {
        store
            .write(iri, Bytes::from(body), "text/turtle")
            .await
            .expect(iri);
    };
    write_ttl(
        "https://pod.example/.acl",
        format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{OWNER}>;
  acl:accessTo <{root}>; acl:default <{root}>;
  acl:mode acl:Read, acl:Write, acl:Control."#
        ),
    )
    .await;
    for (doc, acl, public) in [
        (PUB_DOC, "https://pod.example/alice/pub/doc.acl", true),
        (PUB_OTHER, "https://pod.example/alice/pub/other.acl", true),
        (PRIV_DOC, "https://pod.example/alice/priv/doc.acl", false),
    ] {
        write_ttl(
            doc,
            format!("<{doc}#it> <http://xmlns.com/foaf/0.1/name> \"{SECRET_MARKER}\" ."),
        )
        .await;
        let agent_rule = if public {
            "acl:agentClass <http://xmlns.com/foaf/0.1/Agent>".to_string()
        } else {
            format!("acl:agent <{OWNER}>")
        };
        write_ttl(
            acl,
            format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#read> a acl:Authorization; {agent_rule}; acl:accessTo <{doc}>; acl:mode acl:Read."#
            ),
        )
        .await;
    }
}

async fn body_text(resp: axum::http::Response<Body>) -> String {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn gate() -> Arc<PolicyOdrlGate> {
    Arc::new(PolicyOdrlGate::from_turtle(&policy_turtle()).expect("admissible policy"))
}

/// Deny-overrides: on the WAC-public doc the policy targets, the permitted party reads, the
/// prohibited party gets a 403 DESPITE the static public grant (no row leaked), and anonymous
/// keeps the 401 challenge. Same doc, same WAC state — the differential is the policy's doing.
#[tokio::test]
async fn odrl_deny_overrides_wac_public_grant() {
    let h = Harness::new(Some(gate())).await;
    let req_a = Requester::new(REQ_A);
    let req_b = Requester::new(REQ_B);

    let resp_a = h.get_as(&req_a, "/alice/pub/doc").await;
    assert_eq!(resp_a.status(), StatusCode::OK, "permitted party reads");
    assert!(body_text(resp_a).await.contains(SECRET_MARKER));

    let resp_b = h.get_as(&req_b, "/alice/pub/doc").await;
    assert_eq!(
        resp_b.status(),
        StatusCode::FORBIDDEN,
        "the ODRL prohibition beats the static WAC public grant"
    );
    assert!(!body_text(resp_b).await.contains(SECRET_MARKER));

    let resp_anon = h.get_anon("/alice/pub/doc").await;
    assert_eq!(
        resp_anon.status(),
        StatusCode::UNAUTHORIZED,
        "a targeted graph denies anonymous (401 + challenge), even under a WAC public grant"
    );
    assert!(resp_anon.headers().contains_key(header::WWW_AUTHENTICATE));
    assert!(!body_text(resp_anon).await.contains(SECRET_MARKER));
}

/// Permit-extends: on the WAC-private doc, the ODRL permission admits the permitted party
/// natively (WAC alone grants them nothing); every other party stays denied.
#[tokio::test]
async fn odrl_permit_admits_read_wac_alone_denies() {
    let h = Harness::new(Some(gate())).await;
    let req_a = Requester::new(REQ_A);
    let req_b = Requester::new(REQ_B);

    let resp_a = h.get_as(&req_a, "/alice/priv/doc").await;
    assert_eq!(
        resp_a.status(),
        StatusCode::OK,
        "the ODRL permission is a native grant — no WAC translation involved"
    );
    assert!(body_text(resp_a).await.contains(SECRET_MARKER));

    let resp_b = h.get_as(&req_b, "/alice/priv/doc").await;
    assert_eq!(resp_b.status(), StatusCode::FORBIDDEN);
    assert!(!body_text(resp_b).await.contains(SECRET_MARKER));
}

/// Graph granularity: a doc the policy does NOT target is untouched — WAC alone decides, for
/// every requester (the gate answers NotApplicable, never a blanket behaviour change).
#[tokio::test]
async fn untargeted_graph_is_untouched() {
    let h = Harness::new(Some(gate())).await;
    let req_b = Requester::new(REQ_B);

    let resp_b = h.get_as(&req_b, "/alice/pub/other").await;
    assert_eq!(resp_b.status(), StatusCode::OK);
    assert!(body_text(resp_b).await.contains(SECRET_MARKER));

    let resp_anon = h.get_anon("/alice/pub/other").await;
    assert_eq!(resp_anon.status(), StatusCode::OK, "public WAC grant stands");
}

/// Feature-on-but-unattached: with no gate attached the behaviour is identical to the default
/// build — the prohibited party's read of the public doc succeeds on the WAC grant alone.
#[tokio::test]
async fn unattached_gate_leaves_behaviour_unchanged() {
    let h = Harness::new(None).await;
    let req_b = Requester::new(REQ_B);

    let resp_b = h.get_as(&req_b, "/alice/pub/doc").await;
    assert_eq!(resp_b.status(), StatusCode::OK);
    assert!(body_text(resp_b).await.contains(SECRET_MARKER));
}

/// A prohibition on a collection governs its members over LDP: the WAC-public doc stated
/// `odrl:partOf` the collection is refused to the prohibited party, while a doc outside the
/// collection stays public to them.
#[tokio::test]
async fn collection_rules_govern_member_reads() {
    let coll = "https://pod.example/alice/pub/";
    let policy = format!(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol> a odrl:Set ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <{coll}> ; odrl:assignee <{REQ_B}> ] .
<{PUB_DOC}> odrl:partOf <{coll}> .
"#
    );
    let gate = Arc::new(PolicyOdrlGate::from_turtle(&policy).expect("admissible policy"));
    let h = Harness::new(Some(gate)).await;
    let req_b = Requester::new(REQ_B);

    let member = h.get_as(&req_b, "/alice/pub/doc").await;
    assert_eq!(member.status(), StatusCode::FORBIDDEN, "the member is governed");
    assert!(!body_text(member).await.contains(SECRET_MARKER));

    let outside = h.get_as(&req_b, "/alice/pub/other").await;
    assert_eq!(outside.status(), StatusCode::OK, "a doc outside the collection is untouched");
}

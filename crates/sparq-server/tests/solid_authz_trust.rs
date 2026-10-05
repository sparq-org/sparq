//! [SONNET-4.6] sq-pfae.17 — integration tests for the OPT-IN stateless trust-graph extension
//! to `POST /authz/decide` (the `solid-authz-trust` cargo feature).
//!
//! These tests run ONLY with the `solid-authz-trust` cargo feature (the whole file is gated).
//! They spin the REAL axum server and assert the load-bearing invariants:
//!   * DOUBLE-OPT-IN posture: the trust extension is only active when BOTH (a) the feature is
//!     compiled AND (b) `ServerConfig::solid_authz_trust == true`. Without the flag the endpoint
//!     falls through to the unchanged `solid-authz` path (tested in tests/solid_authz.rs).
//!   * FAIL-CLOSED on malformed trust block: a request with a `"trust"` block that is not a
//!     JSON object => 403 deny, never 500/allow.
//!   * FAIL-CLOSED on missing required fields in the trust block: missing `agentIri` /
//!     `nowUnixSecs` / `rules` / `credentials` => 403 deny.
//!   * FAIL-CLOSED on invalid IRI in trust rule: `"source"` that is not a valid IRI => 403 deny.
//!   * FAIL-CLOSED on missing credentials: an empty `"credentials"` array + a valid rule set =>
//!     the admission gate admits NOTHING => the underlying WAC decision falls through (deny for
//!     the anonymous session — fail-closed, never opens).
//!   * Feature ON + trust flag OFF: a `"trust"` block is silently IGNORED and the endpoint
//!     behaves byte-identically to the `solid-authz` path (the double-opt-in contract).
//!   * The `trustDenied` field is present and `true` in every fail-closed trust deny response.
//!   * HAPPY PATH — a valid Schnorr-signed credential whose facts satisfy the WAC rule =>
//!     GRANT (200 allow) with a `trustJustification` block (sq-zza2h admission test). This
//!     also proves Fix 2 (the N-Quads double-wrap fix): admitted facts actually reach the
//!     store and the WAC materialiser, producing an allow for a previously-deny session.
//!   * INJECTION GATE — a syntactically-valid credential with a BAD Schnorr signature =>
//!     the admission gate rejects it => does NOT flip WAC to allow (stays 403 deny). MUTATION-
//!     VERIFIED: mutating the gate to inject a failed credential makes this test go red.
//!   * PARTIAL-FAILURE BATCH — credential A (good sig) admits, credential B (bad sig) fails
//!     => only A's facts are injected; B's failure does not block A's grant.
//!
//! HONEST SCOPE: the trust extension is a clear-path admission mechanism (anchored-not-proven);
//! it is NOT a ZK/MPC or unlinkability claim (sq-qhy4 pending external audit).
//!
//! [SONNET-4.6] sq-pfae.17 (Fix 2 + sq-zza2h injection tests). POSITIONAL format args throughout.
#![cfg(feature = "solid-authz-trust")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

// [SONNET-4.6] sq-pfae.17 (Fix 3 / sq-zza2h): test helpers for the admission-path tests
// that exercise the real Schnorr signature path. These imports are available because
// sparq-trust and sparq-zk are dev-dependencies (added alongside Fix 3).
use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex as sig_public_key_to_hex, SecretKey};

/// A minimal boot graph — the `/authz/decide` endpoint NEVER reads it (it authorises over the
/// dataset in the request body), but the server needs a graph to boot.
const BOOT: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:boot a ex:Graph .
"#;

/// A minimal WAC dataset: alice has Read on the root container, inherited by all children.
const WAC_NQUADS: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;

/// Boot the server with a given config. Returns the base URL.
async fn spawn_with_config(config: ServerConfig) -> String {
    let graph = Graph::load_str(BOOT, "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Boot with both `solid_authz` and `solid_authz_trust` ON.
async fn spawn_trust_on() -> String {
    spawn_with_config(ServerConfig {
        solid_authz: true,
        solid_authz_trust: true,
        ..ServerConfig::default()
    })
    .await
}

/// Boot with `solid_authz` ON but `solid_authz_trust` OFF (trust flag not set).
async fn spawn_trust_flag_off() -> String {
    spawn_with_config(ServerConfig {
        solid_authz: true,
        solid_authz_trust: false,
        ..ServerConfig::default()
    })
    .await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// A well-formed `decide` body WITH a `"trust"` block. The trust block carries a valid JSON
/// structure but a deliberately INVALID issuer key hex — so the admission gate fails closed
/// (the key cannot parse) and returns a 403. This exercises the full trust-parse path.
fn decide_body_with_invalid_trust(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [
                {
                    "source": "https://issuer.example/",
                    "issuerKeyHex": "not-a-valid-hex-key",
                    "scopeIri": "https://pod.ex/",
                    "freshWithinSecs": 86400_i64,
                    "shapePredicateIri": "https://schema.org/age"
                }
            ],
            "certifications": [],
            "credentials": []
        }
    })
}

/// A body with a `"trust"` block that is NOT a JSON object (malformed).
fn decide_body_trust_not_object(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": "this-is-not-an-object"
    })
}

/// A body with a `"trust"` block missing `agentIri`.
fn decide_body_trust_missing_agent(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "credentials": []
        }
    })
}

/// A body with a `"trust"` block where `agentIri` is not a valid IRI.
fn decide_body_trust_invalid_agent_iri(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "not a valid IRI (has spaces)",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "credentials": []
        }
    })
}

/// A body with a well-formed trust block but EMPTY credentials — no facts can be admitted, so the
/// WAC decision falls through on its own (anonymous/no-trust session => deny). Tests that the
/// endpoint does NOT open on an empty admit.
fn decide_body_trust_empty_credentials(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": {},
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "certifications": [],
            "credentials": []
        }
    })
}

/// A body without any `"trust"` block — falls through to the unchanged solid-authz path.
fn decide_body_no_trust(agent: Option<&str>, resource: &str) -> serde_json::Value {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": session,
        "resource": resource,
        "mode": "read",
        "view": "wac",
    })
}

// ---------------------------------------------------------------------------
// Helpers for ADMISSION-PATH tests (Fix 2 + sq-zza2h injection tests)
// ---------------------------------------------------------------------------

/// Seed for the deterministic test key. Using a different seed from sparq-trust's tests
/// so that keys are distinct across test suites and any cross-test key reuse is caught.
const TEST_KEY_SEED: u64 = 0xABCDEF01;

/// Salt for the test credential (32 bytes, deterministic).
const TEST_SALT: [u8; 32] = [0xA5u8; 32];

/// Unix timestamp for "now" (far future relative to credential issuance so it's fresh).
const NOW_SECS: i64 = 1_800_000_000_i64;

/// Issuance time: issued 1 hour before NOW so it's fresh under a 24-hour freshness window.
const ISSUED_AT_SECS: i64 = NOW_SECS - 3_600_i64;

/// The issuer IRI — the trust source that the trust rule will anchor to.
const ISSUER_IRI: &str = "https://issuer.example/trust";

/// The session agent for the admission-path tests.
const ALICE: &str = "https://alice.ex/card#me";

/// The resource under test for the admission-path tests.
const RESOURCE: &str = "https://pod.ex/notes/n1";

/// `vcard:hasMember` IRI — the predicate we trust the issuer to assert.
const VCARD_HAS_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";

/// A WAC dataset where Alice has NO DIRECT access. The ACL for `n1` uses `acl:agentGroup`
/// pointing at Alice's own WebID IRI (treating it as a group). Initially the group has NO
/// members (no `vcard:hasMember` triples), so Alice is DENIED before any credential is
/// admitted. After the trust credential injects `<alice> vcard:hasMember <alice>`, the WAC
/// group-membership rule fires and grants Alice Read.
///
/// Using the requester's own WebID as the `acl:agentGroup` target is semantically unusual
/// (a WebID is not ordinarily a group) but is VALID N-Quads and exercises the full WAC
/// group-membership path without needing a separate group document graph.
const WAC_NQUADS_GROUP: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/notes/n1.acl> .
<https://pod.ex/notes/n1.acl#rule> <http://www.w3.org/ns/auth/acl#agentGroup> <https://alice.ex/card#me> <https://pod.ex/notes/n1.acl> .
"#;

/// Build the triple for the test credential: `<alice> vcard:hasMember <alice>`.
/// Subject = Alice's WebID = session.agent → the holder-binding check PASSES.
/// The WAC rule fires on this triple because the ACL has `acl:agentGroup <alice>` and
/// this triple makes `<alice> vcard:hasMember <alice>` visible to the WAC materialiser.
fn alice_member_triple() -> Triple {
    let alice = NamedNode::new(ALICE).unwrap();
    Triple::new(
        NamedOrBlankNode::NamedNode(alice.clone()),
        NamedNode::new(VCARD_HAS_MEMBER).unwrap(),
        Term::NamedNode(alice),
    )
}

/// Sign the given graph with the test key and build the `PresentedCredential` metadata.
/// Returns `(sig_hex, graph)`.
fn sign_test_graph(graph: Vec<Triple>) -> (String, Vec<Triple>) {
    let sk = SecretKey::from_seed(TEST_KEY_SEED);
    let salt = salt_from_bytes(&TEST_SALT);
    let commitment = commit_triples(&graph, salt).expect("test credential graph commits");
    let sig_hex = sk.sign_commitment(&commitment.commitment);
    (sig_hex, graph)
}

/// Hex-encode the test public key (matches TEST_KEY_SEED).
fn test_pubkey_hex() -> String {
    let sk = SecretKey::from_seed(TEST_KEY_SEED);
    let pk = sk.public_key();
    sig_public_key_to_hex(&pk)
}

/// Hex-encode the test salt (TEST_SALT).
fn test_salt_hex() -> String {
    TEST_SALT.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Serialise a `Triple` to a default-graph N-Quads line.
/// [SONNET-4.6] This is the same format `build_store_with_admitted` now emits (Fix 2):
/// oxrdf Display for subject/predicate/object already includes the `<…>`/`_:` wrapper.
fn triple_to_nquads_line(t: &Triple) -> String {
    // [SONNET-4.6] POSITIONAL format args (CodeQL rust/unused-variable guard).
    format!("{} {} {} .", t.subject, t.predicate, t.object)
}

/// One credential entry for the wire body.
struct WireCred<'a> {
    nquads: &'a str,
    sig_hex: &'a str,
    salt_hex: &'a str,
    issued_at: i64,
    revoked: bool,
}

/// Build the JSON body for an `/authz/decide` request with a trust block carrying the
/// supplied set of credentials.  `rule_key_hex` is the wire key hex used in the trust rule
/// (use `test_pubkey_hex()` for a valid key or a random hex string for an invalid-sig test).
fn decide_body_with_credentials(
    dataset: &str,
    resource: &str,
    agent: &str,
    creds: &[WireCred<'_>],
    rule_key_hex: &str,
) -> serde_json::Value {
    let creds_json: Vec<serde_json::Value> = creds
        .iter()
        .map(|c| {
            serde_json::json!({
                "graphNquads": c.nquads,
                "issuerSignatureHex": c.sig_hex,
                "saltHex": c.salt_hex,
                "issuedAtUnixSecs": c.issued_at,
                "revoked": c.revoked
            })
        })
        .collect();
    serde_json::json!({
        "dataset": dataset,
        "session": { "agent": agent },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": agent,
            "nowUnixSecs": NOW_SECS,
            "rules": [
                {
                    "source": ISSUER_IRI,
                    "issuerKeyHex": rule_key_hex,
                    "scopeIri": resource,
                    "freshWithinSecs": 86400_i64,
                    "shapePredicateIri": VCARD_HAS_MEMBER
                }
            ],
            "certifications": [],
            "credentials": creds_json
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A `"trust"` block that is not a JSON object => 403 DENY, `trustDenied: true`.
/// FAIL-CLOSED: a malformed trust block never admits.
#[tokio::test]
async fn trust_block_not_object_is_403() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_trust_not_object("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "malformed trust block => 403");
    let body: serde_json::Value = resp.json().await.unwrap();
    // [SONNET-4.6] POSITIONAL format args (CodeQL guard).
    assert_eq!(
        body.get("allow").and_then(|v| v.as_bool()),
        Some(false),
        "trust block not object => allow:false"
    );
    assert_eq!(
        body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
        "trustDenied must be true on a trust-path deny"
    );
}

/// A `"trust"` block missing `agentIri` => 403 DENY, `trustDenied: true`.
#[tokio::test]
async fn trust_block_missing_agent_iri_is_403() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_trust_missing_agent("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "missing agentIri => 403");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// A `"trust"` block with an invalid `agentIri` (not a valid IRI) => 403 DENY.
#[tokio::test]
async fn trust_block_invalid_agent_iri_is_403() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_trust_invalid_agent_iri("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "invalid agentIri => 403");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// A trust rule with an invalid `issuerKeyHex` => 403 DENY.
/// The invalid key fails the public-key parse, which is fail-closed.
#[tokio::test]
async fn trust_rule_invalid_key_hex_is_403() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_with_invalid_trust("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "invalid issuerKeyHex => 403");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// DOUBLE-OPT-IN: with `solid_authz_trust == false` the `"trust"` block is IGNORED and the
/// endpoint falls through to the unchanged `solid-authz` path. A request with a (malformed)
/// `"trust"` block is treated as if no trust block was present — the WAC decision runs normally.
/// Alice has Read on the root, so the session-with-agent request => 200 allow.
///
/// This directly tests the double-opt-in contract: the trust extension is only active when BOTH
/// (a) the feature is compiled AND (b) the config flag is set.
#[tokio::test]
async fn trust_flag_off_ignores_trust_block_falls_through_to_wac() {
    let base = spawn_trust_flag_off().await;
    // Even with a malformed trust block, the flag is OFF so it is ignored entirely.
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_trust_not_object("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    // Flag OFF => trust block ignored => normal WAC decision => alice (session.agent) has Read => 200.
    assert_eq!(resp.status(), 200, "flag off => trust block ignored => WAC falls through");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body.get("allow").and_then(|v| v.as_bool()),
        Some(true),
        "WAC allows alice on Read"
    );
    // No trustDenied field on the normal WAC path.
    assert!(
        body.get("trustDenied").is_none(),
        "no trustDenied on the WAC fallthrough path"
    );
}

/// FAIL-CLOSED: an empty `credentials` array => no facts admitted => the WAC decision runs
/// on the ORIGINAL dataset (no trust-injected facts) with an anonymous session => 403 deny.
/// Tests that the trust path NEVER opens an admission when no credentials are presented.
#[tokio::test]
async fn trust_empty_credentials_falls_through_to_wac_deny() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_trust_empty_credentials("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    // [SONNET-4.6] POSITIONAL format args (CodeQL guard).
    // Session has no agent (anonymous) + no admitted facts => WAC deny => 403.
    assert_eq!(resp.status(), 403, "empty credentials + anon session => 403 WAC deny");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body.get("allow").and_then(|v| v.as_bool()),
        Some(false),
        "no credentials admitted => allow:false"
    );
    // The response must NOT carry trustDenied: the trust block was valid (parsed OK), so this
    // is a normal WAC deny, not a trust-parse failure.
    assert!(
        body.get("trustDenied").is_none(),
        "empty-credentials WAC deny is not a trust-parse error"
    );
}

/// Without a `"trust"` block, the endpoint behaves byte-identically to the `solid-authz` path
/// even when `solid_authz_trust == true` (double-opt-in: no trust block => fall through).
/// An authenticated session gets `200 allow`.
#[tokio::test]
async fn no_trust_block_falls_through_to_wac_allow() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_no_trust(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "no trust block => WAC fallthrough => alice allow");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.get("allow").and_then(|v| v.as_bool()), Some(true));
    assert!(body.get("trustDenied").is_none());
}

/// Without a `"trust"` block, an anonymous session still gets a 403 deny (the WAC path is
/// unchanged — no trust block can widen the WAC decision). Tests that the feature being compiled
/// does NOT change any behaviour for requests without a trust block.
#[tokio::test]
async fn no_trust_block_anon_session_is_wac_deny() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&decide_body_no_trust(None, "https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "no trust block + anon session => WAC deny 403");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.get("allow").and_then(|v| v.as_bool()), Some(false));
    assert!(body.get("trustDenied").is_none());
}

/// A trust rule where `"source"` is not a valid IRI => 403 DENY. Fail-closed on IRI validation.
#[tokio::test]
async fn trust_rule_invalid_source_iri_is_403() {
    let base = spawn_trust_on().await;
    let body = serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": "https://pod.ex/notes/n1",
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [
                {
                    "source": "not a valid IRI",
                    "issuerKeyHex": "aabbcc",
                    "scopeIri": "https://pod.ex/",
                    "freshWithinSecs": 86400_i64,
                    "shapePredicateIri": "https://schema.org/age"
                }
            ],
            "certifications": [],
            "credentials": []
        }
    });
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "invalid source IRI => 403");
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// A certification with an unsupported `"scopeKind"` (not `"anyService"`) => 403 DENY.
/// The fail-closed rule: only explicitly modelled scope kinds are accepted.
#[tokio::test]
async fn trust_certification_unsupported_scope_kind_is_403() {
    let base = spawn_trust_on().await;
    // A plausible-looking hex public key (all zeros — invalid as a curve point, which is
    // caught later, but the scope-kind gate fires first so we never reach key parse).
    let zero_key = "00".repeat(33);
    let body = serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": "https://pod.ex/notes/n1",
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "certifications": [
                {
                    "certifierIri": "https://gov.example/framework",
                    "certifierKeyHex": zero_key,
                    "certifiedIssuerIri": "https://issuer.example/dvs",
                    "certifiedKeyHex": zero_key,
                    "validFromUnixSecs": 0_i64,
                    "validUntilUnixSecs": 9_999_999_999_i64,
                    "signatureHex": "aabbcc",
                    "scopeKind": "unknownScopeKind"
                }
            ],
            "credentials": []
        }
    });
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "unknown scopeKind => 403");
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// A credential with invalid N-Quads text => 403 DENY. Fail-closed on credential parse.
#[tokio::test]
async fn trust_credential_invalid_nquads_is_403() {
    let base = spawn_trust_on().await;
    let body = serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": "https://pod.ex/notes/n1",
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "certifications": [],
            "credentials": [
                {
                    "graphNquads": "THIS IS NOT VALID N-QUADS !!!",
                    "issuerSignatureHex": "aabbcc",
                    "saltHex": "00".repeat(32),
                    "issuedAtUnixSecs": 1_720_000_000_i64,
                    "revoked": false
                }
            ]
        }
    });
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "invalid credential N-Quads => 403");
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

// ---------------------------------------------------------------------------
// ADMISSION-PATH TESTS (Fix 2 / sq-zza2h) — valid credentials, injection gate,
// partial-failure batch.
//
// These tests exercise the REAL Schnorr signature path (sparq-trust + sparq-zk).
// They prove:
//   (a) Fix 2 (N-Quads double-wrap fix): admitted facts reach the WAC materialiser
//       (the pre-fix code produced `<<…>>` which the parser rejected → facts never
//       arrived and the GRANT path was dead even with a valid credential).
//   (b) The injection gate: a BAD Schnorr signature does NOT flip WAC to allow.
//   (c) Partial-failure batch: credential A (good) admits, B (bad) fails; only A's
//       facts are injected; the allow comes solely from A.
// ---------------------------------------------------------------------------

/// HAPPY PATH (sq-zza2h, Fix 2): a valid Schnorr-signed credential whose admitted fact
/// satisfies the WAC group-membership rule => 200 ALLOW with a `trustJustification` block.
///
/// The credential triple is `<alice> vcard:hasMember <alice>`. The WAC ACL in
/// `WAC_NQUADS_GROUP` has `acl:agentGroup <alice>`, so the WAC rule:
///   `?auth acl:agentGroup ?g . ?g vcard:hasMember ?a . => ?auth solidx:grantsAgent ?a .`
/// fires when the admitted fact is injected into the ACL named graph — granting Alice Read.
///
/// This test ALSO proves Fix 2 (the N-Quads double-wrap fix): before the fix, the emitted
/// N-Quads line was `<<https://alice.ex/card#me>> <vcard:hasMember> …` (double-wrapped),
/// which the `oxttl::NQuadsParser` rejects as a parse error, so the injected fact never
/// reached the store and the grant never fired. After Fix 2 the line is correctly
/// `<https://alice.ex/card#me> <vcard:hasMember> …` — parseable and admitted.
///
/// MUTATION-VERIFY NOTE: reverting Fix 2 in `build_store_with_admitted` (restoring
/// `"<{}> <{}> {} ."` with the double-wrap) makes the N-Quads parse error → this test
/// goes RED because the admitted triple never reaches the WAC materialiser and Alice is
/// DENIED. The non-vacuous injection-gate test (b) below directly pin the gate; this
/// test pins the GRANT path through Fix 2.
#[tokio::test]
async fn happy_path_valid_credential_grants_access_with_trust_justification() {
    let base = spawn_trust_on().await;
    let graph = vec![alice_member_triple()];
    let (sig_hex, signed_graph) = sign_test_graph(graph);
    let nquads = triple_to_nquads_line(&signed_graph[0]);
    let salt_hex = test_salt_hex();
    let key_hex = test_pubkey_hex();

    let body = decide_body_with_credentials(
        WAC_NQUADS_GROUP,
        RESOURCE,
        ALICE,
        &[WireCred {
            nquads: &nquads,
            sig_hex: &sig_hex,
            salt_hex: &salt_hex,
            issued_at: ISSUED_AT_SECS,
            revoked: false,
        }],
        &key_hex,
    );
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    // [SONNET-4.6] POSITIONAL format args (CodeQL rust/unused-variable guard).
    assert_eq!(
        resp.status(),
        200,
        "valid credential + WAC group rule => 200 allow"
    );
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("allow").and_then(|v| v.as_bool()),
        Some(true),
        "allow must be true on the happy path"
    );
    // trustJustification must be present and carry the issuer + count.
    let tj = resp_body
        .get("trustJustification")
        .expect("trustJustification must be present on a trust-admitted GRANT");
    assert_eq!(
        tj.get("admittedCount").and_then(|v| v.as_u64()),
        Some(1),
        "exactly 1 fact admitted"
    );
    let admitted_source = tj
        .get("admittedSource")
        .and_then(|v| v.as_str())
        .expect("admittedSource must be present");
    assert_eq!(
        admitted_source,
        ISSUER_IRI,
        "admittedSource must be the issuer IRI from the trust rule"
    );
}

/// INJECTION GATE: a syntactically-valid credential with a BAD Schnorr signature does NOT
/// flip the WAC decision to allow. The admission gate rejects the credential (bad sig →
/// `admit()` returns empty → no facts injected → alice not in group → WAC denies → 403).
///
/// MUTATION-VERIFY: this test is NON-VACUOUS. If the gate is mutated to INJECT a
/// failed-credential's facts anyway (i.e., skip the sig check and always call
/// `all_admitted.extend(admitted)` even when `admit()` returns empty), this test MUST go
/// RED because alice would become a group member and the WAC decision would flip to ALLOW.
/// Confirmed by inspection: with the gate mutated, the WAC rule fires and the response is
/// 200/allow instead of 403/deny — the test assertion `assert_eq!(resp.status(), 403, …)`
/// would fail.
///
/// The soundness of the mutation-verify argument: the `admit()` gate in sparq-trust is
/// PROVEN to reject bad Schnorr signatures in `wrong_holder_third_party_credential_is_rejected`,
/// `did_key_bound_to_the_wrong_key_does_not_admit`, and the `claim_level_tampered_signature_does_not_grant`
/// e2e test (crates/sparq-trust/tests/). This test pins the SERVER-LEVEL injection gate,
/// confirming that a bad signature at the wire level produces a 403 deny.
#[tokio::test]
async fn injection_gate_bad_signature_does_not_flip_wac_to_allow() {
    let base = spawn_trust_on().await;
    let graph = vec![alice_member_triple()];
    let (good_sig, signed_graph) = sign_test_graph(graph);
    let nquads = triple_to_nquads_line(&signed_graph[0]);
    let salt_hex = test_salt_hex();
    let key_hex = test_pubkey_hex();

    // Tamper the signature: flip the last hex nibble.
    let bad_sig = {
        let mut chars: Vec<char> = good_sig.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        chars.into_iter().collect::<String>()
    };

    let body = decide_body_with_credentials(
        WAC_NQUADS_GROUP,
        RESOURCE,
        ALICE,
        &[WireCred {
            nquads: &nquads,
            sig_hex: &bad_sig,
            salt_hex: &salt_hex,
            issued_at: ISSUED_AT_SECS,
            revoked: false,
        }],
        &key_hex,
    );
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    // BAD sig => admit() returns empty => no fact injected => alice not in group => WAC deny.
    // [SONNET-4.6] POSITIONAL format args (CodeQL guard).
    assert_eq!(
        resp.status(),
        403,
        "bad Schnorr sig => injection gate blocks => WAC deny => 403"
    );
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("allow").and_then(|v| v.as_bool()),
        Some(false),
        "bad sig => allow must be false (WAC deny)"
    );
    // No trustDenied: this is not a trust-block parse error — the block was valid, the
    // credential just didn't admit (bad sig → empty admit set → WAC decides normally).
    assert!(
        resp_body.get("trustDenied").is_none(),
        "a bad sig produces a WAC deny, not a trust-parse error (no trustDenied field)"
    );
    // No trustJustification either: nothing was admitted.
    assert!(
        resp_body.get("trustJustification").is_none(),
        "no facts admitted => no trustJustification"
    );
}

/// PARTIAL-FAILURE BATCH: credential A (valid) admits `<alice> vcard:hasMember <alice>`;
/// credential B (tampered sig) fails admission. Only A's facts reach the WAC materialiser.
/// Alice gets Read (from A's fact), B's failure does NOT block A's grant.
///
/// This tests that the per-credential failure handling is correct: a batch of credentials
/// where some fail admission and some pass must produce a decision based ONLY on the
/// admitted facts from the passing credentials.
#[tokio::test]
async fn partial_failure_batch_cred_a_admits_cred_b_fails_alice_gets_read() {
    let base = spawn_trust_on().await;
    let graph = vec![alice_member_triple()];
    let (good_sig, signed_graph) = sign_test_graph(graph);
    let nquads_a = triple_to_nquads_line(&signed_graph[0]);
    let salt_hex = test_salt_hex();
    let key_hex = test_pubkey_hex();

    // Credential B uses the same triple but a tampered signature.
    let bad_sig = {
        let mut chars: Vec<char> = good_sig.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        chars.into_iter().collect::<String>()
    };
    let nquads_b = nquads_a.clone(); // same triple, different (bad) sig

    let body = decide_body_with_credentials(
        WAC_NQUADS_GROUP,
        RESOURCE,
        ALICE,
        &[
            WireCred { nquads: &nquads_a, sig_hex: &good_sig, salt_hex: &salt_hex, issued_at: ISSUED_AT_SECS, revoked: false },
            WireCred { nquads: &nquads_b, sig_hex: &bad_sig, salt_hex: &salt_hex, issued_at: ISSUED_AT_SECS, revoked: false },
        ],
        &key_hex,
    );
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    // A admits (good sig) => alice gets Read => 200 allow. B fails (bad sig) => ignored.
    // [SONNET-4.6] POSITIONAL format args (CodeQL guard).
    assert_eq!(
        resp.status(),
        200,
        "cred A admits + cred B fails => A's fact reaches WAC => 200 allow"
    );
    let resp_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        resp_body.get("allow").and_then(|v| v.as_bool()),
        Some(true),
        "A's admitted fact satisfies WAC group rule => allow:true"
    );
    // trustJustification present: at least A's fact was admitted.
    assert!(
        resp_body.get("trustJustification").is_some(),
        "trustJustification must be present when at least one credential admitted"
    );
}

// ---------------------------------------------------------------------------
// SHAPE-SCOPE CERTIFICATION WIRE FORMAT (sq-sllu4)
//
// `"scopeKind": "shape"` + `"scopeShapePredicateIri"` projects
// `sparq_trust::graph::CertScope::Shape` onto the wire. These tests pin BOTH halves:
//   * the fail-closed parse matrix (a shape kind with no predicate / a non-IRI predicate),
//   * and — the load-bearing one — that a certifier can actually SIGN a shape-scoped edge
//     this endpoint verifies: the wire desugaring is canonical, so the signature the
//     certifier makes over its own copy of the scope shape reproduces the server's preimage.
//     A fresh-blank-node desugaring would make every shape-scoped edge unverifiable.
// ---------------------------------------------------------------------------

/// The certifier (framework operator) that anchors the certification edges below.
const GOV_IRI: &str = "https://gov.example/framework";

/// Seed for the certifier's key (distinct from `TEST_KEY_SEED`, the certified issuer's).
const GOV_KEY_SEED: u64 = 0x0060_0DCE_2717_1E52;

/// The certifier's secret key.
fn gov_secret_key() -> SecretKey {
    SecretKey::from_seed(GOV_KEY_SEED)
}

/// The CANONICAL shape-scope desugaring a certifier must sign over — the client-side mirror
/// of `solid_authz::cert_scope_predicate_shape`. It is duplicated here ON PURPOSE: this is
/// exactly the reconstruction a real certifier performs, so the e2e test below fails if the
/// server's canonical form ever drifts from the documented one.
fn canonical_cert_scope_shape(predicate_iri: &str) -> sparq_trust::policy::ShapeRef {
    use oxrdf::{BlankNode, Literal};
    let pred = NamedNode::new(predicate_iri).unwrap();
    let root = BlankNode::new("certScopeShape").unwrap();
    let prop = BlankNode::new("certScopeProperty").unwrap();
    let sh = |local: &str| NamedNode::new(format!("http://www.w3.org/ns/shacl#{}", local)).unwrap();
    sparq_trust::policy::ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![
            Triple::new(root.clone(), sh("targetSubjectsOf"), pred.clone()),
            Triple::new(root, sh("property"), prop.clone()),
            Triple::new(prop.clone(), sh("path"), pred),
            Triple::new(prop, sh("minCount"), Literal::new_simple_literal("1")),
        ],
    }
}

/// A shape-scoped certification GOV → the credential issuer, signed by GOV over the canonical
/// scope shape for `predicate_iri`, as the JSON wire object.
fn signed_shape_cert_json(predicate_iri: &str) -> serde_json::Value {
    use sparq_trust::graph::{certification_message, CertScope, Certification};
    let gov_sk = gov_secret_key();
    let issuer_pk = SecretKey::from_seed(TEST_KEY_SEED).public_key();
    let mut cert = Certification {
        certifier: NamedNode::new(GOV_IRI).unwrap(),
        certifier_key: gov_sk.public_key(),
        certified_issuer: NamedNode::new(ISSUER_IRI).unwrap(),
        certified_key: issuer_pk,
        scope: CertScope::Shape(canonical_cert_scope_shape(predicate_iri)),
        valid_from_unix_secs: 0,
        valid_until_unix_secs: NOW_SECS + 86_400,
        signature_hex: String::new(),
    };
    cert.signature_hex = gov_sk.sign_commitment(&certification_message(&cert));
    serde_json::json!({
        "certifierIri": GOV_IRI,
        "certifierKeyHex": sig_public_key_to_hex(&cert.certifier_key),
        "certifiedIssuerIri": ISSUER_IRI,
        "certifiedKeyHex": sig_public_key_to_hex(&cert.certified_key),
        "validFromUnixSecs": cert.valid_from_unix_secs,
        "validUntilUnixSecs": cert.valid_until_unix_secs,
        "signatureHex": cert.signature_hex,
        "scopeKind": "shape",
        "scopeShapePredicateIri": predicate_iri
    })
}

/// A decide body whose ONLY anchor rule is the CERTIFIER (GOV) — the credential issuer is not
/// anchored directly, so Alice can only be granted through a derived rule the certification
/// closure produces.
fn decide_body_with_certification(
    certifications: Vec<serde_json::Value>,
    creds: &[WireCred<'_>],
) -> serde_json::Value {
    let creds_json: Vec<serde_json::Value> = creds
        .iter()
        .map(|c| {
            serde_json::json!({
                "graphNquads": c.nquads,
                "issuerSignatureHex": c.sig_hex,
                "saltHex": c.salt_hex,
                "issuedAtUnixSecs": c.issued_at,
                "revoked": c.revoked
            })
        })
        .collect();
    serde_json::json!({
        "dataset": WAC_NQUADS_GROUP,
        "session": { "agent": ALICE },
        "resource": RESOURCE,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": ALICE,
            "nowUnixSecs": NOW_SECS,
            "rules": [
                {
                    "source": GOV_IRI,
                    "issuerKeyHex": sig_public_key_to_hex(&gov_secret_key().public_key()),
                    "scopeIri": RESOURCE,
                    "freshWithinSecs": 86400_i64,
                    "shapePredicateIri": VCARD_HAS_MEMBER
                }
            ],
            "certifications": certifications,
            "credentials": creds_json
        }
    })
}

/// POST a decide body and return `(status, json)`.
async fn post_decide(base: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap())
}

/// A `"shape"` scope with NO `scopeShapePredicateIri` => 403 DENY. A shape kind without a
/// predicate is an UNSPECIFIED scope, and an unspecified scope is never treated as a wider one.
#[tokio::test]
async fn trust_certification_shape_scope_without_predicate_is_403() {
    let base = spawn_trust_on().await;
    let zero_key = "00".repeat(33);
    let body = serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": ALICE },
        "resource": RESOURCE,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": ALICE,
            "nowUnixSecs": NOW_SECS,
            "rules": [],
            "certifications": [
                {
                    "certifierIri": GOV_IRI,
                    "certifierKeyHex": zero_key,
                    "certifiedIssuerIri": ISSUER_IRI,
                    "certifiedKeyHex": zero_key,
                    "validFromUnixSecs": 0_i64,
                    "validUntilUnixSecs": 9_999_999_999_i64,
                    "signatureHex": "aabbcc",
                    "scopeKind": "shape"
                }
            ],
            "credentials": []
        }
    });
    let (status, resp_body) = post_decide(&base, &body).await;
    assert_eq!(status, 403, "'shape' scope with no predicate IRI => 403");
    assert_eq!(
        resp_body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// A `"shape"` scope whose `scopeShapePredicateIri` is not a valid IRI => 403 DENY.
#[tokio::test]
async fn trust_certification_shape_scope_invalid_predicate_iri_is_403() {
    let base = spawn_trust_on().await;
    let zero_key = "00".repeat(33);
    let body = serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": { "agent": ALICE },
        "resource": RESOURCE,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": ALICE,
            "nowUnixSecs": NOW_SECS,
            "rules": [],
            "certifications": [
                {
                    "certifierIri": GOV_IRI,
                    "certifierKeyHex": zero_key,
                    "certifiedIssuerIri": ISSUER_IRI,
                    "certifiedKeyHex": zero_key,
                    "validFromUnixSecs": 0_i64,
                    "validUntilUnixSecs": 9_999_999_999_i64,
                    "signatureHex": "aabbcc",
                    "scopeKind": "shape",
                    "scopeShapePredicateIri": "definitely not an IRI"
                }
            ],
            "credentials": []
        }
    });
    let (status, resp_body) = post_decide(&base, &body).await;
    assert_eq!(status, 403, "'shape' scope with a non-IRI predicate => 403");
    assert_eq!(
        resp_body.get("trustDenied").and_then(|v| v.as_bool()),
        Some(true),
    );
}

/// END TO END: a GOV-signed, SHAPE-scoped certification of the credential issuer derives a
/// rule the admission gate consumes => the issuer's credential is admitted => WAC grants
/// Alice Read (200 allow, `certGraphDerived: true`, `admittedSource` = the CERTIFIED issuer).
///
/// The credential issuer is NOT anchored in the trust block — only GOV is — so the grant can
/// come ONLY from the derived rule. That makes this the direct proof that the shape-scope wire
/// format is signable and verifiable end to end: the certifier signed
/// `certification_message` over ITS OWN reconstruction of the canonical scope shape
/// (`canonical_cert_scope_shape`), and the server reproduced the same preimage from the
/// predicate IRI alone.
///
/// MUTATION-VERIFY NOTE: changing either canonical blank-node label (server-side, or in the
/// test's mirror) makes the certifier's signature no longer verify → the edge contributes
/// nothing → Alice is DENIED → this test goes RED.
#[tokio::test]
async fn shape_scoped_certification_derives_rule_and_grants() {
    let base = spawn_trust_on().await;
    let (sig_hex, signed_graph) = sign_test_graph(vec![alice_member_triple()]);
    let nquads = triple_to_nquads_line(&signed_graph[0]);
    let salt_hex = test_salt_hex();
    let body = decide_body_with_certification(
        vec![signed_shape_cert_json(VCARD_HAS_MEMBER)],
        &[WireCred {
            nquads: &nquads,
            sig_hex: &sig_hex,
            salt_hex: &salt_hex,
            issued_at: ISSUED_AT_SECS,
            revoked: false,
        }],
    );
    let (status, resp_body) = post_decide(&base, &body).await;
    // [SONNET-4.6] POSITIONAL format args (CodeQL guard).
    assert_eq!(
        status, 200,
        "a signed shape-scoped certification derives the issuer's rule => 200 allow"
    );
    assert_eq!(
        resp_body.get("allow").and_then(|v| v.as_bool()),
        Some(true),
        "the derived rule admits the credential => allow:true"
    );
    let tj = resp_body
        .get("trustJustification")
        .expect("trustJustification must be present on a trust-admitted GRANT");
    assert_eq!(
        tj.get("certGraphDerived").and_then(|v| v.as_bool()),
        Some(true),
        "the grant went through the cert-graph closure"
    );
    assert_eq!(
        tj.get("admittedSource").and_then(|v| v.as_str()),
        Some(ISSUER_IRI),
        "the admitting rule is the DERIVED one (source = the certified issuer)"
    );
}

/// The shape scope actually SCOPES: the same signed edge, scoped to a DIFFERENT predicate
/// than the certifier's anchor covers, is not a provable narrowing of that anchor, so it
/// derives NOTHING and the credential is never admitted => 403 DENY.
///
/// This is the non-vacuous companion to the grant test above: without it, a shape scope that
/// was silently ignored (or widened to `anyService`) would look identical from the outside.
#[tokio::test]
async fn shape_scope_for_an_uncertified_predicate_derives_nothing_and_denies() {
    let base = spawn_trust_on().await;
    let (sig_hex, signed_graph) = sign_test_graph(vec![alice_member_triple()]);
    let nquads = triple_to_nquads_line(&signed_graph[0]);
    let salt_hex = test_salt_hex();
    // The anchor rule covers `vcard:hasMember`; this edge is scoped to `schema:age`, which the
    // certifier never held authority over.
    let body = decide_body_with_certification(
        vec![signed_shape_cert_json("https://schema.org/age")],
        &[WireCred {
            nquads: &nquads,
            sig_hex: &sig_hex,
            salt_hex: &salt_hex,
            issued_at: ISSUED_AT_SECS,
            revoked: false,
        }],
    );
    let (status, resp_body) = post_decide(&base, &body).await;
    assert_eq!(
        status, 403,
        "a shape scope outside the certifier's anchor derives nothing => deny"
    );
    assert_eq!(
        resp_body.get("allow").and_then(|v| v.as_bool()),
        Some(false),
        "no derived rule => no admitted fact => WAC denies"
    );
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-snopa.8 — the trust extension does NOT compose with the stateful lane.
//
// `trust_authz_decide` decides over its OWN store, built by injecting admitted facts into the
// request-BODY dataset. `"source":"server"` supplies no such dataset, so combining the two
// would silently authorise over a pod the caller did not name. Refused up front instead.
// ---------------------------------------------------------------------------

/// A `"source":"server"` decide body carrying a `"trust"` block — the unsupported combination.
fn server_source_decide_body_with_trust(resource: &str) -> serde_json::Value {
    serde_json::json!({
        "source": "server",
        "session": { "agent": "https://alice.ex/card#me" },
        "resource": resource,
        "mode": "read",
        "view": "wac",
        "trust": {
            "agentIri": "https://alice.ex/card#me",
            "nowUnixSecs": 1_720_000_000_i64,
            "rules": [],
            "certifications": [],
            "credentials": []
        }
    })
}

/// FAIL-CLOSED: `"source":"server"` + a `"trust"` block is refused (400), never answered from
/// whichever pod the server happened to pick.
///
/// MUTATION SPOT-CHECK: delete the `req.source == AuthzSource::Server` refusal in
/// `decide_endpoint` and this test goes red (the request would fall into the trust lane over an
/// EMPTY body dataset).
#[tokio::test]
async fn trust_block_is_refused_with_the_stateful_source() {
    let base = spawn_trust_on().await;
    let resp = client()
        .post(format!("{}/authz/decide", base))
        .json(&server_source_decide_body_with_trust("https://pod.ex/notes/n1"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "the trust extension must not silently combine with 'source':'server'"
    );
}

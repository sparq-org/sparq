//! sq-bzh1 (epic sq-3183): e2e integration tests for the OPT-IN Triple Pattern
//! Fragments / Linked Data Fragments source endpoint (`GET /tpf`).
//!
//! These tests run ONLY with the `tpf` cargo feature (the whole file is gated). They spin the
//! real axum server and assert, over an HTTP request:
//!   * the OPT-IN posture — `/tpf` is `404` unless the config flag is set;
//!   * a pattern match returns the correct triples as valid RDF;
//!   * the Hydra controls are present (`hydra:totalItems`, `hydra:search`/`hydra:template`);
//!   * paging — `hydra:next` on a non-final page, none on the final page;
//!   * content negotiation (Turtle default, N-Triples on request);
//!   * an empty pattern still returns a well-formed fragment (count 0).
#![cfg(feature = "tpf")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice a ex:Person ; ex:knows ex:bob ; ex:name "Alice" .
    ex:bob   a ex:Person ; ex:name "Bob" .
    ex:carol a ex:Person ; ex:knows ex:alice .
"#;

/// Boots a server with the TPF flag ON (or OFF) and returns its base URL.
async fn spawn(tpf_on: bool) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        tpf: tpf_on,
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tpf_404_when_flag_off() {
    let base = spawn(false).await;
    let resp = client().get(format!("{base}/tpf")).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

// ---------------------------------------------------------------------------
// Pattern match → valid RDF with Hydra metadata.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn predicate_pattern_returns_matching_triples() {
    let base = spawn(true).await;
    // ?s ex:knows ?o → 2 triples (alice→bob, carol→alice).
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>")])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    // The two data triples are present.
    assert!(
        body.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob>"),
        "{body}"
    );
    assert!(
        body.contains("<http://ex/carol> <http://ex/knows> <http://ex/alice>"),
        "{body}"
    );
    // Hydra metadata: totalItems = 2 (the estimate).
    assert!(body.contains("hydra/core#totalItems"), "{body}");
    assert!(body.contains("totalItems> \"2\""), "{body}");
    // The search control + template.
    assert!(body.contains("hydra/core#search"), "{body}");
    assert!(body.contains("subject,predicate,object"), "{body}");
}

#[tokio::test]
async fn fully_bound_exact_one_triple() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[
            ("subject", "<http://ex/alice>"),
            ("predicate", "<http://ex/knows>"),
            ("object", "<http://ex/bob>"),
        ])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob>"),
        "{body}"
    );
    assert!(body.contains("totalItems> \"1\""), "{body}");
}

#[tokio::test]
async fn turtle_is_the_default_and_well_formed() {
    let base = spawn(true).await;
    // No Accept → Turtle.
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/name>")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    // Re-parses as valid Turtle (data + metadata + controls).
    let g = Graph::load_str(&body, "turtle").unwrap();
    assert!(
        g.len() >= 2,
        "turtle fragment must carry the matches + metadata: {body}"
    );
}

// ---------------------------------------------------------------------------
// Paging — next boundary.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_pattern_is_well_formed_with_count_zero() {
    let base = spawn(true).await;
    // A subject absent from the data: no matches, but a well-formed fragment with count 0.
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("subject", "<http://ex/nobody>")])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("totalItems> \"0\""), "{body}");
    // The search control is still present so a client can learn the template.
    assert!(body.contains("hydra/core#search"), "{body}");
}

#[tokio::test]
async fn first_page_has_no_next_when_all_fit() {
    // The default page size (100) holds all 7 triples, so page 0 is final: NO hydra:next, and
    // (being page 0) NO hydra:previous.
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("hydra/core#next"),
        "single-page fragment must not link next: {body}"
    );
    assert!(
        !body.contains("hydra/core#previous"),
        "page 0 must not link previous: {body}"
    );
    // All 7 triples present (3 type + 2 knows + 2 name).
    assert!(body.contains("totalItems> \"7\""), "{body}");
}

#[tokio::test]
async fn page_param_beyond_data_is_empty_but_links_previous() {
    // page=1 with the default size (>data) is past the end: empty data, but it links previous.
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("page", "1")])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("hydra/core#previous"),
        "a later page must link previous: {body}"
    );
    assert!(
        body.contains("page=0"),
        "previous should point at page 0: {body}"
    );
}

#[tokio::test]
async fn malformed_term_is_400() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("subject", "not a valid term")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn non_iri_predicate_is_400() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("predicate", "\"a literal\"")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// (sq-kfel, ASVS-G3) A malformed TPF term parse error must NOT echo the caller's
/// offending term verbatim (the same echo-of-input info-leak class the rest of the surface
/// sanitizes). The error body is the structured JSON envelope with a generic category; the
/// sentinel embedded in the bad term never reaches the body.
#[tokio::test]
async fn malformed_term_does_not_echo_input() {
    const SENTINEL: &str = "tpf_secret_sentinel";
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("subject", format!("not a term {SENTINEL}").as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("{\"error\":"),
        "structured JSON error, got: {body}"
    );
    assert!(
        !body.contains(SENTINEL),
        "TPF term parse error echoed caller input: {body}"
    );
}

// ---------------------------------------------------------------------------
// sq-dxhb — Hydra first/last paging controls (the fuller paging vocabulary).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fragment_carries_first_and_last_paging_controls() {
    // page=1 over the default page size: every page links hydra:first (page 0) and hydra:last.
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("page", "1")])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("hydra/core#first"),
        "fragment must link hydra:first: {body}"
    );
    assert!(
        body.contains("hydra/core#last"),
        "fragment must link hydra:last: {body}"
    );
    // first → page 0; with 7 triples in one default page, last is also page 0.
    assert!(
        body.contains("page=0"),
        "first/last should reference page 0: {body}"
    );
}

// ---------------------------------------------------------------------------
// sq-dxhb — brTPF bindings-restricted fragments (only with the `brtpf` feature).
// ---------------------------------------------------------------------------

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_values_query_param_restricts_the_fragment() {
    let base = spawn(true).await;
    // ?s ex:knows ?o → alice→bob, carol→alice (2). Restrict via `values` to {?s=carol}: 1.
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[
            ("predicate", "<http://ex/knows>"),
            ("values", "s=<http://ex/carol>"),
        ])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Only carol→alice, NOT alice→bob.
    assert!(
        body.contains("<http://ex/carol> <http://ex/knows> <http://ex/alice>"),
        "{body}"
    );
    assert!(
        !body.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob>"),
        "the restriction must exclude the alice→bob triple: {body}"
    );
    // The bindings-restricted count is 1 (NOT the unrestricted 2).
    assert!(body.contains("totalItems> \"1\""), "{body}");
    // The brTPF `values` control is advertised in the Hydra search template/mapping.
    assert!(body.contains("subject,predicate,object,values"), "{body}");
    assert!(body.contains("brtpf#values"), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_post_body_carries_a_binding_set() {
    let base = spawn(true).await;
    // POST the binding set in the body (the conventional brTPF transport for a large set).
    let resp = client()
        .post(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>")])
        .header("Accept", "application/n-triples")
        .body("s=<http://ex/alice>\ns=<http://ex/carol>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Both knows triples (the union of the two bindings).
    assert!(
        body.contains("<http://ex/alice> <http://ex/knows> <http://ex/bob>"),
        "{body}"
    );
    assert!(
        body.contains("<http://ex/carol> <http://ex/knows> <http://ex/alice>"),
        "{body}"
    );
    assert!(body.contains("totalItems> \"2\""), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_empty_values_is_plain_tpf() {
    // An empty `values` ⇒ no restriction: identical to a plain TPF request (all knows triples).
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>"), ("values", "")])
        .header("Accept", "application/n-triples")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("totalItems> \"2\""),
        "unrestricted count: {body}"
    );
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_malformed_bindings_is_400_and_no_echo() {
    const SENTINEL: &str = "brtpf_secret_sentinel";
    let base = spawn(true).await;
    // A malformed term value carrying the sentinel — the parse error quotes it server-side, but
    // the sanitized HTTP body must NOT echo it (same info-leak posture as the TPF term parse).
    let resp = client()
        .post(format!("{base}/tpf"))
        .body(format!("s=not_a_valid_term_{SENTINEL}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("{\"error\":"),
        "structured JSON error: {body}"
    );
    assert!(
        !body.contains(SENTINEL),
        "brTPF binding parse error echoed caller input: {body}"
    );
}

// ---------------------------------------------------------------------------
// sq-r74h — brTPF binding-set DoS caps (mapping count + payload bytes), enforced
// independently of --max-body-bytes and refusing 413 (not 400) so a client can distinguish
// "too large" from "malformed". Only with the `brtpf` feature.
// ---------------------------------------------------------------------------

/// Boots a TPF-on server with explicit brTPF binding-set caps (0 = disabled).
#[cfg(feature = "brtpf")]
async fn spawn_with_caps(max_bindings: usize, max_values_bytes: usize) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        tpf: true,
        brtpf_max_bindings: max_bindings,
        brtpf_max_values_bytes: max_values_bytes,
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_too_many_mappings_is_413() {
    // Cap of 2 mappings; POST 3 → 413 (not 400 — the set is well-formed, just too large).
    let base = spawn_with_caps(2, 0).await;
    let resp = client()
        .post(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>")])
        .body("s=<http://ex/a>\ns=<http://ex/b>\ns=<http://ex/c>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("{\"error\":"),
        "structured JSON error: {body}"
    );
    // The 413 message names the cap knob, so a client/operator knows what to change.
    assert!(body.contains("brtpf-max-bindings"), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_mappings_at_the_cap_succeed() {
    // Exactly at the cap is accepted (boundary): 2 mappings under a cap of 2 → 200.
    let base = spawn_with_caps(2, 0).await;
    let resp = client()
        .post(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>")])
        .header("Accept", "application/n-triples")
        .body("s=<http://ex/alice>\ns=<http://ex/carol>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("totalItems> \"2\""), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_values_query_string_payload_byte_cap_is_413() {
    // The `values` query-string carrier is NOT a body, so --max-body-bytes never bounds it; only
    // --brtpf-max-values-bytes does. A 10-byte payload cap rejects a longer GET `values`.
    let base = spawn_with_caps(0, 10).await;
    let resp = client()
        .get(format!("{base}/tpf"))
        .query(&[
            ("predicate", "<http://ex/knows>"),
            ("values", "s=<http://ex/carol>"), // > 10 bytes
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.contains("brtpf-max-values-bytes"), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_caps_disabled_with_zero_allow_a_large_set() {
    // 0 on both caps disables them — a set larger than the production defaults is accepted.
    let base = spawn_with_caps(0, 0).await;
    let resp = client()
        .post(format!("{base}/tpf"))
        .query(&[("predicate", "<http://ex/knows>")])
        .header("Accept", "application/n-triples")
        .body("s=<http://ex/alice>\ns=<http://ex/carol>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("totalItems> \"2\""), "{body}");
}

#[cfg(feature = "brtpf")]
#[tokio::test]
async fn brtpf_413_payload_message_does_not_echo_input() {
    // The 413 body names only the cap, never the caller's payload (no info-leak / no echo).
    const SENTINEL: &str = "brtpf_dos_secret_sentinel";
    let base = spawn_with_caps(0, 5).await;
    let resp = client()
        .post(format!("{base}/tpf"))
        .body(format!("s=<http://ex/{SENTINEL}>"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains(SENTINEL),
        "413 cap message echoed caller input: {body}"
    );
}

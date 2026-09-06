//! [FABLE-5] sq-u16eq — end-to-end tests for the pod-backed MCP server (feature
//! `solid`): LDP resource/container CRUD tools over a real `PodStore` with a real
//! materialized WAC view, driven through the actual JSON-RPC dispatch core.
//!
//! Load-bearing invariants (each would flip red under the matching mutation):
//! - **Existence non-disclosure (draft §9.3)**: the error for an EXISTING document the
//!   session may not read is byte-identical to the error for a NONEXISTENT one.
//! - **Data-derived containment (draft §6.4)**: `container_list` returns exactly the
//!   stored `ldp:contains` members — including one whose IRI lives OUTSIDE the
//!   container's path — and omits a stored document that has no containment triple
//!   (an IRI-path-guessing implementation would get both wrong).
//! - **Shared dataset (draft §6.4)**: a `resource_put` is immediately visible to the
//!   SPARQL `query` tool, and vice-versa surfaces cannot disagree.
//! - **Write gating (draft §7.1)**: with `allow_update = false` the mutating tools are
//!   neither advertised nor callable.
//! - **ACL write-through (draft §7.3)**: a `resource_put`/`resource_delete` of an
//!   `.acl` re-derives authorization atomically (grant appears/disappears with no
//!   separate materialize call), and a malformed ACL body changes nothing.
//! - **Content negotiation (draft §6.4, sq-wbsf5)**: `accept: text/turtle` yields REAL
//!   prefix-compacted Turtle carrying the same triples as the N-Triples body (N-Triples
//!   relabelled `text/turtle` fails); absent `accept` is unchanged; §9.3 non-disclosure
//!   holds in the Turtle path; and an unservable `accept` answers identically whether
//!   the resource exists, is unreadable, or is absent. Non-RDF bodies are REFUSED with
//!   the scope-out named, never stored.

#![cfg(feature = "solid")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_mcp::{SolidMcpServer, SolidServerConfig};
use sparq_solid::PodStore;

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// A small WAC pod: a root container; `notes/` (one doc, one out-of-path member, one
/// UNLISTED doc); a `secret/` subtree governed by its own ACL (bob-only); and a root
/// ACL granting alice Read/Write/Control on the root and by default.
fn pod() -> PodStore {
    let nq = r#"
<https://pod.ex/> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/notes/> <https://pod.ex/> .
<https://pod.ex/> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/secret/> <https://pod.ex/> .
<https://pod.ex/notes/> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/notes/n1> <https://pod.ex/notes/> .
<https://pod.ex/notes/> <http://www.w3.org/ns/ldp#contains> <https://elsewhere.example/shared/doc> <https://pod.ex/notes/> .
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/notes/unlisted#it> <https://ex.dev/ns#title> "orphan" <https://pod.ex/notes/unlisted> .
<https://pod.ex/secret/s1#it> <https://ex.dev/ns#title> "classified" <https://pod.ex/secret/s1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> <https://pod.ex/.acl> .
<https://pod.ex/secret/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/secret/.acl> .
<https://pod.ex/secret/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/secret/> <https://pod.ex/secret/.acl> .
<https://pod.ex/secret/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://pod.ex/secret/.acl> .
<https://pod.ex/secret/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/secret/.acl> .
"#;
    PodStore::new(Graph::load_dataset(nq, "nquads").expect("fixture parses"))
}

fn server_for(agent: &str, allow_update: bool) -> SolidMcpServer {
    let config = SolidServerConfig {
        agent: Some(agent.to_string()),
        allow_update,
        ..SolidServerConfig::default()
    };
    SolidMcpServer::with_config(pod(), config).expect("materializes")
}

fn parse(resp: &str) -> Value {
    serde_json::from_str(resp).expect("response is valid JSON")
}

fn rpc(server: &mut SolidMcpServer, raw: &str) -> Value {
    parse(&server.handle_message(raw).expect("request gets a response"))
}

/// Call one tool, returning `(text, is_error)` from the MCP `CallToolResult`.
fn tool(server: &mut SolidMcpServer, name: &str, args: Value) -> (String, bool) {
    let req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    });
    let resp = rpc(server, &req.to_string());
    let result = &resp["result"];
    assert!(
        result.is_object(),
        "tools/call must yield a tool result (got protocol error: {resp})"
    );
    (
        result["content"][0]["text"].as_str().expect("text content").to_string(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

fn tool_names(server: &mut SolidMcpServer) -> Vec<String> {
    let resp = rpc(server, r#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#);
    resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect()
}

// ───────────────────────── Class R (read core) ─────────────────────────

#[test]
fn read_only_pod_server_advertises_read_tools_only() {
    let mut s = server_for(ALICE, false);
    let names = tool_names(&mut s);
    assert_eq!(names, ["query", "resource_get", "container_list", "introspect", "shapes", "stats"]);
}

#[test]
fn write_enabled_pod_server_advertises_the_mutating_tools() {
    let mut s = server_for(ALICE, true);
    let names = tool_names(&mut s);
    assert_eq!(
        names,
        ["query", "resource_get", "container_list", "introspect", "shapes", "stats",
         "update", "resource_put", "resource_delete", "container_create"]
    );
}

#[test]
fn resource_get_serves_the_document_as_ntriples() {
    let mut s = server_for(ALICE, false);
    let (text, is_err) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(!is_err, "authorized read must succeed: {text}");
    let v: Value = serde_json::from_str(&text).expect("result is JSON");
    assert_eq!(v["url"], "https://pod.ex/notes/n1");
    assert_eq!(v["content_type"], "application/n-triples");
    let content = v["content"].as_str().expect("content");
    assert!(content.contains("\"hello\""), "the document body is served: {content}");
    assert!(
        !content.contains("classified"),
        "resource_get must serve ONE document, not the dataset"
    );
}

#[test]
fn resource_get_rejects_an_unservable_accept_instead_of_coercing() {
    let mut s = server_for(ALICE, false);
    for accept in ["application/ld+json", "application/rdf+xml", "image/png"] {
        let (text, is_err) = tool(
            &mut s,
            "resource_get",
            json!({"url": "https://pod.ex/notes/n1", "accept": accept}),
        );
        assert!(is_err, "unsupported accept must be a tool error, not silent coercion: {text}");
        assert!(text.contains("unsupported accept"), "honest error: {text}");
        assert!(
            text.contains("application/n-triples") && text.contains("text/turtle"),
            "the refusal must name what IS served: {text}"
        );
    }
}

// ── [SONNET-4.6] sq-wbsf5: `accept` content negotiation (draft §6.4) ──

#[test]
fn resource_get_serves_turtle_when_accept_asks_for_it() {
    // The negotiated body must be REAL Turtle (prefix-compacted, so N-Triples returned
    // under a `text/turtle` label would not pass), and must parse back to the very same
    // triples the default N-Triples body carries — negotiation changes syntax, not content.
    let mut s = server_for(ALICE, false);
    let (text, is_err) = tool(
        &mut s,
        "resource_get",
        json!({"url": "https://pod.ex/notes/", "accept": "text/turtle"}),
    );
    assert!(!is_err, "authorized read must succeed: {text}");
    let v: Value = serde_json::from_str(&text).expect("result is JSON");
    assert_eq!(v["content_type"], "text/turtle");
    let ttl = v["content"].as_str().expect("content");
    assert!(
        ttl.contains("@prefix ldp:") && ttl.contains("ldp:contains"),
        "the body must be prefix-compacted Turtle, not N-Triples relabelled: {ttl}"
    );

    let (nt_text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/"}));
    let nt = serde_json::from_str::<Value>(&nt_text).expect("result is JSON");
    let from_ttl = Graph::load_str(ttl, "turtle").expect("the served Turtle parses");
    let from_nt = Graph::load_str(nt["content"].as_str().expect("content"), "ntriples")
        .expect("the served N-Triples parses");
    assert_eq!(from_ttl.len(), from_nt.len(), "both representations carry the same triples");
    assert!(sparq_engine::ask(
        &from_ttl,
        "ASK { <https://pod.ex/notes/> <http://www.w3.org/ns/ldp#contains> <https://pod.ex/notes/n1> }"
    )
    .expect("ASK evaluates"));
}

#[test]
fn resource_get_defaults_to_ntriples_when_accept_is_absent() {
    // The v1 default is load-bearing: a caller that never sent `accept` must not silently
    // change syntax now that a second representation exists.
    let mut s = server_for(ALICE, false);
    let (text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    let v: Value = serde_json::from_str(&text).expect("result is JSON");
    assert_eq!(v["content_type"], "application/n-triples");
    let (explicit, _) = tool(
        &mut s,
        "resource_get",
        json!({"url": "https://pod.ex/notes/n1", "accept": "application/n-triples"}),
    );
    assert_eq!(text, explicit, "absent `accept` must serve exactly the N-Triples body");
}

#[test]
fn turtle_negotiation_does_not_weaken_existence_non_disclosure() {
    // §9.3 must hold in EVERY representation: a Turtle read of an existing-but-unreadable
    // document is byte-identical to a Turtle read of a nonexistent one.
    let mut s = server_for(ALICE, false);
    let (denied, is_err_denied) = tool(
        &mut s,
        "resource_get",
        json!({"url": "https://pod.ex/secret/s1", "accept": "text/turtle"}),
    );
    let (absent, is_err_absent) = tool(
        &mut s,
        "resource_get",
        json!({"url": "https://pod.ex/secret/nope", "accept": "text/turtle"}),
    );
    assert!(is_err_denied && is_err_absent);
    assert_eq!(
        denied.replace("secret/s1", "X"),
        absent.replace("secret/nope", "X"),
        "the Turtle path must not disclose existence"
    );
    // The message is the SAME template the N-Triples path uses — not a Turtle-specific one.
    let (nt_denied, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/secret/s1"}));
    assert_eq!(denied, nt_denied);
}

#[test]
fn an_unservable_accept_answers_the_same_whether_the_resource_exists_or_not() {
    // Negotiation runs BEFORE the read gate and depends only on the `accept` string, so
    // it cannot be turned into an existence oracle by probing with a bad media type.
    let mut s = server_for(ALICE, false);
    let probe = |s: &mut SolidMcpServer, url: &str| {
        tool(s, "resource_get", json!({"url": url, "accept": "application/rdf+xml"})).0
    };
    let readable = probe(&mut s, "https://pod.ex/notes/n1");
    let unreadable = probe(&mut s, "https://pod.ex/secret/s1");
    let nonexistent = probe(&mut s, "https://pod.ex/notes/nope");
    assert_eq!(readable, unreadable);
    assert_eq!(readable, nonexistent);
}

#[test]
fn resource_put_refuses_a_non_rdf_body_and_names_the_scope_out() {
    // [SONNET-4.6] sq-wbsf5 — the DECIDED non-RDF story: binary resources are scoped out,
    // and the refusal says so rather than looking like a missing parser. If a blob path is
    // ever added this test is the one that must be rewritten, deliberately.
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/photo.png",
            "content": "\u{89}PNG not-really-a-png",
            "content_type": "image/png"
        }),
    );
    assert!(is_err, "a non-RDF body must be refused, never stored: {text}");
    assert!(text.contains("RDF sources only"), "honest error: {text}");
    assert!(text.contains("out of scope"), "the refusal names the scope-out: {text}");
    // And nothing was created — the refusal is total, not partial.
    let (probe, denied) =
        tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/photo.png"}));
    assert!(denied, "the refused resource must not exist: {probe}");
}

#[test]
fn unauthorized_read_is_byte_identical_to_nonexistent() {
    // §9.3: alice may NOT read the (existing) secret doc; the error must be
    // byte-identical to the error for a document that does not exist, so existence is
    // never disclosed. This test fails if denial gets its own message.
    let mut s = server_for(ALICE, false);
    let (denied, e1) =
        tool(&mut s, "resource_get", json!({"url": "https://pod.ex/secret/s1"}));
    let (absent, e2) =
        tool(&mut s, "resource_get", json!({"url": "https://pod.ex/secret/nope"}));
    assert!(e1 && e2, "both must be tool errors");
    assert_eq!(
        denied.replace("secret/s1", "X"),
        absent.replace("secret/nope", "X"),
        "unauthorized-read and nonexistent errors must be indistinguishable"
    );
    // And the same template covers a readable-but-absent target.
    let (readable_absent, e3) =
        tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/nope"}));
    assert!(e3);
    assert_eq!(readable_absent, "resource not found: <https://pod.ex/notes/nope>");
    assert_eq!(denied, "resource not found: <https://pod.ex/secret/s1>");
}

#[test]
fn container_list_derives_members_from_stored_containment_only() {
    let mut s = server_for(ALICE, false);
    let (text, is_err) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/notes/"}));
    assert!(!is_err, "{text}");
    let v: Value = serde_json::from_str(&text).expect("JSON");
    let members: Vec<(&str, bool)> = v["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| (m["url"].as_str().unwrap(), m["container"].as_bool().unwrap()))
        .collect();
    // The out-of-path member IS listed (containment is data, not IRI prefixes)…
    assert!(members.contains(&("https://elsewhere.example/shared/doc", false)));
    assert!(members.contains(&("https://pod.ex/notes/n1", false)));
    // …and the stored-but-unlisted document is NOT (no ldp:contains triple).
    assert!(
        !members.iter().any(|(u, _)| u.contains("unlisted")),
        "a document without a containment triple must not be listed: {members:?}"
    );
    assert_eq!(members.len(), 2);

    // The root lists its two child containers with the container flag set.
    let (text, _) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/"}));
    let v: Value = serde_json::from_str(&text).expect("JSON");
    let flags: Vec<bool> = v["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["container"].as_bool().unwrap())
        .collect();
    assert_eq!(flags, [true, true]);
}

#[test]
fn container_list_non_disclosure_matches_resource_get() {
    let mut s = server_for(ALICE, false);
    let (denied, e1) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/secret/"}));
    assert!(e1);
    assert_eq!(denied, "resource not found: <https://pod.ex/secret/>");
}

#[test]
fn query_tool_is_session_scoped() {
    // The same query under two sessions: alice sees notes, not secrets; bob sees
    // secrets, not notes. Unauthorized data contributes nothing and raises no error.
    let q = json!({
        "sparql": "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } } ORDER BY ?t"
    });
    let mut alice = server_for(ALICE, false);
    let (text, is_err) = tool(&mut alice, "query", q.clone());
    assert!(!is_err, "{text}");
    assert!(text.contains("hello") && !text.contains("classified"), "{text}");

    let mut bob = server_for(BOB, false);
    let (text, is_err) = tool(&mut bob, "query", q);
    assert!(!is_err, "{text}");
    assert!(text.contains("classified") && !text.contains("hello"), "{text}");
}

/// [SONNET-4.6] sq-8n6iv — the aggregate tools are session-scoped exactly as `query` is.
/// The base server's `stats` counts the WHOLE graph; if the pod server's did, alice's
/// totals would include bob's `secret/` documents — an aggregate leak that no
/// per-resource check catches, because no resource was read.
#[test]
fn stats_counts_only_the_documents_the_session_may_read() {
    let mut alice = server_for(ALICE, false);
    let (text, is_err) = tool(&mut alice, "stats", json!({}));
    assert!(!is_err, "{text}");
    let alice_stats: Value = serde_json::from_str(&text).expect("stats is JSON");

    let mut bob = server_for(BOB, false);
    let (text, is_err) = tool(&mut bob, "stats", json!({}));
    assert!(!is_err, "{text}");
    let bob_stats: Value = serde_json::from_str(&text).expect("stats is JSON");

    // Two sessions, two different totals — neither is the pod's total.
    let (a, b) = (alice_stats["triples"].as_u64().unwrap(), bob_stats["triples"].as_u64().unwrap());
    assert!(a > 0 && b > 0, "each session sees its own data: {a} / {b}");
    assert_ne!(a, b, "a whole-pod count would make these equal");
    // The pod holds strictly more than either session can read.
    let whole_pod: u64 = alice
        .store()
        .graph
        .named
        .iter()
        .filter(|(name, _)| !name.to_string().starts_with("<urn:sparq:"))
        .map(|(_, g)| g.len() as u64)
        .sum();
    assert!(a < whole_pod && b < whole_pod, "{a} / {b} must both be under {whole_pod}");
}

#[test]
fn introspect_mines_only_the_authorized_documents() {
    // Alice's authorized documents use ldp:contains and ex:title; bob's `secret/s1`
    // is invisible to her, so nothing OF THAT DOCUMENT may reach her schema.
    //
    // What legitimately DOES appear is the `https://pod.ex/secret/` container IRI: the
    // root container — which alice may read — stores `<pod.ex/> ldp:contains
    // <pod.ex/secret/>`, so the name is part of a document she is authorized to read
    // (the same disclosure `container_list` already makes). The boundary this test pins
    // is the unreadable document's own subjects and terms, not the mention of its
    // container's name in a readable one.
    let mut alice = server_for(ALICE, false);
    let (text, is_err) = tool(&mut alice, "introspect", json!({}));
    assert!(!is_err, "{text}");
    assert!(text.contains("https://ex.dev/ns#title"), "the readable predicate is mined: {text}");
    assert!(!text.contains("secret/s1"), "no trace of the unreadable document: {text}");
    // The materialized authorization view lives in the reserved graph space and is not
    // pod content: mining it would hand the agent the pod's policy vocabulary.
    assert!(!text.contains("urn:sparq:"), "the reserved auth view is not pod data: {text}");

    // The text summary is the same projection, so it cannot disagree.
    let (text, is_err) = tool(&mut alice, "introspect", json!({"format": "text"}));
    assert!(!is_err, "{text}");
    assert!(!text.contains("secret/s1"), "{text}");
}

#[test]
fn aggregate_tools_fail_closed_for_a_session_with_no_grants() {
    // An anonymous session reads nothing, so it must learn nothing in aggregate either
    // — zeros, not the pod's real totals.
    let config = SolidServerConfig::default();
    let mut anon = SolidMcpServer::with_config(pod(), config).expect("materializes");
    let (text, is_err) = tool(&mut anon, "stats", json!({}));
    assert!(!is_err, "{text}");
    let stats: Value = serde_json::from_str(&text).expect("stats is JSON");
    assert_eq!(stats["triples"], 0);
    assert_eq!(stats["classes"], 0);
    assert_eq!(stats["predicates"], 0);

    let (text, is_err) = tool(&mut anon, "introspect", json!({}));
    assert!(!is_err, "{text}");
    assert!(!text.contains("https://ex.dev/ns#title"), "{text}");
}

#[test]
fn a_write_is_reflected_in_the_next_aggregate() {
    // The aggregate is derived per call from the live authorized view, so it can never
    // serve a stale schema after a mutation (or after an ACL change).
    let mut alice = server_for(ALICE, true);
    let (text, _) = tool(&mut alice, "stats", json!({}));
    let before: Value = serde_json::from_str(&text).expect("stats is JSON");

    let (text, is_err) = tool(
        &mut alice,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n2",
            "content_type": "text/turtle",
            "content": "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#tag> \"fresh\" .",
        }),
    );
    assert!(!is_err, "{text}");

    let (text, _) = tool(&mut alice, "stats", json!({}));
    let after: Value = serde_json::from_str(&text).expect("stats is JSON");
    assert!(
        after["triples"].as_u64().unwrap() > before["triples"].as_u64().unwrap(),
        "the new document must be counted: {before} -> {after}"
    );
    let (text, _) = tool(&mut alice, "introspect", json!({}));
    assert!(text.contains("https://ex.dev/ns#tag"), "the new predicate is mined: {text}");
}

// ───────────────────────── Class U (gated writes) ─────────────────────────

#[test]
fn mutating_tools_are_refused_when_updates_are_disabled() {
    let mut s = server_for(ALICE, false);
    for name in ["update", "resource_put", "resource_delete", "container_create"] {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": name, "arguments": {"url": "https://pod.ex/x"} }
        });
        let resp = rpc(&mut s, &req.to_string());
        assert!(
            resp["error"]["message"].as_str().unwrap_or("").contains("read-only"),
            "`{name}` must be refused at the protocol level: {resp}"
        );
    }
}

#[test]
fn resource_put_create_links_containment_and_is_visible_to_query() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n2",
            "content": "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"second\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(!is_err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["created"], true);
    assert_eq!(v["triples"], 1);

    // Containment: the parent listing now includes n2.
    let (text, _) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/notes/"}));
    assert!(text.contains("https://pod.ex/notes/n2"), "created doc must be contained: {text}");

    // Shared dataset (§6.4): the SPARQL tool sees the new document immediately.
    let (text, is_err) = tool(
        &mut s,
        "query",
        json!({"sparql":
            "ASK { GRAPH <https://pod.ex/notes/n2> { ?s <https://ex.dev/ns#title> \"second\" } }"}),
    );
    assert!(!is_err && text.contains("true"), "query must see the put: {text}");

    // And resource_get round-trips it.
    let (text, is_err) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n2"}));
    assert!(!is_err && text.contains("second"), "{text}");
}

#[test]
fn resource_put_replace_swaps_the_named_graph_atomically() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"rewritten\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(!is_err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["created"], false, "replacing an existing doc is not a create");
    let (text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(text.contains("rewritten"), "{text}");
    assert!(!text.contains("hello"), "PUT is a full replacement, not a merge: {text}");
}

#[test]
fn resource_put_malformed_body_mutates_nothing() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "this is not turtle @@@",
            "content_type": "text/turtle"
        }),
    );
    assert!(is_err, "malformed content must be rejected: {text}");
    let (text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(text.contains("hello"), "parse-first: the prior content survives: {text}");
}

#[test]
fn resource_put_write_gates_follow_non_disclosure() {
    let mut s = server_for(ALICE, true);
    // No read on the secret subtree → the not-found error, never a write-denied one.
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/secret/s1",
            "content": "<https://pod.ex/secret/s1#it> <https://ex.dev/ns#title> \"pwned\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(is_err);
    assert_eq!(text, "resource not found: <https://pod.ex/secret/s1>");

    // Bob CAN read secret/ but has no Write → the distinguishable denied error.
    let mut bob = server_for(BOB, true);
    let (text, is_err) = tool(
        &mut bob,
        "resource_put",
        json!({
            "url": "https://pod.ex/secret/s1",
            "content": "<https://pod.ex/secret/s1#it> <https://ex.dev/ns#title> \"mine\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(is_err);
    assert_eq!(text, "write access denied: <https://pod.ex/secret/s1>");
}

#[test]
fn resource_delete_removes_doc_and_containment() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) =
        tool(&mut s, "resource_delete", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(!is_err, "{text}");
    let (text, is_err) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(is_err);
    assert_eq!(text, "resource not found: <https://pod.ex/notes/n1>");
    let (text, _) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/notes/"}));
    assert!(!text.contains("notes/n1"), "containment link must be gone: {text}");
}

#[test]
fn resource_delete_rejects_a_non_empty_container() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(&mut s, "resource_delete", json!({"url": "https://pod.ex/notes/"}));
    assert!(is_err);
    assert!(text.contains("not empty"), "{text}");
    // Still listable afterwards — nothing was deleted.
    let (text, is_err) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/notes/"}));
    assert!(!is_err && text.contains("notes/n1"), "{text}");
}

#[test]
fn container_create_creates_a_typed_linked_empty_container() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) =
        tool(&mut s, "container_create", json!({"url": "https://pod.ex/projects/"}));
    assert!(!is_err, "{text}");
    // Linked into the root listing, flagged as a container.
    let (text, _) = tool(&mut s, "container_list", json!({"url": "https://pod.ex/"}));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["url"] == "https://pod.ex/projects/" && m["container"] == true));
    // Typed, empty, and listable.
    let (text, is_err) =
        tool(&mut s, "container_list", json!({"url": "https://pod.ex/projects/"}));
    assert!(!is_err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["members"].as_array().unwrap().len(), 0);
    let (text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/projects/"}));
    assert!(text.contains("BasicContainer"), "{text}");
    // Slash discipline + duplicate rejection.
    let (text, is_err) = tool(&mut s, "container_create", json!({"url": "https://pod.ex/x"}));
    assert!(is_err && text.contains("slash-terminated"), "{text}");
    let (text, is_err) =
        tool(&mut s, "container_create", json!({"url": "https://pod.ex/projects/"}));
    assert!(is_err && text.contains("already exists"), "{text}");
}

#[test]
fn update_tool_enforces_session_write_authorization() {
    let mut bob = server_for(BOB, true);
    // Bob has Read-only on secret/ — a write there must be DENIED atomically.
    let (text, is_err) = tool(
        &mut bob,
        "update",
        json!({"sparql":
            "INSERT DATA { GRAPH <https://pod.ex/secret/s1> { <urn:x> <urn:y> \"z\" } }"}),
    );
    assert!(is_err, "unwritable target must reject the whole update: {text}");
    // Alice CAN write under notes/.
    let mut alice = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut alice,
        "update",
        json!({"sparql":
            "INSERT DATA { GRAPH <https://pod.ex/notes/n1> { <urn:x> <urn:y> \"z\" } }"}),
    );
    assert!(!is_err, "{text}");
}

/// Draft §9.4: EVERY tool-issued evaluation MUST be bounded. Until sq-yhlf0 the `update`
/// tool was the one hole — the read tools threaded `SolidServerConfig`'s deadline/row cap
/// into the engine, while `update` delegated to the unbudgeted `PodStore::update_as`, so a
/// pathological `DELETE`/`INSERT … WHERE` could run the server's writer unbounded.
///
/// The pathological shape here is a self-cross-product WHERE — the shape that blows up on
/// a real pod. Two limits are exercised:
///
/// - the ROW CAP, which trips on the actual size of the intermediate result with a clock
///   that has NOT run out (the non-degenerate case);
/// - the DEADLINE, set to zero seconds so the abort is deterministic and instant in CI
///   rather than racing a genuine blow-up. It is the same cooperative check a real blow-up
///   trips at, just reached on the first poll.
///
/// Both must surface as a tool ERROR (`isError: true`), not a protocol error and not a
/// silent truncation.
#[test]
fn a_pathological_update_trips_the_tool_budget() {
    let pathological = "INSERT { GRAPH <https://pod.ex/notes/n1> \
                        { <urn:x> <urn:y> \"z\" } } \
                        WHERE { GRAPH <https://pod.ex/notes/n1> { ?s ?p ?o } \
                                GRAPH <https://pod.ex/> { ?s2 ?p2 ?o2 } }";

    let budgeted = |timeout: Option<u64>, max_rows: Option<usize>| SolidServerConfig {
        agent: Some(ALICE.to_string()),
        allow_update: true,
        query_timeout_secs: timeout,
        max_rows,
        ..SolidServerConfig::default()
    };

    // Row cap: the cross-product's intermediate result exceeds it.
    let mut capped = SolidMcpServer::with_config(pod(), budgeted(None, Some(1)))
        .expect("materializes");
    let (text, is_err) = tool(&mut capped, "update", json!({"sparql": pathological}));
    assert!(is_err, "an over-budget update must be a tool error: {text}");
    assert!(
        text.contains("query budget exceeded"),
        "the error must name the budget, not a denial: {text}"
    );

    // Deadline: exhausted before the update is issued.
    let mut timed = SolidMcpServer::with_config(pod(), budgeted(Some(0), None))
        .expect("materializes");
    let (text, is_err) = tool(&mut timed, "update", json!({"sparql": pathological}));
    assert!(is_err, "an over-deadline update must be a tool error: {text}");
    assert!(text.contains("query budget exceeded (timeout)"), "{text}");

    // Positive control: alice IS authorized for this exact update, and under the DEFAULT
    // budget it applies — so the two aborts above are the budget biting, not a denial or a
    // malformed update. The inserted triple is visible afterwards.
    let mut alice = server_for(ALICE, true);
    let (text, is_err) = tool(&mut alice, "update", json!({"sparql": pathological}));
    assert!(!is_err, "the same update succeeds under the default budget: {text}");
    let (doc, is_err) = tool(&mut alice, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(!is_err && doc.contains("urn:x"), "the write really landed: {doc}");
}

// ───────────────────────── ACL write-through (§7.3) ─────────────────────────

#[test]
fn acl_put_and_delete_rederive_authorization_atomically() {
    let mut alice = server_for(ALICE, true);

    // Before: bob cannot see notes/n1 (non-disclosure not-found).
    let mut bob = server_for(BOB, false);
    let (text, is_err) = tool(&mut bob, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(is_err && text == "resource not found: <https://pod.ex/notes/n1>");

    // Alice (Control via the root ACL) PUTs notes/.acl granting alice full + bob Read.
    // Absolute IRIs: the storage layer parses the body with no base IRI.
    let acl_body = r#"
@prefix acl: <http://www.w3.org/ns/auth/acl#> .
<https://pod.ex/notes/.acl#owner> a acl:Authorization ;
  acl:accessTo <https://pod.ex/notes/> ;
  acl:default <https://pod.ex/notes/> ;
  acl:agent <https://alice.ex/card#me> ;
  acl:mode acl:Read, acl:Write, acl:Control .
<https://pod.ex/notes/.acl#reader> a acl:Authorization ;
  acl:default <https://pod.ex/notes/> ;
  acl:agent <https://bob.ex/card#me> ;
  acl:mode acl:Read .
"#;
    let (text, is_err) = tool(
        &mut alice,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/.acl",
            "content": acl_body,
            "content_type": "text/turtle"
        }),
    );
    assert!(!is_err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["access_control"], true);
    assert_eq!(v["created"], true);

    // The grant took effect on ALICE'S OWN STORE immediately (no separate
    // materialize call): bob's session against that store now reads n1. We check on
    // alice's server by asking its store directly.
    let bob_session = sparq_solid::Session {
        agent: Some(BOB),
        client: None,
        issuer: None,
        now: None,
    };
    let d = alice.store().decide(&bob_session, "https://pod.ex/notes/n1", sparq_solid::Mode::Read);
    assert!(d.allow, "the ACL write-through must re-derive authorization atomically");

    // A malformed ACL body is rejected parse-first and changes nothing.
    let (text, is_err) = tool(
        &mut alice,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/.acl",
            "content": "@@@ not turtle",
            "content_type": "text/turtle"
        }),
    );
    assert!(is_err, "{text}");
    let d = alice.store().decide(&bob_session, "https://pod.ex/notes/n1", sparq_solid::Mode::Read);
    assert!(d.allow, "a failed ACL write must leave the prior policy in force");

    // DELETE the ACL: bob's grant disappears with it (delete narrows, atomically).
    let (text, is_err) =
        tool(&mut alice, "resource_delete", json!({"url": "https://pod.ex/notes/.acl"}));
    assert!(!is_err, "{text}");
    let d = alice.store().decide(&bob_session, "https://pod.ex/notes/n1", sparq_solid::Mode::Read);
    assert!(!d.allow, "deleting the ACL must revoke its grants immediately");
}

#[test]
fn acl_write_requires_control_and_non_discloses_without_it() {
    // Bob has Read on secret/ but NOT Control → writing secret/.acl must yield the
    // not-found error (never a distinguishable denied), and the policy must survive.
    let mut bob = server_for(BOB, true);
    let (text, is_err) = tool(
        &mut bob,
        "resource_put",
        json!({
            "url": "https://pod.ex/secret/.acl",
            "content": "<https://pod.ex/secret/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(is_err);
    assert_eq!(text, "resource not found: <https://pod.ex/secret/.acl>");
    let (text, is_err) =
        tool(&mut bob, "resource_delete", json!({"url": "https://pod.ex/secret/.acl"}));
    assert!(is_err);
    assert_eq!(text, "resource not found: <https://pod.ex/secret/.acl>");
    // Bob still reads s1 — the policy was not touched.
    let (text, is_err) = tool(&mut bob, "resource_get", json!({"url": "https://pod.ex/secret/s1"}));
    assert!(!is_err && text.contains("classified"), "{text}");
}

// ───────────────────────── misc hardening ─────────────────────────

#[test]
fn reserved_graph_space_is_never_a_write_target() {
    let mut s = server_for(ALICE, true);
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "urn:sparq:auth",
            "content": "<urn:a> <urn:b> <urn:c> .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(is_err && text.contains("reserved"), "{text}");
    let (text, is_err) = tool(&mut s, "resource_delete", json!({"url": "urn:sparq:auth"}));
    assert!(is_err && text.contains("reserved"), "{text}");
}

#[test]
fn anonymous_session_fails_closed_everywhere() {
    let mut anon =
        SolidMcpServer::with_config(pod(), SolidServerConfig::default()).expect("materializes");
    assert!(!anon.allow_update(), "default config must be read-only");
    let (text, is_err) = tool(&mut anon, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(is_err && text == "resource not found: <https://pod.ex/notes/n1>");
    let (text, is_err) = tool(&mut anon, "container_list", json!({"url": "https://pod.ex/"}));
    assert!(is_err && text == "resource not found: <https://pod.ex/>");
    let (text, is_err) =
        tool(&mut anon, "query", json!({"sparql": "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }"}));
    assert!(!is_err, "query never errors on authorization: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["results"]["bindings"].as_array().unwrap().len(), 0);
}

#[test]
fn initialize_reports_the_pod_server_name() {
    let mut s = server_for(ALICE, false);
    let resp = rpc(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    assert_eq!(resp["result"]["serverInfo"]["name"], "sparq-mcp-solid");
    assert!(resp["result"]["protocolVersion"].is_string());
}

// [SONNET-4.6] sq-bvnqm: the pod server negotiates the protocol version exactly
// like the base server — accept a supported proposal, offer the latest otherwise.
#[test]
fn pod_initialize_negotiates_the_protocol_version() {
    let mut s = server_for(ALICE, false);
    let resp = rpc(
        &mut s,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");

    let resp = rpc(
        &mut s,
        r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
    );
    assert_eq!(resp["result"]["protocolVersion"], sparq_mcp::PROTOCOL_VERSION);
}

// [OPUS-5] gh #2497: the pod server shares the base server's framing core, so it
// receives JSON-RPC 2.0 §6 batches too — the precondition for negotiating 2025-03-26,
// the one revision that requires them. Authorization is unchanged inside a batch: a
// batched read of a forbidden document is refused exactly as a single one is.
#[test]
fn pod_receives_a_jsonrpc_batch_and_still_enforces_authorization() {
    let mut s = server_for(ALICE, false);
    let resp = rpc(
        &mut s,
        r#"[{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"resource_get","arguments":{"uri":"https://pod.ex/secret/s1"}}}]"#,
    );

    let batch = resp.as_array().expect("a batch is answered with an array");
    assert_eq!(batch.len(), 2, "the notification element gets no entry");
    assert_eq!(batch[0]["id"], 1);
    assert_eq!(
        batch[0]["result"]["protocolVersion"], "2025-03-26",
        "the batch-requiring revision is negotiated verbatim now that batches are received"
    );
    assert_eq!(batch[1]["id"], 2);
    assert!(
        batch[1]["result"]["isError"].as_bool().unwrap_or(false),
        "alice may not read bob's secret, batched or not: {}",
        batch[1]
    );
}

#[test]
fn pod_all_notification_batch_gets_no_response() {
    let mut s = server_for(ALICE, false);
    let out = s.handle_message(
        r#"[{"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","method":"ping"}]"#,
    );
    assert!(out.is_none(), "a batch of only notifications stays silent");
}

// ───────── Class N: the resources surface + notifications (draft §8/§10) ─────────
// [SONNET-4.6] sq-cmjmr. Load-bearing invariants, each red under the matching mutation:
// - the `resources` capability is declared WITH `subscribe: true`;
// - `resources/list` shows exactly the documents this session may read;
// - `resources/subscribe` is authorized and non-disclosing (probe-proof);
// - a state change on a subscribed topic yields a CONTENT-FREE notification;
// - authorization is re-checked at EVERY delivery, and a revoked session goes SILENT
//   (no notification at all — least of all a "your access was revoked" one).

/// The `uri`s of `resources/list` for this session.
fn resource_uris(server: &mut SolidMcpServer) -> Vec<String> {
    let resp = rpc(server, r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#);
    resp["result"]["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .map(|r| r["uri"].as_str().expect("uri").to_string())
        .collect()
}

/// Send one `resources/*` request with a `uri` param, returning the whole response.
fn resources_rpc(server: &mut SolidMcpServer, method: &str, uri: &str) -> Value {
    let req = json!({"jsonrpc": "2.0", "id": 8, "method": method, "params": {"uri": uri}});
    rpc(server, &req.to_string())
}

/// Drain the queued notifications as parsed JSON.
fn drain(server: &mut SolidMcpServer) -> Vec<Value> {
    server.take_notifications().iter().map(|m| parse(m)).collect()
}

#[test]
fn initialize_declares_the_resources_capability_with_subscribe() {
    let mut s = server_for(ALICE, false);
    let resp = rpc(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let caps = &resp["result"]["capabilities"];
    assert!(caps["tools"].is_object(), "the tools capability is still declared");
    assert_eq!(caps["resources"]["subscribe"], true, "Class N requires subscribe:true");
    // No overclaim: this server never pushes an unsolicited list-changed notification.
    assert_eq!(caps["resources"]["listChanged"], false);
}

#[test]
fn resources_list_exposes_only_the_documents_this_session_may_read() {
    let mut alice = server_for(ALICE, false);
    let uris = resource_uris(&mut alice);
    assert!(uris.contains(&"https://pod.ex/notes/n1".to_string()));
    assert!(uris.contains(&"https://pod.ex/notes/unlisted".to_string()));
    assert!(
        !uris.iter().any(|u| u.contains("secret/s1")),
        "a document alice cannot read must be ABSENT, not an error: {uris:?}"
    );
    // The reserved graph space is server machinery, not pod content.
    assert!(!uris.iter().any(|u| u.starts_with("urn:sparq:")), "{uris:?}");
    assert!(uris.windows(2).all(|w| w[0] <= w[1]), "listing is deterministic: {uris:?}");

    // The complement: bob sees the secret subtree and none of alice's documents.
    let mut bob = server_for(BOB, false);
    let bob_uris = resource_uris(&mut bob);
    assert!(bob_uris.contains(&"https://pod.ex/secret/s1".to_string()));
    assert!(!bob_uris.iter().any(|u| u.contains("notes/")), "{bob_uris:?}");
}

#[test]
fn resources_read_serves_the_same_bytes_as_resource_get_and_non_discloses() {
    let mut s = server_for(ALICE, false);
    let resp = resources_rpc(&mut s, "resources/read", "https://pod.ex/notes/n1");
    let contents = &resp["result"]["contents"][0];
    assert_eq!(contents["uri"], "https://pod.ex/notes/n1");
    assert_eq!(contents["mimeType"], "application/n-triples");
    let via_resource = contents["text"].as_str().expect("text").to_string();
    let (tool_text, _) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    let via_tool: Value = serde_json::from_str(&tool_text).expect("JSON");
    assert_eq!(
        via_resource,
        via_tool["content"].as_str().expect("content"),
        "the resource and tool read surfaces cannot disagree"
    );

    // §9.3 carries over: unreadable and nonexistent are the SAME error.
    let denied = resources_rpc(&mut s, "resources/read", "https://pod.ex/secret/s1");
    let absent = resources_rpc(&mut s, "resources/read", "https://pod.ex/secret/nope");
    assert_eq!(denied["error"]["code"], absent["error"]["code"]);
    assert_eq!(
        denied["error"]["message"].as_str().unwrap().replace("s1", "X"),
        absent["error"]["message"].as_str().unwrap().replace("nope", "X")
    );
}

#[test]
fn subscribe_is_authorized_at_subscribe_time_and_cannot_probe_for_resources() {
    let mut s = server_for(ALICE, false);
    let ok = resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/notes/n1");
    assert!(ok["error"].is_null(), "an authorized subscribe succeeds: {ok}");
    assert_eq!(s.subscribed_topics(), vec!["https://pod.ex/notes/n1".to_string()]);

    // An EXISTING document alice may not read, and one that does not exist, must be
    // indistinguishable — otherwise subscribe becomes an existence oracle.
    let denied = resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/secret/s1");
    let absent = resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/secret/nope");
    assert_eq!(denied["error"]["code"], absent["error"]["code"]);
    assert_eq!(
        denied["error"]["message"].as_str().unwrap().replace("s1", "X"),
        absent["error"]["message"].as_str().unwrap().replace("nope", "X")
    );
    assert_eq!(
        s.subscribed_topics(),
        vec!["https://pod.ex/notes/n1".to_string()],
        "a refused subscribe registers nothing"
    );

    // Unsubscribe is idempotent and uniform: an unknown topic gets the same empty result.
    let known = resources_rpc(&mut s, "resources/unsubscribe", "https://pod.ex/notes/n1");
    let unknown = resources_rpc(&mut s, "resources/unsubscribe", "https://pod.ex/secret/s1");
    assert_eq!(known["result"], unknown["result"]);
    assert!(s.subscribed_topics().is_empty());
}

#[test]
fn a_change_to_a_subscribed_topic_emits_one_content_free_notification() {
    let mut s = server_for(ALICE, true);
    resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/notes/n1");
    assert!(drain(&mut s).is_empty(), "subscribing alone emits nothing");

    let (_, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"rewritten\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(!is_err, "alice may write notes/n1");

    let notes = drain(&mut s);
    assert_eq!(notes.len(), 1, "one change, one notification: {notes:?}");
    assert_eq!(notes[0]["jsonrpc"], "2.0");
    assert_eq!(notes[0]["method"], "notifications/resources/updated");
    assert!(notes[0]["id"].is_null(), "a notification carries no id");
    assert_eq!(notes[0]["params"]["uri"], "https://pod.ex/notes/n1");
    assert_eq!(notes[0]["params"]["activity"], "Update");
    // CONTENT-FREE (draft §10): topic + activity type, and nothing else. A leak of the
    // changed triples would flip this red.
    let params = notes[0]["params"].as_object().expect("params");
    assert_eq!(params.len(), 2, "payload must stay content-free: {params:?}");
    assert!(!notes[0].to_string().contains("rewritten"));

    assert!(drain(&mut s).is_empty(), "draining is destructive");

    // An UNSUBSCRIBED topic never produces a notification, however loud the change.
    resources_rpc(&mut s, "resources/unsubscribe", "https://pod.ex/notes/n1");
    tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"again\" .",
            "content_type": "application/n-triples"
        }),
    );
    assert!(drain(&mut s).is_empty(), "no subscription, no notification");
}

#[test]
fn membership_and_lifecycle_changes_carry_their_activitystreams_verb() {
    let mut s = server_for(ALICE, true);
    for topic in ["https://pod.ex/notes/", "https://pod.ex/notes/n1"] {
        let resp = resources_rpc(&mut s, "resources/subscribe", topic);
        assert!(resp["error"].is_null(), "{resp}");
    }

    // Creating a member grows the container's stored ldp:contains ⇒ Add on the container.
    tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n2",
            "content": "<https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"two\" .",
            "content_type": "application/n-triples"
        }),
    );
    let notes = drain(&mut s);
    assert_eq!(notes.len(), 1, "only subscribed topics notify: {notes:?}");
    assert_eq!(notes[0]["params"]["uri"], "https://pod.ex/notes/");
    assert_eq!(notes[0]["params"]["activity"], "Add");

    // Deleting a subscribed document ⇒ Delete on it, Remove on its container.
    let (_, is_err) = tool(&mut s, "resource_delete", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(!is_err);
    let mut verbs: Vec<(String, String)> = drain(&mut s)
        .iter()
        .map(|n| {
            (
                n["params"]["uri"].as_str().unwrap().to_string(),
                n["params"]["activity"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    verbs.sort();
    assert_eq!(
        verbs,
        vec![
            ("https://pod.ex/notes/".to_string(), "Remove".to_string()),
            ("https://pod.ex/notes/n1".to_string(), "Delete".to_string()),
        ]
    );

    // Re-creating it ⇒ Create (the subscription survived the delete).
    tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"back\" .",
            "content_type": "application/n-triples"
        }),
    );
    let verbs: Vec<String> = drain(&mut s)
        .iter()
        .map(|n| format!("{} {}", n["params"]["uri"], n["params"]["activity"]))
        .collect();
    assert!(
        verbs.contains(&"\"https://pod.ex/notes/n1\" \"Create\"".to_string()),
        "{verbs:?}"
    );
}

#[test]
fn a_failed_mutation_emits_nothing() {
    let mut s = server_for(ALICE, true);
    resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/notes/n1");
    let (_, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1",
            "content": "this is not RDF at all",
            "content_type": "text/turtle"
        }),
    );
    assert!(is_err, "a malformed body is rejected");
    assert!(drain(&mut s).is_empty(), "nothing changed ⇒ nothing is signalled");
}

#[test]
fn delivery_re_checks_read_access_and_a_revoked_session_goes_silent() {
    // §10: authorization is checked at subscribe time AND again at every delivery. When
    // alice's own read access to the topic is revoked mid-session, deliveries stop —
    // with NO revocation notification, which would itself disclose the change.
    let mut s = server_for(ALICE, true);
    let resp = resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/notes/");
    assert!(resp["error"].is_null(), "{resp}");

    // Positive control: while readable, a change to the topic IS delivered.
    let (_, is_err) = tool(
        &mut s,
        "update",
        json!({"sparql": "INSERT DATA { GRAPH <https://pod.ex/notes/> \
                          { <https://pod.ex/notes/#c> <https://ex.dev/ns#note> \"one\" } }"}),
    );
    assert!(!is_err, "alice may write the notes container");
    let notes = drain(&mut s);
    assert_eq!(notes.len(), 1, "the control delivery must happen: {notes:?}");
    assert_eq!(notes[0]["params"]["activity"], "Update");

    // Revoke alice's READ on notes/ while keeping Write (so she can still change it).
    let acl = r#"
<https://pod.ex/notes/.acl#w> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .
<https://pod.ex/notes/.acl#w> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/> .
<https://pod.ex/notes/.acl#w> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/notes/> .
<https://pod.ex/notes/.acl#w> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> .
<https://pod.ex/notes/.acl#w> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> .
"#;
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/.acl",
            "content": acl,
            "content_type": "application/n-triples"
        }),
    );
    assert!(!is_err, "alice holds Control on the root by default: {text}");
    let (probe, denied) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/"}));
    assert!(denied, "read access really is gone now: {probe}");
    assert!(drain(&mut s).is_empty(), "the ACL swap itself changed no subscribed topic");

    // The topic changes again — and this time the session hears NOTHING.
    let (_, is_err) = tool(
        &mut s,
        "update",
        json!({"sparql": "INSERT DATA { GRAPH <https://pod.ex/notes/> \
                          { <https://pod.ex/notes/#c> <https://ex.dev/ns#note> \"two\" } }"}),
    );
    assert!(!is_err, "write access survived the revocation");
    assert!(
        drain(&mut s).is_empty(),
        "a session that may no longer read the topic gets NOTHING — not even a \
         revocation notice, which would itself disclose the change"
    );
    assert_eq!(
        s.subscribed_topics(),
        vec!["https://pod.ex/notes/".to_string()],
        "the subscription survives revocation; it is silenced, not torn down"
    );
}

#[test]
fn a_delete_cannot_bypass_a_resource_specific_read_revocation() {
    // §10 again, on the ONE transition whose delivery check cannot use the topic's own
    // policy: a `Delete` is authorized at the nearest surviving ancestor, because the
    // deleted resource has no policy left. That fallback must not RE-GRANT read that a
    // resource-specific ACL had taken away — otherwise deleting the child announces its
    // deletion to a session that could no longer read it.
    let mut s = server_for(ALICE, true);
    let resp = resources_rpc(&mut s, "resources/subscribe", "https://pod.ex/notes/n1");
    assert!(resp["error"].is_null(), "{resp}");

    // A CHILD-specific ACL revokes alice's Read on notes/n1 and keeps Write. The PARENT
    // container keeps its inherited Read — that asymmetry is what the anchor fallback
    // would otherwise launder into a delivery.
    let acl = r#"
<https://pod.ex/notes/n1.acl#w> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .
<https://pod.ex/notes/n1.acl#w> <http://www.w3.org/ns/auth/acl#accessTo> <https://pod.ex/notes/n1> .
<https://pod.ex/notes/n1.acl#w> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> .
<https://pod.ex/notes/n1.acl#w> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> .
"#;
    let (text, is_err) = tool(
        &mut s,
        "resource_put",
        json!({
            "url": "https://pod.ex/notes/n1.acl",
            "content": acl,
            "content_type": "application/n-triples"
        }),
    );
    assert!(!is_err, "alice holds Control on notes/n1 by default: {text}");
    let (probe, denied) = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"}));
    assert!(denied, "read on the CHILD really is revoked: {probe}");
    let (parent, parent_err) =
        tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/"}));
    assert!(!parent_err, "the PARENT stays readable — the anchor is not fail-closed: {parent}");
    assert!(drain(&mut s).is_empty(), "the ACL swap itself changed no subscribed topic");

    // Delete the child through the SPARQL path, which needs Write only — so the deletion
    // is reachable by a session that may no longer READ what it deletes.
    let (text, is_err) =
        tool(&mut s, "update", json!({"sparql": "DROP GRAPH <https://pod.ex/notes/n1>"}));
    assert!(!is_err, "write access survived the revocation: {text}");
    let denied_read = tool(&mut s, "resource_get", json!({"url": "https://pod.ex/notes/n1"})).1;
    assert!(denied_read, "the topic really is gone");
    assert!(
        drain(&mut s).is_empty(),
        "the deletion of a resource this session may no longer read must NOT be \
         announced via its parent container's read grant"
    );
    assert_eq!(
        s.subscribed_topics(),
        vec!["https://pod.ex/notes/n1".to_string()],
        "the subscription survives; it is silenced, not torn down"
    );
}

#[test]
fn resources_requests_reject_a_missing_uri_parameter() {
    let mut s = server_for(ALICE, false);
    for method in ["resources/read", "resources/subscribe", "resources/unsubscribe"] {
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": method, "params": {}});
        let resp = rpc(&mut s, &req.to_string());
        assert_eq!(resp["error"]["code"], -32602, "{method} must be invalid-params");
        assert!(resp["error"]["message"].as_str().unwrap().contains(method));
    }
}

//! [FABLE-5] sq-lrtc3.1 + sq-3mu76 — e2e integration tests for the OPT-IN ODRL lane on
//! `POST /authz/query` and (sq-3mu76, §4) the advisory `POST /authz/decide` /
//! `POST /authz/wac-allow` endpoints (the `odrl-authz` cargo feature; the whole file no-ops
//! without it).
//!
//! Two acceptance pillars (the bead's criteria):
//!
//! 1. **Differential decision test** — for a permit / prohibit / deny-overrides / constraint /
//!    conditional(recipient) / fail-closed matrix, the HTTP-lane decision (does the SAME SPARQL
//!    query return the target graph's rows for this session?) must EQUAL
//!    `sparq_policy::evaluate` over the same `(policy, request)` — the library evaluator is the
//!    decision oracle, the HTTP lane its enforcement.
//! 2. **Two requesters, same query, provably different result sets** — one ODRL policy grants
//!    alice g1 and bob g2; the identical query returns disjoint rows per session.
//!
//! Plus the lane's fail-closed HTTP refusals (anonymous / non-read mode / targetless rule /
//! `odrl:perm` conflict strategy) and the dormant-lane invariant (no ODRL in the dataset =>
//! byte-identical `solid-authz` behaviour).
#![cfg(feature = "odrl-authz")]

use sparq_core::Graph;
use sparq_policy::{evaluate, parse_policy_str, Request, Value, ODRL_RECIPIENT};
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const N1: &str = "https://pod.ex/notes/n1";
const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

/// A minimal boot store — `/authz/*` authorises over the request-body dataset, never this.
const BOOT: &str = "@prefix ex: <http://example.org/> . ex:alice a ex:Person .";

/// One content graph (n1) with NO static ACL — every grant must come from the ODRL bridge.
const CONTENT: &str = "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n";

/// The SAME query every session issues: the n1 titles this session is allowed to see.
const QUERY: &str = "SELECT ?o WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?o } }";

async fn spawn() -> String {
    let graph = Graph::load_str(BOOT, "turtle").unwrap();
    let config = ServerConfig { solid_authz: true, ..ServerConfig::default() };
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

/// POST /authz/query as `agent` (anonymous when `None`), optionally with a session `now`.
async fn query_as(
    base: &str,
    dataset: &str,
    agent: Option<&str>,
    now: Option<&str>,
    mode: Option<&str>,
) -> reqwest::Response {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    if let Some(n) = now {
        session.insert("now".into(), n.into());
    }
    let mut body = serde_json::json!({
        "dataset": dataset,
        "session": session,
        "query": QUERY,
        "view": "wac",
    });
    if let Some(m) = mode {
        body["mode"] = m.into();
    }
    client()
        .post(format!("{base}/authz/query"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// The number of result rows in a SPARQL 1.1 JSON results body.
async fn row_count(resp: reqwest::Response) -> usize {
    assert_eq!(resp.status(), 200, "query lane must succeed for this row");
    let v: serde_json::Value = resp.json().await.unwrap();
    v["results"]["bindings"].as_array().map(|a| a.len()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 1. The differential decision matrix: HTTP-lane visibility == sparq_policy::evaluate.
// ---------------------------------------------------------------------------

/// One matrix row: an ODRL policy (default-graph N-Quads), the requesting agent, and the
/// request `now` (when the row exercises a dateTime constraint).
struct MatrixRow {
    name: &'static str,
    policy_nq: String,
    agent: &'static str,
    now: Option<&'static str>,
}

fn permit(assignee: &str, extra: &str) -> String {
    format!(
        "<urn:pol/p> <{ODRL}permission> _:perm .\n\
         _:perm <{ODRL}action> <{ODRL}read> .\n\
         _:perm <{ODRL}target> <{N1}> .\n\
         _:perm <{ODRL}assignee> <{assignee}> .\n{extra}"
    )
}

fn prohibit(assignee: &str) -> String {
    format!(
        "<urn:pol/p> <{ODRL}prohibition> _:proh .\n\
         _:proh <{ODRL}action> <{ODRL}read> .\n\
         _:proh <{ODRL}target> <{N1}> .\n\
         _:proh <{ODRL}assignee> <{assignee}> .\n"
    )
}

/// A `_:perm odrl:constraint` block (leftOperand / operator / rightOperand).
fn constraint(left: &str, op: &str, right_term: &str) -> String {
    format!(
        "_:perm <{ODRL}constraint> _:c .\n\
         _:c <{ODRL}leftOperand> <{ODRL}{left}> .\n\
         _:c <{ODRL}operator> <{ODRL}{op}> .\n\
         _:c <{ODRL}rightOperand> {right_term} .\n"
    )
}

const DT: &str = "^^<http://www.w3.org/2001/XMLSchema#dateTime>";

fn matrix() -> Vec<MatrixRow> {
    vec![
        MatrixRow {
            name: "bare permit, the assignee",
            policy_nq: permit(ALICE, ""),
            agent: ALICE,
            now: None,
        },
        MatrixRow {
            name: "bare permit, a NON-assignee (fail-closed)",
            policy_nq: permit(ALICE, ""),
            agent: BOB,
            now: None,
        },
        MatrixRow {
            name: "permit + matching prohibition (deny-overrides)",
            policy_nq: format!("{}{}", permit(ALICE, ""), prohibit(ALICE)),
            agent: ALICE,
            now: None,
        },
        MatrixRow {
            name: "dateTime constraint satisfied (strict lt, now inside)",
            policy_nq: permit(
                ALICE,
                &constraint("dateTime", "lt", &format!("\"2030-01-01T00:00:00Z\"{DT}")),
            ),
            agent: ALICE,
            now: Some("2026-01-01T00:00:00Z"),
        },
        MatrixRow {
            name: "dateTime constraint violated (strict lt, now past the bound)",
            policy_nq: permit(
                ALICE,
                &constraint("dateTime", "lt", &format!("\"2030-01-01T00:00:00Z\"{DT}")),
            ),
            agent: ALICE,
            now: Some("2031-01-01T00:00:00Z"),
        },
        MatrixRow {
            name: "conditional: recipient eq the requester (delivery-recipient context)",
            policy_nq: permit(ALICE, &constraint("recipient", "eq", &format!("<{ALICE}>"))),
            agent: ALICE,
            now: None,
        },
        MatrixRow {
            name: "conditional: recipient eq someone ELSE (fail-closed)",
            policy_nq: permit(ALICE, &constraint("recipient", "eq", &format!("<{BOB}>"))),
            agent: ALICE,
            now: None,
        },
        MatrixRow {
            name: "purpose constraint with NO purpose evidence (fail-closed)",
            policy_nq: permit(ALICE, &constraint("purpose", "eq", "<https://ex.dev/marketing>")),
            agent: ALICE,
            now: None,
        },
        MatrixRow {
            name: "count constraint on the stateless lane (fail-closed; stateful budgets unimplemented)",
            policy_nq: permit(
                ALICE,
                &constraint(
                    "count",
                    "lteq",
                    "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                ),
            ),
            agent: ALICE,
            now: None,
        },
    ]
}

/// The library-side oracle: `sparq_policy::evaluate` over the SAME `(policy, request)` the
/// HTTP lane materialises — party = the session agent, action = `odrl:read`, target = n1,
/// recipient = the requesting agent (the delivery recipient of the results), `at` = session now.
fn oracle_allows(row: &MatrixRow) -> bool {
    let policy = parse_policy_str(&row.policy_nq, "ntriples").expect("row policy parses");
    let mut request = Request::new(format!("{ODRL}read"))
        .on(N1)
        .by(row.agent)
        .with(ODRL_RECIPIENT, Value::Iri(row.agent.to_owned()));
    if let Some(now) = row.now {
        request = request.at(now);
    }
    evaluate(&policy, &request).allow
}

#[tokio::test]
async fn http_lane_decision_equals_library_evaluate_over_the_matrix() {
    let base = spawn().await;
    for row in matrix() {
        let dataset = format!("{CONTENT}{}", row.policy_nq);
        let resp = query_as(&base, &dataset, Some(row.agent), row.now, None).await;
        let visible = row_count(resp).await > 0;
        let expected = oracle_allows(&row);
        assert_eq!(
            visible, expected,
            "row '{}': HTTP-lane visibility ({visible}) diverged from \
             sparq_policy::evaluate ({expected})",
            row.name
        );
    }
    // The matrix is non-degenerate: it exercises BOTH decisions (guards vacuity —
    // a lane that always denies or always allows cannot pass this).
    let decisions: Vec<bool> = matrix().iter().map(oracle_allows).collect();
    assert!(decisions.iter().any(|d| *d) && decisions.iter().any(|d| !*d));
}

// ---------------------------------------------------------------------------
// 2. Two requesters, SAME query => provably different result sets.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_requesters_same_query_get_different_result_sets() {
    let base = spawn().await;
    // Two content graphs; ONE policy: alice may read g1, bob may read g2.
    let dataset = format!(
        "<https://pod.ex/a#it> <https://ex.dev/ns#title> \"alpha\" <https://pod.ex/a> .\n\
         <https://pod.ex/b#it> <https://ex.dev/ns#title> \"beta\" <https://pod.ex/b> .\n\
         <urn:pol/p> <{ODRL}permission> _:p1 .\n\
         _:p1 <{ODRL}action> <{ODRL}read> .\n\
         _:p1 <{ODRL}target> <https://pod.ex/a> .\n\
         _:p1 <{ODRL}assignee> <{ALICE}> .\n\
         <urn:pol/p> <{ODRL}permission> _:p2 .\n\
         _:p2 <{ODRL}action> <{ODRL}read> .\n\
         _:p2 <{ODRL}target> <https://pod.ex/b> .\n\
         _:p2 <{ODRL}assignee> <{BOB}> .\n"
    );
    let titles = |v: serde_json::Value| -> Vec<String> {
        v["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["o"]["value"].as_str().unwrap().to_owned())
            .collect()
    };
    let alice_resp = query_as(&base, &dataset, Some(ALICE), None, None).await;
    assert_eq!(alice_resp.status(), 200);
    let alice_rows = titles(alice_resp.json().await.unwrap());
    let bob_resp = query_as(&base, &dataset, Some(BOB), None, None).await;
    assert_eq!(bob_resp.status(), 200);
    let bob_rows = titles(bob_resp.json().await.unwrap());
    // The SAME query, two sessions: disjoint, non-empty, policy-scoped results.
    assert_eq!(alice_rows, vec!["alpha".to_owned()], "alice sees exactly her granted graph");
    assert_eq!(bob_rows, vec!["beta".to_owned()], "bob sees exactly his granted graph");
}

// ---------------------------------------------------------------------------
// 3. Fail-closed HTTP refusals — NEVER a silent allow.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lane_refusals_are_4xx_fail_closed() {
    let base = spawn().await;
    let permit_ds = format!("{CONTENT}{}", permit(ALICE, ""));

    // Anonymous session + ODRL in the dataset => 403 (a prohibition could not bind).
    let resp = query_as(&base, &permit_ds, None, None, None).await;
    assert_eq!(resp.status(), 403, "anonymous + ODRL must refuse");

    // A non-read mode => 400 (the lane covers the query/read action only).
    let resp = query_as(&base, &permit_ds, Some(ALICE), None, Some("write")).await;
    assert_eq!(resp.status(), 400, "non-read mode + ODRL must refuse");

    // A targetless prohibition => 400 (under-materialising it would widen access).
    let targetless = format!(
        "{CONTENT}<urn:pol/p> <{ODRL}prohibition> _:proh .\n\
         _:proh <{ODRL}action> <{ODRL}read> .\n\
         _:proh <{ODRL}assignee> <{ALICE}> .\n"
    );
    let resp = query_as(&base, &targetless, Some(ALICE), None, None).await;
    assert_eq!(resp.status(), 400, "targetless rule must refuse");

    // An unimplementable odrl:conflict strategy (odrl:perm) => 400.
    let perm_strategy = format!(
        "{CONTENT}{}<urn:pol/p> <{ODRL}conflict> <{ODRL}perm> .\n",
        permit(ALICE, "")
    );
    let resp = query_as(&base, &perm_strategy, Some(ALICE), None, None).await;
    assert_eq!(resp.status(), 400, "odrl:perm conflict strategy must refuse");
}

// ---------------------------------------------------------------------------
// 4. [FABLE-5] sq-3mu76 — the lane on the ADVISORY endpoints (`/authz/decide`,
//    `/authz/wac-allow`): a prohibition is never reported past; read-scoped
//    advertisement; the query lane's refusal matrix.
// ---------------------------------------------------------------------------

const ACL: &str = "http://www.w3.org/ns/auth/acl#";

/// The n1 content plus a root `.acl`: alice Read (+ the caller's `extra` `.acl` quads).
fn wac_dataset(extra_acl: &str) -> String {
    format!(
        "{CONTENT}\
         <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACL}Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <{ACL}agent> <{ALICE}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#owner> <{ACL}mode> <{ACL}Read> <https://pod.ex/.acl> .\n\
         {extra_acl}"
    )
}

/// A second `.acl` authorization: alice Write + PUBLIC (`foaf:Agent`) Read on the pod.
fn extra_write_and_public_read() -> String {
    format!(
        "<https://pod.ex/.acl#w> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACL}Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#w> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#w> <{ACL}agent> <{ALICE}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#w> <{ACL}mode> <{ACL}Write> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#pub> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ACL}Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#pub> <{ACL}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#pub> <{ACL}agentClass> <http://xmlns.com/foaf/0.1/Agent> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#pub> <{ACL}mode> <{ACL}Read> <https://pod.ex/.acl> .\n"
    )
}

/// POST /authz/decide as `agent` (anonymous when `None`) for `(N1, mode)`.
async fn decide_as(base: &str, dataset: &str, agent: Option<&str>, mode: &str) -> reqwest::Response {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    let body = serde_json::json!({
        "dataset": dataset,
        "session": session,
        "resource": N1,
        "mode": mode,
        "view": "wac",
    });
    client().post(format!("{base}/authz/decide")).json(&body).send().await.unwrap()
}

/// POST /authz/wac-allow as `agent` (anonymous when `None`) for N1.
async fn wac_allow_as(base: &str, dataset: &str, agent: Option<&str>) -> reqwest::Response {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    let body = serde_json::json!({
        "dataset": dataset,
        "session": session,
        "resource": N1,
        "view": "wac",
    });
    client().post(format!("{base}/authz/wac-allow")).json(&body).send().await.unwrap()
}

/// THE sq-3mu76 inconsistency, witnessed then fixed: a dataset-carried ODRL prohibition used to
/// get a `/authz/decide` allow that `/authz/query` refused to honour. The differential pair
/// (same static WAC grant, ± the prohibition) also proves the lane — not the fixture — flips
/// the decision (knocking the endpoint's `apply_odrl_lane` call out turns this red).
#[tokio::test]
async fn decide_prohibition_deny_overrides_static_wac_allow() {
    let base = spawn().await;
    // Control: the static WAC grant alone decides allow (and the lane stays dormant).
    let resp = decide_as(&base, &wac_dataset(""), Some(ALICE), "read").await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["allow"], serde_json::Value::Bool(true));
    // The SAME dataset + an ODRL prohibition of alice's read: deny-overrides, exactly what
    // `/authz/query` enforces (403 = an authoritative resolved deny).
    let prohibited = format!("{}{}", wac_dataset(""), prohibit(ALICE));
    let resp = decide_as(&base, &prohibited, Some(ALICE), "read").await;
    assert_eq!(resp.status(), 403, "an ODRL prohibition must deny the advisory decision");
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["allow"], serde_json::Value::Bool(false));
    assert!(
        v["grantedModes"].as_array().unwrap().is_empty(),
        "the denied read must not stay advertised"
    );
}

/// The grant side composes too: a bridged ODRL permit (bob has NO static grant) decides allow
/// through the same `∪ allow ∖ ∪ deny` index the query lane enforces.
#[tokio::test]
async fn decide_odrl_permit_grants_read_through_the_bridge() {
    let base = spawn().await;
    let dataset = format!("{}{}", wac_dataset(""), permit(BOB, ""));
    // Control: without the permit, bob is denied (the .acl grants alice only).
    let resp = decide_as(&base, &wac_dataset(""), Some(BOB), "read").await;
    assert_eq!(resp.status(), 403);
    // With the ODRL permit the bridge materialises bob's read grant.
    let resp = decide_as(&base, &dataset, Some(BOB), "read").await;
    assert_eq!(resp.status(), 200, "a bridged ODRL permit must decide allow");
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["allow"], serde_json::Value::Bool(true));
    assert_eq!(
        v["grantedModes"],
        serde_json::json!(["read"]),
        "read-scoped advertisement: exactly the lane-evidenced mode"
    );
}

/// Read-scoped advertisement on `/authz/decide`: when the lane fires, a static WAC write grant
/// is NOT advertised alongside the decided read (the lane evidenced no write-action rule —
/// sq-lrtc3.2); dormant (no ODRL), the full mode set is advertised as before.
#[tokio::test]
async fn decide_masks_advertised_modes_to_read_when_lane_fires() {
    let base = spawn().await;
    let wac = wac_dataset(&extra_write_and_public_read());
    // Dormant control: alice's advertisement carries read AND write.
    let resp = decide_as(&base, &wac, Some(ALICE), "read").await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    let modes: Vec<&str> =
        v["grantedModes"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    assert!(modes.contains(&"read") && modes.contains(&"write"));
    // Lane fired (a permit alice already holds statically): the decision is unchanged,
    // the advertisement is scoped to the evidenced read.
    let resp =
        decide_as(&base, &format!("{}{}", wac, permit(ALICE, "")), Some(ALICE), "read").await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["allow"], serde_json::Value::Bool(true));
    assert_eq!(v["grantedModes"], serde_json::json!(["read"]));
}

/// `/authz/decide` inherits the query lane's refusal matrix: non-read mode => 400
/// (sq-lrtc3.2 owns the wider action contract), anonymous session => 403.
#[tokio::test]
async fn decide_refusals_match_the_query_lane() {
    let base = spawn().await;
    let dataset = format!("{}{}", wac_dataset(&extra_write_and_public_read()), permit(ALICE, ""));
    let resp = decide_as(&base, &dataset, Some(ALICE), "write").await;
    assert_eq!(resp.status(), 400, "non-read mode + ODRL must refuse");
    let resp = decide_as(&base, &dataset, None, "read").await;
    assert_eq!(resp.status(), 403, "anonymous + ODRL must refuse");
    // Dormant control: WITHOUT ODRL the same write-mode decide is an ordinary decision
    // (alice holds Write => allow), proving the 400 above is the lane, not the endpoint.
    let resp =
        decide_as(&base, &wac_dataset(&extra_write_and_public_read()), Some(ALICE), "write").await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["allow"], serde_json::Value::Bool(true));
}

/// `/authz/wac-allow` under the lane: a read prohibition is never advertised past, and the
/// advertisement is read-scoped — `user` at most `read`, `public` ALWAYS empty (an anonymous
/// session is refused wherever the lane fires, so public access is never evidenced).
#[tokio::test]
async fn wac_allow_is_read_scoped_and_never_advertises_a_prohibited_read() {
    let base = spawn().await;
    let wac = wac_dataset(&extra_write_and_public_read());
    // Dormant control: the full static advertisement (user read+write, public read).
    let resp = wac_allow_as(&base, &wac, Some(ALICE)).await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    let dormant = v["wacAllow"].as_str().unwrap().to_owned();
    assert!(dormant.contains("write") && dormant.contains(r#"public="read""#));
    // Lane fired, permit only: read survives, write/public are scoped out (fail-closed).
    let resp = wac_allow_as(&base, &format!("{}{}", wac, permit(ALICE, "")), Some(ALICE)).await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["wacAllow"], r#"user="read",public="""#);
    // Lane fired, prohibition: the statically-granted read is NOT advertised past the deny.
    let resp = wac_allow_as(&base, &format!("{}{}", wac, prohibit(ALICE)), Some(ALICE)).await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["wacAllow"], r#"user="",public="""#);
    // And the refusal matrix holds here too: anonymous + ODRL => 403.
    let resp = wac_allow_as(&base, &format!("{}{}", wac, permit(ALICE, "")), None).await;
    assert_eq!(resp.status(), 403);
}

/// [FABLE-5] sq-3mu76 — the trust dispatch decides over its OWN store and would bypass the
/// ODRL lane; the combination is refused up front rather than silently dropping a prohibition.
#[cfg(feature = "solid-authz-trust")]
#[tokio::test]
async fn decide_refuses_trust_block_combined_with_odrl() {
    let graph = Graph::load_str(BOOT, "turtle").unwrap();
    let config = ServerConfig {
        solid_authz: true,
        solid_authz_trust: true,
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let dataset = format!("{}{}", wac_dataset(""), prohibit(ALICE));
    let body = serde_json::json!({
        "dataset": dataset,
        "session": {"agent": ALICE},
        "resource": N1,
        "mode": "read",
        "view": "wac",
        "trust": {"rules": [], "credentials": [], "certifications": []},
    });
    let resp = client().post(format!("{base}/authz/decide")).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 400, "trust + ODRL composition must refuse (fail-closed)");
}

// ---------------------------------------------------------------------------
// 5. Dormant lane: no ODRL in the dataset => plain solid-authz behaviour.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lane_is_dormant_without_odrl_in_the_dataset() {
    let base = spawn().await;
    // The module's static-WAC shape: alice holds an inherited Read grant on n1.
    let wac = "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n\
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n\
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n";
    // Anonymous is NOT refused (no ODRL => the lane never fires) — it just sees nothing.
    let resp = query_as(&base, wac, None, None, None).await;
    assert_eq!(row_count(resp).await, 0, "anonymous WAC-only session sees nothing");
    // And the static WAC grant still works untouched.
    let resp = query_as(&base, wac, Some(ALICE), None, None).await;
    assert_eq!(row_count(resp).await, 1, "static WAC grant admits alice as before");
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-snopa.8 — the stateful lane REFUSES an ODRL-carrying server store.
//
// The ODRL lane reads the request-body dataset; `"source":"server"` carries none. If the
// server's OWN store held ODRL rules and the stateful lane simply ignored them, a PROHIBITION
// would be silently dropped — fail-OPEN. So the lane refuses the request outright instead.
// ---------------------------------------------------------------------------

/// Boots a server whose own loaded store IS `dataset`.
async fn spawn_pod(dataset: &str) -> String {
    let graph = Graph::load_dataset(dataset, "nquads").unwrap();
    let config = ServerConfig { solid_authz: true, ..ServerConfig::default() };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A server store carrying an ODRL PROHIBITION is refused by the stateful lane (400) rather
/// than authorised as if the prohibition were absent.
///
/// MUTATION SPOT-CHECK: delete the `graph_carries_odrl` refusal in `build_server_view` and this
/// test goes red — the request would be answered from a view that never saw the prohibition.
#[tokio::test]
async fn stateful_lane_refuses_an_odrl_carrying_server_store() {
    let dataset = format!("{}{}", CONTENT, prohibit(ALICE));
    let base = spawn_pod(&dataset).await;
    let resp = client()
        .post(format!("{}/authz/query", base))
        .json(&serde_json::json!({
            "source": "server",
            "session": { "agent": ALICE },
            "query": QUERY,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "an ODRL-carrying server store must be refused, never silently un-enforced"
    );
}

/// The refusal is SCOPED to ODRL-carrying stores: a plain WAC pod still serves the stateful
/// lane normally (the guard is not a blanket disable of `"source":"server"`).
#[tokio::test]
async fn stateful_lane_still_serves_a_plain_server_store_under_odrl_authz() {
    let base = spawn_pod(&wac_dataset("")).await;
    let resp = client()
        .post(format!("{}/authz/query", base))
        .json(&serde_json::json!({
            "source": "server",
            "session": { "agent": ALICE },
            "query": QUERY,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "a plain WAC server store is not refused");
    assert_eq!(row_count(resp).await, 1, "alice's static WAC grant still admits her");
}

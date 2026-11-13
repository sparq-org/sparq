//! Status/body parity captured from origin/main with regenerate_auth_ledger.py.
use super::*;
use crate::http::ServerConfig;
use serde_json::{json, Value};

fn cases() -> Vec<(String, Value)> {
    let mut cases = Vec::new();
    let wac = concat!(
        "<https://pod.ex/n1> <urn:p> \"hello\" <https://pod.ex/n1> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
    );
    let acp = [
        (
            "https://pod.ex/n1.acr",
            "http://www.w3.org/ns/solid/acp#accessControl",
            "https://pod.ex/n1.acr#c",
        ),
        (
            "https://pod.ex/n1.acr#c",
            "http://www.w3.org/ns/solid/acp#apply",
            "https://pod.ex/n1.acr#p",
        ),
        (
            "https://pod.ex/n1.acr#p",
            "http://www.w3.org/ns/solid/acp#allow",
            "http://www.w3.org/ns/auth/acl#Read",
        ),
        (
            "https://pod.ex/n1.acr#p",
            "http://www.w3.org/ns/solid/acp#allOf",
            "https://pod.ex/n1.acr#m",
        ),
        (
            "https://pod.ex/n1.acr#m",
            "http://www.w3.org/ns/solid/acp#agent",
            "https://alice.ex/card#me",
        ),
    ]
    .into_iter()
    .map(|(s, p, o)| format!("<{s}> <{p}> <{o}> <https://pod.ex/n1.acr> .\n"))
    .collect::<String>()
        + "<https://pod.ex/n1> <urn:p> \"hello\" <https://pod.ex/n1> .\n";
    for (name, dataset) in [
        ("wac", wac),
        ("acp", acp.as_str()),
        ("empty", ""),
        ("invalid", "invalid N-Quads"),
    ] {
        for agent in [
            None,
            Some("https://alice.ex/card#me"),
            Some("https://bob.ex/card#me"),
        ] {
            for mode in ["read", "write"] {
                cases.push((format!("{name}/{agent:?}/{mode}"), json!({
                    "dataset": dataset, "resource": "https://pod.ex/n1", "mode": mode,
                    "session": {"agent":agent}, "query":"SELECT ?o WHERE { GRAPH <https://pod.ex/n1> { ?s <urn:p> ?o } }",
                })));
            }
        }
    }
    #[cfg(feature = "odrl-authz")]
    for effect in ["permission", "prohibition"] {
        let policy = format!("<urn:policy> <http://www.w3.org/ns/odrl/2/{effect}> _:r .\n_:r <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n_:r <http://www.w3.org/ns/odrl/2/target> <https://pod.ex/n1> .\n_:r <http://www.w3.org/ns/odrl/2/assignee> <https://alice.ex/card#me> .\n");
        let mut request = cases[2].1.clone();
        request["dataset"] = json!(format!("{wac}{policy}"));
        request["session"] = json!({"agent":"https://alice.ex/card#me"});
        cases.push((effect.into(), request));
    }
    #[cfg(feature = "odrl-authz")]
    {
        let mut policy = String::from("<urn:policy> <http://www.w3.org/ns/odrl/2/permission> _:r .\n_:r <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n_:r <http://www.w3.org/ns/odrl/2/target> <https://pod.ex/n1> .\n_:r <http://www.w3.org/ns/odrl/2/constraint> _:c0 .\n");
        for i in 0..34 {
            policy.push_str(&format!("_:c{i} <http://www.w3.org/ns/odrl/2/and> _:l{i} .\n_:l{i} <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> _:c{} .\n_:l{i} <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .\n", i+1));
        }
        policy.push_str("_:c34 <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/purpose> .\n_:c34 <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> .\n_:c34 <http://www.w3.org/ns/odrl/2/rightOperand> \"research\" .\n");
        let mut request = cases[2].1.clone();
        request["dataset"] = json!(format!("{wac}{policy}"));
        request["session"] = json!({"agent":"https://alice.ex/card#me"});
        cases.push(("policy-depth-cap".into(), request));
    }
    cases
}

#[tokio::test]
async fn stateless_auth_ledger_equivalence() {
    let state = AppState::with_config(
        sparq_core::Graph::default(),
        ServerConfig {
            solid_authz: true,
            ..Default::default()
        },
    );
    let mut snapshots = serde_json::Map::new();
    for (name, body) in cases() {
        for endpoint in ["decide", "wac-allow", "query"] {
            let body = Bytes::from(body.to_string());
            let response = match endpoint {
                "decide" => decide_endpoint(State(state.clone()), HeaderMap::new(), body).await,
                "wac-allow" => {
                    wac_allow_endpoint(State(state.clone()), HeaderMap::new(), body).await
                }
                _ => query_endpoint(State(state.clone()), HeaderMap::new(), body).await,
            };
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            snapshots.insert(
                format!("{name}/{endpoint}"),
                json!({"status":status,"body":text}),
            );
        }
    }
    let snapshot = Value::Object(snapshots);
    if let Some(output) = std::env::var_os("AUTH_SERVER_GOLDEN") {
        std::fs::write(
            output,
            serde_json::to_string_pretty(&snapshot).unwrap() + "\n",
        )
        .unwrap();
    } else {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/auth-ledger-main.json")).unwrap();
        let expected = fixture["responses"].clone();
        #[cfg(not(feature = "odrl-authz"))]
        let expected = Value::Object(
            expected
                .as_object()
                .unwrap()
                .iter()
                .filter(|(name, _)| {
                    !name.starts_with("permission/")
                        && !name.starts_with("prohibition/")
                        && !name.starts_with("policy-depth-cap/")
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        );
        assert_eq!(snapshot, expected);
    }
}

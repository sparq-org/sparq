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
    cases.extend(boundary_cases());
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
        let config = ServerConfig {
            solid_authz: true,
            #[cfg(feature = "solid-authz-trust")]
            solid_authz_trust: body.get("trust").is_some(),
            ..Default::default()
        };
        let state = if body.get("trust").is_some() {
            AppState::with_config(sparq_core::Graph::default(), config)
        } else {
            state.clone()
        };
        for endpoint in ["decide", "wac-allow", "query"] {
            if body.get("trust").is_some() && endpoint != "decide" {
                continue;
            }
            let body = Bytes::from(body.to_string());
            let response = if name.starts_with("transport-boundary/") {
                use tower::ServiceExt;
                crate::router(state.clone())
                    .oneshot(
                        axum::http::Request::builder()
                            .method("POST")
                            .uri(format!("/authz/{endpoint}"))
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
            } else {
                match endpoint {
                    "decide" => decide_endpoint(State(state.clone()), HeaderMap::new(), body).await,
                    "wac-allow" => {
                        wac_allow_endpoint(State(state.clone()), HeaderMap::new(), body).await
                    }
                    _ => query_endpoint(State(state.clone()), HeaderMap::new(), body).await,
                }
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
        let expected = Value::Object(
            expected
                .as_object()
                .unwrap()
                .iter()
                .filter(|(name, _)| {
                    (cfg!(feature = "odrl-authz")
                        || !(name.starts_with("permission/")
                            || name.starts_with("prohibition/")
                            || name.starts_with("policy-depth-cap/")
                            || name.starts_with("odrl-boundary/")))
                        && (cfg!(feature = "solid-authz-trust")
                            || !name.starts_with("trust-boundary/"))
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        );
        assert_eq!(
            snapshot.as_object().unwrap().keys().collect::<Vec<_>>(),
            expected.as_object().unwrap().keys().collect::<Vec<_>>()
        );
        for (name, response) in snapshot.as_object().unwrap() {
            if response == &expected[name] {
                continue;
            }
            // The documented verify-budget wording replaces the old pair-only label
            // on these refused blocks. Status and every other response byte still match.
            let mut expected_response = expected[name].clone();
            if name.starts_with("trust-boundary/budget/")
                && response["status"] == 403
                && expected_response["status"] == 403
            {
                let body = expected_response["body"].as_str().unwrap();
                expected_response["body"] = json!(body.replacen(
                    "\"trustDenyReason\":\"trust rule/credential pair limit exceeded\"",
                    "\"trustDenyReason\":\"trust verify budget exceeded\"",
                    1
                ));
            }
            assert_eq!(response, &expected_response, "{name}");
        }
    }
}

fn base_request(dataset: String) -> Value {
    json!({"dataset":dataset,"resource":"https://pod.ex/n1","mode":"read",
        "session":{"agent":"https://alice.ex/card#me"},
        "query":"SELECT ?o WHERE { GRAPH <https://pod.ex/n1> { ?s <urn:p> ?o } }"})
}
fn boundary_acl() -> &'static str {
    concat!(
        "<https://pod.ex/n1> <urn:p> \"hello\" <https://pod.ex/n1> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n")
}
fn dense_dataset(rows: usize, duplicate: bool, odrl: bool) -> String {
    let mut data = boundary_acl().to_owned();
    for i in 0..rows {
        data.push_str(&format!(
            "<a:{:x}> <a:> <a:> <a:>.\n",
            if duplicate { 0 } else { i }
        ));
    }
    if odrl {
        data.push_str(concat!(
            "<a:policy> <http://www.w3.org/ns/odrl/2/permission> _:r .\n",
            "_:r <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n",
            "_:r <http://www.w3.org/ns/odrl/2/target> <https://pod.ex/n1> .\n",
            "_:r <http://www.w3.org/ns/odrl/2/assignee> <https://alice.ex/card#me> .\n"
        ));
    }
    data
}
fn resource_dataset(count: usize, length: usize) -> String {
    let mut data = boundary_acl().to_owned();
    for i in 0..count {
        let prefix = format!("https://pod.ex/{i}");
        let iri = format!("{prefix}{}", "x".repeat(length - prefix.len()));
        data.push_str(&format!("<a:s> <a:p> <a:o> <{iri}> .\n"));
    }
    data
}
fn boundary_cases() -> Vec<(String, Value)> {
    let mut cases = Vec::new();
    for rows in [32000, 32001, 33000, 40000] {
        for duplicate in [false, true] {
            let request = base_request(dense_dataset(rows, duplicate, false));
            cases.push((
                format!("boundary/content/{rows}/{duplicate}"),
                request.clone(),
            ));
            #[cfg(feature = "solid-authz-trust")]
            {
                let mut trust = trust_boundary_body(1, 1, 1, 0);
                trust["dataset"] = request["dataset"].clone();
                cases.push((format!("trust-boundary/content/{rows}/{duplicate}"), trust));
            }
        }
    }
    #[cfg(feature = "odrl-authz")]
    for rows in [16500, 20000, 25000, 31000, 40000] {
        cases.push((
            format!("odrl-boundary/content/{rows}"),
            base_request(dense_dataset(rows, false, true)),
        ));
    }
    for extra in [0, 1] {
        let mut data = boundary_acl().to_owned();
        // The base input contributes its resource, linkage, control and principal facts.
        for i in 0..(32000 - 10 + extra) {
            data.push_str(&format!("<a:> <a:> <a:> <r:{i}>.\n"));
        }
        cases.push((
            format!("boundary/assembled-facts/{extra}"),
            base_request(data),
        ));
    }
    for extra in [0, 1] {
        let mut data = dense_body(false);
        if extra > 0 {
            data.push(' ');
        }
        cases.push((format!("transport-boundary/{extra}"), base_request(data)));
    }
    for odrl in [false, true] {
        #[cfg(not(feature = "odrl-authz"))]
        if odrl {
            continue;
        }
        let data = format!(
            "{}<a:s> <a:p> \"{}\" <a:g>.\n",
            dense_dataset(1, false, odrl),
            "x".repeat(5000)
        );
        cases.push((
            format!(
                "{}long-content-term",
                if odrl { "odrl-boundary/" } else { "boundary/" }
            ),
            base_request(data),
        ));
    }
    for (count, length) in [(2000, 300), (4000, 200), (6000, 120)] {
        cases.push((
            format!("boundary/resources/{count}/{length}"),
            base_request(resource_dataset(count, length)),
        ));
    }
    for extra in [0, 1] {
        cases.push((
            format!("boundary/resource-bytes/{extra}"),
            base_request(resource_byte_boundary(extra)),
        ));
    }
    #[cfg(feature = "odrl-authz")]
    for members in [9998, 9999] {
        let mut body = base_request(collection_boundary_dataset(members));
        body["query"] =
            json!("SELECT ?o WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?o } } ORDER BY ?o");
        cases.push((format!("odrl-boundary/collection/{members}"), body));
    }
    for odrl in [false, true] {
        #[cfg(not(feature = "odrl-authz"))]
        if odrl {
            continue;
        }
        cases.push((
            format!(
                "{}dense-source",
                if odrl { "odrl-boundary/" } else { "boundary/" }
            ),
            base_request(dense_body(odrl)),
        ));
    }
    #[cfg(feature = "solid-authz-trust")]
    for (rules, creds, certs) in [(16, 256, 0), (17, 241, 0), (2, 1, 2047), (1, 1, 2048)] {
        cases.push((
            format!("trust-boundary/budget/{rules}/{creds}/{certs}"),
            trust_boundary_body(rules, creds, 1, certs),
        ));
    }
    cases
}

#[cfg(feature = "solid-authz-trust")]
fn trust_boundary_body(
    rules: usize,
    credentials: usize,
    triples: usize,
    certs: usize,
) -> serde_json::Value {
    use oxrdf::{Literal, NamedNode, Triple};
    use sparq_zk::{
        commit::commit_triples,
        encode::salt_from_bytes,
        sig::{public_key_to_hex, SecretKey},
    };
    let key = SecretKey::from_seed(41);
    let graph: Vec<_> = (0..triples)
        .map(|i| {
            Triple::new(
                NamedNode::new("https://alice.ex/card#me").unwrap(),
                NamedNode::new("urn:claim").unwrap(),
                Literal::from(i as i64),
            )
        })
        .collect();
    let commitment = commit_triples(&graph, salt_from_bytes(&[7; 32])).unwrap();
    let credential = serde_json::json!({
        "graphNquads": graph.iter().map(|t| format!("{t} .\n")).collect::<String>(),
        "issuerSignatureHex": key.sign_commitment(&commitment.commitment),
        "saltHex": "07".repeat(32), "issuedAtUnixSecs": 100, "revoked": false,
    });
    let rule = serde_json::json!({
        "source": "urn:issuer", "issuerKeyHex": public_key_to_hex(&key.public_key()),
        "scopeIri": "https://pod.ex/", "freshWithinSecs": 100, "shapePredicateIri": "urn:claim",
    });
    let child = SecretKey::from_seed(42);
    let mut cert = sparq_trust::graph::Certification {
        certifier: NamedNode::new("urn:issuer").unwrap(),
        certifier_key: key.public_key(),
        certified_issuer: NamedNode::new("urn:child").unwrap(),
        certified_key: child.public_key(),
        scope: sparq_trust::graph::CertScope::AnyService,
        valid_from_unix_secs: 0,
        valid_until_unix_secs: 200,
        signature_hex: String::new(),
    };
    cert.signature_hex = key.sign_commitment(&sparq_trust::graph::certification_message(&cert));
    let certification = serde_json::json!({
        "certifierIri":"urn:issuer", "certifierKeyHex":public_key_to_hex(&key.public_key()),
        "certifiedIssuerIri":"urn:child", "certifiedKeyHex":public_key_to_hex(&child.public_key()),
        "scopeKind":"anyService", "validFromUnixSecs":0, "validUntilUnixSecs":200,
        "signatureHex":cert.signature_hex,
    });
    serde_json::json!({
        "dataset": concat!(
            "<https://pod.ex/n1> <urn:p> <urn:o> <https://pod.ex/n1> .\n",
            "<https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
            "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n",
            "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
            "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
        ),
        "resource": "https://pod.ex/n1", "mode": "read", "session": {"agent":"https://alice.ex/card#me"},
        "trust": {"agentIri":"https://alice.ex/card#me", "nowUnixSecs":100,
            "rules":vec![rule; rules], "credentials":vec![credential; credentials],
            "certifications":vec![certification; certs]},
    })
}

#[tokio::test]
async fn dense_and_resource_boundaries_preserve_default_admission_without_tcp() {
    for (name, body) in boundary_cases() {
        if name.contains("budget/")
            || name.contains("resource-bytes/")
            || name.contains("assembled-facts/")
            || name.starts_with("transport-boundary/")
            || name.contains("collection/")
        {
            continue;
        }
        assert!(
            body.to_string().len() <= ServerConfig::default().max_body_bytes,
            "{name}"
        );
        let config = ServerConfig {
            solid_authz: true,
            #[cfg(feature = "solid-authz-trust")]
            solid_authz_trust: body.get("trust").is_some(),
            ..Default::default()
        };
        let response = decide_endpoint(
            State(AppState::with_config(sparq_core::Graph::default(), config)),
            HeaderMap::new(),
            Bytes::from(body.to_string()),
        )
        .await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{name}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

fn dense_body(odrl: bool) -> String {
    let mut data = dense_dataset(40000, false, odrl);
    let mut encoded = base_request(data.clone()).to_string().len();
    let mut i = 40000;
    loop {
        let row = format!("<a:{i:x}> <a:> <a:> <a:>.\n");
        if encoded + row.len() + 1 + 32 > 1 << 20 {
            break;
        }
        encoded += row.len() + 1;
        data.push_str(&row);
        i += 1;
    }
    let prefix = "<a:fill> <a:> \"";
    let suffix = "\".\n";
    let left = (1 << 20) - encoded - prefix.len() - suffix.len() - 3;
    data.push_str(prefix);
    data.push_str(&"x".repeat(left));
    data.push_str(suffix);
    assert_eq!(base_request(data.clone()).to_string().len(), 1 << 20);
    data
}
fn resource_byte_boundary(extra: usize) -> String {
    let mut data = boundary_acl().to_owned();
    let mut names = std::collections::BTreeSet::from([
        "https://pod.ex/".to_owned(),
        "https://pod.ex/n1".to_owned(),
    ]);

    for i in 0..90 {
        let iri = format!("https://pod.ex/{i}{}/a/b/c/d/e/z", "x".repeat(1500));
        let mut cur = iri.as_str();
        names.insert(cur.to_owned());
        while let Some(pos) = cur[..cur.len().saturating_sub(1)].rfind('/') {
            if pos < "https://pod.ex".len() {
                break;
            }
            cur = &cur[..=pos];
            names.insert(cur.to_owned());
        }
        data.push_str(&format!("<a:> <a:> <a:> <{iri}>.\n"));
    }
    // The input also owns accepted control-document terms; generated facts use
    // their separate generated-lexical account.
    let control_terms = [
        [
            "https://pod.ex/.acl#r",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            "http://www.w3.org/ns/auth/acl#Authorization",
        ],
        [
            "https://pod.ex/.acl#r",
            "http://www.w3.org/ns/auth/acl#default",
            "https://pod.ex/",
        ],
        [
            "https://pod.ex/.acl#r",
            "http://www.w3.org/ns/auth/acl#agent",
            "https://alice.ex/card#me",
        ],
        [
            "https://pod.ex/.acl#r",
            "http://www.w3.org/ns/auth/acl#mode",
            "http://www.w3.org/ns/auth/acl#Read",
        ],
    ];
    let bytes = names.iter().map(String::len).sum::<usize>()
        + control_terms
            .iter()
            .flatten()
            .map(|term| term.len() + std::mem::size_of::<oxrdf::Term>())
            .sum::<usize>();
    // Add direct children with bounded individual names until the exact cap.
    let mut left = (1 << 20) - bytes;
    let mut i = 0;
    while left > 0 {
        let prefix = format!("https://pod.ex/fill{i}");
        let length = left.min(2000);
        assert!(length >= prefix.len());
        let name = format!("{prefix}{}", "y".repeat(length - prefix.len()));
        data.push_str(&format!("<a:> <a:> <a:> <{name}>.\n"));
        left -= length;
        i += 1;
    }
    if extra > 0 {
        data = data.replacen("fill0", "fill0y", 1);
    }
    data
}

#[cfg(feature = "odrl-authz")]
fn collection_boundary_dataset(additional_members: usize) -> String {
    const ALICE: &str = "https://alice.ex/card#me";
    const N1: &str = "https://pod.ex/notes/n1";
    const CONTENT: &str="<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n";
    let odrl = "http://www.w3.org/ns/odrl/2/";
    let acl = "http://www.w3.org/ns/auth/acl#";
    let collection = "https://pod.ex/collections/notes";
    let mut dataset = format!(
            "{CONTENT}\
             <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{acl}Authorization> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#owner> <{acl}default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#owner> <{acl}agent> <{ALICE}> <https://pod.ex/.acl> .\n\
             <https://pod.ex/.acl#owner> <{acl}mode> <{acl}Read> <https://pod.ex/.acl> .\n\
             <https://pod.ex/notes/n2#it> <https://ex.dev/ns#title> \"other\" <https://pod.ex/notes/n2> .\n\
             <{N1}> <{odrl}partOf> <{collection}> .\n\
             <urn:pol/p> <{odrl}prohibition> _:pr .\n\
             _:pr <{odrl}action> <{odrl}read> .\n\
             _:pr <{odrl}target> <{collection}> .\n\
             _:pr <{odrl}assignee> <{ALICE}> .\n"
        );
    for i in 0..additional_members {
        dataset.push_str(&format!(
            "<https://pod.ex/notes/m{i}> <{odrl}partOf> <{collection}> .\n"
        ));
    }
    dataset
}

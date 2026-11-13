//! JSON host adapter for the successful-result relation.
//!
//! Research grade and not externally audited. Witness outputs contain secrets.

use oxrdf::Term;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sparq_core::Graph;
use sparq_zk::{
    commit::commit_graph_content,
    field::{
        field_from_hash_bytes, field_from_hex_str_strict, field_to_be_bytes_32, field_to_hex, Fr,
    },
    sig::{self, SecretKey},
};
use sparq_zk_compose::{
    manifest::StatusListSnapshot,
    result::{
        self, CredentialCapacity, IntegerCapacityPolicy, ResultCredential, ResultOptions,
        ResultPolicy, ResultPresentation, WitnessSelection,
    },
    VerifierNonce,
};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

include!(concat!(env!("OUT_DIR"), "/members.rs"));

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestIssuer {
    seed_hex: String,
    public_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductionIssuer {
    secret_key_hex: String,
    public_key: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Issuer {
    Production(ProductionIssuer),
    Test(TestIssuer),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRef {
    status_list: String,
    version: u64,
    index: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestIssueInput {
    issuer: TestIssuer,
    graph_n_quads: String,
    status: StatusRef,
    seed_hex: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Credential {
    graph_n_quads: String,
    salt: String,
    issuer: String,
    signature: String,
    status_ref: StatusRef,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    trusted_issuers: Vec<String>,
    snapshots: Vec<StatusListSnapshot>,
    min_version: u64,
    max_version: u64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    #[serde(default)]
    hide_in_two: bool,
    #[serde(default)]
    hide_in_u64: bool,
    #[serde(default)]
    first_success: bool,
    max_search_steps: Option<usize>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn parse<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    serde_json::from_str(s).map_err(err)
}
fn nonce(s: &str) -> Result<VerifierNonce, String> {
    field_from_hex_str_strict(s)
        .map(VerifierNonce::from_field)
        .ok_or_else(|| "invalid nonce field".into())
}
fn test_seed(seed: &str) -> Result<u64, String> {
    let hex = seed.strip_prefix("0x").unwrap_or(seed);
    if hex.is_empty() || hex.len() > 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("test issuer seed must be a hexadecimal u64".into());
    }
    u64::from_str_radix(hex, 16).map_err(err)
}
fn test_issuer_key(seed: &str) -> Result<SecretKey, String> {
    Ok(SecretKey::from_seed(test_seed(seed)?))
}
fn policy(s: &str) -> Result<ResultPolicy, String> {
    let p: Policy = parse(s)?;
    let keys = p
        .trusted_issuers
        .iter()
        .map(|s| sig::public_key_from_hex(s).ok_or_else(|| "invalid trusted issuer key".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ResultPolicy {
        trusted_issuers: keys,
        snapshots: p.snapshots,
        min_version: p.min_version,
        max_version: p.max_version,
        admit_unbound_legacy: false,
    })
}
fn credential(s: &str) -> Result<ResultCredential, String> {
    let c: Credential = parse(s)?;
    let graph = Graph::load_str(&c.graph_n_quads, "nquads")?;
    let salt = field_from_hex_str_strict(&c.salt).ok_or("invalid salt")?;
    Ok(ResultCredential {
        graph: commit_graph_content(&graph, salt).map_err(err)?,
        issuer: sig::public_key_from_hex(&c.issuer).ok_or("invalid issuer key")?,
        signature: sig::signature_from_hex(&c.signature).ok_or("invalid issuer signature")?,
        status_list: c.status_ref.status_list,
        status_version: c.status_ref.version,
        status_index: c.status_ref.index,
    })
}
fn inputs(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks_exact(32)
        .map(|word| {
            let mut s = String::from("0x");
            for byte in word {
                s.push_str(&format!("{byte:02x}"));
            }
            s
        })
        .collect()
}
fn noir_numbers(value: Value) -> Value {
    match value {
        Value::Number(n) => Value::String(n.to_string()),
        Value::Array(v) => Value::Array(v.into_iter().map(noir_numbers).collect()),
        Value::Object(v) => {
            Value::Object(v.into_iter().map(|(k, v)| (k, noir_numbers(v))).collect())
        }
        v => v,
    }
}

/// Deterministic issuer convenience for tests only; never production key generation.
#[wasm_bindgen(js_name = testIssuerKeygen)]
pub fn test_issuer_keygen(seed_hex: &str) -> Result<String, String> {
    let sk = test_issuer_key(seed_hex)?;
    serde_json::to_string(&TestIssuer {
        seed_hex: seed_hex.into(),
        public_key: sig::public_key_to_hex(&sk.public_key()),
    })
    .map_err(err)
}

/// Generate a research-grade issuer key using the platform CSPRNG.
/// The returned secret is never persisted; the caller must store it securely.
#[wasm_bindgen(js_name = issuerKeygen)]
pub fn issuer_keygen() -> Result<String, String> {
    let sk = SecretKey::generate().map_err(err)?;
    serde_json::to_string(&ProductionIssuer {
        secret_key_hex: sk.to_hex(),
        public_key: sig::public_key_to_hex(&sk.public_key()),
    })
    .map_err(err)
}

/// Issues a string-canonical credential with a fresh platform-CSPRNG salt and method-bound attestation.
#[wasm_bindgen]
pub fn issue(
    issuer_json: &str,
    graph_n_quads: &str,
    status_ref_json: &str,
) -> Result<String, String> {
    let issuer: Issuer = parse(issuer_json)?;
    let (sk, pk) = issuer_key(issuer)?;
    let status: StatusRef = parse(status_ref_json)?;
    let graph = Graph::load_str(graph_n_quads, "nquads")?;
    let mut entropy = [0u8; 32];
    getrandom_latest::fill(&mut entropy).map_err(err)?;
    issue_credential(
        &sk,
        pk,
        graph_n_quads,
        &graph,
        status,
        field_from_hash_bytes(&entropy),
    )
}

fn issuer_key(issuer: Issuer) -> Result<(SecretKey, String), String> {
    let (sk, expected_pk, mismatch) = match issuer {
        Issuer::Production(issuer) => (
            SecretKey::from_hex(&issuer.secret_key_hex).ok_or("invalid issuer secret key")?,
            issuer.public_key,
            "issuer key differs from secret",
        ),
        Issuer::Test(issuer) => (
            test_issuer_key(&issuer.seed_hex)?,
            issuer.public_key,
            "issuer key differs from seed",
        ),
    };
    let pk = sig::public_key_to_hex(&sk.public_key());
    if pk != expected_pk {
        return Err(mismatch.into());
    }
    Ok((sk, pk))
}

// Exact UTF-8 graph bytes are length-prefixed; the key has fixed canonical width.
fn test_salt(seed: u64, pk: &str, graph_n_quads: &str) -> Fr {
    let mut hash = Sha256::new();
    hash.update(b"sparq.zk-result.test-issue.v1\0");
    hash.update(seed.to_be_bytes());
    hash.update(pk.as_bytes());
    hash.update((graph_n_quads.len() as u64).to_be_bytes());
    hash.update(graph_n_quads.as_bytes());
    field_from_hash_bytes(&hash.finalize().into())
}

/// Deterministic issuance for tests only. Production secret-key objects are refused.
#[wasm_bindgen(js_name = testIssue)]
pub fn test_issue(input_json: &str) -> Result<String, String> {
    let input: TestIssueInput = parse(input_json)?;
    let (sk, pk) = issuer_key(Issuer::Test(input.issuer))?;
    let salt = test_salt(test_seed(&input.seed_hex)?, &pk, &input.graph_n_quads);
    let graph = Graph::load_str(&input.graph_n_quads, "nquads")?;
    issue_credential(&sk, pk, &input.graph_n_quads, &graph, input.status, salt)
}

fn issue_credential(
    sk: &SecretKey,
    pk: String,
    graph_n_quads: &str,
    graph: &Graph,
    status: StatusRef,
    salt: Fr,
) -> Result<String, String> {
    let c = commit_graph_content(graph, salt).map_err(err)?;
    let status_digest = sig::status_ref_digest(
        &sig::status_list_id_to_field(&status.status_list),
        status.index,
        status.version,
    );
    let signature = sk.attest(&c.method_commitment(), &salt, &status_digest, None);
    serde_json::to_string(&Credential {
        graph_n_quads: graph_n_quads.into(),
        salt: field_to_hex(&salt),
        issuer: pk,
        signature,
        status_ref: status,
    })
    .map_err(err)
}

/// Prepares private ABI inputs and the public skeleton using the shared result relation.
#[wasm_bindgen]
pub fn prepare(
    query_text: &str,
    credentials_json: Vec<String>,
    policy_json: &str,
    nonce_hex: &str,
    options_json: &str,
) -> Result<String, String> {
    if credentials_json.len() > sparq_zk_compose::planner::MAX_DISCLOSURE_CREDENTIALS {
        return Err("wallet exceeds disclosure credential limit".into());
    }
    sparq_zk_compose::planner::DisclosureQuery::parse(query_text).map_err(err)?;
    let credentials = credentials_json
        .iter()
        .map(|s| credential(s))
        .collect::<Result<Vec<_>, _>>()?;
    let p = policy(policy_json)?;
    let n = nonce(nonce_hex)?;
    let options: Options = parse(options_json)?;
    let triples = credentials
        .iter()
        .flat_map(|c| c.graph.canonical.triples.iter().cloned())
        .collect::<Vec<_>>();
    let mut graph = Graph::new();
    for t in triples {
        graph.insert_triple(t.subject, t.predicate, t.object)?;
    }
    let evaluated = sparq_engine::query(&graph, query_text)?;
    let rows = evaluated
        .rows
        .iter()
        .map(|row| {
            evaluated
                .vars
                .iter()
                .zip(row)
                .filter_map(|(var, term)| {
                    term.as_ref().map(|t| (var.as_str().to_owned(), t.clone()))
                })
                .collect::<BTreeMap<String, Term>>()
        })
        .collect::<Vec<_>>();
    let defaults = ResultOptions::default();
    let prepared = result::prepare_result_with_options(
        query_text,
        &credentials,
        &rows,
        &p,
        &n,
        ResultOptions {
            credential_capacity: if options.hide_in_two {
                CredentialCapacity::HideInTwo
            } else {
                defaults.credential_capacity
            },
            integer_capacity: if options.hide_in_u64 {
                IntegerCapacityPolicy::HideInU64
            } else {
                defaults.integer_capacity
            },
            witness_selection: if options.first_success {
                WitnessSelection::FirstSuccess
            } else {
                defaults.witness_selection
            },
            max_search_steps: options
                .max_search_steps
                .unwrap_or(defaults.max_search_steps),
        },
    )
    .map_err(err)?;
    canonical_vk_hash(prepared.member())?;
    Ok(json!({"member":prepared.member(),"witnessInputs":noir_numbers(serde_json::to_value(prepared.witness_inputs()).map_err(err)?),"presentationSkeleton":prepared.presentation_skeleton()}).to_string())
}

/// Independently reconstructs the expected statement and candidate report.
/// This performs no proof check or nonce consumption and grants no acceptance.
#[wasm_bindgen(js_name = reconstructPublicInputs)]
pub fn reconstruct_public_inputs(
    expected_query_text: &str,
    presentation_json: &str,
    policy_json: &str,
    nonce_hex: &str,
) -> Result<String, String> {
    let presentation: ResultPresentation = parse(presentation_json)?;
    let p = policy(policy_json)?;
    let n = nonce(nonce_hex)?;
    let reconstructed =
        result::reconstruct_result_public_inputs(expected_query_text, &presentation, &p, &n)
            .map_err(err)?;
    let report = reconstructed_report(expected_query_text, &reconstructed, &p, &n)?;
    Ok(json!({"member":reconstructed.member,"publicInputs":inputs(&reconstructed.public_inputs),"released":presentation.rows,"nonce":n.as_field_hex().0,"report":report}).to_string())
}

// Candidate evidence only: the Node verifier publishes this after proof acceptance
// and nonce consumption. Resolve keys from the actual rebuilt public coordinates,
// never by copying presentation metadata or returning the entire trust list.
fn reconstructed_report(
    expected_query: &str,
    reconstructed: &result::ReconstructedResult,
    policy: &ResultPolicy,
    nonce: &VerifierNonce,
) -> Result<Value, String> {
    let slots = match reconstructed.member.split('_').nth(2) {
        Some("k1") => 1,
        Some("k2") => 2,
        _ => return Err("unsupported issuer report capacity".into()),
    };
    let coordinates = reconstructed
        .public_inputs
        .get(3 * 32..(3 + 2 * slots) * 32)
        .ok_or("missing public issuer coordinates")?;
    let issuers = coordinates
        .chunks_exact(64)
        .map(|pair| {
            policy
                .trusted_issuers
                .iter()
                .find_map(|key| {
                    let (x, y) = key.coords()?;
                    (pair[..32] == field_to_be_bytes_32(&x)
                        && pair[32..] == field_to_be_bytes_32(&y))
                    .then(|| sig::public_key_to_hex(key))
                })
                .ok_or_else(|| "public issuer coordinates outside trust policy".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "rule": {"query": expected_query, "member": reconstructed.member},
        "issuers": issuers,
        "subject": [],
        "fresh": nonce.as_field_hex().0,
        "complete": false,
    }))
}

/// SHA-256 of the canonical member key embedded at build time from the shared registry.
#[wasm_bindgen(js_name = canonicalVkHash)]
pub fn canonical_vk_hash(member: &str) -> Result<String, String> {
    let (_, bytes) = MEMBERS
        .iter()
        .find(|(name, _)| *name == member)
        .ok_or("unsupported canonical member")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str =
        "<https://example.org/alice> <https://example.org/degree> <https://example.org/BSc> .";
    const QUERY: &str =
        "SELECT DISTINCT ?p WHERE { ?p <https://example.org/degree> <https://example.org/BSc> }";
    const STATUS: &str = r#"{"status_list":"https://example.org/status","version":1,"index":0}"#;

    fn fixture() -> (String, String, String) {
        let issuer = test_issuer_keygen("a1").unwrap();
        let key: Value = parse(&issuer).unwrap();
        let policy=json!({"trustedIssuers":[key["publicKey"]],"snapshots":[{"status_list":"https://example.org/status","version":1,"bits":[0]}],"minVersion":1,"maxVersion":1}).to_string();
        let credential = issue(&issuer, GRAPH, STATUS).unwrap();
        (issuer, credential, policy)
    }

    #[test]
    fn test_salt_has_a_stable_domain_separated_derivation() {
        assert_eq!(
            field_to_hex(&test_salt(42, &"0".repeat(64), "abc")),
            "0x004c0958f624df17b2abae7a4d54a6d5a7e2bc4afeb625ee5e5a7ada11530597"
        );
        let (issuer, _, _) = fixture();
        let mut input = json!({"issuer": parse::<Value>(&issuer).unwrap(), "graphNQuads": GRAPH,
            "status": parse::<Value>(STATUS).unwrap(), "seedHex": "2a"});
        let a = test_issue(&input.to_string()).unwrap();
        assert_eq!(a, test_issue(&input.to_string()).unwrap());
        input["seedHex"] = json!("0x002A");
        assert_eq!(a, test_issue(&input.to_string()).unwrap());
        let salt = parse::<Value>(&a).unwrap()["salt"].clone();
        for (field, value) in [
            ("seedHex", json!("2b")),
            ("graphNQuads", json!(GRAPH.replace("alice", "bob"))),
            (
                "issuer",
                parse::<Value>(&test_issuer_keygen("b2").unwrap()).unwrap(),
            ),
        ] {
            let mut changed = input.clone();
            changed[field] = value;
            assert_ne!(
                salt,
                parse::<Value>(&test_issue(&changed.to_string()).unwrap()).unwrap()["salt"]
            );
        }
        input["status"]["index"] = json!(1);
        let changed: Value = parse(&test_issue(&input.to_string()).unwrap()).unwrap();
        assert_eq!(salt, changed["salt"]);
        assert_ne!(
            parse::<Value>(&a).unwrap()["signature"],
            changed["signature"]
        );
        let issued = credential(&a).unwrap();
        let reference = sig::status_ref_digest(
            &sig::status_list_id_to_field(&issued.status_list),
            issued.status_index,
            issued.status_version,
        );
        assert!(sig::verify(
            &issued.issuer,
            &sig::presentation_message_with_status_and_scheme(
                &issued.graph.commitment,
                &issued.graph.salt,
                &reference,
                &sig::scheme_iri_to_field(issued.graph.method_commitment().method.scheme_iri()),
            ),
            &issued.signature
        ));
    }

    #[test]
    fn deterministic_issuance_refuses_production_and_mixed_keys() {
        let production: Value = parse(&issuer_keygen().unwrap()).unwrap();
        let mut input = json!({"issuer": production, "graphNQuads": GRAPH,
            "status": parse::<Value>(STATUS).unwrap(), "seedHex": "2a"});
        assert!(test_issue(&input.to_string()).is_err());
        input["issuer"]["seedHex"] = json!("a1");
        assert!(test_issue(&input.to_string()).is_err());
        input["issuer"] = parse(&test_issuer_keygen("a1").unwrap()).unwrap();
        for bad in ["", "xyz", "10000000000000000"] {
            input["seedHex"] = json!(bad);
            assert!(test_issue(&input.to_string()).is_err());
        }
        input["seedHex"] = json!("2a");
        input["issuer"]["publicKey"] = production["publicKey"].clone();
        assert_eq!(
            test_issue(&input.to_string()).unwrap_err(),
            "issuer key differs from seed"
        );
    }

    #[test]
    fn issuer_report_follows_reconstructed_slots_including_repeats() {
        let (_, c, p) = fixture();
        let mut p = policy(&p).unwrap();
        let other = test_issuer_key("b2").unwrap().public_key();
        p.trusted_issuers.insert(0, other);
        let n = nonce("0x1234").unwrap();
        let c = credential(&c).unwrap();
        let rows = vec![BTreeMap::from([(
            "p".into(),
            Term::NamedNode(oxrdf::NamedNode::new("https://example.org/alice").unwrap()),
        )])];
        for capacity in [CredentialCapacity::Smallest, CredentialCapacity::HideInTwo] {
            let prepared = result::prepare_result_with_options(
                QUERY,
                std::slice::from_ref(&c),
                &rows,
                &p,
                &n,
                ResultOptions {
                    credential_capacity: capacity,
                    ..ResultOptions::default()
                },
            )
            .unwrap();
            let mut reconstructed = result::reconstruct_result_public_inputs(
                QUERY,
                prepared.presentation_skeleton(),
                &p,
                &n,
            )
            .unwrap();
            let slots = if capacity == CredentialCapacity::Smallest {
                1
            } else {
                2
            };
            let report = reconstructed_report(QUERY, &reconstructed, &p, &n).unwrap();
            assert_eq!(
                report,
                json!({
                    "rule": {"query": QUERY, "member": prepared.member()},
                    "issuers": vec![sig::public_key_to_hex(&c.issuer); slots],
                    "subject": [], "fresh": n.as_field_hex().0, "complete": false,
                })
            );
            // The report follows public bytes even when another trusted key occupies
            // the slot; this unit check grants no proof acceptance.
            let (x, y) = other.coords().unwrap();
            reconstructed.public_inputs[3 * 32..4 * 32].copy_from_slice(&field_to_be_bytes_32(&x));
            reconstructed.public_inputs[4 * 32..5 * 32].copy_from_slice(&field_to_be_bytes_32(&y));
            let report = reconstructed_report(QUERY, &reconstructed, &p, &n).unwrap();
            assert_eq!(report["issuers"][0], sig::public_key_to_hex(&other));
            p.trusted_issuers.remove(0);
            assert!(reconstructed_report(QUERY, &reconstructed, &p, &n).is_err());
            p.trusted_issuers.insert(0, other);
        }
    }

    #[test]
    fn production_issuance_imports_secret_and_rejects_mixed_or_mismatched_keys() {
        let encoded = issuer_keygen().unwrap();
        let issuer: Value = parse(&encoded).unwrap();
        assert_eq!(issuer.as_object().unwrap().len(), 2);
        assert!(issuer.get("seedHex").is_none());
        let sk = SecretKey::from_hex(issuer["secretKeyHex"].as_str().unwrap()).unwrap();
        assert_eq!(
            issuer["publicKey"],
            sig::public_key_to_hex(&sk.public_key())
        );
        let issued = credential(&issue(&encoded, GRAPH, STATUS).unwrap()).unwrap();
        let status = sig::status_ref_digest(
            &sig::status_list_id_to_field(&issued.status_list),
            issued.status_index,
            issued.status_version,
        );
        let message = sig::presentation_message_with_status_and_scheme(
            &issued.graph.commitment,
            &issued.graph.salt,
            &status,
            &sig::scheme_iri_to_field(issued.graph.method_commitment().method.scheme_iri()),
        );
        assert!(sig::verify(&sk.public_key(), &message, &issued.signature));

        let mut bad = issuer.clone();
        bad["publicKey"] = json!(sig::public_key_to_hex(
            &SecretKey::from_seed(99).public_key()
        ));
        assert_eq!(
            issue(&bad.to_string(), GRAPH, STATUS).unwrap_err(),
            "issuer key differs from secret"
        );
        for secret in ["00".repeat(32), "ff".repeat(32), "invalid".into()] {
            bad = issuer.clone();
            bad["secretKeyHex"] = json!(secret);
            assert_eq!(
                issue(&bad.to_string(), GRAPH, STATUS).unwrap_err(),
                "invalid issuer secret key"
            );
        }
        bad = issuer;
        bad["seedHex"] = json!("a1");
        assert!(issue(&bad.to_string(), GRAPH, STATUS).is_err());
        let legacy = test_issuer_keygen("a1").unwrap();
        let test: Value = parse(&legacy).unwrap();
        assert!(test.get("secretKeyHex").is_none());
        assert_eq!(test["seedHex"], "a1");
        assert!(issue(&legacy, GRAPH, STATUS).is_ok());
    }

    #[test]
    fn method_bound_issuance_and_shared_public_reconstruction() {
        let (_, c, p) = fixture();
        let prepared: Value =
            parse(&prepare(QUERY, vec![c.clone()], &p, "0x1234", "{}").unwrap()).unwrap();
        assert_eq!(prepared["member"], "result_v5_k1_n16_p3_r4_f0_d10");
        assert_eq!(prepared["presentationSkeleton"]["version"], 5);
        let reconstructed: Value = parse(
            &reconstruct_public_inputs(
                QUERY,
                &prepared["presentationSkeleton"].to_string(),
                &p,
                "0x1234",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepared
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["member", "witnessInputs", "presentationSkeleton"])
        );
        let inputs = prepared["witnessInputs"].as_object().unwrap();
        let acir: Value = serde_json::from_str(include_str!(
            "../../../js/zk-result/assets/result_v5_k1_n16_p3_r4_f0_d10.json"
        ))
        .unwrap();
        let abi = acir["abi"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            inputs
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            abi
        );
        let decoded = credential(&c).unwrap();
        let skeleton: ResultPresentation =
            parse(&prepared["presentationSkeleton"].to_string()).unwrap();
        let rows = skeleton
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(name, term)| (name.clone(), term.to_term().unwrap()))
                    .collect::<BTreeMap<_, _>>()
            })
            .collect::<Vec<_>>();
        let native = result::prepare_result(
            QUERY,
            std::slice::from_ref(&decoded),
            &rows,
            &policy(&p).unwrap(),
            &nonce("0x1234").unwrap(),
        )
        .unwrap();
        assert_eq!(
            reconstructed["publicInputs"],
            json!(super::inputs(native.public_inputs()))
        );
        let status = sig::status_ref_digest(
            &sig::status_list_id_to_field(&decoded.status_list),
            decoded.status_index,
            decoded.status_version,
        );
        let message = sig::presentation_message_with_status_and_scheme(
            &decoded.graph.commitment,
            &decoded.graph.salt,
            &status,
            &sig::scheme_iri_to_field(decoded.graph.method_commitment().method.scheme_iri()),
        );
        assert!(sig::verify(&decoded.issuer, &message, &decoded.signature));
        assert!(!sig::verify(
            &decoded.issuer,
            &sig::commitment_message_with_status(
                &decoded.graph.commitment,
                &decoded.graph.salt,
                &status
            ),
            &decoded.signature
        ));
        assert!(!prepared["presentationSkeleton"]
            .to_string()
            .contains(&field_to_hex(&decoded.graph.salt)));
        assert!(reconstruct_public_inputs(
            "SELECT DISTINCT ?x WHERE {?x <https://example.org/degree> <https://example.org/BSc>}",
            &prepared["presentationSkeleton"].to_string(),
            &p,
            "0x1234"
        )
        .unwrap_err()
        .contains("query differs"));
    }

    #[test]
    fn issuance_uses_fresh_platform_entropy() {
        let (issuer, c, _) = fixture();
        let a: Value = parse(&c).unwrap();
        let b: Value = parse(&issue(&issuer, GRAPH, STATUS).unwrap()).unwrap();
        assert_ne!(a["salt"], b["salt"]);
        assert_ne!(a["signature"], b["signature"]);
    }

    #[test]
    fn rejects_unadmitted_queries_before_evaluation() {
        let (_, c, p) = fixture();
        let error = prepare(
            "SELECT ?x WHERE { SERVICE <urn:unsupported> {?x ?p ?o} }",
            vec!["invalid credential JSON".into()],
            &p,
            "0x1",
            "{}",
        )
        .unwrap_err();
        assert!(error.contains("SELECT must use DISTINCT"));
        assert!(prepare(
            "SELECT ?x WHERE { SERVICE <urn:unsupported> {?x ?p ?o} }",
            vec![c],
            &p,
            "0x1",
            "{}"
        )
        .unwrap_err()
        .contains("SELECT must use DISTINCT"));
    }
}

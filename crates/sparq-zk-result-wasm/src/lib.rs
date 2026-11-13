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
    field::{field_from_hash_bytes, field_from_hex_str_strict, field_to_hex},
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
#[serde(deny_unknown_fields)]
struct StatusRef {
    status_list: String,
    version: u64,
    index: u64,
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
fn issuer_key(seed: &str) -> Result<SecretKey, String> {
    let hex = seed.strip_prefix("0x").unwrap_or(seed);
    if hex.is_empty() || hex.len() > 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("test issuer seed must be a hexadecimal u64".into());
    }
    Ok(SecretKey::from_seed(
        u64::from_str_radix(hex, 16).map_err(err)?,
    ))
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
    let sk = issuer_key(seed_hex)?;
    serde_json::to_string(&TestIssuer {
        seed_hex: seed_hex.into(),
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
    let issuer: TestIssuer = parse(issuer_json)?;
    let sk = issuer_key(&issuer.seed_hex)?;
    let pk = sig::public_key_to_hex(&sk.public_key());
    if pk != issuer.public_key {
        return Err("issuer key differs from seed".into());
    }
    let status: StatusRef = parse(status_ref_json)?;
    let graph = Graph::load_str(graph_n_quads, "nquads")?;
    let mut entropy = [0u8; 32];
    getrandom_latest::fill(&mut entropy).map_err(err)?;
    let salt = field_from_hash_bytes(&entropy);
    let c = commit_graph_content(&graph, salt).map_err(err)?;
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

/// Independently reconstructs the expected statement; it performs no proof check.
#[wasm_bindgen(js_name = reconstructPublicInputs)]
pub fn reconstruct_public_inputs(
    expected_query_text: &str,
    presentation_json: &str,
    policy_json: &str,
    nonce_hex: &str,
) -> Result<String, String> {
    let presentation: ResultPresentation = parse(presentation_json)?;
    let reconstructed = result::reconstruct_result_public_inputs(
        expected_query_text,
        &presentation,
        &policy(policy_json)?,
        &nonce(nonce_hex)?,
    )
    .map_err(err)?;
    Ok(json!({"member":reconstructed.member,"publicInputs":inputs(&reconstructed.public_inputs),"released":presentation.rows,"nonce":nonce(nonce_hex)?.as_field_hex().0}).to_string())
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

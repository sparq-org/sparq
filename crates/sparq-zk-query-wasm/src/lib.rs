//! JSON host logic for the bounded authenticated query method.
//!
//! Research grade and not externally audited. Witness outputs contain secrets.

mod codec;
mod fixtures;
mod issuance;
mod presentation;
mod prover;
mod request;
mod verification;

use codec::{err, json, Result};
use serde_json::{json as value, Value};
use sparq_query_protocol::{Challenge32, ChallengeOutcome, ChallengeStore, ChallengeStoreError};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};
use wasm_bindgen::prelude::*;

const CATALOGUE: &str = include_str!("../../../js/zk-query/assets/catalogue.json");
const TOOLCHAIN: &str = include_str!("../../../js/zk-query/assets/toolchain.pins.json");

/// Digest of the catalogue compiled into this host, independent of runtime files.
#[wasm_bindgen(js_name=catalogueHash)]
pub fn catalogue_hash() -> String {
    codec::digest(CATALOGUE.as_bytes())
}
/// Digest of the toolchain pin data compiled into this host.
#[wasm_bindgen(js_name=toolchainPinsHash)]
pub fn toolchain_pins_hash() -> String {
    codec::digest(TOOLCHAIN.as_bytes())
}

/// Portable JSON operations used by the ESM facade and native reference tests.
#[wasm_bindgen(js_name=hostCall)]
pub fn host_call(operation: &str, input: &str) -> std::result::Result<String, String> {
    let output = match operation {
        "issuerKeygen" => serde_json::to_value(issuance::keygen()?).map_err(err)?,
        "testIssuerKeygen" => {
            let seed: String = codec::parse(input)?;
            serde_json::to_value(issuance::test_keygen(&seed)?).map_err(err)?
        }
        "issue" => serde_json::to_value(issuance::issue(input, false)?).map_err(err)?,
        "testIssue" => serde_json::to_value(issuance::issue(input, true)?).map_err(err)?,
        "plan" => request::plan_info(input)?,
        "testOperatorFixture" => fixtures::operator(input)?,
        "layout" => {
            let input: Value = codec::parse(input)?;
            let policy: request::Policy =
                serde_json::from_value(input["policy"].clone()).map_err(err)?;
            let plan = policy.plan(input["query"].as_str().ok_or("layout query required")?)?;
            let layout = sparq_zk_query::statement::Layout::new(&plan);
            layout
                .audit_authenticated_artifact(&input["artifact"])
                .map_err(err)?;
            value!({"declarations":layout.declarations_authenticated()})
        }
        "request" => serde_json::to_value(request::create(input)?).map_err(err)?,
        "holder" => {
            let input: Value = codec::parse(input)?;
            let wire: request::RequestWire =
                serde_json::from_value(input["request"].clone()).map_err(err)?;
            let r = request::restore(&wire)?;
            let p = r.plan().map_err(err)?;
            request::checked_key(&p, r.key())?;
            request::freshness(&r, input["clock"].as_u64().ok_or("clock rejected")?)?;
            value!({"planHash":codec::hex(&p.hash())})
        }
        "prepareProof" => prover::prepare(input)?,
        "bindProof" => prover::bind(input)?,
        "statement" => {
            let input: verification::Input = codec::parse(input)?;
            verification::statement(&input.request, &input.presentation)?
        }
        "auditArtifact" => {
            let input: Value = codec::parse(input)?;
            let wire: request::RequestWire =
                serde_json::from_value(input["request"].clone()).map_err(err)?;
            let request = request::restore(&wire)?;
            sparq_zk_query::statement::Layout::new(&request.plan().map_err(err)?)
                .audit_authenticated_artifact(&input["artifact"])
                .map_err(err)?;
            value!(true)
        }
        _ => return Err("unknown host operation".into()),
    };
    json(&output)
}

#[wasm_bindgen]
extern "C" {
    /// Synchronous host callback; arguments and storage outcomes use JSON strings.
    #[derive(Clone)]
    pub type Callback;
    #[wasm_bindgen(method,catch,js_name=call)]
    fn invoke(
        this: &Callback,
        context: &JsValue,
        input: &str,
    ) -> std::result::Result<JsValue, JsValue>;
}
thread_local! {
    static CALLBACKS:RefCell<BTreeMap<u64,Callback>>=const{RefCell::new(BTreeMap::new())};
    static NEXT_CALLBACK:Cell<u64>=const{Cell::new(0)};
}
struct RegisteredStore(u64);
impl RegisteredStore {
    fn register(callback: &Callback) -> Result<Self> {
        let id = NEXT_CALLBACK.with(|next| {
            let id = next.get().checked_add(1).ok_or("callback capacity")?;
            next.set(id);
            Ok::<_, String>(id)
        })?;
        CALLBACKS.with(|map| map.borrow_mut().insert(id, callback.clone()));
        Ok(Self(id))
    }
}
impl Drop for RegisteredStore {
    fn drop(&mut self) {
        CALLBACKS.with(|map| map.borrow_mut().remove(&self.0));
    }
}
impl ChallengeStore for RegisteredStore {
    fn consume(
        &self,
        challenge: &Challenge32,
    ) -> std::result::Result<ChallengeOutcome, ChallengeStoreError> {
        let callback = CALLBACKS
            .with(|map| map.borrow().get(&self.0).cloned())
            .ok_or(ChallengeStoreError::new("callback_missing"))?;
        let value = callback
            .invoke(&JsValue::NULL, &codec::hex(challenge.as_bytes()))
            .map_err(|_| ChallengeStoreError::new("callback_failed"))?;
        match value.as_string().as_deref() {
            Some("fresh") => Ok(ChallengeOutcome::Fresh),
            Some("alreadyConsumed") => Ok(ChallengeOutcome::AlreadyConsumed),
            _ => Err(ChallengeStoreError::new("invalid_outcome")),
        }
    }
}

/// Verify with externally supplied synchronous proof and atomic challenge calls.
/// Rust derives the statement, checks its key and window, and consumes last.
#[wasm_bindgen(js_name=verifyWith)]
pub fn verify_with(input: &str, proof: &Callback, store: &Callback, clock: &Callback) -> String {
    let result = (|| {
        let input: verification::Input = codec::parse(input)?;
        let store = RegisteredStore::register(store)?;
        verification::verify(
            input,
            |statement| {
                proof
                    .invoke(&JsValue::NULL, &json(statement)?)
                    .map_err(|_| "proof backend failed".to_string())?
                    .as_bool()
                    .ok_or_else(|| "proof backend outcome rejected".into())
            },
            &store,
            || {
                let value = clock
                    .invoke(&JsValue::NULL, "")
                    .map_err(|_| "clock callback failed".to_string())?;
                let now = value.as_f64().ok_or("clock outcome rejected")?;
                if !now.is_finite()
                    || now < 0.0
                    || now.fract() != 0.0
                    || now > 9_007_199_254_740_991.0
                {
                    return Err("clock outcome rejected".into());
                }
                Ok(now as u64)
            },
        )
    })();
    match result {
        Ok(v) => v.to_string(),
        Err(e) => presentation::refusal(e).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_query_protocol::Challenge32;
    use std::sync::Mutex;
    struct Store(Mutex<Vec<String>>);
    impl ChallengeStore for Store {
        fn consume(
            &self,
            nonce: &Challenge32,
        ) -> std::result::Result<ChallengeOutcome, ChallengeStoreError> {
            self.0.lock().unwrap().push(codec::hex(nonce.as_bytes()));
            Ok(ChallengeOutcome::Fresh)
        }
    }
    fn input() -> verification::Input {
        let mut v: Value = serde_json::from_str(include_str!(
            "../../../js/zk-query/test/fixtures/authenticated.json"
        ))
        .unwrap();
        v.as_object_mut()
            .unwrap()
            .retain(|k, _| k == "request" || k == "presentation");
        serde_json::from_value(v).unwrap()
    }
    #[test]
    fn verifier_consumes_only_after_proof_and_final_clock_checks() {
        let store = Store(Mutex::new(vec![]));
        let calls = Cell::new(0);
        let result = verification::verify(
            input(),
            |_| {
                assert!(store.0.lock().unwrap().is_empty());
                Ok(true)
            },
            &store,
            || {
                calls.set(calls.get() + 1);
                assert!(store.0.lock().unwrap().is_empty());
                Ok(1)
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(result["verified"], true);
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![result["report"]["fresh"].as_str().unwrap().to_owned()]
        );
    }
    #[test]
    fn invalid_proof_and_late_expiry_cannot_consume() {
        let store = Store(Mutex::new(vec![]));
        assert!(
            verification::verify(input(), |_| Ok(false), &store, || Ok(1))
                .unwrap_err()
                .contains("proof rejected")
        );
        let calls = Cell::new(0);
        assert!(verification::verify(
            input(),
            |_| Ok(true),
            &store,
            || {
                calls.set(calls.get() + 1);
                Ok(if calls.get() == 1 { 1 } else { u64::MAX })
            }
        )
        .unwrap_err()
        .contains("validity"));
        assert!(store.0.lock().unwrap().is_empty());
    }
    #[test]
    fn native_issuance_matches_the_independent_suite_vector() {
        let v: Value = serde_json::from_str(include_str!(
            "../../../js/zk-query/test/fixtures/issuance.json"
        ))
        .unwrap();
        let key = issuance::test_keygen(v["issuerSeedHex"].as_str().unwrap()).unwrap();
        let result=issuance::issue(&value!({"issuer":key,"graphNQuads":v["graphNQuads"],"status":{"proofConfigNQuads":v["proofConfigNQuads"]},"seedHex":v["seedHex"]}).to_string(),true).unwrap();
        let mut expected = v["expected"].clone();
        expected["issuer"] = expected
            .as_object_mut()
            .unwrap()
            .remove("publicKey")
            .unwrap();
        assert_eq!(serde_json::to_value(result).unwrap(), expected);
    }
    #[test]
    fn checked_key_refuses_unknown_plan_and_counterfeit_key() {
        let fixture = input();
        let r = request::restore(&fixture.request).unwrap();
        let p = r.plan().unwrap();
        assert!(request::checked_key(&p, r.key()).is_ok());
        assert!(request::checked_key(&p, &[0])
            .unwrap_err()
            .contains("key mismatch"));
        let other = sparq_zk_query::lower::lower(
            "SELECT ?s WHERE {?s <urn:uncatalogued> ?o}",
            fixture.request.policy.bounds.bounds(),
        )
        .unwrap();
        assert!(request::checked_key(&other, r.key())
            .unwrap_err()
            .contains("missing catalogue plan"));
    }
    #[test]
    fn wrong_stored_request_is_refused_before_the_proof_callback() {
        let mut fixture = input();
        let mut request = serde_json::to_value(&fixture.request).unwrap();
        request["nonce"] = value!("08".repeat(32));
        request["clock"] = value!(1);
        let fields = request.as_object_mut().unwrap();
        fields.remove("digest");
        fields.remove("planHash");
        fields.remove("requestRdf");
        fixture.request = request::create(&request.to_string()).unwrap();
        let store = Store(Mutex::new(vec![]));
        assert!(verification::verify(
            fixture,
            |_| panic!("wrong request reached proof callback"),
            &store,
            || Ok(1)
        )
        .unwrap_err()
        .contains("request digest"));
        assert!(store.0.lock().unwrap().is_empty());
    }
}

use crate::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;

fn parse<T: DeserializeOwned>(text: &str) -> std::result::Result<T, JsValue> {
    serde_json::from_str(text).map_err(|_| js_error(crypto::error(None, "json-encoding")))
}
fn js_error(e: Error) -> JsValue {
    JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| "protocol-error".into()))
}
fn encode<T: Serialize>(value: Result<T>) -> std::result::Result<String, JsValue> {
    serde_json::to_string(&value.map_err(js_error)?)
        .map_err(|_| js_error(crypto::error(None, "json-encoding")))
}

#[wasm_bindgen(js_name = newSession)]
pub fn new_session(json: &str) -> std::result::Result<String, JsValue> {
    encode(Session::new(parse(json)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContributeRequest {
    session: Session,
    index: u32,
    keys: SecretKeys,
    input: Input,
}
#[wasm_bindgen(js_name = contribute)]
pub fn wasm_contribute(json: &str) -> std::result::Result<String, JsValue> {
    let r: ContributeRequest = parse(json)?;
    encode(contribute(&r.session, r.index, &r.keys, &r.input))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiveRequest {
    session: Session,
    index: u32,
    keys: SecretKeys,
    contributions: Vec<Contribution>,
    local: LocalShare,
}
#[wasm_bindgen(js_name = receiveAndPartial)]
pub fn wasm_receive(json: &str) -> std::result::Result<String, JsValue> {
    let r: ReceiveRequest = parse(json)?;
    encode(receive_and_partial(
        &r.session,
        r.index,
        &r.keys,
        &r.contributions,
        &r.local,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishRequest {
    session: Session,
    index: u32,
    keys: SecretKeys,
    contributions: Vec<Contribution>,
    local: Option<Partial>,
    messages: Vec<PartialMessage>,
}
#[wasm_bindgen(js_name = finish)]
pub fn wasm_finish(json: &str) -> std::result::Result<String, JsValue> {
    let r: FinishRequest = parse(json)?;
    encode(finish(
        &r.session,
        r.index,
        &r.keys,
        &r.contributions,
        r.local.as_ref(),
        &r.messages,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyRequest {
    session: Session,
    contributions: Vec<Contribution>,
    outcome: SignedOutcome,
}
#[wasm_bindgen(js_name = verifyOutcome)]
pub fn wasm_verify(json: &str) -> std::result::Result<String, JsValue> {
    let r: VerifyRequest = parse(json)?;
    encode(
        verify_outcome(&r.session, &r.contributions, &r.outcome)
            .map(|()| serde_json::json!({"verified": true})),
    )
}
#[wasm_bindgen(js_name = generateKeys)]
pub fn wasm_keys(seed: Option<String>) -> std::result::Result<String, JsValue> {
    let seed = seed
        .map(|s| {
            let bytes: Bytes = parse(
                &serde_json::to_string(&s).map_err(|_| js_error(crypto::error(None, "seed")))?,
            )?;
            crypto::fixed(&bytes, None, "seed").map_err(js_error)
        })
        .transpose()?;
    encode(Ok(generate_keys(seed)))
}

/// Single-process simulation with private transcript; each array entry is a party-input JSON string.
#[wasm_bindgen(js_name = simulateSession)]
pub fn wasm_simulate(
    params_json: &str,
    party_inputs_json: Vec<String>,
) -> std::result::Result<String, JsValue> {
    let parties = party_inputs_json
        .iter()
        .map(|p| parse(p))
        .collect::<std::result::Result<Vec<PartyInput>, _>>()?;
    encode(simulate_session(parse(params_json)?, &parties))
}

//! JSON host logic for the bounded authenticated query method.
//!
//! Research grade and not externally audited. Witness outputs contain secrets.

mod codec;
mod issuance;
mod presentation;
mod prover;
mod request;
mod verification;

use codec::{err,json,Result};
use serde_json::{json as value,Value};
use sparq_query_protocol::{Challenge32,ChallengeOutcome,ChallengeStore,ChallengeStoreError};
use std::{cell::{Cell,RefCell},collections::BTreeMap};
use wasm_bindgen::prelude::*;

const CATALOGUE:&str=include_str!("../../../js/zk-query/assets/catalogue.json");
const TOOLCHAIN:&str=include_str!("../../../js/zk-query/assets/toolchain.pins.json");

/// Digest of the catalogue compiled into this host, independent of runtime files.
#[wasm_bindgen(js_name=catalogueHash)]
pub fn catalogue_hash()->String{codec::digest(CATALOGUE.as_bytes())}
/// Digest of the toolchain pin data compiled into this host.
#[wasm_bindgen(js_name=toolchainPinsHash)]
pub fn toolchain_pins_hash()->String{codec::digest(TOOLCHAIN.as_bytes())}

/// Portable JSON operations used by the ESM facade and native reference tests.
#[wasm_bindgen(js_name=hostCall)]
pub fn host_call(operation:&str,input:&str)->std::result::Result<String,String>{
    let output=match operation {
        "issuerKeygen"=>serde_json::to_value(issuance::keygen()?).map_err(err)?,
        "testIssuerKeygen"=>{let seed:String=codec::parse(input)?;serde_json::to_value(issuance::test_keygen(&seed)?).map_err(err)?},
        "issue"=>serde_json::to_value(issuance::issue(input,false)?).map_err(err)?,
        "testIssue"=>serde_json::to_value(issuance::issue(input,true)?).map_err(err)?,
        "plan"=>request::plan_info(input)?,
        "request"=>serde_json::to_value(request::create(input)?).map_err(err)?,
        "holder"=>{let input:Value=codec::parse(input)?;let wire:request::RequestWire=serde_json::from_value(input["request"].clone()).map_err(err)?;let r=request::restore(&wire)?;let p=r.plan().map_err(err)?;request::checked_key(&p,r.key())?;request::freshness(&r,input["clock"].as_u64().ok_or("clock rejected")?)?;value!({"planHash":codec::hex(&p.hash())})},
        "prepareProof"=>prover::prepare(input)?,
        "bindProof"=>prover::bind(input)?,
        "statement"=>{let input:verification::Input=codec::parse(input)?;verification::statement(&input.request,&input.presentation)?},
        "auditArtifact"=>{let input:Value=codec::parse(input)?;let wire:request::RequestWire=serde_json::from_value(input["request"].clone()).map_err(err)?;
            let request=request::restore(&wire)?;sparq_zk_query::statement::Layout::new(&request.plan().map_err(err)?).audit_authenticated_artifact(&input["artifact"]).map_err(err)?;value!(true)},
        _=>return Err("unknown host operation".into()),
    };
    json(&output)
}

#[wasm_bindgen]
extern "C" {
    /// Synchronous host callback; arguments and storage outcomes use JSON strings.
    #[derive(Clone)]
    pub type Callback;
    #[wasm_bindgen(method,catch,js_name=call)]
    fn invoke(this:&Callback,context:&JsValue,input:&str)->std::result::Result<JsValue,JsValue>;
}
thread_local! {
    static CALLBACKS:RefCell<BTreeMap<u64,Callback>>=const{RefCell::new(BTreeMap::new())};
    static NEXT_CALLBACK:Cell<u64>=const{Cell::new(0)};
}
struct RegisteredStore(u64);
impl RegisteredStore {
    fn register(callback:&Callback)->Result<Self>{
        let id=NEXT_CALLBACK.with(|next|{let id=next.get().checked_add(1).ok_or("callback capacity")?;next.set(id);Ok::<_,String>(id)})?;
        CALLBACKS.with(|map|map.borrow_mut().insert(id,callback.clone()));Ok(Self(id))
    }
}
impl Drop for RegisteredStore {fn drop(&mut self){CALLBACKS.with(|map|map.borrow_mut().remove(&self.0));}}
impl ChallengeStore for RegisteredStore {
    fn consume(&self,challenge:&Challenge32)->std::result::Result<ChallengeOutcome,ChallengeStoreError>{
        let callback=CALLBACKS.with(|map|map.borrow().get(&self.0).cloned()).ok_or(ChallengeStoreError::new("callback_missing"))?;
        let value=callback.invoke(&JsValue::NULL,&codec::hex(challenge.as_bytes())).map_err(|_|ChallengeStoreError::new("callback_failed"))?;
        match value.as_string().as_deref(){Some("fresh")=>Ok(ChallengeOutcome::Fresh),Some("alreadyConsumed")=>Ok(ChallengeOutcome::AlreadyConsumed),_=>Err(ChallengeStoreError::new("invalid_outcome"))}
    }
}

/// Verify with externally supplied synchronous proof and atomic challenge calls.
/// Rust derives the statement, checks its key and window, and consumes last.
#[wasm_bindgen(js_name=verifyWith)]
pub fn verify_with(input:&str,proof:&Callback,store:&Callback,clock:&Callback)->String{
    let result=(||{
        let input:verification::Input=codec::parse(input)?;let store=RegisteredStore::register(store)?;
        verification::verify(input,|statement|{
            proof.invoke(&JsValue::NULL,&json(statement)?).map_err(|_|"proof backend failed".to_string())?.as_bool().ok_or_else(||"proof backend outcome rejected".into())
        },&store,||{
            let value=clock.invoke(&JsValue::NULL,"").map_err(|_|"clock callback failed".to_string())?;
            let now=value.as_f64().ok_or("clock outcome rejected")?;
            if !now.is_finite() || now<0.0 || now.fract()!=0.0 || now>9_007_199_254_740_991.0{return Err("clock outcome rejected".into());}
            Ok(now as u64)
        })
    })();
    match result{Ok(v)=>v.to_string(),Err(e)=>presentation::refusal(e).to_string()}
}

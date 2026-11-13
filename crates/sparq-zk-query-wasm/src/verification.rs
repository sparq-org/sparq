use crate::{codec::{err,hex,Result},presentation::{AnswerWire,PresentationWire},request::{self,RequestWire}};
use serde::Deserialize;
use serde_json::{json,Value};
use sparq_query_protocol::{consume_challenge,ChallengeStore};
use sparq_zk_query::{credential::SuiteEncoder,statement::{Layout,Statement}};

#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub(crate) struct Input {pub request:RequestWire,pub presentation:PresentationWire}

pub(crate) fn statement(wire:&RequestWire,presentation:&PresentationWire)->Result<Value> {
    let request=request::restore(wire)?;
    let p=presentation.native()?;
    if p.request_digest!=request.digest(){return Err("request digest".into());}
    request::agreement(&request,p.input_commitment)?;
    let limits=request.stored().requirements().resources();
    if let sparq_zk_query::answer::Answer::Select{rows,..}=&p.result {
        if rows.len()>limits.released_rows() as usize{return Err("released result capacity".into());}
    }
    if p.encoded().map_err(err)?.len()>limits.presentation_bytes() as usize || serde_json::to_vec(presentation).map_err(err)?.len()>limits.presentation_bytes() as usize {
        return Err("presentation bytes capacity".into());
    }
    let plan=request.plan().map_err(err)?;
    request::checked_key(&plan,request.key())?;
    let statement=Statement{request_digest:request.digest(),input_commitment:p.input_commitment,signature_mode:p.signature_mode,result:p.result.encode(&SuiteEncoder).map_err(err)?};
    let public=Layout::new(&plan).reconstruct_authenticated_with_envelope(&plan,&statement,&request.envelope_inputs().map_err(err)?).map_err(err)?;
    Ok(json!({"proof":presentation.proof,"verificationKey":hex(request.key()),"publicInputs":public.into_iter().map(sparq_zk_query::statement::hex).collect::<Vec<_>>()}))
}

pub(crate) fn verify(
    input:Input, proof:impl FnOnce(&Value)->Result<bool>,store:&dyn ChallengeStore,clock:impl Fn()->Result<u64>,
)->Result<Value>{
    let request=request::restore(&input.request)?;
    request::freshness(&request,clock()?)?;
    let statement=statement(&input.request,&input.presentation)?;
    if input.presentation.proof.is_empty(){return Err("missing proof".into());}
    if !proof(&statement)?{return Err("cryptographic proof rejected".into());}
    let plan=request.plan().map_err(err)?;
    let accepted=json!({"verified":true,"released":AnswerWire::from_answer(&input.presentation.result.answer()?),
        "report":{"rule":{"query":request.stored().query(),"planHash":hex(&plan.hash())},
        "issuers":input.request.policy.trusted_issuers,"subject":[],"fresh":hex(request.stored().challenge().as_bytes()),"complete":true,
        "issuerRoot":hex(&request.issuer_root().map_err(err)?)}});
    request::freshness(&request,clock()?)?;
    consume_challenge(store,request.stored()).map_err(err)?;
    Ok(accepted)
}

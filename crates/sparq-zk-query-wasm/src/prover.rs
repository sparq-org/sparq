use crate::{codec::{err,hex,noir_numbers,witness_json,Result},issuance::Credential,presentation::PresentationWire,request::{self,RequestWire}};
use serde::Deserialize;
use serde_json::{json as value,Value};
use sparq_core::Graph;
use sparq_zk_query::{answer::Answer,credential::SuiteEncoder,interp,method::Presentation,statement::{Layout,Statement},witness};

#[derive(Deserialize)] #[serde(deny_unknown_fields)]
struct Input { #[serde(rename="request")] _request:RequestWire,credentials:Vec<Credential>,clock:u64 }
pub(crate) fn prepare(input:&str)->Result<Value> {
    // Request/key checks precede decoding any private credential into suite cells.
    let envelope:Value=crate::codec::parse(input)?;
    let request_wire:RequestWire=serde_json::from_value(envelope["request"].clone()).map_err(err)?;
    let request=request::restore(&request_wire)?;let plan=request.plan().map_err(err)?;
    request::checked_key(&plan,request.key())?;
    let input:Input=serde_json::from_value(envelope).map_err(err)?;
    request::freshness(&request,input.clock)?;
    let keys=request_wire.policy.keys()?;let mut data=vec![];let mut vocabulary=vec![];
    for credential in &input.credentials {
        let (credential,key)=credential.open()?;
        let index=keys.iter().position(|k|k==&key).ok_or("credential issuer is outside the trusted set")?;
        let siblings=sparq_zk_compose::issuer::key_membership_witness(&keys,sparq_zk_query::credential::ISSUER_DEPTH,index as u64).ok_or("issuer membership rejected")?;
        let graph=Graph::load_str(&credential.document,"nquads").map_err(err)?;
        for triple in graph.iter_ids() {for id in triple {vocabulary.push(graph.dict().term(id));}}
        data.push(sparq_zk_query::credential::Credential{credential,issuer:key,issuer_index:index as u64,issuer_siblings:siblings});
    }
    let cells=data.iter().enumerate().map(|(i,c)|c.cells(i as u32).map_err(err)).collect::<Result<Vec<_>>>()?;
    let trace=interp::evaluate(&plan,&cells,&SuiteEncoder).map_err(err)?;
    let result=Answer::release(&trace.result,&plan,&vocabulary,&SuiteEncoder).map_err(err)?;
    let mut salt=[0;32];getrandom_latest::fill(&mut salt).map_err(err)?;
    let commitment=sparq_zk_query::credential::dataset_commitment(&data,salt).map_err(err)?;
    request::agreement(&request,commitment)?;
    let statement=Statement{request_digest:request.digest(),input_commitment:commitment,signature_mode:1,result:result.encode(&SuiteEncoder).map_err(err)?};
    let envelope=request.envelope_inputs().map_err(err)?;
    let public=Layout::new(&plan).reconstruct_authenticated_with_envelope(&plan,&statement,&envelope).map_err(err)?;
    let toml=witness::prover_toml_authenticated_with_envelope(&plan,&data,&trace,request.digest(),salt,&envelope).map_err(err)?;
    let presentation=PresentationWire::from_native(&Presentation{request_digest:request.digest(),input_commitment:commitment,signature_mode:1,result,proof:vec![]});
    Ok(value!({"planHash":hex(&plan.hash()),"packageName":format!("query_{}",hex(&plan.hash())),
        "witnessInputs":{"json":noir_numbers(witness_json(&toml)?),"toml":toml},"presentationSkeleton":presentation,
        "publicInputs":public.into_iter().map(sparq_zk_query::statement::hex).collect::<Vec<_>>(),"result":presentation.result}))
}
pub(crate) fn bind(input:&str)->Result<Value> {
    #[derive(Deserialize)] #[serde(rename_all="camelCase",deny_unknown_fields)]
    struct Input{request:RequestWire,presentation:PresentationWire,public_inputs:Vec<String>,clock:u64}
    let input:Input=crate::codec::parse(input)?;
    let request=request::restore(&input.request)?;request::freshness(&request,input.clock)?;
    let expected=crate::verification::statement(&input.request,&input.presentation)?;
    if expected["publicInputs"]!=value!(input.public_inputs){return Err("prover public-input mismatch".into());}
    if input.presentation.proof.is_empty(){return Err("prover returned no proof".into());}
    Ok(value!({"result":input.presentation.result,"presentation":input.presentation}))
}

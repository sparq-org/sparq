use crate::codec::{bytes, digest, err, hex, word, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_query_protocol::*;
use sparq_zk::sig;
use sparq_zk_query::{capacity::Bounds, lower::lower, plan::{Plan,ResultForm}, request::{self, Request}};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Limits { pub credentials:u32, pub quads_per_credential:u32, pub rows_per_table:u32 }
impl Limits {
    pub fn bounds(&self)->Bounds { Bounds{credentials:self.credentials,quads_per_credential:self.quads_per_credential,rows_per_table:self.rows_per_table} }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Policy {
    pub bounds:Limits, pub trusted_issuers:Vec<String>, pub audience:String,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub not_before:Option<u64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub not_after:Option<u64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub credential_quad_bounds:Option<Vec<u32>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub input_commitment:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub max_presentation_bytes:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub max_result_size:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub holder_binding:Option<Value>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub accepted_snapshots:Option<Value>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub validity_time:Option<u64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub earliest_created:Option<u64>,
}
impl Policy {
    pub fn normalize(&self)->Result<Self> {
        if self.holder_binding.is_some() || self.accepted_snapshots.is_some() || self.validity_time.is_some() || self.earliest_created.is_some() {
            return Err("credential envelope policy integration is unavailable".into());
        }
        let mut p=self.clone();
        p.trusted_issuers=self.keys()?.iter().map(sig::public_key_to_hex).collect();
        p.trusted_issuers.sort();p.trusted_issuers.dedup();
        if p.trusted_issuers.is_empty() {return Err("trusted issuer set is empty".into());}
        Ok(p)
    }
    pub fn keys(&self)->Result<Vec<sig::PublicKey>> {
        self.trusted_issuers.iter().map(|key|sig::public_key_from_hex(key).ok_or_else(||"trusted issuer key rejected".into())).collect()
    }
    pub fn plan(&self,query:&str)->Result<Plan> {
        let plan=lower(query,self.bounds.bounds()).map_err(err)?;
        if let Some(limits)=&self.credential_quad_bounds {plan.with_credential_quad_bounds(limits).map_err(err)} else {Ok(plan)}
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct RequestWire {
    pub query:String,pub policy:Policy,pub nonce:String,pub plan_hash:String,
    pub verification_key:String,pub digest:String,pub request_rdf:String,
}
fn id(s:&str)->Result<Identifier>{Identifier::new(s).map_err(err)}
fn native(query:&str,p:&Policy,nonce:&str,key:&[u8])->Result<Request> {
    let plan=p.plan(query)?; let keys=p.keys()?;
    let root=sparq_zk_compose::issuer::key_set_root(&keys,sparq_zk_query::credential::ISSUER_DEPTH).ok_or("issuer set capacity or key rejected")?;
    let anchor=p.input_commitment.as_deref().map(word).transpose()?.map(Digest32::new).transpose().map_err(err)?;
    let is_ask=plan.form()==ResultForm::Ask;
    let requirements=QueryRequirements::new(RequirementsSpec{
        contract:if is_ask {ResultContract::AskBoolean}else{ResultContract::SelectSequence},
        mode:EvaluationMode::ExactBounded,
        authority:if anchor.is_some(){ScopeAuthority::VerifierAgreedAnchor}else{ScopeAuthority::HolderDeclared},
        anchor,source_evidence:Some(SourceEvidence::IssuerAuthenticated),status:Some(StatusPolicy::NotRequested),holder:Some(HolderPolicy::BearerAccepted),
        assembly:DatasetAssembly::UnionDefaultGraph,
        query_profile:QueryProfile{dialect:id(request::DIALECT)?,fragment:id(request::PROFILE)?},
        accepted_suites:vec![id(request::SUITE)?],accepted_mappings:vec![id(request::PROFILE)?],accepted_linking:vec![id(request::PROFILE)?],
        resources:ResourceBounds::new(p.max_result_size.unwrap_or(p.bounds.rows_per_table),p.max_presentation_bytes.unwrap_or(1<<24)).map_err(err)?,
        methods:vec![request::descriptor(&plan,key).map_err(err)?],
    }).map_err(err)?;
    let stored=StoredRequest::new(StoredRequestSpec{
        requirements,query:query.into(),challenge:Challenge32::new(word(nonce)?).map_err(err)?,audience:id(&p.audience)?,
        not_before:p.not_before.ok_or("stored request needs its validity start")?,not_after:p.not_after.ok_or("stored request needs its validity end")?,
        form:if is_ask{QueryForm::Ask}else{QueryForm::Select},base_iri:None,describe_policy:None,
    }).map_err(err)?;
    let root=sparq_merkle_suite::poseidon::field_bytes(root);
    if let Some(limits)=&p.credential_quad_bounds {Request::new_authenticated_with_quad_bounds(stored,p.bounds.bounds(),key.to_vec(),root,limits).map_err(err)}
    else{Request::new_authenticated(stored,p.bounds.bounds(),key.to_vec(),root).map_err(err)}
}
pub(crate) fn checked_key(plan:&Plan,key:&[u8])->Result<()> {
    let table:Value=serde_json::from_str(crate::CATALOGUE).map_err(err)?;
    let h=hex(&plan.hash());
    let entry=table["entries"].as_array().ok_or("catalogue rejected")?.iter().find(|e|e["planHash"]==h).ok_or("missing catalogue plan")?;
    if entry["vkSha256"].as_str()!=Some(digest(key).as_str()) {return Err("verification key mismatch".into());}
    Ok(())
}
pub(crate) fn create(input:&str)->Result<RequestWire> {
    #[derive(Deserialize)] #[serde(rename_all="camelCase",deny_unknown_fields)]
    struct Input{query:String,policy:Policy,nonce:String,verification_key:String,clock:u64}
    let input:Input=crate::codec::parse(input)?;let mut policy=input.policy.normalize()?;
    policy.not_before.get_or_insert(input.clock);
    policy.not_after.get_or_insert(input.clock.checked_add(3600).ok_or("request time overflow")?);
    let key=bytes(&input.verification_key)?;let plan=policy.plan(&input.query)?;checked_key(&plan,&key)?;
    let nonce=hex(&word(&input.nonce)?);let request=native(&input.query,&policy,&nonce,&key)?;
    Ok(RequestWire{query:input.query,policy,nonce,plan_hash:hex(&plan.hash()),verification_key:hex(&key),digest:hex(&request.digest()),request_rdf:request.canonical_rdf().into()})
}
pub(crate) fn restore(wire:&RequestWire)->Result<Request> {
    let policy=wire.policy.normalize()?;
    if policy!=wire.policy {return Err("noncanonical stored policy".into());}
    let key=bytes(&wire.verification_key)?;
    let request=native(&wire.query,&policy,&wire.nonce,&key)?;
    if wire.plan_hash!=hex(&request.plan().map_err(err)?.hash()) || wire.digest!=hex(&request.digest()) || wire.request_rdf!=request.canonical_rdf()
        || wire.nonce!=hex(request.stored().challenge().as_bytes()) || wire.verification_key!=hex(&key) {
        return Err("stored request reconstruction differs".into());
    }
    Ok(request)
}
pub(crate) fn plan_info(input:&str)->Result<Value> {
    #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Input{query:String,policy:Policy}
    let input:Input=crate::codec::parse(input)?;let policy=input.policy.normalize()?;let plan=policy.plan(&input.query)?;
    let package=sparq_zk_query::codegen::generate(&plan).map_err(err)?;let capacity=plan.capacity();
    Ok(json!({"planHash":hex(&plan.hash()),"packageName":package.name,"files":package.files,
        "capacity":{"credentials":capacity.credentials,"quadsPerCredential":capacity.quads_per_credential,"rowsPerTable":capacity.rows_per_table},
        "publicInputDeclarations":sparq_zk_query::statement::Layout::new(&plan).declarations_authenticated()}))
}
pub(crate) fn freshness(request:&Request,clock:u64)->Result<()> {
    if clock<request.stored().not_before() || clock>=request.stored().not_after(){Err("request validity window".into())}else{Ok(())}
}
pub(crate) fn agreement(request:&Request,commitment:[u8;32])->Result<()> {
    if request.stored().requirements().anchor().is_some_and(|a|a.as_bytes()!=&commitment){Err("dataset commitment agreement".into())}else{Ok(())}
}

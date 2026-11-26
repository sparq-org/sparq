use crate::{
    codec::{err, hex, Result},
    presentation::{AnswerWire, PresentationWire},
    request::{self, RequestWire},
};
use serde::Deserialize;
use serde_json::{json, Value};
use sparq_query_protocol::{consume_challenge, ChallengeStore};
use sparq_zk_query::{
    credential::SuiteEncoder,
    request::Request,
    statement::{Layout, Statement},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub request: RequestWire,
    pub presentation: PresentationWire,
}

pub(crate) fn statement(request: &Request, presentation: &PresentationWire) -> Result<Value> {
    let p = presentation.native()?;
    if p.request_digest != request.digest() {
        return Err("request digest".into());
    }
    request.check_dataset_agreement(p.input_commitment).map_err(err)?;
    let limits = request.stored().requirements().resources();
    if let sparq_zk_query::answer::Answer::Select { rows, .. } = &p.result {
        if rows.len() > limits.released_rows() as usize {
            return Err("released result capacity".into());
        }
    }
    if p.encoded().map_err(err)?.len() > limits.presentation_bytes() as usize
        || serde_json::to_vec(presentation).map_err(err)?.len()
            > limits.presentation_bytes() as usize
    {
        return Err("presentation bytes capacity".into());
    }
    let plan = request.plan().map_err(err)?;
    request::checked_key(&plan, request.key())?;
    let statement = Statement {
        request_digest: request.digest(),
        input_commitment: p.input_commitment,
        signature_mode: p.signature_mode,
        result: p.result.encode(&SuiteEncoder).map_err(err)?,
    };
    let public = Layout::new(&plan)
        .reconstruct_authenticated_with_envelope(
            &plan,
            &statement,
            &request.envelope_inputs().map_err(err)?,
        )
        .map_err(err)?;
    Ok(
        json!({"proof":presentation.proof,"verificationKey":hex(request.key()),"publicInputs":public.into_iter().map(sparq_zk_query::statement::hex).collect::<Vec<_>>()}),
    )
}

pub(crate) fn verify(
    input: Input,
    proof: impl FnOnce(&Value) -> Result<bool>,
    store: &dyn ChallengeStore,
    clock: impl Fn() -> Result<u64>,
) -> Result<Value> {
    let request = request::restore(&input.request)?;
    request::freshness(&request, clock()?)?;
    let statement = statement(&request, &input.presentation)?;
    if input.presentation.proof.is_empty() {
        return Err("missing proof".into());
    }
    if !proof(&statement)? {
        return Err("cryptographic proof rejected".into());
    }
    let plan = request.plan().map_err(err)?;
    let policy = &input.request.policy;
    // Holder-bound issuers: the listed ones, or every trust member issuer.
    let subject: Vec<String> = match &policy.holder_binding {
        None => vec![],
        Some(binding) => binding.issuers.clone().unwrap_or_else(|| {
            let mut all: Vec<String> = policy
                .trust_members
                .iter()
                .map(|m| m.issuer.clone())
                .collect();
            all.sort();
            all.dedup();
            all
        }),
    };
    let exactness = if request.stored().requirements().anchor().is_some() {
        "agreed-dataset"
    } else {
        "presented-dataset"
    };
    let mut report = json!({"rule":{"query":request.stored().query(),"planHash":hex(&plan.hash())},
        "issuers":policy.trusted_issuers,"subject":subject,"fresh":hex(request.stored().challenge().as_bytes()),"complete":true,
        "trustMembers":policy.trust_members,
        "issuerRoot":hex(&request.issuer_root().map_err(err)?),"exactness":exactness});
    if let Some(evidence) = request.coverage() {
        report["coverage"] = crate::coverage::report(evidence);
    }
    // Only enforced envelope properties are reported.
    if let Some(snapshots) = &policy.accepted_snapshots {
        report["status"] =
            json!({"acceptedSnapshots":snapshots.digests,"allowMissing":snapshots.allow_missing});
    }
    if let Some(t) = policy.validity_time {
        report["validityTime"] = json!(t);
    }
    if let Some(t) = policy.earliest_created {
        report["earliestCreated"] = json!(t);
    }
    if let Some(rows) = &policy.table_row_bounds {
        report["tableRowBounds"] = json!(rows);
    }
    let accepted = json!({"verified":true,"released":AnswerWire::from_answer(&input.presentation.result.answer()?),
        "report":report});
    request::freshness(&request, clock()?)?;
    consume_challenge(store, request.stored()).map_err(err)?;
    Ok(accepted)
}

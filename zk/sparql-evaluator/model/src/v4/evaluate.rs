// [GPT-6] Context specialization and evaluation both execute inside the guest.
use super::*;
use crate::evaluate::{DatasetProfile, admit_query};
use oxrdf::{Literal, NamedNode};
use sparq_core::temporal::{ExactTimeline, year_within_capacity};
use sparq_engine::PreparedQuery;

mod specialize;
const DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

fn prepare(request: &Request) -> Result<PreparedQuery, Rejected> {
    validate_request(request)?;
    let lexical = &request.context.now.datetime;
    let trimmed = lexical.trim_matches([' ', '\t', '\r', '\n']);
    // [GPT-6] This typed context has no implicit string-constructor preprocessing.
    if trimmed != lexical.as_str() {
        return Err(Rejected("V4 NOW requires strict dateTime lexical form"));
    }
    let zone = trimmed
        .split_once('T')
        .is_some_and(|(_, time)| time.ends_with('Z') || time.find(['+', '-']).is_some());
    if !year_within_capacity(lexical, DATETIME, 1, 1_000_000_000) {
        return Err(Rejected("V4 NOW temporal year capacity"));
    }
    if !zone || ExactTimeline::parse_datetime(lexical).is_none() {
        return Err(Rejected("V4 NOW requires a valid dateTime with timezone"));
    }
    let mut query = PreparedQuery::parse(&request.query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?
        .into_query();
    let now = Literal::new_typed_literal(lexical, NamedNode::new_unchecked(DATETIME));
    specialize::now(&mut query, &now)?;
    // [GPT-6] All ordinary exclusions apply after the only allowed specialization.
    // Safe aggregate constants can then pass without admitting fallible operands.
    admit_query(&query, DatasetProfile::GraphResultsBlankFree)?;
    Ok(query.into())
}

/// Validates context and the complete query, including every nested expression.
///
/// Source blank nodes additionally restrict EXISTS during complete evaluation.
///
/// # Errors
/// Rejects invalid contexts, excluded operations and query/specialization capacities.
pub fn admit(request: &Request) -> Result<(), Rejected> {
    prepare(request).map(|_| ())
}

/// Evaluates the actual complete dataset using the independently bound NOW context.
///
/// # Errors
/// Rejects invalid input, wrong source anchors, unsupported queries and any capacity failure.
pub fn evaluate(witness: &Witness) -> Result<Journal, Rejected> {
    let prepared = prepare(&witness.request)?;
    let commitment = dataset_commitment(&witness.dataset, &witness.request.policy)?;
    let provenance = match witness.request.authority {
        DatasetAuthority::VerifierAgreed {
            commitment: expected,
        } => {
            if commitment != expected {
                return Err(Rejected("V4 complete dataset anchor mismatch"));
            }
            Provenance::VerifierAcceptedCommitment
        }
        DatasetAuthority::HolderDeclared => Provenance::HolderDeclaredOnly,
    };
    let result =
        v3::evaluate::evaluate_prepared(&witness.dataset, &witness.request.policy, &prepared)?;
    Ok(Journal {
        version: VERSION,
        request_digest: request_digest(&witness.request)?,
        dataset_commitment: commitment,
        provenance,
        context: witness.request.context.clone(),
        result,
    })
}

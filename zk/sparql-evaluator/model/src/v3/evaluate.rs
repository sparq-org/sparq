// [GPT-6] Native/guest-shared V3 evaluation; execution evidence is separate.
use super::*;
use crate::evaluate::{DatasetProfile, admit_query, ordered, query_budget};
use sparq_engine::{
    PreparedQuery, construct_prepared_with_budget, describe_prepared_with_budget,
    query_prepared_with_budget,
};

/// Checks the complete V3 query, including templates and nested expressions.
///
/// # Errors
/// Rejects unimplemented or nondeterministic operations and excessive query structure.
pub fn admit(request: &Request) -> Result<(), Rejected> {
    validate_request(request)?;
    let prepared =
        PreparedQuery::parse(&request.query).map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(prepared.query(), DatasetProfile::GraphResults)
}

/// Executes the exact source and canonicalizes the complete observable result.
///
/// # Errors
/// Rejects wrong dataset anchors, unsupported input, evaluation errors and any
/// source, query, result-encoding or canonicalization capacity exhaustion.
pub fn evaluate(witness: &Witness) -> Result<Journal, Rejected> {
    let request = &witness.request;
    validate_request(request)?;
    let commitment = dataset_commitment(&witness.dataset, &request.policy)?;
    let provenance = match request.authority {
        DatasetAuthority::VerifierAgreed {
            commitment: expected,
        } => {
            if commitment != expected {
                return Err(Rejected("V3 complete dataset anchor mismatch"));
            }
            Provenance::VerifierAcceptedCommitment
        }
        DatasetAuthority::HolderDeclared => Provenance::HolderDeclaredOnly,
    };
    let prepared =
        PreparedQuery::parse(&request.query).map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(prepared.query(), DatasetProfile::GraphResults)?;
    let graph = v2::evaluate::build_dataset(&witness.dataset, &request.policy.dataset, true)?;
    let max_rows = request.policy.dataset.max_rows;
    let budget = query_budget(max_rows);
    let canonical = &request.policy.canonicalization;
    let result = match prepared.query() {
        spargebra::Query::Select { pattern, .. } => {
            let table = query_prepared_with_budget(&graph, &prepared, &budget)
                .map_err(|_| Rejected("V3 query evaluation or resource budget rejected"))?;
            if table.rows.len() > max_rows as usize {
                return Err(Rejected("V3 result row capacity"));
            }
            result::select(
                table,
                if ordered(pattern) {
                    RowOrder::Sequence
                } else {
                    RowOrder::Bag
                },
                canonical,
            )?
        }
        spargebra::Query::Ask { .. } => {
            let table = query_prepared_with_budget(&graph, &prepared, &budget)
                .map_err(|_| Rejected("V3 query evaluation or resource budget rejected"))?;
            CanonicalResult::Ask(!table.rows.is_empty())
        }
        spargebra::Query::Construct { .. } => {
            let triples = construct_prepared_with_budget(&graph, &prepared, &budget)
                .map_err(|_| Rejected("V3 graph evaluation or resource budget rejected"))?;
            result::graph(triples, canonical)?
        }
        spargebra::Query::Describe { .. } => {
            // [GPT-6] The enum has one explicitly bound closure policy; the engine
            // traverses outgoing blank objects in the selected active default graph.
            let triples = match &request.policy.describe {
                DescribePolicy::OutgoingBlankNodeClosure => {
                    describe_prepared_with_budget(&graph, &prepared, &budget)
                }
            }
            .map_err(|_| Rejected("V3 graph evaluation or resource budget rejected"))?;
            result::graph(triples, canonical)?
        }
    };
    result::check_output(&result, canonical)?;
    Ok(Journal {
        version: VERSION,
        request_digest: request_digest(request)?,
        dataset_commitment: commitment,
        provenance,
        result,
    })
}

//! Checks this method makes before any credential is cloned or evaluated.

use sparq_proved_evaluator_model::MAX_QUERY_BYTES;
use sparq_proved_evaluator_model::authenticated_rdf::{
    MAX_AUTHORIZED_KEYS, MAX_CREDENTIALS, MAX_DOCUMENT_BYTES, MAX_IRI_BYTES,
    MAX_PROOF_CONFIG_BYTES, MAX_TOTAL_BYTES, Request, SignedCredential,
};

use crate::Rejected;

/// Largest JSON encoding of a [`crate::DisclosedProof`] that [`crate::decode_proof`]
/// parses. JSON escapes at most six bytes per input byte, and each signature
/// byte is at most four.
pub const MAX_PROOF_JSON_BYTES: usize = 6 * MAX_TOTAL_BYTES + MAX_CREDENTIALS * 64 * 4 + 4_096;

/// Rejects a credential list over the relation's capacities, from lengths alone.
pub(crate) fn admit_sizes(credentials: &[SignedCredential]) -> Result<(), Rejected> {
    if credentials.is_empty() || credentials.len() > MAX_CREDENTIALS {
        return Err(Rejected("disclosed credential count must be 1 to 4"));
    }
    let mut total = 0usize;
    for credential in credentials {
        if credential.signature.len() != 64
            || credential.document.len() > MAX_DOCUMENT_BYTES
            || credential.proof_config.len() > MAX_PROOF_CONFIG_BYTES
        {
            return Err(Rejected("disclosed credential capacity"));
        }
        total += credential.document.len() + credential.proof_config.len();
    }
    if total > MAX_TOTAL_BYTES {
        return Err(Rejected("disclosed credential total capacity"));
    }
    Ok(())
}

/// Rejects a request over the relation's capacities, from lengths alone, and
/// a query form this method does not admit.
pub(crate) fn admit_request(request: &Request) -> Result<(), Rejected> {
    let keys = &request.policy.authorization;
    if keys.is_empty()
        || keys.len() > MAX_AUTHORIZED_KEYS
        || keys.iter().any(|key| {
            key.issuer.len() > MAX_IRI_BYTES || key.verification_method.len() > MAX_IRI_BYTES
        })
    {
        return Err(Rejected("disclosed request key table capacity"));
    }
    admit_query_form(&request.query)
}

/// Rejects the query forms the answer specification excludes from version 1
/// that the shared relation would otherwise evaluate: DESCRIBE, and FROM or
/// FROM NAMED clauses. The relation itself rejects SERVICE and the functions
/// whose value depends on when or where they are evaluated.
fn admit_query_form(query: &str) -> Result<(), Rejected> {
    if query.len() > MAX_QUERY_BYTES {
        return Err(Rejected("query capacity"));
    }
    let query = spargebra::SparqlParser::new()
        .parse_query(query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?;
    if matches!(query, spargebra::Query::Describe { .. }) {
        return Err(Rejected("DESCRIBE is not admitted"));
    }
    if query.dataset().is_some() {
        return Err(Rejected("FROM and FROM NAMED are not admitted"));
    }
    Ok(())
}

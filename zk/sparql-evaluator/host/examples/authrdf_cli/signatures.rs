use sparq_proved_evaluator::Error;
use sparq_proved_evaluator_model::authenticated_rdf::{self as model, Journal, Request};

// Matching is preparation only. The host verifier still verifies the receipt,
// binds the independent request and checks these signatures before nonce use.
pub(crate) fn in_journal_order(
    journal: &Journal,
    request: &Request,
    mut signatures: Vec<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, Error> {
    if journal.signed_messages.is_empty()
        || journal.signed_messages.len() > model::MAX_CREDENTIALS
        || journal.signed_messages.len() != signatures.len()
    {
        return Err(Error("credential signature count mismatch"));
    }
    let mut individual = journal.clone();
    let mut ordered = Vec::new();
    for entry in &journal.signed_messages {
        individual.signed_messages = vec![entry.clone()];
        let index = signatures
            .iter()
            .position(|signature| {
                model::check_revealed_signatures(
                    &individual,
                    request,
                    std::slice::from_ref(signature),
                )
                .is_ok()
            })
            .ok_or(Error("credential signatures do not match journal"))?;
        ordered.push(signatures.remove(index));
    }
    Ok(ordered)
}

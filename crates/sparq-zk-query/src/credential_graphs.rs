//! Credential graph profile and lane E's issuer-quad stand-in.
//!
//! Credentials are ordered by authenticated signed-message bytes, with canonical
//! document and configuration bytes as tie breakers. Graph labels use the suite
//! request-scoped constructor and that credential rank as their scope. Identical
//! complete credentials still occupy distinct graphs; their tie order is immaterial.
//! The signed RDF vc:issuer field supplies the issuer IRI. A verifier-owned
//! public registry binds it to the signing key verified by the suite. Lane E
//! replaces this stand-in with its policy and envelope integration. This profile is research code and is not externally audited.
use crate::{
    capacity::Bounds,
    credential::{Credential, SuiteEncoder},
    interp::{Cell, CredentialCells, EncodeTerm, Row},
    plan::{Operator, Plan, Slot, Var},
    plan_closed::ClosedOperator,
    Error,
};
pub const VERIFIED_ISSUER: &str = crate::credential::VERIFIED_ISSUER;
pub const ISSUER: &str = "https://www.w3.org/2018/credentials#issuer";
pub fn canonical_credentials(credentials: Vec<Credential>) -> Result<Vec<Credential>, Error> {
    let mut keyed = credentials
        .into_iter()
        .map(|c| {
            let message = sparq_merkle_suite::poseidon::field_bytes(c.signed_message()?);
            Ok((message, c))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    keyed.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then_with(|| x.credential.document.cmp(&y.credential.document))
            .then_with(|| x.credential.proof_config.cmp(&y.credential.proof_config))
    });
    Ok(keyed.into_iter().map(|(_, c)| c).collect())
}
pub fn lower(query: &str, bounds: Bounds) -> Result<Plan, Error> {
    let mut plan = crate::lower::lower(query, bounds)?;
    for node in &mut plan.nodes {
        if let Operator::Scan(pattern) = &node.operator {
            node.operator = Operator::Closed(ClosedOperator::DatasetScan {
                pattern: pattern.clone(),
            });
        }
    }
    if plan.nodes.len() >= 256 {
        return Err(Error::Capacity("plan structure".into()));
    }
    plan.nodes.push(crate::plan::Node {
        operator: Operator::Closed(ClosedOperator::CredentialProfile {
            lexical_capacity: 64,
            literal_capacity: 64,
            literal_types: 31,
            issuers: vec![],
        }),
        output: crate::plan::Schema {
            variables: vec![],
            capacity: plan.capacity().rows_per_table,
            order: crate::plan::CanonicalOrder::TermEncoding,
        },
    });
    Ok(plan)
}
/// Pin integer (1), date/boolean (6), all three (7), or add decimal/string (31) families.
/// The restriction covers every credential row and is included in the plan hash.
/// The integer/date-boolean profiles bound literal lexical values at sixteen bytes.
pub fn with_literal_types(mut plan: Plan, types: u8) -> Result<Plan, Error> {
    if !matches!(types, 1 | 6 | 7 | 31) {
        return Err(Error::Unsupported(
            "closed credential literal profile".into(),
        ));
    }
    let profile = plan
        .nodes
        .iter_mut()
        .find_map(|n| {
            if let Operator::Closed(ClosedOperator::CredentialProfile {
                literal_types,
                literal_capacity,
                ..
            }) = &mut n.operator
            {
                Some((literal_types, literal_capacity))
            } else {
                None
            }
        })
        .ok_or_else(|| Error::Invalid("credential graph profile".into()))?;
    *profile.0 = types;
    *profile.1 = match types {
        7 | 31 => 64,
        _ => 16,
    };
    Ok(plan)
}
pub(crate) fn check_literal_types(types: u8, credentials: &[CredentialCells]) -> Result<(), Error> {
    if !matches!(types, 1 | 6 | 7 | 31) {
        return Err(Error::Unsupported(
            "closed credential literal profile".into(),
        ));
    }
    let mut allowed = vec![];
    for (flag, dt, lexical) in [
        (1, "integer", "0"),
        (2, "boolean", "false"),
        (4, "date", "2000-01-01"),
        (8, "decimal", "0.0"),
        (16, "string", ""),
    ] {
        if types & flag != 0 {
            let c = SuiteEncoder.encode(
                &oxrdf::Literal::new_typed_literal(
                    lexical,
                    oxrdf::NamedNode::new_unchecked(format!(
                        "http://www.w3.org/2001/XMLSchema#{dt}"
                    )),
                )
                .into(),
            )?;
            allowed.push(c.dt);
        }
    }
    if credentials
        .iter()
        .flat_map(|c| &c.quads)
        .any(|q| q[2].kind == 3 && !allowed.contains(&q[2].dt))
    {
        return Err(Error::Unsupported(
            "closed credential literal profile".into(),
        ));
    }
    Ok(())
}
pub fn name(index: u32) -> Result<Cell, Error> {
    crate::credential::credential_graph([0; 32], index)
}
pub(crate) fn is_profile(plan: &Plan) -> bool {
    plan.nodes.iter().any(|n| {
        matches!(
            n.operator,
            Operator::Closed(ClosedOperator::CredentialProfile { .. })
        )
    })
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan(
    pattern: &[Slot; 3],
    vars: &[Var],
    context: Option<&[Cell]>,
    ctxvars: &[Var],
    selection: (Option<u32>, bool),
    credentials: &[CredentialCells],
    params: &[Option<Cell>],
    names: &[Cell],
) -> Result<Vec<Row>, Error> {
    let (credential, include_issuers) = selection;
    if !ctxvars.is_empty() && context.is_none() {
        return Ok(vec![]);
    }
    let mut source = vec![];
    if let Some(c) = credential {
        if let Some(c) = credentials.get(c as usize) {
            source.extend(c.quads.iter().map(|q| [q[0], q[1], q[2]]));
        }
    } else {
        let issuer = SuiteEncoder.encode(&oxrdf::NamedNode::new_unchecked(ISSUER).into())?;
        let predicate =
            SuiteEncoder.encode(&oxrdf::NamedNode::new_unchecked(VERIFIED_ISSUER).into())?;
        for (i, c) in credentials.iter().enumerate() {
            let mut named = None;
            for q in &c.quads {
                source.push([q[0], q[1], q[2]]);
                if include_issuers && q[1].same_term(issuer) {
                    if q[2].kind != 1 || named.is_some_and(|v: Cell| !v.same_term(q[2])) {
                        return Err(Error::Invalid("credential issuer field".into()));
                    }
                    named = Some(q[2]);
                }
            }
            if let Some(issuer) = named {
                source.push([
                    names.get(i).copied().unwrap_or(name(i as u32)?),
                    predicate,
                    issuer,
                ]);
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut rows = vec![];
    for q in source {
        if !seen.insert(crate::interp::row_key(&q)) {
            continue;
        }
        let mut row = vec![Cell::default(); vars.len()];
        let mut matches = true;
        for (i, s) in pattern.iter().enumerate() {
            match s {
                Slot::Parameter(p) => matches &= q[i].same_term(params[*p as usize].expect("term")),
                Slot::Variable(v) => {
                    let outer = ctxvars
                        .iter()
                        .position(|a| a == v)
                        .and_then(|c| context.map(|r| r[c]))
                        .unwrap_or_default();
                    if outer.bound {
                        matches &= q[i].same_term(outer);
                    } else {
                        let col = vars.iter().position(|a| a == v).expect("scan column");
                        matches &= !row[col].bound || row[col].same_term(q[i]);
                        row[col] = q[i];
                    }
                }
            }
        }
        if matches {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// Complete suite cells for the credential-scoped graph profile.
pub fn cells(credentials: &[Credential]) -> Result<Vec<CredentialCells>, Error> {
    credentials
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if !sparq_merkle_suite::poseidon::verify(&c.credential, &c.issuer) {
                return Err(Error::Invalid("suite credential signature".into()));
            }
            let mut quads = vec![];
            for q in c.quads()? {
                if q.predicate.as_str() == VERIFIED_ISSUER {
                    return Err(Error::Excluded(
                        "credential verifiedIssuer assertion".into(),
                    ));
                }
                if q.graph_name != oxrdf::GraphName::DefaultGraph {
                    return Err(Error::Unsupported("credential named graph".into()));
                }
                let terms = [
                    oxrdf::Term::from(q.subject),
                    oxrdf::Term::from(q.predicate),
                    q.object,
                ];
                let mut row = [Cell::default(); 4];
                for (p, t) in terms.iter().enumerate() {
                    let lexical = match t {
                        oxrdf::Term::NamedNode(n) => n.as_str(),
                        oxrdf::Term::BlankNode(n) => n.as_str(),
                        oxrdf::Term::Literal(l) => {
                            if !matches!(
                                l.datatype().as_str(),
                                "http://www.w3.org/2001/XMLSchema#integer"
                                    | "http://www.w3.org/2001/XMLSchema#date"
                                    | "http://www.w3.org/2001/XMLSchema#boolean"
                                    | "http://www.w3.org/2001/XMLSchema#decimal"
                                    | "http://www.w3.org/2001/XMLSchema#string"
                            ) {
                                return Err(Error::Unsupported(
                                    "closed credential literal datatype".into(),
                                ));
                            }
                            l.value()
                        }
                        _ => return Err(Error::Excluded("triple term".into())),
                    };
                    if lexical.len() > 64 {
                        return Err(Error::Capacity("closed credential lexical opening".into()));
                    }
                    row[p] = SuiteEncoder.encode(t)?;
                    if row[p].kind == 2 {
                        row[p].scope = i as u32 + 1;
                    }
                }
                quads.push(row);
            }
            Ok(CredentialCells { quads })
        })
        .collect()
}

/// Lane E stand-in: verifier-owned issuer IRI to verified signing-key registry.
/// These public term parameters are reconstructed by the verifier, never read
/// as trusted identity policy from a presentation.
pub fn bind_issuer_registry(
    mut plan: Plan,
    registry: &[(oxrdf::NamedNode, sparq_zk::sig::PublicKey)],
) -> Result<Plan, Error> {
    if registry.is_empty()
        || registry.len() > 16
        || plan.parameters.len() + registry.len() * 2 > 1024
    {
        return Err(Error::Capacity("issuer registry".into()));
    }
    if plan.nodes.iter().any(|n|matches!(&n.operator,Operator::Closed(ClosedOperator::CredentialProfile{issuers,..}) if !issuers.is_empty())){return Err(Error::Invalid("issuer registry already bound".into()));}
    let mut entries = registry
        .iter()
        .map(|(iri, key)| Ok((iri.clone(), key_name(key)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    entries.sort_by_key(|(iri, key)| (iri.as_str().to_owned(), key.as_str().to_owned()));
    entries.dedup();
    let mut slots = vec![];
    for (iri, key) in entries {
        let a = plan.parameters.len() as u32;
        plan.parameters
            .push(crate::plan::Parameter::Term(iri.into()));
        let b = plan.parameters.len() as u32;
        plan.parameters
            .push(crate::plan::Parameter::Term(key.into()));
        slots.push((a, b));
    }
    let Some(op) = plan.nodes.iter_mut().find_map(|n| {
        if let Operator::Closed(ClosedOperator::CredentialProfile { issuers, .. }) = &mut n.operator
        {
            Some(issuers)
        } else {
            None
        }
    }) else {
        return Err(Error::Invalid("credential graph profile".into()));
    };
    *op = slots;
    Ok(plan)
}
pub fn key_name(key: &sparq_zk::sig::PublicKey) -> Result<oxrdf::NamedNode, Error> {
    if !key.is_prime_order() {
        return Err(Error::Invalid("issuer signing key".into()));
    }
    let point = (key.0.x, key.0.y);
    let hex = |n: &sparq_merkle_suite::Fr| {
        sparq_merkle_suite::poseidon::field_bytes(*n)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    Ok(oxrdf::NamedNode::new_unchecked(format!(
        "urn:sparq:issuer-key:{}{}",
        hex(&point.0),
        hex(&point.1)
    )))
}
pub(crate) fn check_issuer_registry(plan: &Plan, credentials: &[Credential]) -> Result<(), Error> {
    let messages = credentials
        .iter()
        .map(|c| {
            c.signed_message()
                .map(sparq_merkle_suite::poseidon::field_bytes)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    if messages.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(Error::Invalid("canonical credential order".into()));
    }
    if let Some(types) = plan.nodes.iter().find_map(|n| match n.operator {
        Operator::Closed(ClosedOperator::CredentialProfile { literal_types, .. }) => {
            Some(literal_types)
        }
        _ => None,
    }) {
        check_literal_types(types, &cells(credentials)?)?;
    }
    let literal_capacity = plan
        .nodes
        .iter()
        .find_map(|n| match n.operator {
            Operator::Closed(ClosedOperator::CredentialProfile {
                literal_capacity, ..
            }) => Some(literal_capacity),
            _ => None,
        })
        .ok_or_else(|| Error::Invalid("credential graph profile".into()))?;
    let registry = plan
        .nodes
        .iter()
        .find_map(|n| {
            if let Operator::Closed(ClosedOperator::CredentialProfile { issuers, .. }) = &n.operator
            {
                Some(issuers)
            } else {
                None
            }
        })
        .ok_or_else(|| Error::Invalid("credential graph profile".into()))?;
    if registry.is_empty() {
        return Err(Error::Unsupported(
            "credential graph issuer registry (lane E)".into(),
        ));
    }
    for c in credentials {
        let mut issuer = None;
        for q in c.quads()? {
            if let oxrdf::Term::Literal(l) = &q.object {
                if l.value().len() > literal_capacity as usize {
                    return Err(Error::Capacity("closed credential literal capacity".into()));
                }
            }
            if q.predicate.as_str() == ISSUER {
                let oxrdf::Term::NamedNode(name) = q.object else {
                    return Err(Error::Invalid("credential issuer IRI".into()));
                };
                if issuer.as_ref().is_some_and(|i| i != &name) {
                    return Err(Error::Invalid("credential issuer field".into()));
                }
                issuer = Some(name);
            }
        }
        let issuer = issuer.ok_or_else(|| Error::Invalid("credential issuer field".into()))?;
        let key = key_name(&c.issuer)?;
        if !registry.iter().any(|(a, b)| {
            plan.parameters[*a as usize] == crate::plan::Parameter::Term(issuer.clone().into())
                && plan.parameters[*b as usize] == crate::plan::Parameter::Term(key.clone().into())
        }) {
            return Err(Error::Invalid("verified issuer registry".into()));
        }
    }
    Ok(())
}
/// Preserve complete credential boundaries for the closed operator interpreter.
/// Supplied cells are an operator fixture; this entry point authenticates nothing.
pub fn evaluate_cells(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
) -> Result<crate::interp::Trace, Error> {
    evaluate_cells_for_request(plan, credentials, encoder, [0; 32])
}
/// Supplied-cell fixture with graph names derived from an explicit request.
pub fn evaluate_cells_for_request(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
    digest: [u8; 32],
) -> Result<crate::interp::Trace, Error> {
    let mut quads = vec![];
    let mut counts = vec![];
    for (i, c) in credentials.iter().enumerate() {
        counts.push(c.quads.len());
        let graph = crate::credential::credential_graph(digest, i as u32)?;
        for q in &c.quads {
            if q[3].bound {
                return Err(Error::Unsupported("credential named graph".into()));
            }
            if q[..3]
                .iter()
                .any(|c| c.kind == 2 && c.scope != i as u32 + 1)
            {
                return Err(Error::Invalid("credential blank-node scope".into()));
            }
            quads.push([q[0], q[1], q[2], graph]);
        }
    }
    crate::interp::evaluate_dataset(plan, &crate::interp::Dataset { quads, counts }, encoder)
}
/// Evaluate authenticated credentials under the verifier-owned graph policy.
pub fn evaluate(plan: &Plan, credentials: &[Credential]) -> Result<crate::interp::Trace, Error> {
    check_issuer_registry(plan, credentials)?;
    evaluate_for_request(plan, credentials, [0; 32])
}
/// Evaluate with presentation-local graph identities bound to this request.
pub fn evaluate_for_request(
    plan: &Plan,
    credentials: &[Credential],
    digest: [u8; 32],
) -> Result<crate::interp::Trace, Error> {
    check_issuer_registry(plan, credentials)?;
    let cells = cells(credentials)?;
    let mut quads = vec![];
    let mut counts = vec![];
    for (i, c) in cells.iter().enumerate() {
        let graph = crate::credential::credential_graph(digest, i as u32)?;
        counts.push(c.quads.len());
        for q in &c.quads {
            quads.push([q[0], q[1], q[2], graph]);
        }
    }
    crate::interp::evaluate_dataset(
        plan,
        &crate::interp::Dataset { quads, counts },
        &SuiteEncoder,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_issuer_registry_rejects_identity_mismatch() {
        let key = sparq_zk::sig::SecretKey::from_seed(7);
        let source =
            "<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n";
        let config =
            "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
        let credential = sparq_merkle_suite::poseidon::issue(
            source,
            config,
            &key,
            sparq_merkle_suite::Fr::from(19u64),
        )
        .unwrap();
        let c = Credential {
            credential,
            issuer: key.public_key(),
            issuer_index: 0,
            issuer_siblings: vec![sparq_merkle_suite::Fr::from(0u64); 4],
        };
        let p = lower(
            "ASK {}",
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 2,
            },
        )
        .unwrap();
        let p = bind_issuer_registry(
            p,
            &[(
                oxrdf::NamedNode::new_unchecked("urn:issuer"),
                sparq_zk::sig::SecretKey::from_seed(8).public_key(),
            )],
        )
        .unwrap();
        assert!(check_issuer_registry(&p, &[c])
            .unwrap_err()
            .to_string()
            .contains("verified issuer registry"));
    }
}

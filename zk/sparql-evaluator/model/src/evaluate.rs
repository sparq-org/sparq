// [GPT-6] Every step here executes INSIDE the proved guest, including parsing.
use crate::*;
use oxrdf::{Literal, NamedOrBlankNode, Term};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use spargebra::term::{GroundTerm, TermPattern};
use sparq_core::{Graph, dict::Dict};

/// Admits an entire query, including nested expressions and subqueries.
///
/// # Errors
/// Rejects external graphs/services, graph outputs, nondeterminism, custom
/// functions, unsupported RDF 1.2 terms, and resource exhaustion.
pub fn admit(request: &Request) -> Result<(), Rejected> {
    validate_request(request)?;
    let query = spargebra::SparqlParser::new()
        .parse_query(&request.query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(&query)
}

fn admit_query(query: &spargebra::Query) -> Result<(), Rejected> {
    if query.dataset().is_some() {
        return Err(Rejected("dataset clauses are not admitted"));
    }
    let pattern = match query {
        spargebra::Query::Select { pattern, .. } | spargebra::Query::Ask { pattern, .. } => pattern,
        _ => return Err(Rejected("only SELECT and ASK are admitted")),
    };
    enum Visit<'a> {
        Pattern(&'a GraphPattern),
        Expression(&'a Expression),
        Path(&'a PropertyPathExpression),
    }
    let mut pending = vec![Visit::Pattern(pattern)];
    let mut fuel = 0;
    while let Some(node) = pending.pop() {
        fuel += 1;
        if fuel > 1024 {
            return Err(Rejected("query AST capacity"));
        }
        match node {
            Visit::Pattern(p) => match p {
                GraphPattern::Bgp { patterns } => {
                    if patterns.len() > 64 {
                        return Err(Rejected("BGP capacity"));
                    }
                    for p in patterns {
                        pattern_term(&p.subject)?;
                        pattern_term(&p.object)?;
                    }
                }
                GraphPattern::Path {
                    subject,
                    path,
                    object,
                } => {
                    pattern_term(subject)?;
                    pattern_term(object)?;
                    pending.push(Visit::Path(path));
                }
                GraphPattern::Join { left, right }
                | GraphPattern::Union { left, right }
                | GraphPattern::Minus { left, right } => {
                    pending.extend([Visit::Pattern(left), Visit::Pattern(right)]);
                }
                GraphPattern::LeftJoin {
                    left,
                    right,
                    expression,
                } => {
                    pending.extend([Visit::Pattern(left), Visit::Pattern(right)]);
                    if let Some(e) = expression {
                        pending.push(Visit::Expression(e));
                    }
                }
                GraphPattern::Filter { expr, inner } => {
                    pending.extend([Visit::Pattern(inner), Visit::Expression(expr)]);
                }
                GraphPattern::Extend {
                    inner, expression, ..
                } => {
                    pending.extend([Visit::Pattern(inner), Visit::Expression(expression)]);
                }
                GraphPattern::Project { inner, variables } => {
                    if variables.len() > 64 {
                        return Err(Rejected("projection capacity"));
                    }
                    pending.push(Visit::Pattern(inner));
                }
                GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. } => pending.push(Visit::Pattern(inner)),
                GraphPattern::OrderBy { inner, expression } => {
                    pending.push(Visit::Pattern(inner));
                    for e in expression {
                        let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = e;
                        pending.push(Visit::Expression(e));
                    }
                }
                GraphPattern::Group {
                    inner, aggregates, ..
                } => {
                    pending.push(Visit::Pattern(inner));
                    for (_, a) in aggregates {
                        if let AggregateExpression::FunctionCall { name, expr, .. } = a {
                            if matches!(name, AggregateFunction::Custom(_)) {
                                return Err(Rejected("custom aggregate is not admitted"));
                            }
                            pending.push(Visit::Expression(expr));
                        }
                    }
                }
                GraphPattern::Values {
                    variables,
                    bindings,
                } => {
                    if variables.len() > 64 || bindings.len() > MAX_ROWS as usize {
                        return Err(Rejected("VALUES capacity"));
                    }
                    for row in bindings {
                        for cell in row.iter().flatten() {
                            match cell {
                                GroundTerm::NamedNode(_) => {}
                                GroundTerm::Literal(l) => literal(l)?,
                                _ => return Err(Rejected("triple terms are not admitted")),
                            }
                        }
                    }
                }
                GraphPattern::Graph { .. }
                | GraphPattern::Service { .. }
                | GraphPattern::Lateral { .. } => {
                    return Err(Rejected("GRAPH, SERVICE and LATERAL are not admitted"));
                }
            },
            Visit::Expression(e) => match e {
                Expression::Literal(l) => literal(l)?,
                Expression::NamedNode(_) | Expression::Variable(_) | Expression::Bound(_) => {}
                Expression::Or(a, b)
                | Expression::And(a, b)
                | Expression::Equal(a, b)
                | Expression::SameTerm(a, b)
                | Expression::Greater(a, b)
                | Expression::GreaterOrEqual(a, b)
                | Expression::Less(a, b)
                | Expression::LessOrEqual(a, b)
                | Expression::Add(a, b)
                | Expression::Subtract(a, b)
                | Expression::Multiply(a, b)
                | Expression::Divide(a, b) => {
                    pending.extend([Visit::Expression(a), Visit::Expression(b)]);
                }
                Expression::UnaryPlus(e) | Expression::UnaryMinus(e) | Expression::Not(e) => {
                    pending.push(Visit::Expression(e))
                }
                Expression::Exists(p) => pending.push(Visit::Pattern(p)),
                Expression::If(a, b, c) => pending.extend([
                    Visit::Expression(a),
                    Visit::Expression(b),
                    Visit::Expression(c),
                ]),
                Expression::In(e, args) => {
                    pending.push(Visit::Expression(e));
                    pending.extend(args.iter().map(Visit::Expression));
                }
                Expression::Coalesce(args) => pending.extend(args.iter().map(Visit::Expression)),
                Expression::FunctionCall(f, args) => {
                    match f {
                        Function::Now
                        | Function::Rand
                        | Function::Uuid
                        | Function::StrUuid
                        | Function::BNode
                        | Function::Triple
                        | Function::Subject
                        | Function::Predicate
                        | Function::Object
                        | Function::IsTriple
                        | Function::LangDir
                        | Function::HasLang
                        | Function::HasLangDir
                        | Function::StrLangDir => {
                            return Err(Rejected(
                                "nondeterministic or RDF 1.2 function is not admitted",
                            ));
                        }
                        Function::Custom(name)
                            if !matches!(
                                name.as_str(),
                                "http://www.w3.org/2001/XMLSchema#integer"
                                    | "http://www.w3.org/2001/XMLSchema#decimal"
                                    | "http://www.w3.org/2001/XMLSchema#float"
                                    | "http://www.w3.org/2001/XMLSchema#double"
                                    | "http://www.w3.org/2001/XMLSchema#string"
                                    | "http://www.w3.org/2001/XMLSchema#boolean"
                                    | "http://www.w3.org/2001/XMLSchema#dateTime"
                            ) =>
                        {
                            return Err(Rejected("custom function is not admitted"));
                        }
                        _ => {}
                    }
                    pending.extend(args.iter().map(Visit::Expression));
                }
            },
            Visit::Path(p) => match p {
                PropertyPathExpression::NamedNode(_)
                | PropertyPathExpression::NegatedPropertySet(_) => {}
                PropertyPathExpression::Reverse(p)
                | PropertyPathExpression::ZeroOrMore(p)
                | PropertyPathExpression::OneOrMore(p)
                | PropertyPathExpression::ZeroOrOne(p) => pending.push(Visit::Path(p)),
                PropertyPathExpression::Sequence(a, b)
                | PropertyPathExpression::Alternative(a, b) => {
                    pending.extend([Visit::Path(a), Visit::Path(b)])
                }
            },
        }
    }
    Ok(())
}

fn literal(l: &Literal) -> Result<(), Rejected> {
    if l.direction().is_some() {
        Err(Rejected("directional literals are not admitted"))
    } else {
        Ok(())
    }
}

fn pattern_term(term: &TermPattern) -> Result<(), Rejected> {
    match term {
        TermPattern::NamedNode(_) | TermPattern::Variable(_) => Ok(()),
        TermPattern::Literal(l) => literal(l),
        _ => Err(Rejected(
            "query blank nodes and triple terms are not admitted",
        )),
    }
}

fn term_string(term: &Term) -> Result<String, Rejected> {
    match term {
        Term::NamedNode(_) => {}
        Term::Literal(l) => literal(l)?,
        _ => return Err(Rejected("blank nodes and triple terms are not admitted")),
    }
    Ok(term.to_string())
}

fn ordered(mut p: &GraphPattern) -> bool {
    let mut projected = false;
    loop {
        p = match p {
            GraphPattern::OrderBy { .. } => return true,
            // A second projection crosses the subquery's ToMultiset boundary.
            // Its internal ordering does not order the outer SELECT result.
            GraphPattern::Project { inner, .. } if !projected => {
                projected = true;
                inner
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::Extend { inner, .. } => inner,
            _ => return false,
        };
    }
}

/// Executes the authenticated complete input and produces its public journal.
///
/// There is no host-result witness. The guest calls this exact function, rebuilding
/// dictionary/indexes from validated source triples after checking the anchor.
///
/// # Errors
/// Rejects mismatched anchors, unsupported queries/data, or evaluation/resource errors.
pub fn evaluate(witness: &Witness) -> Result<Journal, Rejected> {
    validate_request(&witness.request)?;
    let request = &witness.request;
    let commitment = dataset_commitment(&witness.dataset, &request.policy)?;
    let provenance = match request.authority {
        DatasetAuthority::VerifierAgreed {
            commitment: expected,
        } => {
            if commitment != expected {
                return Err(Rejected("complete dataset anchor mismatch"));
            }
            Provenance::VerifierAcceptedCommitment
        }
        DatasetAuthority::HolderDeclared => Provenance::HolderDeclaredOnly,
    };
    let prepared = sparq_engine::PreparedQuery::parse(&request.query)
        .map_err(|_| Rejected("SPARQL parse rejected"))?;
    admit_query(prepared.query())?;
    let mut dict = Dict::new();
    let mut triples = Vec::new();
    for triple in oxttl::NTriplesParser::new().for_slice(witness.dataset.ntriples.as_bytes()) {
        let triple = triple.map_err(|_| Rejected("N-Triples parse rejected"))?;
        if triples.len() >= request.policy.max_triples as usize {
            return Err(Rejected("source triple capacity"));
        }
        let NamedOrBlankNode::NamedNode(subject) = triple.subject else {
            return Err(Rejected("dataset blank nodes are not admitted"));
        };
        term_string(&triple.object)?;
        triples.push([
            dict.intern(&Term::NamedNode(subject)),
            dict.intern(&Term::NamedNode(triple.predicate)),
            dict.intern(&triple.object),
        ]);
    }
    let graph = Graph::from_parts(dict, triples);
    let budget = sparq_engine::QueryBudget {
        max_rows: Some(request.policy.max_rows as usize),
        // Bound computed terms as well as row counts; this is an estimate, not RSS.
        max_bytes: Some(4 * MAX_DATASET_BYTES as usize),
        ..Default::default()
    };
    let result = sparq_engine::query_prepared_with_budget(&graph, &prepared, &budget)
        .map_err(|_| Rejected("query evaluation or resource budget rejected"))?;
    if result.rows.len() > request.policy.max_rows as usize {
        return Err(Rejected("result row capacity"));
    }
    let result = match prepared.query() {
        spargebra::Query::Ask { .. } => CanonicalResult::Ask(!result.rows.is_empty()),
        spargebra::Query::Select { pattern, .. } => {
            let order = if ordered(pattern) {
                RowOrder::Sequence
            } else {
                RowOrder::Bag
            };
            let mut rows = result
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|t| t.as_ref().map(term_string).transpose())
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            if order == RowOrder::Bag {
                rows.sort();
            }
            CanonicalResult::Select {
                variables: result.vars.iter().map(|v| v.as_str().to_owned()).collect(),
                order,
                rows,
            }
        }
        _ => return Err(Rejected("query form rejected")),
    };
    Ok(Journal {
        version: VERSION,
        request_digest: request_digest(request)?,
        dataset_commitment: commitment,
        provenance,
        result,
    })
}

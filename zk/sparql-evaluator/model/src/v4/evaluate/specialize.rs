// [GPT-6] Bounded structural specialization; original request bytes remain unchanged.
use crate::Rejected;
use oxrdf::Literal;
use spargebra::{
    Query,
    algebra::{AggregateExpression, Expression, Function, GraphPattern, OrderExpression},
};

// Match the admitted AST budget; separately cap newly cloned literal bytes.
const MAX_NODES: usize = 1024;
const MAX_EXPANSION_BYTES: usize = 65_536;

pub(super) fn now(query: &mut Query, literal: &Literal) -> Result<(), Rejected> {
    let pattern = match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. } => pattern,
    };
    enum Visit<'a> {
        Pattern(&'a mut GraphPattern),
        Expression(&'a mut Expression),
    }
    let mut pending = vec![Visit::Pattern(pattern)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some(visit) = pending.pop() {
        nodes = nodes
            .checked_add(1)
            .ok_or(Rejected("V4 query AST capacity"))?;
        if nodes > MAX_NODES {
            return Err(Rejected("V4 query AST capacity"));
        }
        match visit {
            Visit::Pattern(pattern) => match pattern {
                GraphPattern::Bgp { .. }
                | GraphPattern::Path { .. }
                | GraphPattern::Values { .. } => {}
                GraphPattern::Join { left, right }
                | GraphPattern::Union { left, right }
                | GraphPattern::Minus { left, right } => {
                    pending.extend([Visit::Pattern(left), Visit::Pattern(right)])
                }
                GraphPattern::LeftJoin {
                    left,
                    right,
                    expression,
                } => {
                    pending.extend([Visit::Pattern(left), Visit::Pattern(right)]);
                    if let Some(expression) = expression {
                        pending.push(Visit::Expression(expression));
                    }
                }
                GraphPattern::Filter { expr, inner } => {
                    pending.extend([Visit::Pattern(inner), Visit::Expression(expr)])
                }
                GraphPattern::Extend {
                    inner, expression, ..
                } => pending.extend([Visit::Pattern(inner), Visit::Expression(expression)]),
                GraphPattern::Project { inner, .. }
                | GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. }
                | GraphPattern::Graph { inner, .. } => pending.push(Visit::Pattern(inner)),
                GraphPattern::OrderBy { inner, expression } => {
                    pending.push(Visit::Pattern(inner));
                    for order in expression {
                        let (OrderExpression::Asc(expression) | OrderExpression::Desc(expression)) =
                            order;
                        pending.push(Visit::Expression(expression));
                    }
                }
                GraphPattern::Group {
                    inner, aggregates, ..
                } => {
                    pending.push(Visit::Pattern(inner));
                    for (_, aggregate) in aggregates {
                        if let AggregateExpression::FunctionCall { expr, .. } = aggregate {
                            pending.push(Visit::Expression(expr));
                        }
                    }
                }
                GraphPattern::Service { .. } | GraphPattern::Lateral { .. } => {
                    return Err(Rejected("V4 SERVICE and LATERAL are not admitted"));
                }
            },
            Visit::Expression(expression) => match expression {
                Expression::FunctionCall(Function::Now, args) => {
                    if !args.is_empty() {
                        return Err(Rejected("V4 NOW requires no arguments"));
                    }
                    bytes = bytes
                        .checked_add(literal.value().len())
                        .ok_or(Rejected("V4 context expansion capacity"))?;
                    if bytes > MAX_EXPANSION_BYTES {
                        return Err(Rejected("V4 context expansion capacity"));
                    }
                    *expression = Expression::Literal(literal.clone());
                }
                Expression::NamedNode(_)
                | Expression::Literal(_)
                | Expression::Variable(_)
                | Expression::Bound(_) => {}
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
                    pending.extend([Visit::Expression(a), Visit::Expression(b)])
                }
                Expression::UnaryPlus(inner)
                | Expression::UnaryMinus(inner)
                | Expression::Not(inner) => pending.push(Visit::Expression(inner)),
                Expression::Exists(inner) => pending.push(Visit::Pattern(inner)),
                Expression::If(a, b, c) => pending.extend([
                    Visit::Expression(a),
                    Visit::Expression(b),
                    Visit::Expression(c),
                ]),
                Expression::In(inner, args) => {
                    pending.push(Visit::Expression(inner));
                    pending.extend(args.iter_mut().map(Visit::Expression));
                }
                Expression::Coalesce(args) | Expression::FunctionCall(_, args) => {
                    pending.extend(args.iter_mut().map(Visit::Expression))
                }
            },
        }
    }
    Ok(())
}

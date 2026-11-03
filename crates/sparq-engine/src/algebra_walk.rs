//! Shared short-circuiting traversal of graph patterns, expressions and aggregates.
//! Includes EXISTS subpatterns wherever an expression can occur.

use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};

pub(crate) trait Visitor {
    fn pattern(&mut self, _: &GraphPattern) -> bool {
        true
    }
    fn expression(&mut self, _: &Expression) -> bool {
        true
    }
    fn aggregate(&mut self, _: &AggregateExpression) -> bool {
        true
    }
}

pub(crate) fn all_pattern(p: &GraphPattern, visitor: &mut impl Visitor) -> bool {
    visitor.pattern(p)
        && match p {
            GraphPattern::Bgp { .. } | GraphPattern::Path { .. } | GraphPattern::Values { .. } => {
                true
            }
            GraphPattern::Service { inner, .. } => all_pattern(inner, visitor),
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => {
                all_pattern(left, visitor) && all_pattern(right, visitor)
            }
            // `Lateral` (SEP-0006) is unconditionally present — the workspace enables
            // spargebra's `sep-0006`, which is a spargebra feature, not one of ours.
            GraphPattern::Lateral { left, right } => {
                all_pattern(left, visitor) && all_pattern(right, visitor)
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                all_pattern(left, visitor)
                    && all_pattern(right, visitor)
                    && expression
                        .as_ref()
                        .map(|e| all_expr(e, visitor))
                        .unwrap_or(true)
            }
            GraphPattern::Filter { expr, inner } => {
                all_expr(expr, visitor) && all_pattern(inner, visitor)
            }
            GraphPattern::Graph { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Project { inner, .. }
            | GraphPattern::Slice { inner, .. } => all_pattern(inner, visitor),
            GraphPattern::Extend {
                inner, expression, ..
            } => all_pattern(inner, visitor) && all_expr(expression, visitor),
            GraphPattern::OrderBy { inner, expression } => {
                all_pattern(inner, visitor)
                    && expression.iter().all(|o| {
                        let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = o;
                        all_expr(e, visitor)
                    })
            }
            GraphPattern::Group {
                inner, aggregates, ..
            } => {
                all_pattern(inner, visitor)
                    && aggregates.iter().all(|(_, a)| all_aggregate(a, visitor))
            }
        }
}

fn all_aggregate(a: &AggregateExpression, visitor: &mut impl Visitor) -> bool {
    visitor.aggregate(a)
        && match a {
            AggregateExpression::CountSolutions { .. } => true,
            AggregateExpression::FunctionCall { expr, .. } => all_expr(expr, visitor),
        }
}

fn all_expr(e: &Expression, visitor: &mut impl Visitor) -> bool {
    visitor.expression(e)
        && match e {
            Expression::NamedNode(_)
            | Expression::Literal(_)
            | Expression::Variable(_)
            | Expression::Bound(_) => true,
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
            | Expression::Divide(a, b) => all_expr(a, visitor) && all_expr(b, visitor),
            Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
                all_expr(a, visitor)
            }
            Expression::In(a, list) => {
                all_expr(a, visitor) && list.iter().all(|e| all_expr(e, visitor))
            }
            Expression::If(a, b, c) => {
                all_expr(a, visitor) && all_expr(b, visitor) && all_expr(c, visitor)
            }
            Expression::Coalesce(list) => list.iter().all(|e| all_expr(e, visitor)),
            // EXISTS contains a graph pattern, including inside other expressions.
            Expression::Exists(inner) => all_pattern(inner, visitor),
            Expression::FunctionCall(_, args) => args.iter().all(|e| all_expr(e, visitor)),
        }
}

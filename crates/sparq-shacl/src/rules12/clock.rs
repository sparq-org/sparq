//! Freeze NOW in rule queries without changing the validation expression path.
use spargebra::{
    algebra::{AggregateExpression, Expression, Function, GraphPattern, OrderExpression},
    Query,
};

pub(super) fn needed(query: &Query) -> bool {
    let mut probe = query.clone();
    freeze(
        &mut probe,
        &oxrdf::Literal::new_simple_literal("clock probe"),
    );
    &probe != query
}
pub(super) fn freeze(query: &mut Query, now: &oxrdf::Literal) {
    if let Query::Construct { pattern, .. } = query {
        pattern_now(pattern, now);
    }
}
fn expr_now(expr: &mut Expression, now: &oxrdf::Literal) {
    use Expression::*;
    match expr {
        FunctionCall(Function::Now, _) => *expr = Literal(now.clone()),
        Or(a, b)
        | And(a, b)
        | Equal(a, b)
        | SameTerm(a, b)
        | Greater(a, b)
        | GreaterOrEqual(a, b)
        | Less(a, b)
        | LessOrEqual(a, b)
        | Add(a, b)
        | Subtract(a, b)
        | Multiply(a, b)
        | Divide(a, b) => {
            expr_now(a, now);
            expr_now(b, now);
        }
        In(a, args) => {
            expr_now(a, now);
            for a in args {
                expr_now(a, now);
            }
        }
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => expr_now(a, now),
        Exists(p) => pattern_now(p, now),
        If(a, b, c) => {
            expr_now(a, now);
            expr_now(b, now);
            expr_now(c, now);
        }
        Coalesce(args) | FunctionCall(_, args) => {
            for a in args {
                expr_now(a, now);
            }
        }
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => {}
    }
}
fn pattern_now(p: &mut GraphPattern, now: &oxrdf::Literal) {
    use GraphPattern::*;
    match p {
        Join { left, right }
        | Union { left, right }
        | Minus { left, right }
        | Lateral { left, right } => {
            pattern_now(left, now);
            pattern_now(right, now);
        }
        LeftJoin {
            left,
            right,
            expression,
        } => {
            pattern_now(left, now);
            pattern_now(right, now);
            if let Some(e) = expression {
                expr_now(e, now);
            }
        }
        Filter { inner, expr }
        | Extend {
            inner,
            expression: expr,
            ..
        } => {
            pattern_now(inner, now);
            expr_now(expr, now);
        }
        OrderBy { inner, expression } => {
            pattern_now(inner, now);
            for e in expression {
                let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = e;
                expr_now(e, now);
            }
        }
        Group {
            inner, aggregates, ..
        } => {
            pattern_now(inner, now);
            for (_, a) in aggregates {
                if let AggregateExpression::FunctionCall { expr, .. } = a {
                    expr_now(expr, now);
                }
            }
        }
        Graph { inner, .. }
        | Service { inner, .. }
        | Distinct { inner }
        | Reduced { inner }
        | Slice { inner, .. }
        | Project { inner, .. } => pattern_now(inner, now),
        Bgp { .. } | Path { .. } | Values { .. } => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freezes_nested_expressions_without_rewriting_literals() {
        let mut q=spargebra::SparqlParser::new().parse_query("CONSTRUCT {?s <urn:p> ?now} WHERE { {SELECT (MAX(NOW()) AS ?now) WHERE {FILTER EXISTS {BIND(NOW() AS ?n)}}} OPTIONAL {BIND(IF(true,NOW(),NOW()) AS ?s)} FILTER(NOW()=NOW()) BIND('NOW()' AS ?text) } ORDER BY NOW() LIMIT 1").unwrap();
        let now = oxrdf::Literal::new_typed_literal(
            "2001-01-01T00:00:00Z",
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime"),
        );
        freeze(&mut q, &now);
        let rendered = q.to_string();
        assert_eq!(
            rendered.matches("NOW()").count(),
            1,
            "Only the string literal remains"
        );
        assert!(rendered.matches("2001-01-01T00:00:00Z").count() >= 7);
    }
}

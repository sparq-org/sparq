//! Freeze NOW in rule queries without changing the validation expression path.
use spargebra::{
    algebra::{Expression, Function},
    Query,
};

#[cfg(feature = "shacl-af")]
pub(crate) fn needed(query: &Query) -> bool {
    let mut probe = query.clone();
    freeze(
        &mut probe,
        &oxrdf::Literal::new_simple_literal("clock probe"),
    );
    &probe != query
}
pub(crate) fn freeze(query: &mut Query, now: &oxrdf::Literal) {
    let pattern = match query {
        Query::Construct { pattern, .. }
        | Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. } => pattern,
    };
    crate::sparql::walk_pattern(pattern, &mut Freeze(now));
}
struct Freeze<'a>(&'a oxrdf::Literal);

impl crate::sparql::AlgebraVisitor for Freeze<'_> {
    fn expression(&mut self, expr: &mut Expression) {
        if matches!(expr, Expression::FunctionCall(Function::Now, _)) {
            *expr = Expression::Literal(self.0.clone());
        } else {
            crate::sparql::walk_expr(expr, self);
        }
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

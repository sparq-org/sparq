//! Constant-list membership over authenticated RDF terms.
use super::{canonical_integer, date, unsupported, NumericProfile, PlanError};
use oxrdf::Term;
use spargebra::algebra::{Expression, GraphPattern};

/// An IN obligation over an authenticated result variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermInFilter {
    /// Bound variable, without its question mark.
    pub variable: String,
    /// Admitted canonical constants, preserving their RDF datatypes.
    pub constants: Vec<Term>,
}

impl TermInFilter {
    pub(crate) fn accepts(&self, term: &Term) -> bool {
        self.constants.contains(term)
    }

    pub(super) fn validate(&self) -> Result<(), PlanError> {
        if self.constants.is_empty() || self.constants.len() > 16 {
            return Err(unsupported("IN constant list capacity"));
        }
        for term in &self.constants {
            let valid = match term {
                Term::NamedNode(_) => true,
                Term::Literal(l) => {
                    l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string"
                        || canonical_integer(term).is_some()
                        || date::canonical_date(term).is_some()
                }
                _ => false,
            };
            if !valid {
                return Err(unsupported(
                    "IN constant requires IRI, string, canonical unsigned integer or date",
                ));
            }
        }
        Ok(())
    }
}

fn constant(expr: &Expression) -> Result<Term, PlanError> {
    match expr {
        Expression::NamedNode(n) => Ok(n.clone().into()),
        Expression::Literal(l) => Ok(l.clone().into()),
        _ => Err(unsupported("IN list requires constant terms")),
    }
}

fn expression(
    expr: &Expression,
    numeric: NumericProfile,
    out: &mut Vec<TermInFilter>,
) -> Result<Option<Expression>, PlanError> {
    match expr {
        Expression::And(a, b) => {
            let a = expression(a, numeric, out)?;
            let b = expression(b, numeric, out)?;
            Ok(match (a, b) {
                (Some(a), Some(b)) => Some(Expression::And(Box::new(a), Box::new(b))),
                (a, b) => a.or(b),
            })
        }
        Expression::In(value, list) => {
            let Expression::Variable(variable) = value.as_ref() else {
                return Err(unsupported("IN requires a bound variable"));
            };
            if !matches!(numeric, NumericProfile::Unsigned) {
                return Err(unsupported("IN requires the unsigned typed contract"));
            }
            if list.is_empty() || list.len() > 16 {
                return Err(unsupported("IN constant list capacity"));
            }
            let filter = TermInFilter {
                variable: variable.as_str().to_owned(),
                constants: list.iter().map(constant).collect::<Result<_, _>>()?,
            };
            filter.validate()?;
            out.push(filter);
            Ok(None)
        }
        _ => Ok(Some(expr.clone())),
    }
}

pub(super) fn extract(
    pattern: &mut GraphPattern,
    numeric: NumericProfile,
) -> Result<Vec<TermInFilter>, PlanError> {
    let mut out = Vec::new();
    fn walk(
        p: &mut GraphPattern,
        numeric: NumericProfile,
        out: &mut Vec<TermInFilter>,
    ) -> Result<(), PlanError> {
        match p {
            GraphPattern::Filter { expr, inner } => {
                walk(inner, numeric, out)?;
                if let Some(remaining) = expression(expr, numeric, out)? {
                    *expr = remaining;
                } else {
                    *p = *inner.clone();
                }
            }
            GraphPattern::Join { left, right } => {
                walk(left, numeric, out)?;
                walk(right, numeric, out)?;
            }
            _ => {}
        }
        Ok(())
    }
    walk(pattern, numeric, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::planner::{lower_issuer_scope, DisclosureQuery};
    use oxrdf::{Literal, NamedNode};

    fn query(expr: &str) -> String {
        format!("SELECT DISTINCT ?value WHERE {{ ?s <urn:p> ?value . FILTER({expr}) }}")
    }

    #[test]
    fn typed_membership_preserves_terms_and_numeric_filters() {
        let parsed = DisclosureQuery::parse(&query("?value IN (\"BSc\", \"MSc\")")).unwrap();
        let filter = &parsed.in_filters[0];
        assert!(filter.accepts(&Literal::new_simple_literal("BSc").into()));
        assert!(filter.accepts(&Literal::new_simple_literal("MSc").into()));
        assert!(!filter.accepts(&Literal::new_simple_literal("PhD").into()));
        assert!(!filter.accepts(&NamedNode::new("urn:BSc").unwrap().into()));
        let parsed = DisclosureQuery::parse(&query("?value IN (18, 21) && ?value >= 21")).unwrap();
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.in_filters.len(), 1);
        for literal in [
            "\"2000-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>",
            "<urn:BSc>",
        ] {
            assert!(DisclosureQuery::parse(&query(&format!("?value IN ({literal})"))).is_ok());
        }
    }

    #[test]
    fn refuses_noncanonical_nonconjunctive_and_scope_escape() {
        for expr in [
            "?value IN ()",
            "?value IN (?s)",
            "?value IN (-1)",
            "?value IN (\"01\"^^<http://www.w3.org/2001/XMLSchema#integer>)",
            "?value IN (\"2000-01-01Z\"^^<http://www.w3.org/2001/XMLSchema#date>)",
            "?value IN (1.0)",
            "?value NOT IN (1)",
            "?value IN (1) || ?value = 2",
        ] {
            assert!(DisclosureQuery::parse(&query(expr)).is_err(), "{expr}");
        }
        assert!(DisclosureQuery::parse(
            &query("?value IN (1)").replace("DISTINCT ?value", "DISTINCT ?s")
        )
        .is_ok());
        let scoped = "PREFIX vcq: <https://w3id.org/sparq/vcq#> SELECT DISTINCT ?value WHERE { ?g vcq:verifiedIssuer <urn:issuer> . GRAPH ?g { ?s <urn:p> ?value . FILTER(?value IN (\"BSc\", \"MSc\")) } }".to_owned();
        let (flat, issuer) = lower_issuer_scope(&scoped).unwrap();
        assert_eq!(issuer.as_deref(), Some("urn:issuer"));
        assert_eq!(DisclosureQuery::parse(&flat).unwrap().in_filters.len(), 1);
        let escaped = scoped.replace("?s <urn:p> ?value .", "?s <urn:p> ?other .");
        assert!(lower_issuer_scope(&escaped).is_err());
    }
}

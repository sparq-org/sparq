//! Query-side date-key admission; committed RDF terms retain their lexical identity.
use super::{
    canonical_integer, unsupported, IntegerFilter, PlanError, PredicateDatatype,
    MAX_DISCLOSURE_AST_NODES, MAX_DISCLOSURE_FILTERS,
};
use crate::manifest::FilterOp;
use oxrdf::Term;
use spargebra::algebra::{Expression, GraphPattern};

const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

pub(super) fn canonical_date(term: &Term) -> Option<u64> {
    let Term::Literal(literal) = term else {
        return None;
    };
    if literal.datatype().as_str() != XSD_DATE {
        return None;
    }
    let b = literal.value().as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if ![0, 1, 2, 3, 5, 6, 8, 9]
        .iter()
        .all(|&i| b[i].is_ascii_digit())
    {
        return None;
    }
    let year: u64 = literal.value()[..4].parse().ok()?;
    let month: u64 = literal.value()[5..7].parse().ok()?;
    let day: u64 = literal.value()[8..].parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (day >= 1 && day <= days).then_some(year * 10000 + month * 100 + day)
}

/// Dedicated date members admit only a single hidden date comparison.
#[cfg(feature = "successful-results")]
pub(crate) fn hidden_date(query: &super::DisclosureQuery) -> Result<bool, PlanError> {
    let hidden: Vec<_> = query
        .filters
        .iter()
        .filter(|f| !query.projection.contains(&f.variable))
        .collect();
    let dates = hidden
        .iter()
        .filter(|f| f.datatype == PredicateDatatype::Date)
        .count();
    if dates != 0 && hidden.len() != 1 {
        return Err(unsupported(
            "hidden xsd:date requires a single date predicate without hidden integer predicates",
        ));
    }
    Ok(dates != 0)
}

pub(super) fn collect_filters(pattern: &GraphPattern) -> Result<Vec<IntegerFilter>, PlanError> {
    let mut patterns = vec![pattern];
    let mut filters = Vec::new();
    let mut nodes = 0;
    while let Some(pattern) = patterns.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("FILTER AST nodes"));
        }
        match pattern {
            GraphPattern::Bgp { .. } => {}
            GraphPattern::Join { left, right } => {
                patterns.push(right);
                patterns.push(left);
            }
            GraphPattern::Filter { expr, inner } => {
                collect_comparisons(expr, &mut filters)?;
                patterns.push(inner);
            }
            _ => return Err(unsupported("FILTER graph operator")),
        }
    }
    Ok(filters)
}

fn collect_comparisons(
    expr: &Expression,
    filters: &mut Vec<IntegerFilter>,
) -> Result<(), PlanError> {
    use Expression as E;
    let mut pending = vec![expr];
    let mut nodes = 0;
    while let Some(expr) = pending.pop() {
        nodes += 1;
        if nodes > MAX_DISCLOSURE_AST_NODES {
            return Err(PlanError::LimitExceeded("comparison AST nodes"));
        }
        let (op, left, right) = match expr {
            E::And(left, right) => {
                pending.push(right);
                pending.push(left);
                continue;
            }
            E::Less(a, b) => (FilterOp::Lt, a, b),
            E::LessOrEqual(a, b) => (FilterOp::Le, a, b),
            E::Greater(a, b) => (FilterOp::Gt, a, b),
            E::GreaterOrEqual(a, b) => (FilterOp::Ge, a, b),
            E::Equal(a, b) => (FilterOp::Eq, a, b),
            E::Not(inner) => match inner.as_ref() {
                E::Equal(a, b) => (FilterOp::Ne, a, b),
                _ => return Err(unsupported("FILTER negation")),
            },
            _ => return Err(unsupported("FILTER expression")),
        };
        let (variable, constant, op) = match (left.as_ref(), right.as_ref()) {
            (E::Variable(variable), constant) => (variable, constant, op),
            (constant, E::Variable(variable)) => {
                let flipped = match op {
                    FilterOp::Lt => FilterOp::Gt,
                    FilterOp::Le => FilterOp::Ge,
                    FilterOp::Gt => FilterOp::Lt,
                    FilterOp::Ge => FilterOp::Le,
                    FilterOp::Eq => FilterOp::Eq,
                    FilterOp::Ne => FilterOp::Ne,
                };
                (variable, constant, flipped)
            }
            _ => return Err(unsupported("FILTER requires variable and integer literal")),
        };
        let Expression::Literal(literal) = constant else {
            return Err(unsupported(
                "FILTER requires a canonical integer or xsd:date literal",
            ));
        };
        let term = Term::Literal(literal.clone());
        let (bound, datatype) = match literal.datatype().as_str() {
            XSD_DATE => (
                canonical_date(&term).ok_or_else(|| {
                    unsupported("xsd:date requires valid YYYY-MM-DD without timezone")
                })?,
                PredicateDatatype::Date,
            ),
            "http://www.w3.org/2001/XMLSchema#dateTime" => {
                return Err(unsupported("xsd:dateTime FILTER"))
            }
            _ => (
                canonical_integer(&term)
                    .ok_or_else(|| unsupported("canonical nonnegative integer FILTER bound"))?,
                PredicateDatatype::Integer,
            ),
        };
        if filters.len() >= MAX_DISCLOSURE_FILTERS {
            return Err(PlanError::LimitExceeded("admitted FILTER comparisons"));
        }
        filters.push(IntegerFilter {
            variable: variable.as_str().to_owned(),
            op,
            bound,
            datatype,
        });
    }
    Ok(())
}

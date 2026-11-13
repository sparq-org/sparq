//! Query-side date-key admission; committed RDF terms retain their lexical identity.
use super::{canonical_integer, unsupported, IntegerFilter, PlanError, PredicateDatatype};
use crate::manifest::FilterOp;
use oxrdf::Term;
use sparq_zk::verify::{fragment_typed_filters, FilterCmp};

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

pub(super) fn collect_filters(sparql: &str) -> Result<Vec<IntegerFilter>, PlanError> {
    fragment_typed_filters(sparql)
        .map_err(|e| unsupported(&e.to_string()))?
        .into_iter()
        .map(|filter| {
            let term = Term::Literal(filter.constant.clone());
            let (bound, datatype) = match filter.constant.datatype().as_str() {
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
            Ok(IntegerFilter {
                variable: filter.variable,
                op: match filter.op {
                    FilterCmp::Lt => FilterOp::Lt,
                    FilterCmp::Le => FilterOp::Le,
                    FilterCmp::Gt => FilterOp::Gt,
                    FilterCmp::Ge => FilterOp::Ge,
                    FilterCmp::Eq => FilterOp::Eq,
                    FilterCmp::Ne => FilterOp::Ne,
                },
                bound,
                datatype,
            })
        })
        .collect()
}

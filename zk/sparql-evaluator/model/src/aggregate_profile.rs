// [GPT-6] Conservative admission while aggregate error semantics remain unresolved.
use crate::Rejected;
use oxrdf::Variable;
use spargebra::algebra::{AggregateExpression, AggregateFunction, Expression, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern};
use std::collections::BTreeSet;

#[derive(Default)]
struct Domain {
    bound: BTreeSet<Variable>,
    empty: bool,
}

/// Require non-COUNT operands to be bound terms, never fallible expressions.
pub(crate) fn admit(
    inner: &GraphPattern,
    aggregates: &[(Variable, AggregateExpression)],
) -> Result<(), Rejected> {
    if aggregates.iter().all(|(_, a)| {
        matches!(
            a,
            AggregateExpression::CountSolutions { .. }
                | AggregateExpression::FunctionCall {
                    name: AggregateFunction::Count,
                    ..
                }
        )
    }) {
        return Ok(());
    }
    let domain = domain(inner, &mut 1024, 0)?;
    for (_, aggregate) in aggregates {
        let AggregateExpression::FunctionCall { name, expr, .. } = aggregate else {
            continue;
        };
        if matches!(name, AggregateFunction::Count) || domain.empty {
            continue;
        }
        let admitted = match expr {
            Expression::Variable(v) => domain.bound.contains(v),
            Expression::Literal(_) | Expression::NamedNode(_) => true,
            _ => false,
        };
        if !admitted {
            return Err(Rejected("aggregate operand may be unbound or erroneous"));
        }
    }
    Ok(())
}

fn domain(p: &GraphPattern, fuel: &mut usize, depth: usize) -> Result<Domain, Rejected> {
    if *fuel == 0 || depth > 128 {
        return Err(Rejected("aggregate domain capacity"));
    }
    *fuel -= 1;
    let mut result = match p {
        GraphPattern::Bgp { patterns } => {
            let mut d = Domain::default();
            for p in patterns {
                for t in [&p.subject, &p.object] {
                    if let TermPattern::Variable(v) = t {
                        d.bound.insert(v.clone());
                    }
                }
                if let NamedNodePattern::Variable(v) = &p.predicate {
                    d.bound.insert(v.clone());
                }
            }
            d
        }
        GraphPattern::Path {
            subject, object, ..
        } => {
            let mut d = Domain::default();
            for t in [subject, object] {
                if let TermPattern::Variable(v) = t {
                    d.bound.insert(v.clone());
                }
            }
            d
        }
        GraphPattern::Values {
            variables,
            bindings,
        } => Domain {
            bound: variables
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    bindings
                        .iter()
                        .all(|row| row.get(*i).is_some_and(Option::is_some))
                })
                .map(|(_, v)| v.clone())
                .collect(),
            empty: bindings.is_empty(),
        },
        GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
            let mut l = domain(left, fuel, depth + 1)?;
            let r = domain(right, fuel, depth + 1)?;
            if matches!(p, GraphPattern::Join { .. }) {
                l.bound.extend(r.bound);
                l.empty |= r.empty;
            } else {
                l.bound.retain(|v| r.bound.contains(v));
                l.empty &= r.empty;
            }
            l
        }
        GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
            domain(left, fuel, depth + 1)?
        }
        GraphPattern::Filter { inner, expr } => {
            let mut d = domain(inner, fuel, depth + 1)?;
            if matches!(expr, Expression::Literal(l)
                if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#boolean"
                    && matches!(l.value(), "false" | "0"))
            {
                d.empty = true;
            }
            d
        }
        GraphPattern::Project { inner, variables } => {
            let mut d = domain(inner, fuel, depth + 1)?;
            d.bound.retain(|v| variables.contains(v));
            d
        }
        GraphPattern::Graph { inner, name } => {
            let mut d = domain(inner, fuel, depth + 1)?;
            if let NamedNodePattern::Variable(v) = name {
                d.bound.insert(v.clone());
            }
            d
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            let mut d = domain(inner, fuel, depth + 1)?;
            if matches!(
                expression,
                Expression::Literal(_) | Expression::NamedNode(_)
            ) || matches!(expression, Expression::Variable(v) if d.bound.contains(v))
            {
                d.bound.insert(variable.clone());
            }
            d
        }
        GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => domain(inner, fuel, depth + 1)?,
        // Nested aggregate output and unknown operators need separate binding proofs.
        _ => Domain::default(),
    };
    if result.bound.len() > 64 {
        return Err(Rejected("aggregate variable capacity"));
    }
    if result.empty {
        result.bound.clear();
    }
    Ok(result)
}

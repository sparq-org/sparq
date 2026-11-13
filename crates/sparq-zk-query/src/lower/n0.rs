//! Initial expressions occupy a separate seam from lane D's expression compiler.

use super::Builder;
use crate::{plan::*, Error};
use spargebra::{
    algebra::{AggregateExpression, Expression, Function, GraphPattern, OrderExpression},
    term::{NamedNodePattern, TermPattern},
};

impl Builder {
    pub(super) fn slot(&mut self, term: &TermPattern) -> Result<Slot, Error> {
        Ok(match term {
            TermPattern::Variable(v) => Slot::Variable(self.variable(v.as_str())),
            TermPattern::NamedNode(n) => {
                Slot::Parameter(self.parameter(Parameter::Term(n.clone().into())))
            }
            TermPattern::Literal(n) => {
                Slot::Parameter(self.parameter(Parameter::Term(n.clone().into())))
            }
            TermPattern::BlankNode(_) => return Err(Error::Unsupported("query blank node".into())),
            TermPattern::Triple(_) => return Err(Error::Excluded("triple term".into())),
        })
    }
    pub(super) fn named_slot(&mut self, term: &NamedNodePattern) -> Slot {
        match term {
            NamedNodePattern::Variable(v) => Slot::Variable(self.variable(v.as_str())),
            NamedNodePattern::NamedNode(n) => {
                Slot::Parameter(self.parameter(Parameter::Term(n.clone().into())))
            }
        }
    }
    pub(super) fn expression(&mut self, e: &Expression) -> Result<Expr, Error> {
        use Expression as E;
        if let Some(term) = signed_constant(e) {
            return Ok(Expr::Term(Slot::Parameter(
                self.parameter(Parameter::Term(term.into())),
            )));
        }
        let boxed = |s: &mut Self, e: &E| s.expression(e).map(Box::new);
        Ok(match e {
            E::Variable(v) => Expr::Term(Slot::Variable(self.variable(v.as_str()))),
            E::NamedNode(n) => Expr::Term(Slot::Parameter(
                self.parameter(Parameter::Term(n.clone().into())),
            )),
            E::Literal(n) => Expr::Term(Slot::Parameter(
                self.parameter(Parameter::Term(n.clone().into())),
            )),
            E::Equal(a, b) => Expr::Compare(Compare::Equal, boxed(self, a)?, boxed(self, b)?),
            E::Less(a, b) => Expr::Compare(Compare::Less, boxed(self, a)?, boxed(self, b)?),
            E::LessOrEqual(a, b) => {
                Expr::Compare(Compare::LessOrEqual, boxed(self, a)?, boxed(self, b)?)
            }
            E::Greater(a, b) => Expr::Compare(Compare::Greater, boxed(self, a)?, boxed(self, b)?),
            E::GreaterOrEqual(a, b) => {
                Expr::Compare(Compare::GreaterOrEqual, boxed(self, a)?, boxed(self, b)?)
            }
            E::And(a, b) => Expr::And(boxed(self, a)?, boxed(self, b)?),
            E::Or(a, b) => Expr::Or(boxed(self, a)?, boxed(self, b)?),
            E::Not(a) => Expr::Not(boxed(self, a)?),
            E::In(a, bs) => {
                if bs.len() > IN_CAPACITY {
                    return Err(Error::Capacity("IN list".into()));
                }
                let constants = bs
                    .iter()
                    .map(|b| match b {
                        E::NamedNode(n) => Some(oxrdf::Term::NamedNode(n.clone())),
                        E::Literal(l) => Some(oxrdf::Term::Literal(l.clone())),
                        _ => signed_constant(b).map(oxrdf::Term::Literal),
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(constants) = constants {
                    let value = boxed(self, a)?;
                    let start = self.plan.parameters.len() as u32;
                    for i in 0..IN_CAPACITY {
                        let term = constants.get(i).cloned().unwrap_or_else(|| {
                            oxrdf::Literal::new_typed_literal(
                                "0",
                                oxrdf::NamedNode::new_unchecked(
                                    "http://www.w3.org/2001/XMLSchema#integer",
                                ),
                            )
                            .into()
                        });
                        self.parameter(Parameter::Term(term));
                    }
                    let count = self.parameter(Parameter::Index(constants.len() as u32));
                    Expr::InList {
                        value,
                        start,
                        count,
                    }
                } else {
                    Expr::In(
                        boxed(self, a)?,
                        bs.iter()
                            .map(|b| self.expression(b))
                            .collect::<Result<_, _>>()?,
                    )
                }
            }
            E::FunctionCall(f, _) => return Err(Error::Unsupported(f.to_string())),
            E::Exists(_) => return Err(Error::Unsupported("EXISTS / NOT EXISTS".into())),
            E::SameTerm(_, _) => return Err(Error::Unsupported("sameTerm".into())),
            E::Bound(_) => return Err(Error::Unsupported("BOUND".into())),
            E::If(_, _, _) => return Err(Error::Unsupported("IF".into())),
            E::Coalesce(_) => return Err(Error::Unsupported("COALESCE".into())),
            E::Add(_, _)
            | E::Subtract(_, _)
            | E::Multiply(_, _)
            | E::Divide(_, _)
            | E::UnaryPlus(_)
            | E::UnaryMinus(_) => {
                return Err(Error::Unsupported(format!("arithmetic expression {e}")))
            }
        })
    }
}

// The parser represents a signed numeric token as unary syntax. Keep that token
// in the public constant lane without admitting arithmetic on private values.
fn signed_constant(e: &Expression) -> Option<oxrdf::Literal> {
    use Expression as E;
    let (negative, child) = match e {
        E::UnaryPlus(e) => (false, e.as_ref()),
        E::UnaryMinus(e) => (true, e.as_ref()),
        _ => return None,
    };
    let literal = match child {
        E::Literal(l) => l.clone(),
        _ => signed_constant(child)?,
    };
    let local = literal
        .datatype()
        .as_str()
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")?;
    if !matches!(local, "integer" | "decimal" | "float" | "double") {
        return None;
    }
    let value = literal.value().trim_start_matches('+');
    let lexical = if negative {
        value
            .strip_prefix('-')
            .map(str::to_owned)
            .unwrap_or_else(|| format!("-{value}"))
    } else {
        value.to_owned()
    };
    Some(oxrdf::Literal::new_typed_literal(
        lexical,
        literal.datatype().into_owned(),
    ))
}

// Inspect excluded functions even when they sit below an operator admitted in a later stage.
fn check_expression(e: &Expression) -> Result<(), Error> {
    use Expression as E;
    match e {
        E::FunctionCall(f, args) => {
            if matches!(
                f,
                Function::Triple
                    | Function::Subject
                    | Function::Predicate
                    | Function::Object
                    | Function::IsTriple
            ) {
                return Err(Error::Excluded(f.to_string()));
            }
            if matches!(
                f,
                Function::Now | Function::Rand | Function::Uuid | Function::StrUuid
            ) || matches!(f, Function::BNode) && args.is_empty()
            {
                return Err(Error::Excluded(f.to_string()));
            }
            for arg in args {
                check_expression(arg)?;
            }
        }
        E::Or(a, b)
        | E::And(a, b)
        | E::Equal(a, b)
        | E::SameTerm(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Add(a, b)
        | E::Subtract(a, b)
        | E::Multiply(a, b)
        | E::Divide(a, b) => {
            check_expression(a)?;
            check_expression(b)?;
        }
        E::In(a, bs) => {
            check_expression(a)?;
            for b in bs {
                check_expression(b)?;
            }
        }
        E::If(a, b, c) => {
            check_expression(a)?;
            check_expression(b)?;
            check_expression(c)?;
        }
        E::Coalesce(args) => {
            for a in args {
                check_expression(a)?;
            }
        }
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => check_expression(a)?,
        E::Exists(g) => check_exclusions(g)?,
        E::Literal(literal) => check_term(&literal.clone().into())?,
        E::NamedNode(_) | E::Variable(_) | E::Bound(_) => {}
    }
    Ok(())
}

fn check_term(term: &oxrdf::Term) -> Result<(), Error> {
    match term {
        oxrdf::Term::Triple(_) => Err(Error::Excluded("triple term".into())),
        oxrdf::Term::Literal(literal) if literal.direction().is_some() => {
            Err(Error::Unsupported("directional literal".into()))
        }
        _ => Ok(()),
    }
}

fn check_pattern_term(term: &TermPattern) -> Result<(), Error> {
    match term {
        TermPattern::Triple(_) => Err(Error::Excluded("triple term".into())),
        TermPattern::Literal(literal) => check_term(&literal.clone().into()),
        _ => Ok(()),
    }
}

pub(super) fn check_exclusions(g: &GraphPattern) -> Result<(), Error> {
    use GraphPattern as G;
    match g {
        G::Service { .. } => return Err(Error::Excluded("SERVICE".into())),
        G::Join { left, right }
        | G::Union { left, right }
        | G::Minus { left, right }
        | G::Lateral { left, right } => {
            check_exclusions(left)?;
            check_exclusions(right)?;
        }
        G::LeftJoin {
            left,
            right,
            expression,
        } => {
            check_exclusions(left)?;
            check_exclusions(right)?;
            if let Some(e) = expression {
                check_expression(e)?;
            }
        }
        G::Filter { inner, expr } => {
            check_exclusions(inner)?;
            check_expression(expr)?;
        }
        G::Extend {
            inner, expression, ..
        } => {
            check_exclusions(inner)?;
            check_expression(expression)?;
        }
        G::OrderBy { inner, expression } => {
            check_exclusions(inner)?;
            for OrderExpression::Asc(e) | OrderExpression::Desc(e) in expression {
                check_expression(e)?;
            }
        }
        G::Group {
            inner, aggregates, ..
        } => {
            check_exclusions(inner)?;
            for (_, agg) in aggregates {
                if let AggregateExpression::FunctionCall { expr, .. } = agg {
                    check_expression(expr)?;
                }
            }
        }
        G::Graph { inner, .. }
        | G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => check_exclusions(inner)?,
        G::Bgp { patterns } => {
            for pattern in patterns {
                check_pattern_term(&pattern.subject)?;
                check_pattern_term(&pattern.object)?;
            }
        }
        G::Path {
            subject, object, ..
        } => {
            check_pattern_term(subject)?;
            check_pattern_term(object)?;
        }
        G::Values { bindings, .. } => {
            for term in bindings.iter().flatten().flatten() {
                check_term(&term.clone().into())?;
            }
        }
    }
    Ok(())
}

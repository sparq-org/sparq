//! Closed-world lowering; correlated scans implement substitution before evaluation.

use super::Builder;
use crate::{
    plan::*,
    plan_closed::{ClosedOperator as C, Predicate},
    Error,
};
use spargebra::algebra::{Expression, GraphPattern};

/// The core lowering has one dispatch hook; later closed families register here.
pub(super) fn handles(pattern: &GraphPattern, outer: bool) -> bool {
    use GraphPattern as G;
    match pattern {
        _ if super::closed_path::handles(pattern) => true,
        G::LeftJoin { .. }
        | G::Minus { .. }
        | G::Union { .. }
        | G::Extend { .. }
        | G::Graph { .. } => true,
        G::Filter { expr, .. } => has_exists(expr),
        G::Project { .. } => !outer,
        _ => false,
    }
}

pub(super) fn has_exists(e: &Expression) -> bool {
    use Expression as E;
    match e {
        E::Exists(_) => true,
        E::Not(a) | E::UnaryPlus(a) | E::UnaryMinus(a) => has_exists(a),
        E::And(a, b)
        | E::Or(a, b)
        | E::Equal(a, b)
        | E::SameTerm(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::Add(a, b)
        | E::Subtract(a, b)
        | E::Multiply(a, b)
        | E::Divide(a, b) => has_exists(a) || has_exists(b),
        E::In(a, bs) => has_exists(a) || bs.iter().any(has_exists),
        E::If(a, b, c) => has_exists(a) || has_exists(b) || has_exists(c),
        E::Coalesce(args) | E::FunctionCall(_, args) => args.iter().any(has_exists),
        _ => false,
    }
}
impl Builder {
    fn closed_push(&mut self, op: C, variables: Vec<Var>) -> Result<NodeId, Error> {
        self.push(Operator::Closed(op), variables)
    }
    fn merged_schema(&self, a: NodeId, b: NodeId) -> Vec<Var> {
        let mut vars = self.schema(a);
        vars.extend(self.schema(b));
        vars.sort_unstable();
        vars.dedup();
        vars
    }
    fn closed_predicate(
        &mut self,
        e: &Expression,
        input: NodeId,
        context: Option<NodeId>,
        graph: Option<u32>,
    ) -> Result<Predicate, Error> {
        use Expression as E;
        Ok(match e {
            E::Exists(body) => {
                let mut rights = vec![];
                for row in 0..self.plan.capacity.rows_per_table {
                    let vars = context
                        .map_or_else(|| self.schema(input), |p| self.merged_schema(input, p));
                    let capture = self.closed_push(
                        C::Capture {
                            input,
                            row,
                            parent: context,
                        },
                        vars,
                    )?;
                    rights.push(self.closed_in(body, Some(capture), graph)?);
                }
                Predicate::Exists(rights)
            }
            E::Not(a) if has_exists(a) => {
                Predicate::Not(Box::new(self.closed_predicate(a, input, context, graph)?))
            }
            E::And(a, b) if has_exists(e) => Predicate::And(
                Box::new(self.closed_predicate(a, input, context, graph)?),
                Box::new(self.closed_predicate(b, input, context, graph)?),
            ),
            E::Or(a, b) if has_exists(e) => Predicate::Or(
                Box::new(self.closed_predicate(a, input, context, graph)?),
                Box::new(self.closed_predicate(b, input, context, graph)?),
            ),
            _ if has_exists(e) => {
                return Err(Error::Unsupported("EXISTS in a value expression".into()))
            }
            _ => Predicate::Expression(self.expression(e)?),
        })
    }
    pub(super) fn closed_pattern(
        &mut self,
        pattern: &GraphPattern,
        context: Option<NodeId>,
    ) -> Result<NodeId, Error> {
        self.closed_in(pattern, context, None)
    }
    fn closed_in(
        &mut self,
        pattern: &GraphPattern,
        context: Option<NodeId>,
        graph: Option<u32>,
    ) -> Result<NodeId, Error> {
        use GraphPattern as G;
        match pattern {
            _ if super::closed_path::handles(pattern) => self.closed_path(pattern, context, graph),
            G::LeftJoin {
                left,
                right,
                expression,
            } => {
                let a = self.closed_in(left, context, graph)?;
                let b = self.closed_in(right, context, graph)?;
                let condition = expression
                    .as_ref()
                    .map(|e| self.expression(e))
                    .transpose()?;
                self.closed_push(
                    C::LeftJoin {
                        left: a,
                        right: b,
                        condition,
                        context,
                    },
                    self.merged_schema(a, b),
                )
            }
            G::Minus { left, right } | G::Union { left, right } => {
                let a = self.closed_in(left, context, graph)?;
                let b = self.closed_in(right, context, graph)?;
                if matches!(pattern, G::Minus { .. }) {
                    self.closed_push(C::Minus { left: a, right: b }, self.schema(a))
                } else {
                    self.closed_push(C::Union { left: a, right: b }, self.merged_schema(a, b))
                }
            }
            G::Extend {
                inner,
                variable,
                expression,
            } => {
                let input = self.closed_in(inner, context, graph)?;
                let var = self.variable(variable.as_str());
                if self.schema(input).contains(&var) {
                    return Err(Error::Unsupported("BIND target already in scope".into()));
                }
                if context.is_some_and(|c| self.schema(c).contains(&var)) {
                    return Err(Error::Unsupported(
                        "EXISTS substitution into BIND target".into(),
                    ));
                }
                let value = match self.expression(expression)? {
                    Expr::Term(s) => s,
                    _ => {
                        return Err(Error::Unsupported(
                            "BIND value expression (expression lane)".into(),
                        ))
                    }
                };
                let mut vars = self.schema(input);
                vars.push(var);
                vars.sort_unstable();
                vars.dedup();
                self.closed_push(
                    C::Extend {
                        input,
                        variable: var,
                        value,
                        context,
                    },
                    vars,
                )
            }
            G::Filter { inner, expr } => {
                let input = self.closed_in(inner, context, graph)?;
                let predicate = self.closed_predicate(expr, input, context, graph)?;
                self.closed_push(
                    C::Filter {
                        input,
                        predicate,
                        context,
                    },
                    self.schema(input),
                )
            }
            G::Project { inner, variables } => {
                let input = self.closed_in(inner, context, graph)?;
                let vars = variables
                    .iter()
                    .map(|v| self.variable(v.as_str()))
                    .collect();
                self.push(Operator::Project(input), vars)
            }
            G::Graph { name, inner } => {
                let selector = self.named_slot(name);
                let context = match context {
                    Some(c) => c,
                    None => {
                        let unit = self.push(Operator::Unit, vec![])?;
                        self.closed_push(
                            C::Capture {
                                input: unit,
                                row: 0,
                                parent: None,
                            },
                            vec![],
                        )?
                    }
                };
                let mut output = None;
                for credential in 0..self.plan.capacity.credentials {
                    let name = self.parameter(Parameter::Term(
                        oxrdf::NamedNode::new_unchecked(format!(
                            "urn:sparq:credential:{credential}"
                        ))
                        .into(),
                    ));
                    let seed = self.closed_push(
                        C::GraphSeed {
                            context,
                            credential,
                            name,
                            selector: selector.clone(),
                        },
                        self.schema(context),
                    )?;
                    let input = self.closed_in(inner, Some(seed), Some(credential))?;
                    let mut vars = self.schema(input);
                    if let Slot::Variable(v) = selector {
                        vars.push(v);
                        vars.sort_unstable();
                        vars.dedup();
                    }
                    let branch = self.closed_push(
                        C::Graph {
                            input,
                            context: seed,
                            name,
                            selector: selector.clone(),
                        },
                        vars,
                    )?;
                    output = Some(match output {
                        None => branch,
                        Some(left) => self.closed_push(
                            C::Union {
                                left,
                                right: branch,
                            },
                            self.merged_schema(left, branch),
                        )?,
                    });
                }
                Ok(output.expect("credential capacity"))
            }
            G::Bgp { patterns } if context.is_some() => {
                let c = context.expect("substitution context");
                let mut input = None;
                for p in patterns {
                    let slots = [
                        self.slot(&p.subject)?,
                        self.named_slot(&p.predicate),
                        self.slot(&p.object)?,
                    ];
                    let mut vars: Vec<_> = slots
                        .iter()
                        .filter_map(|s| {
                            if let Slot::Variable(v) = s {
                                Some(*v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    vars.sort_unstable();
                    vars.dedup();
                    let scan = self.closed_push(
                        C::Scan {
                            pattern: slots,
                            context: c,
                            credential: graph,
                        },
                        vars,
                    )?;
                    input = Some(match input {
                        None => scan,
                        Some(a) => self.join(a, scan)?,
                    });
                }
                match input {
                    Some(id) => Ok(id),
                    None => self.closed_push(C::Unit { context: c }, vec![]),
                }
            }
            G::Join { left, right } if context.is_some() => {
                let a = self.closed_in(left, context, graph)?;
                let b = self.closed_in(right, context, graph)?;
                self.join(a, b)
            }
            G::Distinct { inner } if context.is_some() => {
                let a = self.closed_in(inner, context, graph)?;
                self.push(Operator::Distinct(a), self.schema(a))
            }
            G::Values { .. } if context.is_some() => {
                let input = self.pattern(pattern, true)?;
                self.closed_push(
                    C::Substitute {
                        input,
                        context: context.expect("substitution"),
                    },
                    self.schema(input),
                )
            }
            G::Slice {
                inner,
                start,
                length,
            } if context.is_some() => {
                let input = self.closed_in(inner, context, graph)?;
                let index =
                    |n: usize| u32::try_from(n).map_err(|_| Error::Capacity("slice index".into()));
                let offset = self.parameter(Parameter::Index(index(*start)?));
                let limit = length
                    .map(|n| index(n).map(|n| self.parameter(Parameter::Index(n))))
                    .transpose()?;
                self.push(
                    Operator::Slice {
                        input,
                        offset,
                        limit,
                    },
                    self.schema(input),
                )
            }
            _ => self.pattern(pattern, true),
        }
    }
}

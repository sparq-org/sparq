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
        G::Join { .. }
        | G::LeftJoin { .. }
        | G::Minus { .. }
        | G::Union { .. }
        | G::Extend { .. }
        | G::Graph { .. }
        | G::Group { .. } => true,
        G::Filter { expr, .. } => has_exists(expr) || super::closed_arith::has_arithmetic(expr),
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
    /// The body can be evaluated once, without substitution, when every
    /// variable it shares with the outer row is bound on both sides: it is a
    /// join of triple patterns and GRAPH blocks whose FILTERs read only
    /// variables those patterns bind, and the shared variables are certainly
    /// bound in the outer input. Then substitution equals a compatible-row test.
    fn uncorrelated(&self, body: &GraphPattern, input: NodeId) -> bool {
        let mut bound = vec![];
        if !uncorrelated_shape(body, &mut bound) {
            return false;
        }
        let schema = self.schema(input);
        let certain = self.certain(input);
        let shared: Vec<Var> = bound
            .iter()
            .filter_map(|name| self.plan.variable_names.iter().position(|v| v == name))
            .map(|i| i as Var)
            .filter(|v| schema.contains(v))
            .collect();
        !shared.is_empty() && shared.iter().all(|v| certain.contains(v))
    }
    /// Variables bound in every row of `id` (conservative: unknown forms bind none).
    fn certain(&self, id: NodeId) -> Vec<Var> {
        let node = &self.plan.nodes[id as usize];
        let mut vars = match &node.operator {
            Operator::Scan(_) | Operator::Closed(C::GraphScan { .. } | C::DatasetScan { .. }) => {
                node.output.variables.clone()
            }
            Operator::Join(a, b) | Operator::Closed(C::Join { left: a, right: b }) => {
                let mut v = self.certain(*a);
                v.extend(self.certain(*b));
                v
            }
            Operator::Filter(a, _)
            | Operator::Project(a)
            | Operator::Distinct(a)
            | Operator::Closed(
                C::Filter { input: a, .. }
                | C::Extend { input: a, .. }
                | C::BindBoolean { input: a, .. },
            ) => self.certain(*a),
            _ => vec![],
        };
        vars.retain(|v| node.output.variables.contains(v));
        vars
    }
    pub(super) fn closed_predicate(
        &mut self,
        e: &Expression,
        input: NodeId,
        context: Option<NodeId>,
        graph: Option<u32>,
    ) -> Result<Predicate, Error> {
        use Expression as E;
        Ok(match e {
            E::Exists(body) => {
                if context.is_none() && graph.is_none() && self.uncorrelated(body, input) {
                    return Ok(Predicate::SemiJoin(self.closed_in(body, None, None)?));
                }
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
            E::Not(a) if has_exists(a) || super::closed_arith::has_arithmetic(a) => {
                Predicate::Not(Box::new(self.closed_predicate(a, input, context, graph)?))
            }
            E::And(a, b) if has_exists(e) || super::closed_arith::has_arithmetic(e) => {
                Predicate::And(
                    Box::new(self.closed_predicate(a, input, context, graph)?),
                    Box::new(self.closed_predicate(b, input, context, graph)?),
                )
            }
            E::Or(a, b) if has_exists(e) || super::closed_arith::has_arithmetic(e) => {
                Predicate::Or(
                    Box::new(self.closed_predicate(a, input, context, graph)?),
                    Box::new(self.closed_predicate(b, input, context, graph)?),
                )
            }
            _ if has_exists(e) => {
                return Err(Error::Unsupported("EXISTS in a value expression".into()))
            }
            _ if super::closed_arith::has_arithmetic(e) => {
                Predicate::Arithmetic(self.closed_arithmetic(e)?)
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
    pub(super) fn closed_in(
        &mut self,
        pattern: &GraphPattern,
        context: Option<NodeId>,
        graph: Option<u32>,
    ) -> Result<NodeId, Error> {
        use GraphPattern as G;
        match pattern {
            G::Group { .. } if context.is_some() => Err(Error::Unsupported(
                "correlated GROUP BY / aggregate substitution".into(),
            )),
            G::Group {
                inner,
                variables,
                aggregates,
            } => {
                let input = self.closed_in(inner, context, graph)?;
                self.closed_group(input, variables, aggregates)
            }
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
                let mut vars = self.schema(input);
                vars.push(var);
                vars.sort_unstable();
                vars.dedup();
                if matches!(
                    expression,
                    Expression::Variable(_) | Expression::NamedNode(_) | Expression::Literal(_)
                ) {
                    let Expr::Term(value) = self.expression(expression)? else {
                        unreachable!("term expression")
                    };
                    return self.closed_push(
                        C::Extend {
                            input,
                            variable: var,
                            value,
                            context,
                        },
                        vars,
                    );
                }
                let predicate = self.closed_predicate(expression, input, context, graph)?;
                self.closed_push(
                    C::BindBoolean {
                        input,
                        variable: var,
                        predicate,
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
            G::Graph { name, inner } if super::closed_graph::supported(inner) => {
                let selector = self.named_slot(name);
                self.closed_graph_body(inner, context, &selector)
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
                    let name = credential;
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
            G::Join { .. } => self.closed_join(pattern, context, graph),
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
/// Triple patterns, joins, GRAPH blocks and FILTERs over pattern-bound variables.
fn uncorrelated_shape(p: &GraphPattern, bound: &mut Vec<String>) -> bool {
    use spargebra::term::{NamedNodePattern as N, TermPattern as T};
    use GraphPattern as G;
    let term = |t: &T, bound: &mut Vec<String>| match t {
        T::Variable(v) => {
            bound.push(v.as_str().into());
            true
        }
        T::NamedNode(_) | T::Literal(_) => true,
        _ => false,
    };
    match p {
        G::Bgp { patterns } => patterns.iter().all(|t| {
            if let N::Variable(v) = &t.predicate {
                bound.push(v.as_str().into());
            }
            term(&t.subject, bound) && term(&t.object, bound)
        }),
        G::Join { left, right } => {
            uncorrelated_shape(left, bound) && uncorrelated_shape(right, bound)
        }
        G::Graph { name, inner } => {
            if let N::Variable(v) = name {
                bound.push(v.as_str().into());
            }
            uncorrelated_shape(inner, bound)
        }
        G::Filter { expr, inner } => {
            let mut used = vec![];
            uncorrelated_shape(inner, bound)
                && expression_variables(expr, &mut used)
                && used.iter().all(|v| bound.contains(v))
        }
        _ => false,
    }
}
/// Collect the variables an expression reads; false for forms that see outside it.
fn expression_variables(e: &Expression, out: &mut Vec<String>) -> bool {
    use Expression as E;
    match e {
        E::Variable(v) => {
            out.push(v.as_str().into());
            true
        }
        E::NamedNode(_) | E::Literal(_) => true,
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
        | E::Divide(a, b) => expression_variables(a, out) && expression_variables(b, out),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expression_variables(a, out),
        E::In(a, bs) => {
            expression_variables(a, out) && bs.iter().all(|b| expression_variables(b, out))
        }
        E::If(a, b, c) => {
            expression_variables(a, out)
                && expression_variables(b, out)
                && expression_variables(c, out)
        }
        E::Coalesce(args) | E::FunctionCall(_, args) => {
            args.iter().all(|a| expression_variables(a, out))
        }
        _ => false,
    }
}

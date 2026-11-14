//! Admission from the workspace SPARQL parser. Refusals always name the construct.

pub mod expr;
#[cfg(test)]
mod expr_functions_semantics;
#[cfg(test)]
mod expr_functions_tests;
#[cfg(test)]
mod expr_live_targets;
pub mod expr_plan;
#[cfg(test)]
mod expr_semantics;
#[cfg(test)]
mod expr_target_tests;
#[cfg(test)]
mod expr_test_support;
#[cfg(test)]
mod expr_tests;
mod n0;

use crate::{capacity::Bounds, plan::*, Error};
use spargebra::{algebra::GraphPattern, SparqlParser};

/// Lower a self-contained query using only public request bounds.
pub fn lower(query: &str, bounds: Bounds) -> Result<Plan, Error> {
    let capacity = bounds.capacity()?;
    let parsed = SparqlParser::new().parse_query(query).map_err(|e| {
        if SparqlParser::new().parse_update(query).is_ok() {
            Error::Excluded("SPARQL Update".into())
        } else {
            Error::Parse(e.to_string())
        }
    })?;
    if let Some(dataset) = parsed.dataset() {
        return Err(Error::Excluded(if dataset.default.is_empty() {
            "FROM NAMED".into()
        } else {
            "FROM".into()
        }));
    }
    let (pattern, form) = match parsed {
        spargebra::Query::Select { pattern, .. } => (pattern, ResultForm::Select),
        spargebra::Query::Ask { pattern, .. } => (pattern, ResultForm::Ask),
        spargebra::Query::Construct { .. } => return Err(Error::Unsupported("CONSTRUCT".into())),
        spargebra::Query::Describe { .. } => return Err(Error::Excluded("DESCRIBE".into())),
    };
    n0::check_exclusions(&pattern)?;
    let mut builder = Builder {
        plan: Plan {
            bounds,
            capacity,
            opening_profile: crate::capacity::OpeningProfile::default(),
            credential_quad_bounds: vec![],
            nodes: vec![],
            parameters: vec![],
            variable_names: vec![],
            result: 0,
            form,
        },
    };
    builder.plan.result = builder.pattern(&pattern, true)?;
    Ok(builder.plan)
}

struct Builder {
    plan: Plan,
}
impl Builder {
    fn variable(&mut self, name: &str) -> Var {
        if let Some(i) = self.plan.variable_names.iter().position(|v| v == name) {
            i as Var
        } else {
            let id = self.plan.variable_names.len() as Var;
            self.plan.variable_names.push(name.into());
            id
        }
    }
    fn parameter(&mut self, parameter: Parameter) -> ParameterId {
        let id = self.plan.parameters.len() as ParameterId;
        self.plan.parameters.push(parameter);
        id
    }
    fn push(&mut self, operator: Operator, variables: Vec<Var>) -> Result<NodeId, Error> {
        if self.plan.nodes.len() >= 256 || variables.len() > 16 || self.plan.parameters.len() > 1024
        {
            return Err(Error::Capacity("plan structure".into()));
        }
        let id = self.plan.nodes.len() as NodeId;
        self.plan.nodes.push(Node {
            operator,
            output: Schema {
                variables,
                capacity: self.plan.capacity.rows_per_table,
                order: CanonicalOrder::TermEncoding,
            },
        });
        Ok(id)
    }
    fn schema(&self, node: NodeId) -> Vec<Var> {
        self.plan.nodes[node as usize].output.variables.clone()
    }
    fn join(&mut self, a: NodeId, b: NodeId) -> Result<NodeId, Error> {
        let mut variables = self.schema(a);
        variables.extend(self.schema(b));
        variables.sort_unstable();
        variables.dedup();
        self.push(Operator::Join(a, b), variables)
    }
    fn pattern(&mut self, pattern: &GraphPattern, outer: bool) -> Result<NodeId, Error> {
        use GraphPattern as G;
        match pattern {
            G::Bgp { patterns } => {
                let mut input = None;
                for pattern in patterns {
                    let slots = [
                        self.slot(&pattern.subject)?,
                        self.named_slot(&pattern.predicate),
                        self.slot(&pattern.object)?,
                    ];
                    let mut variables: Vec<_> = slots
                        .iter()
                        .filter_map(|s| match s {
                            Slot::Variable(v) => Some(*v),
                            _ => None,
                        })
                        .collect();
                    variables.sort_unstable();
                    variables.dedup();
                    let scan = self.push(Operator::Scan(slots), variables)?;
                    input = Some(match input {
                        None => scan,
                        Some(a) => self.join(a, scan)?,
                    });
                }
                match input {
                    Some(id) => Ok(id),
                    None => self.push(Operator::Unit, vec![]),
                }
            }
            G::Join { left, right } => {
                let a = self.pattern(left, false)?;
                let b = self.pattern(right, false)?;
                self.join(a, b)
            }
            G::Filter { expr, inner } => {
                let input = self.pattern(inner, false)?;
                let expression = self.expression(expr)?;
                self.push(Operator::Filter(input, expression), self.schema(input))
            }
            G::Project { inner, variables } => {
                if !outer {
                    return Err(Error::Unsupported("subquery".into()));
                }
                let input = self.pattern(inner, false)?;
                let schema = variables
                    .iter()
                    .map(|v| self.variable(v.as_str()))
                    .collect();
                self.push(Operator::Project(input), schema)
            }
            G::Distinct { inner } => {
                let input = self.pattern(inner, outer)?;
                self.push(Operator::Distinct(input), self.schema(input))
            }
            G::Slice {
                inner,
                start,
                length,
            } => {
                let input = self.pattern(inner, outer)?;
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
            G::Values {
                variables,
                bindings,
            } => {
                if bindings.len() > self.plan.bounds.rows_per_table as usize {
                    return Err(Error::Capacity("VALUES table".into()));
                }
                let schema = variables
                    .iter()
                    .map(|v| self.variable(v.as_str()))
                    .collect();
                let rows = bindings
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|v| v.clone().map(|t| self.parameter(Parameter::Term(t.into()))))
                            .collect()
                    })
                    .collect();
                self.push(Operator::Values(rows), schema)
            }
            G::Service { .. } => Err(Error::Excluded("SERVICE".into())),
            other => Err(Error::Unsupported(
                match other {
                    G::Path { .. } => "property path",
                    G::LeftJoin { .. } => "OPTIONAL",
                    G::Union { .. } => "UNION",
                    G::Graph { .. } => "GRAPH",
                    G::Extend { .. } => "BIND / SELECT expression",
                    G::Minus { .. } => "MINUS",
                    G::OrderBy { .. } => "ORDER BY",
                    G::Reduced { .. } => "REDUCED",
                    G::Group { .. } => "GROUP BY / aggregate / HAVING",
                    G::Lateral { .. } => "LATERAL",
                    _ => unreachable!("handled operator"),
                }
                .into(),
            )),
        }
    }
}

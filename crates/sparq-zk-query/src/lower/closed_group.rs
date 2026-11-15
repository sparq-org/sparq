//! Group lowering stays in the closed-operator extension.
use super::Builder;
use crate::{
    interp_closed_group::AggregateKind as K,
    plan::*,
    plan_closed::{ClosedOperator, GroupAggregate},
    Error,
};
use spargebra::{
    algebra::{AggregateExpression, AggregateFunction, Expression},
    term::Variable,
};
impl Builder {
    pub(super) fn closed_group(
        &mut self,
        input: NodeId,
        variables: &[Variable],
        aggregates: &[(Variable, AggregateExpression)],
    ) -> Result<NodeId, Error> {
        let keys: Vec<_> = variables
            .iter()
            .map(|v| self.variable(v.as_str()))
            .collect();
        if keys.iter().any(|v| !self.schema(input).contains(v)) {
            return Err(Error::Unsupported(
                "GROUP BY variable outside input schema".into(),
            ));
        }
        let mut output = keys.clone();
        let mut specs = vec![];
        for (v, agg) in aggregates {
            let (kind, operand, distinct) = match agg {
                AggregateExpression::CountSolutions { distinct } => (K::CountAll, None, *distinct),
                AggregateExpression::FunctionCall {
                    name,
                    expr,
                    distinct,
                } => {
                    let kind = match name {
                        AggregateFunction::Count => K::Count,
                        AggregateFunction::Sum => K::Sum,
                        AggregateFunction::Min => K::Min,
                        AggregateFunction::Max => K::Max,
                        AggregateFunction::Avg => K::Avg,
                        AggregateFunction::Sample => K::Sample,
                        AggregateFunction::GroupConcat { .. } => {
                            return Err(Error::Unsupported(
                                "GROUP_CONCAT lexical opening required".into(),
                            ))
                        }
                        _ => return Err(Error::Unsupported(format!("aggregate {name}"))),
                    };
                    let Expression::Variable(arg) = expr else {
                        return Err(Error::Unsupported(
                            "aggregate expression operand (lane D)".into(),
                        ));
                    };
                    (kind, Some(self.variable(arg.as_str())), *distinct)
                }
            };
            if operand.is_some_and(|v| !self.schema(input).contains(&v)) {
                return Err(Error::Unsupported(
                    "aggregate operand outside input schema".into(),
                ));
            }
            let variable = self.variable(v.as_str());
            output.push(variable);
            specs.push(GroupAggregate {
                variable,
                kind,
                operand,
                distinct,
            });
        }
        if output.is_empty() {
            return Err(Error::Unsupported("GROUP BY without output columns".into()));
        }
        let id = self.push(
            Operator::Closed(ClosedOperator::Group {
                input,
                keys,
                aggregates: specs,
            }),
            output,
        )?;
        Ok(id)
    }
}

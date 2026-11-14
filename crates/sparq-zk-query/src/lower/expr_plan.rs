//! Mechanical bridge for lane B's initial plan expression shapes.
//! Replace these shape aliases with crate::plan imports when the lanes are combined.
use super::expr::{self, Expr, ExprOp, LaneType, Refusal, TypedDag};
use oxrdf::Term;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Slot {
    Variable(u32),
    Parameter(u32),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compare {
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanExpr {
    Term(Slot),
    Compare(Compare, Box<PlanExpr>, Box<PlanExpr>),
    And(Box<PlanExpr>, Box<PlanExpr>),
    Or(Box<PlanExpr>, Box<PlanExpr>),
    Not(Box<PlanExpr>),
    In(Box<PlanExpr>, Vec<PlanExpr>),
    /// Required additions for arithmetic and correlated-subquery results.
    Call(ExprOp, Vec<PlanExpr>),
    BooleanInput(u32),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Parameter {
    Term(Term),
    Index(u32),
}

/// Preserve the original public parameter positions, including non-term slots.
/// `TypedDag.constants` is expression-local; `parameter_ids` maps it back to the plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanDag {
    pub dag: TypedDag,
    pub parameter_ids: Vec<u32>,
}
pub fn lower(
    tree: &PlanExpr,
    parameters: &[Parameter],
    types: &BTreeMap<u32, LaneType>,
    bind: bool,
) -> Result<PlanDag, Refusal> {
    fn convert(
        e: &PlanExpr,
        p: &[Parameter],
        types: &BTreeMap<u32, LaneType>,
        ids: &mut Vec<u32>,
    ) -> Result<Expr, Refusal> {
        let call = |op, args| Expr::Call { op, args };
        Ok(match e {
            PlanExpr::Term(Slot::Variable(slot)) => Expr::Variable {
                slot: *slot,
                ty: types.get(slot).copied().unwrap_or(LaneType::Dynamic),
            },
            PlanExpr::Term(Slot::Parameter(slot)) => {
                let Some(Parameter::Term(term)) = p.get(*slot as usize) else {
                    return Err(Refusal {
                        function: "parameter".into(),
                        reason: format!("term parameter {slot} is absent or has index type"),
                    });
                };
                ids.push(*slot);
                Expr::Constant(term.clone())
            }
            PlanExpr::BooleanInput(slot) => Expr::BooleanInput { slot: *slot },
            PlanExpr::Compare(op, a, b) => call(
                match op {
                    Compare::Equal => ExprOp::Eq,
                    Compare::Less => ExprOp::Lt,
                    Compare::LessOrEqual => ExprOp::Le,
                    Compare::Greater => ExprOp::Gt,
                    Compare::GreaterOrEqual => ExprOp::Ge,
                },
                vec![convert(a, p, types, ids)?, convert(b, p, types, ids)?],
            ),
            PlanExpr::And(a, b) | PlanExpr::Or(a, b) => call(
                if matches!(e, PlanExpr::And(..)) {
                    ExprOp::And
                } else {
                    ExprOp::Or
                },
                vec![convert(a, p, types, ids)?, convert(b, p, types, ids)?],
            ),
            PlanExpr::Not(a) => call(ExprOp::Not, vec![convert(a, p, types, ids)?]),
            PlanExpr::In(a, b) => call(
                ExprOp::In,
                std::iter::once(a.as_ref())
                    .chain(b.iter())
                    .map(|x| convert(x, p, types, ids))
                    .collect::<Result<_, _>>()?,
            ),
            PlanExpr::Call(op, args) => call(
                op.clone(),
                args.iter()
                    .map(|x| convert(x, p, types, ids))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
    let mut parameter_ids = Vec::new();
    let ir = convert(tree, parameters, types, &mut parameter_ids)?;
    let dag = if bind {
        expr::lower_bind(&ir)?
    } else {
        expr::lower(&ir)?
    };
    Ok(PlanDag { dag, parameter_ids })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::expr::{Gadget, PublicConstant};
    #[test]
    fn plan_positions_types_and_boolean_inputs() {
        let constant = Term::Literal(oxrdf::Literal::from(5));
        let parameters = [Parameter::Index(100), Parameter::Term(constant.clone())];
        let cmp = PlanExpr::Compare(
            Compare::GreaterOrEqual,
            Box::new(PlanExpr::Term(Slot::Variable(7))),
            Box::new(PlanExpr::Term(Slot::Parameter(1))),
        );
        let tree = PlanExpr::And(Box::new(cmp), Box::new(PlanExpr::BooleanInput(2)));
        let output = lower(
            &tree,
            &parameters,
            &BTreeMap::from([(7, LaneType::Integer)]),
            true,
        )
        .unwrap();
        assert_eq!(output.parameter_ids, vec![1]);
        assert_eq!(
            output.dag.constants,
            vec![PublicConstant {
                value: constant,
                ty: LaneType::Integer
            }]
        );
        assert!(
            output
                .dag
                .nodes
                .iter()
                .any(|n| n.gadget == Gadget::BooleanInput(2))
        );
        assert_eq!(output.dag.nodes[output.dag.root].gadget, Gadget::ToTerm);
        assert_eq!(
            lower(
                &PlanExpr::Term(Slot::Parameter(0)),
                &parameters,
                &BTreeMap::new(),
                false
            )
            .unwrap_err()
            .function,
            "parameter"
        );
    }
}

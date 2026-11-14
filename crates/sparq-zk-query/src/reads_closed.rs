//! Whole-input value-read sites for closed operators, shared by codegen and witness.
use crate::{
    closed_arith::Integer,
    plan::{Expr, NodeId, Operator, Plan, Slot, Var},
    plan_closed::{ClosedOperator as C, Predicate},
};
use std::collections::BTreeSet;

pub(crate) struct ReadSite {
    pub input: NodeId,
    pub columns: Vec<usize>,
}
impl ReadSite {
    pub fn name(&self, node: usize) -> String {
        format!("closed_reads_{node}_{}", self.input)
    }
}
fn arithmetic(e: &Integer, vars: &mut BTreeSet<Var>) {
    match e {
        Integer::Term(Slot::Variable(v)) => {
            vars.insert(*v);
        }
        Integer::Term(_) => (),
        Integer::Add(a, b) | Integer::Subtract(a, b) | Integer::Multiply(a, b) => {
            arithmetic(a, vars);
            arithmetic(b, vars);
        }
    }
}
fn expression(e: &Expr, plan: &Plan, vars: &mut BTreeSet<Var>) {
    // Reuse the compiler's consumer analysis, including effective boolean reads.
    let schema: Vec<_> = (0..plan.variable_names().len() as Var).collect();
    vars.extend(
        crate::codegen::read_columns(e, &schema)
            .into_iter()
            .map(|i| schema[i]),
    );
}
fn predicate(p: &Predicate, plan: &Plan, vars: &mut BTreeSet<Var>) {
    match p {
        Predicate::Expression(e) => expression(e, plan, vars),
        Predicate::Arithmetic(e) => {
            arithmetic(&e.left, vars);
            arithmetic(&e.right, vars);
        }
        Predicate::Not(a) => predicate(a, plan, vars),
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            predicate(a, plan, vars);
            predicate(b, plan, vars);
        }
        // EXISTS reads table cardinality. Its child value consumers get their own sites.
        Predicate::Exists(_) => (),
    }
}
pub(crate) fn sites(plan: &Plan, node: usize) -> Vec<ReadSite> {
    let mut vars = BTreeSet::new();
    let mut inputs = vec![];
    match &plan.nodes()[node].operator {
        Operator::Closed(
            C::Filter {
                input,
                predicate: p,
                context,
            }
            | C::BindBoolean {
                input,
                predicate: p,
                context,
                ..
            },
        ) => {
            predicate(p, plan, &mut vars);
            inputs.push(*input);
            inputs.extend(context);
        }
        Operator::Closed(C::LeftJoin {
            left,
            right,
            condition: Some(e),
            context,
        }) => {
            expression(e, plan, &mut vars);
            inputs.extend([*left, *right]);
            inputs.extend(context);
        }
        Operator::Closed(C::Group {
            input, aggregates, ..
        }) => {
            use crate::interp_closed_group::AggregateKind as A;
            vars.extend(
                aggregates
                    .iter()
                    .filter(|a| matches!(a.kind, A::Sum | A::Avg | A::Min | A::Max))
                    .filter_map(|a| a.operand),
            );
            inputs.push(*input);
        }
        _ => (),
    }
    inputs.sort_unstable();
    inputs.dedup();
    inputs
        .into_iter()
        .filter_map(|input| {
            let columns = plan.nodes()[input as usize]
                .output
                .variables
                .iter()
                .enumerate()
                .filter_map(|(i, v)| vars.contains(v).then_some(i))
                .collect::<Vec<_>>();
            (!columns.is_empty()).then_some(ReadSite { input, columns })
        })
        .collect()
}

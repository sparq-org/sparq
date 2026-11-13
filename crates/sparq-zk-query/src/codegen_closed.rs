//! Wiring for closed relations. All constraints live in library gadgets.
use super::*;
use crate::plan_closed::{ClosedOperator as C, Predicate};

fn columns(from: &[Var], to: &[Var]) -> String {
    if to.is_empty() {
        return array([from.len().max(1).to_string()]);
    }
    array(to.iter().map(|v| {
        from.iter()
            .position(|x| x == v)
            .unwrap_or(from.len().max(1))
            .to_string()
    }))
}
fn term(p: u32, ids: &[u32]) -> String {
    format!(
        "parameters[{}]",
        ids.iter().position(|x| *x == p).expect("term parameter")
    )
}
fn steps(e: &Expr, vars: &[Var], ids: &[u32]) -> String {
    let mut out = vec![];
    expression(e, vars, ids, &mut out);
    array(out.iter().map(Step::source))
}
pub(super) fn generate(
    op: &C,
    i: usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) -> Result<(), Error> {
    let schema = |id: NodeId| plan.nodes[id as usize].output.variables.as_slice();
    let vars = &plan.nodes[i].output.variables;
    let cols = |id: NodeId| columns(schema(id), vars);
    match op {
        C::Union { left, right } => writeln!(
            out,
            "    core::ops_closed_union::union(t{left}, t{right}, {}, {}, t{i});",
            cols(*left),
            cols(*right)
        )
        .unwrap(),
        C::Minus { left, right } => writeln!(
            out,
            "    core::ops_closed_negation::minus(t{left}, t{right}, {}, t{i});",
            columns(schema(*right), schema(*left))
        )
        .unwrap(),
        C::Extend {
            input,
            variable,
            value,
            context,
        } => {
            let target = vars
                .iter()
                .position(|v| v == variable)
                .expect("BIND target");
            let (src, c, is_constant) = match value {
                Slot::Variable(v) => (
                    schema(*input)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*input).len().max(1)),
                    "unbound()".into(),
                    false,
                ),
                Slot::Parameter(p) => (0, term(*p, ids), true),
            };
            let (ctx, capture) = match context {
                Some(id) => (
                    format!("t{id}"),
                    match value {
                        Slot::Variable(v) => schema(*id)
                            .iter()
                            .position(|x| x == v)
                            .unwrap_or(schema(*id).len().max(1)),
                        _ => schema(*id).len().max(1),
                    },
                ),
                None => ("core::table::empty::<1,1>()".into(), 1),
            };
            writeln!(out,"    core::ops_closed_extend::extend_context(t{input}, {}, {target}, {src}, {c}, {is_constant}, {ctx}, {capture}, t{i});",cols(*input)).unwrap();
        }
        C::Substitute { input, context } => writeln!(
            out,
            "    core::ops_closed_substitute::values(t{input}, t{context}, {}, t{i});",
            columns(schema(*context), schema(*input))
        )
        .unwrap(),
        C::Capture { input, row, parent } => {
            let (ctx, pc, has) = match parent {
                Some(p) => (format!("t{p}"), cols(*p), true),
                None => (
                    "core::table::empty::<1, 1>()".into(),
                    array(std::iter::repeat_n("1".into(), vars.len().max(1))),
                    false,
                ),
            };
            writeln!(out,"    core::ops_closed_substitute::capture(t{input}, {row}, {ctx}, {has}, {}, {pc}, t{i});",cols(*input)).unwrap();
        }
        C::Unit { context } => writeln!(
            out,
            "    core::ops_closed_substitute::unit(t{context}, t{i});"
        )
        .unwrap(),
        C::GraphSeed {
            context,
            credential,
            name,
            selector,
        } => {
            let (column, constant) = match selector {
                Slot::Variable(v) => (
                    schema(*context)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*context).len().max(1)),
                    "unbound()".into(),
                ),
                Slot::Parameter(p) => (schema(*context).len().max(1), term(*p, ids)),
            };
            writeln!(out,"    core::ops_closed_graph::seed(t{context}, credential_count, {credential}, {}, {column}, {constant}, t{i});",term(*name,ids)).unwrap();
        }
        C::Graph {
            input,
            context,
            name,
            selector,
        } => {
            let (target, capture) = match selector {
                Slot::Variable(v) => (
                    vars.iter().position(|x| x == v).expect("graph target"),
                    schema(*context)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*context).len().max(1)),
                ),
                _ => (vars.len().max(1), schema(*context).len().max(1)),
            };
            writeln!(out,"    core::ops_closed_graph::graph(t{input}, t{context}, {}, {target}, {capture}, {}, t{i});",cols(*input),term(*name,ids)).unwrap();
        }
        C::Scan {
            pattern,
            context,
            credential,
        } => {
            let mapping = array(pattern.iter().map(|s| {
                match s {
                    Slot::Variable(v) => vars
                        .iter()
                        .position(|x| x == v)
                        .expect("scan schema")
                        .to_string(),
                    _ => vars.len().max(1).to_string(),
                }
            }));
            let captures = array(pattern.iter().map(|s| {
                match s {
                    Slot::Variable(v) => schema(*context)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*context).len().max(1))
                        .to_string(),
                    _ => schema(*context).len().max(1).to_string(),
                }
            }));
            let constants = array(pattern.iter().map(|s| match s {
                Slot::Parameter(p) => term(*p, ids),
                _ => "unbound()".into(),
            }));
            let cap = plan.capacity;
            writeln!(out,"    core::ops_closed_substitute::scan::<{}, {}, {}, {}, {}, {}, {}>(credentials, counts, credential_count, bounds, t{context}, {mapping}, {captures}, {constants}, {}, {}, t{i});",cap.credentials,cap.quads_per_credential,cap.credentials*cap.quads_per_credential,cap.rows_per_table,schema(*context).len().max(1),cap.rows_per_table,vars.len().max(1),credential.is_some(),credential.unwrap_or(0)).unwrap();
        }
        C::LeftJoin {
            left,
            right,
            condition,
            context,
        } => {
            let mut ev = vars.clone();
            if let Some(c) = context {
                ev.extend(schema(*c));
                ev.sort_unstable();
                ev.dedup();
            }
            let (ctx, cc) = match context {
                Some(c) => (format!("t{c}"), columns(schema(*c), &ev)),
                None => (
                    "core::table::empty::<1, 1>()".into(),
                    array(std::iter::repeat_n("1".into(), ev.len().max(1))),
                ),
            };
            let program = condition
                .as_ref()
                .map(|e| steps(e, &ev, ids))
                .unwrap_or_else(|| {
                    array([Step {
                        op: 7,
                        ..Step::default()
                    }
                    .source()])
                });
            writeln!(out,"    core::ops_closed_negation::left_join(t{left}, t{right}, {}, {}, {ctx}, {}, {cc}, parameters, {program}, {}, t{i});",cols(*left),cols(*right),columns(vars,&ev),condition.is_some()).unwrap();
        }
        C::Filter {
            input,
            predicate,
            context,
        } => {
            let mut next = 0;
            let value = predicate_source(predicate, *input, *context, i, &mut next, plan, ids, out);
            writeln!(out, "    ops_core::filter(t{input}, {value}, t{i});").unwrap();
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn predicate_source(
    p: &Predicate,
    input: NodeId,
    context: Option<NodeId>,
    node: usize,
    next: &mut usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) -> String {
    let name = format!("closed_predicate_{node}_{}", *next);
    *next += 1;
    match p {
        Predicate::Exists(rights) => writeln!(
            out,
            "    let {name} = core::ops_closed_negation::existence({});",
            array(rights.iter().map(|id| format!("t{id}")))
        )
        .unwrap(),
        Predicate::Expression(e) => {
            let input_vars = &plan.nodes[input as usize].output.variables;
            let mut vars = input_vars.clone();
            let source = if let Some(c) = context {
                let cv = &plan.nodes[c as usize].output.variables;
                vars.extend(cv);
                vars.sort_unstable();
                vars.dedup();
                writeln!(out,"    let overlay_{name} = core::ops_closed_common::overlay(t{input}, t{c}, {}, {});",columns(input_vars,&vars),columns(cv,&vars)).unwrap();
                format!("overlay_{name}")
            } else {
                format!("t{input}")
            };
            writeln!(
                out,
                "    let {name} = ops_core_expr::evaluate({source}, parameters, {});",
                steps(e, &vars, ids)
            )
            .unwrap();
        }
        Predicate::Not(a) => {
            let a = predicate_source(a, input, context, node, next, plan, ids, out);
            writeln!(
                out,
                "    let {name} = core::ops_closed_common::combine({a}, {a}, 0);"
            )
            .unwrap();
        }
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            let a = predicate_source(a, input, context, node, next, plan, ids, out);
            let b = predicate_source(b, input, context, node, next, plan, ids, out);
            writeln!(
                out,
                "    let {name} = core::ops_closed_common::combine({a}, {b}, {});",
                if matches!(p, Predicate::And(..)) {
                    1
                } else {
                    2
                }
            )
            .unwrap();
        }
    }
    name
}

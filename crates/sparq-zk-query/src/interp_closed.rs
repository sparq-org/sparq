//! Exact closed relations over complete input tables.

#[path = "interp_closed_path.rs"]
mod path;

use super::*;
use crate::plan_closed::{ClosedOperator as C, Predicate};

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate(
    op: &C,
    node: &Node,
    plan: &Plan,
    tables: &[Table],
    dataset: &[[Cell; 4]],
    credentials: &[CredentialCells],
    params: &[Option<Cell>],
) -> Result<Vec<Row>, Error> {
    let rows = |id: NodeId| &tables[id as usize].rows;
    let schema = |id: NodeId| plan.nodes[id as usize].output.variables.as_slice();
    let vars = &node.output.variables;
    let context_row = |context: Option<NodeId>, base_vars: &[Var], base: &[Cell]| {
        let mut all = base_vars.to_vec();
        if let Some(c) = context {
            all.extend(schema(c));
            all.sort_unstable();
            all.dedup();
        }
        let row = all
            .iter()
            .map(|v| {
                let outer = context
                    .and_then(|c| rows(c).first().map(|r| lookup(schema(c), r, *v)))
                    .unwrap_or_default();
                if outer.bound {
                    outer
                } else {
                    lookup(base_vars, base, *v)
                }
            })
            .collect::<Row>();
        (all, row)
    };
    Ok(match op {
        C::Path(p) => path::evaluate(p, node, plan, tables, dataset, credentials, params)?,
        C::Union { left, right } => [*left, *right]
            .into_iter()
            .flat_map(|id| {
                rows(id)
                    .iter()
                    .map(move |r| vars.iter().map(|v| lookup(schema(id), r, *v)).collect())
            })
            .collect(),
        C::Minus { left, right } => rows(*left)
            .iter()
            .filter(|a| {
                !rows(*right).iter().any(|b| {
                    let mut shared = false;
                    let mut compatible = true;
                    for v in schema(*left) {
                        let l = lookup(schema(*left), a, *v);
                        let r = lookup(schema(*right), b, *v);
                        shared |= l.bound && r.bound;
                        compatible &= !l.bound || !r.bound || l.same_term(r);
                    }
                    shared && compatible
                })
            })
            .cloned()
            .collect(),
        C::LeftJoin {
            left,
            right,
            condition,
            context,
        } => {
            let mut output = vec![];
            for a in rows(*left) {
                let mut matched = false;
                for b in rows(*right) {
                    let mut compatible = true;
                    let merged: Row = vars
                        .iter()
                        .map(|v| {
                            let l = lookup(schema(*left), a, *v);
                            let r = lookup(schema(*right), b, *v);
                            compatible &= !l.bound || !r.bound || l.same_term(r);
                            if l.bound {
                                l
                            } else {
                                r
                            }
                        })
                        .collect();
                    if compatible {
                        let (eval_vars, eval_row) = context_row(*context, vars, &merged);
                        let (pass, error) = condition
                            .as_ref()
                            .map(|e| eval(e, &eval_vars, &eval_row, params).and_then(Value::ebv))
                            .transpose()?
                            .unwrap_or((true, false));
                        if pass && !error {
                            matched = true;
                            output.push(merged);
                        }
                    }
                }
                if !matched {
                    output.push(vars.iter().map(|v| lookup(schema(*left), a, *v)).collect());
                }
            }
            output
        }
        C::Extend {
            input,
            variable,
            value,
            context,
        } => rows(*input)
            .iter()
            .map(|r| {
                let (sv, sr) = context_row(*context, schema(*input), r);
                vars.iter()
                    .map(|v| {
                        if v == variable {
                            match value {
                                Slot::Variable(v) => lookup(&sv, &sr, *v),
                                Slot::Parameter(p) => params[*p as usize].expect("term parameter"),
                            }
                        } else {
                            lookup(schema(*input), r, *v)
                        }
                    })
                    .collect()
            })
            .collect(),
        C::Substitute { input, context } => {
            let mut output = vec![];
            if let Some(c) = rows(*context).first() {
                for r in rows(*input) {
                    let mut matches = true;
                    let row = schema(*input)
                        .iter()
                        .map(|v| {
                            let outer = lookup(schema(*context), c, *v);
                            let value = lookup(schema(*input), r, *v);
                            matches &= !outer.bound || !value.bound || outer.same_term(value);
                            if outer.bound {
                                Cell::default()
                            } else {
                                value
                            }
                        })
                        .collect();
                    if matches {
                        output.push(row);
                    }
                }
            }
            output
        }
        C::Capture { input, row, parent } => {
            if let Some(r) = rows(*input).get(*row as usize) {
                if parent.is_some_and(|p| rows(p).is_empty()) {
                    vec![]
                } else {
                    let (_, r) = context_row(*parent, schema(*input), r);
                    vec![r]
                }
            } else {
                vec![]
            }
        }
        C::Unit { context } => {
            if rows(*context).is_empty() {
                vec![]
            } else {
                vec![vec![]]
            }
        }
        C::GraphSeed {
            context,
            credential,
            name,
            selector,
        } => {
            let graph = params[*name as usize].expect("graph name");
            let selected = match selector {
                Slot::Parameter(p) => params[*p as usize].expect("graph selector"),
                Slot::Variable(v) => rows(*context)
                    .first()
                    .map(|r| lookup(schema(*context), r, *v))
                    .unwrap_or_default(),
            };
            if (*credential as usize) < credentials.len()
                && (!selected.bound || selected.same_term(graph))
            {
                rows(*context).clone()
            } else {
                vec![]
            }
        }
        C::Graph {
            input,
            context,
            name,
            selector,
        } => {
            let graph = params[*name as usize].expect("graph name");
            let mut output = vec![];
            if let Some(c) = rows(*context).first() {
                for row in rows(*input) {
                    let captured = match selector {
                        Slot::Variable(v) => lookup(schema(*context), c, *v).bound,
                        _ => false,
                    };
                    let mut result = vars
                        .iter()
                        .map(|v| lookup(schema(*input), row, *v))
                        .collect::<Row>();
                    let mut compatible = true;
                    if let Slot::Variable(v) = selector {
                        let col = vars.iter().position(|x| x == v).expect("graph variable");
                        compatible = !result[col].bound || result[col].same_term(graph);
                        if !captured {
                            result[col] = graph;
                        }
                    }
                    if compatible {
                        output.push(result);
                    }
                }
            }
            output
        }
        C::Scan {
            pattern,
            context,
            credential,
        } => {
            let mut output = vec![];
            if let Some(c) = rows(*context).first() {
                let mut named = vec![];
                if let Some(id) = credential {
                    if let Some(c) = credentials.get(*id as usize) {
                        let mut seen = BTreeSet::new();
                        named.extend(
                            c.quads
                                .iter()
                                .filter(|q| {
                                    seen.insert(std::array::from_fn::<_, 3, _>(|i| q[i].identity()))
                                })
                                .copied(),
                        );
                    }
                }
                let source = if credential.is_some() {
                    named.as_slice()
                } else {
                    dataset
                };
                for q in source {
                    let mut row = vec![Cell::default(); vars.len()];
                    let mut matches = true;
                    for (p, s) in pattern.iter().enumerate() {
                        match s {
                            Slot::Parameter(id) => {
                                matches &= q[p].same_term(params[*id as usize].expect("parameter"))
                            }
                            Slot::Variable(v) => {
                                let outer = lookup(schema(*context), c, *v);
                                if outer.bound {
                                    matches &= q[p].same_term(outer);
                                } else {
                                    let col =
                                        vars.iter().position(|x| x == v).expect("scan column");
                                    matches &= !row[col].bound || row[col].same_term(q[p]);
                                    row[col] = q[p];
                                }
                            }
                        }
                    }
                    if matches {
                        output.push(row);
                    }
                }
            }
            output
        }
        C::Filter {
            input,
            predicate,
            context,
        } => {
            let mut output = vec![];
            for (i, r) in rows(*input).iter().enumerate() {
                let (vars, row) = context_row(*context, schema(*input), r);
                let (v, e) = predicate_value(predicate, i, &vars, &row, params, tables)?;
                if v && !e {
                    output.push(r.clone());
                }
            }
            output
        }
    })
}
fn predicate_value(
    p: &Predicate,
    index: usize,
    vars: &[Var],
    row: &[Cell],
    params: &[Option<Cell>],
    tables: &[Table],
) -> Result<(bool, bool), Error> {
    Ok(match p {
        Predicate::Expression(e) => eval(e, vars, row, params)?.ebv()?,
        Predicate::Exists(ids) => (!tables[ids[index] as usize].rows.is_empty(), false),
        Predicate::Not(a) => {
            let (v, e) = predicate_value(a, index, vars, row, params, tables)?;
            (!v, e)
        }
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            let (a, ae) = predicate_value(a, index, vars, row, params, tables)?;
            let (b, be) = predicate_value(b, index, vars, row, params, tables)?;
            if matches!(p, Predicate::And(..)) {
                (a && b, (ae || be) && !(!a && !ae || !b && !be))
            } else {
                (a || b, (ae || be) && !(a && !ae || b && !be))
            }
        }
    })
}
